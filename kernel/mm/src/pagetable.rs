//! The page tables: one first-level table for the whole system (the FCSE
//! single address space, ADR-0003) and coarse second-level tables created on
//! demand.
//!
//! Second-level tables are 1 KiB, so four share one frame (a "slab"); a
//! table is freed when its last mapping goes, and a frame when its last
//! table goes. Memory for tables is reached through [`PhysMem`] and frames
//! come from a [`FrameAlloc`], so this module is pure logic: the HAL supplies
//! the real implementations, tests supply fakes.
//!
//! **VIVT caches** (ADR-0003) set the order of every unmap: clean and
//! invalidate the page's cache lines while its mapping is still valid, then
//! clear the descriptor, then invalidate its TLB entry ([`Mmu`]). New
//! mappings need no maintenance: ARMv5 TLBs never hold faulting entries.

use crate::arm::{self, Ap, Attrs, L1, L2};
use crate::frames::Frames;

/// Physical memory as the kernel sees it.
///
/// Contract for [`zero`](Self::zero) and [`copy`](Self::copy): when they
/// return, RAM holds the result and no cache line holds the destination
/// under any address (on VIVT caches, a dirty line under the kernel's address
/// would hide the data from a user mapping at another address). The HAL
/// meets this by cleaning after writing through the direct map, or by DMA.
///
/// Contract for [`sync`](Self::sync): [`PageTables`] calls it after writing
/// descriptors, and it must make them visible to the MMU's table walker
/// (clean the D-cache lines; whether the XScale walker sees the D-cache is an
/// open question, ADR-0006).
pub trait PhysMem {
    /// The first-level table (4096 entries).
    fn l1(&mut self) -> &mut [u32; 4096];
    /// Physical address of the first-level table.
    fn l1_phys(&self) -> u32;
    /// Makes `len` bytes of page-table memory at physical `phys` visible to
    /// the table walker.
    fn sync(&mut self, phys: u32, len: u32);
    /// The coarse table at physical `phys` (1 KiB aligned, inside a frame
    /// from the frame allocator).
    fn l2(&mut self, phys: u32) -> &mut [u32; 256];
    /// Physical address of frame `f`.
    fn frame_phys(&self, f: usize) -> u32;
    /// The frame containing physical address `phys`.
    fn phys_frame(&self, phys: u32) -> usize;
    /// Fills the 4 KiB page at `phys` with zeros.
    fn zero(&mut self, phys: u32);
    /// Runs `f` on the 4 KiB page at `phys` in place (no page-sized copies:
    /// kernel stacks are small). On entry `f` sees what's in RAM (the caller
    /// has cleaned any user mapping's cache lines); when it returns, RAM
    /// holds what `f` wrote and no cache line holds the page under the
    /// kernel's address, as for [`zero`](Self::zero).
    fn with_page<R>(&mut self, phys: u32, f: impl FnOnce(&mut [u8; 4096]) -> R) -> R;
    /// A kernel-private page (never mapped for a process, so it can't alias),
    /// for the kernel's own structures such as the compressed page store.
    fn kernel_page(&mut self, phys: u32) -> &mut [u8; 4096];
    /// Copies the 4 KiB page at `src` to `dst`.
    fn copy(&mut self, dst: u32, src: u32);
}

/// Cache and TLB maintenance, by modified virtual address.
pub trait Mmu {
    /// Writes back and discards the D-cache lines of the page at `mva`, and
    /// discards its I-cache lines. Called while the page is still mapped.
    fn clean_page(&mut self, mva: u32);
    /// Discards the TLB entry for `mva`.
    fn invalidate_tlb(&mut self, mva: u32);
    /// Writes back and discards both caches entirely (cheaper than many
    /// per-page operations for big ranges).
    fn clean_all(&mut self);
    /// Discards every TLB entry.
    fn invalidate_tlb_all(&mut self);
}

/// Where page-table frames come from.
pub trait FrameAlloc {
    /// Allocates 2^`order` frames, returning the first.
    fn alloc(&mut self, order: usize) -> Option<usize>;
    /// Frees a block from [`alloc`](Self::alloc).
    fn free(&mut self, f: usize);
}

impl<const N: usize> FrameAlloc for Frames<N> {
    fn alloc(&mut self, order: usize) -> Option<usize> {
        Frames::alloc(self, order)
    }

    fn free(&mut self, f: usize) {
        let r = Frames::free(self, f);
        // A failed free is a kernel bug (double free); never let it pass.
        debug_assert!(r.is_ok(), "freeing a frame that isn't allocated");
    }
}

/// Why a mapping was refused.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub enum MapError {
    /// No memory for a second-level table.
    NoMemory,
    /// Something is already mapped there.
    AlreadyMapped,
    /// The address or physical address isn't aligned to the page size.
    Misaligned,
    /// The megabyte is mapped as a section or a table in another domain.
    Conflict,
    /// Every slab slot is in use.
    TooManyTables,
}

impl MapError {
    /// A short name for logs.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            MapError::NoMemory => "no memory",
            MapError::AlreadyMapped => "already mapped",
            MapError::Misaligned => "misaligned",
            MapError::Conflict => "conflict",
            MapError::TooManyTables => "too many tables",
        }
    }
}

/// One page that was unmapped, as reported to the caller.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub struct Unmapped {
    /// Its address.
    pub mva: u32,
    /// The physical page (for a large page, the 4 KiB piece at `mva`).
    pub phys: u32,
    /// Its attributes.
    pub attrs: Attrs,
}

/// Frames holding second-level tables: at most this many (4 tables each,
/// 4 MiB of mapped address space per frame).
pub const MAX_SLABS: usize = 256;
/// Above this many pages, an unmap cleans the whole cache once instead of
/// page by page (32 KiB D-cache: a page-by-page clean of more than a few
/// pages costs more than cleaning everything).
pub const CLEAN_ALL_THRESHOLD: u32 = 8;

const PAGE: u32 = 4096;
const LARGE: u32 = 64 * 1024;
const MIB: u32 = 1 << 20;

/// The system's page tables.
pub struct PageTables {
    /// Valid second-level entries per first-level index (to free empty tables).
    used: [u16; 4096],
    /// Frames carved into tables (frame + 1; 0 = unused slot).
    slab_frame: [u16; MAX_SLABS],
    /// Per slab: bit q set = quarter q is free.
    slab_free: [u8; MAX_SLABS],
}

impl Default for PageTables {
    fn default() -> Self {
        Self::new()
    }
}

impl PageTables {
    /// Empty bookkeeping (all zero: lives in `.bss`). The first-level table
    /// itself is reached through [`PhysMem::l1`] and starts all faults.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            used: [0; 4096],
            slab_frame: [0; MAX_SLABS],
            slab_free: [0; MAX_SLABS],
        }
    }

    /// Maps the 1 MiB section at `mva` to `phys` (kernel mappings: the direct
    /// map, device windows).
    pub fn map_section(
        &mut self,
        mem: &mut impl PhysMem,
        mva: u32,
        phys: u32,
        domain: u32,
        attrs: Attrs,
    ) -> Result<(), MapError> {
        if !mva.is_multiple_of(MIB) || !phys.is_multiple_of(MIB) {
            return Err(MapError::Misaligned);
        }
        let l1 = &mut mem.l1()[(mva >> 20) as usize];
        if *l1 != 0 {
            return Err(MapError::AlreadyMapped);
        }
        *l1 = arm::l1_section(phys, domain, attrs);
        let at = mem.l1_phys() + (mva >> 20) * 4;
        mem.sync(at, 4);
        Ok(())
    }

    /// Maps the 4 KiB page at `mva` to `phys`.
    pub fn map_small(
        &mut self,
        mem: &mut impl PhysMem,
        frames: &mut impl FrameAlloc,
        mva: u32,
        phys: u32,
        domain: u32,
        attrs: Attrs,
    ) -> Result<(), MapError> {
        if !mva.is_multiple_of(PAGE) || !phys.is_multiple_of(PAGE) {
            return Err(MapError::Misaligned);
        }
        let table = self.table_for(mem, frames, mva, domain)?;
        let i = ((mva >> 12) & 0xFF) as usize;
        let t = mem.l2(table);
        if t[i] != 0 {
            return Err(MapError::AlreadyMapped);
        }
        t[i] = arm::l2_small(phys, attrs);
        mem.sync(table + i as u32 * 4, 4);
        self.used[(mva >> 20) as usize] += 1;
        Ok(())
    }

    /// Maps the 64 KiB page at `mva` to `phys` (one TLB entry for 16 pages,
    /// ADR-0006 rule 2).
    pub fn map_large(
        &mut self,
        mem: &mut impl PhysMem,
        frames: &mut impl FrameAlloc,
        mva: u32,
        phys: u32,
        domain: u32,
        attrs: Attrs,
    ) -> Result<(), MapError> {
        if !mva.is_multiple_of(LARGE) || !phys.is_multiple_of(LARGE) {
            return Err(MapError::Misaligned);
        }
        let table = self.table_for(mem, frames, mva, domain)?;
        let first = ((mva >> 12) & 0xF0) as usize;
        let t = mem.l2(table);
        if t[first..first + 16].iter().any(|&e| e != 0) {
            // A table created just now for this call stays (empty) until its
            // megabyte is next unmapped; harmless and bounded.
            return Err(MapError::AlreadyMapped);
        }
        t[first..first + 16].fill(arm::l2_large(phys, attrs));
        mem.sync(table + first as u32 * 4, 64);
        self.used[(mva >> 20) as usize] += 16;
        Ok(())
    }

    /// The physical address and attributes `mva` translates to.
    pub fn translate(&self, mem: &mut impl PhysMem, mva: u32) -> Option<(u32, Attrs)> {
        match arm::l1_decode(mem.l1()[(mva >> 20) as usize]) {
            L1::Section { phys, attrs, .. } => Some((phys | (mva & (MIB - 1)), attrs)),
            L1::Coarse { table_phys, .. } => {
                match arm::l2_decode(mem.l2(table_phys)[((mva >> 12) & 0xFF) as usize]) {
                    L2::Small { phys, attrs } => Some((phys | (mva & (PAGE - 1)), attrs)),
                    L2::Large { phys, attrs } => Some((phys | (mva & (LARGE - 1)), attrs)),
                    L2::Fault | L2::Stored(_) => None,
                }
            }
            L1::Fault | L1::Fine => None,
        }
    }

    /// Changes the permissions of every page mapped in `mva..mva + len`.
    /// Physical pages and cache policy stay as they are (a device window
    /// must never become cacheable). Returns how many entries changed.
    pub fn protect(
        &mut self,
        mem: &mut impl PhysMem,
        mmu: &mut impl Mmu,
        mva: u32,
        len: u32,
        ap: Ap,
    ) -> u32 {
        let mut changed = 0;
        let big = len / PAGE > CLEAN_ALL_THRESHOLD;
        self.for_each_entry(mem, mva, len, |t, i, page_mva| {
            let new = match arm::l2_decode(t[i]) {
                L2::Small { phys, attrs } => arm::l2_small(phys, Attrs { ap, ..attrs }),
                L2::Large { phys, attrs } => arm::l2_large(phys, Attrs { ap, ..attrs }),
                L2::Fault | L2::Stored(_) => return,
            };
            if new != t[i] {
                t[i] = new;
                changed += 1;
                if !big {
                    mmu.invalidate_tlb(page_mva);
                }
            }
        });
        if big && changed > 0 {
            mmu.invalidate_tlb_all();
        }
        changed
    }

    /// Unmaps every page in `mva..mva + len`, reporting each one to
    /// `release` (which frees or returns the frame, and gets the frame
    /// allocator for that). Large pages must be
    /// unmapped whole.
    pub fn unmap<F: FrameAlloc>(
        &mut self,
        mem: &mut impl PhysMem,
        frames: &mut F,
        mmu: &mut impl Mmu,
        mva: u32,
        len: u32,
        mut release: impl FnMut(Unmapped, &mut F),
    ) {
        let big = len / PAGE > CLEAN_ALL_THRESHOLD;
        if big {
            mmu.clean_all(); // while everything is still mapped
        }
        let used = &mut self.used;
        Self::walk(mem, mva, len, |t, i, page_mva| {
            let (phys, attrs, large) = match arm::l2_decode(t[i]) {
                L2::Small { phys, attrs } => (phys, attrs, false),
                L2::Large { phys, attrs } => (phys | (page_mva & (LARGE - 1)), attrs, true),
                L2::Fault => return,
                L2::Stored(_) => {
                    // Stored pages are drained first (take_stored), so their
                    // payloads are freed; reaching one here leaks it.
                    debug_assert!(false, "unmapping a stored page without take_stored");
                    return;
                }
            };
            debug_assert!(
                !large || {
                    let base = page_mva & !(LARGE - 1);
                    base >= mva
                        && u64::from(base) + u64::from(LARGE) <= u64::from(mva) + u64::from(len)
                },
                "large pages must be unmapped whole"
            );
            if !big {
                mmu.clean_page(page_mva);
            }
            release(
                Unmapped {
                    mva: page_mva,
                    phys,
                    attrs,
                },
                frames,
            );
            t[i] = 0;
            if !big {
                mmu.invalidate_tlb(page_mva);
            }
            used[(page_mva >> 20) as usize] -= 1;
        });
        if big {
            mmu.invalidate_tlb_all();
        }
        // Free the tables this emptied (their TLB entries are already gone).
        if len > 0 {
            let last = mva.saturating_add(len - 1) >> 20;
            for idx in (mva >> 20)..=last {
                self.free_table(mem, frames, idx as usize);
            }
        }
    }

    /// Calls `f(table, index, mva)` for every second-level entry in range
    /// (megabytes without a coarse table are skipped), then syncs the
    /// entries visited.
    fn walk(
        mem: &mut impl PhysMem,
        mva: u32,
        len: u32,
        mut f: impl FnMut(&mut [u32; 256], usize, u32),
    ) {
        let end = mva.saturating_add(len);
        let mut page = mva & !(PAGE - 1);
        while page < end {
            let idx = (page >> 20) as usize;
            if let L1::Coarse { table_phys, .. } = arm::l1_decode(mem.l1()[idx]) {
                let t = mem.l2(table_phys);
                let stop = end.min((page | (MIB - 1)).saturating_add(1));
                let first = (page >> 12) & 0xFF;
                let mut n = 0;
                while page < stop {
                    f(t, ((page >> 12) & 0xFF) as usize, page);
                    page += PAGE;
                    n += 1;
                }
                // The walk may have changed any entry it visited.
                mem.sync(table_phys + first * 4, n * 4);
            } else {
                page = (page | (MIB - 1)).saturating_add(1);
            }
            if page == 0 {
                break; // wrapped past the top of the address space
            }
        }
    }

    fn for_each_entry(
        &mut self,
        mem: &mut impl PhysMem,
        mva: u32,
        len: u32,
        f: impl FnMut(&mut [u32; 256], usize, u32),
    ) {
        Self::walk(mem, mva, len, f);
    }

    /// The coarse table for `mva`'s megabyte, creating it if needed.
    fn table_for(
        &mut self,
        mem: &mut impl PhysMem,
        frames: &mut impl FrameAlloc,
        mva: u32,
        domain: u32,
    ) -> Result<u32, MapError> {
        let idx = (mva >> 20) as usize;
        match arm::l1_decode(mem.l1()[idx]) {
            L1::Coarse {
                table_phys,
                domain: d,
            } => {
                if d == domain {
                    Ok(table_phys)
                } else {
                    Err(MapError::Conflict)
                }
            }
            L1::Section { .. } | L1::Fine => Err(MapError::Conflict),
            L1::Fault => {
                let table = self.alloc_table(mem, frames)?;
                mem.l2(table).fill(0);
                mem.sync(table, 1024);
                mem.l1()[idx] = arm::l1_coarse(table, domain);
                let at = mem.l1_phys() + idx as u32 * 4;
                mem.sync(at, 4);
                self.used[idx] = 0;
                Ok(table)
            }
        }
    }

    fn alloc_table(
        &mut self,
        mem: &mut impl PhysMem,
        frames: &mut impl FrameAlloc,
    ) -> Result<u32, MapError> {
        // A free quarter in an existing slab first.
        if let Some(s) = (0..MAX_SLABS).find(|&s| self.slab_frame[s] != 0 && self.slab_free[s] != 0)
        {
            let q = self.slab_free[s].trailing_zeros();
            self.slab_free[s] &= !(1 << q);
            let frame = usize::from(self.slab_frame[s] - 1);
            return Ok(mem.frame_phys(frame) + q * 1024);
        }
        let s = (0..MAX_SLABS)
            .find(|&s| self.slab_frame[s] == 0)
            .ok_or(MapError::TooManyTables)?;
        let frame = frames.alloc(0).ok_or(MapError::NoMemory)?;
        debug_assert!(frame < usize::from(u16::MAX), "frame index fits in u16");
        self.slab_frame[s] = frame as u16 + 1;
        self.slab_free[s] = 0b1110; // quarter 0 handed out now
        Ok(mem.frame_phys(frame))
    }

    fn free_table(&mut self, mem: &mut impl PhysMem, frames: &mut impl FrameAlloc, idx: usize) {
        let L1::Coarse { table_phys, .. } = arm::l1_decode(mem.l1()[idx]) else {
            return;
        };
        if self.used[idx] != 0 {
            return; // remapped since it emptied
        }
        mem.l1()[idx] = 0;
        let at = mem.l1_phys() + idx as u32 * 4;
        mem.sync(at, 4);
        let frame = mem.phys_frame(table_phys);
        let q = (table_phys % PAGE) / 1024;
        let Some(s) = (0..MAX_SLABS).find(|&s| usize::from(self.slab_frame[s]) == frame + 1) else {
            debug_assert!(false, "table not in any slab");
            return;
        };
        self.slab_free[s] |= 1 << q;
        if self.slab_free[s] == 0b1111 {
            self.slab_frame[s] = 0;
            self.slab_free[s] = 0;
            frames.free(frame);
        }
    }

    /// Moves every coarse table in `mva..mva + len` to `domain` (a process
    /// gaining or losing its ARM domain, see [`crate::domains`]): rewrites
    /// the first-level descriptors, syncs them and drops the TLB entries,
    /// which cache the old domain. Returns how many tables moved.
    pub fn retag(
        &mut self,
        mem: &mut impl PhysMem,
        mmu: &mut impl Mmu,
        mva: u32,
        len: u32,
        domain: u32,
    ) -> u32 {
        let mut moved = 0;
        let first = (mva >> 20) as usize;
        let count = (len as usize).div_ceil(MIB as usize);
        for idx in first..(first + count).min(4096) {
            if let L1::Coarse {
                table_phys,
                domain: d,
            } = arm::l1_decode(mem.l1()[idx])
                && d != domain
            {
                mem.l1()[idx] = arm::l1_coarse(table_phys, domain);
                let at = mem.l1_phys() + idx as u32 * 4;
                mem.sync(at, 4);
                moved += 1;
            }
        }
        if moved > 0 {
            mmu.invalidate_tlb_all();
        }
        moved
    }

    /// Replaces the small page mapped at `mva` with a stored entry carrying
    /// `payload` (a compressed copy's handle), returning the physical page
    /// it held. The caller has already cleaned the page's cache lines and
    /// read its contents. `None` (and nothing changed) if `mva` isn't a
    /// mapped small page.
    pub fn store(
        &mut self,
        mem: &mut impl PhysMem,
        mmu: &mut impl Mmu,
        mva: u32,
        payload: u32,
    ) -> Option<u32> {
        let (table, i) = self.entry(mem, mva)?;
        let L2::Small { phys, .. } = arm::l2_decode(mem.l2(table)[i]) else {
            return None;
        };
        mem.l2(table)[i] = arm::l2_stored(payload);
        mem.sync(table + i as u32 * 4, 4);
        mmu.invalidate_tlb(mva & !(PAGE - 1));
        Some(phys)
    }

    /// The payload stored at `mva`, if its entry is a stored one.
    pub fn stored(&self, mem: &mut impl PhysMem, mva: u32) -> Option<u32> {
        let (table, i) = self.entry(mem, mva)?;
        match arm::l2_decode(mem.l2(table)[i]) {
            L2::Stored(p) => Some(p),
            _ => None,
        }
    }

    /// Maps `phys` at `mva` in place of its stored entry (the page came
    /// back). No TLB maintenance: a fault entry is never cached.
    pub fn restore(
        &mut self,
        mem: &mut impl PhysMem,
        mva: u32,
        phys: u32,
        attrs: Attrs,
    ) -> Result<(), MapError> {
        let (table, i) = self.entry(mem, mva).ok_or(MapError::Conflict)?;
        if !matches!(arm::l2_decode(mem.l2(table)[i]), L2::Stored(_)) {
            return Err(MapError::AlreadyMapped);
        }
        mem.l2(table)[i] = arm::l2_small(phys, attrs);
        mem.sync(table + i as u32 * 4, 4);
        Ok(())
    }

    /// Clears stored entries from `mva` towards `end`, putting their
    /// payloads in `out`, and returns how many it took and where to resume
    /// (`>= end` when done). Stops when `out` is full, so callers free each
    /// batch (which needs memory access the walk can't share) and continue:
    /// bounded, and nothing is lost.
    pub fn take_stored(
        &mut self,
        mem: &mut impl PhysMem,
        mva: u32,
        end: u32,
        out: &mut [u32],
    ) -> (usize, u32) {
        let mut n = 0;
        let mut page = mva & !(PAGE - 1);
        while page < end && n < out.len() {
            let idx = (page >> 20) as usize;
            let next_mib = (page | (MIB - 1)).wrapping_add(1);
            if let L1::Coarse { table_phys, .. } = arm::l1_decode(mem.l1()[idx]) {
                let mut changed = None;
                while page < end && page != next_mib && n < out.len() {
                    let i = ((page >> 12) & 0xFF) as usize;
                    if let L2::Stored(p) = arm::l2_decode(mem.l2(table_phys)[i]) {
                        out[n] = p;
                        n += 1;
                        mem.l2(table_phys)[i] = 0;
                        self.used[idx] -= 1;
                        changed = Some(changed.map_or((i, i), |(a, _): (usize, usize)| (a, i)));
                    }
                    page += PAGE;
                }
                if let Some((a, b)) = changed {
                    mem.sync(table_phys + a as u32 * 4, (b - a + 1) as u32 * 4);
                }
            } else {
                page = next_mib;
            }
            if page == 0 {
                return (n, u32::MAX); // wrapped past the top
            }
        }
        (n, page)
    }

    /// The coarse table and index holding `mva`'s entry.
    fn entry(&self, mem: &mut impl PhysMem, mva: u32) -> Option<(u32, usize)> {
        match arm::l1_decode(mem.l1()[(mva >> 20) as usize]) {
            L1::Coarse { table_phys, .. } => Some((table_phys, ((mva >> 12) & 0xFF) as usize)),
            _ => None,
        }
    }

    /// Frames currently holding second-level tables.
    #[must_use]
    pub fn table_frames(&self) -> usize {
        self.slab_frame.iter().filter(|&&f| f != 0).count()
    }
}

#[cfg(test)]
pub(crate) mod fake {
    //! Host fakes for [`PhysMem`] and [`Mmu`].
    use super::*;
    use std::collections::HashMap;

    /// Where the fake pretends its first-level table lives.
    pub const FAKE_L1: u32 = 0xA800_4000;

    /// RAM as a map of 4 KiB pages, based at the X51v's SDRAM address.
    pub struct Ram {
        pub l1: Box<[u32; 4096]>,
        pub pages: HashMap<u32, Box<[u32; 1024]>>,
        /// Every `sync(phys, len)` call, in order.
        pub synced: Vec<(u32, u32)>,
        /// Kernel-private pages, as bytes.
        pub bytes: HashMap<u32, Box<[u8; 4096]>>,
    }

    impl Ram {
        pub fn new() -> Self {
            Self {
                l1: Box::new([0; 4096]),
                pages: HashMap::new(),
                synced: Vec::new(),
                bytes: HashMap::new(),
            }
        }

        pub fn page(&mut self, phys: u32) -> &mut [u32; 1024] {
            self.pages
                .entry(phys & !0xFFF)
                .or_insert_with(|| Box::new([0xDEAD_BEEF; 1024]))
        }
    }

    impl PhysMem for Ram {
        fn l1(&mut self) -> &mut [u32; 4096] {
            &mut self.l1
        }
        fn l1_phys(&self) -> u32 {
            FAKE_L1
        }
        fn sync(&mut self, phys: u32, len: u32) {
            self.synced.push((phys, len));
        }
        fn l2(&mut self, phys: u32) -> &mut [u32; 256] {
            let off = ((phys & 0xFFF) / 4) as usize;
            let page = self.page(phys);
            (&mut page[off..off + 256]).try_into().unwrap()
        }
        fn frame_phys(&self, f: usize) -> u32 {
            crate::layout::RAM_PHYS + (f as u32) * 4096
        }
        fn phys_frame(&self, phys: u32) -> usize {
            ((phys - crate::layout::RAM_PHYS) / 4096) as usize
        }
        fn zero(&mut self, phys: u32) {
            self.page(phys).fill(0);
        }
        fn with_page<R>(&mut self, phys: u32, f: impl FnOnce(&mut [u8; 4096]) -> R) -> R {
            let mut bytes = [0u8; 4096];
            for (o, w) in bytes
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .zip(self.page(phys).iter())
            {
                *o = w.to_le_bytes();
            }
            let r = f(&mut bytes);
            for (w, b) in self.page(phys).iter_mut().zip(bytes.as_chunks::<4>().0) {
                *w = u32::from_le_bytes(*b);
            }
            r
        }
        fn kernel_page(&mut self, phys: u32) -> &mut [u8; 4096] {
            self.bytes
                .entry(phys & !0xFFF)
                .or_insert_with(|| Box::new([0; 4096]))
        }
        fn copy(&mut self, dst: u32, src: u32) {
            let s = *self.page(src);
            *self.page(dst) = s;
        }
    }

    /// Records every maintenance operation in order.
    #[derive(Default)]
    pub struct Log(pub Vec<String>);

    impl Mmu for Log {
        fn clean_page(&mut self, mva: u32) {
            self.0.push(format!("clean {mva:#x}"));
        }
        fn invalidate_tlb(&mut self, mva: u32) {
            self.0.push(format!("tlb {mva:#x}"));
        }
        fn clean_all(&mut self) {
            self.0.push("clean all".into());
        }
        fn invalidate_tlb_all(&mut self) {
            self.0.push("tlb all".into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::{Log, Ram};
    use super::*;
    use crate::arm::{Ap, Cache};

    const RW: Attrs = Attrs {
        ap: Ap::UserWrite,
        cache: Cache::WriteBack,
    };
    const RO: Attrs = Attrs {
        ap: Ap::UserRead,
        cache: Cache::WriteBack,
    };

    fn setup() -> (PageTables, Ram, Frames<256>) {
        let mut f = Frames::<256>::new();
        f.add_range(0, 256).unwrap();
        (PageTables::new(), Ram::new(), f)
    }

    fn phys(f: usize) -> u32 {
        crate::layout::RAM_PHYS + f as u32 * 4096
    }

    #[test]
    fn maps_and_translates_small_pages() {
        let (mut pt, mut ram, mut fr) = setup();
        pt.map_small(&mut ram, &mut fr, 0x0A01_0000, phys(100), 3, RW)
            .unwrap();
        assert_eq!(
            pt.translate(&mut ram, 0x0A01_0123),
            Some((phys(100) + 0x123, RW))
        );
        assert_eq!(pt.translate(&mut ram, 0x0A01_1000), None);
        // One coarse table in domain 3 for that megabyte.
        assert!(matches!(
            arm::l1_decode(ram.l1[0x0A0]),
            L1::Coarse { domain: 3, .. }
        ));
        assert_eq!(pt.table_frames(), 1);
    }

    #[test]
    fn every_descriptor_write_is_synced() {
        let (mut pt, mut ram, mut fr) = setup();
        pt.map_small(&mut ram, &mut fr, 0x0A01_3000, phys(100), 3, RW)
            .unwrap();
        let table = match arm::l1_decode(ram.l1[0x0A0]) {
            L1::Coarse { table_phys, .. } => table_phys,
            _ => unreachable!(),
        };
        assert_eq!(
            ram.synced,
            [
                (table, 1024),                         // new table zeroed
                (super::fake::FAKE_L1 + 0x0A0 * 4, 4), // L1 entry
                (table + 0x13 * 4, 4),                 // the page's entry
            ]
        );
        ram.synced.clear();
        pt.unmap(
            &mut ram,
            &mut fr,
            &mut Log::default(),
            0x0A01_3000,
            0x1000,
            |_, _| {},
        );
        assert_eq!(
            ram.synced,
            [(table + 0x13 * 4, 4), (super::fake::FAKE_L1 + 0x0A0 * 4, 4)],
            "cleared entry, then the freed table's L1 entry"
        );
    }

    #[test]
    fn never_overwrites_a_mapping() {
        let (mut pt, mut ram, mut fr) = setup();
        pt.map_small(&mut ram, &mut fr, 0x1000, phys(10), 0, RW)
            .unwrap();
        assert_eq!(
            pt.map_small(&mut ram, &mut fr, 0x1000, phys(11), 0, RW),
            Err(MapError::AlreadyMapped)
        );
        assert_eq!(pt.translate(&mut ram, 0x1000), Some((phys(10), RW)));
        assert_eq!(
            pt.map_small(&mut ram, &mut fr, 0x2000, phys(12), 1, RW),
            Err(MapError::Conflict),
            "same megabyte, other domain"
        );
        assert_eq!(
            pt.map_small(&mut ram, &mut fr, 0x2001, phys(12), 0, RW),
            Err(MapError::Misaligned)
        );
    }

    #[test]
    fn four_tables_share_one_frame() {
        let (mut pt, mut ram, mut fr) = setup();
        let before = fr.free_frames();
        for mib in 0..4u32 {
            pt.map_small(&mut ram, &mut fr, mib << 20, phys(50 + mib as usize), 0, RW)
                .unwrap();
        }
        assert_eq!(pt.table_frames(), 1);
        assert_eq!(before - fr.free_frames(), 1);
        pt.map_small(&mut ram, &mut fr, 4 << 20, phys(60), 0, RW)
            .unwrap();
        assert_eq!(pt.table_frames(), 2);
    }

    #[test]
    fn large_pages_fill_sixteen_entries() {
        let (mut pt, mut ram, mut fr) = setup();
        let p = crate::layout::RAM_PHYS + 0x10_0000;
        pt.map_large(&mut ram, &mut fr, 0x0403_0000, p, 0, RO)
            .unwrap();
        assert_eq!(pt.translate(&mut ram, 0x0403_0000), Some((p, RO)));
        assert_eq!(pt.translate(&mut ram, 0x0403_FFFC), Some((p + 0xFFFC, RO)));
        assert_eq!(pt.translate(&mut ram, 0x0404_0000), None);
        assert_eq!(
            pt.map_small(&mut ram, &mut fr, 0x0403_5000, phys(1), 0, RW),
            Err(MapError::AlreadyMapped)
        );
        assert_eq!(
            pt.map_large(&mut ram, &mut fr, 0x0403_1000, p, 0, RO),
            Err(MapError::Misaligned)
        );
    }

    #[test]
    fn unmap_cleans_while_mapped_then_clears_then_invalidates() {
        let (mut pt, mut ram, mut fr) = setup();
        pt.map_small(&mut ram, &mut fr, 0x5000, phys(7), 0, RW)
            .unwrap();
        pt.map_small(&mut ram, &mut fr, 0x6000, phys(8), 0, RW)
            .unwrap();
        let mut mmu = Log::default();
        let mut released = Vec::new();
        // `release` sees each page while the cache is clean and before the
        // TLB entry goes; the exact order matters on VIVT caches.
        pt.unmap(&mut ram, &mut fr, &mut mmu, 0x5000, 0x2000, |u, _| {
            released.push(u)
        });
        assert_eq!(
            mmu.0,
            ["clean 0x5000", "tlb 0x5000", "clean 0x6000", "tlb 0x6000"]
        );
        assert_eq!(released.len(), 2);
        assert_eq!(released[0].phys, phys(7));
        assert_eq!(pt.translate(&mut ram, 0x5000), None);
        // The megabyte emptied: its table and the slab frame are freed.
        assert_eq!(ram.l1[0], 0);
        assert_eq!(pt.table_frames(), 0);
        assert_eq!(fr.free_frames(), 256);
    }

    #[test]
    fn big_unmaps_clean_everything_once() {
        let (mut pt, mut ram, mut fr) = setup();
        for i in 0..20u32 {
            pt.map_small(&mut ram, &mut fr, i * 4096, phys(100 + i as usize), 0, RW)
                .unwrap();
        }
        let mut mmu = Log::default();
        let mut n = 0;
        pt.unmap(&mut ram, &mut fr, &mut mmu, 0, 20 * 4096, |_, _| n += 1);
        assert_eq!(n, 20);
        assert_eq!(mmu.0, ["clean all", "tlb all"]);
    }

    #[test]
    fn unmap_reports_every_piece_of_a_large_page() {
        let (mut pt, mut ram, mut fr) = setup();
        let p = crate::layout::RAM_PHYS + 0x20_0000;
        pt.map_large(&mut ram, &mut fr, 0x0001_0000, p, 0, RW)
            .unwrap();
        let mut got = Vec::new();
        pt.unmap(
            &mut ram,
            &mut fr,
            &mut Log::default(),
            0x0001_0000,
            0x1_0000,
            |u, _| {
                got.push(u.phys);
            },
        );
        assert_eq!(got.len(), 16);
        assert_eq!(got[1], p + 0x1000);
        assert_eq!(pt.table_frames(), 0);
    }

    #[test]
    #[should_panic(expected = "unmapped whole")]
    fn half_a_large_page_cannot_be_unmapped() {
        let (mut pt, mut ram, mut fr) = setup();
        let p = crate::layout::RAM_PHYS + 0x20_0000;
        pt.map_large(&mut ram, &mut fr, 0x0001_0000, p, 0, RW)
            .unwrap();
        pt.unmap(
            &mut ram,
            &mut fr,
            &mut Log::default(),
            0x0001_8000,
            0x8000,
            |_, _| {},
        );
    }

    #[test]
    fn protect_rewrites_attributes_and_invalidates() {
        let (mut pt, mut ram, mut fr) = setup();
        pt.map_small(&mut ram, &mut fr, 0x3000, phys(3), 0, RW)
            .unwrap();
        let mut mmu = Log::default();
        assert_eq!(
            pt.protect(&mut ram, &mut mmu, 0x3000, 0x2000, Ap::UserRead),
            1
        );
        assert_eq!(pt.translate(&mut ram, 0x3000), Some((phys(3), RO)));
        assert_eq!(mmu.0, ["tlb 0x3000"]);
        // Already read-only: nothing to do.
        assert_eq!(
            pt.protect(&mut ram, &mut mmu, 0x3000, 0x1000, Ap::UserRead),
            0
        );
    }

    #[test]
    fn protect_keeps_the_cache_policy() {
        let (mut pt, mut ram, mut fr) = setup();
        let dev = Attrs {
            ap: Ap::UserWrite,
            cache: Cache::Device,
        };
        pt.map_small(&mut ram, &mut fr, 0x7000, 0x4000_0000, 0, dev)
            .unwrap();
        pt.protect(&mut ram, &mut Log::default(), 0x7000, 0x1000, Ap::UserRead);
        let (_, attrs) = pt.translate(&mut ram, 0x7000).unwrap();
        assert_eq!(attrs.cache, Cache::Device, "device memory stays uncached");
        assert_eq!(attrs.ap, Ap::UserRead);
    }

    #[test]
    fn retag_moves_a_slot_to_another_domain() {
        let (mut pt, mut ram, mut fr) = setup();
        pt.map_small(&mut ram, &mut fr, 0x0A01_0000, phys(1), 3, RW)
            .unwrap();
        pt.map_small(&mut ram, &mut fr, 0x0A20_0000, phys(2), 3, RW)
            .unwrap();
        pt.map_small(&mut ram, &mut fr, 0x0C00_0000, phys(3), 4, RW)
            .unwrap(); // next slot
        let mut mmu = Log::default();
        assert_eq!(pt.retag(&mut ram, &mut mmu, 0x0A00_0000, 32 << 20, 15), 2);
        assert!(matches!(
            arm::l1_decode(ram.l1[0x0A0]),
            L1::Coarse { domain: 15, .. }
        ));
        assert!(matches!(
            arm::l1_decode(ram.l1[0x0A2]),
            L1::Coarse { domain: 15, .. }
        ));
        assert!(
            matches!(arm::l1_decode(ram.l1[0x0C0]), L1::Coarse { domain: 4, .. }),
            "other slot untouched"
        );
        assert_eq!(mmu.0, ["tlb all"]);
        // Mappings survive, and new ones in that megabyte use the new domain.
        assert_eq!(pt.translate(&mut ram, 0x0A01_0000), Some((phys(1), RW)));
        pt.map_small(&mut ram, &mut fr, 0x0A01_1000, phys(4), 15, RW)
            .unwrap();
        // Nothing to move: no TLB flush.
        mmu.0.clear();
        assert_eq!(pt.retag(&mut ram, &mut mmu, 0x0A00_0000, 32 << 20, 15), 0);
        assert!(mmu.0.is_empty());
    }

    #[test]
    fn stored_entries_replace_and_restore_mappings() {
        let (mut pt, mut ram, mut fr) = setup();
        pt.map_small(&mut ram, &mut fr, 0x5000, phys(9), 0, RW)
            .unwrap();
        let mut mmu = Log::default();
        assert_eq!(pt.store(&mut ram, &mut mmu, 0x5000, 0x77), Some(phys(9)));
        assert_eq!(mmu.0, ["tlb 0x5000"]);
        assert_eq!(pt.translate(&mut ram, 0x5000), None, "faults now");
        assert_eq!(pt.stored(&mut ram, 0x5000), Some(0x77));
        assert_eq!(
            pt.store(&mut ram, &mut mmu, 0x5000, 0x78),
            None,
            "not mapped"
        );
        assert_eq!(
            pt.map_small(&mut ram, &mut fr, 0x5000, phys(10), 0, RW),
            Err(MapError::AlreadyMapped),
            "a stored entry isn't free"
        );
        pt.restore(&mut ram, 0x5000, phys(10), RW).unwrap();
        assert_eq!(pt.translate(&mut ram, 0x5000), Some((phys(10), RW)));
        assert_eq!(pt.stored(&mut ram, 0x5000), None);
    }

    #[test]
    fn take_stored_drains_in_bounded_batches() {
        let (mut pt, mut ram, mut fr) = setup();
        let mut mmu = Log::default();
        // 300 stored pages across two megabytes, one ordinary mapping.
        for k in 0..300u32 {
            let mva = 0x0010_0000 + k * 4096;
            pt.map_small(&mut ram, &mut fr, mva, phys(20), 0, RW)
                .unwrap();
            pt.store(&mut ram, &mut mmu, mva, k + 1).unwrap();
        }
        pt.map_small(&mut ram, &mut fr, 0x0030_0000, phys(21), 0, RW)
            .unwrap();
        let mut got = Vec::new();
        let mut buf = [0u32; 64];
        let (mut at, end) = (0x0010_0000, 0x0040_0000);
        while at < end {
            let (n, next) = pt.take_stored(&mut ram, at, end, &mut buf);
            assert!(n <= 64);
            got.extend_from_slice(&buf[..n]);
            at = next;
        }
        assert_eq!(
            got,
            (1..=300).collect::<Vec<_>>(),
            "every payload, in order, once"
        );
        assert_eq!(pt.stored(&mut ram, 0x0010_0000), None);
        assert_eq!(
            pt.translate(&mut ram, 0x0030_0000),
            Some((phys(21), RW)),
            "mapping untouched"
        );
        // The emptied tables are freed by the normal unmap.
        pt.unmap(
            &mut ram,
            &mut fr,
            &mut mmu,
            0x0010_0000,
            0x0030_0000,
            |_, _| {},
        );
        pt.unmap(&mut ram, &mut fr, &mut mmu, 0x0030_0000, 0x1000, |_, _| {});
        assert_eq!(pt.table_frames(), 0);
    }

    #[test]
    fn sections_for_the_kernel_direct_map() {
        let (mut pt, mut ram, _) = setup();
        let k = Attrs {
            ap: Ap::Kernel,
            cache: Cache::WriteBack,
        };
        pt.map_section(&mut ram, 0x8000_0000, 0xA800_0000, 0, k)
            .unwrap();
        assert_eq!(pt.translate(&mut ram, 0x8001_2345), Some((0xA801_2345, k)));
        assert_eq!(
            pt.map_section(&mut ram, 0x8000_0000, 0xA810_0000, 0, k),
            Err(MapError::AlreadyMapped)
        );
        assert_eq!(
            pt.map_small(
                &mut ram,
                &mut Frames::<4>::new(),
                0x8000_1000,
                phys(1),
                0,
                k
            ),
            Err(MapError::Conflict)
        );
    }

    #[test]
    fn out_of_memory_is_reported() {
        let mut pt = PageTables::new();
        let mut ram = Ram::new();
        let mut none = Frames::<4>::new(); // no frames added
        assert_eq!(
            pt.map_small(&mut ram, &mut none, 0, phys(1), 0, RW),
            Err(MapError::NoMemory)
        );
    }
}
