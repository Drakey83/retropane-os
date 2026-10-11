//! `cargo xtask <command>`: one entry point for every build, check and test,
//! used identically by developers and CI.

#![forbid(unsafe_code)]

mod emu;
mod policy;
mod screen;
mod sdkit;
mod serve;
mod size;
mod usb;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use policy::{DepKind, Placement};
use serde_json::Value;

const TARGET_SPEC: &str = "targets/armv5te-x51v-none-eabi.json";

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
  bench     deterministic emulator benchmarks vs bench/baseline-emulator.json
            (--update records a new baseline)";

fn main() -> ExitCode {
    let cmd = std::env::args().nth(1).unwrap_or_default();
    let flag = std::env::args().nth(2).unwrap_or_default();
    let result = match cmd.as_str() {
        "ci" => ["fmt", "clippy", "lint", "test", "cross"]
            .iter()
            .try_for_each(|c| run_step(c)),
        "fmt" | "clippy" | "lint" | "test" | "cross" | "coverage" | "emu" | "qemu" => {
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
        "emu" => cross()
            .and_then(|()| emu::boot_hello(&workspace_root()))
            .and_then(|()| emu::boot_probe(&workspace_root()))
            .and_then(|()| emu::boot_kernel(&workspace_root()))
            .and_then(|()| emu::boot_threads(&workspace_root()))
            .and_then(|()| emu::boot_light(&workspace_root()))
            .and_then(|()| emu::boot_usb(&workspace_root())),
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
    {
        args.push("-p".into());
        args.push(c.name.clone());
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    cargo(&refs)?;
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
