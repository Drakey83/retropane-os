//! The PXA27x DMA controller, for device FIFOs (PXA27x Developer's Manual,
//! DMA controller chapter).
//!
//! One channel, used through [`rp_hal_api::DmaEngine`]: a transfer between a
//! device FIFO and a memory buffer, paced by the device's request line, in
//! descriptor mode (4 KiB per descriptor, so up to 256 KiB per transfer).
//! The cache maintenance the VIVT caches need is done here, so no caller
//! can forget it: buffers are cleaned (and for reads discarded) before the
//! transfer, and reads discarded again after, so the CPU sees what the
//! device wrote.
//!
//! Buffers must be whole cache lines (32-byte aligned, a multiple of 32
//! bytes long) in identity-mapped memory (the bring-up map): the DMA
//! controller sees physical addresses.

use rp_hal_api::DmaEngine;

use crate::cpu;

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

const DCSR_RUN: u32 = 1 << 31;
/// Descriptor-fetch mode is the reset default (NODESCFETCH clear).
const DCSR_BUSERR: u32 = 1 << 0;
const DCSR_STOPINTR: u32 = 1 << 3;
const DRCMR_MAPVLD: u32 = 1 << 7;
const DDADR_STOP: u32 = 1 << 0;
const DCMD_INCSRC: u32 = 1 << 31;
const DCMD_INCTRG: u32 = 1 << 30;
const DCMD_FLOWSRC: u32 = 1 << 29;
const DCMD_FLOWTRG: u32 = 1 << 28;
/// 32-byte bursts (a FIFO request's worth), byte-wide FIFO accesses.
const DCMD_SIZE32_WIDTH1: u32 = (3 << 16) | (1 << 14);

/// Bytes per descriptor (under the 8,191 the length field holds).
const CHUNK: usize = 4096;
/// Descriptors: up to 256 KiB per transfer.
const DESCRIPTORS: usize = 64;

#[repr(C, align(16))]
struct Chain([[u32; 4]; DESCRIPTORS]);

/// The descriptor chain (only one DMA user: the flag in `rt` hands out one
/// [`Dma`]).
static mut CHAIN: Chain = Chain([[0; 4]; DESCRIPTORS]);

/// The DMA channel handed out by [`crate::rt::take_dma`].
pub struct Dma {
    ch: u32,
    request: u8,
    /// The armed transfer's buffer (address, length) and direction.
    armed: Option<(u32, u32, bool)>,
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
            armed: None,
        }
    }

    fn arm(&mut self, request: u8, fifo: u32, addr: u32, len: usize, to_device: bool) -> bool {
        // Whole cache lines only: a line shared with other data could be
        // dirtied by the CPU mid-transfer and written back over what the
        // device wrote. Callers fall back to programmed I/O.
        let lines = (addr | len as u32) & (cpu::LINE - 1) == 0;
        if self.armed.is_some() || !lines || len == 0 || len > CHUNK * DESCRIPTORS {
            return false;
        }
        // Dirty lines would overwrite what the device writes when evicted;
        // for writes, memory must hold what the CPU wrote.
        cpu::clean_invalidate_range(addr, len as u32);
        // SAFETY: CHAIN is only touched by the single `Dma` (see above), and
        // never while a transfer that reads it runs (armed is None here).
        let chain = unsafe { &mut *core::ptr::addr_of_mut!(CHAIN) };
        let n = len.div_ceil(CHUNK);
        let base = chain.0.as_ptr() as u32;
        for (i, d) in chain.0.iter_mut().take(n).enumerate() {
            let at = addr + (i * CHUNK) as u32;
            let piece = (len - i * CHUNK).min(CHUNK) as u32;
            d[0] = if i + 1 == n {
                DDADR_STOP
            } else {
                base + 16 * (i as u32 + 1)
            };
            if to_device {
                d[1] = at;
                d[2] = fifo;
                d[3] = DCMD_INCSRC | DCMD_FLOWTRG | DCMD_SIZE32_WIDTH1 | piece;
            } else {
                d[1] = fifo;
                d[2] = at;
                d[3] = DCMD_INCTRG | DCMD_FLOWSRC | DCMD_SIZE32_WIDTH1 | piece;
            }
        }
        // The controller reads the chain from memory.
        cpu::clean_range(base, (16 * n) as u32);
        write(dcsr(self.ch), 0);
        write(drcmr(request), DRCMR_MAPVLD | self.ch);
        write(ddadr(self.ch), base);
        write(dcsr(self.ch), DCSR_RUN);
        self.request = request;
        self.armed = Some((addr, len as u32, to_device));
        true
    }

    fn stop(&mut self) {
        write(dcsr(self.ch), 0);
        write(drcmr(self.request), 0);
        if let Some((addr, len, to_device)) = self.armed.take()
            && !to_device
        {
            // What the device wrote, not stale lines.
            cpu::clean_invalidate_range(addr, len);
        }
    }
}

impl DmaEngine for Dma {
    fn arm_from_device(&mut self, request: u8, fifo: u32, buf: &mut [u8]) -> bool {
        let (addr, len) = (buf.as_mut_ptr() as u32, buf.len());
        self.arm(request, fifo, addr, len, false)
    }

    fn arm_to_device(&mut self, request: u8, fifo: u32, buf: &[u8]) -> bool {
        let (addr, len) = (buf.as_ptr() as u32, buf.len());
        self.arm(request, fifo, addr, len, true)
    }

    fn done(&mut self) -> Option<bool> {
        self.armed?;
        let s = read(dcsr(self.ch));
        let ok = if s & DCSR_BUSERR != 0 {
            false
        } else if s & DCSR_STOPINTR != 0 {
            true
        } else {
            return None;
        };
        self.stop();
        Some(ok)
    }

    fn abort(&mut self) {
        self.stop();
    }
}
