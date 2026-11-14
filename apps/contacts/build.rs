//! Links the app as a user image (`hal/user/user.x`), keeping its
//! relocations so `cargo xtask` packages it as a PE file the shell loads at
//! any address (docs/design/app-scope.md).

fn main() {
    println!("cargo:rerun-if-changed=../../hal/user/user.x");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("none") {
        let dir = std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo");
        println!("cargo:rustc-link-arg-bins=-T{dir}/../../hal/user/user.x");
        println!("cargo:rustc-link-arg-bins=--emit-relocs");
    }
}
