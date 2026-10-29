//! The MBX 2D renderer (ADR-0023, option B): draws `rp-draw` frames on the
//! 2700G's 2D engine into one opaque picture, double-buffered. Since the
//! owner's 2026-10-21 votes (Q1 A, Q2 L1) the screens are dithered RGB565:
//! each changed rectangle is composed at 32 bits in a **work picture**,
//! the dither pattern added, then **one copy converts it into the screen**
//! (the engine truncates each channel on the way: `rp-mbx-blend` B7).
//!
//! For each changed rectangle the engine copies the wallpaper in (kept in
//! 2700G memory as RGB565, widened by the engine on the copy), then draws
//! every command that meets it, over it. Every blend is then onto an
//! opaque 32-bit pixel, which the engine does
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
//! - **Dithered.** The screen is RGB565, so each redrawn rectangle ends
//!   with a 4 x 4 ordered dither: the engine adds a threshold pattern to
//!   the work picture (blend word 0: source + destination, saturating,
//!   measured) and the conversion's truncation does the rest. The pattern
//!   is the 3D core's own matrix ([`DITHER_3D`]), so a 3D area meets the 2D
//!   screen without a seam. Four blits per rectangle, one per line phase
//!   (destination stride x 4, the pattern row read with stride 0).
//! - **16-bit where it's final.** The previous frame's rectangles come
//!   forward as 16-bit copies, and the area cache holds screen pixels.
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
/// Bytes per line of a 32 bpp picture (the work picture, the background).
pub const STRIDE: usize = W as usize * 4;
/// Bytes in one 32 bpp picture.
pub const PICTURE_BYTES: usize = STRIDE * H as usize;
/// Bytes per line of a screen (RGB565; also the wallpaper and the cache).
pub const STRIDE16: usize = W as usize * 2;
/// Bytes in one screen.
pub const SCREEN_BYTES: usize = STRIDE16 * H as usize;
const _: () = assert!(SCREEN_BYTES == rp_vram_map::SCREEN_BYTES as usize);
const _: () = assert!(PICTURE_BYTES == rp_vram_map::PICTURE32_BYTES as usize);
const _: () = assert!(ROWS * 2560 <= rp_vram_map::ROWS.size as usize);

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
/// Areas kept in the cache at once.
const CACHED: usize = 8;
/// Keys of areas drawn recently: an area is kept the second time it's
/// drawn, so a one-off doesn't cost a copy into the cache.
const SEEN: usize = 16;
/// Areas smaller than this (pixels) are drawn, not cached: the copy saves
/// little.
const CACHE_MIN_PX: u32 = 16 * 1024;

/// Where everything lives in 2700G local memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    /// The two screens (RGB565).
    pub screens: [usize; 2],
    /// The work picture (32 bpp): each changed rectangle is composed here,
    /// then converted into a screen.
    pub work: usize,
    /// The wallpaper, RGB565.
    pub wallpaper: usize,
    /// The row ring (`ROWS` × 2.5 KiB).
    pub rows: usize,
    /// The image cache and its size.
    pub images: (usize, usize),
    /// The glyph atlases and their size.
    pub glyphs: (usize, usize),
    /// The background: wallpaper and hoisted glass, 32 bpp.
    pub background: usize,
    /// The dither pattern's four rows (`W` pixels each, `STRIDE` apart).
    pub dither: usize,
    /// The fence word.
    pub fence: usize,
    /// Finished areas kept for reuse (an opened menu: one copy instead
    /// of drawing it again), RGB565, and its size.
    pub cache: (usize, usize),
}

impl Layout {
    /// The X51v's: from 2 MiB up (the loader's and Windows Mobile's planes
    /// sit below 1 MiB), about 5.6 MiB in all (ADR-0023 decision 4, Q2 L1).
    /// Taken from the 2700G memory map (`rp-vram-map`, ADR-0027), which
    /// checks the regions for overlaps.
    pub const X51V: Self = {
        use rp_vram_map as m;
        Self {
            screens: [m::SCREENS[0].at as usize, m::SCREENS[1].at as usize],
            work: m::WORK.at as usize,
            wallpaper: m::WALLPAPER.at as usize,
            rows: m::ROWS.at as usize,
            images: (m::IMAGES.at as usize, m::IMAGES.size as usize),
            glyphs: (m::GLYPHS.at as usize, m::GLYPHS.size as usize),
            background: m::BACKGROUND.at as usize,
            dither: m::DITHER.at as usize,
            fence: m::FENCE.at as usize,
            cache: (m::CACHE.at as usize, m::CACHE.size as usize),
        }
    };
}

/// 2700G local memory as the CPU sees it (one definition, in the memory
/// map's crate: the 3D renderer uses it too).
pub use rp_vram_map::LocalMem;

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
    /// Text atlases built (a font in a colour not cached).
    pub atlases: u32,
    /// Pictures uploaded to the image cache.
    pub images: u32,
    /// Times the frame waited for the engine mid-way (row buffers or a
    /// cache full).
    pub waits: u32,
    /// Command words queued.
    pub words: u32,
    /// Pixels the engine copies or fills, and blends (read, mix, write:
    /// the slower kind), by what it was drawing.
    pub px_copy: u32,
    /// See [`Self::px_copy`]: blended pixels outside text.
    pub px_blend: u32,
    /// See [`Self::px_copy`]: text pixels (glyph blends).
    pub px_text: u32,
    /// See [`Self::px_copy`]: dither pixels.
    pub px_dither: u32,
    /// Pixels converted from the work picture into the screen.
    pub px_convert: u32,
    /// Blits queued.
    pub blits: u32,
    /// Rectangles copied forward from the picture on screen.
    pub forward: u32,
    /// Rectangles copied from the cache instead of drawn.
    pub cache_hits: u32,
    /// Rectangles drawn and then kept in the cache.
    pub cache_stores: u32,
    /// Commands the 2D FIFO wouldn't take (full for 100 ms): the rest of
    /// the frame was dropped and it isn't shown ([`Mbx2dRenderer::fifo_full`]).
    pub fifo_full: u32,
}

/// The 3D core's dither matrix (`0x650` bit 3, `rp-gpubench` run P2 on
/// the X51v), its top left at the picture's first pixel: a channel goes
/// one step up where the fraction it loses reaches `t` sixteenths of a
/// step (16: never).
pub const DITHER_3D: [[u32; 4]; 4] = [
    [11, 3, 9, 1],
    [7, 15, 5, 13],
    [10, 2, 12, 4],
    [6, 14, 8, 16],
];

/// The dither added at `(x, y)` before the conversion to RGB565 truncates:
/// per channel `step - ceil(t * step / 16)` (step 8 for red and blue, 4
/// for green), so a channel goes one step up exactly where the 3D core's
/// dither raises it (the fraction lost `f` reaches `t * step / 16`); the
/// add saturates at 255, and the top stays the top, as on the core.
#[must_use]
pub const fn dither_at(x: usize, y: usize) -> u32 {
    let t = DITHER_3D[y % 4][x % 4];
    let rb = 8 - (t * 8).div_ceil(16);
    let g = 4 - (t * 4).div_ceil(16);
    rb << 16 | g << 8 | rb
}

/// A 32 bpp surface laid out as the screen (the work picture, the
/// background), for a blit.
const fn wide(at: usize) -> (Format, usize, i32) {
    (Format::BPP32, at, STRIDE as i32)
}

/// An RGB565 surface laid out as the screen (a screen, the wallpaper).
const fn narrow(at: usize) -> (Format, usize, i32) {
    (Format::RGB565, at, STRIDE16 as i32)
}

/// An XRGB8888 pixel as RGB565, each channel's top bits (what the engine
/// writes: `rp-mbx-blend` B7a).
#[must_use]
pub const fn to565(p: u32) -> u16 {
    ((p >> 8) & 0xF800 | (p >> 5) & 0x07E0 | (p >> 3) & 0x001F) as u16
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
    /// Flips written after waiting [`FLIP_WAIT_US`] for the blanking
    /// without seeing it (written anyway: a refresh may tear).
    pub late_flips: u32,
    /// Holds a finished frame's flip until [`Self::flip`]: the graphics
    /// service draws its 3D areas into the picture first (ADR-0030 G1).
    pub hold: bool,
    /// A finished frame held for [`Self::flip`]: its picture.
    landed: Option<usize>,
    /// The rectangles the last frame drew afresh (not copied forward).
    drawn: [Rect; MAX_DIRTY],
    drawn_n: usize,
    /// Frames the engine didn't finish (each reset it and redraws whole):
    /// a hang.
    pub stalls: u32,
    /// Frames dropped because the 2D FIFO stayed full for 100 ms while
    /// they were queued: counted apart from [`Self::stalls`], so a slow
    /// engine isn't read as a hung one. Such a frame isn't shown; the next
    /// two are drawn whole. Only if its fence never lands either is it
    /// also a stall.
    pub fifo_full: u32,
    /// 2D commands not sent because their rectangle isn't one the engine
    /// draws as asked (empty, inverted, past 13 bits: `cmd::drawable`,
    /// quality audit C47); each would otherwise have drawn nothing, or on
    /// the X51v far outside its picture.
    pub refused: u32,
    /// The frame being queued lost a command (the FIFO stayed full).
    dropping: bool,
    /// The frame queued lost a command: not shown when its fence lands.
    pending_dropped: bool,
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
    /// Bumped by [`Self::wallpaper_changed`]: part of the background's hash,
    /// so the background and every cached area made over the old wallpaper
    /// are made again.
    wall_gen: u32,
    /// Per command: in the background (scratch for `draw`).
    hoist: [bool; MAX_CMDS],
    /// Per command: its bounds (scratch for `draw`).
    bounds: [Rect; MAX_CMDS],
    /// The picture on screen is a whole, finished frame (it can be copied
    /// forward).
    front_whole: bool,
    /// The frame queued finished without a stall.
    pending_whole: bool,
    /// Cached areas: (key, rectangle, address).
    cached: [(u32, Rect, usize); CACHED],
    cached_n: usize,
    cache_top: usize,
    /// Keys of areas drawn lately (a ring).
    seen: [u32; SEEN],
    seen_at: usize,
}

/// The longest wait for the blanking before a flip, us: a refresh (16.6 ms
/// at 60 Hz, 17.7 at Windows Mobile's 56 Hz) and a little.
pub const FLIP_WAIT_US: u32 = 18_000;

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
            late_flips: 0,
            hold: false,
            landed: None,
            drawn: [Rect::new(0, 0, 0, 0); MAX_DIRTY],
            drawn_n: 0,
            stalls: 0,
            fifo_full: 0,
            refused: 0,
            dropping: false,
            pending_dropped: false,
            dither: true,
            last: FrameStats::default(),
            delay_us: |_| {},
            now,
            ticks_per_ms,
            pending: None,
            vsync_due: false,
            bg_hash: None,
            wall_gen: 0,
            front_whole: false,
            pending_whole: false,
            cached: [(0, Rect::new(0, 0, 0, 0), 0); CACHED],
            cached_n: 0,
            cache_top: 0,
            seen: [0; SEEN],
            seen_at: 0,
            hoist: [false; MAX_CMDS],
            bounds: [Rect::new(0, 0, 0, 0); MAX_CMDS],
        }
    }

    /// Stores line `y` of the wallpaper (`W` XRGB8888 pixels) as RGB565,
    /// each channel's top bits: pass a row already quantised to 16 bits
    /// (`rp-today`'s Floyd-Steinberg, the look the owner approved on
    /// 2026-10-17), or its gradients band.
    pub fn wallpaper_row(&mut self, y: u16, px: &[u32]) {
        let n = px.len().min(usize::from(W)) & !1;
        let mut buf = [0u32; CHUNK];
        for (k, pair) in px[..n].chunks(2 * CHUNK).enumerate() {
            let words = pair.len() / 2;
            for (i, w) in buf[..words].iter_mut().enumerate() {
                *w = u32::from(to565(pair[2 * i])) | u32::from(to565(pair[2 * i + 1])) << 16;
            }
            self.mem.store(
                self.l.wallpaper + usize::from(y) * STRIDE16 + 4 * CHUNK * k,
                &buf[..words],
            );
        }
    }

    /// A finished frame held for [`Self::flip`] ([`Self::hold`]): the
    /// address of its picture.
    #[must_use]
    pub fn landed(&self) -> Option<usize> {
        self.landed.map(|k| self.l.screens[k])
    }

    /// The wallpaper was rewritten ([`Self::wallpaper_row`], with no frame
    /// in flight): the background and the area cache are made again, and the
    /// whole screen is drawn afresh.
    pub fn wallpaper_changed(&mut self) {
        self.wall_gen = self.wall_gen.wrapping_add(1);
        self.bg_hash = None;
        self.fresh = 2;
    }

    /// Draws the whole screen afresh on the next two frames (an area that
    /// was another engine's comes back).
    pub fn invalidate_all(&mut self) {
        self.fresh = 2;
    }

    /// Shows the frame held by [`Self::hold`].
    pub fn flip(&mut self) {
        if let Some(k) = self.landed.take() {
            self.show_picture(k);
        }
    }

    /// With no frame queued or held, the picture not on screen: the two
    /// pictures then hold the same frame (the last two frames are drawn
    /// into both), so drawing into it (3D) and [`Self::flip_back`]ing is
    /// a whole frame. Waits for scan-out to leave it first.
    pub fn back(&mut self) -> Option<usize> {
        if self.pending.is_some() || self.landed.is_some() || self.prev_n != 0 {
            return None;
        }
        self.wait_scanout();
        Some(self.l.screens[1 - self.front])
    }

    /// Shows the picture [`Self::back`] gave.
    pub fn flip_back(&mut self) {
        if self.pending.is_none() && self.landed.is_none() {
            self.show_picture(1 - self.front);
        }
    }

    /// The two screens' addresses (screen 0 inside the safe region,
    /// `rp-vram-map`).
    #[must_use]
    pub const fn screens(&self) -> [usize; 2] {
        self.l.screens
    }

    /// The address of the picture on screen.
    #[must_use]
    pub fn front(&self) -> usize {
        self.l.screens[self.front]
    }

    /// Drops the frame held by [`Self::hold`] without showing it: the caller
    /// copied its changes into the picture on screen ([`Self::copy_screens`];
    /// C71 ruling R-B, the graphics service's 3D frames). Both pictures then
    /// hold the frame, so the next one is drawn into the same back picture.
    pub fn keep_front(&mut self) {
        self.landed = None;
    }

    /// Waits (40 ms at most) for scan-out to leave the picture it showed
    /// before the last flip: from then on only the picture on screen is
    /// read (C71: the 3D core starts only while the display reads inside
    /// the safe region).
    pub fn settle(&mut self) {
        self.wait_scanout();
    }

    /// [`Self::settle`] without waiting: whether scan-out has left the
    /// picture it showed before the last flip (R20's option 1 copies into
    /// that picture only then, the graphics service's frame loop never
    /// blocked).
    pub fn settled(&mut self) -> bool {
        if self.vsync_due
            && self.e.regs.read32(rp_drv_mbx_display::reg::DINTRS)
                & rp_drv_mbx_display::event::VEVENT0
                == 0
        {
            return false;
        }
        self.vsync_due = false;
        true
    }

    /// Copies `rects` of the screen at `src` into the one at `dst` on the 2D
    /// engine, and waits for them (C71 ruling R-B: 2D's changes into screen
    /// 0 before a 3D render, the 3D area back into screen 1 after it).
    /// `false`: the engine stalled (reset on the next frame, which is then
    /// drawn whole).
    pub fn copy_screens(&mut self, dst: usize, src: usize, rects: &[Rect]) -> bool {
        let mut ok = true;
        for &r in rects {
            self.last.px_copy += u32::from(r.w) * u32::from(r.h);
            self.last.blits += 1;
            match cmd::blit(
                narrow(dst),
                narrow(src),
                (r.x, r.y),
                Self::coords(r),
                ROP_SRCCOPY,
                0,
                0,
            ) {
                Some((w, n)) => ok &= self.e.submit(&w[..n]).is_ok(),
                None => self.refused += 1,
            }
        }
        let mem = &self.mem;
        ok &= self.e.finish_with(200_000, |at| mem.load(at)).is_ok();
        if !ok {
            self.stalls += 1;
            self.fresh = 2;
        }
        ok
    }

    /// The rectangles the last frame drew afresh (the whole screen for a
    /// whole frame), not those copied forward.
    #[must_use]
    pub fn drawn(&self) -> &[Rect] {
        &self.drawn[..self.drawn_n]
    }

    /// Waits (40 ms at most) for scan-out to leave the picture not on
    /// screen, after a flip.
    fn wait_scanout(&mut self) {
        if self.vsync_due {
            let t = (self.now)();
            while self.e.regs.read32(rp_drv_mbx_display::reg::DINTRS)
                & rp_drv_mbx_display::event::VEVENT0
                == 0
                && self.us_since(t) < 40_000
            {}
            self.vsync_due = false;
        }
    }

    fn show_picture(&mut self, k: usize) {
        self.present(k);
        // From now the old picture scans out until the frame's end.
        rp_drv_mbx_display::ack_events(&mut self.e.regs, rp_drv_mbx_display::event::VEVENT0);
        self.vsync_due = true;
    }

    /// The register window and memory (the image's own use).
    pub fn parts(&mut self) -> (&mut Engine<R>, &mut M) {
        (&mut self.e, &mut self.mem)
    }

    /// Resets the engine and shows screen 0 as an opaque RGB565 graphics
    /// plane (format, size, stride and start). The video plane and cursor
    /// are the caller's (`Display::adopt` switches them off).
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
            format: rp_drv_mbx_display::PixelFormat::Rgb565,
            width: W,
            height: H,
            offset: self.l.screens[0],
            stride: STRIDE16,
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
        // A hidden video plane out of the solo plane's slot (GC2DC run 2).
        rp_drv_mbx_display::layers::park_hidden_video(&mut self.e.regs);
    }

    fn solo(scadr: u32) -> u32 {
        rp_drv_mbx_display::layers::Layer::SOLO.scadr(scadr)
    }

    /// Points the plane at picture `k`, in the vertical blanking (R113 on
    /// the X51v: a `GSCADR` write shows at once, so one written mid-refresh
    /// tore it, every time): it shows from the next refresh, whole. The
    /// wait is at most a refresh (then late, counted, written anyway).
    fn present(&mut self, k: usize) {
        let t = (self.now)();
        while !rp_drv_mbx_display::in_blanking(&mut self.e.regs) {
            if self.us_since(t) > FLIP_WAIT_US {
                self.late_flips += 1;
                break;
            }
        }
        let scadr = (1 << 31) | (self.l.screens[k] / 16) as u32;
        self.e
            .regs
            .write32(rp_drv_mbx_display::reg::GSCADR, Self::solo(scadr));
        self.front = k;
    }

    fn submit(&mut self, w: &[u32]) {
        if self.dropping {
            return; // the frame is lost already: no more 100 ms waits
        }
        self.last.words += w.len() as u32;
        if self.e.submit(w).is_err() {
            self.drop_frame();
        }
    }

    /// The FIFO stayed full: the frame being queued lost a command. Nothing
    /// more of it is queued, and it won't be shown (`draw` then forgets
    /// what it may have left half-made).
    fn drop_frame(&mut self) {
        self.dropping = true;
        self.fifo_full += 1;
        self.last.fifo_full += 1;
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

    /// Copies the wallpaper (RGB565) under `r` into the 32 bpp picture
    /// `dst` (the engine widens it).
    fn wallpaper(&mut self, dst: usize, r: Rect) {
        let src = self.l.wallpaper;
        self.copy_as(wide(dst), narrow(src), r);
    }

    /// Copies `r` of the 32 bpp picture at `src` into the one at `dst`.
    fn copy(&mut self, dst: usize, src: usize, r: Rect) {
        self.copy_as(wide(dst), wide(src), r);
    }

    /// Copies `r` from one surface to another, both laid out as the screen
    /// (the engine converts between their formats).
    fn copy_as(&mut self, dst: (Format, usize, i32), src: (Format, usize, i32), r: Rect) {
        self.last.px_copy += u32::from(r.w) * u32::from(r.h);
        self.last.blits += 1;
        if let Some((w, n)) = cmd::blit(dst, src, (r.x, r.y), Self::coords(r), ROP_SRCCOPY, 0, 0) {
            self.submit(&w[..n]);
        } else {
            self.refused += 1;
        }
    }

    /// Converts `r` of the work picture into screen `dst` (each channel
    /// truncated to 5, 6, 5 bits: `rp-mbx-blend` B7a).
    fn convert(&mut self, dst: usize, r: Rect) {
        self.last.px_convert += u32::from(r.w) * u32::from(r.h);
        self.last.blits += 1;
        let work = self.l.work;
        if let Some((w, n)) = cmd::blit(
            narrow(dst),
            wide(work),
            (r.x, r.y),
            Self::coords(r),
            ROP_SRCCOPY,
            0,
            0,
        ) {
            self.submit(&w[..n]);
        } else {
            self.refused += 1;
        }
    }

    /// `r` set to the opaque colour `c`.
    fn fill(&mut self, dst: usize, r: Rect, c: Argb) {
        self.last.px_copy += u32::from(r.w) * u32::from(r.h);
        self.last.blits += 1;
        if let Some(w) = cmd::fill(Format::BPP32, dst, STRIDE, Self::coords(r), c | 0xFF00_0000) {
            self.submit(&w);
        } else {
            self.refused += 1;
        }
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
        let a = u32::from(r.w) * u32::from(r.h);
        if blend {
            self.last.px_blend += a;
        } else {
            self.last.px_copy += a;
        }
        self.last.blits += 1;
        if let Some((w, n)) = cmd::blit(
            (Format::BPP32, dst, STRIDE as i32),
            (Format::BPP32, at, 0),
            (0, 0),
            Self::coords(r),
            ROP_SRCCOPY,
            if blend { OP_BLEND } else { 0 },
            0,
        ) {
            self.submit(&w[..n]);
        } else {
            self.refused += 1;
        }
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
        self.last.px_blend += u32::from(to.w) * u32::from(to.h);
        self.last.blits += 1;
        if let Some((w, n)) = cmd::blit(
            (Format::BPP32, dst, STRIDE as i32),
            (Format::BPP32, src, stride as i32),
            (sx, sy),
            Self::coords(to),
            ROP_SRCCOPY,
            OP_BLEND,
            0,
        ) {
            self.submit(&w[..n]);
        } else {
            self.refused += 1;
        }
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
            self.last.px_dither += u32::from(r.w) * u32::from(lines);
            self.last.blits += 1;
            if let Some((w, n)) = cmd::blit(
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
            ) {
                self.submit(&w[..n]);
            } else {
                self.refused += 1;
            }
        }
        self.submit(&cmd::blend(BLEND_DELL_TEXT));
    }

    /// Waits for everything queued (to reuse rows or cache space).
    fn wait(&mut self) -> bool {
        if self.dropping {
            return true; // nothing more of this frame is queued
        }
        self.last.waits += 1;
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
        self.last.images += 1;
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
        self.last.atlases += 1;
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

    /// What decides how `clip` looks: the background, the commands that
    /// meet it (their text included) and whether it's dithered.
    fn area_key(&self, frame: &Frame, cmds: usize, clip: Rect) -> u32 {
        let rect = |r: Rect| {
            [
                u32::from(r.x) << 16 | u32::from(r.y),
                u32::from(r.w) << 16 | u32::from(r.h),
            ]
        };
        let mut h = fnv(0x811C_9DC5, self.bg_hash.unwrap_or(0));
        for w in rect(clip).into_iter().chain([u32::from(self.dither)]) {
            h = fnv(h, w);
        }
        for (i, c) in frame.cmds().take(cmds).enumerate() {
            if self.hoist[i] || self.bounds[i].intersect(&clip).is_empty() {
                continue;
            }
            let words: [u32; 5] = match *c {
                Cmd::Fill { r, color } => {
                    let [a, b] = rect(r);
                    [1, a, b, color, 0]
                }
                Cmd::HGradient { r, left, right } => {
                    let [a, b] = rect(r);
                    [2, a, b, left, right]
                }
                Cmd::Outline { r, color } => {
                    let [a, b] = rect(r);
                    [3, a, b, color, 0]
                }
                Cmd::Image { x, y, img } => [
                    4,
                    u32::from(x) << 16 | u32::from(y),
                    core::ptr::from_ref(img) as usize as u32,
                    0,
                    0,
                ],
                Cmd::Text {
                    x,
                    baseline,
                    font,
                    color,
                    text,
                } => {
                    for &b in frame.text_of(text) {
                        h = fnv(h, u32::from(b));
                    }
                    [
                        5,
                        u32::from(x) << 16 | u32::from(baseline),
                        core::ptr::from_ref(font) as usize as u32,
                        color,
                        0,
                    ]
                }
            };
            for w in words {
                h = fnv(h, w);
            }
        }
        h
    }

    /// A cached area's line (RGB565): its width, in whole 16-byte units
    /// (the engine's line pitch is kept aligned, as every surface's is so
    /// far).
    const fn cache_stride(clip: Rect) -> usize {
        (clip.w as usize * 2).next_multiple_of(16)
    }

    /// Copies the cached area at `at` (`clip`'s size, packed) into `clip`
    /// of screen `dst`.
    fn copy_in(&mut self, dst: usize, clip: Rect, at: usize) {
        self.last.px_copy += u32::from(clip.w) * u32::from(clip.h);
        self.last.blits += 1;
        if let Some((w, n)) = cmd::blit(
            narrow(dst),
            (Format::RGB565, at, Self::cache_stride(clip) as i32),
            (0, 0),
            Self::coords(clip),
            ROP_SRCCOPY,
            0,
            0,
        ) {
            self.submit(&w[..n]);
        } else {
            self.refused += 1;
        }
    }

    /// Keeps `clip` of screen `dst`, just drawn, under `key`: the second
    /// time the key is seen (a one-off isn't worth the copy). The engine
    /// works in order, so the copy reads the finished area, and a cached
    /// area overwritten later was read before.
    fn keep(&mut self, dst: usize, clip: Rect, key: u32) {
        if !self.seen.contains(&key) {
            self.seen[self.seen_at] = key;
            self.seen_at = (self.seen_at + 1) % SEEN;
            return;
        }
        let bytes = Self::cache_stride(clip) * usize::from(clip.h);
        if bytes > self.l.cache.1 {
            return;
        }
        if self.cached_n == CACHED || self.cache_top + bytes > self.l.cache.1 {
            self.cached_n = 0;
            self.cache_top = 0;
        }
        let at = self.l.cache.0 + self.cache_top;
        self.cache_top += bytes;
        self.cached[self.cached_n] = (key, clip, at);
        self.cached_n += 1;
        self.last.cache_stores += 1;
        self.last.px_copy += u32::from(clip.w) * u32::from(clip.h);
        self.last.blits += 1;
        if let Some((w, n)) = cmd::blit(
            (Format::RGB565, at, Self::cache_stride(clip) as i32),
            narrow(dst),
            (clip.x, clip.y),
            (0, 0, clip.w, clip.h),
            ROP_SRCCOPY,
            0,
            0,
        ) {
            self.submit(&w[..n]);
        } else {
            self.refused += 1;
        }
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
                        let a = u32::from(to.w) * u32::from(to.h);
                        self.last.px_text += a;
                        self.last.px_blend -= a;
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
        self.dropping = false;
        // A held frame nobody flipped: show it now.
        self.flip();
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
        self.wait_scanout();
        self.last.wait_us = self.us_since(t0);
        let t0 = (self.now)();
        let stalls0 = self.stalls;
        // Which commands go in the background, and has it changed?
        let cmds = frame.cmds().count().min(MAX_CMDS);
        let mut hash = fnv(0x811C_9DC5u32, self.wall_gen);
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
        // What this frame changed is drawn; what the previous frame changed
        // (the picture drawn into is two frames old) is copied forward from
        // the picture on screen, one plain copy instead of drawing it again
        // (2026-10-14: the frame after the Start menu opened drew the whole
        // menu a second time, glass, text and dither: ~30 ms of engine).
        let mut rects = [Rect::default(); MAX_DIRTY];
        let mut n = 0;
        let mut forward = [Rect::default(); MAX_DIRTY];
        let mut nf = 0;
        if self.fresh > 1 || (self.fresh == 1 && !self.front_whole) {
            rects[0] = SCREEN;
            n = 1;
        } else {
            // The frame after a whole one (start-up, a new background): the
            // other picture is two frames old, so all of it comes forward
            // (2026-10-14: the first tap after start-up drew the whole
            // screen again, 14.7 ms of CPU and 23.7 of engine).
            if self.fresh == 1 {
                forward[0] = SCREEN;
                nf = 1;
            }
            for r in frame.dirty() {
                let r = r.intersect(&SCREEN);
                if !r.is_empty() && !rects[..n].iter().any(|d| d.intersect(&r) == r) {
                    rects[n] = r;
                    n += 1;
                }
            }
            // (Nothing more to bring forward when it's all coming.)
            let prev = if nf == 0 { self.prev_n } else { 0 };
            for r in &self.prev[..prev] {
                let r = r.intersect(&SCREEN);
                if !r.is_empty() && !rects[..n].iter().any(|d| d.intersect(&r) == r) {
                    forward[nf] = r;
                    nf += 1;
                }
            }
        }
        self.prev_n = frame.dirty().len().min(MAX_DIRTY);
        self.prev[..self.prev_n].copy_from_slice(&frame.dirty()[..self.prev_n]);
        self.drawn_n = n;
        self.drawn[..n].copy_from_slice(&rects[..n]);
        if n == 0 && nf == 0 {
            return drawn;
        }
        let back = 1 - self.front;
        let dst = self.l.screens[back];
        let shown = self.l.screens[self.front];
        let work = self.l.work;
        // First: drawing over these afterwards is still right.
        for &r in &forward[..nf] {
            self.copy_as(narrow(dst), narrow(shown), r);
            drawn.pixels += u64::from(r.w) * u64::from(r.h);
        }
        for &clip in &rects[..n] {
            drawn.pixels += u64::from(clip.w) * u64::from(clip.h);
            // An area drawn before, unchanged (the same background and
            // commands): one copy from the cache.
            let key = (u32::from(clip.w) * u32::from(clip.h) >= CACHE_MIN_PX)
                .then(|| self.area_key(frame, cmds, clip));
            if let Some(key) = key
                && let Some(&(_, _, at)) = self.cached[..self.cached_n]
                    .iter()
                    .find(|(k, r, _)| *k == key && *r == clip)
            {
                self.copy_in(dst, clip, at);
                self.last.cache_hits += 1;
                continue;
            }
            // Composed at 32 bits in the work picture, dithered there, then
            // converted into the screen (ADR-0023 Q1 A).
            let bg = self.l.background;
            self.copy(work, bg, clip);
            for (i, c) in frame.cmds().take(cmds).enumerate() {
                if self.hoist[i] || self.bounds[i].intersect(&clip).is_empty() {
                    continue;
                }
                self.command(frame, c, clip, work);
                drawn.cmds += 1;
            }
            if self.dither {
                self.dither_rect(work, clip);
            }
            self.convert(dst, clip);
            if let Some(key) = key {
                self.keep(dst, clip, key);
            }
        }
        self.last.rects = n as u32;
        self.last.forward = nf as u32;
        // The fence, and back to the caller: `poll` flips.
        // Queued even for a dropped frame: when it lands, the engine has
        // caught up (no reset needed); if it never does, `draw`'s wait
        // counts a stall and resets.
        let n = self.e.next_fence();
        let fence = cmd::fence(self.l.fence, n);
        self.last.words += fence.len() as u32;
        if self.e.submit(&fence).is_err() && !self.dropping {
            self.drop_frame();
        }
        if self.dropping {
            // Its partial work may be in the background or a cached area
            // (marked made as the frame went on), and in the picture: all
            // made again, the next two frames drawn whole.
            self.bg_hash = None;
            self.cached_n = 0;
            self.fresh = 2;
        }
        self.pending = Some((back, n, (self.now)()));
        self.pending_whole = self.stalls == stalls0 && !self.dropping;
        self.pending_dropped = self.dropping;
        self.last.queue_us = self.us_since(t0);
        drawn
    }

    #[inline(never)]
    fn poll(&mut self) -> bool {
        if self.landed.is_some() {
            return false;
        }
        let Some((back, n, queued)) = self.pending else {
            return true;
        };
        if self.mem.load(self.l.fence) != Some(n) {
            return false;
        }
        self.pending = None;
        self.last.engine_us = self.us_since(queued);
        if self.pending_dropped {
            // Missing commands: never shown. The picture on screen stays,
            // and the next two frames are drawn whole (`drop_frame`).
            self.pending_dropped = false;
            return true;
        }
        self.fresh = self.fresh.saturating_sub(1);
        self.front_whole = self.pending_whole;
        if self.hold {
            self.landed = Some(back);
            return false;
        }
        self.show_picture(back);
        true
    }
}

#[cfg(test)]
mod tests;
