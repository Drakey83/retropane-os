//! Line endings in the working tree: every tracked text file as git keeps
//! it (LF, `.gitattributes` `* text=auto eol=lf`). A tool on Windows that
//! writes CRLF (Python's text mode without `newline='\n'`, PowerShell's
//! `Set-Content`) leaves `git status` clean, since git normalises on commit,
//! but the line-matching checks see `\r\n`: on 2026-10-24 ten such files
//! failed `cargo test --workspace` as a "stale mutant" (the mutant's text
//! was there, with LF). `cargo xtask eol`, CI's first step, names them.

use std::path::Path;
use std::process::Command;

/// The tracked files whose working copy holds a CRLF line ending (all CRLF
/// or mixed), from `git ls-files --eol -z`: entries `i/<index> w/<working>
/// attr/<attributes>\t<path>`, NUL-separated. Binary files (`w/-text`) and
/// files missing from the working tree (`w/`) aren't text with endings.
fn crlf_files(listing: &str) -> Vec<&str> {
    listing
        .split('\0')
        .filter_map(|entry| {
            let (info, path) = entry.split_once('\t')?;
            info.split_whitespace()
                .any(|field| field == "w/crlf" || field == "w/mixed")
                .then_some(path)
        })
        .collect()
}

/// The error for `files`: each named, and how to put them back.
fn report(files: &[&str]) -> String {
    let mut s = format!(
        "{} tracked file(s) have CRLF line endings in the working tree (git keeps LF; \
         a Windows tool rewrote them):\n",
        files.len()
    );
    for f in files {
        s.push_str("    ");
        s.push_str(f);
        s.push('\n');
    }
    s.push_str(
        "  with `git status` clean, delete them and `git checkout -- <file>` each; \
         a script that writes repo files opens them with newline='\\n'",
    );
    s
}

/// CI's step: fails naming every tracked file with a CRLF working copy.
pub fn check(root: &Path) -> Result<(), String> {
    let out = Command::new("git")
        .args(["ls-files", "--eol", "-z"])
        .current_dir(root)
        .output()
        .map_err(|e| format!("git ls-files --eol: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git ls-files --eol: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let listing = String::from_utf8_lossy(&out.stdout);
    let files = crlf_files(&listing);
    if files.is_empty() {
        Ok(())
    } else {
        Err(report(&files))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LF: &str = "i/lf    w/lf    attr/text=auto eol=lf \tdocs/ROADMAP.md";
    const CRLF: &str = "i/lf    w/crlf  attr/text=auto eol=lf \tboot/glesfeat/src/main.rs";
    const MIXED: &str = "i/lf    w/mixed attr/text=auto eol=lf \tassets/Read Me (a).txt";

    #[test]
    fn a_clean_tree_passes() {
        let listing = [LF, "i/-text w/-text attr/binary \tdocs/a.png", ""].join("\0");
        assert!(crlf_files(&listing).is_empty());
        assert!(crlf_files("").is_empty());
    }

    #[test]
    fn crlf_and_mixed_files_are_named_with_their_whole_paths() {
        let listing = [LF, CRLF, MIXED, ""].join("\0");
        assert_eq!(
            crlf_files(&listing),
            ["boot/glesfeat/src/main.rs", "assets/Read Me (a).txt"]
        );
    }

    #[test]
    fn only_the_working_copy_counts() {
        // CRLF kept in the index but LF on disk: git's business, not this
        // check's; a path that merely contains the words isn't matched.
        let listing = [
            "i/crlf  w/lf    attr/text=auto eol=lf \tx.rs",
            "i/lf    w/lf    attr/text=auto eol=lf \tw/crlf notes.md",
            "i/lf    w/      attr/text=auto eol=lf \tdeleted.rs",
        ]
        .join("\0");
        assert!(crlf_files(&listing).is_empty());
    }

    #[test]
    fn the_report_names_every_file_and_the_fix() {
        let r = report(&["a.rs", "b c.md"]);
        assert!(r.starts_with("2 tracked file(s)"));
        assert!(r.contains("\n    a.rs\n    b c.md\n"));
        assert!(r.contains("git checkout -- <file>"));
        assert!(r.contains("newline='\\n'"));
    }
}
