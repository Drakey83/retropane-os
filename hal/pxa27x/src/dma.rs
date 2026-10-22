//! The PXA27x DMA controller, for device FIFOs (PXA27x Developer's Manual,
//! DMA controller chapter).
//!
//! The channels `dma_map` lists, each through [`rp_hal_api::DmaEngine`] (a
//! transfer between a device FIFO and a memory buffer, paced by the
//! device's request line, in descriptor mode: 4 KiB per descriptor, any
//! length, streamed through the channel's ring of slots (its `dma_map`
//! chain) by [`crate::dmachain`] as [`DmaEngine::done`] is polled) or
//! [`rp_hal_api::DmaRing`] (a buffer looped into a device, for audio: cut
//! into as many pieces as the chain holds, the last linked back to the
//! first, until it is given an end; [`crate::dmaring`]).
//! The cache maintenance the VIVT caches need is done here, so no caller
//! can forget it: buffers are cleaned (and for reads discarded) before the
//! transfer, and reads discarded again after, so the CPU sees what the
//! device wrote.
//!
//! Buffers must be whole cache lines (32-byte aligned, a multiple of 32
//! bytes long) in identity-mapped memory (the bring-up map): the DMA
//! controller sees physical addresses.

use rp_hal_api::{DmaEngine, DmaRing, Port, RingEnd, Width, WindowDma};

use crate::cpu;
use crate::dma_map;
use crate::dmachain::{self, Addr, Next, Seen, Shape, Stream};
use crate::dmaring::{self, Ring};

const BASE: usize = 0x4000_0000;
const fn dcsr(ch: u32) -> usize {
    BASE + 4 * ch as usize
}
const fn drcmr(request: u8) -> usize {
    if request < 64 {
        BASE + 0x100 + 4 * request as usize
    } else {
        BASE + 0x1100 + 4 * (request as usize - 64)
    }
}
const fn ddadr(ch: u32) -> usize {
    BASE + 0x200 + 16 * ch as usize
}
const fn dsadr(ch: u32) -> usize {
    ddadr(ch) + 4
}
const fn dtadr(ch: u32) -> usize {
    ddadr(ch) + 8
}

const DCSR_RUN: u32 = 1 << 31;
/// Descriptor-fetch mode is the reset default (NODESCFETCH clear).
const DCSR_BUSERR: u32 = 1 << 0;
const DCSR_STOPINTR: u32 = 1 << 3;
/// `DCSR` reads before a stopping channel is given up on (a 32-byte step
/// takes a few microseconds; this is ~10 ms).
const HALT_POLLS: u32 = 50_000;
const DRCMR_MAPVLD: u32 = 1 << 7;
const DCMD_INCSRC: u32 = 1 << 31;
const DCMD_INCTRG: u32 = 1 << 30;
const DCMD_FLOWSRC: u32 = 1 << 29;
const DCMD_FLOWTRG: u32 = 1 << 28;
/// 32-byte bursts (a FIFO request's worth).
const DCMD_SIZE32: u32 = 3 << 16;

/// The DCMD width field for a port.
fn width(w: Width) -> u32 {
    match w {
        Width::Byte => 1 << 14,
        Width::Half => 2 << 14,
        Width::Word => 3 << 14,
    }
}

/// Bytes per descriptor (under the 8,191 the length field holds).
const CHUNK: u32 = dma_map::CHUNK as u32;

#[repr(C, align(16))]
struct Descriptors([[u32; 4]; dma_map::TOTAL]);

/// Every channel's ring of descriptor slots, each sized by the channel map
/// (`dma_map`); the flags in `rt` hand each channel out once, so each ring
/// has one user.
static mut CHAINS: Descriptors = Descriptors([[0; 4]; dma_map::TOTAL]);

/// The DMA channel handed out by [`crate::rt::take_dma`].
pub struct Dma {
    ch: u32,
    request: u8,
    /// The transfer is paced by `request` (a device FIFO), not memory to
    /// memory: stopping unmaps the request line.
    paced: bool,
    /// The armed transfer's buffer (address, length) and direction.
    armed: Option<(u32, u32, bool)>,
    /// The armed transfer's descriptors, streamed through the chain.
    stream: Option<Stream>,
    /// The armed transfer was written whole, links and all, before the
    /// controller started: it can't stop short, so polls need only `DCSR`.
    whole: bool,
    /// The running ring.
    ring: Option<Ring>,
}

fn write(addr: usize, v: u32) {
    // SAFETY: a DMA controller register (PXA27x memory map, identity-mapped
    // device memory); only the one `Dma` writes them.
    unsafe { (addr as *mut u32).write_volatile(v) }
}

fn read(addr: usize) -> u32 {
    // SAFETY: as in `write`.
    unsafe { (addr as *const u32).read_volatile() }
}

impl Dma {
    /// The DMA channel `ch` (the caller owns it; see [`crate::rt::take_dma`]).
    pub(crate) const fn new(ch: u32) -> Self {
        Self {
            ch,
            request: 0,
            paced: false,
            armed: None,
            stream: None,
            whole: false,
            ring: None,
        }
    }

    /// The channel's registers, read only (for logs): `DCSR`, `DDADR`,
    /// `DSADR`, `DTADR`, and the paced request's `DRCMR`.
    #[must_use]
    pub fn registers(&self) -> [u32; 5] {
        [
            read(dcsr(self.ch)),
            read(ddadr(self.ch)),
            read(dsadr(self.ch)),
            read(dtadr(self.ch)),
            read(drcmr(self.request)),
        ]
    }

    /// Why the running ring's last failed end failed, and the registers
    /// it read then (`DDADR`, `DSADR`, `DCSR` said stopped): for logs.
    #[must_use]
    pub fn ring_failure(&self) -> Option<(&'static str, [u32; 2], bool)> {
        let (why, s) = self.ring?.failure()?;
        Some((why, [s.ddadr, s.dsadr], s.stopped))
    }

    /// Stops the channel and waits until it says it has stopped
    /// (`DCSR.STOPINTR`, bounded). A channel only stops once its current
    /// transfer step ends; until then it still reads its descriptors and
    /// takes no new descriptor address, so nothing may rewrite its chain or
    /// registers before this (2026-10-18: a ring restarted right after a
    /// stop stalled on the X51v, where one restarted seconds later played).
    fn halt(&mut self) -> bool {
        write(dcsr(self.ch), 0);
        for _ in 0..HALT_POLLS {
            if read(dcsr(self.ch)) & DCSR_STOPINTR != 0 {
                return true;
            }
        }
        false
    }

    /// This channel's ring of descriptor slots (its slice of the shared
    /// array).
    fn chain(&mut self) -> &'static mut [[u32; 4]] {
        let (at, n) = (dma_map::first(self.ch), dma_map::descriptors(self.ch));
        // SAFETY: each channel's slice is touched only by its one `Dma`
        // (handed out once by `rt::take_dma`, only for listed channels:
        // slices don't overlap). While the controller runs, only slots it
        // has finished with are written (`dmachain`); the ring's slots are
        // written only while it is stopped.
        unsafe {
            let all: &'static mut Descriptors = &mut *core::ptr::addr_of_mut!(CHAINS);
            &mut all.0[at..at + n]
        }
    }

    /// Fills the chain with a ring of `len` bytes at `addr` looped into
    /// `port` (cut by [`dmaring::cut`]) and starts the channel; `None` if
    /// it can't be cut.
    fn program_ring(&mut self, port: Port, addr: u32, len: u32) -> Option<Ring> {
        self.halt();
        let chain = self.chain();
        let base = chain.as_ptr() as u32;
        let cmd = DCMD_INCSRC | DCMD_FLOWTRG | DCMD_SIZE32 | width(port.width);
        let ring = Ring::start(addr, len, port.addr, cmd, chain, base)?;
        // The controller reads the chain from memory.
        cpu::clean_range(base, 16 * ring.pieces());
        write(dcsr(self.ch), 0);
        write(drcmr(port.request), DRCMR_MAPVLD | self.ch);
        write(ddadr(self.ch), base);
        write(dcsr(self.ch), DCSR_RUN);
        self.request = port.request;
        self.paced = true;
        Some(ring)
    }

    /// The registers, `DCSR` first (once it says stopped, the others read
    /// after it are final).
    fn seen(&self) -> Seen {
        let stopped = read(dcsr(self.ch)) & DCSR_STOPINTR != 0;
        self.snapshot(stopped)
    }

    /// The address registers, agreeing with each other
    /// ([`dmachain::snapshot`]; `stopped` is `DCSR`'s, read first).
    fn snapshot(&self, stopped: bool) -> Seen {
        let ch = self.ch;
        dmachain::snapshot(stopped, |a| {
            read(match a {
                Addr::Ddadr => ddadr(ch),
                Addr::Dsadr => dsadr(ch),
                Addr::Dtadr => dtadr(ch),
            })
        })
    }

    /// Starts streaming `shape`: the first descriptors written, the chain
    /// cleaned, the channel run from its first slot, paced by `request` or
    /// memory to memory. `false` if the shape is refused.
    fn begin(&mut self, shape: Shape, request: Option<u8>) -> bool {
        self.halt();
        let chain = self.chain();
        let base = chain.as_ptr() as u32;
        let Some(stream) = Stream::start(shape, chain, base) else {
            return false;
        };
        // The controller reads the chain from memory: the slots written
        // (the first ones; a short transfer uses few of the ring's).
        cpu::clean_range(base, 16 * stream.written());
        write(dcsr(self.ch), 0);
        if let Some(r) = request {
            write(drcmr(r), DRCMR_MAPVLD | self.ch);
            self.request = r;
        }
        write(ddadr(self.ch), base);
        write(dcsr(self.ch), DCSR_RUN);
        self.paced = request.is_some();
        self.whole = stream.written() == stream.count();
        self.stream = Some(stream);
        true
    }

    fn arm(&mut self, port: Port, addr: u32, len: usize, to_device: bool) -> bool {
        // Whole cache lines only: a line shared with other data could be
        // dirtied by the CPU mid-transfer and written back over what the
        // device wrote. Callers fall back to programmed I/O.
        let Ok(len) = u32::try_from(len) else {
            return false;
        };
        let lines = (addr | len) & (cpu::LINE - 1) == 0;
        if self.armed.is_some() || self.ring.is_some() || !lines || len == 0 {
            return false;
        }
        let cmd = DCMD_SIZE32 | width(port.width);
        let shape = if to_device {
            Shape {
                src: addr,
                dst: port.addr,
                src_step: CHUNK,
                dst_step: 0,
                chunk: CHUNK,
                len,
                cmd: DCMD_INCSRC | DCMD_FLOWTRG | cmd,
            }
        } else {
            Shape {
                src: port.addr,
                dst: addr,
                src_step: 0,
                dst_step: CHUNK,
                chunk: CHUNK,
                len,
                cmd: DCMD_INCTRG | DCMD_FLOWSRC | cmd,
            }
        };
        // Dirty lines would overwrite what the device writes when evicted;
        // for writes, memory must hold what the CPU wrote.
        cpu::clean_invalidate_range(addr, len);
        if !self.begin(shape, Some(port.request)) {
            return false;
        }
        self.armed = Some((addr, len, to_device));
        true
    }

    /// Starts a memory-to-memory copy of `src` (SDRAM, whole cache lines,
    /// any length) to physical address `dst` in device memory that the
    /// CPU doesn't cache (2700G local memory): no request line, the
    /// controller runs at its own pace while the CPU does other work.
    /// Finish with [`DmaEngine::done`] (each poll also feeds the chain).
    /// `false` if refused (busy, not whole lines, or `dst` not word
    /// aligned).
    pub fn arm_copy_to_device_memory(&mut self, src: &[u8], dst: u32) -> bool {
        let addr = src.as_ptr() as u32;
        let Ok(len) = u32::try_from(src.len()) else {
            return false;
        };
        if self.armed.is_some()
            || self.ring.is_some()
            || (addr | len) & (cpu::LINE - 1) != 0
            || dst & 3 != 0
            || len == 0
        {
            return false;
        }
        // The controller reads memory, not the cache.
        cpu::clean_range(addr, len);
        let shape = Shape {
            src: addr,
            dst,
            src_step: CHUNK,
            dst_step: CHUNK,
            chunk: CHUNK,
            len,
            cmd: DCMD_INCSRC | DCMD_INCTRG | DCMD_SIZE32 | width(Width::Word),
        };
        if !self.begin(shape, None) {
            return false;
        }
        self.armed = Some((addr, len, true));
        true
    }

    /// Starts a memory-to-memory copy of `src` into `dst`, both SDRAM
    /// (identity-mapped, as under the bring-up map), whole cache lines and
    /// the same length: the controller as a general copy engine while the
    /// CPU does other work (the performance gate's C11). Finish with
    /// [`DmaEngine::done`]. `false` if refused (busy, the lengths differ,
    /// not whole lines, empty).
    pub fn arm_copy(&mut self, src: &[u8], dst: &mut [u8]) -> bool {
        self.arm_copy_shaped(src, dst, 32, Some(Width::Word))
    }

    /// [`Self::arm_copy`] with the burst size (`8`, `16` or `32` bytes,
    /// DCMD SIZE) and the width field (`None`: 0) chosen: the gate's C11
    /// sweep. `false` also for any other burst.
    pub fn arm_copy_shaped(
        &mut self,
        src: &[u8],
        dst: &mut [u8],
        burst: u32,
        w: Option<Width>,
    ) -> bool {
        let size = match burst {
            8 => 1 << 16,
            16 => 2 << 16,
            32 => DCMD_SIZE32,
            _ => return false,
        };
        let (s, d) = (src.as_ptr() as u32, dst.as_mut_ptr() as u32);
        let Ok(len) = u32::try_from(src.len()) else {
            return false;
        };
        if self.armed.is_some()
            || self.ring.is_some()
            || src.len() != dst.len()
            || (s | d | len) & (cpu::LINE - 1) != 0
            || len == 0
        {
            return false;
        }
        // The controller reads memory, not the cache; and lines the CPU
        // holds of the destination would overwrite what it writes.
        cpu::clean_range(s, len);
        cpu::clean_invalidate_range(d, len);
        let shape = Shape {
            src: s,
            dst: d,
            src_step: CHUNK,
            dst_step: CHUNK,
            chunk: CHUNK,
            len,
            cmd: DCMD_INCSRC | DCMD_INCTRG | size | w.map_or(0, width),
        };
        if !self.begin(shape, None) {
            return false;
        }
        self.armed = Some((d, len, false));
        true
    }

    /// Starts a memory-to-memory copy into `dst` (SDRAM, whole cache lines)
    /// from device memory the CPU doesn't cache, read `window` bytes at a
    /// time from the same physical address `src`: for a device whose data
    /// register repeats across a window (a CompactFlash card's common
    /// memory 0x400-0x7FF, docs/research/storage-speed.md), one descriptor per
    /// window. No request line: the caller starts it only when the device
    /// has the data (DRQ). Finish with [`DmaEngine::done`]. `false` if
    /// refused (busy, not whole lines, `dst` not whole windows, a window
    /// over 4 KiB, or `src` not word aligned).
    pub fn arm_copy_from_device_window(&mut self, src: u32, window: usize, dst: &mut [u8]) -> bool {
        let addr = dst.as_mut_ptr() as u32;
        let (Ok(len), Ok(window)) = (u32::try_from(dst.len()), u32::try_from(window)) else {
            return false;
        };
        if self.armed.is_some()
            || self.ring.is_some()
            || (addr | len) & (cpu::LINE - 1) != 0
            || src & 3 != 0
            || window == 0
            || window > CHUNK
            || len == 0
            || !len.is_multiple_of(window)
        {
            return false;
        }
        // Dirty lines would overwrite what the controller writes.
        cpu::clean_invalidate_range(addr, len);
        let shape = Shape {
            src,
            dst: addr,
            src_step: 0,
            dst_step: window,
            chunk: window,
            len,
            cmd: DCMD_INCSRC | DCMD_INCTRG | DCMD_SIZE32 | width(Width::Word),
        };
        if !self.begin(shape, None) {
            return false;
        }
        self.armed = Some((addr, len, false));
        true
    }

    /// Bytes the armed transfer has moved so far (diagnostics: where a
    /// paced transfer stalled), from the channel's current memory address;
    /// `None` if nothing is armed.
    pub fn moved(&mut self) -> Option<u32> {
        let (addr, len, to_device) = self.armed?;
        // DSADR (to a device) or DTADR (from one): the memory side.
        let at = read(ddadr(self.ch) + if to_device { 4 } else { 8 });
        Some(at.wrapping_sub(addr).min(len))
    }

    // One copy: `done` alone ends a transfer five ways (the loader is at
    // its size budget).
    #[inline(never)]
    fn stop(&mut self) {
        self.halt();
        if self.paced {
            write(drcmr(self.request), 0);
        }
        self.stream = None;
        if let Some((addr, len, to_device)) = self.armed.take()
            && !to_device
        {
            // What the device wrote, not stale lines.
            cpu::clean_invalidate_range(addr, len);
        }
    }
}

impl DmaEngine for Dma {
    fn arm_from_device(&mut self, port: Port, buf: &mut [u8]) -> bool {
        let (addr, len) = (buf.as_mut_ptr() as u32, buf.len());
        self.arm(port, addr, len, false)
    }

    fn arm_to_device(&mut self, port: Port, buf: &[u8]) -> bool {
        let (addr, len) = (buf.as_ptr() as u32, buf.len());
        self.arm(port, addr, len, true)
    }

    fn done(&mut self) -> Option<bool> {
        self.armed?;
        let s = read(dcsr(self.ch));
        if s & DCSR_BUSERR != 0 {
            self.stop();
            return Some(false);
        }
        if self.whole {
            // Nothing to top up and no stop short of the end: the one
            // register the pre-streaming chain read (the emulator's SD
            // benches lost 4-26% to full polls on short reads, 2026-10-15).
            if s & DCSR_STOPINTR == 0 {
                return None;
            }
            self.stop();
            return Some(true);
        }
        // DCSR first: when it says stopped, the address registers read
        // after it are final.
        let seen = self.snapshot(s & DCSR_STOPINTR != 0);
        let mut stream = self.stream?;
        let chain = self.chain();
        let base = chain.as_ptr() as u32;
        // Each new descriptor reaches memory before the link to it.
        let next = stream.poll(chain, seen, &mut |slot| {
            cpu::clean_range(base + 16 * slot as u32, 16);
        });
        self.stream = Some(stream);
        match next {
            Next::Running => None,
            Next::Resume(at) => {
                write(ddadr(self.ch), at);
                write(dcsr(self.ch), DCSR_RUN);
                None
            }
            Next::Done => {
                self.stop();
                Some(true)
            }
            Next::Failed => {
                self.stop();
                Some(false)
            }
        }
    }

    fn abort(&mut self) {
        self.stop();
    }
}

impl WindowDma for Dma {
    fn arm_from_window(&mut self, src: u32, window: usize, buf: &mut [u8]) -> bool {
        self.arm_copy_from_device_window(src, window, buf)
    }

    fn done(&mut self) -> Option<bool> {
        DmaEngine::done(self)
    }

    fn abort(&mut self) {
        DmaEngine::abort(self);
    }
}

/// The channel as [`Ring::end_at`] needs it.
struct RingChannel<'a>(&'a mut Dma);

impl dmaring::Engine for RingChannel<'_> {
    fn seen(&mut self) -> Seen {
        self.0.seen()
    }

    fn link(&mut self, slot: usize, word: u32) {
        let chain = self.0.chain();
        chain[slot][0] = word;
        cpu::clean_range(chain.as_ptr() as u32 + 16 * slot as u32, 16);
    }

    fn restart(&mut self, at: u32) {
        write(ddadr(self.0.ch), at);
        write(dcsr(self.0.ch), DCSR_RUN);
    }
}

/// Register reads a ring's start waits at most for the first piece's load
/// (a descriptor fetch: far fewer; not measured on the X51v).
const FIRST_LOAD_POLLS: u32 = 10_000;

impl DmaRing for Dma {
    fn start(&mut self, port: Port, ring: &[u8]) -> bool {
        let addr = ring.as_ptr() as u32;
        let Ok(len) = u32::try_from(ring.len()) else {
            return false;
        };
        // At least two pieces, whole lines (`dmaring::cut`): the engine can
        // be reading one part while the CPU refills another.
        if self.armed.is_some()
            || self.ring.is_some()
            || dmaring::cut(len, dma_map::descriptors(self.ch) as u32).is_none()
            || addr & (cpu::LINE - 1) != 0
        {
            return false;
        }
        cpu::clean_range(addr, len);
        self.ring = self.program_ring(port, addr, len);
        let Some(ring) = self.ring else {
            return false;
        };
        // Quality audit C56: until the first piece is loaded, `DSADR` is
        // the previous transfer's (inside this ring when it was stopped and
        // started again) and `position` would report it. The load takes a
        // descriptor fetch; bounded, and a ring that never loads is refused.
        for _ in 0..FIRST_LOAD_POLLS {
            if ring.first_loaded(self.seen()) {
                return true;
            }
        }
        self.stop();
        false
    }

    fn position(&mut self) -> Option<usize> {
        // DSADR: the source address the channel reads next.
        let at = self.ring?.offset(read(dsadr(self.ch)))?;
        Some(at as usize)
    }

    fn filled(&mut self, part: &[u8]) {
        let (at, len) = (part.as_ptr() as u32, part.len() as u32);
        if let Some(r) = self.ring
            && r.offset(at).is_some()
            && r.offset(at + len).is_some()
            && len <= r.pieces() * r.piece()
        {
            cpu::clean_range(at, len);
        }
    }

    fn stop(&mut self) {
        if self.ring.take().is_some() {
            self.halt();
            write(drcmr(self.request), 0);
        }
    }

    fn piece(&self) -> usize {
        self.ring.map_or(0, |r| r.piece() as usize)
    }

    fn stopped(&mut self) -> bool {
        self.ring.is_some_and(|r| r.stopped(self.seen()))
    }

    fn end_at(&mut self, at: usize) -> RingEnd {
        let (Some(mut ring), Ok(at)) = (self.ring, u32::try_from(at)) else {
            return RingEnd::Failed;
        };
        // The race's wait (four reads a poll): well over a piece (8 ms at
        // 44.1 kHz); it bounds only a failure.
        let r = ring.end_at(at, &mut RingChannel(self), HALT_POLLS);
        self.ring = Some(ring);
        r
    }
}
