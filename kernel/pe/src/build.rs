//! Writes small PE32 images: test fixtures for the loader (host tests and
//! the kernel image's live test), laid out as CE linkers do (headers then
//! sections at 0x200-aligned file offsets; sections 0x1000-aligned in
//! memory). Built from the public PE specification, like the parser.
//!
//! Bounded and allocation-free: it writes into a caller's buffer and
//! refuses anything that doesn't fit.

use super::{Dir, Error, MAX_SECTIONS, dir};

/// One section to write.
#[derive(Clone, Copy)]
pub struct SectionSpec<'a> {
    /// Name (up to 8 bytes).
    pub name: &'a [u8],
    /// Where it lies in memory, relative to the image base (0x1000-aligned).
    pub rva: u32,
    /// Its size in memory (the rest past `data` reads as zero).
    pub vsize: u32,
    /// Its bytes in the file.
    pub data: &'a [u8],
    /// `scn` flags.
    pub characteristics: u32,
}

/// What the image looks like apart from its sections.
#[derive(Clone, Copy)]
pub struct Header {
    /// Machine (`MACHINE_ARM` or `MACHINE_THUMB`).
    pub machine: u16,
    /// Preferred load address.
    pub image_base: u32,
    /// Entry point RVA.
    pub entry: u32,
    /// A DLL rather than an executable.
    pub dll: bool,
    /// The base-relocation directory, if any.
    pub relocs: Option<Dir>,
}

/// Where the PE headers start (after the DOS header).
const PE: usize = 0x80;
/// The optional header: PE32 with 16 data directories.
const OPT: usize = PE + 24;
const OPT_SIZE: usize = 96 + 16 * 8;
const TABLE: usize = OPT + OPT_SIZE;
const FILE_ALIGN: usize = 0x200;
const SECTION_ALIGN: u32 = 0x1000;

/// Writes the image into `out`; returns its length in bytes.
///
/// # Errors
/// [`Error::TooManySections`], [`Error::Truncated`] if `out` is too small,
/// or [`Error::BadLayout`] for a long name or a misaligned section.
pub fn write(out: &mut [u8], h: &Header, sections: &[SectionSpec<'_>]) -> Result<usize, Error> {
    if sections.len() > MAX_SECTIONS {
        return Err(Error::TooManySections);
    }
    let header_size = (TABLE + 40 * sections.len()).next_multiple_of(FILE_ALIGN);
    out.get_mut(..header_size).ok_or(Error::Truncated)?.fill(0);
    let mut w = Out { out };
    w.bytes(0, b"MZ")?;
    w.u32(0x3C, PE as u32)?;
    w.bytes(PE, b"PE\0\0")?;
    w.u16(PE + 4, h.machine)?;
    w.u16(PE + 6, sections.len() as u16)?;
    w.u16(PE + 20, OPT_SIZE as u16)?;
    w.u16(PE + 22, if h.dll { 0x2102 } else { 0x0102 })?;
    w.u16(OPT, 0x10B)?;
    w.u32(OPT + 16, h.entry)?;
    w.u32(OPT + 28, h.image_base)?;
    w.u32(OPT + 32, SECTION_ALIGN)?;
    w.u32(OPT + 36, FILE_ALIGN as u32)?;
    let image_size = sections
        .iter()
        .map(|s| (s.rva + s.vsize).next_multiple_of(SECTION_ALIGN))
        .max()
        .unwrap_or(SECTION_ALIGN);
    w.u32(OPT + 56, image_size)?;
    w.u32(OPT + 60, header_size as u32)?;
    w.u16(OPT + 68, 9)?; // Windows CE GUI
    w.u32(OPT + 92, 16)?;
    if let Some(r) = h.relocs {
        w.u32(OPT + 96 + dir::BASERELOC * 8, r.rva)?;
        w.u32(OPT + 100 + dir::BASERELOC * 8, r.size)?;
    }
    let mut end = header_size;
    for (i, s) in sections.iter().enumerate() {
        if s.name.len() > 8 || !s.rva.is_multiple_of(SECTION_ALIGN) {
            return Err(Error::BadLayout);
        }
        let at = TABLE + i * 40;
        w.bytes(at, s.name)?;
        w.u32(at + 8, s.vsize)?;
        w.u32(at + 12, s.rva)?;
        w.u32(at + 16, s.data.len() as u32)?;
        w.u32(at + 20, if s.data.is_empty() { 0 } else { end as u32 })?;
        w.u32(at + 36, s.characteristics)?;
        let padded = (end + s.data.len()).next_multiple_of(FILE_ALIGN);
        let dst = w.out.get_mut(end..padded).ok_or(Error::Truncated)?;
        dst.fill(0);
        dst[..s.data.len()].copy_from_slice(s.data);
        end = padded;
    }
    Ok(end)
}

/// Little-endian writes, bounds-checked.
struct Out<'a> {
    out: &'a mut [u8],
}

impl Out<'_> {
    fn bytes(&mut self, at: usize, b: &[u8]) -> Result<(), Error> {
        self.out
            .get_mut(at..at + b.len())
            .ok_or(Error::Truncated)?
            .copy_from_slice(b);
        Ok(())
    }
    fn u16(&mut self, at: usize, v: u16) -> Result<(), Error> {
        self.bytes(at, &v.to_le_bytes())
    }
    fn u32(&mut self, at: usize, v: u32) -> Result<(), Error> {
        self.bytes(at, &v.to_le_bytes())
    }
}

/// Writes one base-relocation block (page `page_rva`, 32-bit `HIGHLOW`
/// fix-ups at `offsets` in the page) into `out`, padded to a multiple of
/// 4 bytes; returns its length.
///
/// # Errors
/// [`Error::Truncated`] if `out` is too small; [`Error::BadLayout`] for an
/// offset past the page.
pub fn reloc_block(out: &mut [u8], page_rva: u32, offsets: &[u16]) -> Result<usize, Error> {
    let entries = offsets.len().next_multiple_of(2);
    let size = 8 + 2 * entries;
    let mut w = Out { out };
    w.u32(0, page_rva)?;
    w.u32(4, size as u32)?;
    for i in 0..entries {
        let e = match offsets.get(i) {
            Some(&o) if o < 0x1000 => 0x3000 | o,
            Some(_) => return Err(Error::BadLayout),
            None => 0, // padding (ABSOLUTE)
        };
        w.u16(8 + 2 * i, e)?;
    }
    Ok(size)
}
