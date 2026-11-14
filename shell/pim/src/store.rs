//! The organiser's one store (docs/design/pim-store.md): appointments,
//! contacts, tasks and notes as records in RAM, kept on the card as a log
//! in two fixed files that never change size.
//!
//! - **In RAM:** the records as they were written, one after another, in
//!   the caller's memory (the arena), and an index of the live ones (the
//!   newest of each id) after it. Nothing is changed in place: a change
//!   appends the record's new version, a removal appends a tombstone.
//! - **On the card:** two slots of [`SLOT_SECTORS`] sectors. A slot is a
//!   header sector (its generation) and then groups: 16 bytes of header
//!   (magic, generation, length, CRC-32) and the arena's bytes, padded to
//!   whole sectors. A change is one group at the slot's tail, usually one
//!   sector. A group that is torn, damaged or of another generation ends
//!   the log, so a power cut loses at most the change being written.
//! - **When a slot or the arena fills:** the live records are packed and
//!   written to the other slot, whose header (a generation one higher) goes
//!   last. Until that header is on the card, the old slot is the truth.
//!
//! The store does no I/O: [`Store::load_want`] and [`Store::next_write`]
//! say which sector to read or write, and the caller (the device's loop,
//! a test's card image) does it, a sector a pass.

/// Bytes in a sector.
pub const SECTOR: usize = 512;
/// Sectors in a slot file (1 MiB).
pub const SLOT_SECTORS: u32 = 2048;
/// The index's room: live records in all.
pub const INDEX_MAX: usize = 2048;
/// The longest field (a note's text).
pub const TEXT_MAX: usize = 2000;
/// Bytes of the caller's memory the index takes (the arena gets the rest).
pub const INDEX_BYTES: usize = 8 * INDEX_MAX;
/// The memory a store is given on the device: a 256 KiB arena and the index.
pub const MEMORY: usize = 256 * 1024 + INDEX_BYTES;

const GROUP_HEAD: usize = 16;
/// A group's payload at most: eight sectors less its header.
const GROUP_MAX: usize = 8 * SECTOR - GROUP_HEAD;
const REC_HEAD: usize = 8;
const GROUP_MAGIC: u32 = 0x3147_5052; // "RPG1"
const SLOT_MAGIC: &[u8; 8] = b"RPPIMS01";
const DELETED: u8 = 1;

/// What a record is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// A calendar appointment.
    Appointment = 1,
    /// A contact.
    Contact = 2,
    /// A task.
    Task = 3,
    /// A note.
    Note = 4,
    /// A calendar (version 2): its name, colour and where it syncs from.
    Calendar = 5,
    /// A task list (version 2): its name and colour.
    List = 6,
}

impl Kind {
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    const fn of(b: u8) -> Option<Self> {
        Some(match b {
            1 => Self::Appointment,
            2 => Self::Contact,
            3 => Self::Task,
            4 => Self::Note,
            5 => Self::Calendar,
            6 => Self::List,
            _ => return None,
        })
    }
}

/// The fields' tags. Text is UTF-8; a number is four bytes, little-endian.
/// A reader skips tags it doesn't know, so later versions can add fields.
pub mod tag {
    /// Appointment and task: what it is.
    pub const TITLE: u8 = 1;
    /// Appointment: where.
    pub const PLACE: u8 = 2;
    /// Appointment, contact, task: free text.
    pub const NOTE: u8 = 3;
    /// Appointment: its start, local minutes from 1970.
    pub const START: u8 = 4;
    /// Appointment: how long, minutes.
    pub const MINUTES: u8 = 5;
    /// Appointment: 1 when it takes the whole day.
    pub const ALL_DAY: u8 = 6;
    /// Appointment: remind this many minutes before (absent: no reminder).
    pub const REMIND: u8 = 7;
    /// Contact: first name.
    pub const FIRST: u8 = 10;
    /// Contact: last name.
    pub const LAST: u8 = 11;
    /// Contact: company.
    pub const COMPANY: u8 = 12;
    /// Contact: mobile phone.
    pub const MOBILE: u8 = 13;
    /// Contact: home phone.
    pub const HOME: u8 = 14;
    /// Contact: work phone.
    pub const WORK: u8 = 15;
    /// Contact: e-mail address.
    pub const EMAIL: u8 = 16;
    /// Contact: postal address.
    pub const ADDRESS: u8 = 17;
    /// Task: the day it is due (days from 1970; absent: no date).
    pub const DUE: u8 = 20;
    /// Task: 0 low, 1 normal, 2 high.
    pub const PRIORITY: u8 = 21;
    /// Task: when it was completed, local minutes (absent: open).
    pub const DONE: u8 = 22;
    /// Note: its text.
    pub const TEXT: u8 = 30;
    /// Note: when it was last changed, local minutes.
    pub const CHANGED: u8 = 31;

    // Version 2 (2026-11-14; a reader of version 1 skips them).
    /// Appointment: its calendar's record id.
    pub const CAL: u8 = 8;
    /// Appointment, task: how it repeats ([`crate::repeat::Rule`], packed).
    pub const REPEAT: u8 = 9;
    /// Contact: 1 when a favourite.
    pub const FAVOURITE: u8 = 18;
    /// Contact: the birthday, days from 1970 (the year 0004 when unknown).
    pub const BIRTHDAY: u8 = 19;
    /// Task: its list's record id.
    pub const LIST: u8 = 23;
    /// Task: the task it is a step of.
    pub const PARENT: u8 = 24;
    /// Task: remind at, local minutes.
    pub const REMIND_AT: u8 = 25;
    /// Note, calendar, list: its identity colour (0-7).
    pub const COLOUR: u8 = 32;
    /// Note: 1 when pinned.
    pub const PINNED: u8 = 33;
    /// Note: handwriting, strokes as packed points (`rp_app_notes`).
    pub const INK: u8 = 34;
    /// Calendar, list: its name.
    pub const NAME: u8 = 35;
    /// Calendar: where it syncs from (an iCal link, a CalDAV address, or
    /// "google"; absent: on this device only).
    pub const SOURCE: u8 = 36;
}

/// A record as stored: read-only.
#[derive(Clone, Copy, Debug)]
pub struct Rec<'a> {
    /// What it is.
    pub kind: Kind,
    /// Its id: the same through every change.
    pub id: u32,
    body: &'a [u8],
}

impl<'a> Rec<'a> {
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    fn field(&self, want: u8) -> Option<&'a [u8]> {
        let mut b = self.body;
        while let [t, l0, l1, rest @ ..] = b {
            let n = usize::from(u16::from_le_bytes([*l0, *l1]));
            let (data, after) = rest.split_at_checked(n)?;
            if *t == want {
                return Some(data);
            }
            b = after;
        }
        None
    }

    /// A text field (empty when absent).
    #[must_use]
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub fn text(&self, tag: u8) -> &'a [u8] {
        self.field(tag).unwrap_or(&[])
    }

    /// A number field.
    #[must_use]
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub fn num(&self, tag: u8) -> Option<u32> {
        let d: [u8; 4] = self.field(tag)?.try_into().ok()?;
        Some(u32::from_le_bytes(d))
    }
}

/// Why a change wasn't taken.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    /// A field over [`TEXT_MAX`], or a record too long for a group.
    TooLong,
    /// No room: the arena or the index is full of live records.
    Full,
    /// No record of that id.
    NotFound,
    /// The card's records are still being read: nothing changes until
    /// [`Store::loaded`].
    Loading,
}

/// A sector of a slot file.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct At {
    /// The slot file: 0 or 1.
    pub slot: u8,
    /// The sector inside it.
    pub sector: u32,
}

#[derive(Clone, Copy, Debug)]
struct Group {
    len: usize,
    sectors: u32,
    sent: u32,
    crc: u32,
}

#[derive(Clone, Copy, Debug)]
struct Loading {
    /// The group being read: its length, CRC, and the bytes had so far.
    group: Option<(usize, u32, usize)>,
}

/// The store.
pub struct Store<'a> {
    buf: &'a mut [u8],
    arena: usize,
    used: usize,
    n: usize,
    next_id: u32,
    slot_sectors: u32,
    /// The slot the card's truth is in, its generation, and where its log
    /// ends.
    active: u8,
    generation: u32,
    tail: u32,
    /// Arena bytes on the card (in `target`'s slot).
    flushed: usize,
    /// The slot being written when it isn't `active` yet, and its
    /// generation: its header goes last.
    switching: Option<(u8, u32)>,
    group: Option<Group>,
    loading: Option<Loading>,
    loaded: bool,
    failures: u8,
    changes: u32,
}

/// CRC-32 (IEEE), bit by bit: groups are a few kilobytes at most.
#[must_use]
#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
pub fn crc32(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in data {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = (c >> 1) ^ (0xEDB8_8320 & 0u32.wrapping_sub(c & 1));
        }
    }
    !c
}

#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
fn le32(b: &[u8], at: usize) -> u32 {
    b.get(at..at + 4)
        .and_then(|s| s.try_into().ok())
        .map_or(0, u32::from_le_bytes)
}

/// The record at `p` of `b`: (kind, flags, id, its whole length), checked
/// as hostile input (a known kind, a body inside `b` whose fields end
/// exactly at its end).
#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
fn rec_at(b: &[u8], p: usize) -> Option<(Kind, u8, u32, usize)> {
    let h = b.get(p..p + REC_HEAD)?;
    let kind = Kind::of(h[0])?;
    let len = usize::from(u16::from_le_bytes([h[2], h[3]]));
    let body = b.get(p + REC_HEAD..p + REC_HEAD + len)?;
    let mut f = body;
    while !f.is_empty() {
        let [_, l0, l1, rest @ ..] = f else {
            return None;
        };
        f = rest.get(usize::from(u16::from_le_bytes([*l0, *l1]))..)?;
    }
    Some((kind, h[1], le32(h, 4), REC_HEAD + len))
}

/// The slot whose header is valid with the higher generation, if any.
#[must_use]
#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
pub fn pick(headers: [&[u8; SECTOR]; 2]) -> Option<(u8, u32)> {
    let generation = |h: &[u8; SECTOR]| {
        (h[..8] == SLOT_MAGIC[..] && crc32(&h[..12]) == le32(h, 12) && le32(h, 8) != 0)
            .then(|| le32(h, 8))
    };
    match (generation(headers[0]), generation(headers[1])) {
        (Some(a), Some(b)) if b > a => Some((1, b)),
        (Some(a), _) => Some((0, a)),
        (None, Some(b)) => Some((1, b)),
        (None, None) => None,
    }
}

impl<'a> Store<'a> {
    /// A store in `memory` ([`MEMORY`] bytes on the device; at least
    /// [`INDEX_BYTES`] and a sector), empty and not yet loaded.
    #[must_use]
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub fn new(memory: &'a mut [u8]) -> Self {
        Self::sized(memory, SLOT_SECTORS)
    }

    /// As [`Store::new`], with slots of `slot_sectors` (tests fill a small
    /// one).
    #[must_use]
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub fn sized(memory: &'a mut [u8], slot_sectors: u32) -> Self {
        let arena = memory.len().saturating_sub(INDEX_BYTES);
        Self {
            buf: memory,
            arena,
            used: 0,
            n: 0,
            next_id: 1,
            slot_sectors,
            active: 0,
            generation: 0,
            tail: 1,
            flushed: 0,
            switching: None,
            group: None,
            loading: None,
            loaded: false,
            failures: 0,
            changes: 0,
        }
    }

    // ---- The index ----------------------------------------------------

    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    fn entry(&self, i: usize) -> (usize, u32) {
        let at = self.arena + 8 * i;
        (le32(self.buf, at) as usize, le32(self.buf, at + 4))
    }

    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    fn set_entry(&mut self, i: usize, off: usize, id: u32) {
        let at = self.arena + 8 * i;
        self.buf[at..at + 4].copy_from_slice(&(off as u32).to_le_bytes());
        self.buf[at + 4..at + 8].copy_from_slice(&id.to_le_bytes());
    }

    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    fn find(&self, id: u32) -> Option<usize> {
        (0..self.n).find(|&i| self.entry(i).1 == id)
    }

    /// Takes the record at `off` into the index: the newest of its id, or
    /// its removal.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    fn index(&mut self, off: usize, flags: u8, id: u32) -> Result<(), Error> {
        self.next_id = self.next_id.max(id.saturating_add(1));
        match (self.find(id), flags & DELETED != 0) {
            (Some(i), true) => {
                let last = self.entry(self.n - 1);
                self.set_entry(i, last.0, last.1);
                self.n -= 1;
            }
            (Some(i), false) => self.set_entry(i, off, id),
            (None, true) => {}
            (None, false) => {
                if self.n == INDEX_MAX {
                    return Err(Error::Full);
                }
                self.set_entry(self.n, off, id);
                self.n += 1;
            }
        }
        Ok(())
    }

    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    fn rec(&self, off: usize) -> Option<Rec<'_>> {
        let (kind, _, id, len) = rec_at(&self.buf[..self.used], off)?;
        Some(Rec {
            kind,
            id,
            body: &self.buf[off + REC_HEAD..off + len],
        })
    }

    // ---- Reading ------------------------------------------------------

    /// The record of `id`.
    #[must_use]
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub fn get(&self, id: u32) -> Option<Rec<'_>> {
        self.rec(self.entry(self.find(id)?).0)
    }

    /// Every live record of `kind`, in no particular order.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub fn all(&self, kind: Kind) -> impl Iterator<Item = Rec<'_>> {
        (0..self.n)
            .filter_map(|i| self.rec(self.entry(i).0))
            .filter(move |r| r.kind == kind)
    }

    /// Every live record of `kind` with its handle: where it lies now, for
    /// a view's table. A handle is good until the next change
    /// ([`Store::changes`]); [`Store::at`] reads it back.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub fn handles(&self, kind: Kind) -> impl Iterator<Item = (u32, Rec<'_>)> {
        (0..self.n)
            .filter_map(|i| {
                let off = self.entry(i).0;
                Some((off as u32, self.rec(off)?))
            })
            .filter(move |(_, r)| r.kind == kind)
    }

    /// The record at a handle from [`Store::handles`].
    #[must_use]
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub fn at(&self, handle: u32) -> Option<Rec<'_>> {
        self.rec(handle as usize)
    }

    /// Live records in all.
    #[must_use]
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub const fn len(&self) -> usize {
        self.n
    }

    /// No live record.
    #[must_use]
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub const fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// A count that changes with every change to the records: a view made
    /// at one count is stale at another.
    #[must_use]
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub const fn changes(&self) -> u32 {
        self.changes
    }

    // ---- Changing -----------------------------------------------------

    /// The live records packed to the arena's start, and the card's other
    /// slot made the one to write (everything, then its header).
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    fn pack(&mut self) {
        let mut to = 0;
        let mut p = 0;
        while let Some((_, _, id, len)) = rec_at(&self.buf[..self.used], p) {
            if let Some(i) = self.find(id)
                && self.entry(i).0 == p
            {
                self.buf.copy_within(p..p + len, to);
                self.set_entry(i, to, id);
                to += len;
            }
            p += len;
        }
        self.used = to;
        self.flushed = 0;
        self.group = None;
        self.tail = 1;
        let (slot, generation) = match self.switching {
            // A switch under way starts again in its slot.
            Some(s) => s,
            None => (
                if self.generation == 0 {
                    0
                } else {
                    1 - self.active
                },
                self.generation + 1,
            ),
        };
        self.switching = Some((slot, generation));
    }

    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    fn append(
        &mut self,
        kind: Kind,
        flags: u8,
        id: u32,
        fields: &[(u8, &[u8])],
    ) -> Result<(), Error> {
        if !self.loaded {
            return Err(Error::Loading);
        }
        if fields.iter().any(|f| f.1.len() > TEXT_MAX) {
            return Err(Error::TooLong);
        }
        let body: usize = fields.iter().map(|f| 3 + f.1.len()).sum();
        let len = REC_HEAD + body;
        if len > GROUP_MAX {
            return Err(Error::TooLong);
        }
        if self.used + len > self.arena {
            self.pack();
            if self.used + len > self.arena {
                return Err(Error::Full);
            }
        }
        let p = self.used;
        let b = &mut self.buf[p..p + len];
        b[0] = kind as u8;
        b[1] = flags;
        b[2..4].copy_from_slice(&(body as u16).to_le_bytes());
        b[4..8].copy_from_slice(&id.to_le_bytes());
        let mut at = REC_HEAD;
        for (t, d) in fields {
            b[at] = *t;
            b[at + 1..at + 3].copy_from_slice(&(d.len() as u16).to_le_bytes());
            b[at + 3..at + 3 + d.len()].copy_from_slice(d);
            at += 3 + d.len();
        }
        self.index(p, flags, id)?;
        self.used += len;
        self.changes = self.changes.wrapping_add(1);
        Ok(())
    }

    /// Adds a record (`id` `None`) or replaces `id`'s with these fields
    /// (empty ones are left out). Its id.
    ///
    /// # Errors
    /// [`Error::TooLong`], [`Error::Full`], [`Error::Loading`], or
    /// [`Error::NotFound`] for an `id` that isn't live.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub fn put(
        &mut self,
        kind: Kind,
        id: Option<u32>,
        fields: &[(u8, &[u8])],
    ) -> Result<u32, Error> {
        let id = match id {
            Some(id) => {
                self.find(id).ok_or(Error::NotFound)?;
                id
            }
            None => {
                if self.n == INDEX_MAX {
                    return Err(Error::Full);
                }
                self.next_id
            }
        };
        // Empty fields aren't stored: at most 16 a record.
        let mut kept = [(0u8, &[][..]); 16];
        let mut k = 0;
        for f in fields.iter().filter(|f| !f.1.is_empty()) {
            *kept.get_mut(k).ok_or(Error::TooLong)? = *f;
            k += 1;
        }
        self.append(kind, 0, id, &kept[..k])?;
        Ok(id)
    }

    /// Removes `id`'s record.
    ///
    /// # Errors
    /// [`Error::NotFound`], or [`Error::Full`] with no room for its
    /// tombstone even after packing.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub fn remove(&mut self, id: u32) -> Result<(), Error> {
        let kind = self.get(id).ok_or(Error::NotFound)?.kind;
        self.append(kind, DELETED, id, &[])
    }

    // ---- The card -----------------------------------------------------

    /// The sector to read next while loading, `None` once loaded. Before
    /// the first, the caller reads both slots' sector 0 and calls
    /// [`Store::load_start`] with [`pick`]'s answer.
    #[must_use]
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub fn load_want(&self) -> Option<At> {
        let l = self.loading?;
        let within = l
            .group
            .map_or(0, |(_, _, got)| ((GROUP_HEAD + got) / SECTOR) as u32);
        Some(At {
            slot: self.active,
            sector: self.tail + within,
        })
    }

    /// Starts the load from the slot [`pick`] chose; with none (a new card,
    /// or both headers damaged) the store starts empty in slot 0.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub fn load_start(&mut self, picked: Option<(u8, u32)>) {
        self.used = 0;
        self.n = 0;
        self.flushed = 0;
        self.tail = 1;
        self.group = None;
        match picked {
            Some((slot, generation)) => {
                self.active = slot;
                self.generation = generation;
                self.switching = None;
                self.loading = Some(Loading { group: None });
                self.loaded = false;
            }
            None => {
                self.active = 0;
                self.generation = 0;
                self.switching = Some((0, 1));
                self.loading = None;
                self.loaded = true;
            }
        }
    }

    /// Whether the load is over: until then records can be read as they
    /// arrive, and nothing can be changed.
    #[must_use]
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub const fn loaded(&self) -> bool {
        self.loaded
    }

    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    fn load_done(&mut self) {
        self.loading = None;
        self.loaded = true;
        if self.switching.is_some() {
            // Packed while loading: the other slot is written from its start.
            self.tail = 1;
            self.flushed = 0;
        }
    }

    /// The sector [`Store::load_want`] asked for (`None`: it couldn't be
    /// read, which ends the log there).
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub fn load_sector(&mut self, sector: Option<&[u8; SECTOR]>) {
        let (Some(l), Some(s)) = (self.loading, sector) else {
            return self.load_done();
        };
        let (len, crc, got, from) = match l.group {
            Some((len, crc, got)) => (len, crc, got, 0),
            None => {
                let len = le32(s, 8) as usize;
                if le32(s, 0) != GROUP_MAGIC
                    || le32(s, 4) != self.generation
                    || len == 0
                    || len > GROUP_MAX
                    || self.tail + (GROUP_HEAD + len).div_ceil(SECTOR) as u32 > self.slot_sectors
                {
                    return self.load_done();
                }
                if self.used + len > self.arena {
                    // The log holds more old versions than the arena has
                    // room for: pack what is live and go on. The card is
                    // rewritten afterwards.
                    let (tail, active, generation) = (self.tail, self.active, self.generation);
                    self.pack();
                    self.tail = tail;
                    self.switching = Some((1 - active, generation + 1));
                    if self.used + len > self.arena {
                        return self.load_done();
                    }
                }
                (len, le32(s, 12), 0, GROUP_HEAD)
            }
        };
        let take = (len - got).min(SECTOR - from);
        self.buf[self.used + got..self.used + got + take].copy_from_slice(&s[from..from + take]);
        let got = got + take;
        if got < len {
            self.loading = Some(Loading {
                group: Some((len, crc, got)),
            });
            return;
        }
        // The whole group: its CRC, then its records one by one.
        let end = self.used + len;
        if crc32(&self.buf[self.used..end]) != crc {
            return self.load_done();
        }
        let mut p = self.used;
        while p < end {
            let Some((_, flags, id, n)) = rec_at(&self.buf[..end], p) else {
                return self.load_done();
            };
            if self.index(p, flags, id).is_err() {
                return self.load_done();
            }
            p += n;
        }
        self.used = end;
        if self.switching.is_none() {
            self.flushed = end;
        }
        self.tail += (GROUP_HEAD + len).div_ceil(SECTOR) as u32;
        self.loading = Some(Loading { group: None });
        self.changes = self.changes.wrapping_add(1);
    }

    /// Whether changes wait to reach the card.
    #[must_use]
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub const fn dirty(&self) -> bool {
        self.loaded && (self.flushed < self.used || self.switching.is_some())
    }

    /// Whether the card stopped taking writes (three failed in a row): the
    /// records stay in RAM, and [`Store::retry`] tries again.
    #[must_use]
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub const fn stalled(&self) -> bool {
        self.failures >= 3
    }

    /// Tries the card again after a stall (a card put back).
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub const fn retry(&mut self) {
        self.failures = 0;
    }

    /// The next sector to write, into `out`: `None` with nothing to write
    /// (or before the load is over, or stalled). The caller writes it and
    /// calls [`Store::written`].
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub fn next_write(&mut self, out: &mut [u8; SECTOR]) -> Option<At> {
        if !self.loaded || self.stalled() {
            return None;
        }
        let (slot, generation) = self.switching.unwrap_or((self.active, self.generation));
        if self.group.is_none() && self.flushed < self.used {
            // The next group: whole records up to its room.
            let mut len = 0;
            while let Some((_, _, _, n)) = rec_at(&self.buf[..self.used], self.flushed + len) {
                if len + n > GROUP_MAX {
                    break;
                }
                len += n;
            }
            let sectors = (GROUP_HEAD + len).div_ceil(SECTOR) as u32;
            if len == 0 {
                return None;
            }
            if self.tail + sectors > self.slot_sectors {
                // The slot is full: everything live goes to the other one.
                if self.switching.is_some() && self.flushed == 0 {
                    // Even packed it doesn't fit a slot.
                    return None;
                }
                self.pack();
                return self.next_write(out);
            }
            self.group = Some(Group {
                len,
                sectors,
                sent: 0,
                crc: crc32(&self.buf[self.flushed..self.flushed + len]),
            });
        }
        out.fill(0);
        if let Some(g) = self.group {
            // Sector `sent` of [header, the arena's bytes, zeros].
            let start = g.sent as usize * SECTOR;
            let mut head = [0u8; GROUP_HEAD];
            head[..4].copy_from_slice(&GROUP_MAGIC.to_le_bytes());
            head[4..8].copy_from_slice(&generation.to_le_bytes());
            head[8..12].copy_from_slice(&(g.len as u32).to_le_bytes());
            head[12..].copy_from_slice(&g.crc.to_le_bytes());
            for (k, o) in out.iter_mut().enumerate() {
                let at = start + k;
                *o = if at < GROUP_HEAD {
                    head[at]
                } else if at - GROUP_HEAD < g.len {
                    self.buf[self.flushed + at - GROUP_HEAD]
                } else {
                    0
                };
            }
            return Some(At {
                slot,
                sector: self.tail + g.sent,
            });
        }
        // Everything is in the new slot: its header makes it the truth.
        self.switching?;
        out[..8].copy_from_slice(SLOT_MAGIC);
        out[8..12].copy_from_slice(&generation.to_le_bytes());
        let crc = crc32(&out[..12]);
        out[12..16].copy_from_slice(&crc.to_le_bytes());
        Some(At { slot, sector: 0 })
    }

    /// How the write [`Store::next_write`] asked for went.
    #[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
    pub fn written(&mut self, ok: bool) {
        if !ok {
            // The group starts again (a torn one ends the log anyway).
            self.failures = self.failures.saturating_add(1);
            if let Some(g) = self.group.as_mut() {
                g.sent = 0;
            }
            return;
        }
        self.failures = 0;
        match self.group {
            Some(mut g) => {
                g.sent += 1;
                if g.sent == g.sectors {
                    self.flushed += g.len;
                    self.tail += g.sectors;
                    self.group = None;
                } else {
                    self.group = Some(g);
                }
            }
            None => {
                if let Some((slot, generation)) = self.switching.take() {
                    self.active = slot;
                    self.generation = generation;
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
