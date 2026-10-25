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
pub const QEMU_RELEASE: &str = "rp-v0.30.0";
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
const PORT_BLOCK: u16 = 20;
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

/// Boots the resident loader the way HaRET does (a raw binary away from its
/// home, so it must move itself), sends it the first-light image over the
/// serial port, and checks that image runs: relocation, the protocol, the
/// CRC check and the hand-off, end to end.
pub fn boot_loader(root: &Path) -> Result<(), String> {
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    let loader = dir.join("rp-loader.bin");
    let light = dir.join("rp-light.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-light"), &light)?;
    let payload = std::fs::read(&light).map_err(|e| format!("{}: {e}", light.display()))?;
    let frame = rp_loadproto::frame(&payload);
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&frame, "RPL END")),
            during: None,
            then: None,
            timeout: BOOT_TIMEOUT,
        },
    )?;
    if !output.contains("RPL OK") {
        return Err("loader: no `RPL OK` before the image started".into());
    }
    check_boot(&output, &["boot_loader_ready"])?;
    // The run log the loader sent after first light returned to it.
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("loader: no run log after the image returned")?;
    if !log.contains("all steps OK") || !log.contains(LIGHT_MARKER) {
        return Err(format!("loader: the run log is incomplete:{log}"));
    }
    eprint!("    run log as the PC receives it:{log}");
    eprintln!(
        "    loader OK: moved itself home, took a {} byte image over serial, CRC checked, \
         started it, and sent its run log back when it returned",
        payload.len()
    );
    Ok(())
}

/// Hand-over facts (items 72 and 77): `rp-handover` under the loader, with
/// Windows Mobile's CPLD shadow table in RAM as at HaRET's hand-off. The
/// loader adopts the table first (its start), so the image finds
/// RetroPane's table with Windows Mobile's bits in it; the hand-off record
/// shows no DMA left running (the loader stops its own) and the 2700G
/// handed out with no fixed wait.
pub fn boot_loader_handover(root: &Path) -> Result<(), String> {
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let loader = dir.join("rp-loader.bin");
    let image = dir.join("rp-handover.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-handover"), &image)?;
    let payload = std::fs::read(&image).map_err(|e| format!("{}: {e}", image.display()))?;
    let frame = rp_loadproto::frame(&payload);
    let args = wm_cpld_table_args();
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &extra,
            input: Some((&frame, "RPL END")),
            during: None,
            then: None,
            timeout: BOOT_TIMEOUT,
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("handover: no run log after the image returned")?;
    for want in [
        "handoff: DMA running 00000000, stopped",
        "table @A8001320: 53435052",
        "shadow from RetroPane's table:",
        " 02=00FD",
        " 10=1000",
        " 12=000F",
        " 16=0003",
        " 18=0004",
        " 1C=000C",
        "HANDOVER-OK",
    ] {
        if !log.contains(want) {
            return Err(format!("handover: no {want:?} in the run log:{log}"));
        }
    }
    eprint!("    run log:{log}");
    eprintln!(
        "    hand-over OK: no fixed wait for the 2700G; the CPLD shadow adopted from Windows Mobile's table by the loader, then shared with the image"
    );
    Ok(())
}

/// The CF detect diagnostic (`rp-cfdetect`) under the loader: it reads the
/// three detect signals as found, with the slot off through its 30 s watch,
/// and powered again, logs each, and returns with the slot powered. (The
/// emulator's slot is empty and its GPIO13 works unpowered, so this checks
/// the image's run, not the device's answer: docs/hw/emulator.md.)
pub fn boot_loader_cfdetect(root: &Path) -> Result<(), String> {
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let loader = dir.join("rp-loader.bin");
    let image = dir.join("rp-cfdetect.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-cfdetect"), &image)?;
    let payload = std::fs::read(&image).map_err(|e| format!("{}: {e}", image.display()))?;
    let frame = rp_loadproto::frame(&payload);
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&frame, "RPL END")),
            during: None,
            then: None,
            timeout: BOOT_TIMEOUT + Duration::from_secs(40),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("cfdetect: no run log after the image returned")?;
    for want in [
        " ms as found: GPIO13 ",
        " ms slot off: GPIO13 ",
        "PULL THE CF CARD OUT NOW",
        "PUSH IT BACK IN NOW",
        " ms end, slot off: GPIO13 ",
        " ms powered 1 s: GPIO13 ",
        "CFDETECT-OK",
    ] {
        if !log.contains(want) {
            return Err(format!("cfdetect: no {want:?} in the run log:{log}"));
        }
    }
    eprintln!("    cf detect OK: every stage ran and was logged, back at the loader");
    Ok(())
}

/// The night-light experiment (item 76) under the loader: every step runs
/// and the channel is put back. The emulator's 2700G applies the graphics
/// gamma table at scan-out (Windows Mobile's grey ramp at reset, gamma off
/// as on a cold display: the image switches it on), so the picture steps
/// run: the screen's grey ramp is seen at full, 1/2 and 1/4 through the
/// table, pixel for pixel as RetroPane's reference (`scanout::gamma` of
/// `scale_gamma`).
pub fn boot_loader_nightlight(root: &Path) -> Result<(), String> {
    use rp_drv_mbx_display::scanout::{WM_GAMMA, gamma, scale_gamma};
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let loader = dir.join("rp-loader.bin");
    let image = dir.join("rp-nightlight.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-nightlight"), &image)?;
    let payload = std::fs::read(&image).map_err(|e| format!("{}: {e}", image.display()))?;
    let frame = rp_loadproto::frame(&payload);
    let shot = scratch(root, "nightlight-screen.ppm")?;
    let guest = match emulator(root)? {
        Emulator::Wsl => to_wsl_path(&shot),
        Emulator::Native(_) => shot.display().to_string(),
    };
    // The image's ramp: grey bar `i` (of 16, 30 pixels wide) from y = 400,
    // in RGB565, then through the table scaled to `picture`/256.
    let bar = |i: u32, picture: u32| {
        let g8 = i * 255 / 15;
        let (r, g) = (((g8 >> 3) << 3) as u8, ((g8 >> 2) << 2) as u8);
        let t = scale_gamma(&WM_GAMMA, picture);
        (gamma(&t, r, 16), gamma(&t, g, 8), gamma(&t, r, 0))
    };
    let pictures = [256, 128, 64, 32];
    let check = || -> Result<(), String> {
        let mut seen = [false; 4];
        let mut last = None;
        let until = Instant::now() + Duration::from_secs(120);
        while !seen.iter().all(|s| *s) {
            if Instant::now() > until {
                return Err(format!(
                    "nightlight: picture steps not all seen through the gamma table (full, 1/2, 1/4, 1/8: {seen:?}); last bars 15 and 8: {last:?}, want {:?}",
                    pictures.map(|p| (bar(15, p), bar(8, p)))
                ));
            }
            let _ = std::fs::remove_file(&shot);
            let mut mon = crate::screen::Monitor::connect(monitor_port())?;
            mon.screendump(&guest, &shot)?;
            let img = crate::screen::parse_ppm(&std::fs::read(&shot).map_err(|e| e.to_string())?)?;
            if img.width >= 480 && img.height >= 640 {
                let px = |x: u32, y: u32| {
                    let o = ((y * img.width + x) * 3) as usize;
                    (img.rgb[o], img.rgb[o + 1], img.rgb[o + 2])
                };
                let got = (px(15 * 30 + 15, 520), px(8 * 30 + 15, 520));
                for (k, p) in pictures.iter().enumerate() {
                    if got == (bar(15, *p), bar(8, *p)) {
                        seen[k] = true;
                    }
                }
                last = Some(got);
            }
            poll_pause(Duration::from_millis(500));
        }
        Ok(())
    };
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&frame, "RPL OK")),
            during: Some(&check),
            then: Some("RPL END"),
            timeout: Duration::from_secs(600),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("nightlight: no run log after the image returned")?;
    for want in [
        "gamma: off, switched on for the steps",
        "step 4: 1/432, picture 1/2",
        "step 5: 1/432, picture 1/4",
        "step 8: 108/216 (WM): whine?",
        "NIGHTLIGHT-OK",
    ] {
        if !log.contains(want) {
            return Err(format!("nightlight: no {want:?} in the run log:{log}"));
        }
    }
    if log.contains("skipped") {
        return Err(format!("nightlight: a step was skipped:{log}"));
    }
    eprintln!(
        "    night light OK: every step ran; the picture steps dimmed the screen through Windows Mobile's gamma table (full, 1/2, 1/4, 1/8), pixel for pixel as the reference"
    );
    Ok(())
}

/// The display calibration test image (docs/design/display-calibration.md)
/// under the loader, as the guided test it is: its picture is pixel-exact
/// below the caption through each gamma table and saturation its steps
/// set (the emulator's 2700G applies the table at scan-out, Windows
/// Mobile's grey ramp at reset). Step 1 right (Black 26); down to step 2,
/// where every other dial is back at 25 (the original), right (White 26);
/// down to step 6, right (Sat 26: the picture redrawn); down to step 7,
/// the starting look, right (the next look); centre (the original); Home
/// puts everything back.
pub fn boot_loader_display_cal(root: &Path) -> Result<(), String> {
    use rp_display_cal::{
        CAPTION_H, Dials, H, PRESET_START, PRESETS, W, look_table, pixel_sat, table,
    };
    use rp_drv_mbx_display::scanout::{GAMMA_ENTRIES, WM_GAMMA, gamma};
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let loader = dir.join("rp-loader.bin");
    let image = dir.join("rp-display-cal.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-display-cal"), &image)?;
    let payload = std::fs::read(&image).map_err(|e| format!("{}: {e}", image.display()))?;
    let frame = rp_loadproto::frame(&payload);
    let shot = scratch(root, "display-cal-screen.ppm")?;
    let guest = match emulator(root)? {
        Emulator::Wsl => to_wsl_path(&shot),
        Emulator::Native(_) => shot.display().to_string(),
    };
    // The picture below the caption at saturation `sat`, through `t`.
    let want = |t: &[u32; GAMMA_ENTRIES], sat: u32| -> Vec<u8> {
        let mut v = Vec::with_capacity((W * (H - CAPTION_H) * 3) as usize);
        for y in CAPTION_H..H {
            for x in 0..W {
                let (r, g, b) = pixel_sat(x, y, sat);
                v.extend_from_slice(&[gamma(t, r, 16), gamma(t, g, 8), gamma(t, b, 0)]);
            }
        }
        v
    };
    let until_table = |name: &str, t: &[u32; GAMMA_ENTRIES], sat: u32| -> Result<(), String> {
        let want = want(t, sat);
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let _ = std::fs::remove_file(&shot);
            let mut mon = crate::screen::Monitor::connect(monitor_port())?;
            mon.screendump(&guest, &shot)?;
            let img = crate::screen::parse_ppm(&std::fs::read(&shot).map_err(|e| e.to_string())?)?;
            let mut differ = usize::MAX;
            if img.width >= W && img.height >= H {
                let mut got = Vec::with_capacity(want.len());
                for y in CAPTION_H..H {
                    let o = ((y * img.width) * 3) as usize;
                    got.extend_from_slice(&img.rgb[o..o + (W * 3) as usize]);
                }
                differ = got
                    .chunks(3)
                    .zip(want.chunks(3))
                    .filter(|(a, b)| a != b)
                    .count();
                if differ == 0 {
                    return Ok(());
                }
            }
            if Instant::now() > deadline {
                let _ = std::fs::copy(&shot, scratch(root, &format!("display-cal-{name}.ppm"))?);
                return Err(format!(
                    "display-cal: the {name} screen differs from the reference in {differ} pixels (target/emulator/display-cal-{name}.ppm)"
                ));
            }
            std::thread::sleep(Duration::from_millis(400));
        }
    };
    let until = |name: &str, d: Dials| until_table(name, &table(d, &WM_GAMMA), d.sat);
    let check = || -> Result<(), String> {
        let key = |k: &str| -> Result<(), String> {
            // Held 200 ms (under the repeat delay), answered before the
            // connection closes.
            crate::screen::Monitor::connect(monitor_port())?.query(&format!("sendkey {k} 200"))?;
            std::thread::sleep(Duration::from_millis(150));
            Ok(())
        };
        let up = |i: usize| Dials::STOCK.moved(i, 1);
        until("original", Dials::STOCK)?;
        key("right")?;
        until("black-26", up(0))?;
        key("down")?;
        until("step-2-reset", Dials::STOCK)?;
        key("right")?;
        until("white-26", up(1))?;
        for _ in 0..4 {
            key("down")?;
        }
        until("step-6-reset", Dials::STOCK)?;
        key("right")?;
        until("sat-26", up(5))?;
        key("down")?;
        until("look-start", PRESETS[PRESET_START].1)?;
        key("right")?;
        until("look-next", PRESETS[PRESET_START + 1].1)?;
        // The first measured look: its own table, the picture as drawn.
        key("right")?;
        until_table(
            "look-measured",
            &look_table(PRESETS.len(), &WM_GAMMA),
            Dials::STOCK.sat,
        )?;
        key("ret")?;
        until("compare-original", Dials::STOCK)?;
        key("f4")
    };
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&frame, "RPL OK")),
            during: Some(&check),
            then: Some("RPL END"),
            timeout: Duration::from_secs(600),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("display-cal: no run log after the image returned")?;
    for want in [
        "gamma: off, switched on",
        "step 1 Black: 26",
        "step 2 White: 26",
        "step 6 Sat: 26",
        "done (Home) after 12 changes; picks: Black 26 White 26 Mid 25 Warm 25 Bright 25 Sat 26",
        "last look: Measured full",
        "DISPLAY-CAL-OK",
    ] {
        if !log.contains(want) {
            return Err(format!("display-cal: no {want:?} in the run log:{log}"));
        }
    }
    eprintln!(
        "    display-cal OK: the guided steps pixel-exact (Black 26, the reset on the next step, White 26, Sat 26 redrawn, two looks and a measured one, the original); Home put back and returned"
    );
    Ok(())
}

/// The C6 looks test under the loader (`rp-looks`), twice: with no card
/// (the built-in photos) and with a card whose `rp-looks` folder holds two
/// photos stored out of name order and a file of the wrong size (ignored).
/// Each time every page pixel-exact below the caption, the strength dial
/// applied in the picture and dithered, driven by keys: 2.00, 2.25 on both
/// photos and the chart, 1.00, the original by compare, the ramps; Home
/// puts everything back.
pub fn boot_loader_looks(root: &Path) -> Result<(), String> {
    looks_run(root, None)?;
    let card = looks_card(&scratch(root, "looks-card.img")?)?;
    looks_run(root, Some(&card))
}

/// A card image with `rp-looks/b.yuv` (the Milky Way), `rp-looks/a.yuv`
/// (the pier), written in that order, and `rp-looks/short.yuv` (too small).
fn looks_card(path: &Path) -> Result<PathBuf, String> {
    use rp_hal_api::{BlockDevice, BlockError};
    use rp_looks::{PHOTOS, YUV_BYTES, unpack};
    struct Mem(Vec<u8>);
    impl BlockDevice for Mem {
        fn read_blocks(&mut self, lba: u32, buf: &mut [u8]) -> Result<(), BlockError> {
            let o = lba as usize * 512;
            let src = self.0.get(o..o + buf.len()).ok_or(BlockError::OutOfRange)?;
            buf.copy_from_slice(src);
            Ok(())
        }
        fn write_blocks(&mut self, lba: u32, buf: &[u8]) -> Result<(), BlockError> {
            let o = lba as usize * 512;
            let dst = self
                .0
                .get_mut(o..o + buf.len())
                .ok_or(BlockError::OutOfRange)?;
            dst.copy_from_slice(buf);
            Ok(())
        }
    }
    let mut photos = [vec![0u8; YUV_BYTES], vec![0u8; YUV_BYTES]];
    for (p, out) in PHOTOS.iter().zip(photos.iter_mut()) {
        unpack(p, out).map_err(|e| format!("looks: a photo did not unpack: {e:?}"))?;
    }
    let img = rp_fat::format::build(&rp_fat::format::Image {
        total_sectors: (SD_IMAGE_BYTES / 512) as u32,
        sectors_per_cluster: 8,
        files: &[],
        blocked_clusters: &[],
        ballast: 0,
    });
    let mut disk = Mem(img);
    {
        let mut vol = rp_fat::Volume::mount(&mut disk, rp_fat::format::PARTITION_START)
            .map_err(|e| format!("looks card: mount: {}", e.name()))?;
        let mut scratch = vec![0u8; 64 * 1024];
        vol.mkdir("rp-looks", &mut scratch)
            .map_err(|e| format!("looks card: mkdir: {}", e.name()))?;
        for (name, data) in [
            ("rp-looks/b.yuv", &photos[1][..]),
            ("rp-looks/a.yuv", &photos[0][..]),
            ("rp-looks/short.yuv", &photos[0][..1000]),
        ] {
            vol.put(name, data, &mut scratch)
                .map_err(|e| format!("looks card: {name}: {}", e.name()))?;
        }
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, &disk.0).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path.to_path_buf())
}

/// One run of the looks test, with `card` in the SD slot or none.
fn looks_run(root: &Path, card: Option<&Path>) -> Result<(), String> {
    use rp_drv_mbx_display::scanout::{GAMMA_ENTRIES, WM_GAMMA, gamma};
    use rp_looks::{
        CAPTION_H, H, PHOTOS, Page, START, W, YUV_BYTES, dial_table, page_pixel, shown, straight,
        unpack,
    };
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let loader = dir.join("rp-loader.bin");
    let image = dir.join("rp-looks.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-looks"), &image)?;
    let payload = std::fs::read(&image).map_err(|e| format!("{}: {e}", image.display()))?;
    let frame = rp_loadproto::frame(&payload);
    let shot = scratch(root, "looks-screen.ppm")?;
    let guest = match emulator(root)? {
        Emulator::Wsl => to_wsl_path(&shot),
        Emulator::Native(_) => shot.display().to_string(),
    };
    let mut photos = [vec![0u8; YUV_BYTES], vec![0u8; YUV_BYTES]];
    for (p, out) in PHOTOS.iter().zip(photos.iter_mut()) {
        unpack(p, out).map_err(|e| format!("looks: a photo did not unpack: {e:?}"))?;
    }
    // Page `page` below the caption: `t` applied in the picture and
    // dithered, then the straight hardware table.
    let st = straight();
    // No card in the emulator: the built-in photos, then the chart and
    // the ramps.
    let want = |page: usize, t: &[u32; GAMMA_ENTRIES]| -> Vec<u8> {
        let (kind, photo): (Page, &[u8]) = match page {
            0 | 1 => (Page::Photo, &photos[page]),
            2 => (Page::Chart, &photos[0]),
            _ => (Page::Ramps, &photos[0]),
        };
        let mut v = Vec::with_capacity((W * (H - CAPTION_H) * 3) as usize);
        for y in CAPTION_H..H {
            for x in 0..W {
                let (r, g, b) = shown(t, page_pixel(kind, photo, x, y), x, y);
                v.extend_from_slice(&[gamma(&st, r, 16), gamma(&st, g, 8), gamma(&st, b, 0)]);
            }
        }
        v
    };
    // `step` None: the original.
    let until = |name: &str, page: usize, step: Option<usize>| -> Result<(), String> {
        let t = step.map_or(WM_GAMMA, |s| dial_table(s, &WM_GAMMA));
        let want = want(page, &t);
        let deadline = Instant::now() + Duration::from_secs(90);
        loop {
            let _ = std::fs::remove_file(&shot);
            let mut mon = crate::screen::Monitor::connect(monitor_port())?;
            mon.screendump(&guest, &shot)?;
            let img = crate::screen::parse_ppm(&std::fs::read(&shot).map_err(|e| e.to_string())?)?;
            let mut differ = usize::MAX;
            if img.width >= W && img.height >= H {
                let mut got = Vec::with_capacity(want.len());
                for y in CAPTION_H..H {
                    let o = ((y * img.width) * 3) as usize;
                    got.extend_from_slice(&img.rgb[o..o + (W * 3) as usize]);
                }
                differ = got
                    .chunks(3)
                    .zip(want.chunks(3))
                    .filter(|(a, b)| a != b)
                    .count();
                if differ == 0 {
                    return Ok(());
                }
            }
            if Instant::now() > deadline {
                let _ = std::fs::copy(&shot, scratch(root, &format!("looks-{name}.ppm"))?);
                return Err(format!(
                    "looks: the {name} screen differs from the reference in {differ} pixels (target/emulator/looks-{name}.ppm)"
                ));
            }
            std::thread::sleep(Duration::from_millis(400));
        }
    };
    let check = || -> Result<(), String> {
        let key = |k: &str| -> Result<(), String> {
            crate::screen::Monitor::connect(monitor_port())?.query(&format!("sendkey {k} 200"))?;
            std::thread::sleep(Duration::from_millis(150));
            Ok(())
        };
        until("strength-2-pier", 0, Some(START))?;
        key("right")?;
        until("strength-2.25-pier", 0, Some(START + 1))?;
        key("down")?;
        until("strength-2.25-milkyway", 1, Some(START + 1))?;
        key("down")?;
        until("strength-2.25-chart", 2, Some(START + 1))?;
        for _ in 0..5 {
            key("left")?;
        }
        until("strength-1-chart", 2, Some(0))?;
        key("ret")?;
        until("compare-original-chart", 2, None)?;
        key("down")?;
        until("original-ramps", 3, None)?;
        key("f4")
    };
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: card,
            steps: &[],
            extra: &[],
            input: Some((&frame, "RPL OK")),
            during: Some(&check),
            then: Some("RPL END"),
            timeout: Duration::from_secs(600),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("looks: no run log after the image returned")?;
    let source = if card.is_some() {
        "2 photos from the card"
    } else {
        "2 photos from the image"
    };
    if !log.contains(source) {
        return Err(format!("looks: no {source:?} in the run log:{log}"));
    }
    for want in [
        "page drawn in",
        "1.00 full (showing the original), Colour chart",
        "done (Home) after 10 changes",
        "last: 1.00, Ramps",
        "LOOKS-OK",
    ] {
        if !log.contains(want) {
            return Err(format!("looks: no {want:?} in the run log:{log}"));
        }
    }
    eprintln!(
        "    looks OK ({}): both photos in name order, the chart and the ramps pixel-exact, the strength dial applied in the picture and dithered (2.00, 2.25, 1.00, original); Home put back and returned",
        if card.is_some() {
            "photos from the card's rp-looks folder, the wrong-sized file ignored"
        } else {
            "no card: the built-in photos"
        }
    );
    Ok(())
}

/// The screen measurement image under the loader, driven by a measurement
/// program written into it (C6, `rp_screen_measure::program`) as `cargo
/// xtask measure --profile` does: grey 128 through the straight table shows
/// exactly 128, then through a halving table exactly 64, each step within
/// 300 ms of plan, everything put back. Then the same image with a damaged
/// program: refused (checksum), nothing shown, returned. The built-in
/// schedule is the same code path (its steps are host-tested).
pub fn boot_loader_screen_measure(root: &Path) -> Result<(), String> {
    use rp_screen_measure::program::{PROGRAM_BYTES, Step, StepLight, encode};
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let loader = dir.join("rp-loader.bin");
    let image = dir.join("rp-screen-measure.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-screen-measure"), &image)?;
    let plain = std::fs::read(&image).map_err(|e| format!("{}: {e}", image.display()))?;
    let room = crate::profile::find_room(&plain)?;
    let lead = 3_000;
    let steps = [
        Step {
            rgb: (128, 128, 128),
            table: 0,
            light: StepLight::Counts(432),
            ms: 4_000,
        },
        Step {
            rgb: (128, 128, 128),
            table: 1,
            light: StepLight::Found,
            ms: 4_000,
        },
        Step {
            rgb: (255, 255, 255),
            table: 0,
            light: StepLight::Counts(0),
            ms: 1_000,
        },
    ];
    let half: [u32; 17] = core::array::from_fn(|i| (((16 * i as u32).min(255)) / 2) * 0x01_0101);
    let mut payload = plain.clone();
    encode(
        &steps,
        &[half],
        lead,
        &mut payload[room..room + PROGRAM_BYTES],
    )
    .map_err(|e| format!("screen-measure program: {}", e.name()))?;
    let starts: Vec<u32> = (0..steps.len())
        .map(|i| lead + steps[..i].iter().map(|s| s.ms).sum::<u32>())
        .collect();
    let shot = scratch(root, "screen-measure.ppm")?;
    let guest = match emulator(root)? {
        Emulator::Wsl => to_wsl_path(&shot),
        Emulator::Native(_) => shot.display().to_string(),
    };
    // The schedule in the guest's time (it shows each patch for its time
    // by its own clock; a busy PC slows it).
    let screen_is = |grey: u8,
                     from_ms: u32,
                     to_ms: u32,
                     t0: &mut GuestClock<'_>|
     -> Result<(), String> {
        while t0.ms()? < u64::from(from_ms) + 800 {
            poll_pause(Duration::from_millis(50));
        }
        let mut seen = None;
        while t0.ms()? < u64::from(to_ms) {
            let _ = std::fs::remove_file(&shot);
            t0.mon.screendump(&guest, &shot)?;
            let img = crate::screen::parse_ppm(&std::fs::read(&shot).map_err(|e| e.to_string())?)?;
            let want = [grey; 3];
            if img
                .rgb
                .chunks(3)
                .take((img.width * 640) as usize)
                .all(|c| c == want)
            {
                return Ok(());
            }
            seen = Some(img.rgb.get(..3).map(<[u8]>::to_vec));
            poll_pause(Duration::from_millis(300));
        }
        Err(format!(
            "screen-measure: grey {grey} never showed on screen between {from_ms} and {to_ms} ms (first pixel {seen:?})"
        ))
    };
    let check = || -> Result<(), String> {
        let mut mon = crate::screen::Monitor::connect(monitor_port())?;
        let mut t0 = GuestClock::start(&mut mon, "screen-measure")?;
        screen_is(128, starts[0], starts[1], &mut t0)?;
        screen_is(64, starts[1], starts[2], &mut t0)
    };
    let frame = rp_loadproto::frame(&payload);
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&frame, "RPL OK")),
            during: Some(&check),
            then: Some("RPL END"),
            timeout: Duration::from_secs(120),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("screen-measure: no run log after the image returned")?;
    for (i, planned) in starts.iter().enumerate() {
        let head = format!("step {i} at ");
        let at = log
            .lines()
            .find_map(|l| l.trim().strip_prefix(&head))
            .and_then(|r| r.split(' ').next())
            .and_then(|n| n.parse::<u32>().ok())
            .ok_or_else(|| format!("screen-measure: no {head:?} in the run log:{log}"))?;
        if at.abs_diff(*planned) > 300 {
            return Err(format!(
                "screen-measure: step {i} started at {at} ms, planned {planned}"
            ));
        }
    }
    for want in ["3 steps, program", "all patches shown", "SCREEN-MEASURE-OK"] {
        if !log.contains(want) {
            return Err(format!("screen-measure: no {want:?} in the run log:{log}"));
        }
    }

    // Damaged: one byte of the steps changed after the checksum was made.
    let mut bad = payload;
    bad[room + 20 + 68 + 1] ^= 0x40;
    let frame = rp_loadproto::frame(&bad);
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&frame, "RPL OK")),
            during: None,
            then: Some("RPL END"),
            timeout: Duration::from_secs(60),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("screen-measure: no run log after the damaged program")?;
    for want in ["program refused: checksum", "SCREEN-MEASURE-OK"] {
        if !log.contains(want) {
            return Err(format!(
                "screen-measure (damaged program): no {want:?} in the run log:{log}"
            ));
        }
    }
    if log.contains("step 0 at") {
        return Err(format!(
            "screen-measure: a damaged program's step was shown:{log}"
        ));
    }
    eprintln!(
        "    screen-measure OK: a program's steps on schedule (each within 300 ms), grey 128 exact through the straight table and 64 through a halving one, a damaged program refused, put back and returned"
    );
    Ok(())
}

/// The refresh-rate test (display calibration C3) under the loader. The
/// loader has already moved the display PLL to 60 Hz (`0xFC5`, as on the
/// device), so the image must refuse to switch and only measure: the
/// emulated frame follows the PLL (rp-v0.29.0, docs/hw/emulator.md), and
/// the counted rate must come out within 1 Hz of 60.36 Hz, twice (at the
/// start, then after D-pad centre); Home puts everything back.
pub fn boot_loader_refresh(root: &Path) -> Result<(), String> {
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let loader = dir.join("rp-loader.bin");
    let image = dir.join("rp-refresh.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-refresh"), &image)?;
    let payload = std::fs::read(&image).map_err(|e| format!("{}: {e}", image.display()))?;
    let frame = rp_loadproto::frame(&payload);
    // The gate's P17 ladder variant walks by itself: no keys.
    let ladder = std::env::var_os("RP_REFRESH_LADDER").is_some();
    let check = || -> Result<(), String> {
        let key = |k: &str| -> Result<(), String> {
            crate::screen::Monitor::connect(monitor_port())?.query(&format!("sendkey {k} 200"))?;
            Ok(())
        };
        if ladder {
            return Ok(());
        }
        // The first measurement (2 s), then a switch press and its 2 s.
        guest_hold(
            &mut crate::screen::Monitor::connect(monitor_port())?,
            5000,
            "refresh",
        )?;
        key("ret")?;
        guest_hold(
            &mut crate::screen::Monitor::connect(monitor_port())?,
            5000,
            "refresh",
        )?;
        key("f4")
    };
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&frame, "RPL OK")),
            during: Some(&check),
            then: Some("RPL END"),
            timeout: Duration::from_secs(300),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("refresh: no run log after the image returned")?;
    if ladder {
        return refresh_ladder_log(log);
    }
    for want in ["no switch, measuring only", "done (Home)", "REFRESH-OK"] {
        if !log.contains(want) {
            return Err(format!("refresh: no {want:?} in the run log:{log}"));
        }
    }
    let rates: Vec<u32> = log
        .lines()
        .filter_map(|l| l.split("clock: ").nth(1))
        .filter_map(|r| r.split(' ').next()?.parse().ok())
        .collect();
    // The fork models the line time from DISPPLL (rp-v0.29.0): the
    // loader's Display::adopt has already moved the panel to 0xFC5, so the
    // image finds it, refuses to switch, and counts ~60.36 Hz, as the X51v
    // measured at 0xFC5 (60.0-60.5 Hz, display calibration C3). The
    // switching path itself (from exactly 0xEC5) isn't reachable here: the
    // loader always switches first, on the device too.
    if rates.len() != 2 || rates.iter().any(|r| r.abs_diff(6036) > 100) {
        return Err(format!(
            "refresh: want two rates within 1 Hz of 60.36 Hz (0xFC5), got {rates:?} (centi-Hz):{log}"
        ));
    }
    eprintln!(
        "    refresh OK: no PLL switch without Windows Mobile's value; frames counted at {:?} centi-Hz (60 Hz, as the loader leaves it); Home put back and returned",
        rates
    );
    Ok(())
}

/// Judges `rp-refresh`'s P17 ladder log: every step of `rp_refresh::LADDER`
/// measured three times within 1 Hz of the rate its pixel clock gives (the
/// fork models the line time from `DISPPLL`, rp-v0.29.0), no fault bits,
/// and the PLL put back as found (the loader's 60 Hz, `0xFC5`).
fn refresh_ladder_log(log: &str) -> Result<(), String> {
    let steps: Vec<&str> = log
        .lines()
        .filter(|l| l.contains("ladder ") && l.contains("DISPPLL"))
        .collect();
    if steps.len() != 4 {
        return Err(format!(
            "refresh ladder: want 4 steps, got {}:{log}",
            steps.len()
        ));
    }
    for l in &steps {
        let expect: u32 = l
            .split("(expect ")
            .nth(1)
            .and_then(|r| r.split(' ').next()?.parse().ok())
            .ok_or_else(|| format!("refresh ladder: no expected rate in {l:?}"))?;
        let rates: Vec<u32> = l
            .split("us: ")
            .nth(1)
            .map(|r| {
                r.split(' ')
                    .take(3)
                    .filter_map(|n| n.parse().ok())
                    .collect()
            })
            .unwrap_or_default();
        if rates.len() != 3 || rates.iter().any(|r| r.abs_diff(expect) > 100) {
            return Err(format!(
                "refresh ladder: want 3 rates within 1 Hz of {expect}, got {rates:?} in {l:?}"
            ));
        }
        if !l.contains("locked true") || !l.ends_with("faults 00000000") {
            return Err(format!("refresh ladder: unlocked or faults in {l:?}"));
        }
    }
    for want in ["done (ladder); DISPPLL back to FC5", "REFRESH-OK"] {
        if !log.contains(want) {
            return Err(format!("refresh ladder: no {want:?} in the run log:{log}"));
        }
    }
    eprintln!(
        "    refresh ladder OK: 4 steps, each within 1 Hz of its pixel clock's rate, PLL put back"
    );
    Ok(())
}

/// The crash path: the loader starts an image that panics on purpose; the
/// panic must come back to the loader (no watchdog reset), reach the PC as
/// a crash report, and be kept on the card as `crashlog.txt`.
pub fn boot_loader_crash(root: &Path) -> Result<(), String> {
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    let loader = dir.join("rp-loader.bin");
    let crash = dir.join("rp-crash.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-crash"), &crash)?;
    let payload = std::fs::read(&crash).map_err(|e| format!("{}: {e}", crash.display()))?;
    let frame = rp_loadproto::frame(&payload);
    let image = rp_fat::format::build(&rp_fat::format::Image {
        total_sectors: 32 * 1024,
        sectors_per_cluster: 8,
        files: &[],
        blocked_clusters: &[],
        ballast: 0,
    });
    let sd = scratch(root, "sd-crash.img")?;
    std::fs::write(&sd, &image).map_err(|e| format!("{}: {e}", sd.display()))?;
    // First the image's own crash box, on the plane actually on screen:
    // rp-crash moved it to offset 0 (where Windows Mobile leaves it), so a
    // box drawn at the conventional 0x60000 never shows. Then, once the
    // loader is back, its crash screen: a red band at the top of its log
    // area (below the 96-pixel status lines) and the log's end.
    let shot = scratch(root, "crash-screen.ppm")?;
    let guest = match emulator(root)? {
        Emulator::Wsl => to_wsl_path(&shot),
        Emulator::Native(_) => shot.display().to_string(),
    };
    let screen = || -> Result<crate::screen::Image, String> {
        let _ = std::fs::remove_file(&shot);
        let mut mon = crate::screen::Monitor::connect(monitor_port())?;
        mon.screendump(&guest, &shot)?;
        let img = crate::screen::parse_ppm(&std::fs::read(&shot).map_err(|e| e.to_string())?)?;
        if img.width < 480 || img.height < 640 {
            return Err(format!("crash screen: {} x {}", img.width, img.height));
        }
        Ok(img)
    };
    let px = |img: &crate::screen::Image, x: u32, y: u32| {
        let o = ((y * img.width + x) * 3) as usize;
        (img.rgb[o], img.rgb[o + 1], img.rgb[o + 2])
    };
    // White as the screendump quantises RGB565 (about 248, 252, 248).
    let white = |(r, g, b): (u8, u8, u8)| r > 230 && g > 230 && b > 230;
    // Polls (the image holds its box for 3 s) until `seen` or `limit`.
    let poll = |limit: Duration,
                seen: &dyn Fn(&crate::screen::Image) -> Result<(), String>|
     -> Result<(), String> {
        let until = Instant::now() + limit;
        loop {
            let last = screen().and_then(|img| seen(&img));
            if last.is_ok() || Instant::now() > until {
                return last;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    };
    let image_box = |img: &crate::screen::Image| -> Result<(), String> {
        // The box: the bottom 64 lines, dark red (0xA800) with white text.
        let (r, g, b) = px(img, 470, 600);
        let text = (576..640).any(|y| (0..480).any(|x| white(px(img, x, y))));
        if (150..190).contains(&r) && g < 40 && b < 40 && text {
            Ok(())
        } else {
            Err(format!(
                "crash screen: the image's crash box isn't on the plane on screen (pixel {r},{g},{b}, text {text})"
            ))
        }
    };
    let loader_screen = |img: &crate::screen::Image| -> Result<(), String> {
        let (r, g, b) = px(img, 470, 96 + 4);
        if r < 200 || g > 40 || b > 40 {
            return Err(format!("crash screen: no red band (pixel {r},{g},{b})"));
        }
        // Text in the band (white) and below it (the log, light grey).
        let band = (0..480).any(|x| white(px(img, x, 96 + 10)));
        let text = (96 + 50..96 + 320)
            .any(|y| (0..480).any(|x| px(img, x, y).0 > 200 && px(img, x, y).1 > 200));
        if !band || !text {
            return Err(format!("crash screen: band text {band}, log text {text}"));
        }
        Ok(())
    };
    let check_screen = || -> Result<(), String> {
        poll(Duration::from_secs(5), &image_box)?;
        poll(Duration::from_secs(8), &loader_screen)?;
        // The run log goes out with the crash screen: let it arrive.
        Ok(())
    };
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: Some(&sd),
            steps: &[],
            extra: &[],
            // The image's own panic line, printed just before it draws its
            // box: the checks start while the box is up.
            input: Some((&frame, "PANIC deliberate test panic")),
            during: Some(&check_screen),
            then: Some("RPL END"),
            timeout: BOOT_TIMEOUT,
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("crash: no run log after the image crashed")?;
    if !log.contains("PANIC deliberate test panic") || !log.contains("[the image crashed") {
        return Err(format!("crash: the run log doesn't report the crash:{log}"));
    }
    if log.contains("did not return") {
        return Err("crash: went through a watchdog reset instead of back to the loader".into());
    }
    let saved = card_file(&sd, "crashlog.txt")?;
    if !String::from_utf8_lossy(&saved).contains("PANIC deliberate test panic") {
        return Err("crash: crashlog.txt on the card doesn't hold the report".into());
    }
    // The chain's end can't rest on what the starter left in r11 (the
    // emulator's loader happens to leave 0; the X51v's may not): `_start`
    // must clear it right before `rp_main`, read from the image.
    if !crate::symbols::start_clears_fp(&release.join("rp-crash"))? {
        return Err(
            "crash: rp-crash's _start doesn't clear r11 right before rp_main (frame-pointer build)"
                .into(),
        );
    }
    check_chain(
        root,
        &release.join("rp-crash"),
        log,
        "the panic",
        None,
        (MAIN, false),
    )?;
    if !String::from_utf8_lossy(&saved).contains("CHAIN ") {
        return Err("crash: crashlog.txt on the card doesn't hold the backtrace".into());
    }
    eprintln!(
        "    crash path OK: the image's crash box showed on the plane on screen (offset 0, as WM          leaves it); the panic came back to the loader, reached the PC and the card, and is on          the screen"
    );
    crash_kinds(root, &loader)
}

/// `rp-crash`'s `main`, as `cargo xtask symbols` names it.
const MAIN: &str = "rp_crash::image::main";
/// The function that calls `main` (`rp_image::entry!`'s).
const INTO_MAIN: &str = "rp_crash::image::__rp_image_main";

/// A frame-pointer image's backtrace in its run log (ADR-0001,
/// hal/pxa27x/src/backtrace.rs): one `CHAIN` line that reaches where the
/// image began (`start`), its last call `_start`'s (the chain unbroken to
/// the bottom), and the function `want.0` in it: its first call if
/// `want.1`, else any call or where it faulted (`at`). The addresses are
/// read through `cargo xtask symbols` on `elf`, the image as built.
fn check_chain(
    root: &Path,
    elf: &Path,
    log: &str,
    what: &str,
    at: Option<u32>,
    want: (&str, bool),
) -> Result<(), String> {
    let chains: Vec<&str> = log
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("CHAIN "))
        .collect();
    let [chain] = chains[..] else {
        return Err(format!(
            "crash {what}: want one CHAIN line in the run log, found {}:{log}",
            chains.len()
        ));
    };
    let calls: Vec<String> = chain
        .split_whitespace()
        .skip(1)
        .take_while(|w| w.starts_with("0x"))
        .map(str::to_string)
        .collect();
    let end = chain
        .split_whitespace()
        .skip(1 + calls.len())
        .collect::<Vec<_>>()
        .join(" ");
    if end != "start" || calls.is_empty() {
        return Err(format!(
            "crash {what}: the chain doesn't reach the start: `{chain}`"
        ));
    }
    let elf_arg = elf.display().to_string();
    let named = crate::symbols::symbols(root, &elf_arg, &calls)?;
    // A `symbols` line's function, without its offset.
    let function = |l: &str| -> String {
        let f = l.split_whitespace().nth(1).unwrap_or("");
        f.split_once('+').map_or(f, |(name, _)| name).to_string()
    };
    if named.last().map(|l| function(l)).as_deref() != Some("_start") {
        return Err(format!(
            "crash {what}: the chain's last call isn't _start's:\n{}",
            named.join("\n")
        ));
    }
    let (name, first) = want;
    let mut seen = named.clone();
    if first {
        seen.truncate(1);
    } else if let Some(at) = at {
        seen.extend(crate::symbols::symbols(
            root,
            &elf_arg,
            &[format!("{at:#x}")],
        )?);
    }
    if !seen.iter().any(|l| function(l) == name) {
        return Err(format!(
            "crash {what}: want {name} {}:\n{}",
            if first {
                "as the chain's first call"
            } else {
                "in the chain or where it faulted"
            },
            named.join("\n")
        ));
    }
    eprintln!(
        "    backtrace OK ({what}): {} calls back to _start, {name} {}",
        calls.len(),
        if first { "first" } else { "among them" }
    );
    for l in &named {
        eprintln!("        {l}");
    }
    Ok(())
}

/// Each fault kind through rp-image's crash path (quality audit C38):
/// `rp-crash` built to fault by a data abort, a prefetch abort, an
/// undefined instruction, a system call with no handler and the run limit.
/// Each must come back to the loader marked as a crash (never "did not
/// return", the watchdog's report of a hang, nor a clean return), with its
/// fault line in the run log and on the card, the SPSR of the mode it came
/// from (C44, C61) and its backtrace back to `_start` (ADR-0001).
fn crash_kinds(root: &Path, loader: &Path) -> Result<(), String> {
    for (switch, line, want) in [
        ("RP_CRASH_DATA", "FAULT DataAbort at ", (MAIN, false)),
        // `main` jumped to an unmapped address: the fault's address says
        // nothing, the chain's first call is the call into `main`.
        (
            "RP_CRASH_PREFETCH",
            "FAULT PrefetchAbort at ",
            (INTO_MAIN, true),
        ),
        ("RP_CRASH_UNDEFINED", "FAULT Undefined at ", (MAIN, false)),
        ("RP_CRASH_SWI", "FAULT Swi at ", (MAIN, false)),
        // Interrupted in `spin`: its caller first.
        ("RP_CRASH_RUN_LIMIT", "FAULT RunLimit at ", (MAIN, true)),
    ] {
        let image = variant_image(root, "rp-crash", (switch, "1"))?;
        let payload = std::fs::read(&image).map_err(|e| format!("{}: {e}", image.display()))?;
        let frame = rp_loadproto::frame(&payload);
        let card = rp_fat::format::build(&rp_fat::format::Image {
            total_sectors: 32 * 1024,
            sectors_per_cluster: 8,
            files: &[],
            blocked_clusters: &[],
            ballast: 0,
        });
        let sd = scratch(
            root,
            &format!("sd-crash-{}.img", switch.to_ascii_lowercase()),
        )?;
        std::fs::write(&sd, &card).map_err(|e| format!("{}: {e}", sd.display()))?;
        let output = boot(
            root,
            &Boot {
                image: loader,
                marker: LOADER_MARKER,
                screenshot: None,
                sd_image: Some(&sd),
                steps: &[],
                extra: &[],
                // The fault line, on the serial port first.
                input: Some((&frame, line)),
                during: None,
                then: Some("RPL END"),
                timeout: BOOT_TIMEOUT,
            },
        )?;
        let log = output
            .rsplit_once("RPL LOG")
            .map(|(_, l)| l)
            .ok_or_else(|| format!("crash {switch}: no run log after the image faulted"))?;
        if !log.contains(line) || !log.contains("[the image crashed") {
            return Err(format!(
                "crash {switch}: the run log doesn't report the fault as a crash:{log}"
            ));
        }
        if log.contains("did not return") {
            return Err(format!(
                "crash {switch}: went through a watchdog reset instead of back to the loader"
            ));
        }
        // The SPSR names the mode the fault came from (quality audit C44:
        // the abort entries reported 0, on the X51v): an image runs in SVC
        // or SYS mode.
        let spsr = log
            .split(line)
            .nth(1)
            .and_then(|rest| rest.split(" spsr 0x").nth(1))
            .and_then(|v| u32::from_str_radix(v.get(..8)?, 16).ok())
            .ok_or_else(|| format!("crash {switch}: no SPSR in the fault line:{log}"))?;
        if !matches!(spsr & 0x1F, 0x13 | 0x1F) {
            return Err(format!(
                "crash {switch}: the fault line's SPSR 0x{spsr:08x} names no mode an image runs in"
            ));
        }
        let saved = card_file(&sd, "crashlog.txt")?;
        if !String::from_utf8_lossy(&saved).contains(line) {
            return Err(format!(
                "crash {switch}: crashlog.txt on the card doesn't hold the fault"
            ));
        }
        let at = log
            .split(line)
            .nth(1)
            .and_then(|rest| u32::from_str_radix(rest.strip_prefix("0x")?.get(..8)?, 16).ok());
        check_chain(root, &image.with_extension("elf"), log, switch, at, want)?;
        eprintln!(
            "    crash path OK: `{}` came back to the loader as a crash",
            line.trim_end()
        );
    }
    crash_board(root, loader)
}

/// The kinds of board state the loader keeps that the emulator models
/// (all six: the CPLD, PWM0 and the 2700G's registers hold what's
/// written). Quality audit C39.
const BOARD_KINDS: [&str; 6] = [
    "amplifier",
    "codec/USB bits",
    "backlight",
    "gamma",
    "MBXCLK",
    "M24CLK",
];

/// Quality audit C39 (option B): `rp-crash` built with `RP_CRASH_BOARD`
/// changes every kind of board state the loader keeps, then panics. The
/// loader, recording the board at its first start, must put each kind back
/// on the return and read it back as recorded.
fn crash_board(root: &Path, loader: &Path) -> Result<(), String> {
    let image = variant_image(root, "rp-crash", ("RP_CRASH_BOARD", "1"))?;
    let payload = std::fs::read(&image).map_err(|e| format!("{}: {e}", image.display()))?;
    let frame = rp_loadproto::frame(&payload);
    let output = boot(
        root,
        &Boot {
            image: loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&frame, "RETROPANE-CRASH-TEST")),
            during: None,
            then: Some("RPL END"),
            timeout: BOOT_TIMEOUT,
        },
    )?;
    if !output.contains("board: recorded as Windows Mobile left it") {
        return Err(format!(
            "crash board: the loader's first start didn't record the board:{output}"
        ));
    }
    let line = output
        .lines()
        .find(|l| l.contains("board: put back"))
        .ok_or_else(|| format!("crash board: nothing put back on the return:{output}"))?;
    for kind in BOARD_KINDS {
        if !line.contains(&format!(" {kind};")) {
            return Err(format!("crash board: `{kind}` not put back: {line}"));
        }
    }
    if !line.ends_with("read back: as recorded") {
        return Err(format!("crash board: not as recorded after: {line}"));
    }
    eprintln!("    crash board OK: {line}");
    Ok(())
}

/// Sends the loader an image that hangs with interrupts off (one `b .`
/// instruction, MMU and IRQs off), which the soft run limit can't reach,
/// and checks the backstop: the hardware watchdog resets the machine, the
/// loader starts again and reports that the run didn't return. The emulator
/// restarts the loader itself; on the device Windows Mobile's StartUp
/// folder does (docs: SD kit README, loader/). A 1 s limit keeps the
/// backstop at 31 s. The image starts the cycle counter first (`PMNC` bit 0,
/// as rp-noise leaves it): emulator releases before rp-v0.14.0 aborted on
/// the reset with the counter running instead of rebooting as the device does.
pub fn boot_loader_hang(root: &Path) -> Result<(), String> {
    let loader = raw_image(root, "rp-loader")?;
    let mut hang = rp_loadproto::control_frame("limit 1");
    // mov r0, #1; mcr p14, 0, r0, c0, c1, 0 (PMNC: count); b .
    let words = [0xE3A0_0001u32, 0xEE00_0E11, 0xEAFF_FFFE];
    let code: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    hang.extend(rp_loadproto::frame(&code));
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&hang, "did not return")),
            during: None,
            then: None,
            timeout: Duration::from_secs(120),
        },
    )?;
    if !output.contains("RPL OK") {
        return Err("watchdog: the hanging image was never started".into());
    }
    eprintln!("    watchdog OK: a hung image reset the device and the loader reported it");
    Ok(())
}

/// The card's loader after an image returns, with no PC program on its
/// port: it keeps the watchdog (armed by the run) fed however long it waits.
/// Its link patience (90 s, then a reset) is for a loader started as an
/// image only; on 2026-10-17 it fired after an rp-today run whose USB never
/// came back, and the Axim landed on Dell's factory prompt. The emulated PC
/// enumerates the loader but no program opens its port (the USB chardev has
/// no client), so the link is never proven, as on that day. A loader that
/// stopped feeding would reset within 90 + 30 s of the return; this waits
/// 150 s and wants no start after the one the return makes.
pub fn boot_loader_keeps_feeding(root: &Path) -> Result<(), String> {
    if !has_usb_model(root) {
        eprintln!("    keep-fed: skipped (this emulator has no USB controller model)");
        return Ok(());
    }
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let loader = dir.join("rp-loader.bin");
    let light = dir.join("rp-light.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-light"), &light)?;
    let payload = std::fs::read(&light).map_err(|e| format!("{}: {e}", light.display()))?;
    let input = rp_loadproto::frame(&payload);
    let args = usb_args(4);
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let wait = || -> Result<(), String> {
        guest_hold(
            &mut crate::screen::Monitor::connect(monitor_port())?,
            150000,
            "keeps-feeding",
        )?;
        Ok(())
    };
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &extra,
            input: Some((&input, "RPL END")),
            during: Some(&wait),
            then: None,
            timeout: Duration::from_secs(400),
        },
    )?;
    // Kept for reading: what the loader printed while it waited.
    let _ = std::fs::write(dir.join("keepfed.txt"), &output);
    // Ready once at the first boot, once after the image returned; a third
    // is a reset.
    let starts = output.matches("BENCH boot_loader_ready").count();
    if starts != 2 {
        return Err(format!(
            "keep-fed: the loader started {starts} times (2 wanted: a watchdog reset after the image returned?):{output}"
        ));
    }
    eprintln!(
        "    keep-fed OK: after the image returned, the loader kept the watchdog fed for 150 s with no PC program on its port"
    );
    Ok(())
}

/// The soft run limit (overnight item 55): an image that runs forever with
/// interrupts on and never checks in must come back to the loader with a
/// crash report saying so, with no reset (on the X51v a reset lands on
/// Dell's factory-restore prompt). A 2 s limit keeps the test short.
pub fn boot_loader_run_limit(root: &Path) -> Result<(), String> {
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    let loader = dir.join("rp-loader.bin");
    let hang = dir.join("rp-hang.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-hang"), &hang)?;
    let payload = std::fs::read(&hang).map_err(|e| format!("{}: {e}", hang.display()))?;
    let mut input = rp_loadproto::control_frame("limit 2");
    input.extend(rp_loadproto::frame(&payload));
    let started = Instant::now();
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&input, "RPL END")),
            during: None,
            then: None,
            timeout: Duration::from_secs(60),
        },
    )?;
    let took = started.elapsed();
    if !output.contains("RPL OK limit 2 s") {
        return Err("run limit: the loader didn't take `limit 2`".into());
    }
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("run limit: no run log after the image came back")?;
    if !log.contains("RETROPANE-HANG-TEST") || !log.contains("FAULT RunLimit") {
        return Err(format!("run limit: the run log doesn't report it:{log}"));
    }
    // The interrupted code's mode in the report (quality audit C61: the
    // FIQ entry kept none, every run-limit report said spsr 0): rp-hang
    // spins in SVC mode.
    let spsr = log
        .split("FAULT RunLimit at ")
        .nth(1)
        .and_then(|rest| rest.split(" spsr 0x").nth(1))
        .and_then(|v| u32::from_str_radix(v.get(..8)?, 16).ok())
        .ok_or_else(|| format!("run limit: no SPSR in the fault line:{log}"))?;
    if spsr & 0x1F != 0x13 {
        return Err(format!(
            "run limit: the fault line's SPSR 0x{spsr:08x} isn't SVC mode, where rp-hang spins"
        ));
    }
    // rp-hang is built may-omit (ADR-0001): no walker, and its report says so.
    if !log.contains(rp_hal_pxa27x::backtrace::NO_CHAIN) || log.contains("CHAIN ") {
        return Err(format!(
            "run limit: a may-omit image's report must say `{}` and walk nothing:{log}",
            rp_hal_pxa27x::backtrace::NO_CHAIN
        ));
    }
    if log.contains("did not return") {
        return Err("run limit: went through a reset instead of back to the loader".into());
    }
    eprintln!(
        "    run limit OK: a program that never checked in came back to the loader after its 2 s limit, with a report and no reset ({} s in all)",
        took.as_secs()
    );
    Ok(())
}

/// The concept-picture image (rp-picture): the loader starts it, the screen
/// must show the picture exactly (the screendump against the source, after
/// RGB565), and after its 20 s it must be back at the loader through a
/// normal return: no run-limit crash, no reset. Only then may it go to the
/// device.
pub fn boot_loader_picture(root: &Path) -> Result<(), String> {
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    let loader = dir.join("rp-loader.bin");
    let bin = dir.join("rp-picture.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-picture"), &bin)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let want = expected_picture(root)?;
    let shot = scratch(root, "picture-screen.ppm")?;
    let guest = match emulator(root)? {
        Emulator::Wsl => to_wsl_path(&shot),
        Emulator::Native(_) => shot.display().to_string(),
    };
    let check = || -> Result<(), String> {
        // Called as soon as the image says the picture is up.
        let shown = Instant::now();
        // The emulator redraws its screen on a timer: give it time to show
        // the new frame (as the crash-screen check does).
        guest_hold(
            &mut crate::screen::Monitor::connect(monitor_port())?,
            1500,
            "picture",
        )?;
        let _ = std::fs::remove_file(&shot);
        let mut mon = crate::screen::Monitor::connect(monitor_port())?;
        mon.screendump(&guest, &shot)?;
        let img = crate::screen::parse_ppm(&std::fs::read(&shot).map_err(|e| e.to_string())?)?;
        if (img.width, img.height) != (480, 640) {
            return Err(format!("picture: screen is {}x{}", img.width, img.height));
        }
        // The screendump widens RGB565 to 8 bits; compare in RGB565 terms.
        let mut off = 0usize;
        for (i, px) in want.as_chunks::<2>().0.iter().enumerate() {
            let v = u16::from_le_bytes(*px);
            let (r, g, b) = (img.rgb[i * 3], img.rgb[i * 3 + 1], img.rgb[i * 3 + 2]);
            if u16::from(r >> 3) != v >> 11
                || u16::from(g >> 2) != (v >> 5) & 63
                || u16::from(b >> 3) != v & 31
            {
                off += 1;
            }
        }
        if off != 0 {
            return Err(format!(
                "picture: {off} of 307200 pixels differ from the source"
            ));
        }
        // Its return is awaited after the check (the loader's RPL END).
        let _ = shown;
        Ok(())
    };
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&rp_loadproto::frame(&payload), "RETROPANE-PICTURE-SHOWN")),
            during: Some(&check),
            then: Some("RPL END"),
            timeout: Duration::from_secs(90),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or_else(|| {
            let tail: String = output
                .chars()
                .rev()
                .take(600)
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            format!(
                "picture: never came back to the loader; serial ends:
{tail}"
            )
        })?;
    if !log.contains("RETROPANE-PICTURE-SHOWN") {
        return Err(format!(
            "picture: the run log doesn't have the marker:{log}"
        ));
    }
    if crashed(log) || log.contains("did not return") {
        return Err(format!("picture: didn't return cleanly:{log}"));
    }
    eprintln!(
        "    picture OK: shown pixel-exact, back at the loader after {} s with no crash or reset",
        rp_picture_seconds()
    );
    Ok(())
}

/// The plane-blending image (rp-blend, overnight item 60): the loader
/// starts it; each of its stages must show exactly what the
/// reference model (`rp_drv_mbx_display::scanout`) computes from the same
/// drawing and register programming, which a host test already checks
/// against the intended colours. Each screendump is matched against all
/// the stages, so a slow host can't pass one stage off as another. Then
/// it must be back at the loader with the wallpaper gone.
/// `rp-blend`'s P9 scaler steps (`RP_BLEND_SCALE`, the gate's P9/V5): the
/// emulator doesn't model the video plane's scaler (docs/hw/emulator.md),
/// so this checks what it can: every step comes up, the scaler registers
/// read back as `layers::VideoPlane::registers_scaled` wrote them for that
/// step, the display put back and the image returned. Whether the picture
/// is right is the owner's eye on the X51v.
pub fn boot_loader_blend_scale(root: &Path) -> Result<(), String> {
    use rp_drv_mbx_display::layers::{Layer, Scale, VideoFormat, VideoPlane, reg};
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    let loader = dir.join("rp-loader.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    let bin = variant_image_with(
        root,
        "rp-blend",
        ("RP_BLEND_SCALE", "1"),
        Some(("RP_BLEND_SECONDS", "2")),
    )?;
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
            input: Some((&rp_loadproto::frame(&payload), "RPL OK")),
            during: None,
            then: Some("RPL END"),
            timeout: Duration::from_secs(300),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("blend P9: no run log after the image returned")?;
    if crashed(log) {
        return Err(format!("blend P9: crashed:{log}"));
    }
    let wall = VideoPlane::new(VideoFormat::Uyvy, 480, 640, 0x50_0000);
    let lines: Vec<&str> = log
        .lines()
        .filter(|l| l.starts_with("P9 scaler:"))
        .collect();
    if lines.len() != rp_blend_steps().len() {
        return Err(format!(
            "blend P9: want {} scaler lines, got {}:{log}",
            rp_blend_steps().len(),
            lines.len()
        ));
    }
    for (line, (src, out)) in lines.iter().zip(rp_blend_steps()) {
        let mut v = VideoPlane::new(VideoFormat::Uyvy, src.2, src.3, wall.offset);
        v.stride = wall.stride;
        let scale = Scale {
            out_w: out.0,
            out_h: out.1,
            x: out.2,
            y: out.3,
            taps: out.4,
            ur_c: out.5,
            filter: rp_drv_mbx_display::layers::ScalerFilter::Mbxfb,
        };
        let regs = v.registers_scaled(Layer::SOLO, false, scale);
        let want = |o: usize| regs.iter().find(|r| r.0 == o).map_or(0, |r| r.1);
        let expect = format!(
            "P9 scaler: VSADR {:08X} SSSIZE {:08X} SPOCTRL {:08X} SHCTRL {:08X}",
            want(reg::VSADR),
            want(reg::SSSIZE),
            want(reg::SPOCTRL),
            want(reg::SHCTRL)
        );
        if line.trim() != expect {
            return Err(format!("blend P9: read back {line:?}, wrote {expect:?}"));
        }
    }
    let steps = log.lines().filter(|l| l.starts_with("P9 step ")).count();
    if steps != rp_blend_steps().len() || !log.contains("restored:") {
        return Err(format!(
            "blend P9: {steps} steps held, or nothing put back:{log}"
        ));
    }
    eprintln!(
        "    blend P9 OK: {steps} scaler steps up, each read back as written, put back (the picture: the X51v, watched)"
    );
    Ok(())
}

/// `rp-blend`'s run P9d (`RP_BLEND_FILTER`, gate V5): every step comes
/// up; the scaler's size, pitch and place registers read back as
/// `registers_scaled` wrote them, and on every scaling step the filter's
/// fifteen registers as `filter_words` computed them (the fork stores
/// them: docs/hw/emulator.md), so the fitted filter and the two proof sets
/// reach the chip as the host tests built them; `GDRCTRL`'s doubling set
/// on the last step only; the display put back. The pictures are the
/// owner's eye on the X51v.
pub fn boot_loader_blend_filter(root: &Path) -> Result<(), String> {
    use rp_drv_mbx_display::blendtest::{FILTER_STEPS, FILTER2_STEPS};
    blend_filter_run(root, "RP_BLEND_FILTER", &FILTER_STEPS, "P9d")?;
    blend_filter_run(root, "RP_BLEND_FILTER2", &FILTER2_STEPS, "P9e")
}

/// One of [`boot_loader_blend_filter`]'s runs: `flag`'s image, `steps`.
fn blend_filter_run(
    root: &Path,
    flag: &str,
    steps: &[rp_drv_mbx_display::blendtest::FilterStep],
    name: &str,
) -> Result<(), String> {
    use rp_drv_mbx_display::blendtest::{self, GDRCTRL_DOUBLE};
    use rp_drv_mbx_display::layers::{Layer, VideoFormat, VideoPlane, reg};
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    let loader = dir.join("rp-loader.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    let bin = variant_image_with(
        root,
        "rp-blend",
        (flag, "1"),
        Some(("RP_BLEND_SECONDS", "2")),
    )?;
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
            input: Some((&rp_loadproto::frame(&payload), "RPL OK")),
            during: None,
            then: Some("RPL END"),
            timeout: Duration::from_secs(300),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or_else(|| format!("blend {name}: no run log after the image returned"))?;
    if crashed(log) {
        return Err(format!("blend {name}: crashed:{log}"));
    }
    let scaler: Vec<&str> = log
        .lines()
        .filter(|l| l.starts_with("P9 scaler:"))
        .collect();
    let filter: Vec<&str> = log
        .lines()
        .filter(|l| l.starts_with("P9d filter:"))
        .collect();
    if scaler.len() != steps.len() || filter.len() != steps.len() {
        return Err(format!(
            "blend {name}: want {} scaler and filter lines, got {} and {}:{log}",
            steps.len(),
            scaler.len(),
            filter.len()
        ));
    }
    let mut filters_checked = 0;
    for (i, step) in steps.iter().enumerate() {
        let (_, _, w, h) = step.src;
        let mut v = VideoPlane::new(VideoFormat::Uyvy, w, h, blendtest::FILTER_AT);
        v.stride = blendtest::W * 2;
        let regs = v.registers_scaled(Layer::SOLO, false, step.scale);
        let want = |o: usize| regs.iter().find(|r| r.0 == o).map_or(0, |r| r.1);
        let expect = format!(
            "P9 scaler: VSADR {:08X} SSSIZE {:08X} SPOCTRL {:08X} SHCTRL {:08X}",
            want(reg::VSADR),
            want(reg::SSSIZE),
            want(reg::SPOCTRL),
            want(reg::SHCTRL)
        );
        if scaler[i].trim() != expect {
            return Err(format!(
                "blend {name} step {}: read back {:?}, wrote {expect:?}",
                i + 1,
                scaler[i]
            ));
        }
        let (words, gdr) = filter[i]
            .trim()
            .strip_prefix("P9d filter:")
            .and_then(|l| l.split_once("; GDRCTRL "))
            .ok_or_else(|| format!("blend {name} step {}: {:?}", i + 1, filter[i]))?;
        let gdr = u32::from_str_radix(gdr.trim(), 16).map_err(|e| format!("{e}: {gdr}"))?;
        if (gdr & GDRCTRL_DOUBLE == GDRCTRL_DOUBLE) != step.double {
            return Err(format!(
                "blend {name} step {}: GDRCTRL {gdr:08X}, doubling wanted {}",
                i + 1,
                step.double
            ));
        }
        if v.scales(step.scale) {
            let want: Vec<String> = v
                .filter_words(step.scale)
                .iter()
                .map(|(_, w)| format!("{w:08X}"))
                .collect();
            if words.split_whitespace().collect::<Vec<_>>() != want {
                return Err(format!(
                    "blend {name} step {}: filter read back {words:?}, wrote {want:?}",
                    i + 1
                ));
            }
            filters_checked += 1;
        }
    }
    let held = log.lines().filter(|l| l.starts_with("P9d step ")).count();
    if held != steps.len() || !log.contains("restored:") {
        return Err(format!(
            "blend {name}: {held} steps held, or nothing put back:{log}"
        ));
    }
    eprintln!(
        "    blend {name} OK: {held} steps up, the scaler and {filters_checked} filters read back as computed, the doubling only where set, put back (the pictures: the X51v, watched)"
    );
    Ok(())
}

/// A P9 step's source rectangle (x, y, w, h) and output (w, h, x, y, taps).
type BlendSrc = (u16, u16, u16, u16);
type BlendOut = (u16, u16, u16, u16, u8, bool);

/// `rp-blend`'s `SCALE_STEPS` as (source, output) tuples, kept in step with
/// the image's table by the check above (a mismatch shows as a read-back
/// difference).
fn rp_blend_steps() -> [(BlendSrc, BlendOut); 9] {
    [
        ((0, 0, 480, 640), (0, 0, 0, 0, 1, false)),
        ((0, 0, 240, 320), (480, 640, 0, 0, 4, false)),
        ((0, 0, 240, 320), (480, 640, 0, 0, 1, false)),
        ((0, 0, 480, 640), (240, 320, 120, 160, 4, false)),
        ((0, 0, 480, 640), (360, 480, 60, 80, 4, false)),
        ((0, 400, 240, 240), (0, 0, 240, 320, 1, false)),
        ((0, 0, 240, 320), (480, 640, 0, 0, 1, true)),
        ((0, 0, 480, 640), (240, 320, 120, 160, 4, true)),
        ((0, 0, 480, 640), (360, 480, 60, 80, 4, true)),
    ]
}

pub fn boot_loader_blend(root: &Path) -> Result<(), String> {
    use rp_drv_mbx_display::blendtest::{self, Stage};
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    let loader = dir.join("rp-loader.bin");
    let bin = dir.join("rp-blend.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-blend"), &bin)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let want: Vec<Vec<u8>> = Stage::ALL.iter().map(|&s| blend_screen(s)).collect();
    let wsl = matches!(emulator(root)?, Emulator::Wsl);
    // Kept per stage (target/emulator/blend-A.ppm ...) for a look by eye.
    let dump = |name: char| -> Result<Vec<u8>, String> {
        let shot = scratch(root, &format!("blend-{name}.ppm"))?;
        let guest = if wsl {
            to_wsl_path(&shot)
        } else {
            shot.display().to_string()
        };
        let _ = std::fs::remove_file(&shot);
        let mut mon = crate::screen::Monitor::connect(monitor_port())?;
        mon.screendump(&guest, &shot)?;
        let img = crate::screen::parse_ppm(&std::fs::read(&shot).map_err(|e| e.to_string())?)?;
        if (img.width as usize, img.height as usize) != (blendtest::W, blendtest::H) {
            return Err(format!("blend: screen is {}x{}", img.width, img.height));
        }
        Ok(img.rgb)
    };
    let stage_time = Duration::from_secs(u64::from(RP_BLEND_STAGE_SECONDS));
    let check = || -> Result<(), String> {
        // Called as soon as stage A is up. Each stage's screen must appear,
        // in order: poll the screen rather than trust timing. The image
        // holds a stage for its time in *emulated* seconds, which run
        // slower than the wall clock when the PC is busy (a full suite), so
        // a skipped stage is caught by the order (a later stage showing
        // first), and the deadline is only a generous safety net.
        let safety = stage_time * 4 + Duration::from_secs(20);
        for (i, stage) in Stage::ALL.iter().enumerate() {
            let deadline = Instant::now() + safety;
            loop {
                poll_pause(Duration::from_millis(700));
                let got = dump(stage.letter())?;
                let differ = |w: &[u8]| {
                    got.chunks(3)
                        .zip(w.chunks(3))
                        .filter(|(a, b)| a != b)
                        .count()
                };
                let counts: Vec<usize> = want.iter().map(|w| differ(w)).collect();
                if counts[i] == 0 {
                    break;
                }
                if counts[i + 1..].contains(&0) || Instant::now() > deadline {
                    return Err(format!(
                        "blend: stage {} never showed as the reference has it; pixels differing from each stage: {counts:?}",
                        stage.letter()
                    ));
                }
            }
        }
        // After the last stage, it puts everything back and returns; the
        // loader takes the screen again: no trace of the wallpaper (polled,
        // for the same reason).
        let bars = |s: &[u8]| s[(435 * blendtest::W + 90) * 3..][..3].to_vec();
        let deadline = Instant::now() + safety;
        loop {
            poll_pause(Duration::from_millis(700));
            let got = dump('R')?;
            if !want.iter().any(|w| bars(w) == bars(&got)) {
                return Ok(());
            }
            if Instant::now() > deadline {
                return Err("blend: the wallpaper is still on screen after the return".into());
            }
        }
    };
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&rp_loadproto::frame(&payload), "RETROPANE-BLEND-A")),
            during: Some(&check),
            then: Some("RPL END"),
            timeout: Duration::from_secs(600),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("blend: never came back to the loader")?;
    for want in [
        "RETROPANE-BLEND-A",
        "RETROPANE-BLEND-B",
        "RETROPANE-BLEND-C",
        "RETROPANE-BLEND-D",
        "RETROPANE-BLEND-E",
        "RETROPANE-BLEND-F",
        "restored:",
    ] {
        if !log.contains(want) {
            return Err(format!("blend: the run log has no `{want}`:{log}"));
        }
    }
    if crashed(log) || log.contains("did not return") {
        return Err(format!("blend: didn't return cleanly:{log}"));
    }
    eprintln!(
        "    blend OK: every stage pixel-exact against the reference, registers put back, back at the loader"
    );
    Ok(())
}

/// How long rp-blend holds each stage (kept in step with the image).
const RP_BLEND_STAGE_SECONDS: u32 = 15;

/// The GPU test image (rp-mbx, item 69): the loader starts it; its fills
/// go through the emulated MBX 2D engine (retropane-qemu rp-v0.12.0 on),
/// so its own read-back checks must pass, and the picture it leaves up
/// must have the GPU-drawn bars and frame with exact edges (ends
/// exclusive). Then Enter returns it to the loader. Skipped, with a note,
/// on an emulator without the model.
pub fn boot_loader_mbx(root: &Path) -> Result<(), String> {
    if !device_has(root, "intel-2700g", "mbx-2d") {
        eprintln!("    mbx: skipped (this emulator release doesn't model the MBX 2D engine)");
        return Ok(());
    }
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    let loader = dir.join("rp-loader.bin");
    let bin = dir.join("rp-mbx.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-mbx"), &bin)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let shot = scratch(root, "mbx-screen.ppm")?;
    let guest = match emulator(root)? {
        Emulator::Wsl => to_wsl_path(&shot),
        Emulator::Native(_) => shot.display().to_string(),
    };
    let check = || -> Result<(), String> {
        // Give the emulator's screen timer a frame to show the picture.
        guest_hold(
            &mut crate::screen::Monitor::connect(monitor_port())?,
            1500,
            "mbx",
        )?;
        let _ = std::fs::remove_file(&shot);
        let mut mon = crate::screen::Monitor::connect(monitor_port())?;
        mon.screendump(&guest, &shot)?;
        let img = crate::screen::parse_ppm(&std::fs::read(&shot).map_err(|e| e.to_string())?)?;
        if (img.width, img.height) != (480, 640) {
            return Err(format!("mbx: screen is {}x{}", img.width, img.height));
        }
        // Inside each part of the scene, and one pixel past each end.
        let (black, red, green, blue, white) = (
            [0, 0, 0],
            [255, 0, 0],
            [0, 255, 0],
            [0, 0, 255],
            [255, 255, 255],
        );
        let want: [((usize, usize), [u8; 3]); 12] = [
            ((60, 100), red),
            ((159, 539), red),
            ((160, 300), black),
            ((110, 540), black),
            ((240, 300), green),
            ((370, 300), blue),
            ((419, 300), blue),
            ((420, 300), black),
            ((22, 300), white),
            ((300, 22), white),
            ((459, 619), white),
            ((40, 60), black),
        ];
        let mut wrong = Vec::new();
        for ((x, y), rgb) in want {
            let i = (y * 480 + x) * 3;
            // The screendump widens RGB565 to 8 bits: compare the top bits.
            let got = [img.rgb[i], img.rgb[i + 1], img.rgb[i + 2]];
            if got.iter().zip(rgb).any(|(g, w)| g >> 3 != w >> 3) {
                wrong.push(format!("({x},{y}) {got:?} not {rgb:?}"));
            }
        }
        if !wrong.is_empty() {
            return Err(format!("mbx: the GPU-drawn picture is wrong at {wrong:?}"));
        }
        // A button press returns it to the loader.
        mon.query("sendkey ret 200")?;
        Ok(())
    };
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&rp_loadproto::frame(&payload), "RETROPANE-MBX-2D")),
            during: Some(&check),
            then: Some("RPL END"),
            timeout: Duration::from_secs(60),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or_else(|| {
            let tail: String = output
                .chars()
                .rev()
                .take(600)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            format!(
                "mbx: never came back to the loader after the button press; serial ends:
{tail}"
            )
        })?;
    if !log.contains("RETROPANE-MBX-2D: PASS") {
        return Err(format!("mbx: the image's own checks failed:{log}"));
    }
    if crashed(log) || log.contains("did not return") {
        return Err(format!("mbx: didn't return cleanly:{log}"));
    }
    for l in log.lines().filter(|l| {
        l.starts_with("dma:")
            || l.starts_with("probe:")
            || l.starts_with("down ")
            || l.starts_with("time ")
            || l.starts_with("completion:")
            || l.contains(" us: ")
            || l.starts_with("  after clearing")
            || l.starts_with("  every status")
            || l.starts_with("irq:")
            || l.contains(" in 40 ms: ")
            || l.starts_with("  [")
            || l.contains("full-screen")
    }) {
        eprintln!("      {}", l.trim_end());
    }
    eprintln!(
        "    mbx OK: fence, rectangle edges and the GPU-drawn picture exact; back at the loader on a button press"
    );
    Ok(())
}

/// rp-mbx-blend's run on the owner's X51v (2026-10-14): the lines the
/// emulator must reproduce.
const MBX_BLEND_DEVICE: &str = include_str!("../data/rp-mbx-blend-device.txt");

/// The 4-digit hex pixels after a run-log line's colon, before any verdict.
fn logged_pixels(rest: &str) -> Vec<&str> {
    rest.split_whitespace()
        .take_while(|t| t.len() == 4 && t.chars().all(|c| c.is_ascii_hexdigit()))
        .collect()
}

/// Where an emulator run of rp-mbx-blend differs from the device's
/// (`MBX_BLEND_DEVICE`): one entry per line. Pixels are compared exactly;
/// the event lines up to the delays (host-independent under `-icount`, but
/// the device's came from its own clears). Not compared, because nobody
/// knows the answer: format code 4's rows (meaning unknown) and the 32-bit
/// codes' pixels 2-3 from the 16-bit source (they read memory the image
/// never wrote: the device's leftovers).
fn mbx_blend_mismatches(log: &str, device: &str) -> Vec<String> {
    let mut bad = Vec::new();
    for want in device
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
    {
        let Some((key, value)) = want.split_once(": ") else {
            continue;
        };
        if key.starts_with("B3 code 4 ") {
            continue;
        }
        let got = log
            .lines()
            .map(str::trim)
            .find_map(|l| l.strip_prefix(key).and_then(|r| r.strip_prefix(": ")));
        let Some(got) = got else {
            bad.push(format!("no `{key}` line"));
            continue;
        };
        let same = if key.starts_with('B') {
            let (w, g) = (logged_pixels(value), logged_pixels(got));
            let n = if ["12", "13", "14", "15"]
                .iter()
                .any(|c| key.starts_with(&format!("B3 code {c} src16")))
            {
                2
            } else {
                w.len()
            };
            w.len() == g.len() && w[..n] == g[..n]
        } else if key == "E1 frame" {
            frame_close(value, got)
        } else if key.starts_with("E3") {
            value.split(", after").next() == got.split(", after").next()
        } else {
            value == got
        };
        if !same {
            bad.push(format!(
                "{key}: device{}, emulator{}",
                strip_tail(value),
                strip_tail(got)
            ));
        }
    }
    bad
}

/// Two `E1 frame` values agree: the same lines, timings within 0.2% (the
/// device's own runs differ by ~0.02%: 17712 and 17710 us a frame).
fn frame_close(want: &str, got: &str) -> bool {
    let nums = |v: &str| -> Vec<u64> {
        v.split(|c: char| !c.is_ascii_digit())
            .filter_map(|t| t.parse().ok())
            .collect()
    };
    let (w, g) = (nums(want), nums(got));
    // lines 0..TOP (N), US us a frame (MHZ mHz), NS ns a line
    w.len() == 6
        && g.len() == 6
        && w[..3] == g[..3]
        && w[3..]
            .iter()
            .zip(&g[3..])
            .all(|(&a, &b)| a.abs_diff(b) * 500 <= a)
}

/// A run-log value without its verdict, for messages.
fn strip_tail(v: &str) -> String {
    format!(" `{}`", v.split(" ->").next().unwrap_or(v))
}

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

/// The alpha test, depth "less" and 8-bit grey textures (rp-glesfeat,
/// ADR-0026, 2026-10-18): every case runs and is logged, back at the loader
/// on a button; with the fork's model of them (`mbx-3d-alpha`, rp-v0.22.0)
/// every pixel as the X51v drew it (docs/hw/emulator.md).
pub fn boot_loader_glesfeat(root: &Path) -> Result<(), String> {
    if !device_has(root, "intel-2700g", "mbx-3d") {
        eprintln!("    glesfeat: skipped (this emulator release doesn't model the MBX 3D core)");
        return Ok(());
    }
    // One of tests G-G3 alone (`RP_EMU_GLESFEAT_ONLY=g3`), for checking a
    // changed test before its device run without the half hour the others
    // take; `cargo xtask ci` (before every push) runs them all.
    match std::env::var("RP_EMU_GLESFEAT_ONLY").as_deref() {
        Ok("g") => return boot_loader_glesfeat_g(root, 1),
        Ok("g2") => return boot_loader_glesfeat_g(root, 2),
        Ok("g3") => return boot_loader_glesfeat_g(root, 3),
        Ok("g4") => return boot_loader_glesfeat_g(root, 4),
        Ok("g5") => return boot_loader_glesfeat_g5(root),
        Ok("g6") => return boot_loader_glesfeat_g(root, 6),
        Ok("g7") => return boot_loader_glesfeat_g(root, 7),
        Ok("g8") => return boot_loader_glesfeat_g(root, 8),
        Ok("g9") => return boot_loader_glesfeat_g9(root),
        Ok("g10") => return boot_loader_glesfeat_g(root, 10),
        Ok("g11") => return boot_loader_glesfeat_g(root, 11),
        Ok("g12") => return boot_loader_glesfeat_g(root, 12),
        Ok("g13") => return boot_loader_glesfeat_g(root, 13),
        Ok("g14") => return boot_loader_glesfeat_g(root, 14),
        Ok("g15") => return boot_loader_glesfeat_g(root, 15),
        Ok("g16") => return boot_loader_glesfeat_g(root, 16),
        Ok("g17") => return boot_loader_glesfeat_g(root, 17),
        Ok("g18") => return boot_loader_glesfeat_g(root, 18),
        Ok("g19") => return boot_loader_glesfeat_g(root, 19),
        Ok("g20") => return boot_loader_glesfeat_g(root, 20),
        Ok("g21") => return boot_loader_glesfeat_g(root, 21),
        Ok(other) => {
            return Err(format!("RP_EMU_GLESFEAT_ONLY={other}: want g or g2-g21"));
        }
        Err(_) => {}
    }
    let loader = raw_image(root, "rp-loader")?;
    let bin = raw_image(root, "rp-glesfeat")?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let check = || -> Result<(), String> {
        let mut mon = crate::screen::Monitor::connect(monitor_port())?;
        mon.query("sendkey ret 200")?;
        Ok(())
    };
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&rp_loadproto::frame(&payload), "RETROPANE-GLESFEAT: ")),
            during: Some(&check),
            then: Some("RPL END"),
            timeout: Duration::from_secs(180),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("glesfeat: never came back to the loader")?;
    if crashed(log) || log.contains("did not return") {
        return Err(format!("glesfeat: didn't return cleanly:{log}"));
    }
    for want in [
        "parameter memory ready: true",
        "A always, ref 0 (control)",
        "A less 0 (version 23's depth 'less')",
        "D blue z 0.8 mode 1 (version 23's form)",
        "L code 10, texels 0x60, TEX2 0x86086610",
        "M: the second texture unit through the TA",
        "M two units, unit 1 TEX2 0xd6087e10",
        "RETROPANE-GLESFEAT: DONE",
    ] {
        if !log.contains(want) {
            return Err(format!("glesfeat: the run log has no `{want}`:{log}"));
        }
    }
    // With the model of the device's results (rp-v0.22.0): the pixels the
    // X51v drew (2026-10-18), word for word.
    if device_has(root, "intel-2700g", "mbx-3d-alpha") {
        for want in [
            "A always, ref 0 (control): ISP 0xe0000000 TSP 0x22200e80: done true; square (200,200) 0x40ff0000",
            "A greater 128: ISP 0x90000000 TSP 0x22200e80: done true; square (200,200) 0xff202020",
            "A greater 32: ISP 0x84000000 TSP 0x22200e80: done true; square (200,200) 0x40ff0000",
            "A less 128: ISP 0x30000000 TSP 0x22200e80: done true; square (200,200) 0x40ff0000",
            "A less 0 (version 23's depth 'less'): ISP 0x20000000 TSP 0x22200e80: done true; square (200,200) 0xff202020",
            "D blue z 0.3 mode 1 (alpha always), ISP 0xe0000000 TSP 0x22200200: done true; overlap (250,250) 0xff0000ff, blue alone (350,350) 0xff202020, red alone (150,150) 0xffff0000",
            "D blue z 0.3 mode 4 (alpha always), ISP 0xe0000000 TSP 0x22200800: done true; overlap (250,250) 0xffff0000, blue alone (350,350) 0xff0000ff",
            "D blue z 0.3 mode 1 (version 23's form), ISP 0x20000000 TSP 0x22200200: done true; overlap (250,250) 0xffff0000, blue alone (350,350) 0xff202020",
            "D blue z 0.8 mode 1 (alpha always), ISP 0xe0000000 TSP 0x22200200: done true; overlap (250,250) 0xffff0000, blue alone (350,350) 0xff202020",
            "L code 10, texels 0x60, TEX2 0x86084610: done true; square (200,200) 0x60606060",
            "L code 10, texels 0xc0, TEX2 0x86084610: done true; square (200,200) 0xc0c0c0c0",
            "L code 10, texels 0x60, TEX2 0x86086610: done true; square (200,200) 0xff606060",
        ] {
            if !log.contains(want) {
                return Err(format!("glesfeat: not as on the device, no `{want}`:{log}"));
            }
        }
        eprintln!("    glesfeat OK: every case drew as on the X51v, back at the loader");
        boot_loader_glesfeat_g(root, 1)?;
        boot_loader_glesfeat_g(root, 2)?;
        boot_loader_glesfeat_g(root, 3)?;
        return boot_loader_glesfeat_g(root, 4);
    }
    eprintln!("    glesfeat OK: every case ran and was logged, back at the loader");
    boot_loader_glesfeat_g(root, 1)?;
    boot_loader_glesfeat_g(root, 2)?;
    boot_loader_glesfeat_g(root, 3)?;
    boot_loader_glesfeat_g(root, 4)
}

/// Test G (`RP_GLESFEAT_G`, the give-up audit's item 1): Gouraud through
/// the TA against our objects at the cores' divide 1; `test` 2, test G2
/// (`RP_GLESFEAT_G2`): the TA's choices one at a time through our direct
/// path, then the TA; 3, test G3 (`RP_GLESFEAT_G3`): the backgrounds,
/// Dell's colour clear among them, at three divide-1 clocks. Every way that
/// draws has a repeatable reference and a
/// right control after each divide-1 setting (the clock put back), and the image
/// returns; a release that doesn't model Windows Mobile's 2700G clocks
/// refuses the change, which the image logs and the check accepts.
/// Test G5 (`RP_GLESFEAT_G5`, the owner's method step 3): Dell's chip
/// state copied under the loader, then halved. The fork draws Gouraud
/// right at every clock, so the image is built with
/// `RP_GLESFEAT_G5_CULPRIT` naming one item (0x6d0, [`G5_CULPRIT`]) a
/// check needs to count as right: the halving must run and end on exactly
/// that item, and the item verified at all three settings. Also held: our
/// way and Dell's whole state draw with repeatable references, every
/// divide-1 setting's control right after it (or the clock refused),
/// Dell's MBX words read back from the core as written, the chip compared
/// with Dell's at 74.75 / 1, no inherited way (the fork starts at Windows
/// Mobile's divide 2: the inherited way isn't exercised here,
/// docs/hw/emulator.md), and the image back at the loader.
fn boot_loader_glesfeat_g5(root: &Path) -> Result<(), String> {
    const G5_CULPRIT: &str = "6";
    let loader = raw_image(root, "rp-loader")?;
    let bin = variant_image_with(
        root,
        "rp-glesfeat",
        ("RP_GLESFEAT_G5", "1"),
        Some(("RP_GLESFEAT_G5_CULPRIT", G5_CULPRIT)),
    )?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let check = || -> Result<(), String> {
        let mut mon = crate::screen::Monitor::connect(monitor_port())?;
        mon.query("sendkey ret 200")?;
        Ok(())
    };
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&rp_loadproto::frame(&payload), "G5: right at 133.25 / 1")),
            during: Some(&check),
            then: Some("RPL END"),
            timeout: Duration::from_secs(1800),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("glesfeat G5: never came back to the loader")?;
    if crashed(log) || log.contains("did not return") {
        return Err(format!("glesfeat G5: didn't return cleanly:{log}"));
    }
    for want in [
        "test G5, Gouraud at the cores' divide 1",
        "parameter memory ready: true",
        "(not divide 1 with a clock running): no inherited way",
        "G5: the emulator's check: a check is right only with item 6 in its mask",
        "G5 way 1 (ours):",
        "G5 state: ours at 74.75 / 1 against Dell's",
        "  ours at 74.75 / 1 after a render: the chip against Dell's",
        "G5 way 2 (ours with Dell's whole state):",
        "G5: ours wrong and Dell's whole state right at 74.75 / 1: halving there",
        "G5: found in ",
        ", mask 0x00040: [0x6d0]",
        "G5: right at 74.75 / 1 of 20: ours 20; Dell's whole state 20; the items found 20",
        "G5: right at 99.67 / 1 of 20: ours 20; Dell's whole state 20; the items found 20",
        "G5: right at 133.25 / 1 of 20: ours 20; Dell's whole state 20; the items found 20",
        "    display after the change: underrun ",
        "RETROPANE-GLESFEAT: DONE",
    ] {
        if !log.contains(want) {
            return Err(format!("glesfeat G5: the run log has no `{want}`:{log}"));
        }
    }
    // Dell's whole state reached the core: way 2's read-back holds every
    // MBX word of the image's list, as written (the fork keeps written
    // words; a test that forgot to write them would pass otherwise).
    let way2 = log
        .split("G5 way 2 (")
        .nth(1)
        .and_then(|s| s.split("read back after the render:").nth(1))
        .and_then(|s| s.lines().next())
        .ok_or(format!("glesfeat G5: way 2 read nothing back:{log}"))?;
    for word in [
        "0x130 0x000087f5",
        "0x610 0x00030001",
        "0x61c 0x00020007",
        "0x6d0 0x00000001",
        "0x6dc 0x3f800001",
        "0x85c 0x01d49ce1",
    ] {
        if !way2.contains(word) {
            return Err(format!(
                "glesfeat G5: way 2's read-back lacks `{word}`: `{way2}`"
            ));
        }
    }
    // Dell's NINTPW at each divide-1 setting of a way that holds it.
    for v in ["0x15", "0x1c", "0x26"] {
        if !log.contains(&format!("    NINTPW {v} by Dell's rule (read back {v})")) {
            return Err(format!(
                "glesfeat G5: no NINTPW {v} by Dell's rule, read back:{log}"
            ));
        }
    }
    // Every check of the halving at one setting; their count as logged.
    let checks = log.matches("G5 check 0x").count();
    let said: usize = log
        .split("G5: found in ")
        .nth(1)
        .and_then(|s| s.split(' ').next())
        .and_then(|n| n.parse().ok())
        .ok_or(format!("glesfeat G5: no check count:{log}"))?;
    if checks == 0 || checks != said || checks > 30 {
        return Err(format!(
            "glesfeat G5: {checks} checks logged, {said} said:{log}"
        ));
    }
    // Every way-setting: a repeatable reference and a right control (none
    // refused: the release models Windows Mobile's clocks). Ways 1, 2 and
    // the verification three settings each, every check one.
    let refs = log
        .matches("rendered true then true, the second 0 words off the first")
        .count();
    let controls = log
        .matches("control at Windows Mobile's clock after: right")
        .count();
    let refused = log.matches("the clock change was refused").count();
    if refused != 0 || refs != 3 + checks || controls != 9 + checks {
        return Err(format!(
            "glesfeat G5: {refs} references, {controls} right controls and {refused} refusals for {checks} checks:{log}"
        ));
    }
    Ok(())
}

/// Test G9 (`RP_GLESFEAT_G9`): every register of the 2700G's two blocks and
/// the PXA27x's, under the loader after our render at Windows Mobile's
/// clock and at 74.75 / 1, for the diff with `rp-wmprobe` v5's. Held: both
/// dumps whole (1,024 MBX words, the system block's 56 known words, every PXA
/// register of `diag`'s list, each once), the renders drawn, the clock put
/// back, the image back at the loader.
fn boot_loader_glesfeat_g9(root: &Path) -> Result<(), String> {
    let loader = raw_image(root, "rp-loader")?;
    let bin = variant_image_with(root, "rp-glesfeat", ("RP_GLESFEAT_G9", "1"), None)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let check = || -> Result<(), String> {
        let mut mon = crate::screen::Monitor::connect(monitor_port())?;
        mon.query("sendkey ret 200")?;
        Ok(())
    };
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&rp_loadproto::frame(&payload), "full loader div1: ends")),
            during: Some(&check),
            then: Some("RPL END"),
            // The emulator writes the ~2,300 lines a dump slowly: 900 s saw
            // one dump and two thirds of the next (2026-10-23).
            timeout: Duration::from_secs(2700),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("glesfeat G9: never came back to the loader")?;
    if crashed(log) || log.contains("did not return") {
        return Err(format!("glesfeat G9: didn't return cleanly:{log}"));
    }
    for want in [
        "test G9, Gouraud at the cores' divide 1",
        "G9 at Windows Mobile's clock: memory manager ready true, rendered true",
        "G9 at 74.75 / 1: memory manager ready true, rendered true",
        "RETROPANE-GLESFEAT: DONE",
    ] {
        if !log.contains(want) {
            return Err(format!("glesfeat G9: the run log has no `{want}`:{log}"));
        }
    }
    let pxa = rp_hal_pxa27x::diag::COUNT;
    for tag in ["loader div2", "loader div1"] {
        let lines: Vec<&str> = log
            .lines()
            .filter(|l| l.starts_with(&format!("full {tag}: ")))
            .collect();
        let count = |what: &str| lines.iter().filter(|l| l.contains(what)).count();
        let (mbx, sys, px) = (count(": MBX 0x"), count(": SYS 0x"), count(": PXA 0x"));
        if mbx != 1024 || sys != 56 || px != pxa || !lines.iter().any(|l| l.ends_with("ends")) {
            return Err(format!(
                "glesfeat G9: {tag}: {mbx} MBX, {sys} SYS and {px} PXA lines, want 1024, 56 and {pxa}:{log}"
            ));
        }
        for frozen in ["SYS 0x012c ", "SYS 0x030c ", "SYS 0x085c ", "SYS 0x0f00 "] {
            if count(frozen) != 0 {
                return Err(format!(
                    "glesfeat G9: {tag} read {frozen}(it froze Windows Mobile):{log}"
                ));
            }
        }
    }
    if !log.contains("read back: COREPLL 0xbb1 MBXCLK 0x6") {
        return Err(format!(
            "glesfeat G9: Windows Mobile's clock not read back:{log}"
        ));
    }
    eprintln!("    glesfeat G9 OK: both dumps whole, back at the loader");
    Ok(())
}

fn boot_loader_glesfeat_g(root: &Path, test: u8) -> Result<(), String> {
    let (env, name, ours, after) = if test == 21 {
        ("RP_GLESFEAT_G21", "G21", "G21 way 1 (", "G21 way 2 (")
    } else if test == 20 {
        ("RP_GLESFEAT_G20", "G20", "G20 way 1 (", "G20 way 2 (")
    } else if test == 19 {
        ("RP_GLESFEAT_G19", "G19", "G19 way 1 (", "G19 way 2 (")
    } else if test == 18 {
        ("RP_GLESFEAT_G18", "G18", "G18 way 1 (", "G18 way 2 (")
    } else if test == 17 {
        ("RP_GLESFEAT_G17", "G17", "G17 way 1 (", "G17 way 2 (")
    } else if test == 16 {
        ("RP_GLESFEAT_G16", "G16", "G16 way 1 (", "G16 way 2 (")
    } else if test == 15 {
        ("RP_GLESFEAT_G15", "G15", "G15 way 1 (", "G15 way 2 (")
    } else if test == 14 {
        ("RP_GLESFEAT_G14", "G14", "G14 way 2 (", "G14 way 3 (")
    } else if test == 13 {
        ("RP_GLESFEAT_G13", "G13", "G13 way 1 (", "G13 way 2 (")
    } else if test == 12 {
        ("RP_GLESFEAT_G12", "G12", "G12 way 1 (", "G12 way 2 (")
    } else if test == 11 {
        ("RP_GLESFEAT_G11", "G11", "G11 way 1 (", "G11 way 2 (")
    } else if test == 10 {
        ("RP_GLESFEAT_G10", "G10", "G10 way 1 (", "G10 way 2 (")
    } else if test == 8 {
        ("RP_GLESFEAT_G8", "G8", "G8 way 1 (", "G8 way 2 (")
    } else if test == 7 {
        ("RP_GLESFEAT_G7", "G7", "G7 way 1 (", "G7 way 2 (")
    } else if test == 6 {
        ("RP_GLESFEAT_G6", "G6", "G6 way 1 (", "G6 way 2 (")
    } else if test == 4 {
        ("RP_GLESFEAT_G4", "G4", "G4 way 1 (", "G4 way 2 (")
    } else if test == 3 {
        ("RP_GLESFEAT_G3", "G3", "G3 way 1 (", "G3 way 2 (")
    } else if test == 2 {
        ("RP_GLESFEAT_G2", "G2", "G2 way 1 (", "G2 way 2 (")
    } else {
        ("RP_GLESFEAT_G", "G", "G way 2 (", "G way 3")
    };
    let loader = raw_image(root, "rp-loader")?;
    // G7 replays a frame file: in the emulator, one of our own scene
    // (`g7::synthetic`: Dell's frames stay local), whose replay must draw
    // the very picture way 1 draws directly. G21 puts its target on the
    // 600 KiB screen: RGB565, as Dell's.
    let frame = root.join("target/g7-synthetic-frame.bin");
    let also = if matches!(
        test,
        7 | 8 | 10 | 11 | 12 | 13 | 14 | 15 | 16 | 17 | 18 | 19 | 20 | 21
    ) {
        std::fs::write(&frame, crate::g7::synthetic(test == 21))
            .map_err(|e| format!("{}: {e}", frame.display()))?;
        Some(("RP_GLESFEAT_G7_FRAME", frame.to_str().unwrap_or("")))
    } else {
        None
    };
    // G14's first way runs on the chip as Dell's driver left it at boot;
    // the emulator starts with every 2700G clock off, so its build runs
    // the way whatever the start (`any-start`, never a device build).
    let value = if test == 14 { "any-start" } else { "1" };
    let bin = variant_image_with(root, "rp-glesfeat", (env, value), also)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let check = || -> Result<(), String> {
        let mut mon = crate::screen::Monitor::connect(monitor_port())?;
        mon.query("sendkey ret 200")?;
        Ok(())
    };
    let summary = format!("{name}: right at 74.75 / 1");
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&rp_loadproto::frame(&payload), &summary)),
            during: Some(&check),
            then: Some("RPL END"),
            timeout: Duration::from_secs(900),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or(format!("glesfeat {name}: never came back to the loader"))?;
    if crashed(log) || log.contains("did not return") {
        return Err(format!("glesfeat {name}: didn't return cleanly:{log}"));
    }
    let wants: &[&str] = if test == 21 {
        &[
            "test G21, Gouraud at the cores' divide 1",
            "parameter memory ready: true",
            "G21: amendment P's C against A, the screen at 0x0 (the safe region ends at 0xae000)",
            "G21 way 1 (Dell's frame off screen, a 2D copy onto the safe screen):",
            "G21 way 2 (Dell's frame drawn straight into the safe screen):",
            "G21 way 3 (Dell's frame into the safe screen, the display paused):",
            "    G21 set-up: the screen at 0x0, GSCADR 0x80000000, ",
            "; the target at 0x200000 (Dell's place, off screen); the display's fetch paused during each render: false; a 2D copy onto the screen after each render: true",
            "; the target at 0x0 (the screen itself); the display's fetch paused during each render: false; a 2D copy onto the screen after each render: false",
            "; the target at 0x0 (the screen itself); the display's fetch paused during each render: true; a 2D copy onto the screen after each render: false",
            "G21: right at 74.75 / 1 of 20:",
            "G21: right at 133.25 / 1 of 20:",
            "RETROPANE-GLESFEAT: DONE",
        ]
    } else if test == 20 {
        &[
            "G17: the screen as found: offset 0x",
            "test G20, Gouraud at the cores' divide 1",
            "parameter memory ready: true",
            "G20 way 1 (Dell's frame, the screen safe, no video plane):",
            "G20 way 2 (Dell's frame, the screen safe, the video plane full outside):",
            "G20 way 3 (Dell's frame, the screen safe, a small video plane inside):",
            "G20 way 4 (Dell's frame, the screen safe, a small video plane outside):",
            "G20 way 5 (a 2D copy, the screen safe):",
            "G20 way 6 (a 2D copy, the screen outside):",
            "    G20 set-up: the screen at 0x0, GSCADR 0x80000000; video None at 0x0, 0 rows",
            "    G20 set-up: the screen at 0x0, GSCADR 0x80000000; video FullOutside at 0x100000, 640 rows",
            "    G20 set-up: the screen at 0x0, GSCADR 0x80000000; video BandInside at 0x96000, 96 rows",
            "    G20 set-up: the screen at 0x0, GSCADR 0x80000000; video BandOutside at 0x100000, 96 rows",
            "    G20 set-up: the screen at 0x100000, GSCADR 0x80010000; video None at 0x0, 0 rows",
            "G20: right at 74.75 / 1 of 20:",
            "RETROPANE-GLESFEAT: DONE",
        ]
    } else if test == 19 {
        &[
            "G17: the screen as found: offset 0x",
            "test G19, Gouraud at the cores' divide 1",
            "parameter memory ready: true",
            "G19: Dell's frame at 74.75 MHz / 1 with the screen at 10 places in local memory; the on-die frame buffer: ODFBPWR 0x",
            "G19 way 1 (Dell's frame, the screen at the place below):",
            "G19: right at 74.75 / 1 of 20:",
            "RETROPANE-GLESFEAT: DONE",
        ]
    } else if test == 18 {
        &[
            "G17: the screen as found: offset 0x",
            "test G18, Gouraud at the cores' divide 1",
            "parameter memory ready: true",
            "G18: Dell's frame at 74.75 MHz / 1 with the screen at 10 places in local memory",
            "G18 way 1 (Dell's frame, the screen at the place below):",
            "G18: right at 74.75 / 1 of 20:",
            "RETROPANE-GLESFEAT: DONE",
        ]
    } else if test == 17 {
        &[
            "G17: the screen as found: offset 0x",
            "  as found: the 2700G's registers below the MBX block",
            "    DMCTRL (0x2188) 0x",
            "  ours (the image's label shown): ",
            "test G17, Gouraud at the cores' divide 1",
            "parameter memory ready: true",
            "G17 way 1 (Dell's frame, the screen at our place):",
            "G17 way 2 (Dell's frame, the screen at Windows Mobile's place):",
            "G17 way 3 (Dell's frame, the screen at our place, Linux's DMCTRL):",
            "G17: right at 74.75 / 1 of 20:",
            "G17: right at 133.25 / 1 of 20:",
            "RETROPANE-GLESFEAT: DONE",
        ]
    } else if test == 16 {
        &[
            "test G16, Gouraud at the cores' divide 1",
            "parameter memory ready: true",
            "G16 way 1 (Dell's frame, as before):",
            "G16 way 2 (Dell's frame, local memory uncached and unbuffered):",
            "G16 way 3 (Dell's frame, the display's fetch off during each render):",
            "G16 way 4 (Dell's frame, both):",
            "    local memory mapped Device (was Some(WriteCombine), reads back Some(Device))",
            "    local memory mapped WriteCombine (was Some(Device), reads back Some(WriteCombine))",
            "G16: right at 74.75 / 1 of 20:",
            "G16: right at 133.25 / 1 of 20:",
            "RETROPANE-GLESFEAT: DONE",
        ]
    } else if test == 15 {
        &[
            "test G15, Gouraud at the cores' divide 1",
            "parameter memory ready: true",
            "G15: Dell's frame at 74.75, 99.67 and 133.25 MHz / 1 with the CPU as found",
            "G15 way 1 (Dell's frame, the CPU as found):",
            "G15 way 2 (Dell's frame, the CPU at 208 MHz, its voltage as found):",
            "G15 way 3 (Dell's frame, the CPU at 208 MHz and Dell's voltage for it):",
            "G15: right at 74.75 / 1 of 20:",
            "G15: right at 133.25 / 1 of 20:",
            "): as found",
            "RETROPANE-GLESFEAT: DONE",
        ]
    } else if test == 14 {
        &[
            "test G14, Gouraud at the cores' divide 1",
            "G14 way 1 (Dell's frame at the clock his driver set at boot, no clock of ours changed):",
            "  not at Dell's divide 1: run anyway (RP_GLESFEAT_G14=any-start, the emulator's build)",
            "    display after the 3D clock on: ",
            "  at Dell's clock, before ours: ",
            "  the clock as found throughout: true",
            "G14: right at Dell's own clock of 20: Dell's frame 20",
            "parameter memory ready: true",
            "G14 way 2 (ours, our places):",
            "G14 way 3 (Dell's frame replayed):",
            "G14: right at 74.75 / 1 of 20:",
            "G14: right at 133.25 / 1 of 20:",
            "RETROPANE-GLESFEAT: DONE",
        ]
    } else if test == 13 {
        &[
            "test G13, Gouraud at the cores' divide 1",
            "parameter memory ready: true",
            "G13 way 1 (ours, the display at the loader's 60 Hz):",
            "G13 way 4 (Dell's frame, the display at Windows Mobile's 56 Hz):",
            "    DISPPLL 0xfc5 -> 0xec5",
            "    DISPPLL 0xec5 -> 0xfc5",
            "G13: right at 74.75 / 1 of 20:",
            "G13: right at 133.25 / 1 of 20:",
            "RETROPANE-GLESFEAT: DONE",
        ]
    } else if test == 12 {
        &[
            "test G12, Gouraud at the cores' divide 1",
            "parameter memory ready: true",
            "G12 way 1 (ours as before):",
            "G12 way 3 (Dell's frame, the CPU silent while it renders):",
            "G12 way 5 (Dell's frame, the CPU silent and the loader-only clocks off):",
            "G12: right at 74.75 / 1 of 20:",
            "G12: right at 133.25 / 1 of 20:",
            "G12: CKEN at the end ",
            "RETROPANE-GLESFEAT: DONE",
        ]
    } else if test == 11 {
        &[
            "test G11, Gouraud at the cores' divide 1",
            "parameter memory ready: true",
            "G11 way 1 (ours, CS2 as found):",
            "G11 way 2 (ours, CS2 Dell's at divide 1):",
            "G11 way 3 (Dell's frame, CS2 Dell's at divide 1):",
            "G11 way 4 (Dell's frame, CS2, NINTPW and clock order Dell's):",
            // Dell's CS2 half reached MSC1 at divide 1, and Windows
            // Mobile's is back at the end.
            "(CS2's half Dell's at divide 1)",
            "    NINTPW 0x15 by Dell's rule (read back 0x15)",
            "G11: right at 74.75 / 1 of 20:",
            "G11: right at 133.25 / 1 of 20:",
            "RETROPANE-GLESFEAT: DONE",
        ]
    } else if test == 10 {
        &[
            "test G10, Gouraud at the cores' divide 1",
            "parameter memory ready: true",
            "G10 way 1 (ours as before: written, rendered and read at divide 1):",
            "G10 way 2 (ours: written and read at Windows Mobile's clock, rendered at divide 1):",
            "G10 way 3 (Dell's frame: written and read at Windows Mobile's clock, rendered at divide 1):",
            "G10 way 4 (ours: written and rendered at divide 1, read at Windows Mobile's clock):",
            // 20 renders, a change to divide 1 and back around each.
            "    40 clock changes around the renders",
            "G10: right at 74.75 / 1 of 20:",
            "G10: right at 99.67 / 1 of 20:",
            "G10: right at 133.25 / 1 of 20:",
            "RETROPANE-GLESFEAT: DONE",
        ]
    } else if test == 8 {
        &[
            "test G8, Gouraud at the cores' divide 1",
            "parameter memory ready: true",
            "G8 way 1 (ours, the status read without a pause (as before)):",
            "G8 way 2 (ours, the chip left alone, the status read once after):",
            "G8 way 3 (ours, the status read once a millisecond):",
            "G8 way 4 (Dell's frame, the chip left alone, the status read once after):",
            "G8 way 5 (Dell's frame, the status read once a millisecond):",
            "  Dell's own picture: none in this frame",
            "G8: right at 74.75 / 1 of 20:",
            "G8: right at 99.67 / 1 of 20:",
            "G8: right at 133.25 / 1 of 20:",
            "    display after the change: underrun ",
            "RETROPANE-GLESFEAT: DONE",
        ]
    } else if test == 7 {
        &[
            "test G7, Gouraud at the cores' divide 1",
            "parameter memory ready: true",
            "G7: the frame: ",
            "target 0x200000 (ARGB8888), parameters 0x600000 (1024 KiB, table 0x700000), region array 0xd10000",
            "G7 way 1 (ours, our places):",
            "G7 way 2 (Dell's frame replayed):",
            "G7 way 3 (Dell's frame replayed, his NINTPW and clock order):",
            "  Dell's own picture: none in this frame",
            "    NINTPW 0x15 by Dell's rule (read back 0x15)",
            "G7: right at 74.75 / 1 of 20:",
            "G7: right at 99.67 / 1 of 20:",
            "G7: right at 133.25 / 1 of 20:",
            "    display after the change: underrun ",
            "; the screen on throughout: ",
            "RETROPANE-GLESFEAT: DONE",
        ]
    } else if test == 6 {
        &[
            "test G6, Gouraud at the cores' divide 1",
            "parameter memory ready: true",
            "G6 way 1 (ours, our places):",
            "G6 way 2 (Dell's parameter memory (8 MiB at 0x22b000)):",
            "G6 way 3 (Dell's places and state (RGB565 target)):",
            "G6 way 4 (Dell's places and state, through the TA):",
            // Each layout reached the core: our addresses, then Dell's
            // parameter memory (its table, first and last page, base), then
            // all his places with his RGB565 target (`0x650` 1, stride
            // 0xf0 words).
            "  layout read back after the render: 0x608 0x00d10000 0x60c 0x00600000 0x650 0x00000006 0x65c 0x00200000 0x660 0x000001e0 0x824 0x00700000",
            "  layout read back after the render: 0x608 0x00d10000 0x60c 0x0022b000 0x650 0x00000006 0x65c 0x00b69000 0x660 0x000001e0 0x824 0x001f8000 0x828 0x0000022c 0x82c 0x00000a2a 0x83c 0x0022b000",
            "  layout read back after the render: 0x608 0x00f35000 0x60c 0x0022b000 0x650 0x00000001 0x65c 0x00e9d000 0x660 0x000000f0 0x824 0x001f8000 0x828 0x0000022c 0x82c 0x00000a2a 0x83c 0x0022b000",
            // Dell's state with his places (way 3).
            "  read back after the render: 0x130 0x000087f5 0x610 0x00030001",
            "G6: right at 74.75 / 1 of 20:",
            "G6: right at 99.67 / 1 of 20:",
            "G6: right at 133.25 / 1 of 20:",
            "    display after the change: underrun ",
            "; the screen on throughout: ",
            "RETROPANE-GLESFEAT: DONE",
        ]
    } else if test == 4 {
        &[
            "test G4, Gouraud at the cores' divide 1",
            "parameter memory ready: true",
            "G4 way 1 (ours):",
            "G4 way 2 (G2's colourless clear):",
            "G4 way 3 (Dell's table, 32-bit):",
            "G4 way 4 (Dell's table and RGB565):",
            "G4 way 10 (Dell's 0x6dc alone):",
            // Dell's words reached the core (way 3's readback after its
            // render; a test that forgot to write them would pass otherwise:
            // the fork doesn't model what they do).
            "  read back after the render: 0x610 0x00030001 0x614 0x1e3ce500 0x618 0x1e3ce500 0x61c 0x00020007 0x648 0x00000001 0x6d8 0x09000000 0x6dc 0x3f800001",
            "G4: right at 74.75 / 1 of 20:",
            "G4: right at 99.67 / 1 of 20:",
            "G4: right at 133.25 / 1 of 20:",
            "    display after the change: underrun ",
            "; the screen on throughout: ",
            "RETROPANE-GLESFEAT: DONE",
        ]
    } else if test == 3 {
        &[
            "test G3, Gouraud at the cores' divide 1",
            "parameter memory ready: true",
            "G3 way 1 (our giant flat triangle):",
            "G3 way 2 (Dell's clear (a flat quad)):",
            "G3 way 3 (Dell's clear and 4-corner quad):",
            "G3 way 4 (flat, screen-sized triangles):",
            "G3 way 5 (our giant triangle at z 1.0):",
            "G3 way 6 (a Gouraud giant triangle):",
            "G3 way 7 (no background):",
            "G3 way 8 (Dell's clear and strip, through the TA):",
            "G3: right at 74.75 / 1 of 20:",
            "G3: right at 99.67 / 1 of 20:",
            "G3: right at 133.25 / 1 of 20:",
            "    display after the change: underrun ",
            "    display back at Windows Mobile's clock: underrun ",
            "; the screen on throughout: ",
            "RETROPANE-GLESFEAT: DONE",
        ]
    } else if test == 2 {
        &[
            "test G2, Gouraud at the cores' divide 1",
            "parameter memory ready: true",
            "G2 way 1 (ours (G's way 2)):",
            "G2 way 2 (one 4-corner object):",
            "G2 way 3 (z 1.0):",
            "G2 way 4 (the clear underneath):",
            "G2 way 5 (per-tile lists):",
            "G2 way 6 (all four: the TA's form):",
            "G2 way 7 (through the TA, one strip of four):",
            "G2 way 8 (through the TA):",
            "G2: right at 74.75 / 1 of 20:",
            "RETROPANE-GLESFEAT: DONE",
        ]
    } else {
        &[
            "test G, Gouraud at the cores' divide 1",
            "parameter memory ready: true",
            "G way 1 (through the TA):",
            "G way 2 (our objects, our layout 4):",
            "G way 4 (through the TA, one strip of four):",
            "G: right at 74.75 / 1 of 20:",
            "RETROPANE-GLESFEAT: DONE",
        ]
    };
    for want in wants {
        if !log.contains(want) {
            return Err(format!(
                "glesfeat {name}: the run log has no `{want}`:{log}"
            ));
        }
    }
    // Every way that drew: its reference rendered twice the same, then the
    // clock refused or a right control after it. The fork doesn't model
    // the tile accelerator (docs/hw/emulator.md: its TA never says done),
    // so a TA way may stop at its reference, safely (and G's way 3 is
    // skipped for want of the TA's object); our own way must always draw.
    let head = format!("{name} way ");
    let ways = log
        .lines()
        .filter(|l| l.starts_with(&head) && l.trim_end().ends_with("):"))
        .count();
    let refs = log
        .matches("rendered true then true, the second 0 words off the first")
        .count();
    let no_ta = log.matches("no repeatable reference").count();
    let ta_silent = log.matches("the TA never said done: no render").count();
    // References the image refused for holding no quad (G4's colourless
    // clear drawn alone first leaves the next renders white, on the X51v
    // and in the fork alike): the guard working, not a fault. In G4 it is
    // exactly way 2; nowhere else.
    let no_quad = log.matches("the reference holds no quad").count();
    let want_no_quad = usize::from(test == 4);
    if no_quad != want_no_quad {
        return Err(format!(
            "glesfeat {name}: {no_quad} references refused for holding no quad, want {want_no_quad}:{log}"
        ));
    }
    // Each way that stopped for want of the TA: its two reference renders.
    if (no_ta - no_quad.min(no_ta)) * 2 != ta_silent {
        return Err(format!(
            "glesfeat {name}: {no_ta} references that didn't repeat, {ta_silent} renders the TA never finished:{log}"
        ));
    }
    let drew = ways - no_ta.min(ways);
    let refused = log.matches("the clock change was refused").count();
    let controls = log
        .matches("control at Windows Mobile's clock after: right")
        .count();
    // A refused no-quad reference still rendered twice alike: not a way
    // that drew.
    if refs - no_quad.min(refs) != drew {
        return Err(format!(
            "glesfeat {name}: {refs} repeatable references for {drew} ways that drew:{log}"
        ));
    }
    // Each way that drew runs every setting: G3 three, G and G2 one;
    // G14's first way none (it stays at the clock it found).
    let settings = if test == 18 || test == 19 || test == 20 {
        1
    } else if test >= 3 {
        3
    } else {
        1
    };
    let clocked = drew - usize::from(test == 14).min(drew);
    if clocked == 0 || refused + controls != clocked * settings {
        return Err(format!(
            "glesfeat {name}: {drew} ways drew, {refused} refused the clock and {controls} came back right:{log}"
        ));
    }
    let own = log
        .split(ours)
        .nth(1)
        .and_then(|s| s.split(after).next())
        .unwrap_or("");
    if !own.contains("rendered true then true, the second 0 words off the first") {
        return Err(format!("glesfeat {name}: our own way didn't draw:{log}"));
    }
    // Ways that must draw one picture, in the forms the fork models (list
    // layout 4): G2's two-triangle forms over our flat background (ways 1,
    // 3 and 5: ours, z 1.0, per-tile lists); G3's black layout-4
    // backgrounds under the same quad (ways 1, 4, 5 and 6: giant, screen
    // triangles, z 1.0, Gouraud). Their references' hashes equal, so a way
    // that drew a wrong picture the same way twice (a broken region array,
    // an empty list) is caught too.
    let same: &[u32] = match test {
        2 => &[3, 5],
        3 => &[4, 5, 6],
        6 => &[2],
        // The replays of our own scene: way 1's picture exactly.
        7 => &[2, 3],
        // Every wait, and the replays of our own scene: one picture.
        8 => &[2, 3, 4, 5],
        // Every mode, and the replay of our own scene: one picture.
        10 => &[2, 3, 4],
        // CS2's timing, and the replays of our own scene: one picture.
        11 => &[2, 3, 4],
        // The replays of our own scene, silent or clocks off: one picture.
        12 => &[2, 3, 4, 5],
        // Either display clock, ours or the replay of our scene: one picture.
        13 => &[2, 3, 4],
        // The replay of our scene at the clock found (way 1, the anchor),
        // ours and the replay after our clock change: one picture.
        14 => &[2, 3],
        // The replay of our scene at every CPU speed and voltage.
        15 => &[2, 3],
        // The replay of our scene however mapped, the display on or off.
        16 => &[2, 3, 4],
        // The replay of our scene wherever the screen is, either DMCTRL.
        17 => &[2, 3],
        // The replay of our scene at every screen place.
        // Every place drawn: the emulator's frame leaves one out (nine).
        18 => &[2, 3, 4, 5, 6, 7, 8, 9],
        // Every place drawn (none over the emulator's frame).
        19 => &[2, 3, 4, 5, 6, 7, 8, 9, 10],
        // The replay of our scene whatever the video plane does.
        20 => &[2, 3, 4],
        // The replay of our scene wherever its target sits: one picture.
        21 => &[2, 3],
        _ => &[],
    };
    if !same.is_empty() {
        let hash = |way: u32| {
            log.split(&format!("{name} way {way} ("))
                .nth(1)
                .and_then(|s| s.split("reference hash ").nth(1))
                .map(|s| s.chars().take(10).collect::<String>())
        };
        let first = hash(1).ok_or(format!(
            "glesfeat {name}: way 1 has no reference hash:{log}"
        ))?;
        for &way in same {
            if hash(way).as_deref() != Some(first.as_str()) {
                return Err(format!(
                    "glesfeat {name}: way {way}'s reference isn't way 1's picture ({:?} against {first}):{log}",
                    hash(way)
                ));
            }
        }
    }
    // G3 and G4: every background drawn alone first (the quad hides it in
    // the reference), way 1's (the giant flat triangle) anchored to all
    // opaque black; in G3 the two others that cover the whole target black
    // in the fork's modelled form (ways 5 and 6: the giant triangle at z
    // 1.0, the giant black Gouraud one) the same picture. A background that
    // didn't draw, or drew the wrong colour, is caught here: the reference
    // can't show it.
    if test == 3 || test == 4 {
        let (same, alone_want): (&[u32], usize) = if test == 3 { (&[5, 6], 6) } else { (&[], 10) };
        let bg = |way: u32| {
            log.split(&format!("{name} way {way} ("))
                .nth(1)
                .and_then(|s| s.split("  background alone: ").nth(1))
                .map(|s| s.lines().next().unwrap_or("").trim().to_string())
        };
        let first = bg(1).ok_or(format!(
            "glesfeat {name}: way 1 has no background alone:{log}"
        ))?;
        // The anchor: the giant triangle covers every pixel, so its picture
        // alone is 480 x 640 words of opaque black, hashed as the image
        // hashes (FNV-1a over words). Without it the group could agree on a
        // wrong picture (all three drawn white, say).
        let black = (0..480 * 640).fold(0x811C_9DC5u32, |h, _| {
            (h ^ 0xFF00_0000).wrapping_mul(0x0100_0193)
        });
        let want = format!("rendered true, hash 0x{black:08x}");
        if first != want {
            return Err(format!(
                "glesfeat {name}: way 1's background alone is {first:?}, want {want:?} (all opaque black):{log}"
            ));
        }
        for &way in same {
            if bg(way).as_deref() != Some(first.as_str()) {
                return Err(format!(
                    "glesfeat {name}: way {way}'s background isn't way 1's ({:?} against {first}):{log}",
                    bg(way)
                ));
            }
        }
        let alone = log.matches("  background alone: rendered true").count();
        if alone != alone_want {
            return Err(format!(
                "glesfeat {name}: {alone} backgrounds rendered alone, want {alone_want} (every direct way with a background):{log}"
            ));
        }
    }
    // The control after divide 1 proves the clock was put back; the same
    // read back as Windows Mobile's.
    if controls > 0 && !log.contains("read back: COREPLL 0xbb1 MBXCLK 0x6") {
        return Err(format!(
            "glesfeat {name}: Windows Mobile's clock not read back:{log}"
        ));
    }
    // G21: each way drew where it says (0x65C read back after the
    // reference: Dell's place in way 1, the screen in ways 2 and 3), and at
    // every setting it ran: 20 renders, way 1's 20 copies onto the screen
    // each exact, no copy in the others, the display's fetch read back off
    // in way 3's renders only.
    if test == 21 {
        for way in 1..=3u32 {
            let seg = log
                .split(&format!("{name} way {way} ("))
                .nth(1)
                .and_then(|s| s.split(&format!("{name} way {} (", way + 1)).next())
                .unwrap_or("");
            let target = if way == 1 { 0x0020_0000u32 } else { 0 };
            let readback = format!(" 0x65c 0x{target:08x} ");
            if !seg.contains("  layout read back after the render:") || !seg.contains(&readback) {
                return Err(format!(
                    "glesfeat {name}: way {way}'s target didn't read back at 0x{target:x}:{log}"
                ));
            }
            let reports: Vec<&str> = seg
                .lines()
                .filter(|l| l.starts_with("    G21 at ") && l.contains(", the 20 renders: "))
                .collect();
            let ran = 3 - seg.matches("the clock change was refused").count();
            if reports.len() != ran || ran == 0 {
                return Err(format!(
                    "glesfeat {name}: way {way} reported {} settings' renders, want {ran}:{log}",
                    reports.len()
                ));
            }
            let copies = if way == 1 {
                "; 20 2D copies onto the screen, "
            } else {
                "; 0 2D copies onto the screen, "
            };
            for r in reports {
                let paused = !r.ends_with("read back off: 0");
                if !r.contains(": 20 renders, ")
                    || !r.contains(copies)
                    || !r.contains(", 0 not exact (0 words off in all)")
                    || paused != (way == 3)
                {
                    return Err(format!("glesfeat {name}: way {way}: {r}:{log}"));
                }
            }
        }
    }
    // G16: each way's mapping and the display's fetch as set, and renders
    // made with the fetch read back off in ways 3 and 4 only (each count is
    // logged at the next way's set-up, the last at the end).
    // G20: the 2D copies (ways 5 and 6) one picture, not the frame's, and
    // the video plane on (its stream bit) in ways 2-4 only.
    if test == 20 {
        let hash = |way: u32| {
            log.split(&format!("{name} way {way} ("))
                .nth(1)
                .and_then(|s| s.split("reference hash ").nth(1))
                .map(|s| s.chars().take(10).collect::<String>())
        };
        if hash(5).is_none() || hash(5) != hash(6) || hash(5) == hash(1) {
            return Err(format!(
                "glesfeat {name}: the 2D copies' references {:?} and {:?} (the frame's {:?}):{log}",
                hash(5),
                hash(6),
                hash(1)
            ));
        }
        let vscadr = |way: u32| {
            log.split(&format!("{name} way {way} ("))
                .next()
                .and_then(|b| b.rsplit("    G20 set-up: ").next())
                .and_then(|l| l.split("VSCADR 0x").nth(1))
                .and_then(|h| u32::from_str_radix(&h[..8.min(h.len())], 16).ok())
        };
        for way in 1..=6u32 {
            let on = vscadr(way).map(|v| v >> 31 == 1);
            if on != Some((2..=4).contains(&way)) {
                return Err(format!(
                    "glesfeat {name}: way {way}'s video stream on: {on:?}, want {}:{log}",
                    (2..=4).contains(&way)
                ));
            }
        }
    }
    // G18: every place that wasn't left out drew, its screen set and read
    // back there (`GSCADR` its place / 16, the stream on), in order.
    if test == 18 || test == 19 {
        let places: [usize; 10] = if test == 18 {
            [
                0x0, 0x400, 0x800, 0x1000, 0x2000, 0x8_0000, 0x10_0000, 0x12_0000, 0xC8_0000,
                0xF0_0000,
            ]
        } else {
            [
                0x4000, 0x8000, 0x1_0000, 0x1_8000, 0x2_0000, 0x3_0000, 0x4_0000, 0x5_0000,
                0x6_0000, 0x7_0000,
            ]
        };
        let left = format!("{name}: place ");
        // Each place either set at its own address or named as left out
        // (one over the frame: the emulator's frame, our own scene, lies
        // elsewhere than Dell's), never both.
        let mut seen = Vec::new();
        for line in log.lines() {
            if let Some(rest) = line.strip_prefix(left.as_str())
                && line.ends_with("left out")
            {
                let k: usize = rest
                    .split(' ')
                    .next()
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(0);
                seen.push((k, false));
            }
            if let Some(rest) = line.strip_prefix("    the screen at 0x")
                && let Some(p) = rest.split("(place ").nth(1)
            {
                let k: usize = p
                    .split(')')
                    .next()
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(0);
                let at = places.get(k.wrapping_sub(1)).copied().unwrap_or(usize::MAX);
                let want = format!(
                    "{at:x} (place {k}), GSCADR 0x{:08x}",
                    1u32 << 31 | (at / 16) as u32
                );
                if !rest.starts_with(&want) {
                    return Err(format!(
                        "glesfeat {name}: want `{want}`, got `{line}`:{log}"
                    ));
                }
                seen.push((k, true));
            }
        }
        let mut ks: Vec<usize> = seen.iter().map(|s| s.0).collect();
        ks.sort_unstable();
        if ks != (1..=places.len()).collect::<Vec<_>>() || seen.iter().filter(|s| s.1).count() < 8 {
            return Err(format!(
                "glesfeat {name}: places set (true) or left out {seen:?}, want each of 1-{} once, 8 or more set:{log}",
                places.len()
            ));
        }
    }
    // G17: each way's screen place and DMCTRL as set and read back: way 2
    // at the place found at start (not ours), way 3 with Linux's DMCTRL.
    if test == 17 {
        let found = log
            .split("G17: the screen as found: offset 0x")
            .nth(1)
            .and_then(|s| s.split(',').next())
            .and_then(|h| usize::from_str_radix(h, 16).ok())
            .ok_or(format!("glesfeat {name}: no screen as found:{log}"))?;
        // Each way's set-up is logged just before its header.
        let set = |way: u32| {
            log.split(&format!("{name} way {way} ("))
                .next()
                .and_then(|before| before.rsplit("    the screen at 0x").next())
                .map(|s| s.lines().next().unwrap_or("").to_string())
                .unwrap_or_default()
        };
        let ours = 0x0010_0000usize;
        for (way, at, gscadr_ok, dmctrl) in [
            (1, ours, true, None),
            (2, found, true, None),
            (3, ours, true, Some("DMCTRL 0xd0303010 (Linux's)")),
        ] {
            let line = set(way);
            let want_at = format!("{at:x} (");
            // GSCADR holds the plane's place / 16 with the stream on.
            let want_g = format!("GSCADR 0x{:08x}", 1u32 << 31 | (at / 16) as u32);
            if !line.starts_with(&want_at) || (gscadr_ok && !line.contains(&want_g)) {
                return Err(format!(
                    "glesfeat {name}: way {way}'s screen isn't at 0x{at:x} ({want_g}): {line}:{log}"
                ));
            }
            if let Some(d) = dmctrl
                && !line.contains(d)
            {
                return Err(format!(
                    "glesfeat {name}: way {way}'s DMCTRL isn't Linux's: {line}:{log}"
                ));
            }
            if dmctrl.is_none() && line.contains("(Linux's)") {
                return Err(format!(
                    "glesfeat {name}: way {way} has Linux's DMCTRL: {line}:{log}"
                ));
            }
        }
        if found == ours {
            return Err(format!(
                "glesfeat {name}: the screen found at start is the label's own place, so way 2 tests nothing:{log}"
            ));
        }
    }
    if test == 16 {
        let counts: Vec<u32> = log
            .lines()
            .filter_map(|l| {
                l.strip_prefix(
                    "    the chip's runs with the display's fetch off since the last way: ",
                )
                .and_then(|n| n.trim().parse().ok())
            })
            .collect();
        // The first is the set-up before way 1 (nothing before it).
        let ok = counts.len() == 5
            && counts[0] == 0
            && counts[1] == 0
            && counts[2] == 0
            && counts[3] > 0
            && counts[4] > 0;
        if !ok {
            return Err(format!(
                "glesfeat {name}: renders with the display's fetch off {counts:?}, want [0, 0, 0, >0, >0]:{log}"
            ));
        }
        let set = |way: u32| {
            log.split(&format!("{name} way {way} ("))
                .next()
                .and_then(|before| before.rsplit("    local memory mapped ").next())
                .map(|s| s.lines().next().unwrap_or("").to_string())
                .unwrap_or_default()
        };
        for (way, map, dark) in [
            (1, "WriteCombine", false),
            (2, "Device", false),
            (3, "WriteCombine", true),
            (4, "Device", true),
        ] {
            let line = set(way);
            let want_dark = format!("fetch off during each render: {dark}");
            if !line.starts_with(map) || !line.contains(&want_dark) {
                return Err(format!(
                    "glesfeat {name}: way {way}'s set-up isn't {map}, {want_dark}: {line}:{log}"
                ));
            }
        }
    }
    if test == 15 {
        let cpu = |way: u32| {
            log.split(&format!("{name} way {way} ("))
                .next()
                .and_then(|before| before.rsplit("    CPU: ").next())
                .map(|s| s.lines().next().unwrap_or("").to_string())
                .unwrap_or_default()
        };
        let ours_pins = log.contains("the pins plain outputs: true");
        for (way, want, code) in [
            (1, None, None),
            (2, Some("core counted 208"), None),
            (
                3,
                Some("core counted 208"),
                Some("voltage code 0b100 (Dell's label Some(1163) mV)"),
            ),
        ] {
            let line = cpu(way);
            if line.is_empty() {
                return Err(format!("glesfeat {name}: way {way} has no CPU line:{log}"));
            }
            if let Some(w) = want
                && !line.contains("CCCR 0x02000110")
            {
                return Err(format!(
                    "glesfeat {name}: way {way}'s CPU isn't at 208 MHz ({w}): {line}:{log}"
                ));
            }
            if way == 1 && line.contains("CCCR 0x02000110") {
                return Err(format!(
                    "glesfeat {name}: way 1's CPU isn't as found: {line}:{log}"
                ));
            }
            if let (true, Some(c)) = (ours_pins, code)
                && !line.contains(c)
            {
                return Err(format!(
                    "glesfeat {name}: way {way}'s voltage isn't Dell's for 208 MHz: {line}:{log}"
                ));
            }
            if way == 2 && ours_pins && line.contains("voltage code 0b100") {
                return Err(format!(
                    "glesfeat {name}: way 2's voltage was lowered: {line}:{log}"
                ));
            }
        }
    }
    eprintln!(
        "    glesfeat {name} OK: {ways} ways, {drew} drew (references repeatable; the TA not modelled: {no_ta}), the clock refused in {refused}, put back in {controls}, back at the loader"
    );
    Ok(())
}

/// The MBX 3D core's first-triangle test (rp-mbx3d, ADR-0024 M4): runs
/// under the loader, every try timed out (no 3D model), back on a button.
pub fn boot_loader_mbx3d(root: &Path) -> Result<(), String> {
    if !device_has(root, "intel-2700g", "mbx-2d") {
        eprintln!("    mbx3d: skipped (this emulator release doesn't model the MBX 2D engine)");
        return Ok(());
    }
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    let loader = dir.join("rp-loader.bin");
    let bin = dir.join("rp-mbx3d.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-mbx3d"), &bin)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let check = || -> Result<(), String> {
        let mut mon = crate::screen::Monitor::connect(monitor_port())?;
        mon.query("sendkey ret 200")?;
        Ok(())
    };
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&rp_loadproto::frame(&payload), "RETROPANE-MBX3D: ")),
            during: Some(&check),
            then: Some("RPL END"),
            timeout: Duration::from_secs(120),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("mbx3d: never came back to the loader after the button press")?;
    if crashed(log) || log.contains("did not return") {
        return Err(format!("mbx3d: didn't return cleanly:{log}"));
    }
    let modelled = device_has(root, "intel-2700g", "mbx-3d");
    // Without the fork's 3D model (before rp-v0.15.0) the image must try
    // the parameter memory set-up without and with the 3D clock, get no
    // "ready", try no render, and come back. With it: the memory manager
    // is ready only with the 3D clock on (as on the device), and the red
    // triangle draws with the target register set (try A' or B).
    let want: &[&str] = if modelled {
        &[
            "core ID 0x01020000",
            "(3D clock on)",
            "drawn 480 px across, 640 down",
            "RETROPANE-MBX3D: DREW",
        ]
    } else {
        &[
            "core ID 0x01020000",
            "P: parameter memory set-up, MBXCLK",
            "(3D clock on)",
            "the parameter memory never said ready: no render tried",
            "RETROPANE-MBX3D: nothing drawn",
        ]
    };
    for want in want {
        if !log.contains(want) {
            return Err(format!("mbx3d: the run log has no `{want}`:{log}"));
        }
    }
    // With the fork's render-done interrupt and texel model (mbx-3d-irq,
    // rp-v0.16.0): what version 24 saw on the device.
    // (Version 24's stages only: later versions ask other questions.)
    if modelled && device_has(root, "intel-2700g", "mbx-3d-irq") && log.contains("T10 MINTENABLE") {
        let line = |key: &str| log.lines().find(|l| l.contains(key)).unwrap_or("");
        for (key, want) in [
            ("MINTENABLE 0x800f, 0x130 0x0004:", "edges 0"),
            ("MINTENABLE 0x800f, 0x130 0x8004:", "edges 1"),
            ("MINTENABLE 0x8001, 0x130 0x8004:", "edges 0"),
            ("MINTENABLE 0x8004, 0x130 0x8004:", "edges 1"),
            (
                "T12 TEX1 bit 31 (linear)",
                "ffff3f3f, ffff7f7f, ffffbfbf, ffffffff, ffffbfbf",
            ),
            (
                "T13 PVRTC 4 bpp (code 28), red",
                "[ffff0000, ffff0000, ffff0000, ffff0000]",
            ),
            (
                "T13 PVRTC 4 bpp, words swapped",
                "[ff000000, ff000000, ff000000, ff000000]",
            ),
            ("T13 32-bit texels", "[ffff0000, ff00ff00, ff0000ff"),
        ] {
            if !line(key).contains(want) {
                return Err(format!(
                    "mbx3d: `{key}` should show `{want}` as on the device:{log}"
                ));
            }
        }
    }
    // Version 25's stages, likewise (mipmaps and PVRTC's block order,
    // from rp-v0.16.0's model).
    if modelled && device_has(root, "intel-2700g", "mbx-3d-irq") && log.contains("T12b") {
        let line = |key: &str| log.lines().find(|l| l.contains(key)).unwrap_or("");
        for (key, want) in [
            (
                "T12b layout B, TEX0 bits 12-9 = 7:",
                "16 px quad 0xff0000ff, of the 8 px quad 0xffffff00",
            ),
            (
                "T12b layout A, TEX0 bits 12-9 = 7:",
                "16 px quad 0xffff0000, of the 8 px quad 0xffff0000",
            ),
            (
                "T13b PVRTC 4 bpp",
                "[[00, 21, 84, a5], [10, 31, 94, b5], [42, 63, c6, e7], [52, 73, d6, f7]]",
            ),
        ] {
            if !line(key).contains(want) {
                return Err(format!(
                    "mbx3d: `{key}` should show `{want}` as on the device:{log}"
                ));
            }
        }
    }
    // Version 26's 16-bit targets, likewise.
    if modelled && device_has(root, "intel-2700g", "mbx-3d-irq") && log.contains("S26b") {
        let line = |key: &str| log.lines().find(|l| l.contains(key)).unwrap_or("");
        for (key, want) in [
            (
                "S26b 0x650 1, 0x660 240:",
                "(2,2) 0xf800 (18,2) 0x07e0 (2,34) 0x001f (18,34) 0xffff; drawn to x Some(32) on row 2, y Some(64)",
            ),
            (
                "S26b 0x650 3, 0x660 240:",
                "(2,2) 0xfc00 (18,2) 0x83e0 (2,34) 0x801f (18,34) 0xffff",
            ),
        ] {
            if !line(key).contains(want) {
                return Err(format!(
                    "mbx3d: `{key}` should show `{want}` as on the device:{log}"
                ));
            }
        }
    }
    // Version 29's option A: the model has neither the PXA's bus timing
    // nor the 2700G's clock and memory registers as Windows Mobile leaves
    // them, so the image measures today's setting once (every check must
    // pass) and changes nothing.
    if modelled {
        for want in [
            "S29 as found",
            "S29 today: 99.67 MHz, divide by 2, CS3 0x2484:",
            "S29 not Windows Mobile's state: nothing changed",
        ] {
            if !log.contains(want) {
                return Err(format!("mbx3d: the run log has no `{want}`:{log}"));
            }
        }
        let today = log.lines().find(|l| l.contains("S29 today:")).unwrap_or("");
        if !today.ends_with("| GOOD") {
            return Err(format!("mbx3d: today's setting should check GOOD:{log}"));
        }
    }
    eprintln!(
        "    mbx3d OK: {}, back at the loader on a button press",
        if modelled {
            "the red triangle drawn by the modelled 3D core"
        } else {
            "ran every try safely (the 3D core isn't modelled)"
        }
    );
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
    clocks_fault_path(root, &loader, modelled)
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
    if log.contains("MSC1 not as expected: nothing changed") {
        eprintln!(
            "    cs3bus OK: the emulator's MSC1 isn't Windows Mobile's, and it refused to change anything"
        );
        return Ok(());
    }
    for want in [
        "all steps passed",
        "soak at step 5 ",
        " 0 misses",
        "MSC1 restored 24842481",
    ] {
        if !log.contains(want) {
            return Err(format!("cs3bus: the run log has no `{want}`:{log}"));
        }
    }
    eprintln!("    cs3bus OK: every step and the soak ran, MSC1 put back");
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

/// The GPU measuring run for the 3D compositor proposal (rp-gpubench,
/// docs/design/ui.md): run A (fill, layers, partial frames, rectangular
/// textures, uploads, the frame-event watch tests) and run B (the
/// texture-word probes, built with `RP_GPUBENCH_PROBES`), each under the
/// loader, then run C (the video plane's repoint and pans,
/// `RP_GPUBENCH_PLANES`) and run D (its undocumented format codes,
/// `RP_GPUBENCH_VFMT`), run E (faster uploads, `RP_GPUBENCH_UPLOAD`) and
/// run F (text as 4 bpp strips, `RP_GPUBENCH_TEXT`), runs P, P2 and P2W
/// (the 3D core's target formats, `RP_GPUBENCH_TARGETS`,
/// `RP_GPUBENCH_TARGETS2`, `RP_GPUBENCH_TARGETS_WATCH`) and the clock runs,
/// with key presses for their watch steps. The
/// models' times say nothing about the device's (the fork's 3D
/// core doesn't model the clock or the fill costs: emulator.md), so this
/// checks that every stage runs, the 2700G memory pool is all given back,
/// the two upload paths write the same texture, nothing faults, and each
/// run returns to the loader. Run A's watch tests end at their 30 s cap
/// (or at once if the scan line isn't modelled).
pub fn boot_loader_gpubench(root: &Path) -> Result<(), String> {
    if !device_has(root, "intel-2700g", "mbx-2d") {
        eprintln!("    gpubench: skipped (this emulator release doesn't model the MBX 2D engine)");
        return Ok(());
    }
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    let loader = dir.join("rp-loader.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    let run_a = dir.join("rp-gpubench.bin");
    crate::sdkit::raw_binary(&release.join("rp-gpubench"), &run_a)?;
    let run_b = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_PROBES", "1"))?;
    let run_c = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_PLANES", "1"))?;
    let run_d = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_VFMT", "1"))?;
    let run_e = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_UPLOAD", "1"))?;
    let run_f = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_TEXT", "1"))?;
    let run_g = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_CLOCK", "1"))?;
    let run_r = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_REVIVE", "1"))?;
    let run_s = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_SNAPSHOT", "1"))?;
    let run_u = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_UNDERRUN", "1"))?;
    let run_v = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_LOAD", "1"))?;
    let run_x = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_CLEAR", "1"))?;
    let run_l = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_LADDER", "1"))?;
    let run_z = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_REBUILD", "1"))?;
    let run_y = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_RESCUE", "1"))?;
    let run_n = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_START", "1"))?;
    let run_q = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_SLOW", "1"))?;
    let run_p = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_TARGETS", "1"))?;
    let run_p2 = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_TARGETS2", "1"))?;
    let run_p2w = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_TARGETS_WATCH", "1"))?;
    let run_w = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_FINGERPRINT", "1"))?;
    let run_t = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_FETCH", "1"))?;
    let run_t3 = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_RESOLVE", "1"))?;
    let run_p2x = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_SPECKS", "1"))?;
    let run_p2y = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_SPECKS2", "1"))?;
    let run_p2l = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_LONG_COUNT", "1"))?;
    let run_ff = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_FIFO_FULL", "1"))?;
    let run_fs = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_REAL_STALL", "1"))?;
    let run_ft = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_TOGETHER", "1"))?;
    let run_fi = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_IRQ_BITS", "1"))?;
    let run_fd = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_DMA_FEED", "1"))?;
    let run_ch = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_HARNESS", "1"))?;
    let run_c71 = variant_image_with(
        root,
        "rp-gpubench",
        ("RP_GPUBENCH_HARNESS", "1"),
        Some(("RP_GPUBENCH_C71", "1")),
    )?;
    let run_g21 = variant_image_with(
        root,
        "rp-gpubench",
        ("RP_GPUBENCH_HARNESS", "1"),
        Some(("RP_GPUBENCH_G21", "1")),
    )?;
    // Run G21: every kind's reference repeatable, and every way's 20
    // renders exact against it (the fork is exact: the device's are the
    // finding), A's copies exact, C's none.
    let g21_kinds = [
        "flat cards 200 x 260",
        "textured cards 1:1 (128 x 128)",
        "textured cards 1.56x, point sampling",
        "textured cards 1.56x, bilinear",
        "textured cards shrunk 2:1 (64 x 64), point sampling",
    ];
    let mut g21_wants: Vec<String> = vec!["  CH G21: the screen at 0x0 (GSCADR ".into()];
    for k in g21_kinds {
        g21_wants.push(format!(
            "  CH G21 {k}: the reference (the reference: off screen, the display paused): rendered Some("
        ));
        g21_wants.push(" the last 0 words off the one kept []; hash ".into());
        for (way, copies) in [
            ("A: off screen, a 2D copy onto the screen", 20),
            ("C: drawn straight into the screen", 0),
            ("C with the display paused", 0),
        ] {
            g21_wants.push(format!(
                "  CH G21 {k}: {way}: 20 renders (0 unfinished), 0 differ from the reference, 0 words in all []; render fastest "
            ));
            g21_wants.push(format!("us; {copies} 2D copies onto the screen, fastest "));
        }
    }
    g21_wants.extend(
        [
            "  CH G21: done",
            "everything given back: true",
            "RETROPANE-GPUBENCH: done",
        ]
        .map(String::from),
    );
    let g21_wants: Vec<&str> = g21_wants.iter().map(String::as_str).collect();
    let run_lim = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_LIMITS", "1"))?;
    let run_fi2 = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_STATUS2", "1"))?;
    let run_bx = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_BX", "1"))?;
    // Run GS (the clocks' soak) with 70 s a setting: one minute's line and
    // the closing line each.
    let run_gs = variant_image_with(
        root,
        "rp-gpubench",
        ("RP_GPUBENCH_SOAK", "1"),
        Some(("RP_GPUBENCH_SOAK_SECS", "70")),
    )?;
    let run_gv = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_MEMCLOCK", "1"))?;
    let run_gc2f = variant_image_with(
        root,
        "rp-gpubench",
        ("RP_GPUBENCH_GC2_F", "1"),
        Some(("RP_GPUBENCH_GC2_RENDERS", "1")),
    )?;
    let run_gc2h = variant_image_with(
        root,
        "rp-gpubench",
        ("RP_GPUBENCH_GC2_H", "1"),
        Some(("RP_GPUBENCH_GC2_RENDERS", "1")),
    )?;
    let run_gc2r = variant_image_with(
        root,
        "rp-gpubench",
        ("RP_GPUBENCH_GC2_R", "1"),
        Some(("RP_GPUBENCH_GC2_RENDERS", "1")),
    )?;
    // Every 37th flip: the render table, MBXCLK, the system registers and
    // COREPLL all sampled.
    let run_gc2x = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_GC2X_STEP", "37"))?;
    let run_gc2h2 = variant_image_with(
        root,
        "rp-gpubench",
        ("RP_GPUBENCH_GC2_H2", "1"),
        Some(("RP_GPUBENCH_GC2_RENDERS", "1")),
    )?;
    let run_gc2w = variant_image_with(
        root,
        "rp-gpubench",
        ("RP_GPUBENCH_GC2_W", "1"),
        Some(("RP_GPUBENCH_GC2_RENDERS", "1")),
    )?;
    let run_gc3 = variant_image_with(
        root,
        "rp-gpubench",
        ("RP_GPUBENCH_GC3", "1"),
        Some(("RP_GPUBENCH_GC2_RENDERS", "1")),
    )?;
    let run_gc2m = variant_image_with(
        root,
        "rp-gpubench",
        ("RP_GPUBENCH_GC2M", "1"),
        Some(("RP_GPUBENCH_GC2_RENDERS", "1")),
    )?;
    let run_gc2d = variant_image_with(
        root,
        "rp-gpubench",
        ("RP_GPUBENCH_GC2_D", "1"),
        Some(("RP_GPUBENCH_GC2_RENDERS", "1")),
    )?;
    let run_gc = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_CMAP", "1"))?;
    let run_gc2 = variant_image_with(
        root,
        "rp-gpubench",
        ("RP_GPUBENCH_GC2", "1"),
        Some(("RP_GPUBENCH_GC2_RENDERS", "1")),
    )?;
    let run_gc2s = variant_image_with(
        root,
        "rp-gpubench",
        ("RP_GPUBENCH_GC2_SAFE", "1"),
        Some(("RP_GPUBENCH_GC2_RENDERS", "1")),
    )?;
    let run_gc2i = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_INSIDE", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "1"),
            ("RP_GPUBENCH_SOAK_SECS", "10"),
        ],
    )?;
    let run_gc2j = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_SOAK2", "1"),
            ("RP_GPUBENCH_SOAK_SECS", "10"),
        ],
    )?;
    let run_gc2k = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_PATCH", "1"),
            ("RP_GPUBENCH_SOAK_SECS", "10"),
        ],
    )?;
    let run_gc2l = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_SWEEP", "1"),
            ("RP_GPUBENCH_SOAK_SECS", "10"),
        ],
    )?;
    let run_gc2o = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_ORDER", "1"),
            ("RP_GPUBENCH_SOAK_SECS", "10"),
        ],
    )?;
    let run_gc2p = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_FAST", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2q = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_WRITEOUT", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_GC2Q_FORCE", "1"),
        ],
    )?;
    let run_gc2t = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_TIMING", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2u = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_TEXLOAD", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2v = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_EDGE", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2y = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_CURED", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_GC2Y_SOAK_SECS", "20"),
        ],
    )?;
    let run_gc2z = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_FIX", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2aa = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_FREE", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_GC2AA_SOAK_SECS", "20"),
        ],
    )?;
    let run_gc2ab = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_LASTPIX", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2ac = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_RING", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
        ],
    )?;
    let run_gc2ad = variant_image_envs(
        root,
        "rp-gpubench",
        &[
            ("RP_GPUBENCH_GC2_NIGHT", "1"),
            ("RP_GPUBENCH_GC2_RENDERS", "3"),
            ("RP_GPUBENCH_GC2AD_SOAK_SECS", "20"),
        ],
    )?;
    let run_ft2 = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_TOGETHER2", "1"))?;
    let run_ft3 = variant_image(root, "rp-gpubench", ("RP_GPUBENCH_FT3", "1"))?;
    let run_ft2f = variant_image_with(
        root,
        "rp-gpubench",
        ("RP_GPUBENCH_TOGETHER2", "1"),
        Some(("RP_GPUBENCH_FT2_FAST", "1")),
    )?;
    // Run H without its watchdog (the emulator's check needs it back).
    let run_h = variant_image_with(
        root,
        "rp-gpubench",
        ("RP_GPUBENCH_FREEZE", "1"),
        Some(("RP_GPUBENCH_NO_WATCHDOG", "1")),
    )?;
    let modelled = device_has(root, "intel-2700g", "mbx-3d");
    // What the X51v showed that the fork models from rp-v0.29.0
    // (rp-gpubench, 2026-10-19): rectangular textures, a plane start
    // taking effect mid-frame, the video plane's RGB formats.
    let rect = device_has(root, "intel-2700g", "mbx-3d-rect");
    let repoint = device_has(root, "intel-2700g", "plane-repoint");
    let video_rgb = device_has(root, "intel-2700g", "video-rgb");
    let shot = scratch(root, "gpubench-screen.ppm")?;
    let guest = match emulator(root)? {
        Emulator::Wsl => to_wsl_path(&shot),
        Emulator::Native(_) => shot.display().to_string(),
    };
    // (name, image, wanted lines, whether they need the 3D model, key
    // presses: guest ms before the first, between them, how many). Runs C
    // and D draw no 3D; their presses end the watch steps early (a press
    // landing outside one is ignored by the image, which then waits its
    // cap), never more than the steps, so none reaches the loader.
    let runs: [GpubenchRun; 71] = [
        (
            "run A",
            &run_a,
            &[
                "G4a: rectangular textures",
                "G1 565 bilinear 1:1 / 16-bit",
                "G2 translucent flat / 32-bit",
                "G3 1 opaque 565 layer, one tile",
                "U4b SDRAM to the texture by DMA",
                "U3 and U4 wrote the same texture: true",
                "U5b strip",
                "  W1: ",
                "  W2: ",
                "  W3: ",
            ],
            true,
            (0, 0, 0),
        ),
        (
            "run B",
            &run_b,
            &["B: TEX0 words", "  B: 20 tries"],
            true,
            (0, 0, 0),
        ),
        (
            "run C",
            &run_c,
            &[
                "  W4: ",
                "the list converted RGB -> UYVY",
                "  W5: ",
                "the RGB list drawn",
                "  W6: ",
            ],
            false,
            (15_000, 8_000, 3),
        ),
        (
            "run D",
            &run_d,
            &["  try code 0", "  try code 8"],
            false,
            (6_000, 3_000, 9),
        ),
        (
            "run E",
            &run_e,
            &[
                "E1a agrees with the reference order: true",
                "E1c the texture in 2700G memory is right: true",
                "E1d fast reorder written by the CPU straight in",
                "E1e the texture in 2700G memory is right: true",
                "E3 PVRTC 2 bpp layer",
                "E4 a screen of text",
            ],
            false,
            (0, 0, 0),
        ),
        (
            "run F",
            &run_f,
            &[
                "  F0 RGB565 pixels",
                "  F1 4 bpp strips",
                "  F3a one 2D command a line",
                "  F3b one 2D command for all",
                "  F4 fill + a plain 2D copy a glyph",
                "  F5 fill",
                "  F6a full commands, batched",
                "  F6b surfaces once",
                "  F6c surfaces once",
                "  F7 the word cache made once: 259 words",
                "  F7 the line cache made once: 37 lines",
                "  F7a a copy a word (cached words): 259 operations",
                "  F7b a copy a line (cached lines): 37 operations",
                "  F7c one copy (the cached screen): 1 operations",
            ],
            false,
            (0, 0, 0),
        ),
        (
            "run G",
            &run_g,
            &["G5: the clocks", "RETROPANE-GPUBENCH: done"],
            false,
            (0, 0, 0),
        ),
        // Run GS (the clocks' soak): checked below.
        (
            "run GS",
            &run_gs,
            &["GS: the clocks' warm soak", "RETROPANE-GPUBENCH: done"],
            false,
            (0, 0, 0),
        ),
        // Run FT2 (2D and 3D at once by scan-out and placement): every
        // setting ran and finished; the model has no interference, so every
        // setting is clean.
        (
            "run FT2",
            &run_ft2,
            &[
                "  FT2 16-bit screen, copies apart: ",
                "  FT2 scan-out stopped, copies beside: ",
                "  FT2 32-bit screen, copies beside: ",
                "FT2: 6 of 6 settings clean",
                "RETROPANE-GPUBENCH: done",
            ],
            true,
            (0, 0, 0),
        ),
        // Run FT2 at 133.25 MHz: checked below (a release that doesn't
        // model Windows Mobile's clocks refuses).
        (
            "run FT2F",
            &run_ft2f,
            &[
                "FT2: the 2D engine and the 3D core at once",
                "RETROPANE-GPUBENCH: done",
            ],
            true,
            (0, 0, 0),
        ),
        // Run GC (the content map): checked below.
        (
            "run GC",
            &run_gc,
            &[
                "GC: which clock settings draw every content right",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2 (the content map by the owner's robustness rules, one
        // render a kind a setting): checked below.
        (
            "run GC2",
            &run_gc2,
            &[
                "GC2: run GC by the owner's robustness rules",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2S: GC2 with its label at offset 0 (C71, G17-G19) and
        // divide 1 to 133.25: runs to its end and returns (the pinned
        // release refuses the clocks as for GC2).
        (
            "run GC2S",
            &run_gc2s,
            &[
                "GC2: run GC by the owner's robustness rules",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2I: GC2S with the 16-bit kinds' targets inside the safe
        // region, the 32-bit ones outside, then the soak: runs to its end
        // and returns (the pinned release doesn't model Windows Mobile's
        // clock, so GC2 stops at its first check, as for GC2S; the whole
        // run is stepped on the wm-clocks build: docs/hw/2700g.md).
        (
            "run GC2I",
            &run_gc2i,
            &[
                "GC2: run GC with every 16-bit kind's target inside the safe region, the 32-bit kinds outside (GC2I), then a soak,",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2J: GC2I's references, then its soak (10 s a setting
        // here): runs to its end and returns (the pinned release doesn't
        // model Windows Mobile's clock; stepped on the wm-clocks build).
        (
            "run GC2J",
            &run_gc2j,
            &[
                "GC2: the 16-bit kinds' targets inside the safe region (GC2J: no map;",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2K: GC2J's patch isolated (20 s at 133.25 / 1 here): runs
        // to its end and returns (the pinned release doesn't model Windows
        // Mobile's clock; stepped on the wm-clocks build).
        (
            "run GC2K",
            &run_gc2k,
            &[
                "GC2: the 16-bit kinds' targets inside the safe region (GC2K: no map;",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2L: GC2K's patch mapped (30 s at 133.25 / 1 here): runs to
        // its end and returns (the pinned release doesn't model Windows
        // Mobile's clock; stepped on the wm-clocks build).
        (
            "run GC2L",
            &run_gc2l,
            &[
                "GC2: the 16-bit kinds' targets inside the safe region (GC2L: no map;",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2O: the tile orders (75 s at 133.25 / 1 here): runs to its
        // end and returns (the pinned release doesn't model Windows
        // Mobile's clock; stepped on the wm-clocks build).
        (
            "run GC2O",
            &run_gc2o,
            &[
                "GC2: the 16-bit kinds' targets inside the safe region (GC2O: no map;",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2P: GC2O made fast (3 rounds here), then FT3 in each tile
        // order: runs to its end and returns (the pinned release doesn't
        // model Windows Mobile's clock; stepped on the wm-clocks build).
        (
            "run GC2P",
            &run_gc2p,
            &[
                "GC2: the 16-bit kinds' targets inside the safe region (GC2P: no map;",
                "GC2P part 2: done",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2Q: FT3's copies queued, after or none; the write-out
        // search; divide 1 above 133.25: runs to its end and returns (the
        // pinned release doesn't model Windows Mobile's clock; stepped on
        // the wm-clocks build).
        (
            "run GC2Q",
            &run_gc2q,
            &["GC2Q parts A and B: done", "RETROPANE-GPUBENCH: done"],
            false,
            (0, 0, 0),
        ),
        // Run GC2AD (R36): the night soak, cut to 20 s (the emulator draws
        // exactly: every way right).
        (
            "run GC2AD",
            &run_gc2ad,
            &[
                "GC2AD (R36): at 149.50 / 1, reversed, 0 min",
                "wrong renders by way [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]",
                "GC2AD soak 149.50 / 1: controls at Windows Mobile's clock after: 14 of 14 right",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2AC: the ring cure against the full hidden layers at
        // 149.50 / 1 (the emulator draws exactly: every way right).
        (
            "run GC2AC",
            &run_gc2ac,
            &[
                "GC2AC 149.50 / 1: 500 rounds (3500 renders)",
                "wrong renders by way [0, 0, 0, 0, 0, 0, 0]",
                "GC2AC 149.50 / 1: controls at Windows Mobile's clock after: 7 of 7 right",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2AB: the hidden layers against the last-pixel fault at
        // 152.75 / 1 (the emulator draws exactly: no fallback).
        (
            "run GC2AB",
            &run_gc2ab,
            &[
                "GC2AB speed 99.67 / 2 (Windows Mobile's): 20 rounds (160 renders)",
                "GC2AB speed 149.50 / 1: 20 rounds (160 renders)",
                "GC2AB 152.75 / 1: 800 rounds (6400 renders)",
                "GC2AB 152.75 / 1: controls at Windows Mobile's clock after: 8 of 8 right",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2AA: the free corner cures, then the winner soaked (the
        // emulator draws exactly: every way right, the first preferred at
        // 149.50, a 20 s soak).
        (
            "run GC2AA",
            &run_gc2aa,
            &[
                "GC2AA 139.75 / 1: 1000 rounds (4000 renders)",
                "GC2AA 149.50 / 1: right in every render: one hidden layer, columns reversed",
                "GC2AA soak: one hidden layer, columns reversed at 149.50 / 1 over every 16-bit kind and GC2K's worst, 0 min",
                "GC2AA soak 149.50 / 1: controls at Windows Mobile's clock after: 7 of 7 right",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2Z: the narrower texture shifts and the stronger corner
        // cures: every way's references held, the run returns.
        (
            "run GC2Z",
            &run_gc2z,
            &[
                "GC2Z 1 149.50 / 1: 30 rounds (360 renders)",
                "GC2Z 2 139.75 / 1: 1000 rounds (4000 renders)",
                "GC2Z 2 139.75 / 1: controls at Windows Mobile's clock after: 4 of 4 right",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2Y: both cures over the climb, then the soak at the highest
        // clean clock (the emulator draws exactly: 149.50, a 20 s soak).
        (
            "run GC2Y",
            &run_gc2y,
            &[
                "GC2Y climb 133.25 / 1: the cured seven all right",
                "GC2Y climb 149.50 / 1: the cured seven all right",
                "GC2Y soak: the cured seven at 149.50 / 1 (the highest clock with all right) for 0 min",
                "GC2Y soak 149.50 / 1: controls at Windows Mobile's clock after: 7 of 7 right",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2V: FT2's quad edge isolated at 146.25 / 1, the hidden
        // layer at 133.25 / 1: every way's references held, the run returns
        // (wm-clocks build).
        (
            "run GC2V",
            &run_gc2v,
            &[
                "GC2V A 143.00 / 1: 10 rounds (90 renders)",
                "GC2V A 146.25 / 1: 10 rounds (90 renders)",
                "GC2V B 133.25 / 1: 200 rounds (600 renders)",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2U: FT2 and GC2K's worst with a 32-bit, a 16-bit and a PVRTC
        // texture and with busy tiles, at every climb step: every way's
        // references held and the run returns (wm-clocks build).
        (
            "run GC2U",
            &run_gc2u,
            &[
                "GC2U: divide 1 at 149.50 / 1, PVRTC texture",
                "PVRTC texture 149.50 / 1: 30 rounds (60 renders)",
                "32-bit texture 133.25 / 1: 100 rounds (400 renders)",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2T: the local memory's timing loosened on FT3, GC2K's worst
        // and the climb: every timing written, held and put back, and the
        // run returns (stepped on the wm-clocks build).
        (
            "run GC2T",
            &run_gc2t,
            &[
                "GC2T 1 133.25 / 1 all +1 (LMTIM 0xa55d3, reads 0xa55d3)",
                "GC2T 2: GC2K's worst at 133.25 / 1, all +1 (LMTIM Some(a55d3))",
                "GC2T 3: divide 1 at 149.50 / 1, all +1",
                "GC2Q parts A and B: done",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2D (idea (d), Gouraud as a texture, by GC2's rules, one
        // render a kind a setting): checked below.
        (
            "run GC2D",
            &run_gc2d,
            &[
                "GC2: idea (d), Gouraud drawn as a bilinear texture",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Runs GC2F (idea (f): eight render tables) and GC2H (past 133
        // MHz), one render a kind a setting: checked below.
        (
            "run GC2F",
            &run_gc2f,
            &[
                "GC2: idea (f), Gouraud under eight render tables",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        (
            "run GC2H",
            &run_gc2h,
            &["GC2: run GC past 133 MHz", "RETROPANE-GPUBENCH: done"],
            false,
            (0, 0, 0),
        ),
        // Run GC2R (divide 1's registers and ways in), one render a way:
        // checked below.
        (
            "run GC2R",
            &run_gc2r,
            &["GC2R: what divide 1 changes", "RETROPANE-GPUBENCH: done"],
            false,
            (0, 0, 0),
        ),
        // Run GC2H2 (GC2H to 169 MHz): checked below.
        (
            "run GC2H2",
            &run_gc2h2,
            &["GC2: run GC to 169 MHz", "RETROPANE-GPUBENCH: done"],
            false,
            (0, 0, 0),
        ),
        // Run GC2W (the write-out downsample at divide 1): checked below.
        (
            "run GC2W",
            &run_gc2w,
            &[
                "GC2: the write-out downsample at divide 1",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC3 (the memory interface and the display's refresh window,
        // run FT3 under each): checked below.
        (
            "run GC3",
            &run_gc3,
            &["GC3 FT3 under ", "RETROPANE-GPUBENCH: done"],
            false,
            (0, 0, 0),
        ),
        // Run FT3 (2D and 3D at once in option C's layout, three clocks):
        // checked below.
        (
            "run FT3",
            &run_ft3,
            &[
                "FT3: the 2D engine and the 3D core at once",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GC2M (divide 1 with M24CLK as Dell's driver sets it):
        // checked below.
        (
            "run GC2M",
            &run_gc2m,
            &["GC2: divide 1 with M24CLK", "RETROPANE-GPUBENCH: done"],
            false,
            (0, 0, 0),
        ),
        // Run GC2X (MBXCLK's reserved bits at divide 1): checked below.
        (
            "run GC2X",
            &run_gc2x,
            &[
                "GC2X: every undocumented bit near the cores",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run GV (the local memory's clock, V1): checked below.
        (
            "run GV",
            &run_gv,
            &[
                "GV: V1, the 2700G's local memory clock",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run R (the dark display's restart steps): every step's registers
        // and cards, and back at the loader.
        (
            "run R",
            &run_r,
            &[
                "R: as found, before anything is touched",
                "R0: adopted",
                "R1: the sync generator",
                "R2: the display PLL",
                "R3: the whole display set-up",
                "R4: the system clock",
                "R: after R4",
                "RETROPANE-GPUBENCH: done (run R)",
            ],
            false,
            (0, 0, 0),
        ),
        // Run S (a read-only register snapshot): every line, back at the
        // loader.
        (
            "run S",
            &run_s,
            &[
                "S: every named register below the MBX block",
                " SYSCFG 0x",
                " CSC05 0x",
                "the picture at offset 0, mid-screen",
                "RETROPANE-GPUBENCH: done (run S)",
            ],
            false,
            (0, 0, 0),
        ),
        // Run U (the underrun reproduced and cleared): every hold logged
        // (the emulator has no underrun), back at the loader.
        (
            "run U",
            &run_u,
            &[
                "U: as found",
                "U1: 1 ms on the reference clock",
                "U4: 30 ms on the reference clock",
                "U5: CL3 at 99.67 MHz for 100 ms",
                "U6: G4's lock traces",
                "U: at the end",
                "RETROPANE-GPUBENCH: done (run U)",
            ],
            false,
            (0, 0, 0),
        ),
        // Run V (which work at which clock starves the display): every
        // clock's line (no underrun in the emulator), put back.
        (
            "run V",
            &run_v,
            &["RETROPANE-GPUBENCH: done"],
            false,
            (0, 0, 0),
        ),
        // Run X (the underrun flag cleared, nothing else): its lines.
        (
            "run X",
            &run_x,
            &[
                "X: as found",
                "X: the flag cleared",
                "X: after the card",
                "X1: the sync generator off and on",
                "X3: the whole display set-up",
                "RETROPANE-GPUBENCH: done (run X)",
            ],
            false,
            (0, 0, 0),
        ),
        // Run L (the recovery ladder): the emulator's DSIG reads 0, so
        // every rung runs and none recovers it.
        (
            "run L",
            &run_l,
            &[
                "L: as found",
                "L1: the pixel clock off and on",
                "L4: the local memory to self-refresh and back",
                "L6: the memory clock off for 1 ms",
                "RETROPANE-GPUBENCH: done (run L)",
            ],
            false,
            (0, 0, 0),
        ),
        // Run Z (the reset and the rebuild): every phase and both dumps.
        (
            "run Z",
            &run_z,
            &[
                "Z: as found",
                "Z1: SYSRST next",
                "Z1b: SYSCFG 0xffffff0c",
                "Z2: the clocks next",
                "Z3: the clocks and memory read back",
                "Z5: the display",
                "Z7: every register after the rebuild",
                "RETROPANE-GPUBENCH: done (run Z)",
            ],
            false,
            (0, 0, 0),
        ),
        // Run H (the card log against a frozen bus): the emulator answers
        // the read, so the run comes back; every line is on the card
        // (checked for every run).
        (
            "run H",
            &run_h,
            &[
                "H: line 1 of 40 before the freeze",
                "H: line 40 of 40 before the freeze",
                "H: reading 0x12c next",
                "H: the read returned",
                "H: the watchdog left unarmed",
            ],
            false,
            (0, 0, 0),
        ),
        // Run Y (the rescue measured): checked below (refused cleanly
        // on a fork that doesn't start as Windows Mobile leaves the clocks).
        (
            "run Y",
            &run_y,
            &["Y: the rescue measured", "RETROPANE-GPUBENCH: done"],
            false,
            (0, 0, 0),
        ),
        // Run Q (what ends the slow state): refused cleanly on a fork that
        // doesn't start the clocks as Windows Mobile leaves them, else every
        // step measured.
        (
            "run Q",
            &run_q,
            &["RETROPANE-GPUBENCH: run Q", "RETROPANE-GPUBENCH: done"],
            false,
            (0, 0, 0),
        ),
        // Run N (the start-up watched): every stage read, the three cards.
        (
            "run N",
            &run_n,
            &[
                "N0 the 2700G handed out: DINTRS",
                "N6 the screen buffer shown (black)",
                "N7 card 1",
                "N8 card 2",
                "N11 card 3",
                "N15 after the lock trace 108.33 -> 99.67 MHz",
                "N: done",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run P (the 3D core's target formats): every code tried and
        // decoded. The fork writes 32-bit pixels for every code but 1 and
        // 3 (the others aren't measured yet: emulator.md), so all 52 read
        // back in place and none hangs; the device's answer is the run's.
        (
            "run P",
            &run_p,
            &[
                "P3: render targets",
                "  P3 try 2 0x650 0x00000001",
                "    565 rows (the picture's stride): 1024/1024 pixels in place",
                "    1555 rows (the picture's stride): 1024/1024 pixels in place",
                "    8888 rows (the picture's stride): 1024/1024 pixels in place",
                "  P3 try 52 0x650 0x80000006",
                "P3: 52 tries, 52 decoded (9 in 10 pixels in place at least), 0 hung, 0 wrote nothing",
                "everything given back: true",
                "RETROPANE-GPUBENCH: done",
            ],
            true,
            (0, 0, 0),
        ),
        // Run P2 (run P's follow-up): every part runs. The fork writes
        // codes 1 and 3 as RGB565 and ARGB1555 and the rest as ARGB8888,
        // undithered (emulator.md), so both shapes decode in every code
        // and RGB565's ramp truncates; the device's answers are the run's.
        (
            "run P2",
            &run_p2,
            &[
                "P2a: target shapes",
                "  P2a 16 x 16: 8 of 8 decoded",
                "  P2a 32 x 16: 8 of 8 decoded",
                "  P2b code 1",
                "    red (5 bits): 0 rounded up, 0 other, 0 of 64 places not a threshold",
                "  P2b code 15",
                "  P2c code 6: as drawn",
                "  P2d: 192 tries, 0 hung, 0 not plain ARGB8888 rows",
                "P2: 232 tries, 0 hung",
                "everything given back: true",
                "RETROPANE-GPUBENCH: done",
            ],
            true,
            (0, 0, 0),
        ),
        // Run P2W (watched): its four pictures, each to its 30 s cap (no
        // presses), back at the loader.
        (
            "run P2W",
            &run_p2w,
            &[
                "  P2W step 1: 24-BIT (0x650 4",
                "    the black bar after the render: 0 of 19200 pixels not black",
                "    planes: GSCTRL 0x",
                "  P2W step 4: 32-BIT (0x650 6",
                "P2W: done",
                "everything given back: true",
                "RETROPANE-GPUBENCH: done",
            ],
            true,
            (0, 0, 0),
        ),
        // Run W (the fingerprint): every register line; the release fork
        // doesn't start the clocks as Windows Mobile leaves them, so it
        // says so and measures nothing (the wm-clocks build measures).
        (
            "run W",
            &run_w,
            &[
                "W PXA CLOCKS: CCCR 0x",
                "W PXA MEMORY",
                "W CPU: ID 0x69054",
                "W: every named 2700G register",
                " SYSCFG 0x",
                "everything given back: true",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // Run T (the display's fetch, 0x644's mapping): both parts run (the
        // release fork measures nothing at its own clock), every row of
        // the three mappings logged.
        (
            "run T",
            &run_t,
            &[
                "T1: the display's fetch",
                "T2: 0x644 bits 0 and 1",
                "  T2 0x644 = 1",
                "  T2 0x644 = 3",
                "    row 31: changed 0x",
                "everything given back: true",
                "RETROPANE-GPUBENCH: done",
            ],
            true,
            (0, 0, 0),
        ),
        // Run T3 (the downsample's cost): every setting timed (the fork's
        // times say nothing about the device's).
        (
            "run T3",
            &run_t3,
            &[
                "  T3 0x644 = 0: ",
                "  T3 0x644 = 3: ",
                "T3: plain ",
                "everything given back: true",
                "RETROPANE-GPUBENCH: done",
            ],
            true,
            (0, 0, 0),
        ),
        // Run P2X (the specks): every render checked; the fork renders
        // the 32-bit and RGB565 ones right and every render alike (its
        // 24-bit is written as 32-bit: emulator.md, so only its line).
        (
            "run P2X",
            &run_p2x,
            &[
                "  P2X code 6 render 1 (true): bars: 0 white, 0 never written, 0 other wrong; 0 against render 1",
                "  P2X code 6 render 5 (true): bars: 0 white, 0 never written, 0 other wrong; 0 against render 1",
                "  P2X code 4 render 5 ",
                "  P2X code 1 render 5 (true): bars: 0 white, 0 never written, 0 other wrong; 0 against render 1",
                "P2X: done",
                "everything given back: true",
                "RETROPANE-GPUBENCH: done",
            ],
            true,
            (0, 0, 0),
        ),
        // Run P2Y (what the errors need): every part; the fork reads and
        // renders without errors.
        (
            "run P2Y",
            &run_p2y,
            &[
                "  Y1 pass 1: 0 of 307200 words read differently twice in a row",
                "  Y1 pass 2: 0 of 307200 words read differently twice in a row",
                "  Y2 the screen at 32 bits:",
                "  Y2 the screen stopped:",
                "  P2X code 6 render 5 (true): bars: 0 white, 0 never written, 0 other wrong; 0 against render 1",
                "P2Y: done",
                "everything given back: true",
                "RETROPANE-GPUBENCH: done",
            ],
            true,
            (0, 0, 0),
        ),
        (
            // The model draws every render right, so the controls find
            // nothing here either: the check is that every render runs,
            // is checked and counted, and the run comes back.
            "run P2L",
            &run_p2l,
            &[
                "  P2L first render (true), the screen at 32 bits: bars: 0 white, 0 never written, 0 other wrong",
                "  P2L control before (the screen at 32 bits): 5 renders (0 unfinished)",
                "  the screen: P2W's picture dithered to RGB565 (true), scanned out at 16 bits",
                "  P2L so far, the screen at 16 bits: 500 renders (0 unfinished)",
                "  P2L the screen at 16 bits: 1000 renders (0 unfinished)",
                "  the reference: changed at 0 of 40 checks",
                "  P2L control after (the screen at 32 bits): 5 renders (0 unfinished)",
                "P2L: 0 of 1000 renders under the 16-bit screen had a wrong pixel (0 pixels); the controls at 32 bits: 0 of 5 before, 0 of 5 after",
                "P2L: done",
                "everything given back: true",
                "RETROPANE-GPUBENCH: done",
            ],
            true,
            (0, 0, 0),
        ),
        (
            // FF1 needs only every fill taken (whether the model's FIFO
            // ever reads full is the device's question); FF2's drop is
            // the renderer's, on any engine.
            "run FF",
            &run_ff,
            &[
                "  FF1: 300 fills queued in ",
                "0 refused (FIFO full 100 ms)",
                "  ok  B dropped: counted once, apart from stalls",
                "  ok  the dropped frame isn't shown (A still on screen)",
                "  ok  B after the drop is B (within 2 a channel, the host test's bound)",
                "FF: FF2 11 checks, 0 BAD",
                "FF: done",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        (
            // C43's real stall: what the engine does with its clock off is
            // the device's question (the fork may not stop it, and its FIFO
            // never fills: FS3 then doesn't judge the drop); every step must
            // run, put the clock back and pass what it does judge.
            "run FS",
            &run_fs,
            &[
                "FS1: 20 full-screen fills",
                "  ok  FS1: the queued work finished once the clock was back",
                "FS2: not repeated",
                "  ok  FS3: the engine caught up (the fence landed)",
                "  ok  FS3: A still on screen after the stall",
                "  ok  FS3: B after the stall is B drawn after A (the screens' hashes)",
                " checks, 0 BAD; MBXCLK at the end ",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        (
            // The gate's P8: the engines at once; both results as alone.
            "run FT",
            &run_ft,
            &[
                "  FT alone: 3D Some(",
                "0 words of 153600 differ from the first",
                "FT together: 0 words of 153600 differ from the render alone",
                "differing pixels 0 in the textured quad, 0 in the Gouraud background only",
                "the render as alone: true; the copies as alone: true",
                "FT: both right together",
                "RETROPANE-GPUBENCH: done",
            ],
            true,
            (0, 0, 0),
        ),
        (
            // The gate's P18: the MBX status bits around 2D and 3D jobs,
            // the enables put back.
            "run FI",
            &run_fi,
            &[
                "  FI as found: status 0x",
                "  FI enables 0 (0x00000000): idle 0x",
                "  FI enables all (0xffffffff): idle 0x",
                "3D started true",
                "FI: enables put back",
                "RETROPANE-GPUBENCH: done",
            ],
            true,
            (0, 0, 0),
        ),
        (
            // The gate's T3: 2D commands into the command window by DMA.
            "run FD",
            &run_fd,
            &[
                "  FD 0 CPU: idle first true, sent true",
                "  FD 2 DMA: idle first true",
                "the picture as the CPU's: true",
                "FD: both ways drew the same",
                "RETROPANE-GPUBENCH: done",
            ],
            true,
            (0, 0, 0),
        ),
        // The backlog's P12 and V17: every burst and width moves the copy
        // right (the emulator's DMA ignores the burst), every phase read.
        (
            "run BX",
            &run_bx,
            &[
                "  BX P12 burst 8 width 0: 960 KiB in",
                "  BX P12 burst 32 width word: 960 KiB in",
                "the copy right: true",
                "  BX V17 a DMA into local memory each frame:",
                "BX: done",
                "everything given back: true",
                "RETROPANE-GPUBENCH: done",
            ],
            false,
            (0, 0, 0),
        ),
        // The status bits run FI left (P18): every phase is read and the
        // memory given back; what the bits do is the device's.
        (
            "run FI2",
            &run_fi2,
            &[
                "FI2: MINTSTAT, SINTSTAT",
                "  FI2 idle 20 ms:",
                "  FI2 a full 2D FIFO draining:",
                "  FI2 2D: 420 words sent true, done true",
                "  FI2 a 3D render:",
                "FI2: done",
                "everything given back: true",
                "RETROPANE-GPUBENCH: done",
            ],
            true,
            (0, 0, 0),
        ),
        // Run CH's C71 diagnostic: six W5 sequences, each held to a fresh
        // render (the emulator's are exact: the device's are the finding).
        (
            "run C71",
            &run_c71,
            &[
                "  CH C71 the background object run 3:",
                "  CH C71 a flat quad in slot 0 run 3:",
                "  CH C71 the same scene rendered whole 20 times:",
                "  CH C71 the same picture read 20 times, no render between: 0 reads differ",
                "  CH C71 textured cards shrunk 2:1 (64 x 64), point sampling: rendered whole 20 times",
                "  CH C71 flat cards 200 x 260, the display's fetch paused: rendered whole 20 times",
                "  CH C71 flat cards 200 x 260, the display reading offset 0: rendered whole 20 times",
                "  CH C71: done",
                "everything given back: true",
                "RETROPANE-GPUBENCH: done",
            ],
            true,
            (0, 0, 0),
        ),
        // Run G21 on C71's scenes: the safe screen, A against C.
        ("run G21", &run_g21, &g21_wants, true, (0, 0, 0)),
        // The 3D core's limits (P14): every measurement finishes and the
        // memory is given back; the times are the device's.
        (
            "run LIM",
            &run_lim,
            &[
                "LIM: the 3D core's limits",
                "  LIM 63 textured 64 x 64 quads:",
                "  LIM 63 translucent full-screen layers:",
                "  LIM 32 quads, a texture each:",
                "  LIM a 1024 x 1024 texture over the screen",
                "LIM: done",
                "everything given back: true",
                "RETROPANE-GPUBENCH: done",
            ],
            true,
            (0, 0, 0),
        ),
        // The compositor harness (docs/design/compositor-harness.md): every
        // sequence runs, both 3D ways end on one picture, and everything is
        // given back; the emulator's times mean nothing (emulator.md).
        (
            "run CH",
            &run_ch,
            &[
                "CH: the compositor harness",
                "  CH W1 pan (VSCADR a frame): 160 frames",
                "  CH W2 row by 2D BY DMA: 7 rows (7 done)",
                "  CH W2 row by DMA PIXELS: 7 rows (7 done)",
                "  CH W4 tap by 2D BY DMA:",
                "  CH plane steps done: 10 of 18",
                "  CH W5 turns and pipeline end on the same picture: true",
                "  CH W6 turns and pipeline end on the same picture: true",
                "  CH 3D steps done: 18 of 18",
                "  CH W5 turns, every tile every frame: the last picture against a fresh full render",
                "CH: done",
                "everything given back: true",
                "RETROPANE-GPUBENCH: done",
            ],
            true,
            (0, 0, 0),
        ),
    ];
    // One card for every run, with someone else's file on it: each run
    // keeps its log there as it goes (the loader's card log), the next number each
    // time, and touches nothing outside rplog/ (the card is shared).
    let other = b"another session's file: must stay as it is".to_vec();
    let card = rp_fat::format::build(&rp_fat::format::Image {
        total_sectors: 64 * 1024,
        sectors_per_cluster: 8,
        files: &[rp_fat::format::Entry {
            name: "other.txt",
            data: &other,
        }],
        blocked_clusters: &[],
        ballast: 0,
    });
    let sd = scratch(root, "sd-gpubench.img")?;
    std::fs::write(&sd, &card).map_err(|e| format!("{}: {e}", sd.display()))?;
    // `RETROPANE_GPUBENCH_RUNS="run E,run FT"`: only those runs (a new
    // run's check without the other 29, ~25 minutes); the card logs are
    // numbered by the runs that ran.
    let only: Option<Vec<String>> = std::env::var("RETROPANE_GPUBENCH_RUNS")
        .ok()
        .map(|v| v.split(',').map(|n| n.trim().to_string()).collect());
    let mut number = 0u32;
    for (name, bin, want, needs_model, (first, every, presses)) in runs {
        if only.as_ref().is_some_and(|o| !o.iter().any(|n| n == name)) {
            continue;
        }
        number += 1;
        let payload = std::fs::read(bin).map_err(|e| format!("{}: {e}", bin.display()))?;
        // Named, so each run's card log is rplog/gpubench-NNNN.txt.
        let mut input = rp_loadproto::control_frame("name gpubench");
        input.extend(rp_loadproto::frame(&payload));
        let keys = || -> Result<(), String> {
            if presses == 0 {
                return Ok(());
            }
            let mut mon = crate::screen::Monitor::connect(monitor_port())?;
            guest_hold(&mut mon, first, "gpubench: before the first key")?;
            for k in 0..presses {
                if k > 0 {
                    guest_hold(&mut mon, every, "gpubench: between keys")?;
                }
                // Run C's W4 is up before the first key: VSCADR to the
                // yellow half at line 320 and back at 560 every frame.
                if name == "run C" && k == 0 && repoint {
                    gpubench_until(&mut mon, &guest, &shot, 40, "w4", &|img| {
                        gpubench_expect(img, "W4 above the band", 240, 200, (0, 0, 1))?;
                        gpubench_expect(img, "W4 band", 240, 440, (1, 1, 0))?;
                        gpubench_expect(img, "W4 below the band", 240, 600, (0, 0, 1))
                    })?;
                }
                // Run D's code 7 is up before the eighth key: the RGB565
                // card on the video plane.
                if name == "run D" && k == 7 && video_rgb {
                    let bars = [
                        (1, 1, 1),
                        (1, 1, 0),
                        (0, 1, 1),
                        (0, 1, 0),
                        (1, 0, 1),
                        (1, 0, 0),
                        (0, 0, 1),
                        (0, 0, 0),
                    ];
                    gpubench_until(&mut mon, &guest, &shot, 12, "code7", &|img| {
                        for (i, want) in bars.into_iter().enumerate() {
                            gpubench_expect(img, "code 7 bar", 30 + 60 * i as u32, 200, want)?;
                        }
                        gpubench_expect(img, "code 7 ramp's left", 4, 400, (0, 0, 0))?;
                        gpubench_expect(img, "code 7 ramp's right", 476, 400, (1, 1, 1))
                    })?;
                }
                mon.query("sendkey ret 200")?;
            }
            Ok(())
        };
        let output = boot(
            root,
            &Boot {
                image: &loader,
                marker: LOADER_MARKER,
                screenshot: None,
                sd_image: Some(&sd),
                steps: &[],
                extra: &[],
                // The loader's start of the image: the steps below run
                // while it is on screen (its own marker comes at its end).
                input: Some((&input, "RPL OK sha256")),
                during: Some(&keys),
                then: Some("RPL END"),
                timeout: Duration::from_secs(900),
            },
        )?;
        let log = output
            .rsplit_once("RPL LOG")
            .map(|(_, l)| l)
            .ok_or_else(|| format!("gpubench {name}: never came back to the loader"))?;
        if crashed(log) || log.contains("did not return") {
            return Err(format!("gpubench {name}: didn't return cleanly:{log}"));
        }
        gpubench_card_log(&sd, name, number, log)?;
        // Run R returns before any 2700G memory is taken: no pool line.
        let always: &[&str] = if matches!(
            name,
            "run R" | "run S" | "run U" | "run X" | "run L" | "run Z"
        ) {
            &["RETROPANE-GPUBENCH: done"]
        } else {
            &["everything given back: true", "RETROPANE-GPUBENCH: done"]
        };
        // Without the 3D model every render is refused (`NoCore`): only
        // the plumbing is checked for runs A and B.
        let checked: &[&str] = if modelled || !needs_model { want } else { &[] };
        for w in always.iter().chain(checked) {
            if !log.contains(w) {
                return Err(format!("gpubench {name}: the run log has no `{w}`:{log}"));
            }
        }
        // Run FI: the 3D render's done status carries bits 2 and 3 on the
        // X51v (0x0c or 0x4c); bit 3 is modelled since rp-v0.30.0.
        if name == "run FI" {
            let done = fi_render_done(log);
            if done.len() != 3 || done.iter().any(|d| d & 0xC != 0xC) {
                return Err(format!(
                    "gpubench run FI: render done words {done:x?}, want bits 2 and 3 in all three:{log}"
                ));
            }
        }
        // Run G steps the clocks only from Windows Mobile's; a fork whose
        // core PLL reads 0 (rp-v0.29.0 and older: docs/hw/emulator.md) gets
        // the refusal, and one that starts as Windows Mobile leaves the
        // chip must trace the lock bit, run both passes with every step
        // GOOD twice (the models don't fail at any clock) and every control
        // GOOD, and put everything back.
        // Run GS soaks only from Windows Mobile's clock, as run G; stepping,
        // both settings must run to their end with every round and control
        // GOOD (the models don't fail at any clock), a minute's line each,
        // and everything put back. `DSIG` isn't modelled (reads 0:
        // emulator.md), so whether the label is sent is the device's.
        if name == "run GS" {
            let refused =
                log.contains("not Windows Mobile's clock and bus timing: nothing changed");
            let soaked = [
                "  the 16-bit label shown: DSIG 0x",
                "  soak 1: 99.67 MHz CL2, cores / 1: lock Some(",
                "  GS 99.67 MHz CL2, cores / 1, minute 1: ",
                "  soak 1 at 99.67 MHz CL2, cores / 1: ",
                "    control at 99.67 MHz after soak 1: GOOD",
                "  soak 2: 133.25 MHz CL2, cores / 1: lock Some(",
                "  GS 133.25 MHz CL2, cores / 1, minute 2: ",
                "  soak 2 at 133.25 MHz CL2, cores / 1: ",
                "    control at 99.67 MHz after soak 2: GOOD",
                "  put back: COREPLL 0xbb1",
            ]
            .iter()
            .all(|w| log.contains(w))
                && log.matches(", 0 BAD | worst 3D").count() == 6
                && log.matches(" | GOOD").count() == 4
                && !log.contains("| BAD")
                && !log.contains("STOPPED");
            if !(refused && log.contains("as found: COREPLL 0x0,") || soaked) {
                return Err(format!(
                    "gpubench run GS: neither refused cleanly nor soaked both settings:{log}"
                ));
            }
        }
        // Run GV steps only from Windows Mobile's clock, as runs G and GS;
        // stepping, all five settings GOOD with a control GOOD after each
        // (the models' times don't follow the clock, so only that every
        // setting ran right and the clock went back), and put back.
        if name == "run FT2F" {
            let refused =
                log.contains("not Windows Mobile's clock and bus timing: nothing changed");
            let ran = log.contains("  FT2 at 133.25 MHz CL2, cores / 1: lock Some(")
                && log.contains("FT2: 6 of 6 settings clean")
                && log.contains("  put back: COREPLL 0xbb1");
            if !(refused || ran) {
                return Err(format!(
                    "gpubench run FT2F: neither refused cleanly nor ran every setting at 133.25 MHz:
{log}"
                ));
            }
        }
        // Run GC refuses where Windows Mobile's clocks aren't modelled;
        // stepping, every reference repeatable and every kind right at
        // every setting (the models draw the same at any clock).
        if name == "run GC" {
            let refused =
                log.contains("not Windows Mobile's clock and bus timing: nothing changed");
            let mapped = log.matches("  GC reference ").count() == 7
                && !log.contains("isn't repeatable")
                && log
                    .matches("7 of 7 kinds right; the control at Windows Mobile's clock after it: 7 of 7 right")
                    .count()
                    == 5
                && !log.contains("| WRONG")
                && log.contains("  put back: COREPLL 0xbb1");
            if !(refused || mapped) {
                return Err(format!(
                    "gpubench run GC: neither refused cleanly nor mapped every setting:\n{log}"
                ));
            }
        }
        // Run GC2 refuses where Windows Mobile's clocks aren't modelled;
        // stepping, every reference proven, every kind right with
        // everything held at every setting both ways (the models draw the
        // same at any clock), the references the same at the end, and
        // each held to the host model of the 3D core.
        if name == "run GC2" {
            // Every reference the host model's, but the dithered one: the
            // wm-clocks fork build (rp-v0.29.0-1) has no 3D dither model.
            let modelled = log
                .lines()
                .filter(|l| {
                    l.starts_with("  GC2 reference ")
                        && !l.contains(" at the end: ")
                        && !l.starts_with("  GC2 reference RGB565 dithered Gouraud: ")
                })
                .filter(|l| l.ends_with("the host model's pixels: EQUAL in every band"))
                .count();
            let refused =
                log.contains("not Windows Mobile's clock and bus timing: nothing changed");
            let mapped = log.matches("  GC2 reference ").count() == 18
                && log.contains("  references: every one repeatable")
                && modelled == 8
                && log
                    .matches(
                        "9 of 9 kinds right with everything held; their controls: 9 of 9 right",
                    )
                    .count()
                    == 24
                && !log.contains("WRONG")
                && !log.contains("the clock didn't take")
                && log.contains("  GC2 references at the end: 9 of 9 as at the start")
                && log.contains("  put back: COREPLL 0xbb1");
            if !(refused || mapped) {
                return Err(format!(
                    "gpubench run GC2: neither refused cleanly nor mapped every setting:
{log}"
                ));
            }
        }
        // Run GC2D refuses where Windows Mobile's clocks aren't modelled;
        // stepping, as run GC2: every reference proven, every kind right
        // with everything held at every setting both ways, the references
        // the same at the end, and the texture way measured against
        // Gouraud for both targets.
        if name == "run GC2D" {
            let refused =
                log.contains("not Windows Mobile's clock and bus timing: nothing changed");
            let mapped = log.matches("  GC2 reference ").count() == 8
                && log.contains("  references: every one repeatable")
                && log.matches("  (d) at Windows Mobile's clock: ").count() == 2
                && log
                    .matches(
                        "4 of 4 kinds right with everything held; their controls: 4 of 4 right",
                    )
                    .count()
                    == 14
                && !log.contains("WRONG")
                && !log.contains("the clock didn't take")
                && log.contains("  GC2 references at the end: 4 of 4 as at the start")
                && log.contains("  put back: COREPLL 0xbb1");
            if !(refused || mapped) {
                return Err(format!(
                    "gpubench run GC2D: neither refused cleanly nor mapped every setting:
{log}"
                ));
            }
        }
        // GC2F and GC2H refuse where Windows Mobile's clocks aren't
        // modelled; stepping, as run GC2: every reference proven and the
        // same at the end, every kind right with everything held at every
        // setting both ways (GC2F: under each of its eight tables, each
        // compared with ours at Windows Mobile's clock).
        for (run, refs, line, settings, ends, tables) in [
            (
                "run GC2F",
                48,
                "3 of 3 kinds right with everything held; their controls: 3 of 3 right",
                48,
                ("  GC2 references at the end: 3 of 3 as at the start", 8),
                7 * 3,
            ),
            (
                "run GC2M",
                54,
                "9 of 9 kinds right with everything held; their controls: 9 of 9 right",
                24,
                ("  GC2 references at the end: 9 of 9 as at the start", 3),
                2 * 9,
            ),
            (
                "run GC2W",
                72,
                "9 of 9 kinds right with everything held; their controls: 9 of 9 right",
                24,
                ("  GC2 references at the end: 9 of 9 as at the start", 4),
                3 * 9,
            ),
            (
                "run GC2H2",
                18,
                "9 of 9 kinds right with everything held; their controls: 9 of 9 right",
                18,
                ("  GC2 references at the end: 9 of 9 as at the start", 1),
                0,
            ),
            (
                "run GC2H",
                18,
                "9 of 9 kinds right with everything held; their controls: 9 of 9 right",
                12,
                ("  GC2 references at the end: 9 of 9 as at the start", 1),
                0,
            ),
        ] {
            if name != run {
                continue;
            }
            let refused =
                log.contains("not Windows Mobile's clock and bus timing: nothing changed");
            let mapped = log.matches("  GC2 reference ").count() == refs
                && log.matches("  references: every one repeatable").count() == ends.1
                && log.matches(line).count() == settings
                && log.matches(ends.0).count() == ends.1
                && log
                    .matches(" reference at Windows Mobile's clock differs from ours in ")
                    .count()
                    == tables
                && !log.contains("WRONG")
                && !log.contains("the clock didn't take")
                && log.contains("  put back: COREPLL 0xbb1");
            if !(refused || mapped) {
                return Err(format!(
                    "gpubench {run}: neither refused cleanly nor mapped every setting:
{log}"
                ));
            }
        }
        // GC2R refuses where Windows Mobile's clocks aren't modelled;
        // stepping, the references repeatable, the registers compared
        // (the models change none but the clocks'), and every way into
        // 58.50 / 1 right on both kinds (the models draw the same at any
        // clock) with every control right.
        if name == "run GC2R" {
            let refused =
                log.contains("not Windows Mobile's clock and bus timing: nothing changed");
            let ways = log
                .lines()
                .filter(|l| l.starts_with("  GC2R ") && l.contains(" | "))
                .collect::<Vec<_>>();
            let mapped = log.matches("  GC2R reference ").count() == 2
                && !log.contains("isn't repeatable")
                && log.contains("  GC2R registers: ")
                && ways.len() == 10
                && ways.iter().all(|l| {
                    l.contains(": B wrong in 0 of 1 (the clock not taken 0)")
                        && l.ends_with("| A wrong in 0 of 1")
                })
                && log.contains("  put back: COREPLL 0xbb1");
            if !(refused || mapped) {
                return Err(format!(
                    "gpubench run GC2R: neither refused cleanly nor tried every way:
{log}"
                ));
            }
        }
        // FT3 (alone, and four times in GC3): every setting run and clean
        // (the models have no contention), the target at 0. With Windows
        // Mobile's clock modelled, three clocks (12 settings) and the clock
        // put back after each step; without it, the as-found clock only (4),
        // never nothing. GC3 also puts each memory setting back.
        if name == "run FT3" || name == "run GC3" {
            let stepped = !log.contains("the clock steps skipped, the clock as found only");
            let (settings, put_backs) = if stepped { (12, 2) } else { (4, 0) };
            let runs = if name == "run GC3" { 4 } else { 1 };
            let summary = format!("FT3: {settings} of {settings} settings clean");
            if log.matches(summary.as_str()).count() != runs
                || log.matches(" | CLEAN").count() != settings * runs
                || log.matches(" | ERRORS").count() != 0
                || log.matches("  put back: COREPLL 0xbb1,").count() != put_backs * runs
                || (name == "run GC3" && log.matches("  put back: LMCFG ").count() != 4)
            {
                return Err(format!(
                    "gpubench {name}: FT3 not run clean at every setting (want {runs} x {settings}), or a clock or memory setting not put back:\n{log}"
                ));
            }
            eprintln!(
                "    {name}: FT3 {runs} x {settings} settings clean{}",
                if stepped {
                    " at all three clocks, the clock put back"
                } else {
                    " at the clock as found (this release doesn't model Windows Mobile's clock)"
                }
            );
        }
        // GC2X refuses where Windows Mobile's clocks aren't modelled;
        // stepping, divide 1's own result and every flip tried (the
        // models draw right whatever the bits), each control right, and
        // the clock put back clean.
        if name == "run GC2X" {
            let refused =
                log.contains("not Windows Mobile's clock and bus timing: nothing changed");
            let tried = log
                .lines()
                .filter(|l| l.starts_with("  GC2X ") && l.contains(": reads 0x"))
                .collect::<Vec<_>>();
            let mapped = tried.len() == 13
                && tried
                    .iter()
                    .all(|l| l.contains(": RIGHT AT DIVIDE 1") || l.starts_with("  GC2X NO FLIP"))
                && tried.iter().all(|l| l.ends_with("| control right"))
                && log.contains(&format!(
                    "  GC2X: {} flips tried; {} drew right",
                    tried.len() - 1,
                    tried.len() - 1
                ))
                && log.contains("  put back: COREPLL 0xbb1, MBXCLK 0x6,");
            if !(refused || mapped) {
                return Err(format!(
                    "gpubench run GC2X: neither refused cleanly nor tried every flip:
{log}"
                ));
            }
        }
        if name == "run GV" {
            let refused =
                log.contains("not Windows Mobile's clock and bus timing: nothing changed");
            let stepped = (1..=5).all(|k| {
                log.contains(&format!("    control at 99.67 MHz after GV step {k}: GOOD"))
            }) && log.matches("  GV ").count() == 5
                && log.matches("  GV ").count() == log.matches("us | GOOD").count()
                && log.contains("  put back: COREPLL 0xbb1")
                && !log.contains("STOPPED");
            if !(refused && log.contains("as found: COREPLL 0x0,") || stepped) {
                return Err(format!(
                    "gpubench run GV: neither refused cleanly nor stepped every setting:\n{log}"
                ));
            }
        }
        if name == "run G" {
            let refused =
                log.contains("not Windows Mobile's clock and bus timing: nothing changed");
            let stepped = [
                "  lock trace 99.67 -> 74.75 MHz",
                "  lock trace 108.33 -> 99.67 MHz",
                "  pass 1: rising",
                "  pass 2: the same steps in reverse",
                "    control at 99.67 MHz step 22: GOOD",
                "  soak at 99.67 MHz, cores / 1: ",
                "    control at 99.67 MHz after the soak: GOOD",
                "  put back: COREPLL 0xbb1",
            ]
            .iter()
            .all(|w| log.contains(w))
                && log.matches("| GOOD").count() == 45
                && log.contains(" 0 BAD, the display short in 0")
                && !log.contains("| BAD")
                && !log.contains("STOPPED");
            if !(refused && log.contains("as found: COREPLL 0x0,") || stepped) {
                return Err(format!(
                    "gpubench run G: neither refused cleanly nor stepped every series:{log}"
                ));
            }
            eprintln!(
                "    gpubench run G: {}",
                if stepped {
                    "every step and control GOOD on the modelled clocks, put back"
                } else {
                    "refused: this fork doesn't start the clocks as Windows Mobile leaves them"
                }
            );
        }
        // Run V is built on run G's set-up: on a fork that doesn't start the
        // clocks as Windows Mobile leaves them it refuses as G does (its
        // lines were wanted unconditionally: the check failed on such forks).
        if name == "run V" {
            let refused =
                log.contains("not Windows Mobile's clock and bus timing: nothing changed");
            let loaded = [
                "V: which work at which clock",
                "  99.66 MHz, cores / 2: flush x5 false, 2D fill x20 false, whole check false (GOOD)",
                "  99.66 MHz, cores / 1: flush x5 false, 2D fill x20 false, whole check false (GOOD)",
                "  133.25 MHz, cores / 2: flush x5 false",
                "  put back: COREPLL 0xbb1",
            ]
            .iter()
            .all(|w| log.contains(w));
            if !(refused && log.contains("as found: COREPLL 0x0,") || loaded) {
                return Err(format!(
                    "gpubench run V: neither refused cleanly nor ran every load:{log}"
                ));
            }
            eprintln!(
                "    gpubench run V: {}",
                if loaded {
                    "every load at every clock, the display never short"
                } else {
                    "refused: this fork doesn't start the clocks as Windows Mobile leaves them"
                }
            );
        }
        if name == "run Y" {
            let refused = log.contains("not Windows Mobile's clock and bus timing (COREPLL 0x0,");
            let rescued = [
                "Y0: before the rebuild (the reference): speed against Y0",
                "Z1: SYSRST next",
                "Z5: the display",
                "Y1: after the rebuild: speed against Y0",
                "Y2: speed against Y0",
                "Y3: speed against Y0",
                "  self-refresh entered: true, active again: true",
                "Y5: speed against Y0",
                "everything given back: true",
            ]
            .iter()
            .all(|w| log.contains(w))
                && log.matches("| GOOD").count() == 6
                && !log.contains("| BAD");
            if !(refused || rescued) {
                return Err(format!(
                    "gpubench run Y: neither refused cleanly nor every check GOOD:{log}"
                ));
            }
            eprintln!(
                "    gpubench run Y: {}",
                if rescued {
                    "the reference, the rebuild and every restart checked GOOD"
                } else {
                    "refused: this fork doesn't start the clocks as Windows Mobile leaves them"
                }
            );
        }
        // Run F's text must come out exactly: white pixels = lit coverage,
        // and F4's, F5's and F6a's pictures word for word F0's (F6b and F6c
        // ask whether the engine keeps its surfaces: the device answers;
        // here they need only to run).
        // F7's three pictures (the gate's F6: fewer operations) must be F0's
        // too, word for word.
        if name == "run F"
            && log
                .lines()
                .filter(|l| l.starts_with("  F7") && l.contains("words differing from F0: 0"))
                .count()
                < 3
        {
            return Err(format!(
                "gpubench run F: an F7 picture differs from F0's:{log}"
            ));
        }
        if name == "run F" && log.matches("words differing from F0: 0").count() < 3 {
            return Err(format!(
                "gpubench run F: F4, F5 or F6a differs from F0's picture:{log}"
            ));
        }
        if name == "run F" {
            for l in log.lines().filter(|l| l.contains("; white ")) {
                let nums: Vec<&str> = l
                    .rsplit("; white ")
                    .next()
                    .unwrap_or("")
                    .split(", lit ")
                    .collect();
                if nums.len() != 2 || nums[0].trim() != nums[1].trim() {
                    return Err(format!("gpubench run F: the text came out wrong: `{l}`"));
                }
            }
        }
        if modelled && name == "run A" && !log.contains(" 0 hung renders recovered") {
            return Err(format!("gpubench {name}: a render hung on the model:{log}"));
        }
        // G4a: every checked pixel of all three rectangles its own texel,
        // as on the X51v.
        if rect && name == "run A" && log.matches(": 64 of 64 checked pixels").count() != 3 {
            return Err(format!(
                "gpubench {name}: G4a's rectangles should draw as on the device:{log}"
            ));
        }
        // A run that says "no map" runs no map settings (GC2T ran GC2's whole
        // map on the X51v, missing from a hand-kept list; the emulator's 3
        // renders a setting hid the time).
        if log.contains("no map;") && !log.contains(" at 0 settings rising then falling") {
            return Err(format!(
                "gpubench {name}: a no-map run ran GC2's map settings:{log}"
            ));
        }
        // A part that couldn't get its memory logs and skips itself; on the
        // emulator that is always the image's own sizing bug (GC2Q's part C
        // ran nothing on the X51v, and this check passed it).
        for short in [
            "not enough SDRAM",
            "the plan holds too many",
            "no 2700G memory for",
        ] {
            if let Some(l) = log.lines().find(|l| l.contains(short)) {
                return Err(format!(
                    "gpubench {name}: a part ran short of memory and skipped itself ({l}):{log}"
                ));
            }
        }
        eprintln!("    gpubench {name} OK: every stage ran and it returned to the loader");
    }
    if card_file(&sd, "other.txt")? != other {
        return Err("gpubench: the card logs changed a file outside rplog/".into());
    }
    eprintln!(
        "    gpubench: every run's card log matched its run log; nothing else on the card changed"
    );
    Ok(())
}

/// RetroPane's own 3D renderer's device test (rp-mbx3d-api, ADR-0026):
/// runs under the loader. With the fork's 3D model (`mbx-3d`, rp-v0.15.0)
/// every counted check must pass, as on the device (run 3: 0 BAD), and
/// every T9 try draw right (the model keeps the core's state across the
/// clock: an assumption until run 4). Without it, the renderer's set-up
/// never hears "ready" and every frame is refused (`NoCore`) without a
/// fault. Either way the clock, plane and core are put back and it returns
/// on a button.
pub fn boot_loader_mbx3d_api(root: &Path) -> Result<(), String> {
    if !device_has(root, "intel-2700g", "mbx-2d") {
        eprintln!("    mbx3d-api: skipped (this emulator release doesn't model the MBX 2D engine)");
        return Ok(());
    }
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    let loader = dir.join("rp-loader.bin");
    let bin = dir.join("rp-mbx3d-api.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-mbx3d-api"), &bin)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let check = || -> Result<(), String> {
        let mut mon = crate::screen::Monitor::connect(monitor_port())?;
        mon.query("sendkey ret 200")?;
        Ok(())
    };
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&rp_loadproto::frame(&payload), "RETROPANE-MBX3D-API: ")),
            during: Some(&check),
            then: Some("RPL END"),
            timeout: Duration::from_secs(600),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("mbx3d-api: never came back to the loader after the button press")?;
    if crashed(log) || log.contains("did not return") {
        return Err(format!("mbx3d-api: didn't return cleanly:{log}"));
    }
    let modelled = device_has(root, "intel-2700g", "mbx-3d");
    let want: &[&str] = if modelled {
        &[
            "S1 (T1)",
            "S9 (T9)",
            "everything given back: true",
            " checks, 0 BAD",
        ]
    } else {
        &[
            "S1 (T1)",
            "whole screen: NoCore",
            "S7: the 3D clock gate",
            "everything given back: true",
            "RETROPANE-MBX3D-API: ",
        ]
    };
    for want in want {
        if !log.contains(want) {
            return Err(format!("mbx3d-api: the run log has no `{want}`:{log}"));
        }
    }
    if modelled {
        let tries = log.matches("    T9 ").count();
        if tries != 6 || log.contains("WRONG") || log.contains("    BAD ") {
            return Err(format!(
                "mbx3d-api: {tries} T9 tries, every one must draw right, no check BAD:{log}"
            ));
        }
    }
    eprintln!(
        "    mbx3d-api OK: {}, back at the loader on a button press",
        if modelled {
            "every check passed on the modelled 3D core, as on the device"
        } else {
            "every stage ran, every frame refused safely (the 3D core isn't modelled)"
        }
    );
    Ok(())
}

/// The MVED1 video decoder's first test (rp-mved, ADR-0025 S0-S2): runs
/// under the loader, back on a button. The fork doesn't model the decoder
/// (docs/hw/emulator.md): its registers read as written (status 0), so S0
/// and S1 see an idle unit, the conversion times out, the guards hold,
/// mode 3 isn't tried, and the image puts the clock back.
pub fn boot_loader_mved(root: &Path) -> Result<(), String> {
    if !device_has(root, "intel-2700g", "mbx-2d") {
        eprintln!("    mved: skipped (this emulator release doesn't model the MBX 2D engine)");
        return Ok(());
    }
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let loader = dir.join("rp-loader.bin");
    let bin = dir.join("rp-mved.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-mved"), &bin)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let check = || -> Result<(), String> {
        let mut mon = crate::screen::Monitor::connect(monitor_port())?;
        mon.query("sendkey ret 200")?;
        Ok(())
    };
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&rp_loadproto::frame(&payload), "RETROPANE-MVED: ")),
            during: Some(&check),
            then: Some("RPL END"),
            timeout: Duration::from_secs(120),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("mved: never came back to the loader after the button press")?;
    if crashed(log) || log.contains("did not return") {
        return Err(format!("mved: didn't return cleanly:{log}"));
    }
    for want in [
        "S0: M24CLK 0x0",
        "set to 0x3",
        "S1: reset; FIFO 0x00000000; status 0x00000000: passed",
        "S2: 64 x 64 4:2:0 at 0x400000 to YUY2 at 0x410000",
        "job 0x30=0x80000 0x34=0x82000 0x38=0x20040 0x3c=0x2020 0x48=0x1",
        "NOT DONE after",
        "guards intact: true; destination words unwritten: 2048 of 2048",
        "RETROPANE-MVED: S0 passed, S1 passed, S2 YUY2 FAILED, mode 3 not run",
        "put back: the unit reset, M24CLK 0x0",
    ] {
        if !log.contains(want) {
            return Err(format!("mved: the run log has no `{want}`:{log}"));
        }
    }
    if log.contains("S2 mode 3: to") {
        return Err(format!("mved: mode 3 ran after a failed conversion:{log}"));
    }
    eprintln!(
        "    mved OK: every stage safe, the conversion timed out inside its guards (the decoder isn't modelled), back at the loader on a button press"
    );
    Ok(())
}

/// The blend and display-event probe (rp-mbx-blend, renderer step 1): the
/// loader starts it; it must run every stage to its verdict without a
/// fault, put the blend state and the display registers back, and return
/// on a button; then B7 alone ([`mbx_blend_b7`]). Under `-icount` (the
/// event stages time scan lines), on an emulator that models the blend
/// and the display's events (`mbx-blend`, rp-v0.13.0), its B1-B3 pixels
/// and E1/E3/E4 timings must match the device's run (docs/hw/emulator.md).
pub fn boot_loader_mbx_blend(root: &Path) -> Result<(), String> {
    if !device_has(root, "intel-2700g", "mbx-2d") {
        eprintln!("    mbx-blend: skipped (this emulator release doesn't model the MBX 2D engine)");
        return Ok(());
    }
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    let loader = dir.join("rp-loader.bin");
    let bin = dir.join("rp-mbx-blend.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-mbx-blend"), &bin)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let check = || -> Result<(), String> {
        let mut mon = crate::screen::Monitor::connect(monitor_port())?;
        mon.query("sendkey ret 200")?;
        Ok(())
    };
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &["-icount", "shift=0"],
            input: Some((&rp_loadproto::frame(&payload), "RETROPANE-MBX-BLEND: ")),
            during: Some(&check),
            then: Some("RPL END"),
            // -icount makes the run deterministic and slow: 71 s on the
            // owner's PC, past 90 s on CI's runners (2026-10-14).
            timeout: Duration::from_secs(300),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("mbx-blend: never came back to the loader after the button press")?;
    if crashed(log) || log.contains("did not return") {
        return Err(format!("mbx-blend: didn't return cleanly:{log}"));
    }
    for want in [
        "RETROPANE-MBX-BLEND: PASS",
        "E restore: DVECTRL, DINTRE, DINTRCNT as found: yes",
        "B1 Dell (blend 0x09100000, bit 17)",
        "B2 0x09100000:",
        "B3 code 15 src32 plain",
        "B4 fill 0x8000f800 bit 17, blend 0:",
        "B blend state put back to 0: yes",
        "B5 under/overrun bits",
        "B6d the screen shows four 32 bpp ramps",
    ] {
        if !log.contains(want) {
            return Err(format!("mbx-blend: the run log has no `{want}`:{log}"));
        }
    }
    for l in log.lines().filter(|l| {
        [
            "E",
            "B",
            "  ->",
            "  stuck",
            "  after",
            "picture",
            "RETROPANE",
        ]
        .iter()
        .any(|p| l.starts_with(p))
    }) {
        eprintln!("      {}", l.trim_end());
    }
    if device_has(root, "intel-2700g", "mbx-blend") {
        let bad = mbx_blend_mismatches(log, MBX_BLEND_DEVICE);
        if !bad.is_empty() {
            return Err(format!(
                "mbx-blend: {} line(s) differ from the X51v's run:
  {}",
                bad.len(),
                bad.join(
                    "
  "
                )
            ));
        }
        eprintln!(
            "    mbx-blend OK: B1-B3 pixels and the E1/E3/E4 scan-line timings match the X51v's run; state put back, back at the loader on a button press"
        );
    } else {
        eprintln!(
            "    mbx-blend OK: every stage ran, state put back, back at the loader on a button press (this emulator release doesn't model the blend: not compared with the device)"
        );
    }
    mbx_blend_b7(root)?;
    mbx_blend_b8(root)?;
    mbx_blend_b9(root)?;
    mbx_blend_b10(root)?;
    mbx_blend_b11(root)?;
    mbx_blend_b12(root)
}

/// rp-mbx-blend B7's run on the owner's X51v (2026-10-21): the lines the
/// emulator must reproduce.
const MBX_BLEND_B7_DEVICE: &str = include_str!("../data/rp-mbx-blend-b7-device.txt");

/// The B7 result line the emulator doesn't model (docs/hw/emulator.md):
/// header bit 21 wrote nothing on the X51v, meaning unknown.
const MBX_BLEND_B7_UNMODELLED: &str = "B7c header bit 0x00200000:";

/// Where an emulator run of B7 differs from the device's: each device
/// line (B7a, B7b and B7c's results) must appear in the log exactly, but
/// [`MBX_BLEND_B7_UNMODELLED`]'s.
fn mbx_blend_b7_mismatches(log: &str, device: &str) -> Vec<String> {
    device_mismatches(log, device, &[MBX_BLEND_B7_UNMODELLED])
}

/// Where an emulator run differs from the device's: each device line
/// (not `#` comments) must appear in the log exactly, but those starting
/// with one of `skip`.
fn device_mismatches(log: &str, device: &str, skip: &[&str]) -> Vec<String> {
    device
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .filter(|l| !skip.iter().any(|s| l.starts_with(s)))
        .filter(|want| !log.lines().any(|l| l.trim_end() == *want))
        .map(|want| format!("device `{want}`: not in the emulator's log"))
        .collect()
}

/// rp-mbx-blend B9's and B10's runs on the owner's X51v: the lines an
/// emulator with `mbx-2d-pal` (and `mbx-2d-rop`) must reproduce exactly.
const MBX_BLEND_B9_DEVICE: &str = include_str!("../data/rp-mbx-blend-b9-device.txt");
const MBX_BLEND_B10_DEVICE: &str = include_str!("../data/rp-mbx-blend-b10-device.txt");

/// rp-mbx-blend B11's run on the owner's X51v (2026-10-22): the lines an
/// emulator with `mbx-2d-rop` must reproduce exactly.
const MBX_BLEND_B11_DEVICE: &str = include_str!("../data/rp-mbx-blend-b11-device.txt");

/// B10 alone (`RP_MBX_BLEND_B10`, the performance gate's P13, V15, V17):
/// all 256 raster operations run (none stuck), each part to its line, the
/// state put back, the image back by itself. The answers are the device's.
fn mbx_blend_b10(root: &Path) -> Result<(), String> {
    let bin = variant_image(root, "rp-mbx-blend", ("RP_MBX_BLEND_B10", "1"))?;
    let loader = emu_dir(root)?.join("rp-loader.bin");
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
            input: Some((&rp_loadproto::frame(&payload), "RETROPANE-MBX-BLEND: ")),
            during: None,
            then: Some("RPL END"),
            timeout: Duration::from_secs(300),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("mbx-blend B10: never came back to the loader")?;
    if crashed(log) {
        return Err(format!("mbx-blend B10: crashed:{log}"));
    }
    for want in [
        " of 256 as GDI's ROP3 (pattern = colour word), 0 stuck",
        "B10b 8 bpp fill of 0xa5, 8 pixels: ran ",
        "B10b 24 bpp fill of 0x123456, 8 pixels: ran ",
        "B10c status while 20 fills are queued 0x",
        "B10d a fill with header bit 21: ran ",
        "B10 scale, key and blend put back: yes",
        "RETROPANE-MBX-BLEND: PASS - B10 done",
    ] {
        if !log.contains(want) {
            return Err(format!("mbx-blend B10: the run log has no `{want}`:{log}"));
        }
    }
    // With the fork's ROP model (mbx-2d-rop) and bit 21 (mbx-2d-pal):
    // every line as the X51v's but B10c (the FIFO level while busy).
    if device_has(root, "intel-2700g", "mbx-2d-rop")
        && device_has(root, "intel-2700g", "mbx-2d-pal")
    {
        let wrong = device_mismatches(log, MBX_BLEND_B10_DEVICE, &[]);
        if !wrong.is_empty() {
            return Err(format!(
                "mbx-blend B10: not as on the X51v:
{}",
                wrong.join(
                    "
"
                )
            ));
        }
        eprintln!(
            "    mbx-blend B10 matches the X51v's run line for line (B10c's FIFO level left out)"
        );
    }
    for l in log.lines().filter(|l| l.starts_with("B10")) {
        eprintln!("      {}", l.trim_end());
    }
    eprintln!("    mbx-blend B10 OK: every part ran, state put back, back at the loader");
    Ok(())
}

/// `rp-mbx-blend` B12 (the gate's backlog 7-10: Dell's brush, palette and
/// low-depth words, bits 23/24): the fork doesn't model the brush surface
/// or origin words, so this checks every step ran or was reset cleanly,
/// logged its line, and put the engine's state back; the results are the
/// X51v's.
fn mbx_blend_b12(root: &Path) -> Result<(), String> {
    let bin = variant_image(root, "rp-mbx-blend", ("RP_MBX_BLEND_B12", "1"))?;
    let loader = emu_dir(root)?.join("rp-loader.bin");
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
            input: Some((&rp_loadproto::frame(&payload), "RETROPANE-MBX-BLEND: ")),
            during: None,
            then: Some("RPL END"),
            timeout: Duration::from_secs(300),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("mbx-blend B12: never came back to the loader")?;
    if crashed(log) {
        return Err(format!("mbx-blend B12: crashed:{log}"));
    }
    for want in [
        "B12a brush 32 bpp origin (0,0) size field 8: ran ",
        "B12a brush 32 bpp origin (3,5) size field 8: ran ",
        "B12b brush 8 bpp through the brush palette: ran ",
        "B12c 4 bpp source through the source palette: ran ",
        "B12d 2 bpp source through the source palette: ran ",
        "B12e header bits 0x01800000 rect (8,4)-(16,8): ran ",
        "B12f copy onto itself one pixel right, rows of 256, header bits 0x01000000: ran ",
        "B12f copy onto itself one row down, rows of 256, header bits 0x00800000: ran ",
        "B12 scale, key and blend put back: yes",
        "B12 done",
    ] {
        if !log.contains(want) {
            return Err(format!("mbx-blend B12: the run log has no `{want}`:{log}"));
        }
    }
    for l in log.lines().filter(|l| l.starts_with("B12")) {
        eprintln!("      {}", l.trim_end());
    }
    eprintln!("    mbx-blend B12 OK: every step ran, state put back (the results are the X51v's)");
    Ok(())
}

fn mbx_blend_b11(root: &Path) -> Result<(), String> {
    let bin = variant_image(root, "rp-mbx-blend", ("RP_MBX_BLEND_B11", "1"))?;
    let loader = emu_dir(root)?.join("rp-loader.bin");
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
            input: Some((&rp_loadproto::frame(&payload), "RETROPANE-MBX-BLEND: ")),
            during: None,
            then: Some("RPL END"),
            timeout: Duration::from_secs(300),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("mbx-blend B11: never came back to the loader")?;
    if crashed(log) {
        return Err(format!("mbx-blend B11: crashed:{log}"));
    }
    for want in [
        "B11a code 0 (header bits 0x00000000) rect 8x4: ran true; 32 source pixels written",
        "B11a code 3 (header bits 0x06000000) rect 4x8: ran ",
        "B11b ROP4 0xaaf0 bit offset 0: ran ",
        "B11b ROP4 0xaaf0 bit offset 3: ran ",
        "B11 scale, key and blend put back: yes",
        "RETROPANE-MBX-BLEND: PASS - B11 done",
    ] {
        if !log.contains(want) {
            return Err(format!("mbx-blend B11: the run log has no `{want}`:{log}"));
        }
    }
    // With the fork's model (mbx-2d-rop): every line as the X51v drew it.
    if device_has(root, "intel-2700g", "mbx-2d-rop") {
        let wrong = device_mismatches(log, MBX_BLEND_B11_DEVICE, &[]);
        if !wrong.is_empty() {
            return Err(format!(
                "mbx-blend B11: not as on the X51v:
{}",
                wrong.join(
                    "
"
                )
            ));
        }
        eprintln!("    mbx-blend B11 matches the X51v's run line for line");
    }
    for l in log.lines().filter(|l| l.starts_with("B11")) {
        eprintln!("      {}", l.trim_end());
    }
    eprintln!("    mbx-blend B11 OK: every part ran, state put back, back at the loader");
    Ok(())
}

/// B9 alone (`RP_MBX_BLEND_B9`, the performance gate's V15, P6, P7, V3):
/// every try must run to its line (a try the engine sticks on is reset and
/// logged, never the end of the run), the state put back, and the image
/// back by itself. The answers are the device's (the fork models the clip,
/// not the scaler, palettes or 1 bpp sources).
fn mbx_blend_b9(root: &Path) -> Result<(), String> {
    let bin = variant_image(root, "rp-mbx-blend", ("RP_MBX_BLEND_B9", "1"))?;
    let loader = emu_dir(root)?.join("rp-loader.bin");
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
            input: Some((&rp_loadproto::frame(&payload), "RETROPANE-MBX-BLEND: ")),
            during: None,
            then: Some("RPL END"),
            timeout: Duration::from_secs(300),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("mbx-blend B9: never came back to the loader")?;
    if crashed(log) {
        return Err(format!("mbx-blend B9: crashed:{log}"));
    }
    for want in [
        "B9a clip (8, 8)-(24, 20), bit 18: ran true",
        "B9b scale 1:1: ran ",
        "B9b scale x1.5: ran ",
        "B9c 8 bpp with the palette word: ran ",
        "B9c 8 bpp without it, first: ran ",
        "B9e full screen (us each): 2x upscale from 240 x 320 32-bit Some(",
        "B9d 1 bpp 0x0F: ran ",
        "B9 scale, key and blend put back: yes",
        "RETROPANE-MBX-BLEND: PASS - B9 done",
    ] {
        if !log.contains(want) {
            return Err(format!("mbx-blend B9: the run log has no `{want}`:{log}"));
        }
    }
    // With the fork's palette and scaler (mbx-2d-pal): as the X51v drew.
    if device_has(root, "intel-2700g", "mbx-2d-pal") {
        let wrong = device_mismatches(log, MBX_BLEND_B9_DEVICE, &[]);
        if !wrong.is_empty() {
            return Err(format!(
                "mbx-blend B9: not as on the X51v:
{}",
                wrong.join(
                    "
"
                )
            ));
        }
        eprintln!(
            "    mbx-blend B9 matches the X51v's run line for line (B9c's stale palette and B9e's timings left out)"
        );
    }
    for l in log.lines().filter(|l| l.starts_with("B9")) {
        eprintln!("      {}", l.trim_end());
    }
    eprintln!("    mbx-blend B9 OK: every try ran to its line, state put back, back at the loader");
    Ok(())
}

/// B8 alone (`RP_MBX_BLEND_B8`, the performance gate's P21): the dither
/// add both ways must give the same picture, word for word, and each be
/// timed; the image returns by itself. Timings aren't modelled.
fn mbx_blend_b8(root: &Path) -> Result<(), String> {
    let bin = variant_image(root, "rp-mbx-blend", ("RP_MBX_BLEND_B8", "1"))?;
    let loader = emu_dir(root)?.join("rp-loader.bin");
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
            input: Some((&rp_loadproto::frame(&payload), "RETROPANE-MBX-BLEND: ")),
            during: None,
            then: Some("RPL END"),
            timeout: Duration::from_secs(300),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("mbx-blend B8: never came back to the loader")?;
    if crashed(log) {
        return Err(format!("mbx-blend B8: crashed:{log}"));
    }
    for want in [
        "B8 the two ways' results: 0 of 307200 words differ",
        "B8 dither add, full screen: four blits from pattern rows Some(",
        "B8 dither add, 480 x 80: four blits from pattern rows Some(",
        "B8c 10 full-screen fills, the fence polled every 0 us: landed after Some(",
        "B8c 10 full-screen fills, the fence polled every 5000 us: landed after Some(",
        "B8 blend state put back to 0: yes",
        "RETROPANE-MBX-BLEND: PASS - B8 done",
    ] {
        if !log.contains(want) {
            return Err(format!("mbx-blend B8: the run log has no `{want}`:{log}"));
        }
    }
    eprintln!(
        "    mbx-blend B8 OK: both ways of adding the dither give the same picture, both timed (timings not modelled)"
    );
    Ok(())
}

/// B7 alone (`RP_MBX_BLEND_B7`, ADR-0023's 16-bit screen): every part
/// must run to its line and the image return to the loader by itself (no
/// button); on an emulator that models the blend (`mbx-blend`), its
/// conversion, blend and dither-try results must equal the X51v's run
/// (timings aren't modelled, header bit 21 isn't: docs/hw/emulator.md).
fn mbx_blend_b7(root: &Path) -> Result<(), String> {
    let bin = variant_image(root, "rp-mbx-blend", ("RP_MBX_BLEND_B7", "1"))?;
    let loader = emu_dir(root)?.join("rp-loader.bin");
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
            input: Some((&rp_loadproto::frame(&payload), "RETROPANE-MBX-BLEND: ")),
            during: None,
            then: Some("RPL END"),
            timeout: Duration::from_secs(300),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("mbx-blend B7: never came back to the loader")?;
    if crashed(log) || log.contains("did not return") {
        return Err(format!("mbx-blend B7: didn't return cleanly:{log}"));
    }
    let tries = rp_mbx_blend_tries();
    for want in [
        "RETROPANE-MBX-BLEND: PASS - B7 done",
        "B7a convert red: of 256 levels",
        "B7a convert green: of 256 levels",
        "B7a convert blue: of 256 levels",
        "B7b over alpha 0xff red:",
        "B7b over alpha 0x80 blue:",
        "B7d convert 32 to 16 bits, full screen: Some(",
        "B7d copy 16 to 16 bits (a forward copy), full screen: Some(",
        "B7d dither add at 32 bits (4 blits, blend 0), full screen: Some(",
        "B7 blend state put back to 0: yes",
    ] {
        if !log.contains(want) {
            return Err(format!("mbx-blend B7: the run log has no `{want}`:{log}"));
        }
    }
    let done = log
        .lines()
        .filter(|l| l.starts_with("B7c ") && l.contains(": trunc "))
        .count();
    if done != tries {
        return Err(format!(
            "mbx-blend B7: {done} of {tries} B7c tries logged a result:{log}"
        ));
    }
    for l in log.lines().filter(|l| l.starts_with("B7")) {
        eprintln!("      {}", l.trim_end());
    }
    if !device_has(root, "intel-2700g", "mbx-blend") {
        eprintln!(
            "    mbx-blend B7 OK: every part ran ({tries} B7c tries), blend state put back, back at the loader by itself (this emulator release doesn't model the blend: not compared with the device)"
        );
        return Ok(());
    }
    // Header bit 21 is modelled with mbx-2d-pal (fork branch rp-next).
    let bit21 = device_has(root, "intel-2700g", "mbx-2d-pal");
    let bad = if bit21 {
        device_mismatches(log, MBX_BLEND_B7_DEVICE, &[])
    } else {
        mbx_blend_b7_mismatches(log, MBX_BLEND_B7_DEVICE)
    };
    if !bad.is_empty() {
        return Err(format!(
            "mbx-blend B7: {} line(s) differ from the X51v's run:\n  {}",
            bad.len(),
            bad.join("\n  ")
        ));
    }
    eprintln!(
        "    mbx-blend B7 OK: every part ran ({tries} B7c tries), the conversion, blends and tries match the X51v's run ({}), back at the loader by itself",
        if bit21 {
            "header bit 21 too"
        } else {
            "header bit 21 not modelled"
        }
    );
    Ok(())
}

/// B7c's tries (`rp-mbx-blend`'s `dither_tries`: 1 plain, 4 header bits,
/// 1 blend path, 24 blend-word bits).
const fn rp_mbx_blend_tries() -> usize {
    1 + 4 + 1 + 4 + 20
}

/// The wallpaper rp-today was built with: its build script's output, the
/// newest `wallpaper.yuv` under the release build folder in an `rp-today`
/// folder (Cargo's layout is `build/rp-today-<hash>/out` or, newer,
/// `build/rp-today/<hash>/out`: both are searched).
fn today_wallpaper(release: &Path) -> Result<Vec<u8>, String> {
    fn find(dir: &Path, depth: u32, in_today: bool, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.filter_map(Result::ok) {
            let p = e.path();
            let today = in_today || e.file_name().to_string_lossy().starts_with("rp-today");
            if p.is_dir() && depth > 0 {
                find(&p, depth - 1, today, out);
            } else if today && e.file_name() == "wallpaper.yuv" {
                out.push(p);
            }
        }
    }
    let mut found = Vec::new();
    find(&release.join("build"), 4, false, &mut found);
    let newest = found
        .into_iter()
        .max_by_key(|p| p.metadata().and_then(|m| m.modified()).ok())
        .ok_or("today: no wallpaper.yuv from rp-today's build")?;
    std::fs::read(&newest).map_err(|e| format!("{}: {e}", newest.display()))
}

/// What rp-today should show: the same set-up (`rp_today::show_gpu`),
/// shell and GPU renderer (ADR-0023) as the image, on the Rust model of
/// the 2D engine, frame after frame; the emulator's own model of the
/// engine must draw the same pictures. `steps` are applied between
/// screens; the first screen is before any. The clock's time is
/// arbitrary: the check masks it.
fn today_screens(wall: &[u8], steps: &[rp_shell::Input]) -> Vec<Vec<u8>> {
    use rp_draw::{Frame, Renderer};
    use rp_mbx_render::model::{self, Bus, Mem};
    use rp_mbx_render::{Layout, Mbx2dRenderer, SCREEN_BYTES};

    let mem = model::memory();
    let mut gpu = Mbx2dRenderer::new(
        Bus::new(mem.clone()),
        Mem(mem.clone()),
        Layout::X51V,
        || 0,
        1,
    );
    rp_today::show_gpu(&mut gpu, wall, |_| {}, &mut rp_today::WallScratch::new());
    let mut shell = rp_shell::Shell::new(rp_today::shell_fonts(), rp_today::icons(), true);
    shell.set_time(rp_shell::Time {
        hour: 12,
        minute: 0,
        weekday: 0,
        day: 1,
        month: 1,
    });
    let mut frame = Frame::new();
    let mut screens = Vec::new();
    for step in std::iter::once(None).chain(steps.iter().map(Some)) {
        if let Some(&e) = step {
            shell.input(e);
        }
        shell.frame(&mut frame);
        gpu.draw(&frame);
        while !gpu.poll() {}
        let scadr = gpu.parts().0.regs.reg(rp_drv_mbx_display::reg::GSCADR);
        let base = ((scadr & 0x7F_FFFF) * 16) as usize;
        let m = mem.borrow();
        let out = m[base..base + SCREEN_BYTES]
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| shown_565(u16::from_le_bytes(*p)))
            .collect();
        screens.push(out);
    }
    screens
}

/// A screen pixel (RGB565) as the emulator's screendump shows it: each
/// channel shifted up, its low bits 0 (the fork's scan-out, docs/hw/2700g.md
/// "Plane composition": "RGB565 shifted up without bit replication").
fn shown_565(p: u16) -> [u8; 3] {
    let p = u32::from(p);
    [
        ((p >> 11) << 3) as u8,
        (((p >> 5) & 0x3F) << 2) as u8,
        ((p & 0x1F) << 3) as u8,
    ]
}

/// The wallpaper comparison's screens (the test image's "Wallpaper
/// compare", the owner's 16-bit check; since ADR-0023's L1 dithered
/// against plain 16 bits, quality audit C48): the wallpaper alone as the
/// OS keeps it (dithered), then plain, each once its notice has gone,
/// through the same shell and renderer code as the image.
fn compare_screens(wall: &[u8]) -> [Vec<u8>; 2] {
    use rp_draw::{Frame, Renderer};
    use rp_mbx_render::model::{self, Bus, Mem};
    use rp_mbx_render::{Layout, Mbx2dRenderer, SCREEN_BYTES};
    use rp_shell::{Action, Input, Shell};

    let mem = model::memory();
    let mut gpu = Mbx2dRenderer::new(
        Bus::new(mem.clone()),
        Mem(mem.clone()),
        Layout::X51V,
        || 0,
        1,
    );
    let mut scratch = rp_today::WallScratch::new();
    rp_today::show_gpu(&mut gpu, wall, |_| {}, &mut scratch);
    let mut shell = Shell::new(rp_today::shell_fonts(), rp_today::icons(), true);
    let mut frame = Frame::new();
    let mut step = |shell: &mut Shell, gpu: &mut Mbx2dRenderer<Bus, Mem>| -> Vec<u8> {
        shell.frame(&mut frame);
        gpu.draw(&frame);
        while !gpu.poll() {}
        let scadr = gpu.parts().0.regs.reg(rp_drv_mbx_display::reg::GSCADR);
        let base = ((scadr & 0x7F_FFFF) * 16) as usize;
        let m = mem.borrow();
        m[base..base + SCREEN_BYTES]
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| shown_565(u16::from_le_bytes(*p)))
            .collect()
    };
    let tap = |shell: &mut Shell, r: rp_draw::Rect| {
        let (x, y) = (r.x + r.w / 2, r.y + r.h / 2);
        shell.input(Input::PenDown { x, y });
        shell.input(Input::PenUp { x, y })
    };
    step(&mut shell, &mut gpu);
    tap(&mut shell, rp_shell::START);
    step(&mut shell, &mut gpu);
    tap(&mut shell, Shell::menu_entry(Shell::menu_entries(true) - 3));
    assert!(
        shell.wallpaper_only(),
        "the compare entry is third from the end"
    );
    step(&mut shell, &mut gpu);
    shell.tick(10_000);
    let dithered = step(&mut shell, &mut gpu);
    let act = tap(&mut shell, rp_draw::Rect::new(200, 300, 80, 40));
    assert_eq!(act, Action::WallpaperDepth { dithered: false });
    rp_today::load_wallpaper(&mut gpu, wall, rp_today::Depth::Full, &mut scratch);
    gpu.wallpaper_changed();
    step(&mut shell, &mut gpu);
    shell.tick(20_000);
    let plain = step(&mut shell, &mut gpu);
    [dithered, plain]
}

/// Pixels that differ outside the clock's areas (their text depends on
/// when the check runs).
fn today_differ(got: &[u8], want: &[u8]) -> usize {
    let masked = [
        rp_shell::DATE_BOX,
        rp_shell::FORECAST_ROW,
        rp_shell::TITLE_CLOCK,
    ];
    got.chunks(3)
        .zip(want.chunks(3))
        .enumerate()
        .filter(|(i, (a, b))| {
            let (x, y) = ((i % rp_today::W) as u16, (i / rp_today::W) as u16);
            a != b && !masked.iter().any(|r| r.contains_point(x, y))
        })
        .count()
}

/// Pixels that differ within 32 px of a press at (`x`, `y`), unmasked.
fn press_answer(got: &[u8], want: &[u8], x: u16, y: u16) -> usize {
    let near = rp_draw::Rect::new(x.saturating_sub(32), y.saturating_sub(32), 64, 64);
    got.chunks(3)
        .zip(want.chunks(3))
        .enumerate()
        .filter(|(i, (a, b))| {
            let (px, py) = ((i % rp_today::W) as u16, (i / rp_today::W) as u16);
            a != b && near.contains_point(px, py)
        })
        .count()
}

/// How long emu-today waits for the image to answer a press on screen: a
/// backstop only, never a timing (the press waits for the answer).
const PRESS_ANSWER: Duration = Duration::from_secs(60);
/// A tap the image answers on the lift, held this long in its own time
/// (its pen filter lands a press in ~30 ms of samples).
const TAP_GUEST_MS: u32 = 150;

/// The shell's Today screen (ADR-0021) under the loader: pixel-exact
/// against the reference (clocks masked), still there after 40 s untouched
/// (past the run limit: idle sleeps must check in), a stylus tap on Start opens the
/// menu (pixel-exact again), and the menu's "Back to loader" returns with
/// the touch-to-flush latencies in the run log.
pub fn boot_loader_today(root: &Path) -> Result<(), String> {
    use rp_shell::Input;
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    let loader = dir.join("rp-loader.bin");
    let bin = dir.join("rp-today.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-today"), &bin)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let wall = today_wallpaper(&release)?;
    let start = rp_shell::START;
    let (sx, sy) = (start.x + start.w / 2, start.y + start.h / 2);
    let want = today_screens(
        &wall,
        &[
            Input::PenDown { x: sx, y: sy },
            Input::PenUp { x: sx, y: sy },
        ],
    );
    let bare = compare_screens(&wall);
    let wsl = matches!(emulator(root)?, Emulator::Wsl);
    let dump = |name: &str| -> Result<Vec<u8>, String> {
        let shot = scratch(root, &format!("today-{name}.ppm"))?;
        let guest = if wsl {
            to_wsl_path(&shot)
        } else {
            shot.display().to_string()
        };
        let _ = std::fs::remove_file(&shot);
        let mut mon = crate::screen::Monitor::connect(monitor_port())?;
        mon.screendump(&guest, &shot)?;
        let img = crate::screen::parse_ppm(&std::fs::read(&shot).map_err(|e| e.to_string())?)?;
        Ok(img.rgb)
    };
    // Polls the screen until it matches (emulated time runs slower than
    // the wall clock on a busy PC: never a fixed sleep).
    let until = |name: &str, want: &[u8]| -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            poll_pause(Duration::from_millis(500));
            let n = today_differ(&dump(name)?, want);
            if n == 0 {
                return Ok(());
            }
            if Instant::now() > deadline {
                // The expected picture beside the screendump, to compare.
                let mut ppm = format!(
                    "P6
{} {}
255
",
                    rp_today::W,
                    rp_today::H
                )
                .into_bytes();
                ppm.extend_from_slice(want);
                let _ = std::fs::write(scratch(root, &format!("today-{name}-want.ppm"))?, ppm);
                return Err(format!(
                    "today: the {name} screen differs from the reference in {n} pixels (target/emulator/today-{name}.ppm, expected today-{name}-want.ppm)"
                ));
            }
        }
    };
    let check = || -> Result<(), String> {
        until("first", &want[0])?;
        // Left alone past the loader's 30 s run limit: an idle image must
        // keep checking in (it slept through it once and the watchdog reset
        // the device). Then it must still be there, unchanged.
        guest_hold(
            &mut crate::screen::Monitor::connect(monitor_port())?,
            40000,
            "today",
        )?;
        until("idle", &want[0])?;
        let mut qmp = crate::screen::Qmp::connect(monitor_port() + 1)?;
        // Every press is held until the image answers it on screen, never
        // for a fixed time: the image samples the pen in its own time, and
        // a busy host gives it few samples per wall-clock second (with
        // every core busy a 700 ms hold fell short of the align screen's
        // HOLD samples and the menu-aligned check failed by 234960 px).
        let changed = |name: &str, from: &[u8]| -> Result<Vec<u8>, String> {
            let deadline = Instant::now() + PRESS_ANSWER;
            loop {
                let now = dump(name)?;
                if today_differ(&now, from) > 0 {
                    return Ok(now);
                }
                if Instant::now() > deadline {
                    return Err(format!("today: no answer on screen to the {name}"));
                }
                std::thread::sleep(Duration::from_millis(200));
            }
        };
        // A press's answer shows where the pen is (a pressed target, a
        // target turning green): only pixels within 32 px of it count, so
        // the clock's areas needn't be masked (the align screen's centre
        // target lies inside them, and a masked answer was never seen).
        let answered = |name: &str, from: &[u8], x: u16, y: u16| -> Result<Vec<u8>, String> {
            let deadline = Instant::now() + PRESS_ANSWER;
            loop {
                let now = dump(name)?;
                if press_answer(&now, from, x, y) > 0 {
                    return Ok(now);
                }
                if Instant::now() > deadline {
                    return Err(format!("today: no answer on screen to the {name}"));
                }
                std::thread::sleep(Duration::from_millis(200));
            }
        };
        // Guest time, for presses the image answers only on the lift (the
        // bare wallpaper's depth flip).
        let guest_hold = |ms: u32| {
            guest_hold(
                &mut crate::screen::Monitor::connect(monitor_port())?,
                ms,
                "today",
            )
        };
        // Down until the screen shows the press (a pressed target, the
        // align target turning green), or for `guest_ms` of the image's
        // time when it answers only on the lift; then up; returns the
        // screen while held.
        let press =
            |qmp: &mut crate::screen::Qmp, name: &str, x: u16, y: u16, guest_ms: Option<u32>| {
                let before = dump(name)?;
                touch(qmp, &Action::TouchDown(x, y))?;
                let held = match guest_ms {
                    None => answered(&format!("{name}-down"), &before, x, y),
                    Some(ms) => guest_hold(ms).map(|()| before),
                };
                touch(qmp, &Action::TouchUp)?;
                held
            };
        press(&mut qmp, "start", sx, sy, None)?;
        until("menu", &want[2])?;
        // The align screen (the test image's second-to-last entry): hold
        // each target, then the check dot, as the owner would.
        let entries = rp_shell::Shell::menu_entries(true);
        let align = rp_shell::Shell::menu_entry(entries - 2);
        let held = press(
            &mut qmp,
            "align",
            align.x + align.w / 2,
            align.y + align.h / 2,
            None,
        )?;
        changed("align-up", &held)?;
        for (i, &(x, y)) in rp_shell::align::TARGETS
            .iter()
            .chain([&rp_shell::align::CHECK])
            .enumerate()
        {
            // The lift moves on (the next target, or the Today screen).
            let held = press(&mut qmp, &format!("target{i}"), x, y, None)?;
            changed(&format!("target{i}-up"), &held)?;
        }
        // Calibrated: Start must open the same menu, pixel for pixel (the
        // taps land where they should through the new calibration).
        press(&mut qmp, "start-aligned", sx, sy, None)?;
        until("menu-aligned", &want[2])?;
        // The wallpaper comparison (the owner's 16-bit check, the third
        // entry from the end): the wallpaper alone, pixel-exact dithered
        // and plain once each notice has gone; a tap flips it, Home goes
        // back. Ends dithered, so the menu below matches as before.
        let compare = rp_shell::Shell::menu_entry(entries - 3);
        press(
            &mut qmp,
            "compare",
            compare.x + compare.w / 2,
            compare.y + compare.h / 2,
            None,
        )?;
        until("bare-dithered", &bare[0])?;
        press(&mut qmp, "flip-plain", 240, 320, Some(TAP_GUEST_MS))?;
        until("bare-plain", &bare[1])?;
        press(&mut qmp, "flip-dithered", 240, 320, Some(TAP_GUEST_MS))?;
        until("bare-dithered-again", &bare[0])?;
        // Home, held in the image's own time (a `sendkey` hold is wall
        // clock, and under load the image never sampled it).
        qmp.key("f4", true)?;
        guest_hold(TAP_GUEST_MS)?;
        qmp.key("f4", false)?;
        // Home leaves the wallpaper alone: wait for the Today screen (its
        // depth notice gone), not for a time (under load 500 ms wasn't
        // enough and Start was pressed on the bare wallpaper).
        until("today-after-compare", &want[0])?;
        press(&mut qmp, "start-after-compare", sx, sy, None)?;
        until("menu-after-compare", &want[2])?;
        // The last menu entry is "Back to loader" (test images only).
        let back = rp_shell::Shell::menu_entry(entries - 1);
        press(
            &mut qmp,
            "back",
            back.x + back.w / 2,
            back.y + back.h / 2,
            None,
        )?;
        // Back at the loader: the wallpaper between the clock and weather
        // panels (rows 210-217, nothing drawn over it) is gone. Polled, so
        // the run log has been sent when the emulator stops.
        let strip = |s: &[u8]| s[210 * rp_today::W * 3..218 * rp_today::W * 3].to_vec();
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            poll_pause(Duration::from_millis(500));
            if strip(&dump("after")?) != strip(&want[2]) {
                return Ok(());
            }
            if Instant::now() > deadline {
                return Err("today: still showing after Back to loader".into());
            }
        }
    };
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            // Guest time from instructions, not the host clock: with every
            // core busy the starved guest saw its time jump past the
            // loader's 30 s run limit mid-hold ("FAULT RunLimit", checking
            // in every 5 ms of its own time; 2026-10-15). The idle wait
            // stays in real time, which a mostly sleeping guest follows.
            extra: &["-icount", "shift=0"],
            input: Some((&rp_loadproto::frame(&payload), rp_today::MARKER)),
            during: Some(&check),
            then: Some("RPL END"),
            timeout: Duration::from_secs(600),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("today: never came back to the loader")?;
    if crashed(log) || log.contains("did not return") {
        return Err(format!("today: didn't return cleanly:{log}"));
    }
    let line = log
        .lines()
        .find(|l| l.contains("touch to flush:"))
        .ok_or_else(|| format!("today: no latency line in the run log:{log}"))?;
    let answered: u32 = line
        .split("flush: ")
        .nth(1)
        .and_then(|t| t.split(' ').next())
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| format!("today: unreadable latency line {line:?}"))?;
    if answered < 2 {
        return Err(format!("today: the taps weren't answered: {line:?}"));
    }
    let calibrated = log
        .lines()
        .find(|l| l.contains("calibrated:"))
        .ok_or_else(|| format!("today: the align screen never finished:{log}"))?;
    let stack =
        check_stack(log)?.ok_or_else(|| format!("today: no stack line in the run log:{log}"))?;
    eprintln!(
        "    today OK: Today screen pixel-exact (clocks masked) and still up after 40 s idle, Start opened the menu pixel-exact, the align screen calibrated ({}) and the menu opened pixel-exact again, the wallpaper alone pixel-exact dithered and plain (16 bits), Back to loader returned; {}; stack {} of {} bytes",
        calibrated.trim(),
        line.trim(),
        stack.0,
        stack.1
    );
    Ok(())
}

/// The touch replay (`RP_TODAY_REPLAY`, the ratchet's touch row,
/// docs/design/perf-ratchet.md): the owner's recorded taps through the
/// image's filter, shell and drawing, no stylus. It must take all 28
/// presses, answer them, write its `TR` line and return to the loader by
/// itself. Its times are the emulator's, not the device's: not compared.
pub fn today_replay(root: &Path) -> Result<(), String> {
    let bin = variant_image(root, "rp-today", ("RP_TODAY_REPLAY", "1"))?;
    let loader = emu_dir(root)?.join("rp-loader.bin");
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
            input: Some((&rp_loadproto::frame(&payload), rp_today::MARKER)),
            during: None,
            then: Some("RPL END"),
            timeout: Duration::from_secs(300),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("today replay: never came back to the loader")?;
    if crashed(log) || log.contains("did not return") {
        return Err(format!("today replay: didn't return cleanly:{log}"));
    }
    let Some(tr) = log.lines().find(|l| l.starts_with("TR touch replay: ")) else {
        return Err(format!("today replay: no `TR touch replay:` line:{log}"));
    };
    if !tr.contains(" 28 presses, ") {
        return Err(format!(
            "today replay: not the recording's 28 presses: {tr}"
        ));
    }
    if log.lines().any(|l| l.starts_with("R ")) {
        return Err(format!(
            "today replay: the recorder logged samples (it is off in the replay):{log}"
        ));
    }
    eprintln!("      {}", tr.trim_end());
    eprintln!(
        "    today replay OK: 28 presses replayed and answered, back at the loader by itself (times are the emulator's)"
    );
    Ok(())
}

/// What rp-blend's `stage` should put on screen: the image's own drawing
/// and register programming (shared code), through the reference model.
fn blend_screen(stage: rp_drv_mbx_display::blendtest::Stage) -> Vec<u8> {
    use rp_drv_mbx_display::{Display, blendtest, scanout};
    let mut vram = vec![0u8; 16 << 20];
    let mut d = Display::new(RegFile::default());
    let _ = d.adopt(vram.len());
    blendtest::draw(stage, &mut vram, &mut vec![0u8; blendtest::WORK]);
    blendtest::program(&mut d, stage);
    let regs = d.into_inner();
    let mut out = vec![0u8; blendtest::W * blendtest::H * 3];
    scanout::compose(|o| regs.0.get(&o).copied().unwrap_or(0), &vram, &mut out);
    out
}

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

/// Port of the emulated USB serial port (the UDC model's chardev); each test
/// gets its own (`n`), so one can never reach another test's emulator.
fn usb_port(n: u16) -> u16 {
    assert!(n < USB_PORTS, "usb_port({n}): past this run's port block");
    monitor_port() + 2 + n
}

/// QEMU arguments that connect the UDC model's serial data to a TCP port,
/// with the model's protocol checks reported (guest errors).
fn usb_args(n: u16) -> [String; 6] {
    [
        "-d".into(),
        "guest_errors".into(),
        "-chardev".into(),
        format!(
            "socket,id=usb,host=127.0.0.1,port={},server=on,wait=off",
            usb_port(n)
        ),
        "-global".into(),
        "pxa27x-udc.chardev=usb".into(),
    ]
}

/// QEMU arguments for the UDC model's mass storage chardev (ADR-0014).
fn msc_args(n: u16) -> [String; 4] {
    [
        "-chardev".into(),
        format!(
            "socket,id=msc,host=127.0.0.1,port={},server=on,wait=off",
            usb_port(n)
        ),
        "-global".into(),
        "pxa27x-udc.msc=msc".into(),
    ]
}

/// Whether the emulator's `device` model has the property `prop`.
/// An emulator that can't be used (refused as the wrong release, quality
/// audit C41; missing; not downloaded) ends the run. The feature probes
/// (`device_has` and its siblings) once answered "not modelled" here, so a
/// refused emulator made checks print "skipped" and pass (found
/// 2026-10-21 with C41's refusal).
fn no_emulator(e: &str) -> ! {
    eprintln!("error: {e}");
    std::process::exit(1)
}

fn device_has(root: &Path, device: &str, prop: &str) -> bool {
    let out = match emulator(root) {
        Ok(Emulator::Native(exe)) => Command::new(exe)
            .args(["-M", "x51v", "-device", &format!("{device},help")])
            .output(),
        Ok(Emulator::Wsl) => {
            wsl_bash(&format!("{} -M x51v -device {device},help", wsl_qemu())).output()
        }
        Err(e) => no_emulator(&e),
    };
    out.is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains(&format!("{prop}=")))
}

/// Whether the emulator starts with `-global <setting>`: an emulator whose
/// device lacks the property refuses to start. For devices whose `,help`
/// can't be asked (the PC Card socket's instantiates it outside a machine,
/// which asserts in QEMU's memory code).
fn emulator_accepts_global(root: &Path, setting: &str) -> bool {
    let args = format!("-M x51v -display none -S -monitor stdio -serial null -global {setting}");
    let child = match emulator(root) {
        Ok(Emulator::Native(exe)) => {
            // Windows QEMU never reads a stdin pipe (item 61), so `quit`
            // can't end it: it waited forever (2026-10-16, emu-cf on the
            // rp-v0.17.0 Windows release). A rejected setting ends QEMU at
            // once with an error; one still paused after the wait was taken.
            let native = args.replace("-monitor stdio", "-monitor none");
            let Ok(mut child) = Command::new(exe)
                .args(native.split(' '))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            else {
                return false;
            };
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline {
                if let Ok(Some(status)) = child.try_wait() {
                    return status.success();
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            let _ = child.kill();
            let _ = child.wait();
            return true;
        }
        Ok(Emulator::Wsl) => wsl_bash(&format!("echo quit | {} {args}", wsl_qemu()))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn(),
        Err(e) => no_emulator(&e),
    };
    let Ok(mut child) = child else {
        return false;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(b"quit\n");
    }
    child.wait().is_ok_and(|s| s.success())
}

/// Whether a device at `path` in the running emulator has `prop`.
fn device_has_runtime(mon: &mut crate::screen::Monitor, path: &str, prop: &str) -> bool {
    mon.query(&format!("qom-list {path}"))
        .is_ok_and(|out| out.contains(prop))
}

/// Whether the emulator's UDC model bridges a mass storage function
/// (retropane-qemu after rp-v0.8.0).
fn has_msc_model(root: &Path) -> bool {
    device_has(root, "pxa27x-udc", "msc")
}

/// The block the emulated card is too slow on for a multi-block read
/// (like the owner's card at block 10880): inside the drive test's raw
/// 1 MiB read, before the partition, so only that read meets it.
const SLOW_BLOCK: u32 = 1000;

/// Whether the emulator in use has the USB device controller model
/// (retropane-qemu after rp-v0.6.0). Older releases skip the USB tests.
fn has_usb_model(root: &Path) -> bool {
    let out = match emulator(root) {
        Ok(Emulator::Native(exe)) => Command::new(exe)
            .args(["-M", "x51v", "-device", "pxa27x-udc,help"])
            .output(),
        Ok(Emulator::Wsl) => {
            wsl_bash(&format!("{} -M x51v -device pxa27x-udc,help", wsl_qemu())).output()
        }
        Err(e) => no_emulator(&e),
    };
    out.is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("chardev"))
}

/// Connects to the emulated USB serial port.
fn usb_connect(n: u16) -> Result<std::net::TcpStream, String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match std::net::TcpStream::connect(("127.0.0.1", usb_port(n))) {
            Ok(s) => {
                s.set_read_timeout(Some(Duration::from_millis(200)))
                    .map_err(|e| e.to_string())?;
                return Ok(s);
            }
            Err(e) if Instant::now() > deadline => {
                return Err(format!("emulated USB serial port: {e}"));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

/// Reads from `s` until `want` arrives (or `secs` pass). Returns what came.
fn usb_read_until(s: &mut std::net::TcpStream, want: &str, secs: u64) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(secs);
    let mut got = Vec::new();
    let mut buf = [0u8; 256];
    while Instant::now() < deadline {
        match s.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                got.extend_from_slice(&buf[..n]);
                if String::from_utf8_lossy(&got).contains(want) {
                    let text = String::from_utf8_lossy(&got).into_owned();
                    check_stack(&text)?;
                    return Ok(text);
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Err(format!(
        "no `{want}` on the emulated USB port; got {:?}",
        String::from_utf8_lossy(&got)
    ))
}

/// Reads until `start` and the end of its line: the reply's whole line
/// (with whatever came before it), however the bytes were split into reads.
/// (Reading until `start` and then until a newline waited for a second
/// line whenever the reply arrived in one piece.)
fn usb_read_line(s: &mut std::net::TcpStream, start: &str, secs: u64) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(secs);
    let mut got = Vec::new();
    let mut buf = [0u8; 256];
    while Instant::now() < deadline {
        match s.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                got.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&got);
                if let Some(at) = text.find(start)
                    && text[at..].contains('\n')
                {
                    check_stack(&text)?;
                    return Ok(text.into_owned());
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Err(format!(
        "no whole `{start}` line on the emulated USB port; got {:?}",
        String::from_utf8_lossy(&got)
    ))
}

/// The USB link test end to end: the emulator's host enumerates rp-usb,
/// opens the port (the banner arrives) and gets its echo back.
pub fn boot_usb_link(root: &Path) -> Result<(), String> {
    if !has_usb_model(root) {
        eprintln!("    usb link: skipped (this emulator has no USB controller model)");
        return Ok(());
    }
    let image = crate::arm_release(root).join("rp-usb");
    let args = usb_args(0);
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let check = || -> Result<(), String> {
        let mut s = usb_connect(0)?;
        usb_read_until(&mut s, "RetroPane 26 USB console OK", 10)?;
        s.write_all(b"echo through USB\r\n")
            .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "echo through USB", 10)?;
        Ok(())
    };
    boot(
        root,
        &Boot {
            image: &image,
            marker: USB_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &extra,
            input: None,
            during: Some(&check),
            then: None,
            timeout: BOOT_TIMEOUT,
        },
    )?;
    eprintln!("    usb link OK: enumerated by the emulated host, banner and echo over USB");
    Ok(())
}

/// The loader over USB: first light sent through the emulated USB serial
/// port, its run log back the same way.
/// `cargo xtask emu-usb`: the loader-over-USB check on its own.
pub fn boot_loader_usb_alone(root: &Path) -> Result<(), String> {
    usb_images(root)?;
    boot_loader_usb(root)
}

/// `cargo xtask emu-drive`: the drive check alone (item 74: its blocks go
/// by the UDC's DMA both ways).
pub fn boot_loader_drive_alone(root: &Path) -> Result<(), String> {
    usb_images(root)?;
    boot_loader_drive(root)
}

/// The raw images the USB checks send.
fn usb_images(root: &Path) -> Result<(), String> {
    let dir = emu_dir(root)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let release = crate::arm_release(root);
    for image in ["rp-loader", "rp-light"] {
        crate::sdkit::raw_binary(&release.join(image), &dir.join(format!("{image}.bin")))?;
    }
    Ok(())
}

pub fn boot_loader_usb(root: &Path) -> Result<(), String> {
    if !has_usb_model(root) {
        eprintln!("    loader over USB: skipped (this emulator has no USB controller model)");
        return Ok(());
    }
    let loader = raw_image(root, "rp-loader")?;
    let light = raw_image(root, "rp-light")?;
    let payload = std::fs::read(&light).map_err(|e| format!("{}: {e}", light.display()))?;
    let frame = rp_loadproto::frame(&payload);
    let args = usb_args(1);
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let check = || -> Result<(), String> {
        let mut s = usb_connect(1)?;
        usb_read_until(&mut s, "RPLOAD1 READY", 10)?;
        s.write_all(&frame).map_err(|e| e.to_string())?;
        let ok = usb_read_line(&mut s, "RPL OK sha256 ", 15)?;
        let theirs = ok.rsplit("RPL OK sha256 ").next().unwrap_or("").trim();
        crate::usb::check_digest(theirs, &payload, "image over USB")?;
        // The loader disconnects to run the image and reconnects when it
        // returns (the emulated host enumerates it again); the socket stays
        // up throughout, and the run log arrives once the port is reopened.
        let log = usb_read_until(&mut s, "RPL END", 30)?;
        if !log.contains("all steps OK") || !log.contains("slot: fresh") {
            return Err(format!("run log over USB incomplete: {log:?}"));
        }
        // The same image again under the same loader: it must start fresh
        // (rt::Slot: filled once per start, `.bss` zeroed by `_start`), not
        // find the first run's state. Sent with the card log's name in the
        // same transfer, as `cargo xtask load` sends it: the image's first
        // bytes share a packet with the name's end (2026-10-20, on the
        // X51v: the loader dropped them and the image never arrived).
        let mut both = rp_loadproto::control_frame("name light");
        both.extend_from_slice(&frame);
        s.write_all(&both).map_err(|e| e.to_string())?;
        usb_read_line(&mut s, "RPL OK name", 15)?;
        usb_read_line(&mut s, "RPL OK sha256 ", 15)?;
        let again = usb_read_until(&mut s, "RPL END", 30)?;
        if !again.contains("all steps OK") || !again.contains("slot: fresh") {
            return Err(format!("second run of the same image not fresh: {again:?}"));
        }
        // A new loader sent as an image takes over; its predecessor's run is
        // no hang (2026-10-16: the device saved a 45-byte "did not return"
        // report over crashlog.txt each time a loader was tried from RAM).
        let new_loader =
            std::fs::read(&loader).map_err(|e| format!("{}: {e}", loader.display()))?;
        s.write_all(&rp_loadproto::frame(&new_loader))
            .map_err(|e| e.to_string())?;
        usb_read_line(&mut s, "RPL OK sha256 ", 15)?;
        // The new loader sends the run's log by itself once the port is open.
        let run = usb_read_until(&mut s, "RPL END", 30)?;
        if !run.contains("[the image was a new loader: it took over]")
            || run.contains("did not return")
        {
            return Err(format!("loader hand-over reported as: {run:?}"));
        }
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
    eprintln!(
        "    loader over USB OK: image in, run log out, through the emulated USB port; the same image twice, fresh both times"
    );
    Ok(())
}

/// The trace over USB (item 54): the loader runs rp-threads (which carries
/// on the ring the loader started), and after it returned, `trace` hands
/// the ring over: the image's start and end marks and its thread switches
/// must decode, in order, none torn.
pub fn boot_loader_trace(root: &Path) -> Result<(), String> {
    if !has_usb_model(root) {
        eprintln!("    trace: skipped (this emulator has no USB controller model)");
        return Ok(());
    }
    let dir = emu_dir(root)?;
    let release = crate::arm_release(root);
    crate::sdkit::raw_binary(&release.join("rp-loader"), &dir.join("rp-loader.bin"))?;
    let threads = dir.join("rp-threads.bin");
    crate::sdkit::raw_binary(&release.join("rp-threads"), &threads)?;
    let payload = std::fs::read(&threads).map_err(|e| format!("{}: {e}", threads.display()))?;
    let frame = rp_loadproto::frame(&payload);
    let loader = dir.join("rp-loader.bin");
    let args = usb_args(11);
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let check = || -> Result<(), String> {
        let mut s = usb_connect(11)?;
        usb_read_until(&mut s, "RPLOAD1 READY", 10)?;
        s.write_all(&frame).map_err(|e| e.to_string())?;
        let log = usb_read_until(&mut s, "RPL END", 60)?;
        if !log.contains(THREADS_MARKER) {
            return Err(format!("rp-threads didn't pass under the loader: {log:?}"));
        }
        s.write_all(&rp_loadproto::control_frame("trace"))
            .map_err(|e| e.to_string())?;
        let reply = usb_read_until(&mut s, " kept\r\n", 20)?;
        let lines: Vec<String> = reply
            .lines()
            .filter_map(|l| l.trim_end().strip_prefix("RPD "))
            .map(str::to_string)
            .collect();
        let words = crate::trace::parse(&lines)?;
        let (total, events) =
            rp_kernel_trace::decode_words(&words).ok_or("trace: no ring in the reply")?;
        let events: Vec<rp_kernel_trace::Event> = events.collect();
        use rp_kernel_trace::Kind;
        let marks: Vec<(u32, u32)> = events
            .iter()
            .filter(|e| e.kind == Kind::Mark)
            .map(|e| (e.a, e.b))
            .collect();
        let switches = events.iter().filter(|e| e.kind == Kind::Switch).count();
        let in_order = events.windows(2).all(|w| w[1].seq == w[0].seq + 1);
        // Marks: the image's start (1) and its end with every check passed
        // (2, 1); if the ring wrapped, the start may be gone.
        let ended = marks.last() == Some(&(2, 1));
        let started = marks.first() == Some(&(1, 0)) || u64::from(total) > events.len() as u64;
        if !(ended && started && switches > 10 && in_order) {
            return Err(format!(
                "trace: total {total}, {} kept, marks {marks:?}, {switches} switches, in order {in_order}",
                events.len()
            ));
        }
        eprintln!(
            "    trace OK: {total} events from rp-threads under the loader ({switches} switches kept), decoded on the PC"
        );
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
            timeout: Duration::from_secs(90),
        },
    )?;
    Ok(())
}

/// A card image as large as a real card, as a sparse file: only the
/// formatter's non-zero parts are written (a few MB however large the
/// card). On Windows the file is marked sparse before it is sized, so NTFS
/// never fills in the zeros, even when the emulator writes near the end.
fn sparse_card(path: &Path, img: &rp_fat::format::Image<'_>) -> Result<(), String> {
    use std::io::{Seek, SeekFrom, Write};
    let err = |e: std::io::Error| format!("{}: {e}", path.display());
    let _ = std::fs::remove_file(path);
    std::fs::File::create(path).map_err(err)?;
    #[cfg(windows)]
    run(Command::new("fsutil")
        .args(["sparse", "setflag"])
        .arg(path)
        .stdout(std::process::Stdio::null()))?;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(err)?;
    f.set_len(u64::from(img.total_sectors) * 512).map_err(err)?;
    let mut failed = None;
    rp_fat::format::write_sparse(img, &mut |off, data| {
        if failed.is_none()
            && let Err(e) = f.seek(SeekFrom::Start(off)).and_then(|_| f.write_all(data))
        {
            failed = Some(e);
        }
    });
    failed.map_or(Ok(()), |e| Err(err(e)))
}

/// Tests at the owner's card sizes (item 57): the loader with a 4 GB card
/// (4 KiB clusters, a million of them) and a 32 GB card (16 KiB clusters,
/// two million), each 90% full of ballast (allocated clusters, chained in
/// the FAT) so the long commands do their full-size work: start-up (mount
/// and boot-file guard), `card check` (every chain, both FATs), `put` (free
/// space found past the ballast), the last blocks scanned, and the drive's
/// capacity and last block. Each must finish, with the loader answering,
/// inside the run limit: slow-at-full-size problems (the watchdog resets of
/// 2026-10-11/12) show up here before the device.
pub fn boot_loader_full_size(root: &Path) -> Result<(), String> {
    if !has_usb_model(root) || !has_msc_model(root) {
        eprintln!("    full-size cards: skipped (no USB or mass storage model)");
        return Ok(());
    }
    let dir = emu_dir(root)?;
    let loader = dir.join("rp-loader.bin");
    crate::sdkit::raw_binary(&crate::arm_release(root).join("rp-loader"), &loader)?;
    let note: Vec<u8> = (0..20_000u32).map(|i| (i * 7 + 1) as u8).collect();
    let put = rp_loadproto::file_frame("full-size.txt", &note);
    for (label, gib, spc) in [("4 GB", 4u64, 8u32), ("32 GB", 32, 32)] {
        let bytes = gib << 30;
        let sectors = (bytes / 512) as u32;
        let cluster = u64::from(spc) * 512;
        let ballast = bytes * 9 / 10 / cluster * cluster;
        let image = scratch(root, &format!("full-{gib}g.img"))?;
        sparse_card(
            &image,
            &rp_fat::format::Image {
                total_sectors: sectors,
                sectors_per_cluster: spc,
                files: &[rp_fat::format::Entry {
                    name: "hello.txt",
                    data: b"a card as big as the owner's",
                }],
                blocked_clusters: &[],
                ballast,
            },
        )?;
        // FSInfo's free count and next-free hint as "unknown" (many cards
        // carry them so), so `put` has to find the free space by scanning
        // the FAT past the ballast: the slow path, at full size.
        {
            use std::io::{Seek, SeekFrom, Write};
            let fsinfo = u64::from(rp_fat::format::PARTITION_START + 1) * 512;
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .open(&image)
                .map_err(|e| format!("{}: {e}", image.display()))?;
            f.seek(SeekFrom::Start(fsinfo + 488))
                .and_then(|_| f.write_all(&[0xFF; 8]))
                .map_err(|e| format!("{}: {e}", image.display()))?;
        }
        let mut args: Vec<String> = usb_args(12).into();
        args.extend(msc_args(13));
        let extra: Vec<&str> = args.iter().map(String::as_str).collect();
        let check = || -> Result<(), String> {
            let started = Instant::now();
            let mut s = usb_connect(12)?;
            usb_read_until(&mut s, "RPLOAD1 READY", 120)?;
            let ready = started.elapsed();
            let mut step =
                |cmd: &[u8], want: &str, what: &str| -> Result<(String, Duration), String> {
                    let t = Instant::now();
                    s.write_all(cmd).map_err(|e| e.to_string())?;
                    // The reply line (the idle loader's READY lines don't
                    // start with "RPL ").
                    let fail = |e: String| format!("{label} card, {what}: {e}");
                    let mut got = usb_read_until(&mut s, "RPL ", 300).map_err(fail)?;
                    while !got.rsplit("RPL ").next().unwrap_or("").contains("\r\n") {
                        got += &usb_read_until(&mut s, "\r\n", 30).map_err(fail)?;
                    }
                    let tail = got.rsplit("RPL ").next().unwrap_or("");
                    let reply = format!("RPL {}", tail.split("\r\n").next().unwrap_or(""));
                    if !reply.contains(want) {
                        return Err(format!("{label} card, {what}: {reply:?}"));
                    }
                    Ok((reply, t.elapsed()))
                };
            let check_cmd = rp_loadproto::control_frame("card check");
            let (_, check1) = step(&check_cmd, "RPL OK check clean", "card check")?;
            let (_, put_t) = step(&put, "RPL OK 20000 bytes", "put past the ballast")?;
            let (after, check2) = step(&check_cmd, "RPL OK check clean", "card check after put")?;
            if !after.contains("unchecked 0)") {
                return Err(format!("{label} card: clusters left unchecked: {after:?}"));
            }
            let last = sectors - 4096;
            let scan = rp_loadproto::control_frame(&format!("card scan {last} 4096"));
            let (_, scan_t) = step(&scan, "4096: 0 failed", "scan of the last blocks")?;
            // The PC's view: the whole card, its last block readable.
            let mut bot = Bot::connect(13)?;
            bot.mount()?;
            let deadline = Instant::now() + Duration::from_secs(30);
            while bot.command(&[0; 6], 0, false, &[])?.status != 0 {
                let _ = bot.sense()?;
                if Instant::now() > deadline {
                    return Err(format!("{label} card: the drive never became ready"));
                }
                std::thread::sleep(Duration::from_millis(250));
            }
            let r = bot.command(&[0x25, 0, 0, 0, 0, 0, 0, 0, 0, 0], 8, true, &[])?;
            if r.data.get(..4) != Some(&(sectors - 1).to_be_bytes()[..]) {
                return Err(format!("{label} card: capacity {:?}", r.data));
            }
            let mut block = [0u8; 512];
            rp_hal_api::BlockDevice::read_blocks(&mut bot, sectors - 1, &mut block)
                .map_err(|_| format!("{label} card: last block unreadable over the drive"))?;
            eprintln!(
                "    {label} card OK: ready {:.1} s, check {:.1} s, put {:.1} s, check {:.1} s, end scan {:.1} s; drive capacity and last block",
                ready.as_secs_f32(),
                check1.as_secs_f32(),
                put_t.as_secs_f32(),
                check2.as_secs_f32(),
                scan_t.as_secs_f32()
            );
            Ok(())
        };
        let result = boot(
            root,
            &Boot {
                image: &loader,
                marker: LOADER_MARKER,
                screenshot: None,
                sd_image: Some(&image),
                steps: &[],
                extra: &extra,
                input: None,
                during: Some(&check),
                then: None,
                timeout: Duration::from_secs(900),
            },
        );
        let _ = std::fs::remove_file(&image);
        result?;
    }
    Ok(())
}

/// A disk image in memory, for checking the emulator's card afterwards.
struct Disk(Vec<u8>);

impl rp_hal_api::BlockDevice for Disk {
    fn read_blocks(&mut self, lba: u32, buf: &mut [u8]) -> Result<(), rp_hal_api::BlockError> {
        let o = lba as usize * 512;
        if o + buf.len() > self.0.len() {
            return Err(rp_hal_api::BlockError::OutOfRange);
        }
        buf.copy_from_slice(&self.0[o..o + buf.len()]);
        Ok(())
    }
    fn write_blocks(&mut self, lba: u32, buf: &[u8]) -> Result<(), rp_hal_api::BlockError> {
        let o = lba as usize * 512;
        if o + buf.len() > self.0.len() {
            return Err(rp_hal_api::BlockError::OutOfRange);
        }
        self.0[o..o + buf.len()].copy_from_slice(buf);
        Ok(())
    }
}

/// A test card for the card log: FAT32 with someone else's file on it
/// (`other`), and `seeded` old `rplog/light-NNNN.txt` files (for the
/// cleanup that keeps each image's newest `KEEP`).
fn cardlog_card(path: &Path, other: &[u8], seeded: u32) -> Result<(), String> {
    let card = rp_fat::format::build(&rp_fat::format::Image {
        total_sectors: 64 * 1024,
        sectors_per_cluster: 8,
        files: &[rp_fat::format::Entry {
            name: "other.txt",
            data: other,
        }],
        blocked_clusters: &[],
        ballast: 0,
    });
    let mut vol = rp_fat::AnyVolume::mount(Disk(card), rp_fat::format::PARTITION_START)
        .map_err(|e| format!("card image: {}", e.name()))?;
    let mut scratch = vec![0u8; 64 * 1024];
    for n in 1..=seeded {
        vol.put(&format!("rplog/light-{n:04}.txt"), b"old", &mut scratch)
            .map_err(|e| format!("seeding the card: {}", e.name()))?;
    }
    std::fs::write(path, &vol.device().0).map_err(|e| format!("{}: {e}", path.display()))
}

/// The files in `folder` on a card image (names only).
fn card_names(image: &Path, folder: &str) -> Result<Vec<String>, String> {
    let data = std::fs::read(image).map_err(|e| format!("{}: {e}", image.display()))?;
    let mut vol = rp_fat::AnyVolume::mount(Disk(data), rp_fat::format::PARTITION_START)
        .map_err(|e| format!("card image: {}", e.name()))?;
    let mut names = Vec::new();
    vol.list(folder, &mut |n, dir, _| {
        if !dir {
            names.push(String::from_utf8_lossy(n).into_owned());
        }
    })
    .map_err(|e| format!("{folder}: {}", e.name()))?;
    Ok(names)
}

/// The run log as the PC received it (after the last `RPL LOG`), with
/// `\n` line ends, the notes the loader adds after the image's text left
/// out.
fn received_log(output: &str) -> Result<String, String> {
    let (_, log) = output.rsplit_once("RPL LOG").ok_or("no run log")?;
    let log = log.split("RPL END").next().unwrap_or(log);
    let text = log.replace("\r\n", "\n");
    let text = text.trim_start_matches('\n');
    let end = text
        .find("\n[the image ")
        .or_else(|| text.find("\n[run log full"))
        .unwrap_or(text.len());
    Ok(text[..end].trim_end_matches('\n').to_string())
}

/// A run's card log: both cards' files of `file` joined, as `cargo xtask
/// cardlog` joins them.
fn joined_card_log(sd: &Path, cf: &Path, file: &str) -> Result<String, String> {
    let sd_part = card_file(sd, file).ok();
    let cf_part = card_file(cf, file).ok();
    if sd_part.is_none() && cf_part.is_none() {
        return Err(format!("{file} is on neither card"));
    }
    for part in [&sd_part, &cf_part].into_iter().flatten() {
        if part.len() != rp_hal_x51v::cardlog::FILE_BYTES {
            return Err(format!(
                "{file} is {} bytes, reserved {}",
                part.len(),
                rp_hal_x51v::cardlog::FILE_BYTES
            ));
        }
    }
    let bytes: Vec<u8> =
        rp_hal_x51v::cardlog::join(sd_part.as_deref(), cf_part.as_deref()).collect();
    Ok(String::from_utf8_lossy(&bytes)
        .replace("\r\n", "\n")
        .trim_end_matches('\n')
        .to_string())
}

/// The card log for every run the loader starts (quality audit C24, the
/// owner's vote B): with an SD and a CF card in, each run's log is on the
/// cards line for line as the run goes, joined it is exactly the run log
/// the PC receives (the loader's closing line included), and nothing
/// outside `rplog/` changes on either card.
///
/// - rp-light: on the SD card; the CF card's file stays unwritten. The
///   number follows the 40 seeded files (41), and the oldest beyond the
///   newest 32 are deleted, 8 a run.
/// - rp-sdtime takes the SD controller: the log moves to the CF card with
///   a line saying so, and the two halves join into the whole log.
/// - rp-crash: closed as crashed.
/// - An image that hangs with interrupts off: the hardware watchdog resets
///   the machine (RAM kept, as a soft reset keeps it), and the loader closes
///   the log at its next start as a run that didn't return.
pub fn boot_loader_cardlog(root: &Path) -> Result<(), String> {
    // A failure's whole text (the card's log with it) kept in a file: a
    // rare failure was lost to a filtered terminal once (2026-10-22, the
    // sdtime guard, 1 run in 10; quality audit C60).
    cardlog_checks(root).map_err(|e| {
        let kept = scratch(root, "cardlog-failure.txt")
            .ok()
            .filter(|p| std::fs::write(p, &e).is_ok());
        match kept {
            Some(p) => format!(
                "{e}
(kept in {})",
                p.display()
            ),
            None => e,
        }
    })
}

fn cardlog_checks(root: &Path) -> Result<(), String> {
    let loader = raw_image(root, "rp-loader")?;
    let other = b"another session's file: must stay as it is".to_vec();
    let sd = scratch(root, "sd-cardlog.img")?;
    let cf = scratch(root, "cf-cardlog.img")?;
    cardlog_card(&sd, &other, 40)?;
    cardlog_card(&cf, &other, 0)?;
    let cf_drive = drive_arg(root, "ide", &cf)?;
    let run =
        |name: &str, payload: &[u8], limit: Option<u32>, until: &str| -> Result<String, String> {
            let mut input = rp_loadproto::control_frame(&format!("name {name}"));
            if let Some(s) = limit {
                input.extend(rp_loadproto::control_frame(&format!("limit {s}")));
            }
            input.extend(rp_loadproto::frame(payload));
            let output = boot(
                root,
                &Boot {
                    image: &loader,
                    marker: LOADER_MARKER,
                    screenshot: None,
                    sd_image: Some(&sd),
                    steps: &[],
                    extra: &["-drive", &cf_drive],
                    input: Some((&input, until)),
                    during: None,
                    then: Some("RPL END"),
                    timeout: Duration::from_secs(600),
                },
            )?;
            if !output.contains("RPL OK name") {
                return Err(format!("cardlog {name}: the loader didn't take the name"));
            }
            Ok(output)
        };
    let check = |name: &str, number: u32, output: &str, how: &str| -> Result<String, String> {
        let file = format!("rplog/{name}-{number:04}.txt");
        let card = joined_card_log(&sd, &cf, &file).map_err(|e| format!("cardlog {name}: {e}"))?;
        let log = received_log(output).map_err(|e| format!("cardlog {name}: {e}"))?;
        if card != log {
            return Err(format!(
                "cardlog {name}: {file} differs from the run log:\n--- card ---\n{card}\n--- run log ---\n{log}"
            ));
        }
        let first = format!("[loader: card log {file} on the SD and CF cards]");
        if !card.contains(&first) {
            return Err(format!("cardlog {name}: no `{first}` in:\n{card}"));
        }
        let closing = card.lines().last().unwrap_or("");
        if !closing.starts_with(&format!("[card log closed: the run {how}"))
            || !closing.contains(" 0 refused")
        {
            return Err(format!(
                "cardlog {name}: the closing line is wrong: {closing}"
            ));
        }
        // Written as the run went, not only by the loader at its end (a
        // hang that loses the RAM leaves only what was written as it went):
        // a sector write a line, so at least as many writes as the lines
        // before the closing line, but for the loader's first two. Since
        // quality audit C59 a line that ends while the card still programs
        // goes with the next write: the emulated SD card never programs (a
        // write a line still), the CF card is busy for a moment after each
        // (rp-sdtime's part folds lines: 47 writes for 54, its guard run 40
        // for 101), so there at least one write for every four lines. A
        // ratio, not the intent: the proper check counts the program's
        // writes apart from the loader's (quality audit C59).
        let writes: usize = closing
            .split(" sector writes")
            .next()
            .and_then(|h| h.rsplit(' ').next())
            .and_then(|n| n.parse().ok())
            .ok_or_else(|| format!("cardlog {name}: no write count in: {closing}"))?;
        let lines = card.lines().count() - 1;
        let floor = if card.contains("continuing on the CF card") {
            lines / 4
        } else {
            lines
        };
        if writes + 2 < floor {
            return Err(format!(
                "cardlog {name}: {writes} sector writes for {lines} lines: the lines weren't written as the run went"
            ));
        }
        Ok(card)
    };
    let image = |n: &str| -> Result<Vec<u8>, String> {
        let p = raw_image(root, n)?;
        std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))
    };

    let out = run("light", &image("rp-light")?, None, "RPL END")?;
    check("light", 41, &out, "returned")?;
    if card_file(&cf, "rplog/light-0041.txt")?
        .iter()
        .any(|&b| b != 0)
    {
        return Err(
            "cardlog light: the CF card's file was written though the SD card was free".into(),
        );
    }
    let mut lights: Vec<String> = card_names(&sd, "rplog")?
        .into_iter()
        .filter(|n| n.starts_with("light-"))
        .collect();
    lights.sort();
    if lights.len() != 33 || lights[0] != "light-0009.txt" {
        return Err(format!(
            "cardlog light: the SD card should keep light-0009..0041 (the 8 oldest of those beyond 32 deleted): {lights:?}"
        ));
    }
    eprintln!(
        "    cardlog light OK: the run log on the SD card line for line, numbered 41, the 8 oldest beyond 32 deleted"
    );

    let out = run("sdtime", &image("rp-sdtime")?, None, "RPL END")?;
    let card = check("sdtime", 1, &out, "returned")?;
    let moved = "[card log: the SD card was taken by the program; continuing on the CF card]";
    if card.matches(moved).count() != 1 {
        return Err(format!("cardlog sdtime: no `{moved}` once:\n{card}"));
    }
    if card_file(&cf, "rplog/sdtime-0001.txt")?
        .iter()
        .all(|&b| b == 0)
    {
        return Err("cardlog sdtime: nothing on the CF card".into());
    }
    eprintln!(
        "    cardlog sdtime OK: the SD test moved the log to the CF card; the halves join into the run log"
    );

    // rp-sdtime's guard run (quality audit C32): block numbers off the
    // card refused by the card-log writer and the driver, nothing written.
    let guard = variant_image(root, "rp-sdtime", ("RP_SDTIME_GUARD", "1"))?;
    let guard = std::fs::read(&guard).map_err(|e| format!("{}: {e}", guard.display()))?;
    let out = run("sdtime", &guard, None, "RPL END")?;
    let card = check("sdtime", 2, &out, "returned")?;
    for want in [
        "G1 the card-log writer: the loader handed over ",
        "ok  the loader's capacity is the driver's",
        "ok  the last block as it was",
        "G7a: OUT_OF_RANGE in ",
        "  G7b wait 10000 us: read Ok(())",
        "RETROPANE-SDTIME: the guard run: ",
    ] {
        if !card.contains(want) {
            return Err(format!(
                "cardlog sdtime guard: no `{want}`:
{card}"
            ));
        }
    }
    // Version 6 (C46's follow-up): every wait the driver counted has its
    // own `end wait` line, so an unseen one is named by its step.
    let counted = card
        .split("the driver collected after ")
        .nth(1)
        .and_then(|r| r.split_whitespace().next())
        .and_then(|n| n.parse::<usize>().ok())
        .ok_or_else(|| {
            format!(
                "cardlog sdtime guard: no count of the driver's waits:
{card}"
            )
        })?;
    let logged = card
        .lines()
        .filter(|l| l.starts_with("  end wait "))
        .count();
    if counted == 0 || logged != counted {
        return Err(format!(
            "cardlog sdtime guard: {logged} `end wait` lines for the driver's {counted} waits:
{card}"
        ));
    }
    if card.contains("  BAD ") || !card.contains(" checks, 0 BAD") {
        return Err(format!(
            "cardlog sdtime guard: a check failed:
{card}"
        ));
    }
    eprintln!(
        "    cardlog sdtime guard OK: every block off the card refused before a command, the last block as it was, each end-of-card wait logged"
    );

    let out = run("crash", &image("rp-crash")?, None, "RPL END")?;
    check("crash", 1, &out, "crashed")?;
    eprintln!("    cardlog crash OK: closed as crashed");

    // mov r0, #1; mcr p14, 0, r0, c0, c1, 0 (PMNC: count); b .
    let words = [0xE3A0_0001u32, 0xEE00_0E11, 0xEAFF_FFFE];
    let code: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    let out = run("hang", &code, Some(1), "did not return")?;
    check("hang", 1, &out, "did not return")?;
    eprintln!(
        "    cardlog hang OK: a watchdog reset with RAM kept, closed at the loader's next start"
    );

    for (card, what) in [(&sd, "SD"), (&cf, "CF")] {
        if card_file(card, "other.txt")? != other {
            return Err(format!(
                "cardlog: the card log changed a file outside rplog/ on the {what} card"
            ));
        }
    }
    eprintln!("    cardlog OK: nothing outside rplog/ changed on either card");
    Ok(())
}

/// A gpubench run's card log (the loader's, `rp_hal_x51v::cardlog`)
/// against its run log: the run's own number, word for word the run log
/// the PC received (from the loader's first line to its closing line),
/// closed as a run that returned, no write refused.
fn gpubench_card_log(sd: &Path, name: &str, number: u32, log: &str) -> Result<(), String> {
    let file = format!("rplog/gpubench-{number:04}.txt");
    let raw = card_file(sd, &file).map_err(|e| format!("gpubench {name}: card log: {e}"))?;
    if raw.len() != rp_hal_x51v::cardlog::FILE_BYTES {
        return Err(format!(
            "gpubench {name}: {file} is {} bytes, reserved {}",
            raw.len(),
            rp_hal_x51v::cardlog::FILE_BYTES
        ));
    }
    let card: Vec<u8> = rp_hal_x51v::cardlog::join(Some(&raw), None).collect();
    let card = String::from_utf8_lossy(&card)
        .replace("\r\n", "\n")
        .trim_end_matches('\n')
        .to_string();
    let log = received_log(&format!("RPL LOG{log}"))?;
    if card != log {
        return Err(format!(
            "gpubench {name}: {file} differs from the run log:\n--- card ---\n{card}\n--- run log ---\n{log}"
        ));
    }
    let closing = card.lines().last().unwrap_or("");
    if !closing.starts_with("[card log closed: the run returned") || !closing.contains(" 0 refused")
    {
        return Err(format!(
            "gpubench {name}: {file} doesn't end with a clean closing line: {closing}"
        ));
    }
    Ok(())
}

/// Reads root-directory file `name` from a FAT32 disk image.
fn card_file(image: &Path, name: &str) -> Result<Vec<u8>, String> {
    let data = std::fs::read(image).map_err(|e| format!("{}: {e}", image.display()))?;
    // FAT32 or exFAT, whichever the image holds (both formatters put the
    // partition at the same place).
    let mut vol = rp_fat::AnyVolume::mount(Disk(data), rp_fat::format::PARTITION_START)
        .map_err(|e| format!("card image: {}", e.name()))?;
    let mut out = Vec::new();
    let mut scratch = vec![0u8; 64 * 1024];
    vol.read_back(name, &mut scratch, &mut |c| out.extend_from_slice(c))
        .map_err(|e| format!("{name}: {}", e.name()))?;
    Ok(out)
}

/// A `-drive` argument for `path` on `interface`, as the emulator in use
/// sees paths (WSL or native).
fn drive_arg(root: &Path, interface: &str, path: &Path) -> Result<String, String> {
    let p = match emulator(root)? {
        Emulator::Wsl => to_wsl_path(path),
        Emulator::Native(_) => path.display().to_string(),
    };
    Ok(format!("if={interface},format=raw,file={p}"))
}

/// `cargo xtask emu-storfast`: the storage speed-ups' device test
/// (`rp-storfast`, docs/research/storage-speed.md) started by the loader
/// with a 128 MiB FAT32 SD card and a CF card, as on the device. The card
/// swap and lock steps aren't modelled (the image waits for them and
/// carries on); everything else must pass: CF paths equal, 20 attach
/// cycles, SD reads of every size class by one DMA transfer, the DMA write
/// soak exact, the test files removed.
pub fn boot_loader_storfast(root: &Path) -> Result<(), String> {
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let loader = dir.join("rp-loader.bin");
    let image = dir.join("rp-storfast.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-storfast"), &image)?;
    let sd = scratch(root, "sd-storfast.img")?;
    let card = rp_fat::format::build(&rp_fat::format::Image {
        total_sectors: 128 * 2048,
        sectors_per_cluster: 1,
        // The quick-run file: the emulated FIFO moves bytes slowly.
        files: &[
            rp_fat::format::Entry {
                name: "retropane.bin",
                data: b"kit stand-in",
            },
            rp_fat::format::Entry {
                name: "storfast-quick.txt",
                data: b"short run",
            },
        ],
        blocked_clusters: &[],
        ballast: 0,
    });
    std::fs::write(&sd, &card).map_err(|e| format!("{}: {e}", sd.display()))?;
    let cf = scratch(root, "cf-storfast.img")?;
    let cf_data: Vec<u8> = (0..16u32 << 20)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
        .collect();
    std::fs::write(&cf, cf_data).map_err(|e| format!("{}: {e}", cf.display()))?;
    let payload = std::fs::read(&image).map_err(|e| format!("{}: {e}", image.display()))?;
    let input = rp_loadproto::frame(&payload);
    let cf_drive = drive_arg(root, "ide", &cf)?;
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: Some(&sd),
            steps: &[],
            extra: &["-drive", &cf_drive],
            input: Some((&input, "RPL END")),
            during: None,
            then: None,
            timeout: Duration::from_secs(900),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("storfast: no run log")?;
    if crashed(log) || !log.contains("RETROPANE-STORFAST: done") {
        return Err(format!("storfast: didn't finish cleanly:{log}"));
    }
    let dma_window = emulator_accepts_global(root, "pxa2xx-pcmcia.word-cycles=on");
    let mut want = vec![
        "storfast-quick.txt found",
        "C3 3 attach cycles: all good",
        "C4 tune (measured, ",
        "the card's limit ",
        "C6 Windows Mobile's timing put back: MCMEM1 0x00020508",
        "KiB): all right, one transfer each",
        "S2 the speed writes read back: 0 blocks differ",
        "of the last writes read back, 0 blocks differ",
        "S9 test files and folder removed",
    ];
    if dma_window {
        want.push("reads by DMA from the window (0 paths refused)");
        want.push("C2 read 1024 KiB by DMA from the window: ");
    }
    for w in want {
        if !log.contains(w) {
            return Err(format!("storfast: no `{w}` in the run log:{log}"));
        }
    }
    for line in log
        .lines()
        .filter(|l| l.starts_with("S3") && l.contains("blocks differ"))
    {
        if !line.ends_with(" 0 blocks differ") {
            return Err(format!("storfast: {line}"));
        }
    }
    for line in log.lines().filter(|l| {
        l.starts_with("S2 soak") || l.starts_with("C2 read") || l.starts_with("C4 own timing")
    }) {
        let clean = line.contains(" 0 write errors, 0 read errors, 0 blocks differ")
            || line.contains("0 commands failed, 0 sectors differ")
            || line.ends_with(", 0 failed, 0 sectors differ");
        if !clean {
            return Err(format!("storfast: {line}"));
        }
    }
    eprintln!(
        "    storfast OK (short run): CF read paths equal, attach cycles good; SD every size class by one DMA transfer; the DMA write soak read back exact; test files removed; run log:{log}"
    );
    Ok(())
}

/// `cargo xtask emu-cf`: the CompactFlash check on its own.
pub fn boot_loader_cf_alone(root: &Path) -> Result<(), String> {
    let dir = emu_dir(root)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    crate::sdkit::raw_binary(
        &crate::arm_release(root).join("rp-loader"),
        &dir.join("rp-loader.bin"),
    )?;
    boot_loader_cf(root)
}

/// The CompactFlash slot (docs/hw/cf.md): a CF card image in socket 1 and
/// no SD card, so the CF path stands alone. The emulated slot starts
/// powered, as Windows Mobile hands it over (its CPLD table, measured);
/// the loader attaches the card on its own (powered through the CPLD,
/// reset, identified) before it
/// offers the drive, so the PC's first question finds the second drive
/// ready; `cf` reports it (capacity, write cache
/// off, partition) without taking it again; the CPLD writes leave the
/// backlight on (its registers don't read back, docs/hw/cpld.md); pulled
/// out, the slot is switched off, and pushed back in, attached again.
pub fn boot_loader_cf(root: &Path) -> Result<(), String> {
    if !has_usb_model(root) {
        eprintln!("    CF card: skipped (this emulator has no USB controller model)");
        return Ok(());
    }
    let loader = raw_image(root, "rp-loader")?;
    let image = rp_fat::format::build(&rp_fat::format::Image {
        total_sectors: 32 * 1024, // 16 MiB
        sectors_per_cluster: 8,
        files: &[rp_fat::format::Entry {
            name: "on-cf.txt",
            data: b"compact flash",
        }],
        blocked_clusters: &[],
        ballast: 0,
    });
    let cf = scratch(root, "cf.img")?;
    std::fs::write(&cf, image).map_err(|e| format!("{}: {e}", cf.display()))?;
    let mut args: Vec<String> = usb_args(2).into();
    args.extend(msc_args(3));
    args.push("-drive".into());
    args.push(drive_arg(root, "ide", &cf)?);
    args.extend(wm_cpld_table_args());
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let via_drive: Vec<u8> = (0..4096u32).map(|i| (i * 29 + 3) as u8).collect();
    let check = || -> Result<(), String> {
        let mut s = usb_connect(2)?;
        let mut bot = Bot::connect(3)?;
        bot.lun = 1;
        usb_read_until(&mut s, "RPLOAD1 READY", 10)?;
        // The slot starts powered with its card ready, as Windows Mobile
        // hands it over: the attach is the reset alone, not the 0.4 s power
        // cycle (boot time; the device's CF stage was 457 ms with it).
        s.write_all(&rp_loadproto::control_frame("handoff"))
            .map_err(|e| e.to_string())?;
        let stages = usb_read_until(
            &mut s, " ms
", 10,
        )?;
        let at = |key: &str| -> Option<u32> {
            stages
                .split(key)
                .nth(1)?
                .split(" ms")
                .next()?
                .trim()
                .parse()
                .ok()
        };
        // The board line reaches the PC (audit C57: serial only before).
        s.write_all(&rp_loadproto::control_frame("board"))
            .map_err(|e| e.to_string())?;
        let board = usb_read_until(
            &mut s, "
", 10,
        )?;
        if !board.contains("RPL OK board: recorded as Windows Mobile left it") {
            return Err(format!(
                "the loader's board line didn't reach the PC: {board:?}"
            ));
        }
        match (at("dma+fat "), at("cf ")) {
            (Some(before), Some(cf)) if cf - before < 200 => {}
            _ => {
                return Err(format!(
                    "CF attach at start-up power-cycled a powered slot: {stages:?}"
                ));
            }
        }
        // Nothing asked: the loader attaches the card in the slot itself,
        // and the PC's second drive becomes ready (after "medium changed").
        let ready = |bot: &mut Bot, what: &str| -> Result<(), String> {
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                let r = bot.command(&[0; 6], 0, false, &[])?;
                if r.status == 0 {
                    return Ok(());
                }
                let _ = bot.sense()?;
                if Instant::now() > deadline {
                    return Err(format!("CF unit never became ready ({what})"));
                }
                std::thread::sleep(Duration::from_millis(200));
            }
        };
        // The card in the slot at start-up is attached before the PC can
        // see the drive: a fresh mount's first question (READ CAPACITY)
        // finds it, with no "no medium" or "medium changed" first (the
        // owner's flaky "USB Drive (J:)": Explorer drew the drive empty, its
        // icon minutes later).
        let first = bot.command(&[0x25, 0, 0, 0, 0, 0, 0, 0, 0, 0], 8, true, &[])?;
        if first.status != 0 {
            let sense = bot.sense()?;
            return Err(format!(
                "CF unit not ready at the PC's first question (sense {sense:?}): the card in the slot at start-up must be attached before the drive is offered"
            ));
        }
        // `cf` reports the card the loader attached, without taking it again.
        s.write_all(&rp_loadproto::control_frame("cf"))
            .map_err(|e| e.to_string())?;
        let reply = usb_read_line(&mut s, "RPL ", 20)?;
        // Reads come by DMA from the card's data window once IDENTIFY read
        // through it matched the 16-bit port (docs/research/storage-speed.md):
        // on a fork modelling the PXA's two card cycles per 32-bit access
        // (rp-v0.17.0's `word-cycles`). On one that doesn't, the window
        // reads wrong and the driver must keep the port: the check is the
        // guard, so a wrong window never reaches a sector.
        let path = if emulator_accepts_global(root, "pxa2xx-pcmcia.word-cycles=on") {
            "reads by DMA from the window (0 paths refused)"
        } else {
            "reads by 16-bit port (1 paths refused)"
        };
        let want = "RPL OK cf: 16 MB (32768 sectors), write cache off, ";
        if !reply.contains(want)
            || !reply.contains(path)
            || !reply.contains("FAT32 partition")
            || !reply.contains("was card")
        {
            return Err(format!("unexpected reply to cf (want {path:?}): {reply:?}"));
        }
        // Within a second of the attach the loader measures the card's own
        // timing (ruling M, R7): the emulator's card reads right at every
        // timing (timing isn't modelled), so it must be applied.
        // Asked again every 200 ms (no fixed wait), for up to 10 s.
        let mut reply = String::new();
        for _ in 0..50 {
            s.write_all(&rp_loadproto::control_frame("cf"))
                .map_err(|e| e.to_string())?;
            reply = usb_read_line(&mut s, "RPL ", 20)?;
            if reply.contains("; timing own, ") {
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        if !reply.contains("; timing own, ") || !reply.contains("writes at Windows Mobile's") {
            return Err(format!("cf: the card's own timing not measured: {reply:?}"));
        }
        // The power sequence and the card's reset went through the CPLD,
        // whose registers don't read back: the backlight enable sharing
        // register 0x18 with RESET must still be on.
        let mut mon = crate::screen::Monitor::connect(monitor_port())?;
        if device_has_runtime(&mut mon, "/machine/peripheral/mbx2700g", "backlight") {
            let lit = mon.query("qom-get /machine/peripheral/mbx2700g backlight")?;
            if lit != "true" {
                return Err(format!("CF: the backlight went off ({lit})"));
            }
        }
        // The shadow came from Windows Mobile's table, and the bits it had
        // on in the registers the CF attach wrote are still on (item 77):
        // SD power (0x16 bit 0) and the backlight (0x18 bit 2) beside the
        // slot's own.
        s.write_all(&rp_loadproto::control_frame("cpld"))
            .map_err(|e| e.to_string())?;
        let mut reply = usb_read_until(&mut s, "shadow from", 10)?;
        if !reply.rsplit("shadow from").next().is_some_and(|t| {
            t.contains(
                "
",
            )
        }) {
            reply += &usb_read_until(
                &mut s, "
", 10,
            )?;
        }
        for want in [
            "shadow from WM's table:",
            " 02=00fd",
            " 10=1000",
            " 12=000f",
            " 16=0003",
            " 18=0004",
            " 1c=000c",
        ] {
            if !reply.contains(want) {
                return Err(format!(
                    "CF: the CPLD's shadow isn't Windows Mobile's table with its bits kept (no {want:?}): {reply:?}"
                ));
            }
        }
        let r = bot.command(&[0x12, 0, 0, 0, 36, 0], 36, true, &[])?;
        if r.data.get(16..23) != Some(&b"CF card"[..]) {
            return Err(format!("CF unit: INQUIRY {:?}", r.data));
        }
        let r = bot.command(&[0x25, 0, 0, 0, 0, 0, 0, 0, 0, 0], 8, true, &[])?;
        if r.data.get(..4) != Some(&32767u32.to_be_bytes()[..]) {
            return Err(format!("CF unit: capacity {:?}", r.data));
        }
        let mut mbr = [0u8; 512];
        rp_hal_api::BlockDevice::read_blocks(&mut bot, 0, &mut mbr)
            .map_err(|_| "CF unit: sector 0 unreadable".to_string())?;
        if mbr[510..] != [0x55, 0xAA] {
            return Err("CF unit: sector 0 isn't the image's MBR".into());
        }
        // Read-only until the owner says otherwise...
        if rp_hal_api::BlockDevice::write_blocks(&mut bot, 30000, &via_drive).is_ok() {
            return Err("CF unit: written while read-only".into());
        }
        let _ = bot.sense()?;
        s.write_all(&rp_loadproto::control_frame("drive rw"))
            .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL OK drive read-write", 10)?;
        let deadline = Instant::now() + Duration::from_secs(10);
        while rp_hal_api::BlockDevice::write_blocks(&mut bot, 30000, &via_drive).is_err() {
            let _ = bot.sense()?; // medium changed, once
            if Instant::now() > deadline {
                return Err("CF unit: write refused after drive rw".into());
            }
        }
        let mut back = vec![0u8; via_drive.len()];
        rp_hal_api::BlockDevice::read_blocks(&mut bot, 30000, &mut back)
            .map_err(|_| "CF unit: read-back failed".to_string())?;
        if back != via_drive {
            return Err("CF unit: read-back differs".into());
        }
        // Pulled out: the loader lets the card go and switches the slot
        // off; pushed back in, it is attached again from a power cycle.
        if device_has_runtime(&mut mon, "/machine/peripheral/cfslot", "pressed") {
            mon.query("qom-set /machine/peripheral/cfslot pressed true")?;
            // The reply ends its event list with ",\r\n" (the idle loader's
            // READY lines don't).
            let mut cf_log = || -> Result<String, String> {
                s.write_all(&rp_loadproto::control_frame("cf log"))
                    .map_err(|e| e.to_string())?;
                usb_read_until(&mut s, ",\r\n", 10)
            };
            // The loader notices on its detect poll (debounced, in emulated
            // seconds, slower than ours on a busy PC): poll, never a guess.
            let mut poll = |what: &str, done: &dyn Fn(&str) -> bool| -> Result<String, String> {
                let deadline = Instant::now() + Duration::from_secs(30);
                loop {
                    let log = cf_log()?;
                    if done(&log) {
                        return Ok(log);
                    }
                    if Instant::now() > deadline {
                        return Err(format!("CF {what}: {log:?}"));
                    }
                    poll_pause(Duration::from_millis(500));
                }
            };
            poll("removed: the slot was never powered off", &|l| {
                l.contains("power off")
            })?;
            mon.query("qom-set /machine/peripheral/cfslot pressed false")?;
            let log = poll("re-inserted: never attached again", &|l| {
                l.rsplit("power off")
                    .next()
                    .is_some_and(|a| a.contains("attached"))
            })?;
            // Ordered events, no timing: a slot left powered would show
            // READY as soon as the card went back in, before the loader's
            // debounced attach (which powers it from off itself).
            let after_off = log.rsplit("power off").next().unwrap_or_default();
            let before_attach = after_off.split("attached").next().unwrap_or_default();
            if before_attach.contains("ready 1") {
                return Err(format!("CF removed: slot not powered off: {log:?}"));
            }
            ready(&mut bot, "re-inserted card")?;
            let mut mbr = [0u8; 512];
            rp_hal_api::BlockDevice::read_blocks(&mut bot, 0, &mut mbr)
                .map_err(|_| "CF re-inserted: sector 0 unreadable".to_string())?;
        }
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
    let after = std::fs::read(&cf).map_err(|e| e.to_string())?;
    if after[30000 * 512..30000 * 512 + via_drive.len()] != via_drive[..] {
        return Err("CF: the drive's write isn't in the card image".into());
    }
    eprintln!(
        "    CF card OK: attached on its own as Windows Mobile left the slot, before the drive is offered (ready at the PC's first question), powered through the CPLD with the backlight kept on; identified in socket 1 (16 MB, write cache off) by cf without a remount, the PC's second drive: read, refused while read-only, written after drive rw, checked in the image; pulled out: slot powered off, attached again on re-insert (no READY before)"
    );
    Ok(())
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

/// Audio: `rp-tone` under the loader over USB (WM8750 over I2C, I2S fed by
/// a DMA ring): 440 Hz for 500 ms to the speaker, then, with the headphones
/// plugged in while it waits, 1000 Hz for 50 ms to them. The emulator records
/// to a wav file, and the first tone's pitch is measured from its zero
/// crossings.
pub fn boot_loader_tone(root: &Path) -> Result<(), String> {
    if !has_usb_model(root) {
        eprintln!("    tone: skipped (this emulator has no USB controller model)");
        return Ok(());
    }
    let loader = raw_image(root, "rp-loader")?;
    let image = raw_image(root, "rp-tone")?;
    let payload = std::fs::read(&image).map_err(|e| format!("{}: {e}", image.display()))?;
    let frame = rp_loadproto::frame(&payload);
    let wav = scratch(root, "tone.wav")?;
    let _ = std::fs::remove_file(&wav);
    let wav_path = match emulator(root)? {
        Emulator::Wsl => to_wsl_path(&wav),
        Emulator::Native(_) => wav.display().to_string(),
    };
    let port = monitor_port();
    let mut args: Vec<String> = usb_args(2).into();
    args.extend([
        "-audiodev".into(),
        format!("wav,id=snd,path={wav_path}{WAV_VOICE}"),
        "-machine".into(),
        "audiodev=snd".into(),
    ]);
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let check = || -> Result<(), String> {
        let mut s = usb_connect(2)?;
        usb_read_until(&mut s, "RPLOAD1 READY", 10)?;
        s.write_all(&frame).map_err(|e| e.to_string())?;
        usb_read_line(&mut s, "RPL OK sha256 ", 15)?;
        // Headphones in while the image waits for the jack (after its
        // first tone): the second tone goes to them.
        guest_hold(
            &mut crate::screen::Monitor::connect(monitor_port())?,
            4000,
            "tone",
        )?;
        let mut mon = crate::screen::Monitor::connect(port)?;
        // An emulator with the audio wiring as measured under Windows
        // Mobile (rp-v0.23.0, `codec-power`: the codec answers only while
        // CPLD 0x1C bit 1 powers it, the jack on GPIO83 high when plugged in)
        // checks the jack too; an older one modelled the jack on GPIO96,
        // which the image no longer reads, and only the first tone is
        // checked there.
        let wired = mon.query("qom-get /machine codec-power")?.contains("true");
        mon.query("qom-set /machine/peripheral/jack pressed true")?;
        let log = usb_read_until(&mut s, "RPL END", 40)?;
        let mut wants = vec![
            "probe standard, as run 2",
            "tone 440 Hz 1000 ms (48000 frames",
            "tone 660 Hz 1000 ms (48000 frames",
            "tone 880 Hz 1000 ms (48000 frames",
            "by DMA",
            "to speaker",
            "codec at 0x1a",
            "speaker amplifier on (CPLD 14 bit 1)",
            "codec power put back off",
            "TONE-OK",
        ];
        // A release with the measured I2S restart rule (rp-v0.24.0) drops
        // the switch-on straight after a stop, as the X51v does.
        if mon.query("qom-get /machine i2s-restart")?.contains("true") {
            wants.push("I2S switched on again at once after a stop: taken false");
        }
        if wired {
            wants.extend([
                "headphones out (GPIO83 0; GPIO96 1)",
                "headphones in (GPIO83 1)",
                "speaker amplifier off (headphones)",
                "PLUG IN THE HEADPHONES NOW (30 s)",
                "to headphones",
            ]);
        } else {
            eprintln!(
                "    tone: this emulator's jack is the old GPIO96 model (before rp-v0.23.0): the jack part is skipped"
            );
        }
        for want in wants {
            if !log.contains(want) {
                return Err(format!("tone: no {want:?} in the run log:{log}"));
            }
        }
        if let Some(line) = log.lines().find(|l| l.starts_with("tone 440")) {
            eprintln!("    rp-tone: {}", line.trim());
        }
        // Let the backend write out what it has.
        guest_hold(&mut mon, 500, "tone")?;
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
            timeout: Duration::from_secs(90),
        },
    )?;
    let (found, seconds) = first_notes(&wav, &[440.0, 660.0, 880.0], "tone")?;
    eprintln!(
        "    tone OK: WM8750 found on I2C, three notes through I2S, recorded at {:.1}, {:.1} and {:.1} Hz over {seconds:.3} s",
        found[0], found[1], found[2]
    );
    Ok(())
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
fn variant_image_envs(root: &Path, image: &str, envs: &[(&str, &str)]) -> Result<PathBuf, String> {
    let name = envs
        .iter()
        .map(|e| e.0)
        .collect::<Vec<_>>()
        .join("-")
        .to_ascii_lowercase();
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

/// The codec's sleep (ADR-0032 R5, S1b): `rp-audio-s1` built with
/// `RP_AUDIO_SLEEP`. After a note the codec sleeps within the quiet time
/// with I2S still running; a sound with no wake first wakes it from sleep
/// (VMID and the amplifier: ~150 ms); three taps on the screen wake it as
/// the stylus lands, so each click starts right at the lift.
pub fn boot_loader_audio_sleep(root: &Path) -> Result<(), String> {
    if !has_usb_model(root) {
        eprintln!("    audio sleep: skipped (this emulator has no USB controller model)");
        return Ok(());
    }
    let loader = raw_image(root, "rp-loader")?;
    let image = variant_image(root, "rp-audio-s1", ("RP_AUDIO_SLEEP", "1"))?;
    let payload = std::fs::read(&image).map_err(|e| format!("{}: {e}", image.display()))?;
    let frame = rp_loadproto::frame(&payload);
    let port = monitor_port();
    let args: Vec<String> = usb_args(2).into();
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let check = || -> Result<(), String> {
        let mut s = usb_connect(2)?;
        usb_read_until(&mut s, "RPLOAD1 READY", 10)?;
        let mut mon = crate::screen::Monitor::connect(port)?;
        mon.query("qom-set /machine/peripheral/jack pressed false")?;
        let mut qmp = crate::screen::Qmp::connect(port + 1)?;
        s.write_all(&frame).map_err(|e| e.to_string())?;
        usb_read_line(&mut s, "RPL OK sha256 ", 15)?;
        // Three taps, each held 4 s (the image waits up to 30 s for each,
        // so a slow emulator still finds the stylus down), 12 s apart.
        let centre = (0x4000, 0x4000);
        guest_hold(&mut mon, 20000, "audio-sleep")?;
        for _ in 0..3 {
            qmp.touch(Some(centre))?;
            guest_hold(&mut mon, 4000, "audio-sleep")?;
            qmp.touch(None)?;
            guest_hold(&mut mon, 8000, "audio-sleep")?;
        }
        let log = usb_read_until(&mut s, "RPL END", 60)?;
        for want in [
            "SLEEP TEST: HEADPHONES OUT",
            "TEST 1:",
            "TEST 2: A NOTE WITH NO TOUCH FIRST",
            "TEST 5:",
            "DONE: DID YOU HEAR 2 NOTES AND 3 CLICKS?",
            "board bits put back as found",
            "AUDIO-S1-OK",
        ] {
            if !log.contains(want) {
                return Err(format!("audio sleep: no {want:?} in the run log:{log}"));
            }
        }
        let asleep = log.matches("codec asleep").count();
        let running = log.matches("I2S still running: true").count();
        if asleep != 5 || running != 5 {
            return Err(format!(
                "audio sleep: {asleep} sleeps, {running} with I2S running (wanted 5 and 5):{log}"
            ));
        }
        let woke = log.matches("(from sleep: true)").count();
        if woke != 4 {
            return Err(format!(
                "audio sleep: {woke} wakes from sleep (wanted 4: the alarm-style note and three taps):{log}"
            ));
        }
        // Each click starts right at the lift: the codec was already up.
        for l in log.lines().filter(|l| l.contains("the click started")) {
            let ms: u32 = l
                .split("started ")
                .nth(1)
                .and_then(|r| r.split(' ').next())
                .and_then(|n| n.parse().ok())
                .ok_or(format!("audio sleep: unreadable click line {l:?}"))?;
            if ms > 30 {
                return Err(format!(
                    "audio sleep: a click started {ms} ms after the lift:{log}"
                ));
            }
        }
        if log.contains("didn't sleep") || log.contains("no tap in 30 s") {
            return Err(format!("audio sleep: a step didn't happen:{log}"));
        }
        if let Some(l) = log.lines().find(|l| l.contains("the chip woke in")) {
            eprintln!("    audio sleep: {}", l.trim());
        }
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
            timeout: Duration::from_secs(150),
        },
    )?;
    eprintln!(
        "    audio sleep OK: five sleeps with I2S running, four wakes from sleep, every click at the lift"
    );
    Ok(())
}

/// ADR-0032 A6's device bench (`rp-tone` built with `RP_TONE_DMA_RESTART`):
/// the audio DMA channel stops at its chain's STOP link while I2S runs,
/// stays stopped 0-1,000 ms, and is restarted as option B would restart
/// it. Here every try must stop at the chain's end, move again, finish and
/// leave I2S on; the timings are the device's to measure (the emulator's
/// DMA restarts in no time), so none is checked. The codec is off, so the
/// link must drain on its own clock as on the X51v (`i2s-free-run`,
/// rp-v0.27.0+); an older emulator never moves the DMA, and is skipped.
pub fn boot_loader_dma_restart(root: &Path) -> Result<(), String> {
    if !has_usb_model(root) {
        eprintln!("    dma restart: skipped (this emulator has no USB controller model)");
        return Ok(());
    }
    let loader = raw_image(root, "rp-loader")?;
    let image = variant_image(root, "rp-tone", ("RP_TONE_DMA_RESTART", "1"))?;
    let payload = std::fs::read(&image).map_err(|e| format!("{}: {e}", image.display()))?;
    let frame = rp_loadproto::frame(&payload);
    // The model's I2S drains its FIFO only into an audio backend (without
    // one the DMA moves a burst and waits): a recording nobody checks.
    let wav = scratch(root, "dma-restart.wav")?;
    let _ = std::fs::remove_file(&wav);
    let wav_path = match emulator(root)? {
        Emulator::Wsl => to_wsl_path(&wav),
        Emulator::Native(_) => wav.display().to_string(),
    };
    let mut args: Vec<String> = usb_args(2).into();
    args.extend([
        "-audiodev".into(),
        format!("wav,id=snd,path={wav_path}{WAV_VOICE}"),
        "-machine".into(),
        "audiodev=snd".into(),
    ]);
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let port = monitor_port();
    let skipped = std::cell::Cell::new(false);
    let check = || -> Result<(), String> {
        let mut s = usb_connect(2)?;
        usb_read_until(&mut s, "RPLOAD1 READY", 10)?;
        let mut mon = crate::screen::Monitor::connect(port)?;
        if !mon.query("qom-get /machine i2s-free-run")?.contains("true") {
            skipped.set(true);
            return Ok(());
        }
        s.write_all(&frame).map_err(|e| e.to_string())?;
        usb_read_line(&mut s, "RPL OK sha256 ", 15)?;
        let log = usb_read_until(&mut s, "RPL END", 90)?;
        for want in [
            "restart: I2S at 44.1 kHz",
            "restart: 15 tries",
            "RETROPANE-TONE-OK",
        ] {
            if !log.contains(want) {
                return Err(format!("dma restart: no {want:?} in the run log:{log}"));
            }
        }
        let tries = log.lines().filter(|l| l.contains(" try ")).count();
        let moved = log
            .lines()
            .filter(|l| l.contains(" try ") && l.contains("I2S on true; finished Some(true)"))
            .count();
        let chain_end = log
            .lines()
            .filter(|l| l.contains(" try ") && l.contains("stopped at byte 131072,"))
            .count();
        if tries != 15 || moved != 15 || chain_end != 15 {
            return Err(format!(
                "dma restart: {tries} tries, {moved} moved and finished with I2S on, {chain_end} stopped at the chain's end (wanted 15 each):{log}"
            ));
        }
        if log.contains("NO MOVE") || log.contains("never stopped") || log.contains("refused") {
            return Err(format!("dma restart: a try failed:{log}"));
        }
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
            timeout: Duration::from_secs(150),
        },
    )?;
    if skipped.get() {
        eprintln!(
            "    dma restart: skipped (this emulator's I2S drains only into a powered codec: before rp-v0.28.0)"
        );
        return Ok(());
    }
    eprintln!(
        "    dma restart OK: 15 stops at the chain's end with I2S running, every restart moved and finished"
    );
    Ok(())
}

/// Audio service step S1 (ADR-0032, `rp-audio-s1`): audio up at 44.1 kHz
/// through the service, three notes to the speaker (pitch checked in the
/// recording: a codec set to another rate than I2S would shift it), the
/// headphones plugged in while up (re-routed by the edge latch), down and
/// straight back up (the restart guard waits, then a note plays: the
/// emulator drops an early switch-on as the X51v does), the headphones
/// pulled while down (no edge: the jack line is low with the codec off, as
/// measured; the next power-up reads it), the board bits put back.
pub fn boot_loader_audio_s1(root: &Path) -> Result<(), String> {
    if !has_usb_model(root) {
        eprintln!("    audio S1: skipped (this emulator has no USB controller model)");
        return Ok(());
    }
    let loader = raw_image(root, "rp-loader")?;
    let image = raw_image(root, "rp-audio-s1")?;
    let payload = std::fs::read(&image).map_err(|e| format!("{}: {e}", image.display()))?;
    let frame = rp_loadproto::frame(&payload);
    let wav = scratch(root, "audio-s1.wav")?;
    let _ = std::fs::remove_file(&wav);
    let wav_path = match emulator(root)? {
        Emulator::Wsl => to_wsl_path(&wav),
        Emulator::Native(_) => wav.display().to_string(),
    };
    let port = monitor_port();
    let mut args: Vec<String> = usb_args(2).into();
    args.extend([
        "-audiodev".into(),
        format!("wav,id=snd,path={wav_path}{WAV_VOICE}"),
        "-machine".into(),
        "audiodev=snd".into(),
    ]);
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let check = || -> Result<(), String> {
        let mut s = usb_connect(2)?;
        usb_read_until(&mut s, "RPLOAD1 READY", 10)?;
        let mut mon = crate::screen::Monitor::connect(port)?;
        // The audio wiring (rp-v0.23.0+), the I2S restart rule (rp-v0.24.0+)
        // and the jack line waiting for VMID (rp-v0.25.0+): what this check is about.
        for prop in ["codec-power", "i2s-restart", "jack-vmid"] {
            if !mon
                .query(&format!("qom-get /machine {prop}"))?
                .contains("true")
            {
                return Err(format!(
                    "audio S1: this emulator lacks the {prop} model (pin rp-v0.25.0 or later)"
                ));
            }
        }
        // The owner's steps: headphones in at the start, out at step 2. At
        // step 4 the check leaves them out (the owner puts them back): the
        // model records only the speaker's voice, and the recording keeps
        // only the last power-up's sound, so the last note must go to the
        // speaker to be checked.
        mon.query("qom-set /machine/peripheral/jack pressed true")?;
        s.write_all(&frame).map_err(|e| e.to_string())?;
        usb_read_line(&mut s, "RPL OK sha256 ", 15)?;
        guest_hold(&mut mon, 7000, "audio-s1")?;
        mon.query("qom-set /machine/peripheral/jack pressed false")?;
        let log = usb_read_until(&mut s, "RPL END", 60)?;
        for want in [
            "RetroPane audio S1 (44.1 kHz",
            "STEP 1: HEADPHONES IN",
            "up (first): codec at 0x1a, to headphones, waited 0 ms",
            "note 440 Hz: 44100 frames",
            "note 880 Hz: 44100 frames",
            "STEP 2: PULL THE HEADPHONES OUT NOW",
            "jack: rerouted to speaker (edge latch, debounced)",
            "board: speaker amplifier on",
            "STEP 3: LEAVE THEM OUT",
            "up (straight after down): codec at 0x1a, to speaker",
            "STEP 4: PUT THE HEADPHONES BACK IN NOW",
            "while down: a jack edge came: false",
            "up (after the change while down): codec at 0x1a, to speaker",
            "board: speaker amplifier off",
            "DONE: 6 NOTES IN ALL",
            "board bits put back as found",
            "AUDIO-S1-OK",
        ] {
            if !log.contains(want) {
                return Err(format!("audio S1: no {want:?} in the run log:{log}"));
            }
        }
        // The guard waited (most of 700 ms: down takes a few) and the note
        // after it played.
        let waited = log
            .lines()
            .find(|l| l.starts_with("up (straight after down)"))
            .and_then(|l| l.split("waited ").nth(1))
            .and_then(|r| r.split(' ').next())
            .and_then(|n| n.parse::<u32>().ok())
            .ok_or("audio S1: no guard wait in the log")?;
        if !(500..=700).contains(&waited) {
            return Err(format!("audio S1: the restart guard waited {waited} ms"));
        }
        let played = log.matches("frames in").count();
        if played != 6 {
            return Err(format!("audio S1: {played} notes played, wanted 6:{log}"));
        }
        if log.contains("failed") || log.contains("stream ") {
            return Err(format!("audio S1: a step failed:{log}"));
        }
        guest_hold(&mut mon, 500, "audio-s1")?;
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
            timeout: Duration::from_secs(120),
        },
    )?;
    // Each power-up re-opens the emulator codec's voice, and its wav
    // backend starts the file again: the recording holds the last session,
    // its one note (880 Hz). Its pitch proves both sides at 44.1 kHz (a
    // codec left at 48 kHz would play it ~9% sharp).
    let (found, seconds) = first_notes(&wav, &[880.0], "audio S1")?;
    eprintln!(
        "    audio S1 OK: three power-ups at 44.1 kHz through the service, the last note recorded at {:.1} Hz over {seconds:.3} s; jack re-routed while up, the restart guard waited, no jack change seen for our own power-down",
        found[0]
    );
    Ok(())
}

/// A PC that stays silent after a plug-in (quality audit B1, docs/hw/udc.md
/// "unplugged, but nobody came"): the emulated host ignores the loader's
/// first plug-in (`pxa27x-udc.ignore-plugs=1`, rp-v0.26.0+). The loader's
/// retry (`pxa-udc` `unplug::Patience`: no bus reset for 10 s, unplug and
/// present again) must get it configured and talking; without the retry it
/// waits forever (an emulator mutant).
pub fn boot_loader_silent_pc(root: &Path) -> Result<(), String> {
    if !has_usb_model(root) || !device_has(root, "pxa27x-udc", "ignore-plugs") {
        eprintln!(
            "    silent PC: skipped (this emulator's USB host can't stay silent: rp-v0.26.0)"
        );
        return Ok(());
    }
    let loader = raw_image(root, "rp-loader")?;
    let mut args: Vec<String> = usb_args(15).into();
    args.extend(["-global".into(), "pxa27x-udc.ignore-plugs=1".into()]);
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let check = || -> Result<(), String> {
        // The first plug-in passes in silence; the retry comes after 10 s
        // of the loader's time without a bus reset.
        let mut s = usb_connect(15)?;
        usb_read_until(&mut s, "RPLOAD1 READY", 90)?;
        Ok(())
    };
    let output = boot(
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
            timeout: Duration::from_secs(150),
        },
    )?;
    let _ = output;
    eprintln!("    silent PC OK: the first plug-in ignored, the loader's retry got it configured");
    Ok(())
}

/// ADR-0032 A6, audit E3 (`rp-audio-s2` built with `RP_AUDIO_E3`): the
/// ring that stops at the end of the mixed sound. A: a rising scale (a
/// semitone every 100 ms), the mixer stalled 400 ms after 1 s: the
/// recording must hold the scale, one gap of silence, and the scale
/// carrying on, its notes never going down (a replay of the ring's last
/// quarter second would), none missing. B: the scale again with five wakes
/// held back until the engine is inside the end's piece (the race): the
/// notes in order with no gap. The log: the stall late once and stopped,
/// five races forced, nothing failed. The headphones are out (the model
/// records the speaker's voice).
pub fn boot_loader_audio_e3(root: &Path) -> Result<(), String> {
    if !has_usb_model(root) {
        eprintln!("    audio E3: skipped (this emulator has no USB controller model)");
        return Ok(());
    }
    let loader = raw_image(root, "rp-loader")?;
    let image = variant_image(root, "rp-audio-s2", ("RP_AUDIO_E3", "1"))?;
    let payload = std::fs::read(&image).map_err(|e| format!("{}: {e}", image.display()))?;
    let frame = rp_loadproto::frame(&payload);
    let wav = scratch(root, "audio-e3.wav")?;
    let _ = std::fs::remove_file(&wav);
    let wav_path = match emulator(root)? {
        Emulator::Wsl => to_wsl_path(&wav),
        Emulator::Native(_) => wav.display().to_string(),
    };
    let port = monitor_port();
    let mut args: Vec<String> = usb_args(2).into();
    args.extend([
        "-audiodev".into(),
        format!("wav,id=snd,path={wav_path}{WAV_VOICE}"),
        "-machine".into(),
        "audiodev=snd".into(),
    ]);
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let check = || -> Result<(), String> {
        let mut s = usb_connect(2)?;
        usb_read_until(&mut s, "RPLOAD1 READY", 10)?;
        let mut mon = crate::screen::Monitor::connect(port)?;
        mon.query("qom-set /machine/peripheral/jack pressed false")?;
        s.write_all(&frame).map_err(|e| e.to_string())?;
        usb_read_line(&mut s, "RPL OK sha256 ", 15)?;
        let log = usb_read_until(&mut s, "RPL END", 150)?;
        for want in [
            "E3 TEST A",
            "E3A RESULT",
            "E3A RING",
            "E3 TEST B",
            "E3B RESULT",
            "E3B RING",
            "E3 ALL",
            "E3 DONE",
            "board bits put back as found",
            "AUDIO-S2-OK",
        ] {
            if !log.contains(want) {
                return Err(format!("audio E3: no {want:?} in the run log:{log}"));
            }
        }
        let line = |key: &str| -> Result<&str, String> {
            log.lines()
                .find(|l| l.contains(key))
                .ok_or(format!("audio E3: no {key:?} line"))
        };
        let after = |l: &str, key: &str| -> Result<u64, String> {
            l.split(key)
                .nth(1)
                .and_then(|r| r.trim_start().split([' ', ',']).next())
                .and_then(|n| n.parse().ok())
                .ok_or(format!("audio E3: no {key:?} in {l:?}"))
        };
        let a = line("E3A RING")?;
        if after(a, "late")? != 1 || after(a, "stops")? < 1 || after(a, "restarts")? < 1 {
            return Err(format!(
                "audio E3: the stall wasn't one late wake with the engine stopped and started again: {a:?}"
            ));
        }
        let b = line("E3B RING")?;
        if after(b, "forced")? != 5 || after(b, "late")? != 0 {
            return Err(format!("audio E3: five races not forced cleanly: {b:?}"));
        }
        // No stream heard dry, in either part (A6 run 2: the scale ran dry
        // once after the stall; a dry stretch is now counted only when it
        // plays).
        for part in ["E3A RESULT", "E3B RESULT"] {
            let l = line(part)?;
            if after(l, "dry")? != 0 {
                return Err(format!("audio E3: a stream ran dry: {l:?}"));
            }
        }
        let all = line("E3 ALL")?;
        if after(a, "failed")? != 0 || after(b, "failed")? != 0 || after(all, "failed")? != 0 {
            return Err(format!("audio E3: the ring's end failed to move:{log}"));
        }
        // The engine stood stopped through the rest of the stall: 400 ms
        // less the mix it had (245 ms) and its piece's silence.
        let stall = line("  stall:")?;
        if after(stall, "end, stopped")? < 100 {
            return Err(format!(
                "audio E3: the engine stopped under 100 ms: {stall:?}"
            ));
        }
        if !line("  stall:")?.contains("had stopped") {
            return Err(format!(
                "audio E3: the engine still played at the stall's end (longer than the mix and its piece): {:?}",
                line("  stall:")?
            ));
        }
        for l in log.lines().filter(|l| {
            l.contains("E3A") || l.contains("E3B") || l.contains("stall:") || l.contains("race ")
        }) {
            eprintln!("    audio E3: {}", l.trim());
        }
        guest_hold(&mut mon, 500, "audio-e3")?;
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
            timeout: Duration::from_secs(240),
        },
    )?;
    let found = e3_recording(&wav)?;
    eprintln!("    audio E3 OK: {found}");
    Ok(())
}

/// The scale `rp-audio-s2`'s E3 run plays (a semitone every 100 ms).
const E3_SCALE: [f64; 24] = [
    440.0, 466.0, 494.0, 523.0, 554.0, 587.0, 622.0, 659.0, 698.0, 740.0, 784.0, 831.0, 880.0,
    932.0, 988.0, 1047.0, 1109.0, 1175.0, 1245.0, 1319.0, 1397.0, 1480.0, 1568.0, 1661.0,
];

/// The pitch of `x` from its first and last zero crossings, each placed
/// between its two samples (a fraction of a sample): accurate to well
/// under 1 Hz over 20 ms, where counting crossings resolves only 50 Hz
/// (coarser than a semitone).
fn fine_pitch(x: &[i16], rate: u32) -> f64 {
    let at: Vec<f64> = x
        .windows(2)
        .enumerate()
        .filter(|(_, p)| (p[0] < 0) != (p[1] < 0))
        .map(|(i, p)| {
            let (a, b) = (f64::from(p[0]), f64::from(p[1]));
            i as f64 + a / (a - b)
        })
        .collect();
    match (at.first(), at.last()) {
        (Some(f), Some(l)) if at.len() > 2 && l > f => {
            (at.len() - 1) as f64 / 2.0 / ((l - f) / f64::from(rate))
        }
        _ => 0.0,
    }
}

/// `emu-audio-e3`'s rule that a scale never goes down (quality audit
/// C22). A window raises the highest note only when the next window is the
/// same note: a window across a join (or a note's quiet onset after
/// silence) can read as any pitch, and one read too high made every window
/// of the true note after it "below" (C22's "note 1 after 3/4" at part B's
/// first window, 2026-10-22). Two windows in a row below the highest
/// confirmed note are a replay (a lap of the ring is a quarter second of
/// earlier notes, a dozen windows).
#[derive(Default)]
struct Rising {
    /// The highest note seen in two windows in a row.
    high: usize,
    /// The window before, if it was a note.
    prev: Option<usize>,
    /// Windows in a row below `high` by more than one note.
    below: usize,
}

impl Rising {
    /// A window holding note `n`: `Some(high)` if it is a replay.
    fn note(&mut self, n: usize) -> Option<usize> {
        self.below = if n + 1 < self.high { self.below + 1 } else { 0 };
        if self.below >= 2 {
            return Some(self.high);
        }
        if self.prev == Some(n) {
            self.high = self.high.max(n);
        }
        self.prev = Some(n);
        None
    }

    /// A window that is quiet or no note of the scale.
    fn other(&mut self) {
        self.prev = None;
    }
}

/// `emu-audio-e3`'s recording: 20 ms windows, each silent, a note of the
/// scale (its pitch within 2.5%, by [`fine_pitch`]), or neither (a join
/// between notes). The two scales (A, B) are the two runs of notes split
/// by the long pause; in each the notes never go down and all 24 are
/// there (one window out of order is a join, where a stop's cleared
/// frames skew the pitch: a replay of the old looping ring would be a
/// quarter second of earlier notes); B has no gap over 20 ms. A's stall
/// gap: the fork's I2S sends nothing while its FIFO is empty (the stopped
/// DMA), so the recording closes it up (docs/hw/emulator.md); the log's
/// "stopped N ms" holds it instead. Should the model send silence there,
/// A's one gap must be at least 100 ms.
fn e3_recording(wav: &Path) -> Result<String, String> {
    let (x, rate) = wav_left(wav, "audio E3")?;
    let w = rate as usize / 50;
    #[derive(Clone, Copy, PartialEq)]
    enum Win {
        Quiet,
        Note(usize),
        Other,
    }
    let wins: Vec<Win> = x
        .chunks_exact(w)
        .map(|c| {
            if rms(c) < 200.0 {
                return Win::Quiet;
            }
            let hz = fine_pitch(c, rate);
            E3_SCALE
                .iter()
                .position(|&n| (hz - n).abs() < n * 0.025)
                .map_or(Win::Other, Win::Note)
        })
        .collect();
    // Runs of notes, split where at least 600 ms is quiet (the pauses).
    let mut parts: Vec<Vec<(usize, Win)>> = Vec::new();
    let mut quiet = usize::MAX;
    for (k, &v) in wins.iter().enumerate() {
        match v {
            Win::Quiet => quiet = quiet.saturating_add(1),
            _ => {
                if quiet >= 30 {
                    parts.push(Vec::new());
                }
                quiet = 0;
            }
        }
        if let Some(p) = parts.last_mut() {
            p.push((k, v));
        }
    }
    for p in &mut parts {
        while p.last().is_some_and(|&(_, v)| v == Win::Quiet) {
            p.pop();
        }
    }
    let parts: Vec<_> = parts.into_iter().filter(|p| p.len() > 25).collect();
    if parts.len() != 2 {
        return Err(format!(
            "audio E3: the recording holds {} scales, wanted 2 (A, B)",
            parts.len()
        ));
    }
    let mut said = Vec::new();
    // In windows: A's gap at least 100 ms, B's at most 20 ms.
    for (name, p, gap_min, gap_max) in [("A", &parts[0], 5, usize::MAX), ("B", &parts[1], 0, 1)] {
        let mut rising = Rising::default();
        let mut seen = [0usize; 24];
        let (mut run, mut gaps, mut longest) = (0usize, 0usize, 0usize);
        for &(k, v) in p {
            match v {
                Win::Note(n) => {
                    if let Some(high) = rising.note(n) {
                        return Err(format!(
                            "audio E3: in {name} the scale went down at {:.2} s (note {} after {}): a replay",
                            k as f64 / 50.0,
                            n + 1,
                            high + 1
                        ));
                    }
                    seen[n] += 1;
                }
                Win::Quiet => {
                    rising.other();
                    run += 1;
                    continue;
                }
                Win::Other => rising.other(),
            }
            if run > 1 {
                gaps += usize::from(run >= gap_min.max(2));
            }
            longest = longest.max(run);
            run = 0;
        }
        let missing: Vec<usize> = (0..24).filter(|&n| seen[n] < 2).map(|n| n + 1).collect();
        if !missing.is_empty() {
            return Err(format!(
                "audio E3: in {name} notes {missing:?} are missing or short"
            ));
        }
        if name == "A" && (gaps > 1 || (gaps == 1 && longest < gap_min)) {
            return Err(format!(
                "audio E3: A holds {gaps} gaps of silence (the longest {} ms), wanted one of at least 100 ms",
                longest * 20
            ));
        }
        if longest > gap_max {
            return Err(format!(
                "audio E3: B has a gap of {} ms (the race must leave none)",
                longest * 20
            ));
        }
        said.push(format!(
            "{name} all 24 notes rising, longest silence {} ms",
            longest * 20
        ));
    }
    Ok(said.join("; "))
}

/// Audio service step S2 (ADR-0032, `rp-audio-s2`): the mixer on the
/// emulated X51v, the headphones out the whole run (the model records the
/// speaker's voice). The two notes of test 1 come from a 22.05 kHz and a
/// 48 kHz stream, the second placed to start where the first ends: both
/// must be recorded at 880 Hz, back to back (a wrong conversion ratio shifts
/// the pitch). Three taps through QMP each get a click within 10 ms; the
/// tune alone sleeps long (few wakes a second), with clicks the interval is
/// short; no wake is late and no stream runs dry anywhere; the limiter
/// works on the overload (6A) and not at half volume (6B); the board bits
/// are put back.
pub fn boot_loader_audio_s2(root: &Path) -> Result<(), String> {
    if !has_usb_model(root) {
        eprintln!("    audio S2: skipped (this emulator has no USB controller model)");
        return Ok(());
    }
    // The clicks the log says were played (taps and timed ones), for the
    // recording's check.
    let clicks_seen = std::cell::RefCell::new(0usize);
    let loader = raw_image(root, "rp-loader")?;
    let image = raw_image(root, "rp-audio-s2")?;
    let payload = std::fs::read(&image).map_err(|e| format!("{}: {e}", image.display()))?;
    let frame = rp_loadproto::frame(&payload);
    let wav = scratch(root, "audio-s2.wav")?;
    let _ = std::fs::remove_file(&wav);
    let wav_path = match emulator(root)? {
        Emulator::Wsl => to_wsl_path(&wav),
        Emulator::Native(_) => wav.display().to_string(),
    };
    let port = monitor_port();
    let mut args: Vec<String> = usb_args(2).into();
    args.extend([
        "-audiodev".into(),
        format!("wav,id=snd,path={wav_path}{WAV_VOICE}"),
        "-machine".into(),
        "audiodev=snd".into(),
    ]);
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let check = || -> Result<(), String> {
        let mut s = usb_connect(2)?;
        usb_read_until(&mut s, "RPLOAD1 READY", 10)?;
        let mut mon = crate::screen::Monitor::connect(port)?;
        mon.query("qom-set /machine/peripheral/jack pressed false")?;
        // The link at SADIV's measured rate (rp-v0.26.0+): checked when the
        // model is there.
        let rates = mon.query("qom-get /machine i2s-rates")?.contains("true");
        let mut qmp = crate::screen::Qmp::connect(port + 1)?;
        s.write_all(&frame).map_err(|e| e.to_string())?;
        usb_read_line(&mut s, "RPL OK sha256 ", 15)?;
        // Test 2 waits up to 30 s for each tap from ~8 s into the run.
        let centre = (0x4000, 0x4000);
        guest_hold(&mut mon, 12000, "audio-s2")?;
        for _ in 0..3 {
            qmp.touch(Some(centre))?;
            guest_hold(&mut mon, 2000, "audio-s2")?;
            qmp.touch(None)?;
            guest_hold(&mut mon, 3000, "audio-s2")?;
        }
        let log = usb_read_until(&mut s, "RPL END", 180)?;
        for want in [
            "RetroPane audio S2: the mixer (44.1 kHz",
            "TEST 1: TWO NOTES",
            "TEST 1 RESULT",
            "TEST 2: TAP THE SCREEN",
            "TEST 2 RESULT",
            "TEST 3 RESULT",
            "TEST 4 RESULT",
            "5A 22 kHz RESULT",
            "5B 11 kHz RESULT",
            "5C 48 kHz RESULT",
            "TEST 6: PULL THE HEADPHONES OUT NOW",
            "sound to the speaker",
            "6A RESULT",
            "6B RESULT",
            "7A RESULT",
            "7B RESULT",
            "DONE: TELL ME TESTS 1-7",
            "board bits put back as found",
            "AUDIO-S2-OK",
        ] {
            if !log.contains(want) {
                return Err(format!("audio S2: no {want:?} in the run log:{log}"));
            }
        }
        // The second note got the start it asked for, to the frame.
        let placed = log
            .lines()
            .find(|l| l.contains("second asked for"))
            .ok_or("audio S2: no placement line")?;
        let asked = placed
            .split("asked for ")
            .nth(1)
            .and_then(|r| r.split(' ').next())
            .ok_or(format!("audio S2: unreadable {placed:?}"))?;
        if !placed.contains(&format!("got Some({asked})")) {
            return Err(format!(
                "audio S2: the second note didn't start where asked: {placed:?}"
            ));
        }
        // A number after `key` in `line` (`12.3`, `4`).
        let num = |line: &str, key: &str| -> Result<f64, String> {
            line.split(key)
                .next()
                .and_then(|l| l.rsplit(' ').next())
                .and_then(|n| n.trim_end_matches('%').parse().ok())
                .ok_or(format!("audio S2: no {key:?} in {line:?}"))
        };
        let after = |line: &str, key: &str| -> Result<f64, String> {
            line.split(key)
                .nth(1)
                .and_then(|r| r.trim_start().split([' ', ',']).next())
                .and_then(|n| n.parse().ok())
                .ok_or(format!("audio S2: no {key:?} in {line:?}"))
        };
        let result = |name: &str| -> Result<&str, String> {
            log.lines()
                .find(|l| l.starts_with(&format!("{name} RESULT")))
                .ok_or(format!("audio S2: no {name} result"))
        };
        for name in [
            "TEST 1",
            "TEST 2",
            "TEST 3",
            "TEST 4",
            "5A 22 kHz",
            "5B 11 kHz",
            "5C 48 kHz",
            "6A",
            "6B",
            "7A",
            "7B",
        ] {
            let l = result(name)?;
            if after(l, "late")? != 0.0 || after(l, "dry")? != 0.0 {
                return Err(format!("audio S2: a late wake or a stream run dry: {l:?}"));
            }
        }
        let taps: Vec<&str> = log
            .lines()
            .filter(|l| l.contains("after the pen landed"))
            .collect();
        if taps.len() != 3 {
            return Err(format!(
                "audio S2: {} taps clicked, wanted 3:{log}",
                taps.len()
            ));
        }
        for l in &taps {
            let ms = after(l, "first sample")?;
            if ms > 10.0 {
                return Err(format!("audio S2: a tap's click {ms} ms late: {l:?}"));
            }
        }
        let clicks = log
            .lines()
            .find(|l| l.contains(" clicks: first sample"))
            .ok_or("audio S2: no click summary")?;
        let worst = num(clicks, " ms at worst")?;
        *clicks_seen.borrow_mut() = clicks
            .trim()
            .split(' ')
            .next()
            .and_then(|n| n.parse::<usize>().ok())
            .ok_or(format!("audio S2: unreadable {clicks:?}"))?
            + taps.len();
        if worst > 10.0 {
            return Err(format!("audio S2: a click {worst} ms late: {clicks:?}"));
        }
        let alone = num(result("TEST 3")?, " wakes/s")?;
        let busy = num(result("TEST 4")?, " wakes/s")?;
        if alone > 12.0 || busy < 40.0 {
            return Err(format!(
                "audio S2: {alone} wakes/s with the tune alone (at most 12), {busy} with clicks (at least 40)"
            ));
        }
        if after(result("6A")?, "limited")? == 0.0 {
            return Err("audio S2: the overload in 6A wasn't limited".into());
        }
        let half = result("6B")?;
        if after(half, "limited")? != 0.0 || after(half, "clipped")? != 0.0 {
            return Err(format!(
                "audio S2: half volume was limited or clipped: {half:?}"
            ));
        }
        // Test 2's 16 s: the DMA's rate against the guest's OS timer. With
        // the model it's the X51v's measured 44,317; the emulator's pacing
        // keeps it within 0.3%.
        let dma = after(result("TEST 2")?, "DMA at")?;
        if rates && !(44_190.0..=44_450.0).contains(&dma) {
            return Err(format!(
                "audio S2: the link ran at {dma} frames/s in test 2 (the X51v's 44,317, within 0.3%)"
            ));
        }
        if !rates {
            eprintln!(
                "    audio S2: the link's rate not checked (the emulator lacks i2s-rates: rp-v0.26.0)"
            );
        }
        if !log.contains(" 0 late, 0 dry, 0 clipped") || !log.contains(" failed 0") {
            return Err(format!(
                "audio S2: a late wake, a stream run dry, a clip or a ring end that failed over the whole run:{log}"
            ));
        }
        for l in log.lines().filter(|l| {
            l.contains("RESULT") || l.contains("after the pen") || l.contains(" clicks:")
        }) {
            eprintln!("    audio S2: {}", l.trim());
        }
        guest_hold(&mut mon, 500, "audio-s2")?;
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
            timeout: Duration::from_secs(240),
        },
    )?;
    let clicks_played = clicks_seen.into_inner();
    // Test 1's notes, the first sound of the run: 880 Hz from 22.05 kHz,
    // then 880 Hz from 48 kHz.
    let (found, seconds) = first_notes(&wav, &[880.0, 880.0], "audio S2")?;
    // The rest of the recording (one power-up: it holds the whole run).
    let rest = s2_recording(&wav, clicks_played)?;
    eprintln!(
        "    audio S2 OK: the two-rate notes recorded at {:.1} and {:.1} Hz over {seconds:.3} s, back to back; {rest}; taps and clicks within 10 ms; long sleeps with music alone; nothing late or dry",
        found[0], found[1]
    );
    Ok(())
}

/// A wav's left channel and its rate.
fn wav_left(wav: &Path, what: &str) -> Result<(Vec<i16>, u32), String> {
    let data = std::fs::read(wav).map_err(|e| format!("{}: {e}", wav.display()))?;
    if data.len() < 44 || &data[..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err(format!("{what}: the emulator wrote no wav"));
    }
    let rate = u32::from_le_bytes([data[24], data[25], data[26], data[27]]);
    let channels = usize::from(u16::from_le_bytes([data[22], data[23]]));
    let left = data[44..]
        .chunks_exact(2 * channels)
        .map(|f| i16::from_le_bytes([f[0], f[1]]))
        .collect();
    Ok((left, rate))
}

/// The stretches of sound in `x` (loud samples with less than `gap`
/// samples of quiet inside), as ranges.
fn stretches(x: &[i16], gap: usize) -> Vec<std::ops::Range<usize>> {
    let loud = |s: i16| s.unsigned_abs() > 2000;
    let mut out = Vec::new();
    let mut start = None;
    let mut last = 0;
    for (k, &s) in x.iter().enumerate() {
        if loud(s) {
            if start.is_none() {
                start = Some(k);
            }
            last = k;
        } else if let Some(st) = start
            && k - last > gap
        {
            out.push(st..last + 1);
            start = None;
        }
    }
    if let Some(st) = start {
        out.push(st..last + 1);
    }
    out
}

/// The pitch of `x` by zero crossings.
fn pitch(x: &[i16], rate: u32) -> f64 {
    let crossings = x.windows(2).filter(|p| (p[0] < 0) != (p[1] < 0)).count();
    crossings as f64 / 2.0 / (x.len() as f64 / f64::from(rate))
}

fn rms(x: &[i16]) -> f64 {
    (x.iter().map(|&s| f64::from(s) * f64::from(s)).sum::<f64>() / x.len().max(1) as f64).sqrt()
}

/// `emu-audio-s2`'s recording past test 1 (quality audit C13): nine long
/// stretches of sound (tests 1, 3, 5A-5C, 6A, 6B, 7A, 7B; the quieter tunes
/// under tests 2 and 4 fall below the threshold) and one short one per
/// click the log says was played (`clicks`). The tune (659, 784, 880, 988 Hz... a
/// quarter second each) from 22.05, 11.025 and 48 kHz (5A-5C) at its
/// pitch, note by note; 6A (two loud parts, the limiter) with no flat tops
/// and no louder than 6B (the same at half volume) by more than the
/// limiter's headroom; 7A (the speaker filter on) quieter than 7B on the
/// low tune's first note (330 Hz, where the filter cuts about 2 dB) and the
/// same on its last (659 Hz).
fn s2_recording(wav: &Path, clicks: usize) -> Result<String, String> {
    let (x, rate) = wav_left(wav, "audio S2")?;
    let all = stretches(&x, rate as usize / 10);
    let (parts, short): (Vec<_>, Vec<_>) = all
        .iter()
        .cloned()
        .partition(|r| r.len() > rate as usize / 2);
    if parts.len() != 9 || short.len() != clicks {
        let secs: Vec<String> = all
            .iter()
            .map(|r| format!("{:.2}", r.len() as f64 / f64::from(rate)))
            .collect();
        return Err(format!(
            "audio S2: the recording has {} long stretches (wanted 9) and {} clicks (the log played {clicks}); seconds: {}",
            parts.len(),
            short.len(),
            secs.join(", ")
        ));
    }
    let note = rate as usize / 4;
    // The middle 60% of note `k` of a stretch.
    let at = |r: &std::ops::Range<usize>, k: usize| {
        let s = r.start + k * note;
        &x[s + note / 5..s + note * 4 / 5]
    };
    const TUNE: [f64; 8] = [659.0, 784.0, 880.0, 988.0, 1175.0, 988.0, 880.0, 784.0];
    for (name, r) in [("5A", &parts[2]), ("5B", &parts[3]), ("5C", &parts[4])] {
        for (k, &want) in TUNE.iter().enumerate() {
            let hz = pitch(at(r, k), rate);
            if (hz - want).abs() > want * 0.02 {
                return Err(format!(
                    "audio S2: {name} note {} recorded at {hz:.1} Hz, wanted {want} Hz",
                    k + 1
                ));
            }
        }
    }
    // 6A: limited, not clipped: no runs of equal samples at its peak.
    let (a, b) = (&x[parts[5].clone()], &x[parts[6].clone()]);
    let peak = |v: &[i16]| v.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
    let (pa, pb) = (peak(a), peak(b));
    let flat = a
        .windows(3)
        .filter(|w| w[0] == w[1] && w[1] == w[2] && w[0].unsigned_abs() >= pa - pa / 64)
        .count();
    if flat > 0 || f64::from(pa) > f64::from(pb) * 1.15 {
        return Err(format!(
            "audio S2: 6A peaks {pa} against 6B's {pb}, {flat} flat tops (clipped)"
        ));
    }
    // 7A against 7B: the filter's cut at 330 Hz, nothing at 659 Hz.
    let (fa, fb) = (&parts[7], &parts[8]);
    let low = 20.0 * (rms(at(fa, 0)) / rms(at(fb, 0))).log10();
    let high = 20.0 * (rms(at(fa, 5)) / rms(at(fb, 5))).log10();
    if !(-3.5..=-1.0).contains(&low) || high.abs() > 0.8 {
        return Err(format!(
            "audio S2: the speaker filter took {low:.1} dB at 330 Hz and {high:.1} dB at 659 Hz (wanted about -2.3 and 0)"
        ));
    }
    Ok(format!(
        "{clicks} clicks recorded; the tune at pitch from 22.05, 11.025 and 48 kHz; 6A limited without a flat top (peak {pa}, 6B {pb}); the speaker filter {low:.1} dB at 330 Hz, {high:.1} dB at 659 Hz"
    ))
}

/// The pause between two looks of a wait loop (a screen, a register, a
/// log polled until it shows what's waited for): never a wait by itself
/// (quality audit C2's guard counts sleeps outside this).
fn poll_pause(d: Duration) {
    std::thread::sleep(d);
}

/// Waits until the guest's own clock (OSCR0, 3.25 MHz, read through `mon`,
/// the check's monitor connection: QEMU's monitor takes one client at a
/// time, so a hold never opens its own while the check holds one) has
/// moved `ms` on: "let the image run this long" in
/// its time, not the PC's (a busy PC slows the guest; quality audit C2).
/// An error, naming `what`, if the guest's clock stops for a minute.
fn guest_hold(mon: &mut crate::screen::Monitor, ms: u32, what: &str) -> Result<(), String> {
    let mut clock = GuestClock::start(mon, what)?;
    while clock.ms()? < u64::from(ms) {
        poll_pause(Duration::from_millis(100));
    }
    Ok(())
}

/// The guest's time since [`GuestClock::start`], from its OS timer (OSCR0,
/// 3.25 MHz, read through the monitor), past the counter's wrap.
struct GuestClock<'m> {
    mon: &'m mut crate::screen::Monitor,
    what: String,
    last: u32,
    ticks: u64,
    stuck: Instant,
}

impl<'m> GuestClock<'m> {
    fn start(mon: &'m mut crate::screen::Monitor, what: &str) -> Result<Self, String> {
        let mut c = Self {
            mon,
            what: what.to_string(),
            last: 0,
            ticks: 0,
            // A guest whose clock hasn't moved by then has stopped (a hung
            // emulator, not slow guest work).
            stuck: Instant::now() + Duration::from_secs(60),
        };
        c.last = c.oscr()?;
        Ok(c)
    }

    fn oscr(&mut self) -> Result<u32, String> {
        // A busy emulator sometimes misses one answer: asked again before
        // it counts (a stopped guest is caught by `ms`'s minute).
        let mut tries = 0;
        let out = loop {
            match self.mon.query("xp /1wx 0x40a00010") {
                Ok(o) => break o,
                Err(e) if tries >= 3 => return Err(e),
                Err(_) => tries += 1,
            }
        };
        let hex = out
            .rsplit("0x")
            .next()
            .map(|h| h.trim().trim_end_matches(|c: char| !c.is_ascii_hexdigit()))
            .unwrap_or("");
        u32::from_str_radix(hex, 16).map_err(|_| format!("{}: OSCR0 read: {out}", self.what))
    }

    /// Milliseconds of guest time so far; an error if the guest's clock
    /// hasn't moved for a minute.
    fn ms(&mut self) -> Result<u64, String> {
        let now = self.oscr()?;
        let step = now.wrapping_sub(self.last);
        if step > 0 {
            self.stuck = Instant::now() + Duration::from_secs(60);
        } else if Instant::now() > self.stuck {
            return Err(format!("{}: the guest's clock stopped", self.what));
        }
        self.ticks += u64::from(step);
        self.last = now;
        Ok(self.ticks / 3_250)
    }
}

/// The first stretch of sound in an emulator recording (`wav`): `want`'s
/// notes back to back, a second each. Their measured frequencies (within
/// 4% of `want`, else an error naming `what`) and the stretch's length.
fn first_notes(wav: &Path, want: &[f64], what: &str) -> Result<(Vec<f64>, f64), String> {
    let data = std::fs::read(wav).map_err(|e| format!("{}: {e}", wav.display()))?;
    if data.len() < 44 || &data[..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err(format!("{what}: the emulator wrote no wav"));
    }
    let rate = u32::from_le_bytes([data[24], data[25], data[26], data[27]]);
    let channels = usize::from(u16::from_le_bytes([data[22], data[23]]));
    let left: Vec<i16> = data[44..]
        .chunks_exact(2 * channels)
        .map(|f| i16::from_le_bytes([f[0], f[1]]))
        .collect();
    // The notes: the stretch between the first and last loud samples.
    let loud = |s: &i16| s.unsigned_abs() > 2000;
    let Some(start) = left.iter().position(loud) else {
        return Err(format!("{what}: the recording is silent"));
    };
    // They end at the first tenth of a second of quiet (later sounds come
    // seconds after).
    let gap = (rate / 10) as usize;
    let mut end = start;
    let mut quiet = 0;
    for (k, s) in left.iter().enumerate().skip(start) {
        if loud(s) {
            end = k;
            quiet = 0;
        } else {
            quiet += 1;
            if quiet > gap {
                break;
            }
        }
    }
    let tones = &left[start..=end];
    let seconds = tones.len() as f64 / f64::from(rate);
    let n = want.len() as f64;
    if !((n - 0.2)..=(n + 0.4)).contains(&seconds) {
        return Err(format!(
            "{what}: recorded {seconds:.3} s of sound (wanted {} notes of 1 s)",
            want.len()
        ));
    }
    // Each note, its middle 80% (away from the joins).
    let part_len = tones.len() / want.len();
    let mut found = Vec::new();
    for (k, &w) in want.iter().enumerate() {
        let part = &tones[k * part_len + part_len / 10..(k + 1) * part_len - part_len / 10];
        let crossings = part.windows(2).filter(|p| (p[0] < 0) != (p[1] < 0)).count();
        let hz = crossings as f64 / 2.0 / (part.len() as f64 / f64::from(rate));
        if (hz - w).abs() > w * 0.04 {
            return Err(format!(
                "{what}: note {} recorded at {hz:.1} Hz, wanted {w} Hz",
                k + 1
            ));
        }
        found.push(hz);
    }
    Ok((found, seconds))
}

/// Writing files to an exFAT card (SDXC) through the loader: a file
/// replaced, one created, one put into folders that didn't exist,
/// everything checked in the image afterwards, and the volume left clean
/// (VolumeDirty clear).
pub fn boot_loader_put_exfat(root: &Path) -> Result<(), String> {
    if !has_usb_model(root) {
        eprintln!("    exFAT put: skipped (this emulator has no USB controller model)");
        return Ok(());
    }
    let loader = raw_image(root, "rp-loader")?;
    let old: Vec<u8> = (0..3000u32).map(|i| (i * 3) as u8).collect();
    let new: Vec<u8> = (0..70_000u32).map(|i| (i * 17 + 1) as u8).collect();
    let created: Vec<u8> = (0..9000u32).map(|i| (i * 7 + 2) as u8).collect();
    let script = b"set ramaddr 0xa8000000\r\n".to_vec();
    let image = rp_fat::exfat::format::build(
        32 * 1024, // 16 MiB
        3,         // 4 KiB clusters
        &[
            rp_fat::exfat::format::Entry {
                name: "retropane.bin",
                data: &old,
                fragment: false,
            },
            rp_fat::exfat::format::Entry {
                name: "startup.txt",
                data: &script,
                fragment: true,
            },
        ],
    );
    let sd = scratch(root, "sd-exfat.img")?;
    std::fs::write(&sd, image).map_err(|e| format!("{}: {e}", sd.display()))?;
    let args = usb_args(2);
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let check = || -> Result<(), String> {
        let mut s = usb_connect(2)?;
        usb_read_until(&mut s, "RPLOAD1 READY", 10)?;
        s.write_all(&rp_loadproto::file_frame("retropane.bin", &new))
            .map_err(|e| e.to_string())?;
        let reply = usb_read_line(&mut s, "RPL ", 40)?;
        let line = reply
            .lines()
            .find(|l| l.starts_with("RPL "))
            .unwrap_or("")
            .to_string();
        let theirs = line.rsplit("sha256 ").next().unwrap_or("").trim();
        crate::usb::check_digest(theirs, &new, "file on the card")?;
        if !line.starts_with("RPL OK 70000 bytes") || !line.contains("contiguous") {
            return Err(format!("unexpected reply to the exFAT put: {line:?}"));
        }
        eprintln!("    loader (exFAT): {line}");
        s.write_all(&rp_loadproto::file_frame("New File.dat", &created))
            .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL OK 9000 bytes", 40)?;
        // Into folders that don't exist yet: made on the way.
        s.write_all(&rp_loadproto::file_frame("Apps/Sub Folder/x.txt", b"x"))
            .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL OK 1 bytes", 20)?;
        s.write_all(&rp_loadproto::control_frame("card check"))
            .map_err(|e| e.to_string())?;
        let reply = usb_read_line(&mut s, "RPL ", 40)?;
        if !reply.contains("RPL OK check clean") || !reply.contains(" 0 lost") {
            return Err(format!("exFAT card check after the writes: {reply:?}"));
        }
        Ok(())
    };
    boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: Some(&sd),
            steps: &[],
            extra: &extra,
            input: None,
            during: Some(&check),
            then: None,
            timeout: Duration::from_secs(90),
        },
    )?;
    if card_file(&sd, "retropane.bin")? != new {
        return Err("exFAT: retropane.bin on the card isn't what was sent".into());
    }
    if card_file(&sd, "NEW FILE.DAT")? != created {
        return Err("exFAT: the created file isn't on the card".into());
    }
    if card_file(&sd, "startup.txt")? != script {
        return Err("exFAT: startup.txt changed".into());
    }
    if card_file(&sd, "apps/sub folder/X.TXT")? != b"x" {
        return Err("exFAT: the file in new folders isn't on the card".into());
    }
    let after = std::fs::read(&sd).map_err(|e| e.to_string())?;
    let flags = after[rp_fat::exfat::format::PARTITION_START as usize * 512 + 106];
    if flags & 0x02 != 0 {
        return Err("exFAT: the volume was left marked dirty".into());
    }
    eprintln!(
        "    exFAT put OK: replaced, created, and put into new folders through the loader; volume clean"
    );
    Ok(())
}

/// Most read commands `boot_loader_put`'s write may take (FAT search in
/// runs; see docs/perf.md).
const MAX_PUT_READS: u32 = 15;

/// Writing a file to the card through the loader (ADR-0013): a FAT32 card
/// image with an old `retropane.bin`, a bigger new one sent over the
/// emulated USB port, the loader's OK, and the image checked on the host
/// afterwards: the new contents, and the other file untouched. Most of the
/// card is taken behind the FSInfo hint, so the free-cluster scan walks
/// ~90% of the FAT first (the stale-hint case that cost 3.7 s on the
/// device); the loader reports its read commands.
pub fn boot_loader_put(root: &Path) -> Result<(), String> {
    if !has_usb_model(root) {
        eprintln!("    file to card: skipped (this emulator has no USB controller model)");
        return Ok(());
    }
    let loader = raw_image(root, "rp-loader")?;
    let old: Vec<u8> = (0..3000u32).map(|i| (i * 7) as u8).collect();
    let new: Vec<u8> = (0..70_000u32).map(|i| (i * 13 + 5) as u8).collect();
    let script = b"set ramaddr 0xa8000000\r\n".to_vec();
    // Clusters 8..3600 of ~4,080 in use, the hint (cluster 5) before them.
    let taken: Vec<u32> = (8..3600).collect();
    let image = rp_fat::format::build(&rp_fat::format::Image {
        total_sectors: 32 * 1024, // 16 MiB
        sectors_per_cluster: 8,   // 4 KiB clusters, as on the owner's card
        files: &[
            rp_fat::format::Entry {
                name: "retropane.bin",
                data: &old,
            },
            rp_fat::format::Entry {
                name: "startup.txt",
                data: &script,
            },
        ],
        blocked_clusters: &taken,
        ballast: 0,
    });
    let sd = scratch(root, "sd-put.img")?;
    // A damaged first-FAT sector in a free region (sector 20: clusters
    // 2560-2687, all free): invalid entries, as on the owner's card. The
    // loader's `card repair` must restore it from the second FAT.
    let mut image = image;
    let fat1 = rp_fat::format::PARTITION_START + 32;
    let damaged = fat1 + 20;
    for (k, b) in image[damaged as usize * 512..damaged as usize * 512 + 512]
        .iter_mut()
        .enumerate()
    {
        *b = if k % 4 == 3 { 0x0A } else { 0x80 | k as u8 };
    }
    // ... with one entry landing in range by chance, as on the owner's card.
    let e5 = damaged as usize * 512 + 5 * 4;
    image[e5..e5 + 4].copy_from_slice(&0x0000_0123u32.to_le_bytes());
    std::fs::write(&sd, image).map_err(|e| format!("{}: {e}", sd.display()))?;
    let frame = rp_loadproto::file_frame("retropane.bin", &new);
    // A file the card doesn't have: created (long name, so the loader
    // writes long-name entries and a ~1 short name).
    let created: Vec<u8> = (0..6000u32).map(|i| (i * 5 + 3) as u8).collect();
    let create = rp_loadproto::file_frame("RetroPane loader.lnk", &created);
    let bad = rp_loadproto::file_frame("bad*name", b"x");
    let args = usb_args(2);
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let check = || -> Result<(), String> {
        let mut s = usb_connect(2)?;
        usb_read_until(&mut s, "RPLOAD1 READY", 10)?;
        // No CF card in this machine: the slot reads empty (CPLD 0x04 bit 1)
        // and the socket is never touched.
        s.write_all(&rp_loadproto::control_frame("cf"))
            .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL ERR cf: no card\r\n", 20)?;
        // SD DMA: reads and writes on after the start-up self-test (writes
        // since rp-storfast's device soak); `card dma writes` still answers;
        // the puts below write by DMA and are checked in the image afterwards.
        s.write_all(&rp_loadproto::control_frame("card dma"))
            .map_err(|e| e.to_string())?;
        usb_read_until(
            &mut s,
            "RPL OK dma reads on (self-test passed), writes on",
            20,
        )?;
        s.write_all(&rp_loadproto::control_frame("card dma writes"))
            .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "writes on", 20)?;
        s.write_all(&frame).map_err(|e| e.to_string())?;
        let reply = usb_read_line(&mut s, "RPL ", 40)?;
        let line = reply
            .lines()
            .find(|l| l.starts_with("RPL "))
            .unwrap_or("")
            .to_string();
        let theirs = line.rsplit("sha256 ").next().unwrap_or("").trim();
        crate::usb::check_digest(theirs, &new, "file on the card")?;
        if !line.starts_with("RPL OK 70000 bytes") || !line.contains("contiguous") {
            return Err(format!("unexpected reply to the file: {line:?}"));
        }
        eprintln!("    loader: {line}");
        // 35 read commands when the FAT was read one sector per command
        // (2026-10-11); reading it in runs, 12.
        let reads: u32 = line
            .split_once(" (")
            .and_then(|(_, r)| r.split_once(" reads)"))
            .and_then(|(n, _)| n.parse().ok())
            .ok_or_else(|| format!("no read count in {line:?}"))?;
        if reads > MAX_PUT_READS {
            return Err(format!(
                "the write took {reads} read commands (at most {MAX_PUT_READS}): is the FAT search reading sector by sector again?"
            ));
        }
        s.write_all(&create).map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL OK 6000 bytes", 40)?;
        s.write_all(&bad).map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL ERR bad name", 20)?;
        // Into folders that don't exist yet: created on the way.
        s.write_all(&rp_loadproto::file_frame(
            "Apps/Sub Folder/hello.txt",
            b"hello",
        ))
        .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL OK 5 bytes", 20)?;
        // Browse and fetch, as `cargo xtask ls` and `get` do.
        s.write_all(&rp_loadproto::control_frame("ls Apps/Sub Folder"))
            .map_err(|e| e.to_string())?;
        let text = usb_read_until(&mut s, "RPL OK 1 entries", 20)?;
        let (entries, _) = crate::usb::parse_query(&text, "ls")?;
        if entries != ["5 hello.txt"] {
            return Err(format!("ls: {entries:?}"));
        }
        s.write_all(&rp_loadproto::control_frame("get retropane.bin"))
            .map_err(|e| e.to_string())?;
        let text = usb_read_line(&mut s, "RPL OK 70000 bytes crc", 40)?;
        let (lines, reply) = crate::usb::parse_query(&text, "get")?;
        if crate::usb::decode_file(&lines, &reply)? != new {
            return Err("get: the file fetched isn't the one put".into());
        }
        s.write_all(&rp_loadproto::control_frame("get no such file"))
            .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL ERR", 20)?;
        // The check finds the FAT sector this card was given damaged.
        s.write_all(&rp_loadproto::control_frame("card check"))
            .map_err(|e| e.to_string())?;
        let reply = usb_read_line(&mut s, "RPL ", 40)?;
        if !reply.contains("RPL OK check PROBLEMS") || !reply.contains("FAT copies differ 1,") {
            return Err(format!(
                "card check didn't find the damaged FAT sector: {reply:?}"
            ));
        }
        // Scrub: a first-FAT sector is rewritten and verified; anything
        // without a mirror is refused.
        let fat1 = rp_fat::format::PARTITION_START + 32;
        s.write_all(&rp_loadproto::control_frame(&format!("card scrub {fat1}")))
            .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL OK scrub", 20)?;
        s.write_all(&rp_loadproto::control_frame("card scrub 5"))
            .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL ERR scrub: only first-FAT", 20)?;
        // Repair: the damaged sector is restored; a healthy one is left.
        s.write_all(&rp_loadproto::control_frame(&format!(
            "card repair {damaged}"
        )))
        .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL OK repair", 20)?;
        s.write_all(&rp_loadproto::control_frame(&format!("card repair {fat1}")))
            .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "already matches", 20)?;
        // Peek: the boot sector's first bytes, read-only.
        s.write_all(&rp_loadproto::control_frame("card peek 2048 0"))
            .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL OK peek 2048+0: eb 58 90", 20)?;
        // Deleting: a file put for the purpose; the boot files refused;
        // a folder with something in it refused.
        s.write_all(&rp_loadproto::file_frame("remove me.txt", b"bye"))
            .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL OK 3 bytes", 20)?;
        s.write_all(&rp_loadproto::control_frame("rm remove me.txt"))
            .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL OK removed remove me.txt", 20)?;
        s.write_all(&rp_loadproto::control_frame("rm RetroPane.bin"))
            .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL ERR rm: that file boots the device", 20)?;
        s.write_all(&rp_loadproto::control_frame("rm Apps"))
            .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL ERR rm: folder not empty", 20)?;
        // Repaired, and after every write: clean.
        s.write_all(&rp_loadproto::control_frame("card check"))
            .map_err(|e| e.to_string())?;
        let reply = usb_read_line(&mut s, "RPL ", 40)?;
        if !reply.contains("RPL OK check clean") {
            return Err(format!("card check after the repair: {reply:?}"));
        }
        // Long file-system work keeps the watchdog fed as it reads (found
        // on the device: a 4 GB card check outlasted the 30 s run limit).
        s.write_all(&rp_loadproto::control_frame("drive status"))
            .map_err(|e| e.to_string())?;
        let status = usb_read_until(&mut s, "%\r\n", 10)?;
        let kept: u32 = status
            .split("kept alive ")
            .nth(1)
            .and_then(|t| t.split(',').next())
            .and_then(|n| n.trim().parse().ok())
            .ok_or_else(|| format!("no keep-alive count in {status:?}"))?;
        if kept < 100 {
            return Err(format!(
                "card commands didn't keep the watchdog fed: {kept}"
            ));
        }
        eprintln!("    keep-alive OK: card commands fed the watchdog at {kept} block reads");
        Ok(())
    };
    boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: Some(&sd),
            steps: &[],
            extra: &extra,
            input: None,
            during: Some(&check),
            then: None,
            timeout: Duration::from_secs(90),
        },
    )?;
    let got = card_file(&sd, "retropane.bin")?;
    if got != new {
        return Err(format!(
            "the card holds {} bytes of retropane.bin after the put, not the {} sent",
            got.len(),
            new.len()
        ));
    }
    if card_file(&sd, "startup.txt")? != script {
        return Err("startup.txt changed on the card".into());
    }
    let after = std::fs::read(&sd).map_err(|e| e.to_string())?;
    let fat_sectors = {
        let b = &after[rp_fat::format::PARTITION_START as usize * 512..];
        u32::from_le_bytes([b[36], b[37], b[38], b[39]])
    };
    let one = &after[damaged as usize * 512..damaged as usize * 512 + 512];
    let two = &after
        [(damaged + fat_sectors) as usize * 512..(damaged + fat_sectors) as usize * 512 + 512];
    if one != two {
        return Err("card repair: the first FAT sector still differs from its mirror".into());
    }
    if card_file(&sd, "remove me.txt").is_ok() {
        return Err("rm: the removed file is still on the card".into());
    }
    if card_file(&sd, "Apps/Sub Folder/hello.txt")? != b"hello" {
        return Err("the file in new folders isn't on the card".into());
    }
    if card_file(&sd, "RetroPane loader.lnk")? != created
        || card_file(&sd, "RETROP~1.LNK")? != created
    {
        return Err("the created file isn't on the card (long or short name)".into());
    }
    eprintln!("    file to card OK: replaced through the loader, verified in the image");
    Ok(())
}

/// A USB Mass Storage host over the UDC model's framed chardev: each OUT
/// transfer goes as a 4-byte length and its bytes, each IN packet comes as
/// a 1-byte length and its bytes (ADR-0014).
struct Bot {
    s: std::net::TcpStream,
    tag: u32,
    /// The logical unit commands go to (0: SD card, 1: CF card).
    lun: u8,
}

/// One command's outcome.
struct BotReply {
    data: Vec<u8>,
    status: u8,
    residue: u32,
}

impl Bot {
    fn connect(n: u16) -> Result<Self, String> {
        Ok(Self {
            s: usb_connect(n)?,
            tag: 1,
            lun: 0,
        })
    }

    fn send_transfer(&mut self, data: &[u8]) -> Result<(), String> {
        let mut f = (data.len() as u32).to_le_bytes().to_vec();
        f.extend_from_slice(data);
        self.s.write_all(&f).map_err(|e| e.to_string())
    }

    fn read_exact(&mut self, buf: &mut [u8], deadline: Instant) -> Result<(), String> {
        let mut got = 0;
        while got < buf.len() {
            if Instant::now() > deadline {
                return Err("drive: no answer".into());
            }
            match self.s.read(&mut buf[got..]) {
                Ok(0) => return Err("drive: connection closed".into()),
                Ok(n) => got += n,
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        Ok(())
    }

    fn packet(&mut self, deadline: Instant) -> Result<Vec<u8>, String> {
        let mut n = [0u8; 1];
        self.read_exact(&mut n, deadline)?;
        let mut p = vec![0u8; usize::from(n[0])];
        self.read_exact(&mut p, deadline)?;
        Ok(p)
    }

    /// Sends a raw transfer that isn't a command (hostile-input tests).
    fn junk(&mut self, data: &[u8]) -> Result<(), String> {
        self.send_transfer(data)
    }

    fn command(
        &mut self,
        cb: &[u8],
        len: u32,
        dir_in: bool,
        out: &[u8],
    ) -> Result<BotReply, String> {
        self.tag = self.tag.wrapping_add(1);
        let mut c = vec![0u8; 31];
        c[..4].copy_from_slice(b"USBC");
        c[4..8].copy_from_slice(&self.tag.to_le_bytes());
        c[8..12].copy_from_slice(&len.to_le_bytes());
        c[12] = if dir_in { 0x80 } else { 0 };
        c[13] = self.lun;
        c[14] = cb.len() as u8;
        c[15..15 + cb.len()].copy_from_slice(cb);
        self.send_transfer(&c)?;
        if !out.is_empty() {
            self.send_transfer(out)?;
        }
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut data = Vec::new();
        loop {
            let p = self.packet(deadline)?;
            if p.len() == 13 && &p[..4] == b"USBS" && p[4..8] == self.tag.to_le_bytes() {
                return Ok(BotReply {
                    data,
                    status: p[12],
                    residue: u32::from_le_bytes([p[8], p[9], p[10], p[11]]),
                });
            }
            data.extend_from_slice(&p);
        }
    }

    /// A fresh host's mount: READ CAPACITY first (as Windows, Linux and
    /// macOS mount), until the unit answers (a change reported once, or a
    /// card still being attached), so the drive takes the view as current.
    fn mount(&mut self) -> Result<u32, String> {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let r = self.command(&[0x25, 0, 0, 0, 0, 0, 0, 0, 0, 0], 8, true, &[])?;
            if r.status == 0 && r.data.len() == 8 {
                return Ok(u32::from_be_bytes([r.data[0], r.data[1], r.data[2], r.data[3]]) + 1);
            }
            let _ = self.sense()?;
            if Instant::now() > deadline {
                return Err(format!(
                    "drive: unit {} never answered READ CAPACITY",
                    self.lun
                ));
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    fn sense(&mut self) -> Result<(u8, u8, u8), String> {
        let r = self.command(&[0x03, 0, 0, 0, 18, 0], 18, true, &[])?;
        if r.data.len() < 14 {
            return Err(format!("drive: REQUEST SENSE gave {} bytes", r.data.len()));
        }
        Ok((r.data[2], r.data[12], r.data[13]))
    }
}

fn rw10(op: u8, lba: u32, count: u16) -> [u8; 10] {
    let mut cb = [0u8; 10];
    cb[0] = op;
    cb[2..6].copy_from_slice(&lba.to_be_bytes());
    cb[7..9].copy_from_slice(&count.to_be_bytes());
    cb
}

impl rp_hal_api::BlockDevice for Bot {
    fn read_blocks(&mut self, lba: u32, buf: &mut [u8]) -> Result<(), rp_hal_api::BlockError> {
        for (i, chunk) in buf.chunks_mut(64 * 512).enumerate() {
            let at = lba + (i * 64) as u32;
            let n = (chunk.len() / 512) as u16;
            let r = self
                .command(&rw10(0x28, at, n), chunk.len() as u32, true, &[])
                .map_err(|_| rp_hal_api::BlockError::Device)?;
            if r.status != 0 || r.data.len() != chunk.len() {
                return Err(rp_hal_api::BlockError::Device);
            }
            chunk.copy_from_slice(&r.data);
        }
        Ok(())
    }
    fn write_blocks(&mut self, lba: u32, buf: &[u8]) -> Result<(), rp_hal_api::BlockError> {
        for (i, chunk) in buf.chunks(64 * 512).enumerate() {
            let at = lba + (i * 64) as u32;
            let n = (chunk.len() / 512) as u16;
            let r = self
                .command(&rw10(0x2A, at, n), chunk.len() as u32, false, chunk)
                .map_err(|_| rp_hal_api::BlockError::Device)?;
            if r.status != 0 {
                // As a PC does: ask why. DATA PROTECT is "write-protected".
                return Err(match self.sense() {
                    Ok((0x07, 0x27, 0x00)) => rp_hal_api::BlockError::ReadOnly,
                    _ => rp_hal_api::BlockError::Device,
                });
            }
        }
        Ok(())
    }
}

/// After the device changed the medium: polls like Windows (TEST UNIT
/// READY every 250 ms) and needs the medium to go away ("no medium" at
/// least once: Windows drops its cached view only then, item 65), then come
/// back with "medium changed" exactly once, then be ready.
fn expect_away_then_back(bot: &mut Bot, after: &str) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut seq = Vec::new();
    loop {
        let r = bot.command(&[0; 6], 0, false, &[])?;
        let s = if r.status == 0 {
            (0, 0, 0)
        } else {
            bot.sense()?
        };
        if seq.last() != Some(&s) {
            seq.push(s);
        }
        if r.status == 0 {
            break;
        }
        if Instant::now() > deadline {
            return Err(format!("drive after {after}: never ready again ({seq:x?})"));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    if seq != [(0x02, 0x3A, 0x00), (0x06, 0x28, 0x00), (0, 0, 0)] {
        return Err(format!(
            "drive after {after}: wanted away, medium changed, ready; got {seq:x?}"
        ));
    }
    Ok(())
}

/// The first block of a FAT32 volume's root directory, from its boot
/// sector (read through the drive, as the PC would).
fn root_dir_lba(bot: &mut Bot) -> Result<u32, String> {
    let ps = rp_fat::format::PARTITION_START;
    let mut b = [0u8; 512];
    rp_hal_api::BlockDevice::read_blocks(bot, ps, &mut b)
        .map_err(|_| "drive: boot sector unreadable".to_string())?;
    let le16 = |o: usize| u32::from(u16::from_le_bytes([b[o], b[o + 1]]));
    let le32 = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    let (spc, reserved, fats, fat_size, root) = (
        u32::from(b[13]),
        le16(14),
        u32::from(b[16]),
        le32(36),
        le32(44),
    );
    Ok(ps + reserved + fats * fat_size + (root - 2) * spc)
}

/// `cargo xtask emu-stale`: the stale-view check on its own.
pub fn boot_loader_stale_view_alone(root: &Path) -> Result<(), String> {
    let dir = emu_dir(root)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    crate::sdkit::raw_binary(
        &crate::arm_release(root).join("rp-loader"),
        &dir.join("rp-loader.bin"),
    )?;
    boot_loader_stale_view(root)
}

/// The stale-view class (noise-ri.bin lost, 2026-10-15): the PC writes the
/// card through the drive, then something writes it while the loader isn't
/// running (here the crash report the loader keeps after rp-crash; on the
/// device an image such as rp-noise, or Windows Mobile). The X51v's pull-up
/// is hard-wired, so Windows can re-attach without noticing the device went
/// and keep its cached root directory. Its first write after the loader is
/// back (that old root directory) must be refused with the card untouched,
/// the drive must go away and come back changed, and after re-reading, the
/// PC's next file must leave every entry in place: the PC's first file, the
/// crash report and the new one, checked in the card image.
pub fn boot_loader_stale_view(root: &Path) -> Result<(), String> {
    if !has_msc_model(root) {
        eprintln!("    stale view: skipped (this emulator's USB model has no mass storage bridge)");
        return Ok(());
    }
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    let crash = dir.join("rp-crash.bin");
    crate::sdkit::raw_binary(&release.join("rp-crash"), &crash)?;
    let payload = std::fs::read(&crash).map_err(|e| format!("{}: {e}", crash.display()))?;
    let frame = rp_loadproto::frame(&payload);
    let loader = dir.join("rp-loader.bin");
    let image = rp_fat::format::build(&rp_fat::format::Image {
        total_sectors: 32 * 1024,
        sectors_per_cluster: 8,
        files: &[],
        blocked_clusters: &[],
        ballast: 0,
    });
    let sd = scratch(root, "sd-stale.img")?;
    std::fs::write(&sd, &image).map_err(|e| format!("{}: {e}", sd.display()))?;
    let before = b"written by the PC before the image ran\r\n";
    let after = b"written by the PC after the loader came back\r\n";
    let mut args: Vec<String> = usb_args(14).to_vec();
    args.extend(msc_args(15));
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let check = || -> Result<(), String> {
        let mut bot = Bot::connect(15)?;
        let mut s = usb_connect(14)?;
        usb_read_until(&mut s, "RPLOAD1 READY", 10)?;
        bot.mount()?;
        let mut scratch = vec![0u8; 32 * 1024];
        {
            let mut vol = rp_fat::Volume::mount(&mut bot, rp_fat::format::PARTITION_START)
                .map_err(|e| format!("stale view: mount: {}", e.name()))?;
            vol.put("pc-before.txt", before, &mut scratch)
                .map_err(|e| format!("stale view: put: {}", e.name()))?;
        }
        // The root directory as the PC caches it.
        let root_lba = root_dir_lba(&mut bot)?;
        let mut stale = vec![0u8; 512];
        rp_hal_api::BlockDevice::read_blocks(&mut bot, root_lba, &mut stale)
            .map_err(|_| "stale view: root directory unreadable".to_string())?;
        // An image that crashes: the loader comes back and keeps the
        // report on the card before it offers the drive again.
        s.write_all(&frame).map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL OK sha256 ", 15)?;
        usb_read_until(&mut s, "RPLOAD1 READY", 60)?;
        // The PC kept its view and writes its old root directory back.
        let r = bot.command(&rw10(0x2A, root_lba, 1), 512, false, &stale)?;
        if r.status == 0 {
            return Err(
                "stale view: the PC's root directory from before the image was written back (the crash report's entry is lost)"
                    .into(),
            );
        }
        let sense = bot.sense()?;
        if sense != (0x02, 0x3A, 0x00) {
            return Err(format!(
                "stale view: the old view's write failed with {sense:x?}, not \"no medium\""
            ));
        }
        expect_away_then_back(&mut bot, "the image's run")?;
        // Re-read (a fresh view): the report is there; the next file
        // goes next to it.
        bot.mount()?;
        let report = drive_file(&mut bot, "crashlog.txt")?;
        if report.is_empty() {
            return Err("stale view: crashlog.txt is empty".into());
        }
        let mut vol = rp_fat::Volume::mount(&mut bot, rp_fat::format::PARTITION_START)
            .map_err(|e| format!("stale view: mount after: {}", e.name()))?;
        vol.put("pc-after.txt", after, &mut scratch)
            .map_err(|e| format!("stale view: put after: {}", e.name()))?;
        Ok(())
    };
    boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: Some(&sd),
            steps: &[],
            extra: &extra,
            input: None,
            during: Some(&check),
            then: None,
            timeout: Duration::from_secs(90),
        },
    )?;
    for (name, want) in [("pc-before.txt", &before[..]), ("pc-after.txt", &after[..])] {
        if card_file(&sd, name)? != want {
            return Err(format!("stale view: {name} differs in the card image"));
        }
    }
    if card_file(&sd, "crashlog.txt")?.is_empty() {
        return Err("stale view: crashlog.txt lost from the card image".into());
    }
    eprintln!(
        "    stale view OK: after the image's run (crash report kept on the card) the PC's old root directory was refused with no medium, the drive went away and came back changed, and the PC's files and the report all stayed"
    );
    Ok(())
}

/// Reads `name` from the card through the drive.
fn drive_file(bot: &mut Bot, name: &str) -> Result<Vec<u8>, String> {
    let mut vol = rp_fat::Volume::mount(&mut *bot, rp_fat::format::PARTITION_START)
        .map_err(|e| format!("drive: mount: {}", e.name()))?;
    let f = vol
        .find(name)
        .map_err(|e| format!("drive: {name}: {}", e.name()))?;
    let mut out = Vec::new();
    let mut scratch = vec![0u8; 32 * 1024];
    vol.read_file(&f, &mut scratch, |c| out.extend_from_slice(c))
        .map_err(|e| format!("drive: {name}: {}", e.name()))?;
    Ok(out)
}

/// The SD card as a USB drive (ADR-0014), end to end in the emulator: the
/// loader's mass storage interface next to its serial port; the test mounts
/// the FAT32 card image through the drive with `rp-fat`, reads a file,
/// replaces another (copy-then-swap through SCSI writes), checks that a
/// serial `put` is refused while the PC writes and accepted after, that the
/// drive then reports the medium changed, and that malformed commands fail
/// cleanly. The image on the host is checked afterwards.
pub fn boot_loader_drive(root: &Path) -> Result<(), String> {
    if !has_msc_model(root) {
        eprintln!("    drive: skipped (this emulator's USB model has no mass storage bridge)");
        return Ok(());
    }
    let loader = raw_image(root, "rp-loader")?;
    let old: Vec<u8> = (0..5000u32).map(|i| (i * 3) as u8).collect();
    let script = b"set ramaddr 0xa8000000\r\n".to_vec();
    let image = rp_fat::format::build(&rp_fat::format::Image {
        total_sectors: 32 * 1024,
        sectors_per_cluster: 8,
        files: &[
            rp_fat::format::Entry {
                name: "retropane.bin",
                data: &old,
            },
            rp_fat::format::Entry {
                name: "startup.txt",
                data: &script,
            },
        ],
        blocked_clusters: &[],
        ballast: 0,
    });
    let blocks = (image.len() / 512) as u32;
    let sd = scratch(root, "sd-drive.img")?;
    std::fs::write(&sd, &image).map_err(|e| format!("{}: {e}", sd.display()))?;
    let via_drive = b"written through the USB drive\r\n".to_vec();
    let via_serial: Vec<u8> = (0..9000u32).map(|i| (i * 11 + 1) as u8).collect();
    let put = rp_loadproto::file_frame("retropane.bin", &via_serial);
    let mut args: Vec<String> = usb_args(5).to_vec();
    args.extend(msc_args(6));
    let slow = device_has(root, "pxa2xx-mmci", "slow-address");
    if slow {
        // The test image is a standard-capacity card: byte addresses.
        args.extend([
            "-global".into(),
            format!("pxa2xx-mmci.slow-address={}", SLOW_BLOCK * 512),
        ]);
    }
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let check = || -> Result<(), String> {
        let mut bot = Bot::connect(6)?;
        let mut serial = usb_connect(5)?;
        usb_read_until(&mut serial, "RPLOAD1 READY", 10)?;
        bot.mount()?;
        // The drive becomes ready.
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let r = bot.command(&[0; 6], 0, false, &[])?;
            if r.status == 0 {
                break;
            }
            let _ = bot.sense()?;
            if Instant::now() > deadline {
                return Err("drive: never became ready".into());
            }
        }
        let r = bot.command(&[0x12, 0, 0, 0, 36, 0], 36, true, &[])?;
        if r.data.len() != 36 || &r.data[8..16] != b"RetroPan" || r.data[1] & 0x80 == 0 {
            return Err(format!("drive: INQUIRY {:?}", r.data));
        }
        let r = bot.command(&[0x25, 0, 0, 0, 0, 0, 0, 0, 0, 0], 8, true, &[])?;
        let last = u32::from_be_bytes([r.data[0], r.data[1], r.data[2], r.data[3]]);
        if last + 1 != blocks {
            return Err(format!(
                "drive: capacity {} blocks, image {blocks}",
                last + 1
            ));
        }
        if drive_file(&mut bot, "retropane.bin")? != old {
            return Err("drive: retropane.bin read through the drive differs".into());
        }
        // Writable by default (ADR-0014, the boot-file guard): MODE SENSE
        // says so, but the PC can't change what boots the device.
        let write_protected = |bot: &mut Bot| -> Result<bool, String> {
            let r = bot.command(&[0x1A, 0, 0x3F, 0, 255, 0], 255, true, &[])?;
            Ok(r.data.get(2).is_some_and(|b| b & 0x80 != 0))
        };
        if write_protected(&mut bot)? {
            return Err("drive: write-protected at start (should be guarded, writable)".into());
        }
        let refuses_boot_file = |bot: &mut Bot, why: &str| -> Result<(), String> {
            let mut vol = rp_fat::Volume::mount(&mut *bot, rp_fat::format::PARTITION_START)
                .map_err(|e| format!("drive: mount: {}", e.name()))?;
            let mut scratch = vec![0u8; 32 * 1024];
            // Refused as write-protected, not failed some other way.
            let protected = Err(rp_fat::Error::Device(rp_hal_api::BlockError::ReadOnly));
            let replaced = vol
                .replace("retropane.bin", b"not a loader", &mut scratch)
                .map(|_| ());
            let removed = vol.remove("startup.txt");
            if replaced != protected || removed != protected {
                return Err(format!(
                    "drive: boot files not protected ({why}): replace {replaced:?}, remove {removed:?}"
                ));
            }
            Ok(())
        };
        refuses_boot_file(&mut bot, "at start")?;
        if drive_file(&mut bot, "retropane.bin")? != old {
            return Err("drive: a refused write changed retropane.bin".into());
        }
        // `drive ro` locks it all, `drive rw` gives the guarded drive back.
        for (cmd, reply, wp) in [
            ("drive ro", "RPL OK drive read-only", true),
            ("drive rw", "RPL OK drive read-write", false),
        ] {
            serial
                .write_all(&rp_loadproto::control_frame(cmd))
                .map_err(|e| e.to_string())?;
            usb_read_until(&mut serial, reply, 10)?;
            expect_away_then_back(&mut bot, cmd)?;
            if write_protected(&mut bot)? != wp {
                return Err(format!("drive: protection wrong after {cmd}"));
            }
            if wp {
                let r = bot.command(&rw10(0x2A, 100, 1), 512, false, &[0; 512])?;
                if r.status != 1 || bot.sense()? != (0x07, 0x27, 0x00) {
                    return Err("drive: a write was accepted while read-only".into());
                }
            }
        }
        guest_hold(
            &mut crate::screen::Monitor::connect(monitor_port())?,
            2500,
            "drive",
        )?; // the write grace
        // Write a new file through the drive (the PC's side).
        {
            let mut vol = rp_fat::Volume::mount(&mut bot, rp_fat::format::PARTITION_START)
                .map_err(|e| format!("drive: mount: {}", e.name()))?;
            let mut scratch = vec![0u8; 32 * 1024];
            vol.put("notes.txt", &via_drive, &mut scratch)
                .map_err(|e| format!("drive: put: {}", e.name()))?;
        }
        // The device refuses to write while the PC just wrote...
        serial.write_all(&put).map_err(|e| e.to_string())?;
        let r = usb_read_line(&mut serial, "RPL ", 20)?;
        if !r.contains("RPL ERR card busy") {
            return Err(format!("serial put during a drive write: {r:?}"));
        }
        // ... and accepts once the PC has been quiet for the grace period.
        guest_hold(
            &mut crate::screen::Monitor::connect(monitor_port())?,
            2500,
            "drive",
        )?;
        serial.write_all(&put).map_err(|e| e.to_string())?;
        let r = usb_read_until(&mut serial, "RPL OK", 30)?;
        let _ = r;
        // The medium goes away until the PC has noticed, then comes back
        // changed: the PC's view then matches the card.
        expect_away_then_back(&mut bot, "the device's put")?;
        if drive_file(&mut bot, "retropane.bin")? != via_serial {
            return Err("drive: doesn't show the file the device wrote".into());
        }
        if drive_file(&mut bot, "notes.txt")? != via_drive {
            return Err("drive: lost the file it wrote".into());
        }
        // The guard follows the device's own write: the new loader's
        // clusters are protected now.
        guest_hold(
            &mut crate::screen::Monitor::connect(monitor_port())?,
            2500,
            "drive",
        )?;
        refuses_boot_file(&mut bot, "after the device replaced it")?;
        if drive_file(&mut bot, "retropane.bin")? != via_serial {
            return Err("drive: a refused write changed the new retropane.bin".into());
        }
        // Hostile input: junk transfers are ignored, an out-of-range and a
        // wrapping read fail with sense, and the drive keeps working.
        bot.junk(&[0x55; 31])?;
        bot.junk(b"USBC")?;
        let r = bot.command(&rw10(0x28, blocks, 1), 512, true, &[])?;
        if r.status != 1 || bot.sense()? != (0x05, 0x21, 0x00) {
            return Err("drive: out-of-range read not refused".into());
        }
        let r = bot.command(&rw10(0x28, u32::MAX, 2), 1024, true, &[])?;
        if r.status != 1 || r.residue != 1024 {
            return Err("drive: wrapping read not refused".into());
        }
        let r = bot.command(&[0; 6], 0, false, &[])?;
        if r.status != 0 {
            return Err("drive: not ready after hostile input".into());
        }
        // Throughput (emulator wall clock; the device's number is the real one).
        // It crosses the slow block: the card driver must fall back to
        // single blocks and still return exactly the card's bytes.
        let t = Instant::now();
        let r = bot.command(&rw10(0x28, 0, 2048), 2048 * 512, true, &[])?;
        if r.status != 0 || r.data.len() != 2048 * 512 {
            return Err(format!(
                "drive: 1 MiB read across the slow block {SLOW_BLOCK} failed (status {})",
                r.status
            ));
        }
        if r.data != image[..2048 * 512] {
            return Err("drive: the 1 MiB read doesn't match the card image".into());
        }
        eprintln!(
            "    drive: 1 MiB read in {} ms through the emulated drive",
            t.elapsed().as_millis()
        );
        // About 3 s on an idle PC (a loader napping mid-transfer took 17),
        // but the wall clock is the PC's, not the loader's: the loader's
        // own count of naps mid-command is checked below instead.
        // Item 58: sequential reads, as a PC copying a file makes them: the
        // second starts where the first ended, so its first chunk was read
        // ahead (by DMA, while the first one's last chunk went out). Both
        // must still be exactly the card's bytes.
        for at in [4096u32, 4096 + 128] {
            let r = bot.command(&rw10(0x28, at, 128), 128 * 512, true, &[])?;
            let want = &image[at as usize * 512..(at as usize + 128) * 512];
            if r.status != 0 || r.data != want {
                return Err(format!(
                    "drive: sequential read at {at} doesn't match the card"
                ));
            }
        }
        serial
            .write_all(&rp_loadproto::control_frame("drive status"))
            .map_err(|e| e.to_string())?;
        let status = usb_read_until(&mut serial, "%\r\n", 10)?;
        let ahead: u32 = status
            .split("read ahead ")
            .nth(1)
            .and_then(|t| t.split(',').next())
            .and_then(|n| n.trim().parse().ok())
            .ok_or_else(|| format!("no read-ahead count in {status:?}"))?;
        // A freshly formatted card: Windows' dirty flag clear as found and
        // after the drive work (the emulated PC isn't Windows).
        if !status.contains("dirty flag found clear now clear") {
            return Err(format!("drive: dirty flag not read as clear: {status:?}"));
        }
        if ahead == 0 {
            return Err(format!(
                "drive: sequential reads found nothing read ahead: {status:?}"
            ));
        }
        if !status.contains("naps mid-command 0,") {
            return Err(format!(
                "drive: the loader napped with a drive command under way: {status:?}"
            ));
        }
        eprintln!("    drive read-ahead OK: {ahead} reads found their first chunk already read");
        Ok(())
    };
    boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: Some(&sd),
            steps: &[],
            extra: &extra,
            input: None,
            during: Some(&check),
            then: None,
            timeout: Duration::from_secs(120),
        },
    )?;
    if card_file(&sd, "notes.txt")? != via_drive || card_file(&sd, "retropane.bin")? != via_serial {
        return Err("drive: the card image doesn't hold both writes".into());
    }
    eprintln!(
        "    drive OK: the card mounted over USB mass storage, files read and written both ways, ownership rules and hostile input checked"
    );
    Ok(())
}

/// The loader's USB layout fallback: a host that refuses composite devices
/// (the model's `accept-composite=off`) never configures the serial port
/// plus drive; the loader must offer the serial-only layout by itself
/// within its patience (25 s) and the PC then reaches it as before.
pub fn boot_loader_fallback(root: &Path) -> Result<(), String> {
    let help = match emulator(root) {
        Ok(Emulator::Native(exe)) => Command::new(exe)
            .args(["-M", "x51v", "-device", "pxa27x-udc,help"])
            .output(),
        Ok(Emulator::Wsl) => {
            wsl_bash(&format!("{} -M x51v -device pxa27x-udc,help", wsl_qemu())).output()
        }
        Err(e) => return Err(e),
    };
    if !help.is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("accept-composite")) {
        eprintln!(
            "    USB layout fallback: skipped (this emulator's host can't refuse composites)"
        );
        return Ok(());
    }
    let loader = raw_image(root, "rp-loader")?;
    let mut args: Vec<String> = usb_args(7).to_vec();
    args.extend(["-global".into(), "pxa27x-udc.accept-composite=off".into()]);
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let check = || -> Result<(), String> {
        let mut s = usb_connect(7)?;
        usb_read_until(&mut s, "RPLOAD1 READY", 60)?;
        // And it answers commands there (2026-10-24: on the X51v the
        // fallback enumerated and opened, and no command got a reply).
        s.write_all(&rp_loadproto::control_frame("handoff"))
            .map_err(|e| e.to_string())?;
        let reply = usb_read_line(&mut s, "RPL ", 20)?;
        if !reply.contains("RPL OK handoff") {
            return Err(format!(
                "serial-only layout: no reply to a command: {reply:?}"
            ));
        }
        Ok(())
    };
    // A READY over USB from a host that refuses composites can only come
    // through the serial-only layout: that is the switch.
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
            timeout: Duration::from_secs(90),
        },
    )?;
    eprintln!(
        "    USB layout fallback OK: a host refusing composites reached the loader serial-only"
    );
    Ok(())
}

/// The SD card's lock switch: a card the emulator inserts read-only
/// (`readonly=on`, which the X51v model shows on GPIO27) must refuse a
/// file from the PC, and the drive must report it write-protected.
pub fn boot_loader_locked(root: &Path) -> Result<(), String> {
    if !has_usb_model(root) {
        eprintln!("    locked card: skipped (no USB model)");
        return Ok(());
    }
    let loader = raw_image(root, "rp-loader")?;
    let image = rp_fat::format::build(&rp_fat::format::Image {
        total_sectors: 32 * 1024,
        sectors_per_cluster: 8,
        files: &[rp_fat::format::Entry {
            name: "a.bin",
            data: b"old",
        }],
        blocked_clusters: &[],
        ballast: 0,
    });
    let sd = scratch(root, "sd-locked.img")?;
    std::fs::write(&sd, &image).map_err(|e| format!("{}: {e}", sd.display()))?;
    if !device_has(root, "pxa2xx-mmci", "locked") {
        eprintln!("    locked card: skipped (this emulator has no SD lock switch)");
        return Ok(());
    }
    // The lock switch is the slot's (a tab the host reads); the card itself
    // stays writable, as on real hardware, so a loader that ignored the
    // switch would change the image.
    let mut args: Vec<String> = usb_args(8).to_vec();
    args.extend(["-global".into(), "pxa2xx-mmci.locked=on".into()]);
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let put = rp_loadproto::file_frame("a.bin", b"new");
    let check = || -> Result<(), String> {
        let mut s = usb_connect(8)?;
        usb_read_until(&mut s, "RPLOAD1 READY", 10)?;
        s.write_all(&put).map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL ERR card is write-protected", 20)?;
        Ok(())
    };
    boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: Some(&sd),
            steps: &[],
            extra: &extra,
            input: None,
            during: Some(&check),
            then: None,
            timeout: Duration::from_secs(60),
        },
    )?;
    if card_file(&sd, "a.bin")? != b"old" {
        return Err("locked card: the file changed".into());
    }
    eprintln!("    locked card OK: the lock switch refused the write, the card is unchanged");
    Ok(())
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

/// Parses `BENCH <name> <ticks> <iters>` lines.
pub fn parse_bench(output: &str) -> Vec<BenchResult> {
    output
        .lines()
        .filter_map(|l| {
            let mut it = l.trim().strip_prefix("BENCH ")?.split(' ');
            let name = it.next()?.to_string();
            let ticks: f64 = it.next()?.parse().ok()?;
            let iters: f64 = it.next()?.parse().ok()?;
            (iters > 0.0).then(|| BenchResult {
                name,
                ns: ticks * 1e9 / TICK_HZ / iters,
            })
        })
        .collect()
}

/// `cargo xtask bench [--update]`: deterministic micro-benchmarks in the
/// emulator, compared with `bench/baseline-emulator.json`.
pub fn bench(root: &Path, update: bool) -> Result<(), String> {
    let image = crate::arm_release(root).join("rp-bench");
    let sd = write_sd_test_image(&scratch(root, "sd-bench.img")?)?;
    let output = boot(
        root,
        &Boot {
            image: &image,
            marker: FAULT_EXPECT,
            screenshot: None,
            sd_image: Some(&sd),
            steps: &[],
            // One virtual nanosecond per instruction: timings become a
            // deterministic measure of the work done.
            extra: &["-icount", "shift=0"],
            input: None,
            during: None,
            then: None,
            timeout: Duration::from_secs(120),
        },
    )?;
    if !output.contains(BENCH_DONE) {
        return Err(format!("benchmarks didn't finish:\n{output}"));
    }
    eprintln!("    fault path OK: deliberate unmapped read reported as a data abort");
    // Checks that must have run (and passed), not merely not failed.
    for check in [
        "mem",
        "crypto",
        "sd_multi_block_matches",
        "sd_dma_past_chain",
        "gfx",
    ] {
        if !output.contains(&format!("CHECK {check} OK")) {
            return Err(format!(
                "on-target check `{check}` missing or failed:\n{output}"
            ));
        }
    }
    // Every on-target check must pass, not just the memory one.
    let failed: Vec<&str> = output
        .lines()
        .filter(|l| l.starts_with("CHECK ") && l.contains(" FAIL"))
        .collect();
    if !failed.is_empty() {
        return Err(format!("on-target checks failed:\n{}", failed.join("\n")));
    }
    for l in output.lines().filter(|l| l.starts_with("CHECK ")) {
        eprintln!("    {l}");
    }
    let results = parse_bench(&output);
    if results.is_empty() {
        return Err(format!("no benchmark results. Serial output:\n{output}"));
    }
    let path = root.join("bench").join("baseline-emulator.json");
    let baseline: Option<serde_json::Value> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok());
    let same_emulator = baseline
        .as_ref()
        .is_some_and(|b| b["emulator"] == serde_json::json!(QEMU_RELEASE));

    let mut regressions = Vec::new();
    eprintln!(
        "    {:<22} {:>14} {:>14} {:>8}",
        "benchmark", "ns/iter", "baseline", "change"
    );
    for r in &results {
        let base = baseline
            .as_ref()
            .filter(|_| same_emulator)
            .and_then(|b| b["results"][&r.name].as_f64());
        let (base_s, change_s) = match base {
            Some(b) if b > 0.0 => {
                let change = r.ns / b - 1.0;
                if change > REGRESSION_LIMIT {
                    regressions.push(format!("{} is {:.1}% slower", r.name, change * 100.0));
                }
                (format!("{b:.0}"), format!("{:+.1}%", change * 100.0))
            }
            _ => ("-".into(), "new".into()),
        };
        eprintln!(
            "    {:<22} {:>14.0} {:>14} {:>8}",
            r.name, r.ns, base_s, change_s
        );
    }

    if update {
        let map: serde_json::Map<String, serde_json::Value> = results
            .iter()
            .map(|r| (r.name.clone(), serde_json::json!(r.ns.round())))
            .collect();
        let doc = serde_json::json!({
            "emulator": QEMU_RELEASE,
            "units": "nanoseconds per iteration, QEMU -icount shift=0 (deterministic)",
            "results": map,
        });
        std::fs::create_dir_all(path.parent().expect("has parent")).map_err(|e| e.to_string())?;
        let text = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())? + "\n";
        std::fs::write(&path, text).map_err(|e| e.to_string())?;
        eprintln!("    baseline updated: {}", path.display());
        return Ok(());
    }
    if baseline.is_some() && !same_emulator {
        eprintln!("    note: baseline was recorded on a different emulator release; not comparing");
    }
    if regressions.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "performance regressions: {}",
            regressions.join("; ")
        ))
    }
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

/// The stack's headroom, for every image the emulator runs: each reports
/// "stack: deepest N of M bytes" when it returns to the loader, and use above
/// 80% fails. A silent overflow freezes the image (rp-today ran at 96% of its
/// stack and an older build froze the Axim into the watchdog error screen,
/// 2026-10-17). Returns the deepest report seen, as (used, size).
fn check_stack(text: &str) -> Result<Option<(u32, u32)>, String> {
    let mut deepest: Option<(u32, u32)> = None;
    for rest in text.split("stack: deepest ").skip(1) {
        let mut n = rest.split(" of ").map(|p| {
            p.split(|c: char| !c.is_ascii_digit())
                .next()
                .and_then(|d| d.parse::<u32>().ok())
        });
        let (Some(Some(used)), Some(Some(size))) = (n.next(), n.next()) else {
            return Err(format!(
                "unreadable stack report: {:?}",
                rest.lines().next().unwrap_or("")
            ));
        };
        if u64::from(used) * 100 > u64::from(size) * 80 {
            return Err(format!(
                "the stack went {used} of {size} bytes deep (over 80%: an overflow freezes the image)"
            ));
        }
        if deepest
            .is_none_or(|(u, s)| u64::from(used) * u64::from(s) > u64::from(u) * u64::from(size))
        {
            deepest = Some((used, size));
        }
    }
    Ok(deepest)
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

/// The performance gate's P4 test image (rp-tabench): runs under the
/// loader. With the fork's 3D model the CPU binner's way draws every set
/// and its frames 2-5 repeat the first; the fork doesn't model the tile
/// accelerator, so every TA way must run, time out ("TA done 0") and move
/// on, which is the hang path a device TA hang takes (Q2 A). The clock,
/// plane and core put back, it returns on a button.
pub fn boot_loader_tabench(root: &Path) -> Result<(), String> {
    if !device_has(root, "intel-2700g", "mbx-3d") {
        eprintln!("    tabench: skipped (this emulator release doesn't model the MBX 3D core)");
        return Ok(());
    }
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    let loader = dir.join("rp-loader.bin");
    let bin = dir.join("rp-tabench.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-tabench"), &bin)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let check = || -> Result<(), String> {
        let mut mon = crate::screen::Monitor::connect(monitor_port())?;
        mon.query("sendkey ret 200")?;
        Ok(())
    };
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&rp_loadproto::frame(&payload), "RETROPANE-TABENCH: DONE")),
            during: Some(&check),
            then: Some("RPL END"),
            timeout: Duration::from_secs(3600),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("tabench: never came back to the loader after the button press")?;
    if crashed(log) || log.contains("did not return") {
        return Err(format!("tabench: didn't return cleanly:{log}"));
    }
    for want in [
        "SET 6 of 6",
        "memory manager ready: true",
        "RETROPANE-TABENCH: DONE",
    ] {
        if !log.contains(want) {
            return Err(format!("tabench: the run log has no `{want}`:{log}"));
        }
    }
    let binner = log.matches("  CPU BINNER: TA done 5;").count();
    let repeated = log
        .lines()
        .filter(|l| l.starts_with("  CPU BINNER:") && l.contains("frames 2-5 as the first: 4 of 4"))
        .count();
    let binned = log.matches("    binned: ").count();
    let ta = log.lines().filter(|l| l.starts_with("  TA, ")).count();
    if binner != 6 || repeated != 6 || binned != 6 || ta != 30 || log.contains("render past 2 s") {
        return Err(format!(
            "tabench: {binned} sets binned, {binner} binner ways drawn, {repeated} repeatable, {ta} TA ways run (6, 6, 6, 30 wanted, no render past 2 s):{log}"
        ));
    }
    eprintln!(
        "    tabench OK: the binner drew all six sets repeatably on the modelled 3D core; all 30 TA ways ran and timed out safely (the fork has no TA); back at the loader on a button press"
    );
    boot_loader_tabench_isolate(root)
}

/// Run 3 of rp-tabench (`RP_TABENCH_ISOLATE=1`, audit C79: the core reset): every way's
/// binner reference draws on the modelled 3D core; every TA frame times
/// out (the fork has no TA) and the run moves on; it returns on a button.
fn boot_loader_tabench_isolate(root: &Path) -> Result<(), String> {
    let release = crate::arm_release(root);
    let dir = emu_dir(root)?;
    let loader = dir.join("rp-loader.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    let bin = variant_image(root, "rp-tabench", ("RP_TABENCH_ISOLATE", "1"))?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let check = || -> Result<(), String> {
        let mut mon = crate::screen::Monitor::connect(monitor_port())?;
        mon.query("sendkey ret 200")?;
        Ok(())
    };
    let output = boot(
        root,
        &Boot {
            image: &loader,
            marker: LOADER_MARKER,
            screenshot: None,
            sd_image: None,
            steps: &[],
            extra: &[],
            input: Some((&rp_loadproto::frame(&payload), "RETROPANE-TABENCH: DONE")),
            during: Some(&check),
            then: Some("RPL END"),
            timeout: Duration::from_secs(1800),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("tabench run 3: never came back to the loader after the button press")?;
    if crashed(log) || log.contains("did not return") {
        return Err(format!("tabench run 3: didn't return cleanly:{log}"));
    }
    let references = log.matches("  reference (the binner) rendered in ").count();
    let ways = log.lines().filter(|l| l.starts_with("WAY ")).count();
    let done = "RETROPANE-TABENCH: DONE - run 3: 0 of 30 TA frames drawn like the binner";
    if ways != 6 || references != 6 || !log.contains(done) || log.contains("render past 2 s") {
        return Err(format!(
            "tabench run 3: {ways} ways, {references} references drawn (6 and 6 wanted, every TA frame timed out, no render past 2 s):{log}"
        ));
    }
    eprintln!(
        "    tabench run 3 OK: all 6 ways' binner references drew on the modelled 3D core; all 30 TA frames timed out safely (the fork has no TA)"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    /// Every scratch file and raw binary goes through the helpers that
    /// create their folders: a check that built a path by hand failed in
    /// any fresh checkout ("no screenshot at ...ppm", QEMU can't make the
    /// folder), or relied on a file an earlier check left in target/emu.
    #[test]
    fn emulator_files_go_through_the_helpers() {
        let src = include_str!("emu.rs");
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
        let src = include_str!("emu.rs");
        let code = src.split("#[cfg(test)]\nmod tests").next().expect("code");
        let kills = code
            .lines()
            .filter(|l| !l.trim_start().starts_with("//") && l.contains("pkill"))
            .count();
        assert_eq!(kills, 2, "pkill outside stop_own_qemu / stop_own_load");
    }

    /// The body of `pub fn name` in this file.
    fn body(name: &str) -> &'static str {
        let src = include_str!("emu.rs");
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
        let src = include_str!("emu.rs");
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
            if line.starts_with("fn poll_pause") {
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
        let loader = include_str!("../../boot/loader/src/main.rs");
        assert!(
            loader.contains("naps mid-command {}"),
            "the loader reports it"
        );
        let src = include_str!("emu.rs");
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
        let src = include_str!("emu.rs");
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
            include!("../../tools/build/local_input.rs");
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
        let src = include_str!("emu.rs");
        let mut seen = 0;
        for call in ["usb_connect(", "usb_args(", "msc_args(", "usb_port("] {
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
