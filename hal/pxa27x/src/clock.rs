//! Elapsed time that outlasts the OS timer's wrap.
//!
//! `OSCR0` counts at 3.25 MHz in 32 bits, so it wraps every 1,321.5 s (22 min
//! 1.5 s). `ticks().wrapping_sub(t0)` is right only for spans shorter than
//! that; a longer span silently comes out small. That froze
//! `rp-screen-measure` 22 minutes into a 26-minute run (2026-10-17): its next
//! patch was due just past the wrap, and "milliseconds since the start" fell
//! back to zero and never reached it.
//!
//! [`Elapsed`] keeps a 64-bit tick total and folds in the counter's
//! difference on every read, so it stays right for any length of run as long
//! as it is read at least once per wrap (every wait loop reads it far more
//! often than that). Any span that can exceed a few minutes uses it.

/// OS timer frequency (`OSCR0`).
pub const TICK_HZ: u32 = 3_250_000;

/// The longest span `ticks().wrapping_sub(t0)` measures correctly, in
/// milliseconds (2^32 ticks at 3.25 MHz).
pub const WRAP_MS: u64 = (1u64 << 32) * 1000 / TICK_HZ as u64;

/// Time since a start, past any number of timer wraps.
///
/// Counts whole milliseconds plus a remainder of ticks, with only 32-bit
/// division (a 64-bit one would pull a library routine into the loader).
#[derive(Clone, Copy, Debug)]
pub struct Elapsed {
    last: u32,
    ms: u64,
    rest: u32,
}

/// OS timer ticks per millisecond.
const TICKS_PER_MS: u32 = TICK_HZ / 1000;

impl Elapsed {
    /// Starts counting at the timer value `now`.
    #[must_use]
    pub const fn starting_at(now: u32) -> Self {
        Self {
            last: now,
            ms: 0,
            rest: 0,
        }
    }

    /// Milliseconds since the start, given the timer value `now`. Must be
    /// called at least once per wrap ([`WRAP_MS`]).
    pub fn ms_at(&mut self, now: u32) -> u64 {
        let d = now.wrapping_sub(self.last);
        self.last = now;
        // `rest` < TICKS_PER_MS, so this can't overflow: d % 3250 + 3249.
        let rest = d % TICKS_PER_MS + self.rest;
        self.ms += u64::from(d / TICKS_PER_MS + rest / TICKS_PER_MS);
        self.rest = rest % TICKS_PER_MS;
        self.ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wrap_is_22_minutes() {
        assert_eq!(WRAP_MS, 1_321_528);
    }

    /// The bug's own numbers: started near the top of the counter, read every
    /// 20 ms for 30 minutes; elapsed time keeps growing through the wrap.
    #[test]
    fn counts_through_the_wrap() {
        let t0 = u32::MAX - 5_000 * TICKS_PER_MS;
        let mut e = Elapsed::starting_at(t0);
        let mut now = t0;
        let mut prev = 0;
        for step in 1..=(30 * 60 * 1000 / 20_u32) {
            now = now.wrapping_add(20 * TICKS_PER_MS);
            let ms = e.ms_at(now);
            assert!(ms > prev, "time went back at step {step}: {prev} -> {ms}");
            assert_eq!(ms, u64::from(step) * 20);
            prev = ms;
        }
        // The plain 32-bit difference has long since wrapped.
        assert!(u64::from(now.wrapping_sub(t0) / TICKS_PER_MS) < prev);
    }

    #[test]
    fn many_wraps() {
        let mut e = Elapsed::starting_at(0);
        let mut now = 0u32;
        for _ in 0..10 * 4 {
            // Quarter-wrap steps: ten whole wraps, about 3.7 hours.
            now = now.wrapping_add(1 << 30);
            e.ms_at(now);
        }
        assert_eq!(e.ms_at(now), (10u64 << 32) / u64::from(TICKS_PER_MS));
    }

    /// Odd-sized steps lose nothing to rounding: the remainder carries.
    #[test]
    fn remainders_carry() {
        let mut e = Elapsed::starting_at(7);
        let mut now = 7u32;
        let mut ticks = 0u64;
        for i in 0..100_000u32 {
            let d = 1 + i * 7919 % 9000;
            now = now.wrapping_add(d);
            ticks += u64::from(d);
            assert_eq!(e.ms_at(now), ticks / u64::from(TICKS_PER_MS));
        }
    }
}
