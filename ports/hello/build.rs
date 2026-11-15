//! Links the image with the shared `boot/link.x` and, when `cargo xtask`
//! built them (`RP_C_LIBS`, xtask/src/cc.rs), the C program and picolibc.

fn main() {
    println!("cargo:rerun-if-changed=../../boot/link.x");
    println!("cargo:rerun-if-env-changed=RP_C_LIBS");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("none") {
        return;
    }
    let dir = std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo");
    println!("cargo:rustc-link-arg-bins=-T{dir}/../../boot/link.x");
    // A C program's stack: its buffers are locals (this one's are small;
    // a game's are not), so more than an image's usual 16 KiB.
    println!("cargo:rustc-link-arg-bins=--defsym=__stack_size=0x20000");
    if let Ok(libs) = std::env::var("RP_C_LIBS") {
        println!("cargo:rerun-if-changed={libs}/libhello.a");
        println!("cargo:rerun-if-changed={libs}/libc.a");
        println!("cargo:rustc-link-search=native={libs}");
        // The program first, then the library it calls into.
        println!("cargo:rustc-link-lib=static=hello");
        println!("cargo:rustc-link-lib=static=c");
    }
}
