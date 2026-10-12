//! PXA27x pulse-width modulator, and the X51v backlight on PWM0
//! (docs/hw/backlight.md).
//!
//! Each channel has three registers: control (prescale), duty cycle (10
//! bits, plus a full-duty bit) and period (10 bits): the output is high for
//! `duty / (period + 1)` of each cycle. Windows Mobile leaves the backlight
//! at period 0xD7 (216 counts); [`Backlight`] keeps whatever period it
//! finds (and sets that one if none is set up) and only moves the duty.
//!
//! Levels are perceptual: brightness looks roughly like the square of the
//! duty cycle, so `level` percent maps to `level²` of full duty and 50%
//! looks like half, not almost full. Any level above 0 keeps at least one
//! count on (never black unless asked), 100% uses full duty, and changes
//! fade in small steps instead of jumping.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use rp_hal_api::RegisterBus;

/// Register offsets within a channel.
pub mod reg {
    /// Control: prescale `[5:0]`, abrupt shutdown `[6]`.
    pub const CR: usize = 0x0;
    /// Duty cycle `[9:0]`, full duty `[10]`.
    pub const DCR: usize = 0x4;
    /// Period `[9:0]`: the cycle is `period + 1` counts.
    pub const PCR: usize = 0x8;
}

const FULL_DUTY: u32 = 1 << 10;
const DUTY_MASK: u32 = 0x3FF;
/// The period Windows Mobile uses for the backlight (and the default when
/// none is set up): 216 counts.
pub const WM_PERIOD: u32 = 0xD7;
/// The prescale Windows Mobile uses.
pub const WM_PRESCALE: u32 = 1;
/// Fade steps.
const STEPS: u32 = 16;
/// Microseconds between fade steps (16 steps: about a quarter second).
pub const STEP_US: u32 = 15_000;

/// A backlight on one PWM channel.
pub struct Backlight<B> {
    bus: B,
}

impl<B: RegisterBus> Backlight<B> {
    /// Takes the channel; if no period is set up (cold start, the
    /// emulator), sets Windows Mobile's period and prescale, light off.
    pub fn new(mut bus: B) -> Self {
        if bus.read32(reg::PCR) & DUTY_MASK == 0 {
            bus.write32(reg::CR, WM_PRESCALE);
            bus.write32(reg::PCR, WM_PERIOD);
            bus.write32(reg::DCR, 0);
        }
        Self { bus }
    }

    fn counts(&mut self) -> u32 {
        (self.bus.read32(reg::PCR) & DUTY_MASK) + 1
    }

    /// The duty (counts on per cycle) and the cycle length.
    pub fn duty(&mut self) -> (u32, u32) {
        let counts = self.counts();
        let d = self.bus.read32(reg::DCR);
        let on = if d & FULL_DUTY != 0 {
            counts
        } else {
            (d & DUTY_MASK).min(counts)
        };
        (on, counts)
    }

    /// The duty for `level` percent (0..=100) on a cycle of `counts`.
    #[must_use]
    pub fn duty_for(level: u32, counts: u32) -> u32 {
        let level = level.min(100);
        if level == 0 {
            return 0;
        }
        (level * level * counts / 10_000).max(1)
    }

    /// The level (percent) the current duty corresponds to (the inverse of
    /// [`Backlight::duty_for`], rounded).
    pub fn level(&mut self) -> u32 {
        let (on, counts) = self.duty();
        // sqrt(on / counts) * 100, in integers.
        let target = on * 10_000 / counts;
        let mut l = 0;
        while l < 100 && (l + 1) * (l + 1) <= target {
            l += 1;
        }
        // Round: pick the nearer of l and l + 1.
        if l < 100 && target - l * l > (l + 1) * (l + 1) - target {
            l += 1;
        }
        l
    }

    fn set_duty(&mut self, on: u32, counts: u32) {
        let v = if on >= counts { FULL_DUTY } else { on };
        self.bus.write32(reg::DCR, v);
    }

    /// Sets `level` percent at once.
    pub fn set(&mut self, level: u32) {
        let counts = self.counts();
        self.set_duty(Self::duty_for(level, counts), counts);
    }

    /// Fades to `level` percent in [`STEPS`] perceptual steps, calling
    /// `delay` (microseconds) between them.
    pub fn fade_to(&mut self, level: u32, delay: &mut dyn FnMut(u32)) {
        let from = self.level();
        let to = level.min(100);
        let counts = self.counts();
        for i in 1..=STEPS {
            let l = if to >= from {
                from + (to - from) * i / STEPS
            } else {
                from - (from - to) * i / STEPS
            };
            // The last step is exactly the target.
            let on = Self::duty_for(if i == STEPS { to } else { l }, counts);
            self.set_duty(on, counts);
            if i < STEPS {
                delay(STEP_US);
            }
        }
    }

    /// Gives the channel back.
    pub fn into_inner(self) -> B {
        self.bus
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rp_hal_mock::MockBus;

    fn wm() -> MockBus {
        let mut b = MockBus::new();
        b.preset(reg::CR, 1);
        b.preset(reg::DCR, 0x89);
        b.preset(reg::PCR, 0xD7);
        b
    }

    #[test]
    fn keeps_windows_mobiles_set_up_and_reads_its_level() {
        let mut bl = Backlight::new(wm());
        assert_eq!(bl.duty(), (137, 216));
        // 137/216 = 63% of full duty, which looks like ~80%.
        assert_eq!(bl.level(), 80);
        let bus = bl.into_inner();
        assert!(bus.writes().is_empty(), "nothing changed just by taking it");
    }

    #[test]
    fn a_cold_channel_gets_windows_mobiles_period_and_starts_dark() {
        let bl = Backlight::new(MockBus::new());
        let w = bl.into_inner().writes();
        assert_eq!(w, [(reg::CR, 1), (reg::PCR, 0xD7), (reg::DCR, 0)]);
    }

    #[test]
    fn levels_are_perceptual_with_a_floor_and_full_duty_at_100() {
        assert_eq!(Backlight::<MockBus>::duty_for(0, 216), 0);
        assert_eq!(
            Backlight::<MockBus>::duty_for(1, 216),
            1,
            "never black above 0"
        );
        assert_eq!(
            Backlight::<MockBus>::duty_for(50, 216),
            54,
            "half looks half"
        );
        assert_eq!(Backlight::<MockBus>::duty_for(100, 216), 216);
        assert_eq!(Backlight::<MockBus>::duty_for(250, 216), 216, "clamped");
        let mut bl = Backlight::new(wm());
        bl.set(100);
        assert_eq!(bl.duty(), (216, 216));
        assert!(bl.into_inner().writes().contains(&(reg::DCR, FULL_DUTY)));
        // Every level reads back as one that sets the same duty: the
        // period has 216 counts, so the lowest levels share 1 count (one
        // count already looks like ~7%). The period isn't lengthened for
        // finer steps: that would lower the PWM frequency towards the
        // audible range, where backlight circuits whine.
        let mut bl = Backlight::new(wm());
        for l in [1, 5, 10, 33, 50, 64, 80, 99, 100] {
            bl.set(l);
            let (on, counts) = bl.duty();
            let got = bl.level();
            assert_eq!(
                Backlight::<MockBus>::duty_for(got, counts),
                on,
                "level {l} read back {got}"
            );
            if l >= 20 {
                assert!(got.abs_diff(l) <= 1, "level {l} read back {got}");
            }
        }
    }

    #[test]
    fn fades_monotonically_in_steps_and_lands_exactly() {
        let mut bl = Backlight::new(wm());
        let mut delays = 0;
        bl.fade_to(20, &mut |us| {
            assert_eq!(us, STEP_US);
            delays += 1;
        });
        assert_eq!(delays, STEPS - 1);
        let duties: Vec<u32> = bl
            .into_inner()
            .writes()
            .iter()
            .filter(|(o, _)| *o == reg::DCR)
            .map(|&(_, v)| v)
            .collect();
        assert_eq!(duties.len(), STEPS as usize);
        assert!(duties.windows(2).all(|w| w[1] <= w[0]), "{duties:?}");
        assert_eq!(
            *duties.last().unwrap(),
            Backlight::<MockBus>::duty_for(20, 216)
        );
    }
}
