//! A game's sound: one stream of the audio service's mixer (ADR-0032),
//! behind the port layer's C calls (`rp_snd_open`, `rp_snd_room`,
//! `rp_snd_write`, `rp_snd_played`, `rp_snd_close`).
//!
//! Iterate, don't imitate (rule 0). On Windows Mobile a game queues
//! `waveOut` buffers and is called back as each is done; on Linux and the
//! consoles SDL pulls samples from the game on a thread of its own, and the
//! engine guesses how far the sound has played from the size of the last
//! pull; the DOS port programs the sound card's DMA itself. Here the game
//! writes its mixed 16-bit samples straight into the ring the service's
//! mixer reads (ADR-0032 R3: no copy, no callback, no audio thread to lock
//! against), at its own rate (the mixer converts: R4), as a Media stream
//! (the role ADR-0032 gives a game's sound: paused when the headphones come
//! out, A2), and reads back how many of its frames have really left the
//! DMA engine (R3's played position): the engine's own count, not a guess.
//!
//! Who wakes the mixer: the game itself, from every port-layer call (the
//! clock read each frame, each sound call). A C game here is one thread
//! and the audio host has no thread of its own yet (ADR-0032 R1, waiting
//! on how a big program is run: half-life-readiness.md line 6), so the
//! stream is opened as keeping its own pace: the mix runs to the end of
//! the game's data and a slow frame is covered by what the game queued,
//! not by the mixer's 15 ms interval. A stall longer than that (a map
//! loading) is silence, never a stutter: the engine stops itself at the
//! mix's end (ADR-0032 A6).

use rp_audio::mixer::{MIXER_BYTES, Mixer, StreamConfig, StreamId, min_ring_bytes};
use rp_audio::role::Role;
use rp_hal_api::DmaRing;
use rp_hal_cabi::Errno;

/// What [`crate::sys::System`] asks of a game's sound, whatever DMA engine
/// is under it. Times are milliseconds on the system's clock.
pub trait Voice {
    /// Opens the stream: `rate` in Hz (4,000 to 96,000), 1 or 2 channels.
    ///
    /// # Errors
    /// Already open (`EBUSY`), a rate or channel count the mixer does not
    /// take (`EINVAL`), no room for its ring (`ENOMEM`), or the DMA engine
    /// refused the ring (`EIO`).
    fn open(&mut self, rate: u32, channels: u8, now_ms: u32) -> Result<(), Errno>;
    /// Frames that may be written now.
    ///
    /// # Errors
    /// Not open (`EBADF`).
    fn room(&mut self, now_ms: u32) -> Result<u32, Errno>;
    /// Takes `samples` (interleaved, left first): as many whole frames as
    /// there is room for; how many frames.
    ///
    /// # Errors
    /// Not open (`EBADF`).
    fn write(&mut self, samples: &[i16], now_ms: u32) -> Result<u32, Errno>;
    /// The stream's frames that have left the DMA engine.
    ///
    /// # Errors
    /// Not open (`EBADF`).
    fn played(&mut self, now_ms: u32) -> Result<u64, Errno>;
    /// Closes the stream (what was not yet played is dropped).
    ///
    /// # Errors
    /// Not open (`EBADF`).
    fn close(&mut self, now_ms: u32) -> Result<(), Errno>;
    /// Wakes the mixer if its time has come: called from every port-layer
    /// call, cheap when it has not.
    fn pump(&mut self, now_ms: u32);
    /// The program is over: its stream closed if it left one open, the DMA
    /// engine stopped (before the codec goes down).
    fn shut(&mut self);
    /// The mixer's counts for the run's log (Half-Life run 39: the sound's
    /// write 5.1% of the play, about 350 cycles a sample played: mixed
    /// against re-mixed against played tells where).
    fn report(&self, _out: &mut dyn core::fmt::Write) {}
}

/// The longest write the stream's ring is sized for, in milliseconds of
/// sound (ADR-0032 condition 4: the DMA ring, the margin and one refill).
pub const REFILL_MS: u32 = 100;

/// The memory a [`Sound`] needs for a stream of `rate` and `channels`: the
/// mixer's own and the stream's ring. `None` for a rate or channel count
/// the mixer does not take.
#[must_use]
pub fn memory_for(rate: u32, channels: u8) -> Option<usize> {
    let cfg = config(rate, channels);
    min_ring_bytes(&cfg).ok().map(|ring| MIXER_BYTES + ring)
}

fn config(rate: u32, channels: u8) -> StreamConfig {
    let refill = (u64::from(rate) * u64::from(REFILL_MS) / 1_000) as u32;
    let mut cfg = StreamConfig::new(Role::Media, rate, channels, refill.max(1));
    cfg.own_pace = true;
    cfg
}

/// The mixer, its DMA engine and one game stream.
pub struct Sound<'a, R: DmaRing> {
    mixer: Mixer<'a>,
    dma: R,
    /// The stream's ring, while no stream is open.
    ring: Option<&'a mut [u8]>,
    open: Option<(StreamId, usize)>,
    /// When the mixer asked to be woken next.
    due_ms: u32,
}

impl<'a, R: DmaRing> Sound<'a, R> {
    /// The sound over `memory` (the mixer's [`MIXER_BYTES`] first, aligned
    /// as the DMA engine needs, the rest the stream's ring: see
    /// [`memory_for`]) and `dma`, with the codec up and I2S running;
    /// `speaker` turns the speaker's filter on. `None` if `memory` is too
    /// small for the mixer.
    pub fn new(memory: &'a mut [u8], dma: R, speaker: bool) -> Option<Self> {
        let (mixer, ring) = memory.split_at_mut_checked(MIXER_BYTES)?;
        Some(Self {
            mixer: Mixer::new(mixer, speaker),
            dma,
            ring: Some(ring),
            open: None,
            due_ms: 0,
        })
    }

    fn tick(&mut self, now_ms: u32) {
        let t = self.mixer.tick(&mut self.dma, now_ms);
        self.due_ms = now_ms.wrapping_add(t.next_ms);
    }

    fn stream(&self) -> Result<(StreamId, usize), Errno> {
        self.open.ok_or(Errno::BadHandle)
    }

    /// The mixer's counts (late wakes, dry streams, stops), for a log.
    #[must_use]
    pub fn stats(&self) -> rp_audio::mixer::Stats {
        self.mixer.stats()
    }

    /// Times the mixer's stages with a core cycle counter (R306 item 3).
    pub fn set_clock(&mut self, clock: fn() -> u32) {
        self.mixer.set_clock(clock);
    }

    /// The mixer's chain the old way (R360's A/B, `-rpsndchain0`).
    pub fn set_reference_chain(&mut self, on: bool) {
        self.mixer.set_reference_chain(on);
    }

    /// The DMA engine (tests look at what it played).
    pub fn dma(&mut self) -> &mut R {
        &mut self.dma
    }
}

impl<R: DmaRing> Voice for Sound<'_, R> {
    fn open(&mut self, rate: u32, channels: u8, now_ms: u32) -> Result<(), Errno> {
        if self.open.is_some() {
            return Err(Errno::Busy);
        }
        let cfg = config(rate, channels);
        let need = min_ring_bytes(&cfg).map_err(|_| Errno::Invalid)?;
        if self.ring.as_ref().is_none_or(|r| r.len() < need) {
            return Err(Errno::NoMemory);
        }
        if !self.mixer.is_running() {
            let t = self
                .mixer
                .start(&mut self.dma, now_ms)
                .map_err(|_| Errno::Io)?;
            self.due_ms = now_ms.wrapping_add(t.next_ms);
        }
        let ring = self.ring.take().ok_or(Errno::NoMemory)?;
        // Sized and checked above: the mixer takes it.
        let id = self
            .mixer
            .open(cfg, ring, now_ms)
            .map_err(|_| Errno::Invalid)?;
        self.open = Some((id, usize::from(channels)));
        Ok(())
    }

    fn room(&mut self, now_ms: u32) -> Result<u32, Errno> {
        let (id, _) = self.stream()?;
        self.pump(now_ms);
        let st = self.mixer.status(id, now_ms).ok_or(Errno::BadHandle)?;
        Ok(u32::try_from(st.room).unwrap_or(u32::MAX))
    }

    fn write(&mut self, samples: &[i16], now_ms: u32) -> Result<u32, Errno> {
        let (id, _) = self.stream()?;
        // What has played since the last wake is room now.
        self.tick(now_ms);
        let frames = self
            .mixer
            .write(id, samples, now_ms)
            .map_err(|_| Errno::BadHandle)?;
        if frames > 0 {
            // New sound: mixed in now, not at the next wake.
            self.tick(now_ms);
        }
        Ok(frames as u32)
    }

    fn played(&mut self, now_ms: u32) -> Result<u64, Errno> {
        let (id, _) = self.stream()?;
        self.tick(now_ms);
        self.mixer
            .status(id, now_ms)
            .map(|st| st.played)
            .ok_or(Errno::BadHandle)
    }

    fn close(&mut self, now_ms: u32) -> Result<(), Errno> {
        let (id, _) = self.stream()?;
        self.open = None;
        self.ring = self.mixer.close(id);
        self.tick(now_ms);
        Ok(())
    }

    fn pump(&mut self, now_ms: u32) {
        // Signed distance, so the clock's wrap (49 days) is no stall.
        if self.mixer.is_running() && now_ms.wrapping_sub(self.due_ms) as i32 >= 0 {
            self.tick(now_ms);
        }
    }

    fn report(&self, out: &mut dyn core::fmt::Write) {
        let s = self.mixer.stats();
        let _ = writeln!(
            out,
            "sound: the mixer woke {} times, mixed {} frames ({} of them re-mixed by {} rewrites), {} late, {} dry, {} samples clipped",
            s.wakes, s.mixed, s.rewritten, s.rewrites, s.late, s.starved, s.clipped
        );
        if s.cyc_tick > 0 {
            // Cycles a frame mixed, by stage (R306 item 3).
            let per = |c: u64| c / s.mixed.max(1);
            let _ = writeln!(
                out,
                "sound: the mixer's cycles a frame mixed: {} adding the streams, {} the chain (speaker filter, limiter, dither), {} writing the cache back; {} a frame for its wakes whole ({} cycles a wake)",
                per(s.cyc_add),
                per(s.cyc_chain),
                per(s.cyc_filled),
                per(s.cyc_tick),
                s.cyc_tick / u64::from(s.wakes.max(1))
            );
            // Kit 69: where a wake's time goes beyond the stages above.
            let _ = writeln!(
                out,
                "sound: the mixer's cycles a frame mixed, by part of the wake: {} the clock and the clearing of what played, {} the mixing whole, {} moving the ring's end, {} the rest; {} copying the programs' samples in (outside the wakes)",
                per(s.cyc_clock),
                per(s.cyc_mix),
                per(s.cyc_end),
                per(s
                    .cyc_tick
                    .saturating_sub(s.cyc_clock + s.cyc_mix + s.cyc_end)),
                per(s.cyc_write)
            );
        }
    }

    fn shut(&mut self) {
        if let Some((id, _)) = self.open.take() {
            self.ring = self.mixer.close(id);
        }
        self.mixer.stop(&mut self.dma);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use rp_audio::mixer::RING_FRAMES;
    use rp_hal_api::{Port, RingEnd};

    /// A model of the DMA engine at 44.1 kHz: its own copy of the ring
    /// (what the cache wrote back), cut into 32 pieces, stopping at the end
    /// of the piece the mixer's end is in. Time is moved by the test.
    pub(crate) struct Engine {
        mem: Vec<u8>,
        base: usize,
        started: bool,
        moving: bool,
        played: u64,
        piece: u64,
        end: Option<u64>,
        stop_loaded: bool,
        /// Refuses the ring (a DMA channel that is busy).
        pub(crate) refuse: bool,
        /// Every frame that went out, silence while stopped.
        pub(crate) out: Vec<(i16, i16)>,
    }

    impl Engine {
        pub(crate) fn new() -> Self {
            Self {
                mem: Vec::new(),
                base: 0,
                started: false,
                moving: false,
                played: 0,
                piece: RING_FRAMES / 32,
                end: None,
                stop_loaded: false,
                refuse: false,
                out: Vec::new(),
            }
        }

        fn pieces(&self) -> u64 {
            RING_FRAMES / self.piece
        }

        /// `ms` of time: the engine plays its frames, or nothing.
        pub(crate) fn pass_ms(&mut self, ms: u32) {
            for _ in 0..u64::from(ms) * 441 / 10 {
                if !self.moving {
                    self.out.push((0, 0));
                    continue;
                }
                if self.played.is_multiple_of(self.piece) {
                    self.stop_loaded = self.end == Some(self.played / self.piece % self.pieces());
                }
                let at = (self.played % RING_FRAMES) as usize * 4;
                let b = &self.mem[at..at + 4];
                self.out.push((
                    i16::from_le_bytes([b[0], b[1]]),
                    i16::from_le_bytes([b[2], b[3]]),
                ));
                self.played += 1;
                if self.played.is_multiple_of(self.piece) && self.stop_loaded {
                    self.moving = false;
                }
            }
        }
    }

    impl DmaRing for Engine {
        fn start(&mut self, _: Port, ring: &[u8]) -> bool {
            if self.refuse {
                return false;
            }
            self.mem = ring.to_vec();
            self.base = ring.as_ptr() as usize;
            self.started = true;
            self.moving = true;
            self.end = None;
            self.played = 0;
            true
        }
        fn position(&mut self) -> Option<usize> {
            self.started
                .then(|| (self.played % RING_FRAMES) as usize * 4)
        }
        fn filled(&mut self, part: &[u8]) {
            if self.started {
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
                // The race: it stops at the old end's piece; then restarts.
                while self.moving {
                    self.pass_ms(1);
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

    /// The most frames the rate converter holds back at a stream's end
    /// until the samples after them are written.
    const TAIL: u64 = 64;

    pub(crate) fn memory(rate: u32, channels: u8) -> Vec<u8> {
        vec![0u8; memory_for(rate, channels).expect("a rate the mixer takes")]
    }

    /// A game's loop: every `frame_ms` it reads the room and writes that
    /// frame's worth of a square wave (so every written sample is known),
    /// keeping `ahead_ms` queued.
    fn play(s: &mut Sound<'_, Engine>, rate: u32, frame_ms: u32, ahead_ms: u32, frames: u32) {
        let per = (rate * frame_ms / 1_000) as usize;
        let mut now = 0;
        let mut n = 0u64;
        let mut wave = |count: usize| -> Vec<i16> {
            (0..count)
                .map(|_| {
                    n += 1;
                    if (n / 32).is_multiple_of(2) {
                        8_000
                    } else {
                        -8_000
                    }
                })
                .collect()
        };
        assert_eq!(
            s.write(&wave(per * ahead_ms as usize / frame_ms as usize), now),
            { Ok((per * ahead_ms as usize / frame_ms as usize) as u32) }
        );
        for _ in 0..frames {
            s.dma().pass_ms(frame_ms);
            now += frame_ms;
            // As the game does: the clock read (a pump), then the sound.
            s.pump(now);
            let room = s.room(now).unwrap() as usize;
            assert!(room >= per, "room for a frame's sound at {now} ms: {room}");
            assert_eq!(s.write(&wave(per), now), Ok(per as u32));
        }
    }

    #[test]
    fn a_game_s_samples_come_out_of_the_engine_whole_at_30_and_at_12_frames_a_second() {
        for (frame_ms, ahead_ms) in [(33, 99), (80, 160)] {
            let mut mem = memory(44_100, 1);
            let mut s = Sound::new(&mut mem, Engine::new(), false).unwrap();
            assert_eq!(s.open(44_100, 1, 0), Ok(()));
            play(&mut s, 44_100, frame_ms, ahead_ms, 60);
            let st = s.stats();
            assert_eq!(
                (st.late, st.starved, st.stops, st.ring_failed),
                (0, 0, 0, 0),
                "{frame_ms} ms frames: {st:?}"
            );
            // 44.1 kHz at full volume is bit-exact (ADR-0032 A1): what went
            // out is the square wave, both ears the same, never a sample
            // that was not written.
            let out = &s.dma().out;
            let heard: Vec<_> = out.iter().filter(|f| f.0 != 0).collect();
            assert!(
                heard.len() as u32 > 44_100 * frame_ms * 55 / 1_000,
                "{} frames heard",
                heard.len()
            );
            assert!(heard.iter().all(|f| f.0 == f.1 && f.0.abs() == 8_000));
            // No hole inside the sound (a late wake would be zeros).
            let first = out.iter().position(|f| f.0 != 0).unwrap();
            let last = out.iter().rposition(|f| f.0 != 0).unwrap();
            assert!(out[first..=last].iter().all(|f| f.0 != 0), "a gap");
            // Half-periods of 32 frames: the pitch is the game's.
            let flips = out[first..=last]
                .windows(2)
                .filter(|w| w[0].0 != w[1].0)
                .count();
            assert!(flips.abs_diff((last - first) / 32) <= 1, "{flips} flips");
        }
    }

    #[test]
    fn the_played_count_is_the_engine_s_own_and_room_comes_back_as_it_plays() {
        let mut mem = memory(22_050, 2);
        let mut s = Sound::new(&mut mem, Engine::new(), false).unwrap();
        assert_eq!(s.open(22_050, 2, 0), Ok(()));
        let room = s.room(0).unwrap();
        assert!(room >= 22_050 / 10, "at least a refill: {room}");
        assert_eq!(s.played(0), Ok(0));
        // A fifth of a second, stereo; an odd sample at the end is not a
        // frame.
        let sound = vec![1_000i16; 2 * 4_410 + 1];
        assert_eq!(s.write(&sound, 0), Ok(4_410));
        assert_eq!(s.room(0), Ok(room - 4_410));
        let mut last = 0;
        for now in (20..=400).step_by(20) {
            s.dma().pass_ms(20);
            let p = s.played(now).unwrap();
            assert!(p >= last && p <= 4_410, "{p} after {last}");
            last = p;
        }
        // All of it but the converter's reach (the last frames wait for
        // the samples after them, which a later write or the close
        // brings), and no more.
        assert!((4_410 - TAIL..=4_410).contains(&last), "{last} played");
        // The room is back, but for what the converter still reads behind
        // the played position and ahead of it.
        let back = s.room(400).unwrap();
        assert!((room - 2 * TAIL as u32..=room).contains(&back), "{back}");
        // More than the room: what fits is taken, in whole frames.
        let left = s.room(400).unwrap();
        let big = vec![0i16; (left as usize + 100) * 2];
        assert_eq!(s.write(&big, 400), Ok(left));
        assert_eq!(s.write(&big, 400), Ok(0));
    }

    #[test]
    fn a_stall_longer_than_the_queue_is_silence_and_the_sound_carries_on() {
        let mut mem = memory(44_100, 1);
        let mut s = Sound::new(&mut mem, Engine::new(), false).unwrap();
        assert_eq!(s.open(44_100, 1, 0), Ok(()));
        let tone = vec![5_000i16; 4_410];
        assert_eq!(s.write(&tone, 0), Ok(4_410));
        // A map loads: two seconds with no call at all.
        s.dma().pass_ms(2_000);
        let first = s.played(2_000).unwrap();
        assert!((4_410 - TAIL..=4_410).contains(&first), "{first} played");
        assert_eq!(s.write(&tone, 2_000), Ok(4_410));
        s.dma().pass_ms(300);
        let both = s.played(2_300).unwrap();
        assert!((8_820 - TAIL..=8_820).contains(&both), "{both} played");
        let out = &s.dma().out;
        // Every frame is the tone or silence, and the tone's frames went
        // out once each: nothing replayed (ADR-0032 A6).
        assert!(out.iter().all(|f| f.0 == 0 || f.0 == 5_000));
        assert_eq!(out.iter().filter(|f| f.0 == 5_000).count() as u64, both);
    }

    #[test]
    fn opens_and_calls_that_cannot_be_honoured_are_refused() {
        let mut mem = memory(44_100, 2);
        let mut s = Sound::new(&mut mem, Engine::new(), true).unwrap();
        // Not open: every call says so.
        assert_eq!(s.room(0), Err(Errno::BadHandle));
        assert_eq!(s.write(&[0; 4], 0), Err(Errno::BadHandle));
        assert_eq!(s.played(0), Err(Errno::BadHandle));
        assert_eq!(s.close(0), Err(Errno::BadHandle));
        s.pump(0);
        // Rates and channels at and past the mixer's limits.
        for (rate, ch) in [(3_999, 1), (96_001, 1), (0, 2), (44_100, 0), (44_100, 3)] {
            assert_eq!(s.open(rate, ch, 0), Err(Errno::Invalid), "{rate} Hz x{ch}");
            assert!(memory_for(rate, ch).is_none());
        }
        // A ring sized for 44.1 kHz stereo has no room for 96 kHz.
        assert_eq!(s.open(96_000, 2, 0), Err(Errno::NoMemory));
        assert_eq!(s.open(4_000, 1, 0), Ok(()));
        assert_eq!(s.open(4_000, 1, 0), Err(Errno::Busy), "one stream");
        assert_eq!(s.close(5), Ok(()));
        assert_eq!(s.close(5), Err(Errno::BadHandle));
        // Closed: the ring is back, and another rate opens.
        assert_eq!(s.open(44_100, 2, 10), Ok(()));
        // Memory too small for the mixer itself.
        assert!(Sound::new(&mut [0u8; 100], Engine::new(), false).is_none());
        // A DMA engine that refuses the ring.
        let mut mem = memory(44_100, 2);
        let mut busy = Engine::new();
        busy.refuse = true;
        let mut s = Sound::new(&mut mem, busy, false).unwrap();
        assert_eq!(s.open(44_100, 2, 0), Err(Errno::Io));
        assert_eq!(s.room(0), Err(Errno::BadHandle));
    }

    #[test]
    fn a_program_that_ends_with_its_sound_open_leaves_the_engine_stopped() {
        let mut mem = memory(44_100, 1);
        let mut s = Sound::new(&mut mem, Engine::new(), false).unwrap();
        assert_eq!(s.open(44_100, 1, 0), Ok(()));
        assert_eq!(s.write(&vec![7_000i16; 8_820], 0), Ok(8_820));
        s.dma().pass_ms(50);
        s.shut();
        assert_eq!(s.dma().piece(), 0, "no ring runs");
        let heard = s.dma().out.len();
        s.dma().pass_ms(500);
        assert!(s.dma().out[heard..].iter().all(|f| *f == (0, 0)));
        // Shut twice, and with nothing open: nothing to do.
        s.shut();
        assert_eq!(s.room(600), Err(Errno::BadHandle));
        // The stream's ring came back: the sound opens again.
        assert_eq!(s.open(22_050, 1, 700), Ok(()));
    }

    #[test]
    fn closing_drops_what_was_not_played() {
        let mut mem = memory(44_100, 1);
        let mut s = Sound::new(&mut mem, Engine::new(), false).unwrap();
        assert_eq!(s.open(44_100, 1, 0), Ok(()));
        assert_eq!(s.write(&vec![7_000i16; 13_230], 0), Ok(13_230));
        s.dma().pass_ms(100);
        assert_eq!(s.close(100), Ok(()));
        s.dma().pass_ms(900);
        s.pump(1_000);
        let heard = s.dma().out.iter().filter(|f| f.0 != 0).count();
        // 100 ms and at most the mixer's margin and one piece more, of the
        // 300 ms written.
        assert!(
            (4_000..=5_500).contains(&heard),
            "{heard} frames after close"
        );
    }
}
