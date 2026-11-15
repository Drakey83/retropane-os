//! A game's clock: microseconds since it started, from the OS timer's
//! counter.
//!
//! Iterate, don't imitate (rule 0). Windows Mobile gives a game
//! `GetTickCount` (whole milliseconds: a 60 Hz frame is 16 or 17 of them)
//! or a performance counter through a kernel call; Linux gives
//! `clock_gettime` through the kernel or its shared page, and the engine
//! turns it into a `double` of seconds at once (a software divide and
//! several software-float calls a frame on a CPU with no FPU). Ours reads
//! the PXA270's own 3.25 MHz counter, which needs no call into anything,
//! and keeps exact whole microseconds in 64 bits: the counter's 13 ticks
//! are 4 microseconds exactly, so nothing drifts and nothing is rounded
//! away (the remainder is carried), the counter's wrap every 22 minutes is
//! folded in on each read, and a game's frame time is integer arithmetic
//! until the game itself asks for a float.

/// The OS timer's ticks in [`US_PER`] microseconds (3.25 MHz: 13 ticks are
/// 4 microseconds exactly).
const TICKS_PER: u32 = 13;
/// See [`TICKS_PER`].
const US_PER: u32 = 4;

/// Microseconds since a start, past any number of the counter's wraps as
/// long as it is read at least once a wrap (22 minutes; a game reads it
/// every frame).
#[derive(Clone, Copy, Debug)]
pub struct Micros {
    last: u32,
    us: u64,
    /// Ticks times [`US_PER`] not yet a whole microsecond (under 13).
    rest: u32,
}

impl Micros {
    /// Starts counting at the counter's value `now`.
    #[must_use]
    pub const fn starting_at(now: u32) -> Self {
        Self {
            last: now,
            us: 0,
            rest: 0,
        }
    }

    /// Microseconds since the start, given the counter's value `now`.
    /// Never goes back.
    pub fn at(&mut self, now: u32) -> u64 {
        let d = now.wrapping_sub(self.last);
        self.last = now;
        // 32-bit divides only (a 64-bit one is a library call on this
        // CPU): the whole thirteens, then what is left with the carry,
        // which is under 13 * 4 + 13.
        let rest = d % TICKS_PER * US_PER + self.rest;
        self.us += u64::from(d / TICKS_PER) * u64::from(US_PER) + u64::from(rest / TICKS_PER);
        self.rest = rest % TICKS_PER;
        self.us
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_of_ticks_is_a_million_microseconds() {
        let mut m = Micros::starting_at(5);
        assert_eq!(m.at(5), 0);
        assert_eq!(m.at(5 + 3_250_000), 1_000_000);
        // One tick is 4/13 of a microsecond: the fourth tick is the first
        // whole one, and thirteen are four exactly.
        let mut m = Micros::starting_at(0);
        let got: Vec<u64> = (1..=13).map(|t| m.at(t)).collect();
        assert_eq!(got, [0, 0, 0, 1, 1, 1, 2, 2, 2, 3, 3, 3, 4]);
    }

    /// Steps of every odd size lose nothing: the total is the ticks' own.
    #[test]
    fn the_remainder_is_carried_and_time_never_goes_back() {
        let mut m = Micros::starting_at(u32::MAX - 1000);
        let mut now = u32::MAX - 1000;
        let mut ticks = 0u64;
        let mut prev = 0;
        for i in 0..200_000u32 {
            let d = i * 7919 % 60_000;
            now = now.wrapping_add(d);
            ticks += u64::from(d);
            let us = m.at(now);
            assert_eq!(us, ticks * 4 / 13, "at step {i}");
            assert!(us >= prev);
            prev = us;
        }
    }

    /// Ten wraps of the counter (3.7 hours), read a quarter-wrap apart, and
    /// one read a tick short of a whole wrap.
    #[test]
    fn the_counter_s_wrap_is_folded_in() {
        let mut m = Micros::starting_at(0);
        let mut now = 0u32;
        for _ in 0..40 {
            now = now.wrapping_add(1 << 30);
            m.at(now);
        }
        assert_eq!(m.at(now), (10u64 << 32) * 4 / 13);
        let mut m = Micros::starting_at(7);
        assert_eq!(m.at(6), u64::from(u32::MAX) * 4 / 13);
    }
}
