//! A text file in as a note's lines (notes-rule0.md, "import text files").
//!
//! A file from the PC or the card is outside input (docs/security.md): it
//! is read as bytes, never trusted. Iterate, don't imitate (rule 0):
//! Windows Mobile's Notes couldn't import at all (Pocket Word converted
//! `.txt` with no checks); desktop editors guess an encoding and show
//! whatever comes. Ours takes UTF-8 (with or without its byte-order mark)
//! and plain ASCII, checks every byte as it streams (overlong forms,
//! surrogates, code points past U+10FFFF, a sequence cut off at the end,
//! NUL and every control byte but tab and the line ends are refused, by
//! offset, so a damaged file is named and nothing of it is kept), turns
//! Markdown's task items into checklist items, and cuts a line longer than
//! a store chunk into continuation lines rather than refusing it. Bytes 1
//! and 2 mark ink and continuation lines inside a note: as control bytes
//! they are refused here, so a file can never forge one.

use crate::doc::{CONT, EDIT_ROOM};

/// The largest file taken: the organiser's whole store is 256 KB.
pub const MAX_FILE: u32 = 256 * 1024;
/// The longest line kept whole (a longer one goes on as continuations).
pub const LINE_MAX: usize = EDIT_ROOM - 8;

/// Why a file was refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Damage {
    /// Bigger than [`MAX_FILE`].
    TooBig(u32),
    /// A NUL or control byte at this offset.
    Control(u32),
    /// Not UTF-8 at this offset.
    NotUtf8(u32),
    /// The file ends inside a character.
    Truncated(u32),
    /// The note had no room for it (the store is full).
    NoRoom,
}

impl Damage {
    /// What the tray says.
    #[must_use]
    pub const fn says(self) -> &'static [u8] {
        match self {
            Self::TooBig(_) => b"NOT IMPORTED: OVER 256 KB",
            Self::Control(_) => b"NOT IMPORTED: NOT A TEXT FILE",
            Self::NotUtf8(_) => b"NOT IMPORTED: NOT UTF-8 TEXT",
            Self::Truncated(_) => b"NOT IMPORTED: FILE CUT SHORT",
            Self::NoRoom => b"NOT IMPORTED: NO ROOM LEFT",
        }
    }
}

/// The streaming reader: bytes in, whole lines out.
pub struct Reader {
    line: [u8; LINE_MAX + 4],
    n: usize,
    /// Bytes read so far (offsets in [`Damage`]).
    at: u32,
    /// The UTF-8 sequence being read: bytes still to come, and the bounds
    /// its next byte must lie in.
    more: u8,
    lo: u8,
    hi: u8,
    /// The last byte was a CR (a CRLF's LF is then no second line end).
    cr: bool,
    /// Whether the line being read continues one cut for length.
    cont: bool,
    /// Bytes of the byte-order mark matched (3: past the start).
    bom: u8,
}

impl Default for Reader {
    fn default() -> Self {
        Self::new()
    }
}

impl Reader {
    /// A reader at the file's start.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            line: [0; LINE_MAX + 4],
            n: 0,
            at: 0,
            more: 0,
            lo: 0x80,
            hi: 0xBF,
            cr: false,
            cont: false,
            bom: 0,
        }
    }

    /// The file's next bytes; each whole line given to `out` (as a note
    /// keeps it). A refusal stops it: what `out` was given is the
    /// caller's to take back.
    ///
    /// # Errors
    /// [`Damage`], or what `out` answers.
    pub fn feed(
        &mut self,
        bytes: &[u8],
        out: &mut dyn FnMut(&[u8]) -> Result<(), Damage>,
    ) -> Result<(), Damage> {
        for &b in bytes {
            let at = self.at;
            self.at += 1;
            // The byte-order mark, at the very start only; a start that
            // only looked like one is read as it was.
            if self.bom < 3 {
                const BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];
                if b == BOM[usize::from(self.bom)] {
                    self.bom += 1;
                    continue;
                }
                let seen = self.bom;
                self.bom = 3;
                for (k, &m) in BOM[..usize::from(seen)].iter().enumerate() {
                    self.byte(m, k as u32, out)?;
                }
            }
            self.byte(b, at, out)?;
        }
        Ok(())
    }

    fn byte(
        &mut self,
        b: u8,
        at: u32,
        out: &mut dyn FnMut(&[u8]) -> Result<(), Damage>,
    ) -> Result<(), Damage> {
        if self.more > 0 {
            if b < self.lo || b > self.hi {
                return Err(Damage::NotUtf8(at));
            }
            self.more -= 1;
            self.lo = 0x80;
            self.hi = 0xBF;
            return self.push(b, out);
        }
        match b {
            b'\n' if self.cr => self.cr = false,
            b'\n' | b'\r' => {
                self.cr = b == b'\r';
                self.end_line(out)?;
            }
            b'\t' => {
                self.cr = false;
                self.push(b' ', out)?;
            }
            0..=0x1F | 0x7F => return Err(Damage::Control(at)),
            0x20..=0x7E => {
                self.cr = false;
                self.push(b, out)?;
            }
            // A lead byte: how many follow, and the first one's bounds (no
            // overlong forms, no surrogates, nothing past U+10FFFF).
            0xC2..=0xDF => self.lead(b, 1, 0x80, 0xBF, out)?,
            0xE0 => self.lead(b, 2, 0xA0, 0xBF, out)?,
            0xE1..=0xEC | 0xEE..=0xEF => self.lead(b, 2, 0x80, 0xBF, out)?,
            0xED => self.lead(b, 2, 0x80, 0x9F, out)?,
            0xF0 => self.lead(b, 3, 0x90, 0xBF, out)?,
            0xF1..=0xF3 => self.lead(b, 3, 0x80, 0xBF, out)?,
            0xF4 => self.lead(b, 3, 0x80, 0x8F, out)?,
            _ => return Err(Damage::NotUtf8(at)),
        }
        Ok(())
    }

    fn lead(
        &mut self,
        b: u8,
        more: u8,
        lo: u8,
        hi: u8,
        out: &mut dyn FnMut(&[u8]) -> Result<(), Damage>,
    ) -> Result<(), Damage> {
        self.cr = false;
        // A whole sequence stays on one line: cut before its lead.
        if self.n + usize::from(more) + 1 > LINE_MAX {
            self.cut(out)?;
        }
        self.more = more;
        self.lo = lo;
        self.hi = hi;
        self.line[self.n] = b;
        self.n += 1;
        Ok(())
    }

    fn push(
        &mut self,
        b: u8,
        out: &mut dyn FnMut(&[u8]) -> Result<(), Damage>,
    ) -> Result<(), Damage> {
        if self.n >= LINE_MAX && self.more == 0 && b & 0xC0 != 0x80 {
            self.cut(out)?;
        }
        self.line[self.n] = b;
        self.n += 1;
        Ok(())
    }

    /// The line so far given out, the rest to follow as a continuation.
    fn cut(&mut self, out: &mut dyn FnMut(&[u8]) -> Result<(), Damage>) -> Result<(), Damage> {
        self.give(out)?;
        self.cont = true;
        Ok(())
    }

    fn end_line(&mut self, out: &mut dyn FnMut(&[u8]) -> Result<(), Damage>) -> Result<(), Damage> {
        self.give(out)?;
        self.cont = false;
        Ok(())
    }

    /// The line read given out as a note's line: Markdown's task items as
    /// checklist items, a continuation marked.
    fn give(&mut self, out: &mut dyn FnMut(&[u8]) -> Result<(), Damage>) -> Result<(), Damage> {
        let l = &self.line[..self.n];
        let mut keep = [0u8; LINE_MAX + 8];
        let mut k = 0;
        let mut put = |b: &[u8], k: &mut usize| {
            keep[*k..*k + b.len()].copy_from_slice(b);
            *k += b.len();
        };
        if self.cont {
            put(&[CONT], &mut k);
            put(l, &mut k);
        } else {
            let body = l
                .strip_prefix(b"- ")
                .or_else(|| l.strip_prefix(b"* "))
                .or_else(|| l.strip_prefix(b"+ "))
                .unwrap_or(l);
            if let Some(t) = body.strip_prefix(b"[ ] ") {
                put(b"[ ] ", &mut k);
                put(t, &mut k);
            } else if let Some(t) = body
                .strip_prefix(b"[x] ")
                .or_else(|| body.strip_prefix(b"[X] "))
            {
                put(b"[x] ", &mut k);
                put(t, &mut k);
            } else {
                put(l, &mut k);
            }
        }
        self.n = 0;
        out(&keep[..k])
    }

    /// The file's end: the last line given out (a file needn't end with a
    /// line end).
    ///
    /// # Errors
    /// [`Damage::Truncated`] inside a character, or what `out` answers.
    pub fn finish(
        &mut self,
        out: &mut dyn FnMut(&[u8]) -> Result<(), Damage>,
    ) -> Result<(), Damage> {
        if self.more > 0 || (self.bom > 0 && self.bom < 3) {
            return Err(Damage::Truncated(self.at));
        }
        if self.n > 0 {
            self.give(out)?;
        }
        Ok(())
    }
}
