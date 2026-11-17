//! Step 4f's probe (`-rp4f <seconds>`, docs/design/ref-rp.md "Step 4f's
//! probe before the build"): the questions the device must answer before
//! the feed by DMA is built, asked on the game's own frames.
//!
//! Iterate, don't imitate (rule 0). Dell's GLES driver writes every word
//! with the CPU and lets a full FIFO stall the bus; P4's `rp-tabench`
//! moved one unbroken list by DMA in pieces the FIFO had room for. Neither
//! answers what the game's frame needs: its words are runs (a state
//! block, a strip's corners) with a write to the end window after each
//! strip, ~1,000 a frame. So the probe feeds the game's real frames three
//! ways and compares them on the chip's own output:
//!
//! - the CPU (today's feed): the reference picture, hashed;
//! - **paced** (phase A, P4's way): each run's line-aligned middle by DMA
//!   in pieces no larger than the FIFO's room, its ragged ends and the
//!   strip's end by the CPU;
//! - **unpaced** (phase B, the question): the same middles by DMA with no
//!   room check, the bus left to stall on a full FIFO as Dell's driver
//!   lets the CPU's stores stall.
//!
//! While each DMA piece runs, the CPU walks a buffer four times the
//! D-cache at one load a line (every load a miss to SDRAM), against the
//! same walk with nothing on the bus: what share of its memory speed the
//! CPU keeps while the chip is fed (question b). The wait itself is the
//! free time a frame (question c). Every DMA pass's picture is hashed
//! against the CPU pass's of the same words (question a).
//!
//! Phase B runs last and may freeze the X51v (a TA that never drains with
//! the DMA holding the bus): its first line is logged (and so on the card)
//! before its first piece, and any failure ends the probe. A piece not
//! done in 200 ms is aborted; a TA or render that never finishes resets
//! the core (the runner's own recovery).

use core::fmt::Arguments;

use rp_drv_mbx3d::ta;
use rp_hal_api::DmaEngine;
use rp_hal_mem::{load32_volatile, words_as_bytes};
use rp_hal_pxa27x::dma::Dma;
use rp_hal_pxa27x::{Mmio, cpu, rt};
use rp_port::gfx::send_runs;

use super::{CLOCK, Open, TA_LIMIT_US, us_since};

/// The TA window's physical address for the DMA: the 2700G's register
/// block (CS3 + `MBX_REG_OFFSET`) plus [`ta::PORT`], as `rp-tabench`'s.
const TA_WINDOW: u32 = 0x0C00_0000 + rp_hal_x51v::MBX_REG_OFFSET + ta::PORT as u32;
/// Frames in each phase.
const FRAMES: u32 = 16;
/// A DMA piece not done in this long is aborted (a full FIFO drains in
/// microseconds when the TA runs).
const PIECE_LIMIT_US: u32 = 200_000;
/// Bytes of the cache-missing walk: four times the PXA270's 32 KiB D-cache.
pub const WALK_BYTES: usize = 128 * 1024;
/// Loads between polls in the walk.
const LOADS_A_POLL: usize = 16;
/// Ticks (3.25 MHz) of the idle walk measured before each phase.
const IDLE_TICKS: u32 = 3_250 * 20;

/// The cache-missing walk: one byte a line over [`WALK_BYTES`].
struct Walk {
    buf: &'static [u8],
    at: usize,
    sink: u32,
}

impl Walk {
    /// [`LOADS_A_POLL`] loads, each a line on from the last.
    fn step(&mut self) {
        let mut acc = self.sink;
        for _ in 0..LOADS_A_POLL {
            acc = acc.wrapping_add(u32::from(core::hint::black_box(self.buf[self.at])));
            self.at += cpu::LINE as usize;
            if self.at >= self.buf.len() {
                self.at = 0;
            }
        }
        self.sink = core::hint::black_box(acc);
    }

    /// Loads a tick with nothing else on the bus (one tick read a poll, as
    /// the DMA's wait reads one register a poll).
    fn idle_rate(&mut self) -> (u64, u64) {
        let t0 = rt::ticks();
        let mut loads = 0u64;
        loop {
            self.step();
            loads += LOADS_A_POLL as u64;
            let t = rt::ticks().wrapping_sub(t0);
            if t >= IDLE_TICKS {
                return (loads, u64::from(t));
            }
        }
    }
}

/// One phase's sums.
#[derive(Clone, Copy, Default)]
struct Sums {
    frames: u32,
    /// The CPU pass's feed (us) and words, for the same frames.
    cpu_feed_us: u32,
    words: u32,
    /// The DMA pass's feed (us, start to the scene's end in).
    dma_feed_us: u32,
    /// Waiting for DMA pieces (us): the CPU's free time.
    wait_us: u32,
    pieces: u32,
    dma_words: u32,
    cpu_words: u32,
    /// The walk while pieces ran: loads and ticks.
    loads: u64,
    ticks: u64,
    /// The walk idle (measured at the phase's start).
    idle_loads: u64,
    idle_ticks: u64,
    same: u32,
    differ: u32,
    ta_hangs: u32,
    render_hangs: u32,
    /// Pieces the DMA refused, timed out on, or stopped with a bus error.
    dma_failed: u32,
    level_max: u32,
}

/// What a run's feed by DMA needs while it goes.
struct Pass<'a, 'r> {
    feed: ta::Feed<'r, Mmio>,
    dma: &'a mut Dma,
    walk: &'a mut Walk,
    paced: bool,
    s: &'a mut Sums,
    ok: bool,
}

impl Pass<'_, '_> {
    /// One run's words: its head up to a cache line and its tail by the
    /// CPU, its whole lines by DMA (the controller reads whole lines).
    fn run(&mut self, ws: &[u32]) -> bool {
        let n = ws.len();
        let addr = ws.as_ptr().addr();
        let head = ((cpu::LINE as usize - (addr & (cpu::LINE as usize - 1))) % cpu::LINE as usize
            / 4)
        .min(n);
        let mid_end = head + ((n - head) & !7);
        if !self.feed.put_words(&ws[..head]) {
            return self.fail();
        }
        self.s.cpu_words += head as u32;
        let mut at = head;
        while at < mid_end {
            let window = (ta::PORT_SIZE - self.feed.window_offset()) / 4;
            let mut piece = (mid_end - at).min(window);
            if self.paced {
                piece = piece.min(self.feed.room() as usize);
            }
            piece &= !7;
            if piece == 0 {
                // Across the window's wrap, or (paced) no room for a line:
                // one line by the CPU, which waits for room itself.
                if !self.feed.put_words(&ws[at..at + 8]) {
                    return self.fail();
                }
                self.s.cpu_words += 8;
                at += 8;
                continue;
            }
            // `arm` cleans the lines and drains the write buffer: the
            // CPU's words before it are in the window first.
            let dst = TA_WINDOW + self.feed.window_offset() as u32;
            if !self
                .dma
                .arm_copy_to_device_memory(words_as_bytes(&ws[at..at + piece]), dst)
            {
                self.s.dma_failed += 1;
                return self.fail();
            }
            let t0 = rt::ticks();
            let mut loads = 0u64;
            let done = loop {
                match self.dma.done() {
                    Some(r) => break r,
                    None if us_since(t0) > PIECE_LIMIT_US => {
                        self.dma.abort();
                        break false;
                    }
                    None => {
                        self.walk.step();
                        loads += LOADS_A_POLL as u64;
                    }
                }
            };
            let t = rt::ticks().wrapping_sub(t0);
            self.s.loads += loads;
            self.s.ticks += u64::from(t);
            self.s.wait_us += t / 13 * 4;
            self.s.pieces += 1;
            if !done {
                self.s.dma_failed += 1;
                return self.fail();
            }
            self.feed.advanced(piece as u32);
            self.s.dma_words += piece as u32;
            at += piece;
        }
        if !self.feed.put_words(&ws[mid_end..]) {
            return self.fail();
        }
        self.s.cpu_words += (n - mid_end) as u32;
        true
    }

    fn fail(&mut self) -> bool {
        self.ok = false;
        false
    }
}

/// The probe: when it starts, where it is, and its DMA channel and walk.
pub struct Probe {
    dma: Dma,
    walk: Walk,
    /// Seconds of frames before phase A.
    after_us: u64,
    seen_us: u64,
    last: Option<u32>,
    /// Frames probed so far (phase A, then B).
    n: u32,
    sums: Sums,
    /// A failure ended the probe.
    stopped: bool,
}

impl Probe {
    /// The probe on DMA channel `dma`, its walk over `walk` (at least
    /// [`WALK_BYTES`], cached RAM), starting `after_s` seconds into the
    /// screen's frames.
    pub fn new(dma: Dma, walk: &'static [u8], after_s: u32) -> Self {
        Self {
            dma,
            walk: Walk {
                buf: &walk[..WALK_BYTES.min(walk.len())],
                at: 0,
                sink: 0,
            },
            after_us: u64::from(after_s) * 1_000_000,
            seen_us: 0,
            last: None,
            n: 0,
            sums: Sums::default(),
            stopped: false,
        }
    }

    /// Whether this frame is a probed one (called once a frame).
    pub fn due(&mut self) -> bool {
        let now = rt::ticks();
        if let Some(t) = self.last {
            self.seen_us += u64::from(now.wrapping_sub(t) / 13 * 4);
        }
        self.last = Some(now);
        !self.stopped && self.seen_us >= self.after_us && self.n < 2 * FRAMES
    }

    /// One probed frame: the CPU pass, then the phase's DMA pass, each
    /// rendered and its picture hashed. The DMA pass's picture stays on
    /// the screen.
    pub fn frame(
        &mut self,
        o: &mut Open,
        regs: &mut Mmio,
        parts: &[&[u32]],
        bg: &[u32],
        log: fn(Arguments<'_>),
    ) -> Result<(), rp_hal_cabi::Errno> {
        let paced = self.n < FRAMES;
        let first = self.n % FRAMES == 0;
        if first {
            self.sums = Sums::default();
            if self.walk.buf.len() < WALK_BYTES {
                log(format_args!(
                    "rp-4f: no room for the walk: the probe not run"
                ));
                self.stopped = true;
                return Ok(());
            }
            let (l, t) = self.walk.idle_rate();
            self.sums.idle_loads = l;
            self.sums.idle_ticks = t;
            log(format_args!(
                "rp-4f: phase {} ({}) starts: {FRAMES} frames, each fed by the CPU then by DMA, both pictures hashed{}",
                if paced { "A" } else { "B" },
                if paced {
                    "paced: pieces no larger than the FIFO's room"
                } else {
                    "unpaced: no room check, the bus may stall; this may freeze the X51v"
                },
                if paced { "" } else { " (a soft reset then)" }
            ));
        }
        self.n += 1;
        let s = &mut self.sums;
        // The CPU's pass: the reference.
        let height = o.lay.ta.height;
        let cpu = feed_cpu(o, regs, parts, bg);
        let cpu_hash = match cpu {
            f if f.done => {
                s.cpu_feed_us += f.feed_us;
                s.words += f.words;
                render_hash(o, regs, height)
            }
            _ => {
                s.ta_hangs += 1;
                o.recover(regs);
                None
            }
        };
        // The DMA pass.
        let _ = ta::clear(o.vram, &o.lay.ta);
        cpu::drain_write_buffer();
        let mut pass = Pass {
            feed: ta::Feed::start(regs, &o.ready, &o.lay.ta, CLOCK, TA_LIMIT_US),
            dma: &mut self.dma,
            walk: &mut self.walk,
            paced,
            s,
            ok: true,
        };
        let _ = ta::draws(&[bg]).all(|i| pass.feed.put(i));
        for words in parts {
            if !pass.ok {
                break;
            }
            send_runs(words, &mut pass, Pass::run, |p| {
                p.feed.put(ta::Item::End) || p.fail()
            });
        }
        let ok = pass.ok;
        let fed = pass.feed.finish();
        let s = &mut self.sums;
        s.dma_feed_us += fed.feed_us;
        s.level_max = s.level_max.max(fed.level_max);
        s.frames += 1;
        let mut failed = !ok;
        if fed.done {
            match (render_hash(o, regs, height), cpu_hash) {
                (Some(d), Some(c)) if d == c => s.same += 1,
                (Some(_), Some(_)) => s.differ += 1,
                (None, _) => {
                    s.render_hangs += 1;
                    failed = true;
                }
                (Some(_), None) => {}
            }
        } else {
            s.ta_hangs += 1;
            o.recover(regs);
            failed = true;
        }
        if failed {
            log(format_args!(
                "rp-4f: phase {} frame {}: the DMA pass failed (fed {}, TA done {}, stuck {:?}): the probe ends here",
                if paced { "A" } else { "B" },
                self.n,
                ok,
                fed.done,
                fed.stuck
            ));
            self.stopped = true;
        }
        if failed || self.n % FRAMES == 0 {
            self.report(paced, log);
        }
        Ok(())
    }

    /// The phase's line.
    fn report(&self, paced: bool, log: fn(Arguments<'_>)) {
        let s = &self.sums;
        let f = s.frames.max(1);
        let tenths = |us: u32| {
            let t = us / f / 100;
            (t / 10, t % 10)
        };
        let (cf, cft) = tenths(s.cpu_feed_us);
        let (df, dft) = tenths(s.dma_feed_us);
        let (w, wt) = tenths(s.wait_us);
        // The walk's speed while pieces ran against idle, in tenths of a
        // percent (loads a tick, both).
        let kept = if s.ticks == 0 || s.idle_loads == 0 {
            0
        } else {
            s.loads * s.idle_ticks * 1000 / (s.ticks * s.idle_loads)
        };
        log(format_args!(
            "rp-4f: phase {} ({}): {} frames, {} words a frame ({} by DMA, {} by the CPU) in {} pieces; the feed {df}.{dft} ms by DMA against {cf}.{cft} ms by the CPU; the CPU free {w}.{wt} ms a frame while pieces ran, its memory walk at {}.{}% of its idle speed meanwhile ({} loads in {} ticks; idle {} in {}); pictures the same {}, different {}; TA hangs {}, render hangs {}, DMA failures {}; the FIFO's level at most {} words",
            if paced { "A" } else { "B" },
            if paced { "paced" } else { "unpaced" },
            s.frames,
            s.words / f,
            s.dma_words / f,
            s.cpu_words / f,
            s.pieces / f,
            kept / 10,
            kept % 10,
            s.loads,
            s.ticks,
            s.idle_loads,
            s.idle_ticks,
            s.same,
            s.differ,
            s.ta_hangs,
            s.render_hangs,
            s.dma_failed,
            s.level_max
        ));
    }
}

/// The frame by the CPU, as the runner feeds it.
fn feed_cpu(o: &mut Open, regs: &mut Mmio, parts: &[&[u32]], bg: &[u32]) -> ta::Fed {
    let _ = ta::clear(o.vram, &o.lay.ta);
    cpu::drain_write_buffer();
    let mut feed = ta::Feed::start(regs, &o.ready, &o.lay.ta, CLOCK, TA_LIMIT_US);
    let _ = ta::draws(&[bg]).all(|i| feed.put(i));
    for words in parts {
        send_runs(
            words,
            &mut feed,
            |f, ws| f.put_words(ws),
            |f| f.put(ta::Item::End),
        );
    }
    feed.finish()
}

/// The render started and waited for, then the drawn rows (`height`)
/// hashed (FNV-1a over words); `None` if the render hung (the core then
/// reset by `wait_render`).
fn render_hash(o: &mut Open, regs: &mut Mmio, height: u32) -> Option<u32> {
    o.start_render(regs);
    o.wait_render(regs).ok()?;
    let bytes = (height * super::W * 2) as usize;
    let mut h = 0x811C_9DC5u32;
    let mut at = 0;
    while at < bytes {
        h = (h ^ load32_volatile(o.vram, at).unwrap_or(0)).wrapping_mul(0x0100_0193);
        at += 4;
    }
    Some(h)
}
