//! The plane-blending test scene (`rp-blend`, overnight item 60), shared by
//! the device image and `cargo xtask emu` so both draw and program exactly
//! the same thing.
//!
//! A generated wallpaper (a sky gradient, colour bars, grey steps, fine
//! stripes and a red/blue checker) goes in the **video plane**; a glass
//! panel with opaque text, a dark status bar and an alpha strip go in the
//! **graphics plane** with per-pixel alpha on top. The **top half** of each
//! colour bar and grey step is drawn opaque by the graphics plane (plain
//! RGB), the bottom half is the video plane: on the device, any difference
//! between the halves is the chip's colour conversion, whatever the camera
//! does. Four stages, each labelled on screen so a photo explains itself:
//!
//! | Stage | Video | Graphics |
//! |---|---|---|
//! | A | packed 4:2:2 (UYVY) | ARGB4444 |
//! | B | planar 4:2:0 | ARGB4444 |
//! | C | planar 4:2:0 | ARGB8888 (to judge ARGB4444's banding) |
//! | D | as C, video gamma off | ARGB8888 |
//! | E | planar 4:2:0 | ARGB4444, ordered dither (C's smoothness at A's size?) |
//! | F | planar 4:2:0 | as E, drawn the way the OS will: an `rp-gfx` layer in RAM, dirty flushes |
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

/// One of the stages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Video 4:2:2, graphics ARGB4444.
    A,
    /// Video 4:2:0, graphics ARGB4444.
    B,
    /// Video 4:2:0, graphics ARGB8888.
    C,
    /// As C with the video plane's gamma bit off (`VSCTRL[26]`).
    D,
    /// Video 4:2:0, graphics ARGB4444 with ordered dithering.
    E,
    /// As E, but composed in an `rp-gfx` layer and flushed through dirty
    /// rectangles (the OS's own drawing path).
    F,
}

impl Stage {
    /// All of them, in the order shown.
    pub const ALL: [Self; 6] = [Self::A, Self::B, Self::C, Self::D, Self::E, Self::F];

    /// The stage's letter.
    #[must_use]
    pub const fn letter(self) -> char {
        match self {
            Self::A => 'A',
            Self::B => 'B',
            Self::C => 'C',
            Self::D => 'D',
            Self::E => 'E',
            Self::F => 'F',
        }
    }

    /// The two lines of its on-screen label.
    #[must_use]
    pub const fn label(self) -> [&'static str; 2] {
        match self {
            Self::A => ["A  video 4:2:2", "   UI ARGB4444"],
            Self::B => ["B  video 4:2:0", "   UI ARGB4444"],
            Self::C => ["C  video 4:2:0", "   UI ARGB8888"],
            Self::D => ["D  C, no gamma", "   UI ARGB8888"],
            Self::E => ["E  video 4:2:0", "   UI 4444 dith"],
            Self::F => ["F  E via rp-gfx", "   dirty flush"],
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
            Self::D => (gfx(PixelFormat::Argb8888, 0x70_0000), yuv420),
            Self::E => (gfx(PixelFormat::Argb4444, 0x90_0000), yuv420),
            Self::F => (gfx(PixelFormat::Argb4444, 0xA0_0000), yuv420),
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
    if stage == Stage::D {
        d.regs()
            .modify(crate::layers::reg::VSCTRL, rp_hal_api::Field::bit(26), 0);
    }
    d.set_graphics(&gfx, GRAPHICS);
}

/// Rows where the graphics plane draws the wallpaper's own colours,
/// opaque: the top half of the colour bars and of the grey steps.
const fn reference_row(y: usize) -> bool {
    (400 <= y && y < 435) || (470 <= y && y < 505)
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
    if reference_row(y) {
        let (r, g, b) = wall(x, y);
        return 0xFF00_0000 | (r as u32) << 16 | (g as u32) << 8 | b as u32;
    }
    let (px, py, pw, ph) = PANEL;
    if (px..px + pw).contains(&x) && (py..py + ph).contains(&y) {
        let edge = x < px + 2 || x >= px + pw - 2 || y < py + 2 || y >= py + ph - 2;
        return if edge { EDGE } else { glass(x, y) };
    }
    let (sx, sy, sw, sh) = STRIP;
    if (sy..sy + sh).contains(&y) && (sx..sx + 16 * sw).contains(&x) {
        let a = (x - sx) / sw * 17;
        return (a as u32) << 24 | 0x00FF_2828;
    }
    0
}

/// The panel's 2-pixel frame.
const EDGE: u32 = 0xDCFF_FFFF;

/// Frosted glass inside the panel: alpha rising left to right, white to
/// pale blue top to bottom (smooth, so ARGB4444's 16 steps show as bands).
const fn glass(x: usize, y: usize) -> u32 {
    let (px, py, pw, ph) = PANEL;
    let a = 64 + (x - px) * 96 / pw;
    let t = (y - py) * 255 / ph;
    let (r, g) = (255 - t * 55 / 255, 255 - t * 30 / 255);
    (a as u32) << 24 | (r as u32) << 16 | (g as u32) << 8 | 0xFF
}

/// ARGB8888 to ARGB4444, each channel rounded.
#[must_use]
pub const fn to_4444(c: u32) -> u16 {
    const fn q(c: u32, s: u32) -> u32 {
        ((c >> s & 0xFF) * 15 + 127) / 255
    }
    (q(c, 24) << 12 | q(c, 16) << 8 | q(c, 8) << 4 | q(c, 0)) as u16
}

/// [`to_4444`] with the 4 × 4 ordered dither the owner approved (stage E).
/// It lives in `rp-gfx`, whose flush uses it, so the test scene and the OS
/// can't drift apart.
pub use rp_gfx::layer::to_4444_dithered;

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
pub const SAMPLES: [(usize, usize); 28] = [
    (90, 417),
    (330, 417),
    (30, 487),
    (270, 487), // reference halves (graphics plane)
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
/// Stage F composes its interface in `work` ([`WORK`] bytes, word
/// aligned); the other stages don't touch it.
///
/// # Panics
/// If `vram` is smaller than the planes need (16 MiB covers them), or for
/// stage F if `work` is too small or not word aligned.
pub fn draw(stage: Stage, vram: &mut [u8], work: &mut [u8]) {
    let (gfx, video) = stage.planes();
    let mut row = [0u8; W * 4];
    if stage == Stage::F {
        compose(&gfx, vram, work);
    }
    for y in (0..H).take_while(|_| stage != Stage::F) {
        let line = &mut row[..W * gfx.format.bytes()];
        for x in 0..W {
            let c = ui(stage, x, y);
            match gfx.format {
                PixelFormat::Argb8888 => line[x * 4..][..4].copy_from_slice(&c.to_le_bytes()),
                _ if stage == Stage::E => {
                    line[x * 2..][..2].copy_from_slice(&to_4444_dithered(c, x, y).to_le_bytes());
                }
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
        VideoFormat::Rgb565 => {
            for y in 0..H {
                for x in 0..W {
                    let (r, g, b) = wall(x, y);
                    let c =
                        (u16::from(r >> 3) << 11) | (u16::from(g >> 2) << 5) | u16::from(b >> 3);
                    row[x * 2..][..2].copy_from_slice(&c.to_le_bytes());
                }
                let at = video.offset + y * video.stride;
                vram[at..at + W * 2].copy_from_slice(&row[..W * 2]);
            }
        }
        // No stage shows an ARGB video plane.
        VideoFormat::Argb8888 | VideoFormat::Argb4444 => {}
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

/// RAM [`draw`] needs for stage F's interface layer.
pub const WORK: usize = W * H * 4;

/// Stage F's interface, drawn the way the OS will draw it: composed in a
/// RAM layer from `rp-gfx`'s primitives (fills, a glass gradient per row,
/// glyph masks drawn over), then sent to the plane as a compositor does: a
/// first full frame (all clear), then only the dirty rectangles, converted
/// with the dither on the way. The result must be byte for byte what
/// stage E's per-pixel conversion gives for the same picture (tested), so
/// the device shows E and F identically unless the drawing path is wrong.
#[inline(never)]
fn compose(gfx: &Plane, vram: &mut [u8], work: &mut [u8]) {
    use rp_gfx::Rect;
    use rp_gfx::dirty::Dirty;
    use rp_gfx::layer::{self, Layer, Mask, PlaneFormat};

    let rect =
        |x: usize, y: usize, w: usize, h: usize| Rect::new(x as u32, y as u32, w as u32, h as u32);
    let Some(mut l) = Layer::in_bytes(work, W as u32, H as u32) else {
        panic!("stage F needs {WORK} bytes of word-aligned RAM");
    };
    let fmt = PlaneFormat::Argb4444Dithered;
    let plane = &mut vram[gfx.offset..gfx.offset + gfx.stride * H];
    let full = l.bounds();
    l.fill(full, layer::CLEAR);
    layer::flush(&l.view(), fmt, plane, gfx.stride, &[full]);

    let mut dirty = Dirty::<8>::new(full);
    let mut fill = |l: &mut Layer<'_>, r: Rect, c: u32| {
        l.fill(r, c);
        dirty.add(r);
    };
    fill(&mut l, rect(0, 0, W, STATUS_H), 0x9600_0000);
    // The opaque reference halves: one flat rectangle per bar or step.
    for y0 in [400, 470] {
        for k in 0..8 {
            let (r, g, b) = wall(k * 60, y0);
            let c = 0xFF00_0000 | (r as u32) << 16 | (g as u32) << 8 | b as u32;
            fill(&mut l, rect(k * 60, y0, 60, 35), c);
        }
    }
    let (sx, sy, sw, sh) = STRIP;
    for k in 0..16 {
        let c = (k as u32 * 17) << 24 | 0x00FF_2828;
        fill(&mut l, rect(sx + k * sw, sy, sw, sh), c);
    }
    // The panel: its frame, then the glass a row at a time.
    let (px, py, pw, ph) = PANEL;
    fill(&mut l, rect(px, py, pw, ph), EDGE);
    let mut cols = [0u32; PANEL.2 - 4];
    for y in py + 2..py + ph - 2 {
        for (k, c) in cols.iter_mut().enumerate() {
            *c = glass(px + 2 + k, y);
        }
        l.fill_columns(rect(px + 2, y, pw - 4, 1), &cols);
    }
    // Text last, over everything: each glyph a coverage mask.
    let mut cover = [0u8; font::WIDTH * 4 * font::HEIGHT * 4];
    for Line(lx, ly, s, color, text) in lines(Stage::F) {
        let (gw, gh) = (font::WIDTH * s, font::HEIGHT * s);
        for (i, ch) in text.chars().enumerate() {
            let g = font::glyph(ch);
            for (k, c) in cover[..gw * gh].iter_mut().enumerate() {
                let (gx, gy) = (k % gw / s, k / gw / s);
                *c = if g[gy] & (1 << (font::WIDTH - 1 - gx)) != 0 {
                    255
                } else {
                    0
                };
            }
            let mask = Mask {
                cover: &cover[..gw * gh],
                w: gw as u32,
                h: gh as u32,
            };
            let x = lx + i * (font::WIDTH + 1) * s;
            l.draw_mask(&mask, color, x as u32, ly as u32);
        }
        dirty.add(rect(lx, ly, text.len() * (font::WIDTH + 1) * s, gh));
    }
    layer::flush(&l.view(), fmt, plane, gfx.stride, dirty.rects());
}

fn yuv_at(x: usize, y: usize) -> (u8, u8, u8) {
    let (r, g, b) = wall(x, y);
    rgb_to_yuv(r, g, b)
}

// ---- Run P9d: the scaler filter's format proven, and one fitted ----

/// Where P9d's stripe pattern goes: stage A's UYVY wallpaper's place
/// (stage A isn't shown in P9d), 480 x 640, 2 bytes a pixel.
pub const FILTER_AT: usize = 0x50_0000;

/// Black and white at studio range (`Y` 16 and 235), and their average.
pub const FILTER_DARK: u8 = 16;
/// White at studio range.
pub const FILTER_LIGHT: u8 = 235;
/// The two's average, rounded up.
pub const FILTER_MID: u8 = 126;

/// P9d's pattern's luma at `(x, y)` (grey throughout: `U` = `V` = 128):
/// four quarters of 240 x 320, the finest detail the scaler can be asked
/// to keep: top left one-pixel vertical stripes (the horizontal filter's
/// test), top right one-pixel horizontal stripes (the vertical filter's),
/// bottom left a one-pixel checker (both), bottom right four-pixel
/// stripes beside a patch of their average (what a right filter turns the
/// fine stripes into when shrinking).
#[must_use]
pub const fn filter_luma(x: usize, y: usize) -> u8 {
    let on = match (x < W / 2, y < H / 2) {
        (true, true) => x.is_multiple_of(2),
        (false, true) => y.is_multiple_of(2),
        (true, false) => (x + y).is_multiple_of(2),
        (false, false) => {
            if x < W / 2 + W / 4 {
                (x / 4).is_multiple_of(2)
            } else {
                return FILTER_MID;
            }
        }
    };
    if on { FILTER_LIGHT } else { FILTER_DARK }
}

/// Writes P9d's pattern at [`FILTER_AT`] (row by row, as every plane here).
pub fn draw_filter_pattern(vram: &mut [u8]) {
    let mut row = [0u8; W * 2];
    for y in 0..H {
        for x in (0..W).step_by(2) {
            row[x * 2..][..4].copy_from_slice(&[
                128,
                filter_luma(x, y),
                128,
                filter_luma(x + 1, y),
            ]);
        }
        let at = FILTER_AT + y * W * 2;
        vram[at..at + W * 2].copy_from_slice(&row);
    }
}

/// `GDRCTRL`'s pixel and line doubling (mbxfb's `PIXDBL`, `LNDBL`; Dell's
/// driver sets them for Windows Mobile's 2x mode, `ddi_2700g`
/// `0x021270B8`-`0x021270E8`, read 2026-10-24): reminder R16.
// inherited: Dell ddi_2700g and Linux mbxfb (GDRCTRL bits 31 and 29), proven on the X51v by P9d step 11
pub const GDRCTRL_DOUBLE: u32 = 1 << 31 | 1 << 29;

/// One of P9d's steps: what it is and what to see, the source rectangle
/// in the pattern, the scale, and whether the graphics plane (the label)
/// is doubled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FilterStep {
    /// Line 2 on screen.
    pub name: &'static str,
    /// Line 3: what the owner should see.
    pub expect: &'static str,
    /// The source (x, y, w, h) in the pattern.
    pub src: (u16, u16, u16, u16),
    /// The scale, place and filter.
    pub scale: crate::layers::Scale,
    /// `GDRCTRL`'s doubling on (R16).
    pub double: bool,
    /// The source is stage A's wallpaper (at [`FILTER_AT`] too, drawn
    /// again over the pattern) instead of the stripe pattern.
    pub wallpaper: bool,
}

const fn step(
    name: &'static str,
    expect: &'static str,
    src: (u16, u16, u16, u16),
    (out_w, out_h, x, y): (u16, u16, u16, u16),
    ur_c: bool,
    filter: crate::layers::ScalerFilter,
) -> FilterStep {
    FilterStep {
        name,
        expect,
        src,
        scale: crate::layers::Scale {
            out_w,
            out_h,
            x,
            y,
            taps: 4,
            ur_c,
            filter,
        },
        double: false,
        wallpaper: false,
    }
}

/// P9d's steps (2026-10-25, gate V5, quality audit C73 and C70, reminder
/// R16). Shrinking with mbxfb's set against the fitted one (1/2, 3/4);
/// enlarging the vertical stripes 2x with only the centre weight (hard
/// two-pixel stripes if the layout is read right), with mbxfb's set
/// (softer) and with mbxfb's halved (darker and green if 64 is one: the
/// chroma's 128 halves too); half size in one direction at a time without
/// bit 27 (which direction's scaling underruns, C70); then the graphics
/// plane doubled (R16). Bit 27 on wherever the picture is judged (`p9c`:
/// shrinking streaks without it).
pub const FILTER_STEPS: [FilterStep; 11] = {
    use crate::filter::Filter;
    use crate::layers::ScalerFilter::{Fitted, Given, Mbxfb};
    const FULL: (u16, u16, u16, u16) = (0, 0, W as u16, H as u16);
    const QUARTER: (u16, u16, u16, u16) = (0, 0, W as u16 / 2, H as u16 / 2);
    const HALF: (u16, u16, u16, u16) = (240, 320, 120, 160);
    const THREE_Q: (u16, u16, u16, u16) = (360, 480, 60, 80);
    const UP: (u16, u16, u16, u16) = (480, 640, 0, 0);
    const SAME: (u16, u16, u16, u16) = (0, 0, 0, 0);
    let mut d = step(
        "GRAPHICS DOUBLED",
        "LABEL 2X SIZE",
        FULL,
        SAME,
        false,
        Mbxfb,
    );
    d.double = true;
    [
        step(
            "1:1 (CONTROL)",
            "FINE STRIPES, GREY PATCH",
            FULL,
            SAME,
            false,
            Mbxfb,
        ),
        step(
            "HALF, LINUX FILTER",
            "STRIPES OR BANDS?",
            FULL,
            HALF,
            true,
            Mbxfb,
        ),
        step(
            "HALF, FITTED FILTER",
            "EVEN GREY LIKE PATCH",
            FULL,
            HALF,
            true,
            Fitted,
        ),
        step(
            "3/4, LINUX FILTER",
            "STRIPES OR BANDS?",
            FULL,
            THREE_Q,
            true,
            Mbxfb,
        ),
        step(
            "3/4, FITTED FILTER",
            "EVEN GREY LIKE PATCH",
            FULL,
            THREE_Q,
            true,
            Fitted,
        ),
        step(
            "2X UP, CENTRE ONLY",
            "HARD 2-PIXEL STRIPES",
            QUARTER,
            UP,
            true,
            Given(Filter::NEAREST),
        ),
        step(
            "2X UP, LINUX FILTER",
            "SOFTER STRIPES",
            QUARTER,
            UP,
            true,
            Mbxfb,
        ),
        step(
            "2X UP, HALF WEIGHTS",
            "AS LAST, DARK + GREEN",
            QUARTER,
            UP,
            true,
            Given(Filter::MBXFB.halved()),
        ),
        step(
            "HALF ACROSS, NO BIT 27",
            "UNDERRUN COUNTED",
            FULL,
            (240, 640, 120, 0),
            false,
            Fitted,
        ),
        step(
            "HALF DOWN, NO BIT 27",
            "UNDERRUN COUNTED",
            FULL,
            (480, 320, 0, 160),
            false,
            Fitted,
        ),
        d,
    ]
};

/// `s` on the wallpaper.
const fn on_wallpaper(mut s: FilterStep) -> FilterStep {
    s.wallpaper = true;
    s
}

/// Run P9e's steps (2026-10-25, after P9d's photos and the owner's rule
/// 0: the fitted cubic right at half size, stripes left at 3/4): the
/// tables computed for this hardware (`Filter::designed`) against P9d's
/// cubic and against the strongest blur (stripes gone, soft), on the
/// stripe pattern (do stripes go?) and on the wallpaper (as sharp as
/// Linux's set?), at 3/4; the designed table at half size; then half
/// height with `VideoPlane::scale_to`'s bit 27 (clean, not torn).
pub const FILTER2_STEPS: [FilterStep; 8] = {
    use crate::layers::ScalerFilter::{Designed, Fitted, Mbxfb, Stretched};
    const FULL: (u16, u16, u16, u16) = (0, 0, W as u16, H as u16);
    const THREE_Q: (u16, u16, u16, u16) = (360, 480, 60, 80);
    const HALF: (u16, u16, u16, u16) = (240, 320, 120, 160);
    [
        step(
            "STRIPES 3/4, OLD FILTER",
            "STRIPES (AS P9D STEP 5)",
            FULL,
            THREE_Q,
            true,
            Fitted,
        ),
        step(
            "STRIPES 3/4, BIG BLUR",
            "NO STRIPES, SOFT",
            FULL,
            THREE_Q,
            true,
            Stretched(2, 1),
        ),
        step(
            "STRIPES 3/4, DESIGNED",
            "NO STRIPES?",
            FULL,
            THREE_Q,
            true,
            Designed,
        ),
        on_wallpaper(step(
            "PICTURE 3/4, LINUX",
            "SHARPNESS REFERENCE",
            FULL,
            THREE_Q,
            true,
            Mbxfb,
        )),
        on_wallpaper(step(
            "PICTURE 3/4, BIG BLUR",
            "SOFTER THAN LINUX",
            FULL,
            THREE_Q,
            true,
            Stretched(2, 1),
        )),
        on_wallpaper(step(
            "PICTURE 3/4, DESIGNED",
            "AS SHARP AS LINUX?",
            FULL,
            THREE_Q,
            true,
            Designed,
        )),
        step(
            "STRIPES HALF, DESIGNED",
            "EVEN GREY LIKE PATCH?",
            FULL,
            HALF,
            true,
            Designed,
        ),
        on_wallpaper(step(
            "HALF HEIGHT, AUTO",
            "CLEAN, NOT TORN",
            FULL,
            (480, 320, 0, 160),
            true,
            Mbxfb,
        )),
    ]
};

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
        draw(stage, &mut vram, &mut vec![0u8; WORK]);
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
            let tol = if matches!(stage, Stage::C | Stage::D) {
                6
            } else {
                18
            };
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
    fn stage_f_drawn_like_the_os_is_byte_for_byte_the_dithered_picture() {
        let mut vram = vec![0u8; 16 << 20];
        // Garbage where the plane goes: the first full flush must clear it.
        vram.iter_mut()
            .enumerate()
            .for_each(|(i, b)| *b = (i * 31) as u8);
        let mut words = vec![0u32; WORK / 4];
        draw(
            Stage::F,
            &mut vram,
            rp_hal_mem::words_as_bytes_mut(&mut words),
        );
        let (gfx, _) = Stage::F.planes();
        for y in 0..H {
            for x in 0..W {
                let at = gfx.offset + y * gfx.stride + x * 2;
                let got = u16::from_le_bytes([vram[at], vram[at + 1]]);
                let ui = ui(Stage::F, x, y);
                assert_eq!(got, to_4444_dithered(ui, x, y), "({x},{y}): ui {ui:#010x}");
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
            if matches!(stage, Stage::A | Stage::B) {
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
        for (x, y) in [(0, 0), (3, 1), (2, 3)] {
            assert_eq!(to_4444_dithered(0xFFFF_FFFF, x, y), 0xFFFF);
            assert_eq!(to_4444_dithered(0, x, y), 0);
        }
        // Over the 4 x 4 pattern, a level between two steps averages to
        // within one sixteenth of a step of itself.
        let sum: u32 = (0..16)
            .map(|i| u32::from(to_4444_dithered(0x80, i % 4, i / 4) & 0xF))
            .sum();
        assert!(sum.abs_diff(0x80 * 15 * 16 / 255) <= 1, "{sum}");
    }

    #[test]
    fn p9d_pattern_holds_the_finest_detail_and_its_average() {
        // Each quarter's pattern at its corners and edges (both phases).
        assert_eq!(
            (filter_luma(0, 0), filter_luma(1, 0)),
            (FILTER_LIGHT, FILTER_DARK)
        );
        assert_eq!(
            (filter_luma(239, 319), filter_luma(238, 0)),
            (FILTER_DARK, FILTER_LIGHT)
        );
        assert_eq!(
            (filter_luma(240, 0), filter_luma(240, 1)),
            (FILTER_LIGHT, FILTER_DARK)
        );
        assert_eq!(
            (filter_luma(0, 320), filter_luma(1, 320)),
            (FILTER_LIGHT, FILTER_DARK)
        );
        assert_eq!(
            (filter_luma(0, 321), filter_luma(239, 639)),
            (FILTER_DARK, FILTER_LIGHT)
        );
        assert_eq!(
            (filter_luma(240, 400), filter_luma(244, 400)),
            (FILTER_LIGHT, FILTER_DARK)
        );
        assert_eq!(
            (filter_luma(360, 320), filter_luma(479, 639)),
            (FILTER_MID, FILTER_MID)
        );
        // The patch is the stripes' average.
        let avg = (u16::from(FILTER_DARK) + u16::from(FILTER_LIGHT)).div_ceil(2);
        assert_eq!(u16::from(FILTER_MID), avg);
        // Drawn in UYVY at FILTER_AT, grey chroma, clear of rp-blend's
        // label plane at 0xB00000.
        let mut vram = std::vec![0u8; 0xB0_0000];
        draw_filter_pattern(&mut vram);
        assert_eq!(
            &vram[FILTER_AT..FILTER_AT + 4],
            &[128, FILTER_LIGHT, 128, FILTER_DARK]
        );
        let last = FILTER_AT + (H - 1) * W * 2 + (W - 2) * 2;
        assert_eq!(&vram[last..last + 4], &[128, FILTER_MID, 128, FILTER_MID]);
        const { assert!(FILTER_AT + W * H * 2 <= 0xB0_0000) };
    }

    #[test]
    fn p9d_steps_scale_within_the_scalers_range_and_name_their_filter() {
        use crate::layers::ScalerFilter;
        let mut fitted = 0;
        for s in FILTER_STEPS {
            let (_, _, w, h) = s.src;
            let mut v = VideoPlane::new(VideoFormat::Uyvy, w, h, FILTER_AT);
            v.stride = W * 2;
            // Panics if a ratio or place is out of range.
            let _ = v.registers_scaled(VIDEO, false, s.scale);
            assert!(s.name.len() <= 24 && s.expect.len() <= 24, "{}", s.name);
            fitted += usize::from(s.scale.filter == ScalerFilter::Fitted);
        }
        assert_eq!(fitted, 4);
        // The two proofs of the format; the doubling last and alone.
        assert_eq!(
            FILTER_STEPS[5].scale.filter,
            ScalerFilter::Given(crate::filter::Filter::NEAREST)
        );
        assert_eq!(
            FILTER_STEPS[7].scale.filter,
            ScalerFilter::Given(crate::filter::Filter::MBXFB.halved())
        );
        assert!(FILTER_STEPS[10].double);
        assert!(FILTER_STEPS[..10].iter().all(|s| !s.double));
        // Bit 27 off only on the 1:1 control, the underrun steps and the
        // doubling.
        let off: std::vec::Vec<usize> = (0..11).filter(|&i| !FILTER_STEPS[i].scale.ur_c).collect();
        assert_eq!(off, [0, 8, 9, 10]);
        assert_eq!(GDRCTRL_DOUBLE, 0xA000_0000);
    }

    #[test]
    fn p9e_puts_the_designed_filter_beside_the_old_one_and_the_big_blur() {
        use crate::layers::ScalerFilter::{Designed, Fitted, Mbxfb, Stretched};
        let filters: std::vec::Vec<_> = FILTER2_STEPS.iter().map(|s| s.scale.filter).collect();
        assert_eq!(
            filters,
            [
                Fitted,
                Stretched(2, 1),
                Designed,
                Mbxfb,
                Stretched(2, 1),
                Designed,
                Designed,
                Mbxfb
            ]
        );
        let wall: std::vec::Vec<bool> = FILTER2_STEPS.iter().map(|s| s.wallpaper).collect();
        assert_eq!(wall, [false, false, false, true, true, true, false, true]);
        let sizes: std::vec::Vec<(u16, u16)> = FILTER2_STEPS
            .iter()
            .map(|s| (s.scale.out_w, s.scale.out_h))
            .collect();
        assert_eq!(
            sizes,
            [
                (360, 480),
                (360, 480),
                (360, 480),
                (360, 480),
                (360, 480),
                (360, 480),
                (240, 320),
                (480, 320)
            ]
        );
        for s in &FILTER2_STEPS {
            assert!(s.scale.ur_c && !s.double);
            assert!(s.name.len() <= 24 && s.expect.len() <= 24, "{}", s.name);
            let mut v = VideoPlane::new(VideoFormat::Uyvy, 480, 640, FILTER_AT);
            v.stride = W * 2;
            let _ = v.registers_scaled(VIDEO, false, s.scale);
        }
        // The last step is scale_to's bit 27 and taps for half height
        // (run with mbxfb's filter, before the designed tables became
        // scale_to's default).
        let v = VideoPlane::new(VideoFormat::Uyvy, 480, 640, FILTER_AT);
        let auto = v.scale_to(480, 320, 0, 160);
        let s = FILTER2_STEPS[7].scale;
        assert_eq!(
            (s.ur_c, s.taps, s.out_h),
            (auto.ur_c, auto.taps, auto.out_h)
        );
    }
}
