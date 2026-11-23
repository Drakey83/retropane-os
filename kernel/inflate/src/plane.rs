//! Lossless picture planes (`RPPL`): the wallpaper's Y, U and V packed on
//! the PC and unpacked once on the device, bit for bit (R208).
//!
//! **Rule 0.** PNG (the textbook way, and what this crate did first:
//! Paeth then deflate) predicts each byte from one fixed rule and hands the
//! differences to a coder built for text, which finds repeats a smooth
//! photo doesn't have: 138.8 KB for the sunset. JPEG-LS (LOCO-I) and the
//! context-modelled coders after it (CALIC, FFV1, JPEG XL's modular mode)
//! predict with a median edge detector and code each difference with
//! statistics kept for its neighbourhood. JPEG XL's decoder is far too big
//! for a resident image; ours keeps what pays on this data and nothing
//! else: LOCO-I's median predictor, eleven contexts by local activity (the
//! three gradients' sum, on a log scale), an adaptive binary range coder
//! (LZMA's, 11-bit probabilities) coding each difference as zero, sign,
//! size class and mantissa, and the adaptation rate chosen at build for
//! this picture. About 1 KB of decoder; ~100 KB for the sunset, ~28%
//! under PNG's way, the same pixels (the tests compare them).
//!
//! - **Safe and bounded:** a damaged stream decodes into `dst` or returns
//!   an [`Error`]: never a panic, never a write past `dst` or a read past
//!   `src`; the planes' Adler-32 is checked.
//! - Layout: `RPPL`, version 1, the rate, the plane count, each plane's
//!   width and height (u16, little-endian) and the low bits all its bytes
//!   have clear (coded without them: the sunset's planes are 6-bit), the
//!   Adler-32 of every plane's bytes in order (u32, little-endian), then
//!   the range coder's bytes.

use super::{Error, adler32};

/// The stream's first bytes.
pub const MAGIC: [u8; 4] = *b"RPPL";
const VERSION: u8 = 1;
/// The most planes a stream holds.
pub const MAX_PLANES: usize = 4;
/// Contexts by activity (log scale: the three gradients' sum is < 1024).
const CTX: usize = 11;
/// A probability's bits.
const PROB_BITS: u32 = 11;
const HALF: u16 = 1 << (PROB_BITS - 1);
/// The adaptation rates a stream may ask for.
pub(crate) const RATES: core::ops::RangeInclusive<u8> = 3..=6;

/// One binary decision coded (the encoder) or read (the decoder): `bit`
/// is the encoder's; the bit coded is returned.
pub(crate) trait Coder {
    fn bit(&mut self, p: &mut u16, bit: u32) -> u32;
}

/// The statistics a plane is coded with.
struct Model {
    rate: u32,
    zero: [u16; CTX],
    sign: [u16; CTX],
    /// The size class (0-7: the magnitude's top bit), in unary.
    class: [[u16; 8]; CTX],
    /// The mantissa's bits, by class and position.
    mant: [[u16; 8]; 8],
}

impl Model {
    fn new(rate: u8) -> Self {
        Self {
            rate: u32::from(rate),
            zero: [HALF; CTX],
            sign: [HALF; CTX],
            class: [[HALF; 8]; CTX],
            mant: [[HALF; 8]; 8],
        }
    }

    /// The difference `e` (the encoder's; the decoder passes 0), coded in
    /// context `ctx`; the difference coded is returned.
    #[inline(always)]
    fn diff<C: Coder>(&mut self, c: &mut C, ctx: usize, e: i8) -> i8 {
        let r = self.rate;
        let code = |c: &mut C, p: &mut u16, bit: u32| {
            let b = c.bit(p, bit);
            if b == 0 {
                *p += ((1 << PROB_BITS) - *p) >> r;
            } else {
                *p -= *p >> r;
            }
            b
        };
        if code(c, &mut self.zero[ctx], u32::from(e != 0)) == 0 {
            return 0;
        }
        let neg = code(c, &mut self.sign[ctx], u32::from(e < 0));
        // 1..=128.
        let m = u32::from(e.unsigned_abs());
        let k = 31 - m.max(1).leading_zeros();
        // Unary: one decision for the commonest class (1), two for 2-3.
        let mut n = 0u32;
        while n < 7 && code(c, &mut self.class[ctx][n as usize], u32::from(n < k)) == 1 {
            n += 1;
        }
        let k = n;
        let mut v = 1u32;
        for pos in (0..k).rev() {
            let b = code(c, &mut self.mant[k as usize][pos as usize], (m >> pos) & 1);
            v = v << 1 | b;
        }
        // `v` is at most 255 from a damaged stream: wraps, as any byte may.
        let v = v as u8;
        if neg == 1 {
            v.wrapping_neg() as i8
        } else {
            v as i8
        }
    }
}

/// One byte: predicted from its neighbours left `a`, up `b`, up-left
/// `cc` and up-right `d` (LOCO-I's median of `a`, `b` and `a + b - cc`),
/// its context the three gradients' sum on a log scale, its difference
/// coded; the encoder's `v` holds the byte, the decoder's is set.
#[inline(always)]
fn pixel<C: Coder>(c: &mut C, m: &mut Model, a: u8, b: u8, cc: u8, d: u8, v: &mut u8) {
    let (ia, ib, ic, id) = (i32::from(a), i32::from(b), i32::from(cc), i32::from(d));
    let pred = if ic >= ia.max(ib) {
        ia.min(ib)
    } else if ic <= ia.min(ib) {
        ia.max(ib)
    } else {
        ia + ib - ic
    } as u8;
    let act = (id - ib).unsigned_abs() + (ib - ic).unsigned_abs() + (ic - ia).unsigned_abs();
    let ctx = ((32 - act.leading_zeros()) as usize).min(CTX - 1);
    let e = v.wrapping_sub(pred) as i8;
    *v = pred.wrapping_add(m.diff(c, ctx, e) as u8);
}

/// [`pixel`] out of line: the edges' calls share one copy, and only a
/// row's middle has it inline.
#[inline(never)]
fn edge<C: Coder>(c: &mut C, m: &mut Model, a: u8, b: u8, cc: u8, d: u8, v: &mut u8) {
    pixel(c, m, a, b, cc, d, v);
}

/// Walks a `w` x `h` plane (`px` exactly that) in order, each byte coded
/// from the ones before it ([`pixel`]); the encoder's `px` holds the
/// picture, the decoder's is filled. Off the edges: the first row's
/// neighbours are all its left (128 before the first), the first column's
/// left and up-left its up, the last column's up-right its up. The rows'
/// neighbours are carried in registers, with no edge test inside a row.
fn walk<C: Coder>(c: &mut C, m: &mut Model, px: &mut [u8], w: usize, h: usize) {
    let mut a = 128u8;
    for v in &mut px[..w] {
        edge(c, m, a, a, a, a, v);
        a = *v;
    }
    for y in 1..h {
        let (done, cur) = px.split_at_mut(y * w);
        let prev = &done[(y - 1) * w..];
        let cur = &mut cur[..w];
        if w == 1 {
            edge(c, m, prev[0], prev[0], prev[0], prev[0], &mut cur[0]);
            continue;
        }
        edge(c, m, prev[0], prev[0], prev[0], prev[1], &mut cur[0]);
        let (mut a, mut cc, mut b) = (cur[0], prev[0], prev[1]);
        let (mid, last) = cur[1..].split_at_mut(w - 2);
        for (v, &d) in mid.iter_mut().zip(&prev[2..]) {
            pixel(c, m, a, b, cc, d, v);
            (a, cc, b) = (*v, b, d);
        }
        edge(c, m, a, b, cc, b, &mut last[0]);
    }
}

/// LZMA's range decoder over `src`.
struct Decoder<'a> {
    src: &'a [u8],
    at: usize,
    range: u32,
    code: u32,
    /// A read past the end happened (the stream is short).
    over: bool,
}

impl Decoder<'_> {
    fn byte(&mut self) -> u32 {
        if let Some(&b) = self.src.get(self.at) {
            self.at += 1;
            u32::from(b)
        } else {
            self.over = true;
            0
        }
    }
}

impl Coder for Decoder<'_> {
    #[inline(always)]
    fn bit(&mut self, p: &mut u16, _: u32) -> u32 {
        let bound = (self.range >> PROB_BITS) * u32::from(*p);
        let b = if self.code < bound {
            self.range = bound;
            0
        } else {
            self.code -= bound;
            self.range -= bound;
            1
        };
        // A loop, as the encoder's: at the slowest rates a probability
        // falls to 7 (of 2048), so one bit can take the range below 2^16
        // and two bytes are due. LZMA's single test relies on its rate 5
        // keeping every probability at 31 or more (R208: the generated
        // sky, packed at rate 3, read short with one).
        while self.range < 1 << 24 {
            self.range <<= 8;
            self.code = self.code << 8 | self.byte();
        }
        b
    }
}

/// The planes' sizes `src` holds, after checking its header.
/// A plane's width, height and the low bits every one of its bytes has
/// clear (coded without them).
type Dims = (usize, usize, u32);

fn header(src: &[u8]) -> Result<(u8, usize, [Dims; MAX_PLANES], usize), Error> {
    let [m0, m1, m2, m3, ver, rate, n, ..] = *src else {
        return Err(Error::Truncated);
    };
    if [m0, m1, m2, m3] != MAGIC || ver != VERSION || !RATES.contains(&rate) {
        return Err(Error::Header);
    }
    let n = usize::from(n);
    if n == 0 || n > MAX_PLANES {
        return Err(Error::Header);
    }
    let mut dims = [(0, 0, 0); MAX_PLANES];
    let mut at = 7;
    for d in dims.iter_mut().take(n) {
        let s = src.get(at..at + 5).ok_or(Error::Truncated)?;
        *d = (
            usize::from(u16::from_le_bytes([s[0], s[1]])),
            usize::from(u16::from_le_bytes([s[2], s[3]])),
            u32::from(s[4]),
        );
        if d.0 == 0 || d.1 == 0 || d.2 > 7 {
            return Err(Error::Header);
        }
        at += 5;
    }
    Ok((rate, n, dims, at))
}

/// Unpacks `RPPL` stream `src` into `dst`, its planes one after another;
/// `want` is the planes' sizes (width, height) the caller lays out, and a
/// stream of any other is refused. The bytes written.
///
/// # Errors
/// [`Error::Header`] for a wrong header or sizes, [`Error::Full`] when the
/// planes don't fit `dst`, [`Error::Truncated`] for a short stream and
/// [`Error::Checksum`] for damage.
pub fn unpack_planes(src: &[u8], want: &[(usize, usize)], dst: &mut [u8]) -> Result<usize, Error> {
    let (rate, n, dims, at) = header(src)?;
    if n != want.len() || dims.iter().zip(want).any(|(d, w)| (d.0, d.1) != *w) {
        return Err(Error::Header);
    }
    let total: usize = want.iter().map(|&(w, h)| w * h).sum();
    if total > dst.len() {
        return Err(Error::Full);
    }
    let s = src.get(at..at + 4).ok_or(Error::Truncated)?;
    let sum = u32::from_le_bytes([s[0], s[1], s[2], s[3]]);
    let mut d = Decoder {
        src,
        at: at + 4,
        range: u32::MAX,
        code: 0,
        over: false,
    };
    // LZMA's first byte is always 0; then four of the code.
    if d.byte() != 0 {
        return Err(Error::Bad);
    }
    for _ in 0..4 {
        d.code = d.code << 8 | d.byte();
    }
    let mut start = 0;
    for &(w, h, low) in &dims[..n] {
        let mut m = Model::new(rate);
        let px = &mut dst[start..start + w * h];
        walk(&mut d, &mut m, px, w, h);
        if low > 0 {
            for v in px {
                *v <<= low;
            }
        }
        start += w * h;
    }
    if d.over {
        return Err(Error::Truncated);
    }
    if adler32(&dst[..total]) != sum {
        return Err(Error::Checksum);
    }
    Ok(total)
}

/// The PC side (feature `pack`): planes packed as [`unpack_planes`] reads
/// them, the adaptation rate tried at every value and the smallest kept.
#[cfg(any(test, feature = "pack"))]
pub mod pack {
    extern crate std;

    use std::vec::Vec;

    use super::{Coder, MAGIC, MAX_PLANES, Model, PROB_BITS, RATES, VERSION, walk};
    use crate::adler32;

    /// LZMA's range encoder.
    struct Encoder {
        out: Vec<u8>,
        low: u64,
        range: u32,
        cache: u8,
        pending: u64,
    }

    impl Encoder {
        fn shift_low(&mut self) {
            if self.low < 0xFF00_0000 || self.low > 0xFFFF_FFFF {
                let carry = (self.low >> 32) as u8;
                let mut b = self.cache;
                loop {
                    self.out.push(b.wrapping_add(carry));
                    b = 0xFF;
                    self.pending -= 1;
                    if self.pending == 0 {
                        break;
                    }
                }
                self.cache = (self.low >> 24) as u8;
            }
            self.pending += 1;
            self.low = (self.low & 0x00FF_FFFF) << 8;
        }
    }

    impl Coder for Encoder {
        fn bit(&mut self, p: &mut u16, bit: u32) -> u32 {
            let bound = (self.range >> PROB_BITS) * u32::from(*p);
            if bit == 0 {
                self.range = bound;
            } else {
                self.low += u64::from(bound);
                self.range -= bound;
            }
            while self.range < 1 << 24 {
                self.range <<= 8;
                self.shift_low();
            }
            bit
        }
    }

    fn with_rate(planes: &[(&[u8], usize, usize)], rate: u8) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        out.extend([VERSION, rate, planes.len() as u8]);
        let mut all = Vec::new();
        for &(px, w, h) in planes {
            out.extend((w as u16).to_le_bytes());
            out.extend((h as u16).to_le_bytes());
            out.push(low_bits(&px[..w * h]) as u8);
            all.extend_from_slice(&px[..w * h]);
        }
        out.extend(adler32(&all).to_le_bytes());
        let head = out.len() + 1;
        let mut e = Encoder {
            out,
            low: 0,
            range: u32::MAX,
            cache: 0,
            pending: 1,
        };
        for &(px, w, h) in planes {
            let mut m = Model::new(rate);
            let low = low_bits(&px[..w * h]);
            let mut copy: Vec<u8> = px[..w * h].iter().map(|&v| v >> low).collect();
            walk(&mut e, &mut m, &mut copy, w, h);
        }
        for _ in 0..5 {
            e.shift_low();
        }
        // The flush's last bytes the decoder never reads: dropped.
        let mut out = e.out;
        while out.len() > head && read_back(&out[..out.len() - 1], planes) {
            out.pop();
        }
        out
    }

    /// The low bits clear in every byte of `px` (at most 7).
    fn low_bits(px: &[u8]) -> u32 {
        px.iter().fold(0u8, |o, &v| o | v).trailing_zeros().min(7)
    }

    /// `z` decodes to `planes` exactly.
    fn read_back(z: &[u8], planes: &[(&[u8], usize, usize)]) -> bool {
        let dims: Vec<(usize, usize)> = planes.iter().map(|&(_, w, h)| (w, h)).collect();
        let mut out = std::vec![0u8; dims.iter().map(|&(w, h)| w * h).sum()];
        crate::unpack_planes(z, &dims, &mut out).is_ok()
            && planes
                .iter()
                .flat_map(|&(p, w, h)| p[..w * h].iter())
                .eq(out.iter())
    }

    /// Packs `planes` (each its bytes, width and height, rows `width`
    /// apart) into one `RPPL` stream.
    ///
    /// # Panics
    /// More than [`MAX_PLANES`] planes, an empty one, one past 65,535 a
    /// side, or one shorter than its width times its height.
    #[must_use]
    pub fn pack_planes(planes: &[(&[u8], usize, usize)]) -> Vec<u8> {
        assert!(!planes.is_empty() && planes.len() <= MAX_PLANES);
        for &(px, w, h) in planes {
            assert!(w > 0 && h > 0 && w <= 0xFFFF && h <= 0xFFFF && px.len() >= w * h);
        }
        let z = RATES
            .map(|r| with_rate(planes, r))
            .min_by_key(Vec::len)
            .expect("at least one rate");
        // The packer reads its own stream back: a picture it can't
        // round-trip stops the build, never reaches an image.
        assert!(read_back(&z, planes), "RPPL: the stream doesn't read back");
        z
    }

    /// [`pack_planes`] at adaptation rate `rate` only (the tests try each).
    #[cfg(test)]
    pub(crate) fn pack_planes_at(planes: &[(&[u8], usize, usize)], rate: u8) -> Vec<u8> {
        with_rate(planes, rate)
    }
}
