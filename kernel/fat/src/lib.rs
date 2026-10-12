//! FAT32 on a [`BlockDevice`] (ADR-0013): the SD card as Windows Mobile and
//! every PC format it.
//!
//! Reads: mount a partition, find a file in the root directory by its long
//! or short name, read it in runs of consecutive sectors. Writes: replace a
//! file's contents by **copy-then-swap**: the new contents go into fresh
//! clusters (one contiguous run when there is one), the chain is linked in
//! every FAT copy, then the directory entry is switched to the new chain
//! with a single sector write, and only then are the old clusters freed.
//! Power lost at any point leaves the complete old file or the complete new
//! one. No heap: two sector buffers live in the [`Volume`].
//!
//! Not here yet: creating files, subdirectories, timestamps (no clock).

#![cfg_attr(not(any(test, feature = "std")), no_std)]
#![forbid(unsafe_code)]

use rp_hal_api::{BlockDevice, BlockError};

/// Sector size; the only one FAT32 cards use.
pub const SECTOR: usize = 512;
/// Directory entries per sector.
const ENTRIES: usize = SECTOR / 32;
/// FAT entries per sector.
const FAT_PER_SECTOR: u32 = (SECTOR / 4) as u32;
/// End-of-chain marker written; anything from 0x0FFF_FFF8 reads as one.
const EOC: u32 = 0x0FFF_FFFF;
const EOC_MIN: u32 = 0x0FFF_FFF8;
const FAT_MASK: u32 = 0x0FFF_FFFF;
/// Longest file name (the FAT limit is 255; this is what's stored).
pub const NAME_MAX: usize = 64;
const ATTR_DIRECTORY: u8 = 0x10;
const ATTR_LONG_NAME: u8 = 0x0F;
const FSINFO_UNKNOWN: u32 = 0xFFFF_FFFF;

/// What went wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The device failed.
    Device(BlockError),
    /// The partition isn't FAT32 with 512-byte sectors.
    NotFat32,
    /// No such file in the root directory.
    NotFound,
    /// Not enough free clusters for the new contents.
    NoRoom,
    /// A cluster chain or directory is damaged.
    Corrupt,
    /// The name is longer than [`NAME_MAX`] or not ASCII.
    BadName,
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
            Self::NotFat32 => "not FAT32",
            Self::NotFound => "not found",
            Self::NoRoom => "no room",
            Self::Corrupt => "corrupt",
            Self::BadName => "bad name",
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

/// What [`Volume::walk_root`] calls per entry: sector, index, raw entry,
/// long name; `true` stops the walk.
type Visit<'a> = dyn FnMut(u32, u8, &[u8; 32], &[u8]) -> bool + 'a;

/// A mounted FAT32 partition.
pub struct Volume<D> {
    dev: D,
    sectors_per_cluster: u32,
    fat_start: u32,
    fat_sectors: u32,
    fats: u32,
    data_start: u32,
    root_cluster: u32,
    /// Data clusters, numbered 2..2 + clusters.
    clusters: u32,
    fsinfo_lba: Option<u32>,
    buf: [u8; SECTOR],
    /// Write-back cache of one FAT sector (`fat_cached` = its index).
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

impl<D: BlockDevice> Volume<D> {
    /// Mounts the FAT32 partition whose first sector is `base`.
    pub fn mount(mut dev: D, base: u32) -> Result<Self, Error> {
        let mut buf = [0u8; SECTOR];
        dev.read_blocks(base, &mut buf)?;
        let bytes_per_sector = le16(&buf, 11);
        let sectors_per_cluster = u32::from(buf[13]);
        let reserved = u32::from(le16(&buf, 14));
        let fats = u32::from(buf[16]);
        let fat_sectors = le32(&buf, 36);
        let root_cluster = le32(&buf, 44);
        let total = match le16(&buf, 19) {
            0 => le32(&buf, 32),
            n => u32::from(n),
        };
        let fsinfo = le16(&buf, 48);
        if usize::from(bytes_per_sector) != SECTOR
            || sectors_per_cluster == 0
            || !sectors_per_cluster.is_power_of_two()
            || reserved == 0
            || fats == 0
            || fat_sectors == 0
            || le16(&buf, 17) != 0 // FAT32 has no fixed root directory
            || root_cluster < 2
            || buf[510..512] != [0x55, 0xAA]
        {
            return Err(Error::NotFat32);
        }
        let fat_start = base + reserved;
        let data_start = fat_start + fats * fat_sectors;
        let data_sectors = total.saturating_sub(reserved + fats * fat_sectors);
        // Volumes under 65,525 clusters are FAT12/16 by the specification's
        // rule of thumb, but test images are tiny: the FAT32 markers above
        // (no fixed root directory, a root cluster, 32-bit FAT size) decide.
        let clusters = data_sectors / sectors_per_cluster;
        Ok(Self {
            dev,
            sectors_per_cluster,
            fat_start,
            fat_sectors,
            fats,
            data_start,
            root_cluster,
            clusters,
            fsinfo_lba: (fsinfo != 0 && fsinfo != 0xFFFF).then(|| base + u32::from(fsinfo)),
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

    fn fat_get(&mut self, cluster: u32) -> Result<u32, Error> {
        if !self.valid_cluster(cluster) {
            return Err(Error::Corrupt);
        }
        self.fat_load(cluster / FAT_PER_SECTOR)?;
        let o = (cluster % FAT_PER_SECTOR) as usize * 4;
        Ok(le32(&self.fat_buf, o) & FAT_MASK)
    }

    fn fat_set(&mut self, cluster: u32, value: u32) -> Result<(), Error> {
        if !self.valid_cluster(cluster) {
            return Err(Error::Corrupt);
        }
        self.fat_load(cluster / FAT_PER_SECTOR)?;
        let o = (cluster % FAT_PER_SECTOR) as usize * 4;
        let v = (le32(&self.fat_buf, o) & !FAT_MASK) | (value & FAT_MASK);
        self.fat_buf[o..o + 4].copy_from_slice(&v.to_le_bytes());
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

    /// The first free cluster at or after `from` (wrapping once), or `None`.
    fn find_free(&mut self, from: u32) -> Result<Option<u32>, Error> {
        let start = if self.valid_cluster(from) { from } else { 2 };
        let end = 2 + self.clusters;
        let mut c = start;
        loop {
            if self.fat_get(c)? == 0 {
                return Ok(Some(c));
            }
            c += 1;
            if c == end {
                c = 2;
            }
            if c == start {
                return Ok(None);
            }
        }
    }

    /// The first run of `need` consecutive free clusters at or after `from`
    /// (wrapping once), or `None`.
    fn find_free_run(&mut self, from: u32, need: u32) -> Result<Option<u32>, Error> {
        let end = 2 + self.clusters;
        let mut from = if self.valid_cluster(from) { from } else { 2 };
        for _pass in 0..2 {
            let mut c = from;
            let mut run_start = c;
            let mut run = 0;
            while c < end {
                if self.fat_get(c)? == 0 {
                    if run == 0 {
                        run_start = c;
                    }
                    run += 1;
                    if run == need {
                        return Ok(Some(run_start));
                    }
                } else {
                    run = 0;
                }
                c += 1;
            }
            from = 2;
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
    fn walk_root(&mut self, f: &mut Visit<'_>) -> Result<bool, Error> {
        let mut cluster = self.root_cluster;
        let mut visited = 0u32;
        self.lfn_valid = false;
        self.lfn_len = 0;
        loop {
            let lba = self.cluster_lba(cluster);
            for s in 0..self.sectors_per_cluster {
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
            visited += 1;
            match self.next_cluster(cluster)? {
                Some(c) if visited <= self.clusters => cluster = c,
                Some(_) => return Err(Error::Corrupt),
                None => return Ok(false),
            }
        }
    }

    /// Calls `f` for every file and subdirectory in the root directory
    /// (volume labels and deleted entries skipped).
    pub fn for_each_root(&mut self, mut f: impl FnMut(&DirEntry<'_>)) -> Result<(), Error> {
        self.walk_root(&mut |_, _, e, lfn| {
            if e[11] & 0x08 != 0 {
                return false; // volume label
            }
            let mut short = [0u8; 12];
            let n = short_to_display(&e[..11], &mut short);
            let name = if lfn.is_empty() { &short[..n] } else { lfn };
            f(&DirEntry {
                name,
                size: le32(e, 28),
                first_cluster: (u32::from(le16(e, 20)) << 16) | u32::from(le16(e, 26)),
                is_dir: e[11] & ATTR_DIRECTORY != 0,
            });
            false
        })?;
        Ok(())
    }

    /// Finds a file in the root directory by name (ASCII, case-insensitive;
    /// long or short form).
    pub fn find(&mut self, name: &str) -> Result<File, Error> {
        let want = name.as_bytes();
        check_name(want)?;
        let mut found = None;
        self.walk_root(&mut |lba, index, e, lfn| {
            if e[11] & (0x08 | ATTR_DIRECTORY) != 0 {
                return false;
            }
            let mut short = [0u8; 12];
            let n = short_to_display(&e[..11], &mut short);
            let hit = (!lfn.is_empty() && lfn.eq_ignore_ascii_case(want))
                || short[..n].eq_ignore_ascii_case(want);
            if hit {
                found = Some(File {
                    entry_lba: lba,
                    entry_index: index,
                    first_cluster: (u32::from(le16(e, 20)) << 16) | u32::from(le16(e, 26)),
                    size: le32(e, 28),
                });
            }
            hit
        })?;
        found.ok_or(Error::NotFound)
    }

    // --- Reading -----------------------------------------------------------

    /// Reads `file` in order, calling `f` with each piece; `scratch` (whole
    /// sectors, at least one) bounds how much is read per device command,
    /// so a larger scratch means fewer, longer transfers.
    ///
    /// # Panics
    /// If `scratch` is shorter than a sector.
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

    /// Writes `data` to `cluster`, padding the last sector with zeros; `data`
    /// is at most one cluster.
    fn write_cluster(&mut self, cluster: u32, data: &[u8]) -> Result<(), Error> {
        let lba = self.cluster_lba(cluster);
        let whole = data.len() / SECTOR * SECTOR;
        if whole > 0 {
            self.dev.write_blocks(lba, &data[..whole])?;
        }
        if whole < data.len() {
            self.buf = [0; SECTOR];
            self.buf[..data.len() - whole].copy_from_slice(&data[whole..]);
            self.dev
                .write_blocks(lba + (whole / SECTOR) as u32, &self.buf)?;
        }
        Ok(())
    }

    /// Replaces the contents of the root-directory file `name` with `data`
    /// by copy-then-swap (see the crate docs). The old contents stay on the
    /// card, untouched, until the new ones are complete and linked.
    pub fn replace(&mut self, name: &str, data: &[u8]) -> Result<Written, Error> {
        let file = self.find(name)?;
        let cb = self.cluster_bytes();
        let need = u32::try_from(data.len().div_ceil(cb)).map_err(|_| Error::NoRoom)?;
        let hint = self.fsinfo_hint()?;

        let mut written = Written {
            first_cluster: 0,
            clusters: need,
            contiguous: true,
        };
        if need > 0 {
            match self.find_free_run(hint, need)? {
                Some(start) => {
                    // One run: write it as few long transfers as the data
                    // allows (whole clusters in one go), then link it.
                    written.first_cluster = start;
                    let whole = data.len() / cb * cb;
                    if whole > 0 {
                        self.dev
                            .write_blocks(self.cluster_lba(start), &data[..whole])?;
                    }
                    if whole < data.len() {
                        self.write_cluster(start + need - 1, &data[whole..])?;
                    }
                    for c in start..start + need {
                        self.fat_set(c, if c + 1 == start + need { EOC } else { c + 1 })?;
                    }
                }
                None => {
                    written.contiguous = false;
                    let mut prev = None;
                    let mut from = hint;
                    for chunk in data.chunks(cb) {
                        let Some(c) = self.find_free(from)? else {
                            // Out of space part way: give back what was
                            // taken (the old file is still whole).
                            if written.first_cluster != 0 {
                                self.free_chain(written.first_cluster)?;
                            }
                            return Err(Error::NoRoom);
                        };
                        self.write_cluster(c, chunk)?;
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

        let next_hint = if need > 0 {
            written.first_cluster + if written.contiguous { need } else { 0 }
        } else {
            hint
        };
        self.fsinfo_adjust(-i64::from(need), Some(next_hint))?;
        if file.first_cluster != 0 {
            self.free_chain(file.first_cluster)?;
        }
        Ok(written)
    }
}

/// `NAME.EXT` from an 8.3 entry name; returns the length written to `out`.
fn short_to_display(short: &[u8], out: &mut [u8; 12]) -> usize {
    let mut n = 0;
    for &b in short[..8].iter().take_while(|&&b| b != b' ') {
        out[n] = if b == 0x05 { 0xE5 } else { b };
        n += 1;
    }
    let ext: &[u8] = &short[8..11];
    let ext_len = ext.iter().take_while(|&&b| b != b' ').count();
    if ext_len > 0 {
        out[n] = b'.';
        n += 1;
        out[n..n + ext_len].copy_from_slice(&ext[..ext_len]);
        n += ext_len;
    }
    n
}

#[cfg(any(test, feature = "std"))]
pub mod format;

#[cfg(test)]
mod tests;
