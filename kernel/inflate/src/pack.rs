//! The PC side (feature `pack`, for build scripts): zlib packing with
//! LZ77 (hash chains over a 32 KiB window) and one dynamic Huffman block
//! per 64 Ki symbols, and PNG's Paeth filter. Within a few percent of
//! `zlib -9` on the test image's wallpaper; what it makes, [`zlib`]
//! (`super::zlib`) reads, and the tests check both ways.

extern crate std;

use std::vec;
use std::vec::Vec;

use super::{DIST_BASE, DIST_EXTRA, LEN_BASE, LEN_EXTRA, ORDER, adler32, paeth};

/// Writes bits least significant first.
struct Out {
    bytes: Vec<u8>,
    buf: u64,
    n: u32,
}

impl Out {
    fn put(&mut self, v: u32, k: u32) {
        self.buf |= u64::from(v) << self.n;
        self.n += k;
        while self.n >= 8 {
            self.bytes.push(self.buf as u8);
            self.buf >>= 8;
            self.n -= 8;
        }
    }

    /// A Huffman code, stored most significant bit first.
    fn code(&mut self, code: u32, len: u8) {
        let mut rev = 0;
        for k in 0..len {
            rev |= ((code >> k) & 1) << (len - 1 - k);
        }
        self.put(rev, u32::from(len));
    }

    fn flush(&mut self) {
        if self.n > 0 {
            self.bytes.push(self.buf as u8);
            self.buf = 0;
            self.n = 0;
        }
    }
}

/// Code lengths (at most `max`) for `freq`: Huffman's, the counts halved
/// until the longest fits. Unused symbols get 0.
fn lengths(freq: &[u32], max: u8) -> Vec<u8> {
    let mut f: Vec<u32> = freq.to_vec();
    loop {
        let mut nodes: Vec<(u32, Vec<usize>)> = f
            .iter()
            .enumerate()
            .filter(|&(_, &c)| c > 0)
            .map(|(i, &c)| (c, vec![i]))
            .collect();
        let mut len = vec![0u8; f.len()];
        if nodes.len() == 1 {
            len[nodes[0].1[0]] = 1;
            return len;
        }
        while nodes.len() > 1 {
            nodes.sort_by_key(|n| core::cmp::Reverse(n.0));
            let (Some(a), Some(b)) = (nodes.pop(), nodes.pop()) else {
                break;
            };
            for &s in a.1.iter().chain(&b.1) {
                len[s] += 1;
            }
            let mut syms = a.1;
            syms.extend(b.1);
            nodes.push((a.0 + b.0, syms));
        }
        if len.iter().all(|&l| l <= max) {
            return len;
        }
        for c in &mut f {
            if *c > 0 {
                *c = c.div_ceil(2);
            }
        }
    }
}

/// Canonical codes for `len` (RFC 1951 3.2.2).
fn codes(len: &[u8]) -> Vec<u32> {
    let mut count = [0u32; 16];
    for &l in len {
        count[usize::from(l)] += 1;
    }
    count[0] = 0;
    let mut next = [0u32; 16];
    let mut code = 0;
    for bits in 1..16 {
        code = (code + count[bits - 1]) << 1;
        next[bits] = code;
    }
    len.iter()
        .map(|&l| {
            if l == 0 {
                0
            } else {
                let c = next[usize::from(l)];
                next[usize::from(l)] += 1;
                c
            }
        })
        .collect()
}

/// One LZ77 item: a literal, or a (length, distance) copy.
#[derive(Clone, Copy)]
enum Item {
    Lit(u8),
    Copy(u16, u16),
}

fn len_code(len: u16) -> usize {
    LEN_BASE.iter().rposition(|&b| b <= len).unwrap_or(0)
}

fn dist_code(d: u16) -> usize {
    DIST_BASE.iter().rposition(|&b| b <= d).unwrap_or(0)
}

/// LZ77 over `data`: the longest match among the last `depth` positions
/// with the same three bytes.
fn matches(data: &[u8], depth: usize) -> Vec<Item> {
    const WINDOW: usize = 32 * 1024;
    let mut head = vec![usize::MAX; 1 << 16];
    let mut prev = vec![usize::MAX; data.len()];
    let hash = |i: usize| {
        (usize::from(data[i]) << 8 ^ usize::from(data[i + 1]) << 4 ^ usize::from(data[i + 2]))
            & 0xFFFF
    };
    let mut items = Vec::new();
    let mut i = 0;
    let insert = |i: usize, head: &mut Vec<usize>, prev: &mut Vec<usize>| {
        if i + 2 < data.len() {
            let h = hash(i);
            prev[i] = head[h];
            head[h] = i;
        }
    };
    while i < data.len() {
        let mut best = (0usize, 0usize);
        if i + 2 < data.len() {
            let mut cand = head[hash(i)];
            let mut tries = depth;
            while cand != usize::MAX && i - cand <= WINDOW && tries > 0 {
                let max = (data.len() - i).min(258);
                let n = (0..max)
                    .take_while(|&k| data[cand + k] == data[i + k])
                    .count();
                if n > best.0 {
                    best = (n, i - cand);
                    if n == max {
                        break;
                    }
                }
                cand = prev[cand];
                tries -= 1;
            }
        }
        if best.0 >= 3 {
            items.push(Item::Copy(best.0 as u16, best.1 as u16));
            for k in i..i + best.0 {
                insert(k, &mut head, &mut prev);
            }
            i += best.0;
        } else {
            items.push(Item::Lit(data[i]));
            insert(i, &mut head, &mut prev);
            i += 1;
        }
    }
    items
}

/// One dynamic block of `items`.
fn block(out: &mut Out, items: &[Item], last: bool) {
    let mut lf = [0u32; 286];
    let mut df = [0u32; 30];
    for it in items {
        match *it {
            Item::Lit(b) => lf[usize::from(b)] += 1,
            Item::Copy(l, d) => {
                lf[257 + len_code(l)] += 1;
                df[dist_code(d)] += 1;
            }
        }
    }
    lf[256] += 1;
    if df.iter().all(|&c| c == 0) {
        df[0] = 1;
    }
    let ll = lengths(&lf, 15);
    let dl = lengths(&df, 15);
    let nlen = ll.iter().rposition(|&l| l > 0).unwrap_or(256).max(256) + 1;
    let ndist = dl.iter().rposition(|&l| l > 0).unwrap_or(0) + 1;
    // The code lengths are sent plainly (symbols 0-15, no runs).
    let mut cf = [0u32; 19];
    for &l in ll[..nlen].iter().chain(&dl[..ndist]) {
        cf[usize::from(l)] += 1;
    }
    let cl = lengths(&cf, 7);
    let cc = codes(&cl);
    let ncode = ORDER.iter().rposition(|&k| cl[k] > 0).unwrap_or(0).max(3) + 1;
    out.put(u32::from(last), 1);
    out.put(2, 2);
    out.put((nlen - 257) as u32, 5);
    out.put((ndist - 1) as u32, 5);
    out.put((ncode - 4) as u32, 4);
    for &k in &ORDER[..ncode] {
        out.put(u32::from(cl[k]), 3);
    }
    for &l in ll[..nlen].iter().chain(&dl[..ndist]) {
        out.code(cc[usize::from(l)], cl[usize::from(l)]);
    }
    let (lc, dc) = (codes(&ll), codes(&dl));
    for it in items {
        match *it {
            Item::Lit(b) => out.code(lc[usize::from(b)], ll[usize::from(b)]),
            Item::Copy(l, d) => {
                let k = len_code(l);
                out.code(lc[257 + k], ll[257 + k]);
                out.put(u32::from(l - LEN_BASE[k]), u32::from(LEN_EXTRA[k]));
                let k = dist_code(d);
                out.code(dc[k], dl[k]);
                out.put(u32::from(d - DIST_BASE[k]), u32::from(DIST_EXTRA[k]));
            }
        }
    }
    out.code(lc[256], ll[256]);
}

/// `data` as zlib (header, dynamic blocks, Adler-32).
#[must_use]
pub fn zlib(data: &[u8]) -> Vec<u8> {
    let mut out = Out {
        bytes: vec![0x78, 0xDA],
        buf: 0,
        n: 0,
    };
    let items = matches(data, 64);
    let chunks: Vec<&[Item]> = items.chunks(64 * 1024).collect();
    if chunks.is_empty() {
        block(&mut out, &[], true);
    }
    for (k, c) in chunks.iter().enumerate() {
        block(&mut out, c, k + 1 == chunks.len());
    }
    out.flush();
    out.bytes.extend(adler32(data).to_be_bytes());
    out.bytes
}

/// PNG's Paeth filter on a `w` x `h` plane: each byte as its difference
/// from the prediction ([`super::unpaeth`] undoes it).
#[must_use]
pub fn paeth_filter(px: &[u8], w: usize, h: usize) -> Vec<u8> {
    let at = |x: usize, y: usize| px[y * w + x];
    let mut f = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let a = if x > 0 { at(x - 1, y) } else { 0 };
            let b = if y > 0 { at(x, y - 1) } else { 0 };
            let c = if x > 0 && y > 0 { at(x - 1, y - 1) } else { 0 };
            f[y * w + x] = at(x, y).wrapping_sub(paeth(a, b, c));
        }
    }
    f
}
