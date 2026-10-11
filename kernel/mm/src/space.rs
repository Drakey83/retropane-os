//! Address spaces and demand paging (ADR-0003, ADR-0005).
//!
//! An [`AddressSpace`] is a bounded, sorted list of [`Region`]s inside one
//! window of the address space: a process slot, or the shared area for
//! memory-mapped files. Nothing is mapped when a region is created; pages
//! arrive on first touch through [`handle_fault`]:
//!
//! - [`Backing::Zero`]: a fresh zeroed frame.
//! - [`Backing::File`]: the frame from the unified page cache ([`PageCache`]),
//!   mapped directly, so every process shares one copy (ADR-0005 rules 2-3).
//!   Private writable file data (a PE's `.data`) is mapped read-only first and
//!   copied on the first write.
//! - [`Backing::Phys`]: a fixed physical window (device memory, framebuffers).
//!
//! Whose frame a page holds is decided from the region and the mapping
//! itself, with no per-frame metadata: in a private file region a writable
//! mapping is the process's own copy and a read-only one is the cache's.
//!
//! Rules enforced here, all derived from the hardware:
//!
//! - **W^X:** a region is never both writable and executable, unless the
//!   space opts in for a legacy app ([`AddressSpace::allow_write_exec`]).
//!   ARMv5 has no execute-never bit, so this protects code from being
//!   written, not data from being executed.
//! - **Shared writable file mappings live only in the shared area**, at the
//!   same address in every process. Mapping one frame writable at two
//!   addresses would let VIVT caches hold two diverging dirty copies (CE5
//!   placed memory-mapped files globally for the same reason).
//! - **The kernel never writes user memory through a read-only mapping.**
//!   ARMv5's "user read-only" permission still lets privileged code write,
//!   which would corrupt a shared page-cache page; kernel writes to user
//!   memory go through [`prepare_user_write`] first.

use crate::arm::{Ap, Attrs, Cache};
use crate::layout;
use crate::pagetable::{FrameAlloc, MapError, Mmu, PageTables, PhysMem};

const PAGE: u32 = 4096;

/// Region permissions.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub struct Prot(u8);

impl Prot {
    /// No access (a reserved range).
    pub const NONE: Self = Self(0);
    /// Readable.
    pub const R: Self = Self(1);
    /// Readable and writable.
    pub const RW: Self = Self(1 | 2);
    /// Readable and executable.
    pub const RX: Self = Self(1 | 4);
    /// Everything (legacy apps only, see W^X).
    pub const RWX: Self = Self(1 | 2 | 4);

    /// Readable.
    #[must_use]
    pub const fn read(self) -> bool {
        self.0 & 1 != 0
    }
    /// Writable.
    #[must_use]
    pub const fn write(self) -> bool {
        self.0 & 2 != 0
    }
    /// Executable.
    #[must_use]
    pub const fn exec(self) -> bool {
        self.0 & 4 != 0
    }
}

/// What a region's pages hold.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub enum Backing {
    /// Zero-filled anonymous memory.
    Zero,
    /// A file (or any page-cache object) from byte `offset` (page aligned).
    File {
        /// The page-cache object.
        object: u32,
        /// Byte offset of the region's first page in the object.
        offset: u32,
        /// Writes go to the file (shared) rather than to a private copy.
        shared: bool,
    },
    /// A fixed physical window.
    Phys {
        /// Physical address of the region's first page.
        base: u32,
        /// Cache policy (device registers vs framebuffers).
        cache: Cache,
    },
}

/// A range of an address space.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub struct Region {
    /// First address (MVA, page aligned).
    pub start: u32,
    /// Length in bytes (page multiple, non-zero).
    pub len: u32,
    /// Permissions.
    pub prot: Prot,
    /// Contents.
    pub backing: Backing,
}

impl Region {
    fn end(&self) -> u64 {
        u64::from(self.start) + u64::from(self.len)
    }

    fn contains(&self, mva: u32) -> bool {
        mva >= self.start && u64::from(mva) < self.end()
    }

    /// The part of this region from `start` for `len` bytes (inside it).
    fn slice(&self, start: u32, len: u32) -> Region {
        let skip = start - self.start;
        let backing = match self.backing {
            Backing::File {
                object,
                offset,
                shared,
            } => Backing::File {
                object,
                offset: offset + skip,
                shared,
            },
            Backing::Phys { base, cache } => Backing::Phys {
                base: base + skip,
                cache,
            },
            Backing::Zero => Backing::Zero,
        };
        Region {
            start,
            len,
            prot: self.prot,
            backing,
        }
    }
}

/// Regions per address space (ADR-0005 rule 9: bounded). A WM app with a
/// dozen DLLs uses a few dozen.
pub const MAX_REGIONS: usize = 64;

/// Why a region operation was refused.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub enum SpaceError {
    /// Not page aligned, empty, or outside the space's window.
    BadRange,
    /// Overlaps an existing region.
    Overlap,
    /// Writable and executable without the legacy opt-in.
    WriteExec,
    /// A shared writable file mapping outside the shared area.
    SharedWriteNotGlobal,
    /// [`MAX_REGIONS`] reached.
    TooManyRegions,
}

impl SpaceError {
    /// A short name for logs.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            SpaceError::BadRange => "bad range",
            SpaceError::Overlap => "overlap",
            SpaceError::WriteExec => "write+exec",
            SpaceError::SharedWriteNotGlobal => "shared write outside shared area",
            SpaceError::TooManyRegions => "too many regions",
        }
    }
}

/// One address space: a window of MVAs and its regions.
pub struct AddressSpace {
    lo: u32,
    hi: u32,
    domain: u32,
    allow_wx: bool,
    regions: [Option<Region>; MAX_REGIONS],
    count: usize,
}

impl AddressSpace {
    /// The space of the process in `slot` (its 32 MiB, as MVAs), using ARM
    /// `domain` for its page tables.
    #[must_use]
    pub const fn process(slot: u32, domain: u32) -> Self {
        let lo = layout::slot_base(slot);
        Self::window(lo, lo + layout::SLOT_SIZE, domain)
    }

    /// The shared area for memory-mapped files.
    #[must_use]
    pub const fn shared(domain: u32) -> Self {
        Self::window(layout::SHARED_BASE, layout::KERNEL_BASE, domain)
    }

    const fn window(lo: u32, hi: u32, domain: u32) -> Self {
        Self {
            lo,
            hi,
            domain,
            allow_wx: false,
            regions: [None; MAX_REGIONS],
            count: 0,
        }
    }

    /// Lets this space have writable, executable regions: for legacy apps
    /// that generate code (emulators with dynamic recompilers, for example).
    pub fn allow_write_exec(&mut self) {
        self.allow_wx = true;
    }

    /// Moves this space to another ARM domain (it gained or lost one, see
    /// [`crate::domains`]). New page tables use it; existing ones are moved
    /// with [`PageTables::retag`](crate::pagetable::PageTables::retag).
    pub fn set_domain(&mut self, domain: u32) {
        self.domain = domain;
    }

    /// The window this space covers (start, end).
    #[must_use]
    pub const fn bounds(&self) -> (u32, u32) {
        (self.lo, self.hi)
    }

    /// The ARM domain of this space's page tables.
    #[must_use]
    pub const fn domain(&self) -> u32 {
        self.domain
    }

    fn is_shared_area(&self) -> bool {
        self.lo >= layout::SHARED_BASE
    }

    /// The regions, in address order.
    pub fn regions(&self) -> impl Iterator<Item = &Region> {
        self.regions[..self.count].iter().flatten()
    }

    /// The region containing `mva`.
    #[must_use]
    pub fn find(&self, mva: u32) -> Option<&Region> {
        self.regions().find(|r| r.contains(mva))
    }

    fn check_range(&self, start: u32, len: u32) -> Result<(), SpaceError> {
        let end = u64::from(start) + u64::from(len);
        if len == 0
            || !start.is_multiple_of(PAGE)
            || !len.is_multiple_of(PAGE)
            || start < self.lo
            || end > u64::from(self.hi)
        {
            return Err(SpaceError::BadRange);
        }
        Ok(())
    }

    fn check_policy(&self, prot: Prot, backing: Backing) -> Result<(), SpaceError> {
        if prot.write() && prot.exec() && !self.allow_wx {
            return Err(SpaceError::WriteExec);
        }
        if let Backing::File { shared: true, .. } = backing
            && prot.write()
            && !self.is_shared_area()
        {
            return Err(SpaceError::SharedWriteNotGlobal);
        }
        Ok(())
    }

    /// Adds a region. Nothing is mapped until it's touched.
    pub fn map(&mut self, r: Region) -> Result<(), SpaceError> {
        self.check_range(r.start, r.len)?;
        self.check_policy(r.prot, r.backing)?;
        if self
            .regions()
            .any(|o| u64::from(o.start) < r.end() && u64::from(r.start) < o.end())
        {
            return Err(SpaceError::Overlap);
        }
        self.insert(r)
    }

    fn insert(&mut self, r: Region) -> Result<(), SpaceError> {
        if self.count == MAX_REGIONS {
            return Err(SpaceError::TooManyRegions);
        }
        let at = self.regions[..self.count]
            .iter()
            .position(|o| o.is_some_and(|o| o.start > r.start))
            .unwrap_or(self.count);
        self.regions.copy_within(at..self.count, at + 1);
        self.regions[at] = Some(r);
        self.count += 1;
        Ok(())
    }

    fn remove_at(&mut self, i: usize) -> Region {
        let r = self.regions[i].take();
        self.regions.copy_within(i + 1..self.count, i);
        self.count -= 1;
        self.regions[self.count] = None;
        r.unwrap_or_else(|| unreachable!("regions are packed"))
    }

    /// Removes `start..start + len` from the regions (splitting at the
    /// edges) and calls `each(piece)` for every removed piece, so the caller
    /// can unmap its pages. Atomic: if a split would exceed
    /// [`MAX_REGIONS`], nothing changes.
    pub fn carve(
        &mut self,
        start: u32,
        len: u32,
        mut each: impl FnMut(&Region),
    ) -> Result<(), SpaceError> {
        self.check_range(start, len)?;
        let end = u64::from(start) + u64::from(len);
        // Punching a hole in the middle of one region needs one extra slot.
        let splits = self
            .regions()
            .filter(|r| r.start < start && r.end() > end)
            .count();
        if self.count + splits > MAX_REGIONS {
            return Err(SpaceError::TooManyRegions);
        }
        let mut i = 0;
        while i < self.count {
            let r = self.regions[i].unwrap_or_else(|| unreachable!());
            if r.end() <= u64::from(start) || u64::from(r.start) >= end {
                i += 1;
                continue;
            }
            let cut_lo = r.start.max(start);
            let cut_hi = r.end().min(end) as u32;
            each(&r.slice(cut_lo, cut_hi - cut_lo));
            self.remove_at(i);
            if r.start < cut_lo {
                self.insert(r.slice(r.start, cut_lo - r.start))?;
                i += 1;
            }
            if u64::from(cut_hi) < r.end() {
                self.insert(r.slice(cut_hi, (r.end() - u64::from(cut_hi)) as u32))?;
                i += 1;
            }
        }
        Ok(())
    }

    /// Changes the permissions of `start..start + len`, which must be fully
    /// covered by regions.
    pub fn protect(&mut self, start: u32, len: u32, prot: Prot) -> Result<(), SpaceError> {
        self.check_range(start, len)?;
        let mut pieces = [None; MAX_REGIONS];
        let mut n = 0;
        let mut covered = 0u64;
        for r in self.regions() {
            if r.end() > u64::from(start) && u64::from(r.start) < u64::from(start) + u64::from(len)
            {
                let lo = r.start.max(start);
                let hi = r.end().min(u64::from(start) + u64::from(len)) as u32;
                self.check_policy(prot, r.backing)?;
                covered += u64::from(hi - lo);
                if n < pieces.len() {
                    pieces[n] = Some(r.slice(lo, hi - lo));
                    n += 1;
                }
            }
        }
        if covered != u64::from(len) {
            return Err(SpaceError::BadRange);
        }
        self.carve(start, len, |_| {})?;
        for p in pieces[..n].iter().flatten() {
            self.insert(Region { prot, ..*p })?;
        }
        Ok(())
    }
}

/// The unified page cache, as paging sees it.
pub trait PageCache {
    /// The frame (physical address) holding page `page` (4 KiB units) of
    /// `object`, reading it in if needed. Takes a reference.
    fn get(&mut self, object: u32, page: u32) -> Result<u32, IoError>;
    /// Drops a reference taken by [`get`](Self::get).
    fn release(&mut self, phys: u32);
}

/// A page couldn't be read.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub struct IoError;

/// What kind of access faulted.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub enum Access {
    /// A data read.
    Read,
    /// A data write.
    Write,
    /// An instruction fetch.
    Exec,
}

/// The access to resolve for an abort, without decoding the faulting
/// instruction. ARMv5 doesn't say whether a data abort was a read or a write,
/// and decoding instructions (ARM and Thumb) is slow and fragile. Instead:
///
/// - a **translation** fault is resolved as a read. Zero pages are mapped
///   writable anyway if the region allows; a private file page is mapped
///   read-only, and if the access was a write it retries, takes a permission
///   fault, and is copied then;
/// - a **permission** fault on a readable page can only be a write (RetroPane
///   never maps a readable page without user read access);
/// - a prefetch abort is an instruction fetch;
/// - domain and other faults aren't paging faults: `None`.
///
/// Cost: at most one extra fault the first time a private file page is
/// written, instead of decoding every faulting instruction.
#[must_use]
pub fn access_for(kind: crate::arm::FaultKind, prefetch: bool) -> Option<Access> {
    use crate::arm::FaultKind;
    if prefetch {
        return Some(Access::Exec);
    }
    match kind {
        FaultKind::Translation => Some(Access::Read),
        FaultKind::Permission => Some(Access::Write),
        FaultKind::Domain | FaultKind::Other(_) => None,
    }
}

/// How a fault was resolved.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub enum Outcome {
    /// A page was mapped.
    Mapped,
    /// A private copy replaced a shared page-cache page.
    CopiedOnWrite,
    /// The page was already mapped with enough permission (another path
    /// resolved it first).
    Spurious,
}

/// Why a fault couldn't be resolved (the process gets an access violation,
/// or the kernel bug is reported).
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub enum FaultError {
    /// No region at that address.
    NoRegion,
    /// The region doesn't allow that access.
    Protection,
    /// No free frame.
    NoMemory,
    /// The backing page couldn't be read.
    Io,
    /// The page tables refused the mapping.
    Map(MapError),
}

impl FaultError {
    /// A short name for logs.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            FaultError::NoRegion => "no region",
            FaultError::Protection => "protection",
            FaultError::NoMemory => "no memory",
            FaultError::Io => "I/O error",
            FaultError::Map(e) => e.name(),
        }
    }
}

/// Everything paging works with, borrowed for one operation.
pub struct Vm<'a, M, F, U, C> {
    /// The page tables.
    pub tables: &'a mut PageTables,
    /// Physical memory.
    pub mem: &'a mut M,
    /// Frame allocator.
    pub frames: &'a mut F,
    /// Cache and TLB maintenance.
    pub mmu: &'a mut U,
    /// The page cache.
    pub cache: &'a mut C,
}

fn attrs_for(r: &Region, writable_now: bool) -> Attrs {
    let cache = match r.backing {
        Backing::Phys { cache, .. } => cache,
        _ => Cache::WriteBack,
    };
    let ap = if !r.prot.read() {
        Ap::None
    } else if writable_now {
        Ap::UserWrite
    } else {
        Ap::UserRead
    };
    Attrs { ap, cache }
}

impl<M: PhysMem, F: FrameAlloc, U: Mmu, C: PageCache> Vm<'_, M, F, U, C> {
    /// Resolves a fault at `mva` in `space`.
    pub fn handle_fault(
        &mut self,
        space: &AddressSpace,
        mva: u32,
        access: Access,
    ) -> Result<Outcome, FaultError> {
        let r = *space.find(mva).ok_or(FaultError::NoRegion)?;
        let allowed = match access {
            Access::Read => r.prot.read(),
            Access::Write => r.prot.write(),
            Access::Exec => r.prot.exec(),
        };
        if !allowed {
            return Err(FaultError::Protection);
        }
        let page = mva & !(PAGE - 1);
        let private_file = matches!(r.backing, Backing::File { shared: false, .. });

        if let Some((phys, attrs)) = self.tables.translate(self.mem, page) {
            if access != Access::Write || attrs.ap.user_writable() {
                return Ok(Outcome::Spurious);
            }
            if private_file && r.prot.write() {
                return self.copy_on_write(space, &r, page, phys);
            }
            return Err(FaultError::Protection);
        }

        let (phys, writable) = match r.backing {
            Backing::Zero => {
                let f = self.frames.alloc(0).ok_or(FaultError::NoMemory)?;
                let phys = self.mem.frame_phys(f);
                self.mem.zero(phys);
                (phys, r.prot.write())
            }
            Backing::File {
                object,
                offset,
                shared,
            } => {
                let index = (offset + (page - r.start)) / PAGE;
                let phys = self.cache.get(object, index).map_err(|_| FaultError::Io)?;
                // A private page starts as the cache's (read-only) copy, and
                // a write fault copies it, unless the very first access is a
                // write: then copy straight away.
                if !shared && r.prot.write() && access == Access::Write {
                    let f = match self.frames.alloc(0) {
                        Some(f) => f,
                        None => {
                            self.cache.release(phys);
                            return Err(FaultError::NoMemory);
                        }
                    };
                    let copy = self.mem.frame_phys(f);
                    self.mem.copy(copy, phys);
                    self.cache.release(phys);
                    (copy, true)
                } else {
                    (phys, shared && r.prot.write())
                }
            }
            Backing::Phys { base, .. } => (base + (page - r.start), r.prot.write()),
        };
        let attrs = attrs_for(&r, writable);
        if let Err(e) =
            self.tables
                .map_small(self.mem, self.frames, page, phys, space.domain(), attrs)
        {
            let base = self.mem.frame_phys(0);
            release_page(&r, phys, writable, base, self.frames, self.cache);
            return Err(FaultError::Map(e));
        }
        Ok(Outcome::Mapped)
    }

    fn copy_on_write(
        &mut self,
        space: &AddressSpace,
        r: &Region,
        page: u32,
        shared_phys: u32,
    ) -> Result<Outcome, FaultError> {
        let f = self.frames.alloc(0).ok_or(FaultError::NoMemory)?;
        let copy = self.mem.frame_phys(f);
        // The shared page is mapped read-only, so the cache holds no dirty
        // lines for it: copying through the direct map is safe.
        self.mem.copy(copy, shared_phys);
        let cache = &mut *self.cache;
        self.tables
            .unmap(self.mem, self.frames, self.mmu, page, PAGE, |u, _| {
                cache.release(u.phys)
            });
        self.tables
            .map_small(
                self.mem,
                self.frames,
                page,
                copy,
                space.domain(),
                attrs_for(r, true),
            )
            .map_err(FaultError::Map)?;
        Ok(Outcome::CopiedOnWrite)
    }

    /// Removes `start..start + len` from `space`, unmapping its pages and
    /// returning their frames.
    pub fn unmap(
        &mut self,
        space: &mut AddressSpace,
        start: u32,
        len: u32,
    ) -> Result<(), SpaceError> {
        let mut pieces = [None; MAX_REGIONS + 1];
        let mut n = 0;
        space.carve(start, len, |p| {
            if n < pieces.len() {
                pieces[n] = Some(*p);
                n += 1;
            }
        })?;
        let base = self.mem.frame_phys(0);
        for p in pieces[..n].iter().flatten() {
            let cache = &mut *self.cache;
            self.tables.unmap(
                self.mem,
                self.frames,
                self.mmu,
                p.start,
                p.len,
                |u, frames| {
                    release_page(p, u.phys, u.attrs.ap.user_writable(), base, frames, cache);
                },
            );
        }
        Ok(())
    }

    /// Changes permissions of `start..start + len` in `space` and of any
    /// pages already mapped there. Pages that become writable keep their
    /// current mapping read-only where copy-on-write must still happen.
    pub fn protect(
        &mut self,
        space: &mut AddressSpace,
        start: u32,
        len: u32,
        prot: Prot,
    ) -> Result<(), SpaceError> {
        space.protect(start, len, prot)?;
        // Downgrades take effect now; upgrades happen lazily through faults,
        // which keeps copy-on-write correct with no extra bookkeeping. Only
        // permissions change: each page keeps its cache policy.
        if !prot.write() || !prot.read() {
            let ap = if prot.read() { Ap::UserRead } else { Ap::None };
            self.tables.protect(self.mem, self.mmu, start, len, ap);
        }
        Ok(())
    }

    /// Makes `start..start + len` safe for the kernel to write on behalf of
    /// the process: every page is present and privately writable (copying on
    /// write where needed). Required because ARMv5 lets privileged code write
    /// pages that are read-only for the user.
    pub fn prepare_user_write(
        &mut self,
        space: &AddressSpace,
        start: u32,
        len: u32,
    ) -> Result<(), FaultError> {
        if len == 0 {
            return Ok(());
        }
        let last = start.checked_add(len - 1).ok_or(FaultError::NoRegion)?;
        let mut page = start & !(PAGE - 1);
        loop {
            self.handle_fault(space, page, Access::Write)?;
            if page >= (last & !(PAGE - 1)) {
                return Ok(());
            }
            page += PAGE;
        }
    }
}

/// Gives back the frame of a page leaving region `r`. `base` is the physical
/// address of frame 0 (frames are linear in physical memory).
fn release_page<F: FrameAlloc, C: PageCache>(
    r: &Region,
    phys: u32,
    writable: bool,
    base: u32,
    frames: &mut F,
    cache: &mut C,
) {
    let frame = ((phys - base) / PAGE) as usize;
    match r.backing {
        Backing::Zero => frames.free(frame),
        Backing::File { shared: true, .. } => cache.release(phys),
        // Private: a writable mapping is the process's copy, a read-only one
        // the cache's.
        Backing::File { shared: false, .. } => {
            if writable {
                frames.free(frame);
            } else {
                cache.release(phys);
            }
        }
        Backing::Phys { .. } => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frames::Frames;
    use crate::pagetable::fake::{Log, Ram};
    use std::collections::HashMap;

    /// Page-cache frames live above the allocator's pool.
    const CACHE_PHYS: u32 = layout::RAM_PHYS + 0x0100_0000;

    #[derive(Default)]
    struct Cache {
        slots: HashMap<(u32, u32), u32>,
        refs: HashMap<u32, i32>,
        fail: bool,
    }

    impl Cache {
        fn total_refs(&self) -> i32 {
            self.refs.values().sum()
        }
    }

    impl PageCache for Cache {
        fn get(&mut self, object: u32, page: u32) -> Result<u32, IoError> {
            if self.fail {
                return Err(IoError);
            }
            let n = self.slots.len() as u32;
            let phys = *self
                .slots
                .entry((object, page))
                .or_insert(CACHE_PHYS + n * 4096);
            *self.refs.entry(phys).or_default() += 1;
            Ok(phys)
        }
        fn release(&mut self, phys: u32) {
            let r = self
                .refs
                .get_mut(&phys)
                .expect("released a page never taken");
            *r -= 1;
            assert!(*r >= 0, "page released twice");
        }
    }

    struct World {
        tables: PageTables,
        ram: Ram,
        frames: Frames<512>,
        mmu: Log,
        cache: Cache,
    }

    impl World {
        fn new() -> Self {
            let mut frames = Frames::<512>::new();
            frames.add_range(0, 512).unwrap();
            Self {
                tables: PageTables::new(),
                ram: Ram::new(),
                frames,
                mmu: Log::default(),
                cache: Cache::default(),
            }
        }

        fn vm(&mut self) -> Vm<'_, Ram, Frames<512>, Log, Cache> {
            Vm {
                tables: &mut self.tables,
                mem: &mut self.ram,
                frames: &mut self.frames,
                mmu: &mut self.mmu,
                cache: &mut self.cache,
            }
        }

        /// First word of the page `mva` maps to, if any.
        fn peek(&mut self, mva: u32) -> Option<u32> {
            let (phys, _) = self.tables.translate(&mut self.ram, mva)?;
            Some(self.ram.page(phys)[0])
        }
    }

    const SLOT: u32 = 5;
    fn base() -> u32 {
        layout::slot_base(SLOT)
    }
    fn zero(start: u32, len: u32, prot: Prot) -> Region {
        Region {
            start,
            len,
            prot,
            backing: Backing::Zero,
        }
    }
    fn file(start: u32, len: u32, prot: Prot, shared: bool) -> Region {
        Region {
            start,
            len,
            prot,
            backing: Backing::File {
                object: 9,
                offset: 0x3000,
                shared,
            },
        }
    }

    #[test]
    fn aborts_map_to_accesses_without_decoding() {
        use crate::arm::FaultKind;
        assert_eq!(
            access_for(FaultKind::Translation, false),
            Some(Access::Read)
        );
        assert_eq!(
            access_for(FaultKind::Permission, false),
            Some(Access::Write)
        );
        assert_eq!(access_for(FaultKind::Domain, false), None);
        assert_eq!(access_for(FaultKind::Other(1), false), None);
        assert_eq!(access_for(FaultKind::Other(0), true), Some(Access::Exec));
    }

    #[test]
    fn a_write_resolved_as_read_then_permission_still_copies() {
        // The two-step path `access_for` relies on: a write to an unmapped
        // private file page first resolves as a read (read-only cache page),
        // the retried write takes a permission fault, and that copies.
        let mut w = World::new();
        let mut s = AddressSpace::process(SLOT, 2);
        let a = base() + 0x10000;
        s.map(file(a, 0x1000, Prot::RW, false)).unwrap();
        let first = access_for(crate::arm::FaultKind::Translation, false).unwrap();
        assert_eq!(w.vm().handle_fault(&s, a, first), Ok(Outcome::Mapped));
        let second = access_for(crate::arm::FaultKind::Permission, false).unwrap();
        assert_eq!(
            w.vm().handle_fault(&s, a, second),
            Ok(Outcome::CopiedOnWrite)
        );
        // And a write to an unmapped page of a read-only region ends as a
        // protection error, after the same two steps.
        s.map(zero(a + 0x1000, 0x1000, Prot::R)).unwrap();
        assert_eq!(
            w.vm().handle_fault(&s, a + 0x1000, first),
            Ok(Outcome::Mapped)
        );
        assert_eq!(
            w.vm().handle_fault(&s, a + 0x1000, second),
            Err(FaultError::Protection)
        );
    }

    #[test]
    fn policy_w_x_and_shared_writes() {
        let mut s = AddressSpace::process(SLOT, 2);
        assert_eq!(
            s.map(zero(base() + 0x10000, 0x1000, Prot::RWX)),
            Err(SpaceError::WriteExec)
        );
        assert_eq!(
            s.map(file(base() + 0x10000, 0x1000, Prot::RW, true)),
            Err(SpaceError::SharedWriteNotGlobal)
        );
        // Read-only shared file mappings are fine anywhere.
        s.map(file(base() + 0x10000, 0x1000, Prot::R, true))
            .unwrap();
        // Legacy opt-in.
        s.allow_write_exec();
        s.map(zero(base() + 0x20000, 0x1000, Prot::RWX)).unwrap();
        // The shared area accepts shared writable mappings.
        let mut g = AddressSpace::shared(1);
        g.map(file(layout::SHARED_BASE, 0x2000, Prot::RW, true))
            .unwrap();
    }

    #[test]
    fn ranges_must_fit_the_window_and_not_overlap() {
        let mut s = AddressSpace::process(SLOT, 2);
        assert_eq!(
            s.map(zero(base() + 1, 0x1000, Prot::RW)),
            Err(SpaceError::BadRange)
        );
        assert_eq!(s.map(zero(base(), 0, Prot::RW)), Err(SpaceError::BadRange));
        assert_eq!(
            s.map(zero(base() + layout::SLOT_SIZE - 0x1000, 0x2000, Prot::RW)),
            Err(SpaceError::BadRange)
        );
        assert_eq!(
            s.map(zero(base() - 0x1000, 0x1000, Prot::RW)),
            Err(SpaceError::BadRange)
        );
        s.map(zero(base() + 0x10000, 0x4000, Prot::RW)).unwrap();
        assert_eq!(
            s.map(zero(base() + 0x13000, 0x1000, Prot::RW)),
            Err(SpaceError::Overlap)
        );
        s.map(zero(base() + 0x14000, 0x1000, Prot::RW)).unwrap();
        let starts: Vec<_> = s.regions().map(|r| r.start).collect();
        assert_eq!(starts, [base() + 0x10000, base() + 0x14000]);
    }

    #[test]
    fn carve_splits_and_keeps_file_offsets() {
        let mut s = AddressSpace::process(SLOT, 2);
        s.map(file(base() + 0x10000, 0x4000, Prot::R, false))
            .unwrap();
        let mut cut = Vec::new();
        s.carve(base() + 0x11000, 0x2000, |r| cut.push(*r)).unwrap();
        assert_eq!(cut.len(), 1);
        assert!(matches!(
            cut[0].backing,
            Backing::File { offset: 0x4000, .. }
        ));
        let left: Vec<_> = s.regions().copied().collect();
        assert_eq!(left.len(), 2);
        assert_eq!((left[0].start, left[0].len), (base() + 0x10000, 0x1000));
        assert_eq!((left[1].start, left[1].len), (base() + 0x13000, 0x1000));
        assert!(matches!(
            left[1].backing,
            Backing::File { offset: 0x6000, .. }
        ));
    }

    #[test]
    fn carve_is_atomic_when_full() {
        let mut s = AddressSpace::process(SLOT, 2);
        for i in 0..MAX_REGIONS as u32 {
            s.map(zero(base() + 0x10000 + i * 0x4000, 0x3000, Prot::RW))
                .unwrap();
        }
        // Punching a hole would need one region too many.
        assert_eq!(
            s.carve(base() + 0x11000, 0x1000, |_| panic!("must not cut")),
            Err(SpaceError::TooManyRegions)
        );
        assert_eq!(s.regions().count(), MAX_REGIONS);
    }

    #[test]
    fn protect_splits_regions() {
        let mut s = AddressSpace::process(SLOT, 2);
        s.map(zero(base() + 0x10000, 0x3000, Prot::RW)).unwrap();
        s.protect(base() + 0x11000, 0x1000, Prot::R).unwrap();
        let p: Vec<_> = s.regions().map(|r| (r.start - base(), r.prot)).collect();
        assert_eq!(
            p,
            [(0x10000, Prot::RW), (0x11000, Prot::R), (0x12000, Prot::RW)]
        );
        assert_eq!(
            s.protect(base() + 0x12000, 0x2000, Prot::R),
            Err(SpaceError::BadRange)
        );
        assert_eq!(
            s.protect(base() + 0x10000, 0x1000, Prot::RWX),
            Err(SpaceError::WriteExec)
        );
    }

    #[test]
    fn zero_pages_arrive_on_first_touch() {
        let mut w = World::new();
        let mut s = AddressSpace::process(SLOT, 2);
        let a = base() + 0x10000;
        s.map(zero(a, 0x2000, Prot::RW)).unwrap();
        assert_eq!(w.peek(a), None, "nothing mapped up front");
        assert_eq!(
            w.vm().handle_fault(&s, a + 0x123, Access::Write),
            Ok(Outcome::Mapped)
        );
        assert_eq!(w.peek(a), Some(0), "zeroed");
        let (_, attrs) = w.tables.translate(&mut w.ram, a).unwrap();
        assert_eq!(attrs.ap, Ap::UserWrite);
        assert_eq!(
            w.vm().handle_fault(&s, a, Access::Read),
            Ok(Outcome::Spurious)
        );
    }

    #[test]
    fn protection_and_missing_regions() {
        let mut w = World::new();
        let mut s = AddressSpace::process(SLOT, 2);
        let a = base() + 0x10000;
        s.map(zero(a, 0x1000, Prot::R)).unwrap();
        assert_eq!(
            w.vm().handle_fault(&s, a, Access::Write),
            Err(FaultError::Protection)
        );
        assert_eq!(
            w.vm().handle_fault(&s, a, Access::Exec),
            Err(FaultError::Protection)
        );
        assert_eq!(
            w.vm().handle_fault(&s, a + 0x1000, Access::Read),
            Err(FaultError::NoRegion)
        );
        // A read-only zero page, then a write: still a protection fault.
        w.vm().handle_fault(&s, a, Access::Read).unwrap();
        assert_eq!(
            w.vm().handle_fault(&s, a, Access::Write),
            Err(FaultError::Protection)
        );
    }

    #[test]
    fn shared_file_pages_are_one_copy_for_everyone() {
        let mut w = World::new();
        let mut p1 = AddressSpace::process(5, 2);
        let mut p2 = AddressSpace::process(6, 3);
        let (b1, b2) = (layout::slot_base(5), layout::slot_base(6));
        p1.map(file(b1 + 0x10000, 0x1000, Prot::RX, false)).unwrap();
        p2.map(file(b2 + 0x40000, 0x1000, Prot::RX, false)).unwrap();
        w.vm()
            .handle_fault(&p1, b1 + 0x10000, Access::Exec)
            .unwrap();
        w.vm()
            .handle_fault(&p2, b2 + 0x40000, Access::Read)
            .unwrap();
        let a = w.tables.translate(&mut w.ram, b1 + 0x10000).unwrap();
        let b = w.tables.translate(&mut w.ram, b2 + 0x40000).unwrap();
        assert_eq!(a.0, b.0, "same physical page");
        assert_eq!(a.1.ap, Ap::UserRead);
        assert_eq!(
            w.cache.slots.keys().next(),
            Some(&(9, 3)),
            "offset 0x3000 = page 3"
        );
        // Only page-table frames came from the allocator (two megabytes, one
        // slab frame).
        assert_eq!(w.frames.free_frames(), 512 - 1);
    }

    #[test]
    fn private_file_pages_copy_on_write() {
        let mut w = World::new();
        let mut s = AddressSpace::process(SLOT, 2);
        let a = base() + 0x10000;
        s.map(file(a, 0x1000, Prot::RW, false)).unwrap();
        w.vm().handle_fault(&s, a, Access::Read).unwrap();
        let (shared, attrs) = w.tables.translate(&mut w.ram, a).unwrap();
        assert!(shared >= CACHE_PHYS);
        assert_eq!(attrs.ap, Ap::UserRead, "read-only until written");
        w.ram.page(shared)[0] = 0x1234_5678;
        w.mmu.0.clear();
        assert_eq!(
            w.vm().handle_fault(&s, a, Access::Write),
            Ok(Outcome::CopiedOnWrite)
        );
        let (copy, attrs) = w.tables.translate(&mut w.ram, a).unwrap();
        assert_ne!(copy, shared);
        assert_eq!(attrs.ap, Ap::UserWrite);
        assert_eq!(w.peek(a), Some(0x1234_5678), "contents copied");
        assert_eq!(w.cache.total_refs(), 0, "cache page released");
        // The old mapping was cleaned and invalidated before the new one.
        assert_eq!(w.mmu.0, [format!("clean {a:#x}"), format!("tlb {a:#x}")]);
    }

    #[test]
    fn a_first_write_copies_straight_away() {
        let mut w = World::new();
        let mut s = AddressSpace::process(SLOT, 2);
        let a = base() + 0x10000;
        s.map(file(a, 0x1000, Prot::RW, false)).unwrap();
        assert_eq!(
            w.vm().handle_fault(&s, a, Access::Write),
            Ok(Outcome::Mapped)
        );
        let (phys, attrs) = w.tables.translate(&mut w.ram, a).unwrap();
        assert!(phys < CACHE_PHYS, "a private frame, not the cache's");
        assert_eq!(attrs.ap, Ap::UserWrite);
        assert_eq!(w.cache.total_refs(), 0);
    }

    #[test]
    fn unmap_returns_every_frame_and_reference() {
        let mut w = World::new();
        let mut s = AddressSpace::process(SLOT, 2);
        let a = base() + 0x10000;
        s.map(zero(a, 0x3000, Prot::RW)).unwrap();
        s.map(file(a + 0x3000, 0x2000, Prot::RW, false)).unwrap();
        for off in [0, 0x1000, 0x2000] {
            w.vm().handle_fault(&s, a + off, Access::Write).unwrap();
        }
        w.vm().handle_fault(&s, a + 0x3000, Access::Read).unwrap(); // cache page
        w.vm().handle_fault(&s, a + 0x4000, Access::Write).unwrap(); // private copy
        assert!(w.frames.free_frames() < 512);
        w.vm().unmap(&mut s, a, 0x5000).unwrap();
        assert_eq!(
            w.frames.free_frames(),
            512,
            "zero pages, copy and table freed"
        );
        assert_eq!(w.cache.total_refs(), 0, "cache reference dropped");
        assert_eq!(s.regions().count(), 0);
    }

    #[test]
    fn large_unmaps_never_lose_pages() {
        let mut w = World::new();
        let mut s = AddressSpace::process(SLOT, 2);
        let a = base() + 0x10_0000;
        let pages = 300u32; // more than any fixed batch
        s.map(zero(a, pages * 4096, Prot::RW)).unwrap();
        for i in 0..pages {
            w.vm()
                .handle_fault(&s, a + i * 4096, Access::Write)
                .unwrap();
        }
        w.vm().unmap(&mut s, a, pages * 4096).unwrap();
        assert_eq!(w.frames.free_frames(), 512);
    }

    #[test]
    fn prepare_user_write_breaks_sharing_first() {
        let mut w = World::new();
        let mut s = AddressSpace::process(SLOT, 2);
        let a = base() + 0x10000;
        s.map(file(a, 0x2000, Prot::RW, false)).unwrap();
        w.vm().handle_fault(&s, a, Access::Read).unwrap();
        w.vm().prepare_user_write(&s, a + 0x800, 0x1000).unwrap();
        for off in [0, 0x1000] {
            let (phys, attrs) = w.tables.translate(&mut w.ram, a + off).unwrap();
            assert!(phys < CACHE_PHYS && attrs.ap == Ap::UserWrite);
        }
        assert_eq!(w.cache.total_refs(), 0);
        // Read-only memory can't be written by the kernel on the user's behalf.
        s.map(zero(a + 0x2000, 0x1000, Prot::R)).unwrap();
        assert_eq!(
            w.vm().prepare_user_write(&s, a + 0x2000, 4),
            Err(FaultError::Protection)
        );
    }

    #[test]
    fn protect_downgrades_mapped_pages_now() {
        let mut w = World::new();
        let mut s = AddressSpace::process(SLOT, 2);
        let a = base() + 0x10000;
        s.map(zero(a, 0x1000, Prot::RW)).unwrap();
        w.vm().handle_fault(&s, a, Access::Write).unwrap();
        w.mmu.0.clear();
        w.vm().protect(&mut s, a, 0x1000, Prot::R).unwrap();
        assert_eq!(
            w.tables.translate(&mut w.ram, a).unwrap().1.ap,
            Ap::UserRead
        );
        assert_eq!(w.mmu.0, [format!("tlb {a:#x}")]);
        assert_eq!(
            w.vm().handle_fault(&s, a, Access::Write),
            Err(FaultError::Protection)
        );
    }

    #[test]
    fn failures_leak_nothing() {
        let mut w = World::new();
        let mut s = AddressSpace::process(SLOT, 2);
        let a = base() + 0x10000;
        s.map(file(a, 0x1000, Prot::R, false)).unwrap();
        w.cache.fail = true;
        assert_eq!(
            w.vm().handle_fault(&s, a, Access::Read),
            Err(FaultError::Io)
        );
        // Out of frames: the zero page can't be allocated.
        let mut empty = Frames::<512>::new();
        let mut s2 = AddressSpace::process(6, 3);
        s2.map(zero(layout::slot_base(6), 0x1000, Prot::RW))
            .unwrap();
        let mut vm = Vm {
            tables: &mut w.tables,
            mem: &mut w.ram,
            frames: &mut empty,
            mmu: &mut w.mmu,
            cache: &mut w.cache,
        };
        assert_eq!(
            vm.handle_fault(&s2, layout::slot_base(6), Access::Write),
            Err(FaultError::NoMemory)
        );
        assert_eq!(w.frames.free_frames(), 512);
    }
}
