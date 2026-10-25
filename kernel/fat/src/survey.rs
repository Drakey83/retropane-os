//! A card's layout, read without mounting it (ADR-0036): the partition
//! table's first entry and the boot sector's geometry, as another
//! formatter (the card's maker, Windows, Windows Mobile) left them, to
//! compare with what [`crate::mkfs`] writes. Read-only; every number comes
//! from the card and is only reported, never used to address it beyond the
//! two sectors read.

use rp_hal_api::BlockDevice;

use crate::mkfs::Fs;
use crate::{Error, FatKind, SECTOR, le16, le32};

/// What a card's first sectors say.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Survey {
    /// The partition table's first entry: type, start, sectors (all 0: no
    /// MBR, the volume starts at sector 0).
    pub part_type: u8,
    /// The partition's first sector.
    pub part_start: u32,
    /// Sectors in the partition.
    pub part_sectors: u32,
    /// The file system, when the boot sector names one this crate knows.
    pub fs: Option<Fs>,
    /// The formatter's name in the boot sector (FAT's OEM name; exFAT's is
    /// always "EXFAT").
    pub oem: [u8; 8],
    /// Bytes per cluster.
    pub cluster_bytes: u32,
    /// From the partition's start to the first FAT.
    pub fat_offset: u32,
    /// FAT copies and sectors in each.
    pub fats: u32,
    /// Sectors per FAT.
    pub fat_sectors: u32,
    /// FAT12/16's root directory entries.
    pub root_entries: u32,
    /// From the partition's start to the data area (exFAT: the heap).
    pub data_offset: u32,
    /// Data clusters.
    pub clusters: u32,
    /// The boot sector's label (FAT; spaces for exFAT, whose label is in
    /// its root directory).
    pub label: [u8; 11],
}

impl Survey {
    /// The largest power of two, in sectors, dividing where the data area
    /// starts on the card (capped at 64 MiB): how its clusters line up
    /// with the card's erase blocks.
    #[must_use]
    pub fn data_alignment(&self) -> u32 {
        let at = self.part_start.wrapping_add(self.data_offset);
        if at == 0 {
            return 1 << 17;
        }
        (1u32 << at.trailing_zeros()).min(1 << 17)
    }
}

/// Reads sector 0 and the boot sector it points to.
///
/// # Errors
/// The device's; [`Error::NotFat32`] when neither is a FAT or exFAT boot
/// sector (the partition table is still filled in).
pub fn survey<D: BlockDevice>(dev: &mut D) -> Result<Survey, Error> {
    let mut b = [0u8; SECTOR];
    dev.read_blocks(0, &mut b)?;
    let mut s = Survey::default();
    let is_boot = |b: &[u8]| {
        b[510..512] == [0x55, 0xAA] && (b[0] == 0xEB || b[0] == 0xE9) && le16(b, 11) == 512
    };
    // An MBR, unless sector 0 is itself a boot sector (a "superfloppy").
    if b[510..512] == [0x55, 0xAA] && !is_boot(&b) && &b[3..11] != b"EXFAT   " {
        s.part_type = b[446 + 4];
        s.part_start = le32(&b, 446 + 8);
        s.part_sectors = le32(&b, 446 + 12);
        dev.read_blocks(s.part_start, &mut b)?;
    }
    s.oem.copy_from_slice(&b[3..11]);
    if &b[3..11] == b"EXFAT   " {
        s.fs = Some(Fs::ExFat);
        s.cluster_bytes = (SECTOR as u32).checked_shl(u32::from(b[109])).unwrap_or(0);
        s.fat_offset = le32(&b, 80);
        s.fat_sectors = le32(&b, 84);
        s.fats = u32::from(b[110]);
        s.data_offset = le32(&b, 88);
        s.clusters = le32(&b, 92);
        s.label = [b' '; 11];
        return Ok(s);
    }
    if !is_boot(&b) {
        return Err(Error::NotFat32);
    }
    let spc = u32::from(b[13]);
    s.cluster_bytes = spc * SECTOR as u32;
    s.fat_offset = u32::from(le16(&b, 14));
    s.fats = u32::from(b[16]);
    s.root_entries = u32::from(le16(&b, 17));
    let fat16 = u32::from(le16(&b, 22));
    let fat32 = s.root_entries == 0 && fat16 == 0;
    s.fat_sectors = if fat32 { le32(&b, 36) } else { fat16 };
    let total = match le16(&b, 19) {
        0 => le32(&b, 32),
        n => u32::from(n),
    };
    let root = (s.root_entries * 32).div_ceil(SECTOR as u32);
    s.data_offset = s
        .fat_offset
        .saturating_add(s.fats.saturating_mul(s.fat_sectors))
        .saturating_add(root);
    s.clusters = total
        .saturating_sub(s.data_offset)
        .checked_div(spc)
        .unwrap_or(0);
    let kind = if fat32 {
        FatKind::Fat32
    } else if s.clusters < crate::FAT12_LIMIT {
        FatKind::Fat12
    } else {
        FatKind::Fat16
    };
    s.fs = Some(match kind {
        FatKind::Fat12 => Fs::Fat12,
        FatKind::Fat16 => Fs::Fat16,
        FatKind::Fat32 => Fs::Fat32,
    });
    let at = if fat32 { 71 } else { 43 };
    s.label.copy_from_slice(&b[at..at + 11]);
    Ok(s)
}
