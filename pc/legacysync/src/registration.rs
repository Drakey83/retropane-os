//! How old installers find LegacySync's `ceappmgr.exe`, and how LegacySync's
//! installer takes those registry values over and gives them back
//! (docs/legacysync/ceappmgr-standin.md, option A, chosen by the owner
//! 2026-10-15).
//!
//! Three `.reg` files, all made here, none applied here:
//!
//! - **take-over** ([`reg_file`]): points both lookups, in both registry
//!   views, at LegacySync.
//! - **backup** ([`backup_reg`]): the values the take-over replaces, exactly
//!   as they were (copied line for line from `reg export`, so any type and
//!   any locale round-trip), and a deletion for each value or key that
//!   didn't exist. Importing it is the uninstall's restore.
//! - the start-up check ([`check`]): whether the lookups still point at this
//!   LegacySync (a WMDC repair takes them back).
//!
//! **Nothing in this program writes the registry.** Reading uses
//! `reg export` (to a temporary file) and `reg query`; applying the files is
//! the installer's job, run elevated, after the owner says so.

use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

/// The version LegacySync reports as ActiveSync's: WMDC 6.1's (build 6965),
/// so installers that refuse "old ActiveSync" proceed.
pub const REPORTED_VERSION: (u32, u32, u32) = (6, 1, 6965);

/// The two registry views: 64-bit tools read the first, the 32-bit
/// installers the second.
pub const VIEWS: [&str; 2] = ["SOFTWARE", "SOFTWARE\\WOW6432Node"];

const SERVICES: &str = "Microsoft\\Windows CE Services";
const APP_PATHS: &str = "Microsoft\\Windows\\CurrentVersion\\App Paths\\CEAPPMGR.EXE";
const SERVICES_VALUES: &[&str] = &[
    "InstalledDir",
    "MajorVersion",
    "MinorVersion",
    "BuildNumber",
];
/// `""` is a key's default value (`@` in a `.reg` file).
const APP_PATHS_VALUES: &[&str] = &[""];

/// A key LegacySync manages and the values in it it replaces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Managed {
    /// Full path, `HKEY_LOCAL_MACHINE\...`.
    pub key: String,
    /// Value names (`""` = the default value).
    pub values: &'static [&'static str],
}

/// The four keys (two lookups, two views).
#[must_use]
pub fn managed() -> Vec<Managed> {
    let mut out = Vec::new();
    for view in VIEWS {
        out.push(Managed {
            key: format!("HKEY_LOCAL_MACHINE\\{view}\\{SERVICES}"),
            values: SERVICES_VALUES,
        });
        out.push(Managed {
            key: format!("HKEY_LOCAL_MACHINE\\{view}\\{APP_PATHS}"),
            values: APP_PATHS_VALUES,
        });
    }
    out
}

/// What a managed key holds now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// The key doesn't exist.
    KeyAbsent,
    /// The key exists; each managed value's `.reg` lines, if present.
    Present(Vec<(&'static str, Option<String>)>),
}

/// Doubles backslashes and quotes for a `.reg` string.
fn reg_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// The `.reg` line prefix of value `name`.
fn value_prefix(name: &str) -> String {
    if name.is_empty() {
        "@=".to_owned()
    } else {
        format!("\"{}\"=", reg_string(name))
    }
}

/// The folder of a Windows path, split as Windows does (`\` or `/`)
/// whatever system runs this: the registry holds Windows paths, and
/// `Path::parent` splits only on `/` elsewhere (the CI host).
fn windows_parent(path: &str) -> &str {
    path.rsplit_once(['\\', '/']).map_or("", |(dir, _)| dir)
}

/// The take-over: `exe` (LegacySync's `ceappmgr.exe`) registered in both
/// views.
#[must_use]
pub fn reg_file(exe: &Path) -> String {
    let exe_s = exe.to_string_lossy();
    let dir = windows_parent(&exe_s);
    let (major, minor, build) = REPORTED_VERSION;
    let mut t = String::from("Windows Registry Editor Version 5.00\n");
    for view in VIEWS {
        let _ = write!(
            t,
            "\n[HKEY_LOCAL_MACHINE\\{view}\\{SERVICES}]\n\
             \"InstalledDir\"=\"{}\"\n\
             \"MajorVersion\"=dword:{major:08x}\n\
             \"MinorVersion\"=dword:{minor:08x}\n\
             \"BuildNumber\"=dword:{build:08x}\n\
             \n[HKEY_LOCAL_MACHINE\\{view}\\{APP_PATHS}]\n\
             @=\"{}\"\n",
            reg_string(dir),
            reg_string(&exe.to_string_lossy()),
        );
    }
    t
}

/// Reads the managed values of `key` from `reg export` output: only the
/// key's own section, each value's line(s) verbatim (continued lines
/// included).
#[must_use]
pub fn parse_export(text: &str, key: &str, names: &'static [&'static str]) -> State {
    let header = format!("[{key}]");
    let mut lines = text.lines().map(|l| l.trim_end_matches('\r'));
    if !lines.any(|l| l.eq_ignore_ascii_case(&header)) {
        return State::KeyAbsent;
    }
    let section: Vec<&str> = lines.take_while(|l| !l.starts_with('[')).collect();
    let mut out = Vec::new();
    for &name in names {
        let prefix = value_prefix(name);
        let mut found = None;
        let mut i = 0;
        while i < section.len() {
            if section[i].len() >= prefix.len()
                && section[i][..prefix.len()].eq_ignore_ascii_case(&prefix)
            {
                let mut v = section[i].to_owned();
                while v.ends_with('\\') && i + 1 < section.len() {
                    i += 1;
                    v.push('\n');
                    v.push_str(section[i]);
                }
                found = Some(v);
                break;
            }
            i += 1;
        }
        out.push((name, found));
    }
    State::Present(out)
}

/// The backup (and uninstall restore): every managed value as it was, a
/// deletion (`=-`) for each that was absent, and a key deletion for each
/// key that was absent (LegacySync made it, so removing it, with any
/// Application Manager records LegacySync added below it, is right).
#[must_use]
pub fn backup_reg(states: &[(Managed, State)]) -> String {
    let mut t = String::from(
        "Windows Registry Editor Version 5.00\n\n\
         ; LegacySync: the values its installer replaced, as they were.\n\
         ; Importing this file (the uninstaller does) puts them back.\n",
    );
    for (m, state) in states {
        match state {
            State::KeyAbsent => {
                let _ = write!(t, "\n[-{}]\n", m.key);
            }
            State::Present(values) => {
                let _ = write!(t, "\n[{}]\n", m.key);
                for (name, lines) in values {
                    match lines {
                        Some(l) => {
                            let _ = writeln!(t, "{l}");
                        }
                        None => {
                            let _ = writeln!(t, "{}-", value_prefix(name));
                        }
                    }
                }
            }
        }
    }
    t
}

/// A `.reg` string value's text (`"x"="C:\\a"` → `C:\a`), if it is one.
fn string_value(lines: &str) -> Option<String> {
    let (_, rest) = lines.split_once("=\"")?;
    let body = rest.strip_suffix('"')?;
    let mut out = String::new();
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        out.push(if c == '\\' { chars.next()? } else { c });
    }
    Some(out)
}

/// The string value `name` of a managed key's state.
fn get(state: &State, name: &str) -> Option<String> {
    match state {
        State::Present(values) => values
            .iter()
            .find(|(n, _)| *n == name)?
            .1
            .as_deref()
            .and_then(string_value),
        State::KeyAbsent => None,
    }
}

/// Whether `states` point both lookups, in both views, at `exe`.
#[must_use]
pub fn points_at(states: &[(Managed, State)], exe: &Path) -> bool {
    let exe_s = exe.to_string_lossy();
    let dir_s = windows_parent(&exe_s);
    states.iter().all(|(m, s)| {
        let (name, want) = if m.values == SERVICES_VALUES {
            ("InstalledDir", dir_s)
        } else {
            ("", &*exe_s)
        };
        get(s, name).is_some_and(|v| v.eq_ignore_ascii_case(want))
    })
}

/// Reads one managed key's state with `reg export` (read-only; the export
/// goes to a temporary file that is removed). A key `reg` can't export is
/// absent if `reg query` can't find it either.
pub fn read_state(m: &Managed) -> Result<State, String> {
    let file = std::env::temp_dir().join(format!("rp-legacysync-{}.reg", std::process::id()));
    let exported = Command::new("reg")
        .args(["export", &m.key])
        .arg(&file)
        .args(["/y", "/reg:64"])
        .output()
        .map_err(|e| format!("can't run reg: {e}"))?;
    if !exported.status.success() {
        let _ = std::fs::remove_file(&file);
        let exists = Command::new("reg")
            .args(["query", &m.key, "/reg:64"])
            .output()
            .map_err(|e| format!("can't run reg: {e}"))?
            .status
            .success();
        return if exists {
            Err(format!("reg export {} failed", m.key))
        } else {
            Ok(State::KeyAbsent)
        };
    }
    let bytes = std::fs::read(&file).map_err(|e| format!("reading the export: {e}"));
    let _ = std::fs::remove_file(&file);
    let bytes = bytes?;
    // reg export writes UTF-16LE with a byte-order mark.
    let text = match bytes.strip_prefix(b"\xFF\xFE") {
        Some(rest) => {
            let units: Vec<u16> = rest
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_le_bytes(*c))
                .collect();
            String::from_utf16_lossy(&units)
        }
        None => String::from_utf8_lossy(&bytes).into_owned(),
    };
    Ok(parse_export(&text, &m.key, m.values))
}

/// Every managed key's current state.
pub fn read_all() -> Result<Vec<(Managed, State)>, String> {
    managed()
        .into_iter()
        .map(|m| read_state(&m).map(|s| (m, s)))
        .collect()
}

/// The backup for the installer, refused when the lookups already point at
/// `exe`: a reinstall must keep the first install's backup, or the
/// uninstaller would "restore" LegacySync itself.
pub fn backup_for_install(states: &[(Managed, State)], exe: &Path) -> Result<String, String> {
    let ours = states.iter().any(|(m, s)| {
        m.values == APP_PATHS_VALUES
            && get(s, "").is_some_and(|v| v.eq_ignore_ascii_case(&exe.to_string_lossy()))
    });
    if ours {
        return Err(
            "the lookups already point at this LegacySync: keep the backup from its first install"
                .to_owned(),
        );
    }
    Ok(backup_reg(states))
}

/// The start-up check, in words.
#[must_use]
pub fn check(states: &[(Managed, State)], exe: &Path) -> String {
    if points_at(states, exe) {
        return "Old installers find this LegacySync (both lookups, both views).".to_owned();
    }
    let mut t = String::from("Old installers don't reach this LegacySync:\n");
    for (m, s) in states {
        let name = if m.values == SERVICES_VALUES {
            "InstalledDir"
        } else {
            ""
        };
        let shown = if name.is_empty() { "(default)" } else { name };
        let _ = writeln!(
            t,
            "  {} {shown} = {}",
            m.key,
            get(s, name).unwrap_or_else(|| "(not set)".to_owned())
        );
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXE: &str = "C:\\Program Files\\RetroPane\\LegacySync\\ceappmgr.exe";

    #[test]
    fn take_over_covers_both_views_and_both_lookups() {
        let t = reg_file(Path::new(EXE));
        assert!(t.starts_with("Windows Registry Editor Version 5.00\n"));
        assert_eq!(
            t.matches("\"InstalledDir\"=\"C:\\\\Program Files\\\\RetroPane\\\\LegacySync\"")
                .count(),
            2
        );
        assert_eq!(
            t.matches("@=\"C:\\\\Program Files\\\\RetroPane\\\\LegacySync\\\\ceappmgr.exe\"")
                .count(),
            2
        );
        assert!(t.contains(
            "[HKEY_LOCAL_MACHINE\\SOFTWARE\\WOW6432Node\\Microsoft\\Windows CE Services]"
        ));
        assert!(t.contains("\"MajorVersion\"=dword:00000006"));
    }

    /// The shape of `reg export` on the owner's PC (WMDC installed).
    const WMDC_SERVICES: &str = "Windows Registry Editor Version 5.00\r\n\r\n\
        [HKEY_LOCAL_MACHINE\\SOFTWARE\\WOW6432Node\\Microsoft\\Windows CE Services]\r\n\
        \"SerialPort\"=\"Bluetooth\"\r\n\
        \"MajorVersion\"=dword:00000006\r\n\
        \"MinorVersion\"=dword:00000000\r\n\
        \"InstalledDir\"=\"C:\\\\WINDOWS\\\\WindowsMobile\"\r\n\
        \"Odd\"=hex(7):41,00,\\\r\n  00,00\r\n\
        \r\n\
        [HKEY_LOCAL_MACHINE\\SOFTWARE\\WOW6432Node\\Microsoft\\Windows CE Services\\AppMgr]\r\n\
        \"BuildNumber\"=dword:00000099\r\n";

    #[test]
    fn export_values_are_kept_verbatim_and_subkeys_ignored() {
        let key = "HKEY_LOCAL_MACHINE\\SOFTWARE\\WOW6432Node\\Microsoft\\Windows CE Services";
        let s = parse_export(WMDC_SERVICES, key, SERVICES_VALUES);
        assert_eq!(
            s,
            State::Present(vec![
                (
                    "InstalledDir",
                    Some("\"InstalledDir\"=\"C:\\\\WINDOWS\\\\WindowsMobile\"".to_owned())
                ),
                (
                    "MajorVersion",
                    Some("\"MajorVersion\"=dword:00000006".to_owned())
                ),
                (
                    "MinorVersion",
                    Some("\"MinorVersion\"=dword:00000000".to_owned())
                ),
                // In the AppMgr subkey only: absent from this key.
                ("BuildNumber", None),
            ])
        );
        let odd = parse_export(WMDC_SERVICES, key, &["Odd"]);
        assert_eq!(
            odd,
            State::Present(vec![(
                "Odd",
                Some("\"Odd\"=hex(7):41,00,\\\n  00,00".to_owned())
            )])
        );
        assert_eq!(
            parse_export(
                WMDC_SERVICES,
                "HKEY_LOCAL_MACHINE\\SOFTWARE\\Other",
                SERVICES_VALUES
            ),
            State::KeyAbsent
        );
    }

    fn states(installed: Option<&str>, app_path: Option<&str>) -> Vec<(Managed, State)> {
        managed()
            .into_iter()
            .map(|m| {
                let s = if m.values == SERVICES_VALUES {
                    State::Present(vec![
                        (
                            "InstalledDir",
                            installed.map(|v| format!("\"InstalledDir\"=\"{}\"", reg_string(v))),
                        ),
                        (
                            "MajorVersion",
                            Some("\"MajorVersion\"=dword:00000006".to_owned()),
                        ),
                        ("MinorVersion", None),
                        ("BuildNumber", None),
                    ])
                } else {
                    match app_path {
                        Some(p) => {
                            State::Present(vec![("", Some(format!("@=\"{}\"", reg_string(p))))])
                        }
                        None => State::KeyAbsent,
                    }
                };
                (m, s)
            })
            .collect()
    }

    #[test]
    fn backup_restores_values_deletes_new_ones_and_new_keys() {
        let b = backup_reg(&states(Some("C:\\WINDOWS\\WindowsMobile"), None));
        assert!(b.contains("[HKEY_LOCAL_MACHINE\\SOFTWARE\\WOW6432Node\\Microsoft\\Windows CE Services]\n\"InstalledDir\"=\"C:\\\\WINDOWS\\\\WindowsMobile\"\n\"MajorVersion\"=dword:00000006\n\"MinorVersion\"=-\n\"BuildNumber\"=-\n"), "{b}");
        assert_eq!(b.matches("[-HKEY_LOCAL_MACHINE\\").count(), 2, "{b}");
        assert!(b.contains("[-HKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths\\CEAPPMGR.EXE]"));
        let b = backup_reg(&states(
            None,
            Some("C:\\WINDOWS\\WindowsMobile\\CEAppMgr.exe"),
        ));
        assert!(b.contains("\"InstalledDir\"=-\n"));
        assert!(b.contains("@=\"C:\\\\WINDOWS\\\\WindowsMobile\\\\CEAppMgr.exe\"\n"));
    }

    #[test]
    fn the_check_and_the_reinstall_guard() {
        let exe = Path::new(EXE);
        let dir = "C:\\Program Files\\RetroPane\\LegacySync";
        let ours = states(Some(dir), Some(EXE));
        assert!(points_at(&ours, exe));
        assert!(check(&ours, exe).starts_with("Old installers find this LegacySync"));
        assert!(
            backup_for_install(&ours, exe).is_err(),
            "a reinstall keeps the first backup"
        );
        let wmdc = states(
            Some("C:\\WINDOWS\\WindowsMobile"),
            Some("C:\\WINDOWS\\WindowsMobile\\CEAppMgr.exe"),
        );
        assert!(!points_at(&wmdc, exe));
        assert!(
            check(&wmdc, exe)
                .contains("CEAPPMGR.EXE (default) = C:\\WINDOWS\\WindowsMobile\\CEAppMgr.exe")
        );
        assert!(backup_for_install(&wmdc, exe).is_ok());
        // Case doesn't matter to Windows, so not to the check either.
        assert!(points_at(
            &states(Some(&dir.to_uppercase()), Some(&EXE.to_uppercase())),
            exe
        ));
    }

    #[test]
    fn string_values_unescape() {
        assert_eq!(
            string_value("@=\"C:\\\\a \\\"b\\\"\"").as_deref(),
            Some("C:\\a \"b\"")
        );
        assert_eq!(string_value("\"x\"=dword:00000001"), None);
    }

    /// Reads the real registry (read-only). On the owner's PC, WMDC owns the
    /// lookups, so the backup must hold its values.
    #[cfg(windows)]
    #[test]
    fn the_real_registry_reads_back() {
        let all = read_all().unwrap();
        assert_eq!(all.len(), 4);
        let b = backup_reg(&all);
        eprintln!("{b}");
        assert!(b.starts_with("Windows Registry Editor Version 5.00"));
    }
}
