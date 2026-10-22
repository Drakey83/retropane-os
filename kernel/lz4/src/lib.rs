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
/// Misses in a row before the search step grows by one byte (LZ4's usual
/// value: 64).
const SKIP_SHIFT: usize = 6;
/// The spec's end rules.
const LAST_LITERALS: usize = 5;
const MF_LIMIT: usize = 12;

/// Worst-case compressed size of `n` bytes (incompressible input grows by
/// one length byte per 255 literals, plus the token).
#[must_use]
pub const fn max_compressed(n: usize) -> usize {
    n + n / 255 + 16
}

/// The hash's multiplier: LZ4's own (Knuth's 2^32 / golden ratio,
/// `0x9E3779B1`) shifted down 5 bits, so it fits in 27. The XScale ends a
/// multiply early by its multiplier's size: 32 bits costs 2 cycles more
/// than 15, 27 bits 1 more (docs/hw/cpu.md, measured on the X51v), and
/// LLVM puts the hash's use right after it, so the wait is paid every
/// position. Measured 2026-10-22 on the X51v (`rp-probe`, same build
/// method each): compress 221.4 -> 204.2 us a page (-7.8%); the same
/// compression on a corpus of our images and texts (69.39% vs 69.41%,
/// 81.49% vs 81.45%; `multipliers_compress`), where 16 or 15 bits lose
/// 0.7 points. `RP_LZ4_HASH_K` (decimal) at build time overrides it, for
/// a sweep.
pub const HASH_K: u32 = match option_env!("RP_LZ4_HASH_K") {
    Some(s) => parse_u32(s),
    None => 0x04F1_BBCD,
};

/// A decimal `u32` at compile time (0 for anything else).
const fn parse_u32(s: &str) -> u32 {
    let b = s.as_bytes();
    let mut i = 0;
    let mut v = 0u32;
    while i < b.len() {
        if !b[i].is_ascii_digit() {
            return 0;
        }
        v = v.wrapping_mul(10).wrapping_add((b[i] - b'0') as u32);
        i += 1;
    }
    v
}

#[inline(always)]
fn hash<const K: u32>(v: u32) -> usize {
    (v.wrapping_mul(K) >> (32 - HASH_BITS)) as usize
}

/// The little-endian word at `at` (one bounds check, not four).
#[inline(always)]
fn read32(b: &[u8], at: usize) -> u32 {
    match b.get(at..at + 4) {
        Some(&[x, y, z, w]) => u32::from_le_bytes([x, y, z, w]),
        _ => panic!("lz4: read past the block"),
    }
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

/// One sequence: the fast path (short literals and match, room for it)
/// inline in the compressor's loop, the rest out of line (the performance
/// gate's C9: a build with more inlining compressed 26-33% faster on the
/// X51v, 2026-10-22).
#[inline(always)]
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
    put_sequence_long(dst, o, literals, matched, token, ml)
}

/// [`put_sequence`]'s general case: long literals or match, or near the
/// end of `dst`.
#[cold]
#[inline(never)]
fn put_sequence_long(
    dst: &mut [u8],
    o: &mut usize,
    literals: &[u8],
    matched: Option<(u16, usize)>,
    token: u8,
    ml: usize,
) -> Result<(), Error> {
    let lit = literals.len();
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
    // Zeroed as made: the loop doesn't clear it again.
    compress_k::<HASH_K>(&mut [0; TABLE_LEN], src, dst)
}

/// Entries in the compressor's hash table.
pub const TABLE_LEN: usize = 1 << HASH_BITS;

/// [`compress`] with a caller-provided hash table (8 KiB) that is **not
/// cleared**: the kernel keeps one table for every page it compresses
/// (kernel stacks are small, and clearing 8 KiB a page cost ~4,600 cycles on
/// the X51v whenever the table wasn't in the D-cache, docs/hw/cpu.md).
///
/// The table may hold anything: positions from earlier calls, or values
/// never written by this crate. Each entry is only a hint: a candidate is
/// taken only if it lies before the current position (so inside `src`) and
/// its four bytes equal the current four, so a stale entry either finds a
/// real match or is passed over. The output is always a valid block that
/// [`decompress`] restores exactly, but it depends on what the table held:
/// the same `src` can compress to different (equally valid) bytes after
/// different earlier calls. [`compress`] (a fresh, zeroed table each call)
/// is the deterministic one.
pub fn compress_with(
    table: &mut [u16; TABLE_LEN],
    src: &[u8],
    dst: &mut [u8],
) -> Result<usize, Error> {
    compress_k::<HASH_K>(table, src, dst)
}

/// [`compress_with`] with the hash's multiplier `K` (the tests compare
/// multipliers' compression on the same data). `table` holds anything (see
/// [`compress_with`]): every candidate it gives is checked before use.
///
/// Never inlined: a caller's 8 KiB table on the stack ([`compress`]) must
/// stay out of this loop's frame. An ARM load reaches 4,095 bytes from its
/// base, so with the table in the same frame every local beyond it took two
/// adds and a stall to reach from `sp`: text compress 6.2% slower with frame
/// pointers off, where `r11` had reached them (2026-10-22, the X51v's event
/// counters: +4.8% instructions, +34,626 data-stall cycles per 16 pages;
/// ADR-0001 "Why, and what was fixed").
#[inline(never)]
fn compress_k<const K: u32>(
    table: &mut [u16; TABLE_LEN],
    src: &[u8],
    dst: &mut [u8],
) -> Result<usize, Error> {
    if src.len() > 0x1_0000 {
        return Err(Error::Overflow);
    }
    let mut o = 0;
    let mut anchor = 0; // start of pending literals
    if src.len() >= MF_LIMIT {
        let limit = src.len() - MF_LIMIT;
        // Position 0 is never a match source (a zeroed table's entries
        // would point at it), so start one in; a candidate is taken only
        // before `i`, with its four bytes equal (the table is only hints).
        let mut i = 1;
        // Skip acceleration: after every 2^SKIP_SHIFT misses in a row, step
        // one byte further, so incompressible data (already-compressed
        // files, media) is skimmed instead of searched byte by byte. Any
        // match resets the step.
        let mut misses = 0usize;
        table[hash::<K>(read32(src, 0))] = 0;
        while i <= limit {
            let v = read32(src, i);
            let h = hash::<K>(v);
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
                misses = 0;
                i += len;
                anchor = i;
                // Index a position inside the match so repeats are found.
                if i <= limit {
                    table[hash::<K>(read32(src, i - 2))] = (i - 2) as u16;
                }
            } else {
                misses += 1;
                i += 1 + (misses >> SKIP_SHIFT);
            }
        }
    }
    put_sequence(dst, &mut o, &src[anchor..], None)?;
    Ok(o)
}

#[cfg(test)]
mod tests;
