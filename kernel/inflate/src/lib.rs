//! Deflate (RFC 1951) and zlib (RFC 1950) decoding, clean-room from the
//! RFCs, for assets packed on the PC and unpacked once on the device (the
//! test image's wallpaper: sheet `docs/research/sheets/today-image-room.md`),
//! and [`unpaeth`], PNG's Paeth predictor undone.
//!
//! - **Safe and bounded:** any input, however damaged, either decodes into
//!   `dst` or returns an [`Error`]; never a panic, never a write past
//!   `dst`, never a read past `src`. Every loop is bounded by the output
//!   or the input.
//! - **Small:** canonical Huffman codes decoded from their counts per
//!   length (RFC 1951 3.2.2), stored, fixed and dynamic blocks; no
//!   dictionary, no streaming. The packer is Python's `zlib`
//!   (`tools/shell-assets/pack.py`).

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

#[cfg(any(test, feature = "pack"))]
pub mod pack;

/// Why data couldn't be decoded.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    /// The input ended mid-block.
    Truncated,
    /// The output doesn't fit in `dst`.
    Full,
    /// A block type, code set, length or distance the format forbids.
    Bad,
    /// A back-reference before the start of the output.
    Distance,
    /// The zlib header is wrong or asks for a preset dictionary.
    Header,
    /// The zlib checksum doesn't match what was decoded.
    Checksum,
}

/// Reads bits, least significant first, as deflate stores them.
struct Bits<'a> {
    src: &'a [u8],
    at: usize,
    buf: u32,
    n: u32,
}

impl Bits<'_> {
    fn need(&mut self, k: u32) -> Result<u32, Error> {
        while self.n < k {
            let b = *self.src.get(self.at).ok_or(Error::Truncated)?;
            self.at += 1;
            self.buf |= u32::from(b) << self.n;
            self.n += 8;
        }
        let v = self.buf & ((1u32 << k) - 1);
        self.buf >>= k;
        self.n -= k;
        Ok(v)
    }

    /// Drops the bits left in the current byte (a stored block's start).
    fn align(&mut self) {
        self.buf = 0;
        self.n = 0;
    }
}

/// A canonical Huffman code: how many codes of each length (1-15) and the
/// symbols in code order.
struct Huffman {
    count: [u16; 16],
    symbol: [u16; 320],
}

impl Huffman {
    /// The code for `lengths` (one per symbol, 0: unused). An
    /// over-subscribed set is refused; an incomplete one decodes until a
    /// missing code is met, which fails.
    fn new(lengths: &[u8]) -> Result<Self, Error> {
        let mut h = Self {
            count: [0; 16],
            symbol: [0; 320],
        };
        if lengths.len() > h.symbol.len() {
            return Err(Error::Bad);
        }
        for &l in lengths {
            h.count[usize::from(l & 15)] += 1;
        }
        h.count[0] = 0;
        let mut left: i32 = 1;
        for &c in &h.count[1..] {
            left = left * 2 - i32::from(c);
            if left < 0 {
                return Err(Error::Bad);
            }
        }
        let mut offs = [0u16; 16];
        for len in 1..15 {
            offs[len + 1] = offs[len] + h.count[len];
        }
        for (sym, &l) in lengths.iter().enumerate() {
            if l != 0 {
                let o = &mut offs[usize::from(l & 15)];
                h.symbol[usize::from(*o)] = sym as u16;
                *o += 1;
            }
        }
        Ok(h)
    }

    fn decode(&self, b: &mut Bits<'_>) -> Result<u16, Error> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for &count in &self.count[1..] {
            code |= b.need(1)? as i32;
            let count = i32::from(count);
            if code - count < first {
                return self
                    .symbol
                    .get((index + code - first) as usize)
                    .copied()
                    .ok_or(Error::Bad);
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err(Error::Bad)
    }
}

const LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LEN_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

/// The order code-length codes are stored in (RFC 1951 3.2.7).
const ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

/// Decodes one block's symbols into `dst` from `*out`.
fn codes(
    b: &mut Bits<'_>,
    dst: &mut [u8],
    out: &mut usize,
    lit: &Huffman,
    dist: &Huffman,
) -> Result<(), Error> {
    loop {
        let sym = lit.decode(b)?;
        match sym {
            0..=255 => {
                *dst.get_mut(*out).ok_or(Error::Full)? = sym as u8;
                *out += 1;
            }
            256 => return Ok(()),
            _ => {
                let i = usize::from(sym - 257);
                let (Some(&base), Some(&extra)) = (LEN_BASE.get(i), LEN_EXTRA.get(i)) else {
                    return Err(Error::Bad);
                };
                let len = usize::from(base) + b.need(u32::from(extra))? as usize;
                let d = usize::from(dist.decode(b)?);
                let (Some(&base), Some(&extra)) = (DIST_BASE.get(d), DIST_EXTRA.get(d)) else {
                    return Err(Error::Bad);
                };
                let back = usize::from(base) + b.need(u32::from(extra))? as usize;
                if back > *out {
                    return Err(Error::Distance);
                }
                if *out + len > dst.len() {
                    return Err(Error::Full);
                }
                let from = *out - back;
                if back >= len {
                    dst.copy_within(from..from + len, *out);
                } else {
                    // Overlapping: each byte reads one just written.
                    for k in 0..len {
                        dst[*out + k] = dst[from + k];
                    }
                }
                *out += len;
            }
        }
    }
}

/// The fixed code's literal and length lengths (RFC 1951 3.2.6).
fn fixed_lengths() -> [u8; 288] {
    let mut l = [8u8; 288];
    l[144..256].fill(9);
    l[256..280].fill(7);
    l
}

/// Decodes raw deflate data `src` into `dst`; the bytes written.
///
/// # Errors
/// Any damage to `src`, or output larger than `dst` ([`Error`]).
pub fn inflate(src: &[u8], dst: &mut [u8]) -> Result<usize, Error> {
    let mut b = Bits {
        src,
        at: 0,
        buf: 0,
        n: 0,
    };
    let mut out = 0;
    loop {
        let last = b.need(1)? == 1;
        match b.need(2)? {
            0 => {
                b.align();
                let at = b.at;
                let head = src.get(at..at + 4).ok_or(Error::Truncated)?;
                let len = usize::from(u16::from_le_bytes([head[0], head[1]]));
                let nlen = u16::from_le_bytes([head[2], head[3]]);
                if nlen != !(len as u16) {
                    return Err(Error::Bad);
                }
                let data = src.get(at + 4..at + 4 + len).ok_or(Error::Truncated)?;
                dst.get_mut(out..out + len)
                    .ok_or(Error::Full)?
                    .copy_from_slice(data);
                out += len;
                b.at = at + 4 + len;
            }
            1 => {
                let lit = Huffman::new(&fixed_lengths())?;
                let dist = Huffman::new(&[5u8; 30])?;
                codes(&mut b, dst, &mut out, &lit, &dist)?;
            }
            2 => {
                let nlen = b.need(5)? as usize + 257;
                let ndist = b.need(5)? as usize + 1;
                let ncode = b.need(4)? as usize + 4;
                if nlen > 286 || ndist > 30 {
                    return Err(Error::Bad);
                }
                let mut cl = [0u8; 19];
                for &k in &ORDER[..ncode] {
                    cl[k] = b.need(3)? as u8;
                }
                let lencode = Huffman::new(&cl)?;
                let mut l = [0u8; 316];
                let mut i = 0;
                while i < nlen + ndist {
                    let sym = lencode.decode(&mut b)?;
                    let (v, n) = match sym {
                        0..=15 => (sym as u8, 1),
                        16 => {
                            let prev = *l.get(i.wrapping_sub(1)).ok_or(Error::Bad)?;
                            (prev, 3 + b.need(2)? as usize)
                        }
                        17 => (0, 3 + b.need(3)? as usize),
                        _ => (0, 11 + b.need(7)? as usize),
                    };
                    if i + n > nlen + ndist {
                        return Err(Error::Bad);
                    }
                    l[i..i + n].fill(v);
                    i += n;
                }
                if l[256] == 0 {
                    return Err(Error::Bad);
                }
                let lit = Huffman::new(&l[..nlen])?;
                let dist = Huffman::new(&l[nlen..nlen + ndist])?;
                codes(&mut b, dst, &mut out, &lit, &dist)?;
            }
            _ => return Err(Error::Bad),
        }
        if last {
            return Ok(out);
        }
    }
}

/// Adler-32 (RFC 1950) of `data`.
#[must_use]
pub fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in data.chunks(5552) {
        for &x in chunk {
            a += u32::from(x);
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    b << 16 | a
}

/// Decodes zlib data `src` (header, deflate, Adler-32) into `dst`; the
/// bytes written, checked against the checksum.
///
/// # Errors
/// A wrong header or checksum, or any [`inflate`] error.
pub fn zlib(src: &[u8], dst: &mut [u8]) -> Result<usize, Error> {
    let [cmf, flg, ..] = *src else {
        return Err(Error::Truncated);
    };
    let check = (u16::from(cmf) << 8 | u16::from(flg)) % 31;
    if cmf & 0x0F != 8 || cmf >> 4 > 7 || check != 0 || flg & 0x20 != 0 {
        return Err(Error::Header);
    }
    let n = inflate(&src[2..], dst)?;
    let tail = src.len().checked_sub(4).ok_or(Error::Truncated)?;
    let want = u32::from_be_bytes([src[tail], src[tail + 1], src[tail + 2], src[tail + 3]]);
    if adler32(&dst[..n]) != want {
        return Err(Error::Checksum);
    }
    Ok(n)
}

/// Undoes PNG's Paeth filter (RFC 2083 filter type 4) in place on a
/// `w` x `h` plane of bytes, rows `w` apart: each byte was stored as its
/// difference from the Paeth prediction of its left, upper and upper-left
/// neighbours (0 off the edges). A plane shorter than `w * h` is left as
/// it is.
pub fn unpaeth(px: &mut [u8], w: usize, h: usize) {
    if px.len() < w * h {
        return;
    }
    for y in 0..h {
        for x in 0..w {
            let a = if x > 0 { px[y * w + x - 1] } else { 0 };
            let b = if y > 0 { px[(y - 1) * w + x] } else { 0 };
            let c = if x > 0 && y > 0 {
                px[(y - 1) * w + x - 1]
            } else {
                0
            };
            px[y * w + x] = px[y * w + x].wrapping_add(paeth(a, b, c));
        }
    }
}

/// The Paeth predictor: whichever of left `a`, up `b` and up-left `c` is
/// nearest `a + b - c`, ties to `a`, then `b`.
#[must_use]
pub fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let (ia, ib, ic) = (i16::from(a), i16::from(b), i16::from(c));
    let p = ia + ib - ic;
    let (pa, pb, pc) = ((p - ia).abs(), (p - ib).abs(), (p - ic).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

#[cfg(test)]
mod tests;
