//! `cargo xtask measure`: the X51v's screen measured with a colorimeter
//! (docs/design/display-calibration.md, "Measured with a colorimeter").
//!
//! Starts `rp-screen-measure` on the Axim, then reads the colorimeter on
//! each of its patches at the time the shared schedule
//! ([`rp_screen_measure::PATCHES`]) says, with ArgyllCMS's `spotread` (one
//! reading per run, about 10 s). Writes the readings and a report under
//! `local/screen-measure/`: the panel's grey curve and gamma, black and
//! white in cd/m², contrast, white point, primaries, the backlight's steps,
//! drift, and a gamma table that makes the panel standard (gamma 2.2, D65
//! white).
//!
//! Argyll is found through `RETROPANE_ARGYLL` (its `bin` folder) or under
//! the user's Downloads. The colorimeter's port is the one Argyll lists as
//! a USB instrument; serial ports (the Axim is one) are never used.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use rp_screen_measure::{PATCHES, READ_AT_MS, start_ms, total_ms};

/// One reading, in cd/m² (XYZ, Y = luminance).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Xyz {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Xyz {
    fn minus(self, o: Self) -> Self {
        Self {
            x: (self.x - o.x).max(0.0),
            y: (self.y - o.y).max(0.0),
            z: (self.z - o.z).max(0.0),
        }
    }
    /// Chromaticity (x, y).
    fn xy(self) -> (f64, f64) {
        let s = self.x + self.y + self.z;
        if s <= 0.0 {
            (0.0, 0.0)
        } else {
            (self.x / s, self.y / s)
        }
    }
}

pub fn measure(root: &Path) -> Result<(), String> {
    let spotread = find_spotread()?;
    let port = instrument_port(&spotread)?;
    let ccss = home().join(".local/share/ArgyllCMS/WLEDFamily_07Feb11.ccss");
    eprintln!(
        "    colorimeter: Argyll port {port}; white-LED correction {}",
        if ccss.exists() {
            "on"
        } else {
            "not found (off)"
        }
    );
    eprintln!(
        "    {} patches, about {} minutes. Keep the sensor flat on the screen's middle; Home on the Axim stops early",
        PATCHES.len(),
        total_ms().div_ceil(60_000)
    );
    let t0 = crate::usb::start(root, "rp-screen-measure")?;
    eprintln!("    the Axim started the patches");
    let mut readings: Vec<Option<Xyz>> = Vec::new();
    for (i, p) in PATCHES.iter().enumerate() {
        let read_at = t0 + Duration::from_millis(u64::from(start_ms(i) + READ_AT_MS));
        let ends = t0 + Duration::from_millis(u64::from(start_ms(i) + p.ms));
        if let Some(wait) = read_at.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
        let began = Instant::now();
        let r = read_once(&spotread, port, &ccss);
        let took = began.elapsed().as_secs_f64();
        let late = Instant::now() > ends;
        let r = match r {
            Ok(_) if late => {
                eprintln!(
                    "    {:2}/{} {}: reading ran past the patch ({took:.1} s), dropped",
                    i + 1,
                    PATCHES.len(),
                    p.name
                );
                None
            }
            Ok(v) => {
                eprintln!(
                    "    {:2}/{} {}: {:.3} cd/m2 ({took:.1} s)",
                    i + 1,
                    PATCHES.len(),
                    p.name,
                    v.y
                );
                Some(v)
            }
            Err(e) => {
                eprintln!(
                    "    {:2}/{} {}: no reading ({e})",
                    i + 1,
                    PATCHES.len(),
                    p.name
                );
                None
            }
        };
        readings.push(r);
    }
    let dir = root.join("local/screen-measure");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let mut csv = String::from("patch,name,X,Y,Z\n");
    for (i, (p, r)) in PATCHES.iter().zip(&readings).enumerate() {
        match r {
            Some(v) => csv += &format!("{i},{},{:.5},{:.5},{:.5}\n", p.name, v.x, v.y, v.z),
            None => csv += &format!("{i},{},,,\n", p.name),
        }
    }
    let csv_path = dir.join(format!("readings-{stamp}.csv"));
    std::fs::write(&csv_path, csv).map_err(|e| format!("{}: {e}", csv_path.display()))?;
    let report = analyse(&readings)?;
    let md_path = dir.join(format!("report-{stamp}.md"));
    std::fs::write(&md_path, &report).map_err(|e| format!("{}: {e}", md_path.display()))?;
    eprintln!("{report}");
    eprintln!("    saved {} and {}", csv_path.display(), md_path.display());
    Ok(())
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var("USERPROFILE").unwrap_or_else(|_| ".".into()))
}

/// Argyll's `spotread`: `RETROPANE_ARGYLL` (the `bin` folder), else the
/// newest `Argyll_*` under Downloads.
fn find_spotread() -> Result<PathBuf, String> {
    if let Ok(dir) = std::env::var("RETROPANE_ARGYLL") {
        let p = Path::new(&dir).join("spotread.exe");
        return if p.exists() {
            Ok(p)
        } else {
            Err(format!("RETROPANE_ARGYLL: no spotread.exe in {dir}"))
        };
    }
    let mut found = Vec::new();
    let downloads = home().join("Downloads");
    for e in std::fs::read_dir(&downloads)
        .into_iter()
        .flatten()
        .flatten()
    {
        if !e.file_name().to_string_lossy().starts_with("Argyll") {
            continue;
        }
        for sub in [e.path(), e.path().join(e.file_name())] {
            let p = sub.join("bin/spotread.exe");
            if p.exists() {
                found.push(p);
            }
        }
        for inner in std::fs::read_dir(e.path()).into_iter().flatten().flatten() {
            let p = inner.path().join("bin/spotread.exe");
            if p.exists() {
                found.push(p);
            }
        }
    }
    found.sort();
    found
        .pop()
        .ok_or_else(|| "ArgyllCMS not found: unpack it under Downloads or set RETROPANE_ARGYLL to its bin folder".into())
}

/// The port number Argyll gives the USB colorimeter (never a COM port).
fn instrument_port(spotread: &Path) -> Result<u32, String> {
    let out = Command::new(spotread)
        .arg("-?")
        .current_dir(std::env::temp_dir())
        .output()
        .map_err(|e| format!("{}: {e}", spotread.display()))?;
    let text = String::from_utf8_lossy(&out.stdout) + String::from_utf8_lossy(&out.stderr);
    parse_port(&text).ok_or_else(|| {
        "Argyll lists no USB colorimeter: is the Spyder plugged in, on the ArgyllCMS driver?".into()
    })
}

fn parse_port(listing: &str) -> Option<u32> {
    listing.lines().find_map(|l| {
        let (n, name) = l.trim().split_once(" = ")?;
        let name = name.to_ascii_lowercase();
        (name.contains("libusb") || name.contains("spyder") || name.contains("hid"))
            .then(|| n.trim().parse().ok())
            .flatten()
    })
}

/// One emissive reading.
fn read_once(spotread: &Path, port: u32, ccss: &Path) -> Result<Xyz, String> {
    let mut cmd = Command::new(spotread);
    cmd.args(["-e", "-c", &port.to_string(), "-O"]);
    if ccss.exists() {
        cmd.arg("-X").arg(ccss);
    }
    let out = cmd
        .current_dir(std::env::temp_dir())
        .output()
        .map_err(|e| format!("spotread: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    parse_result(&text).ok_or_else(|| {
        let last = text
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("");
        format!("spotread said: {}", last.trim())
    })
}

fn parse_result(text: &str) -> Option<Xyz> {
    let line = text.lines().find(|l| l.contains("Result is XYZ:"))?;
    let rest = line.split("XYZ:").nth(1)?;
    let mut n = rest.split([' ', ',']).filter_map(|t| t.parse::<f64>().ok());
    Some(Xyz {
        x: n.next()?,
        y: n.next()?,
        z: n.next()?,
    })
}

/// McCamy's correlated colour temperature from (x, y).
fn cct((x, y): (f64, f64)) -> f64 {
    let n = (x - 0.3320) / (0.1858 - y);
    449.0 * n.powi(3) + 3525.0 * n.powi(2) + 6823.3 * n + 5520.33
}

/// Solves `m k = w` for a 3x3 `m` (columns: the primaries' XYZ).
fn solve3(m: [[f64; 3]; 3], w: [f64; 3]) -> Option<[f64; 3]> {
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
        for r in 0..3 {
            a[r][c] = w[r];
        }
        det(a) / d
    }))
}

/// The report, from one reading per patch (in [`PATCHES`] order).
pub fn analyse(readings: &[Option<Xyz>]) -> Result<String, String> {
    let get = |name: &str| -> Option<Xyz> {
        PATCHES
            .iter()
            .position(|p| p.name == name)
            .and_then(|i| readings.get(i).copied().flatten())
    };
    let stray = get("stray").unwrap_or(Xyz {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    });
    let need = |name: &str| {
        get(name)
            .map(|v| v.minus(stray))
            .ok_or(format!("no reading for {name}"))
    };
    let black = need("grey 0")?;
    let white = need("grey 255")?;
    let span = white.y - black.y;
    if span <= 0.0 {
        return Err("white reads no brighter than black: was the sensor on the screen?".into());
    }
    let mut out = String::from("# X51v screen measurement\n\n");
    out += &format!(
        "Stray light (backlight off): {:.3} cd/m2, subtracted from every reading.\n\n",
        stray.y
    );
    out += &format!(
        "| | |\n|---|---|\n| White (Windows Mobile's backlight) | {:.1} cd/m2 |\n| Black | {:.3} cd/m2 |\n| Contrast | {:.0}:1 |\n",
        white.y,
        black.y,
        white.y / black.y.max(1e-6)
    );
    let wxy = white.xy();
    out += &format!(
        "| White point | x {:.4} y {:.4}, about {:.0} K (D65 is x 0.3127 y 0.3290, 6504 K) |\n",
        wxy.0,
        wxy.1,
        cct(wxy)
    );
    // The grey curve, normalised between black and white.
    let mut curve: Vec<(f64, f64)> = vec![];
    out += "\n## Grey curve (straight gamma table)\n\n| Level | cd/m2 | of white | gamma here |\n|---|---|---|---|\n";
    let mut gammas = vec![];
    for g in rp_screen_measure::GREYS {
        let Some(v) = get(&format!("grey {g}")).map(|v| v.minus(stray)) else {
            continue;
        };
        let n = ((v.y - black.y) / span).clamp(0.0, 1.0);
        curve.push((f64::from(g), n));
        let x = f64::from(g) / 255.0;
        let local = if g > 0 && g < 255 && n > 0.0 {
            n.ln() / x.ln()
        } else {
            f64::NAN
        };
        if (64..=224).contains(&g) && local.is_finite() {
            gammas.push(local);
        }
        out += &format!(
            "| {g} | {:.3} | {:.2}% | {} |\n",
            v.y,
            n * 100.0,
            if local.is_finite() {
                format!("{local:.2}")
            } else {
                "-".into()
            }
        );
    }
    let gamma = if gammas.is_empty() {
        f64::NAN
    } else {
        gammas.iter().sum::<f64>() / gammas.len() as f64
    };
    out += &format!("\nPanel gamma (mid-tones, 64-224): **{gamma:.2}** (standard: 2.2).\n");
    // Primaries.
    let prim: Vec<Option<Xyz>> = ["red 255", "green 255", "blue 255"]
        .iter()
        .map(|n| get(n).map(|v| v.minus(stray).minus(black)))
        .collect();
    out += "\n## Primaries\n\n| | x | y | cd/m2 |\n|---|---|---|---|\n";
    for (name, p) in ["Red", "Green", "Blue"].iter().zip(&prim) {
        if let Some(p) = p {
            let (x, y) = p.xy();
            out += &format!("| {name} | {x:.4} | {y:.4} | {:.2} |\n", p.y);
        }
    }
    // Backlight.
    out += "\n## Backlight (white)\n\n| Backlight | cd/m2 |\n|---|---|\n";
    for k in 1..=8 {
        let name = if k == 8 {
            "white at full light".to_string()
        } else {
            format!("white at {k}/8 light")
        };
        if let Some(v) = get(&name) {
            out += &format!("| {k}/8 | {:.1} |\n", v.minus(stray).y);
        }
    }
    if let Some(b) = get("black at full light") {
        out += &format!(
            "\nBlack at full backlight: {:.3} cd/m2.\n",
            b.minus(stray).y
        );
    }
    if let Some(d) = get("white again (drift)") {
        out += &format!(
            "Drift: white at the end reads {:+.1}% against the start.\n",
            (d.minus(stray).y / white.y - 1.0) * 100.0
        );
    }
    // The correcting table: gamma 2.2 between the panel's own black and
    // white, white balanced to D65 by turning channels down.
    let table = correction(&curve, &prim);
    out += "\n## Correcting gamma table (gamma 2.2, D65 white)\n\n";
    match table {
        Some((t, k)) => {
            out += &format!(
                "Channel scale for D65: red {:.3}, green {:.3}, blue {:.3}.\n\n```\n",
                k[0], k[1], k[2]
            );
            for w in t {
                out += &format!("{w:06X} ");
            }
            out += "\n```\n";
        }
        None => out += "Not enough readings for a table.\n",
    }
    Ok(out)
}

/// Inverse of the measured curve: the level giving normalised light `l`.
fn level_for(curve: &[(f64, f64)], l: f64) -> f64 {
    if l <= 0.0 {
        return 0.0;
    }
    for w in curve.windows(2) {
        let ((v0, l0), (v1, l1)) = (w[0], w[1]);
        if l <= l1 && l1 > l0 {
            return v0 + (v1 - v0) * ((l - l0) / (l1 - l0)).clamp(0.0, 1.0);
        }
    }
    255.0
}

/// The 17-point table and the D65 channel scales, from the grey curve and
/// the primaries.
fn correction(curve: &[(f64, f64)], prim: &[Option<Xyz>]) -> Option<([u32; 17], [f64; 3])> {
    if curve.len() < 6 {
        return None;
    }
    let k = match (prim[0], prim[1], prim[2]) {
        (Some(r), Some(g), Some(b)) => {
            let m = [[r.x, g.x, b.x], [r.y, g.y, b.y], [r.z, g.z, b.z]];
            let k = solve3(m, [0.9505, 1.0, 1.0890])?;
            let top = k.iter().cloned().fold(f64::MIN, f64::max);
            if top <= 0.0 || k.iter().any(|&v| v <= 0.0) {
                [1.0; 3]
            } else {
                k.map(|v| v / top)
            }
        }
        _ => [1.0; 3],
    };
    let table = core::array::from_fn(|i| {
        let x = (16.0 * i as f64).min(255.0) / 255.0;
        let target = x.powf(2.2);
        let lane = |c: usize| level_for(curve, target * k[c]).round().clamp(0.0, 255.0) as u32;
        lane(0) << 16 | lane(1) << 8 | lane(2)
    });
    Some((table, k))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Readings from a made-up panel of gamma `g`, 200 cd/m2 white,
    /// 0.5 black, a D65 white from sRGB-like primaries.
    fn fake(g: f64) -> Vec<Option<Xyz>> {
        let prim = [
            Xyz {
                x: 0.4124,
                y: 0.2126,
                z: 0.0193,
            },
            Xyz {
                x: 0.3576,
                y: 0.7152,
                z: 0.1192,
            },
            Xyz {
                x: 0.1805,
                y: 0.0722,
                z: 0.9505,
            },
        ];
        PATCHES
            .iter()
            .map(|p| {
                let lin = |v: u8| (f64::from(v) / 255.0).powf(g);
                let (r, gr, b) = (lin(p.rgb.0), lin(p.rgb.1), lin(p.rgb.2));
                let light = match p.light {
                    rp_screen_measure::Light::Off => 0.0,
                    rp_screen_measure::Light::Found => 200.0,
                    rp_screen_measure::Light::Eighths(n) => 300.0 * f64::from(n) / 8.0,
                };
                let mix = |f: fn(&Xyz) -> f64| {
                    light * (r * f(&prim[0]) + gr * f(&prim[1]) + b * f(&prim[2]))
                        + light / 400.0
                            * f(&Xyz {
                                x: 0.9505,
                                y: 1.0,
                                z: 1.089,
                            })
                        + 0.01
                };
                Some(Xyz {
                    x: mix(|p| p.x),
                    y: mix(|p| p.y),
                    z: mix(|p| p.z),
                })
            })
            .collect()
    }

    #[test]
    fn a_gamma_1_5_panel_is_measured_as_1_5_and_corrected() {
        let r = analyse(&fake(1.5)).unwrap();
        assert!(r.contains("**1.5"), "{r}");
        assert!(r.contains("6504 K") && r.contains("Correcting gamma table"));
        // The table darkens the mid-tones of a too-bright panel.
        let (t, k) = correction(
            &rp_screen_measure::GREYS
                .iter()
                .map(|&g| (f64::from(g), (f64::from(g) / 255.0).powf(1.5)))
                .collect::<Vec<_>>(),
            &[None, None, None],
        )
        .unwrap();
        assert_eq!(k, [1.0; 3]);
        assert!((t[8] & 0xFF) < 128, "{:06X}", t[8]);
        assert_eq!(t[0], 0);
        assert!(t.windows(2).all(|w| (w[1] & 0xFF) >= (w[0] & 0xFF)));
    }

    #[test]
    fn spotread_output_and_port_listing_parse() {
        let r =
            parse_result(" Result is XYZ: 2.583447 2.604601 5.139377, D50 Lab: 18.38 1.40 -20.0")
                .unwrap();
        assert!((r.y - 2.604601).abs() < 1e-9);
        let listing = " -c listno   Set instrument port\n    1 = 'COM1'\n    2 = 'libusb0-0001 (Datacolor Spyder4)'\n    3 = 'COM4'\n";
        assert_eq!(parse_port(listing), Some(2));
        assert_eq!(parse_port("    1 = 'COM1'\n    2 = 'COM4'\n"), None);
    }

    #[test]
    fn missing_readings_are_reported_not_invented() {
        let mut r = fake(2.2);
        r[1] = None; // grey 0
        assert!(analyse(&r).unwrap_err().contains("grey 0"));
    }
}
