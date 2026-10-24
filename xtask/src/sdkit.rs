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

/// HaRET's start-up script. HaRET puts the kernel 32 KiB into RAM, where our
/// images are linked (boot/link.x). Its machine list has the X50/X51 (RAM at
/// 0xA8000000), but on a real X51v it fell back to 0xA0000000, where there
/// is no RAM: it copied the image into nothing and hung. So the script sets
/// the RAM start itself.
const DEFAULT_TXT: &str = "\
# RetroPane 26 SD boot (HaRET script)
set ramaddr 0xa8000000
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

Other images: copy one folder's retropane.bin over the one next to haret.exe.

- light/  First light: fills the screen with a colour after each startup
          step, so a hang shows where it stopped.
- usb/    USB link test: plug the sync cable or cradle into the PC first.
          The PC finds a COM port (no driver needed); on the PC run
          `cargo xtask usb`. The yellow lines at the top show what the PC
          has done (resets, setups, configured, open, bytes).
- loader/ Resident loader: start it once with HaRET, cable plugged in; then
          every `cargo xtask load <image>` on the PC sends a new image over
          USB and starts it. No more copying files. An image that runs past
          30 s without checking in comes back to the loader with a crash
          report, even one spinning with interrupts off. Only an image that
          never sets up RetroPane's runtime is reset by the watchdog (30 s
          later), which lands on Dell's restore prompt: press Mail to
          cancel, never Contacts. `cargo xtask load` refuses images that
          never return.
          It also updates the card: `cargo xtask put <file>` replaces a file
          in the card's root directory (verified by reading it back), and
          `cargo xtask install` puts this kit's loader, startup.txt, default.txt
          and haret.exe there in one go. Soft-reset afterwards to start the
          new loader. Files not yet on the card are reported, not created.
          Optional, for runs that need nobody at the device:
          * startup.txt: copy it next to haret.exe and HaRET boots by itself
            when started (no \"Run\" tap). Delete it to get the menu back.
          * RetroPane loader.lnk: copy it to \\Windows\\StartUp and Windows
            Mobile starts HaRET (and so the loader) at every boot, including
            after a watchdog reset. To stop: delete it from \\Windows\\StartUp,
            or just take the SD card out (then Windows Mobile boots normally).
- cards/  The cards' drive icons on the PC (by Juanje, puntodepartida.com,
          used with permission). `cargo xtask install` puts the sd/ pair on
          the SD card; copy the cf/ pair to the CF card's root yourself.
";

/// A Windows CE shortcut: the length of what follows `#`, then the command.
fn shortcut(target: &str) -> String {
    let cmd = format!("\"{target}\"");
    format!("{}#{cmd}", cmd.chars().count())
}

/// A card's `autorun.inf`: only the drive icon (CRLF, as Windows writes it).
fn autorun_inf(icon: &str) -> String {
    format!("[autorun]\r\nicon={icon}\r\n")
}

/// Builds the kit into `target/sdkit`.
pub fn build(root: &Path) -> Result<(), String> {
    let kit = root.join("target").join("sdkit");
    std::fs::create_dir_all(&kit).map_err(|e| format!("{}: {e}", kit.display()))?;

    let elf = crate::arm_release(root).join("rp-probe");
    let bin = kit.join("retropane.bin");
    crate::emu::run(
        Command::new(objcopy())
            .args(["-O", "binary"])
            .arg(&elf)
            .arg(&bin),
    )?;
    let size = std::fs::metadata(&bin).map_err(|e| e.to_string())?.len();
    eprintln!("    retropane.bin: {size} bytes (raw image for 0xA8008000)");

    // Other images, each swapped in as retropane.bin from its own folder:
    // first light (boot/light), for when the probe shows nothing, the USB
    // link test (boot/usb) and the resident loader (boot/loader), ADR-0012.
    for (dir, image) in [
        ("light", "rp-light"),
        ("usb", "rp-usb"),
        ("loader", "rp-loader"),
    ] {
        let out = kit.join(dir);
        std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
        crate::emu::run(
            Command::new(objcopy())
                .args(["-O", "binary"])
                .arg(crate::arm_release(root).join(image))
                .arg(out.join("retropane.bin")),
        )?;
    }

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

    for (name, text) in [
        ("default.txt", DEFAULT_TXT.to_string()),
        ("README.txt", README.to_string()),
        // Loader mode: HaRET runs startup.txt by itself when it starts.
        ("loader/startup.txt", DEFAULT_TXT.to_string()),
        (
            "loader/RetroPane loader.lnk",
            shortcut(r"\Storage Card\haret.exe"),
        ),
    ] {
        // Windows Mobile tools expect CRLF.
        std::fs::write(kit.join(name), text.replace('\n', "\r\n"))
            .map_err(|e| format!("{name}: {e}"))?;
    }

    // The cards' drive icons (assets/cards, used with the author's
    // permission): each card's root gets its icon and an autorun.inf naming
    // it, which is all Windows reads from a removable drive's autorun.inf.
    for (dir, icon) in [("sd", "card-sd.ico"), ("cf", "card-compact-flash.ico")] {
        let out = kit.join("cards").join(dir);
        std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
        std::fs::copy(root.join("assets/cards").join(icon), out.join(icon))
            .map_err(|e| format!("{icon}: {e}"))?;
        std::fs::write(out.join("autorun.inf"), autorun_inf(icon))
            .map_err(|e| format!("cards/{dir}/autorun.inf: {e}"))?;
    }
    eprintln!("    SD kit ready: {}", kit.display());
    Ok(())
}

/// Writes `elf` as a raw binary (what HaRET and the loader start) to `out`.
pub fn raw_binary(elf: &Path, out: &Path) -> Result<(), String> {
    crate::emu::run(
        Command::new(objcopy())
            .args(["-O", "binary"])
            .arg(elf)
            .arg(out),
    )
}

/// `llvm-objcopy` from the toolchain's `llvm-tools` component.
pub(crate) fn objcopy() -> std::path::PathBuf {
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
    fn autorun_names_the_icon_only() {
        // The bytes of the file already on the owner's SD card.
        assert_eq!(
            autorun_inf("card-sd.ico"),
            "[autorun]\r\nicon=card-sd.ico\r\n"
        );
    }

    #[test]
    fn shortcuts_count_the_characters_after_the_hash() {
        assert_eq!(
            shortcut(r"\Storage Card\haret.exe"),
            r#"25#"\Storage Card\haret.exe""#
        );
    }

    #[test]
    fn script_boots_our_image() {
        assert!(DEFAULT_TXT.contains("set kernel retropane.bin"));
        // The X51v's RAM: HaRET's own guess (0xA0000000) is empty there.
        assert!(DEFAULT_TXT.contains("set ramaddr 0xa8000000"));
        assert!(DEFAULT_TXT.trim_end().ends_with("bootlinux"));
    }
}
