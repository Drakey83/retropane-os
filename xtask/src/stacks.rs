//! P6 of docs/design/weather-rule0.md: the stack, checked by the build. The
//! images are built again with `-Zemit-stack-sizes` (in [`DIR`]): the
//! compiler writes each function's frame into `.stack_sizes`, exact, where
//! [`crate::frames`] reads `sub sp` from the disassembly (and skipped every
//! image with a library part, rp-today among them: its shell made on the
//! stack took a 15.8 KB frame and overflowed the X51v's 24 KiB, today-0037).
//!
//! Two rules, every image:
//! - no function's frame is 4 KB or more, but an image's `main` and the
//!   reviewed set-up functions of the test images ([`crate::frames::REVIEWED`]);
//! - the deepest path of direct calls from the image's entry is under 75%
//!   of its stack (`__stack_top - __stack_bottom`, boot/link.x). Calls
//!   through a pointer (`blx rN`) can't be followed and are counted and
//!   printed; recursion is printed too (each cycle counted once).
//!
//! Linux (`checkstack.pl`) and the embedded tools (`cargo call-stack`,
//! StackAnalyzer) do the same two halves; ours reads the compiler's own
//! numbers from the very build the X51v runs, in CI, every commit.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The build folder of the frame-size build.
pub const DIR: &str = "target-stack";
/// The flag that has rustc write `.stack_sizes`.
pub const FLAG: &str = "-Zemit-stack-sizes";
/// A frame this big or bigger fails (ARM's load reach, and P6's budget).
const FRAME_LIMIT: u32 = 4096;
/// The deepest path's share of the stack that fails, in percent.
const DEPTH_LIMIT_PERCENT: u32 = 75;

/// Images over the depth limit when the rule came in (2026-10-31), each at
/// its measured deepest path: it may shrink, never grow, until its fix
/// (docs/reminders.md R149) brings it under the limit and it leaves the list.
pub const DEPTH_OVER: &[(&str, u32)] = &[
    // The MTP format path: main's 10.6 KB frame, the guard rebuilt with
    // its Guard and volume on the stack (5.2 + 3.2 + 5.0 KB), then a
    // panic's report: 87% of 32 KiB, static and through direct calls.
    ("rp-loader", 28_696),
    // A test image: main 4.4 KB, save 2.4 KB, mount 4.0 KB.
    ("rp-noise", 12_296),
];

/// An LLVM tool next to `llvm-objcopy` (the toolchain's `llvm-tools`).
fn tool(name: &str) -> PathBuf {
    let mut p = crate::sdkit::objcopy();
    p.set_file_name(if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    });
    p
}

fn run(tool_name: &str, args: &[&str], elf: &Path) -> Result<String, String> {
    let out = Command::new(tool(tool_name))
        .args(args)
        .arg(elf)
        .output()
        .map_err(|e| format!("{tool_name}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{tool_name} {}: {}",
            elf.display(),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `llvm-readobj --stack-sizes`: each function's frame, by mangled name
/// (several names for one body when the linker folded them).
pub fn frame_sizes(readobj: &str) -> HashMap<String, u32> {
    let mut out = HashMap::new();
    let mut names: Vec<String> = Vec::new();
    for line in readobj.lines() {
        let t = line.trim();
        if let Some(list) = t
            .strip_prefix("Functions: [")
            .and_then(|l| l.strip_suffix(']'))
        {
            // Mangled names hold no ", " (v0 and legacy alike).
            names = list.split(", ").map(str::to_string).collect();
        } else if let Some(n) = t.strip_prefix("Size: 0x")
            && let Ok(n) = u32::from_str_radix(n, 16)
        {
            for name in names.drain(..) {
                let e = out.entry(name).or_insert(0);
                *e = (*e).max(n);
            }
        }
    }
    out
}

/// One function's calls, read from the disassembly.
#[derive(Default, Debug, PartialEq)]
pub struct Calls {
    /// Called with a link (`bl`, `blx <f>`): its frame on top of ours.
    pub calls: Vec<String>,
    /// Branched to (a tail call): its frame in place of ours.
    pub tails: Vec<String>,
    /// Calls through a register: not followed.
    pub indirect: u32,
}

const CONDS: &[&str] = &[
    "", "eq", "ne", "cs", "hs", "cc", "lo", "mi", "pl", "vs", "vc", "hi", "ls", "ge", "lt", "gt",
    "le", "al",
];

/// `llvm-objdump -d --no-show-raw-insn` (names not demangled): each
/// function's direct calls and tail calls (a branch to another function's
/// first instruction) and its calls through a register.
pub fn call_graph(disassembly: &str) -> HashMap<String, Calls> {
    let mut out: HashMap<String, Calls> = HashMap::new();
    let mut function = String::new();
    for line in disassembly.lines() {
        if let Some(name) = line
            .strip_suffix(">:")
            .and_then(|l| l.split_once(" <"))
            .map(|(_, n)| n)
        {
            function = name.to_string();
            out.entry(function.clone()).or_default();
            continue;
        }
        let mut parts = line.split('\t').map(str::trim).filter(|p| !p.is_empty());
        let (Some(addr), Some(mnemonic)) = (parts.next(), parts.next()) else {
            continue;
        };
        if !addr.ends_with(':') || function.is_empty() {
            continue;
        }
        let operands = parts.next().unwrap_or("");
        let m = mnemonic.trim_end_matches(".w").trim_end_matches(".n");
        let target = operands
            .split_once('<')
            .and_then(|(_, r)| r.split_once('>'))
            .map(|(n, _)| n)
            .filter(|n| !n.contains('+'));
        let entry = out.entry(function.clone()).or_default();
        let link = m
            .strip_prefix("blx")
            .or_else(|| m.strip_prefix("bl"))
            .is_some_and(|c| CONDS.contains(&c));
        if link {
            match target {
                Some(t) => entry.calls.push(t.to_string()),
                None if !operands.contains('<') => entry.indirect += 1,
                None => {}
            }
        } else if m.strip_prefix('b').is_some_and(|c| CONDS.contains(&c)) {
            if let Some(t) = target.filter(|t| *t != function) {
                entry.tails.push(t.to_string());
            }
        } else if m.strip_prefix("bx").is_some_and(|c| CONDS.contains(&c)) && operands != "lr" {
            // A jump through a register that is not a return.
            entry.indirect += 1;
        }
    }
    out
}

/// The deepest path from `root`: (bytes, the path's functions).
pub struct Depth {
    pub bytes: u32,
    pub path: Vec<String>,
    /// Functions on paths that call back into themselves (counted once).
    pub cycles: HashSet<String>,
    /// Calls through a register in the functions reachable from `root`.
    pub indirect: u32,
}

/// The deepest stack of direct calls from `root`, each function's frame
/// from `frames` (none: 0, assembly).
pub fn deepest(root: &str, graph: &HashMap<String, Calls>, frames: &HashMap<String, u32>) -> Depth {
    struct Walk<'a> {
        graph: &'a HashMap<String, Calls>,
        frames: &'a HashMap<String, u32>,
        done: HashMap<String, (u32, Option<String>)>,
        on_path: HashSet<String>,
        cycles: HashSet<String>,
    }
    impl Walk<'_> {
        /// (depth at entry, the next function on the deepest path).
        fn depth(&mut self, f: &str) -> u32 {
            if let Some(&(d, _)) = self.done.get(f) {
                return d;
            }
            if !self.on_path.insert(f.to_string()) {
                self.cycles.insert(f.to_string());
                return 0;
            }
            let frame = self.frames.get(f).copied().unwrap_or(0);
            let mut best = (frame, None);
            if let Some(c) = self.graph.get(f) {
                let calls = c.calls.clone();
                let tails = c.tails.clone();
                for callee in calls {
                    let d = frame + self.depth(&callee);
                    if d > best.0 {
                        best = (d, Some(callee));
                    }
                }
                for callee in tails {
                    let d = self.depth(&callee);
                    if d > best.0 {
                        best = (d, Some(callee));
                    }
                }
            }
            self.on_path.remove(f);
            self.done.insert(f.to_string(), best.clone());
            best.0
        }
    }
    let mut w = Walk {
        graph,
        frames,
        done: HashMap::new(),
        on_path: HashSet::new(),
        cycles: HashSet::new(),
    };
    let bytes = w.depth(root);
    let mut path = vec![root.to_string()];
    let mut seen = HashSet::from([root.to_string()]);
    while let Some((_, Some(next))) = w.done.get(path.last().unwrap()) {
        if !seen.insert(next.clone()) {
            break;
        }
        path.push(next.clone());
    }
    let indirect = w
        .done
        .keys()
        .filter_map(|f| graph.get(f))
        .map(|c| c.indirect)
        .sum();
    Depth {
        bytes,
        path,
        cycles: w.cycles,
        indirect,
    }
}

/// `llvm-nm -n` lines: (address, name) of every symbol.
fn symbols(nm: &str) -> Vec<(u32, String)> {
    nm.lines()
        .filter_map(|l| {
            let mut p = l.splitn(3, ' ');
            let addr = u32::from_str_radix(p.next()?, 16).ok()?;
            let _kind = p.next()?;
            Some((addr, p.next()?.to_string()))
        })
        .collect()
}

/// The image's own `main` (demangled `<crate>::...::main` or `main`).
fn is_main(demangled: &str) -> bool {
    demangled == "main" || demangled.ends_with("::main")
}

/// Checks every image in `elfs`; prints each one's frame and depth line.
pub fn check(elfs: &[PathBuf]) -> Result<(), String> {
    let mut failed = Vec::new();
    eprintln!("    image                 stack  deepest  share  through a pointer");
    for elf in elfs {
        let image = elf
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let frames = frame_sizes(&run("llvm-readobj", &["--stack-sizes"], elf)?);
        if frames.is_empty() {
            failed.push(format!("{image}: no .stack_sizes (built without {FLAG}?)"));
            continue;
        }
        // Mangled and demangled names, line for line.
        let raw = symbols(&run("llvm-nm", &["-n", "--defined-only"], elf)?);
        let nice = symbols(&run("llvm-nm", &["-n", "-C", "--defined-only"], elf)?);
        let demangle: HashMap<&str, &str> = raw
            .iter()
            .zip(&nice)
            .map(|((_, m), (_, d))| (m.as_str(), d.as_str()))
            .collect();
        let name = |m: &str| demangle.get(m).copied().unwrap_or(m).to_string();
        // By demangled name: one generic made twice (two hashes) is one line.
        let mut big: Vec<(String, u32)> = Vec::new();
        for (f, &bytes) in &frames {
            let d = name(f);
            let entry = format!("{image}: {d}");
            if bytes >= FRAME_LIMIT
                && !is_main(&d)
                && !crate::frames::REVIEWED.contains(&entry.as_str())
                && !big.iter().any(|(e, _)| *e == entry)
            {
                big.push((entry, bytes));
            }
        }
        big.sort();
        failed.extend(big.into_iter().map(|(e, b)| format!("{e}: a {b} B frame")));
        let sym = |s: &str| raw.iter().find(|(_, n)| n == s).map(|(a, _)| *a);
        let headers = run("llvm-readobj", &["--file-headers"], elf)?;
        let entry_addr = headers
            .lines()
            .find_map(|l| l.trim().strip_prefix("Entry: 0x"))
            .and_then(|a| u32::from_str_radix(a, 16).ok());
        // The reset code (`_start`, boot/link.x), else the entry's function:
        // the last text symbol at that address (labels like `$a` aside). The
        // loader's entry is its vector table, whose jumps are loads of pc.
        let root = sym("_start").map(|_| "_start".to_string()).or_else(|| {
            entry_addr.and_then(|a| {
                raw.iter()
                    .rev()
                    .find(|(s, n)| *s == a && !n.starts_with('$'))
                    .map(|(_, n)| n.clone())
            })
        });
        let Some(root) = root else {
            failed.push(format!("{image}: no function at the ELF's entry"));
            continue;
        };
        let graph = call_graph(&run("llvm-objdump", &["-d", "--no-show-raw-insn"], elf)?);
        let depth = deepest(&root, &graph, &frames);
        let stack = sym("__stack_top")
            .zip(sym("__stack_bottom"))
            .map(|(t, b)| t.saturating_sub(b));
        let share = stack.map(|s| depth.bytes * 100 / s.max(1));
        eprintln!(
            "    {image:<20} {:>6}  {:>7}  {:>5}  {}{}",
            stack.map_or("-".into(), |s| s.to_string()),
            depth.bytes,
            share.map_or("-".into(), |p| format!("{p}%")),
            depth.indirect,
            if depth.cycles.is_empty() {
                String::new()
            } else {
                format!(", {} recursive", depth.cycles.len())
            }
        );
        let path: Vec<String> = depth
            .path
            .iter()
            .map(|f| format!("{} ({} B)", name(f), frames.get(f).copied().unwrap_or(0)))
            .collect();
        // `RP_STACK_PATH=<image>`: that image's deepest path, passing or not.
        if std::env::var("RP_STACK_PATH").is_ok_and(|v| v == image) {
            eprintln!(
                "      {}",
                path.join(
                    "
      "
                )
            );
        }
        let over = DEPTH_OVER
            .iter()
            .find(|(i, _)| *i == image)
            .map(|(_, b)| *b);
        if let Some(was) = over
            && depth.bytes <= was
            && share.is_some_and(|p| p >= DEPTH_LIMIT_PERCENT)
        {
            eprintln!("      (over the limit, listed at {was} B: may only shrink, R149)");
        } else if let Some(p) = share
            && p >= DEPTH_LIMIT_PERCENT
        {
            failed.push(format!(
                "{image}: the deepest path is {} B, {p}% of its {} B stack (limit \
                 {DEPTH_LIMIT_PERCENT}%):\n      {}",
                depth.bytes,
                stack.unwrap_or(0),
                path.join("\n      ")
            ));
        }
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "the stack rule (xtask/src/stacks.rs, docs/design/weather-rule0.md P6: make a big \
             object in its slot with rt::Slot::put_in, keep big arrays out of frames):\n  {}",
            failed.join("\n  ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_frames_with_folded_names() {
        let r = "StackSizes [\n  Entry {\n    Functions: [_RNa, _RNb]\n    Size: 0x22B8\n  }\n  \
                 Entry {\n    Functions: [_RNc]\n    Size: 0x10\n  }\n]";
        let f = frame_sizes(r);
        assert_eq!(f["_RNa"], 0x22B8);
        assert_eq!(f["_RNb"], 0x22B8);
        assert_eq!(f["_RNc"], 0x10);
    }

    #[test]
    fn tells_calls_from_branches_and_conditions() {
        let d = "\
a0000000 <f>:
a0000000:      \tpush\t{r4, lr}
a0000004:      \tbl\t0xa0000100 <g>
a0000008:      \tblne\t0xa0000200 <h>
a000000c:      \tblt\t0xa0000010 <f+0x10>
a0000010:      \tble\t0xa0000300 <k>
a0000014:      \tblx\tr3
a0000018:      \tbx\tlr
a000001c:      \tb.w\t0xa0000400 <m>
a0000020:      \tbx\tr2
a0000024:      \tbic\tr0, r0, #1
a0000028:      \tldr\tr0, [pc, #0x10]        @ 0xa0000500 <n>
";
        let g = call_graph(d);
        assert_eq!(g["f"].calls, ["g", "h"]);
        assert_eq!(g["f"].tails, ["k", "m"]);
        assert_eq!(g["f"].indirect, 2);
    }

    #[test]
    fn deepest_adds_calls_replaces_tails_and_survives_recursion() {
        let mut graph = HashMap::new();
        graph.insert(
            "main".to_string(),
            Calls {
                calls: vec!["a".into(), "b".into()],
                tails: vec![],
                indirect: 1,
            },
        );
        graph.insert(
            "a".to_string(),
            Calls {
                calls: vec!["a".into()],
                tails: vec!["big".into()],
                indirect: 0,
            },
        );
        graph.insert("b".to_string(), Calls::default());
        graph.insert("big".to_string(), Calls::default());
        let frames = HashMap::from([
            ("main".to_string(), 100),
            ("a".to_string(), 10),
            ("b".to_string(), 50),
            ("big".to_string(), 300),
        ]);
        let d = deepest("main", &graph, &frames);
        // main + big in a's place (a's tail call): 100 + 300.
        assert_eq!(d.bytes, 400);
        assert_eq!(d.path, ["main", "a", "big"]);
        assert!(d.cycles.contains("a"));
        assert_eq!(d.indirect, 1);
    }

    #[test]
    fn main_is_the_images_own() {
        assert!(is_main("rp_today::image::main"));
        assert!(is_main("main"));
        assert!(!is_main("rp_today::image::domain"));
        assert!(!is_main("rp_today::image::main::{closure#9}"));
    }
}
