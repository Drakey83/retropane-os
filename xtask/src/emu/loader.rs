//! The loader's own checks and the images it runs on screen: boot timing, handover, CF detect, night light, display calibration, looks, screen measure, refresh, scene, crash reports, hangs and the run limit.

#[allow(clippy::wildcard_imports)]
use super::*;

/// Boots the resident loader the way HaRET does (a raw binary away from its
/// home, so it must move itself), sends it the first-light image over the
/// serial port, and checks that image runs: relocation, the protocol, the
/// CRC check and the hand-off, end to end.
pub fn boot_loader(root: &Path) -> Result<(), String> {
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    let loader = dir.join("rp-loader.bin");
    let light = dir.join("rp-light.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-light"), &light)?;
    let payload = std::fs::read(&light).map_err(|e| format!("{}: {e}", light.display()))?;
    let frame = rp_loadproto::frame(&payload);
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&frame, "RPL END")),
            during: None,
            then: None,
            timeout: BOOT_TIMEOUT,
        },
    )?;
    if !output.contains("RPL OK") {
        return Err("loader: no `RPL OK` before the image started".into());
    }
    check_boot(&output, &["boot_loader_ready"])?;
    // The run log the loader sent after first light returned to it.
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("loader: no run log after the image returned")?;
    if !log.contains("all steps OK") || !log.contains(LIGHT_MARKER) {
        return Err(format!("loader: the run log is incomplete:{log}"));
    }
    eprint!("    run log as the PC receives it:{log}");
    eprintln!(
        "    loader OK: moved itself home, took a {} byte image over serial, CRC checked, \
         started it, and sent its run log back when it returned",
        payload.len()
    );
    Ok(())
}

/// Hand-over facts (items 72 and 77): `rp-handover` under the loader, with
/// Windows Mobile's CPLD shadow table in RAM as at HaRET's hand-off. The
/// loader adopts the table first (its start), so the image finds
/// RetroPane's table with Windows Mobile's bits in it; the hand-off record
/// shows no DMA left running (the loader stops its own) and the 2700G
/// handed out with no fixed wait.
pub fn boot_loader_handover(root: &Path) -> Result<(), String> {
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let loader = dir.join("rp-loader.bin");
    let image = dir.join("rp-handover.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-handover"), &image)?;
    let payload = std::fs::read(&image).map_err(|e| format!("{}: {e}", image.display()))?;
    let frame = rp_loadproto::frame(&payload);
    let args = wm_cpld_table_args();
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &extra,
            input: Some((&frame, "RPL END")),
            during: None,
            then: None,
            timeout: BOOT_TIMEOUT,
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("handover: no run log after the image returned")?;
    for want in [
        "handoff: DMA running 00000000, stopped",
        "table @A8001320: 53435052",
        "shadow from RetroPane's table:",
        " 02=00FD",
        " 10=1000",
        " 12=000F",
        " 16=0003",
        " 18=0004",
        " 1C=000C",
        "HANDOVER-OK",
    ] {
        if !log.contains(want) {
            return Err(format!("handover: no {want:?} in the run log:{log}"));
        }
    }
    eprint!("    run log:{log}");
    eprintln!(
        "    hand-over OK: no fixed wait for the 2700G; the CPLD shadow adopted from Windows Mobile's table by the loader, then shared with the image"
    );
    Ok(())
}

/// The CF detect diagnostic (`rp-cfdetect`) under the loader: it reads the
/// three detect signals as found, with the slot off through its 30 s watch,
/// and powered again, logs each, and returns with the slot powered. (The
/// emulator's slot is empty and its GPIO13 works unpowered, so this checks
/// the image's run, not the device's answer: docs/hw/emulator.md.)
pub fn boot_loader_cfdetect(root: &Path) -> Result<(), String> {
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let loader = dir.join("rp-loader.bin");
    let image = dir.join("rp-cfdetect.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-cfdetect"), &image)?;
    let payload = std::fs::read(&image).map_err(|e| format!("{}: {e}", image.display()))?;
    let frame = rp_loadproto::frame(&payload);
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&frame, "RPL END")),
            during: None,
            then: None,
            timeout: BOOT_TIMEOUT + Duration::from_secs(40),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("cfdetect: no run log after the image returned")?;
    for want in [
        " ms as found: GPIO13 ",
        " ms slot off: GPIO13 ",
        "PULL THE CF CARD OUT NOW",
        "PUSH IT BACK IN NOW",
        " ms end, slot off: GPIO13 ",
        " ms powered 1 s: GPIO13 ",
        "CFDETECT-OK",
    ] {
        if !log.contains(want) {
            return Err(format!("cfdetect: no {want:?} in the run log:{log}"));
        }
    }
    eprintln!("    cf detect OK: every stage ran and was logged, back at the loader");
    Ok(())
}

/// The night-light experiment (item 76) under the loader: every step runs
/// and the channel is put back. The emulator's 2700G applies the graphics
/// gamma table at scan-out (Windows Mobile's grey ramp at reset, gamma off
/// as on a cold display: the image switches it on), so the picture steps
/// run: the screen's grey ramp is seen at full, 1/2 and 1/4 through the
/// table, pixel for pixel as RetroPane's reference (`scanout::gamma` of
/// `scale_gamma`).
pub fn boot_loader_nightlight(root: &Path) -> Result<(), String> {
    use rp_drv_mbx_display::scanout::{WM_GAMMA, gamma, scale_gamma};
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let loader = dir.join("rp-loader.bin");
    let image = dir.join("rp-nightlight.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-nightlight"), &image)?;
    let payload = std::fs::read(&image).map_err(|e| format!("{}: {e}", image.display()))?;
    let frame = rp_loadproto::frame(&payload);
    let shot = scratch(root, "nightlight-screen.ppm")?;
    let guest = match emulator(root)? {
        Emulator::Wsl => to_wsl_path(&shot),
        Emulator::Native(_) => shot.display().to_string(),
    };
    // The image's ramp: grey bar `i` (of 16, 30 pixels wide) from y = 400,
    // in RGB565, then through the table scaled to `picture`/256.
    let bar = |i: u32, picture: u32| {
        let g8 = i * 255 / 15;
        let (r, g) = (((g8 >> 3) << 3) as u8, ((g8 >> 2) << 2) as u8);
        let t = scale_gamma(&WM_GAMMA, picture);
        (gamma(&t, r, 16), gamma(&t, g, 8), gamma(&t, r, 0))
    };
    let pictures = [256, 128, 64, 32];
    let check = || -> Result<(), String> {
        let mut seen = [false; 4];
        let mut last = None;
        let until = Instant::now() + Duration::from_secs(120);
        while !seen.iter().all(|s| *s) {
            if Instant::now() > until {
                return Err(format!(
                    "nightlight: picture steps not all seen through the gamma table (full, 1/2, 1/4, 1/8: {seen:?}); last bars 15 and 8: {last:?}, want {:?}",
                    pictures.map(|p| (bar(15, p), bar(8, p)))
                ));
            }
            let _ = std::fs::remove_file(&shot);
            let mut mon = crate::screen::Monitor::connect(monitor_port())?;
            mon.screendump(&guest, &shot)?;
            let img = crate::screen::parse_ppm(&std::fs::read(&shot).map_err(|e| e.to_string())?)?;
            if img.width >= 480 && img.height >= 640 {
                let px = |x: u32, y: u32| {
                    let o = ((y * img.width + x) * 3) as usize;
                    (img.rgb[o], img.rgb[o + 1], img.rgb[o + 2])
                };
                let got = (px(15 * 30 + 15, 520), px(8 * 30 + 15, 520));
                for (k, p) in pictures.iter().enumerate() {
                    if got == (bar(15, *p), bar(8, *p)) {
                        seen[k] = true;
                    }
                }
                last = Some(got);
            }
            poll_pause(Duration::from_millis(500));
        }
        Ok(())
    };
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&frame, "RPL OK")),
            during: Some(&check),
            then: Some("RPL END"),
            timeout: Duration::from_secs(600),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("nightlight: no run log after the image returned")?;
    for want in [
        "gamma: off, switched on for the steps",
        "step 4: 1/432, picture 1/2",
        "step 5: 1/432, picture 1/4",
        "step 8: 108/216 (WM): whine?",
        "NIGHTLIGHT-OK",
    ] {
        if !log.contains(want) {
            return Err(format!("nightlight: no {want:?} in the run log:{log}"));
        }
    }
    if log.contains("skipped") {
        return Err(format!("nightlight: a step was skipped:{log}"));
    }
    eprintln!(
        "    night light OK: every step ran; the picture steps dimmed the screen through Windows Mobile's gamma table (full, 1/2, 1/4, 1/8), pixel for pixel as the reference"
    );
    Ok(())
}

/// The display calibration test image (docs/design/display-calibration.md)
/// under the loader, as the guided test it is: its picture is pixel-exact
/// below the caption through each gamma table and saturation its steps
/// set (the emulator's 2700G applies the table at scan-out, Windows
/// Mobile's grey ramp at reset). Step 1 right (Black 26); down to step 2,
/// where every other dial is back at 25 (the original), right (White 26);
/// down to step 6, right (Sat 26: the picture redrawn); down to step 7,
/// the starting look, right (the next look); centre (the original); Home
/// puts everything back.
pub fn boot_loader_display_cal(root: &Path) -> Result<(), String> {
    use rp_display_cal::{
        CAPTION_H, Dials, H, PRESET_START, PRESETS, W, look_table, pixel_sat, table,
    };
    use rp_drv_mbx_display::scanout::{GAMMA_ENTRIES, WM_GAMMA, gamma};
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let loader = dir.join("rp-loader.bin");
    let image = dir.join("rp-display-cal.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-display-cal"), &image)?;
    let payload = std::fs::read(&image).map_err(|e| format!("{}: {e}", image.display()))?;
    let frame = rp_loadproto::frame(&payload);
    let shot = scratch(root, "display-cal-screen.ppm")?;
    let guest = match emulator(root)? {
        Emulator::Wsl => to_wsl_path(&shot),
        Emulator::Native(_) => shot.display().to_string(),
    };
    // The picture below the caption at saturation `sat`, through `t`.
    let want = |t: &[u32; GAMMA_ENTRIES], sat: u32| -> Vec<u8> {
        let mut v = Vec::with_capacity((W * (H - CAPTION_H) * 3) as usize);
        for y in CAPTION_H..H {
            for x in 0..W {
                let (r, g, b) = pixel_sat(x, y, sat);
                v.extend_from_slice(&[gamma(t, r, 16), gamma(t, g, 8), gamma(t, b, 0)]);
            }
        }
        v
    };
    let until_table = |name: &str, t: &[u32; GAMMA_ENTRIES], sat: u32| -> Result<(), String> {
        let want = want(t, sat);
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let _ = std::fs::remove_file(&shot);
            let mut mon = crate::screen::Monitor::connect(monitor_port())?;
            mon.screendump(&guest, &shot)?;
            let img = crate::screen::parse_ppm(&std::fs::read(&shot).map_err(|e| e.to_string())?)?;
            let mut differ = usize::MAX;
            if img.width >= W && img.height >= H {
                let mut got = Vec::with_capacity(want.len());
                for y in CAPTION_H..H {
                    let o = ((y * img.width) * 3) as usize;
                    got.extend_from_slice(&img.rgb[o..o + (W * 3) as usize]);
                }
                differ = got
                    .chunks(3)
                    .zip(want.chunks(3))
                    .filter(|(a, b)| a != b)
                    .count();
                if differ == 0 {
                    return Ok(());
                }
            }
            if Instant::now() > deadline {
                let _ = std::fs::copy(&shot, scratch(root, &format!("display-cal-{name}.ppm"))?);
                return Err(format!(
                    "display-cal: the {name} screen differs from the reference in {differ} pixels (target/emulator/display-cal-{name}.ppm)"
                ));
            }
            std::thread::sleep(Duration::from_millis(400));
        }
    };
    let until = |name: &str, d: Dials| until_table(name, &table(d, &WM_GAMMA), d.sat);
    let check = || -> Result<(), String> {
        let key = |k: &str| -> Result<(), String> {
            // Held 200 ms (under the repeat delay), answered before the
            // connection closes.
            crate::screen::Monitor::connect(monitor_port())?.query(&format!("sendkey {k} 200"))?;
            std::thread::sleep(Duration::from_millis(150));
            Ok(())
        };
        let up = |i: usize| Dials::STOCK.moved(i, 1);
        until("original", Dials::STOCK)?;
        key("right")?;
        until("black-26", up(0))?;
        key("down")?;
        until("step-2-reset", Dials::STOCK)?;
        key("right")?;
        until("white-26", up(1))?;
        for _ in 0..4 {
            key("down")?;
        }
        until("step-6-reset", Dials::STOCK)?;
        key("right")?;
        until("sat-26", up(5))?;
        key("down")?;
        until("look-start", PRESETS[PRESET_START].1)?;
        key("right")?;
        until("look-next", PRESETS[PRESET_START + 1].1)?;
        // The first measured look: its own table, the picture as drawn.
        key("right")?;
        until_table(
            "look-measured",
            &look_table(PRESETS.len(), &WM_GAMMA),
            Dials::STOCK.sat,
        )?;
        key("ret")?;
        until("compare-original", Dials::STOCK)?;
        key("f4")
    };
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&frame, "RPL OK")),
            during: Some(&check),
            then: Some("RPL END"),
            timeout: Duration::from_secs(600),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("display-cal: no run log after the image returned")?;
    for want in [
        "gamma: off, switched on",
        "step 1 Black: 26",
        "step 2 White: 26",
        "step 6 Sat: 26",
        "done (Home) after 12 changes; picks: Black 26 White 26 Mid 25 Warm 25 Bright 25 Sat 26",
        "last look: Measured full",
        "DISPLAY-CAL-OK",
    ] {
        if !log.contains(want) {
            return Err(format!("display-cal: no {want:?} in the run log:{log}"));
        }
    }
    eprintln!(
        "    display-cal OK: the guided steps pixel-exact (Black 26, the reset on the next step, White 26, Sat 26 redrawn, two looks and a measured one, the original); Home put back and returned"
    );
    Ok(())
}

/// The C6 looks test under the loader (`rp-looks`), twice: with no card
/// (the built-in photos) and with a card whose `rp-looks` folder holds two
/// photos stored out of name order and a file of the wrong size (ignored).
/// Each time every page pixel-exact below the caption, the strength dial
/// applied in the picture and dithered, driven by keys: 2.00, 2.25 on both
/// photos and the chart, 1.00, the original by compare, the ramps; Home
/// puts everything back.
pub fn boot_loader_looks(root: &Path) -> Result<(), String> {
    looks_run(root, None)?;
    let card = looks_card(&scratch(root, "looks-card.img")?)?;
    looks_run(root, Some(&card))
}

/// A card image with `rp-looks/b.yuv` (the Milky Way), `rp-looks/a.yuv`
/// (the pier), written in that order, and `rp-looks/short.yuv` (too small).
pub(crate) fn looks_card(path: &Path) -> Result<PathBuf, String> {
    use rp_hal_api::{BlockDevice, BlockError};
    use rp_looks::{PHOTOS, YUV_BYTES, unpack};
    struct Mem(Vec<u8>);
    impl BlockDevice for Mem {
        fn read_blocks(&mut self, lba: u32, buf: &mut [u8]) -> Result<(), BlockError> {
            let o = lba as usize * 512;
            let src = self.0.get(o..o + buf.len()).ok_or(BlockError::OutOfRange)?;
            buf.copy_from_slice(src);
            Ok(())
        }
        fn write_blocks(&mut self, lba: u32, buf: &[u8]) -> Result<(), BlockError> {
            let o = lba as usize * 512;
            let dst = self
                .0
                .get_mut(o..o + buf.len())
                .ok_or(BlockError::OutOfRange)?;
            dst.copy_from_slice(buf);
            Ok(())
        }
    }
    let mut photos = [vec![0u8; YUV_BYTES], vec![0u8; YUV_BYTES]];
    for (p, out) in PHOTOS.iter().zip(photos.iter_mut()) {
        unpack(p, out).map_err(|e| format!("looks: a photo did not unpack: {e:?}"))?;
    }
    let img = rp_fat::format::build(&rp_fat::format::Image {
        total_sectors: (SD_IMAGE_BYTES / 512) as u32,
        sectors_per_cluster: 8,
        files: &[],
        blocked_clusters: &[],
        ballast: 0,
    });
    let mut disk = Mem(img);
    {
        let mut vol = rp_fat::Volume::mount(&mut disk, rp_fat::format::PARTITION_START)
            .map_err(|e| format!("looks card: mount: {}", e.name()))?;
        let mut scratch = vec![0u8; 64 * 1024];
        vol.mkdir("rp-looks", &mut scratch)
            .map_err(|e| format!("looks card: mkdir: {}", e.name()))?;
        for (name, data) in [
            ("rp-looks/b.yuv", &photos[1][..]),
            ("rp-looks/a.yuv", &photos[0][..]),
            ("rp-looks/short.yuv", &photos[0][..1000]),
        ] {
            vol.put(name, data, &mut scratch)
                .map_err(|e| format!("looks card: {name}: {}", e.name()))?;
        }
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, &disk.0).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path.to_path_buf())
}

/// One run of the looks test, with `card` in the SD slot or none.
pub(crate) fn looks_run(root: &Path, card: Option<&Path>) -> Result<(), String> {
    use rp_drv_mbx_display::scanout::{GAMMA_ENTRIES, WM_GAMMA, gamma};
    use rp_looks::{
        CAPTION_H, H, PHOTOS, Page, START, W, YUV_BYTES, dial_table, page_pixel, shown, straight,
        unpack,
    };
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let loader = dir.join("rp-loader.bin");
    let image = dir.join("rp-looks.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-looks"), &image)?;
    let payload = std::fs::read(&image).map_err(|e| format!("{}: {e}", image.display()))?;
    let frame = rp_loadproto::frame(&payload);
    let shot = scratch(root, "looks-screen.ppm")?;
    let guest = match emulator(root)? {
        Emulator::Wsl => to_wsl_path(&shot),
        Emulator::Native(_) => shot.display().to_string(),
    };
    let mut photos = [vec![0u8; YUV_BYTES], vec![0u8; YUV_BYTES]];
    for (p, out) in PHOTOS.iter().zip(photos.iter_mut()) {
        unpack(p, out).map_err(|e| format!("looks: a photo did not unpack: {e:?}"))?;
    }
    // Page `page` below the caption: `t` applied in the picture and
    // dithered, then the straight hardware table.
    let st = straight();
    // No card in the emulator: the built-in photos, then the chart and
    // the ramps.
    let want = |page: usize, t: &[u32; GAMMA_ENTRIES]| -> Vec<u8> {
        let (kind, photo): (Page, &[u8]) = match page {
            0 | 1 => (Page::Photo, &photos[page]),
            2 => (Page::Chart, &photos[0]),
            _ => (Page::Ramps, &photos[0]),
        };
        let mut v = Vec::with_capacity((W * (H - CAPTION_H) * 3) as usize);
        for y in CAPTION_H..H {
            for x in 0..W {
                let (r, g, b) = shown(t, page_pixel(kind, photo, x, y), x, y);
                v.extend_from_slice(&[gamma(&st, r, 16), gamma(&st, g, 8), gamma(&st, b, 0)]);
            }
        }
        v
    };
    // `step` None: the original.
    let until = |name: &str, page: usize, step: Option<usize>| -> Result<(), String> {
        let t = step.map_or(WM_GAMMA, |s| dial_table(s, &WM_GAMMA));
        let want = want(page, &t);
        let deadline = Instant::now() + Duration::from_secs(90);
        loop {
            let _ = std::fs::remove_file(&shot);
            let mut mon = crate::screen::Monitor::connect(monitor_port())?;
            mon.screendump(&guest, &shot)?;
            let img = crate::screen::parse_ppm(&std::fs::read(&shot).map_err(|e| e.to_string())?)?;
            let mut differ = usize::MAX;
            if img.width >= W && img.height >= H {
                let mut got = Vec::with_capacity(want.len());
                for y in CAPTION_H..H {
                    let o = ((y * img.width) * 3) as usize;
                    got.extend_from_slice(&img.rgb[o..o + (W * 3) as usize]);
                }
                differ = got
                    .chunks(3)
                    .zip(want.chunks(3))
                    .filter(|(a, b)| a != b)
                    .count();
                if differ == 0 {
                    return Ok(());
                }
            }
            if Instant::now() > deadline {
                let _ = std::fs::copy(&shot, scratch(root, &format!("looks-{name}.ppm"))?);
                return Err(format!(
                    "looks: the {name} screen differs from the reference in {differ} pixels (target/emulator/looks-{name}.ppm)"
                ));
            }
            std::thread::sleep(Duration::from_millis(400));
        }
    };
    let check = || -> Result<(), String> {
        let key = |k: &str| -> Result<(), String> {
            crate::screen::Monitor::connect(monitor_port())?.query(&format!("sendkey {k} 200"))?;
            std::thread::sleep(Duration::from_millis(150));
            Ok(())
        };
        until("strength-2-pier", 0, Some(START))?;
        key("right")?;
        until("strength-2.25-pier", 0, Some(START + 1))?;
        key("down")?;
        until("strength-2.25-milkyway", 1, Some(START + 1))?;
        key("down")?;
        until("strength-2.25-chart", 2, Some(START + 1))?;
        for _ in 0..5 {
            key("left")?;
        }
        until("strength-1-chart", 2, Some(0))?;
        key("ret")?;
        until("compare-original-chart", 2, None)?;
        key("down")?;
        until("original-ramps", 3, None)?;
        key("f4")
    };
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: card,
            steps: &[],
            extra: &[],
            input: Some((&frame, "RPL OK")),
            during: Some(&check),
            then: Some("RPL END"),
            timeout: Duration::from_secs(600),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("looks: no run log after the image returned")?;
    let source = if card.is_some() {
        "2 photos from the card"
    } else {
        "2 photos from the image"
    };
    if !log.contains(source) {
        return Err(format!("looks: no {source:?} in the run log:{log}"));
    }
    for want in [
        "page drawn in",
        "1.00 full (showing the original), Colour chart",
        "done (Home) after 10 changes",
        "last: 1.00, Ramps",
        "LOOKS-OK",
    ] {
        if !log.contains(want) {
            return Err(format!("looks: no {want:?} in the run log:{log}"));
        }
    }
    eprintln!(
        "    looks OK ({}): both photos in name order, the chart and the ramps pixel-exact, the strength dial applied in the picture and dithered (2.00, 2.25, 1.00, original); Home put back and returned",
        if card.is_some() {
            "photos from the card's rp-looks folder, the wrong-sized file ignored"
        } else {
            "no card: the built-in photos"
        }
    );
    Ok(())
}

/// The screen measurement image under the loader, driven by a measurement
/// program written into it (C6, `rp_screen_measure::program`) as `cargo
/// xtask measure --profile` does: grey 128 through the straight table shows
/// exactly 128, then through a halving table exactly 64, each step within
/// 300 ms of plan, everything put back. Then the same image with a damaged
/// program: refused (checksum), nothing shown, returned. The built-in
/// schedule is the same code path (its steps are host-tested).
pub fn boot_loader_screen_measure(root: &Path) -> Result<(), String> {
    use rp_screen_measure::program::{PROGRAM_BYTES, Step, StepLight, encode};
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let loader = dir.join("rp-loader.bin");
    let image = dir.join("rp-screen-measure.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-screen-measure"), &image)?;
    let plain = std::fs::read(&image).map_err(|e| format!("{}: {e}", image.display()))?;
    let room = crate::profile::find_room(&plain)?;
    let lead = 3_000;
    let steps = [
        Step {
            rgb: (128, 128, 128),
            table: 0,
            light: StepLight::Counts(432),
            ms: 4_000,
        },
        Step {
            rgb: (128, 128, 128),
            table: 1,
            light: StepLight::Found,
            ms: 4_000,
        },
        Step {
            rgb: (255, 255, 255),
            table: 0,
            light: StepLight::Counts(0),
            ms: 1_000,
        },
    ];
    let half: [u32; 17] = core::array::from_fn(|i| (((16 * i as u32).min(255)) / 2) * 0x01_0101);
    let mut payload = plain.clone();
    encode(
        &steps,
        &[half],
        lead,
        &mut payload[room..room + PROGRAM_BYTES],
    )
    .map_err(|e| format!("screen-measure program: {}", e.name()))?;
    let starts: Vec<u32> = (0..steps.len())
        .map(|i| lead + steps[..i].iter().map(|s| s.ms).sum::<u32>())
        .collect();
    let shot = scratch(root, "screen-measure.ppm")?;
    let guest = match emulator(root)? {
        Emulator::Wsl => to_wsl_path(&shot),
        Emulator::Native(_) => shot.display().to_string(),
    };
    // The schedule in the guest's time (it shows each patch for its time
    // by its own clock; a busy PC slows it).
    let screen_is = |grey: u8,
                     from_ms: u32,
                     to_ms: u32,
                     t0: &mut GuestClock<'_>|
     -> Result<(), String> {
        while t0.ms()? < u64::from(from_ms) + 800 {
            poll_pause(Duration::from_millis(50));
        }
        let mut seen = None;
        while t0.ms()? < u64::from(to_ms) {
            let _ = std::fs::remove_file(&shot);
            t0.mon.screendump(&guest, &shot)?;
            let img = crate::screen::parse_ppm(&std::fs::read(&shot).map_err(|e| e.to_string())?)?;
            let want = [grey; 3];
            if img
                .rgb
                .chunks(3)
                .take((img.width * 640) as usize)
                .all(|c| c == want)
            {
                return Ok(());
            }
            seen = Some(img.rgb.get(..3).map(<[u8]>::to_vec));
            poll_pause(Duration::from_millis(300));
        }
        Err(format!(
            "screen-measure: grey {grey} never showed on screen between {from_ms} and {to_ms} ms (first pixel {seen:?})"
        ))
    };
    let check = || -> Result<(), String> {
        let mut mon = crate::screen::Monitor::connect(monitor_port())?;
        let mut t0 = GuestClock::start(&mut mon, "screen-measure")?;
        screen_is(128, starts[0], starts[1], &mut t0)?;
        screen_is(64, starts[1], starts[2], &mut t0)
    };
    let frame = rp_loadproto::frame(&payload);
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&frame, "RPL OK")),
            during: Some(&check),
            then: Some("RPL END"),
            timeout: Duration::from_secs(120),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("screen-measure: no run log after the image returned")?;
    for (i, planned) in starts.iter().enumerate() {
        let head = format!("step {i} at ");
        let at = log
            .lines()
            .find_map(|l| l.trim().strip_prefix(&head))
            .and_then(|r| r.split(' ').next())
            .and_then(|n| n.parse::<u32>().ok())
            .ok_or_else(|| format!("screen-measure: no {head:?} in the run log:{log}"))?;
        if at.abs_diff(*planned) > 300 {
            return Err(format!(
                "screen-measure: step {i} started at {at} ms, planned {planned}"
            ));
        }
    }
    for want in ["3 steps, program", "all patches shown", "SCREEN-MEASURE-OK"] {
        if !log.contains(want) {
            return Err(format!("screen-measure: no {want:?} in the run log:{log}"));
        }
    }

    // Damaged: one byte of the steps changed after the checksum was made.
    let mut bad = payload;
    bad[room + 20 + 68 + 1] ^= 0x40;
    let frame = rp_loadproto::frame(&bad);
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&frame, "RPL OK")),
            during: None,
            then: Some("RPL END"),
            timeout: Duration::from_secs(60),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("screen-measure: no run log after the damaged program")?;
    for want in ["program refused: checksum", "SCREEN-MEASURE-OK"] {
        if !log.contains(want) {
            return Err(format!(
                "screen-measure (damaged program): no {want:?} in the run log:{log}"
            ));
        }
    }
    if log.contains("step 0 at") {
        return Err(format!(
            "screen-measure: a damaged program's step was shown:{log}"
        ));
    }
    eprintln!(
        "    screen-measure OK: a program's steps on schedule (each within 300 ms), grey 128 exact through the straight table and 64 through a halving one, a damaged program refused, put back and returned"
    );
    Ok(())
}

/// The refresh-rate test (display calibration C3) under the loader. The
/// loader has already moved the display PLL to 60 Hz (`0xFC5`, as on the
/// device), so the image must refuse to switch and only measure: the
/// emulated frame follows the PLL (rp-v0.29.0, docs/hw/emulator.md), and
/// the counted rate must come out within 1 Hz of 60.36 Hz, twice (at the
/// start, then after D-pad centre); Home puts everything back.
pub fn boot_loader_refresh(root: &Path) -> Result<(), String> {
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let loader = dir.join("rp-loader.bin");
    let image = dir.join("rp-refresh.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-refresh"), &image)?;
    let payload = std::fs::read(&image).map_err(|e| format!("{}: {e}", image.display()))?;
    let frame = rp_loadproto::frame(&payload);
    // The gate's P17 ladder variant walks by itself: no keys.
    let ladder = std::env::var_os("RP_REFRESH_LADDER").is_some();
    let check = || -> Result<(), String> {
        let key = |k: &str| -> Result<(), String> {
            crate::screen::Monitor::connect(monitor_port())?.query(&format!("sendkey {k} 200"))?;
            Ok(())
        };
        if ladder {
            return Ok(());
        }
        // The first measurement (2 s), then a switch press and its 2 s.
        guest_hold(
            &mut crate::screen::Monitor::connect(monitor_port())?,
            5000,
            "refresh",
        )?;
        key("ret")?;
        guest_hold(
            &mut crate::screen::Monitor::connect(monitor_port())?,
            5000,
            "refresh",
        )?;
        key("f4")
    };
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&frame, "RPL OK")),
            during: Some(&check),
            then: Some("RPL END"),
            timeout: Duration::from_secs(300),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("refresh: no run log after the image returned")?;
    if ladder {
        return refresh_ladder_log(log);
    }
    for want in ["no switch, measuring only", "done (Home)", "REFRESH-OK"] {
        if !log.contains(want) {
            return Err(format!("refresh: no {want:?} in the run log:{log}"));
        }
    }
    let rates: Vec<u32> = log
        .lines()
        .filter_map(|l| l.split("clock: ").nth(1))
        .filter_map(|r| r.split(' ').next()?.parse().ok())
        .collect();
    // The fork models the line time from DISPPLL (rp-v0.29.0): the
    // loader's Display::adopt has already moved the panel to 0xFC5, so the
    // image finds it, refuses to switch, and counts ~60.36 Hz, as the X51v
    // measured at 0xFC5 (60.0-60.5 Hz, display calibration C3). The
    // switching path itself (from exactly 0xEC5) isn't reachable here: the
    // loader always switches first, on the device too.
    if rates.len() != 2 || rates.iter().any(|r| r.abs_diff(6036) > 100) {
        return Err(format!(
            "refresh: want two rates within 1 Hz of 60.36 Hz (0xFC5), got {rates:?} (centi-Hz):{log}"
        ));
    }
    eprintln!(
        "    refresh OK: no PLL switch without Windows Mobile's value; frames counted at {:?} centi-Hz (60 Hz, as the loader leaves it); Home put back and returned",
        rates
    );
    Ok(())
}

/// `rp-scene` (R110) under the loader: the graphics service drives a 3D
/// scene beside a 2D bar through three phases (Windows Mobile's clock, the
/// 149.50 / 2 step, the checkerboard by the copy fallback), back to back
/// and one-off. The emulator models no `DSIG` (the references read 0, so
/// no refresh is judged torn): this checks the shape, every frame through
/// the service with nothing refused, recovered or stalled, the clock step
/// and its put-back. The device run judges the refreshes.
pub fn boot_loader_scene(root: &Path, divide1: bool) -> Result<(), String> {
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let loader = dir.join("rp-loader.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    let image = if divide1 {
        // R121's device check: three more phases at 143.00 / 1.
        variant_image_build(root, "rp-scene", &[("RP_SCENE_DIVIDE1", "1")])?
    } else {
        let image = dir.join("rp-scene.bin");
        crate::sdkit::raw_binary(&release.join("rp-scene"), &image)?;
        image
    };
    let payload = std::fs::read(&image).map_err(|e| format!("{}: {e}", image.display()))?;
    let frame = rp_loadproto::frame(&payload);
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&frame, "RPL OK")),
            during: None,
            then: Some("RPL END"),
            timeout: Duration::from_secs(600),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("scene: no run log after the image returned")?;
    scene_log(log, divide1)?;
    for l in log
        .lines()
        .filter(|l| l.starts_with('P') || l.contains("clock to"))
    {
        eprintln!("      {l}");
    }
    if divide1 {
        eprintln!(
            "    scene OK (divide 1): five phases, 1,000 frames through the graphics service, nothing refused, recovered or stalled; the cures on every screen-1 render held on (P5, P6) or left to the clock (P8), none held off (P7); the clock stepped to 149.50 / 2, then 143.00 / 1, and put back"
        );
    } else {
        eprintln!(
            "    scene OK: three phases, 600 frames through the graphics service, nothing refused, recovered or stalled; the clock stepped to 149.50 / 2 and put back"
        );
    }
    Ok(())
}

/// Judges `rp-scene`'s run log (see [`boot_loader_scene`]); `divide1`:
/// the variant's phases P5-P7 at 143.00 / 1 too.
pub(crate) fn scene_log(log: &str, divide1: bool) -> Result<(), String> {
    let d1: &[&str] = if divide1 {
        &[
            "clock to 143.00 / 1: Some(",
            "P7 done; underrun flag",
            "P8 done; underrun flag",
        ]
    } else {
        &[]
    };
    for &want in [
        "RETROPANE-SCENE-OK (failed 0, recovered 0)",
        "clock to 149.50 / 2: Some(",
        "put back: Some(",
    ]
    .iter()
    .chain(d1)
    {
        if !log.contains(want) {
            return Err(format!("scene: no {want:?} in the run log:{log}"));
        }
    }
    // The divide-1 phases' references against P3's: the same, but for
    // row 544 (band 34) alone, the fork's known write one row past a 3D
    // area (docs/hw/emulator.md "Known emulator issues"; the X51v doesn't,
    // scene-0002), which varies with the clock in the emulator.
    if divide1 {
        for p in [5, 6, 7, 8] {
            let want = format!("P{p} references against the last phase's (below the label): ");
            let line = log
                .lines()
                .find(|l| l.starts_with(&want))
                .ok_or_else(|| format!("scene: no {want:?} line:{log}"))?;
            let verdict = &line[want.len()..];
            if !verdict.starts_with("the same;")
                && verdict != "DIFFERENT; bands that differ 0400000000"
            {
                return Err(format!(
                    "scene: P{p}'s references differ beyond row 544: {line:?}"
                ));
            }
        }
    }
    for what in ["back to back", "one-off"] {
        let want = format!("P4 2D flips (no scene) {what}: 60 frames;");
        let line = log
            .lines()
            .find(|l| l.starts_with(&want))
            .ok_or_else(|| format!("scene: no {want:?} line:{log}"))?;
        if !line.contains("failed 0;") {
            return Err(format!("scene: P4 {what}: {line:?}"));
        }
    }
    let phases: &[usize] = if divide1 {
        &[3, 5, 6, 7, 8]
    } else {
        &[1, 2, 3]
    };
    for &p in phases {
        for (what, n) in [("back to back", 150), ("one-off", 50)] {
            let line = log
                .lines()
                .find(|l| {
                    l.starts_with(&format!("P{p} ")) && l.contains(&format!("{what}: {n} frames"))
                })
                .ok_or_else(|| format!("scene: no P{p} {what} line:{log}"))?;
            for want in [
                "copy stalls 0;",
                "failed 0, recovered 0;",
                "2D engine stalls 0",
            ] {
                if !line.contains(want) {
                    return Err(format!("scene: P{p} {what}: no {want:?} in {line:?}"));
                }
            }
            // The divide-1 cures on screen-1 renders: none at divide 2 or
            // held off (P7), all when held on (P5, P6) or left to the
            // clock at 143.00 / 1 (P8, the shipped rule).
            let (one, cured) = scene_screen1(line)
                .ok_or_else(|| format!("scene: P{p} {what}: no screen-1 counts in {line:?}"))?;
            let want_cured = match p {
                5 | 6 | 8 => one,
                _ => 0,
            };
            if cured != want_cured || (p == 8 && one == 0) {
                return Err(format!(
                    "scene: P{p} {what}: {cured} of {one} renders into screen 1 with the cures, want {want_cured}: {line:?}"
                ));
            }
        }
    }
    Ok(())
}

/// A phase line's renders into screen 1 and how many took the divide-1
/// cures (`screen1 N (with the cures M)`).
pub(crate) fn scene_screen1(line: &str) -> Option<(u32, u32)> {
    let rest = line.split(" screen1 ").nth(1)?;
    let one = rest.split(' ').next()?.parse().ok()?;
    let cured = rest
        .split("(with the cures ")
        .nth(1)?
        .split(')')
        .next()?
        .parse()
        .ok()?;
    Some((one, cured))
}

/// Judges `rp-refresh`'s P17 ladder log: every step of `rp_refresh::LADDER`
/// measured three times within 1 Hz of the rate its pixel clock gives (the
/// fork models the line time from `DISPPLL`, rp-v0.29.0), no fault bits,
/// and the PLL put back as found (the loader's 60 Hz, `0xFC5`).
pub(crate) fn refresh_ladder_log(log: &str) -> Result<(), String> {
    let steps: Vec<&str> = log
        .lines()
        .filter(|l| l.contains("ladder ") && l.contains("DISPPLL"))
        .collect();
    if steps.len() != 4 {
        return Err(format!(
            "refresh ladder: want 4 steps, got {}:{log}",
            steps.len()
        ));
    }
    for l in &steps {
        let expect: u32 = l
            .split("(expect ")
            .nth(1)
            .and_then(|r| r.split(' ').next()?.parse().ok())
            .ok_or_else(|| format!("refresh ladder: no expected rate in {l:?}"))?;
        let rates: Vec<u32> = l
            .split("us: ")
            .nth(1)
            .map(|r| {
                r.split(' ')
                    .take(3)
                    .filter_map(|n| n.parse().ok())
                    .collect()
            })
            .unwrap_or_default();
        if rates.len() != 3 || rates.iter().any(|r| r.abs_diff(expect) > 100) {
            return Err(format!(
                "refresh ladder: want 3 rates within 1 Hz of {expect}, got {rates:?} in {l:?}"
            ));
        }
        if !l.contains("locked true") || !l.ends_with("faults 00000000") {
            return Err(format!("refresh ladder: unlocked or faults in {l:?}"));
        }
    }
    for want in ["done (ladder); DISPPLL back to FC5", "REFRESH-OK"] {
        if !log.contains(want) {
            return Err(format!("refresh ladder: no {want:?} in the run log:{log}"));
        }
    }
    eprintln!(
        "    refresh ladder OK: 4 steps, each within 1 Hz of its pixel clock's rate, PLL put back"
    );
    Ok(())
}

/// The crash path: the loader starts an image that panics on purpose; the
/// panic must come back to the loader (no watchdog reset), reach the PC as
/// a crash report, and be kept on the card as `crashlog.txt`.
pub fn boot_loader_crash(root: &Path) -> Result<(), String> {
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    let loader = dir.join("rp-loader.bin");
    let crash = dir.join("rp-crash.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-crash"), &crash)?;
    let payload = std::fs::read(&crash).map_err(|e| format!("{}: {e}", crash.display()))?;
    let frame = rp_loadproto::frame(&payload);
    let image = rp_fat::format::build(&rp_fat::format::Image {
        total_sectors: 32 * 1024,
        sectors_per_cluster: 8,
        files: &[],
        blocked_clusters: &[],
        ballast: 0,
    });
    let sd = scratch(root, "sd-crash.img")?;
    std::fs::write(&sd, &image).map_err(|e| format!("{}: {e}", sd.display()))?;
    // First the image's own crash box, on the plane actually on screen:
    // rp-crash moved it to offset 0 (where Windows Mobile leaves it), so a
    // box drawn at the conventional 0x60000 never shows. Then, once the
    // loader is back, its crash screen: a red band at the top of its log
    // area (below the 96-pixel status lines) and the log's end.
    let shot = scratch(root, "crash-screen.ppm")?;
    let guest = match emulator(root)? {
        Emulator::Wsl => to_wsl_path(&shot),
        Emulator::Native(_) => shot.display().to_string(),
    };
    let screen = || -> Result<crate::screen::Image, String> {
        let _ = std::fs::remove_file(&shot);
        let mut mon = crate::screen::Monitor::connect(monitor_port())?;
        mon.screendump(&guest, &shot)?;
        let img = crate::screen::parse_ppm(&std::fs::read(&shot).map_err(|e| e.to_string())?)?;
        if img.width < 480 || img.height < 640 {
            return Err(format!("crash screen: {} x {}", img.width, img.height));
        }
        Ok(img)
    };
    let px = |img: &crate::screen::Image, x: u32, y: u32| {
        let o = ((y * img.width + x) * 3) as usize;
        (img.rgb[o], img.rgb[o + 1], img.rgb[o + 2])
    };
    // White as the screendump quantises RGB565 (about 248, 252, 248).
    let white = |(r, g, b): (u8, u8, u8)| r > 230 && g > 230 && b > 230;
    // Polls (the image holds its box for 3 s) until `seen` or `limit`.
    let poll = |limit: Duration,
                seen: &dyn Fn(&crate::screen::Image) -> Result<(), String>|
     -> Result<(), String> {
        let until = Instant::now() + limit;
        loop {
            let last = screen().and_then(|img| seen(&img));
            if last.is_ok() || Instant::now() > until {
                return last;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    };
    let image_box = |img: &crate::screen::Image| -> Result<(), String> {
        // The box: the bottom 64 lines, dark red (0xA800) with white text.
        let (r, g, b) = px(img, 470, 600);
        let text = (576..640).any(|y| (0..480).any(|x| white(px(img, x, y))));
        if (150..190).contains(&r) && g < 40 && b < 40 && text {
            Ok(())
        } else {
            Err(format!(
                "crash screen: the image's crash box isn't on the plane on screen (pixel {r},{g},{b}, text {text})"
            ))
        }
    };
    let loader_screen = |img: &crate::screen::Image| -> Result<(), String> {
        let (r, g, b) = px(img, 470, 96 + 4);
        if r < 200 || g > 40 || b > 40 {
            return Err(format!("crash screen: no red band (pixel {r},{g},{b})"));
        }
        // Text in the band (white) and below it (the log, light grey).
        let band = (0..480).any(|x| white(px(img, x, 96 + 10)));
        let text = (96 + 50..96 + 320)
            .any(|y| (0..480).any(|x| px(img, x, y).0 > 200 && px(img, x, y).1 > 200));
        if !band || !text {
            return Err(format!("crash screen: band text {band}, log text {text}"));
        }
        Ok(())
    };
    let check_screen = || -> Result<(), String> {
        poll(Duration::from_secs(5), &image_box)?;
        poll(Duration::from_secs(8), &loader_screen)?;
        // The run log goes out with the crash screen: let it arrive.
        Ok(())
    };
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: Some(&sd),
            steps: &[],
            extra: &[],
            // The image's own panic line, printed just before it draws its
            // box: the checks start while the box is up.
            input: Some((&frame, "PANIC deliberate test panic")),
            during: Some(&check_screen),
            then: Some("RPL END"),
            timeout: BOOT_TIMEOUT,
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("crash: no run log after the image crashed")?;
    if !log.contains("PANIC deliberate test panic") || !log.contains("[the image crashed") {
        return Err(format!("crash: the run log doesn't report the crash:{log}"));
    }
    if log.contains("did not return") {
        return Err("crash: went through a watchdog reset instead of back to the loader".into());
    }
    let saved = card_file(&sd, "crashlog.txt")?;
    if !String::from_utf8_lossy(&saved).contains("PANIC deliberate test panic") {
        return Err("crash: crashlog.txt on the card doesn't hold the report".into());
    }
    // The chain's end can't rest on what the starter left in r11 (the
    // emulator's loader happens to leave 0; the X51v's may not): `_start`
    // must clear it right before `rp_main`, read from the image.
    if !crate::symbols::start_clears_fp(&release.join("rp-crash"))? {
        return Err(
            "crash: rp-crash's _start doesn't clear r11 right before rp_main (frame-pointer build)"
                .into(),
        );
    }
    check_chain(
        root,
        &release.join("rp-crash"),
        log,
        "the panic",
        None,
        (MAIN, false),
    )?;
    if !String::from_utf8_lossy(&saved).contains("CHAIN ") {
        return Err("crash: crashlog.txt on the card doesn't hold the backtrace".into());
    }
    eprintln!(
        "    crash path OK: the image's crash box showed on the plane on screen (offset 0, as WM          leaves it); the panic came back to the loader, reached the PC and the card, and is on          the screen"
    );
    crash_kinds(root, &loader)
}

/// `rp-crash`'s `main`, as `cargo xtask symbols` names it.
pub(crate) const MAIN: &str = "rp_crash::image::main";
/// The function that calls `main` (`rp_image::entry!`'s).
pub(crate) const INTO_MAIN: &str = "rp_crash::image::__rp_image_main";

/// A frame-pointer image's backtrace in its run log (ADR-0001,
/// hal/pxa27x/src/backtrace.rs): one `CHAIN` line that reaches where the
/// image began (`start`), its last call `_start`'s (the chain unbroken to
/// the bottom), and the function `want.0` in it: its first call if
/// `want.1`, else any call or where it faulted (`at`). The addresses are
/// read through `cargo xtask symbols` on `elf`, the image as built.
pub(crate) fn check_chain(
    root: &Path,
    elf: &Path,
    log: &str,
    what: &str,
    at: Option<u32>,
    want: (&str, bool),
) -> Result<(), String> {
    let chains: Vec<&str> = log
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("CHAIN "))
        .collect();
    let [chain] = chains[..] else {
        return Err(format!(
            "crash {what}: want one CHAIN line in the run log, found {}:{log}",
            chains.len()
        ));
    };
    let calls: Vec<String> = chain
        .split_whitespace()
        .skip(1)
        .take_while(|w| w.starts_with("0x"))
        .map(str::to_string)
        .collect();
    let end = chain
        .split_whitespace()
        .skip(1 + calls.len())
        .collect::<Vec<_>>()
        .join(" ");
    if end != "start" || calls.is_empty() {
        return Err(format!(
            "crash {what}: the chain doesn't reach the start: `{chain}`"
        ));
    }
    let elf_arg = elf.display().to_string();
    let named = crate::symbols::symbols(root, &elf_arg, &calls)?;
    // A `symbols` line's function, without its offset.
    let function = |l: &str| -> String {
        let f = l.split_whitespace().nth(1).unwrap_or("");
        f.split_once('+').map_or(f, |(name, _)| name).to_string()
    };
    if named.last().map(|l| function(l)).as_deref() != Some("_start") {
        return Err(format!(
            "crash {what}: the chain's last call isn't _start's:\n{}",
            named.join("\n")
        ));
    }
    let (name, first) = want;
    let mut seen = named.clone();
    if first {
        seen.truncate(1);
    } else if let Some(at) = at {
        seen.extend(crate::symbols::symbols(
            root,
            &elf_arg,
            &[format!("{at:#x}")],
        )?);
    }
    if !seen.iter().any(|l| function(l) == name) {
        return Err(format!(
            "crash {what}: want {name} {}:\n{}",
            if first {
                "as the chain's first call"
            } else {
                "in the chain or where it faulted"
            },
            named.join("\n")
        ));
    }
    eprintln!(
        "    backtrace OK ({what}): {} calls back to _start, {name} {}",
        calls.len(),
        if first { "first" } else { "among them" }
    );
    for l in &named {
        eprintln!("        {l}");
    }
    Ok(())
}

/// Each fault kind through rp-image's crash path (quality audit C38):
/// `rp-crash` built to fault by a data abort, a prefetch abort, an
/// undefined instruction, a system call with no handler and the run limit.
/// Each must come back to the loader marked as a crash (never "did not
/// return", the watchdog's report of a hang, nor a clean return), with its
/// fault line in the run log and on the card, the SPSR of the mode it came
/// from (C44, C61) and its backtrace back to `_start` (ADR-0001).
pub(crate) fn crash_kinds(root: &Path, loader: &Path) -> Result<(), String> {
    for (switch, line, want) in [
        ("RP_CRASH_DATA", "FAULT DataAbort at ", (MAIN, false)),
        // `main` jumped to an unmapped address: the fault's address says
        // nothing, the chain's first call is the call into `main`.
        (
            "RP_CRASH_PREFETCH",
            "FAULT PrefetchAbort at ",
            (INTO_MAIN, true),
        ),
        ("RP_CRASH_UNDEFINED", "FAULT Undefined at ", (MAIN, false)),
        ("RP_CRASH_SWI", "FAULT Swi at ", (MAIN, false)),
        // Interrupted in `spin`: its caller first.
        ("RP_CRASH_RUN_LIMIT", "FAULT RunLimit at ", (MAIN, true)),
    ] {
        let image = variant_image(root, "rp-crash", (switch, "1"))?;
        let payload = std::fs::read(&image).map_err(|e| format!("{}: {e}", image.display()))?;
        let frame = rp_loadproto::frame(&payload);
        let card = rp_fat::format::build(&rp_fat::format::Image {
            total_sectors: 32 * 1024,
            sectors_per_cluster: 8,
            files: &[],
            blocked_clusters: &[],
            ballast: 0,
        });
        let sd = scratch(
            root,
            &format!("sd-crash-{}.img", switch.to_ascii_lowercase()),
        )?;
        std::fs::write(&sd, &card).map_err(|e| format!("{}: {e}", sd.display()))?;
        let output = boot(
            root,
            &Boot {
                image: loader,
                marker: LOADER_MARKER,
                screenshot: None,
                sd_image: Some(&sd),
                steps: &[],
                extra: &[],
                // The fault line, on the serial port first.
                input: Some((&frame, line)),
                during: None,
                then: Some("RPL END"),
                timeout: BOOT_TIMEOUT,
            },
        )?;
        let log = output
            .rsplit_once("RPL LOG")
            .map(|(_, l)| l)
            .ok_or_else(|| format!("crash {switch}: no run log after the image faulted"))?;
        if !log.contains(line) || !log.contains("[the image crashed") {
            return Err(format!(
                "crash {switch}: the run log doesn't report the fault as a crash:{log}"
            ));
        }
        if log.contains("did not return") {
            return Err(format!(
                "crash {switch}: went through a watchdog reset instead of back to the loader"
            ));
        }
        // The SPSR names the mode the fault came from (quality audit C44:
        // the abort entries reported 0, on the X51v): an image runs in SVC
        // or SYS mode.
        let spsr = log
            .split(line)
            .nth(1)
            .and_then(|rest| rest.split(" spsr 0x").nth(1))
            .and_then(|v| u32::from_str_radix(v.get(..8)?, 16).ok())
            .ok_or_else(|| format!("crash {switch}: no SPSR in the fault line:{log}"))?;
        if !matches!(spsr & 0x1F, 0x13 | 0x1F) {
            return Err(format!(
                "crash {switch}: the fault line's SPSR 0x{spsr:08x} names no mode an image runs in"
            ));
        }
        let saved = card_file(&sd, "crashlog.txt")?;
        if !String::from_utf8_lossy(&saved).contains(line) {
            return Err(format!(
                "crash {switch}: crashlog.txt on the card doesn't hold the fault"
            ));
        }
        let at = log
            .split(line)
            .nth(1)
            .and_then(|rest| u32::from_str_radix(rest.strip_prefix("0x")?.get(..8)?, 16).ok());
        check_chain(root, &image.with_extension("elf"), log, switch, at, want)?;
        eprintln!(
            "    crash path OK: `{}` came back to the loader as a crash",
            line.trim_end()
        );
    }
    crash_board(root, loader)
}

/// The kinds of board state the loader keeps that the emulator models
/// (all six: the CPLD, PWM0 and the 2700G's registers hold what's
/// written). Quality audit C39.
pub(crate) const BOARD_KINDS: [&str; 6] = [
    "amplifier",
    "codec/USB bits",
    "backlight",
    "gamma",
    "MBXCLK",
    "M24CLK",
];

/// Quality audit C39 (option B): `rp-crash` built with `RP_CRASH_BOARD`
/// changes every kind of board state the loader keeps, then panics. The
/// loader, recording the board at its first start, must put each kind back
/// on the return and read it back as recorded.
pub(crate) fn crash_board(root: &Path, loader: &Path) -> Result<(), String> {
    let image = variant_image(root, "rp-crash", ("RP_CRASH_BOARD", "1"))?;
    let payload = std::fs::read(&image).map_err(|e| format!("{}: {e}", image.display()))?;
    let frame = rp_loadproto::frame(&payload);
    let output = boot(
        root,
        &Boot {
            image: loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&frame, "RETROPANE-CRASH-TEST")),
            during: None,
            then: Some("RPL END"),
            timeout: BOOT_TIMEOUT,
        },
    )?;
    if !output.contains("board: recorded as Windows Mobile left it") {
        return Err(format!(
            "crash board: the loader's first start didn't record the board:{output}"
        ));
    }
    let line = output
        .lines()
        .find(|l| l.contains("board: put back"))
        .ok_or_else(|| format!("crash board: nothing put back on the return:{output}"))?;
    for kind in BOARD_KINDS {
        if !line.contains(&format!(" {kind};")) {
            return Err(format!("crash board: `{kind}` not put back: {line}"));
        }
    }
    if !line.ends_with("read back: as recorded") {
        return Err(format!("crash board: not as recorded after: {line}"));
    }
    eprintln!("    crash board OK: {line}");
    Ok(())
}

/// Sends the loader an image that hangs with interrupts off (one `b .`
/// instruction, MMU and IRQs off), which the soft run limit can't reach,
/// and checks the backstop: the hardware watchdog resets the machine, the
/// loader starts again and reports that the run didn't return. The emulator
/// restarts the loader itself; on the device Windows Mobile's StartUp
/// folder does (docs: SD kit README, loader/). A 1 s limit keeps the
/// backstop at 31 s. The image starts the cycle counter first (`PMNC` bit 0,
/// as rp-noise leaves it): emulator releases before rp-v0.14.0 aborted on
/// the reset with the counter running instead of rebooting as the device does.
pub fn boot_loader_hang(root: &Path) -> Result<(), String> {
    let loader = raw_image(root, "rp-loader")?;
    let mut hang = rp_loadproto::control_frame("limit 1");
    // mov r0, #1; mcr p14, 0, r0, c0, c1, 0 (PMNC: count); b .
    let words = [0xE3A0_0001u32, 0xEE00_0E11, 0xEAFF_FFFE];
    let code: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    hang.extend(rp_loadproto::frame(&code));
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&hang, "did not return")),
            during: None,
            then: None,
            timeout: Duration::from_secs(120),
        },
    )?;
    if !output.contains("RPL OK") {
        return Err("watchdog: the hanging image was never started".into());
    }
    eprintln!("    watchdog OK: a hung image reset the device and the loader reported it");
    Ok(())
}

/// The card's loader after an image returns, with no PC program on its
/// port: it keeps the watchdog (armed by the run) fed however long it waits.
/// Its link patience (90 s, then a reset) is for a loader started as an
/// image only; on 2026-10-17 it fired after an rp-today run whose USB never
/// came back, and the Axim landed on Dell's factory prompt. The emulated PC
/// enumerates the loader but no program opens its port (the USB chardev has
/// no client), so the link is never proven, as on that day. A loader that
/// stopped feeding would reset within 90 + 30 s of the return; this waits
/// 150 s and wants no start after the one the return makes.
pub fn boot_loader_keeps_feeding(root: &Path) -> Result<(), String> {
    if !has_usb_model(root) {
        eprintln!("    keep-fed: skipped (this emulator has no USB controller model)");
        return Ok(());
    }
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let loader = dir.join("rp-loader.bin");
    let light = dir.join("rp-light.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-light"), &light)?;
    let payload = std::fs::read(&light).map_err(|e| format!("{}: {e}", light.display()))?;
    let input = rp_loadproto::frame(&payload);
    let args = usb_args(4);
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let wait = || -> Result<(), String> {
        guest_hold(
            &mut crate::screen::Monitor::connect(monitor_port())?,
            150000,
            "keeps-feeding",
        )?;
        Ok(())
    };
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &extra,
            input: Some((&input, "RPL END")),
            during: Some(&wait),
            then: None,
            timeout: Duration::from_secs(400),
        },
    )?;
    // Kept for reading: what the loader printed while it waited.
    let _ = std::fs::write(dir.join("keepfed.txt"), &output);
    // Ready once at the first boot, once after the image returned; a third
    // is a reset.
    let starts = output.matches("BENCH boot_loader_ready").count();
    if starts != 2 {
        return Err(format!(
            "keep-fed: the loader started {starts} times (2 wanted: a watchdog reset after the image returned?):{output}"
        ));
    }
    eprintln!(
        "    keep-fed OK: after the image returned, the loader kept the watchdog fed for 150 s with no PC program on its port"
    );
    Ok(())
}

/// The soft run limit (overnight item 55): an image that runs forever with
/// interrupts on and never checks in must come back to the loader with a
/// crash report saying so, with no reset (on the X51v a reset lands on
/// Dell's factory-restore prompt). A 2 s limit keeps the test short.
pub fn boot_loader_run_limit(root: &Path) -> Result<(), String> {
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    let loader = dir.join("rp-loader.bin");
    let hang = dir.join("rp-hang.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-hang"), &hang)?;
    let payload = std::fs::read(&hang).map_err(|e| format!("{}: {e}", hang.display()))?;
    let mut input = rp_loadproto::control_frame("limit 2");
    input.extend(rp_loadproto::frame(&payload));
    let started = Instant::now();
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&input, "RPL END")),
            during: None,
            then: None,
            timeout: Duration::from_secs(60),
        },
    )?;
    let took = started.elapsed();
    if !output.contains("RPL OK limit 2 s") {
        return Err("run limit: the loader didn't take `limit 2`".into());
    }
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("run limit: no run log after the image came back")?;
    if !log.contains("RETROPANE-HANG-TEST") || !log.contains("FAULT RunLimit") {
        return Err(format!("run limit: the run log doesn't report it:{log}"));
    }
    // The interrupted code's mode in the report (quality audit C61: the
    // FIQ entry kept none, every run-limit report said spsr 0): rp-hang
    // spins in SVC mode.
    let spsr = log
        .split("FAULT RunLimit at ")
        .nth(1)
        .and_then(|rest| rest.split(" spsr 0x").nth(1))
        .and_then(|v| u32::from_str_radix(v.get(..8)?, 16).ok())
        .ok_or_else(|| format!("run limit: no SPSR in the fault line:{log}"))?;
    if spsr & 0x1F != 0x13 {
        return Err(format!(
            "run limit: the fault line's SPSR 0x{spsr:08x} isn't SVC mode, where rp-hang spins"
        ));
    }
    // rp-hang is built may-omit (ADR-0001): no walker, and its report says so.
    if !log.contains(rp_hal_pxa27x::backtrace::NO_CHAIN) || log.contains("CHAIN ") {
        return Err(format!(
            "run limit: a may-omit image's report must say `{}` and walk nothing:{log}",
            rp_hal_pxa27x::backtrace::NO_CHAIN
        ));
    }
    if log.contains("did not return") {
        return Err("run limit: went through a reset instead of back to the loader".into());
    }
    eprintln!(
        "    run limit OK: a program that never checked in came back to the loader after its 2 s limit, with a report and no reset ({} s in all)",
        took.as_secs()
    );
    Ok(())
}
