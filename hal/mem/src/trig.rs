//! `sin` and `cos` in integers for the game modules (docs/design/ref-rp.md
//! "Lever 2"; R294).
//!
//! Iterate, don't imitate (rule 0). The C library's way (musl, as Rust's
//! `libm` and so `compiler_builtins` carry it): `sinf` widened to doubles
//! (`k_sinf` works in `f64`), `sin` correctly rounded doubles with a
//! Cody-Waite or Payne-Hanek reduction, every step a software-float call
//! on the PXA270, which has no FPU (run 15: the client's `__muldf3` and
//! `__adddf3` 6% of the play, much of it `k_sin`). Ours: the argument's
//! significand times a 96-bit window of 2/pi's bits (Payne-Hanek's idea,
//! one `u128` product: exact for every float and double, no huge-argument
//! fallback), the quarter turn split off in integers, and sin and cos of
//! the rest by two degree-4 polynomials in f^2 whose Q30 coefficients were
//! fitted offline to this exact integer evaluation (Lawson's minimax,
//! `tools/numerics/fit_trig.py`): at most 1.2e-9 absolute
//! error before the final rounding, under a twentieth of a float's ulp at
//! one. Each call also keeps the other function of the same argument, since
//! the game asks for `sin(a)` then `cos(a)` (AngleVectors, AngleQuaternion).
//!
//! What it gives up: the double versions are accurate to ~1e-9 absolute
//! (not to a double's last bit); the game stores every result in a float.

/// 2/pi's bits after the binary point, 32 a word, most significant first
/// (enough for any double's exponent: bit 991 + 96 + 32).
const TWO_OVER_PI: [u32; 40] = [
    0xA2F9_836E, 0x4E44_1529, 0xFC27_57D1, 0xF534_DDC0, 0xDB62_9599, 0x3C43_9041, 0xFE51_63AB,
    0xDEBB_C561, 0xB724_6E3A, 0x424D_D2E0, 0x0649_2EEA, 0x09D1_921C, 0xFE1D_EB1C, 0xB129_A73E,
    0xE882_35F5, 0x2EBB_4484, 0xE99C_7026, 0xB45F_7E41, 0x3991_D639, 0x8353_39F4, 0x9C84_5F8B,
    0xBDF9_283B, 0x1FF8_97FF, 0xDE05_980F, 0xEF2F_118B, 0x5A0A_6D1F, 0x6D36_7ECF, 0x27CB_09B7,
    0x4F46_3F66, 0x9E5F_EA2D, 0x7527_BAC7, 0xEBE5_F17B, 0x3D07_39F7, 0x8A52_92EA, 0x6BFB_5FB1,
    0x1F8D_5D08, 0x5603_3046, 0xFC7B_6BAB, 0xF0CF_BC20, 0x9AF4_361D,
];

/// sin(pi/2 f) / f as a polynomial in u = f^2, Q30 (f in [-1/2, 1/2]).
const S: [i64; 5] = [1_686_629_713, -693_598_667, 85_569_259, -5_026_472, 169_873];
/// cos(pi/2 f) as a polynomial in u = f^2, Q30.
const C: [i64; 5] = [1_073_741_824, -1_324_675_869, 272_375_234, -22_398_331, 970_267];

/// `(acc * u) >> 32` steps from the top coefficient down, `u` in Q32.
fn horner(c: &[i64; 5], u: u32) -> i64 {
    let u = i64::from(u);
    let mut acc = c[4];
    for &k in c[..4].iter().rev() {
        acc = k + ((acc * u) >> 32);
    }
    acc
}

/// A real number as `(negative, mantissa, exponent)`: `mantissa * 2^exponent`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Num {
    neg: bool,
    mant: u64,
    exp: i32,
}

/// The 96 bits of 2/pi from bit `i0` (1-based; bit i weighs 2^-i).
fn window(i0: u32) -> u128 {
    let j = ((i0 - 1) / 32) as usize;
    let r = (i0 - 1) % 32;
    let w = |k: usize| u128::from(TWO_OVER_PI.get(j + k).copied().unwrap_or(0));
    let all = w(0) << 96 | w(1) << 64 | w(2) << 32 | w(3);
    (all << r) >> 32
}

/// `x = m * 2^e` (m's top bit set) times 2/pi, modulo 4, in Q62.
fn quarter_turns(m: u64, e: i32) -> u64 {
    let i0 = if e > 1 { (e - 1) as u32 } else { 1 };
    let w = window(i0);
    // x * (2/pi) * 2^62 ~ m * w * 2^(e - 33 - i0); the bits of 2/pi before
    // i0 give whole multiples of 2^64 (four quarter turns), dropped.
    let sh = 33 + i0 as i32 - e;
    if sh >= 160 {
        return 0;
    }
    let sh = sh as u32;
    let (mh, ml) = (m >> 32, m & 0xFFFF_FFFF);
    let hi = if sh >= 32 + 128 { 0 } else { shr(u128::from(mh) * w, sh - 32) };
    let lo = if sh >= 128 || ml == 0 { 0 } else { shr(u128::from(ml) * w, sh) };
    hi.wrapping_add(lo)
}

fn shr(v: u128, s: u32) -> u64 {
    if s >= 128 { 0 } else { (v >> s) as u64 }
}

/// sin and cos of `x = m * 2^e` (x > 0).
fn sin_cos(m: u64, e: i32) -> (Num, Num) {
    let y = quarter_turns(m, e);
    let k = (y.wrapping_add(1 << 61) >> 62) as u32;
    let f = y.wrapping_sub(u64::from(k) << 62) as i64;
    let (fneg, a) = (f < 0, f.unsigned_abs());
    let (sin_f, cos_f) = if a == 0 {
        (Num { neg: false, mant: 0, exp: 0 }, Num { neg: false, mant: 1, exp: 0 })
    } else {
        let lz = a.leading_zeros();
        let fm = ((a << lz) >> 32) as u32;
        // f = fm * 2^(-30 - lz); u = f^2 in Q32.
        let sh = 28 + 2 * lz;
        let u = if sh >= 64 { 0 } else { ((u64::from(fm) * u64::from(fm)) >> sh) as u32 };
        let s = horner(&S, u) as u64;
        let c = horner(&C, u) as u64;
        (
            Num { neg: fneg, mant: u64::from(fm) * s, exp: -60 - lz as i32 },
            Num { neg: false, mant: c, exp: -30 },
        )
    };
    let pick = |q: u32| {
        let n = if q & 1 == 0 { sin_f } else { cos_f };
        Num { neg: n.neg ^ (q & 2 != 0), ..n }
    };
    (pick(k & 3), pick(k.wrapping_add(1) & 3))
}

/// `n` rounded to nearest (ties to even) into a float of `bits` significand
/// bits; `(sign, biased exponent, fraction)` packed by the caller's layout.
fn round(n: Num, sig_bits: u32) -> Option<(bool, i32, u64)> {
    if n.mant == 0 {
        return None;
    }
    let lz = n.mant.leading_zeros();
    let m = n.mant << lz;
    let drop = 64 - sig_bits;
    let mut sig = m >> drop;
    let rest = m & ((1u64 << drop) - 1);
    let half = 1u64 << (drop - 1);
    if rest > half || (rest == half && sig & 1 == 1) {
        sig += 1;
    }
    // The value is sig * 2^(exp + 63 - lz - (sig_bits - 1)) with sig in
    // [2^(sig_bits-1), 2^sig_bits].
    let mut e = n.exp + 63 - lz as i32;
    if sig >> sig_bits != 0 {
        sig >>= 1;
        e += 1;
    }
    Some((n.neg, e, sig))
}

fn to_f32(n: Num) -> u32 {
    match round(n, 24) {
        None => u32::from(n.neg) << 31,
        Some((neg, e, sig)) => {
            let biased = e + 127;
            // Results are within [2^-126, 1] for normal arguments.
            let biased = biased.clamp(1, 254) as u32;
            u32::from(neg) << 31 | biased << 23 | (sig as u32 & 0x7F_FFFF)
        }
    }
}

fn to_f64(n: Num) -> u64 {
    match round(n, 53) {
        None => u64::from(n.neg) << 63,
        Some((neg, e, sig)) => {
            let biased = (e + 1023).clamp(1, 2046) as u64;
            u64::from(neg) << 63 | biased << 52 | (sig & 0xF_FFFF_FFFF_FFFF)
        }
    }
}

/// sinf and cosf of the float with bits `x`, as bits.
#[must_use]
pub fn sin_cos_f32(x: u32) -> (u32, u32) {
    let neg = x >> 31 != 0;
    let be = (x >> 23) & 0xFF;
    if be == 0xFF {
        let nan = if x & 0x7F_FFFF != 0 { x | 0x40_0000 } else { 0x7FC0_0000 };
        return (nan, nan);
    }
    if be == 0 {
        // Zero and the subnormals: sin x = x, cos x = 1 to the float's bit.
        return (x, 0x3F80_0000);
    }
    let m = u64::from((x & 0x7F_FFFF) | 0x80_0000) << 40;
    let (s, c) = sin_cos(m, be as i32 - 190);
    let s = to_f32(s);
    (if neg { s ^ 0x8000_0000 } else { s }, to_f32(c))
}

/// sin and cos of the double with bits `x`, as bits.
#[must_use]
pub fn sin_cos_f64(x: u64) -> (u64, u64) {
    let neg = x >> 63 != 0;
    let be = ((x >> 52) & 0x7FF) as i32;
    if be == 0x7FF {
        let nan = if x & 0xF_FFFF_FFFF_FFFF != 0 { x | 1 << 51 } else { 0x7FF8 << 48 };
        return (nan, nan);
    }
    if be == 0 {
        return (x, 0x3FF0 << 48);
    }
    let m = ((x & 0xF_FFFF_FFFF_FFFF) | 1 << 52) << 11;
    let (s, c) = sin_cos(m, be - 1086);
    let s = to_f64(s);
    (if neg { s ^ 1 << 63 } else { s }, to_f64(c))
}

/// The game's calls, kept: the last argument and both results. The game
/// modules run on one thread and no interrupt calls these (the sound and
/// input handlers are the runner's, in Rust), so a plain cell does.
#[cfg(all(target_arch = "arm", target_os = "none", feature = "float"))]
mod exports {
    use core::cell::UnsafeCell;

    use super::{sin_cos_f32, sin_cos_f64};

    /// One entry: the argument's bits, then its sin's and cos's.
    struct Last<T>(UnsafeCell<[T; 3]>);
    // SAFETY: one thread only (above).
    unsafe impl<T> Sync for Last<T> {}

    impl<T: Copy + PartialEq> Last<T> {
        fn pair(&self, x: T, work: fn(T) -> (T, T)) -> (T, T) {
            // SAFETY: one thread, no reentry: `work` calls nothing here.
            let e = unsafe { &mut *self.0.get() };
            if e[0] != x {
                let r = work(x);
                *e = [x, r.0, r.1];
            }
            (e[1], e[2])
        }
    }

    /// Zero's sin and cos to start with.
    static F: Last<u32> = Last(UnsafeCell::new([0, 0, 0x3F80_0000]));
    static D: Last<u64> = Last(UnsafeCell::new([0, 0, 0x3FF0 << 48]));

    fn f32_pair(x: u32) -> (u32, u32) {
        F.pair(x, sin_cos_f32)
    }

    fn f64_pair(x: u64) -> (u64, u64) {
        D.pair(x, sin_cos_f64)
    }

    #[unsafe(no_mangle)]
    extern "C" fn sinf(x: f32) -> f32 {
        f32::from_bits(f32_pair(x.to_bits()).0)
    }

    #[unsafe(no_mangle)]
    extern "C" fn cosf(x: f32) -> f32 {
        f32::from_bits(f32_pair(x.to_bits()).1)
    }

    #[unsafe(no_mangle)]
    unsafe extern "C" fn sincosf(x: f32, s: *mut f32, c: *mut f32) {
        let (a, b) = f32_pair(x.to_bits());
        // SAFETY: C contract: two valid floats to write.
        unsafe {
            *s = f32::from_bits(a);
            *c = f32::from_bits(b);
        }
    }

    #[unsafe(no_mangle)]
    extern "C" fn sin(x: f64) -> f64 {
        f64::from_bits(f64_pair(x.to_bits()).0)
    }

    #[unsafe(no_mangle)]
    extern "C" fn cos(x: f64) -> f64 {
        f64::from_bits(f64_pair(x.to_bits()).1)
    }

    #[unsafe(no_mangle)]
    unsafe extern "C" fn sincos(x: f64, s: *mut f64, c: *mut f64) {
        let (a, b) = f64_pair(x.to_bits());
        // SAFETY: C contract: two valid doubles to write.
        unsafe {
            *s = f64::from_bits(a);
            *c = f64::from_bits(b);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{sin_cos_f32, sin_cos_f64};

    /// A small deterministic generator (xorshift).
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
    }

    fn ulp32(v: f32) -> f64 {
        let b = v.abs().to_bits().max(0x0080_0000);
        f64::from(f32::from_bits(b + 1) - f32::from_bits(b))
    }

    /// Each float result within one ulp of the true value (double's), or
    /// within 2^-28 absolute where the result is small (near a zero).
    fn check32(x: f32) {
        let (s, c) = sin_cos_f32(x.to_bits());
        let (s, c) = (f32::from_bits(s), f32::from_bits(c));
        let xd = f64::from(x);
        for (got, want, name) in [(s, xd.sin(), "sin"), (c, xd.cos(), "cos")] {
            let err = (f64::from(got) - want).abs();
            let ok = err <= ulp32(want as f32) || err <= 2f64.powi(-28);
            assert!(ok, "{name}({x:e}) = {got:e}, want {want:e} (error {err:e})");
        }
    }

    #[test]
    fn sinf_and_cosf_are_within_a_float_ulp_over_the_game_s_angles() {
        let mut r = Rng(0x2026_1116);
        for _ in 0..400_000 {
            let v = (r.next() >> 11) as f64 / (1u64 << 53) as f64;
            check32(((v * 2.0 - 1.0) * 1000.0) as f32);
        }
    }

    #[test]
    fn sinf_and_cosf_hold_at_every_exponent_and_at_multiples_of_a_quarter_turn() {
        let mut r = Rng(7);
        for _ in 0..200_000 {
            let bits = (r.next() as u32) & 0x7F7F_FFFF;
            if bits >> 23 == 0 || f32::from_bits(bits) > 1.0e7 {
                continue;
            }
            check32(f32::from_bits(bits));
        }
        for k in -2000..2000 {
            let q = k as f64 * core::f64::consts::FRAC_PI_2;
            check32(q as f32);
            check32(f32::from_bits((q as f32).to_bits() + 1));
        }
        for x in [0.0f32, -0.0, 1e-30, 1e-6, 0.5, 1.0, 1e5, core::f32::consts::PI] {
            check32(x);
        }
    }

    #[test]
    fn huge_and_special_floats_follow_the_c_library() {
        let (s, c) = sin_cos_f32(f32::INFINITY.to_bits());
        assert!(f32::from_bits(s).is_nan() && f32::from_bits(c).is_nan());
        let (s, c) = sin_cos_f32(f32::NAN.to_bits());
        assert!(f32::from_bits(s).is_nan() && f32::from_bits(c).is_nan());
        assert_eq!(sin_cos_f32((-0.0f32).to_bits()), ((-0.0f32).to_bits(), 1.0f32.to_bits()));
        // Huge arguments reduce exactly (2/pi's bits reach every exponent).
        for x in [1e20f32, 3.0e38, f32::MAX] {
            let (s, c) = sin_cos_f32(x.to_bits());
            let (s, c) = (f32::from_bits(s), f32::from_bits(c));
            assert!((s * s + c * c - 1.0).abs() < 1e-6, "{x:e}: {s} {c}");
        }
        // sin(1e22) is a well-known hard case: -0.8522008497671888...
        let s = f32::from_bits(sin_cos_f32(1e22f32.to_bits()).0);
        assert!((f64::from(s) - f64::from(1e22f32).sin()).abs() < 1e-6, "{s}");
    }

    #[test]
    fn the_double_versions_are_within_a_billionth() {
        let mut r = Rng(99);
        for _ in 0..200_000 {
            let v = (r.next() >> 11) as f64 / (1u64 << 53) as f64;
            let x = (v * 2.0 - 1.0) * 1.0e6;
            let (s, c) = sin_cos_f64(x.to_bits());
            assert!((f64::from_bits(s) - x.sin()).abs() < 2e-9, "sin({x})");
            assert!((f64::from_bits(c) - x.cos()).abs() < 2e-9, "cos({x})");
        }
        for x in [1e-300f64, 1e-9, 1.0, 1e15, 1e300] {
            let (s, c) = sin_cos_f64(x.to_bits());
            assert!((f64::from_bits(s) - x.sin()).abs() < 2e-9, "sin({x})");
            assert!((f64::from_bits(c) - x.cos()).abs() < 2e-9, "cos({x})");
        }
    }
}
