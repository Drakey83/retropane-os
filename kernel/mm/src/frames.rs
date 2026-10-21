//! Physical frame allocator: a binary buddy allocator over up to
//! [`MAX_FRAMES`](crate::zpool::MAX_FRAMES) frames (64 MiB).
//!
//! Hands out naturally aligned blocks of 2^order frames, 4 KiB to 4 MiB (order
//! 0 to [`MAX_ORDER`]). The kernel needs contiguous blocks for page tables and
//! DMA buffers; everything else takes order 0. Allocating and freeing are
//! O([`MAX_ORDER`]), and freeing merges buddies straight away, so free memory
//! stays in the largest blocks it can.
//!
//! Designed for 64 MB (ADR-0005):
//!
//! - **Bookkeeping lives beside the frames, not in them.** Free lists are
//!   linked through two `u16` side arrays, so allocating or freeing never
//!   touches the free memory itself (no cache misses on cold pages, no mapping
//!   needed). Cost: 5 bytes per frame, 80 KiB for the X51v's 64 MiB.
//! - **All zeros means empty.** A fresh allocator is plain zeroed memory, so a
//!   `static` costs nothing in the image (it lands in `.bss`), and frames join
//!   only when [`Frames::add_range`] hands them over: holes in physical memory
//!   (the kernel image, reserved regions) are simply never added.
//! - **One type for every pool:** system RAM and the 2700G's local memory
//!   (rule 5) are separate instances.
//!
//! Frames are indices relative to the pool's start; the caller turns them into
//! physical addresses.

use crate::zpool::MAX_FRAMES;

/// Largest block order: 2^10 frames = 4 MiB.
pub const MAX_ORDER: usize = 10;
const ORDERS: usize = MAX_ORDER + 1;

/// Tag bits: the frame heads a free block.
const FREE: u8 = 0x80;
/// Tag bits: the frame heads an allocated block.
const USED: u8 = 0x40;
const ORDER_MASK: u8 = 0x0F;

/// Bytes in a block of `order`.
#[must_use]
pub const fn block_bytes(order: usize) -> usize {
    crate::PAGE_SIZE << order
}

/// The smallest order whose blocks hold `bytes`, if one exists.
#[must_use]
pub const fn order_for(bytes: usize) -> Option<usize> {
    let mut order = 0;
    while order <= MAX_ORDER {
        if block_bytes(order) >= bytes {
            return Some(order);
        }
        order += 1;
    }
    None
}

/// Why a free or a range addition was refused.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub enum Error {
    /// The frame doesn't head an allocated block (a double free, an interior
    /// frame, or a frame the allocator never owned).
    NotAllocated,
    /// The range reaches past the allocator's capacity.
    OutOfRange,
    /// Part of the range already belongs to the allocator.
    Overlap,
}

impl Error {
    /// A short name for logs (no `Debug` in release images, ADR-0005 rule 8).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Error::NotAllocated => "not allocated",
            Error::OutOfRange => "out of range",
            Error::Overlap => "overlap",
        }
    }
}

/// A buddy allocator over `N` frames (`N` <= [`MAX_FRAMES`]: a stored page
/// table entry names a frame in 14 bits).
pub struct Frames<const N: usize> {
    /// Free-list links, stored as index + 1 so that 0 means "none".
    next: [u16; N],
    prev: [u16; N],
    /// Per frame: 0 (not a block head), or `FREE`/`USED` | order.
    tag: [u8; N],
    /// Head of each order's free list (index + 1, 0 = empty).
    heads: [u16; ORDERS],
    free: usize,
}

impl<const N: usize> Default for Frames<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Frames<N> {
    /// An allocator that owns no frames yet.
    #[must_use]
    pub const fn new() -> Self {
        // Frame numbers go into stored page table entries (compressed
        // handles, aged pages) with 14 bits to spare: a bigger allocator
        // would hand out frames those entries can't name. Checked at
        // compile time for the kernel's static allocator.
        assert!(N <= MAX_FRAMES, "frame numbers must fit a stored entry");
        Self {
            next: [0; N],
            prev: [0; N],
            tag: [0; N],
            heads: [0; ORDERS],
            free: 0,
        }
    }

    /// Hands frames `start..start + count` to the allocator, as the largest
    /// aligned blocks that fit. Each frame can be added once.
    pub fn add_range(&mut self, start: usize, count: usize) -> Result<(), Error> {
        let end = start.checked_add(count).ok_or(Error::OutOfRange)?;
        if end > N {
            return Err(Error::OutOfRange);
        }
        if (start..end).any(|f| self.owns(f)) {
            return Err(Error::Overlap);
        }
        let mut f = start;
        while f < end {
            let mut order = MAX_ORDER;
            while !f.is_multiple_of(1 << order) || f + (1 << order) > end {
                order -= 1;
            }
            self.release(f, order);
            f += 1 << order;
        }
        Ok(())
    }

    /// Whether frame `f` is part of any block, free or allocated.
    fn owns(&self, f: usize) -> bool {
        (0..=MAX_ORDER).any(|order| {
            let head = f & !((1 << order) - 1);
            let t = self.tag[head];
            t != 0 && usize::from(t & ORDER_MASK) >= order
        })
    }

    /// Allocates a block of 2^`order` frames, returning its first frame
    /// (aligned to 2^`order`).
    pub fn alloc(&mut self, order: usize) -> Option<usize> {
        if order > MAX_ORDER {
            return None;
        }
        let mut have = (order..=MAX_ORDER).find(|&o| self.heads[o] != 0)?;
        let f = usize::from(self.heads[have]) - 1;
        self.unlink(f, have);
        // Split, giving back the upper halves.
        while have > order {
            have -= 1;
            self.link(f + (1 << have), have);
        }
        self.tag[f] = USED | order as u8;
        self.free -= 1 << order;
        Some(f)
    }

    /// Frees the block that starts at frame `f`. The order comes from the
    /// allocation, so a block can't be freed with the wrong size.
    pub fn free(&mut self, f: usize) -> Result<(), Error> {
        let t = *self.tag.get(f).ok_or(Error::NotAllocated)?;
        if t & USED == 0 {
            return Err(Error::NotAllocated);
        }
        self.release(f, usize::from(t & ORDER_MASK));
        Ok(())
    }

    /// Returns block `f` of `order` to the free lists, merging buddies.
    fn release(&mut self, mut f: usize, mut order: usize) {
        self.free += 1 << order;
        while order < MAX_ORDER {
            let buddy = f ^ (1 << order);
            if buddy >= N || self.tag[buddy] != FREE | order as u8 {
                break;
            }
            self.unlink(buddy, order);
            self.tag[f] = 0;
            f = f.min(buddy);
            order += 1;
        }
        self.link(f, order);
    }

    fn link(&mut self, f: usize, order: usize) {
        let head = self.heads[order];
        self.next[f] = head;
        self.prev[f] = 0;
        if head != 0 {
            self.prev[usize::from(head) - 1] = f as u16 + 1;
        }
        self.heads[order] = f as u16 + 1;
        self.tag[f] = FREE | order as u8;
    }

    fn unlink(&mut self, f: usize, order: usize) {
        let (n, p) = (self.next[f], self.prev[f]);
        if p == 0 {
            self.heads[order] = n;
        } else {
            self.next[usize::from(p) - 1] = n;
        }
        if n != 0 {
            self.prev[usize::from(n) - 1] = p;
        }
        self.tag[f] = 0;
    }

    /// Free frames in total.
    #[must_use]
    pub fn free_frames(&self) -> usize {
        self.free
    }

    /// The largest order with a free block: the biggest contiguous allocation
    /// that can succeed right now.
    #[must_use]
    pub fn largest_free_order(&self) -> Option<usize> {
        (0..=MAX_ORDER).rev().find(|&o| self.heads[o] != 0)
    }

    /// Free blocks of each order (walks the lists; for metrics and tests).
    #[must_use]
    pub fn free_blocks(&self) -> [usize; ORDERS] {
        let mut out = [0; ORDERS];
        for (order, n) in out.iter_mut().enumerate() {
            let mut link = self.heads[order];
            while link != 0 {
                *n += 1;
                link = self.next[usize::from(link) - 1];
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Checks every structural invariant: list links agree both ways, tags
    /// match list membership, blocks are aligned, in range and don't overlap,
    /// no two free buddies are left unmerged, and the free count adds up.
    fn check<const N: usize>(a: &Frames<N>) {
        let mut seen = vec![false; N];
        let mut total = 0;
        for order in 0..=MAX_ORDER {
            let mut prev = 0u16;
            let mut link = a.heads[order];
            while link != 0 {
                let f = usize::from(link) - 1;
                assert_eq!(a.prev[f], prev, "back link of {f}");
                assert_eq!(a.tag[f], FREE | order as u8, "tag of {f}");
                assert!(f.is_multiple_of(1 << order), "alignment of {f}");
                assert!(f + (1 << order) <= N);
                for s in &mut seen[f..f + (1 << order)] {
                    assert!(!*s, "overlap at {f}");
                    *s = true;
                }
                if order < MAX_ORDER {
                    let buddy = f ^ (1 << order);
                    assert!(
                        buddy >= N || a.tag[buddy] != FREE | order as u8,
                        "unmerged buddies {f} and {buddy}"
                    );
                }
                total += 1 << order;
                prev = link;
                link = a.next[f];
            }
        }
        assert_eq!(total, a.free, "free count");
    }

    #[test]
    fn empty_until_frames_are_added() {
        let mut a = Frames::<64>::new();
        assert_eq!(a.alloc(0), None);
        assert_eq!(a.free_frames(), 0);
        check(&a);
    }

    #[test]
    fn add_range_builds_the_largest_aligned_blocks() {
        let mut a = Frames::<4096>::new();
        a.add_range(3, 2048 + 5).unwrap(); // 3..2056
        check(&a);
        assert_eq!(a.free_frames(), 2053);
        // 3 | 4..8 | 8..16 | ... | 1024..2048 | 2048..2056
        let blocks = a.free_blocks();
        assert_eq!(blocks[0], 1);
        assert_eq!(blocks[2], 1);
        assert_eq!(blocks[3], 2); // 8..16 and 2048..2056
        assert_eq!(blocks[10], 1);
        assert_eq!(a.largest_free_order(), Some(10));
    }

    #[test]
    fn add_range_rejects_overlap_and_overflow() {
        let mut a = Frames::<64>::new();
        a.add_range(8, 8).unwrap();
        assert!(a.add_range(15, 2) == Err(Error::Overlap));
        assert!(a.add_range(0, 9) == Err(Error::Overlap));
        assert!(a.add_range(60, 5) == Err(Error::OutOfRange));
        assert!(a.add_range(usize::MAX, 2) == Err(Error::OutOfRange));
        let f = a.alloc(0).unwrap();
        assert!(
            a.add_range(f, 1) == Err(Error::Overlap),
            "allocated frames too"
        );
        a.add_range(16, 48).unwrap();
        check(&a);
    }

    #[test]
    fn blocks_are_aligned_and_split_from_the_smallest_fit() {
        let mut a = Frames::<1024>::new();
        a.add_range(0, 1024).unwrap();
        let x = a.alloc(0).unwrap();
        assert_eq!(x, 0);
        // The 4 MiB block split into one free block of every order 0..9.
        assert_eq!(a.free_blocks(), [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0]);
        let y = a.alloc(2).unwrap();
        assert!(y.is_multiple_of(4));
        assert_eq!(y, 4, "uses the existing order-2 block, no new split");
        check(&a);
    }

    #[test]
    fn freeing_merges_back_to_one_block() {
        let mut a = Frames::<1024>::new();
        a.add_range(0, 1024).unwrap();
        let got: Vec<_> = (0..1024).map(|_| a.alloc(0).unwrap()).collect();
        assert_eq!(a.alloc(0), None);
        assert_eq!(a.free_frames(), 0);
        for f in got.iter().rev().step_by(2).chain(got.iter().step_by(2)) {
            a.free(*f).unwrap();
        }
        check(&a);
        assert_eq!(a.free_blocks()[MAX_ORDER], 1);
        assert_eq!(a.free_frames(), 1024);
    }

    #[test]
    fn rejects_double_and_interior_frees() {
        let mut a = Frames::<64>::new();
        a.add_range(0, 64).unwrap();
        let f = a.alloc(3).unwrap();
        assert!(a.free(f + 1) == Err(Error::NotAllocated));
        a.free(f).unwrap();
        assert!(a.free(f) == Err(Error::NotAllocated));
        assert!(a.free(64) == Err(Error::NotAllocated));
        assert!(a.free(usize::MAX) == Err(Error::NotAllocated));
        check(&a);
    }

    #[test]
    fn never_merges_into_frames_it_does_not_own() {
        // Frames 0..4 are a hole (say, the kernel image).
        let mut a = Frames::<16>::new();
        a.add_range(4, 12).unwrap();
        let f = a.alloc(2).unwrap();
        a.free(f).unwrap();
        check(&a);
        assert_eq!(a.largest_free_order(), Some(3)); // 8..16, not 0..16
        assert_eq!(a.free_frames(), 12);
    }

    #[test]
    fn order_helpers() {
        assert_eq!(order_for(0), Some(0));
        assert_eq!(order_for(4096), Some(0));
        assert_eq!(order_for(4097), Some(1));
        assert_eq!(order_for(16 * 1024), Some(2)); // an L1 page table
        assert_eq!(order_for(4 << 20), Some(MAX_ORDER));
        assert_eq!(order_for((4 << 20) + 1), None);
        assert_eq!(block_bytes(MAX_ORDER), 4 << 20);
    }

    #[test]
    #[should_panic(expected = "frame numbers must fit a stored entry")]
    fn more_frames_than_a_stored_entry_names_are_refused() {
        let _ = Frames::<{ MAX_FRAMES + 1 }>::new();
    }

    #[test]
    fn sized_for_the_x51v() {
        // 64 MiB of 4 KiB frames: 5 bytes each, plus a few list heads.
        let size = core::mem::size_of::<Frames<16384>>();
        assert!(size <= 16384 * 5 + 64, "{size}");
    }

    /// Random allocs and frees of mixed sizes, checked against a shadow map
    /// of who owns each frame, then everything freed: the allocator must end
    /// exactly where it started.
    #[test]
    fn random_workload_against_a_model() {
        const N: usize = 4096;
        let mut a = Frames::<N>::new();
        a.add_range(1, N - 1).unwrap(); // odd start: exercises small blocks
        let fresh = a.free_blocks();
        let mut rng = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        let mut owner = vec![false; N];
        let mut live: Vec<(usize, usize)> = Vec::new();
        for step in 0..20_000 {
            if live.is_empty() || next() % 3 != 0 {
                // Mostly small blocks, sometimes large ones.
                let order = (next() % 64).trailing_zeros().min(MAX_ORDER as u32) as usize;
                if let Some(f) = a.alloc(order) {
                    assert!(f.is_multiple_of(1 << order));
                    for o in &mut owner[f..f + (1 << order)] {
                        assert!(!*o, "frame handed out twice (step {step})");
                        *o = true;
                    }
                    live.push((f, order));
                } else {
                    assert!(a.largest_free_order().is_none_or(|l| l < order));
                }
            } else {
                let (f, order) = live.swap_remove(next() as usize % live.len());
                a.free(f).unwrap();
                owner[f..f + (1 << order)].fill(false);
            }
            if step % 1000 == 0 {
                check(&a);
            }
        }
        for (f, _) in live.drain(..) {
            a.free(f).unwrap();
        }
        check(&a);
        assert_eq!(a.free_frames(), N - 1);
        assert_eq!(a.free_blocks(), fresh);
    }
}
