//! The formatter (ADR-0036 point 3): FAT12, FAT16, FAT32 and exFAT, written
//! to any [`BlockDevice`] on the device itself, with no memory beyond the
//! caller's scratch.
//!
//! [`plan`] computes and checks a layout before anything is written: an MBR
//! with one partition, the partition and the data area (exFAT: the cluster
//! heap) aligned to the erase unit, as the SD Association's formatter does
//! and Windows doesn't, so every cluster lies inside one erase block. A
//! cluster count outside the format's range, a cluster size it can't have,
//! a bad label or a card too small is an error that says which, and nothing
//! is written. [`write`] then writes it in multi-block runs:
//!
//! 1. the old boot sector (where the old MBR's partition starts) and the
//!    new one are cleared first, so a format cut short is never mounted as
//!    the old file system over a half-written new FAT;
//! 2. the FATs (and FAT12/16's fixed root) zeroed, then their first
//!    entries, the root directory and its label, FAT32's FSInfo, exFAT's
//!    allocation bitmap, up-case table and FAT chains;
//! 3. the MBR;
//! 4. the boot sectors last, backups before the main one.
//!
//! A quick format: data clusters are not erased (what was in them is
//! unreachable). exFAT gets Microsoft's recommended up-case table (exFAT
//! specification section 7.2.5.1: 5,836 bytes, checksum `0xE619D30D`),
//! stored here as [`UPCASE`]'s ranges and expanded as it is written.
//!
//! Security (docs/security.md): nothing is parsed from the card but the old
//! MBR's first partition start, used only as a sector to clear, and only
//! inside the card.

use rp_hal_api::BlockDevice;

use crate::exfat::boot_checksum;
use crate::{Error, FatKind, SECTOR};

/// A file system the formatter writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fs {
    /// Under 4,085 clusters.
    Fat12,
    /// 4,085 to 65,524 clusters.
    Fat16,
    /// 65,525 clusters and up.
    Fat32,
    /// SDXC's format; Windows Mobile 5/6 can't read it.
    ExFat,
}

impl Fs {
    /// The name the loader prints and parses.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Fat12 => "FAT12",
            Self::Fat16 => "FAT16",
            Self::Fat32 => "FAT32",
            Self::ExFat => "exFAT",
        }
    }

    /// The file system a name means (`fat12`, `FAT16`, `exfat`...).
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        [Self::Fat12, Self::Fat16, Self::Fat32, Self::ExFat]
            .into_iter()
            .find(|f| f.name().eq_ignore_ascii_case(name))
    }

    fn kind(self) -> FatKind {
        match self {
            Self::Fat12 => FatKind::Fat12,
            Self::Fat16 => FatKind::Fat16,
            _ => FatKind::Fat32,
        }
    }

    /// The cluster counts the format allows: FAT's by Microsoft's FAT
    /// specification ("FAT type determination"), exFAT's by its own (the
    /// ClusterCount field, section 3.1.9).
    fn counts(self) -> core::ops::Range<u32> {
        match self {
            Self::Fat12 => 1..crate::FAT12_LIMIT,
            Self::Fat16 => crate::FAT12_LIMIT..crate::FAT16_LIMIT,
            Self::Fat32 => crate::FAT16_LIMIT..crate::MAX_CLUSTERS + 1,
            Self::ExFat => 1..0xFFFF_FFF6,
        }
    }

    /// The largest cluster, in sectors: FAT's 64 KiB (Windows NT's limit;
    /// 32 KiB is the portable one, the largest any default here picks),
    /// exFAT's 32 MiB.
    fn max_cluster(self) -> u32 {
        match self {
            Self::ExFat => 1 << 16,
            _ => 128,
        }
    }

    /// The MBR partition type: FAT12, FAT16 (under or over 32 MiB), FAT32
    /// with LBA, exFAT.
    fn partition_type(self, sectors: u32) -> u8 {
        match self {
            Self::Fat12 => 0x01,
            Self::Fat16 if sectors < 65_536 => 0x04,
            Self::Fat16 => 0x06,
            Self::Fat32 => 0x0C,
            Self::ExFat => 0x07,
        }
    }
}

/// What to format: `None` fields are chosen by [`plan`].
#[derive(Clone, Copy, Debug)]
pub struct Options<'a> {
    /// The file system; `None`: by the card's size (ADR-0036 Q1 A).
    pub fs: Option<Fs>,
    /// Bytes per cluster (a power of two); `None`: by size (Q2 A).
    pub cluster_bytes: Option<u32>,
    /// The erase unit, in sectors (a power of two); `None`: by size (Q3 A).
    pub align_sectors: Option<u32>,
    /// The volume label: up to 11 characters, ASCII, none of `"*+,./:;<=>?[\]|`.
    pub label: &'a str,
    /// The volume serial number (the caller's clock or counter).
    pub serial: u32,
}

/// Why a layout can't be made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanError {
    /// The card is too small for the format.
    TooSmall,
    /// The cluster size isn't a power of two in the format's range.
    ClusterSize,
    /// The card at that cluster size has too few or too many clusters for
    /// the format (pick the other FAT, or another cluster size).
    ClusterCount,
    /// The erase unit isn't a power of two, or is too large to pad to.
    Alignment,
    /// The label is too long or has a character FAT doesn't allow.
    Label,
}

impl PlanError {
    /// A short fixed name, for the loader's reply.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::TooSmall => "card too small for that format",
            Self::ClusterSize => "cluster size not allowed for that format",
            Self::ClusterCount => "cluster count outside that format's range",
            Self::Alignment => "bad erase unit",
            Self::Label => "bad label",
        }
    }
}

/// A checked layout, ready for [`write`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    /// The file system.
    pub fs: Fs,
    /// The card's sectors.
    pub total: u32,
    /// The partition's first sector (the boot sector) and its length.
    pub part_start: u32,
    /// Sectors in the partition.
    pub part_sectors: u32,
    /// Sectors per cluster.
    pub sectors_per_cluster: u32,
    /// From the partition's start to the first FAT (FAT: the reserved
    /// sectors; exFAT: FatOffset).
    pub fat_offset: u32,
    /// FAT copies (2 for FAT, 1 for exFAT).
    pub fats: u32,
    /// Sectors per FAT.
    pub fat_sectors: u32,
    /// FAT12/16's fixed root directory, in sectors (0 otherwise).
    pub root_sectors: u32,
    /// From the partition's start to the data area (exFAT: the cluster heap).
    pub data_offset: u32,
    /// Data clusters.
    pub clusters: u32,
    /// The erase unit everything is aligned to, in sectors.
    pub align: u32,
    label: [u8; 11],
    label_len: usize,
    serial: u32,
}

/// FAT12/16's root directory: 512 entries (what every formatter writes).
const ROOT_ENTRIES: u32 = 512;
/// exFAT's boot region: main and backup, 12 sectors each.
const EX_BOOT: u32 = 24;
/// exFAT's smallest volume, 1 MiB (the specification's VolumeLength minimum).
const EX_MIN: u32 = (1 << 20) / SECTOR as u32;

/// The defaults by size (ADR-0036 Q1 A, Q2 A: the SD Association's rule;
/// its tables as recalled, to be checked against the published File System
/// Specification: quality audit C84): the format and bytes per cluster.
fn defaults(total: u32) -> (Fs, u32) {
    const MIB: u64 = 1 << 20;
    let bytes = u64::from(total) * SECTOR as u64;
    if bytes <= 8 * MIB {
        (Fs::Fat12, 8 << 10)
    } else if bytes <= 64 * MIB {
        (Fs::Fat12, 16 << 10)
    } else if bytes <= 1024 * MIB {
        (Fs::Fat16, 16 << 10)
    } else if bytes <= 2048 * MIB {
        (Fs::Fat16, 32 << 10)
    } else if bytes <= 32 * 1024 * MIB {
        (Fs::Fat32, 32 << 10)
    } else if bytes <= 512 * 1024 * MIB {
        (Fs::ExFat, 32 << 10)
    } else {
        (Fs::ExFat, 128 << 10)
    }
}

/// The erase unit by size (ADR-0036 Q3 A): the largest power of two at
/// most a 256th of the card, at most 4 MiB, at least a cluster.
fn default_align(total: u32, spc: u32) -> u32 {
    let cap = (total / 256).max(1);
    let p = 1u32 << (31 - cap.leading_zeros());
    p.min(8192).max(spc)
}

/// The label as stored: upper-case for FAT (`fat`), as given for exFAT.
fn check_label(label: &str) -> Result<([u8; 11], usize), PlanError> {
    let b = label.as_bytes();
    if b.len() > 11
        || b.iter()
            .any(|&c| !(0x20..0x7F).contains(&c) || b"\"*+,./:;<=>?[\\]|".contains(&c))
    {
        return Err(PlanError::Label);
    }
    let mut out = [b' '; 11];
    out[..b.len()].copy_from_slice(b);
    Ok((out, b.len()))
}

/// Computes and checks the layout for a card of `total` sectors.
///
/// # Errors
/// Which choice can't work: see [`PlanError`].
pub fn plan(total: u32, opts: &Options<'_>) -> Result<Layout, PlanError> {
    let (label, label_len) = check_label(opts.label)?;
    if let Some(a) = opts.align_sectors
        && (a == 0 || !a.is_power_of_two())
    {
        return Err(PlanError::Alignment);
    }
    let spc_of = |bytes: u32| -> Result<u32, PlanError> {
        if bytes < SECTOR as u32 || !bytes.is_power_of_two() {
            return Err(PlanError::ClusterSize);
        }
        Ok(bytes / SECTOR as u32)
    };
    let (auto_fs, auto_bytes) = defaults(total);
    let build = |fs: Fs, spc: u32| -> Result<Layout, PlanError> {
        if spc > fs.max_cluster() {
            return Err(PlanError::ClusterSize);
        }
        let align = opts.align_sectors.unwrap_or(default_align(total, spc));
        let mut l = layout(fs, total, spc, align)?;
        l.label = label;
        l.label_len = label_len;
        l.serial = opts.serial;
        Ok(l)
    };
    match (opts.fs, opts.cluster_bytes) {
        (fs, Some(bytes)) => {
            let spc = spc_of(bytes)?;
            match fs {
                Some(fs) => build(fs, spc),
                // A cluster size but no format: the size's format, or the
                // other FAT if the count says so.
                None => build(auto_fs, spc).or_else(|e| match (e, auto_fs) {
                    (PlanError::ClusterCount, Fs::Fat12) => build(Fs::Fat16, spc),
                    (PlanError::ClusterCount, Fs::Fat16) => build(Fs::Fat12, spc),
                    _ => Err(e),
                }),
            }
        }
        (None, None) => {
            let spc = auto_bytes / SECTOR as u32;
            build(auto_fs, spc).or_else(|e| match (e, auto_fs) {
                (PlanError::ClusterCount, Fs::Fat12) => build(Fs::Fat16, spc),
                (PlanError::ClusterCount, Fs::Fat16) => build(Fs::Fat12, spc),
                _ => Err(e),
            })
        }
        (Some(fs), None) => {
            // The size's cluster when it suits the format, else the nearest
            // that does: smaller first (less slack), then larger, never past
            // 32 KiB for FAT (the portable limit).
            let start_bytes = if fs == auto_fs {
                auto_bytes
            } else if fs == Fs::ExFat {
                32 << 10
            } else {
                auto_bytes.min(32 << 10)
            };
            let start = start_bytes / SECTOR as u32;
            let top = if fs == Fs::ExFat {
                fs.max_cluster()
            } else {
                64
            };
            let mut err = PlanError::ClusterCount;
            let down = core::iter::successors(Some(start), |&s| (s > 1).then_some(s / 2));
            let up = core::iter::successors(Some(start * 2), |&s| (s < top).then_some(s * 2))
                .filter(|&s| s <= top);
            for spc in down.chain(up) {
                match build(fs, spc) {
                    Ok(l) => return Ok(l),
                    Err(e @ (PlanError::Label | PlanError::Alignment)) => return Err(e),
                    Err(e) => err = e,
                }
            }
            Err(err)
        }
    }
}

/// The layout of `fs` with `spc` sectors per cluster, aligned to `align`.
fn layout(fs: Fs, total: u32, spc: u32, align: u32) -> Result<Layout, PlanError> {
    let part_start = align;
    let part = total.checked_sub(part_start).ok_or(PlanError::TooSmall)?;
    let mut l = Layout {
        fs,
        total,
        part_start,
        part_sectors: part,
        sectors_per_cluster: spc,
        fat_offset: 0,
        fats: 2,
        fat_sectors: 0,
        root_sectors: 0,
        data_offset: 0,
        clusters: 0,
        align,
        label: [b' '; 11],
        label_len: 0,
        serial: 0,
    };
    let round_up = |n: u32| -> Option<u32> { n.checked_next_multiple_of(align) };
    if fs == Fs::ExFat {
        if part < EX_MIN {
            return Err(PlanError::TooSmall);
        }
        // The FAT sized for every cluster the heap could hold, then the heap
        // aligned after it (which only leaves fewer clusters).
        let est = (part - EX_BOOT) / spc;
        let fat = (u64::from(est) * 4 + 8).div_ceil(SECTOR as u64);
        let fat = u32::try_from(fat).map_err(|_| PlanError::TooSmall)?;
        let heap = round_up(EX_BOOT + fat).ok_or(PlanError::Alignment)?;
        let clusters = part.checked_sub(heap).ok_or(PlanError::TooSmall)? / spc;
        let cb = spc * SECTOR as u32;
        let meta = clusters.div_ceil(8).div_ceil(cb) + UPCASE_BYTES.div_ceil(cb) + 1;
        if clusters <= meta {
            return Err(PlanError::TooSmall);
        }
        if !fs.counts().contains(&clusters) {
            return Err(PlanError::ClusterCount);
        }
        l.fat_offset = EX_BOOT;
        l.fats = 1;
        l.fat_sectors = fat;
        l.data_offset = heap;
        l.clusters = clusters;
        return Ok(l);
    }
    let kind = fs.kind();
    let root = if fs == Fs::Fat32 {
        0
    } else {
        ROOT_ENTRIES * 32 / SECTOR as u32
    };
    let min_reserved = if fs == Fs::Fat32 { 32 } else { 1 };
    let est = part
        .checked_sub(min_reserved + root)
        .ok_or(PlanError::TooSmall)?
        / spc;
    if est == 0 {
        return Err(PlanError::TooSmall);
    }
    // Entries 0 to est + 1, rounded up to whole sectors.
    let last = est.min(crate::MAX_CLUSTERS) + 1;
    let fat = (kind.offset(last) + kind.width()).div_ceil(SECTOR as u32);
    // A 16-bit FAT size can't hold it: far more clusters than FAT12/16 allow.
    if fs != Fs::Fat32 && fat > 0xFFFF {
        return Err(PlanError::ClusterCount);
    }
    let sys = min_reserved + 2 * fat + root;
    // The reserved sectors padded so the data area starts on the erase unit.
    let data = round_up(part_start + sys).ok_or(PlanError::Alignment)? - part_start;
    let reserved = data - 2 * fat - root;
    if reserved > 0xFFFF {
        return Err(PlanError::Alignment);
    }
    let clusters = part.checked_sub(data).ok_or(PlanError::TooSmall)? / spc;
    if clusters == 0 {
        return Err(PlanError::TooSmall);
    }
    if !fs.counts().contains(&clusters) {
        return Err(PlanError::ClusterCount);
    }
    l.fat_offset = reserved;
    l.fat_sectors = fat;
    l.root_sectors = root;
    l.data_offset = data;
    l.clusters = clusters;
    Ok(l)
}

impl Layout {
    /// The volume label (as given; FAT stores it upper-case).
    #[must_use]
    pub fn label(&self) -> &[u8] {
        &self.label[..self.label_len]
    }

    /// Bytes per cluster.
    #[must_use]
    pub fn cluster_bytes(&self) -> u32 {
        self.sectors_per_cluster * SECTOR as u32
    }

    /// exFAT: clusters of the bitmap, the up-case table and the root (the
    /// first clusters of the heap, in that order).
    fn ex_meta(&self) -> (u32, u32) {
        let cb = self.cluster_bytes();
        (
            self.clusters.div_ceil(8).div_ceil(cb),
            UPCASE_BYTES.div_ceil(cb),
        )
    }
}

fn put16(b: &mut [u8], o: usize, v: u16) {
    b[o..o + 2].copy_from_slice(&v.to_le_bytes());
}

fn put32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}

fn put64(b: &mut [u8], o: usize, v: u64) {
    b[o..o + 8].copy_from_slice(&v.to_le_bytes());
}

/// A cylinder-head-sector address for the MBR (255 heads, 63 sectors, as
/// every tool since the 1990s), or "use LBA" past cylinder 1023.
fn chs(lba: u32) -> [u8; 3] {
    let c = lba / (255 * 63);
    if c > 1023 {
        return [0xFE, 0xFF, 0xFF];
    }
    let h = (lba / 63) % 255;
    let s = lba % 63 + 1;
    [h as u8, (s as u8 & 0x3F) | ((c >> 2) as u8 & 0xC0), c as u8]
}

/// "Not a boot disk": the jump target of every boot sector here holds
/// `int 18h` (the BIOS's "no bootable disk") and a loop.
const BOOT_CODE: [u8; 4] = [0xCD, 0x18, 0xEB, 0xFE];

/// Writes `count` zero sectors from `lba`, in runs as long as `scratch`.
fn zero<D: BlockDevice>(
    dev: &mut D,
    lba: u32,
    count: u32,
    scratch: &mut [u8],
) -> Result<(), Error> {
    let per = (scratch.len() / SECTOR) as u32;
    scratch[..per as usize * SECTOR].fill(0);
    let mut done = 0;
    while done < count {
        let n = per.min(count - done);
        dev.write_blocks(lba + done, &scratch[..n as usize * SECTOR])?;
        done += n;
    }
    Ok(())
}

/// Formats the card as `l` says (see the module documentation for the
/// order). `scratch`: at least one sector; more writes in longer runs.
///
/// # Errors
/// [`Error::NoRoom`] for a scratch under a sector or a layout that doesn't
/// fit the device; the device's errors (the card is then unformatted: its
/// boot sector cleared, not yet written).
pub fn write<D: BlockDevice>(dev: &mut D, l: &Layout, scratch: &mut [u8]) -> Result<(), Error> {
    let len = scratch.len() / SECTOR * SECTOR;
    if len == 0 || l.part_start.checked_add(l.part_sectors) != Some(l.total) {
        return Err(Error::NoRoom);
    }
    let s = &mut scratch[..len];
    // 1. The old boot sector and the new one's place cleared.
    let mut sector = [0u8; SECTOR];
    dev.read_blocks(0, &mut sector)?;
    if sector[510..512] == [0x55, 0xAA] {
        let old = u32::from_le_bytes([sector[454], sector[455], sector[456], sector[457]]);
        if (1..l.total).contains(&old) && old != l.part_start {
            zero(dev, old, 1, s)?;
        }
    }
    zero(dev, l.part_start, 1, s)?;
    match l.fs {
        Fs::ExFat => write_exfat(dev, l, s)?,
        _ => write_fat(dev, l, s)?,
    }
    // 3. The MBR.
    sector.fill(0);
    put32(&mut sector, 440, l.serial);
    let e = 446;
    sector[e + 1..e + 4].copy_from_slice(&chs(l.part_start));
    sector[e + 4] = l.fs.partition_type(l.part_sectors);
    sector[e + 5..e + 8].copy_from_slice(&chs(l.total - 1));
    put32(&mut sector, e + 8, l.part_start);
    put32(&mut sector, e + 12, l.part_sectors);
    sector[510] = 0x55;
    sector[511] = 0xAA;
    dev.write_blocks(0, &sector)?;
    // 4. The boot sectors, the main one last.
    match l.fs {
        Fs::ExFat => {
            let mut region = [[0u8; SECTOR]; 12];
            ex_boot_region(l, &mut region);
            for base in [l.part_start + 12, l.part_start] {
                for (i, sec) in region.iter().enumerate().rev() {
                    dev.write_blocks(base + i as u32, sec)?;
                }
            }
        }
        _ => {
            let boot = fat_boot_sector(l);
            if l.fs == Fs::Fat32 {
                dev.write_blocks(l.part_start + 6, &boot)?;
            }
            dev.write_blocks(l.part_start, &boot)?;
        }
    }
    Ok(())
}

/// FAT: the FATs and fixed root zeroed, their first entries, the root's
/// label, FSInfo.
fn write_fat<D: BlockDevice>(dev: &mut D, l: &Layout, s: &mut [u8]) -> Result<(), Error> {
    let p = l.part_start;
    let fat_start = p + l.fat_offset;
    let fat32 = l.fs == Fs::Fat32;
    // FAT32's root is cluster 2, the data area's first.
    let root_len = if fat32 {
        l.sectors_per_cluster
    } else {
        l.root_sectors
    };
    zero(dev, fat_start, 2 * l.fat_sectors + l.root_sectors, s)?;
    if fat32 {
        zero(dev, p + l.data_offset, root_len, s)?;
        // The backup boot region's FSInfo and third sector, and the rest of
        // the reserved sectors, cleared of an old file system's.
        zero(dev, p + 1, l.fat_offset - 1, s)?;
    }
    let mut sector = [0u8; SECTOR];
    // Entries 0 and 1 (the media byte, then end of chain); FAT32 also the
    // root's cluster 2, one cluster long.
    let first: &[u8] = match l.fs {
        Fs::Fat12 => &[0xF8, 0xFF, 0xFF],
        Fs::Fat16 => &[0xF8, 0xFF, 0xFF, 0xFF],
        _ => &[
            0xF8, 0xFF, 0xFF, 0x0F, 0xFF, 0xFF, 0xFF, 0x0F, 0xFF, 0xFF, 0xFF, 0x0F,
        ],
    };
    sector[..first.len()].copy_from_slice(first);
    for copy in 0..2 {
        dev.write_blocks(fat_start + copy * l.fat_sectors, &sector)?;
    }
    if l.label_len > 0 {
        sector.fill(0);
        for (d, c) in sector[..11].iter_mut().zip(l.label) {
            *d = c.to_ascii_uppercase();
        }
        sector[11] = 0x08; // volume label
        let root = if fat32 {
            p + l.data_offset
        } else {
            fat_start + 2 * l.fat_sectors
        };
        dev.write_blocks(root, &sector)?;
    }
    if fat32 {
        sector.fill(0);
        put32(&mut sector, 0, 0x4161_5252);
        put32(&mut sector, 484, 0x6141_7272);
        put32(&mut sector, 488, l.clusters - 1);
        put32(&mut sector, 492, 3);
        sector[510] = 0x55;
        sector[511] = 0xAA;
        dev.write_blocks(p + 1, &sector)?;
        dev.write_blocks(p + 7, &sector)?;
    }
    Ok(())
}

/// The FAT12/16/32 boot sector.
fn fat_boot_sector(l: &Layout) -> [u8; SECTOR] {
    let mut b = [0u8; SECTOR];
    let fat32 = l.fs == Fs::Fat32;
    let (jump, code) = if fat32 { (0x58, 90) } else { (0x3C, 62) };
    b[0..3].copy_from_slice(&[0xEB, jump, 0x90]);
    // The OEM name Microsoft's specification recommends for compatibility.
    b[3..11].copy_from_slice(b"MSWIN4.1");
    put16(&mut b, 11, SECTOR as u16);
    b[13] = l.sectors_per_cluster as u8;
    put16(&mut b, 14, l.fat_offset as u16);
    b[16] = 2;
    if !fat32 {
        put16(&mut b, 17, ROOT_ENTRIES as u16);
    }
    if !fat32 && l.part_sectors < 0x1_0000 {
        put16(&mut b, 19, l.part_sectors as u16);
    } else {
        put32(&mut b, 32, l.part_sectors);
    }
    b[21] = 0xF8;
    put16(&mut b, 24, 63);
    put16(&mut b, 26, 255);
    put32(&mut b, 28, l.part_start);
    let ext = if fat32 {
        put32(&mut b, 36, l.fat_sectors);
        put32(&mut b, 44, 2); // the root's cluster
        put16(&mut b, 48, 1); // FSInfo
        put16(&mut b, 50, 6); // the backup boot sector
        64
    } else {
        put16(&mut b, 22, l.fat_sectors as u16);
        36
    };
    b[ext] = 0x80;
    b[ext + 2] = 0x29;
    put32(&mut b, ext + 3, l.serial);
    let mut label = *b"NO NAME    ";
    if l.label_len > 0 {
        for (d, c) in label.iter_mut().zip(l.label) {
            *d = c.to_ascii_uppercase();
        }
    }
    b[ext + 7..ext + 18].copy_from_slice(&label);
    b[ext + 18..ext + 26].copy_from_slice(match l.fs {
        Fs::Fat12 => b"FAT12   ",
        Fs::Fat16 => b"FAT16   ",
        _ => b"FAT32   ",
    });
    b[code..code + BOOT_CODE.len()].copy_from_slice(&BOOT_CODE);
    b[510] = 0x55;
    b[511] = 0xAA;
    b
}

/// exFAT: the FAT, the bitmap, the up-case table and the root directory.
fn write_exfat<D: BlockDevice>(dev: &mut D, l: &Layout, s: &mut [u8]) -> Result<(), Error> {
    let p = l.part_start;
    let spc = l.sectors_per_cluster;
    let (bitmap_n, upcase_n) = l.ex_meta();
    let used = bitmap_n + upcase_n + 1;
    let lba = |c: u32| p + l.data_offset + (c - 2) * spc;
    // The boot regions' other sectors and the gap to the FAT cleared, the
    // FAT, and the clusters the metadata takes.
    zero(dev, p + 1, l.fat_offset - 1 + l.fat_sectors, s)?;
    zero(dev, lba(2), used * spc, s)?;
    // The FAT: entries 0 and 1, then the three chains (the bitmap, the
    // up-case table, the root), each to its end. The used clusters are the
    // heap's first, so only the FAT's first sectors hold anything.
    let chains = [
        (2, bitmap_n),
        (2 + bitmap_n, upcase_n),
        (2 + bitmap_n + upcase_n, 1),
    ];
    let entries = 2 + used;
    let mut sector = [0u8; SECTOR];
    for sec in 0..(entries * 4).div_ceil(SECTOR as u32) {
        sector.fill(0);
        for i in 0..(SECTOR / 4) as u32 {
            let c = sec * (SECTOR / 4) as u32 + i;
            let v = match c {
                0 => 0xFFFF_FFF8,
                1 => 0xFFFF_FFFF,
                _ => match chains.iter().find(|&&(f, n)| (f..f + n).contains(&c)) {
                    Some(&(f, n)) if c + 1 < f + n => c + 1,
                    Some(_) => 0xFFFF_FFFF,
                    None => 0,
                },
            };
            put32(&mut sector, i as usize * 4, v);
        }
        dev.write_blocks(p + l.fat_offset + sec, &sector)?;
    }
    // The bitmap: the metadata's clusters (the first `used`) in use.
    for sec in 0..used.div_ceil(SECTOR as u32 * 8) {
        sector.fill(0);
        for bit in 0..(SECTOR * 8) as u32 {
            let k = sec * (SECTOR * 8) as u32 + bit;
            if k < used {
                sector[(bit / 8) as usize] |= 1 << (bit % 8);
            }
        }
        dev.write_blocks(lba(2) + sec, &sector)?;
    }
    // The up-case table, streamed.
    let mut words = Upcase::new();
    let mut sec = 0;
    loop {
        sector.fill(0);
        let mut n = 0;
        for w in words.by_ref().take(SECTOR / 2) {
            put16(&mut sector, n * 2, w);
            n += 1;
        }
        if n == 0 {
            break;
        }
        dev.write_blocks(lba(2 + bitmap_n) + sec, &sector)?;
        sec += 1;
    }
    // The root: the label, the bitmap and the up-case table's entries.
    sector.fill(0);
    sector[0] = if l.label_len > 0 { 0x83 } else { 0x03 };
    sector[1] = l.label_len as u8;
    for (i, &c) in l.label[..l.label_len].iter().enumerate() {
        put16(&mut sector, 2 + 2 * i, u16::from(c));
    }
    sector[32] = 0x81;
    put32(&mut sector, 32 + 20, 2);
    put64(&mut sector, 32 + 24, u64::from(l.clusters.div_ceil(8)));
    sector[64] = 0x82;
    put32(&mut sector, 64 + 4, UPCASE_CHECKSUM);
    put32(&mut sector, 64 + 20, 2 + bitmap_n);
    put64(&mut sector, 64 + 24, u64::from(UPCASE_BYTES));
    dev.write_blocks(lba(2 + bitmap_n + upcase_n), &sector)?;
    Ok(())
}

/// The exFAT boot region: the boot sector, eight extended boot sectors,
/// the OEM parameters and a reserved sector (both empty), the checksum.
fn ex_boot_region(l: &Layout, r: &mut [[u8; SECTOR]; 12]) {
    let (bitmap_n, upcase_n) = l.ex_meta();
    let b = &mut r[0];
    b[0..3].copy_from_slice(&[0xEB, 0x76, 0x90]);
    b[3..11].copy_from_slice(b"EXFAT   ");
    put64(b, 64, u64::from(l.part_start));
    put64(b, 72, u64::from(l.part_sectors));
    put32(b, 80, l.fat_offset);
    put32(b, 84, l.fat_sectors);
    put32(b, 88, l.data_offset);
    put32(b, 92, l.clusters);
    put32(b, 96, 2 + bitmap_n + upcase_n);
    put32(b, 100, l.serial);
    put16(b, 104, 0x0100); // revision 1.00
    b[108] = 9;
    b[109] = l.sectors_per_cluster.trailing_zeros() as u8;
    b[110] = 1;
    b[111] = 0x80;
    let used = u64::from(bitmap_n + upcase_n + 1);
    b[112] = (used * 100 / u64::from(l.clusters)) as u8;
    b[120..120 + BOOT_CODE.len()].copy_from_slice(&BOOT_CODE);
    b[510] = 0x55;
    b[511] = 0xAA;
    for ext in &mut r[1..9] {
        ext[510] = 0x55;
        ext[511] = 0xAA;
    }
    let sum = r[..11]
        .iter()
        .enumerate()
        .fold(0, |sum, (i, sec)| boot_checksum(sum, i, sec));
    for w in r[11].as_chunks_mut::<4>().0 {
        *w = sum.to_le_bytes();
    }
}

/// The recommended up-case table's size and checksum (exFAT specification
/// section 7.2.5.1).
pub const UPCASE_BYTES: u32 = 5836;
/// See [`UPCASE_BYTES`].
pub const UPCASE_CHECKSUM: u32 = 0xE619_D30D;

/// The recommended table's non-identity mappings: (first character, count,
/// what to add, step): characters `first`, `first + step`, ... map to
/// themselves plus the delta. Built from the specification's Table 25 and
/// checked byte for byte against it by expanding (host test: size and
/// checksum).
const UPCASE: [(u16, u8, i16, u8); 119] = [
    (0x0061, 26, -32, 1),
    (0x00e0, 23, -32, 1),
    (0x00f8, 7, -32, 1),
    (0x00ff, 1, 121, 1),
    (0x0101, 24, -1, 2),
    (0x0133, 3, -1, 2),
    (0x013a, 8, -1, 2),
    (0x014b, 23, -1, 2),
    (0x017a, 3, -1, 2),
    (0x0180, 1, 195, 1),
    (0x0183, 2, -1, 2),
    (0x0188, 1, -1, 1),
    (0x018c, 1, -1, 1),
    (0x0192, 1, -1, 1),
    (0x0195, 1, 97, 1),
    (0x0199, 1, -1, 1),
    (0x019a, 1, 163, 1),
    (0x019e, 1, 130, 1),
    (0x01a1, 3, -1, 2),
    (0x01a8, 1, -1, 1),
    (0x01ad, 1, -1, 1),
    (0x01b0, 1, -1, 1),
    (0x01b4, 2, -1, 2),
    (0x01b9, 1, -1, 1),
    (0x01bd, 1, -1, 1),
    (0x01bf, 1, 56, 1),
    (0x01c6, 1, -2, 1),
    (0x01c9, 1, -2, 1),
    (0x01cc, 1, -2, 1),
    (0x01ce, 8, -1, 2),
    (0x01dd, 1, -79, 1),
    (0x01df, 9, -1, 2),
    (0x01f3, 1, -2, 1),
    (0x01f5, 1, -1, 1),
    (0x01f9, 20, -1, 2),
    (0x0223, 9, -1, 2),
    (0x023a, 1, 10795, 1),
    (0x023c, 1, -1, 1),
    (0x023e, 1, 10792, 1),
    (0x0242, 1, -1, 1),
    (0x0247, 5, -1, 2),
    (0x0253, 1, -210, 1),
    (0x0254, 1, -206, 1),
    (0x0256, 2, -205, 1),
    (0x0259, 1, -202, 1),
    (0x025b, 1, -203, 1),
    (0x0260, 1, -205, 1),
    (0x0263, 1, -207, 1),
    (0x0268, 1, -209, 1),
    (0x0269, 1, -211, 1),
    (0x026b, 1, 10743, 1),
    (0x026f, 1, -211, 1),
    (0x0272, 1, -213, 1),
    (0x0275, 1, -214, 1),
    (0x027d, 1, 10727, 1),
    (0x0280, 1, -218, 1),
    (0x0283, 1, -218, 1),
    (0x0288, 1, -218, 1),
    (0x0289, 1, -69, 1),
    (0x028a, 2, -217, 1),
    (0x028c, 1, -71, 1),
    (0x0292, 1, -219, 1),
    (0x037b, 3, 130, 1),
    (0x03ac, 1, -38, 1),
    (0x03ad, 3, -37, 1),
    (0x03b1, 17, -32, 1),
    (0x03c2, 1, -31, 1),
    (0x03c3, 9, -32, 1),
    (0x03cc, 1, -64, 1),
    (0x03cd, 2, -63, 1),
    (0x03d9, 12, -1, 2),
    (0x03f2, 1, 7, 1),
    (0x03f8, 1, -1, 1),
    (0x03fb, 1, -1, 1),
    (0x0430, 32, -32, 1),
    (0x0450, 16, -80, 1),
    (0x0461, 17, -1, 2),
    (0x048b, 27, -1, 2),
    (0x04c2, 7, -1, 2),
    (0x04cf, 1, -15, 1),
    (0x04d1, 34, -1, 2),
    (0x0561, 38, -48, 1),
    (0x1d7d, 1, 3814, 1),
    (0x1e01, 75, -1, 2),
    (0x1ea1, 45, -1, 2),
    (0x1f00, 8, 8, 1),
    (0x1f10, 6, 8, 1),
    (0x1f20, 8, 8, 1),
    (0x1f30, 8, 8, 1),
    (0x1f40, 6, 8, 1),
    (0x1f51, 4, 8, 2),
    (0x1f60, 8, 8, 1),
    (0x1f70, 2, 74, 1),
    (0x1f72, 4, 86, 1),
    (0x1f76, 2, 100, 1),
    (0x1f78, 2, 128, 1),
    (0x1f7a, 2, 112, 1),
    (0x1f7c, 2, 126, 1),
    (0x1f80, 8, 8, 1),
    (0x1f90, 8, 8, 1),
    (0x1fa0, 8, 8, 1),
    (0x1fb0, 2, 8, 1),
    (0x1fb3, 1, 9, 1),
    (0x1fcc, 1, -9, 1),
    (0x1fd0, 2, 8, 1),
    (0x1fe0, 2, 8, 1),
    (0x1fe5, 1, 7, 1),
    (0x1ffc, 1, -9, 1),
    (0x214e, 1, -28, 1),
    (0x2170, 16, -16, 1),
    (0x2184, 1, -1, 1),
    (0x24d0, 26, -26, 1),
    (0x2c30, 47, -48, 1),
    (0x2c61, 1, -1, 1),
    (0x2c68, 3, -1, 2),
    (0x2c76, 1, -1, 1),
    (0x2c81, 50, -1, 2),
    (0x2d00, 38, -7264, 1),
    (0xff41, 26, -32, 1),
];

/// The identity runs the recommended table stores compressed (`0xFFFF`,
/// then the run's length): (first character, length).
const UPCASE_RUNS: [(u32, u32); 4] = [(1415, 6134), (8581, 843), (9450, 1862), (11558, 53787)];

/// The recommended up-case table, word by word, as stored.
pub(crate) struct Upcase {
    /// The next character to map.
    c: u32,
    /// A run's length, owed after its `0xFFFF`.
    owed: Option<u16>,
}

impl Upcase {
    pub(crate) fn new() -> Self {
        Self { c: 0, owed: None }
    }
}

impl Iterator for Upcase {
    type Item = u16;

    fn next(&mut self) -> Option<u16> {
        if let Some(n) = self.owed.take() {
            return Some(n);
        }
        if self.c > 0xFFFF {
            return None;
        }
        if let Some(&(_, n)) = UPCASE_RUNS.iter().find(|&&(at, _)| at == self.c) {
            self.c += n;
            self.owed = Some(n as u16);
            return Some(0xFFFF);
        }
        let c = self.c;
        self.c += 1;
        let mapped = UPCASE
            .iter()
            .take_while(|&&(first, ..)| u32::from(first) <= c)
            .find(|&&(first, n, _, step)| {
                let (first, step) = (u32::from(first), u32::from(step));
                (c - first).is_multiple_of(step) && (c - first) / step < u32::from(n)
            })
            .map_or(c, |&(_, _, delta, _)| {
                c.wrapping_add_signed(i32::from(delta))
            });
        Some(mapped as u16)
    }
}

#[cfg(test)]
#[path = "mkfs_tests.rs"]
mod tests;
