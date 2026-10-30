//! A list's scroll as the pen moves it (the owner, 2026-10-30: "i cant drag
//! any list up and down"; pull-to-scroll and the rebound): it follows the
//! pen 1:1, keeps going after a flick and slows down, and past either end
//! gives way (a rubber band) then springs back.
//!
//! Fixed point throughout (the PXA270 has no FPU): positions in 1/256 px,
//! speeds in 1/256 px a millisecond, stepped a millisecond at a time, so the
//! same pen gives the same movement on the device, in the preview and in
//! the tests.
//!
//! Iterate, don't imitate (rule 0): WM5 lists moved a row per D-pad press
//! or by a scroll bar, with no momentum; iOS and Android fling with an
//! exponential slow-down and spring back past the ends (the textbook values
//! are a starting point, not the answer). Ours: the same feel in integers,
//! the speed measured over the pen's last 80 ms (its samples come ~7 ms
//! apart on the X51v), and a critically damped spring so a rebound never
//! wobbles. The constants are first drafts to tune on the device (R124's
//! run records the owner's own flicks).

/// One pixel.
const PX: i32 = 256;
/// The speed kept per millisecond of a fling, in 1/4096 (0.998: iOS's
/// "normal" deceleration, a first draft).
const KEEP: i32 = 4088;
/// A fling slower than this stops (1/256 px a ms: 0.02 px/ms).
const STOP: i32 = 5;
/// Pen samples older than this don't count towards the speed.
const SPEED_WINDOW_MS: u32 = 80;
/// The spring past an end: stiffness and damping per ms, critically damped
/// (`c = 2 sqrt(k)`), settling in ~250 ms: k = 0.0004, c = 0.04, both in
/// 1/10,000.
const SPRING_K: i32 = 4;
const SPRING_C: i32 = 400;
/// How far past an end a pull may show, as a share of the pull (1/3) and at
/// most this many pixels.
const BAND_MAX_PX: i32 = 120;

/// One list's scroll.
#[derive(Clone, Copy, Debug)]
pub struct Scroll {
    pos: i32,
    max: i32,
    vel: i32,
    /// The pen's last y while it drags.
    pen: Option<i32>,
    /// Where the pen has pulled the content, unbanded (1/256 px), while
    /// dragging: `pos` is this with the band applied past an end.
    raw: i32,
    /// Recent pen samples (ms, y in px), newest last.
    samples: [(u32, i32); 6],
    n: usize,
    last_ms: u32,
}

impl Scroll {
    /// At the top, able to scroll `max_px` (0: content that fits, which
    /// still gives way and springs back).
    #[must_use]
    pub const fn new(max_px: u16) -> Self {
        Self {
            pos: 0,
            max: max_px as i32 * PX,
            vel: 0,
            pen: None,
            raw: 0,
            samples: [(0, 0); 6],
            n: 0,
            last_ms: 0,
        }
    }

    /// How far the content is scrolled, in pixels (negative: pulled down
    /// past the top; past `max`: pulled up past the bottom).
    #[must_use]
    pub const fn offset(&self) -> i32 {
        (self.pos + PX / 2).div_euclid(PX)
    }

    /// The furthest it scrolls, in pixels.
    #[must_use]
    pub const fn max(&self) -> i32 {
        self.max / PX
    }

    /// Whether it is moving by itself (a fling or a rebound).
    #[must_use]
    pub const fn moving(&self) -> bool {
        self.pen.is_none() && (self.vel != 0 || self.pos < 0 || self.pos > self.max)
    }

    /// Whether the pen holds it.
    #[must_use]
    pub const fn held(&self) -> bool {
        self.pen.is_some()
    }

    /// Jumps to `px` (clamped), stopping any movement (the D-pad's rows).
    pub fn set(&mut self, px: i32) {
        self.pos = (px * PX).clamp(0, self.max);
        self.vel = 0;
    }

    /// The pen takes hold at `y`: a fling stops where it is (as on a
    /// phone, a touch catches a moving list).
    pub fn press(&mut self, y: u16, now: u32) {
        self.vel = 0;
        self.pen = Some(i32::from(y));
        self.raw = self.pos + self.overshoot() * 2;
        self.samples[0] = (now, i32::from(y));
        self.n = 1;
        self.last_ms = now;
    }

    /// Past an end now, in 1/256 px (signed; 0 inside).
    const fn overshoot(&self) -> i32 {
        if self.pos < 0 {
            self.pos
        } else if self.pos > self.max {
            self.pos - self.max
        } else {
            0
        }
    }

    /// The pen moved to `y`: the content follows 1:1 inside, a third as
    /// far past an end (and no further than [`BAND_MAX_PX`]).
    pub fn drag(&mut self, y: u16, now: u32) {
        let Some(last) = self.pen else { return };
        let y = i32::from(y);
        let d = (last - y) * PX;
        self.pen = Some(y);
        if self.n == self.samples.len() {
            self.samples.copy_within(1.., 0);
            self.n -= 1;
        }
        self.samples[self.n] = (now, y);
        self.n += 1;
        self.last_ms = now;
        self.raw += d;
        let band = BAND_MAX_PX * PX;
        self.pos = if self.raw < 0 {
            (self.raw / 3).max(-band)
        } else if self.raw > self.max {
            (self.max + (self.raw - self.max) / 3).min(self.max + band)
        } else {
            self.raw
        };
    }

    /// The pen lifted: the speed of its last 80 ms carries on (or the
    /// spring takes it back inside).
    pub fn release(&mut self, now: u32) {
        if self.pen.take().is_none() {
            return;
        }
        let recent = &self.samples[..self.n];
        let first = recent
            .iter()
            .find(|(t, _)| now.wrapping_sub(*t) <= SPEED_WINDOW_MS)
            .copied();
        let last = recent.last().copied();
        self.vel = match (first, last) {
            (Some((t0, y0)), Some((t1, y1))) if t1 != t0 && now.wrapping_sub(t1) < 40 => {
                (y0 - y1) * PX / t1.wrapping_sub(t0) as i32
            }
            _ => 0,
        };
        // Only away from an end it is pulled past: towards it the spring
        // does the work.
        if self.overshoot() != 0 {
            self.vel = 0;
        }
        self.last_ms = now;
    }

    /// Moves it on to `now` (ms); true if it moved.
    pub fn step(&mut self, now: u32) -> bool {
        let dt = now.wrapping_sub(self.last_ms).min(100);
        self.last_ms = now;
        if !self.moving() {
            return false;
        }
        let before = self.offset();
        for _ in 0..dt {
            let x = self.overshoot();
            if x == 0 {
                self.vel = self.vel * KEEP / 4096;
                if self.vel.abs() < STOP {
                    self.vel = 0;
                }
            } else {
                // Spring: a = -k x - c v.
                let a = (-SPRING_K * x - SPRING_C * self.vel) / 10_000;
                self.vel += if a == 0 { -x.signum() } else { a };
            }
            self.pos += self.vel;
            // Back inside from past an end: it stops at the end.
            let x2 = self.overshoot();
            if x != 0 && (x2 == 0 || x2.signum() != x.signum()) {
                self.pos = if x < 0 { 0 } else { self.max };
                self.vel = 0;
            }
            if self.vel == 0 && self.overshoot() == 0 {
                break;
            }
        }
        self.offset() != before || !self.moving()
    }
}
