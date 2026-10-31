//! The CPU renderer (ADR-0021): draws an `rp_draw::Frame` into the
//! interface [`Layer`] and flushes the changed rectangles to the 2700G's
//! graphics plane.
//!
//! For each dirty rectangle: clear it, draw every command that meets it
//! (clipped to it), in order. Then one flush of all of them, widened to
//! whole words, in the plane's format. Commands that miss a rectangle cost
//! a bounds check. The MBX renderer (item 69) replaces this whole type and
//! nothing else.

use rp_draw::{Cmd, Drawn, Frame, Renderer};

use crate::Rect;
use crate::layer::{self, Argb, ArgbImage, CLEAR, Layer, Mask, PlaneFormat};

/// The widest gradient (the screen in either orientation).
const MAX_W: usize = 640;

/// An `rp_draw` rectangle as `rp-gfx`'s.
#[must_use]
pub fn rect(r: rp_draw::Rect) -> Rect {
    Rect::new(
        u32::from(r.x),
        u32::from(r.y),
        u32::from(r.w),
        u32::from(r.h),
    )
}

/// Column `k` of `w` going from `left` to `right`, each channel rounded.
#[must_use]
pub fn gradient_at(left: Argb, right: Argb, k: u32, w: u32) -> Argb {
    let span = w.saturating_sub(1).max(1);
    let k = k.min(span);
    let mut out = 0;
    for shift in [0, 8, 16, 24] {
        let (a, b) = ((left >> shift) & 0xFF, (right >> shift) & 0xFF);
        // a + (b - a) * k / span, rounded, without going negative.
        let v = (a * (span - k) + b * k + span / 2) / span;
        out |= v << shift;
    }
    out
}

/// Draws frames on the CPU into a layer, out to a plane.
pub struct CpuRenderer<'a> {
    layer: Layer<'a>,
    plane: &'a mut [u8],
    stride: usize,
    fmt: PlaneFormat,
}

impl<'a> CpuRenderer<'a> {
    /// Draws into `layer`, flushing to `plane` (rows `stride` bytes apart,
    /// the layer's size) in `fmt`.
    pub fn new(layer: Layer<'a>, plane: &'a mut [u8], stride: usize, fmt: PlaneFormat) -> Self {
        Self {
            layer,
            plane,
            stride,
            fmt,
        }
    }

    /// The layer as drawn (tests, screenshots).
    #[must_use]
    pub fn layer(&self) -> &Layer<'a> {
        &self.layer
    }

    /// Paints `r` of the layer with `paint(pixels, stride)` (what the
    /// draw commands can't: Hourly's curve, a line per pixel), then flushes
    /// `r` out to the plane as a draw does.
    pub fn paint(&mut self, r: Rect, paint: impl FnOnce(&mut [Argb], usize)) {
        let r = r.intersect(&self.layer.bounds());
        if r.is_empty() {
            return;
        }
        let (px, stride) = self.layer.pixels_mut();
        paint(px, stride);
        let out = [crate::dirty::widen(r).intersect(&self.layer.bounds())];
        let _ = layer::flush(&self.layer.view(), self.fmt, self.plane, self.stride, &out);
    }

    /// The plane's bytes (tests).
    #[must_use]
    pub fn plane(&self) -> &[u8] {
        self.plane
    }

    fn command(&mut self, frame: &Frame, cmd: &Cmd, clip: Rect) {
        match *cmd {
            Cmd::Fill { r, color } => self.layer.fill(rect(r).intersect(&clip), color),
            Cmd::Clip { .. } => {}
            Cmd::HGradient { r, left, right } => {
                let full = rect(r);
                let to = full.intersect(&clip);
                if to.is_empty() {
                    return;
                }
                let mut cols = [0; MAX_W];
                let first = to.x - full.x;
                for (k, c) in cols.iter_mut().take(to.w as usize).enumerate() {
                    *c = gradient_at(left, right, first + k as u32, full.w);
                }
                self.layer
                    .fill_columns(to, &cols[..(to.w as usize).min(MAX_W)]);
            }
            Cmd::Outline { r, color } => {
                let r = rect(r);
                if r.is_empty() {
                    return;
                }
                let inner = r.h.saturating_sub(2);
                for edge in [
                    Rect::new(r.x, r.y, r.w, 1),
                    Rect::new(r.x, r.bottom() - 1, r.w, u32::from(r.h > 1)),
                    Rect::new(r.x, r.y + 1, 1, inner),
                    Rect::new(r.right() - 1, r.y + 1, u32::from(r.w > 1), inner),
                ] {
                    self.layer.fill_over(edge.intersect(&clip), color);
                }
            }
            Cmd::Image { x, y, img } => {
                let src = ArgbImage {
                    px: img.px,
                    w: u32::from(img.w),
                    h: u32::from(img.h),
                };
                self.layer
                    .draw_over_in(&src, i32::from(x), i32::from(y), clip);
            }
            Cmd::Text {
                x,
                baseline,
                font,
                color,
                text,
            } => {
                let mut pen = i32::from(x);
                for &c in frame.text_of(text) {
                    let Some(g) = font.glyph(c) else {
                        continue;
                    };
                    let mask = Mask {
                        cover: g.cover,
                        w: u32::from(g.w),
                        h: u32::from(g.h),
                    };
                    let gx = pen + i32::from(g.left);
                    let gy = i32::from(baseline) - i32::from(g.top);
                    self.layer.draw_mask_in(&mask, color, gx, gy, clip);
                    pen += i32::from(g.advance);
                }
            }
        }
    }
}

/// Dirty rectangles a flush takes at once (`rp_draw::MAX_DIRTY`).
const MAX_DIRTY: usize = rp_draw::MAX_DIRTY;

impl Renderer for CpuRenderer<'_> {
    fn draw(&mut self, frame: &Frame) -> Drawn {
        let mut drawn = Drawn::default();
        let mut out = [Rect::default(); MAX_DIRTY];
        let mut n = 0;
        for d in frame.dirty() {
            let clip = rect(*d).intersect(&self.layer.bounds());
            if clip.is_empty() {
                continue;
            }
            self.layer.fill(clip, CLEAR);
            for (cmd, own) in frame.cmds_clipped() {
                let clip = rect(own).intersect(&clip);
                if rect(cmd.bounds(frame)).intersect(&clip).is_empty() {
                    continue;
                }
                self.command(frame, cmd, clip);
                drawn.cmds += 1;
            }
            if let Some(slot) = out.get_mut(n) {
                *slot = crate::dirty::widen(clip).intersect(&self.layer.bounds());
                n += 1;
            }
        }
        drawn.pixels = layer::flush(
            &self.layer.view(),
            self.fmt,
            self.plane,
            self.stride,
            &out[..n],
        );
        drawn
    }
}

#[cfg(test)]
mod tests;
