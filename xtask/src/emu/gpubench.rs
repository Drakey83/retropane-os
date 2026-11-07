//! rp-gpubench's emulator runs: every variant and its checks.

#[allow(clippy::wildcard_imports)]
use super::*;

/// The GPU measuring run for the 3D compositor proposal (rp-gpubench,
/// docs/design/ui.md): run A (fill, layers, partial frames, rectangular
/// textures, uploads, the frame-event watch tests) and run B (the
/// texture-word probes, built with `RP_GPUBENCH_PROBES`), each under the
/// loader, then run C (the video plane's repoint and pans,
/// `RP_GPUBENCH_PLANES`) and run D (its undocumented format codes,
/// `RP_GPUBENCH_VFMT`), run E (faster uploads, `RP_GPUBENCH_UPLOAD`) and
/// run F (text as 4 bpp strips, `RP_GPUBENCH_TEXT`), runs P, P2 and P2W
/// (the 3D core's target formats, `RP_GPUBENCH_TARGETS`,
/// `RP_GPUBENCH_TARGETS2`, `RP_GPUBENCH_TARGETS_WATCH`) and the clock runs,
/// with key presses for their watch steps. The
/// models' times say nothing about the device's (the fork's 3D
/// core doesn't model the clock or the fill costs: emulator.md), so this
/// checks that every stage runs, the 2700G memory pool is all given back,
/// the two upload paths write the same texture, nothing faults, and each
/// run returns to the loader. Run A's watch tests end at their 30 s cap
/// (or at once if the scan line isn't modelled).
pub fn boot_loader_gpubench(root: &Path) -> Result<(), String> {
    if !device_has(root, "intel-2700g", "mbx-2d") {
        eprintln!("    gpubench: skipped (this emulator release doesn't model the MBX 2D engine)");
        return Ok(());
    }
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    let loader = dir.join("rp-loader.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    let run_a = dir.join("rp-gpubench.bin");
    crate::sdkit::raw_binary(&release.join("rp-gpubench"), &run_a)?;
    // `RETROPANE_GPUBENCH_RUNS="run E,run FT"`: only those runs (a new
    // run's check without the others); the card logs are numbered by the
    // runs that ran. Their variant images alone are built (below, as each
    // run starts).
    let only: Option<Vec<String>> = std::env::var("RETROPANE_GPUBENCH_RUNS")
        .ok()
        .map(|v| v.split(',').map(|n| n.trim().to_string()).collect());
    if only.is_some() {
        DEFERRED.with(|d| *d.borrow_mut() = Some(Deferred::new()));
    }
    let variants = variant_images_gpubench(root, &dir, &run_a, only.as_deref());
    DEFERRED.with(|d| *d.borrow_mut() = None);
    variants
}

/// [`boot_loader_gpubench`] with its runs' images named (built, or their
/// builds deferred).
#[allow(clippy::too_many_lines)]
pub(crate) fn variant_images_gpubench(
    root: &Path,
    dir: &Path,
    run_a: &Path,
    only: Option<&[String]>,
) -> Result<(), String> {
    let loader = dir.join("rp-loader.bin");
    let run_a = run_a.to_path_buf();
    let run_b = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_PROBES", "1"))?;
    let run_c = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_PLANES", "1"))?;
    let run_d = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_VFMT", "1"))?;
    let run_e = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_UPLOAD", "1"))?;
    let run_f = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_TEXT", "1"))?;
    let run_g = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_CLOCK", "1"))?;
    let run_r = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_REVIVE", "1"))?;
    let run_s = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_SNAPSHOT", "1"))?;
    let run_u = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_UNDERRUN", "1"))?;
    let run_v = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_LOAD", "1"))?;
    let run_x = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_CLEAR", "1"))?;
    let run_l = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_LADDER", "1"))?;
    let run_z = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_REBUILD", "1"))?;
    let run_y = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_RESCUE", "1"))?;
    let run_n = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_START", "1"))?;
    let run_q = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_SLOW", "1"))?;
    let run_p = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_TARGETS", "1"))?;
    let run_p2 = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_TARGETS2", "1"))?;
    let run_p2w = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_TARGETS_WATCH", "1"))?;
    let run_w = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_FINGERPRINT", "1"))?;
    let run_t = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_FETCH", "1"))?;
    let run_t3 = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_RESOLVE", "1"))?;
    let run_p2x = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_SPECKS", "1"))?;
    let run_p2y = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_SPECKS2", "1"))?;
    let run_p2l = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_LONG_COUNT", "1"))?;
    let run_ff = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_FIFO_FULL", "1"))?;
    let run_fs = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_REAL_STALL", "1"))?;
    let run_ft = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_TOGETHER", "1"))?;
    let run_fi = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_IRQ_BITS", "1"))?;
    let run_fd = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_DMA_FEED", "1"))?;
    let run_ch = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_HARNESS", "1"))?;
    // Run WG (the Weather movement on the planes): the watched block's
    // wait 1 s and its repeats cut; the device runs every repeat.
    let run_wgr = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_WEATHER", "1"),
            ("RP_GPUBENCH_WG_R", "1"),
            ("RP_GPUBENCH_WG_QUICK", "1"),
        ],
    )?;
    let run_wg = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_WEATHER", "1"),
            ("RP_GPUBENCH_WG_QUICK", "1"),
            ("RP_GPUBENCH_WG_WAIT_SECS", "1"),
        ],
    )?;
    let run_c71 = variant_image_with(
        root,
        "rp-gpubench",
        ("RP_GPUBENCH_HARNESS", "1"),
        Some(("RP_GPUBENCH_C71", "1")),
    )?;
    let run_c71_rb = variant_image_with(
        root,
        "rp-gpubench",
        ("RP_GPUBENCH_HARNESS", "1"),
        Some(("RP_GPUBENCH_C71_RB", "1")),
    )?;
    let run_g21 = variant_image_with(
        root,
        "rp-gpubench",
        ("RP_GPUBENCH_HARNESS", "1"),
        Some(("RP_GPUBENCH_G21", "1")),
    )?;
    // Run G21: every kind's reference repeatable, and every way's 20
    // renders exact against it (the fork is exact: the device's are the
    // finding), A's copies exact, C's none.
    let g21_kinds = [
        "flat cards 200 x 260",
        "textured cards 1:1 (128 x 128)",
        "textured cards 1.56x, point sampling",
        "textured cards 1.56x, bilinear",
        "textured cards shrunk 2:1 (64 x 64), point sampling",
    ];
    let mut g21_wants: Vec<String> = vec!["  CH G21: the screen at 0x0 (GSCADR ".into()];
    for k in g21_kinds {
        g21_wants.push(format!(
            "  CH G21 {k}: the reference (the reference: off screen, the display paused): rendered Some("
        ));
        g21_wants.push(" the last 0 words off the one kept []; hash ".into());
        for (way, copies) in [
            ("A: off screen, a 2D copy onto the screen", 20),
            ("C: drawn straight into the screen", 0),
            ("C with the display paused", 0),
        ] {
            g21_wants.push(format!(
                "  CH G21 {k}: {way}: 20 renders (0 unfinished), 0 differ from the reference, 0 words in all []; render fastest "
            ));
            g21_wants.push(format!("us; {copies} 2D copies onto the screen, fastest "));
        }
    }
    g21_wants.extend(
        [
            "  CH G21: done",
            "everything given back: true",
            "RETROPANE-GPUBENCH: done",
        ]
        .map(String::from),
    );
    let g21_wants: Vec<&str> = g21_wants.iter().map(String::as_str).collect();
    let run_lim = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_LIMITS", "1"))?;
    let run_fi2 = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_STATUS2", "1"))?;
    let run_bx = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_BX", "1"))?;
    // Run GS (the clocks' soak) with 70 s a setting: one minute's line and
    // the closing line each.
    let run_gs = variant_image_with(
        root,
        "rp-gpubench",
        ("RP_GPUBENCH_SOAK", "1"),
        Some(("RP_GPUBENCH_SOAK_SECS", "70")),
    )?;
    let run_gv = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_MEMCLOCK", "1"))?;
    let run_gc2f = variant_image_with(
        root,
        "rp-gpubench",
        ("RP_GPUBENCH_GC2_F", "1"),
        Some(("RP_GPUBENCH_GC2_RENDERS", "1")),
    )?;
    let run_gc2h = variant_image_with(
        root,
        "rp-gpubench",
        ("RP_GPUBENCH_GC2_H", "1"),
        Some(("RP_GPUBENCH_GC2_RENDERS", "1")),
    )?;
    let run_gc2r = variant_image_with(
        root,
        "rp-gpubench",
        ("RP_GPUBENCH_GC2_R", "1"),
        Some(("RP_GPUBENCH_GC2_RENDERS", "1")),
    )?;
    // Every 37th flip: the render table, MBXCLK, the system registers and
    // COREPLL all sampled.
    let run_gc2x = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_GC2X_STEP", "37"))?;
    let run_gc2h2 = variant_image_with(
        root,
        "rp-gpubench",
        ("RP_GPUBENCH_GC2_H2", "1"),
        Some(("RP_GPUBENCH_GC2_RENDERS", "1")),
    )?;
    let run_gc2w = variant_image_with(
        root,
        "rp-gpubench",
        ("RP_GPUBENCH_GC2_W", "1"),
        Some(("RP_GPUBENCH_GC2_RENDERS", "1")),
    )?;
    let run_gc3 = variant_image_with(
        root,
        "rp-gpubench",
        ("RP_GPUBENCH_GC3", "1"),
        Some(("RP_GPUBENCH_GC2_RENDERS", "1")),
    )?;
    let run_gc2m = variant_image_with(
        root,
        "rp-gpubench",
        ("RP_GPUBENCH_GC2M", "1"),
        Some(("RP_GPUBENCH_GC2_RENDERS", "1")),
    )?;
    let run_gc2d = variant_image_with(
        root,
        "rp-gpubench",
        ("RP_GPUBENCH_GC2_D", "1"),
        Some(("RP_GPUBENCH_GC2_RENDERS", "1")),
    )?;
    let run_gc = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_CMAP", "1"))?;
    let run_gc2 = variant_image_with(
        root,
        "rp-gpubench",
        ("RP_GPUBENCH_GC2", "1"),
        Some(("RP_GPUBENCH_GC2_RENDERS", "1")),
    )?;
    let run_gc2s = variant_image_with(
        root,
        "rp-gpubench",
        ("RP_GPUBENCH_GC2_SAFE", "1"),
        Some(("RP_GPUBENCH_GC2_RENDERS", "1")),
    )?;
    let run_gc2i = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_INSIDE", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "1"),
            ("RP_GPUBENCH_SOAK_SECS", "10"),
        ],
    )?;
    let run_gc2j = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_SOAK2", "1"),
            ("RP_GPUBENCH_SOAK_SECS", "10"),
        ],
    )?;
    let run_gc2k = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_PATCH", "1"),
            ("RP_GPUBENCH_SOAK_SECS", "10"),
        ],
    )?;
    let run_gc2l = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_SWEEP", "1"),
            ("RP_GPUBENCH_SOAK_SECS", "10"),
        ],
    )?;
    let run_gc2o = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_ORDER", "1"),
            ("RP_GPUBENCH_SOAK_SECS", "10"),
        ],
    )?;
    let run_gc2p = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_FAST", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2q = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_WRITEOUT", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_GC2Q_FORCE", "1"),
        ],
    )?;
    let run_gc2t = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_TIMING", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2u = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_TEXLOAD", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2v = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_EDGE", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2y = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_CURED", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_GC2Y_SOAK_SECS", "20"),
        ],
    )?;
    let run_gc2z = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_FIX", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2aa = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_FREE", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_GC2AA_SOAK_SECS", "20"),
        ],
    )?;
    let run_gc2ab = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_LASTPIX", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2ac = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_RING", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2ad = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_NIGHT", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_GC2AD_SOAK_SECS", "20"),
        ],
    )?;
    let run_gc2ae = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_TRACE", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_GC2AE_ROUNDS", "2"),
        ],
    )?;
    let run_gc2af = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_SPLIT", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_GC2AE_ROUNDS", "2"),
        ],
    )?;
    let run_gc2ag = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_INNER", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_GC2AE_ROUNDS", "2"),
        ],
    )?;
    let run_gc2ah = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_GEO", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_GC2AE_ROUNDS", "2"),
        ],
    )?;
    let run_gc2ai = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_QUAD", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_GC2AE_ROUNDS", "2"),
        ],
    )?;
    let run_gc2aj = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_CORNER", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_GC2AE_ROUNDS", "1"),
        ],
    )?;
    let run_gc2ak = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_LADDER", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_GC2AE_ROUNDS", "1"),
        ],
    )?;
    let run_gc2al = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_CURE", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_GC2AE_ROUNDS", "1"),
        ],
    )?;
    let run_gc2am = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_TEXEL", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_GC2AE_ROUNDS", "1"),
        ],
    )?;
    let run_gc2an = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_ORDERS", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_GC2AE_ROUNDS", "2"),
        ],
    )?;
    let run_gc2ao = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_PROOF", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_GC2AE_ROUNDS", "1"),
        ],
    )?;
    let run_gc2ap = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_BITS", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2aq = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_PUNCH", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_GC2AE_ROUNDS", "1"),
        ],
    )?;
    let run_gc2ax = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_TIMER_ALL", "1"),
            ("RP_GPUBENCH_GC2AE_ROUNDS", "1"),
        ],
    )?;
    let run_gc2aw = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_BAND", "1"),
            ("RP_GPUBENCH_GC2AE_ROUNDS", "1"),
        ],
    )?;
    let run_gc2av = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_WHERE", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2au = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_TIMER", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2at = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_664", "1"),
            ("RP_GPUBENCH_GC2_664_MIX", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2as = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_664", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2ar = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_REGS", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2bz = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_TEAR", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2ca = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2cb = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_GC2_HALF_SCALER", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2cc = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_GC2_HALF_CARD", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2cd = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_GC2_HALF_RGB", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2ce = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_GC2_HALF_COLOUR", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2cf = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_GC2_HALF_FIXED", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2cg = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_GC2_HALF_CSC", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2ch = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_GC2_HALF_ROWS", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2ci = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_GC2_HALF_STRIDE", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2cj = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_GC2_HALF_DOT3", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2ck = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_GC2_HALF_COMBINE", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2cl = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_GC2_HALF_DOT3B", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2cm = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_GC2_HALF_CONVERT", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2cn = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_GC2_HALF_FAST", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2co = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_GC2_HALF_ONE", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2ct = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_GC2_HALF_LINES", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2db = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_GC2_HALF_R101", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2dc = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_GC2_HALF_R101_SWITCH", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2dc3 = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_GC2_HALF_R101_SWITCH", "1"),
            ("RP_GPUBENCH_GC2_HALF_R101_WAYS", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2dc4 = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_GC2_HALF_R101_SWITCH", "1"),
            ("RP_GPUBENCH_GC2_HALF_R101_COST", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2dd = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_GC2_HALF_R108", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2de = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_GC2_HALF_R20", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2df = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_GC2_HALF_R20P", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_r116g = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_R116G", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_r117s = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_R117S", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_r121a = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_R121A", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_r121b = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_R121B", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_r121c = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_R121C", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_r121s = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_R121S", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_r117t = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_R117T", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_r118c = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_R118C", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_r118b = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_R118B", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_r118 = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_R118", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_r117 = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_R117", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_r23e = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_R23E", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_r23t = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_R23T", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_r114 = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_R114", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_r113b = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_R113B", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_r113 = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_R113", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2cs = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_GC2_HALF_LOOK", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2cp = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_GC2_HALF_ROUTES", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2cp_low = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_GC2_HALF_ROUTES", "1"),
            ("RP_GPUBENCH_GC2_HALF_ROUTES_LOW", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2cp_r102 = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_GC2_HALF_ROUTES", "1"),
            ("RP_GPUBENCH_GC2CP_R102", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2cp_every = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_GC2_HALF_ROUTES", "1"),
            ("RP_GPUBENCH_GC2_HALF_ROUTES_LOW", "1"),
            ("RP_GPUBENCH_GC2CP_EVERY", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2i_div2 = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2I_DIV2", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "1"),
            ("RP_GPUBENCH_SOAK_SECS", "10"),
        ],
    )?;
    let run_gc2i_div2_low = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2I_DIV2_LOW", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "1"),
            ("RP_GPUBENCH_SOAK_SECS", "10"),
        ],
    )?;
    let run_gc2i_div2_top = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2I_DIV2_TOP", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "1"),
            ("RP_GPUBENCH_SOAK_SECS", "10"),
        ],
    )?;
    let run_gc2cu = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_GC2_HALF_QUADS", "1"),
            ("RP_GPUBENCH_GC2_QUADS_KINDS", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2cv = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_GC2_HALF_QUADFIT", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2cw = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_GC2_HALF_OFFSETS", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2cx = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_GC2_HALF_CONVERT", "1"),
            ("RP_GPUBENCH_GC2_HALF_EXACT", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2cy = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HALF", "1"),
            ("RP_GPUBENCH_GC2_HALF_R80", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2cz = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_TEAR", "1"),
            ("RP_GPUBENCH_GC2_TEAR_SCHED", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2by = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_GAMES", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_SOAK_SECS", "20"),
        ],
    )?;
    let run_gc2bx = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_FIXES", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_SOAK_SECS", "20"),
        ],
    )?;
    let run_gc2cr = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_SCALED", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_SOAK_SECS", "20"),
        ],
    )?;
    let run_gc2cq = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_PVR2", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_SOAK_SECS", "20"),
        ],
    )?;
    let run_gc2bw = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_MESH", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_SOAK_SECS", "20"),
        ],
    )?;
    let run_gc2bv = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_SEAL", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_SOAK_SECS", "20"),
        ],
    )?;
    let run_gc2bu = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_DIAGONAL", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_SOAK_SECS", "20"),
        ],
    )?;
    let run_gc2bt = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_REGION", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_SOAK_SECS", "20"),
        ],
    )?;
    let run_gc2bs = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_PORT", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_SOAK_SECS", "20"),
        ],
    )?;
    let run_gc2br = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_SHADE_HOT", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_SOAK_SECS", "20"),
        ],
    )?;
    let run_gc2bq = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_ORDER_HOT", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_SOAK_SECS", "20"),
        ],
    )?;
    let run_gc2bp = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_PROVE3", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_SOAK_SECS", "20"),
        ],
    )?;
    let run_gc2bo = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_PROVE2", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_SOAK_SECS", "20"),
        ],
    )?;
    let run_gc2bn = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_NUDGE_SIZE", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_SOAK_SECS", "20"),
        ],
    )?;
    let run_gc2bm = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_NUDGE", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_SOAK_SECS", "20"),
        ],
    )?;
    let run_gc2bl = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_MAP", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_SOAK_SECS", "20"),
        ],
    )?;
    let run_gc2bk = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_UPSAMPLE", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_SOAK_SECS", "20"),
        ],
    )?;
    let run_gc2bj = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_SPLIT10", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_SOAK_SECS", "20"),
        ],
    )?;
    let run_gc2bi = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_MAGNIFY", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_SOAK_SECS", "20"),
        ],
    )?;
    let run_gc2bh = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_TEXEL_LAST", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_SOAK_SECS", "20"),
        ],
    )?;
    let run_gc2bg = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_PROVE", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_SOAK_SECS", "20"),
        ],
    )?;
    let run_gc2bf = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_COMBO", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_SOAK_SECS", "20"),
        ],
    )?;
    let run_gc2be = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_VIDEO", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_SOAK_SECS", "20"),
        ],
    )?;
    let run_gc2bd = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_STRIP", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_SOAK_SECS", "20"),
        ],
    )?;
    let run_gc2bc = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_SCHEDULE", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_SOAK_SECS", "20"),
        ],
    )?;
    let run_gc2bb = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_ONE", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_SOAK_SECS", "20"),
        ],
    )?;
    let run_gc2ba = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_CAUSE", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_SOAK_SECS", "20"),
        ],
    )?;
    let run_gc2az = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_HEAT", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_SOAK_SECS", "20"),
        ],
    )?;
    let run_gc2ay = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_BLOCK", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_ft2 = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_TOGETHER2", "1"))?;
    let run_ft3 = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_FT3", "1"))?;
    let run_ft2f = variant_image_with(
        root,
        "rp-gpubench",
        ("RP_GPUBENCH_TOGETHER2", "1"),
        Some(("RP_GPUBENCH_FT2_FAST", "1")),
    )?;
    // Run H without its watchdog (the emulator's check needs it back).
    let run_h = variant_image_with(
        root,
        "rp-gpubench",
        ("RP_GPUBENCH_FREEZE", "1"),
        Some(("RP_GPUBENCH_NO_WATCHDOG", "1")),
    )?;
    let modelled = device_has(root, "intel-2700g", "mbx-3d");
    // What the X51v showed that the fork models from rp-v0.29.0
    // (rp-gpubench, 2026-10-19): rectangular textures, a plane start
    // taking effect mid-frame, the video plane's RGB formats.
    let rect = device_has(root, "intel-2700g", "mbx-3d-rect");
    let repoint = device_has(root, "intel-2700g", "plane-repoint");
    let video_rgb = device_has(root, "intel-2700g", "video-rgb");
    let shot = scratch(root, "gpubench-screen.ppm")?;
    let guest = match emulator(root)? {
        Emulator::Wsl => to_wsl_path(&shot),
        Emulator::Native(_) => shot.display().to_string(),
    };
    // (name, image, wanted lines, whether they need the 3D model, key
    // presses: guest ms before the first, between them, how many). Runs C
    // and D draw no 3D; their presses end the watch steps early (a press
    // landing outside one is ignored by the image, which then waits its
    // cap), never more than the steps, so none reaches the loader.
    let runs: [GpubenchRun; 177] = [
        (
            "run A",
            &run_a,
            &[
                "G4a: rectangular textures",
                "G1 565 bilinear 1:1 / 16-bit",
                "G2 translucent flat / 32-bit",
                "G3 1 opaque 565 layer, one tile",
                "U4b SDRAM to the texture by DMA",
                "U3 and U4 wrote the same texture: true",
                "U5b strip",
                "  W1: ",
                "  W2: ",
                "  W3: ",
            ],
            true,
            (0, 0, 0),
        ),
        (
            "run B",
            &run_b,
            &["B: TEX0 words", "  B: 20 tries"],
            true,
            (0, 0, 0),
        ),
        (
            "run C",
            &run_c,
            &[
                "  W4: ",
                "the list converted RGB -> UYVY",
                "  W5: ",
                "the RGB list drawn",
                "  W6: ",
            ],
            false,
            (15_000, 8_000, 3),
        ),
        (
            "run D",
            &run_d,
            &["  try code 0", "  try code 8"],
            false,
            (6_000, 3_000, 9),
        ),
        (
            "run E",
            &run_e,
            &[
                "E1a agrees with the reference order: true",
                "E1c the texture in 2700G memory is right: true",
                "E1d fast reorder written by the CPU straight in",
                "E1e the texture in 2700G memory is right: true",
                "E3 PVRTC 2 bpp layer",
                "E4 a screen of text",
            ],
            false,
            (0, 0, 0),
        ),
        (
            "run F",
            &run_f,
            &[
                "  F0 RGB565 pixels",
                "  F1 4 bpp strips",
                "  F3a one 2D command a line",
                "  F3b one 2D command for all",
                "  F4 fill + a plain 2D copy a glyph",
                "  F5 fill",
                "  F6a full commands, batched",
                "  F6b surfaces once",
                "  F6c surfaces once",
                "  F7 the word cache made once: 259 words",
                "  F7 the line cache made once: 37 lines",
                "  F7a a copy a word (cached words): 259 operations",
                "  F7b a copy a line (cached lines): 37 operations",
                "  F7c one copy (the cached screen): 1 operations",
            ],
            false,
            (0, 0, 0),
        ),
        (
            "run G",
            &run_g,
            &["G5: the clocks", "RETROPANE-GPUBENCH: done"],
            false,
            (0, 0, 0),
        ),
        // Run GS (the clocks' soak): checked below.
        (
            "run GS",
            &run_gs,
            &["GS: the clocks' warm soak", "RETROPANE-GPUBENCH: done"],
            false,
            (0, 0, 0),
        ),
        // Run FT2 (2D and 3D at once by scan-out and placement): every
        // setting ran and finished; the model has no interference, so every
        // setting is clean.
        (
            "run FT2",
            &run_ft2,
            &[
                "  FT2 16-bit screen, copies apart: ",
                "  FT2 scan-out stopped, copies beside: ",
                "  FT2 32-bit screen, copies beside: ",
                "FT2: 6 of 6 settings clean",
                "RETROPANE-GPUBENCH: done",
            ],
            true,
            (0, 0, 0),
        ),
        // Run FT2 at 133.25 MHz: checked below (a release that doesn't
        // model Windows Mobile's clocks refuses).
        (
            "run FT2F",
            &run_ft2f,
            &[
                "FT2: the 2D engine and the 3D core at once",
                "RETROPANE-GPUBENCH: done",
            ],
            true,
            (0, 0, 0),
        ),
        // Run GC (the content map): checked below.
        (
            "run GC",
            &run_gc,
            &[
                "GC: which clock settings draw every content right",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2 (the content map by the owner's robustness rules, one
        // render a kind a setting): checked below.
        (
            "run GC2",
            &run_gc2,
            &[
                "GC2: run GC by the owner's robustness rules",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2S: GC2 with its label at offset 0 (C71, G17-G19) and
        // divide 1 to 133.25: runs to its end and returns (the pinned
        // release refuses the clocks as for GC2).
        (
            "run GC2S",
            &run_gc2s,
            &[
                "GC2: run GC by the owner's robustness rules",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2I: GC2S with the 16-bit kinds' targets inside the safe
        // region, the 32-bit ones outside, then the soak: runs to its end
        // and returns (the pinned release doesn't model Windows Mobile's
        // clock, so GC2 stops at its first check, as for GC2S; the whole
        // run is stepped on the wm-clocks build: docs/hw/2700g.md).
        (
            "run GC2I",
            &run_gc2i,
            &[
                "GC2: run GC with every 16-bit kind's target inside the safe region, the 32-bit kinds outside (GC2I), then a soak,",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2J: GC2I's references, then its soak (10 s a setting
        // here): runs to its end and returns (the pinned release doesn't
        // model Windows Mobile's clock; stepped on the wm-clocks build).
        (
            "run GC2J",
            &run_gc2j,
            &[
                "GC2: the 16-bit kinds' targets inside the safe region (GC2J: no map;",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2K: GC2J's patch isolated (20 s at 133.25 / 1 here): runs
        // to its end and returns (the pinned release doesn't model Windows
        // Mobile's clock; stepped on the wm-clocks build).
        (
            "run GC2K",
            &run_gc2k,
            &[
                "GC2: the 16-bit kinds' targets inside the safe region (GC2K: no map;",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2L: GC2K's patch mapped (30 s at 133.25 / 1 here): runs to
        // its end and returns (the pinned release doesn't model Windows
        // Mobile's clock; stepped on the wm-clocks build).
        (
            "run GC2L",
            &run_gc2l,
            &[
                "GC2: the 16-bit kinds' targets inside the safe region (GC2L: no map;",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2O: the tile orders (75 s at 133.25 / 1 here): runs to its
        // end and returns (the pinned release doesn't model Windows
        // Mobile's clock; stepped on the wm-clocks build).
        (
            "run GC2O",
            &run_gc2o,
            &[
                "GC2: the 16-bit kinds' targets inside the safe region (GC2O: no map;",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2P: GC2O made fast (3 rounds here), then FT3 in each tile
        // order: runs to its end and returns (the pinned release doesn't
        // model Windows Mobile's clock; stepped on the wm-clocks build).
        (
            "run GC2P",
            &run_gc2p,
            &[
                "GC2: the 16-bit kinds' targets inside the safe region (GC2P: no map;",
                "GC2P part 2: done",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2Q: FT3's copies queued, after or none; the write-out
        // search; divide 1 above 133.25: runs to its end and returns (the
        // pinned release doesn't model Windows Mobile's clock; stepped on
        // the wm-clocks build).
        (
            "run GC2Q",
            &run_gc2q,
            &["GC2Q parts A and B: done", "RETROPANE-GPUBENCH: done"],
            false,
            (0, 0, 0),
        ),
        // Run GC2AX (R69): GC2AQ's plans with the timer at 0 and 15 (one
        // round each here; the emulator draws exactly).
        (
            "run GC2AX",
            &run_gc2ax,
            &[
                "GC2AX (R69): 0x664 at 0 and at 15 at 149.50 / 1, reversed; each the untextured fault generator (1 rounds) and the flat and textured corner sweeps (1 rounds a scene)",
                "GC2AX timer 0 generator",
                "GC2AX timer 15 tex",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2AW (R69): GC2AM's phase sweep with the timer at 0 and 15
        // (one round a phase here; the emulator draws exactly).
        (
            "run GC2AW",
            &run_gc2aw,
            &[
                "GC2AW (R69): GC2AM's phase sweep (31 phases of 3 / 65,536 of the texture, 1 rounds) at 149.50 / 1 with 0x664 at 0, then at 15",
                "GC2AW timer 0",
                "GC2AW timer 15",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2AV (R69): every wrong word placed (the emulator draws
        // exactly, so none).
        (
            "run GC2AV",
            &run_gc2av,
            &[
                "GC2AV (R69): GC2AM's phase scene 0 at 149.50 / 1, reversed, opaque; every wrong word placed",
                "GC2AV reference:",
                "GC2AV timer 15: words wrong by render [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]; by row in the tile [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]",
                "GC2AV timer 0: tiles hit, most first: (and 0 more after these 0)",
                "GC2AV: done; the control at Windows Mobile's clock after: Some(0) words off",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2AU (R69): 0x664's timer against a clean control, its
        // higher bits and every tile order (the emulator draws exactly).
        (
            "run GC2AU",
            &run_gc2au,
            &[
                "GC2AU (R69): GC2AM's phase scene 0 at 149.50 / 1, reversed, opaque; 0x664 = 15 against a clean control",
                "GC2AU reference:",
                "GC2AU 1 0x664 = 15: 20 of 20 right, words wrong total 0,",
                "GC2AU 1 none written: 20 of 20 right, words wrong total 0,",
                "GC2AU 2 0x664 = 0x8000000f: 8 of 8 right, words wrong total 0,",
                "GC2AU 3 interleaved: timer 0 20 of 20 right, total 0,",
                "GC2AU: done; the control at Windows Mobile's clock after: Some(0) words off",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2AT (R69): GC2AS mixed (values bit-reversed, a render with
        // no word written beside each pair; the emulator draws exactly).
        (
            "run GC2AT",
            &run_gc2at,
            &[
                "GC2AT (R69): GC2AM's phase scene 0 at 149.50 / 1, reversed, opaque; 0x664 bits 0-3 at each of 16 values, 20 renders each",
                "GC2AT reference:",
                "GC2AT step 0 0x664 = 0: 20 of 20 right, total 0,",
                "GC2AT step 15 0x664 = 15: 20 of 20 right, total 0,",
                "none written: 20 right, total 0,",
                "GC2AT: renders right by value [20, 20, 20, 20, 20, 20, 20, 20, 20, 20, 20, 20, 20, 20, 20, 20] of 20 each; the control at Windows Mobile's clock after: Some(0) words off",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2AS (R69): 0x664's low field swept (the emulator draws
        // exactly, so every render of every value is right).
        (
            "run GC2AS",
            &run_gc2as,
            &[
                "GC2AS (R69): GC2AM's phase scene 0 at 149.50 / 1, reversed, opaque; 0x664 bits 0-3 at each of 16 values, 20 renders each",
                "GC2AS reference:",
                "GC2AS 0x664 = 0: 20 of 20 renders right",
                "GC2AS 0x664 = 15: 20 of 20 renders right, words wrong total 0 (0x6FC = 15: 20 of 20 right, total 0)",
                "GC2AS: renders right by value [20, 20, 20, 20, 20, 20, 20, 20, 20, 20, 20, 20, 20, 20, 20, 20] of 20 each; the control at Windows Mobile's clock after: Some(0) words off",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2BY (R76): the game ladder, shortened (the emulator draws
        // exactly: every step clean).
        // Run GC2BZ (R20): each tile row against the scan, one render a
        // setting (the fork's scan counts from the virtual clock; its tile
        // timing isn't the device's, so only the run's shape is checked).
        (
            "run GC2BZ",
            &run_gc2bz,
            &[
                "GC2BZ (R20): each tile row against the scan line",
                "GC2BZ frame 3 reference:",
                "GC2BZ WM 99.67 / 2 rows frame 0 from line 0: ",
                "GC2BZ WM 99.67 / 2 checker frame 3 from line 600: ",
                "GC2BZ 74.75 / 1 rows frame 0 from line 0: ",
                "GC2BZ 74.75 / 1 checker frame 3 from line 600: ",
                "GC2BZ done: ",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2CA (R78 item 2): the game frames at three sizes shown
        // full screen five ways, two renders a setting (the fork doesn't
        // model the scaler or the doubling: the registers and pictures
        // only; its times aren't the device's).
        (
            "run GC2CA",
            &run_gc2ca,
            &[
                "GC2CA (R78 item 2): 4 game frames at [(480, 640), (360, 480), (240, 320)]",
                "GC2CA 240 x 320 frame 3 reference: Some(",
                "GC2CA WM 99.67 / 2 480 x 640 native frame 0: 2 renders, 0 pictures wrong, 0 not finished",
                "GC2CA WM 99.67 / 2 240 x 320 video plane 2x, a pair frame 2: 2 renders, 0 pictures wrong, 0 not finished",
                "GC2CA 74.75 / 1 240 x 320 Dell's doubling, a pair frame 3: 2 renders, 0 pictures wrong, 0 not finished",
                "GC2CA 74.75 / 1 360 x 480 video plane 4/3 frame 1: 2 renders, 0 pictures wrong, 0 not finished",
                // The depth frame at full size: wrong in both renders when
                // three renderers took the stack past 80% (2026-10-28).
                "GC2CA 74.75 / 1 480 x 640 native frame 3: 2 renders, 0 pictures wrong, 0 not finished",
                "GC2CA 74.75 / 1 speed-up 240 x 320 video plane 2x, a pair against native",
                "GC2CA watch frame 1: shown full size, Dell's 2x, the video plane's 2x, each named first",
                "GC2CA done",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2CB: the video plane's scaler on RGB565, seven ways, judged
        // by DSIG (the fork reads it 0: every way "flat" there; the shape
        // only).
        (
            "run GC2CB",
            &run_gc2cb,
            &[
                "GC2CB 1 OURS: RGB 1:1: 240x320 to 240x320",
                "    GC2CB 2 VSCADR 0x",
                "GC2CB 5 P9C WAY: RGB 2X: 240x320 to 480x640",
                "GC2CB 6 P9C WAY: UYVY 2X: 240x320 to 480x640",
                "GC2CA done",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2CC: the test card through the video plane's 2x, held for
        // the owner's camera (the emulator: no hold; the shape only).
        (
            "run GC2CC",
            &run_gc2cc,
            &[
                "GC2CC 1 RGB CARD 1:1: held 0 ms",
                "GC2CC 4 UYVY CARD OUR 2X: held 0 ms",
                "GC2CC 5 RGB CARD DELL 2X: held 0 ms",
                "GC2CA done",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2CD: RGB through the video plane's scaler seven ways, each
        // held for the owner's camera (the emulator: no hold; the shape).
        (
            "run GC2CD",
            &run_gc2cd,
            &[
                "GC2CD 1 RGB 2X PITCH X2: held 0 ms",
                "GC2CD 6 ARGB8888 2X: VSCTRL 0x",
                "GC2CD 7 RGB 2X X2 NEAREST: held 0 ms",
                "GC2CA done",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2CE: RGB's colours through the video plane's scaler, one
        // setting a picture, held for the owner's camera.
        (
            "run GC2CE",
            &run_gc2ce,
            &[
                "GC2CE 1 RGB SH X2: held 0 ms",
                "GC2CE 2 RGB CONV: VSCTRL 0x",
                "GC2CE 7 UYVY CONTROL: held 0 ms",
                "GC2CA done",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2CF: the driver's fixed RGB 2x on the card, then the game
        // frames full size, Dell's 2x and ours, each held for the camera.
        (
            "run GC2CF",
            &run_gc2cf,
            &[
                "GC2CF 1 RGB 2X FIXED: held 0 ms",
                "GC2CF 4 HALF SIZE: OURS 2X: held 0 ms",
                "GC2CF 7 HALF SIZE: OURS 2X: held 0 ms",
                "GC2CA done",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2CG: the 2D engine's 32 format codes, RGB565 to each and
        // UYVY from each, read back for a colour conversion (no photos).
        (
            "run GC2CG",
            &run_gc2cg,
            &[
                "GC2CG to code 10: 8 of 16 words written; f800f800 07e007e0",
                "GC2CG from code 31:",
                "GC2CG done:",
                "GC2CA done",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2CH: a self-addressing texture card read with TEX0 bit 0
        // beside every other texture and render-block bit, each read-back
        // fitted to rows or twiddled order (no photos). The emulator reads
        // every texture twiddled: bit 0 is unmodelled.
        (
            "run GC2CH",
            &run_gc2ch,
            &[
                "GC2CH 16-bit control (TEX0 bit 0 off), size code 2: twiddled; rows 4/1024",
                "GC2CH 32-bit, TEX0 bit 0, size code 6: twiddled;",
                "GC2CH TEX0 bit 0, across size code 3: twiddled;",
                "GC2CH TEX0 bit 0, TEX1 bit 3: twiddled; rows 4/1024 (start 512,",
                "GC2CH TEX0 bit 0, TEX1 bit 30: HUNG (the core was reset)",
                "GC2CH render block: 0 tries read other than TEX0 bit 0 alone",
                "GC2CH done: 1786 tries, 0 read as rows (strides in bytes []), 1784 twiddled, 1 other, 0 not rendered, 1 hung;",
                "GC2CA done",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2CI: GC2CH's row-order mode with TEX1 bit 29 at every size,
        // level and LOD, each drawn 1:1 (no photos). The emulator reads
        // every texture twiddled: bit 0 is unmodelled (emulator.md).
        (
            "run GC2CI",
            &run_gc2ci,
            &[
                "GC2CI TEX0 bit 0 alone (control), size code 2: twiddled;",
                "GC2CI bit 29, 16-bit, 256 x 512 5: twiddled;",
                "GC2CI 256 x 512, TEX1 bits 30-29 = 2: HUNG (the core was reset)",
                "GC2CI done: 57 tries, 0 read as rows (strides in bytes []), 56 twiddled, 0 other, 0 not rendered, 1 hung;",
                "GC2CA done",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2CJ: DOT3's formula on read-back ramps under eight tints,
        // then a 120 x 320 conversion-sized pass timed (no photos). The
        // emulator draws DOT3 as it draws any unknown combine.
        (
            "run GC2CJ",
            &run_gc2cj,
            &[
                "GC2CJ TEX2 0xd6886610 tint 0xffffffff red  : ff048080 ff248080 ff448080",
                "GC2CJ TEX2 0x86096612 tint 0xffff0000 blue:",
                "GC2CJ DOT3, rows (TEX0 bit 0): fastest",
                "GC2CJ done: 0 hung;",
                "GC2CA done",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2CK: each TEX2 bit around three combine words, outputs
        // checked for a channel following another channel's ramp (no
        // photos). The emulator models MODULATE only: nothing crosses.
        (
            "run GC2CK",
            &run_gc2ck,
            &[
                "GC2CK 0x86096613: red r... green .g.. blue ..b.; ff012020-ff392020",
                "GC2CK done: 99 tries, 0 cross-channel, 0 hung;",
                "GC2CA done",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2CL: Dell's DOT3 words with TEX0 bit 13 set as Dell's
        // combine builder sets it, ramps read back under eight tints (no
        // photos). The emulator's odd format codes are its own guess.
        (
            "run GC2CL",
            &run_gc2cl,
            &[
                "GC2CL 0xc6046608 TEX0 ^ 0x0, tint 0xffffffff: red r... green .g.. blue ..b.;",
                "GC2CL done: 27 tries, 0 cross-channel, 0 hung;",
                "GC2CA done",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2CM: our RGB-to-UYVY conversion whole (three 3D passes, two
        // 2D placements), timed and every word checked (no photos). The
        // emulator draws DOT3 as MODULATE, so its Y check counts errors.
        (
            "run GC2CM",
            &run_gc2cm,
            &[
                "GC2CM RGB565 card: fastest A",
                "GC2CM RGB565 card word (60, 160): 0x",
                "GC2CM ARGB8888 card check: U' and V' wrong in",
                "GC2CM done:",
                "GC2CA done",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2CN: GC2CM's conversion in eight ways (two clocks, Y' as
        // two renders or one tall one, serial or 2D overlapped), each timed
        // and checked (no photos). The emulator's words are wrong as in
        // GC2CM (no row order, no DOT3); its times are its own.
        (
            "run GC2CN",
            &run_gc2cn,
            &[
                "GC2CN WM 99.67 / 2, Y' two renders, serial:",
                "GC2CN 74.75 / 1, Y' one tall render, overlapped:",
                "GC2CM buffer 1 check:",
                "GC2CN done:",
                "GC2CA done",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2CO: the conversion's three quads in one render (each with
        // its own combine words) against three renders, serial or with the
        // 2D placements overlapped, timed and checked (no photos). The
        // emulator's words are wrong as in GC2CM; its times are its own.
        (
            "run GC2CO",
            &run_gc2co,
            &[
                "GC2CO three renders, serial:",
                "GC2CO one render, overlapped:",
                "GC2CM set 1 check:",
                "GC2CO done:",
                "GC2CA done",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2CT: 480 x 320 with the graphics plane's line doubling
        // alone, and as 480-wide UYVY through the video plane's vertical
        // scale, four labelled photos (the run's shape).
        (
            "run GC2CT",
            &run_gc2ct,
            &[
                "GC2CT 1 GDRCTRL 0x2",
                "GC2CT 1 TEXTURED FRAME LINE DOUBLING ONLY: rendered Some(",
                "GC2CT 4 FLAT FRAME SMOOTH DOWN AS UYVY: rendered Some(",
                "GC2CA done",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2DB (R101): DSIG's coverage on the three routes. The fork
        // doesn't model DSIG (reads 0: docs/hw/emulator.md), so this checks
        // the run's shape and that every probe's word was written and put
        // back where the route shows it; the coverage is the device's.
        (
            "run GC2DB",
            &run_gc2db,
            &[
                "GC2DB (R101): DSIG's coverage",
                "GC2DB route 1 NATIVE 480 X 640 positions: 0 of 60 changed DSIG, 0 held a frame, 60 back to 0x00000000 when put back; words written and put back 60 of 60",
                "GC2DB route 2 480 X 320 LINES 2X positions: 0 of 60 changed DSIG, 0 held a frame, 60 back to 0x00000000 when put back; words written and put back 60 of 60",
                "GC2DB route 3 240 X 320 SMOOTH 2X positions: 0 of 60 changed DSIG, 0 held a frame, 60 back to 0x00000000 when put back; words written and put back 60 of 60",
                "GC2DB route 3 240 X 320 SMOOTH 2X bits: 0 of 24 changed DSIG, 0 held, 24 back; words written and put back 24 of 24",
                "GC2DB done: 236 probes, words written and put back 236,",
                "GC2CA done",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2DC (R101): route switches checked by DSIG every frame. The
        // fork reads DSIG 0, so the frames are read, not judged: this
        // checks the run's shape, that the scan line paces every read (none
        // missed) and that the pictures fit the pool.
        (
            "run GC2DC",
            &run_gc2dc,
            &[
                "GC2DC (R101): route switches checked by DSIG every frame",
                "GC2DC return: native fresh ",
                "GC2DC: the frame ",
                "GC2DC part 1: ",
                "not read 0, not judged ",
                "GC2DC part 2 1 a switch every 1 frames: 12 frames read: 0 the route in place as the frame began, 0 another route whole, 0 NEITHER ROUTE (first None), 0 not read (not judged: no DSIG)",
                "GC2DC part 2 2 a switch every 8 frames: 12 frames read: 0 the route in place as the frame began, 0 another route whole, 0 NEITHER ROUTE (first None), 0 not read (not judged: no DSIG)",
                "GC2CA done",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // GC2DC run 3 (R101): three ways into and out of the smooth route,
        // each switched on line 0 and every frame; the fork reads DSIG 0,
        // so the run's shape and every frame read on time.
        (
            "run GC2DC ways",
            &run_gc2dc3,
            &[
                "GC2DC return: native fresh ",
                "GC2DC way 1 A TODAY: CLOCK OFF, ALL REWRITTEN: pairs on line 0: 0 frames of neither route",
                "GC2DC way 2 B DELL: STREAM BIT ONLY: pairs on line 0: 0 frames of neither route",
                "GC2DC way 3 C OURS: VIDEO ALWAYS ON, UNDER: pairs on line 0: 0 frames of neither route, the slowest settled in frame 0; a switch every frame: 12 frames: 0 the new route whole, 0 another route whole, 0 NEITHER ROUTE, 0 not read (not judged",
                "GC2CA done",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // GC2DC run 4 (R101): way C's cost (renders with the video plane
        // off, reading the shown picture, reading outside the safe region)
        // and way C' switched every 1 and 8 frames; the fork reads DSIG 0.
        (
            "run GC2DC cost",
            &run_gc2dc4,
            &[
                "GC2DC cost 1 video off: 2 of 2 rendered",
                "GC2DC cost 2 video on, reading the shown picture (C'): 2 of 2 rendered",
                "GC2DC cost 3 video on, reading outside the safe region (C): 2 of 2 rendered",
                "GC2DC long 1 way C' a switch every 1 frames: 12 frames: 0 the route in place whole, 0 another route whole, 0 NEITHER ROUTE (first None), 0 not read (not judged",
                "GC2DC long 2 way C' a switch every 8 frames: 12 frames:",
                "GC2CA done",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2DD (R108 layout B): way C' at 149.50 / 2, the half
        // routes switched every 1 and 8 frames with the next frame
        // rendering across each switch, then native's render cost with the
        // video plane off and reading the shown picture. The fork reads
        // DSIG 0 (not judged) and has no clock: the run's shape and the
        // renders are what this checks.
        (
            "run GC2DD",
            &run_gc2dd,
            &[
                "GC2DD (R108 layout B): way C' at 149.50 / 2",
                "GC2DD 149.50 / 2: lock Some(",
                "), the display's underrun flag false",
                "GC2DD at 149.50 / 2: the pictures' hashes Some(",
                "GC2DD long 1 way C' at 149.50 / 2 a switch every 1 frames, rendering across each: 12 passes over 0 display frames (a line ",
                "0 judged by DSIG, 0 not read; 0 the route in place whole, 0 the other route whole, 0 NEITHER ROUTE (first None), 0 lines missed (not judged",
                "GC2DD long 2 way C' at 149.50 / 2 a switch every 8 frames, rendering across each: 12 passes over 0 display frames",
                "renders 13 done, 0 failed,",
                "0 lines missed (not judged: no DSIG, or the signatures not steady and different); 11 switches, ",
                "the first frame after 0 of them judged;",
                "11 new renderers set up after a switch",
                "the pictures afterwards the references' (exact)",
                "GC2DD cost 1 video off: 2 of 2 rendered",
                "GC2DD cost 2 video on, reading the shown picture (C'): 2 of 2 rendered",
                "GC2DD done: Windows Mobile's clock put back",
                "GC2CA done",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2DF (R20's proof run, the owner's go 2026-10-29): the copy
        // fallback against the blanking start and option 1, one-off and
        // back to back, and audit C93's copy start lines and landing order.
        // The fork has no DSIG (every refresh "not judged") and no timing
        // of its own: the run's shape, every picture and every copy right,
        // nothing failed or hung, is what this checks.
        (
            "run GC2DF",
            &run_gc2df,
            &[
                "GC2DF (R20's proof run, the owner's go 2026-10-29): the copy fallback against the blanking start (rows, checkerboard) and option 1 at 149.50 / 2",
                "GC2DF 149.50 / 2: lock Some(",
                "GC2DF at 149.50 / 2: 8 of 8 pictures the references'",
                "NOT JUDGED (not steady, zero or alike)",
                "GC2DF the whole-screen copy ",
                ": its window Some(",
                "GC2DF C93 copy begun at line 400 (outside): 1 copies, begun at lines ",
                "GC2DF C93 copy begun at line 480 (outside): 1 copies, begun at lines ",
                "GC2DF C93 copy begun at line 520 (outside): 1 copies, begun at lines ",
                "(inside the window): 1 copies, begun at lines ",
                "; 0 wrong, failed or missed; refreshes not judged",
                "GC2DF C93 landing order, 1 whole-screen copies: 0 tile rows landed before the row above",
                "GC2DF one-off copy behind the scan (the ruling): 4 frames; 0 failed, 0 hung, 0 pictures wrong; render done ",
                "GC2DF one-off blanking start, rows: 4 frames; 0 failed, 0 hung, 0 pictures wrong; render done ",
                "GC2DF one-off blanking start, checkerboard: 4 frames; 0 failed, 0 hung, 0 pictures wrong; render done ",
                "GC2DF one-off option 1 (flip, copy back, flip back): 4 frames; 0 failed, 0 hung, 0 pictures wrong; render done ",
                "GC2DF back to back copy behind the scan (the ruling): 4 frames; 0 failed, 0 hung, 0 pictures wrong; render done ",
                "GC2DF back to back blanking start, rows: 4 frames; 0 failed, 0 hung, 0 pictures wrong; render done ",
                "GC2DF back to back blanking start, checkerboard: 4 frames; 0 failed, 0 hung, 0 pictures wrong; render done ",
                "GC2DF back to back option 1 (flip, copy back, flip back): 4 frames; 0 failed, 0 hung, 0 pictures wrong; render done ",
                "GC2DF done: Windows Mobile's clock put back; 0 hung in all",
                "GC2CA done",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run R116G (lists on die on the game frames): the shape, every
        // render right (the fork has no on-die timing).
        (
            "run R116G",
            &run_r116g,
            &[
                "R116G (lists on die on the game frames, the owner's go 2026-10-29)",
                "R116G lists in SDRAM, frame 0, target 0x000000: the core ",
                "R116G lists in SDRAM, frame 0, target 0x296000: the core ",
                "R116G lists in SDRAM, frame 1, target 0x000000: the core ",
                "R116G lists in SDRAM, frame 1, target 0x296000: the core ",
                "R116G lists in SDRAM, frame 2, target 0x000000: the core ",
                "R116G lists in SDRAM, frame 2, target 0x296000: the core ",
                "R116G lists in SDRAM, frame 3, target 0x000000: the core ",
                "R116G lists in SDRAM, frame 3, target 0x296000: the core ",
                "R116G lists on die, frame 0, target 0x000000: the core ",
                "R116G lists on die, frame 0, target 0x296000: the core ",
                "R116G lists on die, frame 1, target 0x000000: the core ",
                "R116G lists on die, frame 1, target 0x296000: the core ",
                "R116G lists on die, frame 2, target 0x000000: the core ",
                "R116G lists on die, frame 2, target 0x296000: the core ",
                "R116G lists on die, frame 3, target 0x000000: the core ",
                "R116G lists on die, frame 3, target 0x296000: the core ",
                "R116G done: the display's fetch on, Windows Mobile's clock put back",
                "GC2CA done",
                "everything given back: true",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run R121A (R121's test 1): the shape, every way's renders right (the
        // fork models no divide-1 fault).
        (
            "run R121A",
            &run_r121a,
            &[
                "R121A (R121, the owner 2026-10-30)",
                "R121A references at 74.75 / 1: drawn and repeatable true",
                "R121A 143.00 / 1, screen 0 shown (as in use; the control, wrong in run 3), frame 0: 0 of 2 renders wrong",
                "R121A 143.00 / 1, screen 0 shown (as in use; the control, wrong in run 3), frame 1: 0 of 2 renders wrong",
                "R121A 143.00 / 1, screen 0 shown (as in use; the control, wrong in run 3), frame 2: 0 of 2 renders wrong",
                "R121A 143.00 / 1, screen 0 shown (as in use; the control, wrong in run 3), frame 3: 0 of 2 renders wrong",
                "R121A screen 0 shown (as in use; the control, wrong in run 3): the underrun flag ",
                "R121A 143.00 / 1, a quarter of the reads (240 x 320 at 0x0, Dell's doubling), frame 0: 0 of 2 renders wrong",
                "R121A 143.00 / 1, a quarter of the reads (240 x 320 at 0x0, Dell's doubling), frame 1: 0 of 2 renders wrong",
                "R121A 143.00 / 1, a quarter of the reads (240 x 320 at 0x0, Dell's doubling), frame 2: 0 of 2 renders wrong",
                "R121A 143.00 / 1, a quarter of the reads (240 x 320 at 0x0, Dell's doubling), frame 3: 0 of 2 renders wrong",
                "R121A a quarter of the reads (240 x 320 at 0x0, Dell's doubling): the underrun flag ",
                "R121A 143.00 / 1, checker tile order, frame 0: 0 of 2 renders wrong",
                "R121A 143.00 / 1, checker tile order, frame 1: 0 of 2 renders wrong",
                "R121A 143.00 / 1, checker tile order, frame 2: 0 of 2 renders wrong",
                "R121A 143.00 / 1, checker tile order, frame 3: 0 of 2 renders wrong",
                "R121A checker tile order: the underrun flag ",
                "R121A references at 74.75 / 1 (with the overlay): drawn and repeatable true",
                "R121A 143.00 / 1, an alpha-0 quad blended over the frame (the write-out paced), frame 0: 0 of 2 renders wrong",
                "R121A 143.00 / 1, an alpha-0 quad blended over the frame (the write-out paced), frame 1: 0 of 2 renders wrong",
                "R121A 143.00 / 1, an alpha-0 quad blended over the frame (the write-out paced), frame 2: 0 of 2 renders wrong",
                "R121A 143.00 / 1, an alpha-0 quad blended over the frame (the write-out paced), frame 3: 0 of 2 renders wrong",
                "R121A an alpha-0 quad blended over the frame (the write-out paced): the underrun flag ",
                "R121A 143.00 / 1, the display's fetch off (the control, right in R118C), frame 0: 0 of 2 renders wrong",
                "R121A 143.00 / 1, the display's fetch off (the control, right in R118C), frame 1: 0 of 2 renders wrong",
                "R121A 143.00 / 1, the display's fetch off (the control, right in R118C), frame 2: 0 of 2 renders wrong",
                "R121A 143.00 / 1, the display's fetch off (the control, right in R118C), frame 3: 0 of 2 renders wrong",
                "R121A the display's fetch off (the control, right in R118C): the underrun flag ",
                "R121A done: Windows Mobile's clock put back",
                "GC2CA done",
                "everything given back: true",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run R121B (R121's test 2): the shape, every way's renders right.
        (
            "run R121B",
            &run_r121b,
            &[
                "R121B (R121, the owner 2026-10-30)",
                "R121B references at 74.75 / 1: drawn and repeatable true",
                "R121B 143.00 / 1, as in use: lists on die, the underlay, rows, frame 0: 0 of 2 renders wrong",
                "R121B 143.00 / 1, as in use: lists on die, the underlay, rows, frame 1: 0 of 2 renders wrong",
                "R121B 143.00 / 1, as in use: lists on die, the underlay, rows, frame 2: 0 of 2 renders wrong",
                "R121B 143.00 / 1, as in use: lists on die, the underlay, rows, frame 3: 0 of 2 renders wrong",
                "R121B as in use: lists on die, the underlay, rows: the underrun flag ",
                "R121B 143.00 / 1, as in use, checker order, frame 0: 0 of 2 renders wrong",
                "R121B 143.00 / 1, as in use, checker order, frame 1: 0 of 2 renders wrong",
                "R121B 143.00 / 1, as in use, checker order, frame 2: 0 of 2 renders wrong",
                "R121B 143.00 / 1, as in use, checker order, frame 3: 0 of 2 renders wrong",
                "R121B as in use, checker order: the underrun flag ",
                "R121B 143.00 / 1, as in use without the underlay, frame 0: 0 of 2 renders wrong",
                "R121B 143.00 / 1, as in use without the underlay, frame 1: 0 of 2 renders wrong",
                "R121B 143.00 / 1, as in use without the underlay, frame 2: 0 of 2 renders wrong",
                "R121B 143.00 / 1, as in use without the underlay, frame 3: 0 of 2 renders wrong",
                "R121B as in use without the underlay: the underrun flag ",
                "R121B 143.00 / 1, lists in SDRAM (run 3's), the underlay, rows, frame 0: 0 of 2 renders wrong",
                "R121B 143.00 / 1, lists in SDRAM (run 3's), the underlay, rows, frame 1: 0 of 2 renders wrong",
                "R121B 143.00 / 1, lists in SDRAM (run 3's), the underlay, rows, frame 2: 0 of 2 renders wrong",
                "R121B 143.00 / 1, lists in SDRAM (run 3's), the underlay, rows, frame 3: 0 of 2 renders wrong",
                "R121B lists in SDRAM (run 3's), the underlay, rows: the underrun flag ",
                "R121B done: Windows Mobile's clock put back",
                "GC2CA done",
                "everything given back: true",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run R121C (R121's test 3): the shape, every way's renders right.
        (
            "run R121C",
            &run_r121c,
            &[
                "R121C (R121, the owner 2026-10-30)",
                "R121C references at 74.75 / 1: drawn and repeatable true",
                "R121C 143.00 / 1, as in use (the control), frame 0: 0 of 2 renders wrong",
                "R121C 143.00 / 1, as in use (the control), frame 1: 0 of 2 renders wrong",
                "R121C 143.00 / 1, as in use (the control), frame 2: 0 of 2 renders wrong",
                "R121C 143.00 / 1, as in use (the control), frame 3: 0 of 2 renders wrong",
                "R121C as in use (the control): the underrun flag ",
                "R121C 143.00 / 1, a quarter of the reads (240 x 320 at 0x0, Dell's doubling), frame 0: 0 of 2 renders wrong",
                "R121C 143.00 / 1, a quarter of the reads (240 x 320 at 0x0, Dell's doubling), frame 1: 0 of 2 renders wrong",
                "R121C 143.00 / 1, a quarter of the reads (240 x 320 at 0x0, Dell's doubling), frame 2: 0 of 2 renders wrong",
                "R121C 143.00 / 1, a quarter of the reads (240 x 320 at 0x0, Dell's doubling), frame 3: 0 of 2 renders wrong",
                "R121C a quarter of the reads (240 x 320 at 0x0, Dell's doubling): the underrun flag ",
                "R121C 143.00 / 2, R120's switch: as in use at 143.00 / 2, frame 0: 0 of 2 renders wrong",
                "R121C 143.00 / 2, R120's switch: as in use at 143.00 / 2, frame 1: 0 of 2 renders wrong",
                "R121C 143.00 / 2, R120's switch: as in use at 143.00 / 2, frame 2: 0 of 2 renders wrong",
                "R121C 143.00 / 2, R120's switch: as in use at 143.00 / 2, frame 3: 0 of 2 renders wrong",
                "R121C R120's switch: as in use at 143.00 / 2: the underrun flag ",
                "R121C references at 74.75 / 1 (with the overlay): drawn and repeatable true",
                "R121C 143.00 / 1, an alpha-0 quad blended over the frame (the write-out paced), frame 0: 0 of 2 renders wrong",
                "R121C 143.00 / 1, an alpha-0 quad blended over the frame (the write-out paced), frame 1: 0 of 2 renders wrong",
                "R121C 143.00 / 1, an alpha-0 quad blended over the frame (the write-out paced), frame 2: 0 of 2 renders wrong",
                "R121C 143.00 / 1, an alpha-0 quad blended over the frame (the write-out paced), frame 3: 0 of 2 renders wrong",
                "R121C an alpha-0 quad blended over the frame (the write-out paced): the underrun flag ",
                "R121C done: Windows Mobile's clock put back",
                "GC2CA done",
                "everything given back: true",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run R121S (the overlay's soak): the shape, two renders right.
        (
            "run R121S",
            &run_r121s,
            &[
                "R121S (R121C's overlay proven, the owner's go 2026-10-30)",
                "R121S references at 74.75 / 1 (with the overlay): drawn and repeatable true",
                "R121S 1 renders",
                "R121S 2 renders",
                "R121S end: 2 renders in ",
                "wrong 0 of 2; not finished 0; underruns 0",
                "R121S done: Windows Mobile's clock put back",
                "GC2CA done",
                "everything given back: true",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run R117T (R120's soak): the shape, two round trips right.
        (
            "run R117T",
            &run_r117t,
            &[
                "R117T (R120's soak, the owner's go 2026-10-30)",
                "R117T references at 74.75 / 1: drawn and repeatable true",
                "R117T 1 round trips",
                "R117T 2 round trips",
                "R117T end: 2 round trips in ",
                "renders wrong into SDRAM at divide 2 0 of 2, into screen 0 at divide 1 0 of 2; not finished 0; underruns 0",
                "R117T done: Windows Mobile's clock put back",
                "GC2CA done",
                "everything given back: true",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run R117S (the speed switch's price): the shape, every render
        // right at every wait (the fork models no divide-1 fault).
        (
            "run R117S",
            &run_r117s,
            &[
                "R117S (the speed switch's price, the owner's fallback 2026-10-29)",
                "R117S references at 74.75 / 1: drawn and repeatable true",
                "R117S wait 1000 us: 1 -> 2 ",
                "R117S wait 300 us: 1 -> 2 ",
                "R117S wait 100 us: 1 -> 2 ",
                "R117S wait 30 us: 1 -> 2 ",
                "R117S wait 10 us: 1 -> 2 ",
                "R117S wait 3 us: 1 -> 2 ",
                "R117S wait 0 us: 1 -> 2 ",
                "renders wrong into SDRAM at divide 2 0 of 2, into screen 0 at divide 1 0 of 2; not finished 0",
                "R117S done: Windows Mobile's clock put back",
                "GC2CA done",
                "everything given back: true",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run R118 test 3 (the renderer's underlay): the shape, every
        // render right, the underlay's picture ours.
        (
            "run R118C",
            &run_r118c,
            &[
                "R118 test 3 (the renderer's underlay, the owner's go 2026-10-29)",
                "R118C references at 74.75 / 1: drawn true, the second drawings 0 words off the first, the underlay's 0",
                "R118C 143.00 / 1, underlay, frame 0: 0 of 2 renders wrong, 0 words; the core ",
                "R118C 143.00 / 1, underlay, frame 1: 0 of 2 renders wrong, 0 words; the core ",
                "R118C 143.00 / 1, underlay, frame 2: 0 of 2 renders wrong, 0 words; the core ",
                "R118C 143.00 / 1, underlay, frame 3: 0 of 2 renders wrong, 0 words; the core ",
                "R118C 143.00 / 1, no underlay (control), frame 0: 0 of 2 renders wrong, 0 words; the core ",
                "R118C 143.00 / 1, no underlay (control), frame 1: 0 of 2 renders wrong, 0 words; the core ",
                "R118C 143.00 / 1, no underlay (control), frame 2: 0 of 2 renders wrong, 0 words; the core ",
                "R118C 143.00 / 1, no underlay (control), frame 3: 0 of 2 renders wrong, 0 words; the core ",
                "R118C done: the display's fetch on, Windows Mobile's clock put back",
                "GC2CA done",
                "everything given back: true",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run R118 test 2 (Dell's clear's shape): the shape, every render
        // right, every form's picture equal to ours (the fork models no
        // divide-1 fault).
        (
            "run R118B",
            &run_r118b,
            &[
                "R118 test 2 (Dell's clear's shape, the owner's chase 2026-10-29)",
                "R118B references at 74.75 / 1: drawn true, the second drawings 0 words off the first; ",
                "R118B 143.00 / 1, flat, our background (control): 0 of 2 renders wrong, 0 words; the core ",
                "R118B 143.00 / 1, flat, our background + quad under: 0 of 2 renders wrong, 0 words; the core ",
                "R118B 143.00 / 1, layered, quad under: 0 of 2 renders wrong, 0 words; the core ",
                "R118B 143.00 / 1, textured, quad under: 0 of 2 renders wrong, 0 words; the core ",
                "R118B 143.00 / 1, depth, quad under: 0 of 2 renders wrong, 0 words; the core ",
                "R118B done: the display's fetch on, Windows Mobile's clock put back",
                "GC2CA done",
                "everything given back: true",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run R118 test 1 (the flat frame into SDRAM): the shape, every
        // render right (the fork models no divide-1 fault).
        (
            "run R118",
            &run_r118,
            &[
                "R118 test 1 (the flat frame into SDRAM, the owner's chase 2026-10-29)",
                "R118 reference at 74.75 / 1: drawn true, the second drawing 0 words off the first, the underlaid form 0",
                "R118 143.00 / 1, as R117: 0 of 2 renders wrong, 0 words; the core ",
                "R118 143.00 / 1, black quad under the mesh: 0 of 2 renders wrong, 0 words; the core ",
                "R118 139.75 / 1, as R117: 0 of 2 renders wrong, 0 words; the core ",
                "R118 133.25 / 1, as R117: 0 of 2 renders wrong, 0 words; the core ",
                "R118 done: the display's fetch on, Windows Mobile's clock put back",
                "GC2CA done",
                "everything given back: true",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run R117 (divide 1 and the copy path): the shape, every render
        // right (the fork models no divide-1 fault).
        (
            "run R117",
            &run_r117,
            &[
                "R117 (divide 1 and the copy path, the owner's go 2026-10-29)",
                "R117 references at 74.75 / 1: drawn true, the second drawing 0 words off the first",
                "R117 143.00 / 1, target SDRAM, lists SDRAM, rows, frame 0: 0 of 2 renders wrong, 0 words (worst 0); the core ",
                "R117 143.00 / 1, target SDRAM, lists SDRAM, rows, frame 1: 0 of 2 renders wrong, 0 words (worst 0); the core ",
                "R117 143.00 / 1, target SDRAM, lists SDRAM, rows, frame 2: 0 of 2 renders wrong, 0 words (worst 0); the core ",
                "R117 143.00 / 1, target SDRAM, lists SDRAM, rows, frame 3: 0 of 2 renders wrong, 0 words (worst 0); the core ",
                "R117 143.00 / 1, target SDRAM, lists on die, rows, frame 0: 0 of 2 renders wrong, 0 words (worst 0); the core ",
                "R117 143.00 / 1, target SDRAM, lists on die, rows, frame 1: 0 of 2 renders wrong, 0 words (worst 0); the core ",
                "R117 143.00 / 1, target SDRAM, lists on die, rows, frame 2: 0 of 2 renders wrong, 0 words (worst 0); the core ",
                "R117 143.00 / 1, target SDRAM, lists on die, rows, frame 3: 0 of 2 renders wrong, 0 words (worst 0); the core ",
                "R117 143.00 / 1, target on die, lists SDRAM, rows, frame 0: 0 of 2 renders wrong, 0 words (worst 0); the core ",
                "R117 143.00 / 1, target on die, lists SDRAM, rows, frame 1: 0 of 2 renders wrong, 0 words (worst 0); the core ",
                "R117 143.00 / 1, target on die, lists SDRAM, rows, frame 2: 0 of 2 renders wrong, 0 words (worst 0); the core ",
                "R117 143.00 / 1, target on die, lists SDRAM, rows, frame 3: 0 of 2 renders wrong, 0 words (worst 0); the core ",
                "R117 143.00 / 1, target on die, lists on die, rows, frame 0: 0 of 2 renders wrong, 0 words (worst 0); the core ",
                "R117 143.00 / 1, target on die, lists on die, rows, frame 1: 0 of 2 renders wrong, 0 words (worst 0); the core ",
                "R117 143.00 / 1, target on die, lists on die, rows, frame 2: 0 of 2 renders wrong, 0 words (worst 0); the core ",
                "R117 143.00 / 1, target on die, lists on die, rows, frame 3: 0 of 2 renders wrong, 0 words (worst 0); the core ",
                "R117 143.00 / 1, target SDRAM, lists on die, checker, frame 0: 0 of 2 renders wrong, 0 words (worst 0); the core ",
                "R117 143.00 / 1, target SDRAM, lists on die, checker, frame 1: 0 of 2 renders wrong, 0 words (worst 0); the core ",
                "R117 143.00 / 1, target SDRAM, lists on die, checker, frame 2: 0 of 2 renders wrong, 0 words (worst 0); the core ",
                "R117 143.00 / 1, target SDRAM, lists on die, checker, frame 3: 0 of 2 renders wrong, 0 words (worst 0); the core ",
                "renders wrong after: at divide 2 0, back at divide 1 0; steps refused 0",
                "R117 done: the display's fetch on, Windows Mobile's clock put back",
                "GC2CA done",
                "everything given back: true",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run R23E (the safe region's end): the shape, every render right
        // (the fork models no display-against-core clash).
        (
            "run R23E",
            &run_r23e,
            &[
                "R23E (the safe region's end, the owner's go 2026-10-29)",
                "R23E 74.75 / 1: references drawn true with the display's fetch off (was on), 0 words off 74.75 / 1's",
                "R23E 74.75 / 1, screen 0x18000-0xae000, frame 0: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 74.75 / 1, screen 0x18000-0xae000, frame 1: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 74.75 / 1, screen 0x18000-0xae000, frame 2: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 74.75 / 1, screen 0x18000-0xae000, frame 3: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 74.75 / 1, screen ending 0xae000: the underrun flag before ",
                "R23E 74.75 / 1, screen 0x1a000-0xb0000, frame 0: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 74.75 / 1, screen 0x1a000-0xb0000, frame 1: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 74.75 / 1, screen 0x1a000-0xb0000, frame 2: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 74.75 / 1, screen 0x1a000-0xb0000, frame 3: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 74.75 / 1, screen ending 0xb0000: the underrun flag before ",
                "R23E 74.75 / 1, screen 0x1a400-0xb0400, frame 0: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 74.75 / 1, screen 0x1a400-0xb0400, frame 1: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 74.75 / 1, screen 0x1a400-0xb0400, frame 2: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 74.75 / 1, screen 0x1a400-0xb0400, frame 3: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 74.75 / 1, screen ending 0xb0400: the underrun flag before ",
                "R23E 74.75 / 1, screen 0x1c000-0xb2000, frame 0: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 74.75 / 1, screen 0x1c000-0xb2000, frame 1: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 74.75 / 1, screen 0x1c000-0xb2000, frame 2: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 74.75 / 1, screen 0x1c000-0xb2000, frame 3: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 74.75 / 1, screen ending 0xb2000: the underrun flag before ",
                "R23E 74.75 / 1, screen 0x20000-0xb6000, frame 0: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 74.75 / 1, screen 0x20000-0xb6000, frame 1: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 74.75 / 1, screen 0x20000-0xb6000, frame 2: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 74.75 / 1, screen 0x20000-0xb6000, frame 3: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 74.75 / 1, screen ending 0xb6000: the underrun flag before ",
                "R23E 149.50 / 2: references drawn true with the display's fetch off (was on), 0 words off 74.75 / 1's",
                "R23E 149.50 / 2, screen 0x18000-0xae000, frame 0: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 149.50 / 2, screen 0x18000-0xae000, frame 1: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 149.50 / 2, screen 0x18000-0xae000, frame 2: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 149.50 / 2, screen 0x18000-0xae000, frame 3: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 149.50 / 2, screen ending 0xae000: the underrun flag before ",
                "R23E 149.50 / 2, screen 0x1a000-0xb0000, frame 0: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 149.50 / 2, screen 0x1a000-0xb0000, frame 1: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 149.50 / 2, screen 0x1a000-0xb0000, frame 2: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 149.50 / 2, screen 0x1a000-0xb0000, frame 3: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 149.50 / 2, screen ending 0xb0000: the underrun flag before ",
                "R23E 149.50 / 2, screen 0x1a400-0xb0400, frame 0: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 149.50 / 2, screen 0x1a400-0xb0400, frame 1: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 149.50 / 2, screen 0x1a400-0xb0400, frame 2: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 149.50 / 2, screen 0x1a400-0xb0400, frame 3: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 149.50 / 2, screen ending 0xb0400: the underrun flag before ",
                "R23E 149.50 / 2, screen 0x1c000-0xb2000, frame 0: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 149.50 / 2, screen 0x1c000-0xb2000, frame 1: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 149.50 / 2, screen 0x1c000-0xb2000, frame 2: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 149.50 / 2, screen 0x1c000-0xb2000, frame 3: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 149.50 / 2, screen ending 0xb2000: the underrun flag before ",
                "R23E 149.50 / 2, screen 0x20000-0xb6000, frame 0: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 149.50 / 2, screen 0x20000-0xb6000, frame 1: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 149.50 / 2, screen 0x20000-0xb6000, frame 2: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 149.50 / 2, screen 0x20000-0xb6000, frame 3: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 149.50 / 2, screen ending 0xb6000: the underrun flag before ",
                "R23E 143.00 / 1: references drawn true with the display's fetch off (was on), 0 words off 74.75 / 1's",
                "R23E 143.00 / 1, screen 0x18000-0xae000, frame 0: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 143.00 / 1, screen 0x18000-0xae000, frame 1: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 143.00 / 1, screen 0x18000-0xae000, frame 2: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 143.00 / 1, screen 0x18000-0xae000, frame 3: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 143.00 / 1, screen ending 0xae000: the underrun flag before ",
                "R23E 143.00 / 1, screen 0x1a000-0xb0000, frame 0: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 143.00 / 1, screen 0x1a000-0xb0000, frame 1: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 143.00 / 1, screen 0x1a000-0xb0000, frame 2: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 143.00 / 1, screen 0x1a000-0xb0000, frame 3: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 143.00 / 1, screen ending 0xb0000: the underrun flag before ",
                "R23E 143.00 / 1, screen 0x1a400-0xb0400, frame 0: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 143.00 / 1, screen 0x1a400-0xb0400, frame 1: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 143.00 / 1, screen 0x1a400-0xb0400, frame 2: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 143.00 / 1, screen 0x1a400-0xb0400, frame 3: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 143.00 / 1, screen ending 0xb0400: the underrun flag before ",
                "R23E 143.00 / 1, screen 0x1c000-0xb2000, frame 0: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 143.00 / 1, screen 0x1c000-0xb2000, frame 1: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 143.00 / 1, screen 0x1c000-0xb2000, frame 2: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 143.00 / 1, screen 0x1c000-0xb2000, frame 3: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 143.00 / 1, screen ending 0xb2000: the underrun flag before ",
                "R23E 143.00 / 1, screen 0x20000-0xb6000, frame 0: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 143.00 / 1, screen 0x20000-0xb6000, frame 1: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 143.00 / 1, screen 0x20000-0xb6000, frame 2: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 143.00 / 1, screen 0x20000-0xb6000, frame 3: 0 of 2 renders wrong, 0 words (worst 0)",
                "R23E 143.00 / 1, screen ending 0xb6000: the underrun flag before ",
                "R23E done: the planes as found, the display's fetch on, Windows Mobile's clock put back",
                "GC2CA done",
                "everything given back: true",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run R23T (on-die targets): the shape, every render right (the
        // fork has no on-die timing).
        (
            "run R23T",
            &run_r23t,
            &[
                "R23T (on-die targets, the owner's go 2026-10-29)",
                "R23T target 0x000000 (0 KiB of 600 past 0xB0000), frame 0: the core ",
                "R23T target 0x000000 (0 KiB of 600 past 0xB0000), frame 1: the core ",
                "R23T target 0x000000 (0 KiB of 600 past 0xB0000), frame 2: the core ",
                "R23T target 0x000000 (0 KiB of 600 past 0xB0000), frame 3: the core ",
                "R23T target 0x01a000 (0 KiB of 600 past 0xB0000), frame 0: the core ",
                "R23T target 0x01a000 (0 KiB of 600 past 0xB0000), frame 1: the core ",
                "R23T target 0x01a000 (0 KiB of 600 past 0xB0000), frame 2: the core ",
                "R23T target 0x01a000 (0 KiB of 600 past 0xB0000), frame 3: the core ",
                "R23T target 0x030000 (88 KiB of 600 past 0xB0000), frame 0: the core ",
                "R23T target 0x030000 (88 KiB of 600 past 0xB0000), frame 1: the core ",
                "R23T target 0x030000 (88 KiB of 600 past 0xB0000), frame 2: the core ",
                "R23T target 0x030000 (88 KiB of 600 past 0xB0000), frame 3: the core ",
                "R23T target 0x050000 (216 KiB of 600 past 0xB0000), frame 0: the core ",
                "R23T target 0x050000 (216 KiB of 600 past 0xB0000), frame 1: the core ",
                "R23T target 0x050000 (216 KiB of 600 past 0xB0000), frame 2: the core ",
                "R23T target 0x050000 (216 KiB of 600 past 0xB0000), frame 3: the core ",
                "R23T target 0x066000 (304 KiB of 600 past 0xB0000), frame 0: the core ",
                "R23T target 0x066000 (304 KiB of 600 past 0xB0000), frame 1: the core ",
                "R23T target 0x066000 (304 KiB of 600 past 0xB0000), frame 2: the core ",
                "R23T target 0x066000 (304 KiB of 600 past 0xB0000), frame 3: the core ",
                "R23T target 0x200000 (600 KiB of 600 past 0xB0000), frame 0: the core ",
                "R23T target 0x200000 (600 KiB of 600 past 0xB0000), frame 1: the core ",
                "R23T target 0x200000 (600 KiB of 600 past 0xB0000), frame 2: the core ",
                "R23T target 0x200000 (600 KiB of 600 past 0xB0000), frame 3: the core ",
                "R23T target 0x296000 (600 KiB of 600 past 0xB0000), frame 0: the core ",
                "R23T target 0x296000 (600 KiB of 600 past 0xB0000), frame 1: the core ",
                "R23T target 0x296000 (600 KiB of 600 past 0xB0000), frame 2: the core ",
                "R23T target 0x296000 (600 KiB of 600 past 0xB0000), frame 3: the core ",
                "R23T done: the display's fetch on, Windows Mobile's clock put back",
                "GC2CA done",
                "everything given back: true",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run R114 (the screen-1 address sweep): the shape, every render
        // right (the fork has no bank timing).
        (
            "run R114",
            &run_r114,
            &[
                "R114 (the screen-1 address sweep, the owner's go 2026-10-29)",
                "R114 target +0x0, frame 0: the core ",
                "R114 target +0x0, frame 1: the core ",
                "R114 target +0x0, frame 2: the core ",
                "R114 target +0x0, frame 3: the core ",
                "R114 target +0x800, frame 0: the core ",
                "R114 target +0x800, frame 1: the core ",
                "R114 target +0x800, frame 2: the core ",
                "R114 target +0x800, frame 3: the core ",
                "R114 target +0x1000, frame 0: the core ",
                "R114 target +0x1000, frame 1: the core ",
                "R114 target +0x1000, frame 2: the core ",
                "R114 target +0x1000, frame 3: the core ",
                "R114 target +0x2000, frame 0: the core ",
                "R114 target +0x2000, frame 1: the core ",
                "R114 target +0x2000, frame 2: the core ",
                "R114 target +0x2000, frame 3: the core ",
                "R114 target +0x4000, frame 0: the core ",
                "R114 target +0x4000, frame 1: the core ",
                "R114 target +0x4000, frame 2: the core ",
                "R114 target +0x4000, frame 3: the core ",
                "R114 target +0x8000, frame 0: the core ",
                "R114 target +0x8000, frame 1: the core ",
                "R114 target +0x8000, frame 2: the core ",
                "R114 target +0x8000, frame 3: the core ",
                "R114 target +0x10000, frame 0: the core ",
                "R114 target +0x10000, frame 1: the core ",
                "R114 target +0x10000, frame 2: the core ",
                "R114 target +0x10000, frame 3: the core ",
                "R114 target +0x20000, frame 0: the core ",
                "R114 target +0x20000, frame 1: the core ",
                "R114 target +0x20000, frame 2: the core ",
                "R114 target +0x20000, frame 3: the core ",
                "R114 target +0x40000, frame 0: the core ",
                "R114 target +0x40000, frame 1: the core ",
                "R114 target +0x40000, frame 2: the core ",
                "R114 target +0x40000, frame 3: the core ",
                "R114 target +0x80000, frame 0: the core ",
                "R114 target +0x80000, frame 1: the core ",
                "R114 target +0x80000, frame 2: the core ",
                "R114 target +0x80000, frame 3: the core ",
                "R114 target +0x100000, frame 0: the core ",
                "R114 target +0x100000, frame 1: the core ",
                "R114 target +0x100000, frame 2: the core ",
                "R114 target +0x100000, frame 3: the core ",
                "R114 textures +0x0, frame 0: the core ",
                "R114 textures +0x0, frame 1: the core ",
                "R114 textures +0x0, frame 2: the core ",
                "R114 textures +0x0, frame 3: the core ",
                "R114 textures +0x800, frame 0: the core ",
                "R114 textures +0x800, frame 1: the core ",
                "R114 textures +0x800, frame 2: the core ",
                "R114 textures +0x800, frame 3: the core ",
                "R114 textures +0x1000, frame 0: the core ",
                "R114 textures +0x1000, frame 1: the core ",
                "R114 textures +0x1000, frame 2: the core ",
                "R114 textures +0x1000, frame 3: the core ",
                "R114 textures +0x2000, frame 0: the core ",
                "R114 textures +0x2000, frame 1: the core ",
                "R114 textures +0x2000, frame 2: the core ",
                "R114 textures +0x2000, frame 3: the core ",
                "R114 textures +0x4000, frame 0: the core ",
                "R114 textures +0x4000, frame 1: the core ",
                "R114 textures +0x4000, frame 2: the core ",
                "R114 textures +0x4000, frame 3: the core ",
                "R114 textures +0x8000, frame 0: the core ",
                "R114 textures +0x8000, frame 1: the core ",
                "R114 textures +0x8000, frame 2: the core ",
                "R114 textures +0x8000, frame 3: the core ",
                "R114 textures +0x10000, frame 0: the core ",
                "R114 textures +0x10000, frame 1: the core ",
                "R114 textures +0x10000, frame 2: the core ",
                "R114 textures +0x10000, frame 3: the core ",
                "R114 textures +0x20000, frame 0: the core ",
                "R114 textures +0x20000, frame 1: the core ",
                "R114 textures +0x20000, frame 2: the core ",
                "R114 textures +0x20000, frame 3: the core ",
                "R114 textures +0x40000, frame 0: the core ",
                "R114 textures +0x40000, frame 1: the core ",
                "R114 textures +0x40000, frame 2: the core ",
                "R114 textures +0x40000, frame 3: the core ",
                "R114 textures +0x80000, frame 0: the core ",
                "R114 textures +0x80000, frame 1: the core ",
                "R114 textures +0x80000, frame 2: the core ",
                "R114 textures +0x80000, frame 3: the core ",
                "R114 textures +0x100000, frame 0: the core ",
                "R114 textures +0x100000, frame 1: the core ",
                "R114 textures +0x100000, frame 2: the core ",
                "R114 textures +0x100000, frame 3: the core ",
                "R114 done: Windows Mobile's clock put back",
                "GC2CA done",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // R113 run 2 (when a flip written with DSCTRL bit 24 shows): the
        // shape only (the fork has no DSIG or update latch).
        (
            "run R113B",
            &run_r113b,
            &[
                "R113 run 2 (when a flip written with DSCTRL bit 24 shows, the owner's go 2026-10-29)",
                "R113 run 2 bit 24, DUCTRL = 1 after the write, written at line 100: 1 flips: B first whole in refresh",
                "R113 run 2 bit 24, DUCTRL = 1 after the write, written at line 320: 1 flips: B first whole in refresh",
                "R113 run 2 bit 24, DUCTRL = 1 after the write, written at line 600: 1 flips: B first whole in refresh",
                "R113 run 2 bit 24, DUCTRL = 1 after the write: DSCTRL back to 0x",
                "R113 run 2 bit 24, DUCTRL = 1 after the write and at each refresh, written at line 100: 1 flips: B first whole in refresh",
                "R113 run 2 bit 24, DUCTRL = 1 after the write and at each refresh, written at line 320: 1 flips: B first whole in refresh",
                "R113 run 2 bit 24, DUCTRL = 1 after the write and at each refresh, written at line 600: 1 flips: B first whole in refresh",
                "R113 run 2 bit 24, DUCTRL = 1 after the write and at each refresh: DSCTRL back to 0x",
                "R113 run 2 bit 24, no DUCTRL write, written at line 100: 1 flips: B first whole in refresh",
                "R113 run 2 bit 24, no DUCTRL write, written at line 320: 1 flips: B first whole in refresh",
                "R113 run 2 bit 24, no DUCTRL write, written at line 600: 1 flips: B first whole in refresh",
                "R113 run 2 bit 24, no DUCTRL write: DSCTRL back to 0x",
                "R113 run 2 done: DSCTRL 0x",
                "GC2CA done",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run R113 (flips written mid-scan, the owner's go 2026-10-29):
        // GSCADR repointed at three lines under each DSCTRL update setting.
        // The fork has no DSIG (every flip "not judged") and no scan-out
        // latch: the run's shape, every line reached, every setting put
        // back, is what this checks.
        (
            "run R113",
            &run_r113,
            &[
                "R113 (flips written mid-scan, the owner's go 2026-10-29): GSCADR repointed between two whole pictures",
                "R113 as found, written at line 100: 2 flips: held to the next refresh 0, torn 0, new at once 0, the next refresh not new 0, not judged 2, line missed 0",
                "R113 as found, written at line 320: 2 flips: held to the next refresh 0, torn 0, new at once 0, the next refresh not new 0, not judged 2, line missed 0",
                "R113 as found, written at line 600: 2 flips: held to the next refresh 0, torn 0, new at once 0, the next refresh not new 0, not judged 2, line missed 0",
                "R113 as found: DSCTRL back to 0x",
                "R113 as found, DUCTRL = 1 (Dell's flip), written at line 100: 2 flips: held to the next refresh 0, torn 0, new at once 0, the next refresh not new 0, not judged 2, line missed 0",
                "R113 as found, DUCTRL = 1 (Dell's flip), written at line 320: 2 flips: held to the next refresh 0, torn 0, new at once 0, the next refresh not new 0, not judged 2, line missed 0",
                "R113 as found, DUCTRL = 1 (Dell's flip), written at line 600: 2 flips: held to the next refresh 0, torn 0, new at once 0, the next refresh not new 0, not judged 2, line missed 0",
                "R113 as found, DUCTRL = 1 (Dell's flip): DSCTRL back to 0x",
                "R113 DSCTRL bit 24 (UPDCNT), DUCTRL = 1, written at line 100: 2 flips: held to the next refresh 0, torn 0, new at once 0, the next refresh not new 0, not judged 2, line missed 0",
                "R113 DSCTRL bit 24 (UPDCNT), DUCTRL = 1, written at line 320: 2 flips: held to the next refresh 0, torn 0, new at once 0, the next refresh not new 0, not judged 2, line missed 0",
                "R113 DSCTRL bit 24 (UPDCNT), DUCTRL = 1, written at line 600: 2 flips: held to the next refresh 0, torn 0, new at once 0, the next refresh not new 0, not judged 2, line missed 0",
                "R113 DSCTRL bit 24 (UPDCNT), DUCTRL = 1: DSCTRL back to 0x",
                "R113 DSCTRL bit 25 (UPDINTCNT), DUCTRL = 1, written at line 100: 2 flips: held to the next refresh 0, torn 0, new at once 0, the next refresh not new 0, not judged 2, line missed 0",
                "R113 DSCTRL bit 25 (UPDINTCNT), DUCTRL = 1, written at line 320: 2 flips: held to the next refresh 0, torn 0, new at once 0, the next refresh not new 0, not judged 2, line missed 0",
                "R113 DSCTRL bit 25 (UPDINTCNT), DUCTRL = 1, written at line 600: 2 flips: held to the next refresh 0, torn 0, new at once 0, the next refresh not new 0, not judged 2, line missed 0",
                "R113 DSCTRL bit 25 (UPDINTCNT), DUCTRL = 1: DSCTRL back to 0x",
                "R113 DSCTRL bit 26 (UPDSYNCCNT), DUCTRL = 1, written at line 100: 2 flips: held to the next refresh 0, torn 0, new at once 0, the next refresh not new 0, not judged 2, line missed 0",
                "R113 DSCTRL bit 26 (UPDSYNCCNT), DUCTRL = 1, written at line 320: 2 flips: held to the next refresh 0, torn 0, new at once 0, the next refresh not new 0, not judged 2, line missed 0",
                "R113 DSCTRL bit 26 (UPDSYNCCNT), DUCTRL = 1, written at line 600: 2 flips: held to the next refresh 0, torn 0, new at once 0, the next refresh not new 0, not judged 2, line missed 0",
                "R113 DSCTRL bit 26 (UPDSYNCCNT), DUCTRL = 1: DSCTRL back to 0x",
                "R113 DSCTRL UPDWAIT 1 (bits 16-19), DUCTRL = 1, written at line 100: 2 flips: held to the next refresh 0, torn 0, new at once 0, the next refresh not new 0, not judged 2, line missed 0",
                "R113 DSCTRL UPDWAIT 1 (bits 16-19), DUCTRL = 1, written at line 320: 2 flips: held to the next refresh 0, torn 0, new at once 0, the next refresh not new 0, not judged 2, line missed 0",
                "R113 DSCTRL UPDWAIT 1 (bits 16-19), DUCTRL = 1, written at line 600: 2 flips: held to the next refresh 0, torn 0, new at once 0, the next refresh not new 0, not judged 2, line missed 0",
                "R113 DSCTRL UPDWAIT 1 (bits 16-19), DUCTRL = 1: DSCTRL back to 0x",
                "R113 done: DSCTRL 0x",
                "GC2CA done",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2DE (R20's option 1 soak): 3D targets at screen 1's place
        // (outside the safe region) at 149.50 / 2 with screen 0 shown, every
        // render read back word for word; each screen's render cost, the
        // 2D copy back, and the two fallbacks side by side. The fork has
        // no clock or DSIG: the run's shape, the renders and the copies are
        // what this checks.
        (
            "run GC2DE",
            &run_gc2de,
            &[
                "GC2DE (R20, the owner's option 1): 3D targets at screen 1's place 0x296000",
                "GC2DE at Windows Mobile's clock into screen 1: words off the references by frame [0, 0, 0, 0]",
                "GC2DE 149.50 / 2: lock Some(",
                "GC2DE at 149.50 / 2 into screen 0's place (inside): 4 of 4 pictures the references'",
                "GC2DE soak at 149.50 / 2 into screen 1 with screen 0 shown: 12 renders checked word for word, 0 WRONG (first (render, frame, words) None, the most words 0), 0 not rendered, 0 hung, 0 poison fills failed",
                "GC2DE soak frame 3 the core: ",
                "GC2DE cost frame 3: into screen 0 the core ",
                ", 0 wrong; into screen 1 the core ",
                "GC2DE the 2D copy from screen 1 back into screen 0 at 149.50 / 2: the whole screen ",
                "; 0 wrong or not finished",
                "GC2DE fallback blanking start (screen 0, today's until the soak passes), 4 frames asked for at lines [0, 213, 426]: from the ask to the render done ",
                "GC2DE fallback option 1 (screen 1, flip, copy back), 4 frames asked for at lines [0, 213, 426]: from the ask to the render done ",
                "; 0 wrong or not rendered; 0 scan lines missed in both",
                "GC2DE done: Windows Mobile's clock put back; 0 hung in all",
                "GC2CA done",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2CS: the smooth 2x's pictures for the owner's eye, eight
        // labelled photos (the emulator shows each at once: no button; the
        // fork models no scaler, so the run's shape is what this checks).
        (
            "run GC2CS",
            &run_gc2cs,
            &[
                "GC2CS 1 TEXTURED FRAME FULL SIZE (NATIVE): rendered Some(",
                "GC2CS 4 TEXTURED FRAME OURS: SMOOTH 2X: rendered Some(",
                "GC2CS 8 FLAT FRAME OURS: SMOOTH 2X: rendered Some(",
                "GC2CA done",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2CP: the smooth 2x's routes against native on the game
        // frames (480 x 640; 480 x 320; 240 x 320 converted serially with
        // a full or a registers-only renderer switch, or overlapped with the
        // next frame's lists), at 74.75 / 1 and 143.00 / 1, every picture
        // checked; GC2CM's card converted at each clock. The emulator's
        // UYVY words are wrong as in GC2CM (no row order, no DOT3) and its
        // times its own: the run's shape is what this checks.
        (
            "run GC2CP",
            &run_gc2cp,
            &[
                "GC2CP (R78 item 2): 4 game frames",
                "GC2CP 240 x 320 frame 3 reference:",
                "GC2CM GC2CP card at 74.75 / 1 check:",
                "GC2CP 74.75 / 1 (a) 480 x 640 native frame 0: 2 of 2 frames, 0 pictures wrong",
                "GC2CP 74.75 / 1 (b) 480 x 320 frame 3: 2 of 2 frames, 0 pictures wrong",
                "GC2CP 74.75 / 1 (c) 240 x 320 converted, serial, full switch frame 0: 2 of 2 frames;",
                "GC2CP 74.75 / 1 (c') 240 x 320 converted, serial, registers switch frame 3: 2 of 2 frames;",
                "GC2CM GC2CP card at 143.00 / 1 check:",
                "GC2CP 143.00 / 1 (d) 240 x 320 converted, overlapped frame 3: 2 frames;",
                "GC2CP 143.00 / 1 (d') 240 x 320 converted, lists overlapped, 2D apart frame 3: 2 frames;",
                "GC2CP 143.00 / 1 (e) 480 x 320, two frames in flight frame 3: 3 frames;",
                "    its frame: ",
                "GC2CP 143.00 / 1 speed-up (d) 240 x 320 converted, overlapped against native",
                "GC2CP done:",
                "GC2CA done",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2CZ (R20's scheduler): the run's shape (the emulator's scan
        // and render times are its own, not the X51v's).
        (
            "run GC2CZ",
            &run_gc2cz,
            &[
                "GC2CZ (R20): the scheduler",
                "GC2CZ WM 99.67 / 2 interleaved: ",
                "GC2CZ 74.75 / 1 rows: ",
                "GC2CZ done: ",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2CY (R80 at WM clock and 74.75 / 1): the one-pixel faults
        // with more renders, the time split, R3's bus timing. The fork draws
        // no divide-1 fault; the run's shape and the bus switch are checked.
        (
            "run GC2CY",
            &run_gc2cy,
            &[
                "GC2CY WM 99.67 / 2 360 x 480 frame 0: 0 of 3 renders wrong",
                "GC2CY 74.75 / 1 240 x 320 frame 3: 0 of 3 renders wrong",
                "GC2CY part B: MSC1 as found",
                "GC2CY 74.75 / 1 CS3 R3 7/3/1 (MSC1 0x",
                "native frame 2, the region array written whole: CPU set-up",
                "GC2CY done:",
                "GC2CA done",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2CX (R89): GC2CM's conversion storing the converter's
        // offsets. The fork models neither row-order textures nor DOT3, so
        // its words are wrong here as in GC2CM; the run's shape is what this
        // checks (the X51v's run checks the words).
        (
            "run GC2CX",
            &run_gc2cx,
            &[
                "GC2CX (R89): the offsets stored",
                "GC2CX RGB565 card U: off by",
                "GC2CX RGB565 card check: ",
                "GC2CX ARGB8888 card check: ",
                "GC2CM done:",
                "GC2CA done",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2CW (R89): DOT3, MODULATE and the offsets blend measured on
        // a ramp. The fork models MODULATE and blending, not DOT3: its DOT3
        // lines are the fork's, the run's shape is what this checks.
        (
            "run GC2CW",
            &run_gc2cw,
            &[
                "GC2CW (R89): the ramp v = 32 row + column",
                "GC2CW try 0 DOT3 TEX2 0x86096612 tint 0xff002000",
                "GC2CW try 8 MODULATE TEX2",
                "GC2CW try 9 MODULATE + offsets",
                "GC2CW try 12 DOT3 + offsets",
                "GC2CW try 12 green v224: ",
                "GC2CW done:",
                "GC2CA done",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2CV (R93 after GC2CU): the textured four-corner object over
        // a warped quad. The fork draws it as its two triangles (the rule
        // on the X51v is what the run measures), so here every way is the
        // same and the grids are the triangles'.
        (
            "run GC2CV",
            &run_gc2cv,
            &[
                "GC2CV mesh Triangles: Some(",
                "GC2CV mesh Quads: Some(",
                "GC2CV mesh: the same both ways",
                "GC2CV warped Triangles y=13:",
                "GC2CV warped Quads y=611:",
                "GC2CV warped: the same both ways",
                "GC2CV parallelogram: the same both ways",
                "GC2CV done:",
                "GC2CA done",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2CP at Windows Mobile's clock and 74.75 / 1 (the owner's
        // overnight order: the pipelined ways split into set-up and wait).
        (
            "run GC2CP low",
            &run_gc2cp_low,
            &[
                "GC2CP WM 99.67 / 2 (a) 480 x 640 native frame 0: 2 of 2 frames, 0 pictures wrong",
                "GC2CP 74.75 / 1 (d') 240 x 320 converted, lists overlapped, 2D apart frame 3: 2 frames;",
                "GC2CP WM 99.67 / 2 (e) 480 x 320, two frames in flight frame 3: 3 frames;",
                "waiting on the core",
                "GC2CP done:",
                "GC2CA done",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // GC2CP low with way (e)'s every frame checked (the ruling packet of
        // 2026-10-29, G4): 100 frames a kind, each read back and the target
        // poisoned again.
        (
            "run GC2CP every",
            &run_gc2cp_every,
            &[
                "GC2CP WM 99.67 / 2 (e) 480 x 320, two frames in flight frame 0: every frame checked: 100 of 100, 0 wrong",
                "GC2CP 74.75 / 1 (e) 480 x 320, two frames in flight frame 3: every frame checked: 100 of 100, 0 wrong",
                "GC2CP done:",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // R102: route 3's pipelined way (d') against its serial (c'), every
        // pipelined frame hashed against the serial output, at three clocks.
        (
            "run GC2CP R102",
            &run_gc2cp_r102,
            &[
                "GC2CP WM 99.67 / 2 (d') 240 x 320 converted, lists overlapped, 2D apart frame 0: every frame checked: 101 of 101, 0 differ from the serial output",
                "GC2CP 149.50 / 2 (d') 240 x 320 converted, lists overlapped, 2D apart frame 3: every frame checked: 101 of 101, 0 differ from the serial output",
                "GC2CP done:",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // GC2I's soak at divide 2 alone (the ruling packet, ruling 3): no
        // map, 149.50 / 2 and 133.25 / 2, every kind; 10 s a setting here.
        // Runs to its end and returns (the pinned release doesn't model
        // Windows Mobile's clock: stepped on the wm-clocks build).
        (
            "run GC2I divide 2",
            &run_gc2i_div2,
            &["RETROPANE-GPUBENCH: done"],
            false,
            (0, 0, 0),
        ),
        // The same a rung down: 133.25 / 2 then 117.00 / 2.
        (
            "run GC2I divide 2 low",
            &run_gc2i_div2_low,
            &["RETROPANE-GPUBENCH: done"],
            false,
            (0, 0, 0),
        ),
        // R103: 149.50 / 2 in the shipping configuration (the 16-bit kinds
        // inside the safe region), a ladder down on a failure; the
        // emulator draws every render right, so its top rung ends it.
        (
            "run GC2I divide 2 top",
            &run_gc2i_div2_top,
            &[
                "GC2I soak: 6 kinds (inside the safe region, as the OS draws (R103)) at 4 settings",
                "GC2I soak: 149.50 / 2 is the highest clean rung of this run (R103)",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2CU (R78 item 3, R93): each quad as one four-corner object
        // against two triangles; the fork reads list layouts 5 and 9 as
        // the host model does (drawn as the strip's two triangles), so every
        // kind and game frame must draw the same both ways here.
        (
            "run GC2CU",
            &run_gc2cu,
            &[
                "GC2CU (R78 item 3): each quad as one four-corner object",
                "GC2CU kind 0 (RGB565 Gouraud): ",
                "GC2CU kind 0 with eight quads off the screen: Some(0) words off the triangles' picture",
                "GC2CU kind 5 (RGB565 bilinear 565 texture): ",
                "GC2CU kind 6 (RGB565 Gouraud + blended 4444): ",
                "GC2CP 480 x 320 frame 3 reference:",
                "GC2CU triangles 74.75 / 1 (a) 480 x 640 native frame 0: 2 of 2 frames, 0 pictures wrong",
                "GC2CU quads 74.75 / 1 (a) 480 x 640 native frame 2: 2 of 2 frames, 0 pictures wrong",
                "GC2CU quads 143.00 / 1 (b) 480 x 320 frame 3: 2 of 2 frames, 0 pictures wrong",
                "    its frame: ",
                // Kind 1 is RGB565 dithered (0x650 code 9), which the fork
                // doesn't model (docs/hw/emulator.md): its first render
                // writes 32-bit pixels at the 16-bit stride. Every other kind
                // must draw the same; the X51v must show 13 of 13.
                "kinds drawn the same both ways at Windows Mobile's clock: 12 of 13 (different: 1); game frames wrong against their own form's references: triangles 0, four-corner objects 0",
                "GC2CA done",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        (
            "run GC2BY",
            &run_gc2by,
            &[
                "GC2BY (R76): the game frames on the warm ladder",
                "GC2BY warm-up reference:",
                "GC2BY A slice 10: ",
                "GC2BY frame 3 reference:",
                "GC2BY 149.50 / 1 (lock ",
                "GC2BY 108.33 / 1 (lock ",
                "GC2BY 149.50 / 2 (lock ",
                "GC2BY steps with every game frame right: 149.50 / 1; 146.25 / 1; 143.00 / 1; 139.75 / 1; 133.25 / 1; 117.00 / 1; 108.33 / 1; 149.50 / 2; ",
                "GC2BY controls at Windows Mobile's clock after: 4 of 4 right",
                "GC2BY done, TEMP0",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2BX (R74, R69): the game test and the fix search, shortened
        // (the emulator draws exactly: every frame right, every trial clean
        // and confirmed, the first three proven).
        (
            "run GC2BX",
            &run_gc2bx,
            &[
                "GC2BX (R74, R69): the game test and every driver-level fix",
                "GC2BX warm-up reference:",
                "GC2BX A slice 10: ",
                "GC2BX frame 7 reference:",
                "GC2BX games the mesh depth tested: 0 of 1 wrong",
                "GC2BX games the generator (scene 3 split TL-BR): 0 of 1 wrong",
                "GC2BX games done, TEMP0",
                "GC2BX search done: ",
                "GC2BX proof Reg(",
                "GC2BX done, TEMP0",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2CR (R80): the reduced-size game frames on the warm ladder
        // 143.00 down to 108.33 / 1, one round a size and step (the
        // emulator draws exactly: every step right).
        (
            "run GC2CR",
            &run_gc2cr,
            &[
                "GC2CR (R80): the game frames at",
                "GC2CR warm-up reference:",
                "GC2CR A slice 10: ",
                "GC2CR 240 x 320 frame 3 reference:",
                "GC2CR 143.00 / 1 480 x 320 (lock ",
                "GC2CR 108.33 / 1 480 x 640 (lock ",
                "GC2CR 480 x 320: steps with every frame right: 143.00 / 1; 139.75 / 1; 133.25 / 1; 117.00 / 1; 108.33 / 1; ",
                "GC2CR done, TEMP0",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2CQ (R69): the PowerVR2's settings by value, a few trials of
        // each part (the emulator models none of them: its map shows which
        // fields its model draws differently, its hot part holds each to
        // its own pictures).
        (
            "run GC2CQ",
            &run_gc2cq,
            &[
                "GC2CQ (R69, the PowerVR2's settings): 6 trials",
                "GC2CQ generator reference:",
                "GC2CQ textured mesh reference:",
                "GC2CQ map done: ",
                "GC2CQ warm-up reference:",
                "GC2CQ A slice 10: ",
                "GC2CQ generator again reference:",
                "GC2CQ hot done: 6 trials",
                "GC2CQ done, TEMP0",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2BW (R74): the game-shaped frames, shortened (the emulator
        // draws exactly: every frame right).
        (
            "run GC2BW",
            &run_gc2bw,
            &[
                "GC2BW (R74): the crack in a game-shaped frame",
                "GC2BW warm-up reference:",
                "GC2BW A slice 10: ",
                "GC2BW frame 3 reference:",
                "GC2BW control reference:",
                "GC2BW the textured mesh: 0 of 2 wrong at 149.50 / 1",
                "GC2BW the mesh depth tested: 0 of 2 wrong at 149.50 / 1",
                "GC2BW controls at Windows Mobile's clock after: 5 of 5 right",
                "GC2BW done, TEMP0",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2BV (R69): the seals, shortened (the emulator draws
        // exactly, so both seals are clean and both proofs run).
        (
            "run GC2BV",
            &run_gc2bv,
            &[
                "GC2BV (R69): the crack on the rectangles' diagonal against its seals",
                "GC2BV reference:",
                "GC2BV A slice 10: ",
                "GC2BV forms 149.50 / 1: controls at Windows Mobile's clock after: 7 of 7 right",
                "GC2BV forms done, TEMP0",
                "GC2BV proof band done, TEMP0",
                "GC2BV proof band sweep 149.50 / 1: controls at Windows Mobile's clock after: 23 of 23 right",
                "GC2BV proof copy sweep done, TEMP0",
                "GC2BV done, TEMP0",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2BU (R69): the diagonal's forms, shortened (the emulator
        // draws exactly, so every form is clean and all four proofs run).
        (
            "run GC2BU",
            &run_gc2bu,
            &[
                "GC2BU (R69): the race's dropped group on the quad's diagonal",
                "GC2BU reference:",
                "GC2BU A slice 10: ",
                "GC2BU forms 149.50 / 1: controls at Windows Mobile's clock after: 6 of 6 right",
                "GC2BU forms done, TEMP0",
                "GC2BU map 149.50 / 1: controls at Windows Mobile's clock after: 8 of 8 right",
                "GC2BU map done, TEMP0",
                "GC2BU proof depth done, TEMP0",
                "GC2BU proof flip done, TEMP0",
                "GC2BU done, TEMP0",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2BT (R69): the region array on the die, shortened (the
        // emulator draws exactly, so the die side is clean and the proof
        // runs).
        (
            "run GC2BT",
            &run_gc2bt,
            &[
                "GC2BT (R69): the region array (and the manager's block) on the die",
                "GC2BT base's region array at 0x",
                "GC2BT die's region array at 0x",
                "GC2BT warm-up reference:",
                "GC2BT A slice 10: ",
                "GC2BT die at Windows Mobile's clock: Some(0) words off",
                "GC2BT the region array on the die on scene 20: 0 of 2 wrong",
                "GC2BT the CPU quiet (base2's array) on scene 3 without its layer: 0 of 2 wrong",
                "the region array on the die clean in all three: true",
                "GC2BT proof textured done, TEMP0",
                "GC2BT done, TEMP0",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2BS (R69): the green step's selector, bit and the port's
        // cures, shortened (the emulator draws exactly, so every way is
        // clean and the proof runs its first three).
        (
            "run GC2BS",
            &run_gc2bs,
            &[
                "GC2BS (R69): the green step's selector and bit",
                "GC2BS warm-up reference:",
                "GC2BS A slice 10: ",
                "GC2BS forms done, TEMP0",
                "GC2BS the target in SDRAM: 0 of 2 wrong",
                "GC2BS ways done, TEMP0",
                "GC2BS proof the video plane as scanout, scene 3 with its layer: 0 of 2 wrong",
                "GC2BS done, TEMP0",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2BR (R69): scene 3's green step against the shading,
        // shortened (the emulator draws exactly).
        (
            "run GC2BR",
            &run_gc2br,
            &[
                "GC2BR (R69): scene 3's green step against the shading",
                "GC2BR reference:",
                "GC2BR A slice 10: ",
                "GC2BR rectangles Gouraud done, TEMP0",
                "GC2BR sweep Gouraud done, TEMP0",
                "GC2BR scene 3 again done, TEMP0",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2BQ (R69): the tile orders at peak heat, shortened (the
        // emulator draws exactly).
        (
            "run GC2BQ",
            &run_gc2bq,
            &[
                "GC2BQ (R69): the tile race's residue at peak heat",
                "GC2BQ reference:",
                "GC2BQ A slice 10: ",
                "GC2BQ scrambled done, TEMP0",
                "GC2BQ columns reversed scene 3 done, TEMP0",
                "GC2BQ checker again scene 3 done, TEMP0",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2BP (R69): the four cures' proof with the limited nudge and
        // scene 3 alone, shortened (the emulator draws exactly).
        (
            "run GC2BP",
            &run_gc2bp,
            &[
                "GC2BP (R69): divide 1's four cures at once",
                "GC2BP the plane enlarged to 16 x 16 at 0x",
                ": 0 words read back wrong",
                "GC2BP reference:",
                "GC2BP A slice 10: ",
                "GC2BP kinds done, TEMP0",
                "GC2BP scene 3 done, TEMP0",
                "GC2BP soak done, TEMP0",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2BO (R69): the four cures' proof, shortened (10 s warm-up, a
        // round a scene, a 20 s soak; the emulator draws exactly).
        (
            "run GC2BO",
            &run_gc2bo,
            &[
                "GC2BO (R69): divide 1's four cures at once",
                "GC2BO the plane enlarged to 16 x 16 at 0x",
                ": 0 words read back wrong",
                "GC2BO reference:",
                "GC2BO A slice 10: ",
                "GC2BO flat done, TEMP0",
                "GC2BO textured done, TEMP0",
                "GC2BO ring done, TEMP0",
                "GC2BO kinds done, TEMP0",
                "GC2BO soak done, TEMP0",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2BN (R69): the nudge's size, shortened (10 s warm-up, two
        // renders a scene; the emulator draws exactly).
        (
            "run GC2BN",
            &run_gc2bn,
            &[
                "GC2BN (R69): the nudge's size against the bilinear error",
                "GC2BN the 16 x 16 ramp at 0x",
                ": 0 words read back wrong",
                "GC2BN A slice 10: ",
                "GC2BN scene 0 reference:",
                "GC2BN scene 15 (the 16 x 16 enlargement +32;",
                "0 of 2 renders wrong, 0 wrong pixels",
                "GC2BN done, TEMP0",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2BM (R69): the nudge, shortened (10 s warm-up, two renders a
        // scene; the emulator draws exactly, so no scene logs counts).
        (
            "run GC2BM",
            &run_gc2bm,
            &[
                "GC2BM (R69): the nudged span against the bilinear error",
                "GC2BM the 16 x 16 ramp at 0x",
                ": 0 words read back wrong",
                "GC2BM A slice 10: ",
                "GC2BM scene 0 reference:",
                "GC2BM scene 15 (kind 10 nudged, the other diagonal;",
                "0 of 2 renders wrong, 0 wrong pixels",
                "GC2BM done, TEMP0",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2BL (R69): the map, shortened (10 s warm-up, two renders a
        // scene; the emulator draws exactly, so no scene logs counts).
        (
            "run GC2BL",
            &run_gc2bl,
            &[
                "GC2BL (R69): the bilinear error mapped",
                "GC2BL the 16 x 16 texture at 0x",
                ": 0 words read back wrong",
                "GC2BL A slice 10: ",
                "GC2BL scene 0 reference:",
                "GC2BL scene 0 (1/32 texel a pixel, texel-centre phase (GC2BK's wrong case); u0 2048, span 61440, v 2048; lock",
                "GC2BL scene 11 (1/32, v between two rows; u0 2048, span 61440, v 4096; lock",
                "GC2BL done, TEMP0",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2BK (R69): the threshold and the enlarged plane, shortened
        // (10 s warm-up, a round a scene; the emulator draws exactly).
        (
            "run GC2BK",
            &run_gc2bk,
            &[
                "GC2BK (R69): the bilinear filter's magnification limit",
                "GC2BK the plane enlarged to 8 x 8 at 0x",
                "GC2BK the plane enlarged to 16 x 16 at 0x",
                ": 0 words read back wrong",
                "GC2BK reference:",
                "GC2BK A slice 10: ",
                "GC2BK threshold done, TEMP0",
                "GC2BK enlarged 8 done, TEMP0",
                "GC2BK enlarged 16 done, TEMP0",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2BJ (R69): the split, shortened (10 s warm-up, a round a
        // scene; the emulator draws exactly).
        (
            "run GC2BJ",
            &run_gc2bj,
            &[
                "GC2BJ (R69): the texel error at large extent",
                "GC2BJ reference:",
                "GC2BJ A slice 10: ",
                "GC2BJ 149.50 / 1: 1 rounds (8 renders)",
                "GC2BJ 149.50 / 1 done, TEMP0",
                "GC2BJ 146.25 / 1 done, TEMP0",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2BI (R69): the magnification ladder, shortened (10 s
        // warm-up, a round a scene; the emulator draws exactly).
        (
            "run GC2BI",
            &run_gc2bi,
            &[
                "GC2BI (R69): the bilinear filter's magnification",
                "GC2BI reference:",
                "GC2BI A slice 10: ",
                "GC2BI compensated 149.50 / 1: 1 rounds (11 renders)",
                "GC2BI compensated done, TEMP0",
                "GC2BI 149.50 / 1 done, TEMP0",
                "GC2BI 146.25 / 1 done, TEMP0",
                "GC2BI 143.00 / 1 done, TEMP0",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2BH (R69): the texel error's last run, shortened (10 s
        // warm-up, a round a scene; the emulator draws exactly).
        (
            "run GC2BH",
            &run_gc2bh,
            &[
                "GC2BH (R69): the texel coordinate error",
                "GC2BH reference:",
                "GC2BH A slice 10: ",
                "GC2BH as they are done, TEMP0",
                "GC2BH shifted done, TEMP0",
                "GC2BH punch-through done, TEMP0",
                "GC2BH ladder 117.00 / 1: lock",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2BG (R69): the proof, shortened (10 s warm-up, a round a
        // scene, a one-minute soak; the emulator draws exactly).
        (
            "run GC2BG",
            &run_gc2bg,
            &[
                "GC2BG (R69): the fix's proof",
                "GC2BG reference:",
                "GC2BG A slice 10: ",
                "GC2BG flat 149.50 / 1: lock",
                "GC2BG flat done, TEMP0",
                "GC2BG textured done, TEMP0",
                "GC2BG ring done, TEMP0",
                "GC2BG kinds done, TEMP0",
                "GC2BG soak 149.50 / 1: lock",
                "GC2BG soak done, TEMP0",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2BF (R69): the levers on top of the on-die lists, shortened
        // (10 s warm-up, two pairs a way; the emulator draws exactly).
        (
            "run GC2BF",
            &run_gc2bf,
            &[
                "GC2BF (R69): what the display's reads leave",
                "GC2BF reference:",
                "GC2BF the on-die renderer at Windows Mobile's clock: Some(0) words off",
                "GC2BF A slice 10: ",
                "GC2BF on-die + the display's fetch off: 0 of 2 wrong",
                "GC2BF on-die + the checker order: 0 of 2 wrong",
                "GC2BF on-die + punch-through: 0 of 2 wrong",
                "GC2BF ladder 139.75 / 1 (lock",
                "the control at Windows Mobile's clock after: Some(0) words off",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2BE (R69): the design, shortened (10 s warm-up, two pairs a
        // way; the emulator draws exactly: the half picture is the
        // reference's rows and nothing is wrong).
        (
            "run GC2BE",
            &run_gc2be,
            &[
                "GC2BE (R69): the design on scene 20's rows 0-319",
                "GC2BE reference:",
                "GC2BE half picture at Windows Mobile's clock: 0 words off the full reference's rows 0-319; B again Some(0) words off; A Some(0); the on-die renderer Some(0)",
                "GC2BE A slice 10: ",
                "GC2BE the video plane shows A at 2x",
                "GC2BE B (never read): 0 of 2 wrong",
                "GC2BE B with the lists on the die: 0 of 2 wrong",
                "GC2BE A with the lists on the die: 0 of 2 wrong",
                "GC2BE B with the video plane hidden: 0 of 2 wrong",
                "GC2BE the full scene after: 0 of 50 wrong",
                "the control at Windows Mobile's clock after: Some(0) words off",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2BD (R69): the strip, shortened (10 s warm-up, two pairs a
        // way; the emulator draws exactly: the strip is the reference's
        // rows and nothing is wrong).
        (
            "run GC2BD",
            &run_gc2bd,
            &[
                "GC2BD (R69): the core rendering where the display isn't reading",
                "GC2BD reference:",
                "GC2BD strip at Windows Mobile's clock: 0 words off the full reference's rows 224 on; drawn again Some(0) words off; at the hidden address Some(0) words off",
                "GC2BD A slice 10: ",
                "GC2BD strip hidden (0x96000): 0 of 2 wrong",
                "GC2BD strip hidden with the fetch off: 0 of 2 wrong",
                "GC2BD ring scene, the lists on the die: 0 of 2 wrong",
                "GC2BD scene 20, the lists on the die: 0 of 2 wrong",
                "GC2BD the full scene after: 0 of 50 wrong",
                "the control at Windows Mobile's clock after: Some(0) words off",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2BC (R69): the schedule's levers, shortened (10 s warm-up,
        // two pairs a way; the emulator draws exactly and its scan line
        // may not advance: waits may time out, which is logged, not
        // checked).
        (
            "run GC2BC",
            &run_gc2bc,
            &[
                "GC2BC (R69): the schedule's levers",
                "GC2BC TEMP0 at the start:",
                "GC2BC reference:",
                "GC2BC ring scene: reference true; the on-die renderer draws it true",
                "GC2BC A slice 10: ",
                "GC2BC part 2: slot 0's TSP word with bit 14 set",
                "GC2BC punch-through: 0 of 1 wrong",
                "GC2BC DMCTRL 0xd0303010 (Linux mbxfb's): 0 of 2 wrong",
                "GC2BC begun at line 610: 0 of 2 wrong",
                "GC2BC 4 bands behind the scan: 0 of 2 wrong",
                "GC2BC ring scene, both: 0 of 2 wrong",
                "GC2BC texel phases 0-11 at 149.50 / 1 warm, wrong of 5 by phase (9: no reference) [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]",
                "the control at Windows Mobile's clock after: Some(0) words off",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2BB (R69): the one run for the fix, shortened (10 s a
        // phase, two pairs a way; the emulator draws exactly, so nothing
        // is wrong and the on-die lists draw the same picture).
        (
            "run GC2BB",
            &run_gc2bb,
            &[
                "GC2BB (R69): one run for the fix",
                "GC2BB TEMP0 at the start:",
                "GC2BB reference:",
                "GC2BB lists on-die at 0xa5000 (36864 bytes): Some(0) words off",
                "GC2BB D slice 10: ",
                "GC2BB part 2, the ways",
                "GC2BB way baseline: 0 of 2 wrong",
                "GC2BB way lists on-die: 0 of 2 wrong",
                "GC2BB way all three: 0 of 2 wrong",
                "GC2BB way punch-through: 0 of 2 wrong",
                "GC2BB ladder 117.00 / 1 (lock",
                "GC2BB low overdraw (no hidden layer) at 149.50 / 1 warm: opaque 0 of 2 wrong",
                "GC2BB phase 11 reference:",
                "GC2BB texel phases 0-11 at 149.50 / 1 warm, wrong of 5 by phase (9: no reference) [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]",
                "the control at Windows Mobile's clock after: Some(0) words off",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2BA (R69): the cause test, shortened (10 s a phase; the
        // emulator draws exactly, so nothing is wrong).
        (
            "run GC2BA",
            &run_gc2ba,
            &[
                "GC2BA (R69): GC2AJ's flat corner scene 20 at 149.50 / 1, reversed, poisoned, nothing flipped, 10 s a phase in 10 slices with TEMP0",
                "GC2BA TEMP0 at the start:",
                "GC2BA reference:",
                "GC2BA A slice 10: ",
                "GC2BA B: the clock set again at once",
                "GC2BA B slice 10: ",
                "GC2BA C slice 10: 0 renders, 0 wrong",
                "GC2BA D slice 10: ",
                "the control at Windows Mobile's clock after: Some(0) words off",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2AZ (R69): the rate over time, shortened (20 s hot, 10 s
        // idle, 20 s hot, two pairs a bit; the emulator draws exactly, so
        // nothing is wrong).
        (
            "run GC2AZ",
            &run_gc2az,
            &[
                "GC2AZ (R69): GC2AJ's flat corner scene 20 at 149.50 / 1, reversed, poisoned, nothing flipped: 20 s hot in 20 slices, 10 s idle at Windows Mobile's clock, 20 s hot again; then 6 bits in 2 interleaved pairs each",
                "GC2AZ reference:",
                "GC2AZ phase 1 slice 20:",
                "GC2AZ idle: 10 s at Windows Mobile's clock",
                "GC2AZ phase 3 slice 20:",
                "GC2AZ hot phases: first 0 of",
                "GC2AZ bit 0x858 bit 9 (from 0x00000000): flipped 0 of 2 wrong, its controls 0 of 2; 0 not finished",
                "the control at Windows Mobile's clock after: Some(0) words off",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2AY (R69): the TA block search (the emulator draws exactly,
        // so every render is right and every bit is held; one render a bit
        // and one more).
        (
            "run GC2AY",
            &run_gc2ay,
            &[
                "GC2AY (R69): GC2AJ's flat corner scene 20 at 149.50 / 1, reversed, poisoned; each bit of 25 TA and parameter-manager words flipped alone, 1 renders a bit, a clean bit held 1 more",
                "GC2AY reference:",
                "GC2AY: the words as found",
                "GC2AY: unflipped, 0 of 2 renders wrong",
                "GC2AY word 24 0x87c",
                "800 bits tried",
                "the control at Windows Mobile's clock after: Some(0) words off",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2AR (R69): the register search (the emulator draws exactly,
        // so the unflipped renders are right; a flip may change its
        // picture, logged, not checked).
        (
            "run GC2AR",
            &run_gc2ar,
            &[
                "GC2AR (R69): GC2AM's phase scene 0 at 149.50 / 1, reversed, opaque; each bit of 53 render-block words",
                "GC2AR reference:",
                "GC2AR: unflipped, (words wrong, us) in three renders [(Some(0),",
                "GC2AR word 52 ",
                "bits tried",
                "the control at Windows Mobile's clock after: Some(0) words off",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2AQ (R69): the punch-through bits, one round a scene (the
        // emulator draws exactly: every way right).
        (
            "run GC2AQ",
            &run_gc2aq,
            &[
                "GC2AQ (R69): TSP bits on every object at 149.50 / 1, reversed",
                "GC2AQ none generator 149.50 / 1: controls at Windows Mobile's clock after: 1 of 1 right",
                "GC2AQ bit14 flat 149.50 / 1: controls at Windows Mobile's clock after: 23 of 23 right",
                "GC2AQ bit15 tex 149.50 / 1: controls at Windows Mobile's clock after: 22 of 22 right",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2AP (R69): the header bit search (the emulator draws
        // exactly, so the unflipped renders are right; a flip may change the
        // emulator's picture, which is logged, not checked).
        (
            "run GC2AP",
            &run_gc2ap,
            &[
                "GC2AP (R69): GC2AM's phase scene 0 at 149.50 / 1, interleaved, 134 header bits flipped alone",
                "GC2AP reference:",
                "GC2AP: unflipped, words wrong in three renders [Some(0), Some(0), Some(0)]",
                "GC2AP header word 4:",
                "bits tried",
                "the control at Windows Mobile's clock after: Some(0) words off",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2AO (R69): the proof's six plans, one round a scene (the
        // emulator draws exactly: every way right).
        (
            "run GC2AO",
            &run_gc2ao,
            &[
                "GC2AO (R69): both cures over the corner sweeps at 149.50 / 1",
                "GC2AO flat layer interleaved 149.50 / 1: controls at Windows Mobile's clock after: 22 of 22 right",
                "GC2AO tex bare interleaved 149.50 / 1: controls at Windows Mobile's clock after: 22 of 22 right",
                "GC2AO tex layer checker 149.50 / 1: controls at Windows Mobile's clock after: 22 of 22 right",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2AN (R69): the six tile orders, two rounds (the emulator
        // draws exactly: every order the same picture, every way right).
        (
            "run GC2AN",
            &run_gc2an,
            &[
                "GC2AN (R69): the untextured fault generator at 149.50 / 1 in six tile orders",
                "GC2AN: each render's target filled first with 0xf81f (in no reference)",
                "every way in its own order at Windows Mobile's clock: 6 of 6 its scene's picture",
                "wrong renders by way [0, 0, 0, 0, 0, 0]",
                "GC2AN 149.50 / 1: controls at Windows Mobile's clock after: 6 of 6 right",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2AM (R69): the phase sweep and the safe ratios, one round a
        // scene (the emulator draws exactly: every way right).
        (
            "run GC2AM",
            &run_gc2am,
            &[
                "GC2AM (R69): divide 1's texel picks",
                "GC2AM phase 149.50 / 1: controls at Windows Mobile's clock after: 31 of 31 right",
                "GC2AM 1:1 149.50 / 1: controls at Windows Mobile's clock after: 26 of 26 right",
                "GC2AM 1:2 117.00 / 1: controls at Windows Mobile's clock after: 26 of 26 right",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2AL (R69): the cures, one round a scene (the emulator draws
        // exactly: every way right).
        (
            "run GC2AL",
            &run_gc2al,
            &[
                "GC2AL (R69): the textured corner sweep",
                "GC2AL one layer 149.50 / 1: controls at Windows Mobile's clock after: 26 of 26 right",
                "GC2AL rows 149.50 / 1: controls at Windows Mobile's clock after: 26 of 26 right",
                "GC2AL two layers 149.50 / 1: controls at Windows Mobile's clock after: 26 of 26 right",
                "GC2AL blended 149.50 / 1: controls at Windows Mobile's clock after: 26 of 26 right",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2AK (R69): the ladder, one round a scene at each setting
        // (the emulator draws exactly: every way right).
        (
            "run GC2AK",
            &run_gc2ak,
            &[
                "GC2AK (R69): the corner sweep with one full layer alone",
                "GC2AK flat 149.50 / 1: controls at Windows Mobile's clock after: 23 of 23 right",
                "GC2AK uv 149.50 / 1: controls at Windows Mobile's clock after: 22 of 22 right",
                "GC2AK flat 108.33 / 1: controls at Windows Mobile's clock after: 23 of 23 right",
                "GC2AK uv 149.50 / 2: controls at Windows Mobile's clock after: 22 of 22 right",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2AJ (R69): the corner sweep, one round (the emulator draws
        // exactly: every way right).
        (
            "run GC2AJ",
            &run_gc2aj,
            &[
                "GC2AJ (R69): the corner sweep with one full layer alone",
                "GC2AJ flat: each render's target filled first with 0xf81f (in no reference)",
                "GC2AJ flat 149.50 / 1: controls at Windows Mobile's clock after: 23 of 23 right",
                "GC2AJ textured 149.50 / 1: controls at Windows Mobile's clock after: 22 of 22 right",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2AI (R69): the quad ways, two rounds (the emulator draws
        // exactly: every way right).
        (
            "run GC2AI",
            &run_gc2ai,
            &[
                "GC2AI (R69): the flat quad over the ring and one full layer",
                "GC2AI: each render's target filled first with 0xf81f (in no reference)",
                "wrong renders by way [0, 0, 0, 0, 0, 0, 0, 0]",
                "GC2AI 149.50 / 1: controls at Windows Mobile's clock after: 8 of 8 right",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2AH (R69): the geometry ways, two rounds (the emulator draws
        // exactly: every way right).
        (
            "run GC2AH",
            &run_gc2ah,
            &[
                "GC2AH (R69): the geometry at 149.50 / 1, no texture",
                "GC2AH: each render's target filled first with 0xf81f (in no reference)",
                "wrong renders by way [0, 0, 0, 0, 0, 0]",
                "GC2AH 149.50 / 1: controls at Windows Mobile's clock after: 6 of 6 right",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2AG (R69): the fault generator with the 3D core's own
        // traffic changed one kind at a time, two renders a way (the
        // emulator draws exactly: every way right, the texture's copy and
        // the moved parameter memory sound).
        (
            "run GC2AG",
            &run_gc2ag,
            &[
                "GC2AG (R69): FT2 with the ring and one full layer, reversed, 149.50 / 1",
                "copied to 0x",
                ": 0 words read back wrong",
                "GC2AG as GC2AE 149.50 / 1: controls at Windows Mobile's clock after: 1 of 1 right",
                "GC2AG texture inside 149.50 / 1: controls at Windows Mobile's clock after: 1 of 1 right",
                "GC2AG untextured 149.50 / 1: controls at Windows Mobile's clock after: 1 of 1 right",
                "GC2AG lists moved 149.50 / 1: controls at Windows Mobile's clock after: 1 of 1 right",
                "GC2AG both moved 149.50 / 1: controls at Windows Mobile's clock after: 1 of 1 right",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2AF (R69): the fault generator with the display's fetch, the
        // CPU's status reads or both off, at 74.75 / 1 and 149.50 / 2, two
        // renders a way (every way right in the emulator; the fetch-off
        // ways' controls right: the underrun flag settled first).
        (
            "run GC2AF",
            &run_gc2af,
            &[
                "GC2AF (R69): FT2 with the ring and one full layer, reversed",
                "GC2AF as GC2AE 149.50 / 1: controls at Windows Mobile's clock after: 1 of 1 right",
                "GC2AF fetch off 149.50 / 1: controls at Windows Mobile's clock after: 1 of 1 right",
                "GC2AF CPU quiet 149.50 / 1: controls at Windows Mobile's clock after: 1 of 1 right",
                "GC2AF both off 149.50 / 1: controls at Windows Mobile's clock after: 1 of 1 right",
                "GC2AF Dell's 74.75 / 1: controls at Windows Mobile's clock after: 1 of 1 right",
                "GC2AF divide 2 149.50 / 2: controls at Windows Mobile's clock after: 1 of 1 right",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2AE (R69): the trace, the conditions and the margin, two
        // renders a part (the emulator draws exactly: every way right, the
        // display's fetch put back on).
        (
            "run GC2AE",
            &run_gc2ae,
            &[
                "GC2AE (R69): FT2 with the ring and one full layer",
                "GC2AE trace: each render's target filled first with 0xf81f (in no reference)",
                "GC2AE trace 149.50 / 1: controls at Windows Mobile's clock after: 1 of 1 right",
                "GC2AE rows 149.50 / 1: controls at Windows Mobile's clock after: 1 of 1 right",
                "GC2AE fetch off 149.50 / 1: controls at Windows Mobile's clock after: 1 of 1 right",
                "GC2AE clock 146.25 / 1: controls at Windows Mobile's clock after: 1 of 1 right",
                "GC2AE clock 133.25 / 1: controls at Windows Mobile's clock after: 1 of 1 right",
                "GC2AE margin 146.25 / 1: controls at Windows Mobile's clock after: 7 of 7 right",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2AD (R36): the night soak, cut to 20 s (the emulator draws
        // exactly: every way right).
        (
            "run GC2AD",
            &run_gc2ad,
            &[
                "GC2AD (R36): at 149.50 / 1, reversed, 0 min",
                // Every render's target poisoned first (a pixel the core never
                // writes then reads wrong); the emulator overwrites it all.
                "GC2AD soak: each render's target filled first with 0xf81f (in no reference)",
                "wrong renders by way [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]",
                "GC2AD soak 149.50 / 1: controls at Windows Mobile's clock after: 18 of 18 right",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2AC: the ring cure against the full hidden layers at
        // 149.50 / 1 (the emulator draws exactly: every way right).
        (
            "run GC2AC",
            &run_gc2ac,
            &[
                "GC2AC 149.50 / 1: 500 rounds (3500 renders)",
                "wrong renders by way [0, 0, 0, 0, 0, 0, 0]",
                "GC2AC 149.50 / 1: controls at Windows Mobile's clock after: 7 of 7 right",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2AB: the hidden layers against the last-pixel fault at
        // 152.75 / 1 (the emulator draws exactly: no fallback).
        (
            "run GC2AB",
            &run_gc2ab,
            &[
                "GC2AB speed 99.67 / 2 (Windows Mobile's): 20 rounds (160 renders)",
                "GC2AB speed 149.50 / 1: 20 rounds (160 renders)",
                "GC2AB 152.75 / 1: 800 rounds (6400 renders)",
                "GC2AB 152.75 / 1: controls at Windows Mobile's clock after: 8 of 8 right",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2AA: the free corner cures, then the winner soaked (the
        // emulator draws exactly: every way right, the first preferred at
        // 149.50, a 20 s soak).
        (
            "run GC2AA",
            &run_gc2aa,
            &[
                "GC2AA 139.75 / 1: 1000 rounds (4000 renders)",
                "GC2AA 149.50 / 1: right in every render: one hidden layer, columns reversed",
                "GC2AA soak: one hidden layer, columns reversed at 149.50 / 1 over every 16-bit kind and GC2K's worst, 0 min",
                "GC2AA soak 149.50 / 1: controls at Windows Mobile's clock after: 7 of 7 right",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2Z: the narrower texture shifts and the stronger corner
        // cures: every way's references held, the run returns.
        (
            "run GC2Z",
            &run_gc2z,
            &[
                "GC2Z 1 149.50 / 1: 30 rounds (360 renders)",
                "GC2Z 2 139.75 / 1: 1000 rounds (4000 renders)",
                "GC2Z 2 139.75 / 1: controls at Windows Mobile's clock after: 4 of 4 right",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2Y: both cures over the climb, then the soak at the highest
        // clean clock (the emulator draws exactly: 149.50, a 20 s soak).
        (
            "run GC2Y",
            &run_gc2y,
            &[
                "GC2Y climb 133.25 / 1: the cured seven all right",
                "GC2Y climb 149.50 / 1: the cured seven all right",
                "GC2Y soak: the cured seven at 149.50 / 1 (the highest clock with all right) for 0 min",
                "GC2Y soak 149.50 / 1: controls at Windows Mobile's clock after: 7 of 7 right",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2V: FT2's quad edge isolated at 146.25 / 1, the hidden
        // layer at 133.25 / 1: every way's references held, the run returns
        // (wm-clocks build).
        (
            "run GC2V",
            &run_gc2v,
            &[
                "GC2V A 143.00 / 1: 10 rounds (90 renders)",
                "GC2V A 146.25 / 1: 10 rounds (90 renders)",
                "GC2V B 133.25 / 1: 200 rounds (600 renders)",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2U: FT2 and GC2K's worst with a 32-bit, a 16-bit and a PVRTC
        // texture and with busy tiles, at every climb step: every way's
        // references held and the run returns (wm-clocks build).
        (
            "run GC2U",
            &run_gc2u,
            &[
                "GC2U: divide 1 at 149.50 / 1, PVRTC texture",
                "PVRTC texture 149.50 / 1: 30 rounds (60 renders)",
                "32-bit texture 133.25 / 1: 100 rounds (400 renders)",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2T: the local memory's timing loosened on FT3, GC2K's worst
        // and the climb: every timing written, held and put back, and the
        // run returns (stepped on the wm-clocks build).
        (
            "run GC2T",
            &run_gc2t,
            &[
                "GC2T 1 133.25 / 1 all +1 (LMTIM 0xa55d3, reads 0xa55d3)",
                "GC2T 2: GC2K's worst at 133.25 / 1, all +1 (LMTIM Some(a55d3))",
                "GC2T 3: divide 1 at 149.50 / 1, all +1",
                "GC2Q parts A and B: done",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2D (idea (d), Gouraud as a texture, by GC2's rules, one
        // render a kind a setting): checked below.
        (
            "run GC2D",
            &run_gc2d,
            &[
                "GC2: idea (d), Gouraud drawn as a bilinear texture",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Runs GC2F (idea (f): eight render tables) and GC2H (past 133
        // MHz), one render a kind a setting: checked below.
        (
            "run GC2F",
            &run_gc2f,
            &[
                "GC2: idea (f), Gouraud under eight render tables",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        (
            "run GC2H",
            &run_gc2h,
            &["GC2: run GC past 133 MHz", "RETROPANE-GPUBENCH: done"],
            false,
            (0, 0, 0),
        ),
        // Run GC2R (divide 1's registers and ways in), one render a way:
        // checked below.
        (
            "run GC2R",
            &run_gc2r,
            &["GC2R: what divide 1 changes", "RETROPANE-GPUBENCH: done"],
            false,
            (0, 0, 0),
        ),
        // Run GC2H2 (GC2H to 169 MHz): checked below.
        (
            "run GC2H2",
            &run_gc2h2,
            &["GC2: run GC to 169 MHz", "RETROPANE-GPUBENCH: done"],
            false,
            (0, 0, 0),
        ),
        // Run GC2W (the write-out downsample at divide 1): checked below.
        (
            "run GC2W",
            &run_gc2w,
            &[
                "GC2: the write-out downsample at divide 1",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC3 (the memory interface and the display's refresh window,
        // run FT3 under each): checked below.
        (
            "run GC3",
            &run_gc3,
            &["GC3 FT3 under ", "RETROPANE-GPUBENCH: done"],
            false,
            (0, 0, 0),
        ),
        // Run FT3 (2D and 3D at once in option C's layout, three clocks):
        // checked below.
        (
            "run FT3",
            &run_ft3,
            &[
                "FT3: the 2D engine and the 3D core at once",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2M (divide 1 with M24CLK as Dell's driver sets it):
        // checked below.
        (
            "run GC2M",
            &run_gc2m,
            &["GC2: divide 1 with M24CLK", "RETROPANE-GPUBENCH: done"],
            false,
            (0, 0, 0),
        ),
        // Run GC2X (MBXCLK's reserved bits at divide 1): checked below.
        (
            "run GC2X",
            &run_gc2x,
            &[
                "GC2X: every undocumented bit near the cores",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GV (the local memory's clock, V1): checked below.
        (
            "run GV",
            &run_gv,
            &[
                "GV: V1, the 2700G's local memory clock",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run R (the dark display's restart steps): every step's registers
        // and cards, and back at the loader.
        (
            "run R",
            &run_r,
            &[
                "R: as found, before anything is touched",
                "R0: adopted",
                "R1: the sync generator",
                "R2: the display PLL",
                "R3: the whole display set-up",
                "R4: the system clock",
                "R: after R4",
                "RETROPANE-GPUBENCH: done (run R)",
            ],
            false,
            (0, 0, 0),
        ),
        // Run S (a read-only register snapshot): every line, back at the
        // loader.
        (
            "run S",
            &run_s,
            &[
                "S: every named register below the MBX block",
                " SYSCFG 0x",
                " CSC05 0x",
                "the picture at offset 0, mid-screen",
                "RETROPANE-GPUBENCH: done (run S)",
            ],
            false,
            (0, 0, 0),
        ),
        // Run U (the underrun reproduced and cleared): every hold logged
        // (the emulator has no underrun), back at the loader.
        (
            "run U",
            &run_u,
            &[
                "U: as found",
                "U1: 1 ms on the reference clock",
                "U4: 30 ms on the reference clock",
                "U5: CL3 at 99.67 MHz for 100 ms",
                "U6: G4's lock traces",
                "U: at the end",
                "RETROPANE-GPUBENCH: done (run U)",
            ],
            false,
            (0, 0, 0),
        ),
        // Run V (which work at which clock starves the display): every
        // clock's line (no underrun in the emulator), put back.
        (
            "run V",
            &run_v,
            &["RETROPANE-GPUBENCH: done"],
            false,
            (0, 0, 0),
        ),
        // Run X (the underrun flag cleared, nothing else): its lines.
        (
            "run X",
            &run_x,
            &[
                "X: as found",
                "X: the flag cleared",
                "X: after the card",
                "X1: the sync generator off and on",
                "X3: the whole display set-up",
                "RETROPANE-GPUBENCH: done (run X)",
            ],
            false,
            (0, 0, 0),
        ),
        // Run L (the recovery ladder): the emulator's DSIG reads 0, so
        // every rung runs and none recovers it.
        (
            "run L",
            &run_l,
            &[
                "L: as found",
                "L1: the pixel clock off and on",
                "L4: the local memory to self-refresh and back",
                "L6: the memory clock off for 1 ms",
                "RETROPANE-GPUBENCH: done (run L)",
            ],
            false,
            (0, 0, 0),
        ),
        // Run Z (the reset and the rebuild): every phase and both dumps.
        (
            "run Z",
            &run_z,
            &[
                "Z: as found",
                "Z1: SYSRST next",
                "Z1b: SYSCFG 0xffffff0c",
                "Z2: the clocks next",
                "Z3: the clocks and memory read back",
                "Z5: the display",
                "Z7: every register after the rebuild",
                "RETROPANE-GPUBENCH: done (run Z)",
            ],
            false,
            (0, 0, 0),
        ),
        // Run H (the card log against a frozen bus): the emulator answers
        // the read, so the run comes back; every line is on the card
        // (checked for every run).
        (
            "run H",
            &run_h,
            &[
                "H: line 1 of 40 before the freeze",
                "H: line 40 of 40 before the freeze",
                "H: reading 0x12c next",
                "H: the read returned",
                "H: the watchdog left unarmed",
            ],
            false,
            (0, 0, 0),
        ),
        // Run Y (the rescue measured): checked below (refused cleanly
        // on a fork that doesn't start as Windows Mobile leaves the clocks).
        (
            "run Y",
            &run_y,
            &["Y: the rescue measured", "RETROPANE-GPUBENCH: done"],
            false,
            (0, 0, 0),
        ),
        // Run Q (what ends the slow state): refused cleanly on a fork that
        // doesn't start the clocks as Windows Mobile leaves them, else every
        // step measured.
        (
            "run Q",
            &run_q,
            &["RETROPANE-GPUBENCH: run Q", "RETROPANE-GPUBENCH: done"],
            false,
            (0, 0, 0),
        ),
        // Run N (the start-up watched): every stage read, the three cards.
        (
            "run N",
            &run_n,
            &[
                "N0 the 2700G handed out: DINTRS",
                "N6 the screen buffer shown (black)",
                "N7 card 1",
                "N8 card 2",
                "N11 card 3",
                "N15 after the lock trace 108.33 -> 99.67 MHz",
                "N: done",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run P (the 3D core's target formats): every code tried and
        // decoded. The fork writes 32-bit pixels for every code but 1 and
        // 3 (the others aren't measured yet: emulator.md), so all 52 read
        // back in place and none hangs; the device's answer is the run's.
        (
            "run P",
            &run_p,
            &[
                "P3: render targets",
                "  P3 try 2 0x650 0x00000001",
                "    565 rows (the picture's stride): 1024/1024 pixels in place",
                "    1555 rows (the picture's stride): 1024/1024 pixels in place",
                "    8888 rows (the picture's stride): 1024/1024 pixels in place",
                "  P3 try 52 0x650 0x80000006",
                "P3: 52 tries, 52 decoded (9 in 10 pixels in place at least), 0 hung, 0 wrote nothing",
                "everything given back: true",
                "RETROPANE-GPUBENCH: done",
            ],
            true,
            (0, 0, 0),
        ),
        // Run P2 (run P's follow-up): every part runs. The fork writes
        // codes 1 and 3 as RGB565 and ARGB1555 and the rest as ARGB8888,
        // undithered (emulator.md), so both shapes decode in every code
        // and RGB565's ramp truncates; the device's answers are the run's.
        (
            "run P2",
            &run_p2,
            &[
                "P2a: target shapes",
                "  P2a 16 x 16: 8 of 8 decoded",
                "  P2a 32 x 16: 8 of 8 decoded",
                "  P2b code 1",
                "    red (5 bits): 0 rounded up, 0 other, 0 of 64 places not a threshold",
                "  P2b code 15",
                "  P2c code 6: as drawn",
                "  P2d: 192 tries, 0 hung, 0 not plain ARGB8888 rows",
                "P2: 232 tries, 0 hung",
                "everything given back: true",
                "RETROPANE-GPUBENCH: done",
            ],
            true,
            (0, 0, 0),
        ),
        // Run P2W (watched): its four pictures, each to its 30 s cap (no
        // presses), back at the loader.
        (
            "run P2W",
            &run_p2w,
            &[
                "  P2W step 1: 24-BIT (0x650 4",
                "    the black bar after the render: 0 of 19200 pixels not black",
                "    planes: GSCTRL 0x",
                "  P2W step 4: 32-BIT (0x650 6",
                "P2W: done",
                "everything given back: true",
                "RETROPANE-GPUBENCH: done",
            ],
            true,
            (0, 0, 0),
        ),
        // Run W (the fingerprint): every register line; the release fork
        // doesn't start the clocks as Windows Mobile leaves them, so it
        // says so and measures nothing (the wm-clocks build measures).
        (
            "run W",
            &run_w,
            &[
                "W PXA CLOCKS: CCCR 0x",
                "W PXA MEMORY",
                "W CPU: ID 0x69054",
                "W: every named 2700G register",
                " SYSCFG 0x",
                "everything given back: true",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run T (the display's fetch, 0x644's mapping): both parts run (the
        // release fork measures nothing at its own clock), every row of
        // the three mappings logged.
        (
            "run T",
            &run_t,
            &[
                "T1: the display's fetch",
                "T2: 0x644 bits 0 and 1",
                "  T2 0x644 = 1",
                "  T2 0x644 = 3",
                "    row 31: changed 0x",
                "everything given back: true",
                "RETROPANE-GPUBENCH: done",
            ],
            true,
            (0, 0, 0),
        ),
        // Run T3 (the downsample's cost): every setting timed (the fork's
        // times say nothing about the device's).
        (
            "run T3",
            &run_t3,
            &[
                "  T3 0x644 = 0: ",
                "  T3 0x644 = 3: ",
                "T3: plain ",
                "everything given back: true",
                "RETROPANE-GPUBENCH: done",
            ],
            true,
            (0, 0, 0),
        ),
        // Run P2X (the specks): every render checked; the fork renders
        // the 32-bit and RGB565 ones right and every render alike (its
        // 24-bit is written as 32-bit: emulator.md, so only its line).
        (
            "run P2X",
            &run_p2x,
            &[
                "  P2X code 6 render 1 (true): bars: 0 white, 0 never written, 0 other wrong; 0 against render 1",
                "  P2X code 6 render 5 (true): bars: 0 white, 0 never written, 0 other wrong; 0 against render 1",
                "  P2X code 4 render 5 ",
                "  P2X code 1 render 5 (true): bars: 0 white, 0 never written, 0 other wrong; 0 against render 1",
                "P2X: done",
                "everything given back: true",
                "RETROPANE-GPUBENCH: done",
            ],
            true,
            (0, 0, 0),
        ),
        // Run P2Y (what the errors need): every part; the fork reads and
        // renders without errors.
        (
            "run P2Y",
            &run_p2y,
            &[
                "  Y1 pass 1: 0 of 307200 words read differently twice in a row",
                "  Y1 pass 2: 0 of 307200 words read differently twice in a row",
                "  Y2 the screen at 32 bits:",
                "  Y2 the screen stopped:",
                "  P2X code 6 render 5 (true): bars: 0 white, 0 never written, 0 other wrong; 0 against render 1",
                "P2Y: done",
                "everything given back: true",
                "RETROPANE-GPUBENCH: done",
            ],
            true,
            (0, 0, 0),
        ),
        (
            // The model draws every render right, so the controls find
            // nothing here either: the check is that every render runs,
            // is checked and counted, and the run comes back.
            "run P2L",
            &run_p2l,
            &[
                "  P2L first render (true), the screen at 32 bits: bars: 0 white, 0 never written, 0 other wrong",
                "  P2L control before (the screen at 32 bits): 5 renders (0 unfinished)",
                "  the screen: P2W's picture dithered to RGB565 (true), scanned out at 16 bits",
                "  P2L so far, the screen at 16 bits: 500 renders (0 unfinished)",
                "  P2L the screen at 16 bits: 1000 renders (0 unfinished)",
                "  the reference: changed at 0 of 40 checks",
                "  P2L control after (the screen at 32 bits): 5 renders (0 unfinished)",
                "P2L: 0 of 1000 renders under the 16-bit screen had a wrong pixel (0 pixels); the controls at 32 bits: 0 of 5 before, 0 of 5 after",
                "P2L: done",
                "everything given back: true",
                "RETROPANE-GPUBENCH: done",
            ],
            true,
            (0, 0, 0),
        ),
        (
            // FF1 needs only every fill taken (whether the model's FIFO
            // ever reads full is the device's question); FF2's drop is
            // the renderer's, on any engine.
            "run FF",
            &run_ff,
            &[
                "  FF1: 300 fills queued in ",
                "0 refused (FIFO full 100 ms)",
                "  ok  B dropped: counted once, apart from stalls",
                "  ok  the dropped frame isn't shown (A still on screen)",
                "  ok  B after the drop is B (within 2 a channel, the host test's bound)",
                "FF: FF2 11 checks, 0 BAD",
                "FF: done",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        (
            // C43's real stall: what the engine does with its clock off is
            // the device's question (the fork may not stop it, and its FIFO
            // never fills: FS3 then doesn't judge the drop); every step must
            // run, put the clock back and pass what it does judge.
            "run FS",
            &run_fs,
            &[
                "FS1: 20 full-screen fills",
                "  ok  FS1: the queued work finished once the clock was back",
                "FS2: not repeated",
                "  ok  FS3: the engine caught up (the fence landed)",
                "  ok  FS3: A still on screen after the stall",
                "  ok  FS3: B after the stall is B drawn after A (the screens' hashes)",
                " checks, 0 BAD; MBXCLK at the end ",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        (
            // The gate's P8: the engines at once; both results as alone.
            "run FT",
            &run_ft,
            &[
                "  FT alone: 3D Some(",
                "0 words of 153600 differ from the first",
                "FT together: 0 words of 153600 differ from the render alone",
                "differing pixels 0 in the textured quad, 0 in the Gouraud background only",
                "the render as alone: true; the copies as alone: true",
                "FT: both right together",
                "RETROPANE-GPUBENCH: done",
            ],
            true,
            (0, 0, 0),
        ),
        (
            // The gate's P18: the MBX status bits around 2D and 3D jobs,
            // the enables put back.
            "run FI",
            &run_fi,
            &[
                "  FI as found: status 0x",
                "  FI enables 0 (0x00000000): idle 0x",
                "  FI enables all (0xffffffff): idle 0x",
                "3D started true",
                "FI: enables put back",
                "RETROPANE-GPUBENCH: done",
            ],
            true,
            (0, 0, 0),
        ),
        (
            // The gate's T3: 2D commands into the command window by DMA.
            "run FD",
            &run_fd,
            &[
                "  FD 0 CPU: idle first true, sent true",
                "  FD 2 DMA: idle first true",
                "the picture as the CPU's: true",
                "FD: both ways drew the same",
                "RETROPANE-GPUBENCH: done",
            ],
            true,
            (0, 0, 0),
        ),
        // The backlog's P12 and V17: every burst and width moves the copy
        // right (the emulator's DMA ignores the burst), every phase read.
        (
            "run BX",
            &run_bx,
            &[
                "  BX P12 burst 8 width 0: 960 KiB in",
                "  BX P12 burst 32 width word: 960 KiB in",
                "the copy right: true",
                "  BX V17 a DMA into local memory each frame:",
                "BX: done",
                "everything given back: true",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // The status bits run FI left (P18): every phase is read and the
        // memory given back; what the bits do is the device's.
        (
            "run FI2",
            &run_fi2,
            &[
                "FI2: MINTSTAT, SINTSTAT",
                "  FI2 idle 20 ms:",
                "  FI2 a full 2D FIFO draining:",
                "  FI2 2D: 420 words sent true, done true",
                "  FI2 a 3D render:",
                "FI2: done",
                "everything given back: true",
                "RETROPANE-GPUBENCH: done",
            ],
            true,
            (0, 0, 0),
        ),
        // Run CH's C71 diagnostic: six W5 sequences, each held to a fresh
        // render (the emulator's are exact: the device's are the finding).
        (
            "run C71",
            &run_c71,
            &[
                "  CH C71 the background object run 3:",
                "  CH C71 a flat quad in slot 0 run 3:",
                "  CH C71 the same scene rendered whole 20 times:",
                "  CH C71 the same picture read 20 times, no render between: 0 reads differ",
                "  CH C71 textured cards shrunk 2:1 (64 x 64), point sampling: rendered whole 20 times",
                "  CH C71 flat cards 200 x 260, the display's fetch paused: rendered whole 20 times",
                "  CH C71 flat cards 200 x 260, the display reading offset 0: rendered whole 20 times",
                "  CH C71: done",
                "everything given back: true",
                "RETROPANE-GPUBENCH: done",
            ],
            true,
            (0, 0, 0),
        ),
        // Run G21 on C71's scenes: the safe screen, A against C.
        ("run G21", &run_g21, &g21_wants, true, (0, 0, 0)),
        // R99: C71 under R-B, W5's partial frames into the shown screen at
        // 0x0, each held to a fresh whole render: the fork draws them
        // alike, so every frame matches.
        (
            "run C71 R-B",
            &run_c71_rb,
            &[
                "  CH C71 R-B: W5's partial frames into the shown screen at 0x0",
                "  CH C71 R-B the background object: 60 partial frames, 0 differ from their fresh whole render (0 words in rows 0-559, 0 with rows 560-639 off), 0 renders unfinished",
                "  CH C71 R-B a flat quad in slot 0: 60 partial frames, 0 differ from their fresh whole render (0 words in rows 0-559, 0 with rows 560-639 off), 0 renders unfinished",
                "  CH C71 R-B: 0 frames wrong or unfinished of 120",
                "  CH C71 R-B: done",
                "RETROPANE-GPUBENCH: done",
            ],
            true,
            (0, 0, 0),
        ),
        // The 3D core's limits (P14): every measurement finishes and the
        // memory is given back; the times are the device's.
        (
            "run LIM",
            &run_lim,
            &[
                "LIM: the 3D core's limits",
                "  LIM 63 textured 64 x 64 quads:",
                "  LIM 63 translucent full-screen layers:",
                "  LIM 32 quads, a texture each:",
                "  LIM a 1024 x 1024 texture over the screen",
                "LIM: done",
                "everything given back: true",
                "RETROPANE-GPUBENCH: done",
            ],
            true,
            (0, 0, 0),
        ),
        // The compositor harness (docs/design/compositor-harness.md): every
        // sequence runs, both 3D ways end on one picture, and everything is
        // given back; the emulator's times mean nothing (emulator.md).
        (
            "run CH",
            &run_ch,
            &[
                "CH: the compositor harness",
                "  CH W1 pan (VSCADR a frame): 160 frames",
                "  CH W2 row by 2D BY DMA: 7 rows (7 done)",
                "  CH W2 row by DMA PIXELS: 7 rows (7 done)",
                "  CH W4 tap by 2D BY DMA:",
                "  CH plane steps done: 10 of 18",
                "  CH W5 turns and pipeline end on the same picture: true",
                "  CH W6 turns and pipeline end on the same picture: true",
                "  CH 3D steps done: 18 of 18",
                "  CH W5 turns, every tile every frame: the last picture against a fresh full render",
                "CH: done",
                "everything given back: true",
                "RETROPANE-GPUBENCH: done",
            ],
            true,
            (0, 0, 0),
        ),
        // Run WG: the emulator doesn't model DSIG (reads 0), so every
        // refresh is "not judged"; what it checks is the wiring: every step
        // runs, the L2 layouts through the scan-out model (a clear window is
        // screen 0 alone), the 2D engine's ARGB pictures word for word, M7's
        // settings written and put back, everything given back.
        // Run WG-R (R134): Reveal written ways A-G and H at ARGB4444; the
        // emulator checks each way runs and DSCTRL comes back as found.
        (
            "run WG-R",
            &run_wgr,
            &[
                "  WG-R (R134, sheet r134-reveal-tears.md)",
                "  WG A REVEAL AS RUN WG: 19 states",
                "  WG B REVEAL, VSADR LAST: 19 states",
                "  WG C REVEAL, VSADR FIRST: 19 states",
                "  WG D REVEAL, LINES 640-642: 19 states",
                "  WG E THE TOP ALONE MOVING: 19 states",
                "  WG F THE HEIGHT ALONE MOVING: 19 states",
                "  WG G1 HELD, BIT 24 CLEARED: 19 states",
                "  WG G2 HELD, DUCTRL 1 TWICE: 19 states",
                "  WG G3 HELD, DUCTRL 0 THEN 1: 19 states",
                "  WG H REVEAL AT ARGB4444: 19 states",
                "  WG H TURN AT ARGB4444, 1-PIXEL STEPS (8-PIXEL STARTS, THE WINDOW 0-7): 64 states",
                "WG: done",
                "everything given back: true",
                "RETROPANE-GPUBENCH: done",
            ],
            true,
            (0, 0, 0),
        ),
        (
            "run WG",
            &run_wg,
            &[
                "WG: the Weather movement on the planes",
                "  WG M1 ARGB8888 per-pixel alpha on the video plane: the clear lines read as screen 0 alone not judged, the page differs from screen 0 not judged, the page differs from its alpha ignored not judged; the model: true true true",
                "  WG M1 ARGB4444 per-pixel alpha on the video plane: the clear lines read as screen 0 alone not judged, the page differs from screen 0 not judged, the page differs from its alpha ignored not judged; the model: true true true",
                "  WG M2 fling and rebound (VSCADR, ARGB8888): ",
                "  WG M2 Reveal (VSADR, VSCTRL, SSSIZE and VSCADR a frame): 19 states",
                "  WG M2 Cascade (VSCTRL, SSSIZE and VSCADR a frame): 19 states",
                "  WG M3 turn, 4-pixel steps (VSCADR, stride 3840): 121 states",
                "  WG M3 turn, 1-pixel steps (VSCADR, VSADR x and VSCTRL width): 64 states",
                "  WG M5 an alpha-0 fill 480 x 462 (32 bits): ",
                "done true; 0 of 221760 words wrong",
                "  WG M5 ten rows' cards 464 x 41 copied (32 bits): ",
                "done true; 0 of 216000 words wrong",
                "  WG M5 24 radar cells 30 x 32 copied (32 bits): ",
                "done true; 0 of 23040 words wrong",
                "  WG M5 the neighbour page 480 x 462 into the strip (32 bits): ",
                "  WG M5 screen 0 copied 480 x 640 (16 bits): ",
                "done true; 0 of 153600 words wrong",
                "  WG M5 the hand-back (the page over the wallpaper, 32 bits, converted into the screen): ",
                "done true; 0 of 110880 words differ from the CPU's \"over\"",
                "  WG M5 an ARGB4444 row card 480 x 41 copied (16 bits): ",
                "done true; 0 of 9840 words wrong",
                "  WG M4 the scan-out model's mix against the 2D engine's compose: ",
                "  WG M4 composed -> layer: 50 hand-offs: 0 in the refresh due, 0 a refresh late, 0 not in three, 0 with a refresh of NEITHER (not judged)",
                "  WG M4 layer -> composed: 50 hand-offs: 0 in the refresh due, 0 a refresh late, 0 not in three, 0 with a refresh of NEITHER (not judged)",
                "  WG M2 fling and rebound (VSCADR, ARGB8888): 256 states",
                "  WG watched block: ",
                "  WG M7 step 1 of 3, MBXCLK clocks off (bits 1-0 = 0, the divider kept): about to write",
                "  WG M7 step 2 of 3, MBXCLK 0 (Dell's off path): about to write",
                "  WG M7 step 3 of 3, ODFBPWR 1 (the on-die buffer active low power): about to write",
                "WG: done",
                "everything given back: true",
                "RETROPANE-GPUBENCH: done",
            ],
            true,
            (0, 0, 0),
        ),
    ];
    // One card for every run, with someone else's file on it: each run
    // keeps its log there as it goes (the loader's card log), the next number each
    // time, and touches nothing outside rplog/ (the card is shared).
    let other = b"another session's file: must stay as it is".to_vec();
    let card = rp_fat::format::build(&rp_fat::format::Image {
        total_sectors: 64 * 1024,
        sectors_per_cluster: 8,
        files: &[rp_fat::format::Entry {
            name: "other.txt",
            data: &other,
        }],
        blocked_clusters: &[],
        ballast: 0,
    });
    let sd = scratch(root, "sd-gpubench.img")?;
    std::fs::write(&sd, &card).map_err(|e| format!("{}: {e}", sd.display()))?;
    let mut number = 0u32;
    for (name, bin, want, needs_model, (first, every, presses)) in runs {
        if only.is_some_and(|o| !o.iter().any(|n| n == name)) {
            continue;
        }
        number += 1;
        variant_built(root, bin)?;
        let payload = std::fs::read(bin).map_err(|e| format!("{}: {e}", bin.display()))?;
        // Named, so each run's card log is rplog/gpubench-NNNN.txt.
        let mut input = rp_loadproto::control_frame("name gpubench");
        input.extend(rp_loadproto::frame(&payload));
        let keys = || -> Result<(), String> {
            if presses == 0 {
                return Ok(());
            }
            let mut mon = crate::screen::Monitor::connect(monitor_port())?;
            guest_hold(&mut mon, first, "gpubench: before the first key")?;
            for k in 0..presses {
                if k > 0 {
                    guest_hold(&mut mon, every, "gpubench: between keys")?;
                }
                // Run C's W4 is up before the first key: VSCADR to the
                // yellow half at line 320 and back at 560 every frame.
                if name == "run C" && k == 0 && repoint {
                    gpubench_until(&mut mon, &guest, &shot, 40, "w4", &|img| {
                        gpubench_expect(img, "W4 above the band", 240, 200, (0, 0, 1))?;
                        gpubench_expect(img, "W4 band", 240, 440, (1, 1, 0))?;
                        gpubench_expect(img, "W4 below the band", 240, 600, (0, 0, 1))
                    })?;
                }
                // Run D's code 7 is up before the eighth key: the RGB565
                // card on the video plane.
                if name == "run D" && k == 7 && video_rgb {
                    let bars = [
                        (1, 1, 1),
                        (1, 1, 0),
                        (0, 1, 1),
                        (0, 1, 0),
                        (1, 0, 1),
                        (1, 0, 0),
                        (0, 0, 1),
                        (0, 0, 0),
                    ];
                    gpubench_until(&mut mon, &guest, &shot, 12, "code7", &|img| {
                        for (i, want) in bars.into_iter().enumerate() {
                            gpubench_expect(img, "code 7 bar", 30 + 60 * i as u32, 200, want)?;
                        }
                        gpubench_expect(img, "code 7 ramp's left", 4, 400, (0, 0, 0))?;
                        gpubench_expect(img, "code 7 ramp's right", 476, 400, (1, 1, 1))
                    })?;
                }
                mon.query("sendkey ret 200")?;
            }
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
                // The loader's start of the image: the steps below run
                // while it is on screen (its own marker comes at its end).
                input: Some((&input, "RPL OK sha256")),
                during: Some(&keys),
                then: Some("RPL END"),
                timeout: Duration::from_secs(900),
            },
        )?;
        let log = output
            .rsplit_once("RPL LOG")
            .map(|(_, l)| l)
            .ok_or_else(|| format!("gpubench {name}: never came back to the loader"))?;
        if crashed(log) || log.contains("did not return") {
            return Err(format!("gpubench {name}: didn't return cleanly:{log}"));
        }
        gpubench_card_log(&sd, name, number, log)?;
        // Run R returns before any 2700G memory is taken: no pool line.
        let always: &[&str] = if matches!(
            name,
            "run R" | "run S" | "run U" | "run X" | "run L" | "run Z"
        ) {
            &["RETROPANE-GPUBENCH: done"]
        } else {
            &["everything given back: true", "RETROPANE-GPUBENCH: done"]
        };
        // A run that steps the clocks from Windows Mobile's refuses on a
        // fork that starts them at 0 (the release; R186: the R69 runs from
        // GC2AX on were written against the unreleased wm-clocks build and
        // failed on rp-v0.31.0 for want of lines a refusal never prints).
        // The X51v always starts at Windows Mobile's COREPLL, so this can
        // only be the emulator: the plumbing is checked, as for runs G, Y,
        // Q and W one by one.
        let refused_clocks = gpubench_refused_clocks(log);
        if refused_clocks {
            eprintln!(
                "    gpubench {name}: refused cleanly (this fork doesn't start the clocks as Windows Mobile leaves them)"
            );
        }
        // Without the 3D model every render is refused (`NoCore`): only
        // the plumbing is checked for runs A and B.
        let checked: &[&str] = if refused_clocks || !(modelled || !needs_model) {
            &[]
        } else {
            want
        };
        for w in always.iter().chain(checked) {
            if !log.contains(w) {
                return Err(format!("gpubench {name}: the run log has no `{w}`:{log}"));
            }
        }
        // Run FI: the 3D render's done status carries bits 2 and 3 on the
        // X51v (0x0c or 0x4c); bit 3 is modelled since rp-v0.30.0.
        if name == "run FI" {
            let done = fi_render_done(log);
            if done.len() != 3 || done.iter().any(|d| d & 0xC != 0xC) {
                return Err(format!(
                    "gpubench run FI: render done words {done:x?}, want bits 2 and 3 in all three:{log}"
                ));
            }
        }
        // Run G steps the clocks only from Windows Mobile's; a fork whose
        // core PLL reads 0 (rp-v0.29.0 and older: docs/hw/emulator.md) gets
        // the refusal, and one that starts as Windows Mobile leaves the
        // chip must trace the lock bit, run both passes with every step
        // GOOD twice (the models don't fail at any clock) and every control
        // GOOD, and put everything back.
        // Run GS soaks only from Windows Mobile's clock, as run G; stepping,
        // both settings must run to their end with every round and control
        // GOOD (the models don't fail at any clock), a minute's line each,
        // and everything put back. `DSIG` isn't modelled (reads 0:
        // emulator.md), so whether the label is sent is the device's.
        if name == "run GS" {
            let refused =
                log.contains("not Windows Mobile's clock and bus timing: nothing changed");
            let soaked = [
                "  the 16-bit label shown: DSIG 0x",
                "  soak 1: 99.67 MHz CL2, cores / 1: lock Some(",
                "  GS 99.67 MHz CL2, cores / 1, minute 1: ",
                "  soak 1 at 99.67 MHz CL2, cores / 1: ",
                "    control at 99.67 MHz after soak 1: GOOD",
                "  soak 2: 133.25 MHz CL2, cores / 1: lock Some(",
                "  GS 133.25 MHz CL2, cores / 1, minute 2: ",
                "  soak 2 at 133.25 MHz CL2, cores / 1: ",
                "    control at 99.67 MHz after soak 2: GOOD",
                "  put back: COREPLL 0xbb1",
            ]
            .iter()
            .all(|w| log.contains(w))
                && log.matches(", 0 BAD | worst 3D").count() == 6
                && log.matches(" | GOOD").count() == 4
                && !log.contains("| BAD")
                && !log.contains("STOPPED");
            if !(refused && log.contains("as found: COREPLL 0x0,") || soaked) {
                return Err(format!(
                    "gpubench run GS: neither refused cleanly nor soaked both settings:{log}"
                ));
            }
        }
        // Run GV steps only from Windows Mobile's clock, as runs G and GS;
        // stepping, all five settings GOOD with a control GOOD after each
        // (the models' times don't follow the clock, so only that every
        // setting ran right and the clock went back), and put back.
        if name == "run FT2F" {
            let refused =
                log.contains("not Windows Mobile's clock and bus timing: nothing changed");
            let ran = log.contains("  FT2 at 133.25 MHz CL2, cores / 1: lock Some(")
                && log.contains("FT2: 6 of 6 settings clean")
                && log.contains("  put back: COREPLL 0xbb1");
            if !(refused || ran) {
                return Err(format!(
                    "gpubench run FT2F: neither refused cleanly nor ran every setting at 133.25 MHz:
{log}"
                ));
            }
        }
        // Run GC refuses where Windows Mobile's clocks aren't modelled;
        // stepping, every reference repeatable and every kind right at
        // every setting (the models draw the same at any clock).
        if name == "run GC" {
            let refused =
                log.contains("not Windows Mobile's clock and bus timing: nothing changed");
            let mapped = log.matches("  GC reference ").count() == 7
                && !log.contains("isn't repeatable")
                && log
                    .matches("7 of 7 kinds right; the control at Windows Mobile's clock after it: 7 of 7 right")
                    .count()
                    == 5
                && !log.contains("| WRONG")
                && log.contains("  put back: COREPLL 0xbb1");
            if !(refused || mapped) {
                return Err(format!(
                    "gpubench run GC: neither refused cleanly nor mapped every setting:\n{log}"
                ));
            }
        }
        // Run GC2 refuses where Windows Mobile's clocks aren't modelled;
        // stepping, every reference proven, every kind right with
        // everything held at every setting both ways (the models draw the
        // same at any clock), the references the same at the end, and
        // each held to the host model of the 3D core.
        if name == "run GC2" {
            // Every reference the host model's, but the dithered one: the
            // wm-clocks fork build (rp-v0.29.0-1) has no 3D dither model.
            let modelled = log
                .lines()
                .filter(|l| {
                    l.starts_with("  GC2 reference ")
                        && !l.contains(" at the end: ")
                        && !l.starts_with("  GC2 reference RGB565 dithered Gouraud: ")
                })
                .filter(|l| l.ends_with("the host model's pixels: EQUAL in every band"))
                .count();
            let refused =
                log.contains("not Windows Mobile's clock and bus timing: nothing changed");
            let mapped = log.matches("  GC2 reference ").count() == 18
                && log.contains("  references: every one repeatable")
                && modelled == 8
                && log
                    .matches(
                        "9 of 9 kinds right with everything held; their controls: 9 of 9 right",
                    )
                    .count()
                    == 24
                && !log.contains("WRONG")
                && !log.contains("the clock didn't take")
                && log.contains("  GC2 references at the end: 9 of 9 as at the start")
                && log.contains("  put back: COREPLL 0xbb1");
            if !(refused || mapped) {
                return Err(format!(
                    "gpubench run GC2: neither refused cleanly nor mapped every setting:
{log}"
                ));
            }
        }
        // Run GC2D refuses where Windows Mobile's clocks aren't modelled;
        // stepping, as run GC2: every reference proven, every kind right
        // with everything held at every setting both ways, the references
        // the same at the end, and the texture way measured against
        // Gouraud for both targets.
        if name == "run GC2D" {
            let refused =
                log.contains("not Windows Mobile's clock and bus timing: nothing changed");
            let mapped = log.matches("  GC2 reference ").count() == 8
                && log.contains("  references: every one repeatable")
                && log.matches("  (d) at Windows Mobile's clock: ").count() == 2
                && log
                    .matches(
                        "4 of 4 kinds right with everything held; their controls: 4 of 4 right",
                    )
                    .count()
                    == 14
                && !log.contains("WRONG")
                && !log.contains("the clock didn't take")
                && log.contains("  GC2 references at the end: 4 of 4 as at the start")
                && log.contains("  put back: COREPLL 0xbb1");
            if !(refused || mapped) {
                return Err(format!(
                    "gpubench run GC2D: neither refused cleanly nor mapped every setting:
{log}"
                ));
            }
        }
        // GC2F and GC2H refuse where Windows Mobile's clocks aren't
        // modelled; stepping, as run GC2: every reference proven and the
        // same at the end, every kind right with everything held at every
        // setting both ways (GC2F: under each of its eight tables, each
        // compared with ours at Windows Mobile's clock).
        for (run, refs, line, settings, ends, tables) in [
            (
                "run GC2F",
                48,
                "3 of 3 kinds right with everything held; their controls: 3 of 3 right",
                48,
                ("  GC2 references at the end: 3 of 3 as at the start", 8),
                7 * 3,
            ),
            (
                "run GC2M",
                54,
                "9 of 9 kinds right with everything held; their controls: 9 of 9 right",
                24,
                ("  GC2 references at the end: 9 of 9 as at the start", 3),
                2 * 9,
            ),
            (
                "run GC2W",
                72,
                "9 of 9 kinds right with everything held; their controls: 9 of 9 right",
                24,
                ("  GC2 references at the end: 9 of 9 as at the start", 4),
                3 * 9,
            ),
            (
                "run GC2H2",
                18,
                "9 of 9 kinds right with everything held; their controls: 9 of 9 right",
                18,
                ("  GC2 references at the end: 9 of 9 as at the start", 1),
                0,
            ),
            (
                "run GC2H",
                18,
                "9 of 9 kinds right with everything held; their controls: 9 of 9 right",
                12,
                ("  GC2 references at the end: 9 of 9 as at the start", 1),
                0,
            ),
        ] {
            if name != run {
                continue;
            }
            let refused =
                log.contains("not Windows Mobile's clock and bus timing: nothing changed");
            let mapped = log.matches("  GC2 reference ").count() == refs
                && log.matches("  references: every one repeatable").count() == ends.1
                && log.matches(line).count() == settings
                && log.matches(ends.0).count() == ends.1
                && log
                    .matches(" reference at Windows Mobile's clock differs from ours in ")
                    .count()
                    == tables
                && !log.contains("WRONG")
                && !log.contains("the clock didn't take")
                && log.contains("  put back: COREPLL 0xbb1");
            if !(refused || mapped) {
                return Err(format!(
                    "gpubench {run}: neither refused cleanly nor mapped every setting:
{log}"
                ));
            }
        }
        // GC2R refuses where Windows Mobile's clocks aren't modelled;
        // stepping, the references repeatable, the registers compared
        // (the models change none but the clocks'), and every way into
        // 58.50 / 1 right on both kinds (the models draw the same at any
        // clock) with every control right.
        if name == "run GC2R" {
            let refused =
                log.contains("not Windows Mobile's clock and bus timing: nothing changed");
            let ways = log
                .lines()
                .filter(|l| l.starts_with("  GC2R ") && l.contains(" | "))
                .collect::<Vec<_>>();
            let mapped = log.matches("  GC2R reference ").count() == 2
                && !log.contains("isn't repeatable")
                && log.contains("  GC2R registers: ")
                && ways.len() == 10
                && ways.iter().all(|l| {
                    l.contains(": B wrong in 0 of 1 (the clock not taken 0)")
                        && l.ends_with("| A wrong in 0 of 1")
                })
                && log.contains("  put back: COREPLL 0xbb1");
            if !(refused || mapped) {
                return Err(format!(
                    "gpubench run GC2R: neither refused cleanly nor tried every way:
{log}"
                ));
            }
        }
        // FT3 (alone, and four times in GC3): every setting run and clean
        // (the models have no contention), the target at 0. With Windows
        // Mobile's clock modelled, three clocks (12 settings) and the clock
        // put back after each step; without it, the as-found clock only (4),
        // never nothing. GC3 also puts each memory setting back.
        if name == "run FT3" || name == "run GC3" {
            let stepped = !log.contains("the clock steps skipped, the clock as found only");
            let (settings, put_backs) = if stepped { (12, 2) } else { (4, 0) };
            let runs = if name == "run GC3" { 4 } else { 1 };
            let summary = format!("FT3: {settings} of {settings} settings clean");
            if log.matches(summary.as_str()).count() != runs
                || log.matches(" | CLEAN").count() != settings * runs
                || log.matches(" | ERRORS").count() != 0
                || log.matches("  put back: COREPLL 0xbb1,").count() != put_backs * runs
                || (name == "run GC3" && log.matches("  put back: LMCFG ").count() != 4)
            {
                return Err(format!(
                    "gpubench {name}: FT3 not run clean at every setting (want {runs} x {settings}), or a clock or memory setting not put back:\n{log}"
                ));
            }
            eprintln!(
                "    {name}: FT3 {runs} x {settings} settings clean{}",
                if stepped {
                    " at all three clocks, the clock put back"
                } else {
                    " at the clock as found (this release doesn't model Windows Mobile's clock)"
                }
            );
        }
        // GC2X refuses where Windows Mobile's clocks aren't modelled;
        // stepping, divide 1's own result and every flip tried (the
        // models draw right whatever the bits), each control right, and
        // the clock put back clean.
        if name == "run GC2X" {
            let refused =
                log.contains("not Windows Mobile's clock and bus timing: nothing changed");
            let tried = log
                .lines()
                .filter(|l| l.starts_with("  GC2X ") && l.contains(": reads 0x"))
                .collect::<Vec<_>>();
            let mapped = tried.len() == 13
                && tried
                    .iter()
                    .all(|l| l.contains(": RIGHT AT DIVIDE 1") || l.starts_with("  GC2X NO FLIP"))
                && tried.iter().all(|l| l.ends_with("| control right"))
                && log.contains(&format!(
                    "  GC2X: {} flips tried; {} drew right",
                    tried.len() - 1,
                    tried.len() - 1
                ))
                && log.contains("  put back: COREPLL 0xbb1, MBXCLK 0x6,");
            if !(refused || mapped) {
                return Err(format!(
                    "gpubench run GC2X: neither refused cleanly nor tried every flip:
{log}"
                ));
            }
        }
        if name == "run GV" {
            let refused =
                log.contains("not Windows Mobile's clock and bus timing: nothing changed");
            let stepped = (1..=5).all(|k| {
                log.contains(&format!("    control at 99.67 MHz after GV step {k}: GOOD"))
            }) && log.matches("  GV ").count() == 5
                && log.matches("  GV ").count() == log.matches("us | GOOD").count()
                && log.contains("  put back: COREPLL 0xbb1")
                && !log.contains("STOPPED");
            if !(refused && log.contains("as found: COREPLL 0x0,") || stepped) {
                return Err(format!(
                    "gpubench run GV: neither refused cleanly nor stepped every setting:\n{log}"
                ));
            }
        }
        if name == "run G" {
            let refused =
                log.contains("not Windows Mobile's clock and bus timing: nothing changed");
            let stepped = [
                "  lock trace 99.67 -> 74.75 MHz",
                "  lock trace 108.33 -> 99.67 MHz",
                "  pass 1: rising",
                "  pass 2: the same steps in reverse",
                "    control at 99.67 MHz step 22: GOOD",
                "  soak at 99.67 MHz, cores / 1: ",
                "    control at 99.67 MHz after the soak: GOOD",
                "  put back: COREPLL 0xbb1",
            ]
            .iter()
            .all(|w| log.contains(w))
                && log.matches("| GOOD").count() == 45
                && log.contains(" 0 BAD, the display short in 0")
                && !log.contains("| BAD")
                && !log.contains("STOPPED");
            if !(refused && log.contains("as found: COREPLL 0x0,") || stepped) {
                return Err(format!(
                    "gpubench run G: neither refused cleanly nor stepped every series:{log}"
                ));
            }
            eprintln!(
                "    gpubench run G: {}",
                if stepped {
                    "every step and control GOOD on the modelled clocks, put back"
                } else {
                    "refused: this fork doesn't start the clocks as Windows Mobile leaves them"
                }
            );
        }
        // Run V is built on run G's set-up: on a fork that doesn't start the
        // clocks as Windows Mobile leaves them it refuses as G does (its
        // lines were wanted unconditionally: the check failed on such forks).
        if name == "run V" {
            let refused =
                log.contains("not Windows Mobile's clock and bus timing: nothing changed");
            let loaded = [
                "V: which work at which clock",
                "  99.66 MHz, cores / 2: flush x5 false, 2D fill x20 false, whole check false (GOOD)",
                "  99.66 MHz, cores / 1: flush x5 false, 2D fill x20 false, whole check false (GOOD)",
                "  133.25 MHz, cores / 2: flush x5 false",
                "  put back: COREPLL 0xbb1",
            ]
            .iter()
            .all(|w| log.contains(w));
            if !(refused && log.contains("as found: COREPLL 0x0,") || loaded) {
                return Err(format!(
                    "gpubench run V: neither refused cleanly nor ran every load:{log}"
                ));
            }
            eprintln!(
                "    gpubench run V: {}",
                if loaded {
                    "every load at every clock, the display never short"
                } else {
                    "refused: this fork doesn't start the clocks as Windows Mobile leaves them"
                }
            );
        }
        if name == "run Y" {
            let refused = log.contains("not Windows Mobile's clock and bus timing (COREPLL 0x0,");
            let rescued = [
                "Y0: before the rebuild (the reference): speed against Y0",
                "Z1: SYSRST next",
                "Z5: the display",
                "Y1: after the rebuild: speed against Y0",
                "Y2: speed against Y0",
                "Y3: speed against Y0",
                "  self-refresh entered: true, active again: true",
                "Y5: speed against Y0",
                "everything given back: true",
            ]
            .iter()
            .all(|w| log.contains(w))
                && log.matches("| GOOD").count() == 6
                && !log.contains("| BAD");
            if !(refused || rescued) {
                return Err(format!(
                    "gpubench run Y: neither refused cleanly nor every check GOOD:{log}"
                ));
            }
            eprintln!(
                "    gpubench run Y: {}",
                if rescued {
                    "the reference, the rebuild and every restart checked GOOD"
                } else {
                    "refused: this fork doesn't start the clocks as Windows Mobile leaves them"
                }
            );
        }
        // Run F's text must come out exactly: white pixels = lit coverage,
        // and F4's, F5's and F6a's pictures word for word F0's (F6b and F6c
        // ask whether the engine keeps its surfaces: the device answers;
        // here they need only to run).
        // F7's three pictures (the gate's F6: fewer operations) must be F0's
        // too, word for word.
        if name == "run F"
            && log
                .lines()
                .filter(|l| l.starts_with("  F7") && l.contains("words differing from F0: 0"))
                .count()
                < 3
        {
            return Err(format!(
                "gpubench run F: an F7 picture differs from F0's:{log}"
            ));
        }
        if name == "run F" && log.matches("words differing from F0: 0").count() < 3 {
            return Err(format!(
                "gpubench run F: F4, F5 or F6a differs from F0's picture:{log}"
            ));
        }
        if name == "run F" {
            for l in log.lines().filter(|l| l.contains("; white ")) {
                let nums: Vec<&str> = l
                    .rsplit("; white ")
                    .next()
                    .unwrap_or("")
                    .split(", lit ")
                    .collect();
                if nums.len() != 2 || nums[0].trim() != nums[1].trim() {
                    return Err(format!("gpubench run F: the text came out wrong: `{l}`"));
                }
            }
        }
        if modelled && name == "run A" && !log.contains(" 0 hung renders recovered") {
            return Err(format!("gpubench {name}: a render hung on the model:{log}"));
        }
        // G4a: every checked pixel of all three rectangles its own texel,
        // as on the X51v.
        if rect && name == "run A" && log.matches(": 64 of 64 checked pixels").count() != 3 {
            return Err(format!(
                "gpubench {name}: G4a's rectangles should draw as on the device:{log}"
            ));
        }
        // A run that says "no map" runs no map settings (GC2T ran GC2's whole
        // map on the X51v, missing from a hand-kept list; the emulator's 3
        // renders a setting hid the time).
        if log.contains("no map;") && !log.contains(" at 0 settings rising then falling") {
            return Err(format!(
                "gpubench {name}: a no-map run ran GC2's map settings:{log}"
            ));
        }
        // A part that couldn't get its memory logs and skips itself; on the
        // emulator that is always the image's own sizing bug (GC2Q's part C
        // ran nothing on the X51v, and this check passed it).
        for short in [
            "not enough SDRAM",
            "the plan holds too many",
            "no 2700G memory for",
        ] {
            if let Some(l) = log.lines().find(|l| l.contains(short)) {
                return Err(format!(
                    "gpubench {name}: a part ran short of memory and skipped itself ({l}):{log}"
                ));
            }
        }
        eprintln!("    gpubench {name} OK: every stage ran and it returned to the loader");
    }
    if card_file(&sd, "other.txt")? != other {
        return Err("gpubench: the card logs changed a file outside rplog/".into());
    }
    eprintln!(
        "    gpubench: every run's card log matched its run log; nothing else on the card changed"
    );
    Ok(())
}
