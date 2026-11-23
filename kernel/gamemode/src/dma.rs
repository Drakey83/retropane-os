//! A game's DMA, checked and translated by the kernel (G7a,
//! docs/design/game-mode-build.md G7).
//!
//! The PXA270 has no IOMMU: a DMA channel reaches every physical address.
//! A game given the controller's registers could copy over the kernel or a
//! frozen program, which is what Windows Mobile's in-process drivers and
//! Linux's UIO allow. Here the game never writes a DMA register. It builds
//! its chain in its own memory with its own (virtual) addresses and hands
//! it to the kernel in one system call; [`translate`] checks every
//! descriptor against the game's own mappings and grants and writes the
//! chain the controller runs, in the kernel's memory, with physical
//! addresses. Linux's `dma_map_single` costs a call a buffer; this costs one
//! a chain.
//!
//! - **Memory sides** (the address increments): every byte must be the
//!   game's, readable for a source and writable for a target. A descriptor
//!   is split where its pages are not physically contiguous (4 KiB demand
//!   pages; a section is contiguous for its whole megabyte).
//! - **Device sides** (the address holds still: a FIFO): the address must be
//!   inside a device window the game holds a grant for; it becomes the
//!   device's physical address.
//! - **Flags:** interrupts, fly-by and descriptor compare are refused (the
//!   game holds no interrupt; fly-by drives an external chip's pins).
//!   Flow control, width and burst size pass as given.
//! - **Bounded:** at most [`MAX_IN`] descriptors in, [`MAX_OUT`] out;
//!   anything beyond is refused before anything is written.

/// `DCMD` bits (PXA27x Developer's Manual, the DMA controller's `DCMDx`).
pub mod dcmd {
    /// The source address increments (a memory source).
    pub const INC_SRC: u32 = 1 << 31;
    /// The target address increments (a memory target).
    pub const INC_TRG: u32 = 1 << 30;
    /// Flow control by the source's request line.
    pub const FLOW_SRC: u32 = 1 << 29;
    /// Flow control by the target's request line.
    pub const FLOW_TRG: u32 = 1 << 28;
    /// Descriptor compare.
    pub const CMP_EN: u32 = 1 << 25;
    /// Address mode (with compare).
    pub const ADDR_MODE: u32 = 1 << 23;
    /// An interrupt when the descriptor starts.
    pub const START_IRQ: u32 = 1 << 22;
    /// An interrupt when the descriptor ends.
    pub const END_IRQ: u32 = 1 << 21;
    /// Fly-by source.
    pub const FLY_BY_S: u32 = 1 << 20;
    /// Fly-by target.
    pub const FLY_BY_T: u32 = 1 << 19;
    /// The length field (bytes, at most 8,191).
    pub const LEN: u32 = 0x1FFF;
    /// What a game may never set.
    pub const REFUSED: u32 = CMP_EN | ADDR_MODE | START_IRQ | END_IRQ | FLY_BY_S | FLY_BY_T;
}

/// `DDADR`'s stop bit: the chain ends after this descriptor.
pub const STOP: u32 = 1;
/// Descriptors a game may hand in at once.
pub const MAX_IN: usize = 32;
/// Descriptors the kernel's chain holds: each one in may split at most at
/// two page edges (8,191 bytes touch at most three pages).
pub const MAX_OUT: usize = 3 * MAX_IN;

const PAGE: u32 = 4096;

/// A descriptor as the game writes it: its own addresses; `next` is not
/// read (the chain is the array, in order).
#[derive(Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub struct Desc {
    /// The source address.
    pub src: u32,
    /// The target address.
    pub dst: u32,
    /// `DCMD`.
    pub cmd: u32,
}

/// A descriptor as the controller reads it (`DDADR`, `DSADR`, `DTADR`,
/// `DCMD`), 16 bytes.
pub type HwDesc = [u32; 4];

/// Why a chain was refused (nothing was written).
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub enum DmaError {
    /// No descriptors, or more than [`MAX_IN`].
    Count,
    /// Descriptor `n` sets a refused flag, has no length, or both sides
    /// hold still.
    Flags(u8),
    /// Descriptor `n` names memory that is not the game's (or not
    /// writable, for a target).
    NotYours(u8),
    /// Descriptor `n` names a device the game holds no grant for.
    NoGrant(u8),
    /// The split chain would pass [`MAX_OUT`].
    TooLong,
}

/// What the game may reach: its own memory, page by page, and its device
/// windows.
pub trait GameMemory {
    /// The physical address of the game's byte at `va` (the page's
    /// translation), if the game has it mapped, and writable when `write`.
    fn phys(&mut self, va: u32, write: bool) -> Option<u32>;
    /// The device's physical address for `va` inside a window the game is
    /// granted (and the window's bytes from `va` on), if it is.
    fn device(&mut self, va: u32) -> Option<(u32, u32)>;
}

/// One side of a descriptor: where it starts and whether it moves.
#[derive(Clone, Copy)]
struct Side {
    va: u32,
    moves: bool,
    write: bool,
}

/// How many bytes from `va` (at most `len`) are physically contiguous in
/// the game's memory, and where they start; `None` if the first byte isn't
/// the game's.
fn run(m: &mut impl GameMemory, va: u32, len: u32, write: bool) -> Option<(u32, u32)> {
    let start = m.phys(va, write)?;
    let mut n = (PAGE - va % PAGE).min(len);
    while n < len {
        let next = m.phys(va.checked_add(n)?, write)?;
        if next != start.wrapping_add(n) {
            break;
        }
        n = (n + PAGE).min(len);
    }
    Some((start, n))
}

/// Checks `chain` (the game's, in order) and writes the controller's chain
/// into `out`, its first descriptor at physical address `out_phys` (16-byte
/// aligned), each linked to the next and the last marked [`STOP`]. The
/// descriptors written, or why the chain was refused; on a refusal `out`
/// holds nothing that a channel could be started on (nothing is written
/// before every descriptor has been checked and split).
pub fn translate(
    m: &mut impl GameMemory,
    chain: &[Desc],
    out: &mut [HwDesc; MAX_OUT],
    out_phys: u32,
) -> Result<usize, DmaError> {
    if chain.is_empty() || chain.len() > MAX_IN {
        return Err(DmaError::Count);
    }
    // Pass 1: every descriptor checked and its pieces counted; nothing
    // written.
    let mut total = 0;
    for (i, d) in chain.iter().enumerate() {
        total += pieces(m, i as u8, d, &mut |_| {})?;
        if total > MAX_OUT {
            return Err(DmaError::TooLong);
        }
    }
    // Pass 2: the same walk, writing.
    let mut n = 0;
    for (i, d) in chain.iter().enumerate() {
        pieces(m, i as u8, d, &mut |p| {
            out[n] = p;
            n += 1;
        })?;
    }
    for (k, d) in out[..n].iter_mut().enumerate() {
        d[0] = if k + 1 == n {
            STOP
        } else {
            out_phys + 16 * (k as u32 + 1)
        };
    }
    Ok(n)
}

/// Descriptor `d`'s pieces (`DDADR` left 0 for [`translate`] to link),
/// each given to `put`; how many, or why `d` is refused.
fn pieces(
    m: &mut impl GameMemory,
    i: u8,
    d: &Desc,
    put: &mut dyn FnMut(HwDesc),
) -> Result<usize, DmaError> {
    let len = d.cmd & dcmd::LEN;
    let src = Side {
        va: d.src,
        moves: d.cmd & dcmd::INC_SRC != 0,
        write: false,
    };
    let dst = Side {
        va: d.dst,
        moves: d.cmd & dcmd::INC_TRG != 0,
        write: true,
    };
    if d.cmd & dcmd::REFUSED != 0 || len == 0 || !(src.moves || dst.moves) {
        return Err(DmaError::Flags(i));
    }
    // A device side: inside its window for every byte the descriptor
    // moves through it (a FIFO's address holds still, so its one word).
    let mut fixed = |s: Side| -> Result<u32, DmaError> {
        match m.device(s.va) {
            Some((pa, room)) if room >= 4 => Ok(pa),
            _ => Err(DmaError::NoGrant(i)),
        }
    };
    let src_dev = if src.moves { None } else { Some(fixed(src)?) };
    let dst_dev = if dst.moves { None } else { Some(fixed(dst)?) };
    let keep = d.cmd & !dcmd::LEN;
    let mut done = 0;
    let mut count = 0;
    while done < len {
        let left = len - done;
        let mut piece = left;
        let mut at = [0u32; 2];
        for (k, (side, dev)) in [(src, src_dev), (dst, dst_dev)].into_iter().enumerate() {
            at[k] = match dev {
                Some(pa) => pa,
                None => {
                    let va = side.va.checked_add(done).ok_or(DmaError::NotYours(i))?;
                    let (pa, n) = run(m, va, left, side.write).ok_or(DmaError::NotYours(i))?;
                    piece = piece.min(n);
                    pa
                }
            };
        }
        put([0, at[0], at[1], keep | piece]);
        done += piece;
        count += 1;
    }
    Ok(count)
}

/// A cache operation a game may ask for ([`check_range`] first).
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub enum CacheOp {
    /// Write back the D-cache lines (before a device reads memory).
    Clean,
    /// Discard them (after a device wrote memory): needs write access.
    Invalidate,
    /// Write back, then discard.
    CleanInvalidate,
    /// Drain the write buffer (no range).
    Drain,
}

impl CacheOp {
    /// From the system call's operation number.
    #[must_use]
    pub const fn from_u32(v: u32) -> Option<Self> {
        match v {
            0 => Some(Self::Clean),
            1 => Some(Self::Invalidate),
            2 => Some(Self::CleanInvalidate),
            3 => Some(Self::Drain),
            _ => None,
        }
    }
}

/// The most a cache call covers: beyond it the whole D-cache is cheaper
/// than line by line, and a game asking for more is refused (it can ask
/// again) so one call can't hold the kernel for long.
pub const CACHE_MOST: u32 = 64 * 1024;

/// Whether `op` on `va..va + len` is allowed: every page the game's
/// (writable, for a discard: a discard throws away what's in the cache, so
/// it is a write). [`CacheOp::Drain`] takes no range.
pub fn check_range(m: &mut impl GameMemory, op: CacheOp, va: u32, len: u32) -> bool {
    if op == CacheOp::Drain {
        return true;
    }
    if len == 0 || len > CACHE_MOST {
        return false;
    }
    let Some(end) = va.checked_add(len) else {
        return false;
    };
    let write = op != CacheOp::Clean;
    let mut page = va & !(PAGE - 1);
    while page < end {
        if m.phys(page.max(va), write).is_none() {
            return false;
        }
        page += PAGE;
    }
    true
}

#[cfg(test)]
mod tests;
