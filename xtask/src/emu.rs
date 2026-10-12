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
pub const QEMU_RELEASE: &str = "rp-v0.9.7";
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
        expect: &[Expect::Text("KEY Up down"), Expect::Text("KEY Up up")],
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
        expect: &[Expect::Text("KEY Home down"), Expect::Text("KEY Home up")],
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
    "VM unmap OK",
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
            steps: &[],
            extra: &[],
            input: None,
            during: None,
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
    eprintln!("    kernel VM OK: page tables, demand paging, copy-on-write, FCSE, unmap");
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
    "USER fault contained OK",
    "USER iWMMXt lazy switch OK",
    "USER KData OK",
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
        "    threads OK: context switches, events, sleeps, tickless idle, user mode, syscalls, preemption, KData"
    );
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
            timeout: BOOT_TIMEOUT,
        },
    )?;
    if !output.contains("RPL OK") {
        return Err("loader: no `RPL OK` before the image started".into());
    }
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
    });
    let sd = root.join("target/emulator/sd-crash.img");
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
            input: Some((&frame, "RPL END")),
            during: None,
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
        "    crash path OK: the panic came back to the loader, reached the PC and is on the card"
    );
    Ok(())
}

/// Sends the loader an image that hangs (one `b .` instruction, MMU and
/// IRQs off) and checks recovery: after the run limit the watchdog resets
/// the machine, the loader starts again and reports that the run didn't
/// return. The emulator restarts the loader itself; on the device Windows
/// Mobile's StartUp folder does (docs: SD kit README, loader/).
pub fn boot_loader_hang(root: &Path) -> Result<(), String> {
    let loader = root.join("target/emu/rp-loader.bin");
    crate::sdkit::raw_binary(
        &root.join("target/armv5te-x51v-none-eabi/release/rp-loader"),
        &loader,
    )?;
    let hang = rp_loadproto::frame(&0xEAFF_FFFEu32.to_le_bytes());
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
            timeout: Duration::from_secs(120),
        },
    )?;
    if !output.contains("RPL OK") {
        return Err("watchdog: the hanging image was never started".into());
    }
    eprintln!("    watchdog OK: a hung image reset the device and the loader reported it");
    Ok(())
}

/// Port of the emulated USB serial port (the UDC model's chardev); each test
/// gets its own (`n`), so one can never reach another test's emulator.
fn usb_port(n: u16) -> u16 {
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
            wsl_bash(&format!("{WSL_QEMU} -M x51v -device {device},help")).output()
        }
        Err(_) => return false,
    };
    out.is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains(&format!("{prop}=")))
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
            wsl_bash(&format!("{WSL_QEMU} -M x51v -device pxa27x-udc,help")).output()
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
                    return Ok(String::from_utf8_lossy(&got).into_owned());
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
            timeout: BOOT_TIMEOUT,
        },
    )?;
    eprintln!("    usb link OK: enumerated by the emulated host, banner and echo over USB");
    Ok(())
}

/// The loader over USB: first light sent through the emulated USB serial
/// port, its run log back the same way.
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
        usb_read_until(&mut s, "RPL OK", 15)?;
        // The loader disconnects to run the image and reconnects when it
        // returns (the emulated host enumerates it again); the socket stays
        // up throughout, and the run log arrives once the port is reopened.
        let log = usb_read_until(&mut s, "RPL END", 30)?;
        if !log.contains("all steps OK") {
            return Err(format!("run log over USB incomplete: {log:?}"));
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
            timeout: Duration::from_secs(60),
        },
    )?;
    eprintln!("    loader over USB OK: image in, run log out, through the emulated USB port");
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
    vol.read_back(name, &mut scratch, |c| out.extend_from_slice(c))
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

/// The CompactFlash slot (docs/hw/cf.md): a CF card image in socket 1,
/// identified through the loader (`cf`): its capacity, its write cache
/// turned off, its partition seen; and no SD card, so the CF path stands
/// alone.
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
    });
    let cf = root.join("target/emulator/cf.img");
    std::fs::write(&cf, image).map_err(|e| format!("{}: {e}", cf.display()))?;
    let mut args: Vec<String> = usb_args(2).into();
    args.extend(msc_args(3));
    args.push("-drive".into());
    args.push(drive_arg(root, "ide", &cf)?);
    let extra: Vec<&str> = args.iter().map(String::as_str).collect();
    let via_drive: Vec<u8> = (0..4096u32).map(|i| (i * 29 + 3) as u8).collect();
    let check = || -> Result<(), String> {
        let mut s = usb_connect(2)?;
        let mut bot = Bot::connect(3)?;
        bot.lun = 1;
        usb_read_until(&mut s, "RPLOAD1 READY", 10)?;
        // Before the owner attaches it: an empty card reader (after, at
        // most, one "medium changed" for the detect line settling).
        let mut r = bot.command(&[0; 6], 0, false, &[])?;
        let mut sense = bot.sense()?;
        if sense.1 == 0x28 {
            r = bot.command(&[0; 6], 0, false, &[])?;
            sense = bot.sense()?;
        }
        if r.status == 0 || sense.1 != 0x3A {
            return Err(format!(
                "CF unit before 'cf': status {}, sense {sense:x?}",
                r.status
            ));
        }
        s.write_all(&rp_loadproto::control_frame("cf"))
            .map_err(|e| e.to_string())?;
        let reply = usb_read_until(&mut s, "model", 20)?;
        let want = "RPL OK cf: 16 MB (32768 sectors), write cache off, FAT32 partition";
        if !reply.contains(want) {
            return Err(format!("unexpected reply to cf: {reply:?}"));
        }
        // Asked again: the same card, not re-initialised.
        s.write_all(&rp_loadproto::control_frame("cf"))
            .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, want, 20)?;
        // Now the PC sees it: medium changed once, then ready.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let r = bot.command(&[0; 6], 0, false, &[])?;
            if r.status == 0 {
                break;
            }
            let _ = bot.sense()?;
            if Instant::now() > deadline {
                return Err("CF unit never became ready".into());
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
            timeout: Duration::from_secs(60),
        },
    )?;
    let after = std::fs::read(&cf).map_err(|e| e.to_string())?;
    if after[30000 * 512..30000 * 512 + via_drive.len()] != via_drive[..] {
        return Err("CF: the drive's write isn't in the card image".into());
    }
    eprintln!(
        "    CF card OK: identified in socket 1 (16 MB, write cache off), then the PC's second drive: read, refused while read-only, written after drive rw, checked in the image"
    );
    Ok(())
}

/// Audio: `tone 440 500` through the loader (WM8750 over I2C, I2S polled),
/// recorded by the emulator to a wav file, and the recording's pitch
/// measured from its zero crossings.
pub fn boot_loader_tone(root: &Path) -> Result<(), String> {
    if !has_usb_model(root) {
        eprintln!("    tone: skipped (this emulator has no USB controller model)");
        return Ok(());
    }
    let loader = root.join("target/emu/rp-loader.bin");
    let wav = root.join("target/emulator/tone.wav");
    let _ = std::fs::remove_file(&wav);
    let wav_path = match emulator(root)? {
        Emulator::Wsl => to_wsl_path(&wav),
        Emulator::Native(_) => wav.display().to_string(),
    };
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
        s.write_all(&rp_loadproto::control_frame("tone 440 500"))
            .map_err(|e| e.to_string())?;
        let reply = usb_read_until(&mut s, "\n", 20)?;
        if !reply.contains("RPL OK tone 440 Hz 500 ms (24000 frames") {
            return Err(format!("unexpected reply to tone: {reply:?}"));
        }
        eprintln!("    loader: {}", reply.trim());
        s.write_all(&rp_loadproto::control_frame("tone 5 5"))
            .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL ERR usage", 10)?;
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
            timeout: Duration::from_secs(60),
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
    let (Some(start), Some(end)) = (left.iter().position(loud), left.iter().rposition(loud)) else {
        return Err("tone: the recording is silent".into());
    };
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
        let reply = usb_read_until(&mut s, "RPL ", 40)?;
        let reply = usb_read_until(&mut s, "\n", 10).map(|tail| reply + &tail)?;
        let line = reply
            .lines()
            .find(|l| l.starts_with("RPL "))
            .unwrap_or("")
            .to_string();
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
        // No CF card in this machine: the slot reads empty and the socket
        // is never touched.
        s.write_all(&rp_loadproto::control_frame("cf"))
            .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL ERR cf: no card (GPIO13 high)", 20)?;
        // SD DMA: reads on after the start-up self-test, writes only when
        // asked; the puts below then write by DMA and are checked in the
        // image afterwards.
        s.write_all(&rp_loadproto::control_frame("card dma"))
            .map_err(|e| e.to_string())?;
        usb_read_until(
            &mut s,
            "RPL OK dma reads on (self-test passed), writes off",
            20,
        )?;
        s.write_all(&rp_loadproto::control_frame("card dma writes"))
            .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "writes on", 20)?;
        s.write_all(&frame).map_err(|e| e.to_string())?;
        let reply = usb_read_until(&mut s, "RPL ", 40)?;
        let reply = usb_read_until(&mut s, "\n", 10).map(|tail| reply + &tail)?;
        let line = reply
            .lines()
            .find(|l| l.starts_with("RPL "))
            .unwrap_or("")
            .to_string();
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
                return Err(rp_hal_api::BlockError::Device);
            }
        }
        Ok(())
    }
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
        // Read-only by default: MODE SENSE says so and a write is refused.
        let r = bot.command(&[0x1A, 0, 0x3F, 0, 255, 0], 255, true, &[])?;
        if r.data.get(2).is_none_or(|b| b & 0x80 == 0) {
            return Err("drive: not reported write-protected at start".into());
        }
        let r = bot.command(&rw10(0x2A, 100, 1), 512, false, &[0; 512])?;
        if r.status != 1 || bot.sense()? != (0x07, 0x27, 0x00) {
            return Err("drive: a write was accepted while read-only".into());
        }
        // The PC asks for read-write over the serial port.
        serial
            .write_all(&rp_loadproto::control_frame("drive rw"))
            .map_err(|e| e.to_string())?;
        usb_read_until(&mut serial, "RPL OK drive read-write", 10)?;
        let r = bot.command(&[0; 6], 0, false, &[])?;
        if r.status != 1 || bot.sense()? != (0x06, 0x28, 0x00) {
            return Err("drive: no 'medium changed' after the mode change".into());
        }
        std::thread::sleep(Duration::from_millis(2500)); // the write grace
        // Replace a file through the drive (the PC's side).
        {
            let mut vol = rp_fat::Volume::mount(&mut bot, rp_fat::format::PARTITION_START)
                .map_err(|e| format!("drive: mount: {}", e.name()))?;
            let mut scratch = vec![0u8; 32 * 1024];
            vol.replace("startup.txt", &via_drive, &mut scratch)
                .map_err(|e| format!("drive: replace: {}", e.name()))?;
        }
        // The device refuses to write while the PC just wrote...
        serial.write_all(&put).map_err(|e| e.to_string())?;
        let r = usb_read_until(&mut serial, "RPL ", 20)?;
        let r = r + &usb_read_until(&mut serial, "\n", 5)?;
        if !r.contains("RPL ERR card busy") {
            return Err(format!("serial put during a drive write: {r:?}"));
        }
        // ... and accepts once the PC has been quiet for the grace period.
        std::thread::sleep(Duration::from_millis(2500));
        serial.write_all(&put).map_err(|e| e.to_string())?;
        let r = usb_read_until(&mut serial, "RPL OK", 30)?;
        let _ = r;
        // The drive reports the change once, then works.
        let r = bot.command(&[0; 6], 0, false, &[])?;
        if r.status != 1 || bot.sense()? != (0x06, 0x28, 0x00) {
            return Err("drive: no 'medium changed' after the device wrote".into());
        }
        if drive_file(&mut bot, "retropane.bin")? != via_serial {
            return Err("drive: doesn't show the file the device wrote".into());
        }
        if drive_file(&mut bot, "startup.txt")? != via_drive {
            return Err("drive: lost the file it wrote".into());
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
            timeout: Duration::from_secs(120),
        },
    )?;
    if card_file(&sd, "startup.txt")? != via_drive || card_file(&sd, "retropane.bin")? != via_serial
    {
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
            wsl_bash(&format!("{WSL_QEMU} -M x51v -device pxa27x-udc,help")).output()
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
/// perceptual 30% (9% duty: 19 of 216 counts) and `backlight 100` to full
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
        s.write_all(&rp_loadproto::control_frame("backlight 30"))
            .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL OK backlight 30% (duty 19/216)", 10)?;
        let mut mon = crate::screen::Monitor::connect(port)?;
        let dcr = mon.query("qom-get /machine/peripheral/pwm dcr0")?;
        let pcr = mon.query("qom-get /machine/peripheral/pwm pcr0")?;
        if dcr != "19" || pcr != "215" {
            return Err(format!(
                "backlight: PWM0 duty {dcr}, period {pcr}; wanted 19, 215"
            ));
        }
        s.write_all(&rp_loadproto::control_frame("backlight 100"))
            .map_err(|e| e.to_string())?;
        usb_read_until(&mut s, "RPL OK backlight 100% (duty 216/216)", 10)?;
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
            // 60% -> 0.36 of 216 counts = 77.
            if off != "0" || on != "77" {
                return Err(format!(
                    "power button: screen off/on gave DCR {off} then {on}; wanted 0, 77"
                ));
            }
            eprintln!("    power button OK: press turned the screen off, the next back on at 60%");
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
    let output = boot(
        root,
        &Boot {
            image: &image,
            marker: PROBE_MARKER,
            screenshot: Some(&shot),
            sd_image: Some(&sd),
            steps: &[],
            extra: &[],
            input: None,
            during: None,
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
    let ppm = std::fs::read(&shot).map_err(|e| format!("{}: {e}", shot.display()))?;
    crate::screen::check_text_screen(&ppm)?;
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
            timeout: Duration::from_secs(120),
        },
    )?;
    if !output.contains(BENCH_DONE) {
        return Err(format!("benchmarks didn't finish:\n{output}"));
    }
    eprintln!("    fault path OK: deliberate unmapped read reported as a data abort");
    if !output.contains("CHECK mem OK") {
        return Err(format!(
            "memory routines failed their on-target check:\n{output}"
        ));
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
    timeout: Duration,
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
        timeout,
    } = *cfg;
    let port = monitor_port();
    let monitor = format!("tcp:127.0.0.1:{port},server,nowait");
    let qmp = format!("tcp:127.0.0.1:{},server,nowait", port + 1);
    let emu = emulator(root)?;
    let wsl = matches!(emu, Emulator::Wsl);
    let mut cmd = match emu {
        Emulator::Native(exe) => {
            eprintln!("    emulator: {}", exe.display());
            let mut c = Command::new(exe);
            c.args(EMU_ARGS.split(' '))
                .args(["-monitor", &monitor, "-qmp", &qmp])
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
            wsl_bash(&format!(
                "exec timeout {} {WSL_QEMU} {EMU_ARGS} {} -monitor {monitor} -qmp {qmp} -kernel '{}'{sd}",
                timeout.as_secs() + 10,
                extra.join(" "),
                to_wsl_path(image)
            ))
        }
    };
    let mut child = cmd
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
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
        deadline: Instant::now() + timeout,
    };
    let stdin = child.stdin.take();
    let result = run_session(&mut serial, marker, port, wsl, screenshot, steps)
        .and_then(|()| send_input(&mut serial, stdin, input))
        .and_then(|()| during.map_or(Ok(()), |f| f()));
    let _ = child.kill();
    let _ = child.wait();
    if wsl {
        // Killing wsl.exe may not reach the Linux process at once; make sure
        // it's gone before the next test binds the same ports.
        let _ = wsl_bash("pkill -x qemu-system-arm; sleep 0.3; true").status();
    }
    match result {
        Ok(()) => Ok(serial.output),
        Err(e) => Err(format!("{e}. Serial output so far:\n{}", serial.output)),
    }
}

/// Sends `input`'s bytes to the guest's serial port and waits for its text.
fn send_input(
    serial: &mut Serial,
    stdin: Option<std::process::ChildStdin>,
    input: Option<(&[u8], &str)>,
) -> Result<(), String> {
    let Some((bytes, expect)) = input else {
        return Ok(());
    };
    let mut stdin = stdin.ok_or("serial input not connected")?;
    let from = serial.output.len();
    stdin
        .write_all(bytes)
        .and_then(|()| stdin.flush())
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
