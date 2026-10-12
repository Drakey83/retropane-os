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
use crate::zpool::{Handle, Pages, ZBuffers, ZPool};

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

    /// The same permissions, *pinned*: the region's pages are loaded when
    /// [`Vm::populate`] asks and then never aged, compressed or evicted
    /// (the input path, the compositor's working set). Pinned memory
    /// counts against the space's pin limit.
    #[must_use]
    pub const fn pinned(self) -> Self {
        Self(self.0 | 8)
    }

    /// Whether pinned.
    #[must_use]
    pub const fn is_pinned(self) -> bool {
        self.0 & 8 != 0
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

/// Pinned memory a space may hold unless the kernel raises its limit (a
/// trusted service such as the compositor): an app can't pin the device
/// full.
pub const DEFAULT_PIN_LIMIT: u32 = 64 * 1024;

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
    /// Pinning this would exceed the space's pin limit.
    PinLimit,
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
            SpaceError::PinLimit => "pin limit",
        }
    }
}

/// One address space: a window of MVAs and its regions.
pub struct AddressSpace {
    lo: u32,
    hi: u32,
    domain: u32,
    allow_wx: bool,
    pin_limit: u32,
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
            pin_limit: DEFAULT_PIN_LIMIT,
            regions: [None; MAX_REGIONS],
            count: 0,
        }
    }

    /// Reuses this record for the process in `slot` (a new process): no
    /// regions, W^X enforced, ARM `domain`. In place, so a record never
    /// passes through a (small) kernel stack. The caller has already
    /// unmapped everything the previous process had.
    pub fn reset_process(&mut self, slot: u32, domain: u32) {
        self.lo = layout::slot_base(slot);
        self.hi = self.lo + layout::SLOT_SIZE;
        self.domain = domain;
        self.allow_wx = false;
        self.pin_limit = DEFAULT_PIN_LIMIT;
        self.regions[..self.count].fill(None);
        self.count = 0;
    }

    /// Lets this space have writable, executable regions: for legacy apps
    /// that generate code (emulators with dynamic recompilers, for example).
    pub fn allow_write_exec(&mut self) {
        self.allow_wx = true;
    }

    /// Sets how much memory this space may pin (the kernel's decision, for
    /// trusted services; apps keep [`DEFAULT_PIN_LIMIT`]).
    pub fn set_pin_limit(&mut self, bytes: u32) {
        self.pin_limit = bytes;
    }

    /// Bytes of pinned regions.
    #[must_use]
    pub fn pinned_bytes(&self) -> u64 {
        self.regions()
            .filter(|r| r.prot.is_pinned())
            .map(|r| u64::from(r.len))
            .sum()
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
        if r.prot.is_pinned() && self.pinned_bytes() + u64::from(r.len) > u64::from(self.pin_limit)
        {
            return Err(SpaceError::PinLimit);
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
    /// covered by regions. Pinned pieces stay pinned; `prot` may pin more
    /// (within the pin limit).
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
        if prot.is_pinned() {
            let newly: u64 = pieces[..n]
                .iter()
                .flatten()
                .filter(|p| !p.prot.is_pinned())
                .map(|p| u64::from(p.len))
                .sum();
            if self.pinned_bytes() + newly > u64::from(self.pin_limit) {
                return Err(SpaceError::PinLimit);
            }
        }
        self.carve(start, len, |_| {})?;
        for p in pieces[..n].iter().flatten() {
            let prot = if p.prot.is_pinned() {
                prot.pinned()
            } else {
                prot
            };
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
    /// A compressed page was decompressed back into place.
    Restored,
    /// An aged page was touched: mapped again as it was (a minor fault:
    /// the reclaim clock's "referenced" bit, which ARMv5 lacks).
    Referenced,
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
    /// Compressed copies of idle pages (ADR-0005 rule 6).
    pub zpool: &'a mut ZPool,
    /// Buffers for compressing and decompressing (static, not on the stack).
    pub zbuf: &'a mut ZBuffers,
}

/// The compressed page store's frames, reached through the kernel's
/// physical-memory access (they're kernel-private).
struct ZPages<'m, M>(&'m mut M);

impl<M: PhysMem> Pages for ZPages<'_, M> {
    fn page(&mut self, f: usize) -> &mut [u8; 4096] {
        let phys = self.0.frame_phys(f);
        self.0.kernel_page(phys)
    }
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

        if let Some(raw) = self.tables.stored(self.mem, page) {
            if let Some((f, writable)) = decode_aged(raw) {
                // Writable only if it was and the region still allows it.
                let attrs = attrs_for(&r, writable && r.prot.write());
                let phys = self.mem.frame_phys(f);
                self.tables
                    .restore(self.mem, page, phys, attrs)
                    .map_err(FaultError::Map)?;
                return Ok(Outcome::Referenced);
            }
            return self.restore(&r, page, raw);
        }

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

    /// Decompresses the page stored under `raw` back into a fresh frame.
    fn restore(&mut self, r: &Region, page: u32, raw: u32) -> Result<Outcome, FaultError> {
        let h = Handle::from_raw(raw);
        let f = self.frames.alloc(0).ok_or(FaultError::NoMemory)?;
        let phys = self.mem.frame_phys(f);
        // Copy the compressed bytes out of the store (the page and the store
        // can't both be borrowed), then decompress straight into the frame:
        // no page-sized buffer on the stack (this runs on small kernel and
        // abort stacks).
        let packed = &mut self.zbuf.packed;
        let len = {
            let mut store = ZPages(&mut *self.mem);
            let src = self.zpool.get(&mut store, h);
            packed[..src.len()].copy_from_slice(src);
            src.len()
        };
        let ok = self.mem.with_page(phys, |page| {
            rp_lz4::decompress(&packed[..len], page) == Ok(4096)
        });
        if !ok {
            // A corrupt compressed copy is a kernel bug: never map garbage.
            self.frames.free(f);
            return Err(FaultError::Io);
        }
        self.zpool.free(self.frames, &mut ZPages(&mut *self.mem), h);
        self.tables
            .restore(self.mem, page, phys, attrs_for(r, r.prot.write()))
            .map_err(FaultError::Map)?;
        Ok(Outcome::Restored)
    }

    /// Compresses the anonymous page mapped at `mva` into the compressed page
    /// store, frees its frame and leaves the store's handle in its page-table
    /// entry, so the next touch faults and [`handle_fault`](Self::handle_fault)
    /// brings it back. Returns whether it was compressed: pages that aren't
    /// mapped, aren't anonymous, or don't compress well enough stay as they
    /// are.
    pub fn compress(&mut self, space: &AddressSpace, mva: u32) -> Result<bool, FaultError> {
        let page = mva & !(PAGE - 1);
        let Some(r) = space.find(page) else {
            return Ok(false);
        };
        if r.backing != Backing::Zero || r.prot.is_pinned() {
            return Ok(false);
        }
        let Some((phys, _)) = self.tables.translate(self.mem, page) else {
            return Ok(false);
        };
        // The process may have dirty lines for it under its own address
        // (VIVT): write them back before reading the frame.
        self.mmu.clean_page(page);
        let Some(h) = self.pack(phys)? else {
            return Ok(false);
        };
        if self
            .tables
            .store(self.mem, self.mmu, page, h.raw())
            .is_none()
        {
            self.zpool.free(self.frames, &mut ZPages(&mut *self.mem), h);
            return Ok(false);
        }
        self.frames.free(self.mem.phys_frame(phys));
        Ok(true)
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

    /// Loads every page of `start..start + len` now (each as its first touch
    /// would: zero-filled, from the page cache, or decompressed; writable
    /// pages already private), so later touches never fault: for pinned
    /// regions on the input path. Returns how many pages it loaded.
    pub fn populate(
        &mut self,
        space: &AddressSpace,
        start: u32,
        len: u32,
    ) -> Result<u32, FaultError> {
        let mut loaded = 0;
        let mut page = start & !(PAGE - 1);
        let end = u64::from(start) + u64::from(len);
        while u64::from(page) < end {
            let r = *space.find(page).ok_or(FaultError::NoRegion)?;
            let access = if r.prot.write() {
                Access::Write
            } else {
                Access::Read
            };
            if self.tables.translate(self.mem, page).is_none() {
                self.handle_fault(space, page, access)?;
                loaded += 1;
            }
            // A private file page mapped read-only so far: make the copy now.
            if access == Access::Write
                && self
                    .tables
                    .translate(self.mem, page)
                    .is_some_and(|(_, a)| !a.ap.user_writable())
            {
                self.handle_fault(space, page, Access::Write)?;
            }
            page += PAGE;
        }
        Ok(loaded)
    }

    /// Compresses the frame at `phys` into the store: its handle, or `None`
    /// if it doesn't compress well enough. The caller has written back its
    /// cache lines.
    fn pack(&mut self, phys: u32) -> Result<Option<Handle>, FaultError> {
        let zb = &mut *self.zbuf;
        let packed_len = self.mem.with_page(phys, |p| {
            rp_lz4::compress_with(&mut zb.table, p, &mut zb.packed)
        });
        let Ok(n) = packed_len else {
            return Ok(None);
        };
        match self
            .zpool
            .store(self.frames, &mut ZPages(&mut *self.mem), &zb.packed[..n])
        {
            Ok(h) => Ok(Some(h)),
            Err(crate::zpool::ZError::TooBig) => Ok(None),
            Err(crate::zpool::ZError::NoMemory) => Err(FaultError::NoMemory),
        }
    }

    /// One step of the reclaim clock over `space` (ADR-0005): from `*hand`,
    /// visits up to `budget` pages of its regions in address order
    /// (wrapping). A resident page is *aged*: written back from the cache
    /// and made to fault on its next touch, which maps it again at once
    /// ([`Outcome::Referenced`]). A page still aged when the hand comes
    /// back wasn't touched for a whole sweep, so it is evicted: a frame the
    /// process owns (anonymous memory, a private copy) is compressed; a
    /// page-cache page is simply let go (it reloads on demand). Device
    /// windows are never touched. Stops after freeing `want` frames.
    pub fn reclaim(
        &mut self,
        space: &AddressSpace,
        hand: &mut u32,
        budget: u32,
        want: u32,
    ) -> Result<Reclaimed, FaultError> {
        let mut done = Reclaimed::default();
        while done.scanned < budget && done.compressed + done.dropped < want {
            let Some(page) = next_reclaimable(space, *hand) else {
                break;
            };
            *hand = page.wrapping_add(PAGE);
            done.scanned += 1;
            let Some(r) = space.find(page).copied() else {
                break;
            };
            if let Some((phys, attrs)) = self.tables.translate(self.mem, page) {
                // First visit: a second chance.
                self.mmu.clean_page(page);
                let f = self.mem.phys_frame(phys);
                let payload = aged(f, attrs.ap.user_writable());
                if self
                    .tables
                    .store(self.mem, self.mmu, page, payload)
                    .is_some()
                {
                    done.aged += 1;
                }
                continue;
            }
            let Some((f, writable)) = self.tables.stored(self.mem, page).and_then(decode_aged)
            else {
                continue; // unmapped, or compressed already
            };
            let phys = self.mem.frame_phys(f);
            let owned = match r.backing {
                Backing::Zero => true,
                Backing::File { shared, .. } => !shared && writable,
                Backing::Phys { .. } => continue,
            };
            if owned {
                // Its lines were written back when it was aged, and it
                // can't have been written since (any touch maps it again).
                let Some(h) = self.pack(phys)? else {
                    continue; // incompressible: stays resident, aged
                };
                self.tables.replace_stored(self.mem, page, h.raw());
                self.frames.free(f);
                done.compressed += 1;
            } else {
                self.tables.replace_stored(self.mem, page, 0);
                self.cache.release(phys);
                done.dropped += 1;
            }
        }
        Ok(done)
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
            // Compressed pages first: free their copies, in bounded batches.
            let end = p.start + p.len;
            let mut at = p.start;
            let mut batch = [0u32; 64];
            while at < end {
                let (k, next) = self.tables.take_stored(self.mem, at, end, &mut batch);
                for &raw in &batch[..k] {
                    if let Some((f, writable)) = decode_aged(raw) {
                        // Aged but resident: released like a mapped page.
                        let phys = self.mem.frame_phys(f);
                        release_page(p, phys, writable, base, self.frames, self.cache);
                    } else {
                        self.zpool.free(
                            self.frames,
                            &mut ZPages(&mut *self.mem),
                            Handle::from_raw(raw),
                        );
                    }
                }
                at = next;
            }
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
/// What a [`Vm::reclaim`] step did.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(test, derive(Debug))]
pub struct Reclaimed {
    /// Pages the hand visited.
    pub scanned: u32,
    /// Resident pages given a second chance.
    pub aged: u32,
    /// Untouched pages compressed (their frames freed).
    pub compressed: u32,
    /// Untouched page-cache pages let go.
    pub dropped: u32,
}

/// An aged page's stored payload: still resident (its frame, and whether
/// it was mapped writable), in a size class the compressed store never
/// uses, so it can't be mistaken for a compressed copy's handle.
const AGED_CLASS: u32 = 0xFF;
const _: () = assert!(crate::zpool::CLASSES < AGED_CLASS as usize);

fn aged(frame: usize, writable: bool) -> u32 {
    debug_assert!(frame < 1 << 14, "stored payloads have 30 bits");
    (frame as u32) << 16 | AGED_CLASS << 8 | u32::from(writable)
}

fn decode_aged(raw: u32) -> Option<(usize, bool)> {
    ((raw >> 8) & 0xFF == AGED_CLASS).then_some(((raw >> 16) as usize, raw & 1 != 0))
}

/// Whether reclaim may take pages from `r` (everything but device and
/// fixed physical windows, and pinned regions).
fn reclaimable(r: &Region) -> bool {
    !matches!(r.backing, Backing::Phys { .. }) && !r.prot.is_pinned()
}

/// The first reclaimable page at or after `hand`, wrapping to the lowest.
fn next_reclaimable(space: &AddressSpace, hand: u32) -> Option<u32> {
    let page = hand & !(PAGE - 1);
    let mut next = None::<u32>;
    let mut lowest = None::<u32>;
    for r in space.regions().filter(|r| reclaimable(r)) {
        if r.start <= page && page - r.start < r.len {
            return Some(page);
        }
        if r.start > page && next.is_none_or(|n| r.start < n) {
            next = Some(r.start);
        }
        if lowest.is_none_or(|l| r.start < l) {
            lowest = Some(r.start);
        }
    }
    next.or(lowest)
}

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
        zpool: ZPool,
        zbuf: Box<ZBuffers>,
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
                zpool: ZPool::new(),
                zbuf: Box::new(ZBuffers::new()),
            }
        }

        fn vm(&mut self) -> Vm<'_, Ram, Frames<512>, Log, Cache> {
            Vm {
                tables: &mut self.tables,
                mem: &mut self.ram,
                frames: &mut self.frames,
                mmu: &mut self.mmu,
                cache: &mut self.cache,
                zpool: &mut self.zpool,
                zbuf: &mut self.zbuf,
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
    fn idle_pages_compress_and_come_back_on_touch() {
        let mut w = World::new();
        let mut s = AddressSpace::process(SLOT, 2);
        let a = base() + 0x10000;
        s.map(zero(a, 0x3000, Prot::RW)).unwrap();
        for k in 0..3 {
            w.vm()
                .handle_fault(&s, a + k * 0x1000, Access::Write)
                .unwrap();
        }
        // Give the pages contents: mostly zeros, a few words.
        for k in 0..3u32 {
            let (phys, _) = w.tables.translate(&mut w.ram, a + k * 0x1000).unwrap();
            w.ram.page(phys)[k as usize * 10] = 0xC0DE_0000 + k;
        }
        let before = w.frames.free_frames();
        w.mmu.0.clear();
        for k in 0..3 {
            assert_eq!(w.vm().compress(&s, a + k * 0x1000), Ok(true));
        }
        assert_eq!(
            w.mmu.0[0],
            format!("clean {a:#x}"),
            "cache cleaned before reading"
        );
        assert_eq!(
            w.tables.translate(&mut w.ram, a),
            None,
            "unmapped while compressed"
        );
        let (zframes, objects, _) = w.zpool.stats();
        assert_eq!(objects, 3);
        assert_eq!(
            w.frames.free_frames(),
            before + 3 - zframes,
            "three frames freed, one zpool frame used"
        );
        // Touch one: it comes back with its contents.
        assert_eq!(
            w.vm().handle_fault(&s, a + 0x1000 + 40, Access::Read),
            Ok(Outcome::Restored)
        );
        assert_eq!(w.peek(a + 0x1000).map(|_| ()), Some(()));
        let (phys, attrs) = w.tables.translate(&mut w.ram, a + 0x1000).unwrap();
        assert_eq!(w.ram.page(phys)[10], 0xC0DE_0001);
        assert_eq!(attrs.ap, Ap::UserWrite);
        assert_eq!(w.zpool.stats().1, 2);
        // Unmapping frees the remaining compressed copies too.
        w.vm().unmap(&mut s, a, 0x3000).unwrap();
        assert_eq!(w.zpool.stats(), (0, 0, 0));
        assert_eq!(w.frames.free_frames(), 512);
    }

    #[test]
    fn only_anonymous_compressible_pages_are_compressed() {
        let mut w = World::new();
        let mut s = AddressSpace::process(SLOT, 2);
        let a = base() + 0x10000;
        s.map(zero(a, 0x2000, Prot::RW)).unwrap();
        s.map(file(a + 0x2000, 0x1000, Prot::R, false)).unwrap();
        assert_eq!(w.vm().compress(&s, a), Ok(false), "not mapped yet");
        w.vm().handle_fault(&s, a, Access::Write).unwrap();
        w.vm().handle_fault(&s, a + 0x2000, Access::Read).unwrap();
        assert_eq!(
            w.vm().compress(&s, a + 0x2000),
            Ok(false),
            "file pages are dropped, not compressed"
        );
        assert_eq!(w.vm().compress(&s, a + 0x9000), Ok(false), "no region");
        // Noise doesn't compress: it stays mapped.
        let (phys, _) = w.tables.translate(&mut w.ram, a).unwrap();
        let mut x = 7u32;
        for v in w.ram.page(phys).iter_mut() {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            *v = x;
        }
        assert_eq!(w.vm().compress(&s, a), Ok(false));
        assert!(w.tables.translate(&mut w.ram, a).is_some());
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
            zpool: &mut w.zpool,
            zbuf: &mut w.zbuf,
        };
        assert_eq!(
            vm.handle_fault(&s2, layout::slot_base(6), Access::Write),
            Err(FaultError::NoMemory)
        );
        assert_eq!(w.frames.free_frames(), 512);
    }

    /// Faults in `n` writable zero pages at `a` and marks each with a word.
    fn resident(w: &mut World, s: &AddressSpace, a: u32, n: u32) {
        for k in 0..n {
            w.vm()
                .handle_fault(s, a + k * 0x1000, Access::Write)
                .unwrap();
            let (phys, _) = w.tables.translate(&mut w.ram, a + k * 0x1000).unwrap();
            w.ram.page(phys)[3] = 0xA6E0_0000 + k;
        }
    }

    #[test]
    fn reclaim_gives_a_second_chance_then_compresses_untouched_pages() {
        let mut w = World::new();
        let mut s = AddressSpace::process(SLOT, 2);
        let a = base() + 0x10000;
        s.map(zero(a, 0x4000, Prot::RW)).unwrap();
        resident(&mut w, &s, a, 4);
        let free0 = w.frames.free_frames();
        // First sweep: everything is aged, nothing freed yet.
        let mut hand = a;
        let r = w.vm().reclaim(&s, &mut hand, 4, u32::MAX).unwrap();
        assert_eq!(
            r,
            Reclaimed {
                scanned: 4,
                aged: 4,
                compressed: 0,
                dropped: 0
            }
        );
        assert_eq!(w.tables.translate(&mut w.ram, a), None, "aged pages fault");
        assert_eq!(w.frames.free_frames(), free0);
        // Page 0 is touched: a minor fault maps it again, as it was.
        assert_eq!(
            w.vm().handle_fault(&s, a + 8, Access::Read),
            Ok(Outcome::Referenced)
        );
        let (phys, attrs) = w.tables.translate(&mut w.ram, a).unwrap();
        assert_eq!(w.ram.page(phys)[3], 0xA6E0_0000);
        assert_eq!(attrs.ap, Ap::UserWrite, "it was writable, and still is");
        // Second sweep: page 0 gets another chance, the rest go.
        let mut hand = a;
        let r = w.vm().reclaim(&s, &mut hand, 4, u32::MAX).unwrap();
        assert_eq!((r.aged, r.compressed, r.dropped), (1, 3, 0));
        let (zframes, objects, _) = w.zpool.stats();
        assert_eq!(objects, 3);
        assert_eq!(w.frames.free_frames(), free0 + 3 - zframes);
        // An evicted page comes back intact.
        assert_eq!(
            w.vm().handle_fault(&s, a + 0x2000, Access::Read),
            Ok(Outcome::Restored)
        );
        let (phys, _) = w.tables.translate(&mut w.ram, a + 0x2000).unwrap();
        assert_eq!(w.ram.page(phys)[3], 0xA6E0_0002);
        // Unmapping frees everything: aged, compressed and mapped pages.
        w.vm().unmap(&mut s, a, 0x4000).unwrap();
        assert_eq!(w.zpool.stats(), (0, 0, 0));
        assert_eq!(w.frames.free_frames(), 512);
    }

    #[test]
    fn reclaim_lets_page_cache_pages_go_and_they_reload() {
        let mut w = World::new();
        let mut s = AddressSpace::process(SLOT, 2);
        let a = base() + 0x10000;
        s.map(file(a, 0x2000, Prot::R, false)).unwrap();
        s.map(file(a + 0x2000, 0x1000, Prot::R, true)).unwrap();
        for k in 0..3 {
            w.vm()
                .handle_fault(&s, a + k * 0x1000, Access::Read)
                .unwrap();
        }
        assert_eq!(w.cache.total_refs(), 3);
        let mut hand = a;
        w.vm().reclaim(&s, &mut hand, 3, u32::MAX).unwrap();
        let r = w.vm().reclaim(&s, &mut hand, 3, u32::MAX).unwrap();
        assert_eq!((r.compressed, r.dropped), (0, 3));
        assert_eq!(w.cache.total_refs(), 0, "every cache reference returned");
        assert_eq!(w.zpool.stats().1, 0, "nothing compressed");
        assert_eq!(
            w.vm().handle_fault(&s, a + 0x1000, Access::Read),
            Ok(Outcome::Mapped),
            "reloaded from the cache"
        );
        assert_eq!(w.cache.total_refs(), 1);
    }

    #[test]
    fn a_private_copy_is_compressed_not_dropped() {
        let mut w = World::new();
        let mut s = AddressSpace::process(SLOT, 2);
        let a = base() + 0x10000;
        s.map(file(a, 0x1000, Prot::RW, false)).unwrap();
        w.vm().handle_fault(&s, a, Access::Write).unwrap(); // copied
        let mut hand = a;
        w.vm().reclaim(&s, &mut hand, 1, u32::MAX).unwrap();
        let r = w.vm().reclaim(&s, &mut hand, 1, u32::MAX).unwrap();
        assert_eq!((r.compressed, r.dropped), (1, 0));
        assert_eq!(w.cache.total_refs(), 0);
        assert_eq!(
            w.vm().handle_fault(&s, a, Access::Write),
            Ok(Outcome::Restored)
        );
        let (_, attrs) = w.tables.translate(&mut w.ram, a).unwrap();
        assert_eq!(attrs.ap, Ap::UserWrite, "still the process's own copy");
    }

    #[test]
    fn reclaim_skips_device_windows_wraps_and_stops_when_satisfied() {
        let mut w = World::new();
        let mut s = AddressSpace::process(SLOT, 2);
        let a = base() + 0x10000;
        s.map(Region {
            start: a,
            len: 0x2000,
            prot: Prot::RW,
            backing: Backing::Phys {
                base: 0x4400_0000,
                cache: crate::arm::Cache::Device,
            },
        })
        .unwrap();
        s.map(zero(a + 0x8000, 0x4000, Prot::RW)).unwrap();
        w.vm().handle_fault(&s, a, Access::Read).unwrap();
        resident(&mut w, &s, a + 0x8000, 4);
        // The hand starts in the device window: it jumps to the next
        // reclaimable region, and the window's page stays mapped.
        let mut hand = a;
        let r = w.vm().reclaim(&s, &mut hand, 2, u32::MAX).unwrap();
        assert_eq!(r.aged, 2);
        assert_eq!(hand, a + 0xA000);
        assert!(w.tables.translate(&mut w.ram, a).is_some());
        // Past the end it wraps to the lowest reclaimable page.
        let mut hand = a + 0xC000;
        let r = w.vm().reclaim(&s, &mut hand, 1, u32::MAX).unwrap();
        assert_eq!(r.scanned, 1);
        assert_eq!(hand, a + 0x9000, "wrapped to the region's first page");
        // It stops once it freed what was asked for.
        let mut w = World::new();
        resident(&mut w, &s, a + 0x8000, 4);
        let mut hand = a + 0x8000;
        w.vm().reclaim(&s, &mut hand, 4, u32::MAX).unwrap();
        let r = w.vm().reclaim(&s, &mut hand, 8, 2).unwrap();
        assert_eq!((r.scanned, r.compressed), (2, 2));
        // An empty space: nothing to do, no endless loop.
        let empty = AddressSpace::process(SLOT, 2);
        let r = w.vm().reclaim(&empty, &mut hand, 8, 8).unwrap();
        assert_eq!(r, Reclaimed::default());
    }

    #[test]
    fn a_page_made_read_only_while_aged_comes_back_read_only() {
        let mut w = World::new();
        let mut s = AddressSpace::process(SLOT, 2);
        let a = base() + 0x10000;
        s.map(zero(a, 0x1000, Prot::RW)).unwrap();
        resident(&mut w, &s, a, 1);
        let mut hand = a;
        w.vm().reclaim(&s, &mut hand, 1, u32::MAX).unwrap();
        w.vm().protect(&mut s, a, 0x1000, Prot::R).unwrap();
        assert_eq!(
            w.vm().handle_fault(&s, a, Access::Read),
            Ok(Outcome::Referenced)
        );
        let (_, attrs) = w.tables.translate(&mut w.ram, a).unwrap();
        assert_eq!(attrs.ap, Ap::UserRead);
        assert_eq!(
            w.vm().handle_fault(&s, a, Access::Write),
            Err(FaultError::Protection)
        );
    }

    #[test]
    fn a_record_is_reused_in_place_for_another_process() {
        let mut s = AddressSpace::process(SLOT, 2);
        s.map(zero(base() + 0x10000, 0x1000, Prot::RW)).unwrap();
        s.allow_write_exec();
        s.reset_process(9, 4);
        assert_eq!(s.regions().count(), 0);
        assert_eq!(s.domain(), 4);
        // Its window is slot 9's now, and W^X is back.
        let b9 = layout::slot_base(9);
        assert_eq!(
            s.map(zero(base() + 0x10000, 0x1000, Prot::RW)),
            Err(SpaceError::BadRange)
        );
        assert_eq!(
            s.map(zero(b9 + 0x10000, 0x1000, Prot::RWX)),
            Err(SpaceError::WriteExec)
        );
        assert!(s.map(zero(b9 + 0x10000, 0x1000, Prot::RW)).is_ok());
    }

    #[test]
    fn pinned_pages_stay_resident_through_reclaim() {
        let mut w = World::new();
        let mut s = AddressSpace::process(SLOT, 2);
        let a = base() + 0x10000;
        s.map(zero(a, 0x2000, Prot::RW.pinned())).unwrap();
        s.map(zero(a + 0x2000, 0x2000, Prot::RW)).unwrap();
        // Populating loads the pinned pages now: no fault later.
        assert_eq!(w.vm().populate(&s, a, 0x2000), Ok(2));
        assert_eq!(w.vm().populate(&s, a, 0x2000), Ok(0), "already there");
        resident(&mut w, &s, a + 0x2000, 2);
        let mut hand = a;
        for _ in 0..3 {
            w.vm().reclaim(&s, &mut hand, 8, u32::MAX).unwrap();
        }
        // The ordinary pages went; the pinned ones never left.
        assert!(w.tables.translate(&mut w.ram, a).is_some());
        assert!(w.tables.translate(&mut w.ram, a + 0x1000).is_some());
        assert!(w.tables.translate(&mut w.ram, a + 0x2000).is_none());
        assert_eq!(
            w.vm().compress(&s, a),
            Ok(false),
            "pinned: never compressed"
        );
    }

    #[test]
    fn pinning_is_limited_and_survives_protect() {
        let mut s = AddressSpace::process(SLOT, 2);
        let a = base() + 0x10000;
        let limit = DEFAULT_PIN_LIMIT;
        assert_eq!(
            s.map(zero(a, limit + 0x1000, Prot::RW.pinned())),
            Err(SpaceError::PinLimit)
        );
        s.map(zero(a, limit, Prot::RW.pinned())).unwrap();
        assert_eq!(s.pinned_bytes(), u64::from(limit));
        // More pinning is refused, by map or by protect.
        let b = a + limit;
        s.map(zero(b, 0x1000, Prot::RW)).unwrap();
        assert_eq!(
            s.protect(b, 0x1000, Prot::R.pinned()),
            Err(SpaceError::PinLimit)
        );
        // Changing a pinned region's permissions keeps it pinned.
        s.protect(a, 0x1000, Prot::R).unwrap();
        assert!(s.find(a).unwrap().prot.is_pinned());
        assert_eq!(s.pinned_bytes(), u64::from(limit));
        // The kernel can raise a trusted service's limit.
        s.set_pin_limit(limit + 0x1000);
        assert!(s.protect(b, 0x1000, Prot::R.pinned()).is_ok());
    }
}
