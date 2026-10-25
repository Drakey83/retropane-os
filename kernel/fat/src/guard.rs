//! A write guard for a volume the PC writes over USB (ADR-0014 point 3,
//! ADR-0036 point 2): everything is allowed except changing what boots the
//! device. The PC writes sectors, not files, so the guard records, for
//! each protected file (by path, in the root or in a folder), the sectors
//! and bytes that make it, plus the card's layout (the partition table and
//! the boot sector's geometry), and refuses a write that would change any
//! of them. Renaming the volume, new files and folders, other files'
//! changes and deletes all pass; deleting, renaming, overwriting or moving
//! a boot file, or formatting the card, is refused (Windows reports the
//! disk as write protected). A protected folder (the CF card's
//! `RetroPane/`, which holds its boot files) keeps its entry: it can't be
//! renamed, moved or deleted, while what it holds changes freely, its
//! protected files apart.
//!
//! - **FAT12, FAT16, FAT32:** a protected file's directory entry (name,
//!   attributes, first cluster, size), its chain in every FAT copy, its
//!   data clusters. FAT12's entries can straddle two sectors: a write
//!   covering half an entry is judged on the half it covers. FAT12/16's
//!   fixed root directory is judged like any directory sector.
//! - **exFAT:** a protected file's entry set (the file entry's type,
//!   count and attributes; the stream entry whole; the name entry whole:
//!   the timestamps and the set checksum may change), its FAT chain when it
//!   has one, its data clusters, and its clusters' bits in the allocation
//!   bitmap (they stay set); the boot region (sector 0's layout bytes
//!   exactly, all but VolumeFlags and PercentInUse, which Windows updates;
//!   the other boot sectors by a 64-bit hash, since only sector 0 sets the
//!   layout), main and backup.
//!
//! Security (docs/security.md): the guard is built from the card (hostile
//! until checked: the chains are walked with the volume's bounds checks and
//! a step limit), and judged per sector, so a write split across commands
//! or mixed with legitimate changes can't slip a protected byte through.

use rp_hal_api::BlockDevice;

use crate::{
    EOC_MIN, Error, FAT_MASK, FatKind, SECTOR, Volume, entry_first, entry_get, le16, le32,
};

/// Files and folders a guard can protect (the CF card's: its boot folder
/// and the four files in it).
pub(crate) const FILES: usize = 5;
/// Consecutive cluster runs per protected file (more: the guard can't be
/// built, and the caller keeps the card read-only).
pub(crate) const EXTENTS: usize = 8;
/// Boot-sector bytes that define a FAT volume's layout: the BPB from bytes
/// per sector to FAT32's backup boot sector's place (FAT12/16: to the
/// total sector count). The label and serial after them may change.
const GEOMETRY32: core::ops::Range<usize> = 11..52;
const GEOMETRY16: core::ops::Range<usize> = 11..36;
/// The MBR's partition table and signature.
const PARTITIONS: core::ops::Range<usize> = 446..512;

/// What the PC may not change on a volume: see the module documentation.
#[derive(Clone)]
pub struct Guard {
    /// The partition's first sector (0: no MBR, a "superfloppy").
    base: u32,
    partitions: [u8; PARTITIONS.end - PARTITIONS.start],
    kind: Kind,
}

// No heap to box the larger variant into: the exFAT guard keeps its
// protected entry sets byte for byte (~1.3 KiB), the FAT one ~0.7 KiB; the
// loader's stack check (`cargo xtask stack`, emu-cf) watches both.
#[allow(clippy::large_enum_variant)]
#[derive(Clone)]
enum Kind {
    Fat(FatGuard),
    Ex(ExGuard),
}

/// A run of clusters a protected file uses: first cluster, length.
type Extents = [(u32, u32); EXTENTS];

/// Adds cluster `c` to `runs` (growing the last run when it follows it);
/// `false` when that would need more than [`EXTENTS`] runs.
pub(crate) fn add_run(runs: &mut Extents, n: &mut usize, c: u32) -> bool {
    match n.checked_sub(1).map(|k| &mut runs[k]) {
        Some((s, len)) if *s + *len == c => *len += 1,
        _ => {
            let Some(slot) = runs.get_mut(*n) else {
                return false;
            };
            *slot = (c, 1);
            *n += 1;
        }
    }
    true
}

fn holds(runs: &[(u32, u32)], cluster: u32) -> bool {
    runs.iter().any(|&(s, n)| (s..s + n).contains(&cluster))
}

/// What the FAT must say for `cluster` (one of the runs'): the next
/// cluster, or [`FAT_MASK`] for the chain's end.
fn expected(runs: &[(u32, u32)], cluster: u32) -> Option<u32> {
    let k = runs
        .iter()
        .position(|&(s, n)| (s..s + n).contains(&cluster))?;
    let (s, n) = runs[k];
    Some(if cluster + 1 < s + n {
        cluster + 1
    } else if let Some(&(next, _)) = runs.get(k + 1) {
        next
    } else {
        FAT_MASK
    })
}

// --- FAT12/16/32 -----------------------------------------------------------

#[derive(Clone, Copy, Default)]
struct Protected {
    entry_lba: u32,
    entry_offset: usize,
    /// Directory entry bytes 0..12: name and attributes.
    name_attr: [u8; 12],
    first_cluster: u32,
    size: u32,
    /// The chain's end mark as stored (FAT12: a write covering half of
    /// it is checked against these bits).
    end_raw: u32,
    extents: Extents,
    runs: usize,
}

#[derive(Clone)]
struct FatGuard {
    fat: FatKind,
    backup_boot: Option<u32>,
    geometry: [u8; GEOMETRY32.end - GEOMETRY32.start],
    geometry_len: usize,
    fat_start: u32,
    fat_sectors: u32,
    fats: u32,
    root_lba: u32,
    data_start: u32,
    sectors_per_cluster: u32,
    files: [Protected; FILES],
    count: usize,
}

impl<D: BlockDevice> Volume<D> {
    /// A guard protecting the files and folders in `names` (paths) that
    /// exist (missing ones are skipped). A folder's entry is kept (its
    /// name, attributes and first cluster), not its contents. Fails, so
    /// the caller keeps the card read-only, when a file is more fragmented
    /// than the guard can record or the FATs aren't mirrored (a PC then
    /// writes only the active one).
    #[inline(never)]
    pub fn guard(&mut self, names: &[&str]) -> Result<Guard, Error> {
        if !self.mirrored && self.fats > 1 {
            return Err(Error::Corrupt);
        }
        let fat32 = self.kind == FatKind::Fat32;
        let geo = if fat32 { GEOMETRY32 } else { GEOMETRY16 };
        let mut g = FatGuard {
            fat: self.kind,
            backup_boot: None,
            geometry: [0; GEOMETRY32.end - GEOMETRY32.start],
            geometry_len: geo.len(),
            fat_start: self.fat_start,
            fat_sectors: self.fat_sectors,
            fats: self.fats,
            root_lba: self.root_lba,
            data_start: self.data_start,
            sectors_per_cluster: self.sectors_per_cluster,
            files: [Protected::default(); FILES],
            count: 0,
        };
        self.dev.read_blocks(self.base, &mut self.buf)?;
        g.geometry[..geo.len()].copy_from_slice(&self.buf[geo]);
        // Only FAT32 has a backup boot sector (FAT16's bytes 50-51 are its
        // label's).
        g.backup_boot = match le16(&self.buf, 50) {
            _ if !fat32 => None,
            0 | 0xFFFF => None,
            n => Some(self.base + u32::from(n)),
        };
        let partitions = self.partition_table()?;
        for name in names.iter().take(FILES) {
            let f = match self.find_file_or_folder(name) {
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
            // A folder's isn't recorded: the PC may add entries to it (its
            // chain grows) and change the unprotected ones.
            let folder = p.name_attr[11] & 0x10 != 0;
            let mut c = Some(f.first_cluster).filter(|&c| c != 0 && !folder);
            let mut steps = 0u32;
            while let Some(cl) = c {
                steps += 1;
                if steps > self.clusters || !add_run(&mut p.extents, &mut p.runs, cl) {
                    return Err(Error::Corrupt);
                }
                c = self.next_cluster(cl)?;
                if c.is_none() {
                    p.end_raw = self.fat_raw(cl)?;
                }
            }
            g.files[g.count] = p;
            g.count += 1;
        }
        Ok(Guard {
            base: self.base,
            partitions,
            kind: Kind::Fat(g),
        })
    }

    /// The MBR's partition table, when the volume has an MBR before it.
    fn partition_table(&mut self) -> Result<[u8; PARTITIONS.end - PARTITIONS.start], Error> {
        let mut t = [0; PARTITIONS.end - PARTITIONS.start];
        if self.base != 0 {
            self.dev.read_blocks(0, &mut self.buf)?;
            t.copy_from_slice(&self.buf[PARTITIONS]);
        }
        Ok(t)
    }

    /// `cluster`'s FAT entry as stored (end marks not widened).
    fn fat_raw(&mut self, cluster: u32) -> Result<u32, Error> {
        let o = self.fat_locate(cluster)?;
        Ok(entry_get(self.kind, &self.fat_buf, o, cluster))
    }

    /// The file `path` names, or else the folder.
    pub(crate) fn find_file_or_folder(&mut self, path: &str) -> Result<crate::File, Error> {
        match self.find(path) {
            Err(Error::NotFound) => {
                let (dir, leaf) = self.parent_of(path)?;
                self.find_entry(dir, leaf, true)
            }
            r => r,
        }
    }
}

impl FatGuard {
    fn allows_sector(&self, lba: u32, s: &[u8]) -> bool {
        if Some(lba) == self.backup_boot {
            return s[GEOMETRY32] == self.geometry[..self.geometry_len];
        }
        let files = &self.files[..self.count];
        // A FAT sector: the protected chains' entries must stay as they are.
        let fat_end = self.fat_start + self.fats * self.fat_sectors;
        if (self.fat_start..fat_end).contains(&lba) {
            // Under 2^28 entries: mount refuses larger FATs.
            let lo = (lba - self.fat_start) % self.fat_sectors * SECTOR as u32;
            let hi = lo + SECTOR as u32;
            // The entries touching these bytes (FAT12: one may start in the
            // sector before).
            let c_lo = match self.fat {
                FatKind::Fat12 => self.fat.first_at(lo).saturating_sub(1),
                _ => self.fat.first_at(lo),
            };
            let c_hi = self.fat.first_at(hi);
            return files.iter().all(|f| {
                let runs = &f.extents[..f.runs];
                runs.iter().all(|&(start, n)| {
                    (start.max(c_lo)..(start + n).min(c_hi)).all(|c| {
                        expected(runs, c)
                            .is_none_or(|want| self.entry_ok(s, lo, c, want, f.end_raw))
                    })
                })
            });
        }
        if lba < self.data_start {
            // FAT12/16's fixed root: a directory sector, judged below.
            // Anything else here is a reserved sector (FSInfo and the like).
            if lba < self.root_lba {
                return true;
            }
        } else {
            let cluster = (lba - self.data_start) / self.sectors_per_cluster + 2;
            if files.iter().any(|f| holds(&f.extents[..f.runs], cluster)) {
                return false;
            }
        }
        // A directory sector: the protected entries' identity must stay.
        files.iter().filter(|f| f.entry_lba == lba).all(|f| {
            let e = &s[f.entry_offset..f.entry_offset + 32];
            e[..12] == f.name_attr
                && entry_first(e, self.fat) == f.first_cluster
                && le32(e, 28) == f.size
        })
    }

    /// Whether `cluster`'s entry, as far as sector `s` (FAT bytes from `lo`)
    /// holds it, says `want` (a cluster, or [`FAT_MASK`]: any end mark;
    /// `end_raw` is the mark as stored, for half an entry).
    fn entry_ok(&self, s: &[u8], lo: u32, cluster: u32, want: u32, end_raw: u32) -> bool {
        let at = i64::from(self.fat.offset(cluster)) - i64::from(lo);
        let byte = |i: i64| usize::try_from(i).ok().and_then(|i| s.get(i).copied());
        let (b0, b1) = (byte(at), byte(at + 1));
        let end = want == FAT_MASK;
        match self.fat {
            FatKind::Fat32 => {
                let v = le32(s, at as usize) & FAT_MASK;
                if end { v >= EOC_MIN } else { v == want }
            }
            FatKind::Fat16 => {
                let v = u32::from(le16(s, at as usize));
                if end { v >= 0xFFF8 } else { v == want }
            }
            FatKind::Fat12 => {
                let w = u32::from(b0.unwrap_or(0)) | u32::from(b1.unwrap_or(0)) << 8;
                let m = if b0.is_some() { 0xFF } else { 0 } | if b1.is_some() { 0xFF00 } else { 0 };
                let (v, vm) = if cluster & 1 == 0 {
                    (w & 0xFFF, m & 0xFFF)
                } else {
                    (w >> 4, m >> 4)
                };
                if vm == 0xFFF {
                    if end { v >= 0xFF8 } else { v == want }
                } else {
                    let raw = if end { end_raw } else { want };
                    v & vm == raw & vm
                }
            }
        }
    }
}

// --- exFAT -----------------------------------------------------------------

/// The longest entry set a protected exFAT file may have: the file entry,
/// the stream entry, one name entry (names up to 15 characters; every boot
/// file's is). A longer one: no guard, the card read-only.
pub(crate) const EX_SET: usize = 3;

#[derive(Clone, Copy, Default)]
pub(crate) struct ExProtected {
    /// Where each entry of the set is: sector, byte offset.
    pub(crate) at: [(u32, u16); EX_SET],
    /// The set as on the card.
    pub(crate) set: [[u8; 32]; EX_SET],
    pub(crate) entries: usize,
    pub(crate) folder: bool,
    /// Chained through the FAT (not "no FAT chain").
    pub(crate) chained: bool,
    pub(crate) extents: Extents,
    pub(crate) runs: usize,
}

impl ExProtected {
    /// Whether entry `k` of the set, as `e`, keeps what may not change.
    fn entry_ok(&self, k: usize, e: &[u8]) -> bool {
        let was = &self.set[k];
        match k {
            // The file entry: type (in use), secondary count, attributes.
            0 => e[0] == was[0] && e[1] == was[1] && e[4..6] == was[4..6],
            // A folder's stream: its name's length and hash, its first
            // cluster (it may grow, or be chained once it does).
            1 if self.folder => e[0] == was[0] && e[3..6] == was[3..6] && e[20..24] == was[20..24],
            // A file's stream, the name: whole.
            _ => e[..32] == was[..],
        }
    }
}

/// A 64-bit FNV-1a hash of a boot-region sector (sector 0's own layout is
/// compared byte for byte; these only guard sectors the layout doesn't
/// come from).
pub(crate) fn sector_hash(s: &[u8], skip: &[usize]) -> u64 {
    s.iter()
        .enumerate()
        .fold(0xCBF2_9CE4_8422_2325, |h, (i, &b)| {
            let b = if skip.contains(&i) { 0 } else { b };
            (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01B3)
        })
}

/// exFAT boot sector bytes Windows changes (VolumeFlags, PercentInUse).
pub(crate) const EX_VOLATILE: [usize; 3] = [106, 107, 112];

#[derive(Clone)]
pub(crate) struct ExGuard {
    /// Sector 0's bytes 0..120 (the volatile ones not compared).
    pub(crate) boot: [u8; 120],
    /// Hashes of boot-region sectors 1..12 and 12..24 (12: the backup boot
    /// sector, volatile bytes skipped).
    pub(crate) region: [u64; 23],
    pub(crate) fat_start: u32,
    pub(crate) fat_sectors: u32,
    pub(crate) heap_start: u32,
    pub(crate) cluster_shift: u32,
    /// The allocation bitmap's first sector and bytes.
    pub(crate) bitmap: (u32, u32),
    pub(crate) files: [ExProtected; FILES],
    pub(crate) count: usize,
}

impl Guard {
    /// The exFAT guard, as `exfat` builds it.
    pub(crate) fn exfat(base: u32, partitions: [u8; 66], g: ExGuard) -> Self {
        Self {
            base,
            partitions,
            kind: Kind::Ex(g),
        }
    }
}

impl ExGuard {
    fn allows_sector(&self, base: u32, lba: u32, s: &[u8]) -> bool {
        // The boot region, main and backup.
        if (base..base + 24).contains(&lba) {
            let k = (lba - base) as usize;
            return match k {
                0 => (0..120)
                    .filter(|i| !EX_VOLATILE.contains(i))
                    .all(|i| s[i] == self.boot[i]),
                12 => sector_hash(s, &EX_VOLATILE) == self.region[11],
                _ => sector_hash(s, &[]) == self.region[k - 1],
            };
        }
        let files = &self.files[..self.count];
        // The FAT: a chained file's links stay.
        if (self.fat_start..self.fat_start + self.fat_sectors).contains(&lba) {
            let first = (lba - self.fat_start) * (SECTOR as u32 / 4);
            return files.iter().filter(|f| f.chained).all(|f| {
                let runs = &f.extents[..f.runs];
                (0..SECTOR as u32 / 4).all(|i| match expected(runs, first + i) {
                    None => true,
                    Some(FAT_MASK) => le32(s, i as usize * 4) == 0xFFFF_FFFF,
                    Some(next) => le32(s, i as usize * 4) == next,
                })
            });
        }
        // The bitmap: a protected cluster's bit stays set.
        let (bm, bytes) = self.bitmap;
        if (bm..bm + bytes.div_ceil(SECTOR as u32)).contains(&lba) {
            let first = (lba - bm) * SECTOR as u32 * 8 + 2;
            return files.iter().all(|f| {
                f.extents[..f.runs].iter().all(|&(start, n)| {
                    (start.max(first)..(start + n).min(first + SECTOR as u32 * 8)).all(|c| {
                        let bit = c - first;
                        s[(bit / 8) as usize] & 1 << (bit % 8) != 0
                    })
                })
            });
        }
        if lba >= self.heap_start {
            let cluster = ((lba - self.heap_start) >> self.cluster_shift) + 2;
            if files.iter().any(|f| holds(&f.extents[..f.runs], cluster)) {
                return false;
            }
        }
        // A directory sector: the protected sets' identity.
        files.iter().all(|f| {
            (0..f.entries).all(|k| {
                let (at, o) = f.at[k];
                at != lba || f.entry_ok(k, &s[usize::from(o)..usize::from(o) + 32])
            })
        })
    }
}

impl Guard {
    /// Files and folders protected.
    #[must_use]
    pub fn files(&self) -> usize {
        match &self.kind {
            Kind::Fat(g) => g.count,
            Kind::Ex(g) => g.count,
        }
    }

    /// Whether the PC may write `data` (whole sectors) from `lba` on.
    #[must_use]
    pub fn allows(&self, lba: u32, data: &[u8]) -> bool {
        // `lba` comes from the USB host: a run past the last block number
        // is refused, never wrapped.
        data.chunks(SECTOR).zip(0u32..).all(|(s, i)| {
            s.len() == SECTOR
                && lba
                    .checked_add(i)
                    .is_some_and(|at| self.allows_sector(at, s))
        })
    }

    fn allows_sector(&self, lba: u32, s: &[u8]) -> bool {
        // The MBR's partition table, and nothing else before the partition
        // (the gap holds no file system, so nothing a PC needs to write).
        if lba < self.base {
            return lba == 0 && s[PARTITIONS] == self.partitions;
        }
        match &self.kind {
            Kind::Fat(g) => {
                if lba == self.base {
                    let geo = if g.fat == FatKind::Fat32 {
                        GEOMETRY32
                    } else {
                        GEOMETRY16
                    };
                    return s[geo] == g.geometry[..g.geometry_len];
                }
                g.allows_sector(lba, s)
            }
            Kind::Ex(g) => g.allows_sector(self.base, lba, s),
        }
    }
}

#[cfg(test)]
#[path = "guard_tests.rs"]
mod tests;
