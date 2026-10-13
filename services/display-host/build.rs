//! Links the host as a user process image (`hal/user/user.x`), keeping its
//! relocations so `cargo xtask` can package it as a PE file the kernel
//! loads at any base.

fn main() {
    println!("cargo:rerun-if-changed=../../hal/user/user.x");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("none") {
        let dir = std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo");
        println!("cargo:rustc-link-arg-bins=-T{dir}/../../hal/user/user.x");
        println!("cargo:rustc-link-arg-bins=--emit-relocs");
    }
}
