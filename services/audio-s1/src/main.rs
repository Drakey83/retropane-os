//! Audio service step S1 on the X51v (ADR-0032), a guided run with the
//! owner listening. Everything goes through `rp-audio`, the service's core:
//!
//! 1. Audio up (44.1 kHz, the session rate: vote A1), routed by the jack;
//!    three notes, a second each.
//! 2. "Unplug / plug in the headphones now": the jack's edge latch while up,
//!    debounced, re-routes; a note to the new output.
//! 3. Audio down, then straight back up: the I2S restart guard waits out
//!    the ~645 ms the X51v needs (logged), then a note.
//! 4. Audio down; "change the headphones again now": does a jack edge come
//!    at all while the codec is unpowered (measured: GPIO83 reads 0 either
//!    way when idle, so likely not)? Then up: the jack read at power-up
//!    routes it; a note.
//! 5. Measured throughout: GPIO83 at 0, 25, 75 and 150 ms after the codec's
//!    set-up (when is the jack line valid? Dell's driver waits 75 ms).
//!
//! The board bits (codec power CPLD `0x1C` bit 1, speaker amplifier `0x14`
//! bit 1) are switched only through the shared shadow and put back as
//! found at the end.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed once everything was reported.
pub const MARKER: &str = "RETROPANE-AUDIO-S1-OK";

#[cfg(target_os = "none")]
mod image {
    use core::cell::RefCell;
    use core::fmt::Write;

    use rp_audio::{Audio, Board, PlayError};
    use rp_drv_cpld::Cpld;
    use rp_drv_cpld::handover;
    use rp_drv_mbx_display::console::Console;
    use rp_drv_mbx_display::{Display, crash};
    use rp_drv_pxa_gpio::Gpio;
    use rp_drv_pxa_i2c::PxaI2c;
    use rp_drv_pxa_i2s::{PxaI2s, Tone};
    use rp_drv_wm8750::{Gate, Gated, Output};
    use rp_hal_pxa27x::dma::Dma;
    use rp_hal_pxa27x::{Mmio, dma_map, rt};
    use rp_hal_x51v::MBX_VRAM_SIZE;
    use rp_hal_x51v::devices;

    rp_hal_pxa27x::entry!(main);

    /// The headphone jack (high = plugged in, with the codec powered).
    const JACK: u8 = 83;
    /// The codec's I2C gate.
    const I2C_GATE: u8 = 82;
    const CODEC_POWER: (usize, u16) = (0x1C, 1 << 1);
    const SPEAKER_AMP: (usize, u16) = (0x14, 1 << 1);
    /// The I2S pins as Windows Mobile sets them: (GPIO, function, output).
    const I2S_PINS: [(u8, u8, bool); 5] = [
        (28, 1, true),
        (29, 2, false),
        (30, 1, true),
        (31, 1, true),
        (113, 1, true),
    ];
    /// `RP_AUDIO_AMP_TEST` at build time: the speaker-only diagnostic (S1
    /// runs 2-3: one of two speaker notes unheard), numbered tests instead
    /// of the steps.
    const AMP_TEST: bool = option_env!("RP_AUDIO_AMP_TEST").is_some();
    /// `RP_AUDIO_AMP_TIMING` at build time: how long the speaker amplifier
    /// takes to start from cold (the amplifier diagnostic: the first speaker
    /// sound after a long quiet spell is lost): numbered short notes every
    /// 0.5 s from its switch-on.
    const AMP_TIMING: bool = option_env!("RP_AUDIO_AMP_TIMING").is_some();
    /// `RP_AUDIO_AB_TEST` at build time: the unheard 440 Hz speaker note,
    /// one change per test (pitch, power-up, level), numbered on screen.
    const AB_TEST: bool = option_env!("RP_AUDIO_AB_TEST").is_some();
    /// `RP_AUDIO_SWEEP` at build time: the speaker's response, read by the
    /// owner's phone sound meter: a quiet reading, then numbered pitches,
    /// high to low, 3 s each at full level.
    const SWEEP: bool = option_env!("RP_AUDIO_SWEEP").is_some();
    /// Output level, 0-100: 60, or `RP_AUDIO_LEVEL` at build time.
    const LEVEL: u32 = match option_env!("RP_AUDIO_LEVEL") {
        Some(v) => parse(v),
        None => 60,
    };
    /// OS timer ticks per millisecond (3.25 MHz).
    const TICK_MS: u32 = 3_250;
    const RING: usize = 8192;
    const NOTE_MS: u32 = 1_000;
    /// How long each "now" step waits for the owner.
    const CHANGE_WAIT_S: u32 = 30;
    const DOWN_WAIT_S: u32 = 15;

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

    type SharedGpio = RefCell<Gpio<Mmio>>;

    /// The X51v's audio lines.
    struct X51vAudio<'a> {
        gpio: &'a SharedGpio,
        cpld: Option<Cpld<Mmio, handover::Table<'static>>>,
        /// When the codec's supply came on (OS timer ticks), for the jack's
        /// settle measurement.
        powered_at: u32,
        /// When the speaker amplifier was last switched on (OS timer ticks).
        amp_on_at: u32,
    }

    impl X51vAudio<'_> {
        fn bit(&mut self, (reg, bit): (usize, u16), on: bool) {
            if let Some(c) = self.cpld.as_mut() {
                if on {
                    c.set(reg, bit);
                } else {
                    c.clear(reg, bit);
                }
            }
        }
    }

    impl Board for X51vAudio<'_> {
        fn codec_power(&mut self, on: bool) {
            self.bit(CODEC_POWER, on);
            if on {
                self.powered_at = rt::ticks();
            }
            let _ = writeln!(devices::RunLog, "board: codec power {}", onoff(on));
        }
        fn speaker_amp(&mut self, on: bool) {
            self.bit(SPEAKER_AMP, on);
            if on {
                self.amp_on_at = rt::ticks();
            }
            let _ = writeln!(devices::RunLog, "board: speaker amplifier {}", onoff(on));
        }
        fn pins_up(&mut self) {
            let mut g = self.gpio.borrow_mut();
            for pin in [117, 118] {
                g.set_function(pin, 1);
                g.make_output(pin, true);
            }
            g.set_function(I2C_GATE, 0);
            g.make_output(I2C_GATE, false);
            for (pin, function, output) in I2S_PINS {
                g.set_function(pin, function);
                if output {
                    g.make_output(pin, false);
                } else {
                    g.make_input(pin);
                }
            }
        }
        fn pins_down(&mut self) {
            // As Windows Mobile's audio power-down: the I2C lines plain
            // outputs held low; the gate closed.
            let mut g = self.gpio.borrow_mut();
            for pin in [117, 118] {
                g.make_output(pin, false);
                g.set_function(pin, 0);
            }
            g.write(I2C_GATE, false);
        }
        fn jack_in(&mut self) -> bool {
            let level = self.gpio.borrow_mut().read(JACK);
            let ms = rt::ticks().wrapping_sub(self.powered_at) / TICK_MS;
            let _ = writeln!(
                devices::RunLog,
                "board: jack read {ms} ms after power: GPIO83 {}",
                u8::from(level)
            );
            level
        }
        fn take_jack_edge(&mut self) -> bool {
            self.gpio.borrow_mut().take_edge(JACK)
        }
        fn now_ms(&mut self) -> u32 {
            rt::ticks() / TICK_MS
        }
        fn delay_us(&mut self, us: u32) {
            // In slices, the watchdog fed between.
            let mut left = us;
            while left > 0 {
                let step = left.min(100_000);
                rt::delay_us(step);
                rt::check_in();
                left -= step;
            }
        }
    }

    /// The codec's I2C gate on the shared GPIO block.
    struct GatePin<'a>(&'a SharedGpio);

    impl Gate for GatePin<'_> {
        fn set(&mut self, open: bool) {
            self.0.borrow_mut().write(I2C_GATE, open);
        }
    }

    type Service<'a> = Audio<X51vAudio<'a>, Gated<PxaI2c<Mmio>, GatePin<'a>>, PxaI2s<Mmio>>;

    fn onoff(on: bool) -> &'static str {
        if on { "on" } else { "off" }
    }

    fn name(o: Output) -> &'static str {
        match o {
            Output::Headphones => "headphones",
            Output::Speaker => "speaker",
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

    /// Brings audio up and logs it.
    fn up(out: &mut impl Write, a: &mut Service<'_>, what: &str) -> bool {
        match a.up() {
            Ok(p) => {
                let _ = writeln!(
                    out,
                    "up ({what}): codec at {:#04x}, to {}, waited {} ms for the I2S guard, jack edge while down: {}",
                    p.address,
                    name(p.output),
                    p.waited_ms,
                    p.jack_was_dirty
                );
                // When is the jack line valid? Sampled every 50 ms for 1 s
                // after the power-up (which read it after Dell's 75 ms).
                let _ = write!(out, "  GPIO83 every 50 ms after up:");
                for _ in 0..20 {
                    let level = a.board_mut().gpio.borrow_mut().read(JACK);
                    let _ = write!(out, "{}", u8::from(level));
                    a.board_mut().delay_us(50_000);
                }
                let _ = writeln!(out);
                true
            }
            Err(e) => {
                let _ = writeln!(out, "up ({what}) failed: {e:?} (everything put back down)");
                false
            }
        }
    }

    /// Plays `hz` for a second and logs it.
    fn note(out: &mut impl Write, a: &mut Service<'_>, dma: &mut Dma, ring: &mut [u8], hz: u32) {
        let rate = rp_audio::LINK_RATE.hz();
        let frames = rate * NOTE_MS / 1000;
        let mut tone = Tone::new(hz, rate, 12_000);
        let r = a.play(
            dma,
            ring,
            &mut tone,
            frames,
            &mut rt::ticks,
            3_250_000 / 5,
            &mut rt::check_in,
        );
        let to = a.output().map_or("nowhere", name);
        match r {
            Ok(s) => {
                let _ = writeln!(
                    out,
                    "note {hz} Hz: {frames} frames in {} ms by DMA (CPU busy {}%), to {to}",
                    s.total / u64::from(TICK_MS),
                    s.busy * 100 / s.total.max(1)
                );
            }
            Err(PlayError::Up(e)) => {
                let _ = writeln!(out, "note {hz} Hz: audio didn't come up: {e:?}");
            }
            Err(PlayError::Stream(e)) => {
                let _ = writeln!(out, "note {hz} Hz: stream {e:?}");
            }
        }
    }

    /// The speaker-only diagnostic: each test one change, numbered on the
    /// screen just before its note, a different pitch each, 2 s of quiet
    /// after. The owner counts which numbers they heard.
    fn amp_test(out: &mut impl Write, a: &mut Service<'_>, dma: &mut Dma, ring: &mut [u8]) {
        let _ = writeln!(out, "AMP TEST: HEADPHONES OUT THE WHOLE TIME");
        let _ = writeln!(out, "WATCH FOR TEST 1 TO TEST 5; NOTE WHICH ONES YOU HEAR");
        a.board_mut().delay_us(3_000_000);
        if !up(out, a, "amp test") {
            return;
        }
        // (test, note, amplifier off for ms (0 = not cycled), settle ms,
        // power down and up instead)
        let tests: [(u32, u32, u32, u32, bool); 5] = [
            (1, 440, 0, 0, false),
            (2, 523, 900, 70, false),
            (3, 587, 900, 500, false),
            (4, 659, 3_000, 70, false),
            (5, 784, 0, 0, true),
        ];
        for (n, hz, off_ms, settle_ms, cycle) in tests {
            if cycle {
                down(out, a);
                if !up(out, a, "test 5") {
                    return;
                }
            } else if off_ms > 0 {
                a.board_mut().speaker_amp(false);
                a.board_mut().delay_us(off_ms * 1_000);
                a.board_mut().speaker_amp(true);
                a.board_mut().delay_us(settle_ms * 1_000);
            }
            let _ = writeln!(
                out,
                "TEST {n}: amplifier off {off_ms} ms, settle {settle_ms} ms{}",
                if cycle { ", audio down and up" } else { "" }
            );
            note(out, a, dma, ring, hz);
            a.board_mut().delay_us(2_000_000);
        }
        down(out, a);
        let _ = writeln!(out, "DONE: WHICH TESTS DID YOU HEAR?");
    }

    /// The amplifier's cold start, timed: audio up to the speaker, then 20
    /// numbered notes of 150 ms, one every 0.5 s, each logged with its time
    /// since the amplifier's switch-on. The owner remembers the first number
    /// heard.
    fn amp_timing(out: &mut impl Write, a: &mut Service<'_>, dma: &mut Dma, ring: &mut [u8]) {
        let _ = writeln!(out, "AMP TIMING: HEADPHONES OUT THE WHOLE TIME");
        let _ = writeln!(out, "REMEMBER THE FIRST NUMBER YOU HEAR A BEEP FOR");
        a.board_mut().delay_us(3_000_000);
        match a.up() {
            Ok(p) => {
                let _ = writeln!(out, "up: to {}", name(p.output));
            }
            Err(e) => {
                let _ = writeln!(out, "up failed: {e:?}");
                return;
            }
        }
        let rate = rp_audio::LINK_RATE.hz();
        for n in 1..=20u32 {
            let start = rt::ticks();
            let ms = start.wrapping_sub(a.board_mut().amp_on_at) / TICK_MS;
            let _ = writeln!(out, "BEEP {n} ({ms} ms after the amplifier came on)");
            let mut tone = Tone::new(660, rate, 12_000);
            let _ = a.play(
                dma,
                ring,
                &mut tone,
                rate * 150 / 1000,
                &mut rt::ticks,
                3_250_000 / 5,
                &mut rt::check_in,
            );
            // The next beep 0.5 s after this one started.
            let used = rt::ticks().wrapping_sub(start) / TICK_MS;
            a.board_mut().delay_us(500u32.saturating_sub(used) * 1_000);
        }
        down(out, a);
        let _ = writeln!(out, "DONE: WHAT WAS THE FIRST NUMBER YOU HEARD?");
    }

    /// One change at a time: 660 and 440 Hz alternating with nothing else
    /// changed (tests 1-4), each after a power-down and up as runs 2-3's
    /// step 3 did (5, 6), and 440 Hz at full level (7). Each test's number
    /// is on screen just before its 1 s note, then 5 s of quiet (time to
    /// write it down).
    fn ab_test(out: &mut impl Write, a: &mut Service<'_>, dma: &mut Dma, ring: &mut [u8]) {
        let _ = writeln!(out, "A/B TEST: HEADPHONES OUT THE WHOLE TIME");
        let _ = writeln!(out, "WATCH FOR TEST 1 TO TEST 7; NOTE WHICH ONES YOU HEAR");
        a.board_mut().delay_us(3_000_000);
        if !up(out, a, "a/b test") {
            return;
        }
        // (test, pitch, power down and up first, full level)
        let tests: [(u32, u32, bool, bool); 7] = [
            (1, 660, false, false),
            (2, 440, false, false),
            (3, 660, false, false),
            (4, 440, false, false),
            (5, 660, true, false),
            (6, 440, true, false),
            (7, 440, false, true),
        ];
        for (n, hz, cycle, full) in tests {
            if cycle {
                down(out, a);
                if !up(out, a, "a/b cycle") {
                    return;
                }
            }
            if full {
                let _ = a.set_level(100);
            }
            let _ = writeln!(
                out,
                "TEST {n}: {hz} Hz{}{}, level {}",
                if cycle {
                    ", after audio down and up"
                } else {
                    ""
                },
                if full { ", full level" } else { "" },
                a.level()
            );
            note(out, a, dma, ring, hz);
            if full {
                let _ = a.set_level(LEVEL);
            }
            // Time for the owner to write each one down.
            a.board_mut().delay_us(5_000_000);
        }
        down(out, a);
        let _ = writeln!(out, "DONE: WHICH TESTS DID YOU HEAR?");
    }

    /// NOTE 0 is 4 s of quiet (the room's own level), then ten pitches from
    /// 2 kHz down to 220 Hz on the speaker at full level, 3 s each (a phone
    /// meter settles), each number on screen first, 5 s of quiet after
    /// (time to write the reading down).
    fn sweep(out: &mut impl Write, a: &mut Service<'_>, dma: &mut Dma, ring: &mut [u8]) {
        let _ = writeln!(out, "SPEAKER SWEEP: HEADPHONES OUT, METER AT THE SPEAKER");
        let _ = writeln!(
            out,
            "TAKE IT OUT OF THE DOCK NOW: NOTE 0 COMES IN 30 SECONDS"
        );
        a.board_mut().delay_us(30_000_000);
        if !up(out, a, "sweep") {
            return;
        }
        let _ = a.set_level(100);
        let rate = rp_audio::LINK_RATE.hz();
        let _ = writeln!(out, "NOTE 0: QUIET - WRITE DOWN THE ROOM'S READING");
        a.board_mut().delay_us(5_000_000);
        for (n, hz) in [2000u32, 1500, 1000, 800, 660, 523, 440, 330, 262, 220]
            .into_iter()
            .enumerate()
        {
            let _ = writeln!(out, "NOTE {}: {hz} Hz", n + 1);
            let mut tone = Tone::new(hz, rate, 12_000);
            if a.play(
                dma,
                ring,
                &mut tone,
                rate * 3,
                &mut rt::ticks,
                3_250_000 / 5,
                &mut rt::check_in,
            )
            .is_err()
            {
                let _ = writeln!(out, "  note {} did not play", n + 1);
            }
            a.board_mut().delay_us(5_000_000);
        }
        let _ = a.set_level(LEVEL);
        down(out, a);
        let _ = writeln!(out, "DONE: SEND ME THE 11 READINGS, NOTE 0 TO NOTE 10");
    }

    fn down(out: &mut impl Write, a: &mut Service<'_>) {
        match a.down() {
            Ok(()) => {
                let _ = writeln!(out, "down");
            }
            Err(e) => {
                let _ = writeln!(out, "down, a codec write failed: {e:?} (supply off anyway)");
            }
        }
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
        let mut out = Tee(&mut con);
        let _ = writeln!(out, "RetroPane audio S1 (44.1 kHz, level {LEVEL})");
        let cpld = devices::take_cpld().map(|(b, t)| Cpld::adopt(b, t).0);
        let found = cpld
            .as_ref()
            .map(|c| (c.shadow(SPEAKER_AMP.0), c.shadow(CODEC_POWER.0)));
        match found {
            Some((r14, r1c)) => {
                let _ = writeln!(out, "CPLD found: 14={r14:04X} 1C={r1c:04X}");
            }
            None => {
                let _ = writeln!(out, "no CPLD: the codec can't be powered");
                finish(None, found)
            }
        }
        let (Some(gpio_bus), Some(i2c), Some(i2s), Some(scratch), Some(mut dma)) = (
            rt::take_gpio(),
            rt::take_i2c(),
            rt::take_i2s(),
            devices::take_scratch(),
            rt::take_dma(dma_map::AUDIO),
        ) else {
            let _ = writeln!(out, "GPIO, I2C, I2S, scratch or DMA unavailable");
            finish(cpld, found)
        };
        let gpio = RefCell::new(Gpio::new(gpio_bus));
        {
            let mut g = gpio.borrow_mut();
            g.make_input(JACK);
            g.watch_edges(JACK, true, true);
            let _ = g.take_edge(JACK);
        }
        let board = X51vAudio {
            gpio: &gpio,
            cpld,
            powered_at: 0,
            amp_on_at: 0,
        };
        let bus = Gated {
            bus: PxaI2c::new(i2c),
            gate: GatePin(&gpio),
        };
        let mut a: Service<'_> = Audio::new(board, PxaI2s::new(i2s), bus, LEVEL);
        let ring = &mut scratch[..RING];

        // The owner's steps, the same words as in the chat: headphones in
        // at the start; fixed prompts that never depend on a reading.
        if SWEEP {
            sweep(&mut out, &mut a, &mut dma, ring);
            let cpld = a.board_mut().cpld.take();
            finish(cpld, found)
        }
        if AB_TEST {
            ab_test(&mut out, &mut a, &mut dma, ring);
            let cpld = a.board_mut().cpld.take();
            finish(cpld, found)
        }
        if AMP_TIMING {
            amp_timing(&mut out, &mut a, &mut dma, ring);
            let cpld = a.board_mut().cpld.take();
            finish(cpld, found)
        }
        if AMP_TEST {
            amp_test(&mut out, &mut a, &mut dma, ring);
            let cpld = a.board_mut().cpld.take();
            finish(cpld, found)
        }
        let _ = writeln!(out, "STEP 1: HEADPHONES IN. LISTEN: 3 NOTES");
        if !up(&mut out, &mut a, "first") {
            finish(a.board_mut().cpld.take(), found)
        }
        for hz in [440, 660, 880] {
            note(&mut out, &mut a, &mut dma, ring, hz);
        }

        // 2. The jack while up.
        let _ = writeln!(
            out,
            "STEP 2: PULL THE HEADPHONES OUT NOW ({CHANGE_WAIT_S} s). LISTEN: 1 NOTE"
        );
        let mut rerouted = None;
        for _ in 0..CHANGE_WAIT_S * 100 {
            if let Some(o) = a.poll_jack() {
                rerouted = Some(o);
                break;
            }
            a.board_mut().delay_us(10_000);
        }
        match rerouted {
            Some(o) => {
                let _ = writeln!(out, "jack: rerouted to {} (edge latch, debounced)", name(o));
                note(&mut out, &mut a, &mut dma, ring, 660);
            }
            None => {
                let _ = writeln!(out, "jack: no change in {CHANGE_WAIT_S} s");
            }
        }

        // 3. Down, then straight back up: the guard.
        let _ = writeln!(
            out,
            "STEP 3: LEAVE THEM OUT. LISTEN: 1 NOTE AFTER A SHORT PAUSE"
        );
        down(&mut out, &mut a);
        if up(&mut out, &mut a, "straight after down") {
            note(&mut out, &mut a, &mut dma, ring, 440);
        }

        // 4. The jack while down.
        down(&mut out, &mut a);
        let _ = writeln!(
            out,
            "STEP 4: PUT THE HEADPHONES BACK IN NOW ({DOWN_WAIT_S} s). THEN LISTEN: 1 NOTE"
        );
        for _ in 0..DOWN_WAIT_S * 100 {
            let _ = a.poll_jack();
            a.board_mut().delay_us(10_000);
        }
        let _ = writeln!(
            out,
            "while down: a jack edge came: {} (nothing was powered for it)",
            a.jack_dirty()
        );
        if up(&mut out, &mut a, "after the change while down") {
            note(&mut out, &mut a, &mut dma, ring, 880);
        }
        down(&mut out, &mut a);
        let _ = writeln!(out, "DONE: 6 NOTES IN ALL");
        let cpld = a.board_mut().cpld.take();
        finish(cpld, found)
    }

    /// Puts the two board bits back as found, reports, returns to the loader.
    fn finish(cpld: Option<Cpld<Mmio, handover::Table<'static>>>, found: Option<(u16, u16)>) -> ! {
        if let (Some(mut c), Some((r14, r1c))) = (cpld, found) {
            for ((reg, bit), was) in [(SPEAKER_AMP, r14), (CODEC_POWER, r1c)] {
                if was & bit != 0 {
                    c.set(reg, bit);
                } else {
                    c.clear(reg, bit);
                }
            }
            let _ = writeln!(
                devices::RunLog,
                "board bits put back as found: 14={:04X} 1C={:04X}",
                c.shadow(SPEAKER_AMP.0),
                c.shadow(CODEC_POWER.0)
            );
        }
        let _ = writeln!(devices::RunLog, "{}", super::MARKER);
        if devices::loader_present() {
            rt::delay_us(1_000_000);
            devices::return_to_loader();
        }
        halt()
    }

    /// Writes to the screen and the run log.
    struct Tee<'a, 'b>(&'a mut Console<'b>);

    impl Write for Tee<'_, '_> {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let _ = self.0.write_str(s);
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
