//! The drive checks over USB mass storage: puts, stale views, the drive, fallback, locked.

#[allow(clippy::wildcard_imports)]
use super::*;

/// Writing files to an exFAT card (SDXC) through the loader: a file
/// replaced, one created, one put into folders that didn't exist,
/// everything checked in the image afterwards, and the volume left clean
/// (VolumeDirty clear).
pub fn boot_loader_put_exfat(root: &Path) -> Result<(), String> {
    if !has_usb_model(root) {
        eprintln!("    exFAT put: skipped (this emulator has no USB controller model)");
        return Ok(());
    }
    let loader = raw_image(root, "rp-loader")?;
    let old: Vec<u8> = (0..3000u32).map(|i| (i * 3) as u8).collect();
    let new: Vec<u8> = (0..70_000u32).map(|i| (i * 17 + 1) as u8).collect();
    let created: Vec<u8> = (0..9000u32).map(|i| (i * 7 + 2) as u8).collect();
    let script = b"set ramaddr 0xa8000000\r\n".to_vec();
    let image = rp_fat::exfat::format::build(
        32 * 1024, // 16 MiB
        3,         // 4 KiB clusters
        &[
            rp_fat::exfat::format::Entry {
                name: "retropane.bin",
                data: &old,
                fragment: false,
            },
            rp_fat::exfat::format::Entry {
                name: "startup.txt",
                data: &script,
                fragment: true,
            },
        ],
    );
    let sd = scratch(root, "sd-exfat.img")?;
    std::fs::write(&sd, image).map_err(|e| format!("{}: {e}", sd.display()))?;
    let args = usb_args(2);
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let check = || -> Result<(), String> {
        let mut s = usb_connect(2)?;
        usb_read_until(&mut s, "RPLOAD1 READY", 10)?;
        s.write_all(&rp_loadproto::file_frame("retropane.bin", &new))
            .map_err(|e| e.to_string())?;
        let reply = usb_read_line(&mut s, "RPL ", 40)?;
        let line = reply
            .lines()
            .find(|l| l.starts_with("RPL "))
            .unwrap_or("")
            .to_string();
        let theirs = line.rsplit("sha256 ").next().unwrap_or("").trim();
        crate::usb::check_digest(theirs, &new, "file on the card")?;
        if !line.starts_with("RPL OK 70000 bytes") || !line.contains("contiguous") {
            return Err(format!("unexpected reply to the exFAT put: {line:?}"));
        }
        eprintln!("    loader (exFAT): {line}");
        s.write_all(&rp_loadproto::file_frame("New File.dat", &created))
            .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL OK 9000 bytes", 40)?;
        // Into folders that don't exist yet: made on the way.
        s.write_all(&rp_loadproto::file_frame("Apps/Sub Folder/x.txt", b"x"))
            .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL OK 1 bytes", 20)?;
        s.write_all(&rp_loadproto::control_frame("card check"))
            .map_err(|e| e.to_string())?;
        let reply = usb_read_line(&mut s, "RPL ", 40)?;
        if !reply.contains("RPL OK check clean") || !reply.contains(" 0 lost") {
            return Err(format!("exFAT card check after the writes: {reply:?}"));
        }
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
            timeout: Duration::from_secs(90),
        },
    )?;
    if card_file(&sd, "retropane.bin")? != new {
        return Err("exFAT: retropane.bin on the card isn't what was sent".into());
    }
    if card_file(&sd, "NEW FILE.DAT")? != created {
        return Err("exFAT: the created file isn't on the card".into());
    }
    if card_file(&sd, "startup.txt")? != script {
        return Err("exFAT: startup.txt changed".into());
    }
    if card_file(&sd, "apps/sub folder/X.TXT")? != b"x" {
        return Err("exFAT: the file in new folders isn't on the card".into());
    }
    let after = std::fs::read(&sd).map_err(|e| e.to_string())?;
    let flags = after[rp_fat::exfat::format::PARTITION_START as usize * 512 + 106];
    if flags & 0x02 != 0 {
        return Err("exFAT: the volume was left marked dirty".into());
    }
    eprintln!(
        "    exFAT put OK: replaced, created, and put into new folders through the loader; volume clean"
    );
    Ok(())
}

/// Most read commands `boot_loader_put`'s write may take (FAT search in
/// runs; see docs/perf.md).
pub(crate) const MAX_PUT_READS: u32 = 15;

/// Writing a file to the card through the loader (ADR-0013): a FAT32 card
/// image with an old `retropane.bin`, a bigger new one sent over the
/// emulated USB port, the loader's OK, and the image checked on the host
/// afterwards: the new contents, and the other file untouched. Most of the
/// card is taken behind the FSInfo hint, so the free-cluster scan walks
/// ~90% of the FAT first (the stale-hint case that cost 3.7 s on the
/// device); the loader reports its read commands.
pub fn boot_loader_put(root: &Path) -> Result<(), String> {
    if !has_usb_model(root) {
        eprintln!("    file to card: skipped (this emulator has no USB controller model)");
        return Ok(());
    }
    let loader = raw_image(root, "rp-loader")?;
    let old: Vec<u8> = (0..3000u32).map(|i| (i * 7) as u8).collect();
    let new: Vec<u8> = (0..70_000u32).map(|i| (i * 13 + 5) as u8).collect();
    let script = b"set ramaddr 0xa8000000\r\n".to_vec();
    // Clusters 8..3600 of ~4,080 in use, the hint (cluster 5) before them.
    let taken: Vec<u32> = (8..3600).collect();
    let image = rp_fat::format::build(&rp_fat::format::Image {
        total_sectors: 32 * 1024, // 16 MiB
        sectors_per_cluster: 8,   // 4 KiB clusters, as on the owner's card
        files: &[
            rp_fat::format::Entry {
                name: "retropane.bin",
                data: &old,
            },
            rp_fat::format::Entry {
                name: "startup.txt",
                data: &script,
            },
        ],
        blocked_clusters: &taken,
        ballast: 0,
    });
    let sd = scratch(root, "sd-put.img")?;
    // A damaged first-FAT sector in a free region (sector 20: clusters
    // 2560-2687, all free): invalid entries, as on the owner's card. The
    // loader's `card repair` must restore it from the second FAT.
    let mut image = image;
    let fat1 = rp_fat::format::PARTITION_START + 32;
    let damaged = fat1 + 20;
    for (k, b) in image[damaged as usize * 512..damaged as usize * 512 + 512]
        .iter_mut()
        .enumerate()
    {
        *b = if k % 4 == 3 { 0x0A } else { 0x80 | k as u8 };
    }
    // ... with one entry landing in range by chance, as on the owner's card.
    let e5 = damaged as usize * 512 + 5 * 4;
    image[e5..e5 + 4].copy_from_slice(&0x0000_0123u32.to_le_bytes());
    std::fs::write(&sd, image).map_err(|e| format!("{}: {e}", sd.display()))?;
    let frame = rp_loadproto::file_frame("retropane.bin", &new);
    // A file the card doesn't have: created (long name, so the loader
    // writes long-name entries and a ~1 short name).
    let created: Vec<u8> = (0..6000u32).map(|i| (i * 5 + 3) as u8).collect();
    let create = rp_loadproto::file_frame("RetroPane loader.lnk", &created);
    let bad = rp_loadproto::file_frame("bad*name", b"x");
    let args = usb_args(2);
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let check = || -> Result<(), String> {
        let mut s = usb_connect(2)?;
        usb_read_until(&mut s, "RPLOAD1 READY", 10)?;
        // No CF card in this machine: the slot reads empty (CPLD 0x04 bit 1)
        // and the socket is never touched.
        s.write_all(&rp_loadproto::control_frame("cf"))
            .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL ERR cf: no card\r\n", 20)?;
        // SD DMA: reads and writes on after the start-up self-test (writes
        // since rp-storfast's device soak); `card dma writes` still answers;
        // the puts below write by DMA and are checked in the image afterwards.
        s.write_all(&rp_loadproto::control_frame("card dma"))
            .map_err(|e| e.to_string())?;
        usb_read_until(
            &mut s,
            "RPL OK dma reads on (self-test passed), writes on",
            20,
        )?;
        s.write_all(&rp_loadproto::control_frame("card dma writes"))
            .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "writes on", 20)?;
        s.write_all(&frame).map_err(|e| e.to_string())?;
        let reply = usb_read_line(&mut s, "RPL ", 40)?;
        let line = reply
            .lines()
            .find(|l| l.starts_with("RPL "))
            .unwrap_or("")
            .to_string();
        let theirs = line.rsplit("sha256 ").next().unwrap_or("").trim();
        crate::usb::check_digest(theirs, &new, "file on the card")?;
        if !line.starts_with("RPL OK 70000 bytes") || !line.contains("contiguous") {
            return Err(format!("unexpected reply to the file: {line:?}"));
        }
        eprintln!("    loader: {line}");
        // 35 read commands when the FAT was read one sector per command
        // (2026-10-11); reading it in runs, 12.
        let reads: u32 = line
            .split_once(" (")
            .and_then(|(_, r)| r.split_once(" reads)"))
            .and_then(|(n, _)| n.parse().ok())
            .ok_or_else(|| format!("no read count in {line:?}"))?;
        if reads > MAX_PUT_READS {
            return Err(format!(
                "the write took {reads} read commands (at most {MAX_PUT_READS}): is the FAT search reading sector by sector again?"
            ));
        }
        s.write_all(&create).map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL OK 6000 bytes", 40)?;
        s.write_all(&bad).map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL ERR bad name", 20)?;
        // Into folders that don't exist yet: created on the way.
        s.write_all(&rp_loadproto::file_frame(
            "Apps/Sub Folder/hello.txt",
            b"hello",
        ))
        .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL OK 5 bytes", 20)?;
        // Browse and fetch, as `cargo xtask ls` and `get` do.
        s.write_all(&rp_loadproto::control_frame("ls Apps/Sub Folder"))
            .map_err(|e| e.to_string())?;
        let text = usb_read_until(&mut s, "RPL OK 1 entries", 20)?;
        let (entries, _) = crate::usb::parse_query(&text, "ls")?;
        if entries != ["5 hello.txt"] {
            return Err(format!("ls: {entries:?}"));
        }
        s.write_all(&rp_loadproto::control_frame("get retropane.bin"))
            .map_err(|e| e.to_string())?;
        let text = usb_read_line(&mut s, "RPL OK 70000 bytes crc", 40)?;
        let (lines, reply) = crate::usb::parse_query(&text, "get")?;
        if crate::usb::decode_file(&lines, &reply)? != new {
            return Err("get: the file fetched isn't the one put".into());
        }
        s.write_all(&rp_loadproto::control_frame("get no such file"))
            .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL ERR", 20)?;
        // The check finds the FAT sector this card was given damaged.
        s.write_all(&rp_loadproto::control_frame("card check"))
            .map_err(|e| e.to_string())?;
        let reply = usb_read_line(&mut s, "RPL ", 40)?;
        if !reply.contains("RPL OK check PROBLEMS") || !reply.contains("FAT copies differ 1,") {
            return Err(format!(
                "card check didn't find the damaged FAT sector: {reply:?}"
            ));
        }
        // Scrub: a first-FAT sector is rewritten and verified; anything
        // without a mirror is refused.
        let fat1 = rp_fat::format::PARTITION_START + 32;
        s.write_all(&rp_loadproto::control_frame(&format!("card scrub {fat1}")))
            .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL OK scrub", 20)?;
        s.write_all(&rp_loadproto::control_frame("card scrub 5"))
            .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL ERR scrub: only first-FAT", 20)?;
        // Repair: the damaged sector is restored; a healthy one is left.
        s.write_all(&rp_loadproto::control_frame(&format!(
            "card repair {damaged}"
        )))
        .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL OK repair", 20)?;
        s.write_all(&rp_loadproto::control_frame(&format!("card repair {fat1}")))
            .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "already matches", 20)?;
        // Peek: the boot sector's first bytes, read-only.
        s.write_all(&rp_loadproto::control_frame("card peek 2048 0"))
            .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL OK peek 2048+0: eb 58 90", 20)?;
        // Deleting: a file put for the purpose; the boot files refused;
        // a folder with something in it refused.
        s.write_all(&rp_loadproto::file_frame("remove me.txt", b"bye"))
            .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL OK 3 bytes", 20)?;
        s.write_all(&rp_loadproto::control_frame("rm remove me.txt"))
            .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL OK removed remove me.txt", 20)?;
        s.write_all(&rp_loadproto::control_frame("rm RetroPane.bin"))
            .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL ERR rm: that file boots the device", 20)?;
        s.write_all(&rp_loadproto::control_frame("rm Apps"))
            .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL ERR rm: folder not empty", 20)?;
        // Repaired, and after every write: clean.
        s.write_all(&rp_loadproto::control_frame("card check"))
            .map_err(|e| e.to_string())?;
        let reply = usb_read_line(&mut s, "RPL ", 40)?;
        if !reply.contains("RPL OK check clean") {
            return Err(format!("card check after the repair: {reply:?}"));
        }
        // Long file-system work keeps the watchdog fed as it reads (found
        // on the device: a 4 GB card check outlasted the 30 s run limit).
        s.write_all(&rp_loadproto::control_frame("drive status"))
            .map_err(|e| e.to_string())?;
        let status = usb_read_until(&mut s, "%\r\n", 10)?;
        let kept: u32 = status
            .split("kept alive ")
            .nth(1)
            .and_then(|t| t.split(',').next())
            .and_then(|n| n.trim().parse().ok())
            .ok_or_else(|| format!("no keep-alive count in {status:?}"))?;
        if kept < 100 {
            return Err(format!(
                "card commands didn't keep the watchdog fed: {kept}"
            ));
        }
        eprintln!("    keep-alive OK: card commands fed the watchdog at {kept} block reads");
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
            timeout: Duration::from_secs(90),
        },
    )?;
    let got = card_file(&sd, "retropane.bin")?;
    if got != new {
        return Err(format!(
            "the card holds {} bytes of retropane.bin after the put, not the {} sent",
            got.len(),
            new.len()
        ));
    }
    if card_file(&sd, "startup.txt")? != script {
        return Err("startup.txt changed on the card".into());
    }
    let after = std::fs::read(&sd).map_err(|e| e.to_string())?;
    let fat_sectors = {
        let b = &after[rp_fat::format::PARTITION_START as usize * 512..];
        u32::from_le_bytes([b[36], b[37], b[38], b[39]])
    };
    let one = &after[damaged as usize * 512..damaged as usize * 512 + 512];
    let two = &after
        [(damaged + fat_sectors) as usize * 512..(damaged + fat_sectors) as usize * 512 + 512];
    if one != two {
        return Err("card repair: the first FAT sector still differs from its mirror".into());
    }
    if card_file(&sd, "remove me.txt").is_ok() {
        return Err("rm: the removed file is still on the card".into());
    }
    if card_file(&sd, "Apps/Sub Folder/hello.txt")? != b"hello" {
        return Err("the file in new folders isn't on the card".into());
    }
    if card_file(&sd, "RetroPane loader.lnk")? != created
        || card_file(&sd, "RETROP~1.LNK")? != created
    {
        return Err("the created file isn't on the card (long or short name)".into());
    }
    eprintln!("    file to card OK: replaced through the loader, verified in the image");
    Ok(())
}

/// A USB Mass Storage host over the UDC model's framed chardev: each OUT
/// transfer goes as a 4-byte length and its bytes, each IN packet comes as
/// a 1-byte length and its bytes (ADR-0014).
pub(crate) struct Bot {
    pub(crate) s: std::net::TcpStream,
    pub(crate) tag: u32,
    /// The logical unit commands go to (0: SD card, 1: CF card).
    pub(crate) lun: u8,
    /// How long a command may take before it counts as no answer (a
    /// hang detector, not a measurement: the guest's own time decides).
    pub(crate) wait: Duration,
}

/// One command's outcome.
pub(crate) struct BotReply {
    pub(crate) data: Vec<u8>,
    pub(crate) status: u8,
    pub(crate) residue: u32,
}

impl Bot {
    pub(crate) fn connect(n: u16) -> Result<Self, String> {
        Ok(Self {
            s: usb_connect(n)?,
            tag: 1,
            lun: 0,
            wait: Duration::from_secs(20),
        })
    }

    pub(crate) fn send_transfer(&mut self, data: &[u8]) -> Result<(), String> {
        let mut f = (data.len() as u32).to_le_bytes().to_vec();
        f.extend_from_slice(data);
        self.s.write_all(&f).map_err(|e| e.to_string())
    }

    pub(crate) fn read_exact(&mut self, buf: &mut [u8], deadline: Instant) -> Result<(), String> {
        let mut got = 0;
        while got < buf.len() {
            if Instant::now() > deadline {
                return Err("drive: no answer".into());
            }
            match self.s.read(&mut buf[got..]) {
                Ok(0) => return Err("drive: connection closed".into()),
                Ok(n) => got += n,
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        Ok(())
    }

    pub(crate) fn packet(&mut self, deadline: Instant) -> Result<Vec<u8>, String> {
        let mut n = [0u8; 1];
        self.read_exact(&mut n, deadline)?;
        let mut p = vec![0u8; usize::from(n[0])];
        self.read_exact(&mut p, deadline)?;
        Ok(p)
    }

    /// Sends a raw transfer that isn't a command (hostile-input tests).
    pub(crate) fn junk(&mut self, data: &[u8]) -> Result<(), String> {
        self.send_transfer(data)
    }

    pub(crate) fn command(
        &mut self,
        cb: &[u8],
        len: u32,
        dir_in: bool,
        out: &[u8],
    ) -> Result<BotReply, String> {
        self.tag = self.tag.wrapping_add(1);
        let mut c = vec![0u8; 31];
        c[..4].copy_from_slice(b"USBC");
        c[4..8].copy_from_slice(&self.tag.to_le_bytes());
        c[8..12].copy_from_slice(&len.to_le_bytes());
        c[12] = if dir_in { 0x80 } else { 0 };
        c[13] = self.lun;
        c[14] = cb.len() as u8;
        c[15..15 + cb.len()].copy_from_slice(cb);
        self.send_transfer(&c)?;
        if !out.is_empty() {
            self.send_transfer(out)?;
        }
        let deadline = Instant::now() + self.wait;
        let mut data = Vec::new();
        loop {
            let p = self.packet(deadline)?;
            if p.len() == 13 && &p[..4] == b"USBS" && p[4..8] == self.tag.to_le_bytes() {
                return Ok(BotReply {
                    data,
                    status: p[12],
                    residue: u32::from_le_bytes([p[8], p[9], p[10], p[11]]),
                });
            }
            data.extend_from_slice(&p);
        }
    }

    /// A fresh host's mount: READ CAPACITY first (as Windows, Linux and
    /// macOS mount), until the unit answers (a change reported once, or a
    /// card still being attached), so the drive takes the view as current.
    pub(crate) fn mount(&mut self) -> Result<u32, String> {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let r = self.command(&[0x25, 0, 0, 0, 0, 0, 0, 0, 0, 0], 8, true, &[])?;
            if r.status == 0 && r.data.len() == 8 {
                return Ok(u32::from_be_bytes([r.data[0], r.data[1], r.data[2], r.data[3]]) + 1);
            }
            let _ = self.sense()?;
            if Instant::now() > deadline {
                return Err(format!(
                    "drive: unit {} never answered READ CAPACITY",
                    self.lun
                ));
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    pub(crate) fn sense(&mut self) -> Result<(u8, u8, u8), String> {
        let r = self.command(&[0x03, 0, 0, 0, 18, 0], 18, true, &[])?;
        if r.data.len() < 14 {
            return Err(format!("drive: REQUEST SENSE gave {} bytes", r.data.len()));
        }
        Ok((r.data[2], r.data[12], r.data[13]))
    }
}

pub(crate) fn rw10(op: u8, lba: u32, count: u16) -> [u8; 10] {
    let mut cb = [0u8; 10];
    cb[0] = op;
    cb[2..6].copy_from_slice(&lba.to_be_bytes());
    cb[7..9].copy_from_slice(&count.to_be_bytes());
    cb
}

impl rp_hal_api::BlockDevice for Bot {
    fn read_blocks(&mut self, lba: u32, buf: &mut [u8]) -> Result<(), rp_hal_api::BlockError> {
        for (i, chunk) in buf.chunks_mut(64 * 512).enumerate() {
            let at = lba + (i * 64) as u32;
            let n = (chunk.len() / 512) as u16;
            let r = self
                .command(&rw10(0x28, at, n), chunk.len() as u32, true, &[])
                .map_err(|_| rp_hal_api::BlockError::Device)?;
            if r.status != 0 || r.data.len() != chunk.len() {
                return Err(rp_hal_api::BlockError::Device);
            }
            chunk.copy_from_slice(&r.data);
        }
        Ok(())
    }
    fn write_blocks(&mut self, lba: u32, buf: &[u8]) -> Result<(), rp_hal_api::BlockError> {
        for (i, chunk) in buf.chunks(64 * 512).enumerate() {
            let at = lba + (i * 64) as u32;
            let n = (chunk.len() / 512) as u16;
            let r = self
                .command(&rw10(0x2A, at, n), chunk.len() as u32, false, chunk)
                .map_err(|_| rp_hal_api::BlockError::Device)?;
            if r.status != 0 {
                // As a PC does: ask why. DATA PROTECT is "write-protected".
                return Err(match self.sense() {
                    Ok((0x07, 0x27, 0x00)) => rp_hal_api::BlockError::ReadOnly,
                    _ => rp_hal_api::BlockError::Device,
                });
            }
        }
        Ok(())
    }
}

/// After the device changed the medium: polls like Windows (TEST UNIT
/// READY every 250 ms) and needs the medium to go away ("no medium" at
/// least once: Windows drops its cached view only then, item 65), then come
/// back with "medium changed" exactly once, then be ready.
pub(crate) fn expect_away_then_back(bot: &mut Bot, after: &str) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut seq = Vec::new();
    loop {
        let r = bot.command(&[0; 6], 0, false, &[])?;
        let s = if r.status == 0 {
            (0, 0, 0)
        } else {
            bot.sense()?
        };
        if seq.last() != Some(&s) {
            seq.push(s);
        }
        if r.status == 0 {
            break;
        }
        if Instant::now() > deadline {
            return Err(format!("drive after {after}: never ready again ({seq:x?})"));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    if seq != [(0x02, 0x3A, 0x00), (0x06, 0x28, 0x00), (0, 0, 0)] {
        return Err(format!(
            "drive after {after}: wanted away, medium changed, ready; got {seq:x?}"
        ));
    }
    Ok(())
}

/// The attribute byte of the short entry `name` (11 bytes, 8.3 padded) in
/// the FAT12/16 volume at `start`'s fixed root directory, read raw through
/// `dev` (the PC's view when `dev` is the drive).
pub(crate) fn fixed_root_attr(
    dev: &mut dyn rp_hal_api::BlockDevice,
    start: u32,
    name: &[u8; 11],
) -> Option<u8> {
    let mut b = [0u8; 512];
    dev.read_blocks(start, &mut b).ok()?;
    let le16 = |o: usize| u32::from(u16::from_le_bytes([b[o], b[o + 1]]));
    let root = start + le16(14) + u32::from(b[16]) * le16(22);
    let sectors = (le16(17) * 32).div_ceil(512);
    let mut s = [0u8; 512];
    for k in 0..sectors {
        dev.read_blocks(root + k, &mut s).ok()?;
        if let Some(e) = s.chunks(32).find(|e| &e[..11] == name) {
            return Some(e[11]);
        }
    }
    None
}

/// After a write the guard refused, the drive stays as it was: ready, no
/// medium change (the owner, 2026-10-25: no blink, nothing that looks
/// broken; ruling A's away-and-back withdrawn). Watched for longer than
/// the loader's old quiet time and the PC's write grace together.
pub(crate) fn expect_no_blink_after_refusal(bot: &mut Bot, after: &str) -> Result<(), String> {
    let until = Instant::now() + Duration::from_secs(3);
    while Instant::now() < until {
        let r = bot.command(&[0; 6], 0, false, &[])?;
        if r.status != 0 {
            let s = bot.sense()?;
            return Err(format!(
                "drive after {after}: went away ({s:x?}): no blink wanted"
            ));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Ok(())
}

/// The first block of a FAT32 volume's root directory, from its boot
/// sector (read through the drive, as the PC would).
pub(crate) fn root_dir_lba(bot: &mut Bot) -> Result<u32, String> {
    let ps = rp_fat::format::PARTITION_START;
    let mut b = [0u8; 512];
    rp_hal_api::BlockDevice::read_blocks(bot, ps, &mut b)
        .map_err(|_| "drive: boot sector unreadable".to_string())?;
    let le16 = |o: usize| u32::from(u16::from_le_bytes([b[o], b[o + 1]]));
    let le32 = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    let (spc, reserved, fats, fat_size, root) = (
        u32::from(b[13]),
        le16(14),
        u32::from(b[16]),
        le32(36),
        le32(44),
    );
    Ok(ps + reserved + fats * fat_size + (root - 2) * spc)
}

/// `cargo xtask emu-stale`: the stale-view check on its own.
pub fn boot_loader_stale_view_alone(root: &Path) -> Result<(), String> {
    let dir = emu_dir(root)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    crate::sdkit::raw_binary(
        &crate::arm_release(root).join("rp-loader"),
        &dir.join("rp-loader.bin"),
    )?;
    boot_loader_stale_view(root)
}

/// The stale-view class (noise-ri.bin lost, 2026-10-15): the PC writes the
/// card through the drive, then something writes it while the loader isn't
/// running (here the crash report the loader keeps after rp-crash; on the
/// device an image such as rp-noise, or Windows Mobile). The X51v's pull-up
/// is hard-wired, so Windows can re-attach without noticing the device went
/// and keep its cached root directory. Its first write after the loader is
/// back (that old root directory) must be refused with the card untouched,
/// the drive must go away and come back changed, and after re-reading, the
/// PC's next file must leave every entry in place: the PC's first file, the
/// crash report and the new one, checked in the card image.
pub fn boot_loader_stale_view(root: &Path) -> Result<(), String> {
    if !has_msc_model(root) {
        eprintln!("    stale view: skipped (this emulator's USB model has no mass storage bridge)");
        return Ok(());
    }
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    let crash = dir.join("rp-crash.bin");
    crate::sdkit::raw_binary(&release.join("rp-crash"), &crash)?;
    let payload = std::fs::read(&crash).map_err(|e| format!("{}: {e}", crash.display()))?;
    let frame = rp_loadproto::frame(&payload);
    let loader = dir.join("rp-loader.bin");
    let image = rp_fat::format::build(&rp_fat::format::Image {
        total_sectors: 32 * 1024,
        sectors_per_cluster: 8,
        files: &[],
        blocked_clusters: &[],
        ballast: 0,
    });
    let sd = scratch(root, "sd-stale.img")?;
    std::fs::write(&sd, &image).map_err(|e| format!("{}: {e}", sd.display()))?;
    let before = b"written by the PC before the image ran\r\n";
    let after = b"written by the PC after the loader came back\r\n";
    let mut args: Vec<String> = usb_args(14).to_vec();
    args.extend(msc_args(15));
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let check = || -> Result<(), String> {
        let mut bot = Bot::connect(15)?;
        let mut s = usb_connect(14)?;
        usb_read_until(&mut s, "RPLOAD1 READY", 10)?;
        bot.mount()?;
        let mut scratch = vec![0u8; 32 * 1024];
        {
            let mut vol = rp_fat::Volume::mount(&mut bot, rp_fat::format::PARTITION_START)
                .map_err(|e| format!("stale view: mount: {}", e.name()))?;
            vol.put("pc-before.txt", before, &mut scratch)
                .map_err(|e| format!("stale view: put: {}", e.name()))?;
        }
        // The root directory as the PC caches it.
        let root_lba = root_dir_lba(&mut bot)?;
        let mut stale = vec![0u8; 512];
        rp_hal_api::BlockDevice::read_blocks(&mut bot, root_lba, &mut stale)
            .map_err(|_| "stale view: root directory unreadable".to_string())?;
        // An image that crashes: the loader comes back and keeps the
        // report on the card before it offers the drive again.
        s.write_all(&frame).map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL OK sha256 ", 15)?;
        usb_read_until(&mut s, "RPLOAD1 READY", 60)?;
        // The PC kept its view and writes its old root directory back.
        let r = bot.command(&rw10(0x2A, root_lba, 1), 512, false, &stale)?;
        if r.status == 0 {
            return Err(
                "stale view: the PC's root directory from before the image was written back (the crash report's entry is lost)"
                    .into(),
            );
        }
        let sense = bot.sense()?;
        if sense != (0x02, 0x3A, 0x00) {
            return Err(format!(
                "stale view: the old view's write failed with {sense:x?}, not \"no medium\""
            ));
        }
        expect_away_then_back(&mut bot, "the image's run")?;
        // Re-read (a fresh view): the report is there; the next file
        // goes next to it.
        bot.mount()?;
        let report = drive_file(&mut bot, "crashlog.txt")?;
        if report.is_empty() {
            return Err("stale view: crashlog.txt is empty".into());
        }
        let mut vol = rp_fat::Volume::mount(&mut bot, rp_fat::format::PARTITION_START)
            .map_err(|e| format!("stale view: mount after: {}", e.name()))?;
        vol.put("pc-after.txt", after, &mut scratch)
            .map_err(|e| format!("stale view: put after: {}", e.name()))?;
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
            timeout: Duration::from_secs(90),
        },
    )?;
    for (name, want) in [("pc-before.txt", &before[..]), ("pc-after.txt", &after[..])] {
        if card_file(&sd, name)? != want {
            return Err(format!("stale view: {name} differs in the card image"));
        }
    }
    if card_file(&sd, "crashlog.txt")?.is_empty() {
        return Err("stale view: crashlog.txt lost from the card image".into());
    }
    eprintln!(
        "    stale view OK: after the image's run (crash report kept on the card) the PC's old root directory was refused with no medium, the drive went away and came back changed, and the PC's files and the report all stayed"
    );
    Ok(())
}

/// Reads `name` from the card through the drive.
pub(crate) fn drive_file(bot: &mut Bot, name: &str) -> Result<Vec<u8>, String> {
    let mut vol = rp_fat::Volume::mount(&mut *bot, rp_fat::format::PARTITION_START)
        .map_err(|e| format!("drive: mount: {}", e.name()))?;
    let f = vol
        .find(name)
        .map_err(|e| format!("drive: {name}: {}", e.name()))?;
    let mut out = Vec::new();
    let mut scratch = vec![0u8; 32 * 1024];
    vol.read_file(&f, &mut scratch, |c| out.extend_from_slice(c))
        .map_err(|e| format!("drive: {name}: {}", e.name()))?;
    Ok(out)
}

/// The SD card as a USB drive (ADR-0014), end to end in the emulator: the
/// loader's mass storage interface next to its serial port; the test mounts
/// the FAT32 card image through the drive with `rp-fat`, reads a file,
/// replaces another (copy-then-swap through SCSI writes), checks that a
/// serial `put` is refused while the PC writes and accepted after, that the
/// drive then reports the medium changed, and that malformed commands fail
/// cleanly. The image on the host is checked afterwards.
pub fn boot_loader_drive(root: &Path) -> Result<(), String> {
    if !has_msc_model(root) {
        eprintln!("    drive: skipped (this emulator's USB model has no mass storage bridge)");
        return Ok(());
    }
    let loader = raw_image(root, "rp-loader")?;
    let old: Vec<u8> = (0..5000u32).map(|i| (i * 3) as u8).collect();
    let script = b"set ramaddr 0xa8000000\r\n".to_vec();
    let image = rp_fat::format::build(&rp_fat::format::Image {
        total_sectors: 32 * 1024,
        sectors_per_cluster: 8,
        files: &[
            rp_fat::format::Entry {
                name: "retropane.bin",
                data: &old,
            },
            rp_fat::format::Entry {
                name: "startup.txt",
                data: &script,
            },
        ],
        blocked_clusters: &[],
        ballast: 0,
    });
    let blocks = (image.len() / 512) as u32;
    let sd = scratch(root, "sd-drive.img")?;
    std::fs::write(&sd, &image).map_err(|e| format!("{}: {e}", sd.display()))?;
    let via_drive = b"written through the USB drive\r\n".to_vec();
    let via_serial: Vec<u8> = (0..9000u32).map(|i| (i * 11 + 1) as u8).collect();
    let put = rp_loadproto::file_frame("retropane.bin", &via_serial);
    let mut args: Vec<String> = usb_args(5).to_vec();
    args.extend(msc_args(6));
    let slow = device_has(root, "pxa2xx-mmci", "slow-address");
    if slow {
        // The test image is a standard-capacity card: byte addresses.
        args.extend([
            "-global".into(),
            format!("pxa2xx-mmci.slow-address={}", SLOW_BLOCK * 512),
        ]);
    }
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let check = || -> Result<(), String> {
        let mut bot = Bot::connect(6)?;
        let mut serial = usb_connect(5)?;
        usb_read_until(&mut serial, "RPLOAD1 READY", 10)?;
        bot.mount()?;
        // The drive becomes ready.
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let r = bot.command(&[0; 6], 0, false, &[])?;
            if r.status == 0 {
                break;
            }
            let _ = bot.sense()?;
            if Instant::now() > deadline {
                return Err("drive: never became ready".into());
            }
        }
        let r = bot.command(&[0x12, 0, 0, 0, 36, 0], 36, true, &[])?;
        if r.data.len() != 36 || &r.data[8..16] != b"RetroPan" || r.data[1] & 0x80 == 0 {
            return Err(format!("drive: INQUIRY {:?}", r.data));
        }
        let r = bot.command(&[0x25, 0, 0, 0, 0, 0, 0, 0, 0, 0], 8, true, &[])?;
        let last = u32::from_be_bytes([r.data[0], r.data[1], r.data[2], r.data[3]]);
        if last + 1 != blocks {
            return Err(format!(
                "drive: capacity {} blocks, image {blocks}",
                last + 1
            ));
        }
        if drive_file(&mut bot, "retropane.bin")? != old {
            return Err("drive: retropane.bin read through the drive differs".into());
        }
        // Writable by default (ADR-0014, the boot-file guard): MODE SENSE
        // says so, but the PC can't change what boots the device.
        let write_protected = |bot: &mut Bot| -> Result<bool, String> {
            let r = bot.command(&[0x1A, 0, 0x3F, 0, 255, 0], 255, true, &[])?;
            Ok(r.data.get(2).is_some_and(|b| b & 0x80 != 0))
        };
        if write_protected(&mut bot)? {
            return Err("drive: write-protected at start (should be guarded, writable)".into());
        }
        let refuses_boot_file = |bot: &mut Bot, why: &str| -> Result<(), String> {
            let mut vol = rp_fat::Volume::mount(&mut *bot, rp_fat::format::PARTITION_START)
                .map_err(|e| format!("drive: mount: {}", e.name()))?;
            let mut scratch = vec![0u8; 32 * 1024];
            // Refused as write-protected, not failed some other way.
            let protected = Err(rp_fat::Error::Device(rp_hal_api::BlockError::ReadOnly));
            let replaced = vol
                .replace("retropane.bin", b"not a loader", &mut scratch)
                .map(|_| ());
            let removed = vol.remove("startup.txt");
            if replaced != protected || removed != protected {
                return Err(format!(
                    "drive: boot files not protected ({why}): replace {replaced:?}, remove {removed:?}"
                ));
            }
            Ok(())
        };
        refuses_boot_file(&mut bot, "at start")?;
        // The owner's ruling A (2026-10-25): after a refused write the drive
        // goes away and comes back, so the PC drops the view it changed.
        expect_no_blink_after_refusal(&mut bot, "a write the guard refused")?;
        if drive_file(&mut bot, "retropane.bin")? != old {
            return Err("drive: a refused write changed retropane.bin".into());
        }
        // `drive ro` locks it all, `drive rw` gives the guarded drive back.
        for (cmd, reply, wp) in [
            ("drive ro", "RPL OK drive read-only", true),
            ("drive rw", "RPL OK drive read-write", false),
        ] {
            serial
                .write_all(&rp_loadproto::control_frame(cmd))
                .map_err(|e| e.to_string())?;
            usb_read_until(&mut serial, reply, 10)?;
            expect_away_then_back(&mut bot, cmd)?;
            if write_protected(&mut bot)? != wp {
                return Err(format!("drive: protection wrong after {cmd}"));
            }
            if wp {
                let r = bot.command(&rw10(0x2A, 100, 1), 512, false, &[0; 512])?;
                if r.status != 1 || bot.sense()? != (0x07, 0x27, 0x00) {
                    return Err("drive: a write was accepted while read-only".into());
                }
            }
        }
        guest_hold(
            &mut crate::screen::Monitor::connect(monitor_port())?,
            2500,
            "drive",
        )?; // the write grace
        // Write a new file through the drive (the PC's side).
        {
            let mut vol = rp_fat::Volume::mount(&mut bot, rp_fat::format::PARTITION_START)
                .map_err(|e| format!("drive: mount: {}", e.name()))?;
            let mut scratch = vec![0u8; 32 * 1024];
            vol.put("notes.txt", &via_drive, &mut scratch)
                .map_err(|e| format!("drive: put: {}", e.name()))?;
        }
        // The device refuses to write while the PC just wrote...
        serial.write_all(&put).map_err(|e| e.to_string())?;
        let r = usb_read_line(&mut serial, "RPL ", 20)?;
        if !r.contains("RPL ERR card busy") {
            return Err(format!("serial put during a drive write: {r:?}"));
        }
        // ... and accepts once the PC has been quiet for the grace period.
        guest_hold(
            &mut crate::screen::Monitor::connect(monitor_port())?,
            2500,
            "drive",
        )?;
        serial.write_all(&put).map_err(|e| e.to_string())?;
        let r = usb_read_until(&mut serial, "RPL OK", 30)?;
        let _ = r;
        // The medium goes away until the PC has noticed, then comes back
        // changed: the PC's view then matches the card.
        expect_away_then_back(&mut bot, "the device's put")?;
        if drive_file(&mut bot, "retropane.bin")? != via_serial {
            return Err("drive: doesn't show the file the device wrote".into());
        }
        if drive_file(&mut bot, "notes.txt")? != via_drive {
            return Err("drive: lost the file it wrote".into());
        }
        // The guard follows the device's own write: the new loader's
        // clusters are protected now.
        guest_hold(
            &mut crate::screen::Monitor::connect(monitor_port())?,
            2500,
            "drive",
        )?;
        refuses_boot_file(&mut bot, "after the device replaced it")?;
        expect_no_blink_after_refusal(&mut bot, "a write the guard refused (again)")?;
        if drive_file(&mut bot, "retropane.bin")? != via_serial {
            return Err("drive: a refused write changed the new retropane.bin".into());
        }
        // Hostile input: junk transfers are ignored, an out-of-range and a
        // wrapping read fail with sense, and the drive keeps working.
        bot.junk(&[0x55; 31])?;
        bot.junk(b"USBC")?;
        let r = bot.command(&rw10(0x28, blocks, 1), 512, true, &[])?;
        if r.status != 1 || bot.sense()? != (0x05, 0x21, 0x00) {
            return Err("drive: out-of-range read not refused".into());
        }
        let r = bot.command(&rw10(0x28, u32::MAX, 2), 1024, true, &[])?;
        if r.status != 1 || r.residue != 1024 {
            return Err("drive: wrapping read not refused".into());
        }
        let r = bot.command(&[0; 6], 0, false, &[])?;
        if r.status != 0 {
            return Err("drive: not ready after hostile input".into());
        }
        // Throughput (emulator wall clock; the device's number is the real one).
        // It crosses the slow block: the card driver must fall back to
        // single blocks and still return exactly the card's bytes.
        let t = Instant::now();
        // Across the slow block, block by block on a loaded PC: more than
        // the usual 20 s may pass (R42, 2026-10-25), still progress.
        bot.wait = Duration::from_secs(120);
        let r = bot.command(&rw10(0x28, 0, 2048), 2048 * 512, true, &[]);
        bot.wait = Duration::from_secs(20);
        let r = r?;
        if r.status != 0 || r.data.len() != 2048 * 512 {
            return Err(format!(
                "drive: 1 MiB read across the slow block {SLOW_BLOCK} failed (status {})",
                r.status
            ));
        }
        if r.data != image[..2048 * 512] {
            return Err("drive: the 1 MiB read doesn't match the card image".into());
        }
        eprintln!(
            "    drive: 1 MiB read in {} ms through the emulated drive",
            t.elapsed().as_millis()
        );
        // About 3 s on an idle PC (a loader napping mid-transfer took 17),
        // but the wall clock is the PC's, not the loader's: the loader's
        // own count of naps mid-command is checked below instead.
        // Item 58: sequential reads, as a PC copying a file makes them: the
        // second starts where the first ended, so its first chunk was read
        // ahead (by DMA, while the first one's last chunk went out). Both
        // must still be exactly the card's bytes.
        for at in [4096u32, 4096 + 128] {
            let r = bot.command(&rw10(0x28, at, 128), 128 * 512, true, &[])?;
            let want = &image[at as usize * 512..(at as usize + 128) * 512];
            if r.status != 0 || r.data != want {
                return Err(format!(
                    "drive: sequential read at {at} doesn't match the card"
                ));
            }
        }
        serial
            .write_all(&rp_loadproto::control_frame("drive status"))
            .map_err(|e| e.to_string())?;
        let status = usb_read_until(&mut serial, "%\r\n", 10)?;
        let ahead: u32 = status
            .split("read ahead ")
            .nth(1)
            .and_then(|t| t.split(',').next())
            .and_then(|n| n.trim().parse().ok())
            .ok_or_else(|| format!("no read-ahead count in {status:?}"))?;
        // A freshly formatted card: Windows' dirty flag clear as found and
        // after the drive work (the emulated PC isn't Windows).
        if !status.contains("dirty flag found clear now clear") {
            return Err(format!("drive: dirty flag not read as clear: {status:?}"));
        }
        if ahead == 0 {
            return Err(format!(
                "drive: sequential reads found nothing read ahead: {status:?}"
            ));
        }
        if !status.contains("naps mid-command 0,") {
            return Err(format!(
                "drive: the loader napped with a drive command under way: {status:?}"
            ));
        }
        eprintln!("    drive read-ahead OK: {ahead} reads found their first chunk already read");
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
            timeout: Duration::from_secs(120),
        },
    )?;
    if card_file(&sd, "notes.txt")? != via_drive || card_file(&sd, "retropane.bin")? != via_serial {
        return Err("drive: the card image doesn't hold both writes".into());
    }
    eprintln!(
        "    drive OK: the card mounted over USB mass storage, files read and written both ways, ownership rules and hostile input checked"
    );
    Ok(())
}

/// The loader's USB layout fallback: a host that refuses composite devices
/// (the model's `accept-composite=off`) never configures the serial port
/// plus drive; the loader must offer the serial-only layout by itself
/// within its patience (25 s) and the PC then reaches it as before.
pub fn boot_loader_fallback(root: &Path) -> Result<(), String> {
    let help = match emulator(root) {
        Ok(Emulator::Native(exe)) => Command::new(exe)
            .args(["-M", "x51v", "-device", "pxa27x-udc,help"])
            .output(),
        Ok(Emulator::Wsl) => {
            wsl_bash(&format!("{} -M x51v -device pxa27x-udc,help", wsl_qemu())).output()
        }
        Err(e) => return Err(e),
    };
    if !help.is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("accept-composite")) {
        eprintln!(
            "    USB layout fallback: skipped (this emulator's host can't refuse composites)"
        );
        return Ok(());
    }
    let loader = raw_image(root, "rp-loader")?;
    let mut args: Vec<String> = usb_args(7).to_vec();
    args.extend(["-global".into(), "pxa27x-udc.accept-composite=off".into()]);
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let check = || -> Result<(), String> {
        let mut s = usb_connect(7)?;
        usb_read_until(&mut s, "RPLOAD1 READY", 60)?;
        // And it answers commands there (2026-10-24: on the X51v the
        // fallback enumerated and opened, and no command got a reply).
        s.write_all(&rp_loadproto::control_frame("handoff"))
            .map_err(|e| e.to_string())?;
        let reply = usb_read_line(&mut s, "RPL ", 20)?;
        if !reply.contains("RPL OK handoff") {
            return Err(format!(
                "serial-only layout: no reply to a command: {reply:?}"
            ));
        }
        Ok(())
    };
    // A READY over USB from a host that refuses composites can only come
    // through the serial-only layout: that is the switch.
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
            timeout: Duration::from_secs(90),
        },
    )?;
    eprintln!(
        "    USB layout fallback OK: a host refusing composites reached the loader serial-only"
    );
    Ok(())
}

/// The SD card's lock switch: a card the emulator inserts read-only
/// (`readonly=on`, which the X51v model shows on GPIO27) must refuse a
/// file from the PC, and the drive must report it write-protected.
pub fn boot_loader_locked(root: &Path) -> Result<(), String> {
    if !has_usb_model(root) {
        eprintln!("    locked card: skipped (no USB model)");
        return Ok(());
    }
    let loader = raw_image(root, "rp-loader")?;
    let image = rp_fat::format::build(&rp_fat::format::Image {
        total_sectors: 32 * 1024,
        sectors_per_cluster: 8,
        files: &[rp_fat::format::Entry {
            name: "a.bin",
            data: b"old",
        }],
        blocked_clusters: &[],
        ballast: 0,
    });
    let sd = scratch(root, "sd-locked.img")?;
    std::fs::write(&sd, &image).map_err(|e| format!("{}: {e}", sd.display()))?;
    if !device_has(root, "pxa2xx-mmci", "locked") {
        eprintln!("    locked card: skipped (this emulator has no SD lock switch)");
        return Ok(());
    }
    // The lock switch is the slot's (a tab the host reads); the card itself
    // stays writable, as on real hardware, so a loader that ignored the
    // switch would change the image.
    let mut args: Vec<String> = usb_args(8).to_vec();
    args.extend(["-global".into(), "pxa2xx-mmci.locked=on".into()]);
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let put = rp_loadproto::file_frame("a.bin", b"new");
    let check = || -> Result<(), String> {
        let mut s = usb_connect(8)?;
        usb_read_until(&mut s, "RPLOAD1 READY", 10)?;
        s.write_all(&put).map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL ERR card is write-protected", 20)?;
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
            timeout: Duration::from_secs(60),
        },
    )?;
    if card_file(&sd, "a.bin")? != b"old" {
        return Err("locked card: the file changed".into());
    }
    eprintln!("    locked card OK: the lock switch refused the write, the card is unchanged");
    Ok(())
}
