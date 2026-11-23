//! A person's picture written as a JPEG (contacts-rule0.md "vCard 3 and
//! 4 import and export": a picture survives an export and an import back,
//! in any phone or in Outlook).
//!
//! Iterate, don't imitate (rule 0): a desktop links libjpeg (an encoder of
//! every option, its tables built in memory at start); WM kept the JPEG it
//! was given. Ours writes the one picture it ever needs, a 96-pixel square,
//! as a baseline 4:2:0 JPEG (what every reader takes) with the standard
//! tables of ITU T.81 Annex K (read byte for byte from a libjpeg-written
//! file, not typed) built at compile time into flash: no table in RAM, no
//! frame buffer (the pixels come from the caller one at a time, Contacts
//! reads them straight from its kept 6.9 KB disc), about 4 KB a person.
//! The forward DCT is the reader's own cosine table run the other way.

use crate::jpeg::SIDE;

/// The standard quality-50 tables, in zigzag order (Annex K.1).
const Q_LUMA: [u8; 64] = [
    16, 11, 12, 14, 12, 10, 16, 14, 13, 14, 18, 17, 16, 19, 24, 40, 26, 24, 22, 22, 24, 49, 35, 37,
    29, 40, 58, 51, 61, 60, 57, 51, 56, 55, 64, 72, 92, 78, 64, 68, 87, 69, 55, 56, 80, 109, 81,
    87, 95, 98, 103, 104, 103, 62, 77, 113, 121, 112, 100, 120, 92, 101, 103, 99,
];
const Q_CHROMA: [u8; 64] = {
    let mut q = [99u8; 64];
    let head = [17, 18, 18, 24, 21, 24, 47, 26, 26, 47, 99, 66, 56, 66];
    let mut k = 0;
    while k < head.len() {
        q[k] = head[k];
        k += 1;
    }
    q
};

/// The standard Huffman tables (Annex K.3): counts by length, symbols.
const DC_COUNTS: [[u8; 16]; 2] = [
    [0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0],
    [0, 3, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0],
];
const DC_SYMS: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
const AC_COUNTS: [[u8; 16]; 2] = [
    [0, 2, 1, 3, 3, 2, 4, 3, 5, 5, 4, 4, 0, 0, 1, 125],
    [0, 2, 1, 2, 4, 4, 3, 4, 7, 5, 4, 4, 0, 1, 2, 119],
];
const AC_SYMS: [[u8; 162]; 2] = [
    [
        0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61,
        0x07, 0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xa1, 0x08, 0x23, 0x42, 0xb1, 0xc1, 0x15, 0x52,
        0xd1, 0xf0, 0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0a, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x25,
        0x26, 0x27, 0x28, 0x29, 0x2a, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44, 0x45,
        0x46, 0x47, 0x48, 0x49, 0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x63, 0x64,
        0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x83,
        0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99,
        0x9a, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6,
        0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3,
        0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda, 0xe1, 0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8,
        0xe9, 0xea, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa,
    ],
    [
        0x00, 0x01, 0x02, 0x03, 0x11, 0x04, 0x05, 0x21, 0x31, 0x06, 0x12, 0x41, 0x51, 0x07, 0x61,
        0x71, 0x13, 0x22, 0x32, 0x81, 0x08, 0x14, 0x42, 0x91, 0xa1, 0xb1, 0xc1, 0x09, 0x23, 0x33,
        0x52, 0xf0, 0x15, 0x62, 0x72, 0xd1, 0x0a, 0x16, 0x24, 0x34, 0xe1, 0x25, 0xf1, 0x17, 0x18,
        0x19, 0x1a, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44,
        0x45, 0x46, 0x47, 0x48, 0x49, 0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x63,
        0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a,
        0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97,
        0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4,
        0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca,
        0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda, 0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7,
        0xe8, 0xe9, 0xea, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa,
    ],
];

/// A table's codes by symbol: (code, length), built at compile time.
type Codes = [(u16, u8); 256];

const fn codes(counts: &[u8; 16], syms: &[u8]) -> Codes {
    let mut out = [(0u16, 0u8); 256];
    let mut code = 0u16;
    let mut k = 0;
    let mut len = 0;
    while len < 16 {
        let mut n = counts[len];
        while n > 0 {
            out[syms[k] as usize] = (code, len as u8 + 1);
            code += 1;
            k += 1;
            n -= 1;
        }
        code <<= 1;
        len += 1;
    }
    out
}

static DC_CODES: [Codes; 2] = [
    codes(&DC_COUNTS[0], &DC_SYMS),
    codes(&DC_COUNTS[1], &DC_SYMS),
];
static AC_CODES: [Codes; 2] = [
    codes(&AC_COUNTS[0], &AC_SYMS[0]),
    codes(&AC_COUNTS[1], &AC_SYMS[1]),
];

/// The zigzag order's natural places (as the reader's).
const ZIGZAG: [u8; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20,
    13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59,
    52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

/// `C(u) cos((2x + 1) u pi / 16)` times 4096 (the reader's K8).
const K8: [[i32; 8]; 8] = [
    [2896, 4017, 3784, 3406, 2896, 2276, 1567, 799],
    [2896, 3406, 1567, -799, -2896, -4017, -3784, -2276],
    [2896, 2276, -1567, -4017, -2896, 799, 3784, 3406],
    [2896, 799, -3784, -2276, 2896, 3406, -1567, -4017],
    [2896, -799, -3784, 2276, 2896, -3406, -1567, 4017],
    [2896, -2276, -1567, 4017, -2896, -799, 3784, -3406],
    [2896, -3406, 1567, 799, -2896, 4017, -3784, 2276],
    [2896, -4017, 3784, -3406, 2896, -2276, 1567, -799],
];

/// A quality-50 table scaled to `quality` (libjpeg's curve), zigzag order.
fn scaled(base: &[u8; 64], quality: u8) -> [u8; 64] {
    let q = u32::from(quality.clamp(1, 100));
    let s = if q < 50 { 5000 / q } else { 200 - 2 * q };
    let mut out = [1u8; 64];
    for (o, &b) in out.iter_mut().zip(base) {
        *o = ((u32::from(b) * s + 50) / 100).clamp(1, 255) as u8;
    }
    out
}

/// Bytes into the caller's buffer: `None` once past its end.
struct Out<'a> {
    b: &'a mut [u8],
    n: usize,
    full: bool,
    acc: u32,
    bits: u32,
}

impl Out<'_> {
    fn put(&mut self, s: &[u8]) {
        for &c in s {
            match self.b.get_mut(self.n) {
                Some(o) => {
                    *o = c;
                    self.n += 1;
                }
                None => self.full = true,
            }
        }
    }
    /// `len` bits of `v`, a 0xFF in the data followed by a stuffed 0.
    fn bits(&mut self, v: u32, len: u32) {
        if len == 0 {
            return;
        }
        self.acc = self.acc << len | (v & ((1 << len) - 1));
        self.bits += len;
        while self.bits >= 8 {
            self.bits -= 8;
            let byte = (self.acc >> self.bits) as u8;
            self.put(&[byte]);
            if byte == 0xFF {
                self.put(&[0]);
            }
        }
    }
    /// The last bits padded with ones.
    fn flush(&mut self) {
        if self.bits > 0 {
            let pad = 8 - self.bits;
            self.bits(0x7F, pad);
        }
    }
}

/// One 8x8 block (level-shifted samples) coded: its DC from `pred`.
fn block(o: &mut Out<'_>, f: &[i32; 64], q: &[u8; 64], chroma: usize, pred: &mut i32) {
    // The forward DCT: F(u,v) = 1/4 sum K[x][u] K[y][v] f(x,y).
    let mut tmp = [0i64; 64];
    for y in 0..8 {
        for u in 0..8 {
            let mut a = 0i64;
            for x in 0..8 {
                a += i64::from(K8[x][u]) * i64::from(f[y * 8 + x]);
            }
            tmp[y * 8 + u] = a;
        }
    }
    let mut z = [0i32; 64];
    for k in 0..64 {
        let nat = usize::from(ZIGZAG[k]);
        let (v, u) = (nat / 8, nat % 8);
        let mut a = 0i64;
        for y in 0..8 {
            a += i64::from(K8[y][v]) * tmp[y * 8 + u];
        }
        // 1/4, the two 4096s and the step, rounded to nearest (integers:
        // the PXA270 has no floating point).
        let d = 4 * 4096 * 4096 * i64::from(q[k]);
        z[k] = ((a + if a < 0 { -d / 2 } else { d / 2 }) / d) as i32;
    }
    let size = |v: i32| 32 - v.unsigned_abs().leading_zeros();
    let amp = |v: i32, n: u32| if v < 0 { (v - 1) as u32 } else { v as u32 } & ((1u32 << n) - 1);
    // DC.
    let diff = z[0] - *pred;
    *pred = z[0];
    let n = size(diff);
    let (code, len) = DC_CODES[chroma][n as usize];
    o.bits(u32::from(code), u32::from(len));
    o.bits(amp(diff, n), n);
    // AC, runs of zeros.
    let mut run = 0;
    for &v in &z[1..] {
        if v == 0 {
            run += 1;
            continue;
        }
        while run > 15 {
            let (code, len) = AC_CODES[chroma][0xF0];
            o.bits(u32::from(code), u32::from(len));
            run -= 16;
        }
        let n = size(v).min(10);
        let (code, len) = AC_CODES[chroma][(run << 4 | n) as usize];
        o.bits(u32::from(code), u32::from(len));
        o.bits(amp(v, n), n);
        run = 0;
    }
    if run > 0 {
        let (code, len) = AC_CODES[chroma][0];
        o.bits(u32::from(code), u32::from(len));
    }
}

/// The square ([`SIDE`] pixels, red, green, blue from `px`) written as a
/// baseline JPEG into `out` at `quality` (1-100): its length, or `None`
/// when it doesn't fit (a lower quality then).
#[inline(never)]
pub fn encode(
    px: &dyn Fn(usize, usize) -> (u8, u8, u8),
    quality: u8,
    out: &mut [u8],
) -> Option<usize> {
    let ql = scaled(&Q_LUMA, quality);
    let qc = scaled(&Q_CHROMA, quality);
    let mut o = Out {
        b: out,
        n: 0,
        full: false,
        acc: 0,
        bits: 0,
    };
    o.put(&[0xFF, 0xD8]);
    o.put(&[
        0xFF, 0xE0, 0, 16, b'J', b'F', b'I', b'F', 0, 1, 1, 0, 0, 1, 0, 1, 0, 0,
    ]);
    for (t, q) in [(0u8, &ql), (1, &qc)] {
        o.put(&[0xFF, 0xDB, 0, 67, t]);
        o.put(q);
    }
    let side = SIDE as u8;
    o.put(&[
        0xFF, 0xC0, 0, 17, 8, 0, side, 0, side, 3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1,
    ]);
    for (class, t, counts, syms) in [
        (0u8, 0u8, &DC_COUNTS[0], &DC_SYMS[..]),
        (1, 0, &AC_COUNTS[0], &AC_SYMS[0][..]),
        (0, 1, &DC_COUNTS[1], &DC_SYMS[..]),
        (1, 1, &AC_COUNTS[1], &AC_SYMS[1][..]),
    ] {
        let len = 2 + 1 + 16 + syms.len();
        o.put(&[0xFF, 0xC4, (len >> 8) as u8, len as u8, class << 4 | t]);
        o.put(counts);
        o.put(syms);
    }
    o.put(&[0xFF, 0xDA, 0, 12, 3, 1, 0x00, 2, 0x11, 3, 0x11, 0, 63, 0]);
    let mut preds = [0i32; 3];
    let ycc = |x: usize, y: usize| {
        let (r, g, b) = px(x, y);
        let (r, g, b) = (i32::from(r), i32::from(g), i32::from(b));
        (
            (19_595 * r + 38_470 * g + 7_471 * b + 32_768) >> 16,
            ((-11_059 * r - 21_709 * g + 32_768 * b + 32_768) >> 16) + 128,
            ((32_768 * r - 27_439 * g - 5_329 * b + 32_768) >> 16) + 128,
        )
    };
    for my in 0..SIDE / 16 {
        for mx in 0..SIDE / 16 {
            // Four luma blocks, then the two colours at a quarter.
            for b in 0..4 {
                let mut f = [0i32; 64];
                for (k, v) in f.iter_mut().enumerate() {
                    let (x, y) = (mx * 16 + (b % 2) * 8 + k % 8, my * 16 + (b / 2) * 8 + k / 8);
                    *v = ycc(x, y).0 - 128;
                }
                block(&mut o, &f, &ql, 0, &mut preds[0]);
            }
            for (c, pred) in preds.iter_mut().enumerate().skip(1) {
                let mut f = [0i32; 64];
                for (k, v) in f.iter_mut().enumerate() {
                    let (x, y) = (mx * 16 + (k % 8) * 2, my * 16 + (k / 8) * 2);
                    let mut s = 0;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let p = ycc(x + dx, y + dy);
                        s += if c == 1 { p.1 } else { p.2 };
                    }
                    *v = (s + 2) / 4 - 128;
                }
                block(&mut o, &f, &qc, 1, pred);
            }
        }
    }
    o.flush();
    o.put(&[0xFF, 0xD9]);
    (!o.full).then_some(o.n)
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use crate::jpeg::{Decoder, Progress, WORDS};

    fn scene(x: usize, y: usize) -> (u8, u8, u8) {
        let r = (x * 255 / 95) as u8;
        let g = (y * 255 / 95) as u8;
        let b = if (x / 12 + y / 12).is_multiple_of(2) {
            220
        } else {
            40
        };
        (r, g, b)
    }

    #[test]
    fn a_square_written_and_read_back_by_the_reader_is_the_square() {
        let mut buf = std::vec![0u8; 16 * 1024];
        for quality in [95u8, 85, 50] {
            let n = encode(&scene, quality, &mut buf).expect("fits");
            let mut d = Decoder::new();
            let mut acc = std::vec![0u32; WORDS];
            assert_eq!(d.feed(&buf[..n], true, &mut acc), Ok(Progress::Done));
            assert_eq!(d.size(), (96, 96));
            let mut total = 0u64;
            for y in 0..96 {
                for x in 0..96 {
                    let (a, b) = (d.rgb(&acc, x, y), scene(x, y));
                    total += u64::from(a.0.abs_diff(b.0))
                        + u64::from(a.1.abs_diff(b.1))
                        + u64::from(a.2.abs_diff(b.2));
                }
            }
            let mean = total as f64 / (96.0 * 96.0 * 3.0);
            // (Quality 50 on hard-edged blue squares at a quarter of the
            // colour: the loss any JPEG has there.)
            let limit = if quality >= 85 { 6.0 } else { 14.0 };
            if let Ok(p) = std::env::var("RP_JPEG_OUT") {
                std::fs::write(std::format!("{p}-{quality}.jpg"), &buf[..n]).unwrap();
            }
            assert!(
                mean < limit,
                "quality {quality}: {mean:.2} levels off, {n} bytes"
            );
            if quality == 85 {
                assert!(n < 8 * 1024, "{n} bytes");
            }
        }
        // Too small a buffer: refused, never cut.
        assert_eq!(encode(&scene, 85, &mut [0u8; 600]), None);
    }

    #[test]
    fn its_huffman_tables_are_the_standard_prefix_codes() {
        // Every code fits its length and no two symbols share a code.
        for t in DC_CODES.iter().chain(&AC_CODES) {
            let used: std::vec::Vec<_> = t.iter().filter(|c| c.1 > 0).collect();
            for (k, a) in used.iter().enumerate() {
                assert!(u32::from(a.0) < 1 << a.1);
                for b in &used[k + 1..] {
                    let n = a.1.min(b.1);
                    assert_ne!(a.0 >> (a.1 - n), b.0 >> (b.1 - n), "a prefix of another");
                }
            }
        }
        assert_eq!(AC_CODES[0][0].1, 4, "EOB is 1010");
        assert_eq!(AC_CODES[0][0].0, 0b1010);
    }
}
