//! R340's probe (`-rpprobe`): a 2700G register that counts a render's
//! progress, searched for in the game.
//!
//! Iterate, don't imitate (rule 0). A safe overlap of the TA and a render
//! needs to know how far the render in flight has got (the frame log,
//! runs 355-356: a render can fall to a quarter of the one before it, and
//! a TA can outrun the shortest render, so no timing rule is certain).
//! Dell's queue never overlaps; PowerVR's drivers know their chip's
//! registers; nothing published names one for the MBX in the 2700G. Ours
//! reads the core's register block while a render runs, the CPU waiting
//! through a few renders in strict order (nothing else on the chip):
//! snapshots ~2 ms apart from the render's start to its end. Reads only:
//! the TA's start (`0x800`) and the interrupt clear (`0x134`) left alone;
//! a register whose read ends in the bus's time-out is never read again,
//! and three such reads end the probe. The PC looks for a word that moves
//! steadily from the start to the end of each render (`gfx-probe.bin`).

use rp_drv_blit2d::MBX;
use rp_hal_api::RegisterBus;
use rp_hal_pxa27x::{Mmio, rt};
use rp_port::gfx::bus_timed_out;

/// Words read: `0x000`-`0x7FC`, then the TA block `0x804`-`0x85C`.
const LOW_WORDS: usize = 512;
const TA_AT: usize = 0x804;
const TA_WORDS: usize = 23;
pub const WORDS: usize = LOW_WORDS + TA_WORDS;
/// A snapshot: render number, microseconds since its start, 1 when the
/// render was seen done, reads that timed out so far; then the words.
const HEAD_WORDS: usize = 4;
const SNAP_WORDS: usize = HEAD_WORDS + WORDS;
/// Renders probed and snapshots a render at most.
pub const RENDERS: usize = 24;
const PER_RENDER: usize = 7;
const SNAPS: usize = RENDERS * PER_RENDER;
const MAGIC: &[u8; 8] = b"RPPROBE1";
const HEAD: usize = 16;
pub const ROOM: usize = HEAD + SNAPS * SNAP_WORDS * 4;
pub const FILE: &str = "gfx-probe.bin";
/// The frames the probe starts at (the fight's middle).
pub const FROM_FRAME: u32 = 400;
/// Between snapshots (six cover ~21 ms of a ~23 ms render; the last is
/// at its end).
const GAP_US: u32 = 3_500;
/// A render watched this long at most.
const LIMIT_US: u32 = 80_000;

pub struct RegProbe {
    mem: &'static mut [u8],
    pub on: bool,
    renders: usize,
    snaps: usize,
    slow: u32,
    skip: [u32; WORDS.div_ceil(32)],
}

fn offset(i: usize) -> usize {
    if i < LOW_WORDS {
        4 * i
    } else {
        TA_AT + 4 * (i - LOW_WORDS)
    }
}

impl RegProbe {
    pub fn new(mem: &'static mut [u8]) -> Option<Self> {
        let mem = mem.get_mut(..ROOM)?;
        mem[..HEAD].fill(0);
        mem[..8].copy_from_slice(MAGIC);
        mem[12..16].copy_from_slice(&(WORDS as u32).to_le_bytes());
        let mut skip = [0u32; WORDS.div_ceil(32)];
        for off in [0x800usize, 0x134] {
            if off < 4 * LOW_WORDS {
                skip[off / 4 / 32] |= 1 << (off / 4 % 32);
            }
        }
        Some(Self {
            mem,
            on: false,
            renders: 0,
            snaps: 0,
            slow: 0,
            skip,
        })
    }

    fn put(&mut self, k: usize, v: u32) {
        let a = HEAD + 4 * (self.snaps * SNAP_WORDS + k);
        self.mem[a..a + 4].copy_from_slice(&v.to_le_bytes());
    }

    fn snap(&mut self, regs: &mut Mmio, n: u32, us: u32, done: u32) {
        if self.snaps >= SNAPS {
            return;
        }
        for i in 0..WORDS {
            let v = if self.skip[i / 32] & (1 << (i % 32)) != 0 || self.slow >= 3 {
                0xDEAD_BEEF
            } else {
                let t0 = rt::ticks();
                let v = regs.read32(MBX + offset(i));
                if bus_timed_out(rt::ticks().wrapping_sub(t0)) {
                    self.skip[i / 32] |= 1 << (i % 32);
                    self.slow += 1;
                }
                v
            };
            self.put(HEAD_WORDS + i, v);
        }
        self.put(0, n);
        self.put(1, us);
        self.put(2, done);
        self.put(3, self.slow);
        self.snaps += 1;
    }

    /// Whether frame `frame`'s render (just started) is one to probe.
    pub fn wants(&self, frame: u32) -> bool {
        self.on && frame >= FROM_FRAME && self.renders < RENDERS && self.slow < 3
    }

    /// The render just started, watched to its end: `done` reads the
    /// status (the render's done bit, never cleared here).
    pub fn watch(&mut self, regs: &mut Mmio, done: impl Fn(&mut Mmio) -> bool) {
        let n = self.renders as u32;
        self.renders += 1;
        let t0 = rt::ticks();
        let us = || rt::ticks().wrapping_sub(t0) / 13 * 4;
        for _ in 0..PER_RENDER - 1 {
            let d = done(regs);
            self.snap(regs, n, us(), u32::from(d));
            if d {
                return;
            }
            let next = us() + GAP_US;
            while us() < next && !done(regs) {}
        }
        while us() < LIMIT_US && !done(regs) {}
        let d = done(regs);
        self.snap(regs, n, us(), u32::from(d));
    }

    pub fn bytes(&mut self) -> &[u8] {
        let n = self.snaps as u32;
        self.mem[8..12].copy_from_slice(&n.to_le_bytes());
        &self.mem[..HEAD + self.snaps * SNAP_WORDS * 4]
    }
}
