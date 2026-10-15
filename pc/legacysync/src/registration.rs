//! How old installers find LegacySync's `CeAppMgr.exe`
//! (docs/legacysync/ceappmgr-standin.md): the registry values they read,
//! as a `.reg` file for the owner to review. Nothing here writes the
//! registry.

use std::fmt::Write as _;
use std::path::Path;

/// The version LegacySync reports as ActiveSync's: WMDC 6.1's (build 6965),
/// so installers that refuse "old ActiveSync" proceed.
pub const REPORTED_VERSION: (u32, u32, u32) = (6, 1, 6965);

/// Doubles backslashes and quotes for a `.reg` string.
fn reg_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// The `.reg` text registering `exe` (LegacySync's `CeAppMgr.exe`) in both
/// registry views: the 32-bit installers read `WOW6432Node`, 64-bit tools
/// the native view.
#[must_use]
pub fn reg_file(exe: &Path) -> String {
    let dir = exe.parent().unwrap_or(Path::new(""));
    let (major, minor, build) = REPORTED_VERSION;
    let mut t = String::from("Windows Registry Editor Version 5.00\n");
    for view in ["SOFTWARE", "SOFTWARE\\WOW6432Node"] {
        let _ = write!(
            t,
            "\n[HKEY_LOCAL_MACHINE\\{view}\\Microsoft\\Windows CE Services]\n\
             \"InstalledDir\"=\"{}\"\n\
             \"MajorVersion\"=dword:{major:08x}\n\
             \"MinorVersion\"=dword:{minor:08x}\n\
             \"BuildNumber\"=dword:{build:08x}\n\
             \n[HKEY_LOCAL_MACHINE\\{view}\\Microsoft\\Windows\\CurrentVersion\\App Paths\\CEAPPMGR.EXE]\n\
             @=\"{}\"\n",
            reg_string(&dir.to_string_lossy()),
            reg_string(&exe.to_string_lossy()),
        );
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_views_and_both_lookups() {
        let t = reg_file(Path::new(
            "C:\\Program Files\\RetroPane\\LegacySync\\CeAppMgr.exe",
        ));
        assert!(t.starts_with("Windows Registry Editor Version 5.00\n"));
        assert_eq!(
            t.matches("\"InstalledDir\"=\"C:\\\\Program Files\\\\RetroPane\\\\LegacySync\"")
                .count(),
            2
        );
        assert_eq!(t.matches("App Paths\\CEAPPMGR.EXE]\n@=\"C:\\\\Program Files\\\\RetroPane\\\\LegacySync\\\\CeAppMgr.exe\"").count(), 2);
        assert!(t.contains(
            "[HKEY_LOCAL_MACHINE\\SOFTWARE\\WOW6432Node\\Microsoft\\Windows CE Services]"
        ));
        assert!(t.contains("\"MajorVersion\"=dword:00000006"));
    }
}
