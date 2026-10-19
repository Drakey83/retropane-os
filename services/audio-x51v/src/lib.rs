//! The X51v's audio wiring, as the audio service ([`rp_audio`]) sees it:
//! the codec's supply (CPLD `0x1C` bit 1) and the speaker amplifier (CPLD
//! `0x14` bit 1) through the shared shadow, the I2C and I2S pins, the
//! codec's I2C gate (GPIO82) and the headphone jack (GPIO83, high while the
//! codec is powered and set up). Moved out of the S1 image (where it was
//! proven on the device, 2026-10-19) so every audio image and the audio
//! driver host use the one copy.
//!
//! Every board switch is logged to the run log ("board: ..."), as the
//! emulator checks and the device logs expect.

#![cfg_attr(target_os = "none", no_std)]
#![forbid(unsafe_code)]

#[cfg(target_os = "none")]
mod x51v {
    use core::cell::RefCell;
    use core::fmt::Write;

    use rp_audio::{Audio, Board};
    use rp_drv_cpld::{Cpld, handover};
    use rp_drv_pxa_gpio::Gpio;
    use rp_drv_pxa_i2c::PxaI2c;
    use rp_drv_pxa_i2s::PxaI2s;
    use rp_drv_wm8750::{Gate, Gated, Output};
    use rp_hal_pxa27x::{Mmio, rt};
    use rp_hal_x51v::devices;

    /// The headphone jack (high = plugged in, with the codec powered).
    pub const JACK: u8 = 83;
    /// The codec's I2C gate.
    pub const I2C_GATE: u8 = 82;
    /// The codec's supply: (CPLD register, bit).
    pub const CODEC_POWER: (usize, u16) = (0x1C, 1 << 1);
    /// The speaker amplifier: (CPLD register, bit).
    pub const SPEAKER_AMP: (usize, u16) = (0x14, 1 << 1);
    /// The I2S pins as Windows Mobile sets them: (GPIO, function, output).
    pub const I2S_PINS: [(u8, u8, bool); 5] = [
        (28, 1, true),
        (29, 2, false),
        (30, 1, true),
        (31, 1, true),
        (113, 1, true),
    ];
    /// OS timer ticks per millisecond (3.25 MHz).
    pub const TICK_MS: u32 = 3_250;

    /// The GPIO block, shared by the board, the gate and the image.
    pub type SharedGpio = RefCell<Gpio<Mmio>>;
    /// The CPLD with the handover's shadow.
    pub type BoardCpld = Cpld<Mmio, handover::Table<'static>>;

    /// The X51v's audio lines.
    pub struct X51vAudio<'a> {
        /// The GPIO block.
        pub gpio: &'a SharedGpio,
        /// The CPLD (none: nothing can be powered).
        pub cpld: Option<BoardCpld>,
        /// When the codec's supply came on (OS timer ticks), for the jack's
        /// settle measurement.
        pub powered_at: u32,
        /// When the speaker amplifier was last switched on (OS timer ticks).
        pub amp_on_at: u32,
    }

    impl<'a> X51vAudio<'a> {
        /// The lines over `gpio` and `cpld`: the jack an input with both
        /// edges latched (and the latch cleared).
        pub fn new(gpio: &'a SharedGpio, cpld: Option<BoardCpld>) -> Self {
            {
                let mut g = gpio.borrow_mut();
                g.make_input(JACK);
                g.watch_edges(JACK, true, true);
                let _ = g.take_edge(JACK);
            }
            Self {
                gpio,
                cpld,
                powered_at: 0,
                amp_on_at: 0,
            }
        }

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
    pub struct GatePin<'a>(pub &'a SharedGpio);

    impl Gate for GatePin<'_> {
        fn set(&mut self, open: bool) {
            self.0.borrow_mut().write(I2C_GATE, open);
        }
    }

    /// The audio service over the X51v's lines.
    pub type Service<'a> = Audio<X51vAudio<'a>, Gated<PxaI2c<Mmio>, GatePin<'a>>, PxaI2s<Mmio>>;

    /// The service over the X51v: audio down, `level` 0-100.
    pub fn service<'a>(
        gpio: &'a SharedGpio,
        cpld: Option<BoardCpld>,
        i2c: Mmio,
        i2s: Mmio,
        level: u32,
    ) -> Service<'a> {
        let bus = Gated {
            bus: PxaI2c::new(i2c),
            gate: GatePin(gpio),
        };
        Audio::new(X51vAudio::new(gpio, cpld), PxaI2s::new(i2s), bus, level)
    }

    /// The two board bits as found at the start (to put them back).
    #[must_use]
    pub fn found(cpld: Option<&BoardCpld>) -> Option<(u16, u16)> {
        cpld.map(|c| (c.shadow(SPEAKER_AMP.0), c.shadow(CODEC_POWER.0)))
    }

    /// Puts the two board bits back as `found` them, and logs it.
    pub fn put_back(cpld: Option<BoardCpld>, found: Option<(u16, u16)>) {
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
    }

    /// "on" or "off".
    #[must_use]
    pub fn onoff(on: bool) -> &'static str {
        if on { "on" } else { "off" }
    }

    /// An output's name for the log.
    #[must_use]
    pub fn name(o: Output) -> &'static str {
        match o {
            Output::Headphones => "headphones",
            Output::Speaker => "speaker",
        }
    }
}

#[cfg(target_os = "none")]
pub use x51v::*;
