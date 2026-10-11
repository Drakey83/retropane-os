//! Rebuilding PE files from IMGFS modules.
//!
//! An IMGFS module stores Windows CE's ROM loader structures instead of PE
//! headers (layouts per the public Windows CE documentation, verified on
//! X51v modules):
//!
//! * `e32_rom` (0x70 bytes): section count, image flags, entry RVA, image base,
//!   subsystem version, stack size, image size, COM descriptor (sect14),
//!   timestamp, nine `{rva, size}` data directories (`EXP IMP RES EXC SEC FIX
//!   DEB IMD MSP`) and the subsystem.
//! * `o32_rom[n]` (24 bytes each): virtual size, RVA, stored size, data
//!   pointer, real (load) address, flags.
//!
//! The ROM builder may have moved writable sections to a RAM address
//! (`realaddr != base + rva`) and patched code to match. To make the rebuilt
//! file disassemble sensibly at its image base, those references are moved
//! back using the module's own base relocations, as the community `recmod`
//! tool did.

use crate::Error;
use crate::bytes::{u16_at, u32_at};

const E32_SIZE: usize = 0x70;
const O32_SIZE: usize = 24;
const DIRS: usize = 9;
const FIX: usize = 5; // base relocations

/// CE-only section flag (ROM compression); meaningless in a rebuilt file.
const SCN_COMPRESSED: u32 = 0x0000_2000;
const SCN_CODE: u32 = 0x0000_0020;
const SCN_WRITE: u32 = 0x8000_0000;

const FILE_ALIGN: usize = 0x200;
const SECT_ALIGN: u32 = 0x1000;
/// ARM Thumb-interworking, the machine type of WM5/6 ARMV4I binaries.
const MACHINE_THUMB: u16 = 0x01C2;

/// `e32_rom`: the module's image header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct E32 {
    /// Number of sections.
    pub objcnt: u16,
    /// PE characteristics.
    pub imageflags: u16,
    /// Entry point RVA.
    pub entryrva: u32,
    /// Image base.
    pub vbase: u32,
    /// Subsystem version.
    pub subsys_version: (u16, u16),
    /// Stack reserve.
    pub stackmax: u32,
    /// Image size.
    pub vsize: u32,
    /// COM descriptor directory.
    pub sect14: (u32, u32),
    /// Link timestamp.
    pub timestamp: u32,
    /// Data directories `EXP IMP RES EXC SEC FIX DEB IMD MSP` as `(rva, size)`.
    pub dirs: [(u32, u32); DIRS],
    /// PE subsystem.
    pub subsys: u16,
}

/// `o32_rom`: one section header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct O32 {
    /// Virtual size.
    pub vsize: u32,
    /// RVA.
    pub rva: u32,
    /// Stored size.
    pub psize: u32,
    /// Address the ROM builder placed the section at.
    pub realaddr: u32,
    /// Section flags.
    pub flags: u32,
}

/// Parses a module header blob into `e32_rom` and its `o32_rom` entries.
pub fn parse_header(h: &[u8]) -> Result<(E32, Vec<O32>), Error> {
    let mut dirs = [(0, 0); DIRS];
    for (i, d) in dirs.iter_mut().enumerate() {
        *d = (u32_at(h, 0x24 + i * 8)?, u32_at(h, 0x28 + i * 8)?);
    }
    let e32 = E32 {
        objcnt: u16_at(h, 0x00)?,
        imageflags: u16_at(h, 0x02)?,
        entryrva: u32_at(h, 0x04)?,
        vbase: u32_at(h, 0x08)?,
        subsys_version: (u16_at(h, 0x0C)?, u16_at(h, 0x0E)?),
        stackmax: u32_at(h, 0x10)?,
        vsize: u32_at(h, 0x14)?,
        sect14: (u32_at(h, 0x18)?, u32_at(h, 0x1C)?),
        timestamp: u32_at(h, 0x20)?,
        dirs,
        subsys: u16_at(h, 0x6C)?,
    };
    if e32.objcnt == 0 || e32.objcnt > 32 {
        return Err(Error::Format(format!(
            "implausible section count {}",
            e32.objcnt
        )));
    }
    let o32 = (0..usize::from(e32.objcnt))
        .map(|i| {
            let o = E32_SIZE + i * O32_SIZE;
            Ok(O32 {
                vsize: u32_at(h, o)?,
                rva: u32_at(h, o + 4)?,
                psize: u32_at(h, o + 8)?,
                realaddr: u32_at(h, o + 16)?,
                flags: u32_at(h, o + 20)?,
            })
        })
        .collect::<Result<Vec<_>, Error>>()?;
    Ok((e32, o32))
}

/// Rebuilds a PE image from a module header blob and its section bodies.
pub fn rebuild(header: &[u8], bodies: &[Vec<u8>]) -> Result<Vec<u8>, Error> {
    let (e32, o32) = parse_header(header)?;
    if bodies.len() != o32.len() {
        return Err(Error::Format(format!(
            "header lists {} sections but {} are stored",
            o32.len(),
            bodies.len()
        )));
    }
    let mut bodies = bodies.to_vec();
    unmove_sections(&e32, &o32, &mut bodies)?;
    Ok(write_pe(&e32, &o32, &bodies))
}

/// Moves references to relocated sections back to `vbase + rva`.
fn unmove_sections(e32: &E32, o32: &[O32], bodies: &mut [Vec<u8>]) -> Result<(), Error> {
    let moved: Vec<(u32, u32, u32)> = o32
        .iter()
        .filter(|s| s.realaddr != 0 && s.realaddr != e32.vbase.wrapping_add(s.rva))
        .map(|s| {
            (
                s.realaddr,
                s.vsize.max(s.psize),
                e32.vbase.wrapping_add(s.rva),
            )
        })
        .collect();
    if moved.is_empty() {
        return Ok(());
    }
    let (fix_rva, fix_size) = e32.dirs[FIX];
    let relocs = if fix_size == 0 {
        None
    } else {
        read_rva(o32, bodies, fix_rva, fix_size as usize)
    };
    match relocs.filter(|r| standard_relocs(r, e32.vsize)) {
        Some(relocs) => {
            for target in highlow_targets(&relocs)? {
                if let Some((si, off)) = locate(o32, bodies, target) {
                    unmove_word(&mut bodies[si], off, &moved);
                }
            }
        }
        // IMGFS modules often carry Windows CE's compact relocation encoding
        // instead of PE base relocations (observed: Dell X51v drivers), and
        // some kitchens strip relocations entirely. Fall back to ARM literal
        // pools: every word loaded by `LDR Rd, [PC, #imm]` that points into a
        // moved section is an address and gets moved back.
        None => {
            for (i, s) in o32.iter().enumerate() {
                if s.flags & SCN_CODE == 0 {
                    continue;
                }
                for off in arm_literal_pool_words(&bodies[i]) {
                    unmove_word(&mut bodies[i], off, &moved);
                }
            }
        }
    }
    Ok(())
}

/// Moves the word at `off` back to its home section if it points into a moved one.
fn unmove_word(body: &mut [u8], off: usize, moved: &[(u32, u32, u32)]) {
    let Some(word) = body.get(off..off + 4) else {
        return;
    };
    let v = u32::from_le_bytes(word.try_into().expect("4 bytes"));
    if let Some(&(real, _, home)) = moved.iter().find(|&&(r, l, _)| v >= r && v - r < l) {
        body[off..off + 4].copy_from_slice(&(v - real + home).to_le_bytes());
    }
}

/// Whether `relocs` looks like PE base relocations: every block's page lies
/// inside the image and every block size is sane.
fn standard_relocs(relocs: &[u8], image_size: u32) -> bool {
    let mut pos = 0;
    let mut blocks = 0;
    while pos + 8 <= relocs.len() {
        let page = u32::from_le_bytes(relocs[pos..pos + 4].try_into().expect("4 bytes"));
        let size =
            u32::from_le_bytes(relocs[pos + 4..pos + 8].try_into().expect("4 bytes")) as usize;
        if size == 0 && page == 0 {
            break; // padding
        }
        if page >= image_size.max(1)
            || size < 8
            || !size.is_multiple_of(2)
            || pos + size > relocs.len()
        {
            return false;
        }
        pos += size;
        blocks += 1;
    }
    blocks > 0
}

/// Offsets (within an ARM code section) of words referenced by
/// `LDR Rd, [PC, #+/-imm12]` instructions, deduplicated.
fn arm_literal_pool_words(code: &[u8]) -> Vec<usize> {
    let mut out = std::collections::BTreeSet::new();
    for off in (0..code.len().saturating_sub(3)).step_by(4) {
        let ins = u32::from_le_bytes(code[off..off + 4].try_into().expect("4 bytes"));
        // cond 01 0 P=1 U 0 W=0 L=1 Rn=1111: LDR (literal), word, any condition except NV.
        if ins & 0x0F7F_0000 != 0x051F_0000 || ins >> 28 == 0xF {
            continue;
        }
        let imm = (ins & 0xFFF) as usize;
        let target = if ins & (1 << 23) != 0 {
            off.checked_add(8 + imm)
        } else {
            (off + 8).checked_sub(imm)
        };
        if let Some(t) = target.filter(|&t| t.is_multiple_of(4) && t + 4 <= code.len()) {
            out.insert(t);
        }
    }
    out.into_iter().collect()
}

/// RVAs of every `IMAGE_REL_BASED_HIGHLOW` fixup in a `.reloc` blob.
fn highlow_targets(relocs: &[u8]) -> Result<Vec<u32>, Error> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos + 8 <= relocs.len() {
        let page = u32_at(relocs, pos)?;
        let size = u32_at(relocs, pos + 4)? as usize;
        if size < 8 || pos + size > relocs.len() {
            break;
        }
        for e in (pos + 8..pos + size).step_by(2) {
            let entry = u16_at(relocs, e)?;
            if entry >> 12 == 3 {
                out.push(page + u32::from(entry & 0x0FFF));
            }
        }
        pos += size;
    }
    Ok(out)
}

fn locate(o32: &[O32], bodies: &[Vec<u8>], rva: u32) -> Option<(usize, usize)> {
    o32.iter().zip(bodies).enumerate().find_map(|(i, (s, b))| {
        let off = rva.checked_sub(s.rva)? as usize;
        (off < b.len()).then_some((i, off))
    })
}

/// Reads up to `len` bytes at `rva`. Directory sizes may cover a section's
/// virtual size while the ROM builder stored fewer bytes (observed: `.reloc`
/// with size 0x1000 but 0x428 bytes stored), so the read stops at the end of
/// the stored data.
fn read_rva(o32: &[O32], bodies: &[Vec<u8>], rva: u32, len: usize) -> Option<Vec<u8>> {
    let (i, off) = locate(o32, bodies, rva)?;
    let end = (off + len).min(bodies[i].len());
    Some(bodies[i][off..end].to_vec())
}

/// Picks a conventional section name from flags and data directories.
fn section_name(e32: &E32, s: &O32) -> &'static [u8; 8] {
    let in_dir = |d: usize| {
        let (rva, size) = e32.dirs[d];
        size != 0 && rva >= s.rva && rva < s.rva + s.vsize.max(s.psize)
    };
    if s.flags & SCN_CODE != 0 {
        b".text\0\0\0"
    } else if in_dir(FIX) && s.flags & SCN_WRITE == 0 && e32.dirs[FIX].0 == s.rva {
        b".reloc\0\0"
    } else if in_dir(2) && e32.dirs[2].0 == s.rva {
        b".rsrc\0\0\0"
    } else if in_dir(3) && e32.dirs[3].0 == s.rva {
        b".pdata\0\0"
    } else if s.flags & SCN_WRITE != 0 {
        b".data\0\0\0"
    } else {
        b".rdata\0\0"
    }
}

/// Clamps a data directory to the bytes actually stored, so PE readers don't
/// run past a section's raw data. For `.reloc`, the exact length is the sum
/// of its blocks.
fn stored_dir_size(o32: &[O32], bodies: &[Vec<u8>], dir: usize, rva: u32, size: u32) -> u32 {
    const SEC: usize = 4; // holds a file offset, not an RVA
    if size == 0 || dir == SEC {
        return size;
    }
    let Some(data) = read_rva(o32, bodies, rva, size as usize) else {
        return size;
    };
    if dir == FIX {
        let mut len = 0usize;
        while len + 8 <= data.len() {
            let block =
                u32::from_le_bytes(data[len + 4..len + 8].try_into().expect("4 bytes")) as usize;
            if block < 8 || len + block > data.len() {
                break;
            }
            len += block;
        }
        return len as u32;
    }
    data.len() as u32
}

fn align(v: usize, a: usize) -> usize {
    v.div_ceil(a) * a
}

fn write_pe(e32: &E32, o32: &[O32], bodies: &[Vec<u8>]) -> Vec<u8> {
    let nsect = o32.len();
    let pe_off = 0x80usize;
    let opt_size = 0xE0usize;
    let headers = align(pe_off + 4 + 20 + opt_size + nsect * 40, FILE_ALIGN);

    let mut raw_ptrs = Vec::with_capacity(nsect);
    let mut pos = headers;
    for b in bodies {
        raw_ptrs.push(pos);
        pos += align(b.len(), FILE_ALIGN);
    }
    let mut f = vec![0u8; pos];
    let put16 = |f: &mut [u8], o: usize, v: u16| f[o..o + 2].copy_from_slice(&v.to_le_bytes());
    let put32 = |f: &mut [u8], o: usize, v: u32| f[o..o + 4].copy_from_slice(&v.to_le_bytes());

    // DOS header.
    f[0..2].copy_from_slice(b"MZ");
    put32(&mut f, 0x3C, pe_off as u32);

    // COFF header.
    f[pe_off..pe_off + 4].copy_from_slice(b"PE\0\0");
    let c = pe_off + 4;
    put16(&mut f, c, MACHINE_THUMB);
    put16(&mut f, c + 2, nsect as u16);
    put32(&mut f, c + 4, e32.timestamp);
    put16(&mut f, c + 16, opt_size as u16);
    put16(&mut f, c + 18, e32.imageflags);

    // Optional header (PE32).
    let o = c + 20;
    let code_size: u32 = o32
        .iter()
        .filter(|s| s.flags & SCN_CODE != 0)
        .map(|s| s.vsize)
        .sum();
    let base_of_code = o32
        .iter()
        .find(|s| s.flags & SCN_CODE != 0)
        .map_or(0, |s| s.rva);
    put16(&mut f, o, 0x010B);
    put32(&mut f, o + 4, code_size);
    put32(&mut f, o + 16, e32.entryrva);
    put32(&mut f, o + 20, base_of_code);
    put32(&mut f, o + 28, e32.vbase);
    put32(&mut f, o + 32, SECT_ALIGN);
    put32(&mut f, o + 36, FILE_ALIGN as u32);
    put16(&mut f, o + 40, e32.subsys_version.0);
    put16(&mut f, o + 42, e32.subsys_version.1);
    put16(&mut f, o + 48, e32.subsys_version.0);
    put16(&mut f, o + 50, e32.subsys_version.1);
    // e32 vsize can exclude trailing sections (observed: .reloc), so derive it.
    let image_size = o32
        .iter()
        .map(|s| s.rva as usize + align(s.vsize.max(s.psize) as usize, SECT_ALIGN as usize))
        .max()
        .unwrap_or(0)
        .max(e32.vsize as usize);
    put32(&mut f, o + 56, image_size as u32);
    put32(&mut f, o + 60, headers as u32);
    put16(&mut f, o + 68, e32.subsys);
    put32(&mut f, o + 72, e32.stackmax);
    put32(&mut f, o + 76, 0x1000);
    put32(&mut f, o + 80, 0x0010_0000);
    put32(&mut f, o + 84, 0x1000);
    put32(&mut f, o + 92, 16);
    for (i, &(rva, size)) in e32.dirs.iter().enumerate() {
        put32(&mut f, o + 96 + i * 8, rva);
        put32(
            &mut f,
            o + 100 + i * 8,
            stored_dir_size(o32, bodies, i, rva, size),
        );
    }
    put32(&mut f, o + 96 + 14 * 8, e32.sect14.0);
    put32(&mut f, o + 100 + 14 * 8, e32.sect14.1);

    // Section table and bodies.
    let st = o + opt_size;
    for (i, (s, b)) in o32.iter().zip(bodies).enumerate() {
        let h = st + i * 40;
        f[h..h + 8].copy_from_slice(section_name(e32, s));
        put32(&mut f, h + 8, s.vsize);
        put32(&mut f, h + 12, s.rva);
        put32(&mut f, h + 16, align(b.len(), FILE_ALIGN) as u32);
        put32(&mut f, h + 20, raw_ptrs[i] as u32);
        put32(&mut f, h + 36, s.flags & !SCN_COMPRESSED);
        f[raw_ptrs[i]..raw_ptrs[i] + b.len()].copy_from_slice(b);
    }
    fix_debug_directory(&mut f, e32, o32, &raw_ptrs);
    f
}

/// Debug directory entries carry file offsets from the original (pre-ROM)
/// file. Point them at our layout, or clear them if their data wasn't kept.
fn fix_debug_directory(f: &mut [u8], e32: &E32, o32: &[O32], raw_ptrs: &[usize]) {
    const DEB: usize = 6;
    let (rva, size) = e32.dirs[DEB];
    let to_file = |rva: u32| {
        o32.iter().zip(raw_ptrs).find_map(|(s, &raw)| {
            let off = rva.checked_sub(s.rva)?;
            (off < s.psize).then_some(raw + off as usize)
        })
    };
    let Some(dir) = to_file(rva) else { return };
    for e in (0..size as usize / 28).map(|i| dir + i * 28) {
        if e + 28 > f.len() {
            break;
        }
        let data_rva = u32::from_le_bytes(f[e + 20..e + 24].try_into().expect("4 bytes"));
        let fixed = if data_rva == 0 {
            None
        } else {
            to_file(data_rva)
        };
        match fixed {
            Some(ptr) => f[e + 24..e + 28].copy_from_slice(&(ptr as u32).to_le_bytes()),
            None => {
                f[e + 16..e + 20].fill(0); // SizeOfData
                f[e + 24..e + 28].fill(0); // PointerToRawData
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A two-section module: code at RVA 0x1000, data at 0x2000 moved by the
    /// ROM builder to 0x0180_0000, plus a relocation in the code that points
    /// into the moved data.
    fn sample() -> (Vec<u8>, Vec<Vec<u8>>) {
        let vbase = 0x0200_0000u32;
        let mut h = vec![0u8; E32_SIZE + 3 * O32_SIZE];
        let p16 = |h: &mut Vec<u8>, o: usize, v: u16| h[o..o + 2].copy_from_slice(&v.to_le_bytes());
        let p32 = |h: &mut Vec<u8>, o: usize, v: u32| h[o..o + 4].copy_from_slice(&v.to_le_bytes());
        p16(&mut h, 0x00, 3);
        p16(&mut h, 0x02, 0x2102);
        p32(&mut h, 0x04, 0x1004);
        p32(&mut h, 0x08, vbase);
        p32(&mut h, 0x14, 0x4000);
        p32(&mut h, 0x24 + FIX * 8, 0x3000); // FIX dir
        p32(&mut h, 0x28 + FIX * 8, 12);
        p16(&mut h, 0x6C, 9);
        let sects = [
            (0x10u32, 0x1000u32, 0x10u32, vbase + 0x1000, 0x6000_2020u32),
            (0x10, 0x2000, 0x10, 0x0180_0000, 0xC000_0040),
            (12, 0x3000, 12, vbase + 0x3000, 0x4200_0040),
        ];
        for (i, &(vs, rva, ps, real, fl)) in sects.iter().enumerate() {
            let o = E32_SIZE + i * O32_SIZE;
            p32(&mut h, o, vs);
            p32(&mut h, o + 4, rva);
            p32(&mut h, o + 8, ps);
            p32(&mut h, o + 16, real);
            p32(&mut h, o + 20, fl);
        }
        let mut code = vec![0u8; 0x10];
        code[8..12].copy_from_slice(&0x0180_0004u32.to_le_bytes()); // -> moved data
        let data = vec![0xAA; 0x10];
        let mut reloc = Vec::new();
        reloc.extend_from_slice(&0x1000u32.to_le_bytes()); // page RVA
        reloc.extend_from_slice(&12u32.to_le_bytes()); // block size
        reloc.extend_from_slice(&(0x3000u16 | 8).to_le_bytes()); // HIGHLOW at +8
        reloc.extend_from_slice(&0u16.to_le_bytes()); // padding
        (h, vec![code, data, reloc])
    }

    #[test]
    fn parses_e32_and_o32() {
        let (h, _) = sample();
        let (e32, o32) = parse_header(&h).unwrap();
        assert_eq!(e32.objcnt, 3);
        assert_eq!(e32.vbase, 0x0200_0000);
        assert_eq!(e32.subsys, 9);
        assert_eq!(o32[1].realaddr, 0x0180_0000);
    }

    #[test]
    fn rejects_section_count_mismatch() {
        let (h, mut b) = sample();
        b.pop();
        assert!(rebuild(&h, &b).is_err());
    }

    #[test]
    fn rejects_implausible_header() {
        assert!(parse_header(&[0u8; E32_SIZE]).is_err());
    }

    #[test]
    fn rebuilt_file_is_a_valid_pe() {
        let (h, b) = sample();
        let pe = rebuild(&h, &b).unwrap();
        assert_eq!(&pe[0..2], b"MZ");
        let off = u32_at(&pe, 0x3C).unwrap() as usize;
        assert_eq!(&pe[off..off + 4], b"PE\0\0");
        assert_eq!(u16_at(&pe, off + 4).unwrap(), MACHINE_THUMB);
        assert_eq!(u16_at(&pe, off + 6).unwrap(), 3);
        assert_eq!(u32_at(&pe, off + 24 + 28).unwrap(), 0x0200_0000); // ImageBase
        assert_eq!(u32_at(&pe, off + 24 + 16).unwrap(), 0x1004); // entry point
        let st = off + 24 + 0xE0;
        assert_eq!(&pe[st..st + 5], b".text");
        assert_eq!(&pe[st + 40..st + 45], b".data");
        assert_eq!(&pe[st + 80..st + 86], b".reloc");
        // CE-only "compressed" flag is stripped.
        assert_eq!(u32_at(&pe, st + 36).unwrap() & SCN_COMPRESSED, 0);
    }

    #[test]
    fn references_to_moved_data_are_moved_back() {
        let (h, b) = sample();
        let pe = rebuild(&h, &b).unwrap();
        let off = u32_at(&pe, 0x3C).unwrap() as usize;
        let st = off + 24 + 0xE0;
        let code_raw = u32_at(&pe, st + 20).unwrap() as usize;
        // 0x0180_0004 (moved) -> vbase + 0x2000 + 4
        assert_eq!(u32_at(&pe, code_raw + 8).unwrap(), 0x0200_2004);
    }

    #[test]
    fn relocation_directory_larger_than_stored_data_is_clamped() {
        let (mut h, b) = sample();
        h[0x28 + FIX * 8..0x2C + FIX * 8].copy_from_slice(&0x1000u32.to_le_bytes());
        let pe = rebuild(&h, &b).unwrap();
        let off = u32_at(&pe, 0x3C).unwrap() as usize;
        let code_raw = u32_at(&pe, off + 24 + 0xE0 + 20).unwrap() as usize;
        assert_eq!(u32_at(&pe, code_raw + 8).unwrap(), 0x0200_2004);
    }

    #[test]
    fn size_of_image_covers_every_section() {
        let (h, b) = sample();
        let pe = rebuild(&h, &b).unwrap();
        let off = u32_at(&pe, 0x3C).unwrap() as usize;
        // .reloc at 0x3000 needs the image to extend to 0x4000.
        assert!(u32_at(&pe, off + 24 + 56).unwrap() >= 0x4000);
    }

    #[test]
    fn reloc_directory_is_trimmed_to_its_blocks() {
        let (mut h, b) = sample();
        h[0x28 + FIX * 8..0x2C + FIX * 8].copy_from_slice(&0x1000u32.to_le_bytes());
        let pe = rebuild(&h, &b).unwrap();
        let off = u32_at(&pe, 0x3C).unwrap() as usize;
        assert_eq!(u32_at(&pe, off + 24 + 100 + FIX * 8).unwrap(), 12);
    }

    #[test]
    fn stripped_relocations_still_rebuild() {
        let (mut h, mut b) = sample();
        // Directory says 0x3000, but drop the .reloc section entirely.
        h[0x00..0x02].copy_from_slice(&2u16.to_le_bytes());
        b.pop();
        let pe = rebuild(&h[..E32_SIZE + 2 * O32_SIZE], &b).unwrap();
        let off = u32_at(&pe, 0x3C).unwrap() as usize;
        let code_raw = u32_at(&pe, off + 24 + 0xE0 + 20).unwrap() as usize;
        // Reference left at the RAM address.
        assert_eq!(u32_at(&pe, code_raw + 8).unwrap(), 0x0180_0004);
    }

    #[test]
    fn nonstandard_relocs_fall_back_to_literal_pools() {
        let (h, mut b) = sample();
        // Replace .reloc with bytes that aren't PE base relocations.
        b[2] = vec![0x00, 0x00, 0xAD, 0x01, 0x9D, 0x90, 0x22, 0x9F, 0, 0, 0, 0];
        // Code: LDR r0, [pc, #0] at +0 -> literal at +8 (moved data address);
        // +12 holds the same value but is not a literal, so it must be untouched.
        let mut code = vec![0u8; 0x10];
        code[0..4].copy_from_slice(&0xE59F_0000u32.to_le_bytes());
        code[8..12].copy_from_slice(&0x0180_0004u32.to_le_bytes());
        code[12..16].copy_from_slice(&0x0180_0004u32.to_le_bytes());
        b[0] = code;
        let pe = rebuild(&h, &b).unwrap();
        let off = u32_at(&pe, 0x3C).unwrap() as usize;
        let code_raw = u32_at(&pe, off + 24 + 0xE0 + 20).unwrap() as usize;
        assert_eq!(u32_at(&pe, code_raw + 8).unwrap(), 0x0200_2004);
        assert_eq!(u32_at(&pe, code_raw + 12).unwrap(), 0x0180_0004);
    }

    #[test]
    fn literal_pool_scanner_handles_both_directions() {
        let mut code = vec![0u8; 0x20];
        code[0x10..0x14].copy_from_slice(&0xE59F_1004u32.to_le_bytes()); // ldr r1,[pc,#4] -> 0x1C
        code[0x14..0x18].copy_from_slice(&0xE51F_2014u32.to_le_bytes()); // ldr r2,[pc,#-0x14] -> 0x08
        assert_eq!(arm_literal_pool_words(&code), vec![0x08, 0x1C]);
    }

    #[test]
    fn highlow_parser_skips_padding_entries() {
        let mut r = Vec::new();
        r.extend_from_slice(&0x5000u32.to_le_bytes());
        r.extend_from_slice(&12u32.to_le_bytes());
        r.extend_from_slice(&0x3010u16.to_le_bytes());
        r.extend_from_slice(&0u16.to_le_bytes());
        assert_eq!(highlow_targets(&r).unwrap(), vec![0x5010]);
    }
}
