//! The temperature guard for games (R76; ADR-0035 amendment D, "The game
//! step", ruled by the owner 2026-11-09): the 3D core at the game step,
//! 136.50 / 1, steps down to 133.25 / 1 at peak heat and back up when it
//! cools, with hysteresis.
//!
//! **Rule 0.** Dell's driver has no guard: one clock, rated by the vendor
//! for its hottest day. Phones and PC GPUs boost and back off by a
//! temperature table fitted per product line. Ours fits the trip to what
//! this chip was *proven* at: the game step runs only inside the
//! temperatures at which it drew every game frame right, and the peak-heat
//! rung past them. Every number here is a measurement, not a table:
//!
//! - **The trip.** `TEMP0` is the touch controller's diode, 64 conversions
//!   averaged (lower is warmer, about 3-4 counts a degree, docs/hw/2700g.md
//!   GC2BB). The hottest reading of any run that proved the game step on
//!   moved game frames is **867** (GC2DT, 136.50 and 137.80 / 1; GC2DV,
//!   4,800 of 4,800 at 137.80 / 1, every part 867; GC2DR). So the step
//!   down is at **866**, the first reading hotter than every proof.
//! - **The hysteresis.** A steady chip's reading moves by one count (GC2DV
//!   867 in all 12 parts; GC2DG 864-865 within a run): the step back up is
//!   at **868**, one count cooler than the proven edge, so the reading's
//!   own spread can't flip the clock.
//! - **The pace.** One reading every 10 s, ADR-0035 rule 6's check period
//!   in a game; the chip's heat moves over minutes (GC2BB: 7 counts in 10
//!   minutes from idle), so a faster guard buys nothing.
//!
//! These are the shipped defaults for every unit until the per-device
//! self-test (R71) measures each chip's own envelope and replaces them (it
//! is still open: decisions-2026-11-12-evening.md). Neither rung has been
//! run hotter than 867 yet: 133.25 / 1 past the trip is the owner's ruled
//! peak-heat rung, not a measured one, and the guard's error checks (rule
//! 6) stay the net under both.
//!
//! **Untrusted input** (ADR-0035 "Security"): a reading that is missing or
//! at the converter's rails (0 or 4095: an open or shorted input) counts as
//! hot. A clock change that doesn't take latches Windows Mobile's setting
//! (rules 2 and 4) until a re-test.

use crate::clock::{Rung, Speed};

/// The game step: 13 x 21 / 2 = 136.50 MHz, the cores at divide 1 (the
/// owner's ruling of 2026-11-09; `rp-gpubench`'s lever (21, 2, 0)).
pub const GAME_STEP: Rung = Rung {
    speed: Speed::ladder(21 << 7 | 2 << 4 | 1),
    div: 1,
};

/// The peak-heat rung: 13 x 41 / 4 = 133.25 MHz, the cores at divide 1
/// (lever (41, 4, 0)).
pub const PEAK_HEAT: Rung = Rung {
    speed: Speed::ladder(41 << 7 | 4 << 4 | 1),
    div: 1,
};

/// The shipped step down: `TEMP0` at or below this (hotter than every run
/// that proved the game step, whose hottest read 867).
pub const TRIP: u16 = 866;

/// The shipped step back up: `TEMP0` at or above this (one count of the
/// reading's spread cooler than the proven edge).
pub const CLEAR: u16 = 868;

/// One reading every this many milliseconds (ADR-0035 rule 6, in a game).
pub const PERIOD_MS: u32 = 10_000;

/// The guard: which rung games run at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeatGuard {
    trip: u16,
    clear: u16,
    hot: bool,
    failed: bool,
    last_ms: Option<u32>,
}

impl HeatGuard {
    /// The shipped guard ([`TRIP`], [`CLEAR`]), at the game step.
    pub const SHIPPED: Self = Self::new(TRIP, CLEAR);

    /// A guard stepping down at `trip` or hotter (lower) and up at `clear`
    /// or cooler; `clear` is kept above `trip` (a band of at least one
    /// count), so it can't step both ways on one reading.
    #[must_use]
    pub const fn new(trip: u16, clear: u16) -> Self {
        Self {
            trip,
            clear: if clear > trip { clear } else { trip + 1 },
            hot: false,
            failed: false,
            last_ms: None,
        }
    }

    /// The thresholds (trip, clear).
    #[must_use]
    pub const fn thresholds(&self) -> (u16, u16) {
        (self.trip, self.clear)
    }

    /// The rung games run at now.
    #[must_use]
    pub const fn rung(&self) -> Rung {
        if self.failed {
            Rung::WM
        } else if self.hot {
            PEAK_HEAT
        } else {
            GAME_STEP
        }
    }

    /// Whether it is at the peak-heat rung.
    #[must_use]
    pub const fn hot(&self) -> bool {
        self.hot
    }

    /// Whether a change failed and Windows Mobile's setting is latched.
    #[must_use]
    pub const fn failed(&self) -> bool {
        self.failed
    }

    /// A reading of `TEMP0` at `now_ms` (`None`: none could be had): the
    /// rung to change to, if it changes. Readings closer together than
    /// [`PERIOD_MS`] are ignored (the first is always taken).
    pub fn reading(&mut self, now_ms: u32, temp0: Option<u16>) -> Option<Rung> {
        if self.failed {
            return None;
        }
        if let Some(t) = self.last_ms
            && now_ms.wrapping_sub(t) < PERIOD_MS
        {
            return None;
        }
        self.last_ms = Some(now_ms);
        let was = self.hot;
        self.hot = match temp0.filter(|&v| v != 0 && v < 4095) {
            None => true,
            Some(v) if v <= self.trip => true,
            Some(v) if v >= self.clear => false,
            Some(_) => was,
        };
        (self.hot != was).then(|| self.rung())
    }

    /// A change to [`Self::rung`] didn't take (the PLL unlocked or a
    /// memory-controller write never held): Windows Mobile's setting from
    /// now on, until a re-test makes a new guard.
    pub fn change_failed(&mut self) -> Rung {
        self.failed = true;
        Rung::WM
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rungs_are_the_ruled_ones() {
        assert_eq!(GAME_STEP.core_khz(), 136_500);
        assert_eq!(PEAK_HEAT.core_khz(), 133_250);
        // CL2 at both (ADR-0035 rule 3).
        assert_eq!(GAME_STEP.speed.lmtype, Speed::WM.lmtype);
        assert_eq!(PEAK_HEAT.speed.lmtype, Speed::WM.lmtype);
        // The trip hotter than the proven edge (867), the clear cooler.
        let proven_edge = 867;
        assert!(TRIP < proven_edge && CLEAR > proven_edge);
        assert_eq!(PERIOD_MS, 10_000);
    }

    #[test]
    fn it_steps_down_at_the_trip_and_up_only_past_the_band() {
        let mut g = HeatGuard::SHIPPED;
        assert_eq!(g.rung(), GAME_STEP);
        let mut t = 0;
        let mut at = |g: &mut HeatGuard, v: u16| {
            t += PERIOD_MS;
            g.reading(t, Some(v))
        };
        assert_eq!(at(&mut g, 880), None);
        assert_eq!(at(&mut g, 867), None, "the proven edge keeps the step");
        assert_eq!(at(&mut g, 866), Some(PEAK_HEAT));
        assert_eq!(at(&mut g, 860), None);
        // Inside the band: stays down, whichever way it moves.
        assert_eq!(at(&mut g, 867), None);
        assert_eq!(at(&mut g, 866), None);
        assert_eq!(at(&mut g, 867), None);
        assert!(g.hot());
        assert_eq!(at(&mut g, 868), Some(GAME_STEP));
        // Inside the band from the cool side: stays up.
        assert_eq!(at(&mut g, 867), None);
        assert_eq!(g.rung(), GAME_STEP);
    }

    #[test]
    fn a_reading_that_flickers_one_count_never_flips_the_clock() {
        let mut g = HeatGuard::SHIPPED;
        let mut changes = 0;
        for (i, v) in [867u16, 866, 867, 866, 867, 866, 867, 866]
            .into_iter()
            .enumerate()
        {
            changes += u32::from(g.reading(i as u32 * PERIOD_MS, Some(v)).is_some());
        }
        assert_eq!(changes, 1, "down once, never back up inside the band");
        let mut g = HeatGuard::SHIPPED;
        let mut changes = 0;
        for (i, v) in [868u16, 867, 868, 867, 868].into_iter().enumerate() {
            changes += u32::from(g.reading(i as u32 * PERIOD_MS, Some(v)).is_some());
        }
        assert_eq!(changes, 0);
    }

    #[test]
    fn a_missing_or_railed_reading_counts_as_hot() {
        for bad in [None, Some(0), Some(4095)] {
            let mut g = HeatGuard::SHIPPED;
            assert_eq!(g.reading(0, bad), Some(PEAK_HEAT), "{bad:?}");
        }
        // And a good cool reading brings it back.
        let mut g = HeatGuard::SHIPPED;
        g.reading(0, None);
        assert_eq!(g.reading(PERIOD_MS, Some(870)), Some(GAME_STEP));
    }

    #[test]
    fn readings_inside_the_period_are_ignored() {
        let mut g = HeatGuard::SHIPPED;
        assert_eq!(g.reading(5, Some(880)), None);
        assert_eq!(g.reading(5 + PERIOD_MS - 1, Some(800)), None, "too soon");
        assert!(!g.hot());
        assert_eq!(g.reading(5 + PERIOD_MS, Some(800)), Some(PEAK_HEAT));
        // Across the clock's wrap.
        let mut g = HeatGuard::SHIPPED;
        g.reading(u32::MAX - 100, Some(880));
        assert_eq!(g.reading(PERIOD_MS, Some(800)), Some(PEAK_HEAT));
    }

    #[test]
    fn a_failed_change_latches_windows_mobiles_setting() {
        let mut g = HeatGuard::SHIPPED;
        assert_eq!(g.reading(0, Some(800)), Some(PEAK_HEAT));
        assert_eq!(g.change_failed(), Rung::WM);
        assert_eq!(g.rung(), Rung::WM);
        assert!(g.failed());
        assert_eq!(g.reading(PERIOD_MS, Some(900)), None);
        assert_eq!(g.rung(), Rung::WM);
    }

    #[test]
    fn a_band_is_always_at_least_one_count() {
        assert_eq!(HeatGuard::new(870, 870).thresholds(), (870, 871));
        assert_eq!(HeatGuard::new(870, 860).thresholds(), (870, 871));
        assert_eq!(HeatGuard::new(870, 873).thresholds(), (870, 873));
    }
}
