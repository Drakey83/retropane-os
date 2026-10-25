//! Files and folders by where their entries are, for a responder that
//! names objects by place: MTP (ADR-0037). Everything a PC asks for comes
//! by location, never by path: the children of a folder from a cursor, an
//! object from its entry's place, a new file reserved whole and named only
//! once complete.
//!
//! - **A location** ([`Node::loc`]) is the place of the first entry of an
//!   object's directory entry set: `sector << 4 | slot` (a FAT long name's
//!   first entry, or the short entry when it has none; exFAT's file entry).
//!   It stays while the object exists unchanged: other entries never move
//!   (new ones go into free slots); a rename or a move gives a new one.
//!   [`ROOT`] is the root folder.
//! - **Names** are UTF-16, up to 255 units (FAT long names and exFAT names
//!   as stored), compared case-insensitively. The rest of `rp-fat` keeps
//!   its ASCII paths.
//! - **Writes are all or nothing** ([`Reservation`]): a file's clusters are
//!   allocated whole (one run when there is one) before its bytes arrive,
//!   with a hidden temporary entry in the root (`~RPMTPnn.TMP`) so a power
//!   cut is cleaned up at the next mount ([`Volume::node_recover`]); the
//!   real entry is written only once every byte is on the card. Committing
//!   first marks the temporary entry `~RPMTC` (with the target folder), then
//!   writes the real entry, then deletes the temporary one, so recovery
//!   knows which half a cut interrupted.
//!
//! Security (docs/security.md): locations come from the PC, so every one
//! is checked against the volume (inside a directory sector, at the start
//! of a whole, checksummed set) before anything is read or written; names
//! are checked against the file system's rules.

use rp_hal_api::BlockDevice;

use crate::{ATTR_DIRECTORY, ATTR_LONG_NAME, DIR_MAX_BYTES, ENTRIES, EOC, Error, SECTOR, Volume};
use crate::{le16, le32, short_checksum};

/// The root folder's location.
pub const ROOT: u64 = 0;
/// Most UTF-16 units in a name (FAT and exFAT).
pub const NAME_UNITS: usize = 255;
/// Long-name entries a 255-unit name needs.
const LFN_ENTRIES: usize = NAME_UNITS.div_ceil(13);

/// A date and time as the card records it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stamp {
    /// Four-digit year (FAT: 1980-2107).
    pub year: u16,
    /// 1..=12.
    pub month: u8,
    /// 1..=31.
    pub day: u8,
    /// 0..=23.
    pub hour: u8,
    /// 0..=59.
    pub minute: u8,
    /// 0..=59.
    pub second: u8,
    /// Hundredths past the second (0..=99), when recorded.
    pub centis: Option<u8>,
    /// Offset from UTC in minutes, when recorded (exFAT only).
    pub zone: Option<i16>,
}

impl Stamp {
    /// From FAT's date and time words (and the 10 ms count of a creation
    /// time); `None` for a zero date (none recorded).
    fn from_fat(date: u16, time: u16, tens: Option<u8>) -> Option<Self> {
        if date == 0 {
            return None;
        }
        let tens = tens.filter(|&t| t < 200);
        let s = Self {
            year: 1980 + (date >> 9),
            month: ((date >> 5) & 15) as u8,
            day: (date & 31) as u8,
            hour: (time >> 11) as u8,
            minute: ((time >> 5) & 63) as u8,
            second: ((time & 31) * 2) as u8 + tens.map_or(0, |t| t / 100),
            centis: tens.map(|t| t % 100),
            zone: None,
        };
        (s.month >= 1
            && s.month <= 12
            && s.day >= 1
            && s.hour < 24
            && s.minute < 60
            && s.second < 60)
            .then_some(s)
    }

    /// FAT's date, time and 10 ms words; `None` outside FAT's years.
    fn to_fat(self) -> Option<(u16, u16, u8)> {
        if !(1980..=2107).contains(&self.year) || self.month == 0 || self.day == 0 {
            return None;
        }
        let date = ((self.year - 1980) << 9) | (u16::from(self.month) << 5) | u16::from(self.day);
        let time = (u16::from(self.hour) << 11)
            | (u16::from(self.minute) << 5)
            | u16::from(self.second / 2);
        let tens = (self.second % 2) * 100 + self.centis.unwrap_or(0);
        Some((date, time, tens))
    }
}

/// One file or folder.
#[derive(Clone, Copy, Debug)]
pub struct Node {
    /// Where its entry set starts.
    pub loc: u64,
    /// Where its short entry (FAT) or file entry (exFAT) is, the place the
    /// boot-file guard records.
    pub entry: u64,
    /// Its name, UTF-16.
    pub name: [u16; NAME_UNITS],
    /// Units used in `name`.
    pub name_len: usize,
    /// A folder.
    pub folder: bool,
    /// Size in bytes.
    pub size: u64,
    /// First cluster (0: empty).
    pub first_cluster: u32,
    /// Attribute bits (read-only 0x01, hidden 0x02, system 0x04, folder
    /// 0x10, archive 0x20).
    pub attrs: u8,
    /// Created, when recorded.
    pub created: Option<Stamp>,
    /// Last modified, when recorded.
    pub modified: Option<Stamp>,
}

impl Default for Node {
    fn default() -> Self {
        Self {
            loc: ROOT,
            entry: ROOT,
            name: [0; NAME_UNITS],
            name_len: 0,
            folder: false,
            size: 0,
            first_cluster: 0,
            attrs: 0,
            created: None,
            modified: None,
        }
    }
}

impl Node {
    /// The name's units.
    #[must_use]
    pub fn name(&self) -> &[u16] {
        &self.name[..self.name_len]
    }
}

/// A file being written: its clusters allocated, its bytes arriving, its
/// name not yet on the card.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reservation {
    /// First cluster.
    pub first: u32,
    /// Clusters allocated.
    pub clusters: u32,
    /// Bytes it will hold.
    pub size: u64,
    /// One run of clusters.
    pub contiguous: bool,
    /// The temporary entry's place.
    pub temp: u64,
    /// The folder it goes into (cluster; FAT12/16's fixed root 0).
    pub dir_cluster: u32,
    /// The last cluster a write reached and its index in the file, so
    /// writes in order follow a chain without walking it again.
    at: (u32, u32),
}

impl Reservation {
    pub(crate) const fn new(
        first: u32,
        clusters: u32,
        size: u64,
        contiguous: bool,
        temp: u64,
        dir_cluster: u32,
    ) -> Self {
        Self {
            first,
            clusters,
            size,
            contiguous,
            temp,
            dir_cluster,
            at: (first, 0),
        }
    }

    /// Where to start following the chain for cluster `index`: the last
    /// one reached when it is at or before `index`, else the first.
    pub(crate) fn last_reached(&self, index: u32) -> (u32, u32) {
        if index >= self.at.1 {
            self.at
        } else {
            (self.first, 0)
        }
    }

    /// The chain was followed to cluster `c`, the file's `k`th.
    pub(crate) fn reached(&mut self, c: u32, k: u32) {
        self.at = (c, k);
    }
}

/// Characters no FAT or exFAT name may hold.
fn forbidden(u: u16) -> bool {
    u < 0x20
        || matches!(
            u,
            0x22 | 0x2A | 0x2F | 0x3A | 0x3C | 0x3E | 0x3F | 0x5C | 0x7C
        )
}

/// Whether `name` may be a new file's or folder's: 1..=255 units, none
/// forbidden, no trailing dot or space, not `.` or `..`.
#[must_use]
pub fn valid_name(name: &[u16]) -> bool {
    !name.is_empty()
        && name.len() <= NAME_UNITS
        && !name.iter().any(|&u| forbidden(u))
        && !matches!(name.last(), Some(0x2E | 0x20))
        && !name.iter().all(|&u| u == 0x2E)
}

/// One unit upper-cased (single-character mappings of the Basic
/// Multilingual Plane; what isn't one maps to itself).
#[must_use]
pub fn upcase(u: u16) -> u16 {
    if u < 0x80 {
        return u16::from((u as u8).to_ascii_uppercase());
    }
    match char::from_u32(u32::from(u)) {
        Some(c) => {
            let mut up = c.to_uppercase();
            match (up.next(), up.next()) {
                (Some(x), None) if (x as u32) <= 0xFFFF => x as u32 as u16,
                _ => u,
            }
        }
        None => u, // a surrogate half: as is
    }
}

/// Whether two names are the same, ignoring case.
#[must_use]
pub fn same_name(a: &[u16], b: &[u16]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(&x, &y)| x == y || upcase(x) == upcase(y))
}

fn split(loc: u64) -> (u32, usize) {
    ((loc >> 4) as u32, (loc & 15) as usize)
}

fn join(lba: u32, slot: usize) -> u64 {
    u64::from(lba) << 4 | slot as u64
}

/// The temporary entries' short names: `~RPMTP` (pending) or `~RPMTC`
/// (committing) and two digits, extension `TMP`.
const TEMP_BASE: &[u8; 5] = b"~RPMT";
const TEMP_PENDING: u8 = b'P';
const TEMP_COMMITTING: u8 = b'C';

fn temp_name(state: u8, n: u8) -> [u8; 11] {
    let mut s = *b"~RPMTP00TMP";
    s[5] = state;
    s[6] = b'0' + n / 10;
    s[7] = b'0' + n % 10;
    s
}

fn is_temp(short: &[u8]) -> Option<u8> {
    (short[..5] == TEMP_BASE[..]
        && matches!(short[5], TEMP_PENDING | TEMP_COMMITTING)
        && short[6].is_ascii_digit()
        && short[7].is_ascii_digit()
        && &short[8..11] == b"TMP")
        .then_some(short[5])
}

/// The long-name entries for `name`, last piece first as they go on disk;
/// returns how many.
fn lfn16(name: &[u16], checksum: u8, out: &mut [[u8; 32]; LFN_ENTRIES]) -> usize {
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
            let c = match idx.cmp(&name.len()) {
                core::cmp::Ordering::Less => name[idx],
                core::cmp::Ordering::Equal => 0,
                core::cmp::Ordering::Greater => 0xFFFF,
            };
            e[o..o + 2].copy_from_slice(&c.to_le_bytes());
        }
    }
    pieces
}

/// A long name being read, entry by entry.
struct Lfn {
    units: [u16; LFN_ENTRIES * 13],
    /// The sequence number expected next (0: none in progress or done).
    next: u8,
    pieces: u8,
    checksum: u8,
    /// Where its first entry is.
    start: u64,
}

impl Lfn {
    const fn new() -> Self {
        Self {
            units: [0; LFN_ENTRIES * 13],
            next: 0,
            pieces: 0,
            checksum: 0,
            start: 0,
        }
    }

    fn reset(&mut self) {
        self.next = 0;
        self.pieces = 0;
    }

    /// Takes a long-name entry at `pos`.
    fn take(&mut self, e: &[u8], pos: u64) {
        const AT: [usize; 13] = [1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30];
        let seq = e[0] & 0x1F;
        if e[0] & 0x40 != 0 {
            if seq == 0 || usize::from(seq) > LFN_ENTRIES {
                self.reset();
                return;
            }
            self.pieces = seq;
            self.next = seq;
            self.checksum = e[13];
            self.start = pos;
            self.units[usize::from(seq - 1) * 13..usize::from(seq) * 13].fill(0xFFFF);
        } else if self.next == 0 || seq != self.next || e[13] != self.checksum {
            self.reset();
            return;
        }
        let base = usize::from(seq - 1) * 13;
        for (i, &o) in AT.iter().enumerate() {
            self.units[base + i] = le16(e, o);
        }
        self.next -= 1;
    }

    /// The name for a short entry with this checksum, if a whole long name
    /// precedes it: its units and where it started.
    fn finish(&mut self, checksum: u8) -> Option<(&[u16], u64)> {
        let whole = self.pieces > 0 && self.next == 0 && self.checksum == checksum;
        let pieces = usize::from(self.pieces);
        self.reset();
        if !whole {
            return None;
        }
        let all = &self.units[..pieces * 13];
        let len = all
            .iter()
            .position(|&u| u == 0 || u == 0xFFFF)
            .unwrap_or(all.len());
        (len > 0 && len <= NAME_UNITS).then(|| (&self.units[..len], self.start))
    }
}

/// The short display name (`NAME.EXT`, lower case where Windows NT's case
/// flags say so) as UTF-16 (OEM bytes over 0x7F read as themselves).
fn short16(e: &[u8], out: &mut [u16; NAME_UNITS]) -> usize {
    let mut b = [0u8; 12];
    let n = crate::short_to_display(&e[..11], e[12], &mut b);
    for (o, &c) in out.iter_mut().zip(&b[..n]) {
        *o = u16::from(c);
    }
    n
}

/// A position in a directory walk: the next entry's place, entries seen.
fn cursor_pack(lba: u32, slot: usize, seen: u32) -> u64 {
    u64::from(seen) << 36 | join(lba, slot)
}

fn cursor_unpack(c: u64) -> (u32, usize, u32) {
    let (lba, slot) = split(c & ((1 << 36) - 1));
    (lba, slot, (c >> 36) as u32)
}

impl<D: BlockDevice> Volume<D> {
    /// The directory sector after `lba` in its directory, if any.
    fn dir_after(&mut self, lba: u32) -> Result<Option<u32>, Error> {
        if self.root_sectors > 0
            && (self.root_lba..self.root_lba + self.root_sectors).contains(&lba)
        {
            return Ok((lba + 1 < self.root_lba + self.root_sectors).then_some(lba + 1));
        }
        if lba < self.data_start {
            return Err(Error::Corrupt);
        }
        let rel = lba - self.data_start;
        if !(rel + 1).is_multiple_of(self.sectors_per_cluster) {
            return Ok(Some(lba + 1));
        }
        let c = rel / self.sectors_per_cluster + 2;
        Ok(self.next_cluster(c)?.map(|n| self.cluster_lba(n)))
    }

    /// Whether `lba` can hold directory entries (the fixed root, or a data
    /// cluster).
    fn dir_sector_ok(&self, lba: u32) -> bool {
        let fixed = self.root_sectors > 0
            && (self.root_lba..self.root_lba + self.root_sectors).contains(&lba);
        let data = lba >= self.data_start
            && ((lba - self.data_start) / self.sectors_per_cluster) < self.clusters;
        fixed || data
    }

    /// The first cluster of folder `dir` (a location, or [`ROOT`]); the
    /// fixed root is 0.
    fn folder_cluster(&mut self, dir: u64) -> Result<u32, Error> {
        if dir == ROOT {
            return Ok(self.root_cluster);
        }
        let mut n = Node::default();
        self.node_stat(dir, &mut n)?;
        if !n.folder || !self.valid_cluster(n.first_cluster) || n.first_cluster == self.root_cluster
        {
            return Err(Error::NotFound);
        }
        Ok(n.first_cluster)
    }

    /// Reads an entry set from its first entry at `loc` into `out` (its
    /// short entry's place in `out.entry`). Deleted, half or mismatched
    /// sets are [`Error::NotFound`].
    fn read_set(&mut self, loc: u64, out: &mut Node) -> Result<(), Error> {
        let (mut lba, slot) = split(loc);
        if !self.dir_sector_ok(lba) {
            return Err(Error::NotFound);
        }
        let mut lfn = Lfn::new();
        let mut i = slot;
        self.dev.read_blocks(lba, &mut self.buf)?;
        for _ in 0..=LFN_ENTRIES {
            if i == ENTRIES {
                lba = self.dir_after(lba)?.ok_or(Error::NotFound)?;
                self.dev.read_blocks(lba, &mut self.buf)?;
                i = 0;
            }
            let mut e = [0u8; 32];
            e.copy_from_slice(&self.buf[i * 32..i * 32 + 32]);
            let pos = join(lba, i);
            if matches!(e[0], 0x00 | 0xE5) {
                return Err(Error::NotFound);
            }
            if e[11] & 0x3F == ATTR_LONG_NAME {
                // Only a set's first entry starts a name.
                if pos == loc && e[0] & 0x40 == 0 {
                    return Err(Error::NotFound);
                }
                lfn.take(&e, pos);
                if lfn.pieces == 0 {
                    return Err(Error::NotFound);
                }
                i += 1;
                continue;
            }
            if e[11] & 0x08 != 0 || e[0] == b'.' {
                return Err(Error::NotFound);
            }
            let short: [u8; 11] = e[..11].try_into().unwrap_or([0; 11]);
            let named = lfn.finish(short_checksum(&short)).map(|(u, start)| {
                out.name[..u.len()].copy_from_slice(u);
                (u.len(), start)
            });
            match named {
                Some((len, start)) if start == loc => out.name_len = len,
                Some(_) => return Err(Error::NotFound),
                // A long name that didn't check out: this isn't a set's start.
                None if pos != loc => return Err(Error::NotFound),
                None => out.name_len = short16(&e, &mut out.name),
            }
            self.fill_node(&e, loc, pos, out);
            return Ok(());
        }
        Err(Error::NotFound)
    }

    fn fill_node(&self, e: &[u8; 32], loc: u64, entry: u64, out: &mut Node) {
        out.loc = loc;
        out.entry = entry;
        out.folder = e[11] & ATTR_DIRECTORY != 0;
        out.size = if out.folder {
            0
        } else {
            u64::from(le32(e, 28))
        };
        out.first_cluster = crate::entry_first(e, self.kind);
        out.attrs = e[11];
        out.created = Stamp::from_fat(le16(e, 16), le16(e, 14), Some(e[13]));
        out.modified = Stamp::from_fat(le16(e, 24), le16(e, 22), None);
    }

    /// The object at `loc` as it is now.
    ///
    /// # Errors
    /// [`Error::NotFound`] when `loc` isn't the start of a whole entry set.
    pub fn node_stat(&mut self, loc: u64, out: &mut Node) -> Result<(), Error> {
        if loc == ROOT {
            return Err(Error::NotFound);
        }
        self.read_set(loc, out)
    }

    /// The child after `cursor` (0 to start; advanced) in folder `dir`:
    /// `Ok(false)` at the end. Volume labels, `.`, `..` and the temporary
    /// entries of writes in progress are skipped.
    ///
    /// # Errors
    /// The folder is gone or damaged, or the card failed.
    pub fn node_child(
        &mut self,
        dir: u64,
        cursor: &mut u64,
        out: &mut Node,
    ) -> Result<bool, Error> {
        let (mut lba, mut i, mut seen) = if *cursor == 0 {
            let c = self.folder_cluster(dir)?;
            (self.dir_unit(c).0, 0, 0)
        } else {
            cursor_unpack(*cursor)
        };
        if lba == 0 || !self.dir_sector_ok(lba) {
            return Ok(false);
        }
        let max = (DIR_MAX_BYTES / 32) as u32;
        let mut lfn = Lfn::new();
        self.dev.read_blocks(lba, &mut self.buf)?;
        loop {
            if i == ENTRIES {
                match self.dir_after(lba)? {
                    Some(next) => {
                        lba = next;
                        self.dev.read_blocks(lba, &mut self.buf)?;
                        i = 0;
                    }
                    None => {
                        *cursor = cursor_pack(0, 0, seen);
                        return Ok(false);
                    }
                }
            }
            seen += 1;
            if seen > max {
                return Err(Error::Corrupt);
            }
            let mut e = [0u8; 32];
            e.copy_from_slice(&self.buf[i * 32..i * 32 + 32]);
            let pos = join(lba, i);
            i += 1;
            match e[0] {
                0x00 => {
                    *cursor = cursor_pack(0, 0, seen);
                    return Ok(false);
                }
                0xE5 => {
                    lfn.reset();
                    continue;
                }
                _ => {}
            }
            if e[11] & 0x3F == ATTR_LONG_NAME {
                lfn.take(&e, pos);
                continue;
            }
            let short: [u8; 11] = e[..11].try_into().unwrap_or([0; 11]);
            if e[11] & 0x08 != 0 || e[0] == b'.' || is_temp(&short).is_some() {
                lfn.reset();
                continue;
            }
            let loc = match lfn.finish(short_checksum(&short)) {
                Some((u, start)) => {
                    out.name[..u.len()].copy_from_slice(u);
                    out.name_len = u.len();
                    start
                }
                None => {
                    out.name_len = short16(&e, &mut out.name);
                    pos
                }
            };
            self.fill_node(&e, loc, pos, out);
            *cursor = cursor_pack(lba, i, seen);
            // Past this sector's end: the cursor resumes at the next one.
            if i == ENTRIES {
                *cursor = match self.dir_after(lba)? {
                    Some(next) => cursor_pack(next, 0, seen),
                    None => cursor_pack(0, 0, seen),
                };
            }
            return Ok(true);
        }
    }

    /// Whether folder `dir_cluster` holds an entry named `name` (long or
    /// short name, any case), the temporary entries of writes in progress
    /// apart.
    fn name_taken(&mut self, dir_cluster: u32, name: &[u16]) -> Result<bool, Error> {
        let (mut lba, _) = self.dir_unit(dir_cluster);
        let mut i = 0;
        let mut lfn = Lfn::new();
        let mut short_name = [0u16; NAME_UNITS];
        let mut seen = 0u32;
        self.dev.read_blocks(lba, &mut self.buf)?;
        loop {
            if i == ENTRIES {
                match self.dir_after(lba)? {
                    Some(n) => {
                        lba = n;
                        self.dev.read_blocks(lba, &mut self.buf)?;
                        i = 0;
                    }
                    None => return Ok(false),
                }
            }
            seen += 1;
            if seen > (DIR_MAX_BYTES / 32) as u32 {
                return Err(Error::Corrupt);
            }
            let mut e = [0u8; 32];
            e.copy_from_slice(&self.buf[i * 32..i * 32 + 32]);
            let pos = join(lba, i);
            i += 1;
            match e[0] {
                0x00 => return Ok(false),
                0xE5 => {
                    lfn.reset();
                    continue;
                }
                _ => {}
            }
            if e[11] & 0x3F == ATTR_LONG_NAME {
                lfn.take(&e, pos);
                continue;
            }
            let short: [u8; 11] = e[..11].try_into().unwrap_or([0; 11]);
            let long = lfn
                .finish(short_checksum(&short))
                .is_some_and(|(u, _)| same_name(u, name));
            if e[0] == b'.' || e[11] & 0x08 != 0 || is_temp(&short).is_some() {
                continue;
            }
            let n = short16(&e, &mut short_name);
            if long || same_name(&short_name[..n], name) {
                return Ok(true);
            }
        }
    }

    /// The short name for `name` in folder `dir_cluster`: its own 8.3 form
    /// when it is one (with Windows NT's case flags), else `BASIS~N.EXT`
    /// with long-name entries.
    fn short_for(&mut self, dir_cluster: u32, name: &[u16]) -> Result<([u8; 11], u8, bool), Error> {
        let mut ascii = [0u8; NAME_UNITS];
        let mut all_ascii = true;
        for (a, &u) in ascii.iter_mut().zip(name) {
            *a = if u < 0x80 {
                u as u8
            } else {
                all_ascii = false;
                b'_'
            };
        }
        let ascii = &ascii[..name.len()];
        if all_ascii && let Some((s, nt)) = crate::plain_short(ascii) {
            return Ok((s, nt, false));
        }
        for k in 1..1_000_000 {
            let s = crate::numbered_short(ascii, k);
            if !self.short_exists(dir_cluster, &s)? {
                return Ok((s, 0, true));
            }
        }
        Err(Error::NoRoom)
    }

    /// Writes an entry set for `name` into folder `dir_cluster` (long-name
    /// entries in order, the short entry last, so it exists only once
    /// whole); returns its location and how many clusters the folder grew.
    #[allow(clippy::too_many_arguments)]
    fn add_set16(
        &mut self,
        dir_cluster: u32,
        name: &[u16],
        attr: u8,
        first_cluster: u32,
        size: u32,
        created: Option<Stamp>,
        modified: Option<Stamp>,
        scratch: &mut [u8],
    ) -> Result<(u64, u32), Error> {
        let (short, nt, long) = self.short_for(dir_cluster, name)?;
        let mut entries = [[0u8; 32]; LFN_ENTRIES + 1];
        let mut count = 0;
        if long {
            let mut l = [[0u8; 32]; LFN_ENTRIES];
            count = lfn16(name, short_checksum(&short), &mut l);
            entries[..count].copy_from_slice(&l[..count]);
        }
        let e = &mut entries[count];
        e[..11].copy_from_slice(&short);
        e[11] = attr;
        e[12] = nt;
        if let Some((date, time, tens)) = created.and_then(Stamp::to_fat) {
            e[13] = tens;
            e[14..16].copy_from_slice(&time.to_le_bytes());
            e[16..18].copy_from_slice(&date.to_le_bytes());
            e[18..20].copy_from_slice(&date.to_le_bytes());
        }
        if let Some((date, time, _)) = modified.or(created).and_then(Stamp::to_fat) {
            e[22..24].copy_from_slice(&time.to_le_bytes());
            e[24..26].copy_from_slice(&date.to_le_bytes());
        }
        e[20..22].copy_from_slice(&((first_cluster >> 16) as u16).to_le_bytes());
        e[26..28].copy_from_slice(&(first_cluster as u16).to_le_bytes());
        e[28..32].copy_from_slice(&size.to_le_bytes());
        count += 1;
        let mut grown = 0;
        let slots = self.free_slots16(dir_cluster, count, scratch, &mut grown)?;
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
        Ok((join(slots[0].0, usize::from(slots[0].1)), grown))
    }

    /// `need` consecutive free slots in `dir`, growing it when it has none
    /// (as `free_slots`, for sets of up to 21 entries).
    fn free_slots16(
        &mut self,
        dir: u32,
        need: usize,
        scratch: &mut [u8],
        grown: &mut u32,
    ) -> Result<[(u32, u8); LFN_ENTRIES + 1], Error> {
        let mut run = [(0u32, 0u8); LFN_ENTRIES + 1];
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
            if self.fixed_root(dir) {
                return Err(Error::NoRoom);
            }
            visited += 1;
            match self.next_cluster(cluster)? {
                Some(c) if visited < self.dir_clusters_max() => cluster = c,
                Some(_) => return Err(Error::Corrupt),
                None if visited >= self.dir_clusters_max() => return Err(Error::NoRoom),
                None => {
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
                    self.fsinfo_adjust(-1, Some(c + 1))?;
                    *grown += 1;
                    cluster = c;
                }
            }
        }
    }

    /// Marks the entry set starting at `loc` deleted: the short entry
    /// first, so the object vanishes at once, then its long-name entries.
    fn delete_set(&mut self, loc: u64) -> Result<(), Error> {
        let mut n = Node::default();
        self.read_set(loc, &mut n)?;
        let (short_lba, short_slot) = split(n.entry);
        self.dev.read_blocks(short_lba, &mut self.buf)?;
        self.buf[short_slot * 32] = 0xE5;
        self.dev.write_blocks(short_lba, &self.buf)?;
        // Then the long-name entries, from the first to the short one.
        let (mut lba, mut i) = split(loc);
        if join(lba, i) == n.entry {
            return Ok(());
        }
        self.dev.read_blocks(lba, &mut self.buf)?;
        for _ in 0..=LFN_ENTRIES {
            if i == ENTRIES {
                self.dev.write_blocks(lba, &self.buf)?;
                lba = self.dir_after(lba)?.ok_or(Error::Corrupt)?;
                self.dev.read_blocks(lba, &mut self.buf)?;
                i = 0;
            }
            if join(lba, i) == n.entry {
                break;
            }
            self.buf[i * 32] = 0xE5;
            i += 1;
        }
        self.dev.write_blocks(lba, &self.buf)?;
        Ok(())
    }

    /// Creates a folder or an empty file named `name` in `dir`.
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
        let dc = self.folder_cluster(dir)?;
        if self.name_taken(dc, name)? {
            return Err(Error::Exists);
        }
        let attr = (if folder { ATTR_DIRECTORY } else { 0x20 }) | if hidden { 0x02 } else { 0 };
        if !folder {
            let (loc, _) = self.add_set16(dc, name, attr, 0, 0, stamps.0, stamps.1, scratch)?;
            return Ok(loc);
        }
        let hint = self.fsinfo_hint()?;
        let Some(c) = self.find_free_run(hint, 1, scratch)? else {
            return Err(Error::NoRoom);
        };
        let lba = self.cluster_lba(c);
        let up = if dc == self.root_cluster { 0 } else { dc };
        for sec in 0..self.sectors_per_cluster {
            self.buf = [0; SECTOR];
            if sec == 0 {
                self.buf[..32].copy_from_slice(&dot_entry(b".          ", c));
                self.buf[32..64].copy_from_slice(&dot_entry(b"..         ", up));
            }
            self.dev.write_blocks(lba + sec, &self.buf)?;
        }
        self.fat_set(c, EOC)?;
        self.fat_flush()?;
        match self.add_set16(dc, name, attr, c, 0, stamps.0, stamps.1, scratch) {
            Ok((loc, grown)) => {
                self.fsinfo_adjust(-i64::from(1 + grown), Some(c + 1))?;
                Ok(loc)
            }
            Err(e) => {
                self.free_chain(c)?;
                Err(e)
            }
        }
    }

    /// Reserves a file of `size` bytes for folder `dir`: its clusters
    /// allocated and chained (one run when there is one), a temporary entry
    /// in the root naming them, nothing visible to anyone else.
    ///
    /// # Errors
    /// [`Error::NoRoom`] (the space isn't there: nothing is changed),
    /// [`Error::Fragmented`] never; the folder is gone.
    pub fn node_reserve(
        &mut self,
        dir: u64,
        size: u64,
        scratch: &mut [u8],
    ) -> Result<Reservation, Error> {
        let dc = self.folder_cluster(dir)?;
        let size32 = u32::try_from(size).map_err(|_| Error::NoRoom)?;
        let cb = self.cluster_bytes() as u64;
        let need = u32::try_from(size.div_ceil(cb)).map_err(|_| Error::NoRoom)?;
        if need == 0 {
            return Err(Error::NoRoom);
        }
        let hint = self.fsinfo_hint()?;
        let (first, contiguous) = match self.find_free_run(hint, need, scratch)? {
            Some(start) => {
                for c in start..start + need {
                    self.fat_set(c, if c + 1 == start + need { EOC } else { c + 1 })?;
                }
                (start, true)
            }
            None => {
                // Chained: free clusters one by one (a full search first, so a
                // card without the space is refused untouched).
                if self.free_clusters(scratch)? < need {
                    return Err(Error::NoRoom);
                }
                let mut first = 0;
                let mut prev = None;
                let mut from = hint;
                for _ in 0..need {
                    let Some(c) = self.find_free_run(from, 1, scratch)? else {
                        if first != 0 {
                            self.free_chain(first)?;
                        }
                        return Err(Error::NoRoom);
                    };
                    self.fat_set(c, EOC)?;
                    match prev {
                        Some(p) => self.fat_set(p, c)?,
                        None => first = c,
                    }
                    prev = Some(c);
                    from = c + 1;
                }
                (first, false)
            }
        };
        self.fat_flush()?;
        self.fsinfo_adjust(
            -i64::from(need),
            Some(if contiguous { first + need } else { hint }),
        )?;
        // The temporary entry: hidden + system, in the root, its creation
        // time words holding the target folder's cluster.
        let temp = match self.add_temp(first, size32, dc, scratch) {
            Ok(t) => t,
            Err(e) => {
                self.free_chain(first)?;
                return Err(e);
            }
        };
        Ok(Reservation {
            first,
            clusters: need,
            size,
            contiguous,
            temp,
            dir_cluster: dc,
            at: (first, 0),
        })
    }

    fn add_temp(
        &mut self,
        first: u32,
        size: u32,
        dir_cluster: u32,
        scratch: &mut [u8],
    ) -> Result<u64, Error> {
        let root = self.root_cluster;
        let mut n = 0u8;
        let short = loop {
            let s = temp_name(TEMP_PENDING, n);
            let c = temp_name(TEMP_COMMITTING, n);
            if !self.short_exists(root, &s)? && !self.short_exists(root, &c)? {
                break s;
            }
            n += 1;
            if n >= 100 {
                return Err(Error::NoRoom);
            }
        };
        let mut e = [0u8; 32];
        e[..11].copy_from_slice(&short);
        e[11] = 0x06;
        e[14..18].copy_from_slice(&dir_cluster.to_le_bytes());
        e[20..22].copy_from_slice(&((first >> 16) as u16).to_le_bytes());
        e[26..28].copy_from_slice(&(first as u16).to_le_bytes());
        e[28..32].copy_from_slice(&size.to_le_bytes());
        let mut grown = 0;
        let slot = self.free_slots16(root, 1, scratch, &mut grown)?[0];
        self.dev.read_blocks(slot.0, &mut self.buf)?;
        let o = usize::from(slot.1) * 32;
        self.buf[o..o + 32].copy_from_slice(&e);
        self.dev.write_blocks(slot.0, &self.buf)?;
        Ok(join(slot.0, usize::from(slot.1)))
    }

    /// Free clusters, counted from the FAT (for a refusal before anything
    /// changes).
    fn free_clusters(&mut self, scratch: &mut [u8]) -> Result<u32, Error> {
        let _ = scratch;
        let mut free = 0;
        for c in 2..2 + self.clusters {
            if self.fat_get(c)? == 0 {
                free += 1;
            }
        }
        Ok(free)
    }

    /// Writes `data` into a reservation at `offset` (a whole number of
    /// sectors, except at the file's end, whose last sector is padded with
    /// zeros).
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
                // Forward from the last cluster reached (writes come in
                // order); from the start otherwise.
                let (mut c, mut k) = if index >= r.at.1 { r.at } else { (r.first, 0) };
                while k < index {
                    c = self.next_cluster(c)?.ok_or(Error::Corrupt)?;
                    k += 1;
                }
                r.at = (c, k);
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
                let tail = &data[at + whole..at + n];
                let pad = if scratch.len() >= SECTOR {
                    &mut scratch[..SECTOR]
                } else {
                    &mut self.buf[..]
                };
                pad.fill(0);
                pad[..tail.len()].copy_from_slice(tail);
                let sector: [u8; SECTOR] = pad[..SECTOR].try_into().unwrap_or([0; SECTOR]);
                self.dev
                    .write_blocks(lba + (whole / SECTOR) as u32, &sector)?;
            }
            at += n;
        }
        Ok(())
    }

    /// Names a complete reservation `name` in its folder: the temporary
    /// entry marked committing, the real entry set written, the temporary
    /// entry deleted. Returns the file's location.
    ///
    /// # Errors
    /// A bad or taken name (the reservation stays, to retry or abandon),
    /// or the card failed.
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
        if self.folder_cluster(dir)? != r.dir_cluster {
            return Err(Error::NotFound);
        }
        if self.name_taken(r.dir_cluster, name)? {
            return Err(Error::Exists);
        }
        let size = u32::try_from(r.size).map_err(|_| Error::NoRoom)?;
        self.set_temp_state(r.temp, TEMP_COMMITTING)?;
        let attr = 0x20 | if hidden { 0x02 } else { 0 };
        let (loc, grown) = self.add_set16(
            r.dir_cluster,
            name,
            attr,
            r.first,
            size,
            stamps.0,
            stamps.1,
            scratch,
        )?;
        if grown > 0 {
            self.fsinfo_adjust(-i64::from(grown), None)?;
        }
        self.kill_temp(r.temp)?;
        Ok(loc)
    }

    fn set_temp_state(&mut self, temp: u64, state: u8) -> Result<(), Error> {
        let (lba, slot) = split(temp);
        self.dev.read_blocks(lba, &mut self.buf)?;
        let o = slot * 32;
        if is_temp(&self.buf[o..o + 11]).is_none() {
            return Err(Error::Corrupt);
        }
        self.buf[o + 5] = state;
        self.dev.write_blocks(lba, &self.buf)?;
        Ok(())
    }

    fn kill_temp(&mut self, temp: u64) -> Result<(), Error> {
        let (lba, slot) = split(temp);
        self.dev.read_blocks(lba, &mut self.buf)?;
        let o = slot * 32;
        if is_temp(&self.buf[o..o + 11]).is_some() {
            self.buf[o] = 0xE5;
            self.dev.write_blocks(lba, &self.buf)?;
        }
        Ok(())
    }

    /// Drops a reservation: its temporary entry, then its clusters.
    ///
    /// # Errors
    /// The card failed.
    pub fn node_abandon(&mut self, r: &Reservation) -> Result<(), Error> {
        self.kill_temp(r.temp)?;
        self.free_chain(r.first)
    }

    /// Cleans up writes a power cut interrupted (the temporary entries in
    /// the root): a pending one's clusters are freed; a committing one's
    /// are kept if its folder holds a file using them, else freed. Returns
    /// how many were found.
    ///
    /// # Errors
    /// The card failed or is damaged.
    pub fn node_recover(&mut self) -> Result<u32, Error> {
        // At most 200 (two digits per name: 100 pending or committing).
        for fixed in 0..200 {
            let root = self.root_cluster;
            let mut found = None;
            let kind = self.kind;
            self.walk_dir(root, &mut |lba, idx, e, _| {
                if let Some(state) = is_temp(&e[..11]) {
                    found = Some((
                        join(lba, usize::from(idx)),
                        state,
                        crate::entry_first(e, kind),
                        le32(e, 14),
                    ));
                    return true;
                }
                false
            })?;
            let Some((temp, state, first, dir)) = found else {
                return Ok(fixed);
            };
            let mut keep = false;
            if state == TEMP_COMMITTING && (dir == self.root_cluster || self.valid_cluster(dir)) {
                self.walk_dir(dir, &mut |_, _, e, _| {
                    keep = is_temp(&e[..11]).is_none()
                        && e[11] & 0x3F != ATTR_LONG_NAME
                        && crate::entry_first(e, kind) == first
                        && first != 0;
                    keep
                })?;
            }
            self.kill_temp(temp)?;
            if !keep && self.valid_cluster(first) {
                self.free_chain(first)?;
            }
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
        if n.folder {
            let mut c = 0u64;
            let mut child = Node::default();
            if self.node_child(loc, &mut c, &mut child)? {
                return Err(Error::NotEmpty);
            }
        }
        self.delete_set(loc)?;
        if n.first_cluster != 0 && self.valid_cluster(n.first_cluster) {
            self.free_chain(n.first_cluster)?;
        }
        Ok(())
    }

    /// The cluster of folder `dir`, checked to hold the entry at `entry`
    /// (a location from the PC is checked against the folder it says).
    fn holding_folder(&mut self, dir: u64, entry: u64) -> Result<u32, Error> {
        let dc = self.folder_cluster(dir)?;
        let (lba, _) = split(entry);
        let fixed = self.root_sectors > 0
            && (self.root_lba..self.root_lba + self.root_sectors).contains(&lba);
        let ok = if self.fixed_root(dc) {
            fixed
        } else {
            !fixed && lba >= self.data_start && {
                let c = (lba - self.data_start) / self.sectors_per_cluster + 2;
                self.chain_holds(dc, c)?
            }
        };
        if ok { Ok(dc) } else { Err(Error::NotFound) }
    }

    /// Whether the chain from `head` (a directory, so at most its largest
    /// size) holds cluster `c`.
    fn chain_holds(&mut self, head: u32, c: u32) -> Result<bool, Error> {
        let mut k = head;
        for _ in 0..self.dir_clusters_max() {
            if k == c {
                return Ok(true);
            }
            match self.next_cluster(k)? {
                Some(n) => k = n,
                None => return Ok(false),
            }
        }
        Ok(false)
    }

    /// Renames the object at `loc` in folder `dir`: a new entry set, then
    /// the old one deleted (both whole at every step); returns the new
    /// location.
    ///
    /// # Errors
    /// [`Error::BadName`], [`Error::Exists`], `loc` not in `dir`, or the
    /// card failed.
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
        let dc = self.holding_folder(dir, n.entry)?;
        // Only the case changing is allowed: the name "taken" is its own.
        if !same_name(n.name(), name) && self.name_taken(dc, name)? {
            return Err(Error::Exists);
        }
        if n.name() == name {
            return Ok(loc);
        }
        self.reentry(&n, dc, name, scratch)
    }

    /// Writes `n`'s entry under `name` in folder `dc` (its attributes,
    /// cluster, size and dates kept) and deletes its old set.
    fn reentry(
        &mut self,
        n: &Node,
        dc: u32,
        name: &[u16],
        scratch: &mut [u8],
    ) -> Result<u64, Error> {
        let (lba, slot) = split(n.entry);
        self.dev.read_blocks(lba, &mut self.buf)?;
        let old: [u8; 32] = self.buf[slot * 32..slot * 32 + 32]
            .try_into()
            .unwrap_or([0; 32]);
        let size = le32(&old, 28);
        let (new, grown) = self.add_set16(
            dc,
            name,
            old[11],
            n.first_cluster,
            size,
            None,
            None,
            scratch,
        )?;
        let mut ns = Node::default();
        self.read_set(new, &mut ns)?;
        let (el, es) = split(ns.entry);
        self.dev.read_blocks(el, &mut self.buf)?;
        self.buf[es * 32 + 13..es * 32 + 20].copy_from_slice(&old[13..20]);
        self.buf[es * 32 + 22..es * 32 + 26].copy_from_slice(&old[22..26]);
        self.dev.write_blocks(el, &self.buf)?;
        if grown > 0 {
            self.fsinfo_adjust(-i64::from(grown), None)?;
        }
        self.delete_set(n.loc)?;
        Ok(new)
    }

    /// Moves the object at `loc` from folder `from` into folder `dir` on
    /// the same volume (its entry rewritten there, no data moved; a
    /// folder's `..` follows); returns the new location.
    ///
    /// # Errors
    /// [`Error::Exists`] (the name is taken there, or a folder would go
    /// into itself), `loc` not in `from`, or the card failed.
    pub fn node_move(
        &mut self,
        from: u64,
        loc: u64,
        dir: u64,
        scratch: &mut [u8],
    ) -> Result<u64, Error> {
        let mut n = Node::default();
        self.node_stat(loc, &mut n)?;
        let src = self.holding_folder(from, n.entry)?;
        let dc = self.folder_cluster(dir)?;
        if dc == src {
            return Ok(loc);
        }
        if n.folder && (dc == n.first_cluster || self.is_below(dc, n.first_cluster)?) {
            return Err(Error::Exists);
        }
        if self.name_taken(dc, n.name())? {
            return Err(Error::Exists);
        }
        let name = n.name;
        let new = self.reentry(&n, dc, &name[..n.name_len], scratch)?;
        if n.folder {
            // Its `..` now names the new parent (0 for the root).
            let up = if dc == self.root_cluster { 0 } else { dc };
            let lba = self.cluster_lba(n.first_cluster);
            self.dev.read_blocks(lba, &mut self.buf)?;
            if &self.buf[32..43] == b"..         " {
                self.buf[32 + 20..32 + 22].copy_from_slice(&((up >> 16) as u16).to_le_bytes());
                self.buf[32 + 26..32 + 28].copy_from_slice(&(up as u16).to_le_bytes());
                self.dev.write_blocks(lba, &self.buf)?;
            }
        }
        Ok(new)
    }

    /// Whether folder cluster `c` lies under folder cluster `top` (by `..`).
    fn is_below(&mut self, mut c: u32, top: u32) -> Result<bool, Error> {
        for _ in 0..64 {
            if c == top {
                return Ok(true);
            }
            if c == self.root_cluster || c == 0 || !self.valid_cluster(c) {
                return Ok(false);
            }
            self.dev.read_blocks(self.cluster_lba(c), &mut self.buf)?;
            if &self.buf[32..43] != b"..         " {
                return Ok(false);
            }
            c = crate::entry_first(&self.buf[32..64], self.kind);
        }
        Ok(false)
    }

    /// Reads file bytes from `offset` into `buf`; returns how many.
    ///
    /// # Errors
    /// The card failed or the chain is damaged.
    pub fn node_read(
        &mut self,
        n: &Node,
        offset: u64,
        buf: &mut [u8],
        scratch: &mut [u8],
    ) -> Result<usize, Error> {
        let file = crate::File {
            entry_lba: 0,
            entry_index: 0,
            first_cluster: n.first_cluster,
            size: u32::try_from(n.size).map_err(|_| Error::Corrupt)?,
        };
        let mut got = 0usize;
        self.read_range(&file, offset, buf.len() as u64, scratch, |piece| {
            buf[got..got + piece.len()].copy_from_slice(piece);
            got += piece.len();
        })?;
        Ok(got)
    }

    /// Capacity and free space in bytes (free from FSInfo when it knows,
    /// else counted).
    ///
    /// # Errors
    /// The card failed.
    pub fn node_space(&mut self, scratch: &mut [u8]) -> Result<(u64, u64), Error> {
        let cb = self.cluster_bytes() as u64;
        let mut free = None;
        if let Some(lba) = self.fsinfo_lba {
            self.dev.read_blocks(lba, &mut self.buf)?;
            if le32(&self.buf, 0) == 0x4161_5252 && le32(&self.buf, 484) == 0x6141_7272 {
                let f = le32(&self.buf, 488);
                if f <= self.clusters {
                    free = Some(f);
                }
            }
        }
        let free = match free {
            Some(f) => f,
            None => self.free_clusters(scratch)?,
        };
        Ok((u64::from(self.clusters) * cb, u64::from(free) * cb))
    }

    /// The largest file the volume holds: 4 GiB - 1 (FAT's 32-bit size).
    #[must_use]
    pub fn node_max_file(&self) -> u64 {
        u64::from(u32::MAX)
    }
}

/// A `.` or `..` entry.
fn dot_entry(name: &[u8; 11], cluster: u32) -> [u8; 32] {
    let mut e = [0u8; 32];
    e[..11].copy_from_slice(name);
    e[11] = ATTR_DIRECTORY;
    e[20..22].copy_from_slice(&((cluster >> 16) as u16).to_le_bytes());
    e[26..28].copy_from_slice(&(cluster as u16).to_le_bytes());
    e
}
