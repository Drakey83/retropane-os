//! `cargo xtask sdkit`: everything needed to boot RetroPane on a real X51v
//! from its SD card, in `target/sdkit/`.
//!
//! - `haret.exe`: HaRET, built from pinned sources in WSL
//!   (`scripts/haret-wsl-build.sh`). It runs as a normal Windows Mobile
//!   program, copies our image to 0xA8008000, switches the MMU off and jumps
//!   to it. Nothing is written to flash: a soft reset returns to Windows
//!   Mobile.
//! - `default.txt`: the HaRET script it runs on start.
//! - `retropane.bin`: the probe image (`rp-probe`) as a raw binary: it shows
//!   everything RetroPane needs to learn from the device on screen, in pages
//!   for a phone photo.
//! - `README.txt`: the steps for the device.

use std::path::Path;
use std::process::Command;

use crate::emu::{to_wsl_path, wsl_distro};

/// HaRET's start-up script. HaRET already knows the X51v ("AximX50": RAM at
/// 0xA8000000, 64 MiB), and puts the kernel 32 KiB into RAM, where our
/// images are linked (boot/link.x).
const DEFAULT_TXT: &str = "\
# RetroPane 26 SD boot (HaRET script)
set kernel retropane.bin
set cmdline \"\"
bootlinux
";

const README: &str = "\
RetroPane 26 - SD card probe kit
================================

What this does: HaRET (a Windows Mobile program) loads RetroPane's hardware
probe into RAM and starts it. The probe reads how the device is set up, runs
benchmarks, and shows everything on screen in 4 pages. Nothing is written to
the device's flash. To get back to Windows Mobile, soft-reset (the reset
button on the bottom edge). Save any open work first: a soft reset closes
running programs.

1. Copy haret.exe, default.txt and retropane.bin to the root of the SD card
   (or one folder; keep the three files together).
2. Put the card in the X51v. On the device, open File Explorer, go to
   Storage Card, and tap haret.exe.
3. Tap \"Run\". HaRET shows its boot messages, then RetroPane shows progress
   lines (\"benchmarks...\" takes a few seconds), then page 1 of the report.
4. Photograph each page: page 1, then press the D-pad right (or tap the
   screen) for pages 2, 3 and 4. The page number is in the bottom line. Hold
   the phone square to the screen and close enough that the text fills the
   photo.
5. Soft-reset to return to Windows Mobile.

Every line ends in a two-digit checksum, so misread digits get caught. If it
stops early, photograph whatever is on screen: the last line says how far it
got.
";

/// Builds the kit into `target/sdkit`.
pub fn build(root: &Path) -> Result<(), String> {
    let kit = root.join("target").join("sdkit");
    std::fs::create_dir_all(&kit).map_err(|e| format!("{}: {e}", kit.display()))?;

    let elf = root.join("target/armv5te-x51v-none-eabi/release/rp-probe");
    let bin = kit.join("retropane.bin");
    crate::emu::run(
        Command::new(objcopy())
            .args(["-O", "binary"])
            .arg(&elf)
            .arg(&bin),
    )?;
    let size = std::fs::metadata(&bin).map_err(|e| e.to_string())?.len();
    eprintln!("    retropane.bin: {size} bytes (raw image for 0xA8008000)");

    // The first-light image, for when the probe shows nothing: swap it in as
    // retropane.bin (boot/light).
    let light = kit.join("light");
    std::fs::create_dir_all(&light).map_err(|e| format!("{}: {e}", light.display()))?;
    crate::emu::run(
        Command::new(objcopy())
            .args(["-O", "binary"])
            .arg(root.join("target/armv5te-x51v-none-eabi/release/rp-light"))
            .arg(light.join("retropane.bin")),
    )?;

    let script = root
        .join("xtask")
        .join("scripts")
        .join("haret-wsl-build.sh");
    crate::emu::run(
        Command::new("wsl")
            .args(["-d", &wsl_distro(), "--", "bash"])
            .arg(to_wsl_path(&script))
            .arg(to_wsl_path(&kit)),
    )?;

    for (name, text) in [("default.txt", DEFAULT_TXT), ("README.txt", README)] {
        // Windows Mobile tools expect CRLF.
        std::fs::write(kit.join(name), text.replace('\n', "\r\n"))
            .map_err(|e| format!("{name}: {e}"))?;
    }
    eprintln!("    SD kit ready: {}", kit.display());
    Ok(())
}

/// `llvm-objcopy` from the toolchain's `llvm-tools` component.
fn objcopy() -> std::path::PathBuf {
    let sysroot = Command::new("rustc")
        .args(["--print", "sysroot"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    let host = Command::new("rustc")
        .args(["-vV"])
        .output()
        .ok()
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .find_map(|l| l.strip_prefix("host: ").map(str::to_string))
        })
        .unwrap_or_default();
    let exe = if cfg!(windows) {
        "llvm-objcopy.exe"
    } else {
        "llvm-objcopy"
    };
    Path::new(&sysroot)
        .join("lib/rustlib")
        .join(host)
        .join("bin")
        .join(exe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_boots_our_image() {
        assert!(DEFAULT_TXT.contains("set kernel retropane.bin"));
        assert!(DEFAULT_TXT.trim_end().ends_with("bootlinux"));
    }
}
