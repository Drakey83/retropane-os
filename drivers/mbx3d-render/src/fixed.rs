//! Float words from integers.
//!
//! The 3D core reads positions, depths and texture coordinates as IEEE
//! single-precision floats, but the PXA270 has no FPU: a float operation
//! links a software helper costing tens of cycles (the no-float rule,
//! `xtask/src/size.rs`). The renderer keeps everything in fixed point and
//! builds the float's bits with integer operations only, rounded to
//! nearest-even exactly as a float conversion would.

/// The float bits of `v / 2^frac`, rounded to nearest-even (exact when
/// `|v| < 2^24`). `frac` is at most 30.
#[must_use]
pub const fn f32_bits(v: i32, frac: u32) -> u32 {
    if v == 0 {
        return 0;
    }
    let sign = if v < 0 { 1u32 << 31 } else { 0 };
    let m = v.unsigned_abs();
    let top = 31 - m.leading_zeros();
    let mut exp = top as i32 - frac as i32 + 127;
    let mant = if top <= 23 {
        m << (23 - top)
    } else {
        let shift = top - 23;
        let rest = m & ((1 << shift) - 1);
        let half = 1 << (shift - 1);
        let mut q = m >> shift;
        if rest > half || (rest == half && q & 1 == 1) {
            q += 1;
        }
        if q == 1 << 24 {
            q >>= 1;
            exp += 1;
        }
        q
    };
    sign | (exp as u32) << 23 | (mant & 0x007F_FFFF)
}

/// `1.0`.
pub const ONE: u32 = f32_bits(1, 0);

#[cfg(test)]
mod tests {
    use super::*;

    fn want(v: i32, frac: u32) -> u32 {
        ((f64::from(v) / f64::from(1u32 << frac)) as f32).to_bits()
    }

    #[test]
    fn matches_the_float_conversion() {
        for frac in [0, 4, 8, 16, 24, 30] {
            for v in [
                0,
                1,
                -1,
                2,
                3,
                7,
                480,
                -480,
                640,
                2048,
                32_767,
                65_536,
                (1 << 24) - 1,
                1 << 24,
                (1 << 24) + 1,
                (1 << 24) + 3,
                (1 << 25) + 2,
                (1 << 25) + 6,
                i32::MAX,
                i32::MIN,
                i32::MIN + 1,
            ] {
                assert_eq!(f32_bits(v, frac), want(v, frac), "{v} / 2^{frac}");
            }
        }
        // A sweep with a simple generator (no dependencies).
        let mut x = 0x1234_5678u32;
        for _ in 0..100_000 {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            let frac = x % 31;
            let v = x as i32 >> (x % 23);
            assert_eq!(f32_bits(v, frac), want(v, frac), "{v} / 2^{frac}");
        }
    }

    #[test]
    fn constants() {
        assert_eq!(ONE, 0x3F80_0000);
        assert_eq!(f32_bits(1, 1), 0x3F00_0000, "0.5");
        assert_eq!(f32_bits(32_768, 16), 0x3F00_0000, "0.5 in 16.16");
        assert_eq!(f32_bits(2048, 0), 0x4500_0000);
    }
}
