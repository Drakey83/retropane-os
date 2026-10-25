//! The video plane's scaler filter: what its fifteen registers hold, and
//! a set computed for any scale (quality audit C73, gate V5).
//!
//! **The format, read from Linux mbxfb's start-up set (static, 2026-10-25;
//! unproven on the X51v until `rp-blend` run P9d):** each coefficient is a
//! signed byte, 64 standing for a weight of one; each direction holds half
//! of a symmetric kernel sampled every 1/8 of a source pixel, from its
//! outer edge in to its centre: 17 bytes for the vertical filter's 4 taps
//! (`VSCOEFF0-4`, distances 2 down to 0), 33 for the horizontal's 8
//! (`HSCOEFF0-8`, distances 4 down to 0), packed low byte first. Read this
//! way, mbxfb's set is a windowed sinc (zero at every whole pixel) whose
//! taps sum to exactly 64 at each of the eight phases, both directions:
//! the evidence for the reading.
//!
//! mbxfb's own words: "good for scaling up; for scaling down, the
//! application has to calculate them". A kernel cut for enlarging passes
//! detail finer than the smaller picture can hold, which aliases (moiré on
//! fine stripes). [`Filter::fitted`] stretches the kernel by the shrink
//! factor instead: horizontally a Keys cubic (Catmull-Rom, exact at 1:1),
//! which at half size spans the 8 taps exactly; vertically the same cubic
//! when enlarging and a tent when shrinking (a stretched cubic would need 8
//! taps). Each phase's taps are then made to sum to exactly 64, so flat
//! areas keep their brightness at every phase. Integer arithmetic only (the
//! PXA270 has no FPU).

use crate::layers::reg;

/// Bytes in the vertical half kernel: 2 x 8 + 1.
pub const V_LEN: usize = 17;
/// Bytes in the horizontal half kernel: 4 x 8 + 1.
pub const H_LEN: usize = 33;

/// A weight of one.
pub const ONE: i32 = 64;

/// `SVCTRL` as mbxfb writes it: both initial phases `1 << 10` (half a
/// pixel in 5.11). Copied, not understood.
// inherited: C73, Linux mbxfb (drivers/video/fbdev/mbx/mbxfb.c, mbxfb_setup)
pub const SVCTRL_MBXFB: u32 = (1 << 10) << 16 | 1 << 10;

/// The scaler's two half kernels (see the module's notes on the format).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Filter {
    /// Vertical, outer edge (2 pixels out) to the centre.
    pub vertical: [i8; V_LEN],
    /// Horizontal, outer edge (4 pixels out) to the centre.
    pub horizontal: [i8; H_LEN],
}

impl Filter {
    /// mbxfb's start-up set, unpacked.
    // inherited: C73, Linux mbxfb (mbxfb_setup's VSCOEFF0-4, HSCOEFF0-8)
    pub const MBXFB: Self = Self {
        vertical: [
            0, 1, 0, -1, -2, -3, -4, -3, 0, 5, 13, 23, 34, 45, 55, 61, 64,
        ],
        horizontal: [
            0, 1, 1, -1, 0, 0, 0, 0, 0, 0, 1, 2, 2, 3, 2, 1, 0, -2, -5, -7, -9, -10, -9, -5, 0, 7,
            17, 28, 39, 49, 57, 62, 64,
        ],
    };

    /// Only the centre: each output pixel takes its nearest source pixel
    /// (P9d's proof of the layout: hard blocks when enlarging).
    pub const NEAREST: Self = {
        let mut f = Self {
            vertical: [0; V_LEN],
            horizontal: [0; H_LEN],
        };
        f.vertical[V_LEN - 1] = ONE as i8;
        f.horizontal[H_LEN - 1] = ONE as i8;
        f
    };

    /// Every weight halved (P9d's proof that 64 is one: the picture dims).
    #[must_use]
    pub const fn halved(self) -> Self {
        let mut f = self;
        let mut i = 0;
        while i < H_LEN {
            if i < V_LEN {
                f.vertical[i] /= 2;
            }
            f.horizontal[i] /= 2;
            i += 1;
        }
        f
    }

    /// The fifteen register writes: `SVCTRL`, `VSCOEFF0-4`, `HSCOEFF0-8`.
    #[must_use]
    pub const fn words(&self) -> [(usize, u32); 15] {
        let mut w = [(0, 0); 15];
        w[0] = (reg::SVCTRL, SVCTRL_MBXFB);
        let mut i = 0;
        while i < 5 {
            w[1 + i] = (reg::VSCOEFF0 + 4 * i, pack(&self.vertical, 4 * i));
            i += 1;
        }
        let mut i = 0;
        while i < 9 {
            w[6 + i] = (reg::HSCOEFF0 + 4 * i, pack(&self.horizontal, 4 * i));
            i += 1;
        }
        w
    }

    /// The set for a picture `src` pixels wide and high shown `out` wide
    /// and high (each direction its own factor; enlarging uses the 1:1
    /// kernel).
    ///
    /// # Panics
    /// If a size is 0, or a direction shrinks past 2:1 (the taps can't
    /// hold the stretched kernel; [`crate::layers`] refuses it too).
    #[must_use]
    pub fn fitted(src: (u32, u32), out: (u32, u32)) -> Self {
        let mut f = Self {
            vertical: [0; V_LEN],
            horizontal: [0; H_LEN],
        };
        let h = half(H_LEN - 1, src.0, out.0, Kernel::Keys);
        let v = half(
            V_LEN - 1,
            src.1,
            out.1,
            if out.1 < src.1 {
                Kernel::Tent
            } else {
                Kernel::Keys
            },
        );
        for (d, s) in f.horizontal.iter_mut().zip(h) {
            *d = s;
        }
        for (d, s) in f.vertical.iter_mut().zip(v) {
            *d = s;
        }
        f
    }
}

/// Bytes `at..at + 4` of `b` (0 past its end), low byte first.
const fn pack(b: &[i8], at: usize) -> u32 {
    let mut w = 0;
    let mut j = 0;
    while j < 4 {
        if at + j < b.len() {
            w |= (b[at + j] as u8 as u32) << (8 * j);
        }
        j += 1;
    }
    w
}

/// The sum of each phase's taps (`taps` of them), phase `p` sampling the
/// kernel at every whole pixel offset by `p / 8` (phases `p` and `8 - p`
/// mirror each other): 64 at every phase keeps flat areas flat.
#[must_use]
pub fn phase_sums(half: &[i8], taps: i32) -> [i32; 8] {
    let n = half.len() as i32 - 1;
    let at = |d: i32| {
        let d = d.abs();
        if d <= n {
            i32::from(half[(n - d) as usize])
        } else {
            0
        }
    };
    let mut s = [0; 8];
    for (p, sum) in s.iter_mut().enumerate() {
        *sum = (-taps..=taps).map(|i| at(8 * i - p as i32)).sum();
    }
    s
}

#[derive(Clone, Copy)]
enum Kernel {
    /// Keys' cubic, a = -1/2, over two pixels each side.
    Keys,
    /// A tent over one pixel each side.
    Tent,
}

/// `kernel` at `t / den` pixels, times `2 den^3` (exact in integers).
fn shape(kernel: Kernel, t: i128, den: i128) -> i128 {
    let (t2, d2) = (t * t, den * den);
    match kernel {
        Kernel::Keys if t < den => 3 * t2 * t - 5 * t2 * den + 2 * d2 * den,
        Kernel::Keys if t < 2 * den => -t2 * t + 5 * t2 * den - 8 * t * d2 + 4 * d2 * den,
        Kernel::Tent if t < den => 2 * d2 * (den - t),
        _ => 0,
    }
}

/// One direction's half kernel (`n + 1` bytes of the result, `n` = 16 or
/// 32 eighths) for `src` shown at `out`: the kernel stretched by `src /
/// out` when shrinking and scaled by `out / src` so it still sums to one;
/// then each phase made to sum to exactly 64 (its weights scaled to 64 and
/// rounded, the remainder put on its largest weight).
fn half(n: usize, src: u32, out: u32, kernel: Kernel) -> [i8; H_LEN] {
    assert!(src > 0 && out > 0, "a size of 0");
    assert!(src <= 2 * out, "shrinks past 2:1");
    let (num, den_s) = if out < src {
        (i128::from(out), i128::from(src))
    } else {
        (1, 1)
    };
    let den = 8 * den_s;
    // Each byte's weight, all over one common denominator, and its phase
    // class (distance in eighths mod 8, folded: phases p and 8 - p share
    // their weights) and how many times it counts in that phase's sum
    // (twice when the phase meets it on both sides of the centre).
    let mut raw = [0i128; H_LEN];
    for (k, r) in raw.iter_mut().enumerate().take(n + 1) {
        let d8 = (n - k) as i128;
        *r = num * shape(kernel, num * d8, den);
    }
    let class = |k: usize| {
        let m = (n - k) % 8;
        m.min(8 - m)
    };
    let mult = |k: usize| {
        let d8 = n - k;
        if (d8.is_multiple_of(8) && d8 != 0) || d8 % 8 == 4 {
            2
        } else {
            1
        }
    };
    let mut out_b = [0i8; H_LEN];
    for c in 0..=4 {
        let in_c = || (0..=n).filter(move |&k| class(k) == c);
        let sum: i128 = in_c().map(|k| mult(k) * raw[k]).sum();
        assert!(sum > 0, "a phase with no weight");
        let mut got: i128 = 0;
        for k in in_c() {
            // 64 raw / sum, rounded half away from zero.
            let q = 2 * i128::from(ONE) * raw[k];
            let v = if q >= 0 {
                (q + sum) / (2 * sum)
            } else {
                -((-q + sum) / (2 * sum))
            };
            out_b[k] = v as i8;
            got += mult(k) * v;
        }
        // The remainder on the largest weight (the centre for phase 0).
        let fix = if c == 0 {
            n
        } else {
            in_c()
                .max_by_key(|&k| (i128::from(out_b[k]).abs(), k))
                .unwrap_or(n)
        };
        let left = i128::from(ONE) - got;
        out_b[fix] = (i128::from(out_b[fix]) + left / mult(fix)) as i8;
    }
    out_b
}

#[cfg(test)]
mod tests;
