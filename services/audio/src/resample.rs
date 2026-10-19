//! Rate conversion to the session rate, 44.1 kHz (ADR-0032 R4 step 1, A1).
//!
//! **Bandlimited interpolation from one table** (J. O. Smith's method): a
//! Kaiser-windowed sinc is stored once, one wing, [`PHASES`] points per
//! zero crossing; every ratio reads it, upsampling (8, 11.025, 16, 22.05,
//! 32 kHz) and downsampling (48 kHz: the filter stretched to the output's
//! band) alike. The whole-number steps of the old games' rates (11.025 kHz
//! x4, 22.05 kHz x2) land exactly on table points: no interpolation between
//! points, as A1 wanted. Other ratios interpolate linearly between points.
//!
//! **Stateless by output time**: the source position of output frame `t` is
//! exact rational arithmetic from the stream's anchor (no accumulated
//! phase, no filter history kept), so re-mixing any stretch of time
//! (rewrite-ahead, R2) gives the same samples as the first time.
//!
//! The table is computed at compile time (`const fn`, double precision),
//! so no generated file is checked in. Quality is measured by the host
//! tests (THD+N, images, aliasing) against the numbers in docs/hw/audio.md.
//! Scalar fixed point; the iWMMXt loop waits for the device's CPU
//! measurement (quality audit).

/// The session rate everything is converted to.
pub const OUT: u32 = 44_100;
/// Zero crossings per wing: the filter is `2 * ZEROS` source frames long
/// when upsampling.
pub const ZEROS: usize = 10;
/// Table points per zero crossing.
pub const PHASES: usize = 64;
/// Kaiser window's shape.
const BETA: f64 = 8.0;
/// The cut-off as a share of the slower side's Nyquist frequency: 0.90
/// trades the top of the band (19 kHz from 48 kHz content: -3.6 dB) for
/// aliasing and images (measured in the host tests).
const CUTOFF: f64 = 0.90;
/// Points in the wing, one guard point (zero) for the interpolation.
const LEN: usize = ZEROS * PHASES + 2;
/// The most taps a wing has (a 96 kHz source: the filter stretched).
pub const MAX_TAPS: usize = (ZEROS * 96_000).div_ceil(OUT as usize);

/// The wing, Q15.
pub static WING: [i16; LEN] = wing();

pub(crate) const fn sin(x: f64) -> f64 {
    // Reduced to [-pi, pi], then the Taylor series (enough terms for
    // double precision there).
    let two_pi = 2.0 * core::f64::consts::PI;
    let mut x = x - two_pi * ((x / two_pi) as i64 as f64);
    if x > core::f64::consts::PI {
        x -= two_pi;
    } else if x < -core::f64::consts::PI {
        x += two_pi;
    }
    let mut term = x;
    let mut sum = x;
    let mut n = 1;
    while n < 30 {
        term = -term * x * x / (((2 * n) * (2 * n + 1)) as f64);
        sum += term;
        n += 1;
    }
    sum
}

const fn sqrt(v: f64) -> f64 {
    if v <= 0.0 {
        return 0.0;
    }
    let mut g = if v > 1.0 { v } else { 1.0 };
    let mut i = 0;
    while i < 60 {
        g = 0.5 * (g + v / g);
        i += 1;
    }
    g
}

/// The modified Bessel function of the first kind, order 0 (its series).
const fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let mut k = 1;
    while k < 60 {
        let f = x / (2.0 * k as f64);
        term *= f * f;
        sum += term;
        k += 1;
    }
    sum
}

const fn wing() -> [i16; LEN] {
    let mut w = [0i16; LEN];
    let i0b = bessel_i0(BETA);
    let mut i = 0;
    while i < ZEROS * PHASES + 1 {
        let t = i as f64 / PHASES as f64;
        let x = CUTOFF * t;
        let sinc = if i == 0 {
            1.0
        } else {
            sin(core::f64::consts::PI * x) / (core::f64::consts::PI * x)
        };
        let r = t / ZEROS as f64;
        let win = bessel_i0(BETA * sqrt(1.0 - r * r)) / i0b;
        let v = CUTOFF * sinc * win * 32768.0;
        w[i] = if v >= 0.0 {
            (v + 0.5) as i64 as i16
        } else {
            (v - 0.5) as i64 as i16
        };
        i += 1;
    }
    w
}

const fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// Table points the filter spans (past them, 0).
pub const POINTS: usize = ZEROS * PHASES;

/// 48 kHz's coefficients, worked out at compile time: a 48 kHz source's
/// instants fall on only 148 phases (multiples of 300 / 44100), so its
/// wing for phase `300 k` is row `k`; the right wing of phase `p` is the
/// row of `44100 - p`. The same integers [`Ratio::coefs`] computes (one
/// formula, [`wing_coefs`]); 6.5 KB read-only, for the commonest
/// fractional rate.
pub static T48: [[i16; MAX_TAPS]; 148] = table48();

const fn table48() -> [[i16; MAX_TAPS]; 148] {
    let r = Ratio::of(48_000);
    let mut t = [[0i16; MAX_TAPS]; 148];
    let mut k = 0;
    while k < 148 {
        let w = wing_coefs(&r, (k as u32 * 300) * PHASES as u32);
        let mut i = 0;
        while i < MAX_TAPS {
            t[k][i] = w[i] as i16;
            i += 1;
        }
        k += 1;
    }
    t
}

/// One wing's coefficients from table position `start` (in units of
/// `1 / d` of a table point): the one formula behind [`Ratio::coefs`] and
/// [`T48`].
const fn wing_coefs(r: &Ratio, start: u32) -> [i32; MAX_TAPS] {
    let mut out = [0i32; MAX_TAPS];
    let (mut l, mut rem) = (start / r.d, start % r.d);
    let mut i = 0;
    while i < r.taps as usize {
        out[i] = if (l as usize) < POINTS {
            coef_of(r, l, rem)
        } else {
            0
        };
        l += r.step_l;
        rem += r.step_r;
        if rem >= r.d {
            rem -= r.d;
            l += 1;
        }
        i += 1;
    }
    out
}

/// The coefficient at table point `l` and `rem / d` past it, Q15, with the
/// stretched filter's scale when downsampling (so a sum stays in 32 bits
/// at every rate).
const fn coef_of(r: &Ratio, l: u32, rem: u32) -> i32 {
    let a = WING[l as usize] as i32;
    let c = if r.exact || rem == 0 {
        a
    } else {
        let diff = (WING[l as usize + 1] as i32 - a) as i64;
        a + ((diff * rem as i64 * r.inv_d) >> 32) as i32
    };
    if r.gain == 32_768 {
        c
    } else {
        ((c as i64 * r.gain) >> 15) as i32
    }
}

/// One source rate's conversion: constants worked out once, at open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ratio {
    /// The source rate.
    pub rate: u32,
    /// The table's divisor: the faster of the two rates.
    d: u32,
    /// Taps per wing.
    taps: u32,
    /// Output scale, Q15 (below 1 when downsampling).
    gain: i64,
    /// One tap's step through the table: whole points and the remainder.
    step_l: u32,
    step_r: u32,
    /// `2^32 / d`, so the interpolation needs no division.
    inv_d: i64,
    /// Every tap lands on a table point (whole-number steps).
    exact: bool,
}

/// Why a rate isn't taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BadRate(pub u32);

impl Ratio {
    /// The conversion from `rate` (4 kHz to 96 kHz) to [`OUT`].
    pub fn new(rate: u32) -> Result<Self, BadRate> {
        if !(4_000..=96_000).contains(&rate) {
            return Err(BadRate(rate));
        }
        Ok(Self::of(rate))
    }

    /// [`Self::new`] without the check (4-96 kHz assumed).
    const fn of(rate: u32) -> Self {
        let d = if rate > OUT { rate } else { OUT };
        let step = OUT * PHASES as u32;
        // Upsampling: ZEROS taps a wing; downsampling: the stretched filter
        // reaches ZEROS * rate / OUT source frames.
        let taps = (ZEROS as u32 * d).div_ceil(OUT);
        Self {
            rate,
            d,
            taps,
            gain: if rate > OUT {
                OUT as i64 * 32_768 / rate as i64
            } else {
                32_768
            },
            step_l: step / d,
            step_r: step % d,
            inv_d: (1i64 << 32) / d as i64,
            exact: rate <= OUT && (gcd(rate, OUT) * PHASES as u32).is_multiple_of(OUT),
        }
    }

    /// No conversion at all: the source is at the session rate.
    #[must_use]
    pub fn is_bypass(&self) -> bool {
        self.rate == OUT
    }

    /// Source frames the filter reads before the current one (and, one
    /// more, after it): what must stay in a stream's ring behind the played
    /// position, and be written ahead of the mixed one.
    #[must_use]
    pub fn reach(&self) -> u32 {
        if self.is_bypass() { 0 } else { self.taps }
    }

    /// Whether every tap lands on a table point.
    #[must_use]
    pub fn is_exact(&self) -> bool {
        self.exact
    }

    /// Where output frame `n` (counted from the anchor) reads the source:
    /// the source frame at or before it, and how far past it in
    /// [`OUT`]ths of a source frame.
    #[must_use]
    pub fn locate(&self, n: u64) -> (u64, u32) {
        let num = n * u64::from(self.rate);
        (num / u64::from(OUT), (num % u64::from(OUT)) as u32)
    }

    /// The next output frame's place, from this one's: `(frames to move on
    /// in the source, new phase)`.
    #[must_use]
    pub fn advance(&self, phase: u32) -> (u32, u32) {
        let mut p = phase + self.rate;
        let mut moved = 0;
        while p >= OUT {
            p -= OUT;
            moved += 1;
        }
        (moved, p)
    }

    /// Taps per wing.
    #[must_use]
    pub fn taps(&self) -> usize {
        self.taps as usize
    }

    /// The filter's coefficients (Q15) for one output instant `phase` (how
    /// far past the source frame at or before it, in [`OUT`]ths of a
    /// frame): `left[k]` weighs the frame `k` before that one (`0`: it
    /// itself), `right[k]` the frame `k + 1` after it. Past [`Self::taps`],
    /// or past the filter's end, 0. Computed once per instant, shared by
    /// every channel.
    pub fn coefs(&self, phase: u32, left: &mut [i32; MAX_TAPS], right: &mut [i32; MAX_TAPS]) {
        let p = PHASES as u32;
        *left = wing_coefs(self, phase * p);
        *right = wing_coefs(self, (OUT - phase) * p);
    }

    /// For a whole-number step (11.025, 22.05 kHz): the coefficients at
    /// `phase` are plain table points, every [`PHASES`]th from the first
    /// one returned for each wing (left, right); a point at or past
    /// [`POINTS`] counts 0. `None` for any other rate.
    #[must_use]
    pub fn stride(&self, phase: u32) -> Option<(usize, usize)> {
        if !self.exact || self.is_bypass() {
            return None;
        }
        let p = PHASES as u32;
        Some((
            (phase * p / OUT) as usize,
            ((OUT - phase) * p / OUT) as usize,
        ))
    }

    /// For 48 kHz: the rows of [`T48`] for `phase` (left, right).
    #[must_use]
    pub fn rows48(
        &self,
        phase: u32,
    ) -> Option<(&'static [i16; MAX_TAPS], &'static [i16; MAX_TAPS])> {
        if self.rate != 48_000 {
            return None;
        }
        let k = (phase / 300) as usize;
        Some((&T48[k], &T48[147 - k]))
    }

    /// A sum of samples times [`Self::coefs`] (Q15, under 2^31: see the
    /// test `the_sum_fits_32_bits`) to the output sample, Q8.
    #[must_use]
    pub fn finish(acc: i32) -> i32 {
        acc >> 7
    }

    /// One output sample, Q8 (a 16-bit sample shifted up 8): the source
    /// frame at or before the instant is `at(0)`, the ones before it
    /// `at(-1)`, `at(-2)`..., after it `at(1)`...; `phase` is how far past
    /// `at(0)` the instant is, in [`OUT`]ths of a source frame. The
    /// reference the mixer's block path must equal.
    pub fn sample(&self, phase: u32, at: &mut impl FnMut(i32) -> i32) -> i32 {
        let (mut left, mut right) = ([0; MAX_TAPS], [0; MAX_TAPS]);
        self.coefs(phase, &mut left, &mut right);
        let mut acc: i32 = 0;
        for k in 0..self.taps as usize {
            acc += at(-(k as i32)) * left[k] + at(k as i32 + 1) * right[k];
        }
        Self::finish(acc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    /// Converts a whole signal (zeros outside it), Q8 out.
    fn convert(x: &[i16], rate: u32) -> Vec<f64> {
        let r = Ratio::new(rate).unwrap();
        let n_out = (x.len() as u64 * u64::from(OUT) / u64::from(rate)) as usize;
        (0..n_out as u64)
            .map(|n| {
                let (i, ph) = r.locate(n);
                let mut at = |o: i32| {
                    let j = i as i64 + i64::from(o);
                    if (0..x.len() as i64).contains(&j) {
                        i32::from(x[j as usize])
                    } else {
                        0
                    }
                };
                f64::from(r.sample(ph, &mut at)) / 256.0
            })
            .collect()
    }

    fn sine(rate: u32, hz: f64, amp: f64, secs: f64) -> Vec<i16> {
        (0..(f64::from(rate) * secs) as usize)
            .map(|n| (amp * (2.0 * PI * hz * n as f64 / f64::from(rate)).sin()).round() as i16)
            .collect()
    }

    /// Least-squares fit of a sine at `hz` (and DC): its amplitude, and
    /// the residual's RMS (THD+N's numerator).
    fn fit(y: &[f64], hz: f64) -> (f64, f64) {
        let w = 2.0 * PI * hz / f64::from(OUT);
        let (mut ss, mut sc, mut cc, mut ys, mut yc, mut s1, mut c1, mut y1) =
            (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
        let n = y.len() as f64;
        for (i, &v) in y.iter().enumerate() {
            let (s, c) = (w * i as f64).sin_cos();
            ss += s * s;
            sc += s * c;
            cc += c * c;
            ys += v * s;
            yc += v * c;
            s1 += s;
            c1 += c;
            y1 += v;
        }
        // Solve the 3x3 normal equations [ss sc s1; sc cc c1; s1 c1 n].
        let m = [[ss, sc, s1], [sc, cc, c1], [s1, c1, n]];
        let b = [ys, yc, y1];
        let det = |m: [[f64; 3]; 3]| {
            m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
                - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
                + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
        };
        let d = det(m);
        let col = |k: usize| {
            let mut mm = m;
            for r in 0..3 {
                mm[r][k] = b[r];
            }
            det(mm) / d
        };
        let (a, bb, dc) = (col(0), col(1), col(2));
        let mut res = 0.0;
        for (i, &v) in y.iter().enumerate() {
            let (s, c) = (w * i as f64).sin_cos();
            let e = v - a * s - bb * c - dc;
            res += e * e;
        }
        (a.hypot(bb), (res / n).sqrt())
    }

    fn db(x: f64) -> f64 {
        20.0 * x.log10()
    }

    /// THD+N (dB) and gain (dB) of a 20000-amplitude sine converted from
    /// `rate`, the filter's start and end cut off.
    fn thd_n(rate: u32, hz: f64) -> (f64, f64) {
        let y = convert(&sine(rate, hz, 20_000.0, 0.1), rate);
        let y = &y[200..y.len() - 200];
        let (amp, res) = fit(y, hz);
        (db(res / (amp / 2f64.sqrt())), db(amp / 20_000.0))
    }

    /// The level (dB against the input's 20000) of `at_hz` in the
    /// conversion of a sine at `hz` from `rate`.
    fn level_at(rate: u32, hz: f64, at_hz: f64) -> f64 {
        let y = convert(&sine(rate, hz, 20_000.0, 0.1), rate);
        db(fit(&y[200..y.len() - 200], at_hz).0 / 20_000.0)
    }

    #[test]
    fn the_table_is_the_windowed_sinc() {
        // The compile-time table against the same formula in std's maths.
        let i0 = |x: f64| {
            (1..60)
                .fold((1.0, 1.0), |(s, t), k| {
                    let f = x / (2.0 * f64::from(k));
                    let t = t * f * f;
                    (s + t, t)
                })
                .0
        };
        for (i, &got) in WING.iter().enumerate().take(ZEROS * PHASES + 1) {
            let t = i as f64 / PHASES as f64;
            let x = CUTOFF * t;
            let sinc = if i == 0 {
                1.0
            } else {
                (PI * x).sin() / (PI * x)
            };
            let r = t / ZEROS as f64;
            let want =
                CUTOFF * sinc * i0(BETA * (1.0 - r * r).max(0.0).sqrt()) / i0(BETA) * 32768.0;
            assert!(
                (f64::from(got) - want).abs() <= 0.5 + 1e-6,
                "point {i}: {got} vs {want}"
            );
        }
        assert_eq!(WING[ZEROS * PHASES + 1], 0, "the guard point");
    }

    #[test]
    fn the_old_games_rates_step_exactly_and_48k_does_not() {
        for rate in [11_025, 22_050] {
            assert!(Ratio::new(rate).unwrap().is_exact(), "{rate}");
        }
        for rate in [8_000, 16_000, 32_000, 48_000] {
            assert!(!Ratio::new(rate).unwrap().is_exact(), "{rate}");
        }
        assert!(Ratio::new(OUT).unwrap().is_bypass());
        assert_eq!(Ratio::new(3_999), Err(BadRate(3_999)));
        assert_eq!(Ratio::new(96_001), Err(BadRate(96_001)));
    }

    #[test]
    fn locate_and_advance_agree_for_every_rate() {
        for rate in [8_000, 11_025, 22_050, 32_000, 44_100, 48_000] {
            let r = Ratio::new(rate).unwrap();
            let (mut i, mut ph) = r.locate(0);
            for n in 1..5_000u64 {
                let (moved, p) = r.advance(ph);
                i += u64::from(moved);
                ph = p;
                assert_eq!((i, ph), r.locate(n), "{rate} at {n}");
            }
        }
    }

    // The measured quality (docs/hw/audio.md "The mixer's numbers"): each
    // target a few dB inside what the filter gives, so a worse filter or a
    // broken interpolation fails.

    #[test]
    fn a_sine_converts_cleanly_from_every_rate() {
        for (rate, hz, target) in [
            (8_000, 1_000.0, -80.0),
            (11_025, 1_000.0, -84.0),
            (22_050, 1_000.0, -86.0),
            (22_050, 5_000.0, -80.0),
            (32_000, 1_000.0, -80.0),
            (48_000, 1_000.0, -80.0),
            (48_000, 10_000.0, -78.0),
        ] {
            let (thd, gain) = thd_n(rate, hz);
            assert!(
                thd < target,
                "{rate} Hz source, {hz} Hz: THD+N {thd:.1} dB (target {target})"
            );
            assert!(
                gain.abs() < 0.1,
                "{rate} Hz source, {hz} Hz: gain {gain:.2} dB"
            );
        }
    }

    #[test]
    fn upsampling_images_stay_down() {
        // A tone at f from a source at fs makes an image at fs - f.
        for (rate, hz) in [
            (22_050, 5_000.0),
            (22_050, 1_000.0),
            (11_025, 1_000.0),
            (8_000, 1_000.0),
        ] {
            let img = f64::from(rate) - hz;
            let l = level_at(rate, hz, img);
            assert!(
                l < -65.0,
                "{rate} Hz source, {hz} Hz: image at {img} Hz {l:.1} dB"
            );
        }
    }

    #[test]
    fn downsampling_48k_folds_little_and_keeps_the_band() {
        // 23 kHz is above the output's 22.05 kHz: it folds to 21.1 kHz.
        let alias = level_at(48_000, 23_000.0, f64::from(OUT) - 23_000.0);
        assert!(alias < -22.0, "23 kHz folded to 21.1 kHz at {alias:.1} dB");
        let top = level_at(48_000, 19_000.0, 19_000.0);
        assert!(top > -4.5, "19 kHz kept at {top:.1} dB");
        let mid = level_at(48_000, 15_000.0, 15_000.0);
        assert!(mid.abs() < 0.2, "15 kHz at {mid:.2} dB");
    }

    #[test]
    fn the_48k_table_and_the_strides_are_the_computed_coefficients() {
        let r = Ratio::new(48_000).unwrap();
        let (mut l, mut rr) = ([0; MAX_TAPS], [0; MAX_TAPS]);
        for k in 0..148u32 {
            let phase = k * 300;
            if phase >= OUT {
                break;
            }
            r.coefs(phase, &mut l, &mut rr);
            let (tl, tr) = r.rows48(phase).unwrap();
            for i in 0..MAX_TAPS {
                assert_eq!(
                    (i32::from(tl[i]), i32::from(tr[i])),
                    (l[i], rr[i]),
                    "phase {phase} tap {i}"
                );
            }
        }
        for rate in [11_025, 22_050] {
            let r = Ratio::new(rate).unwrap();
            let mut ph = 0;
            for _ in 0..8 {
                r.coefs(ph, &mut l, &mut rr);
                let (a, b) = r.stride(ph).unwrap();
                for i in 0..r.taps() {
                    let at = |p: usize| if p < POINTS { i32::from(WING[p]) } else { 0 };
                    assert_eq!(
                        (at(a + i * PHASES), at(b + i * PHASES)),
                        (l[i], rr[i]),
                        "{rate} at {ph}, tap {i}"
                    );
                }
                ph = r.advance(ph).1;
            }
        }
        assert!(Ratio::new(48_000).unwrap().stride(0).is_none());
        assert!(Ratio::new(OUT).unwrap().stride(0).is_none());
        assert!(Ratio::new(22_050).unwrap().rows48(0).is_none());
    }

    #[test]
    fn the_sum_fits_32_bits() {
        // Every instant of every rate: the coefficients' magnitudes summed,
        // times a full-scale sample, stay under 2^31.
        for rate in [
            4_000, 8_000, 11_025, 16_000, 22_050, 32_000, 44_000, 48_000, 88_200, 96_000,
        ] {
            let r = Ratio::new(rate).unwrap();
            assert!(r.taps() <= MAX_TAPS);
            let (mut l, mut rr) = ([0; MAX_TAPS], [0; MAX_TAPS]);
            for phase in (0..OUT).step_by(7) {
                r.coefs(phase, &mut l, &mut rr);
                let sum: i64 = l.iter().chain(&rr).map(|c| i64::from(*c).abs()).sum();
                assert!(sum * 32_768 < 1 << 31, "{rate} at {phase}: {sum}");
            }
        }
    }

    #[test]
    fn full_scale_never_overflows() {
        // A full-scale square wave at the worst phase: the filter's overshoot
        // must still fit the mixer's 32-bit sum with room for many streams.
        for rate in [8_000, 22_050, 48_000] {
            let x: Vec<i16> = (0..4_000)
                .map(|n| if (n / 3) % 2 == 0 { i16::MAX } else { i16::MIN })
                .collect();
            let y = convert(&x, rate);
            let peak = y.iter().fold(0.0f64, |m, v| m.max(v.abs()));
            assert!(peak < 32_768.0 * 2.0, "{rate}: peak {peak}");
        }
    }
}
