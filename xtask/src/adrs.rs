//! The ADR index (`docs/adr/README.md`): one row per decision record, its
//! number, title, status, date and amendments, generated from the records
//! themselves so it can't drift (`cargo xtask adr-index` writes it; `cargo
//! xtask adrs`, a CI step, fails when it is stale).

use std::path::Path;

/// Where the records live.
const DIR: &str = "docs/adr";
/// The index, inside [`DIR`].
const INDEX: &str = "README.md";

/// One record's index row.
#[derive(Debug, PartialEq)]
struct Adr {
    file: String,
    number: String,
    title: String,
    status: &'static str,
    date: String,
    amendments: usize,
}

/// The status words a record's `- **Status:**` line may open with.
const STATUSES: [&str; 4] = ["Accepted", "Proposed", "Superseded", "Rejected"];

/// Reads one record's text. Errors name what's missing.
fn parse(file: &str, text: &str) -> Result<Adr, String> {
    let number = file
        .get(..4)
        .filter(|n| n.bytes().all(|b| b.is_ascii_digit()))
        .ok_or_else(|| format!("{file}: the name doesn't start with a four-digit number"))?;
    let heading = text
        .lines()
        .next()
        .and_then(|l| l.strip_prefix(&format!("# ADR-{number}: ")))
        .ok_or_else(|| format!("{file}: the first line isn't `# ADR-{number}: <title>`"))?;
    let field = |name: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(&format!("- **{name}:** ")))
            .ok_or_else(|| format!("{file}: no `- **{name}:**` line"))
    };
    let status_line = field("Status")?.trim_start_matches('*');
    let status = STATUSES
        .into_iter()
        .find(|s| status_line.starts_with(s))
        .ok_or_else(|| {
            format!(
                "{file}: the status line opens with none of {}",
                STATUSES.join(", ")
            )
        })?;
    let date = field("Date")?.trim().to_string();
    let amendments = text
        .lines()
        .filter(|l| l.starts_with("## Amendment"))
        .count();
    Ok(Adr {
        file: file.to_string(),
        number: number.to_string(),
        title: heading.trim().to_string(),
        status,
        date,
        amendments,
    })
}

/// The index's text for `adrs` (in number order).
fn render(adrs: &[Adr]) -> String {
    let mut out = String::from(
        "# Architecture decision records\n\n\
         Every decision record, generated from the records themselves by \
         `cargo xtask adr-index` (CI's `adrs` step fails when this page is \
         stale: edit the records, then regenerate). The status is the first \
         word of each record's status line; the record has the details (who \
         decided, when, which options). New records start from \
         [0000-template.md](0000-template.md).\n\n\
         | ADR | Title | Status | Date | Amendments |\n\
         |---|---|---|---|---|\n",
    );
    for a in adrs {
        out.push_str(&format!(
            "| [{}]({}) | {} | {} | {} | {} |\n",
            a.number,
            a.file,
            a.title.replace('|', "\\|"),
            a.status,
            a.date,
            if a.amendments == 0 {
                String::new()
            } else {
                a.amendments.to_string()
            }
        ));
    }
    let count = |s: &str| adrs.iter().filter(|a| a.status == s).count();
    out.push_str(&format!(
        "\n{} records: {} accepted, {} proposed, {} superseded, {} rejected.\n",
        adrs.len(),
        count("Accepted"),
        count("Proposed"),
        count("Superseded"),
        count("Rejected")
    ));
    out
}

/// Every record in [`DIR`] (not the template, not the index), parsed.
fn collect(root: &Path) -> Result<Vec<Adr>, String> {
    let dir = root.join(DIR);
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.ends_with(".md") && n != INDEX && !n.starts_with("0000-"))
        .collect();
    names.sort();
    let adrs = names
        .iter()
        .map(|n| {
            let text = std::fs::read_to_string(dir.join(n)).map_err(|e| format!("{n}: {e}"))?;
            parse(n, &text)
        })
        .collect::<Result<Vec<_>, _>>()?;
    for pair in adrs.windows(2) {
        if pair[0].number == pair[1].number {
            return Err(format!(
                "two records numbered {}: {} and {}",
                pair[0].number, pair[0].file, pair[1].file
            ));
        }
    }
    Ok(adrs)
}

/// `cargo xtask adr-index`: writes the index.
pub fn write(root: &Path) -> Result<(), String> {
    let adrs = collect(root)?;
    let path = root.join(DIR).join(INDEX);
    std::fs::write(&path, render(&adrs)).map_err(|e| format!("{}: {e}", path.display()))?;
    eprintln!("    {} records indexed in {DIR}/{INDEX}", adrs.len());
    Ok(())
}

/// `cargo xtask adrs` (a CI step): the index must be what the records say.
pub fn check(root: &Path) -> Result<(), String> {
    let adrs = collect(root)?;
    let path = root.join(DIR).join(INDEX);
    let have = std::fs::read_to_string(&path).unwrap_or_default();
    if have.replace("\r\n", "\n") != render(&adrs) {
        return Err(format!(
            "{DIR}/{INDEX} is stale or missing: run `cargo xtask adr-index`"
        ));
    }
    eprintln!("    {} decision records, the index current", adrs.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "# ADR-0035: The 2700G's clocks\n\n- **Status:** **Accepted (owner, 2026-10-22)**\n- **Date:** 2026-10-22\n\n## Amendment: one\n\n## Amendment: two\n";

    #[test]
    fn a_record_gives_its_row() {
        let a = parse("0035-2700g-clocks.md", GOOD).unwrap();
        assert_eq!(a.number, "0035");
        assert_eq!(a.title, "The 2700G's clocks");
        assert_eq!(a.status, "Accepted");
        assert_eq!(a.date, "2026-10-22");
        assert_eq!(a.amendments, 2);
        let page = render(&[a]);
        assert!(page.contains(
            "| [0035](0035-2700g-clocks.md) | The 2700G's clocks | Accepted | 2026-10-22 | 2 |"
        ));
        assert!(page.contains("1 records: 1 accepted, 0 proposed"));
    }

    #[test]
    fn a_broken_record_is_named() {
        for (text, want) in [
            ("# ADR-0034: Wrong number\n", "first line"),
            ("# ADR-0035: T\n- **Date:** 2026-10-22\n", "Status"),
            ("# ADR-0035: T\n- **Status:** Pending\n", "opens with none"),
            ("# ADR-0035: T\n- **Status:** Proposed\n", "Date"),
        ] {
            let e = parse("0035-x.md", text).unwrap_err();
            assert!(e.contains(want), "{e}");
        }
        assert!(parse("x35-x.md", GOOD).unwrap_err().contains("four-digit"));
    }

    #[test]
    fn a_pipe_in_a_title_keeps_the_table() {
        let a = parse(
            "0001-x.md",
            "# ADR-0001: A | B\n- **Status:** Proposed\n- **Date:** d\n",
        )
        .unwrap();
        assert!(render(&[a]).contains("A \\| B"));
    }

    #[test]
    fn the_real_records_all_parse_and_the_index_is_current() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let adrs = collect(&root).unwrap();
        assert!(adrs.len() >= 35);
        check(&root).unwrap();
    }
}
