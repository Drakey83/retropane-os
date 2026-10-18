//! Audio check (docs/hw/audio.md): what the loader's `tone` command did,
//! as a test image of its own, so the loader doesn't carry the audio drivers
//! (the loader diet, 2026-10-16).
//!
//! 1. The WM8750 is looked for on I2C (at 0x1A, with its I2C gate,
//!    GPIO82, open for each write as Windows Mobile does: docs/hw/audio.md)
//!    four ways, one change each against run 2's: standard mode ~50 ms
//!    after power (run 2), fast mode (Windows Mobile's, measured), standard
//!    300 ms after power (Windows Mobile talked to it ~260 ms after), and
//!    fast at 300 ms. Then routed by the jack (GPIO83, high = plugged in,
//!    valid only with audio powered: measured under Windows Mobile).
//! 2. 440 Hz for 500 ms at 48 kHz, streamed through I2S by DMA from a ring
//!    the CPU refills by halves; the log says how long it really took (the
//!    codec's clock paces it) and how much of that the CPU was busy.
//! 3. Up to 15 s for the jack to change (plug the headphones in or out);
//!    on a change, 1000 Hz for 50 ms to the new output.
//!
//! The speaker amplifier (CPLD `0x14` bit 1, measured under Windows
//! Mobile: on only for the speaker) is switched on for the speaker, 70 ms
//! settle, and put back as found at the end, as the codec's power is.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed once everything was reported.
pub const MARKER: &str = "RETROPANE-TONE-OK";

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_drv_cpld::Cpld;
    use rp_drv_cpld::handover;
    use rp_drv_mbx_display::console::Console;
    use rp_drv_mbx_display::{Display, crash};
    use rp_drv_pxa_gpio::Gpio;
    use rp_drv_pxa_i2c::PxaI2c;
    use rp_drv_pxa_i2s::{PxaI2s, Rate, StreamError, Tone, stream};
    use rp_drv_wm8750::{Gate, Gated, Output, Wm8750};
    use rp_hal_api::I2cBus;
    use rp_hal_pxa27x::dma::Dma;
    use rp_hal_pxa27x::{Mmio, dma_map, rt};
    use rp_hal_x51v::MBX_VRAM_SIZE;
    use rp_hal_x51v::devices;

    rp_hal_pxa27x::entry!(main);

    /// Headphone jack: GPIO83, high = plugged in, read only with the codec
    /// powered (measured under Windows Mobile, `rp-wmaudio` 2026-10-18;
    /// GPIO96, taken for it before, reads 1 whenever audio is powered).
    const HP_DETECT: u8 = 83;
    /// The line taken for the jack before, logged beside it.
    const WAS_JACK: u8 = 96;
    /// Output volume, 0-100: 60, or `RP_TONE_LEVEL` at build time (a quiet
    /// run at night, 2026-10-18).
    const LEVEL: u32 = match option_env!("RP_TONE_LEVEL") {
        Some(v) => parse(v),
        None => 60,
    };

    /// A decimal number at compile time (100 at most).
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

    /// The codec's power: CPLD `0x1C` bit 1, set by Windows Mobile's audio
    /// power-up, off at hand-over (docs/hw/audio.md, likely).
    const CODEC_POWER: (usize, u16) = (0x1C, 1 << 1);
    /// The I2S pins as Windows Mobile sets them: (GPIO, function, output).
    const I2S_PINS: [(u8, u8, bool); 5] = [
        (28, 1, true),
        (29, 2, false),
        (30, 1, true),
        (31, 1, true),
        (113, 1, true),
    ];
    /// After switching it on, before the first I2C write.
    const POWER_SETTLE_US: u32 = 50_000;
    /// The speaker amplifier: CPLD `0x14` bit 1 (measured), 70 ms to settle
    /// (Dell's driver).
    const SPEAKER_AMP: (usize, u16) = (0x14, 1 << 1);
    const AMP_SETTLE_US: u32 = 70_000;
    /// The probe tries: name, fast mode, earliest time after power (ms).
    const TRIES: [(&str, bool, u32); 4] = [
        ("standard, as run 2", false, 0),
        ("fast mode", true, 0),
        ("standard, 300 ms after power", false, 300),
        ("fast, 300 ms after power", true, 300),
    ];

    /// The board's CPLD, and whether this run switched the codec's power on
    /// (then it switches it off again before returning).
    struct Power {
        cpld: Option<Cpld<Mmio, handover::Table<'static>>>,
        ours: bool,
        /// Whether this run switched the speaker amplifier on.
        amp: bool,
    }

    impl Power {
        /// The speaker amplifier on for the speaker, off for headphones.
        fn speaker(&mut self, on: bool) {
            let Some(c) = self.cpld.as_mut() else { return };
            if on && c.shadow(SPEAKER_AMP.0) & SPEAKER_AMP.1 == 0 {
                c.set(SPEAKER_AMP.0, SPEAKER_AMP.1);
                self.amp = true;
                rt::delay_us(AMP_SETTLE_US);
                let _ = writeln!(devices::RunLog, "speaker amplifier on (CPLD 14 bit 1)");
            } else if !on && self.amp {
                c.clear(SPEAKER_AMP.0, SPEAKER_AMP.1);
                self.amp = false;
                let _ = writeln!(devices::RunLog, "speaker amplifier off (headphones)");
            }
        }

        fn off(&mut self) {
            if self.amp
                && let Some(c) = self.cpld.as_mut()
            {
                c.clear(SPEAKER_AMP.0, SPEAKER_AMP.1);
                self.amp = false;
                let _ = writeln!(devices::RunLog, "speaker amplifier put back off");
            }
            if self.ours
                && let Some(c) = self.cpld.as_mut()
            {
                c.clear(CODEC_POWER.0, CODEC_POWER.1);
                self.ours = false;
                let _ = writeln!(devices::RunLog, "codec power put back off");
            }
        }
    }

    /// The codec's I2C gate: high for each transfer, low after.
    const I2C_GATE: u8 = 82;

    /// The GPIO block, holding the codec's I2C gate.
    struct Pins(Gpio<Mmio>);

    impl Gate for Pins {
        fn set(&mut self, open: bool) {
            self.0.write(I2C_GATE, open);
        }
    }

    type Codec = Wm8750<Gated<PxaI2c<Mmio>, Pins>>;
    /// OS timer ticks per second (3.25 MHz).
    const TICK_1S: u32 = 3_250_000;
    /// How long to wait for the jack to change.
    const JACK_WAIT_S: u32 = 15;
    /// The streaming ring (the CPU refills it by halves).
    const RING: usize = 8192;

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

    fn output(plugged: bool) -> Output {
        if plugged {
            Output::Headphones
        } else {
            Output::Speaker
        }
    }

    /// Plays `hz` for `ms` and logs how it went.
    fn play(
        out: &mut impl Write,
        codec: &Codec,
        i2s: &mut PxaI2s<Mmio>,
        dma: &mut Dma,
        ring: &mut [u8],
        hz: u32,
        ms: u32,
    ) {
        let rate = Rate::Khz48;
        let frames = rate.hz() / 1000 * ms;
        let mut source = Tone::new(hz, rate.hz(), 12_000);
        i2s.start(rate);
        let r = stream(
            dma,
            ring,
            &mut source,
            frames,
            &mut rt::ticks,
            TICK_1S / 5,
            &mut rt::check_in,
        );
        i2s.stop();
        match r {
            Ok(s) => {
                let _ = writeln!(
                    out,
                    "tone {hz} Hz {ms} ms ({frames} frames in {} ms, by DMA, CPU busy {}%), codec at {:#04x}, to {}",
                    s.total / u64::from(TICK_1S / 1000),
                    s.busy * 100 / s.total.max(1),
                    codec.address(),
                    if codec.output() == Output::Headphones {
                        "headphones"
                    } else {
                        "speaker"
                    }
                );
            }
            Err(e) => {
                let why = match e {
                    StreamError::Refused => "refused",
                    StreamError::Stalled => "stalled",
                };
                let _ = writeln!(out, "tone: DMA stream {why}");
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
        let _ = writeln!(out, "RetroPane 26 tone");
        // The codec's power, through the shared shadow (one unknown board
        // bit: an owner-watched run).
        let mut power = Power {
            cpld: devices::take_cpld().map(|(b, t)| Cpld::adopt(b, t).0),
            ours: false,
            amp: false,
        };

        if let Some(c) = power.cpld.as_ref() {
            let (r14, r1c) = (c.shadow(0x14), c.shadow(CODEC_POWER.0));
            let _ = writeln!(out, "CPLD found: 14={r14:04X} 1C={r1c:04X}");
        } else {
            let _ = writeln!(out, "tone: no CPLD");
        }
        let (Some(gpio_bus), Some(scratch)) = (rt::take_gpio(), devices::take_scratch()) else {
            let _ = writeln!(out, "tone: GPIO or scratch unavailable");
            finish(&mut power)
        };
        let mut gpio = Gpio::new(gpio_bus);
        gpio.make_input(HP_DETECT);
        // The pins as found: Windows Mobile turns the I2C pins into plain
        // outputs held low when it powers audio down (docs/hw/audio.md).
        let _ = writeln!(
            out,
            "found: GPIO117 fn {} {}, GPIO118 fn {} {}, GPIO82 fn {} {}",
            gpio.function(117),
            if gpio.is_output(117) { "out" } else { "in" },
            gpio.function(118),
            if gpio.is_output(118) { "out" } else { "in" },
            gpio.function(I2C_GATE),
            if gpio.is_output(I2C_GATE) {
                "out"
            } else {
                "in"
            },
        );
        // As Windows Mobile's audio power-up: the I2C pins to function 1
        // (SCL, SDA), outputs; the gate a plain GPIO, closed until a write
        // opens it.
        for pin in [117, 118] {
            gpio.set_function(pin, 1);
            gpio.make_output(pin, true);
        }
        gpio.set_function(I2C_GATE, 0);
        gpio.make_output(I2C_GATE, false);

        let (Some(i2c), Some(i2s)) = (rt::take_i2c(), rt::take_i2s()) else {
            let _ = writeln!(out, "tone: I2C or I2S unavailable");
            finish(&mut power)
        };
        // As Windows Mobile's power-up (docs/hw/audio.md, "Power-up order"):
        // the I2S pins (BITCLK, SDATA_IN, SDATA_OUT, SYNC, SYSCLK), then
        // the controller running, so the codec has its master clock
        // (SYSCLK, GPIO113) before the first I2C write; then 1 ms.
        for (pin, function, output) in I2S_PINS {
            gpio.set_function(pin, function);
            if output {
                gpio.make_output(pin, false);
            } else {
                gpio.make_input(pin);
            }
        }
        let mut i2s = PxaI2s::new(i2s);
        i2s.start(Rate::Khz48);
        rt::delay_us(1_000);
        let _ = writeln!(out, "I2S running before the codec (SYSCLK on GPIO113)");
        let mut gated = Gated {
            bus: PxaI2c::new(i2c),
            gate: Pins(gpio),
        };
        // The controller alone, before the codec's power: a working one
        // reports "nobody" within a millisecond (gate closed, then open).
        log_bus(&mut out, "idle, before any transfer", &mut gated.bus);
        let t = rt::ticks();
        let r = gated.bus.write(rp_drv_wm8750::ADDRESSES[0], &[]);
        log_try(
            &mut out,
            "controller check, gate closed, no power",
            r,
            t,
            &mut gated.bus,
        );
        let t = rt::ticks();
        let r = gated.write(rp_drv_wm8750::ADDRESSES[0], &[]);
        log_try(
            &mut out,
            "controller check, gate open, no power",
            r,
            t,
            &mut gated.bus,
        );
        // Now the codec's power (CPLD 0x1C bit 1, measured under Windows
        // Mobile), as runs 1-3.
        if let Some(c) = power.cpld.as_mut()
            && c.shadow(CODEC_POWER.0) & CODEC_POWER.1 == 0
        {
            c.set(CODEC_POWER.0, CODEC_POWER.1);
            power.ours = true;
            rt::delay_us(POWER_SETTLE_US);
            let _ = writeln!(out, "codec power on (CPLD 1C bit 1)");
        }
        let powered_at = rt::ticks();
        let mut plugged = gated.gate.0.read(HP_DETECT);
        let _ = writeln!(
            out,
            "headphones {} (GPIO83 {}; GPIO96 {})",
            if plugged { "in" } else { "out" },
            u8::from(plugged),
            u8::from(gated.gate.0.read(WAS_JACK))
        );
        log_bus(&mut out, "codec powered", &mut gated.bus);
        // Each try an address-only probe at both addresses, logged with its
        // time after power; every try runs, so each change is seen alone.
        let mut first = None;
        for (k, &(name, fast, after_ms)) in TRIES.iter().enumerate() {
            rt::check_in();
            while rt::ticks().wrapping_sub(powered_at) < after_ms * (TICK_1S / 1000) {}
            gated.bus.set_fast(fast);
            let ms = rt::ticks().wrapping_sub(powered_at) / (TICK_1S / 1000);
            let _ = writeln!(out, "probe {name} ({ms} ms after power):");
            let answered = rp_drv_wm8750::ADDRESSES.map(|a| {
                let t = rt::ticks();
                let r = gated.write(a, &[]);
                log_try(
                    &mut out,
                    if a == 0x1A { "  0x1a" } else { "  0x1b" },
                    r,
                    t,
                    &mut gated.bus,
                );
                r.is_ok()
            });
            if first.is_none() && answered.contains(&true) {
                first = Some(k);
            }
        }
        let Some(k) = first else {
            let _ = writeln!(out, "tone: no codec answered any of the four ways");
            finish(&mut power)
        };
        gated.bus.set_fast(TRIES[k].1);
        let mut codec: Codec = match Wm8750::init(gated, LEVEL) {
            Ok(c) => c,
            Err(_) => {
                let _ = writeln!(out, "tone: the codec answered the probe but not its set-up");
                finish(&mut power)
            }
        };
        let _ = writeln!(out, "codec at {:#04x}", codec.address());
        power.speaker(!plugged);
        let _ = codec.route(output(plugged));
        let Some(mut dma) = rt::take_dma(dma_map::AUDIO) else {
            let _ = writeln!(out, "tone: no DMA channel");
            finish(&mut power)
        };
        let ring = &mut scratch[..RING];
        play(&mut out, &codec, &mut i2s, &mut dma, ring, 440, 500);

        // The jack: a change that holds over two polls 1 s apart.
        let _ = writeln!(out, "PLUG HEADPHONES IN OR OUT NOW ({JACK_WAIT_S} s)");
        let mut seen = plugged;
        let mut changed = false;
        for _ in 0..JACK_WAIT_S {
            rt::check_in();
            rt::delay_us(1_000_000);
            let now = codec.bus_mut().gate.0.read(HP_DETECT);
            if now == seen && now != plugged {
                plugged = now;
                changed = true;
                break;
            }
            seen = now;
        }
        if changed {
            let _ = writeln!(
                out,
                "headphones {} (GPIO83 {})",
                if plugged { "in" } else { "out" },
                u8::from(plugged)
            );
            power.speaker(!plugged);
            let _ = codec.route(output(plugged));
            play(&mut out, &codec, &mut i2s, &mut dma, ring, 1000, 50);
        } else {
            let _ = writeln!(out, "jack: no change in {JACK_WAIT_S} s");
        }
        finish(&mut power)
    }

    /// The bus lines and the controller's registers.
    fn log_bus(out: &mut impl Write, what: &str, bus: &mut PxaI2c<Mmio>) {
        let [ibmr, icr, isr] = bus.snapshot();
        let _ = writeln!(
            out,
            "  bus {what}: SCL {} SDA {} (IBMR {ibmr:x}), ICR {icr:x}, ISR {isr:x}",
            (ibmr >> 1) & 1,
            ibmr & 1
        );
    }

    /// One transfer's outcome: what kind, how long, the controller after.
    fn log_try(
        out: &mut impl Write,
        what: &str,
        r: Result<(), rp_hal_api::I2cError>,
        t0: u32,
        bus: &mut PxaI2c<Mmio>,
    ) {
        let us = rt::ticks().wrapping_sub(t0) / 13 * 4;
        let kind = match r {
            Ok(()) => "ACK",
            Err(rp_hal_api::I2cError::NoDevice) => "no acknowledge",
            Err(rp_hal_api::I2cError::Nak) => "data refused",
            Err(rp_hal_api::I2cError::Bus) => "bus never finished (timeout)",
        };
        let _ = writeln!(out, "{what}: {kind} in {us} us");
        log_bus(out, "after", bus);
    }

    fn finish(power: &mut Power) -> ! {
        power.off();
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
