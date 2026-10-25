//! The socket's timing for one card (the owner's ruling M, reminder R7,
//! 2026-10-24; docs/research/storage-speed.md). Windows Mobile programs one
//! common-memory timing for every card (`MCMEM` `0x00020508`: a 260 ns
//! cycle at a 208 MHz memory clock). Here each card gets its own: the
//! driver finds the shortest timing it reads right at ([`shorter`], one
//! field a step), then backs off 1.5x from it ([`with_margin`]). A card's
//! CIS rating ([`cis_speed_ns`]) is logged, not trusted: an older card on
//! the X51v rated itself 250 ns and read right at 124 (ruling A, the rating
//! with a margin, made it 0.68x Windows Mobile's speed).
//!
//! Pure: the ladder and the margin; the driver reads the card at each step.

/// The margin over the measured limit, in tenths: 1.5x.
pub const MARGIN_TENTHS: u32 = 15;

/// A timing's fields, in the order the search shortens them: the strobe
/// (the most time a step), the hold, the set-up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    /// `ASST`, 3 cycles a step.
    Strobe,
    /// `HOLD`.
    Hold,
    /// `SET`.
    SetUp,
}

/// The fields in search order.
pub const FIELDS: [Field; 3] = [Field::Strobe, Field::Hold, Field::SetUp];

/// `t` one step shorter in `field`, or `None` at its zero.
#[must_use]
pub const fn shorter(t: Timing, field: Field) -> Option<Timing> {
    match field {
        Field::Strobe if t.asst > 0 => Some(Timing {
            asst: t.asst - 1,
            ..t
        }),
        Field::Hold if t.hold > 0 => Some(Timing {
            hold: t.hold - 1,
            ..t
        }),
        Field::SetUp if t.set > 0 => Some(Timing {
            set: t.set - 1,
            ..t
        }),
        _ => None,
    }
}

/// The timing to use for a card whose shortest right reads came at
/// `limit`: each part of the cycle (the set-up `SET` + 2, the strobe
/// 3 x `ASST` + 6, the hold) at least [`MARGIN_TENTHS`] of the limit's and
/// at least one step longer (a part the card needed none of still gets
/// some), so the access and the whole cycle are 1.5x or more; and whether
/// a field had to be clamped to [`Timing::SLOWEST`]'s.
#[must_use]
pub fn with_margin(limit: Timing) -> (Timing, bool) {
    let grow = |x: u32| (x * MARGIN_TENTHS).div_ceil(10);
    let set = grow(limit.set + 2).saturating_sub(2).max(limit.set + 1);
    let asst = grow(3 * limit.asst + 6)
        .saturating_sub(6)
        .div_ceil(3)
        .max(limit.asst + 1);
    let hold = grow(limit.hold).max(limit.hold + 1);
    let s = Timing::SLOWEST;
    let t = Timing {
        set: set.min(s.set),
        asst: asst.min(s.asst),
        hold: hold.min(s.hold),
    };
    (t, set > s.set || asst > s.asst || hold > s.hold)
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

#[cfg(test)]
mod tests {
    use super::*;

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

    /// The ladder: one field, one step, down to zero, never below.
    #[test]
    fn the_ladder_shortens_one_field_a_step() {
        let wm = Timing::WINDOWS_MOBILE;
        assert_eq!(shorter(wm, Field::Strobe), Some(Timing { asst: 9, ..wm }));
        assert_eq!(shorter(wm, Field::Hold), Some(Timing { hold: 7, ..wm }));
        assert_eq!(shorter(wm, Field::SetUp), Some(Timing { set: 7, ..wm }));
        let zero = Timing {
            set: 0,
            asst: 0,
            hold: 0,
        };
        for f in FIELDS {
            assert_eq!(shorter(zero, f), None);
        }
        // Windows Mobile's timing to zero: 10 + 8 + 8 steps, each shorter.
        let (mut t, mut n) = (wm, 0);
        for f in FIELDS {
            while let Some(next) = shorter(t, f) {
                assert!(next.cycle_cycles() < t.cycle_cycles());
                t = next;
                n += 1;
            }
        }
        assert_eq!((t, n), (zero, 26));
    }

    /// Every part at least 1.5x and one step over the limit, the access and
    /// the cycle with them, and no field could be one less.
    #[test]
    fn the_margin_is_one_and_a_half_times_the_limit_and_no_more() {
        let parts = |t: Timing| [t.set + 2, 3 * t.asst + 6, t.hold];
        for set in 0..20 {
            for asst in 0..12 {
                for hold in 0..20 {
                    let limit = Timing { set, asst, hold };
                    let (t, clamped) = with_margin(limit);
                    assert!(!clamped, "{limit:?}");
                    let ok = |t: Timing| {
                        parts(t)
                            .iter()
                            .zip(parts(limit))
                            .all(|(&p, l)| p * 10 >= l * 15)
                            && t.set > limit.set
                            && t.asst > limit.asst
                            && t.hold > limit.hold
                    };
                    assert!(ok(t), "{limit:?} -> {t:?}");
                    assert!(t.access_cycles() * 10 >= limit.access_cycles() * 15);
                    assert!(t.cycle_cycles() * 10 >= limit.cycle_cycles() * 15);
                    for less in [
                        Timing {
                            set: t.set - 1,
                            ..t
                        },
                        Timing {
                            asst: t.asst - 1,
                            ..t
                        },
                        Timing {
                            hold: t.hold - 1,
                            ..t
                        },
                    ] {
                        assert!(!ok(less), "{limit:?}: {less:?} would do");
                    }
                }
            }
        }
    }

    /// The two cards measured on the X51v (2026-10-24, rp-storfast C5):
    /// the 8 GB card right at set 2, strobe 1, hold 4 (17 cycles); the older
    /// 1 GB one at set 8, strobe 1, hold 7 (26). With the margin: 27 and 41
    /// cycles, against Windows Mobile's 54 for both.
    #[test]
    fn the_owners_cards_with_the_margin() {
        let card = |set, asst, hold| with_margin(Timing { set, asst, hold }).0;
        assert_eq!(card(2, 1, 4).cycle_cycles(), 27);
        assert_eq!(card(8, 1, 7).cycle_cycles(), 41);
    }

    /// A limit at the socket's slowest is clamped there.
    #[test]
    fn a_limit_near_the_slowest_is_clamped() {
        let (t, clamped) = with_margin(Timing::SLOWEST);
        assert!(clamped);
        assert_eq!(t, Timing::SLOWEST);
    }
}
