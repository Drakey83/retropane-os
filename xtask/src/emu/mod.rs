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

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// The recording voice's buffer (in us) on every `-audiodev wav`: the link
/// pushes samples at its own rate (rp-v0.28.0, `i2s-link-clock`) and the
/// host's audio timer takes them when it runs, every ~30 ms on a Windows
/// PC (2026-10-19), so the voice holds a quarter second rather than the
/// default's few periods, and nothing recorded is dropped.
const WAV_VOICE: &str = ",out.buffer-length=250000";

/// Emulator release this checkout is tested against. Bump deliberately.
pub const QEMU_RELEASE: &str = "rp-v0.31.0";
const RELEASE_URL: &str = "https://github.com/Drakey83/retropane-qemu/releases/download";

/// How long an image gets to print its success marker.
const BOOT_TIMEOUT: Duration = Duration::from_secs(20);

/// Must match `rp_hello::MARKER`.
const HELLO_MARKER: &str = "RETROPANE-HELLO-OK";

/// Where `cargo xtask qemu` builds inside WSL.
const WSL_QEMU_DEFAULT: &str = "$HOME/retropane-qemu/build/qemu-system-arm";

/// Whether a run log shows a crash: any of `rp-image`'s crash lines (a
/// fault, a panic, a failed setup step) or the loader's mark for a run
/// that came back crashed (quality audit C38: one test for every check, so
/// a new kind of crash line can't slip past some of them).
fn crashed(log: &str) -> bool {
    CRASH_MARKS.iter().any(|m| log.contains(m))
}

/// What a crash leaves in a run log. Plain substrings: an image's own
/// text must never hold one (P9e's label "DEFAULT" read as a crash,
/// 2026-10-25; [`labels_hold_no_crash_mark`] checks the step tables).
const CRASH_MARKS: [&str; 4] = ["FAULT", "PANIC", "SETUP-FAIL", "[the image crashed"];

#[test]
fn labels_hold_no_crash_mark() {
    use rp_drv_mbx_display::blendtest::{FILTER_STEPS, FILTER2_STEPS};
    for s in FILTER_STEPS.iter().chain(&FILTER2_STEPS) {
        for text in [s.name, s.expect] {
            assert!(!crashed(text), "{text:?} reads as a crash");
        }
    }
}

/// The WSL emulator binary: `RETROPANE_WSL_QEMU` (a fork branch built in
/// its own directory, without replacing the shared build other sessions
/// use), else `cargo xtask qemu`'s.
fn wsl_qemu() -> String {
    std::env::var("RETROPANE_WSL_QEMU").unwrap_or_else(|_| WSL_QEMU_DEFAULT.into())
}

/// Set to show the emulated screen in a window while a check runs (SDL; in
/// WSL through Windows 11's own Linux windows), for a person to watch what
/// the check looks for. Off by default: checks run without a window.
const SHOW_ENV: &str = "RETROPANE_EMU_SHOW";

/// The machine and its display: none, or a window with [`SHOW_ENV`].
fn emu_args() -> &'static str {
    if std::env::var_os(SHOW_ENV).is_some() {
        "-M x51v -display sdl"
    } else {
        "-M x51v -display none"
    }
}

/// The guest's serial port, on a local TCP socket for every platform (item
/// 61: Windows QEMU never read a stdin pipe). QEMU waits for the harness to
/// connect before the guest starts (`wait=on`), so no early output is lost.
fn serial_port() -> u16 {
    monitor_port() - 1
}

mod drive;
mod mtp;
#[allow(unused_imports)]
pub(crate) use drive::*;
mod card;
#[allow(unused_imports)]
pub(crate) use card::*;
mod usblink;
#[allow(unused_imports)]
pub(crate) use usblink::*;
mod audio;
#[allow(unused_imports)]
pub(crate) use audio::*;
mod loader;
#[allow(unused_imports)]
pub(crate) use loader::*;
mod display;
#[allow(unused_imports)]
pub(crate) use display::*;
mod gpu;
#[allow(unused_imports)]
pub(crate) use gpu::*;
mod gpubench;
#[allow(unused_imports)]
pub(crate) use gpubench::*;
mod today;
#[allow(unused_imports)]
pub(crate) use today::*;
mod perf;
#[allow(unused_imports)]
pub(crate) use perf::*;
mod stacks;
#[allow(unused_imports)]
pub(crate) use stacks::*;
mod radar;
#[allow(unused_imports)]
pub(crate) use radar::*;
mod look;
#[allow(unused_imports)]
pub(crate) use look::*;
pub use mtp::boot_loader_mtp;

/// Local TCP port for QEMU's monitor (screenshots). `RETROPANE_MONITOR_PORT` overrides.
///
/// Every port a run uses is an offset from this one, inside a block of
/// [`PORT_BLOCK`] this process reserved (`devlock::reserve_ports`): serial
/// one below, QMP one above, the USB chardevs after that. Two `emu` runs at
/// once (two sessions) once shared fixed ports and failed.
fn monitor_port() -> u16 {
    // The block's lock lives here until the process ends; the OS frees
    // the block then.
    static BASE: std::sync::OnceLock<(u16, Option<std::fs::File>)> = std::sync::OnceLock::new();
    BASE.get_or_init(|| {
        if let Some(p) = std::env::var("RETROPANE_MONITOR_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
        {
            return (p, None);
        }
        let (start, lock) = crate::devlock::reserve_ports(
            &crate::devlock::dir(),
            PORT_FIRST,
            PORT_BLOCK,
            PORT_BLOCKS,
            &crate::devlock::port_is_free,
        )
        .unwrap_or_else(|e| panic!("emulator ports: {e}"));
        (start + 1, Some(lock))
    })
    .0
}

/// The emulator's port blocks: `PORT_BLOCKS` blocks of `PORT_BLOCK` ports
/// from `PORT_FIRST`. A block holds serial, monitor, QMP and
/// [`USB_PORTS`] USB chardevs.
const PORT_FIRST: u16 = 45_440;
const PORT_BLOCK: u16 = 24;
const PORT_BLOCKS: u16 = 64;
const USB_PORTS: u16 = PORT_BLOCK - 3;

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

pub(crate) fn wsl_distro() -> String {
    std::env::var("RETROPANE_WSL_DISTRO").unwrap_or_else(|_| "Ubuntu".into())
}

/// `C:\x\y` (or `\\?\C:\x\y`) -> `/mnt/c/x/y`.
pub(crate) fn to_wsl_path(p: &Path) -> String {
    let s = p.to_string_lossy();
    let s = s.trim_start_matches(r"\\?\").replace('\\', "/");
    match s.as_bytes() {
        [d, b':', ..] => format!("/mnt/{}{}", (*d as char).to_ascii_lowercase(), &s[2..]),
        _ => s,
    }
}

/// Runs a script in WSL, failing if it fails.
pub(crate) fn wsl_run(script: &str) -> Result<(), String> {
    run(&mut wsl_bash(script))
}

fn wsl_bash(script: &str) -> Command {
    // `wsl.exe` hands its command line through a shell that breaks `$(...)`
    // (bash then refuses the whole script and nothing runs: the clean-up
    // killed no emulator for a night, 2026-10-19). Such a script goes on
    // stdin instead (as `start_load`'s does).
    assert!(
        !script.contains("$("),
        "a `$(...)` in a script for wsl: send it on stdin (see start_load)"
    );
    let mut c = Command::new("wsl");
    c.args(["-d", &wsl_distro(), "--", "bash", "-lc", script]);
    c
}

fn wsl_build_exists() -> bool {
    cfg!(windows)
        && wsl_bash(&format!("test -x {}", wsl_qemu()))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
}

/// The fork's source folder for a WSL build's binary (`<src>/build/
/// qemu-system-arm`), or `None` if it isn't laid out so.
fn wsl_source_dir(bin: &str) -> Option<&str> {
    bin.strip_suffix("/build/qemu-system-arm")
        .filter(|d| !d.is_empty())
}

/// Set to run the checks on a local build that isn't [`QEMU_RELEASE`]
/// (developing the fork itself): its results then say nothing about the
/// release (quality audit C41).
const ANY_EMU_ENV: &str = "RETROPANE_QEMU_ANY";

/// Quality audit C41: a WSL build is used only if its source is exactly
/// [`QEMU_RELEASE`] (`git describe --tags --dirty`, so a build a commit
/// past the tag, or with local changes, is refused too). Checked once a
/// run.
fn check_wsl_release() -> Result<(), String> {
    static CHECKED: std::sync::OnceLock<Result<(), String>> = std::sync::OnceLock::new();
    CHECKED
        .get_or_init(|| {
            let bin = wsl_qemu();
            let Some(dir) = wsl_source_dir(&bin) else {
                return Err(format!(
                    "the WSL emulator {bin} isn't at <source>/build/qemu-system-arm: its release can't be read (quality audit C41); set {ANY_EMU_ENV}=1 to run on it anyway"
                ));
            };
            let out = wsl_bash(&format!("git -C {dir} describe --tags --dirty"))
                .output()
                .map_err(|e| format!("wsl: {e}"))?;
            let described = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if described == QEMU_RELEASE {
                eprintln!("    emulator: local WSL build {bin}, {described}");
                return Ok(());
            }
            let msg = format!(
                "the WSL emulator {bin} is {described:?}, not {QEMU_RELEASE}, the release these checks are written for (quality audit C41): point RETROPANE_WSL_QEMU at a build of exactly {QEMU_RELEASE}, or RETROPANE_QEMU=release to download it"
            );
            if std::env::var_os(ANY_EMU_ENV).is_some() {
                eprintln!("    WARNING: {msg}; running anyway ({ANY_EMU_ENV})");
                Ok(())
            } else {
                Err(format!("{msg}; or {ANY_EMU_ENV}=1 to run anyway (fork work)"))
            }
        })
        .clone()
}

fn emulator(root: &Path) -> Result<Emulator, String> {
    match std::env::var("RETROPANE_QEMU") {
        Ok(v) if v.eq_ignore_ascii_case("wsl") => {
            check_wsl_release()?;
            return Ok(Emulator::Wsl);
        }
        // The published release for this platform (the release check: the
        // build contributors download, not the local one).
        Ok(v) if v.eq_ignore_ascii_case("release") => {
            return download_release(root).map(Emulator::Native);
        }
        Ok(v) => return Ok(Emulator::Native(PathBuf::from(v))),
        Err(_) => {}
    }
    if wsl_build_exists() {
        check_wsl_release()?;
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
    let cache = emulator_dir(root).join(QEMU_RELEASE);
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

pub(crate) fn run(cmd: &mut Command) -> Result<(), String> {
    let status = cmd.status().map_err(|e| format!("{cmd:?}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{cmd:?} failed"))
    }
}

/// What a step does.
enum Action {
    /// A QEMU human-monitor command.
    Hmp(&'static str),
    /// Put the stylus down at screen pixel `(x, y)`.
    TouchDown(u16, u16),
    /// Lift the stylus.
    TouchUp,
}

/// What a step expects on serial afterwards.
enum Expect {
    /// This exact text.
    Text(&'static str),
    /// A `TOUCH x y` line within 2 pixels of `(x, y)`.
    TouchNear(u16, u16),
}

/// A scripted interaction after boot.
struct Step {
    action: Action,
    expect: &'static [Expect],
}

/// Interactions checked after the hello image boots: button presses (QEMU
/// `sendkey` names, see docs/hw/keypad.md) and stylus taps (docs/hw/touch.md).
const HELLO_STEPS: &[Step] = &[
    Step {
        action: Action::Hmp("sendkey up"),
        expect: &[Expect::Text("KEY Up down vk 26"), Expect::Text("KEY Up up")],
    },
    Step {
        action: Action::Hmp("sendkey ret"),
        expect: &[Expect::Text("KEY Enter down"), Expect::Text("KEY Enter up")],
    },
    Step {
        action: Action::Hmp("sendkey f1"),
        expect: &[
            Expect::Text("KEY Calendar down"),
            Expect::Text("KEY Calendar up"),
        ],
    },
    Step {
        action: Action::Hmp("sendkey f4"),
        expect: &[
            Expect::Text("KEY Home down vk c4"),
            Expect::Text("KEY Home up vk c4"),
        ],
    },
    // Held: the D-pad repeats; a program button gets its press-and-hold.
    Step {
        action: Action::Hmp("sendkey down 900"),
        expect: &[
            Expect::Text("KEY Down repeat vk 28"),
            Expect::Text("KEY Down up"),
        ],
    },
    Step {
        action: Action::Hmp("sendkey f2 1500"),
        expect: &[
            Expect::Text("KEY Contacts hold vk c2"),
            Expect::Text("KEY Contacts up after hold"),
        ],
    },
    Step {
        action: Action::TouchDown(120, 160),
        expect: &[Expect::TouchNear(120, 160)],
    },
    Step {
        action: Action::TouchUp,
        expect: &[Expect::Text("TOUCH up")],
    },
    Step {
        action: Action::TouchDown(400, 600),
        expect: &[Expect::TouchNear(400, 600)],
    },
    Step {
        action: Action::TouchUp,
        expect: &[Expect::Text("TOUCH up")],
    },
];

/// The test SD image: 8 MiB (QEMU wants a power of two) with one FAT32 (LBA)
/// partition entry, which the hello image must report.
const SD_IMAGE_BYTES: usize = 8 * 1024 * 1024;
const SD_EXPECT_CARD: &str = "SD: SD card, RCA";
const SD_EXPECT_PARTITION: &str = "SD: partition 1 type 0x0c start 2048 blocks 14336";

/// What the probe reports about its test card's file system: exFAT (an
/// SDXC card's), so the probe covers the exFAT reader on the target; the
/// FAT32 path is covered by the loader's card tests.
const SD_EXPECT_FAT: &str = "exFAT 4096 B/cluster";

/// Writes the test SD image: a FAT32 volume (4 KiB clusters, as on the
/// owner's card) holding a small `retropane.bin`, so the probe's root
/// listing has something to show.
fn write_sd_test_image(path: &Path) -> Result<PathBuf, String> {
    let bin: Vec<u8> = (0..1500u32).map(|i| i as u8).collect();
    let img = rp_fat::format::build(&rp_fat::format::Image {
        total_sectors: (SD_IMAGE_BYTES / 512) as u32,
        sectors_per_cluster: 8,
        files: &[rp_fat::format::Entry {
            name: "retropane.bin",
            data: &bin,
        }],
        blocked_clusters: &[],
        ballast: 0,
    });
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, img).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path.to_path_buf())
}

/// The checks' scratch folder (screenshots, card images, recordings) and
/// the emulator release's cache.
fn emulator_dir(root: &Path) -> PathBuf {
    root.join("target").join("emulator")
}

/// `name` in the scratch folder, which is created here: QEMU writes some of
/// these files itself and can't make the folder, and a fresh checkout has
/// none (only downloading the release made one, which hid this from the
/// release path). Every check's scratch file goes through here; a test
/// keeps it that way.
fn scratch(root: &Path, name: &str) -> Result<PathBuf, String> {
    let dir = emulator_dir(root);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(dir.join(name))
}

/// The raw-binary folder (`target/emu`), created here.
fn emu_dir(root: &Path) -> Result<PathBuf, String> {
    let dir = root.join("target").join("emu");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(dir)
}

/// The release build's `image` as a raw binary in `target/emu`, made here:
/// every check stands alone, never relying on a file an earlier check left.
fn raw_image(root: &Path, image: &str) -> Result<PathBuf, String> {
    let out = emu_dir(root)?.join(format!("{image}.bin"));
    crate::sdkit::raw_binary(&crate::arm_release(root).join(image), &out)?;
    Ok(out)
}

/// Screen size of the emulated X51v, for converting taps to absolute pointer units.
const SCREEN: (u32, u32) = (480, 640);

/// Boots the bring-up image, checks it reports success over the UART,
/// screenshots the emulated display and verifies the test pattern, then
/// presses buttons and checks the image reports them.
pub fn boot_hello(root: &Path) -> Result<(), String> {
    let image = crate::arm_release(root).join("rp-hello");
    let shot = scratch(root, "hello-screen.ppm")?;
    let sd = write_sd_test_image(&scratch(root, "sd-test.img")?)?;
    let output = boot(
        root,
        &Boot {
            image: &image,
            marker: HELLO_MARKER,
            screenshot: Some(&shot),
            sd_image: Some(&sd),
            steps: HELLO_STEPS,
            extra: &[],
            input: None,
            during: None,
            then: None,
            timeout: BOOT_TIMEOUT,
        },
    )?;
    for want in [SD_EXPECT_CARD, SD_EXPECT_PARTITION] {
        if !output.contains(want) {
            return Err(format!("SD check failed: no `{want}` on serial"));
        }
    }
    eprint!("{output}");
    if !output.ends_with('\n') {
        eprintln!();
    }
    eprintln!("    emulator boot OK");
    let ppm = std::fs::read(&shot).map_err(|e| format!("{}: {e}", shot.display()))?;
    crate::screen::check_boot_screen(&ppm)?;
    eprintln!(
        "    display OK: test pattern and boot log verified ({})",
        shot.display()
    );
    eprintln!("    SD OK: card identified and partition table read");
    eprintln!(
        "    input OK: {} button/stylus interactions reported",
        HELLO_STEPS.len()
    );
    Ok(())
}

/// Must match `rp_kernel::MARKER`.
const KERNEL_MARKER: &str = "RETROPANE-KERNEL-OK";
/// Every VM step the kernel image must pass.
const KERNEL_EXPECT: &[&str] = &[
    "VM tables OK",
    "VM device map OK",
    "VM zero page OK",
    "VM file page OK",
    "VM copy-on-write OK",
    "VM FCSE OK",
    "VM domains OK",
    "VM fault loop OK",
    "VM compressed RAM OK",
    "VM reclaim OK",
    "VM pinned OK",
    "VM many regions OK",
    "VM unmap OK",
    // IPC (ADR-0016): two user processes, a migrating call and back 1000
    // times, a faulting server unwound to its caller, a forged handle
    // refused, and each process's memory its own.
    "IPC call OK",
    "IPC server fault OK",
    "IPC foreign handle OK",
    "IPC nested OK",
    "IPC too deep OK",
    "IPC isolation OK",
    "IPC preempted in calls OK",
    "IPC PE image relocated OK",
    "IPC PE pages loaded on demand OK",
    "IPC service manager OK",
    "IPC supervised restart OK",
    "IPC spawn and exit OK",
    "IPC native slots OK",
    "IPC handle pages OK",
    // A real driver in a host (item 46): the compiled display host runs the
    // backlight on its granted PWM window, restarts after a fault in a
    // call and in its own thread, and keeps the level the hardware shows.
    "IPC driver host served OK",
    "IPC driver host restarted OK",
    "TRACE ring OK",
];

/// After the kernel's marker: the emulator presses the power button and the
/// display host (its granted interrupt) turns the light off and on again.
const KERNEL_STEPS: &[Step] = &[
    Step {
        action: Action::Hmp("qom-set /machine/peripheral/power pressed true"),
        expect: &[],
    },
    Step {
        action: Action::Hmp("qom-set /machine/peripheral/power pressed false"),
        expect: &[Expect::Text("DISPLAY button: light off")],
    },
    Step {
        action: Action::Hmp("qom-set /machine/peripheral/power pressed true"),
        expect: &[],
    },
    Step {
        action: Action::Hmp("qom-set /machine/peripheral/power pressed false"),
        expect: &[Expect::Text("DISPLAY button: light back on")],
    },
];

/// Boots the kernel image: the VM layer live on the CPU, with real page
/// faults resolved by demand paging and copy-on-write.
pub fn boot_kernel(root: &Path) -> Result<(), String> {
    let image = crate::arm_release(root).join("rp-kernel");
    let output = boot(
        root,
        &Boot {
            image: &image,
            marker: KERNEL_MARKER,
            screenshot: None,
            sd_image: None,
            steps: KERNEL_STEPS,
            extra: &[],
            input: None,
            during: None,
            then: None,
            timeout: BOOT_TIMEOUT,
        },
    )?;
    eprint!("{output}");
    if !output.ends_with('\n') {
        eprintln!();
    }
    for want in KERNEL_EXPECT {
        if !output.contains(want) {
            return Err(format!("kernel check failed: no `{want}` on serial"));
        }
    }
    check_boot(&output, &["boot_kernel_tables"])?;
    eprintln!("    kernel VM OK: page tables, demand paging, copy-on-write, FCSE, unmap");
    eprintln!("    kernel IPC OK: migrating calls between processes, fault unwind, handles");
    eprintln!(
        "    kernel driver host OK: backlight driver in a compiled user process, granted PWM window and power-button interrupt, restarted twice"
    );
    Ok(())
}

/// Must match `rp_threads::MARKER`.
const THREADS_MARKER: &str = "RETROPANE-THREADS-OK";
/// What the threads image must report.
const THREADS_EXPECT: &[&str] = &[
    "THREADS ping-pong OK",
    "THREADS sleep OK",
    "THREADS mutex inheritance OK",
    "THREADS interrupt thread OK",
    "USER mode OK",
    "USER yield OK",
    "USER preempted OK",
    "USER preempted after a system call OK",
    "USER fault contained OK",
    "USER iWMMXt lazy switch OK",
    "USER KData OK",
    "THREADS FCSE native slot OK",
];

/// Boots the threads image: scheduler, context switch, events, sleeping and
/// the tickless idle alarm, live on the CPU.
pub fn boot_threads(root: &Path) -> Result<(), String> {
    let image = crate::arm_release(root).join("rp-threads");
    let output = boot(
        root,
        &Boot {
            image: &image,
            marker: THREADS_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            // Guest time from instructions: its quanta and sleeps are timer
            // deadlines, and on host time a busy PC let a 5 ms quantum end
            // before the first thread reached its system call (the order
            // check saw `BS`).
            extra: &["-icount", "shift=0"],
            input: None,
            during: None,
            then: None,
            timeout: BOOT_TIMEOUT,
        },
    )?;
    eprint!("{output}");
    if !output.ends_with('\n') {
        eprintln!();
    }
    for want in THREADS_EXPECT {
        if !output.contains(want) {
            return Err(format!("threads check failed: no `{want}` on serial"));
        }
    }
    eprintln!(
        "    threads OK: context switches, events, sleeps, tickless idle, user mode, syscalls, preemption, KData, FCSE in a native slot"
    );
    Ok(())
}

/// Must match `rp_quantum::MARKER`.
const QUANTUM_MARKER: &str = "RETROPANE-QUANTUM-OK";

/// Boots the quantum image (ADR-0010 amendment): every time-slice policy
/// runs its touch, frame, busy-loop and switch-cost tests. The emulator's
/// timing isn't the device's, so only that each ran is checked; the numbers
/// are printed (the device's come from `cargo xtask load rp-quantum`).
pub fn boot_quantum(root: &Path) -> Result<(), String> {
    let image = crate::arm_release(root).join("rp-quantum");
    let output = boot(
        root,
        &Boot {
            image: &image,
            marker: QUANTUM_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: None,
            during: None,
            then: None,
            // Eight policies of ~2.5 s each, in real time (~21 s), with
            // room for a loaded host.
            timeout: Duration::from_secs(120),
        },
    )?;
    eprint!("{output}");
    if !output.ends_with('\n') {
        eprintln!();
    }
    for test in ["touch", "anim", "busy", "hogs"] {
        let runs = output
            .lines()
            .filter(|l| l.starts_with("QT q") && l.contains(&format!(" {test} ")))
            .count();
        if runs != 8 {
            return Err(format!(
                "quantum check failed: {runs} `{test}` results, not 8"
            ));
        }
    }
    eprintln!("    quantum OK: eight time-slice policies measured");
    Ok(())
}

/// Must match `rp_light::MARKER`.
const LIGHT_MARKER: &str = "RETROPANE-LIGHT-OK";

/// Boots the first-light image: every startup step must pass.
pub fn boot_light(root: &Path) -> Result<(), String> {
    let image = crate::arm_release(root).join("rp-light");
    let output = boot(
        root,
        &Boot {
            image: &image,
            marker: LIGHT_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: None,
            during: None,
            then: None,
            timeout: BOOT_TIMEOUT,
        },
    )?;
    eprint!("{output}");
    if !output.contains("all steps OK") {
        return Err("first light: `all steps OK` missing on serial".into());
    }
    eprintln!("    first light OK");
    Ok(())
}

/// Must match `rp_loader::MARKER`.
const LOADER_MARKER: &str = "RPLOAD1 READY";

/// Boot-time budgets (docs/design-brief.md), in milliseconds from an
/// image's entry. The emulator runs in real time on a shared host, so these
/// are loose: they catch a regression of many times, not 10%. The device's
/// numbers are what count, and go in docs/perf.md.
const BOOT_BUDGETS_MS: &[(&str, f64)] = &[
    // The loader: 200 ms of it is the USB detach (the PC must see the
    // device leave); GPIO, the card and the DMA self-test run inside it.
    // Was 742 ms (a 16 MiB beacon fill, then everything in turn), then
    // 504 ms (a fixed 500 ms 2700G settle, replaced by waiting for the
    // hand-off's DMA to stop: `devices::take_mbx`): 204 ms in the emulator.
    ("boot_loader_ready", 260.0),
    // The kernel's frame allocator, page tables and device map.
    ("boot_kernel_tables", 100.0),
];

/// Checks the `BENCH boot_*` lines in `output` against [`BOOT_BUDGETS_MS`]:
/// each named one must be present and within budget.
fn check_boot(output: &str, names: &[&str]) -> Result<(), String> {
    let measured = parse_bench(output);
    for name in names {
        let budget = BOOT_BUDGETS_MS
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, b)| *b)
            .ok_or_else(|| format!("no boot budget for {name}"))?;
        let ms = measured
            .iter()
            .find(|r| r.name == *name)
            .map(|r| r.ns / 1e6)
            .ok_or_else(|| format!("boot time: no `BENCH {name}` line"))?;
        if ms > budget {
            return Err(format!(
                "boot time: {name} took {ms:.1} ms, over its {budget} ms budget"
            ));
        }
        eprintln!("    boot time {name}: {ms:.1} ms (budget {budget} ms)");
    }
    Ok(())
}

/// How long rp-blend holds each stage (kept in step with the image).
const RP_BLEND_STAGE_SECONDS: u32 = 15;

/// The noise-source measurement (rp-noise, ADR-0029): under the loader it
/// must read the clocks, collect every source, save each to the card
/// whole (1,000,000 16-bit samples), append its restart row to the rows
/// already there, and return. The emulator's timing comes from the PC, so
/// its numbers say nothing about the Axim's noise (docs/hw/emulator.md):
/// this checks the plumbing only.
/// The ways rp-noise tries to run an OS timer channel on the 32 kHz clock
/// must count as they did on the X51v (rp-noise run 3, docs/hw/entropy.md):
/// only channel 5 with `OMCR5` bit 6 (`0xC1`). Channel 11 with `0x81` isn't
/// checked: it counted in run 3 and not in run 4. An emulator that counts
/// every way (rp-v0.13.0 and older) fails here.
fn noise_timer_ways(log: &str) -> Result<(), String> {
    const WAYS: &[(&str, bool)] = &[
        ("channel 5 OMCR 0x81 mode first", false),
        ("channel 5 OMCR 0x81 counter first", false),
        ("channel 5 OMCR 0x01 mode first", false),
        ("channel 5 OMCR 0xc1 mode first", true),
        ("channel 5 OMCR 0x82 mode first", false),
        ("channel 5 OMCR 0x84 mode first", false),
        ("channel 6 OMCR 0x81 mode first", false),
        ("channel 8 OMCR 0x81 mode first", false),
    ];
    for &(way, counts) in WAYS {
        let prefix = format!("timer: {way}:");
        let line = log
            .lines()
            .find(|l| l.trim_start().starts_with(&prefix))
            .ok_or_else(|| format!("noise: the run log has no `{prefix}`:{log}"))?;
        // "... count {c0} -> {c1} in 2 ms ..."
        let counted = line
            .split_once(", count ")
            .and_then(|(_, r)| r.split_once(" in "))
            .and_then(|(r, _)| r.split_once(" -> "))
            .and_then(|(a, b)| Some((a.parse::<u32>().ok()?, b.parse::<u32>().ok()?)))
            .map(|(c0, c1)| c1.wrapping_sub(c0))
            .ok_or_else(|| format!("noise: can't read the counts in `{line}`"))?;
        // 32,768 Hz is ~65 counts in 2 ms; the device's other ways stayed at 0 (runs 3 and 4).
        let ok = if counts {
            (40..=100).contains(&counted)
        } else {
            counted == 0
        };
        if !ok {
            return Err(format!(
                "noise: {way} counted {counted} in 2 ms; on the X51v it {} (rp-noise run 3): `{line}`",
                if counts { "counts ~65" } else { "never counts" }
            ));
        }
    }
    Ok(())
}

pub fn boot_loader_noise(root: &Path) -> Result<(), String> {
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let loader = dir.join("rp-loader.bin");
    let bin = dir.join("rp-noise.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-noise"), &bin)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    // Two restart rows from "earlier runs": the run must keep them and add
    // its own after them.
    let earlier: Vec<u8> = (0..2000u32).map(|i| (i * 7) as u8).collect();
    let image = rp_fat::format::build(&rp_fat::format::Image {
        total_sectors: 64 * 1024,
        sectors_per_cluster: 8,
        files: &[rp_fat::format::Entry {
            name: "noise-ri.bin",
            data: &earlier,
        }],
        blocked_clusters: &[],
        ballast: 0,
    });
    let sd = scratch(root, "sd-noise.img")?;
    std::fs::write(&sd, &image).map_err(|e| format!("{}: {e}", sd.display()))?;
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: Some(&sd),
            steps: &[],
            extra: &[],
            input: Some((&rp_loadproto::frame(&payload), "RETROPANE-NOISE: done")),
            during: None,
            then: Some("RPL END"),
            timeout: Duration::from_secs(600),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("noise: never came back to the loader")?;
    if crashed(log) || log.contains("did not return") {
        return Err(format!("noise: didn't return cleanly:{log}"));
    }
    for want in [
        "clocks: OSCC",
        "clocks: core measured",
        "card: ready",
        "timer: sampling with channel 5, OMCR 0xc1",
        "A interrupt (cycles between 32 kHz tick interrupts): 1000000 samples",
        "A spaced (cycles across 16 tick interrupts, low 16 bits): 100000 samples",
        "ticks missed and resynchronised:",
        "B one-clock (cycles for 64 SDRAM reads): 1000000 samples",
        "T TEMP0: 1000000 samples",
        "T VBAT: 1000000 samples",
        "T AUX: 1000000 samples",
        "T TEMP1: 1000000 samples",
        // The emulator's ticks follow the PC's clock: never a healthy run,
        // so the restart test must refuse the row (ADR-0029 R6); the
        // append itself is host-tested (`append_row`).
        "noise-ri.bin: row REFUSED (unhealthy run",
        "noise.txt: saved",
        "RETROPANE-NOISE: done",
    ] {
        if !log.contains(want) {
            return Err(format!("noise: the run log has no `{want}`:{log}"));
        }
    }
    noise_timer_ways(log)?;
    let spaced = card_file(&sd, "noise-as.bin")?;
    if spaced.len() != 200_000 {
        return Err(format!(
            "noise: noise-as.bin is {} bytes, want 200,000",
            spaced.len()
        ));
    }
    for name in [
        "noise-ai.bin",
        "noise-b.bin",
        "noise-t0.bin",
        "noise-vb.bin",
        "noise-ax.bin",
        "noise-t1.bin",
    ] {
        let data = card_file(&sd, name)?;
        if data.len() != 2_000_000 {
            return Err(format!(
                "noise: {name} is {} bytes, want 2,000,000",
                data.len()
            ));
        }
    }
    let rows = card_file(&sd, "noise-ri.bin")?;
    if rows[..] != earlier[..] {
        return Err(format!(
            "noise: noise-ri.bin is {} bytes, want the 2 earlier rows untouched (row refused)",
            rows.len()
        ));
    }
    let text = card_file(&sd, "noise.txt")?;
    if !String::from_utf8_lossy(&text).contains("MCV min-entropy") {
        return Err("noise: noise.txt has no summaries".into());
    }
    // Quick mode: with `noise-quick.txt` on the card, only the restart row.
    let image = rp_fat::format::build(&rp_fat::format::Image {
        total_sectors: 64 * 1024,
        sectors_per_cluster: 8,
        files: &[
            rp_fat::format::Entry {
                name: "noise-quick.txt",
                data: b"restart rows only",
            },
            rp_fat::format::Entry {
                name: "noise-ri.bin",
                data: &earlier[..1000],
            },
        ],
        blocked_clusters: &[],
        ballast: 0,
    });
    std::fs::write(&sd, &image).map_err(|e| format!("{}: {e}", sd.display()))?;
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: Some(&sd),
            steps: &[],
            extra: &[],
            input: Some((&rp_loadproto::frame(&payload), "RETROPANE-NOISE: done")),
            during: None,
            then: Some("RPL END"),
            timeout: Duration::from_secs(120),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("noise quick: never came back to the loader")?;
    for want in [
        "quick mode (noise-quick.txt)",
        "noise-ri.bin: row REFUSED (unhealthy run",
    ] {
        if !log.contains(want) {
            return Err(format!("noise quick: the run log has no `{want}`:{log}"));
        }
    }
    if log.contains("B one-clock") || crashed(log) {
        return Err(format!(
            "noise quick: did more than the restart row, or crashed:{log}"
        ));
    }
    let rows = card_file(&sd, "noise-ri.bin")?;
    if rows[..] != earlier[..1000] {
        return Err(format!(
            "noise quick: noise-ri.bin is {} bytes, want it untouched (row refused)",
            rows.len()
        ));
    }
    eprintln!(
        "    noise OK: clocks read, the 32 kHz ticks taken as interrupts (every tick and spaced), the other sources collected and saved whole, the restart row refused as unhealthy (the emulator's ticks are the PC's) and the file untouched, quick mode does only that, back at the loader"
    );
    Ok(())
}

/// The USB disconnect hunt (rp-unplug, docs/hw/udc.md T4): every candidate
/// tried and undone, the host controller's port 2 reached, the run back at
/// the loader with no fault. The emulator can't say which candidate
/// disconnects on the device (its pull-up isn't modelled): this only proves
/// the image is safe to send.
pub fn boot_loader_unplug(root: &Path) -> Result<(), String> {
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let loader = dir.join("rp-loader.bin");
    let bin = dir.join("rp-unplug.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-unplug"), &bin)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((
                &rp_loadproto::frame(&payload),
                "RetroPane 26 USB disconnect hunt",
            )),
            during: None,
            then: Some("RPL END"),
            timeout: Duration::from_secs(240),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("unplug: never came back to the loader")?;
    if crashed(log) || log.contains("did not return") {
        return Err(format!("unplug: didn't return cleanly:{log}"));
    }
    for want in [
        "UDC on, UP2OCR",
        "UP2OCR was",
        "host controller SE0 on port 2 at",
        "UP2OCR 0 with the UDC off at",
        "RETROPANE-UNPLUG-DONE",
    ] {
        if !log.contains(want) {
            return Err(format!("unplug: the run log has no `{want}`:{log}"));
        }
    }
    eprintln!("    unplug OK: every candidate tried and undone, back at the loader");
    Ok(())
}

/// The clocks test image (rp-clocks, ADR-0031): runs under the loader and
/// returns by itself. Every operating point is visited and the starting
/// clocks and voltage code put back; the image must come back without a
/// fault and print its verdict. Which checks the emulator can pass depends
/// on what the fork models (docs/hw/emulator.md, "PXA270 clocks"): the
/// log is shown for reading, and a fault, a panic or no verdict fail.
pub fn boot_loader_clocks(root: &Path) -> Result<(), String> {
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let loader = dir.join("rp-loader.bin");
    let bin = dir.join("rp-clocks.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-clocks"), &bin)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&rp_loadproto::frame(&payload), "RETROPANE-CLOCKS: ")),
            during: None,
            then: Some("RPL END"),
            timeout: Duration::from_secs(300),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("clocks: never came back to the loader")?;
    if crashed(log) || log.contains("did not return") {
        return Err(format!("clocks: didn't return cleanly:{log}"));
    }
    for want in ["C1 as found", "C2 624 MHz", "end: CCCR", " checks, "] {
        if !log.contains(want) {
            return Err(format!("clocks: the run log has no `{want}`:{log}"));
        }
    }
    // A fork that models the clocks (the cycle counter at the core clock,
    // CCSR latched at F, WM's voltage pins: branch pxa-clocks) must pass
    // every check, with every point visited.
    let modelled = log.contains("ok  cycle counter agrees");
    if modelled {
        if !log.contains(" checks, 0 BAD") || log.contains("skipped") {
            return Err(format!(
                "clocks: the modelled clocks must pass every check:{log}"
            ));
        }
        eprintln!("    clocks OK: every point visited, every check passed on the modelled clocks");
    } else {
        eprintln!("    clocks: returned cleanly; run log:{log}");
    }
    clocks_fault_path(root, &loader, modelled)?;
    clocks_past_rating(root, &loader, modelled)
}

/// `rp-clocks` stage C2b (`RP_CLOCKS_OC`, C2's test past 624 MHz, one
/// second of checks a step here): it returns cleanly with every step
/// logged and the clocks put back; on a fork that models the clocks, every
/// check passes.
fn clocks_past_rating(root: &Path, loader: &Path, modelled: bool) -> Result<(), String> {
    let bin = variant_image_envs(
        root,
        "rp-clocks",
        &[("RP_CLOCKS_OC", "1"), ("RP_CLOCKS_OC_SHORT", "1")],
    )?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let output = boot(
        root,
        &Boot {
            image: loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&rp_loadproto::frame(&payload), "RETROPANE-CLOCKS: ")),
            during: None,
            then: Some("RPL END"),
            timeout: Duration::from_secs(300),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("clocks C2b: never came back to the loader")?;
    if crashed(log) || log.contains("did not return") {
        return Err(format!("clocks C2b: didn't return cleanly:{log}"));
    }
    for want in [
        "C2b (the owner's go 2026-10-30)",
        "C2b 624 (the control) MHz",
        "end: CCCR",
        " checks, ",
    ] {
        if !log.contains(want) {
            return Err(format!("clocks C2b: the run log has no `{want}`:{log}"));
        }
    }
    if log.contains("C2 624 MHz") {
        return Err(format!("clocks C2b: C2 ran too:{log}"));
    }
    if modelled && !log.contains(" checks, 0 BAD") {
        return Err(format!(
            "clocks C2b: the modelled clocks must pass every check:{log}"
        ));
    }
    eprintln!("    clocks C2b OK: the stage ran, returned cleanly, the clocks put back");
    Ok(())
}

/// `rp-clocks` run F (quality audit C31): `CCCR` asking for another point,
/// the clocks saved so, then a data abort; the crash path's put-back must
/// keep the point running and log its checks before the crash report.
fn clocks_fault_path(root: &Path, loader: &Path, modelled: bool) -> Result<(), String> {
    let bin = variant_image(root, "rp-clocks", ("RP_CLOCKS_FAULT", "1"))?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let output = boot(
        root,
        &Boot {
            image: loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&rp_loadproto::frame(&payload), "RETROPANE-CLOCKS: run F")),
            during: None,
            then: Some("RPL END"),
            timeout: Duration::from_secs(120),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("clocks run F: never came back to the loader")?;
    // The order the crash path promises: the put-back's lines, then the
    // fault's.
    let put_back = log.find("F5 put back by the crash path");
    let fault = log.find("FAULT DataAbort");
    match (put_back, fault) {
        (Some(p), Some(f)) if p < f => {}
        _ => {
            return Err(format!(
                "clocks run F: wanted the put-back's lines, then the data abort's report:{log}"
            ));
        }
    }
    for want in [
        "F2 CCCR asks for 2N 2",
        "F3 saved with CCCR",
        "F6 end",
        "run F: ",
    ] {
        if !log.contains(want) {
            return Err(format!("clocks run F: the run log has no `{want}`:{log}"));
        }
    }
    if modelled && !log.contains("run F: 13 checks, 0 BAD") {
        return Err(format!(
            "clocks run F: the modelled clocks must pass every check:{log}"
        ));
    }
    eprintln!("    clocks run F OK: the fault path put the running point back and CCCR as found");
    Ok(())
}

/// Takes a screendump each guest second until `check` passes (the step
/// it looks for is up), `secs` at most; the last one is kept beside `shot`
/// as `<shot>-<tag>.ppm` for a person to look at. The image's steps start
/// after work whose length depends on the emulator's speed, so the check
/// waits for the picture instead of a time.
/// A gpubench run that refused cleanly because the clocks start at 0 (the
/// release fork: Windows Mobile's COREPLL never modelled), with nothing
/// else run: no render measured, so no other part's checks are skipped.
fn gpubench_refused_clocks(log: &str) -> bool {
    log.contains("  as found: COREPLL 0x0,")
        && log.contains("not Windows Mobile's clock and bus timing: nothing changed")
        && log.contains("RETROPANE-GPUBENCH: done, 0 renders measured")
}

fn gpubench_until(
    mon: &mut crate::screen::Monitor,
    guest: &str,
    shot: &Path,
    secs: u32,
    tag: &str,
    check: &dyn Fn(&crate::screen::Image) -> Result<(), String>,
) -> Result<(), String> {
    let mut last = String::from("no screendump");
    for _ in 0..secs {
        guest_hold(mon, 1000, "gpubench: waiting for the picture")?;
        let _ = std::fs::remove_file(shot);
        mon.screendump(guest, shot)?;
        let data = std::fs::read(shot).map_err(|e| e.to_string())?;
        let img = crate::screen::parse_ppm(&data)?;
        let _ = std::fs::write(shot.with_extension(format!("{tag}.ppm")), &data);
        match check(&img) {
            Ok(()) => return Ok(()),
            Err(e) => last = e,
        }
    }
    Err(format!("{last} (after {secs} s of screendumps)"))
}

/// Pixel `(x, y)` is the colour `want` names channel by channel (1 bright,
/// 0 dark): what a test card or a flat fill shows, with room for the
/// colour conversion.
fn gpubench_expect(
    img: &crate::screen::Image,
    what: &str,
    x: u32,
    y: u32,
    want: (u8, u8, u8),
) -> Result<(), String> {
    let i = ((y * img.width + x) * 3) as usize;
    let got = (img.rgb[i], img.rgb[i + 1], img.rgb[i + 2]);
    let ok = |c: u8, on: u8| if on == 1 { c >= 150 } else { c <= 110 };
    if ok(got.0, want.0) && ok(got.1, want.1) && ok(got.2, want.2) {
        Ok(())
    } else {
        Err(format!(
            "gpubench: {what} at ({x},{y}) is {got:?}, should be {want:?} (1 bright, 0 dark)"
        ))
    }
}

/// The 2700G bus-timing experiment (rp-cs3bus, docs/research/memory-
/// timing.md §7): runs under the loader and returns. The emulator doesn't
/// model bus timing (every step reads right), so this checks the plumbing:
/// with `MSC1` as Windows Mobile leaves it, every step runs with its 2D
/// fill, the soak runs at two steps looser than the tightest, and `MSC1`
/// is put back; with any other `MSC1` it refuses and changes nothing.
/// R71's bus check (`rp-cs3bus` built with `RP_CS3BUS_SELFTEST`, two
/// seconds of warm traffic): the fork models no bus timing, so every check
/// passes and 7/2/0 is picked; the run's shape and the restore are checked.
///
/// Then the negative control (`RP_CS3BUS_CANDIDATE=420`, the ruling packet of
/// 2026-10-29): the same check with 4/2/0 tried in place of 7/2/0. The fork
/// reads right at any timing, so here it passes too: only its shape (the
/// candidate named in every step and the pick) is checked; the X51v must
/// show the misses.
fn cs3bus_selftest(root: &Path, dir: &Path, loader: &Path) -> Result<(), String> {
    cs3bus_selftest_one(root, dir, loader, None)?;
    cs3bus_selftest_one(root, dir, loader, Some("420"))
}

fn cs3bus_selftest_one(
    root: &Path,
    dir: &Path,
    loader: &Path,
    candidate: Option<&str>,
) -> Result<(), String> {
    let mut envs = vec![("RP_CS3BUS_SELFTEST", "1"), ("RP_CS3BUS_WARM_SECS", "2")];
    if let Some(c) = candidate {
        envs.push(("RP_CS3BUS_CANDIDATE", c));
    }
    let img = variant_image_envs(root, "rp-cs3bus", &envs)?;
    let _ = dir;
    let payload = std::fs::read(&img).map_err(|e| format!("{}: {e}", img.display()))?;
    let output = boot(
        root,
        &Boot {
            image: loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&rp_loadproto::frame(&payload), "RPL OK sha256")),
            during: None,
            then: Some("RPL END"),
            timeout: Duration::from_secs(600),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("cs3bus self-test: never came back to the loader")?;
    if log.contains("MSC1 not as expected: nothing changed") {
        eprintln!("    cs3bus self-test OK: MSC1 isn't Windows Mobile's here, nothing changed");
        return Ok(());
    }
    let (cold, warm, pick) = if candidate.is_some() {
        (
            "4/2/0 cold (MSC1 ",
            "warm traffic at 4/2/0: ",
            "self-test pick: (4, 2, 0) (4/2/0 passed cold and warm); the default passed",
        )
    } else {
        (
            "7/2/0 cold (MSC1 ",
            "warm traffic at 7/2/0: ",
            "self-test pick: (7, 2, 0) (7/2/0 passed cold and warm); the default passed",
        )
    };
    for want in [
        "cs3bus self-test (R71): six patterns over 512 KiB",
        "default 7/3/1 (MSC1 ",
        cold,
        warm,
        pick,
        "MSC1 restored 24842481",
        "RETROPANE-CS3BUS-DONE",
    ] {
        if crashed(log) || !log.contains(want) {
            return Err(format!("cs3bus self-test: the log has no `{want}`:{log}"));
        }
    }
    eprintln!(
        "    cs3bus self-test OK ({}): both checks clean (no bus timing modelled), the candidate picked, MSC1 put back",
        candidate.unwrap_or("720")
    );
    Ok(())
}

pub fn boot_loader_cs3bus(root: &Path) -> Result<(), String> {
    if !device_has(root, "intel-2700g", "mbx-2d") {
        eprintln!("    cs3bus: skipped (this emulator release doesn't model the MBX 2D engine)");
        return Ok(());
    }
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    let loader = dir.join("rp-loader.bin");
    let bin = dir.join("rp-cs3bus.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-cs3bus"), &bin)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&rp_loadproto::frame(&payload), "RPL OK sha256")),
            during: None,
            then: Some("RPL END"),
            timeout: Duration::from_secs(600),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("cs3bus: never came back to the loader")?;
    if crashed(log) || !log.contains("RETROPANE-CS3BUS-DONE") {
        return Err(format!("cs3bus: didn't finish cleanly:{log}"));
    }
    cs3bus_selftest(root, &dir, &loader)?;
    if log.contains("MSC1 not as expected: nothing changed") {
        eprintln!(
            "    cs3bus OK: the emulator's MSC1 isn't Windows Mobile's, and it refused to change anything"
        );
        return Ok(());
    }
    // The fork doesn't model bus timing, so every point passes and has the
    // margin; the soak's point is the fastest flush, which the host's
    // timing picks, so any point past Windows Mobile's is accepted.
    for want in [
        "every point ran",
        "grid: 54 of 54 points passed, 54 with the margin",
        "soak at point ",
        " 0 misses",
        "MSC1 restored 24842481",
    ] {
        if !log.contains(want) {
            return Err(format!("cs3bus: the run log has no `{want}`:{log}"));
        }
    }
    eprintln!("    cs3bus OK: every point of the grid and the soak ran, MSC1 put back");
    Ok(())
}

/// The CPLD bus-timing test (rp-cpldbus, reminder R8): started by the
/// loader, it must walk every read step to the PXA27x's VLIO minimum (the
/// emulator's CPLD is never misread: timing isn't modelled), every step
/// clean, and put Windows Mobile's `MSC2` back; with any other `MSC2` it
/// refuses and changes nothing.
pub fn boot_loader_cpldbus(root: &Path) -> Result<(), String> {
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    let loader = dir.join("rp-loader.bin");
    let bin = dir.join("rp-cpldbus.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-cpldbus"), &bin)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&rp_loadproto::frame(&payload), "RPL OK sha256")),
            during: None,
            then: Some("RPL END"),
            timeout: Duration::from_secs(600),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("cpldbus: never came back to the loader")?;
    if crashed(log) || !log.contains("RETROPANE-CPLDBUS-DONE") {
        return Err(format!("cpldbus: didn't finish cleanly:{log}"));
    }
    if log.contains("MSC2 not as expected: nothing changed") {
        eprintln!(
            "    cpldbus OK: the emulator's MSC2 isn't Windows Mobile's, and it refused to change anything"
        );
        return Ok(());
    }
    let steps = log
        .lines()
        .filter(|l| l.starts_with('A') && l.contains(" RRR ") && l.contains(" wrong of "))
        .count();
    if steps != 32 {
        return Err(format!("cpldbus: {steps} read steps, not 32:{log}"));
    }
    for line in log.lines().filter(|l| l.contains(" wrong of ")) {
        if !line.contains(", 0 wrong of ") {
            return Err(format!("cpldbus: {line}"));
        }
    }
    for want in [
        "A done: the shortest clean RDF 3 RDN 2 RRR 0 (9 clocks against 106)",
        "cpldbus done; MSC2 restored 7FF47FFC",
    ] {
        if !log.contains(want) {
            return Err(format!("cpldbus: the run log has no `{want}`:{log}"));
        }
    }
    eprintln!("    cpldbus OK: all 32 read steps clean, MSC2 put back");
    Ok(())
}

/// One `rp-gpubench` run for [`boot_loader_gpubench`]: its name, image,
/// wanted log lines, whether they need the 3D model, and its key presses
/// (guest ms before the first, between them, how many).
type GpubenchRun<'a> = (&'a str, &'a PathBuf, &'a [&'a str], bool, (u32, u32, u32));

/// Registers as plain storage: the emulator's 2700G from reset.
#[derive(Default)]
struct RegFile(std::collections::HashMap<usize, u32>);

impl rp_hal_api::RegisterBus for RegFile {
    fn read32(&mut self, offset: usize) -> u32 {
        self.0.get(&offset).copied().unwrap_or(0)
    }

    fn write32(&mut self, offset: usize, value: u32) {
        self.0.insert(offset, value);
    }
}

/// How long rp-picture shows its picture (kept in step with the image).
const fn rp_picture_seconds() -> u32 {
    20
}

/// What rp-picture shows: `local/concept/picture.rgb565`, or the generated
/// gradient its build uses when that's absent.
fn expected_picture(root: &Path) -> Result<Vec<u8>, String> {
    const BYTES: usize = 480 * 640 * 2;
    match std::fs::read(root.join("local/concept/picture.rgb565")) {
        Ok(p) if p.len() == BYTES => Ok(p),
        Ok(p) => Err(format!("local/concept/picture.rgb565: {} bytes", p.len())),
        Err(_) => {
            let mut v = Vec::with_capacity(BYTES);
            for y in 0..640u32 {
                for x in 0..480u32 {
                    let px = ((x * 31 / 479) << 11) | ((y * 63 / 639) << 5) | (31 - y * 31 / 639);
                    v.extend_from_slice(&(px as u16).to_le_bytes());
                }
            }
            Ok(v)
        }
    }
}

/// Windows Mobile's shared CPLD shadow table as it is in RAM at HaRET's
/// hand-off (docs/hw/cpld.md "Shadow at hand-over"; slots in Windows
/// Mobile's order: 0x02, 0x10, 0x12, 0x14, 0x16, 0x18, 0x1A, 0x1C, 0x1E,
/// 0x08), as QEMU arguments that put it there before the loader starts:
/// the emulator otherwise starts from reset, with nothing in RAM. The
/// values measured on the owner's X51v (`rp-handover`, 2026-10-16), the
/// same the emulated CPLD's latches start with: backlight on, the SD and
/// CF slots powered.
fn wm_cpld_table_args() -> Vec<String> {
    const TABLE: u32 = 0xA800_1320;
    const SLOTS: [u32; 10] = [
        0x00FD, 0x1000, 0x000F, 0x0008, 0x0003, 0x0004, 0x0006, 0x000C, 0, 0,
    ];
    SLOTS
        .iter()
        .enumerate()
        .filter(|(_, v)| **v != 0)
        .flat_map(|(i, v)| {
            [
                "-device".to_string(),
                format!(
                    "loader,addr={:#x},data={v:#x},data-len=4",
                    TABLE + 4 * i as u32
                ),
            ]
        })
        .collect()
}

/// An image built with one of its build-time switches set (`env` = `(name,
/// value)`), in its own target folder so the plain build stays as it is;
/// the raw file to load.
fn variant_image(root: &Path, image: &str, env: (&str, &str)) -> Result<PathBuf, String> {
    variant_image_with(root, image, env, None)
}

/// [`variant_image`] with a second switch `also` set (its own folder too).
fn variant_image_with(
    root: &Path,
    image: &str,
    env: (&str, &str),
    also: Option<(&str, &str)>,
) -> Result<PathBuf, String> {
    match also {
        Some(a) => variant_image_envs(root, image, &[env, a]),
        None => variant_image_envs(root, image, &[env]),
    }
}

/// [`variant_image`] built with every one of `envs` (named by their keys,
/// joined: one or two keys name it as [`variant_image_with`] always did).
/// A variant image whose build waits until a check needs it: its image
/// and build variables, by the binary's path.
type Deferred = std::collections::HashMap<PathBuf, (String, Vec<(String, String)>)>;

std::thread_local! {
    /// While `Some`, [`variant_image_envs`] only names the binary and
    /// records its build here ([`variant_built`] builds it): `emu-gpubench`
    /// with `RETROPANE_GPUBENCH_RUNS` builds only the runs it runs (all
    /// ~127 variants rebuilt one by one took most of a one-run check).
    static DEFERRED: std::cell::RefCell<Option<Deferred>> = const { std::cell::RefCell::new(None) };
}

/// The binary at `bin` built now if its build was deferred (a no-op
/// otherwise).
fn variant_built(root: &Path, bin: &Path) -> Result<(), String> {
    let job = DEFERRED.with(|d| d.borrow_mut().as_mut().and_then(|m| m.remove(bin)));
    let Some((image, envs)) = job else {
        return Ok(());
    };
    let envs: Vec<(&str, &str)> = envs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let built = variant_image_build(root, &image, &envs)?;
    if built != bin {
        return Err(format!("{}: built as {}", bin.display(), built.display()));
    }
    Ok(())
}

fn variant_image_envs(root: &Path, image: &str, envs: &[(&str, &str)]) -> Result<PathBuf, String> {
    let deferred = DEFERRED.with(|d| {
        d.borrow_mut().as_mut().map(|m| {
            let out =
                emu_dir(root).map(|dir| dir.join(format!("{image}-{}.bin", variant_name(envs))));
            if let Ok(out) = &out {
                let job = envs
                    .iter()
                    .map(|&(k, v)| (k.to_string(), v.to_string()))
                    .collect();
                m.insert(out.clone(), (image.to_string(), job));
            }
            out
        })
    });
    match deferred {
        Some(out) => out,
        None => variant_image_build(root, image, envs),
    }
}

/// A variant's name: its variables' names, joined.
fn variant_name(envs: &[(&str, &str)]) -> String {
    envs.iter()
        .map(|e| e.0)
        .collect::<Vec<_>>()
        .join("-")
        .to_ascii_lowercase()
}

/// [`variant_image_envs`]'s build: the image with `envs` set, its size
/// checked, its binary and ELF beside the other images.
fn variant_image_build(root: &Path, image: &str, envs: &[(&str, &str)]) -> Result<PathBuf, String> {
    let name = variant_name(envs);
    let mut cmd = Command::new("cargo");
    let dir = crate::frame_pointer_build(&mut cmd, image, &format!("target/variants/{name}"));
    crate::with_local_inputs(&mut cmd);
    cmd.current_dir(root);
    for &(k, v) in envs {
        cmd.env(k, v);
    }
    run(cmd.args([
        "build",
        "--release",
        "--target",
        crate::TARGET_SPEC,
        "-Zbuild-std=core,compiler_builtins",
        "-Zjson-target-spec",
        "-q",
        "-p",
        image,
        "--target-dir",
        &dir,
    ]))?;
    let elf = root
        .join(&dir)
        .join("armv5te-x51v-none-eabi/release")
        .join(image);
    crate::size::check_variant(&elf, image)?;
    let out = emu_dir(root)?.join(format!("{image}-{name}.bin"));
    crate::sdkit::raw_binary(&elf, &out)?;
    // The ELF beside it, for its addresses (`check_chain`, `symbols`).
    let kept = out.with_extension("elf");
    std::fs::copy(&elf, &kept).map_err(|e| format!("{}: {e}", kept.display()))?;
    Ok(out)
}

/// The backlight through the loader: `backlight 30` must fade PWM0 to the
/// perceptual 30% (9% duty: 38 of RetroPane's 432 counts, prescale 0: the
/// same 30.09 kHz as Windows Mobile's 216 at prescale 1) and `backlight 100` to full
/// duty, checked in the emulator's PWM registers through the monitor.
pub fn boot_loader_backlight(root: &Path) -> Result<(), String> {
    if !has_usb_model(root) || !device_has(root, "pxa27x-pwm", "dcr0") {
        eprintln!("    backlight: skipped (no USB or PWM model)");
        return Ok(());
    }
    let loader = raw_image(root, "rp-loader")?;
    let args = usb_args(9);
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let port = monitor_port();
    let check = || -> Result<(), String> {
        let mut s = usb_connect(9)?;
        usb_read_until(&mut s, "RPLOAD1 READY", 10)?;
        // An idle loader sleeps between polls, and still answers at once.
        guest_hold(
            &mut crate::screen::Monitor::connect(monitor_port())?,
            2000,
            "backlight",
        )?;
        let asked = Instant::now();
        s.write_all(&rp_loadproto::control_frame("drive status"))
            .map_err(|e| e.to_string())?;
        let reply = usb_read_until(&mut s, "%\r\n", 10)?;
        let took = asked.elapsed();
        let idle: u32 = reply
            .rsplit("idle ")
            .next()
            .and_then(|t| t.split('%').next())
            .and_then(|n| n.trim().parse().ok())
            .ok_or_else(|| format!("no idle share in {reply:?}"))?;
        if idle < 80 || took > Duration::from_millis(500) {
            return Err(format!("idle loader: {idle}% asleep, answered in {took:?}"));
        }
        eprintln!(
            "    idle OK: the waiting loader was {idle}% asleep and answered in {} ms",
            took.as_millis()
        );
        // Boot time by stage (`handoff`): each stage ends after the one
        // before it, and all before the 2700G is taken and the loader ready.
        s.write_all(&rp_loadproto::control_frame("handoff"))
            .map_err(|e| e.to_string())?;
        let reply = usb_read_until(
            &mut s, " ms
", 10,
        )?;
        let after = |key: &str| -> Option<u32> {
            reply
                .split(key)
                .nth(1)?
                .split(" ms")
                .next()?
                .trim()
                .parse()
                .ok()
        };
        let times = [
            after("card "),
            after("dma+fat "),
            after("cf "),
            after("2700G at "),
            after("ready at "),
        ];
        if times.iter().any(Option::is_none) || !times.windows(2).all(|w| w[0] <= w[1]) {
            return Err(format!(
                "handoff: stages missing or out of order in {reply:?}"
            ));
        }
        eprintln!("    boot stages OK: {}", reply.trim());
        s.write_all(&rp_loadproto::control_frame("backlight 30"))
            .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL OK backlight 30% (duty 38/432)", 10)?;
        let mut mon = crate::screen::Monitor::connect(port)?;
        let dcr = mon.query("qom-get /machine/peripheral/pwm dcr0")?;
        let pcr = mon.query("qom-get /machine/peripheral/pwm pcr0")?;
        let cr = mon.query("qom-get /machine/peripheral/pwm cr0")?;
        if dcr != "38" || pcr != "431" || cr != "0" {
            return Err(format!(
                "backlight: PWM0 duty {dcr}, period {pcr}, prescale {cr}; wanted 38, 431, 0"
            ));
        }
        s.write_all(&rp_loadproto::control_frame("backlight 100"))
            .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL OK backlight 100% (duty 432/432)", 10)?;
        let dcr = mon.query("qom-get /machine/peripheral/pwm dcr0")?;
        if dcr != "1024" {
            return Err(format!("backlight: 100% isn't full duty (DCR {dcr})"));
        }
        // The power button, if this emulator has it: a press turns the
        // screen off, the next brings it back at the level it had.
        if device_has_runtime(&mut mon, "/machine/peripheral/power", "pressed") {
            s.write_all(&rp_loadproto::control_frame("backlight 60"))
                .map_err(|e| e.to_string())?;
            usb_read_until(&mut s, "RPL OK backlight 60%", 10)?;
            // Held until the duty reads `want` (the loader fades on the
            // press), never for a fixed time: a busy host gives the loader
            // few polls per second, as emu-today's taps showed. The gap
            // after the release is the loader's to see it (no debounce on
            // a release; nothing to watch for).
            let press = |mon: &mut crate::screen::Monitor, want: &str| -> Result<String, String> {
                mon.query("qom-set /machine/peripheral/power pressed true")?;
                let deadline = Instant::now() + Duration::from_secs(20);
                let got = loop {
                    let got = mon.query("qom-get /machine/peripheral/pwm dcr0")?;
                    if got == want || Instant::now() > deadline {
                        break got;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                };
                mon.query("qom-set /machine/peripheral/power pressed false")?;
                guest_hold(mon, 800, "backlight")?;
                Ok(got)
            };
            let off = press(&mut mon, "0")?;
            let on = press(&mut mon, "155")?;
            // 60% -> 0.36 of 432 counts = 155.
            if off != "0" || on != "155" {
                return Err(format!(
                    "power button: screen off/on gave DCR {off} then {on}; wanted 0, 155"
                ));
            }
            eprintln!("    power button OK: press turned the screen off, the next back on at 60%");
        }
        // `reboot wm`: the emulator doesn't model GPIO99 as a reset (not yet
        // confirmed on the device), so the loader must say so and carry on.
        s.write_all(&rp_loadproto::control_frame("reboot wm"))
            .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL OK reboot wm", 10)?;
        usb_read_until(&mut s, "RPL ERR reboot wm: no reset after 1 s", 10)?;
        s.write_all(&rp_loadproto::control_frame("backlight 100"))
            .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL OK backlight 100%", 10)?;
        eprintln!("    reboot wm OK: GPIO99 driven, no reset modelled, the loader carried on");
        Ok(())
    };
    boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &extra,
            input: None,
            during: Some(&check),
            then: None,
            timeout: Duration::from_secs(60),
        },
    )?;
    eprintln!("    backlight OK: faded to 30% and 100% through the loader, checked in PWM0");
    Ok(())
}

/// Must match `rp_usb::MARKER`.
const USB_MARKER: &str = "RETROPANE-USB-READY";

/// Boots the USB link test. The emulator has no UDC model yet (ADR-0012),
/// so this only proves the image starts, enables the controller and connects
/// without faulting; the driver itself is tested against a simulated host.
pub fn boot_usb(root: &Path) -> Result<(), String> {
    let image = crate::arm_release(root).join("rp-usb");
    let output = boot(
        root,
        &Boot {
            image: &image,
            marker: USB_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: None,
            during: None,
            then: None,
            timeout: BOOT_TIMEOUT,
        },
    )?;
    eprint!("{output}");
    if !output.contains("UDC enabled") {
        return Err("usb: `UDC enabled` missing on serial".into());
    }
    eprintln!("    usb image OK (starts and connects; no UDC model to enumerate against)");
    Ok(())
}

/// Must match `rp_probe::MARKER`.
const PROBE_MARKER: &str = "RETROPANE-PROBE-DONE";
/// Text every probe report must contain.
const PROBE_EXPECT: &[&str] = &[
    "1 SYSTEM",
    "2 WM STATE",
    "3 2700G",
    "4 BENCH",
    "check mem OK",
    "Sharp VGA",
    "SD SD RCA",
    "-bit",
    "chase_sram_256k",
    "tlb_spill_60_l1sram",
    "check wmmx_copy OK",
    "check wmmx_avg OK",
    "check wmmx_dot OK",
    "check dma_shapes OK",
    // rp-v0.30.0: iWMMXt adds saturate, WALIGNR at offset 0 returns wRn.
    "check wmmx_mix OK",
    "check wmmx_align OK",
];

/// Probe checks the emulator is known to fail, each with why (they pass on
/// the X51v): any other failed check fails `emu-probe` (a failed check was
/// never looked at before 2026-10-22).
/// Run FI's 3D render-done status words, one per enables setting
/// (`3D started true: busy 0x..., done 0x...,`).
fn fi_render_done(log: &str) -> Vec<u32> {
    log.lines()
        .filter_map(|l| l.split_once("3D started true: ")?.1.split_once("done 0x"))
        .filter_map(|(_, rest)| u32::from_str_radix(rest.get(..8)?, 16).ok())
        .collect()
}

/// (rp-v0.30.0's iWMMXt fixes moved `wmmx_mix` and `wmmx_align` into
/// `PROBE_EXPECT`.)
const PROBE_EMULATOR_LIMITS: &[(&str, &str)] = &[
    (
        "pmu_insns",
        "the fork's event counters read 0 (cache, TLB and instruction counts not modellable: docs/hw/emulator.md)",
    ),
    (
        "pmu_branches",
        "the fork's event counters read 0 (docs/hw/emulator.md)",
    ),
    (
        "pmu_dmiss",
        "the fork's event counters read 0 (docs/hw/emulator.md)",
    ),
];

/// Boots the probe image (the real-device report), then checks every page
/// line's checksum, that the report is complete, and that it's on screen.
pub fn boot_probe(root: &Path) -> Result<(), String> {
    let image = crate::arm_release(root).join("rp-probe");
    let shot = scratch(root, "probe-screen.ppm")?;
    let bin: Vec<u8> = (0..1500u32).map(|i| i as u8).collect();
    let exfat = rp_fat::exfat::format::build(
        (SD_IMAGE_BYTES / 512) as u32,
        3, // 4 KiB clusters
        &[rp_fat::exfat::format::Entry {
            name: "retropane.bin",
            data: &bin,
            fragment: false,
        }],
    );
    let sd = scratch(root, "sd-probe-exfat.img")?;
    std::fs::write(&sd, exfat).map_err(|e| format!("{}: {e}", sd.display()))?;
    // The report may still be drawing when its last serial line goes out
    // (seen on a busy CI runner): screenshot until it is complete, for up
    // to 10 s.
    let guest = match emulator(root)? {
        Emulator::Wsl => to_wsl_path(&shot),
        Emulator::Native(_) => shot.display().to_string(),
    };
    let screen = || -> Result<(), String> {
        let mut mon = crate::screen::Monitor::connect(monitor_port())?;
        let mut last = String::new();
        for _ in 0..20 {
            let _ = std::fs::remove_file(&shot);
            mon.screendump(&guest, &shot)?;
            let ppm = std::fs::read(&shot).map_err(|e| format!("{}: {e}", shot.display()))?;
            match crate::screen::check_text_screen(&ppm) {
                Ok(()) => return Ok(()),
                Err(e) => last = e,
            }
            poll_pause(Duration::from_millis(500));
        }
        Err(last)
    };
    let output = boot(
        root,
        &Boot {
            image: &image,
            marker: PROBE_MARKER,
            screenshot: None,
            sd_image: Some(&sd),
            steps: &[],
            extra: &[],
            input: None,
            during: Some(&screen),
            then: None,
            timeout: BOOT_TIMEOUT,
        },
    )?;
    eprint!("{output}");
    if !output.ends_with('\n') {
        eprintln!();
    }
    let lines = check_probe_report(&output)?;
    if !output.contains(SD_EXPECT_FAT) || !output.contains("retropane.bin 1500 B") {
        return Err(format!(
            "probe: no `{SD_EXPECT_FAT}` / `retropane.bin 1500 B` line in the report"
        ));
    }
    eprintln!("    probe OK: {lines} report lines, all checksums valid, report on screen");
    probe_pages(root)?;
    probe_mini(root)?;
    probe_wmhop(root)
}

/// The probe's `RP_PROBE_MINI` variant (the CPU gate's V1 and C5): the
/// window mapped as sections and into the mini data cache; the emulator
/// models no cache (times are the device's: emulator.md), so this checks
/// both windows map `backing`'s frames and every row is reported.
fn probe_mini(root: &Path) -> Result<(), String> {
    let image = variant_image(root, "rp-probe", ("RP_PROBE_MINI", "1"))?;
    let loader = emu_dir(root)?.join("rp-loader.bin");
    crate::sdkit::raw_binary(&crate::arm_release(root).join("rp-loader"), &loader)?;
    let payload = std::fs::read(&image).map_err(|e| format!("{}: {e}", image.display()))?;
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&rp_loadproto::frame(&payload), "RPL OK sha256")),
            during: None,
            then: Some("RPL END"),
            timeout: Duration::from_secs(600),
        },
    )?;
    let output = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l.to_string())
        .ok_or("probe mini: never came back to the loader")?;
    for l in output
        .lines()
        .filter(|l| l.starts_with('P') && l.contains('|'))
    {
        check_report_line(l)?;
    }
    for n in [
        "walk_normal_512",
        "walk_mini_512",
        "walk_normal_2k",
        "walk_mini_2k",
        "walk_mini_8k",
        "hot_after_stream_normal",
        "hot_after_stream_mini",
    ] {
        let named = output
            .lines()
            .filter_map(|l| l.split_once('|').map(|(_, t)| t))
            .any(|t| t.split_whitespace().any(|w| w == n));
        if !named {
            return Err(format!("probe mini: `{n}` missing:\n{output}"));
        }
    }
    if crashed(&output) || !output.contains("check mini_mapping OK") {
        return Err(format!(
            "probe mini: crashed, or a window wasn't the backing's frames:\n{output}"
        ));
    }
    eprintln!("    probe mini OK: both windows mapped, every row reported");
    Ok(())
}

/// The probe's `RP_PROBE_WMHOP` variant (the CPU gate's V7): Windows
/// Mobile's hop programs (`suite::wm_hops`) in the 8 MiB window under each
/// mapping. The emulator models no cache or TLB timing (emulator.md), so
/// this checks the chains run under every mapping (no fault, no failed
/// check) and every row is in the checksummed report.
fn probe_wmhop(root: &Path) -> Result<(), String> {
    let image = variant_image(root, "rp-probe", ("RP_PROBE_WMHOP", "1"))?;
    let loader = emu_dir(root)?.join("rp-loader.bin");
    crate::sdkit::raw_binary(&crate::arm_release(root).join("rp-loader"), &loader)?;
    let payload = std::fs::read(&image).map_err(|e| format!("{}: {e}", image.display()))?;
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&rp_loadproto::frame(&payload), "RPL OK sha256")),
            during: None,
            then: Some("RPL END"),
            timeout: Duration::from_secs(600),
        },
    )?;
    let output = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l.to_string())
        .ok_or("probe wmhop: never came back to the loader")?;
    for l in output
        .lines()
        .filter(|l| l.starts_with('P') && l.contains('|'))
    {
        check_report_line(l)?;
        let text = l.split_once('|').map_or("", |(_, t)| t);
        let mut words = text.split_whitespace();
        if words.next() == Some("check") && words.nth(1) == Some("FAIL") {
            return Err(format!("probe wmhop: a check failed: {}", text.trim()));
        }
    }
    let named = |n: &str| {
        output
            .lines()
            .filter_map(|l| l.split_once('|').map(|(_, t)| t))
            .any(|t| t.split_whitespace().any(|w| w == n))
    };
    for map in ["sections", "64k", "4k", "4k_l2sram"] {
        for n in [
            format!("wm_chase_{map}"),
            format!("wm_walk8m_{map}"),
            format!("wm_walk256k_{map}"),
            format!("wm_walk32k_{map}"),
            format!("wm_walk8m_{map}_dtlb"),
            format!("wm_chase_{map}_dcache"),
        ] {
            if !named(&n) {
                return Err(format!(
                    "probe wmhop: `{n}` missing:
{output}"
                ));
            }
        }
    }
    if crashed(&output) {
        return Err(format!(
            "probe wmhop: crashed:
{output}"
        ));
    }
    eprintln!(
        "    probe wmhop OK: Windows Mobile's hop programs under all four mappings, every row reported"
    );
    Ok(())
}

/// The probe's `RP_PROBE_PAGES` variant (the CPU gate's C4 and V7): the
/// same 8 MiB window mapped as sections, 64 KiB and 4 KiB pages, its
/// tables in SDRAM and the internal SRAM, then the first-level table moved
/// into the SRAM. The emulator's MMU walks every one (the times and TLB
/// counts are the device's: emulator.md), so this checks each mapping
/// reads the chain back (no fault, no failed check) and every row is in
/// the checksummed report.
fn probe_pages(root: &Path) -> Result<(), String> {
    let image = variant_image(root, "rp-probe", ("RP_PROBE_PAGES", "1"))?;
    let loader = emu_dir(root)?.join("rp-loader.bin");
    crate::sdkit::raw_binary(&crate::arm_release(root).join("rp-loader"), &loader)?;
    let payload = std::fs::read(&image).map_err(|e| format!("{}: {e}", image.display()))?;
    // Under the loader, as on the device (`cargo xtask load`).
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&rp_loadproto::frame(&payload), "RPL OK sha256")),
            during: None,
            then: Some("RPL END"),
            timeout: Duration::from_secs(600),
        },
    )?;
    let output = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l.to_string())
        .ok_or("probe pages: never came back to the loader")?;
    let mut lines = 0;
    for l in output
        .lines()
        .filter(|l| l.starts_with('P') && l.contains('|'))
    {
        check_report_line(l)?;
        lines += 1;
        let text = l.split_once('|').map_or("", |(_, t)| t);
        let mut words = text.split_whitespace();
        if words.next() == Some("check") && words.nth(1) == Some("FAIL") {
            return Err(format!("probe pages: a check failed: {}", text.trim()));
        }
    }
    for row in [
        "pages_8m_sections",
        "pages_8m_64k",
        "pages_8m_4k",
        "pages_8m_4k_l2sram",
        "pages_8m_64k_l2sram",
        "pages_8m_sections_l1sram",
        "pages_8m_4k_l1sram",
        "pages_8m_4k_l1l2sram",
    ] {
        // Each row's chase, its page-hopping twin (`suite::far_chase`) and
        // both miss counts.
        let far = row.replace("pages_8m", "far");
        let names = [
            row.to_string(),
            format!("{row}_dtlb"),
            far.clone(),
            format!("{far}_dtlb"),
        ];
        let named = |n: &str| {
            output
                .lines()
                .filter_map(|l| l.split_once('|').map(|(_, t)| t))
                .any(|t| t.split_whitespace().any(|w| w == n))
        };
        if let Some(n) = names.iter().find(|n| !named(n)) {
            return Err(format!("probe pages: `{n}` missing:\n{output}"));
        }
    }
    // TLB lock-down (C4), with the tables in SDRAM and in the SRAM: rows
    // and counts (the fork's lock is a no-op or modelled: the counts are
    // the device's).
    for n in [
        "tlblock_none",
        "tlblock_16",
        "tlblock_none_dtlb",
        "tlblock_16_dtlb",
        "tlblock_none_sram",
        "tlblock_16_sram",
        "tlblock_none_sram_dtlb",
        "tlblock_16_sram_dtlb",
    ] {
        let named = output
            .lines()
            .filter_map(|l| l.split_once('|').map(|(_, t)| t))
            .any(|t| t.split_whitespace().any(|w| w == n));
        if !named {
            return Err(format!("probe pages: `{n}` missing:\n{output}"));
        }
    }
    if crashed(&output) || output.matches("check pages_mapping OK").count() != 2 {
        return Err(format!(
            "probe pages: crashed, or a window wasn't the backing's frames:\n{output}"
        ));
    }
    eprintln!("    probe pages OK: {lines} report lines, every mapping chased");
    Ok(())
}

/// The probe report's line checksum: the content's bytes (trailing spaces
/// trimmed) summed modulo 256. Must match `rp_probe::report::checksum`.
pub fn probe_checksum(content: &str) -> u8 {
    content
        .trim_end()
        .bytes()
        .fold(0u8, |a, b| a.wrapping_add(b))
}

/// Verifies one `P<page>|<line>` report line; returns its page (1-4).
pub fn check_report_line(l: &str) -> Result<usize, String> {
    let (page, text) = l
        .strip_prefix('P')
        .and_then(|r| r.split_once('|'))
        .ok_or_else(|| format!("not a report line: `{l}`"))?;
    let page: usize = page.parse().map_err(|_| format!("bad page in `{l}`"))?;
    if !(1..=PROBE_PAGES_MAX).contains(&page) || text.len() != 40 {
        return Err(format!("malformed report line `{l}`"));
    }
    let (content, sum) = text.split_at(37);
    let want = u8::from_str_radix(sum.trim(), 16).map_err(|_| format!("bad checksum in `{l}`"))?;
    if probe_checksum(content) != want {
        return Err(format!("checksum mismatch in `{l}`"));
    }
    Ok(page)
}

/// The most report pages a probe line may name (the probe's `PAGES` is 5
/// since the performance gate's benchmarks; the check takes as many as the
/// report has, so the two can't drift into a silent mismatch).
const PROBE_PAGES_MAX: usize = 9;

/// Verifies the `P<page>|<line>` copy of the report on serial: every page
/// from 1 to the highest named non-empty, at least 4. Returns the number
/// of lines.
pub fn check_probe_report(output: &str) -> Result<usize, String> {
    check_probe_report_with(output, PROBE_EMULATOR_LIMITS)
}

/// [`check_probe_report`] with the emulator's known limits given (a
/// failed check named in `limits` is let through).
fn check_probe_report_with(output: &str, limits: &[(&str, &str)]) -> Result<usize, String> {
    let mut pages = [0usize; PROBE_PAGES_MAX];
    for l in output.lines() {
        if !l.starts_with('P') || !l.contains('|') {
            continue;
        }
        pages[check_report_line(l)? - 1] += 1;
    }
    let last = pages.iter().rposition(|&n| n > 0).map_or(0, |p| p + 1);
    if last < 4 {
        return Err(format!("probe report has {last} pages, 4 at least"));
    }
    if let Some(p) = pages[..last].iter().position(|&n| n == 0) {
        return Err(format!("probe report page {} is empty", p + 1));
    }
    if output.contains("DROPPED") {
        return Err("probe report dropped results (its table is full)".into());
    }
    for want in PROBE_EXPECT {
        if !output.contains(want) {
            return Err(format!("probe report lacks `{want}`"));
        }
    }
    if output.contains("OVERFLOW") {
        return Err("probe report overflowed a page".into());
    }
    for l in output.lines() {
        let Some((_, text)) = l.split_once('|') else {
            continue;
        };
        let mut words = text.split_whitespace();
        if words.next() != Some("check") {
            continue;
        }
        let (name, verdict) = (words.next().unwrap_or(""), words.next());
        if verdict == Some("FAIL") && !limits.iter().any(|(n, _)| *n == name) {
            return Err(format!("probe check `{name}` failed: {}", text.trim()));
        }
    }
    Ok(pages.iter().sum())
}

/// Must match `rp_bench::DONE`.
const BENCH_DONE: &str = "BENCH-DONE";
/// The bench image's deliberate read of unmapped memory must be reported so.
const FAULT_EXPECT: &str = "FAULT DataAbort at ";
/// OS timer frequency the bench image counts in.
const TICK_HZ: f64 = 3_250_000.0;
/// A slowdown beyond this fails `cargo xtask bench`. Results are deterministic
/// under `-icount`, so any real change in work shows up; the margin only
/// absorbs timer granularity.
const REGRESSION_LIMIT: f64 = 0.02;

/// One benchmark result in nanoseconds per iteration (emulated time).
#[derive(Debug, Clone, PartialEq)]
pub struct BenchResult {
    /// Benchmark name.
    pub name: String,
    /// Nanoseconds per iteration.
    pub ns: f64,
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
        self.wait_until(from, |o| o.contains(text))
    }

    /// Waits until `pred` holds for the output from byte `from` onwards.
    fn wait_until(&mut self, from: usize, pred: impl Fn(&str) -> bool) -> bool {
        loop {
            if self.output.get(from..).is_some_and(&pred) {
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
/// How to boot an image in the emulator.
struct Boot<'a> {
    image: &'a Path,
    /// Serial text that means the image is up (or done).
    marker: &'a str,
    screenshot: Option<&'a Path>,
    sd_image: Option<&'a Path>,
    steps: &'a [Step],
    /// Extra QEMU arguments.
    extra: &'a [&'a str],
    /// Bytes to send to the serial port once `marker` appears, and the text
    /// that must follow.
    input: Option<(&'a [u8], &'a str)>,
    /// Run while the emulator is still up, after `input` (e.g. talk to the
    /// emulated USB serial port).
    during: Option<&'a dyn Fn() -> Result<(), String>>,
    /// Serial text to wait for after `during`, before the emulator stops:
    /// the run log's end, for checks that read it (`during` returns on what
    /// it sees on screen, and the log can still be on its way).
    then: Option<&'a str>,
    timeout: Duration,
}

/// A check's extra emulator arguments, with the sound recorded to
/// `throwaway` (a wav backend) unless the check records it itself
/// (`-audiodev`): without an audio backend QEMU plays the guest's sound on
/// the PC's speakers (it beeped at the owner, 2026-10-19). Not QEMU's
/// `none` backend: on Windows it stalled the machine protocol (QMP) mid-run.
fn silent_unless_recorded<'a>(extra: &[&'a str], throwaway: &'a str) -> Vec<&'a str> {
    let mut v = extra.to_vec();
    if !extra.contains(&"-audiodev") {
        v.extend(["-audiodev", throwaway, "-machine", "audiodev=rpsilent"]);
    }
    v
}

fn boot(root: &Path, cfg: &Boot<'_>) -> Result<String, String> {
    let Boot {
        image,
        marker,
        screenshot,
        sd_image,
        steps,
        extra,
        input,
        during,
        then,
        timeout,
    } = *cfg;
    let port = monitor_port();
    let monitor = format!("tcp:127.0.0.1:{port},server,nowait");
    let serial_arg = format!("tcp:127.0.0.1:{},server=on,wait=on", serial_port());
    let qmp = format!("tcp:127.0.0.1:{},server,nowait", port + 1);
    let emu = emulator(root)?;
    let wsl = matches!(emu, Emulator::Wsl);
    let silent = scratch(root, "silent.wav")?;
    let throwaway = format!(
        "wav,id=rpsilent,path={}",
        if wsl {
            to_wsl_path(&silent)
        } else {
            silent.display().to_string()
        }
    );
    let extra = &silent_unless_recorded(extra, &throwaway);
    let _load = match std::env::var_os(LOAD_ENV) {
        Some(_) if wsl => Some(start_load(port, timeout + Duration::from_secs(30))?),
        Some(_) => {
            return Err(format!(
                "{LOAD_ENV}: the load runs in WSL, with the WSL emulator"
            ));
        }
        None => None,
    };
    let mut cmd = match emu {
        Emulator::Native(exe) => {
            eprintln!("    emulator: {}", exe.display());
            let mut c = Command::new(exe);
            c.args(emu_args().split(' '))
                .args(["-serial", &serial_arg, "-monitor", &monitor, "-qmp", &qmp])
                .args(extra)
                .arg("-kernel")
                .arg(image);
            if let Some(sd) = sd_image {
                c.arg("-drive")
                    .arg(format!("if=sd,format=raw,file={}", sd.display()));
            }
            c
        }
        Emulator::Wsl => {
            eprintln!("    emulator: local WSL build");
            // `timeout` guarantees the Linux process exits even if killing
            // wsl.exe doesn't reach it.
            let sd = sd_image
                .map(|p| format!(" -drive 'if=sd,format=raw,file={}'", to_wsl_path(p)))
                .unwrap_or_default();
            let wsl_qemu = wsl_qemu();
            wsl_bash(&format!(
                "exec timeout {} {wsl_qemu} {} {} -serial {serial_arg} -monitor {monitor} -qmp {qmp} -kernel '{}'{sd}",
                timeout.as_secs() + 10,
                emu_args(),
                extra.join(" "),
                to_wsl_path(image)
            ))
        }
    };
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("failed to start emulator: {e}"))?;

    // The serial socket: QEMU listens and holds the guest until we connect.
    let link = connect_serial(&mut child);
    let link = match link {
        Ok(l) => l,
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e);
        }
    };
    // Stream the guest's output through a channel so we can enforce a
    // timeout; the same socket carries what we send.
    let mut reader = link.try_clone().map_err(|e| e.to_string())?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = [0u8; 256];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });

    let mut serial = Serial {
        rx,
        output: String::new(),
        deadline: Instant::now() + timeout,
    };
    let mut sent_at = 0;
    let result = run_session(&mut serial, marker, port, wsl, screenshot, steps)
        .and_then(|()| {
            sent_at = serial.output.len();
            send_input(&mut serial, &link, input)
        })
        .and_then(|()| {
            // The time limit is the run's, from here: the upload before it
            // grows with the image (rp-picture's 630 KiB takes ~55 s over
            // the emulated serial line, and a 4.8 KiB larger build ran out
            // of its 90 s, 2026-10-20). A hang still ends at the limit.
            serial.deadline = Instant::now() + timeout;
            during.map_or(Ok(()), |f| f())
        })
        .and_then(|()| match then {
            Some(text) if !serial.wait_for(text, sent_at) => {
                Err(format!("no `{text}` on serial after the check"))
            }
            _ => Ok(()),
        });
    let _ = child.kill();
    let _ = child.wait();
    if wsl {
        // Killing wsl.exe may not reach the Linux process at once; make sure
        // it's gone before the next test binds the same ports.
        let _ = wsl_bash(&stop_own_qemu(port)).status();
    }
    // Whatever the guest printed while `during` ran is still queued: keep it,
    // so a test can check what happened after its own waiting.
    for chunk in serial.rx.try_iter() {
        serial.output.push_str(&String::from_utf8_lossy(&chunk));
    }
    match result.and_then(|()| check_stack(&serial.output).map(|_| ())) {
        Ok(()) => Ok(serial.output),
        Err(e) => Err(format!("{e}. Serial output so far:\n{}", serial.output)),
    }
}

/// `RETROPANE_EMU_LOAD=1`: every check runs with all of WSL's cores busy,
/// to prove it waits on the image and not on the wall clock (emu-today
/// passed on an idle PC and failed on a busy one, 2026-10-15).
const LOAD_ENV: &str = "RETROPANE_EMU_LOAD";

/// Marks the load's processes, with the run's monitor port after it, for
/// [`Load`]'s drop to end this run's load and no other.
const LOAD_TAG: &str = "rp-emu-load";

/// Ends this run's emulator and no other: its command line names this
/// run's monitor port. Two sessions' runs share WSL (each in its own port
/// block), and a `pkill -x qemu-system-arm` here once ended the other
/// run's emulator mid-check (rp-blend under load, 2026-10-15: "terminating
/// on signal 15"). The `[,]` keeps the pattern from matching its own
/// shell's command line. Then it waits (up to 10 s) until that emulator
/// has gone: a check started while the last one lingered found the monitor
/// port answered by the old emulator and its WAV file still open (no sound
/// voice, the DMA never drained; 2026-10-19). One that ignores the TERM is
/// killed outright after the 10 s and waited for again: one lived on for
/// minutes past both this TERM and its `timeout 100`'s, writing the same
/// WAV, and the next checks' codec never asked for a sample (`emu-tone`
/// stalled, `emu-audio-s2` read the link 0.9% slow; 2026-10-19).
fn stop_own_qemu(monitor_port: u16) -> String {
    // No `$(...)` in a script for `wsl`: it reaches bash broken (a syntax
    // error, and then nothing in the script runs at all).
    let tries = vec!["t"; 100].join(" ");
    let mine = format!("'qemu-system-arm .*-monitor tcp:127.0.0.1:{monitor_port}[,]'");
    let wait = format!("for i in {tries}; do pgrep -f {mine} >/dev/null || break; sleep 0.1; done");
    format!("pkill -f {mine}; {wait}; pkill -KILL -f {mine}; {wait}; true")
}

/// The load script: one spinner per core, a fixed count, each ended by
/// `timeout` after `secs` even if nothing stops it (an unbounded spawn
/// loop once froze WSL). It waits for them: WSL ends them with it. Each
/// spinner carries `LOAD_TAG-port;`, the run's own mark.
fn load_script(monitor_port: u16, secs: u64) -> String {
    format!(
        "for i in $(seq $(nproc)); do timeout {secs} bash -c ': {LOAD_TAG}-{monitor_port}; while :; do :; done' & done; wait
"
    )
}

/// Ends this run's load and no other's (see [`stop_own_qemu`]); the `[;]`
/// keeps the pattern from matching its own shell's command line.
fn stop_own_load(monitor_port: u16) -> String {
    format!("pkill -f '{LOAD_TAG}-{monitor_port}[;]'; true")
}

/// Starts the load in WSL (see [`LOAD_ENV`]); it lasts until the returned
/// guard drops (or its limit). The script goes on stdin: `wsl.exe` passes a command line
/// through a shell, which would expand its `$(...)` early.
fn start_load(monitor_port: u16, limit: Duration) -> Result<Load, String> {
    eprintln!(
        "    load: one spinner per WSL core for at most {} s",
        limit.as_secs()
    );
    let mut child = Command::new("wsl")
        .args(["-d", &wsl_distro(), "--", "bash", "-s"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .map_err(|e| format!("starting the load: {e}"))?;
    child
        .stdin
        .take()
        .expect("piped")
        .write_all(load_script(monitor_port, limit.as_secs()).as_bytes())
        .map_err(|e| format!("starting the load: {e}"))?;
    Ok(Load(child, monitor_port))
}

/// The running load (and its run's monitor port); dropping it ends it, on
/// every way out of `boot`.
struct Load(std::process::Child, u16);

impl Drop for Load {
    fn drop(&mut self) {
        let _ = wsl_bash(&stop_own_load(self.1)).status();
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Connects to the guest's serial socket once QEMU listens (it holds the
/// guest until then); fails if QEMU exits first or never listens.
fn connect_serial(child: &mut std::process::Child) -> Result<std::net::TcpStream, String> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Ok(s) = std::net::TcpStream::connect(("127.0.0.1", serial_port())) {
            // WSL's localhost relay accepts at once, even before QEMU
            // listens inside WSL, then drops the connection: a socket that
            // is already closed (a peek reads end-of-stream) is that, so
            // try again. Silence (a timeout) is a live QEMU holding the
            // guest for us.
            s.set_read_timeout(Some(Duration::from_millis(300)))
                .map_err(|e| e.to_string())?;
            let mut probe = [0u8; 1];
            let live = match s.peek(&mut probe) {
                Ok(n) => n > 0,
                Err(e) => matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ),
            };
            if live {
                s.set_read_timeout(None).map_err(|e| e.to_string())?;
                s.set_nodelay(true).map_err(|e| e.to_string())?;
                return Ok(s);
            }
        }
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!(
                "the emulator exited before its serial port opened ({status})"
            ));
        }
        if Instant::now() > deadline {
            return Err("the emulator's serial port never opened".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Sends `input`'s bytes to the guest's serial port and waits for its text.
fn send_input(
    serial: &mut Serial,
    link: &std::net::TcpStream,
    input: Option<(&[u8], &str)>,
) -> Result<(), String> {
    let Some((bytes, expect)) = input else {
        return Ok(());
    };
    let mut link = link;
    let from = serial.output.len();
    link.write_all(bytes)
        .and_then(|()| link.flush())
        .map_err(|e| format!("writing to the serial port: {e}"))?;
    if serial.wait_for(expect, from) {
        Ok(())
    } else {
        Err(format!(
            "after sending {} bytes: no `{expect}` on serial",
            bytes.len()
        ))
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
        return Err(format!("no `{marker}` before the deadline"));
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
    let mut qmp: Option<crate::screen::Qmp> = None;
    for step in steps {
        let from = serial.output.len();
        let what = match step.action {
            Action::Hmp(cmd) => {
                mon.command(cmd)?;
                cmd.to_string()
            }
            Action::TouchDown(..) | Action::TouchUp => {
                if qmp.is_none() {
                    qmp = Some(crate::screen::Qmp::connect(port + 1)?);
                }
                touch(qmp.as_mut().expect("connected above"), &step.action)?
            }
        };
        for want in step.expect {
            let ok = match *want {
                Expect::Text(t) => serial.wait_for(t, from),
                Expect::TouchNear(x, y) => serial.wait_until(from, |o| touch_near(o, x, y)),
            };
            if !ok {
                let desc = match *want {
                    Expect::Text(t) => t.to_string(),
                    Expect::TouchNear(x, y) => format!("TOUCH near {x},{y}"),
                };
                return Err(format!("after `{what}`: no `{desc}` on serial"));
            }
        }
    }
    Ok(())
}

/// Sends a stylus action over QMP; returns a description for error messages.
fn touch(qmp: &mut crate::screen::Qmp, action: &Action) -> Result<String, String> {
    match *action {
        Action::TouchDown(x, y) => {
            let ax = u32::from(x) * 0x7FFF / (SCREEN.0 - 1);
            let ay = u32::from(y) * 0x7FFF / (SCREEN.1 - 1);
            qmp.touch(Some((ax, ay)))?;
            Ok(format!("touch down at {x},{y}"))
        }
        Action::TouchUp => {
            qmp.touch(None)?;
            Ok("touch up".into())
        }
        Action::Hmp(_) => unreachable!("not a touch"),
    }
}

/// Whether `output` has a `TOUCH x y` line within 2 pixels of `(x, y)`.
fn touch_near(output: &str, x: u16, y: u16) -> bool {
    output.lines().any(|line| {
        let mut it = line.trim().strip_prefix("TOUCH ").unwrap_or("").split(' ');
        match (
            it.next().and_then(|v| v.parse::<u16>().ok()),
            it.next().and_then(|v| v.parse::<u16>().ok()),
        ) {
            (Some(lx), Some(ly)) => lx.abs_diff(x) <= 2 && ly.abs_diff(y) <= 2,
            _ => false,
        }
    })
}

#[cfg(test)]
mod tests {
    /// Every file of the emulator harness (emu/), for the tests that read
    /// its source.
    fn sources() -> &'static str {
        static ALL: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        ALL.get_or_init(|| {
            [
                include_str!("mtp.rs"),
                include_str!("drive.rs"),
                include_str!("card.rs"),
                include_str!("usblink.rs"),
                include_str!("audio.rs"),
                include_str!("loader.rs"),
                include_str!("display.rs"),
                include_str!("gpu.rs"),
                include_str!("gpubench.rs"),
                include_str!("look.rs"),
                include_str!("radar.rs"),
                include_str!("stacks.rs"),
                include_str!("perf.rs"),
                include_str!("today.rs"),
                // Last: its tests quote the names the others define.
                include_str!("mod.rs"),
            ]
            .concat()
        })
    }

    /// R186: a run refusing on a fork whose clocks start at 0 is clean
    /// only if nothing else ran; a refusal beside measured renders, or at
    /// Windows Mobile's own COREPLL, is not.
    #[test]
    fn a_gpubench_clock_refusal_counts_only_when_nothing_else_ran() {
        let refused = "  as found: COREPLL 0x0, MBXCLK 0x0, MSC1 0x24842481
  not Windows Mobile's clock and bus timing: nothing changed
RETROPANE-GPUBENCH: done, 0 renders measured, 0 hung renders recovered
";
        assert!(gpubench_refused_clocks(refused));
        assert!(!gpubench_refused_clocks(
            &refused.replace("0 renders measured", "12 renders measured")
        ));
        assert!(!gpubench_refused_clocks(
            &refused.replace("COREPLL 0x0,", "COREPLL 0xbb1,")
        ));
    }

    /// A reference without its quad passes only where the fork can't draw
    /// the way's object forms (or G4's way 2); a modelled way losing its
    /// quad fails, and an unmodelled way showing one fails too (R58).
    #[test]
    fn glesfeat_no_quad_is_judged_way_by_way() {
        let modelled = "\nG2 way 1 (ours):\n    list word 0x40800000 (layout 4), object: e0000000; background: list word 0x40800000, 17 words\n  reference at Windows Mobile's clock: rendered true then true\n";
        // G3's way 2: a four-corner background, its word logged unlabelled.
        let quad_bg = "\nG3 way 2 (Dell's clear (a flat quad)):\n    list word 0x40800000 (layout 4), object: e0000000; background: list word 0x60a00000, 22 words\n  the reference holds no quad\n  reference at Windows Mobile's clock: rendered true then true\n";
        assert_eq!(
            super::glesfeat_no_quad(&format!("test G3{quad_bg}"), "G3", 3),
            Ok(1)
        );
        let corner = "\nG2 way 2 (one 4-corner object):\n    list word 0x60a00000 (layout 5), object: e0000000; background: list word 0x40800000, 17 words\n  the reference holds no quad (its background alone)\n  reference at Windows Mobile's clock: rendered true then true\n";
        let clear = "\nG2 way 4 (the clear underneath):\n    list word 0x40800000 (layout 4), object: e0000000; background: list word 0x40000000, 10 words\n  the reference holds no quad (its background alone)\n  reference at Windows Mobile's clock: rendered true then true\n";
        let log = format!("test G2{modelled}{corner}{clear}");
        assert_eq!(super::glesfeat_no_quad(&log, "G2", 2), Ok(2));
        // A modelled way that lost its quad.
        let lost = modelled.replace(
            "  reference at",
            "  the reference holds no quad\n  reference at",
        );
        assert!(super::glesfeat_no_quad(&format!("test G2{lost}"), "G2", 2).is_err());
        // An unmodelled way that showed one (the fork drew it after all).
        let shown = corner.replace("  the reference holds no quad (its background alone)\n", "");
        assert!(super::glesfeat_no_quad(&format!("test G2{shown}"), "G2", 2).is_err());
        // An unmodelled way that never rendered (the TA silent): no refusal.
        let silent = clear
            .replace("  the reference holds no quad (its background alone)\n", "")
            .replace("rendered true then true", "rendered false then false");
        assert_eq!(
            super::glesfeat_no_quad(&format!("test G2{silent}"), "G2", 2),
            Ok(0)
        );
        // G4's way 2 on any form.
        let g4 = modelled.replace("G2 way 1", "G4 way 2").replace(
            "  reference at",
            "  the reference holds no quad\n  reference at",
        );
        assert_eq!(
            super::glesfeat_no_quad(&format!("test G4{g4}"), "G4", 4),
            Ok(1)
        );
    }

    /// Every scratch file and raw binary goes through the helpers that
    /// create their folders: a check that built a path by hand failed in
    /// any fresh checkout ("no screenshot at ...ppm", QEMU can't make the
    /// folder), or relied on a file an earlier check left in target/emu.
    #[test]
    fn emulator_files_go_through_the_helpers() {
        let src = sources();
        let code = src.split("#[cfg(test)]\nmod tests").next().expect("code");
        let code: String = code
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for (needle, allowed) in [
            ("\"target/emu", 0),
            ("join(\"emulator\")", 1), // emulator_dir
            ("join(\"emu\")", 1),      // emu_dir
        ] {
            assert_eq!(
                code.matches(needle).count(),
                allowed,
                "`{needle}` outside emulator_dir/emu_dir: use scratch(), emu_dir() or raw_image()"
            );
        }
    }

    /// The load is bounded twice over: a fixed number of spinners, each
    /// ended by `timeout`.
    #[test]
    fn the_load_is_bounded() {
        let s = super::load_script(45_441, 90);
        assert!(s.starts_with("for i in $(seq $(nproc)); do "), "{s}");
        assert!(s.contains("timeout 90 bash"), "{s}");
        assert!(s.contains(&format!("{}-45441;", super::LOAD_TAG)), "{s}");
        assert_eq!(s.matches("while").count(), 1, "{s}");
    }

    /// Whether `pkill -f`'s pattern (ERE; only the `.`, `.*` and `[c]`
    /// these patterns use) matches somewhere in `line`.
    fn ere_finds(pat: &str, line: &str) -> bool {
        fn here(p: &[u8], t: &[u8]) -> bool {
            match p {
                [] => true,
                [b'.', b'*', rest @ ..] => (0..=t.len()).any(|i| here(rest, &t[i..])),
                [b'[', c, b']', rest @ ..] => t.first() == Some(c) && here(rest, &t[1..]),
                [b'.', rest @ ..] => !t.is_empty() && here(rest, &t[1..]),
                [c, rest @ ..] => t.first() == Some(c) && here(rest, &t[1..]),
            }
        }
        (0..=line.len()).any(|i| here(pat.as_bytes(), &line.as_bytes()[i..]))
    }

    /// The `pkill -f '...'` pattern in a cleanup command.
    fn pattern(cmd: &str) -> &str {
        cmd.split('\'').nth(1).expect("quoted pattern")
    }

    /// A run's cleanup ends its own emulator and load, never another
    /// session's run (one ended rp-blend mid-check), nor the shell running
    /// the pkill.
    #[test]
    fn cleanup_ends_only_this_runs_processes() {
        let qemu = |port: u16| {
            format!(
                "timeout 70 /home/u/retropane-qemu/build/qemu-system-arm -M x51v -display none -serial tcp:127.0.0.1:{},server=on,wait=on -monitor tcp:127.0.0.1:{port},server,nowait -qmp tcp:127.0.0.1:{},server,nowait -kernel '/mnt/c/x/rp-loader.bin'",
                port - 1,
                port + 1
            )
        };
        let stop = super::stop_own_qemu(45_441);
        assert!(!stop.contains("$("), "it goes through wsl_bash: {stop}");
        let p = pattern(&stop);
        assert!(ere_finds(p, &qemu(45_441)), "{p}");
        assert!(!ere_finds(p, &qemu(45_461)), "another run's emulator");
        assert!(!ere_finds(p, &qemu(4_544)), "a port that is a prefix");
        assert!(!ere_finds(p, &format!("bash -c {stop}")), "its own shell");
        // One deaf to the TERM is killed (it outlived TERM for minutes).
        assert!(stop.contains("pkill -KILL -f "), "{stop}");
        assert!(
            stop.split("pkill").skip(1).all(|k| pattern(k) == p),
            "every kill is this run's: {stop}"
        );

        let spinner = |port: u16| {
            let s = super::load_script(port, 90);
            let body = s.split("bash -c '").nth(1).expect("spinner");
            format!("bash -c {}", body.split('\'').next().expect("body"))
        };
        let stop = super::stop_own_load(45_441);
        let p = pattern(&stop);
        assert!(ere_finds(p, &spinner(45_441)), "{p}");
        assert!(!ere_finds(p, &spinner(45_461)), "another run's load");
        assert!(!ere_finds(p, &format!("bash -c {stop}")), "its own shell");

        // No other pkill: every kill goes through these two.
        let src = sources();
        let code = src.split("#[cfg(test)]\nmod tests").next().expect("code");
        let kills = code
            .lines()
            .filter(|l| !l.trim_start().starts_with("//") && l.contains("pkill"))
            .count();
        assert_eq!(kills, 2, "pkill outside stop_own_qemu / stop_own_load");
    }

    /// The body of `pub fn name` in this file.
    fn body(name: &str) -> &'static str {
        let src = sources();
        let start = src.find(&format!("pub fn {name}(")).expect(name);
        let body = &src[start..];
        &body[..body.find("\n}\n").expect("end")]
    }

    /// The threads image's checks are timer deadlines in its own time (a
    /// 5 ms quantum): on host time a busy PC ended a quantum before the
    /// first thread reached its system call.
    #[test]
    fn threads_keep_instruction_time() {
        assert!(body("boot_threads").contains("\"-icount\""));
    }

    /// No check bounds the guest's work by the PC's wall clock: a busy PC
    /// made the drive's 1 MiB read take 8.7 s against an 8 s limit. The
    /// drive check reads the loader's own count of naps mid-command (what
    /// the limit was there to catch), which the loader must report. The
    /// one wall-clock bound left is the idle loader's answer (500 ms; 26 ms
    /// with every core busy), which is about the PC's round trip.
    /// No check waits by a fixed sleep of half a second or more on the PC's
    /// clock (quality audit C2): a wait is on the image (a serial marker,
    /// the screen, a register), in the guest's time (`guest_hold`,
    /// `GuestClock`), or a short pause inside a wait loop (`poll_pause`).
    #[test]
    fn no_fixed_sleeps_of_half_a_second_or_more() {
        let src = sources();
        let code = src
            .split(
                "#[cfg(test)]
mod tests",
            )
            .next()
            .expect("code");
        let mut bad = Vec::new();
        let mut in_poll = false;
        for (i, line) in code.lines().enumerate() {
            // Moved whole into audio.rs, it reads as pub(crate) there.
            if line.starts_with("fn poll_pause") || line.starts_with("pub(crate) fn poll_pause") {
                in_poll = true;
            } else if line.starts_with('}') {
                in_poll = false;
            }
            let Some(at) = line.find("thread::sleep(") else {
                continue;
            };
            let arg = &line[at + "thread::sleep(".len()..];
            let ms = if let Some(v) = arg.strip_prefix("Duration::from_millis(") {
                v.split(')')
                    .next()
                    .and_then(|n| n.replace('_', "").parse::<u64>().ok())
            } else if let Some(v) = arg.strip_prefix("Duration::from_secs(") {
                v.split(')')
                    .next()
                    .and_then(|n| n.replace('_', "").parse::<u64>().ok())
                    .map(|s| s * 1000)
            } else {
                None
            };
            match ms {
                Some(ms) if ms < 500 => {}
                _ if in_poll => {}
                _ => bad.push(format!("line {}: {}", i + 1, line.trim())),
            }
        }
        assert!(
            bad.is_empty(),
            "fixed sleeps (wait on the guest instead): {bad:#?}"
        );
    }

    #[test]
    fn guest_work_is_not_timed_on_the_wall_clock() {
        let drive = body("boot_loader_drive");
        assert!(
            drive.contains("naps mid-command 0,"),
            "drive checks the count"
        );
        assert!(!drive.contains("elapsed() >"), "drive: no wall-clock bound");
        let loader = include_str!("../../../boot/loader/src/main.rs");
        assert!(
            loader.contains("naps mid-command {}"),
            "the loader reports it"
        );
        let src = sources();
        let code = src.split("#[cfg(test)]\nmod tests").next().expect("code");
        let bounds = code.matches("elapsed() >").count() + code.matches("took >").count();
        assert_eq!(
            bounds, 1,
            "a new wall-clock bound: wait on the guest instead"
        );
    }

    /// emu-today holds every press until the screen answers it: no stylus
    /// action with a fixed sleep (a busy host starved the align screen of
    /// samples in a 700 ms hold); and its guest keeps instruction time, so
    /// a starved guest can't overrun the run limit.
    #[test]
    fn a_press_answer_counts_only_near_the_pen() {
        let w = rp_today::W;
        let blank = vec![0u8; w * rp_today::H * 3];
        let mut green = blank.clone();
        // The centre target turning green (inside the masked clock areas
        // of the Today screen, 2026-10-18), and a clock tick far away.
        green[(320 * w + 240) * 3 + 1] = 0xFF;
        assert_eq!(super::press_answer(&green, &blank, 240, 320), 1);
        let mut tick = blank.clone();
        tick[(10 * w + 460) * 3] = 0xFF;
        assert_eq!(super::press_answer(&tick, &blank, 240, 320), 0);
    }

    #[test]
    fn today_presses_wait_for_the_screen() {
        let src = sources();
        let start = src.find("pub fn boot_loader_today").expect("today");
        let body = &src[start..];
        let body = &body[..body.find("\n}\n").expect("end")];
        assert_eq!(
            body.matches("Action::TouchDown").count(),
            1,
            "only in press"
        );
        assert_eq!(body.matches("Action::TouchUp").count(), 1, "only in press");
        let press = &body[body.find("let press =").expect("press")..];
        let press = &press[..press.find("};").expect("press end")];
        assert!(!press.contains("sleep"), "press must wait on the screen");
        assert!(body.contains("\"-icount\""), "guest time from instructions");
    }

    #[test]
    fn the_stack_check_passes_80_percent_and_fails_above() {
        use super::check_stack;
        assert_eq!(check_stack("no report").unwrap(), None);
        // The guard words written (quality audit C27, C90): a failure
        // whatever the depth says.
        assert!(check_stack("stack: deepest 100 of 16384 bytes - OVERFLOWED: the guard words at its bottom were written").is_err());
        assert_eq!(
            check_stack("stack: deepest 6108 of 12288 bytes\n").unwrap(),
            Some((6108, 12288))
        );
        assert!(check_stack("stack: deepest 8192 of 10240 bytes").is_ok());
        assert!(check_stack("stack: deepest 23480 of 24576 bytes").is_err());
        // Two runs in one output: either over 80% fails.
        assert!(
            check_stack("stack: deepest 100 of 16384 bytes\nstack: deepest 15000 of 16384 bytes")
                .is_err()
        );
        assert!(check_stack("stack: deepest lots of room").is_err());
    }

    /// Quality audit C40: the folder watched for a `local/` file is the
    /// nearest that exists, so a file (or folder, or link) arriving with
    /// an old time still changes something cargo watches.
    #[test]
    fn a_local_file_is_watched_through_the_nearest_existing_folder() {
        mod local_input {
            include!("../../../tools/build/local_input.rs");
            pub fn nearest(p: &std::path::Path) -> Option<std::path::PathBuf> {
                nearest_existing_dir(p)
            }
        }
        let root = std::env::temp_dir().join(format!("rp-c40-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("local/concept/picture.rgb565");
        // No local/: the root itself.
        assert_eq!(local_input::nearest(&file), Some(root.clone()));
        std::fs::create_dir_all(root.join("local")).unwrap();
        assert_eq!(local_input::nearest(&file), Some(root.join("local")));
        std::fs::create_dir_all(root.join("local/concept")).unwrap();
        assert_eq!(
            local_input::nearest(&file),
            Some(root.join("local/concept"))
        );
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(
            local_input::nearest(&file),
            Some(root.join("local/concept"))
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Quality audit C40's last case: a picture overwritten in place by an
    /// older-dated copy changes the fingerprint the build scripts watch;
    /// a file outside their folders doesn't.
    #[test]
    fn an_older_copy_over_a_local_picture_changes_the_fingerprint() {
        let root = std::env::temp_dir().join(format!("rp-c40b-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let none = crate::local_inputs(&root);
        std::fs::create_dir_all(root.join("local/concept")).unwrap();
        let file = root.join("local/concept/picture.rgb565");
        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
        let write = |bytes: &[u8], time: std::time::SystemTime| {
            std::fs::write(&file, bytes).unwrap();
            std::fs::File::options()
                .write(true)
                .open(&file)
                .unwrap()
                .set_modified(time)
                .unwrap();
        };
        write(b"first", old);
        let first = crate::local_inputs(&root);
        assert_ne!(first, none);
        assert_eq!(
            crate::local_inputs(&root),
            first,
            "steady when nothing changes"
        );
        // Same length, an even older time: cargo's time check misses this.
        write(b"other", old - std::time::Duration::from_secs(86_400));
        let second = crate::local_inputs(&root);
        assert_ne!(second, first);
        // A run log in local/ isn't an input.
        std::fs::write(root.join("local/rp-today-run.txt"), b"log").unwrap();
        assert_eq!(crate::local_inputs(&root), second);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Quality audit C22: a scale's first window read too high (part B's
    /// onset, 2026-10-22: one window of note 4, then note 1) is not a
    /// replay; a quarter second of earlier notes still is.
    #[test]
    fn e3_rising_ignores_one_wrong_window_and_catches_a_replay() {
        use super::Rising;
        let run = |notes: &[Option<usize>]| -> Option<(usize, usize)> {
            let mut r = Rising::default();
            for (k, n) in notes.iter().enumerate() {
                match n {
                    Some(n) => {
                        if let Some(high) = r.note(*n) {
                            return Some((k, high));
                        }
                    }
                    None => r.other(),
                }
            }
            None
        };
        // Five windows a note (100 ms of 20 ms windows), notes from `from`.
        let scale = |from: usize, to: usize| -> Vec<Option<usize>> {
            (from..to).flat_map(|n| [Some(n); 5]).collect()
        };
        // The failing run's onset: silence, note 4 once, then the scale.
        let mut onset = vec![None, None, Some(3)];
        onset.extend(scale(0, 24));
        assert_eq!(run(&onset), None);
        // A join window between notes, read high or low.
        let mut joins = scale(0, 6);
        joins.insert(10, Some(9));
        joins.insert(20, Some(0));
        assert_eq!(run(&joins), None);
        // A replay: the ring's last quarter second again after note 12.
        let mut replay = scale(0, 12);
        replay.extend(scale(7, 12));
        assert_eq!(run(&replay), Some((61, 11)));
    }

    /// Run FI's check reads the X51v's own lines (local/rp-gpubench-runFI.txt).
    #[test]
    fn run_fi_reads_the_render_done_words() {
        let device = "  FI enables as found (0x00000000): idle 0x00000000; 2D busy 0x00820000, done (true) 0x00000000, cleared 0x00000000; 3D started true: busy 0x00000000, done 0x0000000c, after the renderer's poll 0x00000048, cleared 0x00000000
  FI enables 0 (0x00000000): idle 0x00000000; 2D busy 0x00820000, done (true) 0x00000000, cleared 0x00000000; 3D started true: busy 0x00000000, done 0x0000004c, after the renderer's poll 0x00000048, cleared 0x00000000
  FI enables 3D false";
        assert_eq!(super::fi_render_done(device), vec![0x0c, 0x4c]);
        assert!(super::fi_render_done("3D started true: busy 0x0, done 0xzz").is_empty());
    }

    #[test]
    fn a_wsl_build_is_found_by_its_source_folder() {
        use super::wsl_source_dir;
        assert_eq!(
            wsl_source_dir("$HOME/retropane-qemu-029/build/qemu-system-arm"),
            Some("$HOME/retropane-qemu-029")
        );
        assert_eq!(wsl_source_dir("/usr/bin/qemu-system-arm"), None);
        assert_eq!(wsl_source_dir("/build/qemu-system-arm"), None);
    }

    #[test]
    fn mbx_blend_b7_compares_all_but_bit_21() {
        use super::{MBX_BLEND_B7_DEVICE, MBX_BLEND_B7_UNMODELLED, mbx_blend_b7_mismatches};
        let lines: Vec<&str> = MBX_BLEND_B7_DEVICE
            .lines()
            .filter(|l| !l.starts_with('#'))
            .collect();
        // 3 conversion lines, 6 blend lines, 30 tries.
        assert_eq!(lines.len(), 3 + 6 + 30);
        let log = lines.join("\n");
        assert!(mbx_blend_b7_mismatches(&log, MBX_BLEND_B7_DEVICE).is_empty());
        // One channel's count off: caught.
        let off = log.replacen("truncates 256", "truncates 255", 1);
        assert_eq!(mbx_blend_b7_mismatches(&off, MBX_BLEND_B7_DEVICE).len(), 1);
        // A try missing: caught.
        let short: String = lines
            .iter()
            .filter(|l| !l.starts_with("B7c blend word 0x02000001:"))
            .map(|l| format!("{l}\n"))
            .collect();
        assert_eq!(
            mbx_blend_b7_mismatches(&short, MBX_BLEND_B7_DEVICE).len(),
            1
        );
        // Bit 21's line differing (the emulator writes the copy): not counted.
        let bit21: String = lines
            .iter()
            .filter(|l| !l.starts_with(MBX_BLEND_B7_UNMODELLED))
            .map(|l| format!("{l}\n"))
            .collect();
        assert!(mbx_blend_b7_mismatches(&bit21, MBX_BLEND_B7_DEVICE).is_empty());
    }

    #[test]
    fn mbx_blend_device_matches_itself_and_catches_a_pixel() {
        use super::{MBX_BLEND_DEVICE, mbx_blend_mismatches};
        let log: String = MBX_BLEND_DEVICE
            .lines()
            .map(|l| format!("      {l}\n"))
            .collect();
        assert_eq!(
            mbx_blend_mismatches(&log, MBX_BLEND_DEVICE),
            Vec::<String>::new()
        );
        // An unblended (untouched) row, as an emulator without the model gives.
        let off = log.replace(
            "B2 0x09100000: 009d 0217 05a8 0722",
            "B2 0x09100000: 001f 001f 001f 001f",
        );
        assert_eq!(mbx_blend_mismatches(&off, MBX_BLEND_DEVICE).len(), 1);
        // The unknown pixels are not compared.
        let free = log.replace(
            "B3 code 12 src16 plain: 0060 0460 946d dc76",
            "B3 code 12 src16 plain: 0060 0460 0000 0000",
        );
        assert!(mbx_blend_mismatches(&free, MBX_BLEND_DEVICE).is_empty());
        // The frame within 0.2% (device run 2), but not a line short.
        let run2 = log.replace(
            "17712 us a frame (56458 mHz), 27460 ns",
            "17710 us a frame (56465 mHz), 27457 ns",
        );
        assert!(mbx_blend_mismatches(&run2, MBX_BLEND_DEVICE).is_empty());
        let short = log.replace("lines 0..644 (645)", "lines 0..643 (644)");
        assert_eq!(mbx_blend_mismatches(&short, MBX_BLEND_DEVICE).len(), 1);
        // A VEVENT on the wrong line.
        let late = log.replace(
            "first at lines [400, 400, 400]",
            "first at lines [403, 403, 405]",
        );
        assert_eq!(mbx_blend_mismatches(&late, MBX_BLEND_DEVICE).len(), 1);
    }

    use super::*;

    #[test]
    fn probe_checksum_matches_the_image() {
        // Same vector as rp-probe's report test.
        let sum: u32 = b"CPU ID 69054117".iter().map(|&b| u32::from(b)).sum();
        assert_eq!(
            u32::from(probe_checksum("CPU ID 69054117      ")),
            sum % 256
        );
    }

    /// Every USB chardev a test names fits in the run's port block, so no
    /// test reaches past it into another run's ports.
    #[test]
    fn usb_ports_fit_the_port_block() {
        let src = sources();
        let mut seen = 0;
        for call in [
            "usb_connect(",
            "usb_args(",
            "msc_args(",
            "mtp_args(",
            "usb_port(",
            "Initiator::connect(",
        ] {
            for rest in src.split(call).skip(1) {
                let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
                if let Ok(n) = digits.parse::<u16>() {
                    assert!(n < USB_PORTS, "{call}{n}): past the port block");
                    seen += 1;
                }
            }
        }
        assert!(seen > 10, "the scan found the calls");
        assert!(PORT_FIRST as u32 + PORT_BLOCK as u32 * PORT_BLOCKS as u32 <= 49_152);
    }

    #[test]
    fn probe_report_checks() {
        let line = |p: u8, t: &str| format!("P{p}|{t:<37} {:02X}", probe_checksum(t));
        let mut out = String::new();
        for (p, t) in [
            (1, "1 SYSTEM"),
            (2, "2 WM STATE"),
            (3, "3 2700G"),
            (4, "4 BENCH"),
        ] {
            out += &line(p, t);
            out.push('\n');
        }
        for t in PROBE_EXPECT {
            out += &line(4, t);
            out.push('\n');
        }
        assert_eq!(check_probe_report(&out), Ok(4 + PROBE_EXPECT.len()));
        let bad = out.replacen("1 SYSTEM", "1 SYST3M", 1);
        assert!(check_probe_report(&bad).unwrap_err().contains("checksum"));
        // A failed check fails the report, unless it's a listed emulator
        // limit.
        let failed = out.clone()
            + &line(4, "check mem FAIL short")
            + "
";
        assert!(
            check_probe_report(&failed)
                .unwrap_err()
                .contains("`mem` failed")
        );
        let known = out
            + &line(4, "check wmmx_mix FAIL differs from")
            + "
";
        // rp-v0.30.0 models it: no longer a limit.
        assert!(check_probe_report(&known).is_err());
        assert_eq!(
            check_probe_report_with(&known, &[("wmmx_mix", "a known limit")]),
            Ok(5 + PROBE_EXPECT.len())
        );
    }

    #[test]
    fn parses_bench_lines() {
        let r = parse_bench("noise\nBENCH memcpy_64k 3250 10\nBENCH bad x 1\nBENCH-DONE\n");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].name, "memcpy_64k");
        assert!((r[0].ns - 100_000.0).abs() < 1e-6); // 3250 ticks = 1 ms, / 10
    }

    #[test]
    fn the_emulator_never_plays_sound_out_loud() {
        let file = "wav,id=rpsilent,path=t.wav";
        let silent = silent_unless_recorded(&["-usb"], file);
        assert_eq!(
            silent,
            ["-usb", "-audiodev", file, "-machine", "audiodev=rpsilent"]
        );
        // A check that records keeps its own backend.
        let recorded = [
            "-audiodev",
            "wav,id=snd,path=x.wav",
            "-machine",
            "audiodev=snd",
        ];
        assert_eq!(silent_unless_recorded(&recorded, file), recorded);
    }

    #[test]
    fn touch_lines_match_within_tolerance() {
        assert!(touch_near("KEY Up up\nTOUCH 119 161\n", 120, 160));
        assert!(!touch_near("TOUCH 110 160\n", 120, 160));
        assert!(!touch_near("TOUCH up\n", 120, 160));
    }

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
