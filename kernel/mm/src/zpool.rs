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
        debug_assert!(frame < 1 << 16 && class < 1 << 8 && slot < 1 << 8);
        Self((frame as u32) << 16 | (class as u32) << 8 | slot as u32)
    }
    fn frame(self) -> usize {
        (self.0 >> 16) as usize
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

/// Why an object wasn't stored.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub enum ZError {
    /// Larger than [`MAX_OBJECT`]: keep the page uncompressed instead.
    TooBig,
    /// No frame available.
    NoMemory,
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
        }
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
