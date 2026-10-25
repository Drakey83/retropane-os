//! Feeding the tile accelerator (P4 step 1, docs/design/p4-ta-speed.md):
//! `rp-glesfeat`'s proven `through_ta` steps as one driver function, for a
//! stream of any length.
//!
//! Dell's GLES driver writes every word into the TA window with the CPU and
//! leaves a long batch to stall the bus when the FIFO is full. Ours sends
//! the stream in pieces as large as the FIFO's room, read from the status
//! register once a piece (a credit, not a check a word: each read crosses
//! the same 30 MB/s bus as the words), and a FIFO that stays full ends the
//! scene with its status instead of hanging the CPU on a write. The FIFO is
//! the 2D engine's (ADR-0026 T8, still to be proven on the device): the
//! level each piece saw is logged in [`Fed`], the T8 evidence P4's run reads.

use rp_drv_blit2d::{FIFO_WORDS, MBX, free_bytes, reg};
use rp_hal_api::RegisterBus;
use rp_hal_mem::store32_volatile;

use super::ta::{DONE, END_PORT, PORT, PORT_SIZE, SCENE_END, start_table};

/// `0x85C`, the TA table's grid word: [`SCENE_END`] goes on it while the
/// scene's end word goes in.
const GRID: usize = 0x85C;

/// Where the TA works in 2700G local memory, and the target it bins for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    /// The tail buffer ([`start_table`]'s `tail`: 128 KiB aligned, 4 bytes
    /// a tile).
    pub tail: u32,
    /// Bytes of the tail buffer zeroed before each scene.
    pub tail_bytes: u32,
    /// The region array the TA writes, (tiles + 1) x 8 bytes; the 3D
    /// stage's start table points `0x608` at it.
    pub regions: u32,
    /// The target's size in pixels.
    pub width: u32,
    /// See [`Self::width`].
    pub height: u32,
}

impl Layout {
    /// The tile grid, [`super::TILE_W`] x [`super::TILE_H`] tiles.
    #[must_use]
    pub const fn grid(&self) -> (u32, u32) {
        (self.width / super::TILE_W, self.height / super::TILE_H)
    }

    /// The TA's start table for this layout.
    #[must_use]
    pub fn start_table(&self) -> [(usize, u32); 9] {
        let (cols, rows) = self.grid();
        start_table(self.tail, self.regions, self.width, self.height, cols, rows)
    }
}

/// Zeroes the tail buffer and the region array in `vram` (2700G local
/// memory from 0) before a scene; false if either runs past `vram`. The
/// caller drains the CPU's write buffer before [`feed`].
pub fn clear(vram: &mut [u8], l: &Layout) -> bool {
    let (cols, rows) = l.grid();
    let tail = (0..l.tail_bytes as usize / 4).map(|k| l.tail as usize + 4 * k);
    let regions = (0..2 * (cols * rows + 1) as usize).map(|k| l.regions as usize + 4 * k);
    tail.chain(regions)
        .all(|at| store32_volatile(vram, at, 0).is_some())
}

/// What [`feed`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Fed {
    /// The TA said done.
    pub done: bool,
    /// The FIFO stayed full past the limit: the status then, the stream
    /// cut short there and no scene end sent.
    pub stuck: Option<u32>,
    /// Words written into the TA window and its end window.
    pub words: u32,
    /// Status reads made for room (one a piece, more while full).
    pub level_reads: u32,
    /// The most words the FIFO level showed queued.
    pub level_max: u32,
    /// Microseconds from the first register write to the scene's end word
    /// in: the CPU's feeding.
    pub feed_us: u32,
    /// Microseconds from the first register write to the done bit (0 if it
    /// never came).
    pub ta_us: u32,
}

/// The clock [`feed`] times with: a free-running wrapping tick counter and
/// its rate, as [`rp_drv_blit2d::Engine`] takes it.
#[derive(Clone, Copy)]
pub struct Clock {
    /// The tick count now.
    pub now: fn() -> u32,
    /// Ticks a millisecond.
    pub ticks_per_ms: u32,
}

impl Clock {
    fn us_since(&self, t0: u32) -> u32 {
        let t = u64::from((self.now)().wrapping_sub(t0));
        (t * 1000 / u64::from(self.ticks_per_ms.max(1))) as u32
    }
}

/// `draws` through the tile accelerator on `regs` (the 2700G register
/// window): the done bit enabled, the start table, each draw into the TA
/// window followed by a `1` into the end window (each window's offset
/// counting on and wrapping at [`PORT_SIZE`], as Dell's driver counts), the
/// scene's end, then the done bit polled for up to `limit_us` (ADR-0026 D3:
/// no interrupt from the 3D core). Every word waits for room in the FIFO;
/// room that doesn't come within `limit_us` ends the stream ([`Fed::stuck`]).
/// [`clear`] first. The 2D engine idle meanwhile (ADR-0026 T8: one FIFO).
pub fn feed<B: RegisterBus>(
    regs: &mut B,
    l: &Layout,
    draws: &[&[u32]],
    clock: Clock,
    limit_us: u32,
) -> Fed {
    let mut fed = Fed::default();
    let en = regs.read32(MBX + reg::ENABLE);
    regs.write32(MBX + reg::CLEAR, DONE);
    regs.write32(MBX + reg::ENABLE, en | DONE);
    let t0 = (clock.now)();
    let table = l.start_table();
    for &(off, v) in &table {
        regs.write32(MBX + off, v);
    }
    let grid = table.iter().find(|p| p.0 == GRID).map_or(0, |p| p.1);
    let mut sink = Sink {
        regs,
        clock,
        limit_us,
        credit: 0,
        fed: &mut fed,
    };
    let (mut at, mut ends) = (0usize, 0usize);
    let sent = draws.iter().all(|d| {
        d.iter().all(|&v| {
            let ok = sink.put(PORT + at, v);
            at = (at + 4) % PORT_SIZE;
            ok
        }) && {
            let ok = sink.put(END_PORT + ends, 1);
            ends = (ends + 4) % PORT_SIZE;
            ok
        }
    }) && {
        sink.regs.write32(MBX + GRID, grid | SCENE_END);
        sink.put(PORT + at, super::END)
    };
    fed.feed_us = clock.us_since(t0);
    if sent {
        while clock.us_since(t0) < limit_us {
            if regs.read32(MBX + reg::STATUS) & DONE != 0 {
                fed.done = true;
                fed.ta_us = clock.us_since(t0);
                break;
            }
        }
    }
    regs.write32(MBX + reg::CLEAR, DONE);
    regs.write32(MBX + reg::ENABLE, en);
    regs.write32(MBX + GRID, grid);
    fed
}

/// The FIFO's writer: words go in while the credit (free words at the last
/// status read) lasts.
struct Sink<'a, B> {
    regs: &'a mut B,
    clock: Clock,
    limit_us: u32,
    credit: u32,
    fed: &'a mut Fed,
}

impl<B: RegisterBus> Sink<'_, B> {
    /// Writes `v` to `offset` once the FIFO has room; false (and
    /// [`Fed::stuck`]) if it stayed full for the limit.
    fn put(&mut self, offset: usize, v: u32) -> bool {
        if self.credit == 0 {
            let t0 = (self.clock.now)();
            loop {
                let s = self.regs.read32(MBX + reg::STATUS);
                self.fed.level_reads += 1;
                let free = free_bytes(s) / 4;
                self.fed.level_max = self.fed.level_max.max(FIFO_WORDS - free);
                if free != 0 {
                    self.credit = free;
                    break;
                }
                if self.clock.us_since(t0) > self.limit_us {
                    self.fed.stuck = Some(s);
                    return false;
                }
            }
        }
        self.regs.write32(offset, v);
        self.credit -= 1;
        self.fed.words += 1;
        true
    }
}
