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
pub const QEMU_RELEASE: &str = "rp-v0.29.0";
const RELEASE_URL: &str = "https://github.com/Drakey83/retropane-qemu/releases/download";

/// How long an image gets to print its success marker.
const BOOT_TIMEOUT: Duration = Duration::from_secs(20);

/// Must match `rp_hello::MARKER`.
const HELLO_MARKER: &str = "RETROPANE-HELLO-OK";

/// Where `cargo xtask qemu` builds inside WSL.
const WSL_QEMU_DEFAULT: &str = "$HOME/retropane-qemu/build/qemu-system-arm";

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

fn emulator(root: &Path) -> Result<Emulator, String> {
    match std::env::var("RETROPANE_QEMU") {
        Ok(v) if v.eq_ignore_ascii_case("wsl") => return Ok(Emulator::Wsl),
        // The published release for this platform (the release check: the
        // build contributors download, not the local one).
        Ok(v) if v.eq_ignore_ascii_case("release") => {
            return download_release(root).map(Emulator::Native);
        }
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
    crate::sdkit::raw_binary(
        &root
            .join("target/armv5te-x51v-none-eabi/release")
            .join(image),
        &out,
    )?;
    Ok(out)
}

/// Screen size of the emulated X51v, for converting taps to absolute pointer units.
const SCREEN: (u32, u32) = (480, 640);

/// Boots the bring-up image, checks it reports success over the UART,
/// screenshots the emulated display and verifies the test pattern, then
/// presses buttons and checks the image reports them.
pub fn boot_hello(root: &Path) -> Result<(), String> {
    let image = root.join("target/armv5te-x51v-none-eabi/release/rp-hello");
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
    let image = root.join("target/armv5te-x51v-none-eabi/release/rp-kernel");
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
    let image = root.join("target/armv5te-x51v-none-eabi/release/rp-threads");
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
    let image = root.join("target/armv5te-x51v-none-eabi/release/rp-quantum");
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
    let image = root.join("target/armv5te-x51v-none-eabi/release/rp-light");
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
    let release = root.join("target/armv5te-x51v-none-eabi/release");
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
    let release = root.join("target/armv5te-x51v-none-eabi/release");
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
    let release = root.join("target/armv5te-x51v-none-eabi/release");
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
    let release = root.join("target/armv5te-x51v-none-eabi/release");
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
    let release = root.join("target/armv5te-x51v-none-eabi/release");
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
    let release = root.join("target/armv5te-x51v-none-eabi/release");
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
    let release = root.join("target/armv5te-x51v-none-eabi/release");
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
    let release = root.join("target/armv5te-x51v-none-eabi/release");
    let dir = emu_dir(root)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let loader = dir.join("rp-loader.bin");
    let image = dir.join("rp-refresh.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-refresh"), &image)?;
    let payload = std::fs::read(&image).map_err(|e| format!("{}: {e}", image.display()))?;
    let frame = rp_loadproto::frame(&payload);
    let check = || -> Result<(), String> {
        let key = |k: &str| -> Result<(), String> {
            crate::screen::Monitor::connect(monitor_port())?.query(&format!("sendkey {k} 200"))?;
            Ok(())
        };
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

/// The crash path: the loader starts an image that panics on purpose; the
/// panic must come back to the loader (no watchdog reset), reach the PC as
/// a crash report, and be kept on the card as `crashlog.txt`.
pub fn boot_loader_crash(root: &Path) -> Result<(), String> {
    let release = root.join("target/armv5te-x51v-none-eabi/release");
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
    eprintln!(
        "    crash path OK: the image's crash box showed on the plane on screen (offset 0, as WM          leaves it); the panic came back to the loader, reached the PC and the card, and is on          the screen"
    );
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
    let release = root.join("target/armv5te-x51v-none-eabi/release");
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
    let release = root.join("target/armv5te-x51v-none-eabi/release");
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
    let release = root.join("target/armv5te-x51v-none-eabi/release");
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
    if log.contains("FAULT") || log.contains("PANIC") || log.contains("did not return") {
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
pub fn boot_loader_blend(root: &Path) -> Result<(), String> {
    use rp_drv_mbx_display::blendtest::{self, Stage};
    let release = root.join("target/armv5te-x51v-none-eabi/release");
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
    if log.contains("FAULT") || log.contains("PANIC") || log.contains("did not return") {
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
    let release = root.join("target/armv5te-x51v-none-eabi/release");
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
        mon.command("sendkey ret")?;
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
    if log.contains("FAULT") || log.contains("PANIC") || log.contains("did not return") {
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

/// The blend and display-event probe (rp-mbx-blend, renderer step 1): the
/// loader starts it; it must run every stage to its verdict without a
/// fault, put the blend state and the display registers back, and return
/// on a button. Under `-icount` (the event stages time scan lines), on an
/// emulator that models the blend and the display's events (`mbx-blend`,
/// rp-v0.13.0), its B1-B3 pixels and E1/E3/E4 timings must match the
/// device's run (docs/hw/emulator.md).
/// The blend and display-event probe (rp-mbx-blend, renderer step 1): the
/// loader starts it; it must run every stage to its verdict without a
/// fault, put the blend state and the display registers back, and return
/// on a button. Under `-icount` (the event stages time scan lines), on an
/// emulator that models the blend and the display's events (`mbx-blend`,
/// rp-v0.13.0), its B1-B3 pixels and E1/E3/E4 timings must match the
/// device's run (docs/hw/emulator.md).
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
    let release = root.join("target/armv5te-x51v-none-eabi/release");
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
    if log.contains("FAULT") || log.contains("PANIC") || log.contains("did not return") {
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
    if log.contains("B one-clock") || log.contains("FAULT") || log.contains("PANIC") {
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
    let release = root.join("target/armv5te-x51v-none-eabi/release");
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
    if log.contains("FAULT") || log.contains("PANIC") || log.contains("did not return") {
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
    if log.contains("FAULT") || log.contains("PANIC") || log.contains("did not return") {
        return Err(format!("glesfeat: didn't return cleanly:{log}"));
    }
    for want in [
        "parameter memory ready: true",
        "A always, ref 0 (control)",
        "A less 0 (version 23's depth 'less')",
        "D blue z 0.8 mode 1 (version 23's form)",
        "L code 10, texels 0x60, TEX2 0x86086610",
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
        return Ok(());
    }
    eprintln!("    glesfeat OK: every case ran and was logged, back at the loader");
    Ok(())
}

/// The MBX 3D core's first-triangle test (rp-mbx3d, ADR-0024 M4): runs
/// under the loader, every try timed out (no 3D model), back on a button.
pub fn boot_loader_mbx3d(root: &Path) -> Result<(), String> {
    if !device_has(root, "intel-2700g", "mbx-2d") {
        eprintln!("    mbx3d: skipped (this emulator release doesn't model the MBX 2D engine)");
        return Ok(());
    }
    let release = root.join("target/armv5te-x51v-none-eabi/release");
    let dir = emu_dir(root)?;
    let loader = dir.join("rp-loader.bin");
    let bin = dir.join("rp-mbx3d.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-mbx3d"), &bin)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let check = || -> Result<(), String> {
        let mut mon = crate::screen::Monitor::connect(monitor_port())?;
        mon.command("sendkey ret")?;
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
    if log.contains("FAULT") || log.contains("PANIC") || log.contains("did not return") {
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
    let release = root.join("target/armv5te-x51v-none-eabi/release");
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
    if log.contains("FAULT") || log.contains("PANIC") || log.contains("did not return") {
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
    if log.contains("ok  cycle counter agrees") {
        if !log.contains(" checks, 0 BAD") || log.contains("skipped") {
            return Err(format!(
                "clocks: the modelled clocks must pass every check:{log}"
            ));
        }
        eprintln!("    clocks OK: every point visited, every check passed on the modelled clocks");
        return Ok(());
    }
    eprintln!("    clocks: returned cleanly; run log:{log}");
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
    let release = root.join("target/armv5te-x51v-none-eabi/release");
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
    if log.contains("FAULT") || log.contains("PANIC") || !log.contains("RETROPANE-CS3BUS-DONE") {
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
/// run F (text as 4 bpp strips, `RP_GPUBENCH_TEXT`), run P (the 3D
/// core's target formats, `RP_GPUBENCH_TARGETS`) and the clock runs,
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
    let release = root.join("target/armv5te-x51v-none-eabi/release");
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
    let runs: [GpubenchRun; 19] = [
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
    for (number, (name, bin, want, needs_model, (first, every, presses))) in (1u32..).zip(runs) {
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
                mon.command("sendkey ret")?;
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
        if log.contains("FAULT") || log.contains("PANIC") || log.contains("did not return") {
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
        // Run G steps the clocks only from Windows Mobile's; a fork whose
        // core PLL reads 0 (rp-v0.29.0 and older: docs/hw/emulator.md) gets
        // the refusal, and one that starts as Windows Mobile leaves the
        // chip must trace the lock bit, run both passes with every step
        // GOOD twice (the models don't fail at any clock) and every control
        // GOOD, and put everything back.
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
    let release = root.join("target/armv5te-x51v-none-eabi/release");
    let dir = emu_dir(root)?;
    let loader = dir.join("rp-loader.bin");
    let bin = dir.join("rp-mbx3d-api.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-mbx3d-api"), &bin)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let check = || -> Result<(), String> {
        let mut mon = crate::screen::Monitor::connect(monitor_port())?;
        mon.command("sendkey ret")?;
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
    if log.contains("FAULT") || log.contains("PANIC") || log.contains("did not return") {
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
    let release = root.join("target/armv5te-x51v-none-eabi/release");
    let dir = emu_dir(root)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let loader = dir.join("rp-loader.bin");
    let bin = dir.join("rp-mved.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-mved"), &bin)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let check = || -> Result<(), String> {
        let mut mon = crate::screen::Monitor::connect(monitor_port())?;
        mon.command("sendkey ret")?;
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
    if log.contains("FAULT") || log.contains("PANIC") || log.contains("did not return") {
        return Err(format!("mved: didn't return cleanly:{log}"));
    }
    for want in [
        "S0: M24CLK 0x0",
        "set to 0x3",
        "S1: reset; FIFO 0x00000000; status 0x00000000: passed",
        "S2: 64 x 64 4:2:0 at 0x400000 to YUY2 at 0x410000",
        "job 0x30=0x80000 0x34=0x82000 0x38=0x20040 0x3c=0x4020 0x48=0x1",
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

pub fn boot_loader_mbx_blend(root: &Path) -> Result<(), String> {
    if !device_has(root, "intel-2700g", "mbx-2d") {
        eprintln!("    mbx-blend: skipped (this emulator release doesn't model the MBX 2D engine)");
        return Ok(());
    }
    let release = root.join("target/armv5te-x51v-none-eabi/release");
    let dir = emu_dir(root)?;
    let loader = dir.join("rp-loader.bin");
    let bin = dir.join("rp-mbx-blend.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-mbx-blend"), &bin)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let check = || -> Result<(), String> {
        let mut mon = crate::screen::Monitor::connect(monitor_port())?;
        mon.command("sendkey ret")?;
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
    if log.contains("FAULT") || log.contains("PANIC") || log.contains("did not return") {
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
    Ok(())
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
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| [p[2], p[1], p[0]])
            .collect();
        screens.push(out);
    }
    screens
}

/// The wallpaper comparison's screens (the test image's "Wallpaper
/// compare", the owner's 16-bit check): the wallpaper alone at full depth,
/// then at 16 bits, each once its notice has gone, through the same shell
/// and renderer code as the image.
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
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| [p[2], p[1], p[0]])
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
    let full = step(&mut shell, &mut gpu);
    let act = tap(&mut shell, rp_draw::Rect::new(200, 300, 80, 40));
    assert_eq!(act, Action::WallpaperDepth { sixteen: true });
    rp_today::load_wallpaper(&mut gpu, wall, rp_today::Depth::Rgb565, &mut scratch);
    gpu.wallpaper_changed();
    step(&mut shell, &mut gpu);
    shell.tick(20_000);
    let low = step(&mut shell, &mut gpu);
    [full, low]
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
    let release = root.join("target/armv5te-x51v-none-eabi/release");
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
        // entry from the end): the wallpaper alone, pixel-exact at full
        // depth and at 16 bits once each notice has gone; a tap flips the
        // depth, Home goes back. Ends at full depth, so the menu below
        // matches as before.
        let compare = rp_shell::Shell::menu_entry(entries - 3);
        press(
            &mut qmp,
            "compare",
            compare.x + compare.w / 2,
            compare.y + compare.h / 2,
            None,
        )?;
        until("bare-32", &bare[0])?;
        press(&mut qmp, "flip-16", 240, 320, Some(TAP_GUEST_MS))?;
        until("bare-16", &bare[1])?;
        press(&mut qmp, "flip-32", 240, 320, Some(TAP_GUEST_MS))?;
        until("bare-32-again", &bare[0])?;
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
    if log.contains("FAULT") || log.contains("PANIC") || log.contains("did not return") {
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
        "    today OK: Today screen pixel-exact (clocks masked) and still up after 40 s idle, Start opened the menu pixel-exact, the align screen calibrated ({}) and the menu opened pixel-exact again, the wallpaper alone pixel-exact at 32 and 16 bits, Back to loader returned; {}; stack {} of {} bytes",
        calibrated.trim(),
        line.trim(),
        stack.0,
        stack.1
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
fn device_has(root: &Path, device: &str, prop: &str) -> bool {
    let out = match emulator(root) {
        Ok(Emulator::Native(exe)) => Command::new(exe)
            .args(["-M", "x51v", "-device", &format!("{device},help")])
            .output(),
        Ok(Emulator::Wsl) => {
            wsl_bash(&format!("{} -M x51v -device {device},help", wsl_qemu())).output()
        }
        Err(_) => return false,
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
        Err(_) => return false,
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
        Err(_) => return false,
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
    let image = root.join("target/armv5te-x51v-none-eabi/release/rp-usb");
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
    let release = root.join("target/armv5te-x51v-none-eabi/release");
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
    let release = root.join("target/armv5te-x51v-none-eabi/release");
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
    crate::sdkit::raw_binary(
        &root.join("target/armv5te-x51v-none-eabi/release/rp-loader"),
        &loader,
    )?;
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
        // before the closing line, but for the loader's first two.
        let writes: usize = closing
            .split(" sector writes")
            .next()
            .and_then(|h| h.rsplit(' ').next())
            .and_then(|n| n.parse().ok())
            .ok_or_else(|| format!("cardlog {name}: no write count in: {closing}"))?;
        let lines = card.lines().count() - 1;
        if writes + 2 < lines {
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
    let release = root.join("target/armv5te-x51v-none-eabi/release");
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
    if log.contains("FAULT") || log.contains("PANIC") || !log.contains("RETROPANE-STORFAST: done") {
        return Err(format!("storfast: didn't finish cleanly:{log}"));
    }
    let dma_window = emulator_accepts_global(root, "pxa2xx-pcmcia.word-cycles=on");
    let mut want = vec![
        "storfast-quick.txt found",
        "C3 3 attach cycles: all good",
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
    for line in log
        .lines()
        .filter(|l| l.starts_with("S2 soak") || l.starts_with("C2 read"))
    {
        let clean = line.contains(" 0 write errors, 0 read errors, 0 blocks differ")
            || (line.contains("0 commands failed, 0 sectors differ"));
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
        &root.join("target/armv5te-x51v-none-eabi/release/rp-loader"),
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
    let name = match also {
        Some(a) => format!("{}-{}", env.0, a.0).to_ascii_lowercase(),
        None => env.0.to_ascii_lowercase(),
    };
    let dir = format!("target/variants/{name}");
    let mut cmd = Command::new("cargo");
    cmd.current_dir(root).env(env.0, env.1);
    if let Some((k, v)) = also {
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
    let out = emu_dir(root)?.join(format!("{image}-{name}.bin"));
    crate::sdkit::raw_binary(
        &root
            .join(&dir)
            .join("armv5te-x51v-none-eabi/release")
            .join(image),
        &out,
    )?;
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
        let mut last = 0;
        let mut below = 0;
        let mut seen = [0usize; 24];
        let (mut run, mut gaps, mut longest) = (0usize, 0usize, 0usize);
        for &(k, v) in p {
            match v {
                Win::Note(n) => {
                    below = if n + 1 < last { below + 1 } else { 0 };
                    if below >= 2 {
                        return Err(format!(
                            "audio E3: in {name} the scale went down at {:.2} s (note {} after {}): a replay",
                            k as f64 / 50.0,
                            n + 1,
                            last + 1
                        ));
                    }
                    last = last.max(n);
                    seen[n] += 1;
                }
                Win::Quiet => {
                    run += 1;
                    continue;
                }
                Win::Other => {}
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
        &root.join("target/armv5te-x51v-none-eabi/release/rp-loader"),
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
    let release = root.join("target/armv5te-x51v-none-eabi/release");
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
    let image = root.join("target/armv5te-x51v-none-eabi/release/rp-usb");
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
];

/// Boots the probe image (the real-device report), then checks every page
/// line's checksum, that the report is complete, and that it's on screen.
pub fn boot_probe(root: &Path) -> Result<(), String> {
    let image = root.join("target/armv5te-x51v-none-eabi/release/rp-probe");
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
    if !(1..=4).contains(&page) || text.len() != 40 {
        return Err(format!("malformed report line `{l}`"));
    }
    let (content, sum) = text.split_at(37);
    let want = u8::from_str_radix(sum.trim(), 16).map_err(|_| format!("bad checksum in `{l}`"))?;
    if probe_checksum(content) != want {
        return Err(format!("checksum mismatch in `{l}`"));
    }
    Ok(page)
}

/// Verifies the `P<page>|<line>` copy of the report on serial. Returns the
/// number of lines.
pub fn check_probe_report(output: &str) -> Result<usize, String> {
    let mut pages = [0usize; 4];
    for l in output.lines() {
        if !l.starts_with('P') || !l.contains('|') {
            continue;
        }
        pages[check_report_line(l)? - 1] += 1;
    }
    if let Some(p) = pages.iter().position(|&n| n == 0) {
        return Err(format!("probe report page {} is empty", p + 1));
    }
    for want in PROBE_EXPECT {
        if !output.contains(want) {
            return Err(format!("probe report lacks `{want}`"));
        }
    }
    if output.contains("OVERFLOW") {
        return Err("probe report overflowed a page".into());
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
    let image = root.join("target/armv5te-x51v-none-eabi/release/rp-bench");
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
