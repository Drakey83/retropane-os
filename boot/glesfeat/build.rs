//! Links the image with the shared `boot/link.x` when building for the X51v.

fn main() {
    println!("cargo:rerun-if-changed=../link.x");
    // Test G7: Dell's frame file (`cargo xtask g7-frame`), embedded when
    // named; it stays local, so the image builds without it otherwise.
    println!("cargo:rerun-if-env-changed=RP_GLESFEAT_G7_FRAME");
    if let Ok(f) = std::env::var("RP_GLESFEAT_G7_FRAME") {
        println!("cargo:rerun-if-changed={f}");
        println!("cargo:rustc-cfg=rp_g7");
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("none") {
        let dir = std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo");
        println!("cargo:rustc-link-arg-bins=-T{dir}/../link.x");
    }
}
