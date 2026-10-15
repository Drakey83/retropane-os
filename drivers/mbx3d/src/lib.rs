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
/// The TSP word Dell's driver writes with [`ISP_ALWAYS`] (bits not yet
/// decoded; Gouraud-shaded, opaque as far as the objects that use it go).
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
/// Status bit: the 3D stage finished a render.
pub const RENDER_DONE: u32 = 1 << 2;

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
        (0x6D8, 0x0900_0000),
        (0x838, 1),
    ]
}

#[cfg(test)]
mod tests;
