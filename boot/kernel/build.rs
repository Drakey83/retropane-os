//! Links the image with the shared `boot/link.x` when building for the X51v,
//! with an abort-mode stack: the kernel resolves page faults in abort mode.

fn main() {
    println!("cargo:rerun-if-changed=../link.x");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("none") {
        let dir = std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo");
        println!("cargo:rustc-link-arg-bins=-T{dir}/../link.x");
        println!("cargo:rustc-link-arg-bins=--defsym=__abt_stack_size=4096");
    }
}
