//! FAT32 on a [`BlockDevice`] (ADR-0013): the SD card as Windows Mobile and
//! every PC format it.
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
//! Not here yet: subdirectories, deleting, timestamps (no clock yet: new
//! entries carry none, which Windows shows as unknown).

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
    /// The name is longer than [`NAME_MAX`], not ASCII, or has characters
    /// FAT doesn't allow.
    BadName,
    /// A file of that name already exists (to create).
    Exists,
    /// The partition isn't exFAT (or its boot region checksum is wrong).
    NotExFat,
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
            Self::Exists => "exists",
            Self::NotExFat => "not exFAT",
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
        // Everything below comes from the card, so it is hostile until
        // checked (docs/security.md): overflow-checked, and the data area,
        // every cluster and the root must lie inside the partition. Then
        // every in-range cluster's sector number fits in a u32, so the code
        // that trusts validated clusters can't overflow.
        let fat_total = fats.checked_mul(fat_sectors);
        let fat_start = base.checked_add(reserved);
        let data_start = fat_start.zip(fat_total).and_then(|(s, t)| s.checked_add(t));
        let end = base.checked_add(total);
        let (Some(fat_total), Some(fat_start), Some(data_start), Some(_)) =
            (fat_total, fat_start, data_start, end)
        else {
            return Err(Error::NotFat32);
        };
        let data_sectors = total.saturating_sub(reserved.saturating_add(fat_total));
        // Volumes under 65,525 clusters are FAT12/16 by the specification's
        // rule of thumb, but test images are tiny: the FAT32 markers above
        // (no fixed root directory, a root cluster, 32-bit FAT size) decide.
        let clusters = data_sectors / sectors_per_cluster;
        // The FAT must have an entry for every cluster.
        let fat_entries = u64::from(fat_sectors) * u64::from(FAT_PER_SECTOR);
        if clusters == 0
            || u64::from(clusters) + 2 > fat_entries
            || !(2..2 + clusters).contains(&root_cluster)
        {
            return Err(Error::NotFat32);
        }
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
        let max = (scratch.len() / SECTOR) as u32;
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
                let s = c / FAT_PER_SECTOR;
                let v = if max == 0 || self.fat_cached == Some(s) {
                    self.fat_get(c)?
                } else {
                    if !(first..first + len).contains(&s) {
                        let n = grow.min(max).min((hi - 1) / FAT_PER_SECTOR - s + 1);
                        self.dev
                            .read_blocks(self.fat_start + s, &mut scratch[..n as usize * SECTOR])?;
                        (first, len) = (s, n);
                        grow = grow.saturating_mul(2);
                    }
                    let i = (s - first) * FAT_PER_SECTOR + c % FAT_PER_SECTOR;
                    le32(scratch, i as usize * 4) & FAT_MASK
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
            let n = short_to_display(&e[..11], e[12], &mut short);
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
            let n = short_to_display(&e[..11], e[12], &mut short);
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

    /// Replaces `name`'s contents, or creates it when it doesn't exist.
    pub fn put(&mut self, name: &str, data: &[u8], scratch: &mut [u8]) -> Result<Written, Error> {
        match self.replace(name, data, scratch) {
            Err(Error::NotFound) => self.create(name, data, scratch),
            r => r,
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
        name: &str,
        data: &[u8],
        scratch: &mut [u8],
    ) -> Result<Written, Error> {
        let n = name.as_bytes();
        check_new_name(n)?;
        match self.find(name) {
            Ok(_) => return Err(Error::Exists),
            Err(Error::NotFound) => {}
            Err(e) => return Err(e),
        }
        // The short name: the 8.3 form itself, or BASIS~N with long-name
        // entries, N the first free number.
        let (short, nt, lfn) = match plain_short(n) {
            Some((s, nt)) => (s, nt, false),
            None => {
                let mut k = 1;
                loop {
                    let s = numbered_short(n, k);
                    if !self.short_exists(&s)? {
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

        let hint = self.fsinfo_hint()?;
        let written = self.store(data, hint, scratch)?;
        let size = u32::try_from(data.len()).map_err(|_| Error::NoRoom)?;
        let e = &mut entries[count];
        e[..11].copy_from_slice(&short);
        e[11] = 0x20;
        e[12] = nt;
        e[20..22].copy_from_slice(&((written.first_cluster >> 16) as u16).to_le_bytes());
        e[26..28].copy_from_slice(&(written.first_cluster as u16).to_le_bytes());
        e[28..32].copy_from_slice(&size.to_le_bytes());
        count += 1;

        let mut grown = 0;
        let slots = match self.free_slots(count, scratch, &mut grown) {
            Ok(s) => s,
            Err(err) => {
                if written.first_cluster != 0 {
                    self.free_chain(written.first_cluster)?;
                }
                return Err(err);
            }
        };
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
        self.fsinfo_adjust(
            -i64::from(written.clusters + grown),
            Some(self.next_hint(&written, hint)),
        )?;
        Ok(written)
    }

    fn next_hint(&self, w: &Written, hint: u32) -> u32 {
        if w.clusters > 0 {
            w.first_cluster + if w.contiguous { w.clusters } else { 0 }
        } else {
            hint
        }
    }

    /// Whether a root entry has the short name `short`.
    fn short_exists(&mut self, short: &[u8; 11]) -> Result<bool, Error> {
        self.walk_root(&mut |_, _, e, _| e[..11] == short[..])
    }

    /// Finds `need` consecutive free slots in the root directory (deleted
    /// or past its end), growing it by a zeroed cluster if it has none;
    /// returns their (sector, index), in order.
    fn free_slots(
        &mut self,
        need: usize,
        scratch: &mut [u8],
        grown: &mut u32,
    ) -> Result<[(u32, u8); LFN_MAX + 1], Error> {
        let mut run = [(0u32, 0u8); LFN_MAX + 1];
        let mut len = 0;
        let mut cluster = self.root_cluster;
        let mut visited = 0u32;
        loop {
            let lba = self.cluster_lba(cluster);
            for s in 0..self.sectors_per_cluster {
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
            visited += 1;
            match self.next_cluster(cluster)? {
                Some(c) if visited <= self.clusters => cluster = c,
                Some(_) => return Err(Error::Corrupt),
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
    fn store(&mut self, data: &[u8], hint: u32, scratch: &mut [u8]) -> Result<Written, Error> {
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
                        let Some(c) = self.find_free_run(from, 1, scratch)? else {
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

#[cfg(test)]
mod tests;
