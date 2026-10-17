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
//! | 0-51 | caption: curve, strength, A/B (drawn by the image, not here) |
//! | 52-111 | near-black steps 0, 4 .. 32 |
//! | 112-171 | near-white steps 224, 228 .. 252, 255 |
//! | 172-211 | grey ramp 0-255 |
//! | 212-301 | red, green, blue ramps |
//! | 302-381 | gamma check: a checkerboard (black and white, average 50%) around solid squares |
//! | 382-401 | the squares' labels: the panel gamma each one matches |
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

/// The gamma check's solid levels: each matches the line pairs' average
/// (50% light) on a panel of the gamma under it (255 x 0.5^(1/gamma)).
pub const CHECK_LEVELS: [u8; 6] = [128, 161, 175, 186, 194, 202];
/// The panel gamma each [`CHECK_LEVELS`] square matches.
pub const CHECK_LABELS: [&str; 6] = ["1.0", "1.5", "1.8", "2.2", "2.5", "3.0"];

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

/// The curves the owner flips between.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Curve {
    /// The table as found (Windows Mobile's straight ramp).
    Stock,
    /// Shadows lifted: dark levels raised, black and white kept.
    Shadows,
    /// Highlights spread: the top of the range brought down under the
    /// panel's clip (device run 2: 224-255 looked like one band), the
    /// lower half kept.
    Highlights,
    /// Shadows, then Highlights, at the same strength.
    Both,
    /// An S-curve: more contrast in the middle, black and white kept.
    Contrast,
    /// Warmer white: blue (and a little green) turned down.
    Warm,
    /// Cooler white: red (and a little green) turned down.
    Cool,
}

impl Curve {
    /// In flip order.
    pub const ALL: [Curve; 7] = [
        Self::Stock,
        Self::Shadows,
        Self::Highlights,
        Self::Both,
        Self::Contrast,
        Self::Warm,
        Self::Cool,
    ];

    /// The caption and run log's name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Stock => "Stock",
            Self::Shadows => "Shadows",
            Self::Contrast => "Contrast",
            Self::Warm => "Warm",
            Self::Cool => "Cool",
            Self::Highlights => "Highlights",
            Self::Both => "Shadows+Highl.",
        }
    }

    /// The next curve in flip order (`back`: the one before), wrapping.
    #[must_use]
    pub fn step(self, back: bool) -> Self {
        let i = Self::ALL.iter().position(|&c| c == self).unwrap_or(0);
        let n = Self::ALL.len();
        Self::ALL[if back { (i + n - 1) % n } else { (i + 1) % n }]
    }
}

/// Strength range (D-pad up and down) and where each curve starts.
pub const STRENGTH_MAX: u32 = 10;
/// The strength a curve starts at.
pub const STRENGTH_START: u32 = 5;

/// One level through `curve` at `strength` (0..=[`STRENGTH_MAX`]) on lane
/// `shift` (16 red, 8 green, 0 blue).
#[must_use]
pub fn shape(curve: Curve, strength: u32, v: u32, shift: u32) -> u32 {
    let s = strength.min(STRENGTH_MAX);
    let v = v.min(255);
    let out = match curve {
        Curve::Stock => v,
        // v + k * v(255 - v)/255: at full strength 2v - v^2/255, the lift
        // of a gamma of about 1.6.
        Curve::Shadows => v + s * v * (255 - v) / (255 * STRENGTH_MAX),
        // Linear to 128, then the rest squeezed up to 255 - 5k (205 at
        // full strength): the top steps land below the clip, apart.
        Curve::Highlights => {
            let top = 255 - 5 * s;
            if v <= 128 {
                v
            } else {
                128 + (v - 128) * (top - 128) / 127
            }
        }
        Curve::Both => shape(
            Curve::Highlights,
            s,
            shape(Curve::Shadows, s, v, shift),
            shift,
        ),
        // Toward smoothstep: 3t^2 - 2t^3.
        Curve::Contrast => {
            let sm = v * v * (765 - 2 * v) / (255 * 255);
            (v * (STRENGTH_MAX - s) + sm * s) / STRENGTH_MAX
        }
        // At full strength blue to 77%, green to 92%.
        Curve::Warm => match shift {
            0 => v * (256 - 6 * s) / 256,
            8 => v * (256 - 2 * s) / 256,
            _ => v,
        },
        Curve::Cool => match shift {
            16 => v * (256 - 6 * s) / 256,
            8 => v * (256 - 2 * s) / 256,
            _ => v,
        },
    };
    out.min(255)
}

/// The gamma table for `curve` at `strength`: each lane of `found` (the
/// table as the image found it) put through [`shape`], so Stock is exactly
/// `found`.
#[must_use]
pub fn table(curve: Curve, strength: u32, found: &[u32; GAMMA_ENTRIES]) -> [u32; GAMMA_ENTRIES] {
    core::array::from_fn(|i| {
        [16, 8, 0]
            .iter()
            .map(|&sh| shape(curve, strength, found[i] >> sh & 0xFF, sh) << sh)
            .fold(0, |a, l| a | l)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rp_drv_mbx_display::scanout::WM_GAMMA;

    #[test]
    fn stock_is_the_table_as_found() {
        assert_eq!(table(Curve::Stock, 7, &WM_GAMMA), WM_GAMMA);
        for c in Curve::ALL {
            assert_eq!(table(c, 0, &WM_GAMMA), WM_GAMMA, "{c:?} at strength 0");
        }
    }

    #[test]
    fn curves_keep_black_and_never_fall() {
        for c in Curve::ALL {
            for s in 0..=STRENGTH_MAX {
                for sh in [16, 8, 0] {
                    assert_eq!(shape(c, s, 0, sh), 0);
                    let mut last = 0;
                    for v in 0..=255 {
                        let o = shape(c, s, v, sh);
                        assert!(o >= last, "{c:?} {s} falls at {v}");
                        last = o;
                    }
                }
            }
        }
    }

    #[test]
    fn curves_do_what_their_names_say() {
        assert!(shape(Curve::Shadows, 10, 32, 8) > 55);
        assert_eq!(shape(Curve::Shadows, 10, 255, 8), 255);
        assert!(shape(Curve::Contrast, 10, 64, 8) < 50);
        assert!(shape(Curve::Contrast, 10, 192, 8) > 205);
        assert!(shape(Curve::Warm, 10, 255, 0) < shape(Curve::Warm, 10, 255, 16));
        assert!(shape(Curve::Cool, 10, 255, 16) < shape(Curve::Cool, 10, 255, 0));
        // Highlights: the near-white steps (4 apart) stay apart, under 240.
        assert_eq!(shape(Curve::Highlights, 10, 100, 8), 100);
        assert!(shape(Curve::Highlights, 5, 255, 8) <= 230);
        assert!(shape(Curve::Highlights, 10, 228, 8) > shape(Curve::Highlights, 10, 224, 8));
        assert!(shape(Curve::Both, 5, 16, 8) > 16);
        assert!(shape(Curve::Both, 5, 255, 8) < 255);
    }

    #[test]
    fn flip_order_wraps_both_ways() {
        assert_eq!(Curve::Cool.step(false), Curve::Stock);
        assert_eq!(Curve::Stock.step(false), Curve::Shadows);
        assert_eq!(Curve::Shadows.step(false), Curve::Highlights);
        assert_eq!(Curve::Stock.step(true), Curve::Cool);
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
        // Most of the scene (outside the flame and the torch's pool) dark.
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
            assert!(lit > 20, "label {col}: {lit} pixels");
        }
    }
}
