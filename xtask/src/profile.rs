//! `cargo xtask measure --profile`: the X51v's screen measured for a colour
//! profile (docs/design/display-calibration.md, C6 step 2).
//!
//! Patches: an ArgyllCMS `targen` set (full spread, dark region emphasised:
//! this panel crushes its shadows) plus exact greys every 8 levels and each
//! channel every 16, with white read again every [`DRIFT_EVERY`] patches to
//! correct the backlight's drift. They go to the Axim as a program written
//! into a copy of `rp-screen-measure` (`rp_screen_measure::program`), at full
//! backlight unless asked otherwise, through the straight gamma table or a
//! table given with `--table`. The colorimeter reads through one long-running
//! `spotread` ([`crate::spot`]). Patches whose reading missed its window are
//! shown again in a second pass with longer windows (the image comes back to
//! the loader by itself, so no hands are needed).
//!
//! Results under `local/screen-measure/c6/`: the readings (`.csv`), and an
//! Argyll `.ti3` (XYZ scaled to white = 100, white's cd/m² recorded) for
//! `colprof`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use rp_screen_measure::program::{self, PROGRAM_BYTES, Step, StepLight, encode};

use crate::measure::{Xyz, find_spotread, home, instrument_port, read_once};
use crate::spot::Spot;

/// A white reading every this many patches, for drift.
pub const DRIFT_EVERY: usize = 40;
/// Before the first patch (the PC is ready before the image starts).
const LEAD_MS: u32 = 5_000;
/// Into a patch before the reading starts: the panel settles.
const SETTLE_MS: u32 = 700;

/// What to measure.
pub struct Options {
    /// The file names' start.
    pub name: String,
    /// Patches from this `.ti1` instead of a new profiling set.
    pub ti1: Option<PathBuf>,
    /// A 17-point table to show the patches through (one `0x00RRGGBB` a line).
    pub table: Option<PathBuf>,
    /// The backlight.
    pub light: StepLight,
    /// Only print the plan (no colorimeter, no Axim).
    pub dry: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Patch,
    Drift,
}

#[derive(Clone, Copy, Debug)]
struct Target {
    rgb: (u8, u8, u8),
    kind: Kind,
}

#[derive(Clone, Copy, Debug)]
struct Reading {
    xyz: Xyz,
    secs: f64,
    at_ms: u64,
}

/// The panel's rough luminance for `rgb` at full backlight (runs 2-3:
/// black 2.1, white 57 cd/m², gamma 1.8), to size each reading's window.
fn expected_y((r, g, b): (u8, u8, u8)) -> f64 {
    let c = |v: u8| (f64::from(v) / 255.0).powf(1.8);
    2.1 + 55.0 * (0.21 * c(r) + 0.72 * c(g) + 0.07 * c(b))
}

/// How long a reading may take at about `y` cd/m² (the Spyder4 integrates
/// longer in the dark).
fn window_ms(y: f64, stretch: u32) -> u32 {
    let base = if y >= 20.0 {
        3_500
    } else if y >= 8.0 {
        5_000
    } else if y >= 4.0 {
        8_000
    } else {
        14_000
    };
    base * stretch
}

fn steps(targets: &[Target], table: u8, light: StepLight, stretch: u32) -> Vec<Step> {
    targets
        .iter()
        .map(|t| Step {
            rgb: t.rgb,
            // White for drift always through the straight table: the
            // panel's own white, comparable across runs.
            table: if t.kind == Kind::Drift { 0 } else { table },
            light,
            ms: (SETTLE_MS + window_ms(expected_y(t.rgb), stretch)).div_ceil(10) * 10,
        })
        .collect()
}

/// Reads an Argyll `.ti1`/`.ti3`'s RGB patches (0-100) as 8-bit values.
fn read_ti(text: &str) -> Result<Vec<(u8, u8, u8)>, String> {
    let mut fields: Vec<&str> = Vec::new();
    let mut lines = text.lines();
    while let Some(l) = lines.next() {
        if l.trim() == "BEGIN_DATA_FORMAT" {
            for f in lines.by_ref() {
                if f.trim() == "END_DATA_FORMAT" {
                    break;
                }
                fields.extend(f.split_whitespace());
            }
        }
        if l.trim() == "BEGIN_DATA" {
            break;
        }
    }
    let col = |n: &str| {
        fields
            .iter()
            .position(|f| *f == n)
            .ok_or(format!("no {n} field"))
    };
    let (r, g, b) = (col("RGB_R")?, col("RGB_G")?, col("RGB_B")?);
    let mut out = Vec::new();
    for l in lines {
        if l.trim() == "END_DATA" {
            break;
        }
        let v: Vec<&str> = l.split_whitespace().collect();
        let get = |i: usize| -> Result<u8, String> {
            let x: f64 = v
                .get(i)
                .and_then(|s| s.parse().ok())
                .ok_or(format!("bad data line: {l}"))?;
            Ok((x.clamp(0.0, 100.0) * 255.0 / 100.0).round() as u8)
        };
        out.push((get(r)?, get(g)?, get(b)?));
    }
    if out.is_empty() {
        return Err("no patches".into());
    }
    Ok(out)
}

/// The profiling set: Argyll's spread plus exact greys and channel ramps.
fn profile_set(targen: &Path, dir: &Path, name: &str) -> Result<Vec<(u8, u8, u8)>, String> {
    let base = dir.join(name);
    let out = Command::new(targen)
        .args(["-v", "-d3", "-G", "-e2", "-B2", "-f150", "-V1.6"])
        .arg(&base)
        .output()
        .map_err(|e| format!("targen: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "targen failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let ti1 = base.with_extension("ti1");
    let text = std::fs::read_to_string(&ti1).map_err(|e| format!("{}: {e}", ti1.display()))?;
    let mut set = read_ti(&text)?;
    let mut extra = Vec::new();
    for g in (0..=256).step_by(8) {
        let g = g.min(255) as u8;
        extra.push((g, g, g));
    }
    for v in (16..=256).step_by(16) {
        let v = v.min(255) as u8;
        extra.extend([(v, 0, 0), (0, v, 0), (0, 0, v)]);
    }
    for e in extra {
        if !set.contains(&e) {
            set.push(e);
        }
    }
    Ok(set)
}

fn with_drift(patches: &[(u8, u8, u8)]) -> Vec<Target> {
    let white = Target {
        rgb: (255, 255, 255),
        kind: Kind::Drift,
    };
    let mut t = vec![white];
    for (i, &rgb) in patches.iter().enumerate() {
        if i > 0 && i % DRIFT_EVERY == 0 {
            t.push(white);
        }
        t.push(Target {
            rgb,
            kind: Kind::Patch,
        });
    }
    t.push(white);
    t
}

fn read_table(path: &Path) -> Result<[u32; 17], String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let words: Vec<u32> = text
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
        .map(|s| u32::from_str_radix(s.trim_start_matches("0x"), 16))
        .collect::<Result<_, _>>()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    words
        .try_into()
        .map_err(|w: Vec<u32>| format!("{}: {} words, want 17", path.display(), w.len()))
}

/// The built image with `steps` written into its program room.
fn image_with(
    root: &Path,
    steps: &[Step],
    tables: &[[u32; 17]],
    tag: &str,
) -> Result<PathBuf, String> {
    crate::cross()?;
    let out = root.join("target/load");
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let plain = out.join("rp-screen-measure.bin");
    crate::sdkit::raw_binary(&crate::arm_release(root).join("rp-screen-measure"), &plain)?;
    let mut bin = std::fs::read(&plain).map_err(|e| format!("{}: {e}", plain.display()))?;
    let at = find_room(&bin)?;
    encode(steps, tables, LEAD_MS, &mut bin[at..at + PROGRAM_BYTES])
        .map_err(|e| format!("program: {}", e.name()))?;
    let path = out.join(format!("rp-screen-measure-{tag}.bin"));
    std::fs::write(&path, &bin).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// Where the program room starts: the only empty room in the image.
pub(crate) fn find_room(bin: &[u8]) -> Result<usize, String> {
    let empty = program::placeholder();
    let hits: Vec<usize> = bin
        .windows(program::MAGIC.len())
        .enumerate()
        .filter(|(_, w)| *w == program::MAGIC)
        .map(|(i, _)| i)
        .filter(|&i| bin.get(i..i + PROGRAM_BYTES) == Some(&empty[..]))
        .collect();
    match hits[..] {
        [one] => Ok(one),
        [] => Err("no program room in rp-screen-measure".into()),
        _ => Err("more than one program room in rp-screen-measure".into()),
    }
}

/// One pass on the device; a reading (or none) per step.
fn pass(
    root: &Path,
    spot: &mut Meter,
    image: &Path,
    steps: &[Step],
    targets: &[Target],
) -> Result<Vec<Option<Reading>>, String> {
    let image = image.to_str().ok_or("image path isn't valid text")?;
    let mut t0 = None;
    for attempt in 0..10 {
        match crate::usb::start(root, image) {
            Ok(t) => {
                t0 = Some(t);
                break;
            }
            Err(e) if attempt < 9 => {
                eprintln!("    loader not ready ({e}); again in 3 s");
                std::thread::sleep(Duration::from_secs(3));
            }
            Err(e) => return Err(e),
        }
    }
    let t0 = t0.ok_or("no start")?;
    eprintln!("    the Axim started the patches");
    let mut out = Vec::with_capacity(steps.len());
    let mut start = LEAD_MS;
    for (i, (s, t)) in steps.iter().zip(targets).enumerate() {
        let read_at = t0 + Duration::from_millis(u64::from(start + SETTLE_MS));
        let ends = t0 + Duration::from_millis(u64::from(start + s.ms));
        start += s.ms;
        let now = Instant::now();
        if now > read_at {
            // Still busy with an earlier patch: no clean window left.
            if ends.saturating_duration_since(now) < Duration::from_millis(2_500) {
                eprintln!(
                    "    {:3}/{} {:?}: skipped (no time left)",
                    i + 1,
                    steps.len(),
                    t.rgb
                );
                out.push(None);
                continue;
            }
        } else {
            std::thread::sleep(read_at - now);
        }
        let began = Instant::now();
        let budget = ends.saturating_duration_since(began).as_millis() as u32;
        match spot.read(budget.saturating_sub(100)) {
            Ok(xyz) => {
                let secs = began.elapsed().as_secs_f64();
                eprintln!(
                    "    {:3}/{} {:?}{}: {:.3} cd/m2 ({secs:.1} s)",
                    i + 1,
                    steps.len(),
                    t.rgb,
                    if t.kind == Kind::Drift { " drift" } else { "" },
                    xyz.y
                );
                out.push(Some(Reading {
                    xyz,
                    secs,
                    at_ms: began.duration_since(t0).as_millis() as u64,
                }));
            }
            Err(e) => {
                eprintln!(
                    "    {:3}/{} {:?}: no reading ({e})",
                    i + 1,
                    steps.len(),
                    t.rgb
                );
                out.push(None);
                // A reading still running would answer the next key: let it
                // finish and throw it away.
                spot.drain();
            }
        }
    }
    Ok(out)
}

/// The white drift factor at `at_ms`: the white readings interpolated in
/// time, over their mean.
fn drift_at(whites: &[(u64, f64)], at_ms: u64) -> f64 {
    let mean = whites.iter().map(|w| w.1).sum::<f64>() / whites.len() as f64;
    let y = match whites.iter().position(|w| w.0 >= at_ms) {
        Some(0) => whites[0].1,
        None => whites[whites.len() - 1].1,
        Some(k) => {
            let (a, b) = (whites[k - 1], whites[k]);
            let f = (at_ms - a.0) as f64 / (b.0 - a.0).max(1) as f64;
            a.1 + (b.1 - a.1) * f
        }
    };
    y / mean
}

/// `cargo xtask measure --profile|--ti1 <file> [--table <file>] [--light found]`.
pub fn run(root: &Path, opts: &Options) -> Result<(), String> {
    let spotread = find_spotread()?;
    let bin = spotread.parent().ok_or("no Argyll bin folder")?;
    let ccss = home().join(".local/share/ArgyllCMS/WLEDFamily_07Feb11.ccss");
    let dir = root.join("local/screen-measure/c6");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let name = format!("{}-{stamp}", opts.name);
    let patches = match &opts.ti1 {
        Some(p) => {
            read_ti(&std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?)?
        }
        None => profile_set(&bin.join("targen.exe"), &dir, &name)?,
    };
    let tables: Vec<[u32; 17]> = opts
        .table
        .iter()
        .map(|p| read_table(p))
        .collect::<Result<_, _>>()?;
    let table = u8::from(!tables.is_empty());
    let targets = with_drift(&patches);
    let first = steps(&targets, table, opts.light, 1);
    let total: u32 = LEAD_MS + first.iter().map(|s| s.ms).sum::<u32>();
    eprintln!(
        "    {} patches + {} white drift readings, about {} minutes (more if some need a second pass); white-LED correction {}",
        patches.len(),
        targets.len() - patches.len(),
        total.div_ceil(60_000),
        if ccss.exists() { "on" } else { "off" }
    );
    if opts.dry {
        let mut room = program::placeholder();
        let used = encode(&first, &tables, LEAD_MS, &mut room)
            .map_err(|e| format!("program: {}", e.name()))?;
        eprintln!("    dry run: program {used} of {PROGRAM_BYTES} bytes; nothing sent");
        return Ok(());
    }
    let mut spot = open_spot()?;
    let readings = measure_targets(root, &mut spot, &targets, table, &tables, opts.light)?;
    drop(spot);
    write_results(&dir, &name, &targets, &readings, opts.light)
}

/// The colorimeter: one long-running spotread in a pseudo-console, or, if
/// that can't start, one spotread run per reading (about 10 s each, so
/// every window is made four times longer).
pub enum Meter {
    Pty(Spot),
    OneShot {
        spotread: PathBuf,
        port: u32,
        ccss: PathBuf,
    },
}

impl Meter {
    fn read(&mut self, ms: u32) -> Result<Xyz, String> {
        match self {
            Self::Pty(s) => s.read(ms),
            Self::OneShot {
                spotread,
                port,
                ccss,
            } => read_once(spotread, *port, ccss),
        }
    }

    /// After a reading that ran out of time: a pseudo-console reading may
    /// still be running; let it end and throw it away.
    fn drain(&mut self) {
        if let Self::Pty(s) = self {
            let _ = s.read(30_000);
        }
    }

    fn stretch(&self) -> u32 {
        match self {
            Self::Pty(_) => 1,
            Self::OneShot { .. } => 4,
        }
    }
}

/// The colorimeter, ready.
pub fn open_spot() -> Result<Meter, String> {
    let spotread = find_spotread()?;
    let port = instrument_port(&spotread)?;
    let ccss = home().join(".local/share/ArgyllCMS/WLEDFamily_07Feb11.ccss");
    eprintln!(
        "    colorimeter: Argyll port {port}, white-LED correction {}",
        if ccss.exists() { "on" } else { "off" }
    );
    match Spot::open(&spotread, port, &ccss) {
        Ok(s) => {
            eprintln!("    colorimeter ready (one spotread kept open)");
            Ok(Meter::Pty(s))
        }
        Err(e) => {
            eprintln!(
                "    the pseudo-console spotread didn't start ({e}): one spotread per reading instead, windows 4x longer"
            );
            Ok(Meter::OneShot {
                spotread,
                port,
                ccss,
            })
        }
    }
}

/// Shows `targets` (patches through `table`, drift whites through the
/// straight one) and reads each; the ones that missed get a second pass
/// with three times the window.
fn measure_targets(
    root: &Path,
    spot: &mut Meter,
    targets: &[Target],
    table: u8,
    tables: &[[u32; 17]],
    light: StepLight,
) -> Result<Vec<Option<Reading>>, String> {
    let first = steps(targets, table, light, spot.stretch());
    let image = image_with(root, &first, tables, "pass1")?;
    let mut readings = pass(root, spot, &image, &first, targets)?;
    let missed: Vec<usize> = (0..targets.len())
        .filter(|&i| readings[i].is_none())
        .collect();
    if !missed.is_empty() {
        let again: Vec<Target> = missed.iter().map(|&i| targets[i]).collect();
        let more = steps(&again, table, light, 3 * spot.stretch());
        eprintln!("    second pass: {} patches", again.len());
        // The image is back in the loader once its last patch ends.
        std::thread::sleep(Duration::from_secs(4));
        let image = image_with(root, &more, tables, "pass2")?;
        let r2 = pass(root, spot, &image, &more, &again)?;
        // Second-pass times count from after the first pass.
        let offset = readings
            .iter()
            .flatten()
            .map(|r| r.at_ms)
            .max()
            .unwrap_or(0)
            + 10_000;
        for (&i, r) in missed.iter().zip(r2) {
            readings[i] = r.map(|r| Reading {
                at_ms: r.at_ms + offset,
                ..r
            });
        }
    }
    Ok(readings)
}

/// For calibration passes: `patches` shown through `table` (with drift
/// whites around them), each reading drift-corrected; also the panel's own
/// white over the run (the drift readings' mean).
pub fn measure_corrected(
    root: &Path,
    spot: &mut Meter,
    patches: &[(u8, u8, u8)],
    table: [u32; 17],
    light: StepLight,
) -> Result<(Vec<Option<Xyz>>, Xyz), String> {
    let targets = with_drift(patches);
    let readings = measure_targets(root, spot, &targets, 1, &[table], light)?;
    let mut whites: Vec<(u64, f64)> = Vec::new();
    let mut sum = [0.0; 3];
    for (t, r) in targets.iter().zip(&readings) {
        if let (Kind::Drift, Some(r)) = (t.kind, r) {
            whites.push((r.at_ms, r.xyz.y));
            sum = [sum[0] + r.xyz.x, sum[1] + r.xyz.y, sum[2] + r.xyz.z];
        }
    }
    if whites.is_empty() {
        return Err("no white reading in the pass".into());
    }
    whites.sort_by_key(|w| w.0);
    let n = whites.len() as f64;
    let white = Xyz {
        x: sum[0] / n,
        y: sum[1] / n,
        z: sum[2] / n,
    };
    let out = targets
        .iter()
        .zip(&readings)
        .filter(|(t, _)| t.kind == Kind::Patch)
        .map(|(_, r)| {
            r.map(|r| {
                let f = drift_at(&whites, r.at_ms);
                Xyz {
                    x: r.xyz.x / f,
                    y: r.xyz.y / f,
                    z: r.xyz.z / f,
                }
            })
        })
        .collect();
    Ok((out, white))
}

fn write_results(
    dir: &Path,
    name: &str,
    targets: &[Target],
    readings: &[Option<Reading>],
    light: StepLight,
) -> Result<(), String> {
    let mut csv = String::from("index,kind,R,G,B,X,Y,Z,seconds,at_ms\n");
    for (i, (t, r)) in targets.iter().zip(readings).enumerate() {
        let (r8, g8, b8) = t.rgb;
        let kind = if t.kind == Kind::Drift {
            "drift"
        } else {
            "patch"
        };
        match r {
            Some(r) => {
                let _ = writeln!(
                    csv,
                    "{i},{kind},{r8},{g8},{b8},{:.5},{:.5},{:.5},{:.2},{}",
                    r.xyz.x, r.xyz.y, r.xyz.z, r.secs, r.at_ms
                );
            }
            None => {
                let _ = writeln!(csv, "{i},{kind},{r8},{g8},{b8},,,,,");
            }
        }
    }
    let csv_path = dir.join(format!("{name}.csv"));
    std::fs::write(&csv_path, csv).map_err(|e| format!("{}: {e}", csv_path.display()))?;

    let mut whites: Vec<(u64, f64)> = targets
        .iter()
        .zip(readings)
        .filter(|(t, _)| t.kind == Kind::Drift)
        .filter_map(|(_, r)| r.map(|r| (r.at_ms, r.xyz.y)))
        .collect();
    whites.sort_by_key(|w| w.0);
    if whites.is_empty() {
        return Err(format!(
            "no white reading: no profile data (readings in {})",
            csv_path.display()
        ));
    }
    let span = whites.iter().map(|w| w.1).fold(f64::MIN, f64::max)
        / whites.iter().map(|w| w.1).fold(f64::MAX, f64::min);
    // Corrected readings of the patches.
    let data: Vec<((u8, u8, u8), Xyz)> = targets
        .iter()
        .zip(readings)
        .filter(|(t, _)| t.kind == Kind::Patch)
        .filter_map(|(t, r)| {
            r.map(|r| {
                let f = drift_at(&whites, r.at_ms);
                (
                    t.rgb,
                    Xyz {
                        x: r.xyz.x / f,
                        y: r.xyz.y / f,
                        z: r.xyz.z / f,
                    },
                )
            })
        })
        .collect();
    // White: the drift readings' mean (each patch is scaled to it).
    let drift: Vec<Xyz> = targets
        .iter()
        .zip(readings)
        .filter(|(t, _)| t.kind == Kind::Drift)
        .filter_map(|(_, r)| r.map(|r| r.xyz))
        .collect();
    let n = drift.len() as f64;
    let white = Xyz {
        x: drift.iter().map(|v| v.x).sum::<f64>() / n,
        y: drift.iter().map(|v| v.y).sum::<f64>() / n,
        z: drift.iter().map(|v| v.z).sum::<f64>() / n,
    };
    let mut ti3 = String::new();
    let _ = writeln!(ti3, "CTI3\n");
    let _ = writeln!(
        ti3,
        "DESCRIPTOR \"RetroPane X51v screen readings ({name})\""
    );
    let _ = writeln!(ti3, "ORIGINATOR \"RetroPane cargo xtask measure\"");
    let _ = writeln!(ti3, "KEYWORD \"DEVICE_CLASS\"\nDEVICE_CLASS \"DISPLAY\"");
    let _ = writeln!(ti3, "KEYWORD \"COLOR_REP\"\nCOLOR_REP \"RGB_XYZ\"");
    let _ = writeln!(
        ti3,
        "KEYWORD \"LUMINANCE_XYZ_CDM2\"\nLUMINANCE_XYZ_CDM2 \"{:.6} {:.6} {:.6}\"",
        white.x, white.y, white.z
    );
    let _ = writeln!(
        ti3,
        "KEYWORD \"NORMALIZED_TO_Y_100\"\nNORMALIZED_TO_Y_100 \"YES\""
    );
    let _ = writeln!(
        ti3,
        "KEYWORD \"BACKLIGHT\"\nBACKLIGHT \"{}\"",
        match light {
            StepLight::Found => "as found".to_string(),
            StepLight::Counts(c) => format!("{c} of 432"),
        }
    );
    let _ = writeln!(
        ti3,
        "\nNUMBER_OF_FIELDS 7\nBEGIN_DATA_FORMAT\nSAMPLE_ID RGB_R RGB_G RGB_B XYZ_X XYZ_Y XYZ_Z\nEND_DATA_FORMAT\n"
    );
    let _ = writeln!(ti3, "NUMBER_OF_SETS {}\nBEGIN_DATA", data.len());
    let s = 100.0 / white.y;
    for (i, ((r, g, b), v)) in data.iter().enumerate() {
        let pc = |c: &u8| f64::from(*c) * 100.0 / 255.0;
        let _ = writeln!(
            ti3,
            "{} {:.4} {:.4} {:.4} {:.5} {:.5} {:.5}",
            i + 1,
            pc(r),
            pc(g),
            pc(b),
            v.x * s,
            v.y * s,
            v.z * s
        );
    }
    let _ = writeln!(ti3, "END_DATA");
    let ti3_path = dir.join(format!("{name}.ti3"));
    std::fs::write(&ti3_path, ti3).map_err(|e| format!("{}: {e}", ti3_path.display()))?;
    let read = readings.iter().flatten().count();
    let black = data.iter().find(|(c, _)| *c == (0, 0, 0)).map(|d| d.1.y);
    eprintln!(
        "    {read} of {} read; white {:.1} cd/m2, black {}, drift over the run {:.1}%",
        targets.len(),
        white.y,
        black.map_or("not read".into(), |b| format!(
            "{b:.2} cd/m2 ({:.0}:1)",
            white.y / b
        )),
        (span - 1.0) * 100.0
    );
    eprintln!(
        "    saved {} and {}",
        csv_path.display(),
        ti3_path.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ti_files_parse_to_8_bit_patches() {
        let t = "CTI1\nNUMBER_OF_FIELDS 4\nBEGIN_DATA_FORMAT\nSAMPLE_ID RGB_R RGB_G RGB_B\nEND_DATA_FORMAT\nNUMBER_OF_SETS 2\nBEGIN_DATA\n1 100.0 50.0 0.0\n2 2.941176 0 0\nEND_DATA\n";
        assert_eq!(read_ti(t).unwrap(), vec![(255, 128, 0), (7, 0, 0)]);
        assert!(read_ti("CTI1\n").is_err());
    }

    #[test]
    fn drift_whites_wrap_the_patches() {
        let p: Vec<(u8, u8, u8)> = (0..85).map(|i| (i as u8, 0, 0)).collect();
        let t = with_drift(&p);
        let drift = t.iter().filter(|t| t.kind == Kind::Drift).count();
        assert_eq!(drift, 4, "start, after 40, after 80, end");
        assert_eq!(t.first().unwrap().kind, Kind::Drift);
        assert_eq!(t.last().unwrap().kind, Kind::Drift);
        assert_eq!(t.len(), 85 + 4);
    }

    #[test]
    fn drift_interpolates_between_whites() {
        let w = [(0, 40.0), (1000, 42.0)];
        assert!((drift_at(&w, 500) - 1.0).abs() < 1e-9);
        assert!((drift_at(&w, 0) - 40.0 / 41.0).abs() < 1e-9);
        assert!((drift_at(&w, 5000) - 42.0 / 41.0).abs() < 1e-9);
    }

    #[test]
    fn darker_patches_get_longer_windows_and_fit_the_format() {
        let dark = steps(&with_drift(&[(0, 0, 0)]), 0, StepLight::Counts(432), 3);
        assert!(dark[1].ms > dark[0].ms);
        let mut room = program::placeholder();
        encode(&dark, &[], LEAD_MS, &mut room).unwrap();
    }

    #[test]
    fn tables_read_as_seventeen_words() {
        let d = std::env::temp_dir().join("rp-table-test.txt");
        let words: Vec<String> = (0..17)
            .map(|i| format!("0x00{:02X}{:02X}{:02X}", i, i, i))
            .collect();
        std::fs::write(&d, words.join("\n")).unwrap();
        assert_eq!(read_table(&d).unwrap()[16], 0x0010_1010);
        std::fs::write(&d, "0x0 0x1").unwrap();
        assert!(read_table(&d).is_err());
    }
}
