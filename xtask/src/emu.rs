//! `cargo xtask emu`: boots RetroPane images in the x51v emulator.
//!
//! The emulator (our QEMU 9.1 fork, ADR-0002) is downloaded from the
//! retropane-qemu GitHub release on first use, so contributors never build
//! QEMU. Set `RETROPANE_QEMU` to use a local build instead.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Emulator release this checkout is tested against. Bump deliberately.
pub const QEMU_RELEASE: &str = "rp-v0.1.0";
const RELEASE_URL: &str = "https://github.com/Drakey83/retropane-qemu/releases/download";

/// How long an image gets to print its success marker.
const BOOT_TIMEOUT: Duration = Duration::from_secs(20);

/// Must match `rp_hello::MARKER`.
const HELLO_MARKER: &str = "RETROPANE-HELLO-OK";

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

/// Finds the emulator, downloading the pinned release if necessary.
fn qemu(root: &Path) -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("RETROPANE_QEMU") {
        return Ok(PathBuf::from(path));
    }
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

/// Boots the bring-up image and checks it reports success over the UART.
pub fn boot_hello(root: &Path) -> Result<(), String> {
    let image = root.join("target/armv5te-x51v-none-eabi/release/rp-hello");
    let output = boot(root, &image, HELLO_MARKER)?;
    eprint!("{output}");
    if !output.ends_with('\n') {
        eprintln!();
    }
    eprintln!("    emulator boot OK");
    Ok(())
}

/// Boots `image` and waits until its serial output contains `marker`.
/// Returns the serial output.
fn boot(root: &Path, image: &Path, marker: &str) -> Result<String, String> {
    let qemu = qemu(root)?;
    let mut child = Command::new(&qemu)
        .args([
            "-M", "x51v", "-display", "none", "-monitor", "none", "-serial", "stdio",
        ])
        .arg("-kernel")
        .arg(image)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("failed to start {}: {e}", qemu.display()))?;

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

    let deadline = Instant::now() + BOOT_TIMEOUT;
    let mut output = String::new();
    let found = loop {
        if output.contains(marker) {
            break true;
        }
        let Some(left) = deadline.checked_duration_since(Instant::now()) else {
            break false;
        };
        match rx.recv_timeout(left) {
            Ok(chunk) => output.push_str(&String::from_utf8_lossy(&chunk)),
            Err(_) => break false,
        }
    };
    let _ = child.kill();
    let _ = child.wait();

    if found {
        Ok(output)
    } else {
        Err(format!(
            "no `{marker}` within {}s. Serial output so far:\n{output}",
            BOOT_TIMEOUT.as_secs()
        ))
    }
}
