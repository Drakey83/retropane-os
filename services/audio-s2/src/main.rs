//! Audio service step S2 on the X51v (ADR-0032): the mixer, a guided run
//! with the owner listening. Everything goes through `rp-audio` (the
//! mixer and the S1 core) on the X51v's wiring (`rp-audio-x51v`).
//!
//! Headphones in for tests 1-5 (the quality of the sound), out for 6-7
//! (the speaker's own protection):
//!
//! 1. Two notes of the same pitch, from a 22.05 kHz and a 48 kHz source, the
//!    second placed to start exactly where the first ends (a requested
//!    start, condition 3).
//! 2. A tune, and a click for each tap on the screen: how long from the
//!    pen landing to the click's first sample, with music mixed far ahead.
//! 3. The tune alone: background media, long sleeps (wakes per second).
//! 4. The tune and clicks every half second or so: the short interval,
//!    each click's start latency, the rewrites.
//! 5. The tune from the old games' rates, 22.05 and 11.025 kHz, then 48 kHz:
//!    the resampler's CPU share for each.
//! 6. Headphones out. Two loud parts that overload together: A limited, B
//!    the same at half volume (clean, quieter): does A crackle?
//! 7. A low tune at full level on the speaker: A with the speaker filter,
//!    B without: which crackles, which sounds better?
//!
//! Each test logs its wakes per second, the mixer's CPU share, how far
//! ahead it mixed, rewrites, late wakes, streams that ran dry, the
//! limiter's work. The board bits are put back as found at the end.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed once everything was reported.
pub const MARKER: &str = "RETROPANE-AUDIO-S2-OK";

#[cfg(target_os = "none")]
mod image {
    use core::cell::RefCell;
    use core::fmt::Write;

    use rp_audio::Board;
    use rp_audio::mixer::{
        MIXER_BYTES, Mixer, Start, Stats, StreamConfig, StreamId, UNITY, min_ring_bytes,
    };
    use rp_audio::resample::OUT;
    use rp_audio::role::Role;
    use rp_audio_x51v::{BoardCpld, Service, TICK_MS, found, name, put_back};
    use rp_drv_cpld::Cpld;
    use rp_drv_mbx_display::console::Console;
    use rp_drv_mbx_display::{Display, crash};
    use rp_drv_pxa_gpio::{Gpio, GpioPin};
    use rp_drv_pxa_i2s::Tone;
    use rp_drv_pxa_ssp::Ssp;
    use rp_drv_tsc2046::Tsc2046;
    use rp_drv_wm8750::Output;
    use rp_hal_pxa27x::dma::Dma;
    use rp_hal_pxa27x::{Mmio, clocks, dma_map, rt};
    use rp_hal_x51v::MBX_VRAM_SIZE;
    use rp_hal_x51v::devices;

    rp_hal_pxa27x::entry!(main);

    /// Output level, 0-100: 60, or `RP_AUDIO_LEVEL` at build time.
    const LEVEL: u32 = match option_env!("RP_AUDIO_LEVEL") {
        Some(v) => parse(v),
        None => 60,
    };
    /// The touch chip's select and pen lines (docs/hw/touch.md).
    const TS_CS: u8 = 24;
    const TS_PENIRQ: u8 = 94;
    /// Stream rings: this many, each this big (the largest, 48 kHz stereo,
    /// needs ~56 KiB by the sizing rule).
    const POOL: usize = 8;
    const POOL_BYTES: usize = 64 * 1024;
    /// Taps in test 2.
    const TAPS: u32 = 3;
    /// `RP_AUDIO_BENCH` at build time: the mixer's pieces timed, no sound
    /// and no board bits touched.
    const BENCH: bool = option_env!("RP_AUDIO_BENCH").is_some();
    /// `RP_AUDIO_QUICK` at build time: no step waits for the owner (no
    /// taps, no headphone change): a measurement run, unattended.
    const QUICK: bool = option_env!("RP_AUDIO_QUICK").is_some();
    /// How long each "now" step waits for the owner.
    const WAIT_S: u32 = 30;

    /// The tunes (Hz), a quarter second a note.
    const TUNE: [u32; 8] = [659, 784, 880, 988, 1175, 988, 880, 784];
    const HARMONY: [u32; 8] = [784, 988, 1047, 1175, 1397, 1175, 1047, 988];
    const LOW_TUNE: [u32; 6] = [330, 392, 440, 494, 587, 659];

    const fn parse(v: &str) -> u32 {
        let b = v.as_bytes();
        let mut i = 0;
        let mut n = 0u32;
        while i < b.len() {
            n = n * 10 + (b[i] - b'0') as u32;
            i += 1;
        }
        if n > 100 { 100 } else { n }
    }

    fn ms() -> u32 {
        rt::ticks() / TICK_MS
    }

    /// A tune: notes back to back, each with a 5 ms rise and a 20 ms fall
    /// (no clicks at the joins).
    struct Synth {
        notes: &'static [u32],
        note: u32,
        rate: u32,
        amp: u16,
        ch: usize,
        n: u32,
        i: usize,
        tone: Tone,
        /// Frames still to come.
        left: u32,
    }

    impl Synth {
        fn new(
            notes: &'static [u32],
            note_ms: u32,
            rate: u32,
            amp: u16,
            ch: usize,
            total_ms: u32,
        ) -> Self {
            Self {
                notes,
                note: rate * note_ms / 1000,
                rate,
                amp,
                ch,
                n: 0,
                i: 0,
                tone: Tone::new(notes[0], rate, amp),
                left: (u64::from(rate) * u64::from(total_ms) / 1000) as u32,
            }
        }

        fn frame(&mut self) -> i16 {
            if self.n == self.note {
                self.n = 0;
                self.i = (self.i + 1) % self.notes.len();
                self.tone = Tone::new(self.notes[self.i], self.rate, self.amp);
            }
            let s = (self.tone.next().unwrap_or(0) & 0xFFFF) as u16 as i16;
            let rise = (self.rate / 200).max(1);
            let fall = (self.rate / 50).max(1);
            let env = (self.n * 32_768 / rise)
                .min(32_768)
                .min((self.note - self.n) * 32_768 / fall);
            self.n += 1;
            self.left = self.left.saturating_sub(1);
            ((i32::from(s) * env as i32) >> 15) as i16
        }
    }

    /// Everything the run drives.
    struct Rig<'a, 'g> {
        m: Mixer<'a>,
        dma: Dma,
        pool: [Option<&'a mut [u8]>; POOL],
        a: Service<'g>,
        next: u32,
        /// OS timer ticks spent in the mixer's wakes.
        busy: u64,
        /// Core cycles spent in them (the cycle counter), and the frames
        /// they mixed: the cost per frame, whatever the clock.
        cycles: u64,
        mixed: u64,
        lead_max: u64,
    }

    /// Where a test started, for its result line.
    struct Mark {
        stats: Stats,
        at: u32,
        busy: u64,
        clock: u64,
        cycles: u64,
        mixed: u64,
        screen: (u32, u32),
    }

    impl Rig<'_, '_> {
        fn tick(&mut self) -> rp_audio::mixer::Tick {
            let t0 = rt::ticks();
            let (late, lead, due) = (self.m.stats().late, self.m.lead(), self.next);
            let c0 = clocks::cycles();
            let t = self.m.tick(&mut self.dma, ms());
            self.cycles += u64::from(clocks::cycles().wrapping_sub(c0));
            self.mixed += t.mixed;
            self.busy += u64::from(rt::ticks().wrapping_sub(t0));
            if self.m.stats().late != late {
                let _ = writeln!(
                    devices::RunLog,
                    "  late wake at {} ms (due {due}, {} ms mixed ahead at the last wake)",
                    t0 / TICK_MS,
                    lead * 1_000 / u64::from(OUT)
                );
            }
            self.next = ms() + t.next_ms;
            self.lead_max = self.lead_max.max(self.m.lead());
            t
        }

        fn open(&mut self, cfg: StreamConfig) -> Option<StreamId> {
            let need = min_ring_bytes(&cfg).ok()?;
            let i = self
                .pool
                .iter()
                .position(|b| b.as_ref().is_some_and(|b| b.len() >= need))?;
            let buf = self.pool[i].take()?;
            self.m.open(cfg, buf, ms()).ok()
        }

        fn close(&mut self, id: StreamId) {
            if let Some(b) = self.m.close(id)
                && let Some(p) = self.pool.iter_mut().find(|p| p.is_none())
            {
                *p = Some(b);
            }
        }

        /// Fills the room with `s`, `refill` frames at a time (as a
        /// program does when told there's room: one wake for all of it).
        fn feed(&mut self, id: StreamId, s: &mut Synth, refill: u32) -> bool {
            let mut wrote = false;
            while self.feed_one(id, s, refill) {
                wrote = true;
            }
            wrote
        }

        fn feed_one(&mut self, id: StreamId, s: &mut Synth, refill: u32) -> bool {
            let now = ms();
            if s.left == 0 {
                self.m.end(id);
                return false;
            }
            let n = refill.min(s.left);
            let Some(st) = self.m.status(id, now) else {
                return false;
            };
            if st.room < u64::from(n) {
                return false;
            }
            {
                let Some((a, b)) = self.m.writable(id) else {
                    return false;
                };
                let mut out = a
                    .as_chunks_mut::<2>()
                    .0
                    .iter_mut()
                    .chain(b.as_chunks_mut::<2>().0.iter_mut());
                for _ in 0..n {
                    let v = s.frame().to_le_bytes();
                    for _ in 0..s.ch {
                        if let Some(d) = out.next() {
                            *d = v;
                        }
                    }
                }
            }
            self.m.commit(id, u64::from(n), now).is_ok()
        }

        /// Runs for `for_ms`: `act` each pass (the programs: true if one
        /// wrote), a wake when one wrote or when due, the jack watched.
        fn run(
            &mut self,
            out: &mut Tee<'_, '_>,
            for_ms: u32,
            act: &mut dyn FnMut(&mut Self) -> bool,
        ) {
            let end = ms() + for_ms;
            while ms() < end {
                if act(self) || ms() >= self.next {
                    self.tick();
                }
                self.jack(out);
                rt::delay_us(250);
                rt::check_in();
            }
        }

        /// A jack change while up: the route and the speaker's filter follow.
        fn jack(&mut self, out: &mut Tee<'_, '_>) -> Option<Output> {
            let o = self.a.poll_jack()?;
            self.m.set_speaker(o == Output::Speaker);
            self.tick();
            let _ = writeln!(out, "jack: rerouted to {}", name(o));
            Some(o)
        }

        fn mark(&mut self, out: &Tee<'_, '_>) -> Mark {
            Mark {
                stats: self.m.stats(),
                at: rt::ticks(),
                busy: self.busy,
                clock: self.m.clock_now(&mut self.dma),
                cycles: self.cycles,
                mixed: self.mixed,
                screen: out.screen(),
            }
        }

        fn report(&mut self, out: &mut Tee<'_, '_>, what: &str, from: &Mark) {
            // A full lead ahead before drawing (see `pause`).
            self.tick();
            let s = self.m.stats();
            let t = rt::ticks().wrapping_sub(from.at).max(1);
            let elapsed = t / TICK_MS;
            let wakes10 =
                u64::from(s.wakes - from.stats.wakes) * 10_000 / u64::from(elapsed.max(1));
            let cpu10 = (self.busy - from.busy) * 1_000 / u64::from(t);
            // The DMA's frames against the OS timer (3.25 MHz), both ends
            // read fresh: the link's real rate (A1: 44.1 kHz from SADIV 0x0D
            // is approximate).
            let now = self.m.clock_now(&mut self.dma);
            let t = rt::ticks().wrapping_sub(from.at).max(1);
            let rate = (now - from.clock) * 3_250_000 / u64::from(t);
            let per_frame = (self.cycles - from.cycles) / (self.mixed - from.mixed).max(1);
            let (writes, slowest) = out.screen();
            let _ = writeln!(
                devices::RunLog,
                "  {what}: {per_frame} cycles per frame mixed, {} screen writes (slowest {} ms)",
                writes - from.screen.0,
                slowest
            );
            let _ = writeln!(
                out,
                "{what} RESULT: {elapsed} ms, {}.{} wakes/s, mixer CPU {}.{}%, lead up to {} ms, rewrites {}, late {}, dry {}, clipped {}, limited {}, DMA at {rate} frames/s",
                wakes10 / 10,
                wakes10 % 10,
                cpu10 / 10,
                cpu10 % 10,
                self.lead_max * 1_000 / u64::from(OUT),
                s.rewrites - from.stats.rewrites,
                s.late - from.stats.late,
                s.starved - from.stats.starved,
                s.clipped - from.stats.clipped,
                s.limited - from.stats.limited,
            );
            self.lead_max = 0;
        }

        /// A 25 ms click (2 kHz), its own stream (as `PlaySound` would);
        /// returns it, and its start latency in tenths of a ms (from the
        /// write to its first sample at the DMA).
        fn click(&mut self) -> Option<(StreamId, u64, u64)> {
            let cfg = StreamConfig::new(Role::System, OUT, 1, 1_103);
            let id = self.open(cfg)?;
            let mut s = Synth::new(&[2_000], 25, OUT, 10_000, 1, 25);
            self.feed(id, &mut s, 1_103);
            self.m.end(id);
            let t = self.tick();
            let st = self.m.status(id, ms())?;
            let late = st.actual_start? - st.placed_at?;
            Some((
                id,
                late * 10_000 / u64::from(OUT),
                t.rewrote * 1_000 / u64::from(OUT),
            ))
        }

        /// Closes every drained stream in `ids`.
        fn reap(&mut self, ids: &mut [Option<StreamId>]) {
            for slot in ids.iter_mut() {
                if let Some(id) = *slot
                    && self.m.status(id, ms()).is_none_or(|s| s.drained)
                {
                    self.close(id);
                    *slot = None;
                }
            }
        }

        /// Plays the tune from one stream (`rate`, `ch`) for `for_ms`, at
        /// `volume`, with a second part if `harmony`.
        fn tune(
            &mut self,
            out: &mut Tee<'_, '_>,
            notes: &'static [u32],
            rate: u32,
            ch: usize,
            amp: u16,
            for_ms: u32,
            volume: u32,
            harmony: bool,
        ) {
            let refill = rate / 30;
            let mut cfg = StreamConfig::new(Role::Media, rate, ch as u8, refill);
            cfg.volume = volume;
            let Some(id) = self.open(cfg) else {
                let _ = writeln!(out, "  the tune's stream didn't open");
                return;
            };
            let mut s = Synth::new(notes, 250, rate, amp, ch, for_ms);
            let mut h = harmony.then(|| Synth::new(&HARMONY, 250, rate, amp, ch, for_ms));
            let hid = if harmony { self.open(cfg) } else { None };
            self.run(out, for_ms + 300, &mut |r| {
                let a = r.feed(id, &mut s, refill);
                let b = match (hid, h.as_mut()) {
                    (Some(hid), Some(h)) => r.feed(hid, h, refill),
                    _ => false,
                };
                a | b
            });
            self.close(id);
            if let Some(hid) = hid {
                self.close(hid);
            }
        }
    }

    /// A DMA engine that isn't there: its position moves 100 ms a look, so
    /// the mixer mixes a steady amount each wake; nothing is handed to
    /// hardware. For timing the mixer's own work.
    struct NoDma {
        pos: usize,
    }

    impl rp_hal_api::DmaRing for NoDma {
        fn start(&mut self, _: rp_hal_api::Port, _: &[u8]) -> bool {
            self.pos = 0;
            true
        }
        fn position(&mut self) -> Option<usize> {
            self.pos = (self.pos + 4_410 * 4) % rp_audio::mixer::DMA_RING_BYTES;
            Some(self.pos)
        }
        fn filled(&mut self, _: &[u8]) {}
        fn stop(&mut self) {}
        fn piece(&self) -> usize {
            rp_audio::mixer::DMA_RING_BYTES / 32
        }
        fn stopped(&mut self) -> bool {
            false
        }
        fn end_at(&mut self, _: usize) -> rp_hal_api::RingEnd {
            rp_hal_api::RingEnd::Running
        }
    }

    /// The link's real rate at each divider: I2S on (no codec, no board
    /// bits, no sound), the DMA looping a silent 8 KiB ring, its frames
    /// counted for 5 s against the OS timer.
    fn link_rates(out: &mut Tee<'_, '_>, ring: &mut [u8]) {
        use rp_drv_pxa_i2s::{PxaI2s, TX_PORT};
        use rp_hal_api::DmaRing;
        let (Some(i2s), Some(mut dma)) = (rt::take_i2s(), rt::take_dma(dma_map::AUDIO)) else {
            let _ = writeln!(out, "  link rates: no I2S or DMA");
            return;
        };
        let mut link = PxaI2s::new(i2s);
        let ring = &mut ring[..8192];
        ring.fill(0);
        // Around the two documented ones (0x0C 48 kHz, 0x0D 44.1 kHz), and
        // the documented 22.05 and 11.025 kHz ones.
        for sadiv in [0x0Bu32, 0x0C, 0x0D, 0x0E, 0x0F, 0x10, 0x1A, 0x34] {
            // A switch-on within ~645 ms of a switch-off is dropped.
            rt::delay_us(1_000_000);
            rt::check_in();
            if !link.start_divider(sadiv) {
                let _ = writeln!(out, "  SADIV {sadiv:#04x}: I2S didn't start");
                continue;
            }
            dma.filled(ring);
            if !dma.start(TX_PORT, ring) {
                let _ = writeln!(out, "  SADIV {sadiv:#04x}: the DMA refused the ring");
                link.stop();
                continue;
            }
            rt::delay_us(100_000);
            let frames_in_ring = (ring.len() / 4) as u64;
            let mut last = (dma.position().unwrap_or(0) / 4) as u64;
            let t0 = rt::ticks();
            let mut frames = 0u64;
            // Polled every 2 ms (the ring is 42 ms at 48 kHz).
            while rt::ticks().wrapping_sub(t0) < 5 * 3_250_000 {
                rt::delay_us(2_000);
                let pos = (dma.position().unwrap_or(0) / 4) as u64;
                frames += (pos + frames_in_ring - last) % frames_in_ring;
                last = pos;
                rt::check_in();
            }
            let t = u64::from(rt::ticks().wrapping_sub(t0));
            dma.stop();
            link.stop();
            let _ = writeln!(
                out,
                "  link SADIV {sadiv:#04x}: {} frames/s against the OS timer",
                frames * 3_250_000 / t
            );
        }
    }

    /// `RP_AUDIO_BENCH`: no sound, no board bits: each piece of the mixer
    /// timed with the cycle counter, cycles per frame (or per call).
    fn bench(out: &mut Tee<'_, '_>, scratch: &'static mut [u8]) {
        use rp_audio::chain::{BLOCK, State};
        use rp_audio::resample::{MAX_TAPS, Ratio};
        let _ = writeln!(out, "BENCH: cycles at 624 MHz, no audio hardware used");
        let frames = |c: u32, n: u32| c / n.max(1);
        // The chain on one block, three ways.
        for (what, speaker, exact) in [
            ("chain, 16-bit values, no filter", false, true),
            ("chain, dithered, no filter", false, false),
            ("chain, dithered, speaker filter", true, false),
        ] {
            let mut st = State::default();
            let mut o = [0u8; 4 * BLOCK];
            let c0 = clocks::cycles();
            for n in 0..400i32 {
                let mut mix = [0i32; 2 * BLOCK];
                for (k, s) in mix.iter_mut().enumerate() {
                    let v = (n * 977 + k as i32 * 131) % 20_000 - 10_000;
                    *s = if exact { v << 8 } else { (v << 8) + 77 };
                }
                let _ = st.run(&mut mix, speaker, &mut o);
                if n % 50 == 0 {
                    rt::check_in();
                }
            }
            let c = clocks::cycles().wrapping_sub(c0);
            let _ = writeln!(
                out,
                "  {what}: {} per frame (fill included)",
                frames(c, 400 * BLOCK as u32)
            );
        }
        // Coefficients per output frame.
        for rate in [22_050u32, 11_025, 48_000] {
            let r = Ratio::new(rate).unwrap_or_else(|_| halt());
            let (mut l, mut rr) = ([0; MAX_TAPS], [0; MAX_TAPS]);
            let mut ph = 0u32;
            let c0 = clocks::cycles();
            for k in 0..10_000 {
                r.coefs(ph, &mut l, &mut rr);
                ph = r.advance(ph).1;
                if k % 1_000 == 0 {
                    rt::check_in();
                }
            }
            let c = clocks::cycles().wrapping_sub(c0);
            let _ = writeln!(
                out,
                "  coefficients from {rate} Hz: {} per frame",
                frames(c, 10_000)
            );
        }
        // The whole mixer, one stream, no hardware: cycles per frame mixed.
        let (mem, ring) = scratch.split_at_mut(rp_audio::mixer::MIXER_BYTES);
        link_rates(out, &mut mem[..]);
        for (rate, ch, speaker) in [
            (44_100u32, 2u8, false),
            (44_100, 2, true),
            (22_050, 1, false),
            (11_025, 1, false),
            (48_000, 2, false),
        ] {
            let mut m = Mixer::new(&mut mem[..], speaker);
            let mut dma = NoDma { pos: 0 };
            let cfg = StreamConfig::new(Role::Media, rate, ch, rate / 10);
            let Ok(id) = m.open(cfg, &mut ring[..2 * 1024 * 1024], 0) else {
                let _ = writeln!(out, "  {rate} Hz: didn't open");
                continue;
            };
            // Ten seconds of a tone written once (the ring holds it all).
            let mut s = Synth::new(&TUNE, 250, rate, 9_000, usize::from(ch), 10_000);
            {
                let Some((a, b)) = m.writable(id) else {
                    continue;
                };
                let mut w = a
                    .as_chunks_mut::<2>()
                    .0
                    .iter_mut()
                    .chain(b.as_chunks_mut::<2>().0.iter_mut());
                let mut k = 0u64;
                while s.left > 0 {
                    let v = s.frame().to_le_bytes();
                    for _ in 0..ch {
                        if let Some(d) = w.next() {
                            *d = v;
                        }
                    }
                    k += 1;
                    if k % 10_000 == 0 {
                        rt::check_in();
                    }
                }
                let _ = m.commit(id, k, 0);
            }
            let _ = m.start(&mut dma, 0);
            let (c0, f0) = (clocks::cycles(), m.stats().mixed);
            for t in 0..60u32 {
                let _ = m.tick(&mut dma, 100 * t);
                rt::check_in();
            }
            let (c, f) = (clocks::cycles().wrapping_sub(c0), m.stats().mixed - f0);
            let _ = writeln!(
                out,
                "  mixer, {rate} Hz {}{}: {} per frame mixed ({f} frames)",
                if ch == 1 { "mono" } else { "stereo" },
                if speaker { ", speaker filter" } else { "" },
                frames(c, f as u32)
            );
        }
    }

    fn on_fault(kind: rt::Fault, addr: u32) -> ! {
        crash_out(format_args!("FAULT {} at {addr:#010x}", kind.name()))
    }

    #[inline(never)]
    fn crash_out(msg: core::fmt::Arguments) -> ! {
        if let Some((regs, vram)) = devices::emergency_mbx() {
            let mut c =
                crash::emergency_console(regs, vram, (0, 0), (480, 64), 2, (0xFFFF, 0x0000));
            let _ = write!(c, "{msg}");
        }
        devices::crash_to_loader(msg);
        halt()
    }

    /// Quiet for `ms_`, then a wake: whatever is drawn next (a console
    /// line can take ~100 ms, scrolling) starts with the mix a full lead
    /// ahead, as the audio host's own thread would keep it.
    fn pause(out: &mut Tee<'_, '_>, rig: &mut Rig<'_, '_>, ms_: u32) {
        rig.run(out, ms_, &mut |_| false);
        rig.tick();
    }

    /// Test 1: the same pitch from two rates, back to back.
    fn two_notes(out: &mut Tee<'_, '_>, rig: &mut Rig<'_, '_>) {
        let _ = writeln!(
            out,
            "TEST 1: TWO NOTES, THE SAME PITCH (FROM 22 kHz, THEN 48 kHz)"
        );
        let mark = rig.mark(out);
        let Some(a) = rig.open(StreamConfig::new(Role::Media, 22_050, 1, 2_205)) else {
            let _ = writeln!(out, "  stream didn't open");
            return;
        };
        let mut sa = Synth::new(&[880], 1_000, 22_050, 12_000, 1, 1_000);
        rig.feed(a, &mut sa, 2_205);
        rig.tick();
        let Some(first) = rig.m.status(a, ms()).and_then(|s| s.actual_start) else {
            let _ = writeln!(out, "  the first note wasn't placed");
            return;
        };
        let mut cfg = StreamConfig::new(Role::Media, 48_000, 2, 2_400);
        cfg.start = Start::At(first + u64::from(OUT));
        let Some(b) = rig.open(cfg) else {
            let _ = writeln!(out, "  second stream didn't open");
            return;
        };
        let mut sb = Synth::new(&[880], 1_000, 48_000, 12_000, 2, 1_000);
        rig.run(out, 2_400, &mut |r| {
            let x = r.feed(a, &mut sa, 2_205);
            let y = r.feed(b, &mut sb, 2_400);
            x | y
        });
        let second = rig.m.status(b, ms()).and_then(|s| s.actual_start);
        rig.tick();
        let _ = writeln!(
            out,
            "  first note at frame {first}, second asked for {} and got {second:?}",
            first + u64::from(OUT)
        );
        rig.close(a);
        rig.close(b);
        rig.report(out, "TEST 1", &mark);
    }

    /// Test 2: a click for each tap, over the tune.
    fn taps(
        out: &mut Tee<'_, '_>,
        rig: &mut Rig<'_, '_>,
        touch: &mut Tsc2046<Ssp<Mmio>, GpioPin<'_, Mmio>, GpioPin<'_, Mmio>>,
    ) {
        let _ = writeln!(
            out,
            "TEST 2: TAP THE SCREEN {TAPS} TIMES: A CLICK AS THE PEN LANDS"
        );
        if QUICK {
            let _ = writeln!(out, "  (quick run: no taps)");
            return;
        }
        let mark = rig.mark(out);
        let cfg = StreamConfig::new(Role::Media, OUT, 2, 1_411);
        let Some(id) = rig.open(cfg) else {
            return;
        };
        let mut s = Synth::new(&TUNE, 250, OUT, 6_000, 2, u32::MAX / 2);
        let mut clicks: [Option<StreamId>; 4] = [None; 4];
        for n in 1..=TAPS {
            // Wait for the pen, the tune playing.
            let t0 = ms();
            let mut landed = false;
            while ms() - t0 < WAIT_S * 1_000 {
                if touch.pen_down() {
                    landed = true;
                    break;
                }
                if rig.feed(id, &mut s, 1_411) || ms() >= rig.next {
                    rig.tick();
                }
                rig.reap(&mut clicks);
                rt::delay_us(250);
                rt::check_in();
            }
            if !landed {
                let _ = writeln!(out, "  no tap in {WAIT_S} s");
                continue;
            }
            match rig.click() {
                Some((c, late10, ahead)) => {
                    let _ = writeln!(
                        out,
                        "  tap {n}: the click's first sample {}.{} ms after the pen landed (re-mixed {ahead} ms ahead)",
                        late10 / 10,
                        late10 % 10
                    );
                    if let Some(free) = clicks.iter_mut().find(|c| c.is_none()) {
                        *free = Some(c);
                    }
                }
                None => {
                    let _ = writeln!(out, "  tap {n}: the click didn't open");
                }
            }
            // The pen lifts.
            while touch.pen_down() {
                if rig.feed(id, &mut s, 1_411) || ms() >= rig.next {
                    rig.tick();
                }
                rig.reap(&mut clicks);
                rt::delay_us(250);
                rt::check_in();
            }
        }
        rig.m.end(id);
        pause(out, rig, 500);
        rig.reap(&mut clicks);
        rig.close(id);
        rig.report(out, "TEST 2", &mark);
    }

    /// Test 4: clicks every 0.4-0.6 s over the tune.
    fn clicks(out: &mut Tee<'_, '_>, rig: &mut Rig<'_, '_>) {
        let _ = writeln!(out, "TEST 4: THE TUNE AND CLICKS");
        let mark = rig.mark(out);
        let Some(id) = rig.open(StreamConfig::new(Role::Media, OUT, 2, 1_411)) else {
            return;
        };
        let mut s = Synth::new(&TUNE, 250, OUT, 6_000, 2, 6_000);
        let mut clicks: [Option<StreamId>; 4] = [None; 4];
        let mut next = ms() + 300;
        let mut rng = 0x1234_5678u32;
        let (mut n, mut worst, mut sum) = (0u64, 0u64, 0u64);
        rig.run(out, 6_300, &mut |r| {
            r.reap(&mut clicks);
            let wrote = r.feed(id, &mut s, 1_411);
            if ms() >= next && s.left > 0 {
                rng ^= rng << 13;
                rng ^= rng >> 17;
                rng ^= rng << 5;
                next = ms() + 400 + rng % 200;
                if let Some((c, late10, _)) = r.click() {
                    n += 1;
                    worst = worst.max(late10);
                    sum += late10;
                    if let Some(free) = clicks.iter_mut().find(|c| c.is_none()) {
                        *free = Some(c);
                    }
                }
            }
            wrote
        });
        let avg = sum / n.max(1);
        rig.tick();
        let _ = writeln!(
            out,
            "  {n} clicks: first sample after the write {}.{} ms on average, {}.{} ms at worst",
            avg / 10,
            avg % 10,
            worst / 10,
            worst % 10
        );
        rig.reap(&mut clicks);
        rig.close(id);
        rig.report(out, "TEST 4", &mark);
    }

    fn main() -> ! {
        rt::set_fault_handler(on_fault);
        rt::init_mmu();
        let Some((regs, vram)) = devices::take_mbx() else {
            halt()
        };
        let (plane, _) = Display::new(regs).adopt(MBX_VRAM_SIZE as usize);
        let end = plane.offset + plane.stride * usize::from(plane.height);
        let mut con = Console::new(
            &mut vram[plane.offset..end],
            plane.stride,
            (0, 0),
            (480, 640),
            2,
            (0xFFFF, 0x0000),
        );
        let mut out = Tee(&mut con, 0, 0);
        let _ = writeln!(
            out,
            "RetroPane audio S2: the mixer (44.1 kHz, level {LEVEL})"
        );
        let raw = clocks::read();
        let _ = writeln!(
            devices::RunLog,
            "core: {} kHz measured (CCCR {:08X}, CLKCFG {:X}: {} kHz by the registers)",
            clocks::measure_core_khz(20),
            raw.cccr,
            raw.clkcfg,
            raw.point().core_khz()
        );
        clocks::cycles_start();
        if BENCH {
            let cpld = devices::take_cpld().map(|(b, t)| Cpld::adopt(b, t).0);
            match devices::take_scratch() {
                Some(scratch) => bench(&mut out, scratch),
                None => {
                    let _ = writeln!(out, "no scratch");
                }
            }
            let found = found(cpld.as_ref());
            finish(cpld, found)
        }
        let cpld = devices::take_cpld().map(|(b, t)| Cpld::adopt(b, t).0);
        let found = found(cpld.as_ref());
        if found.is_none() {
            let _ = writeln!(out, "no CPLD: the codec can't be powered");
            finish(None, found)
        }
        let (Some(gpio_bus), Some(i2c), Some(i2s), Some(scratch), Some(dma), Some(ssp)) = (
            rt::take_gpio(),
            rt::take_i2c(),
            rt::take_i2s(),
            devices::take_scratch(),
            rt::take_dma(dma_map::AUDIO),
            rt::take_ssp1(),
        ) else {
            let _ = writeln!(out, "GPIO, I2C, I2S, scratch, DMA or SSP1 unavailable");
            finish(cpld, found)
        };
        let gpio = RefCell::new(Gpio::new(gpio_bus));
        // The touch chip: its select a plain GPIO (Windows Mobile leaves it
        // as SSP1's frame), one reading to arm its pen line (GPIO94).
        {
            let mut g = gpio.borrow_mut();
            g.make_output(TS_CS, true);
            g.set_function(TS_CS, 0);
            g.make_input(TS_PENIRQ);
        }
        let mut spi = Ssp::new(ssp);
        spi.configure_spi(2_600_000);
        let mut touch = Tsc2046::new(
            spi,
            GpioPin::new(&gpio, TS_CS),
            GpioPin::new(&gpio, TS_PENIRQ),
        );
        let _ = touch.read(rp_drv_tsc2046::channel::X);

        let mut a = rp_audio_x51v::service(&gpio, cpld, i2c, i2s, LEVEL);
        // The DMA ring first (the scratch is cache-line aligned), then the
        // stream rings.
        let (ring, mut rest) = scratch.split_at_mut(MIXER_BYTES);
        let mut pool: [Option<&mut [u8]>; POOL] = Default::default();
        for p in &mut pool {
            let (b, r) = core::mem::take(&mut rest).split_at_mut(POOL_BYTES);
            *p = Some(b);
            rest = r;
        }
        let _ = writeln!(out, "HEADPHONES IN FOR TESTS 1-5, OUT FOR 6-7");
        a.board_mut().delay_us(3_000_000);
        let output = match a.up() {
            Ok(p) => {
                let _ = writeln!(
                    out,
                    "up: codec at {:#04x}, to {}",
                    p.address,
                    name(p.output)
                );
                p.output
            }
            Err(e) => {
                let _ = writeln!(out, "up failed: {e:?}");
                let cpld = a.board_mut().cpld.take();
                finish(cpld, found)
            }
        };
        let mut rig = Rig {
            m: Mixer::new(ring, output == Output::Speaker),
            dma,
            pool,
            a,
            next: 0,
            busy: 0,
            cycles: 0,
            mixed: 0,
            lead_max: 0,
        };
        let started = rig.m.start(&mut rig.dma, ms());
        if started.is_err() {
            let _ = writeln!(out, "the DMA engine refused the ring");
            let _ = rig.a.down();
            let cpld = rig.a.board_mut().cpld.take();
            finish(cpld, found)
        }

        two_notes(&mut out, &mut rig);
        pause(&mut out, &mut rig, 2_000);
        taps(&mut out, &mut rig, &mut touch);
        pause(&mut out, &mut rig, 2_000);

        let _ = writeln!(out, "TEST 3: THE TUNE ALONE (LONG SLEEPS)");
        let mark = rig.mark(&out);
        rig.tune(&mut out, &TUNE, OUT, 2, 9_000, 6_000, UNITY, false);
        rig.report(&mut out, "TEST 3", &mark);
        pause(&mut out, &mut rig, 2_000);

        clicks(&mut out, &mut rig);
        pause(&mut out, &mut rig, 2_000);

        let _ = writeln!(out, "TEST 5: THE TUNE FROM 22 kHz, 11 kHz, THEN 48 kHz");
        for (part, rate, ch) in [
            ("5A 22 kHz", 22_050, 1),
            ("5B 11 kHz", 11_025, 1),
            ("5C 48 kHz", 48_000, 2),
        ] {
            let mark = rig.mark(&out);
            let _ = writeln!(out, "TEST {part}");
            rig.tune(&mut out, &TUNE, rate, ch, 9_000, 4_000, UNITY, false);
            rig.report(&mut out, part, &mark);
            pause(&mut out, &mut rig, 1_000);
        }

        let _ = writeln!(out, "TEST 6: PULL THE HEADPHONES OUT NOW ({WAIT_S} s)");
        if rig.a.output() != Some(Output::Speaker) && !QUICK {
            let t0 = ms();
            while ms() - t0 < WAIT_S * 1_000 {
                if rig.jack(&mut out) == Some(Output::Speaker) {
                    break;
                }
                if ms() >= rig.next {
                    rig.tick();
                }
                rt::delay_us(1_000);
                rt::check_in();
            }
        }
        rig.tick();
        let _ = writeln!(
            out,
            "  sound to the {}",
            rig.a.output().map_or("nowhere", name)
        );
        pause(&mut out, &mut rig, 2_000);
        let _ = writeln!(out, "TEST 6A: TWO LOUD PARTS TOGETHER (THE LIMITER)");
        let mark = rig.mark(&out);
        rig.tune(&mut out, &TUNE, OUT, 2, 30_000, 5_000, UNITY, true);
        rig.report(&mut out, "6A", &mark);
        pause(&mut out, &mut rig, 1_500);
        let _ = writeln!(
            out,
            "TEST 6B: THE SAME AT HALF VOLUME (NO LIMITER NEEDED: ABOUT AS LOUD AS 6A)"
        );
        let mark = rig.mark(&out);
        rig.tune(&mut out, &TUNE, OUT, 2, 30_000, 5_000, UNITY / 2, true);
        rig.report(&mut out, "6B", &mark);
        pause(&mut out, &mut rig, 2_000);

        let _ = writeln!(out, "TEST 7A: A LOW TUNE, SPEAKER FILTER ON");
        let mark = rig.mark(&out);
        rig.tune(&mut out, &LOW_TUNE, OUT, 2, 30_000, 5_000, UNITY, false);
        rig.report(&mut out, "7A", &mark);
        pause(&mut out, &mut rig, 1_500);
        let _ = writeln!(out, "TEST 7B: THE SAME, SPEAKER FILTER OFF");
        rig.m.set_speaker(false);
        let mark = rig.mark(&out);
        rig.tune(&mut out, &LOW_TUNE, OUT, 2, 30_000, 5_000, UNITY, false);
        rig.report(&mut out, "7B", &mark);
        rig.m.set_speaker(rig.a.output() == Some(Output::Speaker));
        pause(&mut out, &mut rig, 500);

        rig.m.stop(&mut rig.dma);
        let s = rig.m.stats();
        let _ = writeln!(
            out,
            "ALL: {} wakes, {} rewrites ({} ms re-mixed), {} late, {} dry, {} clipped, {} laps lost",
            s.wakes,
            s.rewrites,
            s.rewritten * 1_000 / u64::from(OUT),
            s.late,
            s.starved,
            s.clipped,
            s.laps_lost
        );
        match rig.a.down() {
            Ok(()) => {
                let _ = writeln!(out, "down");
            }
            Err(e) => {
                let _ = writeln!(out, "down, a codec write failed: {e:?} (supply off anyway)");
            }
        }
        let _ = writeln!(out, "DONE: TELL ME TESTS 1-7 (PITCH, CLICKS, CRACKLE)");
        let cpld = rig.a.board_mut().cpld.take();
        finish(cpld, found)
    }

    /// Puts the two board bits back as found, reports, returns to the loader.
    fn finish(cpld: Option<BoardCpld>, found: Option<(u16, u16)>) -> ! {
        put_back(cpld, found);
        let _ = writeln!(devices::RunLog, "{}", super::MARKER);
        if devices::loader_present() {
            rt::delay_us(1_000_000);
            devices::return_to_loader();
        }
        halt()
    }

    /// Writes to the screen and the run log.
    struct Tee<'a, 'b>(&'a mut Console<'b>, u32, u32);

    impl Tee<'_, '_> {
        /// Screen writes so far and the slowest (ms): drawing (and
        /// scrolling) the console stops everything else, the mixer's wakes
        /// included.
        fn screen(&self) -> (u32, u32) {
            (self.1, self.2)
        }
    }

    impl Write for Tee<'_, '_> {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let t0 = rt::ticks();
            let _ = self.0.write_str(s);
            let ms = rt::ticks().wrapping_sub(t0) / TICK_MS;
            self.1 += 1;
            self.2 = self.2.max(ms);
            devices::log_write(s.as_bytes());
            Ok(())
        }
    }

    fn halt() -> ! {
        loop {
            core::hint::spin_loop();
        }
    }

    #[panic_handler]
    fn panic(info: &core::panic::PanicInfo) -> ! {
        match info.location() {
            Some(l) => crash_out(format_args!("PANIC {}:{}", l.file(), l.line())),
            None => crash_out(format_args!("PANIC")),
        }
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}
