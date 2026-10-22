//! A ring of DMA descriptors looping a buffer into a device, which can be
//! given an end (ADR-0032 A6, option B: the audio DMA stops where the
//! mixed sound ends). Pure logic, host-tested against a model engine;
//! [`crate::dma`] does the registers and the cache.
//!
//! The buffer is cut into equal **pieces**, one descriptor each, the last
//! linked back to the first: as many pieces as the channel's chain holds,
//! each whole cache lines (the audio ring: 32 pieces of 1,408 bytes, 352
//! frames, 7.9 ms). With no end the engine loops for ever, as before.
//!
//! **The end.** The controller loads a descriptor into its registers when
//! it starts it and never reads it again (PXA27x manual; the reason
//! [`crate::dmachain`] works). So the end is the STOP bit in the link of
//! the piece holding the first byte not yet written: the engine plays that
//! piece (the rest of it silence, the writer's part) and stops. Moving the
//! end is two 16-byte writes, **the new STOP first, then the old one
//! cleared**: for a moment two pieces carry STOP and the engine stops at
//! the first it reaches; the other order would leave a moment with none,
//! and an engine passing then would go round the whole ring.
//!
//! **The race.** If the engine has already loaded the old end's piece when
//! the end moves on, clearing its STOP is too late: it stops there, short
//! of the new end. The registers tell it apart: `DDADR` (the link of the
//! piece the engine is in) carries STOP, and it names the next piece, so
//! the piece the engine is in is the one before. Then the engine is waited
//! for (it stops within a piece) and started again at once at the next
//! piece; the device bench measured that restart at 8 us (docs/hw/audio.md,
//! 2026-10-19).
//!
//! **The rule the writer keeps** (the invariant the race argument and the
//! tests rest on): the end's piece starts past where the engine reads, and
//! ends no further than one ring past the start of the piece the engine
//! reads, or last read if it is stopped or between pieces (so *lead < ring
//! − one piece*, strictly: an end one ring on would be the piece just
//! behind a stopped engine, which can't be told from an end it has
//! already played). An end in the engine's own piece
//! breaks it (the engine loaded that piece without STOP, so it would go on
//! round): reported as a failure, and contained by moving the end one
//! piece on (at most a piece of what's there plays, never a lap).

use crate::dmachain::{SLOT, STOP, Seen};

/// Most bytes in a piece (under the 8,191 the length field holds).
pub const MAX_PIECE: u32 = 4096;
/// A piece is a whole number of these (a cache line, a 32-byte burst).
pub const LINE: u32 = 32;

/// The channel, as moving the end needs it.
pub trait Engine {
    /// The registers: `DCSR` first (stopped), then `DDADR`, `DSADR`,
    /// `DTADR` (once stopped, those read after it are final).
    fn seen(&mut self) -> Seen;
    /// Writes descriptor slot `slot`'s link word and makes it visible to
    /// the controller before returning (written back from the cache).
    fn link(&mut self, slot: usize, word: u32);
    /// Starts the stopped channel at descriptor address `at`.
    fn restart(&mut self, at: u32);
}

/// What the registers say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Look {
    /// Moving, in this piece, its link without STOP.
    Moving(u32),
    /// Moving in this piece, which it loaded with STOP: it stops at its end.
    Stopping(u32),
    /// Stopped after a piece: this one is next.
    Stopped(u32),
    /// Registers that make no sense for this ring.
    Bad,
}

/// How a ring of `len` bytes is cut for a chain of `slots` descriptors:
/// the bytes per piece, as many pieces as the chain holds, each whole
/// lines and at most [`MAX_PIECE`]; `None` without at least two.
#[must_use]
pub fn cut(len: u32, slots: u32) -> Option<u32> {
    let mut n = slots.min(len / LINE);
    while n >= 2 {
        let piece = len / n;
        if piece * n == len && piece.is_multiple_of(LINE) && piece <= MAX_PIECE {
            return Some(piece);
        }
        n -= 1;
    }
    None
}

/// A ring running through a chain of descriptors at physical address
/// `base`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ring {
    /// The buffer (physical) and its length.
    addr: u32,
    len: u32,
    /// Bytes per piece, and how many.
    piece: u32,
    pieces: u32,
    base: u32,
    /// The piece whose link carries STOP, once there is an end.
    end: Option<u32>,
    /// Why the last [`Ring::end_at`] that failed did, and the registers it
    /// read then (for logs: a failure is never expected).
    failure: Option<(&'static str, Seen)>,
}

impl Ring {
    /// Writes the looping chain for `len` bytes at `addr` into `chain`
    /// (each piece to `dst`, command `cmd` without its length); the caller
    /// writes `chain` back to memory and starts the controller at `base`.
    /// `None` if it can't be cut ([`cut`]) or isn't aligned.
    pub fn start(
        addr: u32,
        len: u32,
        dst: u32,
        cmd: u32,
        chain: &mut [[u32; 4]],
        base: u32,
    ) -> Option<Self> {
        let piece = cut(len, chain.len() as u32)?;
        if !addr.is_multiple_of(LINE)
            || !base.is_multiple_of(SLOT)
            || addr.checked_add(len).is_none()
        {
            return None;
        }
        let pieces = len / piece;
        let ring = Self {
            addr,
            len,
            piece,
            pieces,
            base,
            end: None,
            failure: None,
        };
        for (i, d) in (0..pieces).zip(chain.iter_mut()) {
            *d = [ring.link(i), addr + i * piece, dst, cmd | piece];
        }
        Some(ring)
    }

    /// Bytes per piece.
    #[must_use]
    pub const fn piece(&self) -> u32 {
        self.piece
    }

    /// Pieces in the ring (the chain's slots used).
    #[must_use]
    pub const fn pieces(&self) -> u32 {
        self.pieces
    }

    /// Why the last failed [`Ring::end_at`] failed, and what it read.
    #[must_use]
    pub const fn failure(&self) -> Option<(&'static str, Seen)> {
        self.failure
    }

    /// The piece carrying the end, if any (tests, logs).
    #[must_use]
    pub const fn end(&self) -> Option<u32> {
        self.end
    }

    /// Where the engine reads next, as an offset in the ring, from `DSADR`
    /// (the end of the ring is its start again); `None` outside it.
    #[must_use]
    pub const fn offset(&self, dsadr: u32) -> Option<u32> {
        let at = dsadr.wrapping_sub(self.addr);
        if at == self.len {
            Some(0)
        } else if at < self.len {
            Some(at)
        } else {
            None
        }
    }

    /// The engine has stopped at the end (or short of it).
    #[must_use]
    pub const fn stopped(&self, seen: Seen) -> bool {
        self.end.is_some() && seen.stopped
    }

    /// Right after the chain is started at slot 0 (`DDADR` written with
    /// `base`): whether the controller has loaded the first piece. Until
    /// then `DSADR` is the previous transfer's, and after a stop and start
    /// of the same ring it lies inside the ring and reads as a position
    /// (quality audit C56). Loading piece 0 puts its link (slot 1) in
    /// `DDADR`. Only meaningful in the first piece: in the last piece
    /// `DDADR` names slot 0 again.
    #[must_use]
    pub fn first_loaded(&self, s: Seen) -> bool {
        s.stopped || self.slot_of(s.ddadr) != Some(0)
    }

    /// The slot `DDADR` names, if it is one of this ring's.
    fn slot_of(&self, ddadr: u32) -> Option<u32> {
        let off = (ddadr & !(SLOT - 1)).wrapping_sub(self.base);
        (off.is_multiple_of(SLOT) && off / SLOT < self.pieces).then_some(off / SLOT)
    }

    fn look(&self, s: Seen) -> Look {
        let at = self.offset(s.dsadr);
        if !s.stopped && s.ddadr & STOP == 0 && at.is_none() && self.slot_of(s.ddadr) == Some(0) {
            // Started, its first piece not loaded yet (`DSADR` still a
            // previous transfer's): in no piece.
            return Look::Moving(u32::MAX);
        }
        let Some(at) = at else {
            return Look::Bad;
        };
        if s.stopped {
            // Stopped at a STOP link: after a whole piece, `DSADR` at the
            // next one's start (the bench's restart rests on this).
            if !at.is_multiple_of(self.piece) {
                return Look::Bad;
            }
            return Look::Stopped(at / self.piece);
        }
        // Moving: `DDADR` is the link of the piece it is in.
        let Some(next) = self.slot_of(s.ddadr) else {
            return Look::Bad;
        };
        let c = if next == 0 { self.pieces - 1 } else { next - 1 };
        // `DSADR` within that piece (its end included; the ring's end
        // reads as 0) or the registers aren't this ring's.
        let start = c * self.piece;
        let within = (at >= start && at <= start + self.piece) || (c + 1 == self.pieces && at == 0);
        if !within {
            return Look::Bad;
        }
        if s.ddadr & STOP != 0 {
            Look::Stopping(c)
        } else {
            Look::Moving(c)
        }
    }

    /// Piece `i`'s link word without STOP: the next piece's slot.
    const fn link(&self, i: u32) -> u32 {
        self.base + SLOT * if i + 1 < self.pieces { i + 1 } else { 0 }
    }

    /// Puts STOP on piece `k`'s link and takes it off the old end's: the
    /// new one first.
    fn move_end(&mut self, k: u32, eng: &mut dyn Engine) {
        if self.end == Some(k) {
            return;
        }
        eng.link(k as usize, self.link(k) | STOP);
        if let Some(j) = self.end {
            eng.link(j as usize, self.link(j));
        }
        self.end = Some(k);
    }

    /// The sound ends before byte `at` of the ring: moves the end to the
    /// piece holding it and makes the engine play up to there (see the
    /// module notes; `polls`: how many register reads the race's wait may
    /// take, more than a piece's time).
    pub fn end_at(&mut self, at: u32, eng: &mut dyn Engine, polls: u32) -> rp_hal_api::RingEnd {
        use rp_hal_api::RingEnd;
        if at >= self.len {
            return RingEnd::Failed;
        }
        let k = at / self.piece;
        self.move_end(k, eng);
        let mut s = eng.seen();
        let first = s;
        let raced = match self.look(s) {
            Look::Moving(c) if c == k => {
                // The rule broken: it loaded the end's piece before its STOP
                // and would go round. One piece on, so at most that plays.
                let k1 = if k + 1 == self.pieces { 0 } else { k + 1 };
                self.move_end(k1, eng);
                self.failure = Some(("the end in the engine's own piece", s));
                return RingEnd::Failed;
            }
            Look::Moving(_) => return RingEnd::Running,
            Look::Stopping(c) if c == k => return RingEnd::Running,
            Look::Stopping(_) => {
                // The race: it stops at the old end's piece; wait for it.
                let mut n = 0;
                while !s.stopped {
                    if n == polls {
                        self.failure = Some(("the race's wait ran out", first));
                        return RingEnd::Failed;
                    }
                    n += 1;
                    s = eng.seen();
                }
                true
            }
            Look::Stopped(_) => false,
            Look::Bad => {
                self.failure = Some(("registers that make no sense", s));
                return RingEnd::Failed;
            }
        };
        let Look::Stopped(r) = self.look(s) else {
            self.failure = Some(("stopped mid-piece after the race", s));
            return RingEnd::Failed;
        };
        if r == if k + 1 == self.pieces { 0 } else { k + 1 } {
            // Stopped just after the new end's piece: nothing past it.
            return RingEnd::Stopped;
        }
        eng.restart(self.base + SLOT * r);
        if raced {
            RingEnd::Raced
        } else {
            RingEnd::Restarted
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use rp_hal_api::RingEnd;
    use std::vec;
    use std::vec::Vec;

    const CHAIN: u32 = 0x1000;
    const BUF: u32 = 0xA010_0000;
    const DEVICE: u32 = 0x4040_0080;
    const LEN: u32 = 32 * 1408;
    const FRAMES: u64 = (LEN / 4) as u64;
    const PIECE_FRAMES: u64 = 352;
    const N: u32 = 32;
    /// The writer's margin (5 ms, the mixer's), in frames.
    const MARGIN: u64 = 221;

    /// The DMA controller as the PXA27x manual describes it: loads a
    /// descriptor from memory, moves its bytes a 32-byte burst (8 frames)
    /// at a time, follows the link or stops. It moves on while the CPU
    /// works: `jitter` steps around each link write and now and then one
    /// at a register read (never more than the writer's margin in all),
    /// and now and then the CPU is held up between the two link writes of
    /// a move (an interrupt) for up to `held` steps, longer than any
    /// margin: only the order of the two writes keeps the end then.
    struct Model {
        /// The chain as the controller reads it (memory).
        memory: Vec<[u32; 4]>,
        /// The ring's frames (what the writer put there).
        ring: Vec<u32>,
        ddadr: u32,
        dsadr: u32,
        left: u32,
        running: bool,
        /// Every frame played, in order.
        played: Vec<u32>,
        rng: Lcg,
        jitter: u32,
        held: u64,
        /// Link writes in this move so far.
        links: u32,
        restarts: u32,
    }

    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u32 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 33) as u32
        }
        fn below(&mut self, n: u64) -> u64 {
            u64::from(self.next()) % n.max(1)
        }
    }

    impl Model {
        fn new(seed: u64, jitter: u32) -> (Self, Ring) {
            let mut chain = vec![[0xDEAD_BEEF; 4]; N as usize];
            let ring = Ring::start(BUF, LEN, DEVICE, 0x9000_0000, &mut chain, CHAIN).unwrap();
            let m = Self {
                memory: chain,
                ring: vec![0; FRAMES as usize],
                ddadr: CHAIN,
                // Stale: a previous transfer's.
                dsadr: BUF + 0x40_0000,
                left: 0,
                running: true,
                played: Vec::new(),
                rng: Lcg(seed),
                jitter,
                held: 0,
                links: 0,
                restarts: 0,
            };
            (m, ring)
        }

        /// One step: a load or a burst. False once stopped.
        fn step(&mut self) -> bool {
            if !self.running {
                return false;
            }
            if self.left == 0 {
                let slot = ((self.ddadr & !15) - CHAIN) / SLOT;
                let d = self.memory[slot as usize];
                self.ddadr = d[0];
                self.dsadr = d[1];
                self.left = d[3] & 0x1FFF;
                return true;
            }
            for _ in 0..8 {
                let f = ((self.dsadr - BUF) / 4) as usize;
                self.played.push(self.ring[f]);
                self.dsadr += 4;
            }
            self.left -= 32;
            if self.left == 0 && self.ddadr & STOP != 0 {
                self.running = false;
            }
            true
        }

        /// Plays until `frames` more frames went out (or it stops).
        fn play(&mut self, frames: u64) {
            let to = self.played.len() as u64 + frames;
            while (self.played.len() as u64) < to && self.step() {}
        }

        fn wander(&mut self) {
            let n = self.rng.below(u64::from(self.jitter) + 1);
            for _ in 0..n {
                self.step();
            }
        }
    }

    impl Engine for Model {
        fn seen(&mut self) -> Seen {
            if self.rng.below(4) == 0 {
                self.step();
            }
            Seen {
                stopped: !self.running,
                ddadr: self.ddadr,
                dsadr: self.dsadr,
                dtadr: DEVICE,
            }
        }
        fn link(&mut self, slot: usize, word: u32) {
            // Steps before the line reaches memory, and after.
            self.wander();
            self.memory[slot][0] = word;
            if self.links == 0 && self.rng.below(4) == 0 {
                let n = self.rng.below(self.held + 1);
                for _ in 0..n {
                    self.step();
                }
            } else {
                self.wander();
            }
            self.links += 1;
        }
        fn restart(&mut self, at: u32) {
            assert!(!self.running, "restarted while running");
            self.ddadr = at;
            self.left = 0;
            self.running = true;
            self.restarts += 1;
        }
    }

    fn tag(k: u64, generation: u32) -> u32 {
        (1 << 31) | ((generation & 0x7FF) << 20) | (k as u32 & 0xF_FFFF)
    }

    fn piece_end(f: u64) -> u64 {
        (f / PIECE_FRAMES + 1) * PIECE_FRAMES
    }

    /// The writer (the mixer's part, modelled): at each wake it writes
    /// ahead of the engine to a new end chosen at random within the rule,
    /// sometimes re-writing from just past the margin (and so sometimes
    /// shortening the end: audit E4's case), clears the rest of the end's
    /// piece, and moves the end; between wakes the engine plays on alone
    /// for a random time, sometimes far longer than the ring. Every frame
    /// played must be the one written for its time, or silence where
    /// nothing was: never a stale lap, never a stale tail. Returns how
    /// often each [`RingEnd`] came back.
    fn run(seed: u64, wakes: u32, jitter: u32, held: u64, max_gap: u64) -> [u32; 5] {
        let (mut m, mut ring) = Model::new(seed, jitter);
        m.held = held;
        let mut rng = Lcg(seed ^ 0x5555);
        // What each output frame (counted from the engine's start) is.
        let mut meant: Vec<u32> = vec![0; 1 << 20];
        let mut end: u64 = 0;
        let mut generation = 0u32;
        let mut counts = [0u32; 5];
        for _ in 0..wakes {
            let pos = m.played.len() as u64;
            if (pos + 2 * FRAMES) as usize >= meant.len() {
                break;
            }
            // Where writing starts. Past the end (in its silent tail, or
            // stopped after it): where the engine stops, so the sound
            // carries on there after the gap. Else the end, or a rewrite
            // from past the margin.
            let from = if pos > end {
                piece_end(end)
            } else if end >= pos + MARGIN && rng.below(3) == 0 {
                pos + MARGIN + rng.below(end - pos - MARGIN + 1)
            } else {
                end
            };
            // The new end: its piece starts at least the margin past the
            // engine and ends within a ring of the start of the engine's
            // (lead < ring - piece).
            let lo = (pos + MARGIN).div_ceil(PIECE_FRAMES) * PIECE_FRAMES;
            // (The engine's piece: the one holding the last frame it read.)
            let last = pos.max(1) - 1;
            let hi = (last / PIECE_FRAMES + u64::from(N)) * PIECE_FRAMES - 1;
            let new_end = (lo + rng.below(hi - lo + 1)).max(from);
            if from < end {
                generation += 1;
            }
            for k in from..new_end {
                meant[k as usize] = tag(k, generation);
                m.ring[(k % FRAMES) as usize] = tag(k, generation);
            }
            // Nothing past the end plays: silence to the end of its piece.
            for k in new_end..piece_end(new_end) {
                meant[k as usize] = 0;
                m.ring[(k % FRAMES) as usize] = 0;
            }
            m.links = 0;
            let r = ring.end_at(((new_end % FRAMES) * 4) as u32, &mut m, 50_000);
            end = new_end;
            counts[match r {
                RingEnd::Running => 0,
                RingEnd::Restarted => 1,
                RingEnd::Raced => 2,
                RingEnd::Stopped => 3,
                RingEnd::Failed => 4,
            }] += 1;
            assert_ne!(r, RingEnd::Failed, "seed {seed}");
            // Whatever it found, the engine now plays on to the new end (or,
            // held up past it, has stopped there).
            let at_end = m.played.len() as u64 == piece_end(new_end);
            assert!(
                m.running || at_end,
                "left stopped short of the end (seed {seed})"
            );
            if m.ddadr & STOP != 0 {
                let next = ((m.ddadr & !15) - CHAIN) / SLOT;
                assert_eq!(Some((next + N - 1) % N), ring.end(), "would stop short");
            }
            let gap = if rng.below(8) == 0 {
                rng.below(3 * FRAMES)
            } else {
                rng.below(max_gap)
            };
            m.play(gap);
        }
        m.play(2 * FRAMES);
        assert!(!m.running, "never stopped at the end");
        assert_eq!(m.played.len() as u64, piece_end(end), "stopped elsewhere");
        for (k, &f) in m.played.iter().enumerate() {
            assert_eq!(f, meant[k], "frame {k} (seed {seed})");
        }
        counts
    }

    #[test]
    fn every_frame_played_is_the_one_written_for_its_time_or_silence() {
        let mut all = [0u32; 5];
        for seed in 0..60 {
            // Held up for up to 4 pieces (32 ms): past the end now and then.
            let c = run(seed, 300, 6, 4 * PIECE_FRAMES / 8, 9_000);
            for (a, b) in all.iter_mut().zip(c) {
                *a += b;
            }
        }
        // Every path was taken: running on, restarts after a stall, races.
        assert!(all[0] > 1_000, "{all:?}");
        assert!(all[1] > 100, "{all:?}");
        assert!(all[2] > 10, "{all:?}");
    }

    #[test]
    fn a_ring_with_no_end_loops_as_before() {
        let (mut m, _ring) = Model::new(1, 0);
        for (k, f) in m.ring.iter_mut().enumerate() {
            *f = k as u32 + 1;
        }
        m.play(3 * FRAMES + 5);
        assert!(m.running);
        for (k, &f) in m.played.iter().enumerate() {
            assert_eq!(f, (k as u64 % FRAMES) as u32 + 1);
        }
    }

    #[test]
    fn the_audio_ring_is_cut_in_32_pieces_of_1408_bytes() {
        assert_eq!(cut(45_056, 32), Some(1_408));
        // S1's and the link-rate bench's 8 KiB ring.
        assert_eq!(cut(8_192, 32), Some(256));
        // Fewer slots: fewer, larger pieces, still whole lines.
        assert_eq!(cut(45_056, 11), Some(4_096));
        assert_eq!(cut(12_288, 32), Some(384));
        // No cut with two whole-line pieces of at most 4 KiB.
        assert_eq!(cut(32, 32), None);
        assert_eq!(cut(100, 32), None);
        assert_eq!(cut(64 * 4096, 32), None);
        let mut chain = [[0; 4]; 32];
        assert!(Ring::start(BUF + 4, LEN, DEVICE, 0, &mut chain, CHAIN).is_none());
        assert!(Ring::start(BUF, LEN, DEVICE, 0, &mut chain, CHAIN + 4).is_none());
        let r = Ring::start(BUF, LEN, DEVICE, 7 << 16, &mut chain, CHAIN).unwrap();
        assert_eq!((r.piece(), r.pieces(), r.end()), (1_408, 32, None));
        assert_eq!(chain[0], [CHAIN + 16, BUF, DEVICE, (7 << 16) | 1_408]);
        assert_eq!(
            chain[31],
            [CHAIN, BUF + 31 * 1_408, DEVICE, (7 << 16) | 1_408]
        );
    }

    #[test]
    fn the_end_of_the_ring_reads_as_its_start() {
        let mut chain = [[0; 4]; 32];
        let r = Ring::start(BUF, LEN, DEVICE, 0, &mut chain, CHAIN).unwrap();
        assert_eq!(r.offset(BUF), Some(0));
        assert_eq!(r.offset(BUF + LEN - 4), Some(LEN - 4));
        assert_eq!(r.offset(BUF + LEN), Some(0));
        assert_eq!(r.offset(BUF + LEN + 4), None);
        assert_eq!(r.offset(BUF - 4), None);
    }

    #[test]
    fn an_end_in_the_engine_s_own_piece_fails_and_holds_a_piece_on() {
        let (mut m, mut ring) = Model::new(3, 0);
        // The engine is in piece 5, loaded without STOP.
        m.play(5 * PIECE_FRAMES + 10);
        let at = (5 * PIECE_FRAMES as u32 + 100) * 4;
        assert_eq!(ring.end_at(at, &mut m, 50_000), RingEnd::Failed);
        assert_eq!(ring.end(), Some(6));
        m.play(2 * FRAMES);
        assert_eq!(m.played.len() as u64, 7 * PIECE_FRAMES, "went round");
    }

    #[test]
    fn a_stopped_engine_at_the_new_end_is_left_stopped() {
        let (mut m, mut ring) = Model::new(4, 0);
        assert_eq!(ring.end_at(3 * 1_408, &mut m, 50_000), RingEnd::Running);
        m.play(FRAMES);
        assert!(!m.running);
        assert_eq!(m.played.len() as u64, 4 * PIECE_FRAMES);
        assert_eq!(ring.end_at(3 * 1_408 + 8, &mut m, 50_000), RingEnd::Stopped);
        assert!(!m.running);
        // A real end further on starts it where it stopped.
        assert_eq!(ring.end_at(9 * 1_408, &mut m, 50_000), RingEnd::Restarted);
        assert!(m.running);
        assert_eq!(m.restarts, 1);
        m.play(FRAMES);
        assert_eq!(m.played.len() as u64, 10 * PIECE_FRAMES);
    }

    #[test]
    fn the_race_waits_for_the_stop_and_starts_it_again_at_once() {
        let (mut m, mut ring) = Model::new(5, 0);
        assert_eq!(ring.end_at(3 * 1_408, &mut m, 50_000), RingEnd::Running);
        // The engine has started the end's piece (loaded with STOP).
        m.play(3 * PIECE_FRAMES + 40);
        assert!(m.running && m.ddadr & STOP != 0);
        assert_eq!(ring.end_at(8 * 1_408, &mut m, 50_000), RingEnd::Raced);
        assert!(m.running);
        assert_eq!(
            m.played.len() as u64,
            4 * PIECE_FRAMES,
            "restarted after the piece"
        );
        m.play(FRAMES);
        assert_eq!(m.played.len() as u64, 9 * PIECE_FRAMES);
        // A wait that runs out fails.
        let (mut m, mut ring) = Model::new(6, 0);
        ring.end_at(3 * 1_408, &mut m, 50_000);
        m.play(3 * PIECE_FRAMES + 40);
        assert_eq!(ring.end_at(8 * 1_408, &mut m, 1), RingEnd::Failed);
    }

    #[test]
    fn registers_that_make_no_sense_fail() {
        struct Wild;
        impl Engine for Wild {
            fn seen(&mut self) -> Seen {
                Seen {
                    stopped: true,
                    ddadr: CHAIN | STOP,
                    dsadr: BUF + 100,
                    dtadr: 0,
                }
            }
            fn link(&mut self, _: usize, _: u32) {}
            fn restart(&mut self, _: u32) {
                panic!("restarted mid-piece");
            }
        }
        let mut chain = [[0; 4]; 32];
        let mut r = Ring::start(BUF, LEN, DEVICE, 0, &mut chain, CHAIN).unwrap();
        assert_eq!(r.end_at(9 * 1_408, &mut Wild, 10), RingEnd::Failed);
        assert_eq!(r.end_at(LEN, &mut Wild, 10), RingEnd::Failed);
    }

    /// C56: a ring started again over a stale `DSADR` inside it reads a
    /// position until the first piece loads; `first_loaded` says when.
    #[test]
    fn the_first_load_is_seen_over_a_stale_position_inside_the_ring() {
        let (mut m, ring) = Model::new(7, 0);
        // A previous run of this ring stopped 5 pieces and 64 bytes in.
        m.dsadr = BUF + 5 * ring.piece() + 64;
        let before = Seen {
            stopped: false,
            ddadr: m.ddadr,
            dsadr: m.dsadr,
            dtadr: DEVICE,
        };
        assert!(!ring.first_loaded(before));
        assert_eq!(
            ring.offset(before.dsadr),
            Some(5 * ring.piece() + 64),
            "the hazard"
        );
        assert!(m.step(), "the load");
        let after = Seen {
            stopped: false,
            ddadr: m.ddadr,
            dsadr: m.dsadr,
            dtadr: DEVICE,
        };
        assert!(ring.first_loaded(after));
        assert_eq!(ring.offset(after.dsadr), Some(0));
        let stopped = Seen {
            stopped: true,
            ..before
        };
        assert!(ring.first_loaded(stopped));
    }
}
