//! exFAT reading (Microsoft's published exFAT specification, 2019): what
//! SDXC cards (64 GB and up) come formatted with, and what stock Windows
//! Mobile never read.
//!
//! Mount checks the boot region's checksum and every field it uses; the
//! root directory's entry sets are checked against their set checksums;
//! files are read contiguously ("no FAT chain") or through the FAT, with
//! bytes past the valid data length read as zeros.
//!
//! Folders: paths with `/` or `\` (as on FAT32); a folder is stored, as
//! Windows stores it, in one run of clusters with no FAT chain, or chained
//! (the root always is). [`ExVolume::mkdir`] zeroes the new folder's
//! cluster and marks it used before the entry that makes it exist.
//!
//! Writing ([`ExVolume::put`]): the allocation bitmap, not
//! a FAT scan, finds free space, and a file that fits in one free run is
//! stored with no FAT chain at all (exFAT's contiguous files); only when the
//! card is too fragmented are clusters chained through the FAT. As on
//! FAT32 (`Volume::replace`), the order makes a power cut harmless: data
//! and bitmap first, then one sector write switches the directory (a
//! replaced file's file and stream entries share a sector; a new file's set
//! fits one sector or is written last-sector-first behind the end of the
//! directory), then the old clusters are freed. The boot sector's
//! VolumeDirty flag is set for the duration, so a cut is checked by the
//! next PC to see the card. No timestamps yet (no clock).
//!
//! Security (docs/security.md): every length, offset and cluster number
//! from the card is checked before use, chains and directories are bounded,
//! and a malformed card gives an error, never a panic or a loop.

use rp_hal_api::BlockDevice;

use crate::{Error, NAME_MAX, SECTOR, Written};

const ENTRY: usize = 32;
const FAT_EOC: u32 = 0xFFFF_FFFF;
const FAT_BAD: u32 = 0xFFFF_FFF7;
/// Entry types (in use: bit 7 set).
const T_FILE: u8 = 0x85;
const T_STREAM: u8 = 0xC0;
const T_NAME: u8 = 0xC1;
const T_BITMAP: u8 = 0x81;
const ATTR_DIRECTORY: u16 = 0x10;
const ATTR_ARCHIVE: u16 = 0x20;
/// Stream extension flags.
const ALLOC_POSSIBLE: u8 = 0x01;
const NO_FAT_CHAIN: u8 = 0x02;
/// Boot sector VolumeFlags (byte 106): the volume may be inconsistent.
const VOLUME_DIRTY: u8 = 0x02;
/// Directory entries per sector.
const SLOTS: usize = SECTOR / ENTRY;
/// A file entry set: the file entry, the stream extension, up to 17 name
/// entries (255 characters).
const MAX_SECONDARY: usize = 18;
/// Entries of a set kept while walking: enough for any name this module
/// can be asked for (the checksum covers the rest as they pass).
const KEEP: usize = 2 + NAME_MAX.div_ceil(15);

/// A root-directory file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExFile {
    first_cluster: u32,
    /// Bytes in the file.
    pub size: u64,
    /// Bytes actually written; past this the file reads as zeros.
    pub valid: u64,
    contiguous: bool,
    /// A subdirectory.
    pub is_dir: bool,
}

/// What [`ExVolume::walk_slots`] calls per entry: sector, the sector's
/// place in the directory, slot, entry; `true` stops the walk.
type SlotVisit<'a> = dyn FnMut(u32, u32, usize, &[u8; ENTRY]) -> bool + 'a;

/// A directory: its first cluster, and how its clusters follow: one run
/// of `len` clusters (no FAT chain), or through the FAT (at most `len`).
/// A folder also knows where its own entry is (the sector and slot of its
/// file entry, its set within that sector), so it can grow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Dir {
    first: u32,
    contiguous: bool,
    len: u32,
    entry: Option<(u32, usize)>,
}

/// Where a file's entry set is: its file entry's sector (and that
/// sector's place in the directory) and slot, and the set itself.
struct Found {
    lba: u32,
    nth: u32,
    slot: usize,
    /// The set's first entries (all of them when `count <= KEEP`).
    set: [[u8; ENTRY]; KEEP],
    /// Entries in the set.
    count: usize,
    /// The name was decoded whole (not cut, nothing outside ASCII), so it
    /// can be matched: a cut name must not match its first characters.
    whole: bool,
}

/// A mounted exFAT volume.
pub struct ExVolume<D> {
    dev: D,
    /// The boot sector.
    base: u32,
    /// VolumeDirty was already set at mount (then a write leaves it set).
    was_dirty: bool,
    /// Entry sets whose checksum failed, counted as directories are walked
    /// (for [`ExVolume::check`]).
    bad_sets: u32,
    /// The allocation bitmap: first sector and length in bytes (found on
    /// the first write).
    bitmap: Option<(u32, u32)>,
    fat_start: u32,
    heap_start: u32,
    cluster_shift: u32,
    clusters: u32,
    root_cluster: u32,
    buf: [u8; SECTOR],
}

fn le16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

fn le32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn le64(b: &[u8], o: usize) -> u64 {
    u64::from(le32(b, o)) | u64::from(le32(b, o + 4)) << 32
}

/// The boot region checksum over `sector` (sector `index` of 0..11): bytes
/// 106, 107 (VolumeFlags) and 112 (PercentInUse) of sector 0 don't count.
#[inline(never)]
fn boot_checksum(mut sum: u32, index: usize, sector: &[u8]) -> u32 {
    for (i, &b) in sector.iter().enumerate() {
        if index == 0 && matches!(i, 106 | 107 | 112) {
            continue;
        }
        sum = sum.rotate_right(1).wrapping_add(u32::from(b));
    }
    sum
}

/// Adds one entry to an entry set checksum (bytes 2-3 of the first entry,
/// where the checksum lives, don't count).
#[inline(never)]
fn sum_entry(mut sum: u16, e: &[u8; ENTRY], first: bool) -> u16 {
    for (i, &b) in e.iter().enumerate() {
        if !(first && (i == 2 || i == 3)) {
            sum = sum.rotate_right(1).wrapping_add(u16::from(b));
        }
    }
    sum
}

/// The entry set checksum.
fn set_checksum(set: &[[u8; ENTRY]]) -> u16 {
    set.iter()
        .enumerate()
        .fold(0, |sum, (k, e)| sum_entry(sum, e, k == 0))
}

impl<D: BlockDevice> ExVolume<D> {
    /// Mounts the exFAT volume whose boot sector is at `base`.
    #[inline(never)]
    pub fn mount(mut dev: D, base: u32) -> Result<Self, Error> {
        let mut buf = [0u8; SECTOR];
        // The boot region: 11 sectors checksummed, sector 11 repeats the sum.
        let mut sum = 0u32;
        let mut boot = [0u8; SECTOR];
        for i in 0..11 {
            dev.read_blocks(base.checked_add(i as u32).ok_or(Error::NotExFat)?, &mut buf)?;
            if i == 0 {
                boot = buf;
            }
            sum = boot_checksum(sum, i, &buf);
        }
        dev.read_blocks(base.checked_add(11).ok_or(Error::NotExFat)?, &mut buf)?;
        let b = &boot;
        if &b[3..11] != b"EXFAT   "
            || b[11..64].iter().any(|&x| x != 0)
            || b[510..512] != [0x55, 0xAA]
            || buf
                .as_chunks::<4>()
                .0
                .iter()
                .any(|w| u32::from_le_bytes(*w) != sum)
        {
            return Err(Error::NotExFat);
        }
        let fat_offset = le32(b, 80);
        let fat_length = le32(b, 84);
        let heap_offset = le32(b, 88);
        let clusters = le32(b, 92);
        let root_cluster = le32(b, 96);
        let sector_shift = u32::from(b[108]);
        let cluster_shift = u32::from(b[109]);
        let volume_length = le64(b, 72);
        // Only 512-byte sectors (the block size of every card here); at most
        // 32 MiB clusters, as the specification allows.
        if sector_shift != 9
            || cluster_shift > 16
            || clusters == 0
            || clusters > 0xFFFF_FFF5
            || !(2..2 + clusters).contains(&root_cluster)
            || u64::from(fat_length) * (SECTOR as u64 / 4) < u64::from(clusters) + 2
            || u64::from(heap_offset) + (u64::from(clusters) << cluster_shift) > volume_length
            || u64::from(fat_offset) + u64::from(fat_length) > u64::from(heap_offset)
        {
            return Err(Error::NotExFat);
        }
        let end = u64::from(base) + volume_length;
        if end > u64::from(u32::MAX) {
            // Beyond 32-bit block numbers: 2 TiB, more than any SD card.
            return Err(Error::NotExFat);
        }
        Ok(Self {
            dev,
            base,
            was_dirty: b[106] & VOLUME_DIRTY != 0,
            bad_sets: 0,
            bitmap: None,
            fat_start: base + fat_offset,
            heap_start: base + heap_offset,
            cluster_shift,
            clusters,
            root_cluster,
            buf,
        })
    }

    /// Bytes per cluster.
    #[must_use]
    pub fn cluster_bytes(&self) -> usize {
        SECTOR << self.cluster_shift
    }

    /// Data clusters on the volume.
    #[must_use]
    pub fn clusters(&self) -> u32 {
        self.clusters
    }

    /// The block device.
    pub fn device(&self) -> &D {
        &self.dev
    }

    /// Gives the device back.
    pub fn into_inner(self) -> D {
        self.dev
    }

    fn valid_cluster(&self, c: u32) -> bool {
        (2..2 + self.clusters).contains(&c)
    }

    fn cluster_lba(&self, c: u32) -> u32 {
        // In range by mount's checks for any valid cluster.
        self.heap_start + ((c - 2) << self.cluster_shift)
    }

    #[inline(never)]
    fn fat_next(&mut self, c: u32) -> Result<Option<u32>, Error> {
        let sector = self.fat_start + c / (SECTOR as u32 / 4);
        self.dev.read_blocks(sector, &mut self.buf)?;
        let v = le32(&self.buf, (c % (SECTOR as u32 / 4)) as usize * 4);
        if v == FAT_EOC {
            Ok(None)
        } else if v == FAT_BAD || !self.valid_cluster(v) {
            Err(Error::Corrupt)
        } else {
            Ok(Some(v))
        }
    }

    /// The root directory (always chained through the FAT).
    fn root(&self) -> Dir {
        Dir {
            first: self.root_cluster,
            contiguous: false,
            len: self.clusters,
            entry: None,
        }
    }

    /// The cluster after `c` in `dir`, if it has one (`k`: clusters so far).
    fn dir_next(&mut self, dir: Dir, c: u32, k: u32) -> Result<Option<u32>, Error> {
        if k + 1 >= dir.len {
            // A folder ends at its size. The root has no size: its bound is
            // the volume, and a chain that long is a loop.
            let root = dir.first == self.root_cluster && dir.entry.is_none() && !dir.contiguous;
            return if root { Err(Error::Corrupt) } else { Ok(None) };
        }
        if dir.contiguous {
            return Ok(Some(c + 1));
        }
        self.fat_next(c)
    }

    /// Calls `f` with every entry of `dir` up to and including the
    /// end-of-directory entry: its sector, that sector's place in the
    /// directory, and its slot. Stops when `f` returns `true`.
    #[inline(never)]
    fn walk_slots(&mut self, dir: Dir, f: &mut SlotVisit<'_>) -> Result<(), Error> {
        let mut cluster = dir.first;
        let mut nth = 0u32;
        for k in 0.. {
            let lba = self.cluster_lba(cluster);
            for s in 0..1u32 << self.cluster_shift {
                self.dev.read_blocks(lba + s, &mut self.buf)?;
                for (i, e) in self.buf.as_chunks::<ENTRY>().0.iter().enumerate() {
                    if f(lba + s, nth, i, e) || e[0] == 0 {
                        return Ok(());
                    }
                }
                nth += 1;
            }
            match self.dir_next(dir, cluster, k)? {
                Some(c) => cluster = c,
                None => break,
            }
        }
        Ok(())
    }

    /// `dir`'s `nth` sector, if the directory has that many.
    fn dir_sector(&mut self, dir: Dir, nth: u32) -> Result<Option<u32>, Error> {
        let mut cluster = dir.first;
        for k in 0..nth >> self.cluster_shift {
            match self.dir_next(dir, cluster, k)? {
                Some(c) => cluster = c,
                None => return Ok(None),
            }
        }
        Ok(Some(
            self.cluster_lba(cluster) + (nth & ((1 << self.cluster_shift) - 1)),
        ))
    }

    /// Calls `f` for each file or folder in `dir` whose entry set checks
    /// out, with its name (ASCII; other characters as `?`, cut at
    /// [`NAME_MAX`]) and where its set is; stops when `f` returns `true`.
    fn walk_dir(
        &mut self,
        dir: Dir,
        f: &mut dyn FnMut(&[u8], &ExFile, &Found) -> bool,
    ) -> Result<(), Error> {
        let mut found = Found {
            lba: 0,
            nth: 0,
            slot: 0,
            set: [[0u8; ENTRY]; KEEP],
            count: 0,
            whole: false,
        };
        let mut want = 0usize; // entries still to collect for the set
        let mut seen = 0usize;
        let mut sum = 0u16;
        let mut bad = 0u32;
        let r = self.walk_slots(dir, &mut |lba, nth, slot, e| {
            if e[0] == 0 {
                return true;
            }
            if want > 0 {
                if seen < KEEP {
                    found.set[seen] = *e;
                }
                seen += 1;
                sum = sum_entry(sum, e, false);
                want -= 1;
                if want > 0 {
                    return false;
                }
                found.count = seen;
                let Some((name, n, file, whole)) = parse_set(&found.set[..seen.min(KEEP)], sum)
                else {
                    bad += 1;
                    return false;
                };
                found.whole = whole;
                return f(&name[..n], &file, &found);
            }
            if e[0] == T_FILE {
                let secondary = usize::from(e[1]);
                if (2..=MAX_SECONDARY).contains(&secondary) {
                    found.lba = lba;
                    found.nth = nth;
                    found.slot = slot;
                    found.set[0] = *e;
                    seen = 1;
                    sum = sum_entry(0, e, true);
                    want = secondary;
                }
            }
            false
        });
        self.bad_sets = self.bad_sets.wrapping_add(bad);
        r
    }

    /// Checks the volume, read-only (see [`crate::check`]): every folder
    /// walked, every cluster files and folders use marked once and checked
    /// against the allocation bitmap, then the bitmap scanned for clusters
    /// nothing reached. `seen` needs a bit per cluster; `scratch` a sector
    /// and 4 bytes.
    #[inline(never)]
    pub fn check(&mut self, seen: &mut [u8], scratch: &mut [u8]) -> Result<crate::Health, Error> {
        use crate::check::{BATCH, PENDING, Seen};
        let total = self.clusters + 2;
        if seen.len() < (total as usize).div_ceil(8) || scratch.len() < SECTOR + 4 {
            return Err(Error::NoRoom);
        }
        seen[..(total as usize).div_ceil(8)].fill(0);
        let mut seen = Seen(seen);
        let mut h = crate::Health::default();
        let (bitmap_lba, bitmap_bytes) = self.bitmap()?;
        // The root's clusters, and those of the bitmap and up-case table.
        let root = self.root();
        self.mark_dir(root, &mut seen, &mut h, scratch)?;
        let mut special = [(0u32, 0u64); 2];
        self.walk_slots(root, &mut |_, _, _, e| {
            match e[0] {
                T_BITMAP => special[0] = (le32(e, 20), le64(e, 24)),
                0x82 => special[1] = (le32(e, 20), le64(e, 24)),
                _ => {}
            }
            false
        })?;
        for (first, len) in special {
            if len > 0 {
                let f = ExFile {
                    first_cluster: first,
                    size: len,
                    valid: len,
                    contiguous: false,
                    is_dir: false,
                };
                self.mark_file(&f, &mut seen, &mut h, scratch)?;
            }
        }
        let mut pending = [root; PENDING];
        let mut np = 1;
        while np > 0 {
            np -= 1;
            let dir = pending[np];
            let mut skip = 0usize;
            loop {
                let mut batch = [None; BATCH];
                let (mut nb, mut idx, mut more) = (0, 0usize, false);
                let before = self.bad_sets;
                let walked = self.walk_dir(dir, &mut |_, file, _| {
                    if idx < skip {
                        idx += 1;
                        return false;
                    }
                    if nb == BATCH {
                        more = true;
                        return true;
                    }
                    batch[nb] = Some(*file);
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
                }
                if !more {
                    // The walk that read the whole folder counts its bad sets.
                    h.bad_sets += self.bad_sets.wrapping_sub(before);
                }
                for f in batch[..nb].iter().flatten() {
                    if f.is_dir {
                        h.folders += 1;
                        match self.dir_of(f) {
                            Ok(d) => {
                                if self.mark_dir(d, &mut seen, &mut h, scratch)? {
                                    if np < PENDING {
                                        pending[np] = d;
                                        np += 1;
                                    } else {
                                        h.unchecked += 1;
                                    }
                                }
                            }
                            Err(_) => h.bad_chains += 1,
                        }
                    } else {
                        h.files += 1;
                        self.mark_file(f, &mut seen, &mut h, scratch)?;
                    }
                }
                if !more {
                    break;
                }
                skip += nb;
            }
        }
        // Allocated in the bitmap, reached by nothing.
        let mut s = 0;
        while s * SECTOR < bitmap_bytes as usize {
            self.dev
                .read_blocks(bitmap_lba + s as u32, &mut scratch[..SECTOR])?;
            for (i, &byte) in scratch[..SECTOR].iter().enumerate() {
                for bit in 0..8 {
                    let c = ((s * SECTOR + i) * 8 + bit) as u32 + 2;
                    if c < total && byte >> bit & 1 == 1 && !seen.get(c) {
                        h.lost += 1;
                    }
                }
            }
            s += 1;
        }
        Ok(h)
    }

    /// Whether cluster `c` is marked used in the bitmap (one sector cached in
    /// `cache`: its first 4 bytes the sector index + 1, then the sector).
    fn bitmap_bit(&mut self, c: u32, cache: &mut [u8]) -> Result<bool, Error> {
        let (lba0, _) = self.bitmap()?;
        let bit = c - 2;
        let sector = bit / (SECTOR as u32 * 8);
        let (tag, data) = cache.split_at_mut(4);
        if u32::from_le_bytes([tag[0], tag[1], tag[2], tag[3]]) != sector + 1 {
            self.dev.read_blocks(lba0 + sector, &mut data[..SECTOR])?;
            tag.copy_from_slice(&(sector + 1).to_le_bytes());
        }
        let i = (bit % (SECTOR as u32 * 8)) as usize;
        Ok(data[i / 8] >> (i % 8) & 1 == 1)
    }

    /// Marks one cluster of a file or folder and checks the bitmap has it.
    fn mark_one(
        &mut self,
        c: u32,
        seen: &mut crate::check::Seen<'_>,
        h: &mut crate::Health,
        cache: &mut [u8],
    ) -> Result<bool, Error> {
        if !self.valid_cluster(c) {
            h.bad_chains += 1;
            return Ok(false);
        }
        if !seen.mark(c) {
            h.cross_linked += 1;
            return Ok(false);
        }
        h.used += 1;
        if !self.bitmap_bit(c, cache)? {
            h.free_but_used += 1;
        }
        Ok(true)
    }

    /// Marks a file's clusters (one run, or its chain).
    fn mark_file(
        &mut self,
        f: &ExFile,
        seen: &mut crate::check::Seen<'_>,
        h: &mut crate::Health,
        scratch: &mut [u8],
    ) -> Result<(), Error> {
        let n = f.size.div_ceil(self.cluster_bytes() as u64);
        if n == 0 {
            return Ok(());
        }
        if n > u64::from(self.clusters) {
            h.bad_chains += 1;
            return Ok(());
        }
        let cache = &mut scratch[..SECTOR + 4];
        cache[..4].fill(0);
        let mut c = f.first_cluster;
        for k in 0..n {
            if !self.mark_one(c, seen, h, cache)? {
                return Ok(());
            }
            if k + 1 < n {
                c = if f.contiguous {
                    c + 1
                } else {
                    match self.fat_next(c) {
                        Ok(Some(next)) => next,
                        Ok(None) => {
                            h.size_mismatch += 1;
                            return Ok(());
                        }
                        Err(Error::Corrupt) => {
                            h.bad_chains += 1;
                            return Ok(());
                        }
                        Err(e) => return Err(e),
                    }
                };
            }
        }
        Ok(())
    }

    /// Marks a folder's clusters; `true` if all were reached for the first
    /// time (so it is walked once).
    fn mark_dir(
        &mut self,
        dir: Dir,
        seen: &mut crate::check::Seen<'_>,
        h: &mut crate::Health,
        scratch: &mut [u8],
    ) -> Result<bool, Error> {
        let cache = &mut scratch[..SECTOR + 4];
        cache[..4].fill(0);
        let mut c = dir.first;
        for k in 0.. {
            if !self.mark_one(c, seen, h, cache)? {
                return Ok(false);
            }
            match self.dir_next(dir, c, k) {
                Ok(Some(next)) => c = next,
                Ok(None) => break,
                Err(Error::Corrupt) => {
                    h.bad_chains += 1;
                    return Ok(false);
                }
                Err(e) => return Err(e),
            }
        }
        Ok(true)
    }
}

/// Checks (against `sum`, the whole set's checksum) and decodes a file
/// entry set from its first entries; also whether the name came out whole.
#[inline(never)]
fn parse_set(set: &[[u8; ENTRY]], sum: u16) -> Option<([u8; NAME_MAX], usize, ExFile, bool)> {
    if sum != le16(&set[0], 2) || set[1][0] != T_STREAM {
        return None;
    }
    let st = &set[1];
    let name_len = usize::from(st[3]);
    let file = ExFile {
        first_cluster: le32(st, 20),
        size: le64(st, 24),
        valid: le64(st, 8).min(le64(st, 24)),
        contiguous: st[1] & 0x02 != 0,
        is_dir: le16(&set[0], 4) & ATTR_DIRECTORY != 0,
    };
    let mut name = [0u8; NAME_MAX];
    let mut n = 0;
    let mut ascii = true;
    for e in set[2..].iter().filter(|e| e[0] == T_NAME) {
        for k in 0..15 {
            if n >= name_len.min(NAME_MAX) {
                break;
            }
            let c = le16(e, 2 + 2 * k);
            ascii &= c < 0x80;
            name[n] = if c < 0x80 { c as u8 } else { b'?' };
            n += 1;
        }
    }
    (n > 0).then_some((name, n, file, ascii && n == name_len))
}

/// The stream extension's name hash: over the up-cased name's UTF-16
/// code units, low byte first (ASCII names: the up-case table's mapping
/// for ASCII is the plain one).
#[inline(never)]
fn name_hash(name: &[u8]) -> u16 {
    let mut h = 0u16;
    for &c in name {
        for b in u16::from(c.to_ascii_uppercase()).to_le_bytes() {
            h = h.rotate_right(1).wrapping_add(u16::from(b));
        }
    }
    h
}

/// A file entry set for `name` (ASCII, checked by the caller): the file
/// entry, the stream extension, the name entries; checksummed.
fn build_set(
    name: &[u8],
    attrs: u16,
    first_cluster: u32,
    size: u64,
    contiguous: bool,
) -> ([[u8; ENTRY]; KEEP], usize) {
    let names = name.len().div_ceil(15);
    let count = 2 + names;
    let mut set = [[0u8; ENTRY]; KEEP];
    set[0][0] = T_FILE;
    set[0][1] = (1 + names) as u8;
    set[0][4..6].copy_from_slice(&attrs.to_le_bytes());
    set[1][0] = T_STREAM;
    set[1][3] = name.len() as u8;
    set[1][4..6].copy_from_slice(&name_hash(name).to_le_bytes());
    for (k, chunk) in name.chunks(15).enumerate() {
        set[2 + k][0] = T_NAME;
        for (i, &c) in chunk.iter().enumerate() {
            set[2 + k][2 + 2 * i] = c;
        }
    }
    set_stream(&mut set[..count], first_cluster, size, contiguous);
    (set, count)
}

/// Points a set's stream extension at new contents and re-checksums it.
fn set_stream(set: &mut [[u8; ENTRY]], first_cluster: u32, size: u64, contiguous: bool) {
    let st = &mut set[1];
    st[1] = ALLOC_POSSIBLE
        | if contiguous && size > 0 {
            NO_FAT_CHAIN
        } else {
            0
        };
    st[8..16].copy_from_slice(&size.to_le_bytes());
    st[20..24].copy_from_slice(&first_cluster.to_le_bytes());
    st[24..32].copy_from_slice(&size.to_le_bytes());
    let sum = set_checksum(set);
    set[0][2..4].copy_from_slice(&sum.to_le_bytes());
}

impl<D: BlockDevice> ExVolume<D> {
    /// Calls `f` with each root file's name and details.
    pub fn for_each_root(&mut self, f: impl FnMut(&[u8], &ExFile)) -> Result<(), Error> {
        self.for_each(self.root(), f)
    }

    /// Calls `f` with the name and details of each entry of the folder
    /// `path`.
    pub fn for_each_in(&mut self, path: &str, f: impl FnMut(&[u8], &ExFile)) -> Result<(), Error> {
        let dir = self.open_dir(path, 0)?;
        self.for_each(dir, f)
    }

    fn for_each(&mut self, dir: Dir, mut f: impl FnMut(&[u8], &ExFile)) -> Result<(), Error> {
        self.walk_dir(dir, &mut |n, file, _| {
            f(n, file);
            false
        })
    }

    /// Finds a file or folder by path (`/` or `\` between names; ASCII,
    /// case-insensitive).
    pub fn find(&mut self, path: &str) -> Result<ExFile, Error> {
        let (dir, leaf) = self.parent_of(path)?;
        self.find_set(dir, leaf).map(|(file, _)| file)
    }

    /// The folder `path` names (all of it, less `skip_last` components).
    #[inline(never)]
    fn open_dir(&mut self, path: &str, skip_last: usize) -> Result<Dir, Error> {
        let parts = crate::components(path)?;
        let n = crate::components(path)?.count();
        let mut dir = self.root();
        for name in parts.take(n.saturating_sub(skip_last)) {
            let (f, at) = self.find_set(dir, name)?;
            if !f.is_dir {
                return Err(Error::NotFound);
            }
            dir = self.dir_of(&f)?;
            // Growable when its whole set is in one sector and known.
            if at.slot + at.count <= SLOTS && at.count <= KEEP {
                dir.entry = Some((at.lba, at.slot));
            }
        }
        Ok(dir)
    }

    /// The folder an entry describes, checked against the volume.
    fn dir_of(&self, f: &ExFile) -> Result<Dir, Error> {
        let cb = self.cluster_bytes() as u64;
        if !self.valid_cluster(f.first_cluster) || f.size == 0 || !f.size.is_multiple_of(cb) {
            return Err(Error::Corrupt);
        }
        let len = f.size / cb;
        if len > u64::from(self.clusters)
            || (f.contiguous && u64::from(f.first_cluster) + len > 2 + u64::from(self.clusters))
        {
            return Err(Error::Corrupt);
        }
        Ok(Dir {
            first: f.first_cluster,
            contiguous: f.contiguous,
            len: len as u32,
            entry: None,
        })
    }

    /// The folder holding `path`'s last component, and that component.
    fn parent_of<'p>(&mut self, path: &'p str) -> Result<(Dir, &'p str), Error> {
        let leaf = crate::components(path)?.next_back().ok_or(Error::BadName)?;
        Ok((self.open_dir(path, 1)?, leaf))
    }

    #[inline(never)]
    fn find_set(&mut self, dir: Dir, name: &str) -> Result<(ExFile, Found), Error> {
        let want = name.as_bytes();
        let mut found = None;
        self.walk_dir(dir, &mut |n, file, at| {
            let hit = at.whole && n.eq_ignore_ascii_case(want);
            if hit {
                found = Some((
                    *file,
                    Found {
                        lba: at.lba,
                        nth: at.nth,
                        slot: at.slot,
                        set: at.set,
                        count: at.count,
                        whole: at.whole,
                    },
                ));
            }
            hit
        })?;
        found.ok_or(Error::NotFound)
    }

    /// Creates the folder `path` (its parent must exist): a zeroed cluster
    /// marked used first, then the entry that makes it exist. An existing
    /// file or folder of that name is [`Error::Exists`].
    ///
    /// # Panics
    /// If `scratch` is shorter than a sector.
    #[inline(never)]
    pub fn mkdir(&mut self, path: &str, scratch: &mut [u8]) -> Result<(), Error> {
        assert!(scratch.len() >= SECTOR, "scratch must hold a sector");
        let (parent, leaf) = self.parent_of(path)?;
        crate::check_new_name(leaf.as_bytes())?;
        match self.find_set(parent, leaf) {
            Ok(_) => return Err(Error::Exists),
            Err(Error::NotFound) => {}
            Err(e) => return Err(e),
        }
        let (Some(c), _) = self.scan(2, 1, scratch)? else {
            return Err(Error::NoRoom);
        };
        self.set_dirty(true)?;
        let zero = &mut scratch[..SECTOR];
        zero.fill(0);
        let lba = self.cluster_lba(c);
        for s in 0..1u32 << self.cluster_shift {
            self.dev.write_blocks(lba + s, zero)?;
        }
        self.mark(c, 1, true)?;
        let cb = self.cluster_bytes() as u64;
        let (set, count) = build_set(leaf.as_bytes(), ATTR_DIRECTORY, c, cb, true);
        self.add_set(parent, &set[..count], scratch)?;
        if !self.was_dirty {
            self.set_dirty(false)?;
        }
        Ok(())
    }

    /// Replaces `path`'s contents, or creates it (and any missing parent
    /// folders, like `mkdir -p`). See the module documentation for the
    /// write order that makes a power cut harmless.
    ///
    /// # Panics
    /// If `scratch` is shorter than a sector.
    #[inline(never)]
    pub fn put(&mut self, path: &str, data: &[u8], scratch: &mut [u8]) -> Result<Written, Error> {
        assert!(scratch.len() >= SECTOR, "scratch must hold a sector");
        let (dir, name) = match self.parent_of(path) {
            Err(Error::NotFound) => {
                let mut parts = crate::components(path)?;
                parts.next_back();
                for d in parts {
                    let end = d.as_ptr() as usize - path.as_ptr() as usize + d.len();
                    match self.mkdir(&path[..end], scratch) {
                        Ok(()) | Err(Error::Exists) => {}
                        Err(e) => return Err(e),
                    }
                }
                self.parent_of(path)?
            }
            r => r?,
        };
        crate::check_new_name(name.as_bytes())?;
        let old = match self.find_set(dir, name) {
            Ok((file, _)) if file.is_dir => return Err(Error::Exists),
            Ok(found) => Some(found),
            Err(Error::NotFound) => None,
            Err(e) => return Err(e),
        };
        self.set_dirty(true)?;
        let written = match self.store(data, scratch) {
            // Refused before anything was written: the volume is as it was.
            Err(Error::NoRoom) if !self.was_dirty => {
                self.set_dirty(false)?;
                return Err(Error::NoRoom);
            }
            r => r?,
        };
        let size = data.len() as u64;
        match old {
            Some((file, at)) if at.slot + 1 < SLOTS && at.count <= KEEP => {
                // The swap: file and stream entries share this sector (and
                // the whole set is known, for its checksum).
                let mut set = at.set;
                let attrs = le16(&set[0], 4) | ATTR_ARCHIVE; // changed
                set[0][4..6].copy_from_slice(&attrs.to_le_bytes());
                set_stream(
                    &mut set[..at.count],
                    written.first_cluster,
                    size,
                    written.contiguous,
                );
                self.dev.read_blocks(at.lba, &mut self.buf)?;
                let o = at.slot * ENTRY;
                if self.buf[o..o + ENTRY] != at.set[0] {
                    return Err(Error::Corrupt); // changed under us
                }
                self.buf[o..o + ENTRY].copy_from_slice(&set[0]);
                self.buf[o + ENTRY..o + 2 * ENTRY].copy_from_slice(&set[1]);
                self.dev.write_blocks(at.lba, &self.buf)?;
                self.release(&file)?;
            }
            Some((file, at)) => {
                // The file entry ends its sector (or the set has entries
                // past the name, a vendor's): a new set, then the old one
                // deleted (a cut between leaves both, both whole).
                let (set, count) = build_set(
                    name.as_bytes(),
                    ATTR_ARCHIVE,
                    written.first_cluster,
                    size,
                    written.contiguous,
                );
                self.add_set(dir, &set[..count], scratch)?;
                self.delete_set(dir, &at)?;
                self.release(&file)?;
            }
            None => {
                let (set, count) = build_set(
                    name.as_bytes(),
                    ATTR_ARCHIVE,
                    written.first_cluster,
                    size,
                    written.contiguous,
                );
                self.add_set(dir, &set[..count], scratch)?;
            }
        }
        if !self.was_dirty {
            self.set_dirty(false)?;
        }
        Ok(written)
    }

    /// Deletes the file, or the empty folder, at `path`: its entry set
    /// first (file entry's sector first, so it vanishes at once), then its
    /// clusters freed in the bitmap; a cut in between leaks space.
    #[inline(never)]
    pub fn remove(&mut self, path: &str) -> Result<(), Error> {
        let (dir, leaf) = self.parent_of(path)?;
        let (file, at) = self.find_set(dir, leaf)?;
        if file.is_dir {
            let inside = self.dir_of(&file)?;
            let mut any = false;
            self.walk_dir(inside, &mut |_, _, _| {
                any = true;
                true
            })?;
            if any {
                return Err(Error::NotEmpty);
            }
        }
        self.set_dirty(true)?;
        self.delete_set(dir, &at)?;
        self.release(&file)?;
        if !self.was_dirty {
            self.set_dirty(false)?;
        }
        Ok(())
    }

    /// Sets or clears VolumeDirty (outside the boot checksum by design);
    /// PercentInUse becomes "unknown", which it now is.
    fn set_dirty(&mut self, dirty: bool) -> Result<(), Error> {
        self.dev.read_blocks(self.base, &mut self.buf)?;
        let flags = self.buf[106];
        self.buf[106] = if dirty {
            flags | VOLUME_DIRTY
        } else {
            flags & !VOLUME_DIRTY
        };
        self.buf[112] = 0xFF;
        if self.buf[106] != flags || dirty {
            self.dev.write_blocks(self.base, &self.buf)?;
        }
        Ok(())
    }

    /// The allocation bitmap: the root's bitmap entry, checked to cover
    /// every cluster and to be one run (as formatters write it).
    fn bitmap(&mut self) -> Result<(u32, u32), Error> {
        if let Some(b) = self.bitmap {
            return Ok(b);
        }
        let mut entry = None;
        self.walk_slots(self.root(), &mut |_, _, _, e| {
            let hit = e[0] == T_BITMAP;
            if hit {
                entry = Some((le32(e, 20), le64(e, 24)));
            }
            hit
        })?;
        let (first, len) = entry.ok_or(Error::Corrupt)?;
        let need = self.clusters.div_ceil(8);
        let spans = need.div_ceil(self.cluster_bytes() as u32);
        if !self.valid_cluster(first)
            || len < u64::from(need)
            || u64::from(first) + u64::from(spans) > 2 + u64::from(self.clusters)
        {
            return Err(Error::Corrupt);
        }
        let mut c = first;
        for _ in 1..spans {
            match self.fat_next(c)? {
                Some(next) if next == c + 1 => c = next,
                _ => return Err(Error::Corrupt),
            }
        }
        let b = (self.cluster_lba(first), need);
        self.bitmap = Some(b);
        Ok(b)
    }

    /// The first run of `n` free clusters from cluster `from` on, and how
    /// many free clusters the scan saw (all of them from `from` when there
    /// is no such run).
    fn scan(&mut self, from: u32, n: u32, scratch: &mut [u8]) -> Result<(Option<u32>, u32), Error> {
        let (lba0, bytes) = self.bitmap()?;
        let total = self.clusters;
        let per = (scratch.len() / SECTOR) as u32;
        let sectors = bytes.div_ceil(SECTOR as u32);
        let bits_per = SECTOR as u32 * 8;
        let mut bit = from.saturating_sub(2);
        let (mut start, mut run, mut free) = (bit, 0u32, 0u32);
        while bit < total {
            let sector = bit / bits_per;
            let secs = (sectors - sector).min(per);
            let chunk = &mut scratch[..secs as usize * SECTOR];
            self.dev.read_blocks(lba0 + sector, chunk)?;
            let end = ((sector + secs) * bits_per).min(total);
            while bit < end {
                let byte = chunk[(bit / 8 - sector * SECTOR as u32) as usize];
                let step = if bit.is_multiple_of(8) && bit + 8 <= end && (byte == 0 || byte == 0xFF)
                {
                    8
                } else {
                    1
                };
                if (byte >> (bit % 8)) & ((1u16 << step) - 1) as u8 == 0 {
                    if run == 0 {
                        start = bit;
                    }
                    run += step;
                    free += step;
                    if run >= n {
                        return Ok((Some(start + 2), free));
                    }
                } else {
                    run = 0;
                }
                bit += step;
            }
        }
        Ok((None, free))
    }

    /// Marks clusters `first..first + n` used or free in the bitmap.
    fn mark(&mut self, first: u32, n: u32, used: bool) -> Result<(), Error> {
        let (lba0, _) = self.bitmap()?;
        let bits_per = SECTOR as u32 * 8;
        let mut bit = first - 2;
        let end = bit + n;
        while bit < end {
            let sector = bit / bits_per;
            let upto = end.min((sector + 1) * bits_per);
            self.dev.read_blocks(lba0 + sector, &mut self.buf)?;
            for b in bit..upto {
                let i = (b % bits_per / 8) as usize;
                if used {
                    self.buf[i] |= 1 << (b % 8);
                } else {
                    self.buf[i] &= !(1 << (b % 8));
                }
            }
            self.dev.write_blocks(lba0 + sector, &self.buf)?;
            bit = upto;
        }
        Ok(())
    }

    fn fat_set(&mut self, c: u32, v: u32) -> Result<(), Error> {
        let sector = self.fat_start + c / (SECTOR as u32 / 4);
        self.dev.read_blocks(sector, &mut self.buf)?;
        let o = (c % (SECTOR as u32 / 4)) as usize * 4;
        self.buf[o..o + 4].copy_from_slice(&v.to_le_bytes());
        self.dev.write_blocks(sector, &self.buf)?;
        Ok(())
    }

    /// Writes `data` from sector `lba` on (the last partial sector through
    /// `scratch`, zero-padded).
    fn write_data(&mut self, lba: u32, data: &[u8], scratch: &mut [u8]) -> Result<(), Error> {
        let whole = data.len() / SECTOR * SECTOR;
        if whole > 0 {
            self.dev.write_blocks(lba, &data[..whole])?;
        }
        if whole < data.len() {
            let tail = &mut scratch[..SECTOR];
            tail.fill(0);
            tail[..data.len() - whole].copy_from_slice(&data[whole..]);
            self.dev.write_blocks(lba + (whole / SECTOR) as u32, tail)?;
        }
        Ok(())
    }

    /// Stores `data` in free clusters, marked used: one run with no FAT
    /// chain if there is one long enough, else chained through the FAT.
    #[inline(never)]
    fn store(&mut self, data: &[u8], scratch: &mut [u8]) -> Result<Written, Error> {
        let cb = self.cluster_bytes();
        let n = u32::try_from(data.len().div_ceil(cb)).map_err(|_| Error::NoRoom)?;
        if n == 0 {
            return Ok(Written {
                first_cluster: 0,
                clusters: 0,
                contiguous: true,
            });
        }
        let (run, free) = self.scan(2, n, scratch)?;
        if let Some(first) = run {
            self.write_data(self.cluster_lba(first), data, scratch)?;
            self.mark(first, n, true)?;
            return Ok(Written {
                first_cluster: first,
                clusters: n,
                contiguous: true,
            });
        }
        if free < n {
            return Err(Error::NoRoom);
        }
        let mut first = 0;
        let mut prev = None;
        let mut from = 2;
        for chunk in data.chunks(cb) {
            let (Some(c), _) = self.scan(from, 1, scratch)? else {
                return Err(Error::NoRoom);
            };
            self.write_data(self.cluster_lba(c), chunk, scratch)?;
            self.mark(c, 1, true)?;
            match prev {
                Some(p) => self.fat_set(p, c)?,
                None => first = c,
            }
            prev = Some(c);
            from = c + 1;
        }
        if let Some(p) = prev {
            self.fat_set(p, FAT_EOC)?;
        }
        Ok(Written {
            first_cluster: first,
            clusters: n,
            contiguous: false,
        })
    }

    /// Frees an old file's clusters in the bitmap (FAT entries of free
    /// clusters mean nothing in exFAT, so they stay).
    fn release(&mut self, file: &ExFile) -> Result<(), Error> {
        if file.size == 0 || !self.valid_cluster(file.first_cluster) {
            return Ok(());
        }
        let n = file
            .size
            .div_ceil(self.cluster_bytes() as u64)
            .min(u64::from(self.clusters)) as u32;
        if file.contiguous {
            if u64::from(file.first_cluster) + u64::from(n) > 2 + u64::from(self.clusters) {
                return Err(Error::Corrupt);
            }
            return self.mark(file.first_cluster, n, false);
        }
        let mut c = file.first_cluster;
        for k in 0..n {
            let next = self.fat_next(c)?;
            self.mark(c, 1, false)?;
            match next {
                Some(nc) if k + 1 < n => c = nc,
                _ => break,
            }
        }
        Ok(())
    }

    /// Writes an entry set into `dir`: into deleted entries
    /// within one sector (one write), or at the end of the directory, the
    /// sectors after the first written first, so the set exists only once
    /// its file entry's sector (which ends the directory until then) lands.
    /// A full directory grows by a cluster first.
    #[inline(never)]
    fn add_set(
        &mut self,
        mut dir: Dir,
        set: &[[u8; ENTRY]],
        scratch: &mut [u8],
    ) -> Result<(), Error> {
        for _ in 0..2 {
            match self.place_set(dir, set) {
                Err(Error::NoRoom) => self.grow(&mut dir, scratch)?,
                r => return r,
            }
        }
        Err(Error::NoRoom)
    }

    /// Grows `dir` by one zeroed cluster, marked used before anything
    /// links it. The root links it through the FAT. A folder keeps one run
    /// when the next cluster is free; otherwise its run is chained through
    /// the FAT first (ignored while its entry still says "no FAT chain").
    /// Then one sector write of the folder's entry (size, chain flag) makes
    /// the cluster part of it.
    fn grow(&mut self, dir: &mut Dir, scratch: &mut [u8]) -> Result<(), Error> {
        if dir.entry.is_none() && dir.first != self.root_cluster {
            return Err(Error::NoRoom); // a folder whose entry spans sectors
        }
        // The directory's last cluster.
        let mut last = dir.first;
        if dir.contiguous {
            last = dir.first + dir.len - 1;
        } else {
            for k in 0.. {
                match self.dir_next(*dir, last, k)? {
                    Some(c) => last = c,
                    None => break,
                }
            }
        }
        let new = match self.scan(last + 1, 1, scratch)? {
            (Some(c), _) if c == last + 1 => c,
            _ => match self.scan(2, 1, scratch)? {
                (Some(c), _) => c,
                (None, _) => return Err(Error::NoRoom),
            },
        };
        let zero = &mut scratch[..SECTOR];
        zero.fill(0);
        let lba = self.cluster_lba(new);
        for s in 0..1u32 << self.cluster_shift {
            self.dev.write_blocks(lba + s, zero)?;
        }
        self.mark(new, 1, true)?;
        let Some((at, slot)) = dir.entry else {
            // The root: chained, no size to change.
            self.fat_set(new, FAT_EOC)?;
            self.fat_set(last, new)?;
            return Ok(());
        };
        let stays = dir.contiguous && new == last + 1;
        if !stays {
            if dir.contiguous {
                for c in dir.first..last {
                    self.fat_set(c, c + 1)?;
                }
            }
            self.fat_set(new, FAT_EOC)?;
            self.fat_set(last, new)?;
        }
        // The switch: the folder's own entry, one sector.
        self.dev.read_blocks(at, &mut self.buf)?;
        let count = 1 + usize::from(self.buf[slot * ENTRY + 1]);
        if self.buf[slot * ENTRY] != T_FILE || slot + count > SLOTS || count > KEEP {
            return Err(Error::Corrupt);
        }
        let mut set = [[0u8; ENTRY]; KEEP];
        for (k, e) in set.iter_mut().take(count).enumerate() {
            e.copy_from_slice(&self.buf[(slot + k) * ENTRY..(slot + k + 1) * ENTRY]);
        }
        let size = u64::from(dir.len + 1) * self.cluster_bytes() as u64;
        set_stream(&mut set[..count], dir.first, size, stays);
        for (k, e) in set.iter().take(count).enumerate() {
            self.buf[(slot + k) * ENTRY..(slot + k + 1) * ENTRY].copy_from_slice(e);
        }
        self.dev.write_blocks(at, &self.buf)?;
        dir.len += 1;
        dir.contiguous = stays;
        Ok(())
    }

    /// [`ExVolume::add_set`] without growing: `NoRoom` when it doesn't fit.
    fn place_set(&mut self, dir: Dir, set: &[[u8; ENTRY]]) -> Result<(), Error> {
        let count = set.len();
        let mut middle = None;
        let mut end = None;
        let mut run = (0usize, 0usize); // start slot, length
        self.walk_slots(dir, &mut |lba, nth, slot, e| {
            if slot == 0 {
                run = (0, 0);
            }
            if e[0] == 0 {
                end = Some((lba, nth, slot));
                return true;
            }
            if e[0] & 0x80 == 0 {
                if run.1 == 0 {
                    run.0 = slot;
                }
                run.1 += 1;
                if run.1 >= count {
                    middle = Some((lba, run.0));
                    return true;
                }
            } else {
                run.1 = 0;
            }
            false
        })?;
        if let Some((lba, slot)) = middle {
            return self.write_entries(lba, slot, set);
        }
        let (lba, nth, slot) = end.ok_or(Error::NoRoom)?;
        // Every sector it needs must exist before anything is written.
        let spill = (slot + count).div_ceil(SLOTS) as u32;
        let mut sectors = [0u32; KEEP];
        for k in 1..spill {
            sectors[k as usize] = self.dir_sector(dir, nth + k)?.ok_or(Error::NoRoom)?;
        }
        // The pieces in later sectors first, the file entry's sector last.
        for k in (1..spill).rev() {
            let from = k as usize * SLOTS - slot;
            let to = (from + SLOTS).min(count);
            self.write_entries(sectors[k as usize], 0, &set[from..to])?;
        }
        self.write_entries(lba, slot, &set[..(SLOTS - slot).min(count)])
    }

    fn write_entries(
        &mut self,
        lba: u32,
        slot: usize,
        entries: &[[u8; ENTRY]],
    ) -> Result<(), Error> {
        self.dev.read_blocks(lba, &mut self.buf)?;
        for (k, e) in entries.iter().enumerate() {
            let o = (slot + k) * ENTRY;
            self.buf[o..o + ENTRY].copy_from_slice(e);
        }
        self.dev.write_blocks(lba, &self.buf)?;
        Ok(())
    }

    /// Marks a set's entries unused (its file entry's sector first).
    fn delete_set(&mut self, dir: Dir, at: &Found) -> Result<(), Error> {
        let spill = (at.slot + at.count).div_ceil(SLOTS) as u32;
        for k in 0..spill {
            let lba = if k == 0 {
                at.lba
            } else {
                self.dir_sector(dir, at.nth + k)?.ok_or(Error::Corrupt)?
            };
            self.dev.read_blocks(lba, &mut self.buf)?;
            let from = if k == 0 { at.slot } else { 0 };
            let to = (at.slot + at.count - k as usize * SLOTS).min(SLOTS);
            for s in from..to {
                self.buf[s * ENTRY] &= 0x7F;
            }
            self.dev.write_blocks(lba, &self.buf)?;
        }
        Ok(())
    }

    /// Reads `file` in order, calling `f` with each piece (at most
    /// `scratch`'s length, whole sectors); bytes past the valid data length
    /// come as zeros.
    ///
    /// # Panics
    /// If `scratch` is shorter than a sector.
    #[inline(never)]
    pub fn read_file(
        &mut self,
        file: &ExFile,
        scratch: &mut [u8],
        mut f: impl FnMut(&[u8]),
    ) -> Result<(), Error> {
        let max = scratch.len() / SECTOR * SECTOR;
        assert!(max >= SECTOR, "scratch must hold a sector");
        if file.size == 0 {
            return Ok(());
        }
        if !self.valid_cluster(file.first_cluster) {
            return Err(Error::Corrupt);
        }
        let cb = self.cluster_bytes() as u64;
        let total_clusters = file.size.div_ceil(cb);
        if total_clusters > u64::from(self.clusters) {
            return Err(Error::Corrupt);
        }
        if file.contiguous
            && u64::from(file.first_cluster) + total_clusters > 2 + u64::from(self.clusters)
        {
            return Err(Error::Corrupt);
        }
        let mut done = 0u64;
        let mut cluster = file.first_cluster;
        for k in 0..total_clusters {
            let lba = self.cluster_lba(cluster);
            let mut off = 0u64;
            while off < cb && done < file.size {
                let n = (max as u64).min(cb - off).min(file.size - done) as usize;
                let secs = n.div_ceil(SECTOR);
                let piece = &mut scratch[..secs * SECTOR];
                if done >= file.valid {
                    piece.fill(0);
                } else {
                    self.dev
                        .read_blocks(lba + (off / SECTOR as u64) as u32, piece)?;
                    // Zeros from the valid length on, within this piece.
                    let keep = (file.valid - done).min(n as u64) as usize;
                    piece[keep..n].fill(0);
                }
                f(&piece[..n]);
                done += n as u64;
                off += n as u64;
            }
            if k + 1 < total_clusters {
                cluster = if file.contiguous {
                    cluster + 1
                } else {
                    self.fat_next(cluster)?.ok_or(Error::Corrupt)?
                };
            }
        }
        Ok(())
    }
}

/// A small exFAT formatter (host only), for the tests and the emulator's
/// card: MBR with one partition (type 0x07), the boot region with its
/// checksum, the FAT, an allocation bitmap, an up-case table, and files in
/// the root directory (contiguous, or chained through the FAT when
/// `fragment` asks for it).
#[cfg(any(test, feature = "std"))]
pub mod format {
    use super::{ENTRY, SECTOR, boot_checksum, set_checksum};

    /// Where the partition starts.
    pub const PARTITION_START: u32 = 2048;

    /// A file for the root directory.
    pub struct Entry<'a> {
        /// Name (ASCII).
        pub name: &'a str,
        /// Contents.
        pub data: &'a [u8],
        /// Store it through the FAT, every other cluster (fragmented).
        pub fragment: bool,
    }

    /// Builds a whole disk image of `total_sectors` with 2^`cluster_shift`
    /// sectors per cluster.
    ///
    /// # Panics
    /// If the files don't fit.
    #[must_use]
    pub fn build(total_sectors: u32, cluster_shift: u32, files: &[Entry<'_>]) -> Vec<u8> {
        let spc = 1u32 << cluster_shift;
        let cb = (spc as usize) * SECTOR;
        let vol = total_sectors - PARTITION_START;
        let fat_offset = 24u32;
        let clusters_est = vol / spc;
        let fat_length = (clusters_est + 2).div_ceil(128);
        let heap_offset = (fat_offset + fat_length).next_multiple_of(spc);
        let clusters = (vol - heap_offset) / spc;
        let mut disk = vec![0u8; total_sectors as usize * SECTOR];
        let e = 446;
        disk[e + 4] = 0x07;
        disk[e + 8..e + 12].copy_from_slice(&PARTITION_START.to_le_bytes());
        disk[e + 12..e + 16].copy_from_slice(&vol.to_le_bytes());
        disk[510] = 0x55;
        disk[511] = 0xAA;

        let base = PARTITION_START as usize * SECTOR;
        let mut fat = vec![0u32; fat_length as usize * 128];
        fat[0] = 0xFFFF_FFF8;
        fat[1] = 0xFFFF_FFFF;
        let mut bitmap = vec![0u8; (clusters as usize).div_ceil(8)];
        let mut next = 2u32;
        let mut used = |n: u32, chained: bool, fat: &mut Vec<u32>, bitmap: &mut Vec<u8>| -> u32 {
            let first = next;
            for k in 0..n {
                let c = next;
                bitmap[(c - 2) as usize / 8] |= 1 << ((c - 2) % 8);
                if chained {
                    fat[c as usize] = if k + 1 == n { 0xFFFF_FFFF } else { c + 2 };
                    next += 2;
                } else {
                    next += 1;
                }
                assert!(next < clusters + 2, "image too small");
            }
            if !chained && n > 0 {
                fat[(first + n - 1) as usize] = 0; // contiguous: no FAT chain
            }
            first
        };
        let cl = |c: u32| base + ((heap_offset + (c - 2) * spc) as usize) * SECTOR;

        let bitmap_cluster = used(
            (bitmap.len() as u32).div_ceil(cb as u32).max(1),
            true,
            &mut fat,
            &mut bitmap,
        );
        let upcase: Vec<u8> = (0u16..128)
            .map(|c| {
                if (b'a' as u16..=b'z' as u16).contains(&c) {
                    c - 32
                } else {
                    c
                }
            })
            .flat_map(u16::to_le_bytes)
            .collect();
        let upcase_cluster = used(1, true, &mut fat, &mut bitmap);
        let root_cluster = used(1, true, &mut fat, &mut bitmap);
        let mut root = vec![0u8; cb];
        let mut at = 0;
        let mut put = |root: &mut Vec<u8>, e: [u8; ENTRY]| {
            root[at..at + ENTRY].copy_from_slice(&e);
            at += ENTRY;
        };
        let mut e = [0u8; ENTRY];
        e[0] = 0x81;
        e[20..24].copy_from_slice(&bitmap_cluster.to_le_bytes());
        e[24..32].copy_from_slice(&(bitmap.len() as u64).to_le_bytes());
        put(&mut root, e);
        let mut e = [0u8; ENTRY];
        e[0] = 0x82;
        let up_sum = upcase
            .iter()
            .fold(0u32, |s, &b| s.rotate_right(1).wrapping_add(u32::from(b)));
        e[4..8].copy_from_slice(&up_sum.to_le_bytes());
        e[20..24].copy_from_slice(&upcase_cluster.to_le_bytes());
        e[24..32].copy_from_slice(&(upcase.len() as u64).to_le_bytes());
        put(&mut root, e);

        let mut placed = Vec::new();
        for f in files {
            let n = (f.data.len() as u32).div_ceil(cb as u32);
            let first = if n == 0 {
                0
            } else {
                used(n, f.fragment, &mut fat, &mut bitmap)
            };
            placed.push((first, n));
            let name: Vec<u16> = f.name.encode_utf16().collect();
            let names = name.len().div_ceil(15);
            let mut set = vec![[0u8; ENTRY]; 2 + names];
            set[0][0] = 0x85;
            set[0][1] = (1 + names) as u8;
            set[0][4] = 0x20;
            set[1][0] = 0xC0;
            set[1][1] = 0x01 | if f.fragment { 0 } else { 0x02 };
            set[1][3] = name.len() as u8;
            set[1][4..6].copy_from_slice(&super::name_hash(f.name.as_bytes()).to_le_bytes());
            set[1][8..16].copy_from_slice(&(f.data.len() as u64).to_le_bytes());
            set[1][20..24].copy_from_slice(&first.to_le_bytes());
            set[1][24..32].copy_from_slice(&(f.data.len() as u64).to_le_bytes());
            for (k, chunk) in name.chunks(15).enumerate() {
                let s = &mut set[2 + k];
                s[0] = 0xC1;
                for (i, &c) in chunk.iter().enumerate() {
                    s[2 + 2 * i..4 + 2 * i].copy_from_slice(&c.to_le_bytes());
                }
            }
            let sum = set_checksum(&set);
            set[0][2..4].copy_from_slice(&sum.to_le_bytes());
            for s in set {
                put(&mut root, s);
            }
        }
        disk[cl(root_cluster)..cl(root_cluster) + cb].copy_from_slice(&root);
        disk[cl(bitmap_cluster)..cl(bitmap_cluster) + bitmap.len()].copy_from_slice(&bitmap);
        disk[cl(upcase_cluster)..cl(upcase_cluster) + upcase.len()].copy_from_slice(&upcase);
        for (f, &(first, n)) in files.iter().zip(&placed) {
            for k in 0..n as usize {
                let c = if f.fragment {
                    first + 2 * k as u32
                } else {
                    first + k as u32
                };
                let piece = &f.data[k * cb..((k + 1) * cb).min(f.data.len())];
                disk[cl(c)..cl(c) + piece.len()].copy_from_slice(piece);
            }
        }
        let fo = base + fat_offset as usize * SECTOR;
        for (i, v) in fat.iter().enumerate() {
            disk[fo + i * 4..fo + i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }

        // Boot sector and its region checksum.
        let b = &mut disk[base..base + SECTOR];
        b[0..3].copy_from_slice(&[0xEB, 0x76, 0x90]);
        b[3..11].copy_from_slice(b"EXFAT   ");
        b[64..72].copy_from_slice(&u64::from(PARTITION_START).to_le_bytes());
        b[72..80].copy_from_slice(&u64::from(vol).to_le_bytes());
        b[80..84].copy_from_slice(&fat_offset.to_le_bytes());
        b[84..88].copy_from_slice(&fat_length.to_le_bytes());
        b[88..92].copy_from_slice(&heap_offset.to_le_bytes());
        b[92..96].copy_from_slice(&clusters.to_le_bytes());
        b[96..100].copy_from_slice(&root_cluster.to_le_bytes());
        b[100..104].copy_from_slice(&0x2026_1012u32.to_le_bytes());
        b[104..106].copy_from_slice(&0x0100u16.to_le_bytes());
        b[108] = 9;
        b[109] = cluster_shift as u8;
        b[110] = 1;
        b[111] = 0x80;
        b[510] = 0x55;
        b[511] = 0xAA;
        let mut sum = 0u32;
        for i in 0..11 {
            let s = &disk[base + i * SECTOR..base + (i + 1) * SECTOR];
            sum = boot_checksum(sum, i, s);
        }
        for w in disk[base + 11 * SECTOR..base + 12 * SECTOR]
            .as_chunks_mut::<4>()
            .0
        {
            w.copy_from_slice(&sum.to_le_bytes());
        }
        disk
    }
}

#[cfg(test)]
mod tests {
    use super::format::{Entry, PARTITION_START, build};
    use super::*;
    use rp_hal_api::BlockError;

    /// A card in memory; writes can be made to fail from the `fail_at`th
    /// on (a power cut: that write and every later one never land).
    #[allow(non_snake_case)]
    fn Mem(data: Vec<u8>) -> Mem {
        Mem {
            data,
            writes: 0,
            fail_at: None,
        }
    }

    struct Mem {
        data: Vec<u8>,
        writes: usize,
        fail_at: Option<usize>,
    }

    impl BlockDevice for Mem {
        fn read_blocks(&mut self, lba: u32, buf: &mut [u8]) -> Result<(), BlockError> {
            let o = lba as usize * SECTOR;
            let src = self
                .data
                .get(o..o + buf.len())
                .ok_or(BlockError::OutOfRange)?;
            buf.copy_from_slice(src);
            Ok(())
        }
        fn write_blocks(&mut self, lba: u32, buf: &[u8]) -> Result<(), BlockError> {
            if self.fail_at.is_some_and(|f| self.writes >= f) {
                return Err(BlockError::Device);
            }
            self.writes += 1;
            let o = lba as usize * SECTOR;
            self.data
                .get_mut(o..o + buf.len())
                .ok_or(BlockError::OutOfRange)?
                .copy_from_slice(buf);
            Ok(())
        }
    }

    fn pattern(n: usize, seed: u8) -> Vec<u8> {
        (0..n).map(|i| (i as u8).wrapping_mul(29) ^ seed).collect()
    }

    fn read_all(v: &mut ExVolume<Mem>, name: &str, scratch: usize) -> Vec<u8> {
        let f = v.find(name).unwrap();
        let mut out = Vec::new();
        let mut s = vec![0u8; scratch];
        v.read_file(&f, &mut s, |c| out.extend_from_slice(c))
            .unwrap();
        out
    }

    fn image() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        let a = pattern(9000, 1);
        let b = pattern(5000, 2);
        let img = build(
            2048 + 2048,
            2,
            &[
                Entry {
                    name: "retropane.bin",
                    data: &a,
                    fragment: false,
                },
                Entry {
                    name: "A Fragmented File With A Long Name.dat",
                    data: &b,
                    fragment: true,
                },
                Entry {
                    name: "empty.txt",
                    data: b"",
                    fragment: false,
                },
            ],
        );
        (img, a, b)
    }

    /// Whether cluster `c` is marked used in the bitmap.
    fn used(v: &mut ExVolume<Mem>, c: u32) -> bool {
        let (lba0, _) = v.bitmap().unwrap();
        let bit = (c - 2) as usize;
        v.dev.data[lba0 as usize * SECTOR + bit / 8] >> (bit % 8) & 1 == 1
    }

    fn used_count(v: &mut ExVolume<Mem>) -> u32 {
        (2..2 + v.clusters).filter(|&c| used(v, c)).count() as u32
    }

    /// Every cluster of every file is marked used (a power cut must never
    /// leave live data in "free" space), and the boot region still checks.
    fn consistent(v: &mut ExVolume<Mem>) {
        let mut files = Vec::new();
        v.for_each_root(|n, f| files.push((String::from_utf8_lossy(n).into_owned(), *f)))
            .unwrap();
        for (name, f) in files {
            if f.size == 0 {
                continue;
            }
            let n = f.size.div_ceil(v.cluster_bytes() as u64) as u32;
            let mut c = f.first_cluster;
            for k in 0..n {
                assert!(used(v, c), "{name}: cluster {c} is marked free");
                if k + 1 < n {
                    c = if f.contiguous {
                        c + 1
                    } else {
                        v.fat_next(c).unwrap().unwrap()
                    };
                }
            }
        }
        let img = v.dev.data.clone();
        assert!(ExVolume::mount(Mem(img), PARTITION_START).is_ok());
    }

    fn dirty(v: &ExVolume<Mem>) -> bool {
        v.dev.data[PARTITION_START as usize * SECTOR + 106] & VOLUME_DIRTY != 0
    }

    #[test]
    fn replace_and_create_keep_the_bitmap_and_checksums_right() {
        let (img, _, b) = image();
        let mut v = ExVolume::mount(Mem(img), PARTITION_START).unwrap();
        let mut scratch = vec![0u8; 4096];
        let before = used_count(&mut v);
        let old = v.find("retropane.bin").unwrap();
        let new = pattern(20_000, 7);
        let w = v.put("RetroPane.bin", &new, &mut scratch).unwrap();
        assert!(w.contiguous && w.clusters == 10);
        assert_eq!(read_all(&mut v, "retropane.bin", 4096), new);
        assert_eq!(
            read_all(&mut v, "a fragmented file with a long name.dat", 512),
            b
        );
        // 9,000 bytes (5 clusters) out, 20,000 (10) in.
        assert_eq!(used_count(&mut v), before - 5 + 10);
        assert!(!used(&mut v, old.first_cluster));
        assert!(!dirty(&v));
        // Create: short, long (64), mixed case; and an empty file.
        let long = "L".repeat(30) + &"o".repeat(30) + ".txt";
        for (name, data) in [
            ("new.txt", pattern(100, 3)),
            (long.as_str(), pattern(3000, 4)),
            ("Mixed Case.Dat", pattern(4096, 5)),
            ("zero", Vec::new()),
        ] {
            v.put(name, &data, &mut scratch).unwrap();
            assert_eq!(read_all(&mut v, &name.to_uppercase(), 512), data);
        }
        // The name hash is over the up-cased name, as Windows looks it up.
        let root = v.root();
        let (_, at) = v.find_set(root, "mixed case.dat").unwrap();
        assert_eq!(le16(&at.set[1], 4), name_hash(b"MIXED CASE.DAT"));
        assert_eq!(name_hash(b"Mixed Case.Dat"), name_hash(b"MIXED CASE.DAT"));
        // Replace with nothing, then with something again.
        v.put("new.txt", b"", &mut scratch).unwrap();
        assert_eq!(read_all(&mut v, "new.txt", 512), b"");
        v.put("new.txt", b"back", &mut scratch).unwrap();
        assert_eq!(read_all(&mut v, "new.txt", 512), b"back");
        consistent(&mut v);
        // Bad names and folders are refused.
        assert_eq!(v.put("a*b", b"x", &mut scratch), Err(Error::BadName));
        assert_eq!(
            v.put(&"n".repeat(65), b"x", &mut scratch),
            Err(Error::BadName)
        );
    }

    /// A file whose name is longer than this module reads (cut at
    /// NAME_MAX) must not be taken for one named by its first characters.
    #[test]
    fn a_cut_long_name_never_matches_its_prefix() {
        let long = "x".repeat(80);
        let img = build(
            4096,
            2,
            &[Entry {
                name: &long,
                data: b"keep me",
                fragment: false,
            }],
        );
        let mut v = ExVolume::mount(Mem(img), PARTITION_START).unwrap();
        let prefix = &long[..NAME_MAX];
        assert_eq!(v.find(prefix), Err(Error::NotFound));
        let mut scratch = vec![0u8; 4096];
        v.put(prefix, b"new", &mut scratch).unwrap();
        // Two files now: the long one untouched.
        let mut n = 0;
        let mut long_data = None;
        v.for_each_root(|name, f| {
            n += 1;
            if name.len() == NAME_MAX && f.size == 7 {
                long_data = Some(*f);
            }
        })
        .unwrap();
        assert_eq!(n, 2);
        let f = long_data.expect("the long-named file is still there");
        let mut got = Vec::new();
        v.read_file(&f, &mut scratch, |c| got.extend_from_slice(c))
            .unwrap();
        assert_eq!(got, b"keep me");
    }

    #[test]
    fn too_big_is_refused_with_the_card_unchanged() {
        let (img, _, _) = image();
        let mut v = ExVolume::mount(Mem(img.clone()), PARTITION_START).unwrap();
        let mut scratch = vec![0u8; 4096];
        let huge = vec![1u8; 4 * 1024 * 1024];
        assert_eq!(v.put("big.bin", &huge, &mut scratch), Err(Error::NoRoom));
        assert!(!dirty(&v));
        assert!(v.find("big.bin").is_err());
        let mut v2 = ExVolume::mount(Mem(img), PARTITION_START).unwrap();
        assert_eq!(used_count(&mut v), used_count(&mut v2));
    }

    #[test]
    fn a_fragmented_card_gets_a_fat_chain() {
        let (img, _, _) = image();
        let mut v = ExVolume::mount(Mem(img), PARTITION_START).unwrap();
        let mut scratch = vec![0u8; 512];
        // Every other free cluster taken: no run of two is left.
        let free: Vec<u32> = (2..2 + v.clusters).filter(|&c| !used(&mut v, c)).collect();
        for &c in free.iter().step_by(2) {
            v.mark(c, 1, true).unwrap();
        }
        let data = pattern(7000, 9); // 4 clusters
        let w = v.put("frag.bin", &data, &mut scratch).unwrap();
        assert!(!w.contiguous);
        assert_eq!(w.first_cluster, free[1]);
        let f = v.find("frag.bin").unwrap();
        assert!(!f.contiguous);
        assert_eq!(read_all(&mut v, "frag.bin", 512), data);
        // Replaced by a contiguous file (once room is made), the chain's
        // clusters are freed.
        for &c in free.iter().step_by(2) {
            v.mark(c, 1, false).unwrap();
        }
        let w2 = v.put("frag.bin", &pattern(5000, 1), &mut scratch).unwrap();
        assert!(w2.contiguous);
        for k in [1usize, 3, 5, 7] {
            if w2.first_cluster > free[k] || free[k] >= w2.first_cluster + w2.clusters {
                assert!(
                    !used(&mut v, free[k]),
                    "old chain cluster {} still used",
                    free[k]
                );
            }
        }
        consistent(&mut v);
    }

    /// Sets whose file entry ends a sector take the add-then-delete path,
    /// and sets created across sectors at the end of the directory.
    #[test]
    fn sets_across_sector_boundaries() {
        let (img, _, _) = image();
        let mut v = ExVolume::mount(Mem(img), PARTITION_START).unwrap();
        let mut scratch = vec![0u8; 4096];
        // Root: bitmap, up-case, then sets of 3, 5, 3 entries: slot 13 next.
        // Six 3-entry files fill 13..30; the next set's file entry is at 31
        // (the last slot of the second sector) and spills into the third.
        for i in 0..6 {
            v.put(&format!("f{i}"), &[i as u8; 10], &mut scratch)
                .unwrap();
        }
        let edge = "a name long enough for two name entries";
        v.put(edge, b"first", &mut scratch).unwrap();
        let root = v.root();
        let (_, at) = v.find_set(root, edge).unwrap();
        assert_eq!((at.nth, at.slot), (1, 15));
        v.put(edge, b"second", &mut scratch).unwrap();
        assert_eq!(read_all(&mut v, edge, 512), b"second");
        let mut seen = 0;
        v.for_each_root(|n, _| seen += usize::from(n == edge.as_bytes()))
            .unwrap();
        assert_eq!(seen, 1, "the old set is gone");
        for i in 0..6 {
            assert_eq!(read_all(&mut v, &format!("f{i}"), 512), [i as u8; 10]);
        }
        consistent(&mut v);
    }

    /// A power cut at every write of each kind of put: afterwards the file
    /// is whole (old or new; absent for a create), the others untouched,
    /// no live cluster marked free, and the card marked dirty.
    #[test]
    fn power_loss_at_every_write_is_harmless() {
        let (img, a, b) = image();
        let mut scratch = vec![0u8; 2048];
        let new = pattern(12_000, 11);
        let edge = "a name long enough for two name entries";
        // A card where the edge-case set already exists.
        let mut prep = ExVolume::mount(Mem(img.clone()), PARTITION_START).unwrap();
        for i in 0..6 {
            prep.put(&format!("f{i}"), &[1; 10], &mut scratch).unwrap();
        }
        prep.put(edge, b"old", &mut scratch).unwrap();
        let edge_img = prep.into_inner().data;
        // Fragmented: every other free cluster taken.
        let mut frag = ExVolume::mount(Mem(img.clone()), PARTITION_START).unwrap();
        let free: Vec<u32> = (2..2 + frag.clusters)
            .filter(|&c| !used(&mut frag, c))
            .collect();
        for &c in free.iter().step_by(2) {
            frag.mark(c, 1, true).unwrap();
        }
        let frag_img = frag.into_inner().data;
        // (card, file put, its old contents if it existed)
        type Case<'a> = (&'a Vec<u8>, &'a str, Option<&'a [u8]>);
        let cases: [Case<'_>; 4] = [
            (&img, "retropane.bin", Some(&a)),
            (&img, "brand new.bin", None),
            (&edge_img, edge, Some(b"old")),
            (&frag_img, "retropane.bin", Some(&a)),
        ];
        for (base, name, old) in cases {
            let total = {
                let mut v = ExVolume::mount(Mem(base.clone()), PARTITION_START).unwrap();
                v.put(name, &new, &mut scratch).unwrap();
                v.dev.writes
            };
            for fail_at in 0..total {
                let mut disk = Mem(base.clone());
                disk.fail_at = Some(fail_at);
                let mut v = ExVolume::mount(disk, PARTITION_START).unwrap();
                assert!(v.put(name, &new, &mut scratch).is_err());
                let after = v.into_inner().data;
                let mut v = ExVolume::mount(Mem(after), PARTITION_START).unwrap();
                match (v.find(name), old) {
                    (Ok(_), _) => {
                        let got = read_all(&mut v, name, 2048);
                        assert!(
                            got == new || old.is_some_and(|o| got == o),
                            "{name} at write {fail_at}: neither old nor new"
                        );
                    }
                    (Err(Error::NotFound), None) => {}
                    (r, _) => panic!("{name} at write {fail_at}: {r:?}"),
                }
                if name != "retropane.bin" {
                    assert_eq!(read_all(&mut v, "retropane.bin", 2048), a);
                }
                assert_eq!(
                    read_all(&mut v, "a fragmented file with a long name.dat", 2048),
                    b
                );
                consistent(&mut v);
                assert!(
                    fail_at == 0 || dirty(&v),
                    "{name}: a cut left the card clean"
                );
            }
        }
    }

    #[test]
    fn folders_paths_and_put_creating_parents() {
        let (img, a, _) = image();
        let mut v = ExVolume::mount(Mem(img), PARTITION_START).unwrap();
        let mut scratch = vec![0u8; 2048];
        v.mkdir("Apps", &mut scratch).unwrap();
        assert_eq!(v.mkdir("apps", &mut scratch), Err(Error::Exists));
        assert_eq!(v.mkdir("retropane.bin", &mut scratch), Err(Error::Exists));
        let game = pattern(5000, 21);
        v.put("Apps/Game One.exe", &game, &mut scratch).unwrap();
        // Parents made as needed, either separator.
        v.put("Apps\\Sub Folder\\deep/readme.txt", b"deep", &mut scratch)
            .unwrap();
        assert_eq!(read_all(&mut v, "apps/game one.exe", 4096), game);
        assert_eq!(
            read_all(&mut v, "Apps/Sub Folder/deep/readme.txt", 512),
            b"deep"
        );
        // Replace inside a folder, and the root untouched.
        v.put("Apps/Game One.exe", b"v2", &mut scratch).unwrap();
        assert_eq!(read_all(&mut v, "Apps/Game One.exe", 512), b"v2");
        assert_eq!(read_all(&mut v, "retropane.bin", 4096), a);
        let mut apps = Vec::new();
        v.for_each_in("Apps", |n, f| {
            apps.push((String::from_utf8_lossy(n).into_owned(), f.is_dir))
        })
        .unwrap();
        assert_eq!(
            apps,
            [("Game One.exe".into(), false), ("Sub Folder".into(), true)]
        );
        // Folders are one run, as Windows writes them.
        let f = v.find("Apps").unwrap();
        assert!(f.is_dir && f.contiguous && f.size == v.cluster_bytes() as u64);
        // A file isn't a folder; missing folders aren't found.
        assert_eq!(v.find("retropane.bin/x"), Err(Error::NotFound));
        assert_eq!(v.find("nothere/x"), Err(Error::NotFound));
        assert_eq!(v.find(&("a/".repeat(20) + "f")), Err(Error::BadName));
        assert!(!dirty(&v));
        consistent(&mut v);
    }

    #[test]
    fn folders_grow_in_one_run_or_by_a_chain_and_the_root_too() {
        let (img, _, _) = image();
        let mut v = ExVolume::mount(Mem(img), PARTITION_START).unwrap();
        let mut scratch = vec![0u8; 2048];
        // 2 KiB clusters: 64 entries, 3 per short name: 21 files a cluster.
        v.mkdir("run", &mut scratch).unwrap();
        for i in 0..50 {
            v.put(&format!("run/f{i:02}"), &[i as u8], &mut scratch)
                .unwrap();
        }
        let f = v.find("run").unwrap();
        assert_eq!(f.size, 3 * 2048);
        // The run's next clusters were taken by the files: it chained.
        assert!(!f.contiguous);
        for i in 0..50 {
            assert_eq!(read_all(&mut v, &format!("run/f{i:02}"), 512), [i as u8]);
        }
        // A folder with free space after it stays one run.
        v.mkdir("solo", &mut scratch).unwrap();
        let solo = v.find("solo").unwrap();
        // Empty files take no clusters: the one after the folder stays
        // free, so it grows in one run.
        assert!(!used(&mut v, solo.first_cluster + 1));
        let mut v2 = v;
        for i in 0..30 {
            v2.put(&format!("solo/{i:02}"), b"", &mut scratch).unwrap();
        }
        let solo = v2.find("solo").unwrap();
        assert_eq!(solo.size, 2 * 2048);
        assert!(solo.contiguous, "grew into the free cluster after it");
        // The root grows through the FAT.
        for i in 0..40 {
            v2.put(&format!("root file {i:02}.txt"), &[1], &mut scratch)
                .unwrap();
        }
        for i in 0..40 {
            assert_eq!(
                read_all(&mut v2, &format!("root file {i:02}.txt"), 512),
                [1]
            );
        }
        consistent(&mut v2);
        assert!(!dirty(&v2));
    }

    /// A power cut at every write of a put that has to grow a folder: the
    /// folder and its files stay readable, the new file whole or absent.
    #[test]
    fn power_loss_while_a_folder_grows_is_harmless() {
        let (img, _, _) = image();
        let mut scratch = vec![0u8; 2048];
        let mut v = ExVolume::mount(Mem(img), PARTITION_START).unwrap();
        v.mkdir("d", &mut scratch).unwrap();
        for i in 0..21 {
            v.put(&format!("d/{i:02}"), &[i as u8], &mut scratch)
                .unwrap();
        }
        // Taking the cluster after the folder forces a chain conversion.
        let d = v.find("d").unwrap();
        if !used(&mut v, d.first_cluster + 1) {
            v.mark(d.first_cluster + 1, 1, true).unwrap();
        }
        // Free space full of valid entry sets for a file named "ghost": a
        // new folder cluster that isn't zeroed would list them.
        let (ghost, n) = build_set(b"ghost", ATTR_ARCHIVE, 0, 0, true);
        let free: Vec<u32> = (2..2 + v.clusters).filter(|&c| !used(&mut v, c)).collect();
        for c in free {
            let o = v.cluster_lba(c) as usize * SECTOR;
            for (k, e) in v.dev.data[o..o + 2048]
                .as_chunks_mut::<ENTRY>()
                .0
                .iter_mut()
                .enumerate()
            {
                e.copy_from_slice(&ghost[k % n]);
            }
        }
        let ghosts = |v: &mut ExVolume<Mem>| {
            let mut found = false;
            let _ = v.for_each_in("d", |name, _| found |= name == b"ghost");
            found
        };
        let full = v.into_inner().data;
        let total = {
            let mut v = ExVolume::mount(Mem(full.clone()), PARTITION_START).unwrap();
            v.put("d/new", b"grown", &mut scratch).unwrap();
            assert!(!v.find("d").unwrap().contiguous);
            assert!(!ghosts(&mut v), "the new folder cluster wasn't zeroed");
            v.dev.writes
        };
        for fail_at in 0..total {
            let mut disk = Mem(full.clone());
            disk.fail_at = Some(fail_at);
            let mut v = ExVolume::mount(disk, PARTITION_START).unwrap();
            assert!(v.put("d/new", b"grown", &mut scratch).is_err());
            let mut v = ExVolume::mount(Mem(v.into_inner().data), PARTITION_START).unwrap();
            for i in 0..21 {
                assert_eq!(
                    read_all(&mut v, &format!("d/{i:02}"), 512),
                    [i as u8],
                    "write {fail_at}"
                );
            }
            match v.find("d/new") {
                Ok(_) => assert_eq!(read_all(&mut v, "d/new", 512), b"grown"),
                Err(Error::NotFound) => {}
                Err(e) => panic!("write {fail_at}: {e:?}"),
            }
            assert!(
                !ghosts(&mut v),
                "write {fail_at}: stale entries in the folder"
            );
            consistent(&mut v);
        }
    }

    #[test]
    fn power_loss_while_making_folders_is_harmless() {
        let (img, a, _) = image();
        let mut scratch = vec![0u8; 2048];
        let data = pattern(3000, 22);
        let total = {
            let mut v = ExVolume::mount(Mem(img.clone()), PARTITION_START).unwrap();
            v.put("x/y/z.bin", &data, &mut scratch).unwrap();
            v.dev.writes
        };
        for fail_at in 0..total {
            let mut disk = Mem(img.clone());
            disk.fail_at = Some(fail_at);
            let mut v = ExVolume::mount(disk, PARTITION_START).unwrap();
            assert!(v.put("x/y/z.bin", &data, &mut scratch).is_err());
            let after = v.into_inner().data;
            let mut v = ExVolume::mount(Mem(after), PARTITION_START).unwrap();
            match v.find("x/y/z.bin") {
                Ok(_) => assert_eq!(read_all(&mut v, "x/y/z.bin", 2048), data),
                Err(Error::NotFound) => {}
                Err(e) => panic!("write {fail_at}: {e:?}"),
            }
            let _ = v.for_each_in("x", |_, _| {});
            let _ = v.for_each_in("x/y", |_, _| {});
            assert_eq!(read_all(&mut v, "retropane.bin", 2048), a);
            consistent(&mut v);
        }
    }

    #[test]
    fn a_folder_that_points_back_at_the_root_is_corrupt_not_a_loop() {
        let (img, _, _) = image();
        let mut v = ExVolume::mount(Mem(img), PARTITION_START).unwrap();
        let mut scratch = vec![0u8; 2048];
        v.mkdir("loop", &mut scratch).unwrap();
        // Point the folder's stream at the root cluster, chained, as long
        // as the volume.
        let root = v.root();
        let (_, at) = v.find_set(root, "loop").unwrap();
        let mut set = at.set;
        let size = u64::from(v.clusters) * v.cluster_bytes() as u64;
        set_stream(&mut set[..at.count], v.root_cluster, size, false);
        let o = at.lba as usize * SECTOR + at.slot * ENTRY;
        v.dev.data[o..o + ENTRY].copy_from_slice(&set[0]);
        v.dev.data[o + ENTRY..o + 2 * ENTRY].copy_from_slice(&set[1]);
        // Walks stay bounded: an error or not found, never a hang.
        assert!(v.find("loop/loop/loop/loop/x").is_err());
        let _ = v.put("loop/loop/new.txt", b"x", &mut scratch);
    }

    fn health(v: &mut ExVolume<Mem>) -> crate::Health {
        let mut seen = vec![0u8; (v.clusters as usize + 2).div_ceil(8)];
        v.check(&mut seen, &mut [0u8; 1024]).unwrap()
    }

    #[test]
    fn a_healthy_exfat_volume_checks_clean_after_many_writes() {
        let (img, _, _) = image();
        let mut v = ExVolume::mount(Mem(img), PARTITION_START).unwrap();
        let mut scratch = vec![0u8; 2048];
        let h = health(&mut v);
        assert!(h.clean() && h.lost == 0 && h.files == 3, "{h:?}");
        for i in 0..40 {
            v.put(
                &format!("dir {}/f{i}", i % 2),
                &pattern(700 * (i % 5), i as u8),
                &mut scratch,
            )
            .unwrap();
        }
        v.put("retropane.bin", &pattern(10_000, 1), &mut scratch)
            .unwrap();
        let h = health(&mut v);
        assert!(h.clean() && h.lost == 0, "{h:?}");
        assert_eq!((h.files, h.folders), (43, 2), "{h:?}");
    }

    #[test]
    fn exfat_damage_of_every_kind_is_found() {
        let (img, _, _) = image();
        let mut v = ExVolume::mount(Mem(img.clone()), PARTITION_START).unwrap();
        let root = v.root();
        let (a, at_a) = v.find_set(root, "retropane.bin").unwrap();
        // Free but used: the bitmap forgets one of retropane.bin's clusters.
        let mut w = ExVolume::mount(Mem(img.clone()), PARTITION_START).unwrap();
        w.mark(a.first_cluster + 1, 1, false).unwrap();
        let h = health(&mut w);
        assert!(h.free_but_used == 1 && !h.clean(), "{h:?}");
        // Lost: the bitmap holds a cluster nothing uses.
        let mut w = ExVolume::mount(Mem(img.clone()), PARTITION_START).unwrap();
        let (Some(free), _) = w.scan(2, 1, &mut vec![0u8; 512]).unwrap() else {
            panic!("no free cluster")
        };
        w.mark(free, 1, true).unwrap();
        let h = health(&mut w);
        assert!(h.clean() && h.lost == 1, "{h:?}");
        // Cross-link: the empty file now claims retropane.bin's clusters.
        let mut w = ExVolume::mount(Mem(img.clone()), PARTITION_START).unwrap();
        let (_, at_e) = w.find_set(root, "empty.txt").unwrap();
        let mut set = at_e.set;
        set_stream(&mut set[..at_e.count], a.first_cluster, 4096, true);
        let o = at_e.lba as usize * SECTOR + at_e.slot * ENTRY;
        w.dev.data[o..o + ENTRY].copy_from_slice(&set[0]);
        w.dev.data[o + ENTRY..o + 2 * ENTRY].copy_from_slice(&set[1]);
        let h = health(&mut w);
        assert!(h.cross_linked >= 1, "{h:?}");
        // A set whose checksum fails.
        let mut w = ExVolume::mount(Mem(img.clone()), PARTITION_START).unwrap();
        let o = at_a.lba as usize * SECTOR + at_a.slot * ENTRY;
        w.dev.data[o + 2 * ENTRY + 4] ^= 1; // a name character
        let h = health(&mut w);
        assert!(h.bad_sets == 1 && !h.clean(), "{h:?}");
        // No room for the marks: refused, not overrun.
        assert_eq!(v.check(&mut [0u8; 1], &mut [0u8; 1024]), Err(Error::NoRoom));
    }

    #[test]
    fn exfat_power_cuts_leak_space_at_worst_never_damage() {
        let (img, _, _) = image();
        let mut scratch = vec![0u8; 2048];
        let new = pattern(9000, 6);
        let total = {
            let mut v = ExVolume::mount(Mem(img.clone()), PARTITION_START).unwrap();
            v.put("x/y.bin", &new, &mut scratch).unwrap();
            v.put("retropane.bin", &new, &mut scratch).unwrap();
            v.dev.writes
        };
        for fail_at in 0..total {
            let mut disk = Mem(img.clone());
            disk.fail_at = Some(fail_at);
            let mut v = ExVolume::mount(disk, PARTITION_START).unwrap();
            let _ = v
                .put("x/y.bin", &new, &mut scratch)
                .and_then(|_| v.put("retropane.bin", &new, &mut scratch));
            let mut v = ExVolume::mount(Mem(v.into_inner().data), PARTITION_START).unwrap();
            let h = health(&mut v);
            assert!(h.clean(), "write {fail_at}: {h:?}");
        }
    }

    #[test]
    fn checking_hostile_exfat_cards_never_panics() {
        let (base_img, _, _) = image();
        let base = PARTITION_START as usize * SECTOR;
        let mut x: u64 = 0xC4EC_E0FA;
        let mut rnd = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for _ in 0..1500 {
            let mut img = base_img.clone();
            for _ in 0..1 + rnd() % 6 {
                let at = base + 24 * SECTOR + (rnd() as usize % (img.len() - base - 24 * SECTOR));
                img[at] = rnd() as u8;
            }
            if let Ok(mut v) = ExVolume::mount(Mem(img), PARTITION_START) {
                let mut seen = vec![0u8; (v.clusters as usize + 2).div_ceil(8)];
                let _ = v.check(&mut seen, &mut [0u8; 1024]);
            }
        }
    }

    #[test]
    fn exfat_removal_leaves_a_clean_volume_and_survives_power_cuts() {
        let (img, a, _) = image();
        let mut scratch = vec![0u8; 2048];
        let mut v = ExVolume::mount(Mem(img), PARTITION_START).unwrap();
        v.put("box/inner.txt", &pattern(3000, 3), &mut scratch)
            .unwrap();
        assert_eq!(v.remove("box"), Err(Error::NotEmpty));
        v.remove("box/inner.txt").unwrap();
        v.remove("box").unwrap();
        v.remove("A Fragmented File With A Long Name.dat").unwrap();
        assert_eq!(v.find("box"), Err(Error::NotFound));
        let h = health(&mut v);
        assert!(h.clean() && h.lost == 0 && h.files == 2, "{h:?}");
        assert!(!dirty(&v));
        let base = v.into_inner().data;
        let total = {
            let mut v = ExVolume::mount(Mem(base.clone()), PARTITION_START).unwrap();
            v.remove("retropane.bin").unwrap();
            v.dev.writes
        };
        for fail_at in 0..total {
            let mut disk = Mem(base.clone());
            disk.fail_at = Some(fail_at);
            let mut v = ExVolume::mount(disk, PARTITION_START).unwrap();
            assert!(v.remove("retropane.bin").is_err());
            let mut v = ExVolume::mount(Mem(v.into_inner().data), PARTITION_START).unwrap();
            match v.find("retropane.bin") {
                Ok(_) => assert_eq!(read_all(&mut v, "retropane.bin", 2048), a),
                Err(Error::NotFound) => {}
                Err(e) => panic!("write {fail_at}: {e:?}"),
            }
            let h = health(&mut v);
            assert!(h.clean(), "write {fail_at}: {h:?}");
        }
    }

    /// Writing to a hostile card: errors, never a panic or a loop.
    #[test]
    fn writing_to_hostile_exfat_cards_never_panics() {
        let (base_img, _, _) = image();
        let base = PARTITION_START as usize * SECTOR;
        let mut x: u64 = 0x5EED_2026;
        let mut rnd = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let mut scratch = vec![0u8; 4096];
        for i in 0..1500 {
            let mut img = base_img.clone();
            for _ in 0..1 + rnd() % 6 {
                let region = match i % 3 {
                    0 => base..base + SECTOR,
                    1 => base + 24 * SECTOR..base + 48 * SECTOR,
                    _ => base..img.len(),
                };
                let at = region.start + (rnd() as usize % region.len());
                img[at] = rnd() as u8;
            }
            let mut sum = 0u32;
            for k in 0..11 {
                sum = boot_checksum(sum, k, &img[base + k * SECTOR..base + (k + 1) * SECTOR]);
            }
            for w in img[base + 11 * SECTOR..base + 12 * SECTOR]
                .as_chunks_mut::<4>()
                .0
            {
                w.copy_from_slice(&sum.to_le_bytes());
            }
            let Ok(mut v) = ExVolume::mount(Mem(img), PARTITION_START) else {
                continue;
            };
            let _ = v.put("retropane.bin", &[7; 5000], &mut scratch);
            let _ = v.put("new file.txt", &[8; 3000], &mut scratch);
            let _ = v.put(
                "A Fragmented File With A Long Name.dat/x",
                b"y",
                &mut scratch,
            );
            let _ = v.put("dir/sub/file", &[9; 100], &mut scratch);
        }
    }

    #[test]
    fn mounts_lists_and_reads_contiguous_and_chained_files() {
        let (img, a, b) = image();
        let mut v = ExVolume::mount(Mem(img), PARTITION_START).unwrap();
        assert_eq!(v.cluster_bytes(), 2048);
        let mut names = Vec::new();
        v.for_each_root(|n, f| names.push((String::from_utf8_lossy(n).into_owned(), f.size)))
            .unwrap();
        assert_eq!(
            names,
            [
                ("retropane.bin".to_string(), 9000),
                ("A Fragmented File With A Long Name.dat".to_string(), 5000),
                ("empty.txt".to_string(), 0),
            ]
        );
        for scratch in [512, 2048, 4096, 65536] {
            assert_eq!(read_all(&mut v, "RETROPANE.BIN", scratch), a);
            assert_eq!(
                read_all(&mut v, "a fragmented file with a long name.dat", scratch),
                b
            );
        }
        assert_eq!(read_all(&mut v, "empty.txt", 512), b"");
        assert_eq!(v.find("missing"), Err(Error::NotFound));
    }

    #[test]
    fn bytes_past_the_valid_length_read_as_zeros() {
        let (mut img, a, _) = image();
        // Lower retropane.bin's valid data length to 1000 (and fix the set
        // checksum, as a real writer would).
        let mut v = ExVolume::mount(Mem(img.clone()), PARTITION_START).unwrap();
        let root = v.cluster_lba(v.root_cluster) as usize * SECTOR;
        let set_at = root + 2 * ENTRY;
        img[set_at + ENTRY + 8..set_at + ENTRY + 16].copy_from_slice(&1000u64.to_le_bytes());
        let mut set = [[0u8; ENTRY]; 3];
        for (k, s) in set.iter_mut().enumerate() {
            s.copy_from_slice(&img[set_at + k * ENTRY..set_at + (k + 1) * ENTRY]);
        }
        let sum = set_checksum(&set);
        img[set_at + 2..set_at + 4].copy_from_slice(&sum.to_le_bytes());
        v = ExVolume::mount(Mem(img), PARTITION_START).unwrap();
        let got = read_all(&mut v, "retropane.bin", 1536);
        assert_eq!(&got[..1000], &a[..1000]);
        assert!(got[1000..].iter().all(|&x| x == 0));
        assert_eq!(got.len(), 9000);
    }

    #[test]
    fn a_damaged_entry_set_is_skipped_and_a_damaged_boot_region_refused() {
        let (mut img, _, _) = image();
        let v = ExVolume::mount(Mem(img.clone()), PARTITION_START).unwrap();
        let root = v.cluster_lba(v.root_cluster) as usize * SECTOR;
        img[root + 2 * ENTRY + 2 * ENTRY + 4] ^= 1; // a name character
        let mut v = ExVolume::mount(Mem(img.clone()), PARTITION_START).unwrap();
        assert_eq!(v.find("retropane.bin"), Err(Error::NotFound));
        assert!(v.find("empty.txt").is_ok(), "the others still read");
        let base = PARTITION_START as usize * SECTOR;
        img[base + 3 * SECTOR + 7] ^= 1; // inside the checksummed region
        assert_eq!(
            ExVolume::mount(Mem(img), PARTITION_START).err(),
            Some(Error::NotExFat)
        );
    }

    #[test]
    fn a_fat32_volume_is_not_exfat() {
        let fat32 = crate::format::build(&crate::format::Image {
            total_sectors: 3072,
            sectors_per_cluster: 2,
            files: &[],
            blocked_clusters: &[],
        });
        assert_eq!(
            ExVolume::mount(Mem(fat32), PARTITION_START).err(),
            Some(Error::NotExFat)
        );
    }

    /// Hostile cards: random corruption anywhere in the structures, with the
    /// boot checksum fixed up so the damage gets past mount. Nothing may
    /// panic or loop.
    #[test]
    fn hostile_exfat_cards_never_panic_or_loop() {
        let (base_img, _, _) = image();
        let base = PARTITION_START as usize * SECTOR;
        let mut x: u64 = 0xE0FA_7201;
        let mut rnd = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for i in 0..2000 {
            let mut img = base_img.clone();
            for _ in 0..1 + rnd() % 6 {
                let region = match i % 3 {
                    0 => base..base + SECTOR,
                    1 => base + 24 * SECTOR..base + 40 * SECTOR,
                    _ => base..img.len(),
                };
                let at = region.start + (rnd() as usize % region.len());
                img[at] = rnd() as u8;
            }
            // Re-sign the boot region so mount sees the corrupted fields.
            let mut sum = 0u32;
            for k in 0..11 {
                sum = boot_checksum(sum, k, &img[base + k * SECTOR..base + (k + 1) * SECTOR]);
            }
            for w in img[base + 11 * SECTOR..base + 12 * SECTOR]
                .as_chunks_mut::<4>()
                .0
            {
                w.copy_from_slice(&sum.to_le_bytes());
            }
            let Ok(mut v) = ExVolume::mount(Mem(img), PARTITION_START) else {
                continue;
            };
            let mut scratch = vec![0u8; 4096];
            let _ = v.for_each_root(|_, _| {});
            for name in ["retropane.bin", "A Fragmented File With A Long Name.dat"] {
                if let Ok(f) = v.find(name) {
                    let mut total = 0u64;
                    let _ = v.read_file(&f, &mut scratch, |c| total += c.len() as u64);
                    assert!(total <= f.size);
                }
            }
        }
    }
}
