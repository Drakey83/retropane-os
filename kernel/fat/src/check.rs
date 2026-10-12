//! A read-only file system check, as a modern fsck: every folder walked from
//! the root, every cluster a file or folder uses marked once (in a bitmap
//! the caller lends), then the allocation (FAT32's FAT, exFAT's bitmap)
//! compared with what was reached. Problems are counted and reported, never
//! "fixed" silently: repairs are separate, explicit tools.

use rp_hal_api::BlockDevice;

use crate::{ATTR_DIRECTORY, Error, FAT_MASK, FAT_PER_SECTOR, SECTOR, Volume, le16, le32};

/// What a check found.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Health {
    /// Files and folders reached.
    pub files: u32,
    /// Folders reached (the root not counted).
    pub folders: u32,
    /// Clusters files and folders use.
    pub used: u32,
    /// Clusters reached twice: two files sharing one, or a chain looping.
    pub cross_linked: u32,
    /// Chains with links outside the volume, or folders that can't be read.
    pub bad_chains: u32,
    /// Files whose chain is longer or shorter than their size needs.
    pub size_mismatch: u32,
    /// Clusters allocated but reached from nothing (space leaked, harmless).
    pub lost: u32,
    /// exFAT: clusters a file uses but the bitmap calls free (dangerous: the
    /// next write could take them).
    pub free_but_used: u32,
    /// exFAT: entry sets whose checksum fails (those files are invisible).
    pub bad_sets: u32,
    /// FAT32: FAT sectors where the mirrored copies differ.
    pub fat_copies_differ: u32,
    /// Folders not walked (nested deeper than the check follows).
    pub unchecked: u32,
}

impl Health {
    /// Nothing that loses or risks data (lost clusters only waste space).
    #[must_use]
    pub fn clean(&self) -> bool {
        self.cross_linked == 0
            && self.bad_chains == 0
            && self.size_mismatch == 0
            && self.free_but_used == 0
            && self.bad_sets == 0
            && self.fat_copies_differ == 0
            && self.unchecked == 0
    }
}

/// Folders waiting to be walked: the deepest nesting followed.
pub(crate) const PENDING: usize = 128;
/// Entries taken from a folder per pass (no buffer grows with a folder).
pub(crate) const BATCH: usize = 16;

/// Which clusters have been reached.
pub(crate) struct Seen<'a>(pub(crate) &'a mut [u8]);

impl Seen<'_> {
    /// Marks `c`; `false` if it was already marked.
    pub(crate) fn mark(&mut self, c: u32) -> bool {
        let (i, m) = ((c / 8) as usize, 1u8 << (c % 8));
        let fresh = self.0[i] & m == 0;
        self.0[i] |= m;
        fresh
    }

    pub(crate) fn get(&self, c: u32) -> bool {
        self.0[(c / 8) as usize] & (1 << (c % 8)) != 0
    }
}

impl<D: BlockDevice> Volume<D> {
    /// Checks the volume, read-only. `seen` needs a bit per cluster
    /// (clusters / 8 bytes, [`Error::NoRoom`] otherwise); `scratch` at least
    /// two sectors (more reads the FAT in fewer commands).
    #[inline(never)]
    pub fn check(&mut self, seen: &mut [u8], scratch: &mut [u8]) -> Result<Health, Error> {
        let total = self.clusters + 2;
        if seen.len() < (total as usize).div_ceil(8) || scratch.len() < 2 * SECTOR {
            return Err(Error::NoRoom);
        }
        seen[..(total as usize).div_ceil(8)].fill(0);
        let mut seen = Seen(seen);
        let mut h = Health::default();
        let mut pending = [0u32; PENDING];
        let mut np = 0;
        if self.chain(self.root_cluster, None, &mut seen, &mut h)? {
            pending[0] = self.root_cluster;
            np = 1;
        }
        while np > 0 {
            np -= 1;
            let dir = pending[np];
            let mut skip = 0usize;
            loop {
                let mut batch = [(0u32, 0u32, false); BATCH];
                let (mut nb, mut idx, mut more) = (0, 0usize, false);
                let walked = self.walk_dir(dir, &mut |_, _, e, _| {
                    // Volume labels, `.` and `..` aren't files.
                    if e[11] & 0x08 != 0 || e[0] == b'.' {
                        return false;
                    }
                    if idx < skip {
                        idx += 1;
                        return false;
                    }
                    if nb == BATCH {
                        more = true;
                        return true;
                    }
                    let first = u32::from(le16(e, 20)) << 16 | u32::from(le16(e, 26));
                    batch[nb] = (first, le32(e, 28), e[11] & ATTR_DIRECTORY != 0);
                    nb += 1;
                    idx += 1;
                    false
                });
                match walked {
                    Err(Error::Corrupt) => {
                        h.bad_chains += 1;
                        break;
                    }
                    r => r?,
                };
                for &(first, size, is_dir) in &batch[..nb] {
                    if is_dir {
                        h.folders += 1;
                        if self.chain(first, None, &mut seen, &mut h)? {
                            if np < PENDING {
                                pending[np] = first;
                                np += 1;
                            } else {
                                h.unchecked += 1;
                            }
                        }
                    } else {
                        h.files += 1;
                        if first == 0 {
                            h.size_mismatch += u32::from(size != 0);
                        } else {
                            self.chain(first, Some(size), &mut seen, &mut h)?;
                        }
                    }
                }
                if !more {
                    break;
                }
                skip += nb;
            }
        }
        self.scan_fat(&seen, scratch, &mut h)?;
        Ok(h)
    }

    /// Follows and marks a chain; checks its length against `size` (files).
    /// `true` if it was whole and reached for the first time.
    fn chain(
        &mut self,
        first: u32,
        size: Option<u32>,
        seen: &mut Seen<'_>,
        h: &mut Health,
    ) -> Result<bool, Error> {
        if !self.valid_cluster(first) {
            h.bad_chains += 1;
            return Ok(false);
        }
        let mut c = first;
        let mut n = 0u32;
        loop {
            if !seen.mark(c) {
                h.cross_linked += 1;
                return Ok(false);
            }
            n += 1;
            h.used += 1;
            match self.next_cluster(c) {
                Ok(Some(next)) => c = next,
                Ok(None) => break,
                Err(Error::Corrupt) => {
                    h.bad_chains += 1;
                    return Ok(false);
                }
                Err(e) => return Err(e),
            }
        }
        if let Some(size) = size
            && u64::from(size).div_ceil(self.cluster_bytes() as u64) != u64::from(n)
        {
            h.size_mismatch += 1;
        }
        Ok(true)
    }

    /// Counts allocated clusters nothing reached, and, when the FATs are
    /// mirrored, the sectors where the copies differ.
    fn scan_fat(
        &mut self,
        seen: &Seen<'_>,
        scratch: &mut [u8],
        h: &mut Health,
    ) -> Result<(), Error> {
        let half = scratch.len() / 2 / SECTOR * SECTOR;
        let (one, two) = scratch.split_at_mut(half);
        let per = (half / SECTOR) as u32;
        let compare = self.mirrored && self.fats >= 2;
        let total = self.clusters + 2;
        let mut s = 0;
        while s < self.fat_sectors && s * FAT_PER_SECTOR < total {
            let n = per.min(self.fat_sectors - s);
            let len = n as usize * SECTOR;
            self.dev.read_blocks(self.fat_start + s, &mut one[..len])?;
            if compare {
                self.dev
                    .read_blocks(self.fat_start + self.fat_sectors + s, &mut two[..len])?;
                h.fat_copies_differ += one[..len]
                    .as_chunks::<SECTOR>()
                    .0
                    .iter()
                    .zip(two[..len].as_chunks::<SECTOR>().0)
                    .filter(|(a, b)| a != b)
                    .count() as u32;
            }
            for (i, e) in one[..len].as_chunks::<4>().0.iter().enumerate() {
                let c = s * FAT_PER_SECTOR + i as u32;
                if !(2..total).contains(&c) {
                    continue;
                }
                let v = u32::from_le_bytes(*e) & FAT_MASK;
                // Free, or marked bad by a formatter: not lost.
                if v != 0 && v != 0x0FFF_FFF7 && !seen.get(c) {
                    h.lost += 1;
                }
            }
            s += n;
        }
        Ok(())
    }
}
