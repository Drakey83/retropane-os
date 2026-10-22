//! RetroPane's default picture (the owner's decision, 2026-10-22,
//! docs/design/display-calibration.md): the S-curve at strength 1.75 and
//! saturation 1.2x, both in the picture, before the dither (on this 6-bit
//! panel a curve after the dither, in the hardware gamma table, bands).
//! Windows Mobile's own gamma table stays in the hardware as found, so the
//! result is what the owner judged: the curve composed with that table.
//!
//! Applied where pictures enter the system (a wallpaper as it loads, icons,
//! a theme's colours as they're built), never per frame: blends of two
//! curved colours then differ slightly from the curve of the blend (the
//! curve isn't linear), the price of costing nothing per frame.
//!
//! The owner judged the two separately (saturation in display-calibration
//! run 5, Sat 30; the curve in looks strength run 4): together they are
//! first seen on the device in `rp-today`.

/// Saturation as a fraction: colours move `SAT_NUM / SAT_DEN` as far from
/// their grey (display-cal's dial 30 of 25: 1.2x).
pub const SAT_NUM: i32 = 30;
/// See [`SAT_NUM`].
pub const SAT_DEN: i32 = 25;

/// The S-curve at strength 1.75 for every 8-bit value: each half of the
/// range to the power 1.75, meeting at the middle (`boot/looks`' formula;
/// a host test rebuilds it).
pub const CURVE: [u8; 256] = [
    0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 2, 2, 2, 3, 3, 3, 4, 4, 5, 5, 5, 6, 6, 7, 7, 8, 8, 9, 10, 10,
    11, 11, 12, 13, 13, 14, 15, 15, 16, 17, 18, 18, 19, 20, 21, 21, 22, 23, 24, 25, 26, 27, 27, 28,
    29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 44, 45, 46, 47, 48, 49, 50, 52, 53, 54,
    55, 56, 58, 59, 60, 61, 63, 64, 65, 67, 68, 69, 71, 72, 73, 75, 76, 78, 79, 80, 82, 83, 85, 86,
    88, 89, 91, 92, 94, 95, 97, 98, 100, 102, 103, 105, 106, 108, 110, 111, 113, 115, 116, 118,
    120, 121, 123, 125, 127, 128, 130, 132, 134, 135, 137, 139, 140, 142, 144, 145, 147, 149, 150,
    152, 153, 155, 157, 158, 160, 161, 163, 164, 166, 167, 169, 170, 172, 173, 175, 176, 177, 179,
    180, 182, 183, 184, 186, 187, 188, 190, 191, 192, 194, 195, 196, 197, 199, 200, 201, 202, 203,
    205, 206, 207, 208, 209, 210, 211, 213, 214, 215, 216, 217, 218, 219, 220, 221, 222, 223, 224,
    225, 226, 227, 228, 228, 229, 230, 231, 232, 233, 234, 234, 235, 236, 237, 237, 238, 239, 240,
    240, 241, 242, 242, 243, 244, 244, 245, 245, 246, 247, 247, 248, 248, 249, 249, 250, 250, 250,
    251, 251, 252, 252, 252, 253, 253, 253, 254, 254, 254, 254, 254, 255, 255, 255, 255, 255, 255,
];

/// One colour in the default picture: saturation (around its grey, luma
/// 77 R + 150 G + 29 B over 256, as display-cal's dial), then the curve
/// on each channel.
#[must_use]
pub const fn apply(r: u8, g: u8, b: u8) -> (u8, u8, u8) {
    let (r, g, b) = (r as i32, g as i32, b as i32);
    let y = (77 * r + 150 * g + 29 * b) >> 8;
    (CURVE[sat(y, r)], CURVE[sat(y, g)], CURVE[sat(y, b)])
}

/// One channel `v` moved from its grey `y` by the saturation, clamped.
const fn sat(y: i32, v: i32) -> usize {
    let s = y + (v - y) * SAT_NUM / SAT_DEN;
    if s < 0 {
        0
    } else if s > 255 {
        255
    } else {
        s as usize
    }
}

/// An ARGB colour (not premultiplied) in the default picture, alpha kept:
/// for colours fixed at build time (a theme's), `const`. For a
/// translucent colour this is the look of the colour, not of the blend it
/// makes (the module notes).
#[must_use]
pub const fn argb(c: u32) -> u32 {
    let (r, g, b) = apply((c >> 16) as u8, (c >> 8) as u8, c as u8);
    c & 0xFF00_0000 | (r as u32) << 16 | (g as u32) << 8 | b as u32
}

/// Whether the default picture is on: `RP_PICTURE_ORIGINAL` at build time
/// shows Windows Mobile's picture instead (the Original look, for a
/// side-by-side check).
pub const ON: bool = option_env!("RP_PICTURE_ORIGINAL").is_none();

/// [`argb`] when the default picture is [`ON`], else `c` as given.
#[must_use]
pub const fn theme(c: u32) -> u32 {
    if ON { argb(c) } else { c }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_curve_is_the_formula_at_1_75() {
        for (i, &c) in CURVE.iter().enumerate() {
            let x = i as f64 / 255.0;
            let y = if x < 0.5 {
                0.5 * (2.0 * x).powf(1.75)
            } else {
                1.0 - 0.5 * (2.0 * (1.0 - x)).powf(1.75)
            };
            assert_eq!(c, (255.0 * y).round() as u8, "at {i}");
        }
        assert!(CURVE.windows(2).all(|w| w[1] >= w[0]), "rises");
        assert_eq!((CURVE[0], CURVE[128], CURVE[255]), (0, 128, 255));
    }

    #[test]
    fn greys_only_take_the_curve_and_colours_move_away_from_grey() {
        for v in [0u8, 40, 128, 200, 255] {
            let c = CURVE[usize::from(v)];
            assert_eq!(apply(v, v, v), (c, c, c));
        }
        // A muted red gets redder: its distance from grey grows 1.2x
        // before the curve.
        let (r, g, b) = apply(180, 100, 100);
        assert!(r > CURVE[180] && g < CURVE[100] && b < CURVE[100]);
        // Saturation clamps, never wraps.
        assert_eq!(apply(255, 0, 0).0, 255);
        assert_eq!(apply(0, 255, 255).0, 0);
    }

    #[test]
    fn argb_keeps_alpha_and_matches_apply() {
        let (r, g, b) = apply(0x4C, 0x54, 0x5C);
        assert_eq!(
            argb(0x8C4C_545C),
            0x8C00_0000 | u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b)
        );
        assert_eq!(argb(0xFFFF_FFFF), 0xFFFF_FFFF);
        assert_eq!(argb(0x5000_0000), 0x5000_0000);
    }
}
