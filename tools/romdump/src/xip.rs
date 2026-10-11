//! XIP ("execute in place") ROM images: partition types 0x20 and 0x23.
//!
//! Layout (public Windows CE ROM format, verified on X51v images):
//!
//! * At image offset 0x40: `"ECEC"`, then the virtual address of the `ROMHDR`
//!   and its offset within the image.
//! * `ROMHDR` (0x54 bytes): first/last physical address (`physfirst` is the
//!   image's base virtual address), module count (+0x10) and file count (+0x30).
//! * Followed by `TOCentry[nummods]` (32 bytes: attributes, filetime, size,
//!   name VA, `e32_rom` VA, `o32_rom` VA, load offset) and
//!   `FILESentry[numfiles]` (28 bytes: attributes, filetime, real size,
//!   stored size, name VA, data VA).
//! * Module sections live at `o32_dataptr`. Sections flagged
//!   `IMAGE_SCN_COMPRESSED`, and files whose stored size differs from their
//!   real size, use Windows CE's ROM compression, which romdump does not
//!   implement. On the X51v the main XIP (0x23) is entirely uncompressed.

use crate::Error;
use crate::bytes::{slice, u16_at, u32_at};

const ROMHDR_SIZE: usize = 0x54;
const TOC_SIZE: usize = 32;
const FILES_SIZE: usize = 28;
const E32_SIZE: usize = 0x70;
const O32_SIZE: usize = 24;
const SCN_COMPRESSED: u32 = 0x2000;

/// One stored section of an XIP module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XipSection {
    /// Image offset of the stored bytes.
    pub offset: usize,
    /// Stored length.
    pub len: usize,
    /// Stored with ROM compression (not supported).
    pub compressed: bool,
}

/// What an XIP entry is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum XipKind {
    /// A module: `header` is `e32_rom` followed by its `o32_rom` entries (the
    /// same layout IMGFS stores), ready for [`crate::pe::rebuild`].
    Module {
        /// `e32_rom` + `o32_rom[]`.
        header: Vec<u8>,
        /// Stored sections, in `o32` order.
        sections: Vec<XipSection>,
    },
    /// A plain file.
    File {
        /// Image offset of the data.
        offset: usize,
        /// Real size.
        real: usize,
        /// Stored size (differs from `real` when compressed).
        stored: usize,
    },
}

/// A file or module in an XIP image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XipItem {
    /// File name.
    pub name: String,
    /// Module or file.
    pub kind: XipKind,
}

/// A parsed XIP image.
#[derive(Debug)]
pub struct Xip<'a> {
    image: &'a [u8],
    /// Modules first, then files, in ROM order.
    pub items: Vec<XipItem>,
}

impl<'a> Xip<'a> {
    /// Parses the XIP image `image` (a whole partition).
    pub fn parse(image: &'a [u8]) -> Result<Self, Error> {
        if slice(image, 0x40, 4)? != b"ECEC" {
            return Err(Error::Format("not an XIP image (no ECEC signature)".into()));
        }
        let rh = u32_at(image, 0x48)? as usize;
        if rh + ROMHDR_SIZE > image.len() {
            return Err(Error::Truncated(rh));
        }
        let physfirst = u32_at(image, rh + 0x08)?;
        let nummods = u32_at(image, rh + 0x10)? as usize;
        let numfiles = u32_at(image, rh + 0x30)? as usize;
        if nummods > 4096 || numfiles > 4096 {
            return Err(Error::Format("implausible XIP module/file count".into()));
        }
        let v2o = |va: u32| -> Result<usize, Error> {
            let off = va
                .checked_sub(physfirst)
                .ok_or_else(|| Error::Format(format!("address {va:#x} below image base")))?
                as usize;
            if off >= image.len() {
                return Err(Error::Truncated(off));
            }
            Ok(off)
        };
        let name_at = |va: u32| -> Result<String, Error> {
            let off = v2o(va)?;
            let end = image[off..]
                .iter()
                .position(|&b| b == 0)
                .map_or(image.len(), |n| off + n);
            Ok(String::from_utf8_lossy(&image[off..end]).into_owned())
        };

        let mut items = Vec::with_capacity(nummods + numfiles);
        let toc = rh + ROMHDR_SIZE;
        for i in 0..nummods {
            let t = toc + i * TOC_SIZE;
            let name = name_at(u32_at(image, t + 16)?)?;
            let e32 = v2o(u32_at(image, t + 20)?)?;
            let o32 = v2o(u32_at(image, t + 24)?)?;
            let objcnt = usize::from(u16_at(image, e32)?);
            let mut header = slice(image, e32, E32_SIZE)?.to_vec();
            header.extend_from_slice(slice(image, o32, objcnt * O32_SIZE)?);
            let mut sections = Vec::with_capacity(objcnt);
            for j in 0..objcnt {
                let o = o32 + j * O32_SIZE;
                let psize = u32_at(image, o + 8)? as usize;
                let dataptr = u32_at(image, o + 12)?;
                let flags = u32_at(image, o + 20)?;
                let offset = if psize == 0 { 0 } else { v2o(dataptr)? };
                sections.push(XipSection {
                    offset,
                    len: psize,
                    compressed: flags & SCN_COMPRESSED != 0,
                });
            }
            items.push(XipItem {
                name,
                kind: XipKind::Module { header, sections },
            });
        }
        let files = toc + nummods * TOC_SIZE;
        for i in 0..numfiles {
            let f = files + i * FILES_SIZE;
            items.push(XipItem {
                name: name_at(u32_at(image, f + 20)?)?,
                kind: XipKind::File {
                    real: u32_at(image, f + 12)? as usize,
                    stored: u32_at(image, f + 16)? as usize,
                    offset: v2o(u32_at(image, f + 24)?)?,
                },
            });
        }
        Ok(Self { image, items })
    }

    /// Finds an item by name, case-insensitively.
    #[must_use]
    pub fn find(&self, name: &str) -> Option<&XipItem> {
        self.items
            .iter()
            .find(|i| i.name.eq_ignore_ascii_case(name))
    }

    /// Extracts an item: files as-is, modules rebuilt into PE files.
    pub fn extract(&self, item: &XipItem) -> Result<Vec<u8>, Error> {
        match &item.kind {
            XipKind::File {
                offset,
                real,
                stored,
            } => {
                if real != stored {
                    return Err(Error::Format("ROM-compressed file (unsupported)".into()));
                }
                Ok(slice(self.image, *offset, *real)?.to_vec())
            }
            XipKind::Module { header, sections } => {
                let bodies = sections
                    .iter()
                    .map(|s| {
                        if s.compressed {
                            Err(Error::Format("ROM-compressed section (unsupported)".into()))
                        } else {
                            Ok(slice(self.image, s.offset, s.len)?.to_vec())
                        }
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                crate::pe::rebuild(header, &bodies)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: u32 = 0x8C20_0000;

    fn put(d: &mut [u8], off: usize, v: u32) {
        d[off..off + 4].copy_from_slice(&v.to_le_bytes());
    }

    /// One module (`m.dll`, one 16-byte code section) and one file (`f.txt`).
    fn image(compressed: bool) -> Vec<u8> {
        let mut d = vec![0u8; 0x1000];
        d[0x40..0x44].copy_from_slice(b"ECEC");
        let rh = 0x100;
        put(&mut d, 0x48, rh as u32);
        put(&mut d, rh + 0x08, BASE);
        put(&mut d, rh + 0x10, 1);
        put(&mut d, rh + 0x30, 1);
        let toc = rh + ROMHDR_SIZE;
        put(&mut d, toc + 16, BASE + 0x800); // name
        put(&mut d, toc + 20, BASE + 0x400); // e32
        put(&mut d, toc + 24, BASE + 0x480); // o32
        d[0x800..0x805].copy_from_slice(b"m.dll");
        d[0x400..0x402].copy_from_slice(&1u16.to_le_bytes()); // objcnt
        put(&mut d, 0x408, 0x0200_0000); // vbase
        put(&mut d, 0x480, 16); // vsize
        put(&mut d, 0x484, 0x1000); // rva
        put(&mut d, 0x488, 16); // psize
        put(&mut d, 0x48C, BASE + 0x900); // dataptr
        put(
            &mut d,
            0x494,
            0x6000_0020 | if compressed { SCN_COMPRESSED } else { 0 },
        );
        d[0x900..0x910].copy_from_slice(&[0xE1; 16]);
        let f = toc + TOC_SIZE;
        put(&mut d, f + 12, 3);
        put(&mut d, f + 16, if compressed { 2 } else { 3 });
        put(&mut d, f + 20, BASE + 0x810);
        put(&mut d, f + 24, BASE + 0xA00);
        d[0x810..0x815].copy_from_slice(b"f.txt");
        d[0xA00..0xA03].copy_from_slice(b"abc");
        d
    }

    #[test]
    fn lists_modules_and_files() {
        let d = image(false);
        let xip = Xip::parse(&d).unwrap();
        let names: Vec<_> = xip.items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["m.dll", "f.txt"]);
    }

    #[test]
    fn extracts_file_and_rebuilds_module() {
        let d = image(false);
        let xip = Xip::parse(&d).unwrap();
        assert_eq!(xip.extract(xip.find("F.TXT").unwrap()).unwrap(), b"abc");
        let pe = xip.extract(xip.find("m.dll").unwrap()).unwrap();
        assert_eq!(&pe[0..2], b"MZ");
    }

    #[test]
    fn compressed_items_are_reported_not_misread() {
        let d = image(true);
        let xip = Xip::parse(&d).unwrap();
        assert!(xip.extract(xip.find("f.txt").unwrap()).is_err());
        assert!(xip.extract(xip.find("m.dll").unwrap()).is_err());
    }

    #[test]
    fn rejects_non_xip() {
        assert!(Xip::parse(&[0u8; 0x200]).is_err());
    }
}
