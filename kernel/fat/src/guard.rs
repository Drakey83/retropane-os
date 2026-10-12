//! A write guard for a FAT32 volume the PC writes over USB: everything is
//! allowed except changing what boots the device. The PC writes sectors,
//! not files, so the guard records, for each protected root file, the
//! sectors and bytes that make it (its directory entry's name, attributes,
//! first cluster and size; its cluster chain in every FAT copy; its data
//! clusters), plus the card's layout (the partition table and the boot
//! sector's geometry), and refuses a write that would change any of them.
//! Renaming the volume, new files and folders, other files' changes and
//! deletes all pass; deleting, renaming, overwriting or moving a boot file,
//! or formatting the card, is refused (Windows reports the disk as write
//! protected).
//!
//! Security (docs/security.md): the guard is built from the card (hostile
//! until checked: the chains are walked with the volume's bounds checks and
//! a step limit), and judged per sector, so a write split across commands
//! or mixed with legitimate changes can't slip a protected byte through.

use rp_hal_api::BlockDevice;

use crate::{EOC_MIN, Error, FAT_MASK, FAT_PER_SECTOR, SECTOR, Volume, le16, le32};

/// Root files a guard can protect.
const FILES: usize = 4;
/// Consecutive cluster runs per protected file (more: the guard can't be
/// built, and the caller keeps the card read-only).
const EXTENTS: usize = 8;
/// Boot-sector bytes that define the volume's layout (BPB from bytes per
/// sector to the backup boot sector's place). The label and serial after
/// them may change.
const GEOMETRY: core::ops::Range<usize> = 11..52;
/// The MBR's partition table and signature.
const PARTITIONS: core::ops::Range<usize> = 446..512;

#[derive(Clone, Copy, Default)]
struct Protected {
    entry_lba: u32,
    entry_offset: usize,
    /// Directory entry bytes 0..12: name and attributes.
    name_attr: [u8; 12],
    first_cluster: u32,
    size: u32,
    /// Cluster runs: first cluster, length.
    extents: [(u32, u32); EXTENTS],
    runs: usize,
}

impl Protected {
    fn holds(&self, cluster: u32) -> bool {
        self.extents[..self.runs]
            .iter()
            .any(|&(s, n)| (s..s + n).contains(&cluster))
    }

    /// What the FAT must say for `cluster` (one of this file's).
    fn expected(&self, cluster: u32) -> Option<u32> {
        let runs = &self.extents[..self.runs];
        let k = runs
            .iter()
            .position(|&(s, n)| (s..s + n).contains(&cluster))?;
        let (s, n) = runs[k];
        Some(if cluster + 1 < s + n {
            cluster + 1
        } else if let Some(&(next, _)) = runs.get(k + 1) {
            next
        } else {
            FAT_MASK // any end-of-chain mark (checked as >= EOC_MIN)
        })
    }
}

/// What the PC may not change on a volume: see the module documentation.
#[derive(Clone)]
pub struct Guard {
    /// The partition's first sector (0: no MBR, a "superfloppy").
    base: u32,
    backup_boot: Option<u32>,
    geometry: [u8; GEOMETRY.end - GEOMETRY.start],
    partitions: [u8; PARTITIONS.end - PARTITIONS.start],
    fat_start: u32,
    fat_sectors: u32,
    fats: u32,
    data_start: u32,
    sectors_per_cluster: u32,
    files: [Protected; FILES],
    count: usize,
}

impl<D: BlockDevice> Volume<D> {
    /// A guard protecting the root files in `names` that exist (missing ones
    /// are skipped). Fails, so the caller keeps the card read-only, when a
    /// file is more fragmented than the guard can record or the FATs aren't
    /// mirrored (a PC then writes only the active one).
    #[inline(never)]
    pub fn guard(&mut self, names: &[&str]) -> Result<Guard, Error> {
        if !self.mirrored && self.fats > 1 {
            return Err(Error::Corrupt);
        }
        let mut g = Guard {
            base: self.base,
            backup_boot: None,
            geometry: [0; GEOMETRY.end - GEOMETRY.start],
            partitions: [0; PARTITIONS.end - PARTITIONS.start],
            fat_start: self.fat_start,
            fat_sectors: self.fat_sectors,
            fats: self.fats,
            data_start: self.data_start,
            sectors_per_cluster: self.sectors_per_cluster,
            files: [Protected::default(); FILES],
            count: 0,
        };
        self.dev.read_blocks(self.base, &mut self.buf)?;
        g.geometry.copy_from_slice(&self.buf[GEOMETRY]);
        g.backup_boot = match le16(&self.buf, 50) {
            0 | 0xFFFF => None,
            n => Some(self.base + u32::from(n)),
        };
        if self.base != 0 {
            self.dev.read_blocks(0, &mut self.buf)?;
            g.partitions.copy_from_slice(&self.buf[PARTITIONS]);
        }
        for name in names.iter().take(FILES) {
            let f = match self.find(name) {
                Ok(f) => f,
                Err(Error::NotFound) => continue,
                Err(e) => return Err(e),
            };
            let mut p = Protected {
                entry_lba: f.entry_lba,
                entry_offset: usize::from(f.entry_index) * 32,
                first_cluster: f.first_cluster,
                size: f.size,
                ..Protected::default()
            };
            self.dev.read_blocks(f.entry_lba, &mut self.buf)?;
            p.name_attr
                .copy_from_slice(&self.buf[p.entry_offset..p.entry_offset + 12]);
            // The chain, as runs; every step bounded by the cluster count.
            let mut c = Some(f.first_cluster).filter(|&c| c != 0);
            let mut steps = 0u32;
            while let Some(cl) = c {
                steps += 1;
                if steps > self.clusters {
                    return Err(Error::Corrupt);
                }
                match p.runs.checked_sub(1).map(|k| &mut p.extents[k]) {
                    Some((s, n)) if *s + *n == cl => *n += 1,
                    _ => {
                        let slot = p.extents.get_mut(p.runs).ok_or(Error::Corrupt)?;
                        *slot = (cl, 1);
                        p.runs += 1;
                    }
                }
                c = self.next_cluster(cl)?;
            }
            g.files[g.count] = p;
            g.count += 1;
        }
        Ok(g)
    }
}

impl Guard {
    /// Root files protected.
    #[must_use]
    pub fn files(&self) -> usize {
        self.count
    }

    /// Whether the PC may write `data` (whole sectors) from `lba` on.
    #[must_use]
    pub fn allows(&self, lba: u32, data: &[u8]) -> bool {
        data.chunks(SECTOR)
            .zip(lba..)
            .all(|(s, at)| s.len() == SECTOR && self.allows_sector(at, s))
    }

    fn allows_sector(&self, lba: u32, s: &[u8]) -> bool {
        // The MBR's partition table, and nothing else before the partition
        // (the gap holds no file system, so nothing a PC needs to write).
        if lba < self.base {
            return lba == 0 && s[PARTITIONS] == self.partitions;
        }
        if lba == self.base || Some(lba) == self.backup_boot {
            return s[GEOMETRY] == self.geometry;
        }
        let files = &self.files[..self.count];
        // A FAT sector: the protected chains' entries must stay as they are.
        let fat_end = self.fat_start + self.fats * self.fat_sectors;
        if (self.fat_start..fat_end).contains(&lba) {
            let first = (lba - self.fat_start) % self.fat_sectors * FAT_PER_SECTOR;
            return (0..FAT_PER_SECTOR).all(|i| {
                let cluster = first + i;
                let v = le32(s, i as usize * 4) & FAT_MASK;
                files.iter().all(|f| match f.expected(cluster) {
                    None => true,
                    Some(FAT_MASK) => v >= EOC_MIN,
                    Some(next) => v == next,
                })
            });
        }
        if lba < self.data_start {
            // Reserved sectors after the boot sector (FSInfo and the like).
            return true;
        }
        let cluster = (lba - self.data_start) / self.sectors_per_cluster + 2;
        if files.iter().any(|f| f.holds(cluster)) {
            return false;
        }
        // A directory sector: the protected entries' identity must stay.
        files.iter().filter(|f| f.entry_lba == lba).all(|f| {
            let e = &s[f.entry_offset..f.entry_offset + 32];
            e[..12] == f.name_attr
                && (u32::from(le16(e, 20)) << 16 | u32::from(le16(e, 26))) == f.first_cluster
                && le32(e, 28) == f.size
        })
    }
}

#[cfg(test)]
#[path = "guard_tests.rs"]
mod tests;
