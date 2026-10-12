//! Links the image with the shared `boot/link.x` when building for the X51v,
//! and supplies the picture: `local/concept/picture.rgb565` (480 x 640,
//! RGB565 little-endian; `local/` is never committed, so concept art with
//! third-party pieces stays out of the repository) or, when that's absent,
//! a generated test gradient so the image always builds.

use std::path::Path;

const BYTES: usize = 480 * 640 * 2;

fn main() {
    println!("cargo:rerun-if-changed=../link.x");
    let dir = std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo");
    let source = Path::new(&dir).join("../../local/concept/picture.rgb565");
    // The folder too: a picture added or removed with an old timestamp
    // changes the folder, not anything cargo saw before.
    println!("cargo:rerun-if-changed={}", source.display());
    if let Some(folder) = source.parent().filter(|f| f.is_dir()) {
        println!("cargo:rerun-if-changed={}", folder.display());
    }
    let picture = match std::fs::read(&source) {
        Ok(p) if p.len() == BYTES => p,
        Ok(p) => panic!("{}: {} bytes, want {BYTES}", source.display(), p.len()),
        Err(_) => gradient(),
    };
    let out = std::env::var("OUT_DIR").expect("set by cargo");
    std::fs::write(Path::new(&out).join("picture.rgb565"), picture).expect("OUT_DIR writable");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("none") {
        println!("cargo:rustc-link-arg-bins=-T{dir}/../link.x");
    }
}

/// Blue at the top to green at the bottom, red rising left to right.
fn gradient() -> Vec<u8> {
    let mut v = Vec::with_capacity(BYTES);
    for y in 0..640u32 {
        for x in 0..480u32 {
            let px = ((x * 31 / 479) << 11) | ((y * 63 / 639) << 5) | (31 - y * 31 / 639);
            v.extend_from_slice(&(px as u16).to_le_bytes());
        }
    }
    v
}
