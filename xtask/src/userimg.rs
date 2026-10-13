//! User process images (ADR-0009, ADR-0016): a program linked by
//! `hal/user/user.x` (an ELF with its relocations kept) becomes a PE file
//! the kernel's loader maps at any base: `.text` (read + execute), `.rdata`
//! (read), `.data` (read + write, `.bss` included) and `.reloc` (a
//! `HIGHLOW` fix-up for every absolute address in them).
//!
//! The ELF is our own linker's output, but read as untrusted anyway: every
//! offset and size is bounds-checked, and a relocation type this packer
//! doesn't understand is an error, never silently dropped.

use std::path::Path;

use rp_kernel_pe::build::{self, Header, SectionSpec};
use rp_kernel_pe::{Dir, MACHINE_ARM, scn};

/// The base `user.x` links at (the PE's preferred base).
pub const BASE: u32 = 0x0001_0000;

/// The programs packaged, by crate name (`target/.../release/<name>.pe`).
pub const IMAGES: &[&str] = &["rp-display-host"];

const SHT_REL: u32 = 9;
const SHT_RELA: u32 = 4;
/// ARM relocation types: absolute words (fixed up) and position-relative
/// ones (nothing to do when the whole image moves).
const R_ARM_NONE: u32 = 0;
const R_ARM_ABS32: u32 = 2;
const R_ARM_REL32: u32 = 3;
const R_ARM_CALL: u32 = 28;
const R_ARM_JUMP24: u32 = 29;
const R_ARM_TARGET1: u32 = 38;
const R_ARM_V4BX: u32 = 40;
const R_ARM_PREL31: u32 = 42;

struct Section<'a> {
    name: &'a str,
    kind: u32,
    addr: u32,
    offset: usize,
    size: usize,
    info: usize,
}

fn u16_at(d: &[u8], at: usize) -> Result<u16, String> {
    d.get(at..at + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or_else(|| format!("ELF truncated at {at:#x}"))
}

fn u32_at(d: &[u8], at: usize) -> Result<u32, String> {
    d.get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| format!("ELF truncated at {at:#x}"))
}

fn sections(d: &[u8]) -> Result<(u32, Vec<Section<'_>>), String> {
    if d.get(..6) != Some(&[0x7F, b'E', b'L', b'F', 1, 1][..]) {
        return Err("not a little-endian ELF32 file".into());
    }
    if u16_at(d, 18)? != 40 {
        return Err("not an ARM ELF".into());
    }
    let entry = u32_at(d, 24)?;
    let shoff = u32_at(d, 32)? as usize;
    let shentsize = usize::from(u16_at(d, 46)?);
    let shnum = usize::from(u16_at(d, 48)?);
    let shstrndx = usize::from(u16_at(d, 50)?);
    if shentsize < 40 {
        return Err("bad section header size".into());
    }
    let header = |i: usize| -> Result<[u32; 10], String> {
        let at = shoff
            .checked_add(i.checked_mul(shentsize).ok_or("overflow")?)
            .ok_or("overflow")?;
        let mut h = [0u32; 10];
        for (k, v) in h.iter_mut().enumerate() {
            *v = u32_at(d, at + 4 * k)?;
        }
        Ok(h)
    };
    let names = header(shstrndx)?;
    let (names_at, names_len) = (names[4] as usize, names[5] as usize);
    let strtab = d
        .get(names_at..names_at.checked_add(names_len).ok_or("overflow")?)
        .ok_or("section names outside the file")?;
    let mut out = Vec::with_capacity(shnum);
    for i in 0..shnum {
        let h = header(i)?;
        let name_at = h[0] as usize;
        let name = strtab
            .get(name_at..)
            .and_then(|s| s.split(|&b| b == 0).next())
            .and_then(|s| std::str::from_utf8(s).ok())
            .ok_or("bad section name")?;
        out.push(Section {
            name,
            kind: h[1],
            addr: h[3],
            offset: h[4] as usize,
            size: h[5] as usize,
            info: h[7] as usize,
        });
    }
    Ok((entry, out))
}

/// The PE file for the ELF `elf`.
pub fn package(elf: &[u8]) -> Result<Vec<u8>, String> {
    let (entry, secs) = sections(elf)?;
    let want = [
        (".text", scn::CODE | scn::EXECUTE | scn::READ),
        (".rdata", scn::READ),
        (".data", scn::READ | scn::WRITE),
    ];
    let mut kept: Vec<(usize, &[u8], u32, u32)> = Vec::new(); // (index, bytes, rva, flags)
    for (name, flags) in want {
        let Some(i) = secs.iter().position(|s| s.name == name) else {
            continue;
        };
        let s = &secs[i];
        if s.size == 0 {
            continue;
        }
        let rva = s
            .addr
            .checked_sub(BASE)
            .filter(|r| r.is_multiple_of(0x1000) && *r >= 0x1000)
            .ok_or_else(|| format!("{name} at {:#x}: not page-aligned past the base", s.addr))?;
        let bytes = elf
            .get(s.offset..s.offset.checked_add(s.size).ok_or("overflow")?)
            .ok_or_else(|| format!("{name} outside the file"))?;
        kept.push((i, bytes, rva, flags));
    }
    // Every absolute word in a kept section, by address.
    let mut fixups: Vec<u32> = Vec::new();
    for r in secs
        .iter()
        .filter(|s| s.kind == SHT_REL || s.kind == SHT_RELA)
    {
        if !kept.iter().any(|k| k.0 == r.info) {
            continue; // debug information and the like
        }
        if r.kind == SHT_RELA {
            return Err(format!("{}: RELA relocations not expected", r.name));
        }
        let table = elf
            .get(r.offset..r.offset.checked_add(r.size).ok_or("overflow")?)
            .ok_or_else(|| format!("{} outside the file", r.name))?;
        for e in table.as_chunks::<8>().0 {
            let at = u32::from_le_bytes([e[0], e[1], e[2], e[3]]);
            let kind = e[4].into();
            match kind {
                R_ARM_ABS32 | R_ARM_TARGET1 => fixups.push(at),
                R_ARM_NONE | R_ARM_REL32 | R_ARM_CALL | R_ARM_JUMP24 | R_ARM_V4BX
                | R_ARM_PREL31 => {}
                other => {
                    return Err(format!(
                        "{}: relocation type {other} at {at:#x} not supported",
                        r.name
                    ));
                }
            }
        }
    }
    fixups.sort_unstable();
    fixups.dedup();
    // `.reloc`: one block per page, after the last section.
    let end = kept
        .iter()
        .map(|k| (k.2 + k.1.len() as u32).next_multiple_of(0x1000))
        .max()
        .ok_or("no code")?;
    let mut rel = vec![0u8; 16 + fixups.len() * 4 + 8 * fixups.len()];
    let mut len = 0;
    let mut i = 0;
    while i < fixups.len() {
        let rva = fixups[i] - BASE;
        let page = rva & !0xFFF;
        let mut offsets = Vec::new();
        while i < fixups.len() && (fixups[i] - BASE) & !0xFFF == page {
            offsets.push(((fixups[i] - BASE) & 0xFFF) as u16);
            i += 1;
        }
        len += build::reloc_block(&mut rel[len..], page, &offsets).map_err(|e| e.name())?;
    }
    rel.truncate(len);
    let mut specs: Vec<SectionSpec<'_>> = kept
        .iter()
        .map(|&(i, bytes, rva, flags)| SectionSpec {
            name: secs[i].name.as_bytes(),
            rva,
            vsize: bytes.len() as u32,
            data: bytes,
            characteristics: flags,
        })
        .collect();
    if !rel.is_empty() {
        specs.push(SectionSpec {
            name: b".reloc",
            rva: end,
            vsize: rel.len() as u32,
            data: &rel,
            characteristics: scn::READ,
        });
    }
    let header = Header {
        machine: MACHINE_ARM,
        image_base: BASE,
        entry: entry.checked_sub(BASE).ok_or("entry below the base")?,
        dll: false,
        relocs: (!rel.is_empty()).then_some(Dir {
            rva: end,
            size: rel.len() as u32,
        }),
    };
    let total: usize = 0x400 + specs.iter().map(|s| s.data.len() + 0x200).sum::<usize>();
    let mut out = vec![0u8; total];
    let n = build::write(&mut out, &header, &specs).map_err(|e| e.name())?;
    out.truncate(n);
    Ok(out)
}

/// Packages every program in [`IMAGES`] built in `release` (next to it,
/// `<name>.pe`), reporting its sizes.
pub fn package_all(release: &Path) -> Result<(), String> {
    for name in IMAGES {
        let elf_path = release.join(name);
        let elf = std::fs::read(&elf_path).map_err(|e| format!("{}: {e}", elf_path.display()))?;
        let pe = package(&elf).map_err(|e| format!("{name}: {e}"))?;
        let out = release.join(format!("{name}.pe"));
        std::fs::write(&out, &pe).map_err(|e| format!("{}: {e}", out.display()))?;
        eprintln!("    {name}.pe: {} bytes", pe.len());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rp_kernel_pe::Image;

    /// A minimal ARM ELF: `.text` at the base's first page with `code`, and
    /// `.rel.text` with `rels` (offset in `.text`, type).
    fn elf(code: &[u8], text_addr: u32, rels: &[(u32, u8)]) -> Vec<u8> {
        let names = b"\0.text\0.rel.text\0.shstrtab\0";
        let mut rel = Vec::new();
        for &(off, kind) in rels {
            rel.extend_from_slice(&(text_addr + off).to_le_bytes());
            rel.extend_from_slice(&u32::from(kind).to_le_bytes());
        }
        let text_at = 52;
        let rel_at = text_at + code.len();
        let names_at = rel_at + rel.len();
        let sh_at = (names_at + names.len()).next_multiple_of(4);
        let mut d = vec![0u8; sh_at + 4 * 40];
        d[..6].copy_from_slice(&[0x7F, b'E', b'L', b'F', 1, 1]);
        d[18..20].copy_from_slice(&40u16.to_le_bytes());
        d[24..28].copy_from_slice(&text_addr.to_le_bytes());
        d[32..36].copy_from_slice(&(sh_at as u32).to_le_bytes());
        d[46..48].copy_from_slice(&40u16.to_le_bytes());
        d[48..50].copy_from_slice(&4u16.to_le_bytes());
        d[50..52].copy_from_slice(&3u16.to_le_bytes());
        d[text_at..rel_at].copy_from_slice(code);
        d[rel_at..names_at].copy_from_slice(&rel);
        d[names_at..names_at + names.len()].copy_from_slice(names);
        let mut sh = |i: usize, f: [u32; 10]| {
            for (k, v) in f.iter().enumerate() {
                let at = sh_at + i * 40 + 4 * k;
                d[at..at + 4].copy_from_slice(&v.to_le_bytes());
            }
        };
        sh(
            1,
            [
                1,
                1,
                6,
                text_addr,
                text_at as u32,
                code.len() as u32,
                0,
                0,
                4,
                0,
            ],
        );
        sh(
            2,
            [
                7,
                SHT_REL,
                0,
                0,
                rel_at as u32,
                rel.len() as u32,
                0,
                1,
                4,
                8,
            ],
        );
        sh(
            3,
            [17, 3, 0, 0, names_at as u32, names.len() as u32, 0, 0, 1, 0],
        );
        d
    }

    #[test]
    fn absolute_words_become_highlow_fixups_and_branches_dont() {
        let code = [0u8; 64];
        let pe = package(&elf(&code, BASE + 0x1000, &[(8, 2), (20, 28), (40, 38)])).unwrap();
        let image = Image::parse(&pe).ok().expect("the packed image parses");
        assert_eq!((image.image_base, image.entry), (BASE, 0x1000));
        let d = image.dirs[rp_kernel_pe::dir::BASERELOC];
        let s = image.sections().find(|s| s.rva == d.rva).unwrap();
        let rel = &pe[s.raw_offset as usize..][..d.size as usize];
        // One block for page 0x1000: offsets 8 and 40 (types 2 and 38), not
        // the branch (28).
        assert_eq!(&rel[..4], &0x1000u32.to_le_bytes());
        let entries: Vec<u16> = rel[8..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .filter(|&e| e != 0)
            .collect();
        assert_eq!(entries, [0x3008, 0x3028]);
        assert!(!image.writable_code());
    }

    #[test]
    fn hostile_or_unexpected_input_is_refused() {
        let code = [0u8; 16];
        assert!(package(b"not an elf").is_err());
        assert!(package(&elf(&code, BASE + 0x1000, &[])[..60]).is_err());
        // A relocation type this packer doesn't understand.
        assert!(package(&elf(&code, BASE + 0x1000, &[(0, 43)])).is_err());
        // Code not page-aligned, or below the image's first page.
        assert!(package(&elf(&code, BASE + 0x1004, &[])).is_err());
        assert!(package(&elf(&code, BASE, &[])).is_err());
        // Section offsets past the end.
        let mut bad = elf(&code, BASE + 0x1000, &[]);
        let sh_at = u32::from_le_bytes(bad[32..36].try_into().unwrap()) as usize;
        bad[sh_at + 40 + 16..sh_at + 40 + 20].copy_from_slice(&0xFFFF_FFF0u32.to_le_bytes());
        assert!(package(&bad).is_err());
        // Every truncation of a valid file fails cleanly (no panic).
        let good = elf(&code, BASE + 0x1000, &[(4, 2)]);
        for n in 0..good.len() {
            let _ = package(&good[..n]);
        }
    }
}
