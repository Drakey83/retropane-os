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
}

impl<I: I2cBus> Wm8750<I> {
    /// Finds the codec (whichever address answers) and sets it up for
    /// 48 kHz 16-bit I2S playback from a 12.288 MHz master clock (256 fs),
    /// outputs at `level` (0-100). The bus comes back if no codec answers.
    pub fn init(mut i2c: I, level: u32) -> Result<Self, (I2cError, I)> {
        let Some(addr) = ADDRESSES.into_iter().find(|&a| i2c.write(a, &[]).is_ok()) else {
            return Err((I2cError::NoDevice, i2c));
        };
        let mut codec = Self {
            i2c,
            addr,
            level: 0,
            output: Output::Speaker,
        };
        let setup = [
            (reg::RESET, 0),
            // VMID 50k divider (playback), VREF on.
            (reg::PWR1, 0x0C0),
            // DACs, both output pairs.
            (reg::PWR2, 0x1F8),
            // I2S, 16-bit, slave (the PXA drives the clocks).
            (reg::IFACE, 0x002),
            // 48 kHz from 12.288 MHz (normal mode, 256 fs).
            (reg::SRATE, 0x000),
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
        match codec.set_level(level) {
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
        self.i2c
            .write(self.addr, &[(r << 1) | (v >> 8) as u8 & 1, v as u8])
    }
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
        let mut codec = Wm8750::init(gated, 60).ok().unwrap();
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
            let codec = Wm8750::init(bus(addr), 100).ok().unwrap();
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
        let r = Wm8750::init(bus(0x50), 50);
        let Err((e, b)) = r else {
            panic!("found a codec that isn't there")
        };
        assert_eq!(e, I2cError::NoDevice);
        assert!(b.regs.is_empty(), "nothing written to the bus");
    }

    #[test]
    fn headphones_mute_the_speaker_and_unplugging_brings_it_back() {
        let mut codec = Wm8750::init(bus(0x1A), 100).ok().unwrap();
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
        let mut codec = Wm8750::init(bus(0x1A), 30).ok().unwrap();
        codec.set_level(250).unwrap();
        assert_eq!(codec.level(), 100);
    }
}
