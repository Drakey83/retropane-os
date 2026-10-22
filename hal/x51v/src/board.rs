//! The board as the loader first found it (quality audit C39, the owner's
//! option B, 2026-10-21): the state test images change and a crash (or a
//! careless return) would leave changed. At its first start from Windows
//! Mobile (HaRET) the resident loader records it; every time an image
//! comes back, crashed or not, the loader puts back whatever differs. One
//! place covers every image, today's and tomorrow's.
//!
//! What's kept, each with its owner in the hardware docs:
//!
//! - CPLD `0x14` bit 1 (the speaker amplifier) and `0x1C` bits 1-3 (the
//!   codec's power; `0x1C` bits 2-3, the USB/dock side): docs/hw/cpld.md;
//! - the backlight's PWM0 (`PWM_CTRL0`, `PWM_PWDUTY0`, `PWM_PERVAL0`);
//! - the 2700G's gamma: `GFXGAM0`-`16` and `GSCTRL` bit 26;
//! - the 2700G's `MBXCLK` and `M24CLK` (the MBX cores' and the video
//!   decoder's clocks).
//!
//! The record is pure data here (packed into words with a check); the
//! device side, which reads and writes it in its reserved RAM, is in
//! [`crate::devices`].

/// CPLD `0x14`'s bits kept (the speaker amplifier).
pub const CPLD14_BITS: u16 = 0x0002;
/// CPLD `0x1C`'s bits kept (the codec's power, the USB/dock bits).
pub const CPLD1C_BITS: u16 = 0x000E;
/// Gamma table words (`GFXGAM0`-`16`).
pub const GAMMA_WORDS: usize = 17;
/// `GSCTRL`'s gamma enable.
pub const GSCTRL_GAMMA: u32 = 1 << 26;

/// The board state the loader keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Board {
    /// CPLD `0x14` (only [`CPLD14_BITS`] matter).
    pub cpld14: u16,
    /// CPLD `0x1C` (only [`CPLD1C_BITS`] matter).
    pub cpld1c: u16,
    /// PWM0 control, duty and period.
    pub pwm: [u32; 3],
    /// `GSCTRL` (only [`GSCTRL_GAMMA`] matters).
    pub gsctrl: u32,
    /// `GFXGAM0`-`16`.
    pub gamma: [u32; GAMMA_WORDS],
    /// `MBXCLK`.
    pub mbxclk: u32,
    /// `M24CLK`.
    pub m24clk: u32,
}

/// What differs between two boards, one bit a kind (for the loader's
/// report: which kinds it put back).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Differs(pub u8);

impl Differs {
    /// The amplifier bit.
    pub const AMP: u8 = 1;
    /// The codec / USB bits of `0x1C`.
    pub const CPLD1C: u8 = 2;
    /// The backlight.
    pub const PWM: u8 = 4;
    /// The gamma table or its enable.
    pub const GAMMA: u8 = 8;
    /// `MBXCLK`.
    pub const MBXCLK: u8 = 16;
    /// `M24CLK`.
    pub const M24CLK: u8 = 32;
    /// The kinds' names, in bit order, for the report.
    pub const NAMES: [&'static str; 6] = [
        "amplifier",
        "codec/USB bits",
        "backlight",
        "gamma",
        "MBXCLK",
        "M24CLK",
    ];

    /// Nothing differs.
    #[must_use]
    pub const fn none(self) -> bool {
        self.0 == 0
    }
}

impl Board {
    /// Which kinds of `now` differ from this record.
    #[must_use]
    pub fn differs(&self, now: &Board) -> Differs {
        let mut d = 0;
        if (self.cpld14 ^ now.cpld14) & CPLD14_BITS != 0 {
            d |= Differs::AMP;
        }
        if (self.cpld1c ^ now.cpld1c) & CPLD1C_BITS != 0 {
            d |= Differs::CPLD1C;
        }
        if self.pwm != now.pwm {
            d |= Differs::PWM;
        }
        if (self.gsctrl ^ now.gsctrl) & GSCTRL_GAMMA != 0 || self.gamma != now.gamma {
            d |= Differs::GAMMA;
        }
        if self.mbxclk != now.mbxclk {
            d |= Differs::MBXCLK;
        }
        if self.m24clk != now.m24clk {
            d |= Differs::M24CLK;
        }
        Differs(d)
    }
}

/// Words a record takes, check included.
pub const WORDS: usize = 4 + 3 + 1 + GAMMA_WORDS + 2 + 1;
/// The record's first word.
pub const MAGIC: u32 = u32::from_le_bytes(*b"RPBD");

/// The check word: every other word summed, rotated as it goes (a cleared
/// or half-written record doesn't pass).
fn check(words: &[u32]) -> u32 {
    words
        .iter()
        .fold(0x5A5A_5A5A_u32, |s, &w| s.rotate_left(5) ^ w)
}

/// `b` as the words kept in RAM: magic, a version, the two CPLD values,
/// PWM, `GSCTRL`, gamma, the clocks, the check.
#[must_use]
pub fn pack(b: &Board) -> [u32; WORDS] {
    let mut w = [0u32; WORDS];
    w[0] = MAGIC;
    w[1] = 1;
    w[2] = u32::from(b.cpld14);
    w[3] = u32::from(b.cpld1c);
    w[4..7].copy_from_slice(&b.pwm);
    w[7] = b.gsctrl;
    w[8..8 + GAMMA_WORDS].copy_from_slice(&b.gamma);
    w[8 + GAMMA_WORDS] = b.mbxclk;
    w[9 + GAMMA_WORDS] = b.m24clk;
    w[WORDS - 1] = check(&w[..WORDS - 1]);
    w
}

/// The record in `w`, or `None` if it isn't one (no magic, another
/// version, a failed check).
#[must_use]
pub fn unpack(w: &[u32; WORDS]) -> Option<Board> {
    if w[0] != MAGIC || w[1] != 1 || w[WORDS - 1] != check(&w[..WORDS - 1]) {
        return None;
    }
    Some(Board {
        cpld14: w[2] as u16,
        cpld1c: w[3] as u16,
        pwm: [w[4], w[5], w[6]],
        gsctrl: w[7],
        gamma: core::array::from_fn(|k| w[8 + k]),
        mbxclk: w[8 + GAMMA_WORDS],
        m24clk: w[9 + GAMMA_WORDS],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board() -> Board {
        Board {
            cpld14: 0x00F0,
            cpld1c: 0x000F,
            pwm: [0x3F, 0x200, 0x3FF],
            gsctrl: 0x0600_780F,
            gamma: core::array::from_fn(|k| k as u32 * 0x0010_1010),
            mbxclk: 5,
            m24clk: 0x9,
        }
    }

    #[test]
    fn a_record_round_trips_and_a_damaged_one_is_refused() {
        let b = board();
        let w = pack(&b);
        assert_eq!(unpack(&w), Some(b));
        for k in 0..WORDS {
            let mut bad = w;
            bad[k] ^= 1;
            assert_eq!(unpack(&bad), None, "word {k} flipped");
        }
        assert_eq!(unpack(&[0; WORDS]), None, "cleared");
    }

    #[test]
    fn only_the_kept_bits_count_and_each_kind_is_named() {
        let b = board();
        let mut n = b;
        // Bits outside the kept ones don't count.
        n.cpld14 ^= 0x0001;
        n.cpld1c ^= 0x0001;
        n.gsctrl ^= 0x0000_0001;
        assert!(b.differs(&n).none());
        n.cpld14 ^= CPLD14_BITS;
        n.cpld1c ^= 0x0008;
        n.pwm[1] = 0;
        n.gamma[16] ^= 1;
        n.mbxclk = 6;
        n.m24clk = 0;
        assert_eq!(b.differs(&n).0, 0x3F);
        let mut g = b;
        g.gsctrl ^= GSCTRL_GAMMA;
        assert_eq!(b.differs(&g).0, Differs::GAMMA);
        assert_eq!(Differs::NAMES.len(), 6);
    }
}
