//! Links the image with the shared `boot/link.x` when building for the X51v,
//! and supplies:
//!
//! - the wallpaper: `local/wallpapers/<name>/<name>.yuv`, `<name>` from
//!   `RP_TODAY_WALLPAPER` (default `pier`; made by
//!   `tools/shell-assets/wallpaper.py` from the photo; `local/` is never
//!   committed) or, when that's absent, a generated dusk sky, so the image
//!   always builds. A test image holds one (950 KiB image area), so the
//!   16-bit comparison is one build per wallpaper;
//! - the weather pictures (`local/icons/icons.rs`, else empty);
//! - the clock's start: the build machine's local time (there is no RTC
//!   driver yet), as minutes since 1970 plus the UTC offset. The clock is
//!   right to within the time between building and running.

use std::path::Path;

include!("../../tools/build/local_input.rs");

const W: usize = 480;
const H: usize = 640;
const BYTES: usize = W * H * 3 / 2;

fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo");
    let out = std::env::var("OUT_DIR").expect("set by cargo");
    println!("cargo:rerun-if-env-changed=RP_TODAY_WALLPAPER");
    let name = std::env::var("RP_TODAY_WALLPAPER").unwrap_or_else(|_| "pier".into());
    assert!(
        !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'),
        "RP_TODAY_WALLPAPER: a folder name under local/wallpapers"
    );
    let source = Path::new(&dir).join(format!("../../local/wallpapers/{name}/{name}.yuv"));
    watch_local(&source);
    let wall = match std::fs::read(&source) {
        Ok(p) if p.len() == BYTES => p,
        Ok(p) => panic!("{}: {} bytes, want {BYTES}", source.display(), p.len()),
        Err(_) => {
            // Asked for by name: say it isn't there (CI has none, quietly).
            if std::env::var("RP_TODAY_WALLPAPER").is_ok() {
                println!(
                    "cargo:warning=no {}: the generated sky instead",
                    source.display()
                );
            }
            sky()
        }
    };
    println!("cargo:rustc-env=RP_TODAY_WALLPAPER_NAME={name}");
    std::fs::write(Path::new(&out).join("wallpaper.yuv"), wall).expect("OUT_DIR writable");

    // The weather pictures: `local/icons/icons.rs` (made by
    // `tools/shell-assets/icons.py` from assets/weather/vclouds; `local/` is
    // never committed) or, when absent, empty pictures: the screen then
    // draws outlines where they go.
    let icons = Path::new(&dir).join("../../local/icons/icons.rs");
    watch_local(&icons);
    let data = std::fs::read_to_string(&icons).unwrap_or_else(|_| {
        let empty = "Image { w: 0, h: 0, px: &[] }";
        format!(
            "pub static HERO: Image = {empty};
pub static SMALL: [Image; 5] = [{empty}, {empty}, {empty}, {empty}, {empty}];
"
        )
    });
    // The owner's default picture (`rp_draw::look`, 2026-10-22) on every
    // icon pixel (non-premultiplied ARGB: the colour changes, alpha not),
    // the same function the wallpaper goes through as it loads;
    // `RP_PICTURE_ORIGINAL` leaves them as made.
    println!("cargo:rerun-if-env-changed=RP_PICTURE_ORIGINAL");
    let data = if std::env::var("RP_PICTURE_ORIGINAL").is_ok() {
        data
    } else {
        looked(&data)
    };
    std::fs::write(Path::new(&out).join("icons.rs"), data).expect("OUT_DIR writable");

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
        // 12 KiB of stack. It was 24, then 40: the image used 23.5 KiB (96% of
        // 24) and an older build froze the Axim (the watchdog error screen,
        // 2026-10-17). Of that, 22 KiB was `main`'s frame holding the
        // long-lived objects; they are in `rt::Slot`s in `.bss` now, where
        // the zeroed-RAM budget counts them. Every emulator image fails above
        // 80% stack use.
        println!("cargo:rustc-link-arg-bins=--defsym=__stack_size=12288");
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

/// `text` with every `0xAARRGGBB` pixel (the generator's only hex: ten
/// characters) through `rp_draw::look::apply`, alpha kept.
fn looked(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("0x") {
        out.push_str(&rest[..i]);
        let hex = rest
            .get(i + 2..i + 10)
            .filter(|h| h.bytes().all(|b| b.is_ascii_hexdigit()));
        match hex.and_then(|h| u32::from_str_radix(h, 16).ok()) {
            Some(px) => {
                let [a, r, g, b] = px.to_be_bytes();
                let (r, g, b) = rp_draw::look::apply(r, g, b);
                out.push_str(&format!("0x{:08x}", u32::from_be_bytes([a, r, g, b])));
                rest = &rest[i + 10..];
            }
            None => {
                out.push_str("0x");
                rest = &rest[i + 2..];
            }
        }
    }
    out.push_str(rest);
    out
}
