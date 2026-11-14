//! Wrapped text with a caret (the note editor, docs/design/notes-rule0.md):
//! where each line of a text starts and ends at a width, and where a byte
//! offset or a point on screen falls among them.
//!
//! Lines break at a new-line byte, else at the last space that fits, else
//! (one long word) where the width runs out. The breaks are found by
//! walking the text; a note is at most 2,000 bytes, and the walk runs when
//! a key changes it, never while the pen moves (P1).

use rp_draw::Font;

/// One line of a wrapped text: the bytes `start..end` (the space or
/// new-line that ended it is not in it).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct LineAt {
    pub start: usize,
    pub end: usize,
}

/// The line starting at `start` of `text` in `width` px, and where the
/// next one starts (`None`: this was the last).
#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
pub(crate) fn line_from(
    text: &[u8],
    start: usize,
    font: &Font,
    width: u16,
) -> (LineAt, Option<usize>) {
    let mut x = 0u16;
    let mut space = None;
    let mut k = start;
    while k < text.len() {
        let b = text[k];
        if b == b'\n' {
            return (LineAt { start, end: k }, Some(k + 1));
        }
        let w = font.width(&text[k..=k]);
        if x + w > width && k > start {
            // Past the edge: back to the last space, else cut the word.
            return match space {
                Some(s) => (LineAt { start, end: s }, Some(s + 1)),
                None => (LineAt { start, end: k }, Some(k)),
            };
        }
        if b == b' ' {
            space = Some(k);
        }
        x += w;
        k += 1;
    }
    (LineAt { start, end: k }, None)
}

/// Calls `each` with every line's number and extent, in order.
#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
pub(crate) fn lines(text: &[u8], font: &Font, width: u16, mut each: impl FnMut(usize, LineAt)) {
    let mut at = Some(0);
    let mut n = 0;
    while let Some(start) = at {
        let (line, next) = line_from(text, start, font, width);
        each(n, line);
        n += 1;
        at = next;
    }
}

/// The line the caret at byte `caret` is on, and its x from the line's
/// left. A caret on a break's byte (the space or new-line that ended a
/// line) shows at that line's end.
#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
pub(crate) fn caret_at(text: &[u8], caret: usize, font: &Font, width: u16) -> (usize, u16) {
    let mut found = (0, 0);
    let mut done = false;
    lines(text, font, width, |n, l| {
        if !done && caret >= l.start {
            let c = caret.min(l.end);
            found = (n, font.width(&text[l.start..c]));
            // The caret is here unless a later line starts at or before it.
            done = caret <= l.end;
        }
    });
    found
}

/// The byte offset nearest to x `x` on line `line` (past the last line:
/// the text's end).
#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
pub(crate) fn offset_at(text: &[u8], line: usize, x: u16, font: &Font, width: u16) -> usize {
    let mut found = text.len();
    lines(text, font, width, |n, l| {
        if n == line {
            let mut at = l.start;
            let mut left = 0u16;
            while at < l.end {
                let w = font.width(&text[at..=at]);
                if left + w / 2 >= x {
                    break;
                }
                left += w;
                at += 1;
            }
            found = at;
        }
    });
    found
}

/// How many lines `text` wraps to.
#[cfg_attr(target_arch = "arm", instruction_set(arm::t32))]
pub(crate) fn count(text: &[u8], font: &Font, width: u16) -> usize {
    let mut n = 0;
    lines(text, font, width, |k, _| n = k + 1);
    n
}
