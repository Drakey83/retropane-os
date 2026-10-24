//! Test G7's frame file: one frame of Dell's driver, taken out of the
//! 2700G's local memory after his divide-1 draws (`rp-wmprobe` v4's
//! capture) by `cargo xtask g7-frame`, for the image to replay word for
//! word. Shared by the image (`#[path]`-free: it's `src/frame.rs`) and
//! xtask (by `#[path]`), so the two sides read one format.
//!
//! Little-endian 32-bit words: [`MAGIC`], [`VERSION`]; the register count
//! and `(offset, value)` pairs (MBX block offsets, written before the
//! render, the start excepted); the segment count and each segment's
//! address in local memory, its word count and its words; then the hash of
//! Dell's own picture (FNV-1a over the target's words, as the image hashes)
//! and its word count. The image refuses any file that breaks these rules
//! (`parse`), never draws half of one.

/// "RPG7".
pub const MAGIC: u32 = 0x3747_5052;
/// This layout.
pub const VERSION: u32 = 1;
/// The 2700G's local memory.
pub const LOCAL: usize = 16 << 20;
/// The registers a frame never carries: the reset, the status and its
/// clear, the 3D stage's start (the image writes it last) and the TA's.
pub const NEVER: [usize; 5] = [0x080, 0x12C, 0x134, 0x680, 0x800];

/// A parsed frame, borrowing the file.
#[derive(Clone, Copy, Debug)]
pub struct Frame<'a> {
    regs: &'a [u8],
    segs: &'a [u8],
    nsegs: usize,
    /// Dell's picture's hash, and its words (0 if the file has none).
    pub picture_hash: u32,
    /// See [`Frame::picture_hash`].
    pub picture_words: u32,
}

fn word(b: &[u8], i: usize) -> Option<u32> {
    let s = b.get(4 * i..4 * i + 4)?;
    Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// The file checked and parsed.
///
/// # Errors
/// What is wrong with it.
pub fn parse(b: &[u8]) -> Result<Frame<'_>, &'static str> {
    if b.len() % 4 != 0 {
        return Err("not whole words");
    }
    if word(b, 0) != Some(MAGIC) {
        return Err("not a frame file");
    }
    if word(b, 1) != Some(VERSION) {
        return Err("another version");
    }
    let nregs = word(b, 2).ok_or("short")? as usize;
    if nregs > 1024 {
        return Err("too many registers");
    }
    let regs = b.get(12..12 + 8 * nregs).ok_or("short")?;
    for k in 0..nregs {
        let off = word(regs, 2 * k).ok_or("short")? as usize;
        if off >= 0x1000 || off % 4 != 0 || NEVER.contains(&off) {
            return Err("a register outside the MBX block, or one never written");
        }
    }
    let mut at = 3 + 2 * nregs;
    let nsegs = word(b, at).ok_or("short")? as usize;
    at += 1;
    let segs_from = 4 * at;
    for _ in 0..nsegs {
        let addr = word(b, at).ok_or("short")? as usize;
        let words = word(b, at + 1).ok_or("short")? as usize;
        if addr % 4 != 0 || words == 0 || addr + 4 * words > LOCAL {
            return Err("a segment outside local memory");
        }
        at += 2 + words;
        if 4 * at > b.len() {
            return Err("short");
        }
    }
    let segs = &b[segs_from..4 * at];
    let picture_hash = word(b, at).ok_or("short")?;
    let picture_words = word(b, at + 1).ok_or("short")?;
    if 4 * (at + 2) != b.len() {
        return Err("bytes after the end");
    }
    Ok(Frame {
        regs,
        segs,
        nsegs,
        picture_hash,
        picture_words,
    })
}

impl<'a> Frame<'a> {
    /// The registers, in file order.
    pub fn regs(&self) -> impl Iterator<Item = (usize, u32)> + 'a {
        let r = self.regs;
        (0..r.len() / 8).map(move |k| {
            (
                word(r, 2 * k).unwrap_or(0) as usize,
                word(r, 2 * k + 1).unwrap_or(0),
            )
        })
    }

    /// One register's value, if the frame has it.
    #[must_use]
    pub fn reg(&self, off: usize) -> Option<u32> {
        self.regs().find(|p| p.0 == off).map(|p| p.1)
    }

    /// The segments: address and words (as little-endian bytes).
    pub fn segs(&self) -> impl Iterator<Item = (usize, &'a [u8])> + 'a {
        let s = self.segs;
        let mut at = 0usize;
        (0..self.nsegs).map(move |_| {
            let addr = word(s, at).unwrap_or(0) as usize;
            let words = word(s, at + 1).unwrap_or(0) as usize;
            let data = &s[4 * (at + 2)..4 * (at + 2 + words)];
            at += 2 + words;
            (addr, data)
        })
    }

    /// Whether any segment touches `[from, from + bytes)`.
    #[must_use]
    pub fn touches(&self, from: usize, bytes: usize) -> bool {
        self.segs()
            .any(|(a, d)| a < from + bytes && from < a + d.len())
    }
}

/// The file for `regs`, `segs` and Dell's picture (host side: xtask and the
/// tests).
#[cfg(not(target_os = "none"))]
#[must_use]
pub fn write(regs: &[(usize, u32)], segs: &[(usize, Vec<u32>)], picture: (u32, u32)) -> Vec<u8> {
    let mut w: Vec<u32> = vec![MAGIC, VERSION, regs.len() as u32];
    for &(off, v) in regs {
        w.push(off as u32);
        w.push(v);
    }
    w.push(segs.len() as u32);
    for (addr, data) in segs {
        w.push(*addr as u32);
        w.push(data.len() as u32);
        w.extend_from_slice(data);
    }
    w.push(picture.0);
    w.push(picture.1);
    w.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// FNV-1a over words, as the image hashes a picture.
#[must_use]
pub fn hash(words: impl IntoIterator<Item = u32>) -> u32 {
    words
        .into_iter()
        .fold(0x811C_9DC5u32, |h, v| (h ^ v).wrapping_mul(0x0100_0193))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<u8> {
        write(
            &[(0x608, 0x00F3_5000), (0x650, 1)],
            &[(0x00F3_5000, vec![1, 2, 3]), (0x0022_B000, vec![9])],
            (0xABCD_0123, 153_600),
        )
    }

    #[test]
    fn a_written_frame_reads_back_the_same() {
        let b = sample();
        let f = parse(&b).expect("parses");
        assert_eq!(
            f.regs().collect::<Vec<_>>(),
            [(0x608, 0x00F3_5000), (0x650, 1)]
        );
        assert_eq!(f.reg(0x650), Some(1));
        assert_eq!(f.reg(0x654), None);
        let segs: Vec<_> = f.segs().map(|(a, d)| (a, d.len())).collect();
        assert_eq!(segs, [(0x00F3_5000, 12), (0x0022_B000, 4)]);
        assert_eq!(f.segs().nth(1).map(|s| s.1), Some(&9u32.to_le_bytes()[..]));
        assert_eq!((f.picture_hash, f.picture_words), (0xABCD_0123, 153_600));
        assert!(f.touches(0x00F3_5008, 4));
        assert!(!f.touches(0x00F3_500C, 4), "just past the first segment");
        assert!(!f.touches(0x00F3_4FFC, 4), "just before it");
    }

    #[test]
    fn every_broken_file_is_refused() {
        let good = sample();
        let mut cases: Vec<(Vec<u8>, &str)> = vec![
            (good[..good.len() - 1].to_vec(), "not whole words"),
            (good[..good.len() - 4].to_vec(), "short"),
            ([&good[..], &[0, 0, 0, 0]].concat(), "bytes after the end"),
        ];
        let mut bad_magic = good.clone();
        bad_magic[0] ^= 1;
        cases.push((bad_magic, "not a frame file"));
        let mut bad_version = good.clone();
        bad_version[4] = 2;
        cases.push((bad_version, "another version"));
        for (b, why) in cases {
            assert_eq!(parse(&b).err(), Some(why));
        }
        for off in NEVER.iter().copied().chain([0x1000, 0x602]) {
            let b = write(&[(off, 1)], &[], (0, 0));
            assert!(parse(&b).is_err(), "register 0x{off:x}");
        }
        for (addr, words) in [(LOCAL - 4, 2), (0x1002, 1)] {
            let b = write(&[], &[(addr, vec![0; words])], (0, 0));
            assert!(parse(&b).is_err(), "segment 0x{addr:x}");
        }
        let b = write(&[], &[(0x1000, vec![])], (0, 0));
        assert!(parse(&b).is_err(), "an empty segment");
        // The edges that are allowed: the last word of local memory, the
        // last register.
        assert!(parse(&write(&[(0xFFC, 0)], &[(LOCAL - 4, vec![7])], (0, 0))).is_ok());
    }

    #[test]
    fn the_hash_is_the_images() {
        // FNV-1a's empty value and one known step.
        assert_eq!(hash([]), 0x811C_9DC5);
        assert_eq!(hash([0]), 0x811C_9DC5u32.wrapping_mul(0x0100_0193));
    }
}
