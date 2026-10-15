//! TI TSC2046 touch screen controller (docs/hw/touch.md, "TSC2046 protocol").
//!
//! Board-independent: it talks through any [`SpiBus`], drives a chip-select
//! [`OutputPin`] and reads the pen interrupt from an [`InputPin`].

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use rp_hal_api::{InputPin, OutputPin, SpiBus};

/// Conversion channels (differential mode).
pub mod channel {
    /// Y position.
    pub const Y: u8 = 1;
    /// Pressure Z1.
    pub const Z1: u8 = 3;
    /// Pressure Z2.
    pub const Z2: u8 = 4;
    /// X position.
    pub const X: u8 = 5;
}

/// Power-down bits (PD1..PD0) of a control byte (TSC2046 datasheet).
pub mod pd {
    /// Powered down between conversions, pen interrupt enabled: the last
    /// conversion of a sample, so the next touch interrupts again.
    pub const IDLE: u8 = 0b00;
    /// Converter on, pen interrupt disabled: the panel's drivers stay
    /// powered between back-to-back conversions, so the plates settle.
    pub const ADC_ON: u8 = 0b01;
}

/// Control byte for a 12-bit differential conversion on `ch`, powering down
/// between conversions so the pen interrupt stays armed (PD = 00).
#[must_use]
pub const fn command(ch: u8) -> u8 {
    command_pd(ch, pd::IDLE)
}

/// Control byte for a 12-bit differential conversion on `ch` with
/// power-down bits `pd`.
#[must_use]
pub const fn command_pd(ch: u8, pd: u8) -> u8 {
    0x80 | ((ch & 7) << 4) | (pd & 3)
}

/// Conversions spent letting a channel settle before it is measured
/// (~10 us each at 2.6 MHz: ~1.3 ms). Measured on the X51v (2026-10-13):
/// a fresh position conversion starts far off (X near 0, Y near 4095) and
/// approaches the pen's value ever more slowly; "stop when two in a row
/// agree" stopped early on the slow tail, so taps drifted upward at
/// random (the owner: buttons above the one touched). A fixed, generous
/// settling time, then an average, doesn't depend on the curve's shape.
pub const SETTLE_CONVERSIONS: u16 = 128;

/// Settling per channel (2026-10-13): Y holds its value after 16
/// conversions; X gets 256, the setting of the build the owner called
/// "much better" (64 with faster sampling was much worse; recordings of
/// real taps will settle the question rather than more guesses).
#[must_use]
pub const fn settle_for(ch: u8) -> u16 {
    match ch {
        channel::X => 2 * SETTLE_CONVERSIONS,
        channel::Y => SETTLE_CONVERSIONS / 4,
        _ => SETTLE_CONVERSIONS / 8,
    }
}

/// Conversions averaged once settled.
pub const AVERAGE: u16 = 8;

/// Where on its settling curve [`Settle::curve`] records a channel: after
/// this many conversions.
pub const CURVE_AT: [u16; 6] = [1, 4, 16, 32, 64, 128];

/// Shorter settling times [`Settle::early`] measures a channel at: the
/// value a sample would keep had it settled only this many conversions
/// (the next [`AVERAGE`] conversions averaged), for deciding on recorded
/// taps how short a sample can be (docs/hw/touch.md). Free: they are
/// conversions the full settling makes anyway.
pub const EARLY_AT: [u16; 4] = [16, 32, 64, 128];

/// One channel's settling, for diagnostics and calibration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Settle {
    /// The conversions at [`CURVE_AT`], to see the curve on the device.
    pub curve: [u16; 6],
    /// The value kept: the average of [`AVERAGE`] conversions after
    /// [`settle_for`] the channel.
    pub value: u16,
    /// The value had the channel settled only [`EARLY_AT`] conversions;
    /// 0 where that average doesn't fit inside its settling.
    pub early: [u16; 4],
}

/// Both position channels' settling in the last sample.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Trace {
    /// The X channel.
    pub x: Settle,
    /// The Y channel.
    pub y: Settle,
}

/// Decodes the two result bytes of a 12-bit conversion.
#[must_use]
pub const fn decode(hi: u8, lo: u8) -> u16 {
    ((((hi as u16) << 8) | lo as u16) >> 3) & 0xFFF
}

/// One raw sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawTouch {
    /// X channel, 12-bit.
    pub x: u16,
    /// Y channel, 12-bit.
    pub y: u16,
    /// Pressure Z1, 12-bit.
    pub z1: u16,
    /// Pressure Z2, 12-bit.
    pub z2: u16,
}

/// The controller.
pub struct Tsc2046<S, C, P> {
    spi: S,
    cs: C,
    pen: P,
}

impl<S: SpiBus, C: OutputPin, P: InputPin> Tsc2046<S, C, P> {
    /// Wires up the controller. Deselects it.
    pub fn new(spi: S, mut cs: C, pen: P) -> Self {
        cs.set_high();
        Self { spi, cs, pen }
    }

    /// Whether the screen is being touched (pen interrupt is active low).
    pub fn pen_down(&mut self) -> bool {
        !self.pen.is_high()
    }

    /// Converts `ch` with the converter on for [`settle_for`] it, then
    /// averages [`AVERAGE`] more.
    pub fn settle(&mut self, ch: u8) -> Settle {
        let mut curve = [0; 6];
        let mut at = 0;
        let mut early = [0u32; 4];
        let settle = settle_for(ch);
        for n in 1..=settle {
            let v = self.read_pd(ch, pd::ADC_ON);
            if CURVE_AT.get(at) == Some(&n) {
                curve[at] = v;
                at += 1;
            }
            for (sum, &k) in early.iter_mut().zip(&EARLY_AT) {
                if n > k && n <= k + AVERAGE && k + AVERAGE <= settle {
                    *sum += u32::from(v);
                }
            }
        }
        let sum: u32 = (0..AVERAGE)
            .map(|_| u32::from(self.read_pd(ch, pd::ADC_ON)))
            .sum();
        let n = u32::from(AVERAGE);
        let avg = |sum: u32| ((sum + n / 2) / n) as u16;
        Settle {
            curve,
            value: avg(sum),
            early: early.map(avg),
        }
    }

    /// One 12-bit conversion on `ch`, powered down after it.
    pub fn read(&mut self, ch: u8) -> u16 {
        self.read_pd(ch, pd::IDLE)
    }

    /// One 12-bit conversion on `ch` with power-down bits `pd`.
    pub fn read_pd(&mut self, ch: u8, pd: u8) -> u16 {
        self.cs.set_low();
        self.spi.transfer(command_pd(ch, pd));
        let hi = self.spi.transfer(0);
        let lo = self.spi.transfer(0);
        self.cs.set_high();
        decode(hi, lo)
    }

    /// A raw sample if the screen is touched. Samples with no measured
    /// pressure (Z1 = 0) are discarded: the pen interrupt can briefly
    /// disagree with the panel while the stylus lands or lifts.
    pub fn sample(&mut self) -> Option<RawTouch> {
        self.sample_traced().map(|(t, _)| t)
    }

    /// As [`Self::sample`], with how the position channels settled (see
    /// [`Self::settle`]); the last conversion powers down, re-arming the
    /// pen interrupt.
    pub fn sample_traced(&mut self) -> Option<(RawTouch, Trace)> {
        if !self.pen_down() {
            return None;
        }
        let trace = Trace {
            x: self.settle(channel::X),
            y: self.settle(channel::Y),
        };
        let z1 = self.settle(channel::Z1).value;
        let t = RawTouch {
            x: trace.x.value,
            y: trace.y.value,
            z1,
            z2: self.read_pd(channel::Z2, pd::IDLE),
        };
        (t.z1 != 0).then_some((t, trace))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn command_bytes() {
        assert_eq!(command(channel::X), 0xD0);
        assert_eq!(command(channel::Y), 0x90);
        assert_eq!(command(channel::Z1), 0xB0);
        assert_eq!(command(channel::Z2), 0xC0);
    }

    #[test]
    fn decode_follows_datasheet_bit_positions() {
        // Value 0xABC: byte1 = 0 busy + D11..D5, byte2 = D4..D0 + 000.
        let v: u16 = 0xABC;
        let wire = v << 3;
        assert_eq!(decode((wire >> 8) as u8, wire as u8), 0xABC);
        assert_eq!(decode(0xFF, 0xFF), 0xFFF);
    }

    /// A fake TSC2046 following the datasheet's byte timing, plus a log.
    #[derive(Default)]
    struct Chip {
        values: [u16; 8],
        /// Served in order instead of `values` while any remain.
        ramp: Vec<u16>,
        selected: bool,
        phase: u8,
        result: u16,
        log: Vec<String>,
    }

    struct Spi(Rc<RefCell<Chip>>);
    impl SpiBus for Spi {
        fn transfer(&mut self, b: u8) -> u8 {
            let mut c = self.0.borrow_mut();
            assert!(c.selected, "transfer while deselected");
            c.log.push(format!("tx {b:02x}"));
            match c.phase {
                0 => {
                    let v = if c.ramp.is_empty() {
                        c.values[usize::from((b >> 4) & 7)]
                    } else {
                        c.ramp.remove(0)
                    };
                    c.result = v << 3;
                    c.phase = 1;
                    0
                }
                1 => {
                    c.phase = 2;
                    (c.result >> 8) as u8
                }
                _ => {
                    c.phase = 0;
                    c.result as u8
                }
            }
        }
    }
    struct Cs(Rc<RefCell<Chip>>);
    impl OutputPin for Cs {
        fn set_high(&mut self) {
            self.0.borrow_mut().selected = false;
        }
        fn set_low(&mut self) {
            self.0.borrow_mut().selected = true;
        }
    }
    struct Pen(bool);
    impl InputPin for Pen {
        fn is_high(&mut self) -> bool {
            self.0
        }
    }

    fn chip(values: [u16; 8]) -> Rc<RefCell<Chip>> {
        Rc::new(RefCell::new(Chip {
            values,
            ..Chip::default()
        }))
    }

    #[test]
    fn reads_a_full_sample_when_touched() {
        let c = chip([0, 1111, 0, 600, 2000, 3333, 0, 0]);
        let mut ts = Tsc2046::new(Spi(c.clone()), Cs(c.clone()), Pen(false));
        let s = ts.sample().unwrap();
        assert_eq!(
            s,
            RawTouch {
                x: 3333,
                y: 1111,
                z1: 600,
                z2: 2000
            }
        );
        assert!(!c.borrow().selected);
        // X, Y and Z1 settled and averaged, then Z2: three bytes each.
        let per = |ch| usize::from(settle_for(ch) + AVERAGE);
        let (x, y, z1) = (per(channel::X), per(channel::Y), per(channel::Z1));
        let log = &c.borrow().log;
        assert_eq!(log.len(), (x + y + z1 + 1) * 3);
        // The converter stays on until the last conversion, which powers
        // down so the pen interrupt re-arms.
        let commands: Vec<&String> = log.iter().step_by(3).collect();
        assert_eq!(commands[0], "tx d1");
        assert_eq!(commands[x], "tx 91");
        assert_eq!(commands[x + y], "tx b1");
        assert_eq!(commands[x + y + z1], "tx c0");
    }

    #[test]
    fn a_channel_settles_for_a_fixed_time_then_averages() {
        // A slow approach whose steps get small long before it arrives (the
        // X51v's tail): stopping at "two in a row agree" would keep ~1500.
        let c = chip([0; 8]);
        let mut ramp: Vec<u16> = Vec::new();
        let mut v = 100.0f64;
        for _ in 0..settle_for(channel::X) {
            v += (2000.0 - v) * 0.05;
            ramp.push(v as u16);
        }
        ramp.extend([1998, 2002, 2000, 2001, 1999, 2000, 2003, 1997]);
        c.borrow_mut().ramp = ramp.clone();
        let mut ts = Tsc2046::new(Spi(c.clone()), Cs(c.clone()), Pen(false));
        let s = ts.settle(channel::X);
        assert_eq!(s.value, 2000);
        assert_eq!(s.curve[0], 195);
        assert!(s.curve[1] < s.curve[2] && s.curve[4] < s.curve[5]);
        // Settled for less, the value would have been further off.
        let early_x = |k: usize| {
            let at = usize::from(EARLY_AT[k]);
            let sum: u32 = ramp[at..at + 8].iter().map(|&v| u32::from(v)).sum();
            ((sum + 4) / 8) as u16
        };
        assert_eq!(s.early, [early_x(0), early_x(1), early_x(2), early_x(3)]);
        assert!(s.early[0] < s.early[1] && s.early[3] < s.value);
    }

    #[test]
    fn early_values_only_inside_the_settling() {
        let c = chip([0, 1111, 0, 600, 2000, 3333, 0, 0]);
        let mut ts = Tsc2046::new(Spi(c.clone()), Cs(c.clone()), Pen(false));
        assert_eq!(ts.settle(channel::X).early, [3333; 4]);
        // Y settles 32: only the 16-conversion value fits.
        assert_eq!(ts.settle(channel::Y).early, [1111, 0, 0, 0]);
    }

    #[test]
    fn command_bytes_with_power_down_bits() {
        assert_eq!(command_pd(channel::X, pd::ADC_ON), 0xD1);
        assert_eq!(command_pd(channel::Z2, pd::IDLE), 0xC0);
        assert_eq!(command_pd(channel::Y, 7), 0x93); // masked to two bits
    }

    #[test]
    fn zero_pressure_samples_are_discarded() {
        let c = chip([0, 1111, 0, 0, 2000, 3333, 0, 0]); // Z1 = 0
        let mut ts = Tsc2046::new(Spi(c.clone()), Cs(c.clone()), Pen(false));
        assert!(ts.sample().is_none());
    }

    #[test]
    fn no_sample_without_pen() {
        let c = chip([0; 8]);
        let mut ts = Tsc2046::new(Spi(c.clone()), Cs(c.clone()), Pen(true));
        assert!(ts.sample().is_none());
        assert!(c.borrow().log.is_empty());
    }
}
