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

/// The most conversions spent waiting for a channel to settle (~10 us
/// each at 2.6 MHz). Measured on the X51v (2026-10-13): with the panel
/// powered down between conversions the X plate read near 0 wherever the
/// pen was; with the converter kept on, four conversions still climbed
/// steadily (66, 412, 589, 669): the lines settle over hundreds of
/// microseconds (filter capacitors, as AxDroid's 100-150 us hinted). So a
/// channel is read until two conversions in a row agree, not a fixed count.
pub const MAX_READS: u16 = 64;

/// Two conversions this close (of 4096) count as settled. Tight: the
/// lines approach their value geometrically (about 30% of the gap a
/// conversion), so a step of `d` still leaves about `2.3 d` to go; at 8
/// that was several pixels off (caught by the emulator's settling model).
/// A noisier panel just reads up to [`MAX_READS`], settled by then.
pub const SETTLED_WITHIN: u16 = 2;

/// One channel's settling, for diagnostics and calibration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Settle {
    /// The first conversion (unsettled).
    pub first: u16,
    /// The value kept.
    pub value: u16,
    /// Conversions it took ([`MAX_READS`] if it never settled).
    pub reads: u16,
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

/// Maps raw samples to screen pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Calibration {
    /// Raw value at the left/top edge of each axis (after any swap).
    pub raw_min: (u16, u16),
    /// Raw value at the right/bottom edge of each axis (after any swap).
    pub raw_max: (u16, u16),
    /// Screen size in pixels.
    pub screen: (u16, u16),
    /// The X channel follows the screen's vertical axis (and Y horizontal).
    pub swap_xy: bool,
}

impl Calibration {
    /// The owner's X51v, measured 2026-10-13 (five taps: the corners and
    /// the centre, docs/hw/touch.md): the X channel follows the screen's
    /// vertical axis (99 at the top, 2741 at the bottom), the Y channel the
    /// horizontal one, mirrored (3719 at the left, 319 at the right); the
    /// centre tap fell where a straight line puts it. A user calibration
    /// screen (item 76) will refine it per device.
    pub const X51V: Calibration = Calibration {
        raw_min: (3719, 99),
        raw_max: (319, 2741),
        screen: (480, 640),
        swap_xy: true,
    };

    /// Screen coordinates for a raw sample, clamped to the screen.
    #[must_use]
    pub fn to_screen(&self, t: RawTouch) -> (u16, u16) {
        let (h, v) = if self.swap_xy { (t.y, t.x) } else { (t.x, t.y) };
        let map = |raw: u16, min: u16, max: u16, size: u16| -> u16 {
            let span = i32::from(max) - i32::from(min);
            if span == 0 || size == 0 {
                return 0;
            }
            let pos = (i32::from(raw) - i32::from(min)) * i32::from(size) / span;
            pos.clamp(0, i32::from(size) - 1) as u16
        };
        (
            map(h, self.raw_min.0, self.raw_max.0, self.screen.0),
            map(v, self.raw_min.1, self.raw_max.1, self.screen.1),
        )
    }
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

    /// Converts `ch` with the converter on until two conversions in a row
    /// agree (or [`MAX_READS`]), keeping the later one.
    pub fn settle(&mut self, ch: u8) -> Settle {
        let first = self.read_pd(ch, pd::ADC_ON);
        let mut prev = first;
        for reads in 2..=MAX_READS {
            let v = self.read_pd(ch, pd::ADC_ON);
            if v.abs_diff(prev) <= SETTLED_WITHIN {
                return Settle {
                    first,
                    value: v,
                    reads,
                };
            }
            prev = v;
        }
        Settle {
            first,
            value: prev,
            reads: MAX_READS,
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

    /// As [`Self::sample`], with how the position channels settled: each
    /// channel is converted with the converter on until two conversions in
    /// a row agree within [`SETTLED_WITHIN`] (at most [`MAX_READS`]); the
    /// last conversion powers down, re-arming the pen interrupt.
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
        // A steady chip: X, Y and Z1 settle in two conversions each, then
        // Z2: seven conversions of three bytes.
        let log = &c.borrow().log;
        assert_eq!(log.len(), 21);
        // The converter stays on until the last conversion, which powers
        // down so the pen interrupt re-arms.
        let commands: Vec<&String> = log.iter().step_by(3).collect();
        assert_eq!(commands[0], "tx d1");
        assert_eq!(commands[2], "tx 91");
        assert_eq!(commands[4], "tx b1");
        assert_eq!(commands[6], "tx c0");
    }

    #[test]
    fn a_channel_is_read_until_it_settles() {
        // The X51v's X plate climbing as measured, then steady.
        let c = chip([0; 8]);
        c.borrow_mut().ramp = vec![66, 412, 589, 669, 702, 708, 709];
        let mut ts = Tsc2046::new(Spi(c.clone()), Cs(c.clone()), Pen(false));
        let s = ts.settle(channel::X);
        assert_eq!(
            s,
            Settle {
                first: 66,
                value: 709,
                reads: 7
            }
        );
        // Never settling: gives up at the bound with the last value.
        let c = chip([0; 8]);
        c.borrow_mut().ramp = (0..200u16).map(|i| (i * 40) % 4096).collect();
        let mut ts = Tsc2046::new(Spi(c.clone()), Cs(c.clone()), Pen(false));
        assert_eq!(ts.settle(channel::X).reads, MAX_READS);
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

    #[test]
    fn calibration_swaps_and_scales() {
        let cal = Calibration::X51V;
        let at = |x, y| cal.to_screen(RawTouch { x, y, z1: 0, z2: 0 });
        // Swapped and mirrored: horizontal from Y (high at the left),
        // vertical from X (low at the top).
        assert_eq!(at(99, 3719), (0, 0));
        assert_eq!(at(2741, 319), (479, 639));
        // The owner's taps (2026-10-13): the centre, and Start.
        let (cx, cy) = at(1423, 2017);
        assert!(cx.abs_diff(240) < 12 && cy.abs_diff(320) < 12, "{cx} {cy}");
        let (sx, sy) = at(209, 3508);
        assert!(sx < 96 && sy < 36, "Start at {sx} {sy}");
        // Off the panel's range: clamped to the screen.
        assert_eq!(at(0, 4095), (0, 0));
        assert_eq!(at(4095, 0), (479, 639));
    }
}
