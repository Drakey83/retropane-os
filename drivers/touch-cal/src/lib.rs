//! Touch screen calibration (item 76): raw touch readings to screen pixels.
//!
//! An **affine map** (the method of tslib and Windows Mobile's own Align
//! Screen):
//!
//! - `screen_x = (a·rx + b·ry + c) / 65536`
//! - `screen_y = (d·rx + e·ry + f) / 65536`
//!
//! It takes in one go what a per-axis scale can't: axes swapped (the
//! X51v's are), mirrored (its horizontal is), scaled, offset, and slightly
//! rotated or skewed.
//!
//! [`Calibration::fit`] finds the six coefficients by least squares from
//! the align screen's points (raw reading, where the target was). Integer
//! arithmetic only (the PXA270 has no FPU): the normal equations of the
//! readings centred on their means, in `i64`, solved by Cramer's rule. [`Calibration::check`] fits on all but
//! one point and measures how far it predicts the one left out: a panel
//! that isn't linear shows there, before anyone has to aim around it.
//!
//! Board-independent: the touch chip's driver gives raw readings, this
//! maps them, the shell runs the align screen.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

/// The coefficients' fixed-point scale (16 fractional bits).
pub const ONE: i32 = 1 << 16;

/// A raw reading and where on screen it should land.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    /// Raw X channel reading.
    pub rx: u16,
    /// Raw Y channel reading.
    pub ry: u16,
    /// Screen x the reading should map to.
    pub sx: u16,
    /// Screen y.
    pub sy: u16,
}

/// An affine map from raw readings to screen pixels (coefficients scaled by
/// [`ONE`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Calibration {
    /// `screen_x = (a·rx + b·ry + c) / ONE`.
    pub a: i32,
    /// See [`Self::a`].
    pub b: i32,
    /// See [`Self::a`].
    pub c: i32,
    /// `screen_y = (d·rx + e·ry + f) / ONE`.
    pub d: i32,
    /// See [`Self::d`].
    pub e: i32,
    /// See [`Self::d`].
    pub f: i32,
}

impl Calibration {
    /// The X51v before anyone aligns it: the owner's device, aligned on
    /// 2026-10-13 with the align screen and the touch filter (four
    /// alignments agreed within 0.3% in scale, ~9 px in vertical offset).
    /// Other X51v panels differ a little: the align screen refines it.
    pub const X51V_DEFAULT: Calibration = Calibration {
        a: -43,
        b: -9345,
        c: 34_538_826,
        d: 13925,
        e: -54,
        f: -2_709_680,
    };

    /// A per-axis map: the screen's horizontal from `h.0`'s channel, raw
    /// `h.1` at the left edge to `h.2` at the right (either order), and the
    /// vertical likewise from `v`.
    #[must_use]
    pub const fn from_axes(
        h: (Axis, u16, u16),
        v: (Axis, u16, u16),
        screen: (u16, u16),
    ) -> Calibration {
        // scale = size / (raw_end - raw_start), offset puts raw_start at 0.
        const fn axis(a: (Axis, u16, u16), size: u16) -> (i32, i32, i32) {
            let span = a.2 as i32 - a.1 as i32;
            let span = if span == 0 { 1 } else { span };
            let k = (size as i64 * ONE as i64 / span as i64) as i32;
            let c = -(a.1 as i64 * k as i64) as i32;
            match a.0 {
                Axis::X => (k, 0, c),
                Axis::Y => (0, k, c),
            }
        }
        let (a, b, c) = axis(h, screen.0);
        let (d, e, f) = axis(v, screen.1);
        Calibration { a, b, c, d, e, f }
    }

    /// Screen coordinates for a raw reading, clamped to `screen`.
    #[must_use]
    pub fn to_screen(&self, rx: u16, ry: u16, screen: (u16, u16)) -> (u16, u16) {
        let (x, y) = self.map(rx, ry);
        let clamp = |v: i64, size: u16| v.clamp(0, i64::from(size.max(1)) - 1) as u16;
        (clamp(x, screen.0), clamp(y, screen.1))
    }

    /// The unclamped mapping, rounded to the nearest pixel.
    #[must_use]
    pub fn map(&self, rx: u16, ry: u16) -> (i64, i64) {
        let (rx, ry) = (i64::from(rx), i64::from(ry));
        let one = i64::from(ONE);
        let round = |v: i64| v.div_euclid(one) + i64::from(v.rem_euclid(one) >= one / 2);
        (
            round(i64::from(self.a) * rx + i64::from(self.b) * ry + i64::from(self.c)),
            round(i64::from(self.d) * rx + i64::from(self.e) * ry + i64::from(self.f)),
        )
    }

    /// The least-squares affine map through `points` (3 to [`MAX_POINTS`],
    /// not all in a line). `None` when they don't determine one (too few or
    /// too many, collinear: the same raw reading for different targets).
    ///
    /// In 64-bit integers: the readings are centred on their (rounded)
    /// means first, which keeps every sum and determinant well inside
    /// `i64` (128-bit division cost 8 KiB of runtime code on this CPU).
    #[must_use]
    pub fn fit(points: &[Point]) -> Option<Calibration> {
        let n = points.len();
        if !(3..=MAX_POINTS).contains(&n) {
            return None;
        }
        let n = n as i64;
        let mean = |f: fn(&Point) -> u16| -> i64 {
            (points.iter().map(|p| i64::from(f(p))).sum::<i64>() + n / 2) / n
        };
        let (mx, my) = (mean(|p| p.rx), mean(|p| p.ry));
        // The screen points too: then the intercepts' column is near zero
        // and no product of sums can overflow.
        let mt = [mean(|p| p.sx), mean(|p| p.sy)];
        // Normal equations for the centred readings (u, v) and screen (t):
        // [Suu Suv Su; Suv Svv Sv; Su Sv n] (a b c')' = (Su·S, Sv·S, S S)'.
        let (mut suu, mut suv, mut svv, mut su, mut sv) = (0i64, 0i64, 0i64, 0i64, 0i64);
        let mut rhs = [[0i64; 3]; 2];
        for p in points {
            let (u, v) = (i64::from(p.rx) - mx, i64::from(p.ry) - my);
            suu += u * u;
            suv += u * v;
            svv += v * v;
            su += u;
            sv += v;
            for (r, t) in rhs
                .iter_mut()
                .zip([i64::from(p.sx) - mt[0], i64::from(p.sy) - mt[1]])
            {
                r[0] += u * t;
                r[1] += v * t;
                r[2] += t;
            }
        }
        let m = [[suu, suv, su], [suv, svv, sv], [su, sv, n]];
        let det = det3(&m)?;
        if det == 0 {
            return None;
        }
        let mut out = [0i64; 6];
        for (axis, r) in rhs.iter().enumerate() {
            for col in 0..3 {
                let mut mc = m;
                for (row, v) in mc.iter_mut().zip(r) {
                    row[col] = *v;
                }
                out[axis * 3 + col] = scaled_div(det3(&mc)?, det)?;
            }
        }
        // Back from centred values: c = c' + mean·ONE - a·mx - b·my.
        let one = i64::from(ONE);
        let back =
            |a: i64, b: i64, c: i64, t: i64| i32::try_from(c + t * one - a * mx - b * my).ok();
        Some(Calibration {
            a: i32::try_from(out[0]).ok()?,
            b: i32::try_from(out[1]).ok()?,
            c: back(out[0], out[1], out[2], mt[0])?,
            d: i32::try_from(out[3]).ok()?,
            e: i32::try_from(out[4]).ok()?,
            f: back(out[3], out[4], out[5], mt[1])?,
        })
    }

    /// How far, in pixels, a fit on all the other points misses point `i`
    /// (the larger of the two axes). `None` if that fit can't be made.
    #[must_use]
    pub fn check(points: &[Point], i: usize) -> Option<u32> {
        let p = points.get(i)?;
        let mut rest = [Point {
            rx: 0,
            ry: 0,
            sx: 0,
            sy: 0,
        }; 16];
        let mut n = 0;
        for (j, q) in points.iter().enumerate() {
            if j != i {
                *rest.get_mut(n)? = *q;
                n += 1;
            }
        }
        let cal = Calibration::fit(&rest[..n])?;
        let (x, y) = cal.map(p.rx, p.ry);
        let miss = (x - i64::from(p.sx))
            .unsigned_abs()
            .max((y - i64::from(p.sy)).unsigned_abs());
        u32::try_from(miss).ok()
    }
}

/// A raw channel, for [`Calibration::from_axes`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    /// The X channel.
    X,
    /// The Y channel.
    Y,
}

/// The most points [`Calibration::fit`] takes (bounds every sum: readings
/// under 4096, screens under 4096 pixels).
pub const MAX_POINTS: usize = 16;

/// The determinant, or `None` on overflow (never, within [`MAX_POINTS`]).
fn det3(m: &[[i64; 3]; 3]) -> Option<i64> {
    let minor = |a: i64, b: i64, c: i64, d: i64| a.checked_mul(b)?.checked_sub(c.checked_mul(d)?);
    let t0 = m[0][0].checked_mul(minor(m[1][1], m[2][2], m[1][2], m[2][1])?)?;
    let t1 = m[0][1].checked_mul(minor(m[1][0], m[2][2], m[1][2], m[2][0])?)?;
    let t2 = m[0][2].checked_mul(minor(m[1][0], m[2][1], m[1][1], m[2][0])?)?;
    t0.checked_sub(t1)?.checked_add(t2)
}

/// `num / den` in 1/[`ONE`] units, rounded; when `num · ONE` would
/// overflow, the divisor is scaled down instead (`den` is then far larger
/// than `ONE`, so nothing that matters is lost).
fn scaled_div(num: i64, den: i64) -> Option<i64> {
    let (num, den) = if den < 0 { (-num, -den) } else { (num, den) };
    let half = |d: i64| if num < 0 { -d / 2 } else { d / 2 };
    match num.checked_mul(i64::from(ONE)) {
        Some(big) => Some((big + half(den)) / den),
        None => {
            let d = den >> 16;
            (d != 0).then(|| (num + half(d)) / d)
        }
    }
}

/// The median of `v` (sorted in place); 0 for none. An insertion sort:
/// the align screen's 32 readings don't need the general sort's 2 KiB.
#[must_use]
pub fn median(v: &mut [u16]) -> u16 {
    for i in 1..v.len() {
        let mut j = i;
        while j > 0 && v[j - 1] > v[j] {
            v.swap(j - 1, j);
            j -= 1;
        }
    }
    v.get(v.len() / 2).copied().unwrap_or(0)
}

/// One raw sample: both positions and the first pressure reading (Z1;
/// it rises with pressure on the X51v's panel).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    /// Raw X channel.
    pub x: u16,
    /// Raw Y channel.
    pub y: u16,
    /// Raw Z1.
    pub z1: u16,
}

/// Below this Z1 there is no real contact (a lifting stylus read 5).
pub const Z_MIN: u16 = 150;
/// A sample's position counts only once pressure has steadied: Z1 within
/// this many 1/256ths of the previous sample's. Recorded on the X51v
/// (2026-10-13): Z1 climbs ~5% a sample while the stylus lands (1501,
/// 1578, 1598, 1619) and the positions creep with it.
pub const Z_STEADY: u32 = 16;
/// A sample's position counts only while Z1 is at least this many
/// 1/256ths of the touch's peak: as the stylus lifts, Z1 falls over 3-4
/// samples (1603, 1589, 1546, 1424) and the positions slide 7-15 px.
pub const Z_HELD: u32 = 243;
/// Two consecutive counted samples this close (raw, either channel) give
/// a position: ~3 px on the X51v's vertical, ~2 on its horizontal.
pub const STABLE_WITHIN: u16 = 12;

/// What the pen did, from [`Filter::sample`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pen {
    /// Nothing to report.
    Nothing,
    /// First stable position of a touch (raw).
    Down(u16, u16),
    /// A new stable position while down (raw).
    Move(u16, u16),
    /// A stable sample at the same position (raw): the align screen
    /// counts these while a target is held.
    Hold(u16, u16),
    /// Lifted; the last stable position (raw), never one taken as the
    /// stylus left.
    Up(u16, u16),
}

/// Turns raw samples into pen events a person meant, by pressure: a
/// position counts only once pressure has steadied after the landing and
/// while it stays near the touch's peak (so not as the stylus lifts), and
/// is reported when two such samples agree. Tuned and tested against the
/// owner's recorded taps (`tests/replay.rs`).
#[derive(Clone, Copy, Debug, Default)]
pub struct Filter {
    /// When the pen landed, while it is down.
    landed: Option<u32>,
    /// The previous sample's Z1 (for steadiness) and the touch's peak.
    last_z: u16,
    peak_z: u16,
    /// The previous counted sample, if the last sample counted.
    prev: Option<(u16, u16)>,
    /// The last stable position reported, while down.
    at: Option<(u16, u16)>,
    /// Pressure has fallen from its peak: the stylus is leaving.
    lifting: bool,
}

impl Filter {
    /// No pen.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            landed: None,
            last_z: 0,
            peak_z: 0,
            prev: None,
            at: None,
            lifting: false,
        }
    }

    /// Whether a touch is in progress (landed, reported or not).
    #[must_use]
    pub const fn down(&self) -> bool {
        self.landed.is_some()
    }

    /// When the pen landed, if it is down (for latency: the person's
    /// touch, not the filter's verdict).
    #[must_use]
    pub const fn landed_at(&self) -> Option<u32> {
        self.landed
    }

    /// One sample (`None`: no pressure, or no pen) at `now_ms`.
    pub fn sample(&mut self, raw: Option<Sample>, now_ms: u32) -> Pen {
        let Some(s) = raw.filter(|s| s.z1 >= Z_MIN) else {
            let up = self.at.map_or(Pen::Nothing, |(x, y)| Pen::Up(x, y));
            *self = Self::new();
            return up;
        };
        self.landed.get_or_insert(now_ms);
        let steady =
            u32::from(s.z1.abs_diff(self.last_z)) * 256 <= Z_STEADY * u32::from(self.last_z.max(1));
        self.last_z = s.z1;
        self.peak_z = self.peak_z.max(s.z1);
        if u32::from(s.z1) * 256 < Z_HELD * u32::from(self.peak_z) {
            // Once leaving, never back: the rest of the touch is the lift.
            self.lifting = true;
        }
        if !steady || self.lifting {
            self.prev = None;
            return Pen::Nothing;
        }
        let Some((px, py)) = self.prev.replace((s.x, s.y)) else {
            return Pen::Nothing;
        };
        let near = |a: u16, b: u16| a.abs_diff(b) <= STABLE_WITHIN;
        if !(near(px, s.x) && near(py, s.y)) {
            return Pen::Nothing;
        }
        let avg = |a: u16, b: u16| (u32::from(a) + u32::from(b)).div_ceil(2) as u16;
        let pos = (avg(px, s.x), avg(py, s.y));
        match self.at.replace(pos) {
            None => Pen::Down(pos.0, pos.1),
            Some(old) if old != pos => Pen::Move(pos.0, pos.1),
            Some(_) => Pen::Hold(pos.0, pos.1),
        }
    }
}

#[cfg(test)]
mod tests;
