//! PE/COFF images for the loader (ADR-0009): parsing, laying out the image
//! view page by page, and relocating each page as it's filled.
//!
//! Format facts are from Microsoft's public "PE Format" specification. WM5
//! and WM6.x binaries are PE32 with machine `0x01C2` (ARM with Thumb
//! interworking, "ARMV4I") or `0x01C0` (ARM).
//!
//! The loader never copies a whole image. It describes the image as it lies
//! in memory (the *image view*: headers, then each section at its RVA, zero
//! fill elsewhere), and the page cache fills one view page at a time with
//! [`Image::fill_page`], reading only the file ranges that page needs, then
//! applies that page's base relocations with [`Relocations::apply`] (PE
//! groups relocations by 4 KiB page, which makes this cheap). Pages nobody
//! touches are never read or relocated.
//!
//! Every input is untrusted: malformed images give an [`Error`], never a
//! panic or an out-of-bounds access.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

/// Page size of the image view.
pub const PAGE: u32 = 4096;

/// `IMAGE_FILE_MACHINE_ARM`.
pub const MACHINE_ARM: u16 = 0x01C0;
/// `IMAGE_FILE_MACHINE_THUMB` (ARM with Thumb interworking: WM's ARMV4I).
pub const MACHINE_THUMB: u16 = 0x01C2;

/// Most sections an image may have (bounded, ADR-0005 rule 9).
pub const MAX_SECTIONS: usize = 16;

/// Why an image was refused.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub enum Error {
    /// Shorter than its headers say.
    Truncated,
    /// No `MZ` / `PE\0\0` signature.
    NotPe,
    /// Not PE32 (`0x10B`) or not an ARM machine.
    Unsupported,
    /// More than [`MAX_SECTIONS`] sections.
    TooManySections,
    /// A section or directory lies outside the image or overlaps wrongly.
    BadLayout,
    /// A base relocation type the loader doesn't implement.
    BadRelocation,
}

impl Error {
    /// A short name for logs.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Error::Truncated => "truncated",
            Error::NotPe => "not PE",
            Error::Unsupported => "unsupported",
            Error::TooManySections => "too many sections",
            Error::BadLayout => "bad layout",
            Error::BadRelocation => "bad relocation",
        }
    }
}

fn u16_at(b: &[u8], off: usize) -> Result<u16, Error> {
    let s = b.get(off..off + 2).ok_or(Error::Truncated)?;
    Ok(u16::from_le_bytes([s[0], s[1]]))
}

fn u32_at(b: &[u8], off: usize) -> Result<u32, Error> {
    let s = b.get(off..off + 4).ok_or(Error::Truncated)?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// Section characteristics bits.
pub mod scn {
    /// Contains code.
    pub const CODE: u32 = 0x0000_0020;
    /// Uninitialised data.
    pub const UNINIT: u32 = 0x0000_0080;
    /// Shared between processes.
    pub const SHARED: u32 = 0x1000_0000;
    /// Executable.
    pub const EXECUTE: u32 = 0x2000_0000;
    /// Readable.
    pub const READ: u32 = 0x4000_0000;
    /// Writable.
    pub const WRITE: u32 = 0x8000_0000;
}

/// One section.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(test, derive(Debug))]
pub struct Section {
    /// Name (up to 8 bytes, NUL padded).
    pub name: [u8; 8],
    /// Size in memory.
    pub virtual_size: u32,
    /// Address relative to the image base.
    pub rva: u32,
    /// Bytes stored in the file (the rest of `virtual_size` is zero).
    pub raw_size: u32,
    /// Where the stored bytes start in the file.
    pub raw_offset: u32,
    /// [`scn`] bits.
    pub characteristics: u32,
}

impl Section {
    /// Size in memory (some linkers leave `virtual_size` 0: use the raw size).
    #[must_use]
    pub fn mem_size(&self) -> u32 {
        if self.virtual_size == 0 {
            self.raw_size
        } else {
            self.virtual_size
        }
    }
}

/// A data directory entry.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(test, derive(Debug))]
pub struct Dir {
    /// Relative virtual address.
    pub rva: u32,
    /// Size in bytes.
    pub size: u32,
}

/// Data directory indices.
pub mod dir {
    /// Exports.
    pub const EXPORT: usize = 0;
    /// Imports.
    pub const IMPORT: usize = 1;
    /// Base relocations.
    pub const BASERELOC: usize = 5;
}

/// A parsed image (from its headers alone).
#[derive(Clone, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub struct Image {
    /// Machine type ([`MACHINE_ARM`] or [`MACHINE_THUMB`]).
    pub machine: u16,
    /// COFF characteristics (`0x2000` = DLL, `0x0001` = relocations stripped).
    pub characteristics: u16,
    /// Entry point RVA (0 for resource-only DLLs).
    pub entry: u32,
    /// Preferred load address.
    pub image_base: u32,
    /// Size of the image in memory.
    pub image_size: u32,
    /// Size of the headers in the file (also mapped at the base).
    pub header_size: u32,
    /// Section alignment in memory.
    pub section_alignment: u32,
    /// Subsystem (9 = Windows CE GUI).
    pub subsystem: u16,
    /// Data directories (missing ones are zero).
    pub dirs: [Dir; 16],
    sections: [Section; MAX_SECTIONS],
    count: usize,
}

impl Image {
    /// Parses the headers. `headers` must hold at least the DOS header, PE
    /// headers and section table (the first `SizeOfHeaders` bytes always do).
    pub fn parse(headers: &[u8]) -> Result<Self, Error> {
        if headers.get(0..2) != Some(b"MZ") {
            return Err(Error::NotPe);
        }
        let pe = u32_at(headers, 0x3C)? as usize;
        match headers.get(pe..pe.saturating_add(4)) {
            None => return Err(Error::Truncated),
            Some(sig) if sig != b"PE\0\0" => return Err(Error::NotPe),
            Some(_) => {}
        }
        let coff = pe + 4;
        let machine = u16_at(headers, coff)?;
        let count = usize::from(u16_at(headers, coff + 2)?);
        let opt_size = usize::from(u16_at(headers, coff + 16)?);
        let characteristics = u16_at(headers, coff + 18)?;
        let opt = coff + 20;
        if u16_at(headers, opt)? != 0x10B || !matches!(machine, MACHINE_ARM | MACHINE_THUMB) {
            return Err(Error::Unsupported);
        }
        if count > MAX_SECTIONS {
            return Err(Error::TooManySections);
        }
        let mut dirs = [Dir::default(); 16];
        let ndirs = (u32_at(headers, opt + 92)? as usize).min(16);
        if opt_size < 96 + ndirs * 8 {
            return Err(Error::BadLayout);
        }
        for (i, d) in dirs.iter_mut().enumerate().take(ndirs) {
            *d = Dir {
                rva: u32_at(headers, opt + 96 + i * 8)?,
                size: u32_at(headers, opt + 100 + i * 8)?,
            };
        }
        let mut img = Image {
            machine,
            characteristics,
            entry: u32_at(headers, opt + 16)?,
            image_base: u32_at(headers, opt + 28)?,
            section_alignment: u32_at(headers, opt + 32)?,
            image_size: u32_at(headers, opt + 56)?,
            header_size: u32_at(headers, opt + 60)?,
            subsystem: u16_at(headers, opt + 68)?,
            dirs,
            sections: [Section::default(); MAX_SECTIONS],
            count,
        };
        let table = opt + opt_size;
        for (i, slot) in img.sections[..count].iter_mut().enumerate() {
            let at = table + i * 40;
            let raw = headers.get(at..at + 40).ok_or(Error::Truncated)?;
            let mut name = [0u8; 8];
            name.copy_from_slice(&raw[..8]);
            *slot = Section {
                name,
                virtual_size: u32_at(raw, 8)?,
                rva: u32_at(raw, 12)?,
                raw_size: u32_at(raw, 16)?,
                raw_offset: u32_at(raw, 20)?,
                characteristics: u32_at(raw, 36)?,
            };
        }
        img.validate()?;
        Ok(img)
    }

    fn validate(&self) -> Result<(), Error> {
        if self.image_size == 0 || self.header_size > self.image_size {
            return Err(Error::BadLayout);
        }
        let mut prev_end = self.header_size;
        for s in self.sections() {
            let end = s.rva.checked_add(s.mem_size()).ok_or(Error::BadLayout)?;
            // Sections must ascend without overlapping and fit the image.
            if s.rva < prev_end || end > self.image_size.next_multiple_of(PAGE) {
                return Err(Error::BadLayout);
            }
            s.raw_offset
                .checked_add(s.raw_size)
                .ok_or(Error::BadLayout)?;
            prev_end = end;
        }
        for d in &self.dirs {
            if d.size != 0 {
                let end = d.rva.checked_add(d.size).ok_or(Error::BadLayout)?;
                if end > self.image_size.next_multiple_of(PAGE) {
                    return Err(Error::BadLayout);
                }
            }
        }
        Ok(())
    }

    /// The sections, in address order.
    pub fn sections(&self) -> impl Iterator<Item = &Section> {
        self.sections[..self.count].iter()
    }

    /// Whether this is a DLL.
    #[must_use]
    pub fn is_dll(&self) -> bool {
        self.characteristics & 0x2000 != 0
    }

    /// Number of pages in the image view.
    #[must_use]
    pub fn pages(&self) -> u32 {
        self.image_size.div_ceil(PAGE)
    }

    /// Fills `out` with view page `page` (its RVA is `page * PAGE`), not yet
    /// relocated. `read(file_offset, buf)` reads from the image file.
    pub fn fill_page<E>(
        &self,
        page: u32,
        out: &mut [u8; PAGE as usize],
        read: impl FnMut(u32, &mut [u8]) -> Result<(), E>,
    ) -> Result<(), FillError<E>> {
        if page >= self.pages() {
            return Err(FillError::Image(Error::BadLayout));
        }
        self.read_view(page * PAGE, out, read)
    }

    /// Reads `out.len()` bytes of the image view from `rva`, unrelocated:
    /// zero, then the headers and every section's file bytes that fall in
    /// the range. Past the end of the image reads as zero.
    pub fn read_view<E>(
        &self,
        rva: u32,
        out: &mut [u8],
        mut read: impl FnMut(u32, &mut [u8]) -> Result<(), E>,
    ) -> Result<(), FillError<E>> {
        out.fill(0);
        let lo = rva;
        let hi = rva
            .checked_add(out.len() as u32)
            .ok_or(FillError::Image(Error::BadLayout))?;
        // The headers occupy RVA 0..header_size, from file offset 0.
        let pieces = core::iter::once((0, self.header_size, 0)).chain(
            self.sections()
                .filter(|s| s.characteristics & scn::UNINIT == 0)
                .map(|s| (s.rva, s.raw_size.min(s.mem_size()), s.raw_offset)),
        );
        for (rva, len, file) in pieces {
            let (a, b) = (rva.max(lo), (rva + len).min(hi));
            if a < b {
                let dst = &mut out[(a - lo) as usize..(b - lo) as usize];
                read(file + (a - rva), dst).map_err(FillError::Read)?;
            }
        }
        Ok(())
    }

    /// The memory protection of view page `page`: the union of everything
    /// that lies in it (headers are read-only). `None` for a page no section
    /// covers (it stays unmapped: a guard).
    #[must_use]
    pub fn page_protection(&self, page: u32) -> Option<Protection> {
        let lo = page * PAGE;
        let hi = lo + PAGE;
        let mut p: Option<Protection> = (lo < self.header_size).then_some(Protection::default());
        for s in self.sections() {
            let end = s.rva + s.mem_size().max(1);
            if s.rva < hi && end > lo {
                let q = p.get_or_insert_with(Protection::default);
                let c = s.characteristics;
                q.write |= c & scn::WRITE != 0;
                q.exec |= c & (scn::EXECUTE | scn::CODE) != 0;
                q.shared |= c & scn::SHARED != 0;
            }
        }
        p
    }
}

/// A page's protection in the image view.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(test, derive(Debug))]
pub struct Protection {
    /// Writable (private copy-on-write, unless `shared`).
    pub write: bool,
    /// Executable.
    pub exec: bool,
    /// Writes are shared by all processes (`IMAGE_SCN_MEM_SHARED`).
    pub shared: bool,
}

/// Why a page couldn't be filled.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub enum FillError<E> {
    /// The image is malformed.
    Image(Error),
    /// Reading the file failed.
    Read(E),
}

/// An image's base relocations, indexed by page.
///
/// PE groups relocations into blocks of one 4 KiB page each, so relocating a
/// page needs only its own block, plus the previous page's for a 32-bit
/// value that straddles the boundary.
pub struct Relocations<'a> {
    data: &'a [u8],
}

/// `IMAGE_REL_BASED_ABSOLUTE`: padding.
const REL_ABSOLUTE: u16 = 0;
/// `IMAGE_REL_BASED_HIGHLOW`: add the delta to a 32-bit value.
const REL_HIGHLOW: u16 = 3;

impl<'a> Relocations<'a> {
    /// The relocation directory's contents (read from the image view).
    /// Checks every block's framing and every entry's type up front, so
    /// [`apply`](Self::apply) can't meet anything it doesn't understand.
    pub fn new(data: &'a [u8]) -> Result<Self, Error> {
        let r = Self { data };
        let mut prev_page = None;
        for block in r.blocks() {
            let (page_rva, entries) = block?;
            if !page_rva.is_multiple_of(PAGE) || prev_page.is_some_and(|p| page_rva <= p) {
                return Err(Error::BadLayout);
            }
            prev_page = Some(page_rva);
            for e in entries.as_chunks::<2>().0 {
                let e = u16::from_le_bytes([e[0], e[1]]);
                if !matches!(e >> 12, REL_ABSOLUTE | REL_HIGHLOW) {
                    return Err(Error::BadRelocation);
                }
            }
        }
        Ok(r)
    }

    fn blocks(&self) -> impl Iterator<Item = Result<(u32, &'a [u8]), Error>> {
        let data = self.data;
        let mut at = 0usize;
        core::iter::from_fn(move || {
            if at + 8 > data.len() {
                return None;
            }
            let page = u32_at(data, at).ok()?;
            let size = u32_at(data, at + 4).ok()? as usize;
            if size == 0 && page == 0 {
                return None; // zero padding after the last block
            }
            if size < 8 || !size.is_multiple_of(2) || at + size > data.len() {
                at = data.len();
                return Some(Err(Error::BadLayout));
            }
            let entries = &data[at + 8..at + size];
            at += size;
            Some(Ok((page, entries)))
        })
    }

    fn block_for(&self, page_rva: u32) -> Option<&'a [u8]> {
        self.blocks()
            .flatten()
            .find(|&(p, _)| p == page_rva)
            .map(|(_, e)| e)
    }

    /// Adds `delta` (load address minus preferred base) to every 32-bit
    /// value in view page `page`, given the page's bytes in `out`. A value
    /// that straddles into the next page has its first bytes here: they're
    /// relocated using `next` (the next page's first 4 bytes, as filled) for
    /// the carry; a value straddling in from the previous page gets its top
    /// bytes fixed the same way. Each page is relocated independently, so
    /// pages can be filled in any order.
    pub fn apply(
        &self,
        page: u32,
        out: &mut [u8; PAGE as usize],
        prev: [u8; 4],
        next: [u8; 4],
        delta: u32,
    ) {
        if delta == 0 {
            return;
        }
        let rva = page * PAGE;
        if let Some(entries) = self.block_for(rva) {
            for e in entries.as_chunks::<2>().0 {
                let e = u16::from_le_bytes([e[0], e[1]]);
                if e >> 12 != REL_HIGHLOW {
                    continue;
                }
                let off = usize::from(e & 0xFFF);
                // The value's four bytes: the ones in this page plus, past the
                // end, the next page's.
                let mut v = [0u8; 4];
                for (i, b) in v.iter_mut().enumerate() {
                    *b = if off + i < PAGE as usize {
                        out[off + i]
                    } else {
                        next[off + i - PAGE as usize]
                    };
                }
                let fixed = u32::from_le_bytes(v).wrapping_add(delta).to_le_bytes();
                for (i, b) in fixed.iter().enumerate() {
                    if off + i < PAGE as usize {
                        out[off + i] = *b;
                    }
                }
            }
        }
        // A value from the previous page's block that ends in this page.
        if page > 0
            && let Some(entries) = self.block_for(rva - PAGE)
        {
            for e in entries.as_chunks::<2>().0 {
                let e = u16::from_le_bytes([e[0], e[1]]);
                let off = usize::from(e & 0xFFF);
                if e >> 12 != REL_HIGHLOW || off + 4 <= PAGE as usize {
                    continue;
                }
                let head = PAGE as usize - off; // bytes in the previous page
                let mut v = [0u8; 4];
                v[..head].copy_from_slice(&prev[4 - head..]);
                v[head..].copy_from_slice(&out[..4 - head]);
                let fixed = u32::from_le_bytes(v).wrapping_add(delta).to_le_bytes();
                out[..4 - head].copy_from_slice(&fixed[head..]);
            }
        }
    }
}

/// One imported function.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub enum Import<'a> {
    /// By ordinal (many CE apps import `coredll` this way).
    Ordinal(u16),
    /// By name, with the exporter's hint index.
    Name {
        /// Where to start looking in the exporter's name table.
        hint: u16,
        /// The name (ASCII, without the NUL).
        name: &'a [u8],
    },
}

/// Longest DLL or function name the loader accepts.
pub const MAX_NAME: usize = 64;

/// Calls `f(dll, import, iat_rva)` for every imported function, where
/// `iat_rva` is the import address table slot the loader fills with the
/// function's address. `read(rva, buf)` reads the image view (unrelocated is
/// fine: import tables hold RVAs, never absolute addresses).
///
/// Uses the import lookup table when present and the IAT otherwise (CE
/// linkers often leave the lookup table out). Bounded: at most 64 DLLs and
/// 4096 functions per DLL, or [`Error::BadLayout`].
pub fn for_each_import<E>(
    img: &Image,
    mut read: impl FnMut(u32, &mut [u8]) -> Result<(), E>,
    mut f: impl FnMut(&[u8], Import<'_>, u32),
) -> Result<(), FillError<E>> {
    let d = img.dirs[dir::IMPORT];
    if d.size == 0 {
        return Ok(());
    }
    for i in 0..64u32 {
        let at = d.rva + i * 20;
        let lookup = view_u32(img, at, &mut read)?;
        let name_rva = view_u32(img, at + 12, &mut read)?;
        let iat = view_u32(img, at + 16, &mut read)?;
        if lookup == 0 && name_rva == 0 && iat == 0 {
            return Ok(());
        }
        let mut dll = [0u8; MAX_NAME];
        let dll_len = read_name(img, name_rva, &mut dll, &mut read)?;
        let table = if lookup != 0 { lookup } else { iat };
        for k in 0..4096u32 {
            let mut b = [0u8; 4];
            img.read_view(table + k * 4, &mut b, &mut read)?;
            let thunk = u32::from_le_bytes(b);
            if thunk == 0 {
                break;
            }
            if k == 4095 {
                return Err(FillError::Image(Error::BadLayout));
            }
            let slot = iat + k * 4;
            if thunk & 0x8000_0000 != 0 {
                f(&dll[..dll_len], Import::Ordinal(thunk as u16), slot);
            } else {
                let mut h = [0u8; 2];
                img.read_view(thunk, &mut h, &mut read)?;
                let mut name = [0u8; MAX_NAME];
                let n = read_name(img, thunk + 2, &mut name, &mut read)?;
                f(
                    &dll[..dll_len],
                    Import::Name {
                        hint: u16::from_le_bytes(h),
                        name: &name[..n],
                    },
                    slot,
                );
            }
        }
    }
    Err(FillError::Image(Error::BadLayout))
}

fn view_u32<E>(
    img: &Image,
    rva: u32,
    read: &mut impl FnMut(u32, &mut [u8]) -> Result<(), E>,
) -> Result<u32, FillError<E>> {
    let mut b = [0u8; 4];
    img.read_view(rva, &mut b, &mut *read)?;
    Ok(u32::from_le_bytes(b))
}

/// Reads a NUL-terminated name at `rva` into `out`, returning its length.
fn read_name<E>(
    img: &Image,
    rva: u32,
    out: &mut [u8; MAX_NAME],
    read: &mut impl FnMut(u32, &mut [u8]) -> Result<(), E>,
) -> Result<usize, FillError<E>> {
    img.read_view(rva, out, &mut *read)?;
    out.iter()
        .position(|&b| b == 0)
        .ok_or(FillError::Image(Error::BadLayout))
}

#[cfg(test)]
mod tests;
