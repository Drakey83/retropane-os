//! Microsoft cabinet files (MSCF), as Pocket PC programs ship in.
//!
//! Clean-room from Microsoft's public cabinet specification ([MS-CAB]) and
//! RFC 1951, checked against seven real device CABs
//! (docs/legacysync/activesync-interfaces.md §5). Shared by the CE
//! personality's installer (ADR-0028 L2, on the device) and LegacySync (on
//! the PC).
//!
//! A cabinet holds *folders* (compressed streams, cut into data blocks of up
//! to 32 KiB uncompressed) and *files* (byte ranges of a folder's
//! uncompressed stream). Supported: one cabinet (no spanning sets),
//! uncompressed and MSZIP folders. LZX and Quantum are reported as
//! [`Error::Unsupported`]: no device CAB seen uses them.
//!
//! **Outside input** (docs/security.md): the cabinet comes from a card or
//! from a PC. Everything is read through [`Source::read_at`] with checked
//! offsets; every count, length and offset is validated before use;
//! malformed input is an [`Error`], never a panic or a hang. No heap: the
//! caller owns the ~100 KiB [`Scratch`] buffer, and folders are streamed,
//! so a 22 MB game never needs to be in RAM at once (docs/memory-budget.md).

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

mod inflate;

/// Why a cabinet couldn't be read.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    /// The source couldn't be read (I/O error or a read past its end).
    Io,
    /// Not a cabinet (bad signature or version).
    NotCabinet,
    /// A structure runs past the end of the cabinet, or an offset or count
    /// points outside it.
    Truncated,
    /// A feature this reader doesn't handle.
    Unsupported(Unsupported),
    /// A data block's checksum doesn't match.
    Checksum,
    /// Compressed data or a structure is malformed.
    Corrupt,
    /// The caller's sink refused the data.
    Sink,
}

/// Features [`Error::Unsupported`] names.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Unsupported {
    /// A file continued from or into another cabinet of a set.
    Spanning,
    /// Quantum compression.
    Quantum,
    /// LZX compression.
    Lzx,
    /// An unknown compression type.
    Compression(u16),
}

impl Error {
    /// A short name for logs.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Error::Io => "read failed",
            Error::NotCabinet => "not a cabinet",
            Error::Truncated => "truncated",
            Error::Unsupported(Unsupported::Spanning) => "spans cabinets",
            Error::Unsupported(Unsupported::Quantum) => "Quantum compression",
            Error::Unsupported(Unsupported::Lzx) => "LZX compression",
            Error::Unsupported(Unsupported::Compression(_)) => "unknown compression",
            Error::Checksum => "checksum mismatch",
            Error::Corrupt => "corrupt",
            Error::Sink => "output refused",
        }
    }
}

/// Where a cabinet's bytes come from: a slice in tests and on the PC, a file
/// on a card on the device.
pub trait Source {
    /// Total length in bytes.
    fn len(&self) -> u64;
    /// Whether the source is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Fills `buf` from `offset`. A read past the end is an error.
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), Error>;
}

impl Source for &[u8] {
    fn len(&self) -> u64 {
        <[u8]>::len(self) as u64
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), Error> {
        let start = usize::try_from(offset).map_err(|_| Error::Io)?;
        let end = start.checked_add(buf.len()).ok_or(Error::Io)?;
        buf.copy_from_slice(self.get(start..end).ok_or(Error::Io)?);
        Ok(())
    }
}

/// Largest uncompressed data block ([MS-CAB]: 32 KiB).
pub const MAX_BLOCK: usize = 32 * 1024;
/// Largest compressed data block ([MS-CAB]: 32 KiB + 6 KiB of slack).
pub const MAX_BLOCK_IN: usize = MAX_BLOCK + 6 * 1024;
/// Longest file name, with its NUL ([MS-CAB]: 256 bytes).
pub const MAX_NAME: usize = 256;

const HEADER_LEN: u64 = 36;
const FLAG_PREV: u16 = 0x0001;
const FLAG_NEXT: u16 = 0x0002;
const FLAG_RESERVE: u16 = 0x0004;

/// A cabinet's header, validated against the source's length.
#[derive(Clone, Copy, Debug)]
pub struct Cabinet {
    size: u32,
    files_offset: u32,
    folder_count: u16,
    file_count: u16,
    /// Offset of the first folder entry.
    folders_offset: u32,
    /// Bytes of each folder entry (8 + its reserved area).
    folder_entry: u32,
    /// Reserved bytes in each data block header.
    data_reserve: u8,
    /// The set id and the index in it (for logs).
    set_id: u16,
}

/// A folder: one compressed stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Folder {
    /// Offset of its first data block in the cabinet.
    pub data_offset: u32,
    /// Number of data blocks.
    pub blocks: u16,
    /// How it is compressed.
    pub compression: Compression,
}

/// A folder's compression.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression {
    /// Stored as is.
    None,
    /// DEFLATE blocks behind a `CK` signature.
    MsZip,
}

/// A file's entry.
#[derive(Clone, Copy, Debug)]
pub struct File {
    /// Uncompressed size.
    pub size: u32,
    /// Offset in its folder's uncompressed stream.
    pub offset: u32,
    /// Its folder's index.
    pub folder: u16,
    /// MS-DOS date and time.
    pub date: u16,
    /// MS-DOS time.
    pub time: u16,
    /// Attributes (bit 7 `0x80` = the name is UTF-8).
    pub attributes: u16,
    name: [u8; MAX_NAME],
    name_len: u8,
}

impl File {
    /// The stored name, without the NUL (1-255 bytes, never containing NUL).
    #[must_use]
    pub fn name(&self) -> &[u8] {
        &self.name[..usize::from(self.name_len)]
    }
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

impl Cabinet {
    /// Reads and validates the header at the start of `src`.
    pub fn open<S: Source>(src: &mut S) -> Result<Self, Error> {
        if src.len() < HEADER_LEN {
            return Err(Error::NotCabinet);
        }
        let mut h = [0u8; HEADER_LEN as usize];
        src.read_at(0, &mut h)?;
        if &h[0..4] != b"MSCF" || h[24] != 3 || h[25] != 1 {
            return Err(Error::NotCabinet);
        }
        let size = u32_at(&h, 8);
        if u64::from(size) > src.len() || u64::from(size) < HEADER_LEN {
            return Err(Error::Truncated);
        }
        let files_offset = u32_at(&h, 16);
        let folder_count = u16_at(&h, 26);
        let file_count = u16_at(&h, 28);
        let flags = u16_at(&h, 30);
        let set_id = u16_at(&h, 32);
        if flags & (FLAG_PREV | FLAG_NEXT) != 0 {
            return Err(Error::Unsupported(Unsupported::Spanning));
        }
        let mut at = HEADER_LEN;
        let (mut folder_reserve, mut data_reserve) = (0u8, 0u8);
        if flags & FLAG_RESERVE != 0 {
            let mut r = [0u8; 4];
            read_in(src, size, at, &mut r)?;
            let header_reserve = u16_at(&r, 0);
            folder_reserve = r[2];
            data_reserve = r[3];
            at += 4 + u64::from(header_reserve);
        }
        let folders_offset = u32::try_from(at).map_err(|_| Error::Truncated)?;
        let folder_entry = 8 + u32::from(folder_reserve);
        let folders_end =
            u64::from(folders_offset) + u64::from(folder_count) * u64::from(folder_entry);
        if folders_end > u64::from(size) || u64::from(files_offset) > u64::from(size) {
            return Err(Error::Truncated);
        }
        if file_count > 0 && folder_count == 0 {
            return Err(Error::Corrupt);
        }
        Ok(Cabinet {
            size,
            files_offset,
            folder_count,
            file_count,
            folders_offset,
            folder_entry,
            data_reserve,
            set_id,
        })
    }

    /// Number of folders.
    #[must_use]
    pub const fn folder_count(&self) -> u16 {
        self.folder_count
    }

    /// Number of files.
    #[must_use]
    pub const fn file_count(&self) -> u16 {
        self.file_count
    }

    /// Cabinet size from its header.
    #[must_use]
    pub const fn size(&self) -> u32 {
        self.size
    }

    /// The set id (cabinets of one set share it).
    #[must_use]
    pub const fn set_id(&self) -> u16 {
        self.set_id
    }

    /// Reads folder `index`'s entry.
    pub fn folder<S: Source>(&self, src: &mut S, index: u16) -> Result<Folder, Error> {
        if index >= self.folder_count {
            return Err(Error::Corrupt);
        }
        let at = u64::from(self.folders_offset) + u64::from(index) * u64::from(self.folder_entry);
        let mut e = [0u8; 8];
        read_in(src, self.size, at, &mut e)?;
        let data_offset = u32_at(&e, 0);
        let blocks = u16_at(&e, 4);
        let compression = match u16_at(&e, 6) & 0x000F {
            0 => Compression::None,
            1 => Compression::MsZip,
            2 => return Err(Error::Unsupported(Unsupported::Quantum)),
            3 => return Err(Error::Unsupported(Unsupported::Lzx)),
            other => return Err(Error::Unsupported(Unsupported::Compression(other))),
        };
        if data_offset > self.size {
            return Err(Error::Truncated);
        }
        Ok(Folder {
            data_offset,
            blocks,
            compression,
        })
    }

    /// Iterates over the file entries in stored order. After the first
    /// error the iterator ends.
    pub fn files<'s, S: Source>(&self, src: &'s mut S) -> Files<'s, S> {
        Files {
            cab: *self,
            src,
            next: 0,
            at: u64::from(self.files_offset),
            failed: false,
        }
    }

    /// Streams folder `index`'s uncompressed bytes to `sink` as
    /// `(offset in the folder, bytes)`, in order, one data block at a time.
    /// Each block's checksum (when non-zero) and declared sizes are checked
    /// before its bytes are passed on.
    pub fn extract_folder<S: Source, F>(
        &self,
        src: &mut S,
        index: u16,
        scratch: &mut Scratch,
        mut sink: F,
    ) -> Result<(), Error>
    where
        F: FnMut(u64, &[u8]) -> Result<Flow, Error>,
    {
        let folder = self.folder(src, index)?;
        let mut at = u64::from(folder.data_offset);
        let mut produced: u64 = 0;
        // MSZIP history: window[..hist] holds the last up-to-32 KiB of output.
        let mut hist = 0usize;
        for _ in 0..folder.blocks {
            let mut h = [0u8; 8];
            read_in(src, self.size, at, &mut h)?;
            let csum = u32_at(&h, 0);
            let packed = usize::from(u16_at(&h, 4));
            let unpacked = usize::from(u16_at(&h, 6));
            if packed > MAX_BLOCK_IN || unpacked > MAX_BLOCK || unpacked == 0 {
                return Err(Error::Corrupt);
            }
            at += 8 + u64::from(self.data_reserve);
            let data = &mut scratch.block[..packed];
            read_in(src, self.size, at, data)?;
            at += packed as u64;
            if csum != 0 && checksum(&h[4..8], checksum(data, 0)) != csum {
                return Err(Error::Checksum);
            }
            let out: &[u8] = match folder.compression {
                Compression::None => {
                    if packed != unpacked {
                        return Err(Error::Corrupt);
                    }
                    &scratch.block[..packed]
                }
                Compression::MsZip => {
                    if packed < 2 || &data[..2] != b"CK" {
                        return Err(Error::Corrupt);
                    }
                    let n = inflate::inflate(&data[2..], &mut scratch.window, hist, unpacked)?;
                    if n != unpacked {
                        return Err(Error::Corrupt);
                    }
                    let out = &scratch.window[hist..hist + n];
                    hist += n;
                    out
                }
            };
            let flow = sink(produced, out)?;
            produced += out.len() as u64;
            if folder.compression == Compression::MsZip && hist > MAX_BLOCK {
                scratch.window.copy_within(hist - MAX_BLOCK..hist, 0);
                hist = MAX_BLOCK;
            }
            if flow == Flow::Stop {
                break;
            }
        }
        Ok(())
    }

    /// Streams one file's bytes to `sink` as `(offset in the file, bytes)`.
    /// Fails with [`Error::Truncated`] if its folder ends before the file
    /// does.
    pub fn extract_file<S: Source, F>(
        &self,
        src: &mut S,
        file: &File,
        scratch: &mut Scratch,
        mut sink: F,
    ) -> Result<(), Error>
    where
        F: FnMut(u64, &[u8]) -> Result<(), Error>,
    {
        let start = u64::from(file.offset);
        let end = start + u64::from(file.size);
        let mut done = start;
        if file.size > 0 {
            self.extract_folder(src, file.folder, scratch, |at, bytes| {
                let block_end = at + bytes.len() as u64;
                if at > done {
                    return Err(Error::Corrupt);
                }
                if block_end > done && at < end {
                    let from = (done - at) as usize;
                    let to = (end.min(block_end) - at) as usize;
                    sink(done - start, &bytes[from..to])?;
                    done = at + to as u64;
                }
                Ok(if done >= end {
                    Flow::Stop
                } else {
                    Flow::Continue
                })
            })?;
        }
        if done < end {
            Err(Error::Truncated)
        } else {
            Ok(())
        }
    }
}

/// Whether [`Cabinet::extract_folder`] should keep going.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Flow {
    /// Decode the next block.
    Continue,
    /// Stop after this block (the rest isn't needed).
    Stop,
}

/// Working memory for extraction: one compressed block and the MSZIP
/// window (32 KiB of history + one 32 KiB block). ~102 KiB; the caller
/// decides where it lives.
pub struct Scratch {
    block: [u8; MAX_BLOCK_IN],
    window: [u8; 2 * MAX_BLOCK],
}

impl Scratch {
    /// A zeroed scratch area.
    #[must_use]
    pub const fn new() -> Self {
        Scratch {
            block: [0; MAX_BLOCK_IN],
            window: [0; 2 * MAX_BLOCK],
        }
    }
}

impl Default for Scratch {
    fn default() -> Self {
        Self::new()
    }
}

/// Iterator over a cabinet's file entries.
pub struct Files<'s, S: Source> {
    cab: Cabinet,
    src: &'s mut S,
    next: u16,
    at: u64,
    failed: bool,
}

impl<S: Source> Files<'_, S> {
    fn read_one(&mut self) -> Result<File, Error> {
        let mut e = [0u8; 16];
        read_in(self.src, self.cab.size, self.at, &mut e)?;
        let size = u32_at(&e, 0);
        let offset = u32_at(&e, 4);
        let folder = u16_at(&e, 8);
        if folder >= 0xFFFD {
            return Err(Error::Unsupported(Unsupported::Spanning));
        }
        if folder >= self.cab.folder_count || offset.checked_add(size).is_none() {
            return Err(Error::Corrupt);
        }
        // The name: NUL-terminated, at most MAX_NAME bytes with the NUL,
        // inside the cabinet.
        let name_at = self.at + 16;
        let avail = u64::from(self.cab.size)
            .saturating_sub(name_at)
            .min(MAX_NAME as u64) as usize;
        let mut name = [0u8; MAX_NAME];
        read_in(self.src, self.cab.size, name_at, &mut name[..avail])?;
        let len = name[..avail]
            .iter()
            .position(|&b| b == 0)
            .ok_or(Error::Truncated)?;
        if len == 0 {
            return Err(Error::Corrupt);
        }
        name[len..].fill(0);
        self.at = name_at + len as u64 + 1;
        Ok(File {
            size,
            offset,
            folder,
            date: u16_at(&e, 10),
            time: u16_at(&e, 12),
            attributes: u16_at(&e, 14),
            name,
            name_len: len as u8,
        })
    }
}

impl<S: Source> Iterator for Files<'_, S> {
    type Item = Result<File, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed || self.next >= self.cab.file_count {
            return None;
        }
        self.next += 1;
        let r = self.read_one();
        self.failed = r.is_err();
        Some(r)
    }
}

/// Reads `buf.len()` bytes at `at`, refusing anything past the cabinet's
/// declared size.
fn read_in<S: Source>(src: &mut S, size: u32, at: u64, buf: &mut [u8]) -> Result<(), Error> {
    let end = at.checked_add(buf.len() as u64).ok_or(Error::Truncated)?;
    if end > u64::from(size) {
        return Err(Error::Truncated);
    }
    src.read_at(at, buf)
}

/// [MS-CAB]'s data block checksum: the XOR of the input as little-endian
/// 32-bit words, a 1-3 byte tail packed with its first byte highest.
/// Verified on 1,963 real blocks: data first (seed 0), then the block's two
/// length fields seeded with that.
#[must_use]
pub fn checksum(data: &[u8], seed: u32) -> u32 {
    let mut sum = seed;
    let (words, tail) = data.as_chunks::<4>();
    for w in words {
        sum ^= u32::from_le_bytes(*w);
    }
    let tail = tail.iter().fold(0u32, |acc, &b| (acc << 8) | u32::from(b));
    sum ^ tail
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_tail_packs_first_byte_highest() {
        assert_eq!(checksum(&[1, 2, 3, 4], 0), 0x0403_0201);
        assert_eq!(
            checksum(&[1, 2, 3, 4, 0xAA, 0xBB, 0xCC], 0),
            0x0403_0201 ^ 0x00AA_BBCC
        );
        assert_eq!(checksum(&[0xAA], 0x10), 0xBA);
    }

    #[test]
    fn a_short_or_foreign_file_is_not_a_cabinet() {
        let mut s: &[u8] = b"MSCF";
        assert_eq!(Cabinet::open(&mut s).unwrap_err(), Error::NotCabinet);
        let mut s: &[u8] = &[0u8; 64];
        assert_eq!(Cabinet::open(&mut s).unwrap_err(), Error::NotCabinet);
    }
}
