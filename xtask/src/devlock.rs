//! The machine-wide Axim lock: one process at a time talks to the docked
//! X51v (docs/testing.md, "One device run at a time").
//!
//! Two sessions' runs overlapped on 2026-10-15: a hand-made "pause file"
//! one session deleted for a few seconds let another's restart loop start a
//! load while the first run had the device. A file whose *existence* means
//! "busy" is a convention anyone can lift in passing; this is an OS lock on
//! `%LOCALAPPDATA%\RetroPane\axim.lock` (`LockFileEx` on Windows, `flock`
//! elsewhere), which the OS drops when the holder's handle closes, so a
//! killed or crashed holder can never leave the Axim locked.
//!
//! - **Where it is taken:** [`ensure_held`], called by `usb::powershell`,
//!   the one function that runs anything on the device's port. Every
//!   device command goes through it, so no list of commands can fall out of
//!   date (a source-scan test keeps it the only way out). It is held from
//!   the first contact to the end of the process: a `load` holds it across
//!   the send, the device's reconnect and the run log, never across the
//!   build before it.
//! - **A second caller waits**, saying who holds it, or with `--no-wait`
//!   (or `RP_AXIM_NO_WAIT=1`) fails at once.
//! - **The holder is visible:** `axim.holder` next to the lock names the
//!   command line (image included), the process, the checkout and the time;
//!   `cargo xtask axim` prints it. The note is only believed while the lock
//!   is actually held, so a crashed holder's leftover note means nothing.

use std::fs::File;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The lock this process holds, kept until it exits (the OS releases it).
static HELD: Mutex<Option<Held>> = Mutex::new(None);

/// How often a waiter tries again.
const POLL: Duration = Duration::from_millis(250);
/// How often a waiter repeats that it is still waiting.
const REMIND: Duration = Duration::from_secs(60);

/// A held lock: dropping it (or the process ending) releases it.
pub struct Held {
    _file: File,
    holder: PathBuf,
}

impl Drop for Held {
    fn drop(&mut self) {
        // Tidy only; the note is ignored while the lock is free anyway.
        let _ = std::fs::remove_file(&self.holder);
    }
}

/// Whether a busy lock is waited for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wait {
    Yes,
    No,
}

/// `%LOCALAPPDATA%\RetroPane` (the temp folder elsewhere): one place for
/// every checkout and worktree on the machine.
pub fn dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map_or_else(std::env::temp_dir, PathBuf::from)
        .join("RetroPane")
}

/// Takes the machine-wide lock for this process if it doesn't hold it yet.
pub fn ensure_held() -> Result<(), String> {
    let mut held = HELD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if held.is_none() {
        let wait = if no_wait_requested() {
            Wait::No
        } else {
            Wait::Yes
        };
        *held = Some(acquire(&dir(), &describe_self(), wait, &mut |m| {
            eprintln!("    {m}");
        })?);
    }
    Ok(())
}

/// `--no-wait` anywhere on the command line, or `RP_AXIM_NO_WAIT=1`.
pub fn no_wait_requested() -> bool {
    std::env::args().any(|a| a == NO_WAIT_FLAG)
        || std::env::var("RP_AXIM_NO_WAIT").is_ok_and(|v| v == "1")
}

/// The command-line flag for failing at once on a busy Axim.
pub const NO_WAIT_FLAG: &str = "--no-wait";

/// This process as the holder note names it: the command (the image is in
/// it), the checkout (a worktree's name says which session), the process
/// id (to find or kill it) and an optional `RP_AXIM_WHO` label.
fn describe_self() -> String {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cwd = std::env::current_dir().map_or_else(|_| "?".into(), |p| p.display().to_string());
    let who = std::env::var("RP_AXIM_WHO").unwrap_or_default();
    format!(
        "cmd=cargo xtask {}\npid={}\ncwd={cwd}\nwho={who}\n",
        args.join(" "),
        std::process::id()
    )
}

/// Takes the lock in `dir`, writing `who` (plus the time) as the holder
/// note. A busy lock is waited for (`report` gets "device busy: ..."
/// lines) or, with [`Wait::No`], refused with the same text.
pub fn acquire(
    dir: &Path,
    who: &str,
    wait: Wait,
    report: &mut dyn FnMut(&str),
) -> Result<Held, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join("axim.lock");
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let holder = dir.join("axim.holder");
    let start = Instant::now();
    let mut shown = String::new();
    let mut reminded = start;
    loop {
        match file.try_lock() {
            Ok(()) => break,
            Err(std::fs::TryLockError::WouldBlock) => {}
            Err(std::fs::TryLockError::Error(e)) => {
                return Err(format!("{}: {e}", path.display()));
            }
        }
        let note = busy_line(&holder);
        if wait == Wait::No {
            return Err(format!("{note}; not waiting ({NO_WAIT_FLAG})"));
        }
        if note != shown {
            report(&format!("{note}, waiting"));
            shown = note;
            reminded = Instant::now();
        } else if reminded.elapsed() >= REMIND {
            report(&format!(
                "still waiting for the Axim ({} s)",
                start.elapsed().as_secs()
            ));
            reminded = Instant::now();
        }
        std::thread::sleep(POLL);
    }
    let since = now_secs();
    std::fs::write(&holder, format!("{who}since={since}\n"))
        .map_err(|e| format!("{}: {e}", holder.display()))?;
    Ok(Held {
        _file: file,
        holder,
    })
}

/// "device busy: <command> (pid, checkout) since <time>", from the note.
fn busy_line(holder: &Path) -> String {
    match std::fs::read_to_string(holder) {
        Ok(text) => format!("device busy: {}", describe_note(&text, now_secs())),
        // Between the holder taking the lock and writing its note.
        Err(e) if e.kind() == ErrorKind::NotFound => {
            "device busy: (no holder note: an older xtask build, or one just starting)".into()
        }
        Err(_) => "device busy: (holder note unreadable)".into(),
    }
}

/// A holder note as one line.
pub fn describe_note(text: &str, now: u64) -> String {
    let field = |k: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(k)?.strip_prefix('='))
            .unwrap_or("")
    };
    let mut s = field("cmd").to_string();
    if s.is_empty() {
        s = "(unknown command)".into();
    }
    let who = field("who");
    if !who.is_empty() {
        s = format!("{who}: {s}");
    }
    // The note is written on Windows: split on both separators, so it reads
    // the same on every host (Path only knows its own host's separator).
    let place = field("cwd")
        .rsplit(['/', '\\'])
        .find(|n| !n.is_empty())
        .map(|n| format!(", in {n}"))
        .unwrap_or_default();
    s += &format!(" (pid {}{place})", field("pid"));
    if let Ok(since) = field("since").parse::<u64>() {
        s += &format!(
            " since {} ({} ago)",
            utc_clock(since),
            ago(now.saturating_sub(since))
        );
    }
    s
}

/// `cargo xtask axim`: who holds the Axim, if anyone.
pub fn status(dir: &Path) -> Result<String, String> {
    let path = dir.join("axim.lock");
    let file = match File::options().write(true).open(&path) {
        Ok(f) => f,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok("the Axim is free".into()),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    match file.try_lock() {
        Ok(()) => Ok("the Axim is free".into()),
        Err(std::fs::TryLockError::WouldBlock) => Ok(busy_line(&dir.join("axim.holder"))),
        Err(std::fs::TryLockError::Error(e)) => Err(format!("{}: {e}", path.display())),
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// The emulator's sibling of the Axim lock: a block of `len` local TCP
/// ports for one process's emulator runs (serial, monitor, QMP, USB...).
/// Two `cargo xtask emu*` runs on fixed ports collided on 2026-10-15
/// ("Address already in use"). Each block is claimed with an OS lock on
/// `emu-ports/<first>.lock`, so no two xtask processes share one and a
/// killed one frees its block; a block with a port some other program
/// holds (`is_free`, e.g. a QEMU that outlived its xtask) is skipped.
/// Returns the first port and the lock to keep for the process's life.
pub fn reserve_ports(
    dir: &Path,
    first: u16,
    len: u16,
    blocks: u16,
    is_free: &dyn Fn(u16) -> bool,
) -> Result<(u16, File), String> {
    let dir = dir.join("emu-ports");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    // Start at a different block in each process, so concurrent runs don't
    // all race for the first one.
    let offset = u16::try_from(std::process::id() % u32::from(blocks)).unwrap_or(0);
    for i in 0..blocks {
        let start = first + (i + offset) % blocks * len;
        let path = dir.join(format!("{start}.lock"));
        let file = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if file.try_lock().is_ok() && (start..start + len).all(is_free) {
            return Ok((start, file));
        }
    }
    Err(format!(
        "no free block of {len} emulator ports in {first}..{} (another {blocks} runs at once?)",
        first + blocks * len
    ))
}

/// Whether nothing listens on local `port` (for [`reserve_ports`]).
pub fn port_is_free(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}

fn ago(secs: u64) -> String {
    match secs {
        0..60 => format!("{secs} s"),
        60..3600 => format!("{} min {} s", secs / 60, secs % 60),
        _ => format!("{} h {} min", secs / 3600, secs % 3600 / 60),
    }
}

/// "2026-10-15 19:41:03 UTC" (no time-zone data in std).
fn utc_clock(secs: u64) -> String {
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // Days to a civil date (H. Hinnant's algorithm).
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + u64::from(m <= 2);
    format!(
        "{y}-{m:02}-{d:02} {:02}:{:02}:{:02} UTC",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rp-devlock-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn second_caller_waits_until_the_first_is_done() {
        let dir = temp_dir("wait");
        let first = acquire(
            &dir,
            "cmd=cargo xtask load rp-a\npid=1\n",
            Wait::Yes,
            &mut |_| {},
        )
        .expect("first");
        let (tx, rx) = mpsc::channel();
        let d2 = dir.clone();
        let second = std::thread::spawn(move || {
            let mut said = Vec::new();
            let held = acquire(
                &d2,
                "cmd=cargo xtask load rp-b\npid=2\n",
                Wait::Yes,
                &mut |m| {
                    said.push(m.to_string());
                },
            );
            tx.send(()).unwrap();
            (held.map(|_| ()), said)
        });
        // Still waiting while the first holds it.
        assert!(rx.recv_timeout(Duration::from_millis(800)).is_err());
        drop(first);
        rx.recv_timeout(Duration::from_secs(5))
            .expect("the second caller gets the lock once the first lets go");
        let (held, said) = second.join().unwrap();
        held.expect("second");
        assert!(
            said.iter().any(
                |m| m.starts_with("device busy: cargo xtask load rp-a (pid 1")
                    && m.ends_with(", waiting")
            ),
            "{said:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_wait_fails_at_once_naming_the_holder() {
        let dir = temp_dir("nowait");
        let _first = acquire(
            &dir,
            "cmd=cargo xtask put x\npid=7\n",
            Wait::Yes,
            &mut |_| {},
        )
        .expect("first");
        let t = Instant::now();
        let err = acquire(&dir, "cmd=cargo xtask ls\n", Wait::No, &mut |_| {})
            .err()
            .expect("busy");
        assert!(t.elapsed() < Duration::from_secs(2));
        assert!(
            err.starts_with("device busy: cargo xtask put x (pid 7"),
            "{err}"
        );
        assert!(err.ends_with("not waiting (--no-wait)"), "{err}");
        assert!(
            status(&dir)
                .unwrap()
                .starts_with("device busy: cargo xtask put x")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_free_lock_is_free_even_with_a_stale_note() {
        let dir = temp_dir("stale");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("axim.lock"), "").unwrap();
        std::fs::write(dir.join("axim.holder"), "cmd=cargo xtask load crashed\n").unwrap();
        assert_eq!(status(&dir).unwrap(), "the Axim is free");
        acquire(&dir, "cmd=x\n", Wait::No, &mut |_| {}).expect("a leftover note never blocks");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The child half of the crash test: holds the lock in its own process
    /// until killed. Does nothing unless the parent test started it.
    #[test]
    #[ignore = "started by a_killed_holder_releases_the_lock"]
    fn child_holds_the_lock() {
        let Some(dir) = std::env::var_os("RP_DEVLOCK_CHILD_DIR") else {
            return;
        };
        let _held = acquire(
            Path::new(&dir),
            "cmd=child\npid=0\n",
            Wait::Yes,
            &mut |_| {},
        )
        .expect("child");
        std::thread::sleep(Duration::from_secs(60));
    }

    #[test]
    fn a_killed_holder_releases_the_lock() {
        let dir = temp_dir("killed");
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "devlock::tests::child_holds_the_lock",
                "--ignored",
                "--nocapture",
            ])
            .env("RP_DEVLOCK_CHILD_DIR", &dir)
            .stdout(std::process::Stdio::null())
            .spawn()
            .expect("child");
        // Wait for the child to hold it (another process, so a real OS lock).
        let t = Instant::now();
        while !status(&dir).is_ok_and(|s| s.starts_with("device busy: child")) {
            assert!(
                t.elapsed() < Duration::from_secs(20),
                "the child never took the lock"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(acquire(&dir, "cmd=x\n", Wait::No, &mut |_| {}).is_err());
        // Killed without any clean-up of its own: the OS must release it.
        child.kill().unwrap();
        child.wait().unwrap();
        let t = Instant::now();
        loop {
            match acquire(&dir, "cmd=x\n", Wait::No, &mut |_| {}) {
                Ok(_) => break,
                Err(e) => assert!(t.elapsed() < Duration::from_secs(5), "still held: {e}"),
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn notes_read_as_one_line() {
        let note = "cmd=cargo xtask load rp-mbx3d-api\npid=4242\n\
                    cwd=C:\\r\\.claude\\worktrees\\mbx3d-render\nwho=\nsince=1792093263\n";
        assert_eq!(
            describe_note(note, 1_792_093_263 + 130),
            "cargo xtask load rp-mbx3d-api (pid 4242, in mbx3d-render) \
             since 2026-10-15 19:41:03 UTC (2 min 10 s ago)"
        );
        assert!(describe_note("who=rng loop\ncmd=c\n", 0).starts_with("rng loop: c"));
    }

    /// The class guard: nothing in xtask may reach the device except
    /// through `usb::powershell`, which takes the lock first. A new path
    /// that spawns PowerShell or opens a serial port elsewhere fails here.
    #[test]
    fn only_usb_powershell_reaches_the_device() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        for entry in std::fs::read_dir(&src).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if name == "devlock.rs" || path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            let spawns = text.matches("Command::new(\"powershell").count()
                + text.matches("Command::new(\"pwsh").count();
            let ports = text.matches("System.IO.Ports").count();
            if name == "usb.rs" {
                assert_eq!(spawns, 1, "usb.rs: PowerShell only from fn powershell");
                let body = text.split("fn powershell(").nth(1).unwrap();
                let body = &body[..body.find("\n}").unwrap()];
                assert!(
                    body.contains("crate::devlock::ensure_held()?"),
                    "fn powershell must take the Axim lock before anything else"
                );
            } else if name == "spot.rs" {
                // The colorimeter's pseudo-console host: once, after the lock.
                assert_eq!((spawns, ports), (1, 0), "spot.rs: one PowerShell host");
                let body = text.split("pub fn open(").nth(1).unwrap();
                let lock = body.find("crate::devlock::ensure_held()?").unwrap();
                assert!(lock < body.find("Command::new(\"powershell").unwrap());
            } else {
                assert_eq!(
                    spawns + ports,
                    0,
                    "{name}: talks to the device without the lock"
                );
            }
        }
    }

    #[test]
    fn concurrent_emulator_runs_get_their_own_ports() {
        let dir = temp_dir("ports");
        let free = |_| true;
        let (a, _la) = reserve_ports(&dir, 50_000, 16, 4, &free).expect("first");
        let (b, _lb) = reserve_ports(&dir, 50_000, 16, 4, &free).expect("second");
        assert_ne!(a, b);
        for s in [a, b] {
            assert!(
                (50_000..50_064).contains(&s) && (s - 50_000) % 16 == 0,
                "{s}"
            );
        }
        // A block with one port held by some other program is skipped.
        let (c, _lc) =
            reserve_ports(&dir, 50_000, 16, 4, &|p| p != a + 20 && p != b + 20).expect("third");
        assert!(![a, b].contains(&c) && !(c..c + 16).any(|p| p == a + 20 || p == b + 20));
        let (d, ld) = reserve_ports(&dir, 50_000, 16, 4, &free).expect("fourth");
        assert!(
            reserve_ports(&dir, 50_000, 16, 4, &free).is_err(),
            "all four taken"
        );
        // A finished (or killed) run's block is free again.
        drop(ld);
        assert_eq!(
            reserve_ports(&dir, 50_000, 16, 4, &free).expect("again").0,
            d
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clock_and_ages() {
        assert_eq!(utc_clock(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(utc_clock(951_782_400), "2000-02-29 00:00:00 UTC");
        assert_eq!(ago(59), "59 s");
        assert_eq!(ago(3_725), "1 h 2 min");
    }
}
