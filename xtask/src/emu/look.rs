//! The look check's emulator run (R185: emu-today-look).

#[allow(clippy::wildcard_imports)]
use super::*;

/// `cargo xtask emu-today-look`: R185's look check unattended (sheet
/// `docs/research/sheets/r185-look-check-unattended.md`): the image built
/// with the look check and the recording `cargo xtask look-replay` makes
/// for the default settings (Weather, then Appearance three times round
/// with every page after each), run in the emulator to the loader; passes
/// when no pass saw the wrong look, both looks were judged right on every
/// page, and few passes were unknown.
pub fn today_look(root: &Path) -> Result<(), String> {
    crate::lookcheck::recording(root, None, crate::lookcheck::RECORDING_EMU)?;
    let gap = crate::lookcheck::GAP_MS.to_string();
    let envs = [
        ("RP_TODAY_REPLAY", "1"),
        ("RP_TODAY_REPLAY_FILE", crate::lookcheck::RECORDING_EMU),
        ("RP_TODAY_REPLAY_GAP_MS", gap.as_str()),
        ("RP_TODAY_LOOK_CHECK", "1"),
    ];
    let bin = variant_image_envs(root, "rp-today", &envs)?;
    let loader = emu_dir(root)?.join("rp-loader.bin");
    crate::sdkit::raw_binary(&crate::arm_release(root).join("rp-loader"), &loader)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            // Guest time from instructions, 2 ns each (500 million a
            // second: the nearest power of two at or below the PXA270's
            // 624 MHz, one a cycle at best), so the frame budget measures
            // the work, not the PC's load (R171: host time read the same
            // still frame 20 ms idle and 68 ms beside a build).
            extra: &["-icount", "shift=1"],
            input: Some((&rp_loadproto::frame(&payload), rp_today::MARKER)),
            during: None,
            then: Some("RPL END"),
            timeout: Duration::from_secs(600),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("today look: never came back to the loader")?;
    if crashed(log) || log.contains("did not return") {
        return Err(format!("today look: didn't return cleanly:{log}"));
    }
    let kept = scratch(root, "today-look-log.txt")?;
    let _ = std::fs::write(&kept, log);
    crate::lookcheck::judge(log).map_err(|e| format!("today look: {e}"))?;
    let w = crate::lookcheck::frame_budget(log).map_err(|e| format!("today look: {e}"))?;
    eprintln!(
        "      frame budget (R171, guest time under -icount): the worst still frame {} us, the worst moving frame {} us (33,000 allowed each), the worst single whole-screen change over 33 ms {} us (45,000 allowed, the owner's rule of 2026-11-10)",
        w.still, w.moving, w.whole
    );
    eprintln!("    today look OK (log {})", kept.display());
    Ok(())
}
