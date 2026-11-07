//! The display and blend checks: rp-picture, rp-blend (scale, filter, stages).

#[allow(clippy::wildcard_imports)]
use super::*;

/// The concept-picture image (rp-picture): the loader starts it, the screen
/// must show the picture exactly (the screendump against the source, after
/// RGB565), and after its 20 s it must be back at the loader through a
/// normal return: no run-limit crash, no reset. Only then may it go to the
/// device.
pub fn boot_loader_picture(root: &Path) -> Result<(), String> {
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    let loader = dir.join("rp-loader.bin");
    let bin = dir.join("rp-picture.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-picture"), &bin)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let want = expected_picture(root)?;
    let shot = scratch(root, "picture-screen.ppm")?;
    let guest = match emulator(root)? {
        Emulator::Wsl => to_wsl_path(&shot),
        Emulator::Native(_) => shot.display().to_string(),
    };
    let check = || -> Result<(), String> {
        // Called as soon as the image says the picture is up.
        let shown = Instant::now();
        // The emulator redraws its screen on a timer: give it time to show
        // the new frame (as the crash-screen check does).
        guest_hold(
            &mut crate::screen::Monitor::connect(monitor_port())?,
            1500,
            "picture",
        )?;
        let _ = std::fs::remove_file(&shot);
        let mut mon = crate::screen::Monitor::connect(monitor_port())?;
        mon.screendump(&guest, &shot)?;
        let img = crate::screen::parse_ppm(&std::fs::read(&shot).map_err(|e| e.to_string())?)?;
        if (img.width, img.height) != (480, 640) {
            return Err(format!("picture: screen is {}x{}", img.width, img.height));
        }
        // The screendump widens RGB565 to 8 bits; compare in RGB565 terms.
        let mut off = 0usize;
        for (i, px) in want.as_chunks::<2>().0.iter().enumerate() {
            let v = u16::from_le_bytes(*px);
            let (r, g, b) = (img.rgb[i * 3], img.rgb[i * 3 + 1], img.rgb[i * 3 + 2]);
            if u16::from(r >> 3) != v >> 11
                || u16::from(g >> 2) != (v >> 5) & 63
                || u16::from(b >> 3) != v & 31
            {
                off += 1;
            }
        }
        if off != 0 {
            return Err(format!(
                "picture: {off} of 307200 pixels differ from the source"
            ));
        }
        // Its return is awaited after the check (the loader's RPL END).
        let _ = shown;
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
            input: Some((&rp_loadproto::frame(&payload), "RETROPANE-PICTURE-SHOWN")),
            during: Some(&check),
            then: Some("RPL END"),
            timeout: Duration::from_secs(90),
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
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            format!(
                "picture: never came back to the loader; serial ends:
{tail}"
            )
        })?;
    if !log.contains("RETROPANE-PICTURE-SHOWN") {
        return Err(format!(
            "picture: the run log doesn't have the marker:{log}"
        ));
    }
    if crashed(log) || log.contains("did not return") {
        return Err(format!("picture: didn't return cleanly:{log}"));
    }
    eprintln!(
        "    picture OK: shown pixel-exact, back at the loader after {} s with no crash or reset",
        rp_picture_seconds()
    );
    Ok(())
}

/// The plane-blending image (rp-blend, overnight item 60): the loader
/// starts it; each of its stages must show exactly what the
/// reference model (`rp_drv_mbx_display::scanout`) computes from the same
/// drawing and register programming, which a host test already checks
/// against the intended colours. Each screendump is matched against all
/// the stages, so a slow host can't pass one stage off as another. Then
/// it must be back at the loader with the wallpaper gone.
/// `rp-blend`'s P9 scaler steps (`RP_BLEND_SCALE`, the gate's P9/V5): the
/// emulator doesn't model the video plane's scaler (docs/hw/emulator.md),
/// so this checks what it can: every step comes up, the scaler registers
/// read back as `layers::VideoPlane::registers_scaled` wrote them for that
/// step, the display put back and the image returned. Whether the picture
/// is right is the owner's eye on the X51v.
pub fn boot_loader_blend_scale(root: &Path) -> Result<(), String> {
    use rp_drv_mbx_display::layers::{Layer, Scale, VideoFormat, VideoPlane, reg};
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    let loader = dir.join("rp-loader.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    let bin = variant_image_with(
        root,
        "rp-blend",
        ("RP_BLEND_SCALE", "1"),
        Some(("RP_BLEND_SECONDS", "2")),
    )?;
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
            input: Some((&rp_loadproto::frame(&payload), "RPL OK")),
            during: None,
            then: Some("RPL END"),
            timeout: Duration::from_secs(300),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("blend P9: no run log after the image returned")?;
    if crashed(log) {
        return Err(format!("blend P9: crashed:{log}"));
    }
    let wall = VideoPlane::new(VideoFormat::Uyvy, 480, 640, 0x50_0000);
    let lines: Vec<&str> = log
        .lines()
        .filter(|l| l.starts_with("P9 scaler:"))
        .collect();
    if lines.len() != rp_blend_steps().len() {
        return Err(format!(
            "blend P9: want {} scaler lines, got {}:{log}",
            rp_blend_steps().len(),
            lines.len()
        ));
    }
    for (line, (src, out)) in lines.iter().zip(rp_blend_steps()) {
        let mut v = VideoPlane::new(VideoFormat::Uyvy, src.2, src.3, wall.offset);
        v.stride = wall.stride;
        let scale = Scale {
            out_w: out.0,
            out_h: out.1,
            x: out.2,
            y: out.3,
            taps: out.4,
            ur_c: out.5,
            filter: rp_drv_mbx_display::layers::ScalerFilter::Mbxfb,
        };
        let regs = v.registers_scaled(Layer::SOLO, false, scale);
        let want = |o: usize| regs.iter().find(|r| r.0 == o).map_or(0, |r| r.1);
        let expect = format!(
            "P9 scaler: VSADR {:08X} SSSIZE {:08X} SPOCTRL {:08X} SHCTRL {:08X}",
            want(reg::VSADR),
            want(reg::SSSIZE),
            want(reg::SPOCTRL),
            want(reg::SHCTRL)
        );
        if line.trim() != expect {
            return Err(format!("blend P9: read back {line:?}, wrote {expect:?}"));
        }
    }
    let steps = log.lines().filter(|l| l.starts_with("P9 step ")).count();
    if steps != rp_blend_steps().len() || !log.contains("restored:") {
        return Err(format!(
            "blend P9: {steps} steps held, or nothing put back:{log}"
        ));
    }
    eprintln!(
        "    blend P9 OK: {steps} scaler steps up, each read back as written, put back (the picture: the X51v, watched)"
    );
    Ok(())
}

/// `rp-blend`'s run P9d (`RP_BLEND_FILTER`, gate V5): every step comes
/// up; the scaler's size, pitch and place registers read back as
/// `registers_scaled` wrote them, and on every scaling step the filter's
/// fifteen registers as `filter_words` computed them (the fork stores
/// them: docs/hw/emulator.md), so the fitted filter and the two proof sets
/// reach the chip as the host tests built them; `GDRCTRL`'s doubling set
/// on the last step only; the display put back. The pictures are the
/// owner's eye on the X51v.
pub fn boot_loader_blend_filter(root: &Path) -> Result<(), String> {
    use rp_drv_mbx_display::blendtest::{FILTER_STEPS, FILTER2_STEPS};
    blend_filter_run(root, "RP_BLEND_FILTER", &FILTER_STEPS, "P9d")?;
    blend_filter_run(root, "RP_BLEND_FILTER2", &FILTER2_STEPS, "P9e")
}

/// One of [`boot_loader_blend_filter`]'s runs: `flag`'s image, `steps`.
pub(crate) fn blend_filter_run(
    root: &Path,
    flag: &str,
    steps: &[rp_drv_mbx_display::blendtest::FilterStep],
    name: &str,
) -> Result<(), String> {
    use rp_drv_mbx_display::blendtest::{self, GDRCTRL_DOUBLE};
    use rp_drv_mbx_display::layers::{Layer, VideoFormat, VideoPlane, reg};
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    let loader = dir.join("rp-loader.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    let bin = variant_image_with(
        root,
        "rp-blend",
        (flag, "1"),
        Some(("RP_BLEND_SECONDS", "2")),
    )?;
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
            input: Some((&rp_loadproto::frame(&payload), "RPL OK")),
            during: None,
            then: Some("RPL END"),
            timeout: Duration::from_secs(300),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or_else(|| format!("blend {name}: no run log after the image returned"))?;
    if crashed(log) {
        return Err(format!("blend {name}: crashed:{log}"));
    }
    let scaler: Vec<&str> = log
        .lines()
        .filter(|l| l.starts_with("P9 scaler:"))
        .collect();
    let filter: Vec<&str> = log
        .lines()
        .filter(|l| l.starts_with("P9d filter:"))
        .collect();
    if scaler.len() != steps.len() || filter.len() != steps.len() {
        return Err(format!(
            "blend {name}: want {} scaler and filter lines, got {} and {}:{log}",
            steps.len(),
            scaler.len(),
            filter.len()
        ));
    }
    let mut filters_checked = 0;
    for (i, step) in steps.iter().enumerate() {
        let (_, _, w, h) = step.src;
        let mut v = VideoPlane::new(VideoFormat::Uyvy, w, h, blendtest::FILTER_AT);
        v.stride = blendtest::W * 2;
        let regs = v.registers_scaled(Layer::SOLO, false, step.scale);
        let want = |o: usize| regs.iter().find(|r| r.0 == o).map_or(0, |r| r.1);
        let expect = format!(
            "P9 scaler: VSADR {:08X} SSSIZE {:08X} SPOCTRL {:08X} SHCTRL {:08X}",
            want(reg::VSADR),
            want(reg::SSSIZE),
            want(reg::SPOCTRL),
            want(reg::SHCTRL)
        );
        if scaler[i].trim() != expect {
            return Err(format!(
                "blend {name} step {}: read back {:?}, wrote {expect:?}",
                i + 1,
                scaler[i]
            ));
        }
        let (words, gdr) = filter[i]
            .trim()
            .strip_prefix("P9d filter:")
            .and_then(|l| l.split_once("; GDRCTRL "))
            .ok_or_else(|| format!("blend {name} step {}: {:?}", i + 1, filter[i]))?;
        let gdr = u32::from_str_radix(gdr.trim(), 16).map_err(|e| format!("{e}: {gdr}"))?;
        if (gdr & GDRCTRL_DOUBLE == GDRCTRL_DOUBLE) != step.double {
            return Err(format!(
                "blend {name} step {}: GDRCTRL {gdr:08X}, doubling wanted {}",
                i + 1,
                step.double
            ));
        }
        if v.scales(step.scale) {
            let want: Vec<String> = v
                .filter_words(step.scale)
                .iter()
                .map(|(_, w)| format!("{w:08X}"))
                .collect();
            if words.split_whitespace().collect::<Vec<_>>() != want {
                return Err(format!(
                    "blend {name} step {}: filter read back {words:?}, wrote {want:?}",
                    i + 1
                ));
            }
            filters_checked += 1;
        }
    }
    let held = log.lines().filter(|l| l.starts_with("P9d step ")).count();
    if held != steps.len() || !log.contains("restored:") {
        return Err(format!(
            "blend {name}: {held} steps held, or nothing put back:{log}"
        ));
    }
    eprintln!(
        "    blend {name} OK: {held} steps up, the scaler and {filters_checked} filters read back as computed, the doubling only where set, put back (the pictures: the X51v, watched)"
    );
    Ok(())
}

/// A P9 step's source rectangle (x, y, w, h) and output (w, h, x, y, taps).
pub(crate) type BlendSrc = (u16, u16, u16, u16);
pub(crate) type BlendOut = (u16, u16, u16, u16, u8, bool);

/// `rp-blend`'s `SCALE_STEPS` as (source, output) tuples, kept in step with
/// the image's table by the check above (a mismatch shows as a read-back
/// difference).
pub(crate) fn rp_blend_steps() -> [(BlendSrc, BlendOut); 9] {
    [
        ((0, 0, 480, 640), (0, 0, 0, 0, 1, false)),
        ((0, 0, 240, 320), (480, 640, 0, 0, 4, false)),
        ((0, 0, 240, 320), (480, 640, 0, 0, 1, false)),
        ((0, 0, 480, 640), (240, 320, 120, 160, 4, false)),
        ((0, 0, 480, 640), (360, 480, 60, 80, 4, false)),
        ((0, 400, 240, 240), (0, 0, 240, 320, 1, false)),
        ((0, 0, 240, 320), (480, 640, 0, 0, 1, true)),
        ((0, 0, 480, 640), (240, 320, 120, 160, 4, true)),
        ((0, 0, 480, 640), (360, 480, 60, 80, 4, true)),
    ]
}

pub fn boot_loader_blend(root: &Path) -> Result<(), String> {
    use rp_drv_mbx_display::blendtest::{self, Stage};
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    let loader = dir.join("rp-loader.bin");
    let bin = dir.join("rp-blend.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-blend"), &bin)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let want: Vec<Vec<u8>> = Stage::ALL.iter().map(|&s| blend_screen(s)).collect();
    let wsl = matches!(emulator(root)?, Emulator::Wsl);
    // Kept per stage (target/emulator/blend-A.ppm ...) for a look by eye.
    let dump = |name: char| -> Result<Vec<u8>, String> {
        let shot = scratch(root, &format!("blend-{name}.ppm"))?;
        let guest = if wsl {
            to_wsl_path(&shot)
        } else {
            shot.display().to_string()
        };
        let _ = std::fs::remove_file(&shot);
        let mut mon = crate::screen::Monitor::connect(monitor_port())?;
        mon.screendump(&guest, &shot)?;
        let img = crate::screen::parse_ppm(&std::fs::read(&shot).map_err(|e| e.to_string())?)?;
        if (img.width as usize, img.height as usize) != (blendtest::W, blendtest::H) {
            return Err(format!("blend: screen is {}x{}", img.width, img.height));
        }
        Ok(img.rgb)
    };
    let stage_time = Duration::from_secs(u64::from(RP_BLEND_STAGE_SECONDS));
    let check = || -> Result<(), String> {
        // Called as soon as stage A is up. Each stage's screen must appear,
        // in order: poll the screen rather than trust timing. The image
        // holds a stage for its time in *emulated* seconds, which run
        // slower than the wall clock when the PC is busy (a full suite), so
        // a skipped stage is caught by the order (a later stage showing
        // first), and the deadline is only a generous safety net.
        let safety = stage_time * 4 + Duration::from_secs(20);
        for (i, stage) in Stage::ALL.iter().enumerate() {
            let deadline = Instant::now() + safety;
            loop {
                poll_pause(Duration::from_millis(700));
                let got = dump(stage.letter())?;
                let differ = |w: &[u8]| {
                    got.chunks(3)
                        .zip(w.chunks(3))
                        .filter(|(a, b)| a != b)
                        .count()
                };
                let counts: Vec<usize> = want.iter().map(|w| differ(w)).collect();
                if counts[i] == 0 {
                    break;
                }
                if counts[i + 1..].contains(&0) || Instant::now() > deadline {
                    return Err(format!(
                        "blend: stage {} never showed as the reference has it; pixels differing from each stage: {counts:?}",
                        stage.letter()
                    ));
                }
            }
        }
        // After the last stage, it puts everything back and returns; the
        // loader takes the screen again: no trace of the wallpaper (polled,
        // for the same reason).
        let bars = |s: &[u8]| s[(435 * blendtest::W + 90) * 3..][..3].to_vec();
        let deadline = Instant::now() + safety;
        loop {
            poll_pause(Duration::from_millis(700));
            let got = dump('R')?;
            if !want.iter().any(|w| bars(w) == bars(&got)) {
                return Ok(());
            }
            if Instant::now() > deadline {
                return Err("blend: the wallpaper is still on screen after the return".into());
            }
        }
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
            input: Some((&rp_loadproto::frame(&payload), "RETROPANE-BLEND-A")),
            during: Some(&check),
            then: Some("RPL END"),
            timeout: Duration::from_secs(600),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("blend: never came back to the loader")?;
    for want in [
        "RETROPANE-BLEND-A",
        "RETROPANE-BLEND-B",
        "RETROPANE-BLEND-C",
        "RETROPANE-BLEND-D",
        "RETROPANE-BLEND-E",
        "RETROPANE-BLEND-F",
        "restored:",
    ] {
        if !log.contains(want) {
            return Err(format!("blend: the run log has no `{want}`:{log}"));
        }
    }
    if crashed(log) || log.contains("did not return") {
        return Err(format!("blend: didn't return cleanly:{log}"));
    }
    eprintln!(
        "    blend OK: every stage pixel-exact against the reference, registers put back, back at the loader"
    );
    Ok(())
}
