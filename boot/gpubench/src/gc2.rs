//! Run GC2's scenes and checks (ROADMAP step 3, the owner's robustness
//! rules, 2026-10-23), shared by the image and its host test: the test
//! renders every scene through the model of the 3D core
//! (`rp-mbx3d-render`'s `model` feature) and holds [`MODEL`] to it, and the
//! image checks its references at Windows Mobile's clock against
//! [`MODEL`] (rule 1). Run GC's seven kinds are the first seven here.

use rp_mbx3d_render::{Draw, Paint, PixelFormat, Texture, UV_ONE, address, format, untwiddle};

/// The pictures' size.
pub const W: u32 = 480;
pub const H: u32 = 640;
/// A band: rows hashed together and where a wrong pixel is placed
/// (rule 7).
pub const BAND_ROWS: u32 = 16;
pub const BANDS: usize = (H / BAND_ROWS) as usize;

/// The kinds of drawing: (name, the target's format).
pub const KINDS: [(&str, PixelFormat); 13] = [
    ("RGB565 Gouraud", PixelFormat::Rgb565),
    ("RGB565 dithered Gouraud", PixelFormat::Rgb565Dithered),
    ("ARGB8888 Gouraud", PixelFormat::Argb8888),
    ("RGB565 Gouraud + 8888 texture (FT2)", PixelFormat::Rgb565),
    ("ARGB8888 GS scene", PixelFormat::Argb8888),
    ("RGB565 bilinear 565 texture", PixelFormat::Rgb565),
    ("RGB565 Gouraud + blended 4444", PixelFormat::Rgb565),
    ("RGB565 Gouraud capped", PixelFormat::Rgb565),
    ("ARGB8888 Gouraud capped", PixelFormat::Argb8888),
    ("RGB565 planar Gouraud", PixelFormat::Rgb565),
    (
        "RGB565 planar Gouraud as a texture (d)",
        PixelFormat::Rgb565,
    ),
    ("ARGB8888 planar Gouraud", PixelFormat::Argb8888),
    (
        "ARGB8888 planar Gouraud as a texture (d)",
        PixelFormat::Argb8888,
    ),
];

/// Run GC2's kinds: the content map's nine.
pub const RUN: core::ops::Range<usize> = 0..9;
/// Run GC2D's kinds (the roadmap's idea (d): Gouraud drawn as a texture,
/// since textures were right at every clock): the planar Gouraud quad and
/// the same drawn as a bilinear texture, on each target.
pub const RUN_D: core::ops::Range<usize> = 9..13;
/// GC2D's pairs: (the Gouraud kind, the same drawn as a texture).
pub const PAIRS_D: [(usize, usize); 2] = [(9, 10), (11, 12)];

const GREY: u32 = 0xFF20_2020;
const WHITE: u32 = 0xFFFF_FFFF;
const BLACK: u32 = 0xFF00_0000;
const BLUE: u32 = 0xFF10_20A0;
const YELLOW: u32 = 0xFFFF_E000;

/// The Gouraud corners (top left, top right, bottom right, bottom left).
const CORNERS: [u32; 4] = [BLACK, WHITE, BLUE, YELLOW];
/// The same, every colour channel one RGB565 step below its top (red and
/// blue at most 30 of 31, `0xF7`; green 62 of 63, `0xFB`), so a +1 step
/// can't wrap at white (the roadmap's idea (c)).
const CORNERS_565_CAPPED: [u32; 4] = [BLACK, 0xFFF7_FBF7, BLUE, 0xFFF7_E000];
/// The same for ARGB8888: every channel, alpha too, at most `0xFE`.
const CORNERS_8888_CAPPED: [u32; 4] = [0xFE00_0000, 0xFEFE_FEFE, 0xFE10_20A0, 0xFEFE_E000];

/// A plane's corners: top left, top right, bottom left free, bottom
/// right their sum less the top left (`c1 + c2 - c0` in every channel,
/// none past its top), so the colour is one plane across the quad: what
/// two Gouraud triangles draw and what bilinear filtering between four
/// texels draws alike (idea (d)).
const PLANE: [u32; 4] = [0xFF00_0000, 0xFFF0_0000, 0xFF00_E0F0, 0xFFF0_E0F0];

/// The textures the scenes use: RGB565 (side 128, drawn nearest and
/// bilinear), ARGB4444 (side 128), ARGB8888 (side 32, clamped), and the
/// plane's corners as an ARGB8888 texture (side 8, the least: the corners
/// in its top left 2 x 2, bilinear, clamped).
#[derive(Clone, Copy)]
pub struct Tex {
    pub t565: Texture,
    pub t4444: Texture,
    pub t8888: Texture,
    pub plane: Texture,
}

const fn texture(code: u8, f: u32, mode: u32) -> Texture {
    Texture {
        at: 0,
        code,
        format: f as u8,
        u_mode: mode as u8,
        v_mode: mode as u8,
        linear: false,
        levels: 1,
    }
}

/// [`Tex`]'s textures as described, not yet placed (`at` 0): the caller
/// takes memory for each and writes [`texture_words`] there.
pub const TEXTURES: [Texture; 4] = [
    texture(4, format::RGB565, address::WRAP),
    texture(4, format::ARGB4444, address::WRAP),
    texture(2, format::ARGB8888, address::CLAMP),
    Texture {
        linear: true,
        ..texture(0, format::ARGB8888, address::CLAMP)
    },
];

/// Texture `n` of [`TEXTURES`] word by word in its twiddled order:
/// `put(word index, word)`.
pub fn texture_words(n: usize, mut put: impl FnMut(u32, u32)) {
    let t = TEXTURES[n];
    let texels = t.side() * t.side();
    if n == 3 {
        // The plane's corners at (0, 0)-(1, 1); the rest each its nearest
        // corner (only texels 0-1 are sampled).
        for i in 0..texels {
            let (x, y) = untwiddle(i);
            put(i, PLANE[(2 * y.min(1) + x.min(1)) as usize]);
        }
        return;
    }
    if n == 2 {
        for i in 0..texels {
            let (x, y) = untwiddle(i);
            put(i, texel8888(x, y));
        }
        return;
    }
    let f = if n == 0 { texel565 } else { texel4444 };
    for k in 0..texels / 2 {
        let (x0, y0) = untwiddle(2 * k);
        let (x1, y1) = untwiddle(2 * k + 1);
        put(k, u32::from(f(x0, y0)) | u32::from(f(x1, y1)) << 16);
    }
}

/// The RGB565 texture's texel: stripes and a ramp.
pub fn texel565(x: u32, y: u32) -> u16 {
    let r = (x >> 4) & 31;
    let gr = ((x + y) >> 3) & 63;
    let b = (y >> 4) & 31;
    (r << 11 | gr << 5 | b) as u16
}

/// The ARGB4444 texture's texel: alpha 8/15 (half see-through), stripes.
pub fn texel4444(x: u32, y: u32) -> u16 {
    (0x8000 | ((x >> 5) & 15) << 8 | ((y >> 5) & 15) << 4 | ((x ^ y) >> 4) & 15) as u16
}

/// The ARGB8888 texture's texel: red `x * 8`, green `y * 8`, blue `0xA0`.
pub const fn texel8888(x: u32, y: u32) -> u32 {
    0xFF00_00A0 | (x << 3) << 16 | (y << 3) << 8
}

/// The whole target covered by `tex` at `scale` texels a pixel, in
/// sixteenths (16: 1:1).
fn cover(tex: Texture, scale: i32, blend: bool) -> Draw {
    let side = tex.side() as i32;
    let u1 = (W as i32 * scale / 16) * (UV_ONE / side);
    let v1 = (H as i32 * scale / 16) * (UV_ONE / side);
    let d = Draw::rect(
        0,
        0,
        W as i32,
        H as i32,
        Paint::Texture {
            tex,
            uv: [(0, 0), (u1, 0), (u1, v1), (0, v1)],
            tint: WHITE,
        },
    );
    if blend { d.over() } else { d }
}

fn flat(c: u32, blend: bool) -> Draw {
    let d = Draw::rect(0, 0, W as i32, H as i32, Paint::Flat(c));
    if blend { d.over() } else { d }
}

/// Kind `k`'s scene: the background and three slots.
#[must_use]
pub fn scene(k: usize, t: Tex) -> (u32, [Option<Draw>; 3]) {
    let shade = |c| Draw::rect(0, 0, W as i32, H as i32, Paint::Gouraud(c));
    let gouraud = shade(CORNERS);
    let mut lin = t.t565;
    lin.linear = true;
    match k {
        0..=2 => (BLACK, [Some(gouraud), None, None]),
        3 => (
            BLACK,
            [
                Some(gouraud),
                Some(Draw::rect(40, 40, 400, 560, Paint::texture(t.t8888))),
                None,
            ],
        ),
        4 => (
            GREY,
            [
                Some(cover(t.t565, 16, false)),
                Some(flat(0x8040_A0C0, true)),
                Some(Draw::rect(100, 200, 96, 96, Paint::Flat(0xFFFF_2000))),
            ],
        ),
        5 => (GREY, [Some(cover(lin, 11, false)), None, None]),
        6 => (BLACK, [Some(gouraud), Some(cover(t.t4444, 16, true)), None]),
        7 => (BLACK, [Some(shade(CORNERS_565_CAPPED)), None, None]),
        8 => (BLACK, [Some(shade(CORNERS_8888_CAPPED)), None, None]),
        9 | 11 => (
            BLACK,
            [
                Some(shade([PLANE[0], PLANE[1], PLANE[3], PLANE[2]])),
                None,
                None,
            ],
        ),
        _ => {
            // From texel 0's centre to texel 1's (an eighth of the side
            // apart), so the filter blends the four corners across the
            // whole quad.
            let (a, b) = (UV_ONE / 16, 3 * UV_ONE / 16);
            let paint = Paint::Texture {
                tex: t.plane,
                uv: [(a, a), (b, a), (b, b), (a, b)],
                tint: WHITE,
            };
            (
                BLACK,
                [
                    Some(Draw::rect(0, 0, W as i32, H as i32, paint)),
                    None,
                    None,
                ],
            )
        }
    }
}

/// Words in kind `k`'s picture.
#[must_use]
pub const fn words(k: usize) -> u32 {
    W * H * KINDS[k].1.bytes() / 4
}

/// Run GC2I: the safe region's end. While the 3D core renders, the display
/// may read local memory below it (`rp-glesfeat` G19 on the owner's X51v),
/// and a target inside it drew Dell's frame right at 99.67 and 133.25 MHz
/// / 1 where one outside didn't (G21). This run's own; `rp-vram-map` takes
/// the measured one with option A (ROADMAP step 3 (16)(b)).
pub const SAFE_END: u32 = 0xA_E000;

/// Run GC2I's label: a band of 64 rows of the 16-bit screen at local
/// memory `0x0` (960-byte rows), the rest of the screen showing the
/// target below it.
pub const LABEL_ROWS: u32 = 64;
pub const LABEL_BAND: u32 = LABEL_ROWS * W * 2;

/// Where run GC2I's targets inside the safe region start: right after the
/// label band, on a 4 KiB boundary (as every target before), so the screen
/// shows a 16-bit target row for row below the label.
pub const INSIDE_AT: u32 = LABEL_BAND;
const _: () = assert!(INSIDE_AT.is_multiple_of(0x1000));

/// Whether kind `k`'s target fits inside the safe region after the label
/// band: the 16-bit kinds (600 KiB); a 32-bit one (1.2 MiB) doesn't.
#[must_use]
pub const fn fits_inside(k: usize) -> bool {
    INSIDE_AT + 4 * words(k) <= SAFE_END
}

/// Where kind `k` renders in run GC2I: inside the safe region if it fits,
/// else at `outside` (as runs GC2 and GC2S: the 32-bit kinds, the run's own
/// control for the placement).
#[must_use]
pub const fn target_at(k: usize, outside: u32) -> u32 {
    if fits_inside(k) { INSIDE_AT } else { outside }
}

/// Run GC2J: where the 32-bit texture FT2 reads ([`TEXTURES`] 2, 4 KiB)
/// is copied inside the safe region, to test whether its reads from
/// outside are a second constraint at divide 1: a page after the 16-bit
/// target's end, never at it (the emulator writes the dithered target,
/// `0x650` 9, as 32-bit pixels at the 16-bit stride, one row past the
/// end: docs/hw/emulator.md; the image also checks the copy after every
/// render).
pub const TEX_INSIDE: u32 = INSIDE_AT + 2 * W * H + 0x1000;

/// Kind 3, RGB565 Gouraud with the 8888 texture: the kind GC2I's soak
/// found one wrong pixel in at 133.25 / 1.
pub const FT2: usize = 3;

/// Run GC2J's soak rotation: (kind, its 8888 texture inside the safe
/// region): every 16-bit kind with its textures where they are (outside),
/// and FT2 again with its texture inside.
pub const SOAK2: [(usize, bool); 7] = [
    (0, false),
    (1, false),
    (FT2, false),
    (FT2, true),
    (5, false),
    (6, false),
    (7, false),
];

/// Run GC2K's ways (the owner, 2026-10-24: isolate GC2J's patch, every
/// wrong pixel at x 21-37, y 604-607 beside FT2's quad's bottom-left
/// corner, in the last rows of a tile row, across tile seams 24 and 32;
/// the 3D core's tiles are 8 x 16): FT2 as is, the whole scene shifted, the
/// quad moved, the quad untextured, the quad with a 16-bit texture.
pub const PATCH_WAYS: [&str; 5] = [
    "FT2 as is",
    "the whole scene shifted (+3, +5)",
    "the quad moved, its corner at (61, 571)",
    "the quad untextured (flat grey)",
    "the quad with a 16-bit texture (the same texels, RGB565)",
];

/// Way 1's shift: off every tile seam (not a multiple of 8 across or 16
/// down), so a patch that follows the content moves and one tied to the
/// tiles doesn't.
pub const PATCH_SHIFT: (i32, i32) = (3, 5);

/// FT2's quad: x, y, width, height.
pub const FT2_QUAD: (i32, i32, i32, i32) = (40, 40, 400, 560);

/// Way 2's quad: its bottom-left corner at (61, 571), off every seam and
/// away from GC2J's patch.
pub const PATCH_MOVED: (i32, i32, i32, i32) = (61, 37, 380, 534);

/// Way 3's flat grey.
pub const PATCH_GREY: u32 = 0xFF80_8080;

/// Way `way`'s quad (x, y, width, height).
#[must_use]
pub const fn patch_quad(way: usize) -> (i32, i32, i32, i32) {
    let (x, y, w, h) = FT2_QUAD;
    match way {
        1 => (x + PATCH_SHIFT.0, y + PATCH_SHIFT.1, w, h),
        2 => PATCH_MOVED,
        _ => FT2_QUAD,
    }
}

/// Way `way`'s quad's bottom-left corner.
#[must_use]
pub const fn patch_corner(way: usize) -> (i32, i32) {
    let (x, y, _, h) = patch_quad(way);
    (x, y + h)
}

/// Way `way`'s scene: FT2's (its Gouraud background and the textured
/// quad) changed as the way says; `t565` is the 16-bit texture for way 4.
#[must_use]
pub fn patch_scene(way: usize, t: Tex, t565: Texture) -> (u32, [Option<Draw>; 3]) {
    let (sx, sy) = if way == 1 { PATCH_SHIFT } else { (0, 0) };
    let gouraud = Draw::rect(sx, sy, W as i32, H as i32, Paint::Gouraud(CORNERS));
    let (x, y, w, h) = patch_quad(way);
    let paint = match way {
        3 => Paint::Flat(PATCH_GREY),
        4 => Paint::texture(t565),
        _ => Paint::texture(t.t8888),
    };
    (
        BLACK,
        [Some(gouraud), Some(Draw::rect(x, y, w, h, paint)), None],
    )
}

/// GC2J's patch as seen (x 21-37 by y 604-607), its columns widened by 2
/// each side.
pub const GC2J_PATCH: (i32, i32, i32, i32) = (19, 604, 39, 607);

/// Where a wrong pixel at `(x, y)` of way `way` lies: at GC2J's place on
/// the screen, at GC2J's place relative to this way's quad corner (where
/// FT2's corner was: the patch following the content), both (ways where
/// the corner didn't move), or neither.
#[must_use]
pub const fn patch_spot(way: usize, x: i32, y: i32) -> (bool, bool) {
    let (x0, y0, x1, y1) = GC2J_PATCH;
    let (cx, cy) = patch_corner(way);
    let (fx, fy) = patch_corner(0);
    let (rx, ry) = (x - cx + fx, y - cy + fy);
    (
        x >= x0 && x <= x1 && y >= y0 && y <= y1,
        rx >= x0 && rx <= x1 && ry >= y0 && ry <= y1,
    )
}

/// One of run GC2L's ways: the textured quad (x, y, width, height), the
/// Gouraud background's shift, where the target is, and whether the
/// renderer lists the tiles in reverse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SweepWay {
    pub quad: (i32, i32, i32, i32),
    pub shift: (i32, i32),
    pub at: u32,
    pub reverse: bool,
}

impl SweepWay {
    /// The quad's bottom-left corner.
    #[must_use]
    pub const fn corner(&self) -> (i32, i32) {
        (self.quad.0, self.quad.1 + self.quad.3)
    }
}

/// Run GC2L's second target place: 32 KiB past [`INSIDE_AT`], still inside
/// the safe region (the screen's picture moves 34 rows and 128 pixels in
/// memory, so a fault tied to the memory address moves on the screen and
/// one tied to the screen's tiles doesn't).
pub const SWEEP_AT_OFFSET: u32 = 0x8000;

/// Run GC2L's third: `0x0`, where option C draws (ADR-0034 amendment P:
/// the screen itself).
pub const SWEEP_AT_SCREEN: u32 = 0;

/// The tile sweep's quad (width, height) and its corner's place inside its
/// 8 x 16 tile: GC2K's worst (corner (43, 605): 3 across, 13 down).
pub const SWEEP_SMALL: (i32, i32) = (120, 120);
pub const SWEEP_IN_TILE: (i32, i32) = (3, 13);

/// The tile sweep's tile columns and rows (4 x 4 over the screen; tile
/// column 5 and row 37 are GC2K's).
pub const SWEEP_COLS: [i32; 4] = [5, 20, 35, 42];
pub const SWEEP_ROWS: [i32; 4] = [9, 19, 29, 37];

/// Where GC2L's row and tile sweeps go: 4 and 20 (after the four
/// controls).
pub const SWEEP_ROWS_AT: usize = 4;
pub const SWEEP_TILES_AT: usize = 20;

/// Run GC2L's ways: 0 GC2K's worst way (the whole scene shifted, corner
/// (43, 605)) at [`INSIDE_AT`]; 1 the same at [`SWEEP_AT_OFFSET`] past it;
/// 2 the same at [`SWEEP_AT_SCREEN`] (option C's place); 3 the same at
/// [`INSIDE_AT`] with the tiles listed in reverse; 4-19 the row sweep
/// (FT2's quad at x 43, its bottom edge on each row 592-607 of the tile
/// row, the background unshifted); 20-35 the tile sweep ([`SWEEP_SMALL`]
/// quads, the corner at [`SWEEP_IN_TILE`] in each of [`SWEEP_COLS`] x
/// [`SWEEP_ROWS`]).
pub const SWEEP: [SweepWay; 36] = {
    let shifted = SweepWay {
        quad: (43, 45, 400, 560),
        shift: PATCH_SHIFT,
        at: INSIDE_AT,
        reverse: false,
    };
    let mut w = [shifted; 36];
    w[1].at = INSIDE_AT + SWEEP_AT_OFFSET;
    w[2].at = SWEEP_AT_SCREEN;
    w[3].reverse = true;
    let mut r = 0;
    while r < 16 {
        // (x, y, width, height): the bottom edge on row 592 + r.
        w[SWEEP_ROWS_AT + r] = SweepWay {
            quad: (43, 45, 400, 547 + r as i32),
            shift: (0, 0),
            at: INSIDE_AT,
            reverse: false,
        };
        r += 1;
    }
    let mut i = 0;
    while i < 16 {
        let (cx, cy) = (
            8 * SWEEP_COLS[i % 4] + SWEEP_IN_TILE.0,
            16 * SWEEP_ROWS[i / 4] + SWEEP_IN_TILE.1,
        );
        w[SWEEP_TILES_AT + i] = SweepWay {
            quad: (cx, cy - SWEEP_SMALL.1, SWEEP_SMALL.0, SWEEP_SMALL.1),
            shift: (0, 0),
            at: INSIDE_AT,
            reverse: false,
        };
        i += 1;
    }
    w
};

/// Way `way`'s scene: FT2's Gouraud background (shifted as the way says)
/// and its 8888 texture on the way's quad.
#[must_use]
pub fn sweep_scene(way: usize, t: Tex) -> (u32, [Option<Draw>; 3]) {
    let s = SWEEP[way];
    let gouraud = Draw::rect(
        s.shift.0,
        s.shift.1,
        W as i32,
        H as i32,
        Paint::Gouraud(CORNERS),
    );
    let (x, y, w, h) = s.quad;
    (
        BLACK,
        [
            Some(gouraud),
            Some(Draw::rect(x, y, w, h, Paint::texture(t.t8888))),
            None,
        ],
    )
}

/// One of run GC2O's scenes: a [`SWEEP`] way's scene, GC2K's worst scene
/// mirrored left to right, or a GC2 kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scene {
    Sweep(usize),
    Mirrored,
    Kind(usize),
    /// Run GC2U's busy tiles: FT2 (`false`) or GC2K's worst (`true`) with
    /// [`BUSY_LAYER`] blended over the whole screen last, so every tile has
    /// more to draw before its write-out (the tile-race hypothesis: the
    /// next tile reaching the tile buffer before the last one's rows are
    /// out).
    Busy(bool),
    /// Run GC2V's edge ways: [`EDGE_WAYS`]' way.
    Edge(usize),
    /// Run GC2V's hidden layer: FT2 (`false`) or GC2K's worst (`true`) with
    /// [`HIDDEN_LAYER`] drawn first under the whole scene: opaque and
    /// covered by the Gouraud background (all but GC2K's worst's (3, 5)
    /// strip, where it shows, in its reference too), so the core's
    /// hidden-surface removal shades almost none of it, but every tile has
    /// one more object to sort (a cheaper busy tile than [`BUSY_LAYER`]'s
    /// blend, which doubled the render).
    Hidden(bool),
}

/// Run GC2V's hidden layer's colour (opaque; never seen).
pub const HIDDEN_LAYER: u32 = 0xFF80_4020;

/// How an edge way's quad is painted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgePaint {
    /// FT2's 32-bit texture, as FT2 draws it.
    Tex,
    /// One flat colour, no texture.
    Flat,
    /// The texture with its coordinates moved half a texel down (the side
    /// 32 texture: 1/64 of `UV_ONE`).
    TexHalf,
}

/// One of run GC2V's edge ways at 146.25 / 1 (the owner, 2026-10-25,
/// after GC2U: every render wrong on the textured quad's top row from
/// 146.25): FT2's quad (x 40, 400 x 560) with its top edge at `y`, painted
/// as `paint`, over the Gouraud background or the black one alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EdgeWay {
    pub y: i32,
    pub paint: EdgePaint,
    pub gouraud: bool,
}

const fn edge(y: i32, paint: EdgePaint, gouraud: bool) -> EdgeWay {
    EdgeWay { y, paint, gouraud }
}

/// GC2V's edge ways: FT2 as is (the control, y 40), the top edge moved
/// through its tile row (tile rows are 16 high: 32 its first row, 47 its
/// last) and to the screen's top, then at y 40 flat, the texture half a
/// texel down, and without the Gouraud background.
pub const EDGE_WAYS: [EdgeWay; 9] = [
    edge(40, EdgePaint::Tex, true),
    edge(32, EdgePaint::Tex, true),
    edge(36, EdgePaint::Tex, true),
    edge(44, EdgePaint::Tex, true),
    edge(47, EdgePaint::Tex, true),
    edge(0, EdgePaint::Tex, true),
    edge(40, EdgePaint::Flat, true),
    edge(40, EdgePaint::TexHalf, true),
    edge(40, EdgePaint::Tex, false),
];
pub const EDGE_SCENES: [Scene; 9] = [
    Scene::Edge(0),
    Scene::Edge(1),
    Scene::Edge(2),
    Scene::Edge(3),
    Scene::Edge(4),
    Scene::Edge(5),
    Scene::Edge(6),
    Scene::Edge(7),
    Scene::Edge(8),
];
pub const EDGE_WAYS_ORDER: [(usize, u8); 9] = [
    (0, ROWS),
    (1, ROWS),
    (2, ROWS),
    (3, ROWS),
    (4, ROWS),
    (5, ROWS),
    (6, ROWS),
    (7, ROWS),
    (8, ROWS),
];
/// The edge ways' rounds: every render was wrong at 146.25 in GC2U.
pub const EDGE_ROUNDS: u32 = 10;
/// The flat edge way's colour.
pub const EDGE_FLAT: u32 = 0xFF30_A0E0;

/// Run GC2V's part B at 133.25 / 1: GC2K's worst as is, with busy tiles
/// (GC2U: 0 of 100 against 3-7, at 85% more render time) and with the
/// hidden layer, [`HIDDEN_ROUNDS`] rounds each.
pub const HIDDEN_SCENES: [Scene; 3] = [Scene::Sweep(0), Scene::Busy(true), Scene::Hidden(true)];
pub const HIDDEN_WAYS: [(usize, u8); 3] = [(0, ROWS), (1, ROWS), (2, ROWS)];
pub const HIDDEN_ROUNDS: u32 = 200;

/// Edge way `w`'s background and slots.
#[must_use]
pub fn edge_scene(w: usize, t: Tex) -> (u32, [Option<Draw>; 3]) {
    let e = EDGE_WAYS[w];
    let (x, _, wd, h) = FT2_QUAD;
    let paint = match e.paint {
        EdgePaint::Tex => Paint::texture(t.t8888),
        EdgePaint::Flat => Paint::Flat(EDGE_FLAT),
        EdgePaint::TexHalf => {
            let d = UV_ONE / 64;
            Paint::Texture {
                tex: t.t8888,
                uv: [(0, d), (UV_ONE, d), (UV_ONE, UV_ONE + d), (0, UV_ONE + d)],
                tint: WHITE,
            }
        }
    };
    let quad = Some(Draw::rect(x, e.y, wd, h, paint));
    let gouraud = Draw::rect(0, 0, W as i32, H as i32, Paint::Gouraud(CORNERS));
    if e.gouraud {
        (BLACK, [Some(gouraud), quad, None])
    } else {
        (BLACK, [quad, None, None])
    }
}

/// Run GC2U's busy layer: black at an eighth's opacity, blended over the
/// whole screen (it reads no texture: only the tiles' own work grows).
pub const BUSY_LAYER: u32 = 0x2000_0000;

/// Run GC2U's scenes (the owner, 2026-10-25: "build it", after GC2T: the
/// texture lighter, and the tile race): FT2 and GC2K's worst, then both
/// with busy tiles. The texture is the caller's (32-bit, 16-bit or PVRTC in
/// [`Tex::t8888`]'s place); the busy ways run with the 32-bit one only.
pub const TEXLOAD_SCENES: [Scene; 4] = [
    Scene::Kind(FT2),
    Scene::Sweep(0),
    Scene::Busy(false),
    Scene::Busy(true),
];
pub const TEXLOAD_WAYS: [(usize, u8); 4] = [(0, ROWS), (1, ROWS), (2, ROWS), (3, ROWS)];
/// The lighter textures' ways: FT2 and GC2K's worst only.
pub const TEXLOAD_LIGHT_WAYS: [(usize, u8); 2] = [(0, ROWS), (1, ROWS)];
/// Rounds a way: at 133.25 / 1 (GC2K's worst wrong about 4 in 100) and
/// above (FT2 wrong in every render from 146.25).
pub const TEXLOAD_ROUNDS: [u32; 4] = [100, 30, 30, 30];

/// The mirrored scene's quad (GC2K's worst, x 43-443, mirrored to 37-437)
/// and its background's shift: its bottom-right corner at (437, 605), the
/// tiles beside it to its right, drawn after it in row order and before it
/// in reverse.
pub const MIRRORED_QUAD: (i32, i32, i32, i32) = (37, 45, 400, 560);
pub const MIRRORED_SHIFT: (i32, i32) = (-PATCH_SHIFT.0, PATCH_SHIFT.1);

/// Run GC2O's scenes: GC2K's worst (`SWEEP[0]`), the row sweep's two worst
/// (rows 597 and 598), the worst mirrored, the six 16-bit GC2 kinds; then
/// the row sweep and the tile sweep (32).
pub const ORDER_SCENES: [Scene; 42] = {
    let mut s = [Scene::Sweep(0); 42];
    s[1] = Scene::Sweep(SWEEP_ROWS_AT + 5);
    s[2] = Scene::Sweep(SWEEP_ROWS_AT + 6);
    s[3] = Scene::Mirrored;
    let kinds = [0, 1, FT2, 5, 6, 7];
    let mut i = 0;
    while i < 6 {
        s[4 + i] = Scene::Kind(kinds[i]);
        i += 1;
    }
    let mut j = 0;
    while j < 32 {
        s[10 + j] = Scene::Sweep(SWEEP_ROWS_AT + j);
        j += 1;
    }
    s
};

/// How GC2O lists the tiles (`rp_mbx3d_render::TileOrder`'s, by number:
/// the shared module stays free of the renderer's state).
pub const ROWS: u8 = 0;
pub const REVERSED: u8 = 1;
pub const COLUMNS_REVERSED: u8 = 2;

/// Run GC2O's ways: (an [`ORDER_SCENES`] index, its tile order): the first
/// ten scenes in all three orders, the sweeps' 32 in the two new ones (row
/// order measured by GC2L).
pub const ORDER_WAYS: [(usize, u8); 94] = {
    let mut w = [(0, ROWS); 94];
    let mut n = 0;
    let mut s = 0;
    while s < 10 {
        w[n] = (s, ROWS);
        w[n + 1] = (s, REVERSED);
        w[n + 2] = (s, COLUMNS_REVERSED);
        n += 3;
        s += 1;
    }
    while s < 42 {
        w[n] = (s, REVERSED);
        w[n + 1] = (s, COLUMNS_REVERSED);
        n += 2;
        s += 1;
    }
    w
};

/// Run GC2P's scenes (GC2O made fast, the owner, 2026-10-24): GC2K's
/// worst; the row sweep's rows 595-603 (the rows that went wrong in GC2L);
/// the mirror; FT2; the five other 16-bit kinds. No tile sweep (never wrong
/// in GC2L).
pub const FAST_SCENES: [Scene; 17] = {
    let mut s = [Scene::Sweep(0); 17];
    let mut r = 0;
    while r < 9 {
        s[1 + r] = Scene::Sweep(SWEEP_ROWS_AT + 3 + r);
        r += 1;
    }
    s[10] = Scene::Mirrored;
    s[11] = Scene::Kind(FT2);
    let kinds = [0, 1, 5, 6, 7];
    let mut i = 0;
    while i < 5 {
        s[12 + i] = Scene::Kind(kinds[i]);
        i += 1;
    }
    s
};

/// GC2P's scenes in all three orders (the rest in the two new ones): GC2K's
/// worst, rows 597 and 598 (GC2L's worst rows), the mirror, FT2.
pub const FAST_ALL_ORDERS: [usize; 5] = [0, 3, 4, 10, 11];

/// Run GC2P's ways: [`FAST_ALL_ORDERS`]' scenes in rows, reversed and
/// columns reversed; every other scene in reversed and columns reversed.
pub const FAST_WAYS: [(usize, u8); 39] = {
    let mut w = [(0, ROWS); 39];
    let mut n = 0;
    let mut s = 0;
    while s < 17 {
        let mut all = false;
        let mut i = 0;
        while i < FAST_ALL_ORDERS.len() {
            all |= FAST_ALL_ORDERS[i] == s;
            i += 1;
        }
        if all {
            w[n] = (s, ROWS);
            n += 1;
        }
        w[n] = (s, REVERSED);
        w[n + 1] = (s, COLUMNS_REVERSED);
        n += 2;
        s += 1;
    }
    w
};

/// GC2P's renders a way: enough to see GC2L's normal-order rate (about 1 in
/// 9 for GC2K's worst, 1 in 14 for row 597) many times over: about 16
/// wrong expected in 150, so 0 of 150 in a new order means something.
pub const FAST_ROUNDS: u32 = 150;

/// Run GC2Q's climb above 133.25 MHz at divide 1 (the owner, 2026-10-24:
/// "i really wanted to see what overclock higher then 133 would do"): the
/// core PLL's (M, N) for 133.25 (the reference), 139.75, 146.25 and 149.50
/// MHz (13 MHz x M / N), every one below 156 MHz, where the local memory
/// failed (run GC2H2), 149.50 the highest it ran right (GC2H).
pub const ABOVE_PLL: [(u32, u32); 4] = [(41, 4), (43, 4), (45, 4), (23, 2)];

/// The climb's scenes: GC2's six 16-bit kinds, each in row order.
pub const ABOVE_SCENES: [Scene; 6] = [
    Scene::Kind(0),
    Scene::Kind(1),
    Scene::Kind(FT2),
    Scene::Kind(5),
    Scene::Kind(6),
    Scene::Kind(7),
];
pub const ABOVE_WAYS: [(usize, u8); 6] = [
    (0, ROWS),
    (1, ROWS),
    (2, ROWS),
    (3, ROWS),
    (4, ROWS),
    (5, ROWS),
];

/// GC2Q's confirmation scene for a write-out setting the search finds:
/// GC2K's worst in row order (about 1 render in 10 wrong at 133.25 / 1).
pub const CONFIRM_SCENES: [Scene; 1] = [Scene::Sweep(0)];
pub const CONFIRM_WAYS: [(usize, u8); 1] = [(0, ROWS)];

/// Run GC2T's local-memory timings (the owner, 2026-10-24: "fix it and do
/// the test", after GC2Q: the fault a tile's last rows written while
/// another reader needs another row, uniform, not heat): each a name and
/// the cycles added to `LMTIM`'s fields [tRAS, tRP, tRCD, tRC, tDPL] over
/// the rated timing (`clock::Speed::rated`), whose write recovery tDPL is 2
/// cycles at every clock (15 ns at 133.25 MHz, 13.7 at 146.25: the suspect).
/// tRC +1 raises tRAS with it, keeping tRAS + tRP >= tRC + 1 as the rated
/// timing does.
pub const TIM_WAYS: [(&str, [u32; 5]); 7] = [
    ("rated", [0, 0, 0, 0, 0]),
    ("tDPL +1", [0, 0, 0, 0, 1]),
    ("tDPL +2", [0, 0, 0, 0, 2]),
    ("tRP +1", [0, 1, 0, 0, 0]),
    ("tRCD +1", [0, 0, 1, 0, 0]),
    ("tRC +1", [1, 0, 0, 1, 0]),
    ("all +1", [1, 1, 1, 1, 1]),
];

/// `LMTIM`'s fields' shifts, in [`TIM_WAYS`]' order (4 bits each).
pub const TIM_SHIFTS: [u32; 5] = [16, 12, 8, 4, 0];

/// GC2T's timings on GC2K's worst at 133.25 / 1 (rated, tDPL +1, +2, all
/// +1) and over the climb (rated, tDPL +1, all +1): indices into
/// [`TIM_WAYS`].
pub const TIM_CONFIRM: [usize; 4] = [0, 1, 2, 6];
pub const TIM_CLIMB: [usize; 3] = [0, 1, 6];

/// `base` (an `LMTIM` value) with `add` cycles on its fields, or `None`
/// when a field would pass its 4 bits. Bits above the fields are kept.
#[must_use]
pub const fn lmtim_with(base: u32, add: [u32; 5]) -> Option<u32> {
    let mut out = base & !0xF_FFFF;
    let mut i = 0;
    while i < 5 {
        let v = (base >> TIM_SHIFTS[i] & 0xF) + add[i];
        if v > 0xF {
            return None;
        }
        out |= v << TIM_SHIFTS[i];
        i += 1;
    }
    Some(out)
}

/// Scene `s`'s background and slots.
#[must_use]
pub fn order_scene(s: Scene, t: Tex) -> (u32, [Option<Draw>; 3]) {
    match s {
        Scene::Sweep(w) => sweep_scene(w, t),
        Scene::Kind(k) => scene(k, t),
        Scene::Busy(worst) => {
            let (bg, mut slots) = if worst {
                sweep_scene(0, t)
            } else {
                scene(FT2, t)
            };
            slots[2] = Some(flat(BUSY_LAYER, true));
            (bg, slots)
        }
        Scene::Edge(w) => edge_scene(w, t),
        Scene::Hidden(worst) => {
            let (bg, slots) = if worst {
                sweep_scene(0, t)
            } else {
                scene(FT2, t)
            };
            (bg, [Some(flat(HIDDEN_LAYER, false)), slots[0], slots[1]])
        }
        Scene::Mirrored => {
            let c = [CORNERS[1], CORNERS[0], CORNERS[3], CORNERS[2]];
            let (sx, sy) = MIRRORED_SHIFT;
            let gouraud = Draw::rect(sx, sy, W as i32, H as i32, Paint::Gouraud(c));
            let (x, y, w, h) = MIRRORED_QUAD;
            let quad = Draw::rect(
                x,
                y,
                w,
                h,
                Paint::Texture {
                    tex: t.t8888,
                    uv: [(UV_ONE, 0), (0, 0), (0, UV_ONE), (UV_ONE, UV_ONE)],
                    tint: WHITE,
                },
            );
            (BLACK, [Some(gouraud), Some(quad), None])
        }
    }
}

/// Where scene `s`'s wrong pixels are placed from: the textured quad's
/// corner GC2K's patch sits beside (the bottom-left; the mirrored scene's
/// bottom-right; FT2's quad's), else the screen's corner (0, 0).
#[must_use]
pub const fn order_corner(s: Scene) -> (i32, i32) {
    match s {
        Scene::Sweep(w) => SWEEP[w].corner(),
        Scene::Mirrored => (
            MIRRORED_QUAD.0 + MIRRORED_QUAD.2,
            MIRRORED_QUAD.1 + MIRRORED_QUAD.3,
        ),
        Scene::Kind(FT2) | Scene::Busy(false) => patch_corner(0),
        Scene::Busy(true) | Scene::Hidden(true) => SWEEP[0].corner(),
        Scene::Hidden(false) => patch_corner(0),
        // The quad's top-left corner: the fault's row against the edge.
        Scene::Edge(w) => (FT2_QUAD.0, EDGE_WAYS[w].y),
        Scene::Kind(_) => (0, 0),
    }
}

/// Words in one band of kind `k`'s picture.
#[must_use]
pub const fn band_words(k: usize) -> u32 {
    words(k) / BANDS as u32
}

/// The bands' hash: FNV-1a over words, from [`HASH_START`].
#[must_use]
pub const fn fold(h: u32, w: u32) -> u32 {
    (h ^ w).wrapping_mul(0x0100_0193)
}

pub const HASH_START: u32 = 0x811C_9DC5;

/// Each kind's band hashes as the model renders it (the host test holds
/// this table to the model; regenerate it from the test's message).
pub const MODEL: [[u32; BANDS]; 13] = include!("gc2_model.in");

/// How a wrong pixel differs from the reference (rule 7). Channel by
/// channel (alpha too, on ARGB8888): a channel that held its top value or
/// one below and dropped by more than half its range wrapped; otherwise
/// every channel 0 or +1 is "+1 step", 0 or -1 "-1 step", anything else
/// "other".
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Miss {
    Plus1 = 0,
    Minus1 = 1,
    Wrapped = 2,
    Other = 3,
}

/// The channels of a pixel in `f`: (shift, bits).
const fn channels(f: PixelFormat) -> &'static [(u32, u32)] {
    match f {
        PixelFormat::Argb8888 => &[(24, 8), (16, 8), (8, 8), (0, 8)],
        PixelFormat::Rgb565 | PixelFormat::Rgb565Dithered => &[(11, 5), (5, 6), (0, 5)],
        PixelFormat::Argb1555 => &[(15, 1), (10, 5), (5, 5), (0, 5)],
    }
}

/// How `got` differs from `want` (two different pixels in `f`).
#[must_use]
pub fn miss(f: PixelFormat, want: u32, got: u32) -> Miss {
    let (mut up, mut down, mut wrapped) = (false, false, false);
    let mut other = false;
    for &(shift, bits) in channels(f) {
        let max = (1i32 << bits) - 1;
        let w = (want >> shift) as i32 & max;
        let g = (got >> shift) as i32 & max;
        match g - w {
            0 => {}
            1 => up = true,
            -1 => down = true,
            d if w >= max - 1 && -d > max / 2 => wrapped = true,
            _ => other = true,
        }
    }
    if wrapped {
        Miss::Wrapped
    } else if other || (up && down) {
        Miss::Other
    } else if up {
        Miss::Plus1
    } else {
        Miss::Minus1
    }
}

/// The wrong pixels of one or more renders of a kind, described (rule 7).
#[derive(Clone, Copy, Default)]
pub struct Tally {
    /// Pixels by [`Miss`].
    pub by: [u32; 4],
    /// Bit `b`: a wrong pixel in band `b` (16 rows).
    pub bands: u64,
    /// The wrong pixels' bounding box: x0, y0, x1, y1 (inclusive).
    pub bbox: Option<(u16, u16, u16, u16)>,
    /// The first three: x, y, the reference's pixel, the read one.
    pub first: [(u16, u16, u32, u32); 3],
    pub n_first: usize,
}

impl Tally {
    /// Every wrong pixel.
    #[must_use]
    pub fn pixels(&self) -> u32 {
        self.by.iter().sum()
    }

    /// Word `i` of a picture in `f` read as `got` where the reference
    /// holds `want`: each differing pixel in it counted.
    pub fn add(&mut self, f: PixelFormat, i: u32, want: u32, got: u32) {
        let per = if f.bytes() == 4 { 1 } else { 2 };
        let (bits, mask) = if per == 1 {
            (32, u32::MAX)
        } else {
            (16, 0xFFFF)
        };
        for p in 0..per {
            let (a, b) = (want >> (bits * p) & mask, got >> (bits * p) & mask);
            if a == b {
                continue;
            }
            let at = i * per + p;
            let (x, y) = ((at % W) as u16, (at / W) as u16);
            self.by[miss(f, a, b) as usize] += 1;
            self.bands |= 1 << (u32::from(y) / BAND_ROWS);
            self.bbox = Some(match self.bbox {
                None => (x, y, x, y),
                Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
            });
            if self.n_first < self.first.len() {
                self.first[self.n_first] = (x, y, a, b);
                self.n_first += 1;
            }
        }
    }

    /// `o`'s pixels added to these (the first three kept from whichever
    /// came first).
    pub fn merge(&mut self, o: &Self) {
        for (a, b) in self.by.iter_mut().zip(o.by) {
            *a += b;
        }
        self.bands |= o.bands;
        self.bbox = match (self.bbox, o.bbox) {
            (None, b) | (b, None) => b,
            (Some((a0, b0, a1, b1)), Some((c0, d0, c1, d1))) => {
                Some((a0.min(c0), b0.min(d0), a1.max(c1), b1.max(d1)))
            }
        };
        for &f in &o.first[..o.n_first] {
            if self.n_first < self.first.len() {
                self.first[self.n_first] = f;
                self.n_first += 1;
            }
        }
    }
}

#[cfg(test)]
#[path = "gc2_tests.rs"]
mod tests;
