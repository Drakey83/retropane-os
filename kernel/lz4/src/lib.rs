//! LZ4 block compression (ADR-0005 rules 4 and 6: compressed storage and
//! compressed RAM).
//!
//! Clean-room from the public LZ4 block format specification. A block is a
//! series of *sequences*: a token byte (high nibble: literal count, low
//! nibble: match length − 4; 15 means "more length bytes follow, each added,
//! until one isn't 255"), the literals, a 2-byte little-endian match offset
//! (1-65535 bytes back), then the extra match length bytes. The last
//! sequence has literals only. The spec's end rules: the last 5 bytes are
//! literals, and the last match starts at least 12 bytes before the end.
//!
//! - [`decompress`] is *safe*: any input, however hostile, either decodes
//!   within `dst` or returns an error. Never a panic or an out-of-bounds
//!   write. Long non-overlapping copies go through `copy_within` (the HAL's
//!   ldm/stm `memmove`), overlapping ones by a doubling copy.
//! - [`compress`] is a fast greedy compressor sized for 4 KiB pages (zram) and
//!   storage blocks, with a fixed hash table on the caller's stack, no heap.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

/// Why a block couldn't be decoded or encoded.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub enum Error {
    /// The block ends in the middle of a sequence.
    Truncated,
    /// A match points before the start of the output.
    BadOffset,
    /// The output doesn't fit in `dst`.
    Overflow,
}

impl Error {
    /// A short name for logs.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Error::Truncated => "truncated",
            Error::BadOffset => "bad offset",
            Error::Overflow => "overflow",
        }
    }
}

/// Reads an extended length: `base` (the nibble) plus bytes while they're
/// 255.
#[inline(always)]
fn length(src: &[u8], at: &mut usize, base: usize) -> Result<usize, Error> {
    let mut n = base;
    if base == 15 {
        loop {
            let b = *src.get(*at).ok_or(Error::Truncated)?;
            *at += 1;
            n = n.checked_add(usize::from(b)).ok_or(Error::Overflow)?;
            if b != 255 {
                break;
            }
        }
    }
    Ok(n)
}

/// Decodes the block `src` into `dst`, returning the number of bytes
/// written.
pub fn decompress(src: &[u8], dst: &mut [u8]) -> Result<usize, Error> {
    let (mut i, mut o) = (0usize, 0usize);
    loop {
        let token = *src.get(i).ok_or(Error::Truncated)?;
        // Fast path for the common sequence: both lengths fit their nibble
        // and both buffers have room for the largest such sequence (14
        // literals + offset; 18-byte match), so one check covers every
        // access below.
        let (lit_n, ml_n) = (usize::from(token >> 4), usize::from(token & 0xF));
        if lit_n < 15 && ml_n < 15 && i + 1 + 14 + 2 < src.len() && o + 14 + 18 <= dst.len() {
            let at = i + 1;
            // Slice once, then copy without per-byte bounds checks.
            for (d, s) in dst[o..o + lit_n].iter_mut().zip(&src[at..at + lit_n]) {
                *d = *s;
            }
            let (i2, o2) = (at + lit_n, o + lit_n);
            let offset = usize::from(src[i2]) | (usize::from(src[i2 + 1]) << 8);
            if offset == 0 || offset > o2 {
                return Err(Error::BadOffset);
            }
            let len = ml_n + 4;
            copy_short(&mut dst[o2 - offset..o2 + len], offset);
            i = i2 + 2;
            o = o2 + len;
            continue;
        }
        i += 1;
        // Literals.
        let lits = length(src, &mut i, usize::from(token >> 4))?;
        let lit_end = i.checked_add(lits).ok_or(Error::Truncated)?;
        let out_end = o.checked_add(lits).ok_or(Error::Overflow)?;
        let from = src.get(i..lit_end).ok_or(Error::Truncated)?;
        let to = dst.get_mut(o..out_end).ok_or(Error::Overflow)?;
        if lits <= SHORT {
            // Short runs dominate real data: a call to memcpy costs more
            // than the copy (zip: no per-byte bounds checks).
            for (d, s) in to.iter_mut().zip(from) {
                *d = *s;
            }
        } else {
            to.copy_from_slice(from);
        }
        i = lit_end;
        o = out_end;
        if i == src.len() {
            return Ok(o); // the last sequence has no match
        }
        // Match.
        let off = src.get(i..i + 2).ok_or(Error::Truncated)?;
        let offset = usize::from(u16::from_le_bytes([off[0], off[1]]));
        i += 2;
        if offset == 0 || offset > o {
            return Err(Error::BadOffset);
        }
        let len = length(src, &mut i, usize::from(token & 0xF))?
            .checked_add(4)
            .ok_or(Error::Overflow)?;
        let end = o.checked_add(len).ok_or(Error::Overflow)?;
        if end > dst.len() {
            return Err(Error::Overflow);
        }
        copy_match(dst, o, offset, len);
        o = end;
    }
}

/// Copies `len` bytes from `offset` back to position `o` (inside `dst`,
/// checked by the caller), where source and destination may overlap.
/// Copies the tail of `win` (everything after its first `offset` bytes) from
/// its head, forward, as LZ4 defines it. Without overlap the two halves are
/// split and zipped (no per-byte bounds checks); with overlap, each byte may
/// depend on one written just before, so it's copied in order.
#[inline(always)]
fn copy_short(win: &mut [u8], offset: usize) {
    let len = win.len() - offset;
    if offset >= len {
        let (head, tail) = win.split_at_mut(offset);
        for (d, s) in tail.iter_mut().zip(&head[..len]) {
            *d = *s;
        }
    } else {
        for k in 0..len {
            win[offset + k] = win[k];
        }
    }
}

/// Copies at or below this length are done inline: below it, a call to
/// `memcpy`/`memmove` costs more than the bytes.
const SHORT: usize = 16;

#[inline(always)]
fn copy_match(dst: &mut [u8], o: usize, offset: usize, len: usize) {
    let start = o - offset;
    if len <= SHORT {
        copy_short(&mut dst[start..o + len], offset);
    } else if offset >= len {
        // No overlap: one bulk copy (memmove's ldm/stm loops).
        dst.copy_within(start..start + len, o);
    } else if offset == 1 {
        // A run of one byte: a fill.
        let b = dst[start];
        dst[o..o + len].fill(b);
    } else {
        // Overlap: the pattern of `offset` bytes repeats. Copy it, then copy
        // what's been written so far, doubling each time.
        let mut done = 0;
        let mut chunk = offset;
        while done < len {
            let n = chunk.min(len - done);
            dst.copy_within(start..start + n, o + done);
            done += n;
            chunk = o + done - start; // everything from `start` is the pattern
        }
    }
}

/// Bytes the hash table indexes (4 KiB entries).
const HASH_BITS: u32 = 12;
/// The minimum match.
const MIN_MATCH: usize = 4;
/// The spec's end rules.
const LAST_LITERALS: usize = 5;
const MF_LIMIT: usize = 12;

/// Worst-case compressed size of `n` bytes (incompressible input grows by
/// one length byte per 255 literals, plus the token).
#[must_use]
pub const fn max_compressed(n: usize) -> usize {
    n + n / 255 + 16
}

fn hash(v: u32) -> usize {
    (v.wrapping_mul(2_654_435_761) >> (32 - HASH_BITS)) as usize
}

fn read32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn put_length(dst: &mut [u8], o: &mut usize, mut n: usize) -> Result<(), Error> {
    while n >= 255 {
        *dst.get_mut(*o).ok_or(Error::Overflow)? = 255;
        *o += 1;
        n -= 255;
    }
    *dst.get_mut(*o).ok_or(Error::Overflow)? = n as u8;
    *o += 1;
    Ok(())
}

fn put_sequence(
    dst: &mut [u8],
    o: &mut usize,
    literals: &[u8],
    matched: Option<(u16, usize)>,
) -> Result<(), Error> {
    let lit = literals.len();
    let ml = matched.map_or(0, |(_, l)| l - MIN_MATCH);
    let token = ((lit.min(15) as u8) << 4) | ml.min(15) as u8;
    // Fast path: short literals and match, room for the largest such
    // sequence (token, 14 literals, offset): one check, no per-field ones.
    if lit < 15 && ml < 15 && *o + 1 + 14 + 2 <= dst.len() {
        let out = &mut dst[*o..*o + 1 + lit + 2];
        out[0] = token;
        for (d, s) in out[1..=lit].iter_mut().zip(literals) {
            *d = *s;
        }
        let n = if let Some((offset, _)) = matched {
            out[1 + lit..].copy_from_slice(&offset.to_le_bytes());
            1 + lit + 2
        } else {
            1 + lit
        };
        *o += n;
        return Ok(());
    }
    *dst.get_mut(*o).ok_or(Error::Overflow)? = token;
    *o += 1;
    if lit >= 15 {
        put_length(dst, o, lit - 15)?;
    }
    dst.get_mut(*o..*o + lit)
        .ok_or(Error::Overflow)?
        .copy_from_slice(literals);
    *o += lit;
    if let Some((offset, _)) = matched {
        dst.get_mut(*o..*o + 2)
            .ok_or(Error::Overflow)?
            .copy_from_slice(&offset.to_le_bytes());
        *o += 2;
        if ml >= 15 {
            put_length(dst, o, ml - 15)?;
        }
    }
    Ok(())
}

/// Compresses `src` into `dst` (at least [`max_compressed`]`(src.len())`
/// bytes guarantees success), returning the compressed size. Inputs up to
/// 64 KiB per block (offsets are 16-bit and positions are stored as `u16`).
pub fn compress(src: &[u8], dst: &mut [u8]) -> Result<usize, Error> {
    if src.len() > 0x1_0000 {
        return Err(Error::Overflow);
    }
    let mut table = [0u16; 1 << HASH_BITS];
    let mut o = 0;
    let mut anchor = 0; // start of pending literals
    if src.len() >= MF_LIMIT {
        let limit = src.len() - MF_LIMIT;
        // Position 0 is never a match source (the table's zero entries
        // would point at it), so start one in.
        let mut i = 1;
        table[hash(read32(src, 0))] = 0;
        while i <= limit {
            let v = read32(src, i);
            let h = hash(v);
            let cand = usize::from(table[h]);
            table[h] = i as u16;
            if cand < i && i - cand <= 0xFFFF && read32(src, cand) == v {
                // Extend the match, stopping LAST_LITERALS from the end.
                let max = src.len() - LAST_LITERALS;
                // Zipped slices: no per-byte bounds checks in the loop.
                let len = MIN_MATCH
                    + src[cand + MIN_MATCH..]
                        .iter()
                        .zip(&src[i + MIN_MATCH..max])
                        .take_while(|(a, b)| a == b)
                        .count();
                put_sequence(dst, &mut o, &src[anchor..i], Some(((i - cand) as u16, len)))?;
                i += len;
                anchor = i;
                // Index a position inside the match so repeats are found.
                if i <= limit {
                    table[hash(read32(src, i - 2))] = (i - 2) as u16;
                }
            } else {
                i += 1;
            }
        }
    }
    put_sequence(dst, &mut o, &src[anchor..], None)?;
    Ok(o)
}

#[cfg(test)]
mod tests;
