//! The probe report: pages of fixed-width text lines, each ending in a
//! checksum, so values read off a phone photo can be verified.
//!
//! A line is at most [`TEXT`] characters of content, padded, then a space and
//! two hex digits: the sum of the content's bytes (trailing spaces trimmed)
//! modulo 256. Too-long content is cut and marked with `>` (never silently).

use core::fmt::{self, Write};

/// Characters per screen line (480 px / 12 px cells at scale 2).
pub const COLS: usize = 40;
/// Content characters before the checksum.
pub const TEXT: usize = COLS - 3;
/// Lines per page (40 rows: content plus the footer).
pub const ROWS: usize = 39;
/// Pages.
pub const PAGES: usize = 6;

/// The checksum of a line's content.
#[must_use]
pub fn checksum(content: &[u8]) -> u8 {
    let trimmed = content.trim_ascii_end();
    trimmed.iter().fold(0u8, |a, &b| a.wrapping_add(b))
}

/// One line being formatted.
struct LineBuf {
    buf: [u8; TEXT],
    len: usize,
    cut: bool,
}

impl Write for LineBuf {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for &b in s.as_bytes() {
            if self.len < TEXT {
                // Only printable ASCII reaches the screen font.
                self.buf[self.len] = if (0x20..0x7F).contains(&b) { b } else { b'?' };
                self.len += 1;
            } else {
                self.cut = true;
            }
        }
        Ok(())
    }
}

/// Bytes of storage a report needs.
pub const STORAGE: usize = PAGES * ROWS * COLS;

/// The whole report, stored in a caller-provided buffer (bare-metal images
/// have no heap and can't own a large `static`).
pub struct Report<'a> {
    lines: &'a mut [[u8; COLS]],
    used: [usize; PAGES],
    page: usize,
    /// Lines that didn't fit on their page.
    pub overflow: usize,
}

impl<'a> Report<'a> {
    /// An empty report in `storage` (at least [`STORAGE`] bytes), writing to
    /// page 0.
    ///
    /// # Panics
    /// If `storage` is too small.
    pub fn new(storage: &'a mut [u8]) -> Self {
        let (lines, _) = storage[..STORAGE].as_chunks_mut::<COLS>();
        lines.iter_mut().for_each(|l| l.fill(b' '));
        Self {
            lines,
            used: [0; PAGES],
            page: 0,
            overflow: 0,
        }
    }

    /// Continues on page `page`.
    pub fn on_page(&mut self, page: usize) {
        self.page = page.min(PAGES - 1);
    }

    /// Appends a line to the current page.
    pub fn line(&mut self, args: fmt::Arguments) {
        let mut l = LineBuf {
            buf: [b' '; TEXT],
            len: 0,
            cut: false,
        };
        let _ = l.write_fmt(args);
        if l.cut {
            l.buf[TEXT - 1] = b'>';
        }
        let p = self.page;
        if self.used[p] == ROWS {
            self.overflow += 1;
            return;
        }
        let out = &mut self.lines[p * ROWS + self.used[p]];
        out[..TEXT].copy_from_slice(&l.buf);
        let sum = checksum(&l.buf);
        const HEX: &[u8; 16] = b"0123456789ABCDEF";
        out[TEXT] = b' ';
        out[TEXT + 1] = HEX[usize::from(sum >> 4)];
        out[TEXT + 2] = HEX[usize::from(sum & 0xF)];
        self.used[p] += 1;
    }

    /// Two `name value` pairs per line (name up to 9 characters, value as 8
    /// hex digits).
    pub fn pairs(&mut self, items: &[(&str, u32)]) {
        for pair in items.chunks(2) {
            match pair {
                [(a, x), (b, y)] => self.line(format_args!("{a:<9} {x:08X} {b:<9} {y:08X}")),
                [(a, x)] => self.line(format_args!("{a:<9} {x:08X}")),
                _ => {}
            }
        }
    }

    /// The lines of page `p` (only those written).
    pub fn page_lines(&self, p: usize) -> impl Iterator<Item = &[u8; COLS]> {
        self.lines[p * ROWS..p * ROWS + self.used[p]].iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn storage() -> Vec<u8> {
        vec![0; STORAGE]
    }

    fn text(l: &[u8; COLS]) -> &str {
        core::str::from_utf8(l).unwrap()
    }

    #[test]
    fn lines_end_in_their_checksum() {
        let mut buf = storage();
        let mut r = Report::new(&mut buf);
        r.line(format_args!("CPU ID {:08X}", 0x6905_4117u32));
        let l = r.page_lines(0).next().unwrap();
        let content = &l[..TEXT];
        assert!(text(l).starts_with("CPU ID 69054117"));
        let sum: u32 = b"CPU ID 69054117".iter().map(|&b| u32::from(b)).sum();
        assert_eq!(&text(l)[TEXT + 1..], format!("{:02X}", sum % 256));
        assert_eq!(checksum(content), (sum % 256) as u8);
        assert_eq!(l.len(), COLS);
    }

    #[test]
    fn long_lines_are_cut_visibly() {
        let mut buf = storage();
        let mut r = Report::new(&mut buf);
        r.line(format_args!("{}", "x".repeat(60)));
        let l = r.page_lines(0).next().unwrap();
        assert_eq!(l[TEXT - 1], b'>');
    }

    #[test]
    fn pairs_fit_one_line() {
        let mut buf = storage();
        let mut r = Report::new(&mut buf);
        r.pairs(&[("LMPROTMIN", 0xDEAD_BEEF), ("LMPROTMAX", 1), ("CCCR", 2)]);
        let lines: Vec<_> = r.page_lines(0).map(text).collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("LMPROTMIN DEADBEEF LMPROTMAX 00000001"));
        assert!(!lines[0][..TEXT].contains('>'), "two full pairs fit");
        assert!(lines[1].starts_with("CCCR      00000002"));
    }

    #[test]
    fn full_pages_count_overflow() {
        let mut buf = storage();
        let mut r = Report::new(&mut buf);
        r.on_page(3);
        for i in 0..ROWS + 2 {
            r.line(format_args!("{i}"));
        }
        assert_eq!(r.page_lines(3).count(), ROWS);
        assert_eq!(r.overflow, 2);
        r.on_page(99);
        r.line(format_args!("clamped"));
        assert_eq!(r.overflow, 3, "page 3 is still full");
    }

    #[test]
    fn non_printable_bytes_become_question_marks() {
        let mut buf = storage();
        let mut r = Report::new(&mut buf);
        r.line(format_args!("a\tb"));
        assert!(text(r.page_lines(0).next().unwrap()).starts_with("a?b"));
    }
}
