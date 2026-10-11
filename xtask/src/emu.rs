//! `cargo xtask emu` / `cargo xtask qemu`: the x51v emulator.
//!
//! Which emulator `emu` uses, in order:
//!
//! 1. `RETROPANE_QEMU=<path>`: that binary. `RETROPANE_QEMU=wsl` forces the WSL build.
//! 2. On Windows, a local build made by `cargo xtask qemu` inside WSL, if present.
//!    This is the fast loop for emulator development.
//! 3. Otherwise the pinned retropane-qemu GitHub release, downloaded on first use,
//!    so contributors never have to build QEMU.
//!
//! `cargo xtask qemu` syncs the retropane-qemu checkout next to this repository
//! (`RETROPANE_QEMU_SRC` overrides) into WSL (`RETROPANE_WSL_DISTRO`, default
//! `Ubuntu`) and builds it there in under a minute.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Emulator release this checkout is tested against. Bump deliberately.
pub const QEMU_RELEASE: &str = "rp-v0.4.0";
const RELEASE_URL: &str = "https://github.com/Drakey83/retropane-qemu/releases/download";

/// How long an image gets to print its success marker.
const BOOT_TIMEOUT: Duration = Duration::from_secs(20);

/// Must match `rp_hello::MARKER`.
const HELLO_MARKER: &str = "RETROPANE-HELLO-OK";

/// Where `cargo xtask qemu` builds inside WSL.
const WSL_QEMU: &str = "$HOME/retropane-qemu/build/qemu-system-arm";

const EMU_ARGS: &str = "-M x51v -display none -serial stdio";

/// Local TCP port for QEMU's monitor (screenshots). `RETROPANE_MONITOR_PORT` overrides.
fn monitor_port() -> u16 {
    std::env::var("RETROPANE_MONITOR_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(45_455)
}

struct Platform {
    asset: &'static str,
    dir: &'static str,
    exe: &'static str,
}

const fn platform() -> Platform {
    if cfg!(windows) {
        Platform {
            asset: "retropane-qemu-windows-x86_64.zip",
            dir: "retropane-qemu-windows-x86_64",
            exe: "qemu-system-arm.exe",
        }
    } else {
        Platform {
            asset: "retropane-qemu-linux-x86_64.tar.gz",
            dir: "retropane-qemu-linux-x86_64",
            exe: "qemu-system-arm",
        }
    }
}

/// The emulator to run.
enum Emulator {
    /// A native executable.
    Native(PathBuf),
    /// The local build inside WSL.
    Wsl,
}

fn wsl_distro() -> String {
    std::env::var("RETROPANE_WSL_DISTRO").unwrap_or_else(|_| "Ubuntu".into())
}

/// `C:\x\y` (or `\\?\C:\x\y`) -> `/mnt/c/x/y`.
fn to_wsl_path(p: &Path) -> String {
    let s = p.to_string_lossy();
    let s = s.trim_start_matches(r"\\?\").replace('\\', "/");
    match s.as_bytes() {
        [d, b':', ..] => format!("/mnt/{}{}", (*d as char).to_ascii_lowercase(), &s[2..]),
        _ => s,
    }
}

fn wsl_bash(script: &str) -> Command {
    let mut c = Command::new("wsl");
    c.args(["-d", &wsl_distro(), "--", "bash", "-lc", script]);
    c
}

fn wsl_build_exists() -> bool {
    cfg!(windows)
        && wsl_bash(&format!("test -x {WSL_QEMU}"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
}

fn emulator(root: &Path) -> Result<Emulator, String> {
    match std::env::var("RETROPANE_QEMU") {
        Ok(v) if v.eq_ignore_ascii_case("wsl") => return Ok(Emulator::Wsl),
        Ok(v) => return Ok(Emulator::Native(PathBuf::from(v))),
        Err(_) => {}
    }
    if wsl_build_exists() {
        return Ok(Emulator::Wsl);
    }
    download_release(root).map(Emulator::Native)
}

/// `cargo xtask qemu`: syncs the retropane-qemu checkout into WSL and builds it.
pub fn build_wsl(root: &Path) -> Result<(), String> {
    let src = std::env::var_os("RETROPANE_QEMU_SRC")
        .map_or_else(|| root.join("..").join("retropane-qemu"), PathBuf::from);
    let src = src
        .canonicalize()
        .map_err(|e| format!("{}: {e}", src.display()))?;
    let script = root.join("xtask").join("scripts").join("qemu-wsl-build.sh");
    run(Command::new("wsl")
        .args(["-d", &wsl_distro(), "--", "bash"])
        .arg(to_wsl_path(&script))
        .arg(to_wsl_path(&src)))
}

/// Downloads the pinned emulator release if necessary.
fn download_release(root: &Path) -> Result<PathBuf, String> {
    let p = platform();
    let cache = root.join("target").join("emulator").join(QEMU_RELEASE);
    let exe = cache.join(p.dir).join(p.exe);
    if exe.exists() {
        return Ok(exe);
    }
    std::fs::create_dir_all(&cache).map_err(|e| e.to_string())?;
    let archive = cache.join(p.asset);
    let url = format!("{RELEASE_URL}/{QEMU_RELEASE}/{}", p.asset);
    eprintln!("    downloading emulator {QEMU_RELEASE} ...");
    run(Command::new("curl")
        .args(["-fsSL", "-o"])
        .arg(&archive)
        .arg(&url))?;
    // bsdtar (Windows 10+) and GNU tar both handle .zip / .tar.gz here.
    run(Command::new("tar")
        .arg("-xf")
        .arg(&archive)
        .arg("-C")
        .arg(&cache))?;
    if exe.exists() {
        Ok(exe)
    } else {
        Err(format!(
            "emulator archive did not contain {}",
            exe.display()
        ))
    }
}

fn run(cmd: &mut Command) -> Result<(), String> {
    let status = cmd.status().map_err(|e| format!("{cmd:?}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{cmd:?} failed"))
    }
}

/// A scripted interaction after boot: send a monitor command, then wait for
/// serial output containing each `expect` string.
struct Step {
    command: &'static str,
    expect: &'static [&'static str],
}

/// Button presses checked after the hello image boots (QEMU `sendkey` names;
/// see docs/hw/keypad.md for the emulator's key mapping).
const HELLO_STEPS: &[Step] = &[
    Step {
        command: "sendkey up",
        expect: &["KEY Up down", "KEY Up up"],
    },
    Step {
        command: "sendkey ret",
        expect: &["KEY Enter down", "KEY Enter up"],
    },
    Step {
        command: "sendkey f1",
        expect: &["KEY Calendar down", "KEY Calendar up"],
    },
    Step {
        command: "sendkey f4",
        expect: &["KEY Home down", "KEY Home up"],
    },
];

/// Boots the bring-up image, checks it reports success over the UART,
/// screenshots the emulated display and verifies the test pattern, then
/// presses buttons and checks the image reports them.
pub fn boot_hello(root: &Path) -> Result<(), String> {
    let image = root.join("target/armv5te-x51v-none-eabi/release/rp-hello");
    let shot = root
        .join("target")
        .join("emulator")
        .join("hello-screen.ppm");
    let output = boot(root, &image, HELLO_MARKER, Some(&shot), HELLO_STEPS)?;
    eprint!("{output}");
    if !output.ends_with('\n') {
        eprintln!();
    }
    eprintln!("    emulator boot OK");
    let ppm = std::fs::read(&shot).map_err(|e| format!("{}: {e}", shot.display()))?;
    crate::screen::check_test_pattern(&ppm)?;
    eprintln!("    display OK: test pattern verified ({})", shot.display());
    eprintln!(
        "    input OK: {} button presses reported",
        HELLO_STEPS.len()
    );
    Ok(())
}

/// Serial output collected from the emulator, with a deadline.
struct Serial {
    rx: mpsc::Receiver<Vec<u8>>,
    output: String,
    deadline: Instant,
}

impl Serial {
    /// Waits until the output (from byte `from` onwards) contains `text`.
    fn wait_for(&mut self, text: &str, from: usize) -> bool {
        loop {
            if self.output.get(from..).is_some_and(|o| o.contains(text)) {
                return true;
            }
            let Some(left) = self.deadline.checked_duration_since(Instant::now()) else {
                return false;
            };
            match self.rx.recv_timeout(left) {
                Ok(chunk) => self.output.push_str(&String::from_utf8_lossy(&chunk)),
                Err(_) => return false,
            }
        }
    }
}

/// Boots `image`, waits until its serial output contains `marker`, optionally
/// takes a screenshot, then runs `steps`. Returns the serial output.
fn boot(
    root: &Path,
    image: &Path,
    marker: &str,
    screenshot: Option<&Path>,
    steps: &[Step],
) -> Result<String, String> {
    let port = monitor_port();
    let monitor = format!("tcp:127.0.0.1:{port},server,nowait");
    let emu = emulator(root)?;
    let wsl = matches!(emu, Emulator::Wsl);
    let mut cmd = match emu {
        Emulator::Native(exe) => {
            eprintln!("    emulator: {}", exe.display());
            let mut c = Command::new(exe);
            c.args(EMU_ARGS.split(' '))
                .args(["-monitor", &monitor])
                .arg("-kernel")
                .arg(image);
            c
        }
        Emulator::Wsl => {
            eprintln!("    emulator: local WSL build");
            // `timeout` guarantees the Linux process exits even if killing
            // wsl.exe doesn't reach it.
            wsl_bash(&format!(
                "exec timeout {} {WSL_QEMU} {EMU_ARGS} -monitor {monitor} -kernel '{}'",
                BOOT_TIMEOUT.as_secs() + 10,
                to_wsl_path(image)
            ))
        }
    };
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("failed to start emulator: {e}"))?;

    // Stream stdout through a channel so we can enforce a timeout.
    let mut stdout = child.stdout.take().expect("piped");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = [0u8; 256];
        while let Ok(n) = stdout.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });

    let mut serial = Serial {
        rx,
        output: String::new(),
        deadline: Instant::now() + BOOT_TIMEOUT,
    };
    let result = run_session(&mut serial, marker, port, wsl, screenshot, steps);
    let _ = child.kill();
    let _ = child.wait();
    match result {
        Ok(()) => Ok(serial.output),
        Err(e) => Err(format!("{e}. Serial output so far:\n{}", serial.output)),
    }
}

fn run_session(
    serial: &mut Serial,
    marker: &str,
    port: u16,
    wsl: bool,
    screenshot: Option<&Path>,
    steps: &[Step],
) -> Result<(), String> {
    if !serial.wait_for(marker, 0) {
        return Err(format!("no `{marker}` within {}s", BOOT_TIMEOUT.as_secs()));
    }
    if screenshot.is_none() && steps.is_empty() {
        return Ok(());
    }
    let mut mon = crate::screen::Monitor::connect(port)?;
    if let Some(path) = screenshot {
        let _ = std::fs::remove_file(path);
        let guest = if wsl {
            to_wsl_path(path)
        } else {
            path.display().to_string()
        };
        mon.screendump(&guest, path)?;
    }
    for step in steps {
        let from = serial.output.len();
        mon.command(step.command)?;
        for want in step.expect {
            if !serial.wait_for(want, from) {
                return Err(format!("after `{}`: no `{want}` on serial", step.command));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_paths_map_to_wsl_mounts() {
        assert_eq!(
            to_wsl_path(Path::new(r"C:\Users\x\a b")),
            "/mnt/c/Users/x/a b"
        );
        assert_eq!(to_wsl_path(Path::new(r"\\?\D:\r")), "/mnt/d/r");
        assert_eq!(to_wsl_path(Path::new("/already/unix")), "/already/unix");
    }
}
