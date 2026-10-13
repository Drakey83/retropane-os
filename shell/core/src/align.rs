//! Align screen (item 76): five targets, one at a time, then a check tap.
//!
//! While it runs, the shell takes raw touch readings ([`crate::Input::Raw`])
//! instead of screen points. Each target is held until it turns green
//! ([`HOLD`] readings): the median of the readings is the target's point,
//! so a shaky landing or lift doesn't count. After the fifth, the affine
//! fit (`rp-touch-cal`) is made on all five, and checked by fitting on the
//! four corners alone and predicting the centre: a panel that isn't linear
//! is found here ([`MAX_CENTRE_MISS`]). Then one tap on a check dot, mapped
//! through the new calibration, must land within [`MAX_CHECK_MISS`]. Any
//! failure says why and starts again; Home leaves with the old calibration.

use rp_touch_cal::{Calibration, Point, median};

/// The targets: 10% in from each corner, then the centre (checked last).
pub const TARGETS: [(u16, u16); 5] = [(48, 64), (432, 64), (48, 576), (432, 576), (240, 320)];
/// The check dot (away from every target).
pub const CHECK: (u16, u16) = (336, 448);
/// Readings a target must be held for (~5 ms apart: ~80 ms).
pub const HOLD: usize = 16;
/// Readings kept for the median (the later ones replace the earlier).
const KEEP: usize = 32;
/// Pixels the corners' fit may miss the centre by.
pub const MAX_CENTRE_MISS: u32 = 6;
/// Pixels the check tap may miss its dot by.
pub const MAX_CHECK_MISS: u32 = 10;

/// What the align screen is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Waiting for target `n` (0..5).
    Target(u8),
    /// The fit is made; waiting for the check tap.
    Check(Calibration),
}

/// The align screen's state.
pub struct Align {
    stage: Stage,
    xs: [u16; KEEP],
    ys: [u16; KEEP],
    n: usize,
    points: [Point; 5],
    /// Why the last attempt started again (shown until the next target).
    note: Option<&'static str>,
}

/// What a reading or a lift did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Nothing visible changed.
    Same,
    /// Redraw.
    Changed,
    /// Done: the new calibration.
    Done(Calibration),
}

impl Default for Align {
    fn default() -> Self {
        Self::new()
    }
}

impl Align {
    /// At the first target.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            stage: Stage::Target(0),
            xs: [0; KEEP],
            ys: [0; KEEP],
            n: 0,
            points: [Point {
                rx: 0,
                ry: 0,
                sx: 0,
                sy: 0,
            }; 5],
            note: None,
        }
    }

    /// Where it is.
    #[must_use]
    pub const fn stage(&self) -> Stage {
        self.stage
    }

    /// Whether the current target has been held long enough (green).
    #[must_use]
    pub const fn held(&self) -> bool {
        self.n >= HOLD
    }

    /// Why the last attempt started again, if it did.
    #[must_use]
    pub const fn note(&self) -> Option<&'static str> {
        self.note
    }

    /// Where the dot to touch is now.
    #[must_use]
    pub fn target(&self) -> (u16, u16) {
        match self.stage {
            Stage::Target(i) => TARGETS[usize::from(i).min(TARGETS.len() - 1)],
            Stage::Check(_) => CHECK,
        }
    }

    /// A raw reading while the pen is down.
    pub fn reading(&mut self, rx: u16, ry: u16) -> Step {
        let was_held = self.held();
        let at = self.n % KEEP;
        self.xs[at] = rx;
        self.ys[at] = ry;
        self.n += 1;
        if self.held() != was_held {
            Step::Changed
        } else {
            Step::Same
        }
    }

    fn again(&mut self, why: &'static str) {
        self.stage = Stage::Target(0);
        self.note = Some(why);
    }

    /// The pen lifted.
    pub fn lift(&mut self) -> Step {
        let held = self.held();
        let kept = self.n.min(KEEP);
        self.n = 0;
        if !held {
            if kept > 0 {
                self.note = Some("Hold the stylus until the target turns green");
                return Step::Changed;
            }
            return Step::Same;
        }
        let (rx, ry) = (median(&mut self.xs[..kept]), median(&mut self.ys[..kept]));
        match self.stage {
            Stage::Target(i) => {
                let (sx, sy) = TARGETS[usize::from(i)];
                self.points[usize::from(i)] = Point { rx, ry, sx, sy };
                self.note = None;
                if usize::from(i) + 1 < TARGETS.len() {
                    self.stage = Stage::Target(i + 1);
                    return Step::Changed;
                }
                let fit = Calibration::fit(&self.points);
                let centre = Calibration::check(&self.points, TARGETS.len() - 1);
                match (fit, centre) {
                    (Some(cal), Some(miss)) if miss <= MAX_CENTRE_MISS => {
                        self.stage = Stage::Check(cal);
                    }
                    _ => self.again("The points disagree: once more, from the first"),
                }
                Step::Changed
            }
            Stage::Check(cal) => {
                let (x, y) = cal.map(rx, ry);
                let miss = (x - i64::from(CHECK.0))
                    .unsigned_abs()
                    .max((y - i64::from(CHECK.1)).unsigned_abs());
                if miss <= u64::from(MAX_CHECK_MISS) {
                    Step::Done(cal)
                } else {
                    self.again("That missed the dot: once more, from the first");
                    Step::Changed
                }
            }
        }
    }
}
