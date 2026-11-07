//! The radar map kept on the card, checked in the emulator on FAT and exFAT card images (R184 step 3, audit C98: emu-today-radar-card).

#[allow(clippy::wildcard_imports)]
use super::*;

/// R184 step 3 (sheet docs/research/sheets/r184-radar-map-card.md): the
/// place's radar map kept on the card is shown with no dock. The card holds
/// the default icon pack and the build's map kept for the default place
/// (`RPMAPS/<name>.RPM`, its `RPMC` header as the image writes it); the
/// image opens Weather, then Radar; its run log must say the map came from
/// the card (read, checked and unpacked a layer a pass), the page that the
/// rain needs the dock (the emulator has no relay), and the card's file must
/// be left as it was (reading never writes).
pub fn today_radar_card(root: &Path) -> Result<(), String> {
    today_radar_card_on(root, false)?;
    // Audit C98: the same on an exFAT card (SDXC, or one formatted exFAT):
    // the kept map, the icon pack's parts, read through rp_fat::AnyVolume.
    today_radar_card_on(root, true)
}

/// An exFAT card image of the emulator's SD size holding `files` in its
/// root, and where its partition starts.
pub(crate) fn exfat_card_image(files: &[(String, Vec<u8>)]) -> Result<(Vec<u8>, u32), String> {
    let sectors = (SD_IMAGE_BYTES / 512) as u32;
    let o = rp_fat::mkfs::Options {
        fs: Some(rp_fat::mkfs::Fs::ExFat),
        cluster_bytes: Some(32 * 1024),
        align_sectors: None,
        label: "RPEXFAT",
        serial: 0x5250_4558,
    };
    let l = rp_fat::mkfs::plan(sectors, &o).map_err(|e| format!("exFAT card: {}", e.name()))?;
    let mut d = Disk(vec![0; SD_IMAGE_BYTES]);
    rp_fat::mkfs::write(&mut d, &l, &mut vec![0u8; 64 * 1024])
        .map_err(|e| format!("exFAT card: {}", e.name()))?;
    let mut v = rp_fat::AnyVolume::mount(d, l.part_start)
        .map_err(|e| format!("exFAT card: {}", e.name()))?;
    let mut sc = vec![0u8; 64 * 1024];
    for (name, data) in files {
        v.put(name, data, &mut sc)
            .map_err(|e| format!("exFAT card {name}: {}", e.name()))?;
    }
    Ok((v.device().0.clone(), l.part_start))
}

/// [`today_radar_card`] on a FAT card (`exfat` false) or an exFAT one.
pub(crate) fn today_radar_card_on(root: &Path, exfat: bool) -> Result<(), String> {
    use rp_weather::mapcard::{self, Key};
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    let loader = dir.join("rp-loader.bin");
    let bin = dir.join("rp-today.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-today"), &bin)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let map = today_output(&release, "radar.zz")?;
    let p = rp_weather::settings::Settings::DEFAULT.place();
    let key = Key {
        lat: p.lat,
        lon: p.lon,
        zoom: rp_weather::tiles::ZOOM,
    };
    let mut file = mapcard::head(key, &map, 1_793_457_000).to_vec();
    file.extend_from_slice(&map);
    // The icon pack's card when the build made it, else a bare one; on
    // exFAT the pack's parts put on as `cargo xtask load` names them.
    let (base, start) = if exfat {
        let parts: Vec<(String, Vec<u8>)> = match today_output(&release, "rp-icons0.pak") {
            Ok(pack) => {
                let mut v: Vec<(String, Vec<u8>)> = pack
                    .chunks(rp_today::PACK_PART)
                    .enumerate()
                    .map(|(p, c)| (format!("RPIC0{}.PAK", (b'A' + p as u8) as char), c.to_vec()))
                    .collect();
                v.push((
                    format!("RPIC0{}.PAK", (b'A' + v.len() as u8) as char),
                    Vec::new(),
                ));
                v
            }
            Err(_) => Vec::new(),
        };
        exfat_card_image(&parts)?
    } else {
        let img = match today_card(root, &release)? {
            Some(path) => std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?,
            None => rp_fat::format::build(&rp_fat::format::Image {
                total_sectors: (SD_IMAGE_BYTES / 512) as u32,
                sectors_per_cluster: 8,
                files: &[],
                blocked_clusters: &[],
                ballast: 0,
            }),
        };
        (img, rp_fat::format::PARTITION_START)
    };
    let kind = if exfat { "exFAT" } else { "FAT" };
    let mut vol = rp_fat::AnyVolume::mount(Disk(base), start)
        .map_err(|e| format!("card image: {}", e.name()))?;
    if matches!(vol, rp_fat::AnyVolume::ExFat(_)) != exfat {
        return Err(format!("radar card: the card image isn't {kind}"));
    }
    let path = key.path();
    let path = std::str::from_utf8(&path).map_err(|e| e.to_string())?;
    let mut fat_scratch = vec![0u8; 64 * 1024];
    vol.put(path, &file, &mut fat_scratch)
        .map_err(|e| format!("seeding the card: {}", e.name()))?;
    let sd = scratch(
        root,
        if exfat {
            "today-radar-card-exfat.img"
        } else {
            "today-radar-card.img"
        },
    )?;
    std::fs::write(&sd, &vol.device().0).map_err(|e| format!("{}: {e}", sd.display()))?;
    let wsl = matches!(emulator(root)?, Emulator::Wsl);
    let steps = || -> Result<(), String> {
        let hold = |ms: u32| {
            guest_hold(
                &mut crate::screen::Monitor::connect(monitor_port())?,
                ms,
                "radar card",
            )
        };
        let mut qmp = crate::screen::Qmp::connect(monitor_port() + 1)?;
        hold(8000)?;
        // Weather (the Today screen's weather), then its Radar tab.
        for (x, y) in [(240u16, 150u16), (420, 580)] {
            touch(&mut qmp, &Action::TouchDown(x, y))?;
            hold(150)?;
            touch(&mut qmp, &Action::TouchUp)?;
            hold(3000)?;
        }
        hold(4000)?;
        let shot = scratch(root, "today-radar-card.ppm")?;
        let guest = if wsl {
            to_wsl_path(&shot)
        } else {
            shot.display().to_string()
        };
        crate::screen::Monitor::connect(monitor_port())?.screendump(&guest, &shot)?;
        // Back to the loader (Home, Start, its last entry): the run log
        // comes to the PC with the loader's `RPL LOG`.
        qmp.key("f4", true)?;
        hold(150)?;
        qmp.key("f4", false)?;
        hold(2500)?;
        let start = rp_shell::START;
        let back = rp_shell::Shell::menu_entry(rp_shell::Shell::menu_entries(true) - 1);
        for (x, y) in [
            (start.x + start.w / 2, start.y + start.h / 2),
            (back.x + back.w / 2, back.y + back.h / 2),
        ] {
            touch(&mut qmp, &Action::TouchDown(x, y))?;
            hold(150)?;
            touch(&mut qmp, &Action::TouchUp)?;
            hold(2500)?;
        }
        Ok(())
    };
    let log = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: Some(&sd),
            steps: &[],
            extra: &["-icount", "shift=0"],
            input: Some((&rp_loadproto::frame(&payload), rp_today::MARKER)),
            during: Some(&steps),
            then: Some("RPL END"),
            timeout: Duration::from_secs(600),
        },
    )
    .unwrap_or_else(|e| e);
    let log_name = if exfat {
        "today-radar-card-exfat.txt"
    } else {
        "today-radar-card.txt"
    };
    let _ = std::fs::write(scratch(root, log_name)?, &log);
    if !log.contains("RPL LOG") {
        return Err(format!(
            "radar card ({kind}): the image never came back to the loader (target/emulator/{log_name})"
        ));
    }
    let mut wants = vec![
        "radar map: the place's on the card (",
        "radar map: the place's from the card, ",
        "(8 passes)",
    ];
    // R182: the clock from the card's time file (the FAT card carries one).
    if !exfat {
        wants.push("clock: the PC's time at the load");
    }
    for want in wants {
        if !log.contains(want) {
            return Err(format!(
                "radar card ({kind}): no {want:?} in the run log (target/emulator/{log_name})"
            ));
        }
    }
    if log.contains("radar map: the card's not used") || log.contains("RPNET LAYERS") {
        return Err(format!(
            "radar card ({kind}): the kept map wasn't used (target/emulator/{log_name})"
        ));
    }
    // Reading leaves the card's file as it was.
    let img = std::fs::read(&sd).map_err(|e| format!("{}: {e}", sd.display()))?;
    let mut vol = rp_fat::AnyVolume::mount(Disk(img), start)
        .map_err(|e| format!("card image after: {}", e.name()))?;
    let mut back = Vec::new();
    let mut small = vec![0u8; 4096];
    vol.read_range(path, 0, u64::MAX, &mut small, &mut |c| {
        back.extend_from_slice(c)
    })
    .map_err(|e| format!("card after: {}", e.name()))?;
    if back != file {
        return Err(format!(
            "radar card ({kind}): the kept map changed on the card"
        ));
    }
    eprintln!(
        "    radar card ({kind}): the place's map from the card with no dock (target/emulator/today-radar-card.ppm)"
    );
    Ok(())
}
