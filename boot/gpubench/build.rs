//! Links the image with the shared `boot/link.x` when building for the X51v.

fn main() {
    println!("cargo:rerun-if-changed=../link.x");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("none") {
        let dir = std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo");
        println!("cargo:rustc-link-arg-bins=-T{dir}/../link.x");
        // 24 KiB of stack (the default 16): runs GC2H and GC2H2 reached
        // 12,940-12,996 bytes on the X51v and GC3 13,172 in the emulator
        // (2026-10-23), past the 80% the checks allow.
        println!("cargo:rustc-link-arg-bins=--defsym=__stack_size=24576");
    }
}
