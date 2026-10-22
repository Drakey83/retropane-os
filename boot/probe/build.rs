//! Links the image with the shared `boot/link.x` when building for the X51v.

fn main() {
    println!("cargo:rerun-if-changed=../link.x");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("none") {
        let dir = std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo");
        println!("cargo:rustc-link-arg-bins=-T{dir}/../link.x");
        // 32 KiB of stack (the default 16): the device's runs reached
        // 16,076-16,268 of 16,384 bytes, and one crashed (2026-10-22).
        println!("cargo:rustc-link-arg-bins=--defsym=__stack_size=32768");
    }
}
