//! The MBX 3D core's scene words (ADR-0024, milestone 2).
//!
//! The 3D stage of the 2700G's PowerVR MBX draws a frame from three
//! structures in 2700G memory, started by a table of register writes
//! (docs/hw/2700g.md, "The scene bytes"):
//!
//! 1. the **region array**: two words per tile, `x | y << 8` and the
//!    address of the tile's object list; bit 31 marks the last tile;
//! 2. **object lists**: strip pointers to objects, then the end word;
//! 3. **objects**: an ISP word (depth test), a TSP word (shading,
//!    blending), then the vertices grouped: all x/y pairs, then all z,
//!    then all 1/w, then all colours.
//!
//! Every value here is one Windows Mobile's driver itself writes (found in
//! Dell's display driver, no code copied): the first triangle is built
//! only from proven words, and changes one thing at a time from there. The
//! formats are close to the Dreamcast PowerVR2's published ones, the guide
//! for what's still unknown (the ISP/TSP bits, the strip pointer's flags,
//! the tile size, textures, the tile accelerator path).
//!
//! Pure: no hardware access; the test image writes these words.

#![no_std]
#![forbid(unsafe_code)]

/// ISP word "depth compare always" (bits 31-29 = 7), as Dell's driver
/// writes it for its background and minimal objects.
pub const ISP_ALWAYS: u32 = 0xE000_0000;
/// TSP word bit 22: Gouraud shading, colours blended between the vertices
/// (confirmed on the device 2026-10-14: the corners came out near their
/// vertex colours, the middle a third of each).
/// `libGLES_CL.dll`'s `glShadeModel(GL_SMOOTH)` sets bit 22 of its state
/// word, whose bits 16-23 go straight into the TSP word it builds
/// (`0x02267724`; untextured, it ORs in `0x0E80`, Dell's TSP word's low
/// bits). Dell's `0x22200E80` has it clear: drawn flat, the first
/// vertex's colour (measured 2026-10-14; ISP bit 23, the PowerVR2's place
/// for it, changed nothing).
pub const TSP_GOURAUD: u32 = 1 << 22;
/// The TSP word Dell's driver writes with [`ISP_ALWAYS`]: flat-shaded
/// (bit 22 clear), opaque; the other bits not yet decoded.
pub const TSP_DELL: u32 = 0x2220_0E80;
/// Ends an object list.
pub const END: u32 = 0xF000_0000;
/// A strip pointer for one triangle (mask bit 30: triangle 0 drawn; bits
/// 23-21 as Dell's minimal scene sets them), parameters' offset in words in
/// bits 20-0.
pub const STRIP_ONE_TRIANGLE: u32 = 0x4080_0000;
/// Bit 31 of a region array entry: the last tile.
pub const LAST_REGION: u32 = 1 << 31;
/// The 3D stage's start register; writing 1 starts the render.
pub const START: usize = 0x680;
/// Words in a one-triangle object: ISP, TSP, then 15.
pub const TRIANGLE_WORDS: usize = 17;
/// The engine's addresses wrap at 32 MiB (measured for the 2D engine; the
/// 3D table masks them the same way).
pub const ADDR_MASK: u32 = 0x01FF_FFFC;

/// One vertex: screen position (pixels), depth, 1/w, colour (ARGB).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vertex {
    /// Horizontal position.
    pub x: f32,
    /// Vertical position.
    pub y: f32,
    /// Depth.
    pub z: f32,
    /// 1/w (1.0 for flat 2D work).
    pub rhw: f32,
    /// Colour, `0xAARRGGBB`.
    pub argb: u32,
}

impl Vertex {
    /// A flat vertex at `(x, y)` in `argb` (z 0, 1/w 1: as Dell's
    /// background object).
    #[must_use]
    pub const fn flat(x: f32, y: f32, argb: u32) -> Self {
        Self {
            x,
            y,
            z: 0.0,
            rhw: 1.0,
            argb,
        }
    }
}

/// A one-triangle object: `[ISP, TSP, x0, y0, x1, y1, x2, y2, z0..2,
/// rhw0..2, colour0..2]`.
#[must_use]
pub fn triangle(isp: u32, tsp: u32, v: &[Vertex; 3]) -> [u32; TRIANGLE_WORDS] {
    let mut w = [0u32; TRIANGLE_WORDS];
    w[0] = isp;
    w[1] = tsp;
    for (k, p) in v.iter().enumerate() {
        w[2 + 2 * k] = p.x.to_bits();
        w[3 + 2 * k] = p.y.to_bits();
        w[8 + k] = p.z.to_bits();
        w[11 + k] = p.rhw.to_bits();
        w[14 + k] = p.argb;
    }
    w
}

/// An object list with one triangle whose parameters are `params_offset`
/// bytes into the parameter buffer (register `0x60C`): the strip pointer,
/// then the end word.
#[must_use]
pub fn one_triangle_list(params_offset: u32) -> [u32; 2] {
    [
        STRIP_ONE_TRIANGLE | ((params_offset / 4) & 0x001F_FFFF),
        END,
    ]
}

/// Textured objects, as DoomGLES's on the device (ADR-0024 M3, capture
/// version 6, 2026-10-14): `ISP, TEX0, TEX1, TEX2, TSP`, the x/y pairs,
/// z, 1/w, then per vertex its colour and texture coordinates (u, v, 0 to
/// 1). The three texture words are the GLES context's per-unit words
/// (`libGLES_CL.dll` `0x02267434`): format and size, address, combine.
pub mod tex {
    /// The ISP word DoomGLES's textured objects carry (our flat ones:
    /// `0xE0000000`; bits 27, 26 and 23 more, meaning not yet known).
    pub const ISP_GAME: u32 = 0xEC80_0000;
    /// The TSP word of most of DoomGLES's objects: Gouraud (bit 22) and
    /// `0x0200` where untextured ones have `0x0E80`.
    pub const TSP_GAME: u32 = 0x2264_0200;
    /// TEX0 of DoomGLES's 128 x 128 (likely) PVRTC textures: sizes in bits
    /// 27-24 and 23-20 (likely, 3 and 3 here), the format in bits 15-12
    /// (likely; 9 here, PVRTC 4 bpp), the rest as found.
    pub const FORMAT_GAME: u32 = 0x0333_9600;
    /// TEX0's format field (likely).
    pub const FORMAT_SHIFT: u32 = 12;
    /// TEX1's high bits as DoomGLES writes them (bits 31-25: a level of
    /// detail setting); the texture's address / 128 goes below.
    pub const ADDRESS_GAME: u32 = 0xCE00_0000;
    /// TEX2: how the texture and the vertex colour combine; DoomGLES's
    /// (modulate, likely: its vertex colours are greys).
    pub const COMBINE_GAME: u32 = 0x8608_6610;
    /// The list pointer for one textured triangle as DoomGLES's lists have
    /// it (bit 24 where the flat ones have bit 23); the object's offset in
    /// words below.
    pub const STRIP_GAME: u32 = 0x4100_0000;
    /// Words in a textured one-triangle object.
    pub const WORDS: usize = 26;

    /// A vertex with texture coordinates.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Vertex {
        /// Horizontal position (pixels).
        pub x: f32,
        /// Vertical position.
        pub y: f32,
        /// Depth.
        pub z: f32,
        /// 1/w.
        pub rhw: f32,
        /// Colour, `0xAARRGGBB`.
        pub argb: u32,
        /// Texture coordinate across, 0 to 1.
        pub u: f32,
        /// Texture coordinate down, 0 to 1.
        pub v: f32,
    }

    /// TEX1 for a texture at `addr` in 2700G memory (128-byte aligned).
    #[must_use]
    pub const fn address(addr: u32) -> u32 {
        ADDRESS_GAME | (addr >> 7) & 0x01FF_FFFF
    }

    /// A textured one-triangle object.
    #[must_use]
    pub fn triangle(isp: u32, texw: [u32; 3], tsp: u32, v: &[Vertex; 3]) -> [u32; WORDS] {
        let mut w = [0u32; WORDS];
        w[0] = isp;
        w[1..4].copy_from_slice(&texw);
        w[4] = tsp;
        for (k, p) in v.iter().enumerate() {
            w[5 + 2 * k] = p.x.to_bits();
            w[6 + 2 * k] = p.y.to_bits();
            w[11 + k] = p.z.to_bits();
            w[14 + k] = p.rhw.to_bits();
            w[17 + 3 * k] = p.argb;
            w[18 + 3 * k] = p.u.to_bits();
            w[19 + 3 * k] = p.v.to_bits();
        }
        w
    }

    /// An object list of textured triangles at `offsets` (bytes into the
    /// parameter buffer), then the end word.
    #[must_use]
    pub fn list<const N: usize>(offsets: [u32; N]) -> [u32; 8] {
        let mut l = [super::END; 8];
        for (k, o) in offsets.iter().enumerate().take(7) {
            l[k] = STRIP_GAME | ((o / 4) & 0x001F_FFFF);
        }
        l
    }
}

/// A region (tile) as the core covers it on the X51v at 32 bpp, measured
/// 2026-10-14: 8 pixels wide, 16 tall.
pub const TILE_W: u32 = 8;
/// See [`TILE_W`].
pub const TILE_H: u32 = 16;

/// One region array entry: tile `(x, y)`, its object list at `list_addr`,
/// `last` on the final entry.
#[must_use]
pub fn region_entry(x: u32, y: u32, list_addr: u32, last: bool) -> [u32; 2] {
    let w = (x & 127) | (y & 127) << 8;
    [
        if last { w | LAST_REGION } else { w },
        list_addr & ADDR_MASK,
    ]
}

/// Fills `out` with the region array for a `cols` x `rows` tile grid,
/// rows outer, every tile pointing at the object list at `list_addr`.
/// Returns the words written (`2 * cols * rows`), or `None` if `out` is
/// too short, the grid is empty, or a coordinate doesn't fit 7 bits.
pub fn region_array(out: &mut [u32], cols: u32, rows: u32, list_addr: u32) -> Option<usize> {
    if cols == 0 || rows == 0 || cols > 128 || rows > 128 {
        return None;
    }
    let n = 2 * (cols * rows) as usize;
    let out = out.get_mut(..n)?;
    for (k, pair) in out.as_chunks_mut::<2>().0.iter_mut().enumerate() {
        let (x, y) = (k as u32 % cols, k as u32 / cols);
        pair[0] = x | y << 8;
        pair[1] = list_addr & ADDR_MASK;
    }
    out[n - 2] |= LAST_REGION;
    Some(n)
}

/// The 3D stage's start table, as Dell's driver builds it from constants
/// (`0x0211b5b4`): region array and parameter buffer addresses (the latter
/// 4 KiB aligned), a 32 bpp target `width` x `height` with `stride` bytes
/// a line; the last pair starts the render.
#[must_use]
pub fn start_table(
    region_addr: u32,
    params_addr: u32,
    width: u32,
    height: u32,
    stride: u32,
) -> [(usize, u32); 15] {
    [
        (0x608, region_addr & ADDR_MASK),
        (0x60C, params_addr & 0x01FF_F000),
        (0x610, 0),
        (0x644, 0),
        (0x650, 6),
        (0x654, width.saturating_sub(1) << 16),
        (0x658, height.saturating_sub(1) << 16),
        (0x660, stride / 4),
        (0x6D8, 0x0100_0000),
        (0x61C, 0x0002_0004),
        (0x614, 0),
        (0x618, 0),
        (0x648, 0),
        (0x6DC, 0),
        (START, 1),
    ]
}

/// Status (`0x12c`) bit: the parameter memory manager is ready (set after
/// [`pm_init_table`]'s last write; Dell enables it in `0x130` first and
/// clears it by writing it to `0x134`).
pub const PM_READY: u32 = 1 << 6;
/// `0x6D8` as the parameter memory set-up leaves it. The GLES driver
/// never writes `0x6D8` (static reading 2026-10-15), so DoomGLES renders
/// with this; Dell's start table writes `0x01000000` (bit 27 clear) for
/// its flat scenes.
pub const PM_6D8: u32 = 0x0900_0000;
/// How far the parameter memory manager reaches: its pages must lie below
/// 8 MiB of 2700G memory (`rp-mbx3d` version 14, 2026-10-15: with the
/// parameter memory at 12 MiB, the tile accelerator wrote its lists 4 MiB
/// lower, `0xEDC6DC` as `0x6DC6DC`: 11-bit page numbers, likely).
/// DoomGLES's parameter memory is below it too (`0x2B4000`).
pub const PM_REACH: usize = 0x80_0000;
/// Status bit: the 3D stage finished a render.
pub const RENDER_DONE: u32 = 1 << 2;

/// The GLES driver's frame state, the part Dell's start table leaves out
/// (`libGLES_CL.dll`: the 3D table `0x022616dc`-`0x02261848`, patched per
/// frame by the scene routine `0x0225e5c4`; static reading 2026-10-15,
/// meanings likely until the device confirms them).
pub mod frame {
    /// Region header bit 29: on every entry of DoomGLES's frames (capture
    /// version 4 at `0x265000`, version 6 at `0x191000`: 2400 entries, 60
    /// x 40, bit 31 on the last), never in Dell's clear scene. Dell's own
    /// flush path (`0x02118af4`) sets it on a list's tiles.
    pub const REGION_GAME: u32 = 1 << 29;

    /// `0x6DC`: the depth the tiles start at, as a float, with bit 0 set
    /// when the frame clears depth (`glClear` stores the clear depth at
    /// context `+0x6F0`; the scene routine writes it with bit 0,
    /// `0x0225e7e4`). Dell's table writes 0.
    #[must_use]
    pub fn clear_depth(z: f32) -> u32 {
        z.to_bits() | 1
    }

    /// `0x610` with a depth buffer (`0x674` its address) of `tiles` tiles:
    /// `(tiles - 1) << 7`, bit 1 set (store, likely), bit 0 when the depth
    /// is loaded rather than cleared (the scene routine clears it when the
    /// frame clears depth). Without a depth buffer the GLES driver writes
    /// `0x30001` and `0x674 = 0`.
    #[must_use]
    pub fn depth_setup(tiles: u32, load: bool) -> u32 {
        tiles.saturating_sub(1) << 7 | 2 | u32::from(load)
    }

    /// `0x860`: the tile grid, `(cols - 1) | (rows - 1) << 16` (from the
    /// scene object's `+0xC` and `+0x10`, likely columns and rows).
    #[must_use]
    pub fn tile_grid(cols: u32, rows: u32) -> u32 {
        cols.saturating_sub(1) | rows.saturating_sub(1) << 16
    }
}

/// The parameter memory manager's set-up, as Dell's driver writes it
/// before any render (`0x021179a8`): `pm` (4 KiB aligned, `pm_size` bytes,
/// its first 4 KiB a header Dell keeps its own small lists in) and a
/// 16 KiB-aligned 16 KiB block `table`. The last pair starts it; then wait
/// for [`PM_READY`].
#[must_use]
pub fn pm_init_table(pm: u32, pm_size: u32, table: u32) -> [(usize, u32); 6] {
    let first = (pm + 4096) >> 12;
    let pages_minus_one = (pm_size >> 12).saturating_sub(1);
    [
        (0x824, table),
        (0x828, first),
        (0x82C, pages_minus_one + first - 1),
        (0x83C, pm),
        (0x6D8, PM_6D8),
        (0x838, 1),
    ]
}

/// The tile accelerator (TA) path, as the GLES driver drives it (ADR-0024
/// M5; static reading 2026-10-15): the TA takes a stream of words and
/// writes the region array, lists and objects itself.
pub mod ta {
    /// The TA's input window in the register window (`0x0FFF4000`): 4 KiB,
    /// written sequentially, wrapping, sharing the 2D engine's FIFO level.
    pub const PORT: usize = 0x1_4000;
    /// The window one `1` goes to after a batch (`0x0FFF6000`).
    pub const END_PORT: usize = 0x1_6000;
    /// The windows' size.
    pub const PORT_SIZE: usize = 0x1000;
    /// Writing 1 starts the TA (the TA table's last register).
    pub const START: usize = 0x800;
    /// Status bit: the TA finished (Dell's interrupt handler).
    pub const DONE: u32 = 1 << 4;
    /// `1.0e-5` as a float: `0x854` and `0x858`.
    pub const EPSILON: u32 = 0x3727_C5AC;

    /// The TA's start table (`libGLES_CL.dll` `0x02261580`): `tail` a
    /// zeroed 128 KiB-aligned buffer of 4 bytes a tile (likely tail
    /// pointers), `regions` the region array the TA writes ((tiles + 1) x
    /// 8 bytes), a `width` x `height` target of `cols` x `rows` tiles. The
    /// last pair starts it.
    #[must_use]
    pub fn start_table(
        tail: u32,
        regions: u32,
        width: u32,
        height: u32,
        cols: u32,
        rows: u32,
    ) -> [(usize, u32); 9] {
        let grid = rows.saturating_sub(1) | (cols.saturating_sub(1) & 0xFE) << 9;
        [
            (0x840, tail),
            (0x844, regions),
            (0x848, 0x00FF_0004),
            (0x84C, width << 16),
            (0x850, height << 16),
            (0x854, EPSILON),
            (0x858, EPSILON),
            (0x85C, 0x0004_00E1 | grid << 10),
            (START, 1),
        ]
    }

    /// `0x85C` bit 7, set while the scene's end word goes in: a whole
    /// scene (`libGLES_CL.dll` `0x0226fdc0`; clear when the TA is paused
    /// for another context, whose state is then saved). The end word is
    /// [`super::END`] (`0xF0000000`) in the TA window; `0x85C` is put back
    /// after.
    pub const SCENE_END: u32 = 1 << 7;
    /// A primitive's first word: a strip (bits 31-28; `0x5` fan, `0x7`
    /// triangle list), the polygon offset 0.
    pub const STRIP: u32 = 0x4000_0000;

    /// The state block's clip word (mask bit 4; `libGLES_CL.dll`
    /// `0x0226deb4`): a rectangle in 16-pixel units, `1 << 29 | xmin << 21
    /// | xmax << 14 | ymin << 7 | ymax` (max inclusive); the clear sends 0
    /// (no clipping, it seems: version 15 drew the whole screen).
    #[must_use]
    pub fn clip(width: u32, height: u32) -> u32 {
        let last = |n: u32| (n.div_ceil(16).saturating_sub(1)) & 127;
        1 << 29 | last(width) << 14 | last(height)
    }

    /// The texture words of the GLES driver's own textured draw
    /// (`0x0226eae4`), the same as Dell's object in capture 4 (`0x12A000`):
    /// TEX0 `0xA8800000` (size and format low bits from context `+0x134`),
    /// TEX1 `0x0E000000 | address / 128` (likely, from `+0x138`), TEX2
    /// `0xD6886610`; its TSP `0x22220E80`.
    #[must_use]
    pub fn dell_texture(addr: u32) -> [u32; 3] {
        [
            0xA880_0000,
            0x0E00_0000 | (addr >> 7) & 0x01FF_FFFF,
            0xD688_6610,
        ]
    }
    /// The TSP word of that draw.
    pub const TSP_DELL_TEXTURED: u32 = 0x2222_0E80;

    /// Words in a textured, coloured vertex the GLES driver's transform
    /// stage sends (`0x02272064`: 16 bytes, + 4 with a colour, + 8 per
    /// texture unit; the 7-word emitter `0x02274644`): x, y, z, 1/w,
    /// colour, u, v (the order read from the object planes, likely).
    pub const TEX_VERTEX_WORDS: usize = 7;

    /// One textured triangle as a strip: the state block (mask
    /// `0x10000057`: TSP, ISP, texture unit 0's three words, the clip
    /// word, the bit 6 word; the GLES driver's own textured draw
    /// `0x0226eae4` has this form), the strip word, then
    /// three [`TEX_VERTEX_WORDS`]-word vertices. Then a `1` goes to
    /// [`END_PORT`].
    #[must_use]
    pub fn textured_stream(
        tsp: u32,
        isp: u32,
        tex: [u32; 3],
        clip: u32,
        v: &[super::tex::Vertex; 3],
    ) -> [u32; 9 + 3 * TEX_VERTEX_WORDS] {
        let mut s = [0u32; 9 + 3 * TEX_VERTEX_WORDS];
        s[..7].copy_from_slice(&[0x1000_0057, tsp, isp, tex[0], tex[1], tex[2], clip]);
        // s[7]: the bit 6 word, 0 as in the GLES driver's draws.
        s[8] = STRIP;
        for (k, p) in v.iter().enumerate() {
            let at = 9 + k * TEX_VERTEX_WORDS;
            s[at..at + TEX_VERTEX_WORDS].copy_from_slice(&[
                p.x.to_bits(),
                p.y.to_bits(),
                p.z.to_bits(),
                p.rhw.to_bits(),
                p.argb,
                p.u.to_bits(),
                p.v.to_bits(),
            ]);
        }
        s
    }

    /// The GLES driver's clear (`0x0226dff0`), word for word: the state
    /// block (mask `0x10000053`: TSP, ISP, two more words), a strip of one
    /// triangle covering a `width` x `height` target twice over, x, y, z
    /// per vertex. Then a `1` goes to [`END_PORT`].
    #[must_use]
    pub fn clear_stream(width: u32, height: u32) -> [u32; 15] {
        let (w, h) = (2.0 * width as f32, 2.0 * height as f32);
        let one = 1.0f32.to_bits();
        [
            0x1000_0053,
            0x2220_6F80,
            super::ISP_ALWAYS,
            0,
            0,
            STRIP,
            0,
            0,
            one,
            w.to_bits(),
            0,
            one,
            0,
            h.to_bits(),
            one,
        ]
    }
}

#[cfg(test)]
mod tests;
