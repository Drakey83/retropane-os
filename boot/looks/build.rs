//! Links the image with the shared `boot/link.x` when building for the X51v,
//! and packs the photos: `local/wallpapers/<name>/<name>.yuv` (480 x 640
//! planar 4:2:0, made by `tools/shell-assets/wallpaper.py`) for `pier` and
//! `milkyway`. `local/` is never committed (the owner's photos stay out of
//! the repository); a missing photo becomes a generated one so the image
//! always builds (the card's `\rp-looks\` folder holds the full set: these
//! two are the fallback with no card). Each is LZ4-compressed in 64 KiB blocks (a loaded image
//! must stay under 950 KB; two raw photos alone are 900 KB): `u32` block
//! count, `u32` packed size per block, then the blocks.

use std::path::Path;

const YUV_BYTES: usize = 480 * 640 * 3 / 2;
const BLOCK: usize = 0x1_0000;

fn main() {
    println!("cargo:rerun-if-changed=../link.x");
    let dir = std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo");
    let out = std::env::var("OUT_DIR").expect("set by cargo");
    for name in ["pier", "milkyway"] {
        let source = Path::new(&dir).join(format!("../../local/wallpapers/{name}/{name}.yuv"));
        println!("cargo:rerun-if-changed={}", source.display());
        let yuv = match std::fs::read(&source) {
            Ok(p) if p.len() == YUV_BYTES => p,
            Ok(p) => panic!("{}: {} bytes, want {YUV_BYTES}", source.display(), p.len()),
            Err(_) => generated(name == "milkyway"),
        };
        std::fs::write(Path::new(&out).join(format!("{name}.lz4")), pack(&yuv))
            .expect("OUT_DIR writable");
    }
    curves(&out);
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("none") {
        println!("cargo:rustc-link-arg-bins=-T{dir}/../link.x");
    }
}

/// The strength dial's S-curves, made here so the image needs no floating
/// point: strength 1.0 to 4.0 in quarter steps, each half of the range bent
/// to that power and meeting at the middle (1.0 is a straight line). Point
/// `i` is the curve at input `16 * i` (point 16 at 255).
fn curves(out: &str) {
    let mut src = String::from(
        "/// The strength dial's curves (`build.rs`): strength `1 + s / 4` for `s` in 0..13.
         pub const CURVES: [[u8; 17]; 13] = [
",
    );
    for s in 0..13 {
        let k = 1.0 + f64::from(s) / 4.0;
        let row: Vec<String> = (0..17)
            .map(|i| {
                let x = f64::from((16 * i).min(255)) / 255.0;
                let y = if x < 0.5 {
                    0.5 * (2.0 * x).powf(k)
                } else {
                    1.0 - 0.5 * (2.0 * (1.0 - x)).powf(k)
                };
                format!("{}", (255.0 * y).round() as u8)
            })
            .collect();
        src += &format!(
            "    [{}],
",
            row.join(", ")
        );
    }
    src += "];
";
    std::fs::write(Path::new(out).join("curves.rs"), src).expect("OUT_DIR writable");
}

fn pack(data: &[u8]) -> Vec<u8> {
    let blocks: Vec<&[u8]> = data.chunks(BLOCK).collect();
    let mut sizes = Vec::new();
    let mut body = Vec::new();
    for b in &blocks {
        let mut dst = vec![0u8; rp_lz4::max_compressed(b.len())];
        let Ok(n) = rp_lz4::compress(b, &mut dst) else {
            panic!("an LZ4 block didn't pack")
        };
        sizes.push(n as u32);
        body.extend_from_slice(&dst[..n]);
    }
    let mut v = (blocks.len() as u32).to_le_bytes().to_vec();
    for s in sizes {
        v.extend_from_slice(&s.to_le_bytes());
    }
    v.extend_from_slice(&body);
    v
}

/// A stand-in photo: a sky-to-ground gradient (dark when `night`).
fn generated(night: bool) -> Vec<u8> {
    let mut v = vec![0u8; YUV_BYTES];
    for y in 0..640 {
        for x in 0..480 {
            let l = 16 + (y * 200 / 639) as u8 / if night { 4 } else { 1 };
            v[y * 480 + x] = l;
        }
    }
    for (i, c) in v[480 * 640..].iter_mut().enumerate() {
        *c = if i < 480 * 640 / 4 { 150 } else { 110 };
    }
    v
}
