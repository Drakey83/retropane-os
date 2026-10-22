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
    println!("cargo:rerun-if-changed={}", path.display());
    if let Some(dir) = nearest_existing_dir(path) {
        println!("cargo:rerun-if-changed={}", dir.display());
    }
}
