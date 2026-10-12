//! Region lists kept in per-process kernel pages (ADR-0019).
//!
//! An address space's regions (its program, stacks, heap reservations,
//! mapped files) used to live in a fixed array of 64 in every process record:
//! too few for some Windows Mobile programs (every `VirtualAlloc` reservation
//! is a region) and wasted on small services. Here they live in 4 KiB pages
//! taken from the frame allocator as the list grows and given back as it
//! shrinks, up to a per-process quota ([`MAX_PAGES`]), so memory follows real
//! use and one process can't exhaust the kernel.
//!
//! Each region is 20 bytes, encoded by hand into the page's bytes: no unsafe
//! code, and nothing about Rust's layout leaks into the format. The list is
//! sorted by start address and searched by bisection.

use crate::arm::Cache;
use crate::space::{Backing, Prot, Region, SpaceError};

/// Bytes per stored region.
pub const ENTRY_BYTES: usize = 20;
/// Regions per page.
pub const PER_PAGE: usize = 4096 / ENTRY_BYTES;
/// Pages a list may use: the per-process quota.
pub const MAX_PAGES: usize = 8;
/// Regions a list may hold.
pub const MAX_REGIONS: usize = PER_PAGE * MAX_PAGES;

/// Where region pages come from: kernel-private pages from the frame
/// allocator (never mapped for a process).
pub trait RegionPages {
    /// A free page's physical address, or `None` when memory is out.
    fn alloc_page(&mut self) -> Option<u32>;
    /// Gives a page back.
    fn free_page(&mut self, page: u32);
    /// The page's bytes.
    fn page(&mut self, page: u32) -> &mut [u8; 4096];
}

/// A sorted list of regions in pages from a [`RegionPages`]. Small and
/// `Copy`: the record holds only page addresses and a count.
#[derive(Clone, Copy)]
pub struct RegionList {
    pages: [u32; MAX_PAGES],
    count: u32,
}

impl Default for RegionList {
    fn default() -> Self {
        Self::new()
    }
}

impl RegionList {
    /// An empty list, using no pages.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            pages: [0; MAX_PAGES],
            count: 0,
        }
    }

    /// Regions in the list.
    #[must_use]
    pub fn len(&self) -> usize {
        self.count as usize
    }

    /// Whether the list is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Pages the list holds now.
    #[must_use]
    pub fn pages_used(&self) -> usize {
        self.pages.iter().take_while(|&&p| p != 0).count()
    }

    /// The page holding entry `i` (tests look at what it contains).
    #[cfg(test)]
    pub(crate) fn page_of(&self, i: usize) -> u32 {
        self.pages[i / PER_PAGE]
    }

    fn slot(i: usize) -> (usize, usize) {
        (i / PER_PAGE, (i % PER_PAGE) * ENTRY_BYTES)
    }

    /// Region `i` (in address order).
    ///
    /// # Panics
    /// If `i` is out of range.
    pub fn get(&self, rp: &mut impl RegionPages, i: usize) -> Region {
        assert!(i < self.len(), "region index out of range");
        let (p, off) = Self::slot(i);
        decode(&rp.page(self.pages[p])[off..off + ENTRY_BYTES])
    }

    fn put(&self, rp: &mut impl RegionPages, i: usize, r: &Region) {
        let (p, off) = Self::slot(i);
        encode(r, &mut rp.page(self.pages[p])[off..off + ENTRY_BYTES]);
    }

    /// Replaces region `i`.
    pub fn set(&mut self, rp: &mut impl RegionPages, i: usize, r: &Region) {
        assert!(i < self.len(), "region index out of range");
        self.put(rp, i, r);
    }

    /// Makes room for `extra` more regions now, so a change that needs them
    /// can't fail halfway. Pages are taken from `rp` as needed.
    ///
    /// # Errors
    /// [`SpaceError::TooManyRegions`] past the quota, [`SpaceError::NoMemory`]
    /// when no page can be had.
    pub fn reserve(&mut self, rp: &mut impl RegionPages, extra: usize) -> Result<(), SpaceError> {
        let need = self.len() + extra;
        if need > MAX_REGIONS {
            return Err(SpaceError::TooManyRegions);
        }
        while self.pages_used() * PER_PAGE < need {
            let page = rp.alloc_page().ok_or(SpaceError::NoMemory)?;
            let used = self.pages_used();
            self.pages[used] = page;
        }
        Ok(())
    }

    /// Inserts `r` at its place in address order.
    ///
    /// # Errors
    /// As [`reserve`](Self::reserve).
    pub fn insert(&mut self, rp: &mut impl RegionPages, r: &Region) -> Result<usize, SpaceError> {
        self.reserve(rp, 1)?;
        let at = self.first_after(rp, r.start);
        let mut i = self.len();
        while i > at {
            let moved = self.get(rp, i - 1);
            self.count += 1;
            self.put(rp, i, &moved);
            self.count -= 1;
            i -= 1;
        }
        self.count += 1;
        self.put(rp, at, r);
        Ok(at)
    }

    /// Removes and returns region `i`; pages no longer needed go back.
    ///
    /// # Panics
    /// If `i` is out of range.
    pub fn remove(&mut self, rp: &mut impl RegionPages, i: usize) -> Region {
        let r = self.get(rp, i);
        for k in i..self.len() - 1 {
            let next = self.get(rp, k + 1);
            self.put(rp, k, &next);
        }
        self.count -= 1;
        self.trim(rp);
        r
    }

    /// Gives back every page past what the regions need.
    pub fn trim(&mut self, rp: &mut impl RegionPages) {
        let keep = self.len().div_ceil(PER_PAGE);
        for p in &mut self.pages[keep..] {
            if *p != 0 {
                rp.free_page(*p);
                *p = 0;
            }
        }
    }

    /// Empties the list and gives back all its pages.
    pub fn clear(&mut self, rp: &mut impl RegionPages) {
        self.count = 0;
        self.trim(rp);
    }

    /// Index of the first region starting after `addr` (bisection).
    pub(crate) fn first_after(&self, rp: &mut impl RegionPages, addr: u32) -> usize {
        let (mut lo, mut hi) = (0, self.len());
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.get(rp, mid).start > addr {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        lo
    }

    /// The region containing `mva`, and its index.
    pub fn find(&self, rp: &mut impl RegionPages, mva: u32) -> Option<(usize, Region)> {
        let i = self.first_after(rp, mva).checked_sub(1)?;
        let r = self.get(rp, i);
        r.contains(mva).then_some((i, r))
    }
}

const KIND_ZERO: u32 = 0;
const KIND_FILE: u32 = 1;
const KIND_PHYS: u32 = 2;

fn cache_code(c: Cache) -> u32 {
    match c {
        Cache::WriteBack => 0,
        Cache::WriteCombine => 1,
        Cache::Device => 2,
    }
}

fn cache_of(code: u32) -> Cache {
    match code {
        0 => Cache::WriteBack,
        1 => Cache::WriteCombine,
        _ => Cache::Device,
    }
}

fn encode(r: &Region, out: &mut [u8]) {
    let (kind, a, b, shared, cache) = match r.backing {
        Backing::Zero => (KIND_ZERO, 0, 0, false, 0),
        Backing::File {
            object,
            offset,
            shared,
        } => (KIND_FILE, object, offset, shared, 0),
        Backing::Phys { base, cache } => (KIND_PHYS, base, 0, false, cache_code(cache)),
    };
    let flags = u32::from(r.prot.bits()) | kind << 8 | u32::from(shared) << 10 | cache << 12;
    for (k, w) in [r.start, r.len, flags, a, b].into_iter().enumerate() {
        out[k * 4..k * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }
}

fn decode(b: &[u8]) -> Region {
    let w = |k: usize| u32::from_le_bytes([b[k * 4], b[k * 4 + 1], b[k * 4 + 2], b[k * 4 + 3]]);
    let flags = w(2);
    let backing = match (flags >> 8) & 3 {
        KIND_FILE => Backing::File {
            object: w(3),
            offset: w(4),
            shared: flags & 1 << 10 != 0,
        },
        KIND_PHYS => Backing::Phys {
            base: w(3),
            cache: cache_of((flags >> 12) & 3),
        },
        _ => Backing::Zero,
    };
    Region {
        start: w(0),
        len: w(1),
        prot: Prot::from_bits(flags as u8),
        backing,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Pages from a heap, for tests: counts what is in use.
    #[derive(Default)]
    pub(crate) struct HeapPages {
        pages: HashMap<u32, Box<[u8; 4096]>>,
        next: u32,
        pub(crate) limit: Option<usize>,
    }

    impl HeapPages {
        pub(crate) fn in_use(&self) -> usize {
            self.pages.len()
        }
    }

    impl RegionPages for HeapPages {
        fn alloc_page(&mut self) -> Option<u32> {
            if self.limit.is_some_and(|l| self.pages.len() >= l) {
                return None;
            }
            self.next += 0x1000;
            self.pages.insert(self.next, Box::new([0xEE; 4096]));
            Some(self.next)
        }
        fn free_page(&mut self, page: u32) {
            assert!(self.pages.remove(&page).is_some(), "page freed twice");
        }
        fn page(&mut self, page: u32) -> &mut [u8; 4096] {
            self.pages.get_mut(&page).expect("a live page")
        }
    }

    fn region(start: u32, kind: u32) -> Region {
        let backing = match kind % 3 {
            0 => Backing::Zero,
            1 => Backing::File {
                object: start ^ 0x5A5A,
                offset: start / 2,
                shared: start & 0x2000 != 0,
            },
            _ => Backing::Phys {
                base: start ^ 0xA000_0000,
                cache: cache_of(kind % 4),
            },
        };
        Region {
            start,
            len: 0x1000,
            prot: Prot::from_bits((kind % 16) as u8),
            backing,
        }
    }

    #[test]
    fn regions_round_trip_through_the_encoding() {
        let mut rp = HeapPages::default();
        let mut l = RegionList::new();
        for k in 0..12 {
            l.insert(&mut rp, &region(0x10_0000 + k * 0x1000, k))
                .unwrap();
        }
        for k in 0..12 {
            assert_eq!(
                l.get(&mut rp, k as usize),
                region(0x10_0000 + k * 0x1000, k)
            );
        }
    }

    #[test]
    fn matches_a_simple_model_under_random_changes() {
        let mut rp = HeapPages::default();
        let mut l = RegionList::new();
        let mut model: Vec<Region> = Vec::new();
        let mut x = 0x1234_5678u32;
        let mut rnd = || {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x
        };
        for _ in 0..20_000 {
            let r = rnd();
            if r % 3 != 0 || model.is_empty() {
                let start = (rnd() % 4000) * 0x1000;
                if model.iter().any(|m| m.start == start) {
                    continue;
                }
                let reg = region(start, rnd() % 7);
                if l.insert(&mut rp, &reg).is_ok() {
                    let at = model.partition_point(|m| m.start <= start);
                    model.insert(at, reg);
                } else {
                    assert_eq!(model.len(), MAX_REGIONS, "only the quota refuses");
                }
            } else {
                let i = rnd() as usize % model.len();
                assert_eq!(l.remove(&mut rp, i), model.remove(i));
            }
            assert_eq!(l.len(), model.len());
            assert_eq!(
                l.pages_used(),
                model.len().div_ceil(PER_PAGE),
                "pages follow use"
            );
            assert_eq!(rp.in_use(), l.pages_used());
        }
        for (i, m) in model.iter().enumerate() {
            assert_eq!(l.get(&mut rp, i), *m);
            assert_eq!(l.find(&mut rp, m.start + 0x800), Some((i, *m)));
        }
        l.clear(&mut rp);
        assert_eq!(rp.in_use(), 0, "every page back");
    }

    #[test]
    fn the_quota_and_memory_bound_it() {
        let mut rp = HeapPages::default();
        let mut l = RegionList::new();
        for k in 0..MAX_REGIONS as u32 {
            l.insert(&mut rp, &region(k * 0x1000, 0)).unwrap();
        }
        assert_eq!(
            l.insert(&mut rp, &region(0xF000_0000, 0)),
            Err(SpaceError::TooManyRegions)
        );
        assert_eq!(rp.in_use(), MAX_PAGES);
        let mut rp2 = HeapPages {
            limit: Some(1),
            ..HeapPages::default()
        };
        let mut l2 = RegionList::new();
        for k in 0..PER_PAGE as u32 {
            l2.insert(&mut rp2, &region(k * 0x1000, 0)).unwrap();
        }
        assert_eq!(
            l2.insert(&mut rp2, &region(0xF000_0000, 0)),
            Err(SpaceError::NoMemory)
        );
        assert_eq!(l2.len(), PER_PAGE, "nothing changed");
        assert_eq!(l2.find(&mut rp2, 0xF000_0000), None);
    }
}
