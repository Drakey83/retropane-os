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
// 16: Today with the living sky and its card's soft shadows draws text in
// 10 font-and-colour pairs (ADR-0023 rule 3); with 8 slots every frame
// reset the cache and uploaded the fonts again, a stutter (the run of
// 2026-11-01 logged 7 atlases uploaded in its slowest frames). 32 since
// R172's return leg (2026-11-09): Today with the sky and Weather in both
// looks draw 25 pairs between them, so 16 slots evicted Today's on the way
// back however small the glyphs. The memory they share (rule 4, 384 KiB
// since 2026-11-10) is apart; a slot is ~370 bytes of RAM.
const ATLASES: usize = 32;
/// The glyph memory is handed out in chunks of this many bytes: an atlas
/// takes chunks as its glyphs are first drawn (small glyphs packed into its
/// open chunk, a big one in a run of whole chunks), so a pair holds only the
/// glyphs it has drawn (R172: whole fonts per colour took 443 KiB for
/// Today's ten pairs and 854 KiB for Weather's 23, of 512; the glyphs they
/// draw, 279 KiB for all 25).
const GCHUNK: usize = 2048;
/// Chunks the glyph memory may hold (384 KiB, the owner's ruling of
/// 2026-11-10: 279 KiB of glyphs for every pair on the look route, at most
/// 64 KiB more part-filled).
const GCHUNKS: usize = 192;
/// Glyphs a font may have.
// 130: the largest shipped font has 129 (BIG; UI and LABEL 97), and every
// slot's table is this long (32 slots: the RAM is the slots' tables).
const GLYPHS: usize = 130;
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
    /// A screen kept for a return (RGB565, [`Mbx2dRenderer::keep_shown`]).
    pub kept: usize,
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
            kept: m::KEPT.at as usize,
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
    /// The chunk its next small glyph goes into, and the bytes already
    /// used there (`fill` [`GCHUNK`]: none open).
    open: u16,
    fill: u16,
    /// The frame that last drew with it (eviction takes the oldest).
    used: u32,
    at: [u16; GLYPHS],
}

impl Atlas {
    /// A pair with nothing uploaded yet.
    const fn new(font: usize, color: Argb, used: u32) -> Self {
        Self {
            font,
            color,
            open: 0,
            fill: GCHUNK as u16,
            used,
            at: [0; GLYPHS],
        }
    }
}

/// The first run of `k` free chunks (owner 0) below `n`.
fn free_run(owner: &[u8; GCHUNKS], n: usize, k: usize) -> Option<usize> {
    let mut run = 0;
    for (i, &o) in owner[..n].iter().enumerate() {
        if o == 0 {
            run += 1;
            if run == k {
                return Some(i + 1 - k);
            }
        } else {
            run = 0;
        }
    }
    None
}

/// What a row of the ring holds, exactly: `w` columns of `gradient_at(a,
/// b, first + k, span)` (a single colour is `a == b`, `first` 0, `span`
/// 1). Compared whole, never by a hash, so a row is reused only for the
/// same colours.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct RowKey {
    a: Argb,
    b: Argb,
    first: u32,
    span: u32,
    w: u16,
}

/// The most pieces an area splits into around a panel's body (a rect
/// over a panel: its four corners and up to four sides outside it).
const UNDER: usize = 8;

/// Disjoint pieces of an area ([`under_panel`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Pieces {
    r: [Rect; UNDER],
    n: usize,
}

impl Pieces {
    /// These pieces less `hole`, each split into at most four around it:
    /// `None` if they don't fit.
    #[inline(never)]
    fn minus(&self, hole: Rect) -> Option<Self> {
        let mut out = Self::default();
        let mut push = |r: Rect| {
            if r.is_empty() {
                return Some(());
            }
            *out.r.get_mut(out.n)? = r;
            out.n += 1;
            Some(())
        };
        for q in &self.r[..self.n] {
            let t = q.intersect(&hole);
            if t.is_empty() {
                push(*q)?;
                continue;
            }
            push(Rect::new(q.x, q.y, q.w, t.y - q.y))?;
            push(Rect::new(q.x, t.bottom(), q.w, q.bottom() - t.bottom()))?;
            push(Rect::new(q.x, t.y, t.x - q.x, t.h))?;
            push(Rect::new(t.right(), t.y, q.right() - t.right(), t.h))?;
        }
        Some(out)
    }

    fn all(&self) -> &[Rect] {
        &self.r[..self.n]
    }
}

/// The panel that hides the most of `clip`, and the pieces of `clip`
/// where what was drawn before it still shows: its four corners and
/// whatever of `clip` lies outside it (at most [`UNDER`]).
///
/// A panel's body starts from its clears, the wallpaper itself
/// (`rp_draw::panel_cmds`, ADR-0023 rule 3), so every command before it
/// is invisible there: the Start menu over Today drew Today's 85
/// commands under the menu's glass for nothing, 610 blits and 38.6 ms
/// (run today-0140, frame 191). Commands before the panel are drawn in
/// the pieces only, the background copied only there; the pixels are
/// the same. `None`: no panel clears any of `clip`, or the pieces
/// don't fit (drawn as before).
#[inline(never)]
fn under_panel(frame: &Frame, cmds: usize, clip: Rect) -> Option<(usize, Pieces)> {
    let area = |r: &Rect| u32::from(r.w) * u32::from(r.h);
    let mut best: Option<(usize, u32, [Rect; 3])> = None;
    for (i, (c, own)) in frame.cmds_clipped().take(cmds).enumerate() {
        let Cmd::Panel { r, art } = *c else {
            continue;
        };
        let k = art.radius;
        if r.w < 2 * k + 2 || r.h < 2 * k + 2 {
            continue;
        }
        // `panel_cmds`' clears, in the panel's own clip.
        let clears = [
            Rect::new(r.x + k, r.y, r.w - 2 * k, k),
            Rect::new(r.x, r.y + k, r.w, r.h - 2 * k),
            Rect::new(r.x + k, r.y + r.h - k, r.w - 2 * k, k),
        ]
        .map(|q| q.intersect(&own).intersect(&clip));
        // (The three don't overlap.)
        let a = clears.iter().map(area).sum::<u32>();
        if a > 0 && best.is_none_or(|(_, b, _)| a >= b) {
            best = Some((i, a, clears));
        }
    }
    let (p, _, clears) = best?;
    let mut pieces = Pieces::default();
    pieces.r[0] = clip;
    pieces.n = 1;
    for hole in clears {
        pieces = pieces.minus(hole)?;
    }
    Some((p, pieces))
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
    /// Whether the background was rebuilt (whole or in part).
    pub background: bool,
    /// Pixels of the background rebuilt (the whole screen: 307,200).
    pub background_px: u32,
    /// Gradient and translucent rows written by the CPU, and reused from
    /// the ring instead.
    pub rows_written: u32,
    /// See [`Self::rows_written`].
    pub rows_reused: u32,
    /// Text atlases built (a font in a colour not cached).
    pub atlases: u32,
    /// Atlases evicted to make room (the oldest drawn with first).
    pub atlases_evicted: u32,
    /// Glyphs uploaded (each pair's glyphs as they are first drawn).
    pub glyphs: u32,
    /// Pictures uploaded to the image cache.
    pub images: u32,
    /// Pictures dropped from the image cache to make room (the oldest
    /// drawn first).
    pub images_evicted: u32,
    /// Bytes of pictures uploaded.
    pub image_bytes: u32,
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
    /// Pictures not drawn (`Cmd::Pic` whose pixels couldn't be had or
    /// placed), and the last of them (R181: never silent).
    pub pics_missed: rp_draw::pics::Misses,
    /// The screen shown copied into the kept picture with this frame
    /// ([`Mbx2dRenderer::keep_shown`]).
    pub kept: bool,
    /// The frame started from the kept picture, drawing only its changes
    /// ([`Mbx2dRenderer::seed`]).
    pub seeded: bool,
    /// Commands not drawn at all: wholly under a later panel's body in
    /// every area drawn (a command reaching its corners or past it is
    /// drawn there only; R191, ADR-0023 rule 3).
    pub hidden: u32,
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
    /// Uploaded pictures: (key, offset in the cache, bytes, frame last
    /// drawn in).
    images: [Option<(usize, usize, usize, u32)>; IMAGES],
    /// The RAM store `Cmd::Pic` names pictures in (none: they draw
    /// nothing).
    pics: Option<&'static rp_draw::pics::Pics<'static>>,
    atlases: [Option<Atlas>; ATLASES],
    /// Each glyph-memory chunk's pair: its slot + 1, 0 free.
    chunk_owner: [u8; GCHUNKS],
    /// Frames drawn (stamps the atlases' last use).
    frame_no: u32,
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
    /// The hoisted commands the background holds: each one's key (its
    /// kind, place, colours, clip, and the keys of the hoisted commands
    /// before it that it overlaps, so a change of order counts) and
    /// bounds. A frame whose glass differs rebuilds only where the
    /// differing ones were and are (2026-10-30: a sliding row rebuilt the
    /// whole screen's background, 53-58 ms of CPU a frame on the X51v).
    bg_keys: [(u32, Rect); MAX_CMDS],
    bg_n: usize,
    /// Per command: its key if hoisted (scratch for `draw`).
    keys: [u32; MAX_CMDS],
    /// What each row of the ring holds, kept across frames so a row of
    /// the same colours is written once ([`RowKey`]), and the use stamp
    /// that says whether the engine may still read it.
    row_keys: [Option<RowKey>; ROWS],
    row_used: [u32; ROWS],
    /// Bumped at each frame's start and each full wait: a row stamped
    /// with an older value is no longer read by the engine.
    epoch: u32,
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
    /// The wallpaper's rows read band by band ([`Self::wallpaper_bands`]):
    /// the first band's row, the bands' height, and per band the base its
    /// rows are read from (laid out as the screen; `None`: the wallpaper).
    bands: (u16, u16, [Option<usize>; BANDS]),
    /// Where the wallpaper changed since the last frame (a band switch):
    /// its background made again and its screen area drawn there only.
    wall_box: ([Rect; WALL_BOXES], usize),
    /// The kept picture ([`Layout::kept`]): the wallpaper generation the
    /// screen copied into it was drawn over, and whether the frame that
    /// copied it landed whole (`None`: nothing kept, or not usable).
    kept: Option<(u32, Option<u32>, bool)>,
    /// The wallpaper written since the last change, named by the caller
    /// ([`Self::wallpaper_is`]; `None`: unnamed).
    wall_id: Option<u32>,
    /// The screen shown is copied into the kept picture by the next frame.
    keep_next: bool,
    /// The frame in flight copies it (confirmed when it lands).
    keep_landing: bool,
    /// The next frame starts from the kept picture.
    seed_next: bool,
}

/// Areas a band switch redraws at most (more are merged).
pub const WALL_BOXES: usize = 4;

/// Bands of the wallpaper that can be read from elsewhere
/// ([`Mbx2dRenderer::wallpaper_bands`]).
pub const BANDS: usize = 16;

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
        renderer_start!(regs, mem, l, now, ticks_per_ms; value)
    }

    /// [`Self::new`] made straight into `slot`, a field at a time: made on
    /// the stack and moved, the renderer took a 4.7 KB frame at the
    /// image's start (`cargo xtask stacks`, P6).
    pub fn new_in(
        slot: &mut core::mem::MaybeUninit<Self>,
        regs: R,
        mem: M,
        l: Layout,
        now: fn() -> u32,
        ticks_per_ms: u32,
    ) -> &mut Self {
        renderer_start!(regs, mem, l, now, ticks_per_ms; slot,)
    }
}

/// Every field's start, made as a value or straight into a slot
/// (`rp_hal_mem::in_place!`): one list for `new` and `new_in`.
macro_rules! renderer_start {
    ($regs:expr, $mem:expr, $l:expr, $now:expr, $tpm:expr; $($how:tt)*) => {
        rp_hal_mem::in_place!($($how)* Mbx2dRenderer {
            e: Engine::new($regs, $l.fence, $now, $tpm),
            mem: $mem,
            l: $l,
            front: 0,
            prev: [Rect::default(); MAX_DIRTY],
            prev_n: 0,
            fresh: 2,
            row: 0,
            images: [None; IMAGES],
            pics: None,
            atlases: [None; ATLASES],
            chunk_owner: [0; GCHUNKS],
            frame_no: 0,
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
            now: $now,
            ticks_per_ms: $tpm,
            pending: None,
            vsync_due: false,
            bg_hash: None,
            bg_keys: [(0, Rect::new(0, 0, 0, 0)); MAX_CMDS],
            bg_n: 0,
            keys: [0; MAX_CMDS],
            row_keys: [None; ROWS],
            row_used: [0; ROWS],
            epoch: 1,
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
            bands: (0, 1, [None; BANDS]),
            wall_box: ([Rect::new(0, 0, 0, 0); WALL_BOXES], 0),
            kept: None,
            wall_id: None,
            keep_next: false,
            keep_landing: false,
            seed_next: false,
        })
    };
}
use renderer_start;

impl<R: RegisterBus, M: LocalMem> Mbx2dRenderer<R, M> {
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
        self.wall_id = None;
        self.bg_hash = None;
        self.row_keys = [None; ROWS];
        self.fresh = 2;
    }

    /// Reads the wallpaper's rows from `top` on, `h` at a time, band `k`'s
    /// from `bands[k]` (an RGB565 picture `W` pixels wide, its rows
    /// `STRIDE16` apart, from the allocator: the graphics service checks
    /// it) or, where `None` or past the list, from the wallpaper itself;
    /// `changed` is all the screen this changes, in up to [`WALL_BOXES`]
    /// areas (more are merged; R184 step 2: a radar frame composed once into
    /// its bands, a loop step switching them, the areas where the rain
    /// differs). The background and the screen are made again there only,
    /// not whole, and cached areas over them are dropped. No bands (`&[]`)
    /// reads the wallpaper alone again.
    pub fn wallpaper_bands(&mut self, top: u16, h: u16, bands: &[Option<usize>], changed: &[Rect]) {
        let mut b = [None; BANDS];
        for (k, (out, at)) in b.iter_mut().zip(bands).enumerate() {
            let row = usize::from(top) + k * usize::from(h);
            *out = at.map(|a| a.wrapping_sub(row * STRIDE16));
        }
        self.bands = (top, h.max(1), b);
        for c in changed {
            let c = c.intersect(&SCREEN);
            if c.is_empty() {
                continue;
            }
            let (boxes, n) = &mut self.wall_box;
            if *n < WALL_BOXES {
                boxes[*n] = c;
                *n += 1;
            } else {
                boxes[WALL_BOXES - 1] = boxes[WALL_BOXES - 1].union(&c);
            }
            // A cached area over the change holds the old wallpaper.
            let mut k = 0;
            while k < self.cached_n {
                if self.cached[k].1.intersect(&c).is_empty() {
                    k += 1;
                } else {
                    self.cached_n -= 1;
                    self.cached[k] = self.cached[self.cached_n];
                }
            }
        }
    }

    /// Draws the whole screen afresh on the next two frames (an area that
    /// was another engine's comes back).
    pub fn invalidate_all(&mut self) {
        self.fresh = 2;
        self.row_keys = [None; ROWS];
    }

    /// Keeps the screen as shown now: the next frame first copies the
    /// picture on screen into the kept picture ([`Layout::kept`]), one
    /// engine copy queued ahead of its drawing (no CPU wait), so a later
    /// frame can start from it ([`Self::seed`]). For a screen being left
    /// that will be come back to (the owner's ruling of 2026-11-12 on R191:
    /// Today's last picture). Nothing is kept while the wallpaper reads
    /// bands from elsewhere (their pictures may change unseen).
    pub fn keep_shown(&mut self) {
        self.keep_next = true;
    }

    /// Names the wallpaper written since the last change ([`Self::wallpaper_row`],
    /// [`Self::wallpaper_changed`]): the caller's word that the same `id`
    /// always means the same pixels. A screen kept over it stays usable
    /// across changes that end with the same wallpaper written again (the
    /// radar's map written into the wallpaper's rows and the wallpaper put
    /// back: run today-0138 refused Today's kept screen after three such
    /// round trips, its pixels the same).
    pub fn wallpaper_is(&mut self, id: u32) {
        self.wall_id = Some(id);
    }

    /// Whether the kept picture holds a whole screen drawn over the
    /// wallpaper as it is now (unchanged since, or the same named one
    /// written again; the wallpaper alone, no bands).
    #[must_use]
    pub fn kept_ok(&self) -> bool {
        let same_wall = |g: u32, id: Option<u32>| {
            g == self.wall_gen || id.is_some_and(|id| self.wall_id == Some(id))
        };
        matches!(self.kept, Some((g, id, true)) if same_wall(g, id))
            && self.bands.2.iter().all(Option::is_none)
    }

    /// The kept picture's address ([`Layout::kept`]): a screen composed
    /// ahead into it by the graphics service, then [`Self::kept_composed`].
    #[must_use]
    pub const fn kept_at(&self) -> usize {
        self.l.kept
    }

    /// A screen is being composed into the kept picture (`None`: nothing
    /// usable until it lands), or has landed (`Some(whole)`): over the
    /// wallpaper as it is now.
    pub fn kept_composed(&mut self, landed: Option<bool>) {
        self.kept = landed.map(|whole| (self.wall_gen, self.wall_id, whole));
    }

    /// The next frame starts from the kept picture: copied whole into the
    /// picture drawn into, then only the frame's damage drawn over it, so
    /// the caller marks only where its list differs from the kept screen's
    /// (`rp_draw::changes`). `false` (nothing changes) when the kept
    /// picture can't be used: the caller then draws whole. If the kept
    /// picture turns unusable before the frame, the frame is drawn whole.
    pub fn seed(&mut self) -> bool {
        self.seed_next = self.kept_ok();
        self.seed_next
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

    /// The picture store `Cmd::Pic` reads from.
    pub fn set_pics(&mut self, pics: &'static rp_draw::pics::Pics<'static>) {
        self.pics = Some(pics);
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
        let end = r.y + r.h;
        let mut y = r.y;
        while y < end {
            // The longest run of rows read from one place (bands from the
            // wallpaper itself merge with it: one copy, as before bands).
            let (src, mut next) = self.wallpaper_src(y);
            while next < end {
                let (s2, n2) = self.wallpaper_src(next);
                if s2 != src {
                    break;
                }
                next = n2;
            }
            let next = next.min(end);
            self.copy_as(wide(dst), narrow(src), Rect::new(r.x, y, r.w, next - y));
            y = next;
        }
    }

    /// Where wallpaper row `y` is read from (laid out as the screen), and
    /// the first row after `y` that may be read from elsewhere.
    fn wallpaper_src(&self, y: u16) -> (usize, u16) {
        let (top, h, bands) = self.bands;
        let last = top.saturating_add(h.saturating_mul(BANDS as u16));
        match y.checked_sub(top) {
            _ if bands.iter().all(Option::is_none) => (self.l.wallpaper, H),
            None => (self.l.wallpaper, top),
            Some(_) if y >= last => (self.l.wallpaper, H),
            Some(d) => {
                let k = usize::from(d / h);
                (
                    bands[k].unwrap_or(self.l.wallpaper),
                    top + (k as u16 + 1) * h,
                )
            }
        }
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

    /// Where the ring holds the row `key`, writing it if it isn't there:
    /// a row already written (this frame or an earlier one: the ring is
    /// kept) is reused. A slot is overwritten only once the engine can no
    /// longer read it (stamped before the last frame start or full wait),
    /// else everything queued finishes first.
    fn row_at(&mut self, key: RowKey) -> usize {
        if let Some(k) = self.row_keys.iter().position(|r| *r == Some(key)) {
            self.row_used[k] = self.epoch;
            self.last.rows_reused += 1;
            return self.l.rows + k * ROW_BYTES;
        }
        let k = self.row % ROWS;
        self.row = (k + 1) % ROWS;
        if self.row_used[k] == self.epoch {
            self.wait();
        }
        self.row_used[k] = self.epoch;
        self.row_keys[k] = Some(key);
        self.last.rows_written += 1;
        let at = self.l.rows + k * ROW_BYTES;
        let w = usize::from(key.w).min(ROW_BYTES / 4);
        let mut buf = [0u32; CHUNK];
        for k0 in (0..w).step_by(CHUNK) {
            let n = CHUNK.min(w - k0);
            for (i, p) in buf[..n].iter_mut().enumerate() {
                *p = rp_gfx::render::gradient_at(
                    key.a,
                    key.b,
                    key.first + (k0 + i) as u32,
                    key.span,
                );
            }
            self.mem.store(at + 4 * k0, &buf[..n]);
        }
        self.mem.sync();
        at
    }

    /// One row of colours (`key`, `r.w` wide) drawn down `r` (blended if
    /// `blend`).
    fn row(&mut self, dst: usize, r: Rect, key: RowKey, blend: bool) {
        let at = self.row_at(key);
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
            _ => self.row(
                dst,
                r,
                RowKey {
                    a: c,
                    b: c,
                    first: 0,
                    span: 1,
                    w: r.w,
                },
                true,
            ),
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
        // Everything queued so far is read: every row is free again.
        self.epoch = self.epoch.wrapping_add(1);
        ok
    }

    /// Where `img` is uploaded (uploading it now if it isn't).
    fn image(&mut self, img: &'static rp_draw::Image) -> Option<usize> {
        let key = img.px.as_ptr() as usize;
        if let Some(at) = self.cached(key) {
            return Some(at);
        }
        let n = usize::from(img.w) * usize::from(img.h);
        self.upload(key, img.px.get(..n)?)
    }

    /// Where the store's `pic` is uploaded (uploading it now if it isn't):
    /// kept by its handle's key (slot and era, ADR-0030 G3), so a slot
    /// reused for another picture is uploaded again, never shown stale.
    /// Odd keys: a static picture's key is its 4-aligned address.
    fn pic(&mut self, pic: rp_draw::pics::Pic) -> Result<usize, rp_draw::pics::Miss> {
        use rp_draw::pics::Miss;
        let key = (pic.key() as usize) << 2 | 1;
        if let Some(at) = self.cached(key) {
            return Ok(at);
        }
        let pics = self.pics.ok_or(Miss::NoStore)?;
        match pics.read(pic, |px| self.upload(key, px)) {
            Some(Some(at)) => Ok(at),
            Some(None) => Err(Miss::TooBig),
            None => Err(pics.why_not(pic)),
        }
    }

    /// An uploaded picture's place, by key (marked drawn this frame).
    fn cached(&mut self, key: usize) -> Option<usize> {
        let now = self.frame_no;
        let e = self.images.iter_mut().flatten().find(|e| e.0 == key)?;
        e.3 = now;
        Some(self.l.images.0 + e.1)
    }

    /// The lowest free run of `bytes` in the image cache, with a free slot.
    fn image_gap(&self, bytes: usize) -> Option<usize> {
        self.images.iter().position(Option::is_none)?;
        let mut at = 0;
        loop {
            // The first picture at or past `at` that overlaps [at, at + bytes).
            let hit = self
                .images
                .iter()
                .flatten()
                .filter(|e| e.1 < at + bytes && e.1 + e.2 > at)
                .map(|e| e.1 + e.2)
                .max();
            match hit {
                None if at + bytes <= self.l.images.1 => return Some(at),
                None => return None,
                Some(end) => at = end,
            }
        }
    }

    /// Puts `px` in the image cache under `key`. With no room, the picture
    /// drawn longest ago that this frame doesn't draw goes, one at a time
    /// (the owner's ruling of 2026-11-10; the cache used to empty whole, so
    /// Today's 315 KB never stayed); if this frame's own pictures fill it,
    /// the engine is let finish and the cache starts again.
    fn upload(&mut self, key: usize, px: &[Argb]) -> Option<usize> {
        let bytes = (px.len() * 4).next_multiple_of(16);
        if bytes > self.l.images.1 {
            return None;
        }
        let now = self.frame_no;
        let off = loop {
            if let Some(off) = self.image_gap(bytes) {
                break off;
            }
            let oldest = self
                .images
                .iter()
                .enumerate()
                .filter_map(|(k, e)| e.filter(|e| e.3 != now).map(|e| (e.3, k)))
                .min();
            if let Some((_, k)) = oldest {
                self.images[k] = None;
                self.last.images_evicted += 1;
            } else {
                self.wait();
                self.images = [None; IMAGES];
            }
        };
        let at = self.l.images.0 + off;
        self.last.images += 1;
        self.last.image_bytes += bytes as u32;
        self.mem.store(at, px);
        self.mem.sync();
        let slot = self.images.iter().position(Option::is_none)?;
        self.images[slot] = Some((key, off, bytes, now));
        Some(at)
    }

    /// The atlas for (`font`, `color`): its slot, made (empty) if needed.
    ///
    /// A pair's glyphs are uploaded as they are first drawn
    /// ([`Self::glyph_at`]). With every slot taken, the pair drawn with
    /// longest ago (never one this frame draws with) gives its slot and its
    /// chunks up; a look switch's new colours push out the old look's pairs
    /// only, never the ones both looks draw with (R172).
    fn atlas(&mut self, font: &'static rp_draw::Font, color: Argb) -> Option<usize> {
        let key = font as *const rp_draw::Font as usize;
        let now = self.frame_no;
        if let Some(k) = self
            .atlases
            .iter()
            .position(|a| a.is_some_and(|a| a.font == key && a.color == color))
        {
            if let Some(a) = self.atlases[k].as_mut() {
                a.used = now;
            }
            return Some(k);
        }
        if font.glyphs.len() > GLYPHS {
            return None;
        }
        let slot = match self.atlases.iter().position(Option::is_none) {
            Some(k) => k,
            None => {
                let k = self.oldest_atlas(None)?;
                self.drop_atlas(k);
                k
            }
        };
        self.last.atlases += 1;
        self.atlases[slot] = Some(Atlas::new(key, color, now));
        Some(slot)
    }

    /// Slot `k`'s pair dropped, its chunks freed.
    fn drop_atlas(&mut self, k: usize) {
        if self.atlases[k].take().is_some() {
            self.last.atlases_evicted += 1;
        }
        for o in &mut self.chunk_owner {
            if usize::from(*o) == k + 1 {
                *o = 0;
            }
        }
    }

    /// The slot of the pair drawn with longest ago that this frame doesn't
    /// draw with (the engine is done with the frames before: `draw` waits
    /// for them), never `keep`.
    fn oldest_atlas(&self, keep: Option<usize>) -> Option<usize> {
        let now = self.frame_no;
        self.atlases
            .iter()
            .enumerate()
            .filter(|&(k, _)| Some(k) != keep)
            .filter_map(|(k, a)| a.filter(|a| a.used != now).map(|a| (a.used, k)))
            .min()
            .map(|(_, k)| k)
    }

    /// Glyph `idx` of `font` in slot `slot`'s colour: its address in 2700G
    /// memory, uploaded first if this pair hasn't drawn it before. A small
    /// glyph goes into the pair's open chunk if it fits, else into a free
    /// chunk that becomes the open one; a big one into a run of free
    /// chunks. With none free, the pairs drawn with longest ago give theirs
    /// up one at a time; if this frame's own glyphs fill the memory, the
    /// engine is let finish and every other pair is dropped.
    fn glyph_at(&mut self, slot: usize, font: &'static rp_draw::Font, idx: usize) -> Option<usize> {
        let g = font.glyphs.get(idx)?;
        let a = self.atlases[slot]?;
        if a.at[idx] != 0 {
            return Some(self.l.glyphs.0 + (usize::from(a.at[idx]) - 1) * 16);
        }
        let (w, h) = (usize::from(g.w), usize::from(g.h));
        if w == 0 || h == 0 || g.cover.len() < w * h {
            return None;
        }
        let bytes = (w * h * 4).next_multiple_of(16);
        let n = (self.l.glyphs.1 / GCHUNK).min(GCHUNKS);
        let k = bytes.div_ceil(GCHUNK);
        if k > n {
            return None;
        }
        let off = if bytes <= GCHUNK - usize::from(a.fill) {
            usize::from(a.open) * GCHUNK + usize::from(a.fill)
        } else {
            let first = loop {
                if let Some(i) = free_run(&self.chunk_owner, n, k) {
                    break i;
                }
                if let Some(o) = self.oldest_atlas(Some(slot)) {
                    self.drop_atlas(o);
                } else {
                    // This frame's own glyphs fill the memory: let the
                    // engine finish what's queued, then start again.
                    self.wait();
                    for o in 0..ATLASES {
                        if o != slot && self.atlases[o].is_some() {
                            self.drop_atlas(o);
                        }
                    }
                    if let Some(b) = self.atlases[slot] {
                        self.drop_atlas(slot);
                        self.last.atlases_evicted -= 1;
                        self.atlases[slot] = Some(Atlas::new(b.font, b.color, b.used));
                    }
                }
            };
            for o in &mut self.chunk_owner[first..first + k] {
                *o = slot as u8 + 1;
            }
            let b = self.atlases[slot].as_mut()?;
            if k == 1 {
                b.open = first as u16;
                b.fill = 0;
            }
            first * GCHUNK
        };
        let b = self.atlases[slot].as_mut()?;
        if k == 1 && off / GCHUNK == usize::from(b.open) {
            b.fill = (off % GCHUNK + bytes) as u16;
        }
        b.at[idx] = (off / 16 + 1) as u16;
        let color = b.color;
        let (rgb, alpha) = (color & 0x00FF_FFFF, color >> 24);
        let at = self.l.glyphs.0 + off;
        let mut buf = [0u32; CHUNK];
        for (i0, cover) in g.cover[..w * h].chunks(CHUNK).enumerate() {
            for (p, &c) in buf.iter_mut().zip(cover) {
                *p = div255(alpha * u32::from(c)) << 24 | rgb;
            }
            self.mem.store(at + 4 * CHUNK * i0, &buf[..cover.len()]);
        }
        self.mem.sync();
        self.last.glyphs += 1;
        Some(at)
    }

    /// Stamps every atlas `frame`'s text draws with as this frame's before
    /// anything is drawn, so making room for a new pair never evicts one
    /// the frame needs later (a look switch's first new colour came before
    /// the pairs it shares, all stamped alike).
    fn mark_atlases(&mut self, frame: &Frame) {
        let now = self.frame_no;
        for c in frame.cmds() {
            match *c {
                Cmd::Text { font, color, .. } => {
                    let key = font as *const rp_draw::Font as usize;
                    for a in self.atlases.iter_mut().flatten() {
                        if a.font == key && a.color == color {
                            a.used = now;
                        }
                    }
                }
                // The frame's pictures too: an upload early in the frame
                // never evicts one it draws later.
                Cmd::Image { img, .. } => self.mark_image(img.px.as_ptr() as usize),
                Cmd::Pic { pic, .. } => self.mark_image((pic.key() as usize) << 2 | 1),
                _ => {}
            }
        }
    }

    /// Picture `key`, if uploaded, marked as this frame's.
    fn mark_image(&mut self, key: usize) {
        let now = self.frame_no;
        for e in self.images.iter_mut().flatten() {
            if e.0 == key {
                e.3 = now;
            }
        }
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
        for (i, (c, own)) in frame.cmds_clipped().take(cmds).enumerate() {
            if self.hoist[i] || self.bounds[i].intersect(&clip).is_empty() {
                continue;
            }
            for w in rect(own) {
                h = fnv(h, w);
            }
            let words: [u32; 5] = match *c {
                Cmd::Clip { .. } => continue,
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
                Cmd::Pic { x, y, pic } => [
                    7,
                    u32::from(x) << 16 | u32::from(y.cast_unsigned()),
                    pic.key(),
                    u32::from(pic.w) << 16 | u32::from(pic.h),
                    0,
                ],
                Cmd::Panel { r, art } => {
                    let [a, b] = rect(r);
                    [8, a, b, core::ptr::from_ref(art) as usize as u32, 0]
                }
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

    // Out of line: called from two places in `draw` (under a panel and
    // not), one copy (inlined twice it grew rp-scene ~5 KB).
    #[inline(never)]
    fn command(&mut self, frame: &Frame, c: &Cmd, clip: Rect, dst: usize) {
        match *c {
            Cmd::Clip { .. } => {}
            // Its fills and images (ADR-0023 rule 3): the wallpaper copied
            // once under its body (the clears), then its rows and rim only
            // blended over it, not each a copy and a blend (R176); the
            // corners as images.
            Cmd::Panel { r, art } => {
                rp_draw::panel_cmds(r, art, |p| match p {
                    Cmd::Fill { r: f, color } => {
                        let t = f.intersect(&clip);
                        if t.is_empty() {
                        } else if color == rp_draw::CLEAR {
                            self.wallpaper(dst, t);
                        } else {
                            self.over(dst, t, color);
                        }
                    }
                    p => self.command(frame, &p, clip, dst),
                });
            }
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
                let key = if left == right {
                    RowKey {
                        a: left,
                        b: left,
                        first: 0,
                        span: 1,
                        w: t.w,
                    }
                } else {
                    RowKey {
                        a: left,
                        b: right,
                        first,
                        span: u32::from(r.w),
                        w: t.w,
                    }
                };
                self.row(dst, t, key, !opaque);
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
            Cmd::Pic { x, y, pic } => {
                let Some((to, sx, sy)) = place(pic.w, pic.h, i32::from(x), i32::from(y), clip)
                else {
                    return;
                };
                match self.pic(pic) {
                    Ok(at) => self.blend_from(dst, to, (at, usize::from(pic.w) * 4), sx, sy),
                    Err(why) => self.last.pics_missed.note(pic, why),
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
                    let (w, h) = (u16::from(g.w), u16::from(g.h));
                    // Uploaded only if some of it shows.
                    if let Some((to, sx, sy)) = place(w, h, gx, gy, clip) {
                        let Some(at) = self.glyph_at(k, font, idx) else {
                            continue;
                        };
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
        self.frame_no = self.frame_no.wrapping_add(1);
        self.mark_atlases(frame);
        self.dropping = false;
        // A held frame nobody flipped: show it now.
        self.flip();
        while !self.poll() {
            if self.us_since(t0) > 200_000 {
                self.stalls += 1;
                self.pending = None;
                // A copy into the kept picture in that frame may not have
                // run whole.
                if core::mem::take(&mut self.keep_landing) {
                    self.kept = None;
                }
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
        // The previous frame has landed: no row of the ring is read now.
        self.epoch = self.epoch.wrapping_add(1);
        let mut cmds = 0;
        for (i, (c, clip)) in frame.cmds_clipped().take(MAX_CMDS).enumerate() {
            cmds = i + 1;
            let b = c.bounds(frame).intersect(&clip);
            self.bounds[i] = b;
            let glass = matches!(
                c,
                Cmd::Fill { .. } | Cmd::HGradient { .. } | Cmd::Outline { .. }
            );
            let covers_earlier =
                (0..i).any(|j| !self.hoist[j] && !self.bounds[j].intersect(&b).is_empty());
            self.hoist[i] = glass && !covers_earlier && !b.is_empty();
            if self.hoist[i] {
                let (kind, a, b2, r) = match *c {
                    Cmd::Fill { color, r } => (1, color, 0, r),
                    Cmd::HGradient { left, right, r } => (2, left, right, r),
                    Cmd::Outline { color, r } => (3, color, 0, r),
                    _ => (0, 0, 0, b),
                };
                let mut key = fnv(0x811C_9DC5u32, self.wall_gen);
                for w in [
                    kind,
                    u32::from(r.x) << 16 | u32::from(r.y),
                    u32::from(r.w) << 16 | u32::from(r.h),
                    a,
                    b2,
                    u32::from(clip.x) << 16 | u32::from(clip.y),
                    u32::from(clip.w) << 16 | u32::from(clip.h),
                ] {
                    key = fnv(key, w);
                }
                // Order matters only where hoisted glass overlaps.
                for j in 0..i {
                    if self.hoist[j] && !self.bounds[j].intersect(&b).is_empty() {
                        key = fnv(key, self.keys[j]);
                    }
                }
                self.keys[i] = key;
            }
        }
        let mut state = [0u32; 6];
        state[0] = cmd::SCALE_1_TO_1;
        state[1..4].copy_from_slice(&cmd::key(0, 0));
        state[4..].copy_from_slice(&cmd::blend(BLEND_DELL_TEXT));
        self.submit(&state);
        // Where the background must be made again: everywhere if it was
        // never made (or the wallpaper changed), else only where hoisted
        // glass came, went or changed (its old and new places).
        let mut redo = [Rect::default(); MAX_DIRTY];
        let mut nr = 0;
        let whole = self.bg_hash.is_none();
        // A band switch: the background and the screen there (all of it
        // anyway when whole).
        let (boxes, nb) =
            core::mem::replace(&mut self.wall_box, ([Rect::new(0, 0, 0, 0); WALL_BOXES], 0));
        let wall_box = &boxes[..if whole { 0 } else { nb }];
        if whole {
            redo[0] = SCREEN;
            nr = 1;
        } else {
            let mut add = |r: Rect| {
                let mut r = r.intersect(&SCREEN);
                let area = |r: &Rect| u32::from(r.w) * u32::from(r.h);
                // Overlapping places merge when their union costs no more
                // than the two (a panel's old and new place, nudged).
                while let Some(k) = (0..nr).find(|&k| {
                    area(&redo[k].union(&r)) <= area(&redo[k]) + area(&r)
                        && !redo[k].intersect(&r).is_empty()
                }) {
                    r = redo[k].union(&r);
                    nr -= 1;
                    redo[k] = redo[nr];
                }
                if r.is_empty() {
                    return;
                }
                if nr < MAX_DIRTY {
                    redo[nr] = r;
                    nr += 1;
                } else {
                    let best = (0..nr)
                        .min_by_key(|&k| area(&redo[k].union(&r)) - area(&redo[k]))
                        .unwrap_or(0);
                    redo[best] = redo[best].union(&r);
                }
            };
            for &b in wall_box {
                add(b);
            }
            for &(k, r) in &self.bg_keys[..self.bg_n] {
                if !(0..cmds).any(|i| self.hoist[i] && self.keys[i] == k) {
                    add(r);
                }
            }
            for i in 0..cmds {
                if self.hoist[i]
                    && !self.bg_keys[..self.bg_n]
                        .iter()
                        .any(|&(k, _)| k == self.keys[i])
                {
                    add(self.bounds[i]);
                }
            }
        }
        if nr > 0 {
            let bg = self.l.background;
            for &area in &redo[..nr] {
                self.wallpaper(bg, area);
                for (i, (c, own)) in frame.cmds_clipped().take(cmds).enumerate() {
                    if self.hoist[i] && !self.bounds[i].intersect(&area).is_empty() {
                        let clip = own.intersect(&area);
                        self.command(frame, c, clip, bg);
                    }
                }
                self.last.background_px += u32::from(area.w) * u32::from(area.h);
            }
            self.bg_n = 0;
            for i in 0..cmds {
                if self.hoist[i] {
                    self.bg_keys[self.bg_n] = (self.keys[i], self.bounds[i]);
                    self.bg_n += 1;
                }
            }
            self.last.background = true;
        }
        // The area cache's keys start from the background's: any change
        // to it makes every cached area over it new.
        let hash = self.bg_keys[..self.bg_n]
            .iter()
            .fold(fnv(0x811C_9DC5, self.wall_gen), |h, &(k, _)| fnv(h, k));
        self.bg_hash = Some(hash);
        if whole {
            // Both pictures' contents are unknown: drawn whole.
            self.fresh = 2;
        }
        // What this frame changed is drawn; what the previous frame changed
        // (the picture drawn into is two frames old) is copied forward from
        // the picture on screen, one plain copy instead of drawing it again
        // (2026-10-14: the frame after the Start menu opened drew the whole
        // menu a second time, glass, text and dither: ~30 ms of engine).
        // Started from the kept picture (`seed`): its copy is the frame's
        // base, so nothing is brought forward and only the damage (where
        // the list differs from the kept screen's) is drawn. Asked for but
        // unusable now, or the pictures' contents unknown (a reset, a new
        // wallpaper): drawn whole, as the caller's damage assumed the base.
        let asked = core::mem::take(&mut self.seed_next);
        let seeded = asked && self.kept_ok() && self.fresh <= 1;
        if asked && !seeded {
            self.fresh = 2;
        }
        self.last.seeded = seeded;
        let mut rects = [Rect::default(); MAX_DIRTY];
        let mut n = 0;
        let mut forward = [Rect::default(); MAX_DIRTY];
        let mut nf = 0;
        if !seeded && (self.fresh > 1 || (self.fresh == 1 && !self.front_whole)) {
            rects[0] = SCREEN;
            n = 1;
        } else {
            // The frame after a whole one (start-up, a new background): the
            // other picture is two frames old, so all of it comes forward
            // (2026-10-14: the first tap after start-up drew the whole
            // screen again, 14.7 ms of CPU and 23.7 of engine).
            if self.fresh == 1 && !seeded {
                forward[0] = SCREEN;
                nf = 1;
            }
            // The frame's own and the band switch's, never a pixel twice
            // where it can be helped (rp_draw::add_damage, R172).
            for r in frame.dirty().iter().chain(wall_box.iter()) {
                rp_draw::add_damage(&mut rects, &mut n, r.intersect(&SCREEN));
            }
            // (Nothing more to bring forward when it's all coming, or the
            // kept picture is the base.)
            let prev = if nf == 0 && !seeded { self.prev_n } else { 0 };
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
        // The band switch's area, brought forward into the other picture
        // next frame as the frame's own changes are.
        for &b in wall_box {
            if self.prev_n < MAX_DIRTY {
                self.prev[self.prev_n] = b;
                self.prev_n += 1;
            } else {
                self.prev[MAX_DIRTY - 1] = self.prev[MAX_DIRTY - 1].union(&b);
            }
        }
        if seeded {
            // Against the picture on screen, the whole screen changed.
            self.drawn[0] = SCREEN;
            self.drawn_n = 1;
        } else {
            self.drawn_n = n;
            self.drawn[..n].copy_from_slice(&rects[..n]);
        }
        if n == 0 && nf == 0 && !seeded {
            // (A keep waits for a frame that draws: the screen is as shown.)
            return drawn;
        }
        let back = 1 - self.front;
        let dst = self.l.screens[back];
        let shown = self.l.screens[self.front];
        let work = self.l.work;
        // The screen shown into the kept picture, ahead of everything this
        // frame draws (it reads the picture on screen, which this frame
        // doesn't write).
        if core::mem::take(&mut self.keep_next) {
            if self.bands.2.iter().all(Option::is_none) {
                self.copy_as(narrow(self.l.kept), narrow(shown), SCREEN);
                self.kept = Some((self.wall_gen, self.wall_id, false));
                self.keep_landing = true;
                self.last.kept = true;
            } else {
                self.kept = None;
            }
        }
        if seeded {
            self.copy_as(narrow(dst), narrow(self.l.kept), SCREEN);
            drawn.pixels += u64::from(W) * u64::from(H);
        }
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
            // Under a panel's body nothing before it shows: the background
            // and the commands before it go only where it leaves them.
            let bg = self.l.background;
            let under = under_panel(frame, cmds, clip);
            match &under {
                Some((_, pieces)) => {
                    for &q in pieces.all() {
                        self.copy(work, bg, q);
                    }
                }
                None => self.copy(work, bg, clip),
            }
            for (i, (c, own)) in frame.cmds_clipped().take(cmds).enumerate() {
                if self.hoist[i] || self.bounds[i].intersect(&clip).is_empty() {
                    continue;
                }
                // Before the panel: only its pieces; else the whole area.
                let whole = [clip];
                let parts = match &under {
                    Some((p, pieces)) if i < *p => pieces.all(),
                    _ => &whole[..],
                };
                let mut any = false;
                for q in parts {
                    let t = own.intersect(q);
                    if !self.bounds[i].intersect(&t).is_empty() {
                        self.command(frame, c, t, work);
                        any = true;
                    }
                }
                if any {
                    drawn.cmds += 1;
                } else {
                    self.last.hidden += 1;
                }
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
        if seeded {
            // The other picture differs from this one everywhere: the next
            // frame brings the whole screen forward (as after a whole one).
            self.fresh = self.fresh.max(2);
        }
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
        // A copy into the kept picture rode this frame: usable only if the
        // frame ran whole (no stall, nothing dropped).
        if core::mem::take(&mut self.keep_landing) {
            let whole = self.pending_whole && !self.pending_dropped;
            self.kept = self.kept.filter(|_| whole).map(|(g, id, _)| (g, id, true));
        }
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
