//! Links the image with the shared `boot/link.x` when building for the X51v,
//! and supplies:
//!
//! - the wallpaper: `local/wallpapers/pier/pier.yuv` (made by
//!   `tools/shell-assets/wallpaper.py` from the CC0 pier photo; `local/` is
//!   never committed) or, when that's absent, a generated dusk sky, so the
//!   image always builds;
//! - the clock's start: the build machine's local time (there is no RTC
//!   driver yet), as minutes since 1970 plus the UTC offset. The clock is
//!   right to within the time between building and running.

use std::path::Path;

const W: usize = 480;
const H: usize = 640;
const BYTES: usize = W * H * 3 / 2;

fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo");
    let out = std::env::var("OUT_DIR").expect("set by cargo");
    let source = Path::new(&dir).join("../../local/wallpapers/pier/pier.yuv");
    println!("cargo:rerun-if-changed={}", source.display());
    if let Some(folder) = source.parent().filter(|f| f.is_dir()) {
        println!("cargo:rerun-if-changed={}", folder.display());
    }
    let wall = match std::fs::read(&source) {
        Ok(p) if p.len() == BYTES => p,
        Ok(p) => panic!("{}: {} bytes, want {BYTES}", source.display(), p.len()),
        Err(_) => sky(),
    };
    std::fs::write(Path::new(&out).join("wallpaper.yuv"), wall).expect("OUT_DIR writable");

    // Rebuilt each time, so the clock starts near the real time.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=RP_TODAY_FORCE");
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    println!("cargo:rustc-env=RP_TODAY_UTC_MIN={}", secs / 60);
    println!(
        "cargo:rustc-env=RP_TODAY_OFFSET_MIN={}",
        utc_offset_minutes()
    );

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("none") {
        println!("cargo:rustc-link-arg-bins=-T{dir}/../../boot/link.x");
    }
}

/// The build machine's offset from UTC in minutes (0 if it can't be read).
fn utc_offset_minutes() -> i64 {
    let run = |cmd: &str, args: &[&str]| {
        std::process::Command::new(cmd)
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    // Windows: minutes directly. Elsewhere: `date +%z` gives +hhmm.
    if let Some(m) = run(
        "powershell",
        &[
            "-NoProfile",
            "-Command",
            "[DateTimeOffset]::Now.Offset.TotalMinutes",
        ],
    )
    .and_then(|s| s.parse::<f64>().ok())
    {
        return m as i64;
    }
    run("date", &["+%z"])
        .filter(|z| z.len() == 5)
        .and_then(|z| {
            let sign = if z.starts_with('-') { -1 } else { 1 };
            let h: i64 = z[1..3].parse().ok()?;
            let m: i64 = z[3..5].parse().ok()?;
            Some(sign * (h * 60 + m))
        })
        .unwrap_or(0)
}

/// A dusk sky as planar 4:2:0: deep blue at the top to warm low light.
fn sky() -> Vec<u8> {
    let mut y = Vec::with_capacity(W * H);
    for row in 0..H {
        let t = row * 255 / (H - 1);
        y.extend(std::iter::repeat_n((24 + t * 90 / 255) as u8, W));
    }
    let mut u = Vec::with_capacity(W * H / 4);
    let mut v = Vec::with_capacity(W * H / 4);
    for row in 0..H / 2 {
        let t = row * 255 / (H / 2 - 1);
        u.extend(std::iter::repeat_n((150 - t * 40 / 255) as u8, W / 2));
        v.extend(std::iter::repeat_n((118 + t * 30 / 255) as u8, W / 2));
    }
    [y, u, v].concat()
}
