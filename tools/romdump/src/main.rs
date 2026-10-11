//! `romdump` command line.
//!
//! ```text
//! romdump info    <image.nb0>                  partitions and summary
//! romdump list    <image.nb0>                  every file and module in IMGFS
//! romdump extract <image.nb0> <out> [names…]   extract files/modules (all if no names)
//! romdump header  <image.nb0> <module>         hex dump of a module's header blob
//! ```

#![forbid(unsafe_code)]

use std::path::Path;
use std::process::ExitCode;

use romdump::imgfs::{Imgfs, Item, Kind};
use romdump::{Error, nb0, pe};

const USAGE: &str = "usage:
  romdump info    <image.nb0>
  romdump list    <image.nb0>
  romdump extract <image.nb0> <out-dir> [name ...]
  romdump header  <image.nb0> <module>";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["info", image] => info(image),
        ["list", image] => list(image),
        ["extract", image, out, ref names @ ..] => extract(image, out, names),
        ["header", image, module] => header(image, module),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("romdump: {e}");
            ExitCode::FAILURE
        }
    }
}

fn load(path: &str) -> Result<Vec<u8>, Error> {
    Ok(std::fs::read(path)?)
}

fn with_imgfs<T>(path: &str, f: impl FnOnce(&Imgfs<'_>) -> Result<T, Error>) -> Result<T, Error> {
    let image = load(path)?;
    let part = nb0::partitions(&image)?
        .into_iter()
        .find(|p| p.kind == nb0::TYPE_IMGFS)
        .ok_or_else(|| Error::Format("no IMGFS partition".into()))?;
    let fs = Imgfs::parse(&image[part.offset..part.offset + part.len])?;
    f(&fs)
}

fn info(path: &str) -> Result<(), Error> {
    let image = load(path)?;
    println!("{path}: {} bytes", image.len());
    for p in nb0::partitions(&image)? {
        println!(
            "  {:#04x} {:<20} offset {:#010x} size {:#010x}",
            p.kind,
            p.kind_name(),
            p.offset,
            p.len
        );
    }
    with_imgfs(path, |fs| {
        let modules = fs
            .items
            .iter()
            .filter(|i| matches!(i.kind, Kind::Module { .. }))
            .count();
        println!(
            "  IMGFS: {} items ({} modules, {} files), compression {}",
            fs.items.len(),
            modules,
            fs.items.len() - modules,
            fs.compression
        );
        Ok(())
    })
}

fn list(path: &str) -> Result<(), Error> {
    with_imgfs(path, |fs| {
        for item in &fs.items {
            match &item.kind {
                Kind::File => println!("file    {:>9}  {}", item.size, item.name),
                Kind::Module { sections } => {
                    let total: usize = sections.iter().map(|s| s.size).sum();
                    println!(
                        "module  {:>9}  {}  ({} sections)",
                        total,
                        item.name,
                        sections.len()
                    );
                }
            }
        }
        Ok(())
    })
}

fn extract(path: &str, out: &str, names: &[&str]) -> Result<(), Error> {
    let out = Path::new(out);
    std::fs::create_dir_all(out)?;
    with_imgfs(path, |fs| {
        let selected: Vec<&Item> = if names.is_empty() {
            fs.items.iter().collect()
        } else {
            names
                .iter()
                .map(|n| {
                    fs.find(n)
                        .ok_or_else(|| Error::Format(format!("{n}: not found")))
                })
                .collect::<Result<_, _>>()?
        };
        let mut failed = 0;
        for item in selected {
            match extract_one(fs, item) {
                Ok(bytes) => {
                    std::fs::write(out.join(safe_name(&item.name)), bytes)?;
                    println!("extracted {}", item.name);
                }
                Err(e) => {
                    failed += 1;
                    eprintln!("skipped {}: {e}", item.name);
                }
            }
        }
        if failed > 0 {
            eprintln!("{failed} item(s) could not be extracted");
        }
        Ok(())
    })
}

fn extract_one(fs: &Imgfs<'_>, item: &Item) -> Result<Vec<u8>, Error> {
    match &item.kind {
        Kind::File => fs.read(item.data, item.size),
        // `.ROM` / `.VM` are module-typed metadata with no sections: keep raw.
        Kind::Module { sections } if sections.is_empty() => fs.read(item.data, item.size),
        Kind::Module { sections } => {
            let header = fs.read(item.data, item.size)?;
            let bodies = sections
                .iter()
                .map(|s| fs.read(s.data, s.size))
                .collect::<Result<Vec<_>, _>>()?;
            pe::rebuild(&header, &bodies)
        }
    }
}

/// Keeps extracted names inside the output directory.
fn safe_name(name: &str) -> String {
    name.chars()
        .map(|c| if "\\/:*?\"<>|".contains(c) { '_' } else { c })
        .collect()
}

fn header(path: &str, module: &str) -> Result<(), Error> {
    with_imgfs(path, |fs| {
        let item = fs
            .find(module)
            .ok_or_else(|| Error::Format(format!("{module}: not found")))?;
        let blob = fs.read(item.data, item.size)?;
        for (i, row) in blob.chunks(16).enumerate() {
            let hex: Vec<String> = row.iter().map(|b| format!("{b:02x}")).collect();
            println!("{:04x}: {}", i * 16, hex.join(" "));
        }
        if let Kind::Module { sections } = &item.kind {
            for s in sections {
                println!("section {} size {:#x}", s.name, s.size);
            }
        }
        Ok(())
    })
}
