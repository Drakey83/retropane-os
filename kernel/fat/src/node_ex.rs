//! The node layer (ADR-0037, [`crate::node`]) on exFAT: entry sets read and
//! written whole, names in UTF-16 up to 255 units, the stream extension's
//! name hash over the name up-cased by **the card's own up-case table**
//! (read once per name, in one pass), timestamps with their 10 ms counts
//! and time zones.
//!
//! An exFAT location also says how its folder's clusters follow (bit 63:
//! one run, "no FAT chain"), so a set that crosses a cluster edge reads
//! without knowing the folder.

use rp_hal_api::BlockDevice;

use crate::exfat::{ExVolume, set_checksum};
use crate::node::{NAME_UNITS, Node, ROOT, Reservation, Stamp, same_name, valid_name};
use crate::{Error, SECTOR};

const ENTRY: usize = 32;
const SLOTS: usize = SECTOR / ENTRY;
const T_FILE: u8 = 0x85;
const T_STREAM: u8 = 0xC0;
const T_NAME: u8 = 0xC1;
const T_UPCASE: u8 = 0x82;
const ATTR_DIRECTORY: u16 = 0x10;
const ATTR_ARCHIVE: u16 = 0x20;
const ALLOC_POSSIBLE: u8 = 0x01;
const NO_FAT_CHAIN: u8 = 0x02;
/// Entries in the longest set: file, stream, 17 names (255 units).
const SET_MAX: usize = 2 + NAME_UNITS.div_ceil(15);
/// The location bit saying the folder is one run of clusters.
const CONTIGUOUS_DIR: u64 = 1 << 63;
/// The temporary entries' names: `~RPMTP` and two digits, `.TMP`.
const TEMP_PREFIX: &str = "~RPMTP";

fn le16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn le32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
fn le64(b: &[u8], o: usize) -> u64 {
    u64::from(le32(b, o)) | u64::from(le32(b, o + 4)) << 32
}

fn pos(lba: u32, slot: usize) -> u64 {
    u64::from(lba) << 4 | slot as u64
}

fn split(loc: u64) -> (u32, usize) {
    (((loc & !CONTIGUOUS_DIR) >> 4) as u32, (loc & 15) as usize)
}

/// A folder: first cluster, one run or chained, clusters, and its own set's
/// place (for growing) when the folder can grow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NodeDir {
    pub(crate) first: u32,
    pub(crate) contiguous: bool,
    pub(crate) len: u32,
    pub(crate) entry: Option<(u32, usize)>,
    /// The folder's own location ([`ROOT`] for the root).
    pub(crate) loc: u64,
}

/// An exFAT timestamp (with its 10 ms count and UTC offset byte).
fn stamp_from(ts: u32, tens: u8, utc: u8) -> Option<Stamp> {
    if ts == 0 {
        return None;
    }
    let tens = (tens < 200).then_some(tens);
    let s = Stamp {
        year: 1980 + (ts >> 25) as u16,
        month: ((ts >> 21) & 15) as u8,
        day: ((ts >> 16) & 31) as u8,
        hour: ((ts >> 11) & 31) as u8,
        minute: ((ts >> 5) & 63) as u8,
        second: ((ts & 31) * 2) as u8 + tens.map_or(0, |t| t / 100),
        centis: tens.map(|t| t % 100),
        zone: (utc & 0x80 != 0).then(|| {
            // Seven-bit two's complement, 15-minute units.
            let q = i16::from(((utc << 1) as i8) >> 1);
            q * 15
        }),
    };
    (s.month >= 1 && s.month <= 12 && s.day >= 1 && s.hour < 24 && s.minute < 60 && s.second < 60)
        .then_some(s)
}

fn stamp_to(s: Stamp) -> Option<(u32, u8, u8)> {
    if !(1980..=2107).contains(&s.year) || s.month == 0 || s.day == 0 {
        return None;
    }
    let ts = u32::from(s.year - 1980) << 25
        | u32::from(s.month) << 21
        | u32::from(s.day) << 16
        | u32::from(s.hour) << 11
        | u32::from(s.minute) << 5
        | u32::from(s.second / 2);
    let tens = (s.second % 2) * 100 + s.centis.unwrap_or(0);
    let utc = match s.zone {
        Some(z) if z % 15 == 0 && (-64 * 15..=63 * 15).contains(&z) => {
            0x80 | ((z / 15) as i8 as u8 & 0x7F)
        }
        _ => 0,
    };
    Some((ts, tens, utc))
}

/// The stream extension's name hash over up-cased units, low byte first.
fn hash16(up: &[u16]) -> u16 {
    let mut h = 0u16;
    for &c in up {
        for b in c.to_le_bytes() {
            h = h.rotate_right(1).wrapping_add(u16::from(b));
        }
    }
    h
}

fn is_temp(name: &[u16]) -> bool {
    let p: Vec16 = Vec16::from(TEMP_PREFIX);
    name.len() == 12
        && name[..6] == p.units[..6]
        && name[6..8].iter().all(|&u| (0x30..=0x39).contains(&u))
        && name[8..] == [0x2E, 0x54, 0x4D, 0x50]
}

/// A small fixed UTF-16 buffer (no heap).
struct Vec16 {
    units: [u16; 16],
}

impl Vec16 {
    fn from(s: &str) -> Self {
        let mut v = Self { units: [0; 16] };
        for (d, c) in v.units.iter_mut().zip(s.encode_utf16()) {
            *d = c;
        }
        v
    }
}

/// A whole entry set as read.
struct Set {
    e: [[u8; ENTRY]; SET_MAX],
    count: usize,
}

impl<D: BlockDevice> ExVolume<D> {
    /// The sector after `lba` in a folder whose clusters are one run
    /// (`contiguous`, within `bound` clusters from `first` when known) or
    /// chained.
    fn step(
        &mut self,
        lba: u32,
        contiguous: bool,
        dir: Option<&NodeDir>,
    ) -> Result<Option<u32>, Error> {
        let rel = lba.checked_sub(self.heap_start).ok_or(Error::Corrupt)?;
        let spc = 1u32 << self.cluster_shift;
        if (rel + 1) % spc != 0 {
            return Ok(Some(lba + 1));
        }
        let c = rel / spc + 2;
        let next = if contiguous {
            let last = dir.map_or(u32::MAX, |d| d.first.saturating_add(d.len));
            (c + 1 < last && self.valid_cluster(c + 1)).then_some(c + 1)
        } else {
            self.fat_next(c)?
        };
        Ok(next.map(|n| self.cluster_lba(n)))
    }

    fn in_heap(&self, lba: u32) -> bool {
        lba >= self.heap_start && (lba - self.heap_start) >> self.cluster_shift < self.clusters
    }

    /// Reads the set whose file entry is at `loc`, checksum checked.
    fn read_set16(&mut self, loc: u64, set: &mut Set) -> Result<u64, Error> {
        let contiguous = loc & CONTIGUOUS_DIR != 0;
        let (mut lba, mut i) = split(loc);
        if !self.in_heap(lba) {
            return Err(Error::NotFound);
        }
        self.dev.read_blocks(lba, &mut self.buf)?;
        let mut k = 0;
        let mut want = 1;
        while k < want {
            if i == SLOTS {
                lba = self.step(lba, contiguous, None)?.ok_or(Error::NotFound)?;
                self.dev.read_blocks(lba, &mut self.buf)?;
                i = 0;
            }
            set.e[k].copy_from_slice(&self.buf[i * ENTRY..(i + 1) * ENTRY]);
            if k == 0 {
                let sec = usize::from(set.e[0][1]);
                if set.e[0][0] != T_FILE || !(2..SET_MAX).contains(&sec) {
                    return Err(Error::NotFound);
                }
                want = 1 + sec;
            } else if set.e[k][0] & 0x80 == 0 || set.e[k][0] & 0x40 == 0 {
                // A secondary entry in use (type bits: in use, secondary).
                return Err(Error::NotFound);
            }
            k += 1;
            i += 1;
        }
        set.count = want;
        if set_checksum(&set.e[..want]) != le16(&set.e[0], 2) || set.e[1][0] != T_STREAM {
            return Err(Error::NotFound);
        }
        Ok(loc)
    }

    fn node_of(&self, set: &Set, loc: u64, out: &mut Node) -> Result<(), Error> {
        let f = &set.e[0];
        let st = &set.e[1];
        let len = usize::from(st[3]);
        let mut n = 0;
        for e in set.e[2..set.count].iter().filter(|e| e[0] == T_NAME) {
            for k in 0..15 {
                if n < len {
                    out.name[n] = le16(e, 2 + 2 * k);
                    n += 1;
                }
            }
        }
        if n == 0 || n != len {
            return Err(Error::NotFound);
        }
        out.name_len = n;
        out.loc = loc;
        out.entry = loc & !CONTIGUOUS_DIR;
        let attrs = le16(f, 4);
        out.folder = attrs & ATTR_DIRECTORY != 0;
        out.attrs = attrs as u8;
        out.size = if out.folder { 0 } else { le64(st, 24) };
        out.first_cluster = le32(st, 20);
        out.created = stamp_from(le32(f, 8), f[20], f[22]);
        out.modified = stamp_from(le32(f, 12), f[21], f[23]);
        Ok(())
    }

    /// The object at `loc`.
    ///
    /// # Errors
    /// [`Error::NotFound`] when `loc` isn't a whole, checksummed set.
    pub fn node_stat(&mut self, loc: u64, out: &mut Node) -> Result<(), Error> {
        if loc == ROOT {
            return Err(Error::NotFound);
        }
        let mut set = Set {
            e: [[0; ENTRY]; SET_MAX],
            count: 0,
        };
        self.read_set16(loc, &mut set)?;
        self.node_of(&set, loc, out)
    }

    /// Folder `dir` ([`ROOT`] or a folder's location).
    fn node_dir(&mut self, dir: u64) -> Result<NodeDir, Error> {
        if dir == ROOT {
            return Ok(NodeDir {
                first: self.root_cluster,
                contiguous: false,
                len: self.clusters,
                entry: None,
                loc: ROOT,
            });
        }
        let mut set = Set {
            e: [[0; ENTRY]; SET_MAX],
            count: 0,
        };
        self.read_set16(dir, &mut set)?;
        if le16(&set.e[0], 4) & ATTR_DIRECTORY == 0 {
            return Err(Error::NotFound);
        }
        let st = &set.e[1];
        let first = le32(st, 20);
        let size = le64(st, 24);
        let cb = self.cluster_bytes() as u64;
        if !self.valid_cluster(first)
            || size == 0
            || !size.is_multiple_of(cb)
            || size / cb > u64::from(self.clusters)
        {
            return Err(Error::Corrupt);
        }
        let (lba, slot) = split(dir);
        // Growable when the folder's file and stream entries share a sector
        // (the one sector a grow rewrites: `grow16`); a file entry ending its
        // sector can't grow (quality audit C85).
        let entry = (slot + 1 < SLOTS).then_some((lba, slot));
        Ok(NodeDir {
            first,
            contiguous: st[1] & NO_FAT_CHAIN != 0,
            len: (size / cb) as u32,
            entry,
            loc: dir,
        })
    }

    fn dir_for_exfat(d: &NodeDir) -> crate::exfat::Dir {
        crate::exfat::Dir::new(d.first, d.contiguous, d.len, d.entry)
    }

    /// Grows folder `d` by one zeroed cluster, as crate::exfat's grower does
    /// (the cluster marked used before anything links it; one run kept when
    /// the next cluster is free, else the run chained first), then one
    /// sector write of the folder's file and stream entries makes it part
    /// of the folder. Its name entries may lie in later sectors (a long
    /// name): they don't change, and are read only for the checksum.
    fn grow16(&mut self, d: &mut NodeDir, scratch: &mut [u8]) -> Result<(), Error> {
        let root = d.loc == ROOT;
        if !root && d.entry.is_none() {
            return Err(Error::NoRoom);
        }
        let dir = Self::dir_for_exfat(d);
        let mut last = d.first;
        if d.contiguous {
            last = d.first + d.len - 1;
        } else {
            for k in 0.. {
                match self.dir_next(dir, last, k)? {
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
        if root {
            self.fat_set(new, 0xFFFF_FFFF)?;
            self.fat_set(last, new)?;
            return Ok(());
        }
        let stays = d.contiguous && new == last + 1;
        if !stays {
            if d.contiguous {
                for c in d.first..last {
                    self.fat_set(c, c + 1)?;
                }
            }
            self.fat_set(new, 0xFFFF_FFFF)?;
            self.fat_set(last, new)?;
        }
        let mut set = Set {
            e: [[0; ENTRY]; SET_MAX],
            count: 0,
        };
        self.read_set16(d.loc, &mut set)?;
        let size = u64::from(d.len + 1) * self.cluster_bytes() as u64;
        let st = &mut set.e[1];
        st[1] = ALLOC_POSSIBLE | if stays { NO_FAT_CHAIN } else { 0 };
        st[8..16].copy_from_slice(&size.to_le_bytes());
        st[24..32].copy_from_slice(&size.to_le_bytes());
        let sum = set_checksum(&set.e[..set.count]);
        set.e[0][2..4].copy_from_slice(&sum.to_le_bytes());
        let (at, slot) = d.entry.ok_or(Error::NoRoom)?;
        self.dev.read_blocks(at, &mut self.buf)?;
        if self.buf[slot * ENTRY] != T_FILE {
            return Err(Error::Corrupt);
        }
        self.buf[slot * ENTRY..(slot + 2) * ENTRY].copy_from_slice(set.e[..2].as_flattened());
        self.dev.write_blocks(at, &self.buf)?;
        d.len += 1;
        d.contiguous = stays;
        Ok(())
    }

    /// The child after `cursor` in folder `dir`: `Ok(false)` at the end.
    /// Unused entries, the bitmap, the up-case table, the label and the
    /// temporary entries of writes in progress are skipped.
    ///
    /// # Errors
    /// The folder is gone or damaged, or the card failed.
    pub fn node_child(
        &mut self,
        dir: u64,
        cursor: &mut u64,
        out: &mut Node,
    ) -> Result<bool, Error> {
        let d = self.node_dir(dir)?;
        let flag = if d.contiguous { CONTIGUOUS_DIR } else { 0 };
        let (mut lba, mut i, mut seen) = if *cursor == 0 {
            (self.cluster_lba(d.first), 0usize, 0u32)
        } else {
            let (l, s) = split(*cursor & ((1 << 36) - 1));
            (l, s, ((*cursor >> 36) & 0x07FF_FFFF) as u32)
        };
        if lba == 0 || !self.in_heap(lba) {
            return Ok(false);
        }
        let mut set = Set {
            e: [[0; ENTRY]; SET_MAX],
            count: 0,
        };
        self.dev.read_blocks(lba, &mut self.buf)?;
        loop {
            if i == SLOTS {
                match self.step(lba, d.contiguous, Some(&d))? {
                    Some(n) => {
                        lba = n;
                        self.dev.read_blocks(lba, &mut self.buf)?;
                        i = 0;
                    }
                    None => {
                        *cursor = u64::from(seen.max(1)) << 36;
                        return Ok(false);
                    }
                }
            }
            seen += 1;
            if seen > 256 * 1024 {
                return Err(Error::Corrupt);
            }
            let t = self.buf[i * ENTRY];
            let here = pos(lba, i);
            i += 1;
            if t == 0 {
                *cursor = u64::from(seen) << 36;
                return Ok(false);
            }
            if t != T_FILE {
                continue;
            }
            // A set: read it whole (it may run on), then come back after it.
            let loc = here | flag;
            let ok = self
                .read_set16(loc, &mut set)
                .and_then(|_| self.node_of(&set, loc, out));
            // Where to resume: after the set's last entry.
            let mut after_lba = lba;
            let mut after_i = i - 1;
            let count = if ok.is_ok() { set.count } else { 1 };
            for _ in 0..count {
                after_i += 1;
                if after_i == SLOTS {
                    match self.step(after_lba, d.contiguous, Some(&d))? {
                        Some(n) => {
                            after_lba = n;
                            after_i = 0;
                        }
                        None => {
                            after_lba = 0;
                            break;
                        }
                    }
                }
            }
            if ok.is_err() || is_temp(out.name()) {
                // Skip a bad or temporary set; carry on from after it.
                if after_lba == 0 {
                    *cursor = u64::from(seen) << 36;
                    return Ok(false);
                }
                lba = after_lba;
                i = after_i;
                self.dev.read_blocks(lba, &mut self.buf)?;
                continue;
            }
            *cursor = if after_lba == 0 {
                u64::from(seen) << 36
            } else {
                u64::from(seen) << 36 | pos(after_lba, after_i)
            };
            return Ok(true);
        }
    }

    /// The up-case table: first cluster and length in bytes.
    fn upcase_table(&mut self) -> Result<Option<(u32, u64)>, Error> {
        let root = self.node_dir(ROOT)?;
        let mut found = None;
        self.walk_slots(Self::dir_for_exfat(&root), &mut |_, _, _, e| {
            let hit = e[0] == T_UPCASE;
            if hit {
                found = Some((le32(e, 20), le64(e, 24)));
            }
            hit
        })?;
        Ok(found.filter(|&(c, len)| self.valid_cluster(c) && (2..=128 * 1024 + 2).contains(&len)))
    }

    /// `name` up-cased with the card's table, into `up` (ASCII-only names
    /// without reading it; a card with no readable table: single-character
    /// Unicode mappings).
    fn upcase16(&mut self, name: &[u16], up: &mut [u16; NAME_UNITS]) -> Result<(), Error> {
        for (u, &c) in up.iter_mut().zip(name) {
            *u = if c < 0x80 {
                u16::from((c as u8).to_ascii_uppercase())
            } else {
                c
            };
        }
        if name.iter().all(|&c| c < 0x80) {
            return Ok(());
        }
        let Some((first, len)) = self.upcase_table()? else {
            for (u, &c) in up.iter_mut().zip(name) {
                *u = crate::node::upcase(c);
            }
            return Ok(());
        };
        // One pass over the (compressed) table: each unit's mapping is
        // picked up as its index goes by.
        let mut idx: u32 = 0;
        let mut want_count = false;
        let mut done = 0u64;
        let mut cluster = first;
        let cb = self.cluster_bytes() as u64;
        let mut sector = [0u8; SECTOR];
        while done < len && idx <= 0xFFFF {
            let in_cluster = done % cb;
            if done > 0 && in_cluster == 0 {
                cluster = self.fat_next(cluster)?.unwrap_or(cluster + 1);
                if !self.valid_cluster(cluster) {
                    return Err(Error::Corrupt);
                }
            }
            self.dev.read_blocks(
                self.cluster_lba(cluster) + (in_cluster / SECTOR as u64) as u32,
                &mut sector,
            )?;
            let n = (len - done).min(SECTOR as u64) as usize;
            for w in sector[..n & !1].chunks(2) {
                let v = u16::from_le_bytes([w[0], w[1]]);
                if want_count {
                    want_count = false;
                    let end = idx + u32::from(v);
                    for (u, &c) in up.iter_mut().zip(name) {
                        if (idx..end).contains(&u32::from(c)) {
                            *u = c;
                        }
                    }
                    idx = end;
                    continue;
                }
                if v == 0xFFFF {
                    want_count = true;
                    continue;
                }
                for (u, &c) in up.iter_mut().zip(name) {
                    if u32::from(c) == idx {
                        *u = v;
                    }
                }
                idx += 1;
            }
            done += SECTOR as u64;
        }
        Ok(())
    }

    /// Whether folder `d` holds a set named `name` (up-cased comparison:
    /// hash first, then the names).
    fn name_taken16(&mut self, d: &NodeDir, name: &[u16]) -> Result<bool, Error> {
        let mut up = [0u16; NAME_UNITS];
        self.upcase16(name, &mut up)?;
        let want_hash = hash16(&up[..name.len()]);
        let mut cursor = 0u64;
        let mut n = Node::default();
        let dir_loc = self.dir_loc_of(d);
        loop {
            let more = match dir_loc {
                Some(l) => self.node_child(l, &mut cursor, &mut n)?,
                None => false,
            };
            if !more {
                return Ok(false);
            }
            if n.name_len != name.len() {
                continue;
            }
            // The stored hash is the card's; compare it, then the names.
            let mut set = Set {
                e: [[0; ENTRY]; SET_MAX],
                count: 0,
            };
            self.read_set16(n.loc, &mut set)?;
            if le16(&set.e[1], 4) != want_hash && !same_name(n.name(), name) {
                continue;
            }
            let mut other = [0u16; NAME_UNITS];
            let nm = n.name;
            self.upcase16(&nm[..n.name_len], &mut other)?;
            if other[..n.name_len] == up[..name.len()] {
                return Ok(true);
            }
        }
    }

    /// The location naming folder `d` for walks: kept from when it was
    /// opened.
    fn dir_loc_of(&self, d: &NodeDir) -> Option<u64> {
        if d.first == self.root_cluster && d.entry.is_none() && !d.contiguous {
            return Some(ROOT);
        }
        self.node_dir_loc.filter(|(_, nd)| nd == d).map(|(l, _)| l)
    }

    /// Opens folder `dir`, remembering its location for walks.
    fn open16(&mut self, dir: u64) -> Result<NodeDir, Error> {
        let d = self.node_dir(dir)?;
        if dir != ROOT {
            self.node_dir_loc = Some((dir, d));
        }
        Ok(d)
    }

    /// A set for `name`, checksummed.
    #[allow(clippy::too_many_arguments)]
    fn build16(
        &mut self,
        name: &[u16],
        attrs: u16,
        first: u32,
        size: u64,
        contiguous: bool,
        created: Option<Stamp>,
        modified: Option<Stamp>,
        set: &mut Set,
    ) -> Result<(), Error> {
        let mut up = [0u16; NAME_UNITS];
        self.upcase16(name, &mut up)?;
        let names = name.len().div_ceil(15);
        set.count = 2 + names;
        for e in &mut set.e[..set.count] {
            *e = [0; ENTRY];
        }
        let f = &mut set.e[0];
        f[0] = T_FILE;
        f[1] = (1 + names) as u8;
        f[4..6].copy_from_slice(&attrs.to_le_bytes());
        if let Some((ts, tens, utc)) = created.and_then(stamp_to) {
            f[8..12].copy_from_slice(&ts.to_le_bytes());
            f[16..20].copy_from_slice(&ts.to_le_bytes());
            f[20] = tens;
            f[22] = utc;
            f[24] = utc;
        }
        if let Some((ts, tens, utc)) = modified.or(created).and_then(stamp_to) {
            f[12..16].copy_from_slice(&ts.to_le_bytes());
            f[21] = tens;
            f[23] = utc;
        }
        let st = &mut set.e[1];
        st[0] = T_STREAM;
        st[1] = ALLOC_POSSIBLE
            | if contiguous && size > 0 {
                NO_FAT_CHAIN
            } else {
                0
            };
        st[3] = name.len() as u8;
        st[4..6].copy_from_slice(&hash16(&up[..name.len()]).to_le_bytes());
        st[8..16].copy_from_slice(&size.to_le_bytes());
        st[20..24].copy_from_slice(&first.to_le_bytes());
        st[24..32].copy_from_slice(&size.to_le_bytes());
        for (k, chunk) in name.chunks(15).enumerate() {
            let e = &mut set.e[2 + k];
            e[0] = T_NAME;
            for (i, &c) in chunk.iter().enumerate() {
                e[2 + 2 * i..4 + 2 * i].copy_from_slice(&c.to_le_bytes());
            }
        }
        let sum = set_checksum(&set.e[..set.count]);
        set.e[0][2..4].copy_from_slice(&sum.to_le_bytes());
        Ok(())
    }

    /// Writes `set` into folder `d` (crate::exfat's placement: deleted
    /// entries in one sector, or the end, the file entry's sector last);
    /// returns its location.
    fn place16(&mut self, d: &NodeDir, set: &Set, scratch: &mut [u8]) -> Result<u64, Error> {
        let mut d = *d;
        for _ in 0..2 {
            match self.place_set(Self::dir_for_exfat(&d), &set.e[..set.count]) {
                Err(Error::NoRoom) => {
                    self.grow16(&mut d, scratch)?;
                    if self.node_dir_loc.is_some_and(|(l, _)| l == d.loc) {
                        self.node_dir_loc = Some((d.loc, d));
                    }
                }
                r => {
                    // A folder grown out of its run: its children's locations
                    // say "chained" from now on.
                    let flag = if d.contiguous { CONTIGUOUS_DIR } else { 0 };
                    return r.map(|at| at | flag);
                }
            }
        }
        Err(Error::NoRoom)
    }

    /// Marks the set at `loc` unused, its file entry's sector first.
    fn delete16(&mut self, loc: u64) -> Result<(), Error> {
        let mut set = Set {
            e: [[0; ENTRY]; SET_MAX],
            count: 0,
        };
        self.read_set16(loc, &mut set)?;
        let contiguous = loc & CONTIGUOUS_DIR != 0;
        let (mut lba, mut i) = split(loc);
        self.dev.read_blocks(lba, &mut self.buf)?;
        for _ in 0..set.count {
            if i == SLOTS {
                self.dev.write_blocks(lba, &self.buf)?;
                lba = self.step(lba, contiguous, None)?.ok_or(Error::Corrupt)?;
                self.dev.read_blocks(lba, &mut self.buf)?;
                i = 0;
            }
            self.buf[i * ENTRY] &= 0x7F;
            i += 1;
        }
        self.dev.write_blocks(lba, &self.buf)?;
        Ok(())
    }

    /// Creates a folder or an empty file.
    ///
    /// # Errors
    /// [`Error::BadName`], [`Error::Exists`], [`Error::NoRoom`], or the
    /// folder is gone.
    pub fn node_create(
        &mut self,
        dir: u64,
        name: &[u16],
        folder: bool,
        hidden: bool,
        stamps: (Option<Stamp>, Option<Stamp>),
        scratch: &mut [u8],
    ) -> Result<u64, Error> {
        if !valid_name(name) {
            return Err(Error::BadName);
        }
        let d = self.open16(dir)?;
        if self.name_taken16(&d, name)? {
            return Err(Error::Exists);
        }
        let hid = if hidden { 0x02 } else { 0 };
        let mut set = Set {
            e: [[0; ENTRY]; SET_MAX],
            count: 0,
        };
        self.set_dirty(true)?;
        let r = if folder {
            let (Some(c), _) = self.scan(2, 1, scratch)? else {
                return Err(Error::NoRoom);
            };
            let zero = &mut scratch[..SECTOR];
            zero.fill(0);
            let lba = self.cluster_lba(c);
            for s in 0..1u32 << self.cluster_shift {
                self.dev.write_blocks(lba + s, zero)?;
            }
            self.mark(c, 1, true)?;
            let cb = self.cluster_bytes() as u64;
            self.build16(
                name,
                ATTR_DIRECTORY | hid,
                c,
                cb,
                true,
                stamps.0,
                stamps.1,
                &mut set,
            )?;
            let r = self.place16(&d, &set, scratch);
            if r.is_err() {
                self.mark(c, 1, false)?;
            }
            r
        } else {
            self.build16(
                name,
                ATTR_ARCHIVE | hid,
                0,
                0,
                false,
                stamps.0,
                stamps.1,
                &mut set,
            )?;
            self.place16(&d, &set, scratch)
        };
        self.clean_up()?;
        r
    }

    fn clean_up(&mut self) -> Result<(), Error> {
        if !self.was_dirty {
            self.set_dirty(false)?;
        }
        Ok(())
    }

    /// Reserves `size` bytes for folder `dir`: clusters marked used (one run
    /// when there is one, else chained), a temporary set in the root naming
    /// them (its creation timestamp holds the folder's first cluster, its
    /// access timestamp the folder's length and run flag, for recovery).
    ///
    /// # Errors
    /// [`Error::NoRoom`] (nothing changed), or the folder is gone.
    pub fn node_reserve(
        &mut self,
        dir: u64,
        size: u64,
        scratch: &mut [u8],
    ) -> Result<Reservation, Error> {
        let d = self.open16(dir)?;
        let cb = self.cluster_bytes() as u64;
        let need = u32::try_from(size.div_ceil(cb)).map_err(|_| Error::NoRoom)?;
        if need == 0 {
            return Err(Error::NoRoom);
        }
        let (run, free) = self.scan(2, need, scratch)?;
        if run.is_none() && free < need {
            return Err(Error::NoRoom);
        }
        self.set_dirty(true)?;
        let (first, contiguous) = match run {
            Some(c) => {
                self.mark(c, need, true)?;
                (c, true)
            }
            None => {
                let mut first = 0;
                let mut prev = None;
                let mut from = 2;
                for _ in 0..need {
                    let (Some(c), _) = self.scan(from, 1, scratch)? else {
                        return Err(Error::NoRoom);
                    };
                    self.mark(c, 1, true)?;
                    match prev {
                        Some(p) => self.fat_set(p, c)?,
                        None => first = c,
                    }
                    prev = Some(c);
                    from = c + 1;
                }
                if let Some(p) = prev {
                    self.fat_set(p, 0xFFFF_FFFF)?;
                }
                (first, false)
            }
        };
        // The temporary set in the root.
        let root = self.node_dir(ROOT)?;
        let mut n = 0u8;
        let mut tname = [0u16; 12];
        loop {
            let s = format_temp(n);
            tname.copy_from_slice(&s);
            if !self.name_taken16(&root, &tname)? {
                break;
            }
            n += 1;
            if n >= 100 {
                self.release_run(first, need, contiguous)?;
                return Err(Error::NoRoom);
            }
        }
        let mut set = Set {
            e: [[0; ENTRY]; SET_MAX],
            count: 0,
        };
        self.build16(&tname, 0x26, first, size, contiguous, None, None, &mut set)?;
        set.e[0][8..12].copy_from_slice(&d.first.to_le_bytes());
        set.e[0][16..20].copy_from_slice(&(d.len | u32::from(d.contiguous) << 31).to_le_bytes());
        let sum = set_checksum(&set.e[..set.count]);
        set.e[0][2..4].copy_from_slice(&sum.to_le_bytes());
        let temp = match self.place16(&root, &set, scratch) {
            Ok(t) => t,
            Err(e) => {
                self.release_run(first, need, contiguous)?;
                return Err(e);
            }
        };
        Ok(Reservation::new(
            first, need, size, contiguous, temp, d.first,
        ))
    }

    fn release_run(&mut self, first: u32, n: u32, contiguous: bool) -> Result<(), Error> {
        if contiguous {
            return self.mark(first, n, false);
        }
        let mut c = first;
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

    /// Writes into a reservation at `offset` (whole sectors but at the
    /// end).
    ///
    /// # Errors
    /// Past the reservation, an offset off a sector, or the card failed.
    pub fn node_write(
        &mut self,
        r: &mut Reservation,
        offset: u64,
        data: &[u8],
        scratch: &mut [u8],
    ) -> Result<(), Error> {
        if !offset.is_multiple_of(SECTOR as u64) || offset + data.len() as u64 > r.size {
            return Err(Error::NoRoom);
        }
        let cb = self.cluster_bytes() as u64;
        let mut at = 0usize;
        while at < data.len() {
            let off = offset + at as u64;
            let index = (off / cb) as u32;
            let cluster = if r.contiguous {
                r.first + index
            } else {
                let (mut c, mut k) = r.last_reached(index);
                while k < index {
                    c = self.fat_next(c)?.ok_or(Error::Corrupt)?;
                    k += 1;
                }
                r.reached(c, k);
                c
            };
            let in_cluster = off % cb;
            let n = ((cb - in_cluster) as usize).min(data.len() - at);
            let lba = self.cluster_lba(cluster) + (in_cluster / SECTOR as u64) as u32;
            let whole = n / SECTOR * SECTOR;
            if whole > 0 {
                self.dev.write_blocks(lba, &data[at..at + whole])?;
            }
            if whole < n {
                let pad = &mut scratch[..SECTOR];
                pad.fill(0);
                pad[..n - whole].copy_from_slice(&data[at + whole..at + n]);
                self.dev.write_blocks(lba + (whole / SECTOR) as u32, pad)?;
            }
            at += n;
        }
        Ok(())
    }

    /// Names a complete reservation in its folder (the real set written,
    /// then the temporary one deleted; recovery tells the halves apart by
    /// looking for the real one).
    ///
    /// # Errors
    /// A bad or taken name (the reservation stays), or the card failed.
    pub fn node_commit(
        &mut self,
        r: &Reservation,
        dir: u64,
        name: &[u16],
        hidden: bool,
        stamps: (Option<Stamp>, Option<Stamp>),
        scratch: &mut [u8],
    ) -> Result<u64, Error> {
        if !valid_name(name) {
            return Err(Error::BadName);
        }
        let d = self.open16(dir)?;
        if d.first != r.dir_cluster {
            return Err(Error::NotFound);
        }
        if self.name_taken16(&d, name)? {
            return Err(Error::Exists);
        }
        let attrs = ATTR_ARCHIVE | if hidden { 0x02 } else { 0 };
        let mut set = Set {
            e: [[0; ENTRY]; SET_MAX],
            count: 0,
        };
        self.build16(
            name,
            attrs,
            r.first,
            r.size,
            r.contiguous,
            stamps.0,
            stamps.1,
            &mut set,
        )?;
        let loc = self.place16(&d, &set, scratch)?;
        self.delete16(r.temp)?;
        self.clean_up()?;
        Ok(loc)
    }

    /// Drops a reservation: its temporary set, then its clusters.
    ///
    /// # Errors
    /// The card failed.
    pub fn node_abandon(&mut self, r: &Reservation) -> Result<(), Error> {
        let _ = self.delete16(r.temp);
        self.release_run(r.first, r.clusters, r.contiguous)?;
        self.clean_up()
    }

    /// Cleans up writes a power cut interrupted (temporary sets in the
    /// root): clusters kept when the target folder holds a file using them,
    /// else freed. Returns how many were found.
    ///
    /// # Errors
    /// The card failed or is damaged.
    pub fn node_recover(&mut self) -> Result<u32, Error> {
        // At most 200 (two digits per name: 100 pending or committing).
        for fixed in 0..200 {
            let mut found = None;
            // Temporary sets are skipped by node_child: walk raw.
            let root = self.node_dir(ROOT)?;
            let mut set = Set {
                e: [[0; ENTRY]; SET_MAX],
                count: 0,
            };
            let mut candidates = [0u64; 8];
            let mut k = 0;
            self.walk_slots(Self::dir_for_exfat(&root), &mut |lba, _, slot, e| {
                if e[0] == T_FILE && k < candidates.len() {
                    candidates[k] = pos(lba, slot);
                    k += 1;
                }
                false
            })?;
            for &c in &candidates[..k] {
                if self.read_set16(c, &mut set).is_ok() {
                    let mut node = Node::default();
                    if self.node_of(&set, c, &mut node).is_ok() && is_temp(node.name()) {
                        found = Some((
                            c,
                            node,
                            le32(&set.e[0], 8),
                            le32(&set.e[0], 16),
                            set.e[1][1] & NO_FAT_CHAIN != 0,
                        ));
                        break;
                    }
                }
            }
            let Some((temp, node, dir_first, dir_len, contiguous)) = found else {
                return Ok(fixed);
            };
            // Is there a real file using these clusters in its folder?
            let dir = NodeDir {
                first: dir_first,
                contiguous: dir_len >> 31 != 0,
                len: dir_len & 0x7FFF_FFFF,
                entry: None,
                loc: ROOT,
            };
            let mut keep = false;
            if self.valid_cluster(dir.first) && dir.len > 0 {
                let first = node.first_cluster;
                self.walk_slots(Self::dir_for_exfat(&dir), &mut |_, _, _, e| {
                    keep |= e[0] == T_STREAM && le32(e, 20) == first;
                    false
                })?;
                // The temporary set's own stream matches too when it lives in
                // that folder (the root): count only another.
                if dir.first == self.root_cluster {
                    let mut users = 0;
                    self.walk_slots(Self::dir_for_exfat(&dir), &mut |_, _, _, e| {
                        users += u32::from(e[0] == T_STREAM && le32(e, 20) == first);
                        false
                    })?;
                    keep = users > 1;
                }
            }
            self.set_dirty(true)?;
            self.delete16(temp)?;
            if !keep && node.first_cluster != 0 {
                let cb = self.cluster_bytes() as u64;
                let n = node.size.div_ceil(cb) as u32;
                self.release_run(node.first_cluster, n, contiguous)?;
            }
            self.clean_up()?;
        }
        Err(Error::Corrupt)
    }

    /// Deletes a file or an empty folder.
    ///
    /// # Errors
    /// [`Error::NotEmpty`], [`Error::NotFound`], or the card failed.
    pub fn node_delete(&mut self, loc: u64) -> Result<(), Error> {
        let mut n = Node::default();
        self.node_stat(loc, &mut n)?;
        let mut set = Set {
            e: [[0; ENTRY]; SET_MAX],
            count: 0,
        };
        self.read_set16(loc, &mut set)?;
        let contiguous = set.e[1][1] & NO_FAT_CHAIN != 0;
        let alloc = le64(&set.e[1], 24);
        if n.folder {
            let mut c = 0;
            let mut child = Node::default();
            if self.node_child(loc, &mut c, &mut child)? {
                return Err(Error::NotEmpty);
            }
        }
        self.set_dirty(true)?;
        self.delete16(loc)?;
        if n.first_cluster != 0 && alloc > 0 {
            let cb = self.cluster_bytes() as u64;
            self.release_run(n.first_cluster, alloc.div_ceil(cb) as u32, contiguous)?;
        }
        self.clean_up()
    }

    /// Writes the set of `loc` again in folder `to` named `name` (every
    /// other field kept), then deletes the old set.
    fn reset16(
        &mut self,
        loc: u64,
        to: &NodeDir,
        name: &[u16],
        scratch: &mut [u8],
    ) -> Result<u64, Error> {
        let mut old = Set {
            e: [[0; ENTRY]; SET_MAX],
            count: 0,
        };
        self.read_set16(loc, &mut old)?;
        let mut set = Set {
            e: [[0; ENTRY]; SET_MAX],
            count: 0,
        };
        let st = old.e[1];
        let attrs = le16(&old.e[0], 4);
        self.build16(
            name,
            attrs,
            le32(&st, 20),
            le64(&st, 24),
            st[1] & NO_FAT_CHAIN != 0,
            None,
            None,
            &mut set,
        )?;
        // The timestamps, the valid length and the flags as they were.
        set.e[0][8..26].copy_from_slice(&old.e[0][8..26]);
        set.e[1][1] = st[1];
        set.e[1][8..16].copy_from_slice(&st[8..16]);
        let sum = set_checksum(&set.e[..set.count]);
        set.e[0][2..4].copy_from_slice(&sum.to_le_bytes());
        self.set_dirty(true)?;
        let new = self.place16(to, &set, scratch)?;
        self.delete16(loc)?;
        self.clean_up()?;
        Ok(new)
    }

    /// Renames the object at `loc` in folder `dir`; returns its new location.
    ///
    /// # Errors
    /// [`Error::BadName`], [`Error::Exists`], or the card failed.
    pub fn node_rename(
        &mut self,
        dir: u64,
        loc: u64,
        name: &[u16],
        scratch: &mut [u8],
    ) -> Result<u64, Error> {
        if !valid_name(name) {
            return Err(Error::BadName);
        }
        let mut n = Node::default();
        self.node_stat(loc, &mut n)?;
        let d = self.open16(dir)?;
        if !self.holds(&d, loc)? {
            return Err(Error::NotFound);
        }
        if n.name() == name {
            return Ok(loc);
        }
        if !same_name(n.name(), name) && self.name_taken16(&d, name)? {
            return Err(Error::Exists);
        }
        self.reset16(loc, &d, name, scratch)
    }

    /// Whether folder `d` holds the set at `loc` (its sector lies in the
    /// folder's clusters).
    fn holds(&mut self, d: &NodeDir, loc: u64) -> Result<bool, Error> {
        let (lba, _) = split(loc);
        if !self.in_heap(lba) {
            return Ok(false);
        }
        let c = ((lba - self.heap_start) >> self.cluster_shift) + 2;
        if d.contiguous {
            return Ok((d.first..d.first.saturating_add(d.len)).contains(&c));
        }
        let mut k = d.first;
        for _ in 0..d.len.min(self.clusters) {
            if k == c {
                return Ok(true);
            }
            match self.fat_next(k)? {
                Some(nk) => k = nk,
                None => return Ok(false),
            }
        }
        Ok(false)
    }

    /// Moves the object at `loc` from folder `from` into folder `dir` (no
    /// data moves; exFAT folders have no `..`); returns the new location.
    ///
    /// # Errors
    /// [`Error::Exists`] (taken name, or a folder into itself), or the
    /// card failed.
    pub fn node_move(
        &mut self,
        from: u64,
        loc: u64,
        dir: u64,
        scratch: &mut [u8],
    ) -> Result<u64, Error> {
        let mut n = Node::default();
        self.node_stat(loc, &mut n)?;
        let src = self.open16(from)?;
        if !self.holds(&src, loc)? {
            return Err(Error::NotFound);
        }
        let d = self.open16(dir)?;
        if d.first == src.first {
            return Ok(loc);
        }
        if n.folder && (dir == loc || self.below16(dir, loc)?) {
            return Err(Error::Exists);
        }
        let name = n.name;
        if self.name_taken16(&d, &name[..n.name_len])? {
            return Err(Error::Exists);
        }
        self.reset16(loc, &d, &name[..n.name_len], scratch)
    }

    /// Whether folder `dir` lies inside folder `top` (searched from `top`
    /// down, depth-bounded: exFAT records no parents).
    fn below16(&mut self, dir: u64, top: u64) -> Result<bool, Error> {
        let mut stack = [(0u64, 0u64); 16];
        let mut depth = 1;
        stack[0] = (top, 0);
        let mut n = Node::default();
        while depth > 0 {
            let (d, mut c) = stack[depth - 1];
            let more = self.node_child(d, &mut c, &mut n)?;
            stack[depth - 1].1 = c;
            if !more {
                depth -= 1;
                continue;
            }
            if n.folder {
                if n.loc == dir {
                    return Ok(true);
                }
                if depth < stack.len() {
                    stack[depth] = (n.loc, 0);
                    depth += 1;
                }
            }
        }
        Ok(false)
    }

    /// Reads file bytes from `offset` into `buf`; returns how many.
    ///
    /// # Errors
    /// The card failed or the file is damaged.
    pub fn node_read(
        &mut self,
        n: &Node,
        offset: u64,
        buf: &mut [u8],
        scratch: &mut [u8],
    ) -> Result<usize, Error> {
        let mut set = Set {
            e: [[0; ENTRY]; SET_MAX],
            count: 0,
        };
        self.read_set16(n.loc, &mut set)?;
        let st = &set.e[1];
        let file = crate::exfat::ExFile::new(
            le32(st, 20),
            le64(st, 24),
            le64(st, 8),
            st[1] & NO_FAT_CHAIN != 0,
            n.folder,
        );
        let mut got = 0usize;
        self.read_range(&file, offset, buf.len() as u64, scratch, |piece| {
            buf[got..got + piece.len()].copy_from_slice(piece);
            got += piece.len();
        })?;
        Ok(got)
    }

    /// Capacity and free space in bytes (free counted from the bitmap).
    ///
    /// # Errors
    /// The card failed.
    pub fn node_space(&mut self, scratch: &mut [u8]) -> Result<(u64, u64), Error> {
        let (_, free) = self.scan(2, u32::MAX, scratch)?;
        let cb = self.cluster_bytes() as u64;
        Ok((u64::from(self.clusters) * cb, u64::from(free) * cb))
    }

    /// The largest file: exFAT's sizes are 64-bit; the card's capacity.
    #[must_use]
    pub fn node_max_file(&self) -> u64 {
        u64::from(self.clusters) * self.cluster_bytes() as u64
    }
}

/// `~RPMTPnn.TMP` as UTF-16.
fn format_temp(n: u8) -> [u16; 12] {
    let mut out = [0u16; 12];
    for (d, c) in out.iter_mut().zip("~RPMTP00.TMP".encode_utf16()) {
        *d = c;
    }
    out[6] = 0x30 + u16::from(n / 10);
    out[7] = 0x30 + u16::from(n % 10);
    out
}
