//! Links the image with the shared `boot/link.x` and, when `cargo xtask`
//! built them (`RP_C_LIBS`, xtask/src/cc.rs and xtask/src/module.rs), the
//! table of what the modules import and picolibc.

fn main() {
    println!("cargo:rerun-if-changed=../../boot/link.x");
    println!("cargo:rerun-if-env-changed=RP_C_LIBS");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("none") {
        return;
    }
    let dir = std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo");
    println!("cargo:rustc-link-arg-bins=-T{dir}/../../boot/link.x");
    // The image area (928 KiB) is for what is loaded: the C library the
    // modules call is most of this image. Its zeroed RAM and its stack go
    // to the 3 MiB of free RAM after it (boot/link.x).
    println!("cargo:rustc-link-arg-bins=--defsym=__ram_at=0xA8100000");
    // A game's stack: the engine's buffers are locals of several KiB each,
    // nested deep. 2 MiB; every image reports its deepest use at its end.
    println!("cargo:rustc-link-arg-bins=--defsym=__stack_size=0x200000");
    if let Ok(libs) = std::env::var("RP_C_LIBS") {
        println!("cargo:rerun-if-changed={libs}/librprunnertable.a");
        println!("cargo:rerun-if-changed={libs}/libc.a");
        println!("cargo:rustc-link-search=native={libs}");
        // The table first: it names what of the C library is kept.
        println!("cargo:rustc-link-lib=static=rprunnertable");
        println!("cargo:rustc-link-lib=static=c");
    }
}
