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

/// The dials, each 1-50 with **25 = unchanged** for every one (run 5: in
/// run 4 the dials started at different values and a reset to the wrong
/// one darkened the rest of the run). Big, visible changes per click.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dials {
    /// Black end. Above 25 black is lifted so the near-black squares come
    /// apart (50: black at 100 of 255); below, the darkest levels are
    /// pushed into black (deeper shadows, 1: the bottom 48 levels black).
    pub black: u32,
    /// White end. Above 25 white comes down under the panel's clip while
    /// the top 64 levels keep their spacing (50: white at 105); below, the
    /// top is pushed into white (1: the top 48 levels white).
    pub white: u32,
    /// Middle: darker (below 25) or brighter (above) mid-tones, black and
    /// white kept; the gamma check's question.
    pub mid: u32,
    /// White balance: cooler (below 25: red down) or warmer (above: blue
    /// down), green a little down either way; judged on the grey ramp.
    pub warm: u32,
    /// The backlight: 25 as found, 50 full, 1 a twenty-fifth of the found
    /// level (the image sets PWM0; not part of the table).
    pub bright: u32,
    /// Saturation: 25 as drawn, 1 grey, 50 double. The gamma table can't
    /// mix colours, so the image redraws the picture: a preview of what the
    /// OS would apply to pictures and game output.
    pub sat: u32,
}

/// The dials' range.
pub const DIAL_MIN: u32 = 1;
/// The dials' range.
pub const DIAL_MAX: u32 = 50;
/// Every dial's "unchanged" setting.
pub const NEUTRAL: u32 = 25;
/// How many dials.
pub const DIALS: usize = 6;
/// The dials' names, in order.
pub const DIAL_NAMES: [&str; DIALS] = ["Black", "White", "Mid", "Warm", "Bright", "Sat"];

impl Dials {
    /// Every dial unchanged: the table and backlight as found, the picture
    /// as drawn.
    pub const STOCK: Self = Self {
        black: NEUTRAL,
        white: NEUTRAL,
        mid: NEUTRAL,
        warm: NEUTRAL,
        bright: NEUTRAL,
        sat: NEUTRAL,
    };

    /// Dial `i`, in [`DIAL_NAMES`] order.
    #[must_use]
    pub const fn get(&self, i: usize) -> u32 {
        match i {
            0 => self.black,
            1 => self.white,
            2 => self.mid,
            3 => self.warm,
            4 => self.bright,
            _ => self.sat,
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
            4 => self.bright = v,
            _ => self.sat = v,
        }
        self
    }
}

/// The ready-made looks the last step flips between, made from the
/// owner's run 5 picks (Black 28, White 26, Warm 22, Bright 35, Sat 30;
/// the gamma check: box 4, standard gamma 2.2, at Mid 14, and box 2 at
/// 25, so the panel shows mid-tones like gamma 1.5). Mid is a measurement
/// in step 3, not a pick: the looks set it themselves.
pub const PRESETS: [(&str, Dials); 4] = [
    ("Original", Dials::STOCK),
    (
        "Accurate",
        Dials {
            black: 28,
            white: 26,
            mid: 14,
            warm: 22,
            bright: 35,
            sat: 30,
        },
    ),
    (
        "Everyday",
        Dials {
            black: 28,
            white: 26,
            mid: 25,
            warm: 22,
            bright: 35,
            sat: 30,
        },
    ),
    (
        "Dark games",
        Dials {
            black: 31,
            white: 26,
            mid: 38,
            warm: 22,
            bright: 45,
            sat: 32,
        },
    ),
];

/// The look the last step starts on.
pub const PRESET_START: usize = 2;

/// Gamma tables measured with the Spyder4 (`cargo xtask measure`, run 3,
/// 2026-10-17, readings `local/screen-measure/readings-1792276147.csv`):
/// gamma 2.2 between the panel's own black and white, white at D65, half-way
/// (about 9,000 K), or as it is. Shown at full backlight (the panel is dim:
/// 41 cd/m2 at Windows Mobile's 63%, 57 at full).
pub const MEASURED: [(&str, [u32; GAMMA_ENTRIES]); 3] = [
    (
        "Measured full",
        [
            0x000000, 0x030401, 0x0F1206, 0x1C210F, 0x2A3118, 0x384121, 0x465029, 0x546133,
            0x63703D, 0x708146, 0x7F904F, 0x8DA259, 0x9DB463, 0xACC66B, 0xBDD774, 0xCCE87F,
            0xDBFF87,
        ],
    ),
    (
        "Measured half",
        [
            0x000000, 0x040403, 0x0F120C, 0x1D2119, 0x2B3125, 0x394132, 0x47503F, 0x56614B,
            0x657058, 0x738166, 0x829072, 0x90A27F, 0xA0B48C, 0xB0C69A, 0xC1D7A8, 0xD0E8B7,
            0xE0FFC5,
        ],
    ),
    (
        "Measured gamma",
        [
            0x000000, 0x040404, 0x121212, 0x212121, 0x313131, 0x414141, 0x505050, 0x616161,
            0x707070, 0x818181, 0x909090, 0xA2A2A2, 0xB4B4B4, 0xC6C6C6, 0xD7D7D7, 0xE8E8E8,
            0xFFFFFF,
        ],
    ),
];

/// How many looks the last step flips between: the presets, then the
/// measured tables.
pub const LOOKS: usize = PRESETS.len() + MEASURED.len();

/// Look `i`'s name.
#[must_use]
pub fn look_name(i: usize) -> &'static str {
    match i.checked_sub(PRESETS.len()) {
        None => PRESETS[i].0,
        Some(m) => MEASURED[m % MEASURED.len()].0,
    }
}

/// Look `i`'s dials (a measured look: unchanged but the backlight full).
#[must_use]
pub fn look_dials(i: usize) -> Dials {
    match i.checked_sub(PRESETS.len()) {
        None => PRESETS[i].1,
        Some(_) => Dials {
            bright: DIAL_MAX,
            ..Dials::STOCK
        },
    }
}

/// Look `i`'s gamma table, `found` being the table as found.
#[must_use]
pub fn look_table(i: usize, found: &[u32; GAMMA_ENTRIES]) -> [u32; GAMMA_ENTRIES] {
    match i.checked_sub(PRESETS.len()) {
        None => table(PRESETS[i].1, found),
        Some(m) => MEASURED[m % MEASURED.len()].1,
    }
}

/// A dial's distance from unchanged: -24 to +25.
fn off(v: u32) -> i32 {
    v.clamp(DIAL_MIN, DIAL_MAX) as i32 - NEUTRAL as i32
}

/// One level (0-255) of lane `shift` (16 red, 8 green, 0 blue) through the
/// table's dials: black, middle, white, then warmth.
#[must_use]
pub fn shape(d: Dials, v: u32, shift: u32) -> u32 {
    let v = v.min(255) as i32;
    // Black: lifted to `4b` above 25; below, the bottom `2|b|` levels
    // pushed into black and the rest stretched back to 255.
    let b = off(d.black);
    let v = if b >= 0 {
        4 * b + v * (255 - 4 * b) / 255
    } else {
        let cut = -2 * b;
        (v - cut).max(0) * 255 / (255 - cut)
    };
    // Middle: v + m v(255 - v)/255, m from -24/25 to +1.
    let m = off(d.mid);
    let v = v + m * v * (255 - v) / (255 * 25);
    // White: above 25 down to `top`, the top 64 levels kept 4 apart and
    // everything under them squeezed; below, the top `2|w|` levels pushed
    // into white.
    let w = off(d.white);
    const SPAN: i32 = 64;
    let v = if w >= 0 {
        let top = 255 - 6 * w;
        if v >= 255 - SPAN {
            top - (255 - v)
        } else {
            v * (top - SPAN) / (255 - SPAN)
        }
    } else {
        (v * 255 / (255 + 2 * w)).min(255)
    };
    // Warmth: at the ends one colour at 61%, green at 90%.
    let t = off(d.warm);
    let keep = match shift {
        16 if t < 0 => 256 + 4 * t,
        0 if t > 0 => 256 - 4 * t,
        8 => 256 - t.abs(),
        _ => 256,
    };
    (v.clamp(0, 255) * keep / 256).clamp(0, 255) as u32
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

/// The backlight's duty for the Bright dial, of `counts` per cycle, from
/// `found` (the duty as found, of `counts`): exactly `found` at 25, full at
/// 50, down to a twenty-fifth of `found` at 1 (never below 1).
#[must_use]
pub fn backlight(bright: u32, found: u32, counts: u32) -> u32 {
    let found = found.min(counts);
    let b = off(bright);
    if b >= 0 {
        found + (counts - found) * b as u32 / 25
    } else {
        (found * (25 - b.unsigned_abs()) / 25).max(1)
    }
}

/// The picture's colour at `(x, y)` with saturation dial `sat` (25: as
/// [`pixel`]; 1: grey; 50: twice as far from grey).
#[must_use]
pub fn pixel_sat(x: u32, y: u32, sat: u32) -> (u8, u8, u8) {
    let (r, g, b) = pixel(x, y);
    let k = sat.clamp(DIAL_MIN, DIAL_MAX) as i32;
    if k == NEUTRAL as i32 {
        return (r, g, b);
    }
    let (r, g, b) = (i32::from(r), i32::from(g), i32::from(b));
    let y = (77 * r + 150 * g + 29 * b) >> 8;
    // 1 to 25: grey to as drawn; 25 to 50: as drawn to double.
    let (num, den) = if k <= NEUTRAL as i32 {
        (k - 1, NEUTRAL as i32 - 1)
    } else {
        (k, NEUTRAL as i32)
    };
    let c = |v: i32| (y + (v - y) * num / den).clamp(0, 255) as u8;
    (c(r), c(g), c(b))
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
                        ..Dials::STOCK
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
        // Black: lifted a lot at 50, a visible step per click; deeper below.
        assert!(at(50, 25, 25, 0) >= 95);
        assert!(at(26, 25, 25, 0) >= 4);
        assert_eq!(at(1, 25, 25, 40), 0);
        // White: well down at 50; the top squares keep 4 apart; clipped below.
        assert!(at(25, 50, 25, 255) <= 110);
        assert_eq!(at(25, 35, 25, 255) - at(25, 35, 25, 251), 4);
        assert_eq!(at(25, 1, 25, 215), 255);
        // Middle: darker below 25, brighter above, ends kept.
        assert!(at(25, 25, 1, 128) < 70);
        assert!(at(25, 25, 50, 128) > 185);
        assert_eq!(at(25, 25, 1, 255), 255);
        assert_eq!(at(25, 25, 50, 0), 0);
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
    fn backlight_is_as_found_at_25_and_full_at_50() {
        assert_eq!(backlight(25, 274, 432), 274);
        assert_eq!(backlight(50, 274, 432), 432);
        assert!(backlight(1, 274, 432) < 15);
        assert_eq!(backlight(1, 0, 432), 1);
        for b in 1..50 {
            assert!(backlight(b + 1, 274, 432) > backlight(b, 274, 432));
        }
    }

    #[test]
    fn saturation_keeps_greys_and_scales_colour() {
        let (x, y) = (0, BARS_Y.0 + 5); // white bar
        assert_eq!(pixel_sat(x, y, 50), (255, 255, 255));
        let red = (W * 5 / 8 + 5, BARS_Y.0 + 5);
        assert_eq!(pixel_sat(red.0, red.1, 25), (255, 0, 0));
        let (r, g, _) = pixel_sat(red.0, red.1, 1);
        assert_eq!(r, g);
        let (_, g, _) = pixel_sat(red.0, red.1, 13);
        assert!(g > 20);
    }

    #[test]
    fn measured_tables_rise_and_start_at_black() {
        for (name, t) in &MEASURED {
            assert_eq!(t[0], 0, "{name}");
            for sh in [16, 8, 0] {
                assert!(
                    t.windows(2)
                        .all(|w| (w[1] >> sh & 0xFF) >= (w[0] >> sh & 0xFF)),
                    "{name}"
                );
            }
        }
        assert_eq!(look_name(PRESETS.len()), "Measured full");
        assert_eq!(look_dials(LOOKS - 1).bright, DIAL_MAX);
    }

    #[test]
    fn dials_stay_in_range() {
        let d = Dials::STOCK.moved(0, -50).moved(1, 100).moved(2, 3);
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
