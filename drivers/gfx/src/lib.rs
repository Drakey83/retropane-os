//! Pixel paths for the X51v's 16-bit screen (overnight item 52).
//!
//! Everything the shell draws goes through here, in one place, so it can be
//! measured and made fast once:
//!
//! - **Compose in RAM, write the framebuffer only.** The 2700G's memory is
//!   slow to read (docs/hw/2700g.md: 61 ms vs 14.5 ms for a screen), so
//!   drawing happens on a [`Surface`] in SDRAM and only changed rectangles
//!   are copied out ([`dirty`]). Copies are widened to whole 32-bit words so
//!   they always take `memcpy`'s burst path.
//! - **Blend three channels in one multiply.** An RGB565 pixel "spread" into
//!   a word (`0x07E0F81F` mask) leaves room above each channel, so
//!   `(src * a + dst * (32 - a)) >> 5` blends red, green and blue at once
//!   with 5-bit alpha, exactly the same result as doing each channel alone
//!   (tests compare every case against that reference).
//! - **Glass is a precomputed gradient.** A tinted-glass panel (the the reference
//!   look, docs/design/ui.md) varies only across its width, so each column's
//!   premultiplied colour is computed once ([`Gradient`]), not per pixel.
//! - **Nothing trusts its inputs.** Rectangles are clipped with saturating
//!   arithmetic; a hostile rectangle draws nothing rather than panicking.
//!
//! No `unsafe`: byte views of pixels come from `rp-hal-mem`.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod dirty;
pub mod layer;
pub mod render;

/// A 16-bit RGB565 pixel.
pub type Px = u16;

/// A rectangle: `x..x + w` by `y..y + h`. Empty when `w` or `h` is 0.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Rect {
    /// Left edge.
    pub x: u32,
    /// Top edge.
    pub y: u32,
    /// Width.
    pub w: u32,
    /// Height.
    pub h: u32,
}

impl Rect {
    /// A rectangle.
    #[must_use]
    pub const fn new(x: u32, y: u32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }

    /// Whether it covers no pixel.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }

    /// One past the right edge (saturating).
    #[must_use]
    pub const fn right(&self) -> u32 {
        self.x.saturating_add(self.w)
    }

    /// One past the bottom edge (saturating).
    #[must_use]
    pub const fn bottom(&self) -> u32 {
        self.y.saturating_add(self.h)
    }

    /// Pixels covered.
    #[must_use]
    pub const fn area(&self) -> u64 {
        self.w as u64 * self.h as u64
    }

    /// The overlap (empty if none).
    #[must_use]
    pub fn intersect(&self, o: &Rect) -> Rect {
        let (x0, y0) = (self.x.max(o.x), self.y.max(o.y));
        let (x1, y1) = (self.right().min(o.right()), self.bottom().min(o.bottom()));
        if x1 <= x0 || y1 <= y0 {
            Rect::default()
        } else {
            Rect::new(x0, y0, x1 - x0, y1 - y0)
        }
    }

    /// The smallest rectangle covering both (an empty one counts as nothing).
    #[must_use]
    pub fn union(&self, o: &Rect) -> Rect {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        let (x0, y0) = (self.x.min(o.x), self.y.min(o.y));
        let (x1, y1) = (self.right().max(o.right()), self.bottom().max(o.bottom()));
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }

    /// Whether `o` lies entirely inside this one.
    #[must_use]
    pub fn contains(&self, o: &Rect) -> bool {
        o.is_empty() || self.intersect(o) == *o
    }
}

/// RGB565 from 8-bit channels (truncating).
#[must_use]
pub const fn rgb(r: u8, g: u8, b: u8) -> Px {
    ((r as u16 >> 3) << 11) | ((g as u16 >> 2) << 5) | (b as u16 >> 3)
}

const SPREAD: u32 = 0x07E0_F81F;

/// The pixel with green moved up, leaving gaps above each channel.
#[inline(always)]
const fn spread(p: Px) -> u32 {
    (p as u32 | (p as u32) << 16) & SPREAD
}

#[inline(always)]
const fn unspread(v: u32) -> Px {
    let v = v & SPREAD;
    (v | v >> 16) as Px
}

/// 8-bit alpha to the 0..=32 scale the blends use.
#[must_use]
pub const fn alpha5(a: u8) -> u32 {
    (a as u32 * 33) >> 8
}

/// `src` over `dst` with alpha `a` (0..=32): every channel blended at once.
#[inline(always)]
#[must_use]
pub const fn blend(src: Px, dst: Px, a: u32) -> Px {
    unspread((spread(src) * a + spread(dst) * (32 - a)) >> 5)
}

/// The same blend, one channel at a time: the reference the fast paths are
/// tested against (and the "before" in the benchmarks).
#[must_use]
pub fn blend_reference(src: Px, dst: Px, a: u32) -> Px {
    let ch = |s: u32, d: u32| (s * a + d * (32 - a)) >> 5;
    let (s, d) = (u32::from(src), u32::from(dst));
    let r = ch(s >> 11, d >> 11);
    let g = ch((s >> 5) & 63, (d >> 5) & 63);
    let b = ch(s & 31, d & 31);
    (r << 11 | g << 5 | b) as Px
}

/// A colour and how opaque it is (0 clear, 255 solid).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Tint {
    /// The colour.
    pub color: Px,
    /// Opacity.
    pub alpha: u8,
}

/// The widest gradient (the screen's width in either orientation).
pub const MAX_WIDTH: usize = 640;

/// A horizontal glass gradient, precomputed per column: the premultiplied
/// colour and the destination's weight. ~3.2 KB, so keep one in static or
/// heap memory, not on a small stack.
pub struct Gradient {
    width: usize,
    pre: [u32; MAX_WIDTH],
    keep: [u8; MAX_WIDTH],
}

impl Default for Gradient {
    fn default() -> Self {
        Self::new()
    }
}

impl Gradient {
    /// An empty gradient (draws nothing until [`set`](Self::set)).
    #[must_use]
    pub const fn new() -> Self {
        Self {
            width: 0,
            pre: [0; MAX_WIDTH],
            keep: [32; MAX_WIDTH],
        }
    }

    /// From `from` at the left edge to `to` at the right, over `width`
    /// columns (at most [`MAX_WIDTH`]): colour and opacity both interpolated.
    pub fn set(&mut self, width: u32, from: Tint, to: Tint) {
        let w = (width as usize).min(MAX_WIDTH);
        self.width = w;
        let ch = |c: Px| {
            [
                u32::from(c >> 11),
                u32::from((c >> 5) & 63),
                u32::from(c & 31),
            ]
        };
        let (a, b) = (ch(from.color), ch(to.color));
        let span = (w.max(2) - 1) as u32;
        for col in 0..w {
            let t = col as u32;
            let lerp = |p: u32, q: u32| (p * (span - t) + q * t + span / 2) / span;
            let c = (lerp(a[0], b[0]) << 11 | lerp(a[1], b[1]) << 5 | lerp(a[2], b[2])) as Px;
            let al = alpha5(lerp(u32::from(from.alpha), u32::from(to.alpha)) as u8);
            self.pre[col] = spread(c) * al;
            self.keep[col] = (32 - al) as u8;
        }
    }

    /// Columns it covers.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.width as u32
    }
}

/// A read-only view of pixels (a source for copies).
#[derive(Clone, Copy)]
pub struct View<'a> {
    px: &'a [Px],
    w: u32,
    h: u32,
    stride: usize,
}

impl<'a> View<'a> {
    /// `w` x `h` pixels, rows `stride` pixels apart; `None` if `px` is too
    /// short or `stride < w`.
    #[must_use]
    pub fn new(px: &'a [Px], w: u32, h: u32, stride: usize) -> Option<Self> {
        fits(px.len(), w, h, stride).then_some(Self { px, w, h, stride })
    }

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
    pub fn row(&self, y: u32) -> &'a [Px] {
        let at = y as usize * self.stride;
        &self.px[at..at + self.w as usize]
    }

    /// The pixel at `(x, y)` (tests and pickers).
    #[must_use]
    pub fn get(&self, x: u32, y: u32) -> Option<Px> {
        (x < self.w && y < self.h).then(|| self.px[y as usize * self.stride + x as usize])
    }
}

/// An icon or glyph with its own 8-bit alpha per pixel.
#[derive(Clone, Copy)]
pub struct AlphaImage<'a> {
    /// Colours, `w * h`, row after row.
    pub px: &'a [Px],
    /// Opacity per pixel, same layout.
    pub alpha: &'a [u8],
    /// Width.
    pub w: u32,
    /// Height.
    pub h: u32,
}

fn fits(len: usize, w: u32, h: u32, stride: usize) -> bool {
    let (w, h) = (w as usize, h as usize);
    stride >= w
        && (h == 0
            || (h - 1)
                .checked_mul(stride)
                .and_then(|n| n.checked_add(w))
                .is_some_and(|n| n <= len))
}

/// A surface to draw on: pixels in RAM.
pub struct Surface<'a> {
    px: &'a mut [Px],
    w: u32,
    h: u32,
    stride: usize,
}

impl<'a> Surface<'a> {
    /// `w` x `h` pixels, rows `stride` pixels apart; `None` if `px` is too
    /// short or `stride < w`.
    #[must_use]
    pub fn new(px: &'a mut [Px], w: u32, h: u32, stride: usize) -> Option<Self> {
        fits(px.len(), w, h, stride).then_some(Self { px, w, h, stride })
    }

    /// The whole surface.
    #[must_use]
    pub fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.w, self.h)
    }

    /// A read-only view of it.
    #[must_use]
    pub fn view(&self) -> View<'_> {
        View {
            px: self.px,
            w: self.w,
            h: self.h,
            stride: self.stride,
        }
    }

    /// The pixel at `(x, y)`.
    #[must_use]
    pub fn get(&self, x: u32, y: u32) -> Option<Px> {
        self.view().get(x, y)
    }

    /// Row `y`, columns `x..x + w` (already clipped by the caller).
    fn span(&mut self, x: u32, y: u32, w: u32) -> &mut [Px] {
        let at = y as usize * self.stride + x as usize;
        &mut self.px[at..at + w as usize]
    }

    /// Fills `r` (clipped) with `c`.
    pub fn fill(&mut self, r: Rect, c: Px) {
        let r = r.intersect(&self.bounds());
        // `fill32`'s burst stores: 16x fewer instructions than `[u16]::fill`
        // (emulator benchmark `gfx_fill_vga`).
        let word = u32::from(c) | u32::from(c) << 16;
        for y in r.y..r.bottom() {
            rp_hal_mem::fill32(
                rp_hal_mem::pixels_as_bytes_mut(self.span(r.x, y, r.w)),
                word,
            );
        }
    }

    /// Blends `c` at opacity `alpha` over `r` (clipped).
    pub fn blend_fill(&mut self, r: Rect, c: Px, alpha: u8) {
        let a = alpha5(alpha);
        let r = r.intersect(&self.bounds());
        let pre = spread(c) * a;
        let keep = 32 - a;
        for y in r.y..r.bottom() {
            for p in self.span(r.x, y, r.w) {
                *p = unspread((pre + spread(*p) * keep) >> 5);
            }
        }
    }

    /// Tints `r` (clipped) with glass: column `x` of `r` takes the
    /// gradient's column `x - r.x` (columns past its width are left alone).
    pub fn tint(&mut self, r: Rect, g: &Gradient) {
        // Clipping only ever trims the right and bottom (x and y start at 0),
        // so column k of the clipped rectangle is still gradient column k.
        let r = Rect {
            w: r.w.min(g.width()),
            ..r
        }
        .intersect(&self.bounds());
        let cols = r.w as usize;
        for y in r.y..r.bottom() {
            let row = self.span(r.x, y, r.w);
            for ((p, &pre), &keep) in row.iter_mut().zip(&g.pre[..cols]).zip(&g.keep[..cols]) {
                *p = unspread((pre + spread(*p) * u32::from(keep)) >> 5);
            }
        }
    }

    /// Copies `from` (clipped to `src`) to `(dx, dy)` here (clipped too).
    pub fn copy_from(&mut self, src: &View<'_>, from: Rect, dx: u32, dy: u32) {
        let from = from.intersect(&src.bounds());
        let to = Rect::new(dx, dy, from.w, from.h).intersect(&self.bounds());
        let sx = (from.x + (to.x - dx)) as usize;
        let sy = from.y + (to.y - dy);
        for k in 0..to.h {
            let s = src.row(sy + k);
            self.span(to.x, to.y + k, to.w)
                .copy_from_slice(&s[sx..sx + to.w as usize]);
        }
    }

    /// Moves `from` within this surface to `(dx, dy)` (overlap allowed: for
    /// scrolling).
    pub fn copy_within(&mut self, from: Rect, dx: u32, dy: u32) {
        let from = from.intersect(&self.bounds());
        let to = Rect::new(dx, dy, from.w, from.h).intersect(&self.bounds());
        if to.is_empty() {
            return;
        }
        let sx = (from.x + (to.x - dx)) as usize;
        let sy = from.y + (to.y - dy);
        let w = to.w as usize;
        let mut step = |k: u32| {
            let s = (sy + k) as usize * self.stride + sx;
            let d = (to.y + k) as usize * self.stride + to.x as usize;
            self.px.copy_within(s..s + w, d);
        };
        // Moving down: last row first, so no source row is overwritten early.
        if to.y > sy {
            (0..to.h).rev().for_each(&mut step);
        } else {
            (0..to.h).for_each(&mut step);
        }
    }

    /// Draws `img` with its alpha at `(dx, dy)` (clipped).
    pub fn draw_alpha(&mut self, img: &AlphaImage<'_>, dx: u32, dy: u32) {
        let n = (img.w as usize).saturating_mul(img.h as usize);
        if img.px.len() < n || img.alpha.len() < n {
            return;
        }
        let to = Rect::new(dx, dy, img.w, img.h).intersect(&self.bounds());
        for y in to.y..to.bottom() {
            let at = (y - dy) as usize * img.w as usize + (to.x - dx) as usize;
            let src = &img.px[at..at + to.w as usize];
            let al = &img.alpha[at..at + to.w as usize];
            for ((p, &s), &a) in self.span(to.x, y, to.w).iter_mut().zip(src).zip(al) {
                match alpha5(a) {
                    0 => {}
                    32 => *p = s,
                    a => *p = blend(s, *p, a),
                }
            }
        }
    }

    /// Converts a row of 8-bit RGB (3 bytes a pixel) into row `y` from
    /// column `x`, with a 4x4 ordered dither so gradients don't band at 16
    /// bits (photos and wallpapers, converted once). Clipped.
    pub fn put_rgb888_row(&mut self, x: u32, y: u32, rgb: &[u8]) {
        let r = Rect::new(x, y, (rgb.len() / 3) as u32, 1).intersect(&self.bounds());
        if r.is_empty() {
            return;
        }
        let skip = (r.x - x) as usize;
        let row = self.span(r.x, y, r.w);
        for (k, (p, c)) in row
            .iter_mut()
            .zip(rgb[skip * 3..].as_chunks::<3>().0)
            .enumerate()
        {
            let d = BAYER[(y & 3) as usize][(r.x as usize + k) & 3] * 16 + 8;
            let q = |v: u8, max: u32| (u32::from(v) * max + d) / 255;
            *p = (q(c[0], 31) << 11 | q(c[1], 63) << 5 | q(c[2], 31)) as Px;
        }
    }
}

const BAYER: [[u32; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

#[cfg(test)]
mod tests;
