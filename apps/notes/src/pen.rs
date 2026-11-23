//! The pen's points filtered as they come, and a width for each from the
//! pen's speed (notes-rule0.md, "Smooth ink").
//!
//! Iterate, don't imitate (rule 0): Windows Mobile drew the touch
//! controller's raw points as one-pixel lines (jagged, every bit of the
//! resistive panel's noise in the stroke); the textbook fix, a moving
//! average, trades the noise for a fixed lag that is worst exactly when the
//! pen moves fast. Modern pens (Casiez et al., the "1 Euro" filter, CHI
//! 2012) low-pass with a cutoff that rises with speed: slow strokes lose
//! their jitter, fast ones lose no ground. Ours is that filter in integer
//! arithmetic (sixteenths of a pixel, alphas in 1/256: no FPU on the
//! PXA270), its constants chosen by a sweep over the measurement in
//! this crate's tests (a circle written at 1.5-12 px a sample with a +-2 px
//! noise band: a slow pen's jitter halved, 12 to 6 tenths of a pixel; no
//! lag from 6 px a sample, a little over 2 px at the slowest), and the stroke's last
//! point snapped to where the pen lifted, so a filter's lag never shortens
//! what was written. The TSC2046 has no pressure, so the width comes from
//! speed the way a fountain pen's ink does: slow is wide, fast is thin,
//! smoothed so a width never jumps. Device-side tuning against the X51v's
//! own noise is reminder R-D1 (docs/reminders.md).

/// The smoothing at rest, in 1/256 (0.16: a still pen's jitter averaged
/// over about six samples).
pub const ALPHA_REST: i32 = 40;
/// How fast the cutoff rises with speed: alpha grows by this, in 1/256,
/// for each sixteenth of a pixel a sample the pen moves (from about 7 px a
/// sample the filter lets every point through). Chosen by sweeping 168
/// pairs over the tests' noisy circle (2026-11-23): this pair more than
/// halves a slow pen's jitter with no lag at writing speed.
pub const BETA: i32 = 30;

/// The filter's state for one stroke.
#[derive(Clone, Copy, Debug, Default)]
pub struct Filter {
    /// The filtered point, in 1/16 px.
    x: i32,
    y: i32,
    /// The last raw point, in 1/16 px.
    rx: i32,
    ry: i32,
    /// The pen's motion (1/16 px a sample, signed, so the panel's noise
    /// cancels), itself smoothed over four samples.
    vx: i32,
    vy: i32,
    /// The width last given (four bits, `width_scale`), in 1/16.
    w: i32,
    started: bool,
}

impl Filter {
    /// A stroke begins at (`x`, `y`): that point as it is, its width the
    /// slow pen's.
    pub fn start(&mut self, x: i16, y: i16) -> ((i16, i16), u8) {
        let (x, y) = (i32::from(x) * 16, i32::from(y) * 16);
        *self = Self {
            x,
            y,
            rx: x,
            ry: y,
            vx: 0,
            vy: 0,
            w: 11 * 16,
            started: true,
        };
        ((self.x_px(), self.y_px()), 11)
    }

    /// The next raw point: the filtered point and its width.
    pub fn step(&mut self, x: i16, y: i16) -> ((i16, i16), u8) {
        if !self.started {
            return self.start(x, y);
        }
        let (rx, ry) = (i32::from(x) * 16, i32::from(y) * 16);
        let (dx, dy) = (rx - self.rx, ry - self.ry);
        self.rx = rx;
        self.ry = ry;
        // The motion's own low pass (the 1 Euro filter's derivative
        // cutoff), signed: a still pen's noise averages away instead of
        // reading as speed.
        self.vx += (dx - self.vx) / 4;
        self.vy += (dy - self.vy) / 4;
        let v = self.vx.abs() + self.vy.abs();
        let a = (ALPHA_REST + v * BETA / 16).min(256);
        self.x += (rx - self.x) * a / 256;
        self.y += (ry - self.y) * a / 256;
        // Width from speed: 11 (1.25x) at 2 px a sample or slower, 4
        // (0.67x) at 24 px or faster; a quarter of the way a sample.
        let target = (11 * 16 - (v - 32).clamp(0, 352) * 7 / 22).clamp(4 * 16, 11 * 16);
        self.w += (target - self.w) / 4;
        ((self.x_px(), self.y_px()), (self.w / 16) as u8)
    }

    /// The pen lifted at (`x`, `y`): that point exactly, the width kept.
    pub fn lift(&mut self, x: i16, y: i16) -> ((i16, i16), u8) {
        self.x = i32::from(x) * 16;
        self.y = i32::from(y) * 16;
        ((x, y), (self.w / 16) as u8)
    }

    fn x_px(&self) -> i16 {
        ((self.x + 8) >> 4) as i16
    }

    fn y_px(&self) -> i16 {
        ((self.y + 8) >> 4) as i16
    }
}
