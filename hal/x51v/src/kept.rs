//! The kept log's format: a second copy of a run log in the PXA's internal
//! SRAM, for a run that may hang (2026-10-20, after `rp-gpubench` run G2:
//! the loader's run log in SDRAM survived watchdog resets, but G2's hang
//! ended in a soft reset and the loader found no log).
//!
//! Two copies, 128 KiB apart, so a copy part-overwritten after a reset is
//! told apart from an intact one by its header and byte sum. Whether the
//! SRAM keeps its contents across a soft reset is unproven: the first hang
//! with the log kept settles it (docs/hw/memory-map.md). The device side,
//! which writes the copies, is in [`crate::devices`].

/// Where each copy starts in the SRAM.
pub const COPIES: [usize; 2] = [64 * 1024, 192 * 1024];
/// Each copy's size, header included.
pub const SIZE: usize = 16 * 1024;
/// Header words: magic, bytes used, byte sum, !magic.
pub const HEADER: usize = 16;
/// The header's first word.
pub const MAGIC: u32 = u32::from_le_bytes(*b"RPKP");
/// Text a copy holds at most.
pub const CAPACITY: usize = SIZE - HEADER;

/// The byte sum the header carries, `start` plus every byte of `data`.
#[must_use]
pub fn sum(start: u32, data: &[u8]) -> u32 {
    data.iter()
        .fold(start, |s, &b| s.wrapping_add(u32::from(b)))
}

/// The header of a copy holding `used` bytes whose sum is `sum`.
#[must_use]
pub fn header(used: usize, sum: u32) -> [u32; 4] {
    [MAGIC, used as u32, sum, !MAGIC]
}

/// A copy's text if its header and sum are intact, or what's wrong.
///
/// # Errors
///
/// "header absent" when no kept log is there (or its header was
/// overwritten); "damaged" when the length, the sum or the text is wrong.
pub fn parse(copy: &[u8]) -> Result<&str, &'static str> {
    if copy.len() < SIZE {
        return Err("too short");
    }
    let w = |i: usize| {
        u32::from_le_bytes([
            copy[4 * i],
            copy[4 * i + 1],
            copy[4 * i + 2],
            copy[4 * i + 3],
        ])
    };
    if w(0) != MAGIC || w(3) != !MAGIC {
        return Err("no kept log (header absent)");
    }
    let used = w(1) as usize;
    if used > CAPACITY {
        return Err("damaged (length out of range)");
    }
    let text = &copy[HEADER..HEADER + used];
    if sum(0, text) != w(2) {
        return Err("damaged (byte sum)");
    }
    core::str::from_utf8(text).map_err(|_| "damaged (not text)")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn copy_with(text: &[u8]) -> [u8; SIZE] {
        let mut c = [0u8; SIZE];
        for (i, w) in header(text.len(), sum(0, text)).iter().enumerate() {
            c[4 * i..4 * i + 4].copy_from_slice(&w.to_le_bytes());
        }
        c[HEADER..HEADER + text.len()].copy_from_slice(text);
        c
    }

    #[test]
    fn an_intact_copy_reads_back() {
        assert_eq!(parse(&copy_with(b"G3: step 7\n")), Ok("G3: step 7\n"));
        assert_eq!(parse(&copy_with(b"")), Ok(""));
    }

    #[test]
    fn the_sum_runs_on_across_appends() {
        assert_eq!(sum(sum(0, b"ab"), b"cd"), sum(0, b"abcd"));
    }

    #[test]
    fn damage_is_told_apart_from_absence() {
        assert_eq!(parse(&[0u8; SIZE]), Err("no kept log (header absent)"));
        let mut c = copy_with(b"hello");
        c[HEADER + 1] ^= 1;
        assert_eq!(parse(&c), Err("damaged (byte sum)"));
        let mut c = copy_with(b"hello");
        c[4..8].copy_from_slice(&(CAPACITY as u32 + 1).to_le_bytes());
        assert_eq!(parse(&c), Err("damaged (length out of range)"));
        let mut c = copy_with(b"hello");
        c[12] ^= 1;
        assert_eq!(parse(&c), Err("no kept log (header absent)"));
        assert_eq!(parse(&c[..SIZE - 1]), Err("too short"));
    }

    #[test]
    fn the_copies_fit_the_sram_apart() {
        for off in COPIES {
            assert!(off + SIZE <= 256 * 1024);
        }
        assert!(COPIES[0] + SIZE <= COPIES[1]);
    }
}
