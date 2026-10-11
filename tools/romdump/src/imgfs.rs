//! IMGFS: the Windows Mobile 6 ROM file system (partition type 0x25).
//!
//! Layout, as observed on X51v LennySh WM6.1 images (all offsets are relative
//! to the partition start):
//!
//! * **FS header** at 0: GUID `9d2cacf8-d4e3-4d2b-bd30-916ed84f31dc`, then
//!   sizes, the first header block offset (+0x24), the data block size (+0x28)
//!   and the compression name (+0x2C, e.g. `"LZX"`).
//! * **Header blocks**, chained: `u32` signature `0x2F5314CE`, `u32` next block
//!   (0 = end), then fixed-size (0x34-byte) entries.
//! * **Entries**: files (`0xFFFFF6FE`), modules (`0xFFFFFEFE`), module
//!   sections (`0xFFFFF6FD`, linked list from the module) and long-name
//!   storage (`0xFFFFFEFB`).
//! * Each file, module header or section points at a **chunk table** of
//!   `{u16 stored_len, u16 real_len, u32 offset}`. Equal lengths mean the chunk
//!   is stored raw; otherwise the chunk is `{u32 payload_len, u32 real_len}`
//!   followed by an LZX bitstream (64 KiB window, one stream per chunk).

use crate::Error;
use crate::bytes::{slice, u16_at, u32_at, utf16};

/// IMGFS GUID as it appears on disk.
pub const GUID: [u8; 16] = [
    0xF8, 0xAC, 0x2C, 0x9D, 0xE3, 0xD4, 0x2B, 0x4D, 0xBD, 0x30, 0x91, 0x6E, 0xD8, 0x4F, 0x31, 0xDC,
];
const BLOCK_SIGNATURE: u32 = 0x2F53_14CE;

const T_FILE: u32 = 0xFFFF_F6FE;
const T_MODULE: u32 = 0xFFFF_FEFE;
const T_SECTION: u32 = 0xFFFF_F6FD;
const T_NAME: u32 = 0xFFFF_FEFB;

/// Header block size in bytes (one sector per block on all observed images).
const SECTOR: usize = 0x200;

/// Where an item's data lives: a chunk table at `offset`, `len` bytes long.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChunkTable {
    /// Offset of the table within the partition.
    pub offset: usize,
    /// Length of the table in bytes (8 per chunk).
    pub len: usize,
}

/// A module section (`S000`, `S001`, …): one PE section's stored bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    /// Section name as stored (`S000`…).
    pub name: String,
    /// Stored size in bytes.
    pub size: usize,
    /// Its data.
    pub data: ChunkTable,
}

/// What kind of item an entry is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A plain file stored whole.
    File,
    /// A DLL/EXE split into a header blob (`e32_rom` + `o32_rom[]`) and sections.
    Module {
        /// Sections, in order.
        sections: Vec<Section>,
    },
}

/// A file or module in the image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// File name.
    pub name: String,
    /// File attributes (FILE_ATTRIBUTE_*).
    pub attributes: u32,
    /// Size of the file, or of the module's header blob.
    pub size: usize,
    /// The file's data, or the module's header blob.
    pub data: ChunkTable,
    /// File or module.
    pub kind: Kind,
}

/// A parsed IMGFS partition.
#[derive(Debug)]
pub struct Imgfs<'a> {
    part: &'a [u8],
    /// Compression name from the FS header (e.g. `"LZX"`).
    pub compression: String,
    /// All files and modules, in directory order.
    pub items: Vec<Item>,
}

impl<'a> Imgfs<'a> {
    /// Parses the IMGFS partition `part`.
    pub fn parse(part: &'a [u8]) -> Result<Self, Error> {
        if slice(part, 0, 16)? != GUID {
            return Err(Error::Format(
                "not an IMGFS partition (GUID mismatch)".into(),
            ));
        }
        let entry_size = u32_at(part, 0x1C)? as usize;
        if entry_size != 0x34 {
            return Err(Error::Format(format!(
                "unsupported IMGFS entry size {entry_size:#x}"
            )));
        }
        let sectors_per_block = u32_at(part, 0x14)? as usize;
        let first_block = u32_at(part, 0x24)? as usize;
        let compression = String::from_utf8_lossy(slice(part, 0x2C, 4)?)
            .trim_end_matches('\0')
            .to_string();

        let per_block = (sectors_per_block * SECTOR - 8) / entry_size;
        let mut items = Vec::new();
        let mut block = first_block;
        let mut seen = 0usize;
        while block != 0 {
            seen += 1;
            if seen > part.len() / SECTOR {
                return Err(Error::Format("header block chain loops".into()));
            }
            if u32_at(part, block)? != BLOCK_SIGNATURE {
                return Err(Error::Format(format!(
                    "bad header block signature at {block:#x}"
                )));
            }
            for i in 0..per_block {
                let e = block + 8 + i * entry_size;
                match u32_at(part, e)? {
                    T_FILE => items.push(Item {
                        name: read_name(part, e + 0x0C)?,
                        attributes: u32_at(part, e + 0x1C)?,
                        size: u32_at(part, e + 0x18)? as usize,
                        data: table_at(part, e + 0x2C)?,
                        kind: Kind::File,
                    }),
                    T_MODULE => items.push(Item {
                        name: read_name(part, e + 0x0C)?,
                        attributes: u32_at(part, e + 0x1C)?,
                        size: u32_at(part, e + 0x18)? as usize,
                        data: table_at(part, e + 0x2C)?,
                        kind: Kind::Module {
                            sections: read_sections(part, u32_at(part, e + 0x08)? as usize)?,
                        },
                    }),
                    _ => {} // sections, names and free slots are reached via references
                }
            }
            block = u32_at(part, block + 4)? as usize;
        }
        Ok(Self {
            part,
            compression,
            items,
        })
    }

    /// Finds an item by name, case-insensitively.
    #[must_use]
    pub fn find(&self, name: &str) -> Option<&Item> {
        self.items
            .iter()
            .find(|i| i.name.eq_ignore_ascii_case(name))
    }

    /// Reads and decompresses the data behind `table`, truncated to `size`.
    pub fn read(&self, table: ChunkTable, size: usize) -> Result<Vec<u8>, Error> {
        let mut out = Vec::with_capacity(size);
        let raw = slice(self.part, table.offset, table.len)?;
        for entry in raw.as_chunks::<8>().0.iter().map(|e| e.as_slice()) {
            let stored = u16_at(entry, 0)? as usize;
            let real = u16_at(entry, 2)? as usize;
            let offset = u32_at(entry, 4)? as usize;
            if stored == 0 && real == 0 {
                break;
            }
            if out.len() >= size {
                break;
            }
            if stored == real {
                out.extend_from_slice(slice(self.part, offset, real)?);
            } else {
                out.extend_from_slice(&self.decompress_chunk(offset, real)?);
            }
        }
        if out.len() < size {
            return Err(Error::Format(format!(
                "chunk table at {:#x} holds {} bytes, expected {size}",
                table.offset,
                out.len()
            )));
        }
        out.truncate(size);
        Ok(out)
    }

    fn decompress_chunk(&self, offset: usize, real: usize) -> Result<Vec<u8>, Error> {
        if self.compression != "LZX" {
            return Err(Error::Format(format!(
                "unsupported compression {:?}",
                self.compression
            )));
        }
        let payload_len = u32_at(self.part, offset)? as usize;
        let header_real = u32_at(self.part, offset + 4)? as usize;
        if header_real != real {
            return Err(Error::Format(format!(
                "chunk at {offset:#x}: size mismatch"
            )));
        }
        let payload = slice(self.part, offset + 8, payload_len)?;
        let mut lzx = lzxd::Lzxd::new(lzxd::WindowSize::KB64);
        let out = lzx
            .decompress_next(payload, real)
            .map_err(|e| Error::Format(format!("LZX error in chunk at {offset:#x}: {e:?}")))?;
        Ok(out.to_vec())
    }
}

fn table_at(part: &[u8], off: usize) -> Result<ChunkTable, Error> {
    Ok(ChunkTable {
        offset: u32_at(part, off)? as usize,
        len: u32_at(part, off + 4)? as usize,
    })
}

/// Reads a name descriptor: `u16 len, u16 flags`, then either up to 4 UTF-16
/// chars inline, or (`len > 4`) a 4-byte hash and a `u32` offset to the name,
/// which is stored in one or more `0xFFFFFEFB` entries.
fn read_name(part: &[u8], off: usize) -> Result<String, Error> {
    let len = u16_at(part, off)? as usize;
    if len <= 4 {
        return Ok(utf16(slice(part, off + 4, len * 2)?));
    }
    let mut pos = u32_at(part, off + 8)? as usize;
    let mut bytes = Vec::with_capacity(len * 2);
    while bytes.len() < len * 2 {
        if u32_at(part, pos)? == T_NAME {
            bytes.extend_from_slice(slice(part, pos + 4, 0x30)?);
            pos += 0x34;
        } else {
            bytes.extend_from_slice(slice(part, pos, len * 2 - bytes.len())?);
        }
    }
    bytes.truncate(len * 2);
    Ok(utf16(&bytes))
}

fn read_sections(part: &[u8], mut off: usize) -> Result<Vec<Section>, Error> {
    let mut sections = Vec::new();
    while off != 0 {
        if u32_at(part, off)? != T_SECTION {
            return Err(Error::Format(format!("expected section entry at {off:#x}")));
        }
        if sections.len() > 64 {
            return Err(Error::Format("section chain too long".into()));
        }
        sections.push(Section {
            name: read_name(part, off + 0x0C)?,
            size: u32_at(part, off + 0x18)? as usize,
            data: table_at(part, off + 0x1C)?,
        });
        off = u32_at(part, off + 0x08)? as usize;
    }
    Ok(sections)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a minimal IMGFS partition in memory.
    struct Builder {
        d: Vec<u8>,
    }

    impl Builder {
        fn new() -> Self {
            let mut d = vec![0u8; 0x2000];
            d[..16].copy_from_slice(&GUID);
            put(&mut d, 0x14, 1); // sectors per header block
            put(&mut d, 0x1C, 0x34); // entry size
            put(&mut d, 0x24, 0x200); // first header block
            d[0x2C..0x2F].copy_from_slice(b"LZX");
            put(&mut d, 0x200, BLOCK_SIGNATURE);
            Self { d }
        }
        fn entry(&mut self, slot: usize) -> usize {
            0x208 + slot * 0x34
        }
    }

    fn put(d: &mut [u8], off: usize, v: u32) {
        d[off..off + 4].copy_from_slice(&v.to_le_bytes());
    }

    fn short_name(d: &mut [u8], off: usize, name: &str) {
        d[off..off + 2].copy_from_slice(&(name.len() as u16).to_le_bytes());
        for (i, c) in name.encode_utf16().enumerate() {
            d[off + 4 + i * 2..off + 6 + i * 2].copy_from_slice(&c.to_le_bytes());
        }
    }

    #[test]
    fn parses_raw_file_with_short_name() {
        let mut b = Builder::new();
        let e = b.entry(0);
        put(&mut b.d, e, T_FILE);
        short_name(&mut b.d, e + 0x0C, ".VM");
        put(&mut b.d, e + 0x18, 5); // size
        put(&mut b.d, e + 0x2C, 0x1000); // chunk table
        put(&mut b.d, e + 0x30, 8);
        // one raw chunk of 5 bytes at 0x1100
        b.d[0x1000..0x1002].copy_from_slice(&5u16.to_le_bytes());
        b.d[0x1002..0x1004].copy_from_slice(&5u16.to_le_bytes());
        put(&mut b.d, 0x1004, 0x1100);
        b.d[0x1100..0x1105].copy_from_slice(b"hello");

        let fs = Imgfs::parse(&b.d).unwrap();
        let item = fs.find(".vm").unwrap();
        assert_eq!(item.kind, Kind::File);
        assert_eq!(fs.read(item.data, item.size).unwrap(), b"hello");
    }

    #[test]
    fn parses_module_with_long_name_and_sections() {
        let mut b = Builder::new();
        let (m, n, s0, s1) = (b.entry(0), b.entry(1), b.entry(2), b.entry(3));
        put(&mut b.d, m, T_MODULE);
        put(&mut b.d, m + 0x08, s0 as u32);
        b.d[m + 0x0C..m + 0x0E].copy_from_slice(&13u16.to_le_bytes());
        put(&mut b.d, m + 0x14, n as u32); // long-name storage
        put(&mut b.d, n, T_NAME);
        for (i, c) in "ddi_2700g.dll".encode_utf16().enumerate() {
            b.d[n + 4 + i * 2..n + 6 + i * 2].copy_from_slice(&c.to_le_bytes());
        }
        for (e, name, next) in [(s0, "S000", s1), (s1, "S001", 0)] {
            put(&mut b.d, e, T_SECTION);
            put(&mut b.d, e + 0x08, next as u32);
            short_name(&mut b.d, e + 0x0C, name);
        }

        let fs = Imgfs::parse(&b.d).unwrap();
        let item = fs.find("DDI_2700G.DLL").unwrap();
        let Kind::Module { sections } = &item.kind else {
            panic!("not a module")
        };
        let names: Vec<_> = sections.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["S000", "S001"]);
    }

    #[test]
    fn rejects_wrong_guid() {
        assert!(Imgfs::parse(&[0u8; 0x400]).is_err());
    }

    #[test]
    fn rejects_bad_block_signature() {
        let mut b = Builder::new();
        put(&mut b.d, 0x200, 0xDEAD_BEEF);
        assert!(Imgfs::parse(&b.d).is_err());
    }

    #[test]
    fn detects_looping_section_chain() {
        let mut b = Builder::new();
        let (m, s) = (b.entry(0), b.entry(1));
        put(&mut b.d, m, T_MODULE);
        put(&mut b.d, m + 0x08, s as u32);
        put(&mut b.d, s, T_SECTION);
        put(&mut b.d, s + 0x08, s as u32); // points at itself
        assert!(Imgfs::parse(&b.d).is_err());
    }
}
