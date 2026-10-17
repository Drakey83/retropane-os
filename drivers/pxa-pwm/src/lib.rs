//! PXA27x pulse-width modulator, and the X51v backlight on PWM0
//! (docs/hw/backlight.md).
//!
//! Each channel has three registers: control (prescale), duty cycle (10
//! bits, plus a full-duty bit) and period (10 bits): the output is high for
//! `duty / (period + 1)` of each cycle. Windows Mobile runs the backlight
//! at prescale 1 and 216 counts (30.09 kHz); [`Backlight`] takes the channel
//! over at prescale 0 and [`COUNTS`] = 432 counts: the same 30.09 kHz, so
//! nothing new to hear, with steps half the size (the dimmest is half
//! Windows Mobile's). The brightness it finds is kept across the switch.
//! Below the light's dimmest step, [`split`] says how much to dim the
//! picture instead (the display's gamma table, item 76).
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
/// The period Windows Mobile uses for the backlight: 216 counts.
pub const WM_PERIOD: u32 = 0xD7;
/// The prescale Windows Mobile uses (a 6.5 MHz count).
pub const WM_PRESCALE: u32 = 1;
/// RetroPane's prescale: the 13 MHz clock itself, the finest count.
pub const PRESCALE: u32 = 0;
/// RetroPane's cycle: 432 counts at 13 MHz = 30.09 kHz, Windows Mobile's
/// frequency (13 MHz / 2 / 216), so a whine-free choice stays whine-free.
/// A longer cycle would mean a lower frequency, towards the audible range.
pub const COUNTS: u32 = 432;
/// The picture's scale at full, in 1/256ths ([`split`]).
pub const PICTURE_FULL: u32 = 256;
/// The darkest the picture is scaled for brightness: 1/8 (the panel shows
/// 6 bits a channel, so 3 are kept; the 2D engine's dither hides the rest).
pub const PICTURE_MIN: u32 = PICTURE_FULL / 8;
/// Fade steps.
const STEPS: u32 = 16;
/// Microseconds between fade steps (16 steps: about a quarter second).
pub const STEP_US: u32 = 15_000;

/// A backlight on one PWM channel.
pub struct Backlight<B> {
    bus: B,
}

impl<B: RegisterBus> Backlight<B> {
    /// Takes the channel over at [`PRESCALE`] and [`COUNTS`], keeping the
    /// brightness it finds (Windows Mobile's, or a previous program's); a
    /// channel never set up (cold start, the emulator) starts dark.
    pub fn new(mut bus: B) -> Self {
        let pcr = bus.read32(reg::PCR) & DUTY_MASK;
        let d = bus.read32(reg::DCR);
        // The same fraction of the cycle, never off if it was on; a period
        // of 0 is a channel never set up. Written even when already ours
        // (the same values: nothing changes on screen), which keeps this
        // small enough for the display host's budget.
        let on = if pcr == 0 {
            0
        } else if d & FULL_DUTY != 0 {
            FULL_DUTY
        } else {
            let found = d & DUTY_MASK;
            (found * COUNTS / (pcr + 1)).max(u32::from(found != 0))
        };
        bus.write32(reg::CR, PRESCALE);
        bus.write32(reg::PCR, COUNTS - 1);
        bus.write32(reg::DCR, on);
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

/// What a brightness asks of the light and of the picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Split {
    /// Counts on per [`COUNTS`]-count cycle.
    pub duty: u32,
    /// The picture's scale in 1/256ths ([`PICTURE_FULL`] = untouched).
    pub picture: u32,
}

/// Two-stage brightness for `permille` (0..=1000, perceptual like the
/// levels: light = permille² of full). Down to the light's dimmest step only
/// the light moves; below it the light stays at one count and the picture
/// is scaled down instead, to [`PICTURE_MIN`]: a night mode far below
/// Windows Mobile's dimmest (one count of 216) at no CPU cost, with nothing
/// flickering and nothing new to hear. 0 is off.
#[must_use]
pub fn split(permille: u32) -> Split {
    let p = permille.min(1000);
    if p == 0 {
        return Split {
            duty: 0,
            picture: PICTURE_FULL,
        };
    }
    // Light wanted, in 1/(COUNTS * 256) of full.
    let want = (u64::from(p * p) * u64::from(COUNTS * PICTURE_FULL) / 1_000_000) as u32;
    if want >= PICTURE_FULL {
        return Split {
            duty: (want + PICTURE_FULL / 2) / PICTURE_FULL,
            picture: PICTURE_FULL,
        };
    }
    Split {
        duty: 1,
        picture: want.max(PICTURE_MIN),
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
    fn takes_windows_mobiles_light_over_at_the_same_frequency_and_brightness() {
        let mut bl = Backlight::new(wm());
        // 137/216 becomes 274/432: the same light at the same 30.09 kHz.
        assert_eq!(bl.duty(), (274, 432));
        // 63% of full duty, which looks like ~80%.
        assert_eq!(bl.level(), 80);
        let w = bl.into_inner().writes();
        assert_eq!(w, [(reg::CR, 0), (reg::PCR, 431), (reg::DCR, 274)]);
        assert_eq!(
            13_000_000 / (PRESCALE + 1) / COUNTS,
            13_000_000 / (WM_PRESCALE + 1) / (WM_PERIOD + 1)
        );
    }

    #[test]
    fn taking_it_again_keeps_the_values_and_the_ends_are_kept() {
        let mut b = MockBus::new();
        b.preset(reg::CR, 0);
        b.preset(reg::DCR, 5);
        b.preset(reg::PCR, 431);
        assert_eq!(
            Backlight::new(b).into_inner().writes(),
            [(reg::CR, 0), (reg::PCR, 431), (reg::DCR, 5)]
        );
        let mut b = MockBus::new();
        b.preset(reg::CR, 1);
        b.preset(reg::DCR, 1);
        b.preset(reg::PCR, 0xD7);
        assert_eq!(
            Backlight::new(b).duty(),
            (2, 432),
            "Windows Mobile's dimmest"
        );
        let mut b = MockBus::new();
        b.preset(reg::CR, 1);
        b.preset(reg::DCR, FULL_DUTY);
        b.preset(reg::PCR, 0xD7);
        assert_eq!(Backlight::new(b).duty(), (432, 432), "full stays full");
    }

    #[test]
    fn a_cold_channel_gets_retropanes_cycle_and_starts_dark() {
        let bl = Backlight::new(MockBus::new());
        let w = bl.into_inner().writes();
        assert_eq!(w, [(reg::CR, 0), (reg::PCR, 431), (reg::DCR, 0)]);
    }

    #[test]
    fn split_moves_the_light_then_the_picture() {
        let full = PICTURE_FULL;
        assert_eq!(
            split(0),
            Split {
                duty: 0,
                picture: full
            }
        );
        assert_eq!(
            split(1000),
            Split {
                duty: 432,
                picture: full
            }
        );
        assert_eq!(
            split(500),
            Split {
                duty: 108,
                picture: full
            },
            "half looks half"
        );
        // One count: the light's floor, half Windows Mobile's dimmest.
        assert_eq!(
            split(49),
            Split {
                duty: 1,
                picture: full
            }
        );
        // Below it the picture dims, down to 1/8 of the floor.
        let s = split(30);
        assert_eq!(s.duty, 1);
        assert!(s.picture < full && s.picture > PICTURE_MIN, "{s:?}");
        assert_eq!(
            split(1),
            Split {
                duty: 1,
                picture: PICTURE_MIN
            }
        );
        // Never brighter for a lower setting.
        let light = |s: Split| s.duty * s.picture;
        for p in 1..1000 {
            assert!(light(split(p)) <= light(split(p + 1)), "{p}");
        }
    }

    #[test]
    fn levels_are_perceptual_with_a_floor_and_full_duty_at_100() {
        assert_eq!(Backlight::<MockBus>::duty_for(0, 432), 0);
        assert_eq!(
            Backlight::<MockBus>::duty_for(1, 432),
            1,
            "never black above 0"
        );
        assert_eq!(
            Backlight::<MockBus>::duty_for(50, 432),
            108,
            "half looks half"
        );
        assert_eq!(Backlight::<MockBus>::duty_for(100, 432), 432);
        assert_eq!(Backlight::<MockBus>::duty_for(250, 432), 432, "clamped");
        let mut bl = Backlight::new(wm());
        bl.set(100);
        assert_eq!(bl.duty(), (432, 432));
        assert!(bl.into_inner().writes().contains(&(reg::DCR, FULL_DUTY)));
        // Every level reads back as one that sets the same duty: the
        // cycle has 432 counts, so the lowest levels share 1 count (one
        // count looks like ~5%; below it, `split` dims the picture). The
        // cycle isn't lengthened further: at the finest count that would
        // lower the PWM frequency towards the audible range, where
        // backlight circuits whine.
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
        let taken = bl.duty().0;
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
        // The take-over's write (the same light at 432 counts), then the fade.
        assert_eq!(duties[0], taken);
        let duties = &duties[1..];
        assert_eq!(duties.len(), STEPS as usize);
        assert!(duties.windows(2).all(|w| w[1] <= w[0]), "{duties:?}");
        assert_eq!(
            *duties.last().unwrap(),
            Backlight::<MockBus>::duty_for(20, 432)
        );
    }
}
