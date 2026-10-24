//! The socket's timing for one card (the owner's ruling A, reminder R7,
//! 2026-10-24; docs/research/storage-speed.md). Windows Mobile programs one
//! common-memory timing for every card (`MCMEM` `0x00020508`: a 260 ns
//! cycle at a 208 MHz memory clock). Here each card gets the timing its own
//! CIS rates it for, with a margin: faster for a fast card, and slower than
//! Windows Mobile's for a card rated slower than that table (the old cards
//! and Microdrives the owner expects), which Windows Mobile could misread.
//!
//! Pure: what the card says ([`cis_speed_ns`]) to the register's value
//! ([`Timing::mcmem`]). The driver programs it only after a read at the new
//! timing matches the same read at the old one, and keeps the old one
//! otherwise.

/// The CompactFlash specification's common-memory read timing per speed
/// class, in ns: the cycle (tc(R), also the address access time ta(A)),
/// the output-enable access ta(OE), the address set-up tsu(A) and hold
/// th(A). The 250 ns row is in the CF Association's specification (Rev
/// 2.0) and makers' data sheets; the 120, 100 and 80 ns rows come from a
/// maker's data sheet reproducing the specification's table (CF-I20 v1.9),
/// to confirm against the CF 4.1 specification itself: [`MARGIN_TENTHS`] and
/// the driver's read-back check stand between them and the card.
// inherited: R7, the CompactFlash specification via card data sheets (storage-speed.md)
const CLASSES: [Class; 4] = [
    Class::new(250, 125, 30, 20),
    Class::new(120, 60, 15, 15),
    Class::new(100, 50, 10, 15),
    Class::new(80, 45, 10, 10),
];

/// The margin over every minimum, in tenths: 1.5x.
pub const MARGIN_TENTHS: u32 = 15;

/// One speed class's minimum times, ns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Class {
    cycle: u32,
    oe: u32,
    setup: u32,
    hold: u32,
}

impl Class {
    const fn new(cycle: u32, oe: u32, setup: u32, hold: u32) -> Self {
        Self {
            cycle,
            oe,
            setup,
            hold,
        }
    }

    /// The class a card rated `rated` ns is held to: the fastest class no
    /// faster than the card (a card rated 90 ns is held to 100 ns); slower
    /// than 250 ns, the 250 ns class's times stretched in proportion.
    fn for_rating(rated: u32) -> Self {
        if let Some(c) = CLASSES.iter().rev().find(|c| c.cycle >= rated) {
            return *c;
        }
        let s = CLASSES[0];
        let scale = |t: u32| (t * rated).div_ceil(s.cycle);
        Self::new(rated, scale(s.oe), scale(s.setup), scale(s.hold))
    }
}

/// What a `CISTPL_DEVICE` (or `CISTPL_DEVICE_OC`) tuple's body rates the
/// card's common memory at, in ns: the device ID byte's speed field (bits
/// 2-0: 1 = 250, 2 = 200, 3 = 150, 4 = 100 ns; 7 = an extended speed byte
/// follows: bits 6-3 a mantissa, 2-0 a power of ten from 1 ns, bit 7 more
/// extension bytes after it, ignored). `None` for no speed (0), a reserved
/// code (5, 6), a reserved mantissa, a missing byte, or a rating outside
/// 10 ns to 100 us.
#[must_use]
pub fn cis_speed_ns(body: &[u8]) -> Option<u32> {
    let id = *body.first()?;
    let ns = match id & 7 {
        1 => 250,
        2 => 200,
        3 => 150,
        4 => 100,
        7 => {
            let ext = *body.get(1)?;
            // Tenths: 1.0 1.2 1.3 1.5 2.0 2.5 3.0 3.5 4.0 4.5 5.0 5.5 6.0 7.0 8.0.
            const MANTISSA: [u32; 16] = [
                0, 10, 12, 13, 15, 20, 25, 30, 35, 40, 45, 50, 55, 60, 70, 80,
            ];
            let m = MANTISSA[usize::from((ext >> 3) & 0xF)];
            let exp = u32::from(ext & 7);
            if m == 0 || exp > 5 {
                return None;
            }
            m * 10u32.pow(exp) / 10
        }
        _ => return None,
    };
    (10..=100_000).contains(&ns).then_some(ns)
}

/// A common-memory timing in memory-clock cycles, as `MCMEM` holds it
/// (PXA27x manual, tables 6-34, 6-35: set-up SET + 2, read strobe
/// 3 x ASST + 6, write strobe 3 x ASST + 5, hold HOLD).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timing {
    /// Address set-up (bits 6-0).
    pub set: u32,
    /// Strobe (bits 11-7).
    pub asst: u32,
    /// Address hold (bits 19-14).
    pub hold: u32,
}

impl Timing {
    /// The slowest timing the socket has: every field at its largest.
    pub const SLOWEST: Self = Self {
        set: 0x7F,
        asst: 0x1F,
        hold: 0x3F,
    };

    /// Windows Mobile's common-memory timing on the X51v at 208 MHz
    /// (`MCMEM1` `0x00020508`, storage-speed.md A1).
    pub const WINDOWS_MOBILE: Self = Self {
        set: 8,
        asst: 10,
        hold: 8,
    };

    /// The register's value.
    #[must_use]
    pub const fn mcmem(self) -> u32 {
        self.hold << 14 | self.asst << 7 | self.set
    }

    /// A register value's fields.
    #[must_use]
    pub const fn from_mcmem(v: u32) -> Self {
        Self {
            set: v & 0x7F,
            asst: v >> 7 & 0x1F,
            hold: v >> 14 & 0x3F,
        }
    }

    /// Cycles from the address to the data's capture: set-up and read strobe.
    #[must_use]
    pub const fn access_cycles(self) -> u32 {
        self.set + 2 + 3 * self.asst + 6
    }

    /// The whole read cycle in memory-clock cycles.
    #[must_use]
    pub const fn cycle_cycles(self) -> u32 {
        self.access_cycles() + self.hold
    }
}

/// The fastest timing that meets a card rated `rated_ns` with
/// [`MARGIN_TENTHS`] on every minimum, at a memory clock of `clk_ps`
/// picoseconds a cycle, and whether it had to be clamped to
/// [`Timing::SLOWEST`] (a card slower than the socket can wait for). The
/// socket's longest read strobe is 3 x 31 + 6 = 99 cycles (476 ns at
/// 208 MHz): enough for the margin on cards rated up to ~630 ns, past the
/// CF specification's slowest class (250 ns); a slower card gets the
/// longest timing the socket has.
#[must_use]
pub fn for_card(rated_ns: u32, clk_ps: u32) -> (Timing, bool) {
    let c = Class::for_rating(rated_ns);
    // Cycles covering `ns` x the margin, rounded up.
    let cycles = |ns: u32| {
        (u64::from(ns) * u64::from(MARGIN_TENTHS) * 100).div_ceil(u64::from(clk_ps.max(1)))
    };
    let need = |ns: u32| u32::try_from(cycles(ns)).unwrap_or(u32::MAX);
    let mut set = need(c.setup).saturating_sub(2);
    let mut asst = need(c.oe).saturating_sub(6).div_ceil(3);
    // The data is captured at the strobe's end: no sooner than the address
    // access time (the cycle) after the address. The strobe (at most 31
    // steps of 3) is lengthened first, then the set-up (to 127): a slow
    // card's longer wait doesn't run out at the strobe's limit.
    let access = need(c.cycle);
    if set + 2 + 3 * asst + 6 < access {
        let short = access - (set + 2 + 3 * asst + 6);
        asst += short
            .div_ceil(3)
            .min(Timing::SLOWEST.asst.saturating_sub(asst));
        let have = set + 2 + 3 * asst + 6;
        if have < access {
            set += access - have;
        }
    }
    let mut hold = need(c.hold);
    let t = Timing { set, asst, hold };
    if t.cycle_cycles() < need(c.cycle) {
        hold += need(c.cycle) - t.cycle_cycles();
    }
    let t = Timing { set, asst, hold };
    let s = Timing::SLOWEST;
    if t.set > s.set || t.asst > s.asst || t.hold > s.hold {
        (s, true)
    } else {
        (t, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The X51v's memory clock with `CCCR` A set (208 MHz).
    const CLK_208: u32 = 4_808;

    #[test]
    fn the_owners_card_reads_80_ns() {
        // storage-speed.md A2: CISTPL_DEVICE `df 79 01 ff`.
        assert_eq!(cis_speed_ns(&[0xDF, 0x79, 0x01, 0xFF]), Some(80));
    }

    #[test]
    fn every_speed_code_and_bad_input() {
        for (id, ns) in [(1, 250), (2, 200), (3, 150), (4, 100)] {
            assert_eq!(cis_speed_ns(&[0xD0 | id]), Some(ns), "code {id}");
        }
        for id in [0, 5, 6] {
            assert_eq!(cis_speed_ns(&[0xD0 | id]), None, "code {id}");
        }
        assert_eq!(cis_speed_ns(&[]), None);
        assert_eq!(cis_speed_ns(&[0xD7]), None, "extended byte missing");
        // Mantissa 0 is reserved; exponent 6 (1 ms) is past 100 us.
        assert_eq!(cis_speed_ns(&[0xD7, 0x01]), None);
        assert_eq!(cis_speed_ns(&[0xD7, 0x0E]), None);
        // 6.0 x 100 ns = 600 ns; bit 7 set (more bytes) is ignored.
        assert_eq!(cis_speed_ns(&[0xD7, 0xEA]), Some(600));
        assert_eq!(cis_speed_ns(&[0xD7, 0x6A]), Some(600));
        // 1.0 x 1 ns: below 10 ns, refused.
        assert_eq!(cis_speed_ns(&[0xD7, 0x08]), None);
    }

    #[test]
    fn windows_mobiles_register_round_trips() {
        assert_eq!(Timing::WINDOWS_MOBILE.mcmem(), 0x0002_0508);
        assert_eq!(Timing::from_mcmem(0x0002_0508), Timing::WINDOWS_MOBILE);
        assert_eq!(Timing::from_mcmem(Timing::SLOWEST.mcmem()), Timing::SLOWEST);
        assert_eq!(Timing::WINDOWS_MOBILE.cycle_cycles(), 54);
    }

    /// Every minimum met with the margin, and no field could be one less.
    fn assert_tight(rated: u32, clk: u32) {
        let (t, clamped) = for_card(rated, clk);
        assert!(!clamped, "{rated} ns");
        let c = Class::for_rating(rated);
        let ok = |t: Timing| {
            let ns = |cycles: u32| u64::from(cycles) * u64::from(clk);
            let need = |min: u32| u64::from(min) * u64::from(MARGIN_TENTHS) * 100;
            ns(t.set + 2) >= need(c.setup)
                && ns(3 * t.asst + 6) >= need(c.oe)
                && ns(t.access_cycles()) >= need(c.cycle)
                && ns(t.hold) >= need(c.hold)
                && ns(t.cycle_cycles()) >= need(c.cycle)
        };
        assert!(ok(t), "{rated} ns: {t:?}");
        for less in [
            Timing {
                set: t.set.wrapping_sub(1),
                ..t
            },
            Timing {
                asst: t.asst.wrapping_sub(1),
                ..t
            },
            Timing {
                hold: t.hold.wrapping_sub(1),
                ..t
            },
        ] {
            let valid = less.set <= 0x7F && less.asst <= 0x1F && less.hold <= 0x3F;
            assert!(!valid || !ok(less), "{rated} ns: {less:?} would do");
        }
    }

    #[test]
    fn each_class_is_met_with_the_margin_and_no_more() {
        for rated in [80, 100, 120, 150, 200, 250, 300, 600] {
            assert_tight(rated, CLK_208);
        }
        // At the 104 MHz memory clock too.
        for rated in [80, 250, 600] {
            assert_tight(rated, 9_615);
        }
    }

    #[test]
    fn a_fast_card_beats_windows_mobile_and_a_slow_one_gets_more_time() {
        let wm = Timing::WINDOWS_MOBILE.cycle_cycles();
        let (fast, _) = for_card(80, CLK_208);
        // The owner's 80 ns card: 29 cycles (139 ns: its 120 ns with the
        // margin, plus the hold) against 54 (260 ns), 1.87x faster.
        assert_eq!(
            fast,
            Timing {
                set: 2,
                asst: 5,
                hold: 4
            }
        );
        assert!(fast.cycle_cycles() * 3 < wm * 2, "{fast:?}");
        let (slow, _) = for_card(250, CLK_208);
        assert!(slow.cycle_cycles() > wm, "{slow:?}");
        let (slower, _) = for_card(600, CLK_208);
        assert!(slower.cycle_cycles() > slow.cycle_cycles());
    }

    #[test]
    fn slower_ratings_never_get_shorter_times() {
        let mut last = Timing {
            set: 0,
            asst: 0,
            hold: 0,
        };
        // Every rating to the socket's limit (~630 ns at 208 MHz), then
        // past it: clamped, and never shorter than below it.
        for rated in (10..=700).step_by(5) {
            let (t, clamped) = for_card(rated, CLK_208);
            assert_eq!(clamped, rated > 630, "{rated} ns");
            assert!(t.access_cycles() >= last.access_cycles(), "{rated} ns");
            assert!(t.cycle_cycles() >= last.cycle_cycles(), "{rated} ns");
            last = t;
        }
    }

    #[test]
    fn a_card_slower_than_the_socket_gets_its_slowest() {
        let (t, clamped) = for_card(100_000, CLK_208);
        assert!(clamped);
        assert_eq!(t, Timing::SLOWEST);
    }
}
