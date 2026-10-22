//! `cargo xtask gate`: the performance gate's count (docs/ROADMAP.md, "Now:
//! the performance gate"; the owner's votes, 2026-10-21). Reads the two
//! checklists, checks every line named on a page has exactly one row in its
//! ledger (and no row names a line the page doesn't have), and prints the
//! lines done, partial and open. `cargo xtask ci` runs it, so the count in a
//! summary comes from a check, not from memory.

use std::collections::BTreeMap;
use std::path::Path;

/// The checklists, with the short name the count uses.
pub const PAGES: [(&str, &str); 2] = [
    ("2700G", "docs/research/2700g-full-potential.md"),
    ("CPU", "docs/research/cpu-full-potential.md"),
];

/// Where a page's ledger starts.
const LEDGER: &str = "## The ledger (counted by `cargo xtask gate`)";

/// A line's status in a ledger.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    /// Used, or declined with the measured reason: ticked.
    Done,
    /// Some device evidence, more to do.
    Partial,
    /// Not worked yet.
    Open,
}

/// The counts for one page or all.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Count {
    /// Lines done.
    pub done: usize,
    /// Lines partial.
    pub partial: usize,
    /// Lines open.
    pub open: usize,
}

impl Count {
    /// Lines in all.
    #[must_use]
    pub fn lines(&self) -> usize {
        self.done + self.partial + self.open
    }

    fn add(&mut self, s: Status) {
        match s {
            Status::Done => self.done += 1,
            Status::Partial => self.partial += 1,
            Status::Open => self.open += 1,
        }
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

/// A page's ledger rows: line name to status.
fn rows(ledger: &str) -> Result<Vec<(String, Status)>, String> {
    let mut out = Vec::new();
    for l in ledger.lines().filter(|l| l.starts_with("| ")) {
        let cells: Vec<&str> = l.split('|').map(str::trim).collect();
        // ["", line, status, evidence, ""]
        let (Some(&line), Some(&status)) = (cells.get(1), cells.get(2)) else {
            continue;
        };
        if line == "Line" {
            continue;
        }
        let s = match status {
            "done" => Status::Done,
            "partial" => Status::Partial,
            "open" => Status::Open,
            other => {
                return Err(format!(
                    "line {line}: status `{other}` (done, partial or open)"
                ));
            }
        };
        if cells.get(3).is_none_or(|e| e.is_empty()) {
            return Err(format!("line {line}: no evidence or what's left"));
        }
        out.push((line.to_string(), s));
    }
    Ok(out)
}

/// One page's count, or what's wrong with its ledger.
///
/// # Errors
///
/// A line named on the page with no row (or two), a row for a line the page
/// doesn't name, a status other than done, partial or open, a row with no
/// evidence, or no ledger at all.
pub fn count_page(text: &str) -> Result<Count, String> {
    let (body, ledger) = text
        .split_once(LEDGER)
        .ok_or("no ledger section on the page")?;
    let names = named(body);
    let rows = rows(ledger)?;
    let mut seen: BTreeMap<&str, Status> = BTreeMap::new();
    for (line, s) in &rows {
        if seen.insert(line.as_str(), *s).is_some() {
            return Err(format!("line {line}: two ledger rows"));
        }
        if !names.contains(line) {
            return Err(format!("line {line}: in the ledger, not on the page"));
        }
    }
    let mut c = Count::default();
    for n in &names {
        let s = seen
            .get(n.as_str())
            .ok_or_else(|| format!("line {n}: on the page, no ledger row"))?;
        c.add(*s);
    }
    Ok(c)
}

/// `cargo xtask gate`.
///
/// # Errors
///
/// A page unreadable or its ledger wrong (see [`count_page`]).
pub fn run(root: &Path) -> Result<(), String> {
    let mut all = Count::default();
    for (name, path) in PAGES {
        let text = std::fs::read_to_string(root.join(path)).map_err(|e| format!("{path}: {e}"))?;
        let c = count_page(&text).map_err(|e| format!("{path}: {e}"))?;
        eprintln!(
            "    gate {name}: {} lines: {} done, {} partial, {} open",
            c.lines(),
            c.done,
            c.partial,
            c.open
        );
        all.done += c.done;
        all.partial += c.partial;
        all.open += c.open;
    }
    eprintln!(
        "    gate: {} of {} lines done ({} partial, {} open): the gate stays until every line is done and the ratchet runs (docs/ROADMAP.md)",
        all.done,
        all.lines(),
        all.partial,
        all.open
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(rows: &str) -> String {
        format!(
            "| **V1** | a claim |\n| **C1. Clocks** | x |\nsee **P2** and **V1**; **Note** isn't a line\n{LEDGER}\n\n| Line | Status | Evidence |\n|---|---|---|\n{rows}"
        )
    }

    #[test]
    fn a_full_ledger_counts() {
        let c = count_page(&page(
            "| V1 | done | run X |\n| C1 | partial | run Y |\n| P2 | open | not run |\n",
        ))
        .unwrap();
        assert_eq!((c.done, c.partial, c.open, c.lines()), (1, 1, 1, 3));
    }

    #[test]
    fn a_missing_extra_duplicate_or_bad_row_fails() {
        let missing = count_page(&page("| V1 | done | x |\n| C1 | open | x |\n"));
        assert!(missing.unwrap_err().contains("P2"));
        let extra = count_page(&page(
            "| V1 | done | x |\n| C1 | open | x |\n| P2 | open | x |\n| T9 | open | x |\n",
        ));
        assert!(extra.unwrap_err().contains("T9"));
        let twice = count_page(&page(
            "| V1 | done | x |\n| V1 | open | x |\n| C1 | open | x |\n| P2 | open | x |\n",
        ));
        assert!(twice.unwrap_err().contains("two"));
        let bad = count_page(&page("| V1 | maybe | x |\n"));
        assert!(bad.unwrap_err().contains("maybe"));
        let bare = count_page(&page("| V1 | done |  |\n"));
        assert!(bare.unwrap_err().contains("evidence"));
        assert!(count_page("no ledger").is_err());
    }

    #[test]
    fn the_real_pages_are_complete() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut lines = 0;
        for (_, path) in PAGES {
            let text = std::fs::read_to_string(root.join(path)).unwrap();
            lines += count_page(&text).unwrap().lines();
        }
        assert!(lines >= 64, "{lines}");
    }
}
