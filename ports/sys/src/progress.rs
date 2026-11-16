//! What a program says about its own progress, for a test runner to show
//! on the screen: two kinds of line among all it prints.
//!
//! - `rp-step: 3/5 text`: the program is on step 3 of 5, `text` says
//!   what that is.
//! - `rp-note: text`: the newest measurement (a frame rate) for the
//!   screen's note line.
//!
//! In `text` a `|` is a line break on the screen. The lines are ordinary
//! output: they are in the run's log as they were printed.
//!
//! Iterate, don't imitate (rule 0). A port elsewhere shows its progress
//! with the engine's own loading screen (which needs the renderer and the
//! screen call a test image is there to measure without), and a test
//! harness usually greps the engine's chatter for lines it knows, which a
//! quiet command line no longer prints and the next engine words
//! differently. Here the port's own platform layer, which sees the
//! engine's state, says its step in a line of its own; the runner knows
//! nothing of any game, only these two prefixes, and every port after
//! this one gets a labelled screen for two `printf`s.

/// What a step line starts with (after anything else on its line).
pub const STEP: &[u8] = b"rp-step: ";
/// What a note line starts with.
pub const NOTE: &[u8] = b"rp-note: ";
/// A line break on the screen, inside a line's text.
pub const BREAK: u8 = b'|';

/// What one line of a program's output says about its progress.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Said<'a> {
    /// The program is on step `n` of `of`.
    Step {
        /// The step, from 1.
        n: u32,
        /// How many steps there are.
        of: u32,
        /// What the step is.
        text: &'a [u8],
    },
    /// The newest measurement.
    Note(&'a [u8]),
}

/// Where `find` starts in `line`.
fn position(line: &[u8], find: &[u8]) -> Option<usize> {
    line.windows(find.len()).position(|w| w == find)
}

/// A decimal number at the start of `text`: it and what follows it.
/// `None` if there is no digit or it does not fit.
#[must_use]
pub fn number(text: &[u8]) -> Option<(u32, &[u8])> {
    let digits = text.iter().take_while(|b| b.is_ascii_digit()).count();
    if digits == 0 {
        return None;
    }
    let mut n = 0u32;
    for &b in &text[..digits] {
        n = n.checked_mul(10)?.checked_add(u32::from(b - b'0'))?;
    }
    Some((n, &text[digits..]))
}

/// The text of a line from `at`, up to a control character (a terminal
/// colour code after it, a carriage return), without spaces at its end.
fn text(line: &[u8], at: usize) -> &[u8] {
    let rest = &line[at..];
    let end = rest
        .iter()
        .position(|b| b.is_ascii_control())
        .unwrap_or(rest.len());
    rest[..end].trim_ascii_end()
}

/// What `line` (one line of output, without its line end) says, if it is
/// a step or a note. The prefix may come after other text on the line (a
/// program's unfinished line before it, a time stamp). A step line with
/// no `n/of`, a step 0, or one past its count is not a step.
#[must_use]
pub fn said(line: &[u8]) -> Option<Said<'_>> {
    if let Some(at) = position(line, STEP) {
        let (n, rest) = number(&line[at + STEP.len()..])?;
        let (of, rest) = number(rest.strip_prefix(b"/")?)?;
        if n == 0 || n > of {
            return None;
        }
        let skip = line.len() - rest.len();
        let skip = skip + usize::from(rest.first() == Some(&b' '));
        return Some(Said::Step {
            n,
            of,
            text: text(line, skip),
        });
    }
    position(line, NOTE).map(|at| Said::Note(text(line, at + NOTE.len())))
}

/// A program's output cut into lines, however it arrives (a byte at a
/// time, several lines at once): the first `N` bytes of each line are
/// kept, the rest of a longer one is left off.
pub struct Lines<const N: usize> {
    line: [u8; N],
    len: usize,
}

impl<const N: usize> Lines<N> {
    /// No output yet.
    #[allow(clippy::new_without_default)]
    #[must_use]
    pub const fn new() -> Self {
        Self {
            line: [0; N],
            len: 0,
        }
    }

    /// Takes `bytes` of output; `each` gets every line they finish,
    /// without its line end.
    pub fn feed(&mut self, bytes: &[u8], each: &mut dyn FnMut(&[u8])) {
        for &b in bytes {
            if b == b'\n' {
                each(&self.line[..self.len.min(N)]);
                self.len = 0;
            } else {
                if self.len < N {
                    self.line[self.len] = b;
                }
                self.len = self.len.saturating_add(1);
            }
        }
    }
}

/// `seconds` as minutes and the seconds left over, for a clock on the
/// screen (`12:05`).
#[must_use]
pub fn clock(seconds: u32) -> (u32, u32) {
    (seconds / 60, seconds % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_step_line_says_its_step() {
        assert_eq!(
            said(b"rp-step: 3/5 Half-Life is up|loading the map"),
            Some(Said::Step {
                n: 3,
                of: 5,
                text: b"Half-Life is up|loading the map"
            })
        );
        // No text is still a step.
        assert_eq!(
            said(b"rp-step: 5/5"),
            Some(Said::Step {
                n: 5,
                of: 5,
                text: b""
            })
        );
    }

    #[test]
    fn a_note_line_says_its_note() {
        assert_eq!(
            said(b"rp-note: frame rate: 12.3 a second"),
            Some(Said::Note(b"frame rate: 12.3 a second"))
        );
    }

    #[test]
    fn the_prefix_may_come_after_other_text_and_before_a_colour_code() {
        // The engine's time stamp before, its colour reset after.
        assert_eq!(
            said(b"\x1b[0m[00:01:44] rp-note: 9.8 a second  \x1b[0m"),
            Some(Said::Note(b"9.8 a second"))
        );
        assert_eq!(
            said(b"half a line rp-step: 1/2 start\r"),
            Some(Said::Step {
                n: 1,
                of: 2,
                text: b"start"
            })
        );
    }

    #[test]
    fn other_lines_and_damaged_steps_say_nothing() {
        for line in [
            &b"Spawn Server: c1a0"[..],
            b"",
            b"rp-step:",
            b"rp-step: three/5 x",
            b"rp-step: 3 of 5 x",
            b"rp-step: 0/5 x",
            b"rp-step: 6/5 x",
            b"rp-step: 3/ x",
            b"rp-step: 99999999999/5 x",
            b"rp-note:no space",
        ] {
            assert_eq!(said(line), None, "{:?}", core::str::from_utf8(line));
        }
    }

    #[test]
    fn numbers_at_the_edges() {
        assert_eq!(number(b"0"), Some((0, &b""[..])));
        assert_eq!(number(b"4294967295 s"), Some((u32::MAX, &b" s"[..])));
        assert_eq!(number(b"4294967296"), None);
        assert_eq!(number(b"-1"), None);
        assert_eq!(number(b""), None);
    }

    fn lines_of<const N: usize>(chunks: &[&[u8]]) -> Vec<Vec<u8>> {
        let mut lines = Lines::<N>::new();
        let mut got = Vec::new();
        for c in chunks {
            lines.feed(c, &mut |l| got.push(l.to_vec()));
        }
        got
    }

    #[test]
    fn output_is_cut_into_lines_however_it_arrives() {
        let whole = lines_of::<32>(&[b"one\ntwo\n\nthree"]);
        assert_eq!(whole, [&b"one"[..], b"two", b""]);
        // A byte at a time gives the same lines; the unfinished one waits.
        let text = b"one\ntwo\n\nthree\n";
        let bytes: Vec<&[u8]> = text.chunks(1).collect();
        assert_eq!(lines_of::<32>(&bytes), [&b"one"[..], b"two", b"", b"three"]);
    }

    #[test]
    fn a_line_longer_than_the_room_keeps_its_start_and_the_next_is_whole() {
        let got = lines_of::<4>(&[b"abcd\n", b"abcdefgh\nxy\n"]);
        assert_eq!(got, [&b"abcd"[..], b"abcd", b"xy"]);
    }

    #[test]
    fn a_step_split_across_writes_is_found() {
        let mut lines = Lines::<64>::new();
        let mut steps = Vec::new();
        for chunk in [
            &b"rp-st"[..],
            b"ep: 4/5 The map",
            b" plays.\nrp-no",
            b"te: x\n",
        ] {
            lines.feed(chunk, &mut |l| {
                if let Some(Said::Step { n, of, text }) = said(l) {
                    steps.push((n, of, text.to_vec()));
                }
            });
        }
        assert_eq!(steps, [(4, 5, b"The map plays.".to_vec())]);
    }

    #[test]
    fn a_clock_in_minutes_and_seconds() {
        assert_eq!(clock(0), (0, 0));
        assert_eq!(clock(59), (0, 59));
        assert_eq!(clock(60), (1, 0));
        assert_eq!(clock(725), (12, 5));
    }
}
