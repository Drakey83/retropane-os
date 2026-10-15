//! `CeAppMgr.exe`: LegacySync's stand-in for ActiveSync's Application
//! Manager.
//!
//! Called by old installers exactly as ActiveSync was:
//!
//! ```text
//! CeAppMgr.exe [/report] "C:\...\Game.ini" ["C:\...\Other.ini" ...]
//! ```
//!
//! and by people:
//!
//! ```text
//! CeAppMgr.exe --out <folder> "Game.ini"   stage into <folder>
//! CeAppMgr.exe --print-registration        the .reg that would make
//!                                           installers find this program
//! ```
//!
//! Every call is logged (ADR-0028: every installer tried is logged with
//! what it called) in `%LOCALAPPDATA%\RetroPane\LegacySync\legacysync.log`.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use rp_legacysync::{registration, select, stage};

/// LegacySync's data folder.
fn data_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
        .join("RetroPane")
        .join("LegacySync")
}

/// Appends a line to the log; logging problems never stop an install.
fn log(line: &str) {
    let dir = data_dir();
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("legacysync.log"))
    {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let _ = writeln!(f, "{} {line}", utc(secs));
    }
}

/// `YYYY-MM-DD hh:mm:ss` (UTC) from Unix seconds.
fn utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil date from days since 1970-01-01 (Howard Hinnant's method).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}",
        rem / 3600,
        rem / 60 % 60,
        rem % 60
    )
}

fn usage() -> &'static str {
    "LegacySync's Application Manager (CeAppMgr.exe) for RetroPane.\n\
     \n\
     CeAppMgr.exe [/report] \"Game.ini\" [...]   stage each INI's CAB for the device\n\
     CeAppMgr.exe --out <folder> \"Game.ini\"     stage into <folder>\n\
     CeAppMgr.exe --print-registration          show the registry values that let\n\
     \x20                                          installers find this program (writes nothing)\n"
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    log(&format!("called with {args:?}"));
    let mut inis = Vec::new();
    let mut out = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.to_ascii_lowercase().as_str() {
            "--help" | "-h" | "/?" => {
                print!("{}", usage());
                return ExitCode::SUCCESS;
            }
            "--print-registration" => {
                let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("CeAppMgr.exe"));
                print!("{}", registration::reg_file(&exe));
                return ExitCode::SUCCESS;
            }
            "--out" => match it.next() {
                Some(dir) => out = Some(PathBuf::from(dir)),
                None => {
                    eprintln!("--out needs a folder");
                    return ExitCode::FAILURE;
                }
            },
            // ActiveSync's error-report switch: reports always happen here.
            "/report" | "-report" => {}
            other if other.starts_with('/') || other.starts_with('-') => {
                log(&format!("ignored unknown switch {a:?}"));
            }
            _ => inis.push(PathBuf::from(a)),
        }
    }
    if inis.is_empty() {
        eprint!("{}", usage());
        return ExitCode::FAILURE;
    }
    let outbox = out.unwrap_or_else(|| data_dir().join("Outbox"));
    let mut failed = false;
    for ini in &inis {
        failed |= !run(ini, &outbox);
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn run(ini: &Path, outbox: &Path) -> bool {
    match stage::stage(ini, outbox, &select::X51V) {
        Ok(staged) => {
            for s in staged {
                println!("Ready for RetroPane: {} (from {:?})", s.app, s.component);
                println!("  staged in {}", s.folder.display());
                println!("  plan: {}", s.folder.join("plan.txt").display());
                for w in &s.warnings {
                    println!("  note: {w}");
                }
                log(&format!("staged {:?} -> {}", s.app, s.cab.display()));
            }
            println!("Sending to the device comes with the LegacySync link (not built yet).");
            true
        }
        Err(e) => {
            eprintln!("{}: {e}", ini.display());
            log(&format!(
                "{}: refused: {}",
                ini.display(),
                e.replace('\n', " | ")
            ));
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::utc;

    #[test]
    fn dates() {
        assert_eq!(utc(0), "1970-01-01 00:00:00");
        assert_eq!(utc(951_782_400), "2000-02-29 00:00:00");
        assert_eq!(utc(1_791_975_600), "2026-10-14 11:00:00");
    }
}
