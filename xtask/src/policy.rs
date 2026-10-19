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
}
