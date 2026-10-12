//! `apiscan <dir>... [--out report.md]`: scans every PE file under the
//! directories (Windows Mobile programs and DLLs, e.g. a `romdump extract`
//! of a ROM, or third-party apps), and writes the ranked CE API backlog.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use rp_apiscan::{Module, Symbol, markdown, rank};
use rp_kernel_pe::{Exports, Image, Import, MAX_NAME, for_each_import};

fn main() -> ExitCode {
    let mut dirs = Vec::new();
    let mut out = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--out" {
            out = args.next();
        } else {
            dirs.push(PathBuf::from(a));
        }
    }
    if dirs.is_empty() {
        eprintln!("usage: apiscan <dir>... [--out report.md]");
        return ExitCode::from(2);
    }
    let mut files = Vec::new();
    for d in &dirs {
        walk(d, &mut files);
    }
    let modules: Vec<Module> = files.iter().filter_map(|p| scan(p)).collect();
    let backlog = rank(&modules);
    let corpus = dirs
        .iter()
        .map(|d| format!("`{}`", d.display()))
        .collect::<Vec<_>>()
        .join(", ");
    let md = markdown(&backlog, &corpus, 8, 60);
    match out {
        Some(path) => {
            if let Err(e) = std::fs::write(&path, &md) {
                eprintln!("apiscan: {path}: {e}");
                return ExitCode::FAILURE;
            }
            eprintln!(
                "apiscan: {} modules, {} functions ranked -> {path}",
                backlog.modules,
                backlog.apis.len()
            );
        }
        None => print!("{md}"),
    }
    ExitCode::SUCCESS
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

/// One file's imports and exports, if it is a PE image.
fn scan(path: &Path) -> Option<Module> {
    let file = std::fs::read(path).ok()?;
    let img = Image::parse(&file).ok()?;
    let read = |off: u32, buf: &mut [u8]| -> Result<(), ()> {
        let src = file.get(off as usize..off as usize + buf.len()).ok_or(())?;
        buf.copy_from_slice(src);
        Ok(())
    };
    let mut imports = Vec::new();
    let _ = for_each_import(&img, read, |dll, imp, _| {
        let dll = String::from_utf8_lossy(dll).to_ascii_lowercase();
        let sym = match imp {
            Import::Ordinal(o) => Symbol::Ordinal(o),
            Import::Name { name, .. } => Symbol::Name(String::from_utf8_lossy(name).into_owned()),
        };
        imports.push((dll, sym));
    });
    let mut exports = BTreeMap::new();
    if let Ok(Some(e)) = Exports::read(&img, read) {
        for i in 0..e.names.min(8192) {
            let mut buf = [0u8; MAX_NAME];
            if let Ok((n, ordinal)) = e.name_at(&img, i, &mut buf, read)
                && let Ok(o) = u16::try_from(ordinal)
            {
                exports.insert(o, String::from_utf8_lossy(&buf[..n]).into_owned());
            }
        }
    }
    let name = path.file_name()?.to_string_lossy().to_ascii_lowercase();
    Some(Module {
        dll: img.is_dll(),
        name,
        imports,
        exports,
    })
}
