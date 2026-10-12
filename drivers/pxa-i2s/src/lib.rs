//! PXA27x I2S controller (PXA27x Developer's Manual, I2S chapter),
//! playback: the PXA as clock master (bit clock and the codec's 256 fs
//! system clock), 16-bit stereo frames through the transmit FIFO, polled.
//! DMA for streams comes with the audio service; a polled FIFO is enough
//! for tones and tests.
//!
//! Also [`Tone`]: a sine from a phase accumulator and a quarter-wave table,
//! no floating point, exact to 1/65536 Hz.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use rp_hal_api::RegisterBus;

mod reg {
    pub const SACR0: usize = 0x00;
    pub const SACR1: usize = 0x04;
    pub const SASR0: usize = 0x0C;
    pub const SADIV: usize = 0x60;
    pub const SADR: usize = 0x80;
}

const SACR0_ENB: u32 = 1 << 0;
/// Bit clock driven by the PXA.
const SACR0_BCKD: u32 = 1 << 2;
const SACR0_RST: u32 = 1 << 3;
/// Transmit FIFO threshold (for DMA requests): half of 16.
const SACR0_TFTH_8: u32 = 8 << 8;
/// No recording.
const SACR1_DREC: u32 = 1 << 3;
/// Transmit FIFO not full.
const SASR0_TNF: u32 = 1 << 0;

/// Sample rates (the divider for each, PXA27x manual's table).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rate {
    /// 48 kHz.
    Khz48,
    /// 44.1 kHz.
    Khz44_1,
}

impl Rate {
    /// Frames per second.
    #[must_use]
    pub fn hz(self) -> u32 {
        match self {
            Self::Khz48 => 48_000,
            Self::Khz44_1 => 44_100,
        }
    }

    fn sadiv(self) -> u32 {
        match self {
            Self::Khz48 => 0x0C,
            Self::Khz44_1 => 0x0D,
        }
    }
}

/// The controller.
pub struct PxaI2s<B> {
    regs: B,
}

impl<B: RegisterBus> PxaI2s<B> {
    /// Takes the controller's registers (its clock enabled), reset and off.
    pub fn new(mut regs: B) -> Self {
        regs.write32(reg::SACR0, SACR0_RST);
        regs.write32(reg::SACR0, 0);
        Self { regs }
    }

    /// Starts playback at `rate`: clocks out, FIFO draining.
    pub fn start(&mut self, rate: Rate) {
        self.regs.write32(reg::SADIV, rate.sadiv());
        self.regs.write32(reg::SACR1, SACR1_DREC);
        self.regs
            .write32(reg::SACR0, SACR0_ENB | SACR0_BCKD | SACR0_TFTH_8);
    }

    /// Queues one stereo frame (left in the low half) if the FIFO has room.
    pub fn try_write(&mut self, frame: u32) -> bool {
        if self.regs.read32(reg::SASR0) & SASR0_TNF == 0 {
            return false;
        }
        self.regs.write32(reg::SADR, frame);
        true
    }

    /// Stops playback (clocks off).
    pub fn stop(&mut self) {
        self.regs.write32(reg::SACR0, 0);
    }
}

/// A quarter of a sine, 0..=pi/2 in 64 steps, full scale.
const QUARTER: [i16; 65] = [
    0, 804, 1608, 2410, 3212, 4011, 4808, 5602, 6393, 7179, 7962, 8739, 9512, 10278, 11039, 11793,
    12539, 13279, 14010, 14732, 15446, 16151, 16846, 17530, 18204, 18868, 19519, 20159, 20787,
    21403, 22005, 22594, 23170, 23731, 24279, 24811, 25329, 25832, 26319, 26790, 27245, 27683,
    28105, 28510, 28898, 29268, 29621, 29956, 30273, 30571, 30852, 31113, 31356, 31580, 31785,
    31971, 32137, 32285, 32412, 32521, 32609, 32678, 32728, 32757, 32767,
];

/// The sine of `phase` (a full turn is 2^32), full scale, from the table
/// with linear interpolation.
fn sine(phase: u32) -> i32 {
    let quadrant = phase >> 30;
    let within = phase & 0x3FFF_FFFF; // 2^30 per quadrant
    let x = if quadrant & 1 == 0 {
        within
    } else {
        0x3FFF_FFFF - within
    };
    let i = (x >> 24) as usize; // 64 steps
    let frac = ((x >> 8) & 0xFFFF) as i32;
    let a = i32::from(QUARTER[i]);
    let b = i32::from(QUARTER[i + 1]);
    let v = a + (((b - a) * frac) >> 16);
    if quadrant < 2 { v } else { -v }
}

/// A sine tone as stereo frames.
pub struct Tone {
    phase: u32,
    step: u32,
    /// Amplitude, 0..=32767.
    amplitude: i32,
}

impl Tone {
    /// `hz` at `rate` frames per second, at `amplitude` (0..=32767).
    #[must_use]
    pub fn new(hz: u32, rate: u32, amplitude: u16) -> Self {
        let step = ((u64::from(hz) << 32) / u64::from(rate.max(1))) as u32;
        Self {
            phase: 0,
            step,
            amplitude: i32::from(amplitude.min(32767)),
        }
    }
}

impl Iterator for Tone {
    type Item = u32;

    /// The next stereo frame: the same sample on both channels.
    fn next(&mut self) -> Option<u32> {
        let s = ((sine(self.phase) * self.amplitude) >> 15) as i16 as u16;
        self.phase = self.phase.wrapping_add(self.step);
        Some(u32::from(s) << 16 | u32::from(s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rp_hal_mock::MockBus;

    #[test]
    fn start_sets_rate_master_clocks_and_playback_only() {
        let mut i2s = PxaI2s::new(MockBus::new());
        i2s.start(Rate::Khz48);
        let w = i2s.regs.writes();
        assert!(w.contains(&(reg::SADIV, 0x0C)));
        assert!(w.contains(&(reg::SACR1, SACR1_DREC)));
        assert_eq!(
            *w.last().unwrap(),
            (reg::SACR0, SACR0_ENB | SACR0_BCKD | SACR0_TFTH_8)
        );
    }

    #[test]
    fn frames_go_only_when_the_fifo_has_room() {
        let mut bus = MockBus::new();
        bus.script_reads(reg::SASR0, &[SASR0_TNF, 0, SASR0_TNF]);
        let mut i2s = PxaI2s::new(bus);
        assert!(i2s.try_write(1));
        assert!(!i2s.try_write(2));
        assert!(i2s.try_write(3));
        let sent: Vec<u32> = i2s
            .regs
            .writes()
            .iter()
            .filter(|w| w.0 == reg::SADR)
            .map(|w| w.1)
            .collect();
        assert_eq!(sent, [1, 3]);
    }

    #[test]
    fn the_sine_is_accurate_to_a_tenth_of_a_percent() {
        for k in 0..4096u64 {
            let phase = (k << 32) / 4096;
            let want = (std::f64::consts::TAU * k as f64 / 4096.0).sin() * 32767.0;
            let got = f64::from(sine(phase as u32));
            assert!((got - want).abs() < 33.0, "at {k}: {got} vs {want}");
        }
    }

    #[test]
    fn a_tone_has_its_frequency_and_amplitude() {
        // 440 Hz for one second at 48 kHz: 880 zero crossings, both
        // channels the same, peaks at the amplitude.
        let frames: Vec<u32> = Tone::new(440, 48_000, 16_000).take(48_000).collect();
        let left: Vec<i16> = frames.iter().map(|&f| f as u16 as i16).collect();
        assert!(frames.iter().all(|&f| f >> 16 == f & 0xFFFF));
        let crossings = left.windows(2).filter(|w| (w[0] < 0) != (w[1] < 0)).count();
        assert!((878..=882).contains(&crossings), "{crossings}");
        let peak = left.iter().map(|s| s.unsigned_abs()).max().unwrap();
        assert!((15_950..=16_000).contains(&peak), "{peak}");
        // Silence at amplitude 0.
        assert!(Tone::new(440, 48_000, 0).take(100).all(|f| f == 0));
    }
}
