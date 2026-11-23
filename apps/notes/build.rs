//! Links the app as a user image (`hal/user/user.x`), keeping its
//! relocations so `cargo xtask` packages it as a PE file the shell loads at
//! any address (docs/design/app-scope.md); and packs each word list in
//! `words/` into the suggestion dictionary's format (`src/words.rs`).

use std::fmt::Write as _;

fn main() {
    println!("cargo:rerun-if-changed=../../hal/user/user.x");
    println!("cargo:rerun-if-changed=words");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("none") {
        let dir = std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo");
        println!("cargo:rustc-link-arg-bins=-T{dir}/../../hal/user/user.x");
        println!("cargo:rustc-link-arg-bins=--emit-relocs");
    }
    let dir = std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo");
    let out = std::env::var("OUT_DIR").expect("set by cargo");
    let mut list = String::from(
        "/// The packed word lists: (language tag, dictionary).\npub static DICTS: &[(&str, &[u8])] = &[\n",
    );
    let mut names: Vec<_> = std::fs::read_dir(format!("{dir}/words"))
        .expect("the word lists")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "txt"))
        .collect();
    names.sort();
    for p in names {
        println!("cargo:rerun-if-changed={}", p.display());
        let lang = p.file_stem().expect("a name").to_string_lossy().to_string();
        let text = std::fs::read_to_string(&p).expect("a word list");
        let packed = pack(&text);
        let file = format!("{out}/{lang}.dict");
        std::fs::write(&file, &packed).expect("written");
        let _ = writeln!(list, "    ({lang:?}, include_bytes!({file:?})),");
    }
    list.push_str("];\n");
    std::fs::write(format!("{out}/dicts.rs"), list).expect("written");
}

/// A word list (most used first) packed: `RPWD1`, the word count and the
/// block count (u16 each), then each block's offset (u16), then the words
/// sorted, sixteen a block, each as the bytes it shares with the word
/// before in its block, its own bytes' count, those bytes, and its rank
/// (0 the most used, 255 the least).
fn pack(text: &str) -> Vec<u8> {
    let mut seen = std::collections::HashSet::new();
    let words: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter(|w| w.len() < 32 && seen.insert(*w))
        .collect();
    let count = words.len();
    let mut ranked: Vec<(&str, u8)> = words
        .iter()
        .enumerate()
        .map(|(i, w)| (*w, (i * 256 / count.max(1)).min(255) as u8))
        .collect();
    ranked.sort();
    let blocks = count.div_ceil(16);
    let mut data = Vec::new();
    let mut offsets = Vec::new();
    for (i, (w, rank)) in ranked.iter().enumerate() {
        if i % 16 == 0 {
            offsets.push(data.len() as u16);
        }
        let shared = if i % 16 == 0 {
            0
        } else {
            let prev = ranked[i - 1].0.as_bytes();
            w.bytes()
                .zip(prev.iter())
                .take_while(|(a, b)| a == *b)
                .count()
        };
        let own = &w.as_bytes()[shared..];
        data.push(shared as u8);
        data.push(own.len() as u8);
        data.extend_from_slice(own);
        data.push(*rank);
    }
    let mut out = b"RPWD1".to_vec();
    out.extend_from_slice(&(count as u16).to_le_bytes());
    out.extend_from_slice(&(blocks as u16).to_le_bytes());
    for o in offsets {
        out.extend_from_slice(&o.to_le_bytes());
    }
    out.extend_from_slice(&data);
    out
}
