// Shared by the build scripts that read `local/` (`include!`d: build
// scripts can't share a module otherwise). Quality audit C40.
//
// Cargo reruns a build script when a watched path's time is newer than
// its last run. A picture copied into `local/` keeps its own (older) time,
// so watching the file alone missed it: built once without it, the image
// kept its fallback. A folder's time changes when an entry is added or
// removed, whatever the entry's own time, so the nearest folder that
// exists on the way to the file is watched too: the workspace root before
// `local/` exists (a `local/` made or linked in changes it), then `local/`,
// and so on down to the file's own folder.
//
// A file overwritten in place by an older-dated copy changes neither its
// own time past the last run nor its folder's (cargo's checksum freshness,
// `-Zchecksum-freshness`, doesn't cover build scripts: tried 2026-10-22),
// so `cargo xtask` also fingerprints every file under `LOCAL_INPUT_DIRS`
// (path, length, time) into `RP_LOCAL_INPUTS`, which these scripts watch:
// any change to a file, whatever its date, reruns them.

/// The `local/` folders the build scripts read; every watched file must be
/// under one (`watch_local` checks), or `RP_LOCAL_INPUTS` wouldn't see it.
#[allow(dead_code)]
const LOCAL_INPUT_DIRS: &[&str] = &["local/concept", "local/icons", "local/wallpapers"];

/// The variable `cargo xtask` sets to the inputs' fingerprint.
#[allow(dead_code)]
const LOCAL_INPUTS_VAR: &str = "RP_LOCAL_INPUTS";

/// The nearest existing folder above `path` (its own folder if that
/// exists), or `None`.
#[allow(dead_code)]
fn nearest_existing_dir(path: &std::path::Path) -> Option<std::path::PathBuf> {
    path.ancestors()
        .skip(1)
        .find(|a| a.is_dir())
        .map(std::path::Path::to_path_buf)
}

/// Tells cargo to rerun the build script when `path` changes or appears,
/// even with an old time (see above).
#[allow(dead_code)]
fn watch_local(path: &std::path::Path) {
    let slashed = path.to_string_lossy().replace('\\', "/");
    assert!(
        LOCAL_INPUT_DIRS
            .iter()
            .any(|d| slashed.contains(&format!("/{d}/"))),
        "{slashed}: not under LOCAL_INPUT_DIRS (tools/build/local_input.rs)"
    );
    println!("cargo:rerun-if-env-changed={LOCAL_INPUTS_VAR}");
    println!("cargo:rerun-if-changed={}", path.display());
    if let Some(dir) = nearest_existing_dir(path) {
        println!("cargo:rerun-if-changed={}", dir.display());
    }
}
