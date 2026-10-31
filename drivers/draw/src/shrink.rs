//! Pictures made smaller on the device with a filter fitted offline (option
//! C, docs/design/weather-rule0.md): each icon pack keeps only its big
//! (250 px) pictures, and every smaller size the shell shows is made from
//! the big one as it is decoded.
//!
//! The textbook way is a fixed filter (box, bilinear, Lanczos; Android's
//! `inSampleSize` halves and then filters bilinearly). Ours: the kernel is
//! fitted by least squares (`tools/shell-assets/shrink_fit.py`), one per
//! output size, so that shrinking the big picture comes as close as it can
//! to the picture made straight from the master at that size; the fit
//! models exactly this arithmetic (premultiplied alpha, a separable kernel
//! sampled [`P`] times an output pixel and interpolated linearly, each
//! pixel's weights normalised to one). Integer only, no allocation: the
//! caller lends the scratch.

use crate::Argb;

#[path = "shrink_taps.rs"]
mod taps;
pub use taps::{ALL, K40, K66, K160};

/// A kernel's samples per output pixel.
pub const P: u32 = 8;
/// The most taps a pixel takes on one axis (a kernel's radius times the
/// scale, both sides): 250 -> 40 at radius 3 is 40.
pub const MAX_TAPS: usize = 64;

/// A fitted kernel: its samples at 0, 1/[`P`], ... output px, in 1/16384
/// (zero from `radius_p / P` out), for pictures made `side` px.
pub struct Kernel {
    /// The output size it was fitted for.
    pub side: u16,
    /// Its radius in samples (radius × [`P`]).
    pub radius_p: u16,
    /// Its RMS against the masters on the test pictures, × 100 (the
    /// tool's; the tests hold the device's arithmetic to it).
    pub rms_x100: u32,
    /// The samples, the first at 0.
    pub k: &'static [i32],
}

impl Kernel {
    /// The kernel fitted for `side`, else the one for the nearest size.
    #[must_use]
    pub fn for_side(side: u16) -> &'static Self {
        ALL.iter()
            .min_by_key(|k| k.side.abs_diff(side))
            .copied()
            .unwrap_or(&K66)
    }

    /// Its value at `x` (output px × [`P`] × 256), in 1/16384.
    fn at(&self, x: u32) -> i32 {
        let (i, f) = ((x >> 8) as usize, (x & 255) as i32);
        if i >= usize::from(self.radius_p) {
            return 0;
        }
        let a = self.k.get(i).copied().unwrap_or(0);
        let b = self.k.get(i + 1).copied().unwrap_or(0);
        (a * (256 - f) + b * f) >> 8
    }

    /// Taps a pixel takes on an axis going from `n_in` to `n_out`.
    fn taps(&self, n_in: usize, n_out: usize) -> usize {
        // 2 × radius × scale, rounded up, and one each side.
        (2 * usize::from(self.radius_p) * n_in).div_ceil(P as usize * n_out.max(1)) + 2
    }

    /// Output `j`'s weights on an axis from `n_in` to `n_out`: its first
    /// input and, in `w`, its weights in 1/16384, summing to 16384. The
    /// count, 0 when they don't fit `w`.
    fn weights(&self, n_in: usize, n_out: usize, j: usize, w: &mut [i32]) -> (usize, usize) {
        let t = self.taps(n_in, n_out);
        if t > w.len() || n_out == 0 {
            return (0, 0);
        }
        // Input i's distance from j's centre, in output px:
        // ((2i + 1) out - (2j + 1) in) / (2 in).
        let (ni, no) = (n_in as i64, n_out as i64);
        let c2 = (2 * j as i64 + 1) * ni;
        let lo = ((c2 - 2 * i64::from(self.radius_p) * ni / i64::from(P)) / (2 * no) - 1).max(0);
        let lo = (lo as usize).min(n_in.saturating_sub(1));
        let n = t.min(n_in - lo);
        let mut sum = 0;
        for (k, wk) in w[..n].iter_mut().enumerate() {
            let d = ((2 * (lo + k) as i64 + 1) * no - c2).unsigned_abs();
            let x = d * u64::from(P) * 256 / (2 * ni as u64);
            *wk = self.at(x.min(u64::from(u32::MAX)) as u32);
            sum += *wk;
        }
        if sum == 0 {
            return (lo, 0);
        }
        for wk in &mut w[..n] {
            *wk = *wk * 16384 / sum;
        }
        (lo, n)
    }
}

/// The size of a picture `w` x `h` made `side` px: its long side `side`,
/// the other in proportion, rounded half up (the fit's too).
#[must_use]
pub const fn size(w: u16, h: u16, side: u16) -> (u16, u16) {
    let m = if w > h { w as u32 } else { h as u32 };
    if m == 0 {
        return (0, 0);
    }
    let s = side as u32;
    let a = (w as u32 * s + m / 2) / m;
    let b = (h as u32 * s + m / 2) / m;
    (
        if a == 0 { 1 } else { a as u16 },
        if b == 0 { 1 } else { b as u16 },
    )
}

/// The scratch words [`shrink`] needs for `sw` px wide made `dw` wide.
#[must_use]
pub fn scratch_words(sw: usize, dw: usize, k: &Kernel) -> usize {
    sw * 4 + dw * (k.taps(sw, dw) + 1)
}

/// `a × c / 255`, rounded.
const fn mul255(c: u32, a: u32) -> i32 {
    let t = c * a + 128;
    ((t + (t >> 8)) >> 8) as i32
}

/// `255 × 65536 / a` for each alpha (straight colour back from
/// premultiplied without a division a pixel).
static RECIP: [u32; 256] = {
    let mut t = [0u32; 256];
    let mut a = 1;
    while a < 256 {
        t[a] = (255 * 65536 + a as u32 / 2) / a as u32;
        a += 1;
    }
    t
};

/// `src` (`sw` x `sh`, straight ARGB, rows `sw` apart) made smaller into
/// `dst` (`dw` x `dh`, rows `dw` apart) with kernel `k`, through
/// `scratch` ([`scratch_words`]). False, `dst` untouched, if a size is
/// zero or larger than the source, a buffer is short, or a pixel would
/// take more than [`MAX_TAPS`] taps.
pub fn shrink(
    src: &[Argb],
    (sw, sh): (usize, usize),
    dst: &mut [Argb],
    (dw, dh): (usize, usize),
    k: &Kernel,
    scratch: &mut [i32],
) -> bool {
    if dw == 0 || dh == 0 || dw > sw || dh > sh || src.len() < sw * sh || dst.len() < dw * dh {
        return false;
    }
    let th = k.taps(sw, dw);
    if th > MAX_TAPS || k.taps(sh, dh) > MAX_TAPS || scratch.len() < scratch_words(sw, dw, k) {
        return false;
    }
    let (acc, hw) = scratch.split_at_mut(sw * 4);
    // Each output column's first input and weights, once.
    for (x, e) in hw.chunks_exact_mut(th + 1).take(dw).enumerate() {
        let (first, w) = e.split_at_mut(1);
        let (lo, n) = k.weights(sw, dw, x, w);
        first[0] = lo as i32;
        w[n..].fill(0);
    }
    let mut wv = [0i32; MAX_TAPS];
    for y in 0..dh {
        // The rows under this output row, weighted, premultiplied.
        let (lo, n) = k.weights(sh, dh, y, &mut wv);
        acc.fill(0);
        for (t, &w) in wv[..n].iter().enumerate() {
            if w == 0 {
                continue;
            }
            let row = &src[(lo + t) * sw..(lo + t + 1) * sw];
            for (p, a4) in row.iter().zip(acc.as_chunks_mut::<4>().0) {
                let a = p >> 24;
                if a == 0 {
                    continue;
                }
                a4[0] += w * a as i32;
                a4[1] += w * mul255(p >> 16 & 0xFF, a);
                a4[2] += w * mul255(p >> 8 & 0xFF, a);
                a4[3] += w * mul255(p & 0xFF, a);
            }
        }
        // 6 fractional bits kept: the next sum stays inside 32 bits.
        for v in acc.iter_mut() {
            *v >>= 8;
        }
        // Across: each output pixel from its columns.
        let acc4 = acc.as_chunks::<4>().0;
        let out = &mut dst[y * dw..(y + 1) * dw];
        for (o, e) in out.iter_mut().zip(hw.chunks_exact(th + 1)) {
            let lo = e[0] as usize;
            let mut s = [0i32; 4];
            for (t, &w) in e[1..].iter().enumerate() {
                if w == 0 {
                    continue;
                }
                let Some(a4) = acc4.get(lo + t) else {
                    break;
                };
                for c in 0..4 {
                    s[c] += w * a4[c];
                }
            }
            let ch = |v: i32| ((v + (1 << 19)) >> 20).clamp(0, 255) as u32;
            let a = ch(s[0]);
            *o = if a == 0 {
                0
            } else {
                let r = RECIP[a as usize];
                let st = |v: i32| ((ch(v) * r + 32768) >> 16).min(255);
                a << 24 | st(s[1]) << 16 | st(s[2]) << 8 | st(s[3])
            };
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A test picture (drivers/draw/tests/data) as ARGB words, and its size.
    fn png(name: &str) -> (Vec<Argb>, usize, usize) {
        let path = format!("{}/tests/data/{name}.png", env!("CARGO_MANIFEST_DIR"));
        let file = std::fs::read(&path).expect(&path);
        let (w, h) = rp_weather::png::size(&file).expect("a PNG");
        let (w, h) = (w as usize, h as usize);
        let mut out = vec![0; w * h];
        let mut data = vec![0u8; file.len()];
        let mut raw = vec![0u8; h * (1 + w * 4)];
        rp_weather::png::decode(&file, &mut out, &mut data, &mut raw).expect("decoded");
        (out, w, h)
    }

    /// Premultiplied 8-bit RGBA, as the fit compares.
    fn premul(p: Argb) -> [f64; 4] {
        let a = f64::from(p >> 24);
        let c = |s: u32| f64::from(p >> s & 0xFF) * a / 255.0;
        [c(16), c(8), c(0), a]
    }

    /// Each test picture's big one made smaller on the device's arithmetic
    /// comes as close to the master's at that size as the fit said (its
    /// RMS, rounding allowed), and closer than a plain box filter.
    #[test]
    fn the_fitted_kernels_match_the_masters_as_the_fit_said() {
        for k in ALL {
            let (mut err, mut boxed, mut n) = (0.0, 0.0, 0.0);
            for id in ["03", "17"] {
                let (big, bw, bh) = png(&format!("shrink-{id}-big"));
                let (want, ww, wh) = png(&format!("shrink-{id}-{}", k.side));
                let (dw, dh) = size(bw as u16, bh as u16, k.side);
                assert_eq!(
                    (usize::from(dw), usize::from(dh)),
                    (ww, wh),
                    "{id} at {}",
                    k.side
                );
                let mut got = vec![0; ww * wh];
                let mut scratch = vec![0; scratch_words(bw, ww, k)];
                assert!(shrink(&big, (bw, bh), &mut got, (ww, wh), k, &mut scratch));
                // A box over each output pixel's area, for comparison.
                let (sx, sy) = (bw as f64 / ww as f64, bh as f64 / wh as f64);
                for (i, (&g, &w)) in got.iter().zip(&want).enumerate() {
                    let (x, y) = (i % ww, i / ww);
                    let (x0, y0) = ((x as f64 * sx) as usize, (y as f64 * sy) as usize);
                    let (x1, y1) = (
                        (((x + 1) as f64 * sx).ceil() as usize).min(bw),
                        (((y + 1) as f64 * sy).ceil() as usize).min(bh),
                    );
                    let mut b = [0.0; 4];
                    for yy in y0..y1 {
                        for xx in x0..x1 {
                            for (c, v) in b.iter_mut().zip(premul(big[yy * bw + xx])) {
                                *c += v;
                            }
                        }
                    }
                    let area = ((x1 - x0) * (y1 - y0)) as f64;
                    let (g, w) = (premul(g), premul(w));
                    for c in 0..4 {
                        err += (g[c] - w[c]).powi(2);
                        boxed += (b[c] / area - w[c]).powi(2);
                    }
                    n += 4.0;
                }
            }
            let (rms, box_rms) = ((err / n).sqrt(), (boxed / n).sqrt());
            let fit = f64::from(k.rms_x100) / 100.0;
            assert!(
                rms <= fit + 0.5,
                "{} px: RMS {rms:.2}, the fit's {fit:.2}",
                k.side
            );
            assert!(
                rms < box_rms,
                "{} px: RMS {rms:.2}, box {box_rms:.2}",
                k.side
            );
        }
    }

    #[test]
    fn sizes_and_refusals() {
        assert_eq!(size(250, 176, 66), (66, 46));
        assert_eq!(size(169, 176, 40), (38, 40));
        assert_eq!(size(250, 1, 40), (40, 1));
        let src = vec![0xFF80_4020; 20 * 10];
        let mut dst = vec![0; 10 * 5];
        let mut s = vec![0; scratch_words(20, 10, &K66)];
        // Larger than the source, a short buffer, short scratch.
        assert!(!shrink(&src, (20, 10), &mut dst, (21, 5), &K66, &mut s));
        assert!(!shrink(
            &src[..199],
            (20, 10),
            &mut dst,
            (10, 5),
            &K66,
            &mut s
        ));
        assert!(!shrink(
            &src,
            (20, 10),
            &mut dst,
            (10, 5),
            &K66,
            &mut s[..10]
        ));
        // A flat colour stays that colour (the weights sum to one).
        assert!(shrink(&src, (20, 10), &mut dst, (10, 5), &K66, &mut s));
        for &p in &dst {
            let d = |s: u32| (p >> s & 0xFF).abs_diff(0xFF80_4020 >> s & 0xFF);
            assert!(
                d(24) == 0 && d(16) <= 1 && d(8) <= 1 && d(0) <= 1,
                "{p:08X}"
            );
        }
        // See-through stays see-through.
        let clear = vec![0; 20 * 10];
        assert!(shrink(&clear, (20, 10), &mut dst, (10, 5), &K66, &mut s));
        assert!(dst.iter().all(|&p| p == 0));
    }
}
