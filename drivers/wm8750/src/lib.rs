//! Wolfson WM8750 stereo codec (public datasheet), playback side: found on
//! the I2C bus at either of its two addresses (the board's CSB pin picks
//! one; the X51v's isn't recorded, so both are tried), then set up as an
//! I2S slave taking 16-bit samples, DACs routed to both output pairs
//! (headphones: LOUT1/ROUT1; speaker: LOUT2/ROUT2).
//!
//! Registers are write-only: each write is two bytes, the 7-bit register
//! number and the 9-bit value. The driver keeps what it wrote.
//!
//! On the X51v the codec's I2C lines are gated by GPIO82: Windows Mobile's
//! driver drives it high for each transfer and low after (docs/hw/audio.md,
//! static read 2026-10-17); without it the codec never answers. [`Gated`]
//! does that around every write.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use rp_hal_api::{I2cBus, I2cError};

/// The two addresses the codec answers on (CSB low, high).
pub const ADDRESSES: [u8; 2] = [0x1A, 0x1B];

mod reg {
    pub const LOUT1: u8 = 0x02;
    pub const ROUT1: u8 = 0x03;
    pub const DAC_CTRL: u8 = 0x05;
    pub const IFACE: u8 = 0x07;
    pub const SRATE: u8 = 0x08;
    pub const RESET: u8 = 0x0F;
    pub const PWR1: u8 = 0x19;
    pub const PWR2: u8 = 0x1A;
    pub const LMIX1: u8 = 0x22;
    pub const RMIX2: u8 = 0x25;
    pub const LOUT2: u8 = 0x28;
    pub const ROUT2: u8 = 0x29;
}

/// Output volume register: update both channels together, change at a
/// zero crossing (no clicks).
const VOL_UPDATE: u16 = 1 << 8;
const VOL_ZC: u16 = 1 << 7;
/// Output volume codes: 0x79 is 0 dB, 1 dB per step, below 0x30 is mute.
const VOL_0DB: u16 = 0x79;
const VOL_MIN: u16 = 0x30;

/// Register 25 (`PWR1`): VMID on its 5k divider (`VMIDSEL` = 11, fast
/// start) and VREF on.
const PWR1_FAST_START: u16 = 0x1C0;
/// Register 25: VMID on its 50k divider (`VMIDSEL` = 01, playback) and VREF
/// on.
const PWR1_PLAYBACK: u16 = 0x0C0;

/// How long VMID charges on its fast-start divider before the set-up
/// moves it to the playback divider. Copied from Dell's driver (it waits
/// 75 ms after register 25 = 0x1C2 before reading the jack; imitation
/// audit); S1 run 2 read the jack right 75 ms after.
// inherited: imitation-audit Audio, Dell wavedev.dll (75 ms VMID charge)
pub const VMID_CHARGE_US: u32 = 75_000;

/// DAC soft mute (`DAC_CTRL` bit 3).
const DAC_MUTE: u16 = 1 << 3;

/// The sample rate the codec expects, with its master clock at 256 fs (the
/// PXA's I2S system clock, `SADIV` picks both).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleRate {
    /// 48 kHz from 12.288 MHz (`SR` = 00000: proven on the X51v, rp-tone).
    Khz48,
    /// 44.1 kHz from 11.2896 MHz (`SR` = 10000; the datasheet's table as the
    /// emulator's WM8750 model carries it; the session rate, ADR-0032 A1).
    Khz44_1,
}

impl SampleRate {
    /// Register 8 (`SRATE`): normal mode, `SR` in bits 5-1.
    fn srate(self) -> u16 {
        match self {
            Self::Khz48 => 0b00000 << 1,
            Self::Khz44_1 => 0b10000 << 1,
        }
    }
}

/// Where sound goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    /// Headphones only (plugged in: the speaker is muted).
    Headphones,
    /// The speaker (and the headphone pair, harmlessly, with nothing in).
    Speaker,
}

/// A line that must be open for the codec to hear the bus.
pub trait Gate {
    /// Opens (`true`) or closes the gate.
    fn set(&mut self, open: bool);
}

/// An I2C bus behind a [`Gate`]: open for each write, closed after it,
/// whether the write succeeded or not (as Windows Mobile does: the lines
/// are shared with another device, so the gate is never left open).
pub struct Gated<I, G> {
    /// The bus.
    pub bus: I,
    /// The gate.
    pub gate: G,
}

impl<I: I2cBus, G: Gate> I2cBus for Gated<I, G> {
    fn write(&mut self, addr: u8, data: &[u8]) -> Result<(), I2cError> {
        self.gate.set(true);
        let r = self.bus.write(addr, data);
        self.gate.set(false);
        r
    }
}

/// The codec, found and set up.
pub struct Wm8750<I> {
    i2c: I,
    addr: u8,
    level: u32,
    output: Output,
    rate: SampleRate,
}

impl<I: I2cBus> Wm8750<I> {
    /// Finds the codec (whichever address answers) and sets it up for
    /// 48 kHz 16-bit I2S playback from a 12.288 MHz master clock (256 fs),
    /// outputs at `level` (0-100). The bus comes back if no codec answers.
    /// `wait_us` waits that many microseconds: the start-up charges VMID
    /// on its fast divider for [`VMID_CHARGE_US`], then moves it to the
    /// playback divider, so the codec comes back complete (no caller can
    /// leave it half started).
    pub fn init(i2c: I, level: u32, wait_us: &mut dyn FnMut(u32)) -> Result<Self, (I2cError, I)> {
        Self::init_at(i2c, level, SampleRate::Khz48, wait_us)
    }

    /// As [`Self::init`], at `rate` (the I2S side must run at the same rate).
    pub fn init_at(
        mut i2c: I,
        level: u32,
        rate: SampleRate,
        wait_us: &mut dyn FnMut(u32),
    ) -> Result<Self, (I2cError, I)> {
        // Found by its first set-up write, the reset (as Windows Mobile
        // starts; the X51v's controller can't do address-only transfers:
        // docs/hw/audio.md).
        let reset = word(reg::RESET, 0);
        let Some(addr) = ADDRESSES
            .into_iter()
            .find(|&a| i2c.write(a, &reset).is_ok())
        else {
            return Err((I2cError::NoDevice, i2c));
        };
        let mut codec = Self {
            i2c,
            addr,
            level: 0,
            output: Output::Speaker,
            rate,
        };
        let setup = [
            // VMID on its 5k divider (fast start) and VREF: the reference
            // charges quickly, as Windows Mobile's driver starts it
            // (register 25 = 0x1C2, then 75 ms, then the jack: S1 run 1
            // showed the jack line low for seconds with the 50k divider).
            // `normal_vmid` moves to 50k once it has charged.
            (reg::PWR1, PWR1_FAST_START),
            // DACs, both output pairs.
            (reg::PWR2, 0x1F8),
            // I2S, 16-bit, slave (the PXA drives the clocks).
            (reg::IFACE, 0x002),
            // The rate, from a 256 fs master clock (normal mode).
            (reg::SRATE, rate.srate()),
            // DAC unmuted.
            (reg::DAC_CTRL, 0x000),
            // Left DAC to the left mixer, right DAC to the right.
            (reg::LMIX1, 0x150),
            (reg::RMIX2, 0x150),
        ];
        for (r, v) in setup {
            if let Err(e) = codec.set(r, v) {
                return Err((e, codec.i2c));
            }
        }
        if let Err(e) = codec.set_level(level) {
            return Err((e, codec.i2c));
        }
        // VMID charged on the fast divider, then the playback one.
        wait_us(VMID_CHARGE_US);
        match codec.set(reg::PWR1, PWR1_PLAYBACK) {
            Ok(()) => Ok(codec),
            Err(e) => Err((e, codec.i2c)),
        }
    }

    /// The address the codec answered on.
    #[must_use]
    pub fn address(&self) -> u8 {
        self.addr
    }

    /// The output level (0-100).
    #[must_use]
    pub fn level(&self) -> u32 {
        self.level
    }

    /// Sets the outputs to `level` (0-100): 0 mutes, 100 is 0 dB, in 1 dB
    /// steps between, which is even to the ear. The speaker pair stays
    /// muted while headphones are the output.
    pub fn set_level(&mut self, level: u32) -> Result<(), I2cError> {
        let level = level.min(100);
        let code = volume_code(level);
        let speaker = if self.output == Output::Headphones {
            0
        } else {
            code
        };
        for (left, right, c) in [
            (reg::LOUT1, reg::ROUT1, code),
            (reg::LOUT2, reg::ROUT2, speaker),
        ] {
            self.set(left, VOL_ZC | c)?;
            self.set(right, VOL_ZC | VOL_UPDATE | c)?;
        }
        self.level = level;
        Ok(())
    }

    /// Routes sound to `output` (headphones plugged in or not), at the
    /// current level.
    pub fn route(&mut self, output: Output) -> Result<(), I2cError> {
        self.output = output;
        self.set_level(self.level)
    }

    /// Where sound goes now.
    #[must_use]
    pub fn output(&self) -> Output {
        self.output
    }

    /// The sample rate it was set up for.
    #[must_use]
    pub fn rate(&self) -> SampleRate {
        self.rate
    }

    /// Powers the codec's insides down before its supply goes off: the DAC
    /// soft-muted, both output pairs to mute, then outputs and DACs off,
    /// then VMID and VREF off (the datasheet's order: outputs before the
    /// reference, so the outputs don't follow VMID down). Gives the bus
    /// back, with the first failed write's error if one failed (every step
    /// is still tried).
    pub fn power_down(mut self) -> (Result<(), I2cError>, I) {
        let mut first = Ok(());
        let mut step = |r: Result<(), I2cError>| {
            if first.is_ok() {
                first = r;
            }
        };
        step(self.set(reg::DAC_CTRL, DAC_MUTE));
        step(self.set_level(0));
        step(self.set(reg::PWR2, 0));
        step(self.set(reg::PWR1, 0));
        (first, self.i2c)
    }

    /// Gives the bus back.
    pub fn into_inner(self) -> I {
        self.i2c
    }

    /// The bus, between writes (for a board line kept in it, such as the
    /// gate's GPIO block).
    pub fn bus_mut(&mut self) -> &mut I {
        &mut self.i2c
    }

    fn set(&mut self, r: u8, v: u16) -> Result<(), I2cError> {
        self.i2c.write(self.addr, &word(r, v))
    }
}

/// A register write's two bytes: the 7-bit register and the 9-bit value's
/// top bit, then its low 8 bits.
fn word(r: u8, v: u16) -> [u8; 2] {
    [(r << 1) | (v >> 8) as u8 & 1, v as u8]
}

/// The output volume code for `level` (0-100, already clamped).
fn volume_code(level: u32) -> u16 {
    if level == 0 {
        0 // below VOL_MIN: muted
    } else {
        VOL_MIN + ((VOL_0DB - VOL_MIN) as u32 * level / 100) as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bus with a codec at `addr` that records its register writes.
    struct Bus {
        addr: u8,
        regs: Vec<(u8, u16)>,
        checks: Vec<u8>,
    }

    impl I2cBus for Bus {
        fn write(&mut self, addr: u8, data: &[u8]) -> Result<(), I2cError> {
            if addr != self.addr {
                return Err(I2cError::NoDevice);
            }
            match data {
                [] => self.checks.push(addr),
                [a, b] => self
                    .regs
                    .push((a >> 1, u16::from(a & 1) << 8 | u16::from(*b))),
                _ => panic!("WM8750 writes are two bytes"),
            }
            Ok(())
        }
    }

    fn bus(addr: u8) -> Bus {
        Bus {
            addr,
            regs: Vec::new(),
            checks: Vec::new(),
        }
    }

    /// The X51v's case (docs/hw/audio.md): the codec answers only while
    /// the gate is open, and the gate is closed after every write, a
    /// refused one included.
    #[test]
    fn a_gated_codec_is_found_and_the_gate_never_stays_open() {
        use std::cell::Cell;
        use std::rc::Rc;
        struct Behind {
            inner: Bus,
            open: Rc<Cell<bool>>,
        }
        impl I2cBus for Behind {
            fn write(&mut self, addr: u8, data: &[u8]) -> Result<(), I2cError> {
                if !self.open.get() {
                    return Err(I2cError::NoDevice);
                }
                self.inner.write(addr, data)
            }
        }
        struct Pin {
            open: Rc<Cell<bool>>,
            opens: u32,
        }
        impl Gate for Pin {
            fn set(&mut self, open: bool) {
                self.opens += u32::from(open);
                self.open.set(open);
            }
        }
        let open = Rc::new(Cell::new(false));
        // Ungated, the codec looks absent: the first device run's result.
        let mut ungated = Behind {
            inner: bus(0x1A),
            open: open.clone(),
        };
        assert_eq!(ungated.write(0x1A, &[]), Err(I2cError::NoDevice));
        let gated = Gated {
            bus: Behind {
                inner: bus(0x1A),
                open: open.clone(),
            },
            gate: Pin {
                open: open.clone(),
                opens: 0,
            },
        };
        let mut codec = Wm8750::init(gated, 60, &mut |_| {}).ok().unwrap();
        assert_eq!(codec.address(), 0x1A);
        assert!(!open.get(), "gate left open after set-up");
        assert!(codec.route(Output::Headphones).is_ok());
        assert!(!open.get(), "gate left open after a route change");
        let g = codec.into_inner();
        assert!(g.gate.opens as usize >= g.bus.inner.regs.len());
        // A write to nobody fails and still closes the gate.
        let mut g = g;
        assert_eq!(g.write(0x55, &[0, 0]), Err(I2cError::NoDevice));
        assert!(!open.get());
    }

    #[test]
    fn finds_the_codec_on_either_address_and_sets_up_playback() {
        for addr in ADDRESSES {
            let codec = Wm8750::init(bus(addr), 100, &mut |_| {}).ok().unwrap();
            assert_eq!(codec.address(), addr);
            let regs = codec.into_inner().regs;
            assert_eq!(regs[0], (reg::RESET, 0), "reset first");
            assert!(regs.contains(&(reg::IFACE, 0x002)));
            assert!(regs.contains(&(reg::PWR2, 0x1F8)));
            assert!(regs.contains(&(reg::LMIX1, 0x150)) && regs.contains(&(reg::RMIX2, 0x150)));
            // 100% is 0 dB on both pairs, the right channel latching both.
            assert!(regs.contains(&(reg::ROUT1, VOL_UPDATE | VOL_ZC | VOL_0DB)));
            assert!(regs.contains(&(reg::LOUT2, VOL_ZC | VOL_0DB)));
        }
    }

    #[test]
    fn no_codec_gives_the_bus_back() {
        let r = Wm8750::init(bus(0x50), 50, &mut |_| {});
        let Err((e, b)) = r else {
            panic!("found a codec that isn't there")
        };
        assert_eq!(e, I2cError::NoDevice);
        assert!(b.regs.is_empty(), "nothing written to the bus");
    }

    #[test]
    fn headphones_mute_the_speaker_and_unplugging_brings_it_back() {
        let mut codec = Wm8750::init(bus(0x1A), 100, &mut |_| {}).ok().unwrap();
        codec.route(Output::Headphones).unwrap();
        let last =
            |regs: &[(u8, u16)], r: u8| regs.iter().rev().find(|w| w.0 == r).unwrap().1 & 0x7F;
        assert_eq!(last(&codec.i2c.regs, reg::LOUT2), 0);
        assert_eq!(last(&codec.i2c.regs, reg::ROUT2), 0);
        assert_eq!(last(&codec.i2c.regs, reg::LOUT1), VOL_0DB);
        codec.route(Output::Speaker).unwrap();
        assert_eq!(last(&codec.i2c.regs, reg::ROUT2), VOL_0DB);
        assert_eq!(codec.output(), Output::Speaker);
    }

    #[test]
    fn vmid_charges_fast_then_the_set_up_moves_it_to_playback() {
        use std::cell::Cell;
        let written = Cell::new(0usize);
        let mut waited_after = None;
        let mut b = bus(0x1A);
        // Count writes through a wrapper so the wait's position is known.
        struct Counting<'a>(Bus, &'a Cell<usize>);
        impl I2cBus for Counting<'_> {
            fn write(&mut self, addr: u8, data: &[u8]) -> Result<(), I2cError> {
                self.1.set(self.1.get() + 1);
                self.0.write(addr, data)
            }
        }
        b.regs.clear();
        let c = Wm8750::init(Counting(b, &written), 50, &mut |us| {
            assert_eq!(us, VMID_CHARGE_US);
            waited_after = Some(written.get());
        })
        .ok()
        .unwrap();
        let regs = c.into_inner().0.regs;
        // Right after the reset: the 5k fast-start divider.
        assert_eq!(regs[1], (reg::PWR1, PWR1_FAST_START));
        // The wait, then the playback divider as the last write.
        assert_eq!(waited_after, Some(regs.len() - 1));
        assert_eq!(regs.last(), Some(&(reg::PWR1, PWR1_PLAYBACK)));
    }

    #[test]
    fn the_rate_goes_to_register_8() {
        let regs = |rate| {
            Wm8750::init_at(bus(0x1A), 50, rate, &mut |_| {})
                .ok()
                .unwrap()
                .into_inner()
                .regs
        };
        assert!(regs(SampleRate::Khz48).contains(&(reg::SRATE, 0x000)));
        // SR = 10000: 44.1 kHz at 256 fs.
        assert!(regs(SampleRate::Khz44_1).contains(&(reg::SRATE, 0x020)));
        let c = Wm8750::init_at(bus(0x1A), 50, SampleRate::Khz44_1, &mut |_| {})
            .ok()
            .unwrap();
        assert_eq!(c.rate(), SampleRate::Khz44_1);
        assert_eq!(
            Wm8750::init(bus(0x1A), 50, &mut |_| {})
                .ok()
                .unwrap()
                .rate(),
            SampleRate::Khz48
        );
    }

    #[test]
    fn power_down_mutes_then_outputs_then_the_reference() {
        let c = Wm8750::init(bus(0x1A), 80, &mut |_| {}).ok().unwrap();
        let before = c.i2c.regs.len();
        let (r, b) = c.power_down();
        assert_eq!(r, Ok(()));
        let w = &b.regs[before..];
        assert_eq!(w[0], (reg::DAC_CTRL, DAC_MUTE));
        // Both pairs muted, the right channels latching.
        assert!(w[1..5].iter().all(|&(_, v)| v & 0x7F == 0));
        assert_eq!(w[5], (reg::PWR2, 0));
        assert_eq!(w[6], (reg::PWR1, 0));
        assert_eq!(w.len(), 7);
    }

    #[test]
    fn power_down_tries_every_step_and_reports_the_first_failure() {
        struct Flaky {
            inner: Bus,
            fail_at: usize,
            n: usize,
        }
        impl I2cBus for Flaky {
            fn write(&mut self, addr: u8, data: &[u8]) -> Result<(), I2cError> {
                self.n += 1;
                if self.n == self.fail_at {
                    return Err(I2cError::Nak);
                }
                self.inner.write(addr, data)
            }
        }
        let c = Wm8750::init(
            Flaky {
                inner: bus(0x1A),
                fail_at: usize::MAX,
                n: 0,
            },
            80,
            &mut |_| {},
        )
        .ok()
        .unwrap();
        let mut c = c;
        c.i2c.fail_at = c.i2c.n + 1; // the mute fails
        let (r, b) = c.power_down();
        assert_eq!(r, Err(I2cError::Nak));
        // The reference still went off after the failed first step.
        assert_eq!(b.inner.regs.last(), Some(&(reg::PWR1, 0)));
    }

    #[test]
    fn levels_are_db_steps_with_zero_muted() {
        assert_eq!(volume_code(0), 0);
        assert_eq!(volume_code(100), VOL_0DB);
        assert_eq!(volume_code(1), VOL_MIN);
        let mut last = 0;
        for l in 1..=100 {
            let c = volume_code(l);
            assert!(c >= last && (VOL_MIN..=VOL_0DB).contains(&c));
            last = c;
        }
        let mut codec = Wm8750::init(bus(0x1A), 30, &mut |_| {}).ok().unwrap();
        codec.set_level(250).unwrap();
        assert_eq!(codec.level(), 100);
    }
}
