//! The display calibration test image's picture and curves (step C1 of
//! docs/design/display-calibration.md), kept apart from the image so the
//! emulator check builds its reference from the same code.
//!
//! The picture is fixed (drawn once, 32 bpp); only the 2700G's graphics
//! gamma table (`GFXGAM0`-`16`, 17 points per colour) changes, so every
//! curve is seen over exactly the same pixels. Integer arithmetic only (the
//! no-float rule).
//!
//! | Rows | Band |
//! |---|---|
//! | 0-51 | caption: the three dials, A/B (drawn by the image, not here) |
//! | 52-111 | near-black steps 0, 4 .. 32 |
//! | 112-171 | near-white steps 224, 228 .. 252, 255 |
//! | 172-211 | grey ramp 0-255 |
//! | 212-301 | red, green, blue ramps |
//! | 302-381 | gamma check: a checkerboard (black and white, average 50%) around solid squares |
//! | 382-401 | the squares' labels: box numbers 1-6 (panel gamma 1.0-3.0) |
//! | 402-441 | colour bars |
//! | 442-639 | a dark synthetic game scene (a corridor, a torch, a status bar) |

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use rp_drv_mbx_display::font;
use rp_drv_mbx_display::pattern::BARS;
use rp_drv_mbx_display::scanout::GAMMA_ENTRIES;

/// Screen size.
pub const W: u32 = 480;
/// Screen height.
pub const H: u32 = 640;
/// The caption band's height: the picture starts below it.
pub const CAPTION_H: u32 = 52;

const BLACKS: (u32, u32) = (52, 112);
const WHITES: (u32, u32) = (112, 172);
const GREY: (u32, u32) = (172, 212);
const RGB: (u32, u32) = (212, 302);
const CHECK: (u32, u32) = (302, 382);
const LABELS: (u32, u32) = (382, 402);
const BARS_Y: (u32, u32) = (402, 442);
/// The game scene's first row.
pub const SCENE_Y: u32 = 442;

/// Near-black step levels.
pub const BLACK_STEPS: [u8; 9] = [0, 4, 8, 12, 16, 20, 24, 28, 32];
/// Near-white step levels.
pub const WHITE_STEPS: [u8; 9] = [224, 228, 232, 236, 240, 244, 248, 252, 255];

/// The gamma check's solid levels: each matches the checkerboard's
/// average (50% light) on a panel of the gamma in [`CHECK_GAMMA`]
/// (255 x 0.5^(1/gamma)).
pub const CHECK_LEVELS: [u8; 6] = [128, 161, 175, 186, 194, 202];
/// The squares' labels: box numbers the owner reports.
pub const CHECK_LABELS: [&str; 6] = ["1", "2", "3", "4", "5", "6"];
/// The panel gamma each box matches, for the docs and the log.
pub const CHECK_GAMMA: [&str; 6] = ["1.0", "1.5", "1.8", "2.2", "2.5", "3.0"];

/// The picture's colour at `(x, y)`, `y` at or below [`CAPTION_H`]
/// (black above it).
#[must_use]
pub fn pixel(x: u32, y: u32) -> (u8, u8, u8) {
    let grey = |v: u8| (v, v, v);
    let step = |levels: &[u8; 9]| grey(levels[((x * 9 / W) as usize).min(8)]);
    let ramp = (x * 255 / (W - 1)) as u8;
    match y {
        y if y < CAPTION_H => (0, 0, 0),
        y if y < BLACKS.1 => step(&BLACK_STEPS),
        y if y < WHITES.1 => step(&WHITE_STEPS),
        y if y < GREY.1 => grey(ramp),
        y if y < RGB.1 => match (y - RGB.0) / 30 {
            0 => (ramp, 0, 0),
            1 => (0, ramp, 0),
            _ => (0, 0, ramp),
        },
        y if y < CHECK.1 => check(x, y),
        y if y < LABELS.1 => label(x, y),
        y if y < BARS_Y.1 => BARS[(x * 8 / W) as usize],
        y if y < H => scene(x, y - SCENE_Y),
        _ => (0, 0, 0),
    }
}

/// Six columns of a one-pixel black and white checkerboard, a solid square
/// in each. (Device run 2: alternating lines didn't blend into a flat grey
/// on the X51v's panel; a checkerboard has no row structure to see.)
fn check(x: u32, y: u32) -> (u8, u8, u8) {
    let (col, cx) = (x / 80, x % 80);
    let cy = y - CHECK.0;
    if (20..60).contains(&cx) && (20..60).contains(&cy) {
        let v = CHECK_LEVELS[col as usize];
        return (v, v, v);
    }
    if (x + y).is_multiple_of(2) {
        (255, 255, 255)
    } else {
        (0, 0, 0)
    }
}

/// The squares' labels, grey on black, centred under each column at twice
/// the font's size.
fn label(x: u32, y: u32) -> (u8, u8, u8) {
    const S: u32 = 2;
    let (col, cx) = ((x / 80) as usize, x % 80);
    let text = CHECK_LABELS[col].as_bytes();
    let cell = (font::WIDTH as u32 + 1) * S;
    let width = cell * text.len() as u32 - S;
    let (x0, y0) = ((80 - width) / 2, 3);
    let gy = (y - LABELS.0).wrapping_sub(y0) / S;
    let lit = cx >= x0 && cx < x0 + width && (gy as usize) < font::HEIGHT && {
        let i = ((cx - x0) / cell) as usize;
        let gx = ((cx - x0) % cell) / S;
        gx < font::WIDTH as u32
            && font::glyph(char::from(text[i]))[gy as usize] & (1 << (font::WIDTH as u32 - 1 - gx))
                != 0
    };
    if lit { (200, 200, 200) } else { (0, 0, 0) }
}

/// A dark corridor drawn from arithmetic (no game data): stone walls in
/// brick courses, a tiled floor, a ceiling, a torch's warm glow on the left
/// wall, darkness at the far end, and a status bar. Its levels sit mostly
/// between 4 and 60: the range dark games live in.
fn scene(x: u32, sy: u32) -> (u8, u8, u8) {
    let scene_h = H - SCENE_Y;
    if sy >= scene_h - 20 {
        return status_bar(x, sy - (scene_h - 20));
    }
    let (cx, cy) = (x as i32 - 240, sy as i32 - 90);
    let (ax, ay) = (cx.abs().max(1), cy.abs().max(1));
    // Depth (1/16ths of a unit) and the surface's own coordinates.
    let (z, base, tex) = if ax > 2 * ay {
        let z = 64 * 240 / ax;
        let v = (cy * z / 64 + 4096) as u32;
        let u = z as u32;
        // Brick courses 12 high, bricks 32 long, every other course offset.
        let course = v / 12;
        let u = u + if course % 2 == 1 { 16 } else { 0 };
        let mortar = v.is_multiple_of(12) || u.is_multiple_of(32);
        let grain = (u * 7 + v * 13) % 5;
        let (r, g, b) = if mortar { (22, 18, 14) } else { (58, 40, 28) };
        (z, (r + grain as i32, g + grain as i32, b), true)
    } else {
        let z = 64 * 120 / ay;
        let u = (cx * z / 64 + 4096) as u32;
        let w = z as u32;
        let tile = (u / 24 + w / 24).is_multiple_of(2);
        let joint = u.is_multiple_of(24) || w.is_multiple_of(24);
        let c = if cy > 0 {
            if joint {
                (12, 14, 12)
            } else if tile {
                (40, 44, 36)
            } else {
                (30, 34, 28)
            }
        } else {
            (26, 24, 22)
        };
        (z, c, false)
    };
    // Light falls off with depth; the far end goes to black.
    let fall = |c: i32| (c * 96 / (z + 32)).clamp(0, 255);
    let (mut r, mut g, mut b) = (fall(base.0), fall(base.1), fall(base.2));
    // The torch on the left wall, at (70, 70): a warm pool of light.
    if tex || cy < 0 {
        let (dx, dy) = (x as i32 - 70, sy as i32 - 70);
        let d2 = dx * dx + dy * dy;
        let glow = 70 * 900 / (900 + d2);
        r += glow;
        g += glow * 5 / 8;
        b += glow / 4;
    }
    // The flame itself.
    let (fx, fy) = (x as i32 - 70, sy as i32 - 66);
    if fx.abs() <= 3 && (-6..=4).contains(&fy) {
        return (255, 190, 90);
    }
    (r.min(255) as u8, g.min(255) as u8, b.min(255) as u8)
}

/// A game's status bar: dark grey with dim red numbers.
fn status_bar(x: u32, y: u32) -> (u8, u8, u8) {
    const S: u32 = 2;
    let text = b"100%   50   12";
    let cell = (font::WIDTH as u32 + 1) * S;
    let (x0, y0) = (12, 3);
    let gy = y.wrapping_sub(y0) / S;
    let i = (x.wrapping_sub(x0) / cell) as usize;
    let gx = (x.wrapping_sub(x0) % cell) / S;
    let lit = x >= x0
        && i < text.len()
        && (gy as usize) < font::HEIGHT
        && gx < font::WIDTH as u32
        && font::glyph(char::from(text[i]))[gy as usize] & (1 << (font::WIDTH as u32 - 1 - gx))
            != 0;
    if lit {
        (150, 24, 16)
    } else if y == 0 {
        (60, 60, 60)
    } else {
        (32, 32, 32)
    }
}

/// The dials, each 1-50 (the owner's request for run 4: big, visible
/// changes per click, one dial per question).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dials {
    /// Black end: lifts black so the near-black squares come apart (1:
    /// none, 50: black raised by 98 of 255).
    pub black: u32,
    /// White end: brings white down under the panel's clip while keeping
    /// the top 64 levels at full spacing, so the near-white squares come
    /// apart (1: none, 50: white at 108 of 255).
    pub white: u32,
    /// Middle: darker (below 25) or brighter (above) mid-tones, black and
    /// white kept; the gamma check's question.
    pub mid: u32,
    /// White balance: cooler (below 25: red down) or warmer (above: blue
    /// down), green a little down either way; judged on the grey ramp.
    pub warm: u32,
    /// The backlight, in 50ths of full (not part of the table: the image
    /// sets PWM0, starting from the setting found).
    pub bright: u32,
}

/// The dials' range.
pub const DIAL_MIN: u32 = 1;
/// The dials' range.
pub const DIAL_MAX: u32 = 50;
/// The middle and warm dials' neutral setting.
pub const MID_NEUTRAL: u32 = 25;
/// How many dials.
pub const DIALS: usize = 5;
/// The dials' names, in order.
pub const DIAL_NAMES: [&str; DIALS] = ["Black", "White", "Mid", "Warm", "Bright"];

impl Dials {
    /// No change: the table as found, the backlight full.
    pub const STOCK: Self = Self {
        black: DIAL_MIN,
        white: DIAL_MIN,
        mid: MID_NEUTRAL,
        warm: MID_NEUTRAL,
        bright: DIAL_MAX,
    };

    /// Dial `i`, in [`DIAL_NAMES`] order.
    #[must_use]
    pub const fn get(&self, i: usize) -> u32 {
        match i {
            0 => self.black,
            1 => self.white,
            2 => self.mid,
            3 => self.warm,
            _ => self.bright,
        }
    }

    /// Dial `i` moved by `by`, kept in range.
    #[must_use]
    pub fn moved(mut self, i: usize, by: i32) -> Self {
        let v = (self.get(i) as i32 + by).clamp(DIAL_MIN as i32, DIAL_MAX as i32) as u32;
        match i {
            0 => self.black = v,
            1 => self.white = v,
            2 => self.mid = v,
            3 => self.warm = v,
            _ => self.bright = v,
        }
        self
    }
}

/// One level (0-255) of lane `shift` (16 red, 8 green, 0 blue) through the
/// dials: black, middle, white, then warmth.
#[must_use]
pub fn shape(d: Dials, v: u32, shift: u32) -> u32 {
    let v = v.min(255);
    // Black: raised to `off`, white kept.
    let off = 2 * (d.black.clamp(DIAL_MIN, DIAL_MAX) - 1);
    let v = off + v * (255 - off) / 255;
    // Middle: v + m v(255 - v)/255, m from -24/25 to +1.
    let m = d.mid.clamp(DIAL_MIN, DIAL_MAX) as i32 - MID_NEUTRAL as i32;
    let vi = v as i32;
    let v = (vi + m * vi * (255 - vi) / (255 * 25)) as u32;
    // White: down to `top`; the top 64 levels keep their spacing (one
    // panel level per 4), everything under them squeezed.
    let top = 255 - 3 * (d.white.clamp(DIAL_MIN, DIAL_MAX) - 1);
    const SPAN: u32 = 64;
    let v = if v >= 255 - SPAN {
        top - (255 - v)
    } else {
        v * (top - SPAN) / (255 - SPAN)
    };
    // Warmth: at the ends one colour at 61%, green at 90%.
    let w = d.warm.clamp(DIAL_MIN, DIAL_MAX) as i32 - MID_NEUTRAL as i32;
    let keep = match shift {
        16 if w < 0 => 256 + 4 * w,
        0 if w > 0 => 256 - 4 * w,
        8 => 256 - w.abs(),
        _ => 256,
    };
    (v.min(255) * keep as u32 / 256).min(255)
}

/// The gamma table for `d`: each lane of `found` (the table as the image
/// found it) put through [`shape`], so [`Dials::STOCK`] is exactly `found`.
#[must_use]
pub fn table(d: Dials, found: &[u32; GAMMA_ENTRIES]) -> [u32; GAMMA_ENTRIES] {
    core::array::from_fn(|i| {
        [16, 8, 0]
            .iter()
            .map(|&sh| shape(d, found[i] >> sh & 0xFF, sh) << sh)
            .fold(0, |a, l| a | l)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rp_drv_mbx_display::scanout::WM_GAMMA;

    fn all() -> impl Iterator<Item = Dials> {
        let steps = [1, 2, 10, 24, 25, 26, 40, 49, 50];
        steps.into_iter().flat_map(move |black| {
            steps.into_iter().flat_map(move |white| {
                steps.into_iter().flat_map(move |mid| {
                    [1, 25, 50].into_iter().map(move |warm| Dials {
                        black,
                        white,
                        mid,
                        warm,
                        bright: 50,
                    })
                })
            })
        })
    }

    #[test]
    fn stock_is_the_table_as_found() {
        assert_eq!(table(Dials::STOCK, &WM_GAMMA), WM_GAMMA);
        for v in 0..=255 {
            for sh in [16, 8, 0] {
                assert_eq!(shape(Dials::STOCK, v, sh), v);
            }
        }
    }

    #[test]
    fn every_setting_rises_and_stays_in_range() {
        for d in all() {
            for sh in [16, 8, 0] {
                let mut last = 0;
                for v in 0..=255 {
                    let o = shape(d, v, sh);
                    assert!(o <= 255);
                    assert!(o >= last, "{d:?} falls at {v}");
                    last = o;
                }
            }
        }
    }

    #[test]
    fn each_dial_does_its_job_drastically() {
        let at = |black, white, mid, v| {
            let d = Dials {
                black,
                white,
                mid,
                ..Dials::STOCK
            };
            shape(d, v, 8)
        };
        // Black: lifted a lot at 50, a visible step per click.
        assert!(at(50, 1, 25, 0) >= 90);
        assert!(at(2, 1, 25, 0) >= 2);
        // White: well down at 50; the top squares keep 4 apart.
        assert!(at(1, 50, 25, 255) <= 110);
        assert_eq!(at(1, 30, 25, 255) - at(1, 30, 25, 251), 4);
        // Middle: darker below 25, brighter above, ends kept.
        assert!(at(1, 1, 1, 128) < 70);
        assert!(at(1, 1, 50, 128) > 185);
        assert_eq!(at(1, 1, 1, 255), 255);
        assert_eq!(at(1, 1, 50, 0), 0);
        // Warmth: blue down when warm, red down when cool.
        let warm = |warm, sh| {
            shape(
                Dials {
                    warm,
                    ..Dials::STOCK
                },
                255,
                sh,
            )
        };
        assert!(warm(50, 0) < 160 && warm(50, 16) == 255);
        assert!(warm(1, 16) < 165 && warm(1, 0) == 255);
    }

    #[test]
    fn dials_stay_in_range() {
        let d = Dials::STOCK.moved(0, -5).moved(1, 100).moved(2, 3);
        assert_eq!((d.black, d.white, d.mid), (1, 50, 28));
    }

    #[test]
    fn the_scene_is_dark_and_the_bands_are_where_the_table_says() {
        assert_eq!(pixel(0, BLACKS.0), (0, 0, 0));
        assert_eq!(pixel(W - 1, BLACKS.0), (32, 32, 32));
        assert_eq!(pixel(W - 1, WHITES.0), (255, 255, 255));
        assert_eq!(pixel(W - 1, GREY.0), (255, 255, 255));
        assert_eq!(pixel(W - 1, RGB.0 + 30), (0, 255, 0));
        assert_eq!(pixel(40, CHECK.0 + 40), (128, 128, 128));
        assert_eq!(pixel(0, BARS_Y.0), (255, 255, 255));
        let (mut dark, mut all) = (0, 0);
        for y in SCENE_Y..H - 20 {
            for x in (0..W).step_by(3) {
                let (r, g, b) = pixel(x, y);
                all += 1;
                if r.max(g).max(b) <= 64 {
                    dark += 1;
                }
            }
        }
        assert!(dark * 10 >= all * 8, "{dark} of {all} dark");
    }

    #[test]
    fn every_label_shows() {
        for col in 0..6 {
            let lit = (col * 80..col * 80 + 80)
                .flat_map(|x| (LABELS.0..LABELS.1).map(move |y| (x, y)))
                .filter(|&(x, y)| pixel(x, y) != (0, 0, 0))
                .count();
            assert!(lit > 8, "label {col}: {lit} pixels");
        }
    }
}
