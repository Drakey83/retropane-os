//! FAT12/16/32 and exFAT cards as MTP stores (ADR-0037 rule 1): one
//! [`Device`] for every card, on `rp-fat`'s node layer, so the loader now
//! and the storage service later serve cards the same way. What differs
//! per owner (its disks, the boot-file guard, its screen for a format's
//! yes, its name and icon) comes through [`Cards`].
//!
//! Each call mounts the card afresh (one boot-sector read): nothing about
//! a card is cached across calls but its free space, so a card swapped
//! between two operations is never written from a stale view.

use rp_fat::node::{NAME_UNITS, Node, Reservation, Stamp};
use rp_fat::{AnyVolume, Error};
use rp_hal_api::{BlockDevice, BlockError};

use crate::data::{Text, Time};
use crate::device::{Consent, Device, Entry, Kind, Loc, NewObject, StoreError, StoreInfo};

/// Most stores (SD, CF, the internal flash).
pub const MAX_STORES: usize = 3;

/// What the owner of the cards provides.
pub trait Cards {
    /// A card's block device (writes through the owner's guard).
    type Disk<'a>: BlockDevice
    where
        Self: 'a;

    /// How many store slots.
    fn stores(&self) -> u8;

    /// Store `s`'s card and its volume's first sector, `None` with no
    /// medium or when it isn't offered.
    fn disk(&mut self, s: u8) -> Option<(Self::Disk<'_>, u32)>;

    /// Built in or removable.
    fn kind(&self, s: u8) -> Kind;

    /// Whether the PC may change anything on it (a locked card, a card
    /// with no guard: read-only).
    fn writable(&self, s: u8) -> bool;

    /// Whether the entry at `entry` (sector, byte offset of its short or
    /// file entry) is one the device boots from.
    fn protected(&self, s: u8, sector: u32, offset: usize) -> bool;

    /// Whether the device boots from this card.
    fn boots_from(&mut self, s: u8) -> bool;

    /// "SD Card".
    fn description(&self, s: u8) -> &'static str;

    /// Asks the user for a format's yes ([`Device::format_consent`]).
    fn format_consent(&mut self, s: u8, start: bool) -> Consent;

    /// Formats the card (ADR-0036).
    ///
    /// # Errors
    /// The card failed.
    fn format(&mut self, s: u8) -> Result<(), StoreError>;

    /// The device's name for the PC.
    fn friendly_name(&self) -> Text;

    /// The PC renamed the device.
    fn set_friendly_name(&mut self, name: &Text);

    /// The device icon's length (0: none).
    fn icon_len(&mut self) -> u32 {
        0
    }

    /// Reads the icon.
    fn icon_read(&mut self, offset: u32, buf: &mut [u8]) -> usize {
        let _ = (offset, buf);
        0
    }

    /// The PC said what it is.
    fn initiator(&mut self, who: &Text) {
        let _ = who;
    }

    /// Milliseconds since some fixed point.
    fn now_ms(&self) -> u32;
}

/// Borrowed cards are cards (the loader lends its own for each poll).
impl<C: Cards + ?Sized> Cards for &mut C {
    type Disk<'a>
        = C::Disk<'a>
    where
        Self: 'a;

    fn stores(&self) -> u8 {
        (**self).stores()
    }
    fn disk(&mut self, s: u8) -> Option<(Self::Disk<'_>, u32)> {
        (**self).disk(s)
    }
    fn kind(&self, s: u8) -> Kind {
        (**self).kind(s)
    }
    fn writable(&self, s: u8) -> bool {
        (**self).writable(s)
    }
    fn protected(&self, s: u8, sector: u32, offset: usize) -> bool {
        (**self).protected(s, sector, offset)
    }
    fn boots_from(&mut self, s: u8) -> bool {
        (**self).boots_from(s)
    }
    fn description(&self, s: u8) -> &'static str {
        (**self).description(s)
    }
    fn format_consent(&mut self, s: u8, start: bool) -> Consent {
        (**self).format_consent(s, start)
    }
    fn format(&mut self, s: u8) -> Result<(), StoreError> {
        (**self).format(s)
    }
    fn friendly_name(&self) -> Text {
        (**self).friendly_name()
    }
    fn set_friendly_name(&mut self, name: &Text) {
        (**self).set_friendly_name(name);
    }
    fn icon_len(&mut self) -> u32 {
        (**self).icon_len()
    }
    fn icon_read(&mut self, offset: u32, buf: &mut [u8]) -> usize {
        (**self).icon_read(offset, buf)
    }
    fn initiator(&mut self, who: &Text) {
        (**self).initiator(who);
    }
    fn now_ms(&self) -> u32 {
        (**self).now_ms()
    }
}

/// A file being sent: its reservation and what it will be called.
#[derive(Clone, Copy)]
struct Pending {
    r: Reservation,
    dir: Loc,
    name: [u16; NAME_UNITS],
    name_len: usize,
    hidden: bool,
    created: Option<Stamp>,
    modified: Option<Stamp>,
}

/// What the device keeps about the cards between operations: a file being
/// sent, free space and serials. Kept by the owner (in static memory, not
/// on its stack), so a [`FatDevice`] can be put together for each poll
/// while the owner uses its cards between polls.
#[derive(Clone, Copy)]
pub struct FatState {
    pending: [Option<Pending>; MAX_STORES],
    space: [Option<(u64, u64)>; MAX_STORES],
    serials: [Option<u32>; MAX_STORES],
}

impl FatState {
    /// Nothing known yet.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            pending: [None; MAX_STORES],
            space: [None; MAX_STORES],
            serials: [None; MAX_STORES],
        }
    }

    /// Forget what is cached about store `s` (its card changed under us:
    /// the device wrote it, or it was swapped). A reservation on it is
    /// dropped (the next mount's recovery frees its clusters).
    pub fn forget(&mut self, s: u8) {
        if let Some(i) = Some(usize::from(s)).filter(|&i| i < MAX_STORES) {
            self.space[i] = None;
            self.pending[i] = None;
            self.serials[i] = None;
        }
    }
}

impl FatState {
    /// Store `s`'s free space changed under us (the owner wrote a file):
    /// read again at the next GetStorageInfo; a file being sent stays.
    pub fn space_changed(&mut self, s: u8) {
        if let Some(i) = Some(usize::from(s)).filter(|&i| i < MAX_STORES) {
            self.space[i] = None;
        }
    }
}

impl Default for FatState {
    fn default() -> Self {
        Self::new()
    }
}

/// The cards as MTP stores.
pub struct FatDevice<'s, C> {
    cards: C,
    scratch: &'s mut [u8],
    st: &'s mut FatState,
}

fn err(e: Error) -> StoreError {
    match e {
        Error::NotFound => StoreError::NotFound,
        Error::NoRoom | Error::Fragmented => StoreError::Full,
        Error::BadName => StoreError::BadName,
        Error::Exists => StoreError::Exists,
        Error::NotEmpty => StoreError::NotEmpty,
        Error::NotFat32 | Error::NotExFat => StoreError::Absent,
        // A write the guard refused touched a boot file.
        Error::Device(BlockError::ReadOnly) => StoreError::Protected,
        Error::Device(_) | Error::Corrupt => StoreError::Device,
    }
}

fn time_of(s: Option<Stamp>) -> Option<Time> {
    s.map(|s| Time {
        year: s.year,
        month: s.month,
        day: s.day,
        hour: s.hour,
        minute: s.minute,
        second: s.second,
        tenths: s.centis.map(|c| c / 10),
        zone: s.zone,
    })
}

fn stamp_of(t: Option<Time>) -> Option<Stamp> {
    t.map(|t| Stamp {
        year: t.year,
        month: t.month,
        day: t.day,
        hour: t.hour,
        minute: t.minute,
        second: t.second,
        centis: t.tenths.map(|d| d * 10),
        zone: t.zone,
    })
}

/// The volume serial from a boot sector (FAT32 0x43, FAT12/16 0x27,
/// exFAT 100).
fn serial_of(boot: &[u8; 512]) -> u32 {
    let at = if &boot[3..11] == b"EXFAT   " {
        100
    } else if boot[82..87] == *b"FAT32" {
        0x43
    } else {
        0x27
    };
    u32::from_le_bytes([boot[at], boot[at + 1], boot[at + 2], boot[at + 3]])
}

impl<'s, C: Cards> FatDevice<'s, C> {
    /// Serves `cards`, staging card commands in `scratch` (whole sectors;
    /// 8 KiB or more keeps FAT searches short), with what is known about
    /// them in `st`.
    ///
    /// # Panics
    /// If `scratch` is under 4 KiB.
    pub fn new(cards: C, scratch: &'s mut [u8], st: &'s mut FatState) -> Self {
        assert!(scratch.len() >= 4096, "scratch too small");
        Self { cards, scratch, st }
    }

    /// The cards.
    pub fn cards(&mut self) -> &mut C {
        &mut self.cards
    }

    /// Forget what is cached about store `s` (its card changed under us:
    /// the device wrote it, or it was swapped). A reservation on it is
    /// dropped (the next mount's recovery frees its clusters).
    pub fn forget(&mut self, s: u8) {
        if self.slot(s).is_some() {
            self.st.forget(s);
        }
    }

    fn slot(&self, s: u8) -> Option<usize> {
        let i = usize::from(s);
        (i < MAX_STORES && s < self.cards.stores()).then_some(i)
    }

    /// Runs `f` on store `s`'s mounted volume.
    fn with<R>(
        &mut self,
        s: u8,
        f: impl FnOnce(&mut AnyVolume<C::Disk<'_>>, &mut [u8]) -> Result<R, Error>,
    ) -> Result<R, StoreError> {
        let scratch = &mut *self.scratch;
        let (disk, base) = self.cards.disk(s).ok_or(StoreError::Absent)?;
        let mut v = AnyVolume::mount(disk, base).map_err(err)?;
        f(&mut v, scratch).map_err(err)
    }

    fn entry_of(&self, s: u8, n: &Node, out: &mut Entry) {
        let (sector, slot) = ((n.entry >> 4) as u32, (n.entry & 15) as usize);
        let protected = self.cards.protected(s, sector, slot * 32);
        let units = n.name();
        let long = units.len() >= crate::data::STR_MAX;
        let mut cut = units.len().min(crate::data::STR_MAX - 1);
        // Don't split a surrogate pair.
        if long && (0xD800..0xDC00).contains(&units[cut - 1]) {
            cut -= 1;
        }
        out.loc = n.loc;
        out.name = Text::from_units(&units[..cut]).unwrap_or_default();
        out.long_name = long;
        out.folder = n.folder;
        out.size = n.size;
        out.created = time_of(n.created);
        out.modified = time_of(n.modified);
        out.hidden = n.attrs & 0x02 != 0 || protected;
        out.system = n.attrs & 0x04 != 0 || protected;
        out.protected = protected;
        out.uid = 0;
    }

    /// The persistent unique ID [MTP B.2.15], fields side by side: the
    /// volume's serial (32 bits), the store (8), whether the place is a
    /// cluster or an entry (1), the place (39: the first cluster, stable
    /// across renames and moves since the data doesn't move; an empty
    /// file, with none, by its entry's place), the creation time (48).
    fn uid(serial: u32, s: u8, n: &Node) -> u128 {
        let created = n.created.map_or(0u64, |c| {
            u64::from(c.year) << 36
                | u64::from(c.month) << 32
                | u64::from(c.day) << 24
                | u64::from(c.hour) << 16
                | u64::from(c.minute) << 8
                | u64::from(c.second) << 2
                | u64::from(c.centis.unwrap_or(0) / 25)
        });
        let (kind, place) = if n.first_cluster != 0 {
            (0u128, u64::from(n.first_cluster))
        } else {
            (1, n.entry)
        };
        u128::from(serial) << 96
            | u128::from(s) << 88
            | kind << 87
            | u128::from(place & ((1 << 39) - 1)) << 48
            | u128::from(created & ((1 << 48) - 1))
    }

    fn serial(&mut self, s: u8) -> u32 {
        let Some(i) = self.slot(s) else {
            return 0;
        };
        if let Some(v) = self.st.serials[i] {
            return v;
        }
        let Some((mut disk, base)) = self.cards.disk(s) else {
            return 0;
        };
        let mut boot = [0u8; 512];
        if disk.read_blocks(base, &mut boot).is_err() {
            return 0;
        }
        let v = serial_of(&boot);
        self.st.serials[i] = Some(v);
        v
    }

    fn check_change(&mut self, s: u8, loc: Loc) -> Result<(), StoreError> {
        if !self.cards.writable(s) {
            return Err(StoreError::ReadOnly);
        }
        let mut n = Node::default();
        self.with(s, |v, _| v.node_stat(loc, &mut n))?;
        let (sector, slot) = ((n.entry >> 4) as u32, (n.entry & 15) as usize);
        if self.cards.protected(s, sector, slot * 32) {
            return Err(StoreError::Protected);
        }
        Ok(())
    }

    fn changed(&mut self, s: u8) {
        if let Some(i) = self.slot(s) {
            self.st.space[i] = None;
        }
    }
}

impl<C: Cards> Device for FatDevice<'_, C> {
    fn stores(&self) -> u8 {
        self.cards.stores().min(MAX_STORES as u8)
    }

    fn info(&mut self, s: u8) -> Option<StoreInfo> {
        let i = self.slot(s)?;
        let (space, max_file) = match self.st.space[i] {
            Some(sp) => (sp, None),
            None => {
                let r = self
                    .with(s, |v, sc| Ok((v.node_space(sc)?, v.node_max_file())))
                    .ok()?;
                (r.0, Some(r.1))
            }
        };
        self.st.space[i] = Some(space);
        let max_file = match max_file {
            Some(m) => m,
            None => self.with(s, |v, _| Ok(v.node_max_file())).ok()?,
        };
        let serial = self.serial(s);
        let mut id = [0u8; 13];
        id[..4].copy_from_slice(&[b'S', b'0' + s, b'-', b'0']);
        for k in 0..8 {
            let nib = (serial >> (28 - 4 * k)) & 15;
            id[4 + k] = if nib < 10 {
                b'0' + nib as u8
            } else {
                b'A' + nib as u8 - 10
            };
        }
        let volume_id = core::str::from_utf8(&id[..12])
            .ok()
            .and_then(Text::from_text)
            .unwrap_or_default();
        Some(StoreInfo {
            kind: self.cards.kind(s),
            writable: self.cards.writable(s),
            capacity: space.0,
            free: space.1,
            description: Text::from_text(self.cards.description(s)).unwrap_or_default(),
            volume_id,
            max_file,
        })
    }

    fn child(
        &mut self,
        s: u8,
        dir: Loc,
        cursor: &mut u64,
        out: &mut Entry,
    ) -> Result<bool, StoreError> {
        let mut n = Node::default();
        let more = self.with(s, |v, _| v.node_child(dir, cursor, &mut n))?;
        if more {
            self.entry_of(s, &n, out);
            out.uid = Self::uid(self.serial(s), s, &n);
        }
        Ok(more)
    }

    fn stat(&mut self, s: u8, loc: Loc, out: &mut Entry) -> Result<(), StoreError> {
        let mut n = Node::default();
        self.with(s, |v, _| v.node_stat(loc, &mut n))?;
        self.entry_of(s, &n, out);
        out.uid = Self::uid(self.serial(s), s, &n);
        Ok(())
    }

    fn read(&mut self, s: u8, loc: Loc, offset: u64, buf: &mut [u8]) -> Result<usize, StoreError> {
        self.with(s, |v, sc| {
            let mut n = Node::default();
            v.node_stat(loc, &mut n)?;
            v.node_read(&n, offset, buf, sc)
        })
    }

    fn create(&mut self, s: u8, dir: Loc, new: &NewObject) -> Result<Loc, StoreError> {
        if !self.cards.writable(s) {
            return Err(StoreError::ReadOnly);
        }
        let i = self.slot(s).ok_or(StoreError::Absent)?;
        let name = new.name.units();
        let stamps = (stamp_of(new.created), stamp_of(new.modified));
        self.changed(s);
        if new.folder || new.size == 0 {
            return self.with(s, |v, sc| {
                v.node_create(dir, name, new.folder, new.hidden, stamps, sc)
            });
        }
        // Too big for the file system: refused before anything changes.
        let max = self.with(s, |v, _| Ok(v.node_max_file()))?;
        if new.size > max {
            return Err(StoreError::TooLarge);
        }
        if !rp_fat::node::valid_name(name) {
            return Err(StoreError::BadName);
        }
        if let Some(p) = self.st.pending[i].take() {
            let _ = self.with(s, |v, _| v.node_abandon(&p.r));
        }
        let r = self.with(s, |v, sc| v.node_reserve(dir, new.size, sc))?;
        let mut p = Pending {
            r,
            dir,
            name: [0; NAME_UNITS],
            name_len: name.len(),
            hidden: new.hidden,
            created: stamps.0,
            modified: stamps.1,
        };
        p.name[..name.len()].copy_from_slice(name);
        self.st.pending[i] = Some(p);
        Ok(r.temp)
    }

    fn write(&mut self, s: u8, loc: Loc, offset: u64, data: &[u8]) -> Result<(), StoreError> {
        let i = self.slot(s).ok_or(StoreError::Absent)?;
        let mut p = self.st.pending[i]
            .filter(|p| p.r.temp == loc)
            .ok_or(StoreError::NotFound)?;
        let r = self.with(s, |v, sc| v.node_write(&mut p.r, offset, data, sc));
        self.st.pending[i] = Some(p);
        r
    }

    fn commit(&mut self, s: u8, loc: Loc) -> Result<Loc, StoreError> {
        let i = self.slot(s).ok_or(StoreError::Absent)?;
        let p = self.st.pending[i]
            .filter(|p| p.r.temp == loc)
            .ok_or(StoreError::NotFound)?;
        let at = self.with(s, |v, sc| {
            v.node_commit(
                &p.r,
                p.dir,
                &p.name[..p.name_len],
                p.hidden,
                (p.created, p.modified),
                sc,
            )
        })?;
        self.st.pending[i] = None;
        self.changed(s);
        Ok(at)
    }

    fn abandon(&mut self, s: u8, loc: Loc) {
        let Some(i) = self.slot(s) else {
            return;
        };
        if let Some(p) = self.st.pending[i].filter(|p| p.r.temp == loc) {
            let _ = self.with(s, |v, _| v.node_abandon(&p.r));
            self.st.pending[i] = None;
            self.changed(s);
        }
    }

    fn delete(&mut self, s: u8, loc: Loc) -> Result<(), StoreError> {
        self.check_change(s, loc)?;
        self.changed(s);
        self.with(s, |v, _| v.node_delete(loc))
    }

    fn rename(&mut self, s: u8, dir: Loc, loc: Loc, name: &Text) -> Result<Loc, StoreError> {
        self.check_change(s, loc)?;
        self.with(s, |v, sc| v.node_rename(dir, loc, name.units(), sc))
    }

    fn relocate(&mut self, s: u8, from: Loc, loc: Loc, dir: Loc) -> Result<Loc, StoreError> {
        self.check_change(s, loc)?;
        self.with(s, |v, sc| v.node_move(from, loc, dir, sc))
    }

    fn boots_from(&mut self, s: u8) -> bool {
        self.cards.boots_from(s)
    }

    fn format_consent(&mut self, s: u8, start: bool) -> Consent {
        self.cards.format_consent(s, start)
    }

    fn format(&mut self, s: u8) -> Result<(), StoreError> {
        self.forget(s);
        self.cards.format(s)
    }

    fn friendly_name(&self) -> Text {
        self.cards.friendly_name()
    }

    fn set_friendly_name(&mut self, name: &Text) {
        self.cards.set_friendly_name(name);
    }

    fn icon_len(&mut self) -> u32 {
        self.cards.icon_len()
    }

    fn icon_read(&mut self, offset: u32, buf: &mut [u8]) -> usize {
        self.cards.icon_read(offset, buf)
    }

    fn initiator(&mut self, who: &Text) {
        self.cards.initiator(who);
    }

    fn now_ms(&self) -> u32 {
        self.cards.now_ms()
    }
}
