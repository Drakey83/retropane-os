//! `romdump` command line.
//!
//! ```text
//! romdump info    <image.nb0>                  partitions and summary
//! romdump list    <image.nb0>                  every file and module (IMGFS and XIP)
//! romdump extract <image.nb0> <out> [names…]   extract files/modules (all if no names)
//! romdump header  <image.nb0> <module>         hex dump of an IMGFS module's header blob
//! ```

#![forbid(unsafe_code)]

use std::path::Path;
use std::process::ExitCode;

use romdump::imgfs::{Imgfs, Kind};
use romdump::xip::{Xip, XipKind};
use romdump::{Error, nb0, pe};

const USAGE: &str = "usage:
  romdump info    <image.nb0>
  romdump list    <image.nb0>
  romdump extract <image.nb0> <out-dir> [name ...]
  romdump header  <image.nb0> <module>";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["info", image] => Rom::load(image).and_then(|r| r.info(image)),
        ["list", image] => Rom::load(image).and_then(|r| r.list()),
        ["extract", image, out, ref names @ ..] => {
            Rom::load(image).and_then(|r| r.extract(out, names))
        }
        ["header", image, module] => Rom::load(image).and_then(|r| r.header(module)),
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

/// A loaded image and its partitions.
struct Rom {
    image: Vec<u8>,
    parts: Vec<nb0::Partition>,
}

/// One extractable entry, from whichever partition holds it.
struct Entry {
    source: &'static str,
    name: String,
    module: bool,
    size: usize,
}

impl Rom {
    fn load(path: &str) -> Result<Self, Error> {
        let image = std::fs::read(path)?;
        let parts = nb0::partitions(&image)?;
        Ok(Self { image, parts })
    }

    fn part(&self, p: &nb0::Partition) -> &[u8] {
        &self.image[p.offset..p.offset + p.len]
    }

    fn imgfs(&self) -> Result<Option<Imgfs<'_>>, Error> {
        self.parts
            .iter()
            .find(|p| p.kind == nb0::TYPE_IMGFS)
            .map(|p| Imgfs::parse(self.part(p)))
            .transpose()
    }

    /// XIP partitions, main kernel image (0x23) first.
    fn xips(&self) -> Result<Vec<(&'static str, Xip<'_>)>, Error> {
        let mut out = Vec::new();
        for kind in [nb0::TYPE_XIP, nb0::TYPE_XIP_ULDR] {
            for p in self.parts.iter().filter(|p| p.kind == kind) {
                let label = if kind == nb0::TYPE_XIP { "xip" } else { "uldr" };
                out.push((label, Xip::parse(self.part(p))?));
            }
        }
        Ok(out)
    }

    fn entries(&self) -> Result<Vec<Entry>, Error> {
        let mut out = Vec::new();
        if let Some(fs) = self.imgfs()? {
            for i in &fs.items {
                let (module, size) = match &i.kind {
                    Kind::File => (false, i.size),
                    Kind::Module { sections } => (true, sections.iter().map(|s| s.size).sum()),
                };
                out.push(Entry {
                    source: "imgfs",
                    name: i.name.clone(),
                    module,
                    size,
                });
            }
        }
        for (label, xip) in self.xips()? {
            for i in &xip.items {
                let (module, size) = match &i.kind {
                    XipKind::File { real, .. } => (false, *real),
                    XipKind::Module { sections, .. } => {
                        (true, sections.iter().map(|s| s.len).sum())
                    }
                };
                out.push(Entry {
                    source: label,
                    name: i.name.clone(),
                    module,
                    size,
                });
            }
        }
        Ok(out)
    }

    /// Extracts `name`, preferring IMGFS, then the main XIP, then the loader XIP.
    fn extract_one(&self, name: &str) -> Result<Vec<u8>, Error> {
        if let Some(fs) = self.imgfs()?
            && let Some(item) = fs.find(name)
        {
            return match &item.kind {
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
            };
        }
        let mut last_err = None;
        for (_, xip) in self.xips()? {
            if let Some(item) = xip.find(name) {
                match xip.extract(item) {
                    Ok(bytes) => return Ok(bytes),
                    Err(e) => last_err = Some(e),
                }
            }
        }
        Err(last_err.unwrap_or_else(|| Error::Format(format!("{name}: not found"))))
    }

    fn info(&self, path: &str) -> Result<(), Error> {
        println!("{path}: {} bytes", self.image.len());
        for p in &self.parts {
            println!(
                "  {:#04x} {:<20} offset {:#010x} size {:#010x}",
                p.kind,
                p.kind_name(),
                p.offset,
                p.len
            );
        }
        let entries = self.entries()?;
        for source in ["imgfs", "xip", "uldr"] {
            let (m, f) = entries
                .iter()
                .filter(|e| e.source == source)
                .fold(
                    (0, 0),
                    |(m, f), e| if e.module { (m + 1, f) } else { (m, f + 1) },
                );
            if m + f > 0 {
                println!("  {source:<5}: {m} modules, {f} files");
            }
        }
        Ok(())
    }

    fn list(&self) -> Result<(), Error> {
        for e in self.entries()? {
            let kind = if e.module { "module" } else { "file" };
            println!("{:<5} {kind:<6} {:>9}  {}", e.source, e.size, e.name);
        }
        Ok(())
    }

    fn extract(&self, out: &str, names: &[&str]) -> Result<(), Error> {
        let out = Path::new(out);
        std::fs::create_dir_all(out)?;
        let mut selected: Vec<String> = if names.is_empty() {
            self.entries()?.into_iter().map(|e| e.name).collect()
        } else {
            names.iter().map(|n| (*n).to_string()).collect()
        };
        selected.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        let mut failed = 0;
        for name in &selected {
            match self.extract_one(name) {
                Ok(bytes) => {
                    std::fs::write(out.join(safe_name(name)), bytes)?;
                    println!("extracted {name}");
                }
                Err(e) => {
                    failed += 1;
                    eprintln!("skipped {name}: {e}");
                }
            }
        }
        if failed > 0 {
            eprintln!("{failed} item(s) could not be extracted");
        }
        Ok(())
    }

    fn header(&self, module: &str) -> Result<(), Error> {
        let fs = self
            .imgfs()?
            .ok_or_else(|| Error::Format("no IMGFS partition".into()))?;
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
    }
}

/// Keeps extracted names inside the output directory.
fn safe_name(name: &str) -> String {
    name.chars()
        .map(|c| if "\\/:*?\"<>|".contains(c) { '_' } else { c })
        .collect()
}
