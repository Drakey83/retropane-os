//! The compatibility corpus scanner's model: which modules import which
//! functions from which DLLs, resolved and ranked (ROADMAP Phase 3, "import-
//! table scanner produces a ranked API backlog").
//!
//! Windows CE programs import `coredll` mostly *by ordinal*; ordinals are
//! resolved to names through the export tables of the DLLs found in the same
//! corpus. Only interface facts are kept (DLL names, function names,
//! ordinals, counts): nothing from the code.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

/// One imported function, as the importer names it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Symbol {
    /// By name.
    Name(String),
    /// By ordinal (resolved later when the exporter is known).
    Ordinal(u16),
}

/// What one module needs.
#[derive(Clone, Debug, Default)]
pub struct Module {
    /// File name, lower case.
    pub name: String,
    /// A DLL (else a program).
    pub dll: bool,
    /// (DLL lower case, symbol).
    pub imports: Vec<(String, Symbol)>,
    /// Ordinal -> name for the functions this module exports.
    pub exports: BTreeMap<u16, String>,
}

/// One ranked API.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Api {
    /// The exporting DLL.
    pub dll: String,
    /// Its name, or `#ordinal` when no export table in the corpus names it.
    pub function: String,
    /// The ordinal, when imported by one.
    pub ordinal: Option<u16>,
    /// How many modules import it.
    pub importers: usize,
    /// How many of those are programs (not DLLs).
    pub programs: usize,
}

/// The ranked backlog.
#[derive(Debug, Default)]
pub struct Backlog {
    /// Modules scanned.
    pub modules: usize,
    /// Of which programs.
    pub programs: usize,
    /// Every API, most needed first.
    pub apis: Vec<Api>,
    /// Per DLL: (importing modules, distinct functions used), most needed
    /// first.
    pub dlls: Vec<(String, usize, usize)>,
    /// Ordinal imports that no export table in the corpus names.
    pub unresolved: usize,
}

/// One function's use across the corpus: its ordinal, the modules that
/// import it, and how many of those are programs.
type Use<'a> = (Option<u16>, BTreeSet<&'a str>, usize);

/// Resolves and ranks the modules' imports.
#[must_use]
pub fn rank(modules: &[Module]) -> Backlog {
    let exports: BTreeMap<&str, &BTreeMap<u16, String>> = modules
        .iter()
        .filter(|m| !m.exports.is_empty())
        .map(|m| (m.name.as_str(), &m.exports))
        .collect();
    let mut uses: BTreeMap<(String, String), Use<'_>> = BTreeMap::new();
    let mut unresolved = 0;
    for m in modules {
        let mut seen = BTreeSet::new();
        for (dll, sym) in &m.imports {
            let (function, ordinal) = match sym {
                Symbol::Name(n) => (n.clone(), None),
                Symbol::Ordinal(o) => match exports.get(dll.as_str()).and_then(|e| e.get(o)) {
                    Some(n) => (n.clone(), Some(*o)),
                    None => {
                        unresolved += 1;
                        (format!("#{o}"), Some(*o))
                    }
                },
            };
            if !seen.insert((dll.clone(), function.clone())) {
                continue; // imported twice by the same module
            }
            let e = uses
                .entry((dll.clone(), function))
                .or_insert((ordinal, BTreeSet::new(), 0));
            if e.1.insert(m.name.as_str()) && !m.dll {
                e.2 += 1;
            }
            if e.0.is_none() {
                e.0 = ordinal;
            }
        }
    }
    let mut apis: Vec<Api> = uses
        .into_iter()
        .map(|((dll, function), (ordinal, who, programs))| Api {
            dll,
            function,
            ordinal,
            importers: who.len(),
            programs,
        })
        .collect();
    apis.sort_by(|a, b| {
        b.importers
            .cmp(&a.importers)
            .then(b.programs.cmp(&a.programs))
            .then(a.dll.cmp(&b.dll))
            .then(a.function.cmp(&b.function))
    });
    let mut per_dll: BTreeMap<&str, (BTreeSet<&str>, usize)> = BTreeMap::new();
    for m in modules {
        for (dll, _) in &m.imports {
            per_dll
                .entry(dll.as_str())
                .or_default()
                .0
                .insert(m.name.as_str());
        }
    }
    for a in &apis {
        if let Some(e) = per_dll.get_mut(a.dll.as_str()) {
            e.1 += 1;
        }
    }
    let mut dlls: Vec<(String, usize, usize)> = per_dll
        .into_iter()
        .map(|(d, (who, n))| (d.to_string(), who.len(), n))
        .collect();
    dlls.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    Backlog {
        modules: modules.len(),
        programs: modules.iter().filter(|m| !m.dll).count(),
        apis,
        dlls,
        unresolved,
    }
}

/// The backlog as Markdown: a summary, the DLLs, then the top functions of
/// each of the `top_dlls` most-needed DLLs (at most `per_dll` each).
#[must_use]
pub fn markdown(b: &Backlog, corpus: &str, top_dlls: usize, per_dll: usize) -> String {
    let mut s = String::new();
    s.push_str("# CE API backlog\n\n");
    s.push_str(
        "> Interface facts only (DLL and function names, ordinals, how many modules \
         import each), gathered for interoperability from binaries on the owner's own \
         device, which stay local (docs/CLEANROOM.md). Regenerate: `cargo run -p \
         rp-apiscan -- <dirs> --out docs/research/api-backlog.md`.\n\n",
    );
    s.push_str(&format!(
        "Generated by `tools/apiscan` from {corpus}: {} modules ({} programs, {} DLLs), \
         {} distinct functions imported, {} ordinal imports unresolved (no export table \
         in the corpus names them). Ranked by how many modules import each function: the \
         order the CE personality should provide them.\n\n",
        b.modules,
        b.programs,
        b.modules - b.programs,
        b.apis.len(),
        b.unresolved
    ));
    s.push_str("## DLLs\n\n| DLL | Importing modules | Functions used |\n|---|---|---|\n");
    for (d, who, n) in b.dlls.iter().take(40) {
        s.push_str(&format!("| `{d}` | {who} | {n} |\n"));
    }
    for (d, _, _) in b.dlls.iter().take(top_dlls) {
        s.push_str(&format!(
            "\n## `{d}`\n\n| Function | Ordinal | Importers | Programs |\n|---|---|---|---|\n"
        ));
        for a in b.apis.iter().filter(|a| &a.dll == d).take(per_dll) {
            let ord = a.ordinal.map_or(String::from("—"), |o| o.to_string());
            s.push_str(&format!(
                "| `{}` | {ord} | {} | {} |\n",
                a.function, a.importers, a.programs
            ));
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module(name: &str, dll: bool, imports: &[(&str, Symbol)]) -> Module {
        Module {
            name: name.into(),
            dll,
            imports: imports
                .iter()
                .map(|(d, s)| (d.to_string(), s.clone()))
                .collect(),
            exports: BTreeMap::new(),
        }
    }

    #[test]
    fn ordinals_resolve_through_the_corpus_and_rank_by_importers() {
        let mut coredll = module("coredll.dll", true, &[]);
        coredll.exports.insert(1, "CreateFileW".into());
        coredll.exports.insert(2, "ReadFile".into());
        let a = module(
            "a.exe",
            false,
            &[
                ("coredll.dll", Symbol::Ordinal(1)),
                ("coredll.dll", Symbol::Ordinal(2)),
                ("coredll.dll", Symbol::Ordinal(99)),
            ],
        );
        let b = module(
            "b.exe",
            false,
            &[
                ("coredll.dll", Symbol::Ordinal(1)),
                // The same function twice in one module counts once.
                ("coredll.dll", Symbol::Name("CreateFileW".into())),
                ("aygshell.dll", Symbol::Name("SHInitDialog".into())),
            ],
        );
        let c = module(
            "c.dll",
            true,
            &[("coredll.dll", Symbol::Name("ReadFile".into()))],
        );
        let r = rank(&[coredll, a, b, c]);
        assert_eq!((r.modules, r.programs, r.unresolved), (4, 2, 1));
        assert_eq!(r.apis[0].function, "CreateFileW");
        assert_eq!((r.apis[0].importers, r.apis[0].programs), (2, 2));
        assert_eq!(r.apis[0].ordinal, Some(1));
        let read = r.apis.iter().find(|a| a.function == "ReadFile").unwrap();
        assert_eq!((read.importers, read.programs), (2, 1));
        assert!(r.apis.iter().any(|a| a.function == "#99"));
        assert_eq!(r.dlls[0], ("coredll.dll".into(), 3, 3));
        let md = markdown(&r, "a test corpus", 2, 10);
        assert!(md.contains("| `coredll.dll` | 3 | 3 |"));
        assert!(md.contains("| `CreateFileW` | 1 | 2 | 2 |"));
    }
}
