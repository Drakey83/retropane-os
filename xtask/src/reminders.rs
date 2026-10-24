//! The reminders list (`docs/reminders.md`): work parked until its moment,
//! printed by the session-start and pre-device-run hooks
//! (`tools/remind/remind.py`, `.claude/settings.json`). `cargo xtask
//! reminders`, a CI step, checks the table so a malformed row can't drop a
//! reminder out of the hooks silently, and prints how many are open.

use std::path::Path;

/// The list, from the workspace root.
const LIST: &str = "docs/reminders.md";

/// One row's status.
#[derive(Debug, PartialEq, Eq)]
enum Status {
    Open,
    Done,
    Dropped,
}

/// Checks one trigger's form.
fn trigger_ok(t: &str) -> bool {
    if t == "session" || t == "device-run" {
        return true;
    }
    if let Some(image) = t.strip_prefix("image:") {
        return image.starts_with("rp-")
            && image.len() > 3
            && image
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    }
    if let Some(what) = t.strip_prefix("after:") {
        return !what.trim().is_empty();
    }
    if let Some(date) = t.strip_prefix("date:") {
        let b = date.as_bytes();
        return b.len() == 10
            && b.iter().enumerate().all(|(i, c)| {
                if i == 4 || i == 7 {
                    *c == b'-'
                } else {
                    c.is_ascii_digit()
                }
            });
    }
    false
}

/// Parses the table: each `| R<n> |` row checked (five cells, numbered in
/// order from R1, a trigger of a known form, a known status with its
/// reason). Returns each row's status.
fn parse(text: &str) -> Result<Vec<Status>, String> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if !trimmed.starts_with("| R") {
            continue;
        }
        let cells: Vec<&str> = trimmed
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect();
        let at = |msg: &str| format!("{LIST}:{}: {msg}", n + 1);
        let Some(id) = cells.first().and_then(|c| c.strip_prefix('R')) else {
            continue;
        };
        if id.parse::<usize>().is_err() {
            continue; // prose that happens to start with "| R"
        }
        if cells.len() != 5 {
            return Err(at(&format!(
                "{} cells, want 5 (ID, What, Raised, Trigger, Status)",
                cells.len()
            )));
        }
        let want = out.len() + 1;
        if id != want.to_string() {
            return Err(at(&format!("R{id} where R{want} comes next")));
        }
        if cells[1].is_empty() || cells[2].is_empty() {
            return Err(at("What and Raised can't be empty"));
        }
        let triggers: Vec<&str> = cells[3]
            .split(';')
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .collect();
        if triggers.is_empty() {
            return Err(at("no trigger"));
        }
        if let Some(bad) = triggers.iter().find(|t| !trigger_ok(t)) {
            return Err(at(&format!(
                "trigger `{bad}`: session, device-run, image:rp-<name>, after:<what> or date:YYYY-MM-DD"
            )));
        }
        let status = match cells[4] {
            "open" => Status::Open,
            s if s.starts_with("done ") && s.len() > 5 => Status::Done,
            s if s.starts_with("dropped ") && s.len() > 8 => Status::Dropped,
            s => {
                return Err(at(&format!(
                    "status `{s}`: open, done <commit or run>, or dropped <the owner's reason>"
                )));
            }
        };
        out.push(status);
    }
    if out.is_empty() {
        return Err(format!("{LIST}: no reminder rows found"));
    }
    Ok(out)
}

/// The owner's standing rules (2026-10-24), each a bullet under
/// `## Standing rules` that must open with its bold title: the hooks print
/// them first, so a section lost or renamed fails CI.
const STANDING: [&str; 2] = [
    "**Iterate, don't imitate.**",
    "**Nothing is impossible or a limit without proof.**",
];

/// Checks the standing rules' section: present, first, and holding each of
/// [`STANDING`] as a bullet.
fn standing(text: &str) -> Result<(), String> {
    let mut section = text
        .split("\n## ")
        .skip(1)
        .map(|s| s.split_once('\n').unwrap_or((s, "")));
    let Some(("Standing rules", body)) = section.next() else {
        return Err(format!(
            "{LIST}: the first section must be `## Standing rules` (the owner's rules the hooks print first)"
        ));
    };
    for rule in STANDING {
        if !body.lines().any(|l| l.starts_with(&format!("- {rule}"))) {
            return Err(format!("{LIST}: standing rule `{rule}` missing"));
        }
    }
    Ok(())
}

/// CI's step: the standing rules and the table checked, the open count
/// printed.
pub fn run(root: &Path) -> Result<(), String> {
    let path = root.join(LIST);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    standing(&text)?;
    let rows = parse(&text)?;
    let open = rows.iter().filter(|s| **s == Status::Open).count();
    eprintln!(
        "    reminders: {} standing rules, {open} open of {} (docs/reminders.md; the hooks print them at session start and before device runs)",
        STANDING.len(),
        rows.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD: &str = "| ID | What | Raised | Trigger | Status |\n|---|---|---|---|---|\n";

    fn table(rows: &[&str]) -> String {
        format!("# Reminders\n\n{HEAD}{}\n", rows.join("\n"))
    }

    #[test]
    fn a_good_table_counts_open_done_and_dropped() {
        let t = table(&[
            "| R1 | a | owner, 2026-10-24 | session | open |",
            "| R2 | b | Claude | device-run; image:rp-gpubench | done abc1234 |",
            "| R3 | c | Claude | after:the divide-1 work; date:2026-11-01 | dropped the owner: not needed |",
        ]);
        assert_eq!(
            parse(&t).unwrap(),
            [Status::Open, Status::Done, Status::Dropped]
        );
    }

    #[test]
    fn the_real_list_parses() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let text = std::fs::read_to_string(root.join(LIST)).unwrap();
        standing(&text).unwrap();
        assert!(!parse(&text).unwrap().is_empty());
    }

    #[test]
    fn the_standing_rules_must_lead_and_be_whole() {
        let good = format!(
            "# R\n\n## Standing rules\n\n- {} a\n- {} b\n\n## The reminders\n",
            STANDING[0], STANDING[1]
        );
        assert!(standing(&good).is_ok());
        // Missing, not first, or a rule gone: refused.
        assert!(standing("# R\n\n## The reminders\n").is_err());
        let late = format!(
            "# R\n\n## The reminders\n\n## Standing rules\n\n- {} a\n- {} b\n",
            STANDING[0], STANDING[1]
        );
        assert!(standing(&late).is_err());
        let one = format!("# R\n\n## Standing rules\n\n- {} a\n", STANDING[0]);
        assert!(standing(&one).unwrap_err().contains("missing"));
    }

    #[test]
    fn malformed_rows_are_refused_with_their_line() {
        for (row, why) in [
            ("| R1 | a | b | session |", "cells"),
            ("| R2 | a | b | session | open |", "R2 where R1"),
            ("| R1 | | b | session | open |", "empty"),
            ("| R1 | a | b |  | open |", "no trigger"),
            ("| R1 | a | b | sometime | open |", "trigger `sometime`"),
            ("| R1 | a | b | image:gpubench | open |", "trigger"),
            ("| R1 | a | b | date:2026-1-1 | open |", "trigger"),
            ("| R1 | a | b | after: | open |", "trigger"),
            ("| R1 | a | b | session | done |", "status"),
            ("| R1 | a | b | session | maybe |", "status"),
        ] {
            let e = parse(&table(&[row])).unwrap_err();
            assert!(e.contains(why), "{row}: {e}");
            assert!(e.contains("docs/reminders.md:"), "{e}");
        }
        assert!(
            parse("no table here")
                .unwrap_err()
                .contains("no reminder rows")
        );
    }
}
