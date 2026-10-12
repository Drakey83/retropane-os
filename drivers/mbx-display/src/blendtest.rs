//! The plane-blending test scene (`rp-blend`, overnight item 60), shared by
//! the device image and `cargo xtask emu` so both draw and program exactly
//! the same thing.
//!
//! A generated wallpaper (a sky gradient, colour bars, grey steps, fine
//! stripes and a red/blue checker) goes in the **video plane**; a glass
//! panel with opaque text, a dark status bar and an alpha strip go in the
//! **graphics plane** with per-pixel alpha on top. Three stages, each
//! labelled on screen so a photo explains itself:
//!
//! | Stage | Video | Graphics |
//! |---|---|---|
//! | A | packed 4:2:2 (UYVY) | ARGB4444 |
//! | B | planar 4:2:0 | ARGB4444 |
//! | C | planar 4:2:0 | ARGB8888 (to judge ARGB4444's banding) |
//!
//! Everything lives in 2700G memory from 1 MiB up, so the loader's screen
//! at offset 0 is never touched. Drawing builds each row in RAM and only
//! writes the 2700G (the framebuffer is write-only, docs/hw/2700g.md).

use rp_hal_api::RegisterBus;

use crate::layers::{Blend, Layer, Slot, VideoFormat, VideoPlane};
use crate::scanout::rgb_to_yuv;
use crate::{Display, PixelFormat, Plane, font};

/// Screen width.
pub const W: usize = 480;
/// Screen height.
pub const H: usize = 640;

/// One of the three stages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Video 4:2:2, graphics ARGB4444.
    A,
    /// Video 4:2:0, graphics ARGB4444.
    B,
    /// Video 4:2:0, graphics ARGB8888.
    C,
}

impl Stage {
    /// All three, in the order shown.
    pub const ALL: [Self; 3] = [Self::A, Self::B, Self::C];

    /// The stage's letter.
    #[must_use]
    pub const fn letter(self) -> char {
        match self {
            Self::A => 'A',
            Self::B => 'B',
            Self::C => 'C',
        }
    }

    /// The two lines of its on-screen label.
    #[must_use]
    pub const fn label(self) -> [&'static str; 2] {
        match self {
            Self::A => ["A  video 4:2:2", "   UI ARGB4444"],
            Self::B => ["B  video 4:2:0", "   UI ARGB4444"],
            Self::C => ["C  video 4:2:0", "   UI ARGB8888"],
        }
    }

    /// Its graphics and video planes.
    #[must_use]
    pub fn planes(self) -> (Plane, VideoPlane) {
        let gfx = |format: PixelFormat, offset| Plane {
            format,
            width: W as u16,
            height: H as u16,
            offset,
            stride: W * format.bytes(),
        };
        let uyvy = VideoPlane::new(VideoFormat::Uyvy, W as u16, H as u16, 0x50_0000);
        let yuv420 = VideoPlane::new(VideoFormat::Yuv420, W as u16, H as u16, 0x60_0000);
        match self {
            Self::A => (gfx(PixelFormat::Argb4444, 0x10_0000), uyvy),
            Self::B => (gfx(PixelFormat::Argb4444, 0x20_0000), yuv420),
            Self::C => (gfx(PixelFormat::Argb8888, 0x30_0000), yuv420),
        }
    }
}

/// The graphics plane's layer: on top of the wallpaper, per-pixel alpha.
pub const GRAPHICS: Layer = Layer {
    slot: Slot::Middle,
    blend: Blend::PixelAlpha,
};
/// The video plane's layer: the wallpaper, at the bottom, opaque.
pub const VIDEO: Layer = Layer {
    slot: Slot::Bottom,
    blend: Blend::Opaque,
};

/// Programs the display to show `stage` (its planes already drawn).
pub fn program<B: RegisterBus>(d: &mut Display<B>, stage: Stage) {
    let (gfx, video) = stage.planes();
    d.show_video(&video, VIDEO);
    d.set_graphics(&gfx, GRAPHICS);
}

/// The wallpaper's colour at `(x, y)`.
#[must_use]
pub const fn wall(x: usize, y: usize) -> (u8, u8, u8) {
    const BARS: [(u8, u8, u8); 8] = [
        (255, 255, 255),
        (255, 255, 0),
        (0, 255, 255),
        (0, 255, 0),
        (255, 0, 255),
        (255, 0, 0),
        (0, 0, 255),
        (0, 0, 0),
    ];
    if y < 400 {
        // Dusk sky: deep blue at the top to warm orange at the horizon.
        let t = y as u32;
        let r = 20 + t * 210 / 400;
        let g = 40 + t * 100 / 400;
        let b = 110 + (x as u32) * 40 / W as u32 - t * 60 / 400;
        (r as u8, g as u8, b as u8)
    } else if y < 470 {
        BARS[x / 60]
    } else if y < 540 {
        let v = (x / 60 * 255 / 7) as u8;
        (v, v, v)
    } else if x < W / 2 {
        if x.is_multiple_of(2) {
            (255, 255, 255)
        } else {
            (0, 0, 0)
        }
    } else if (x / 7 + y / 7).is_multiple_of(2) {
        // 7-pixel cells: edges fall inside 2-pixel chroma pairs, so 4:2:2
        // and 4:2:0 subsampling show differently.
        (220, 30, 30)
    } else {
        (30, 30, 220)
    }
}

/// The glass panel.
const PANEL: (usize, usize, usize, usize) = (24, 96, 432, 240);
/// The alpha strip: 16 squares, alpha 0 to 255.
const STRIP: (usize, usize, usize, usize) = (24, 350, 27, 36);
const STATUS_H: usize = 36;
const DARK: u32 = 0x0014_1E32;

/// A text line: top-left, scale, colour (ARGB8888), text.
struct Line(usize, usize, usize, u32, &'static str);

fn lines(stage: Stage) -> [Line; 5] {
    let [l1, l2] = stage.label();
    [
        Line(12, 8, 3, 0xFFFF_FFFF, "RetroPane plane test"),
        Line(48, 120, 4, 0xFF00_0000 | DARK, l1),
        Line(48, 160, 4, 0xFF00_0000 | DARK, l2),
        Line(48, 230, 2, 0xFF00_0000 | DARK, "Glass over a photo, mixed"),
        Line(48, 250, 2, 0xFF00_0000 | DARK, "by the display, no CPU."),
    ]
}

fn text_at(lines: &[Line], x: usize, y: usize) -> Option<u32> {
    for &Line(lx, ly, s, color, text) in lines {
        let (cw, ch) = ((font::WIDTH + 1) * s, font::HEIGHT * s);
        if y < ly || y >= ly + ch || x < lx {
            continue;
        }
        let i = (x - lx) / cw;
        let Some(c) = text.chars().nth(i) else {
            continue;
        };
        let (gx, gy) = ((x - lx) % cw / s, (y - ly) / s);
        if gx < font::WIDTH && font::glyph(c)[gy] & (1 << (font::WIDTH - 1 - gx)) != 0 {
            return Some(color);
        }
    }
    None
}

/// The interface's ideal colour at `(x, y)` as ARGB8888 (alpha 0 = clear).
#[must_use]
pub fn ui(stage: Stage, x: usize, y: usize) -> u32 {
    let ls = lines(stage);
    if let Some(c) = text_at(&ls, x, y) {
        return c;
    }
    if y < STATUS_H {
        return 0x9600_0000;
    }
    let (px, py, pw, ph) = PANEL;
    if (px..px + pw).contains(&x) && (py..py + ph).contains(&y) {
        let edge = x < px + 2 || x >= px + pw - 2 || y < py + 2 || y >= py + ph - 2;
        if edge {
            return 0xDCFF_FFFF;
        }
        // Frosted glass: alpha rising left to right, white to pale blue
        // top to bottom (smooth, so ARGB4444's 16 steps show as bands).
        let a = 64 + (x - px) * 96 / pw;
        let t = (y - py) * 255 / ph;
        let (r, g) = (255 - t * 55 / 255, 255 - t * 30 / 255);
        return (a as u32) << 24 | (r as u32) << 16 | (g as u32) << 8 | 0xFF;
    }
    let (sx, sy, sw, sh) = STRIP;
    if (sy..sy + sh).contains(&y) && (sx..sx + 16 * sw).contains(&x) {
        let a = (x - sx) / sw * 17;
        return (a as u32) << 24 | 0x00FF_2828;
    }
    0
}

/// ARGB8888 to ARGB4444, each channel rounded.
#[must_use]
pub const fn to_4444(c: u32) -> u16 {
    const fn q(c: u32, s: u32) -> u32 {
        ((c >> s & 0xFF) * 15 + 127) / 255
    }
    (q(c, 24) << 12 | q(c, 16) << 8 | q(c, 8) << 4 | q(c, 0)) as u16
}

/// What the screen should show at `(x, y)`, ideally: the interface over the
/// wallpaper with 8-bit maths and no format loss. The emulator's exact
/// check uses [`crate::scanout`]; this one catches both sides agreeing on
/// something wrong (sample points, with a tolerance for the formats).
#[must_use]
pub fn intent(stage: Stage, x: usize, y: usize) -> (u8, u8, u8) {
    let (r, g, b) = wall(x, y);
    let c = ui(stage, x, y);
    let k = (c >> 24) as u8;
    let m = crate::scanout::mix;
    (
        m((c >> 16) as u8, r, k),
        m((c >> 8) as u8, g, k),
        m(c as u8, b, k),
    )
}

/// Sample points for [`intent`]: inside flat areas, away from edges where
/// chroma subsampling and text blur colours.
pub const SAMPLES: [(usize, usize); 24] = [
    (240, 60),
    (100, 70),
    (400, 392), // sky
    (30, 435),
    (90, 435),
    (150, 435),
    (210, 435),
    (270, 435),
    (330, 435),
    (390, 435),
    (450, 435), // bars
    (30, 505),
    (150, 505),
    (270, 505),
    (450, 505), // greys
    (40, 300),
    (240, 300),
    (440, 300), // glass
    (37, 368),
    (172, 368),
    (280, 368),
    (442, 368), // strip
    (420, 18),
    (470, 18), // status bar, right of its text
];

/// Draws `stage`'s two planes into the 2700G's memory `vram`, one row at a
/// time from RAM. Stages A and B share no memory with C's graphics, and
/// B and C share the 4:2:0 wallpaper, so all three can be drawn first and
/// switched between instantly.
///
/// # Panics
/// If `vram` is smaller than the planes need (16 MiB covers them).
pub fn draw(stage: Stage, vram: &mut [u8]) {
    let (gfx, video) = stage.planes();
    let mut row = [0u8; W * 4];
    for y in 0..H {
        let line = &mut row[..W * gfx.format.bytes()];
        for x in 0..W {
            let c = ui(stage, x, y);
            match gfx.format {
                PixelFormat::Argb8888 => line[x * 4..][..4].copy_from_slice(&c.to_le_bytes()),
                _ => line[x * 2..][..2].copy_from_slice(&to_4444(c).to_le_bytes()),
            }
        }
        let at = gfx.offset + y * gfx.stride;
        vram[at..at + line.len()].copy_from_slice(line);
    }
    match video.format {
        VideoFormat::Uyvy => {
            for y in 0..H {
                for x in (0..W).step_by(2) {
                    let (y0, u0, v0) = yuv_at(x, y);
                    let (y1, u1, v1) = yuv_at(x + 1, y);
                    let u = (u16::from(u0) + u16::from(u1)).div_ceil(2) as u8;
                    let v = (u16::from(v0) + u16::from(v1)).div_ceil(2) as u8;
                    row[x * 2..][..4].copy_from_slice(&[u, y0, v, y1]);
                }
                let at = video.offset + y * video.stride;
                vram[at..at + W * 2].copy_from_slice(&row[..W * 2]);
            }
        }
        VideoFormat::Yuv420 => {
            for y in (0..H).step_by(2) {
                let (luma, chroma) = row.split_at_mut(W * 2);
                for x in (0..W).step_by(2) {
                    let (mut u, mut v) = (2u16, 2u16); // round to nearest
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let (l, cu, cv) = yuv_at(x + dx, y + dy);
                        luma[dy * W + x + dx] = l;
                        u += u16::from(cu);
                        v += u16::from(cv);
                    }
                    chroma[x / 2] = (u / 4) as u8;
                    chroma[W / 2 + x / 2] = (v / 4) as u8;
                }
                for dy in 0..2 {
                    let at = video.offset + (y + dy) * video.stride;
                    vram[at..at + W].copy_from_slice(&luma[dy * W..][..W]);
                }
                let c = y / 2 * (video.stride / 2);
                vram[video.u_offset + c..][..W / 2].copy_from_slice(&chroma[..W / 2]);
                vram[video.v_offset + c..][..W / 2].copy_from_slice(&chroma[W / 2..W]);
            }
        }
    }
}

fn yuv_at(x: usize, y: usize) -> (u8, u8, u8) {
    let (r, g, b) = wall(x, y);
    rgb_to_yuv(r, g, b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rp_hal_mock::MockBus;

    /// Draws and programs `stage` the way the device does, then computes
    /// the screen with the reference model.
    fn screen(stage: Stage) -> Vec<u8> {
        let mut vram = vec![0u8; 16 << 20];
        let mut d = Display::new(MockBus::new());
        d.enable(
            &crate::X51V_VGA,
            &Plane::fullscreen(&crate::X51V_VGA, PixelFormat::Rgb565),
        );
        draw(stage, &mut vram);
        program(&mut d, stage);
        let bus = d.into_inner();
        let mut out = vec![0u8; W * H * 3];
        crate::scanout::compose(|o| bus.value(o), &vram, &mut out);
        out
    }

    #[test]
    fn every_stage_shows_what_it_means_to() {
        for stage in Stage::ALL {
            let s = screen(stage);
            // ARGB4444 loses up to 8 per channel and alpha step; YUV a few.
            let tol = if stage == Stage::C { 6 } else { 18 };
            for (x, y) in SAMPLES {
                let got = &s[(y * W + x) * 3..][..3];
                let (r, g, b) = intent(stage, x, y);
                for (a, z) in [(r, got[0]), (g, got[1]), (b, got[2])] {
                    assert!(
                        a.abs_diff(z) <= tol,
                        "stage {stage:?} at ({x},{y}): want {r},{g},{b}, got {got:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_stages_differ_where_they_should() {
        let (a, b, c) = (screen(Stage::A), screen(Stage::B), screen(Stage::C));
        assert_ne!(a, b, "labels and video format differ");
        assert_ne!(b, c);
        // 4:2:0 also averages colour across rows: a checker edge between
        // rows 594 and 595 blurs in B, not in A.
        let px = |s: &[u8], x: usize, y: usize| s[(y * W + x) * 3..][..3].to_vec();
        assert_ne!(px(&a, 250, 595), px(&b, 250, 595));
    }

    #[test]
    fn planes_do_not_overlap_each_other_or_the_loader_screen() {
        let mut spans: Vec<(usize, usize)> = Vec::new();
        for stage in Stage::ALL {
            let (g, v) = stage.planes();
            spans.push((g.offset, g.offset + g.stride * H));
            if stage != Stage::C {
                spans.push((v.offset, v.offset + v.bytes()));
            }
        }
        spans.sort_unstable();
        assert!(spans[0].0 >= 1 << 20, "below 1 MiB is the loader's screen");
        for w in spans.windows(2) {
            assert!(w[0].1 <= w[1].0, "{w:?} overlap");
        }
        assert!(spans.last().unwrap().1 <= 16 << 20);
    }

    #[test]
    fn ui_alpha_meanings() {
        assert_eq!(ui(Stage::A, 240, 600) >> 24, 0, "clear over the wallpaper");
        assert_eq!(ui(Stage::A, 24, 360) >> 24, 0, "strip starts clear");
        assert_eq!(ui(Stage::A, 24 + 15 * 27, 360) >> 24, 255, "and ends solid");
        assert_eq!(to_4444(0xFFFF_FFFF), 0xFFFF);
        assert_eq!(to_4444(0x8000_0000) >> 12, 8);
    }
}
