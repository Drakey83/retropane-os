//! `cargo xtask xash-profile <samples.bin> [elf folder]`: where a run of the
//! port runner spent its time, from the program-counter samples it wrote
//! with `-rpsample` (`rt::start_sampling`, R285).
//!
//! Each sample is the interrupted instruction and the return address of
//! the function it was in. Both are named against the ELFs that ran (the
//! runner's and each module's, linked at the addresses they run at,
//! ADR-0039 rule 2), and counted by the run's steps: the engine's start
//! (step 2 to 3), the map's start (3 to 4), the play (4 to 5) and the
//! end. A leaf function's samples (the software-float helpers, `memcpy`)
//! are also counted by their callers, which a flat count of where the
//! program was cannot say.
//!
//! Iterate, don't imitate (rule 0). The textbook profiler instruments the
//! program (`gprof`'s `-pg`: a call into the profiler at every function's
//! start, which on this CPU would swamp the small float helpers it should
//! measure); Windows CE's Monte Carlo profiler and Linux's `perf` sample
//! from a timer or a counter's interrupt inside the OS and keep a kernel
//! buffer. Ours samples the unchanged program on the X51v itself (the
//! emulator's CPU is not the device's), keeps the samples in the runner's
//! own RAM, names them on the PC against the very files that ran, and
//! adds each sample's caller, so a float helper's time is charged to the
//! code that asked for it.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

use crate::symbols::{Symbol, code_symbols, function_at, llvm_tool};

/// The samples' file, as the runner writes it (`SAMPLE_FILE`).
#[derive(Debug, PartialEq, Eq)]
pub struct Samples {
    pub hz: u32,
    /// Ticks that found the room full (samples lost at the end).
    pub full: u32,
    /// The sample at each of the run's steps 1 to 5 (`u32::MAX`: not reached).
    pub at: [u32; 5],
    /// Each sample: where, and the return address.
    pub pairs: Vec<(u32, u32)>,
}

const MAGIC: &[u8; 8] = b"RPSAMP1\0";
const HEAD: usize = 8 + 4 * 8;

/// The file's samples, or why it is not one.
pub fn parse(bytes: &[u8]) -> Result<Samples, String> {
    if bytes.len() < HEAD || &bytes[..8] != MAGIC {
        return Err("not a samples file (no RPSAMP1 header)".into());
    }
    let word = |k: usize| u32::from_le_bytes(bytes[8 + 4 * k..12 + 4 * k].try_into().unwrap());
    let (hz, taken, full) = (word(0), word(1) as usize, word(2));
    let at = [word(3), word(4), word(5), word(6), word(7)];
    let body = &bytes[HEAD..];
    if body.len() != taken * 8 {
        return Err(format!(
            "the header says {taken} samples, the file holds {} bytes of them",
            body.len()
        ));
    }
    let pairs = body
        .as_chunks::<8>()
        .0
        .iter()
        .map(|c| {
            (
                u32::from_le_bytes([c[0], c[1], c[2], c[3]]),
                u32::from_le_bytes([c[4], c[5], c[6], c[7]]),
            )
        })
        .collect();
    Ok(Samples {
        hz,
        full,
        at,
        pairs,
    })
}

/// The code of the files that ran: each one's name and its functions.
pub struct Code(Vec<(String, Vec<Symbol>)>);

impl Code {
    /// The functions of each ELF (`name`, path), from `llvm-nm`.
    pub fn load(elfs: &[(&str, &Path)]) -> Result<Self, String> {
        let mut out = Vec::new();
        for (name, elf) in elfs {
            if !elf.is_file() {
                return Err(format!("{}: no such ELF", elf.display()));
            }
            let nm = Command::new(llvm_tool("llvm-nm"))
                .args(["-n", "-S", "-C", "--defined-only"])
                .arg(elf)
                .output()
                .map_err(|e| format!("llvm-nm: {e}"))?;
            if !nm.status.success() {
                return Err(format!("llvm-nm: {}", String::from_utf8_lossy(&nm.stderr)));
            }
            out.push((
                (*name).to_string(),
                code_symbols(&String::from_utf8_lossy(&nm.stdout)),
            ));
        }
        Ok(Self(out))
    }

    /// From symbol lists already made (tests).
    #[cfg(test)]
    fn of(parts: Vec<(&str, Vec<Symbol>)>) -> Self {
        Self(parts.into_iter().map(|(n, s)| (n.to_string(), s)).collect())
    }

    /// `module: function` for an address, or where it is not code.
    fn name(&self, addr: u32) -> String {
        for (module, symbols) in &self.0 {
            if let Some((f, _)) = function_at(symbols, addr) {
                return format!("{module}: {f}");
            }
        }
        format!("(no function at {addr:#010x})")
    }

    /// The module an address is in.
    fn module(&self, addr: u32) -> &str {
        self.0
            .iter()
            .find(|(_, s)| function_at(s, addr).is_some())
            .map_or("(elsewhere)", |(m, _)| m.as_str())
    }
}

/// A leaf helper whose time is better told by its callers: the
/// software-float and integer-division helpers and the memory copies.
fn helper(function: &str) -> bool {
    let f = function.rsplit(": ").next().unwrap_or(function);
    f.starts_with("__aeabi_")
        || f.starts_with("__") && ["sf", "df", "si", "di"].iter().any(|t| f.contains(t))
        || ["memcpy", "memset", "memmove", "memcmp", "strlen", "strcmp"].contains(&f)
}

/// The report: each stretch of the run, its time, its modules' shares, the
/// functions most sampled, and the callers of the helpers among them.
#[must_use]
pub fn report(s: &Samples, code: &Code, top: usize) -> String {
    let n = s.pairs.len() as u32;
    let ms = |k: u32| u64::from(k) * 1000 / u64::from(s.hz.max(1));
    let at = |i: usize| s.at[i].min(n);
    let stretches = [
        ("the engine's start (step 2 to 3)", 0, at(2)),
        ("the map's start (step 3 to 4)", at(2), at(3)),
        ("the play (step 4 to 5)", at(3), at(4)),
        ("the end (step 5 on)", at(4), n),
    ];
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{n} samples at {} a second ({} ms){}",
        s.hz,
        ms(n),
        if s.full > 0 {
            format!(
                "; {} more found the room full: the run's last {} ms not sampled",
                s.full,
                ms(s.full)
            )
        } else {
            String::new()
        }
    );
    for (what, from, to) in stretches {
        let part = &s.pairs[from as usize..to.max(from) as usize];
        if part.is_empty() {
            continue;
        }
        let total = part.len() as f64;
        let pct = |k: usize| 100.0 * k as f64 / total;
        let _ = writeln!(
            out,
            "\n== {what}: {} samples, {} ms",
            part.len(),
            ms(part.len() as u32)
        );
        let mut modules: HashMap<&str, usize> = HashMap::new();
        let mut functions: HashMap<String, usize> = HashMap::new();
        let mut callers: HashMap<String, HashMap<String, usize>> = HashMap::new();
        for &(pc, lr) in part {
            *modules.entry(code.module(pc)).or_default() += 1;
            let f = code.name(pc);
            if helper(&f) {
                *callers
                    .entry(f.clone())
                    .or_default()
                    .entry(code.name(lr))
                    .or_default() += 1;
            }
            *functions.entry(f).or_default() += 1;
        }
        let mut modules: Vec<_> = modules.into_iter().collect();
        modules.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        let _ = write!(out, "by module:");
        for (m, k) in &modules {
            let _ = write!(out, " {m} {:.1}%", pct(*k));
        }
        let _ = writeln!(out);
        let helpers: usize = functions
            .iter()
            .filter(|(f, _)| helper(f))
            .map(|(_, k)| k)
            .sum();
        let _ = writeln!(
            out,
            "in the helpers (software float, division, memory copies): {:.1}%",
            pct(helpers)
        );
        let mut functions: Vec<_> = functions.into_iter().collect();
        functions.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        for (f, k) in functions.iter().take(top) {
            let _ = writeln!(out, "  {:5.1}%  {k:6}  {f}", pct(*k));
            if let Some(by) = callers.get(f) {
                let mut by: Vec<_> = by.iter().collect();
                by.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
                for (c, j) in by.iter().take(3) {
                    let _ = writeln!(
                        out,
                        "           {:5.1}% of it from {c}",
                        100.0 * **j as f64 / *k as f64
                    );
                }
            }
        }
    }
    out
}

/// `cargo xtask xash-profile`: the report for `file`, named against the
/// modules' ELFs in `modules` (`engine.elf`, `client.elf`, `server.elf`)
/// and the runner's ELF `runner`: the kit keeps the ones it sent in its
/// `elf` folder.
pub fn run(file: &Path, modules: &Path, runner: &Path) -> Result<String, String> {
    let bytes = std::fs::read(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let samples = parse(&bytes)?;
    let runner = runner.to_path_buf();
    let (engine, client, server) = (
        modules.join("engine.elf"),
        modules.join("client.elf"),
        modules.join("server.elf"),
    );
    let code = Code::load(&[
        ("engine", &engine),
        ("client", &client),
        ("server", &server),
        ("runner", &runner),
    ])?;
    Ok(report(&samples, &code, 25))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(hz: u32, full: u32, at: [u32; 5], pairs: &[(u32, u32)]) -> Vec<u8> {
        let mut b = MAGIC.to_vec();
        for w in [hz, pairs.len() as u32, full].iter().chain(at.iter()) {
            b.extend_from_slice(&w.to_le_bytes());
        }
        for (p, l) in pairs {
            b.extend_from_slice(&p.to_le_bytes());
            b.extend_from_slice(&l.to_le_bytes());
        }
        b
    }

    fn sym(start: u32, size: u32, name: &str) -> Symbol {
        Symbol {
            start,
            size,
            name: name.into(),
        }
    }

    #[test]
    fn a_samples_file_reads_back_and_a_damaged_one_is_refused() {
        let pairs = [(0x10, 0x20), (0x30, 0x40)];
        let at = [0, 0, 1, u32::MAX, u32::MAX];
        let b = file(1000, 3, at, &pairs);
        assert_eq!(
            parse(&b).unwrap(),
            Samples {
                hz: 1000,
                full: 3,
                at,
                pairs: pairs.to_vec()
            }
        );
        assert!(parse(&b[..b.len() - 1]).is_err(), "a sample cut short");
        assert!(parse(b"RPSAMP2\0").is_err());
        let mut wrong = b.clone();
        wrong[0] = b'X';
        assert!(parse(&wrong).is_err());
    }

    #[test]
    fn samples_are_named_split_by_step_and_a_helper_is_charged_to_its_callers() {
        let code = Code::of(vec![
            (
                "engine",
                vec![
                    sym(0x1000, 0x100, "R_BuildLightMap"),
                    sym(0x1100, 0x40, "__aeabi_fmul"),
                    sym(0x1200, 0x80, "Mod_LoadFaces"),
                ],
            ),
            ("runner", vec![sym(0x9000, 0x40, "memcpy")]),
        ]);
        // The engine's start: 2 samples; the map's: 6; the play: 2.
        let pairs = [
            (0x1210, 0),
            (0x9004, 0x1220),
            (0x1110, 0x1010),
            (0x1110, 0x1010),
            (0x1110, 0x1210),
            (0x1004, 0),
            (0x1004, 0),
            (0x5000, 0),
            (0x1004, 0),
            (0x1004, 0),
        ];
        let at = [0, 0, 2, 8, 10];
        let r = report(&parse(&file(1000, 0, at, &pairs)).unwrap(), &code, 10);
        let map = r
            .split("== the map's start")
            .nth(1)
            .unwrap()
            .split("== the play")
            .next()
            .unwrap();
        assert!(map.contains("6 samples, 6 ms"), "{map}");
        assert!(map.contains("50.0%       3  engine: __aeabi_fmul"), "{map}");
        assert!(
            map.contains("66.7% of it from engine: R_BuildLightMap"),
            "{map}"
        );
        assert!(
            map.contains("33.3% of it from engine: Mod_LoadFaces"),
            "{map}"
        );
        assert!(map.contains("(no function at 0x00005000)"), "{map}");
        assert!(
            map.contains("in the helpers (software float, division, memory copies): 50.0%"),
            "{map}"
        );
        assert!(
            map.contains("by module: engine 83.3% (elsewhere) 16.7%"),
            "{map}"
        );
        let start = r
            .split("== the engine's start")
            .nth(1)
            .unwrap()
            .split("== the map")
            .next()
            .unwrap();
        assert!(
            start.contains("runner: memcpy") && start.contains("of it from engine: Mod_LoadFaces"),
            "{start}"
        );
        assert!(
            !r.contains("== the end"),
            "no samples after step 5: no stretch"
        );
    }

    #[test]
    fn helpers_are_the_float_division_and_copy_routines() {
        for f in [
            "engine: __aeabi_fmul",
            "client: __addsf3",
            "engine: __divdi3",
            "runner: memcpy",
            "engine: memset",
        ] {
            assert!(helper(f), "{f}");
        }
        for f in [
            "engine: R_BuildLightMap",
            "engine: memcpy_like_but_not",
            "server: __cxa_guard_acquire",
        ] {
            assert!(!helper(f), "{f}");
        }
    }
}
