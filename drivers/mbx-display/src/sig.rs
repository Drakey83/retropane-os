//! The display's output signature (`DSIG`), read once a refresh, judged
//! against the whole pictures the display may show: how a test on the X51v
//! counts torn refreshes (R20).
//!
//! `DSIG` is a signature of the last refresh sent to the panel, published
//! once the next refresh has begun and its scan line passed 2 (run GC2DB on
//! the X51v). A refresh that showed one whole picture reads that picture's
//! signature; one that showed parts of two reads neither. The emulator
//! doesn't model it (reads 0).
//!
//! Moved here from `rp-gpubench` (run GC2DF, R20's proof run) so every image
//! that judges refreshes uses one copy: `rp-gpubench` and `rp-scene` (R110).
//! Times are the caller's own units (`rp-gpubench` passes `rt::ticks`), so
//! nothing here needs a clock.

use rp_hal_api::RegisterBus;

use crate::reg;

/// The display's output signature register.
pub const DSIG: usize = 0x2184;
/// The scan line from which the last refresh's `DSIG` is published (GC2DB).
pub const PUBLISHED_LINE: u32 = 3;

/// Each refresh's `DSIG` judged against the whole pictures the display may
/// legitimately show: the last one shown whole and the ones sent after it,
/// in order (a frame and its sequence number). A refresh showing one of
/// them moves the display on to it (older ones may never come back); one
/// showing none is torn; one showing a picture already replaced is counted
/// apart (`back`).
#[derive(Clone, Copy, Debug)]
pub struct RefreshJudge {
    sigs: [u32; 4],
    seqs: [u32; 4],
    n: usize,
    gone: Option<u32>,
    /// Refreshes judged.
    pub judged: u32,
    /// Refreshes that showed no whole picture.
    pub torn: u32,
    /// Refreshes that showed a picture already replaced.
    pub back: u32,
}

impl RefreshJudge {
    /// The display showing picture `sig` (sequence `seq`), whole.
    #[must_use]
    pub const fn new(seq: u32, sig: u32) -> Self {
        Self {
            sigs: [sig, 0, 0, 0],
            seqs: [seq, 0, 0, 0],
            n: 1,
            gone: None,
            judged: 0,
            torn: 0,
            back: 0,
        }
    }

    /// Picture `sig` (sequence `seq`) sent towards the screen: it may show
    /// from now on (the oldest of four forgotten).
    pub fn push(&mut self, seq: u32, sig: u32) {
        if self.n == self.sigs.len() {
            self.drop_first(1);
        }
        self.sigs[self.n] = sig;
        self.seqs[self.n] = seq;
        self.n += 1;
    }

    fn drop_first(&mut self, k: usize) {
        self.gone = Some(self.sigs[k - 1]);
        self.sigs.copy_within(k..self.n, 0);
        self.seqs.copy_within(k..self.n, 0);
        self.n -= k;
    }

    /// One refresh's signature: the sequence number of the whole picture
    /// it showed, or `None` (torn, or back to a replaced one).
    pub fn judge(&mut self, sig: u32) -> Option<u32> {
        self.judged += 1;
        match self.sigs[..self.n].iter().position(|&s| s == sig) {
            Some(i) => {
                let seq = self.seqs[i];
                if i > 0 {
                    self.drop_first(i);
                }
                Some(seq)
            }
            None if self.gone == Some(sig) => {
                self.back += 1;
                None
            }
            None => {
                self.torn += 1;
                None
            }
        }
    }
}

/// A watch of the display: each refresh's `DSIG`, read once the next
/// refresh has begun and its line passed [`PUBLISHED_LINE`], judged by a
/// [`RefreshJudge`]. Ticked often (well under a refresh apart) while the
/// test waits; a refresh whose start was never seen apart from the last is
/// counted `missed` (its signature unread).
#[derive(Clone, Copy, Debug)]
pub struct Watch {
    /// The judge.
    pub judge: RefreshJudge,
    /// The scan line at the last look.
    line: u32,
    /// When this refresh's start was seen, and the last's (caller's units).
    start: u32,
    prev: u32,
    /// A refresh and a half, in the caller's units.
    late: u32,
    /// This refresh's start seen and the last one's `DSIG` not yet read.
    pending: bool,
    /// The sequence number waited for, and the start of the first refresh
    /// that showed it (or a later one) whole.
    want: u32,
    /// When the first refresh showing the frame waited for began.
    pub shown: Option<u32>,
    /// Refreshes whose `DSIG` was never read (looks too far apart).
    pub missed: u32,
}

impl Watch {
    /// A watch from `now` (the caller's time; `late` a refresh and a half
    /// in the same units), the scan at `line`.
    #[must_use]
    pub const fn new(judge: RefreshJudge, line: u32, now: u32, late: u32) -> Self {
        Self {
            judge,
            line,
            start: now,
            prev: now,
            late,
            pending: false,
            want: u32::MAX,
            shown: None,
            missed: 0,
        }
    }

    /// One look at the scan line (and `DSIG`, once a refresh) at `now`:
    /// the signature read this look, if one was (for a caller that sorts
    /// the torn ones further).
    pub fn tick(&mut self, regs: &mut impl RegisterBus, now: u32) -> Option<u32> {
        let line = regs.read32(reg::DVLNUM) & 0xFFF;
        if line < self.line {
            // A look more than a refresh and a half after the last start:
            // a refresh passed unseen.
            if self.pending || now.wrapping_sub(self.start) > self.late {
                self.missed += 1;
            }
            self.prev = self.start;
            self.start = now;
            self.pending = true;
        }
        self.line = line;
        if self.pending && line >= PUBLISHED_LINE {
            self.pending = false;
            let sig = regs.read32(DSIG);
            if let Some(seq) = self.judge.judge(sig)
                && seq >= self.want
                && self.want != u32::MAX
                && self.shown.is_none()
            {
                self.shown = Some(self.prev);
            }
            return Some(sig);
        }
        None
    }

    /// Picture `sig` (sequence `seq`) sent towards the screen: its first
    /// whole refresh is timed from its start ([`Self::shown`]).
    pub fn send(&mut self, seq: u32, sig: u32) {
        self.judge.push(seq, sig);
        if seq != self.want {
            self.want = seq;
            self.shown = None;
        }
    }
}

/// The signature of the refresh the scan is in now, once it has finished:
/// the scan line seen to wrap (the next refresh begun), then `DSIG` read
/// once the line has passed [`PUBLISHED_LINE`]. `None` if the line didn't
/// wrap within two refreshes (`frame` one refresh in the units of `now`).
pub fn next(regs: &mut impl RegisterBus, mut now: impl FnMut() -> u32, frame: u32) -> Option<u32> {
    let t0 = now();
    let mut line = || regs.read32(reg::DVLNUM) & 0xFFF;
    let mut prev = line();
    loop {
        let l = line();
        if l < prev {
            break;
        }
        prev = l;
        if now().wrapping_sub(t0) > 2 * frame {
            return None;
        }
    }
    while line() < PUBLISHED_LINE {
        if now().wrapping_sub(t0) > 3 * frame {
            return None;
        }
    }
    Some(regs.read32(DSIG))
}

/// A picture's signature once shown: two refreshes, then the third's.
pub fn settled(
    regs: &mut impl RegisterBus,
    mut now: impl FnMut() -> u32,
    frame: u32,
) -> Option<u32> {
    let _ = next(regs, &mut now, frame);
    let _ = next(regs, &mut now, frame);
    next(regs, now, frame)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rp_hal_mock::MockBus;

    #[test]
    fn a_refresh_is_whole_or_torn() {
        // Frame 1 (sig 0xA) shown; frame 2 (0xB) sent: refreshes may show
        // 0xA until 0xB first shows, then 0xB only.
        let mut j = RefreshJudge::new(1, 0xA);
        j.push(2, 0xB);
        assert_eq!(j.judge(0xA), Some(1));
        assert_eq!(j.judge(0x5), None, "neither: torn");
        assert_eq!(j.judge(0xB), Some(2));
        assert_eq!(j.judge(0xA), None, "back to the replaced picture");
        assert_eq!((j.judged, j.torn, j.back), (4, 1, 1));
        // Back to back: frames sent faster than they show; a frame may be
        // skipped (shown never) but the order holds.
        j.push(3, 0xC);
        j.push(4, 0xD);
        assert_eq!(j.judge(0xD), Some(4), "3 skipped");
        assert_eq!(j.judge(0xC), None);
        assert_eq!(
            (j.torn, j.back),
            (1, 2),
            "a skipped picture after its successor: replaced"
        );
        // The same picture at two places (option 1's screen 1, then screen
        // 0) under one sequence number.
        let mut j = RefreshJudge::new(7, 0x1);
        j.push(8, 0x2);
        j.push(8, 0x3);
        assert_eq!(j.judge(0x2), Some(8));
        assert_eq!(j.judge(0x3), Some(8));
        assert_eq!(j.judge(0x3), Some(8));
        assert_eq!(j.torn + j.back, 0);
        // More than four sent: the oldest forgotten, never wrongly accepted.
        let mut j = RefreshJudge::new(0, 0x10);
        for k in 1..=5 {
            j.push(k, 0x10 + k);
        }
        assert_eq!(j.judge(0x10), None);
        assert_eq!(j.judge(0x15), Some(5));
    }

    #[test]
    fn the_watch_reads_each_refresh_once_after_its_line_passes_2() {
        let mut bus = MockBus::new();
        bus.preset(DSIG, 0xA);
        let mut w = Watch::new(RefreshJudge::new(1, 0xA), 600, 0, 150);
        // Before the wrap: nothing judged.
        bus.preset(reg::DVLNUM, 640);
        w.tick(&mut bus, 10);
        assert_eq!(w.judge.judged, 0);
        // Wrapped, line 1: not yet published.
        bus.preset(reg::DVLNUM, 1);
        w.tick(&mut bus, 20);
        assert_eq!(w.judge.judged, 0);
        // Line 3: read once, never twice in the refresh (both edges: 2 is
        // too early, 3 is the first).
        bus.preset(reg::DVLNUM, 2);
        w.tick(&mut bus, 21);
        assert_eq!(w.judge.judged, 0);
        bus.preset(reg::DVLNUM, 3);
        assert_eq!(w.tick(&mut bus, 22), Some(0xA));
        bus.preset(reg::DVLNUM, 300);
        assert_eq!(w.tick(&mut bus, 60), None);
        assert_eq!((w.judge.judged, w.missed), (1, 0));
        // Frame 2 sent; its first whole refresh's start is `shown`.
        w.send(2, 0xB);
        bus.preset(DSIG, 0xB);
        bus.preset(reg::DVLNUM, 0);
        w.tick(&mut bus, 120);
        bus.preset(reg::DVLNUM, 5);
        w.tick(&mut bus, 121);
        assert_eq!(w.shown, Some(20), "the refresh begun at 20 showed it");
        assert_eq!(w.judge.torn, 0);
    }

    #[test]
    fn looks_too_far_apart_count_missed_refreshes() {
        let mut bus = MockBus::new();
        let mut w = Watch::new(RefreshJudge::new(0, 0), 600, 0, 150);
        // A wrap seen 200 after the last start: a refresh passed unseen.
        bus.preset(reg::DVLNUM, 10);
        w.tick(&mut bus, 200);
        assert_eq!(w.missed, 1);
        // A wrap 100 later, read: none missed.
        bus.preset(reg::DVLNUM, 600);
        w.tick(&mut bus, 250);
        bus.preset(reg::DVLNUM, 10);
        w.tick(&mut bus, 300);
        assert_eq!(w.missed, 1);
        // A wrap seen only at line 1 (not yet published), then the next
        // wrap before a look past line 2: that refresh's DSIG never read.
        bus.preset(reg::DVLNUM, 600);
        w.tick(&mut bus, 301);
        bus.preset(reg::DVLNUM, 1);
        w.tick(&mut bus, 350);
        assert_eq!(w.missed, 1);
        // (the look before the wrap at 2: still unpublished)
        bus.preset(reg::DVLNUM, 2);
        w.tick(&mut bus, 360);
        bus.preset(reg::DVLNUM, 0);
        w.tick(&mut bus, 400);
        assert_eq!(w.missed, 2);
    }

    #[test]
    fn next_waits_for_the_wrap_and_the_publish_line() {
        let mut bus = MockBus::new();
        bus.preset(DSIG, 0x1234);
        bus.script_reads(reg::DVLNUM, &[500, 640, 2, 1, 3]);
        let mut t = 0;
        let got = next(
            &mut bus,
            || {
                t += 1;
                t
            },
            100,
        );
        assert_eq!(got, Some(0x1234));
        // Read only after a look at line 3 or later (2 and 1 too early).
        let lines: std::vec::Vec<u32> = bus
            .log()
            .iter()
            .filter_map(|a| match *a {
                rp_hal_mock::Access::Read { offset, value } if offset == reg::DVLNUM => Some(value),
                _ => None,
            })
            .collect();
        assert_eq!(lines, [500, 640, 2, 1, 3]);
        assert!(matches!(
            bus.log().last(),
            Some(rp_hal_mock::Access::Read { offset: DSIG, .. })
        ));
        // No wrap within two refreshes: None.
        let mut bus = MockBus::new();
        bus.preset(reg::DVLNUM, 7);
        let mut t = 0;
        assert_eq!(
            next(
                &mut bus,
                || {
                    t += 10;
                    t
                },
                100
            ),
            None
        );
    }
}
