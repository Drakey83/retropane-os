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
//! - **A quad as one object** on request ([`Objects::Quads`], R78 item
//!   3): a four-corner object in place of two triangles, a third fewer
//!   words and one list word a tile instead of two.
//! - **A cheap set-up a tile** (R88): no division in the tile walk, each
//!   tile's list from a hashed cache with its count and cost, and a whole
//!   frame again keeps the region entries whose quads didn't change
//!   (`rp-bench`'s `r3d_*`: 884 -> 353 instructions a tile).
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

pub mod binner;
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
    pub use rp_drv_mbx3d::tex::{ARGB1555, ARGB4444, ARGB8888, PVRTC2, PVRTC4, RGB332, RGB565};
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

/// What the renderer redoes when the 3D clock comes back on (ADR-0026 D8).
/// Device test T9 (`rp-mbx3d-api` run 4, S9): the core keeps its registers
/// and the memory manager's set-up across `MBXCLK` 2 -> 1 -> 2 (1 ms and
/// 300 ms off), so nothing is redone by default: a clock-on frame's render
/// call took ~1.0 ms instead of ~5.6 ms with the set-up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ClockPolicy {
    /// A core reset and the memory manager's set-up (~4.6 ms).
    SetUpAgain,
    /// Every state register written again on the next frame.
    RewriteRegisters,
    /// Nothing: the core keeps its state (T9, measured).
    #[default]
    KeepAll,
}

/// `0x130` bit 15: the MBX core's interrupts on (Dell's `0x87F5`; version
/// 24: without it render done never reached GPIO85).
pub const IRQ_MASTER: u32 = 1 << 15;
/// The 2700G's system interrupt enable (write-only: never read).
pub const MINTENABLE: usize = 0x018;
/// `MINTENABLE` as Dell writes it: bit 15 the master, bit 2 routes the MBX
/// core (version 24: 0x8004 and 0x800F edged GPIO85, 0x8001/2/8 and 0x000F
/// didn't), bits 0, 1, 3 the display's.
pub const MINTENABLE_DELL: u32 = 0x800F;

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
    /// Bilinear filtering (TEX1 bit 31, `rp-mbx3d` version 24), or point
    /// sampling.
    pub linear: bool,
    /// Mip levels, the base included (1: none). Each level follows the
    /// larger one, the base first (version 25).
    pub levels: u8,
}

impl Texture {
    /// Texels a side.
    #[must_use]
    pub const fn side(&self) -> u32 {
        8 << self.code
    }

    /// Bytes it takes, its mip levels included.
    #[must_use]
    pub const fn bytes(&self) -> u32 {
        let mut total = 0;
        let mut k = 0;
        let levels = if self.levels == 0 {
            1
        } else {
            self.levels as u32
        };
        while k < levels {
            let side = self.side() >> k;
            let texels = side * side;
            total += match self.format as u32 {
                tex::ARGB8888 => texels * 4,
                tex::PVRTC4 => texels / 2,
                tex::PVRTC2 => texels / 4,
                _ => texels * 2,
            };
            k += 1;
        }
        // Whole words: a 1 x 1 level is half a word.
        (total + 3) & !3
    }

    /// The same texture filtered bilinearly (`true`) or point sampled.
    #[must_use]
    pub const fn with_linear(self, linear: bool) -> Self {
        Self { linear, ..self }
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

    /// TEX0, TEX1 and TEX2. A mipmapped texture: TEX0 bits 12-9 its
    /// highest level index (version 25: 0 drew only the base, 3-11 the
    /// right level), TEX1 the nearest level (bit 29; bit 30 hung the core,
    /// bilinear with levels untested: point within the level).
    #[must_use]
    pub fn words(&self) -> [u32; 3] {
        let mut tex0 = tex::with_address(
            tex::tex0(self.code as u32, self.format as u32),
            self.u_mode as u32,
            self.v_mode as u32,
        );
        let filter = if self.levels > 1 {
            tex0 |= (u32::from(self.levels) - 1) << 9;
            tex::TEX1_MIP_NEAREST
        } else if self.linear {
            tex::TEX1_LINEAR
        } else {
            0
        };
        let tex1 = tex::tex1(self.at, tex::TEX1_LOD, filter);
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

/// A depth test: the quad's z against the depth stored at each pixel, by
/// `mode` ([`depth`], GL order: "less" passes a smaller z, "greater" a
/// larger). All eight modes behave on the device (`rp-glesfeat` D,
/// 2026-10-18).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Depth {
    /// One of [`depth`].
    pub mode: u32,
    /// Whether passing pixels store their depth.
    pub write: bool,
    /// The quad's depth, `z / 65536` (each tile starts at 0).
    pub z: u16,
}

/// An alpha test: the quad's pixels are drawn only where their alpha
/// (after the texture) compares true against `reference` by `func` (GL
/// order, as [`depth`]). Cut-out shapes: leaves, fences, icons
/// (`rp_drv_mbx3d::alpha_test`, confirmed on the device 2026-10-18).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AlphaTest {
    /// One of [`depth`]'s modes.
    pub func: u32,
    /// 0-255.
    pub reference: u8,
}

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
    /// An alpha test, or none (every pixel drawn).
    pub alpha: Option<AlphaTest>,
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
            alpha: None,
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

    /// With an alpha test.
    #[must_use]
    pub const fn with_alpha_test(self, a: AlphaTest) -> Self {
        Self {
            alpha: Some(a),
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
    /// Its pixels.
    pub format: PixelFormat,
}

/// The pixels a target holds (`0x650`, `rp-mbx3d` version 26: tiles stay
/// 8 x 16 in every format, `0x660` the stride in bytes / 4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    /// 32-bit `0xAARRGGBB` (`0x650` 6: Dell's).
    Argb8888,
    /// 16-bit RGB565 (`0x650` 1: the panel's own depth, half the bytes to
    /// write and keep).
    Rgb565,
    /// 16-bit ARGB1555 (`0x650` 3; opaque pixels with the alpha bit set).
    Argb1555,
    /// 16-bit RGB565 with the core's 4 x 4 ordered dither (`0x650` 9:
    /// `rp-gpubench` run P2, X51v 2026-10-21): ADR-0023's screens.
    Rgb565Dithered,
}

impl PixelFormat {
    /// `0x650`.
    #[must_use]
    pub const fn code(self) -> u32 {
        match self {
            Self::Argb8888 => 6,
            Self::Rgb565 => 1,
            Self::Argb1555 => 3,
            Self::Rgb565Dithered => 9,
        }
    }

    /// Bytes a pixel.
    #[must_use]
    pub const fn bytes(self) -> u32 {
        match self {
            Self::Argb8888 => 4,
            _ => 2,
        }
    }
}

impl Picture {
    /// ADR-0023's screen picture `n` (0 or 1), from the memory map:
    /// RGB565, rendered with the core's dither (the owner's 2026-10-21
    /// vote: 32-bit renders are exact only under a 16-bit screen).
    #[must_use]
    pub const fn screen(n: usize) -> Self {
        Self {
            at: rp_vram_map::SCREENS[n & 1].at,
            stride: 480 * 2,
            w: 480,
            h: 640,
            format: PixelFormat::Rgb565Dithered,
        }
    }

    /// The test images' 32-bit picture `n` (0 or 1), at the frozen map's
    /// screens (`rp_vram_map::bench32`): what their device runs measured.
    #[must_use]
    pub const fn bench32(n: usize) -> Self {
        Self {
            at: rp_vram_map::bench32::SCREENS[n & 1].at,
            stride: 480 * 4,
            w: 480,
            h: 640,
            format: PixelFormat::Argb8888,
        }
    }

    /// A `w` x `h` picture of `format` at `at`, its lines packed.
    #[must_use]
    pub const fn packed(at: u32, w: u16, h: u16, format: PixelFormat) -> Self {
        Self {
            at,
            stride: w as u32 * format.bytes(),
            w,
            h,
            format,
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
        self.picture.at
            + self.y as u32 * self.picture.stride
            + self.x as u32 * self.picture.format.bytes()
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
    /// A depth or alpha-test mode that isn't one of [`depth`]'s eight.
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
/// (quality audit C20, refitted 2026-10-24): `texels16` is a textured
/// quad's texels a pixel x 16 ([`texels16`]), `None` for flat or Gouraud.
/// Measured on the X51v at Windows Mobile's clock into a 16-bit target
/// (`rp-gpubench` run LIM, 2026-10-24): an opaque flat quad almost nothing
/// (63 covering 2,016 tiles +37 us; `rp-mbx3d-api` run 2's small flat
/// quads ~0.3-0.45 us: 0.25 the compromise); a textured one by how many
/// texels a pixel it reads (a whole texture over the screen: 1.3, 2.1,
/// 3.3, 5.3, 7.4 us more a tile at 0.13, 0.27, 0.53, 1.06, 2.13 texels a
/// pixel; 63 textured 64 x 64 quads at 1:1 5.6 us a tile; run 2's 8x
/// magnified 0.75); a translucent flat layer 5.18 us (8 to 32 layers);
/// translucent and textured about 3.3 us more than textured (G2-G3: 8-9
/// us at 1:1).
#[must_use]
pub const fn quad_cost(texels16: Option<u32>, blended: bool) -> u32 {
    let textured = match texels16 {
        None => 0,
        Some(r) if r < 3 => 5,
        Some(r) if r < 6 => 8,
        Some(r) if r < 12 => 13,
        Some(r) if r < 24 => 21,
        Some(r) if r < 48 => 30,
        Some(_) => 38,
    };
    match (blended, texels16.is_some()) {
        (false, false) => 1,
        (false, true) => textured,
        (true, false) => 21,
        (true, true) => textured + 13,
    }
}

/// A draw's texels a pixel x 16 along its more-sampled axis, for
/// [`quad_cost`]: `None` unless it is textured with a size.
#[must_use]
pub fn texels16(d: &Draw) -> Option<u32> {
    let Paint::Texture { tex, uv, .. } = d.paint else {
        return None;
    };
    let span = |f: &dyn Fn(usize) -> i64| {
        let (lo, hi) = (0..4).fold((i64::MAX, i64::MIN), |(lo, hi), k| {
            (lo.min(f(k)), hi.max(f(k)))
        });
        hi - lo
    };
    let side = i64::from(tex.side());
    // Texels over UV_ONE, pixels over SUB, the ratio times 16.
    let r = |axis: usize| {
        let texels = span(&|k| i64::from(if axis == 0 { uv[k].0 } else { uv[k].1 })) * side;
        let pixels = span(&|k| {
            i64::from(if axis == 0 {
                d.corners[k].x
            } else {
                d.corners[k].y
            })
        });
        (pixels > 0).then(|| texels * 16 * i64::from(SUB) / (i64::from(UV_ONE) * pixels))
    };
    let (a, b) = (r(0)?, r(1)?);
    u32::try_from(a.max(b)).ok()
}

/// How a quad goes to the core (R78 item 3).
///
/// Dell's GLES driver sends a quad as a strip of four and its tile
/// accelerator writes one four-corner object (`rp-glesfeat` test G); its
/// display driver writes its rectangles as four-corner objects directly
/// (`ddi_2700g.dll` `0x02117d9c`); the textbook (and our first renderer)
/// splits every quad into two triangles. Ours: one four-corner object
/// written straight into the parameter memory (no tile accelerator), a
/// third fewer words a quad (22 for 34 flat, 33 for 52 textured) and half
/// the list words in every tile it touches.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Objects {
    /// Two one-triangle objects a quad (the form every device run up to
    /// 2026-10-28 drew).
    #[default]
    Triangles,
    /// One four-corner object a quad: flat and Gouraud drew exactly as two
    /// triangles on the X51v (test G2); textured (layout 9, Dell's
    /// rectangle) not yet drawn by our code on the device.
    Quads,
}

/// Where things sit in the parameter memory, in bytes from its start (the
/// start table's `0x60C`; strip pointers count words from it).
const BG_AT: u32 = 64;
const SLOTS_AT: u32 = 256;
/// A slot: two textured one-triangle objects (2 x 26 words) fit.
const SLOT_BYTES: u32 = 256;
const ARENA_AT: u32 = SLOTS_AT + MAX_SLOTS as u32 * SLOT_BYTES;
/// Object lists kept for reuse: more than a frame's distinct sets of quads
/// (GC2CP's game frames: 123-142 at 480 x 640 and 480 x 320, counted from
/// their meshes; the depth frame's 14 past 128 were written again on each
/// of their tiles, 336 lists and 7,218 words a frame on the X51v). Past it,
/// a list is evicted in turn (its words stay where they are).
const LISTS: usize = 192;
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
    /// One four-corner object ([`Objects::Quads`]).
    FlatQuad,
    /// See [`Self::FlatQuad`].
    TexturedQuad,
}

impl Kind {
    /// The strip pointer flags, the words of one object and the objects a
    /// quad takes.
    const fn strip(self) -> (u32, u32, u32) {
        match self {
            Self::Textured => (tex::STRIP_GAME, tex::WORDS as u32, 2),
            Self::FlatQuad => (m3::STRIP_QUAD, m3::QUAD_WORDS as u32, 1),
            Self::TexturedQuad => (tex::STRIP_QUAD, tex::QUAD_WORDS as u32, 1),
            _ => (m3::STRIP_ONE_TRIANGLE, m3::TRIANGLE_WORDS as u32, 2),
        }
    }

    /// Whether its objects carry the texture words (the header `ISP,
    /// TEX0-2, TSP`).
    const fn textured(self) -> bool {
        matches!(self, Self::Textured | Self::TexturedQuad)
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
    /// Its quads' [`quad_cost`]s summed (kept as slots change), and how
    /// many: each tile listing it adds these, no loop over its slots.
    cost: u16,
    quads: u8,
    /// The next list in its bucket ([`NO_LIST`] ends it).
    next: u8,
}

/// No list (a bucket's or a chain's end).
const NO_LIST: u8 = u8::MAX;
/// Buckets of the list cache: a tile's list found in a step or two, not a
/// scan of up to [`LISTS`] masks (R88).
const BUCKETS: usize = 64;
const _: () = assert!(LISTS < NO_LIST as usize && MAX_SLOTS <= 64 && MAX_SLOTS * 255 <= 0xFFFF);

/// A mask's bucket: its two halves folded and hashed (Fibonacci).
const fn bucket(mask: u64) -> usize {
    let x = (mask as u32) ^ (mask >> 32) as u32;
    (x.wrapping_mul(0x9E37_79B1) >> 26) as usize
}

/// The learned pace's range, x 256: a render a sixteenth to four times
/// [`predict_us`]'s time (the hang deadline stays on the prediction).
const PACE_MIN: u32 = 16;
const PACE_MAX: u32 = 1024;
/// The shortest step between status reads, us.
const PACE_STEP_MIN: u32 = 8;

/// A render [`Renderer::prepare`] wrote and hasn't started.
#[derive(Clone, Copy, Debug)]
struct Staged {
    pic: Picture,
    addr: u32,
    same: bool,
    tiles: u32,
    quads: u32,
    cost: u32,
    lists: u32,
}

#[derive(Clone, Copy, Debug)]
struct Pending {
    start: u32,
    wait: u32,
    /// The time between status reads after the first.
    step: u32,
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

/// The tiles of a `cols` x `rows` grid a quad's bounding box touches, its
/// corners floored and ceiled to whole pixels.
fn draw_tiles(d: &Draw, cols: u32, rows: u32) -> TileRect {
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for p in d.corners {
        (x0, y0) = (x0.min(p.x), y0.min(p.y));
        (x1, y1) = (x1.max(p.x), y1.max(p.y));
    }
    // Floor and ceiling to whole pixels (1/16 units).
    TileRect::of_pixels(
        x0 >> 4,
        y0 >> 4,
        (x1 + SUB - 1) >> 4,
        (y1 + SUB - 1) >> 4,
        cols,
        rows,
    )
}

/// A quad's objects (two triangles, or one four-corner object): the words
/// and their kind.
fn encode(d: &Draw, objects: Objects, flip: bool, rot: usize) -> ([u32; SLOT_WORDS], usize, Kind) {
    // The second word is the alpha test only; the depth mode goes in the
    // TSP (writing it here too made "less" test "alpha < 0": ADR-0026).
    let isp = match d.alpha {
        Some(a) => m3::alpha_test(a.func, u32::from(a.reference)),
        None => m3::ISP_ALWAYS,
    };
    let z = d.depth.map_or(0, |dp| fixed::f32_bits(i32::from(dp.z), 16));
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
    // left (the proven squares' order, `rp-mbx3d` versions 21-23); or,
    // flipped, the other diagonal: top left, top right, bottom right; top
    // left, bottom right, bottom left (the same winding).
    let tris: [[usize; 3]; 2] = if flip {
        [[0, 1, 2], [0, 2, 3]]
    } else {
        [[0, 1, 3], [1, 2, 3]]
    };
    // The same two triangles as one strip of four (`v0 v1 v2`, then
    // `v1 v3 v2`, the winding kept): top left, top right, bottom left,
    // bottom right (test G2's order); flipped, top right, bottom right, top
    // left, bottom left. The vertex rotation is the triangles' alone (a
    // measurement's).
    let strip: [usize; 4] = if flip { [1, 2, 0, 3] } else { [0, 1, 3, 2] };
    let mut w = [0u32; SLOT_WORDS];
    if objects == Objects::Quads {
        return match d.paint {
            Paint::Flat(_) | Paint::Gouraud(_) => {
                let (colours, tsp) = match d.paint {
                    Paint::Gouraud(c) => (c, m3::TSP_DELL | m3::TSP_GOURAUD),
                    Paint::Flat(c) | Paint::Texture { tint: c, .. } => ([c; 4], m3::TSP_DELL),
                };
                let v = strip.map(|k| {
                    let (x, y) = pos(k);
                    m3::Vertex {
                        x,
                        y,
                        z,
                        rhw,
                        argb: colours[k],
                    }
                });
                w[..m3::QUAD_WORDS].copy_from_slice(&m3::quad(isp, state(tsp), &v));
                (w, m3::QUAD_WORDS, Kind::FlatQuad)
            }
            Paint::Texture { tex: t, uv, tint } => {
                let tsp = state(m3::ta::TSP_DELL_TEXTURED);
                let v = strip.map(|k| {
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
                w[..tex::QUAD_WORDS].copy_from_slice(&tex::quad(isp, t.words(), tsp, &v));
                (w, tex::QUAD_WORDS, Kind::TexturedQuad)
            }
        };
    }
    match d.paint {
        Paint::Flat(_) | Paint::Gouraud(_) => {
            let (colours, tsp) = match d.paint {
                Paint::Gouraud(c) => (c, m3::TSP_DELL | m3::TSP_GOURAUD),
                Paint::Flat(c) | Paint::Texture { tint: c, .. } => ([c; 4], m3::TSP_DELL),
            };
            let tsp = state(tsp);
            for (t, tri) in tris.iter().enumerate() {
                let mut tri = *tri;
                tri.rotate_left(rot % 3);
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
            for (n, tri) in tris.iter().enumerate() {
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

/// A texture of side `8 << code` in `format`, not yet placed.
const fn texture(code: u8, format: u32) -> Texture {
    Texture {
        at: 0,
        code,
        format: format as u8,
        u_mode: tex::CLAMP as u8,
        v_mode: tex::CLAMP as u8,
        linear: false,
        levels: 1,
    }
}

/// Uploads a square texture of side `8 << code` (`texels` in rows, 16 bits
/// each, `format` one of [`format`]'s 16-bit ones) into a block from
/// `pool`, twiddled. Clamped at both edges (Dell's), point sampled; see
/// [`Texture::with_modes`] and [`Texture::with_linear`].
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
    let t = texture(code, format);
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

/// The bits of a 16-bit texel's channels (alpha, red, green, blue), from
/// the bottom: RGB332 in the low byte.
const fn channel_bits(format: u32) -> [u32; 4] {
    match format {
        tex::RGB565 => [0, 5, 6, 5],
        tex::ARGB1555 => [1, 5, 5, 5],
        tex::ARGB4444 => [4, 4, 4, 4],
        _ => [0, 3, 3, 2],
    }
}

/// The average of four 16-bit texels, channel by channel (rounded).
fn average4(format: u32, t: [u16; 4]) -> u16 {
    let bits = channel_bits(format);
    let mut out = 0u32;
    let mut shift = 0;
    for &b in bits.iter().rev() {
        let mask = (1 << b) - 1;
        let sum: u32 = t.iter().map(|&v| (u32::from(v) >> shift) & mask).sum();
        out |= ((sum + 2) / 4) << shift;
        shift += b;
    }
    out as u16
}

/// Texel `(x, y)` of mip level `k` of the `side`-wide square `base`: the
/// base itself at level 0, else the [`average4`] of the four level `k - 1`
/// texels under it (the cascade a level-by-level build computes, exactly).
fn cascade(format: u32, base: &[u16], side: u32, k: u32, x: u32, y: u32) -> u16 {
    if k == 0 {
        return base[(y * side + x) as usize];
    }
    let t = |dx: u32, dy: u32| cascade(format, base, side, k - 1, 2 * x + dx, 2 * y + dy);
    average4(format, [t(0, 0), t(1, 0), t(0, 1), t(1, 1)])
}

/// Uploads a square texture of 16-bit texels with its mip levels down to
/// 1 x 1, each made from the larger one by averaging 2 x 2 texels (on the
/// CPU, integer only): the base first, each level after the larger one,
/// each twiddled on its own (`rp-mbx3d` version 25: the 16 and 8 px quads
/// of a 64 x 64 texture drew the 16 and 8 levels). A third more memory
/// than the base alone; scaled-down pictures stop shimmering.
pub fn upload_mipmapped<M: LocalMem>(
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
    let side = 8u32 << code;
    if texels.len() != (side * side) as usize {
        return None;
    }
    let levels = code + 4; // 8 << code down to 1
    let t = Texture {
        levels,
        ..texture(code, format)
    };
    let at = pool.alloc(t.bytes(), 128)?;
    // Each level made from the base texels in `texels` (SDRAM, cached), by
    // the same 2 x 2 cascade a level-by-level build does: a level-k texel
    // is the average of four level k-1 texels, each made the same way
    // down to the base. No read of 2700G memory (the bus's slowest
    // direction, 12 MB/s: the performance gate's T6 found the old build
    // reading each level back, ~64 K words for a 256 x 256 texture), and
    // no level-sized buffer (the stack). A level-k pass reads the base
    // once over: levels x side^2 SDRAM reads in all.
    let mut offset = 0u32;
    for k in 0..u32::from(levels) {
        let s = side >> k;
        let texel = |x: u32, y: u32| -> u16 { cascade(format, texels, side, k, x, y) };
        if s == 1 {
            // One texel: half a word (the next half is padding).
            let w = u32::from(texel(0, 0));
            mem.store((at + offset) as usize, &[w]);
        } else {
            let mut buf = [0u32; 32];
            let words = s * s / 2;
            let mut m = 0;
            while m < words {
                let n = (words - m).min(32);
                for (j, w) in buf[..n as usize].iter_mut().enumerate() {
                    let i = 2 * (m + j as u32);
                    let (x0, y0) = untwiddle(i);
                    let (x1, y1) = untwiddle(i + 1);
                    *w = u32::from(texel(x0, y0)) | u32::from(texel(x1, y1)) << 16;
                }
                mem.store((at + offset + 4 * m) as usize, &buf[..n as usize]);
                m += n;
            }
        }
        mem.sync();
        offset += s * s * 2;
    }
    Some(Texture { at, ..t })
}

/// Uploads a square texture of 32-bit texels, `0xAARRGGBB` (TEX0 code 12:
/// `rp-mbx3d` version 24 decoded red, green and blue exactly; its alpha in
/// blending is untested, so it's drawn opaque), in rows, twiddled.
pub fn upload32<M: LocalMem>(
    pool: &mut Pool,
    mem: &mut M,
    code: u8,
    texels: &[u32],
) -> Option<Texture> {
    let t = texture(code, tex::ARGB8888);
    let side = t.side();
    if code > 7 || texels.len() != (side * side) as usize {
        return None;
    }
    let at = pool.alloc(t.bytes(), 128)?;
    let mut buf = [0u32; 32];
    let words = side * side;
    let mut m = 0;
    while m < words {
        let n = (words - m).min(32);
        for (k, w) in buf[..n as usize].iter_mut().enumerate() {
            let (x, y) = untwiddle(m + k as u32);
            *w = texels[(y * side + x) as usize];
        }
        mem.store((at + 4 * m) as usize, &buf[..n as usize]);
        m += n;
    }
    mem.sync();
    Some(Texture { at, ..t })
}

/// Uploads a PVRTC texture (`format` [`format::PVRTC4`] or
/// [`format::PVRTC2`]) of side `8 << code` as compressed on the PC: its
/// 64-bit blocks as pairs of words, the modulation word first (version 24:
/// the other way round drew black), in the order the encoder writes them
/// (PVRTC's twiddled block order; the device test couldn't tell it from
/// row order). 4 or 2 bits a texel: a 256 x 256 picture in 32 or 16 KiB
/// instead of 128.
pub fn upload_pvrtc<M: LocalMem>(
    pool: &mut Pool,
    mem: &mut M,
    code: u8,
    format: u32,
    blocks: &[u32],
) -> Option<Texture> {
    if code > 7 || !matches!(format, tex::PVRTC4 | tex::PVRTC2) {
        return None;
    }
    let t = texture(code, format);
    if blocks.len() * 4 != t.bytes() as usize {
        return None;
    }
    let at = pool.alloc(t.bytes(), 128)?;
    mem.store(at as usize, blocks);
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
    /// The list cache's buckets: each its first list, chained by
    /// [`List::next`].
    buckets: [u8; BUCKETS],
    /// The cache's next list to give up when it's full.
    hand: u8,
    /// Tiles whose quads changed since the region array was last written
    /// (a set's tiles before and after; not [`Self::invalidate`]'s, whose
    /// quads are the same): only they can need another list.
    lists_changed: Tiles,
    /// The order of the last region array that listed every tile, while
    /// its entries still hold (no list forgotten since): a whole frame
    /// again in that order writes only the entries of [`Self::lists_changed`]
    /// tiles (R88: GC2CP's full frame wrote 4,800 region words a frame for
    /// 18 new list words).
    regions_full: Option<TileOrder>,
    arena: u32,
    pending: Option<Pending>,
    /// What [`Self::prepare`] wrote, waiting for [`Self::start_prepared`].
    staged: Option<Staged>,
    mem_words: u32,
    resets: u32,
    /// A render's time against [`predict_us`]'s, x 256, learned from the
    /// ends seen ([`Self::learn_pace`]).
    pace: u32,
    clock_policy: ClockPolicy,
    /// Render done raises the 2700G's interrupt (GPIO85).
    irq: bool,
    /// `irq` changed since the enables were last written.
    irq_stale: bool,
    /// The order the region array lists the tiles in (the core draws them
    /// in the array's order): a measurement's choice (`rp-gpubench` runs
    /// GC2L, GC2O), row order otherwise.
    tile_order: TileOrder,
    /// A measurement's bit flip in every textured object's header (run
    /// GC2AP; several words at once, run GC2CH): the bits XORed into each
    /// header word (0 ISP, 1-3 TEX0-2, 4 TSP); zeros never change anything.
    header_xor: [u32; 5],
    /// A measurement's bits ORed into every object's TSP word, flat and
    /// textured (run GC2AQ: TSP bit 14, the punch-through object kind, and
    /// bit 15); 0 changes nothing.
    tsp_or: u32,
    /// Every quad split along its other diagonal (`rp-gpubench` run
    /// GC2BU: the tile race's dropped groups sit on the diagonal).
    diagonal_flip: bool,
    /// Each flat triangle's vertices fed this many places rotated (the same
    /// triangle and winding; `rp-gpubench` run GC2BX: the set-up's
    /// arithmetic starts from another corner).
    vertex_rotate: usize,
    /// A measurement's mask XORed into word `.0` (0 ISP, 1 TSP) of every
    /// flat triangle (`rp-gpubench` run GC2BX's bit search).
    flat_xor: (usize, u32),
    /// How each quad goes to the core ([`Objects`]).
    objects: Objects,
}

/// The order the tiles are listed for the core in (the picture is the same
/// in each; when each tile is written differs).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TileOrder {
    /// Row by row from the top, each row from the left (the default).
    #[default]
    Rows,
    /// The last tile first: row by row from the bottom, each from the right.
    Reversed,
    /// Row by row from the top, each row from the right.
    ColumnsReversed,
    /// Column by column from the right, each from the bottom: tiles drawn
    /// one after another sit a tile row (16 lines of the target) apart in
    /// memory, not side by side in the same lines (`rp-gpubench` run GC2AN:
    /// does the tile race at divide 1 need neighbours in memory?).
    Columns,
    /// A checkerboard: every tile whose x + y is even, from the last, then
    /// every odd one, from the last: no tile follows its neighbour (run
    /// GC2AN).
    Checker,
    /// Every tile once, in a fixed scrambled order (the region array's
    /// index stepped by [`SCRAMBLE`], a prime no target's tile count is a
    /// multiple of): consecutive tiles far apart and unrelated (run GC2AN).
    Scrambled,
    /// Row by row from the top, each row's even tiles first, then its odd
    /// ones: no tile follows its side-by-side neighbour (GC2AN: the tile
    /// race at divide 1 needs exactly that), and the picture still fills
    /// from the top down, as the display scans it out (R20's tear-free
    /// design), where the checkerboard and scrambled orders jump about
    /// (run GC2AO).
    Interleaved,
}

/// [`TileOrder::Scrambled`]'s step: a prime above 128 (so it divides no
/// count of at most 128 x 128 tiles) and far from any row length.
pub const SCRAMBLE: usize = 1009;

/// The `i`th tile number (of `cols` x `rows`) [`TileOrder::Columns`],
/// [`TileOrder::Checker`] or [`TileOrder::Scrambled`] lists, for `i` in `0..2
/// * cols * rows` (the checkerboard's two passes; the others use the first
/// half); `None` where the order lists nothing at `i`.
#[must_use]
pub const fn order_tile(order: TileOrder, i: usize, cols: usize, rows: usize) -> Option<usize> {
    let n = cols * rows;
    match order {
        TileOrder::Columns if i < n => {
            let (x, y) = (cols - 1 - i / rows, rows - 1 - i % rows);
            Some(y * cols + x)
        }
        TileOrder::Checker => {
            let k = n - 1 - i % n;
            if (k % cols + k / cols) % 2 == i / n {
                Some(k)
            } else {
                None
            }
        }
        TileOrder::Scrambled if i < n => Some((n - 1 - i) * SCRAMBLE % n),
        TileOrder::Interleaved if i < n => {
            let (y, j, half) = (i / cols, i % cols, cols.div_ceil(2));
            let x = if j < half { 2 * j } else { 2 * (j - half) + 1 };
            Some(y * cols + x)
        }
        _ => None,
    }
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
            buckets: [NO_LIST; BUCKETS],
            hand: 0,
            lists_changed: Tiles::new(),
            regions_full: None,
            arena: ARENA_AT,
            pending: None,
            staged: None,
            mem_words: 0,
            resets: 0,
            pace: 256,
            clock_policy: ClockPolicy::KeepAll,
            irq: false,
            irq_stale: false,
            tile_order: TileOrder::Rows,
            header_xor: [0; 5],
            tsp_or: 0,
            diagonal_flip: false,
            vertex_rotate: 0,
            flat_xor: (0, 0),
            objects: Objects::Triangles,
        })
    }

    /// Another renderer has used the 3D core since this one's last frame:
    /// the core's registers and its memory manager are no longer this
    /// renderer's (each keeps its own shadow of them, ADR-0026 rule 3,
    /// and writes only what it changed), so the next frame sets the core
    /// up again (a reset and the manager's set-up, ~4.6 ms) before it
    /// draws. The OS has one renderer on the core (rule 5); measurement
    /// images with two (`rp-gpubench` GC2AG, GC2BB, GC2BC) call this at
    /// every switch, or the frame draws the other renderer's lists (found
    /// 2026-10-27: GC2BB's "lists on-die" pairs compared on-die against
    /// on-die; the host test `two_renderers_on_one_core`).
    pub fn core_taken(&mut self) {
        self.ready = false;
    }

    /// The tiles listed for the core in reverse order (last tile first), or
    /// in row order (the default). The picture is the same either way; the
    /// order the core draws its tiles in is what changes.
    pub fn set_tiles_reversed(&mut self, on: bool) {
        self.set_tile_order(if on {
            TileOrder::Reversed
        } else {
            TileOrder::Rows
        });
    }

    /// A measurement's bit flip in every textured object's header from the
    /// next [`Self::set`] on: `mask` XORed into header word `word` (0 ISP,
    /// 1-3 TEX0-2, 4 TSP) of both triangles (`rp-gpubench` run GC2AP's
    /// search). A word past the header or a zero mask flips nothing. The
    /// slots' hashes cover the flipped words, so a slot set again with the
    /// same draw is written again.
    pub fn set_header_xor(&mut self, word: usize, mask: u32) {
        let mut masks = [0; 5];
        if let Some(m) = masks.get_mut(word) {
            *m = mask;
        }
        self.header_xor = masks;
    }

    /// [`Self::set_header_xor`] for every header word at once: `masks[w]`
    /// XORed into word `w` (run GC2CH: TEX0's bit 0 with a bit of TEX1 or
    /// TEX2 beside it).
    pub fn set_header_xors(&mut self, masks: [u32; 5]) {
        self.header_xor = masks;
    }

    /// A measurement's bits ORed into every object's TSP word from the next
    /// [`Self::set`] on, flat and textured (`rp-gpubench` run GC2AQ: GC2AP
    /// found TSP bit 14, the punch-through object kind, and bit 15 each end
    /// divide 1's texel-pick fault); 0 changes nothing.
    pub fn set_tsp_or(&mut self, mask: u32) {
        self.tsp_or = mask;
    }

    /// A measurement's split of every quad along its other diagonal from
    /// the next [`Self::set`] on (`rp-gpubench` run GC2BU); the words
    /// differ, so the slots' hashes see it.
    pub fn set_diagonal_flip(&mut self, flip: bool) {
        self.diagonal_flip = flip;
    }

    /// A measurement's rotation of every flat triangle's vertex order (0-2)
    /// from the next [`Self::set`] on (`rp-gpubench` run GC2BX).
    pub fn set_vertex_rotate(&mut self, k: usize) {
        self.vertex_rotate = k % 3;
    }

    /// A measurement's bit flip in every flat triangle from the next
    /// [`Self::set`] on: `mask` XORed into word `word` (0 ISP, 1 TSP); a word
    /// past 1 or a zero mask flips nothing (`rp-gpubench` run GC2BX).
    pub fn set_flat_xor(&mut self, word: usize, mask: u32) {
        self.flat_xor = if word < 2 { (word, mask) } else { (0, 0) };
    }

    /// How each quad goes to the core from the next [`Self::set`] on: two
    /// triangles (the default) or one four-corner object ([`Objects`]). A
    /// slot set again in the other form is written again, and the lists
    /// naming it with it.
    pub fn set_objects(&mut self, objects: Objects) {
        self.objects = objects;
    }

    /// The order the tiles are listed for the core in ([`TileOrder`]).
    pub fn set_tile_order(&mut self, order: TileOrder) {
        self.tile_order = order;
    }

    /// Where its region array is (`rp-gpubench` run GC2BT logs it).
    #[must_use]
    pub fn regions_at(&self) -> u32 {
        self.regions
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

    /// Render done raises the 2700G's interrupt line, GPIO85 (`rp-mbx3d`
    /// version 24: an edge the moment status bit 2 sets, with `0x130` bits
    /// 15 and 2 and `MINTENABLE` bits 15 and 2). The caller then sleeps
    /// until the interrupt instead of the predicted time, and calls
    /// [`Self::poll`] (one status read, which also clears it). The line
    /// also carries the display's events: a wake-up without render done
    /// polls `Wait`. Off by default.
    pub fn set_interrupt(&mut self, on: bool) {
        if on != self.irq {
            self.irq = on;
            self.irq_stale = true;
        }
    }

    /// What to redo when the 3D clock comes back on ([`ClockPolicy`]).
    pub fn set_clock_policy(&mut self, p: ClockPolicy) {
        self.clock_policy = p;
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
        if draw.is_some_and(|d| {
            d.depth.is_some_and(|dp| dp.mode > depth::ALWAYS)
                || d.alpha.is_some_and(|a| a.func > depth::ALWAYS)
        }) {
            return Err(Error::Unsupported);
        }
        let new = match draw {
            None => Slot::default(),
            Some(d) => {
                let (mut words, n, kind) =
                    encode(d, self.objects, self.diagonal_flip, self.vertex_rotate);
                // Each object's header sits where a triangle's does (the
                // four-corner forms keep it: `m3::quad`, `m3::tex::quad`).
                let (_, per, objs) = kind.strip();
                let (per, objs) = (per as usize, objs as usize);
                let (fw, fmask) = self.flat_xor;
                if !kind.textured() && fmask != 0 {
                    for t in 0..objs {
                        words[t * per + fw] ^= fmask;
                    }
                }
                if kind.textured() {
                    for (hw, &mask) in self.header_xor.iter().enumerate() {
                        for t in 0..objs {
                            words[t * per + hw] ^= mask;
                        }
                    }
                }
                if self.tsp_or != 0 {
                    // The TSP: word 4 of a textured object, word 1 of a
                    // flat one (`m3::tex::triangle`, `m3::triangle`).
                    let at = if kind.textured() { 4 } else { 1 };
                    for t in 0..objs {
                        words[t * per + at] |= self.tsp_or;
                    }
                }
                let tiles = draw_tiles(d, self.cols, self.rows);
                let s = Slot {
                    kind,
                    tiles,
                    hash: hash(&words[..n], tiles),
                    cost: quad_cost(texels16(d), d.blend.is_some()).min(255) as u8,
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
        } else if new.cost != old.cost {
            // The lists naming it keep their words; their cost changes.
            for l in &mut self.lists[..self.nlists] {
                if l.mask & 1 << slot != 0 {
                    l.cost = l.cost - u16::from(old.cost) + u16::from(new.cost);
                }
            }
        }
        self.dirty.add_rect(old.tiles, self.cols);
        self.dirty.add_rect(new.tiles, self.cols);
        self.lists_changed.add_rect(old.tiles, self.cols);
        self.lists_changed.add_rect(new.tiles, self.cols);
        self.slots[slot] = new;
        Ok(())
    }

    /// Where slot `slot`'s object words sit in 2700G memory (as [`Self::set`]
    /// stores them), for device tests that change a word the renderer
    /// doesn't build yet (`rp-gpubench`'s texture-word probes) between a
    /// set and a render; `None` for no such slot.
    #[must_use]
    pub fn slot_words_at(&self, slot: usize) -> Option<u32> {
        (slot < MAX_SLOTS).then(|| self.params.at + SLOTS_AT + slot as u32 * SLOT_BYTES)
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

    /// The next render starts clean: the core reset, its parameter memory
    /// manager set up again and every register written, none taken from
    /// the shadow (`rp-gpubench` run GC2's rule 5: no state carried from
    /// one clock setting into the next).
    pub fn start_clean(&mut self) {
        self.ready = false;
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
        self.buckets = [NO_LIST; BUCKETS];
        self.hand = 0;
        // Lists come back at other places: every region entry with them.
        self.regions_full = None;
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
        // The lists kept stay where they are (their tiles' entries hold);
        // the chains are rebuilt for their new places in the cache.
        self.buckets = [NO_LIST; BUCKETS];
        for i in 0..k {
            let b = bucket(self.lists[i].mask);
            self.lists[i].next = self.buckets[b];
            self.buckets[b] = i as u8;
        }
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
        self.staged = None;
        match self.stage(bus, mem, clock, target)? {
            Some(s) => Ok(Some(self.start(bus, clock, s))),
            None => Ok(None),
        }
    }

    /// [`Self::render`]'s first half alone: the region array and the lists
    /// for `target` written now and the core not started, so the CPU can
    /// write them while the core renders another renderer's frame
    /// (`rp-gpubench` run GC2CP: frame N's conversion overlapped with frame
    /// N + 1's lists); [`Self::start_prepared`] starts it. Nothing may be
    /// [`Self::set`] between the two. True if there is something to render.
    /// Refused (`Busy`) while this renderer has a render in flight or the
    /// core needs setting up (a reset, which would end the frame another
    /// renderer has in flight).
    pub fn prepare<B: RegisterBus, M: LocalMem, C: Clock>(
        &mut self,
        bus: &mut B,
        mem: &mut M,
        clock: &mut C,
        target: Target,
    ) -> Result<bool, Error> {
        if self.pending.is_some() || !self.ready {
            return Err(Error::Busy);
        }
        self.staged = self.stage(bus, mem, clock, target)?;
        Ok(self.staged.is_some())
    }

    /// Starts what [`Self::prepare`] wrote: the start table's registers and
    /// the start. Returns the time to [`Self::poll`] at, or `None` if nothing
    /// was prepared.
    pub fn start_prepared<B: RegisterBus, C: Clock>(
        &mut self,
        bus: &mut B,
        clock: &mut C,
    ) -> Result<Option<u32>, Error> {
        if self.pending.is_some() {
            return Err(Error::Busy);
        }
        Ok(self.staged.take().map(|s| self.start(bus, clock, s)))
    }

    /// Another renderer started frames on the core since this one's last,
    /// and the core wasn't reset: only the start table's registers (the
    /// region array, the parameter memory, the target) are no longer this
    /// renderer's, so its next start writes them all again, with no reset
    /// and no memory manager set-up (the ~4.6 ms of [`Self::core_taken`]).
    /// Its lists must lie in their own parameter memory. Whether that is
    /// enough on the X51v (the memory manager is left as the last set-up
    /// wrote it) is what `rp-gpubench` run GC2CP measures, every picture
    /// checked; the OS keeps one renderer on the core (ADR-0026 point 5).
    pub fn registers_taken(&mut self) {
        self.shadow.forget();
    }

    /// The region array and the lists for `target`, the core set up first
    /// if it needs it: what a render starts, or `None` if nothing changed.
    fn stage<B: RegisterBus, M: LocalMem, C: Clock>(
        &mut self,
        bus: &mut B,
        mem: &mut M,
        clock: &mut C,
        target: Target,
    ) -> Result<Option<Staged>, Error> {
        let pic = target.picture;
        if u32::from(target.x) + u32::from(self.w) > u32::from(pic.w)
            || u32::from(target.y) + u32::from(self.h) > u32::from(pic.h)
            || pic.stride < u32::from(pic.w) * pic.format.bytes()
            || !pic.stride.is_multiple_of(4)
            || !pic.at.is_multiple_of(4)
            || !target.addr().is_multiple_of(2)
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
            clock.delay_us(1_000);
            match self.clock_policy {
                ClockPolicy::SetUpAgain => self.ready = false,
                ClockPolicy::RewriteRegisters => self.shadow.forget(),
                ClockPolicy::KeepAll => {}
            }
        }
        if !self.ready {
            self.bring_up(bus, mem, clock)?;
        }
        if self.bg_stale {
            self.write_background(mem);
        }
        if self.irq_stale {
            self.write_irq_enables(bus);
        }
        if self.arena - ARENA_AT > (self.params.size - ARENA_AT) / 2 {
            // Lists are referenced only by the frame being built: reclaim
            // them all between frames before space runs short.
            self.forget_lists();
        }
        let (tiles, quads, cost, lists) = match self.write_regions(mem, &todo) {
            Some(r) => r,
            None => {
                self.forget_lists();
                self.write_regions(mem, &todo).ok_or(Error::TooComplex)?
            }
        };
        mem.sync();
        Ok(Some(Staged {
            pic,
            addr,
            same,
            tiles,
            quads,
            cost,
            lists,
        }))
    }

    /// Starts what [`Self::stage`] wrote (the start table and the start),
    /// then notes which tiles each picture has. Returns the time to poll at.
    fn start<B: RegisterBus, C: Clock>(&mut self, bus: &mut B, clock: &mut C, s: Staged) -> u32 {
        let writes0 = self.shadow.writes;
        let t = m3::start_table(
            self.regions,
            self.params.at,
            u32::from(s.pic.w),
            u32::from(s.pic.h),
            s.pic.stride,
        );
        for &(off, v) in &t[..14] {
            let v = if off == 0x650 { s.pic.format.code() } else { v };
            self.shadow.write(bus, off, v);
        }
        self.shadow.write(bus, 0x65C, s.addr);
        self.shadow.write(bus, m3::START, 1);

        let predicted = predict_us(s.tiles, s.cost);
        // The first look at 7/8 of the learned time, then a step of 1/64 of
        // it: the end seen within ~2% of it, ~9 status reads a frame.
        let expect = (u64::from(predicted) * u64::from(self.pace) / 256) as u32;
        let start = clock.now_us();
        self.pending = Some(Pending {
            start,
            wait: expect - expect / 8,
            step: (expect / 64).max(PACE_STEP_MIN),
            // Generous: lists with many quads take longer a tile than
            // version 19's one object (to measure).
            deadline: predicted.saturating_mul(4).saturating_add(20_000),
            stats: FrameStats {
                tiles: s.tiles,
                quads: s.quads,
                lists: s.lists,
                mem_words: core::mem::take(&mut self.mem_words),
                reg_writes: self.shadow.writes - writes0,
                predicted_us: predicted,
                done_us: 0,
            },
        });
        if s.same {
            self.other.add_all(&self.dirty);
        } else {
            self.other = self.dirty.clone();
            self.targets = [Some(s.addr), self.targets[0]];
        }
        self.dirty.clear();
        start.wrapping_add(expect - expect / 8)
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
            self.learn_pace(&p, gone);
            return Poll::Done(p.stats);
        }
        if gone >= p.deadline {
            self.pending = None;
            self.ready = false;
            self.targets = [None; 2];
            self.regions_full = None;
            self.invalidate_all();
            // Stop the hung core now; a failure here is retried by the
            // next render's set-up.
            let _ = self.bring_up(bus, mem, clock);
            return Poll::Recovered;
        }
        // Not yet: look again a step later (a status read is one bus read;
        // a coarse step wastes the GPU's time).
        Poll::Wait(now.wrapping_add(p.step))
    }

    /// The pace learned from a render seen done `gone` us after its start:
    /// its time against [`predict_us`]'s, x 256. The prediction was fitted
    /// at Windows Mobile's clock; at divide 1 the core takes half or less of
    /// it (`rp-gpubench` GC2CP: every frame waited to the prediction, the
    /// same at 74.75 / 1 and 143.00 / 1). Seen done at the first look, the
    /// render ended some time before it, unknown how long: the pace drops
    /// a quarter below what was seen, so a faster clock is caught up with in
    /// a few frames; seen later, it is known to a step and averaged in.
    fn learn_pace(&mut self, p: &Pending, gone: u32) {
        let predicted = u64::from(p.stats.predicted_us.max(1));
        let seen = (u64::from(gone) * 256 / predicted) as u32;
        self.pace = if gone <= p.wait {
            seen * 3 / 4
        } else if seen < self.pace {
            seen
        } else {
            (3 * self.pace + seen) / 4
        }
        .clamp(PACE_MIN, PACE_MAX);
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
        self.write_irq_enables(bus);
        self.ready = ok;
        if ok { Ok(()) } else { Err(Error::NoCore) }
    }

    /// `0x130` bit 15 (the MBX core's master enable) and `MINTENABLE`
    /// as Dell writes it (`0x800F`: bit 15 the master, bit 2 the MBX core,
    /// bits 0, 1, 3 the display's) when interrupts are on; bit 15 off when
    /// not (render done still sets the status bit).
    fn write_irq_enables<B: RegisterBus>(&mut self, bus: &mut B) {
        let en = bus.read32(MBX + reg::ENABLE);
        if self.irq {
            bus.write32(MBX + reg::ENABLE, en | IRQ_MASTER | m3::RENDER_DONE);
            bus.write32(MINTENABLE, MINTENABLE_DELL);
        } else {
            bus.write32(MBX + reg::ENABLE, en & !IRQ_MASTER);
        }
        self.irq_stale = false;
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
    ///
    /// Per tile (R88: ~2 us a tile on the X51v before, GC2CP): the walk
    /// gives its column and row with no division; its quads come from the
    /// row's (built once a row); its list from the cache's bucket, with its
    /// count and cost; and when every tile is listed again in the order of
    /// the last full array, a tile whose quads didn't change keeps its
    /// entry (nothing stored).
    fn write_regions<M: LocalMem>(
        &mut self,
        mem: &mut M,
        todo: &Tiles,
    ) -> Option<(u32, u32, u32, u32)> {
        let mut buf = [0u32; 64];
        // `buf` holds entries from `run` on; `n` tiles listed so far.
        let (mut nb, mut run, mut n, mut lists, mut stored) = (0usize, 0u32, 0u32, 0u32, 0u32);
        let (mut quads, mut cost) = (0, 0);
        let (mut row, mut row_mask) = (u32::MAX, 0u64);
        let (cols, rows) = (self.cols, self.rows);
        let order = self.tile_order;
        let full = todo.count() == cols * rows;
        let reuse = full && self.regions_full == Some(order);
        // The last tile's entry is in `buf` (it carries the end bit).
        let mut last_in_buf = false;
        let changed = core::mem::take(&mut self.lists_changed);
        self.regions_full = None;
        tiles::walk(order, todo, (cols, rows), |k, x, y| {
            if y != row {
                row = y;
                row_mask = 0;
                for (s, slot) in self.slots.iter().enumerate() {
                    let t = slot.tiles;
                    if slot.kind != Kind::Empty
                        && !t.is_empty()
                        && y >= u32::from(t.y0)
                        && y < u32::from(t.y1)
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
                let t = self.slots[s].tiles;
                if x >= u32::from(t.x0) && x < u32::from(t.x1) {
                    mask |= 1 << s;
                }
            }
            let (at, new, q, c) = self.list_for(mem, mask)?;
            quads += q;
            cost += c;
            lists += u32::from(new);
            if reuse && !new && !changed.has(k) {
                // Its entry from the last full array: the same tile at the
                // same place, the same list.
                if nb > 0 {
                    mem.store((self.regions + 4 * run) as usize, &buf[..nb]);
                    stored += nb as u32;
                    nb = 0;
                }
                n += 1;
                run = 2 * n;
                last_in_buf = false;
                return Some(());
            }
            if nb == buf.len() {
                mem.store((self.regions + 4 * run) as usize, &buf);
                stored += nb as u32;
                run += nb as u32;
                nb = 0;
            }
            let e = m3::region_entry(x, y, self.params.at + at, false);
            buf[nb] = e[0];
            buf[nb + 1] = e[1];
            nb += 2;
            n += 1;
            last_in_buf = true;
            Some(())
        })?;
        if last_in_buf {
            buf[nb - 2] |= m3::LAST_REGION;
        }
        if nb > 0 {
            mem.store((self.regions + 4 * run) as usize, &buf[..nb]);
            stored += nb as u32;
        }
        self.mem_words += stored;
        if full {
            self.regions_full = Some(order);
        }
        Some((n, quads, cost, lists))
    }

    /// Whether the list cache holds together (tests): every cached list
    /// reached from its own bucket exactly once, every chain only its
    /// bucket's lists and ending.
    #[cfg(test)]
    fn cache_ok(&self) -> bool {
        let mut seen = [false; LISTS];
        for (b, &head) in self.buckets.iter().enumerate() {
            let mut i = head;
            let mut steps = 0;
            while i != NO_LIST {
                let k = usize::from(i);
                if k >= self.nlists || seen[k] || bucket(self.lists[k].mask) != b || steps > LISTS {
                    return false;
                }
                seen[k] = true;
                steps += 1;
                i = self.lists[k].next;
            }
        }
        seen[..self.nlists].iter().all(|&s| s)
    }

    /// The list of the quads in `mask` (after the background), in the
    /// parameter memory: reused if written before. Returns its offset,
    /// whether it was written now, its quads and their cost.
    fn list_for<M: LocalMem>(&mut self, mem: &mut M, mask: u64) -> Option<(u32, bool, u32, u32)> {
        let found = |l: &List| (l.at, false, u32::from(l.quads), u32::from(l.cost));
        // Neighbouring tiles mostly share their quads: try the last first.
        if self.last < self.nlists && self.lists[self.last].mask == mask {
            return Some(found(&self.lists[self.last]));
        }
        let b = bucket(mask);
        let mut i = self.buckets[b];
        while i != NO_LIST {
            let l = &self.lists[usize::from(i)];
            if l.mask == mask {
                self.last = usize::from(i);
                return Some(found(l));
            }
            i = l.next;
        }
        let mut w = [m3::END; LIST_MAX];
        let mut n = 0;
        if self.background.is_some() {
            w[0] = m3::STRIP_ONE_TRIANGLE | (BG_AT / 4);
            n = 1;
        }
        let mut cost = 0u32;
        let mut m = mask;
        while m != 0 {
            let s = m.trailing_zeros();
            m &= m - 1;
            let slot = self.slots[s as usize];
            cost += u32::from(slot.cost);
            let (flags, words, objs) = slot.kind.strip();
            let base = (SLOTS_AT + s * SLOT_BYTES) / 4;
            w[n] = flags | base;
            if objs == 2 {
                w[n + 1] = flags | (base + words);
            }
            n += objs as usize;
        }
        n += 1; // the end word
        let at = self.arena;
        if at + 4 * n as u32 > self.params.size {
            return None;
        }
        mem.store((self.params.at + at) as usize, &w[..n]);
        self.arena += 4 * n as u32;
        self.mem_words += n as u32;
        let quads = mask.count_ones();
        // A place in the cache: the next free one, or when it's full the
        // next in turn (never the one just used: the tile beside likely
        // wants it), taken out of its bucket's chain first. A list given
        // up so stays in the parameter memory (tiles listed with it this
        // frame or before keep their entries); if its quads come back it is
        // written again as new, so its tiles' entries are too.
        let i = if self.nlists < LISTS {
            self.nlists += 1;
            self.nlists - 1
        } else {
            let mut v = usize::from(self.hand) % LISTS;
            if v == self.last {
                v = (v + 1) % LISTS;
            }
            self.hand = ((v + 1) % LISTS) as u8;
            let old = bucket(self.lists[v].mask);
            if usize::from(self.buckets[old]) == v {
                self.buckets[old] = self.lists[v].next;
            } else {
                let mut j = usize::from(self.buckets[old]);
                while usize::from(self.lists[j].next) != v {
                    j = usize::from(self.lists[j].next);
                }
                self.lists[j].next = self.lists[v].next;
            }
            v
        };
        self.lists[i] = List {
            mask,
            at,
            cost: cost as u16,
            quads: quads as u8,
            next: self.buckets[b],
        };
        self.buckets[b] = i as u8;
        self.last = i;
        Some((at, true, quads, cost))
    }
}

#[cfg(test)]
mod tests;
