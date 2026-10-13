//! Links the image with the shared `boot/link.x` when building for the X51v,
//! with an abort-mode stack: the kernel resolves page faults in abort mode.
//!
//! Also embeds the user process images `cargo xtask` packaged next to this
//! build's output (`<name>.pe`, `xtask/src/userimg.rs`); an image not built
//! yet is embedded empty, and the kernel's check for it fails.

use std::path::PathBuf;

const USER_IMAGES: &[&str] = &["rp-display-host"];

fn main() {
    println!("cargo:rerun-if-changed=../link.x");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("none") {
        let dir = std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo");
        println!("cargo:rustc-link-arg-bins=-T{dir}/../link.x");
        println!("cargo:rustc-link-arg-bins=--defsym=__abt_stack_size=4096");
    }
    // OUT_DIR is <target>/<triple>/<profile>/build/.../out (the depth
    // under `build` differs between Cargo versions).
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("set by cargo"));
    let release = out
        .ancestors()
        .find(|p| p.file_name().is_some_and(|n| n == "build"))
        .and_then(|p| p.parent())
        .map(PathBuf::from)
        .unwrap_or_default();
    for name in USER_IMAGES {
        let pe = release.join(format!("{name}.pe"));
        println!("cargo:rerun-if-changed={}", pe.display());
        let bytes = std::fs::read(&pe).unwrap_or_default();
        std::fs::write(out.join(format!("{name}.pe")), bytes).expect("OUT_DIR writable");
    }
}
