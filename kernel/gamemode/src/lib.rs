//! Game Mode's return record (docs/design/game-mode-b.md, build step G3 in
//! docs/design/game-mode-build.md): everything the kernel needs to bring
//! Full Mode back after a game (the frozen processes, their threads, the
//! suspended services' records, the full tier's page map), kept in a
//! region reserved at boot outside every process slot.
//!
//! - **Bounded:** the reserve is [`RESERVE`] (256 KiB, the owner's ceiling,
//!   2026-11-23), split into two halves; a record never reaches past its
//!   half, and every length read back is checked against the half before
//!   it is used, so a damaged record is refused, never followed.
//! - **Checked:** the header carries a hash of itself and one of the body
//!   (XXH32, the body's seeded with the record's sequence number, so a
//!   stale body under a newer header fails). A crashed game can't reach the
//!   reserve (it is kernel memory); the hashes catch what can: a stray
//!   kernel write, a bit flipped across a watchdog reset with RAM kept.
//! - **A/B:** a new record goes into the half that doesn't hold the newest
//!   good one, its header made invalid first and written last, so a reset at
//!   any byte of a write leaves the previous record the newest good one.
//! - **Versioned:** [`VERSION`] in the header; any other is refused, as is
//!   any section kind this kernel doesn't know.
//!
//! Rule 0: consoles keep the system's suspended state in a reserved
//! partition of megabytes; Android saves each app's state into a `Bundle`
//! of its own (often lost when the process dies); CE has no such record.
//! Ours is a quarter megabyte of RAM, only the record (the processes stay
//! where they are, frozen), with the A/B commit and two hashes so a crash
//! at any moment leaves a record that is either whole or refused. XXH32
//! instead of the textbook CRC-32: no 1 KiB table in a 64 MiB machine, and
//! word-at-a-time with the PXA270's multiplier rather than a table lookup a
//! byte; the record is checked for accidental damage, not attack.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

/// A game's DMA chains and cache calls, checked and translated (G7a).
pub mod dma;
pub mod tier;

/// The reserve's size: 256 KiB (the owner's ceiling, design B).
pub const RESERVE: usize = 256 * 1024;
/// One half: the most one record may take.
pub const HALF: usize = RESERVE / 2;
/// The header's size.
pub const HEADER: usize = 32;
/// A section's own header: kind, flags, length.
pub const SECTION_HEADER: usize = 8;
/// The record format this kernel writes and reads.
pub const VERSION: u16 = 1;
const MAGIC: [u8; 4] = *b"RPGM";

/// What a section holds.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
#[repr(u16)]
pub enum Kind {
    /// A frozen process.
    Process = 1,
    /// A frozen thread's saved state.
    Thread = 2,
    /// A suspended service's own record.
    Service = 3,
    /// The full tier's page map (which page went where on the card).
    TierMap = 4,
}

impl Kind {
    fn from_u16(v: u16) -> Option<Self> {
        Some(match v {
            1 => Self::Process,
            2 => Self::Thread,
            3 => Self::Service,
            4 => Self::TierMap,
            _ => return None,
        })
    }
}

/// Why a record was refused, or couldn't be written.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
pub enum Error {
    /// The buffer is smaller than a header (or than its half).
    TooSmall,
    /// No record here (the magic doesn't match).
    Empty,
    /// A record of another format version.
    Version,
    /// The header's own hash doesn't match.
    HeaderHash,
    /// The body's length reaches past the half.
    Length,
    /// The body's hash doesn't match.
    BodyHash,
    /// A section's length runs past the body, or its padding or flags
    /// aren't zero, or the section count is wrong.
    Section,
    /// A section of a kind this kernel doesn't know.
    Kind,
    /// Writing: the record doesn't fit its half.
    Full,
}

// ---- XXH32 --------------------------------------------------------------

const P1: u32 = 0x9E37_79B1;
const P2: u32 = 0x85EB_CA77;
const P3: u32 = 0xC2B2_AE3D;
const P4: u32 = 0x27D4_EB2F;
const P5: u32 = 0x1656_67B1;

fn word(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

fn round(acc: u32, w: u32) -> u32 {
    acc.wrapping_add(w.wrapping_mul(P2))
        .rotate_left(13)
        .wrapping_mul(P1)
}

/// XXH32 of `data` with `seed` (Yann Collet's xxHash, 32-bit; checked
/// against its published vectors in the tests).
#[must_use]
#[inline(never)]
pub fn xxh32(data: &[u8], seed: u32) -> u32 {
    let mut rest = data;
    let mut h = if data.len() >= 16 {
        let mut v = [
            seed.wrapping_add(P1).wrapping_add(P2),
            seed.wrapping_add(P2),
            seed,
            seed.wrapping_sub(P1),
        ];
        let (blocks, tail) = data.split_at(data.len() / 16 * 16);
        for b in blocks.as_chunks::<16>().0 {
            for (k, acc) in v.iter_mut().enumerate() {
                *acc = round(*acc, word(&b[k * 4..]));
            }
        }
        rest = tail;
        v[0].rotate_left(1)
            .wrapping_add(v[1].rotate_left(7))
            .wrapping_add(v[2].rotate_left(12))
            .wrapping_add(v[3].rotate_left(18))
    } else {
        seed.wrapping_add(P5)
    };
    h = h.wrapping_add(data.len() as u32);
    while rest.len() >= 4 {
        h = h
            .wrapping_add(word(rest).wrapping_mul(P3))
            .rotate_left(17)
            .wrapping_mul(P4);
        rest = &rest[4..];
    }
    for &b in rest {
        h = h
            .wrapping_add(u32::from(b).wrapping_mul(P5))
            .rotate_left(11)
            .wrapping_mul(P1);
    }
    h ^= h >> 15;
    h = h.wrapping_mul(P2);
    h ^= h >> 13;
    h = h.wrapping_mul(P3);
    h ^ (h >> 16)
}

// ---- writing ------------------------------------------------------------

/// Builds one record in a buffer (one half of the reserve, or any buffer
/// for a test): sections appended, then [`finish`](Self::finish) writes the
/// header last.
pub struct Writer<'a> {
    buf: &'a mut [u8],
    at: usize,
    sections: u16,
}

impl<'a> Writer<'a> {
    /// Starts a record in `buf`, invalidating whatever record was there
    /// first (its magic cleared), so a reset before
    /// [`finish`](Self::finish) leaves no record here, never half of one.
    pub fn new(buf: &'a mut [u8]) -> Result<Self, Error> {
        if buf.len() < HEADER {
            return Err(Error::TooSmall);
        }
        buf[..4].fill(0);
        Ok(Self {
            buf,
            at: HEADER,
            sections: 0,
        })
    }

    /// Appends a section of `kind` holding `payload`.
    pub fn section(&mut self, kind: Kind, payload: &[u8]) -> Result<(), Error> {
        self.open(kind, payload.len())?.copy_from_slice(payload);
        Ok(())
    }

    /// Appends a section of `kind` and `len` bytes, filled in place by
    /// `fill` (so a large section needn't be built elsewhere first).
    pub fn section_with(
        &mut self,
        kind: Kind,
        len: usize,
        fill: impl FnOnce(&mut [u8]),
    ) -> Result<(), Error> {
        fill(self.open(kind, len)?);
        Ok(())
    }

    /// Appends a section's header and padding; its payload's bytes, to
    /// fill. One copy of this in the kernel, however many callers.
    #[inline(never)]
    fn open(&mut self, kind: Kind, len: usize) -> Result<&mut [u8], Error> {
        let padded = len.checked_add(3).ok_or(Error::Full)? & !3;
        let end = self
            .at
            .checked_add(SECTION_HEADER)
            .and_then(|a| a.checked_add(padded))
            .ok_or(Error::Full)?;
        if end > self.buf.len() || self.sections == u16::MAX {
            return Err(Error::Full);
        }
        let len32 = u32::try_from(len).map_err(|_| Error::Full)?;
        let h = &mut self.buf[self.at..self.at + SECTION_HEADER];
        h[..2].copy_from_slice(&(kind as u16).to_le_bytes());
        h[2..4].fill(0);
        h[4..8].copy_from_slice(&len32.to_le_bytes());
        let body = self.at + SECTION_HEADER;
        self.buf[body + len..body + padded].fill(0);
        self.at = end;
        self.sections += 1;
        Ok(&mut self.buf[body..body + len])
    }

    /// Writes the header (last) with sequence number `seq`; the record's
    /// length.
    #[inline(never)]
    pub fn finish(self, seq: u32) -> usize {
        let body_len = (self.at - HEADER) as u32;
        let body_hash = xxh32(&self.buf[HEADER..self.at], seq);
        let h = &mut self.buf[..HEADER];
        h[4..6].copy_from_slice(&VERSION.to_le_bytes());
        h[6..8].copy_from_slice(&self.sections.to_le_bytes());
        h[8..12].copy_from_slice(&body_len.to_le_bytes());
        h[12..16].copy_from_slice(&seq.to_le_bytes());
        h[16..20].copy_from_slice(&body_hash.to_le_bytes());
        h[20..28].fill(0);
        let header_hash = xxh32(&h[4..28], 0);
        h[28..32].copy_from_slice(&header_hash.to_le_bytes());
        // The magic last: until it is there, this is no record.
        h[..4].copy_from_slice(&MAGIC);
        self.at
    }
}

// ---- reading ------------------------------------------------------------

/// A record that passed every check.
#[derive(Clone, Copy)]
pub struct Record<'a> {
    seq: u32,
    count: u16,
    body: &'a [u8],
}

impl<'a> Record<'a> {
    /// Checks the record at the start of `buf` (one half of the reserve):
    /// magic, version, the header's hash, the body's length against `buf`,
    /// the body's hash, then every section's kind, flags, length and
    /// padding. Nothing is trusted before it is checked.
    #[inline(never)]
    pub fn parse(buf: &'a [u8]) -> Result<Self, Error> {
        if buf.len() < HEADER {
            return Err(Error::TooSmall);
        }
        let h = &buf[..HEADER];
        if h[..4] != MAGIC {
            return Err(Error::Empty);
        }
        if u16::from_le_bytes([h[4], h[5]]) != VERSION {
            return Err(Error::Version);
        }
        if xxh32(&h[4..28], 0) != word(&h[28..]) || h[20..28].iter().any(|&b| b != 0) {
            return Err(Error::HeaderHash);
        }
        let count = u16::from_le_bytes([h[6], h[7]]);
        let body_len = word(&h[8..]) as usize;
        let seq = word(&h[12..]);
        if body_len > buf.len() - HEADER || !body_len.is_multiple_of(4) {
            return Err(Error::Length);
        }
        let body = &buf[HEADER..HEADER + body_len];
        if xxh32(body, seq) != word(&h[16..]) {
            return Err(Error::BodyHash);
        }
        let r = Self { seq, count, body };
        let mut n = 0u32;
        let mut at = 0;
        while at < body.len() {
            r.section_at(at)?;
            at = r.next(at);
            n += 1;
        }
        if n != u32::from(count) {
            return Err(Error::Section);
        }
        Ok(r)
    }

    /// The section at body offset `at`, checked.
    fn section_at(&self, at: usize) -> Result<(Kind, &'a [u8]), Error> {
        let b = self.body;
        if b.len() - at < SECTION_HEADER {
            return Err(Error::Section);
        }
        let kind = u16::from_le_bytes([b[at], b[at + 1]]);
        let flags = u16::from_le_bytes([b[at + 2], b[at + 3]]);
        let len = word(&b[at + 4..]) as usize;
        let room = b.len() - at - SECTION_HEADER;
        if flags != 0 || len > room || (len + 3) & !3 > room {
            return Err(Error::Section);
        }
        let start = at + SECTION_HEADER;
        if b[start + len..start + ((len + 3) & !3)]
            .iter()
            .any(|&x| x != 0)
        {
            return Err(Error::Section);
        }
        let kind = Kind::from_u16(kind).ok_or(Error::Kind)?;
        Ok((kind, &b[start..start + len]))
    }

    fn next(&self, at: usize) -> usize {
        let len = word(&self.body[at + 4..]) as usize;
        at + SECTION_HEADER + ((len + 3) & !3)
    }

    /// The record's sequence number.
    #[must_use]
    pub fn seq(&self) -> u32 {
        self.seq
    }

    /// How many sections it holds.
    #[must_use]
    pub fn len(&self) -> usize {
        usize::from(self.count)
    }

    /// Whether it holds none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Its sections in order (each already checked by `parse`).
    pub fn sections(&self) -> impl Iterator<Item = (Kind, &'a [u8])> + '_ {
        let mut at = 0;
        core::iter::from_fn(move || {
            if at >= self.body.len() {
                return None;
            }
            let s = self.section_at(at).ok();
            at = self.next(at);
            s
        })
    }
}

// ---- the reserve, A/B ---------------------------------------------------

/// Is sequence number `a` newer than `b` (wrapping)?
fn newer(a: u32, b: u32) -> bool {
    a != b && a.wrapping_sub(b) < u32::MAX / 2
}

/// The newest good record in `reserve` (both halves checked), and which
/// half holds it.
#[must_use]
#[inline(never)]
pub fn newest(reserve: &[u8]) -> Option<(usize, Record<'_>)> {
    let half = reserve.len() / 2;
    let a = Record::parse(&reserve[..half]).ok();
    let b = Record::parse(&reserve[half..half * 2]).ok();
    match (a, b) {
        (Some(a), Some(b)) => Some(if newer(b.seq, a.seq) { (1, b) } else { (0, a) }),
        (Some(a), None) => Some((0, a)),
        (None, Some(b)) => Some((1, b)),
        (None, None) => None,
    }
}

/// Writes a new record into `reserve` with `build` adding its sections:
/// into the half not holding the newest good record, numbered one past it.
/// If `build` fails, that half is left holding no record and the old one
/// stays the newest. Returns the new record's sequence number and length.
///
/// `build` is a trait object, not a generic: one copy of this in the
/// kernel, however many callers (a generic one cost ~2 KB a caller).
#[inline(never)]
pub fn commit(
    reserve: &mut [u8],
    build: &mut dyn FnMut(&mut Writer<'_>) -> Result<(), Error>,
) -> Result<(u32, usize), Error> {
    let half = reserve.len() / 2;
    if half < HEADER {
        return Err(Error::TooSmall);
    }
    let (target, seq) = match newest(reserve) {
        Some((h, r)) => (1 - h, r.seq.wrapping_add(1)),
        None => (0, 1),
    };
    let buf = &mut reserve[target * half..(target + 1) * half];
    let mut w = Writer::new(buf)?;
    build(&mut w)?;
    Ok((seq, w.finish(seq)))
}

#[cfg(test)]
mod tests;
