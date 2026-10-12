//! Dirty rectangles: what changed since the last frame, and copying just that
//! from the RAM surface to the write-only framebuffer.
//!
//! Rectangles that overlap, or whose union wastes little, merge as they're
//! added, so a burst of small changes (a clock's digits, a ticked task)
//! becomes a few copies; a full list merges into the rectangle that grows
//! least. Every rectangle is widened to whole 32-bit words (even x and
//! width) so the copy out is `memcpy`'s aligned burst path.

use crate::{Rect, View};

/// Extra pixels a merge may cover beyond its two parts (copying a little
/// more beats a second copy's set-up).
pub const MERGE_SLACK: u64 = 512;

/// Up to `N` changed rectangles inside `bounds`.
pub struct Dirty<const N: usize> {
    bounds: Rect,
    rects: [Rect; N],
    n: usize,
}

impl<const N: usize> Dirty<N> {
    /// Nothing changed yet, inside `bounds` (the screen).
    #[must_use]
    pub const fn new(bounds: Rect) -> Self {
        Self {
            bounds,
            rects: [Rect::new(0, 0, 0, 0); N],
            n: 0,
        }
    }

    /// The changed rectangles (disjoint is not promised; they don't need to
    /// be, copies are idempotent).
    #[must_use]
    pub fn rects(&self) -> &[Rect] {
        &self.rects[..self.n]
    }

    /// Pixels that will be copied.
    #[must_use]
    pub fn area(&self) -> u64 {
        self.rects().iter().map(Rect::area).sum()
    }

    /// Forgets everything (after a flush).
    pub fn clear(&mut self) {
        self.n = 0;
    }

    /// Marks `r` changed (clipped to the bounds, widened to whole words).
    pub fn add(&mut self, r: Rect) {
        let mut r = widen(r.intersect(&self.bounds)).intersect(&self.bounds);
        if r.is_empty() || N == 0 {
            return;
        }
        loop {
            // Absorb every rectangle it should merge with; each merge can
            // bring more into reach, so start over after one.
            let mut i = 0;
            while i < self.n {
                let o = self.rects[i];
                let u = r.union(&o);
                if u.area() <= r.area() + o.area() + MERGE_SLACK || !o.intersect(&r).is_empty() {
                    r = u;
                    self.take(i);
                    i = 0;
                } else {
                    i += 1;
                }
            }
            if self.n < N {
                self.rects[self.n] = r;
                self.n += 1;
                return;
            }
            // Full: merge into the rectangle that grows least, then absorb
            // again with the bigger rectangle.
            let mut best = 0;
            let mut least = u64::MAX;
            for (k, o) in self.rects.iter().enumerate() {
                let grow = o.union(&r).area() - o.area();
                if grow < least {
                    (best, least) = (k, grow);
                }
            }
            r = r.union(&self.rects[best]);
            self.take(best);
        }
    }

    /// Removes rectangle `i` (order doesn't matter).
    fn take(&mut self, i: usize) {
        self.n -= 1;
        self.rects[i] = self.rects[self.n];
    }
}

/// `r` widened to even x and even width (whole 32-bit words of RGB565).
#[must_use]
pub fn widen(r: Rect) -> Rect {
    if r.is_empty() {
        return r;
    }
    let x0 = r.x & !1;
    let x1 = r.right().saturating_add(1) & !1;
    Rect::new(x0, r.y, x1 - x0, r.h)
}

/// Copies `rects` of `src` into the framebuffer bytes `fb` (RGB565, rows
/// `fb_stride` bytes apart, same size as `src`): writes only, never reads
/// `fb`. Rectangles are clipped to both; returns the pixels written.
pub fn flush(src: &View<'_>, fb: &mut [u8], fb_stride: usize, rects: &[Rect]) -> u64 {
    let mut written = 0;
    for r in rects {
        let r = r.intersect(&src.bounds());
        for y in r.y..r.bottom() {
            let at = y as usize * fb_stride + r.x as usize * 2;
            let Some(dst) = fb.get_mut(at..at + r.w as usize * 2) else {
                break;
            };
            let row = &src.row(y)[r.x as usize..r.right() as usize];
            dst.copy_from_slice(rp_hal_mem::pixels_as_bytes(row));
            written += u64::from(r.w);
        }
    }
    written
}
