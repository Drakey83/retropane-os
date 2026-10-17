//! C6 step 4: the 2700G's 17-point table calibrated from the panel's readings
//! (docs/design/display-calibration.md).
//!
//! The panel is modelled from its profiling readings (`cargo xtask measure
//! --profile`): its black, plus each channel's own light above black at 17
//! levels, added together (an LCD's channels add nearly exactly; the model's
//! error on the greys and the spread patches is reported). The table makes
//! greys from black to white neutral at a chosen white and follows BT.1886
//! between the panel's own black and that white (the curve made for screens
//! whose black is raised: shadow detail without lifting everything). Greys
//! measured through the table then correct it ([`refine`]).

use std::fmt::Write as _;
use std::path::Path;

/// XYZ in cd/m².
pub type Xyz = [f64; 3];

/// The levels the channel curves are known at: the table's points.
pub const LEVELS: [u8; 17] = [
    0, 16, 32, 48, 64, 80, 96, 112, 128, 144, 160, 176, 192, 208, 224, 240, 255,
];

/// One reading: what was shown and what was read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub rgb: (u8, u8, u8),
    pub xyz: Xyz,
}

/// Reads a `.ti3` written by `cargo xtask measure` back into absolute XYZ.
pub fn read_ti3(text: &str) -> Result<Vec<Sample>, String> {
    let white_y = text
        .lines()
        .find_map(|l| l.strip_prefix("LUMINANCE_XYZ_CDM2 "))
        .and_then(|v| {
            v.trim()
                .trim_matches('"')
                .split_whitespace()
                .nth(1)?
                .parse::<f64>()
                .ok()
        })
        .ok_or("no LUMINANCE_XYZ_CDM2")?;
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
    let idx = [
        col("RGB_R")?,
        col("RGB_G")?,
        col("RGB_B")?,
        col("XYZ_X")?,
        col("XYZ_Y")?,
        col("XYZ_Z")?,
    ];
    let mut out = Vec::new();
    for l in lines {
        if l.trim() == "END_DATA" {
            break;
        }
        let v: Vec<f64> = l
            .split_whitespace()
            .filter_map(|s| s.parse().ok())
            .collect();
        let n = l.split_whitespace().count();
        if v.len() != n {
            return Err(format!("bad data line: {l}"));
        }
        let get = |i: usize| v.get(idx[i]).copied().ok_or(format!("short line: {l}"));
        let c = |i: usize| -> Result<u8, String> {
            Ok((get(i)? * 255.0 / 100.0).round().clamp(0.0, 255.0) as u8)
        };
        let s = white_y / 100.0;
        out.push(Sample {
            rgb: (c(0)?, c(1)?, c(2)?),
            xyz: [get(3)? * s, get(4)? * s, get(5)? * s],
        });
    }
    if out.is_empty() {
        return Err("no readings".into());
    }
    Ok(out)
}

/// The panel: black plus each channel's light above black.
#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    pub black: Xyz,
    /// `[channel][level]`: light above black at [`LEVELS`].
    pub ch: [[Xyz; 17]; 3],
}

fn mean(v: &[Xyz]) -> Option<Xyz> {
    if v.is_empty() {
        return None;
    }
    let n = v.len() as f64;
    Some(core::array::from_fn(|k| {
        v.iter().map(|x| x[k]).sum::<f64>() / n
    }))
}

fn sub(a: Xyz, b: Xyz) -> Xyz {
    core::array::from_fn(|k| a[k] - b[k])
}

fn add(a: Xyz, b: Xyz) -> Xyz {
    core::array::from_fn(|k| a[k] + b[k])
}

fn scale(a: Xyz, s: f64) -> Xyz {
    a.map(|v| v * s)
}

impl Model {
    /// From readings that hold black and every channel alone at [`LEVELS`].
    pub fn from_samples(s: &[Sample]) -> Result<Self, String> {
        let at = |rgb: (u8, u8, u8)| -> Vec<Xyz> {
            s.iter().filter(|x| x.rgb == rgb).map(|x| x.xyz).collect()
        };
        let black = mean(&at((0, 0, 0))).ok_or("no black reading")?;
        let mut ch = [[[0.0; 3]; 17]; 3];
        for (c, curve) in ch.iter_mut().enumerate() {
            for (i, &v) in LEVELS.iter().enumerate().skip(1) {
                let rgb = match c {
                    0 => (v, 0, 0),
                    1 => (0, v, 0),
                    _ => (0, 0, v),
                };
                let m = mean(&at(rgb)).ok_or(format!("no reading of {rgb:?}"))?;
                curve[i] = sub(m, black);
            }
            // Light only grows with the level: smooth out sensor noise
            // (keeps the solve monotonic).
            for i in 1..17 {
                if curve[i][1] < curve[i - 1][1] {
                    curve[i] = curve[i - 1];
                }
            }
        }
        Ok(Self { black, ch })
    }

    fn channel(&self, c: usize, v: f64) -> Xyz {
        let v = v.clamp(0.0, 255.0);
        let i = LEVELS
            .iter()
            .rposition(|&l| f64::from(l) <= v)
            .unwrap_or(0)
            .min(15);
        let (a, b) = (f64::from(LEVELS[i]), f64::from(LEVELS[i + 1]));
        let f = (v - a) / (b - a);
        core::array::from_fn(|k| self.ch[c][i][k] + (self.ch[c][i + 1][k] - self.ch[c][i][k]) * f)
    }

    /// The predicted reading for panel input `rgb` (0-255, fractional).
    pub fn predict(&self, rgb: [f64; 3]) -> Xyz {
        (0..3).fold(self.black, |acc, c| add(acc, self.channel(c, rgb[c])))
    }

    /// The panel input that shows `target` (closest within 0-255).
    pub fn solve(&self, target: Xyz, start: [f64; 3]) -> [f64; 3] {
        let mut x = start;
        for _ in 0..60 {
            let e = sub(target, self.predict(x));
            if e.iter().all(|v| v.abs() < 1e-6) {
                break;
            }
            let j = self.jacobian(x);
            let Some(d) = solve3(j, e) else { break };
            let mut nx = [0.0; 3];
            for c in 0..3 {
                nx[c] = (x[c] + d[c]).clamp(0.0, 255.0);
            }
            if (0..3).all(|c| (nx[c] - x[c]).abs() < 1e-7) {
                break;
            }
            x = nx;
        }
        x
    }

    fn jacobian(&self, x: [f64; 3]) -> [[f64; 3]; 3] {
        let mut j = [[0.0; 3]; 3];
        for c in 0..3 {
            // A one-sided step that stays inside 0-255.
            let h = if x[c] > 254.0 { -1.0 } else { 1.0 };
            let mut y = x;
            y[c] += h;
            let d = sub(self.channel(c, y[c]), self.channel(c, x[c]));
            for k in 0..3 {
                j[k][c] = d[k] / h;
            }
        }
        j
    }
}

/// Solves `m x = v` (Cramer).
pub fn solve3(m: [[f64; 3]; 3], v: Xyz) -> Option<[f64; 3]> {
    let det = |a: [[f64; 3]; 3]| {
        a[0][0] * (a[1][1] * a[2][2] - a[1][2] * a[2][1])
            - a[0][1] * (a[1][0] * a[2][2] - a[1][2] * a[2][0])
            + a[0][2] * (a[1][0] * a[2][1] - a[1][1] * a[2][0])
    };
    let d = det(m);
    if d.abs() < 1e-12 {
        return None;
    }
    Some(core::array::from_fn(|c| {
        let mut a = m;
        for (r, row) in a.iter_mut().enumerate() {
            row[c] = v[r];
        }
        det(a) / d
    }))
}

/// Chromaticity.
pub fn xy(v: Xyz) -> (f64, f64) {
    let s = v[0] + v[1] + v[2];
    if s <= 0.0 {
        (0.0, 0.0)
    } else {
        (v[0] / s, v[1] / s)
    }
}

/// The white point a calibration aims at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum White {
    /// The panel's own (no channel turned down).
    Native,
    /// A daylight white at this colour temperature (K).
    Daylight(f64),
}

impl White {
    pub fn name(self) -> String {
        match self {
            Self::Native => "native".into(),
            Self::Daylight(k) => format!("{k:.0}K"),
        }
    }
}

/// CIE daylight chromaticity at `t` kelvin (4,000-25,000 K).
pub fn daylight_xy(t: f64) -> (f64, f64) {
    let x = if t <= 7000.0 {
        -4.6070e9 / t.powi(3) + 2.9678e6 / t.powi(2) + 0.09911e3 / t + 0.244063
    } else {
        -2.0064e9 / t.powi(3) + 1.9018e6 / t.powi(2) + 0.24748e3 / t + 0.237040
    };
    (x, -3.0 * x * x + 2.870 * x - 0.275)
}

/// The brightest white of chromaticity `w` the panel can show.
pub fn target_white(m: &Model, w: White) -> Xyz {
    let full = m.predict([255.0; 3]);
    let White::Daylight(t) = w else { return full };
    let (x, y) = daylight_xy(t);
    let d = [x / y, 1.0, (1.0 - x - y) / y];
    // White = black + sum of k_c x channel at full, k_c <= 1: the largest
    // multiple s of d reachable.
    let p: [[f64; 3]; 3] = core::array::from_fn(|r| core::array::from_fn(|c| m.ch[c][16][r]));
    let (Some(a), Some(b)) = (solve3(p, d), solve3(p, m.black)) else {
        return full;
    };
    let s = (0..3)
        .filter(|&c| a[c] > 0.0)
        .map(|c| (1.0 + b[c]) / a[c])
        .fold(f64::INFINITY, f64::min);
    if s.is_finite() { scale(d, s) } else { full }
}

/// BT.1886: luminance for signal `v` (0-1) between `lb` and `lw`.
pub fn bt1886(v: f64, lb: f64, lw: f64) -> f64 {
    let g = 2.4;
    let (wr, br) = (lw.powf(1.0 / g), lb.max(0.0).powf(1.0 / g));
    let a = (wr - br).powf(g);
    let b = br / (wr - br);
    a * (v + b).max(0.0).powf(g)
}

/// The reading a grey of input `v` (0-255) should give: the panel's black
/// plus the BT.1886 share of the light from black to the target white.
pub fn grey_target(m: &Model, white: Xyz, v: f64) -> Xyz {
    let lb = m.black[1];
    let lw = white[1];
    let f = ((bt1886(v / 255.0, lb, lw) - lb) / (lw - lb)).clamp(0.0, 1.0);
    add(m.black, scale(sub(white, m.black), f))
}

/// A table as the 2700G holds it: per point, 8 bits a colour.
pub type Table = [[f64; 3]; 17];

/// The table that puts the panel on the targets (continuous values; the
/// words round them).
pub fn table(m: &Model, w: White) -> Table {
    let white = target_white(m, w);
    let mut t = [[0.0; 3]; 17];
    let mut prev = [0.0; 3];
    for (i, row) in t.iter_mut().enumerate().take(16) {
        let v = f64::from(16 * i as u32);
        *row = m.solve(grey_target(m, white, v), prev);
        prev = *row;
    }
    // Point 16 stands for input 256: aim it so input 255 (15/16 of the way
    // from point 15) lands on the target white.
    let top = m.solve(white, prev);
    for c in 0..3 {
        t[16][c] = (t[15][c] + (top[c] - t[15][c]) * 16.0 / 15.0).clamp(0.0, 255.0);
    }
    t
}

/// The 2700G's words for a table.
pub fn words(t: &Table) -> [u32; 17] {
    core::array::from_fn(|i| {
        let q = |c: usize| t[i][c].round().clamp(0.0, 255.0) as u32;
        q(0) << 16 | q(1) << 8 | q(2)
    })
}

/// What the table does to input `v` (0-255), assuming the 2700G interpolates
/// linearly between points 16 apart (the emulator's rule; checked by the
/// greys measured between points).
pub fn through(w: &[u32; 17], v: u8) -> [f64; 3] {
    let i = usize::from(v / 16);
    let f = f64::from(v % 16) / 16.0;
    core::array::from_fn(|c| {
        let at = |k: usize| f64::from((w[k] >> (16 - 8 * c)) & 0xFF);
        at(i) + (at(i + 1) - at(i)) * f
    })
}

/// Corrects a table from greys measured through it: each point's error
/// (target minus reading) moves that point by the model's local slope.
/// `readings`: (input level, reading); levels not on a point are skipped.
pub fn refine(m: &Model, w: White, t: &Table, readings: &[(u8, Xyz)]) -> Table {
    let white = target_white(m, w);
    let mut out = *t;
    for &(v, xyz) in readings {
        let i = match v {
            255 => 16,
            v if v % 16 == 0 => usize::from(v / 16),
            _ => continue,
        };
        let target = if v == 255 {
            white
        } else {
            grey_target(m, white, f64::from(v))
        };
        let at = if v == 255 {
            core::array::from_fn(|c| t[15][c] + (t[16][c] - t[15][c]) * 15.0 / 16.0)
        } else {
            t[i]
        };
        let Some(d) = solve3(m.jacobian(at), sub(target, xyz)) else {
            continue;
        };
        let k = if v == 255 { 16.0 / 15.0 } else { 1.0 };
        for c in 0..3 {
            out[i][c] = (t[i][c] + d[c] * k).clamp(0.0, 255.0);
        }
    }
    out
}

/// CIE Lab of `v` against white `w`.
pub fn lab(v: Xyz, w: Xyz) -> [f64; 3] {
    let f = |t: f64| {
        if t > (6.0f64 / 29.0).powi(3) {
            t.cbrt()
        } else {
            t / (3.0 * (6.0f64 / 29.0).powi(2)) + 4.0 / 29.0
        }
    };
    let (fx, fy, fz) = (f(v[0] / w[0]), f(v[1] / w[1]), f(v[2] / w[2]));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// CIEDE2000 colour difference.
pub fn de2000(a: [f64; 3], b: [f64; 3]) -> f64 {
    use std::f64::consts::PI;
    let (l1, a1, b1) = (a[0], a[1], a[2]);
    let (l2, a2, b2) = (b[0], b[1], b[2]);
    let c1 = a1.hypot(b1);
    let c2 = a2.hypot(b2);
    let cm = (c1 + c2) / 2.0;
    let g = 0.5 * (1.0 - (cm.powi(7) / (cm.powi(7) + 25f64.powi(7))).sqrt());
    let (a1p, a2p) = (a1 * (1.0 + g), a2 * (1.0 + g));
    let (c1p, c2p) = (a1p.hypot(b1), a2p.hypot(b2));
    let hue = |b: f64, a: f64| {
        if a == 0.0 && b == 0.0 {
            0.0
        } else {
            let h = b.atan2(a).to_degrees();
            if h < 0.0 { h + 360.0 } else { h }
        }
    };
    let (h1, h2) = (hue(b1, a1p), hue(b2, a2p));
    let dl = l2 - l1;
    let dc = c2p - c1p;
    let dh = if c1p * c2p == 0.0 {
        0.0
    } else if (h2 - h1).abs() <= 180.0 {
        h2 - h1
    } else if h2 - h1 > 180.0 {
        h2 - h1 - 360.0
    } else {
        h2 - h1 + 360.0
    };
    let dhh = 2.0 * (c1p * c2p).sqrt() * (dh.to_radians() / 2.0).sin();
    let lm = (l1 + l2) / 2.0;
    let cmp = (c1p + c2p) / 2.0;
    let hm = if c1p * c2p == 0.0 {
        h1 + h2
    } else if (h1 - h2).abs() <= 180.0 {
        (h1 + h2) / 2.0
    } else if h1 + h2 < 360.0 {
        (h1 + h2 + 360.0) / 2.0
    } else {
        (h1 + h2 - 360.0) / 2.0
    };
    let t = 1.0 - 0.17 * (hm - 30.0).to_radians().cos()
        + 0.24 * (2.0 * hm).to_radians().cos()
        + 0.32 * (3.0 * hm + 6.0).to_radians().cos()
        - 0.20 * (4.0 * hm - 63.0).to_radians().cos();
    let dtheta = 30.0 * (-((hm - 275.0) / 25.0).powi(2)).exp();
    let rc = 2.0 * (cmp.powi(7) / (cmp.powi(7) + 25f64.powi(7))).sqrt();
    let sl = 1.0 + 0.015 * (lm - 50.0).powi(2) / (20.0 + (lm - 50.0).powi(2)).sqrt();
    let sc = 1.0 + 0.045 * cmp;
    let sh = 1.0 + 0.015 * cmp * t;
    let rt = -(2.0 * dtheta * PI / 180.0).sin() * rc;
    ((dl / sl).powi(2) + (dc / sc).powi(2) + (dhh / sh).powi(2) + rt * (dc / sc) * (dhh / sh))
        .sqrt()
}

/// McCamy's correlated colour temperature.
pub fn cct(v: Xyz) -> f64 {
    let (x, y) = xy(v);
    let n = (x - 0.3320) / (0.1858 - y);
    449.0 * n.powi(3) + 3525.0 * n.powi(2) + 6823.3 * n + 5520.33
}

/// How well the additive model predicts readings it wasn't built from:
/// (mean, worst) Delta E 2000 against the panel's white.
pub fn model_error(m: &Model, s: &[Sample]) -> (f64, f64) {
    let white = m.predict([255.0; 3]);
    let errs: Vec<f64> = s
        .iter()
        .filter(|x| {
            let (r, g, b) = x.rgb;
            [r, g, b].iter().filter(|&&v| v > 0).count() >= 2
        })
        .map(|x| {
            let p = m.predict([f64::from(x.rgb.0), f64::from(x.rgb.1), f64::from(x.rgb.2)]);
            de2000(lab(x.xyz, white), lab(p, white))
        })
        .collect();
    if errs.is_empty() {
        return (0.0, 0.0);
    }
    (
        errs.iter().sum::<f64>() / errs.len() as f64,
        errs.iter().copied().fold(0.0, f64::max),
    )
}

/// `cargo xtask calibrate <profile.ti3>`: tables for the white candidates,
/// written next to the readings, with what each costs and how well the
/// model predicts.
pub fn run(ti3: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(ti3).map_err(|e| format!("{}: {e}", ti3.display()))?;
    let samples = read_ti3(&text)?;
    let m = Model::from_samples(&samples)?;
    let native = m.predict([255.0; 3]);
    let (mean_e, worst_e) = model_error(&m, &samples);
    let mut report = String::new();
    let _ = writeln!(
        report,
        "Panel: white {:.1} cd/m2 at {:.0} K (x {:.4} y {:.4}), black {:.2} cd/m2, contrast {:.0}:1",
        native[1],
        cct(native),
        xy(native).0,
        xy(native).1,
        m.black[1],
        native[1] / m.black[1]
    );
    let _ = writeln!(
        report,
        "Additive model on the mixed patches: mean Delta E 2000 {mean_e:.2}, worst {worst_e:.2}"
    );
    for w in [
        White::Native,
        White::Daylight(9000.0),
        White::Daylight(7500.0),
        White::Daylight(6504.0),
    ] {
        let t = table(&m, w);
        let wd = words(&t);
        let white = target_white(&m, w);
        // Predicted greys through the table's words (rounded), as shown.
        let mut worst = 0.0f64;
        for v in (0..=255)
            .step_by(8)
            .map(|v: u32| v.min(255) as u8)
            .chain([255])
        {
            let p = m.predict(through(&wd, v));
            let e = de2000(
                lab(p, white),
                lab(grey_target(&m, white, f64::from(v)), white),
            );
            worst = worst.max(e);
        }
        let _ = writeln!(
            report,
            "{}: white {:.1} cd/m2 ({:.0}% of the panel's), {:.0} K; predicted grey error worst {worst:.2}",
            w.name(),
            white[1],
            100.0 * white[1] / native[1],
            cct(white)
        );
        let path = ti3.with_file_name(format!(
            "{}-table-{}.txt",
            ti3.file_stem()
                .map_or("cal".into(), |s| s.to_string_lossy().into_owned()),
            w.name()
        ));
        let body: String = wd.iter().map(|w| format!("0x{w:08X}\n")).collect();
        std::fs::write(&path, body).map_err(|e| format!("{}: {e}", path.display()))?;
        let _ = writeln!(report, "    {}", path.display());
    }
    eprint!("{report}");
    let rp = ti3.with_extension("calibration.md");
    std::fs::write(&rp, &report).map_err(|e| format!("{}: {e}", rp.display()))?;
    Ok(())
}

/// The greys a calibration pass reads: every 8 levels, so both the table's
/// points and the levels between them (the 2700G's interpolation) are seen.
pub fn pass_greys() -> Vec<u8> {
    (0..=256u32).step_by(8).map(|v| v.min(255) as u8).collect()
}

/// One pass's errors: per grey, (level, reading, Delta E 2000 to target,
/// the model's prediction's Delta E to the reading).
fn pass_report(m: &Model, w: White, words: &[u32; 17], got: &[(u8, Xyz)], out: &mut String) -> f64 {
    let white = target_white(m, w);
    let mut worst = 0.0f64;
    let mut sum = 0.0;
    let _ = writeln!(
        out,
        "| grey | cd/m2 | target | K | Delta E to target | model vs reading |"
    );
    let _ = writeln!(out, "|---|---|---|---|---|---|");
    for &(v, xyz) in got {
        let want = grey_target(m, white, f64::from(v));
        let e = de2000(lab(xyz, white), lab(want, white));
        let p = m.predict(through(words, v));
        let em = de2000(lab(xyz, white), lab(p, white));
        worst = worst.max(e);
        sum += e;
        let _ = writeln!(
            out,
            "| {v} | {:.3} | {:.3} | {:.0} | {e:.2} | {em:.2} |",
            xyz[1],
            want[1],
            cct(xyz)
        );
    }
    let mean = sum / got.len().max(1) as f64;
    let _ = writeln!(
        out,
        "
mean Delta E {mean:.2}, worst {worst:.2}
"
    );
    worst
}

/// `cargo xtask calibrate <profile.ti3> --verify <whites> [--passes n]`: for
/// each white, the table shown on the Axim and its greys read, corrected
/// from the readings, read again, `passes` times; the last table and every
/// pass's errors saved next to the profile. No hands needed between passes.
pub fn verify(root: &Path, ti3: &Path, whites: &[White], passes: u32) -> Result<(), String> {
    let text = std::fs::read_to_string(ti3).map_err(|e| format!("{}: {e}", ti3.display()))?;
    let m = Model::from_samples(&read_ti3(&text)?)?;
    let native = m.predict([255.0; 3]);
    let greys: Vec<(u8, u8, u8)> = pass_greys().into_iter().map(|g| (g, g, g)).collect();
    let stem = ti3
        .file_stem()
        .map_or("cal".into(), |s| s.to_string_lossy().into_owned());
    let mut spot = crate::profile::open_spot()?;
    let mut report = String::new();
    for &w in whites {
        let mut t = table(&m, w);
        let _ = writeln!(
            report,
            "## {} white
",
            w.name()
        );
        for pass in 1..=passes {
            let wd = words(&t);
            eprintln!("    {} white, pass {pass} of {passes}", w.name());
            let (vals, white_now) = crate::profile::measure_corrected(
                root,
                &mut spot,
                &greys,
                wd,
                rp_screen_measure::program::StepLight::Counts(432),
            )?;
            // To the profile's scale: the backlight may have drifted since.
            let k = native[1] / white_now.y;
            let got: Vec<(u8, Xyz)> = greys
                .iter()
                .zip(vals)
                .filter_map(|(g, v)| v.map(|v| (g.0, [v.x * k, v.y * k, v.z * k])))
                .collect();
            let _ = writeln!(
                report,
                "### pass {pass} (panel white now {:.1} cd/m2, scaled by {k:.4})
",
                white_now.y
            );
            let _ = writeln!(
                report,
                "table: {}
",
                wd.iter()
                    .map(|w| format!("{w:06X}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            );
            let worst = pass_report(&m, w, &wd, &got, &mut report);
            eprintln!("    worst grey Delta E {worst:.2}");
            t = refine(&m, w, &t, &got);
        }
        let path = ti3.with_file_name(format!("{stem}-table-{}-refined.txt", w.name()));
        let body: String = words(&t)
            .iter()
            .map(|w| {
                format!(
                    "0x{w:08X}
"
                )
            })
            .collect();
        std::fs::write(&path, body).map_err(|e| format!("{}: {e}", path.display()))?;
        let _ = writeln!(
            report,
            "refined table (not yet measured): {}
",
            path.display()
        );
        let rp = ti3.with_file_name(format!("{stem}-verify.md"));
        std::fs::write(&rp, &report).map_err(|e| format!("{}: {e}", rp.display()))?;
    }
    drop(spot);
    eprint!("{report}");
    Ok(())
}

/// `native`, or a colour temperature in kelvin.
pub fn parse_white(s: &str) -> Result<White, String> {
    if s == "native" {
        return Ok(White::Native);
    }
    let k: f64 = s
        .trim_end_matches(['K', 'k'])
        .parse()
        .map_err(|_| format!("white {s}: native or a temperature like 6504"))?;
    if !(4000.0..=25000.0).contains(&k) {
        return Err(format!("white {s}: 4000-25000 K"));
    }
    Ok(White::Daylight(k))
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A panel like the owner's: primaries and black from runs 2-3, each
    /// channel gamma 1.8, plus a little non-additive cross-talk.
    pub fn panel(rgb: [f64; 3]) -> Xyz {
        let prim = |x: f64, y: f64, yy: f64| [x / y * yy, yy, (1.0 - x - y) / y * yy];
        let p = [
            prim(0.488, 0.305, 13.0),
            prim(0.295, 0.426, 36.0),
            prim(0.151, 0.134, 8.0),
        ];
        let black = prim(0.27, 0.27, 2.1);
        let mut out = black;
        for c in 0..3 {
            let f = (rgb[c] / 255.0).powf(1.8);
            for k in 0..3 {
                out[k] += p[c][k] * f;
            }
        }
        out
    }

    fn samples() -> Vec<Sample> {
        let mut s = vec![];
        let mut put = |r: u8, g: u8, b: u8| {
            s.push(Sample {
                rgb: (r, g, b),
                xyz: panel([f64::from(r), f64::from(g), f64::from(b)]),
            })
        };
        put(0, 0, 0);
        for &v in &LEVELS[1..] {
            put(v, 0, 0);
            put(0, v, 0);
            put(0, 0, v);
            put(v, v, v);
        }
        put(200, 30, 90);
        s
    }

    #[test]
    fn de2000_matches_sharmas_reference_pairs() {
        // Sharma, Wu, Dalal (2005), pairs 1, 7 and 17.
        let cases = [
            ([50.0, 2.6772, -79.7751], [50.0, 0.0, -82.7485], 2.0425),
            ([50.0, 0.0, 0.0], [50.0, -1.0, 2.0], 2.3669),
            ([50.0, 2.5, 0.0], [73.0, 25.0, -18.0], 27.1492),
        ];
        for (a, b, want) in cases {
            assert!(
                (de2000(a, b) - want).abs() < 1e-3,
                "{a:?} {b:?}: {}",
                de2000(a, b)
            );
        }
    }

    #[test]
    fn the_model_rebuilds_an_additive_panel() {
        let m = Model::from_samples(&samples()).unwrap();
        let (mean, worst) = model_error(&m, &samples());
        assert!(worst < 1.0, "{mean} {worst}");
        let p = m.predict([128.0, 128.0, 128.0]);
        let q = panel([128.0; 3]);
        assert!((p[1] - q[1]).abs() / q[1] < 0.01);
    }

    #[test]
    fn a_d65_table_makes_neutral_bt1886_greys() {
        let m = Model::from_samples(&samples()).unwrap();
        let t = table(&m, White::Daylight(6504.0));
        let w = words(&t);
        let white = target_white(&m, White::Daylight(6504.0));
        assert!((cct(white) - 6504.0).abs() < 60.0, "{}", cct(white));
        // Through the real (synthetic) panel, with the table's rounding.
        for v in [32u8, 64, 128, 192, 240, 255] {
            let shown = panel(through(&w, v));
            let want = grey_target(&m, white, f64::from(v));
            let e = de2000(lab(shown, white), lab(want, white));
            assert!(e < 1.5, "grey {v}: Delta E {e}");
        }
        // White is turned down to reach D65 on a blue panel: blue lowest.
        assert!(
            t[16][2] < t[16][0] && t[16][0] <= t[16][1] + 1e-9,
            "{:?}",
            t[16]
        );
        // The native white keeps every channel's top.
        let n = words(&table(&m, White::Native));
        assert_eq!(n[16] & 0x00FF00, 0x00FF00);
    }

    #[test]
    fn refine_moves_a_point_towards_its_target() {
        let m = Model::from_samples(&samples()).unwrap();
        let w = White::Daylight(9000.0);
        let t = table(&m, w);
        // The real panel 5% dimmer in green than the model.
        let real = |x: [f64; 3]| {
            let mut v = panel(x);
            v[1] *= 0.95;
            v
        };
        let white = target_white(&m, w);
        let err = |t: &Table, v: u8| {
            let p = real(through(&words(t), v));
            de2000(
                lab(p, white),
                lab(grey_target(&m, white, f64::from(v)), white),
            )
        };
        let before = err(&t, 128);
        let r = refine(&m, w, &t, &[(128, real(through(&words(&t), 128)))]);
        let after = err(&r, 128);
        assert!(after < before / 2.0, "{before} -> {after}");
    }

    #[test]
    fn whites_and_pass_greys() {
        assert_eq!(parse_white("native"), Ok(White::Native));
        assert_eq!(parse_white("6504"), Ok(White::Daylight(6504.0)));
        assert_eq!(parse_white("9000K"), Ok(White::Daylight(9000.0)));
        assert!(parse_white("100").is_err() && parse_white("warm").is_err());
        let g = pass_greys();
        assert_eq!((g.len(), g[0], g[1], g[32]), (33, 0, 8, 255));
        // Every table point is among them.
        assert!(LEVELS.iter().all(|l| g.contains(l)));
    }

    #[test]
    fn ti3_round_trip() {
        let t = "CTI3\nLUMINANCE_XYZ_CDM2 \"50.0 57.0 70.0\"\nNUMBER_OF_FIELDS 7\nBEGIN_DATA_FORMAT\nSAMPLE_ID RGB_R RGB_G RGB_B XYZ_X XYZ_Y XYZ_Z\nEND_DATA_FORMAT\nNUMBER_OF_SETS 1\nBEGIN_DATA\n1 100.0 50.1961 0 50 100 120\nEND_DATA\n";
        let s = read_ti3(t).unwrap();
        assert_eq!(s[0].rgb, (255, 128, 0));
        assert!((s[0].xyz[1] - 57.0).abs() < 1e-9);
    }
}
