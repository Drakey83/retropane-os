//! `cargo xtask <command>`: one entry point for every build, check and test,
//! used identically by developers and CI.

#![forbid(unsafe_code)]

mod emu;
mod mutants;
mod policy;
mod screen;
mod sdkit;
mod serve;
mod size;
mod usb;
mod userimg;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use policy::{DepKind, Placement};
use serde_json::Value;

const TARGET_SPEC: &str = "targets/armv5te-x51v-none-eabi.json";
/// The Thumb variant of the target (ADR-0015: cold code as Thumb).
const THUMB_TARGET_SPEC: &str = "targets/thumbv5te-x51v-none-eabi.json";
/// Images built as Thumb (ADR-0015); empty it to go back to all-ARM.
const THUMB_IMAGES: &[&str] = &["rp-loader"];

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  ci        everything CI runs: fmt, clippy, lint, test, cross
  fmt       check formatting
  clippy    lint all crates (warnings are errors)
  lint      architecture policy: layering and unsafe rules (DESIGN.md §3.1)
  test      host unit tests
  cross     build every OS crate for the X51v (ARMv5TE)
  coverage  host test coverage (needs cargo-llvm-cov), 80% line floor
  emu       boot the bring-up, probe, kernel, threads, first-light and USB images
            in the emulator
            (downloads it if needed)
  qemu      build the emulator locally in WSL from ../retropane-qemu (fast loop)
  sdkit     SD card kit for the real X51v: HaRET + raw image (target/sdkit)
            (--serve: also serve it over Wi-Fi for the device's browser)
  usb       talk to an X51v running the USB link test (target/sdkit/usb) over
            its sync cable: find the COM port, check the banner and echo
  load      send an image (name, e.g. rp-probe, or a .bin) to an X51v running
            the resident loader (target/sdkit/loader) over USB
            (--setup: create the admin task that lets it re-find the Axim
            after each run without a cable replug; one UAC prompt, once)
  put       write a file to the X51v's SD card over USB through the resident
            loader (ADR-0013): cargo xtask put <file> [name on the card]
  install   put the kit's loader, HaRET and scripts on the card the same way
            (replaces the files already there; run cargo xtask sdkit first)
  drive     the SD card as a drive on the PC (ADR-0014): `drive rw` lets the
            PC write it, `drive ro` (the default at start) makes it read-only,
            `drive off`/`drive on` withdraws/offers the card (an empty reader
            to the PC), `drive status` reports counters and the last card error
  backlight the X51v's backlight through the loader: `backlight` reports
            it, `backlight <0-100>` fades to that level (perceptual percent)
  card      card diagnostics through the loader: `card scan <lba> <count>`
            reads blocks one by one, reports failures and the slowest read;
            `card scrub <lba>` rewrites a weak first-FAT sector in place,
            verified against its mirror in the second FAT; `card repair
            <lba>` restores a damaged one from the mirror (only when the
            first copy's differing entries are invalid); `card check`
            checks the file system read-only (FAT32 or exFAT); `card dma` reports
            the SD DMA self-test, `card dma writes` lets writes use DMA
  ls        list a folder on the card through the loader (read-only)
  get       `get <file> [local name]`: copy a file from the card, CRC
            checked end to end; never overwrites a local file
  rm        delete a file or empty folder on the card through the loader
            (crash-safe; the files the device boots from are refused)
  cf        identify the CompactFlash card through the loader (read-only;
            then it is the drive's second unit); `cf log`: READY changes,
            attach and drop times; `cf soft`: no hardware reset (diagnostics)
  cpld      the CPLD's registers as the loader found them at start-up
            (read-only diagnostics)
  reboot-wm reset the X51v back into Windows Mobile the way its own
            reboot does (GPIO99; item 64, not yet tried on the device)
  tone     `tone <hz> <ms>`: a sine through the codec (headphones; the
            reply names the output and the CPU's busy share)
  device-check  the read-only checks waiting for the device, in one run
            (drive, SD DMA, backlight, the unscanned card blocks), report
            in target/device-check.txt; --with-owner adds CF and a tone
  emu-picture  only the rp-picture check (a UI concept on screen, then
            back to the loader): run it before `load rp-picture`
  emu-blend only the rp-blend check (plane blending, three stages
            pixel-exact): run it before `load rp-blend`
  emu-crash only the crash path check (an image's crash box on the plane
            on screen, then the loader's crash screen, log and card report)
  emu-kernel only the kernel image's check (VM, IPC, the driver host)
  emu-cf    only the CompactFlash check (slot power through the CPLD,
            the card as the PC's second drive)
  mutants   the deliberate bugs each test suite must catch (docs/testing.md);
            --emu adds the slow emulator ones
  bench     deterministic emulator benchmarks vs bench/baseline-emulator.json
            (--update records a new baseline)";

fn main() -> ExitCode {
    let cmd = std::env::args().nth(1).unwrap_or_default();
    let flag = std::env::args().nth(2).unwrap_or_default();
    let result = match cmd.as_str() {
        "ci" => ["fmt", "clippy", "lint", "test", "cross"]
            .iter()
            .try_for_each(|c| run_step(c)),
        "fmt" | "clippy" | "lint" | "test" | "cross" | "coverage" | "emu" | "emu-picture"
        | "emu-blend" | "emu-crash" | "emu-cf" | "emu-kernel" | "mutants" | "qemu" => {
            run_step(&cmd)
        }
        "sdkit" => {
            eprintln!("==> sdkit");
            cross()
                .and_then(|()| sdkit::build(&workspace_root()))
                .and_then(|()| {
                    if flag == "--serve" {
                        serve::serve(&workspace_root().join("target").join("sdkit"))
                    } else {
                        Ok(())
                    }
                })
        }
        "usb" => {
            eprintln!("==> usb");
            usb::run()
        }
        "load" => {
            eprintln!("==> load {flag}");
            if flag == "--setup" {
                usb::setup_restart_task(&workspace_root())
            } else if flag.is_empty() {
                Err("usage: cargo xtask load <image name | file.bin> | --setup".into())
            } else {
                let force = std::env::args().skip(3).any(|a| a == "--force");
                usb::load(&workspace_root(), &flag, force)
            }
        }
        "put" => {
            eprintln!("==> put {flag}");
            if flag.is_empty() {
                Err("usage: cargo xtask put <file> [name on the card]".into())
            } else {
                usb::put(&workspace_root(), &flag, std::env::args().nth(3).as_deref())
            }
        }
        "drive" => {
            eprintln!("==> drive {flag}");
            match flag.as_str() {
                "rw" | "ro" | "on" | "off" | "status" => {
                    usb::command(&workspace_root(), &format!("drive {flag}"))
                }
                _ => Err("usage: cargo xtask drive rw|ro|on|off|status".into()),
            }
        }
        "card" => {
            // Diagnostics: `cargo xtask card scan <lba> <count>`.
            let rest: Vec<String> = std::env::args().skip(2).collect();
            eprintln!("==> card {}", rest.join(" "));
            usb::command(&workspace_root(), &format!("card {}", rest.join(" ")))
        }
        "backlight" => {
            eprintln!("==> backlight {flag}");
            usb::command(&workspace_root(), &format!("backlight {flag}"))
        }
        "ls" => {
            eprintln!("==> ls {flag}");
            usb::ls(&workspace_root(), &flag)
        }
        "get" => {
            eprintln!("==> get {flag}");
            if flag.is_empty() {
                Err("usage: cargo xtask get <file on the card> [local name]".into())
            } else {
                usb::get(&workspace_root(), &flag, std::env::args().nth(3).as_deref())
            }
        }
        "rm" => {
            let rest: Vec<String> = std::env::args().skip(2).collect();
            eprintln!("==> rm {}", rest.join(" "));
            if rest.is_empty() {
                Err("usage: cargo xtask rm <file or empty folder on the card>".into())
            } else {
                usb::command(&workspace_root(), &format!("rm {}", rest.join(" ")))
            }
        }
        "cf" => {
            // `cf soft`: without the hardware reset (diagnostics only).
            // `cf soft`: without the hardware reset; `cf log`: the slot's
            // timestamped events (diagnostics only).
            let cmd = match std::env::args().nth(2).as_deref() {
                Some("soft") => "cf soft",
                Some("log") => "cf log",
                _ => "cf",
            };
            eprintln!("==> {cmd}");
            usb::command(&workspace_root(), cmd)
        }
        "cpld" => {
            eprintln!("==> cpld");
            usb::command(&workspace_root(), "cpld")
        }
        "reboot-wm" => {
            eprintln!("==> reboot wm");
            usb::command(&workspace_root(), "reboot wm")
        }
        "tone" => {
            let rest: Vec<String> = std::env::args().skip(2).collect();
            eprintln!("==> tone {}", rest.join(" "));
            usb::command(&workspace_root(), &format!("tone {}", rest.join(" ")))
        }
        "device-check" => {
            eprintln!("==> device-check {flag}");
            usb::device_check(&workspace_root(), flag == "--with-owner")
        }
        "install" => {
            eprintln!("==> install");
            // Always from a fresh kit: an old target/sdkit once put a stale
            // loader on the card while reporting "up to date".
            cross()
                .and_then(|()| sdkit::build(&workspace_root()))
                .and_then(|()| usb::install(&workspace_root()))
        }
        "bench" => {
            eprintln!("==> bench");
            cross().and_then(|()| emu::bench(&workspace_root(), flag == "--update"))
        }
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xtask: {e}");
            ExitCode::FAILURE
        }
    }
}

type Result<T = ()> = std::result::Result<T, String>;

fn run_step(step: &str) -> Result {
    eprintln!("==> {step}");
    match step {
        "fmt" => cargo(&["fmt", "--all", "--check"]),
        "clippy" => cargo(&[
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ]),
        "lint" => lint(),
        "test" => cargo(&["test", "--workspace"]),
        "cross" => cross(),
        "emu-picture" => cross().and_then(|()| emu::boot_loader_picture(&workspace_root())),
        "emu-blend" => cross().and_then(|()| emu::boot_loader_blend(&workspace_root())),
        "emu-crash" => cross().and_then(|()| emu::boot_loader_crash(&workspace_root())),
        "emu-cf" => cross().and_then(|()| emu::boot_loader_cf_alone(&workspace_root())),
        "emu-kernel" => cross().and_then(|()| emu::boot_kernel(&workspace_root())),
        "mutants" => mutants::run(&workspace_root(), std::env::args().any(|a| a == "--emu")),
        "emu" => cross()
            .and_then(|()| emu::boot_hello(&workspace_root()))
            .and_then(|()| emu::boot_probe(&workspace_root()))
            .and_then(|()| emu::boot_kernel(&workspace_root()))
            .and_then(|()| emu::boot_threads(&workspace_root()))
            .and_then(|()| emu::boot_light(&workspace_root()))
            .and_then(|()| emu::boot_usb(&workspace_root()))
            .and_then(|()| emu::boot_loader(&workspace_root()))
            .and_then(|()| emu::boot_loader_hang(&workspace_root()))
            .and_then(|()| emu::boot_loader_run_limit(&workspace_root()))
            .and_then(|()| emu::boot_loader_picture(&workspace_root()))
            .and_then(|()| emu::boot_loader_blend(&workspace_root()))
            .and_then(|()| emu::boot_loader_crash(&workspace_root()))
            .and_then(|()| emu::boot_usb_link(&workspace_root()))
            .and_then(|()| emu::boot_loader_usb(&workspace_root()))
            .and_then(|()| emu::boot_loader_put(&workspace_root()))
            .and_then(|()| emu::boot_loader_put_exfat(&workspace_root()))
            .and_then(|()| emu::boot_loader_cf(&workspace_root()))
            .and_then(|()| emu::boot_loader_tone(&workspace_root()))
            .and_then(|()| emu::boot_loader_drive(&workspace_root()))
            .and_then(|()| emu::boot_loader_fallback(&workspace_root()))
            .and_then(|()| emu::boot_loader_locked(&workspace_root()))
            .and_then(|()| emu::boot_loader_backlight(&workspace_root()))
            // The benchmark image's on-target checks (pixel paths, memory,
            // crypto, SD) and no regression past the baseline.
            .and_then(|()| emu::bench(&workspace_root(), false)),
        "qemu" => emu::build_wsl(&workspace_root()),
        "coverage" => cargo(&[
            "llvm-cov",
            "--workspace",
            "--exclude",
            "xtask",
            "--exclude",
            "rp-hello",
            "--exclude",
            "rp-bench",
            "--exclude",
            "rp-bench-suite",
            "--exclude",
            "rp-kernel",
            "--exclude",
            "rp-threads",
            // CLI glue; its logic lives in the tested romdump library.
            "--ignore-filename-regex",
            r"romdump.src.main\.rs",
            "--fail-under-lines",
            "80",
        ]),
        _ => unreachable!(),
    }
}

fn cargo(args: &[&str]) -> Result {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let status = Command::new(cargo)
        .args(args)
        .env("RP_BUILD_ID", build_id())
        .current_dir(workspace_root())
        .status()
        .map_err(|e| format!("failed to run cargo: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`cargo {}` failed", args.join(" ")))
    }
}

/// The commit being built (short hash, `+` if there are uncommitted
/// changes), so a photo of the probe report names its exact build.
fn build_id() -> String {
    let git = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(workspace_root())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    let hash = git(&["rev-parse", "--short=8", "HEAD"]).unwrap_or_else(|| "unknown".into());
    let dirty = git(&["status", "--porcelain"]).is_some_and(|s| !s.is_empty());
    if dirty { format!("{hash}+") } else { hash }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives one level below the workspace root")
        .to_path_buf()
}

/// A workspace crate, as far as the policy checks care.
struct Crate {
    name: String,
    place: Placement,
    lib_root: Option<PathBuf>,
    deps: Vec<(String, DepKind)>,
}

fn workspace_crates() -> Result<Vec<Crate>> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let out = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(workspace_root())
        .output()
        .map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err("cargo metadata failed".into());
    }
    let meta: Value = serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
    let root = workspace_root();
    let mut crates = Vec::new();
    for pkg in meta["packages"].as_array().into_iter().flatten() {
        let name = pkg["name"].as_str().unwrap_or_default().to_string();
        let manifest = PathBuf::from(pkg["manifest_path"].as_str().unwrap_or_default());
        let dir = manifest.parent().unwrap_or(&root);
        let rel = dir
            .strip_prefix(&root)
            .map_err(|_| format!("{name} is outside the workspace"))?
            .to_string_lossy()
            .replace('\\', "/");
        let place = policy::placement(&rel)
            .ok_or_else(|| format!("{name} ({rel}) is not in a known layer directory"))?;
        let lib_root = pkg["targets"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|t| {
                t["kind"]
                    .as_array()
                    .is_some_and(|k| k.iter().any(|k| k == "lib"))
            })
            .and_then(|t| t["src_path"].as_str())
            .map(PathBuf::from);
        let deps = pkg["dependencies"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|d| {
                let kind = match d["kind"].as_str() {
                    Some("dev") => DepKind::Dev,
                    Some("build") => DepKind::Build,
                    _ => DepKind::Normal,
                };
                (d["name"].as_str().unwrap_or_default().to_string(), kind)
            })
            .collect();
        crates.push(Crate {
            name,
            place,
            lib_root,
            deps,
        });
    }
    Ok(crates)
}

fn lint() -> Result {
    let crates = workspace_crates()?;
    let mut violations = Vec::new();
    for c in &crates {
        for (dep, kind) in &c.deps {
            // Only workspace-internal edges are layered.
            if let Some(target) = crates.iter().find(|t| &t.name == dep) {
                violations.extend(policy::check_edge(
                    (&c.name, c.place),
                    (&target.name, target.place),
                    *kind,
                ));
            }
        }
        if let Some(lib) = &c.lib_root {
            let src =
                std::fs::read_to_string(lib).map_err(|e| format!("{}: {e}", lib.display()))?;
            violations.extend(policy::check_unsafe_policy(&c.name, c.place, &src));
        }
    }
    if violations.is_empty() {
        eprintln!("    {} crates, no policy violations", crates.len());
        Ok(())
    } else {
        for v in &violations {
            eprintln!("    violation: {v}");
        }
        Err(format!("{} policy violation(s)", violations.len()))
    }
}

fn cross() -> Result {
    let crates = workspace_crates()?;
    // User process images first: the kernel image embeds them (its build
    // script reads `<name>.pe` next to them).
    let mut user = vec![
        "build",
        "--release",
        "--target",
        TARGET_SPEC,
        "-Zbuild-std=core,compiler_builtins",
        "-Zjson-target-spec",
    ];
    for name in userimg::IMAGES {
        user.extend(["-p", name]);
    }
    cargo(&user)?;
    userimg::package_all(&workspace_root().join("target/armv5te-x51v-none-eabi/release"))?;
    let mut args = vec![
        "build".to_string(),
        "--release".into(),
        "--target".into(),
        TARGET_SPEC.into(),
        "-Zbuild-std=core,compiler_builtins".into(),
        "-Zjson-target-spec".into(),
    ];
    for c in crates
        .iter()
        .filter(|c| matches!(c.place, Placement::Os(_)) && c.name != policy::MOCK_CRATE)
        // Built on their own above: a build's features are unified, and a
        // program's runtime (its panic handler) mustn't reach the kernel.
        .filter(|c| !userimg::IMAGES.contains(&c.name.as_str()))
    {
        args.push("-p".into());
        args.push(c.name.clone());
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    cargo(&refs)?;
    // ADR-0015: these images are built as Thumb code (the HAL's assembly
    // stays ARM, marked per function), then put where the ARM build put
    // them, so everything downstream (sizes, the emulator, the SD kit) uses
    // them unchanged. Reverting to all-ARM is emptying this list.
    for image in THUMB_IMAGES {
        cargo(&[
            "build",
            "--release",
            "--target",
            THUMB_TARGET_SPEC,
            "-Zbuild-std=core,compiler_builtins",
            "-Zjson-target-spec",
            "-p",
            image,
        ])?;
        let from = workspace_root()
            .join("target/thumbv5te-x51v-none-eabi/release")
            .join(image);
        let to = workspace_root()
            .join("target/armv5te-x51v-none-eabi/release")
            .join(image);
        std::fs::copy(&from, &to).map_err(|e| format!("{}: {e}", from.display()))?;
    }
    // Bare-metal images: OS crates that are binaries only.
    let images: Vec<&str> = crates
        .iter()
        .filter(|c| matches!(c.place, Placement::Os(_)) && c.lib_root.is_none())
        .map(|c| c.name.as_str())
        .collect();
    size::check(
        &workspace_root().join("target/armv5te-x51v-none-eabi/release"),
        &images,
    )
}
