//! The performance ratchet (the gate's step 2b; docs/design/perf-ratchet.md):
//! a device suite of speed numbers, each held at its floor in
//! `docs/perf-floors.md` with a tolerance measured from its own noise. A
//! number slower than its floor beyond its tolerance fails until it's fixed
//! or the owner approves the new floor; a number faster beyond it ratchets
//! the floor down (written with `--update`).
//!
//! `cargo xtask perf` runs the suite once on the docked X51v (the loader
//! up) and compares; `cargo xtask perf noise N` runs it `N` times and
//! writes every number's floor (the median) and tolerance (the runs'
//! spread) afresh. Every run's numbers go to `local/perf/`.
//!
//! Every number is a time (lower is better): the probe's ns an iteration,
//! the test images' microseconds. Touch and frame numbers wait for the
//! owner's call on taps; battery for a battery reading (gate P16).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The floors file (committed).
pub const FLOORS: &str = "docs/perf-floors.md";

/// One image of the suite: its name, the build switch for its variant (if
/// any), and the run-log line heads whose `Some(N)` numbers are kept.
struct Step {
    image: &'static str,
    env: Option<(&'static str, &'static str)>,
    heads: &'static [&'static str],
    /// The probe's benchmark rows (`P<page>|name value checksum`).
    probe_rows: bool,
}

const SUITE: &[Step] = &[
    Step {
        image: "rp-probe",
        env: None,
        heads: &[],
        probe_rows: true,
    },
    Step {
        image: "rp-gpubench",
        env: Some(("RP_GPUBENCH_TOGETHER", "1")),
        heads: &["FT alone"],
        probe_rows: false,
    },
    Step {
        image: "rp-mbx-blend",
        env: Some(("RP_MBX_BLEND_B7", "1")),
        heads: &["B7d "],
        probe_rows: false,
    },
    Step {
        image: "rp-mbx-blend",
        env: Some(("RP_MBX_BLEND_B8", "1")),
        heads: &["B8 dither add"],
        probe_rows: false,
    },
    Step {
        image: "rp-mbx-blend",
        env: Some(("RP_MBX_BLEND_B9", "1")),
        heads: &["B9e "],
        probe_rows: false,
    }, // The touch replay (the owner's hybrid, 2026-10-22): recorded taps
    // through rp-today's filter, shell and drawing.
    Step {
        image: "rp-today",
        env: Some(("RP_TODAY_REPLAY", "1")),
        heads: &["TR touch replay:"],
        probe_rows: false,
    },
];

/// Lowercase letters and digits kept, every run of anything else one `_`.
fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('_') && !out.is_empty() {
            out.push('_');
        }
    }
    out.trim_end_matches('_').to_string()
}

/// The numbers in one log line that starts with `head`: each `Some(N)`
/// named after the line's head (to its first `:`) and the words before it
/// since the last `:`, `;` or `,` after the head. `None`s are skipped (a
/// part that didn't finish is reported by the run, not timed).
fn line_numbers(line: &str, out: &mut BTreeMap<String, f64>) {
    let line = line.trim();
    let (head, rest) = match line.split_once(':') {
        Some((h, r)) => (h, r),
        None => return,
    };
    let mut seg_start = 0;
    let mut at = 0;
    while let Some(k) = rest[at..].find("Some(") {
        let open = at + k;
        let Some(close) = rest[open..].find(')').map(|c| open + c) else {
            break;
        };
        let seg = &rest[seg_start..open];
        let seg = seg.rfind([':', ';', ',']).map_or(seg, |d| &seg[d + 1..]);
        if let Ok(v) = rest[open + 5..close].trim().parse::<f64>() {
            let name = if slug(seg).is_empty() {
                slug(head)
            } else {
                format!("{}.{}", slug(head), slug(seg))
            };
            out.insert(name, v);
        }
        at = close + 1;
        seg_start = at;
    }
}

/// The probe's benchmark rows: `P<page>|<name> <value> <checksum>`, value
/// in ns an iteration (not the headings or `check` lines).
fn probe_numbers(log: &str, out: &mut BTreeMap<String, f64>) {
    for l in log.lines() {
        let Some((page, text)) = l.trim().split_once('|') else {
            continue;
        };
        if !page.starts_with('P') {
            continue;
        }
        let words: Vec<&str> = text.split_whitespace().collect();
        if let [name, value, _sum] = words.as_slice()
            && let Ok(v) = value.parse::<f64>()
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            out.insert(format!("probe.{name}"), v);
        }
    }
}

/// Every number a step's run log holds.
fn step_numbers(step: &Step, log: &str) -> BTreeMap<String, f64> {
    let mut out = BTreeMap::new();
    if step.probe_rows {
        probe_numbers(log, &mut out);
    }
    for l in log.lines() {
        let t = l.trim();
        if step.heads.iter().any(|h| t.starts_with(h)) {
            line_numbers(t, &mut out);
        }
    }
    out
}

/// One floor: the number's floor, its tolerance (a fraction), the run that
/// set it, and the owner's approval of a slower floor if any.
#[derive(Clone, Debug, PartialEq)]
pub struct Floor {
    pub floor: f64,
    pub tolerance: f64,
    pub set_by: String,
    pub approved: String,
}

/// The floors file's rows: `| name | floor | tolerance % | set by | approved |`.
pub fn parse_floors(text: &str) -> BTreeMap<String, Floor> {
    let mut out = BTreeMap::new();
    for l in text.lines() {
        let cells: Vec<&str> = l.split('|').map(str::trim).collect();
        // ["", name, floor, tol, set_by, approved, ""]
        if cells.len() < 7 || cells[1].starts_with('-') || cells[1] == "Number" {
            continue;
        }
        let (Ok(floor), Ok(tol)) = (cells[2].parse::<f64>(), cells[3].parse::<f64>()) else {
            continue;
        };
        out.insert(
            cells[1].trim_matches('`').to_string(),
            Floor {
                floor,
                tolerance: tol / 100.0,
                set_by: cells[4].to_string(),
                approved: cells[5].to_string(),
            },
        );
    }
    out
}

/// The floors file's text for `floors`.
pub fn floors_text(floors: &BTreeMap<String, Floor>) -> String {
    let mut s = String::from(
        "# Performance floors (the ratchet: docs/design/perf-ratchet.md)\n\n\
         Written by `cargo xtask perf noise N` and `cargo xtask perf --update`; a\n\
         slower floor needs the owner's approval in its last column. Every number is\n\
         a time on the X51v (lower is better): the probe's ns an iteration, the test\n\
         images' microseconds. The tolerance is the noise measured across the runs\n\
         that set the floor (at least one unit of the number).\n\n\
         | Number | Floor | Tolerance % | Set by | Approved |\n|---|---|---|---|---|\n",
    );
    for (name, f) in floors {
        s += &format!(
            "| `{name}` | {} | {:.4} | {} | {} |\n",
            f.floor,
            f.tolerance * 100.0,
            f.set_by,
            f.approved
        );
    }
    s
}

/// The approval column's opening for a tolerance set from the spread
/// between sessions (separate boots and builds), which back-to-back noise
/// runs can't see: `perf noise` and `refloor` keep it as the least
/// tolerance, and a faster floor keeps the reason. Without it a re-measure
/// silently tightened `gfx_flush_vga` to 0.0035% again, inside a spread of
/// 0.09% between sessions (rulings-2026-10-29.md, R97).
pub const BETWEEN_SESSIONS: &str = "tolerance between sessions:";

/// `new` floors written over `old`: a number whose old row holds a
/// [`BETWEEN_SESSIONS`] tolerance keeps it (and its reason) as its least.
pub fn keep_session_tolerances(old: &BTreeMap<String, Floor>, new: &mut BTreeMap<String, Floor>) {
    for (name, f) in new.iter_mut() {
        if let Some(o) = old.get(name)
            && o.approved.starts_with(BETWEEN_SESSIONS)
        {
            f.tolerance = f.tolerance.max(o.tolerance);
            if f.approved.is_empty() {
                f.approved.clone_from(&o.approved);
            }
        }
    }
}

/// The floors file's rows now, if there is one.
fn old_floors(root: &Path) -> BTreeMap<String, Floor> {
    std::fs::read_to_string(root.join(FLOORS))
        .map(|t| parse_floors(&t))
        .unwrap_or_default()
}

/// A comparison's verdict for one number.
#[derive(Debug, PartialEq)]
pub enum Verdict {
    Slower(f64),
    Faster(f64),
    Within,
    New,
    Missing,
}

/// `value` against `floor`: beyond its tolerance either way, or within.
pub fn judge(value: Option<f64>, floor: Option<&Floor>) -> Verdict {
    match (value, floor) {
        (None, Some(_)) => Verdict::Missing,
        (Some(_), None) => Verdict::New,
        (None, None) => Verdict::Within,
        (Some(v), Some(f)) => {
            let rel = v / f.floor - 1.0;
            if rel > f.tolerance {
                Verdict::Slower(rel)
            } else if -rel > f.tolerance {
                Verdict::Faster(rel)
            } else {
                Verdict::Within
            }
        }
    }
}

/// The step a number moves in, from how it's measured: the probe's ns an
/// iteration come from the 3.25 MHz OS timer (4000/13 ns a tick; at most
/// that, over its iterations), the test images' microseconds from
/// `ticks / 13 * 4` (4 us steps). Five runs can all read the same value,
/// and then the data shows no step at all.
fn quantum(name: &str) -> f64 {
    if name.starts_with("probe.") {
        4000.0 / 13.0
    } else {
        4.0
    }
}

/// Floors from `runs` of every number: the median, and as the tolerance
/// the largest of four standard deviations of the runs, twice the smallest
/// step between their values (the timer's quantum as the data shows it),
/// and one unit, over the median. Not the runs' range: a sixth run falls
/// outside five runs' range a third of the time, so with ~100 numbers the
/// first check failed on seven within a tick (2026-10-22); at 4 standard
/// deviations a number fails by chance about once in 30,000 runs. And at
/// least two of its [`quantum`] steps.
pub fn floors_from_runs(runs: &[BTreeMap<String, f64>], set_by: &str) -> BTreeMap<String, Floor> {
    let mut all: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for r in runs {
        for (k, v) in r {
            all.entry(k.clone()).or_default().push(*v);
        }
    }
    let mut out = BTreeMap::new();
    for (k, mut vs) in all {
        if vs.len() < runs.len() || vs.is_empty() {
            continue; // not in every run: not steady enough to hold
        }
        vs.sort_by(f64::total_cmp);
        let med = vs[vs.len() / 2];
        if med <= 0.0 {
            continue;
        }
        let n = vs.len() as f64;
        let mean = vs.iter().sum::<f64>() / n;
        let sd = if vs.len() > 1 {
            (vs.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / (n - 1.0)).sqrt()
        } else {
            0.0
        };
        let step = vs
            .windows(2)
            .map(|w| w[1] - w[0])
            .filter(|d| *d > 0.0)
            .fold(f64::INFINITY, f64::min);
        let step = if step.is_finite() { step } else { 0.0 };
        let tol = (4.0 * sd).max(2.0 * step).max(2.0 * quantum(&k)) / med;
        out.insert(
            k,
            Floor {
                floor: med,
                tolerance: tol,
                set_by: set_by.to_string(),
                approved: String::new(),
            },
        );
    }
    out
}

/// Builds `step`'s image (or variant) as `target/perf/<image>.bin`, so its
/// card log keeps the image's name.
fn build(root: &Path, step: &Step) -> Result<PathBuf, String> {
    let dir = match step.env {
        Some((k, _)) => format!("target/variants/{}", k.to_ascii_lowercase()),
        None => "target".to_string(),
    };
    let mut cmd = std::process::Command::new("cargo");
    let dir = crate::frame_pointer_build(&mut cmd, step.image, &dir);
    crate::with_local_inputs(&mut cmd);
    cmd.current_dir(root);
    if let Some((k, v)) = step.env {
        cmd.env(k, v);
    }
    let status = cmd
        .args([
            "build",
            "--release",
            "--target",
            crate::TARGET_SPEC,
            "-Zbuild-std=core,compiler_builtins",
            "-Zjson-target-spec",
            "-q",
            "-p",
            step.image,
            "--target-dir",
            &dir,
        ])
        .status()
        .map_err(|e| format!("cargo: {e}"))?;
    if !status.success() {
        return Err(format!("building {} failed", step.image));
    }
    let elf = root
        .join(&dir)
        .join("armv5te-x51v-none-eabi/release")
        .join(step.image);
    let out_dir = root.join("target/perf");
    std::fs::create_dir_all(&out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;
    let bin = out_dir.join(format!("{}.bin", step.image));
    crate::sdkit::raw_binary(&elf, &bin)?;
    Ok(bin)
}

/// The newest saved card log of `image` in `local/cardlog`.
fn newest_cardlog(root: &Path, image: &str) -> Result<String, String> {
    let name = crate::usb::card_log_name(image);
    let dir = root.join("local/cardlog");
    let mut best: Option<(u32, PathBuf)> = None;
    for e in std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let p = e.map_err(|e| e.to_string())?.path();
        let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Some(n) = stem
            .strip_prefix(&format!("{name}-"))
            .and_then(|n| n.parse::<u32>().ok())
        else {
            continue;
        };
        if best.as_ref().is_none_or(|(b, _)| n > *b) {
            best = Some((n, p));
        }
    }
    let (_, p) = best.ok_or(format!("no saved card log of {name}"))?;
    std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))
}

/// One pass of the suite on the device: every number.
fn run_suite(root: &Path) -> Result<BTreeMap<String, f64>, String> {
    let mut all = BTreeMap::new();
    for step in SUITE {
        // A plain image goes by name: the workspace build `cargo xtask
        // load` sends (a single-package build lays the code out apart).
        let bin = match step.env {
            Some(_) => build(root, step)?.to_string_lossy().into_owned(),
            None => step.image.to_string(),
        };
        eprintln!(
            "    perf: {}{}",
            step.image,
            step.env.map(|(k, _)| format!(" ({k})")).unwrap_or_default()
        );
        crate::usb::load(root, &bin, false)?;
        crate::usb::cardlog(root, Some(step.image), None)?;
        let log = newest_cardlog(root, step.image)?;
        let nums = step_numbers(step, &log);
        if nums.is_empty() {
            return Err(format!("{}: no numbers in its run log", step.image));
        }
        all.extend(nums);
    }
    Ok(all)
}

/// Saves a run's numbers to `local/perf/<stamp>.txt`.
fn save_run(root: &Path, stamp: &str, nums: &BTreeMap<String, f64>) -> Result<(), String> {
    let dir = root.join("local/perf");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let text: String = nums.iter().map(|(k, v)| format!("{k} {v}\n")).collect();
    let p = dir.join(format!("{stamp}.txt"));
    std::fs::write(&p, text).map_err(|e| format!("{}: {e}", p.display()))
}

fn stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    format!("run-{secs}")
}

/// `cargo xtask perf noise N`: `n` runs, the floors written afresh.
pub fn noise(root: &Path, n: usize) -> Result<(), String> {
    if n < 3 {
        return Err("noise needs 3 runs at least (the spread of fewer says little)".into());
    }
    let mut runs = Vec::new();
    for k in 0..n {
        eprintln!("==> perf noise: run {} of {n}", k + 1);
        let nums = run_suite(root)?;
        save_run(root, &format!("{}-noise{}", stamp(), k + 1), &nums)?;
        runs.push(nums);
    }
    let mut floors = floors_from_runs(&runs, &format!("noise {n} runs, {}", stamp()));
    keep_session_tolerances(&old_floors(root), &mut floors);
    let p = root.join(FLOORS);
    std::fs::write(&p, floors_text(&floors)).map_err(|e| format!("{}: {e}", p.display()))?;
    eprintln!("    perf: {} floors written to {FLOORS}", floors.len());
    Ok(())
}

/// `cargo xtask perf refloor N`: the floors written afresh from the newest
/// `n` noise runs saved in `local/perf` (no device runs).
pub fn refloor(root: &Path, n: usize) -> Result<(), String> {
    let dir = root.join("local/perf");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.to_string_lossy().contains("-noise"))
        .collect();
    files.sort();
    if files.len() < n || n < 3 {
        return Err(format!(
            "refloor needs {n} (3 or more) saved noise runs; {} found",
            files.len()
        ));
    }
    let mut runs = Vec::new();
    for p in &files[files.len() - n..] {
        let text = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
        runs.push(
            text.lines()
                .filter_map(|l| l.split_once(' '))
                .filter_map(|(k, v)| v.parse::<f64>().ok().map(|v| (k.to_string(), v)))
                .collect::<BTreeMap<_, _>>(),
        );
    }
    let mut floors = floors_from_runs(&runs, &format!("noise {n} runs, refloored"));
    keep_session_tolerances(&old_floors(root), &mut floors);
    let p = root.join(FLOORS);
    std::fs::write(&p, floors_text(&floors)).map_err(|e| format!("{}: {e}", p.display()))?;
    eprintln!(
        "    perf: {} floors written to {FLOORS} from {n} saved runs",
        floors.len()
    );
    Ok(())
}

/// `cargo xtask perf [--update]`: one run against the floors.
pub fn check(root: &Path, update: bool) -> Result<(), String> {
    let p = root.join(FLOORS);
    let text = std::fs::read_to_string(&p).map_err(|e| {
        format!(
            "{}: {e} (run `cargo xtask perf noise 5` first)",
            p.display()
        )
    })?;
    let mut floors = parse_floors(&text);
    let nums = run_suite(root)?;
    let st = stamp();
    save_run(root, &st, &nums)?;
    let names: std::collections::BTreeSet<&String> = floors.keys().chain(nums.keys()).collect();
    let (mut slower, mut faster, mut missing, mut new) = (Vec::new(), Vec::new(), Vec::new(), 0);
    for name in names {
        match judge(nums.get(name).copied(), floors.get(name)) {
            Verdict::Slower(r) => slower.push(format!("{name} +{:.1}%", r * 100.0)),
            Verdict::Faster(r) => faster.push((name.clone(), r)),
            Verdict::Missing => missing.push(name.clone()),
            Verdict::New => new += 1,
            Verdict::Within => {}
        }
    }
    for (name, r) in &faster {
        eprintln!(
            "    perf: {name} {:.1}% faster: the floor ratchets",
            r * 100.0
        );
        if update && let (Some(f), Some(v)) = (floors.get_mut(name), nums.get(name)) {
            f.floor = *v;
            f.set_by = st.clone();
            if !f.approved.starts_with(BETWEEN_SESSIONS) {
                f.approved = String::new();
            }
        }
    }
    if update && !faster.is_empty() {
        std::fs::write(&p, floors_text(&floors)).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    eprintln!(
        "    perf: {} numbers, {} slower, {} faster{}, {} missing, {new} new (not held yet)",
        nums.len(),
        slower.len(),
        faster.len(),
        if update {
            " (floors ratcheted)"
        } else {
            " (`--update` writes them)"
        },
        missing.len()
    );
    if !missing.is_empty() {
        return Err(format!(
            "perf: numbers gone from the run: {}",
            missing.join(", ")
        ));
    }
    if !slower.is_empty() {
        return Err(format!(
            "perf: slower than the floor beyond its noise (fix, or the owner approves the new floor): {}",
            slower.join(", ")
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_name_each_number_by_head_and_part() {
        let mut m = BTreeMap::new();
        line_numbers(
            "B9e full screen (us each): 2x upscale from 240 x 320 32-bit Some(18688); 1:1 from 480 x 640 32-bit Some(9634); 8 bpp through the palette Some(63331); 16-bit copy Some(5033)",
            &mut m,
        );
        assert_eq!(m.len(), 4);
        assert_eq!(
            m["b9e_full_screen_us_each.2x_upscale_from_240_x_320_32_bit"],
            18688.0
        );
        assert_eq!(m["b9e_full_screen_us_each.16_bit_copy"], 5033.0);
        let mut m = BTreeMap::new();
        line_numbers(
            "FT alone: 3D Some(22055) us, 2D (10 copies) Some(71754) us",
            &mut m,
        );
        assert_eq!(m["ft_alone.3d"], 22055.0);
        assert_eq!(m["ft_alone.2d_10_copies"], 71754.0);
        let mut m = BTreeMap::new();
        line_numbers(
            "B7d convert 32 to 16 bits, full screen: Some(9634) us each",
            &mut m,
        );
        assert_eq!(m["b7d_convert_32_to_16_bits_full_screen"], 9634.0);
        // A part that didn't finish isn't a number.
        let mut m = BTreeMap::new();
        line_numbers("FT alone: 3D None us, 2D (10 copies) Some(1) us", &mut m);
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn the_touch_replay_line_gives_ten_numbers() {
        // rp-today's `TR` line, as its format string writes it.
        let mut m = BTreeMap::new();
        line_numbers(
            "TR touch replay: touch to flush median Some(15000), touch to flush worst Some(40000); \
             landing to verdict median Some(7000), landing to verdict worst Some(14000); \
             frame and queueing median Some(400), frame and queueing worst Some(25000); \
             queue to flip median Some(6600), queue to flip worst Some(17000); \
             pen-down period median Some(7000), pen-down period worst Some(9000) \
             (us; 56 answered, 28 presses, 520 periods)",
            &mut m,
        );
        assert_eq!(m.len(), 10, "{m:?}");
        assert_eq!(m["tr_touch_replay.touch_to_flush_median"], 15000.0);
        assert_eq!(m["tr_touch_replay.pen_down_period_worst"], 9000.0);
        // The recorder variant isn't a floor: its head differs.
        let step = SUITE
            .iter()
            .find(|s| s.image == "rp-today")
            .expect("in the suite");
        assert!(!"TR touch replay with the recorder: x".starts_with(step.heads[0]));
    }

    #[test]
    fn probe_rows_are_numbers_but_not_headings_or_checks() {
        let mut m = BTreeMap::new();
        probe_numbers(
            "P5|5 BENCH ns/iteration (more)           D8\nP5|copy_1m                      9630153  E3\nP6|check wmmx_copy OK                    BB\nnoise\n",
            &mut m,
        );
        assert_eq!(m.len(), 1);
        assert_eq!(m["probe.copy_1m"], 9_630_153.0);
    }

    #[test]
    fn a_tolerance_between_sessions_outlives_a_re_measure() {
        let row = |tol: f64, approved: &str| Floor {
            floor: 100.0,
            tolerance: tol,
            set_by: "s".into(),
            approved: approved.into(),
        };
        let reason = format!("{BETWEEN_SESSIONS} 4 sd of 5 sessions");
        let old = BTreeMap::from([
            ("held".to_string(), row(0.002, &reason)),
            ("plain".to_string(), row(0.002, "")),
            ("wider".to_string(), row(0.002, &reason)),
        ]);
        let mut new = BTreeMap::from([
            ("held".to_string(), row(0.0001, "")),
            ("plain".to_string(), row(0.0001, "")),
            ("wider".to_string(), row(0.01, "")),
        ]);
        keep_session_tolerances(&old, &mut new);
        assert_eq!(new["held"].tolerance, 0.002);
        assert_eq!(new["held"].approved, reason);
        // Only a reasoned row is kept; a wider fresh measure wins.
        assert_eq!(new["plain"].tolerance, 0.0001);
        assert_eq!(new["wider"].tolerance, 0.01);
        // And it survives the file.
        let back = parse_floors(&floors_text(&new));
        assert_eq!(back["held"].approved, reason);
        assert!((back["held"].tolerance - 0.002).abs() < 1e-9);
    }

    #[test]
    fn floors_round_trip_and_judge_both_ways() {
        let runs: Vec<BTreeMap<String, f64>> = [100.0, 102.0, 101.0]
            .iter()
            .map(|v| BTreeMap::from([("a".to_string(), *v), ("b".to_string(), 5.0)]))
            .collect();
        let f = floors_from_runs(&runs, "test");
        assert_eq!(f["a"].floor, 101.0);
        // Two 4 us steps (8.0) beat four standard deviations (4.0) here.
        assert!((f["a"].tolerance - 8.0 / 101.0).abs() < 1e-9);
        // No spread: two steps of the number's timer.
        assert!((f["b"].tolerance - 8.0 / 5.0).abs() < 1e-9);
        // A probe number's step is the OS timer's tick.
        let p = floors_from_runs(
            &[
                BTreeMap::from([("probe.x".to_string(), 1e6)]),
                BTreeMap::from([("probe.x".to_string(), 1e6)]),
                BTreeMap::from([("probe.x".to_string(), 1e6)]),
            ],
            "t",
        );
        assert!((p["probe.x"].tolerance - 2.0 * 4000.0 / 13.0 / 1e6).abs() < 1e-12);
        let back = parse_floors(&floors_text(&f));
        assert_eq!(back.len(), 2);
        assert_eq!(back["a"].floor, 101.0);
        assert!((back["a"].tolerance - f["a"].tolerance).abs() < 1e-6);
        assert_eq!(judge(Some(108.0), back.get("a")), Verdict::Within);
        assert!(matches!(
            judge(Some(110.0), back.get("a")),
            Verdict::Slower(_)
        ));
        assert!(matches!(
            judge(Some(90.0), back.get("a")),
            Verdict::Faster(_)
        ));
        assert_eq!(judge(None, back.get("a")), Verdict::Missing);
        assert_eq!(judge(Some(1.0), None), Verdict::New);
    }

    #[test]
    fn a_number_missing_from_a_run_isnt_held() {
        let runs = vec![
            BTreeMap::from([("a".to_string(), 1.0), ("b".to_string(), 2.0)]),
            BTreeMap::from([("a".to_string(), 1.0)]),
            BTreeMap::from([("a".to_string(), 1.0), ("b".to_string(), 2.0)]),
        ];
        let f = floors_from_runs(&runs, "t");
        assert!(f.contains_key("a") && !f.contains_key("b"));
    }
}
