//! `cargo xtask gate`: the gate's count (docs/ROADMAP.md, "Now: the
//! performance gate"; restructured by the owner's rulings, 2026-10-22:
//! docs/research/gate-classification.md). Reads the two checklists, checks
//! every line named on a page has exactly one row in its ledger (and no row
//! names a line the page doesn't have), and counts:
//!
//! - **architectural** lines (`arch N`: their answer changes a design), by
//!   the phase whose design needs them; a phase's gate lifts when its lines
//!   are ticked (done, or declined with the reason) and the ratchet runs.
//!   Phases [`NOW`] are counted: the owner lifted their gate (phase 4
//!   2026-10-24, phases 2 and 5 2026-10-29), so a line not ticked there
//!   now means one reopened since;
//! - **optimizations** (`opt`): the backlog, worked in payoff order from
//!   docs/research/backlog.md, which must hold every unticked one exactly
//!   once (and an architectural line's leftovers as `<line> (rest)`);
//! - `meta` lines (sweeps and reads that make other lines) and `= X`
//!   (the same line as X on the other page, counted there).
//!
//! `cargo xtask ci` runs it, so the count in a summary comes from a check,
//! not from memory.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// The checklists, with the short name the count and the backlog use.
pub const PAGES: [(&str, &str); 2] = [
    ("2700G", "docs/research/2700g-full-potential.md"),
    ("CPU", "docs/research/cpu-full-potential.md"),
];

/// The optimizations' backlog, in payoff order.
pub const BACKLOG: &str = "docs/research/backlog.md";

/// The phases the gate holds now (the owner's ruling B6, 2026-10-22: video
/// (7) and power (9) gate their own phases).
pub const NOW: [u8; 3] = [2, 4, 5];

/// Where a page's ledger starts.
const LEDGER: &str = "## The ledger (counted by `cargo xtask gate`)";

/// A line's status in a ledger.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    /// Used: ticked.
    Done,
    /// Declined with the reason (measured, unmeasurable, or not worth its
    /// device time): ticked; reopened if that changes.
    Declined,
    /// Some device evidence, more to do.
    Partial,
    /// Not worked yet.
    Open,
}

impl Status {
    /// Done or declined.
    #[must_use]
    pub fn ticked(self) -> bool {
        matches!(self, Self::Done | Self::Declined)
    }
}

/// What kind of line a row is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Its answer changes a design decision; the phase whose design needs it.
    Arch(u8),
    /// Faster, no design change: the backlog.
    Opt,
    /// A sweep or read that makes other lines.
    Meta,
    /// The same line as this one on the other page (counted there).
    Same(String),
}

/// One ledger row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// The line's name (`V1`, `P21`, `C6`, `T8`).
    pub line: String,
    /// Its kind.
    pub kind: Kind,
    /// Its status.
    pub status: Status,
}

/// The counts for a set of lines.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Count {
    /// Lines ticked (done or declined).
    pub ticked: usize,
    /// Lines partial.
    pub partial: usize,
    /// Lines open.
    pub open: usize,
}

impl Count {
    /// Lines in all.
    #[must_use]
    pub fn lines(&self) -> usize {
        self.ticked + self.partial + self.open
    }

    fn add(&mut self, s: Status) {
        match s {
            Status::Done | Status::Declined => self.ticked += 1,
            Status::Partial => self.partial += 1,
            Status::Open => self.open += 1,
        }
    }

    fn plus(&mut self, o: Count) {
        self.ticked += o.ticked;
        self.partial += o.partial;
        self.open += o.open;
    }
}

/// The line names a page's text (before its ledger) uses: bold `**V1**`,
/// `**P21**`, `**C1. Clocks**`: a letter of `VPCTF` and a number, bold.
fn named(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let b = text.as_bytes();
    let mut i = 0;
    while let Some(k) = text[i..].find("**") {
        let at = i + k + 2;
        i = at;
        let Some(&c) = b.get(at) else { break };
        if !b"VPCTF".contains(&c) {
            continue;
        }
        let digits = b[at + 1..]
            .iter()
            .take_while(|d| d.is_ascii_digit())
            .count();
        if digits == 0 {
            continue;
        }
        let end = at + 1 + digits;
        if matches!(b.get(end), Some(b'*' | b'.')) {
            out.push(text[at..end].to_string());
        }
    }
    out.sort();
    out.dedup();
    out
}

fn kind(line: &str, k: &str) -> Result<Kind, String> {
    if k == "opt" {
        return Ok(Kind::Opt);
    }
    if k == "meta" {
        return Ok(Kind::Meta);
    }
    if let Some(p) = k.strip_prefix("arch ")
        && let Ok(n) = p.parse::<u8>()
        && n <= 9
    {
        return Ok(Kind::Arch(n));
    }
    if let Some(x) = k.strip_prefix("= ")
        && !x.is_empty()
    {
        return Ok(Kind::Same(x.to_string()));
    }
    Err(format!(
        "line {line}: kind `{k}` (arch N, opt, meta, or = <line>)"
    ))
}

/// A page's ledger rows.
fn rows(ledger: &str) -> Result<Vec<Row>, String> {
    let mut out = Vec::new();
    for l in ledger.lines().filter(|l| l.starts_with("| ")) {
        let cells: Vec<&str> = l.split('|').map(str::trim).collect();
        // ["", line, kind, status, evidence, ""]
        let (Some(&line), Some(&k), Some(&status)) = (cells.get(1), cells.get(2), cells.get(3))
        else {
            continue;
        };
        if line == "Line" {
            continue;
        }
        let kind = kind(line, k)?;
        let status = match status {
            "done" => Status::Done,
            "declined" => Status::Declined,
            "partial" => Status::Partial,
            "open" => Status::Open,
            other => {
                return Err(format!(
                    "line {line}: status `{other}` (done, declined, partial or open)"
                ));
            }
        };
        if cells.get(4).is_none_or(|e| e.is_empty()) {
            return Err(format!("line {line}: no evidence or what's left"));
        }
        out.push(Row {
            line: line.to_string(),
            kind,
            status,
        });
    }
    Ok(out)
}

/// One page's rows, every named line exactly once, or what's wrong with
/// its ledger.
///
/// # Errors
///
/// A line named on the page with no row (or two), a row for a line the page
/// doesn't name, a bad kind or status, a row with no evidence, or no ledger.
pub fn page_rows(text: &str) -> Result<Vec<Row>, String> {
    let (body, ledger) = text
        .split_once(LEDGER)
        .ok_or("no ledger section on the page")?;
    let names = named(body);
    let rows = rows(ledger)?;
    let mut seen = BTreeSet::new();
    for r in &rows {
        if !seen.insert(r.line.as_str()) {
            return Err(format!("line {}: two ledger rows", r.line));
        }
        if !names.contains(&r.line) {
            return Err(format!("line {}: in the ledger, not on the page", r.line));
        }
    }
    for n in &names {
        if !seen.contains(n.as_str()) {
            return Err(format!("line {n}: on the page, no ledger row"));
        }
    }
    Ok(rows)
}

/// The backlog's line column: `2700G V3`, `CPU C6`, `CPU C3 (rest)`.
fn backlog_lines(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| l.starts_with("| "))
        .filter_map(|l| l.split('|').map(str::trim).nth(2).map(str::to_string))
        .filter(|c| PAGES.iter().any(|(p, _)| c.starts_with(&format!("{p} "))))
        .collect()
}

/// Checks the backlog against the pages' rows: every unticked `opt` line
/// once, nothing else but an architectural line's `(rest)`.
///
/// # Errors
///
/// A line missing, listed twice, or listed but not an unticked
/// optimization (nor an architectural line's rest).
pub fn check_backlog(pages: &[(&str, Vec<Row>)], backlog: &str) -> Result<usize, String> {
    let listed = backlog_lines(backlog);
    let mut seen = BTreeSet::new();
    for l in &listed {
        if !seen.insert(l.as_str()) {
            return Err(format!("backlog: {l} twice"));
        }
        let (name, rest) = match l.strip_suffix(" (rest)") {
            Some(n) => (n, true),
            None => (l.as_str(), false),
        };
        let (page, line) = name.split_once(' ').ok_or(format!("backlog: {l}"))?;
        let row = pages
            .iter()
            .find(|(p, _)| *p == page)
            .and_then(|(_, rows)| rows.iter().find(|r| r.line == line))
            .ok_or(format!("backlog: {l} is no ledger line"))?;
        let fits = if rest {
            matches!(row.kind, Kind::Arch(_))
        } else {
            row.kind == Kind::Opt && !row.status.ticked()
        };
        if !fits {
            return Err(format!(
                "backlog: {l} isn't an unticked optimization (or an architectural line's rest)"
            ));
        }
    }
    for (page, rows) in pages {
        for r in rows {
            let name = format!("{page} {}", r.line);
            if r.kind == Kind::Opt && !r.status.ticked() && !seen.contains(name.as_str()) {
                return Err(format!(
                    "backlog: {name} (an unticked optimization) isn't in it"
                ));
            }
        }
    }
    Ok(listed.len())
}

/// The architectural counts by phase, the backlog's and the others'.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Tally {
    /// Architectural lines by phase.
    pub arch: BTreeMap<u8, Count>,
    /// Optimizations.
    pub opt: Count,
    /// Sweeps and reads.
    pub meta: Count,
    /// Rows counted on the other page.
    pub same: usize,
}

impl Tally {
    fn add(&mut self, rows: &[Row]) {
        for r in rows {
            match r.kind {
                Kind::Arch(p) => self.arch.entry(p).or_default().add(r.status),
                Kind::Opt => self.opt.add(r.status),
                Kind::Meta => self.meta.add(r.status),
                Kind::Same(_) => self.same += 1,
            }
        }
    }

    /// The architectural lines of the phases the gate holds now.
    #[must_use]
    pub fn now(&self) -> Count {
        let mut c = Count::default();
        for p in NOW {
            if let Some(x) = self.arch.get(&p) {
                c.plus(*x);
            }
        }
        c
    }
}

/// `cargo xtask gate`.
///
/// # Errors
///
/// A page or the backlog unreadable or wrong (see [`page_rows`],
/// [`check_backlog`]).
pub fn run(root: &Path) -> Result<(), String> {
    let mut pages = Vec::new();
    let mut all = Tally::default();
    for (name, path) in PAGES {
        let text = std::fs::read_to_string(root.join(path)).map_err(|e| format!("{path}: {e}"))?;
        let rows = page_rows(&text).map_err(|e| format!("{path}: {e}"))?;
        all.add(&rows);
        pages.push((name, rows));
    }
    let backlog =
        std::fs::read_to_string(root.join(BACKLOG)).map_err(|e| format!("{BACKLOG}: {e}"))?;
    let listed = check_backlog(&pages, &backlog)?;
    for (p, c) in &all.arch {
        eprintln!(
            "    gate phase {p}: {} of {} architectural lines ticked ({} partial, {} open){}",
            c.ticked,
            c.lines(),
            c.partial,
            c.open,
            if NOW.contains(p) {
                ""
            } else {
                ": gates its own phase"
            }
        );
    }
    eprintln!(
        "    backlog: {} optimizations, {} ticked, {} partial, {} open ({listed} rows in {BACKLOG}); {} meta lines, {} counted on the other page",
        all.opt.lines(),
        all.opt.ticked,
        all.opt.partial,
        all.opt.open,
        all.meta.lines(),
        all.same
    );
    let now = all.now();
    eprintln!(
        "    gate: {} of {} architectural lines ticked for phases 2, 4, 5 ({} partial, {} open): {} (docs/ROADMAP.md)",
        now.ticked,
        now.lines(),
        now.partial,
        now.open,
        if now.partial + now.open == 0 {
            "lifted by the owner (phase 4 2026-10-24, phases 2 and 5 2026-10-29); the ratchet holds performance"
        } else {
            "LIFTED, BUT A LINE HAS REOPENED since: name it to the owner"
        }
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(rows: &str) -> String {
        format!(
            "| **V1** | a claim |\n| **C1. Clocks** | x |\nsee **P2** and **V1**; **Note** isn't a line\n{LEDGER}\n\n| Line | Kind | Status | Evidence |\n|---|---|---|---|\n{rows}"
        )
    }

    #[test]
    fn a_full_ledger_counts_by_kind_and_phase() {
        let rows = page_rows(&page(
            "| V1 | arch 5 | done | run X |\n| C1 | opt | partial | run Y |\n| P2 | arch 9 | declined | no reading |\n",
        ))
        .unwrap();
        let mut t = Tally::default();
        t.add(&rows);
        assert_eq!(
            t.now(),
            Count {
                ticked: 1,
                partial: 0,
                open: 0
            }
        );
        assert_eq!(
            t.arch[&9],
            Count {
                ticked: 1,
                partial: 0,
                open: 0
            }
        );
        assert_eq!(
            t.opt,
            Count {
                ticked: 0,
                partial: 1,
                open: 0
            }
        );
    }

    #[test]
    fn a_missing_extra_duplicate_or_bad_row_fails() {
        let missing = page_rows(&page("| V1 | opt | done | x |\n| C1 | opt | open | x |\n"));
        assert!(missing.unwrap_err().contains("P2"));
        let extra = page_rows(&page(
            "| V1 | opt | done | x |\n| C1 | opt | open | x |\n| P2 | opt | open | x |\n| T9 | opt | open | x |\n",
        ));
        assert!(extra.unwrap_err().contains("T9"));
        let twice = page_rows(&page(
            "| V1 | opt | done | x |\n| V1 | opt | open | x |\n| C1 | opt | open | x |\n| P2 | opt | open | x |\n",
        ));
        assert!(twice.unwrap_err().contains("two"));
        let bad = page_rows(&page("| V1 | opt | maybe | x |\n"));
        assert!(bad.unwrap_err().contains("maybe"));
        let bad_kind = page_rows(&page("| V1 | arch | done | x |\n"));
        assert!(bad_kind.unwrap_err().contains("kind"));
        let bare = page_rows(&page("| V1 | opt | done |  |\n"));
        assert!(bare.unwrap_err().contains("evidence"));
        assert!(page_rows("no ledger").is_err());
    }

    #[test]
    fn the_backlog_holds_every_unticked_optimization_once() {
        let rows = page_rows(&page(
            "| V1 | arch 5 | done | x |\n| C1 | opt | partial | x |\n| P2 | opt | declined | x |\n",
        ))
        .unwrap();
        let pages = [("CPU", rows)];
        let good = "| 1 | CPU C1 | big |\n| 2 | CPU V1 (rest) | small |\n";
        assert_eq!(check_backlog(&pages, good), Ok(2));
        let missing = check_backlog(&pages, "| 1 | CPU V1 (rest) | x |\n");
        assert!(missing.unwrap_err().contains("CPU C1"));
        let twice = check_backlog(&pages, "| 1 | CPU C1 | x |\n| 2 | CPU C1 | x |\n");
        assert!(twice.unwrap_err().contains("twice"));
        let ticked = check_backlog(&pages, "| 1 | CPU C1 | x |\n| 2 | CPU P2 | x |\n");
        assert!(ticked.unwrap_err().contains("CPU P2"));
        let arch = check_backlog(&pages, "| 1 | CPU C1 | x |\n| 2 | CPU V1 | x |\n");
        assert!(arch.unwrap_err().contains("CPU V1"));
    }

    #[test]
    fn the_real_pages_and_backlog_are_complete() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut pages = Vec::new();
        let mut lines = 0;
        for (name, path) in PAGES {
            let text = std::fs::read_to_string(root.join(path)).unwrap();
            let rows = page_rows(&text).unwrap();
            lines += rows.len();
            pages.push((name, rows));
        }
        assert!(lines >= 71, "{lines}");
        let backlog = std::fs::read_to_string(root.join(BACKLOG)).unwrap();
        check_backlog(&pages, &backlog).unwrap();
    }
}
