//! The refresh-rate test's arithmetic (display calibration step C3,
//! docs/design/display-calibration.md), host-testable: the 2700G's display
//! PLL and the frame rate it gives the X51v's panel.
//!
//! The display PLL (`DISPPLL`, `0x03C`) makes `13 MHz x M / (N x 2^P)`
//! (M bits 12-7, N 6-4, P 3-1, bit 0 enable; mbxfb's field layout, checked
//! against Windows Mobile's `0xEC5` = M 29, N 4, P 2 = 23.5625 MHz, Dell's
//! pixel clock, docs/hw/2700g.md S26c). The X51v's frame is 648 x 645
//! clocks (480 + 88 + 64 + 16 by 640 + 1 + 2 + 1 lines plus the line the
//! counter shows as 644: 645 lines measured, docs/hw/emulator.md), so
//! 23.5625 MHz gives 56.4 Hz. The Sharp panel's pixel clock is 24.0-26.3
//! MHz, 25.2 typical (datasheet S, docs/research/screen-swap.md): Dell's is
//! under the range. M 31 gives 25.1875 MHz, the typical, and 60.3 Hz.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

/// Windows Mobile's display PLL on the X51v: M 29, N 4, P 2, enabled.
pub const WM_DISPPLL: u32 = 0xEC5;
/// The test's: M 31, same N and P.
pub const FAST_DISPPLL: u32 = pll(31, 4, 2);
/// Clocks a line.
pub const LINE_CLOCKS: u32 = 648;
/// Lines a frame (measured).
pub const FRAME_LINES: u32 = 645;
/// The panel's pixel clock range, kHz (datasheet S: 38-41.7 ns).
pub const PANEL_KHZ: (u32, u32) = (23_980, 26_316);

/// A `DISPPLL` value from its fields, enabled.
#[must_use]
pub const fn pll(m: u32, n: u32, p: u32) -> u32 {
    (m & 0x3F) << 7 | (n & 7) << 4 | (p & 7) << 1 | 1
}

/// The clock a `DISPPLL` value makes, kHz (`None` if N is 0 or it's off).
#[must_use]
pub const fn pll_khz(v: u32) -> Option<u32> {
    let (m, n, p) = (v >> 7 & 0x3F, v >> 4 & 7, v >> 1 & 7);
    if n == 0 || v & 1 == 0 {
        None
    } else {
        Some(13_000 * m / (n << p))
    }
}

/// The frame rate a pixel clock gives, in hundredths of a hertz.
#[must_use]
pub const fn centi_hz(khz: u32) -> u32 {
    (khz as u64 * 100_000 / (LINE_CLOCKS * FRAME_LINES) as u64) as u32
}

/// Frames counted over `ms` milliseconds, in hundredths of a hertz.
#[must_use]
pub const fn measured_centi_hz(frames: u32, ms: u32) -> u32 {
    if ms == 0 {
        0
    } else {
        (frames as u64 * 100_000 / ms as u64) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_mobiles_pll_is_dells_pixel_clock_and_56_hz() {
        assert_eq!(pll(29, 4, 2), WM_DISPPLL);
        assert_eq!(pll_khz(WM_DISPPLL), Some(23_562));
        assert_eq!(centi_hz(23_562), 5637);
        assert!(
            23_562 < PANEL_KHZ.0,
            "Dell's clock is under the panel's range"
        );
    }

    #[test]
    fn the_fast_pll_is_the_panels_typical_clock_and_60_hz() {
        let khz = pll_khz(FAST_DISPPLL).unwrap();
        assert_eq!(khz, 25_187);
        assert!((PANEL_KHZ.0..=PANEL_KHZ.1).contains(&khz));
        let hz = centi_hz(khz);
        assert!((6000..6050).contains(&hz), "{hz}");
        // Only M changes.
        assert_eq!(FAST_DISPPLL & !(0x3F << 7), WM_DISPPLL & !(0x3F << 7));
    }

    #[test]
    fn counted_frames_become_hertz() {
        assert_eq!(measured_centi_hz(113, 2000), 5650);
        assert_eq!(measured_centi_hz(0, 0), 0);
        assert_eq!(pll_khz(0), None);
    }
}
