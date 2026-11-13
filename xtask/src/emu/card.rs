//! The card checks: full-size and sparse cards, the card log, storfast, formatting and the CF card.

#[allow(clippy::wildcard_imports)]
use super::*;

/// A card image as large as a real card, as a sparse file: only the
/// formatter's non-zero parts are written (a few MB however large the
/// card). On Windows the file is marked sparse before it is sized, so NTFS
/// never fills in the zeros, even when the emulator writes near the end.
pub(crate) fn sparse_card(path: &Path, img: &rp_fat::format::Image<'_>) -> Result<(), String> {
    use std::io::{Seek, SeekFrom, Write};
    let err = |e: std::io::Error| format!("{}: {e}", path.display());
    let _ = std::fs::remove_file(path);
    std::fs::File::create(path).map_err(err)?;
    #[cfg(windows)]
    run(Command::new("fsutil")
        .args(["sparse", "setflag"])
        .arg(path)
        .stdout(std::process::Stdio::null()))?;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(err)?;
    f.set_len(u64::from(img.total_sectors) * 512).map_err(err)?;
    let mut failed = None;
    rp_fat::format::write_sparse(img, &mut |off, data| {
        if failed.is_none()
            && let Err(e) = f.seek(SeekFrom::Start(off)).and_then(|_| f.write_all(data))
        {
            failed = Some(e);
        }
    });
    failed.map_or(Ok(()), |e| Err(err(e)))
}

/// Tests at the owner's card sizes (item 57): the loader with a 4 GB card
/// (4 KiB clusters, a million of them) and a 32 GB card (16 KiB clusters,
/// two million), each 90% full of ballast (allocated clusters, chained in
/// the FAT) so the long commands do their full-size work: start-up (mount
/// and boot-file guard), `card check` (every chain, both FATs), `put` (free
/// space found past the ballast), the last blocks scanned, and the drive's
/// capacity and last block. Each must finish, with the loader answering,
/// inside the run limit: slow-at-full-size problems (the watchdog resets of
/// 2026-10-11/12) show up here before the device.
pub fn boot_loader_full_size(root: &Path) -> Result<(), String> {
    if !has_usb_model(root) || !has_msc_model(root) {
        eprintln!("    full-size cards: skipped (no USB or mass storage model)");
        return Ok(());
    }
    let dir = emu_dir(root)?;
    let loader = dir.join("rp-loader.bin");
    crate::sdkit::raw_binary(&crate::arm_release(root).join("rp-loader"), &loader)?;
    let note: Vec<u8> = (0..20_000u32).map(|i| (i * 7 + 1) as u8).collect();
    let put = rp_loadproto::file_frame("full-size.txt", &note);
    for (label, gib, spc) in [("4 GB", 4u64, 8u32), ("32 GB", 32, 32)] {
        let bytes = gib << 30;
        let sectors = (bytes / 512) as u32;
        let cluster = u64::from(spc) * 512;
        let ballast = bytes * 9 / 10 / cluster * cluster;
        let image = scratch(root, &format!("full-{gib}g.img"))?;
        sparse_card(
            &image,
            &rp_fat::format::Image {
                total_sectors: sectors,
                sectors_per_cluster: spc,
                files: &[rp_fat::format::Entry {
                    name: "hello.txt",
                    data: b"a card as big as the owner's",
                }],
                blocked_clusters: &[],
                ballast,
            },
        )?;
        // FSInfo's free count and next-free hint as "unknown" (many cards
        // carry them so), so `put` has to find the free space by scanning
        // the FAT past the ballast: the slow path, at full size.
        {
            use std::io::{Seek, SeekFrom, Write};
            let fsinfo = u64::from(rp_fat::format::PARTITION_START + 1) * 512;
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .open(&image)
                .map_err(|e| format!("{}: {e}", image.display()))?;
            f.seek(SeekFrom::Start(fsinfo + 488))
                .and_then(|_| f.write_all(&[0xFF; 8]))
                .map_err(|e| format!("{}: {e}", image.display()))?;
        }
        let mut args: Vec<String> = usb_args(12).into();
        args.extend(msc_args(13));
        let extra: Vec<&str> = args.iter().map(String::as_str).collect();
        let check = || -> Result<(), String> {
            let started = Instant::now();
            let mut s = usb_connect(12)?;
            usb_read_until(&mut s, "RPLOAD1 READY", 120)?;
            let ready = started.elapsed();
            let mut step =
                |cmd: &[u8], want: &str, what: &str| -> Result<(String, Duration), String> {
                    let t = Instant::now();
                    s.write_all(cmd).map_err(|e| e.to_string())?;
                    // The reply line (the idle loader's READY lines don't
                    // start with "RPL ").
                    let fail = |e: String| format!("{label} card, {what}: {e}");
                    let mut got = usb_read_until(&mut s, "RPL ", 300).map_err(fail)?;
                    while !got.rsplit("RPL ").next().unwrap_or("").contains("\r\n") {
                        got += &usb_read_until(&mut s, "\r\n", 30).map_err(fail)?;
                    }
                    let tail = got.rsplit("RPL ").next().unwrap_or("");
                    let reply = format!("RPL {}", tail.split("\r\n").next().unwrap_or(""));
                    if !reply.contains(want) {
                        return Err(format!("{label} card, {what}: {reply:?}"));
                    }
                    Ok((reply, t.elapsed()))
                };
            let check_cmd = rp_loadproto::control_frame("card check");
            let (_, check1) = step(&check_cmd, "RPL OK check clean", "card check")?;
            let (_, put_t) = step(&put, "RPL OK 20000 bytes", "put past the ballast")?;
            let (after, check2) = step(&check_cmd, "RPL OK check clean", "card check after put")?;
            if !after.contains("unchecked 0)") {
                return Err(format!("{label} card: clusters left unchecked: {after:?}"));
            }
            let last = sectors - 4096;
            let scan = rp_loadproto::control_frame(&format!("card scan {last} 4096"));
            let (_, scan_t) = step(&scan, "4096: 0 failed", "scan of the last blocks")?;
            // The PC's view: the whole card, its last block readable.
            let mut bot = Bot::connect(13)?;
            bot.mount()?;
            let deadline = Instant::now() + Duration::from_secs(30);
            while bot.command(&[0; 6], 0, false, &[])?.status != 0 {
                let _ = bot.sense()?;
                if Instant::now() > deadline {
                    return Err(format!("{label} card: the drive never became ready"));
                }
                std::thread::sleep(Duration::from_millis(250));
            }
            let r = bot.command(&[0x25, 0, 0, 0, 0, 0, 0, 0, 0, 0], 8, true, &[])?;
            if r.data.get(..4) != Some(&(sectors - 1).to_be_bytes()[..]) {
                return Err(format!("{label} card: capacity {:?}", r.data));
            }
            let mut block = [0u8; 512];
            rp_hal_api::BlockDevice::read_blocks(&mut bot, sectors - 1, &mut block)
                .map_err(|_| format!("{label} card: last block unreadable over the drive"))?;
            eprintln!(
                "    {label} card OK: ready {:.1} s, check {:.1} s, put {:.1} s, check {:.1} s, end scan {:.1} s; drive capacity and last block",
                ready.as_secs_f32(),
                check1.as_secs_f32(),
                put_t.as_secs_f32(),
                check2.as_secs_f32(),
                scan_t.as_secs_f32()
            );
            Ok(())
        };
        let result = boot(
            root,
            &Boot {
                image: &loader,
                marker: LOADER_MARKER,
                screenshot: None,
                sd_image: Some(&image),
                steps: &[],
                extra: &extra,
                input: None,
                during: Some(&check),
                then: None,
                timeout: Duration::from_secs(900),
            },
        );
        let _ = std::fs::remove_file(&image);
        result?;
    }
    Ok(())
}

/// A disk image in memory, for checking the emulator's card afterwards.
pub(crate) struct Disk(pub(crate) Vec<u8>);

impl rp_hal_api::BlockDevice for Disk {
    fn read_blocks(&mut self, lba: u32, buf: &mut [u8]) -> Result<(), rp_hal_api::BlockError> {
        let o = lba as usize * 512;
        if o + buf.len() > self.0.len() {
            return Err(rp_hal_api::BlockError::OutOfRange);
        }
        buf.copy_from_slice(&self.0[o..o + buf.len()]);
        Ok(())
    }
    fn write_blocks(&mut self, lba: u32, buf: &[u8]) -> Result<(), rp_hal_api::BlockError> {
        let o = lba as usize * 512;
        if o + buf.len() > self.0.len() {
            return Err(rp_hal_api::BlockError::OutOfRange);
        }
        self.0[o..o + buf.len()].copy_from_slice(buf);
        Ok(())
    }
}

/// A test card for the card log: FAT32 with someone else's file on it
/// (`other`), and `seeded` old `rplog/light-NNNN.txt` files (for the
/// cleanup that keeps each image's newest `KEEP`).
pub(crate) fn cardlog_card(path: &Path, other: &[u8], seeded: u32) -> Result<(), String> {
    let card = rp_fat::format::build(&rp_fat::format::Image {
        total_sectors: 64 * 1024,
        sectors_per_cluster: 8,
        files: &[rp_fat::format::Entry {
            name: "other.txt",
            data: other,
        }],
        blocked_clusters: &[],
        ballast: 0,
    });
    let mut vol = rp_fat::AnyVolume::mount(Disk(card), rp_fat::format::PARTITION_START)
        .map_err(|e| format!("card image: {}", e.name()))?;
    let mut scratch = vec![0u8; 64 * 1024];
    for n in 1..=seeded {
        vol.put(&format!("rplog/light-{n:04}.txt"), b"old", &mut scratch)
            .map_err(|e| format!("seeding the card: {}", e.name()))?;
    }
    std::fs::write(path, &vol.device().0).map_err(|e| format!("{}: {e}", path.display()))
}

/// The files in `folder` on a card image (names only).
pub(crate) fn card_names(image: &Path, folder: &str) -> Result<Vec<String>, String> {
    let data = std::fs::read(image).map_err(|e| format!("{}: {e}", image.display()))?;
    let mut vol = rp_fat::AnyVolume::mount(Disk(data), rp_fat::format::PARTITION_START)
        .map_err(|e| format!("card image: {}", e.name()))?;
    let mut names = Vec::new();
    vol.list(folder, &mut |n, dir, _| {
        if !dir {
            names.push(String::from_utf8_lossy(n).into_owned());
        }
    })
    .map_err(|e| format!("{folder}: {}", e.name()))?;
    Ok(names)
}

/// The run log as the PC received it (after the last `RPL LOG`), with
/// `\n` line ends, the notes the loader adds after the image's text left
/// out.
pub(crate) fn received_log(output: &str) -> Result<String, String> {
    let (_, log) = output.rsplit_once("RPL LOG").ok_or("no run log")?;
    let log = log.split("RPL END").next().unwrap_or(log);
    let text = log.replace("\r\n", "\n");
    let text = text.trim_start_matches('\n');
    let end = text
        .find("\n[the image ")
        .or_else(|| text.find("\n[run log full"))
        .unwrap_or(text.len());
    Ok(text[..end].trim_end_matches('\n').to_string())
}

/// The slot file holding `name`'s run `number` on a card image, by its
/// `rplog/index.txt` (R22: the run logs reuse fixed files).
pub(crate) fn slot_file(image: &Path, name: &str, number: u32) -> Result<String, String> {
    use rp_hal_x51v::cardlog;
    let index = card_file(image, cardlog::INDEX)?;
    let (slot, _) = cardlog::find(&index, Some(name.as_bytes()), Some(number))
        .ok_or_else(|| format!("{name}-{number:04} isn't in {}", cardlog::INDEX))?;
    let mut path = [0u8; 32];
    let n = cardlog::slot_path(slot, &mut path);
    Ok(String::from_utf8_lossy(&path[..n]).into_owned())
}

/// A run's card log: both cards' halves of `name`'s run `number` joined,
/// as `cargo xtask cardlog` joins them.
pub(crate) fn joined_card_log(
    sd: &Path,
    cf: &Path,
    name: &str,
    number: u32,
) -> Result<String, String> {
    let file = format!("{name}-{number:04}");
    let part = |card: &Path| {
        slot_file(card, name, number)
            .ok()
            .and_then(|f| card_file(card, &f).ok())
    };
    let sd_part = part(sd);
    let cf_part = part(cf);
    if sd_part.is_none() && cf_part.is_none() {
        return Err(format!("{file} is on neither card"));
    }
    for part in [&sd_part, &cf_part].into_iter().flatten() {
        if part.len() != rp_hal_x51v::cardlog::FILE_BYTES {
            return Err(format!(
                "{file} is {} bytes, reserved {}",
                part.len(),
                rp_hal_x51v::cardlog::FILE_BYTES
            ));
        }
    }
    let bytes: Vec<u8> =
        rp_hal_x51v::cardlog::join(sd_part.as_deref(), cf_part.as_deref()).collect();
    Ok(String::from_utf8_lossy(&bytes)
        .replace("\r\n", "\n")
        .trim_end_matches('\n')
        .to_string())
}

/// The card log for every run the loader starts (quality audit C24, the
/// owner's vote B): with an SD and a CF card in, each run's log is on the
/// cards line for line as the run goes, joined it is exactly the run log
/// the PC receives (the loader's closing line included), and nothing
/// outside `rplog/` changes on either card.
///
/// - rp-light: on the SD card; the CF card's file stays unwritten. The
///   number follows the 40 seeded files (41), and the oldest beyond the
///   newest 32 are deleted, 8 a run.
/// - rp-sdtime takes the SD controller: the log moves to the CF card with
///   a line saying so, and the two halves join into the whole log.
/// - rp-crash: closed as crashed.
/// - An image that hangs with interrupts off: the hardware watchdog resets
///   the machine (RAM kept, as a soft reset keeps it), and the loader closes
///   the log at its next start as a run that didn't return.
pub fn boot_loader_cardlog(root: &Path) -> Result<(), String> {
    // A failure's whole text (the card's log with it) kept in a file: a
    // rare failure was lost to a filtered terminal once (2026-10-22, the
    // sdtime guard, 1 run in 10; quality audit C60).
    cardlog_checks(root).map_err(|e| {
        let kept = scratch(root, "cardlog-failure.txt")
            .ok()
            .filter(|p| std::fs::write(p, &e).is_ok());
        match kept {
            Some(p) => format!(
                "{e}
(kept in {})",
                p.display()
            ),
            None => e,
        }
    })
}

pub(crate) fn cardlog_checks(root: &Path) -> Result<(), String> {
    let loader = raw_image(root, "rp-loader")?;
    let other = b"another session's file: must stay as it is".to_vec();
    let sd = scratch(root, "sd-cardlog.img")?;
    let cf = scratch(root, "cf-cardlog.img")?;
    cardlog_card(&sd, &other, 40)?;
    cardlog_card(&cf, &other, 0)?;
    let cf_drive = drive_arg(root, "ide", &cf)?;
    let run =
        |name: &str, payload: &[u8], limit: Option<u32>, until: &str| -> Result<String, String> {
            let mut input = rp_loadproto::control_frame(&format!("name {name}"));
            if let Some(s) = limit {
                input.extend(rp_loadproto::control_frame(&format!("limit {s}")));
            }
            input.extend(rp_loadproto::frame(payload));
            let output = boot(
                root,
                &Boot {
                    image: &loader,
                    marker: LOADER_MARKER,
                    screenshot: None,
                    sd_image: Some(&sd),
                    steps: &[],
                    extra: &["-drive", &cf_drive],
                    input: Some((&input, until)),
                    during: None,
                    // The loader ready again: past the run log, its word on
                    // each card for the PC (R22's vouch) comes before it.
                    then: Some(LOADER_MARKER),
                    timeout: Duration::from_secs(600),
                },
            )?;
            if !output.contains("RPL OK name") {
                return Err(format!("cardlog {name}: the loader didn't take the name"));
            }
            Ok(output)
        };
    let check = |name: &str, number: u32, output: &str, how: &str| -> Result<String, String> {
        let file = format!("{name}-{number:04}");
        let card =
            joined_card_log(&sd, &cf, name, number).map_err(|e| format!("cardlog {name}: {e}"))?;
        let log = received_log(output).map_err(|e| format!("cardlog {name}: {e}"))?;
        if card != log {
            return Err(format!(
                "cardlog {name}: {file} differs from the run log:\n--- card ---\n{card}\n--- run log ---\n{log}"
            ));
        }
        let first = format!("[loader: card log {file} in rplog/ on the SD and CF cards]");
        if !card.contains(&first) {
            return Err(format!("cardlog {name}: no `{first}` in:\n{card}"));
        }
        let closing = card.lines().last().unwrap_or("");
        if !closing.starts_with(&format!("[card log closed: the run {how}"))
            || !closing.contains(" 0 refused")
        {
            return Err(format!(
                "cardlog {name}: the closing line is wrong: {closing}"
            ));
        }
        // Written as the run went, not only by the loader at its end (a
        // hang that loses the RAM leaves only what was written as it went):
        // a sector write a line, so at least as many writes as the lines
        // before the closing line, but for the loader's first two. Since
        // quality audit C59 a line that ends while the card still programs
        // goes with the next write: the emulated SD card never programs (a
        // write a line still), the CF card is busy for a moment after each
        // (rp-sdtime's part folds lines: 47 writes for 54, its guard run 40
        // for 101), so there at least one write for every four lines. A
        // ratio, not the intent: the proper check counts the program's
        // writes apart from the loader's (quality audit C59).
        let writes: usize = closing
            .split(" sector writes")
            .next()
            .and_then(|h| h.rsplit(' ').next())
            .and_then(|n| n.parse().ok())
            .ok_or_else(|| format!("cardlog {name}: no write count in: {closing}"))?;
        let lines = card.lines().count() - 1;
        let floor = if card.contains("continuing on the CF card") {
            lines / 4
        } else {
            lines
        };
        if writes + 2 < floor {
            return Err(format!(
                "cardlog {name}: {writes} sector writes for {lines} lines: the lines weren't written as the run went"
            ));
        }
        Ok(card)
    };
    let image = |n: &str| -> Result<Vec<u8>, String> {
        let p = raw_image(root, n)?;
        std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))
    };

    // rp-clocks' stage C2b (C2 test 1): its card log on the X51v kept only
    // its first sector twice (clocks-0002, -0003, 2026-10-30), the run hung
    // later; here the whole log must reach the cards as it goes.
    let c2b = variant_image_envs(
        root,
        "rp-clocks",
        &[("RP_CLOCKS_OC", "1"), ("RP_CLOCKS_OC_SHORT", "1")],
    )?;
    let c2b = std::fs::read(&c2b).map_err(|e| format!("{}: {e}", c2b.display()))?;
    let out = run("clocks", &c2b, None, "RPL END")?;
    check("clocks", 1, &out, "returned")?;

    let out = run("light", &image("rp-light")?, None, "RPL END")?;
    check("light", 41, &out, "returned")?;
    if card_file(&cf, &slot_file(&cf, "light", 41)?)?
        .iter()
        .any(|&b| b != 0)
    {
        return Err(
            "cardlog light: the CF card's file was written though the SD card was free".into(),
        );
    }
    // The slots made once (R22): 32 log files and the index on each card;
    // the 40 old-style files left as they were (their numbers counted).
    for (card, what) in [(&sd, "SD"), (&cf, "CF")] {
        let names = card_names(card, "rplog")?;
        let slots = names.iter().filter(|n| n.starts_with("log-")).count();
        let old = names.iter().filter(|n| n.starts_with("light-")).count();
        let want_old = if what == "SD" { 40 } else { 0 };
        if slots != rp_hal_x51v::cardlog::SLOTS
            || old != want_old
            || !names.iter().any(|n| n == "index.txt")
        {
            return Err(format!(
                "cardlog light: the {what} card's rplog/ should hold {} slots, the index and {want_old} old files: {names:?}",
                rp_hal_x51v::cardlog::SLOTS
            ));
        }
    }
    eprintln!(
        "    cardlog light OK: the run log on the SD card line for line, numbered 41 (after the 40 old files), 32 slots and an index made on each card"
    );

    // R193: each boot is the loader's first start, so each keeps its own
    // start-up log (`loader-NNNN`): the clocks boot's is 1, light's 2. The
    // board as found, then USB, closed (the PC configured the device, or
    // the run started), the same bytes on both cards.
    let boot =
        joined_card_log(&sd, &cf, "loader", 2).map_err(|e| format!("cardlog loader: {e}"))?;
    for want in [
        "RetroPane loader start-up log (R193), loader-0002 on the SD and CF cards",
        "] CKEN ",
        "] UDCCR ",
        " UP2OCR ",
        "] GPIO14 level ",
        "] CPLD table at 0xA8001320",
        "] CPLD 0x1C (bit 3: the X50v lead): bus ",
        "] cards: SD in, CF in",
        "] usb: resets ",
        // QEMU's -kernel passes an ATAG list as HaRET does; without -append
        // its command line is empty, and nothing is switched (R193's opt-in
        // only on asking: a loader that always set bit 3 fails here).
        "] command line: \"\"; no opt-in: no CPLD bit changes",
    ] {
        if !boot.contains(want) {
            return Err(format!("cardlog loader: no `{want}` in:\n{boot}"));
        }
    }
    if boot.contains("CPLD 0x1C bit 3 on") {
        return Err(format!(
            "cardlog loader: bit 3 set without the opt-in:\n{boot}"
        ));
    }
    let closing = boot.lines().last().unwrap_or("");
    if !closing.contains("[start-up log closed: ")
        || !closing.contains(" 0 refused, 0 bytes dropped]")
    {
        return Err(format!(
            "cardlog loader: the closing line is wrong: {closing}"
        ));
    }
    let halves =
        [&sd, &cf].map(|card| slot_file(card, "loader", 2).and_then(|f| card_file(card, &f)));
    match halves {
        [Ok(a), Ok(b)] if a == b => {}
        _ => return Err("cardlog loader: the SD and CF cards' start-up logs differ".into()),
    }
    // Kept for reading: the emulator's board as the loader found it.
    let kept = scratch(root, "loader-startup-log.txt")?;
    std::fs::write(&kept, &boot).map_err(|e| format!("{}: {e}", kept.display()))?;
    eprintln!(
        "    cardlog loader OK: the start-up log (loader-0002) on both cards alike, the board as found, USB, closed: {closing} (kept in {})",
        kept.display()
    );

    let out = run("sdtime", &image("rp-sdtime")?, None, "RPL END")?;
    let card = check("sdtime", 1, &out, "returned")?;
    let moved = "[card log: the SD card was taken by the program; continuing on the CF card]";
    if card.matches(moved).count() != 1 {
        return Err(format!("cardlog sdtime: no `{moved}` once:\n{card}"));
    }
    if card_file(&cf, &slot_file(&cf, "sdtime", 1)?)?
        .iter()
        .all(|&b| b == 0)
    {
        return Err("cardlog sdtime: nothing on the CF card".into());
    }
    // R22: the run took the SD card (the PC re-reads it) and left the CF
    // card alone, whose slots existed: the PC keeps its view of it.
    if !out.contains("CF card: unchanged by the run (the PC keeps its view)") {
        return Err(format!(
            "cardlog sdtime: the CF card wasn't vouched for after a run that left it alone:\n{out}"
        ));
    }
    if out.contains("SD card: unchanged by the run") {
        return Err("cardlog sdtime: the SD card was vouched for though the run took it".into());
    }
    eprintln!(
        "    cardlog sdtime OK: the SD test moved the log to the CF card; the halves join into the run log"
    );

    // rp-sdtime's guard run (quality audit C32): block numbers off the
    // card refused by the card-log writer and the driver, nothing written.
    let guard = variant_image(root, "rp-sdtime", ("RP_SDTIME_GUARD", "1"))?;
    let guard = std::fs::read(&guard).map_err(|e| format!("{}: {e}", guard.display()))?;
    let out = run("sdtime", &guard, None, "RPL END")?;
    let card = check("sdtime", 2, &out, "returned")?;
    for want in [
        "G1 the card-log writer: the loader handed over ",
        "ok  the loader's capacity is the driver's",
        "ok  the last block as it was",
        "G7a: OUT_OF_RANGE in ",
        "  G7b wait 10000 us: read Ok(())",
        "RETROPANE-SDTIME: the guard run: ",
    ] {
        if !card.contains(want) {
            return Err(format!(
                "cardlog sdtime guard: no `{want}`:
{card}"
            ));
        }
    }
    // Version 6 (C46's follow-up): every wait the driver counted has its
    // own `end wait` line, so an unseen one is named by its step.
    let counted = card
        .split("the driver collected after ")
        .nth(1)
        .and_then(|r| r.split_whitespace().next())
        .and_then(|n| n.parse::<usize>().ok())
        .ok_or_else(|| {
            format!(
                "cardlog sdtime guard: no count of the driver's waits:
{card}"
            )
        })?;
    let logged = card
        .lines()
        .filter(|l| l.starts_with("  end wait "))
        .count();
    if counted == 0 || logged != counted {
        return Err(format!(
            "cardlog sdtime guard: {logged} `end wait` lines for the driver's {counted} waits:
{card}"
        ));
    }
    if card.contains("  BAD ") || !card.contains(" checks, 0 BAD") {
        return Err(format!(
            "cardlog sdtime guard: a check failed:
{card}"
        ));
    }
    eprintln!(
        "    cardlog sdtime guard OK: every block off the card refused before a command, the last block as it was, each end-of-card wait logged"
    );

    let out = run("crash", &image("rp-crash")?, None, "RPL END")?;
    check("crash", 1, &out, "crashed")?;
    eprintln!("    cardlog crash OK: closed as crashed");

    // mov r0, #1; mcr p14, 0, r0, c0, c1, 0 (PMNC: count); b .
    let words = [0xE3A0_0001u32, 0xEE00_0E11, 0xEAFF_FFFE];
    let code: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    let out = run("hang", &code, Some(1), "did not return")?;
    check("hang", 1, &out, "did not return")?;
    eprintln!(
        "    cardlog hang OK: a watchdog reset with RAM kept, closed at the loader's next start"
    );

    // R193's opt-in (docs/research/sheets/r193-x50v-cpld1c-bit3.md): the
    // command line through the ATAG list (QEMU's -append, as HaRET's `set
    // cmdline`), bit 3 at 0 s: set before the controller starts, in the
    // shadow and the board record, logged, then the PC configures.
    let n = (1..=64u32)
        .filter(|&k| slot_file(&sd, "loader", k).is_ok())
        .max()
        .unwrap_or(0)
        + 1;
    let input = rp_loadproto::control_frame("board");
    let settle = || {
        // The PC's configuration (~1 s in) closes the log; its last sectors
        // go to the cards right after.
        std::thread::sleep(Duration::from_secs(4));
        Ok(())
    };
    let out = super::boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: Some(&sd),
            steps: &[],
            extra: &["-drive", &cf_drive, "-append", "rp.x50v-usb-bit3=0"],
            input: Some((&input, "RPL OK")),
            during: Some(&settle),
            then: None,
            timeout: Duration::from_secs(300),
        },
    )?;
    if !out.contains("RPL OK board: ") {
        return Err(format!(
            "cardlog x50v: the loader didn't answer:
{out}"
        ));
    }
    let x50v = joined_card_log(&sd, &cf, "loader", n).map_err(|e| format!("cardlog x50v: {e}"))?;
    let at = |want: &str| {
        x50v.find(want)
            .ok_or_else(|| format!("cardlog x50v: no `{want}` in:\n{x50v}"))
    };
    at(
        "] command line: \"rp.x50v-usb-bit3=0\"; opt-in rp.x50v-usb-bit3: CPLD 0x1C bit 3 set at its time",
    )?;
    let bit = at(
        "] CPLD 0x1C bit 3 on (opt-in rp.x50v-usb-bit3): 0000 -> 0008 (WM slot the same), record 0008",
    )?;
    // Dell's order: the bit before the controller's first event.
    if at("] usb: resets ")? < bit {
        return Err(format!("cardlog x50v: USB started before bit 3:\n{x50v}"));
    }
    if !x50v.ends_with("]") || !x50v.contains("[start-up log closed: the PC configured the device;")
    {
        return Err(format!(
            "cardlog x50v: not closed at the configuration:\n{x50v}"
        ));
    }
    let kept = scratch(root, "loader-startup-log-x50v.txt")?;
    std::fs::write(&kept, &x50v).map_err(|e| format!("{}: {e}", kept.display()))?;
    eprintln!(
        "    cardlog x50v OK: with rp.x50v-usb-bit3=0 on the command line, bit 3 set before USB (0000 -> 0008, shadow and record), then configured (loader-{n:04}, kept in {})",
        kept.display()
    );

    for (card, what) in [(&sd, "SD"), (&cf, "CF")] {
        if card_file(card, "other.txt")? != other {
            return Err(format!(
                "cardlog: the card log changed a file outside rplog/ on the {what} card"
            ));
        }
    }
    eprintln!("    cardlog OK: nothing outside rplog/ changed on either card");
    Ok(())
}

/// A gpubench run's card log (the loader's, `rp_hal_x51v::cardlog`)
/// against its run log: the run's own number, word for word the run log
/// the PC received (from the loader's first line to its closing line),
/// closed as a run that returned, no write refused.
pub(crate) fn gpubench_card_log(
    sd: &Path,
    name: &str,
    number: u32,
    log: &str,
) -> Result<(), String> {
    let file = slot_file(sd, "gpubench", number).map_err(|e| format!("gpubench {name}: {e}"))?;
    let raw = card_file(sd, &file).map_err(|e| format!("gpubench {name}: card log: {e}"))?;
    if raw.len() != rp_hal_x51v::cardlog::FILE_BYTES {
        return Err(format!(
            "gpubench {name}: {file} is {} bytes, reserved {}",
            raw.len(),
            rp_hal_x51v::cardlog::FILE_BYTES
        ));
    }
    let card: Vec<u8> = rp_hal_x51v::cardlog::join(Some(&raw), None).collect();
    let card = String::from_utf8_lossy(&card)
        .replace("\r\n", "\n")
        .trim_end_matches('\n')
        .to_string();
    let log = received_log(&format!("RPL LOG{log}"))?;
    if card != log {
        return Err(format!(
            "gpubench {name}: {file} differs from the run log:\n--- card ---\n{card}\n--- run log ---\n{log}"
        ));
    }
    let closing = card.lines().last().unwrap_or("");
    if !closing.starts_with("[card log closed: the run returned") || !closing.contains(" 0 refused")
    {
        return Err(format!(
            "gpubench {name}: {file} doesn't end with a clean closing line: {closing}"
        ));
    }
    Ok(())
}

/// Reads root-directory file `name` from a FAT32 disk image.
pub(crate) fn card_file(image: &Path, name: &str) -> Result<Vec<u8>, String> {
    let data = std::fs::read(image).map_err(|e| format!("{}: {e}", image.display()))?;
    // FAT32 or exFAT, whichever the image holds (both formatters put the
    // partition at the same place).
    let mut vol = rp_fat::AnyVolume::mount(Disk(data), rp_fat::format::PARTITION_START)
        .map_err(|e| format!("card image: {}", e.name()))?;
    let mut out = Vec::new();
    let mut scratch = vec![0u8; 64 * 1024];
    vol.read_back(name, &mut scratch, &mut |c| out.extend_from_slice(c))
        .map_err(|e| format!("{name}: {}", e.name()))?;
    Ok(out)
}

/// A `-drive` argument for `path` on `interface`, as the emulator in use
/// sees paths (WSL or native).
pub(crate) fn drive_arg(root: &Path, interface: &str, path: &Path) -> Result<String, String> {
    let p = match emulator(root)? {
        Emulator::Wsl => to_wsl_path(path),
        Emulator::Native(_) => path.display().to_string(),
    };
    Ok(format!("if={interface},format=raw,file={p}"))
}

/// `cargo xtask emu-storfast`: the storage speed-ups' device test
/// (`rp-storfast`, docs/research/storage-speed.md) started by the loader
/// with a 128 MiB FAT32 SD card and a CF card, as on the device. The card
/// swap and lock steps aren't modelled (the image waits for them and
/// carries on); everything else must pass: CF paths equal, 20 attach
/// cycles, SD reads of every size class by one DMA transfer, the DMA write
/// soak exact, the test files removed.
pub fn boot_loader_storfast(root: &Path) -> Result<(), String> {
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let loader = dir.join("rp-loader.bin");
    let image = dir.join("rp-storfast.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-storfast"), &image)?;
    let sd = scratch(root, "sd-storfast.img")?;
    let card = rp_fat::format::build(&rp_fat::format::Image {
        total_sectors: 128 * 2048,
        sectors_per_cluster: 1,
        // The quick-run file: the emulated FIFO moves bytes slowly.
        files: &[
            rp_fat::format::Entry {
                name: "retropane.bin",
                data: b"kit stand-in",
            },
            rp_fat::format::Entry {
                name: "storfast-quick.txt",
                data: b"short run",
            },
        ],
        blocked_clusters: &[],
        ballast: 0,
    });
    std::fs::write(&sd, &card).map_err(|e| format!("{}: {e}", sd.display()))?;
    let cf = scratch(root, "cf-storfast.img")?;
    // A FAT32 card (C7 makes its test file there) with a noise file
    // filling most of it (C2-C4 compare what they read, so it isn't zeros).
    let noise: Vec<u8> = (0..12u32 << 20)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
        .collect();
    // 32 MiB: the noise, the loader's 32 run-log slots (8 MiB, made once on
    // each card, R22) and C11's 512 KiB file in one run.
    let cf_data = rp_fat::format::build(&rp_fat::format::Image {
        total_sectors: 64 * 1024,
        sectors_per_cluster: 8,
        files: &[rp_fat::format::Entry {
            name: "noise.bin",
            data: &noise,
        }],
        blocked_clusters: &[],
        ballast: 0,
    });
    std::fs::write(&cf, cf_data).map_err(|e| format!("{}: {e}", cf.display()))?;
    let payload = std::fs::read(&image).map_err(|e| format!("{}: {e}", image.display()))?;
    let input = rp_loadproto::frame(&payload);
    let cf_drive = drive_arg(root, "ide", &cf)?;
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: Some(&sd),
            steps: &[],
            extra: &["-drive", &cf_drive],
            input: Some((&input, "RPL END")),
            during: None,
            then: None,
            timeout: Duration::from_secs(900),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("storfast: no run log")?;
    if crashed(log) || !log.contains("RETROPANE-STORFAST: done") {
        return Err(format!("storfast: didn't finish cleanly:{log}"));
    }
    let dma_window = emulator_accepts_global(root, "pxa2xx-pcmcia.word-cycles=on");
    let mut want = vec![
        "storfast-quick.txt found",
        "C3 3 attach cycles: all good",
        "C4 tune (measured, ",
        "the card's limit ",
        "C6 Windows Mobile's timing put back: MCMEM1 0x00020508",
        "C7 rp-cf-write/WRITE.BIN: LBA ",
        "C7 the shortest right data strobe 0 (",
        "C9 strobe: the shortest right ",
        "C9 hold: the shortest right ",
        "C9 set-up: the shortest right ",
        "C10 data at Windows Mobile's: ",
        "C10 data at strobe 9: ",
        "C10 data at strobe 1: ",
        "C11 fresh at strobe 1: ",
        "C11 rewrite 6 of region 12 at Windows Mobile's: ",
        "C11 read back at the slowest timing: 0 of 960 sectors differ",
        "C11 rp-cf-write/FRESH.BIN removed: true",
        "C12 recovery at strobe 1: ",
        "C12 idle 1000 ms: region 28 rewritten again after the pause: ",
        "C12 CFA feature set: ",
        "C12 read back at the slowest timing: 0 of ",
        "C12 rp-cf-write/ERASE.BIN removed: true",
        "C13 64 sectors, write 6 to the same place: ",
        "C13 a log's pattern, 16 lines two to a sector: ",
        "C13 read back at the slowest timing: 0 sectors differ",
        "C13 rp-cf-write/SIZES.BIN removed: true",
        "C7 rp-cf-write removed from the CF card",
        "C8 done: ",
        "KiB): all right, one transfer each",
        "S2 the speed writes read back: 0 blocks differ",
        "of the last writes read back, 0 blocks differ",
        "S9 test files and folder removed",
    ];
    if dma_window {
        want.push("reads by DMA from the window (0 paths refused)");
        want.push("C2 read 1024 KiB by DMA from the window: ");
    }
    for w in want {
        if !log.contains(w) {
            return Err(format!("storfast: no `{w}` in the run log:{log}"));
        }
    }
    for line in log
        .lines()
        .filter(|l| l.starts_with("S3") && l.contains("blocks differ"))
    {
        if !line.ends_with(" 0 blocks differ") {
            return Err(format!("storfast: {line}"));
        }
    }
    for line in log.lines().filter(|l| {
        l.starts_with("S2 soak") || l.starts_with("C2 read") || l.starts_with("C4 own timing")
    }) {
        let clean = line.contains(" 0 write errors, 0 read errors, 0 blocks differ")
            || line.contains("0 commands failed, 0 sectors differ")
            || line.ends_with(", 0 failed, 0 sectors differ");
        if !clean {
            return Err(format!("storfast: {line}"));
        }
    }
    eprintln!(
        "    storfast OK (short run): CF read paths equal, attach cycles good; SD every size class by one DMA transfer; the DMA write soak read back exact; test files removed; run log:{log}"
    );
    Ok(())
}

/// `cargo xtask emu-cf`: the CompactFlash check on its own.
/// `cargo xtask emu-format` alone: the loader built here first.
pub fn boot_loader_format_alone(root: &Path) -> Result<(), String> {
    let dir = emu_dir(root)?;
    crate::sdkit::raw_binary(
        &crate::arm_release(root).join("rp-loader"),
        &dir.join("rp-loader.bin"),
    )?;
    boot_loader_format(root)
}

/// Formatting on the device (ADR-0036): the loader formats the SD card
/// (128 MiB, FAT32 with the kit's boot files and icon) and the CF card
/// (64 MiB) into each format through `format`, keeping the boot files and
/// the icon `autorun.inf` names (read back), refusing exFAT while boot
/// files are on the card and a FAT12 the card is too big for, and wiping
/// them when told. After each format the PC sees the drive go away and
/// come back, mounts it with `rp-fat` through the drive, finds the kept
/// files whole, and its write over a boot file is refused (the guard
/// rebuilt for FAT16 and exFAT). Afterwards the card images are checked on
/// the host.
pub fn boot_loader_format(root: &Path) -> Result<(), String> {
    if !has_usb_model(root) {
        eprintln!("    format: skipped (this emulator has no USB controller model)");
        return Ok(());
    }
    let loader = raw_image(root, "rp-loader")?;
    let boot_bin: Vec<u8> = (0..70_000u32).map(|i| (i * 7 + 1) as u8).collect();
    let haret: Vec<u8> = (0..20_000u32).map(|i| (i * 13 + 5) as u8).collect();
    let icon: Vec<u8> = (0..3_000u32).map(|i| (i * 3) as u8).collect();
    let autorun = b"[autorun]\r\nicon=card-sd.ico\r\n";
    let big: Vec<u8> = (0..2_621_440u32)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
        .collect();
    let entry = |name, data| rp_fat::format::Entry { name, data };
    let sd_image = rp_fat::format::build(&rp_fat::format::Image {
        total_sectors: 128 * 2048,
        sectors_per_cluster: 8,
        files: &[
            entry("retropane.bin", &boot_bin),
            entry("haret.exe", &haret),
            entry("startup.txt", b"set ramaddr 0xa8000000\n"),
            entry("autorun.inf", autorun),
            entry("card-sd.ico", &icon),
            entry("photo.jpg", b"not kept"),
            entry("big.bin", &big),
        ],
        blocked_clusters: &[],
        ballast: 0,
    });
    let sd = scratch(root, "sd-format.img")?;
    std::fs::write(&sd, sd_image).map_err(|e| format!("{}: {e}", sd.display()))?;
    let cf_image = rp_fat::format::build(&rp_fat::format::Image {
        total_sectors: 64 * 2048,
        sectors_per_cluster: 8,
        files: &[entry("on-cf.txt", b"compact flash")],
        blocked_clusters: &[],
        ballast: 0,
    });
    let cf = scratch(root, "cf-format.img")?;
    std::fs::write(&cf, cf_image).map_err(|e| format!("{}: {e}", cf.display()))?;
    let mut args: Vec<String> = usb_args(2).into();
    args.extend(msc_args(3));
    args.push("-drive".into());
    args.push(drive_arg(root, "ide", &cf)?);
    args.extend(wm_cpld_table_args());
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let check = || -> Result<(), String> {
        let mut s = usb_connect(2)?;
        let mut bot = Bot::connect(3)?;
        usb_read_until(&mut s, "RPLOAD1 READY", 10)?;
        // A frame sent until the loader stops saying "card busy" (it
        // takes no card command within 2 s of the PC's last write).
        let busy_retry = |s: &mut std::net::TcpStream, frame: &[u8], start: &str, secs| {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                s.write_all(frame).map_err(|e| e.to_string())?;
                let reply = usb_read_line(s, start, secs)?;
                if !reply.contains("RPL ERR card busy") || Instant::now() > deadline {
                    return Ok::<_, String>(reply);
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        };
        let say = |s: &mut std::net::TcpStream, cmd: &str, want: &str, secs| {
            let reply = busy_retry(s, &rp_loadproto::control_frame(cmd), "RPL ", secs)?;
            let line = reply
                .lines()
                .find(|l| l.starts_with("RPL "))
                .unwrap_or("")
                .to_string();
            if !line.starts_with(want) {
                return Err(format!("`{cmd}`: {line:?} (wanted {want:?})"));
            }
            eprintln!("    {cmd}: {line}");
            Ok(line)
        };
        // The PC's view of a unit after a format: away, back, then mounted
        // through the drive; the file `name`'s first sector, if it's there.
        let pc_view = |bot: &mut Bot, unit: u8, after: &str, kind: &str, name: &str| {
            bot.lun = unit;
            expect_away_then_back(bot, after)?;
            let mut mbr = [0u8; 512];
            rp_hal_api::BlockDevice::read_blocks(bot, 0, &mut mbr)
                .map_err(|_| format!("{after}: the MBR doesn't read"))?;
            let start = u32::from_le_bytes([mbr[454], mbr[455], mbr[456], mbr[457]]);
            let mut v = rp_fat::AnyVolume::mount(&mut *bot, start)
                .map_err(|e| format!("{after}: the PC can't mount it: {}", e.name()))?;
            let got = match &v {
                rp_fat::AnyVolume::Fat(f) => f.kind().name(),
                rp_fat::AnyVolume::ExFat(_) => "exFAT",
            };
            if got != kind {
                return Err(format!("{after}: the PC sees {got}, not {kind}"));
            }
            Ok::<_, String>(v.extent(name).ok().map(|r| r.lba))
        };
        // The PC mounts both drives first, as at plug-in (a unit's first
        // answer is its power-on "medium changed").
        for unit in [0u8, 1] {
            bot.lun = unit;
            let deadline = Instant::now() + Duration::from_secs(30);
            while bot.command(&[0; 6], 0, false, &[])?.status != 0 {
                let _ = bot.sense()?;
                if Instant::now() > deadline {
                    return Err(format!("unit {unit} never became ready"));
                }
                std::thread::sleep(Duration::from_millis(200));
            }
        }
        // A big file by `hash` and ranged `get`s (R49): three ranges put
        // back together equal it, and the loader's SHA-256 is its own.
        let h = say(&mut s, "hash big.bin", "RPL OK hash 2621440 bytes", 60)?;
        let sha = rp_crypto::hex(&rp_crypto::Sha256::digest(&big));
        if !h.contains(std::str::from_utf8(&sha).unwrap_or("?")) {
            return Err(format!("hash big.bin: {h:?}"));
        }
        let mut whole = Vec::new();
        for at in [0u32, 1_000_000, 2_000_000] {
            let cmd = format!("get @{at}+1000000 big.bin");
            let text = busy_retry(&mut s, &rp_loadproto::control_frame(&cmd), "RPL ", 60)?;
            let (lines, reply) = crate::usb::parse_query(&text, "get")?;
            whole.extend(crate::usb::decode_file(&lines, &reply)?);
        }
        if whole != big {
            return Err(format!(
                "ranged gets of big.bin: {} bytes, not the file",
                whole.len()
            ));
        }
        let cmd = rp_loadproto::control_frame("get @3000000+10 big.bin");
        let past = busy_retry(&mut s, &cmd, "RPL ", 20)?;
        if !past.contains("RPL OK 0 bytes crc 00000000 of 2621440") {
            return Err(format!("a range past the end: {past:?}"));
        }
        // exFAT while the card holds boot files: refused, nothing written
        // (the drive stays as it is).
        say(
            &mut s,
            "format sd exfat",
            "RPL ERR format: Windows Mobile can't read exFAT",
            20,
        )?;
        say(
            &mut s,
            "format sd fat12 cluster 16",
            "RPL ERR format: cluster count",
            20,
        )?;
        say(
            &mut s,
            "format sd fat16 plan",
            "RPL OK plan: would format as FAT16",
            20,
        )?;
        // FAT16, the boot files and the icon kept (photo.jpg isn't).
        let line = say(
            &mut s,
            "format sd fat16 label X51V",
            "RPL OK formatted as FAT16",
            60,
        )?;
        if !line.contains("5 files kept and read back (3 boot)") {
            return Err(format!("format sd fat16 kept the wrong files: {line:?}"));
        }
        let haret_at = pc_view(&mut bot, 0, "format sd fat16", "FAT16", "haret.exe")?
            .ok_or("haret.exe wasn't kept")?;
        // The PC is shown the boot file hidden + system + read-only (the
        // owner's choice, 2026-10-25); the card keeps it as it was (checked
        // on the image afterwards).
        let mut mbr = [0u8; 512];
        rp_hal_api::BlockDevice::read_blocks(&mut bot, 0, &mut mbr)
            .map_err(|_| "the MBR doesn't read")?;
        let fat16_start = u32::from_le_bytes([mbr[454], mbr[455], mbr[456], mbr[457]]);
        match fixed_root_attr(&mut bot, fat16_start, b"HARET   EXE") {
            Some(a) if a & 0x07 == 0x07 => {}
            other => {
                return Err(format!(
                    "haret.exe as the PC sees it: attributes {other:x?}"
                ));
            }
        }
        let r = bot.command(&rw10(0x2A, haret_at, 1), 512, false, &[0; 512])?;
        if r.status == 0 {
            return Err("the PC overwrote haret.exe on the new FAT16 card: no guard".into());
        }
        let _ = bot.sense()?;
        bot.lun = 0;
        expect_no_blink_after_refusal(
            &mut bot,
            "a refused write over haret.exe on the new FAT16 card",
        )?;
        // The refused write counts as the PC writing: the loader takes no
        // card command for 2 s after it (ADR-0014 rule 3); the next one is
        // retried while it says so.
        let text = busy_retry(
            &mut s,
            &rp_loadproto::control_frame("get card-sd.ico"),
            "RPL ",
            40,
        )?;
        let (lines, reply) = crate::usb::parse_query(&text, "get")?;
        if crate::usb::decode_file(&lines, &reply)? != icon {
            return Err("the icon came back different after the format".into());
        }
        say(&mut s, "card check", "RPL OK check clean", 40)?;
        // The layout as the loader reads it back, read-only.
        let line = say(
            &mut s,
            "card layout sd",
            "RPL OK layout: partition type 0x06",
            20,
        )?;
        if !line.contains("FAT16 by 'MSWIN4.1' label 'X51V       ', 16384 B clusters") {
            return Err(format!("card layout after the FAT16 format: {line:?}"));
        }
        // exFAT once the boot files are formatted away; then a file put.
        say(
            &mut s,
            "format sd exfat wipe-boot",
            "RPL OK formatted as exFAT",
            60,
        )?;
        pc_view(&mut bot, 0, "format sd exfat", "exFAT", "x")?;
        s.write_all(&rp_loadproto::file_frame("haret.exe", &haret))
            .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL OK 20000 bytes", 40)?;
        let haret_at = pc_view(&mut bot, 0, "put on exFAT", "exFAT", "haret.exe")?
            .ok_or("haret.exe isn't on the exFAT card")?;
        let r = bot.command(&rw10(0x2A, haret_at, 1), 512, false, &[0; 512])?;
        if r.status == 0 {
            return Err("the PC overwrote haret.exe on the exFAT card: no guard".into());
        }
        let _ = bot.sense()?;
        bot.lun = 0;
        expect_no_blink_after_refusal(
            &mut bot,
            "a refused write over haret.exe on the exFAT card",
        )?;
        // The refused write counts as the PC writing: the loader takes no
        // card command for 2 s after it (ADR-0014 rule 3); the next one is
        // retried while it says so.
        say(&mut s, "card check", "RPL OK check clean", 40)?;
        // FAT32 with 1 KiB clusters keeps the exFAT card's boot file.
        say(
            &mut s,
            "format sd fat32 cluster 1",
            "RPL OK formatted as FAT32 with 1 KiB clusters",
            60,
        )?;
        pc_view(&mut bot, 0, "format sd fat32", "FAT32", "haret.exe")?
            .ok_or("haret.exe wasn't kept through FAT32")?;
        // The CF card: FAT12 by size at 16 KiB clusters is too many, so
        // FAT16 by the default rule; then its boot folder, guarded.
        say(&mut s, "format cf auto", "RPL OK formatted as FAT", 60)?;
        s.write_all(&rp_loadproto::file_frame("cf:RetroPane/haret.exe", &haret))
            .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL OK 20000 bytes", 40)?;
        let mut mbr = [0u8; 512];
        bot.lun = 1;
        expect_away_then_back(&mut bot, "cf put")?;
        rp_hal_api::BlockDevice::read_blocks(&mut bot, 0, &mut mbr)
            .map_err(|_| "the CF MBR doesn't read")?;
        let start = u32::from_le_bytes([mbr[454], mbr[455], mbr[456], mbr[457]]);
        let at = rp_fat::AnyVolume::mount(&mut bot, start)
            .map_err(|e| format!("the PC can't mount the CF card: {}", e.name()))?
            .extent("RetroPane/haret.exe")
            .map_err(|e| format!("cf: {}", e.name()))?
            .lba;
        let r = bot.command(&rw10(0x2A, at, 1), 512, false, &[0; 512])?;
        if r.status == 0 {
            return Err("the PC overwrote the CF card's haret.exe: no guard".into());
        }
        let _ = bot.sense()?;
        bot.lun = 1;
        expect_no_blink_after_refusal(&mut bot, "a refused write over the CF card's haret.exe")?;
        // The refused write counts as the PC writing: the loader takes no
        // card command for 2 s after it (ADR-0014 rule 3); the next one is
        // retried while it says so.
        // exFAT on the CF card with its boot folder: refused; wiped: done.
        say(
            &mut s,
            "format cf exfat",
            "RPL ERR format: Windows Mobile can't read exFAT",
            20,
        )?;
        say(
            &mut s,
            "format cf exfat wipe-boot cluster 32",
            "RPL OK formatted as exFAT with 32 KiB clusters",
            60,
        )?;
        pc_view(&mut bot, 1, "format cf exfat", "exFAT", "x")?;
        Ok(())
    };
    boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: Some(&sd),
            steps: &[],
            extra: &extra,
            input: None,
            during: Some(&check),
            then: None,
            timeout: Duration::from_secs(240),
        },
    )?;
    // The images on the host: the SD card FAT32 with haret.exe whole, the CF
    // card exFAT; both clean.
    for (path, kind) in [(&sd, "FAT32"), (&cf, "exFAT")] {
        let data = std::fs::read(path).map_err(|e| e.to_string())?;
        let start = u32::from_le_bytes([data[454], data[455], data[456], data[457]]);
        let mut v = rp_fat::AnyVolume::mount(Disk(data), start)
            .map_err(|e| format!("{}: {}", path.display(), e.name()))?;
        let got = match &v {
            rp_fat::AnyVolume::Fat(f) => f.kind().name(),
            rp_fat::AnyVolume::ExFat(_) => "exFAT",
        };
        if got != kind {
            return Err(format!("{}: {got}, not {kind}", path.display()));
        }
        let h = v
            .check(&mut vec![0u8; 1 << 20], &mut vec![0u8; 8192])
            .map_err(|e| e.name().to_string())?;
        if !h.clean() {
            return Err(format!("{}: not clean: {h:?}", path.display()));
        }
        if kind == "FAT32" {
            let mut out = Vec::new();
            v.read_back("haret.exe", &mut [0u8; 4096], &mut |c| {
                out.extend_from_slice(c)
            })
            .map_err(|e| e.name().to_string())?;
            if out != haret {
                return Err("haret.exe on the SD image isn't whole".into());
            }
        }
    }
    eprintln!(
        "    format: SD FAT32 -> FAT16 -> exFAT -> FAT32, CF -> FAT -> exFAT; kept files whole, guards rebuilt"
    );
    Ok(())
}

pub fn boot_loader_cf_alone(root: &Path) -> Result<(), String> {
    let dir = emu_dir(root)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    crate::sdkit::raw_binary(
        &crate::arm_release(root).join("rp-loader"),
        &dir.join("rp-loader.bin"),
    )?;
    boot_loader_cf(root)
}

/// The CompactFlash slot (docs/hw/cf.md): a CF card image in socket 1 and
/// no SD card, so the CF path stands alone. The emulated slot starts
/// powered, as Windows Mobile hands it over (its CPLD table, measured);
/// the loader attaches the card on its own (powered through the CPLD,
/// reset, identified) before it
/// offers the drive, so the PC's first question finds the second drive
/// ready; `cf` reports it (capacity, write cache
/// off, partition) without taking it again; the CPLD writes leave the
/// backlight on (its registers don't read back, docs/hw/cpld.md); pulled
/// out, the slot is switched off, and pushed back in, attached again.
pub fn boot_loader_cf(root: &Path) -> Result<(), String> {
    if !has_usb_model(root) {
        eprintln!("    CF card: skipped (this emulator has no USB controller model)");
        return Ok(());
    }
    let loader = raw_image(root, "rp-loader")?;
    let image = rp_fat::format::build(&rp_fat::format::Image {
        total_sectors: 32 * 1024, // 16 MiB
        sectors_per_cluster: 8,
        files: &[rp_fat::format::Entry {
            name: "on-cf.txt",
            data: b"compact flash",
        }],
        blocked_clusters: &[],
        ballast: 0,
    });
    let cf = scratch(root, "cf.img")?;
    std::fs::write(&cf, image).map_err(|e| format!("{}: {e}", cf.display()))?;
    let mut args: Vec<String> = usb_args(2).into();
    args.extend(msc_args(3));
    args.push("-drive".into());
    args.push(drive_arg(root, "ide", &cf)?);
    args.extend(wm_cpld_table_args());
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let via_drive: Vec<u8> = (0..4096u32).map(|i| (i * 29 + 3) as u8).collect();
    let check = || -> Result<(), String> {
        let mut s = usb_connect(2)?;
        let mut bot = Bot::connect(3)?;
        bot.lun = 1;
        usb_read_until(&mut s, "RPLOAD1 READY", 10)?;
        // The slot starts powered with its card ready, as Windows Mobile
        // hands it over: the attach is the reset alone, not the 0.4 s power
        // cycle (boot time; the device's CF stage was 457 ms with it).
        s.write_all(&rp_loadproto::control_frame("handoff"))
            .map_err(|e| e.to_string())?;
        let stages = usb_read_until(
            &mut s, " ms
", 10,
        )?;
        let at = |key: &str| -> Option<u32> {
            stages
                .split(key)
                .nth(1)?
                .split(" ms")
                .next()?
                .trim()
                .parse()
                .ok()
        };
        // The board line reaches the PC (audit C57: serial only before).
        s.write_all(&rp_loadproto::control_frame("board"))
            .map_err(|e| e.to_string())?;
        let board = usb_read_until(
            &mut s, "
", 10,
        )?;
        if !board.contains("RPL OK board: recorded as Windows Mobile left it") {
            return Err(format!(
                "the loader's board line didn't reach the PC: {board:?}"
            ));
        }
        match (at("dma+fat "), at("cf ")) {
            (Some(before), Some(cf)) if cf - before < 200 => {}
            _ => {
                return Err(format!(
                    "CF attach at start-up power-cycled a powered slot: {stages:?}"
                ));
            }
        }
        // Nothing asked: the loader attaches the card in the slot itself,
        // and the PC's second drive becomes ready (after "medium changed").
        let ready = |bot: &mut Bot, what: &str| -> Result<(), String> {
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                let r = bot.command(&[0; 6], 0, false, &[])?;
                if r.status == 0 {
                    return Ok(());
                }
                let _ = bot.sense()?;
                if Instant::now() > deadline {
                    return Err(format!("CF unit never became ready ({what})"));
                }
                std::thread::sleep(Duration::from_millis(200));
            }
        };
        // The card in the slot at start-up is attached before the PC can
        // see the drive: a fresh mount's first question (READ CAPACITY)
        // finds it, with no "no medium" or "medium changed" first (the
        // owner's flaky "USB Drive (J:)": Explorer drew the drive empty, its
        // icon minutes later).
        let first = bot.command(&[0x25, 0, 0, 0, 0, 0, 0, 0, 0, 0], 8, true, &[])?;
        if first.status != 0 {
            let sense = bot.sense()?;
            return Err(format!(
                "CF unit not ready at the PC's first question (sense {sense:?}): the card in the slot at start-up must be attached before the drive is offered"
            ));
        }
        // `cf` reports the card the loader attached, without taking it again.
        s.write_all(&rp_loadproto::control_frame("cf"))
            .map_err(|e| e.to_string())?;
        let reply = usb_read_line(&mut s, "RPL ", 20)?;
        // Reads come by DMA from the card's data window once IDENTIFY read
        // through it matched the 16-bit port (docs/research/storage-speed.md):
        // on a fork modelling the PXA's two card cycles per 32-bit access
        // (rp-v0.17.0's `word-cycles`). On one that doesn't, the window
        // reads wrong and the driver must keep the port: the check is the
        // guard, so a wrong window never reaches a sector.
        let path = if emulator_accepts_global(root, "pxa2xx-pcmcia.word-cycles=on") {
            "reads by DMA from the window (0 paths refused)"
        } else {
            "reads by 16-bit port (1 paths refused)"
        };
        let want = "RPL OK cf: 16 MB (32768 sectors), write cache off, ";
        if !reply.contains(want)
            || !reply.contains(path)
            || !reply.contains("FAT32 partition")
            || !reply.contains("was card")
        {
            return Err(format!("unexpected reply to cf (want {path:?}): {reply:?}"));
        }
        // Within a second of the attach the loader measures the card's own
        // timing (ruling M, R7): the emulator's card reads right at every
        // timing (timing isn't modelled), so it must be applied.
        // Asked again every 200 ms (no fixed wait), for up to 10 s.
        let mut reply = String::new();
        for _ in 0..50 {
            s.write_all(&rp_loadproto::control_frame("cf"))
                .map_err(|e| e.to_string())?;
            reply = usb_read_line(&mut s, "RPL ", 20)?;
            if reply.contains("; timing own, ") {
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        // Its write timing too (the owner's ruling A, R28): measured once,
        // right at the attach (the card, a fresh image, is new to this
        // loader), on fresh sectors of its own file, and kept in
        // rplog/cfwrite.txt; "kept" once a later attach has read it there
        // (only the search writes that line).
        // The loader's stack after the attach's timing searches, its
        // deepest work: under 75% (16 KiB overflowed once into .bss and
        // stopped the loader on the X51v, quality audit C83).
        s.write_all(&rp_loadproto::control_frame("loader stack"))
            .map_err(|e| e.to_string())?;
        let stack_reply = usb_read_line(&mut s, "RPL ", 20)?;
        let stack = stack_reply
            .split("RPL OK stack ")
            .nth(1)
            .and_then(|t| {
                let mut n = t.split(" of ");
                let used: u32 = n.next()?.trim().parse().ok()?;
                let size: u32 = n.next()?.trim().parse().ok()?;
                Some((used, size))
            })
            .ok_or_else(|| format!("loader stack: {stack_reply:?}"))?;
        eprintln!(
            "    loader stack after the CF attach: {} of {} bytes",
            stack.0, stack.1
        );
        if stack.0 * 4 > stack.1 * 3 {
            return Err(format!(
                "the loader's stack is {} of {} bytes after the CF attach (over 75%)",
                stack.0, stack.1
            ));
        }
        if !reply.contains("; timing own, ")
            || !(reply.contains(" cycles (limit ") || reply.contains(" cycles (kept)"))
        {
            return Err(format!("cf: the card's own timing not measured: {reply:?}"));
        }
        // The power sequence and the card's reset went through the CPLD,
        // whose registers don't read back: the backlight enable sharing
        // register 0x18 with RESET must still be on.
        let mut mon = crate::screen::Monitor::connect(monitor_port())?;
        if device_has_runtime(&mut mon, "/machine/peripheral/mbx2700g", "backlight") {
            let lit = mon.query("qom-get /machine/peripheral/mbx2700g backlight")?;
            if lit != "true" {
                return Err(format!("CF: the backlight went off ({lit})"));
            }
        }
        // The shadow came from Windows Mobile's table, and the bits it had
        // on in the registers the CF attach wrote are still on (item 77):
        // SD power (0x16 bit 0) and the backlight (0x18 bit 2) beside the
        // slot's own.
        s.write_all(&rp_loadproto::control_frame("cpld"))
            .map_err(|e| e.to_string())?;
        let mut reply = usb_read_until(&mut s, "shadow from", 10)?;
        if !reply.rsplit("shadow from").next().is_some_and(|t| {
            t.contains(
                "
",
            )
        }) {
            reply += &usb_read_until(
                &mut s, "
", 10,
            )?;
        }
        for want in [
            "shadow from WM's table:",
            " 02=00fd",
            " 10=1000",
            " 12=000f",
            " 16=0003",
            " 18=0004",
            " 1c=000c",
        ] {
            if !reply.contains(want) {
                return Err(format!(
                    "CF: the CPLD's shadow isn't Windows Mobile's table with its bits kept (no {want:?}): {reply:?}"
                ));
            }
        }
        let r = bot.command(&[0x12, 0, 0, 0, 36, 0], 36, true, &[])?;
        if r.data.get(16..23) != Some(&b"CF card"[..]) {
            return Err(format!("CF unit: INQUIRY {:?}", r.data));
        }
        let r = bot.command(&[0x25, 0, 0, 0, 0, 0, 0, 0, 0, 0], 8, true, &[])?;
        if r.data.get(..4) != Some(&32767u32.to_be_bytes()[..]) {
            return Err(format!("CF unit: capacity {:?}", r.data));
        }
        let mut mbr = [0u8; 512];
        rp_hal_api::BlockDevice::read_blocks(&mut bot, 0, &mut mbr)
            .map_err(|_| "CF unit: sector 0 unreadable".to_string())?;
        if mbr[510..] != [0x55, 0xAA] {
            return Err("CF unit: sector 0 isn't the image's MBR".into());
        }
        // Writable by default, like the SD card (ADR-0014, the owner's
        // ruling A 2026-10-24): a write outside the boot files lands.
        if rp_hal_api::BlockDevice::write_blocks(&mut bot, 30000, &via_drive).is_err() {
            return Err("CF unit: a write refused though the drive is writable".into());
        }
        let mut back = vec![0u8; via_drive.len()];
        rp_hal_api::BlockDevice::read_blocks(&mut bot, 30000, &mut back)
            .map_err(|_| "CF unit: read-back failed".to_string())?;
        if back != via_drive {
            return Err("CF unit: read-back differs".into());
        }
        // `drive ro` locks it, `drive rw` opens it again (each a medium
        // change the PC is told of, once).
        let drive = |s: &mut std::net::TcpStream,
                     bot: &mut Bot,
                     cmd: &str,
                     want: &str,
                     writable: bool|
         -> Result<(), String> {
            s.write_all(&rp_loadproto::control_frame(cmd))
                .map_err(|e| e.to_string())?;
            usb_read_until(s, want, 10)?;
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                match rp_hal_api::BlockDevice::write_blocks(bot, 30000, &via_drive) {
                    Ok(()) if writable => return Ok(()),
                    Err(rp_hal_api::BlockError::ReadOnly) if !writable => return Ok(()),
                    _ => {}
                }
                if Instant::now() > deadline {
                    return Err(format!(
                        "CF unit: after {cmd}, writes not {}",
                        if writable { "taken" } else { "refused" }
                    ));
                }
                poll_pause(Duration::from_millis(100));
            }
        };
        drive(
            &mut s,
            &mut bot,
            "drive ro",
            "RPL OK drive read-only",
            false,
        )?;
        drive(
            &mut s,
            &mut bot,
            "drive rw",
            "RPL OK drive read-write",
            true,
        )?;
        // A boot file on the CF card, put there through the loader (`install
        // cf`'s path: `cf:` names), which the PC reads but can't change.
        let boot = (0..9000u32)
            .map(|i| (i * 13 % 251) as u8)
            .collect::<Vec<u8>>();
        // Refused while the PC wrote the card in the last 2 s (it just did):
        // sent again once it has been quiet.
        let deadline = Instant::now() + Duration::from_secs(15);
        let put = loop {
            s.write_all(&rp_loadproto::file_frame("cf:RetroPane/haret.exe", &boot))
                .map_err(|e| e.to_string())?;
            // Its reply: the first line past the idle loader's READY lines.
            let mut put = String::new();
            while !put.lines().any(|l| l.starts_with("RPL ")) {
                put += &usb_read_until(&mut s, "\r\n", 30)?;
            }
            if !put.contains("card busy") || Instant::now() > deadline {
                break put;
            }
            poll_pause(Duration::from_millis(500));
        };
        if !put.contains("RPL OK 9000 bytes") {
            return Err(format!("CF unit: put cf:RetroPane/haret.exe: {put:?}"));
        }
        let start = u32::from_le_bytes([mbr[454], mbr[455], mbr[456], mbr[457]]);
        let deadline = Instant::now() + Duration::from_secs(10);
        let (first, on_drive) = loop {
            // Once the PC has seen the medium change.
            let seen = rp_fat::Volume::mount(&mut bot, start)
                .ok()
                .and_then(|mut vol| {
                    let f = vol.find("RetroPane/haret.exe").ok()?;
                    let mut got = Vec::new();
                    vol.read_file(&f, &mut [0u8; 4096], |c| got.extend_from_slice(c))
                        .ok()?;
                    Some((vol.extent("RetroPane/haret.exe").ok()?.lba, got))
                });
            if let Some(x) = seen {
                break x;
            }
            if Instant::now() > deadline {
                return Err("CF unit: the put boot file never showed through the drive".into());
            }
            let _ = bot.sense();
            poll_pause(Duration::from_millis(100));
        };
        if on_drive != boot {
            return Err("CF unit: the put boot file reads back wrong through the drive".into());
        }
        if rp_hal_api::BlockDevice::write_blocks(&mut bot, first, &[0u8; 512])
            != Err(rp_hal_api::BlockError::ReadOnly)
        {
            return Err("CF unit: the PC overwrote a CF boot file (RetroPane/haret.exe)".into());
        }
        expect_no_blink_after_refusal(&mut bot, "a CF write the guard refused")?;
        // Pulled out: the loader lets the card go and switches the slot
        // off; pushed back in, it is attached again from a power cycle.
        if device_has_runtime(&mut mon, "/machine/peripheral/cfslot", "pressed") {
            mon.query("qom-set /machine/peripheral/cfslot pressed true")?;
            // The reply: the line starting `RPL OK cf:` among the idle
            // loader's READY lines (a long one is cut, its end kept).
            let mut cf_log = || -> Result<String, String> {
                s.write_all(&rp_loadproto::control_frame("cf log"))
                    .map_err(|e| e.to_string())?;
                let mut got = String::new();
                loop {
                    got += &usb_read_until(&mut s, "\r\n", 10)?;
                    if let Some(line) = got.lines().find(|l| l.starts_with("RPL OK cf:")) {
                        return Ok(line.to_string());
                    }
                }
            };
            // The loader notices on its detect poll (debounced, in emulated
            // seconds, slower than ours on a busy PC): poll, never a guess.
            let mut poll = |what: &str, done: &dyn Fn(&str) -> bool| -> Result<String, String> {
                let deadline = Instant::now() + Duration::from_secs(30);
                loop {
                    let log = cf_log()?;
                    if done(&log) {
                        return Ok(log);
                    }
                    if Instant::now() > deadline {
                        return Err(format!("CF {what}: {log:?}"));
                    }
                    poll_pause(Duration::from_millis(500));
                }
            };
            poll("removed: the slot was never powered off", &|l| {
                l.contains("power off")
            })?;
            mon.query("qom-set /machine/peripheral/cfslot pressed false")?;
            let log = match poll("re-inserted: never attached again", &|l| {
                l.rsplit("power off")
                    .next()
                    .is_some_and(|a| a.contains("attached"))
            }) {
                Ok(l) => l,
                // Why the attach failed: `cf` names the slot's last failure
                // ("was ..."), then tries it once more itself (R43).
                Err(e) => {
                    s.write_all(&rp_loadproto::control_frame("cf"))
                        .map_err(|e| e.to_string())?;
                    let cf = usb_read_line(&mut s, "RPL ", 20).unwrap_or_default();
                    return Err(format!("{e}; then cf: {cf:?}"));
                }
            };
            // Ordered events, no timing: a slot left powered would show
            // READY as soon as the card went back in, before the loader's
            // debounced attach (which powers it from off itself).
            let after_off = log.rsplit("power off").next().unwrap_or_default();
            let before_attach = after_off.split("attached").next().unwrap_or_default();
            if before_attach.contains("ready 1") {
                return Err(format!("CF removed: slot not powered off: {log:?}"));
            }
            ready(&mut bot, "re-inserted card")?;
            let mut mbr = [0u8; 512];
            rp_hal_api::BlockDevice::read_blocks(&mut bot, 0, &mut mbr)
                .map_err(|_| "CF re-inserted: sector 0 unreadable".to_string())?;
            // The same card again: its write timing comes from
            // rplog/cfwrite.txt, nothing measured or written (R28).
            let mut reply = String::new();
            for _ in 0..50 {
                s.write_all(&rp_loadproto::control_frame("cf"))
                    .map_err(|e| e.to_string())?;
                reply = usb_read_line(&mut s, "RPL ", 20)?;
                if reply.contains("; writes ") {
                    break;
                }
                poll_pause(Duration::from_millis(200));
            }
            if !reply.contains(" cycles (kept)") {
                return Err(format!(
                    "CF re-inserted: its kept write timing not used: {reply:?}"
                ));
            }
            // A quick swap (R43, on the X51v 0.12 s): out and back in a
            // fraction of a second, nothing reading the card meanwhile. The
            // loader must see it from the detect alone (read every 50 ms)
            // and attach the card again; once a second missed it, and the
            // card came back only when a later read failed.
            // `cf log` (reads only: `cf` would attach the card itself):
            // the slot's events, "attached" each time the loader took it.
            let mut attached = || -> Result<(usize, String), String> {
                s.write_all(&rp_loadproto::control_frame("cf log"))
                    .map_err(|e| e.to_string())?;
                let mut got = String::new();
                loop {
                    got += &usb_read_until(
                        &mut s, "
", 10,
                    )?;
                    if let Some(l) = got.lines().find(|l| l.starts_with("RPL OK cf:")) {
                        let events = l.split("; reads").next().unwrap_or(l).to_string();
                        return Ok((events.matches("attached").count(), events));
                    }
                }
            };
            let (before, _) = attached()?;
            mon.query("qom-set /machine/peripheral/cfslot pressed true")?;
            poll_pause(Duration::from_millis(150));
            mon.query("qom-set /machine/peripheral/cfslot pressed false")?;
            let deadline = Instant::now() + Duration::from_secs(20);
            loop {
                let (now, events) = attached()?;
                if now > before {
                    break;
                }
                if Instant::now() > deadline {
                    return Err(format!(
                        "CF quick swap: not seen by the detect and attached again: {events:?}"
                    ));
                }
                poll_pause(Duration::from_millis(250));
            }
            // The trace since the pull (R43): the PC's commands to the CF
            // drive, each with its time and outcome. This PC asks the drive
            // to be ready and reads sector 0; the trace must hold both.
            ready(&mut bot, "CF after the quick swap")?;
            let mut sector = [0u8; 512];
            rp_hal_api::BlockDevice::read_blocks(&mut bot, 0, &mut sector)
                .map_err(|_| "CF after the quick swap: sector 0 unreadable".to_string())?;
            s.write_all(&rp_loadproto::control_frame("cf trace"))
                .map_err(|e| e.to_string())?;
            let mut trace = usb_read_until(&mut s, "RPL OK cf trace: ", 20)?;
            if !trace.rsplit("RPL OK cf trace: ").next().is_some_and(|t| {
                t.contains(
                    "
",
                )
            }) {
                trace += &usb_read_until(
                    &mut s, "
", 10,
                )?;
            }
            let rpd: Vec<&str> = trace.lines().filter(|l| l.starts_with("RPD ")).collect();
            let total: usize = trace
                .rsplit("RPL OK cf trace: ")
                .next()
                .and_then(|t| t.split_whitespace().next()?.parse().ok())
                .ok_or_else(|| format!("cf trace: no count: {trace:?}"))?;
            // `RPD at took op lba blocks result`.
            let has = |op: &str, rest: &str| {
                rpd.iter().any(|l| {
                    let w: Vec<&str> = l.split_whitespace().collect();
                    w.get(3) == Some(&op) && w[4..].join(" ").starts_with(rest)
                })
            };
            if rpd.len() != total || !has("00", "0 0 ok") || !has("28", "0 1 ok") {
                return Err(format!(
                    "cf trace: {} lines for {total} commands, want TEST UNIT READY and READ(10) of sector 0: {trace:?}",
                    rpd.len()
                ));
            }
            eprintln!("    cf trace: {total} commands to the CF drive since the quick swap");
        }
        Ok(())
    };
    boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &extra,
            input: None,
            during: Some(&check),
            then: None,
            timeout: Duration::from_secs(60),
        },
    )?;
    let after = std::fs::read(&cf).map_err(|e| e.to_string())?;
    if after[30000 * 512..30000 * 512 + via_drive.len()] != via_drive[..] {
        return Err("CF: the drive's write isn't in the card image".into());
    }
    eprintln!(
        "    CF card OK: attached on its own as Windows Mobile left the slot, before the drive is offered (ready at the PC's first question), powered through the CPLD with the backlight kept on; identified in socket 1 (16 MB, write cache off) by cf without a remount, the PC's second drive: read, written by default (refused after drive ro, written after drive rw), checked in the image; a boot file put through the loader (cf:RetroPane/haret.exe) read but not overwritten through the drive; pulled out: slot powered off, attached again on re-insert (no READY before)"
    );
    Ok(())
}
