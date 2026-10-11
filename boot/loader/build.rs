//! Links the loader with its own `link.x` (home 0xA8400000) for the X51v.

fn main() {
    println!("cargo:rerun-if-changed=link.x");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("none") {
        let dir = std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo");
        println!("cargo:rustc-link-arg-bins=-T{dir}/link.x");
    }
}
