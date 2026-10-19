//! DEFLATE decoding (RFC 1951) for MSZIP blocks.
//!
//! Written from the RFC. Decodes into a window buffer whose bytes before
//! `start` are the history (MSZIP carries the previous 32 KiB of output from
//! block to block), so back-references may reach into earlier blocks but
//! never before the start of the window.
//!
//! Hostile input is expected: every length, distance and table count is
//! checked, reading past the input is an error (never zero-filled, so no
//! loop can run without consuming input), and output beyond `limit` is an
//! error. Huffman decoding walks the canonical code one bit at a time
//! against per-length counts (no lookup tables to size, ~1.2 KiB of state on
//! the stack).

use crate::Error;

/// Longest Huffman code DEFLATE allows.
const MAX_BITS: usize = 15;
/// Literal/length symbols (286 used; 286 and 287 appear only in fixed codes).
const MAX_LITLEN: usize = 288;
/// Distance symbols (30 used; 30 and 31 appear only in fixed codes).
const MAX_DIST: usize = 32;

/// Base lengths for length symbols 257..=285.
const LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
/// Extra bits for length symbols 257..=285.
const LEN_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
/// Base distances for distance symbols 0..=29.
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
/// Extra bits for distance symbols 0..=29.
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];
/// The order code-length code lengths are sent in.
const CLEN_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

/// LSB-first bit reader that refuses to read past its input.
struct Bits<'a> {
    input: &'a [u8],
    pos: usize,
    acc: u32,
    count: u32,
}

impl<'a> Bits<'a> {
    fn new(input: &'a [u8]) -> Self {
        Bits {
            input,
            pos: 0,
            acc: 0,
            count: 0,
        }
    }

    /// Reads `n` (≤ 16) bits.
    fn take(&mut self, n: u32) -> Result<u32, Error> {
        while self.count < n {
            let byte = *self.input.get(self.pos).ok_or(Error::Corrupt)?;
            self.pos += 1;
            self.acc |= u32::from(byte) << self.count;
            self.count += 8;
        }
        let v = self.acc & ((1u32 << n) - 1);
        self.acc >>= n;
        self.count -= n;
        Ok(v)
    }

    /// Drops the bits left in the current byte (stored blocks start on a
    /// byte boundary).
    fn align(&mut self) {
        let drop = self.count % 8;
        self.acc >>= drop;
        self.count -= drop;
    }
}

/// A canonical Huffman code: how many codes of each length, and the symbols
/// in code order.
struct Huffman<const N: usize> {
    counts: [u16; MAX_BITS + 1],
    symbols: [u16; N],
}

impl<const N: usize> Huffman<N> {
    /// Builds the code from per-symbol lengths (0 = unused). Over-subscribed
    /// sets are refused; incomplete ones are allowed (RFC 1951 permits a
    /// single distance code) and fail only if an unassigned code is read.
    fn new(lengths: &[u8]) -> Result<Self, Error> {
        let mut h = Huffman {
            counts: [0; MAX_BITS + 1],
            symbols: [0; N],
        };
        if lengths.len() > N {
            return Err(Error::Corrupt);
        }
        for &l in lengths {
            if usize::from(l) > MAX_BITS {
                return Err(Error::Corrupt);
            }
            h.counts[usize::from(l)] += 1;
        }
        h.counts[0] = 0;
        let mut left: i32 = 1;
        for &count in &h.counts[1..] {
            left = (left << 1) - i32::from(count);
            if left < 0 {
                return Err(Error::Corrupt);
            }
        }
        let mut offs = [0u16; MAX_BITS + 2];
        for len in 1..=MAX_BITS {
            offs[len + 1] = offs[len] + h.counts[len];
        }
        for (sym, &l) in lengths.iter().enumerate() {
            if l != 0 {
                let slot = &mut offs[usize::from(l)];
                h.symbols[usize::from(*slot)] = sym as u16;
                *slot += 1;
            }
        }
        Ok(h)
    }

    /// Decodes one symbol.
    fn decode(&self, bits: &mut Bits<'_>) -> Result<u16, Error> {
        let mut code: i32 = 0;
        let mut first: i32 = 0;
        let mut index: i32 = 0;
        for len in 1..=MAX_BITS {
            code |= bits.take(1)? as i32;
            let count = i32::from(self.counts[len]);
            if code - first < count {
                return Ok(self.symbols[(index + code - first) as usize]);
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err(Error::Corrupt)
    }
}

/// Decodes one MSZIP block's DEFLATE data into `window[start..]`.
///
/// `window[..start]` is the history. Decoding stops after the final DEFLATE
/// block, or at a block boundary once `limit` bytes have been written.
/// Returns the number of bytes written; writing more than `limit` (or past
/// the window) is an error.
pub(crate) fn inflate(
    input: &[u8],
    window: &mut [u8],
    start: usize,
    limit: usize,
) -> Result<usize, Error> {
    let end = start.checked_add(limit).ok_or(Error::Corrupt)?;
    if end > window.len() {
        return Err(Error::Corrupt);
    }
    let mut bits = Bits::new(input);
    let mut pos = start;
    loop {
        let last = bits.take(1)? == 1;
        match bits.take(2)? {
            0 => stored(&mut bits, window, &mut pos, end)?,
            1 => {
                let (lit, dist) = fixed_codes()?;
                codes(&mut bits, window, &mut pos, end, &lit, &dist)?;
            }
            2 => {
                let (lit, dist) = dynamic_codes(&mut bits)?;
                codes(&mut bits, window, &mut pos, end, &lit, &dist)?;
            }
            _ => return Err(Error::Corrupt),
        }
        if last || pos == end {
            return Ok(pos - start);
        }
    }
}

fn stored(
    bits: &mut Bits<'_>,
    window: &mut [u8],
    pos: &mut usize,
    end: usize,
) -> Result<(), Error> {
    bits.align();
    let len = bits.take(16)?;
    let nlen = bits.take(16)?;
    if len != !nlen & 0xFFFF {
        return Err(Error::Corrupt);
    }
    // After `align` and two 16-bit reads the accumulator is empty: copy
    // straight from the input.
    debug_assert_eq!(bits.count, 0);
    let len = len as usize;
    let src = bits
        .input
        .get(bits.pos..bits.pos + len)
        .ok_or(Error::Corrupt)?;
    let dst = window
        .get_mut(*pos..)
        .and_then(|w| w.get_mut(..len))
        .ok_or(Error::Corrupt)?;
    if *pos + len > end {
        return Err(Error::Corrupt);
    }
    dst.copy_from_slice(src);
    bits.pos += len;
    *pos += len;
    Ok(())
}

fn fixed_codes() -> Result<(Huffman<MAX_LITLEN>, Huffman<MAX_DIST>), Error> {
    let mut lengths = [0u8; MAX_LITLEN];
    for (i, l) in lengths.iter_mut().enumerate() {
        *l = match i {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    Ok((Huffman::new(&lengths)?, Huffman::new(&[5u8; MAX_DIST])?))
}

fn dynamic_codes(bits: &mut Bits<'_>) -> Result<(Huffman<MAX_LITLEN>, Huffman<MAX_DIST>), Error> {
    let hlit = bits.take(5)? as usize + 257;
    let hdist = bits.take(5)? as usize + 1;
    let hclen = bits.take(4)? as usize + 4;
    if hlit > 286 || hdist > 30 {
        return Err(Error::Corrupt);
    }
    let mut clen = [0u8; 19];
    for &i in &CLEN_ORDER[..hclen] {
        clen[i] = bits.take(3)? as u8;
    }
    let clen_code = Huffman::<19>::new(&clen)?;
    let mut lengths = [0u8; 286 + 30];
    let total = hlit + hdist;
    let mut i = 0;
    while i < total {
        let sym = clen_code.decode(bits)?;
        let (value, repeat) = match sym {
            0..=15 => (sym as u8, 1),
            16 => {
                let prev = *lengths
                    .get(i.wrapping_sub(1))
                    .filter(|_| i > 0)
                    .ok_or(Error::Corrupt)?;
                (prev, 3 + bits.take(2)? as usize)
            }
            17 => (0, 3 + bits.take(3)? as usize),
            18 => (0, 11 + bits.take(7)? as usize),
            _ => return Err(Error::Corrupt),
        };
        if i + repeat > total {
            return Err(Error::Corrupt);
        }
        lengths[i..i + repeat].fill(value);
        i += repeat;
    }
    if lengths[256] == 0 {
        // No end-of-block code: the block could never end.
        return Err(Error::Corrupt);
    }
    Ok((
        Huffman::new(&lengths[..hlit])?,
        Huffman::new(&lengths[hlit..total])?,
    ))
}

fn codes(
    bits: &mut Bits<'_>,
    window: &mut [u8],
    pos: &mut usize,
    end: usize,
    lit: &Huffman<MAX_LITLEN>,
    dist: &Huffman<MAX_DIST>,
) -> Result<(), Error> {
    loop {
        let sym = usize::from(lit.decode(bits)?);
        if sym < 256 {
            if *pos >= end {
                return Err(Error::Corrupt);
            }
            window[*pos] = sym as u8;
            *pos += 1;
        } else if sym == 256 {
            return Ok(());
        } else {
            let s = sym - 257;
            if s >= LEN_BASE.len() {
                return Err(Error::Corrupt);
            }
            let len = usize::from(LEN_BASE[s]) + bits.take(u32::from(LEN_EXTRA[s]))? as usize;
            let d = usize::from(dist.decode(bits)?);
            if d >= DIST_BASE.len() {
                return Err(Error::Corrupt);
            }
            let distance =
                usize::from(DIST_BASE[d]) + bits.take(u32::from(DIST_EXTRA[d]))? as usize;
            if distance > *pos || *pos + len > end {
                return Err(Error::Corrupt);
            }
            // Byte by byte: a match may overlap the bytes it produces.
            for k in *pos..*pos + len {
                window[k] = window[k - distance];
            }
            *pos += len;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_block() {
        // Final stored block "hi".
        let input = [0x01, 0x02, 0x00, 0xFD, 0xFF, b'h', b'i'];
        let mut w = [0u8; 8];
        assert_eq!(inflate(&input, &mut w, 0, 8), Ok(2));
        assert_eq!(&w[..2], b"hi");
    }

    #[test]
    fn stored_length_check() {
        let input = [0x01, 0x02, 0x00, 0xFC, 0xFF, b'h', b'i'];
        let mut w = [0u8; 8];
        assert_eq!(inflate(&input, &mut w, 0, 8), Err(Error::Corrupt));
    }

    #[test]
    fn fixed_block_with_a_match() {
        // zlib.compressobj(9, DEFLATED, -15) of b"abcabcabcabc": a literal
        // run then an overlapping match.
        let input = [0x4B, 0x4C, 0x4A, 0x4E, 0x84, 0x21, 0x00];
        let mut w = [0u8; 32];
        let n = inflate(&input, &mut w, 0, 32).unwrap();
        assert_eq!(&w[..n], b"abcabcabcabc");
    }

    #[test]
    fn a_match_into_the_history_works_and_before_it_fails() {
        // Same stream, but starting after 3 bytes of history: fine. A
        // stream whose first symbol is a match must fail with no history.
        let input = [0x4B, 0x4C, 0x4A, 0x4E, 0x84, 0x21, 0x00];
        let mut w = [0u8; 32];
        w[..3].copy_from_slice(b"xyz");
        let n = inflate(&input, &mut w, 3, 29).unwrap();
        assert_eq!(&w[3..3 + n], b"abcabcabcabc");
        // Fixed block: length 3 (symbol 257, code 0000001), distance 1
        // (code 00000): bits 1,01 then codes; no history before it.
        let bad = [0x03, 0x02, 0x00, 0x00];
        assert_eq!(inflate(&bad, &mut w, 0, 32), Err(Error::Corrupt));
    }

    #[test]
    fn output_limit_is_enforced() {
        let input = [0x4B, 0x4C, 0x4A, 0x4E, 0x84, 0x21, 0x00];
        let mut w = [0u8; 32];
        assert_eq!(inflate(&input, &mut w, 0, 11), Err(Error::Corrupt));
        // And nothing was written past the limit on the way to the error.
        assert!(w[11..].iter().all(|&b| b == 0), "{w:?}");
        // Literals only (zlib's raw deflate of "abcdefghijklmnop"): the
        // limit is met by a literal, not a match.
        let literals = [
            0x4B, 0x4C, 0x4A, 0x4E, 0x49, 0x4D, 0x4B, 0xCF, 0xC8, 0xCC, 0xCA, 0xCE, 0xC9, 0xCD,
            0xCB, 0x2F, 0x00, 0x00,
        ];
        let mut w = [0u8; 32];
        assert_eq!(inflate(&literals, &mut w, 0, 16), Ok(16));
        let mut w = [0u8; 32];
        assert_eq!(inflate(&literals, &mut w, 0, 5), Err(Error::Corrupt));
        assert_eq!(&w[..5], b"abcde");
        assert!(w[5..].iter().all(|&b| b == 0), "{w:?}");
    }

    #[test]
    fn over_subscribed_code_is_refused() {
        assert!(Huffman::<4>::new(&[1, 1, 1]).is_err());
        assert!(Huffman::<4>::new(&[1, 2, 2]).is_ok());
        // Incomplete (one code of length 1) is allowed.
        assert!(Huffman::<4>::new(&[1]).is_ok());
    }

    #[test]
    fn reserved_block_type_and_empty_input_fail() {
        let mut w = [0u8; 8];
        assert_eq!(inflate(&[0x07], &mut w, 0, 8), Err(Error::Corrupt));
        assert_eq!(inflate(&[], &mut w, 0, 8), Err(Error::Corrupt));
    }
}
