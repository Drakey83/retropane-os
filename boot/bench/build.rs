//! Links the image with the shared `boot/link.x` when building for the X51v.

fn main() {
    println!("cargo:rerun-if-changed=../link.x");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("none") {
        let dir = std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo");
        println!("cargo:rustc-link-arg-bins=-T{dir}/../link.x");
        // 32 KiB of stack (the default 16): `main` (9.7 KiB, the suite's
        // checks inlined) and the frame allocator's check (7.2 KiB) went
        // 16,384 of 16,384 bytes deep, over the top of `.bss`, silently
        // until a layout change put the fault handler there (2026-10-26:
        // the bench's own fault check hung). The image reports its deepest
        // stack before its fault, and the emulator check fails past 80%.
        println!("cargo:rustc-link-arg-bins=--defsym=__stack_size=32768");
    }
}
