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
    /// Run GC2BH: kind `k` with every clamped texture shifted half a texel
    /// in u and v ([`TexFix::ClampUv`]: GC2Z's texel cure).
    KindUv(usize),
    /// Run GC2BI: kind 10's texture (texel 0's centre to texel 1's,
    /// bilinear) on one `m` x `m` quad at the top left: magnified `m`
    /// times in each direction (kind 10 itself is 480 x 640).
    Magnified(usize),
    /// Run GC2BI's compensation: kind 10 with its texture coordinates
    /// moved by (du, dv) in [`UV_ONE`] units of the texture (negative:
    /// pulled back against divide 1's one-sided error).
    Shifted10(i32, i32),
    /// Run GC2BJ: kind 10 drawn as `cols` x `rows` quads tiling the screen,
    /// each with its part of the texture span (the same picture in exact
    /// arithmetic), so no pixel lies far from its quad's first corner;
    /// only through [`order_scene_wide`].
    Split10(usize, usize),
    /// Run GC2BK: kind 10 drawn from [`Tex::plane`] holding the plane's
    /// corners enlarged on the CPU ([`upsampled_words`]) to a texture of
    /// side `8 << code`, sampled from its first texel's centre to its
    /// last's: the same picture as kind 10 in exact arithmetic, magnified
    /// `side - 1` times less.
    Upsampled10(u8),
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
    /// Run GC2Y's cured scenes: [`CURED_BASE`]'s scene with both of GC2V's
    /// cures (see [`cure`]).
    Cured(usize),
    /// Run GC2Z's ways: [`FIX_BASE`]'s scene with [`FIXES`]' fix (see
    /// [`fix_scene`]; only through [`order_scene_wide`]).
    Fix(usize, usize),
    /// Run GC2AH's geometry: [`GEO`]'s way (see [`geo_scene`]; only
    /// through [`order_scene_wide`]).
    Geo(usize),
    /// Run GC2AI's quad ways: [`QUAD_WAYS`]' way (see [`quad_scene`]; only
    /// through [`order_scene_wide`]).
    Quad(usize),
    /// Run GC2AJ's corner sweep: textured or flat, scene `i` (see
    /// [`corner_scene`]; only through [`order_scene_wide`]).
    Corner(bool, usize),
    /// Run GC2BB's low-overdraw scene: flat corner scene `i` without its
    /// hidden layer (the same picture: the layer is never seen; only
    /// through [`order_scene_wide`]).
    CornerNoLayer(usize),
    /// Run GC2BR's shading ways: flat corner scene `i` with [`SHADE_FLAT_BG`]
    /// (the Gouraud background drawn flat) and [`SHADE_GOURAUD_RECT`] (the
    /// grey rectangles drawn Gouraud with four equal colours) as `m`'s bits
    /// say (see [`corner_shade_scene`]; only through [`order_scene_wide`]).
    CornerShade(usize, u8),
    /// Run GC2BS's forms of flat corner scene 3 without its hidden layer
    /// (GC2BR: 31% wrong, the chase's strongest generator at peak heat):
    /// `m` one of [`S3_AS_IS`], [`S3_ALONE`], [`S3_ALONE_LEFT`],
    /// [`S3_ALONE_DOWN`], [`S3_REVERSED`], [`S3_GREY84`] (see
    /// [`corner3_scene`]; only through [`order_scene_wide`]).
    Corner3(u8),
    /// Run GC2BU's position map: scene 3's top-right rectangle alone,
    /// without the layer, moved `(dx, dy)` pixels (see [`corner3_scene`]).
    Corner3At(i16, i16),
    /// Run GC2BV's sealed sweep: flat corner scene `i` (its layer kept) with
    /// [`extra_slots`]' seal `seal` ([`S3_BAND`] or [`S3_DOUBLE`]) on every
    /// rectangle (only through [`order_scene_wide`]).
    CornerSeal(usize, u8),
    /// Run GC2AK's textured sweep with the u and v shift (see
    /// [`corner_uv_scene`]; only through [`order_scene_wide`]).
    CornerUv(usize),
    /// Run GC2AL's cure `c`, scene `i` (see [`cure_scene`]; only through
    /// [`order_scene_wide`]).
    Cure(usize, usize),
    /// Run GC2AM's phase step `n` (see [`phase_scene`]).
    Phase(usize),
    /// Run GC2AM's ratio `r`, scene `i` (see [`ratio_scene`]).
    Ratio(usize, usize),
    /// Run GC2AO's proof: the layer or not, textured or flat, scene `i`
    /// (see [`proof_scene`]).
    Proof(bool, bool, usize),
}

/// The objects a scene holds through [`order_scene_wide`]: a scene's own
/// three, up to four hidden layers under them and a busy layer over them
/// (run GC2Z's stronger corner cures).
pub const SLOTS: usize = 8;

/// Where a fix moves texture coordinates by half a texel (to texel
/// centres): GC2Y's u and v shift on every texture ([`cure`]) cured FT2's
/// top row and broke kind 6's wrapping, repeated texture from 143.00.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TexFix {
    /// v only, every texture (GC2V's, which cleared FT2's top row).
    V,
    /// u and v, clamped textures only (wrapping ones as they are).
    ClampUv,
    /// v only, clamped textures only.
    ClampV,
}

/// One of run GC2Z's fixes: the texture shift, how many hidden opaque
/// layers lie under the scene (GC2V's corner cure, one of them), whether
/// GC2U's busy layer is blended over it, and whether a hidden ring
/// ([`ring`]) lies round every textured object (run GC2AC).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fix {
    pub tex: TexFix,
    pub hidden: usize,
    pub busy: bool,
    pub ring: bool,
}

const fn fix(tex: TexFix, hidden: usize, busy: bool) -> Fix {
    Fix {
        tex,
        hidden,
        busy,
        ring: false,
    }
}

/// Run GC2AC's fixes: the clamped v shift, `hidden` full hidden layers and
/// the ring.
const fn ring_fix(hidden: usize) -> Fix {
    Fix {
        tex: TexFix::ClampV,
        hidden,
        busy: false,
        ring: true,
    }
}

/// Run GC2Z's fixes (the owner, 2026-10-25: "Build the narrower texture fix
/// and a stronger corner-patch fix, tested at 139.75-149.50"): the three
/// narrower texture shifts with one hidden layer, then the clamped-v shift
/// with two and four hidden layers, and with one and the busy layer (the
/// cure that cleared GC2K's patch at 133.25, at 85% more render time).
pub const FIXES: [Fix; 9] = [
    fix(TexFix::V, 1, false),
    fix(TexFix::ClampUv, 1, false),
    fix(TexFix::ClampV, 1, false),
    fix(TexFix::ClampV, 2, false),
    fix(TexFix::ClampV, 4, false),
    fix(TexFix::ClampV, 1, true),
    // Run GC2AA: the clamped v shift with no hidden layer (the tile order
    // alone against GC2K's patch).
    fix(TexFix::ClampV, 0, false),
    // Run GC2AC: the ring alone, and the ring over one full hidden layer.
    ring_fix(0),
    ring_fix(1),
];

/// GC2Z's scenes to fix: FT2 and GC2K's worst (clamped textures), kinds 5
/// and 6 (wrapping, repeated textures: 6 broke under GC2Y's shift); then
/// kinds 0, 1 and 7 (run GC2AA's soak: every 16-bit kind).
pub const FIX_BASE: [Scene; 7] = [
    Scene::Kind(FT2),
    Scene::Sweep(0),
    Scene::Kind(5),
    Scene::Kind(6),
    // Run GC2AA's soak: the untextured kinds too (every GC2 16-bit kind).
    Scene::Kind(0),
    Scene::Kind(1),
    Scene::Kind(7),
];

/// Part 1, the texture shifts at 143.00-149.50 / 1: every base scene with
/// each of the first three fixes (12 ways).
pub const FIX_TEX_SCENES: [Scene; 12] = [
    Scene::Fix(0, 0),
    Scene::Fix(1, 0),
    Scene::Fix(2, 0),
    Scene::Fix(3, 0),
    Scene::Fix(0, 1),
    Scene::Fix(1, 1),
    Scene::Fix(2, 1),
    Scene::Fix(3, 1),
    Scene::Fix(0, 2),
    Scene::Fix(1, 2),
    Scene::Fix(2, 2),
    Scene::Fix(3, 2),
];
pub const FIX_TEX_WAYS: [(usize, u8); 12] = [
    (0, ROWS),
    (1, ROWS),
    (2, ROWS),
    (3, ROWS),
    (4, ROWS),
    (5, ROWS),
    (6, ROWS),
    (7, ROWS),
    (8, ROWS),
    (9, ROWS),
    (10, ROWS),
    (11, ROWS),
];
/// Part 1's clocks (13 MHz x M / N): 143.00, 146.25, 149.50; 30 rounds each.
pub const FIX_TEX_PLL: [(u32, u32); 3] = [(44, 4), (45, 4), (23, 2)];
pub const FIX_TEX_ROUNDS: u32 = 30;

/// Part 2, the corner at 139.75 / 1 (GC2Y's soak: 0.43% with one hidden
/// layer): GC2K's worst with one, two and four hidden layers and with one
/// and the busy layer.
pub const FIX_CORNER_SCENES: [Scene; 4] = [
    Scene::Fix(1, 2),
    Scene::Fix(1, 3),
    Scene::Fix(1, 4),
    Scene::Fix(1, 5),
];
pub const FIX_CORNER_WAYS: [(usize, u8); 4] = [(0, ROWS), (1, ROWS), (2, ROWS), (3, ROWS)];
/// 1,000 rounds a way: one hidden layer's 0.43% expects about 4.
pub const FIX_CORNER_ROUNDS: u32 = 1000;

/// Run GC2AA (the owner, 2026-10-25: "Look for a corner fix that costs
/// nothing at 139.75-149.50, then soak the winner"): GC2K's worst with the
/// clamped v shift and one hidden layer, and with none; GC2P's columns
/// reversed (40 wrong in rows against 1 there, at no cost) and reversed.
pub const FREE_SCENES: [Scene; 2] = [Scene::Fix(1, 2), Scene::Fix(1, 6)];
/// The ways, in order: one layer in rows (the control, 3 of 1,000 at
/// 139.75 in GC2Z), one layer columns reversed, none columns reversed, one
/// layer reversed.
pub const FREE_WAYS: [(usize, u8); 4] = [
    (0, ROWS),
    (0, COLUMNS_REVERSED),
    (1, COLUMNS_REVERSED),
    (0, REVERSED),
];
/// Which free ways win first when several are right at a clock: the new
/// ones before the control.
pub const FREE_PREFER: [usize; 4] = [1, 2, 3, 0];
/// The clocks (13 MHz x M / N) and rounds a way: 139.75 (1,000: the
/// control's 0.3% shows), 146.25 and 149.50 (500).
pub const FREE_PLL: [(u32, u32, u32); 3] = [(43, 4, 1000), (45, 4, 500), (23, 2, 500)];
/// The soak: the winner over every 16-bit kind and GC2K's worst, with one
/// hidden layer ([`FREE_SOAK_SCENES`]`[0]`) or none (`[1]`), for
/// [`FREE_SOAK_MINUTES`].
pub const FREE_SOAK_SCENES: [[Scene; 7]; 2] = [
    [
        Scene::Fix(0, 2),
        Scene::Fix(1, 2),
        Scene::Fix(2, 2),
        Scene::Fix(3, 2),
        Scene::Fix(4, 2),
        Scene::Fix(5, 2),
        Scene::Fix(6, 2),
    ],
    [
        Scene::Fix(0, 6),
        Scene::Fix(1, 6),
        Scene::Fix(2, 6),
        Scene::Fix(3, 6),
        Scene::Fix(4, 6),
        Scene::Fix(5, 6),
        Scene::Fix(6, 6),
    ],
];
/// The soak's ways in each tile order (rows, reversed, columns reversed).
pub const FREE_SOAK_WAYS: [[(usize, u8); 7]; 3] = [
    [
        (0, ROWS),
        (1, ROWS),
        (2, ROWS),
        (3, ROWS),
        (4, ROWS),
        (5, ROWS),
        (6, ROWS),
    ],
    [
        (0, REVERSED),
        (1, REVERSED),
        (2, REVERSED),
        (3, REVERSED),
        (4, REVERSED),
        (5, REVERSED),
        (6, REVERSED),
    ],
    [
        (0, COLUMNS_REVERSED),
        (1, COLUMNS_REVERSED),
        (2, COLUMNS_REVERSED),
        (3, COLUMNS_REVERSED),
        (4, COLUMNS_REVERSED),
        (5, COLUMNS_REVERSED),
        (6, COLUMNS_REVERSED),
    ],
];
pub const FREE_SOAK_MINUTES: u32 = 60;

/// Run GC2AB (the owner, 2026-10-25: "Build the short last-pixel cure test
/// at 149.50"): GC2AA's soak left one wrong pixel in 30,219 renders, the
/// last pixel of a tile in kind 0 (plain Gouraud), too rare for 15 minutes
/// to rank cures; so one clock step up, 152.75 / 1, where the race should
/// be commoner, falling back to 149.50 if 152.75 doesn't run. All in
/// reversed order (GC2AA's winner): kinds 0 and 1 with no hidden layer, one
/// and two; GC2K's worst with one and two (the second layer's cost there).
pub const LASTPIX_SCENES: [Scene; 8] = [
    Scene::Fix(4, 6),
    Scene::Fix(4, 2),
    Scene::Fix(4, 3),
    Scene::Fix(5, 6),
    Scene::Fix(5, 2),
    Scene::Fix(5, 3),
    Scene::Fix(1, 2),
    Scene::Fix(1, 3),
];
pub const LASTPIX_WAYS: [(usize, u8); 8] = [
    (0, REVERSED),
    (1, REVERSED),
    (2, REVERSED),
    (3, REVERSED),
    (4, REVERSED),
    (5, REVERSED),
    (6, REVERSED),
    (7, REVERSED),
];
/// The clock (13 MHz x 47 / 4 = 152.75) and the fallback (149.50), and the
/// rounds: 800 x 8 ways, about 13 minutes.
pub const LASTPIX_PLL: [(u32, u32); 2] = [(47, 4), (23, 2)];
pub const LASTPIX_ROUNDS: u32 = 800;

/// Run GC2AC (the owner, 2026-10-25: "build GC2AC, the ring cure for
/// GC2K's rare corner pixel at 149.50 / 1"): GC2K's worst at 149.50 / 1,
/// the clamped v shift throughout, with no cure, one full hidden layer
/// (GC2AB's free one), the ring alone, the ring over one full layer, and
/// two full layers (GC2AB: 16-20% dearer); all in rows (the order the
/// patch shows most in, so the cures are told apart), then one full layer
/// and the ring over it reversed (GC2AA's order). Each way's fastest
/// render is the cure's cost.
pub const RING_SCENES: [Scene; 5] = [
    Scene::Fix(1, 6),
    Scene::Fix(1, 2),
    Scene::Fix(1, 7),
    Scene::Fix(1, 8),
    Scene::Fix(1, 3),
];
pub const RING_WAYS: [(usize, u8); 7] = [
    (0, ROWS),
    (1, ROWS),
    (2, ROWS),
    (3, ROWS),
    (4, ROWS),
    (1, REVERSED),
    (3, REVERSED),
];
/// 13 MHz x 23 / 2 = 149.50, GC2AB's top; 500 rounds x 7 ways (3,500
/// renders, about 7 minutes).
pub const RING_PLL: (u32, u32) = (23, 2);
pub const RING_ROUNDS: u32 = 500;

/// Run GC2AD, reminder R36 (the owner, 2026-10-25: "Build R36 as the 2-hour
/// soak at 149.50 / 1 over every 16-bit kind and GC2K's worst, alternating
/// the ring reversed and one full layer reversed"; then "combine everything
/// for the soak"): each of [`FIX_BASE`]'s seven scenes with the ring (fix
/// 7), then with one full hidden layer (fix 2), and the four textured ones
/// (FT2, GC2K's worst, kinds 5 and 6) with both at once (fix 8, GC2AC: 0 of
/// 500 reversed), the clamped v shift in all; every way reversed, taken in
/// turn, so the cures alternate render by render under the same heat. The
/// ring rings textured objects only: kinds 0, 1 and 7 have none, so their
/// ring ways are drawn with no cure (GC2AB: 0 of 800 each so at 149.50) and
/// both at once would be their full-layer way again.
pub const NIGHT_SCENES: [Scene; 18] = {
    let mut s = [Scene::Fix(0, 0); 18];
    let (mut b, mut n) = (0, 0);
    while b < 7 {
        s[n] = Scene::Fix(b, 7);
        s[n + 1] = Scene::Fix(b, 2);
        n += 2;
        if b < NIGHT_TEXTURED {
            s[n] = Scene::Fix(b, 8);
            n += 1;
        }
        b += 1;
    }
    s
};
/// [`FIX_BASE`]'s first four scenes have textured objects (the ring's
/// work); the last three none.
pub const NIGHT_TEXTURED: usize = 4;
pub const NIGHT_WAYS: [(usize, u8); 18] = {
    let mut w = [(0, REVERSED); 18];
    let mut i = 0;
    while i < 18 {
        w[i] = (i, REVERSED);
        i += 1;
    }
    w
};
/// Run GC2AE, reminder R69 (the owner, 2026-10-25: "do everything that
/// you recommend to give us the clearest picture"): R36's fault generator,
/// FT2 with the ring and one full layer at once (160 of 3,299 wrong at
/// 149.50 / 1 reversed), traced and varied.
pub const TRACE_SCENES: [Scene; 1] = [Scene::Fix(0, 8)];
pub const TRACE_REVERSED: [(usize, u8); 1] = [(0, REVERSED)];
pub const TRACE_ROWS: [(usize, u8); 1] = [(0, ROWS)];
/// Its clocks below 149.50 / 1 (13 MHz x m / n): 146.25, 139.75, 133.25.
pub const TRACE_STEPS: [((u32, u32), &str); 3] = [
    ((45, 4), "146.25 / 1"),
    ((43, 4), "139.75 / 1"),
    ((41, 4), "133.25 / 1"),
];
/// R36's winning cure, one full hidden layer reversed with the clamped v
/// shift, on all seven scenes at 146.25 / 1: the step the owner's X51v
/// would run under ADR-0035 rule 1 (one below its fastest pass).
pub const MARGIN_SCENES: [Scene; 7] = {
    let mut s = [Scene::Fix(0, 2); 7];
    let mut b = 0;
    while b < 7 {
        s[b] = Scene::Fix(b, 2);
        b += 1;
    }
    s
};
pub const MARGIN_WAYS: [(usize, u8); 7] = {
    let mut w = [(0, REVERSED); 7];
    let mut i = 0;
    while i < 7 {
        w[i] = (i, REVERSED);
        i += 1;
    }
    w
};
/// Renders a part: the trace, each condition, the margin's rounds.
pub const TRACE_RENDERS: u32 = 600;
pub const TRACE_CONDITION: u32 = 400;
pub const MARGIN_ROUNDS: u32 = 300;

/// 149.50 / 1 for two hours (the owner asleep).
pub const NIGHT_PLL: (u32, u32) = (23, 2);
pub const NIGHT_MINUTES: u32 = 120;

/// `d`'s texture coordinates moved half a texel as `how` says (none for an
/// untextured object, or a wrapping texture under the clamped-only fixes).
#[must_use]
pub fn shift_texture(d: Draw, how: TexFix) -> Draw {
    let Paint::Texture { tex, uv, tint } = d.paint else {
        return d;
    };
    let clamped =
        u32::from(tex.u_mode) == address::CLAMP && u32::from(tex.v_mode) == address::CLAMP;
    if matches!(how, TexFix::ClampUv | TexFix::ClampV) && !clamped {
        return d;
    }
    let half = UV_ONE / (2 * (8 << tex.code));
    let du = if matches!(how, TexFix::ClampUv) {
        half
    } else {
        0
    };
    Draw {
        paint: Paint::Texture {
            tex,
            uv: uv.map(|(u, v)| (u + du, v + half)),
            tint,
        },
        ..d
    }
}

/// How far a ring's bands reach each side of a textured object's edges:
/// one tile across (tiles are 8 x 16) twice over, one tile row down.
pub const RING_REACH: i32 = 16;

/// Run GC2AC's ring round `d` (the owner, 2026-10-25, after GC2AB: one full
/// hidden layer is free at 149.50 / 1 but a second costs 16-20%): four
/// opaque [`HIDDEN_LAYER`] bands over the tiles along `d`'s bounding box's
/// edges, [`RING_REACH`] pixels each side, inside the screen. GC2K's patch
/// sits by the textured quad's corner, so the ring puts the extra object
/// only in the tiles that need it, where a full layer puts one in every
/// tile. The top and bottom bands span the whole width with the corners;
/// the side bands stop where they start, so no tile gets two.
#[must_use]
pub fn ring(d: &Draw) -> [Draw; 4] {
    ring_with(d, RING_REACH)
}

/// [`ring`] with its bands `reach` pixels each side of the edges (run
/// GC2AH: 12 puts the bands' outer edges mid-tile, 16 on the tile seams for
/// FT2's quad).
#[must_use]
pub fn ring_with(d: &Draw, reach: i32) -> [Draw; 4] {
    use rp_mbx3d_render::SUB;
    let span = |v: [i32; 4]| {
        v.iter()
            .fold((i32::MAX, i32::MIN), |(lo, hi), &a| (lo.min(a), hi.max(a)))
    };
    let (x0, x1) = span(d.corners.map(|p| p.x.div_euclid(SUB)));
    let (y0, y1) = span(d.corners.map(|p| p.y.div_euclid(SUB)));
    let (w, h) = (W as i32, H as i32);
    let band = |l: i32, t: i32, r: i32, b: i32| {
        let (l, t) = (l.clamp(0, w), t.clamp(0, h));
        let (r, b) = (r.clamp(l, w), b.clamp(t, h));
        Draw::rect(l, t, r - l, b - t, Paint::Flat(HIDDEN_LAYER))
    };
    let (l, r) = (x0 - reach, x1 + reach);
    let (top, bottom) = (y0 + reach, y1 - reach);
    [
        band(l, y0 - reach, r, top),
        band(l, bottom, r, y1 + reach),
        band(l, top, x0 + reach, bottom),
        band(x1 - reach, top, r, bottom),
    ]
}

/// [`FIX_BASE`]'s scene `b` with fix `f`: the hidden layers first, the
/// ring round every textured object, the scene's own objects (textures
/// shifted), then the busy layer.
#[must_use]
pub fn fix_scene(b: usize, f: usize, t: Tex) -> (u32, [Option<Draw>; SLOTS]) {
    let fx = FIXES[f];
    let (bg, slots) = order_scene(FIX_BASE[b], t);
    let mut out = [None; SLOTS];
    let mut n = 0;
    for _ in 0..fx.hidden {
        out[n] = Some(flat(HIDDEN_LAYER, false));
        n += 1;
    }
    if fx.ring {
        for d in slots.iter().flatten() {
            if matches!(d.paint, Paint::Texture { .. }) {
                for band in ring(d) {
                    out[n] = Some(band);
                    n += 1;
                }
            }
        }
    }
    for d in slots.into_iter().flatten() {
        out[n] = Some(shift_texture(d, fx.tex));
        n += 1;
    }
    if fx.busy {
        out[n] = Some(flat(BUSY_LAYER, true));
    }
    (bg, out)
}

/// Scene `s` in [`SLOTS`] slots: [`order_scene`]'s three widened, or a
/// GC2Z fix's.
#[must_use]
#[inline(never)]
pub fn order_scene_wide(s: Scene, t: Tex) -> (u32, [Option<Draw>; SLOTS]) {
    if let Scene::Split10(cols, rows) = s {
        return split10_scene(cols, rows, t);
    }
    if let Scene::Fix(b, f) = s {
        return fix_scene(b, f, t);
    }
    if let Scene::Geo(w) = s {
        return geo_scene(w, t);
    }
    if let Scene::Quad(w) = s {
        return quad_scene(w, t);
    }
    if let Scene::Corner(textured, i) = s {
        return corner_scene(textured, i, t);
    }
    if let Scene::CornerShade(i, m) = s {
        return corner_shade_scene(i, m, t);
    }
    if let Scene::Corner3(m) = s {
        return corner3_scene(m, t);
    }
    if let Scene::Corner3At(dx, dy) = s {
        return corner3_at(dx, dy, t);
    }
    if let Scene::CornerSeal(i, _) = s {
        return corner_scene(false, i, t);
    }
    if let Scene::CornerNoLayer(i) = s {
        let (bg, mut slots) = corner_scene(false, i, t);
        slots[0] = None;
        return (bg, slots);
    }
    if let Scene::CornerUv(i) = s {
        return corner_uv_scene(i, t);
    }
    if let Scene::Cure(c, i) = s {
        return cure_scene(c, i, t);
    }
    if let Scene::Phase(n) = s {
        return phase_scene(n, t);
    }
    if let Scene::Ratio(r, i) = s {
        return ratio_scene(r, i, t);
    }
    if let Scene::Proof(layer, textured, i) = s {
        return proof_scene(layer, textured, i, t);
    }
    let (bg, slots) = order_scene(s, t);
    let mut out = [None; SLOTS];
    out[..3].copy_from_slice(&slots);
    (bg, out)
}

/// Run GC2Y's scenes to cure (the owner, 2026-10-25: "Build the
/// combined-fix test: both cures together at 133.25-149.50, then a soak"):
/// GC2's six 16-bit kinds and GC2K's worst.
pub const CURED_BASE: [Scene; 7] = [
    Scene::Kind(0),
    Scene::Kind(1),
    Scene::Kind(FT2),
    Scene::Kind(5),
    Scene::Kind(6),
    Scene::Kind(7),
    Scene::Sweep(0),
];

/// Both of GC2V's cures on a scene: every texture sampled at its texels'
/// centres (its coordinates moved half a texel in u and v: GC2V's top-row
/// fault, cleared at 143.00 and 146.25 by the v half), and [`HIDDEN_LAYER`]
/// drawn first under it (GC2K's patch at 133.25, 9 of 200 to 0). `None`
/// when the scene has no free slot for the layer.
#[must_use]
#[inline(never)]
pub fn cure((bg, slots): (u32, [Option<Draw>; 3])) -> Option<(u32, [Option<Draw>; 3])> {
    if slots[2].is_some() {
        return None;
    }
    let centre = |d: Option<Draw>| {
        d.map(|mut d| {
            if let Paint::Texture { tex, uv, tint } = d.paint {
                let half = UV_ONE / (2 * (8 << tex.code));
                d.paint = Paint::Texture {
                    tex,
                    uv: uv.map(|(u, v)| (u + half, v + half)),
                    tint,
                };
            }
            d
        })
    };
    Some((
        bg,
        [
            Some(flat(HIDDEN_LAYER, false)),
            centre(slots[0]),
            centre(slots[1]),
        ],
    ))
}

/// GC2Y's ways: the seven cured scenes, then FT2 and GC2K's worst uncured
/// (the controls: wrong at their clocks in GC2T-GC2V).
pub const CURED_SCENES: [Scene; 9] = [
    Scene::Cured(0),
    Scene::Cured(1),
    Scene::Cured(2),
    Scene::Cured(3),
    Scene::Cured(4),
    Scene::Cured(5),
    Scene::Cured(6),
    Scene::Kind(FT2),
    Scene::Sweep(0),
];
pub const CURED_WAYS: [(usize, u8); 9] = [
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
/// The soak's ways: the cured scenes only.
pub const CURED_SOAK_WAYS: [(usize, u8); 7] = [
    (0, ROWS),
    (1, ROWS),
    (2, ROWS),
    (3, ROWS),
    (4, ROWS),
    (5, ROWS),
    (6, ROWS),
];
/// The climb's clocks (13 MHz x M / N): 133.25, 139.75, 143.00, 146.25,
/// 149.50; 100 rounds at the first, 30 above.
pub const CURED_PLL: [(u32, u32); 5] = [(41, 4), (43, 4), (44, 4), (45, 4), (23, 2)];
pub const CURED_ROUNDS: [u32; 5] = [100, 30, 30, 30, 30];
/// The soak: at the highest clock where every cured way was right, this
/// many minutes (the owner asleep: a long soak costs no one's time).
pub const CURED_SOAK_MINUTES: u32 = 120;

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
/// Run GC2AN's orders (rp-mbx3d-render's `TileOrder::Columns`, `Checker`,
/// `Scrambled`).
pub const COLUMNS: u8 = 3;
pub const CHECKER: u8 = 4;
pub const SCRAMBLED: u8 = 5;
/// Run GC2AO's order (`TileOrder::Interleaved`).
pub const INTERLEAVED: u8 = 6;

/// Run GC2AN (the owner, 2026-10-26: "i want to go for broke, i wont let go
/// of this till we crack 150", then "150 fast lossless"): GC2AG's
/// untextured fault generator (FT2's scene with the ring over one full
/// layer, the quad flat: 17 of 400 wrong at 149.50 / 1 reversed, the tile
/// race alone) in six tile orders. GC2AL: rows 120 against reversed 20 of
/// 2,200, so the order matters; the new orders keep tiles drawn one after
/// another apart in memory.
pub const ORDER_TRY: [(usize, u8); 6] = [
    (0, REVERSED),
    (0, ROWS),
    (0, COLUMNS_REVERSED),
    (0, COLUMNS),
    (0, CHECKER),
    (0, SCRAMBLED),
];
pub const ORDER_TRY_SCENES: [Scene; 1] = [Scene::Fix(0, 8)];
pub const ORDER_TRY_ROUNDS: u32 = 400;

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
#[inline(never)]
pub fn order_scene(s: Scene, t: Tex) -> (u32, [Option<Draw>; 3]) {
    match s {
        Scene::Sweep(w) => sweep_scene(w, t),
        Scene::Kind(k) => scene(k, t),
        Scene::Magnified(m) => magnified_scene(m, t),
        Scene::Upsampled10(code) => {
            let side = 8i32 << code;
            let (a, b) = (UV_ONE / (2 * side), UV_ONE - UV_ONE / (2 * side));
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
        Scene::Shifted10(du, dv) => {
            let (bg, slots) = scene(10, t);
            let slots = slots.map(|d| {
                d.map(|d| match d.paint {
                    Paint::Texture { tex, uv, tint } => Draw {
                        paint: Paint::Texture {
                            tex,
                            uv: uv.map(|(u, v)| (u + du, v + dv)),
                            tint,
                        },
                        ..d
                    },
                    _ => d,
                })
            });
            (bg, slots)
        }
        Scene::KindUv(k) => {
            let (bg, slots) = scene(k, t);
            (
                bg,
                slots.map(|d| d.map(|d| shift_texture(d, TexFix::ClampUv))),
            )
        }
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
        // Fixes and geometry ways take more than three slots:
        // [`order_scene_wide`] only.
        Scene::Fix(..)
        | Scene::Geo(_)
        | Scene::Quad(_)
        | Scene::Corner(..)
        | Scene::CornerNoLayer(_)
        | Scene::CornerShade(..)
        | Scene::Corner3(_)
        | Scene::Corner3At(..)
        | Scene::CornerSeal(..)
        | Scene::Split10(..)
        | Scene::CornerUv(_)
        | Scene::Cure(..)
        | Scene::Phase(_)
        | Scene::Ratio(..)
        | Scene::Proof(..) => (BLACK, [None, None, None]),
        Scene::Cured(i) => match CURED_BASE[i] {
            Scene::Cured(_) => (BLACK, [None, None, None]),
            base => cure(order_scene(base, t)).unwrap_or((BLACK, [None, None, None])),
        },
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
        Scene::Geo(_) | Scene::Quad(_) => patch_corner(0),
        Scene::Corner(..)
        | Scene::CornerNoLayer(_)
        | Scene::CornerShade(..)
        | Scene::Corner3(_)
        | Scene::Corner3At(..)
        | Scene::CornerSeal(..)
        | Scene::CornerUv(_)
        | Scene::Cure(..)
        | Scene::Ratio(..)
        | Scene::Proof(..) => (0, 0),
        Scene::Phase(_) => (PHASE_RECT.0, PHASE_RECT.1),
        Scene::Fix(b, _) => match FIX_BASE[b] {
            Scene::Kind(FT2) => patch_corner(0),
            Scene::Sweep(w) => SWEEP[w].corner(),
            _ => (0, 0),
        },
        Scene::Cured(i) => match CURED_BASE[i] {
            Scene::Kind(FT2) => patch_corner(0),
            Scene::Sweep(w) => SWEEP[w].corner(),
            _ => (0, 0),
        },
        Scene::Kind(_)
        | Scene::KindUv(_)
        | Scene::Magnified(_)
        | Scene::Shifted10(..)
        | Scene::Split10(..)
        | Scene::Upsampled10(_) => (0, 0),
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

/// The 3D core's tile: 8 x 16 pixels (`rp-mbx3d`'s `TILE_W`, `TILE_H`;
/// the image holds the two equal at build time).
pub const TILE_W: i32 = 8;
pub const TILE_H: i32 = 16;

/// Pixel (x, y) of a 480 x 640 picture in `f` held as words (the 16-bit
/// formats two a word, the left one in the low half); `None` off the
/// picture or past `pic`.
#[must_use]
pub fn pixel(f: PixelFormat, pic: &[u32], x: i32, y: i32) -> Option<u32> {
    if !(0..W as i32).contains(&x) || !(0..H as i32).contains(&y) {
        return None;
    }
    let at = y as usize * W as usize + x as usize;
    if f.bytes() == 4 {
        pic.get(at).copied()
    } else {
        pic.get(at / 2).map(|w| w >> (16 * (at % 2)) & 0xFFFF)
    }
}

/// The pixel sharing (x, y)'s word in a 16-bit picture (its pair), or
/// (x, y) itself in a 32-bit one.
#[must_use]
pub const fn pair(f: PixelFormat, x: i32, y: i32) -> (i32, i32) {
    if f.bytes() == 4 { (x, y) } else { (x ^ 1, y) }
}

/// (x, y)'s place in its tile: column and row, (7, 15) the pixel a tile
/// writes out last.
#[must_use]
pub const fn in_tile(x: i32, y: i32) -> (i32, i32) {
    (x.rem_euclid(TILE_W), y.rem_euclid(TILE_H))
}

/// The same place in the four tiles around (x, y)'s: left, right, above,
/// below. A wrong pixel equal to one of these in the reference is a
/// neighbouring tile's pixel left in the core's tile buffer (the last-pixel
/// fault's candidate (b), 2700g.md "The last-pixel fault's candidates").
#[must_use]
pub const fn tile_mates(x: i32, y: i32) -> [(i32, i32); 4] {
    [
        (x - TILE_W, y),
        (x + TILE_W, y),
        (x, y - TILE_H),
        (x, y + TILE_H),
    ]
}

/// Where value `v` lies in picture `pic` nearest (x, y), within `r` (dx,
/// dy) either side, (x, y) itself left out: the offset (fewest steps, then
/// the first in row order) and how many places in the window hold it. Run
/// GC2AE: where a wrong pixel's colour came from.
#[must_use]
pub fn nearest(
    f: PixelFormat,
    pic: &[u32],
    (x, y): (i32, i32),
    v: u32,
    (rx, ry): (i32, i32),
) -> (Option<(i32, i32)>, u32) {
    let (mut best, mut n) = (None::<(i32, i32)>, 0);
    for dy in -ry..=ry {
        for dx in -rx..=rx {
            if (dx, dy) == (0, 0) || pixel(f, pic, x + dx, y + dy) != Some(v) {
                continue;
            }
            n += 1;
            let d = dx.abs() + dy.abs();
            if best.is_none_or(|(bx, by)| d < bx.abs() + by.abs()) {
                best = Some((dx, dy));
            }
        }
    }
    (best, n)
}

/// How many pixels of `pic` hold `v`.
#[must_use]
pub fn count(f: PixelFormat, pic: &[u32], v: u32) -> u32 {
    let mut n = 0;
    for y in 0..H as i32 {
        for x in 0..W as i32 {
            n += u32::from(pixel(f, pic, x, y) == Some(v));
        }
    }
    n
}

/// Run GC2AG's untextured way: every textured object of `slots` drawn in
/// one flat opaque colour (mid grey) instead, the rest as they are, so the
/// 3D core fetches no texture.
#[must_use]
pub fn untextured<const N: usize>(slots: [Option<Draw>; N]) -> [Option<Draw>; N] {
    slots.map(|d| {
        d.map(|d| match d.paint {
            Paint::Texture { .. } => Draw {
                paint: Paint::Flat(UNTEXTURED),
                ..d
            },
            _ => d,
        })
    })
}

/// [`untextured`]'s colour (mid grey).
pub const UNTEXTURED: u32 = 0xFF80_8080;

/// One of run GC2AH's geometry ways: FT2's scene with its textured quad
/// taken out (no texture: GC2AG's fault stayed without one), `layers` full
/// hidden layers first, then the ring of the quad's outline `reach` pixels
/// each side (0: none), then FT2's other objects (the Gouraud background
/// over them all, so none is seen).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GeoWay {
    pub layers: usize,
    pub reach: i32,
}

/// Run GC2AH (the owner, 2026-10-26: "Build the geometry test next (which
/// stacked shapes in one tile trigger it, and why the full layer alone
/// stays clean)"). The hypothesis from GC2AE-GC2AG: the core shades a
/// tile's last rows wrong when the tile it draws follows one with fewer
/// objects (the faults sat in the bands' first tile column in draw order:
/// x 448-455 reversed, x 24-31 in rows), and one full layer alone is clean
/// because every tile then holds the same count. Its predictions: way 0
/// faults at x 448-455, in rows order at x 24-31; way 1 (a step of one) no
/// more than way 0; way 2 (a step of one over two layers) as way 0 or more;
/// way 3 (two layers, no step) none; way 4 (the band edges mid-tile: a
/// partly covered tile between) tells whether a seam-aligned edge matters.
pub const GEO: [GeoWay; 5] = [
    GeoWay {
        layers: 1,
        reach: RING_REACH,
    },
    GeoWay {
        layers: 0,
        reach: RING_REACH,
    },
    GeoWay {
        layers: 2,
        reach: RING_REACH,
    },
    GeoWay {
        layers: 2,
        reach: 0,
    },
    GeoWay {
        layers: 1,
        reach: 12,
    },
];
pub const GEO_SCENES: [Scene; 5] = [
    Scene::Geo(0),
    Scene::Geo(1),
    Scene::Geo(2),
    Scene::Geo(3),
    Scene::Geo(4),
];
/// Way 0 in both orders (the placement prediction), the rest reversed.
pub const GEO_WAYS: [(usize, u8); 6] = [
    (0, REVERSED),
    (0, ROWS),
    (1, REVERSED),
    (2, REVERSED),
    (3, REVERSED),
    (4, REVERSED),
];
pub const GEO_ROUNDS: u32 = 400;

/// [`GEO`]'s way `w` (see [`GeoWay`]).
#[must_use]
pub fn geo_scene(w: usize, t: Tex) -> (u32, [Option<Draw>; SLOTS]) {
    let gw = GEO[w];
    let (bg, slots) = order_scene(FIX_BASE[0], t);
    let mut out = [None; SLOTS];
    let mut n = 0;
    for _ in 0..gw.layers {
        out[n] = Some(flat(HIDDEN_LAYER, false));
        n += 1;
    }
    for d in slots.iter().flatten() {
        if gw.reach != 0 && matches!(d.paint, Paint::Texture { .. }) {
            for band in ring_with(d, gw.reach) {
                out[n] = Some(band);
                n += 1;
            }
        }
    }
    for d in slots.into_iter().flatten() {
        if !matches!(d.paint, Paint::Texture { .. }) {
            out[n] = Some(d);
            n += 1;
        }
    }
    (bg, out)
}

/// One of run GC2AI's ways: GC2AG's untextured fault generator (one full
/// layer, then the ring of FT2's quad at reach 16, then FT2's Gouraud
/// background, then the quad flat grey on top) with the flat quad at
/// `rect` (x0, y0, x1, y1, pixels) instead of FT2's (40, 40, 440, 600); the
/// ring stays where FT2's quad puts it. `first`: the quad drawn before
/// everything instead (under the Gouraud background, so never seen).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuadWay {
    pub rect: (i32, i32, i32, i32),
    pub first: bool,
}

/// Run GC2AI (the owner, 2026-10-26: "Build the next test: the rectangle
/// back, flat, its edges and corners moved one at a time against the
/// stacked tile"). GC2AH: with FT2's quad taken out the fault went (0 of
/// 2,400); GC2AG's untextured way, the quad flat, had 17 of 400 at x
/// 449-455, y 45-47 (the tile x 448-455, y 32-47: the ring's top band and
/// the layer, no part of the quad). Each way moves one of the quad's edges
/// or its order: (0) as GC2AG; (1) its top edge on the tile seam y 32; (2)
/// its top edge at y 44, mid-tile; (3) its right edge at x 436, mid-tile;
/// (4) at x 448, the seam beside the fault's column; (5) at x 456, over the
/// fault's column; (6) at x 400, far from it; (7) drawn first, hidden.
pub const QUAD_WAYS: [QuadWay; 8] = [
    QuadWay {
        rect: (40, 40, 440, 600),
        first: false,
    },
    QuadWay {
        rect: (40, 32, 440, 600),
        first: false,
    },
    QuadWay {
        rect: (40, 44, 440, 600),
        first: false,
    },
    QuadWay {
        rect: (40, 40, 436, 600),
        first: false,
    },
    QuadWay {
        rect: (40, 40, 448, 600),
        first: false,
    },
    QuadWay {
        rect: (40, 40, 456, 600),
        first: false,
    },
    QuadWay {
        rect: (40, 40, 400, 600),
        first: false,
    },
    QuadWay {
        rect: (40, 40, 440, 600),
        first: true,
    },
];
pub const QUAD_SCENES: [Scene; 8] = {
    let mut s = [Scene::Quad(0); 8];
    let mut i = 0;
    while i < 8 {
        s[i] = Scene::Quad(i);
        i += 1;
    }
    s
};
pub const QUAD_ORDER: [(usize, u8); 8] = {
    let mut w = [(0, REVERSED); 8];
    let mut i = 0;
    while i < 8 {
        w[i] = (i, REVERSED);
        i += 1;
    }
    w
};
pub const QUAD_ROUNDS: u32 = 400;

/// [`QUAD_WAYS`]' way `w` (see [`QuadWay`]).
#[must_use]
pub fn quad_scene(w: usize, t: Tex) -> (u32, [Option<Draw>; SLOTS]) {
    let qw = QUAD_WAYS[w];
    let (bg, slots) = order_scene(FIX_BASE[0], t);
    let (x0, y0, x1, y1) = qw.rect;
    let quad = Draw::rect(x0, y0, x1 - x0, y1 - y0, Paint::Flat(UNTEXTURED));
    let mut out = [None; SLOTS];
    let mut n = 0;
    if qw.first {
        out[n] = Some(quad);
        n += 1;
    }
    out[n] = Some(flat(HIDDEN_LAYER, false));
    n += 1;
    for d in slots.iter().flatten() {
        if matches!(d.paint, Paint::Texture { .. }) {
            for band in ring(d) {
                out[n] = Some(band);
                n += 1;
            }
        }
    }
    for d in slots.into_iter().flatten() {
        if matches!(d.paint, Paint::Texture { .. }) {
            if !qw.first {
                out[n] = Some(quad);
                n += 1;
            }
        } else {
            out[n] = Some(d);
            n += 1;
        }
    }
    (bg, out)
}

/// Run GC2AJ (the owner, 2026-10-26: "Build the corner sweep: the
/// rectangle's corner moved over every position around a tile, with one
/// full layer alone (the recommended fix), and run it"). GC2AI tied the
/// fault to exact corner places, so the cure's clean renders over seven
/// scenes don't cover every geometry. Each scene: one full hidden layer,
/// FT2's Gouraud background, then [`CORNER_RECTS`] rectangles (flat grey,
/// or FT2's 8888 texture with the clamped v shift) in a 2 x 3 grid, every
/// corner of rectangle `k` at the same place in its tile: x offset `k % 8`,
/// y offset `k / 8` (all 128 places of an 8 x 16 tile over the scenes).
pub const CORNER_RECTS: usize = 6;
/// Scenes for the 128 places, six a scene (the last repeats the first
/// places).
pub const CORNER_SCENES_EACH: usize = 128_usize.div_ceil(CORNER_RECTS);
/// Rounds a scene: 200 (a place faulting 1.5% of renders shows with ~95%).
pub const CORNER_ROUNDS: u32 = 200;

/// Rectangle `j` of corner scene `i`: (x0, y0, x1, y1) pixels, and its
/// place's index `k` (0..128).
#[must_use]
pub const fn corner_rect(i: usize, j: usize) -> ((i32, i32, i32, i32), usize) {
    // Cells of 240 x 208 pixels, their origins on tile seams (x a multiple
    // of 8, y of 16), so each edge's offset into its tile is `a` or `c`.
    corner_rect_of(CORNER_RECTS, i, j)
}

/// Corner scene `i`, flat or textured (see [`CORNER_RECTS`]).
#[must_use]
pub fn corner_scene(textured: bool, i: usize, t: Tex) -> (u32, [Option<Draw>; SLOTS]) {
    corner_scene_with(textured, i, t, TexFix::ClampV)
}

/// Corner scene `i`, textured, the clamped texture shifted half a texel in
/// u and v (run GC2AK).
#[must_use]
pub fn corner_uv_scene(i: usize, t: Tex) -> (u32, [Option<Draw>; SLOTS]) {
    corner_scene_with(true, i, t, TexFix::ClampUv)
}

fn corner_scene_with(
    textured: bool,
    i: usize,
    t: Tex,
    shift: TexFix,
) -> (u32, [Option<Draw>; SLOTS]) {
    let (bg, slots) = order_scene(FIX_BASE[0], t);
    let mut out = [None; SLOTS];
    out[0] = Some(flat(HIDDEN_LAYER, false));
    let mut n = 1;
    for d in slots.iter().flatten() {
        if !matches!(d.paint, Paint::Texture { .. }) {
            out[n] = Some(*d);
            n += 1;
        }
    }
    for j in 0..CORNER_RECTS {
        let ((x0, y0, x1, y1), _) = corner_rect(i, j);
        let paint = if textured {
            Paint::texture(t.t8888)
        } else {
            Paint::Flat(UNTEXTURED)
        };
        let d = Draw::rect(x0, y0, x1 - x0, y1 - y0, paint);
        out[n] = Some(shift_texture(d, shift));
        n += 1;
    }
    (bg, out)
}

/// `slots` with every draw moved `dy` pixels down (up for a negative
/// `dy`): a scene's rows rendered into a shorter target (run GC2BD), the
/// tile-relative geometry kept when `dy` is a multiple of the tile's 16
/// rows.
#[must_use]
pub fn shifted<const N: usize>(mut slots: [Option<Draw>; N], dy: i32) -> [Option<Draw>; N] {
    for d in slots.iter_mut().flatten() {
        for p in &mut d.corners {
            p.y += dy * rp_mbx3d_render::SUB;
        }
    }
    slots
}

/// GC2's 16-bit kinds (every RGB565 target, which fits the safe region):
/// the proof run's content set (run GC2BG).
pub const KINDS16: [Scene; 8] = [
    Scene::Kind(0),
    Scene::Kind(1),
    Scene::Kind(FT2),
    Scene::Kind(5),
    Scene::Kind(6),
    Scene::Kind(7),
    Scene::Kind(9),
    Scene::Kind(10),
];

/// Every scene of an `N`-scene plan in the checker order (run GC2BG).
#[must_use]
pub const fn checker_ways<const N: usize>() -> [(usize, u8); N] {
    order_ways(CHECKER)
}

/// [`Scene::CornerShade`]'s bit: the Gouraud background drawn flat, each
/// shaded object in its first corner's colour (run GC2BR).
pub const SHADE_FLAT_BG: u8 = 1;
/// [`Scene::CornerShade`]'s bit: the grey rectangles drawn Gouraud with
/// four equal colours, the same picture through the shading path the
/// background takes (run GC2BR).
pub const SHADE_GOURAUD_RECT: u8 = 2;

/// Flat corner scene `i` with its shading changed as `m`'s bits say
/// ([`SHADE_FLAT_BG`], [`SHADE_GOURAUD_RECT`]; run GC2BR: GC2BP and GC2BQ
/// put every wrong pixel of scene 3 inside one tile's share of a grey
/// rectangle where the tile also holds the Gouraud background, the grey
/// one green step high).
#[must_use]
pub fn corner_shade_scene(i: usize, m: u8, t: Tex) -> (u32, [Option<Draw>; SLOTS]) {
    let (bg, mut slots) = corner_scene(false, i, t);
    for d in slots.iter_mut().flatten() {
        match d.paint {
            Paint::Gouraud(c) if m & SHADE_FLAT_BG != 0 => d.paint = Paint::Flat(c[0]),
            Paint::Flat(UNTEXTURED) if m & SHADE_GOURAUD_RECT != 0 => {
                d.paint = Paint::Gouraud([UNTEXTURED; 4]);
            }
            _ => {}
        }
    }
    (bg, slots)
}

/// [`Scene::Corner3`]: scene 3 as it is, without the hidden layer.
pub const S3_AS_IS: u8 = 0;
/// [`Scene::Corner3`]: the top-right rectangle (the only one that faulted
/// in GC2BP, GC2BQ and GC2BR) alone, the other five taken out.
pub const S3_ALONE: u8 = 1;
/// [`Scene::Corner3`]: the top-right rectangle alone, moved one cell
/// (240 pixels, 30 tiles) left: its edges keep their places in their
/// tiles; only the screen position changes.
pub const S3_ALONE_LEFT: u8 = 2;
/// [`Scene::Corner3`]: the top-right rectangle alone, moved one cell (208
/// pixels, 13 tile rows) down.
pub const S3_ALONE_DOWN: u8 = 3;
/// [`Scene::Corner3`]: the six rectangles drawn in the opposite order
/// (the lists built with the top-right one fifth, not second).
pub const S3_REVERSED: u8 = 4;
/// [`Scene::Corner3`]: the rectangles' grey `0x848484` instead of
/// `0x808080`: its RGB565 green bit 0 already set (`0x21`), so a bit
/// forced high can't show, while an arithmetic +1 step still does.
pub const S3_GREY84: u8 = 5;
/// GC2BS's grey (see [`S3_GREY84`]).
pub const UNTEXTURED84: u32 = 0xFF84_8484;
/// [`Scene::Corner3`]: every object with a depth test, "greater" with
/// writes, the layer at [`S3_Z_LAYER`], the background at [`S3_Z_BG`], the
/// rectangles at [`S3_Z_RECT`] (the same picture through the depth path a
/// game takes; the chase's scenes all took Dell's "always, no write";
/// run GC2BU).
pub const S3_DEPTH: u8 = 6;
/// [`Scene::Corner3`]: as [`S3_DEPTH`] with the compare "always" (the
/// depth written, never compared).
pub const S3_DEPTH_ALWAYS: u8 = 7;
/// [`Scene::Corner3`]: the rectangles moved half a pixel right and down
/// (their edges off the pixel grid; run GC2BU).
pub const S3_HALF: u8 = 8;
/// [`Scene::Corner3`]: the rectangles' corners rotated one place (top
/// right first), so the renderer's split joins top left to bottom right
/// instead of top right to bottom left; the same winding and picture (runs
/// GC2BU, GC2BV: GC2BU's form never reached `gc2_orders`, which builds its
/// own scenes, so it ran as is).
pub const S3_FLIP: u8 = 9;
/// [`Scene::Corner3`]'s flag: each rectangle's diagonal sealed by a band of
/// its own grey ([`seal_band`], in [`extra_slots`]; run GC2BV).
pub const S3_BAND: u8 = 0x40;
/// [`Scene::Corner3`]'s flag: each rectangle drawn a second time split the
/// other way ([`extra_slots`]; run GC2BV).
pub const S3_DOUBLE: u8 = 0x20;
/// [`seal_band`]'s reach along each edge from the corner (pixels): the
/// faults are four-pixel groups beside the diagonal (GC2BS-GC2BU), so the
/// band is wider than a group on each side.
pub const SEAL_REACH: i32 = 8;
/// [`Scene::Corner3`]'s flag: the hidden layer kept (the shipped form).
pub const S3_LAYER: u8 = 0x80;
/// GC2BU's depths (`z / 65536`, larger nearer).
pub const S3_Z_LAYER: u16 = 0x1000;
pub const S3_Z_BG: u16 = 0x3333;
pub const S3_Z_RECT: u16 = 0xCCCC;

/// Flat corner scene 3 without its hidden layer in form `m` (run GC2BS:
/// what selects the top-right rectangle, and whether the green step is a
/// forced bit).
#[must_use]
pub fn corner3_scene(m: u8, t: Tex) -> (u32, [Option<Draw>; SLOTS]) {
    let (bg, mut slots) = corner_scene(false, 3, t);
    let layer = m & S3_LAYER != 0;
    let m = m & !(S3_LAYER | S3_BAND | S3_DOUBLE);
    if !layer {
        slots[0] = None;
    }
    let rects: [usize; CORNER_RECTS] = {
        let mut r = [0; CORNER_RECTS];
        let mut n = 0;
        for (i, d) in slots.iter().enumerate() {
            if let Some(d) = d
                && d.paint == Paint::Flat(UNTEXTURED)
            {
                r[n] = i;
                n += 1;
            }
        }
        debug_assert_eq!(n, CORNER_RECTS);
        r
    };
    match m {
        S3_ALONE | S3_ALONE_LEFT | S3_ALONE_DOWN => {
            for (j, &i) in rects.iter().enumerate() {
                if j != 1 {
                    slots[i] = None;
                }
            }
            if let Some(d) = slots[rects[1]].as_mut() {
                for p in &mut d.corners {
                    if m == S3_ALONE_LEFT {
                        p.x -= 240 * rp_mbx3d_render::SUB;
                    } else if m == S3_ALONE_DOWN {
                        p.y += 208 * rp_mbx3d_render::SUB;
                    }
                }
            }
        }
        S3_REVERSED => {
            let drawn = rects.map(|i| slots[i]);
            for (j, &i) in rects.iter().enumerate() {
                slots[i] = drawn[CORNER_RECTS - 1 - j];
            }
        }
        S3_GREY84 => {
            for &i in &rects {
                if let Some(d) = slots[i].as_mut() {
                    d.paint = Paint::Flat(UNTEXTURED84);
                }
            }
        }
        S3_DEPTH | S3_DEPTH_ALWAYS => {
            let mode = if m == S3_DEPTH {
                rp_mbx3d_render::depth::GREATER
            } else {
                rp_mbx3d_render::depth::ALWAYS
            };
            for (i, d) in slots.iter_mut().enumerate() {
                if let Some(d) = d {
                    let z = if i == 0 {
                        S3_Z_LAYER
                    } else if d.paint == Paint::Flat(UNTEXTURED) {
                        S3_Z_RECT
                    } else {
                        S3_Z_BG
                    };
                    *d = d.with_depth(rp_mbx3d_render::Depth {
                        mode,
                        write: true,
                        z,
                    });
                }
            }
        }
        S3_FLIP => {
            for &i in &rects {
                if let Some(d) = slots[i].as_mut() {
                    d.corners.rotate_left(1);
                }
            }
        }
        S3_HALF => {
            for &i in &rects {
                if let Some(d) = slots[i].as_mut() {
                    for p in &mut d.corners {
                        p.x += rp_mbx3d_render::SUB / 2;
                        p.y += rp_mbx3d_render::SUB / 2;
                    }
                }
            }
        }
        _ => {}
    }
    (bg, slots)
}

/// Scene 3's top-right rectangle alone, without the layer, moved `(dx,
/// dy)` pixels (run GC2BU's position map: where on the screen the race
/// drops a group).
#[must_use]
pub fn corner3_at(dx: i16, dy: i16, t: Tex) -> (u32, [Option<Draw>; SLOTS]) {
    let (bg, mut slots) = corner3_scene(S3_ALONE, t);
    for d in slots.iter_mut().flatten() {
        if d.paint == Paint::Flat(UNTEXTURED) {
            for p in &mut d.corners {
                p.x += i32::from(dx) * rp_mbx3d_render::SUB;
                p.y += i32::from(dy) * rp_mbx3d_render::SUB;
            }
        }
    }
    (bg, slots)
}

/// The band that seals a flat rectangle's diagonal (run GC2BV): the
/// renderer splits a quad (top left, top right, bottom right, bottom left)
/// into (TL, TR, BL) and (TR, BR, BL), so the two meet on TR-BL; the band's
/// corners are [`SEAL_REACH`] pixels from TR and BL along the rectangle's
/// edges (left of TR, below TR, right of BL, above BL), so it straddles
/// that diagonal and stays inside the rectangle, in the rectangle's paint,
/// and its own split (B-D) crosses the diagonal only once, at the middle.
#[must_use]
pub fn seal_band(d: &Draw) -> Draw {
    let r = SEAL_REACH * rp_mbx3d_render::SUB;
    let [_, tr, _, bl] = d.corners;
    let mut b = *d;
    b.corners = [
        rp_mbx3d_render::Point {
            x: tr.x - r,
            y: tr.y,
        },
        rp_mbx3d_render::Point {
            x: tr.x,
            y: tr.y + r,
        },
        rp_mbx3d_render::Point {
            x: bl.x + r,
            y: bl.y,
        },
        rp_mbx3d_render::Point {
            x: bl.x,
            y: bl.y - r,
        },
    ];
    b
}

/// The draws scene `s` puts in the renderer's slots [`SLOTS`] up (the
/// scene's own are [`order_scene_wide`]'s): run GC2BV's seals, a band
/// ([`seal_band`]) or a copy split the other way for every flat grey
/// rectangle; nothing for every other scene.
#[must_use]
pub fn extra_slots(s: Scene, t: Tex) -> [Option<Draw>; SLOTS] {
    let mut out = [None; SLOTS];
    let seal = match s {
        Scene::Corner3(m) => m & (S3_BAND | S3_DOUBLE),
        Scene::CornerSeal(_, m) => m & (S3_BAND | S3_DOUBLE),
        _ => 0,
    };
    if seal == 0 {
        return out;
    }
    let (_, slots) = order_scene_wide(s, t);
    let mut n = 0;
    for d in slots.iter().flatten() {
        if n == SLOTS {
            break;
        }
        if matches!(d.paint, Paint::Flat(UNTEXTURED) | Paint::Flat(UNTEXTURED84)) {
            out[n] = Some(if seal & S3_BAND != 0 {
                seal_band(d)
            } else {
                let mut c = *d;
                c.corners.rotate_left(1);
                c
            });
            n += 1;
        }
    }
    out
}

/// GC2BV's forms of scene 3: without its layer as is, the other diagonal,
/// the band, the copy, as is again; with its layer as is and with the
/// band.
pub const SEAL_FORMS: [Scene; 7] = [
    Scene::Corner3(S3_AS_IS),
    Scene::Corner3(S3_FLIP),
    Scene::Corner3(S3_BAND),
    Scene::Corner3(S3_DOUBLE),
    Scene::Corner3(S3_AS_IS),
    Scene::Corner3(S3_LAYER),
    Scene::Corner3(S3_LAYER | S3_BAND),
];

const fn seal_sweep(seal: u8) -> [Scene; CORNER_SCENES_EACH + 1] {
    let mut s = CORNER_FLAT;
    let mut i = 0;
    while i < CORNER_SCENES_EACH {
        s[i] = Scene::CornerSeal(i, seal);
        i += 1;
    }
    s
}
/// The flat corner sweep (its layer kept) with every rectangle banded, and
/// with every rectangle copied (GC2BV's proofs).
pub const CORNER_FLAT_BAND: [Scene; CORNER_SCENES_EACH + 1] = seal_sweep(S3_BAND);
pub const CORNER_FLAT_DOUBLE: [Scene; CORNER_SCENES_EACH + 1] = seal_sweep(S3_DOUBLE);
/// GC2BV's scene-3 proofs: a seal without and with the layer.
pub const SEAL_PROOF_BAND: [Scene; 2] =
    [Scene::Corner3(S3_BAND), Scene::Corner3(S3_LAYER | S3_BAND)];
pub const SEAL_PROOF_DOUBLE: [Scene; 2] = [
    Scene::Corner3(S3_DOUBLE),
    Scene::Corner3(S3_LAYER | S3_DOUBLE),
];

/// Run GC2BW's game-shaped frames: a warped grid of [`MESH_COLS`] x
/// [`MESH_ROWS`] quads whose neighbours share every corner exactly (a
/// watertight mesh, as a game's models and terrain are), so every
/// triangle edge inside it is shared: on black, a crack shows black.
pub const MESH_COLS: i32 = 5;
pub const MESH_ROWS: i32 = 6;
/// The most draws a mesh frame takes: a full-screen quad and one mesh.
pub const MESH_DRAWS: usize = 1 + (MESH_COLS * MESH_ROWS) as usize;
/// The renderer's slots a mesh frame is set into (all of them, so a frame
/// set after a smaller scene clears what that left).
pub const MESH_SLOTS: usize = 64;
/// GC2BW's frames: flat-coloured mesh on black; the same over a full
/// hidden layer (a game's overdraw); a textured mesh (the 16 x 16 RGB565
/// texture wrapping, about 10 pixels a texel, no nudge, no clamp);
/// the mesh depth tested, "greater" with writes, near over a full-screen
/// quad written far.
pub const MESH_FLAT: u8 = 0;
pub const MESH_LAYERED: u8 = 1;
pub const MESH_TEXTURED: u8 = 2;
pub const MESH_DEPTH: u8 = 3;
/// A mesh's warp: each inner corner moved up to this many pixels each way
/// (sub-pixel amounts, so its edges cross every place in a pixel).
pub const MESH_WARP: i32 = 20;

/// A deterministic hash (the warp's and the colours' source).
const fn mesh_hash(a: u32, b: u32, c: u32) -> u32 {
    let mut h =
        a.wrapping_mul(0x9E37_79B1) ^ b.wrapping_mul(0x85EB_CA77) ^ c.wrapping_mul(0xC2B2_AE3D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h
}

/// Corner `(gx, gy)` of mesh `seed` over the pixel box `(x0, y0, w, h)`:
/// the grid point moved by the warp, the box's border kept straight.
#[must_use]
pub const fn mesh_corner(
    seed: u32,
    gx: i32,
    gy: i32,
    b: (i32, i32, i32, i32),
) -> rp_mbx3d_render::Point {
    let sub = rp_mbx3d_render::SUB;
    let (x0, y0, w, h) = b;
    let mut x = (x0 * sub) + w * sub * gx / MESH_COLS;
    let mut y = (y0 * sub) + h * sub * gy / MESH_ROWS;
    let span = (2 * MESH_WARP * sub + 1) as u32;
    if gx > 0 && gx < MESH_COLS {
        x += (mesh_hash(seed, gx as u32, gy as u32) % span) as i32 - MESH_WARP * sub;
    }
    if gy > 0 && gy < MESH_ROWS {
        y += (mesh_hash(seed ^ 0x55, gx as u32, gy as u32) % span) as i32 - MESH_WARP * sub;
    }
    rp_mbx3d_render::Point { x, y }
}

/// Point `p` of the 480 x 640 screen scaled to a `size` target (exact
/// for any `size` dividing it; a corner shared by two quads stays shared,
/// so the mesh stays watertight).
#[must_use]
pub const fn mesh_scaled(p: rp_mbx3d_render::Point, size: (u16, u16)) -> rp_mbx3d_render::Point {
    rp_mbx3d_render::Point {
        x: p.x * size.0 as i32 / W as i32,
        y: p.y * size.1 as i32 / H as i32,
    }
}

/// Mesh `seed`'s quads over `b`, each painted by `paint(cell, corners)`,
/// scaled from the 480 x 640 screen to `size`.
fn mesh_into(
    out: &mut [Option<Draw>; MESH_SLOTS],
    n: &mut usize,
    (seed, b, size): (u32, (i32, i32, i32, i32), (u16, u16)),
    paint: &dyn Fn(u32, &[rp_mbx3d_render::Point; 4]) -> Paint,
    depth: Option<rp_mbx3d_render::Depth>,
) {
    for gy in 0..MESH_ROWS {
        for gx in 0..MESH_COLS {
            let c = [
                mesh_scaled(mesh_corner(seed, gx, gy, b), size),
                mesh_scaled(mesh_corner(seed, gx + 1, gy, b), size),
                mesh_scaled(mesh_corner(seed, gx + 1, gy + 1, b), size),
                mesh_scaled(mesh_corner(seed, gx, gy + 1, b), size),
            ];
            let mut d = Draw::rect(0, 0, 1, 1, paint((gy * MESH_COLS + gx) as u32, &c));
            d.corners = c;
            d.depth = depth;
            out[*n] = Some(d);
            *n += 1;
        }
    }
}

/// A flat colour for mesh cell `i` of mesh `seed`: bright (each channel at
/// least 0x40), never black or the poison.
fn mesh_colour(seed: u32, i: u32) -> u32 {
    let h = mesh_hash(seed, i, 7);
    0xFF00_0000 | ((h & 0x00BF_BFBF) + 0x0040_4040)
}

/// GC2BW's frame `kind` ([`MESH_FLAT`], [`MESH_LAYERED`],
/// [`MESH_TEXTURED`], [`MESH_DEPTH`]): its background and the renderer's
/// [`MESH_SLOTS`] slots.
#[must_use]
pub fn mesh_frame(kind: u8, t: Tex) -> (u32, [Option<Draw>; MESH_SLOTS]) {
    mesh_frame_in(kind, t, (W as u16, H as u16))
}

/// [`mesh_frame`]'s frame drawn on a `size` target: the same geometry
/// scaled (GC2CA, R78: the game frames at fewer pixels for the video
/// plane's scaler), the texture's repeats kept, so a smaller frame is the
/// full one at a lower resolution.
#[must_use]
pub fn mesh_frame_in(kind: u8, t: Tex, size: (u16, u16)) -> (u32, [Option<Draw>; MESH_SLOTS]) {
    let mut out = [None; MESH_SLOTS];
    let mut n = 0;
    let screen = (0, 0, W as i32, H as i32);
    let whole = |c: u32| Draw::rect(0, 0, i32::from(size.0), i32::from(size.1), Paint::Flat(c));
    let flat = |seed: u32| {
        move |i: u32, _: &[rp_mbx3d_render::Point; 4]| Paint::Flat(mesh_colour(seed, i))
    };
    match kind {
        MESH_LAYERED => {
            out[n] = Some(whole(HIDDEN_LAYER));
            n += 1;
            mesh_into(&mut out, &mut n, (1, screen, size), &flat(1), None);
        }
        MESH_TEXTURED => {
            // Three repeats of the texture across the screen's width: the
            // uv at each corner from its place, so neighbours share them.
            let uv = |p: rp_mbx3d_render::Point| {
                let sub = rp_mbx3d_render::SUB;
                let (w, h) = (i32::from(size.0), i32::from(size.1));
                (
                    (i64::from(UV_ONE) * 3 * i64::from(p.x) / i64::from(w * sub)) as i32,
                    (i64::from(UV_ONE) * 4 * i64::from(p.y) / i64::from(h * sub)) as i32,
                )
            };
            let paint = |_: u32, c: &[rp_mbx3d_render::Point; 4]| Paint::Texture {
                tex: t.t565,
                uv: [uv(c[0]), uv(c[1]), uv(c[2]), uv(c[3])],
                tint: WHITE,
            };
            mesh_into(&mut out, &mut n, (2, screen, size), &paint, None);
        }
        MESH_DEPTH => {
            let d = |z: u16| {
                Some(rp_mbx3d_render::Depth {
                    mode: rp_mbx3d_render::depth::GREATER,
                    write: true,
                    z,
                })
            };
            // A full-screen quad written far, then the mesh near over it,
            // both "greater" with writes: every mesh pixel passes the depth
            // test against the quad's. (Two overlapping meshes made more
            // than the renderer's 128 cached lists, and the uncached ones
            // overflowed the 20 KiB list space on the die: `TooComplex`,
            // GC2BW's emulator check.)
            let mut back = whole(HIDDEN_LAYER);
            back.depth = d(0x3000);
            out[n] = Some(back);
            n += 1;
            mesh_into(&mut out, &mut n, (3, screen, size), &flat(3), d(0xB000));
        }
        _ => mesh_into(&mut out, &mut n, (0, screen, size), &flat(0), None),
    }
    debug_assert!(n <= MESH_DRAWS);
    (BLACK, out)
}

/// Run GC2BY's ladder for games (the owner, 2026-10-27: "make the test
/// that test for game overclocking"): every divide-1 step GC2AK and GC2BB
/// ran, fastest first, then 149.50 MHz with the core at divide 2; each
/// `((m, n), divide, label)`, the system clock 13 MHz x m / n.
pub const GAME_LADDER: [((u32, u32), u32, &str); 8] = [
    ((23, 2), 1, "149.50 / 1"),
    ((45, 4), 1, "146.25 / 1"),
    ((44, 4), 1, "143.00 / 1"),
    ((43, 4), 1, "139.75 / 1"),
    ((41, 4), 1, "133.25 / 1"),
    ((27, 3), 1, "117.00 / 1"),
    ((25, 3), 1, "108.33 / 1"),
    ((23, 2), 2, "149.50 / 2"),
];
/// GC2BY's rounds of the four game frames at each step (100: a frame
/// wrong in 3% of renders shows with ~95%).
pub const GAME_LADDER_ROUNDS: u32 = 100;
/// GC2BY's game frames ([`mesh_frame`]'s four).
pub const GAME_FRAMES: [u8; 4] = [MESH_FLAT, MESH_LAYERED, MESH_TEXTURED, MESH_DEPTH];

/// GC2CA's render sizes (R78 item 2): the screen, 3/4 (a dynamic
/// resolution step the video plane's scaler shows at 4/3) and half (its
/// 2x), each a whole number of 8 x 16 tiles and 16-byte rows.
pub const HALF_SIZES: [(u16, u16); 3] = [(480, 640), (360, 480), (240, 320)];
/// Words of a 16-bit picture of `size`.
#[must_use]
pub const fn words_of(size: (u16, u16)) -> usize {
    size.0 as usize * size.1 as usize / 2
}
/// GC2CA's reference words: each size's [`GAME_FRAMES`].
pub const HALF_REF_WORDS: usize = GAME_FRAMES.len()
    * (words_of(HALF_SIZES[0]) + words_of(HALF_SIZES[1]) + words_of(HALF_SIZES[2]));

/// GC2BU's forms of scene 3 without its layer: as is, the depth path, the
/// depth written only, the rectangles half a pixel over, the other
/// diagonal, as is again.
pub const EDGE_FORMS: [Scene; 6] = [
    Scene::Corner3(S3_AS_IS),
    Scene::Corner3(S3_DEPTH),
    Scene::Corner3(S3_DEPTH_ALWAYS),
    Scene::Corner3(S3_HALF),
    Scene::Corner3(S3_FLIP),
    Scene::Corner3(S3_AS_IS),
];
/// GC2BU's position map: the top-right rectangle alone moved by whole
/// tiles (its edges keep their places in their tiles) from its cell;
/// (0, 208) is the middle-right cell, where the race lives.
pub const EDGE_MAP: [Scene; 8] = [
    Scene::Corner3At(0, 208),
    Scene::Corner3At(0, 112),
    Scene::Corner3At(0, 160),
    Scene::Corner3At(0, 256),
    Scene::Corner3At(0, 304),
    Scene::Corner3At(-120, 208),
    Scene::Corner3At(-240, 208),
    Scene::Corner3At(-120, 160),
];
/// GC2BU's proof pairs: a cure form without and with the layer.
pub const EDGE_PROOF_DEPTH: [Scene; 2] = [
    Scene::Corner3(S3_DEPTH),
    Scene::Corner3(S3_DEPTH | S3_LAYER),
];
pub const EDGE_PROOF_DEPTH_ALWAYS: [Scene; 2] = [
    Scene::Corner3(S3_DEPTH_ALWAYS),
    Scene::Corner3(S3_DEPTH_ALWAYS | S3_LAYER),
];
pub const EDGE_PROOF_HALF: [Scene; 2] =
    [Scene::Corner3(S3_HALF), Scene::Corner3(S3_HALF | S3_LAYER)];
pub const EDGE_PROOF_FLIP: [Scene; 2] =
    [Scene::Corner3(S3_FLIP), Scene::Corner3(S3_FLIP | S3_LAYER)];

/// GC2BS's scene plan: the forms of scene 3 without its hidden layer, the
/// as-is form at both ends, then scene 3 with its layer (the shipped
/// form).
pub const CORNER3_SCENES: [Scene; 8] = [
    Scene::Corner3(S3_AS_IS),
    Scene::Corner3(S3_ALONE),
    Scene::Corner3(S3_ALONE_LEFT),
    Scene::Corner3(S3_ALONE_DOWN),
    Scene::Corner3(S3_REVERSED),
    Scene::Corner3(S3_GREY84),
    Scene::Corner3(S3_AS_IS),
    Scene::Corner(false, 3),
];

/// GC2BR's scene 3 ways: the control, the background flat, the rectangles
/// Gouraud, and without the hidden layer.
pub const SHADE_S3: [Scene; 4] = [
    Scene::CornerShade(3, 0),
    Scene::CornerShade(3, SHADE_FLAT_BG),
    Scene::CornerShade(3, SHADE_GOURAUD_RECT),
    Scene::CornerNoLayer(3),
];

/// The flat corner sweep ([`CORNER_FLAT`]) with its rectangles drawn
/// Gouraud (run GC2BR's breadth check of the cure candidate).
pub const CORNER_FLAT_GOURAUD: [Scene; CORNER_SCENES_EACH + 1] = {
    let mut s = CORNER_FLAT;
    let mut i = 0;
    while i < CORNER_SCENES_EACH {
        s[i] = Scene::CornerShade(i, SHADE_GOURAUD_RECT);
        i += 1;
    }
    s
};

/// Every scene of an `N`-scene plan in tile order `o` (run GC2BQ).
#[must_use]
pub const fn order_ways<const N: usize>(o: u8) -> [(usize, u8); N] {
    let mut w = [(0, o); N];
    let mut i = 0;
    while i < N {
        w[i] = (i, o);
        i += 1;
    }
    w
}

/// GC2BG's soak: the 16-bit kinds, the ring scene, the flat corner scenes
/// that faulted in GC2AJ (20, 18, 6) and the textured one with the u and v
/// shift at scene 20, in turn, in the checker order with the lists on
/// the die.
pub const PROOF_SOAK: [Scene; 13] = [
    Scene::Kind(0),
    Scene::Kind(1),
    Scene::Kind(FT2),
    Scene::Kind(5),
    Scene::Kind(6),
    Scene::Kind(7),
    Scene::Kind(9),
    Scene::Kind(10),
    Scene::Fix(0, 8),
    Scene::Corner(false, 20),
    Scene::Corner(false, 18),
    Scene::Corner(false, 6),
    Scene::CornerUv(20),
];

/// Run GC2BI's scene: kind 10's paint on one `m` x `m` quad (see
/// [`Scene::Magnified`]).
#[must_use]
pub fn magnified_scene(m: usize, t: Tex) -> (u32, [Option<Draw>; 3]) {
    let (a, b) = (UV_ONE / 16, 3 * UV_ONE / 16);
    let paint = Paint::Texture {
        tex: t.plane,
        uv: [(a, a), (b, a), (b, b), (a, b)],
        tint: WHITE,
    };
    let m = m.min(W as usize) as i32;
    (BLACK, [Some(Draw::rect(0, 0, m, m, paint)), None, None])
}

/// GC2BI's magnifications: kind 10's texel span on quads of these sides.
pub const MAGNIFIED: [Scene; 8] = [
    Scene::Magnified(4),
    Scene::Magnified(8),
    Scene::Magnified(16),
    Scene::Magnified(32),
    Scene::Magnified(64),
    Scene::Magnified(128),
    Scene::Magnified(256),
    Scene::Kind(10),
];

/// Run GC2BJ's scene: kind 10 as `cols` x `rows` quads (see
/// [`Scene::Split10`]); each quad's corners at whole pixels, its
/// coordinates kind 10's span interpolated to those pixels (rounded to
/// the nearest unit: the picture is its own reference).
#[must_use]
pub fn split10_scene(cols: usize, rows: usize, t: Tex) -> (u32, [Option<Draw>; SLOTS]) {
    let (a, b) = (UV_ONE / 16, 3 * UV_ONE / 16);
    let (w, h) = (W as i32, H as i32);
    let at = |p: i32, n: i32| a + ((b - a) as i64 * p as i64 / n as i64) as i32;
    let mut out = [None; SLOTS];
    let mut n = 0;
    for j in 0..rows as i32 {
        for i in 0..cols as i32 {
            if n == SLOTS {
                break;
            }
            let (x0, x1) = (w * i / cols as i32, w * (i + 1) / cols as i32);
            let (y0, y1) = (h * j / rows as i32, h * (j + 1) / rows as i32);
            let (u0, u1, v0, v1) = (at(x0, w), at(x1, w), at(y0, h), at(y1, h));
            let paint = Paint::Texture {
                tex: t.plane,
                uv: [(u0, v0), (u1, v0), (u1, v1), (u0, v1)],
                tint: WHITE,
            };
            out[n] = Some(Draw::rect(x0, y0, x1 - x0, y1 - y0, paint));
            n += 1;
        }
    }
    (BLACK, out)
}

/// GC2BJ's scenes: kind 10 as it is; kind 10's span on squares of side
/// 320, 384 and 480 (GC2BI: clean to 256, kind 10 wrong); kind 10 split
/// into 1 x 2, 2 x 2, 2 x 3 and 2 x 4 quads.
pub const SPLIT: [Scene; 8] = [
    Scene::Kind(10),
    Scene::Magnified(320),
    Scene::Magnified(384),
    Scene::Magnified(480),
    Scene::Split10(1, 2),
    Scene::Split10(2, 2),
    Scene::Split10(2, 3),
    Scene::Split10(2, 4),
];

/// The plane's four corners ([`PLANE`]) enlarged to a `side` x `side`
/// ARGB8888 texture on the CPU, word by word in its twiddled order
/// (`put(word index, word)`): texel (i, j) the bilinear blend of the
/// corners at (i, j) / (side - 1), each channel rounded to the nearest.
/// Bilinear blending is linear inside a texel, so sampling these grid
/// points and blending between them draws kind 10's surface again in
/// exact arithmetic, magnified `side - 1` times less (run GC2BK).
pub fn upsampled_words(side: u32, mut put: impl FnMut(u32, u32)) {
    let s = side - 1;
    let ch = |c: u32, k: u32| (c >> k) & 0xFF;
    for i in 0..side * side {
        let (x, y) = untwiddle(i);
        let mut w = 0;
        for k in [0, 8, 16, 24] {
            let n = ch(PLANE[0], k) * (s - x) * (s - y)
                + ch(PLANE[1], k) * x * (s - y)
                + ch(PLANE[2], k) * (s - x) * y
                + ch(PLANE[3], k) * x * y;
            w |= ((n + s * s / 2) / (s * s)) << k;
        }
        put(i, w);
    }
}

/// GC2BK's first plan: kind 10 as it is (the control) and its span on
/// squares placing the threshold between GC2BI's 256 (clean) and GC2BJ's
/// 320 (wrong) at 149.50 / 1.
pub const THRESHOLD: [Scene; 6] = [
    Scene::Kind(10),
    Scene::Magnified(256),
    Scene::Magnified(272),
    Scene::Magnified(288),
    Scene::Magnified(304),
    Scene::Magnified(320),
];

/// `d` with its texture span nudged up `n` [`UV_ONE`] units on each axis
/// (run GC2BM: an upward nudge clears divide 1's short samples on exact
/// steps): u at the right corners, v at the bottom ones, each axis only
/// when the texture is magnified at least [`NUDGE_MAGNIFIED`] times along
/// it (GC2BO: a texture drawn about 1.45 pixels a texel, right as it is,
/// went wrong nudged). Anything not textured as it is.
#[must_use]
pub fn nudged(d: Draw, n: i32) -> Draw {
    let Paint::Texture { tex, uv, tint } = d.paint else {
        return d;
    };
    let texel = i64::from(UV_ONE >> (3 + tex.code));
    let px = |a: i32, b: i32| i64::from((b - a).abs() / rp_mbx3d_render::SUB).max(1);
    // Pixels across and down; the span in texels times the magnification
    // is the pixels, so magnified >= M when pixels * texel >= M * span.
    let (w, h) = (
        px(d.corners[0].x, d.corners[1].x),
        px(d.corners[0].y, d.corners[3].y),
    );
    let (su, sv) = (
        i64::from((uv[1].0 - uv[0].0).abs()).max(1),
        i64::from((uv[3].1 - uv[0].1).abs()).max(1),
    );
    let mag = NUDGE_MAGNIFIED as i64;
    let (nu, nv) = (w * texel >= mag * su, h * texel >= mag * sv);
    let mut uv = uv;
    if nu {
        uv[1].0 += n;
        uv[2].0 += n;
    }
    if nv {
        uv[2].1 += n;
        uv[3].1 += n;
    }
    Draw {
        paint: Paint::Texture { tex, uv, tint },
        ..d
    }
}

/// The magnification from which [`nudged`] nudges an axis (GC2BL-GC2BN:
/// the short-sample faults at 32 times and more; kind 5, about 1.45, is
/// hurt by a nudge).
pub const NUDGE_MAGNIFIED: u32 = 16;

/// GC2BO's content set: every 16-bit kind with the half-texel shift
/// ([`Scene::KindUv`]), kind 10 drawn from its plane enlarged to 16 x 16
/// ([`Scene::Upsampled10`]: the fourth cure) in its place.
pub const PROOF2_KINDS: [Scene; 8] = [
    Scene::KindUv(0),
    Scene::KindUv(1),
    Scene::KindUv(FT2),
    Scene::KindUv(5),
    Scene::KindUv(6),
    Scene::KindUv(7),
    Scene::KindUv(9),
    Scene::Upsampled10(1),
];

/// GC2BO's soak: the content set, the ring scene, the flat corner scenes
/// that faulted in GC2AJ and the textured one with the u and v shift.
pub const PROOF2_SOAK: [Scene; 13] = [
    Scene::KindUv(0),
    Scene::KindUv(1),
    Scene::KindUv(FT2),
    Scene::KindUv(5),
    Scene::KindUv(6),
    Scene::KindUv(7),
    Scene::KindUv(9),
    Scene::Upsampled10(1),
    Scene::Fix(0, 8),
    Scene::Corner(false, 20),
    Scene::Corner(false, 18),
    Scene::Corner(false, 6),
    Scene::CornerUv(20),
];

/// One texel of kind 10's 8 x 8 texture in [`UV_ONE`] units.
pub const K10_TEXEL: i32 = UV_ONE / 8;
/// GC2BI's compensation steps: kind 10's coordinates pulled back by 0.004,
/// 0.008 and 0.012 texel in u, v and both (GC2AM put the warm error under
/// 0.009 texel), and pushed forward 0.008 in both as the sign check;
/// kind 10 as it is first.
pub const COMPENSATE: [Scene; 11] = {
    let d = [
        K10_TEXEL * 4 / 1000,
        K10_TEXEL * 8 / 1000,
        K10_TEXEL * 12 / 1000,
    ];
    [
        Scene::Shifted10(0, 0),
        Scene::Shifted10(-d[0], 0),
        Scene::Shifted10(-d[1], 0),
        Scene::Shifted10(-d[2], 0),
        Scene::Shifted10(0, -d[0]),
        Scene::Shifted10(0, -d[1]),
        Scene::Shifted10(0, -d[2]),
        Scene::Shifted10(-d[0], -d[0]),
        Scene::Shifted10(-d[1], -d[1]),
        Scene::Shifted10(-d[2], -d[2]),
        Scene::Shifted10(d[1], d[1]),
    ]
};

/// GC2BH's textured kinds: FT2 (the clamped 8888 quad), kind 10 (one
/// texture magnified by the bilinear filter), kind 5 (a bilinear 565
/// texture) and kind 6 (a wrapping 4444 texture, which the shift leaves).
pub const TEXEL_KINDS: [Scene; 4] = [
    Scene::Kind(FT2),
    Scene::Kind(10),
    Scene::Kind(5),
    Scene::Kind(6),
];
/// The same with GC2Z's half-texel shift.
pub const TEXEL_KINDS_UV: [Scene; 4] = [
    Scene::KindUv(FT2),
    Scene::KindUv(10),
    Scene::KindUv(5),
    Scene::KindUv(6),
];

/// GC2AJ's flat plan: the corner scenes, then GC2AI's faulting way (the
/// ring over the layer, the quad's corner at x 440, y 40) as the control
/// that the run can see the fault.
pub const CORNER_FLAT: [Scene; CORNER_SCENES_EACH + 1] = {
    let mut s = [Scene::Quad(0); CORNER_SCENES_EACH + 1];
    let mut i = 0;
    while i < CORNER_SCENES_EACH {
        s[i] = Scene::Corner(false, i);
        i += 1;
    }
    s
};
pub const CORNER_TEX: [Scene; CORNER_SCENES_EACH] = {
    let mut s = [Scene::Corner(true, 0); CORNER_SCENES_EACH];
    let mut i = 0;
    while i < CORNER_SCENES_EACH {
        s[i] = Scene::Corner(true, i);
        i += 1;
    }
    s
};
pub const CORNER_WAYS: [(usize, u8); CORNER_SCENES_EACH + 1] = {
    let mut w = [(0, REVERSED); CORNER_SCENES_EACH + 1];
    let mut i = 0;
    while i <= CORNER_SCENES_EACH {
        w[i] = (i, REVERSED);
        i += 1;
    }
    w
};

/// Run GC2AK (the owner, 2026-10-26: "Build the sweep with the u and v
/// texture fix, run at each lower divide-1 step and at 149.50 / 2, to find
/// the fastest setting where every corner position draws right"): GC2AJ's
/// textured sweep with the clamped texture shifted half a texel in u and v
/// (GC2AJ's every-render fault was the u counterpart of the v fault).
pub const CORNER_UV: [Scene; CORNER_SCENES_EACH] = {
    let mut s = [Scene::CornerUv(0); CORNER_SCENES_EACH];
    let mut i = 0;
    while i < CORNER_SCENES_EACH {
        s[i] = Scene::CornerUv(i);
        i += 1;
    }
    s
};
/// GC2AK's settings, fastest first: 13 MHz x m / n, the cores' divide.
pub const LADDER_STEPS: [((u32, u32), u32, &str); 6] = [
    ((23, 2), 1, "149.50 / 1"),
    ((43, 4), 1, "139.75 / 1"),
    ((41, 4), 1, "133.25 / 1"),
    ((27, 3), 1, "117.00 / 1"),
    ((25, 3), 1, "108.33 / 1"),
    ((23, 2), 2, "149.50 / 2"),
];
/// Rounds a scene a setting: 100 (a corner place faulting 3% of renders
/// shows with ~95%; the step that passes gets its own soak).
pub const LADDER_ROUNDS: u32 = 100;

/// Run GC2AL (the owner, 2026-10-26: "Keep working on the fast mode: find
/// what cures the remaining textured faults"). GC2AK left textured faults
/// at every divide-1 step: one pixel in a tile's last rows beside a
/// rectangle's corner, as GC2AI. GC2T's reading: the core starts the next
/// tile in its on-chip buffer before the last rows are written out; every
/// cure here gives a tile more to do first, or changes which tile follows.
/// Five rectangles a scene (two hidden layers or an extra blended layer
/// take the slots of the sixth), all of GC2AK's u and v textured kind.
pub const CURE_RECTS: usize = 5;
pub const CURE_SCENES: usize = 128_usize.div_ceil(CURE_RECTS);
/// GC2AL's cures: (0) one full layer (GC2AK's, the baseline); (1) two full
/// layers; (2) one full layer and [`INVISIBLE`] blended over everything.
/// Tile order is the way's (reversed, or rows for the baseline's twin).
#[cfg(test)]
pub const CURES: usize = 3;
/// Fully transparent black: blended over every tile, nothing seen.
pub const INVISIBLE: u32 = 0x0000_0000;
pub const CURE_ROUNDS: u32 = 100;

/// Rectangle `j` of a scene of `per` (see [`corner_rect`]).
#[must_use]
pub const fn corner_rect_of(per: usize, i: usize, j: usize) -> ((i32, i32, i32, i32), usize) {
    let k = (per * i + j) % 128;
    let (a, c) = ((k % 8) as i32, (k / 8) as i32);
    let (cx, cy) = ((j % 2) as i32 * 240, (j / 2) as i32 * 208);
    ((cx + 16 + a, cy + 16 + c, cx + 200 + a, cy + 176 + c), k)
}

/// GC2AL's scene `i` with cure `cure` (see [`CURES`]).
#[must_use]
pub fn cure_scene(cure: usize, i: usize, t: Tex) -> (u32, [Option<Draw>; SLOTS]) {
    let (bg, slots) = order_scene(FIX_BASE[0], t);
    let mut out = [None; SLOTS];
    let mut n = 0;
    for _ in 0..if cure == 1 { 2 } else { 1 } {
        out[n] = Some(flat(HIDDEN_LAYER, false));
        n += 1;
    }
    for d in slots.iter().flatten() {
        if !matches!(d.paint, Paint::Texture { .. }) {
            out[n] = Some(*d);
            n += 1;
        }
    }
    for j in 0..CURE_RECTS {
        let ((x0, y0, x1, y1), _) = corner_rect_of(CURE_RECTS, i, j);
        let d = Draw::rect(x0, y0, x1 - x0, y1 - y0, Paint::texture(t.t8888));
        out[n] = Some(shift_texture(d, TexFix::ClampUv));
        n += 1;
    }
    if cure == 2 {
        out[n] = Some(flat(INVISIBLE, true));
    }
    (bg, out)
}

const fn cure_set(cure: usize) -> [Scene; CURE_SCENES] {
    let mut s = [Scene::Cure(0, 0); CURE_SCENES];
    let mut i = 0;
    while i < CURE_SCENES {
        s[i] = Scene::Cure(cure, i);
        i += 1;
    }
    s
}
pub const CURE_ONE: [Scene; CURE_SCENES] = cure_set(0);
pub const CURE_TWO: [Scene; CURE_SCENES] = cure_set(1);
pub const CURE_BUSY: [Scene; CURE_SCENES] = cure_set(2);
const fn cure_ways(o: u8) -> [(usize, u8); CURE_SCENES] {
    let mut w = [(0, REVERSED); CURE_SCENES];
    let mut i = 0;
    while i < CURE_SCENES {
        w[i] = (i, o);
        i += 1;
    }
    w
}
pub const CURE_REVERSED: [(usize, u8); CURE_SCENES] = cure_ways(REVERSED);
pub const CURE_ROWS: [(usize, u8); CURE_SCENES] = cure_ways(ROWS);

/// Run GC2AM (the owner, 2026-10-26: "Chase it ... research how similar
/// issues have been solved, cleverly, in the past ... losslessly";
/// docs/design/divide1-texel-margin.md). Part 1: GC2AL's faulting
/// rectangle (scene 1, rectangle 2) alone, its u phase stepped across one
/// period of its sample pattern; part 2: textured rectangles at the safe
/// ratios, each at its best phase, at every corner place of a tile.
pub const PHASE_RECT: (i32, i32, i32, i32) = corner_rect_of(CURE_RECTS, 1, 2).0;
/// Part 1's steps: 31 of [`PHASE_STEP`] UV units (UV_ONE is the whole
/// texture: 2,048 a texel of FT2's 32), together just over 1/23 of a texel
/// (89 units), the sample pattern's period at 32 texels on 184 pixels.
pub const PHASES: usize = 31;
pub const PHASE_STEP: i32 = 3;
pub const PHASE_ROUNDS: u32 = 3;

/// Part 2's ratios: (pixels, texels) of one side: 1:1, 2:1, 4:1, 1:2.
pub const RATIOS: [(i32, i32); 4] = [(1, 1), (2, 1), (4, 1), (1, 2)];
pub const RATIO_ROUNDS: u32 = 30;

/// The best phase at `p / q` pixels a texel (lowest terms), in UV units of
/// a `side`-texel texture: the samples' fractions are `p` evenly spaced
/// values, so a phase of 1 / (2p) texel keeps every one 1 / (2p) from an
/// edge, the most any phase can (docs/design/divide1-texel-margin.md).
#[must_use]
pub const fn best_phase(p: i32, side: i32) -> i32 {
    UV_ONE / side / (2 * p)
}

/// The least distance, in 65,536ths of a texel, of any sample of `n`
/// pixels from a texel edge, at `px` pixels for `texels` texels and a
/// phase of `phase` texel / 65,536 (samples at whole pixels, the core's
/// convention as GC2Z's half-texel cure showed).
#[cfg(test)]
#[must_use]
pub fn margin(px: i64, texels: i64, phase: i64, n: i64) -> i64 {
    let one = 65_536i64;
    (0..n)
        .map(|m| {
            let f = (phase + m * texels * one / px).rem_euclid(one);
            f.min(one - f)
        })
        .min()
        .unwrap_or(one)
}

/// Part 1's scene `n`: one full layer, FT2's Gouraud, then
/// [`PHASE_RECT`] with FT2's texture, shifted half a texel in u and v as
/// GC2AK and `n * PHASE_STEP` more in u.
#[must_use]
pub fn phase_scene(n: usize, t: Tex) -> (u32, [Option<Draw>; SLOTS]) {
    let (bg, mut out) = layer_and_gouraud(t);
    let (x0, y0, x1, y1) = PHASE_RECT;
    let d = shift_texture(
        Draw::rect(x0, y0, x1 - x0, y1 - y0, Paint::texture(t.t8888)),
        TexFix::ClampUv,
    );
    let Paint::Texture { tex, uv, tint } = d.paint else {
        return (bg, out);
    };
    let du = n as i32 * PHASE_STEP;
    out[2] = Some(Draw {
        paint: Paint::Texture {
            tex,
            uv: uv.map(|(u, v)| (u + du, v)),
            tint,
        },
        ..d
    });
    (bg, out)
}

/// Part 2's scene `i` at ratio `r`: one full layer, FT2's Gouraud, then
/// five rectangles of FT2's texture at [`RATIOS`]`[r]`, each at the best
/// phase in u and v, every corner of rectangle `k` at x offset `k % 8`, y
/// offset `k / 8` in its tile (as GC2AJ).
#[must_use]
pub fn ratio_scene(r: usize, i: usize, t: Tex) -> (u32, [Option<Draw>; SLOTS]) {
    let (bg, mut out) = layer_and_gouraud(t);
    let (px, tx) = RATIOS[r];
    let side = t.t8888.side() as i32;
    let size = side * px / tx;
    let ph = best_phase(px, side);
    for j in 0..CURE_RECTS {
        let ((x0, y0, _, _), _) = corner_rect_of(CURE_RECTS, i, j);
        let d = Draw::rect(x0, y0, size, size, Paint::texture(t.t8888));
        let Paint::Texture { tex, uv, tint } = d.paint else {
            continue;
        };
        out[2 + j] = Some(Draw {
            paint: Paint::Texture {
                tex,
                uv: uv.map(|(u, v)| (u + ph, v + ph)),
                tint,
            },
            ..d
        });
    }
    (bg, out)
}

/// One full layer and FT2's Gouraud background, the rest of the slots free.
fn layer_and_gouraud(t: Tex) -> (u32, [Option<Draw>; SLOTS]) {
    let (bg, slots) = order_scene(FIX_BASE[0], t);
    let mut out = [None; SLOTS];
    out[0] = Some(flat(HIDDEN_LAYER, false));
    if let Some(g) = slots
        .iter()
        .flatten()
        .find(|d| !matches!(d.paint, Paint::Texture { .. }))
    {
        out[1] = Some(*g);
    }
    (bg, out)
}

pub const PHASE_SCENES: [Scene; PHASES] = {
    let mut s = [Scene::Phase(0); PHASES];
    let mut i = 0;
    while i < PHASES {
        s[i] = Scene::Phase(i);
        i += 1;
    }
    s
};
pub const PHASE_WAYS: [(usize, u8); PHASES] = {
    let mut w = [(0, REVERSED); PHASES];
    let mut i = 0;
    while i < PHASES {
        w[i] = (i, REVERSED);
        i += 1;
    }
    w
};
const fn ratio_set(r: usize) -> [Scene; CURE_SCENES] {
    let mut s = [Scene::Ratio(0, 0); CURE_SCENES];
    let mut i = 0;
    while i < CURE_SCENES {
        s[i] = Scene::Ratio(r, i);
        i += 1;
    }
    s
}
pub const RATIO_SCENES: [[Scene; CURE_SCENES]; 4] =
    [ratio_set(0), ratio_set(1), ratio_set(2), ratio_set(3)];

/// Run GC2AO (the owner, 2026-10-26: "Build the combined proof run: both
/// cures together over the flat and textured corner sweeps at 149.50 / 1,
/// then an overnight soak"): GC2AJ's corner sweep (six rectangles a scene,
/// all 128 places of a tile) flat, and textured at the phase that keeps
/// every centre sample furthest from a texel edge ([`best_centre_phase`],
/// GC2AM: the core samples at pixel centres), with one full hidden layer
/// or none, in the interleaved order (rows from the top, each row's even
/// tiles then its odd ones) or GC2AN's checker.
#[must_use]
pub fn proof_scene(layer: bool, textured: bool, i: usize, t: Tex) -> (u32, [Option<Draw>; SLOTS]) {
    let (bg, slots) = order_scene(FIX_BASE[0], t);
    let mut out = [None; SLOTS];
    let mut n = 0;
    if layer {
        out[n] = Some(flat(HIDDEN_LAYER, false));
        n += 1;
    }
    if let Some(g) = slots
        .iter()
        .flatten()
        .find(|d| !matches!(d.paint, Paint::Texture { .. }))
    {
        out[n] = Some(*g);
        n += 1;
    }
    let side = t.t8888.side() as i64;
    for j in 0..CORNER_RECTS {
        let ((x0, y0, x1, y1), _) = corner_rect(i, j);
        let d = Draw::rect(
            x0,
            y0,
            x1 - x0,
            y1 - y0,
            if textured {
                Paint::texture(t.t8888)
            } else {
                Paint::Flat(UNTEXTURED)
            },
        );
        out[n] = Some(match d.paint {
            Paint::Texture { tex, uv, tint } => {
                // The phase in UV units: a texel is UV_ONE / side.
                let to_uv = |ph: i64| (ph * i64::from(UV_ONE) / side / 65_536) as i32;
                let pu = to_uv(best_centre_phase(i64::from(x1 - x0), side));
                let pv = to_uv(best_centre_phase(i64::from(y1 - y0), side));
                Draw {
                    paint: Paint::Texture {
                        tex,
                        uv: uv.map(|(u, v)| (u + pu, v + pv)),
                        tint,
                    },
                    ..d
                }
            }
            _ => d,
        });
        n += 1;
    }
    (bg, out)
}

/// The phase (in 65,536ths of a texel) that keeps every pixel-centre sample
/// of `px` pixels showing `texels` texels furthest from a texel edge: the
/// samples sit at phase + (m + 1/2) texels / px; searched in 1/1,024-texel
/// steps over one texel (GC2AM: divide 1's error is under 0.009 texel and
/// one-sided, so the margin, not the side, is what counts).
#[must_use]
pub fn best_centre_phase(px: i64, texels: i64) -> i64 {
    let one = 65_536i64;
    let margin = |ph: i64| {
        (0..px)
            .map(|m| {
                let f = (ph + (2 * m + 1) * texels * one / (2 * px)).rem_euclid(one);
                f.min(one - f)
            })
            .min()
            .unwrap_or(one)
    };
    (0..1024)
        .map(|k| k * 64)
        .max_by_key(|&ph| (margin(ph), -ph))
        .unwrap_or(0)
}

const fn proof_set(layer: bool, textured: bool) -> [Scene; CORNER_SCENES_EACH] {
    let mut s = [Scene::Proof(false, false, 0); CORNER_SCENES_EACH];
    let mut i = 0;
    while i < CORNER_SCENES_EACH {
        s[i] = Scene::Proof(layer, textured, i);
        i += 1;
    }
    s
}
pub const PROOF_FLAT_LAYER: [Scene; CORNER_SCENES_EACH] = proof_set(true, false);
pub const PROOF_TEX_LAYER: [Scene; CORNER_SCENES_EACH] = proof_set(true, true);
pub const PROOF_FLAT_BARE: [Scene; CORNER_SCENES_EACH] = proof_set(false, false);
pub const PROOF_TEX_BARE: [Scene; CORNER_SCENES_EACH] = proof_set(false, true);
const fn proof_ways(o: u8) -> [(usize, u8); CORNER_SCENES_EACH] {
    let mut w = [(0, CHECKER); CORNER_SCENES_EACH];
    let mut i = 0;
    while i < CORNER_SCENES_EACH {
        w[i] = (i, o);
        i += 1;
    }
    w
}
pub const PROOF_CHECKER: [(usize, u8); CORNER_SCENES_EACH] = proof_ways(CHECKER);
pub const PROOF_INTERLEAVED: [(usize, u8); CORNER_SCENES_EACH] = proof_ways(INTERLEAVED);
/// GC2AN's columns order (consecutive tiles never in the same row): the
/// fixed soak's second proven order.
pub const PROOF_COLUMNS: [(usize, u8); CORNER_SCENES_EACH] = proof_ways(COLUMNS);
pub const PROOF_ROUNDS: u32 = 100;

/// Colours tried as a soak's poison, the most saturated first (no GC2
/// scene's Gouraud or texture reaches them, by design; [`absent16`]
/// checks it on the references themselves).
pub const POISON: [u16; 8] = [
    0xF81F, 0x801F, 0xF810, 0x07E1, 0xF801, 0x07FF, 0x0821, 0xFFE0,
];

/// The first of `candidates` that no 16-bit pixel of `pics` (pictures as
/// words, two pixels a word) holds, or `None` if every one appears.
#[must_use]
pub fn absent16(pics: &[u32], candidates: &[u16]) -> Option<u16> {
    let mut seen = 0u32;
    for &w in pics {
        for half in [w & 0xFFFF, w >> 16] {
            for (i, &c) in candidates.iter().enumerate().take(32) {
                if half == u32::from(c) {
                    seen |= 1 << i;
                }
            }
        }
    }
    candidates
        .iter()
        .take(32)
        .enumerate()
        .find(|&(i, _)| seen & (1 << i) == 0)
        .map(|(_, &c)| c)
}

/// Run GC2BZ (R20): the scan frame a tile row first shows in, from where
/// the scan was when it landed (`(frames begun since the render started,
/// scan line)`) and the row's top line `y`: written before the scan
/// reached `y`, this frame; at or after, the next.
#[must_use]
pub const fn shown_in((wraps, line): (u8, u16), y: u32) -> u8 {
    if (line as u32) < y { wraps } else { wraps + 1 }
}

/// Run GC2BZ: a row's frame from its first and last sampled pixels, or
/// `None` if either never landed or the two fall in different frames (the
/// row itself torn).
#[must_use]
pub fn row_shown(first: Option<(u8, u16)>, last: Option<(u8, u16)>, y: u32) -> Option<u8> {
    match (first, last) {
        (Some(a), Some(b)) if shown_in(a, y) == shown_in(b, y) => Some(shown_in(a, y)),
        _ => None,
    }
}

#[cfg(test)]
#[path = "gc2_tests.rs"]
mod tests;
