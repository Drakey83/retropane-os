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
//! frames redraw everything.
//!
//! **Speed (after the first device run, 2026-10-14: 33 ms median, no
//! better than the CPU):**
//!
//! - **The CPU doesn't wait.** `draw` queues the frame and a fence and
//!   returns; [`Renderer::poll`] flips once the fence has landed. Only the
//!   next `draw` waits, if the engine is still busy, and for scan-out to
//!   leave the picture it will draw into (the VEVENT0 status bit, set at
//!   the end of each frame's active lines, measured).
//! - **The glass is drawn once.** Fills, gradients and outlines that come
//!   before anything they'd cover ("hoistable": moving them first changes
//!   nothing) go into a **background picture**, wallpaper included, kept
//!   in 2700G memory and rebuilt only when they change (a hash of them).
//!   Each changed rectangle then starts with one plain copy of it.
//! - **Dithered.** The panel shows 16 or 18 bits (measured: 256-level
//!   ramps band), so each redrawn rectangle ends with a 4 x 4 ordered
//!   dither: the engine adds a threshold pattern (blend word 0: source +
//!   destination, saturating, measured) and the panel's truncation does the
//!   rest. Four blits per rectangle, one per line phase (destination stride
//!   x 4, the pattern row read with stride 0).
//!
//! The renderer touches 2700G memory only through [`LocalMem`] and the
//! engine only through its register window, so the same code runs on the
//! device and, with the `model` feature, against a model of the engine.

#![cfg_attr(not(any(test, feature = "model")), no_std)]
#![forbid(unsafe_code)]

#[cfg(any(test, feature = "model"))]
pub mod model;

use rp_draw::{Argb, Cmd, Drawn, Frame, MAX_CMDS, MAX_DIRTY, Rect, Renderer};
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
    /// The background: wallpaper and hoisted glass, a picture's size.
    pub background: usize,
    /// The dither pattern's four rows (`W` pixels each, `STRIDE` apart).
    pub dither: usize,
    /// The fence word.
    pub fence: usize,
}

impl Layout {
    /// The X51v's: from 2 MiB up (the loader's and Windows Mobile's planes
    /// sit below 1 MiB), about 5.6 MiB in all (ADR-0023 decision 4).
    pub const X51V: Self = Self {
        screens: [0x0020_0000, 0x0034_0000],
        wallpaper: 0x0048_0000,
        rows: 0x005C_0000,
        images: (0x0060_0000, 256 * 1024),
        glyphs: (0x0064_0000, 512 * 1024),
        background: 0x006C_0000,
        dither: 0x0080_0000,
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

/// Words the CPU stages at a time (rows and glyphs stream through it:
/// the stack is 16 KiB, and a 2.5 KiB row buffer per call overflowed it).
const CHUNK: usize = 32;

/// One (font, colour)'s glyphs uploaded: each one's offset in the glyph
/// region in 16-byte units, plus 1 (0 = none).
#[derive(Clone, Copy)]
struct Atlas {
    font: usize,
    color: Argb,
    at: [u16; GLYPHS],
}

/// What one frame cost, for the run log (microseconds).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameStats {
    /// The CPU's time describing the frame (commands, rows, uploads).
    pub queue_us: u32,
    /// Waited at the start of `draw` for the previous frame's engine work
    /// and for scan-out to leave the picture drawn into.
    pub wait_us: u32,
    /// From queueing the fence to the flip (as seen by `poll`: an upper
    /// bound, set by how often it's called).
    pub engine_us: u32,
    /// Rectangles redrawn.
    pub rects: u32,
    /// Whether the background was rebuilt.
    pub background: bool,
}

/// The 4 x 4 ordered (Bayer) pattern, 0..16.
const BAYER: [[u32; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

/// The dither added at `(x, y)`: below one step of a 5-bit channel (8) for
/// red and blue, of a 6-bit one (4) for green; the panel truncates.
#[must_use]
pub const fn dither_at(x: usize, y: usize) -> u32 {
    let t = BAYER[y % 4][x % 4];
    (t >> 1) << 16 | (t >> 2) << 8 | (t >> 1)
}

/// FNV-1a over a word.
fn fnv(h: u32, w: u32) -> u32 {
    w.to_le_bytes()
        .iter()
        .fold(h, |h, &b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193))
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
    /// Runs just before the flip.
    pub flip_wait: Option<fn()>,
    /// Frames the engine didn't finish (each reset it and redraws whole).
    pub stalls: u32,
    /// Dither each redrawn rectangle (on; tests against the CPU renderer
    /// turn it off).
    pub dither: bool,
    /// The last frame's costs.
    pub last: FrameStats,
    delay_us: fn(u32),
    now: fn() -> u32,
    ticks_per_ms: u32,
    /// A frame queued, not yet shown: (its picture, its fence, queued at).
    pending: Option<(usize, u32, u32)>,
    /// A flip happened: the old picture may still be scanning out.
    vsync_due: bool,
    /// The background's hash (None: not built).
    bg_hash: Option<u32>,
    /// Per command: in the background (scratch for `draw`).
    hoist: [bool; MAX_CMDS],
    /// Per command: its bounds (scratch for `draw`).
    bounds: [Rect; MAX_CMDS],
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
            dither: true,
            last: FrameStats::default(),
            delay_us: |_| {},
            now,
            ticks_per_ms,
            pending: None,
            vsync_due: false,
            bg_hash: None,
            hoist: [false; MAX_CMDS],
            bounds: [Rect::new(0, 0, 0, 0); MAX_CMDS],
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
        self.pending = None;
        self.vsync_due = false;
        self.bg_hash = None;
        let mut buf = [0u32; CHUNK];
        for y in 0..4 {
            for x0 in (0..usize::from(W)).step_by(CHUNK) {
                for (i, p) in buf.iter_mut().enumerate() {
                    *p = dither_at(x0 + i, y);
                }
                self.mem.store(self.l.dither + y * STRIDE + 4 * x0, &buf);
            }
        }
        self.mem.sync();
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

    /// Microseconds since tick `t0`.
    fn us_since(&self, t0: u32) -> u32 {
        let t = u64::from((self.now)().wrapping_sub(t0));
        (t * 1000 / u64::from(self.ticks_per_ms.max(1))) as u32
    }

    /// Copies the wallpaper under `r` into picture `dst`.
    fn wallpaper(&mut self, dst: usize, r: Rect) {
        let src = self.l.wallpaper;
        self.copy(dst, src, r);
    }

    /// Copies `r` of the 32 bpp picture at `src` into the one at `dst`.
    fn copy(&mut self, dst: usize, src: usize, r: Rect) {
        let (w, n) = cmd::blit(
            (Format::BPP32, dst, STRIDE as i32),
            (Format::BPP32, src, STRIDE as i32),
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

    /// One row of colours, column `k` of `r` being `col(k)`, drawn down `r`
    /// (blended if `blend`).
    fn row(&mut self, dst: usize, r: Rect, col: impl Fn(u32) -> Argb, blend: bool) {
        if self.row == ROWS {
            self.wait();
            self.row = 0;
        }
        let at = self.l.rows + self.row * ROW_BYTES;
        self.row += 1;
        let w = usize::from(r.w).min(ROW_BYTES / 4);
        let mut buf = [0u32; CHUNK];
        for k0 in (0..w).step_by(CHUNK) {
            let n = CHUNK.min(w - k0);
            for (i, p) in buf[..n].iter_mut().enumerate() {
                *p = col((k0 + i) as u32);
            }
            self.mem.store(at + 4 * k0, &buf[..n]);
        }
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
            _ => self.row(dst, r, |_| c, true),
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

    /// Adds the dither pattern to `r` of picture `dst`: one blit per line
    /// phase (`y % 4`), every fourth line (destination stride × 4), the
    /// phase's pattern row read with stride 0; blend word 0 adds
    /// (saturating). The blend word goes back to "over" after.
    fn dither_rect(&mut self, dst: usize, r: Rect) {
        self.submit(&cmd::blend(0));
        for k in 0..4u16 {
            let first = r.y + (k + 4 - r.y % 4) % 4;
            if first >= r.bottom() {
                continue;
            }
            let lines = (r.bottom() - first).div_ceil(4);
            let (w, n) = cmd::blit(
                (
                    Format::BPP32,
                    dst + usize::from(first) * STRIDE,
                    (4 * STRIDE) as i32,
                ),
                (
                    Format::BPP32,
                    self.l.dither + usize::from(first % 4) * STRIDE,
                    0,
                ),
                (r.x, 0),
                (r.x, 0, r.right(), lines),
                ROP_SRCCOPY,
                OP_BLEND,
                0,
            );
            self.submit(&w[..n]);
        }
        self.submit(&cmd::blend(BLEND_DELL_TEXT));
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
        // Built in place: no copy of the table on the stack.
        self.atlases[slot] = Some(Atlas {
            font: key,
            color,
            at: [0; GLYPHS],
        });
        let mut buf = [0u32; CHUNK];
        for (k, g) in font.glyphs.iter().enumerate() {
            let (w, h) = (usize::from(g.w), usize::from(g.h));
            if w == 0 || h == 0 || g.cover.len() < w * h {
                continue;
            }
            let at = self.l.glyphs.0 + self.glyphs_top;
            for (i0, cover) in g.cover[..w * h].chunks(CHUNK).enumerate() {
                for (p, &c) in buf.iter_mut().zip(cover) {
                    *p = div255(a * u32::from(c)) << 24 | rgb;
                }
                self.mem.store(at + 4 * CHUNK * i0, &buf[..cover.len()]);
            }
            if let Some(atlas) = self.atlases[slot].as_mut() {
                atlas.at[k] = (self.glyphs_top / 16 + 1) as u16;
            }
            self.glyphs_top += (w * h * 4).next_multiple_of(16);
        }
        self.mem.sync();
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
                let first = u32::from(t.x - r.x);
                // Alpha runs linearly from one end's to the other's.
                let opaque = left >> 24 == 255 && right >> 24 == 255;
                if !opaque {
                    self.wallpaper(dst, t);
                }
                let span = u32::from(r.w);
                let col = |k| rp_gfx::render::gradient_at(left, right, first + k, span);
                self.row(dst, t, col, !opaque);
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
                    let unit = self.atlases[k].as_ref().map_or(0, |a| a.at[idx]);
                    if unit == 0 {
                        continue;
                    }
                    let at = self.l.glyphs.0 + (usize::from(unit) - 1) * 16;
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
    // Out of line: one copy, not one per call site (16 KB inlined into
    // rp-today's main loop, 2026-10-14).
    #[inline(never)]
    fn draw(&mut self, frame: &Frame) -> Drawn {
        let mut drawn = Drawn::default();
        // The previous frame shown, and scan-out off the picture we draw
        // into; usually long done by the time anything changes.
        let t0 = (self.now)();
        self.last = FrameStats::default();
        while !self.poll() {
            if self.us_since(t0) > 200_000 {
                self.stalls += 1;
                self.pending = None;
                self.fresh = 2;
                self.bg_hash = None;
                self.e.reset(self.delay_us);
                break;
            }
        }
        if self.vsync_due {
            let t = (self.now)();
            while self.e.regs.read32(rp_drv_mbx_display::reg::DINTRS)
                & rp_drv_mbx_display::event::VEVENT0
                == 0
                && self.us_since(t) < 40_000
            {}
            self.vsync_due = false;
        }
        self.last.wait_us = self.us_since(t0);
        let t0 = (self.now)();
        // Which commands go in the background, and has it changed?
        let cmds = frame.cmds().count().min(MAX_CMDS);
        let mut hash = 0x811C_9DC5u32;
        for (i, c) in frame.cmds().take(MAX_CMDS).enumerate() {
            let b = c.bounds(frame);
            self.bounds[i] = b;
            let glass = matches!(
                c,
                Cmd::Fill { .. } | Cmd::HGradient { .. } | Cmd::Outline { .. }
            );
            let covers_earlier =
                (0..i).any(|j| !self.hoist[j] && !self.bounds[j].intersect(&b).is_empty());
            self.hoist[i] = glass && !covers_earlier;
            if self.hoist[i] {
                let (kind, a, b2) = match *c {
                    Cmd::Fill { color, .. } => (1, color, 0),
                    Cmd::HGradient { left, right, .. } => (2, left, right),
                    Cmd::Outline { color, .. } => (3, color, 0),
                    _ => (0, 0, 0),
                };
                for w in [
                    kind,
                    u32::from(b.x) << 16 | u32::from(b.y),
                    u32::from(b.w) << 16 | u32::from(b.h),
                    a,
                    b2,
                ] {
                    hash = fnv(hash, w);
                }
            }
        }
        self.row = 0;
        let mut state = [0u32; 6];
        state[0] = cmd::SCALE_1_TO_1;
        state[1..4].copy_from_slice(&cmd::key(0, 0));
        state[4..].copy_from_slice(&cmd::blend(BLEND_DELL_TEXT));
        self.submit(&state);
        if self.bg_hash != Some(hash) {
            let bg = self.l.background;
            self.wallpaper(bg, SCREEN);
            for (i, c) in frame.cmds().take(cmds).enumerate() {
                if self.hoist[i] {
                    self.command(frame, c, SCREEN, bg);
                }
            }
            self.bg_hash = Some(hash);
            self.fresh = 2;
            self.last.background = true;
        }
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
        for &clip in &rects[..n] {
            let bg = self.l.background;
            self.copy(dst, bg, clip);
            for (i, c) in frame.cmds().take(cmds).enumerate() {
                if self.hoist[i] || self.bounds[i].intersect(&clip).is_empty() {
                    continue;
                }
                self.command(frame, c, clip, dst);
                drawn.cmds += 1;
            }
            if self.dither {
                self.dither_rect(dst, clip);
            }
            drawn.pixels += u64::from(clip.w) * u64::from(clip.h);
        }
        self.last.rects = n as u32;
        // The fence, and back to the caller: `poll` flips.
        let n = self.e.next_fence();
        let fence = cmd::fence(self.l.fence, n);
        self.submit(&fence);
        self.pending = Some((back, n, (self.now)()));
        self.last.queue_us = self.us_since(t0);
        drawn
    }

    #[inline(never)]
    fn poll(&mut self) -> bool {
        let Some((back, n, queued)) = self.pending else {
            return true;
        };
        if self.mem.load(self.l.fence) != Some(n) {
            return false;
        }
        self.pending = None;
        self.last.engine_us = self.us_since(queued);
        self.fresh = self.fresh.saturating_sub(1);
        if let Some(w) = self.flip_wait {
            w();
        }
        self.present(back);
        // From now the old picture scans out until the frame's end.
        rp_drv_mbx_display::ack_events(&mut self.e.regs, rp_drv_mbx_display::event::VEVENT0);
        self.vsync_due = true;
        true
    }
}

#[cfg(test)]
mod tests;
