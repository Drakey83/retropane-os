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

/// Control byte for a 12-bit differential conversion on `ch`, powering down
/// between conversions so the pen interrupt stays armed (PD = 00).
#[must_use]
pub const fn command(ch: u8) -> u8 {
    0x80 | ((ch & 7) << 4)
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

    /// One 12-bit conversion on `ch`.
    pub fn read(&mut self, ch: u8) -> u16 {
        self.cs.set_low();
        self.spi.transfer(command(ch));
        let hi = self.spi.transfer(0);
        let lo = self.spi.transfer(0);
        self.cs.set_high();
        decode(hi, lo)
    }

    /// A raw sample if the screen is touched.
    pub fn sample(&mut self) -> Option<RawTouch> {
        if !self.pen_down() {
            return None;
        }
        Some(RawTouch {
            x: self.read(channel::X),
            y: self.read(channel::Y),
            z1: self.read(channel::Z1),
            z2: self.read(channel::Z2),
        })
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
        assert_eq!(c.borrow().log.len(), 12);
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
