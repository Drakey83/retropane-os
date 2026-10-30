//! The radar's sample loop (build-ui-foundation.md step 6, until the
//! network brings real frames): rain cells drawn once at start into small
//! ARGB pictures, moved with the wind across 13 frames from an hour ago to
//! an hour ahead, over our own base map (tools/mockups/radar_basemaps.py).
//!
//! Iterate, don't imitate (rule 0): WM5 had no radar; phone apps stream a
//! full picture per frame. Ours makes the sample's rain on the device from
//! a few integers (no picture bytes in the image), draws it with the
//! mock-ups' five rain steps, and moves it as whole cells, so a loop frame
//! costs only the cells' own rectangles. Real frames (PNG, the same
//! steps) replace the cells when the network exists.

/// Frames in the loop: an hour back to an hour ahead, every 10 minutes.
pub const FRAMES: u8 = 13;
/// The frame that is "now".
pub const NOW_FRAME: u8 = 6;
/// Minutes between frames.
pub const STEP_MIN: i16 = 10;

/// The rain steps, light to heavy: colour and the threshold (of 256) at
/// which each starts (the mock-ups' scale).
pub const STEPS: [(u32, u32); 5] = [
    (0x3C_BE50, 64),
    (0x1E_963C, 115),
    (0xEB_D73C, 179),
    (0xF0_8C28, 230),
    (0xDC_3232, 250),
];
/// The rain's opacity (of 255), the map showing through.
pub const ALPHA: u32 = 175;

/// The rain cells' pictures: (width, height).
pub const CELLS: [(u16, u16); 3] = [(168, 104), (120, 76), (96, 60)];

/// Pixels of every cell picture together.
pub const CELL_PX: usize = 168 * 104 + 120 * 76 + 96 * 60;

/// A cheap hash of a point (the cells' ragged texture).
const fn hash(x: u32, y: u32, seed: u32) -> u32 {
    let mut h = x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA77) ^ seed;
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h
}

/// Value noise, 0-255, on a grid of `cell` pixels (bilinear between the
/// grid's hashed corners).
fn noise(x: u32, y: u32, cell: u32, seed: u32) -> u32 {
    let (gx, gy) = (x / cell, y / cell);
    let (fx, fy) = (x % cell, y % cell);
    let c = |i, j| hash(gx + i, gy + j, seed) & 0xFF;
    let top = c(0, 0) * (cell - fx) + c(1, 0) * fx;
    let bot = c(0, 1) * (cell - fx) + c(1, 1) * fx;
    (top * (cell - fy) + bot * fy) / (cell * cell)
}

/// Draws rain cell `k` into `px` (its [`CELLS`] size, row by row): a
/// rounded core falling off to its edge ((1 - r^2)^2), roughened by noise,
/// in the five steps; clear outside.
pub fn cell(px: &mut [u32], k: usize) {
    let Some(&(w, h)) = CELLS.get(k) else { return };
    let (w, h) = (u32::from(w), u32::from(h));
    if px.len() < (w * h) as usize {
        return;
    }
    let seed = 0x5EED_0000 + k as u32;
    for y in 0..h {
        for x in 0..w {
            // Distance from the centre, squared, in 1/1024 of the ellipse.
            let dx = (2 * x + 1) as i32 - w as i32;
            let dy = (2 * y + 1) as i32 - h as i32;
            let r2 = (dx * dx * 1024 / (w * w) as i32 + dy * dy * 1024 / (h * h) as i32) as u32;
            let core = if r2 >= 1024 {
                0
            } else {
                let f = 1024 - r2;
                f * f / 1024 * 256 / 1024
            };
            // Rough edges and patches: noise moves the level both ways.
            let n = noise(x, y, 12, seed) + noise(x, y, 5, seed ^ 0xABCD) / 2;
            let v = (core * (128 + n) / 256).min(255);
            let colour = STEPS.iter().rev().find(|(_, t)| v >= *t).map(|(c, _)| *c);
            px[(y * w + x) as usize] = match colour {
                Some(c) => ALPHA << 24 | c,
                None => 0,
            };
        }
    }
}

/// One rain cell's path: which picture, where it is at "now" (its top
/// left), and how far it moves a frame (px; the wind from the
/// south-west).
#[derive(Clone, Copy, Debug)]
pub struct Track {
    /// Index into [`CELLS`].
    pub cell: u8,
    /// Its place at "now".
    pub at: (i16, i16),
    /// Its move per frame.
    pub step: (i16, i16),
}

/// The sample's cells (screen coordinates, the page's map under them):
/// the big one over the Olympics drifting onto Seattle.
pub const TRACKS: [Track; 4] = [
    Track {
        cell: 0,
        at: (60, 210),
        step: (9, -3),
    },
    Track {
        cell: 1,
        at: (300, 130),
        step: (7, -4),
    },
    Track {
        cell: 2,
        at: (40, 400),
        step: (8, -5),
    },
    Track {
        cell: 2,
        at: (250, 330),
        step: (10, -2),
    },
];

impl Track {
    /// Its top left at `frame` (may be off the page: the caller clips).
    #[must_use]
    pub const fn at(&self, frame: u8) -> (i16, i16) {
        let k = frame as i16 - NOW_FRAME as i16;
        (self.at.0 + self.step.0 * k, self.at.1 + self.step.1 * k)
    }
}
