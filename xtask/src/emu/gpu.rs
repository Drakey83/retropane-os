//! The 2700G's own checks: rp-mbx, rp-glesfeat, rp-mbx3d, rp-mbx3d-api, rp-mved, rp-mbx-blend and rp-tabench.

#[allow(clippy::wildcard_imports)]
use super::*;

/// The GPU test image (rp-mbx, item 69): the loader starts it; its fills
/// go through the emulated MBX 2D engine (retropane-qemu rp-v0.12.0 on),
/// so its own read-back checks must pass, and the picture it leaves up
/// must have the GPU-drawn bars and frame with exact edges (ends
/// exclusive). Then Enter returns it to the loader. Skipped, with a note,
/// on an emulator without the model.
pub fn boot_loader_mbx(root: &Path) -> Result<(), String> {
    if !device_has(root, "intel-2700g", "mbx-2d") {
        eprintln!("    mbx: skipped (this emulator release doesn't model the MBX 2D engine)");
        return Ok(());
    }
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    let loader = dir.join("rp-loader.bin");
    let bin = dir.join("rp-mbx.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-mbx"), &bin)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let shot = scratch(root, "mbx-screen.ppm")?;
    let guest = match emulator(root)? {
        Emulator::Wsl => to_wsl_path(&shot),
        Emulator::Native(_) => shot.display().to_string(),
    };
    let check = || -> Result<(), String> {
        // Give the emulator's screen timer a frame to show the picture.
        guest_hold(
            &mut crate::screen::Monitor::connect(monitor_port())?,
            1500,
            "mbx",
        )?;
        let _ = std::fs::remove_file(&shot);
        let mut mon = crate::screen::Monitor::connect(monitor_port())?;
        mon.screendump(&guest, &shot)?;
        let img = crate::screen::parse_ppm(&std::fs::read(&shot).map_err(|e| e.to_string())?)?;
        if (img.width, img.height) != (480, 640) {
            return Err(format!("mbx: screen is {}x{}", img.width, img.height));
        }
        // Inside each part of the scene, and one pixel past each end.
        let (black, red, green, blue, white) = (
            [0, 0, 0],
            [255, 0, 0],
            [0, 255, 0],
            [0, 0, 255],
            [255, 255, 255],
        );
        let want: [((usize, usize), [u8; 3]); 12] = [
            ((60, 100), red),
            ((159, 539), red),
            ((160, 300), black),
            ((110, 540), black),
            ((240, 300), green),
            ((370, 300), blue),
            ((419, 300), blue),
            ((420, 300), black),
            ((22, 300), white),
            ((300, 22), white),
            ((459, 619), white),
            ((40, 60), black),
        ];
        let mut wrong = Vec::new();
        for ((x, y), rgb) in want {
            let i = (y * 480 + x) * 3;
            // The screendump widens RGB565 to 8 bits: compare the top bits.
            let got = [img.rgb[i], img.rgb[i + 1], img.rgb[i + 2]];
            if got.iter().zip(rgb).any(|(g, w)| g >> 3 != w >> 3) {
                wrong.push(format!("({x},{y}) {got:?} not {rgb:?}"));
            }
        }
        if !wrong.is_empty() {
            return Err(format!("mbx: the GPU-drawn picture is wrong at {wrong:?}"));
        }
        // A button press returns it to the loader.
        mon.query("sendkey ret 200")?;
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
            input: Some((&rp_loadproto::frame(&payload), "RETROPANE-MBX-2D")),
            during: Some(&check),
            then: Some("RPL END"),
            timeout: Duration::from_secs(60),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or_else(|| {
            let tail: String = output
                .chars()
                .rev()
                .take(600)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            format!(
                "mbx: never came back to the loader after the button press; serial ends:
{tail}"
            )
        })?;
    if !log.contains("RETROPANE-MBX-2D: PASS") {
        return Err(format!("mbx: the image's own checks failed:{log}"));
    }
    if crashed(log) || log.contains("did not return") {
        return Err(format!("mbx: didn't return cleanly:{log}"));
    }
    for l in log.lines().filter(|l| {
        l.starts_with("dma:")
            || l.starts_with("probe:")
            || l.starts_with("down ")
            || l.starts_with("time ")
            || l.starts_with("completion:")
            || l.contains(" us: ")
            || l.starts_with("  after clearing")
            || l.starts_with("  every status")
            || l.starts_with("irq:")
            || l.contains(" in 40 ms: ")
            || l.starts_with("  [")
            || l.contains("full-screen")
    }) {
        eprintln!("      {}", l.trim_end());
    }
    eprintln!(
        "    mbx OK: fence, rectangle edges and the GPU-drawn picture exact; back at the loader on a button press"
    );
    Ok(())
}

/// rp-mbx-blend's run on the owner's X51v (2026-10-14): the lines the
/// emulator must reproduce.
pub(crate) const MBX_BLEND_DEVICE: &str = include_str!("../../data/rp-mbx-blend-device.txt");

/// The 4-digit hex pixels after a run-log line's colon, before any verdict.
pub(crate) fn logged_pixels(rest: &str) -> Vec<&str> {
    rest.split_whitespace()
        .take_while(|t| t.len() == 4 && t.chars().all(|c| c.is_ascii_hexdigit()))
        .collect()
}

/// Where an emulator run of rp-mbx-blend differs from the device's
/// (`MBX_BLEND_DEVICE`): one entry per line. Pixels are compared exactly;
/// the event lines up to the delays (host-independent under `-icount`, but
/// the device's came from its own clears). Not compared, because nobody
/// knows the answer: format code 4's rows (meaning unknown) and the 32-bit
/// codes' pixels 2-3 from the 16-bit source (they read memory the image
/// never wrote: the device's leftovers).
pub(crate) fn mbx_blend_mismatches(log: &str, device: &str) -> Vec<String> {
    let mut bad = Vec::new();
    for want in device
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
    {
        let Some((key, value)) = want.split_once(": ") else {
            continue;
        };
        if key.starts_with("B3 code 4 ") {
            continue;
        }
        let got = log
            .lines()
            .map(str::trim)
            .find_map(|l| l.strip_prefix(key).and_then(|r| r.strip_prefix(": ")));
        let Some(got) = got else {
            bad.push(format!("no `{key}` line"));
            continue;
        };
        let same = if key.starts_with('B') {
            let (w, g) = (logged_pixels(value), logged_pixels(got));
            let n = if ["12", "13", "14", "15"]
                .iter()
                .any(|c| key.starts_with(&format!("B3 code {c} src16")))
            {
                2
            } else {
                w.len()
            };
            w.len() == g.len() && w[..n] == g[..n]
        } else if key == "E1 frame" {
            frame_close(value, got)
        } else if key.starts_with("E3") {
            value.split(", after").next() == got.split(", after").next()
        } else {
            value == got
        };
        if !same {
            bad.push(format!(
                "{key}: device{}, emulator{}",
                strip_tail(value),
                strip_tail(got)
            ));
        }
    }
    bad
}

/// Two `E1 frame` values agree: the same lines, timings within 0.2% (the
/// device's own runs differ by ~0.02%: 17712 and 17710 us a frame).
pub(crate) fn frame_close(want: &str, got: &str) -> bool {
    let nums = |v: &str| -> Vec<u64> {
        v.split(|c: char| !c.is_ascii_digit())
            .filter_map(|t| t.parse().ok())
            .collect()
    };
    let (w, g) = (nums(want), nums(got));
    // lines 0..TOP (N), US us a frame (MHZ mHz), NS ns a line
    w.len() == 6
        && g.len() == 6
        && w[..3] == g[..3]
        && w[3..]
            .iter()
            .zip(&g[3..])
            .all(|(&a, &b)| a.abs_diff(b) * 500 <= a)
}

/// A run-log value without its verdict, for messages.
pub(crate) fn strip_tail(v: &str) -> String {
    format!(" `{}`", v.split(" ->").next().unwrap_or(v))
}

/// The alpha test, depth "less" and 8-bit grey textures (rp-glesfeat,
/// ADR-0026, 2026-10-18): every case runs and is logged, back at the loader
/// on a button; with the fork's model of them (`mbx-3d-alpha`, rp-v0.22.0)
/// every pixel as the X51v drew it (docs/hw/emulator.md).
pub fn boot_loader_glesfeat(root: &Path) -> Result<(), String> {
    if !device_has(root, "intel-2700g", "mbx-3d") {
        eprintln!("    glesfeat: skipped (this emulator release doesn't model the MBX 3D core)");
        return Ok(());
    }
    // One of tests G-G3 alone (`RP_EMU_GLESFEAT_ONLY=g3`), for checking a
    // changed test before its device run without the half hour the others
    // take; `cargo xtask emu` runs them all (`cargo xtask ci` runs no
    // emulator check).
    match std::env::var("RP_EMU_GLESFEAT_ONLY").as_deref() {
        Ok("g") => return boot_loader_glesfeat_g(root, 1),
        Ok("g2") => return boot_loader_glesfeat_g(root, 2),
        Ok("g3") => return boot_loader_glesfeat_g(root, 3),
        Ok("g4") => return boot_loader_glesfeat_g(root, 4),
        Ok("g5") => return boot_loader_glesfeat_g5(root),
        Ok("g6") => return boot_loader_glesfeat_g(root, 6),
        Ok("g7") => return boot_loader_glesfeat_g(root, 7),
        Ok("g8") => return boot_loader_glesfeat_g(root, 8),
        Ok("g9") => return boot_loader_glesfeat_g9(root),
        Ok("g10") => return boot_loader_glesfeat_g(root, 10),
        Ok("g11") => return boot_loader_glesfeat_g(root, 11),
        Ok("g12") => return boot_loader_glesfeat_g(root, 12),
        Ok("g13") => return boot_loader_glesfeat_g(root, 13),
        Ok("g14") => return boot_loader_glesfeat_g(root, 14),
        Ok("g15") => return boot_loader_glesfeat_g(root, 15),
        Ok("g16") => return boot_loader_glesfeat_g(root, 16),
        Ok("g17") => return boot_loader_glesfeat_g(root, 17),
        Ok("g18") => return boot_loader_glesfeat_g(root, 18),
        Ok("g19") => return boot_loader_glesfeat_g(root, 19),
        Ok("g20") => return boot_loader_glesfeat_g(root, 20),
        Ok("g21") => return boot_loader_glesfeat_g(root, 21),
        Ok(other) => {
            return Err(format!("RP_EMU_GLESFEAT_ONLY={other}: want g or g2-g21"));
        }
        Err(_) => {}
    }
    let loader = raw_image(root, "rp-loader")?;
    let bin = raw_image(root, "rp-glesfeat")?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let check = || -> Result<(), String> {
        let mut mon = crate::screen::Monitor::connect(monitor_port())?;
        mon.query("sendkey ret 200")?;
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
            input: Some((&rp_loadproto::frame(&payload), "RETROPANE-GLESFEAT: ")),
            during: Some(&check),
            then: Some("RPL END"),
            timeout: Duration::from_secs(180),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("glesfeat: never came back to the loader")?;
    if crashed(log) || log.contains("did not return") {
        return Err(format!("glesfeat: didn't return cleanly:{log}"));
    }
    for want in [
        "parameter memory ready: true",
        "A always, ref 0 (control)",
        "A less 0 (version 23's depth 'less')",
        "D blue z 0.8 mode 1 (version 23's form)",
        "L code 10, texels 0x60, TEX2 0x86086610",
        "M: the second texture unit through the TA",
        "M two units, unit 1 TEX2 0xd6087e10",
        "RETROPANE-GLESFEAT: DONE",
    ] {
        if !log.contains(want) {
            return Err(format!("glesfeat: the run log has no `{want}`:{log}"));
        }
    }
    // With the model of the device's results (rp-v0.22.0): the pixels the
    // X51v drew (2026-10-18), word for word.
    if device_has(root, "intel-2700g", "mbx-3d-alpha") {
        for want in [
            "A always, ref 0 (control): ISP 0xe0000000 TSP 0x22200e80: done true; square (200,200) 0x40ff0000",
            "A greater 128: ISP 0x90000000 TSP 0x22200e80: done true; square (200,200) 0xff202020",
            "A greater 32: ISP 0x84000000 TSP 0x22200e80: done true; square (200,200) 0x40ff0000",
            "A less 128: ISP 0x30000000 TSP 0x22200e80: done true; square (200,200) 0x40ff0000",
            "A less 0 (version 23's depth 'less'): ISP 0x20000000 TSP 0x22200e80: done true; square (200,200) 0xff202020",
            "D blue z 0.3 mode 1 (alpha always), ISP 0xe0000000 TSP 0x22200200: done true; overlap (250,250) 0xff0000ff, blue alone (350,350) 0xff202020, red alone (150,150) 0xffff0000",
            "D blue z 0.3 mode 4 (alpha always), ISP 0xe0000000 TSP 0x22200800: done true; overlap (250,250) 0xffff0000, blue alone (350,350) 0xff0000ff",
            "D blue z 0.3 mode 1 (version 23's form), ISP 0x20000000 TSP 0x22200200: done true; overlap (250,250) 0xffff0000, blue alone (350,350) 0xff202020",
            "D blue z 0.8 mode 1 (alpha always), ISP 0xe0000000 TSP 0x22200200: done true; overlap (250,250) 0xffff0000, blue alone (350,350) 0xff202020",
            "L code 10, texels 0x60, TEX2 0x86084610: done true; square (200,200) 0x60606060",
            "L code 10, texels 0xc0, TEX2 0x86084610: done true; square (200,200) 0xc0c0c0c0",
            "L code 10, texels 0x60, TEX2 0x86086610: done true; square (200,200) 0xff606060",
        ] {
            if !log.contains(want) {
                return Err(format!("glesfeat: not as on the device, no `{want}`:{log}"));
            }
        }
        eprintln!("    glesfeat OK: every case drew as on the X51v, back at the loader");
        boot_loader_glesfeat_g(root, 1)?;
        boot_loader_glesfeat_g(root, 2)?;
        boot_loader_glesfeat_g(root, 3)?;
        return boot_loader_glesfeat_g(root, 4);
    }
    eprintln!("    glesfeat OK: every case ran and was logged, back at the loader");
    boot_loader_glesfeat_g(root, 1)?;
    boot_loader_glesfeat_g(root, 2)?;
    boot_loader_glesfeat_g(root, 3)?;
    boot_loader_glesfeat_g(root, 4)
}

/// Test G (`RP_GLESFEAT_G`, the give-up audit's item 1): Gouraud through
/// the TA against our objects at the cores' divide 1; `test` 2, test G2
/// (`RP_GLESFEAT_G2`): the TA's choices one at a time through our direct
/// path, then the TA; 3, test G3 (`RP_GLESFEAT_G3`): the backgrounds,
/// Dell's colour clear among them, at three divide-1 clocks. Every way that
/// draws has a repeatable reference and a
/// right control after each divide-1 setting (the clock put back), and the image
/// returns; a release that doesn't model Windows Mobile's 2700G clocks
/// refuses the change, which the image logs and the check accepts.
/// Test G5 (`RP_GLESFEAT_G5`, the owner's method step 3): Dell's chip
/// state copied under the loader, then halved. The fork draws Gouraud
/// right at every clock, so the image is built with
/// `RP_GLESFEAT_G5_CULPRIT` naming one item (0x6d0, [`G5_CULPRIT`]) a
/// check needs to count as right: the halving must run and end on exactly
/// that item, and the item verified at all three settings. Also held: our
/// way and Dell's whole state draw with repeatable references, every
/// divide-1 setting's control right after it (or the clock refused),
/// Dell's MBX words read back from the core as written, the chip compared
/// with Dell's at 74.75 / 1, no inherited way (the fork starts at Windows
/// Mobile's divide 2: the inherited way isn't exercised here,
/// docs/hw/emulator.md), and the image back at the loader.
pub(crate) fn boot_loader_glesfeat_g5(root: &Path) -> Result<(), String> {
    const G5_CULPRIT: &str = "6";
    let loader = raw_image(root, "rp-loader")?;
    let bin = variant_image_with(
        root,
        "rp-glesfeat",
        ("RP_GLESFEAT_G5", "1"),
        Some(("RP_GLESFEAT_G5_CULPRIT", G5_CULPRIT)),
    )?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let check = || -> Result<(), String> {
        let mut mon = crate::screen::Monitor::connect(monitor_port())?;
        mon.query("sendkey ret 200")?;
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
            input: Some((&rp_loadproto::frame(&payload), "G5: right at 133.25 / 1")),
            during: Some(&check),
            then: Some("RPL END"),
            timeout: Duration::from_secs(1800),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("glesfeat G5: never came back to the loader")?;
    if crashed(log) || log.contains("did not return") {
        return Err(format!("glesfeat G5: didn't return cleanly:{log}"));
    }
    for want in [
        "test G5, Gouraud at the cores' divide 1",
        "parameter memory ready: true",
        "(not divide 1 with a clock running): no inherited way",
        "G5: the emulator's check: a check is right only with item 6 in its mask",
        "G5 way 1 (ours):",
        "G5 state: ours at 74.75 / 1 against Dell's",
        "  ours at 74.75 / 1 after a render: the chip against Dell's",
        "G5 way 2 (ours with Dell's whole state):",
        "G5: ours wrong and Dell's whole state right at 74.75 / 1: halving there",
        "G5: found in ",
        ", mask 0x00040: [0x6d0]",
        "G5: right at 74.75 / 1 of 20: ours 20; Dell's whole state 20; the items found 20",
        "G5: right at 99.67 / 1 of 20: ours 20; Dell's whole state 20; the items found 20",
        "G5: right at 133.25 / 1 of 20: ours 20; Dell's whole state 20; the items found 20",
        "    display after the change: underrun ",
        "RETROPANE-GLESFEAT: DONE",
    ] {
        if !log.contains(want) {
            return Err(format!("glesfeat G5: the run log has no `{want}`:{log}"));
        }
    }
    // Dell's whole state reached the core: way 2's read-back holds every
    // MBX word of the image's list, as written (the fork keeps written
    // words; a test that forgot to write them would pass otherwise).
    let way2 = log
        .split("G5 way 2 (")
        .nth(1)
        .and_then(|s| s.split("read back after the render:").nth(1))
        .and_then(|s| s.lines().next())
        .ok_or(format!("glesfeat G5: way 2 read nothing back:{log}"))?;
    for word in [
        "0x130 0x000087f5",
        "0x610 0x00030001",
        "0x61c 0x00020007",
        "0x6d0 0x00000001",
        "0x6dc 0x3f800001",
        "0x85c 0x01d49ce1",
    ] {
        if !way2.contains(word) {
            return Err(format!(
                "glesfeat G5: way 2's read-back lacks `{word}`: `{way2}`"
            ));
        }
    }
    // Dell's NINTPW at each divide-1 setting of a way that holds it.
    for v in ["0x15", "0x1c", "0x26"] {
        if !log.contains(&format!("    NINTPW {v} by Dell's rule (read back {v})")) {
            return Err(format!(
                "glesfeat G5: no NINTPW {v} by Dell's rule, read back:{log}"
            ));
        }
    }
    // Every check of the halving at one setting; their count as logged.
    let checks = log.matches("G5 check 0x").count();
    let said: usize = log
        .split("G5: found in ")
        .nth(1)
        .and_then(|s| s.split(' ').next())
        .and_then(|n| n.parse().ok())
        .ok_or(format!("glesfeat G5: no check count:{log}"))?;
    if checks == 0 || checks != said || checks > 30 {
        return Err(format!(
            "glesfeat G5: {checks} checks logged, {said} said:{log}"
        ));
    }
    // Every way-setting: a repeatable reference and a right control (none
    // refused: the release models Windows Mobile's clocks). Ways 1, 2 and
    // the verification three settings each, every check one.
    let refs = log
        .matches("rendered true then true, the second 0 words off the first")
        .count();
    let controls = log
        .matches("control at Windows Mobile's clock after: right")
        .count();
    let refused = log.matches("the clock change was refused").count();
    if refused != 0 || refs != 3 + checks || controls != 9 + checks {
        return Err(format!(
            "glesfeat G5: {refs} references, {controls} right controls and {refused} refusals for {checks} checks:{log}"
        ));
    }
    Ok(())
}

/// Test G9 (`RP_GLESFEAT_G9`): every register of the 2700G's two blocks and
/// the PXA27x's, under the loader after our render at Windows Mobile's
/// clock and at 74.75 / 1, for the diff with `rp-wmprobe` v5's. Held: both
/// dumps whole (1,024 MBX words, the system block's 56 known words, every PXA
/// register of `diag`'s list, each once), the renders drawn, the clock put
/// back, the image back at the loader.
pub(crate) fn boot_loader_glesfeat_g9(root: &Path) -> Result<(), String> {
    let loader = raw_image(root, "rp-loader")?;
    let bin = variant_image_with(root, "rp-glesfeat", ("RP_GLESFEAT_G9", "1"), None)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let check = || -> Result<(), String> {
        let mut mon = crate::screen::Monitor::connect(monitor_port())?;
        mon.query("sendkey ret 200")?;
        Ok(())
    };
    // G9's two register dumps outgrow the loader's 64 KiB run log (R77):
    // the check reads the run's card log, which holds 256 KiB (the X51v's
    // G9 card log holds both dumps whole), as emu-gpubench's runs do.
    let card = rp_fat::format::build(&rp_fat::format::Image {
        total_sectors: 64 * 1024,
        sectors_per_cluster: 8,
        files: &[],
        blocked_clusters: &[],
        ballast: 0,
    });
    let sd = scratch(root, "sd-glesfeat-g9.img")?;
    std::fs::write(&sd, &card).map_err(|e| format!("{}: {e}", sd.display()))?;
    let mut input = rp_loadproto::control_frame("name glesfeat");
    input.extend(rp_loadproto::frame(&payload));
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: Some(&sd),
            steps: &[],
            extra: &[],
            input: Some((&input, "RPL OK sha256")),
            during: Some(&check),
            then: Some("RPL END"),
            // The emulator writes the ~2,300 lines a dump slowly: 900 s saw
            // one dump and two thirds of the next (2026-10-23).
            timeout: Duration::from_secs(2700),
        },
    )?;
    let run_log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("glesfeat G9: never came back to the loader")?;
    if crashed(run_log) || run_log.contains("did not return") {
        return Err(format!("glesfeat G9: didn't return cleanly:{run_log}"));
    }
    let file = slot_file(&sd, "glesfeat", 1).map_err(|e| format!("glesfeat G9: {e}"))?;
    let raw = card_file(&sd, &file).map_err(|e| format!("glesfeat G9: card log: {e}"))?;
    let card_log: Vec<u8> = rp_hal_x51v::cardlog::join(Some(&raw), None).collect();
    let card_log = String::from_utf8_lossy(&card_log).replace("\r\n", "\n");
    if !card_log.contains("[card log closed: the run returned") {
        return Err(format!(
            "glesfeat G9: the card log wasn't closed as a run that returned:{card_log}"
        ));
    }
    let log = card_log.as_str();

    if crashed(log) || log.contains("did not return") {
        return Err(format!("glesfeat G9: didn't return cleanly:{log}"));
    }
    for want in [
        "test G9, Gouraud at the cores' divide 1",
        "G9 at Windows Mobile's clock: memory manager ready true, rendered true",
        "G9 at 74.75 / 1: memory manager ready true, rendered true",
        "RETROPANE-GLESFEAT: DONE",
    ] {
        if !log.contains(want) {
            return Err(format!("glesfeat G9: the run log has no `{want}`:{log}"));
        }
    }
    let pxa = rp_hal_pxa27x::diag::COUNT;
    for tag in ["loader div2", "loader div1"] {
        let lines: Vec<&str> = log
            .lines()
            .filter(|l| l.starts_with(&format!("full {tag}: ")))
            .collect();
        let count = |what: &str| lines.iter().filter(|l| l.contains(what)).count();
        let (mbx, sys, px) = (count(": MBX 0x"), count(": SYS 0x"), count(": PXA 0x"));
        if mbx != 1024 || sys != 56 || px != pxa || !lines.iter().any(|l| l.ends_with("ends")) {
            return Err(format!(
                "glesfeat G9: {tag}: {mbx} MBX, {sys} SYS and {px} PXA lines, want 1024, 56 and {pxa}:{log}"
            ));
        }
        for frozen in ["SYS 0x012c ", "SYS 0x030c ", "SYS 0x085c ", "SYS 0x0f00 "] {
            if count(frozen) != 0 {
                return Err(format!(
                    "glesfeat G9: {tag} read {frozen}(it froze Windows Mobile):{log}"
                ));
            }
        }
    }
    // Windows Mobile's clock as the X51v leaves it (run 26's hand-off,
    // 2700g.md: `MBXCLK 0x5`; G9's device run read back exactly this): the
    // emulator's WM-clocks build starts there too. (This check said `0x6`,
    // never true on the device: R77.)
    if !log.contains("read back: COREPLL 0xbb1 MBXCLK 0x5") {
        return Err(format!(
            "glesfeat G9: Windows Mobile's clock not read back:{log}"
        ));
    }
    eprintln!("    glesfeat G9 OK: both dumps whole, back at the loader");
    Ok(())
}

/// The references a glesfeat run refused for holding no quad, checked way
/// by way against the two cases where that is the guard working (R58).
pub(crate) fn glesfeat_no_quad(log: &str, name: &str, test: u8) -> Result<usize, String> {
    let head = format!("{name} way ");
    let mut no_quad = 0;
    for way in log.split(format!("\n{head}").as_str()).skip(1) {
        let n = way.split(' ').next().unwrap_or("");
        let refused_here = way.contains("the reference holds no quad");
        // The fork's own test (`m3_render`): a list word whose flags (bits
        // 21-31) are none of the forms it draws ends the tile. Since
        // rp-v0.31.1 it draws the four-corner objects too (layouts 5 and 9,
        // `0x60A00000` flat and `0x61200000` textured, R186), as the X51v
        // does (rp-glesfeat G2), so a reference without its quad there is
        // a lost quad, not the fork's gap.
        let unmodelled = way.split("list word 0x").skip(1).any(|w| {
            let hex: String = w.chars().take_while(char::is_ascii_hexdigit).collect();
            u32::from_str_radix(&hex, 16).map_or(true, |p| {
                !matches!(
                    p & 0xFFE0_0000,
                    0x4080_0000 | 0x4100_0000 | 0x60A0_0000 | 0x6120_0000
                )
            })
        });
        let rendered = way.contains("rendered true then true");
        let want = (test == 4 && n == "2") || (unmodelled && rendered);
        if refused_here != want {
            return Err(format!(
                "glesfeat {name}: way {n} {} for holding no quad, want {}:{log}",
                if refused_here {
                    "refused"
                } else {
                    "not refused"
                },
                if want { "refused" } else { "not refused" }
            ));
        }
        no_quad += usize::from(refused_here);
    }
    Ok(no_quad)
}

pub(crate) fn boot_loader_glesfeat_g(root: &Path, test: u8) -> Result<(), String> {
    let (env, name, ours, after) = if test == 21 {
        ("RP_GLESFEAT_G21", "G21", "G21 way 1 (", "G21 way 2 (")
    } else if test == 20 {
        ("RP_GLESFEAT_G20", "G20", "G20 way 1 (", "G20 way 2 (")
    } else if test == 19 {
        ("RP_GLESFEAT_G19", "G19", "G19 way 1 (", "G19 way 2 (")
    } else if test == 18 {
        ("RP_GLESFEAT_G18", "G18", "G18 way 1 (", "G18 way 2 (")
    } else if test == 17 {
        ("RP_GLESFEAT_G17", "G17", "G17 way 1 (", "G17 way 2 (")
    } else if test == 16 {
        ("RP_GLESFEAT_G16", "G16", "G16 way 1 (", "G16 way 2 (")
    } else if test == 15 {
        ("RP_GLESFEAT_G15", "G15", "G15 way 1 (", "G15 way 2 (")
    } else if test == 14 {
        ("RP_GLESFEAT_G14", "G14", "G14 way 2 (", "G14 way 3 (")
    } else if test == 13 {
        ("RP_GLESFEAT_G13", "G13", "G13 way 1 (", "G13 way 2 (")
    } else if test == 12 {
        ("RP_GLESFEAT_G12", "G12", "G12 way 1 (", "G12 way 2 (")
    } else if test == 11 {
        ("RP_GLESFEAT_G11", "G11", "G11 way 1 (", "G11 way 2 (")
    } else if test == 10 {
        ("RP_GLESFEAT_G10", "G10", "G10 way 1 (", "G10 way 2 (")
    } else if test == 8 {
        ("RP_GLESFEAT_G8", "G8", "G8 way 1 (", "G8 way 2 (")
    } else if test == 7 {
        ("RP_GLESFEAT_G7", "G7", "G7 way 1 (", "G7 way 2 (")
    } else if test == 6 {
        ("RP_GLESFEAT_G6", "G6", "G6 way 1 (", "G6 way 2 (")
    } else if test == 4 {
        ("RP_GLESFEAT_G4", "G4", "G4 way 1 (", "G4 way 2 (")
    } else if test == 3 {
        ("RP_GLESFEAT_G3", "G3", "G3 way 1 (", "G3 way 2 (")
    } else if test == 2 {
        ("RP_GLESFEAT_G2", "G2", "G2 way 1 (", "G2 way 2 (")
    } else {
        ("RP_GLESFEAT_G", "G", "G way 2 (", "G way 3")
    };
    let loader = raw_image(root, "rp-loader")?;
    // G7 replays a frame file: in the emulator, one of our own scene
    // (`g7::synthetic`: Dell's frames stay local), whose replay must draw
    // the very picture way 1 draws directly. G21 puts its target on the
    // 600 KiB screen: RGB565, as Dell's.
    let frame = root.join("target/g7-synthetic-frame.bin");
    let also = if matches!(
        test,
        7 | 8 | 10 | 11 | 12 | 13 | 14 | 15 | 16 | 17 | 18 | 19 | 20 | 21
    ) {
        std::fs::write(&frame, crate::g7::synthetic(test == 21))
            .map_err(|e| format!("{}: {e}", frame.display()))?;
        Some(("RP_GLESFEAT_G7_FRAME", frame.to_str().unwrap_or("")))
    } else {
        None
    };
    // G14's first way runs on the chip as Dell's driver left it at boot;
    // the emulator starts with every 2700G clock off, so its build runs
    // the way whatever the start (`any-start`, never a device build).
    let value = if test == 14 { "any-start" } else { "1" };
    let bin = variant_image_with(root, "rp-glesfeat", (env, value), also)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let check = || -> Result<(), String> {
        let mut mon = crate::screen::Monitor::connect(monitor_port())?;
        mon.query("sendkey ret 200")?;
        Ok(())
    };
    let summary = format!("{name}: right at 74.75 / 1");
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&rp_loadproto::frame(&payload), &summary)),
            during: Some(&check),
            then: Some("RPL END"),
            timeout: Duration::from_secs(900),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or(format!("glesfeat {name}: never came back to the loader"))?;
    if crashed(log) || log.contains("did not return") {
        return Err(format!("glesfeat {name}: didn't return cleanly:{log}"));
    }
    let wants: &[&str] = if test == 21 {
        &[
            "test G21, Gouraud at the cores' divide 1",
            "parameter memory ready: true",
            "G21: amendment P's C against A, the screen at 0x0 (the safe region ends at 0xae000)",
            "G21 way 1 (Dell's frame off screen, a 2D copy onto the safe screen):",
            "G21 way 2 (Dell's frame drawn straight into the safe screen):",
            "G21 way 3 (Dell's frame into the safe screen, the display paused):",
            "    G21 set-up: the screen at 0x0, GSCADR 0x80000000, ",
            "; the target at 0x200000 (Dell's place, off screen); the display's fetch paused during each render: false; a 2D copy onto the screen after each render: true",
            "; the target at 0x0 (the screen itself); the display's fetch paused during each render: false; a 2D copy onto the screen after each render: false",
            "; the target at 0x0 (the screen itself); the display's fetch paused during each render: true; a 2D copy onto the screen after each render: false",
            "G21: right at 74.75 / 1 of 20:",
            "G21: right at 133.25 / 1 of 20:",
            "RETROPANE-GLESFEAT: DONE",
        ]
    } else if test == 20 {
        &[
            "G17: the screen as found: offset 0x",
            "test G20, Gouraud at the cores' divide 1",
            "parameter memory ready: true",
            "G20 way 1 (Dell's frame, the screen safe, no video plane):",
            "G20 way 2 (Dell's frame, the screen safe, the video plane full outside):",
            "G20 way 3 (Dell's frame, the screen safe, a small video plane inside):",
            "G20 way 4 (Dell's frame, the screen safe, a small video plane outside):",
            "G20 way 5 (a 2D copy, the screen safe):",
            "G20 way 6 (a 2D copy, the screen outside):",
            "    G20 set-up: the screen at 0x0, GSCADR 0x80000000; video None at 0x0, 0 rows",
            "    G20 set-up: the screen at 0x0, GSCADR 0x80000000; video FullOutside at 0x100000, 640 rows",
            "    G20 set-up: the screen at 0x0, GSCADR 0x80000000; video BandInside at 0x96000, 96 rows",
            "    G20 set-up: the screen at 0x0, GSCADR 0x80000000; video BandOutside at 0x100000, 96 rows",
            "    G20 set-up: the screen at 0x100000, GSCADR 0x80010000; video None at 0x0, 0 rows",
            "G20: right at 74.75 / 1 of 20:",
            "RETROPANE-GLESFEAT: DONE",
        ]
    } else if test == 19 {
        &[
            "G17: the screen as found: offset 0x",
            "test G19, Gouraud at the cores' divide 1",
            "parameter memory ready: true",
            "G19: Dell's frame at 74.75 MHz / 1 with the screen at 10 places in local memory; the on-die frame buffer: ODFBPWR 0x",
            "G19 way 1 (Dell's frame, the screen at the place below):",
            "G19: right at 74.75 / 1 of 20:",
            "RETROPANE-GLESFEAT: DONE",
        ]
    } else if test == 18 {
        &[
            "G17: the screen as found: offset 0x",
            "test G18, Gouraud at the cores' divide 1",
            "parameter memory ready: true",
            "G18: Dell's frame at 74.75 MHz / 1 with the screen at 10 places in local memory",
            "G18 way 1 (Dell's frame, the screen at the place below):",
            "G18: right at 74.75 / 1 of 20:",
            "RETROPANE-GLESFEAT: DONE",
        ]
    } else if test == 17 {
        &[
            "G17: the screen as found: offset 0x",
            "  as found: the 2700G's registers below the MBX block",
            "    DMCTRL (0x2188) 0x",
            "  ours (the image's label shown): ",
            "test G17, Gouraud at the cores' divide 1",
            "parameter memory ready: true",
            "G17 way 1 (Dell's frame, the screen at our place):",
            "G17 way 2 (Dell's frame, the screen at Windows Mobile's place):",
            "G17 way 3 (Dell's frame, the screen at our place, Linux's DMCTRL):",
            "G17: right at 74.75 / 1 of 20:",
            "G17: right at 133.25 / 1 of 20:",
            "RETROPANE-GLESFEAT: DONE",
        ]
    } else if test == 16 {
        &[
            "test G16, Gouraud at the cores' divide 1",
            "parameter memory ready: true",
            "G16 way 1 (Dell's frame, as before):",
            "G16 way 2 (Dell's frame, local memory uncached and unbuffered):",
            "G16 way 3 (Dell's frame, the display's fetch off during each render):",
            "G16 way 4 (Dell's frame, both):",
            "    local memory mapped Device (was Some(WriteCombine), reads back Some(Device))",
            "    local memory mapped WriteCombine (was Some(Device), reads back Some(WriteCombine))",
            "G16: right at 74.75 / 1 of 20:",
            "G16: right at 133.25 / 1 of 20:",
            "RETROPANE-GLESFEAT: DONE",
        ]
    } else if test == 15 {
        &[
            "test G15, Gouraud at the cores' divide 1",
            "parameter memory ready: true",
            "G15: Dell's frame at 74.75, 99.67 and 133.25 MHz / 1 with the CPU as found",
            "G15 way 1 (Dell's frame, the CPU as found):",
            "G15 way 2 (Dell's frame, the CPU at 208 MHz, its voltage as found):",
            "G15 way 3 (Dell's frame, the CPU at 208 MHz and Dell's voltage for it):",
            "G15: right at 74.75 / 1 of 20:",
            "G15: right at 133.25 / 1 of 20:",
            "): as found",
            "RETROPANE-GLESFEAT: DONE",
        ]
    } else if test == 14 {
        &[
            "test G14, Gouraud at the cores' divide 1",
            "G14 way 1 (Dell's frame at the clock his driver set at boot, no clock of ours changed):",
            "  not at Dell's divide 1: run anyway (RP_GLESFEAT_G14=any-start, the emulator's build)",
            "    display after the 3D clock on: ",
            "  at Dell's clock, before ours: ",
            "  the clock as found throughout: true",
            "G14: right at Dell's own clock of 20: Dell's frame 20",
            "parameter memory ready: true",
            "G14 way 2 (ours, our places):",
            "G14 way 3 (Dell's frame replayed):",
            "G14: right at 74.75 / 1 of 20:",
            "G14: right at 133.25 / 1 of 20:",
            "RETROPANE-GLESFEAT: DONE",
        ]
    } else if test == 13 {
        &[
            "test G13, Gouraud at the cores' divide 1",
            "parameter memory ready: true",
            "G13 way 1 (ours, the display at the loader's 60 Hz):",
            "G13 way 4 (Dell's frame, the display at Windows Mobile's 56 Hz):",
            "    DISPPLL 0xfc5 -> 0xec5",
            "    DISPPLL 0xec5 -> 0xfc5",
            "G13: right at 74.75 / 1 of 20:",
            "G13: right at 133.25 / 1 of 20:",
            "RETROPANE-GLESFEAT: DONE",
        ]
    } else if test == 12 {
        &[
            "test G12, Gouraud at the cores' divide 1",
            "parameter memory ready: true",
            "G12 way 1 (ours as before):",
            "G12 way 3 (Dell's frame, the CPU silent while it renders):",
            "G12 way 5 (Dell's frame, the CPU silent and the loader-only clocks off):",
            "G12: right at 74.75 / 1 of 20:",
            "G12: right at 133.25 / 1 of 20:",
            "G12: CKEN at the end ",
            "RETROPANE-GLESFEAT: DONE",
        ]
    } else if test == 11 {
        &[
            "test G11, Gouraud at the cores' divide 1",
            "parameter memory ready: true",
            "G11 way 1 (ours, CS2 as found):",
            "G11 way 2 (ours, CS2 Dell's at divide 1):",
            "G11 way 3 (Dell's frame, CS2 Dell's at divide 1):",
            "G11 way 4 (Dell's frame, CS2, NINTPW and clock order Dell's):",
            // Dell's CS2 half reached MSC1 at divide 1, and Windows
            // Mobile's is back at the end.
            "(CS2's half Dell's at divide 1)",
            "    NINTPW 0x15 by Dell's rule (read back 0x15)",
            "G11: right at 74.75 / 1 of 20:",
            "G11: right at 133.25 / 1 of 20:",
            "RETROPANE-GLESFEAT: DONE",
        ]
    } else if test == 10 {
        &[
            "test G10, Gouraud at the cores' divide 1",
            "parameter memory ready: true",
            "G10 way 1 (ours as before: written, rendered and read at divide 1):",
            "G10 way 2 (ours: written and read at Windows Mobile's clock, rendered at divide 1):",
            "G10 way 3 (Dell's frame: written and read at Windows Mobile's clock, rendered at divide 1):",
            "G10 way 4 (ours: written and rendered at divide 1, read at Windows Mobile's clock):",
            // 20 renders, a change to divide 1 and back around each.
            "    40 clock changes around the renders",
            "G10: right at 74.75 / 1 of 20:",
            "G10: right at 99.67 / 1 of 20:",
            "G10: right at 133.25 / 1 of 20:",
            "RETROPANE-GLESFEAT: DONE",
        ]
    } else if test == 8 {
        &[
            "test G8, Gouraud at the cores' divide 1",
            "parameter memory ready: true",
            "G8 way 1 (ours, the status read without a pause (as before)):",
            "G8 way 2 (ours, the chip left alone, the status read once after):",
            "G8 way 3 (ours, the status read once a millisecond):",
            "G8 way 4 (Dell's frame, the chip left alone, the status read once after):",
            "G8 way 5 (Dell's frame, the status read once a millisecond):",
            "  Dell's own picture: none in this frame",
            "G8: right at 74.75 / 1 of 20:",
            "G8: right at 99.67 / 1 of 20:",
            "G8: right at 133.25 / 1 of 20:",
            "    display after the change: underrun ",
            "RETROPANE-GLESFEAT: DONE",
        ]
    } else if test == 7 {
        &[
            "test G7, Gouraud at the cores' divide 1",
            "parameter memory ready: true",
            "G7: the frame: ",
            "target 0x200000 (ARGB8888), parameters 0x600000 (1024 KiB, table 0x700000), region array 0xd10000",
            "G7 way 1 (ours, our places):",
            "G7 way 2 (Dell's frame replayed):",
            "G7 way 3 (Dell's frame replayed, his NINTPW and clock order):",
            "  Dell's own picture: none in this frame",
            "    NINTPW 0x15 by Dell's rule (read back 0x15)",
            "G7: right at 74.75 / 1 of 20:",
            "G7: right at 99.67 / 1 of 20:",
            "G7: right at 133.25 / 1 of 20:",
            "    display after the change: underrun ",
            "; the screen on throughout: ",
            "RETROPANE-GLESFEAT: DONE",
        ]
    } else if test == 6 {
        &[
            "test G6, Gouraud at the cores' divide 1",
            "parameter memory ready: true",
            "G6 way 1 (ours, our places):",
            "G6 way 2 (Dell's parameter memory (8 MiB at 0x22b000)):",
            "G6 way 3 (Dell's places and state (RGB565 target)):",
            "G6 way 4 (Dell's places and state, through the TA):",
            // Each layout reached the core: our addresses, then Dell's
            // parameter memory (its table, first and last page, base), then
            // all his places with his RGB565 target (`0x650` 1, stride
            // 0xf0 words).
            "  layout read back after the render: 0x608 0x00d10000 0x60c 0x00600000 0x650 0x00000006 0x65c 0x00200000 0x660 0x000001e0 0x824 0x00700000",
            "  layout read back after the render: 0x608 0x00d10000 0x60c 0x0022b000 0x650 0x00000006 0x65c 0x00b69000 0x660 0x000001e0 0x824 0x001f8000 0x828 0x0000022c 0x82c 0x00000a2a 0x83c 0x0022b000",
            "  layout read back after the render: 0x608 0x00f35000 0x60c 0x0022b000 0x650 0x00000001 0x65c 0x00e9d000 0x660 0x000000f0 0x824 0x001f8000 0x828 0x0000022c 0x82c 0x00000a2a 0x83c 0x0022b000",
            // Dell's state with his places (way 3).
            "  read back after the render: 0x130 0x000087f5 0x610 0x00030001",
            "G6: right at 74.75 / 1 of 20:",
            "G6: right at 99.67 / 1 of 20:",
            "G6: right at 133.25 / 1 of 20:",
            "    display after the change: underrun ",
            "; the screen on throughout: ",
            "RETROPANE-GLESFEAT: DONE",
        ]
    } else if test == 4 {
        &[
            "test G4, Gouraud at the cores' divide 1",
            "parameter memory ready: true",
            "G4 way 1 (ours):",
            "G4 way 2 (G2's colourless clear):",
            "G4 way 3 (Dell's table, 32-bit):",
            "G4 way 4 (Dell's table and RGB565):",
            "G4 way 10 (Dell's 0x6dc alone):",
            // Dell's words reached the core (way 3's readback after its
            // render; a test that forgot to write them would pass otherwise:
            // the fork doesn't model what they do).
            "  read back after the render: 0x610 0x00030001 0x614 0x1e3ce500 0x618 0x1e3ce500 0x61c 0x00020007 0x648 0x00000001 0x6d8 0x09000000 0x6dc 0x3f800001",
            "G4: right at 74.75 / 1 of 20:",
            "G4: right at 99.67 / 1 of 20:",
            "G4: right at 133.25 / 1 of 20:",
            "    display after the change: underrun ",
            "; the screen on throughout: ",
            "RETROPANE-GLESFEAT: DONE",
        ]
    } else if test == 3 {
        &[
            "test G3, Gouraud at the cores' divide 1",
            "parameter memory ready: true",
            "G3 way 1 (our giant flat triangle):",
            "G3 way 2 (Dell's clear (a flat quad)):",
            "G3 way 3 (Dell's clear and 4-corner quad):",
            "G3 way 4 (flat, screen-sized triangles):",
            "G3 way 5 (our giant triangle at z 1.0):",
            "G3 way 6 (a Gouraud giant triangle):",
            "G3 way 7 (no background):",
            "G3 way 8 (Dell's clear and strip, through the TA):",
            "G3: right at 74.75 / 1 of 20:",
            "G3: right at 99.67 / 1 of 20:",
            "G3: right at 133.25 / 1 of 20:",
            "    display after the change: underrun ",
            "    display back at Windows Mobile's clock: underrun ",
            "; the screen on throughout: ",
            "RETROPANE-GLESFEAT: DONE",
        ]
    } else if test == 2 {
        &[
            "test G2, Gouraud at the cores' divide 1",
            "parameter memory ready: true",
            "G2 way 1 (ours (G's way 2)):",
            "G2 way 2 (one 4-corner object):",
            "G2 way 3 (z 1.0):",
            "G2 way 4 (the clear underneath):",
            "G2 way 5 (per-tile lists):",
            "G2 way 6 (all four: the TA's form):",
            "G2 way 7 (through the TA, one strip of four):",
            "G2 way 8 (through the TA):",
            "G2: right at 74.75 / 1 of 20:",
            "RETROPANE-GLESFEAT: DONE",
        ]
    } else {
        &[
            "test G, Gouraud at the cores' divide 1",
            "parameter memory ready: true",
            "G way 1 (through the TA):",
            "G way 2 (our objects, our layout 4):",
            "G way 4 (through the TA, one strip of four):",
            "G: right at 74.75 / 1 of 20:",
            "RETROPANE-GLESFEAT: DONE",
        ]
    };
    for want in wants {
        if !log.contains(want) {
            return Err(format!(
                "glesfeat {name}: the run log has no `{want}`:{log}"
            ));
        }
    }
    // Every way that drew: its reference rendered twice the same, then the
    // clock refused or a right control after it. The fork doesn't model
    // the tile accelerator (docs/hw/emulator.md: its TA never says done),
    // so a TA way may stop at its reference, safely (and G's way 3 is
    // skipped for want of the TA's object); our own way must always draw.
    let head = format!("{name} way ");
    let ways = log
        .lines()
        .filter(|l| l.starts_with(&head) && l.trim_end().ends_with("):"))
        .count();
    let refs = log
        .matches("rendered true then true, the second 0 words off the first")
        .count();
    let no_ta = log.matches("no repeatable reference").count();
    let ta_silent = log.matches("the TA never said done: no render").count();
    // References the image refused for holding no quad: the guard working,
    // not a fault, in exactly two cases. G4's way 2, whose colourless clear
    // drawn alone first leaves the next renders white on the X51v and in
    // the fork alike. And a way that rendered with an object form the
    // fork doesn't model (docs/hw/emulator.md: its 3D stage reads only list
    // layout 4's and the textured strip's words, and ends a tile's list at
    // any other, so the GLES clear (layout 0, list word 0x40000000) or a
    // four-corner object (layout 5, list word 0x60A00000) leaves the tile
    // without the quad; G2's ways 2, 4 and 6 and G3's ways 2 and 3, which
    // drew the quad on the X51v). Judged way by way,
    // so a modelled way losing its quad still fails (R58).
    let no_quad = glesfeat_no_quad(log, name, test)?;
    // Each way that stopped for want of the TA: its two reference renders.
    if (no_ta - no_quad.min(no_ta)) * 2 != ta_silent {
        return Err(format!(
            "glesfeat {name}: {no_ta} references that didn't repeat, {ta_silent} renders the TA never finished:{log}"
        ));
    }
    let drew = ways - no_ta.min(ways);
    let refused = log.matches("the clock change was refused").count();
    let controls = log
        .matches("control at Windows Mobile's clock after: right")
        .count();
    // A refused no-quad reference still rendered twice alike: not a way
    // that drew.
    if refs - no_quad.min(refs) != drew {
        return Err(format!(
            "glesfeat {name}: {refs} repeatable references for {drew} ways that drew:{log}"
        ));
    }
    // Each way that drew runs every setting: G3 three, G and G2 one;
    // G14's first way none (it stays at the clock it found).
    let settings = if test == 18 || test == 19 || test == 20 {
        1
    } else if test >= 3 {
        3
    } else {
        1
    };
    let clocked = drew - usize::from(test == 14).min(drew);
    if clocked == 0 || refused + controls != clocked * settings {
        return Err(format!(
            "glesfeat {name}: {drew} ways drew, {refused} refused the clock and {controls} came back right:{log}"
        ));
    }
    let own = log
        .split(ours)
        .nth(1)
        .and_then(|s| s.split(after).next())
        .unwrap_or("");
    if !own.contains("rendered true then true, the second 0 words off the first") {
        return Err(format!("glesfeat {name}: our own way didn't draw:{log}"));
    }
    // Ways that must draw one picture, in the forms the fork models (list
    // layout 4): G2's two-triangle forms over our flat background (ways 1,
    // 3 and 5: ours, z 1.0, per-tile lists); G3's black layout-4
    // backgrounds under the same quad (ways 1, 4, 5 and 6: giant, screen
    // triangles, z 1.0, Gouraud). Their references' hashes equal, so a way
    // that drew a wrong picture the same way twice (a broken region array,
    // an empty list) is caught too.
    let same: &[u32] = match test {
        2 => &[3, 5],
        3 => &[4, 5, 6],
        6 => &[2],
        // The replays of our own scene: way 1's picture exactly.
        7 => &[2, 3],
        // Every wait, and the replays of our own scene: one picture.
        8 => &[2, 3, 4, 5],
        // Every mode, and the replay of our own scene: one picture.
        10 => &[2, 3, 4],
        // CS2's timing, and the replays of our own scene: one picture.
        11 => &[2, 3, 4],
        // The replays of our own scene, silent or clocks off: one picture.
        12 => &[2, 3, 4, 5],
        // Either display clock, ours or the replay of our scene: one picture.
        13 => &[2, 3, 4],
        // The replay of our scene at the clock found (way 1, the anchor),
        // ours and the replay after our clock change: one picture.
        14 => &[2, 3],
        // The replay of our scene at every CPU speed and voltage.
        15 => &[2, 3],
        // The replay of our scene however mapped, the display on or off.
        16 => &[2, 3, 4],
        // The replay of our scene wherever the screen is, either DMCTRL.
        17 => &[2, 3],
        // The replay of our scene at every screen place.
        // Every place drawn: the emulator's frame leaves one out (nine).
        18 => &[2, 3, 4, 5, 6, 7, 8, 9],
        // Every place drawn (none over the emulator's frame).
        19 => &[2, 3, 4, 5, 6, 7, 8, 9, 10],
        // The replay of our scene whatever the video plane does.
        20 => &[2, 3, 4],
        // The replay of our scene wherever its target sits: one picture.
        21 => &[2, 3],
        _ => &[],
    };
    if !same.is_empty() {
        let hash = |way: u32| {
            log.split(&format!("{name} way {way} ("))
                .nth(1)
                .and_then(|s| s.split("reference hash ").nth(1))
                .map(|s| s.chars().take(10).collect::<String>())
        };
        let first = hash(1).ok_or(format!(
            "glesfeat {name}: way 1 has no reference hash:{log}"
        ))?;
        for &way in same {
            if hash(way).as_deref() != Some(first.as_str()) {
                return Err(format!(
                    "glesfeat {name}: way {way}'s reference isn't way 1's picture ({:?} against {first}):{log}",
                    hash(way)
                ));
            }
        }
    }
    // G3 and G4: every background drawn alone first (the quad hides it in
    // the reference), way 1's (the giant flat triangle) anchored to all
    // opaque black; in G3 the two others that cover the whole target black
    // in the fork's modelled form (ways 5 and 6: the giant triangle at z
    // 1.0, the giant black Gouraud one) the same picture. A background that
    // didn't draw, or drew the wrong colour, is caught here: the reference
    // can't show it.
    if test == 3 || test == 4 {
        let (same, alone_want): (&[u32], usize) = if test == 3 { (&[5, 6], 6) } else { (&[], 10) };
        let bg = |way: u32| {
            log.split(&format!("{name} way {way} ("))
                .nth(1)
                .and_then(|s| s.split("  background alone: ").nth(1))
                .map(|s| s.lines().next().unwrap_or("").trim().to_string())
        };
        let first = bg(1).ok_or(format!(
            "glesfeat {name}: way 1 has no background alone:{log}"
        ))?;
        // The anchor: the giant triangle covers every pixel, so its picture
        // alone is 480 x 640 words of opaque black, hashed as the image
        // hashes (FNV-1a over words). Without it the group could agree on a
        // wrong picture (all three drawn white, say).
        let black = (0..480 * 640).fold(0x811C_9DC5u32, |h, _| {
            (h ^ 0xFF00_0000).wrapping_mul(0x0100_0193)
        });
        let want = format!("rendered true, hash 0x{black:08x}");
        if first != want {
            return Err(format!(
                "glesfeat {name}: way 1's background alone is {first:?}, want {want:?} (all opaque black):{log}"
            ));
        }
        for &way in same {
            if bg(way).as_deref() != Some(first.as_str()) {
                return Err(format!(
                    "glesfeat {name}: way {way}'s background isn't way 1's ({:?} against {first}):{log}",
                    bg(way)
                ));
            }
        }
        let alone = log.matches("  background alone: rendered true").count();
        if alone != alone_want {
            return Err(format!(
                "glesfeat {name}: {alone} backgrounds rendered alone, want {alone_want} (every direct way with a background):{log}"
            ));
        }
    }
    // The control after divide 1 proves the clock was put back; the same
    // read back as Windows Mobile's. `0x6` (divide 2, 2D + 3D), not WM's
    // hand-off `0x5` (2D only): the X51v itself reads 0x6 once a test has
    // switched the 3D clock on, in every G1-G21 device log (ruling 4 of
    // docs/reports/rulings-2026-10-29.md; G9 never switches it on and
    // reads 0x5; R98 is whether to hand back 0x5).
    if controls > 0 && !log.contains("read back: COREPLL 0xbb1 MBXCLK 0x6") {
        return Err(format!(
            "glesfeat {name}: Windows Mobile's clock not read back:{log}"
        ));
    }
    // G21: each way drew where it says (0x65C read back after the
    // reference: Dell's place in way 1, the screen in ways 2 and 3), and at
    // every setting it ran: 20 renders, way 1's 20 copies onto the screen
    // each exact, no copy in the others, the display's fetch read back off
    // in way 3's renders only.
    if test == 21 {
        for way in 1..=3u32 {
            let seg = log
                .split(&format!("{name} way {way} ("))
                .nth(1)
                .and_then(|s| s.split(&format!("{name} way {} (", way + 1)).next())
                .unwrap_or("");
            let target = if way == 1 { 0x0020_0000u32 } else { 0 };
            let readback = format!(" 0x65c 0x{target:08x} ");
            if !seg.contains("  layout read back after the render:") || !seg.contains(&readback) {
                return Err(format!(
                    "glesfeat {name}: way {way}'s target didn't read back at 0x{target:x}:{log}"
                ));
            }
            let reports: Vec<&str> = seg
                .lines()
                .filter(|l| l.starts_with("    G21 at ") && l.contains(", the 20 renders: "))
                .collect();
            let ran = 3 - seg.matches("the clock change was refused").count();
            if reports.len() != ran || ran == 0 {
                return Err(format!(
                    "glesfeat {name}: way {way} reported {} settings' renders, want {ran}:{log}",
                    reports.len()
                ));
            }
            let copies = if way == 1 {
                "; 20 2D copies onto the screen, "
            } else {
                "; 0 2D copies onto the screen, "
            };
            for r in reports {
                let paused = !r.ends_with("read back off: 0");
                if !r.contains(": 20 renders, ")
                    || !r.contains(copies)
                    || !r.contains(", 0 not exact (0 words off in all)")
                    || paused != (way == 3)
                {
                    return Err(format!("glesfeat {name}: way {way}: {r}:{log}"));
                }
            }
        }
    }
    // G16: each way's mapping and the display's fetch as set, and renders
    // made with the fetch read back off in ways 3 and 4 only (each count is
    // logged at the next way's set-up, the last at the end).
    // G20: the 2D copies (ways 5 and 6) one picture, not the frame's, and
    // the video plane on (its stream bit) in ways 2-4 only.
    if test == 20 {
        let hash = |way: u32| {
            log.split(&format!("{name} way {way} ("))
                .nth(1)
                .and_then(|s| s.split("reference hash ").nth(1))
                .map(|s| s.chars().take(10).collect::<String>())
        };
        if hash(5).is_none() || hash(5) != hash(6) || hash(5) == hash(1) {
            return Err(format!(
                "glesfeat {name}: the 2D copies' references {:?} and {:?} (the frame's {:?}):{log}",
                hash(5),
                hash(6),
                hash(1)
            ));
        }
        let vscadr = |way: u32| {
            log.split(&format!("{name} way {way} ("))
                .next()
                .and_then(|b| b.rsplit("    G20 set-up: ").next())
                .and_then(|l| l.split("VSCADR 0x").nth(1))
                .and_then(|h| u32::from_str_radix(&h[..8.min(h.len())], 16).ok())
        };
        for way in 1..=6u32 {
            let on = vscadr(way).map(|v| v >> 31 == 1);
            if on != Some((2..=4).contains(&way)) {
                return Err(format!(
                    "glesfeat {name}: way {way}'s video stream on: {on:?}, want {}:{log}",
                    (2..=4).contains(&way)
                ));
            }
        }
    }
    // G18: every place that wasn't left out drew, its screen set and read
    // back there (`GSCADR` its place / 16, the stream on), in order.
    if test == 18 || test == 19 {
        let places: [usize; 10] = if test == 18 {
            [
                0x0, 0x400, 0x800, 0x1000, 0x2000, 0x8_0000, 0x10_0000, 0x12_0000, 0xC8_0000,
                0xF0_0000,
            ]
        } else {
            [
                0x4000, 0x8000, 0x1_0000, 0x1_8000, 0x2_0000, 0x3_0000, 0x4_0000, 0x5_0000,
                0x6_0000, 0x7_0000,
            ]
        };
        let left = format!("{name}: place ");
        // Each place either set at its own address or named as left out
        // (one over the frame: the emulator's frame, our own scene, lies
        // elsewhere than Dell's), never both.
        let mut seen = Vec::new();
        for line in log.lines() {
            if let Some(rest) = line.strip_prefix(left.as_str())
                && line.ends_with("left out")
            {
                let k: usize = rest
                    .split(' ')
                    .next()
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(0);
                seen.push((k, false));
            }
            if let Some(rest) = line.strip_prefix("    the screen at 0x")
                && let Some(p) = rest.split("(place ").nth(1)
            {
                let k: usize = p
                    .split(')')
                    .next()
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(0);
                let at = places.get(k.wrapping_sub(1)).copied().unwrap_or(usize::MAX);
                let want = format!(
                    "{at:x} (place {k}), GSCADR 0x{:08x}",
                    1u32 << 31 | (at / 16) as u32
                );
                if !rest.starts_with(&want) {
                    return Err(format!(
                        "glesfeat {name}: want `{want}`, got `{line}`:{log}"
                    ));
                }
                seen.push((k, true));
            }
        }
        let mut ks: Vec<usize> = seen.iter().map(|s| s.0).collect();
        ks.sort_unstable();
        if ks != (1..=places.len()).collect::<Vec<_>>() || seen.iter().filter(|s| s.1).count() < 8 {
            return Err(format!(
                "glesfeat {name}: places set (true) or left out {seen:?}, want each of 1-{} once, 8 or more set:{log}",
                places.len()
            ));
        }
    }
    // G17: each way's screen place and DMCTRL as set and read back: way 2
    // at the place found at start (not ours), way 3 with Linux's DMCTRL.
    if test == 17 {
        let found = log
            .split("G17: the screen as found: offset 0x")
            .nth(1)
            .and_then(|s| s.split(',').next())
            .and_then(|h| usize::from_str_radix(h, 16).ok())
            .ok_or(format!("glesfeat {name}: no screen as found:{log}"))?;
        // Each way's set-up is logged just before its header.
        let set = |way: u32| {
            log.split(&format!("{name} way {way} ("))
                .next()
                .and_then(|before| before.rsplit("    the screen at 0x").next())
                .map(|s| s.lines().next().unwrap_or("").to_string())
                .unwrap_or_default()
        };
        let ours = 0x0010_0000usize;
        for (way, at, gscadr_ok, dmctrl) in [
            (1, ours, true, None),
            (2, found, true, None),
            (3, ours, true, Some("DMCTRL 0xd0303010 (Linux's)")),
        ] {
            let line = set(way);
            let want_at = format!("{at:x} (");
            // GSCADR holds the plane's place / 16 with the stream on.
            let want_g = format!("GSCADR 0x{:08x}", 1u32 << 31 | (at / 16) as u32);
            if !line.starts_with(&want_at) || (gscadr_ok && !line.contains(&want_g)) {
                return Err(format!(
                    "glesfeat {name}: way {way}'s screen isn't at 0x{at:x} ({want_g}): {line}:{log}"
                ));
            }
            if let Some(d) = dmctrl
                && !line.contains(d)
            {
                return Err(format!(
                    "glesfeat {name}: way {way}'s DMCTRL isn't Linux's: {line}:{log}"
                ));
            }
            if dmctrl.is_none() && line.contains("(Linux's)") {
                return Err(format!(
                    "glesfeat {name}: way {way} has Linux's DMCTRL: {line}:{log}"
                ));
            }
        }
        if found == ours {
            return Err(format!(
                "glesfeat {name}: the screen found at start is the label's own place, so way 2 tests nothing:{log}"
            ));
        }
    }
    if test == 16 {
        let counts: Vec<u32> = log
            .lines()
            .filter_map(|l| {
                l.strip_prefix(
                    "    the chip's runs with the display's fetch off since the last way: ",
                )
                .and_then(|n| n.trim().parse().ok())
            })
            .collect();
        // The first is the set-up before way 1 (nothing before it).
        let ok = counts.len() == 5
            && counts[0] == 0
            && counts[1] == 0
            && counts[2] == 0
            && counts[3] > 0
            && counts[4] > 0;
        if !ok {
            return Err(format!(
                "glesfeat {name}: renders with the display's fetch off {counts:?}, want [0, 0, 0, >0, >0]:{log}"
            ));
        }
        let set = |way: u32| {
            log.split(&format!("{name} way {way} ("))
                .next()
                .and_then(|before| before.rsplit("    local memory mapped ").next())
                .map(|s| s.lines().next().unwrap_or("").to_string())
                .unwrap_or_default()
        };
        for (way, map, dark) in [
            (1, "WriteCombine", false),
            (2, "Device", false),
            (3, "WriteCombine", true),
            (4, "Device", true),
        ] {
            let line = set(way);
            let want_dark = format!("fetch off during each render: {dark}");
            if !line.starts_with(map) || !line.contains(&want_dark) {
                return Err(format!(
                    "glesfeat {name}: way {way}'s set-up isn't {map}, {want_dark}: {line}:{log}"
                ));
            }
        }
    }
    if test == 15 {
        let cpu = |way: u32| {
            log.split(&format!("{name} way {way} ("))
                .next()
                .and_then(|before| before.rsplit("    CPU: ").next())
                .map(|s| s.lines().next().unwrap_or("").to_string())
                .unwrap_or_default()
        };
        let ours_pins = log.contains("the pins plain outputs: true");
        for (way, want, code) in [
            (1, None, None),
            (2, Some("core counted 208"), None),
            (
                3,
                Some("core counted 208"),
                Some("voltage code 0b100 (Dell's label Some(1163) mV)"),
            ),
        ] {
            let line = cpu(way);
            if line.is_empty() {
                return Err(format!("glesfeat {name}: way {way} has no CPU line:{log}"));
            }
            if let Some(w) = want
                && !line.contains("CCCR 0x02000110")
            {
                return Err(format!(
                    "glesfeat {name}: way {way}'s CPU isn't at 208 MHz ({w}): {line}:{log}"
                ));
            }
            if way == 1 && line.contains("CCCR 0x02000110") {
                return Err(format!(
                    "glesfeat {name}: way 1's CPU isn't as found: {line}:{log}"
                ));
            }
            if let (true, Some(c)) = (ours_pins, code)
                && !line.contains(c)
            {
                return Err(format!(
                    "glesfeat {name}: way {way}'s voltage isn't Dell's for 208 MHz: {line}:{log}"
                ));
            }
            if way == 2 && ours_pins && line.contains("voltage code 0b100") {
                return Err(format!(
                    "glesfeat {name}: way 2's voltage was lowered: {line}:{log}"
                ));
            }
        }
    }
    eprintln!(
        "    glesfeat {name} OK: {ways} ways, {drew} drew (references repeatable; the TA not modelled: {no_ta}), the clock refused in {refused}, put back in {controls}, back at the loader"
    );
    Ok(())
}

/// The MBX 3D core's first-triangle test (rp-mbx3d, ADR-0024 M4): runs
/// under the loader, every try timed out (no 3D model), back on a button.
pub fn boot_loader_mbx3d(root: &Path) -> Result<(), String> {
    if !device_has(root, "intel-2700g", "mbx-2d") {
        eprintln!("    mbx3d: skipped (this emulator release doesn't model the MBX 2D engine)");
        return Ok(());
    }
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    let loader = dir.join("rp-loader.bin");
    let bin = dir.join("rp-mbx3d.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-mbx3d"), &bin)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let check = || -> Result<(), String> {
        let mut mon = crate::screen::Monitor::connect(monitor_port())?;
        mon.query("sendkey ret 200")?;
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
            input: Some((&rp_loadproto::frame(&payload), "RETROPANE-MBX3D: ")),
            during: Some(&check),
            then: Some("RPL END"),
            timeout: Duration::from_secs(120),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("mbx3d: never came back to the loader after the button press")?;
    if crashed(log) || log.contains("did not return") {
        return Err(format!("mbx3d: didn't return cleanly:{log}"));
    }
    let modelled = device_has(root, "intel-2700g", "mbx-3d");
    // Without the fork's 3D model (before rp-v0.15.0) the image must try
    // the parameter memory set-up without and with the 3D clock, get no
    // "ready", try no render, and come back. With it: the memory manager
    // is ready only with the 3D clock on (as on the device), and the red
    // triangle draws with the target register set (try A' or B).
    let want: &[&str] = if modelled {
        &[
            "core ID 0x01020000",
            "(3D clock on)",
            "drawn 480 px across, 640 down",
            "RETROPANE-MBX3D: DREW",
        ]
    } else {
        &[
            "core ID 0x01020000",
            "P: parameter memory set-up, MBXCLK",
            "(3D clock on)",
            "the parameter memory never said ready: no render tried",
            "RETROPANE-MBX3D: nothing drawn",
        ]
    };
    for want in want {
        if !log.contains(want) {
            return Err(format!("mbx3d: the run log has no `{want}`:{log}"));
        }
    }
    // With the fork's render-done interrupt and texel model (mbx-3d-irq,
    // rp-v0.16.0): what version 24 saw on the device.
    // (Version 24's stages only: later versions ask other questions.)
    if modelled && device_has(root, "intel-2700g", "mbx-3d-irq") && log.contains("T10 MINTENABLE") {
        let line = |key: &str| log.lines().find(|l| l.contains(key)).unwrap_or("");
        for (key, want) in [
            ("MINTENABLE 0x800f, 0x130 0x0004:", "edges 0"),
            ("MINTENABLE 0x800f, 0x130 0x8004:", "edges 1"),
            ("MINTENABLE 0x8001, 0x130 0x8004:", "edges 0"),
            ("MINTENABLE 0x8004, 0x130 0x8004:", "edges 1"),
            (
                "T12 TEX1 bit 31 (linear)",
                "ffff3f3f, ffff7f7f, ffffbfbf, ffffffff, ffffbfbf",
            ),
            (
                "T13 PVRTC 4 bpp (code 28), red",
                "[ffff0000, ffff0000, ffff0000, ffff0000]",
            ),
            (
                "T13 PVRTC 4 bpp, words swapped",
                "[ff000000, ff000000, ff000000, ff000000]",
            ),
            ("T13 32-bit texels", "[ffff0000, ff00ff00, ff0000ff"),
        ] {
            if !line(key).contains(want) {
                return Err(format!(
                    "mbx3d: `{key}` should show `{want}` as on the device:{log}"
                ));
            }
        }
    }
    // Version 25's stages, likewise (mipmaps and PVRTC's block order,
    // from rp-v0.16.0's model).
    if modelled && device_has(root, "intel-2700g", "mbx-3d-irq") && log.contains("T12b") {
        let line = |key: &str| log.lines().find(|l| l.contains(key)).unwrap_or("");
        for (key, want) in [
            (
                "T12b layout B, TEX0 bits 12-9 = 7:",
                "16 px quad 0xff0000ff, of the 8 px quad 0xffffff00",
            ),
            (
                "T12b layout A, TEX0 bits 12-9 = 7:",
                "16 px quad 0xffff0000, of the 8 px quad 0xffff0000",
            ),
            (
                "T13b PVRTC 4 bpp",
                "[[00, 21, 84, a5], [10, 31, 94, b5], [42, 63, c6, e7], [52, 73, d6, f7]]",
            ),
        ] {
            if !line(key).contains(want) {
                return Err(format!(
                    "mbx3d: `{key}` should show `{want}` as on the device:{log}"
                ));
            }
        }
    }
    // Version 26's 16-bit targets, likewise.
    if modelled && device_has(root, "intel-2700g", "mbx-3d-irq") && log.contains("S26b") {
        let line = |key: &str| log.lines().find(|l| l.contains(key)).unwrap_or("");
        for (key, want) in [
            (
                "S26b 0x650 1, 0x660 240:",
                "(2,2) 0xf800 (18,2) 0x07e0 (2,34) 0x001f (18,34) 0xffff; drawn to x Some(32) on row 2, y Some(64)",
            ),
            (
                "S26b 0x650 3, 0x660 240:",
                "(2,2) 0xfc00 (18,2) 0x83e0 (2,34) 0x801f (18,34) 0xffff",
            ),
        ] {
            if !line(key).contains(want) {
                return Err(format!(
                    "mbx3d: `{key}` should show `{want}` as on the device:{log}"
                ));
            }
        }
    }
    // Version 29's option A: the model has neither the PXA's bus timing
    // nor the 2700G's clock and memory registers as Windows Mobile leaves
    // them, so the image measures today's setting once (every check must
    // pass) and changes nothing.
    if modelled {
        for want in [
            "S29 as found",
            "S29 today: 99.67 MHz, divide by 2, CS3 0x2484:",
            "S29 not Windows Mobile's state: nothing changed",
        ] {
            if !log.contains(want) {
                return Err(format!("mbx3d: the run log has no `{want}`:{log}"));
            }
        }
        let today = log.lines().find(|l| l.contains("S29 today:")).unwrap_or("");
        if !today.ends_with("| GOOD") {
            return Err(format!("mbx3d: today's setting should check GOOD:{log}"));
        }
    }
    eprintln!(
        "    mbx3d OK: {}, back at the loader on a button press",
        if modelled {
            "the red triangle drawn by the modelled 3D core"
        } else {
            "ran every try safely (the 3D core isn't modelled)"
        }
    );
    Ok(())
}

/// RetroPane's own 3D renderer's device test (rp-mbx3d-api, ADR-0026):
/// runs under the loader. With the fork's 3D model (`mbx-3d`, rp-v0.15.0)
/// every counted check must pass, as on the device (run 3: 0 BAD), and
/// every T9 try draw right (the model keeps the core's state across the
/// clock: an assumption until run 4). Without it, the renderer's set-up
/// never hears "ready" and every frame is refused (`NoCore`) without a
/// fault. Either way the clock, plane and core are put back and it returns
/// on a button.
pub fn boot_loader_mbx3d_api(root: &Path) -> Result<(), String> {
    if !device_has(root, "intel-2700g", "mbx-2d") {
        eprintln!("    mbx3d-api: skipped (this emulator release doesn't model the MBX 2D engine)");
        return Ok(());
    }
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    let loader = dir.join("rp-loader.bin");
    let bin = dir.join("rp-mbx3d-api.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-mbx3d-api"), &bin)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let check = || -> Result<(), String> {
        let mut mon = crate::screen::Monitor::connect(monitor_port())?;
        mon.query("sendkey ret 200")?;
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
            input: Some((&rp_loadproto::frame(&payload), "RETROPANE-MBX3D-API: ")),
            during: Some(&check),
            then: Some("RPL END"),
            timeout: Duration::from_secs(600),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("mbx3d-api: never came back to the loader after the button press")?;
    if crashed(log) || log.contains("did not return") {
        return Err(format!("mbx3d-api: didn't return cleanly:{log}"));
    }
    let modelled = device_has(root, "intel-2700g", "mbx-3d");
    let want: &[&str] = if modelled {
        &[
            "S1 (T1)",
            "S9 (T9)",
            "everything given back: true",
            " checks, 0 BAD",
        ]
    } else {
        &[
            "S1 (T1)",
            "whole screen: NoCore",
            "S7: the 3D clock gate",
            "everything given back: true",
            "RETROPANE-MBX3D-API: ",
        ]
    };
    for want in want {
        if !log.contains(want) {
            return Err(format!("mbx3d-api: the run log has no `{want}`:{log}"));
        }
    }
    if modelled {
        let tries = log.matches("    T9 ").count();
        if tries != 6 || log.contains("WRONG") || log.contains("    BAD ") {
            return Err(format!(
                "mbx3d-api: {tries} T9 tries, every one must draw right, no check BAD:{log}"
            ));
        }
    }
    eprintln!(
        "    mbx3d-api OK: {}, back at the loader on a button press",
        if modelled {
            "every check passed on the modelled 3D core, as on the device"
        } else {
            "every stage ran, every frame refused safely (the 3D core isn't modelled)"
        }
    );
    Ok(())
}

/// The MVED1 video decoder's first test (rp-mved, ADR-0025 S0-S2): runs
/// under the loader, back on a button. The fork doesn't model the decoder
/// (docs/hw/emulator.md): its registers read as written (status 0), so S0
/// and S1 see an idle unit, the conversion times out, the guards hold,
/// mode 3 isn't tried, and the image puts the clock back.
pub fn boot_loader_mved(root: &Path) -> Result<(), String> {
    if !device_has(root, "intel-2700g", "mbx-2d") {
        eprintln!("    mved: skipped (this emulator release doesn't model the MBX 2D engine)");
        return Ok(());
    }
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let loader = dir.join("rp-loader.bin");
    let bin = dir.join("rp-mved.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-mved"), &bin)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let check = || -> Result<(), String> {
        let mut mon = crate::screen::Monitor::connect(monitor_port())?;
        mon.query("sendkey ret 200")?;
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
            input: Some((&rp_loadproto::frame(&payload), "RETROPANE-MVED: ")),
            during: Some(&check),
            then: Some("RPL END"),
            timeout: Duration::from_secs(120),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("mved: never came back to the loader after the button press")?;
    if crashed(log) || log.contains("did not return") {
        return Err(format!("mved: didn't return cleanly:{log}"));
    }
    for want in [
        "S0: M24CLK 0x0",
        "set to 0x3",
        "S1: reset; FIFO 0x00000000; status 0x00000000: passed",
        "S2: 64 x 64 4:2:0 at 0x400000 to YUY2 at 0x410000",
        "job 0x30=0x80000 0x34=0x82000 0x38=0x20040 0x3c=0x2020 0x48=0x1",
        "NOT DONE after",
        "guards intact: true; destination words unwritten: 2048 of 2048",
        "RETROPANE-MVED: S0 passed, S1 passed, S2 YUY2 FAILED, mode 3 not run",
        "put back: the unit reset, M24CLK 0x0",
    ] {
        if !log.contains(want) {
            return Err(format!("mved: the run log has no `{want}`:{log}"));
        }
    }
    if log.contains("S2 mode 3: to") {
        return Err(format!("mved: mode 3 ran after a failed conversion:{log}"));
    }
    eprintln!(
        "    mved OK: every stage safe, the conversion timed out inside its guards (the decoder isn't modelled), back at the loader on a button press"
    );
    Ok(())
}

/// The blend and display-event probe (rp-mbx-blend, renderer step 1): the
/// loader starts it; it must run every stage to its verdict without a
/// fault, put the blend state and the display registers back, and return
/// on a button; then B7 alone ([`mbx_blend_b7`]). Under `-icount` (the
/// event stages time scan lines), on an emulator that models the blend
/// and the display's events (`mbx-blend`, rp-v0.13.0), its B1-B3 pixels
/// and E1/E3/E4 timings must match the device's run (docs/hw/emulator.md).
pub fn boot_loader_mbx_blend(root: &Path) -> Result<(), String> {
    if !device_has(root, "intel-2700g", "mbx-2d") {
        eprintln!("    mbx-blend: skipped (this emulator release doesn't model the MBX 2D engine)");
        return Ok(());
    }
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    let loader = dir.join("rp-loader.bin");
    let bin = dir.join("rp-mbx-blend.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-mbx-blend"), &bin)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let check = || -> Result<(), String> {
        let mut mon = crate::screen::Monitor::connect(monitor_port())?;
        mon.query("sendkey ret 200")?;
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
            extra: &["-icount", "shift=0"],
            input: Some((&rp_loadproto::frame(&payload), "RETROPANE-MBX-BLEND: ")),
            during: Some(&check),
            then: Some("RPL END"),
            // -icount makes the run deterministic and slow: 71 s on the
            // owner's PC, past 90 s on CI's runners (2026-10-14).
            timeout: Duration::from_secs(300),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("mbx-blend: never came back to the loader after the button press")?;
    if crashed(log) || log.contains("did not return") {
        return Err(format!("mbx-blend: didn't return cleanly:{log}"));
    }
    for want in [
        "RETROPANE-MBX-BLEND: PASS",
        "E restore: DVECTRL, DINTRE, DINTRCNT as found: yes",
        "B1 Dell (blend 0x09100000, bit 17)",
        "B2 0x09100000:",
        "B3 code 15 src32 plain",
        "B4 fill 0x8000f800 bit 17, blend 0:",
        "B blend state put back to 0: yes",
        "B5 under/overrun bits",
        "B6d the screen shows four 32 bpp ramps",
    ] {
        if !log.contains(want) {
            return Err(format!("mbx-blend: the run log has no `{want}`:{log}"));
        }
    }
    for l in log.lines().filter(|l| {
        [
            "E",
            "B",
            "  ->",
            "  stuck",
            "  after",
            "picture",
            "RETROPANE",
        ]
        .iter()
        .any(|p| l.starts_with(p))
    }) {
        eprintln!("      {}", l.trim_end());
    }
    if device_has(root, "intel-2700g", "mbx-blend") {
        let bad = mbx_blend_mismatches(log, MBX_BLEND_DEVICE);
        if !bad.is_empty() {
            return Err(format!(
                "mbx-blend: {} line(s) differ from the X51v's run:
  {}",
                bad.len(),
                bad.join(
                    "
  "
                )
            ));
        }
        eprintln!(
            "    mbx-blend OK: B1-B3 pixels and the E1/E3/E4 scan-line timings match the X51v's run; state put back, back at the loader on a button press"
        );
    } else {
        eprintln!(
            "    mbx-blend OK: every stage ran, state put back, back at the loader on a button press (this emulator release doesn't model the blend: not compared with the device)"
        );
    }
    mbx_blend_b7(root)?;
    mbx_blend_b8(root)?;
    mbx_blend_b9(root)?;
    mbx_blend_b10(root)?;
    mbx_blend_b11(root)?;
    mbx_blend_b12(root)
}

/// rp-mbx-blend B7's run on the owner's X51v (2026-10-21): the lines the
/// emulator must reproduce.
pub(crate) const MBX_BLEND_B7_DEVICE: &str = include_str!("../../data/rp-mbx-blend-b7-device.txt");

/// The B7 result line the emulator doesn't model (docs/hw/emulator.md):
/// header bit 21 wrote nothing on the X51v, meaning unknown.
pub(crate) const MBX_BLEND_B7_UNMODELLED: &str = "B7c header bit 0x00200000:";

/// Where an emulator run of B7 differs from the device's: each device
/// line (B7a, B7b and B7c's results) must appear in the log exactly, but
/// [`MBX_BLEND_B7_UNMODELLED`]'s.
pub(crate) fn mbx_blend_b7_mismatches(log: &str, device: &str) -> Vec<String> {
    device_mismatches(log, device, &[MBX_BLEND_B7_UNMODELLED])
}

/// Where an emulator run differs from the device's: each device line
/// (not `#` comments) must appear in the log exactly, but those starting
/// with one of `skip`.
pub(crate) fn device_mismatches(log: &str, device: &str, skip: &[&str]) -> Vec<String> {
    device
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .filter(|l| !skip.iter().any(|s| l.starts_with(s)))
        .filter(|want| !log.lines().any(|l| l.trim_end() == *want))
        .map(|want| format!("device `{want}`: not in the emulator's log"))
        .collect()
}

/// rp-mbx-blend B9's and B10's runs on the owner's X51v: the lines an
/// emulator with `mbx-2d-pal` (and `mbx-2d-rop`) must reproduce exactly.
pub(crate) const MBX_BLEND_B9_DEVICE: &str = include_str!("../../data/rp-mbx-blend-b9-device.txt");
pub(crate) const MBX_BLEND_B10_DEVICE: &str =
    include_str!("../../data/rp-mbx-blend-b10-device.txt");

/// rp-mbx-blend B11's run on the owner's X51v (2026-10-22): the lines an
/// emulator with `mbx-2d-rop` must reproduce exactly.
pub(crate) const MBX_BLEND_B11_DEVICE: &str =
    include_str!("../../data/rp-mbx-blend-b11-device.txt");

/// B10 alone (`RP_MBX_BLEND_B10`, the performance gate's P13, V15, V17):
/// all 256 raster operations run (none stuck), each part to its line, the
/// state put back, the image back by itself. The answers are the device's.
pub(crate) fn mbx_blend_b10(root: &Path) -> Result<(), String> {
    let bin = variant_image(root, "rp-mbx-blend", ("RP_MBX_BLEND_B10", "1"))?;
    let loader = emu_dir(root)?.join("rp-loader.bin");
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&rp_loadproto::frame(&payload), "RETROPANE-MBX-BLEND: ")),
            during: None,
            then: Some("RPL END"),
            timeout: Duration::from_secs(300),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("mbx-blend B10: never came back to the loader")?;
    if crashed(log) {
        return Err(format!("mbx-blend B10: crashed:{log}"));
    }
    for want in [
        " of 256 as GDI's ROP3 (pattern = colour word), 0 stuck",
        "B10b 8 bpp fill of 0xa5, 8 pixels: ran ",
        "B10b 24 bpp fill of 0x123456, 8 pixels: ran ",
        "B10c status while 20 fills are queued 0x",
        "B10d a fill with header bit 21: ran ",
        "B10 scale, key and blend put back: yes",
        "RETROPANE-MBX-BLEND: PASS - B10 done",
    ] {
        if !log.contains(want) {
            return Err(format!("mbx-blend B10: the run log has no `{want}`:{log}"));
        }
    }
    // With the fork's ROP model (mbx-2d-rop) and bit 21 (mbx-2d-pal):
    // every line as the X51v's but B10c (the FIFO level while busy).
    if device_has(root, "intel-2700g", "mbx-2d-rop")
        && device_has(root, "intel-2700g", "mbx-2d-pal")
    {
        let wrong = device_mismatches(log, MBX_BLEND_B10_DEVICE, &[]);
        if !wrong.is_empty() {
            return Err(format!(
                "mbx-blend B10: not as on the X51v:
{}",
                wrong.join(
                    "
"
                )
            ));
        }
        eprintln!(
            "    mbx-blend B10 matches the X51v's run line for line (B10c's FIFO level left out)"
        );
    }
    for l in log.lines().filter(|l| l.starts_with("B10")) {
        eprintln!("      {}", l.trim_end());
    }
    eprintln!("    mbx-blend B10 OK: every part ran, state put back, back at the loader");
    Ok(())
}

/// `rp-mbx-blend` B12 (the gate's backlog 7-10: Dell's brush, palette and
/// low-depth words, bits 23/24): the fork doesn't model the brush surface
/// or origin words, so this checks every step ran or was reset cleanly,
/// logged its line, and put the engine's state back; the results are the
/// X51v's.
pub(crate) fn mbx_blend_b12(root: &Path) -> Result<(), String> {
    let bin = variant_image(root, "rp-mbx-blend", ("RP_MBX_BLEND_B12", "1"))?;
    let loader = emu_dir(root)?.join("rp-loader.bin");
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&rp_loadproto::frame(&payload), "RETROPANE-MBX-BLEND: ")),
            during: None,
            then: Some("RPL END"),
            timeout: Duration::from_secs(300),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("mbx-blend B12: never came back to the loader")?;
    if crashed(log) {
        return Err(format!("mbx-blend B12: crashed:{log}"));
    }
    for want in [
        "B12a brush 32 bpp origin (0,0) size field 8: ran ",
        "B12a brush 32 bpp origin (3,5) size field 8: ran ",
        "B12b brush 8 bpp through the brush palette: ran ",
        "B12c 4 bpp source through the source palette: ran ",
        "B12d 2 bpp source through the source palette: ran ",
        "B12e header bits 0x01800000 rect (8,4)-(16,8): ran ",
        "B12f copy onto itself one pixel right, rows of 256, header bits 0x01000000: ran ",
        "B12f copy onto itself one row down, rows of 256, header bits 0x00800000: ran ",
        "B12 scale, key and blend put back: yes",
        "B12 done",
    ] {
        if !log.contains(want) {
            return Err(format!("mbx-blend B12: the run log has no `{want}`:{log}"));
        }
    }
    for l in log.lines().filter(|l| l.starts_with("B12")) {
        eprintln!("      {}", l.trim_end());
    }
    eprintln!("    mbx-blend B12 OK: every step ran, state put back (the results are the X51v's)");
    Ok(())
}

pub(crate) fn mbx_blend_b11(root: &Path) -> Result<(), String> {
    let bin = variant_image(root, "rp-mbx-blend", ("RP_MBX_BLEND_B11", "1"))?;
    let loader = emu_dir(root)?.join("rp-loader.bin");
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&rp_loadproto::frame(&payload), "RETROPANE-MBX-BLEND: ")),
            during: None,
            then: Some("RPL END"),
            timeout: Duration::from_secs(300),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("mbx-blend B11: never came back to the loader")?;
    if crashed(log) {
        return Err(format!("mbx-blend B11: crashed:{log}"));
    }
    for want in [
        "B11a code 0 (header bits 0x00000000) rect 8x4: ran true; 32 source pixels written",
        "B11a code 3 (header bits 0x06000000) rect 4x8: ran ",
        "B11b ROP4 0xaaf0 bit offset 0: ran ",
        "B11b ROP4 0xaaf0 bit offset 3: ran ",
        "B11 scale, key and blend put back: yes",
        "RETROPANE-MBX-BLEND: PASS - B11 done",
    ] {
        if !log.contains(want) {
            return Err(format!("mbx-blend B11: the run log has no `{want}`:{log}"));
        }
    }
    // With the fork's model (mbx-2d-rop): every line as the X51v drew it.
    if device_has(root, "intel-2700g", "mbx-2d-rop") {
        let wrong = device_mismatches(log, MBX_BLEND_B11_DEVICE, &[]);
        if !wrong.is_empty() {
            return Err(format!(
                "mbx-blend B11: not as on the X51v:
{}",
                wrong.join(
                    "
"
                )
            ));
        }
        eprintln!("    mbx-blend B11 matches the X51v's run line for line");
    }
    for l in log.lines().filter(|l| l.starts_with("B11")) {
        eprintln!("      {}", l.trim_end());
    }
    eprintln!("    mbx-blend B11 OK: every part ran, state put back, back at the loader");
    Ok(())
}

/// B9 alone (`RP_MBX_BLEND_B9`, the performance gate's V15, P6, P7, V3):
/// every try must run to its line (a try the engine sticks on is reset and
/// logged, never the end of the run), the state put back, and the image
/// back by itself. The answers are the device's (the fork models the clip,
/// not the scaler, palettes or 1 bpp sources).
pub(crate) fn mbx_blend_b9(root: &Path) -> Result<(), String> {
    let bin = variant_image(root, "rp-mbx-blend", ("RP_MBX_BLEND_B9", "1"))?;
    let loader = emu_dir(root)?.join("rp-loader.bin");
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&rp_loadproto::frame(&payload), "RETROPANE-MBX-BLEND: ")),
            during: None,
            then: Some("RPL END"),
            timeout: Duration::from_secs(300),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("mbx-blend B9: never came back to the loader")?;
    if crashed(log) {
        return Err(format!("mbx-blend B9: crashed:{log}"));
    }
    for want in [
        "B9a clip (8, 8)-(24, 20), bit 18: ran true",
        "B9b scale 1:1: ran ",
        "B9b scale x1.5: ran ",
        "B9c 8 bpp with the palette word: ran ",
        "B9c 8 bpp without it, first: ran ",
        "B9e full screen (us each): 2x upscale from 240 x 320 32-bit Some(",
        "B9d 1 bpp 0x0F: ran ",
        "B9 scale, key and blend put back: yes",
        "RETROPANE-MBX-BLEND: PASS - B9 done",
    ] {
        if !log.contains(want) {
            return Err(format!("mbx-blend B9: the run log has no `{want}`:{log}"));
        }
    }
    // With the fork's palette and scaler (mbx-2d-pal): as the X51v drew.
    if device_has(root, "intel-2700g", "mbx-2d-pal") {
        let wrong = device_mismatches(log, MBX_BLEND_B9_DEVICE, &[]);
        if !wrong.is_empty() {
            return Err(format!(
                "mbx-blend B9: not as on the X51v:
{}",
                wrong.join(
                    "
"
                )
            ));
        }
        eprintln!(
            "    mbx-blend B9 matches the X51v's run line for line (B9c's stale palette and B9e's timings left out)"
        );
    }
    for l in log.lines().filter(|l| l.starts_with("B9")) {
        eprintln!("      {}", l.trim_end());
    }
    eprintln!("    mbx-blend B9 OK: every try ran to its line, state put back, back at the loader");
    Ok(())
}

/// B8 alone (`RP_MBX_BLEND_B8`, the performance gate's P21): the dither
/// add both ways must give the same picture, word for word, and each be
/// timed; the image returns by itself. Timings aren't modelled.
pub(crate) fn mbx_blend_b8(root: &Path) -> Result<(), String> {
    let bin = variant_image(root, "rp-mbx-blend", ("RP_MBX_BLEND_B8", "1"))?;
    let loader = emu_dir(root)?.join("rp-loader.bin");
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&rp_loadproto::frame(&payload), "RETROPANE-MBX-BLEND: ")),
            during: None,
            then: Some("RPL END"),
            timeout: Duration::from_secs(300),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("mbx-blend B8: never came back to the loader")?;
    if crashed(log) {
        return Err(format!("mbx-blend B8: crashed:{log}"));
    }
    for want in [
        "B8 the two ways' results: 0 of 307200 words differ",
        "B8 dither add, full screen: four blits from pattern rows Some(",
        "B8 dither add, 480 x 80: four blits from pattern rows Some(",
        "B8c 10 full-screen fills, the fence polled every 0 us: landed after Some(",
        "B8c 10 full-screen fills, the fence polled every 5000 us: landed after Some(",
        "B8 blend state put back to 0: yes",
        "RETROPANE-MBX-BLEND: PASS - B8 done",
    ] {
        if !log.contains(want) {
            return Err(format!("mbx-blend B8: the run log has no `{want}`:{log}"));
        }
    }
    eprintln!(
        "    mbx-blend B8 OK: both ways of adding the dither give the same picture, both timed (timings not modelled)"
    );
    Ok(())
}

/// B7 alone (`RP_MBX_BLEND_B7`, ADR-0023's 16-bit screen): every part
/// must run to its line and the image return to the loader by itself (no
/// button); on an emulator that models the blend (`mbx-blend`), its
/// conversion, blend and dither-try results must equal the X51v's run
/// (timings aren't modelled, header bit 21 isn't: docs/hw/emulator.md).
pub(crate) fn mbx_blend_b7(root: &Path) -> Result<(), String> {
    let bin = variant_image(root, "rp-mbx-blend", ("RP_MBX_BLEND_B7", "1"))?;
    let loader = emu_dir(root)?.join("rp-loader.bin");
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&rp_loadproto::frame(&payload), "RETROPANE-MBX-BLEND: ")),
            during: None,
            then: Some("RPL END"),
            timeout: Duration::from_secs(300),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("mbx-blend B7: never came back to the loader")?;
    if crashed(log) || log.contains("did not return") {
        return Err(format!("mbx-blend B7: didn't return cleanly:{log}"));
    }
    let tries = rp_mbx_blend_tries();
    for want in [
        "RETROPANE-MBX-BLEND: PASS - B7 done",
        "B7a convert red: of 256 levels",
        "B7a convert green: of 256 levels",
        "B7a convert blue: of 256 levels",
        "B7b over alpha 0xff red:",
        "B7b over alpha 0x80 blue:",
        "B7d convert 32 to 16 bits, full screen: Some(",
        "B7d copy 16 to 16 bits (a forward copy), full screen: Some(",
        "B7d dither add at 32 bits (4 blits, blend 0), full screen: Some(",
        "B7 blend state put back to 0: yes",
    ] {
        if !log.contains(want) {
            return Err(format!("mbx-blend B7: the run log has no `{want}`:{log}"));
        }
    }
    let done = log
        .lines()
        .filter(|l| l.starts_with("B7c ") && l.contains(": trunc "))
        .count();
    if done != tries {
        return Err(format!(
            "mbx-blend B7: {done} of {tries} B7c tries logged a result:{log}"
        ));
    }
    for l in log.lines().filter(|l| l.starts_with("B7")) {
        eprintln!("      {}", l.trim_end());
    }
    if !device_has(root, "intel-2700g", "mbx-blend") {
        eprintln!(
            "    mbx-blend B7 OK: every part ran ({tries} B7c tries), blend state put back, back at the loader by itself (this emulator release doesn't model the blend: not compared with the device)"
        );
        return Ok(());
    }
    // Header bit 21 is modelled with mbx-2d-pal (fork branch rp-next).
    let bit21 = device_has(root, "intel-2700g", "mbx-2d-pal");
    let bad = if bit21 {
        device_mismatches(log, MBX_BLEND_B7_DEVICE, &[])
    } else {
        mbx_blend_b7_mismatches(log, MBX_BLEND_B7_DEVICE)
    };
    if !bad.is_empty() {
        return Err(format!(
            "mbx-blend B7: {} line(s) differ from the X51v's run:\n  {}",
            bad.len(),
            bad.join("\n  ")
        ));
    }
    eprintln!(
        "    mbx-blend B7 OK: every part ran ({tries} B7c tries), the conversion, blends and tries match the X51v's run ({}), back at the loader by itself",
        if bit21 {
            "header bit 21 too"
        } else {
            "header bit 21 not modelled"
        }
    );
    Ok(())
}

/// B7c's tries (`rp-mbx-blend`'s `dither_tries`: 1 plain, 4 header bits,
/// 1 blend path, 24 blend-word bits).
pub(crate) const fn rp_mbx_blend_tries() -> usize {
    1 + 4 + 1 + 4 + 20
}

/// What rp-blend's `stage` should put on screen: the image's own drawing
/// and register programming (shared code), through the reference model.
pub(crate) fn blend_screen(stage: rp_drv_mbx_display::blendtest::Stage) -> Vec<u8> {
    use rp_drv_mbx_display::{Display, blendtest, scanout};
    let mut vram = vec![0u8; 16 << 20];
    let mut d = Display::new(RegFile::default());
    let _ = d.adopt(vram.len());
    blendtest::draw(stage, &mut vram, &mut vec![0u8; blendtest::WORK]);
    blendtest::program(&mut d, stage);
    let regs = d.into_inner();
    let mut out = vec![0u8; blendtest::W * blendtest::H * 3];
    scanout::compose(|o| regs.0.get(&o).copied().unwrap_or(0), &vram, &mut out);
    out
}

/// The performance gate's P4 test image (rp-tabench): runs under the
/// loader. With the fork's 3D model the CPU binner's way draws every set
/// and its frames 2-5 repeat the first; the fork doesn't model the tile
/// accelerator, so every TA way must run, time out ("TA done 0") and move
/// on, which is the hang path a device TA hang takes (Q2 A). The clock,
/// plane and core put back, it returns on a button.
pub fn boot_loader_tabench(root: &Path) -> Result<(), String> {
    if !device_has(root, "intel-2700g", "mbx-3d") {
        eprintln!("    tabench: skipped (this emulator release doesn't model the MBX 3D core)");
        return Ok(());
    }
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    let loader = dir.join("rp-loader.bin");
    let bin = dir.join("rp-tabench.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-tabench"), &bin)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let check = || -> Result<(), String> {
        let mut mon = crate::screen::Monitor::connect(monitor_port())?;
        mon.query("sendkey ret 200")?;
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
            input: Some((&rp_loadproto::frame(&payload), "RETROPANE-TABENCH: DONE")),
            during: Some(&check),
            then: Some("RPL END"),
            timeout: Duration::from_secs(3600),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("tabench: never came back to the loader after the button press")?;
    if crashed(log) || log.contains("did not return") {
        return Err(format!("tabench: didn't return cleanly:{log}"));
    }
    for want in [
        "SET 6 of 6",
        "the TA set up once (the driver's ta::set_up",
        "): ready true;",
        "RETROPANE-TABENCH: DONE",
    ] {
        if !log.contains(want) {
            return Err(format!("tabench: the run log has no `{want}`:{log}"));
        }
    }
    let binner = log.matches("  CPU BINNER: TA done 5;").count();
    let repeated = log
        .lines()
        .filter(|l| l.starts_with("  CPU BINNER:") && l.contains("frames 2-5 as the first: 4 of 4"))
        .count();
    let binned = log.matches("    binned: ").count();
    let ta = log.lines().filter(|l| l.starts_with("  TA, ")).count();
    if binner != 6 || repeated != 6 || binned != 6 || ta != 30 || log.contains("render past 2 s") {
        return Err(format!(
            "tabench: {binned} sets binned, {binner} binner ways drawn, {repeated} repeatable, {ta} TA ways run (6, 6, 6, 30 wanted, no render past 2 s):{log}"
        ));
    }
    eprintln!(
        "    tabench OK: the binner drew all six sets repeatably on the modelled 3D core; all 30 TA ways ran and timed out safely (the fork has no TA); back at the loader on a button press"
    );
    boot_loader_tabench_isolate(root)
}

/// Run 8 of rp-tabench (`RP_TABENCH_ISOLATE=1`, audit C81: the first render after a set-up): every way's
/// binner reference draws on the modelled 3D core; every TA frame times
/// out (the fork has no TA) and the run moves on; it returns on a button.
pub(crate) fn boot_loader_tabench_isolate(root: &Path) -> Result<(), String> {
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    let loader = dir.join("rp-loader.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    let bin = variant_image(root, "rp-tabench", ("RP_TABENCH_ISOLATE", "1"))?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let check = || -> Result<(), String> {
        let mut mon = crate::screen::Monitor::connect(monitor_port())?;
        mon.query("sendkey ret 200")?;
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
            input: Some((&rp_loadproto::frame(&payload), "RETROPANE-TABENCH: DONE")),
            during: Some(&check),
            then: Some("RPL END"),
            timeout: Duration::from_secs(1800),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("tabench run 8: never came back to the loader after the button press")?;
    if crashed(log) || log.contains("did not return") {
        return Err(format!("tabench run 8: didn't return cleanly:{log}"));
    }
    let ways = log
        .lines()
        .filter(|l| l.contains(" set-ups, frame 1 hung "))
        .count();
    let done = "RETROPANE-TABENCH: DONE - run 8: 0 of ";
    if !log.contains("the binner's reference: rendered in ")
        || log.contains("the binner's reference: rendered in 0 us")
        || ways != 5
        || !log.contains(done)
    {
        return Err(format!(
            "tabench run 8: the reference drawn and 5 ways run wanted ({ways} ways), every TA frame timed out:{log}"
        ));
    }
    eprintln!(
        "    tabench run 8 OK: the binner's reference drew on the modelled 3D core; all 5 ways ran, every TA warm-up and frame timed out safely (the fork has no TA)"
    );
    Ok(())
}
