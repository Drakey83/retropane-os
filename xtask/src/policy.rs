//! Architecture policy checks (DESIGN.md §3.1), as pure functions so they can
//! be unit-tested without running cargo.

/// Architectural layer of a crate. A crate may depend only on crates in the
/// same or a lower layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Layer {
    /// `hal/*`: the only layer allowed to touch hardware.
    Hal = 0,
    /// `kernel/`, `drivers/*`, `boot/*`.
    Core = 1,
    /// `services/*`, `gwes/`.
    System = 2,
    /// `personality/*`: the CE compatibility layer.
    Personality = 3,
    /// `shell/`, `apps/*`.
    Apps = 4,
}

/// Where a crate lives in the architecture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// Part of the OS, in a layer.
    Os(Layer),
    /// Host-side programs (`xtask`, `tools/*`, `pc/*`): never a dependency
    /// of OS crates.
    Host,
}

/// Classifies a crate by its directory relative to the workspace root, using
/// `/` separators (e.g. `hal/api`, `drivers/touch`).
#[must_use]
pub fn placement(rel_dir: &str) -> Option<Placement> {
    let top = rel_dir.split('/').next()?;
    Some(match top {
        "hal" => Placement::Os(Layer::Hal),
        "kernel" | "drivers" | "boot" => Placement::Os(Layer::Core),
        "services" | "gwes" => Placement::Os(Layer::System),
        "personality" => Placement::Os(Layer::Personality),
        "shell" | "apps" => Placement::Os(Layer::Apps),
        // `pc`: the PC companions users run (LegacySync, ADR-0028).
        "xtask" | "tools" | "pc" => Placement::Host,
        _ => return None,
    })
}

/// Kind of dependency edge, as reported by `cargo metadata`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DepKind {
    /// `[dependencies]`.
    Normal,
    /// `[build-dependencies]`.
    Build,
    /// `[dev-dependencies]`: tests only, exempt from layering.
    Dev,
}

/// The crate that may only ever be a dev-dependency.
pub const MOCK_CRATE: &str = "rp-hal-mock";

/// Checks one dependency edge. Returns a human-readable violation, if any.
#[must_use]
pub fn check_edge(from: (&str, Placement), to: (&str, Placement), kind: DepKind) -> Option<String> {
    let (from_name, from_place) = from;
    let (to_name, to_place) = to;
    if kind == DepKind::Dev {
        return None;
    }
    match (from_place, to_place) {
        (Placement::Host, _) => None,
        (Placement::Os(_), Placement::Host) => {
            Some(format!("{from_name} depends on host-only crate {to_name}"))
        }
        (Placement::Os(_), Placement::Os(_)) if to_name == MOCK_CRATE => Some(format!(
            "{from_name} uses {MOCK_CRATE} outside [dev-dependencies]"
        )),
        (Placement::Os(a), Placement::Os(b)) if b > a => Some(format!(
            "{from_name} ({a:?}) depends upward on {to_name} ({b:?})"
        )),
        _ => None,
    }
}

/// Checks a crate root's `unsafe` policy: outside the HAL, crates must
/// `#![forbid(unsafe_code)]`.
#[must_use]
pub fn check_unsafe_policy(name: &str, place: Placement, lib_source: &str) -> Option<String> {
    match place {
        Placement::Os(Layer::Hal) | Placement::Host => None,
        Placement::Os(_) if lib_source.contains("#![forbid(unsafe_code)]") => None,
        Placement::Os(_) => Some(format!(
            "{name} is outside the HAL but lacks #![forbid(unsafe_code)]"
        )),
    }
}

/// Checks one tracked file for merge-conflict markers left in by a bad
/// resolution (2026-10-17: a doc was committed with them mid-merge). A
/// line starting with seven `<` or seven `>` and a space is a marker.
#[must_use]
pub fn check_conflict_markers(path: &str, text: &str) -> Option<String> {
    let line = text
        .lines()
        .position(|l| l.starts_with("<<<<<<< ") || l.starts_with(">>>>>>> "))?;
    Some(format!(
        "{path}:{}: a merge-conflict marker was committed",
        line + 1
    ))
}

/// Where a crash path may be written (quality audit C38): `rp-image`, which
/// every image's crashes go through; the loader, which receives them; and
/// the HAL, which defines the hooks (and user programs' own, `hal/user`).
pub const CRASH_PATH_HOMES: &[&str] = &["boot/image/", "boot/loader/", "hal/"];

/// What only those homes may write: a hand-rolled fault handler, panic
/// handler or fault-handler install, the crash flag's return, or the
/// runtime's bare entry (which starts an image without the crash path).
pub const CRASH_PATH_MARKS: &[&str] = &[
    "fn on_fault(",
    "#[panic_handler]",
    "set_fault_handler(",
    "crash_return(",
    "rp_hal_pxa27x::entry!(",
];

/// Checks one Rust source for a crash path of its own (quality audit C38:
/// 36 hand-rolled copies, 8 of which reported a crash as a hang or a clean
/// finish). Comments are skipped; xtask (which names the marks) and the
/// homes are allowed.
#[must_use]
pub fn check_crash_path(path: &str, text: &str) -> Vec<String> {
    if !path.ends_with(".rs")
        || path.starts_with("xtask/")
        || CRASH_PATH_HOMES.iter().any(|h| path.starts_with(h))
    {
        return Vec::new();
    }
    let mut found = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim_start().starts_with("//") {
            continue;
        }
        if let Some(m) = CRASH_PATH_MARKS.iter().find(|m| line.contains(*m)) {
            found.push(format!(
                "{path}:{}: `{m}`: images crash through rp-image (rp_image::entry!, crash, fail, on_crash), never a path of their own (quality audit C38)",
                i + 1
            ));
        } else if line.contains("take_mbx") && !line.contains("rp_image::take_mbx") {
            found.push(format!(
                "{path}:{}: `take_mbx`: images take the display through rp_image::take_mbx, which turns the crash box on (quality audit C38)",
                i + 1
            ));
        }
    }
    found
}

/// Where the 2D commands' unchecked builders may be called: the driver
/// itself and the image that measures the engine's own response to
/// rectangles the checked ones refuse (quality audit C47).
pub const UNCHECKED_2D_HOMES: &[&str] = &["drivers/blit2d/", "boot/mbx/"];

/// Calls of `blit_unchecked` / `copy_unchecked` outside
/// [`UNCHECKED_2D_HOMES`]: drawing goes through the checked commands.
#[must_use]
pub fn check_unchecked_2d(path: &str, text: &str) -> Vec<String> {
    if !path.ends_with(".rs")
        || path.starts_with("xtask/")
        || UNCHECKED_2D_HOMES.iter().any(|h| path.starts_with(h))
    {
        return Vec::new();
    }
    text.lines()
        .enumerate()
        .filter(|(_, l)| !l.trim_start().starts_with("//"))
        .filter(|(_, l)| l.contains("blit_unchecked") || l.contains("copy_unchecked"))
        .map(|(i, _)| {
            format!(
                "{path}:{}: an unchecked 2D command: draw through fill/blit/copy, which refuse rectangles the engine can't draw (quality audit C47); the unchecked ones are for rp-mbx's measurements",
                i + 1
            )
        })
        .collect()
}

/// The one file that writes `MBXCLK` (quality audit C43): the MBX clocks
/// go off only through `clock::stop`, whose guard holds the register bus
/// (one MBX register read with them off hung the X51v's bus, run FS).
pub const MBXCLK_HOME: &str = "drivers/mbx-display/src/clock.rs";

/// `MBXCLK` written outside [`MBXCLK_HOME`]: everything else goes through
/// `clock::stop` (off, guarded) or `clock::restore` (a kept value, never
/// off).
#[must_use]
pub fn check_mbxclk_writes(path: &str, text: &str) -> Vec<String> {
    if !path.ends_with(".rs") || path.starts_with("xtask/") || path == MBXCLK_HOME {
        return Vec::new();
    }
    text.lines()
        .enumerate()
        .filter(|(_, l)| !l.trim_start().starts_with("//"))
        .filter(|(_, l)| l.contains("write32(") && l.contains("MBXCLK"))
        .map(|(i, _)| {
            format!(
                "{path}:{}: MBXCLK written directly: use rp_drv_mbx_display::clock::stop (the clocks off, guarded) or clock::restore (a kept value) (quality audit C43: an MBX register touched with the clocks off hangs the bus)",
                i + 1
            )
        })
        .collect()
}

/// The marker of a value copied from Windows Mobile, Dell or another
/// original (docs/quality-audit.md D1): `// inherited: <audit item>,
/// <source>`.
pub const INHERITED: &str = "// inherited:";

/// One tagged copied value: its file, line and audit item.
#[derive(Debug, PartialEq, Eq)]
pub struct Inherited {
    /// The file.
    pub path: String,
    /// Its line, from 1.
    pub line: usize,
    /// The audit item it belongs to (before the comma).
    pub item: String,
}

/// The `inherited:` tags in `text`, and a violation for each malformed one
/// (an empty audit item or no source after the comma).
#[must_use]
pub fn inherited_tags(path: &str, text: &str) -> (Vec<Inherited>, Vec<String>) {
    let mut tags = Vec::new();
    let mut bad = Vec::new();
    for (k, l) in text.lines().enumerate() {
        let Some(at) = l.find(INHERITED) else {
            continue;
        };
        let rest = l[at + INHERITED.len()..].trim();
        match rest.split_once(',') {
            Some((item, source)) if !item.trim().is_empty() && !source.trim().is_empty() => {
                tags.push(Inherited {
                    path: path.into(),
                    line: k + 1,
                    item: item.trim().into(),
                });
            }
            _ => bad.push(format!(
                "{path}:{}: an `inherited:` tag needs `<audit item>, <source>`",
                k + 1
            )),
        }
    }
    (tags, bad)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unchecked_2d_commands_only_in_their_homes() {
        let bad = "let (w, n) = cmd::copy_unchecked(f, d, s, o, r, 0);";
        assert_eq!(
            check_unchecked_2d("drivers/mbx-render/src/lib.rs", bad).len(),
            1
        );
        assert_eq!(
            check_unchecked_2d("boot/gpubench/src/main.rs", "cmd::blit_unchecked(").len(),
            1
        );
        assert!(check_unchecked_2d("boot/mbx/src/main.rs", bad).is_empty());
        assert!(check_unchecked_2d("drivers/blit2d/src/lib.rs", bad).is_empty());
        assert!(
            check_unchecked_2d(
                "boot/gpubench/src/main.rs",
                "// copy_unchecked is for rp-mbx"
            )
            .is_empty()
        );
    }

    #[test]
    fn inherited_tags_are_counted_and_malformed_ones_refused() {
        let src = format!(
            "const A: u32 = 1;
{INHERITED} imitation-audit Audio, Dell wavedev.dll (1 ms)
const B: u32 = 2;
    {INHERITED} imitation-audit Clocks,
{INHERITED}  , no item
"
        );
        let (tags, bad) = inherited_tags("a.rs", &src);
        assert_eq!(
            tags,
            [Inherited {
                path: "a.rs".into(),
                line: 2,
                item: "imitation-audit Audio".into(),
            }]
        );
        assert_eq!(bad.len(), 2);
        assert!(bad[0].starts_with("a.rs:4:"));
        assert!(bad[1].starts_with("a.rs:5:"));
    }

    const HAL: Placement = Placement::Os(Layer::Hal);
    const CORE: Placement = Placement::Os(Layer::Core);
    const APPS: Placement = Placement::Os(Layer::Apps);

    #[test]
    fn conflict_markers_are_caught_and_lookalikes_are_not() {
        // Built at run time, so this file itself holds no markers.
        let (open, close) = ("<".repeat(7), ">".repeat(7));
        let bad = format!("a\n{open} HEAD\nours\n=======\ntheirs\n{close} 1234 (x)\n");
        assert_eq!(
            check_conflict_markers("docs/perf.md", &bad).as_deref(),
            Some("docs/perf.md:2: a merge-conflict marker was committed")
        );
        // Setext headings, arrows and quoted text are fine.
        let fine = format!("Title\n=======\n> quote\n>>> not a marker\n{open}no space\n");
        assert_eq!(check_conflict_markers("README.md", &fine), None);
    }

    #[test]
    fn a_crash_path_of_its_own_is_refused_outside_the_homes() {
        for m in CRASH_PATH_MARKS {
            let text = format!("mod image {{\n    {m}x)\n}}\n");
            let found = check_crash_path("boot/new/src/main.rs", &text);
            assert_eq!(found.len(), 1, "{m}");
            assert!(
                found[0].starts_with("boot/new/src/main.rs:2: "),
                "{}",
                found[0]
            );
            // The same line is fine in the homes, xtask and other files.
            for ok in [
                "boot/image/src/image.rs",
                "boot/loader/src/main.rs",
                "hal/pxa27x/src/rt.rs",
                "xtask/src/policy.rs",
                "docs/quality-audit.md",
            ] {
                assert!(check_crash_path(ok, &text).is_empty(), "{ok}: {m}");
            }
        }
        // Comments that name them are fine; the shared entry is too.
        let fine = "// no fn on_fault( here\n    /// #[panic_handler] lives in rp-image\nrp_image::entry!(main);\n";
        assert!(check_crash_path("services/x/src/main.rs", fine).is_empty());
        // The display only through rp-image (its crash box), in any spelling.
        for bad in [
            "let d = devices::take_mbx();",
            "let d = rp_hal_x51v::devices::take_mbx();",
            "use rp_hal_x51v::devices::{self, take_mbx};",
        ] {
            assert_eq!(
                check_crash_path("boot/new/src/main.rs", bad).len(),
                1,
                "{bad}"
            );
        }
        assert!(
            check_crash_path("boot/new/src/main.rs", "let d = rp_image::take_mbx();").is_empty()
        );
    }

    #[test]
    fn classifies_directories() {
        assert_eq!(placement("hal/api"), Some(HAL));
        assert_eq!(placement("drivers/touch"), Some(CORE));
        assert_eq!(
            placement("personality/ce/coredll"),
            Some(Placement::Os(Layer::Personality))
        );
        assert_eq!(placement("tools/romdump"), Some(Placement::Host));
        assert_eq!(placement("pc/legacysync"), Some(Placement::Host));
        assert_eq!(placement("random"), None);
    }

    #[test]
    fn downward_and_sideways_edges_are_fine() {
        assert_eq!(
            check_edge(("app", APPS), ("hal", HAL), DepKind::Normal),
            None
        );
        assert_eq!(
            check_edge(("drv", CORE), ("kern", CORE), DepKind::Normal),
            None
        );
    }

    #[test]
    fn upward_edges_are_rejected() {
        let v = check_edge(("drv", CORE), ("app", APPS), DepKind::Normal);
        assert!(v.unwrap().contains("upward"));
    }

    #[test]
    fn mock_only_as_dev_dependency() {
        assert!(check_edge(("drv", CORE), (MOCK_CRATE, HAL), DepKind::Normal).is_some());
        assert!(check_edge(("drv", CORE), (MOCK_CRATE, HAL), DepKind::Build).is_some());
        assert!(check_edge(("drv", CORE), (MOCK_CRATE, HAL), DepKind::Dev).is_none());
    }

    #[test]
    fn os_crates_never_depend_on_host_tools() {
        assert!(check_edge(("hal", HAL), ("xtask", Placement::Host), DepKind::Normal).is_some());
        assert!(check_edge(("xtask", Placement::Host), ("hal", HAL), DepKind::Normal).is_none());
    }

    #[test]
    fn unsafe_policy() {
        assert!(check_unsafe_policy("hal", HAL, "").is_none());
        assert!(check_unsafe_policy("drv", CORE, "#![no_std]").is_some());
        assert!(check_unsafe_policy("drv", CORE, "#![forbid(unsafe_code)]").is_none());
        assert!(check_unsafe_policy("xtask", Placement::Host, "").is_none());
    }

    #[test]
    fn mbxclk_is_written_only_by_the_clock_module() {
        let bad = "        regs.write32(MBXCLK, 0x4);";
        assert_eq!(
            check_mbxclk_writes("boot/gpubench/src/main.rs", bad).len(),
            1
        );
        assert_eq!(check_mbxclk_writes("hal/x51v/src/board.rs", bad).len(), 1);
        assert!(check_mbxclk_writes(MBXCLK_HOME, bad).is_empty());
        assert!(
            check_mbxclk_writes("boot/gpubench/src/main.rs", "// regs.write32(MBXCLK, 0)")
                .is_empty()
        );
        assert!(
            check_mbxclk_writes("boot/x/src/main.rs", "clock::restore(&mut regs, v);").is_empty()
        );
        assert!(
            check_mbxclk_writes("boot/x/src/main.rs", "let v = regs.read32(MBXCLK);").is_empty()
        );
    }
}
