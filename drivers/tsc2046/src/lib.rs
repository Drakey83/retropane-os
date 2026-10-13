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

/// Conversions per channel in a sample. Measured on the X51v (2026-10-13):
/// single conversions with the panel powered down between them read the X
/// plate near 0 (104-265 wherever the pen was); the drivers need time to
/// settle, as AxDroid's 100-150 us settle times said. Repeating with the
/// converter on and keeping the median of the later readings gives them
/// that time.
pub const SETTLE_READS: usize = 4;

/// The median of the last three of `reads` (the first is the unsettled one).
#[must_use]
pub fn settled(reads: [u16; SETTLE_READS]) -> u16 {
    let mut last = [reads[1], reads[2], reads[3]];
    last.sort_unstable();
    last[1]
}

/// Every conversion of the last sample, for diagnostics and calibration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Trace {
    /// The X channel's readings, in order.
    pub x: [u16; SETTLE_READS],
    /// The Y channel's readings, in order.
    pub y: [u16; SETTLE_READS],
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
    /// Default for the X51v's 480 × 640 panel: axes swapped, typical raw range.
    /// A user calibration screen will refine this later.
    pub const X51V: Calibration = Calibration {
        raw_min: (200, 200),
        raw_max: (3900, 3900),
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

    /// As [`Self::sample`], with every position conversion it made: each
    /// channel [`SETTLE_READS`] times with the converter on, the median of
    /// the later ones kept; the last conversion powers down, re-arming the
    /// pen interrupt.
    pub fn sample_traced(&mut self) -> Option<(RawTouch, Trace)> {
        if !self.pen_down() {
            return None;
        }
        let mut trace = Trace::default();
        for r in &mut trace.x {
            *r = self.read_pd(channel::X, pd::ADC_ON);
        }
        for r in &mut trace.y {
            *r = self.read_pd(channel::Y, pd::ADC_ON);
        }
        let z1 = self.read_pd(channel::Z1, pd::ADC_ON);
        let t = RawTouch {
            x: settled(trace.x),
            y: settled(trace.y),
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
                    c.result = c.values[usize::from((b >> 4) & 7)] << 3;
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
        // Four X, four Y, Z1, Z2: ten conversions of three bytes.
        let log = &c.borrow().log;
        assert_eq!(log.len(), 30);
        // The converter stays on until the last conversion, which powers
        // down so the pen interrupt re-arms.
        let commands: Vec<&String> = log.iter().step_by(3).collect();
        assert_eq!(commands[0], "tx d1");
        assert_eq!(commands[4], "tx 91");
        assert_eq!(commands[8], "tx b1");
        assert_eq!(commands[9], "tx c0");
    }

    #[test]
    fn the_settled_value_ignores_the_first_reading() {
        // The X51v's X plate, unsettled then settled (2026-10-13 style).
        assert_eq!(settled([150, 1800, 1840, 1820]), 1820);
        assert_eq!(settled([4012, 2000, 2004, 1990]), 2000);
        // One glitch among the later ones is outvoted.
        assert_eq!(settled([0, 2000, 4095, 2010]), 2010);
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
        // Swapped: horizontal comes from Y, vertical from X.
        let t = RawTouch {
            x: 3900,
            y: 200,
            z1: 0,
            z2: 0,
        };
        assert_eq!(cal.to_screen(t), (0, 639));
        let mid = RawTouch {
            x: 2050,
            y: 2050,
            z1: 0,
            z2: 0,
        };
        assert_eq!(cal.to_screen(mid), (240, 320));
        let out = RawTouch {
            x: 0,
            y: 4095,
            z1: 0,
            z2: 0,
        };
        assert_eq!(cal.to_screen(out), (479, 0));
    }
}
