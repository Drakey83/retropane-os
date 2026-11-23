//! Calendar's motion (calendar-rule0.md: "Week strip that slides under the
//! title", "Smooth zoom between month, week and day"): the timing and the
//! geometry, apart from the drawing so both are tested alone.
//!
//! Iterate, don't imitate (rule 0):
//! - **Their way.** Windows Mobile 5 switched views and weeks with no motion
//!   at all; iOS and Android animate on a free-running GPU compositor with
//!   a curve from a toolkit (CSS's ease-out `cubic-bezier(.215,.61,.355,1)`,
//!   iOS's springs) and time-based steps, and they page a strip on how far
//!   it was dragged or how fast it was flung.
//! - **Ours.** The steps are the X51v's frames, not a clock's: the shell
//!   sends a tick about 30 ms after each frame (the 33 ms rule) and every
//!   frame is held to that budget, so a fixed count of [`FRAMES`] frames
//!   *is* the duration (about 180 ms, inside design-language.md's
//!   150-250 ms) and a frame is never skipped or doubled to catch up with a
//!   clock. The curve is a table of [`FRAMES`] eased positions fitted to
//!   those frames: each frame moves less than the one before (a
//!   deceleration the eye reads as settling), the first moves under half
//!   the way (no jump on the first frame), the last two together move under
//!   4% (the arrival lands softly on the exact pixel). A released strip
//!   pages on distance or on the pen's speed, measured from the touch
//!   filter's own samples.
//! - **Why it wins.** A clock-driven curve on this panel would drop frames
//!   to keep time whenever a frame ran long and stutter; ours has nothing
//!   to drop: the same six pictures every time, each inside its budget,
//!   and the drawing is the renderer's ordinary draw-list difference, so
//!   only what moved is drawn again (the strip's 448 x 62 band; the zoom's
//!   card).

use rp_app::Rect;

/// Frames a motion takes (at the shell's ~30 ms ticks: ~180 ms).
pub const FRAMES: u8 = 6;

/// How far along the motion is at frame `k` (1..=[`FRAMES`]), 256ths:
/// 1 - (1 - t)^3 at t = k / 6, rounded, the last exactly 256. Moves per
/// frame: 108, 72, 44, 23, 8, 1.
pub const EASE: [u16; FRAMES as usize + 1] = [0, 108, 180, 224, 247, 255, 256];

/// The week strip's width: seven 64 px days.
pub const STRIP_W: i16 = 448;

/// How far the pen moves before the strip follows it (a tap's wobble
/// stays a tap: the touch filter's jitter is a few pixels).
pub const DRAG_START: u16 = 8;

/// A released strip pages when it was dragged past this (a quarter of
/// its width) ...
pub const PAGE_AT: i16 = STRIP_W / 4;
/// ... or flung at this many pixels between two of the touch filter's
/// samples (~7 ms apart on the X51v: ~1.7 px/ms, a deliberate flick).
pub const FLING: i16 = 12;

/// The eased position at frame `k` (clamped to the table).
#[must_use]
pub fn ease(k: u8) -> u16 {
    EASE[usize::from(k.min(FRAMES))]
}

/// `a` to `b` at frame `k`.
#[must_use]
pub fn lerp(a: i32, b: i32, k: u8) -> i32 {
    a + (b - a) * i32::from(ease(k)) / 256
}

/// Rectangle `from` grown (or shrunk) toward `to` at frame `k`.
#[must_use]
pub fn grow(from: Rect, to: Rect, k: u8) -> Rect {
    let (x, y) = (
        lerp(from.x.into(), to.x.into(), k),
        lerp(from.y.into(), to.y.into(), k),
    );
    let (r, b) = (
        lerp(from.right().into(), to.right().into(), k),
        lerp(from.bottom().into(), to.bottom().into(), k),
    );
    let c = |v: i32| v.clamp(0, i32::from(u16::MAX)) as u16;
    Rect::new(c(x), c(y), c(r - x), c(b - y))
}

/// Where a released strip goes: dragged `dx` px (left negative), its last
/// step `step` px. `-STRIP_W` brings the next week in (from the right),
/// `STRIP_W` the week before, 0 back to where it was. A fling against the
/// drag's direction goes back.
#[must_use]
pub fn settle(dx: i16, step: i16) -> i16 {
    let flung = step.abs() >= FLING;
    if flung && step.signum() != dx.signum() && dx != 0 {
        return 0;
    }
    let way = if flung { step.signum() } else { dx.signum() };
    if flung || dx.abs() >= PAGE_AT {
        way * STRIP_W
    } else {
        0
    }
}

/// The weeks the strip moves by when it lands at `to` (0 when it goes
/// back).
#[must_use]
pub fn weeks(to: i16) -> i32 {
    -i32::from(to.signum())
}

/// A zoom level: month 0, week 1, day 2 (None for views that don't zoom).
#[must_use]
pub fn level(v: crate::View) -> Option<u8> {
    match v {
        crate::View::Month => Some(0),
        crate::View::Week => Some(1),
        crate::View::Day => Some(2),
        _ => None,
    }
}

/// Text at `x` that may start left of the screen: the glyphs from the
/// first whose pen position is on screen (the ones before it lie wholly
/// left of `x + their width`, inside the strip's clipped margin, so
/// nothing visible is lost while a glyph is narrower than the margin).
/// `None` when no glyph is on screen.
#[must_use]
pub fn on_screen(font: &rp_app::Font, x: i32, s: &[u8]) -> Option<(u16, usize)> {
    if x >= 0 {
        return Some((x.min(i32::from(u16::MAX)) as u16, 0));
    }
    (1..s.len()).find_map(|i| {
        let px = x + i32::from(font.width(&s[..i]));
        (px >= 0).then_some((px as u16, i))
    })
}

/// The part of a span `x..x + w` on screen (x may be negative): its left
/// edge and width, `None` when none is.
#[must_use]
pub fn span(x: i32, w: u16) -> Option<(u16, u16)> {
    let r = x + i32::from(w);
    let l = x.max(0);
    (r > l).then(|| (l as u16, (r - l) as u16))
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;

    #[test]
    fn the_curve_decelerates_and_lands_on_the_pixel() {
        // The table is 1 - (1 - t)^3 at each frame, rounded.
        for k in 1..FRAMES {
            let t = f64::from(k) / f64::from(FRAMES);
            let want = (256.0 * (1.0 - (1.0 - t).powi(3))).round() as u16;
            assert!(want.abs_diff(EASE[usize::from(k)]) <= 1, "frame {k}");
        }
        assert_eq!(ease(FRAMES), 256);
        assert_eq!(ease(0), 0);
        // Each frame moves less than the one before; the first under half.
        let steps: Vec<u16> = EASE.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(steps.windows(2).all(|w| w[1] < w[0]), "{steps:?}");
        assert!(steps[0] < 128);
        assert!(steps[4] + steps[5] < 11, "the arrival soft: under 4%");
        // Exact ends whatever the distance (the last frame is the
        // destination's own picture: no pop when the motion ends).
        for (a, b) in [(0, 448), (-448, 0), (17, -301), (0, 0)] {
            assert_eq!(lerp(a, b, FRAMES), b);
            assert_eq!(lerp(a, b, 0), a);
        }
    }

    #[test]
    fn a_zoom_grows_from_the_cell_to_the_page_and_back() {
        let cell = Rect::new(80, 166, 64, 46);
        let page = Rect::new(0, 36, 480, 564);
        assert_eq!(grow(cell, page, 0), cell);
        assert_eq!(grow(cell, page, FRAMES), page);
        assert_eq!(grow(page, cell, FRAMES), cell);
        // Every frame between holds the one before (it only grows) and
        // stays inside the page.
        let mut last = cell;
        for k in 1..=FRAMES {
            let r = grow(cell, page, k);
            assert!(r.intersect(&last) == last, "frame {k}: {r:?} over {last:?}");
            assert!(page.intersect(&r) == r);
            last = r;
        }
    }

    #[test]
    fn a_released_strip_pages_on_distance_or_speed() {
        // Short and slow: back.
        assert_eq!(settle(-40, -3), 0);
        assert_eq!(settle(60, 2), 0);
        // Far: on.
        assert_eq!(settle(-PAGE_AT, -1), -STRIP_W);
        assert_eq!(settle(PAGE_AT + 30, 0), STRIP_W);
        // A flick pages however short; against the drag it goes back.
        assert_eq!(settle(-20, -FLING), -STRIP_W);
        assert_eq!(settle(20, FLING + 5), STRIP_W);
        assert_eq!(settle(-200, FLING), 0);
        // The weeks moved.
        assert_eq!(weeks(-STRIP_W), 1);
        assert_eq!(weeks(STRIP_W), -1);
        assert_eq!(weeks(0), 0);
    }

    #[test]
    fn spans_and_text_left_of_the_screen() {
        assert_eq!(span(10, 20), Some((10, 20)));
        assert_eq!(span(-5, 20), Some((0, 15)));
        assert_eq!(span(-20, 20), None);
        assert_eq!(span(-30, 20), None);
    }
}
