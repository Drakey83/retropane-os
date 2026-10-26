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

/// One item of a TA stream: a word into the TA window, or the end of a
/// primitive (a `1` into the end window).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    /// A word into the TA window.
    Word(u32),
    /// The primitive ends: a `1` into the end window.
    End,
}

/// `draws` as a stream: each draw's words, then [`Item::End`] (the form
/// `rp-glesfeat`'s `through_ta` sent).
pub fn draws<'a>(draws: &'a [&'a [u32]]) -> impl Iterator<Item = Item> + 'a {
    draws.iter().flat_map(|d| {
        d.iter()
            .map(|&w| Item::Word(w))
            .chain(core::iter::once(Item::End))
    })
}

/// `stream` through the tile accelerator on `regs` (the 2700G register
/// window): [`Feed::start`], each item [`Feed::put`], [`Feed::finish`].
/// The stream may be built as it goes (an iterator over quads): the CPU's
/// words go straight into the window, with no copy of the frame in memory.
/// [`clear`] first. The 2D engine idle meanwhile (ADR-0026 T8: one FIFO).
pub fn feed<B: RegisterBus>(
    regs: &mut B,
    ready: &super::ta_setup::Ready,
    l: &Layout,
    stream: impl IntoIterator<Item = Item>,
    clock: Clock,
    limit_us: u32,
) -> Fed {
    let mut f = Feed::start(regs, ready, l, clock, limit_us);
    let _ = stream.into_iter().all(|item| f.put(item));
    f.finish()
}

/// One scene into the TA, a piece at a time: the CPU's items by
/// [`Self::put`], or another writer's words (DMA) straight into the window
/// at [`Self::window_offset`] once [`Self::room`] says they fit, then
/// [`Self::advanced`]. Each window's offset counts on and wraps at
/// [`PORT_SIZE`], as Dell's driver counts; every CPU word waits for room in
/// the FIFO, and room that doesn't come within the limit ends the scene
/// ([`Fed::stuck`]: nothing more is written, no scene end).
pub struct Feed<'r, B> {
    regs: &'r mut B,
    clock: Clock,
    limit_us: u32,
    /// Free FIFO words at the last status read, less those written since.
    credit: u32,
    fed: Fed,
    /// The TA window's and the end window's offsets (bytes).
    at: usize,
    ends: usize,
    /// ENABLE as found, and the table's `0x85C`.
    en: u32,
    grid: u32,
    t0: u32,
}

impl<'r, B: RegisterBus> Feed<'r, B> {
    /// The done bit cleared and enabled, then the TA's start table for `l`.
    /// `_ready`: the manager set up for the TA ([`super::ta_setup::set_up`],
    /// once, never between scenes: audit C81).
    pub fn start(
        regs: &'r mut B,
        _ready: &super::ta_setup::Ready,
        l: &Layout,
        clock: Clock,
        limit_us: u32,
    ) -> Self {
        let en = regs.read32(MBX + reg::ENABLE);
        regs.write32(MBX + reg::CLEAR, DONE);
        regs.write32(MBX + reg::ENABLE, en | DONE);
        let t0 = (clock.now)();
        let table = l.start_table();
        for &(off, v) in &table {
            regs.write32(MBX + off, v);
        }
        let grid = table.iter().find(|p| p.0 == GRID).map_or(0, |p| p.1);
        Self {
            regs,
            clock,
            limit_us,
            credit: 0,
            fed: Fed::default(),
            at: 0,
            ends: 0,
            en,
            grid,
            t0,
        }
    }

    /// Audit C78 (reminder R37): `0x85C` for the scene's body with the
    /// scene-end bit off (`false`) or on as the GLES driver's table has it
    /// (`true`, the start's); the scene's end sets it either way. Before
    /// the first word.
    pub fn scene_end_in_body(&mut self, on: bool) {
        let v = if on {
            self.grid
        } else {
            self.grid & !SCENE_END
        };
        self.regs.write32(MBX + GRID, v);
    }

    /// `item` into its window once the FIFO has room; false once the scene
    /// has ended stuck.
    pub fn put(&mut self, item: Item) -> bool {
        match item {
            Item::Word(v) => {
                let ok = self.write(PORT + self.at, v);
                if ok {
                    self.at = (self.at + 4) % PORT_SIZE;
                }
                ok
            }
            Item::End => {
                let ok = self.write(END_PORT + self.ends, 1);
                if ok {
                    self.ends = (self.ends + 4) % PORT_SIZE;
                }
                ok
            }
        }
    }

    /// Free FIFO words now (one status read, counted in [`Fed`]); 0 once
    /// stuck.
    pub fn room(&mut self) -> u32 {
        if self.fed.stuck.is_some() {
            return 0;
        }
        let s = self.regs.read32(MBX + reg::STATUS);
        self.fed.level_reads += 1;
        let free = free_bytes(s) / 4;
        self.fed.level_max = self.fed.level_max.max(FIFO_WORDS - free);
        self.credit = free;
        free
    }

    /// Waits up to the limit for room; false (and [`Fed::stuck`]) if none
    /// came.
    fn wait_room(&mut self) -> bool {
        let t0 = (self.clock.now)();
        loop {
            if self.fed.stuck.is_some() {
                return false;
            }
            if self.room() != 0 {
                return true;
            }
            if self.clock.us_since(t0) > self.limit_us {
                self.fed.stuck = Some(self.regs.read32(MBX + reg::STATUS));
                return false;
            }
        }
    }

    fn write(&mut self, offset: usize, v: u32) -> bool {
        if self.credit == 0 && !self.wait_room() {
            return false;
        }
        self.regs.write32(offset, v);
        self.credit -= 1;
        self.fed.words += 1;
        true
    }

    /// Where the next TA window word goes, in bytes from [`PORT`].
    #[must_use]
    pub const fn window_offset(&self) -> usize {
        self.at
    }

    /// Another writer put `words` into the TA window at
    /// [`Self::window_offset`] (within the room [`Self::room`] gave).
    pub fn advanced(&mut self, words: u32) {
        self.at = (self.at + 4 * words as usize) % PORT_SIZE;
        self.credit = self.credit.saturating_sub(words);
        self.fed.words += words;
    }

    /// The scene's end (unless stuck), the done bit polled for up to the
    /// limit from the start (ADR-0026 D3: no interrupt from the 3D core),
    /// then the bit and `0x85C` put back.
    pub fn finish(mut self) -> Fed {
        let grid = self.grid;
        let sent = self.fed.stuck.is_none() && {
            self.regs.write32(MBX + GRID, grid | SCENE_END);
            self.put(Item::Word(super::END))
        };
        let (clock, t0) = (self.clock, self.t0);
        self.fed.feed_us = clock.us_since(t0);
        if sent {
            while clock.us_since(t0) < self.limit_us {
                if self.regs.read32(MBX + reg::STATUS) & DONE != 0 {
                    self.fed.done = true;
                    self.fed.ta_us = clock.us_since(t0);
                    break;
                }
            }
        }
        self.regs.write32(MBX + reg::CLEAR, DONE);
        self.regs.write32(MBX + reg::ENABLE, self.en);
        self.regs.write32(MBX + GRID, grid);
        self.fed
    }
}
