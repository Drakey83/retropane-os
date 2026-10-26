//! MTP 1.1 over the USB Still Image Capture transport (ADR-0037): the
//! device's cards appear on the PC as a portable device, the PC asks for
//! each file and each change, and the device answers at once.
//!
//! Hardware-independent: 64-byte packets move through a [`Pipe`] (the USB
//! controller's bulk pair and interrupt endpoint), files through a
//! [`Device`] (SD, CF, later the internal flash; `rp-fat` behind it).
//! Driven by [`Mtp::poll`] from the owner's main loop; no heap, no
//! threads, no recursion (folder walks use the handle table or a fixed
//! stack).
//!
//! What it does better than the phones' MTP (ADR-0037, "Better than
//! Android"): a protected object is announced read-only and refused before
//! any data moves; a file the PC sends is reserved whole first (Store_Full
//! before a byte moves) and only takes its name once complete, so a cancel
//! or an unplug leaves nothing; folder listings of any depth go in one
//! round trip (two passes over the folders, no RAM beyond the handles);
//! moves on a card are directory changes.
//!
//! Security (docs/security.md, ADR-0037): everything from the cable is
//! hostile until checked: container lengths, types, codes, transaction IDs,
//! every handle and store ID looked up (never an index), every offset and
//! length overflow-checked, every dataset bounded field by field; a data
//! phase that fails is drained to its end before the error response
//! [MTP 4.6]. No operation powers the device down, formats a card without
//! the user's yes on the device, deletes everything at once or changes a
//! protected object.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod codes;
pub mod data;
pub mod device;
#[cfg(any(feature = "fat", test))]
pub mod fat;
pub mod handles;

use codes::{container, dprop, ev, fmt, op, prop, rc, ty};
pub use data::{Bad, Reader, Text, Time, Writer};
pub use device::{Consent, Device, Entry, Kind, Loc, NewObject, ROOT, StoreError, StoreInfo};
pub use handles::{Handles, Slot};

/// Bulk packet size (full speed).
pub const PACKET: usize = 64;
/// A card's sector: object bytes go to the device in whole sectors but at
/// the end ([`Device::write`]).
pub const SECTOR: usize = 512;
/// A container's header: length, type, code, transaction ID [SIC 7.1.1].
pub const HEADER: usize = 12;
/// How long the user has to confirm a format on the device (ADR-0037 Q3).
pub const CONSENT_MS: u32 = 30_000;
/// The deepest folder nesting a recursive delete or copy follows; deeper
/// folders are kept (Partial_Deletion), never a crash.
pub const MAX_DEPTH: usize = 64;

/// One level of a folder walk (a delete or a copy of a whole folder): the
/// folder, the cursor in it, the copy's folder, whether something in it was
/// kept. The owner supplies the walk's table ([`Mtp::new`]): folders as deep
/// as it is long are walked, deeper ones kept.
pub type Level = (Loc, u64, Loc, bool);
/// Events waiting for the interrupt pipe; on overflow the queue becomes one
/// UnreportedStatus (the PC re-reads), never a silent drop.
const EVENTS: usize = 16;
/// Operations kept for the log ([`Mtp::trace`]).
pub const TRACE: usize = 32;
/// Room an object's property list element may need in the staging buffer
/// (14 properties, two names of 255 units): the generator stops before an
/// object that might not fit.
const ELEMENT_MAX: usize = 1400;

/// The USB controller's endpoints for this interface: bulk OUT, bulk IN and
/// interrupt IN.
pub trait Pipe {
    /// Takes the next OUT packet (at most [`PACKET`] bytes) if one arrived.
    fn read_packet(&mut self, buf: &mut [u8; PACKET]) -> Option<usize>;
    /// Whether an IN packet can be queued.
    fn can_write(&mut self) -> bool;
    /// Queues one IN packet; shorter than [`PACKET`] (even empty) ends the
    /// phase.
    fn write_packet(&mut self, data: &[u8]);
    /// Whether an event can be queued on the interrupt endpoint.
    fn event_can_write(&mut self) -> bool;
    /// Queues one event (at most 24 bytes: one packet).
    fn event_write(&mut self, data: &[u8]);
    /// Halts both bulk endpoints (a malformed command [SIC 7]); the host
    /// clears them with CLEAR_FEATURE after Get Device Status.
    fn stall(&mut self) {}
    /// Drops what waits in the bulk FIFOs (a cancel).
    fn flush(&mut self) {}
    /// Lets the controller move what is queued while the device works.
    fn service(&mut self) {}
}

/// Who the device says it is [MTP 5.1.1].
#[derive(Clone, Copy, Debug)]
pub struct Identity {
    /// "Dell".
    pub manufacturer: &'static str,
    /// "Axim X51v" (from the board).
    pub model: &'static str,
    /// The RetroPane version.
    pub version: &'static str,
    /// Exactly 32 hex digits [MTP 5.1.1.14].
    pub serial: [u8; 32],
}

/// Counters, for the screen, the log and tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Operations received.
    pub ops: u32,
    /// Operations answered with something other than OK.
    pub failed: u32,
    /// Malformed commands (the pipe was stalled).
    pub bad_commands: u32,
    /// Object bytes sent to the PC.
    pub bytes_out: u64,
    /// Object bytes received from the PC.
    pub bytes_in: u64,
    /// Events sent.
    pub events: u32,
    /// Event queue overflows (became UnreportedStatus).
    pub event_overflows: u32,
    /// Cancels from the PC.
    pub cancels: u32,
    /// Parameters the PC sent where none is defined (ignored: legacy
    /// hosts aren't locked out).
    pub stray_params: u32,
    /// The handle table was full.
    pub table_full: u32,
}

/// One traced operation ([`Mtp::trace`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Traced {
    /// The operation code.
    pub op: u16,
    /// The response code.
    pub rc: u16,
    /// Its first two parameters.
    pub p1: u32,
    /// Second parameter.
    pub p2: u32,
    /// Bytes moved in its data phase.
    pub bytes: u32,
}

/// A queued event.
#[derive(Clone, Copy, Debug, Default)]
struct Event {
    code: u16,
    p1: u32,
    p2: u32,
    params: u8,
}

/// What a walk covers ([`Walk`]).
#[derive(Clone, Copy, Debug)]
struct Walk {
    /// Stores to walk: one, or every store with a medium (`None`).
    store: Option<u8>,
    /// The store being walked now.
    s: u8,
    /// The top folder (0: the store's root) and whether to yield it.
    top: u32,
    yield_top: bool,
    /// How many levels below `top` (children are level 1).
    depth: u32,
    /// Only objects of this format (0: any).
    format: u16,
    /// Where it is: 0 = the top not yet yielded, 1 = listing the top's
    /// children, 2 = scanning the table (folder at `k`), 3 = done.
    stage: u8,
    k: usize,
    dir: (u32, Loc, u32),
    cursor: u64,
    listing: bool,
}

/// Why a walk stopped early.
#[derive(Clone, Copy, Debug)]
struct Stop(u16);

impl From<handles::Full> for Stop {
    fn from(_: handles::Full) -> Self {
        Self(rc::DEVICE_BUSY)
    }
}

fn store_rc(e: StoreError) -> u16 {
    match e {
        StoreError::Absent => rc::STORE_NOT_AVAILABLE,
        StoreError::NotFound => rc::INVALID_OBJECT_HANDLE,
        StoreError::Exists | StoreError::BadName => rc::INVALID_DATASET,
        StoreError::Full => rc::STORE_FULL,
        StoreError::TooLarge => rc::OBJECT_TOO_LARGE,
        StoreError::ReadOnly => rc::STORE_READ_ONLY,
        StoreError::Protected => rc::OBJECT_WRITE_PROTECTED,
        StoreError::NotEmpty => rc::PARTIAL_DELETION,
        StoreError::Device => rc::GENERAL_ERROR,
        StoreError::Busy => rc::DEVICE_BUSY,
    }
}

impl From<StoreError> for Stop {
    fn from(e: StoreError) -> Self {
        Self(store_rc(e))
    }
}

/// The MTP store ID of store number `s` [MTP 5.2.1]: its logical part is
/// 0 when the slot is empty.
#[must_use]
pub fn store_id(s: u8, present: bool) -> u32 {
    (u32::from(s) + 1) << 16 | u32::from(present)
}

fn store_of(id: u32, stores: u8) -> Option<u8> {
    let phys = id >> 16;
    if id & 0xFFFF != 1 || phys == 0 || phys > u32::from(stores) {
        return None;
    }
    Some((phys - 1) as u8)
}

/// The format a file is shown as, from its extension (the PC uses it for
/// icons and players; the bytes are never changed).
#[must_use]
pub fn format_of(e: &Entry) -> u16 {
    if e.folder {
        return fmt::ASSOCIATION;
    }
    let (ext, n) = e.name.extension();
    match &ext[..n] {
        b"txt" | b"log" | b"ini" | b"inf" | b"csv" => fmt::TEXT,
        b"htm" | b"html" => fmt::HTML,
        b"wav" => fmt::WAV,
        b"mp3" => fmt::MP3,
        b"avi" => fmt::AVI,
        b"mpg" | b"mpeg" => fmt::MPEG,
        b"asf" => fmt::ASF,
        b"jpg" | b"jpeg" => fmt::EXIF_JPEG,
        b"bmp" => fmt::BMP,
        b"gif" => fmt::GIF,
        b"png" => fmt::PNG,
        b"tif" | b"tiff" => fmt::TIFF,
        b"wma" => fmt::WMA,
        b"ogg" => fmt::OGG,
        b"aac" | b"m4a" => fmt::AAC,
        b"flac" => fmt::FLAC,
        b"wmv" => fmt::WMV,
        b"mp4" | b"m4v" => fmt::MP4,
        b"m3u" => fmt::M3U,
        b"xml" => fmt::XML,
        b"doc" => fmt::DOC,
        b"xls" => fmt::XLS,
        b"ppt" => fmt::PPT,
        _ => fmt::UNDEFINED,
    }
}

/// Formats offered in DeviceInfo's playback list: anything can be stored
/// (Undefined, [MTP 5.1.1.10]).
const FORMATS: [u16; 26] = [
    fmt::UNDEFINED,
    fmt::ASSOCIATION,
    fmt::TEXT,
    fmt::HTML,
    fmt::WAV,
    fmt::MP3,
    fmt::AVI,
    fmt::MPEG,
    fmt::ASF,
    fmt::EXIF_JPEG,
    fmt::BMP,
    fmt::GIF,
    fmt::JFIF,
    fmt::PNG,
    fmt::TIFF,
    fmt::WMA,
    fmt::OGG,
    fmt::AAC,
    fmt::FLAC,
    fmt::WMV,
    fmt::MP4,
    fmt::M3U,
    fmt::XML,
    fmt::DOC,
    fmt::XLS,
    fmt::PPT,
];

/// Operations offered (ADR-0037 rule 11).
const OPERATIONS: [u16; 29] = [
    op::GET_DEVICE_INFO,
    op::OPEN_SESSION,
    op::CLOSE_SESSION,
    op::GET_STORAGE_IDS,
    op::GET_STORAGE_INFO,
    op::GET_NUM_OBJECTS,
    op::GET_OBJECT_HANDLES,
    op::GET_OBJECT_INFO,
    op::GET_OBJECT,
    op::GET_PARTIAL_OBJECT,
    op::DELETE_OBJECT,
    op::SEND_OBJECT_INFO,
    op::SEND_OBJECT,
    op::FORMAT_STORE,
    op::GET_DEVICE_PROP_DESC,
    op::GET_DEVICE_PROP_VALUE,
    op::SET_DEVICE_PROP_VALUE,
    op::RESET_DEVICE_PROP_VALUE,
    op::MOVE_OBJECT,
    op::COPY_OBJECT,
    op::GET_OBJECT_PROPS_SUPPORTED,
    op::GET_OBJECT_PROP_DESC,
    op::GET_OBJECT_PROP_VALUE,
    op::SET_OBJECT_PROP_VALUE,
    op::GET_OBJECT_PROP_LIST,
    op::SET_OBJECT_PROP_LIST,
    op::SEND_OBJECT_PROP_LIST,
    op::GET_OBJECT_REFERENCES,
    op::GET_THUMB,
];

/// A folder's picture: the file of one of these names it holds (as
/// Windows' own `folder.jpg` for an album), sent as the folder's thumbnail
/// [MTP 5.3.1, D.2.10] (R61: Windows draws no storage's own icon, but it
/// draws a folder's picture, measured on the X51v). PNG first; BMP for its
/// 32-bit alpha (Windows drew the PNG's see-through pixels solid).
pub const FOLDER_PICTURES: [&str; 2] = ["folder.png", "folder.bmp"];
/// The largest folder picture offered: a thumbnail, not an image.
const FOLDER_PICTURE_MAX: u64 = 512 * 1024;

/// A folder picture: where it is, its size, its format and pixel size.
#[derive(Clone, Copy)]
struct Picture {
    loc: Loc,
    size: u64,
    format: u16,
    width: u32,
    height: u32,
}

/// Folder `e`'s picture ([`FOLDER_PICTURES`], read from its header).
/// `None` for a file, a folder without one, or a file that isn't the
/// format its name says.
fn folder_picture(dev: &mut dyn Device, s: u8, e: &Entry) -> Option<Picture> {
    if !e.folder {
        return None;
    }
    let mut cursor = 0;
    let mut c = Entry::default();
    while dev.child(s, e.loc, &mut cursor, &mut c).ok()? {
        let Some(k) = FOLDER_PICTURES.iter().position(|n| c.name.eq_ascii_ci(n)) else {
            continue;
        };
        if c.folder || c.size > FOLDER_PICTURE_MAX {
            return None;
        }
        let mut head = [0u8; 26];
        if dev.read(s, c.loc, 0, &mut head).ok()? != head.len() {
            return None;
        }
        let be = |i: usize| u32::from_be_bytes([head[i], head[i + 1], head[i + 2], head[i + 3]]);
        let le = |i: usize| i32::from_le_bytes([head[i], head[i + 1], head[i + 2], head[i + 3]]);
        let (format, width, height) = if k == 0 {
            if head[..8] != *b"\x89PNG\r\n\x1a\n" || head[12..16] != *b"IHDR" {
                return None;
            }
            (fmt::PNG, be(16), be(20))
        } else {
            // BITMAPFILEHEADER, then the info header's width and height (a
            // negative height: rows top-down).
            if head[..2] != *b"BM" {
                return None;
            }
            (fmt::BMP, le(18).unsigned_abs(), le(22).unsigned_abs())
        };
        return Some(Picture {
            loc: c.loc,
            size: c.size,
            format,
            width,
            height,
        });
    }
    None
}

/// Events sent.
const EVENT_CODES: [u16; 8] = [
    ev::OBJECT_ADDED,
    ev::OBJECT_REMOVED,
    ev::STORE_ADDED,
    ev::STORE_REMOVED,
    ev::DEVICE_PROP_CHANGED,
    ev::OBJECT_INFO_CHANGED,
    ev::STORAGE_INFO_CHANGED,
    ev::UNREPORTED_STATUS,
];

/// Object properties, the same for every format [MTP 5.3.2.2, 5.3.2.6].
const PROPS: [u16; 14] = [
    prop::STORAGE_ID,
    prop::OBJECT_FORMAT,
    prop::PROTECTION_STATUS,
    prop::OBJECT_SIZE,
    prop::ASSOCIATION_TYPE,
    prop::ASSOCIATION_DESC,
    prop::OBJECT_FILE_NAME,
    prop::DATE_CREATED,
    prop::DATE_MODIFIED,
    prop::PARENT_OBJECT,
    prop::HIDDEN,
    prop::SYSTEM_OBJECT,
    prop::PERSISTENT_UID,
    prop::NAME,
];

/// The one property group every property belongs to [MTP 5.3.2.7.1]
/// (all come from the directory entry: equally fast).
const GROUP: u32 = 1;

/// Characters a FAT or exFAT name can't hold, as the file-name property's
/// regular expression form [MTP B.2.7], so the PC refuses a bad name
/// before it sends anything.
const NAME_PATTERN: &str = "[^\\\\/:*?\"<>|]+";

fn prop_type(p: u16) -> Option<u16> {
    Some(match p {
        prop::STORAGE_ID | prop::ASSOCIATION_DESC | prop::PARENT_OBJECT => ty::UINT32,
        prop::OBJECT_FORMAT
        | prop::PROTECTION_STATUS
        | prop::ASSOCIATION_TYPE
        | prop::HIDDEN
        | prop::SYSTEM_OBJECT => ty::UINT16,
        prop::OBJECT_SIZE => ty::UINT64,
        prop::PERSISTENT_UID => ty::UINT128,
        prop::OBJECT_FILE_NAME | prop::DATE_CREATED | prop::DATE_MODIFIED | prop::NAME => ty::STR,
        _ => return None,
    })
}

/// A pending SendObject: what SendObjectInfo/SendObjectPropList reserved.
#[derive(Clone, Copy, Debug)]
struct Pending {
    s: u8,
    /// The reserved file (renamed into place at commit).
    loc: Loc,
    handle: u32,
    parent: u32,
    size: u64,
}

/// What the data-in phase sends after what is already in the buffer.
#[derive(Clone, Copy, Debug)]
enum Gen {
    /// Nothing more.
    Done,
    /// File bytes `off..end`.
    Object { s: u8, loc: Loc, off: u64, end: u64 },
    /// The icon's bytes `off..end`, then `tail` (the form byte of a
    /// property description).
    Icon { off: u32, end: u32, tail: bool },
    /// Handles found by a walk, `left` still to send.
    Handles { walk: Walk, left: u32 },
    /// A property list found by a walk: `left` objects still to send.
    Props { walk: Walk, prop: u16, left: u32 },
}

/// What the data-out phase does with the bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sink {
    /// Collect a dataset in the buffer (too long: overflow, drained).
    Dataset,
    /// The object's bytes, to the pending reservation.
    Object,
    /// Drop them (the operation already failed).
    Discard,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// Waiting for a command.
    Idle,
    /// Sending `buf[pos..len]`, then what `source` makes; `left` stream bytes
    /// to go (None: unknown, over 4 GB), `zlp` when the stream ends on a
    /// whole packet.
    DataIn,
    /// Receiving the data container.
    DataOut,
    /// Sending the response container.
    Response,
    /// Waiting for the user's yes for a format.
    Consent,
}

/// The MTP function.
pub struct Mtp<'b> {
    id: Identity,
    buf: &'b mut [u8],
    handles: Handles<'b>,
    session: u32,
    phase: Phase,
    // The operation being run.
    op: u16,
    tid: u32,
    params: [u32; 5],
    nparams: usize,
    // Data in.
    pos: usize,
    len: usize,
    source: Gen,
    in_left: u64,
    in_known: bool,
    zlp: bool,
    in_bytes: u64,
    // Data out.
    sink: Sink,
    out_total: u64,
    out_known: bool,
    out_got: u64,
    out_header: [u8; HEADER],
    out_header_len: usize,
    out_overflow: bool,
    /// Bytes of the object written so far; buffered bytes not yet written.
    obj_written: u64,
    // Response.
    resp: [u8; 32],
    resp_len: usize,
    rcode: u16,
    rparams: [u32; 5],
    rcount: usize,
    pending: Option<Pending>,
    consent: (u8, u32),
    events: [Event; EVENTS],
    ev_head: usize,
    ev_len: usize,
    stalled: bool,
    stats: Stats,
    trace: [Traced; TRACE],
    trace_n: usize,
    /// The PC's version string (logged once).
    stack: &'b mut [Level],
}

impl<'b> Mtp<'b> {
    /// A function staging data in `buf` (at least 2 KiB; 16 KiB moves a
    /// card's sectors in one command), its handles in `slots`/`index`
    /// ([`Handles::new`]), folder walks in `walk` ([`MAX_DEPTH`] levels is
    /// the usual). Every table is the owner's (static memory), so the
    /// function itself is small enough to build anywhere.
    ///
    /// # Panics
    /// If `buf` is under 2 KiB, `index` too short or `walk` empty.
    pub fn new(
        id: Identity,
        buf: &'b mut [u8],
        slots: &'b mut [Slot],
        index: &'b mut [u32],
        walk: &'b mut [Level],
    ) -> Self {
        assert!(buf.len() >= 2048, "staging buffer too small");
        assert!(!walk.is_empty(), "no walk table");
        Self {
            id,
            buf,
            handles: Handles::new(slots, index),
            session: 0,
            phase: Phase::Idle,
            op: 0,
            tid: 0,
            params: [0; 5],
            nparams: 0,
            pos: 0,
            len: 0,
            source: Gen::Done,
            in_left: 0,
            in_known: true,
            zlp: false,
            in_bytes: 0,
            sink: Sink::Discard,
            out_total: 0,
            out_known: true,
            out_got: 0,
            out_header: [0; HEADER],
            out_header_len: 0,
            out_overflow: false,
            obj_written: 0,
            resp: [0; 32],
            resp_len: 0,
            rcode: 0,
            rparams: [0; 5],
            rcount: 0,
            pending: None,
            consent: (0, 0),
            events: [Event::default(); EVENTS],
            ev_head: 0,
            ev_len: 0,
            stalled: false,
            stats: Stats::default(),
            trace: [Traced::default(); TRACE],
            trace_n: 0,
            stack: walk,
        }
    }

    /// Counters.
    #[must_use]
    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// The session ID, 0 with none open.
    #[must_use]
    pub fn session(&self) -> u32 {
        self.session
    }

    /// Handles assigned in this session and the table's size.
    #[must_use]
    pub fn handles_used(&self) -> (usize, usize) {
        (self.handles.len(), self.handles.capacity())
    }

    /// Operations since the last call, oldest first (for the card log: the
    /// first device run records what Windows asks, ADR-0037 "open points").
    pub fn take_trace(&mut self, out: &mut [Traced]) -> usize {
        let n = self.trace_n.min(TRACE).min(out.len());
        let start = self.trace_n - n;
        for (i, o) in out[..n].iter_mut().enumerate() {
            *o = self.trace[(start + i) % TRACE];
        }
        self.trace_n = 0;
        n
    }

    /// Whether an operation is between its command and its response.
    #[must_use]
    pub fn busy(&self) -> bool {
        self.phase != Phase::Idle
    }

    // --- Events from the device ------------------------------------------

    fn queue(&mut self, code: u16, params: &[u32]) {
        if self.session == 0 {
            return;
        }
        if self.ev_len == EVENTS {
            // Overflow: one UnreportedStatus says "re-read everything".
            self.stats.event_overflows += 1;
            self.ev_head = 0;
            self.ev_len = 1;
            self.events[0] = Event {
                code: ev::UNREPORTED_STATUS,
                ..Event::default()
            };
            return;
        }
        if self.ev_len == 1 && self.events[self.ev_head].code == ev::UNREPORTED_STATUS {
            return;
        }
        let e = Event {
            code,
            p1: params.first().copied().unwrap_or(0),
            p2: params.get(1).copied().unwrap_or(0),
            params: params.len() as u8,
        };
        self.events[(self.ev_head + self.ev_len) % EVENTS] = e;
        self.ev_len += 1;
    }

    /// A card went in (`present`) or out of store `s`.
    pub fn store_changed(&mut self, s: u8, present: bool) {
        self.handles.kill_store(s);
        if self.pending.is_some_and(|p| p.s == s) {
            // The card and its reservation are gone together.
            self.pending = None;
        }
        if present {
            self.queue(ev::STORE_ADDED, &[store_id(s, true)]);
        } else {
            self.queue(ev::STORE_REMOVED, &[(u32::from(s) + 1) << 16 | 0xFFFF]);
        }
    }

    /// The device itself made `loc` in folder `parent` on store `s` (a file
    /// the loader wrote): the PC hears of it if it has seen the folder
    /// [MTP G.2.3].
    pub fn object_added(&mut self, dev: &mut dyn Device, s: u8, parent: Loc, loc: Loc) {
        if self.session == 0 {
            return;
        }
        let ph = if parent == ROOT {
            0
        } else {
            match self.handles.find(s, parent) {
                Some(h) => h,
                None => return,
            }
        };
        let mut e = Entry::default();
        if dev.stat(s, loc, &mut e).is_err() {
            return;
        }
        match self.handles.get_or_add(s, loc, ph, e.folder) {
            Ok(h) => self.queue(ev::OBJECT_ADDED, &[h]),
            Err(_) => {
                self.stats.table_full += 1;
                self.queue(ev::UNREPORTED_STATUS, &[]);
            }
        }
        self.queue(ev::STORAGE_INFO_CHANGED, &[store_id(s, true)]);
    }

    /// The device itself removed `loc` (called before or after it went).
    pub fn object_removed(&mut self, s: u8, loc: Loc) {
        if let Some(h) = self.handles.find(s, loc) {
            self.handles.kill(h);
            self.queue(ev::OBJECT_REMOVED, &[h]);
            self.queue(ev::STORAGE_INFO_CHANGED, &[store_id(s, true)]);
        }
    }

    /// The device changed `loc` in place (its contents or attributes).
    pub fn object_changed(&mut self, s: u8, loc: Loc) {
        if let Some(h) = self.handles.find(s, loc) {
            self.queue(ev::OBJECT_INFO_CHANGED, &[h]);
            self.queue(ev::STORAGE_INFO_CHANGED, &[store_id(s, true)]);
        }
    }

    /// The device changed many things at once (a card repaired, a whole
    /// folder written): the PC re-reads.
    pub fn everything_changed(&mut self) {
        self.queue(ev::UNREPORTED_STATUS, &[]);
    }

    // --- Class requests (from the controller's endpoint 0) ----------------

    /// Cancel Request [SIC 5.2.1]: the PC abandons transaction `tid`.
    pub fn cancel(&mut self, dev: &mut dyn Device, pipe: &mut dyn Pipe, tid: u32) {
        self.stats.cancels += 1;
        if self.phase == Phase::Idle || tid != self.tid {
            return;
        }
        self.abort(dev, pipe, false);
    }

    /// Device Reset Request [SIC 5.2.3], a USB bus reset or the cable gone:
    /// the operation is abandoned and the session closed (a reserved file
    /// is dropped: nothing half-written stays).
    pub fn reset(&mut self, dev: &mut dyn Device, pipe: &mut dyn Pipe) {
        self.abort(dev, pipe, true);
        self.stalled = false;
    }

    /// Get Device Status [SIC 5.2.4]: the bytes to answer with.
    pub fn device_status(&self, out: &mut [u8; 12]) -> usize {
        if self.stalled {
            // TransactionCancelled with the halted endpoints: the owner's
            // endpoint numbers aren't known here; the controller fills them.
            out[..4].copy_from_slice(&[4, 0, 0x1F, 0x20]);
            4
        } else {
            out[..4].copy_from_slice(&[4, 0, 0x01, 0x20]);
            4
        }
    }

    /// The host cleared the halted endpoints.
    pub fn cleared(&mut self) {
        self.stalled = false;
    }

    fn abort(&mut self, dev: &mut dyn Device, pipe: &mut dyn Pipe, close: bool) {
        pipe.flush();
        self.phase = Phase::Idle;
        self.source = Gen::Done;
        if close {
            self.close_session(dev);
        } else if self.pending.is_some() {
            // A cancelled SendObject: the reservation stays for a retry
            // [MTP D.2.13]; its bytes start over.
            self.obj_written = 0;
        }
    }

    fn close_session(&mut self, dev: &mut dyn Device) {
        if let Some(p) = self.pending.take() {
            dev.abandon(p.s, p.loc);
        }
        self.session = 0;
        self.handles.clear();
        self.ev_len = 0;
    }

    // --- The main loop ------------------------------------------------------

    /// Moves what can move now: a command, data, a response, an event.
    pub fn poll(&mut self, dev: &mut dyn Device, pipe: &mut dyn Pipe) {
        self.send_event(pipe);
        // Several steps per call while the pipe keeps up (whole packets
        // without returning to the loop between each).
        for _ in 0..64 {
            let moved = match self.phase {
                Phase::Idle => self.take_command(dev, pipe),
                Phase::DataIn => self.data_in(dev, pipe),
                Phase::DataOut => self.data_out(dev, pipe),
                Phase::Response => self.send_response(pipe),
                Phase::Consent => self.wait_consent(dev),
            };
            if !moved {
                break;
            }
        }
    }

    fn send_event(&mut self, pipe: &mut dyn Pipe) {
        if self.ev_len == 0 || !pipe.event_can_write() {
            return;
        }
        let e = self.events[self.ev_head];
        self.ev_head = (self.ev_head + 1) % EVENTS;
        self.ev_len -= 1;
        let n = HEADER + 4 * usize::from(e.params);
        let mut b = [0u8; 20];
        b[..4].copy_from_slice(&(n as u32).to_le_bytes());
        b[4..6].copy_from_slice(&container::EVENT.to_le_bytes());
        b[6..8].copy_from_slice(&e.code.to_le_bytes());
        // TransactionID 0: not tied to a transaction [SIC 7.3.1].
        if e.params > 0 {
            b[12..16].copy_from_slice(&e.p1.to_le_bytes());
        }
        if e.params > 1 {
            b[16..20].copy_from_slice(&e.p2.to_le_bytes());
        }
        pipe.event_write(&b[..n]);
        self.stats.events += 1;
    }

    // Its own frame (not the poll loop's): only the phase running is on
    // the owner's stack (the loader's is 32 KiB, ADR-0037 rule 7).
    #[inline(never)]
    fn take_command(&mut self, dev: &mut dyn Device, pipe: &mut dyn Pipe) -> bool {
        let mut pkt = [0u8; PACKET];
        let Some(n) = pipe.read_packet(&mut pkt) else {
            return false;
        };
        let len = u32::from_le_bytes([pkt[0], pkt[1], pkt[2], pkt[3]]) as usize;
        let kind = u16::from_le_bytes([pkt[4], pkt[5]]);
        // A command is 12..=32 bytes and one packet; anything else (or a
        // packet shorter than its own length) is invalid: stall [SIC 7].
        if n < HEADER
            || kind != container::COMMAND
            || !(HEADER..=32).contains(&len)
            || n < len
            || !(len - HEADER).is_multiple_of(4)
        {
            self.stats.bad_commands += 1;
            self.stalled = true;
            pipe.stall();
            return true;
        }
        self.op = u16::from_le_bytes([pkt[6], pkt[7]]);
        self.tid = u32::from_le_bytes([pkt[8], pkt[9], pkt[10], pkt[11]]);
        self.nparams = (len - HEADER) / 4;
        self.params = [0; 5];
        for i in 0..self.nparams {
            let o = HEADER + 4 * i;
            self.params[i] = u32::from_le_bytes([pkt[o], pkt[o + 1], pkt[o + 2], pkt[o + 3]]);
        }
        self.stats.ops += 1;
        self.in_bytes = 0;
        self.run(dev);
        true
    }

    fn p(&self, i: usize) -> u32 {
        self.params[i]
    }

    /// Parameters beyond what `op` defines: counted, ignored.
    fn extra(&mut self, defined: usize) {
        if self.params[defined.min(5)..].iter().any(|&p| p != 0) {
            self.stats.stray_params += 1;
        }
    }

    fn respond(&mut self, code: u16, params: &[u32]) {
        self.rcode = code;
        self.rcount = params.len().min(5);
        self.rparams = [0; 5];
        self.rparams[..self.rcount].copy_from_slice(&params[..self.rcount]);
        let n = HEADER + 4 * self.rcount;
        self.resp[..4].copy_from_slice(&(n as u32).to_le_bytes());
        self.resp[4..6].copy_from_slice(&container::RESPONSE.to_le_bytes());
        self.resp[6..8].copy_from_slice(&code.to_le_bytes());
        self.resp[8..12].copy_from_slice(&self.tid.to_le_bytes());
        for i in 0..self.rcount {
            self.resp[HEADER + 4 * i..HEADER + 4 * i + 4]
                .copy_from_slice(&self.rparams[i].to_le_bytes());
        }
        self.resp_len = n;
        self.phase = Phase::Response;
        if code != rc::OK {
            self.stats.failed += 1;
        }
        let t = Traced {
            op: self.op,
            rc: code,
            p1: self.params[0],
            p2: self.params[1],
            bytes: self.in_bytes.min(u64::from(u32::MAX)) as u32,
        };
        self.trace[self.trace_n % TRACE] = t;
        self.trace_n += 1;
    }

    fn send_response(&mut self, pipe: &mut dyn Pipe) -> bool {
        if !pipe.can_write() {
            return false;
        }
        let n = self.resp_len;
        let r = self.resp;
        pipe.write_packet(&r[..n]);
        self.phase = Phase::Idle;
        true
    }

    // --- Data in --------------------------------------------------------------

    /// Starts a data-in phase: `payload` bytes (`None`: unknown, over 4 GB),
    /// the first `len` of which are already at `buf[HEADER..]`.
    fn start_in(&mut self, payload: Option<u64>, len: usize, source: Gen) {
        let total = payload.map(|p| p + HEADER as u64);
        let field = match total {
            Some(t) if t < u64::from(u32::MAX) => t as u32,
            _ => u32::MAX,
        };
        self.buf[..4].copy_from_slice(&field.to_le_bytes());
        self.buf[4..6].copy_from_slice(&container::DATA.to_le_bytes());
        self.buf[6..8].copy_from_slice(&self.op.to_le_bytes());
        self.buf[8..12].copy_from_slice(&self.tid.to_le_bytes());
        self.pos = 0;
        self.len = HEADER + len;
        self.source = source;
        self.in_known = field != u32::MAX;
        self.in_left = total.unwrap_or(u64::MAX);
        self.zlp = false;
        self.phase = Phase::DataIn;
    }

    // Its own frame (not the poll loop's): only the phase running is on
    // the owner's stack (the loader's is 32 KiB, ADR-0037 rule 7).
    #[inline(never)]
    fn data_in(&mut self, dev: &mut dyn Device, pipe: &mut dyn Pipe) -> bool {
        if self.zlp {
            if !pipe.can_write() {
                return false;
            }
            pipe.write_packet(&[]);
            self.zlp = false;
            self.finish_in(dev);
            return true;
        }
        if self.len - self.pos < PACKET && !matches!(self.source, Gen::Done) {
            // Refill behind what is left (a packet's worth at most).
            let left = self.len - self.pos;
            self.buf.copy_within(self.pos..self.len, 0);
            self.pos = 0;
            self.len = left;
            if let Err(Stop(code)) = self.fill(dev) {
                // The data can't be finished honestly: cut the phase short
                // (the host sees a short transfer) and answer the error.
                self.source = Gen::Done;
                self.in_left = 0;
                self.zlp = false;
                if pipe.can_write() {
                    let n = (self.len - self.pos).min(PACKET - 1);
                    let chunk = &self.buf[self.pos..self.pos + n];
                    pipe.write_packet(chunk);
                }
                self.respond(code, &[]);
                return true;
            }
        }
        if !pipe.can_write() {
            return false;
        }
        let avail = self.len - self.pos;
        let n = avail.min(PACKET);
        if n == 0 {
            // The generator ran dry before the announced length (the card
            // changed under the walk): end the phase now.
            pipe.write_packet(&[]);
            self.finish_in(dev);
            return true;
        }
        let pkt = &self.buf[self.pos..self.pos + n];
        pipe.write_packet(pkt);
        self.pos += n;
        if self.in_known {
            self.in_left = self.in_left.saturating_sub(n as u64);
        }
        let ended_short = n < PACKET;
        let done = if self.in_known {
            self.in_left == 0
        } else {
            matches!(self.source, Gen::Done) && self.pos == self.len
        };
        if done {
            if ended_short {
                self.finish_in(dev);
            } else {
                self.zlp = true;
            }
        } else if ended_short {
            // Short before the end: only when the generator ran dry.
            self.finish_in(dev);
        }
        true
    }

    /// Fills the buffer after `len` from the generator.
    fn fill(&mut self, dev: &mut dyn Device) -> Result<(), Stop> {
        match self.source {
            Gen::Done => {}
            Gen::Object { s, loc, off, end } => {
                let room = self.buf.len() - self.len;
                let want = (end - off).min(room as u64) as usize;
                let n = dev.read(s, loc, off, &mut self.buf[self.len..self.len + want])?;
                if n == 0 && want > 0 {
                    return Err(Stop(rc::INCOMPLETE_TRANSFER));
                }
                self.len += n;
                self.in_bytes += n as u64;
                self.stats.bytes_out += n as u64;
                let off = off + n as u64;
                self.source = if off >= end {
                    Gen::Done
                } else {
                    Gen::Object { s, loc, off, end }
                };
            }
            Gen::Icon { off, end, tail } => {
                let room = self.buf.len() - self.len - 1;
                let want = (end - off).min(room as u32) as usize;
                let n = dev.icon_read(off, &mut self.buf[self.len..self.len + want]);
                self.len += n;
                let off = off + n as u32;
                if off >= end || n == 0 {
                    if tail {
                        self.buf[self.len] = 0; // form flag: none
                        self.len += 1;
                    }
                    self.source = Gen::Done;
                } else {
                    self.source = Gen::Icon { off, end, tail };
                }
            }
            Gen::Handles { mut walk, mut left } => {
                let mut e = Entry::default();
                while left > 0 && self.buf.len() - self.len >= 4 {
                    let Some(h) = self.walk_next(dev, &mut walk, &mut e)? else {
                        left = 0;
                        break;
                    };
                    self.buf[self.len..self.len + 4].copy_from_slice(&h.to_le_bytes());
                    self.len += 4;
                    left -= 1;
                }
                self.source = if left == 0 {
                    Gen::Done
                } else {
                    Gen::Handles { walk, left }
                };
            }
            Gen::Props {
                mut walk,
                prop,
                mut left,
            } => {
                let mut e = Entry::default();
                while left > 0 && self.buf.len() - self.len >= ELEMENT_MAX {
                    let Some(h) = self.walk_next(dev, &mut walk, &mut e)? else {
                        left = 0;
                        break;
                    };
                    let s = walk.s;
                    let parent = self.handles.get(h).map_or(0, |sl| sl.parent);
                    let start = self.len;
                    let mut w = Writer::new(&mut self.buf[start..]);
                    write_props(&mut w, h, s, parent, &e, prop);
                    self.len = start + w.len();
                    left -= 1;
                }
                self.source = if left == 0 {
                    Gen::Done
                } else {
                    Gen::Props { walk, prop, left }
                };
            }
        }
        Ok(())
    }

    fn finish_in(&mut self, dev: &mut dyn Device) {
        let _ = dev;
        let params: [u32; 1];
        let p: &[u32] = if self.op == op::GET_PARTIAL_OBJECT {
            params = [self.in_bytes.min(u64::from(u32::MAX)) as u32];
            &params
        } else {
            &[]
        };
        self.respond(rc::OK, p);
    }

    // --- Data out -------------------------------------------------------------

    fn start_out(&mut self, sink: Sink) {
        self.sink = sink;
        self.out_total = 0;
        self.out_known = true;
        self.out_got = 0;
        self.out_header_len = 0;
        self.out_overflow = false;
        self.len = 0;
        self.obj_written = 0;
        self.phase = Phase::DataOut;
    }

    // Its own frame (not the poll loop's): only the phase running is on
    // the owner's stack (the loader's is 32 KiB, ADR-0037 rule 7).
    #[inline(never)]
    fn data_out(&mut self, dev: &mut dyn Device, pipe: &mut dyn Pipe) -> bool {
        // Make room for a packet before reading one.
        if self.sink == Sink::Object
            && self.buf.len() - self.len < PACKET
            && let Err(e) = self.flush_object(dev, false)
        {
            self.sink = Sink::Discard;
            self.rcode = store_rc(e);
        }
        let mut pkt = [0u8; PACKET];
        let Some(n) = pipe.read_packet(&mut pkt) else {
            return false;
        };
        let mut data = &pkt[..n];
        if self.out_header_len < HEADER {
            let take = (HEADER - self.out_header_len).min(data.len());
            self.out_header[self.out_header_len..self.out_header_len + take]
                .copy_from_slice(&data[..take]);
            self.out_header_len += take;
            data = &data[take..];
            if self.out_header_len == HEADER {
                let h = self.out_header;
                let len = u32::from_le_bytes([h[0], h[1], h[2], h[3]]);
                let kind = u16::from_le_bytes([h[4], h[5]]);
                let code = u16::from_le_bytes([h[6], h[7]]);
                let tid = u32::from_le_bytes([h[8], h[9], h[10], h[11]]);
                if kind != container::DATA
                    || code != self.op
                    || tid != self.tid
                    || (len as usize) < HEADER
                {
                    // Not this operation's data: drop it, fail cleanly.
                    self.sink = Sink::Discard;
                    self.rcode = rc::INVALID_DATASET;
                }
                if len == u32::MAX {
                    self.out_known = false;
                } else {
                    self.out_total = u64::from(len).saturating_sub(HEADER as u64);
                }
            }
        }
        if !data.is_empty() {
            self.take_out(dev, data);
        }
        let short = n < PACKET;
        let done = if self.out_header_len < HEADER {
            short
        } else if self.out_known {
            self.out_got >= self.out_total || short
        } else {
            short
        };
        if done {
            let complete = self.out_header_len == HEADER
                && (!self.out_known || self.out_got == self.out_total);
            self.finish_out(dev, complete);
        }
        true
    }

    #[inline(never)]
    fn take_out(&mut self, dev: &mut dyn Device, data: &[u8]) {
        let data = if self.out_known {
            let left = self.out_total.saturating_sub(self.out_got);
            &data[..(data.len() as u64).min(left) as usize]
        } else {
            data
        };
        self.out_got += data.len() as u64;
        match self.sink {
            Sink::Discard => {}
            Sink::Dataset => {
                if self.len + data.len() > self.buf.len() {
                    self.out_overflow = true;
                } else {
                    self.buf[self.len..self.len + data.len()].copy_from_slice(data);
                    self.len += data.len();
                }
            }
            Sink::Object => {
                let Some(p) = self.pending else {
                    self.sink = Sink::Discard;
                    return;
                };
                // More than announced: refused (Store_Full [MTP D.2.13]),
                // the rest drained.
                if self.obj_written + self.len as u64 + data.len() as u64 > p.size {
                    self.sink = Sink::Discard;
                    self.rcode = rc::STORE_FULL;
                    return;
                }
                self.buf[self.len..self.len + data.len()].copy_from_slice(data);
                self.len += data.len();
                if self.buf.len() - self.len < PACKET
                    && let Err(e) = self.flush_object(dev, false)
                {
                    self.sink = Sink::Discard;
                    self.rcode = store_rc(e);
                }
            }
        }
    }

    /// Writes the buffered object bytes to the reservation (whole sectors
    /// when it can: the buffer holds a whole number of them).
    #[inline(never)]
    fn flush_object(&mut self, dev: &mut dyn Device, last: bool) -> Result<(), StoreError> {
        let Some(p) = self.pending else {
            return Ok(());
        };
        // Whole sectors, the tail kept for the next write (a card writes
        // sectors; the container's 12-byte header shifts the bytes), all of
        // it at the end.
        let n = if last {
            self.len
        } else {
            self.len / SECTOR * SECTOR
        };
        if n == 0 {
            return Ok(());
        }
        dev.write(p.s, p.loc, self.obj_written, &self.buf[..n])?;
        self.obj_written += n as u64;
        self.stats.bytes_in += n as u64;
        self.in_bytes += n as u64;
        self.buf.copy_within(n..self.len, 0);
        self.len -= n;
        Ok(())
    }

    #[inline(never)]
    fn finish_out(&mut self, dev: &mut dyn Device, complete: bool) {
        let failed = self.rcode;
        match self.sink {
            Sink::Discard => {
                let code = if failed == 0 {
                    rc::INCOMPLETE_TRANSFER
                } else {
                    failed
                };
                self.respond(code, &[]);
            }
            Sink::Dataset => {
                if !complete {
                    self.respond(rc::INCOMPLETE_TRANSFER, &[]);
                } else if self.out_overflow {
                    self.respond(rc::INVALID_DATASET, &[]);
                } else {
                    self.dataset_arrived(dev);
                }
            }
            Sink::Object => {
                let flushed = self.flush_object(dev, true);
                let Some(p) = self.pending else {
                    self.respond(rc::NO_VALID_OBJECT_INFO, &[]);
                    return;
                };
                if let Err(e) = flushed {
                    self.obj_written = 0;
                    self.respond(store_rc(e), &[]);
                } else if !complete || self.obj_written != p.size {
                    // Kept for a retry [MTP D.2.13]; nothing is named yet.
                    self.obj_written = 0;
                    self.respond(rc::INCOMPLETE_TRANSFER, &[]);
                } else {
                    match dev.commit(p.s, p.loc) {
                        Ok(loc) => {
                            self.pending = None;
                            self.handles.relocate(p.handle, p.s, loc, p.parent);
                            self.respond(rc::OK, &[]);
                        }
                        Err(e) => {
                            self.obj_written = 0;
                            self.respond(store_rc(e), &[]);
                        }
                    }
                }
            }
        }
    }

    // --- Operations -------------------------------------------------------------

    fn run(&mut self, dev: &mut dyn Device) {
        self.rcode = 0;
        let code = self.op;
        if self.session == 0 && !matches!(code, op::GET_DEVICE_INFO | op::OPEN_SESSION) {
            // Operations that send data must still take it [MTP 4.6].
            if matches!(
                code,
                op::SEND_OBJECT_INFO
                    | op::SEND_OBJECT
                    | op::SET_DEVICE_PROP_VALUE
                    | op::SET_OBJECT_PROP_VALUE
                    | op::SET_OBJECT_PROP_LIST
                    | op::SEND_OBJECT_PROP_LIST
            ) {
                self.rcode = rc::SESSION_NOT_OPEN;
                self.start_out(Sink::Discard);
            } else if OPERATIONS.contains(&code) {
                self.respond(rc::SESSION_NOT_OPEN, &[]);
            } else {
                self.respond(rc::OPERATION_NOT_SUPPORTED, &[]);
            }
            return;
        }
        // A pending reservation lives only until the next operation, unless
        // that is its SendObject [MTP D.2.12].
        if code != op::SEND_OBJECT
            && let Some(p) = self.pending.take()
        {
            dev.abandon(p.s, p.loc);
            self.handles.kill(p.handle);
        }
        match code {
            op::GET_DEVICE_INFO => self.device_info(dev),
            op::OPEN_SESSION => self.open_session(),
            op::CLOSE_SESSION => {
                self.extra(0);
                self.close_session(dev);
                self.respond(rc::OK, &[]);
            }
            op::GET_STORAGE_IDS => self.storage_ids(dev),
            op::GET_STORAGE_INFO => self.storage_info(dev),
            op::GET_NUM_OBJECTS | op::GET_OBJECT_HANDLES => self.object_handles(dev),
            op::GET_OBJECT_INFO => self.object_info(dev),
            op::GET_OBJECT | op::GET_PARTIAL_OBJECT => self.get_object(dev),
            op::DELETE_OBJECT => self.delete_object(dev),
            op::SEND_OBJECT_INFO | op::SEND_OBJECT_PROP_LIST => self.start_out(Sink::Dataset),
            op::SEND_OBJECT => {
                if self.pending.is_some() {
                    self.start_out(Sink::Object);
                } else {
                    self.rcode = rc::NO_VALID_OBJECT_INFO;
                    self.start_out(Sink::Discard);
                }
            }
            op::FORMAT_STORE => self.format_store(dev),
            op::GET_DEVICE_PROP_DESC | op::GET_DEVICE_PROP_VALUE => self.device_prop(dev),
            op::SET_DEVICE_PROP_VALUE => self.start_out(Sink::Dataset),
            op::RESET_DEVICE_PROP_VALUE => self.respond(rc::ACCESS_DENIED, &[]),
            op::MOVE_OBJECT | op::COPY_OBJECT => self.move_or_copy(dev),
            op::GET_OBJECT_PROPS_SUPPORTED => self.props_supported(),
            op::GET_OBJECT_PROP_DESC => self.prop_desc(),
            op::GET_OBJECT_PROP_VALUE => self.prop_value(dev),
            op::SET_OBJECT_PROP_VALUE | op::SET_OBJECT_PROP_LIST => self.start_out(Sink::Dataset),
            op::GET_OBJECT_PROP_LIST => self.prop_list(dev),
            op::GET_OBJECT_REFERENCES => self.references(),
            op::GET_THUMB => self.get_thumb(dev),
            // Offered nowhere (ADR-0037 rule 11): power, self-test, capture,
            // protection, references, thumbnails.
            _ => self.respond(rc::OPERATION_NOT_SUPPORTED, &[]),
        }
    }

    fn dataset_arrived(&mut self, dev: &mut dyn Device) {
        match self.op {
            op::SEND_OBJECT_INFO => self.send_object_info(dev),
            op::SEND_OBJECT_PROP_LIST => self.send_object_prop_list(dev),
            op::SET_DEVICE_PROP_VALUE => self.set_device_prop(dev),
            op::SET_OBJECT_PROP_VALUE => self.set_prop_value(dev),
            op::SET_OBJECT_PROP_LIST => self.set_prop_list(dev),
            _ => self.respond(rc::GENERAL_ERROR, &[]),
        }
    }

    fn device_info(&mut self, dev: &mut dyn Device) {
        self.extra(0);
        let id = self.id;
        let icon = dev.icon_len() > 0;
        let mut w = Writer::new(&mut self.buf[HEADER..]);
        w.u16(100); // PTP 1.00 [MTP 5.1.1.1]
        // Microsoft's extension ID and string, which Windows and libmtp
        // expect (the specification's 0xFFFFFFFF reads as "a camera":
        // ADR-0037 fact 15).
        w.u32(6);
        w.u16(100);
        w.ascii(b"microsoft.com: 1.0; ");
        w.u16(0); // functional mode: standard
        w.a16(&OPERATIONS);
        w.a16(&EVENT_CODES);
        let props_icon = [
            dprop::FRIENDLY_NAME,
            dprop::SESSION_INITIATOR,
            dprop::PERCEIVED_TYPE,
            dprop::DEVICE_ICON,
        ];
        w.a16(if icon { &props_icon } else { &props_icon[..3] });
        w.a16(&[]); // capture formats: none
        w.a16(&FORMATS);
        w.ascii(id.manufacturer.as_bytes());
        w.ascii(id.model.as_bytes());
        w.ascii(id.version.as_bytes());
        w.ascii(&id.serial);
        let n = w.len();
        self.start_in(Some(n as u64), n, Gen::Done);
    }

    fn open_session(&mut self) {
        self.extra(1);
        let sid = self.p(0);
        if sid == 0 {
            self.respond(rc::INVALID_PARAMETER, &[]);
        } else if self.session != 0 {
            let open = self.session;
            self.respond(rc::SESSION_ALREADY_OPEN, &[open]);
        } else {
            self.session = sid;
            self.handles.clear();
            self.ev_len = 0;
            self.respond(rc::OK, &[]);
        }
    }

    fn storage_ids(&mut self, dev: &mut dyn Device) {
        self.extra(0);
        let n = dev.stores();
        let mut w = Writer::new(&mut self.buf[HEADER..]);
        w.u32(u32::from(n));
        for s in 0..n {
            let present = dev.info(s).is_some();
            w.u32(store_id(s, present));
        }
        let len = w.len();
        self.start_in(Some(len as u64), len, Gen::Done);
    }

    fn storage_info(&mut self, dev: &mut dyn Device) {
        self.extra(1);
        let Some(s) = store_of(self.p(0), dev.stores()) else {
            self.respond(rc::INVALID_STORAGE_ID, &[]);
            return;
        };
        let Some(i) = dev.info(s) else {
            self.respond(rc::STORE_NOT_AVAILABLE, &[]);
            return;
        };
        let mut w = Writer::new(&mut self.buf[HEADER..]);
        w.u16(match i.kind {
            Kind::Fixed => 0x0003,
            Kind::Removable => 0x0004,
        });
        w.u16(0x0002); // generic hierarchical
        w.u16(if i.writable { 0x0000 } else { 0x0001 });
        w.u64(i.capacity);
        w.u64(i.free);
        w.u32(u32::MAX); // free space in objects: unused
        w.text(&i.description);
        w.text(&i.volume_id);
        let len = w.len();
        self.start_in(Some(len as u64), len, Gen::Done);
    }

    /// The store and location of a live handle, its entry as it is now.
    fn object(&mut self, dev: &mut dyn Device, h: u32, e: &mut Entry) -> Result<Slot, u16> {
        let slot = self.handles.get(h).ok_or(rc::INVALID_OBJECT_HANDLE)?;
        match dev.stat(slot.store, slot.loc, e) {
            Ok(()) => Ok(slot),
            Err(StoreError::NotFound) => {
                self.handles.kill(h);
                Err(rc::INVALID_OBJECT_HANDLE)
            }
            Err(err) => Err(store_rc(err)),
        }
    }

    /// The walk GetObjectHandles / GetNumObjects ask for: `p1` the store
    /// (0xFFFFFFFF all), `p2` the format, `p3` the folder (0 the whole
    /// store, 0xFFFFFFFF the root).
    fn handles_walk(&mut self, dev: &mut dyn Device) -> Result<Walk, u16> {
        let (sid, format, parent) = (self.p(0), self.p(1), self.p(2));
        let format = u16::try_from(format).map_err(|_| rc::INVALID_OBJECT_FORMAT_CODE)?;
        let store = if sid == u32::MAX {
            None
        } else {
            let s = store_of(sid, dev.stores()).ok_or(rc::INVALID_STORAGE_ID)?;
            if dev.info(s).is_none() {
                return Err(rc::STORE_NOT_AVAILABLE);
            }
            Some(s)
        };
        let (top, depth) = match parent {
            0 => (0, u32::MAX),
            u32::MAX => (0, 1),
            h => {
                let mut e = Entry::default();
                let slot = self.object(dev, h, &mut e)?;
                if !e.folder {
                    return Err(rc::INVALID_PARENT_OBJECT);
                }
                if store.is_some_and(|s| s != slot.store) {
                    return Err(rc::INVALID_PARENT_OBJECT);
                }
                return Ok(Walk::new(Some(slot.store), h, false, 1, format));
            }
        };
        Ok(Walk::new(store, top, false, depth, format))
    }

    /// Counts what a walk yields (assigning handles on the way); also the
    /// bytes a property list for it takes.
    fn count(
        &mut self,
        dev: &mut dyn Device,
        walk: Walk,
        prop: Option<u16>,
    ) -> Result<(u32, u64), Stop> {
        let mut w = walk;
        let mut e = Entry::default();
        let (mut n, mut bytes) = (0u32, 0u64);
        while let Some(h) = self.walk_next(dev, &mut w, &mut e)? {
            n += 1;
            if let Some(p) = prop {
                let parent = self.handles.get(h).map_or(0, |s| s.parent);
                let mut c = Writer::counter();
                write_props(&mut c, h, w.s, parent, &e, p);
                bytes += c.len() as u64;
            }
        }
        Ok((n, bytes))
    }

    fn object_handles(&mut self, dev: &mut dyn Device) {
        self.extra(3);
        let walk = match self.handles_walk(dev) {
            Ok(w) => w,
            Err(code) => {
                self.respond(code, &[]);
                return;
            }
        };
        let n = match self.count(dev, walk, None) {
            Ok((n, _)) => n,
            Err(Stop(code)) => {
                if code == rc::DEVICE_BUSY {
                    self.stats.table_full += 1;
                }
                self.respond(code, &[]);
                return;
            }
        };
        if self.op == op::GET_NUM_OBJECTS {
            self.respond(rc::OK, &[n]);
            return;
        }
        self.buf[HEADER..HEADER + 4].copy_from_slice(&n.to_le_bytes());
        self.start_in(
            Some(4 + 4 * u64::from(n)),
            4,
            Gen::Handles { walk, left: n },
        );
    }

    fn object_info(&mut self, dev: &mut dyn Device) {
        self.extra(1);
        let h = self.p(0);
        let mut e = Entry::default();
        let slot = match self.object(dev, h, &mut e) {
            Ok(s) => s,
            Err(code) => {
                self.respond(code, &[]);
                return;
            }
        };
        let picture = folder_picture(dev, slot.store, &e);
        let mut w = Writer::new(&mut self.buf[HEADER..]);
        w.u32(store_id(slot.store, true));
        w.u16(format_of(&e));
        w.u16(protection(&e));
        w.u32(e.size.min(u64::from(u32::MAX)) as u32);
        match picture {
            Some(p) => {
                w.u16(p.format);
                w.u32(p.size as u32);
                w.u32(p.width);
                w.u32(p.height);
            }
            None => {
                w.u16(0); // thumbnail format
                w.u32(0);
                w.u32(0);
                w.u32(0);
            }
        }
        w.u32(0); // image width, height, depth
        w.u32(0);
        w.u32(0);
        w.u32(slot.parent);
        w.u16(u16::from(e.folder));
        w.u32(0);
        w.u32(0); // sequence number
        w.text(&e.name);
        w.time(e.created.as_ref());
        w.time(e.modified.as_ref());
        w.u8(0); // keywords
        let len = w.len();
        self.start_in(Some(len as u64), len, Gen::Done);
    }

    fn get_object(&mut self, dev: &mut dyn Device) {
        let partial = self.op == op::GET_PARTIAL_OBJECT;
        self.extra(if partial { 3 } else { 1 });
        let h = self.p(0);
        let mut e = Entry::default();
        let slot = match self.object(dev, h, &mut e) {
            Ok(s) => s,
            Err(code) => {
                self.respond(code, &[]);
                return;
            }
        };
        if e.folder {
            self.respond(rc::INVALID_OBJECT_HANDLE, &[]);
            return;
        }
        let (off, end) = if partial {
            let off = u64::from(self.p(1));
            if off > e.size {
                self.respond(rc::INVALID_PARAMETER, &[]);
                return;
            }
            let max = self.p(2);
            let end = if max == u32::MAX {
                e.size
            } else {
                e.size.min(off + u64::from(max))
            };
            (off, end)
        } else {
            (0, e.size)
        };
        let source = if end > off {
            Gen::Object {
                s: slot.store,
                loc: slot.loc,
                off,
                end,
            }
        } else {
            Gen::Done
        };
        let payload = end - off;
        let known = payload < u64::from(u32::MAX) - HEADER as u64;
        self.start_in(known.then_some(payload), 0, source);
    }

    /// GetThumb [MTP D.2.10]: a folder's picture ([`FOLDER_PICTURES`]).
    fn get_thumb(&mut self, dev: &mut dyn Device) {
        self.extra(1);
        let h = self.p(0);
        let mut e = Entry::default();
        let slot = match self.object(dev, h, &mut e) {
            Ok(s) => s,
            Err(code) => {
                self.respond(code, &[]);
                return;
            }
        };
        match folder_picture(dev, slot.store, &e) {
            Some(p) if p.size > 0 => self.start_in(
                Some(p.size),
                0,
                Gen::Object {
                    s: slot.store,
                    loc: p.loc,
                    off: 0,
                    end: p.size,
                },
            ),
            _ => self.respond(rc::NO_THUMBNAIL_PRESENT, &[]),
        }
    }

    fn delete_object(&mut self, dev: &mut dyn Device) {
        self.extra(2);
        let (h, format) = (self.p(0), self.p(1));
        if h == u32::MAX {
            // "Everything on the device": no single operation from a cable
            // empties every card (ADR-0037 rule 11).
            self.respond(rc::INVALID_OBJECT_HANDLE, &[]);
            return;
        }
        if format != 0 {
            self.respond(rc::SPECIFICATION_BY_FORMAT_UNSUPPORTED, &[]);
            return;
        }
        let mut e = Entry::default();
        let slot = match self.object(dev, h, &mut e) {
            Ok(s) => s,
            Err(code) => {
                self.respond(code, &[]);
                return;
            }
        };
        if e.protected {
            self.respond(rc::OBJECT_WRITE_PROTECTED, &[]);
            return;
        }
        if dev.info(slot.store).is_some_and(|i| !i.writable) {
            self.respond(rc::STORE_READ_ONLY, &[]);
            return;
        }
        let code = if e.folder {
            self.delete_tree(dev, slot.store, slot.loc)
        } else {
            match dev.delete(slot.store, slot.loc) {
                Ok(()) => rc::OK,
                Err(err) => store_rc(err),
            }
        };
        if code == rc::OK {
            self.handles.kill(h);
        }
        self.respond(code, &[]);
    }

    /// Deletes a folder and what it holds, children first, with a fixed
    /// stack; a protected object (and so its folders) is kept:
    /// Partial_Deletion [MTP D.2.11].
    fn delete_tree(&mut self, dev: &mut dyn Device, s: u8, top: Loc) -> u16 {
        // (folder, cursor, unused, kept something)
        let mut depth = 1;
        self.stack[0] = (top, 0, 0, false);
        let mut kept_any = false;
        let mut deleted_any = false;
        let mut failed = None;
        let mut e = Entry::default();
        while depth > 0 {
            let (dir, mut cursor, _, kept) = self.stack[depth - 1];
            let more = dev.child(s, dir, &mut cursor, &mut e);
            self.stack[depth - 1].1 = cursor;
            match more {
                Ok(true) => {
                    if e.protected {
                        self.stack[depth - 1].3 = true;
                        kept_any = true;
                    } else if e.folder {
                        if depth == self.stack.len() {
                            self.stack[depth - 1].3 = true;
                            kept_any = true;
                        } else {
                            self.stack[depth] = (e.loc, 0, 0, false);
                            depth += 1;
                        }
                    } else {
                        match dev.delete(s, e.loc) {
                            Ok(()) => {
                                deleted_any = true;
                                if let Some(h) = self.handles.find(s, e.loc) {
                                    self.handles.kill(h);
                                }
                            }
                            Err(err) => {
                                failed.get_or_insert(err);
                                self.stack[depth - 1].3 = true;
                                kept_any = true;
                            }
                        }
                    }
                }
                Ok(false) => {
                    depth -= 1;
                    if kept {
                        if depth > 0 {
                            self.stack[depth - 1].3 = true;
                        }
                    } else {
                        match dev.delete(s, dir) {
                            Ok(()) => {
                                deleted_any = true;
                                if let Some(h) = self.handles.find(s, dir) {
                                    self.handles.kill(h);
                                }
                            }
                            Err(err) => {
                                failed.get_or_insert(err);
                                kept_any = true;
                                if depth > 0 {
                                    self.stack[depth - 1].3 = true;
                                }
                            }
                        }
                    }
                }
                Err(err) => {
                    return store_rc(err);
                }
            }
        }
        match (kept_any, deleted_any, failed) {
            (false, _, _) => rc::OK,
            (true, true, _) => rc::PARTIAL_DELETION,
            (true, false, Some(f)) if f != StoreError::Protected => store_rc(f),
            (true, false, _) => rc::OBJECT_WRITE_PROTECTED,
        }
    }

    /// Where a new object goes: `p1` the store (0: the device chooses),
    /// `p2` the folder (0: the device chooses, 0xFFFFFFFF the root) →
    /// store, folder location, folder handle.
    fn destination(&mut self, dev: &mut dyn Device, size: u64) -> Result<(u8, Loc, u32), u16> {
        let (sid, parent) = (self.p(0), self.p(1));
        let s = if sid == 0 {
            // The first writable store with room.
            (0..dev.stores())
                .find(|&s| dev.info(s).is_some_and(|i| i.writable && i.free >= size))
                .ok_or(rc::STORE_FULL)?
        } else {
            store_of(sid, dev.stores()).ok_or(rc::INVALID_STORAGE_ID)?
        };
        // The order of [MTP 5.3.4.1]: present, writable, room, folder.
        let info = dev.info(s).ok_or(rc::STORE_NOT_AVAILABLE)?;
        if !info.writable {
            return Err(rc::STORE_READ_ONLY);
        }
        if size > info.max_file {
            return Err(rc::OBJECT_TOO_LARGE);
        }
        if size > info.free {
            return Err(rc::STORE_FULL);
        }
        match parent {
            0 | u32::MAX => Ok((s, ROOT, 0)),
            h => {
                let mut e = Entry::default();
                let slot = self.object(dev, h, &mut e)?;
                if !e.folder {
                    return Err(rc::INVALID_PARENT_OBJECT);
                }
                if slot.store != s {
                    return Err(rc::INVALID_PARENT_OBJECT);
                }
                Ok((s, slot.loc, h))
            }
        }
    }

    /// Creates `new` where `p1`/`p2` say and answers with its place and
    /// handle (a file with contents waits for SendObject).
    fn create(&mut self, dev: &mut dyn Device, new: &NewObject) {
        let size = if new.folder { 0 } else { new.size };
        let (s, dir, ph) = match self.destination(dev, size) {
            Ok(d) => d,
            Err(code) => {
                self.respond(code, &[]);
                return;
            }
        };
        let loc = match dev.create(s, dir, new) {
            Ok(l) => l,
            Err(e) => {
                self.respond(store_rc(e), &[]);
                return;
            }
        };
        let h = match self.handles.get_or_add(s, loc, ph, new.folder) {
            Ok(h) => h,
            Err(_) => {
                self.stats.table_full += 1;
                if size > 0 {
                    dev.abandon(s, loc);
                }
                self.respond(rc::DEVICE_BUSY, &[]);
                return;
            }
        };
        if size > 0 {
            self.pending = Some(Pending {
                s,
                loc,
                handle: h,
                parent: ph,
                size,
            });
        }
        let parent = if ph == 0 { u32::MAX } else { ph };
        self.respond(rc::OK, &[store_id(s, true), parent, h]);
    }

    fn send_object_info(&mut self, dev: &mut dyn Device) {
        let n = self.len;
        let parsed = (|| -> Result<NewObject, Bad> {
            let mut r = Reader::new(&self.buf[..n]);
            r.u32()?; // storage: from the parameters
            let format = r.u16()?;
            let protection = r.u16()?;
            let size = r.u32()?;
            // Thumbnail format, then thumbnail size, width, height and image
            // width, height, depth [MTP 5.3.1].
            r.skip(2 + 6 * 4)?;
            r.u32()?; // parent: from the parameters
            let assoc = r.u16()?;
            r.u32()?; // association description
            r.u32()?; // sequence number
            let name = r.text()?;
            let created = Time::parse(&r.text()?)?;
            let modified = Time::parse(&r.text()?)?;
            // Keywords may be missing on older initiators.
            if protection != 0 {
                return Err(Bad);
            }
            let folder = format == fmt::ASSOCIATION;
            if folder && assoc != 0 && assoc != 1 {
                return Err(Bad);
            }
            Ok(NewObject {
                name,
                folder,
                size: u64::from(size),
                created,
                modified,
                hidden: false,
            })
        })();
        match parsed {
            Ok(new) if !new.name.is_empty() => {
                if !new.folder && new.size == u64::from(u32::MAX) {
                    // Over 4 GB announced without its size: it can't be
                    // reserved whole; SendObjectPropList carries 64 bits.
                    self.respond(rc::OBJECT_TOO_LARGE, &[]);
                } else {
                    self.create(dev, &new);
                }
            }
            _ => self.respond(rc::INVALID_DATASET, &[]),
        }
    }

    fn send_object_prop_list(&mut self, dev: &mut dyn Device) {
        let format = self.p(2);
        let size = u64::from(self.p(3)) << 32 | u64::from(self.p(4));
        let n = self.len;
        let mut new = NewObject {
            folder: format == u32::from(fmt::ASSOCIATION),
            size,
            ..NewObject::default()
        };
        let mut failed = None;
        {
            let mut r = Reader::new(&self.buf[..n]);
            let count = r.u32().unwrap_or(u32::MAX);
            if count == u32::MAX || u64::from(count) * 8 > n as u64 {
                failed = Some((rc::INVALID_DATASET, 0));
            } else {
                for i in 0..count {
                    let ok = (|| -> Result<Option<u16>, Bad> {
                        let handle = r.u32()?;
                        let code = r.u16()?;
                        let t = r.u16()?;
                        if handle != 0 || prop_type(code).is_some_and(|want| want != t) {
                            return Ok(Some(rc::INVALID_OBJECT_PROP_FORMAT));
                        }
                        match code {
                            prop::OBJECT_FILE_NAME => new.name = r.text()?,
                            prop::NAME => {
                                r.text()?;
                            }
                            prop::DATE_CREATED => match Time::parse(&r.text()?) {
                                Ok(t) => new.created = t,
                                Err(Bad) => return Ok(Some(rc::INVALID_OBJECT_PROP_VALUE)),
                            },
                            prop::DATE_MODIFIED => match Time::parse(&r.text()?) {
                                Ok(t) => new.modified = t,
                                Err(Bad) => return Ok(Some(rc::INVALID_OBJECT_PROP_VALUE)),
                            },
                            prop::PROTECTION_STATUS => {
                                if r.u16()? != 0 {
                                    return Ok(Some(rc::INVALID_OBJECT_PROP_VALUE));
                                }
                            }
                            prop::HIDDEN => new.hidden = r.u16()? == 1,
                            prop::SYSTEM_OBJECT | prop::ASSOCIATION_TYPE => {
                                r.u16()?;
                            }
                            prop::ASSOCIATION_DESC => {
                                r.u32()?;
                            }
                            prop::PERSISTENT_UID => {
                                r.skip(16)?;
                            }
                            _ => {
                                // A property we don't keep: its value is
                                // skipped by its type, and refused.
                                r.skip_value(t)?;
                                return Ok(Some(rc::OBJECT_PROP_NOT_SUPPORTED));
                            }
                        }
                        Ok(None)
                    })();
                    match ok {
                        Ok(None) => {}
                        Ok(Some(code)) => {
                            failed = Some((code, i));
                            break;
                        }
                        Err(Bad) => {
                            failed = Some((rc::INVALID_DATASET, i));
                            break;
                        }
                    }
                }
            }
        }
        if let Some((code, i)) = failed {
            self.respond(code, &[0, 0, 0, i]);
            return;
        }
        if new.name.is_empty() {
            self.respond(rc::INVALID_DATASET, &[0, 0, 0, 0]);
            return;
        }
        self.create(dev, &new);
    }

    fn format_store(&mut self, dev: &mut dyn Device) {
        self.extra(2);
        let Some(s) = store_of(self.p(0), dev.stores()) else {
            self.respond(rc::INVALID_STORAGE_ID, &[]);
            return;
        };
        if dev.info(s).is_none() {
            self.respond(rc::STORE_NOT_AVAILABLE, &[]);
            return;
        }
        // The card RetroPane boots from is never formatted from the PC
        // (ADR-0037 Q3); any other only with the user's yes on the device.
        if dev.boots_from(s) {
            self.respond(rc::STORE_READ_ONLY, &[]);
            return;
        }
        let fs = self.p(1);
        if fs != 0 && fs != 0x0002 {
            self.respond(rc::INVALID_PARAMETER, &[]);
            return;
        }
        self.consent = (s, dev.now_ms());
        match dev.format_consent(s, true) {
            Consent::Waiting => self.phase = Phase::Consent,
            answer => self.consent_given(dev, answer),
        }
    }

    // Its own frame (not the poll loop's): only the phase running is on
    // the owner's stack (the loader's is 32 KiB, ADR-0037 rule 7).
    #[inline(never)]
    fn wait_consent(&mut self, dev: &mut dyn Device) -> bool {
        let (s, since) = self.consent;
        let answer = if dev.now_ms().wrapping_sub(since) >= CONSENT_MS {
            Consent::No
        } else {
            dev.format_consent(s, false)
        };
        if answer == Consent::Waiting {
            return false;
        }
        self.consent_given(dev, answer);
        true
    }

    fn consent_given(&mut self, dev: &mut dyn Device, answer: Consent) {
        let s = self.consent.0;
        if answer != Consent::Yes {
            self.respond(rc::ACCESS_DENIED, &[]);
            return;
        }
        let r = dev.format(s);
        self.handles.kill_store(s);
        match r {
            Ok(()) => {
                // The card is a new volume: the PC lists it afresh
                // (ADR-0037, FormatStore).
                self.store_changed(s, false);
                self.store_changed(s, true);
                self.respond(rc::OK, &[]);
            }
            Err(e) => self.respond(store_rc(e), &[]),
        }
    }

    fn device_prop(&mut self, dev: &mut dyn Device) {
        self.extra(1);
        let desc = self.op == op::GET_DEVICE_PROP_DESC;
        let code = self.p(0);
        let Ok(code) = u16::try_from(code) else {
            self.respond(rc::DEVICE_PROP_NOT_SUPPORTED, &[]);
            return;
        };
        let name = dev.friendly_name();
        let icon = dev.icon_len();
        let mut w = Writer::new(&mut self.buf[HEADER..]);
        let tail = match code {
            dprop::FRIENDLY_NAME | dprop::SESSION_INITIATOR => {
                let value = if code == dprop::FRIENDLY_NAME {
                    name
                } else {
                    Text::new()
                };
                if desc {
                    w.u16(code);
                    w.u16(ty::STR);
                    w.u8(1); // get/set
                    w.text(&value); // default: what it is now
                    w.text(&value);
                    w.u8(0); // no form
                } else {
                    w.text(&value);
                }
                None
            }
            dprop::PERCEIVED_TYPE => {
                // 5: personal information manager / PDA [MTP C.2.43].
                if desc {
                    w.u16(code);
                    w.u16(ty::UINT32);
                    w.u8(0);
                    w.u32(5);
                    w.u32(5);
                    w.u8(0);
                } else {
                    w.u32(5);
                }
                None
            }
            dprop::DEVICE_ICON if icon > 0 => {
                if desc {
                    w.u16(code);
                    w.u16(ty::AUINT8);
                    w.u8(0);
                    w.u32(0); // default: empty
                }
                w.u32(icon);
                Some(desc)
            }
            _ => {
                self.respond(rc::DEVICE_PROP_NOT_SUPPORTED, &[]);
                return;
            }
        };
        let n = w.len();
        match tail {
            None => self.start_in(Some(n as u64), n, Gen::Done),
            Some(desc) => {
                let total = n as u64 + u64::from(icon) + u64::from(desc);
                self.start_in(
                    Some(total),
                    n,
                    Gen::Icon {
                        off: 0,
                        end: icon,
                        tail: desc,
                    },
                );
            }
        }
    }

    fn set_device_prop(&mut self, dev: &mut dyn Device) {
        let code = self.p(0);
        let n = self.len;
        let value = {
            let mut r = Reader::new(&self.buf[..n]);
            r.text().ok().filter(|_| r.left() == 0)
        };
        match (u16::try_from(code), value) {
            (Ok(dprop::FRIENDLY_NAME), Some(t)) if !t.is_empty() => {
                dev.set_friendly_name(&t);
                self.respond(rc::OK, &[]);
            }
            (Ok(dprop::FRIENDLY_NAME), _) => self.respond(rc::INVALID_DEVICE_PROP_VALUE, &[]),
            (Ok(dprop::SESSION_INITIATOR), Some(t)) => {
                dev.initiator(&t);
                self.respond(rc::OK, &[]);
            }
            (Ok(dprop::SESSION_INITIATOR), None) => {
                self.respond(rc::INVALID_DEVICE_PROP_FORMAT, &[])
            }
            (Ok(dprop::PERCEIVED_TYPE | dprop::DEVICE_ICON), _) => {
                self.respond(rc::ACCESS_DENIED, &[])
            }
            _ => self.respond(rc::DEVICE_PROP_NOT_SUPPORTED, &[]),
        }
    }

    fn move_or_copy(&mut self, dev: &mut dyn Device) {
        self.extra(3);
        let copy = self.op == op::COPY_OBJECT;
        let (h, sid, parent) = (self.p(0), self.p(1), self.p(2));
        let mut e = Entry::default();
        let slot = match self.object(dev, h, &mut e) {
            Ok(s) => s,
            Err(code) => {
                self.respond(code, &[]);
                return;
            }
        };
        if e.protected && !copy {
            self.respond(rc::OBJECT_WRITE_PROTECTED, &[]);
            return;
        }
        let Some(ds) = store_of(sid, dev.stores()) else {
            self.respond(rc::INVALID_STORAGE_ID, &[]);
            return;
        };
        let Some(info) = dev.info(ds) else {
            self.respond(rc::STORE_NOT_AVAILABLE, &[]);
            return;
        };
        if !info.writable {
            self.respond(rc::STORE_READ_ONLY, &[]);
            return;
        }
        let (dir, dh) = if parent == 0 || parent == u32::MAX {
            (ROOT, 0)
        } else {
            let mut pe = Entry::default();
            match self.object(dev, parent, &mut pe) {
                Ok(ps) if pe.folder && ps.store == ds => (ps.loc, parent),
                Ok(_) => {
                    self.respond(rc::INVALID_PARENT_OBJECT, &[]);
                    return;
                }
                Err(code) => {
                    self.respond(code, &[]);
                    return;
                }
            }
        };
        // A folder into itself or below it would orphan the tree.
        if e.folder
            && ds == slot.store
            && (dh == h || self.handles.under(dh, ds, h, u32::MAX).is_some())
        {
            self.respond(rc::INVALID_PARENT_OBJECT, &[]);
            return;
        }
        if !copy && ds == slot.store {
            let Some(from) = self.parent_loc(&slot) else {
                self.respond(rc::INVALID_OBJECT_HANDLE, &[]);
                return;
            };
            match dev.relocate(slot.store, from, slot.loc, dir) {
                Ok(loc) => {
                    self.handles.relocate(h, ds, loc, dh);
                    self.respond(rc::OK, &[]);
                }
                Err(err) => self.respond(store_rc(err), &[]),
            }
            return;
        }
        // A copy (or a move between cards: copy, then delete the original).
        let new = match self.copy_tree(dev, slot.store, &e, ds, dir) {
            Ok(loc) => loc,
            Err(code) => {
                self.respond(code, &[]);
                return;
            }
        };
        if copy {
            match self.handles.get_or_add(ds, new, dh, e.folder) {
                Ok(nh) => self.respond(rc::OK, &[nh]),
                Err(_) => {
                    // The copy is on the card; the PC finds it when it
                    // lists the folder again.
                    self.stats.table_full += 1;
                    self.queue(ev::UNREPORTED_STATUS, &[]);
                    self.respond(rc::OK, &[0]);
                }
            }
            return;
        }
        // The object keeps its handle across a move [MTP D.2.25]: it names
        // the copy before the original goes (whose handles the delete
        // kills).
        self.handles.relocate(h, ds, new, dh);
        let code = if e.folder {
            self.delete_tree(dev, slot.store, slot.loc)
        } else {
            dev.delete(slot.store, slot.loc)
                .map_or_else(store_rc, |()| rc::OK)
        };
        self.respond(
            if code == rc::OK {
                rc::OK
            } else {
                rc::PARTIAL_DELETION
            },
            &[],
        );
    }

    /// Copies one file through the staging buffer (on the device: no USB
    /// traffic).
    fn copy_file(
        &mut self,
        dev: &mut dyn Device,
        s: u8,
        e: &Entry,
        ds: u8,
        dir: Loc,
    ) -> Result<Loc, u16> {
        let new = NewObject {
            name: e.name,
            folder: false,
            size: e.size,
            created: e.created,
            modified: e.modified,
            hidden: e.hidden,
        };
        if let Some(i) = dev.info(ds) {
            if e.size > i.max_file {
                return Err(rc::OBJECT_TOO_LARGE);
            }
            if e.size > i.free {
                return Err(rc::STORE_FULL);
            }
        }
        let loc = dev.create(ds, dir, &new).map_err(store_rc)?;
        if e.size == 0 {
            return Ok(loc);
        }
        let mut off = 0u64;
        while off < e.size {
            let want = (e.size - off).min(self.buf.len() as u64) as usize;
            let r = dev.read(s, e.loc, off, &mut self.buf[..want]);
            let n = match r {
                Ok(n) if n > 0 => n,
                Ok(_) => {
                    dev.abandon(ds, loc);
                    return Err(rc::INCOMPLETE_TRANSFER);
                }
                Err(err) => {
                    dev.abandon(ds, loc);
                    return Err(store_rc(err));
                }
            };
            if let Err(err) = dev.write(ds, loc, off, &self.buf[..n]) {
                dev.abandon(ds, loc);
                return Err(store_rc(err));
            }
            off += n as u64;
        }
        dev.commit(ds, loc).map_err(|err| {
            dev.abandon(ds, loc);
            store_rc(err)
        })
    }

    /// Copies a file or a whole folder (the owner's walk table).
    fn copy_tree(
        &mut self,
        dev: &mut dyn Device,
        s: u8,
        e: &Entry,
        ds: u8,
        dir: Loc,
    ) -> Result<Loc, u16> {
        if !e.folder {
            return self.copy_file(dev, s, e, ds, dir);
        }
        let mk = |name: Text, e: &Entry| NewObject {
            name,
            folder: true,
            created: e.created,
            modified: e.modified,
            hidden: e.hidden,
            ..NewObject::default()
        };
        let top = dev.create(ds, dir, &mk(e.name, e)).map_err(store_rc)?;
        // (source folder, cursor, destination folder)
        self.stack[0] = (e.loc, 0, top, false);
        let mut depth = 1;
        let mut c = Entry::default();
        while depth > 0 {
            let (src, mut cursor, dst, _) = self.stack[depth - 1];
            let more = dev.child(s, src, &mut cursor, &mut c).map_err(store_rc)?;
            self.stack[depth - 1].1 = cursor;
            if !more {
                depth -= 1;
                continue;
            }
            if c.folder {
                if depth == self.stack.len() {
                    return Err(rc::GENERAL_ERROR);
                }
                let sub = dev.create(ds, dst, &mk(c.name, &c)).map_err(store_rc)?;
                self.stack[depth] = (c.loc, 0, sub, false);
                depth += 1;
            } else {
                let child = c;
                self.copy_file(dev, s, &child, ds, dst)?;
            }
        }
        Ok(top)
    }

    fn props_supported(&mut self) {
        self.extra(1);
        let format = self.p(0);
        if u16::try_from(format).map_or(true, |f| !FORMATS.contains(&f)) {
            self.respond(rc::INVALID_OBJECT_FORMAT_CODE, &[]);
            return;
        }
        let mut w = Writer::new(&mut self.buf[HEADER..]);
        w.a16(&PROPS);
        let n = w.len();
        self.start_in(Some(n as u64), n, Gen::Done);
    }

    fn prop_desc(&mut self) {
        self.extra(2);
        let (code, format) = (self.p(0), self.p(1));
        let Some(code) = u16::try_from(code).ok().filter(|c| PROPS.contains(c)) else {
            self.respond(rc::INVALID_OBJECT_PROP_CODE, &[]);
            return;
        };
        if u16::try_from(format).map_or(true, |f| !FORMATS.contains(&f)) {
            self.respond(rc::INVALID_OBJECT_FORMAT_CODE, &[]);
            return;
        }
        let t = prop_type(code).unwrap_or(ty::UINT32);
        let mut w = Writer::new(&mut self.buf[HEADER..]);
        w.u16(code);
        w.u16(t);
        // Only the name can be set (a rename); everything else is the card's.
        w.u8(u8::from(code == prop::OBJECT_FILE_NAME));
        match t {
            ty::UINT16 => w.u16(0),
            ty::UINT32 => w.u32(0),
            ty::UINT64 => w.u64(0),
            ty::UINT128 => w.u128(0),
            _ => w.u8(0),
        }
        w.u32(GROUP);
        match code {
            prop::PROTECTION_STATUS => {
                w.u8(2);
                w.u16(2);
                w.u16(0x0000);
                w.u16(0x0001);
            }
            prop::ASSOCIATION_TYPE | prop::HIDDEN | prop::SYSTEM_OBJECT => {
                w.u8(2);
                w.u16(2);
                w.u16(0);
                w.u16(1);
            }
            prop::DATE_CREATED | prop::DATE_MODIFIED => w.u8(3),
            prop::OBJECT_FILE_NAME => {
                w.u8(5);
                w.ascii(NAME_PATTERN.as_bytes());
            }
            _ => w.u8(0),
        }
        let n = w.len();
        self.start_in(Some(n as u64), n, Gen::Done);
    }

    fn prop_value(&mut self, dev: &mut dyn Device) {
        self.extra(2);
        let (h, code) = (self.p(0), self.p(1));
        let mut e = Entry::default();
        let slot = match self.object(dev, h, &mut e) {
            Ok(s) => s,
            Err(code) => {
                self.respond(code, &[]);
                return;
            }
        };
        let Some(code) = u16::try_from(code).ok().filter(|c| PROPS.contains(c)) else {
            self.respond(rc::INVALID_OBJECT_PROP_CODE, &[]);
            return;
        };
        let mut w = Writer::new(&mut self.buf[HEADER..]);
        write_value(&mut w, code, slot.store, slot.parent, &e);
        let n = w.len();
        self.start_in(Some(n as u64), n, Gen::Done);
    }

    fn prop_list(&mut self, dev: &mut dyn Device) {
        let (h, format, code, group, depth) =
            (self.p(0), self.p(1), self.p(2), self.p(3), self.p(4));
        let Ok(format) = u16::try_from(format) else {
            self.respond(rc::INVALID_OBJECT_FORMAT_CODE, &[]);
            return;
        };
        let prop = match code {
            u32::MAX => 0xFFFF,
            0 => {
                if group == 0 {
                    self.respond(rc::PARAMETER_NOT_SUPPORTED, &[]);
                    return;
                }
                if group != GROUP {
                    self.respond(rc::GROUP_NOT_SUPPORTED, &[]);
                    return;
                }
                0xFFFF
            }
            c => match u16::try_from(c).ok().filter(|c| PROPS.contains(c)) {
                Some(c) => c,
                None => {
                    self.respond(rc::OBJECT_PROP_NOT_SUPPORTED, &[]);
                    return;
                }
            },
        };
        let walk = match h {
            u32::MAX => Walk::new(None, 0, false, u32::MAX, format),
            0 => {
                if depth == 0 {
                    // The root with no depth: the empty set [MTP E.2.1].
                    self.buf[HEADER..HEADER + 4].fill(0);
                    self.start_in(Some(4), 4, Gen::Done);
                    return;
                }
                // Every store's root objects, down to `depth`.
                Walk::new(None, 0, false, depth, format)
            }
            h => {
                let mut e = Entry::default();
                let slot = match self.object(dev, h, &mut e) {
                    Ok(s) => s,
                    Err(code) => {
                        self.respond(code, &[]);
                        return;
                    }
                };
                Walk::new(
                    Some(slot.store),
                    h,
                    true,
                    if e.folder { depth } else { 0 },
                    format,
                )
            }
        };
        let (n, bytes) = match self.count(dev, walk, Some(prop)) {
            Ok(c) => c,
            Err(Stop(code)) => {
                if code == rc::DEVICE_BUSY {
                    self.stats.table_full += 1;
                }
                self.respond(code, &[]);
                return;
            }
        };
        // The element count first: one per (object, property) [MTP E.2.1.1].
        let per = if prop == 0xFFFF {
            PROPS.len() as u32
        } else {
            1
        };
        let count = n * per;
        self.buf[HEADER..HEADER + 4].copy_from_slice(&count.to_le_bytes());
        self.start_in(
            Some(4 + bytes),
            4,
            Gen::Props {
                walk,
                prop,
                left: n,
            },
        );
    }

    fn set_prop_value(&mut self, dev: &mut dyn Device) {
        let (h, code) = (self.p(0), self.p(1));
        let n = self.len;
        let name = {
            let mut r = Reader::new(&self.buf[..n]);
            r.text().ok().filter(|_| r.left() == 0)
        };
        let code = rename_check(code);
        let code = match code {
            Ok(()) => match name {
                Some(name) if !name.is_empty() => self.rename(dev, h, &name),
                _ => rc::INVALID_OBJECT_PROP_FORMAT,
            },
            Err(c) => c,
        };
        self.respond(code, &[]);
    }

    fn set_prop_list(&mut self, dev: &mut dyn Device) {
        let n = self.len;
        let total = match Reader::new(&self.buf[..n]).u32() {
            Ok(t) => t,
            Err(Bad) => {
                self.respond(rc::INVALID_DATASET, &[0]);
                return;
            }
        };
        // Element by element, applied as read; the first failure stops the
        // rest and names its index [MTP E.2.2].
        let mut at = 4;
        for i in 0..total {
            let parsed = {
                let mut r = Reader::new(&self.buf[at..n]);
                let one = (|| -> Result<(u32, u16, u16, Text), Bad> {
                    let (h, code, t) = (r.u32()?, r.u16()?, r.u16()?);
                    let name = if t == ty::STR { r.text()? } else { Text::new() };
                    Ok((h, code, t, name))
                })();
                one.map(|v| (v, n - at - r.left()))
            };
            let Ok(((h, code, t, name), used)) = parsed else {
                self.respond(rc::INVALID_DATASET, &[i]);
                return;
            };
            at += used;
            let result = match rename_check(u32::from(code)) {
                Err(c) => c,
                Ok(()) if t != ty::STR => rc::INVALID_OBJECT_PROP_FORMAT,
                Ok(()) if name.is_empty() => rc::INVALID_OBJECT_PROP_VALUE,
                Ok(()) => self.rename(dev, h, &name),
            };
            if result != rc::OK {
                self.respond(result, &[i]);
                return;
            }
        }
        self.respond(rc::OK, &[0]);
    }

    fn rename(&mut self, dev: &mut dyn Device, h: u32, name: &Text) -> u16 {
        let mut e = Entry::default();
        let slot = match self.object(dev, h, &mut e) {
            Ok(s) => s,
            Err(code) => return code,
        };
        if e.protected || e.long_name {
            return rc::OBJECT_WRITE_PROTECTED;
        }
        let Some(dir) = self.parent_loc(&slot) else {
            return rc::INVALID_OBJECT_HANDLE;
        };
        match dev.rename(slot.store, dir, slot.loc, name) {
            Ok(loc) => {
                self.handles.relocate(h, slot.store, loc, slot.parent);
                rc::OK
            }
            Err(StoreError::BadName | StoreError::Exists) => rc::INVALID_OBJECT_PROP_VALUE,
            Err(err) => store_rc(err),
        }
    }

    /// The location of the folder holding a live object ([`ROOT`] for the
    /// root's children).
    fn parent_loc(&self, slot: &Slot) -> Option<Loc> {
        if slot.parent == 0 {
            Some(ROOT)
        } else {
            self.handles.get(slot.parent).map(|p| p.loc)
        }
    }

    fn references(&mut self) {
        self.extra(1);
        let h = self.p(0);
        if self.handles.get(h).is_none() {
            self.respond(rc::INVALID_OBJECT_HANDLE, &[]);
            return;
        }
        // No references are kept: always the empty array.
        self.buf[HEADER..HEADER + 4].fill(0);
        self.start_in(Some(4), 4, Gen::Done);
    }

    // --- Walks ------------------------------------------------------------

    /// The next object of a walk, assigning its handle. Children of a
    /// folder come from the store's cursor; deeper levels from a scan of
    /// the handle table, which holds every folder found so far (newly found
    /// ones at its end), so a walk of any depth needs no stack and no RAM
    /// beyond the handles.
    fn walk_next(
        &mut self,
        dev: &mut dyn Device,
        w: &mut Walk,
        e: &mut Entry,
    ) -> Result<Option<u32>, Stop> {
        loop {
            match w.stage {
                0 => {
                    // The store to walk: the next one with a medium.
                    if w.top == 0 {
                        let stores = dev.stores();
                        let next = match w.store {
                            Some(s) => (w.s <= s).then_some(s),
                            None => (w.s..stores).find(|&s| dev.info(s).is_some()),
                        };
                        let Some(s) = next else {
                            w.stage = 4;
                            continue;
                        };
                        w.s = s;
                        w.dir = (0, ROOT, 0);
                    } else {
                        let slot = self
                            .handles
                            .get(w.top)
                            .ok_or(Stop(rc::INVALID_OBJECT_HANDLE))?;
                        w.s = slot.store;
                        w.dir = (w.top, slot.loc, 0);
                        if w.yield_top {
                            w.stage = 1;
                            dev.stat(w.s, slot.loc, e)?;
                            if w.format == 0 || format_of(e) == w.format {
                                return Ok(Some(w.top));
                            }
                            continue;
                        }
                    }
                    w.stage = 1;
                }
                1 => {
                    // The top's children (level 1).
                    w.stage = 2;
                    w.cursor = 0;
                    w.k = 0;
                    w.listing = w.depth >= 1
                        && (w.top == 0 || self.handles.get(w.top).is_some_and(|s| s.folder));
                }
                2 => {
                    if w.listing {
                        let (dh, dloc, _) = w.dir;
                        match dev.child(w.s, dloc, &mut w.cursor, e) {
                            Ok(true) => {
                                let h = self.handles.get_or_add(w.s, e.loc, dh, e.folder)?;
                                if w.format == 0 || format_of(e) == w.format {
                                    return Ok(Some(h));
                                }
                                continue;
                            }
                            Ok(false) => {}
                            // A folder the device removed since the PC saw
                            // it (not the walk's own top): its handle goes.
                            Err(StoreError::NotFound) if dh != w.top && dh != 0 => {
                                self.handles.kill(dh);
                            }
                            Err(err) => return Err(err.into()),
                        }
                        w.listing = false;
                    }
                    // The next folder in the table under the top, above the
                    // depth limit.
                    let mut found = None;
                    while let Some(slot) = self.handles.at(w.k) {
                        let h = w.k as u32 + 1;
                        w.k += 1;
                        if !slot.folder || slot.store != w.s {
                            continue;
                        }
                        if let Some(level) =
                            self.handles.under(h, w.s, w.top, w.depth.saturating_sub(1))
                        {
                            if h == w.top {
                                continue; // already listed (stage 1)
                            }
                            found = Some((h, slot.loc, level));
                            break;
                        }
                    }
                    match found {
                        Some((h, loc, level)) if w.depth > 1 => {
                            w.dir = (h, loc, level);
                            w.cursor = 0;
                            w.listing = true;
                        }
                        _ => {
                            // This store is done: the next one, or the end.
                            if w.top == 0 && w.store.is_none() {
                                w.s += 1;
                                w.stage = 0;
                            } else {
                                w.stage = 4;
                            }
                        }
                    }
                }
                _ => return Ok(None),
            }
        }
    }
}

impl Walk {
    fn new(store: Option<u8>, top: u32, yield_top: bool, depth: u32, format: u16) -> Self {
        Self {
            store,
            s: store.unwrap_or(0),
            top,
            yield_top,
            depth,
            format,
            stage: 0,
            k: 0,
            dir: (0, ROOT, 0),
            cursor: 0,
            listing: false,
        }
    }
}

fn protection(e: &Entry) -> u16 {
    if e.protected || e.long_name {
        0x0001
    } else {
        0x0000
    }
}

fn rename_check(code: u32) -> Result<(), u16> {
    match u16::try_from(code) {
        Ok(prop::OBJECT_FILE_NAME) => Ok(()),
        Ok(c) if PROPS.contains(&c) => Err(rc::ACCESS_DENIED),
        _ => Err(rc::INVALID_OBJECT_PROP_CODE),
    }
}

/// One property's value [MTP B.2].
fn write_value(w: &mut Writer<'_>, code: u16, s: u8, parent: u32, e: &Entry) {
    match code {
        prop::STORAGE_ID => w.u32(store_id(s, true)),
        prop::OBJECT_FORMAT => w.u16(format_of(e)),
        prop::PROTECTION_STATUS => w.u16(protection(e)),
        prop::OBJECT_SIZE => w.u64(e.size),
        prop::ASSOCIATION_TYPE => w.u16(u16::from(e.folder)),
        prop::ASSOCIATION_DESC => w.u32(0),
        prop::OBJECT_FILE_NAME | prop::NAME => w.text(&e.name),
        prop::DATE_CREATED => w.time(e.created.as_ref()),
        prop::DATE_MODIFIED => w.time(e.modified.as_ref()),
        prop::PARENT_OBJECT => w.u32(parent),
        prop::HIDDEN => w.u16(u16::from(e.hidden)),
        prop::SYSTEM_OBJECT => w.u16(u16::from(e.system)),
        prop::PERSISTENT_UID => w.u128(e.uid),
        _ => {}
    }
}

/// An object's elements of a property list: every property (`0xFFFF`) or
/// one [MTP E.2.1.1].
fn write_props(w: &mut Writer<'_>, h: u32, s: u8, parent: u32, e: &Entry, which: u16) {
    for &code in &PROPS {
        if which != 0xFFFF && which != code {
            continue;
        }
        w.u32(h);
        w.u16(code);
        w.u16(prop_type(code).unwrap_or(ty::UINT32));
        write_value(w, code, s, parent, e);
    }
}

#[cfg(test)]
mod tests;
