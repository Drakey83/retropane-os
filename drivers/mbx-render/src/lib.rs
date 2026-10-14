//! The MBX 2D renderer (ADR-0023, option B): draws `rp-draw` frames on the
//! 2700G's 2D engine into one opaque XRGB8888 picture, double-buffered.
//!
//! For each changed rectangle the engine copies the wallpaper in (kept in
//! 2700G memory as XRGB8888), then draws every command that meets it, over
//! it. Every blend is then onto an opaque pixel, which the engine does
//! exactly (header bit 17, blend word `0x09100000`: source × A +
//! destination × (1 − A), measured 2026-10-14). The CPU only describes:
//!
//! - **Fill / HGradient** replace what's under them in the CPU renderer's
//!   layer (the wallpaper shows through their alpha), so here: wallpaper
//!   in, then the colour over it. Opaque fills are engine fills; others
//!   are one row of colours written by the CPU into a ring in 2700G memory
//!   and blended down the rectangle with **source stride 0** (every line
//!   reads the same row).
//! - **Outline** draws over: four edges, as fills over.
//! - **Image** is uploaded once to a cache (by its `&'static` address),
//!   then blended.
//! - **Text**: each (font, colour) is uploaded once as an ARGB8888 atlas,
//!   the colour baked in and the alpha the CPU renderer's own
//!   `cover × alpha / 255`, then each glyph is blended.
//!
//! The back picture is two frames old, so each frame redraws its own and
//! the previous frame's dirty rectangles ("buffer age 2"); the first two
//! frames redraw everything. One fence per frame, then the flip: the
//! graphics plane's start address moves to the picture just drawn
//! (`flip_wait`, if set, runs first: the image waits for the VEVENT line
//! there, so a half-drawn frame never shows).
//!
//! The renderer touches 2700G memory only through [`LocalMem`] and the
//! engine only through its register window, so the same code runs on the
//! device and, with the `model` feature, against a model of the engine.

#![cfg_attr(not(any(test, feature = "model")), no_std)]
#![forbid(unsafe_code)]

#[cfg(any(test, feature = "model"))]
pub mod model;

use rp_draw::{Argb, Cmd, Drawn, Frame, MAX_DIRTY, Rect, Renderer};
use rp_drv_blit2d::{self as cmd, BLEND_DELL_TEXT, Engine, Format, OP_BLEND, ROP_SRCCOPY};
use rp_hal_api::RegisterBus;

/// Screen width.
pub const W: u16 = 480;
/// Screen height.
pub const H: u16 = 640;
/// Bytes per line of a 32 bpp picture.
pub const STRIDE: usize = W as usize * 4;
/// Bytes in one 32 bpp picture.
pub const SCREEN_BYTES: usize = STRIDE * H as usize;

/// Rows in the ring for stride-0 blits (reset at each frame's fence).
const ROWS: usize = 64;
/// One row: the widest rectangle (the screen in either orientation).
const ROW_BYTES: usize = 640 * 4;
/// Images kept uploaded.
const IMAGES: usize = 32;
/// (font, colour) atlases kept uploaded.
const ATLASES: usize = 8;
/// Glyphs a font may have.
const GLYPHS: usize = 160;

/// Where everything lives in 2700G local memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    /// The two pictures.
    pub screens: [usize; 2],
    /// The wallpaper, XRGB8888, a picture's size.
    pub wallpaper: usize,
    /// The row ring (`ROWS` × 2.5 KiB).
    pub rows: usize,
    /// The image cache and its size.
    pub images: (usize, usize),
    /// The glyph atlases and their size.
    pub glyphs: (usize, usize),
    /// The fence word.
    pub fence: usize,
}

impl Layout {
    /// The X51v's: from 2 MiB up (the loader's and Windows Mobile's planes
    /// sit below 1 MiB), about 4.4 MiB in all (ADR-0023 decision 4).
    pub const X51V: Self = Self {
        screens: [0x0020_0000, 0x0034_0000],
        wallpaper: 0x0048_0000,
        rows: 0x005C_0000,
        images: (0x0060_0000, 256 * 1024),
        glyphs: (0x0064_0000, 512 * 1024),
        fence: 0x00FF_F000,
    };
}

/// 2700G local memory as the CPU sees it.
pub trait LocalMem {
    /// Stores `words` from byte offset `at`.
    fn store(&mut self, at: usize, words: &[u32]);
    /// The word at byte offset `at`.
    fn load(&self, at: usize) -> Option<u32>;
    /// Makes every store so far visible to the engine (drains the CPU's
    /// write buffer on the device).
    fn sync(&mut self);
}

/// `(x + 127) / 255`: the CPU renderer's glyph alpha.
const fn div255(x: u32) -> u32 {
    (x + 127) / 255
}

/// Where a `w` × `h` source placed at `(dx, dy)` lands inside `clip`, and
/// the source pixel its top left shows (the CPU renderer's `place`).
fn place(w: u16, h: u16, dx: i32, dy: i32, clip: Rect) -> Option<(Rect, u16, u16)> {
    let x0 = dx.max(0);
    let y0 = dy.max(0);
    let x1 = (dx + i32::from(w)).max(0);
    let y1 = (dy + i32::from(h)).max(0);
    let on = Rect::new(
        u16::try_from(x0).ok()?,
        u16::try_from(y0).ok()?,
        u16::try_from(x1 - x0).ok()?,
        u16::try_from(y1 - y0).ok()?,
    );
    let to = on.intersect(&clip);
    if to.is_empty() {
        return None;
    }
    let sx = u16::try_from(i32::from(to.x) - dx).ok()?;
    let sy = u16::try_from(i32::from(to.y) - dy).ok()?;
    Some((to, sx, sy))
}

/// One (font, colour)'s glyphs uploaded: their offsets (0 = none).
#[derive(Clone, Copy)]
struct Atlas {
    font: usize,
    color: Argb,
    at: [u32; GLYPHS],
}

/// Draws frames on the 2D engine. `R` is the 2700G register window, `M`
/// its local memory.
pub struct Mbx2dRenderer<R, M> {
    e: Engine<R>,
    mem: M,
    l: Layout,
    /// The picture on screen (0 or 1).
    front: usize,
    prev: [Rect; MAX_DIRTY],
    prev_n: usize,
    /// Frames still to draw whole (the pictures' contents unknown).
    fresh: u8,
    row: usize,
    images: [(usize, usize); IMAGES],
    images_n: usize,
    images_top: usize,
    atlases: [Option<Atlas>; ATLASES],
    glyphs_top: usize,
    /// Runs between the fence and the flip (the image waits for the
    /// VEVENT line here).
    pub flip_wait: Option<fn()>,
    /// Frames the engine didn't finish (each reset it and redraws whole).
    pub stalls: u32,
    delay_us: fn(u32),
}

/// The whole screen.
const SCREEN: Rect = Rect::new(0, 0, W, H);

impl<R: RegisterBus, M: LocalMem> Mbx2dRenderer<R, M> {
    /// A renderer on `regs` (the 2700G register window), `mem`, laid out
    /// as `l`; the clock for the engine's timeouts is `now` at
    /// `ticks_per_ms`. Call [`Self::wallpaper_row`] for every line, then
    /// [`Self::show`].
    pub fn new(regs: R, mem: M, l: Layout, now: fn() -> u32, ticks_per_ms: u32) -> Self {
        Self {
            e: Engine::new(regs, l.fence, now, ticks_per_ms),
            mem,
            l,
            front: 0,
            prev: [Rect::default(); MAX_DIRTY],
            prev_n: 0,
            fresh: 2,
            row: 0,
            images: [(0, 0); IMAGES],
            images_n: 0,
            images_top: 0,
            atlases: [None; ATLASES],
            glyphs_top: 0,
            flip_wait: None,
            stalls: 0,
            delay_us: |_| {},
        }
    }

    /// Stores line `y` of the wallpaper (XRGB8888, `W` pixels).
    pub fn wallpaper_row(&mut self, y: u16, px: &[u32]) {
        let n = px.len().min(usize::from(W));
        self.mem
            .store(self.l.wallpaper + usize::from(y) * STRIDE, &px[..n]);
    }

    /// The register window and memory (the image's own use).
    pub fn parts(&mut self) -> (&mut Engine<R>, &mut M) {
        (&mut self.e, &mut self.mem)
    }

    /// Resets the engine and shows picture 0 as an opaque 32 bpp
    /// graphics plane (format, size, stride and start). The video plane
    /// and cursor are the caller's (`Display::adopt` switches them off).
    pub fn show(&mut self, delay_us: fn(u32)) {
        self.delay_us = delay_us;
        self.e.reset(delay_us);
        self.front = 0;
        self.fresh = 2;
        let regs = rp_drv_mbx_display::Plane {
            format: rp_drv_mbx_display::PixelFormat::Argb8888,
            width: W,
            height: H,
            offset: self.l.screens[0],
            stride: STRIDE,
        }
        .registers();
        let gamma = self.e.regs.read32(rp_drv_mbx_display::reg::GSCTRL) & (1 << 26);
        for (off, v) in regs {
            let v = match off {
                rp_drv_mbx_display::reg::GSCTRL => v | gamma,
                rp_drv_mbx_display::reg::GSCADR => Self::solo(v),
                rp_drv_mbx_display::reg::GBBASE => rp_drv_mbx_display::layers::Layer::SOLO.bbase(v),
                _ => v,
            };
            self.e.regs.write32(off, v);
        }
    }

    fn solo(scadr: u32) -> u32 {
        rp_drv_mbx_display::layers::Layer::SOLO.scadr(scadr)
    }

    /// Points the plane at picture `k`.
    fn present(&mut self, k: usize) {
        let scadr = (1 << 31) | (self.l.screens[k] / 16) as u32;
        self.e
            .regs
            .write32(rp_drv_mbx_display::reg::GSCADR, Self::solo(scadr));
        self.front = k;
    }

    fn submit(&mut self, w: &[u32]) {
        // A FIFO that stays full is caught by the frame's fence.
        let _ = self.e.submit(w);
    }

    /// Engine coordinates of `r` (end exclusive).
    fn coords(r: Rect) -> (u16, u16, u16, u16) {
        (r.x, r.y, r.right(), r.bottom())
    }

    /// Copies the wallpaper under `r` into picture `dst`.
    fn wallpaper(&mut self, dst: usize, r: Rect) {
        let (w, n) = cmd::blit(
            (Format::BPP32, dst, STRIDE as i32),
            (Format::BPP32, self.l.wallpaper, STRIDE as i32),
            (r.x, r.y),
            Self::coords(r),
            ROP_SRCCOPY,
            0,
            0,
        );
        self.submit(&w[..n]);
    }

    /// `r` set to the opaque colour `c`.
    fn fill(&mut self, dst: usize, r: Rect, c: Argb) {
        let w = cmd::fill(Format::BPP32, dst, STRIDE, Self::coords(r), c | 0xFF00_0000);
        self.submit(&w);
    }

    /// One row of colours, `r.w` long, drawn down `r` (blended if `blend`).
    fn row(&mut self, dst: usize, r: Rect, cols: &[Argb], blend: bool) {
        if self.row == ROWS {
            self.wait();
            self.row = 0;
        }
        let at = self.l.rows + self.row * ROW_BYTES;
        self.row += 1;
        self.mem
            .store(at, &cols[..usize::from(r.w).min(cols.len())]);
        self.mem.sync();
        let (w, n) = cmd::blit(
            (Format::BPP32, dst, STRIDE as i32),
            (Format::BPP32, at, 0),
            (0, 0),
            Self::coords(r),
            ROP_SRCCOPY,
            if blend { OP_BLEND } else { 0 },
            0,
        );
        self.submit(&w[..n]);
    }

    /// `c` over `r` (nothing if clear, a fill if opaque).
    fn over(&mut self, dst: usize, r: Rect, c: Argb) {
        match c >> 24 {
            _ if r.is_empty() => {}
            0 => {}
            255 => self.fill(dst, r, c),
            _ => {
                let cols = [c; 640];
                self.row(dst, r, &cols, true);
            }
        }
    }

    /// Blends the `to` part of a source at `src` (`stride` bytes a line)
    /// starting at its pixel `(sx, sy)` onto picture `dst`.
    fn blend_from(
        &mut self,
        dst: usize,
        to: Rect,
        (src, stride): (usize, usize),
        sx: u16,
        sy: u16,
    ) {
        let (w, n) = cmd::blit(
            (Format::BPP32, dst, STRIDE as i32),
            (Format::BPP32, src, stride as i32),
            (sx, sy),
            Self::coords(to),
            ROP_SRCCOPY,
            OP_BLEND,
            0,
        );
        self.submit(&w[..n]);
    }

    /// Waits for everything queued (to reuse rows or cache space).
    fn wait(&mut self) -> bool {
        let mem = &self.mem;
        let ok = self.e.finish_with(200_000, |at| mem.load(at)).is_ok();
        if !ok {
            self.stalls += 1;
            self.fresh = 2;
        }
        ok
    }

    /// Where `img` is uploaded (uploading it now if it isn't).
    fn image(&mut self, img: &'static rp_draw::Image) -> Option<usize> {
        let key = img.px.as_ptr() as usize;
        if let Some(&(_, at)) = self.images[..self.images_n].iter().find(|(k, _)| *k == key) {
            return Some(at);
        }
        let n = usize::from(img.w) * usize::from(img.h);
        let px = img.px.get(..n)?;
        let bytes = n * 4;
        if bytes > self.l.images.1 {
            return None;
        }
        if self.images_n == IMAGES || self.images_top + bytes > self.l.images.1 {
            // Full: everything queued finishes, then the cache starts again.
            self.wait();
            self.images_n = 0;
            self.images_top = 0;
        }
        let at = self.l.images.0 + self.images_top;
        self.mem.store(at, px);
        self.mem.sync();
        self.images[self.images_n] = (key, at);
        self.images_n += 1;
        self.images_top += bytes.next_multiple_of(16);
        Some(at)
    }

    /// The atlas for (`font`, `color`), uploading it if needed.
    fn atlas(&mut self, font: &'static rp_draw::Font, color: Argb) -> Option<usize> {
        let key = font as *const rp_draw::Font as usize;
        if let Some(k) = self
            .atlases
            .iter()
            .position(|a| a.is_some_and(|a| a.font == key && a.color == color))
        {
            return Some(k);
        }
        let bytes: usize = font
            .glyphs
            .iter()
            .map(|g| (usize::from(g.w) * usize::from(g.h) * 4).next_multiple_of(16))
            .sum();
        if bytes > self.l.glyphs.1 || font.glyphs.len() > GLYPHS {
            return None;
        }
        let slot = match self.atlases.iter().position(Option::is_none) {
            Some(k) if self.glyphs_top + bytes <= self.l.glyphs.1 => k,
            _ => {
                self.wait();
                self.atlases = [None; ATLASES];
                self.glyphs_top = 0;
                0
            }
        };
        let (rgb, a) = (color & 0x00FF_FFFF, color >> 24);
        let mut atlas = Atlas {
            font: key,
            color,
            at: [0; GLYPHS],
        };
        let mut line = [0u32; 256];
        for (k, g) in font.glyphs.iter().enumerate() {
            let (w, h) = (usize::from(g.w), usize::from(g.h));
            if w == 0 || h == 0 || g.cover.len() < w * h || w > line.len() {
                continue;
            }
            let at = self.l.glyphs.0 + self.glyphs_top;
            for y in 0..h {
                for (p, &c) in line.iter_mut().zip(&g.cover[y * w..y * w + w]) {
                    *p = div255(a * u32::from(c)) << 24 | rgb;
                }
                self.mem.store(at + y * w * 4, &line[..w]);
            }
            atlas.at[k] = at as u32;
            self.glyphs_top += (w * h * 4).next_multiple_of(16);
        }
        self.mem.sync();
        self.atlases[slot] = Some(atlas);
        Some(slot)
    }

    fn command(&mut self, frame: &Frame, c: &Cmd, clip: Rect, dst: usize) {
        match *c {
            Cmd::Fill { r, color } => {
                let t = r.intersect(&clip);
                if t.is_empty() {
                    return;
                }
                if color >> 24 != 255 {
                    self.wallpaper(dst, t);
                }
                self.over(dst, t, color);
            }
            Cmd::HGradient { r, left, right } => {
                let t = r.intersect(&clip);
                if t.is_empty() {
                    return;
                }
                let mut cols = [0u32; 640];
                let first = u32::from(t.x - r.x);
                for (k, p) in cols.iter_mut().take(usize::from(t.w)).enumerate() {
                    *p = rp_gfx::render::gradient_at(left, right, first + k as u32, u32::from(r.w));
                }
                let cols = &cols[..usize::from(t.w)];
                let opaque = cols.iter().all(|p| p >> 24 == 255);
                if !opaque {
                    self.wallpaper(dst, t);
                }
                self.row(dst, t, cols, !opaque);
            }
            Cmd::Outline { r, color } => {
                if r.is_empty() {
                    return;
                }
                let inner = r.h.saturating_sub(2);
                for edge in [
                    Rect::new(r.x, r.y, r.w, 1),
                    Rect::new(r.x, r.bottom() - 1, r.w, u16::from(r.h > 1)),
                    Rect::new(r.x, r.y + 1, 1, inner),
                    Rect::new(r.right() - 1, r.y + 1, u16::from(r.w > 1), inner),
                ] {
                    self.over(dst, edge.intersect(&clip), color);
                }
            }
            Cmd::Image { x, y, img } => {
                let Some((to, sx, sy)) = place(img.w, img.h, i32::from(x), i32::from(y), clip)
                else {
                    return;
                };
                if let Some(at) = self.image(img) {
                    self.blend_from(dst, to, (at, usize::from(img.w) * 4), sx, sy);
                }
            }
            Cmd::Text {
                x,
                baseline,
                font,
                color,
                text,
            } => {
                if color >> 24 == 0 {
                    return;
                }
                let Some(k) = self.atlas(font, color) else {
                    return;
                };
                let mut pen = i32::from(x);
                for &ch in frame.text_of(text) {
                    let Some(g) = font.glyph(ch) else {
                        continue;
                    };
                    let gx = pen + i32::from(g.left);
                    let gy = i32::from(baseline) - i32::from(g.top);
                    pen += i32::from(g.advance);
                    let idx = usize::from(ch - font.first);
                    let at = self.atlases[k].map_or(0, |a| a.at[idx]) as usize;
                    if at == 0 {
                        continue;
                    }
                    let (w, h) = (u16::from(g.w), u16::from(g.h));
                    if let Some((to, sx, sy)) = place(w, h, gx, gy, clip) {
                        self.blend_from(dst, to, (at, usize::from(w) * 4), sx, sy);
                    }
                }
            }
        }
    }
}

impl<R: RegisterBus, M: LocalMem> Renderer for Mbx2dRenderer<R, M> {
    fn draw(&mut self, frame: &Frame) -> Drawn {
        let mut drawn = Drawn::default();
        let mut rects = [Rect::default(); 2 * MAX_DIRTY];
        let mut n = 0;
        if self.fresh > 0 {
            rects[0] = SCREEN;
            n = 1;
        } else {
            for r in frame.dirty().iter().chain(&self.prev[..self.prev_n]) {
                let r = r.intersect(&SCREEN);
                if !r.is_empty() && !rects[..n].iter().any(|d| d.intersect(&r) == r) {
                    rects[n] = r;
                    n += 1;
                }
            }
        }
        self.prev_n = frame.dirty().len().min(MAX_DIRTY);
        self.prev[..self.prev_n].copy_from_slice(&frame.dirty()[..self.prev_n]);
        if n == 0 {
            return drawn;
        }
        let back = 1 - self.front;
        let dst = self.l.screens[back];
        self.row = 0;
        let mut state = [0u32; 6];
        state[0] = cmd::SCALE_1_TO_1;
        state[1..4].copy_from_slice(&cmd::key(0, 0));
        state[4..].copy_from_slice(&cmd::blend(BLEND_DELL_TEXT));
        self.submit(&state);
        for &clip in &rects[..n] {
            self.wallpaper(dst, clip);
            for c in frame.cmds() {
                if c.bounds(frame).intersect(&clip).is_empty() {
                    continue;
                }
                self.command(frame, c, clip, dst);
                drawn.cmds += 1;
            }
            drawn.pixels += u64::from(clip.w) * u64::from(clip.h);
        }
        if !self.wait() {
            self.e.reset(self.delay_us);
            return drawn;
        }
        self.fresh = self.fresh.saturating_sub(1);
        if let Some(w) = self.flip_wait {
            w();
        }
        self.present(back);
        drawn
    }
}

#[cfg(test)]
mod tests;
