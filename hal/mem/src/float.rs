//! Single-precision float arithmetic for a CPU without an FPU (the
//! PXA270): add, subtract and multiply on IEEE 754 bits, rounded to
//! nearest-even exactly as the standard says, for the C code of the game
//! ports (R282: in Half-Life's play on the X51v the compiler's generic
//! routines were 41% of a frame, `__addsf3` 21.1% and `__mulsf3` 19.5%).
//!
//! Iterate, don't imitate (rule 0). Every soft-float target links the
//! compiler's generic routines (compiler-rt's C, or Rust's port of it:
//! one code for every float width, special cases tested first) or GCC's
//! ARM assembly (whose licence does not fit RetroPane). Ours is written
//! for one width and this CPU: the common case (two normal numbers) is a
//! straight run with the special cases folded into it or tested once at
//! the end, the leading-zero count of ARMv5 (`clz`) normalises in one
//! step, the 24 x 24-bit product is one `umull`, and the rounding is one
//! add that carries into the exponent by itself (an overflow to infinity
//! included). Written in Rust and not assembly so that every result is
//! checked on the PC against the PC's own IEEE hardware, millions of
//! pairs of every kind; assembly only where the X51v shows the compiler
//! losing. Any NaN in gives a quiet NaN out (which NaN's payload is not
//! promised; nothing in a game reads it).

const SIGN: u32 = 0x8000_0000;
const INF: u32 = 0x7F80_0000;
const QNAN: u32 = 0x7FC0_0000;
const QUIET: u32 = 0x0040_0000;
const MANT: u32 = 0x007F_FFFF;
const ONE: u32 = 0x0080_0000;

/// `a + b` (IEEE single bits).
#[inline]
#[must_use]
pub fn add(a: u32, b: u32) -> u32 {
    // The larger magnitude first: its sign is the result's, and a NaN or
    // an infinity, if there is one, is in it.
    let (x, y) = if (a << 1) < (b << 1) { (b, a) } else { (a, b) };
    let ex = (x << 1) >> 24;
    if ex == 0xFF {
        return add_special(x, y);
    }
    let ey = (y << 1) >> 24;
    // A subnormal (exponent field 0) has no leading one and the smallest
    // normal's scale: one code serves both.
    let mx = ((x & MANT) | if ex != 0 { ONE } else { 0 }) << 6;
    let my = ((y & MANT) | if ey != 0 { ONE } else { 0 }) << 6;
    let (ex, ey) = (ex.max(1), ey.max(1));
    let d = ex - ey;
    // Below half the spacing of the numbers around x, whatever its sign:
    // x is the nearest (proof in the tests' notes).
    if d > 25 {
        return x;
    }
    // y lined up with x, keeping 6 more bits and whether any bit went.
    let my = if d == 0 {
        my
    } else {
        (my >> d) | u32::from(my << (32 - d) != 0)
    };
    let sign = x & SIGN;
    let (mut m, mut e);
    if (x ^ y) & SIGN == 0 {
        m = mx + my;
        e = ex;
        if m >= 1 << 30 {
            m = (m >> 1) | (m & 1);
            e += 1;
            if e == 0xFF {
                return sign | INF;
            }
        }
    } else {
        m = mx - my;
        if m == 0 {
            // x - x is +0 when rounding to nearest.
            return 0;
        }
        // The leading one back to bit 29 (a cancellation of more than
        // one bit only happens when d <= 1, where no bit was lost).
        let lz = m.leading_zeros() - 2;
        m <<= lz;
        let e2 = ex as i32 - lz as i32;
        if e2 < 1 {
            // Subnormal: back to the smallest normal's scale.
            let s = (1 - e2) as u32;
            m = (m >> s) | u32::from(m << (32 - s) != 0);
            e = 1;
        } else {
            e = e2 as u32;
        }
    }
    round(sign, e, m)
}

/// `a - b`.
#[inline]
#[must_use]
pub fn sub(a: u32, b: u32) -> u32 {
    add(a, b ^ SIGN)
}

/// A sign, a biased exponent `e` (at least 1) and a significand `m` with
/// its leading one (if any) at bit 29 and 6 bits below the result's last,
/// rounded to nearest-even and packed. The exponent field is `e - 1` plus
/// the leading one, so a rounding that carries out of the significand
/// raises the exponent, and one that reaches 255 is infinity.
#[inline(always)]
fn round(sign: u32, e: u32, m: u32) -> u32 {
    let r = ((e - 1) << 23) + (m >> 6);
    let g = m & 0x3F;
    let up = u32::from(g > 0x20 || (g == 0x20 && r & 1 != 0));
    sign | (r + up)
}

/// `add` with an infinity or a NaN in `x` (the larger magnitude).
#[cold]
fn add_special(x: u32, y: u32) -> u32 {
    if x << 1 > INF << 1 {
        return x | QUIET;
    }
    // x is an infinity; y the same infinity of the other sign makes NaN.
    if y << 1 == INF << 1 && (x ^ y) & SIGN != 0 {
        return QNAN;
    }
    x
}

/// `a * b` (IEEE single bits).
#[inline]
#[must_use]
pub fn mul(a: u32, b: u32) -> u32 {
    let sign = (a ^ b) & SIGN;
    let ea = (a >> 23) & 0xFF;
    let eb = (b >> 23) & 0xFF;
    // Both normal: one test for both ends of the range.
    if ea.wrapping_sub(1) >= 0xFE || eb.wrapping_sub(1) >= 0xFE {
        return mul_special(a, b);
    }
    let p = u64::from((a & MANT) | ONE) * u64::from((b & MANT) | ONE);
    // p is in [2^46, 2^48): its leading one to bit 47.
    let top = (p >> 47) as u32;
    let p = p << (1 - top);
    let e = (ea + eb + top) as i32 - 127;
    pack_wide(sign, e, p)
}

/// A sign, a biased exponent and a 48-bit significand with its leading
/// one at bit 47, rounded and packed: infinity above the range, a
/// subnormal or zero below it.
#[inline(always)]
fn pack_wide(sign: u32, e: i32, p: u64) -> u32 {
    if e >= 0xFF {
        return sign | INF;
    }
    let (e, p) = if e >= 1 {
        (e as u32, p)
    } else {
        // Subnormal: shifted to the smallest normal's scale, the bits
        // that go kept as one.
        let s = (1 - e) as u32;
        if s > 48 {
            (1, u64::from(p != 0))
        } else {
            (1, (p >> s) | u64::from(p << (64 - s) != 0))
        }
    };
    // The 24 bits kept, then the 24 below as the rounding's.
    let hi = (p >> 24) as u32;
    let lo = p as u32 & 0x00FF_FFFF;
    let r = ((e - 1) << 23) + hi;
    let up = u32::from(lo > 0x0080_0000 || (lo == 0x0080_0000 && r & 1 != 0));
    sign | (r + up)
}

/// `mul` with a zero, a subnormal, an infinity or a NaN.
#[cold]
fn mul_special(a: u32, b: u32) -> u32 {
    let sign = (a ^ b) & SIGN;
    let (aa, ba) = (a & !SIGN, b & !SIGN);
    if aa > INF {
        return a | QUIET;
    }
    if ba > INF {
        return b | QUIET;
    }
    if aa == INF || ba == INF {
        // Infinity times zero is NaN, times anything else infinity.
        return if aa == 0 || ba == 0 { QNAN } else { sign | INF };
    }
    if aa == 0 || ba == 0 {
        return sign;
    }
    // A subnormal: its leading one brought up, its exponent lowered.
    let unpack = |v: u32| -> (i32, u32) {
        let e = (v >> 23) as i32;
        if e == 0 {
            let lz = (v & MANT).leading_zeros() - 8;
            (1 - lz as i32, (v & MANT) << lz)
        } else {
            (e, (v & MANT) | ONE)
        }
    };
    let (ea, ma) = unpack(aa);
    let (eb, mb) = unpack(ba);
    let p = u64::from(ma) * u64::from(mb);
    let top = (p >> 47) as u32;
    let p = p << (1 - top);
    pack_wide(sign, ea + eb + top as i32 - 127, p)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The PC's own IEEE result, any NaN as one.
    fn same(got: u32, want: f32) -> bool {
        let g = f32::from_bits(got);
        if want.is_nan() {
            g.is_nan()
        } else {
            got == want.to_bits()
        }
    }

    fn check(a: u32, b: u32) {
        let (fa, fb) = (f32::from_bits(a), f32::from_bits(b));
        assert!(
            same(add(a, b), fa + fb),
            "{a:#010x} + {b:#010x}: {:#010x}, want {:#010x}",
            add(a, b),
            (fa + fb).to_bits()
        );
        assert!(
            same(sub(a, b), fa - fb),
            "{a:#010x} - {b:#010x}: {:#010x}, want {:#010x}",
            sub(a, b),
            (fa - fb).to_bits()
        );
        assert!(
            same(mul(a, b), fa * fb),
            "{a:#010x} * {b:#010x}: {:#010x}, want {:#010x}",
            mul(a, b),
            (fa * fb).to_bits()
        );
        // Any NaN out is quiet.
        for r in [add(a, b), sub(a, b), mul(a, b)] {
            if f32::from_bits(r).is_nan() {
                assert!(r & QUIET != 0, "{r:#010x} is a signalling NaN");
            }
        }
    }

    /// A small, fast generator (xorshift), the same numbers every run.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 16) as u32
        }
    }

    /// Values at every edge: zeros, the subnormals' ends, the normals'
    /// ends, one and its neighbours, infinities, NaNs of both kinds.
    const EDGES: [u32; 20] = [
        0,
        1,
        2,
        0x0000_0003,
        0x007F_FFFF,
        0x0080_0000,
        0x0080_0001,
        0x00FF_FFFF,
        0x3F7F_FFFF,
        0x3F80_0000,
        0x3F80_0001,
        0x3FFF_FFFF,
        0x4B00_0000,
        0x4B7F_FFFF,
        0x7F7F_FFFE,
        0x7F7F_FFFF,
        0x7F80_0000,
        0x7F80_0001,
        0x7FC0_0000,
        0x7FFF_FFFF,
    ];

    #[test]
    fn every_edge_with_every_edge_both_signs() {
        for &a in &EDGES {
            for &b in &EDGES {
                for (sa, sb) in [(0, 0), (SIGN, 0), (0, SIGN), (SIGN, SIGN)] {
                    check(a | sa, b | sb);
                }
            }
        }
    }

    #[test]
    fn random_bits_of_every_kind() {
        let mut r = Rng(0x9E37_79B9_7F4A_7C15);
        for _ in 0..2_000_000 {
            check(r.next(), r.next());
        }
    }

    #[test]
    fn near_numbers_cancel_and_round_exactly() {
        // Close exponents and close significands: cancellation, the
        // carries, the ties and the subnormal boundary.
        let mut r = Rng(0x2545_F491_4F6C_DD1D);
        for _ in 0..2_000_000 {
            let a = r.next();
            let e = (a >> 23) & 0xFF;
            let step = r.next() % 28;
            let eb = e.saturating_sub(step).min(e + 2).min(254);
            let mant = if r.next() & 1 == 0 {
                (a ^ (r.next() & 0xFF)) & MANT
            } else {
                r.next() & MANT
            };
            let b = (r.next() & SIGN) | (eb << 23) | mant;
            check(a, b);
            // Low exponents: products and sums near and below the normals.
            check(a & 0x81FF_FFFF, b & 0x81FF_FFFF);
            check(a & 0x80FF_FFFF | 0x2000_0000, b & 0x80FF_FFFF | 0x1F00_0000);
        }
    }

    #[test]
    fn halfway_cases_round_to_even() {
        // 1 + 2^-24 is halfway between 1 and the next: stays 1; 1 + 3 *
        // 2^-24 is halfway above an odd one: rounds up.
        for (a, b) in [
            (0x3F80_0000, 0x3380_0000),
            (0x3F80_0001, 0x3380_0000),
            (0x3F80_0000, 0x3440_0000),
        ] {
            check(a, b);
        }
        // Every ulp step around one, against every small addend.
        for m in 0..64 {
            for k in 0..64u32 {
                check(0x3F80_0000 | m, 0x3300_0000 + (k << 18));
                check(0x3F80_0000 | m, 0xB300_0000 + (k << 18));
            }
        }
    }

    /// Every float against a few others: run by hand in release
    /// (`cargo test -p rp-hal-mem --release -- --ignored`), minutes.
    #[test]
    #[ignore = "every float: minutes in release"]
    fn every_float_against_some() {
        for b in [
            0x3F80_0000,
            0xBF80_0001,
            0x0080_0000,
            0x4B40_0001,
            0x3EAA_AAAB,
            0x0000_0001,
            0x7F7F_FFFF,
        ] {
            for a in 0..=u32::MAX {
                check(a, b);
            }
        }
    }
}
