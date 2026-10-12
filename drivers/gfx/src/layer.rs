//! The interface layer: what goes in the 2700G's graphics plane, over the
//! wallpaper in the video plane (overnight item 60, docs/hw/2700g.md).
//!
//! The display mixes the two planes as it scans out, so the CPU never
//! blends the interface with the wallpaper: glass is just pixels with an
//! alpha. What the CPU still does:
//!
//! - **Compose in RAM at full colour.** A [`Layer`] holds ARGB8888 with
//!   straight (not premultiplied) alpha, the form the chip mixes (its
//!   `scanout::mix`). 1.2 MB for the screen; the owner chose this over
//!   16-bit colour so the dither always starts from 8 bits a channel and
//!   gradients behind glass stay as smooth as the stage E they approved.
//! - **Draw over glass.** Text and icons on a translucent panel use the
//!   "over" rule for straight alpha ([`over`]). Solid or clear source
//!   pixels and opaque destinations take short paths (two channels per
//!   multiply, an exact divide-by-255 without a divide); only soft edges
//!   on glass pay for real divisions.
//! - **Convert as it flushes.** [`flush`] copies changed rectangles to the
//!   plane as ARGB4444 with a 4x4 ordered dither (the owner's choice) or as
//!   ARGB8888 (one setting, [`PlaneFormat`]). The dither pattern is fixed
//!   to screen positions, so a small flush writes exactly the pixels a full
//!   one would.

use crate::{Rect, fits};

/// An ARGB8888 pixel, `0xAARRGGBB`, straight alpha (0 clear, 255 solid).
pub type Argb = u32;

/// Fully transparent: the wallpaper shows through.
pub const CLEAR: Argb = 0;

/// The two 8-bit lanes the fast paths work on (`0x00FF00FF`).
const LANES: u32 = 0x00FF_00FF;

/// `(x + 127) / 255` in both 16-bit lanes of `v` at once, without a divide.
/// Exact for every lane value up to 255 * 255 (tested against `/`).
#[inline(always)]
const fn div255_lanes(v: u32) -> u32 {
    let t = v + 0x007F_007F;
    ((t + 0x0001_0001 + ((t >> 8) & LANES)) >> 8) & LANES
}

/// `(x + 127) / 255` for one value up to 255 * 255.
#[inline(always)]
const fn div255(x: u32) -> u32 {
    div255_lanes(x) & 0xFF
}

/// `src` over an opaque `dst` at alpha `a` (0..=255): red and blue in one
/// multiply, green in another. Result opaque.
#[inline(always)]
const fn mix_opaque(src: Argb, dst: Argb, a: u32) -> Argb {
    let k = 255 - a;
    let rb = div255_lanes((src & LANES) * a + (dst & LANES) * k);
    let g = div255_lanes((src >> 8 & 0xFF) * a + (dst >> 8 & 0xFF) * k);
    0xFF00_0000 | rb | g << 8
}

/// `src` over `dst`, both straight alpha: the reference the short paths
/// in [`over`] are tested against, and the path soft edges on glass take.
/// Alpha is `sa + da * (1 - sa)`; each colour channel is the alpha-weighted
/// mean of the two, rounded.
// Out of line: the slow path, and every short path in `over` would
// otherwise carry a copy (measured: kilobytes in the bench image).
#[inline(never)]
#[must_use]
pub fn over_reference(src: Argb, dst: Argb) -> Argb {
    let (sa, da) = (src >> 24, dst >> 24);
    let oa = sa + (da * (255 - sa) + 127) / 255;
    if oa == 0 {
        return CLEAR;
    }
    // Weights scaled by 255: sa * 255 for the source, da * (255 - sa) for
    // what shows of the destination.
    let (ws, wd) = (sa * 255, da * (255 - sa));
    let den = oa * 255;
    let ch = |s: u32| {
        let (s, d) = (src >> s & 0xFF, dst >> s & 0xFF);
        ((s * ws + d * wd + den / 2) / den).min(255)
    };
    oa << 24 | ch(16) << 16 | ch(8) << 8 | ch(0)
}

/// `src` over `dst` (straight alpha), the same result as
/// [`over_reference`] for every input, with short paths for the common
/// cases: solid or clear source, clear or opaque destination.
#[inline]
#[must_use]
pub fn over(src: Argb, dst: Argb) -> Argb {
    match (src >> 24, dst >> 24) {
        (255, _) => src,
        // Nothing over nothing is the one clear value, whatever colour
        // either carried.
        (0, 0) => CLEAR,
        (0, _) => dst,
        (_, 0) => src,
        (a, 255) => mix_opaque(src, dst, a),
        _ => over_reference(src, dst),
    }
}

/// The 4x4 ordered (Bayer) dither pattern: each pixel's place in it sets
/// where its channels round.
const BAYER: [[u32; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

/// The rounding threshold for `(x, y)`, out of 255 * 16.
const fn threshold(x: usize, y: usize) -> u32 {
    (2 * BAYER[y % 4][x % 4] + 1) * 255 / 2
}

/// ARGB8888 to ARGB4444 with a 4 × 4 ordered dither: each channel rounds up
/// or down by the pixel's place in the pattern, so 16 levels average out to
/// smooth gradients at 2 bytes a pixel. 0 and 255 stay exact. The look the
/// owner approved on the device (stage E); [`flush`]'s fast path is tested
/// against it for every channel value and pattern place.
#[must_use]
pub const fn to_4444_dithered(c: Argb, x: usize, y: usize) -> u16 {
    const fn q(c: u32, s: u32, t: u32) -> u32 {
        ((c >> s & 0xFF) * 15 * 16 + t) / (255 * 16)
    }
    let t = threshold(x, y);
    (q(c, 24, t) << 12 | q(c, 16, t) << 8 | q(c, 8, t) << 4 | q(c, 0, t)) as u16
}

/// [`to_4444_dithered`] two channels at a time, no divides: each lane holds
/// `v * 240 + t` (at most 65152), and `/ 4080` is `/ 16` then an exact
/// `/ 255`. `tt` is the threshold in both lanes.
#[inline(always)]
const fn dither_fast(c: Argb, tt: u32) -> u16 {
    #[inline(always)]
    const fn lanes(v: u32, tt: u32) -> u32 {
        let z = ((v * 240 + tt) >> 4) & 0x0FFF_0FFF;
        ((z + 0x0001_0001 + ((z >> 8) & 0x000F_000F)) >> 8) & 0x000F_000F
    }
    let rb = lanes(c & LANES, tt);
    let ag = lanes(c >> 8 & LANES, tt);
    (ag >> 4 & 0xF000 | rb >> 8 & 0x0F00 | ag << 4 & 0x00F0 | rb & 0x000F) as u16
}

/// How the layer is written to the graphics plane: the compositor's one
/// format setting (ARGB4444 dithered by default, the owner's choice; 8888
/// kept so it can be switched to if the dither ever shows).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlaneFormat {
    /// 2 bytes a pixel, ordered dither.
    Argb4444Dithered,
    /// 4 bytes a pixel, a plain copy.
    Argb8888,
}

impl PlaneFormat {
    /// Bytes a pixel in the plane.
    #[must_use]
    pub const fn bytes(self) -> usize {
        match self {
            Self::Argb4444Dithered => 2,
            Self::Argb8888 => 4,
        }
    }
}

/// A read-only view of a layer (the source of a flush).
#[derive(Clone, Copy)]
pub struct LayerView<'a> {
    px: &'a [Argb],
    w: u32,
    h: u32,
    stride: usize,
}

impl<'a> LayerView<'a> {
    /// The whole view.
    #[must_use]
    pub fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.w, self.h)
    }

    /// Row `y` (its `w` pixels).
    ///
    /// # Panics
    /// If `y` is past the bottom.
    #[must_use]
    pub fn row(&self, y: u32) -> &'a [Argb] {
        let at = y as usize * self.stride;
        &self.px[at..at + self.w as usize]
    }

    /// The pixel at `(x, y)`.
    #[must_use]
    pub fn get(&self, x: u32, y: u32) -> Option<Argb> {
        (x < self.w && y < self.h).then(|| self.px[y as usize * self.stride + x as usize])
    }
}

/// An image with its own alpha (icons), ARGB8888 straight, row after row.
#[derive(Clone, Copy)]
pub struct ArgbImage<'a> {
    /// Pixels, `w * h`.
    pub px: &'a [Argb],
    /// Width.
    pub w: u32,
    /// Height.
    pub h: u32,
}

/// A coverage mask (a glyph): how much of each pixel the shape covers,
/// 0..=255, row after row.
#[derive(Clone, Copy)]
pub struct Mask<'a> {
    /// Coverage, `w * h`.
    pub cover: &'a [u8],
    /// Width.
    pub w: u32,
    /// Height.
    pub h: u32,
}

/// The interface layer in RAM: ARGB8888, straight alpha.
pub struct Layer<'a> {
    px: &'a mut [Argb],
    w: u32,
    h: u32,
    stride: usize,
}

impl<'a> Layer<'a> {
    /// `w` x `h` pixels, rows `stride` pixels apart; `None` if `px` is too
    /// short or `stride < w`.
    #[must_use]
    pub fn new(px: &'a mut [Argb], w: u32, h: u32, stride: usize) -> Option<Self> {
        fits(px.len(), w, h, stride).then_some(Self { px, w, h, stride })
    }

    /// A `w` x `h` layer in a byte region (scratch RAM), rows packed; `None`
    /// if it's too small or not word aligned.
    #[must_use]
    pub fn in_bytes(bytes: &'a mut [u8], w: u32, h: u32) -> Option<Self> {
        Self::new(rp_hal_mem::bytes_as_words_mut(bytes)?, w, h, w as usize)
    }

    /// The whole layer.
    #[must_use]
    pub fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.w, self.h)
    }

    /// A read-only view of it.
    #[must_use]
    pub fn view(&self) -> LayerView<'_> {
        LayerView {
            px: self.px,
            w: self.w,
            h: self.h,
            stride: self.stride,
        }
    }

    /// The pixel at `(x, y)`.
    #[must_use]
    pub fn get(&self, x: u32, y: u32) -> Option<Argb> {
        self.view().get(x, y)
    }

    /// Row `y`, columns `x..x + w` (already clipped by the caller).
    fn span(&mut self, x: u32, y: u32, w: u32) -> &mut [Argb] {
        let at = y as usize * self.stride + x as usize;
        &mut self.px[at..at + w as usize]
    }

    /// Sets every pixel of `r` (clipped) to `c`, alpha included: a solid
    /// bar, a flat sheet of glass, or [`CLEAR`] to erase.
    // Out of line: the work is per row, so a call costs nothing, and
    // callers filling many rectangles don't each carry the loop.
    #[inline(never)]
    pub fn fill(&mut self, r: Rect, c: Argb) {
        let r = r.intersect(&self.bounds());
        for y in r.y..r.bottom() {
            rp_hal_mem::fill32(rp_hal_mem::words_as_bytes_mut(self.span(r.x, y, r.w)), c);
        }
    }

    /// Sets `r` (clipped) to a horizontal gradient: column `k` of `r`
    /// takes `cols[k]` on every row (columns past `cols` are left alone).
    /// Glass whose tint and opacity vary across its width.
    // Out of line: the work is per row, so a call costs nothing, and
    // callers filling many rectangles don't each carry the loop.
    #[inline(never)]
    pub fn fill_columns(&mut self, r: Rect, cols: &[Argb]) {
        // As `Surface::tint`: clipping only trims the right and bottom, so
        // column k of the clipped rectangle is still `cols[k]`.
        let r = Rect {
            w: r.w.min(u32::try_from(cols.len()).unwrap_or(u32::MAX)),
            ..r
        }
        .intersect(&self.bounds());
        let cols = &cols[..r.w as usize];
        for y in r.y..r.bottom() {
            self.span(r.x, y, r.w).copy_from_slice(cols);
        }
    }

    /// Draws `img` over what's there at `(dx, dy)` (clipped).
    pub fn draw_over(&mut self, img: &ArgbImage<'_>, dx: u32, dy: u32) {
        let n = (img.w as usize).saturating_mul(img.h as usize);
        if img.px.len() < n {
            return;
        }
        let to = Rect::new(dx, dy, img.w, img.h).intersect(&self.bounds());
        for y in to.y..to.bottom() {
            let at = (y - dy) as usize * img.w as usize + (to.x - dx) as usize;
            let src = &img.px[at..at + to.w as usize];
            for (p, &s) in self.span(to.x, y, to.w).iter_mut().zip(src) {
                *p = over(s, *p);
            }
        }
    }

    /// Draws `color` through `mask` at `(dx, dy)` (clipped): each pixel's
    /// coverage scales the colour's alpha, then it goes over what's there.
    pub fn draw_mask(&mut self, mask: &Mask<'_>, color: Argb, dx: u32, dy: u32) {
        let n = (mask.w as usize).saturating_mul(mask.h as usize);
        if mask.cover.len() < n {
            return;
        }
        let to = Rect::new(dx, dy, mask.w, mask.h).intersect(&self.bounds());
        let (rgb, a) = (color & 0x00FF_FFFF, color >> 24);
        for y in to.y..to.bottom() {
            let at = (y - dy) as usize * mask.w as usize + (to.x - dx) as usize;
            let cover = &mask.cover[at..at + to.w as usize];
            for (p, &c) in self.span(to.x, y, to.w).iter_mut().zip(cover) {
                if c != 0 {
                    *p = over(div255(a * u32::from(c)) << 24 | rgb, *p);
                }
            }
        }
    }
}

/// The widest row a flush converts at once (the screen in either
/// orientation).
const MAX_ROW: usize = 640;

/// Copies `rects` of `src` into the graphics plane's bytes `fb` (rows
/// `fb_stride` bytes apart, the layer's size) in format `fmt`: writes only,
/// never reads `fb`. Rectangles are clipped to both; returns the pixels
/// written. For ARGB4444 the rectangles should be [`crate::dirty::widen`]ed
/// (whole words); `Dirty` already does that.
pub fn flush(
    src: &LayerView<'_>,
    fmt: PlaneFormat,
    fb: &mut [u8],
    fb_stride: usize,
    rects: &[Rect],
) -> u64 {
    let bytes = fmt.bytes();
    let mut row = [0u32; MAX_ROW / 2];
    let mut written = 0;
    for r in rects {
        let r = r.intersect(&src.bounds());
        for y in r.y..r.bottom() {
            let at = y as usize * fb_stride + r.x as usize * bytes;
            let Some(dst) = fb.get_mut(at..at + r.w as usize * bytes) else {
                break;
            };
            let px = &src.row(y)[r.x as usize..r.right() as usize];
            match fmt {
                PlaneFormat::Argb8888 => dst.copy_from_slice(rp_hal_mem::words_as_bytes(px)),
                PlaneFormat::Argb4444Dithered => {
                    // Converted into a row in RAM (in pieces if wider than
                    // the screen), then one burst copy out.
                    // Four pixels a step: one turn of the pattern, so the
                    // thresholds stay put, written as two whole words.
                    let y = y as usize;
                    for (k, (part, out)) in px
                        .chunks(MAX_ROW)
                        .zip(dst.chunks_mut(MAX_ROW * 2))
                        .enumerate()
                    {
                        let x0 = r.x as usize + k * MAX_ROW;
                        let t: [u32; 4] =
                            core::array::from_fn(|j| threshold(x0 + j, y) * 0x0001_0001);
                        let (quads, rest) = part.as_chunks::<4>();
                        let words = &mut row[..quads.len() * 2];
                        for (q, w) in quads.iter().zip(words.as_chunks_mut::<2>().0) {
                            let d = |j: usize| u32::from(dither_fast(q[j], t[j]));
                            *w = [d(0) | d(1) << 16, d(2) | d(3) << 16];
                        }
                        let n = quads.len() * 8;
                        out[..n].copy_from_slice(rp_hal_mem::words_as_bytes(words));
                        for (j, (&c, o)) in
                            rest.iter().zip(out[n..].as_chunks_mut::<2>().0).enumerate()
                        {
                            *o = dither_fast(c, t[j]).to_le_bytes();
                        }
                    }
                }
            }
            written += u64::from(r.w);
        }
    }
    written
}

#[cfg(test)]
mod tests;
