//! When to start a frame so it never tears (R20, ADR-0034 P1 = C, the
//! owner's ruling 2026-10-28 for the shell and apps: C first, A as the
//! fallback).
//!
//! The 3D core draws straight into the picture the display is showing
//! (C). A top-down render begun when the scan is at line `L` is tear-free
//! when it never overtakes the scan in this refresh and has finished every
//! row before the scan comes round in the next (`rp-gpubench` GC2BZ on the
//! X51v: 645 lines a refresh, 640 shown, 27.46 us a line):
//!
//! `640 - T / line  <=  L  <=  1285 - T / line`  (T the render's time)
//!
//! Rule 0: Dell double-buffers and flips in the blanking (A: a copy or a
//! second picture every frame); the textbook beam racer starts at one line
//! from one estimate of `T`. Ours plans from a band of recent render times:
//! the fastest recent frame (x 4/5) sets the earliest safe line, and twice
//! the last frame (the owner's spike rule: a frame suddenly much longer than
//! the last one), never less than the slowest recent x 11/10, must still
//! finish before the scan comes round, a tile row of margin on
//! each side (GC2BZ: line 320 was clean where the bare formula said 335),
//! and the earliest safe line is taken (the later the start, the less room
//! a slow frame has). Where the band doesn't fit, the order isn't top-down
//! (the checkerboard), or there is no history yet: A.

/// The display's scan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scan {
    /// Lines a refresh, the blanking's included.
    pub total: u32,
    /// Lines shown.
    pub shown: u32,
    /// One line's time, ns.
    pub line_ns: u32,
}

/// The X51v's VGA scan under Windows Mobile's 56 Hz set-up (`DISPPLL`
/// `0xEC5`; GC2BZ: 645 lines, 17.7 ms a refresh).
pub const X51V: Scan = Scan {
    total: 645,
    shown: 640,
    line_ns: 27_460,
};

/// The X51v's VGA scan under RetroPane's 60 Hz set-up (`DISPPLL` `0xFC5`):
/// 25,687 ns a line, measured over 64 frames on the X51v (R108 run 3,
/// gpubench-0214; R111). The panel timing's nominal (648 pixel clocks at
/// 25.1875 MHz) is 25,727 ns: 0.16% off, a crystal's tolerance at most a
/// line or two of a 1,285-line plan, inside [`MARGIN`].
pub const X51V_60HZ: Scan = Scan {
    total: 645,
    shown: 640,
    line_ns: 25_687,
};

/// Lines of margin each side: one tile row (rows land a tile row at a time).
pub const MARGIN: u32 = 16;
/// Render times kept.
const KEEP: usize = 8;

/// What a frame does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plan {
    /// C: start rendering into the shown picture when the scan reaches
    /// `line`.
    Behind {
        /// The scan line to start at.
        line: u32,
    },
    /// A: render into the picture not shown and flip in the blanking.
    Copy,
}

/// Recent render times and the plan they give.
#[derive(Clone, Copy, Debug, Default)]
pub struct Schedule {
    us: [u32; KEEP],
    n: usize,
    next: usize,
}

impl Schedule {
    /// No history.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            us: [0; KEEP],
            n: 0,
            next: 0,
        }
    }

    /// A frame's render time, us.
    pub fn record(&mut self, render_us: u32) {
        self.us[self.next] = render_us;
        self.next = (self.next + 1) % KEEP;
        self.n = (self.n + 1).min(KEEP);
    }

    /// Forgets the history (a scene change the times don't foretell).
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// The band of render times planned for, us, or `None` with no
    /// history: from the fastest recent frame x 4/5 (a sudden fast frame
    /// must not overtake the scan) to twice the last frame (the owner's
    /// spike rule: a frame suddenly much longer than the last one), never
    /// under the slowest recent frame x 11/10 (GC2CZ's first run on the
    /// X51v: twice the slowest kept every frame after one spike on A for
    /// eight frames).
    #[must_use]
    pub fn band(&self) -> Option<(u32, u32)> {
        let recent = &self.us[..self.n];
        let lo = *recent.iter().min()?;
        let hi = *recent.iter().max()?;
        let last = self.us[(self.next + KEEP - 1) % KEEP];
        let fast = lo / 5 * 4 + lo % 5 * 4 / 5;
        let slow = last.saturating_mul(2).max(hi.saturating_add(hi / 10));
        Some((fast, slow))
    }

    /// The next frame's plan on `scan`, for a whole-screen render whose
    /// tiles land top down (`top_down`: rows, interleaved) or not
    /// (checkerboard, reversed).
    #[must_use]
    pub fn plan(&self, scan: &Scan, top_down: bool) -> Plan {
        self.plan_rows(scan, top_down, (0, scan.shown))
    }

    /// [`Self::plan`] for a render of the band of screen rows `top..top +
    /// rows` alone (a scene's area, whole tile rows): the earliest line of
    /// [`Self::window`].
    #[must_use]
    pub fn plan_rows(&self, scan: &Scan, top_down: bool, band: (u32, u32)) -> Plan {
        match self.window(scan, top_down, band) {
            Some((line, _)) => Plan::Behind { line },
            None => Plan::Copy,
        }
    }

    /// The scan lines a top-down render of the band `top..top + rows` may
    /// start on, tear-free for every render time in [`Self::band`]:
    /// `(earliest, latest)`, or `None` (A: no history, not top down, or
    /// the band doesn't fit). Rule 0: a whole screen's plan is the band
    /// `0..640`; a scene's area needs the scan only past its own rows, so
    /// a small area fits render times the whole screen's formula refuses,
    /// and the window (not one line) leaves the caller room to start late.
    #[must_use]
    pub fn window(
        &self,
        scan: &Scan,
        top_down: bool,
        (top, rows): (u32, u32),
    ) -> Option<(u32, u32)> {
        let (fast, slow) = self.band()?;
        if !top_down || rows == 0 {
            return None;
        }
        let lines = |us: u32| (u64::from(us) * 1000).div_ceil(u64::from(scan.line_ns));
        let (top, bottom) = (u64::from(top), u64::from(top) + u64::from(rows));
        let (total, tile_rows) = (u64::from(scan.total), u64::from(rows).div_ceil(16));
        // Never overtake the scan in this refresh (the fastest render): its
        // bottom tile row lands after the scan has passed the band's bottom
        // (the binding one when the render is faster than the scan), and
        // its top tile row after the scan has passed that row (when
        // slower); a start once the scan has left the band needs nothing.
        let margin = u64::from(MARGIN);
        let by_bottom = (bottom + margin).saturating_sub(lines(fast));
        let by_top = (top + 16 + margin).saturating_sub(lines(fast) / tile_rows);
        let earliest = by_bottom.max(by_top).min(bottom);
        // Finished before the scan comes round (the slowest render): its
        // bottom tile row before the scan reaches the band's bottom again,
        // and its top tile row before the scan reaches the band's top (the
        // binding one when the render is faster than the scan).
        let by_bottom = (total + bottom).checked_sub(lines(slow) + margin)?;
        let by_top = (total + top).checked_sub(lines(slow).div_ceil(tile_rows) + 1)?;
        let latest = by_bottom.min(by_top).min(total - 1);
        (earliest <= latest).then_some((earliest as u32, latest as u32))
    }
}

/// Whether a top-down render of `us` begun at scan line `start` is
/// tear-free on `scan`, its 640 rows landing a 16-row tile row at a time,
/// evenly (a model of GC2BZ's measurement, for tests and checks).
#[must_use]
pub fn tear_free(scan: &Scan, start: u32, us: u32) -> bool {
    tear_free_rows(scan, (0, scan.shown), start, us)
}

/// [`tear_free`] for a render of the band of rows `first..first + rows`
/// alone.
#[must_use]
pub fn tear_free_rows(scan: &Scan, (first, rows): (u32, u32), start: u32, us: u32) -> bool {
    let tile_rows = u64::from(rows) / 16;
    let line_ns = u64::from(scan.line_ns);
    let start_ns = u64::from(start) * line_ns;
    for j in 0..tile_rows {
        // Tile row j lands (all 16 lines) at this time after the start.
        let land = (j + 1) * u64::from(us) * 1000 / tile_rows;
        let (top, bottom) = (u64::from(first) + j * 16, u64::from(first) + j * 16 + 15);
        // The scan's position at `land`, in lines since this refresh's top.
        let at = (start_ns + land) / line_ns;
        let total = u64::from(scan.total);
        // Shown in this refresh (the scan hasn't reached its top yet) or
        // the next (the scan passed its bottom this refresh and hasn't come
        // round to its top again): anything else is torn.
        let this = at < top && u64::from(start) <= top;
        let next = (u64::from(start) > bottom || at > bottom) && at < total + top;
        if this {
            // Landed before the scan reached it in this refresh: shows new
            // a refresh early, while rows above it (begun below `start`)
            // show old: torn unless every row is in this refresh too.
            return false;
        }
        if !next {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(times: &[u32]) -> Schedule {
        let mut s = Schedule::new();
        for &t in times {
            s.record(t);
        }
        s
    }

    #[test]
    fn the_model_agrees_with_gc2bz() {
        // 74.75 / 1's flat frame (8.4 ms): line 0 torn (the render
        // overtakes the scan half way down). GC2BZ saw line 320 clean (the
        // bare formula: 335); the model is a tile row stricter (a tile row
        // landing while the scan passes through it counts as torn), so it
        // first holds a tile row past the formula: the margin's side.
        assert!(!tear_free(&X51V, 0, 8_400));
        assert!(!tear_free(&X51V, 320, 8_400));
        assert!(tear_free(&X51V, 335 + 16, 8_400));
        // Windows Mobile's clock (12.5-19.6 ms): line 320 clean, the 19.6
        // ms textured frame (longer than a refresh) too.
        assert!(tear_free(&X51V, 320, 12_500));
        assert!(tear_free(&X51V, 320, 19_600));
        // Line 0 clean when the render is slower than the scan.
        assert!(tear_free(&X51V, 0, 19_600));
        // Past two refreshes no line works.
        assert!((0..645).all(|l| !tear_free(&X51V, l, 36_000)));
    }

    #[test]
    fn no_history_or_no_top_down_order_takes_the_copy() {
        assert_eq!(Schedule::new().plan(&X51V, true), Plan::Copy);
        assert_eq!(with(&[8_400]).plan(&X51V, false), Plan::Copy);
    }

    #[test]
    fn every_plan_is_tear_free_across_its_band() {
        // Steady frames, spikes up to 2x, frames to 4/5 of the fastest:
        // every planned line holds for every time in the band.
        for base in (500..36_000).step_by(250) {
            for spread in [0, base / 4, base / 2] {
                let s = with(&[base, base + spread, base]);
                let Plan::Behind { line } = s.plan(&X51V, true) else {
                    continue;
                };
                let (fast, slow) = s.band().unwrap();
                for t in [fast, (fast + slow) / 2, slow] {
                    assert!(
                        tear_free(&X51V, line, t),
                        "history {base}+{spread}: line {line}, a {t} us render tears"
                    );
                }
            }
        }
    }

    #[test]
    fn a_sudden_spike_never_overtakes_the_scan() {
        // The owner's rule: a run of fast frames, then one twice as long.
        let s = with(&[8_400; 8]);
        let Plan::Behind { line } = s.plan(&X51V, true) else {
            panic!("8.4 ms frames fit C");
        };
        assert!(tear_free(&X51V, line, 8_400));
        assert!(
            tear_free(&X51V, line, 16_800),
            "the 2x spike from line {line}"
        );
        // And it planned with margin: later than GC2BZ's clean 320.
        assert!(line > 320, "{line}");
    }

    #[test]
    fn after_a_spike_c_comes_back_at_once() {
        // 74.75 / 1's frames (GC2CZ): 8.4 ms, the textured 16 ms spike.
        let mut s = with(&[8_400, 8_400, 8_400]);
        s.record(16_000);
        // Right after the spike twice it doesn't fit: A once.
        assert_eq!(s.plan(&X51V, true), Plan::Copy);
        s.record(8_400);
        let Plan::Behind { line } = s.plan(&X51V, true) else {
            panic!("a fast frame after the spike goes back to C");
        };
        for t in [6_720, 8_400, 16_800, 17_600] {
            assert!(tear_free(&X51V, line, t), "{t} us from line {line}");
        }
    }

    #[test]
    fn frames_too_long_or_too_spread_take_the_copy() {
        // Slowest x 2 past two refreshes: A.
        assert_eq!(with(&[18_000]).plan(&X51V, true), Plan::Copy);
        // Steady 12.5 ms (WM clock): C.
        assert!(matches!(
            with(&[12_500; 4]).plan(&X51V, true),
            Plan::Behind { .. }
        ));
    }

    #[test]
    fn every_line_of_every_window_is_tear_free_across_its_band() {
        // Bands across the screen (a scene's area), steady and spiky
        // histories: the window's first, middle and last lines hold for the
        // band's fastest, middle and slowest render, on both scans.
        let bands = [
            (0, 640),
            (0, 320),
            (304, 64),
            (320, 320),
            (608, 32),
            (16, 160),
        ];
        let mut windows = 0;
        for scan in [X51V, X51V_60HZ] {
            for band in bands {
                for base in (300..36_000).step_by(700) {
                    for spread in [0, base / 4, base / 2] {
                        let s = with(&[base, base + spread, base]);
                        let Some((first, last)) = s.window(&scan, true, band) else {
                            continue;
                        };
                        windows += 1;
                        let (fast, slow) = s.band().unwrap();
                        for line in [first, (first + last) / 2, last] {
                            for t in [fast, (fast + slow) / 2, slow] {
                                assert!(
                                    tear_free_rows(&scan, band, line, t),
                                    "band {band:?} history {base}+{spread}: line {line} of {first}-{last}, a {t} us render tears"
                                );
                            }
                        }
                    }
                }
            }
        }
        assert!(windows > 500, "{windows}");
    }

    #[test]
    fn a_band_low_on_the_screen_waits_for_its_own_rows() {
        // 9 ms frames, a scene's 64 rows at the foot of the screen: the
        // whole screen's line would land its top rows before the scan
        // reaches them and its bottom rows after (torn); the band's own
        // window starts as the scan nears the band.
        let s = with(&[9_000; 4]);
        let Plan::Behind { line: whole } = s.plan(&X51V_60HZ, true) else {
            panic!("9 ms frames fit the whole screen");
        };
        let band = (576, 64);
        assert!(!tear_free_rows(&X51V_60HZ, band, whole, 7_200));
        let (first, _) = s.window(&X51V_60HZ, true, band).unwrap();
        assert!(first > whole + 100, "{first} against {whole}");
        assert!(tear_free_rows(&X51V_60HZ, band, first, 7_200));
    }

    #[test]
    fn the_whole_screen_plan_is_the_window_start() {
        for t in (500..20_000).step_by(500) {
            let s = with(&[t, t + t / 3]);
            let w = s.window(&X51V, true, (0, 640));
            assert_eq!(
                s.plan(&X51V, true),
                w.map_or(Plan::Copy, |(line, _)| Plan::Behind { line })
            );
        }
    }

    #[test]
    fn faster_frames_start_later() {
        let mut last = 0;
        for t in (2_000..17_000).rev().step_by(500) {
            if let Plan::Behind { line } = with(&[t]).plan(&X51V, true) {
                assert!(line >= last, "{t} us: line {line} before {last}");
                last = line;
            }
        }
        assert!(last > 0);
    }
}
