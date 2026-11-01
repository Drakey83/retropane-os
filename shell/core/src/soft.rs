//! The light look's soft panels (the owner, 2026-11-01, his mock-up: a
//! solid white panel "opaque in the middle and just transparent on the
//! edges with a very gentle transition", "more mathematically precise and
//! smoother"): a near-opaque frosted middle whose rim fades to clear over
//! [`EDGE`] px, its corners rounded by true distance from the inner corner,
//! so no edge or corner shows a step or a box.
//!
//! On the 2D engine, in five commands a panel: the middle a fill, the side
//! rims horizontal gradients (the engine's own), the top and bottom rims and
//! the corners runs of a tiny font whose glyphs are the rim's exact
//! coverage, made at build time (text is what the engine blends over what
//! is under it; its masks stay on the GPU: ~10 KB). Every word sits inside
//! the middle (cards' words start 14 px in), where the light look's
//! contrast test holds it.
//!
//! Iterate (rule 0): phones soften a widget's edge with a blur or a
//! rounded-rectangle mask drawn per pixel by the GPU each frame; the X51v's
//! 2D engine has neither, so ours precomputes the rim once as coverage masks
//! and lets the engine's own gradient and glyph blending draw it, the same
//! command count as the plain glass it replaces.

use rp_draw::{Argb, Cmd, Font, Frame, Glyph, Rect};

/// The rim's width (px).
pub const EDGE: u16 = 16;
const E: usize = EDGE as usize;

/// The panel's colour, a frosted white, near opaque; and its rim's clear
/// end. Both kept by the light look's mapping ([`is_soft`]).
pub const PANEL: Argb = 0xF0F4_F6F9;
const CLEAR: Argb = PANEL & 0x00FF_FFFF;

/// Whether `c` is the soft panels' own (the light look keeps it).
#[must_use]
pub const fn is_soft(c: Argb) -> bool {
    c == PANEL || c == CLEAR
}

const fn isqrt(v: u32) -> u32 {
    if v == 0 {
        return 0;
    }
    let mut x = v;
    let mut y = x.div_ceil(2);
    while y < x {
        x = y;
        y = (x + v / x) / 2;
    }
    x
}

/// Corner `k` (0 top left, 1 top right, 2 bottom left, 3 bottom right):
/// coverage falling linearly with the distance from the panel's inner
/// corner, clear at [`EDGE`] px (a quarter circle: round, soft).
const fn corner(k: usize) -> [u8; E * E] {
    let mut c = [0u8; E * E];
    let mut i = 0;
    while i < E * E {
        let (x, y) = (i % E, i / E);
        // Twice the distance across and down from the inner corner, at the
        // pixel's centre.
        let dx = if k.is_multiple_of(2) {
            2 * E - (2 * x + 1)
        } else {
            2 * x + 1
        };
        let dy = if k < 2 {
            2 * E - (2 * y + 1)
        } else {
            2 * y + 1
        };
        let d = isqrt((dx * dx + dy * dy) as u32 * 256) as usize; // x 16 x 2
        let full = 2 * E * 16;
        c[i] = if d >= full {
            0
        } else {
            ((full - d) * 255 / full) as u8
        };
        i += 1;
    }
    c
}

/// A rim tile `w` px wide: coverage rising linearly from the panel's
/// outside to its inside over [`EDGE`] rows (`top`: the outside is up).
const fn rim<const N: usize>(w: usize, top: bool) -> [u8; N] {
    let mut c = [0u8; N];
    let mut i = 0;
    while i < w * E {
        let y = i / w;
        let inside = if top { 2 * y + 1 } else { 2 * E - (2 * y + 1) };
        c[i] = (inside * 255 / (2 * E)) as u8;
        i += 1;
    }
    c
}

static CORNERS: [[u8; E * E]; 4] = [corner(0), corner(1), corner(2), corner(3)];
static TOP32: [u8; 32 * E] = rim::<{ 32 * E }>(32, true);
static TOP16: [u8; 16 * E] = rim::<{ 16 * E }>(16, true);
static TOP8: [u8; 8 * E] = rim::<{ 8 * E }>(8, true);
static TOP4: [u8; 4 * E] = rim::<{ 4 * E }>(4, true);
static TOP2: [u8; 2 * E] = rim::<{ 2 * E }>(2, true);
static TOP1: [u8; E] = rim::<{ E }>(1, true);
static BOT32: [u8; 32 * E] = rim::<{ 32 * E }>(32, false);
static BOT16: [u8; 16 * E] = rim::<{ 16 * E }>(16, false);
static BOT8: [u8; 8 * E] = rim::<{ 8 * E }>(8, false);
static BOT4: [u8; 4 * E] = rim::<{ 4 * E }>(4, false);
static BOT2: [u8; 2 * E] = rim::<{ 2 * E }>(2, false);
static BOT1: [u8; E] = rim::<{ E }>(1, false);

const fn glyph(w: usize, cover: &'static [u8]) -> Glyph {
    Glyph {
        w: w as u8,
        h: E as u8,
        left: 0,
        top: 0,
        advance: w as u8,
        cover,
    }
}

/// The rims' glyphs: the four corners, then the top tiles and the bottom
/// tiles, 32 to 1 px wide (any width in a few).
static GLYPHS: [Glyph; 16] = [
    glyph(E, &CORNERS[0]),
    glyph(E, &CORNERS[1]),
    glyph(E, &CORNERS[2]),
    glyph(E, &CORNERS[3]),
    glyph(32, &TOP32),
    glyph(16, &TOP16),
    glyph(8, &TOP8),
    glyph(4, &TOP4),
    glyph(2, &TOP2),
    glyph(1, &TOP1),
    glyph(32, &BOT32),
    glyph(16, &BOT16),
    glyph(8, &BOT8),
    glyph(4, &BOT4),
    glyph(2, &BOT2),
    glyph(1, &BOT1),
];

/// The rims' font: each glyph hangs from the baseline (its top there).
pub static RIMS: Font = Font {
    line: E as u8,
    ascent: 0,
    first: b'A',
    glyphs: &GLYPHS,
};

/// A rim run: a corner, tiles across the panel's width less two corners,
/// a corner.
fn rim_run(buf: &mut [u8; 24], w: u16, top: bool) -> usize {
    let (left, right, tiles) = if top { (0u8, 1u8, 4u8) } else { (2, 3, 10) };
    let mut n = 0;
    buf[n] = b'A' + left;
    n += 1;
    let mut rest = w.saturating_sub(2 * EDGE);
    for (k, step) in [32u16, 16, 8, 4, 2, 1].into_iter().enumerate() {
        while rest >= step && n < buf.len() - 1 {
            buf[n] = b'A' + tiles + k as u8;
            n += 1;
            rest -= step;
        }
    }
    buf[n] = b'A' + right;
    n + 1
}

/// The soft panel `r`: the middle, its side rims, its top and bottom rims
/// with the corners. `r` at least [`EDGE`] x 2 each way.
pub fn panel(f: &mut Frame, r: Rect) {
    let mid_h = r.h.saturating_sub(2 * EDGE);
    let y = r.y + EDGE;
    f.push(Cmd::HGradient {
        r: Rect::new(r.x, y, EDGE, mid_h),
        left: CLEAR,
        right: PANEL,
    });
    f.push(Cmd::Fill {
        r: Rect::new(r.x + EDGE, y, r.w - 2 * EDGE, mid_h),
        color: PANEL,
    });
    f.push(Cmd::HGradient {
        r: Rect::new(r.x + r.w - EDGE, y, EDGE, mid_h),
        left: PANEL,
        right: CLEAR,
    });
    let mut buf = [0u8; 24];
    let n = rim_run(&mut buf, r.w, true);
    f.text(r.x, r.y, &RIMS, PANEL, &buf[..n]);
    let n = rim_run(&mut buf, r.w, false);
    f.text(r.x, r.y + r.h - EDGE, &RIMS, PANEL, &buf[..n]);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A run's glyphs add up to the panel's width exactly (no gap, no
    /// overlap darkening a seam).
    #[test]
    fn a_rim_spans_the_panel_exactly() {
        for w in [83u16, 200, 448, 480] {
            for top in [true, false] {
                let mut buf = [0u8; 24];
                let n = rim_run(&mut buf, w, top);
                assert_eq!(RIMS.width(&buf[..n]), w, "{w} {top}");
            }
        }
    }

    /// The rim meets the middle and the sides: full at its inside, clear at
    /// its outside, and the corner equals the side and top rims where they
    /// meet it.
    #[test]
    fn the_rim_is_continuous() {
        let top = &TOP1;
        assert!(top[0] < 16 && top[E - 1] > 240);
        let c = &CORNERS[0];
        // The corner's inner row and column read as the rims next to them.
        for k in 0..E {
            let row = c[(E - 1) * E + k];
            let col = c[k * E + E - 1];
            let rim = top[k];
            let side = ((2 * k + 1) * 255 / (2 * E)) as u8;
            assert!(row.abs_diff(side) <= 24, "row {k}: {row} vs {side}");
            assert!(col.abs_diff(rim) <= 24, "col {k}: {col} vs {rim}");
        }
    }
}
