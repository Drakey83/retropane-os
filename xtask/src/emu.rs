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

/// Emulator release this checkout is tested against. Bump deliberately.
pub const QEMU_RELEASE: &str = "rp-v0.20.0";
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

const EMU_ARGS: &str = "-M x51v -display none";

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

/// Screen size of the emulated X51v, for converting taps to absolute pointer units.
const SCREEN: (u32, u32) = (480, 640);

/// Boots the bring-up image, checks it reports success over the UART,
/// screenshots the emulated display and verifies the test pattern, then
/// presses buttons and checks the image reports them.
pub fn boot_hello(root: &Path) -> Result<(), String> {
    let image = root.join("target/armv5te-x51v-none-eabi/release/rp-hello");
    let shot = root
        .join("target")
        .join("emulator")
        .join("hello-screen.ppm");
    let sd = write_sd_test_image(&root.join("target").join("emulator").join("sd-test.img"))?;
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
    let dir = root.join("target/emu");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
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
    let dir = root.join("target/emu");
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
    let dir = root.join("target/emu");
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
    let dir = root.join("target/emu");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let loader = dir.join("rp-loader.bin");
    let image = dir.join("rp-nightlight.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-nightlight"), &image)?;
    let payload = std::fs::read(&image).map_err(|e| format!("{}: {e}", image.display()))?;
    let frame = rp_loadproto::frame(&payload);
    let shot = root.join("target/emulator/nightlight-screen.ppm");
    std::fs::create_dir_all(root.join("target/emulator")).map_err(|e| e.to_string())?;
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
            std::thread::sleep(Duration::from_millis(500));
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
    let dir = root.join("target/emu");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let loader = dir.join("rp-loader.bin");
    let image = dir.join("rp-display-cal.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-display-cal"), &image)?;
    let payload = std::fs::read(&image).map_err(|e| format!("{}: {e}", image.display()))?;
    let frame = rp_loadproto::frame(&payload);
    let shot = root.join("target/emulator/display-cal-screen.ppm");
    std::fs::create_dir_all(root.join("target/emulator")).map_err(|e| e.to_string())?;
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
                let _ = std::fs::copy(
                    &shot,
                    root.join(format!("target/emulator/display-cal-{name}.ppm")),
                );
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

/// The screen measurement image (`cargo xtask measure`'s patches) under the
/// loader: the whole schedule runs (about 8 minutes), each patch within
/// 300 ms of its planned start, the grey 128 patch is exactly grey 128 on
/// screen (the straight gamma table), and everything is put back.
pub fn boot_loader_screen_measure(root: &Path) -> Result<(), String> {
    use rp_screen_measure::{PATCHES, start_ms, total_ms};
    let release = root.join("target/armv5te-x51v-none-eabi/release");
    let dir = root.join("target/emu");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let loader = dir.join("rp-loader.bin");
    let image = dir.join("rp-screen-measure.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-screen-measure"), &image)?;
    let payload = std::fs::read(&image).map_err(|e| format!("{}: {e}", image.display()))?;
    let frame = rp_loadproto::frame(&payload);
    let shot = root.join("target/emulator/screen-measure.ppm");
    std::fs::create_dir_all(root.join("target/emulator")).map_err(|e| e.to_string())?;
    let guest = match emulator(root)? {
        Emulator::Wsl => to_wsl_path(&shot),
        Emulator::Native(_) => shot.display().to_string(),
    };
    let grey = PATCHES
        .iter()
        .position(|p| p.name == "grey 128")
        .ok_or("no grey 128 patch")?;
    let check = || -> Result<(), String> {
        let t0 = Instant::now();
        // Watch from the patch's start until its end: grey 128 everywhere.
        let from = Duration::from_millis(u64::from(start_ms(grey)) + 1_000);
        let to = Duration::from_millis(u64::from(start_ms(grey) + PATCHES[grey].ms));
        std::thread::sleep(from);
        let mut seen = None;
        while t0.elapsed() < to {
            let _ = std::fs::remove_file(&shot);
            crate::screen::Monitor::connect(monitor_port())?.screendump(&guest, &shot)?;
            let img = crate::screen::parse_ppm(&std::fs::read(&shot).map_err(|e| e.to_string())?)?;
            let all = img
                .rgb
                .chunks(3)
                .take((img.width * 640) as usize)
                .all(|c| c == [128, 128, 128]);
            seen = Some(img.rgb.get(..3).map(<[u8]>::to_vec));
            if all {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        Err(format!(
            "screen-measure: the grey 128 patch never showed as grey 128 (first pixel {seen:?})"
        ))
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
            timeout: Duration::from_millis(u64::from(total_ms()) + 180_000),
        },
    )?;
    let log = output
        .rsplit_once("RPL LOG")
        .map(|(_, l)| l)
        .ok_or("screen-measure: no run log after the image returned")?;
    for (i, p) in PATCHES.iter().enumerate() {
        let head = format!("patch {i} {} at ", p.name);
        let at = log
            .lines()
            .find_map(|l| l.trim().strip_prefix(&head))
            .and_then(|r| r.split(' ').next())
            .and_then(|n| n.parse::<u32>().ok())
            .ok_or_else(|| format!("screen-measure: no {head:?} in the run log:{log}"))?;
        if at.abs_diff(start_ms(i)) > 300 {
            return Err(format!(
                "screen-measure: patch {i} started at {at} ms, planned {}",
                start_ms(i)
            ));
        }
    }
    for want in ["all patches shown", "SCREEN-MEASURE-OK"] {
        if !log.contains(want) {
            return Err(format!("screen-measure: no {want:?} in the run log:{log}"));
        }
    }
    eprintln!(
        "    screen-measure OK: {} patches on schedule (each within 300 ms), grey 128 exact on screen, put back and returned",
        PATCHES.len()
    );
    Ok(())
}

/// The refresh-rate test (display calibration C3) under the loader. The
/// emulator's display PLL reads 0, not Windows Mobile's value, so the image
/// must refuse to switch and only measure: the emulated frame is fixed at
/// the device's 56.46 Hz (the PLL isn't modelled, docs/hw/emulator.md), and
/// the counted rate must come out within 1 Hz of it, twice (at the start,
/// then after D-pad centre); Home puts everything back.
pub fn boot_loader_refresh(root: &Path) -> Result<(), String> {
    let release = root.join("target/armv5te-x51v-none-eabi/release");
    let dir = root.join("target/emu");
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
        std::thread::sleep(Duration::from_secs(5));
        key("ret")?;
        std::thread::sleep(Duration::from_secs(5));
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
    if rates.len() != 2 || rates.iter().any(|r| r.abs_diff(5646) > 100) {
        return Err(format!(
            "refresh: want two rates within 1 Hz of 56.46 Hz, got {rates:?} (centi-Hz):{log}"
        ));
    }
    eprintln!(
        "    refresh OK: no PLL switch without Windows Mobile's value; frames counted at {:?} centi-Hz (56.46 Hz modelled); Home put back and returned",
        rates
    );
    Ok(())
}

/// The crash path: the loader starts an image that panics on purpose; the
/// panic must come back to the loader (no watchdog reset), reach the PC as
/// a crash report, and be kept on the card as `crashlog.txt`.
pub fn boot_loader_crash(root: &Path) -> Result<(), String> {
    let release = root.join("target/armv5te-x51v-none-eabi/release");
    let dir = root.join("target/emu");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
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
    let sd = root.join("target/emulator/sd-crash.img");
    std::fs::create_dir_all(root.join("target/emulator")).map_err(|e| e.to_string())?;
    std::fs::write(&sd, &image).map_err(|e| format!("{}: {e}", sd.display()))?;
    // First the image's own crash box, on the plane actually on screen:
    // rp-crash moved it to offset 0 (where Windows Mobile leaves it), so a
    // box drawn at the conventional 0x60000 never shows. Then, once the
    // loader is back, its crash screen: a red band at the top of its log
    // area (below the 96-pixel status lines) and the log's end.
    let shot = root.join("target/emulator/crash-screen.ppm");
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
        std::thread::sleep(Duration::from_millis(1000));
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
/// backstop at 31 s.
pub fn boot_loader_hang(root: &Path) -> Result<(), String> {
    let loader = root.join("target/emu/rp-loader.bin");
    crate::sdkit::raw_binary(
        &root.join("target/armv5te-x51v-none-eabi/release/rp-loader"),
        &loader,
    )?;
    let mut hang = rp_loadproto::control_frame("limit 1");
    hang.extend(rp_loadproto::frame(&0xEAFF_FFFEu32.to_le_bytes()));
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
    let dir = root.join("target/emu");
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
        std::thread::sleep(Duration::from_secs(150));
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
    let dir = root.join("target/emu");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
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
    let dir = root.join("target/emu");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let loader = dir.join("rp-loader.bin");
    let bin = dir.join("rp-picture.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-picture"), &bin)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let want = expected_picture(root)?;
    let shot = root.join("target/emulator/picture-screen.ppm");
    let guest = match emulator(root)? {
        Emulator::Wsl => to_wsl_path(&shot),
        Emulator::Native(_) => shot.display().to_string(),
    };
    let check = || -> Result<(), String> {
        // Called as soon as the image says the picture is up.
        let shown = Instant::now();
        // The emulator redraws its screen on a timer: give it time to show
        // the new frame (as the crash-screen check does).
        std::thread::sleep(Duration::from_millis(1500));
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
        // Stay up until it has had time to return (20 s shown, then the
        // loader's run log reaches the PC).
        let wait = Duration::from_secs(u64::from(rp_picture_seconds()) + 6);
        std::thread::sleep(wait.saturating_sub(shown.elapsed()));
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
    let dir = root.join("target/emu");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let loader = dir.join("rp-loader.bin");
    let bin = dir.join("rp-blend.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-blend"), &bin)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let want: Vec<Vec<u8>> = Stage::ALL.iter().map(|&s| blend_screen(s)).collect();
    let wsl = matches!(emulator(root)?, Emulator::Wsl);
    // Kept per stage (target/emulator/blend-A.ppm ...) for a look by eye.
    let dump = |name: char| -> Result<Vec<u8>, String> {
        let shot = root.join(format!("target/emulator/blend-{name}.ppm"));
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
                std::thread::sleep(Duration::from_millis(700));
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
            std::thread::sleep(Duration::from_millis(700));
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
    let dir = root.join("target/emu");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let loader = dir.join("rp-loader.bin");
    let bin = dir.join("rp-mbx.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-mbx"), &bin)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let shot = root.join("target/emulator/mbx-screen.ppm");
    let guest = match emulator(root)? {
        Emulator::Wsl => to_wsl_path(&shot),
        Emulator::Native(_) => shot.display().to_string(),
    };
    let check = || -> Result<(), String> {
        // Give the emulator's screen timer a frame to show the picture.
        std::thread::sleep(Duration::from_millis(1500));
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
        std::thread::sleep(Duration::from_secs(4));
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
/// The USB disconnect hunt (rp-unplug, docs/hw/udc.md T4): every candidate
/// tried and undone, the host controller's port 2 reached, the run back at
/// the loader with no fault. The emulator can't say which candidate
/// disconnects on the device (its pull-up isn't modelled): this only proves
/// the image is safe to send.
pub fn boot_loader_unplug(root: &Path) -> Result<(), String> {
    let release = root.join("target/armv5te-x51v-none-eabi/release");
    let dir = root.join("target/emu");
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

/// The MBX 3D core's first-triangle test (rp-mbx3d, ADR-0024 M4): runs
/// under the loader, every try timed out (no 3D model), back on a button.
pub fn boot_loader_mbx3d(root: &Path) -> Result<(), String> {
    if !device_has(root, "intel-2700g", "mbx-2d") {
        eprintln!("    mbx3d: skipped (this emulator release doesn't model the MBX 2D engine)");
        return Ok(());
    }
    let release = root.join("target/armv5te-x51v-none-eabi/release");
    let dir = root.join("target/emu");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let loader = dir.join("rp-loader.bin");
    let bin = dir.join("rp-mbx3d.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-mbx3d"), &bin)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let check = || -> Result<(), String> {
        let mut mon = crate::screen::Monitor::connect(monitor_port())?;
        mon.command("sendkey ret")?;
        std::thread::sleep(Duration::from_secs(4));
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
    let dir = root.join("target/emu");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let loader = dir.join("rp-loader.bin");
    let bin = dir.join("rp-mbx3d-api.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-mbx3d-api"), &bin)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let check = || -> Result<(), String> {
        let mut mon = crate::screen::Monitor::connect(monitor_port())?;
        mon.command("sendkey ret")?;
        std::thread::sleep(Duration::from_secs(4));
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

pub fn boot_loader_mbx_blend(root: &Path) -> Result<(), String> {
    if !device_has(root, "intel-2700g", "mbx-2d") {
        eprintln!("    mbx-blend: skipped (this emulator release doesn't model the MBX 2D engine)");
        return Ok(());
    }
    let release = root.join("target/armv5te-x51v-none-eabi/release");
    let dir = root.join("target/emu");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let loader = dir.join("rp-loader.bin");
    let bin = dir.join("rp-mbx-blend.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-mbx-blend"), &bin)?;
    let payload = std::fs::read(&bin).map_err(|e| format!("{}: {e}", bin.display()))?;
    let check = || -> Result<(), String> {
        let mut mon = crate::screen::Monitor::connect(monitor_port())?;
        mon.command("sendkey ret")?;
        std::thread::sleep(Duration::from_secs(4));
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

/// The shell's Today screen (ADR-0021) under the loader: pixel-exact
/// against the reference (clocks masked), still there after 40 s untouched
/// (past the run limit: idle sleeps must check in), a stylus tap on Start opens the
/// menu (pixel-exact again), and the menu's "Back to loader" returns with
/// the touch-to-flush latencies in the run log.
pub fn boot_loader_today(root: &Path) -> Result<(), String> {
    use rp_shell::Input;
    let release = root.join("target/armv5te-x51v-none-eabi/release");
    let dir = root.join("target/emu");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
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
        let shot = root.join(format!("target/emulator/today-{name}.ppm"));
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
            std::thread::sleep(Duration::from_millis(500));
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
                let _ = std::fs::write(
                    root.join(format!("target/emulator/today-{name}-want.ppm")),
                    ppm,
                );
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
        std::thread::sleep(Duration::from_secs(40));
        until("idle", &want[0])?;
        let mut qmp = crate::screen::Qmp::connect(monitor_port() + 1)?;
        let tap = |qmp: &mut crate::screen::Qmp, x: u16, y: u16| -> Result<(), String> {
            touch(qmp, &Action::TouchDown(x, y))?;
            std::thread::sleep(Duration::from_millis(300));
            touch(qmp, &Action::TouchUp)?;
            Ok(())
        };
        tap(&mut qmp, sx, sy)?;
        until("menu", &want[2])?;
        // The align screen (the test image's second-to-last entry): hold
        // each target, then the check dot, as the owner would.
        let entries = rp_shell::Shell::menu_entries(true);
        let align = rp_shell::Shell::menu_entry(entries - 2);
        tap(&mut qmp, align.x + align.w / 2, align.y + align.h / 2)?;
        std::thread::sleep(Duration::from_millis(500));
        for &(x, y) in rp_shell::align::TARGETS
            .iter()
            .chain([&rp_shell::align::CHECK])
        {
            touch(&mut qmp, &Action::TouchDown(x, y))?;
            std::thread::sleep(Duration::from_millis(700));
            touch(&mut qmp, &Action::TouchUp)?;
            std::thread::sleep(Duration::from_millis(400));
        }
        // Calibrated: Start must open the same menu, pixel for pixel (the
        // taps land where they should through the new calibration).
        tap(&mut qmp, sx, sy)?;
        until("menu-aligned", &want[2])?;
        // The wallpaper comparison (the owner's 16-bit check, the third
        // entry from the end): the wallpaper alone, pixel-exact at full
        // depth and at 16 bits once each notice has gone; a tap flips the
        // depth, Home goes back. Ends at full depth, so the menu below
        // matches as before.
        let compare = rp_shell::Shell::menu_entry(entries - 3);
        tap(
            &mut qmp,
            compare.x + compare.w / 2,
            compare.y + compare.h / 2,
        )?;
        until("bare-32", &bare[0])?;
        tap(&mut qmp, 240, 320)?;
        until("bare-16", &bare[1])?;
        tap(&mut qmp, 240, 320)?;
        until("bare-32-again", &bare[0])?;
        crate::screen::Monitor::connect(monitor_port())?.command("sendkey f4")?;
        std::thread::sleep(Duration::from_millis(500));
        tap(&mut qmp, sx, sy)?;
        until("menu-after-compare", &want[2])?;
        // The last menu entry is "Back to loader" (test images only).
        let back = rp_shell::Shell::menu_entry(entries - 1);
        tap(&mut qmp, back.x + back.w / 2, back.y + back.h / 2)?;
        // Back at the loader: the wallpaper between the clock and weather
        // panels (rows 210-217, nothing drawn over it) is gone. Polled, so
        // the run log has been sent when the emulator stops.
        let strip = |s: &[u8]| s[210 * rp_today::W * 3..218 * rp_today::W * 3].to_vec();
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            std::thread::sleep(Duration::from_millis(500));
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
            extra: &[],
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
    let dir = root.join("target/emu");
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
    let loader = root.join("target/emu/rp-loader.bin");
    let light = root.join("target/emu/rp-light.bin");
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
        // find the first run's state.
        s.write_all(&frame).map_err(|e| e.to_string())?;
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
    let dir = root.join("target/emu");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
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
    let dir = root.join("target/emu");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
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
        let image = root.join(format!("target/emulator/full-{gib}g.img"));
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
    let dir = root.join("target/emu");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    std::fs::create_dir_all(root.join("target/emulator")).map_err(|e| e.to_string())?;
    let loader = dir.join("rp-loader.bin");
    let image = dir.join("rp-storfast.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-storfast"), &image)?;
    let sd = root.join("target/emulator/sd-storfast.img");
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
    let cf = root.join("target/emulator/cf-storfast.img");
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
    let dir = root.join("target/emu");
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
    let loader = root.join("target/emu/rp-loader.bin");
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
    let cf = root.join("target/emulator/cf.img");
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
                    std::thread::sleep(Duration::from_millis(500));
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
    let release = root.join("target/armv5te-x51v-none-eabi/release");
    let dir = root.join("target/emu");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let loader = dir.join("rp-loader.bin");
    let image = dir.join("rp-tone.bin");
    crate::sdkit::raw_binary(&release.join("rp-loader"), &loader)?;
    crate::sdkit::raw_binary(&release.join("rp-tone"), &image)?;
    let payload = std::fs::read(&image).map_err(|e| format!("{}: {e}", image.display()))?;
    let frame = rp_loadproto::frame(&payload);
    let wav = root.join("target/emulator/tone.wav");
    let _ = std::fs::remove_file(&wav);
    let wav_path = match emulator(root)? {
        Emulator::Wsl => to_wsl_path(&wav),
        Emulator::Native(_) => wav.display().to_string(),
    };
    let port = monitor_port();
    let mut args: Vec<String> = usb_args(2).into();
    args.extend([
        "-audiodev".into(),
        format!("wav,id=snd,path={wav_path}"),
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
        std::thread::sleep(Duration::from_secs(4));
        let mut mon = crate::screen::Monitor::connect(port)?;
        mon.query("qom-set /machine/peripheral/jack pressed true")?;
        let log = usb_read_until(&mut s, "RPL END", 40)?;
        for want in [
            "tone 440 Hz 500 ms (24000 frames",
            "by DMA",
            "to speaker",
            "headphones in (GPIO83 1)",
            "tone 1000 Hz 50 ms",
            "to headphones",
            "TONE-OK",
        ] {
            if !log.contains(want) {
                return Err(format!("tone: no {want:?} in the run log:{log}"));
            }
        }
        if let Some(line) = log.lines().find(|l| l.starts_with("tone 440")) {
            eprintln!("    rp-tone: {}", line.trim());
        }
        // Let the backend write out what it has.
        std::thread::sleep(Duration::from_millis(500));
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
    let data = std::fs::read(&wav).map_err(|e| format!("{}: {e}", wav.display()))?;
    if data.len() < 44 || &data[..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err("tone: the emulator wrote no wav".into());
    }
    let rate = u32::from_le_bytes([data[24], data[25], data[26], data[27]]);
    let channels = usize::from(u16::from_le_bytes([data[22], data[23]]));
    let left: Vec<i16> = data[44..]
        .chunks_exact(2 * channels)
        .map(|f| i16::from_le_bytes([f[0], f[1]]))
        .collect();
    // The tone: the stretch between the first and last loud samples.
    let loud = |s: &i16| s.unsigned_abs() > 2000;
    let Some(start) = left.iter().position(loud) else {
        return Err("tone: the recording is silent".into());
    };
    // The first tone only: it ends at the first tenth of a second of quiet
    // (the second, to the headphones, comes seconds later).
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
    let tone = &left[start..=end];
    let crossings = tone.windows(2).filter(|w| (w[0] < 0) != (w[1] < 0)).count();
    let seconds = tone.len() as f64 / f64::from(rate);
    let hz = crossings as f64 / 2.0 / seconds;
    if !(425.0..=455.0).contains(&hz) || !(0.4..=0.6).contains(&seconds) {
        return Err(format!(
            "tone: recorded {hz:.1} Hz for {seconds:.3} s (wanted 440 Hz, 0.5 s)"
        ));
    }
    eprintln!(
        "    tone OK: WM8750 found on I2C, 440 Hz through I2S, recorded at {hz:.1} Hz for {seconds:.3} s"
    );
    Ok(())
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
    let loader = root.join("target/emu/rp-loader.bin");
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
    let sd = root.join("target/emulator/sd-exfat.img");
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
    let loader = root.join("target/emu/rp-loader.bin");
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
    let sd = root.join("target/emulator/sd-put.img");
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
    let dir = root.join("target/emu");
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
    let dir = root.join("target/emu");
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
    std::fs::create_dir_all(root.join("target/emulator")).map_err(|e| e.to_string())?;
    let sd = root.join("target/emulator/sd-stale.img");
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
    let loader = root.join("target/emu/rp-loader.bin");
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
    let sd = root.join("target/emulator/sd-drive.img");
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
        std::thread::sleep(Duration::from_millis(2500)); // the write grace
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
        std::thread::sleep(Duration::from_millis(2500));
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
        std::thread::sleep(Duration::from_millis(2500));
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
        // About 3 s in the emulator; a loader napping mid-transfer took 17.
        if t.elapsed() > Duration::from_secs(8) {
            return Err(format!(
                "drive: 1 MiB took {:?}, far slower than usual",
                t.elapsed()
            ));
        }
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
    let loader = root.join("target/emu/rp-loader.bin");
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
    let loader = root.join("target/emu/rp-loader.bin");
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
    let sd = root.join("target/emulator/sd-locked.img");
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
    let loader = root.join("target/emu/rp-loader.bin");
    let args = usb_args(9);
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let port = monitor_port();
    let check = || -> Result<(), String> {
        let mut s = usb_connect(9)?;
        usb_read_until(&mut s, "RPLOAD1 READY", 10)?;
        // An idle loader sleeps between polls, and still answers at once.
        std::thread::sleep(Duration::from_secs(2));
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
            let press = |mon: &mut crate::screen::Monitor| -> Result<(), String> {
                mon.query("qom-set /machine/peripheral/power pressed true")?;
                std::thread::sleep(Duration::from_millis(400));
                mon.query("qom-set /machine/peripheral/power pressed false")?;
                std::thread::sleep(Duration::from_millis(800));
                Ok(())
            };
            press(&mut mon)?;
            let off = mon.query("qom-get /machine/peripheral/pwm dcr0")?;
            press(&mut mon)?;
            let on = mon.query("qom-get /machine/peripheral/pwm dcr0")?;
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
    let shot = root
        .join("target")
        .join("emulator")
        .join("probe-screen.ppm");
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
    let sd = root.join("target/emulator/sd-probe-exfat.img");
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
            std::thread::sleep(Duration::from_millis(500));
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
    let sd = write_sd_test_image(&root.join("target").join("emulator").join("sd-bench.img"))?;
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
    let mut cmd = match emu {
        Emulator::Native(exe) => {
            eprintln!("    emulator: {}", exe.display());
            let mut c = Command::new(exe);
            c.args(EMU_ARGS.split(' '))
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
                "exec timeout {} {wsl_qemu} {EMU_ARGS} {} -serial {serial_arg} -monitor {monitor} -qmp {qmp} -kernel '{}'{sd}",
                timeout.as_secs() + 10,
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
        .and_then(|()| during.map_or(Ok(()), |f| f()))
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
        // it's gone before the next test binds the same ports. Only this
        // run's emulator, found by its own serial port: other sessions run
        // emulators at the same time on their own port blocks.
        let _ = wsl_bash(&format!(
            "pkill -f -- '-serial tcp:127.0.0.1:{},'; sleep 0.3; true",
            serial_port()
        ))
        .status();
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
            .map(|l| {
                format!(
                    "      {l}
"
                )
            })
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
