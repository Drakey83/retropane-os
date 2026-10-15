//! The X51v's core voltage (ADR-0031): three GPIO outputs pick the core
//! regulator's level, not a power-management IC on the power I2C. GPIO3 and
//! GPIO4 are the PXA270's power-I2C pins, used here as plain GPIOs (function
//! 0); GPIO38 is the third. Found statically in Windows Mobile's OAL
//! (`docs/hw/clocks.md`, 2026-10-15); every value below
//! is a **finding to work** (docs/research/imitation-audit.md): the real
//! rail voltage of each code is unmeasured, and the codes per frequency are
//! what Dell validated, not a proven minimum.

use crate::Confidence;

/// The pins, least significant code bit first: GPIO3, GPIO4, GPIO38.
pub const PINS: [u8; 3] = [3, 4, 38];

/// How sure we are of the pins and codes (static read of the OAL; the
/// probe's levels under WM at 520 MHz, code 0b110, agree).
pub const CONFIDENCE: Confidence = Confidence::Likely;

/// A voltage code: bit 0 GPIO3, bit 1 GPIO4, bit 2 GPIO38.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Code(pub u8);

impl Code {
    /// The code read from the three pins' levels.
    #[must_use]
    pub const fn from_levels(g3: bool, g4: bool, g38: bool) -> Self {
        Self((g3 as u8) | (g4 as u8) << 1 | (g38 as u8) << 2)
    }

    /// The level for each pin in [`PINS`] order.
    #[must_use]
    pub const fn levels(self) -> [bool; 3] {
        [self.0 & 1 != 0, self.0 & 2 != 0, self.0 & 4 != 0]
    }

    /// The millivolts Dell's table labels this code with (never measured
    /// here; two labels share some codes). `None` for codes Dell never uses.
    #[must_use]
    pub const fn dell_label_mv(self) -> Option<u32> {
        match self.0 {
            0b000 => Some(1_050),
            0b100 => Some(1_163),
            0b101 => Some(1_266),
            0b110 => Some(1_364),
            0b111 => Some(1_574),
            _ => None,
        }
    }

    /// The lowest Dell-validated code for a core PLL (turbo) frequency in
    /// kHz: 104 and below 0b000, 208 0b100, 312 0b101, 416 and 520 0b110,
    /// above 0b111. RetroPane runs no point Dell didn't validate at a code
    /// lower than Dell's until measured otherwise (ADR-0031).
    #[must_use]
    pub const fn for_turbo_khz(khz: u32) -> Self {
        Self(if khz <= 104_000 {
            0b000
        } else if khz <= 208_000 {
            0b100
        } else if khz <= 312_000 {
            0b101
        } else if khz <= 520_000 {
            0b110
        } else {
            0b111
        })
    }

    /// Whether `self` is at least as high a level as `other` (codes Dell
    /// uses are ordered by their labels; unknown codes never compare high).
    #[must_use]
    pub const fn covers(self, other: Self) -> bool {
        match (self.dell_label_mv(), other.dell_label_mv()) {
            (Some(a), Some(b)) => a >= b,
            _ => false,
        }
    }
}

/// Wait after raising the code before the faster clock (Dell waits ~2.46
/// ms; the regulator's real slew is unmeasured: a finding to work).
pub const RAISE_SETTLE_US: u32 = 2_500;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_mobiles_520_levels_read_as_its_code() {
        // Probe under WM at 520 MHz: GPLR0 0x05764DF6 (GPIO3 low, GPIO4
        // high), GPLR1 0x01FF4C42 (GPIO38 high).
        let c = Code::from_levels(
            0x0576_4DF6 & 1 << 3 != 0,
            0x0576_4DF6 & 1 << 4 != 0,
            0x01FF_4C42 & 1 << 6 != 0,
        );
        assert_eq!(c, Code(0b110));
        assert_eq!(c, Code::for_turbo_khz(520_000));
        assert_eq!(c.levels(), [false, true, true]);
    }

    #[test]
    fn codes_rise_with_frequency() {
        let mut last = Code(0);
        for khz in [
            104_000, 156_000, 208_000, 312_000, 416_000, 520_000, 624_000,
        ] {
            let c = Code::for_turbo_khz(khz);
            assert!(c.covers(last), "{khz}");
            last = c;
        }
        assert_eq!(last, Code(0b111));
        assert!(!Code(0b110).covers(Code(0b111)));
        assert!(!Code(0b011).covers(Code(0)));
    }
}
