//! The input trail: what the owner pressed, and whether anything answered.
//!
//! The owner, 2026-11-14 ("your log taking needs to be better, i was
//! pressing buttons on calendar and nothing was responding"): the run log
//! kept only slow answers, so a press that got no answer left no line, and
//! the Calendar freeze couldn't be told apart from an app ignoring presses.
//!
//! - **Windows Mobile** logs nothing. **Android** declares an app "not
//!   responding" after 5 s without an answer to an input and dumps its
//!   threads. **Linux** prints a backtrace after 20 s of a stuck CPU.
//! - **Ours:** every press and touch goes into a RAM ring (the last
//!   [`LEN`], no card write while things work); a press with no answer
//!   within [`IGNORED_MS`] while the loop still runs writes one line naming
//!   it, the screen, and the trail before it. A loop that stops is the run
//!   limit's (its report names the stuck instruction, `rt::rp_fiq`). Half a
//!   second, not five: on one screen a press with no answer is already
//!   wrong, and the line costs one card write, only when it happens.

use core::fmt::Write;

/// Entries kept.
pub const LEN: usize = 32;
/// How long a press may go unanswered before it is called ignored (ms).
pub const IGNORED_MS: u32 = 500;
/// At most one "ignored" line this often (ms): a stuck app pressed ten
/// times says so once, not ten card writes.
pub const QUIET_MS: u32 = 2000;
/// Entries written with an "ignored" line.
pub const SHOWN: usize = 8;

/// What came in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum What {
    /// A button (its name, as the shell's `Key` prints).
    Key(&'static str),
    /// A button the shell has no use for (its virtual key code): logged,
    /// so a press of one reads as that, not as a frozen screen.
    Unused(u8),
    /// The pen down at (x, y).
    Down(u16, u16),
    /// The pen up at (x, y).
    Up(u16, u16),
}

impl What {
    /// Whether it asks for an answer (a press, a lift; not a landing).
    const fn is_press(self) -> bool {
        matches!(self, Self::Key(_) | Self::Unused(_) | Self::Up(..))
    }
}

/// One input.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Entry {
    /// When (ms into the run).
    pub ms: u32,
    /// What.
    pub what: What,
    /// The screen it went to (as the shell's `Screen` prints).
    pub screen: &'static str,
    /// The answer's delay (ms) once a frame showed after it.
    pub answered: Option<u32>,
    /// Already called ignored.
    reported: bool,
}

/// The ring.
pub struct Trail {
    e: [Option<Entry>; LEN],
    next: usize,
    last_line: Option<u32>,
}

impl Default for Trail {
    fn default() -> Self {
        Self::new()
    }
}

impl Trail {
    /// Empty.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            e: [None; LEN],
            next: 0,
            last_line: None,
        }
    }

    /// An input at `ms` to `screen`.
    pub fn push(&mut self, ms: u32, what: What, screen: &'static str) {
        self.e[self.next] = Some(Entry {
            ms,
            what,
            screen,
            answered: None,
            reported: false,
        });
        self.next = (self.next + 1) % LEN;
    }

    /// A frame showed at `ms`: every input not answered yet is.
    pub fn answered(&mut self, ms: u32) {
        for e in self.e.iter_mut().flatten() {
            if e.answered.is_none() {
                e.answered = Some(ms.saturating_sub(e.ms));
            }
        }
    }

    /// Entries oldest first.
    fn in_order(&self) -> impl Iterator<Item = &Entry> {
        (0..LEN).filter_map(move |k| self.e[(self.next + k) % LEN].as_ref())
    }

    /// At `ms`, with no frame waiting to be drawn or shown: the oldest
    /// press unanswered for [`IGNORED_MS`], once each, at most one a
    /// [`QUIET_MS`] (the others still marked, so a burst is one line).
    pub fn ignored(&mut self, ms: u32, frame_waiting: bool) -> Option<Entry> {
        if frame_waiting {
            return None;
        }
        let due = |e: &Entry| {
            e.what.is_press()
                && e.answered.is_none()
                && !e.reported
                && ms.saturating_sub(e.ms) >= IGNORED_MS
        };
        let first = *self.in_order().find(|e| due(e))?;
        for e in self.e.iter_mut().flatten() {
            if due(e) {
                e.reported = true;
            }
        }
        if self.last_line.is_some_and(|t| ms.saturating_sub(t) < QUIET_MS) {
            return None;
        }
        self.last_line = Some(ms);
        Some(first)
    }

    /// The last [`SHOWN`] entries, oldest first, on one line.
    ///
    /// # Errors
    /// The writer's.
    pub fn write_last(&self, w: &mut dyn Write) -> core::fmt::Result {
        let n = self.in_order().count();
        for (k, e) in self.in_order().skip(n.saturating_sub(SHOWN)).enumerate() {
            if k > 0 {
                w.write_str("; ")?;
            }
            write_entry(w, e)?;
        }
        Ok(())
    }
}

/// `10852123 key Action on Program, answered in 42 ms` (or `no answer`).
///
/// # Errors
/// The writer's.
pub fn write_entry(w: &mut dyn Write, e: &Entry) -> core::fmt::Result {
    write!(w, "{} ", e.ms)?;
    match e.what {
        What::Key(k) => write!(w, "key {k}")?,
        What::Unused(vk) => write!(w, "button {vk:#04x} (not used)")?,
        What::Down(x, y) => write!(w, "pen down {x},{y}")?,
        What::Up(x, y) => write!(w, "pen up {x},{y}")?,
    }
    write!(w, " on {}", e.screen)?;
    match e.answered {
        Some(d) => write!(w, ", answered in {d} ms"),
        None => w.write_str(", no answer"),
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::string::String;

    #[test]
    fn a_press_answered_is_never_called_ignored() {
        let mut t = Trail::new();
        t.push(1000, What::Key("Action"), "Program");
        t.answered(1040);
        assert_eq!(t.ignored(5000, false), None);
    }

    #[test]
    fn a_press_unanswered_is_called_ignored_after_half_a_second_once() {
        let mut t = Trail::new();
        t.push(1000, What::Key("Action"), "Program");
        // Not yet: under the threshold, or a frame still to show.
        assert_eq!(t.ignored(1499, false), None);
        assert_eq!(t.ignored(1600, true), None);
        let e = t.ignored(1500, false).expect("ignored");
        assert_eq!((e.ms, e.what), (1000, What::Key("Action")));
        // Once.
        assert_eq!(t.ignored(9000, false), None);
    }

    #[test]
    fn a_landing_asks_for_no_answer_its_lift_does() {
        let mut t = Trail::new();
        t.push(1000, What::Down(10, 20), "Program");
        assert_eq!(t.ignored(2000, false), None);
        t.push(2000, What::Up(10, 20), "Program");
        assert!(t.ignored(2500, false).is_some());
    }

    #[test]
    fn a_burst_of_ignored_presses_is_one_line() {
        let mut t = Trail::new();
        for k in 0..10 {
            t.push(1000 + k * 100, What::Key("Up"), "Program");
        }
        assert!(t.ignored(2000, false).is_some());
        // The rest of the burst came due within the quiet time: no line.
        assert_eq!(t.ignored(2500, false), None);
        // A new press after it: a new line.
        t.push(4000, What::Key("Down"), "Program");
        assert!(t.ignored(4600, false).is_some());
    }

    #[test]
    fn the_trail_keeps_the_last_entries_in_order() {
        let mut t = Trail::new();
        for k in 0..(LEN as u32 + 5) {
            t.push(k, What::Key("Up"), "Today");
        }
        t.answered(LEN as u32 + 10);
        let mut s = String::new();
        t.write_last(&mut s).unwrap();
        assert_eq!(s.matches("; ").count(), SHOWN - 1);
        assert!(s.starts_with(&std::format!("{} key Up on Today", LEN as u32 + 5 - SHOWN as u32)));
        assert!(s.ends_with(", answered in 6 ms"));
    }

    #[test]
    fn an_entry_reads_plainly() {
        let mut s = String::new();
        let mut t = Trail::new();
        t.push(10_852_123, What::Up(240, 300), "Program");
        t.write_last(&mut s).unwrap();
        assert_eq!(s, "10852123 pen up 240,300 on Program, no answer");
    }
}
