//! Step 4f, the TA fed by DMA (docs/design/ref-rp.md "Step 4f"): a frame's
//! words go to the tile accelerator as one descriptor chain while the CPU
//! makes the next frame.
//!
//! Iterate, don't imitate (rule 0). Dell's GLES driver writes every word
//! with the CPU and lets a full FIFO stall the bus; P4's `rp-tabench` sent
//! DMA pieces no larger than the FIFO's room, the CPU polling between them,
//! which run 52's probe measured dearer than the CPU on the game's frames
//! (9.2 ms against 5.0: a piece a run). Run 52 also showed the TA takes the
//! game's runs by DMA with no room check (16 of 16 pictures exact, the FIFO
//! full without a hang) and the CPU keeps ~65% of its memory speed while
//! the DMA writes. So here a frame is **one unpaced chain**, armed once:
//! the frame's own words where they lie (no copy), the words the TA wants
//! that are not in them (a strip's word without its count, each strip's
//! end, the scene's end and its grid word) from a small table of constants,
//! so the TA says done with no CPU at all. The render is started by a pump
//! the program's clock reads drive (`Screen::pump`), and the program keeps
//! a frame's words until the next frame call (`RP_GFX_KEEP`: it writes
//! the next frame into its other buffer).

use rp_drv_blit2d::{MBX, reg};
use rp_drv_mbx3d::ta;
use rp_hal_api::{DmaEngine, RegisterBus};
use rp_hal_pxa27x::dma::{CHAIN_MOST, Dma, chain_descriptor};
use rp_hal_pxa27x::{Mmio, cpu, rt};
use rp_port::gfx::{FAN, NO_Z, STATE, STATE2, STATE2_WORDS, STATE_WORDS, STRIP, corner_words_in};

use super::us_since;

/// The 2700G's register window as the DMA sees it (CS3 + `MBX_REG_OFFSET`).
const REGS_PHYS: u32 = 0x0C00_0000 + rp_hal_x51v::MBX_REG_OFFSET;
/// The TA table's grid word (`ta_feed`'s `GRID`): the scene's end bit goes
/// on it while the scene's end word goes in.
const GRID: usize = 0x85C;
/// Words of the table of constants at the memory's start (a whole number
/// of cache lines).
const TABLE: usize = 16;
/// Kit 63: the TA's start table's values (at most 16), then the
/// background strip's words (at most 32), then a block of zeros the
/// chain clears the TA's tail buffer and region array from; the
/// descriptors after them (16-byte aligned).
const T_START: usize = TABLE;
const T_BG: usize = T_START + 16;
const BG_MOST: usize = 32;
const ZERO: usize = T_BG + BG_MOST;
const ZERO_WORDS: usize = 2048;
/// Bytes a clear's descriptor moves: whole 32-byte bursts, under
/// [`MOST`].
const ZERO_CHUNK: u32 = 8160;
const DESC: usize = ZERO + ZERO_WORDS;
const _: () = assert!(DESC % 4 == 0 && ZERO_CHUNK as usize <= 4 * ZERO_WORDS);
/// The 2700G's local memory as the DMA sees it (CS3 from 0).
const VRAM_PHYS: u32 = 0x0C00_0000;
const T_STRIP: usize = 0;
const T_FAN: usize = 1;
const T_ONE: usize = 2;
const T_END: usize = 3;
const T_GRID: usize = 4;
/// The no-z strip and fan headers (`NO_Z`, ADR-0040 step 7g7).
const T_STRIP_NO_Z: usize = 5;
const T_FAN_NO_Z: usize = 6;
/// Bytes of a descriptor's run into the TA window, at most (whole words).
const MOST: u32 = CHAIN_MOST & !3;
/// A chain not done in this long is stopped (a frame's ~20,000 words
/// take a few milliseconds on the bus).
const CHAIN_LIMIT_US: u32 = 200_000;

/// Bytes of the memory the chain feed takes (its table and descriptors):
/// 12,284 descriptors, ~4 a strip; a frame needing more goes by the CPU.
pub const CHAIN_BYTES: usize = 192 * 1024;

/// A frame in flight: when it was armed, whether the DMA has finished,
/// and the registers to put back once the TA said done.
#[derive(Clone, Copy)]
struct Flight {
    t0: u32,
    dma_done: bool,
    en: u32,
    grid: u32,
}

/// What [`Chain::settle`] and [`Chain::pump`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Settled {
    /// Nothing was in flight.
    Idle,
    /// The TA said done: the render may start. Microseconds from the arm.
    Done(u32),
    /// The chain or the TA did not finish (or the DMA stopped on a bus
    /// error): the core must be reset.
    Failed { dma: bool },
}

/// The chain feed: its DMA channel, its memory and the frame in flight.
pub struct Chain {
    dma: Dma,
    mem: &'static mut [u32],
    flight: Option<Flight>,
    /// Descriptors written for the frame being built.
    n: usize,
    /// The window offsets (bytes) the next words and the next end go to.
    at: usize,
    ends: usize,
    /// Descriptors the last frame took.
    pub last_descriptors: u32,
}

impl Chain {
    /// The feed on `dma`, its table and descriptors in `mem` (cached RAM,
    /// at least [`CHAIN_BYTES`]); `None` if `mem` is too small or not on a
    /// cache line.
    pub fn new(dma: Dma, mem: &'static mut [u8]) -> Option<Self> {
        let words = rp_hal_mem::bytes_as_words_mut(mem.get_mut(..CHAIN_BYTES)?)?;
        if words.as_ptr().addr() % cpu::LINE as usize != 0 {
            return None;
        }
        words[T_STRIP] = STRIP;
        words[T_FAN] = FAN;
        words[T_STRIP_NO_Z] = STRIP | NO_Z;
        words[T_FAN_NO_Z] = FAN | NO_Z;
        words[T_ONE] = 1;
        words[T_END] = rp_drv_mbx3d::END;
        words[ZERO..DESC].fill(0);
        // the zeros reach memory once (never written again)
        cpu::clean_range(
            words.as_ptr().addr() as u32 + 4 * ZERO as u32,
            4 * ZERO_WORDS as u32,
        );
        Some(Self {
            dma,
            mem: words,
            flight: None,
            n: 0,
            at: 0,
            ends: 0,
            last_descriptors: 0,
        })
    }

    /// Whether a frame is in flight.
    pub const fn busy(&self) -> bool {
        self.flight.is_some()
    }

    fn addr(&self, word: usize) -> u32 {
        self.mem.as_ptr().addr() as u32 + 4 * word as u32
    }

    /// One descriptor (linked to the next slot; the last is ended at the
    /// arm). False if the memory is full.
    fn push(&mut self, src: u32, dst: u32, len: u32) -> bool {
        let slot = DESC + 4 * self.n;
        if slot + 8 > self.mem.len() {
            return false;
        }
        let next = self.addr(slot + 4);
        self.mem[slot..slot + 4].copy_from_slice(&chain_descriptor(Some(next), src, dst, len));
        self.n += 1;
        true
    }

    /// `bytes` from `src` into the TA window, cut at the window's wrap.
    fn to_window(&mut self, mut src: u32, mut bytes: u32) -> bool {
        while bytes > 0 {
            let room = (ta::PORT_SIZE - self.at) as u32;
            let k = bytes.min(room).min(MOST);
            if !self.push(src, REGS_PHYS + (ta::PORT + self.at) as u32, k) {
                return false;
            }
            self.at = (self.at + k as usize) % ta::PORT_SIZE;
            src += k;
            bytes -= k;
        }
        true
    }

    /// A strip's end: a `1` into the end window.
    fn end(&mut self) -> bool {
        let src = self.addr(T_ONE);
        let ok = self.push(src, REGS_PHYS + (ta::END_PORT + self.ends) as u32, 4);
        self.ends = (self.ends + 4) % ta::PORT_SIZE;
        ok
    }

    /// The chain for `parts` (checked frames: state blocks, strips and fans
    /// only), after the words the CPU already put at the window offsets
    /// `at` and `ends`, ending with the scene's end. False if it does not
    /// fit (the frame then goes by the CPU).
    fn build(
        &mut self,
        parts: &[&[u32]],
        table: &[(usize, u32)],
        bg: &[u32],
        clears: &[(u32, u32)],
        grid: u32,
    ) -> bool {
        self.n = 0;
        self.at = 0;
        self.ends = 0;
        self.mem[T_GRID] = grid | ta::SCENE_END;
        if table.len() > 16 || bg.len() > BG_MOST {
            return false;
        }
        // Kit 63: the scene's start in the chain, in the CPU's old order:
        // the TA's tail buffer and region array cleared (2700G memory),
        // its start table, the background strip and its end.
        let zero = self.addr(ZERO);
        for &(mut at, mut bytes) in clears {
            while bytes > 0 {
                let k = bytes.min(ZERO_CHUNK);
                if !self.push(zero, VRAM_PHYS + at, k) {
                    return false;
                }
                at += k;
                bytes -= k;
            }
        }
        for (i, &(off, v)) in table.iter().enumerate() {
            self.mem[T_START + i] = v;
            if !self.push(self.addr(T_START + i), REGS_PHYS + (MBX + off) as u32, 4) {
                return false;
            }
        }
        self.mem[T_BG..T_BG + bg.len()].copy_from_slice(bg);
        if !(self.to_window(self.addr(T_BG), 4 * bg.len() as u32) && self.end()) {
            return false;
        }
        let mut two = false;
        for words in parts {
            let base = words.as_ptr().addr() as u32;
            let mut k = 0;
            while k < words.len() {
                let w = words[k];
                let src = base + 4 * k as u32;
                if w == STATE || w == STATE2 {
                    // kit 112: a two-texture block's corners carry u1, v1
                    let sw = if w == STATE2 { STATE2_WORDS } else { STATE_WORDS };
                    two = w == STATE2;
                    if !self.to_window(src, 4 * (1 + sw) as u32) {
                        return false;
                    }
                    k += 1 + sw;
                } else {
                    let n = (w & 0xFF) as usize;
                    let fan = w & !NO_Z & 0xFFFF_FF00 == FAN;
                    let head = match (fan, w & NO_Z != 0) {
                        (true, false) => T_FAN,
                        (true, true) => T_FAN_NO_Z,
                        (false, false) => T_STRIP,
                        (false, true) => T_STRIP_NO_Z,
                    };
                    let cw = corner_words_in(w, two);
                    let ok = self.to_window(self.addr(head), 4)
                        && self.to_window(src + 4, (4 * n * cw) as u32)
                        && self.end();
                    if !ok {
                        return false;
                    }
                    k += 1 + n * cw;
                }
            }
        }
        let (g, e) = (self.addr(T_GRID), self.addr(T_END));
        if !(self.push(g, REGS_PHYS + (MBX + GRID) as u32, 4) && self.to_window(e, 4)) {
            return false;
        }
        // The last descriptor ends the chain.
        let last = DESC + 4 * (self.n - 1);
        self.mem[last] = rp_hal_pxa27x::dma::CHAIN_STOP;
        true
    }

    /// The whole frame as one chain armed (kit 63): the TA's tail buffer
    /// and region array cleared in the 2700G's memory, its start table,
    /// `bg` (the background strip), then `parts`, as `ta::clear`, the
    /// table's writes and the background did by the CPU before (Dell's
    /// way: ~21,000 single stores across the bus a frame, ~2.5 ms of
    /// CPU). `Ok(true)`: the frame in flight. `Ok(false)`: the chain does
    /// not fit, nothing started (the frame goes by the CPU, which clears
    /// for itself). `Err(())`: the DMA refused the chain (the caller
    /// resets the core; the frame is not drawn).
    pub fn start(
        &mut self,
        regs: &mut Mmio,
        l: &ta::Layout,
        bg: &[u32],
        parts: &[&[u32]],
    ) -> Result<bool, ()> {
        let table = l.start_table();
        let grid = table.iter().find(|p| p.0 == GRID).map_or(0, |p| p.1);
        let (cols, rows) = l.grid();
        let clears = [(l.tail, l.tail_bytes), (l.regions, 8 * (cols * rows + 1))];
        if !self.build(parts, &table, bg, &clears, grid) {
            return Ok(false);
        }
        // The frame's words reach memory before the DMA reads them, and
        // the table of constants (the descriptors are cleaned by the arm).
        for words in parts {
            cpu::clean_range(words.as_ptr().addr() as u32, 4 * words.len() as u32);
        }
        cpu::clean_range(self.addr(0), 4 * ZERO as u32);
        // The done bit armed by the CPU (its own registers, not the TA's
        // memory); the rest of the scene's start is the chain's.
        let en = regs.read32(MBX + reg::ENABLE);
        regs.write32(MBX + reg::CLEAR, ta::DONE);
        regs.write32(MBX + reg::ENABLE, en | ta::DONE);
        let t0 = rt::ticks();
        let n = 4 * self.n;
        self.last_descriptors = self.n as u32;
        // The arm drains the CPU's writes first (its clean).
        if !self.dma.arm_chain(&self.mem[DESC..DESC + n]) {
            Self::finish(
                regs,
                Flight {
                    t0,
                    dma_done: true,
                    en,
                    grid,
                },
            );
            return Err(());
        }
        self.flight = Some(Flight {
            t0,
            dma_done: false,
            en,
            grid,
        });
        Ok(true)
    }

    /// The registers put back once the scene ended.
    fn finish(regs: &mut Mmio, f: Flight) {
        regs.write32(MBX + reg::CLEAR, ta::DONE);
        regs.write32(MBX + reg::ENABLE, f.en);
        regs.write32(MBX + GRID, f.grid);
    }

    /// One look at the DMA: false if it stopped on a bus error.
    fn look_dma(&mut self, f: &mut Flight) -> bool {
        if !f.dma_done {
            match self.dma.done() {
                None => {}
                Some(ok) => {
                    f.dma_done = true;
                    return ok;
                }
            }
        }
        true
    }

    /// A look, never a wait (the clock reads' pump): whether the frame in
    /// flight is done (the chain stopped and the TA said done); past the
    /// limits it is settled as failed.
    pub fn pump(&mut self, regs: &mut Mmio) -> Settled {
        let Some(mut f) = self.flight else {
            return Settled::Idle;
        };
        if !self.look_dma(&mut f) {
            self.flight = None;
            Self::finish(regs, f);
            return Settled::Failed { dma: true };
        }
        self.flight = Some(f);
        if !f.dma_done {
            return if us_since(f.t0) > CHAIN_LIMIT_US {
                self.settle(regs)
            } else {
                Settled::Idle
            };
        }
        if regs.read32(MBX + reg::STATUS) & ta::DONE == 0 {
            return if us_since(f.t0) > super::TA_LIMIT_US {
                self.settle(regs)
            } else {
                Settled::Idle
            };
        }
        self.flight = None;
        Self::finish(regs, f);
        Settled::Done(us_since(f.t0))
    }

    /// Waits for the frame in flight to end, up to the limits.
    pub fn settle(&mut self, regs: &mut Mmio) -> Settled {
        let Some(mut f) = self.flight.take() else {
            return Settled::Idle;
        };
        let dma_ok = loop {
            if !self.look_dma(&mut f) {
                break false;
            }
            if f.dma_done {
                break true;
            }
            if us_since(f.t0) > CHAIN_LIMIT_US {
                self.dma.abort();
                break false;
            }
        };
        let done = dma_ok
            && loop {
                if regs.read32(MBX + reg::STATUS) & ta::DONE != 0 {
                    break true;
                }
                if us_since(f.t0) > super::TA_LIMIT_US {
                    break false;
                }
            };
        Self::finish(regs, f);
        if done {
            Settled::Done(us_since(f.t0))
        } else {
            Settled::Failed { dma: !dma_ok }
        }
    }
}
