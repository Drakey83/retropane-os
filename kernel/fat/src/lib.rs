//! FAT12, FAT16 and FAT32 on a [`BlockDevice`] (ADR-0013, ADR-0036): the
//! SD and CF cards as Windows Mobile and every PC format them, old 2 GB and
//! smaller cards (FAT16, FAT12) included.
//!
//! The three differ only in the FAT's entry width (12, 16 or 28 bits) and
//! FAT12/16's fixed root directory, a region before the data area that is
//! never grown; the type is decided by the data-cluster count, as
//! Microsoft's FAT specification says, except that a volume with FAT32's
//! markers is FAT32 whatever its size ([`Volume::mount`]).
//!
//! Reads: mount a partition, find a file in the root directory by its long
//! or short name, read it in runs of consecutive sectors. Writes: create a
//! file (its directory entry written last, so a power cut leaves no file or
//! the whole file; long names as Windows writes them, `~1`-style short
//! names, lower-case 8.3 names stored with Windows NT's case flags), or
//! replace a file's contents by **copy-then-swap**: the new contents go into fresh
//! clusters (one contiguous run when there is one), the chain is linked in
//! every FAT copy, then the directory entry is switched to the new chain
//! with a single sector write, and only then are the old clusters freed.
//! Power lost at any point leaves the complete old file or the complete new
//! one. No heap: two sector buffers live in the [`Volume`], and reading a
//! file or searching the FAT for free clusters borrows the caller's
//! scratch, so a bigger scratch means fewer, longer device commands.
//!
//! Not here yet: timestamps (no clock yet: new entries carry none, which
//! Windows shows as unknown). Subdirectories and deleting are here.

#![cfg_attr(not(any(test, feature = "std")), no_std)]
#![forbid(unsafe_code)]

use rp_hal_api::{BlockDevice, BlockError};

/// Sector size; the only one SD and CF cards use.
pub const SECTOR: usize = 512;
/// Directory entries per sector.
const ENTRIES: usize = SECTOR / 32;
/// FAT32 entries per sector.
const FAT_PER_SECTOR: u32 = (SECTOR / 4) as u32;
/// FAT12 has under 4,085 data clusters, FAT16 under 65,525 (Microsoft's
/// FAT specification, "FAT type determination": the count alone decides).
const FAT12_LIMIT: u32 = 4085;
const FAT16_LIMIT: u32 = 65_525;
/// The bad-cluster mark, in FAT32's numbering (narrower FATs are widened
/// to it as they are read).
const BAD: u32 = 0x0FFF_FFF7;
/// Most data clusters a FAT32 volume can have: entries are 28 bits, and
/// the top values mean bad (`0x0FFF_FFF7`) and end of chain.
const MAX_CLUSTERS: u32 = 0x0FFF_FFF5;
/// Most FAT sectors worth reading: enough for [`MAX_CLUSTERS`] (2^21). A
/// card claiming a bigger FAT is refused at mount, so a FAT sector number
/// times [`FAT_PER_SECTOR`] is always under 2^28: no overflow anywhere a
/// FAT position becomes a cluster number (`check`, `Guard`).
const MAX_FAT_SECTORS: u32 = (MAX_CLUSTERS + 2).div_ceil(FAT_PER_SECTOR);
/// The largest directory FAT32 allows: 65,536 entries of 32 bytes (the
/// specification's limit). Directory walks stop there, so a hostile card's
/// endless (non-looping) directory chain costs at most 2 MiB of reads.
const DIR_MAX_BYTES: usize = 65_536 * 32;
/// End-of-chain marker written; anything from 0x0FFF_FFF8 reads as one.
const EOC: u32 = 0x0FFF_FFFF;
const EOC_MIN: u32 = 0x0FFF_FFF8;
const FAT_MASK: u32 = 0x0FFF_FFFF;
/// Longest file name (the FAT limit is 255; this is what's stored).
pub const NAME_MAX: usize = 64;
const ATTR_DIRECTORY: u8 = 0x10;
const ATTR_LONG_NAME: u8 = 0x0F;
const FSINFO_UNKNOWN: u32 = 0xFFFF_FFFF;

/// Which FAT a volume has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FatKind {
    /// 12-bit entries (under 4,085 clusters: cards to about 64 MB).
    Fat12,
    /// 16-bit entries (under 65,525 clusters: cards to 2 GB, 4 GB at most).
    Fat16,
    /// 28-bit entries.
    Fat32,
}

impl FatKind {
    /// The bits an entry holds.
    const fn mask(self) -> u32 {
        match self {
            Self::Fat12 => 0xFFF,
            Self::Fat16 => 0xFFFF,
            Self::Fat32 => FAT_MASK,
        }
    }

    /// The byte where `cluster`'s entry starts in the FAT.
    pub(crate) const fn offset(self, cluster: u32) -> u32 {
        match self {
            Self::Fat12 => cluster + cluster / 2,
            Self::Fat16 => cluster * 2,
            Self::Fat32 => cluster * 4,
        }
    }

    /// Bytes an entry touches (a FAT12 entry: 1.5 bytes, in two).
    pub(crate) const fn width(self) -> u32 {
        match self {
            Self::Fat32 => 4,
            _ => 2,
        }
    }

    /// The first cluster whose entry starts at or after FAT byte `byte`.
    pub(crate) const fn first_at(self, byte: u32) -> u32 {
        match self {
            Self::Fat12 => (byte * 2).div_ceil(3),
            Self::Fat16 => byte.div_ceil(2),
            Self::Fat32 => byte.div_ceil(4),
        }
    }

    /// An entry as stored, widened to FAT32's numbering: every end-of-chain
    /// value reads as [`EOC`], the bad mark as [`BAD`].
    pub(crate) const fn widen(self, v: u32) -> u32 {
        let m = self.mask();
        if v >= m - 7 {
            EOC
        } else if v == m - 8 {
            BAD
        } else {
            v
        }
    }

    /// The name a log or a partition table shows.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Fat12 => "FAT12",
            Self::Fat16 => "FAT16",
            Self::Fat32 => "FAT32",
        }
    }
}

/// `cluster`'s entry as stored, from `b` (the entry's bytes start at `o`),
/// not widened.
pub(crate) fn entry_get(kind: FatKind, b: &[u8], o: usize, cluster: u32) -> u32 {
    match kind {
        FatKind::Fat12 => {
            let w = u32::from(le16(b, o));
            if cluster & 1 == 0 { w & 0xFFF } else { w >> 4 }
        }
        FatKind::Fat16 => u32::from(le16(b, o)),
        FatKind::Fat32 => le32(b, o) & FAT_MASK,
    }
}

/// A directory entry's first cluster (FAT12/16 have no high half: those
/// bytes were OS/2's extended-attribute handle, so they are ignored).
pub(crate) fn entry_first(e: &[u8], kind: FatKind) -> u32 {
    let high = if kind == FatKind::Fat32 {
        u32::from(le16(e, 20)) << 16
    } else {
        0
    };
    high | u32::from(le16(e, 26))
}

/// Stores `value` (cut to the entry's width: [`EOC`] and [`BAD`] become the
/// narrow marks) as `cluster`'s entry at `o`, keeping the bits around it
/// (FAT12's neighbouring half byte, FAT32's top four bits).
fn entry_put(kind: FatKind, b: &mut [u8], o: usize, cluster: u32, value: u32) {
    let v = value & kind.mask();
    match kind {
        FatKind::Fat12 => {
            let w = le16(b, o);
            let w = if cluster & 1 == 0 {
                (w & 0xF000) | v as u16
            } else {
                (w & 0x000F) | (v as u16) << 4
            };
            b[o..o + 2].copy_from_slice(&w.to_le_bytes());
        }
        FatKind::Fat16 => b[o..o + 2].copy_from_slice(&(v as u16).to_le_bytes()),
        FatKind::Fat32 => {
            let w = (le32(b, o) & !FAT_MASK) | v;
            b[o..o + 4].copy_from_slice(&w.to_le_bytes());
        }
    }
}

/// What went wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The device failed.
    Device(BlockError),
    /// The partition isn't FAT12, FAT16 or FAT32 with 512-byte sectors.
    NotFat32,
    /// No such file in the root directory.
    NotFound,
    /// Not enough free clusters for the new contents.
    NoRoom,
    /// A cluster chain or directory is damaged.
    Corrupt,
    /// The name is longer than [`NAME_MAX`], not ASCII, or has characters
    /// FAT doesn't allow.
    BadName,
    /// A file of that name already exists (to create).
    Exists,
    /// The partition isn't exFAT (or its boot region checksum is wrong).
    NotExFat,
    /// The folder still holds files or folders.
    NotEmpty,
    /// No single run of free clusters is long enough (a reservation must
    /// be one run: see [`Reserved`]).
    Fragmented,
}

impl From<BlockError> for Error {
    fn from(e: BlockError) -> Self {
        Self::Device(e)
    }
}

impl Error {
    /// A short fixed name, for logs that shouldn't pull in `Debug` formatting.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Device(_) => "device error",
            Self::NotFat32 => "not FAT",
            Self::NotFound => "not found",
            Self::NoRoom => "no room",
            Self::Corrupt => "corrupt",
            Self::BadName => "bad name",
            Self::Exists => "exists",
            Self::NotExFat => "not exFAT",
            Self::NotEmpty => "folder not empty",
            Self::Fragmented => "no free run long enough",
        }
    }
}

/// Where a root-directory file is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct File {
    /// Sector holding the directory entry.
    entry_lba: u32,
    /// Entry index within that sector.
    entry_index: u8,
    /// First cluster (0 for an empty file).
    first_cluster: u32,
    /// Size in bytes.
    size: u32,
}

impl File {
    /// Size in bytes.
    #[must_use]
    pub fn size(&self) -> u32 {
        self.size
    }

    /// First cluster (0 for an empty file).
    #[must_use]
    pub fn first_cluster(&self) -> u32 {
        self.first_cluster
    }
}

/// A root-directory entry, as reported by [`Volume::for_each_root`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirEntry<'a> {
    /// The long name when the entry has one, else the short name as
    /// `NAME.EXT` (ASCII; non-ASCII characters read as `?`).
    pub name: &'a [u8],
    /// Size in bytes (0 for directories).
    pub size: u32,
    /// First cluster.
    pub first_cluster: u32,
    /// A subdirectory.
    pub is_dir: bool,
}

/// What a replacement did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Written {
    /// First cluster of the new contents.
    pub first_cluster: u32,
    /// Clusters used.
    pub clusters: u32,
    /// The clusters are one consecutive run.
    pub contiguous: bool,
}

/// A file reserved by `reserve`: zero-filled, in one run of sectors, so a
/// program can write its contents in place, sector by sector, without
/// touching the file system again (a run log that survives a reset: every
/// sector written is on the card, the directory and the FAT never change).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reserved {
    /// The file's first sector on the device.
    pub lba: u32,
    /// Its sectors (the size rounded up to whole sectors).
    pub sectors: u32,
}

/// What a store writes: bytes, or zeros (a reservation, written without a
/// buffer of its size).
#[derive(Clone, Copy)]
pub(crate) enum Fill<'a> {
    Bytes(&'a [u8]),
    Zeros(usize),
}

impl<'a> Fill<'a> {
    pub(crate) fn len(self) -> usize {
        match self {
            Self::Bytes(b) => b.len(),
            Self::Zeros(n) => n,
        }
    }

    /// `len` bytes from `at` on.
    pub(crate) fn part(self, at: usize, len: usize) -> Fill<'a> {
        match self {
            Self::Bytes(b) => Self::Bytes(&b[at..at + len]),
            Self::Zeros(_) => Self::Zeros(len),
        }
    }

    /// Writes it from sector `lba` on, the last partial sector zero-padded:
    /// whole sectors of bytes as they are, zeros and the tail through
    /// `scratch` (a sector on the stack when `scratch` is shorter).
    pub(crate) fn write<D: BlockDevice>(
        self,
        dev: &mut D,
        lba: u32,
        scratch: &mut [u8],
    ) -> Result<(), Error> {
        let mut one = [0u8; SECTOR];
        let scratch = if scratch.len() < SECTOR {
            &mut one[..]
        } else {
            scratch
        };
        let n = self.len();
        let whole = match self {
            Self::Bytes(b) => {
                let whole = n / SECTOR * SECTOR;
                if whole > 0 {
                    dev.write_blocks(lba, &b[..whole])?;
                }
                whole
            }
            Self::Zeros(_) => 0,
        };
        let total = n.div_ceil(SECTOR) * SECTOR;
        if whole < total {
            let chunk = scratch.len() / SECTOR * SECTOR;
            let buf = &mut scratch[..chunk];
            buf.fill(0);
            if let Self::Bytes(b) = self {
                // The bytes' tail, under a sector: zero-padded.
                buf[..n - whole].copy_from_slice(&b[whole..]);
            }
            let mut at = whole;
            while at < total {
                let len = chunk.min(total - at);
                dev.write_blocks(lba + (at / SECTOR) as u32, &buf[..len])?;
                if at == whole {
                    buf[..SECTOR].fill(0);
                }
                at += len;
            }
        }
        Ok(())
    }
}

/// What [`Volume::walk_dir`] calls per entry: sector, index, raw entry,
/// long name; `true` stops the walk.
type Visit<'a> = dyn FnMut(u32, u8, &[u8; 32], &[u8]) -> bool + 'a;

/// A mounted FAT12, FAT16 or FAT32 partition.
pub struct Volume<D> {
    dev: D,
    /// The partition's first sector (the boot sector).
    base: u32,
    kind: FatKind,
    sectors_per_cluster: u32,
    fat_start: u32,
    fat_sectors: u32,
    fats: u32,
    /// The FATs are mirrored (BPB ExtFlags bit 7 clear): every copy is
    /// written, so the second is a backup of the first.
    mirrored: bool,
    data_start: u32,
    /// The root directory's first cluster; 0 for FAT12/16's fixed root (the
    /// number `..` entries use for the root), at `root_lba`.
    root_cluster: u32,
    root_lba: u32,
    /// The fixed root's sectors (0 on FAT32).
    root_sectors: u32,
    /// Data clusters, numbered 2..2 + clusters.
    clusters: u32,
    fsinfo_lba: Option<u32>,
    buf: [u8; SECTOR],
    /// Write-back cache of one FAT sector (`fat_cached` = its index). A
    /// FAT12 entry straddling two sectors is read and written as its two
    /// halves, one sector at a time (`fat_raw`, `fat_set`), so no FAT
    /// width costs a second sector of RAM.
    fat_buf: [u8; SECTOR],
    fat_cached: Option<u32>,
    fat_dirty: bool,
    /// Assembled long name of the entry being read.
    lfn: [u8; NAME_MAX],
    lfn_len: usize,
    lfn_checksum: u8,
    lfn_valid: bool,
}

fn le16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

fn le32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// The 8.3 name checksum long-name entries carry.
#[must_use]
pub fn short_checksum(short: &[u8; 11]) -> u8 {
    short.iter().fold(0u8, |sum, &b| {
        ((sum & 1) << 7).wrapping_add(sum >> 1).wrapping_add(b)
    })
}

/// Whether `name` is usable: ASCII, no path separators, not too long.
fn check_name(name: &[u8]) -> Result<(), Error> {
    if name.is_empty()
        || name.len() > NAME_MAX
        || name
            .iter()
            .any(|&b| !(0x20..0x7F).contains(&b) || b == b'/' || b == b'\\')
    {
        return Err(Error::BadName);
    }
    Ok(())
}

/// A name a new file may have: [`check_name`], plus none of the characters
/// FAT forbids, no trailing dot or space, not `.` or `..`.
fn check_new_name(name: &[u8]) -> Result<(), Error> {
    check_name(name)?;
    if name.iter().any(|b| b"\"*:<>?|".contains(b))
        || name.ends_with(b".")
        || name.ends_with(b" ")
        || name.iter().all(|&b| b == b'.')
    {
        return Err(Error::BadName);
    }
    Ok(())
}

/// Most directory levels a path may have.
const MAX_DEPTH: usize = 16;

/// The part of `path` up to the end of `part`, one of its components:
/// the path of that folder. Taken with `get`, not by indexing: a `str`
/// index that could panic links Rust's text-boundary panic, which brings
/// ~4 KiB of Unicode tables into the loader (none is needed: `part` lies
/// in `path`, so this never fails).
fn prefix_to<'a>(path: &'a str, part: &str) -> Result<&'a str, Error> {
    let end = (part.as_ptr() as usize)
        .wrapping_sub(path.as_ptr() as usize)
        .wrapping_add(part.len());
    path.get(..end).ok_or(Error::BadName)
}

/// A path's components (`/` or `\` between them; empty ones skipped);
/// `.`, `..` and more than [`MAX_DEPTH`] levels are refused.
fn components(path: &str) -> Result<impl DoubleEndedIterator<Item = &str>, Error> {
    let parts = || path.split(['/', '\\']).filter(|p| !p.is_empty());
    if parts().count() > MAX_DEPTH || parts().any(|p| p == "." || p == "..") {
        return Err(Error::BadName);
    }
    Ok(parts())
}

/// Characters allowed in an 8.3 short name besides letters and digits.
fn short_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'()-@^_`{}~".contains(&b)
}

/// How a name is stored: its 11-byte short name, Windows NT's case flags
/// (0x08: lower-case base, 0x10: lower-case extension) and whether it
/// needs long-name entries.
fn plain_short(name: &[u8]) -> Option<([u8; 11], u8)> {
    let (base, ext) = match name.iter().rposition(|&b| b == b'.') {
        Some(0) => return None,
        Some(i) => (&name[..i], &name[i + 1..]),
        None => (name, &b""[..]),
    };
    let uniform = |p: &[u8]| {
        p.iter().all(|&b| short_char(b))
            && (p.iter().all(|b| !b.is_ascii_lowercase())
                || p.iter().all(|b| !b.is_ascii_uppercase()))
    };
    if base.is_empty() || base.len() > 8 || ext.len() > 3 || !uniform(base) || !uniform(ext) {
        return None;
    }
    let mut short = [b' '; 11];
    for (d, &s) in short.iter_mut().zip(base) {
        *d = s.to_ascii_uppercase();
    }
    for (d, &s) in short[8..].iter_mut().zip(ext) {
        *d = s.to_ascii_uppercase();
    }
    let mut nt = 0;
    if base.iter().any(u8::is_ascii_lowercase) {
        nt |= 0x08;
    }
    if ext.iter().any(u8::is_ascii_lowercase) {
        nt |= 0x10;
    }
    Some((short, nt))
}

/// The `BASIS~N.EXT` short name Windows derives from a long name.
fn numbered_short(name: &[u8], n: u32) -> [u8; 11] {
    let (base, ext) = match name.iter().rposition(|&b| b == b'.') {
        Some(i) if i > 0 => (&name[..i], &name[i + 1..]),
        _ => (name, &b""[..]),
    };
    let mut short = [b' '; 11];
    let clean = |b: u8| {
        let u = b.to_ascii_uppercase();
        if short_char(u) {
            Some(u)
        } else if b == b' ' || b == b'.' {
            None
        } else {
            Some(b'_')
        }
    };
    let mut digits = [0u8; 7];
    let mut k = digits.len();
    let mut v = n;
    loop {
        k -= 1;
        digits[k] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    digits[k - 1] = b'~';
    let tail = &digits[k - 1..];
    let keep = 8 - tail.len();
    let mut i = 0;
    for b in base.iter().filter_map(|&b| clean(b)).take(keep) {
        short[i] = b;
        i += 1;
    }
    short[i..i + tail.len()].copy_from_slice(tail);
    for (d, b) in short[8..]
        .iter_mut()
        .zip(ext.iter().filter_map(|&b| clean(b)))
    {
        *d = b;
    }
    short
}

/// The long-name entries for `name` (ASCII), last piece first as they go
/// on disk, each 32 bytes; returns how many.
fn lfn_entries(name: &[u8], checksum: u8, out: &mut [[u8; 32]; LFN_MAX]) -> usize {
    const AT: [usize; 13] = [1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30];
    let pieces = name.len().div_ceil(13);
    for p in 0..pieces {
        let e = &mut out[pieces - 1 - p];
        *e = [0; 32];
        e[0] = (p + 1) as u8 | if p + 1 == pieces { 0x40 } else { 0 };
        e[11] = ATTR_LONG_NAME;
        e[13] = checksum;
        for (i, &o) in AT.iter().enumerate() {
            let idx = p * 13 + i;
            let c: u16 = match idx.cmp(&name.len()) {
                core::cmp::Ordering::Less => u16::from(name[idx]),
                core::cmp::Ordering::Equal => 0,
                core::cmp::Ordering::Greater => 0xFFFF,
            };
            e[o..o + 2].copy_from_slice(&c.to_le_bytes());
        }
    }
    pieces
}

/// Most long-name entries a name of [`NAME_MAX`] needs.
const LFN_MAX: usize = NAME_MAX.div_ceil(13);

impl<D: BlockDevice> Volume<D> {
    /// Mounts the FAT12, FAT16 or FAT32 partition whose first sector is
    /// `base`.
    #[inline(never)]
    pub fn mount(mut dev: D, base: u32) -> Result<Self, Error> {
        let mut buf = [0u8; SECTOR];
        dev.read_blocks(base, &mut buf)?;
        let bytes_per_sector = le16(&buf, 11);
        let sectors_per_cluster = u32::from(buf[13]);
        let reserved = u32::from(le16(&buf, 14));
        let fats = u32::from(buf[16]);
        let root_entries = u32::from(le16(&buf, 17));
        let fat16_size = u32::from(le16(&buf, 22));
        // FAT32's markers: no fixed root directory and no 16-bit FAT size.
        let fat32 = root_entries == 0 && fat16_size == 0;
        let fat_sectors = if fat32 { le32(&buf, 36) } else { fat16_size };
        let total = match le16(&buf, 19) {
            0 => le32(&buf, 32),
            n => u32::from(n),
        };
        if usize::from(bytes_per_sector) != SECTOR
            || sectors_per_cluster == 0
            || !sectors_per_cluster.is_power_of_two()
            || reserved == 0
            || fats == 0
            || fat_sectors == 0
            || fat_sectors > MAX_FAT_SECTORS
            || buf[510..512] != [0x55, 0xAA]
        {
            return Err(Error::NotFat32);
        }
        // Everything below comes from the card, so it is hostile until
        // checked (docs/security.md): overflow-checked, and the data area,
        // every cluster and the root must lie inside the partition. Then
        // every in-range cluster's sector number fits in a u32, so the code
        // that trusts validated clusters can't overflow.
        let root_sectors = (root_entries * 32).div_ceil(SECTOR as u32);
        let fat_total = fats.checked_mul(fat_sectors);
        let fat_start = base.checked_add(reserved);
        let root_lba = fat_start.zip(fat_total).and_then(|(s, t)| s.checked_add(t));
        let data_start = root_lba.and_then(|r| r.checked_add(root_sectors));
        let end = base.checked_add(total);
        let (Some(fat_total), Some(fat_start), Some(root_lba), Some(data_start), Some(_)) =
            (fat_total, fat_start, root_lba, data_start, end)
        else {
            return Err(Error::NotFat32);
        };
        let meta = reserved
            .saturating_add(fat_total)
            .saturating_add(root_sectors);
        let data_sectors = total.saturating_sub(meta);
        let clusters = data_sectors / sectors_per_cluster;
        // The cluster count decides FAT12 from FAT16 (the specification's
        // rule); a FAT16-shaped volume with FAT32's count is invalid. Test
        // images are tiny FAT32 ones, so FAT32's markers decide FAT32.
        let kind = if fat32 {
            FatKind::Fat32
        } else if clusters < FAT12_LIMIT {
            FatKind::Fat12
        } else if clusters < FAT16_LIMIT {
            FatKind::Fat16
        } else {
            return Err(Error::NotFat32);
        };
        let root_cluster = if fat32 { le32(&buf, 44) } else { 0 };
        // The FAT must have an entry for every cluster (`clusters` is under
        // 2^28 once checked, so the offset can't overflow).
        let fat_bytes = u64::from(fat_sectors) * SECTOR as u64;
        if clusters == 0
            || clusters > MAX_CLUSTERS
            || u64::from(kind.offset(clusters + 1)) + u64::from(kind.width()) > fat_bytes
            || (fat32 && !(2..2 + clusters).contains(&root_cluster))
        {
            return Err(Error::NotFat32);
        }
        let fsinfo = le16(&buf, 48);
        Ok(Self {
            dev,
            base,
            kind,
            sectors_per_cluster,
            fat_start,
            fat_sectors,
            fats,
            // FAT12/16 have no ExtFlags: every copy is always written.
            mirrored: fats >= 2 && (!fat32 || le16(&buf, 40) & 0x80 == 0),
            data_start,
            root_cluster,
            root_lba,
            root_sectors,
            clusters,
            fsinfo_lba: (fat32 && fsinfo != 0 && fsinfo != 0xFFFF)
                .then(|| base + u32::from(fsinfo)),
            buf,
            fat_buf: [0; SECTOR],
            fat_cached: None,
            fat_dirty: false,
            lfn: [0; NAME_MAX],
            lfn_len: 0,
            lfn_checksum: 0,
            lfn_valid: false,
        })
    }

    /// Which FAT the volume has.
    #[must_use]
    pub fn kind(&self) -> FatKind {
        self.kind
    }

    /// Bytes per cluster.
    #[must_use]
    pub fn cluster_bytes(&self) -> usize {
        self.sectors_per_cluster as usize * SECTOR
    }

    /// Data clusters on the volume.
    #[must_use]
    pub fn clusters(&self) -> u32 {
        self.clusters
    }

    /// The sector holding the same data as `lba` in the second FAT, when
    /// `lba` is a sector of the first FAT and the FATs are mirrored (with
    /// mirroring off, only the active FAT is kept up to date, so the other
    /// is no backup): the card's own copy, for scrubbing or repairing a
    /// weak FAT sector safely.
    #[must_use]
    pub fn fat_mirror(&self, lba: u32) -> Option<u32> {
        let in_first = (self.fat_start..self.fat_start + self.fat_sectors).contains(&lba);
        (in_first && self.mirrored).then(|| lba + self.fat_sectors)
    }

    /// Gives the device back (flushing nothing: every operation leaves the
    /// FAT written).
    pub fn into_inner(self) -> D {
        self.dev
    }

    /// The device, for its own diagnostics.
    #[must_use]
    pub fn device(&self) -> &D {
        &self.dev
    }

    fn cluster_lba(&self, cluster: u32) -> u32 {
        self.data_start + (cluster - 2) * self.sectors_per_cluster
    }

    fn valid_cluster(&self, c: u32) -> bool {
        (2..2 + self.clusters).contains(&c)
    }

    /// Whether directory `dir` is FAT12/16's fixed root.
    fn fixed_root(&self, dir: u32) -> bool {
        dir == 0 && self.root_sectors > 0
    }

    /// Where directory cluster `cluster` lies (its first sector and sector
    /// count); 0 on FAT12/16: the fixed root, whole.
    fn dir_unit(&self, cluster: u32) -> (u32, u32) {
        if self.fixed_root(cluster) {
            (self.root_lba, self.root_sectors)
        } else {
            (self.cluster_lba(cluster), self.sectors_per_cluster)
        }
    }

    /// Most clusters a directory may span ([`DIR_MAX_BYTES`]).
    fn dir_clusters_max(&self) -> u32 {
        (DIR_MAX_BYTES / self.cluster_bytes()).max(1) as u32
    }

    // --- FAT ---------------------------------------------------------------

    fn fat_load(&mut self, sector: u32) -> Result<(), Error> {
        if self.fat_cached == Some(sector) {
            return Ok(());
        }
        self.fat_flush()?;
        self.dev
            .read_blocks(self.fat_start + sector, &mut self.fat_buf)?;
        self.fat_cached = Some(sector);
        Ok(())
    }

    /// Writes the cached FAT sector to every FAT copy.
    fn fat_flush(&mut self) -> Result<(), Error> {
        if let (Some(sector), true) = (self.fat_cached, self.fat_dirty) {
            for copy in 0..self.fats {
                self.dev.write_blocks(
                    self.fat_start + copy * self.fat_sectors + sector,
                    &self.fat_buf,
                )?;
            }
            self.fat_dirty = false;
        }
        Ok(())
    }

    /// Where `cluster`'s entry starts: its FAT sector and the byte in it
    /// (a FAT12 entry at byte 511 runs into the next sector).
    fn fat_at(&self, cluster: u32) -> Result<(u32, usize), Error> {
        if !self.valid_cluster(cluster) {
            return Err(Error::Corrupt);
        }
        let off = self.kind.offset(cluster);
        Ok((off / SECTOR as u32, (off % SECTOR as u32) as usize))
    }

    /// `cluster`'s entry as stored (end marks not widened).
    fn fat_raw(&mut self, cluster: u32) -> Result<u32, Error> {
        let (sector, at) = self.fat_at(cluster)?;
        let mut w = [0u8; 2];
        if at + self.kind.width() as usize <= SECTOR {
            self.fat_load(sector)?;
            return Ok(entry_get(self.kind, &self.fat_buf, at, cluster));
        }
        // Straddling (FAT12 only): its low byte ends this sector, its high
        // byte starts the next.
        self.fat_load(sector)?;
        w[0] = self.fat_buf[SECTOR - 1];
        self.fat_load(sector + 1)?;
        w[1] = self.fat_buf[0];
        Ok(entry_get(self.kind, &w, 0, cluster))
    }

    fn fat_get(&mut self, cluster: u32) -> Result<u32, Error> {
        let raw = self.fat_raw(cluster)?;
        Ok(self.kind.widen(raw))
    }

    fn fat_set(&mut self, cluster: u32, value: u32) -> Result<(), Error> {
        let (sector, at) = self.fat_at(cluster)?;
        if at + self.kind.width() as usize <= SECTOR {
            self.fat_load(sector)?;
            entry_put(self.kind, &mut self.fat_buf, at, cluster, value);
            self.fat_dirty = true;
            return Ok(());
        }
        // Straddling: each half in its own sector, the bits around kept.
        self.fat_load(sector)?;
        let mut w = [self.fat_buf[SECTOR - 1], 0];
        self.fat_load(sector + 1)?;
        w[1] = self.fat_buf[0];
        entry_put(self.kind, &mut w, 0, cluster, value);
        self.fat_buf[0] = w[1];
        self.fat_dirty = true;
        self.fat_load(sector)?;
        self.fat_buf[SECTOR - 1] = w[0];
        self.fat_dirty = true;
        Ok(())
    }

    /// The cluster after `cluster` in its chain, or `None` at the end.
    fn next_cluster(&mut self, cluster: u32) -> Result<Option<u32>, Error> {
        let v = self.fat_get(cluster)?;
        if v >= EOC_MIN {
            Ok(None)
        } else if self.valid_cluster(v) {
            Ok(Some(v))
        } else {
            Err(Error::Corrupt)
        }
    }

    /// Frees a chain from `head` (both FATs).
    fn free_chain(&mut self, head: u32) -> Result<(), Error> {
        let mut c = head;
        let mut freed = 0u32;
        while self.valid_cluster(c) {
            let next = self.fat_get(c)?;
            self.fat_set(c, 0)?;
            freed += 1;
            if next >= EOC_MIN || freed > self.clusters {
                break;
            }
            c = next;
        }
        self.fat_flush()?;
        self.fsinfo_adjust(freed as i64, None)
    }

    /// The first run of `need` (at least one) consecutive free clusters at
    /// or after `from`, wrapping once (a run doesn't wrap past the end), or
    /// `None`.
    ///
    /// The FAT is read through `scratch` in runs of sectors, one command
    /// each: the first run is one sector, and each next one doubles up to
    /// what `scratch` holds, so a hit near `from` costs one short read and a
    /// scan of the whole FAT a few long ones (on the X51v every command
    /// costs ~2.5 ms of start-up, an extra sector ~0.1 ms). The sector in
    /// the write-back cache, which may hold changes not yet written, is
    /// always read from the cache. A `scratch` under one sector falls back
    /// to the cache alone: one sector per command.
    fn find_free_run(
        &mut self,
        from: u32,
        need: u32,
        scratch: &mut [u8],
    ) -> Result<Option<u32>, Error> {
        debug_assert!(need > 0);
        let end = 2 + self.clusters;
        let from = if self.valid_cluster(from) { from } else { 2 };
        // FAT12's entries straddle sectors and its whole FAT is at most 12
        // sectors: the cache alone reads it.
        let max = if self.kind == FatKind::Fat12 {
            0
        } else {
            (scratch.len() / SECTOR) as u32
        };
        let per = SECTOR as u32 / self.kind.width();
        // Sectors held in `scratch`: [first, first + len).
        let (mut first, mut len) = (0u32, 0u32);
        let mut grow = 1u32;
        // Second pass: from the start up to where a run reaching `from`
        // would end (nothing when the first pass began at the start).
        let wrap_end = if from == 2 {
            2
        } else {
            (from + need - 1).min(end)
        };
        for (lo, hi) in [(from, end), (2, wrap_end)] {
            let mut run = 0;
            for c in lo..hi {
                let s = c / per;
                let v = if max == 0 || self.fat_cached == Some(s) {
                    self.fat_get(c)?
                } else {
                    if !(first..first + len).contains(&s) {
                        let n = grow.min(max).min((hi - 1) / per - s + 1);
                        self.dev
                            .read_blocks(self.fat_start + s, &mut scratch[..n as usize * SECTOR])?;
                        (first, len) = (s, n);
                        grow = grow.saturating_mul(2);
                    }
                    let i = (s - first) * per + c % per;
                    let o = (i * self.kind.width()) as usize;
                    entry_get(self.kind, scratch, o, c)
                };
                if v != 0 {
                    run = 0;
                    continue;
                }
                run += 1;
                if run == need {
                    return Ok(Some(c + 1 - need));
                }
            }
        }
        Ok(None)
    }

    // --- FSInfo ------------------------------------------------------------

    /// Adjusts the free-cluster count by `delta` (when it's known) and sets
    /// the next-free hint when given. A damaged FSInfo is left alone.
    fn fsinfo_adjust(&mut self, delta: i64, next_free: Option<u32>) -> Result<(), Error> {
        let Some(lba) = self.fsinfo_lba else {
            return Ok(());
        };
        self.dev.read_blocks(lba, &mut self.buf)?;
        if le32(&self.buf, 0) != 0x4161_5252 || le32(&self.buf, 484) != 0x6141_7272 {
            return Ok(());
        }
        let free = le32(&self.buf, 488);
        if free != FSINFO_UNKNOWN {
            let f = (i64::from(free) + delta).clamp(0, i64::from(self.clusters)) as u32;
            self.buf[488..492].copy_from_slice(&f.to_le_bytes());
        }
        if let Some(n) = next_free {
            self.buf[492..496].copy_from_slice(&n.to_le_bytes());
        }
        self.dev.write_blocks(lba, &self.buf)?;
        Ok(())
    }

    fn fsinfo_hint(&mut self) -> Result<u32, Error> {
        let Some(lba) = self.fsinfo_lba else {
            return Ok(2);
        };
        self.dev.read_blocks(lba, &mut self.buf)?;
        let hint = le32(&self.buf, 492);
        Ok(if self.valid_cluster(hint) { hint } else { 2 })
    }

    // --- Directory ---------------------------------------------------------

    /// Folds a long-name entry into the name being assembled.
    fn take_lfn(&mut self, e: &[u8]) {
        let seq = e[0];
        if seq & 0x40 != 0 {
            self.lfn = [0; NAME_MAX];
            self.lfn_len = 0;
            self.lfn_checksum = e[13];
            self.lfn_valid = true;
        } else if !self.lfn_valid || self.lfn_checksum != e[13] {
            self.lfn_valid = false;
            return;
        }
        let n = usize::from(seq & 0x1F);
        if n == 0 || (n - 1) * 13 >= NAME_MAX {
            self.lfn_valid = n == 0 || self.lfn_valid;
            if n != 0 {
                // Too long to store: keep the entry readable, name truncated.
                self.lfn_len = NAME_MAX;
            }
            return;
        }
        let base = (n - 1) * 13;
        const AT: [usize; 13] = [1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30];
        for (i, &o) in AT.iter().enumerate() {
            let c = le16(e, o);
            if c == 0 {
                break;
            }
            if base + i >= NAME_MAX {
                break;
            }
            self.lfn[base + i] = if c == 0xFFFF {
                0
            } else if c < 0x80 {
                c as u8
            } else {
                b'?'
            };
            if c != 0xFFFF {
                self.lfn_len = self.lfn_len.max(base + i + 1);
            }
        }
    }

    /// Walks the root directory, calling `f` with each entry's sector,
    /// index, raw 32 bytes and assembled long name (empty when it has
    /// none) until `f` returns `true`. Returns whether it did.
    fn walk_dir(&mut self, dir: u32, f: &mut Visit<'_>) -> Result<bool, Error> {
        let mut cluster = dir;
        let fixed = self.fixed_root(dir);
        if !fixed && !self.valid_cluster(cluster) {
            return Err(Error::Corrupt);
        }
        let mut visited = 0u32;
        self.lfn_valid = false;
        self.lfn_len = 0;
        loop {
            let (lba, sectors) = self.dir_unit(cluster);
            for s in 0..sectors {
                self.dev.read_blocks(lba + s, &mut self.buf)?;
                for i in 0..ENTRIES {
                    let mut e = [0u8; 32];
                    e.copy_from_slice(&self.buf[i * 32..i * 32 + 32]);
                    match e[0] {
                        0x00 => return Ok(false),
                        0xE5 => {
                            self.lfn_valid = false;
                            continue;
                        }
                        _ => {}
                    }
                    if e[11] & 0x3F == ATTR_LONG_NAME {
                        self.take_lfn(&e);
                        continue;
                    }
                    let short: [u8; 11] = e[..11].try_into().unwrap_or([b' '; 11]);
                    let has_lfn = self.lfn_valid && self.lfn_checksum == short_checksum(&short);
                    let lfn_len = if has_lfn { self.lfn_len } else { 0 };
                    self.lfn_valid = false;
                    let lfn = self.lfn;
                    if f(lba + s, i as u8, &e, &lfn[..lfn_len]) {
                        return Ok(true);
                    }
                }
            }
            if fixed {
                return Ok(false);
            }
            visited += 1;
            match self.next_cluster(cluster)? {
                // Past the largest directory FAT32 allows (a loop ends here
                // too): damaged.
                Some(c) if visited < self.dir_clusters_max() => cluster = c,
                Some(_) => return Err(Error::Corrupt),
                None => return Ok(false),
            }
        }
    }

    /// Calls `f` for every file and subdirectory in the root directory
    /// (volume labels and deleted entries skipped).
    pub fn for_each_root(&mut self, f: impl FnMut(&DirEntry<'_>)) -> Result<(), Error> {
        self.for_each_in("", f)
    }

    /// Calls `f` for every file and subdirectory in the directory at `path`
    /// (`""` or `"/"`: the root; `.` and `..` entries skipped).
    pub fn for_each_in(
        &mut self,
        path: &str,
        mut f: impl FnMut(&DirEntry<'_>),
    ) -> Result<(), Error> {
        let dir = self.resolve_dir(path)?;
        let kind = self.kind;
        self.walk_dir(dir, &mut |_, _, e, lfn| {
            if e[0] == b'.' {
                return false; // . and ..
            }
            if e[11] & 0x08 != 0 {
                return false; // volume label
            }
            let mut short = [0u8; 12];
            let n = short_to_display(&e[..11], e[12], &mut short);
            let name = if lfn.is_empty() { &short[..n] } else { lfn };
            f(&DirEntry {
                name,
                size: le32(e, 28),
                first_cluster: entry_first(e, kind),
                is_dir: e[11] & ATTR_DIRECTORY != 0,
            });
            false
        })?;
        Ok(())
    }

    /// Finds a file by path (`name`, `dir/name` or `dir\name`; ASCII,
    /// case-insensitive; long or short forms).
    pub fn find(&mut self, path: &str) -> Result<File, Error> {
        let (dir, leaf) = self.parent_of(path)?;
        self.find_in(dir, leaf)
    }

    /// The directory cluster a path's parent names, and its last component.
    fn parent_of<'p>(&mut self, path: &'p str) -> Result<(u32, &'p str), Error> {
        let mut parts = components(path)?;
        let leaf = parts.next_back().ok_or(Error::BadName)?;
        let mut dir = self.root_cluster;
        for d in parts {
            dir = self.lookup_dir(dir, d)?;
        }
        Ok((dir, leaf))
    }

    /// The cluster of the directory at `path` (the root for `""`).
    fn resolve_dir(&mut self, path: &str) -> Result<u32, Error> {
        let mut dir = self.root_cluster;
        for d in components(path)? {
            dir = self.lookup_dir(dir, d)?;
        }
        Ok(dir)
    }

    /// The subdirectory `name` of `dir`.
    fn lookup_dir(&mut self, dir: u32, name: &str) -> Result<u32, Error> {
        match self.find_entry(dir, name, true)?.first_cluster {
            // A subdirectory pointing at the root (or nowhere) is corrupt.
            c if self.valid_cluster(c) && c != self.root_cluster => Ok(c),
            _ => Err(Error::Corrupt),
        }
    }

    /// Finds a file in directory `dir` by name.
    fn find_in(&mut self, dir: u32, name: &str) -> Result<File, Error> {
        check_name(name.as_bytes())?;
        self.find_entry(dir, name, false)
    }

    /// The entry `name` in `dir` that is a directory (`want_dir`) or a file:
    /// one matching walk for both (long or short name, case-insensitive;
    /// volume labels and `.`/`..` never match).
    #[inline(never)]
    fn find_entry(&mut self, dir: u32, name: &str, want_dir: bool) -> Result<File, Error> {
        let want = name.as_bytes();
        let mut found = None;
        let kind = self.kind;
        self.walk_dir(dir, &mut |lba, index, e, lfn| {
            let is_dir = e[11] & ATTR_DIRECTORY != 0;
            if is_dir != want_dir || e[11] & 0x08 != 0 || e[0] == b'.' {
                return false;
            }
            let mut short = [0u8; 12];
            let n = short_to_display(&e[..11], e[12], &mut short);
            let hit = (!lfn.is_empty() && lfn.eq_ignore_ascii_case(want))
                || short[..n].eq_ignore_ascii_case(want);
            if hit {
                found = Some(File {
                    entry_lba: lba,
                    entry_index: index,
                    first_cluster: entry_first(e, kind),
                    size: le32(e, 28),
                });
            }
            hit
        })?;
        found.ok_or(Error::NotFound)
    }

    /// Deletes the file, or the empty folder, at `path`. Crash-safe by
    /// order: the entry goes first (with its long-name entries when they
    /// share its sector: one write, so the name vanishes at once), then the
    /// clusters are freed; a cut in between leaks space, which `check`
    /// reports as lost.
    #[inline(never)]
    pub fn remove(&mut self, path: &str) -> Result<(), Error> {
        let (dir, leaf) = self.parent_of(path)?;
        let (file, is_dir) = match self.find_entry(dir, leaf, false) {
            Ok(f) => (f, false),
            Err(Error::NotFound) => (self.find_entry(dir, leaf, true)?, true),
            Err(e) => return Err(e),
        };
        if is_dir {
            let mut entries = 0u32;
            self.walk_dir(file.first_cluster, &mut |_, _, e, _| {
                if e[0] != b'.' && e[11] & 0x08 == 0 {
                    entries += 1;
                }
                entries > 0
            })?;
            if entries > 0 {
                return Err(Error::NotEmpty);
            }
        }
        self.dev.read_blocks(file.entry_lba, &mut self.buf)?;
        let i = usize::from(file.entry_index) * 32;
        let short: [u8; 11] = self.buf[i..i + 11].try_into().unwrap_or([0; 11]);
        let sum = short_checksum(&short);
        self.buf[i] = 0xE5;
        // Its long-name entries just before it, in this sector.
        let mut k = usize::from(file.entry_index);
        let mut spilled = k == 0;
        while k > 0 {
            k -= 1;
            let o = k * 32;
            if self.buf[o + 11] & 0x3F != ATTR_LONG_NAME
                || self.buf[o + 13] != sum
                || self.buf[o] == 0xE5
            {
                break;
            }
            let last = self.buf[o] & 0x40 != 0;
            self.buf[o] = 0xE5;
            if last {
                break;
            }
            spilled = k == 0;
        }
        self.dev.write_blocks(file.entry_lba, &self.buf)?;
        // ... and those at the end of the sector before, in the same cluster.
        let first_of_cluster = if file.entry_lba < self.data_start {
            file.entry_lba == self.root_lba // FAT12/16's fixed root: one unit
        } else {
            (file.entry_lba - self.data_start).is_multiple_of(self.sectors_per_cluster)
        };
        if spilled && !first_of_cluster {
            let lba = file.entry_lba - 1;
            self.dev.read_blocks(lba, &mut self.buf)?;
            let mut changed = false;
            for k in (0..ENTRIES).rev() {
                let o = k * 32;
                if self.buf[o + 11] & 0x3F != ATTR_LONG_NAME
                    || self.buf[o + 13] != sum
                    || self.buf[o] == 0xE5
                {
                    break;
                }
                let last = self.buf[o] & 0x40 != 0;
                self.buf[o] = 0xE5;
                changed = true;
                if last {
                    break;
                }
            }
            if changed {
                self.dev.write_blocks(lba, &self.buf)?;
            }
        }
        if file.first_cluster != 0 {
            self.free_chain(file.first_cluster)?;
        }
        Ok(())
    }

    // --- Reading -----------------------------------------------------------

    /// Reads `file` in order, calling `f` with each piece; `scratch` (whole
    /// sectors, at least one) bounds how much is read per device command,
    /// so a larger scratch means fewer, longer transfers.
    ///
    /// # Panics
    /// If `scratch` is shorter than a sector.
    #[inline(never)]
    pub fn read_file(
        &mut self,
        file: &File,
        scratch: &mut [u8],
        mut f: impl FnMut(&[u8]),
    ) -> Result<(), Error> {
        let max_sectors = scratch.len() / SECTOR;
        assert!(max_sectors >= 1, "scratch must hold a sector");
        let mut remaining = file.size as usize;
        if remaining == 0 {
            return Ok(());
        }
        let mut cluster = file.first_cluster;
        let mut hops = 0u32;
        while remaining > 0 {
            if !self.valid_cluster(cluster) {
                return Err(Error::Corrupt);
            }
            // Extend the run while the chain stays consecutive and the
            // scratch has room.
            let run_start = cluster;
            let mut run = 1u32;
            let mut next = self.next_cluster(cluster)?;
            let max_clusters = (max_sectors as u32 / self.sectors_per_cluster).max(1);
            while let Some(n) = next
                && n == cluster + 1
                && run < max_clusters
            {
                cluster = n;
                run += 1;
                next = self.next_cluster(cluster)?;
            }
            let bytes = (run as usize * self.cluster_bytes()).min(remaining);
            let sectors = bytes.div_ceil(SECTOR).min(max_sectors);
            let want = (sectors * SECTOR).min(bytes);
            self.dev.read_blocks(
                self.cluster_lba(run_start),
                &mut scratch[..sectors * SECTOR],
            )?;
            f(&scratch[..want]);
            remaining -= want;
            if want < bytes {
                // The scratch is smaller than a cluster: go on inside it.
                let mut off = want;
                while off < bytes {
                    let n = (bytes - off).min(max_sectors * SECTOR);
                    let secs = n.div_ceil(SECTOR);
                    self.dev.read_blocks(
                        self.cluster_lba(run_start) + (off / SECTOR) as u32,
                        &mut scratch[..secs * SECTOR],
                    )?;
                    f(&scratch[..n]);
                    off += n;
                    remaining -= n;
                }
            }
            hops += run;
            if hops > self.clusters {
                return Err(Error::Corrupt);
            }
            match next {
                Some(n) => cluster = n,
                None if remaining == 0 => break,
                None => return Err(Error::Corrupt),
            }
        }
        Ok(())
    }

    // --- Writing -----------------------------------------------------------

    /// Replaces the contents of the root-directory file `name` with `data`
    /// by copy-then-swap (see the crate docs). The old contents stay on the
    /// card, untouched, until the new ones are complete and linked.
    ///
    /// `scratch` is borrowed to search the FAT for free clusters in runs of
    /// sectors; the larger it is (up to the FAT's size), the fewer device
    /// commands a long search takes. Any length works: under a sector, the
    /// FAT is read one sector per command.
    pub fn replace(
        &mut self,
        name: &str,
        data: &[u8],
        scratch: &mut [u8],
    ) -> Result<Written, Error> {
        self.replace_fill(name, Fill::Bytes(data), scratch)
    }

    fn replace_fill(
        &mut self,
        name: &str,
        data: Fill<'_>,
        scratch: &mut [u8],
    ) -> Result<Written, Error> {
        let file = self.find(name)?;
        let hint = self.fsinfo_hint()?;
        let written = self.store(data, hint, scratch)?;

        // The swap: one sector write.
        self.dev.read_blocks(file.entry_lba, &mut self.buf)?;
        let o = usize::from(file.entry_index) * 32;
        let size = u32::try_from(data.len()).map_err(|_| Error::NoRoom)?;
        self.buf[o + 20..o + 22]
            .copy_from_slice(&((written.first_cluster >> 16) as u16).to_le_bytes());
        self.buf[o + 26..o + 28].copy_from_slice(&(written.first_cluster as u16).to_le_bytes());
        self.buf[o + 28..o + 32].copy_from_slice(&size.to_le_bytes());
        self.buf[o + 11] |= 0x20; // archive: changed since the last backup
        self.dev.write_blocks(file.entry_lba, &self.buf)?;

        self.fsinfo_adjust(
            -i64::from(written.clusters),
            Some(self.next_hint(&written, hint)),
        )?;
        if file.first_cluster != 0 {
            self.free_chain(file.first_cluster)?;
        }
        Ok(written)
    }

    /// Replaces `path`'s contents, or creates it (and any missing parent
    /// directories, like `mkdir -p`) when it doesn't exist.
    #[inline(never)]
    pub fn put(&mut self, path: &str, data: &[u8], scratch: &mut [u8]) -> Result<Written, Error> {
        self.put_fill(path, Fill::Bytes(data), scratch)
    }

    /// Replaces or creates `path` (as [`Self::put`]) holding `bytes` zeros
    /// in one run of clusters, and returns where: see [`Reserved`]. A
    /// volume with no free run that long gives [`Error::Fragmented`] (the
    /// file is removed again: nothing is left behind).
    ///
    /// # Panics
    /// If `scratch` is shorter than a sector.
    pub fn reserve(
        &mut self,
        path: &str,
        bytes: usize,
        scratch: &mut [u8],
    ) -> Result<Reserved, Error> {
        let w = self.put_fill(path, Fill::Zeros(bytes), scratch)?;
        if !w.contiguous || w.first_cluster == 0 {
            self.remove(path)?;
            return Err(Error::Fragmented);
        }
        Ok(Reserved {
            lba: self.cluster_lba(w.first_cluster),
            sectors: bytes.div_ceil(SECTOR) as u32,
        })
    }

    /// Where an existing file lies, if its clusters are one run: its first
    /// sector and its sectors (the size rounded up), as [`Self::reserve`]
    /// gives for a new one, without changing anything on the card (the run
    /// logs reuse their files: reminder R22, the PC's cached view stays
    /// right). [`Error::Fragmented`] if the clusters aren't one run,
    /// [`Error::NotFound`] if there's no such file or it's empty.
    pub fn extent(&mut self, path: &str) -> Result<Reserved, Error> {
        let f = self.find(path)?;
        if f.first_cluster < 2 || f.size == 0 {
            return Err(Error::NotFound);
        }
        let cluster_bytes = self.sectors_per_cluster * SECTOR as u32;
        let clusters = f.size.div_ceil(cluster_bytes);
        let mut c = f.first_cluster;
        for _ in 1..clusters {
            match self.next_cluster(c)? {
                Some(n) if n == c + 1 => c = n,
                _ => return Err(Error::Fragmented),
            }
        }
        Ok(Reserved {
            lba: self.cluster_lba(f.first_cluster),
            sectors: f.size.div_ceil(SECTOR as u32),
        })
    }

    fn put_fill(
        &mut self,
        path: &str,
        data: Fill<'_>,
        scratch: &mut [u8],
    ) -> Result<Written, Error> {
        match self.replace_fill(path, data, scratch) {
            Err(Error::NotFound) => {
                let mut parts = components(path)?;
                parts.next_back();
                for d in parts {
                    match self.mkdir(prefix_to(path, d)?, scratch) {
                        Ok(()) | Err(Error::Exists) => {}
                        Err(e) => return Err(e),
                    }
                }
                self.create_fill(path, data, scratch)
            }
            r => r,
        }
    }

    /// Creates the directory `path` (its parent must exist): its first
    /// cluster with `.` and `..` is written and linked before the entry
    /// that makes it exist. An existing directory is [`Error::Exists`].
    pub fn mkdir(&mut self, path: &str, scratch: &mut [u8]) -> Result<(), Error> {
        let (parent, leaf) = self.parent_of(path)?;
        check_new_name(leaf.as_bytes())?;
        match self.lookup_dir(parent, leaf) {
            Ok(_) => return Err(Error::Exists),
            Err(Error::NotFound) => {}
            Err(e) => return Err(e),
        }
        if self.find_in(parent, leaf).is_ok() {
            return Err(Error::Exists); // a file of that name
        }
        let hint = self.fsinfo_hint()?;
        let Some(c) = self.find_free_run(hint, 1, scratch)? else {
            return Err(Error::NoRoom);
        };
        // The new directory's first cluster: . and .., then empty.
        let lba = self.cluster_lba(c);
        for sec in 0..self.sectors_per_cluster {
            self.buf = [0; SECTOR];
            if sec == 0 {
                let dot = |name: &[u8; 11], cluster: u32| {
                    let mut e = [0u8; 32];
                    e[..11].copy_from_slice(name);
                    e[11] = ATTR_DIRECTORY;
                    e[20..22].copy_from_slice(&((cluster >> 16) as u16).to_le_bytes());
                    e[26..28].copy_from_slice(&(cluster as u16).to_le_bytes());
                    e
                };
                // `..` of a top-level directory is cluster 0 (the root).
                let up = if parent == self.root_cluster {
                    0
                } else {
                    parent
                };
                self.buf[..32].copy_from_slice(&dot(b".          ", c));
                self.buf[32..64].copy_from_slice(&dot(b"..         ", up));
            }
            self.dev.write_blocks(lba + sec, &self.buf)?;
        }
        self.fat_set(c, EOC)?;
        self.fat_flush()?;
        match self.add_entry(parent, leaf, ATTR_DIRECTORY, c, 0, scratch) {
            Ok(grown) => {
                self.fsinfo_adjust(-i64::from(1 + grown), Some(c + 1))?;
                Ok(())
            }
            Err(e) => {
                self.free_chain(c)?;
                Err(e)
            }
        }
    }

    /// Creates the root-directory file `name` holding `data`. The contents
    /// and their FAT links are written first, the directory entries last
    /// (the short entry, which makes the file exist, in the last sector
    /// written), so a power cut leaves no file or the whole file; at worst
    /// some clusters stay allocated to nothing, which a disk check frees.
    /// A full root directory grows by a cluster.
    pub fn create(
        &mut self,
        path: &str,
        data: &[u8],
        scratch: &mut [u8],
    ) -> Result<Written, Error> {
        self.create_fill(path, Fill::Bytes(data), scratch)
    }

    fn create_fill(
        &mut self,
        path: &str,
        data: Fill<'_>,
        scratch: &mut [u8],
    ) -> Result<Written, Error> {
        let (dir, leaf) = self.parent_of(path)?;
        check_new_name(leaf.as_bytes())?;
        match self.find_in(dir, leaf) {
            Ok(_) => return Err(Error::Exists),
            Err(Error::NotFound) => {}
            Err(e) => return Err(e),
        }
        if self.lookup_dir(dir, leaf).is_ok() {
            return Err(Error::Exists); // a directory of that name
        }
        let hint = self.fsinfo_hint()?;
        let written = self.store(data, hint, scratch)?;
        let size = u32::try_from(data.len()).map_err(|_| Error::NoRoom)?;
        match self.add_entry(dir, leaf, 0x20, written.first_cluster, size, scratch) {
            Ok(grown) => {
                self.fsinfo_adjust(
                    -i64::from(written.clusters + grown),
                    Some(self.next_hint(&written, hint)),
                )?;
                Ok(written)
            }
            Err(err) => {
                if written.first_cluster != 0 {
                    self.free_chain(written.first_cluster)?;
                }
                Err(err)
            }
        }
    }

    /// Writes the directory entries (long-name entries when needed, then
    /// the short entry, in the last sector written) for `leaf` in `dir`;
    /// returns how many clusters the directory grew by.
    fn add_entry(
        &mut self,
        dir: u32,
        leaf: &str,
        attr: u8,
        first_cluster: u32,
        size: u32,
        scratch: &mut [u8],
    ) -> Result<u32, Error> {
        let n = leaf.as_bytes();
        // The short name: the 8.3 form itself, or BASIS~N with long-name
        // entries, N the first free number.
        let (short, nt, lfn) = match plain_short(n) {
            Some((s, nt)) => (s, nt, false),
            None => {
                let mut k = 1;
                loop {
                    let s = numbered_short(n, k);
                    if !self.short_exists(dir, &s)? {
                        break (s, 0, true);
                    }
                    k += 1;
                    if k > 999_999 {
                        return Err(Error::NoRoom);
                    }
                }
            }
        };
        let mut entries = [[0u8; 32]; LFN_MAX + 1];
        let mut count = 0;
        if lfn {
            let mut l = [[0u8; 32]; LFN_MAX];
            count = lfn_entries(n, short_checksum(&short), &mut l);
            entries[..count].copy_from_slice(&l[..count]);
        }

        let e = &mut entries[count];
        e[..11].copy_from_slice(&short);
        e[11] = attr;
        e[12] = nt;
        e[20..22].copy_from_slice(&((first_cluster >> 16) as u16).to_le_bytes());
        e[26..28].copy_from_slice(&(first_cluster as u16).to_le_bytes());
        e[28..32].copy_from_slice(&size.to_le_bytes());
        count += 1;

        let mut grown = 0;
        let slots = self.free_slots(dir, count, scratch, &mut grown)?;
        // Sector by sector, in order: the short entry is the last slot, so
        // its sector is written last.
        let mut i = 0;
        while i < count {
            let lba = slots[i].0;
            self.dev.read_blocks(lba, &mut self.buf)?;
            while i < count && slots[i].0 == lba {
                let o = usize::from(slots[i].1) * 32;
                self.buf[o..o + 32].copy_from_slice(&entries[i]);
                i += 1;
            }
            self.dev.write_blocks(lba, &self.buf)?;
        }
        Ok(grown)
    }

    fn next_hint(&self, w: &Written, hint: u32) -> u32 {
        if w.clusters > 0 {
            w.first_cluster + if w.contiguous { w.clusters } else { 0 }
        } else {
            hint
        }
    }

    /// Whether an entry of `dir` has the short name `short`.
    fn short_exists(&mut self, dir: u32, short: &[u8; 11]) -> Result<bool, Error> {
        self.walk_dir(dir, &mut |_, _, e, _| e[..11] == short[..])
    }

    /// Finds `need` consecutive free slots in directory `dir` (deleted or
    /// past its end), growing it by a zeroed cluster if it has none;
    /// returns their (sector, index), in order.
    fn free_slots(
        &mut self,
        dir: u32,
        need: usize,
        scratch: &mut [u8],
        grown: &mut u32,
    ) -> Result<[(u32, u8); LFN_MAX + 1], Error> {
        let mut run = [(0u32, 0u8); LFN_MAX + 1];
        let mut len = 0;
        let mut cluster = dir;
        let mut visited = 0u32;
        loop {
            let (lba, sectors) = self.dir_unit(cluster);
            for s in 0..sectors {
                self.dev.read_blocks(lba + s, &mut self.buf)?;
                for i in 0..ENTRIES {
                    if matches!(self.buf[i * 32], 0x00 | 0xE5) {
                        run[len] = (lba + s, i as u8);
                        len += 1;
                        if len == need {
                            return Ok(run);
                        }
                    } else {
                        len = 0;
                    }
                }
            }
            // FAT12/16's fixed root never grows.
            if self.fixed_root(dir) {
                return Err(Error::NoRoom);
            }
            visited += 1;
            match self.next_cluster(cluster)? {
                Some(c) if visited < self.dir_clusters_max() => cluster = c,
                Some(_) => return Err(Error::Corrupt),
                // Full to FAT32's limit: no room to grow.
                None if visited >= self.dir_clusters_max() => return Err(Error::NoRoom),
                None => {
                    // Grow: a zeroed cluster linked after the last one; the
                    // run continues into it.
                    let hint = self.fsinfo_hint()?;
                    let Some(c) = self.find_free_run(hint, 1, scratch)? else {
                        return Err(Error::NoRoom);
                    };
                    self.buf = [0; SECTOR];
                    for s in 0..self.sectors_per_cluster {
                        self.dev.write_blocks(self.cluster_lba(c) + s, &self.buf)?;
                    }
                    self.fat_set(c, EOC)?;
                    self.fat_set(cluster, c)?;
                    self.fat_flush()?;
                    *grown += 1;
                    cluster = c;
                }
            }
        }
    }

    /// Allocates clusters for `data` (one contiguous run when there is one,
    /// searching from `hint`), writes it and links the chain in every FAT
    /// copy. The data is reachable only through what the caller links next.
    fn store(&mut self, data: Fill<'_>, hint: u32, scratch: &mut [u8]) -> Result<Written, Error> {
        let cb = self.cluster_bytes();
        let need = u32::try_from(data.len().div_ceil(cb)).map_err(|_| Error::NoRoom)?;

        let mut written = Written {
            first_cluster: 0,
            clusters: need,
            contiguous: true,
        };
        if need > 0 {
            match self.find_free_run(hint, need, scratch)? {
                Some(start) => {
                    // One run: write it as few long transfers as the data
                    // allows (whole clusters in one go), then link it.
                    written.first_cluster = start;
                    let lba = self.cluster_lba(start);
                    data.write(&mut self.dev, lba, scratch)?;
                    for c in start..start + need {
                        self.fat_set(c, if c + 1 == start + need { EOC } else { c + 1 })?;
                    }
                }
                None => {
                    written.contiguous = false;
                    let mut prev = None;
                    let mut from = hint;
                    for at in (0..data.len()).step_by(cb) {
                        let chunk = data.part(at, cb.min(data.len() - at));
                        let Some(c) = self.find_free_run(from, 1, scratch)? else {
                            // Out of space part way: give back what was
                            // taken (the old file is still whole).
                            if written.first_cluster != 0 {
                                self.free_chain(written.first_cluster)?;
                            }
                            return Err(Error::NoRoom);
                        };
                        let lba = self.cluster_lba(c);
                        chunk.write(&mut self.dev, lba, scratch)?;
                        self.fat_set(c, EOC)?;
                        match prev {
                            Some(p) => self.fat_set(p, c)?,
                            None => written.first_cluster = c,
                        }
                        prev = Some(c);
                        from = c + 1;
                    }
                }
            }
        }
        self.fat_flush()?;
        Ok(written)
    }
}

/// `NAME.EXT` from an 8.3 entry name, in lower case where Windows NT's case
/// flags (`nt`: 0x08 base, 0x10 extension) say so; returns the length.
fn short_to_display(short: &[u8], nt: u8, out: &mut [u8; 12]) -> usize {
    let lower = |b: u8, flag: u8| {
        if nt & flag != 0 {
            b.to_ascii_lowercase()
        } else {
            b
        }
    };
    let mut n = 0;
    for &b in short[..8].iter().take_while(|&&b| b != b' ') {
        out[n] = if b == 0x05 { 0xE5 } else { lower(b, 0x08) };
        n += 1;
    }
    let ext: &[u8] = &short[8..11];
    let ext_len = ext.iter().take_while(|&&b| b != b' ').count();
    if ext_len > 0 {
        out[n] = b'.';
        n += 1;
        for (d, &b) in out[n..n + ext_len].iter_mut().zip(ext) {
            *d = lower(b, 0x10);
        }
        n += ext_len;
    }
    n
}

#[cfg(any(test, feature = "std"))]
pub mod format;

pub mod exfat;
pub mod mkfs;
pub mod survey;

pub mod check;
pub mod guard;
pub use check::Health;
pub use guard::Guard;

/// A FAT (12, 16 or 32) or exFAT volume, whichever the boot sector says: what a caller
/// that only stores and reads files needs (the loader's `put`, crash
/// reports), so SDXC cards work wherever SD and SDHC cards do.
// No heap to box the larger variant into; one lives briefly on the stack.
#[allow(clippy::large_enum_variant)]
pub enum AnyVolume<D> {
    /// FAT12, FAT16 or FAT32 (SD and SDHC cards, CF cards, old cards).
    Fat(Volume<D>),
    /// exFAT (SDXC cards).
    ExFat(exfat::ExVolume<D>),
}

impl<D: BlockDevice> AnyVolume<D> {
    /// Mounts the volume whose boot sector is at `base`.
    pub fn mount(mut dev: D, base: u32) -> Result<Self, Error> {
        let mut boot = [0u8; SECTOR];
        dev.read_blocks(base, &mut boot)?;
        if &boot[3..11] == b"EXFAT   " {
            exfat::ExVolume::mount(dev, base).map(Self::ExFat)
        } else {
            Volume::mount(dev, base).map(Self::Fat)
        }
    }

    /// Replaces or creates the file (crash-safe on either). exFAT: root
    /// directory only for now.
    pub fn put(&mut self, path: &str, data: &[u8], scratch: &mut [u8]) -> Result<Written, Error> {
        match self {
            Self::Fat(v) => v.put(path, data, scratch),
            Self::ExFat(v) => v.put(path, data, scratch),
        }
    }

    /// Reserves a zero-filled file in one run of sectors (see
    /// [`Reserved`]).
    pub fn reserve(
        &mut self,
        path: &str,
        bytes: usize,
        scratch: &mut [u8],
    ) -> Result<Reserved, Error> {
        match self {
            Self::Fat(v) => v.reserve(path, bytes, scratch),
            Self::ExFat(v) => v.reserve(path, bytes, scratch),
        }
    }

    /// Where an existing file lies if contiguous (`Volume::extent`).
    pub fn extent(&mut self, path: &str) -> Result<Reserved, Error> {
        match self {
            Self::Fat(v) => v.extent(path),
            Self::ExFat(v) => v.extent(path),
        }
    }

    /// Reads the file in order, calling `f` with each piece.
    ///
    /// # Panics
    /// If `scratch` is shorter than a sector.
    pub fn read_back(
        &mut self,
        path: &str,
        scratch: &mut [u8],
        f: &mut dyn FnMut(&[u8]),
    ) -> Result<(), Error> {
        match self {
            Self::Fat(v) => {
                let file = v.find(path)?;
                v.read_file(&file, scratch, f)
            }
            Self::ExFat(v) => {
                let file = v.find(path)?;
                v.read_file(&file, scratch, f)
            }
        }
    }

    /// Deletes the file or empty folder at `path` (crash-safe on either).
    pub fn remove(&mut self, path: &str) -> Result<(), Error> {
        match self {
            Self::Fat(v) => v.remove(path),
            Self::ExFat(v) => v.remove(path),
        }
    }

    /// The boot-file guard for whichever format the volume is (see
    /// [`crate::guard`]).
    pub fn guard(&mut self, names: &[&str]) -> Result<Guard, Error> {
        match self {
            Self::Fat(v) => v.guard(names),
            Self::ExFat(v) => v.guard(names),
        }
    }

    /// Checks the volume, read-only ([`check`]).
    pub fn check(&mut self, seen: &mut [u8], scratch: &mut [u8]) -> Result<Health, Error> {
        match self {
            Self::Fat(v) => v.check(seen, scratch),
            Self::ExFat(v) => v.check(seen, scratch),
        }
    }

    /// Calls `f` with each entry of the folder `path` ("" for the root):
    /// its name, whether it is a folder, and its size in bytes.
    pub fn list(&mut self, path: &str, f: &mut dyn FnMut(&[u8], bool, u64)) -> Result<(), Error> {
        match self {
            Self::Fat(v) => v.for_each_in(path, |e| f(e.name, e.is_dir, u64::from(e.size))),
            Self::ExFat(v) => v.for_each_in(path, |n, e| f(n, e.is_dir, e.size)),
        }
    }

    /// The block device.
    pub fn device(&self) -> &D {
        match self {
            Self::Fat(v) => v.device(),
            Self::ExFat(v) => v.device(),
        }
    }
}

#[cfg(test)]
mod tests;
