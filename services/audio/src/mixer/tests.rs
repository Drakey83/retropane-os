//! The mixer against a model of the DMA engine: it plays the ring a frame
//! at a time at 44.1 kHz and sees only what the mixer handed it
//! (`filled`, the cache written back), so a missed hand-over, a late wake
//! or a stale lap is heard in the recording. Like the HAL's ring
//! (`rp_hal_pxa27x::dmaring`), it cuts the ring into pieces and stops at
//! the end of the piece the mixer's end is in.

use super::*;
use crate::resample::Ratio;
use rp_hal_api::RingEnd;

/// The DMA engine: its own copy of the ring (what the cache wrote back).
struct FakeDma {
    mem: Vec<u8>,
    base: usize,
    started: bool,
    /// Moving (not stopped at an end).
    moving: bool,
    /// Frames the engine has played (its clock).
    played: u64,
    /// Output frames since the start, played or silent while stopped.
    time: u64,
    /// Frames per piece.
    piece: u64,
    /// The piece whose link carries STOP.
    end: Option<u64>,
    /// The piece being played was loaded with STOP.
    stop_loaded: bool,
    /// A slow CPU: every hand-over of mixed sound plays this many frames
    /// first (the time mixing took), recorded in `during`, as are the
    /// frames played while the race is waited for.
    lag: u64,
    during: Vec<(i16, i16)>,
    /// Frames the engine has played by the first look at its position.
    head_start: u64,
    /// What went out, in real time (silence while stopped).
    out: Vec<(i16, i16)>,
}

impl FakeDma {
    fn new() -> Self {
        Self {
            mem: Vec::new(),
            base: 0,
            started: false,
            moving: false,
            played: 0,
            time: 0,
            piece: RING_FRAMES / 32,
            end: None,
            stop_loaded: false,
            lag: 0,
            during: Vec::new(),
            head_start: 0,
            out: Vec::new(),
        }
    }

    fn pieces(&self) -> u64 {
        RING_FRAMES / self.piece
    }

    /// One frame of the engine (it is moving).
    fn play(&mut self) -> (i16, i16) {
        if self.played.is_multiple_of(self.piece) {
            // A piece's descriptor loaded: its STOP as it is now.
            self.stop_loaded = self.end == Some(self.played / self.piece % self.pieces());
        }
        let at = (self.played % RING_FRAMES) as usize * 4;
        let b = &self.mem[at..at + 4];
        let f = (
            i16::from_le_bytes([b[0], b[1]]),
            i16::from_le_bytes([b[2], b[3]]),
        );
        self.played += 1;
        if self.played.is_multiple_of(self.piece) && self.stop_loaded {
            self.moving = false;
        }
        f
    }

    /// One output frame of time: the engine's frame if it moves.
    fn step(&mut self) -> Option<(i16, i16)> {
        self.time += 1;
        let f = self.moving.then(|| self.play());
        self.out.push(f.unwrap_or((0, 0)));
        f
    }

    fn pass(&mut self, frames: u64) {
        for _ in 0..frames {
            if let Some(f) = self.step() {
                self.during.push(f);
            }
        }
    }
}

impl DmaRing for FakeDma {
    fn start(&mut self, port: rp_hal_api::Port, ring: &[u8]) -> bool {
        assert_eq!(port, TX_PORT);
        self.mem = ring.to_vec();
        self.base = ring.as_ptr() as usize;
        self.started = true;
        self.moving = true;
        self.end = None;
        self.played = self.head_start;
        self.time = self.time.max(self.head_start);
        true
    }
    fn position(&mut self) -> Option<usize> {
        self.started
            .then(|| (self.played % RING_FRAMES) as usize * 4)
    }
    fn filled(&mut self, part: &[u8]) {
        if self.started {
            self.pass(self.lag);
            let at = part.as_ptr() as usize - self.base;
            self.mem[at..at + part.len()].copy_from_slice(part);
        }
    }
    fn stop(&mut self) {
        self.started = false;
        self.moving = false;
    }
    fn piece(&self) -> usize {
        if self.started {
            self.piece as usize * 4
        } else {
            0
        }
    }
    fn stopped(&mut self) -> bool {
        self.started && !self.moving
    }
    fn end_at(&mut self, at: usize) -> RingEnd {
        let n = self.pieces();
        let k = at as u64 / 4 / self.piece;
        self.end = Some(k);
        if !self.moving {
            if self.played / self.piece % n == (k + 1) % n {
                return RingEnd::Stopped;
            }
            self.moving = true;
            return RingEnd::Restarted;
        }
        let inside = !self.played.is_multiple_of(self.piece);
        let c = self.played / self.piece % n;
        if inside && self.stop_loaded && c != k {
            // The race: it stops at the old end's piece; wait, restart.
            while self.moving {
                self.pass(1);
            }
            self.moving = true;
            return RingEnd::Raced;
        }
        if inside && !self.stop_loaded && c == k {
            self.end = Some((k + 1) % n);
            return RingEnd::Failed;
        }
        RingEnd::Running
    }
}

fn leak<T: Clone + Default>(n: usize) -> &'static mut [T] {
    Box::leak(vec![T::default(); n].into_boxed_slice())
}

/// A deterministic test signal: sample `n` of channel `c`.
fn noise(seed: u32, n: u64, c: usize, amp: i32) -> i16 {
    let mut x = seed ^ (n as u32).wrapping_mul(0x9E37_79B9) ^ ((c as u32) << 29);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    ((x % (2 * amp as u32 + 1)) as i32 - amp) as i16
}

/// A program feeding a stream: `refill` frames whenever there's room,
/// until `total`.
struct Feeder {
    id: StreamId,
    ch: usize,
    seed: u32,
    amp: i32,
    written: u64,
    total: u64,
    refill: u64,
    /// Stops writing (a stall) between these ms.
    stall: Option<(u32, u32)>,
    /// Every sample this value instead of the noise.
    constant: Option<i16>,
}

impl Feeder {
    fn feed(&mut self, m: &mut Mixer<'_>, now: u32) -> bool {
        if self.stall.is_some_and(|(a, b)| (a..b).contains(&now)) {
            return false;
        }
        let mut wrote = false;
        while self.written < self.total {
            let room = m.status(self.id, now).unwrap().room;
            let n = self.refill.min(self.total - self.written);
            if room < n {
                break;
            }
            let (written, ch) = (self.written, self.ch);
            let samples: Vec<i16> = (0..n as usize * ch)
                .map(|k| self.sample(written + (k / ch) as u64, k % ch))
                .collect();
            let (a, b) = m.writable(self.id).unwrap();
            let bytes = a
                .as_chunks_mut::<2>()
                .0
                .iter_mut()
                .chain(b.as_chunks_mut::<2>().0.iter_mut());
            for (d, v) in bytes.zip(&samples) {
                *d = v.to_le_bytes();
            }
            m.commit(self.id, n, now).unwrap();
            self.written += n;
            wrote = true;
        }
        if self.written == self.total {
            m.end(self.id);
        }
        wrote
    }

    fn sample(&self, n: u64, c: usize) -> i16 {
        self.constant
            .unwrap_or_else(|| noise(self.seed, n, c.min(self.ch - 1), self.amp))
    }
}

struct Sim {
    m: Mixer<'static>,
    dma: FakeDma,
    ms: u32,
    heard: Vec<(i16, i16)>,
    next_wake: u32,
    /// The ms of every wake, and its interval.
    wakes: Vec<(u32, u32, bool)>,
}

impl Sim {
    fn new(speaker: bool) -> Self {
        Self::with(speaker, FakeDma::new())
    }

    fn with(speaker: bool, dma: FakeDma) -> Self {
        let mut s = Self {
            m: Mixer::new(leak(MIXER_BYTES), speaker),
            dma,
            ms: 0,
            heard: Vec::new(),
            next_wake: 0,
            wakes: Vec::new(),
        };
        let t = s.m.start(&mut s.dma, 0).unwrap();
        s.next_wake = t.next_ms;
        s
    }

    fn open(&mut self, cfg: StreamConfig) -> StreamId {
        let n = min_ring_bytes(&cfg).unwrap();
        self.m.open(cfg, leak(n), self.ms).unwrap()
    }

    fn tick(&mut self) -> Tick {
        let t = self.m.tick(&mut self.dma, self.ms);
        // Time the wake took (a slow CPU): heard, and the clock moved on.
        let during = core::mem::take(&mut self.dma.during);
        self.heard.extend(during);
        while frames(self.ms) < self.dma.time {
            self.ms += 1;
        }
        self.next_wake = self.ms + t.next_ms;
        self.wakes.push((self.ms, t.next_ms, t.interactive));
        t
    }

    /// Runs `ms` milliseconds: each one the engine plays its frames, then
    /// `act` (the programs) runs; a wake when it did something or when due.
    fn run(&mut self, ms: u32, act: &mut dyn FnMut(&mut Mixer<'static>, u32) -> bool) {
        for _ in 0..ms {
            self.ms += 1;
            self.pass_to(frames(self.ms));
            let acted = act(&mut self.m, self.ms);
            if acted || self.ms >= self.next_wake {
                self.tick();
            }
        }
    }

    /// The engine plays on to output frame `to`.
    fn pass_to(&mut self, to: u64) {
        while self.dma.time < to {
            if let Some(f) = self.dma.step() {
                self.heard.push(f);
            }
        }
    }

    /// Frames heard in `[from, to)` (the engine's frames: the output's
    /// clock, which stands still while it is stopped).
    fn heard(&self, from: u64, to: u64) -> &[(i16, i16)] {
        &self.heard[from as usize..to as usize]
    }
}

fn music(sim: &mut Sim, seed: u32, amp: i32, secs: u64) -> Feeder {
    let id = sim.open(StreamConfig::new(Role::Media, OUT, 2, 1_411));
    Feeder {
        id,
        ch: 2,
        seed,
        amp,
        written: 0,
        total: secs * u64::from(OUT),
        refill: 1_411,
        stall: None,
        constant: None,
    }
}

/// A mono stream of one value (`legacy`: an old program's), fed for `secs`.
fn steady(sim: &mut Sim, role: Role, legacy: bool, value: i16, secs: u64) -> Feeder {
    let mut cfg = StreamConfig::new(role, OUT, 1, 1_411);
    cfg.legacy = legacy;
    let id = sim.open(cfg);
    Feeder {
        id,
        ch: 1,
        seed: 0,
        amp: 0,
        written: 0,
        total: secs * u64::from(OUT),
        refill: 1_411,
        stall: None,
        constant: Some(value),
    }
}

#[test]
fn a_lone_44k_stream_at_full_volume_is_bit_exact() {
    let mut sim = Sim::new(false);
    let mut f = music(&mut sim, 1, 30_000, 2);
    sim.run(2_500, &mut |m, now| f.feed(m, now));
    let st = sim.m.status(f.id, sim.ms).unwrap();
    let start = st.actual_start.unwrap();
    assert!(st.drained);
    for (n, &(l, r)) in sim.heard(start, start + f.total).iter().enumerate() {
        assert_eq!(
            (l, r),
            (f.sample(n as u64, 0), f.sample(n as u64, 1)),
            "frame {n}"
        );
    }
    // Silence before and after: nothing stale from an earlier lap.
    assert!(sim.heard(0, start).iter().all(|&s| s == (0, 0)));
    assert!(
        sim.heard(start + f.total, sim.heard.len() as u64)
            .iter()
            .all(|&s| s == (0, 0))
    );
    assert_eq!(sim.m.stats().late, 0);
    assert_eq!(sim.m.stats().starved, 0);
}

#[test]
fn music_alone_wakes_every_200_ms_and_mixes_a_ring_ahead() {
    let mut sim = Sim::new(false);
    let mut f = music(&mut sim, 2, 8_000, 4);
    sim.run(3_000, &mut |m, now| f.feed(m, now));
    // In the steady state (after the first second): background, and two
    // wakes per 200 ms: the timer's (which frees the program's room) and
    // the program's write (which lets the mix run a ring ahead).
    let steady: Vec<_> = sim
        .wakes
        .iter()
        .filter(|w| w.0 > 1_000 && w.0 < 3_000)
        .collect();
    assert!(steady.iter().all(|w| !w.2), "interactive with music only");
    let per_s = steady.len() as f64 / 2.0;
    assert!(per_s <= 11.0, "{per_s} wakes a second: {steady:?}");
    let timer: Vec<u32> = steady
        .iter()
        .filter(|w| w.1 >= BACKGROUND_MS)
        .map(|w| w.0)
        .collect();
    for g in timer.windows(2) {
        assert!(g[1] - g[0] >= BACKGROUND_MS - 5, "{timer:?}");
    }
    // Right after the program's write the mix is most of a ring ahead.
    sim.tick();
    f.feed(&mut sim.m, sim.ms);
    sim.tick();
    assert!(sim.m.lead() > frames(200), "lead {}", sim.m.lead());
}

#[test]
fn a_click_starts_within_the_margin_while_music_is_mixed_far_ahead() {
    let mut sim = Sim::new(false);
    let mut f = music(&mut sim, 3, 8_000, 3);
    sim.run(1_000, &mut |m, now| f.feed(m, now));
    // A wake, the program's refill, and the mix ahead it allows.
    sim.tick();
    f.feed(&mut sim.m, sim.ms);
    sim.tick();
    assert!(sim.m.lead() > frames(200), "music mixed far ahead first");
    let click = sim.open(StreamConfig::new(Role::System, OUT, 1, 441));
    let samples: Vec<i16> = (0..2_205).map(|n| noise(77, n, 0, 6_000)).collect();
    sim.m.write(click, &samples, sim.ms).unwrap();
    sim.m.end(click);
    let t = sim.tick();
    let st = sim.m.status(click, sim.ms).unwrap();
    let (start, placed) = (st.actual_start.unwrap(), st.placed_at.unwrap());
    assert!(
        start - placed <= MARGIN + SNAP,
        "started {} frames late",
        start - placed
    );
    // Never inside the margin: the engine may already have read that far.
    assert!(
        start - placed >= MARGIN,
        "started {} frames after the DMA",
        start - placed
    );
    assert!(
        t.rewrote > frames(150),
        "the music ahead was re-mixed: {t:?}, lead {}",
        sim.m.lead()
    );
    assert!(t.interactive);
    sim.run(1_000, &mut |m, now| f.feed(m, now));
    // Heard: music plus the click, exactly, from the click's start on.
    let m_start = sim.m.status(f.id, sim.ms).unwrap().actual_start.unwrap();
    for n in 0..4_000u64 {
        let t = start - 500 + n;
        let mf = t - m_start;
        let c = if t >= start && t - start < 2_205 {
            i32::from(samples[(t - start) as usize])
        } else {
            0
        };
        let want_l = (i32::from(f.sample(mf, 0)) + c) as i16;
        let want_r = (i32::from(f.sample(mf, 1)) + c) as i16;
        assert_eq!(sim.heard[t as usize], (want_l, want_r), "frame {t}");
    }
    assert_eq!(sim.m.stats().late, 0);
}

/// One scenario: loud music (the limiter works), a 22.05 kHz click
/// committed at `click_ms` (or placed at `at` from the start); what's heard.
fn rewrite_scenario(click_ms: Option<u32>, at: Option<u64>) -> (Vec<(i16, i16)>, u64) {
    let mut sim = Sim::new(true);
    let mut f = music(&mut sim, 4, 26_000, 2);
    let mut cfg = StreamConfig::new(Role::System, 22_050, 1, 441);
    let samples: Vec<i16> = (0..4_410).map(|n| noise(9, n, 0, 20_000)).collect();
    let mut click = None;
    if let Some(at) = at {
        cfg.start = Start::At(at);
        let id = sim.open(cfg);
        sim.m.write(id, &samples, 0).unwrap();
        sim.m.end(id);
        click = Some(id);
    }
    sim.run(2_400, &mut |m, now| {
        let mut acted = f.feed(m, now);
        if Some(now) == click_ms {
            let n = min_ring_bytes(&cfg).unwrap();
            let id = m.open(cfg, leak(n), now).unwrap();
            m.write(id, &samples, now).unwrap();
            m.end(id);
            click = Some(id);
            acted = true;
        }
        acted
    });
    let start = sim
        .m
        .status(click.unwrap(), sim.ms)
        .unwrap()
        .actual_start
        .unwrap();
    assert!(sim.m.stats().limited > 0, "the limiter had work");
    (sim.heard, start)
}

#[test]
fn the_block_path_gives_exactly_what_the_frame_path_gives() {
    // Every rate the mixer takes, mono and stereo, at full and other
    // volumes, through a ramp, from before the stream's start to past its
    // end (a ring large enough to keep all of it): wherever the block path
    // applies, its sums equal the frame-by-frame reference's.
    let mut used = 0;
    for &rate in &[
        8_000, 11_025, 16_000, 22_050, 32_000, 44_100, 48_000, 88_200, 96_000,
    ] {
        for ch in [1u8, 2] {
            for volume in [UNITY, UNITY / 3, 2 * UNITY] {
                let mut sim = Sim::new(false);
                let mut cfg = StreamConfig::new(Role::Media, rate, ch, rate / 20);
                cfg.volume = volume;
                let frames_in = rate as usize / 2;
                let n = frames_in * usize::from(ch);
                let big = min_ring_bytes(&cfg).unwrap().max(2 * n);
                let id = sim.m.open(cfg, leak(big), 0).unwrap();
                let x: Vec<i16> = (0..n).map(|k| noise(rate, k as u64, 0, 20_000)).collect();
                assert_eq!(sim.m.write(id, &x, 0).unwrap() as usize, frames_in);
                sim.tick();
                sim.run(100, &mut |_, _| false);
                sim.m.set_volume(id, UNITY / 2);
                sim.tick();
                let start = sim.m.status(id, 0).unwrap().actual_start.unwrap();
                let s = sim.m.slots.iter_mut().flatten().next().unwrap();
                assert!(s.ramp.2 > start, "the ramp is inside the stream");
                for b in (start.saturating_sub(64)..start + frames(600)).step_by(BLOCK / 2) {
                    let mut fast = [0i32; 2 * BLOCK];
                    if s.add_fast(b, &mut fast) {
                        used += 1;
                        let mut exact = [0i32; 2 * BLOCK];
                        let starved = s.starved_at;
                        let r = s.add_exact(b, &mut exact);
                        s.starved_at = starved;
                        assert!(
                            !r,
                            "{rate}/{ch}: the reference ran dry where the block path applied"
                        );
                        assert_eq!(
                            fast, exact,
                            "{rate} Hz, {ch} ch, volume {volume}, block {b}"
                        );
                    }
                }
            }
        }
    }
    assert!(used > 5_000, "the block path applied only {used} times");
}

#[test]
fn a_rewrite_sounds_exactly_as_if_the_sound_had_been_known_all_along() {
    // The mix is a function of output time: re-mixing from a rewrite point
    // (resampler, volume ramps, limiter, speaker filter and dither all
    // included) gives what mixing it once would have.
    let (late, start) = rewrite_scenario(Some(700), None);
    let (known, start2) = rewrite_scenario(None, Some(start));
    assert_eq!(start, start2);
    assert_eq!(late.len(), known.len());
    let first = late.iter().zip(&known).position(|(a, b)| a != b);
    assert_eq!(
        first, None,
        "differs from frame {first:?} (click at {start})"
    );
}

#[test]
fn an_interactive_stream_shortens_the_interval_and_it_returns_after_the_hold() {
    let mut sim = Sim::new(false);
    let mut f = music(&mut sim, 5, 8_000, 5);
    let mut click = None;
    sim.run(4_000, &mut |m, now| {
        let mut acted = f.feed(m, now);
        if now == 1_000 {
            let cfg = StreamConfig::new(Role::System, OUT, 1, 441);
            let id = m
                .open(cfg, leak(min_ring_bytes(&cfg).unwrap()), now)
                .unwrap();
            let s: Vec<i16> = (0..441).map(|n| noise(1, n, 0, 5_000)).collect();
            m.write(id, &s, now).unwrap();
            m.end(id);
            click = Some(id);
            acted = true;
        }
        acted
    });
    let short: Vec<_> = sim
        .wakes
        .iter()
        .filter(|w| (1_050..1_900).contains(&w.0))
        .collect();
    assert!(
        short.iter().all(|w| w.2 && w.1 <= INTERACTIVE_MS),
        "{short:?}"
    );
    let back: Vec<_> = sim
        .wakes
        .iter()
        .filter(|w| (2_400..4_000).contains(&w.0))
        .collect();
    assert!(back.iter().all(|w| !w.2), "{back:?}");
    // Back to long sleeps, and nothing missed through both switches.
    assert!(back.iter().any(|w| w.1 >= BACKGROUND_MS - 10), "{back:?}");
    assert_eq!(sim.m.stats().late, 0);
    assert_eq!(sim.m.stats().starved, 0);
    assert!(click.is_some());
}

#[test]
fn the_short_interval_holds_after_a_click_stream_has_closed() {
    // The shell's clicks are short streams of their own: once one has
    // played and closed, the interval stays short for the hold.
    let mut sim = Sim::new(false);
    let mut f = music(&mut sim, 12, 8_000, 3);
    sim.run(800, &mut |m, now| f.feed(m, now));
    let cfg = StreamConfig::new(Role::System, OUT, 1, 441);
    let id = sim.open(cfg);
    sim.m.write(id, &[3_000; 441], sim.ms).unwrap();
    sim.m.end(id);
    sim.tick();
    sim.run(100, &mut |m, now| f.feed(m, now));
    assert!(sim.m.status(id, sim.ms).unwrap().drained);
    assert!(sim.m.close(id).is_some());
    let closed = sim.ms;
    sim.run(1_500, &mut |m, now| f.feed(m, now));
    let held: Vec<_> = sim
        .wakes
        .iter()
        .filter(|w| (closed + 20..closed + HOLD_MS - 150).contains(&w.0))
        .collect();
    assert!(
        !held.is_empty() && held.iter().all(|w| w.2 && w.1 <= INTERACTIVE_MS),
        "{held:?}"
    );
    let after: Vec<_> = sim
        .wakes
        .iter()
        .filter(|w| w.0 > closed + HOLD_MS + 50)
        .collect();
    assert!(after.iter().all(|w| !w.2), "{after:?}");
}

#[test]
fn small_writes_close_together_mark_an_old_program_interactive() {
    let mut sim = Sim::new(false);
    let mut cfg = StreamConfig::new(Role::Media, 22_050, 1, 441);
    cfg.legacy = true;
    assert!(!cfg.low_latency);
    let id = sim.open(cfg);
    // A waveOut game: 20 ms buffers every 20 ms.
    let buf: Vec<i16> = (0..441).map(|n| noise(3, n, 0, 3_000)).collect();
    let mut wrote = 0;
    sim.run(300, &mut |m, now| {
        if now % 20 == 0 {
            m.write(id, &buf, now).unwrap();
            wrote += 1;
            return true;
        }
        false
    });
    assert!(wrote >= SMALL_RUN);
    assert!(sim.m.status(id, sim.ms).unwrap().interactive);
    // Then a large write (a music player's): background again.
    sim.run(200, &mut |_, _| false);
    sim.m.write(id, &[0; 4_410], sim.ms).unwrap();
    assert!(!sim.m.status(id, sim.ms).unwrap().interactive);
}

#[test]
fn large_writes_are_not_interactive() {
    let mut sim = Sim::new(false);
    let mut cfg = StreamConfig::new(Role::Media, 22_050, 1, 4_410);
    cfg.legacy = true;
    let id = sim.open(cfg);
    let buf: Vec<i16> = vec![100; 4_410];
    let mut n = 0;
    sim.run(1_000, &mut |m, now| {
        if now % 150 == 0 && n < 6 {
            n += 1;
            m.write(id, &buf, now).unwrap();
            return true;
        }
        false
    });
    assert!(!sim.m.status(id, sim.ms).unwrap().interactive);
}

#[test]
fn rings_are_sized_for_the_longest_lead_and_a_smaller_one_is_refused() {
    let cfg = StreamConfig::new(Role::Media, OUT, 2, 1_411);
    let need = min_ring_bytes(&cfg).unwrap();
    // ADR-0032 R2: about 50 KiB at 44.1 kHz stereo.
    let kib = need / 1024;
    assert!((48..=54).contains(&kib), "{kib} KiB");
    let mut sim = Sim::new(false);
    let got = need - 2;
    assert_eq!(
        sim.m.open(cfg, leak(got), 0),
        Err(OpenError::RingTooSmall { need, got })
    );
    // A mono 22.05 kHz stream needs about a quarter.
    let small = min_ring_bytes(&StreamConfig::new(Role::Media, 22_050, 1, 441)).unwrap();
    assert!(small * 4 < need + 4 * 3_000, "{small} vs {need}");
    assert_eq!(
        min_ring_bytes(&StreamConfig::new(Role::Media, 3_000, 1, 1)),
        Err(OpenError::BadRate(3_000))
    );
    assert_eq!(
        min_ring_bytes(&StreamConfig::new(Role::Media, OUT, 3, 1)),
        Err(OpenError::BadChannels(3))
    );
}

#[test]
fn a_ring_of_exactly_the_rule_never_runs_dry_across_interval_switches() {
    // The program writes only when told there's room, in its declared size;
    // clicks come and go, so the mixer switches between 15 and 200 ms.
    let mut sim = Sim::new(false);
    let mut f = music(&mut sim, 6, 8_000, 6);
    sim.run(5_500, &mut |m, now| {
        let mut acted = f.feed(m, now);
        if now % 1_700 == 0 {
            let cfg = StreamConfig::new(Role::System, OUT, 1, 441);
            let id = m
                .open(cfg, leak(min_ring_bytes(&cfg).unwrap()), now)
                .unwrap();
            let s: Vec<i16> = vec![500; 441];
            m.write(id, &s, now).unwrap();
            m.end(id);
            acted = true;
        }
        acted
    });
    let modes: Vec<bool> = sim.wakes.iter().map(|w| w.2).collect();
    assert!(modes.contains(&true) && modes.contains(&false));
    assert_eq!(sim.m.stats().starved, 0, "the music ran dry");
    assert_eq!(sim.m.stats().late, 0);
}

#[test]
fn a_stream_that_runs_dry_carries_on_where_it_stopped() {
    let mut sim = Sim::new(false);
    let mut f = music(&mut sim, 7, 8_000, 2);
    f.stall = Some((500, 1_500));
    sim.run(3_000, &mut |m, now| f.feed(m, now));
    assert!(sim.m.stats().starved >= 1);
    // Every frame heard, in order, with one gap of silence.
    let start = sim.m.status(f.id, sim.ms).unwrap().actual_start.unwrap();
    let heard: Vec<_> = sim.heard[start as usize..]
        .iter()
        .copied()
        .filter(|&s| s != (0, 0))
        .collect();
    let want: Vec<_> = (0..f.total)
        .map(|n| (f.sample(n, 0), f.sample(n, 1)))
        .filter(|&s| s != (0, 0))
        .collect();
    assert_eq!(heard.len(), want.len());
    assert!(heard == want, "lost or repeated frames");
    assert!(sim.m.status(f.id, sim.ms).unwrap().drained);
}

/// The engine plays on alone for `ms` (the service doesn't run).
fn stall(sim: &mut Sim, ms: u32) {
    for _ in 0..ms {
        sim.ms += 1;
        sim.pass_to(frames(sim.ms));
    }
}

#[test]
fn a_late_wake_plays_silence_never_an_old_lap_and_nothing_is_lost() {
    let mut sim = Sim::new(false);
    let mut f = music(&mut sim, 8, 8_000, 3);
    sim.run(1_000, &mut |m, now| f.feed(m, now));
    // Just after a wake that mixed a ring ahead (245 ms), the service
    // stalls for longer than that lead and the rest of its piece.
    sim.tick();
    stall(&mut sim, 260);
    // The engine stopped at the ring's end (A6): the late path carries
    // every stream on from there, and the engine is started again.
    assert!(sim.dma.stopped());
    sim.tick();
    sim.run(2_500, &mut |m, now| f.feed(m, now));
    assert_eq!(sim.m.stats().late, 1);
    assert_eq!(sim.m.stats().stops, 1);
    assert_eq!(sim.m.stats().restarts, 1);
    assert_eq!(sim.m.stats().ring_failed, 0);
    // Every non-silent frame heard once, in order: the stall is a gap.
    let start = sim.m.status(f.id, sim.ms).unwrap().actual_start.unwrap();
    let heard: Vec<_> = sim.heard[start as usize..]
        .iter()
        .copied()
        .filter(|&s| s != (0, 0))
        .collect();
    let want: Vec<_> = (0..f.total)
        .map(|n| (f.sample(n, 0), f.sample(n, 1)))
        .filter(|&s| s != (0, 0))
        .collect();
    assert!(
        heard == want,
        "heard {} frames, wanted {}",
        heard.len(),
        want.len()
    );
}

#[test]
fn a_wake_that_takes_time_to_mix_still_wakes_again_in_time() {
    // A slow CPU: each block of 32 frames takes the time of 9 to mix and
    // hand over (28%, as in the emulator), so a wake that mixes a ring
    // ahead ends ~70 ms after it began. The next wake must be timed from where the
    // DMA is after mixing, not from where it was.
    let mut sim = Sim::new(false);
    let mut f = music(&mut sim, 11, 8_000, 4);
    sim.dma.lag = 9;
    sim.run(3_500, &mut |m, now| f.feed(m, now));
    assert_eq!(sim.m.stats().late, 0);
    assert_eq!(sim.m.stats().starved, 0);
    assert_eq!(sim.m.stats().stops, 0);
    assert_eq!(sim.m.stats().ring_failed, 0);
    let start = sim.m.status(f.id, sim.ms).unwrap().actual_start.unwrap();
    let bad = (0..f.total)
        .find(|&n| sim.heard.get((start + n) as usize) != Some(&(f.sample(n, 0), f.sample(n, 1))));
    assert_eq!(bad, None, "a frame not heard as written");
}

#[test]
fn the_engine_moving_before_the_first_wake_is_not_a_late_wake() {
    let mut m = Mixer::new(leak(MIXER_BYTES), false);
    let mut dma = FakeDma {
        head_start: 100,
        ..FakeDma::new()
    };
    m.start(&mut dma, 0).unwrap();
    assert_eq!(m.stats().late, 0);
    assert!(m.lead() > MARGIN);
}

/// Every non-silent frame heard from `start` on, against every non-silent
/// frame `f` wrote: equal means nothing lost, nothing repeated.
fn heard_once(sim: &Sim, f: &Feeder) -> bool {
    let start = sim.m.status(f.id, sim.ms).unwrap().actual_start.unwrap();
    let heard: Vec<_> = sim.heard[start as usize..]
        .iter()
        .copied()
        .filter(|&s| s != (0, 0))
        .collect();
    let want: Vec<_> = (0..f.total)
        .map(|n| (f.sample(n, 0), f.sample(n, 1)))
        .filter(|&s| s != (0, 0))
        .collect();
    heard == want
}

#[test]
fn a_stall_longer_than_the_ring_is_silence_never_a_replay() {
    // Audit E3 (ADR-0032 A6): the looping ring replayed its last quarter
    // second until the mixer woke. Now the engine stops at the mix's end:
    // silence, then the music carries on from where it stopped.
    let mut sim = Sim::new(false);
    let mut f = music(&mut sim, 8, 8_000, 3);
    sim.run(1_000, &mut |m, now| f.feed(m, now));
    sim.tick();
    let (before, out) = (sim.m.clock(), sim.dma.out.len());
    let stop = sim.m.piece_end(sim.m.mixed_to);
    stall(&mut sim, 400);
    assert_eq!(sim.dma.played, stop, "stopped at the end's piece");
    // What went out in the stall: the mix to its end, then silence only.
    let tail: Vec<_> = sim.dma.out[out..].to_vec();
    let quiet = tail.iter().position(|&s| s == (0, 0)).unwrap();
    assert!(tail[quiet..].iter().all(|&s| s == (0, 0)), "a replay");
    assert!(quiet as u64 <= RING_FRAMES);
    sim.tick();
    // The clock counts frames that went out: it stood still while stopped.
    assert_eq!(sim.m.clock(), sim.dma.played);
    assert!(sim.m.clock() - before < frames(400));
    assert_eq!(sim.m.stats().stops, 1);
    sim.run(2_500, &mut |m, now| f.feed(m, now));
    assert!(heard_once(&sim, &f), "lost or repeated frames");
    assert!(sim.m.status(f.id, sim.ms).unwrap().drained);
    assert_eq!(sim.m.stats().ring_failed, 0);
    // The music carries on exactly where the engine stopped: no more gap
    // than the stall.
    assert!(
        sim.heard[stop as usize..stop as usize + 8]
            .iter()
            .all(|&s| s != (0, 0))
    );
}

/// The A6 rule, checked frame by frame: every frame the engine played
/// from `f`'s start is the one mixed for its time, or silence (the stream
/// has no gap of its own, so frame `n` of it is due at `start + n`).
fn each_frame_for_its_time_or_silent(sim: &Sim, f: &Feeder) -> Option<u64> {
    let start = sim.m.status(f.id, sim.ms).unwrap().actual_start.unwrap();
    (0..f.total).find(|&n| {
        let h = sim
            .heard
            .get((start + n) as usize)
            .copied()
            .unwrap_or((0, 0));
        h != (0, 0) && h != (f.sample(n, 0), f.sample(n, 1))
    })
}

#[test]
fn a_mixer_slower_than_the_sound_fails_quietly() {
    // Each 32-frame block takes the time of 40 to mix: the mixer can never
    // keep up. It must still never let the engine play a stale or
    // repeated frame: each frame heard is the one mixed for its time, or
    // silence (where the engine passed it before the mixer wrote it).
    let mut sim = Sim::new(false);
    let mut f = music(&mut sim, 15, 8_000, 2);
    // The ring starts with the music already waiting (as the service
    // starts it for a first stream), so the first wake mixes a full lead.
    f.feed(&mut sim.m, sim.ms);
    sim.m.stop(&mut sim.dma);
    sim.dma.lag = 40;
    let t = sim.m.start(&mut sim.dma, sim.ms).unwrap();
    sim.heard.extend(core::mem::take(&mut sim.dma.during));
    while frames(sim.ms) < sim.dma.time {
        sim.ms += 1;
    }
    sim.next_wake = sim.ms + t.next_ms;
    sim.run(3_000, &mut |m, now| f.feed(m, now));
    assert_eq!(sim.m.stats().ring_failed, 0);
    assert_eq!(sim.m.stats().late, 0);
    // It falls behind: the engine stops at the end (while the mixer is
    // still mixing) and is started again, again and again.
    assert!(sim.m.stats().restarts > 10, "{:?}", sim.m.stats());
    assert_eq!(each_frame_for_its_time_or_silent(&sim, &f), None);
    let start = sim.m.status(f.id, sim.ms).unwrap().actual_start.unwrap();
    let silent = (0..f.total)
        .filter(|&n| sim.heard.get((start + n) as usize) == Some(&(0, 0)))
        .count() as u64;
    assert!(silent * 2 < f.total, "most of it lost");
}

#[test]
fn a_shortening_rewrite_on_a_slow_cpu_keeps_the_end_ahead_of_the_engine() {
    // A slow CPU (each block takes the time of 30 frames to mix) and a
    // click arriving during music: the rewrite ends 35 ms ahead, and while
    // it mixes the engine moves on so far that the new end would land in
    // the piece it is playing (which it loaded without STOP, so it would
    // go round). The mixer mixes on until the end's piece is past it.
    let mut sim = Sim::new(false);
    let mut f = music(&mut sim, 16, 8_000, 3);
    sim.dma.lag = 30;
    sim.run(1_000, &mut |m, now| f.feed(m, now));
    for k in 0..8 {
        let mut click = steady(&mut sim, Role::System, false, 3_000, 1);
        click.total = 441;
        click.feed(&mut sim.m, sim.ms);
        sim.tick();
        sim.run(300 + 37 * k, &mut |m, now| {
            f.feed(m, now) | click.feed(m, now)
        });
        sim.m.close(click.id);
    }
    assert_eq!(sim.m.stats().ring_failed, 0, "{:?}", sim.m.stats());
}

#[test]
fn a_shortening_rewrite_then_a_stall_plays_no_stale_tail() {
    // Audit E4: music mixed ~245 ms ahead, then a click makes the lead
    // 35 ms: the rewrite ends short of the old mix. In a stall the engine
    // plays on to the end of the end's piece: that must be silence, not
    // the old mix (the music without the click).
    let mut sim = Sim::new(false);
    let mut f = music(&mut sim, 12, 8_000, 3);
    sim.run(1_000, &mut |m, now| f.feed(m, now));
    sim.tick();
    let long = sim.m.mixed_to;
    let mut click = steady(&mut sim, Role::System, false, 3_000, 1);
    click.total = 441;
    click.feed(&mut sim.m, sim.ms);
    sim.tick();
    let end = sim.m.mixed_to;
    assert!(
        end + sim.m.piece < long,
        "the rewrite didn't shorten the mix"
    );
    stall(&mut sim, 150);
    assert!(sim.dma.stopped());
    let piece_end = (end / sim.m.piece + 1) * sim.m.piece;
    assert_eq!(sim.dma.played, piece_end, "stopped at the end's piece");
    let tail = &sim.heard[end as usize..piece_end as usize];
    assert!(
        tail.iter().all(|&s| s == (0, 0)),
        "the old mix played past the end"
    );
    sim.run(2_500, &mut |m, now| f.feed(m, now) | click.feed(m, now));
    assert_eq!(sim.m.stats().ring_failed, 0);
}

#[test]
fn a_wake_inside_the_end_s_piece_restarts_at_once_with_no_gap() {
    // The race: the engine has started the end's piece (loaded with its
    // STOP) when the mixer wakes, under a piece of sound left. Moving the
    // end is too late for that piece; the engine is waited for and
    // started again at once: nothing lost, and here (no restart time in
    // the model) no gap at all.
    let mut sim = Sim::new(false);
    let mut f = music(&mut sim, 13, 8_000, 3);
    sim.run(1_000, &mut |m, now| f.feed(m, now));
    let mut raced = 0;
    for _ in 0..20 {
        sim.tick();
        let end = sim.m.mixed_to;
        let start = end / sim.m.piece * sim.m.piece;
        if end - start < 64 {
            sim.run(7, &mut |m, now| f.feed(m, now));
            continue;
        }
        // Into the end's piece, short of the end.
        while sim.dma.played < start + 8 {
            if let Some(fr) = sim.dma.step() {
                sim.heard.push(fr);
            }
        }
        while frames(sim.ms) < sim.dma.time {
            sim.ms += 1;
        }
        f.feed(&mut sim.m, sim.ms);
        sim.tick();
        raced = sim.m.stats().races;
        if raced > 0 {
            break;
        }
    }
    assert!(raced > 0, "no race made");
    assert_eq!(sim.m.stats().late, 0);
    assert_eq!(sim.m.stats().ring_failed, 0);
    sim.run(2_500, &mut |m, now| f.feed(m, now));
    let start = sim.m.status(f.id, sim.ms).unwrap().actual_start.unwrap();
    let bad = (0..f.total)
        .find(|&n| sim.heard.get((start + n) as usize) != Some(&(f.sample(n, 0), f.sample(n, 1))));
    assert_eq!(bad, None, "a frame lost or out of place");
}

#[test]
fn a_ring_cut_in_larger_pieces_still_keeps_the_end_s_rule() {
    // Whatever the HAL's cut (here 11 pieces of 4 KiB, the old chain), the
    // mix never ends in the piece just behind the engine: a background
    // lead of 245 ms is cut back to keep *lead < ring - one piece*.
    let mut sim = Sim::with(
        false,
        FakeDma {
            piece: RING_FRAMES / 11,
            ..FakeDma::new()
        },
    );
    let mut f = music(&mut sim, 14, 8_000, 3);
    sim.run(1_000, &mut |m, now| f.feed(m, now));
    sim.tick();
    stall(&mut sim, 400);
    sim.tick();
    sim.run(2_500, &mut |m, now| f.feed(m, now));
    assert_eq!(sim.m.stats().ring_failed, 0);
    assert!(heard_once(&sim, &f), "lost or repeated frames");
}

#[test]
fn a_requested_start_is_kept_to_the_frame_or_reported_late() {
    let mut sim = Sim::new(false);
    sim.run(100, &mut |_, _| false);
    let at = sim.m.clock() + frames(300) + 17;
    let mut cfg = StreamConfig::new(Role::Notification, OUT, 1, 441);
    cfg.start = Start::At(at);
    let id = sim.open(cfg);
    sim.m.write(id, &[9_000; 441], sim.ms).unwrap();
    sim.m.end(id);
    sim.tick();
    sim.run(600, &mut |_, _| false);
    assert_eq!(sim.m.status(id, sim.ms).unwrap().actual_start, Some(at));
    assert_eq!(sim.heard[at as usize - 1], (0, 0));
    assert_eq!(sim.heard[at as usize], (9_000, 9_000));
    // In the past: as soon as possible, and it says when.
    cfg.start = Start::At(sim.m.clock() - 1_000);
    let id = sim.open(cfg);
    sim.m.write(id, &[9_000; 441], sim.ms).unwrap();
    sim.tick();
    let st = sim.m.status(id, sim.ms).unwrap();
    assert!(st.actual_start.unwrap() >= sim.m.clock() + MARGIN);
}

#[test]
fn a_volume_change_ramps_without_steps() {
    let mut sim = Sim::new(false);
    let cfg = StreamConfig::new(Role::Media, OUT, 1, 4_410);
    let id = sim.open(cfg);
    sim.m.write(id, &vec![16_000; 22_050], 0).unwrap();
    sim.tick();
    sim.run(200, &mut |_, _| false);
    sim.m.set_volume(id, 0);
    sim.tick();
    sim.run(200, &mut |_, _| false);
    let start = sim.m.status(id, sim.ms).unwrap().actual_start.unwrap() as usize;
    let h: Vec<i32> = sim.heard[start..].iter().map(|s| i32::from(s.0)).collect();
    let max_step = h.windows(2).map(|w| (w[1] - w[0]).abs()).max().unwrap();
    assert!(max_step <= 16_000 / RAMP as i32 + 2, "a step of {max_step}");
    assert_eq!(*h.last().unwrap(), 0);
}

#[test]
fn unplugging_pauses_media_mutes_old_media_and_keeps_system_sounds() {
    let mut sim = Sim::new(false);
    let mut native = music(&mut sim, 10, 4_000, 3);
    let mut old_f = steady(&mut sim, Role::Media, true, 1_000, 3);
    let mut sys_f = steady(&mut sim, Role::System, false, 50, 3);
    let old = old_f.id;
    let native_id = native.id;
    let mut feed = |m: &mut Mixer<'static>, now: u32| {
        let a = native.feed(m, now);
        let b = old_f.feed(m, now);
        sys_f.feed(m, now) | a | b
    };
    sim.run(300, &mut feed);
    let before = sim.m.status(native_id, sim.ms).unwrap().played;
    sim.m.unplugged();
    sim.tick();
    sim.run(300, &mut feed);
    let n = sim.m.status(native_id, sim.ms).unwrap();
    assert!(n.paused_by_unplug);
    assert!(
        n.played - before < u64::from(frames(20) as u32),
        "kept playing"
    );
    let o = sim.m.status(old, sim.ms).unwrap();
    assert!(o.muted_by_unplug);
    // Muted: only the system sound (50) is heard now.
    assert_eq!(*sim.heard.last().unwrap(), (50, 50));
    sim.m.replugged();
    sim.tick();
    sim.run(100, &mut feed);
    assert_eq!(*sim.heard.last().unwrap(), (1_050, 1_050));
    // The paused one waits for its program.
    let held = sim.m.status(native_id, sim.ms).unwrap().played;
    sim.m.resume(native_id);
    sim.tick();
    sim.run(100, &mut feed);
    let n = sim.m.status(native_id, sim.ms).unwrap();
    assert!(n.played > held && !n.paused_by_unplug);
}

#[test]
fn a_program_can_only_spoil_its_own_stream() {
    let mut sim = Sim::new(false);
    let cfg = StreamConfig::new(Role::Media, OUT, 2, 1_411);
    let id = sim.open(cfg);
    let room = sim.m.status(id, 0).unwrap().room;
    assert_eq!(sim.m.commit(id, room + 1, 0), Err(WriteError::TooMuch));
    sim.m.commit(id, room, 0).unwrap();
    assert_eq!(sim.m.commit(id, 1, 0), Err(WriteError::TooMuch));
    sim.m.end(id);
    assert_eq!(sim.m.commit(id, 0, 0), Err(WriteError::Ended));
    assert!(sim.m.close(id).is_some());
    // A stale handle reaches nothing, not the slot's next owner.
    let other = sim.open(cfg);
    assert_eq!(sim.m.commit(id, 1, 0), Err(WriteError::NoStream));
    assert!(sim.m.status(id, 0).is_none());
    assert!(sim.m.status(other, 0).is_some());
    for _ in 1..MAX_STREAMS {
        sim.open(StreamConfig::new(Role::System, 8_000, 1, 1));
    }
    let n = min_ring_bytes(&cfg).unwrap();
    assert_eq!(sim.m.open(cfg, leak(n), 0), Err(OpenError::Full));
}

#[test]
fn resampled_streams_are_the_resampler_s_output() {
    let mut sim = Sim::new(false);
    let cfg = StreamConfig::new(Role::Media, 22_050, 1, 2_205);
    let id = sim.open(cfg);
    let x: Vec<i16> = (0..22_050).map(|n| noise(5, n, 0, 9_000)).collect();
    let mut i = 0;
    sim.run(1_500, &mut |m, now| {
        if i < x.len() {
            let n = m.write(id, &x[i..], now).unwrap() as usize;
            i += n;
            return n > 0;
        }
        m.end(id);
        false
    });
    let r = Ratio::new(22_050).unwrap();
    let start = sim.m.status(id, sim.ms).unwrap().actual_start.unwrap();
    for n in (0..40_000u64).step_by(97) {
        let (j, ph) = r.locate(n);
        let mut at = |o: i32| {
            let k = j as i64 + i64::from(o);
            if (0..x.len() as i64).contains(&k) {
                i32::from(x[k as usize])
            } else {
                0
            }
        };
        let q8 = r.sample(ph, &mut at);
        let got = i32::from(sim.heard[(start + n) as usize].0);
        // Dithered to 16 bits: within one step of the exact value.
        assert!(
            (got * 256 - q8).abs() <= 256 + 128,
            "frame {n}: {got} vs {}",
            f64::from(q8) / 256.0
        );
    }
}
