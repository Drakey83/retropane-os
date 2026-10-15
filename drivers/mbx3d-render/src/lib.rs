//! RetroPane's own MBX 3D renderer (ADR-0026): built for RetroPane's
//! work, not a copy of Dell's driver (whose code stays a source of hardware
//! facts only).
//!
//! What it does differently from Dell's (docs/research/driver-inefficiencies.md):
//!
//! - **A retained scene** (D1, option A): up to [`MAX_SLOTS`] quads, each
//!   written into the 3D parameter memory once, when it changes; a frame
//!   that changes nothing writes nothing.
//! - **Only changed tiles are rendered** (D2, measured ~5.2 us a tile,
//!   `rp-mbx3d` version 19): a change marks the tiles it touched (before
//!   and after); the region array lists just those. A button of ~8 tiles
//!   renders in ~60 us where Dell's renders the whole screen (12.5 ms).
//! - **Lists shared by tiles**: each tile's object list holds only the
//!   quads that touch it, and tiles with the same quads share one list
//!   (kept from frame to frame), so a full screen of large quads needs a
//!   handful of lists, not 2400.
//! - **State written once** (D4): a [`Shadow`] of the 3D registers; a frame
//!   after the first costs the start write, one status read and its clear.
//! - **Straight into the screen picture** (D5, version 20: any pixel
//!   offset, stride kept): a [`Target`] is a rectangle of an ADR-0023
//!   picture, no copy.
//! - **The CPU doesn't spin** (D3): GPIO85 never edged for render done
//!   (version 20), so [`Renderer::render`] returns the predicted finish
//!   (~16 us + 5.2 us a tile) and [`Renderer::poll`] reads the status once
//!   then; the caller sleeps until it.
//! - **A hung render is recovered** (version 22's lesson): past a deadline
//!   the core is reset and the memory manager set up again, and the next
//!   frame redraws everything.
//! - **The 3D clock is on only while there's work** (D8, `clock::Gate`).
//!
//! Every word reaching the chip costs ~120 ns over the bus (the real
//! bottleneck), so the renderer counts them ([`FrameStats`]).
//!
//! No floating-point arithmetic: positions are in 1/16 pixel, depths and
//! texture coordinates fixed point, and the floats the core reads are built
//! from them with integer operations ([`fixed`]).
//!
//! Hardware access goes through a [`RegisterBus`] (the 2700G register
//! window) and [`LocalMem`] (its memory) passed to each call, so the
//! graphics service (one GPU owner, D6) keeps both, and the same code runs
//! on the device and, with the `model` feature, against a model of the 3D
//! core.
//!
//! **A rendered tile is written whole**: the core fills it from its own
//! tile buffer (uncovered pixels white, measured), it doesn't load what the
//! picture held. So the scene must cover every tile it renders: the
//! [`Renderer::set_background`] colour (on by default) is drawn first in
//! every tile. It also writes depth (z 0): a tile's depth starts undefined
//! (run 1), so without a background, depth tests need a quad writing depth
//! under them first.

#![cfg_attr(not(any(test, feature = "model")), no_std)]
#![forbid(unsafe_code)]

pub mod fixed;
#[cfg(any(test, feature = "model"))]
pub mod model;
mod shadow;
mod tiles;

pub use rp_drv_mbx3d::{depth, factor};
pub use rp_vram_map::LocalMem;
pub use shadow::Shadow;
pub use tiles::{MAX_TILES, TileRect, Tiles};

use rp_drv_blit2d::{MBX, reg, reset_core};
use rp_drv_mbx_display::clock::Gate;
use rp_drv_mbx3d::{self as m3, tex};
use rp_hal_api::RegisterBus;
use rp_vram_map::{Pool, Region};

/// Sub-pixel units a pixel: positions are in 1/16 pixel.
pub const SUB: i32 = 16;
/// Texture coordinate 1.0 (16.16 fixed point).
pub const UV_ONE: i32 = 1 << 16;
/// Quads a scene holds.
pub const MAX_SLOTS: usize = 64;
/// The 3D clock goes off this long after the last render (D8).
pub const CLOCK_IDLE_US: u32 = 100_000;

/// Texel formats (TEX0 bits 15-13, version 18).
pub mod format {
    pub use rp_drv_mbx3d::tex::{ARGB1555, ARGB4444, RGB332, RGB565};
}
/// What a texture coordinate past the texture does (TEX0 bits 31-28,
/// version 22).
pub mod address {
    pub use rp_drv_mbx3d::tex::{CLAMP, MIRROR, WRAP};
}

/// TEX2 (the combine word) for textures without alpha: Dell's (version 21:
/// the 8 x 8 checker drew exactly).
pub const TEX2_OPAQUE: u32 = 0xD688_6610;
/// TEX2 for textures with alpha: the GLES driver's RGBA modulate (version
/// 23: an ARGB4444 texture at alpha 8/15 blended exactly).
pub const TEX2_ALPHA: u32 = 0x8608_4610;

/// Time and waiting, in microseconds (any wrapping clock).
pub trait Clock {
    /// Now.
    fn now_us(&mut self) -> u32;
    /// Waits `us`.
    fn delay_us(&mut self, us: u32);
}

/// A position in 1/16 pixel ([`SUB`]), from the target's top left.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Point {
    /// Across.
    pub x: i32,
    /// Down.
    pub y: i32,
}

impl Point {
    /// Pixel `(x, y)`.
    #[must_use]
    pub const fn px(x: i32, y: i32) -> Self {
        Self {
            x: x * SUB,
            y: y * SUB,
        }
    }
}

/// A texture uploaded into 2700G memory: square, twiddled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Texture {
    /// Where (128-byte aligned).
    pub at: u32,
    /// Side `8 << code` texels.
    pub code: u8,
    /// One of [`format`].
    pub format: u8,
    /// Across: one of [`address`].
    pub u_mode: u8,
    /// Down: one of [`address`].
    pub v_mode: u8,
}

impl Texture {
    /// Texels a side.
    #[must_use]
    pub const fn side(&self) -> u32 {
        8 << self.code
    }

    /// Bytes it takes.
    #[must_use]
    pub const fn bytes(&self) -> u32 {
        self.side() * self.side() * 2
    }

    /// The same texture with other address modes.
    #[must_use]
    pub const fn with_modes(self, u: u32, v: u32) -> Self {
        Self {
            u_mode: (u & 3) as u8,
            v_mode: (v & 3) as u8,
            ..self
        }
    }

    /// Whether its texels carry alpha.
    #[must_use]
    pub const fn has_alpha(&self) -> bool {
        matches!(self.format as u32, tex::ARGB1555 | tex::ARGB4444)
    }

    /// TEX0, TEX1 and TEX2.
    #[must_use]
    pub fn words(&self) -> [u32; 3] {
        let tex0 = tex::with_address(
            tex::tex0(self.code as u32, self.format as u32),
            self.u_mode as u32,
            self.v_mode as u32,
        );
        let tex1 = m3::ta::dell_texture(self.at)[1];
        let tex2 = if self.has_alpha() {
            TEX2_ALPHA
        } else {
            TEX2_OPAQUE
        };
        [tex0, tex1, tex2]
    }
}

/// How a quad is coloured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Paint {
    /// One colour, `0xAARRGGBB`.
    Flat(u32),
    /// A colour per corner (top left, top right, bottom right, bottom
    /// left), blended between them (TSP bit 22).
    Gouraud([u32; 4]),
    /// A texture, its coordinates per corner (16.16, [`UV_ONE`] = the far
    /// edge), times `tint` (white: the texture as it is).
    Texture {
        /// The texture.
        tex: Texture,
        /// Coordinates per corner.
        uv: [(i32, i32); 4],
        /// Colour the texels are multiplied by.
        tint: u32,
    },
}

impl Paint {
    /// The whole of `tex`, once, untinted.
    #[must_use]
    pub const fn texture(tex: Texture) -> Self {
        Self::Texture {
            tex,
            uv: [(0, 0), (UV_ONE, 0), (UV_ONE, UV_ONE), (0, UV_ONE)],
            tint: 0xFFFF_FFFF,
        }
    }
}

/// A depth test (version 23: larger z is nearer). Only the modes the
/// device showed working are accepted ([`DEPTH_MODES`]: "greater",
/// "greater or equal", "always"); [`Renderer::set`] refuses the others.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Depth {
    /// One of [`depth`].
    pub mode: u32,
    /// Whether passing pixels store their depth.
    pub write: bool,
    /// The quad's depth, `z / 65536` (each tile starts at 0).
    pub z: u16,
}

/// The depth modes [`Renderer::set`] accepts: the ones that behaved as a
/// compare on the device. "Less" and "less or equal" never passed in any
/// test (`rp-mbx3d-api` run 3, S8: whatever the floor's z, with and without
/// writes), and in a frame rendering only some tiles one of them under
/// another quad's tiles made an "always" quad below it vanish (runs 1-3);
/// "never", "equal" and "not equal" were never tried.
pub const DEPTH_MODES: [u32; 3] = [depth::GREATER, depth::GEQUAL, depth::ALWAYS];

/// One quad of the scene.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Draw {
    /// Top left, top right, bottom right, bottom left.
    pub corners: [Point; 4],
    /// Its colour or texture.
    pub paint: Paint,
    /// Blend factors (source, destination: [`factor`]), or opaque.
    pub blend: Option<(u32, u32)>,
    /// A depth test, or none (drawn over whatever is there).
    pub depth: Option<Depth>,
}

impl Draw {
    /// A `w` x `h` pixel rectangle at `(x, y)`.
    #[must_use]
    pub const fn rect(x: i32, y: i32, w: i32, h: i32, paint: Paint) -> Self {
        Self {
            corners: [
                Point::px(x, y),
                Point::px(x + w, y),
                Point::px(x + w, y + h),
                Point::px(x, y + h),
            ],
            paint,
            blend: None,
            depth: None,
        }
    }

    /// Blended over what's below by its alpha (source alpha, inverse).
    #[must_use]
    pub const fn over(self) -> Self {
        Self {
            blend: Some((factor::SRC_ALPHA, factor::INV_SRC_ALPHA)),
            ..self
        }
    }

    /// With a depth test.
    #[must_use]
    pub const fn with_depth(self, d: Depth) -> Self {
        Self {
            depth: Some(d),
            ..self
        }
    }
}

/// A picture in 2700G memory (32 bpp).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Picture {
    /// Its first pixel.
    pub at: u32,
    /// Bytes a line.
    pub stride: u32,
    /// Width in pixels.
    pub w: u16,
    /// Height in pixels.
    pub h: u16,
}

impl Picture {
    /// ADR-0023's screen picture `n` (0 or 1), from the memory map.
    #[must_use]
    pub const fn screen(n: usize) -> Self {
        Self {
            at: rp_vram_map::SCREENS[n & 1].at,
            stride: 480 * 4,
            w: 480,
            h: 640,
        }
    }
}

/// Where a frame goes: the rectangle of `picture` at `(x, y)` the
/// renderer's size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    /// The picture.
    pub picture: Picture,
    /// Left edge in it.
    pub x: u16,
    /// Top edge in it.
    pub y: u16,
}

impl Target {
    /// The rectangle's first pixel (`0x65C`).
    #[must_use]
    pub const fn addr(&self) -> u32 {
        self.picture.at + self.y as u32 * self.picture.stride + self.x as u32 * 4
    }
}

/// Why a call was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// A render is in flight: [`Renderer::poll`] it to the end first.
    Busy,
    /// No such slot.
    BadSlot,
    /// The target doesn't fit its picture, or isn't the renderer's size.
    BadTarget,
    /// The 2700G memory pool is full.
    NoMemory,
    /// The 3D core never said its memory manager was ready.
    NoCore,
    /// The frame's object lists don't fit the parameter memory.
    TooComplex,
    /// A depth mode outside [`DEPTH_MODES`].
    Unsupported,
}

/// What [`Renderer::poll`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Poll {
    /// Nothing in flight.
    Idle,
    /// Not done: ask again at this time.
    Wait(u32),
    /// Done.
    Done(FrameStats),
    /// It hung past its deadline: the core was reset and set up again
    /// (whole MBX: the 2D engine's stream restarts, `Engine::after_core_reset`),
    /// and the next frame redraws everything.
    Recovered,
}

/// What a frame cost.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameStats {
    /// Tiles rendered.
    pub tiles: u32,
    /// Quads listed in them, summed over the tiles.
    pub quads: u32,
    /// Object lists written for it (the others reused).
    pub lists: u32,
    /// Words stored into 2700G memory since the previous frame (quads,
    /// lists, the region array).
    pub mem_words: u32,
    /// Register writes for it.
    pub reg_writes: u32,
    /// The predicted render time.
    pub predicted_us: u32,
    /// When the poll saw it done (an upper bound).
    pub done_us: u32,
}

/// The predicted render time of `tiles` tiles whose quads (beyond the
/// background) cost `quarters` quarter-microseconds in all (see
/// [`quad_cost`]). Fitted to `rp-mbx3d-api` run 2's exact times: ~10 us
/// plus 5.25 us a tile (1 tile 15 us, 2400 tiles 12,521 us; 16 tiles
/// 92-95 us).
#[must_use]
pub const fn predict_us(tiles: u32, quarters: u32) -> u32 {
    10 + tiles * 21 / 4 + quarters / 4
}

/// What one quad adds to a tile's render time, in quarter-microseconds
/// (run 2): opaque flat or Gouraud ~0.3-0.45 us (100 tiles under one quad
/// 559 us; 325 tiles, 416 quads 1,888 us), textured ~1.1 us (285 quads
/// 12,813 us), blended ~4.9 us, about a tile's worth (338 additive tiles
/// 3,387 us; 378 alpha quads 14,342 us).
#[must_use]
pub const fn quad_cost(textured: bool, blended: bool) -> u32 {
    if blended {
        20
    } else if textured {
        4
    } else {
        2
    }
}

/// Where things sit in the parameter memory, in bytes from its start (the
/// start table's `0x60C`; strip pointers count words from it).
const BG_AT: u32 = 64;
const SLOTS_AT: u32 = 256;
/// A slot: two textured one-triangle objects (2 x 26 words) fit.
const SLOT_BYTES: u32 = 256;
const ARENA_AT: u32 = SLOTS_AT + MAX_SLOTS as u32 * SLOT_BYTES;
/// Object lists kept for reuse.
const LISTS: usize = 128;
/// The longest list: the background, two triangles a slot, the end.
const LIST_MAX: usize = 2 + 2 * MAX_SLOTS;
/// Words of a slot's two objects, at most.
const SLOT_WORDS: usize = 2 * tex::WORDS;
const _: () = assert!(SLOT_WORDS * 4 <= SLOT_BYTES as usize);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Kind {
    #[default]
    Empty,
    Flat,
    Textured,
}

impl Kind {
    /// The strip pointer flags and the words of one object.
    const fn strip(self) -> (u32, u32) {
        match self {
            Self::Textured => (tex::STRIP_GAME, tex::WORDS as u32),
            _ => (m3::STRIP_ONE_TRIANGLE, m3::TRIANGLE_WORDS as u32),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Slot {
    kind: Kind,
    tiles: TileRect,
    hash: u64,
    /// [`quad_cost`].
    cost: u8,
}

#[derive(Clone, Copy, Debug, Default)]
struct List {
    mask: u64,
    at: u32,
}

#[derive(Clone, Copy, Debug)]
struct Pending {
    start: u32,
    wait: u32,
    deadline: u32,
    stats: FrameStats,
}

/// FNV-1a, 64 bits: whether a slot's words changed.
fn hash(words: &[u32], tiles: TileRect) -> u64 {
    let mut h = 0xCBF2_9CE4_8422_2325u64;
    let t = u32::from_le_bytes([tiles.x0, tiles.y0, tiles.x1, tiles.y1]);
    for w in words.iter().chain(core::iter::once(&t)) {
        for b in w.to_le_bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01B3);
        }
    }
    h
}

/// A quad's two objects: the words and their kind.
fn encode(d: &Draw) -> ([u32; SLOT_WORDS], usize, Kind) {
    let (isp, z) = match d.depth {
        Some(dp) => ((dp.mode & 7) << 29, fixed::f32_bits(i32::from(dp.z), 16)),
        None => (m3::ISP_ALWAYS, 0),
    };
    let state = |tsp: u32| {
        let tsp = match d.depth {
            Some(dp) => m3::depth_test(tsp, dp.mode, dp.write),
            None => tsp,
        };
        match d.blend {
            Some((s, t)) => m3::blend(tsp, s, t),
            None => tsp,
        }
    };
    let pos = |k: usize| {
        (
            f32::from_bits(fixed::f32_bits(d.corners[k].x, 4)),
            f32::from_bits(fixed::f32_bits(d.corners[k].y, 4)),
        )
    };
    let (z, rhw) = (f32::from_bits(z), f32::from_bits(fixed::ONE));
    // Top left, top right, bottom left; top right, bottom right, bottom
    // left (the proven squares' order, `rp-mbx3d` versions 21-23).
    const TRIS: [[usize; 3]; 2] = [[0, 1, 3], [1, 2, 3]];
    let mut w = [0u32; SLOT_WORDS];
    match d.paint {
        Paint::Flat(_) | Paint::Gouraud(_) => {
            let (colours, tsp) = match d.paint {
                Paint::Gouraud(c) => (c, m3::TSP_DELL | m3::TSP_GOURAUD),
                Paint::Flat(c) | Paint::Texture { tint: c, .. } => ([c; 4], m3::TSP_DELL),
            };
            let tsp = state(tsp);
            for (t, tri) in TRIS.iter().enumerate() {
                let v = tri.map(|k| {
                    let (x, y) = pos(k);
                    m3::Vertex {
                        x,
                        y,
                        z,
                        rhw,
                        argb: colours[k],
                    }
                });
                let at = t * m3::TRIANGLE_WORDS;
                w[at..at + m3::TRIANGLE_WORDS].copy_from_slice(&m3::triangle(isp, tsp, &v));
            }
            (w, 2 * m3::TRIANGLE_WORDS, Kind::Flat)
        }
        Paint::Texture { tex: t, uv, tint } => {
            let tsp = state(m3::ta::TSP_DELL_TEXTURED);
            for (n, tri) in TRIS.iter().enumerate() {
                let v = tri.map(|k| {
                    let (x, y) = pos(k);
                    tex::Vertex {
                        x,
                        y,
                        z,
                        rhw,
                        argb: tint,
                        u: f32::from_bits(fixed::f32_bits(uv[k].0, 16)),
                        v: f32::from_bits(fixed::f32_bits(uv[k].1, 16)),
                    }
                });
                let at = n * tex::WORDS;
                w[at..at + tex::WORDS].copy_from_slice(&tex::triangle(isp, t.words(), tsp, &v));
            }
            (w, 2 * tex::WORDS, Kind::Textured)
        }
    }
}

/// Texel `i` of a twiddled texture: its `(x, y)` (x in the odd bits).
#[must_use]
pub const fn untwiddle(i: u32) -> (u32, u32) {
    let (mut x, mut y, mut b) = (0, 0, 0);
    while b < 16 {
        y |= ((i >> (2 * b)) & 1) << b;
        x |= ((i >> (2 * b + 1)) & 1) << b;
        b += 1;
    }
    (x, y)
}

/// Uploads a square texture of side `8 << code` (`texels` in rows, 16 bits
/// each, `format` one of [`format`]) into a block from `pool`, twiddled.
/// Clamped at both edges (Dell's); see [`Texture::with_modes`].
pub fn upload<M: LocalMem>(
    pool: &mut Pool,
    mem: &mut M,
    code: u8,
    format: u32,
    texels: &[u16],
) -> Option<Texture> {
    if code > 7
        || !matches!(
            format,
            tex::ARGB1555 | tex::RGB565 | tex::ARGB4444 | tex::RGB332
        )
    {
        return None;
    }
    let t = Texture {
        at: 0,
        code,
        format: format as u8,
        u_mode: tex::CLAMP as u8,
        v_mode: tex::CLAMP as u8,
    };
    let side = t.side();
    if texels.len() != (side * side) as usize {
        return None;
    }
    let at = pool.alloc(t.bytes(), 128)?;
    let texel = |i: u32| {
        let (x, y) = untwiddle(i);
        u32::from(texels[(y * side + x) as usize])
    };
    let mut buf = [0u32; 32];
    let words = side * side / 2;
    let mut m = 0;
    while m < words {
        let n = (words - m).min(32);
        for (k, w) in buf[..n as usize].iter_mut().enumerate() {
            let i = 2 * (m + k as u32);
            *w = texel(i) | texel(i + 1) << 16;
        }
        mem.store((at + 4 * m) as usize, &buf[..n as usize]);
        m += n;
    }
    mem.sync();
    Some(Texture { at, ..t })
}

/// Gives a texture's block back to `pool`.
pub fn release(pool: &mut Pool, t: Texture) -> bool {
    pool.free(t.at, t.bytes())
}

/// The renderer: a retained scene of quads, rendered tile by tile into
/// targets of one size.
///
/// **One renderer drives the core at a time** (the graphics service owns
/// it, D6): its register shadow assumes nothing else writes the 3D
/// registers. A new renderer resets the core on its first render, so
/// renderers used one after another are safe; two used in turn are not.
/// Its resets reset the whole MBX: the 2D engine must be idle then, and its
/// stream restarts ([`Self::core_resets`]).
pub struct Renderer {
    params: Region,
    pm_table: Region,
    regions: u32,
    w: u16,
    h: u16,
    cols: u32,
    rows: u32,
    shadow: Shadow,
    gate: Gate,
    ready: bool,
    background: Option<u32>,
    bg_stale: bool,
    slots: [Slot; MAX_SLOTS],
    dirty: Tiles,
    /// Tiles changed since the other picture was last rendered.
    other: Tiles,
    /// The last two targets rendered into, newest first.
    targets: [Option<u32>; 2],
    lists: [List; LISTS],
    nlists: usize,
    last: usize,
    arena: u32,
    pending: Option<Pending>,
    mem_words: u32,
    resets: u32,
}

impl Renderer {
    /// A renderer for targets of `w` x `h` pixels (a multiple of the 8 x 16
    /// tile: a rendered tile is written whole), its region array from
    /// `pool`, the parameter memory and its manager's block where the
    /// 2700G memory map puts them (ADR-0027).
    pub fn new(pool: &mut Pool, w: u16, h: u16) -> Option<Self> {
        Self::new_in(pool, w, h, rp_vram_map::PARAMS, rp_vram_map::PM_TABLE)
    }

    /// [`Self::new`] with the parameter memory and the manager's block
    /// given (tests; the memory map's are the ones the device proved).
    pub fn new_in(
        pool: &mut Pool,
        w: u16,
        h: u16,
        params: Region,
        pm_table: Region,
    ) -> Option<Self> {
        let (cols, rows) = (u32::from(w) / m3::TILE_W, u32::from(h) / m3::TILE_H);
        if w == 0
            || h == 0
            || !u32::from(w).is_multiple_of(m3::TILE_W)
            || !u32::from(h).is_multiple_of(m3::TILE_H)
            || cols > 128
            || rows > 128
            || (cols * rows) as usize > MAX_TILES
            || params.size < ARENA_AT + 4 * LIST_MAX as u32
            || params.at as usize + params.size as usize > m3::PM_REACH
        {
            return None;
        }
        let regions = pool.alloc(cols * rows * 8, 64)?;
        let mut dirty = Tiles::new();
        dirty.add_first((cols * rows) as usize);
        Some(Self {
            params,
            pm_table,
            regions,
            w,
            h,
            cols,
            rows,
            shadow: Shadow::new(),
            gate: Gate::new(CLOCK_IDLE_US),
            ready: false,
            background: Some(0xFF00_0000),
            bg_stale: true,
            slots: [Slot::default(); MAX_SLOTS],
            dirty,
            other: Tiles::new(),
            targets: [None; 2],
            lists: [List::default(); LISTS],
            nlists: 0,
            last: 0,
            arena: ARENA_AT,
            pending: None,
            mem_words: 0,
            resets: 0,
        })
    }

    /// Gives the region array back to `pool`.
    pub fn release(self, pool: &mut Pool) -> bool {
        pool.free(self.regions, self.cols * self.rows * 8)
    }

    /// The size of the targets it renders.
    #[must_use]
    pub const fn size(&self) -> (u16, u16) {
        (self.w, self.h)
    }

    /// Tiles across and down.
    #[must_use]
    pub const fn grid(&self) -> (u32, u32) {
        (self.cols, self.rows)
    }

    /// How many times it has reset the MBX core (set-up, the clock back on,
    /// recovery): when it changes, the 2D engine's stream restarts
    /// (`Engine::after_core_reset`).
    #[must_use]
    pub const fn core_resets(&self) -> u32 {
        self.resets
    }

    /// Whether a render is in flight.
    #[must_use]
    pub const fn busy(&self) -> bool {
        self.pending.is_some()
    }

    /// The colour drawn first in every tile (opaque, `0xFFRRGGBB`), or
    /// none: then the quads must cover every tile rendered (the core fills
    /// the rest white). Black by default.
    pub fn set_background(&mut self, argb: Option<u32>) -> Result<(), Error> {
        if self.pending.is_some() {
            return Err(Error::Busy);
        }
        if argb != self.background {
            if argb.is_some() != self.background.is_some() {
                self.forget_lists();
            }
            self.background = argb;
            self.bg_stale = true;
            self.invalidate_all();
        }
        Ok(())
    }

    /// Puts `draw` in `slot` (drawn in slot order: higher over lower), or
    /// empties it. Writes the quad's words only if they changed, and marks
    /// the tiles it touched before and after.
    pub fn set<M: LocalMem>(
        &mut self,
        mem: &mut M,
        slot: usize,
        draw: Option<&Draw>,
    ) -> Result<(), Error> {
        if self.pending.is_some() {
            return Err(Error::Busy);
        }
        let old = *self.slots.get(slot).ok_or(Error::BadSlot)?;
        if draw
            .and_then(|d| d.depth)
            .is_some_and(|d| !DEPTH_MODES.contains(&d.mode))
        {
            return Err(Error::Unsupported);
        }
        let new = match draw {
            None => Slot::default(),
            Some(d) => {
                let (words, n, kind) = encode(d);
                let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
                for p in d.corners {
                    (x0, y0) = (x0.min(p.x), y0.min(p.y));
                    (x1, y1) = (x1.max(p.x), y1.max(p.y));
                }
                // Floor and ceiling to whole pixels (1/16 units).
                let tiles = TileRect::of_pixels(
                    x0 >> 4,
                    y0 >> 4,
                    (x1 + SUB - 1) >> 4,
                    (y1 + SUB - 1) >> 4,
                    self.cols,
                    self.rows,
                );
                let s = Slot {
                    kind,
                    tiles,
                    hash: hash(&words[..n], tiles),
                    cost: quad_cost(kind == Kind::Textured, d.blend.is_some()) as u8,
                };
                if s.kind == old.kind && s.hash == old.hash {
                    return Ok(());
                }
                let at = self.params.at + SLOTS_AT + slot as u32 * SLOT_BYTES;
                mem.store(at as usize, &words[..n]);
                self.mem_words += n as u32;
                s
            }
        };
        if new.kind == Kind::Empty && old.kind == Kind::Empty {
            return Ok(());
        }
        if new.kind != old.kind {
            // Lists naming this slot have the wrong strip pointers now.
            self.forget_lists_with(1 << slot);
        }
        self.dirty.add_rect(old.tiles, self.cols);
        self.dirty.add_rect(new.tiles, self.cols);
        self.slots[slot] = new;
        Ok(())
    }

    /// Marks pixels `[x, x + w)` x `[y, y + h)` to be rendered again (after
    /// something else drew there).
    pub fn invalidate(&mut self, x: i32, y: i32, w: i32, h: i32) {
        let r = TileRect::of_pixels(x, y, x + w, y + h, self.cols, self.rows);
        self.dirty.add_rect(r, self.cols);
    }

    /// Marks every tile to be rendered again.
    pub fn invalidate_all(&mut self) {
        self.dirty.add_first((self.cols * self.rows) as usize);
    }

    /// Tiles the next render into the last target would cover.
    #[must_use]
    pub fn dirty(&self) -> u32 {
        self.dirty.count()
    }

    fn forget_lists(&mut self) {
        self.nlists = 0;
        self.last = 0;
        self.arena = ARENA_AT;
    }

    fn forget_lists_with(&mut self, bit: u64) {
        let mut k = 0;
        for i in 0..self.nlists {
            if self.lists[i].mask & bit == 0 {
                self.lists[k] = self.lists[i];
                k += 1;
            }
        }
        self.nlists = k;
        self.last = 0;
    }

    /// Starts rendering the changed tiles into `target` (with the changes
    /// that picture missed while another was rendered: double buffering).
    /// Returns the time to [`Self::poll`] at, or `None` if nothing changed.
    pub fn render<B: RegisterBus, M: LocalMem, C: Clock>(
        &mut self,
        bus: &mut B,
        mem: &mut M,
        clock: &mut C,
        target: Target,
    ) -> Result<Option<u32>, Error> {
        if self.pending.is_some() {
            return Err(Error::Busy);
        }
        let pic = target.picture;
        if u32::from(target.x) + u32::from(self.w) > u32::from(pic.w)
            || u32::from(target.y) + u32::from(self.h) > u32::from(pic.h)
            || pic.stride < u32::from(pic.w) * 4
            || !pic.stride.is_multiple_of(4)
            || !pic.at.is_multiple_of(4)
        {
            return Err(Error::BadTarget);
        }
        let addr = target.addr();
        let mut todo = self.dirty.clone();
        let same = self.targets[0] == Some(addr);
        if !same {
            if self.targets[1] == Some(addr) {
                todo.add_all(&self.other);
            } else {
                todo.add_first((self.cols * self.rows) as usize);
            }
        }
        if todo.is_empty() {
            return Ok(None);
        }

        let now = clock.now_us();
        if self.gate.work(bus, now) {
            // Until device test T9 says the core keeps its set-up across
            // the clock off, set it up again.
            clock.delay_us(1_000);
            self.ready = false;
        }
        if !self.ready {
            self.bring_up(bus, mem, clock)?;
        }
        if self.bg_stale {
            self.write_background(mem);
        }
        if self.arena - ARENA_AT > (self.params.size - ARENA_AT) / 2 {
            // Lists are referenced only by the frame being built: reclaim
            // them all between frames before space runs short.
            self.forget_lists();
        }
        let writes0 = self.shadow.writes;
        let (tiles, quads, cost, lists) = match self.write_regions(mem, &todo) {
            Some(r) => r,
            None => {
                self.forget_lists();
                self.write_regions(mem, &todo).ok_or(Error::TooComplex)?
            }
        };
        mem.sync();

        let t = m3::start_table(
            self.regions,
            self.params.at,
            u32::from(pic.w),
            u32::from(pic.h),
            pic.stride,
        );
        for &(off, v) in &t[..14] {
            self.shadow.write(bus, off, v);
        }
        self.shadow.write(bus, 0x65C, addr);
        self.shadow.write(bus, m3::START, 1);

        let predicted = predict_us(tiles, cost);
        let start = clock.now_us();
        self.pending = Some(Pending {
            start,
            wait: predicted,
            // Generous: lists with many quads take longer a tile than
            // version 19's one object (to measure).
            deadline: predicted.saturating_mul(4).saturating_add(20_000),
            stats: FrameStats {
                tiles,
                quads,
                lists,
                mem_words: core::mem::take(&mut self.mem_words),
                reg_writes: self.shadow.writes - writes0,
                predicted_us: predicted,
                done_us: 0,
            },
        });
        if same {
            self.other.add_all(&self.dirty);
        } else {
            self.other = self.dirty.clone();
            self.targets = [Some(addr), self.targets[0]];
        }
        self.dirty.clear();
        Ok(Some(start.wrapping_add(predicted)))
    }

    /// Checks on the render in flight: reads the status once if its
    /// predicted time has come.
    pub fn poll<B: RegisterBus, M: LocalMem, C: Clock>(
        &mut self,
        bus: &mut B,
        mem: &mut M,
        clock: &mut C,
    ) -> Poll {
        let Some(mut p) = self.pending else {
            return Poll::Idle;
        };
        let now = clock.now_us();
        let gone = now.wrapping_sub(p.start);
        if gone < p.wait {
            return Poll::Wait(p.start.wrapping_add(p.wait));
        }
        if bus.read32(MBX + reg::STATUS) & m3::RENDER_DONE != 0 {
            bus.write32(MBX + reg::CLEAR, m3::RENDER_DONE);
            self.pending = None;
            p.stats.done_us = gone;
            return Poll::Done(p.stats);
        }
        if gone >= p.deadline {
            self.pending = None;
            self.ready = false;
            self.targets = [None; 2];
            self.invalidate_all();
            // Stop the hung core now; a failure here is retried by the
            // next render's set-up.
            let _ = self.bring_up(bus, mem, clock);
            return Poll::Recovered;
        }
        // Not yet: look again a sixteenth of the prediction later (a status
        // read is one bus read; a coarse step wastes the GPU's time).
        Poll::Wait(now.wrapping_add((p.stats.predicted_us / 16).max(20)))
    }

    /// When idle: the 3D clock off once [`CLOCK_IDLE_US`] has passed since
    /// the last render. True if it switched off now.
    pub fn idle<B: RegisterBus>(&mut self, bus: &mut B, now_us: u32) -> bool {
        self.pending.is_none() && self.gate.idle(bus, now_us)
    }

    /// When [`Self::idle`] wants to be called (for a tickless timer).
    #[must_use]
    pub fn idle_due(&self) -> Option<u32> {
        self.gate.due()
    }

    /// Resets the core and sets its parameter memory manager up (Dell's
    /// sequence, `rp-mbx3d` versions 3-23), render done enabled.
    fn bring_up<B: RegisterBus, M: LocalMem, C: Clock>(
        &mut self,
        bus: &mut B,
        mem: &mut M,
        clock: &mut C,
    ) -> Result<(), Error> {
        self.ready = false;
        reset_core(bus, |us| clock.delay_us(us));
        self.resets += 1;
        self.shadow.forget();
        let zeros = [0u32; 64];
        for k in 0..self.pm_table.size / 256 {
            mem.store((self.pm_table.at + 256 * k) as usize, &zeros);
        }
        self.mem_words += self.pm_table.size / 4;
        mem.sync();
        let en = bus.read32(MBX + reg::ENABLE);
        bus.write32(MBX + reg::ENABLE, en | m3::PM_READY | m3::RENDER_DONE);
        bus.write32(MBX + reg::CLEAR, m3::PM_READY | m3::RENDER_DONE);
        for (off, v) in m3::pm_init_table(self.params.at, self.params.size, self.pm_table.at) {
            self.shadow.write(bus, off, v);
        }
        let t0 = clock.now_us();
        let ok = loop {
            if bus.read32(MBX + reg::STATUS) & m3::PM_READY != 0 {
                break true;
            }
            if clock.now_us().wrapping_sub(t0) > 100_000 {
                break false;
            }
            clock.delay_us(20);
        };
        bus.write32(MBX + reg::CLEAR, m3::PM_READY);
        bus.write32(MBX + reg::ENABLE, en | m3::RENDER_DONE);
        self.ready = ok;
        if ok { Ok(()) } else { Err(Error::NoCore) }
    }

    fn write_background<M: LocalMem>(&mut self, mem: &mut M) {
        if let Some(c) = self.background {
            // One triangle reaching twice the target's size covers it.
            let (w, h) = (i32::from(self.w) * 2, i32::from(self.h) * 2);
            let f = |v: i32| f32::from_bits(fixed::f32_bits(v, 0));
            let v = [(0, 0), (w, 0), (0, h)].map(|(x, y)| m3::Vertex {
                x: f(x),
                y: f(y),
                z: f(0),
                rhw: f(1),
                argb: c,
            });
            // It writes depth (z 0, the farthest): a tile's depth isn't
            // cleared by `0x6DC = 0` (rp-mbx3d-api run 1: "greater" failed
            // where nothing had written depth; version 23's background
            // wrote it and passed).
            let tsp = m3::depth_test(m3::TSP_DELL, depth::ALWAYS, true);
            let words = m3::triangle(m3::ISP_ALWAYS, tsp, &v);
            mem.store((self.params.at + BG_AT) as usize, &words);
            self.mem_words += words.len() as u32;
        }
        self.bg_stale = false;
    }

    /// The region array listing `todo`, each tile pointing at the list of
    /// the quads touching it. Returns (tiles, quads listed, their
    /// [`quad_cost`]s, lists written), or `None` if the lists ran out of
    /// parameter memory.
    fn write_regions<M: LocalMem>(
        &mut self,
        mem: &mut M,
        todo: &Tiles,
    ) -> Option<(u32, u32, u32, u32)> {
        let mut buf = [0u32; 64];
        let (mut nb, mut flushed, mut n, mut lists) = (0usize, 0u32, 0u32, 0u32);
        let (mut quads, mut cost) = (0, 0);
        let (mut row, mut row_mask) = (u32::MAX, 0u64);
        for k in todo.iter() {
            let (x, y) = (k as u32 % self.cols, k as u32 / self.cols);
            if y >= self.rows {
                break;
            }
            if y != row {
                row = y;
                row_mask = 0;
                for (s, slot) in self.slots.iter().enumerate() {
                    let t = slot.tiles;
                    if slot.kind != Kind::Empty
                        && !t.is_empty()
                        && (u32::from(t.y0)..u32::from(t.y1)).contains(&y)
                    {
                        row_mask |= 1 << s;
                    }
                }
            }
            let mut mask = 0u64;
            let mut m = row_mask;
            while m != 0 {
                let s = m.trailing_zeros() as usize;
                m &= m - 1;
                if self.slots[s].tiles.contains(x, y) {
                    mask |= 1 << s;
                }
            }
            quads += mask.count_ones();
            let mut m = mask;
            while m != 0 {
                cost += u32::from(self.slots[m.trailing_zeros() as usize].cost);
                m &= m - 1;
            }
            let (at, new) = self.list_for(mem, mask)?;
            lists += u32::from(new);
            if nb == buf.len() {
                mem.store((self.regions + 4 * flushed) as usize, &buf);
                flushed += nb as u32;
                nb = 0;
            }
            let e = m3::region_entry(x, y, self.params.at + at, false);
            buf[nb] = e[0];
            buf[nb + 1] = e[1];
            nb += 2;
            n += 1;
        }
        if n > 0 {
            buf[nb - 2] |= m3::LAST_REGION;
            mem.store((self.regions + 4 * flushed) as usize, &buf[..nb]);
        }
        self.mem_words += 2 * n;
        Some((n, quads, cost, lists))
    }

    /// The list of the quads in `mask` (after the background), in the
    /// parameter memory: reused if written before. Returns its offset and
    /// whether it was written now.
    fn list_for<M: LocalMem>(&mut self, mem: &mut M, mask: u64) -> Option<(u32, bool)> {
        // Neighbouring tiles mostly share their quads: try the last first.
        if self.last < self.nlists && self.lists[self.last].mask == mask {
            return Some((self.lists[self.last].at, false));
        }
        if let Some(i) = self.lists[..self.nlists]
            .iter()
            .position(|l| l.mask == mask)
        {
            self.last = i;
            return Some((self.lists[i].at, false));
        }
        let mut w = [m3::END; LIST_MAX];
        let mut n = 0;
        if self.background.is_some() {
            w[0] = m3::STRIP_ONE_TRIANGLE | (BG_AT / 4);
            n = 1;
        }
        let mut m = mask;
        while m != 0 {
            let s = m.trailing_zeros();
            m &= m - 1;
            let (flags, words) = self.slots[s as usize].kind.strip();
            let base = (SLOTS_AT + s * SLOT_BYTES) / 4;
            w[n] = flags | base;
            w[n + 1] = flags | (base + words);
            n += 2;
        }
        n += 1; // the end word
        let at = self.arena;
        if at + 4 * n as u32 > self.params.size {
            return None;
        }
        mem.store((self.params.at + at) as usize, &w[..n]);
        self.arena += 4 * n as u32;
        self.mem_words += n as u32;
        if self.nlists < LISTS {
            self.lists[self.nlists] = List { mask, at };
            self.last = self.nlists;
            self.nlists += 1;
        }
        Some((at, true))
    }
}

#[cfg(test)]
mod tests;
