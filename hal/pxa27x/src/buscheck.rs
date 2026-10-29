//! The 2700G bus check of the per-device self-test (R71, the owner's R3
//! ruling 2026-10-28): which CS3 timing (`MSC1`'s upper half) a unit runs.
//! Every unit runs 7 / 3 / 1 (`RDF` / `RDN` / `RRR`); it moves up to
//! 7 / 2 / 0 only if 7 / 2 / 0 passes both a cold check and a warm one, and
//! a unit that fails either stays at 7 / 3 / 1.
//!
//! Rule 0: Windows Mobile keeps 8 / 4 / 2 on every unit, never checked; the
//! textbook memory test walks a bit pattern once. Ours: six patterns
//! (address in data and its complement for aliasing, alternating bits for
//! neighbouring lines, walking ones and zeros, hashed noise), each written
//! over the region and read back word for word, the check passing only on
//! zero misses over at least [`MIN_TRANSFERS`] (a fault of 1 in 1,000
//! transfers misses so many with probability e^-100), and run again warm,
//! after minutes of bus traffic at the timing under test: a margin that
//! only holds cold is no margin.

/// A test pattern: the word each index gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pattern {
    /// The word's own byte address (catches address aliasing).
    Address,
    /// Its complement.
    Inverse,
    /// Alternating `0xAAAAAAAA` / `0x55555555` (neighbouring data lines).
    Checker,
    /// One bit set, walking.
    WalkOne,
    /// One bit clear, walking.
    WalkZero,
    /// Hashed noise (every bit pattern, data lines switching hard).
    Random,
}

impl Pattern {
    /// Every pattern, in the order run.
    pub const ALL: [Self; 6] = [
        Self::Address,
        Self::Inverse,
        Self::Checker,
        Self::WalkOne,
        Self::WalkZero,
        Self::Random,
    ];

    /// Its name in the log.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Address => "address",
            Self::Inverse => "inverse",
            Self::Checker => "checker",
            Self::WalkOne => "walk1",
            Self::WalkZero => "walk0",
            Self::Random => "random",
        }
    }

    /// The word for index `i` of a region at byte address `base`.
    #[must_use]
    pub const fn word(self, base: u32, i: u32) -> u32 {
        match self {
            Self::Address => base.wrapping_add(i.wrapping_mul(4)),
            Self::Inverse => !base.wrapping_add(i.wrapping_mul(4)),
            Self::Checker => {
                if i & 1 == 0 {
                    0xAAAA_AAAA
                } else {
                    0x5555_5555
                }
            }
            Self::WalkOne => 1 << (i % 32),
            Self::WalkZero => !(1 << (i % 32)),
            Self::Random => mix(i),
        }
    }
}

/// A stateless 32-bit hash (finaliser of MurmurHash3), so any word can be
/// recomputed without replaying a generator.
#[must_use]
pub const fn mix(i: u32) -> u32 {
    let mut x = i.wrapping_mul(0x9E37_79B9) ^ 0x5EED_C0DE;
    x ^= x >> 16;
    x = x.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 13;
    x = x.wrapping_mul(0xC2B2_AE35);
    x ^ (x >> 16)
}

/// CS3's timing: `RDF`, `RDN`, `RRR`.
pub type Timing = (u32, u32, u32);
/// Every unit's timing (the owner's R3 ruling).
pub const DEFAULT: Timing = (7, 3, 1);
/// The faster timing a unit earns by passing both checks.
pub const FAST: Timing = (7, 2, 0);
/// The fewest transfers (writes and reads) a check needs to count.
pub const MIN_TRANSFERS: u64 = 100_000;

/// What a check found.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Words written plus words read.
    pub transfers: u64,
    /// Words read back wrong.
    pub misses: u32,
    /// The first: pattern index, word index, wanted, read.
    pub first: Option<(usize, usize, u32, u32)>,
}

impl Report {
    /// Clean over enough transfers to count.
    #[must_use]
    pub const fn passed(&self) -> bool {
        self.misses == 0 && self.transfers >= MIN_TRANSFERS
    }
}

/// Every pattern over `words` words at byte address `base`: written whole
/// through `write(index, word)`, then read whole through `read(index)`
/// (`None`: unreadable, a miss); `each` called between passes (the
/// watchdog's check-in, the write buffer's drain).
pub fn check(
    base: u32,
    words: usize,
    mut write: impl FnMut(usize, u32),
    mut read: impl FnMut(usize) -> Option<u32>,
    mut each: impl FnMut(),
) -> Report {
    let mut r = Report::default();
    for (p, pat) in Pattern::ALL.into_iter().enumerate() {
        for k in 0..words {
            write(k, pat.word(base, k as u32));
        }
        each();
        for k in 0..words {
            let want = pat.word(base, k as u32);
            let got = read(k);
            if got != Some(want) {
                r.misses = r.misses.saturating_add(1);
                r.first.get_or_insert((p, k, want, got.unwrap_or(!want)));
            }
        }
        each();
        r.transfers += 2 * words as u64;
    }
    r
}

/// The timing a unit runs: [`FAST`] only if the default passed and the fast
/// timing passed cold and warm; [`DEFAULT`] otherwise (and `false` if even
/// the default failed: a fault for the log, never a reason to go faster or
/// slower than the default here).
#[must_use]
pub fn pick(default: &Report, fast_cold: &Report, fast_warm: Option<&Report>) -> (Timing, bool) {
    let fast = default.passed() && fast_cold.passed() && fast_warm.is_some_and(Report::passed);
    (if fast { FAST } else { DEFAULT }, default.passed())
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;
    use std::vec;

    /// A region of `n` words whose every `every`th write (if any) stores a
    /// flipped bit.
    fn run(n: usize, every: Option<usize>) -> Report {
        let mem = core::cell::RefCell::new(vec![0u32; n]);
        let mut writes = 0usize;
        check(
            0x0C14_0000,
            n,
            |k, w| {
                writes += 1;
                mem.borrow_mut()[k] = if every.is_some_and(|e| writes.is_multiple_of(e)) {
                    w ^ 0x10
                } else {
                    w
                };
            },
            |k| Some(mem.borrow()[k]),
            || {},
        )
    }

    #[test]
    fn a_clean_bus_passes_with_enough_transfers() {
        let r = run(128 * 1024, None);
        assert_eq!(r.misses, 0);
        assert_eq!(r.transfers, 6 * 2 * 128 * 1024);
        assert!(r.passed());
        // Too few transfers never count as a pass.
        let small = run(1000, None);
        assert!(!small.passed());
    }

    #[test]
    fn a_fault_of_one_in_a_thousand_is_caught_and_named() {
        let r = run(128 * 1024, Some(1000));
        assert!(r.misses > 700, "{}", r.misses);
        assert!(!r.passed());
        let (p, k, want, got) = r.first.unwrap();
        assert_eq!((p, k), (0, 999));
        assert_eq!(got, want ^ 0x10);
        // One fault in the whole check is caught too.
        assert!(!run(128 * 1024, Some(500_000)).passed());
    }

    #[test]
    fn the_faster_timing_needs_both_checks_and_the_default() {
        let ok = run(32 * 1024, None);
        let bad = run(32 * 1024, Some(1000));
        assert_eq!(pick(&ok, &ok, Some(&ok)), (FAST, true));
        // Cold only, or warm failed, or never warm-checked: the default.
        assert_eq!(pick(&ok, &ok, None), (DEFAULT, true));
        assert_eq!(pick(&ok, &ok, Some(&bad)), (DEFAULT, true));
        assert_eq!(pick(&ok, &bad, Some(&ok)), (DEFAULT, true));
        // The default itself failing: the default kept, the fault flagged.
        assert_eq!(pick(&bad, &ok, Some(&ok)), (DEFAULT, false));
    }

    #[test]
    fn unreadable_words_are_misses() {
        let r = check(0, 32 * 1024, |_, _| {}, |_| None, || {});
        assert_eq!(r.misses, 6 * 32 * 1024);
    }
}
