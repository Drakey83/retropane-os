//! Links the image with the shared `boot/link.x` when building for the X51v.

fn main() {
    println!("cargo:rerun-if-changed=../link.x");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("none") {
        let dir = std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo");
        println!("cargo:rustc-link-arg-bins=-T{dir}/../link.x");
        // 32 KiB of stack (the default 16): runs GC2H and GC2H2 reached
        // 12,940-12,996 bytes on the X51v and GC3 13,172 in the emulator
        // (2026-10-23); the GC2 soaks' shared code then sat at the checks'
        // 80% of 24 KiB, each new run tipping it over (GC2T 19,664, GC2Y
        // 19,928 in the emulator, 2026-10-25): 32 KiB leaves GC2Y at 61%.
        // 40 KiB from 2026-10-25: run GC2AG holds a second renderer (its
        // parameter memory moved) beside the first, 29,256 bytes deep in
        // the emulator (89% of 32 KiB): 40 KiB leaves it at 71%.
        println!("cargo:rustc-link-arg-bins=--defsym=__stack_size=40960");
    }
}
