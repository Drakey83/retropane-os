//! The command line a Linux-style starter leaves (R193; the sheet is
//! `docs/research/sheets/r193-x50v-cpld1c-bit3.md`).
//!
//! HaRET's `bootlinux` (and QEMU's `-kernel`) starts an image with `r2`
//! pointing at an ATAG list: `[size in words, tag]` headers, `ATAG_CORE`
//! first, `ATAG_NONE` last. Its `ATAG_CMDLINE` carries the script's `set
//! cmdline`, which is how the owner picks an opt-in by the folder he starts
//! HaRET from, without the X51v (booting the same card's other folder)
//! ever seeing it. `r2` is untrusted: [`window`] accepts only a word-aligned
//! address in the stretch of SDRAM below the CPLD table where HaRET puts the
//! list and RetroPane keeps nothing, and [`cmdline`] walks at most
//! [`MAX_TAGS`] tags, each checked against the window's end; anything
//! malformed is "no command line" (ADR-0017: one known word is accepted,
//! the rest ignored).

/// `ATAG_CORE`: the list's first tag.
pub const CORE: u32 = 0x5441_0001;
/// `ATAG_CMDLINE`: the command line, NUL-terminated, padded to words.
pub const CMDLINE: u32 = 0x5441_0009;
/// Tags walked at most.
pub const MAX_TAGS: usize = 64;
/// Where a list is believed: from SDRAM's start as HaRET sets `ramaddr`
/// (`0xA8000000`; the list at `+ 0x100`) to Windows Mobile's CPLD table
/// (`0xA8001320`), which nothing before it in RetroPane writes.
pub const WINDOW: core::ops::Range<u32> = 0xA800_0000..0xA800_1320;
/// Command-line bytes kept at most.
pub const LINE: usize = 256;

/// Why a list was not read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bad {
    /// `r2` outside [`WINDOW`] or not word aligned.
    Address,
    /// The first tag isn't `ATAG_CORE` (size 2 or 5).
    NoCore,
    /// A tag runs past the window, or is shorter than its header.
    Size,
    /// More than [`MAX_TAGS`] tags without `ATAG_NONE`.
    TooLong,
}

impl Bad {
    /// In words, for the log.
    #[must_use]
    pub const fn says(self) -> &'static str {
        match self {
            Self::Address => "r2 is no ATAG list's address",
            Self::NoCore => "no ATAG_CORE first",
            Self::Size => "a tag's size runs past the list",
            Self::TooLong => "too many tags",
        }
    }
}

/// The words to read for a list at `r2`: its address and the count up to
/// the window's end; [`Bad::Address`] outside it.
pub fn window(r2: u32) -> Result<(u32, usize), Bad> {
    if !WINDOW.contains(&r2) || !r2.is_multiple_of(4) {
        return Err(Bad::Address);
    }
    Ok((r2, ((WINDOW.end - r2) / 4) as usize))
}

/// The command line in `words` (an ATAG list) copied into `out`: its
/// length (to the first NUL, at most [`LINE`]), `Ok(0)` with no
/// `ATAG_CMDLINE`.
pub fn cmdline(words: &[u32], out: &mut [u8; LINE]) -> Result<usize, Bad> {
    let mut i = 0;
    for n in 0..MAX_TAGS {
        let (Some(&size), tag) = (words.get(i), words.get(i + 1).copied()) else {
            return Err(Bad::Size);
        };
        if n == 0 && (tag != Some(CORE) || !(size == 2 || size == 5)) {
            return Err(Bad::NoCore);
        }
        if size == 0 || tag == Some(0) {
            return Ok(0);
        }
        let size = size as usize;
        if size < 2 || i + size > words.len() {
            return Err(Bad::Size);
        }
        if tag == Some(CMDLINE) {
            let mut len = 0;
            'words: for &w in &words[i + 2..i + size] {
                for b in w.to_le_bytes() {
                    if b == 0 || len == LINE {
                        break 'words;
                    }
                    out[len] = b;
                    len += 1;
                }
            }
            return Ok(len);
        }
        i += size;
    }
    Err(Bad::TooLong)
}

/// The option `key` in a command line: `None` absent, `Some(None)` as a
/// bare word, `Some(Some(value))` as `key=value`. Only whole words match.
#[must_use]
pub fn option<'a>(line: &'a [u8], key: &[u8]) -> Option<Option<&'a [u8]>> {
    line.split(|&b| b == b' ').find_map(|w| {
        let rest = w.strip_prefix(key)?;
        match rest {
            [] => Some(None),
            [b'=', v @ ..] => Some(Some(v)),
            _ => None,
        }
    })
}

/// The X50v USB opt-in's key (`docs/research/sheets/r193-x50v-cpld1c-bit3.md`).
pub const X50V_BIT3: &[u8] = b"rp.x50v-usb-bit3";
/// When the opt-in sets CPLD `0x1C` bit 3 by default (milliseconds after
/// the loader's start): a 30 s baseline before it in the same log.
pub const X50V_BIT3_AT_MS: u32 = 30_000;

/// What the command line asks for CPLD `0x1C` bit 3.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bit3 {
    /// Not asked: the bit stays as found.
    Off,
    /// Set it this many milliseconds after the start.
    At(u32),
    /// Asked with a value that isn't 0-300 seconds: nothing is set, and the
    /// log says so.
    Bad,
}

/// The opt-in in `line`: bare, the default delay; `=s`, `s` seconds (0-300).
#[must_use]
pub fn bit3(line: &[u8]) -> Bit3 {
    match option(line, X50V_BIT3) {
        None => Bit3::Off,
        Some(None) => Bit3::At(X50V_BIT3_AT_MS),
        Some(Some(v)) => match core::str::from_utf8(v)
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
        {
            Some(s) if s <= 300 && !v.is_empty() && v.iter().all(u8::is_ascii_digit) => {
                Bit3::At(s * 1000)
            }
            _ => Bit3::Bad,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A list as HaRET builds it: CORE (5 words), MEM, CMDLINE, NONE.
    fn list(line: &[u8]) -> [u32; 64] {
        let mut w = [0u32; 64];
        w[..5].copy_from_slice(&[5, CORE, 1, 4096, 0]);
        w[5..9].copy_from_slice(&[4, 0x5441_0002, 0x0400_0000, 0xA000_0000]);
        let words = line.len() / 4 + 1;
        w[9] = 2 + words as u32;
        w[10] = CMDLINE;
        for (k, b) in line.iter().enumerate() {
            w[11 + k / 4] |= u32::from(*b) << (8 * (k % 4));
        }
        // NONE after (already zeros).
        w
    }

    fn line_of(words: &[u32]) -> Result<std::vec::Vec<u8>, Bad> {
        let mut out = [0u8; LINE];
        cmdline(words, &mut out).map(|n| out[..n].to_vec())
    }

    extern crate std;

    #[test]
    fn a_harets_list_gives_its_command_line() {
        assert_eq!(
            line_of(&list(b"rp.x50v-usb-bit3")).unwrap(),
            b"rp.x50v-usb-bit3"
        );
        // Exactly a word's multiple: the NUL in the next word.
        assert_eq!(line_of(&list(b"abcd")).unwrap(), b"abcd");
        assert_eq!(line_of(&list(b"")).unwrap(), b"");
    }

    #[test]
    fn no_cmdline_tag_is_an_empty_line_and_a_bad_list_is_said() {
        let mut w = [0u32; 8];
        w[..2].copy_from_slice(&[2, CORE]);
        assert_eq!(line_of(&w), Ok(std::vec![]));
        // Not CORE first, or a CORE of a wrong size.
        assert_eq!(line_of(&[2, CMDLINE, 0, 0]), Err(Bad::NoCore));
        assert_eq!(line_of(&[3, CORE, 0, 0, 0]), Err(Bad::NoCore));
        assert_eq!(line_of(&[0xFFFF_FFFF, 0]), Err(Bad::NoCore));
        // A tag running past the end, or shorter than its header.
        assert_eq!(line_of(&[2, CORE, 40, CMDLINE, 0]), Err(Bad::Size));
        assert_eq!(line_of(&[2, CORE, 1, CMDLINE, 0]), Err(Bad::Size));
        // The list cut before NONE.
        assert_eq!(line_of(&[2, CORE]), Err(Bad::Size));
        assert_eq!(line_of(&[]), Err(Bad::Size));
        // Tags that never end.
        let mut loop_ = [0u32; 2 + 2 * 70];
        loop_[..2].copy_from_slice(&[2, CORE]);
        for k in 0..70 {
            loop_[2 + 2 * k..4 + 2 * k].copy_from_slice(&[2, 0x5441_0002]);
        }
        assert_eq!(line_of(&loop_), Err(Bad::TooLong));
    }

    #[test]
    fn a_line_with_no_nul_stops_at_its_tag_and_at_the_buffer() {
        let mut w = [0u32; 2 + 2 + 100];
        w[..2].copy_from_slice(&[2, CORE]);
        w[2] = 2 + 100;
        w[3] = CMDLINE;
        for x in &mut w[4..] {
            *x = u32::from_le_bytes(*b"xxxx");
        }
        assert_eq!(line_of(&w).unwrap().len(), LINE);
        let mut w = [0u32; 6];
        w[..4].copy_from_slice(&[2, CORE, 3, CMDLINE]);
        w[4] = u32::from_le_bytes(*b"abcd");
        w[5] = u32::from_le_bytes(*b"efgh");
        // The tag is 3 words: one word of line, not the next.
        assert_eq!(line_of(&w).unwrap(), b"abcd");
    }

    #[test]
    fn only_a_window_address_is_read() {
        assert_eq!(window(0xA800_0100), Ok((0xA800_0100, (0x1320 - 0x100) / 4)));
        assert_eq!(window(0xA800_0000).unwrap().1, 0x1320 / 4);
        for bad in [
            0,
            0xA800_0102,
            0xA800_1320,
            0xA7FF_FFFC,
            0xA000_0100,
            0xFFFF_FFFC,
        ] {
            assert_eq!(window(bad), Err(Bad::Address), "{bad:#x}");
        }
    }

    #[test]
    fn the_opt_in_matches_whole_words_only() {
        assert_eq!(bit3(b""), Bit3::Off);
        assert_eq!(bit3(b"rp.x50v-usb-bit3"), Bit3::At(30_000));
        assert_eq!(bit3(b"console=x rp.x50v-usb-bit3 quiet"), Bit3::At(30_000));
        assert_eq!(bit3(b"rp.x50v-usb-bit3=0"), Bit3::At(0));
        assert_eq!(bit3(b"rp.x50v-usb-bit3=300"), Bit3::At(300_000));
        for bad in ["=301", "=", "=-1", "=+5", "=1s", "=99999999999"] {
            assert_eq!(
                bit3(std::format!("rp.x50v-usb-bit3{bad}").as_bytes()),
                Bit3::Bad,
                "{bad}"
            );
        }
        // Longer or shorter words don't match.
        assert_eq!(bit3(b"rp.x50v-usb-bit33"), Bit3::Off);
        assert_eq!(bit3(b"xrp.x50v-usb-bit3"), Bit3::Off);
        assert_eq!(bit3(b"rp.x50v-usb-bit"), Bit3::Off);
        assert_eq!(option(b"a b=1 c", b"b"), Some(Some(&b"1"[..])));
    }
}
