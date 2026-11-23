//! The compressed page store (ADR-0005 rule 6): holds compressed copies of
//! idle anonymous pages, packed into frames.
//!
//! Compressed pages range from a few dozen bytes (a mostly-zero page) to
//! nearly 4 KiB, so they're packed by *size class*: class `k` holds objects
//! of up to `k × 64` bytes, and every frame holds slots of one class only.
//! Waste is under 64 bytes per object. Pages that compress to more than
//! [`MAX_OBJECT`] bytes (75% of a page) aren't worth storing compressed.
//!
//! **No side tables.** Each frame's bookkeeping lives in its first 16 bytes:
//! class, slot bitmap and the links of its class's list of frames with free
//! slots. The store itself is a few dozen bytes of list heads, so it costs
//! nothing until it holds pages, and nothing per frame it doesn't use.
//!
//! Frames come from a [`FrameAlloc`]; their bytes are reached through
//! [`Pages`] (the kernel's direct map). Each slot starts with the object's
//! 2-byte length.

use crate::pagetable::FrameAlloc;

/// Size-class granularity.
pub const GRANULE: usize = 64;
/// Largest object stored (bigger means poor compression: keep the page).
pub const MAX_OBJECT: usize = 3072;
/// Number of size classes.
pub const CLASSES: usize = (MAX_OBJECT + 2).div_ceil(GRANULE);

const PAGE: usize = 4096;

/// Bits a stored payload's low half gives class and slot (the compressed
/// store's [`Handle`] and the memory manager's aged pages share the layout
/// `frame << 16 | class << 8 | slot`).
pub(crate) const FRAME_SHIFT: u32 = 16;
/// Frames a stored payload can name: the payload has
/// [`STORED_BITS`](crate::arm::STORED_BITS) bits and the frame sits above
/// [`FRAME_SHIFT`], so 2^14 (64 MiB of 4 KiB frames). The frame allocator
/// refuses to own more ([`Frames::new`](crate::frames::Frames::new)), so
/// every frame number it hands out fits: no handle can wrap.
pub const MAX_FRAMES: usize = 1 << (crate::arm::STORED_BITS - FRAME_SHIFT);

/// Per-frame header: class (1), used count (1), next and prev frame + 1
/// (2 + 2), reserved (2), slot bitmap (8).
const HEADER: usize = 16;

/// Room for compressing one page: the LZ4 hash table and the output. Lives
/// in static memory (12 KiB, all zero: `.bss`), never on a kernel stack.
pub struct ZBuffers {
    /// The compressor's hash table.
    pub table: [u16; rp_lz4::TABLE_LEN],
    /// A compressed page.
    pub packed: [u8; rp_lz4::max_compressed(PAGE)],
}

impl Default for ZBuffers {
    fn default() -> Self {
        Self::new()
    }
}

impl ZBuffers {
    /// Zeroed buffers.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            table: [0; rp_lz4::TABLE_LEN],
            packed: [0; rp_lz4::max_compressed(PAGE)],
        }
    }
}

/// Frame memory, by frame number.
pub trait Pages {
    /// Frame `f`'s bytes.
    fn page(&mut self, f: usize) -> &mut [u8; PAGE];
}

/// A stored object: frame, class and slot.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub struct Handle(u32);

impl Handle {
    fn new(frame: usize, class: usize, slot: usize) -> Self {
        debug_assert!(frame < MAX_FRAMES && class < 1 << 8 && slot < 1 << 8);
        Self((frame as u32) << FRAME_SHIFT | (class as u32) << 8 | slot as u32)
    }
    fn frame(self) -> usize {
        (self.0 >> FRAME_SHIFT) as usize
    }
    fn class(self) -> usize {
        ((self.0 >> 8) & 0xFF) as usize
    }
    fn slot(self) -> usize {
        (self.0 & 0xFF) as usize
    }
    /// The raw value (for storing in a page-table-side index).
    #[must_use]
    pub fn raw(self) -> u32 {
        self.0
    }
    /// A handle from [`raw`](Self::raw).
    #[must_use]
    pub fn from_raw(raw: u32) -> Self {
        Self(raw)
    }
}

/// A set of frame numbers: one bit a frame, 2 KiB for all 64 MiB. The
/// compressed store keeps the frames it holds in one, and Game Mode's full
/// tier writes them to the card in this order and keeps the set as its page
/// map: a frame's place in the swap file is its rank in the set, so no
/// per-page table is stored at all (Linux's swap keeps a map entry a page).
#[derive(Clone, PartialEq, Eq)]
pub struct FrameSet {
    words: [u32; MAX_FRAMES / 32],
}

impl Default for FrameSet {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameSet {
    /// Words in the set.
    pub const WORDS: usize = MAX_FRAMES / 32;

    /// An empty set.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            words: [0; MAX_FRAMES / 32],
        }
    }

    /// Adds frame `f`.
    pub fn insert(&mut self, f: usize) {
        self.words[f / 32] |= 1 << (f % 32);
    }

    /// Removes frame `f`.
    pub fn remove(&mut self, f: usize) {
        self.words[f / 32] &= !(1 << (f % 32));
    }

    /// Whether frame `f` is in the set.
    #[must_use]
    pub fn contains(&self, f: usize) -> bool {
        f < MAX_FRAMES && self.words[f / 32] & 1 << (f % 32) != 0
    }

    /// Frames in the set.
    #[must_use]
    pub fn len(&self) -> usize {
        self.words.iter().map(|w| w.count_ones() as usize).sum()
    }

    /// Whether the set is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.words.iter().all(|&w| w == 0)
    }

    /// The frames, lowest first.
    pub fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        self.words.iter().enumerate().flat_map(|(i, &w)| {
            let mut w = w;
            core::iter::from_fn(move || {
                (w != 0).then(|| {
                    let b = w.trailing_zeros() as usize;
                    w &= w - 1;
                    i * 32 + b
                })
            })
        })
    }

    /// The raw words (for a record).
    #[must_use]
    pub fn words(&self) -> &[u32; MAX_FRAMES / 32] {
        &self.words
    }

    /// The raw words, to fill from a record.
    pub fn words_mut(&mut self) -> &mut [u32; MAX_FRAMES / 32] {
        &mut self.words
    }

    /// Empties the set.
    pub fn clear(&mut self) {
        self.words.fill(0);
    }
}

/// The compressed store's state while Game Mode's full tier holds its frames
/// on the card ([`ZPool::detach`]): its list heads, counts and frames.
#[derive(Clone, PartialEq, Eq)]
pub struct Parked {
    /// Per class: first frame (+1) with a free slot.
    pub partial: [u16; CLASSES + 1],
    /// Frames, objects and bytes held.
    pub counts: [u32; 3],
    /// The frames themselves.
    pub set: FrameSet,
}

impl Default for Parked {
    fn default() -> Self {
        Self::new()
    }
}

impl Parked {
    /// Nothing parked.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            partial: [0; CLASSES + 1],
            counts: [0; 3],
            set: FrameSet::new(),
        }
    }
}

/// Most frames that may come back somewhere other than where they left
/// ([`ZPool::attach`]): the kernel gives back every frame the game held
/// before the store comes back, so normally none move; a handful that a
/// long-lived kernel allocation took during the game are moved, more is
/// refused before anything changes.
pub const MAX_MOVES: usize = 64;

/// Frames that came back somewhere else: old frame to new.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub struct Moves {
    n: usize,
    pairs: [(u16, u16); MAX_MOVES],
}

impl Default for Moves {
    fn default() -> Self {
        Self::new()
    }
}

impl Moves {
    /// No moves.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            n: 0,
            pairs: [(0, 0); MAX_MOVES],
        }
    }

    /// Records that frame `old` now lives at `new`; false if full.
    pub fn push(&mut self, old: usize, new: usize) -> bool {
        if self.n == MAX_MOVES {
            return false;
        }
        self.pairs[self.n] = (old as u16, new as u16);
        self.n += 1;
        true
    }

    /// Where frame `old` went, if it moved.
    #[must_use]
    pub fn get(&self, old: usize) -> Option<usize> {
        self.pairs[..self.n]
            .iter()
            .find(|p| usize::from(p.0) == old)
            .map(|p| usize::from(p.1))
    }

    /// Where frame `f` is now: moved or not.
    #[must_use]
    pub fn map(&self, f: usize) -> usize {
        self.get(f).unwrap_or(f)
    }

    /// How many moved.
    #[must_use]
    pub fn len(&self) -> usize {
        self.n
    }

    /// Whether none moved.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// Maps a compressed handle's raw value through the moves (`None` if
    /// its frame didn't move, or it isn't a compressed copy's handle).
    #[must_use]
    pub fn rehome(&self, raw: u32) -> Option<u32> {
        let h = Handle::from_raw(raw);
        if h.class() == 0 || h.class() > CLASSES {
            return None;
        }
        let new = self.get(h.frame())?;
        Some(Handle::new(new, h.class(), h.slot()).raw())
    }
}

/// Why an object wasn't stored.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub enum ZError {
    /// Larger than [`MAX_OBJECT`]: keep the page uncompressed instead.
    TooBig,
    /// No frame available.
    NoMemory,
}

/// Why [`ZPool::attach`] refused (nothing changed).
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub enum AttachError {
    /// The store holds objects: the game's pages must go first.
    Busy,
    /// A list in the parked frames doesn't hold together: a link to a
    /// frame not parked, a frame of another class, or more links than
    /// frames (a loop).
    Broken,
}

/// Slot size for `class` (1-based).
const fn slot_size(class: usize) -> usize {
    class * GRANULE
}

/// Slots per frame for `class` (at most 63: the bitmap is 64 bits and the
/// header takes the first slot's room for class 1).
const fn slots(class: usize) -> usize {
    let n = (PAGE - HEADER) / slot_size(class);
    if n > 63 { 63 } else { n }
}

struct Header<'a>(&'a mut [u8; PAGE]);

impl Header<'_> {
    fn class(&self) -> usize {
        usize::from(self.0[0])
    }
    fn used(&self) -> usize {
        usize::from(self.0[1])
    }
    fn link(&self, at: usize) -> u16 {
        u16::from_le_bytes([self.0[at], self.0[at + 1]])
    }
    fn set_link(&mut self, at: usize, v: u16) {
        self.0[at..at + 2].copy_from_slice(&v.to_le_bytes());
    }
    fn next(&self) -> u16 {
        self.link(2)
    }
    fn prev(&self) -> u16 {
        self.link(4)
    }
    fn bitmap(&self) -> u64 {
        u64::from_le_bytes(self.0[8..16].try_into().unwrap_or([0; 8]))
    }
    fn set_bitmap(&mut self, b: u64) {
        self.0[8..16].copy_from_slice(&b.to_le_bytes());
        self.0[1] = b.count_ones() as u8;
    }
}

/// The compressed page store.
pub struct ZPool {
    /// Per class: first frame (+1) with a free slot.
    partial: [u16; CLASSES + 1],
    frames: usize,
    objects: usize,
    bytes: usize,
    /// The frames held.
    owned: FrameSet,
}

impl Default for ZPool {
    fn default() -> Self {
        Self::new()
    }
}

impl ZPool {
    /// An empty store.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            partial: [0; CLASSES + 1],
            frames: 0,
            objects: 0,
            bytes: 0,
            owned: FrameSet::new(),
        }
    }

    /// The frames the store holds.
    #[must_use]
    pub fn frame_set(&self) -> &FrameSet {
        &self.owned
    }

    /// Moves everything stored into `into` and leaves the store empty, its
    /// frames still allocated and untouched: Game Mode's full tier has
    /// written them to the card and frees them next, and a game's own
    /// compressed pages start a fresh store meanwhile.
    pub fn detach(&mut self, into: &mut Parked) {
        into.partial = self.partial;
        into.counts = [self.frames as u32, self.objects as u32, self.bytes as u32];
        into.set.clone_from(&self.owned);
        self.partial = [0; CLASSES + 1];
        self.frames = 0;
        self.objects = 0;
        self.bytes = 0;
        self.owned.clear();
    }

    /// Takes a parked store back once its frames hold their bytes again
    /// (read back from the card), at the frames they left except those in
    /// `moves`: list heads and links are pointed at the new frames (the
    /// page tables' handles are the VM's to rehome). Refused while the store
    /// holds anything (the game's pages go first), and if a list in the
    /// frames doesn't hold together: every link is checked before any is
    /// changed, so frames read back wrong can't send the kernel round a
    /// loop or into a frame that isn't the store's.
    pub fn attach(
        &mut self,
        mem: &mut impl Pages,
        from: &Parked,
        moves: &Moves,
    ) -> Result<(), AttachError> {
        if self.objects != 0 || self.frames != 0 {
            return Err(AttachError::Busy);
        }
        let mut steps = 0u32;
        for class in 1..=CLASSES {
            let mut old = from.partial[class];
            while old != 0 {
                let f = usize::from(old) - 1;
                steps += 1;
                if !from.set.contains(f) || steps > from.counts[0] {
                    return Err(AttachError::Broken);
                }
                let h = Header(mem.page(moves.map(f)));
                if h.class() != class {
                    return Err(AttachError::Broken);
                }
                old = h.next();
            }
        }
        let link = |v: u16| -> u16 {
            if v == 0 {
                0
            } else {
                moves.map(usize::from(v) - 1) as u16 + 1
            }
        };
        for class in 1..=CLASSES {
            let head = link(from.partial[class]);
            self.partial[class] = head;
            let mut f = head;
            while f != 0 {
                let mut h = Header(mem.page(usize::from(f) - 1));
                let (next, prev) = (link(h.next()), link(h.prev()));
                h.set_link(2, next);
                h.set_link(4, prev);
                f = next;
            }
        }
        self.frames = from.counts[0] as usize;
        self.objects = from.counts[1] as usize;
        self.bytes = from.counts[2] as usize;
        for f in from.set.iter() {
            self.owned.insert(moves.map(f));
        }
        Ok(())
    }

    /// Frames in use, objects stored, and their total bytes.
    #[must_use]
    pub fn stats(&self) -> (usize, usize, usize) {
        (self.frames, self.objects, self.bytes)
    }

    /// Stores `data` (a compressed page), returning its handle.
    pub fn store(
        &mut self,
        frames: &mut impl FrameAlloc,
        mem: &mut impl Pages,
        data: &[u8],
    ) -> Result<Handle, ZError> {
        if data.len() > MAX_OBJECT {
            return Err(ZError::TooBig);
        }
        let class = (data.len() + 2).div_ceil(GRANULE).max(1);
        let f = match self.partial[class] {
            0 => {
                let f = frames.alloc(0).ok_or(ZError::NoMemory)?;
                debug_assert!(f < usize::from(u16::MAX), "frame number fits the links");
                let h = Header(mem.page(f));
                h.0[..HEADER].fill(0);
                h.0[0] = class as u8;
                self.link_partial(mem, f, class);
                self.frames += 1;
                self.owned.insert(f);
                f
            }
            head => usize::from(head) - 1,
        };
        let mut h = Header(mem.page(f));
        let bm = h.bitmap();
        let slot = (!bm).trailing_zeros() as usize;
        debug_assert!(slot < slots(class), "partial frame with no free slot");
        h.set_bitmap(bm | 1 << slot);
        let full = h.used() == slots(class);
        let at = HEADER + slot * slot_size(class);
        h.0[at..at + 2].copy_from_slice(&(data.len() as u16).to_le_bytes());
        h.0[at + 2..at + 2 + data.len()].copy_from_slice(data);
        if full {
            self.unlink_partial(mem, f, class);
        }
        self.objects += 1;
        self.bytes += data.len();
        Ok(Handle::new(f, class, slot))
    }

    /// The bytes stored under `h` (a view into the frame: decompress straight
    /// from it).
    pub fn get<'a>(&self, mem: &'a mut impl Pages, h: Handle) -> &'a [u8] {
        let at = HEADER + h.slot() * slot_size(h.class());
        let page = mem.page(h.frame());
        let len = usize::from(u16::from_le_bytes([page[at], page[at + 1]]));
        &page[at + 2..at + 2 + len.min(slot_size(h.class()) - 2)]
    }

    /// Frees the object under `h`; a frame whose last object goes is freed.
    pub fn free(&mut self, frames: &mut impl FrameAlloc, mem: &mut impl Pages, h: Handle) {
        let (f, class, slot) = (h.frame(), h.class(), h.slot());
        let at = HEADER + slot * slot_size(class);
        let mut hd = Header(mem.page(f));
        debug_assert_eq!(hd.class(), class, "handle from another frame");
        let bm = hd.bitmap();
        debug_assert!(bm & 1 << slot != 0, "double free of a compressed page");
        let was_full = hd.used() == slots(class);
        let len = usize::from(u16::from_le_bytes([hd.0[at], hd.0[at + 1]]));
        hd.set_bitmap(bm & !(1 << slot));
        let empty = hd.used() == 0;
        self.objects -= 1;
        self.bytes -= len;
        if was_full {
            self.link_partial(mem, f, class);
        }
        if empty {
            self.unlink_partial(mem, f, class);
            frames.free(f);
            self.frames -= 1;
            self.owned.remove(f);
        }
    }

    fn link_partial(&mut self, mem: &mut impl Pages, f: usize, class: usize) {
        let head = self.partial[class];
        {
            let mut h = Header(mem.page(f));
            h.set_link(2, head);
            h.set_link(4, 0);
        }
        if head != 0 {
            Header(mem.page(usize::from(head) - 1)).set_link(4, f as u16 + 1);
        }
        self.partial[class] = f as u16 + 1;
    }

    fn unlink_partial(&mut self, mem: &mut impl Pages, f: usize, class: usize) {
        let (next, prev) = {
            let h = Header(mem.page(f));
            (h.next(), h.prev())
        };
        if prev == 0 {
            self.partial[class] = next;
        } else {
            Header(mem.page(usize::from(prev) - 1)).set_link(2, next);
        }
        if next != 0 {
            Header(mem.page(usize::from(next) - 1)).set_link(4, prev);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frames::Frames;
    use std::collections::HashMap;

    #[derive(Default)]
    struct Mem(HashMap<usize, Box<[u8; PAGE]>>);

    impl Pages for Mem {
        fn page(&mut self, f: usize) -> &mut [u8; PAGE] {
            self.0.entry(f).or_insert_with(|| Box::new([0xEE; PAGE]))
        }
    }

    fn setup() -> (ZPool, Frames<256>, Mem) {
        let mut fr = Frames::<256>::new();
        fr.add_range(0, 256).unwrap();
        (ZPool::new(), fr, Mem::default())
    }

    #[test]
    fn the_store_keeps_its_frames_in_a_set_lowest_first() {
        let (mut z, mut fr, mut mem) = setup();
        assert!(z.frame_set().is_empty());
        let a = z.store(&mut fr, &mut mem, &[7; 100]).unwrap();
        let b = z.store(&mut fr, &mut mem, &[8; 3000]).unwrap();
        let c = z.store(&mut fr, &mut mem, &[9; 3000]).unwrap();
        let mut want = vec![a.frame(), b.frame(), c.frame()];
        want.sort_unstable();
        assert_eq!(z.frame_set().iter().collect::<Vec<_>>(), want);
        assert_eq!(z.frame_set().len(), 3);
        z.free(&mut fr, &mut mem, b);
        assert!(!z.frame_set().contains(b.frame()));
        assert_eq!(z.frame_set().len(), 2);
        // Set edges: bit 31 and 32 of neighbouring words, the last frame.
        let mut s = FrameSet::new();
        for f in [0, 31, 32, MAX_FRAMES - 1] {
            s.insert(f);
        }
        assert_eq!(s.iter().collect::<Vec<_>>(), [0, 31, 32, MAX_FRAMES - 1]);
        assert!(!s.contains(MAX_FRAMES) && !s.contains(1));
        s.remove(31);
        assert_eq!(s.len(), 3);
        s.clear();
        assert!(s.is_empty());
    }

    #[test]
    fn detach_empties_the_store_and_moves_map_handles() {
        let (mut z, mut fr, mut mem) = setup();
        let h = z.store(&mut fr, &mut mem, &[5; 200]).unwrap();
        let mut p = Parked::new();
        z.detach(&mut p);
        assert_eq!(z.stats(), (0, 0, 0));
        assert_eq!(p.counts, [1, 1, 200]);
        assert!(p.set.contains(h.frame()));
        assert_eq!(p.partial[h.class()], h.frame() as u16 + 1);
        // A fresh store meanwhile starts from nothing.
        let g = z.store(&mut fr, &mut mem, &[6; 200]).unwrap();
        assert_ne!(g.frame(), h.frame());
        let mut m = Moves::new();
        assert_eq!(m.rehome(h.raw()), None);
        assert!(m.push(h.frame(), 99));
        let r = Handle::from_raw(m.rehome(h.raw()).unwrap());
        assert_eq!((r.frame(), r.class(), r.slot()), (99, h.class(), h.slot()));
        assert_eq!((m.map(h.frame()), m.map(3), m.len()), (99, 3, 1));
        // Class 0 and the aged class aren't handles.
        assert_eq!(m.rehome(Handle::new(h.frame(), 0, 1).raw()), None);
        assert_eq!(m.rehome(Handle::new(h.frame(), 0xFF, 0).raw()), None);
        for i in 1..MAX_MOVES {
            assert!(m.push(i + 1000, i));
        }
        assert!(!m.push(5000, 1), "full");
    }

    #[test]
    fn a_parked_list_that_does_not_hold_together_is_refused() {
        let (mut z, mut fr, mut mem) = setup();
        // Two partial frames of one class (two slots a frame): a list of
        // two.
        let x = z.store(&mut fr, &mut mem, &[1; 1400]).unwrap();
        let a = z.store(&mut fr, &mut mem, &[2; 1400]).unwrap();
        let b = z.store(&mut fr, &mut mem, &[3; 1400]).unwrap();
        assert_eq!(slots(a.class()), 2);
        assert_ne!(a.frame(), b.frame());
        z.free(&mut fr, &mut mem, x);
        let mut p = Parked::new();
        z.detach(&mut p);
        let none = Moves::new();
        let (fa, fb) = (a.frame(), b.frame());
        let class = a.class();
        let head = usize::from(p.partial[class]) - 1;
        let saved: [u8; 16] = mem.page(head)[..16].try_into().unwrap();
        // A loop (the head's next pointing at itself), a link off the
        // parked frames, a frame of another class: each refused, nothing
        // taken.
        for bad in 0..3 {
            mem.page(head)[..16].copy_from_slice(&saved);
            let mut h = Header(mem.page(head));
            match bad {
                0 => h.set_link(2, head as u16 + 1),
                // Frame 200 looks like one of the list (the head's header)
                // but was never parked.
                1 => {
                    h.set_link(2, 201);
                    mem.page(200)[..16].copy_from_slice(&saved);
                    Header(mem.page(200)).set_link(2, 0);
                }
                _ => h.0[0] = class as u8 + 1,
            }
            assert_eq!(
                z.attach(&mut mem, &p, &none),
                Err(AttachError::Broken),
                "case {bad}"
            );
            assert_eq!(z.stats(), (0, 0, 0));
            assert!(z.frame_set().is_empty());
        }
        mem.page(head)[..16].copy_from_slice(&saved);
        z.attach(&mut mem, &p, &none).unwrap();
        assert!(z.frame_set().contains(fa) && z.frame_set().contains(fb));
    }

    #[test]
    fn class_layout_facts() {
        assert_eq!(CLASSES, 49);
        assert_eq!(slots(1), 63);
        assert_eq!(slots(2), 31);
        assert_eq!(slots(CLASSES), 1);
        for c in 1..=CLASSES {
            assert!(
                HEADER + slots(c) * slot_size(c) <= PAGE,
                "class {c} fits a frame"
            );
        }
    }

    #[test]
    fn stores_and_reads_back() {
        let (mut z, mut fr, mut mem) = setup();
        let a = z.store(&mut fr, &mut mem, b"tiny").unwrap();
        let big = vec![7u8; 3000];
        let b = z.store(&mut fr, &mut mem, &big).unwrap();
        assert_eq!(z.get(&mut mem, a), b"tiny");
        assert_eq!(z.get(&mut mem, b), &big[..]);
        assert_eq!(z.stats(), (2, 2, 3004), "two classes, two frames");
        assert_eq!(
            z.store(&mut fr, &mut mem, &[0; MAX_OBJECT + 1]),
            Err(ZError::TooBig)
        );
    }

    #[test]
    fn small_objects_share_a_frame_and_frames_return() {
        let (mut z, mut fr, mut mem) = setup();
        let hs: Vec<_> = (0..63u8)
            .map(|i| z.store(&mut fr, &mut mem, &[i; 40]).unwrap())
            .collect();
        assert_eq!(z.stats().0, 1, "63 objects of 40 bytes fit one frame");
        let extra = z.store(&mut fr, &mut mem, &[99; 40]).unwrap();
        assert_eq!(z.stats().0, 2);
        for (i, h) in hs.iter().enumerate() {
            assert_eq!(z.get(&mut mem, *h), &[i as u8; 40]);
        }
        for h in hs {
            z.free(&mut fr, &mut mem, h);
        }
        z.free(&mut fr, &mut mem, extra);
        assert_eq!(z.stats(), (0, 0, 0));
        assert_eq!(fr.free_frames(), 256, "every frame back");
    }

    #[test]
    fn a_full_frame_rejoins_the_free_list_when_a_slot_frees() {
        let (mut z, mut fr, mut mem) = setup();
        // Class 48 (3000 bytes) holds one per frame: each store is full.
        let a = z.store(&mut fr, &mut mem, &[1; 3000]).unwrap();
        let b = z.store(&mut fr, &mut mem, &[2; 3000]).unwrap();
        assert_ne!(a.frame(), b.frame());
        // Class 2 (100 bytes): fill a frame, free one, the next store reuses it.
        let hs: Vec<_> = (0..31)
            .map(|_| z.store(&mut fr, &mut mem, &[3; 100]).unwrap())
            .collect();
        let f = hs[0].frame();
        assert!(hs.iter().all(|h| h.frame() == f));
        z.free(&mut fr, &mut mem, hs[10]);
        let again = z.store(&mut fr, &mut mem, &[4; 100]).unwrap();
        assert_eq!((again.frame(), again.slot()), (f, hs[10].slot()));
    }

    #[test]
    fn random_workload_against_a_model() {
        let (mut z, mut fr, mut mem) = setup();
        let mut rng = 0x1234_5678_9ABC_DEF1u64;
        let mut next = move || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        let mut live: Vec<(Handle, Vec<u8>)> = Vec::new();
        for step in 0..20_000 {
            if live.is_empty() || next() % 5 < 3 {
                let len = (next() % MAX_OBJECT as u64) as usize;
                let data: Vec<u8> = (0..len).map(|_| next() as u8).collect();
                match z.store(&mut fr, &mut mem, &data) {
                    Ok(h) => live.push((h, data)),
                    Err(ZError::NoMemory) => {}
                    Err(e) => panic!("step {step}: {e:?}"),
                }
            } else {
                let (h, data) = live.swap_remove(next() as usize % live.len());
                assert_eq!(z.get(&mut mem, h), &data[..], "step {step}");
                z.free(&mut fr, &mut mem, h);
            }
        }
        for (h, data) in live.drain(..) {
            assert_eq!(z.get(&mut mem, h), &data[..]);
            z.free(&mut fr, &mut mem, h);
        }
        assert_eq!(z.stats(), (0, 0, 0));
        assert_eq!(fr.free_frames(), 256);
    }

    #[test]
    fn handles_round_trip_through_raw() {
        let h = Handle::new(1234, 17, 5);
        assert_eq!(Handle::from_raw(h.raw()), h);
        assert_eq!((h.frame(), h.class(), h.slot()), (1234, 17, 5));
    }

    /// The highest frame the allocator can hand out, in the highest class
    /// and slot, survives being stored in a page table entry and read back.
    #[test]
    fn the_last_frame_round_trips_through_a_stored_entry() {
        use crate::arm::{L2, l2_decode, l2_stored};
        let h = Handle::new(MAX_FRAMES - 1, CLASSES, 63);
        let L2::Stored(raw) = l2_decode(l2_stored(h.raw())) else {
            panic!("not a stored entry");
        };
        let back = Handle::from_raw(raw);
        assert_eq!(back, h);
        assert_eq!(
            (back.frame(), back.class(), back.slot()),
            (MAX_FRAMES - 1, CLASSES, 63)
        );
    }

    #[test]
    #[should_panic(expected = "stored payload out of range")]
    fn a_frame_past_the_limit_cannot_be_stored() {
        // Debug builds catch it at the handle; this is the twin check at
        // the descriptor, for the bits the handle would have lost.
        let _ = crate::arm::l2_stored(((MAX_FRAMES as u32) << FRAME_SHIFT) | 1 << 8);
    }
}

#[cfg(test)]
mod effectiveness {
    //! The store together with the LZ4 codec on a realistic page mix: what
    //! compressed RAM actually saves.
    use super::*;
    use crate::frames::Frames;
    use std::collections::HashMap;

    #[derive(Default)]
    struct Mem(HashMap<usize, Box<[u8; PAGE]>>);

    impl Pages for Mem {
        fn page(&mut self, f: usize) -> &mut [u8; PAGE] {
            self.0.entry(f).or_insert_with(|| Box::new([0; PAGE]))
        }
    }

    #[test]
    fn a_realistic_mix_fits_in_well_under_half() {
        let mut fr = Frames::<1024>::new();
        fr.add_range(0, 1024).unwrap();
        let (mut z, mut mem) = (ZPool::new(), Mem::default());
        let mut x = 0x2468_ACE1u32;
        let mut rnd = move || {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x
        };
        let (mut pages, mut kept_raw) = (0, 0);
        let mut buf = vec![0u8; rp_lz4::max_compressed(PAGE)];
        for i in 0..200 {
            let page: Vec<u8> = match i % 4 {
                // A mostly-zero page (fresh heap, sparse arrays).
                0 => (0..PAGE)
                    .map(|k| if k % 512 == 0 { rnd() as u8 } else { 0 })
                    .collect(),
                // Heap: pointers into a few objects and small integers.
                1 => (0..1024u32)
                    .flat_map(|k| {
                        (if k % 4 == 0 {
                            0xA800_0000 + (rnd() % 64) * 32
                        } else {
                            k % 7
                        })
                        .to_le_bytes()
                    })
                    .collect(),
                // Text and resources.
                2 => b"Settings Today Calendar Contacts Messaging Wi-Fi Programs "
                    .iter()
                    .cycle()
                    .skip(i)
                    .take(PAGE)
                    .copied()
                    .collect(),
                // Media (already compressed).
                _ => (0..PAGE).map(|_| rnd() as u8).collect(),
            };
            pages += 1;
            let n = rp_lz4::compress(&page, &mut buf).unwrap_or_else(|e| panic!("{}", e.name()));
            match z.store(&mut fr, &mut mem, &buf[..n]) {
                Ok(h) => {
                    let mut back = [0u8; PAGE];
                    assert!(rp_lz4::decompress(z.get(&mut mem, h), &mut back) == Ok(PAGE));
                    assert_eq!(&back[..], &page[..]);
                }
                Err(ZError::TooBig) => kept_raw += 1, // media stays as is
                Err(e) => panic!("{e:?}"),
            }
        }
        let (frames, objects, bytes) = z.stats();
        let used = frames + kept_raw;
        eprintln!(
            "{pages} pages -> {frames} frames for {objects} compressed ({bytes} bytes) + {kept_raw} kept raw = {used} ({}%)",
            used * 100 / pages
        );
        assert_eq!(kept_raw, 50, "every media page refused, nothing else");
        assert!(
            used * 100 / pages < 40,
            "compressed RAM should more than halve this mix"
        );
    }
}
