//! The mixer (ADR-0032 S2: R2-R4, the owner's conditions 1-4, 7 and 13).
//!
//! **Streams** are rings the programs write and the mixer reads (R3): a
//! program writes frames in place ([`Mixer::writable`], [`Mixer::commit`]),
//! and they stay in the ring until **played**, not until mixed. A ring is
//! bytes (16-bit little-endian samples, interleaved), as it will be in the
//! shared region between two processes: no alignment is assumed. Each has a
//! role (R3, [`Role`]), a rate (converted to 44.1 kHz, [`crate::resample`]),
//! a volume, a requested start and the **actual start** it got (condition
//! 3), and its played position.
//!
//! **One long DMA ring, rewritten ahead** (R2): [`DMA_RING_BYTES`] (255 ms)
//! loops to I2S. The mixer fills it ahead of the DMA position and then
//! sleeps. When a new sound arrives it **re-mixes from just ahead of the
//! DMA position** ([`MARGIN`] plus at most one [`SNAP`]), so a click starts
//! within milliseconds however far ahead music was mixed. Rewriting is
//! exact because the mix is a function of output time: the resampler keeps
//! no history ([`crate::resample`]), a volume ramp is a function of time,
//! and the chain's small state ([`chain::State`]) is kept at every
//! [`SNAP`] boundary.
//!
//! **The ring ends where the mix ends** (ADR-0032 A6, option B): after
//! each wake's mixing the mixer clears the rest of the piece its mix ends
//! in and moves the ring's end there ([`DmaRing::end_at`]), so a mixer late
//! by any amount makes the engine stop after silence, never replay the
//! ring; the next wake carries every stream on from where the engine
//! stopped and starts it again. The output's clock is the engine's frames,
//! exactly: it stands still while the engine is stopped.
//!
//! **The wake interval adapts** (condition 13): about [`INTERACTIVE_MS`]
//! while an interactive stream plays (its program's low-latency hint, or
//! small writes at short intervals: the old `waveOut` games' pattern), about
//! [`BACKGROUND_MS`] when only background media plays. The mixer always
//! mixes ahead what the coming interval needs and wakes before that runs
//! out, so switching either way is glitch-free; stream rings are sized for
//! the longest lead whatever the interval (condition 4,
//! [`min_ring_bytes`]).
//!
//! **Safety** (R8): every read of a stream's ring is reduced into it (a
//! program can make its own sound wrong, never the mixer read outside its
//! ring); a commit larger than the room is refused.

use rp_drv_pxa_i2s::{StreamError, TX_PORT};
use rp_hal_api::{DmaRing, RingEnd};

use crate::chain::{self, BLOCK};
use crate::resample::{MAX_TAPS, OUT, PHASES, POINTS, Ratio, WING};

/// Source frames a block's fast path decodes at most: a 96 kHz source's
/// 70 frames a block and both wings of its filter.
const WINDOW: usize = 128;
use crate::role::{Role, Unplug, on_unplug};

/// Streams open at once.
pub const MAX_STREAMS: usize = 8;
/// The DMA ring: 44 KiB, 11,264 frames, 255 ms (the HAL cuts it in 32
/// pieces of 1,408 bytes, 352 frames: ADR-0032 A6).
pub const DMA_RING_BYTES: usize = 11 * 4096;
/// The DMA ring in frames.
pub const RING_FRAMES: u64 = (DMA_RING_BYTES / 4) as u64;
/// Re-mixing starts this far ahead of the DMA position at least: 5 ms
/// (ADR-0032 R2's target; the device run measures what it needs).
pub const MARGIN: u64 = 221;
/// Rewrite points are this many frames apart (2.9 ms): the chain's state
/// is kept at each one. A new sound starts at most this much later than
/// the margin allows.
pub const SNAP: u64 = 128;
const SNAPS: usize = (RING_FRAMES / SNAP) as usize;
/// One kept state: its output frame, then the chain's state.
const SNAP_BYTES: usize = 8 + chain::STATE_BYTES;
/// The memory a mixer is given ([`Mixer::new`]): the DMA ring, then the
/// chain's state at every rewrite point. Kept out of the mixer itself (a
/// few hundred bytes) so it can live wherever its owner puts it.
pub const MIXER_BYTES: usize = DMA_RING_BYTES + SNAPS * SNAP_BYTES;
/// The wake interval while an interactive stream plays.
pub const INTERACTIVE_MS: u32 = 15;
/// The wake interval with only background media.
pub const BACKGROUND_MS: u32 = 200;
/// The mixer wakes this long before its mixed sound runs out (the
/// scheduler's slack).
pub const SLACK_MS: u32 = 5;
/// An interactive stream counts as playing this long after its last write
/// (more clicks tend to follow).
pub const HOLD_MS: u32 = 1_000;
/// Volume changes ramp over this many frames (5.8 ms): no zipper noise.
pub const RAMP: u64 = 256;
/// Unity volume, Q15.
pub const UNITY: u32 = 1 << 15;
/// The write pattern that marks a stream interactive (condition 13): this
/// many writes in a row, once it plays, of at most [`SMALL_WRITE_MS`] of
/// sound, each within [`SHORT_GAP_MS`] of the one before, **leaving at most
/// [`SHORT_QUEUE_MS`] queued**. The queue is what tells a game from a music
/// player: both may write small pieces close together (a player topping up
/// its ring does), but only the game keeps little ahead of the sound.
pub const SMALL_RUN: u8 = 3;
/// See [`SMALL_RUN`].
pub const SHORT_QUEUE_MS: u32 = 150;
/// A write leaving more than this queued (ms) ends the pattern.
pub const LONG_QUEUE_MS: u32 = 250;
/// See [`SMALL_RUN`].
pub const SMALL_WRITE_MS: u32 = 50;
/// See [`SMALL_RUN`].
pub const SHORT_GAP_MS: u32 = 100;
/// A write longer than this (ms of sound) ends the interactive pattern.
pub const LARGE_WRITE_MS: u32 = 100;

/// Frames at 44.1 kHz in `ms`.
#[must_use]
pub const fn frames(ms: u32) -> u64 {
    ms as u64 * OUT as u64 / 1_000
}

/// How far ahead the mixer mixes while interactive: two intervals and the
/// margin.
pub const INTERACTIVE_LEAD: u64 = frames(2 * INTERACTIVE_MS) + MARGIN;
/// How far ahead with background media only: the ring less the margin at
/// both ends.
pub const BACKGROUND_LEAD: u64 = (RING_FRAMES - 2 * MARGIN) / BLOCK as u64 * BLOCK as u64;

/// When a stream starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Start {
    /// As soon as the mixer can (its first write).
    Asap,
    /// At this output frame (the output's clock, [`Mixer::clock`]), or as
    /// soon after as the mixer can; the start it got is reported back.
    At(u64),
}

/// A stream's description, given at open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamConfig {
    /// What it is for: required (no guessing for native programs).
    pub role: Role,
    /// Its rate (4-96 kHz).
    pub rate: u32,
    /// 1 (mono) or 2 (stereo, interleaved left first).
    pub channels: u8,
    /// The program's write size, in its frames: the ring holds one on top
    /// of the mixer's lead.
    pub refill: u32,
    /// The program asks for low latency (games, the shell's sounds, calls).
    pub low_latency: bool,
    /// An old Windows Mobile program (its media is muted on unplug, not
    /// paused: it can't be paused from outside).
    pub legacy: bool,
    /// When it starts.
    pub start: Start,
    /// Its volume, Q15 ([`UNITY`]: full; up to 2x).
    pub volume: u32,
}

impl StreamConfig {
    /// A stream at full volume, starting at once, the hint as its role's
    /// default.
    #[must_use]
    pub fn new(role: Role, rate: u32, channels: u8, refill: u32) -> Self {
        Self {
            role,
            rate,
            channels,
            refill,
            low_latency: role.interactive_by_default(),
            legacy: false,
            start: Start::Asap,
            volume: UNITY,
        }
    }
}

/// Why a stream wasn't opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenError {
    /// The rate is outside 4-96 kHz.
    BadRate(u32),
    /// Not 1 or 2 channels.
    BadChannels(u8),
    /// The ring is smaller than the sizing rule ([`min_ring_bytes`]).
    RingTooSmall {
        /// Bytes it must have.
        need: usize,
        /// Bytes it has.
        got: usize,
    },
    /// [`MAX_STREAMS`] are open.
    Full,
}

/// Why a write wasn't taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteError {
    /// No such stream (closed, or a stale handle).
    NoStream,
    /// More frames than the room.
    TooMuch,
    /// The stream was ended.
    Ended,
}

/// A stream's handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamId {
    slot: u8,
    generation: u32,
}

/// A stream's state, for its program (the control block's service side).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamStatus {
    /// The output frame it really started at (condition 3), once placed.
    pub actual_start: Option<u64>,
    /// The DMA position when it was placed (so `actual_start` minus this is
    /// its start latency).
    pub placed_at: Option<u64>,
    /// Its frames really played.
    pub played: u64,
    /// Its frames written.
    pub written: u64,
    /// Frames it may write now.
    pub room: u64,
    /// Ended and everything played.
    pub drained: bool,
    /// Counted as interactive now (hint or write pattern).
    pub interactive: bool,
    /// Paused because the headphones came out (A2); resumes when told.
    pub paused_by_unplug: bool,
    /// Muted because the headphones came out (an old program's media).
    pub muted_by_unplug: bool,
    /// Ran out of data while playing (and hasn't been given more yet).
    pub starved: bool,
}

/// What a wake did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tick {
    /// Wake again in this many ms at the latest.
    pub next_ms: u32,
    /// An interactive stream is playing (the short interval).
    pub interactive: bool,
    /// Frames re-mixed by a rewrite this wake.
    pub rewrote: u64,
    /// Frames mixed this wake (rewrites included).
    pub mixed: u64,
}

/// Counts since the start, for the device run's log.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Wakes ([`Mixer::tick`] calls).
    pub wakes: u32,
    /// Rewrites.
    pub rewrites: u32,
    /// Frames mixed, rewrites included.
    pub mixed: u64,
    /// Of those, re-mixed by rewrites.
    pub rewritten: u64,
    /// The DMA passed the mixed sound: the mixer woke too late.
    pub late: u32,
    /// Times a stream ran out of data while playing.
    pub starved: u32,
    /// Samples the limiter's backstop clipped.
    pub clipped: u32,
    /// Blocks the limiter rode down.
    pub limited: u32,
    /// Wakes that found the engine stopped at the ring's end (a late
    /// mixer, or the race below).
    pub stops: u32,
    /// The engine started again after a stop.
    pub restarts: u32,
    /// Of those, the race: a wake that came after the engine had started
    /// the end's piece (under a piece, 7.9 ms, of sound left), waited for
    /// and restarted at once.
    pub races: u32,
    /// The ring's end couldn't be moved as meant (registers making no
    /// sense, or the rule broken): never expected, never hidden.
    pub ring_failed: u32,
}

struct Slot<'a> {
    generation: u32,
    /// 16-bit little-endian samples, interleaved.
    buf: &'a mut [u8],
    /// Capacity in frames.
    cap: u64,
    ch: usize,
    ratio: Ratio,
    role: Role,
    legacy: bool,
    hint: bool,
    write: u64,
    ended: bool,
    start: Start,
    /// (output frame, source frame): where the source plays.
    anchor: Option<(u64, u64)>,
    actual_start: Option<u64>,
    placed_at: Option<u64>,
    /// The output frame from which it had no data.
    starved_at: Option<u64>,
    /// The earliest output frame the next wake must re-mix for it.
    dirty: Option<u64>,
    paused_at: Option<u64>,
    /// A pause (true) or resume (false) for the next wake.
    pause_req: Option<bool>,
    paused_by_unplug: bool,
    muted_by_unplug: bool,
    volume: u32,
    /// The volume as a ramp: (from, to, starting at output frame).
    ramp: (u32, u32, u64),
    gain_req: bool,
    /// Source frames before this one may be overwritten.
    release: u64,
    played: u64,
    last_commit_ms: Option<u32>,
    last_write_ms: u32,
    small_run: u8,
}

/// The sizing rule (ADR-0032 R2, condition 4): a stream's ring, in bytes,
/// holds the DMA ring's worth of sound plus the margin and a rewrite step,
/// plus one write of its own, plus the resampler's reach on both sides,
/// in its own rate and channels, **whatever its current interval**.
pub fn min_ring_bytes(cfg: &StreamConfig) -> Result<usize, OpenError> {
    if !(1..=2).contains(&cfg.channels) {
        return Err(OpenError::BadChannels(cfg.channels));
    }
    let ratio = Ratio::new(cfg.rate).map_err(|e| OpenError::BadRate(e.0))?;
    let out = RING_FRAMES + MARGIN + SNAP;
    let src = (out * u64::from(cfg.rate)).div_ceil(u64::from(OUT));
    let frames = src + u64::from(cfg.refill) + 2 * u64::from(ratio.reach()) + 2;
    Ok(frames as usize * usize::from(cfg.channels) * 2)
}

impl Slot<'_> {
    fn gain_at(&self, t: u64) -> u32 {
        let (from, to, at) = self.ramp;
        if t >= at + RAMP {
            to
        } else if t <= at {
            from
        } else {
            let f = (t - at) as i64;
            (i64::from(from) + (i64::from(to) - i64::from(from)) * f / RAMP as i64) as u32
        }
    }

    /// The target volume: muted is 0.
    fn target(&self) -> u32 {
        if self.muted_by_unplug { 0 } else { self.volume }
    }

    /// The source frame playing at output frame `t` (whole frames), or
    /// `None` before the start.
    fn src_at(&self, t: u64) -> Option<u64> {
        let (ao, asrc) = self.anchor?;
        let mut t = t;
        if let Some(p) = self.paused_at {
            t = t.min(p);
        }
        if let Some(s) = self.starved_at {
            t = t.min(s);
        }
        if t < ao {
            return Some(asrc);
        }
        Some(asrc + self.ratio.locate(t - ao).0)
    }

    /// One sample of channel `c`, `o` frames from source frame `idx`
    /// (whose place in the ring is `pos`): 0 outside what's in the ring
    /// (before the start, overwritten, or not written yet, `lo..hi`). No
    /// division: the place steps from `pos`.
    #[inline]
    fn tap(&self, idx: i64, pos: i64, o: i32, c: usize, (lo, hi): (i64, i64)) -> i32 {
        let j = idx + i64::from(o);
        if j < lo || j >= hi {
            return 0;
        }
        let cap = self.cap as i64;
        let mut p = pos + i64::from(o);
        if p < 0 {
            p += cap;
        } else if p >= cap {
            p -= cap;
        }
        let i = (p as usize * self.ch + c) * 2;
        match self.buf.get(i..i + 2) {
            Some(b) => i32::from(i16::from_le_bytes([b[0], b[1]])),
            None => 0,
        }
    }

    /// Adds this stream into one block starting at output frame `b`.
    /// Returns whether it ran out of data in the block.
    fn add(&mut self, b: u64, acc: &mut [i32; 2 * BLOCK]) -> bool {
        if self.add_fast(b, acc) {
            return false;
        }
        self.add_exact(b, acc)
    }

    /// The block in one pass, when nothing changes inside it: placed
    /// before it, not paused or run dry in it, its volume steady, every
    /// frame its filter reads written and still in the ring. The stream's
    /// window is decoded once (no bounds or wrap per tap) and the filter's
    /// coefficients are worked out once per frame for both channels. Gives
    /// exactly what [`Self::add_exact`] gives (a test holds them equal);
    /// false (nothing added) when it doesn't apply.
    fn add_fast(&self, b: u64, acc: &mut [i32; 2 * BLOCK]) -> bool {
        let Some((ao, asrc)) = self.anchor else {
            return false;
        };
        let end = b + BLOCK as u64;
        if ao > b
            || self.paused_at.is_some_and(|p| p < end)
            || self.starved_at.is_some_and(|s| s < end)
        {
            return false;
        }
        let at = self.ramp.2;
        if !(at + RAMP <= b || at + 1 >= end) {
            return false;
        }
        let ratio = self.ratio;
        let bypass = ratio.is_bypass();
        let (i0, ph0) = ratio.locate(b - ao);
        let (i0, i1) = (asrc + i0, asrc + ratio.locate(end - 1 - ao).0);
        let reach = u64::from(ratio.reach());
        if i1 + reach + 1 > self.write {
            return false;
        }
        // The window: from the first frame's left wing to the last's right.
        let (left, right) = if bypass {
            (0, 0)
        } else {
            (ratio.taps() as u64 - 1, ratio.taps() as u64)
        };
        if i0 < self.write.saturating_sub(self.cap) + left {
            return false;
        }
        let g = self.gain_at(b);
        if g == 0 {
            return true;
        }
        let add = |acc: &mut [i32; 2 * BLOCK], f: usize, c: usize, s: i32| {
            let v = if g == UNITY {
                s
            } else {
                ((i64::from(s) * i64::from(g)) >> 15) as i32
            };
            if self.ch == 1 {
                acc[2 * f] = acc[2 * f].saturating_add(v);
                acc[2 * f + 1] = acc[2 * f + 1].saturating_add(v);
            } else {
                acc[2 * f + c] = acc[2 * f + c].saturating_add(v);
            }
        };
        if bypass {
            // Straight from the ring, a frame at a time.
            let mut pos = (i0 % self.cap) as usize;
            for f in 0..BLOCK {
                let at = pos * self.ch * 2;
                for c in 0..self.ch {
                    let i = at + 2 * c;
                    add(
                        acc,
                        f,
                        c,
                        i32::from(i16::from_le_bytes([self.buf[i], self.buf[i + 1]])) << 8,
                    );
                }
                pos += 1;
                if pos == self.cap as usize {
                    pos = 0;
                }
            }
            return true;
        }
        let w0 = i0 - left;
        let n = (i1 + right - w0 + 1) as usize;
        if n > WINDOW {
            return false;
        }
        let mut win = [[0i32; WINDOW]; 2];
        let mut pos = (w0 % self.cap) as usize;
        for k in 0..n {
            let at = pos * self.ch * 2;
            for (c, w) in win.iter_mut().enumerate().take(self.ch) {
                let i = at + 2 * c;
                w[k] = i32::from(i16::from_le_bytes([self.buf[i], self.buf[i + 1]]));
            }
            pos += 1;
            if pos == self.cap as usize {
                pos = 0;
            }
        }
        let taps = ratio.taps();
        // Whole-number steps repeat their coefficients every 2 or 4 frames:
        // each phase's set is built once a block (by quarters of a frame).
        let mut quarter = [([0i16; MAX_TAPS], [0i16; MAX_TAPS]); 4];
        let mut have = [false; 4];
        let mut general = ([0i16; MAX_TAPS], [0i16; MAX_TAPS]);
        let (mut idx, mut ph) = (i0, ph0);
        for f in 0..BLOCK {
            if f > 0 {
                let (m, p) = ratio.advance(ph);
                idx += u64::from(m);
                ph = p;
            }
            let base = (idx - w0) as usize;
            // The coefficients: table points by stride (whole-number
            // steps), 48 kHz's table rows, or worked out.
            let (cl, cr): (&[i16; MAX_TAPS], &[i16; MAX_TAPS]) =
                if let Some((a, b)) = ratio.stride(ph) {
                    let q = ph as usize * 4 / OUT as usize;
                    if !have[q] {
                        let (l, r) = &mut quarter[q];
                        for k in 0..taps {
                            let (p, q) = (a + k * PHASES, b + k * PHASES);
                            l[k] = if p < POINTS { WING[p] } else { 0 };
                            r[k] = if q < POINTS { WING[q] } else { 0 };
                        }
                        have[q] = true;
                    }
                    (&quarter[q].0, &quarter[q].1)
                } else if let Some(rows) = ratio.rows48(ph) {
                    rows
                } else {
                    let (mut l, mut r) = ([0i32; MAX_TAPS], [0i32; MAX_TAPS]);
                    ratio.coefs(ph, &mut l, &mut r);
                    for k in 0..MAX_TAPS {
                        // Every coefficient fits 16 bits (Q15, at most 0.9).
                        general.0[k] = l[k] as i16;
                        general.1[k] = r[k] as i16;
                    }
                    (&general.0, &general.1)
                };
            for (c, w) in win.iter().enumerate().take(self.ch) {
                let lw = &w[base + 1 - taps..=base];
                let rw = &w[base + 1..=base + taps];
                let mut sum = 0i32;
                for ((x, y), (cx, cy)) in lw.iter().rev().zip(rw).zip(cl.iter().zip(cr)) {
                    sum += x * i32::from(*cx) + y * i32::from(*cy);
                }
                add(acc, f, c, Ratio::finish(sum));
            }
        }
        true
    }

    /// The block frame by frame: every case (starts, pauses, running dry,
    /// ramps, the ring's edges); the reference for [`Self::add_fast`].
    fn add_exact(&mut self, b: u64, acc: &mut [i32; 2 * BLOCK]) -> bool {
        let Some((ao, asrc)) = self.anchor else {
            return false;
        };
        let reach = u64::from(self.ratio.reach());
        let ratio = self.ratio;
        let bypass = ratio.is_bypass();
        let cap = self.cap;
        // What's in the ring: written and not yet written over.
        let valid = (self.write.saturating_sub(cap) as i64, self.write as i64);
        // (source frame, phase, its place in the ring)
        let mut cur: Option<(u64, u32, u64)> = None;
        for f in 0..BLOCK {
            let t = b + f as u64;
            if t < ao {
                continue;
            }
            if self.paused_at.is_some_and(|p| t >= p) || self.starved_at.is_some_and(|s| t >= s) {
                break;
            }
            let (idx, ph, pos) = match cur {
                None => {
                    let (i, p) = ratio.locate(t - ao);
                    (asrc + i, p, (asrc + i) % cap)
                }
                Some((i, p, pos)) => {
                    let (m, np) = ratio.advance(p);
                    let mut q = pos + u64::from(m);
                    while q >= cap {
                        q -= cap;
                    }
                    (i + u64::from(m), np, q)
                }
            };
            cur = Some((idx, ph, pos));
            // The filter reads up to `reach` frames past the next one.
            if !self.ended && idx + reach + 1 > self.write {
                self.starved_at = Some(t);
                return true;
            }
            if self.ended && idx >= self.write + reach {
                break;
            }
            let g = self.gain_at(t);
            if g == 0 {
                continue;
            }
            let (i, p) = (idx as i64, pos as i64);
            // A mono stream is filtered once and goes to both sides.
            for c in 0..self.ch {
                let s = if bypass {
                    self.tap(i, p, 0, c, valid) << 8
                } else {
                    ratio.sample(ph, &mut |o| self.tap(i, p, o, c, valid))
                };
                let v = if g == UNITY {
                    s
                } else {
                    ((i64::from(s) * i64::from(g)) >> 15) as i32
                };
                if self.ch == 1 {
                    acc[2 * f] = acc[2 * f].saturating_add(v);
                    acc[2 * f + 1] = acc[2 * f + 1].saturating_add(v);
                } else {
                    acc[2 * f + c] = acc[2 * f + c].saturating_add(v);
                }
            }
        }
        false
    }

    /// The output frame where its data runs out (the mix can't go further
    /// for it), if it's playing and not ended.
    fn data_end(&self) -> Option<u64> {
        let (ao, asrc) = self.anchor?;
        if self.ended || self.paused_at.is_some() || self.starved_at.is_some() {
            return None;
        }
        // The last source frame the filter may centre on.
        let last = (self.write + 1).checked_sub(u64::from(self.ratio.reach()) + 2)?;
        if last < asrc {
            return Some(ao);
        }
        let k = last - asrc + 1;
        Some(ao + (k * u64::from(OUT)).div_ceil(u64::from(self.ratio.rate)))
    }

    /// Ran out and was given more since, or never placed but written.
    fn interactive(&self, now_ms: u32) -> bool {
        if !(self.hint || self.small_run >= SMALL_RUN) || self.paused_at.is_some() {
            return false;
        }
        self.write > self.played || now_ms.wrapping_sub(self.last_write_ms) < HOLD_MS
    }
}

fn align_up(v: u64, a: u64) -> u64 {
    v.div_ceil(a) * a
}

fn align_down(v: u64, a: u64) -> u64 {
    v / a * a
}

fn min_opt(a: Option<u64>, b: u64) -> Option<u64> {
    Some(a.map_or(b, |a| a.min(b)))
}

/// The mixer for one output.
pub struct Mixer<'a> {
    slots: [Option<Slot<'a>>; MAX_STREAMS],
    generation: u32,
    ring: &'a mut [u8],
    running: bool,
    /// The output frame the DMA reads next.
    hw: u64,
    last_pos: u64,
    /// Frames mixed into the ring up to here.
    mixed_to: u64,
    state: chain::State,
    /// The chain's state at each [`SNAP`] boundary: (output frame, state),
    /// as bytes ([`SNAP_BYTES`] each), in the memory given.
    snaps: &'a mut [u8],
    speaker: bool,
    /// A closed stream's sound must come out of the mix.
    dirty_all: bool,
    /// The last write to an interactive stream (ms): the short interval
    /// holds for [`HOLD_MS`] after it, even once that stream has closed
    /// (a click is its own short stream; more tend to follow).
    last_interactive_ms: Option<u32>,
    /// The DMA has just started: the first wake doesn't count it late.
    starting: bool,
    /// The ring has been given an end since it started.
    ended: bool,
    /// Frames per piece of the ring (the HAL's cut, [`DmaRing::piece`]).
    piece: u64,
    /// The output frame the ring's end was last put at.
    end_frame: u64,
    stats: Stats,
}

impl<'a> Mixer<'a> {
    /// A mixer over `memory` ([`MIXER_BYTES`]: the DMA ring first, so its
    /// start must be aligned as the DMA engine needs); `speaker` turns on
    /// the speaker's filter.
    ///
    /// # Panics
    /// If `memory` isn't [`MIXER_BYTES`] long.
    pub fn new(memory: &'a mut [u8], speaker: bool) -> Self {
        assert_eq!(memory.len(), MIXER_BYTES, "the mixer's memory");
        let (ring, snaps) = memory.split_at_mut(DMA_RING_BYTES);
        snaps.fill(0xFF);
        Self {
            slots: Default::default(),
            generation: 0,
            ring,
            running: false,
            hw: 0,
            last_pos: 0,
            mixed_to: 0,
            state: chain::State::default(),
            snaps,
            speaker,
            dirty_all: false,
            last_interactive_ms: None,
            starting: false,
            ended: false,
            piece: 0,
            end_frame: 0,
            stats: Stats::default(),
        }
    }

    /// Opens a stream over `buf` (its ring, sized by [`min_ring_bytes`]: a
    /// smaller one is refused with the size it needs).
    pub fn open(
        &mut self,
        cfg: StreamConfig,
        buf: &'a mut [u8],
        now_ms: u32,
    ) -> Result<StreamId, OpenError> {
        let need = min_ring_bytes(&cfg)?;
        if buf.len() < need {
            return Err(OpenError::RingTooSmall {
                need,
                got: buf.len(),
            });
        }
        let ratio = Ratio::new(cfg.rate).map_err(|e| OpenError::BadRate(e.0))?;
        let Some(i) = self.slots.iter().position(Option::is_none) else {
            return Err(OpenError::Full);
        };
        self.generation = self.generation.wrapping_add(1);
        let ch = usize::from(cfg.channels);
        let volume = cfg.volume.min(2 * UNITY);
        self.slots[i] = Some(Slot {
            generation: self.generation,
            cap: (buf.len() / (2 * ch)) as u64,
            buf,
            ch,
            ratio,
            role: cfg.role,
            legacy: cfg.legacy,
            hint: cfg.low_latency,
            write: 0,
            ended: false,
            start: cfg.start,
            anchor: None,
            actual_start: None,
            placed_at: None,
            starved_at: None,
            dirty: None,
            paused_at: None,
            pause_req: None,
            paused_by_unplug: false,
            muted_by_unplug: false,
            volume,
            ramp: (volume, volume, 0),
            gain_req: false,
            release: 0,
            played: 0,
            last_commit_ms: None,
            last_write_ms: now_ms,
            small_run: 0,
        });
        Ok(StreamId {
            slot: i as u8,
            generation: self.generation,
        })
    }

    fn slot(&mut self, id: StreamId) -> Option<&mut Slot<'a>> {
        self.slots
            .get_mut(usize::from(id.slot))?
            .as_mut()
            .filter(|s| s.generation == id.generation)
    }

    fn slot_ref(&self, id: StreamId) -> Option<&Slot<'a>> {
        self.slots
            .get(usize::from(id.slot))?
            .as_ref()
            .filter(|s| s.generation == id.generation)
    }

    /// Closes a stream (its sound stops at the next wake) and gives its
    /// ring back.
    pub fn close(&mut self, id: StreamId) -> Option<&'a mut [u8]> {
        self.slot(id)?;
        let s = self.slots[usize::from(id.slot)].take()?;
        self.dirty_all = true;
        Some(s.buf)
    }

    /// Where the program may write now: two pieces of its ring (the second
    /// after the wrap), 16-bit little-endian samples interleaved, together
    /// the room.
    pub fn writable(&mut self, id: StreamId) -> Option<(&mut [u8], &mut [u8])> {
        let s = self.slot(id)?;
        let room = s.cap - (s.write - s.release);
        let at = (s.write % s.cap) as usize;
        let first = (room as usize).min(s.cap as usize - at);
        let ch = s.ch * 2;
        let (lo, hi) = s.buf.split_at_mut(at * ch);
        let (a, _) = hi.split_at_mut(first * ch);
        let (b, _) = lo.split_at_mut((room as usize - first) * ch);
        Some((a, b))
    }

    /// The program wrote `frames` frames where [`Self::writable`] said.
    pub fn commit(&mut self, id: StreamId, frames: u64, now_ms: u32) -> Result<(), WriteError> {
        let s = self.slot(id).ok_or(WriteError::NoStream)?;
        if s.ended {
            return Err(WriteError::Ended);
        }
        if frames > s.cap - (s.write - s.release) {
            return Err(WriteError::TooMuch);
        }
        if frames == 0 {
            return Ok(());
        }
        s.write += frames;
        // The write pattern (condition 13): small writes close together,
        // keeping a short queue, once it plays.
        let rate = u64::from(s.ratio.rate);
        let ms = (frames * 1_000 / rate) as u32;
        let queued = ((s.write - s.played) * 1_000 / rate) as u32;
        let close = s
            .last_commit_ms
            .is_some_and(|t| now_ms.wrapping_sub(t) <= SHORT_GAP_MS);
        if s.anchor.is_some() && ms <= SMALL_WRITE_MS && close && queued <= SHORT_QUEUE_MS {
            s.small_run = (s.small_run + 1).min(SMALL_RUN);
        } else if ms > LARGE_WRITE_MS || queued > LONG_QUEUE_MS {
            s.small_run = 0;
        }
        s.last_commit_ms = Some(now_ms);
        s.last_write_ms = now_ms;
        let interactive = s.hint || s.small_run >= SMALL_RUN;
        // Not placed yet: the next wake places it. Ran out: re-mix from
        // where it stopped.
        match (s.anchor, s.starved_at) {
            (None, _) => {
                let at = match s.start {
                    Start::Asap => 0,
                    Start::At(t) => t,
                };
                s.dirty = min_opt(s.dirty, at);
            }
            (Some(_), Some(at)) => s.dirty = min_opt(s.dirty, at),
            _ => {}
        }
        if interactive {
            self.last_interactive_ms = Some(now_ms);
        }
        Ok(())
    }

    /// Copies `samples` (interleaved) into the stream: as much as there is
    /// room for, in whole frames; how many frames.
    pub fn write(&mut self, id: StreamId, samples: &[i16], now_ms: u32) -> Result<u64, WriteError> {
        let ch = self.slot(id).ok_or(WriteError::NoStream)?.ch;
        let (a, b) = self.writable(id).ok_or(WriteError::NoStream)?;
        let n = (samples.len() / ch * ch).min((a.len() + b.len()) / 2);
        for (d, s) in a
            .as_chunks_mut::<2>()
            .0
            .iter_mut()
            .chain(b.as_chunks_mut::<2>().0.iter_mut())
            .zip(&samples[..n])
        {
            *d = s.to_le_bytes();
        }
        self.commit(id, (n / ch) as u64, now_ms)?;
        Ok((n / ch) as u64)
    }

    /// No more data: what's written plays out, then the stream is drained.
    pub fn end(&mut self, id: StreamId) {
        if let Some(s) = self.slot(id) {
            s.ended = true;
            if let Some(at) = s.starved_at {
                s.dirty = min_opt(s.dirty, at);
            }
        }
    }

    /// Sets a stream's volume (Q15, up to 2x), ramped from the next wake.
    pub fn set_volume(&mut self, id: StreamId, volume: u32) {
        if let Some(s) = self.slot(id) {
            s.volume = volume.min(2 * UNITY);
            s.gain_req = true;
        }
    }

    /// Pauses a stream from the next wake (its played position holds).
    pub fn pause(&mut self, id: StreamId) {
        if let Some(s) = self.slot(id) {
            s.pause_req = Some(true);
        }
    }

    /// Resumes a paused stream where it stopped.
    pub fn resume(&mut self, id: StreamId) {
        if let Some(s) = self.slot(id) {
            s.pause_req = Some(false);
            s.paused_by_unplug = false;
        }
    }

    /// The headphones came out (A2): each stream by its role's policy
    /// ([`on_unplug`]). The route to the speaker is the service's (S3).
    pub fn unplugged(&mut self) {
        for s in self.slots.iter_mut().flatten() {
            match on_unplug(s.role, s.legacy) {
                Unplug::Pause => {
                    s.pause_req = Some(true);
                    s.paused_by_unplug = true;
                }
                Unplug::Mute => {
                    s.muted_by_unplug = true;
                    s.gain_req = true;
                }
                Unplug::Speaker => {}
            }
        }
    }

    /// The headphones went back in: what unplugging muted is unmuted (a
    /// paused stream waits for its program or the user).
    pub fn replugged(&mut self) {
        for s in self.slots.iter_mut().flatten() {
            if s.muted_by_unplug {
                s.muted_by_unplug = false;
                s.gain_req = true;
            }
        }
    }

    /// A stream's state.
    #[must_use]
    pub fn status(&self, id: StreamId, now_ms: u32) -> Option<StreamStatus> {
        let s = self.slot_ref(id)?;
        Some(StreamStatus {
            actual_start: s.actual_start,
            placed_at: s.placed_at,
            played: s.played,
            written: s.write,
            room: s.cap - (s.write - s.release),
            drained: s.ended && s.played >= s.write,
            interactive: s.interactive(now_ms),
            paused_by_unplug: s.paused_by_unplug,
            muted_by_unplug: s.muted_by_unplug,
            starved: s.starved_at.is_some_and(|at| at <= self.hw),
        })
    }

    /// The output's clock: the frame the DMA reads next.
    #[must_use]
    pub fn clock(&self) -> u64 {
        self.hw
    }

    /// The output's clock read fresh from the DMA position (between wakes
    /// [`Self::clock`] holds the last wake's): for measuring the link's real
    /// rate. Exact: the engine can't run further ahead than the mix, which
    /// is less than a ring (ADR-0032 A6), and it counts only frames that
    /// went out (it stands still while stopped: a window holding a stop
    /// measures the rate wrongly, so [`Stats::stops`] says to drop it).
    pub fn clock_now(&mut self, dma: &mut dyn DmaRing) -> u64 {
        if self.running {
            self.peek_clock(dma)
        } else {
            self.hw
        }
    }

    /// Frames mixed ahead of the clock.
    #[must_use]
    pub fn lead(&self) -> u64 {
        self.mixed_to.saturating_sub(self.hw)
    }

    /// The counts so far.
    #[must_use]
    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// Whether the DMA runs.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Nothing open has anything left to play.
    #[must_use]
    pub fn is_quiet(&self) -> bool {
        self.slots.iter().flatten().all(|s| {
            s.paused_at.is_some() || (s.played >= s.write && s.anchor.is_some()) || s.write == 0
        })
    }

    /// The speaker's filter on (to the speaker) or off; from the next block
    /// mixed (the service re-routes at a wake).
    pub fn set_speaker(&mut self, speaker: bool) {
        if self.speaker != speaker {
            self.speaker = speaker;
            self.dirty_all = true;
        }
    }

    /// Starts the DMA over a silent ring (I2S must be running), then mixes;
    /// [`StreamError::Refused`] if the engine won't take the ring.
    pub fn start(&mut self, dma: &mut dyn DmaRing, now_ms: u32) -> Result<Tick, StreamError> {
        if self.running {
            return Ok(self.tick(dma, now_ms));
        }
        self.ring.fill(0);
        dma.filled(self.ring);
        if !dma.start(TX_PORT, self.ring) {
            return Err(StreamError::Refused);
        }
        // Pieces of whole blocks, at least two (the end's rule needs a
        // piece the engine isn't in).
        let piece = (dma.piece() / 4) as u64;
        if piece == 0 || !piece.is_multiple_of(BLOCK as u64) || piece * 2 > RING_FRAMES {
            dma.stop();
            return Err(StreamError::Refused);
        }
        self.piece = piece;
        self.ended = false;
        self.running = true;
        // The engine starts at the ring's first frame: the clock moves on to
        // the next whole lap so the two agree.
        self.hw = align_up(self.hw, RING_FRAMES);
        self.last_pos = 0;
        self.snaps.fill(0xFF);
        // An end from the start, in the first piece past the margin (the
        // ring is silence up to there): a ring that loops even for one
        // wake can be lapped by an engine faster than the mixer, replaying
        // what was just mixed.
        self.mixed_to = self.hw;
        self.set_end(dma, align_up(self.hw + MARGIN, self.piece));
        self.starting = true;
        Ok(self.tick(dma, now_ms))
    }

    /// Stops the DMA (the ring stops being read); the clock holds.
    pub fn stop(&mut self, dma: &mut dyn DmaRing) {
        if self.running {
            dma.stop();
            self.running = false;
            self.ended = false;
            // Whatever was mixed ahead never plays.
            self.mixed_to = self.hw;
        }
    }

    /// One wake: reads the DMA position, clears what played, places new
    /// streams, re-mixes what changed, mixes ahead, and says when to wake
    /// again. Call it on that timer and after every commit, volume change,
    /// pause or open (ADR-0010's events, later).
    pub fn tick(&mut self, dma: &mut dyn DmaRing, now_ms: u32) -> Tick {
        self.stats.wakes += 1;
        if !self.running {
            return Tick {
                next_ms: BACKGROUND_MS,
                ..Tick::default()
            };
        }
        // Read before the position: once stopped, the position is final.
        if self.ended && dma.stopped() {
            self.stats.stops += 1;
        }
        self.advance_clock(dma);
        let mut tick = Tick::default();

        // The earliest frame we may still write.
        let floor = align_up(self.hw + MARGIN, SNAP);
        if core::mem::take(&mut self.starting) {
            // The engine moved a little since it started over the silent
            // ring: not late, mixing starts from here.
            self.mixed_to = self.mixed_to.max(floor);
        } else if self.mixed_to < self.hw {
            // Too late: the DMA played past the mix (silence: the rest of
            // the end's piece was cleared, and what played before it).
            // Every playing stream continues after the gap, from where it
            // stopped: where the engine stops, at the end of the end's
            // piece (it may still be playing that piece's silence; with no
            // end yet, past the margin).
            self.stats.late += 1;
            let at = self.mixed_to;
            for s in self.slots.iter_mut().flatten() {
                if s.anchor.is_some() && s.starved_at.is_none() && s.paused_at.is_none() {
                    s.starved_at = Some(at);
                    s.dirty = min_opt(s.dirty, at);
                }
            }
            self.mixed_to = if self.ended {
                self.piece_end(at)
            } else {
                floor
            };
        }

        // Where re-mixing must start: the earliest change, not before the
        // floor.
        let mut dirty: Option<u64> = if self.dirty_all { Some(0) } else { None };
        for s in self.slots.iter().flatten() {
            if let Some(d) = s.dirty {
                dirty = min_opt(dirty, d);
            }
            if s.gain_req || s.pause_req.is_some() {
                dirty = min_opt(dirty, 0);
            }
        }
        let resume = match dirty {
            Some(d) => align_down(d, SNAP).max(floor).min(self.mixed_to),
            None => self.mixed_to,
        };
        // Changes take effect at `resume` (at most where mixing stands).
        let resume = if resume < self.mixed_to && self.restore(resume) {
            tick.rewrote = self.mixed_to - resume;
            self.stats.rewrites += 1;
            self.mixed_to = resume;
            resume
        } else {
            // No snapshot (or nothing to redo): changes from the mix's end.
            self.mixed_to
        };
        self.dirty_all = false;
        for s in self.slots.iter_mut().flatten() {
            Self::apply(s, resume, self.hw);
        }

        // Their room first: a program refills when the mixer wakes.
        self.update_played();

        // Mix ahead: how far depends on the mode, and only while every
        // playing stream's data is there (R2: so no sample plays that
        // wasn't mixed), but at least the short lead, so one program that
        // stopped writing can't silence the rest (it alone runs dry).
        tick.interactive = self.slots.iter().flatten().any(|s| s.interactive(now_ms))
            || self
                .last_interactive_ms
                .is_some_and(|t| now_ms.wrapping_sub(t) < HOLD_MS);
        let lead = if tick.interactive {
            INTERACTIVE_LEAD
        } else {
            BACKGROUND_LEAD
        };
        let data = self
            .slots
            .iter()
            .flatten()
            .filter_map(Slot::data_end)
            .min()
            .unwrap_or(u64::MAX)
            .max(self.hw + INTERACTIVE_LEAD);
        let target = align_down((self.hw + lead).min(data), BLOCK as u64)
            .min(self.max_end())
            .max(self.mixed_to);
        let from = self.mixed_to;
        self.mix(from, target, dma);
        // The end's piece must start past the engine by the margin (it
        // moved while mixing): mix on until it does (only on a nearly-late
        // wake; a stream without data runs dry there).
        loop {
            let now = self.peek_clock(dma);
            let next = self.mixed_to + BLOCK as u64;
            if align_down(self.mixed_to, self.piece) >= now + MARGIN || next > self.max_end() {
                break;
            }
            self.mix(self.mixed_to, next, dma);
        }
        tick.mixed = self.mixed_to - from;
        self.stats.mixed += tick.mixed;
        self.stats.rewritten += tick.rewrote;
        self.end_ring(dma);

        self.update_played();

        // Wake at the mode's interval, counted from where the DMA is now
        // (mixing took time), and at the latest when the lead has dropped
        // to its low-water mark (the lead less one interval): a program
        // gets its room at a wake, so it has that long to refill (a
        // decoder's work) before the mix would run out.
        let now = self.peek_clock(dma);
        let to_ms = |f: u64| (f * 1_000 / u64::from(OUT)) as u32;
        let ahead_ms = to_ms(self.mixed_to.saturating_sub(now + MARGIN));
        let interval = if tick.interactive {
            INTERACTIVE_MS
        } else {
            BACKGROUND_MS
        };
        let low_water = to_ms(lead).saturating_sub(interval);
        tick.next_ms = interval
            .min(ahead_ms.saturating_sub(low_water))
            .min(ahead_ms.saturating_sub(SLACK_MS))
            .max(1);
        tick
    }

    /// The end of the piece holding output frame `f`.
    fn piece_end(&self, f: u64) -> u64 {
        align_down(f, self.piece) + self.piece
    }

    /// How far the mix may go (exclusive, whole blocks): the end's rule
    /// (ADR-0032 A6, `rp_hal_pxa27x::dmaring`): the piece the mix ends in
    /// ends no further than one ring past the start of the piece holding
    /// the last frame the engine played, so it is never the piece just
    /// behind the engine (*lead < ring − one piece*).
    fn max_end(&self) -> u64 {
        align_down(self.hw.saturating_sub(1), self.piece) + RING_FRAMES - BLOCK as u64
    }

    /// Clears the rest of the piece the mix ends in (the engine plays it
    /// before it stops: a shortening rewrite leaves the old mix there,
    /// audit E4), then moves the ring's end to the mix's end.
    fn end_ring(&mut self, dma: &mut dyn DmaRing) {
        let (from, to) = (self.mixed_to, self.piece_end(self.mixed_to));
        self.clear(from, to, dma);
        self.set_end(dma, from);
    }

    /// Moves the ring's end to output frame `at` (everything before it
    /// written for its time, the rest of its piece silence).
    fn set_end(&mut self, dma: &mut dyn DmaRing, at: u64) {
        self.end_frame = at;
        match dma.end_at((at % RING_FRAMES) as usize * 4) {
            RingEnd::Running | RingEnd::Stopped => {}
            RingEnd::Restarted => self.stats.restarts += 1,
            RingEnd::Raced => {
                self.stats.restarts += 1;
                self.stats.races += 1;
            }
            RingEnd::Failed => self.stats.ring_failed += 1,
        }
        self.ended = true;
    }

    /// Applies a stream's pending changes at output frame `at` (where mixing
    /// resumes).
    fn apply(s: &mut Slot<'_>, at: u64, hw: u64) {
        if s.anchor.is_none() && s.write > 0 {
            // Placed: at its requested time if that's still ahead, else now.
            let start = match s.start {
                Start::Asap => at,
                Start::At(t) => t.max(at),
            };
            s.anchor = Some((start, 0));
            s.actual_start = Some(start);
            s.placed_at = Some(hw);
            s.ramp = (s.target(), s.target(), start);
        }
        if let (Some(stop), Some((_, _))) = (s.starved_at, s.anchor)
            && s.dirty.is_some()
        {
            if stop >= at {
                // Re-mixed from before it ran out: seamless.
                s.starved_at = None;
            } else if s.write > s.src_at(stop).unwrap_or(0) || s.ended {
                // It played silence from `stop`: carries on after the gap.
                let from = s.src_at(stop).unwrap_or(0);
                s.anchor = Some((at, from));
                s.starved_at = None;
            }
        }
        s.dirty = None;
        match s.pause_req.take() {
            Some(true) if s.paused_at.is_none() => s.paused_at = Some(at),
            Some(false) => {
                if let Some(p) = s.paused_at {
                    // Carries on from the frame it stopped at.
                    if let Some(from) = s.src_at(p) {
                        s.anchor = Some((at, from));
                    }
                    s.paused_at = None;
                }
            }
            _ => {}
        }
        if core::mem::take(&mut s.gain_req) {
            s.ramp = (s.gain_at(at), s.target(), at);
        }
    }

    /// Puts the chain's state back to what it was at `at` (a [`SNAP`]
    /// boundary mixed before); false if that state isn't kept.
    fn restore(&mut self, at: u64) -> bool {
        let i = (at / SNAP) as usize % SNAPS * SNAP_BYTES;
        let Some(b) = self.snaps.get(i..i + SNAP_BYTES) else {
            return false;
        };
        let (when, st) = b.split_at(8);
        let when = u64::from_le_bytes(when.try_into().unwrap_or([0xFF; 8]));
        let Ok(st) = st.try_into() else {
            return false;
        };
        if when != at {
            return false;
        }
        self.state = chain::State::load(st);
        true
    }

    /// Keeps the chain's state at `at` (a [`SNAP`] boundary).
    fn keep(&mut self, at: u64) {
        let i = (at / SNAP) as usize % SNAPS * SNAP_BYTES;
        if let Some(b) = self.snaps.get_mut(i..i + SNAP_BYTES) {
            b[..8].copy_from_slice(&at.to_le_bytes());
            b[8..].copy_from_slice(&self.state.save());
        }
    }

    /// Mixes `[from, to)` (whole blocks) into the ring.
    fn mix(&mut self, from: u64, to: u64, dma: &mut dyn DmaRing) {
        let mut b = from;
        while b < to {
            if b.is_multiple_of(SNAP) {
                self.keep(b);
            }
            let mut acc = [0i32; 2 * BLOCK];
            for s in self.slots.iter_mut().flatten() {
                if s.add(b, &mut acc) {
                    self.stats.starved += 1;
                }
            }
            let at = (b % RING_FRAMES) as usize * 4;
            let out = &mut self.ring[at..at + 4 * BLOCK];
            let r = self.state.run(&mut acc, self.speaker, out);
            // Handed to the engine at once, nearest first: the margin need
            // only cover one block's mixing, however long the stretch.
            dma.filled(out);
            self.stats.clipped += r.clipped;
            self.stats.limited += u32::from(r.limited);
            b += BLOCK as u64;
            // A whole piece mixed past the ring's end: the end moves on to
            // it now, not after the whole stretch, so an engine that was
            // already stopping (the race) or stopped is started again
            // within about two pieces' mixing (under 1 ms on the X51v), not
            // after a long mix. The piece is full: nothing to clear.
            if self.ended && b.is_multiple_of(self.piece) && b > self.piece_end(self.end_frame) {
                self.mixed_to = b;
                self.set_end(dma, b - 1);
            }
        }
        self.mixed_to = to;
    }

    /// Calls `f` on the ring's bytes for frames `[from, to)`, one
    /// contiguous piece at a time (two where it wraps).
    fn each_piece(&mut self, from: u64, to: u64, f: &mut dyn FnMut(&mut [u8])) {
        let mut t = from;
        while t < to {
            let at = (t % RING_FRAMES) as usize;
            let n = ((to - t) as usize).min(RING_FRAMES as usize - at);
            f(&mut self.ring[at * 4..(at + n) * 4]);
            t += n as u64;
        }
    }

    /// Moves the clock on by what the engine played since the last wake:
    /// the position alone, exactly (the ring's end keeps the engine within
    /// the mix, less than a ring ahead: no lap to guess, ADR-0032 A6).
    fn advance_clock(&mut self, dma: &mut dyn DmaRing) {
        let Some(pos) = dma.position() else {
            return;
        };
        let pos = (pos / 4) as u64 % RING_FRAMES;
        let delta = (pos + RING_FRAMES - self.last_pos) % RING_FRAMES;
        let old = self.hw;
        self.hw += delta;
        self.last_pos = pos;
        // What played is cleared: if the mixer is ever late, the DMA plays
        // silence, never a lap-old sound.
        let clear_from = old.max(self.hw.saturating_sub(RING_FRAMES));
        self.clear(clear_from, self.hw, dma);
    }

    /// Where the DMA is now, from its position (the clock not moved: no
    /// lap can pass within one wake).
    fn peek_clock(&mut self, dma: &mut dyn DmaRing) -> u64 {
        match dma.position() {
            Some(pos) => {
                let pos = (pos / 4) as u64 % RING_FRAMES;
                self.hw + (pos + RING_FRAMES - self.last_pos) % RING_FRAMES
            }
            None => self.hw,
        }
    }

    fn clear(&mut self, from: u64, to: u64, dma: &mut dyn DmaRing) {
        self.each_piece(from, to, &mut |part| {
            part.fill(0);
            dma.filled(part);
        });
    }

    fn update_played(&mut self) {
        let hw = self.hw;
        for s in self.slots.iter_mut().flatten() {
            if let Some(p) = s.src_at(hw) {
                s.played = p.min(s.write);
                let reach = u64::from(s.ratio.reach());
                s.release = s.played.saturating_sub(reach + 1);
            }
        }
    }
}

#[cfg(test)]
mod tests;
