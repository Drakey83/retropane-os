//! The PXA27x DMA controller, for device FIFOs (PXA27x Developer's Manual,
//! DMA controller chapter).
//!
//! Channels 0 and 1, each through [`rp_hal_api::DmaEngine`] (a transfer
//! between a device FIFO and a memory buffer, paced by the device's request
//! line, in descriptor mode: 4 KiB per descriptor, any length, streamed
//! through a 32-slot ring by [`crate::dmachain`] as [`DmaEngine::done`] is
//! polled) or
//! [`rp_hal_api::DmaRing`] (a buffer looped into a device without end, for
//! audio: the last descriptor points back at the first).
//! The cache maintenance the VIVT caches need is done here, so no caller
//! can forget it: buffers are cleaned (and for reads discarded) before the
//! transfer, and reads discarded again after, so the CPU sees what the
//! device wrote.
//!
//! Buffers must be whole cache lines (32-byte aligned, a multiple of 32
//! bytes long) in identity-mapped memory (the bring-up map): the DMA
//! controller sees physical addresses.

use rp_hal_api::{DmaEngine, DmaRing, Port, Width};

use crate::cpu;
use crate::dmachain::{Next, Seen, Shape, Stream};

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

/// Channels handed out ([`crate::rt::take_dma`]): each has its own chain.
pub const CHANNELS: u32 = 2;

/// Bytes per descriptor (under the 8,191 the length field holds).
const CHUNK: u32 = 4096;
/// Descriptor slots per channel (512 bytes): an audio ring, or 128 KiB
/// ahead of the controller for a streamed transfer of any length.
const DESCRIPTORS: usize = 32;

#[repr(C, align(16))]
struct Chain([[u32; 4]; DESCRIPTORS]);

/// A descriptor chain per channel (the flags in `rt` hand each channel out
/// once, so each chain has one user).
static mut CHAINS: [Chain; CHANNELS as usize] =
    [const { Chain([[0; 4]; DESCRIPTORS]) }; CHANNELS as usize];

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
    /// The running ring: its address and length.
    ring: Option<(u32, u32)>,
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
            ring: None,
        }
    }

    /// This channel's descriptor chain.
    fn chain(&mut self) -> &'static mut Chain {
        // SAFETY: each channel's chain is touched only by its one `Dma`
        // (handed out once by `rt::take_dma`). While the controller runs,
        // only slots it has finished with are written (`dmachain`); the
        // ring's slots are written only while it is stopped.
        unsafe { &mut (*core::ptr::addr_of_mut!(CHAINS))[self.ch as usize] }
    }

    /// Fills the chain with a ring of `len` bytes (whole chunks, at most
    /// the chain) at `addr` looped into `port`, and starts the channel.
    fn program_ring(&mut self, port: Port, addr: u32, len: u32) {
        let chain = self.chain();
        let n = len / CHUNK;
        let base = chain.0.as_ptr() as u32;
        for (i, d) in (0..n).zip(chain.0.iter_mut()) {
            *d = [
                if i + 1 < n { base + 16 * (i + 1) } else { base },
                addr + i * CHUNK,
                port.addr,
                DCMD_INCSRC | DCMD_FLOWTRG | DCMD_SIZE32 | width(port.width) | CHUNK,
            ];
        }
        // The controller reads the chain from memory.
        cpu::clean_range(base, 16 * n);
        write(dcsr(self.ch), 0);
        write(drcmr(port.request), DRCMR_MAPVLD | self.ch);
        write(ddadr(self.ch), base);
        write(dcsr(self.ch), DCSR_RUN);
        self.request = port.request;
        self.paced = true;
    }

    /// Starts streaming `shape`: the first descriptors written, the chain
    /// cleaned, the channel run from its first slot, paced by `request` or
    /// memory to memory. `false` if the shape is refused.
    fn begin(&mut self, shape: Shape, request: Option<u8>) -> bool {
        let chain = self.chain();
        let base = chain.0.as_ptr() as u32;
        let Some(stream) = Stream::start(shape, &mut chain.0, base) else {
            return false;
        };
        // The controller reads the chain from memory.
        cpu::clean_range(base, (16 * DESCRIPTORS) as u32);
        write(dcsr(self.ch), 0);
        if let Some(r) = request {
            write(drcmr(r), DRCMR_MAPVLD | self.ch);
            self.request = r;
        }
        write(ddadr(self.ch), base);
        write(dcsr(self.ch), DCSR_RUN);
        self.paced = request.is_some();
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

    fn stop(&mut self) {
        write(dcsr(self.ch), 0);
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
        // DCSR first: when it says stopped, the address registers read
        // after it are final.
        let seen = Seen {
            stopped: s & DCSR_STOPINTR != 0,
            ddadr: read(ddadr(self.ch)),
            dsadr: read(dsadr(self.ch)),
            dtadr: read(dtadr(self.ch)),
        };
        let mut stream = self.stream?;
        let chain = self.chain();
        let base = chain.0.as_ptr() as u32;
        // Each new descriptor reaches memory before the link to it.
        let next = stream.poll(&mut chain.0, seen, &mut |slot| {
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

impl DmaRing for Dma {
    fn start(&mut self, port: Port, ring: &[u8]) -> bool {
        let addr = ring.as_ptr() as u32;
        let Ok(len) = u32::try_from(ring.len()) else {
            return false;
        };
        // At least two descriptors, all whole: the engine can be reading
        // one part while the CPU refills another.
        let ok = len >= 2 * CHUNK && len.is_multiple_of(CHUNK) && len <= CHUNK * DESCRIPTORS as u32;
        if self.armed.is_some() || self.ring.is_some() || !ok || addr & (cpu::LINE - 1) != 0 {
            return false;
        }
        cpu::clean_range(addr, len);
        self.program_ring(port, addr, len);
        self.ring = Some((addr, len));
        true
    }

    fn position(&mut self) -> Option<usize> {
        let (addr, len) = self.ring?;
        // DSADR: the source address the channel reads next.
        let at = read(ddadr(self.ch) + 4).wrapping_sub(addr);
        Some((at.min(len.saturating_sub(1))) as usize)
    }

    fn filled(&mut self, part: &[u8]) {
        let (at, len) = (part.as_ptr() as u32, part.len() as u32);
        if let Some((addr, size)) = self.ring
            && at >= addr
            && at - addr + len <= size
        {
            cpu::clean_range(at, len);
        }
    }

    fn stop(&mut self) {
        if self.ring.take().is_some() {
            write(dcsr(self.ch), 0);
            write(drcmr(self.request), 0);
        }
    }
}
