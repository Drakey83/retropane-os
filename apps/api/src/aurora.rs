//! The window backdrop: light behind the glass (design-language.md
//! version 3, "Materials").
//!
//! The owner, 2026-11-14: an app's window "needs to be more fancy then
//! just a plain white or grey background, look into gradience and aero and
//! glass, windows vista, XP, retro modern design".
//!
//! - **Windows Vista** shipped Aurora as a 1920 x 1440 painted picture and
//!   put glass over it; **XP's Luna** used flat blue chrome over a photo;
//!   **Windows Mobile** a flat system colour. A designer's file per app and
//!   look would be ten 600 KB pictures in the image.
//! - **Ours** computes it: a field of light from the app's tint alone,
//!   worked out on a coarse grid (one node per 8 x 8 pixels, the picture has
//!   nothing finer in it) in integers, then spread across the pixels with a
//!   bilinear blend and an ordered dither down to the panel's 16 bits, so
//!   the gradients don't band. A tall falling light (bright under the title
//!   bar, deep at the softkeys), three sweeping ribbons (Aurora's) placed by
//!   the tint's own bits so each app's sky is its own, a glow from the top
//!   left where the Aero light comes from. No file, 4,941 nodes of work
//!   when an app opens, and any theme colour gets its own backdrop.

use crate::Argb;

/// The picture's size (the screen's).
pub const W: usize = 480;
/// The picture's height.
pub const H: usize = 640;
/// A node every this many pixels.
const CELL: usize = 8;
const GW: usize = W / CELL + 1;
const GH: usize = H / CELL + 1;

/// The 4 x 4 ordered dither the screen's own conversion uses (the 3D
/// core's matrix, `rp_mbx_render::DITHER_3D`'s order), 0-15.
const DITHER: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

/// One channel of `a` toward `b`, `k` of 256.
const fn lerp(a: i32, b: i32, k: i32) -> i32 {
    a + (b - a) * k / 256
}

/// A colour's channels.
const fn rgb(c: Argb) -> [i32; 3] {
    [
        ((c >> 16) & 0xFF) as i32,
        ((c >> 8) & 0xFF) as i32,
        (c & 0xFF) as i32,
    ]
}

/// A soft bell around 0, 256 at the centre: `w^4 / (w^2 + d^2)^2`, no
/// table and no square root (a Lorentzian squared: Gaussian enough for
/// light, and exact in integers).
fn bell(d: i32, w: i32) -> i32 {
    // 32 bits hold it (no 64-bit divide on the X51v's CPU: slow, and its
    // routine was most of the painter's size): distances are clamped to
    // 2,047, widths are under 512, so w^2 * 256 is under 2^26 and the sum
    // under 2^23.
    let d = d.clamp(-2047, 2047);
    let (d2, w2) = (d * d, w * w);
    let q = w2 * 256 / (w2 + d2);
    q * q / 256
}

/// The light at node (`gx`, `gy`) for `tint` in the dark or `light` look,
/// each channel 0-255 in 8.4 fixed point (x16).
fn node(gx: usize, gy: usize, tint: Argb, light: bool) -> [i32; 3] {
    // The accents are muted for marks on glass (55% chroma); light behind
    // glass wants the colour back: each channel pushed from the tint's
    // grey by half again.
    let t = {
        let m = rgb(tint);
        let grey = (m[0] + m[1] + m[2]) / 3;
        [
            (m[0] + (m[0] - grey) * 3 / 4).clamp(0, 255),
            (m[1] + (m[1] - grey) * 3 / 4).clamp(0, 255),
            (m[2] + (m[2] - grey) * 3 / 4).clamp(0, 255),
        ]
    };
    let (x, y) = ((gx * CELL) as i32, (gy * CELL) as i32);
    // The tint's bits place the ribbons: each app its own sky.
    let seed = (tint ^ (tint >> 7) ^ (tint >> 13)) as i32;
    let s = |k: i32, m: i32| (seed >> k).rem_euclid(m);
    // Falling light: the top lit, the bottom deep (dark look) or the top
    // clear and the bottom tinted (light look).
    let (top, mid, bottom): ([i32; 3], [i32; 3], [i32; 3]) = if light {
        (
            [252, 253, 255],
            [
                lerp(238, t[0], 26),
                lerp(243, t[1], 26),
                lerp(250, t[2], 26),
            ],
            [
                lerp(206, t[0], 70),
                lerp(216, t[1], 70),
                lerp(230, t[2], 70),
            ],
        )
    } else {
        (
            [
                lerp(20, t[0], 104),
                lerp(26, t[1], 104),
                lerp(38, t[2], 104),
            ],
            [lerp(10, t[0], 44), lerp(13, t[1], 44), lerp(20, t[2], 44)],
            [3, 4, 7],
        )
    };
    let ky = y * 256 / H as i32;
    let mut c = [0i32; 3];
    for (k, ch) in c.iter_mut().enumerate() {
        *ch = if ky < 110 {
            lerp(top[k], mid[k], ky * 256 / 110)
        } else {
            lerp(mid[k], bottom[k], (ky - 110) * 256 / 146)
        } * 16;
    }
    // The ribbons: three arcs sweeping across, each a bell around its
    // curve, wide and faint.
    // Fanning up from the lower left to the upper right, as Aurora's do:
    // (where it starts on the left, its rise across, its bow, its glow's
    // width, its strength).
    let ribbons: [(i32, i32, i32, i32, i32); 3] = [
        (230 + s(0, 50), -150 - s(3, 40), 70, 58, 120),
        (410 + s(5, 40), -170 - s(9, 30), 110, 44, 150),
        (590 + s(2, 30), -150 - s(11, 40), 150, 34, 110),
    ];
    for (y0, slope, bow, wide, strength) in ribbons {
        let u = x * 256 / W as i32;
        let line = y0 + slope * u / 256 + bow * u / 256 * (256 - u) / 256;
        // A wide glow with a bright thin core inside it, brighter toward
        // the right where the ribbon rises into the light.
        let rise = 96 + u * 160 / 256;
        let b = (bell(y - line, wide) * strength / 256
            + bell(y - line, wide / 6 + 2) * strength / 300)
            * rise
            / 256;
        for (k, ch) in c.iter_mut().enumerate() {
            let to = if light {
                lerp(t[k], 255, 30)
            } else {
                lerp(t[k], 255, 70)
            } * 16;
            let amount = (if light { b * 96 / 256 } else { b * 124 / 256 }).min(256);
            *ch = lerp(*ch, to, amount);
        }
    }
    // The Aero light, from the top left; a vignette at the foot in the dark.
    let (dx, dy) = (x + 40, y + 60);
    let glow = bell(dx * 3 / 4, 260) * bell(dy, 250) / 256;
    for ch in &mut c {
        *ch = lerp(*ch, 255 * 16, glow * if light { 150 } else { 34 } / 256);
    }
    if !light {
        // The foot's corners fall away into the dark (a vignette).
        let centre = bell(x - W as i32 / 2, 300);
        let fall = (256 - centre) * y / H as i32;
        for ch in &mut c {
            *ch = lerp(*ch, *ch * 5 / 10, fall);
        }
    }
    c
}

/// Paints the backdrop for `tint` into `out` (`W` x `H` RGB565, row by
/// row), for the dark or the `light` look.
pub fn paint(out: &mut [u16], tint: Argb, light: bool) {
    rows(tint, light, |y, row| {
        if let Some(to) = out.get_mut(y * W..(y + 1) * W) {
            to.copy_from_slice(row);
        }
    });
}

/// [`paint`] a row at a time, top to bottom, each given to `emit` with its
/// number: the X51v writes them straight into the renderer's picture and
/// never holds the picture in RAM (600 KB).
pub fn rows(tint: Argb, light: bool, mut emit: impl FnMut(usize, &[u16; W])) {
    // Two rows of nodes at a time: nothing kept but them.
    let mut row = [0u16; W];
    let mut above = [[0i32; 3]; GW];
    let mut below = [[0i32; 3]; GW];
    for (gx, n) in above.iter_mut().enumerate() {
        *n = node(gx, 0, tint, light);
    }
    for gy in 0..GH - 1 {
        for (gx, n) in below.iter_mut().enumerate() {
            *n = node(gx, gy + 1, tint, light);
        }
        for cy in 0..CELL {
            let y = gy * CELL + cy;
            // Down the cell first: this row's own nodes (x CELL, exact),
            // GW blends a row instead of one a pixel.
            let mut here = [[0i32; 3]; GW];
            for (h, (a, b)) in here.iter_mut().zip(above.iter().zip(&below)) {
                for k in 0..3 {
                    h[k] = a[k] * (CELL - cy) as i32 + b[k] * cy as i32;
                }
            }
            // Across each cell the colour moves by a constant step: one
            // add a channel a pixel, no multiply and no divide (the X51v
            // paints this when a window opens: 307,200 pixels, and the
            // first version's nine blends a pixel took a frame's budget).
            // Values are x16 (the nodes) x CELL (down) x CELL (across):
            // shifted by 10, the dither's threshold added below the bits
            // kept (8 levels lost for red and blue, 4 for green).
            let dither = &DITHER[y & 3];
            for gx in 0..GW - 1 {
                let (a, b) = (here[gx], here[gx + 1]);
                let step = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                let mut v = [a[0] * CELL as i32, a[1] * CELL as i32, a[2] * CELL as i32];
                for (cx, px) in row[gx * CELL..(gx + 1) * CELL].iter_mut().enumerate() {
                    let d = i32::from(dither[cx & 3]);
                    let q =
                        |v: i32, lost: i32| ((v + (d << (lost + 6))) >> 10).clamp(0, 255) >> lost;
                    *px = ((q(v[0], 3) << 11) | (q(v[1], 2) << 5) | q(v[2], 3)) as u16;
                    v[0] += step[0];
                    v[1] += step[1];
                    v[2] += step[2];
                }
            }
            emit(y, &row);
        }
        above = below;
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec;

    fn lum(p: u16) -> i32 {
        let (r, g, b) = (
            i32::from(p >> 11) << 3,
            i32::from((p >> 5) & 0x3F) << 2,
            i32::from(p & 0x1F) << 3,
        );
        (r * 2 + g * 5 + b) / 8
    }

    #[test]
    fn the_dark_backdrop_is_lit_at_the_top_and_deep_at_the_foot() {
        let mut px = vec![0u16; W * H];
        paint(&mut px, 0xFFD9_907E, false);
        let mean = |y0: usize| {
            px[y0 * W..(y0 + 40) * W]
                .iter()
                .map(|&p| lum(p))
                .sum::<i32>()
                / (40 * W) as i32
        };
        assert!(mean(40) > mean(560) + 12, "{} {}", mean(40), mean(560));
        // Dark enough for white text on glass anywhere (the look's 4.5:1).
        assert!(px.iter().all(|&p| lum(p) < 120));
    }

    #[test]
    fn the_light_backdrop_stays_light_everywhere() {
        let mut px = vec![0u16; W * H];
        paint(&mut px, 0xFF74_B1A7, true);
        assert!(px.iter().all(|&p| lum(p) > 150));
    }

    #[test]
    fn neighbours_never_jump_so_gradients_do_not_band() {
        // Dithered 16-bit: a pixel and its neighbour differ by at most the
        // dither's step plus one level of slope, in every channel.
        for (tint, light) in [(0xFF74_9DD3u32, false), (0xFFD2_B37B, true)] {
            let mut px = vec![0u16; W * H];
            paint(&mut px, tint, light);
            for y in 0..H {
                for x in 1..W {
                    let (a, b) = (px[y * W + x - 1], px[y * W + x]);
                    let step = |sh: u32, m: u16| {
                        (i32::from((a >> sh) & m) - i32::from((b >> sh) & m)).abs()
                    };
                    assert!(
                        step(11, 31) <= 2 && step(5, 63) <= 2 && step(0, 31) <= 2,
                        "({x}, {y})"
                    );
                }
            }
        }
    }

    /// `RP_AURORA_DUMP=<dir>`: writes each app's backdrop in both looks as
    /// PPM files there (for the eye).
    #[test]
    fn dump_for_the_eye() {
        let Some(dir) = std::env::var("RP_AURORA_DUMP")
            .ok()
            .filter(|d| !d.is_empty())
        else {
            return;
        };
        for (name, tint) in [
            ("calendar", crate::kit::tint::CALENDAR),
            ("contacts", crate::kit::tint::CONTACTS),
            ("tasks", crate::kit::tint::TASKS),
            ("notes", crate::kit::tint::NOTES),
            ("calculator", crate::kit::tint::CALCULATOR),
            ("weather", crate::kit::tint::WEATHER),
        ] {
            for light in [false, true] {
                let mut px = vec![0u16; W * H];
                paint(&mut px, tint, light);
                let mut out = std::format!(
                    "P6
{W} {H}
255
"
                )
                .into_bytes();
                for p in px {
                    out.extend_from_slice(&[
                        ((p >> 11) << 3) as u8,
                        (((p >> 5) & 0x3F) << 2) as u8,
                        ((p & 0x1F) << 3) as u8,
                    ]);
                }
                let look = if light { "light" } else { "dark" };
                std::fs::write(std::format!("{dir}/aurora-{name}-{look}.ppm"), out).unwrap();
            }
        }
    }

    #[test]
    fn each_tint_has_its_own_sky_and_the_same_tint_the_same() {
        let mut a = vec![0u16; W * H];
        let mut b = vec![0u16; W * H];
        paint(&mut a, 0xFFD9_907E, false);
        paint(&mut b, 0xFF74_B1A7, false);
        assert_ne!(a, b);
        paint(&mut b, 0xFFD9_907E, false);
        assert_eq!(a, b);
    }
}
