//! What the responder needs from the device: its stores (SD, CF, later the
//! internal flash), addressed by store-defined locations, and a few device
//! services (its name, its icon, the user's yes for a format).
//!
//! A [`Loc`] names one file or folder on one store for as long as it
//! exists unchanged; the store chooses it (`rp-fat`: the directory entry's
//! place). The responder maps its session handles onto locations and
//! updates them when an operation returns a new one (rename, move,
//! commit).

use crate::data::{Text, Time};

/// A store-defined location of a file or folder. [`ROOT`] is the store's
/// root folder.
pub type Loc = u64;

/// The root folder of a store.
pub const ROOT: Loc = 0;

/// Why a store operation failed (mapped onto MTP response codes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreError {
    /// No medium, or the store isn't offered.
    Absent,
    /// The location no longer names anything.
    NotFound,
    /// A file or folder of that name is already there.
    Exists,
    /// Not enough free space.
    Full,
    /// The file system can't hold an object this big (FAT32 4 GB, FAT16
    /// 2 GB).
    TooLarge,
    /// The whole store is read-only (locked card, no guard).
    ReadOnly,
    /// This object is protected (a boot file).
    Protected,
    /// The name isn't allowed on this file system.
    BadName,
    /// The folder isn't empty.
    NotEmpty,
    /// The medium failed or is damaged.
    Device,
    /// Busy for now (the device is using the store).
    Busy,
}

/// The kind of a store [MTP 5.2.2.1].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Built in (the internal flash).
    Fixed,
    /// A card.
    Removable,
}

/// What GetStorageInfo reports [MTP 5.2.2].
#[derive(Clone, Copy, Debug)]
pub struct StoreInfo {
    /// Built in or removable.
    pub kind: Kind,
    /// Writable at all (else read-only without deletion).
    pub writable: bool,
    /// Capacity in bytes.
    pub capacity: u64,
    /// Free bytes.
    pub free: u64,
    /// "SD Card", "CF Card".
    pub description: Text,
    /// Unique among stores and stable for this medium (the volume serial).
    pub volume_id: Text,
    /// The largest file the file system holds (FAT32: 4 GiB - 1).
    pub max_file: u64,
}

/// One file or folder.
#[derive(Clone, Copy, Debug)]
pub struct Entry {
    /// Where it is.
    pub loc: Loc,
    /// Its name; for a name MTP can't carry (over 254 units), a shortened
    /// one, with [`Entry::long_name`] set.
    pub name: Text,
    /// The real name didn't fit MTP: shown read-only under `name`.
    pub long_name: bool,
    /// A folder.
    pub folder: bool,
    /// Size in bytes (0 for folders).
    pub size: u64,
    /// Created, when the file system records it.
    pub created: Option<Time>,
    /// Last modified.
    pub modified: Option<Time>,
    /// Hidden attribute (shown to the PC as the Hidden property).
    pub hidden: bool,
    /// System attribute.
    pub system: bool,
    /// Protected: the device boots from it; no change of any kind.
    pub protected: bool,
    /// Persistent unique ID [MTP B.2.15]: the same in every session for as
    /// long as the object exists, never reused.
    pub uid: u128,
}

impl Default for Entry {
    fn default() -> Self {
        Self {
            loc: ROOT,
            name: Text::new(),
            long_name: false,
            folder: false,
            size: 0,
            created: None,
            modified: None,
            hidden: false,
            system: false,
            protected: false,
            uid: 0,
        }
    }
}

/// How a new object is described when it is created.
#[derive(Clone, Copy, Debug, Default)]
pub struct NewObject {
    /// Its name.
    pub name: Text,
    /// A folder.
    pub folder: bool,
    /// Its size in bytes (files).
    pub size: u64,
    /// Dates the PC sent, kept on the card when the file system can.
    pub created: Option<Time>,
    /// Last modified, as the PC sent it.
    pub modified: Option<Time>,
    /// Hidden attribute.
    pub hidden: bool,
}

/// The answer to a format request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Consent {
    /// Still asking the user.
    Waiting,
    /// The user said yes.
    Yes,
    /// The user said no, or the question timed out.
    No,
}

/// The device behind the responder. Store numbers are `0..stores()`; MTP
/// store IDs are `(n + 1) << 16 | 1` ([MTP 5.2.1]).
pub trait Device {
    /// How many store slots the device has (SD, CF, flash): listed even
    /// when empty [MTP D.2.4].
    fn stores(&self) -> u8;

    /// A store's state, `None` with no medium or not offered.
    fn info(&mut self, s: u8) -> Option<StoreInfo>;

    /// The child after `cursor` in folder `dir` (`cursor` 0 starts; it is
    /// advanced): `Ok(false)` at the end. Volume labels, `.` and `..` are
    /// never children.
    ///
    /// # Errors
    /// The store failed, or `dir` is gone.
    fn child(
        &mut self,
        s: u8,
        dir: Loc,
        cursor: &mut u64,
        out: &mut Entry,
    ) -> Result<bool, StoreError>;

    /// The object at `loc` as it is now.
    ///
    /// # Errors
    /// Gone, or the store failed.
    fn stat(&mut self, s: u8, loc: Loc, out: &mut Entry) -> Result<(), StoreError>;

    /// Reads file bytes from `offset`; returns how many (fewer only at the
    /// end).
    ///
    /// # Errors
    /// Gone, or the store failed.
    fn read(&mut self, s: u8, loc: Loc, offset: u64, buf: &mut [u8]) -> Result<usize, StoreError>;

    /// Creates `new` in `dir`. A folder or an empty file exists at once; a
    /// file with contents is reserved whole (all its space, contiguous when
    /// the file system can) under a name the PC never sees, until
    /// [`commit`](Self::commit) or [`abandon`](Self::abandon).
    ///
    /// # Errors
    /// No room, a bad or taken name, a read-only store.
    fn create(&mut self, s: u8, dir: Loc, new: &NewObject) -> Result<Loc, StoreError>;

    /// Writes into a reserved file: in order, every offset a whole number
    /// of 512-byte sectors (only the file's last write ends mid-sector).
    ///
    /// # Errors
    /// The store failed.
    fn write(&mut self, s: u8, loc: Loc, offset: u64, data: &[u8]) -> Result<(), StoreError>;

    /// The reserved file is complete: it takes its real name. Returns its
    /// final location.
    ///
    /// # Errors
    /// The store failed (the reservation stays, to retry or abandon).
    fn commit(&mut self, s: u8, loc: Loc) -> Result<Loc, StoreError>;

    /// Drops a reserved file and frees its space.
    fn abandon(&mut self, s: u8, loc: Loc);

    /// Deletes a file or an empty folder.
    ///
    /// # Errors
    /// Protected, not empty, read-only, or the store failed.
    fn delete(&mut self, s: u8, loc: Loc) -> Result<(), StoreError>;

    /// Renames `loc`, in folder `dir`; returns the new location.
    ///
    /// # Errors
    /// Protected, a bad or taken name, or the store failed.
    fn rename(&mut self, s: u8, dir: Loc, loc: Loc, name: &Text) -> Result<Loc, StoreError>;

    /// Moves within the store (a directory change: no data moves); returns
    /// the new location.
    ///
    /// # Errors
    /// Protected, a taken name, a folder into itself, or the store failed.
    fn relocate(&mut self, s: u8, from: Loc, loc: Loc, dir: Loc) -> Result<Loc, StoreError>;

    /// Whether the device boots from this store's medium (a format from
    /// the PC is refused at once: ADR-0037 Q3).
    fn boots_from(&mut self, s: u8) -> bool;

    /// Asks the user on the device whether store `s` may be formatted (the
    /// screen names the card; Action or OK confirms). Called again until
    /// it isn't [`Consent::Waiting`]; `start` is true on the first call.
    fn format_consent(&mut self, s: u8, start: bool) -> Consent;

    /// Formats store `s` (ADR-0036's layout).
    ///
    /// # Errors
    /// The store failed.
    fn format(&mut self, s: u8) -> Result<(), StoreError>;

    /// The device's name as the PC shows it [MTP C.2.34].
    fn friendly_name(&self) -> Text;

    /// The PC renamed the device: keep it (it shows on the next
    /// connection too).
    fn set_friendly_name(&mut self, name: &Text);

    /// The device icon (a `.ico` [MTP C.2.37]): its length, 0 for none.
    fn icon_len(&mut self) -> u32 {
        0
    }

    /// Reads the icon from `offset`.
    fn icon_read(&mut self, offset: u32, buf: &mut [u8]) -> usize {
        let _ = (offset, buf);
        0
    }

    /// The PC said what it is (Session Initiator Version Info
    /// [MTP C.2.42]), for the log.
    fn initiator(&mut self, who: &Text) {
        let _ = who;
    }

    /// Milliseconds since some fixed point (format consent timing).
    fn now_ms(&self) -> u32;
}
