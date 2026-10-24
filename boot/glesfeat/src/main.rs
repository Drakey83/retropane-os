//! Test image (ADR-0026, static reading of Intel's GLES driver 2026-10-18,
//! docs/hw/2700g.md "Alpha test, fog, the second texture unit"): the MBX
//! 3D core's alpha test, depth "less" without it, and 8-bit grey textures,
//! under the loader. Results only, logged as pixels; no board bit is
//! switched (the 3D clock as `rp-mbx3d` sets it, put back at the end).
//!
//! - **A, alpha test.** The object's second word ("ISP" so far) carries,
//!   by the GLES driver, the alpha test: bits 31-29 the comparison in GL
//!   order, bits 28-21 the reference; TSP bit 14 marks an alpha-tested
//!   object. A red square with vertex alpha 64 over grey, the comparison
//!   always / greater 128 / greater 32 / less 128 / less 0, some also with
//!   TSP bit 14.
//! - **D, depth "less".** Version 23 (T7b) put the depth mode into those
//!   same bits (`mode << 29`, reference 0) and "less" never drew: an alpha
//!   test "alpha < 0" never passes. Repeated with them "always" and the
//!   mode only in TSP bits 11-9, and once the old way (the control).
//! - **L, 8-bit grey textures.** An 8 x 8 texture of one byte value at
//!   format code 10 (the GLES driver's `GL_LUMINANCE`), the combine word
//!   for luminance with modulate (`0x86084610`), white vertices.
//!
//! Every render has a timeout; a timed-out one resets the core and sets
//! the memory manager up again. The target is shown until a button (10 s
//! at most), then everything is put back.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed with the verdict.
pub const MARKER: &str = "RETROPANE-GLESFEAT";

/// The second object word with the alpha test the GLES driver forms
/// (`glAlphaFuncx`, `0x02262060`): comparison `func` (GL order, 0 never ..
/// 7 always) in bits 31-29, reference `reference` in bits 28-21.
#[must_use]
pub const fn alpha_word(func: u32, reference: u32) -> u32 {
    (func & 7) << 29 | (reference & 0xFF) << 21
}

/// TSP bit 14: the alpha-tested object kind (the GLES driver sets it when
/// the test can fail).
pub const TSP_ALPHA_TEST: u32 = 1 << 14;

/// Test G (ROADMAP step 3, the give-up audit's item 1): Gouraud at the
/// cores' divide 1 through the tile accelerator against our hand-written
/// objects. Dell's driver draws Gouraud right at 74.75 MHz / 1
/// (`rp-wmprobe`, 2026-10-23) where ours is wrong in every render (GC2);
/// Dell's driver feeds the TA, and the suspect is our list word's layout
/// field (bits 24-21), 4 for every untextured object, Gouraud included.
/// The quad is GC2's ARGB8888 Gouraud kind: the whole 480 x 640 target,
/// corners black, white, blue, yellow (top left, top right, bottom right,
/// bottom left), as `(x, y, colour)`.
pub const QUAD: [(f32, f32, u32); 4] = [
    (0.0, 0.0, 0xFF00_0000),
    (480.0, 0.0, 0xFFFF_FFFF),
    (480.0, 640.0, 0xFF10_20A0),
    (0.0, 640.0, 0xFFFF_E000),
];
/// The quad's two triangles, as the renderer splits it (`rp-mbx3d-render`
/// `encode`): top left, top right, bottom left; top right, bottom right,
/// bottom left.
pub const TRIS: [[usize; 3]; 2] = [[0, 1, 3], [1, 2, 3]];
/// Dell's untextured smooth TSP (`libGLES_CL` `0x02267700`): Dell's TSP
/// with Gouraud's bit 22; bit 8 clear, a colour with every vertex.
pub const TSP_SMOOTH: u32 = 0x2260_0E80;
/// The state block's mask for an untextured draw (the GLES clear's): the
/// TSP, ISP, clip and bit-6 words follow it.
pub const TA_STATE: u32 = 0x1000_0053;
/// The TA's primitive word for a strip (polygon offset 0).
pub const TA_STRIP: u32 = 0x4000_0000;
/// Words in an untextured, coloured TA vertex (the transform stage's 16
/// bytes + 4 for a colour, `0x02272064`): x, y, z, 1/w, colour.
pub const TA_VERTEX_WORDS: usize = 5;
/// The longest [`ta_draw`]: a strip of four vertices.
pub const TA_DRAW_MAX: usize = 6 + 4 * TA_VERTEX_WORDS;
/// The list word's layout field (bits 24-21: the TA writes 0 for the
/// clear's colourless object, 4 flat, 8 one texture unit, 12 two).
pub const LAYOUT: u32 = 0xF << 21;
/// A list word's flags: everything above the object's offset (bits 20-0).
pub const FLAGS: u32 = 0xFFE0_0000;
/// The list word's triangle mask and what sits above the layout field
/// (bits 31-25).
pub const MASK: u32 = 0xFE00_0000;
/// What test G's pictures are filled with before each render: no pixel of
/// the quad has alpha 0x5A, so one left is a pixel the render never wrote.
pub const SENTINEL: u32 = 0x5A5A_5A5A;

/// One TA draw of [`QUAD`]'s `corners` (at most four) as a strip, as the
/// GLES driver sends one (ADR-0024 M5): the state block ([`TA_STATE`],
/// `tsp`, `isp`, `clip`, the bit-6 word 0), the strip word, then x, y, z,
/// 1/w and colour per vertex (z and 1/w 1.0, as `rp-glesfeat` M's draws).
/// Then a `1` goes to the end window. The words and how many.
#[must_use]
pub fn ta_draw(tsp: u32, isp: u32, clip: u32, corners: &[usize]) -> ([u32; TA_DRAW_MAX], usize) {
    let mut s = [0u32; TA_DRAW_MAX];
    s[..6].copy_from_slice(&[TA_STATE, tsp, isp, clip, 0, TA_STRIP]);
    let one = 1.0f32.to_bits();
    let mut n = 6;
    for &k in corners.iter().take(4) {
        let (x, y, c) = QUAD[k];
        s[n..n + TA_VERTEX_WORDS].copy_from_slice(&[x.to_bits(), y.to_bits(), one, one, c]);
        n += TA_VERTEX_WORDS;
    }
    (s, n)
}

/// Way 3's list flags: `ours` with the TA's layout field (bits 24-21), and
/// with every one of the TA's flags (bits 31-21) when its bits 31-25 (the
/// triangle mask) are ours, so a one-triangle object stays one triangle.
/// Our offset (bits 20-0) is kept. The word, and whether all of the TA's
/// flags were taken.
#[must_use]
pub const fn adopt_flags(ours: u32, ta: u32) -> (u32, bool) {
    if ta & MASK == ours & MASK {
        ((ta & FLAGS) | (ours & !FLAGS), true)
    } else {
        ((ours & !LAYOUT) | (ta & LAYOUT), false)
    }
}

/// Test G2 (2026-10-23, after test G): the TA's strip of four was nearly
/// right at divide 1 where our two triangles were wrong in every render;
/// G2 tries each difference alone through our direct path. The list
/// flags the TA wrote for the strip of four (`0x60A098A0`): triangles 0
/// and 1 (mask bits 30 and 29), layout 5.
pub const QUAD_FLAGS: u32 = 0x60A0_0000;
/// Words in the TA's four-corner Gouraud object: ISP, TSP, x/y x4, z x4,
/// 1/w x4, colour x4.
pub const QUAD_WORDS: usize = 22;
/// The strip's corner order (of [`QUAD`]): top left, top right, bottom
/// left, bottom right.
pub const STRIP_ORDER: [usize; 4] = [0, 1, 3, 2];
/// The GLES clear's TSP (no colour per vertex: its colour from the state).
pub const TSP_CLEAR: u32 = 0x2220_6F80;
/// The list flags the TA wrote for the clear (`0x40012E92`): triangle 0,
/// layout 0.
pub const CLEAR_FLAGS: u32 = 0x4000_0000;
/// Words in the TA's clear object: TSP, x/y x3, z x3 (no ISP, no 1/w, no
/// colours).
pub const CLEAR_WORDS: usize = 10;

/// [`QUAD`] as the one object the TA writes for a strip of four, at depth
/// `z` (the TA's: the z it was sent; 1/w 1.0).
#[must_use]
pub fn quad_object(isp: u32, tsp: u32, z: f32) -> [u32; QUAD_WORDS] {
    four_corner_object(isp, tsp, z, STRIP_ORDER.map(|k| QUAD[k]))
}

/// A four-corner object (the TA's form for a strip of four, list flags
/// [`QUAD_FLAGS`]) of `corners` `(x, y, colour)` in strip order, at depth
/// `z`, 1/w 1.0.
#[must_use]
pub fn four_corner_object(
    isp: u32,
    tsp: u32,
    z: f32,
    corners: [(f32, f32, u32); 4],
) -> [u32; QUAD_WORDS] {
    let mut w = [0u32; QUAD_WORDS];
    w[0] = isp;
    w[1] = tsp;
    for (n, &(x, y, c)) in corners.iter().enumerate() {
        w[2 + 2 * n] = x.to_bits();
        w[3 + 2 * n] = y.to_bits();
        w[10 + n] = z.to_bits();
        w[14 + n] = 1.0f32.to_bits();
        w[18 + n] = c;
    }
    w
}

/// Test G3 (2026-10-23, after G2): the colour clear as Dell's GLES driver
/// draws it (static reading, `libGLES_CL.dll` A08): `glClearColorx`
/// (`0x0225E380`) packs the colour as ARGB8888 into context `+0x5F4`;
/// `glClear(GL_COLOR_BUFFER_BIT)` (`0x0225F064`) only marks a clear
/// pending (`+0x6D8` bit 0, the colour copied to `+0x6EC`); the next draw
/// sends it (`0x0226E744`): this state block (TSP, ISP, the bit-6 word: no
/// clip word), then one strip of four over the viewport.
pub const DELL_CLEAR_STATE: u32 = 0x1000_0043;
/// The clear's TSP: Dell's flat opaque TSP with a colour per vertex
/// (`0x22200E00` when a depth clear goes with it; bit 8 set when the colour
/// isn't cleared).
pub const DELL_CLEAR_TSP: u32 = 0x2220_0E80;
/// The clear's corners, in the order Dell's driver sends them: top left,
/// bottom left, top right, bottom right (x from `+0x6DC`/`+0x6E4`, y from
/// `+0x6E0`/`+0x6E8`).
#[must_use]
pub fn dell_clear_corners(width: u32, height: u32, argb: u32) -> [(f32, f32, u32); 4] {
    let (w, h) = (width as f32, height as f32);
    [
        (0.0, 0.0, argb),
        (0.0, h, argb),
        (w, 0.0, argb),
        (w, h, argb),
    ]
}

/// Words in [`dell_clear_draw`].
pub const DELL_CLEAR_WORDS: usize = 5 + 4 * TA_VERTEX_WORDS;

/// The colour clear as Dell's driver streams it to the TA: the state block
/// (`isp`, the bit-6 word 0), the strip word, four 5-word vertices (x, y,
/// the clear depth `z`, 1/w 1.0, the colour). Then a `1` goes to the end
/// window.
#[must_use]
pub fn dell_clear_draw(
    isp: u32,
    width: u32,
    height: u32,
    z: f32,
    argb: u32,
) -> [u32; DELL_CLEAR_WORDS] {
    let mut s = [0u32; DELL_CLEAR_WORDS];
    s[..5].copy_from_slice(&[DELL_CLEAR_STATE, DELL_CLEAR_TSP, isp, 0, TA_STRIP]);
    for (n, (x, y, c)) in dell_clear_corners(width, height, argb)
        .into_iter()
        .enumerate()
    {
        let at = 5 + n * TA_VERTEX_WORDS;
        s[at..at + TA_VERTEX_WORDS].copy_from_slice(&[
            x.to_bits(),
            y.to_bits(),
            z.to_bits(),
            1.0f32.to_bits(),
            c,
        ]);
    }
    s
}

/// Test G3's settings, rising: the core PLL (13 MHz x M / (N x 2^P)) with
/// the cores at divide 1: 74.75 MHz (Dell's `CoreClock` threshold, where
/// Dell's driver draws right), 99.67 (Windows Mobile's system clock) and
/// 133.25 (the aim; the memory right there, GC2). Each with the memory's
/// timing from Dell's rule but its CAS latency [`G_LMTYPE`]'s.
pub const G3_PLLS: [(u32, &str); 3] = [
    (23 << 7 | 4 << 4 | 1, "74.75"),
    (23 << 7 | 3 << 4 | 1, "99.67"),
    (41 << 7 | 4 << 4 | 1, "133.25"),
];

/// The local memory's CAS latency at every divide-1 setting: CL2 (`LMTYPE`
/// `0xAC8`), as GC2 and the soaks run, not Dell's CL3 above 100 MHz: run
/// G1 drew no 3D picture at 108 MHz with CL3 where run 27 drew right at
/// 133 MHz with CL2, and the warm and cold soaks ran 133.25 MHz CL2 with 0
/// BAD. Test G3's first run took Dell's rule and no render finished at
/// 133.25 / 1 (2026-10-23): the test's fault, not the chip's.
pub const G_LMTYPE: u32 = 0xAC8;

/// Test G4 (2026-10-23, after G3): G3 found Dell's own object sequence
/// wrong at divide 1 under our register set-up, where Dell's driver draws
/// it right. Dell's render table at 74.75 MHz / 1 (`rp-wmprobe` run 2,
/// `local/rp-wmprobe-5.txt`, read after Dell's draws), the words that differ
/// from ours (`rp_drv_mbx3d::start_table`), the target's two aside.
pub const DELL_TABLE: [(usize, u32); 7] = [
    (0x610, 0x0003_0001),
    (0x614, 0x1E3C_E500),
    (0x618, 0x1E3C_E500),
    (0x61C, 0x0002_0007),
    (0x648, 1),
    (0x6D8, 0x0900_0000),
    (0x6DC, 0x3F80_0001),
];
/// Dell's target: RGB565 (`0x650` 1) at its stride in words (`0x660`, 480
/// pixels x 2 bytes / 4).
pub const DELL_TARGET16: [(usize, u32); 2] = [(0x650, 1), (0x660, 480 * 2 / 4)];

/// The GLES clear of a `width` x `height` target as the TA writes its
/// object: one triangle covering the target twice over, z 1.0.
#[must_use]
pub fn clear_object(width: u32, height: u32) -> [u32; CLEAR_WORDS] {
    let (w, h) = (2.0 * width as f32, 2.0 * height as f32);
    let one = 1.0f32.to_bits();
    [
        TSP_CLEAR,
        0,
        0,
        w.to_bits(),
        0,
        0,
        h.to_bits(),
        one,
        one,
        one,
    ]
}

/// Whether triangle `t` touches the rectangle `(x0, y0)`-`(x1, y1)`
/// (edges included): the per-tile lists G2 builds as the TA does, an
/// object listed only in the tiles it touches. No triangle edge has every
/// corner strictly outside it, and the bounding boxes overlap.
#[must_use]
pub fn touches(t: [(f32, f32); 3], (x0, y0, x1, y1): (f32, f32, f32, f32)) -> bool {
    let (lo_x, hi_x) = (
        t[0].0.min(t[1].0).min(t[2].0),
        t[0].0.max(t[1].0).max(t[2].0),
    );
    let (lo_y, hi_y) = (
        t[0].1.min(t[1].1).min(t[2].1),
        t[0].1.max(t[1].1).max(t[2].1),
    );
    if hi_x < x0 || lo_x > x1 || hi_y < y0 || lo_y > y1 {
        return false;
    }
    let cross = |a: (f32, f32), b: (f32, f32), p: (f32, f32)| {
        (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0)
    };
    let s = cross(t[0], t[1], t[2]).signum();
    let corners = [(x0, y0), (x1, y0), (x0, y1), (x1, y1)];
    (0..3).all(|e| {
        let (a, b) = (t[e], t[(e + 1) % 3]);
        corners.iter().any(|&p| s * cross(a, b, p) >= 0.0)
    })
}

/// A picture held to its reference, word by word.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tally {
    /// Words that differ.
    pub wrong: u32,
    /// Of those, words with no channel more than one step off.
    pub one_step: u32,
    /// Of those, words still [`SENTINEL`]: never written.
    pub unwritten: u32,
    /// The largest difference in any one channel (alpha included; a wrap
    /// from 0xFF to 0x00 is 255).
    pub max_step: u32,
    /// The first wrong word: (its index, the reference's, the picture's).
    pub first: Option<(u32, u32, u32)>,
    /// Of the wrong words, those on the picture's outer edge (from
    /// [`Tally::add_px`]).
    pub edge: u32,
    /// The first wrong word off the edge, as `first`.
    pub first_inner: Option<(u32, u32, u32)>,
}

impl Tally {
    /// Word `i`: `want` from the reference, `got` from the picture.
    pub fn add(&mut self, i: u32, want: u32, got: u32) {
        if want == got {
            return;
        }
        self.wrong += 1;
        if got == SENTINEL {
            self.unwritten += 1;
        }
        let step = (0..4)
            .map(|c| ((want >> (8 * c)) & 0xFF).abs_diff((got >> (8 * c)) & 0xFF))
            .max()
            .unwrap_or(0);
        if step <= 1 {
            self.one_step += 1;
        }
        self.max_step = self.max_step.max(step);
        if self.first.is_none() {
            self.first = Some((i, want, got));
        }
    }

    /// Pixel `(x, y)` of a `w` x `h` picture, as [`Tally::add`], its place
    /// counted too (on the outer edge or not).
    pub fn add_px(&mut self, (x, y): (u32, u32), (w, h): (u32, u32), want: u32, got: u32) {
        if want == got {
            return;
        }
        let i = y * w + x;
        self.add(i, want, got);
        if x == 0 || y == 0 || x + 1 == w || y + 1 == h {
            self.edge += 1;
        } else if self.first_inner.is_none() {
            self.first_inner = Some((i, want, got));
        }
    }

    /// `o` added to this one (the first wrong words kept if set).
    pub fn merge(&mut self, o: &Self) {
        self.wrong += o.wrong;
        self.one_step += o.one_step;
        self.unwritten += o.unwritten;
        self.edge += o.edge;
        self.max_step = self.max_step.max(o.max_step);
        if self.first.is_none() {
            self.first = o.first;
        }
        if self.first_inner.is_none() {
            self.first_inner = o.first_inner;
        }
    }
}

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_drv_blit2d::{self as cmd, Engine, Format, MBX, reg};
    use rp_drv_mbx_display::clock::{self, Speed, with_3d};
    use rp_drv_mbx_display::font::{self, WIDTH};
    use rp_drv_mbx_display::{Display, PixelFormat, Plane, layers::Layer, reg as dreg};
    use rp_drv_mbx3d::{self as m3, ISP_ALWAYS, PM_READY, RENDER_DONE, TSP_DELL, Vertex};
    use rp_drv_pxa_keypad::{Keypad, Pressed};
    use rp_hal_api::RegisterBus;
    use rp_hal_mem::{load32_volatile, store32_volatile};
    use rp_hal_pxa27x::{Mmio, cpu, diag, rt};
    use rp_hal_x51v::devices::RunLog;

    use super::{
        MARKER, QUAD, SENTINEL, TRIS, TSP_ALPHA_TEST, TSP_SMOOTH, Tally, adopt_flags, alpha_word,
        ta_draw,
    };

    use rp_image::crash;

    rp_image::entry!(main);

    macro_rules! log {
        ($($t:tt)*) => {{ let _ = writeln!(RunLog, $($t)*); }};
    }

    fn us_since(t0: u32) -> u32 {
        rt::ticks().wrapping_sub(t0) / 13 * 4
    }

    // 2700G local memory, as rp-mbx3d (proven there): target, parameter
    // memory and its manager's block, region array, a texture, the fence.
    const TARGET: usize = 0x0020_0000;
    const W: u32 = 480;
    const H: u32 = 640;
    const STRIDE: usize = 480 * 4;
    const PM: usize = 0x0060_0000;
    const PM_SIZE: u32 = 0x0010_0000;
    const PM_TABLE: usize = 0x0070_0000;
    const _: () = assert!(PM + PM_SIZE as usize <= m3::PM_REACH);
    const PARAMS: usize = PM;
    const OBJECT: usize = PARAMS + 64;
    const REGIONS: usize = 0x00D1_0000;
    const TEX: usize = 0x00E0_0000;
    /// Test M's second texture (8 x 8, one byte a texel).
    const TEX_B: usize = TEX + 0x1_0000;
    /// The TA's tail buffer (128 KiB aligned, 4 bytes a tile) and the
    /// region array it writes (as `rp-mbx3d` versions 14-18).
    const TAIL: usize = 0x00A0_0000;
    const TAIL_SIZE: usize = 0x1_0000;
    const TA_REGIONS: usize = 0x00A2_0000;
    const FENCE: usize = 0x00FF_F000;
    const COLS: u32 = W / m3::TILE_W;
    const ROWS: u32 = H / m3::TILE_H;

    const STATUS: usize = MBX + reg::STATUS;
    const ENABLE: usize = MBX + reg::ENABLE;
    const CLEAR: usize = MBX + reg::CLEAR;
    const MBXCLK: usize = rp_drv_mbx_display::clock::MBXCLK;
    const PLLSTAT: usize = 0x040;

    const GREY: u32 = 0xFF20_2020;
    const STRIP: u32 = m3::STRIP_ONE_TRIANGLE;

    /// Test G alone instead of A, D, L and M (`RP_GLESFEAT_G=1`; device
    /// build `RP_GLESFEAT_G=1 cargo xtask codegen-variant rp-glesfeat g`).
    const TEST_G: bool = option_env!("RP_GLESFEAT_G").is_some() || TEST_G2 || TEST_G3 || TEST_G4;
    /// Test G2 instead of G (`RP_GLESFEAT_G2=1`; device build
    /// `RP_GLESFEAT_G2=1 cargo xtask codegen-variant rp-glesfeat g2`).
    const TEST_G2: bool = option_env!("RP_GLESFEAT_G2").is_some();
    /// Test G3 instead of G (`RP_GLESFEAT_G3=1`; device build
    /// `RP_GLESFEAT_G3=1 cargo xtask codegen-variant rp-glesfeat g3`).
    const TEST_G3: bool = option_env!("RP_GLESFEAT_G3").is_some();
    /// Test G4 instead of G (`RP_GLESFEAT_G4=1`; device build
    /// `RP_GLESFEAT_G4=1 cargo xtask codegen-variant rp-glesfeat g4`).
    const TEST_G4: bool = option_env!("RP_GLESFEAT_G4").is_some();
    /// The label's title and the log's way prefix.
    const G_TITLE: &str = if TEST_G4 {
        "RP-GLESFEAT TEST G4"
    } else if TEST_G3 {
        "RP-GLESFEAT TEST G3"
    } else if TEST_G2 {
        "RP-GLESFEAT TEST G2"
    } else {
        "RP-GLESFEAT TEST G"
    };
    const G_NAME: &str = if TEST_G4 {
        "G4"
    } else if TEST_G3 {
        "G3"
    } else if TEST_G2 {
        "G2"
    } else {
        "G"
    };
    /// Draws per way at divide 1 (GC2's rule 6).
    const G_DRAWS: u32 = 20;
    /// Test G's reference picture (ARGB8888, as the target).
    const REF: usize = 0x0080_0000;
    const _: () = assert!(REF + STRIDE * H as usize <= TAIL);
    /// The label scanned out from the first moment: RGB565 (a 32-bit
    /// screen corrupts 32-bit renders: ADR-0023), after the target.
    const LABEL: usize = 0x0040_0000;
    const LABEL_STRIDE: usize = 480 * 2;
    const _: () = assert!(TARGET + STRIDE * H as usize <= LABEL);
    const _: () = assert!(LABEL + LABEL_STRIDE * H as usize <= PM);
    /// The label's text: lines, the font's scale, colours (RGB565).
    const LABEL_LINES: usize = 16;
    const LABEL_SCALE: usize = 3;
    const LABEL_LINE_H: usize = 32;
    const LABEL_TOP: usize = 24;
    const INK: u32 = 0xFFFF;
    const PAPER: u32 = 0x0010;
    /// CS3 timing (RDF, RDN, RRR): Windows Mobile's, and Dell's tier for
    /// 25-99 MHz (`rp-gpubench`'s `CS3_WM` and `CS3_LONG`).
    const CS3_WM: (u32, u32, u32) = (8, 4, 2);
    const CS3_LONG: (u32, u32, u32) = (13, 9, 7);

    fn store(vram: &mut [u8], at: usize, words: &[u32]) {
        for (k, &w) in words.iter().enumerate() {
            let _ = store32_volatile(vram, at + 4 * k, w);
        }
    }

    fn px(vram: &[u8], x: u32, y: u32) -> u32 {
        load32_volatile(vram, TARGET + y as usize * STRIDE + 4 * x as usize).unwrap_or(0xDEAD_BEEF)
    }

    /// Writes `pairs` with `bit` enabled; true if the bit set in time.
    fn run(e: &mut Engine<Mmio>, pairs: &[(usize, u32)], bit: u32, limit_us: u32) -> bool {
        let en = e.regs.read32(ENABLE);
        e.regs.write32(CLEAR, bit);
        e.regs.write32(ENABLE, en | bit);
        let t0 = rt::ticks();
        for &(off, v) in pairs {
            e.regs.write32(MBX + off, v);
        }
        let mut set = false;
        while us_since(t0) < limit_us {
            if e.regs.read32(STATUS) & bit != 0 {
                set = true;
                break;
            }
        }
        e.regs.write32(CLEAR, bit);
        e.regs.write32(ENABLE, en);
        set
    }

    fn pm_init(e: &mut Engine<Mmio>, vram: &mut [u8]) -> bool {
        for k in 0..4096 {
            let _ = store32_volatile(vram, PM_TABLE + 4 * k, 0);
        }
        cpu::drain_write_buffer();
        let t = m3::pm_init_table(PM as u32, PM_SIZE, PM_TABLE as u32);
        run(e, &t, PM_READY, 100_000)
    }

    /// Every tile listing the one object list at the parameter buffer.
    #[inline(never)]
    fn regions(vram: &mut [u8]) {
        let total = COLS * ROWS;
        for k in 0..total {
            let entry = m3::region_entry(k % COLS, k / COLS, PARAMS as u32, k + 1 == total);
            store(vram, REGIONS + 8 * k as usize, &entry);
        }
        cpu::drain_write_buffer();
    }

    /// Dell's start table with the target register (rp-mbx3d's level 1).
    fn table() -> ([(usize, u32); 16], usize) {
        let dell = m3::start_table(REGIONS as u32, PARAMS as u32, W, H, STRIDE as u32);
        let mut t = [(0usize, 0u32); 16];
        t[..14].copy_from_slice(&dell[..14]);
        t[14] = (0x65C, TARGET as u32);
        t[15] = dell[14];
        (t, 16)
    }

    /// `objs` (words, list pointer flags) in order, rendered; true if it
    /// finished. A timeout resets the core (version 21's lesson).
    #[inline(never)]
    fn scene(e: &mut Engine<Mmio>, vram: &mut [u8], objs: &[(&[u32], u32)]) -> bool {
        scene_with(e, vram, objs, &[])
    }

    /// [`scene`] with the start table's words `over` written instead of
    /// ours (test G4: Dell's render table). Each must be a register the
    /// table writes already; one that isn't is logged and left out.
    #[inline(never)]
    fn scene_with(
        e: &mut Engine<Mmio>,
        vram: &mut [u8],
        objs: &[(&[u32], u32)],
        over: &[(usize, u32)],
    ) -> bool {
        let mut at = OBJECT;
        let mut list = [m3::END; 16];
        let n = objs.len().min(15);
        for (k, (w, flags)) in objs[..n].iter().enumerate() {
            store(vram, at, w);
            list[k] = flags | ((at - PARAMS) / 4) as u32;
            at += 4 * w.len();
        }
        store(vram, PARAMS, &list[..=n]);
        regions(vram);
        let (mut t, len) = table();
        for &(off, v) in over {
            // The last pair starts the render: never replaced.
            match t[..len - 1].iter_mut().find(|p| p.0 == off) {
                Some(p) => p.1 = v,
                None => log!("  (0x{off:x} isn't in the start table: not written)"),
            }
        }
        let done = run(e, &t[..len], RENDER_DONE, 200_000);
        if !done {
            e.reset(rt::delay_us);
            let ok = pm_init(e, vram);
            log!("  (timed out: core reset, memory manager ready again: {ok})");
        }
        done
    }

    fn background(z: f32, tsp: u32) -> [u32; m3::TRIANGLE_WORDS] {
        let v = |x, y| Vertex {
            x,
            y,
            z,
            rhw: 1.0,
            argb: GREY,
        };
        m3::triangle(
            ISP_ALWAYS,
            tsp,
            &[v(0.0, 0.0), v(2048.0, 0.0), v(0.0, 2048.0)],
        )
    }

    /// A flat square (two triangles), `(x0, y0)` to `(x1, y1)`.
    #[allow(clippy::too_many_arguments)]
    fn square(
        (x0, y0, x1, y1): (f32, f32, f32, f32),
        argb: u32,
        z: f32,
        isp: u32,
        tsp: u32,
    ) -> [[u32; m3::TRIANGLE_WORDS]; 2] {
        let v = |x, y| Vertex {
            x,
            y,
            z,
            rhw: 1.0,
            argb,
        };
        [
            m3::triangle(isp, tsp, &[v(x0, y0), v(x1, y0), v(x0, y1)]),
            m3::triangle(isp, tsp, &[v(x1, y0), v(x1, y1), v(x0, y1)]),
        ]
    }

    /// A: the alpha test on a red square with vertex alpha 64.
    #[inline(never)]
    fn alpha(e: &mut Engine<Mmio>, vram: &mut [u8]) {
        let bg = background(0.0, TSP_DELL);
        let cases: [(&str, u32, u32); 7] = [
            ("always, ref 0 (control)", alpha_word(7, 0), TSP_DELL),
            ("greater 128", alpha_word(4, 128), TSP_DELL),
            (
                "greater 128, TSP bit 14",
                alpha_word(4, 128),
                TSP_DELL | TSP_ALPHA_TEST,
            ),
            ("greater 32", alpha_word(4, 32), TSP_DELL),
            ("less 128", alpha_word(1, 128), TSP_DELL),
            (
                "less 128, TSP bit 14",
                alpha_word(1, 128),
                TSP_DELL | TSP_ALPHA_TEST,
            ),
            (
                "less 0 (version 23's depth 'less')",
                alpha_word(1, 0),
                TSP_DELL,
            ),
        ];
        for (name, isp, tsp) in cases {
            rt::check_in();
            let sq = square((100.0, 100.0, 300.0, 300.0), 0x40FF_0000, 0.0, isp, tsp);
            let done = scene(e, vram, &[(&bg, STRIP), (&sq[0], STRIP), (&sq[1], STRIP)]);
            log!(
                "  A {name}: ISP 0x{isp:08x} TSP 0x{tsp:08x}: done {done}; square (200,200) 0x{:08x}, outside (20,20) 0x{:08x}",
                px(vram, 200, 200),
                px(vram, 20, 20)
            );
        }
    }

    /// D: version 23's depth test with the alpha bits "always".
    #[inline(never)]
    fn depth(e: &mut Engine<Mmio>, vram: &mut [u8]) {
        let tsp = |mode: u32| (TSP_DELL & !0x0F80) | mode << 9;
        let bg = background(0.1, tsp(7));
        let red = square(
            (100.0, 100.0, 300.0, 300.0),
            0xFFFF_0000,
            0.5,
            ISP_ALWAYS,
            tsp(7),
        );
        for (zb, tenths) in [(0.3f32, 3u32), (0.8, 8)] {
            for (mode, isp, form) in [
                (1u32, ISP_ALWAYS, "alpha always"),
                (3, ISP_ALWAYS, "alpha always"),
                (4, ISP_ALWAYS, "alpha always"),
                (1, 1 << 29, "version 23's form"),
            ] {
                rt::check_in();
                let blue = square(
                    (200.0, 200.0, 400.0, 400.0),
                    0xFF00_00FF,
                    zb,
                    isp,
                    tsp(mode),
                );
                let done = scene(
                    e,
                    vram,
                    &[
                        (&bg, STRIP),
                        (&red[0], STRIP),
                        (&red[1], STRIP),
                        (&blue[0], STRIP),
                        (&blue[1], STRIP),
                    ],
                );
                log!(
                    "  D blue z 0.{tenths} mode {mode} ({form}), ISP 0x{isp:08x} TSP 0x{:08x}: done {done}; overlap (250,250) 0x{:08x}, blue alone (350,350) 0x{:08x}, red alone (150,150) 0x{:08x}",
                    tsp(mode),
                    px(vram, 250, 250),
                    px(vram, 350, 350),
                    px(vram, 150, 150)
                );
            }
        }
    }

    /// L: an 8 x 8 texture of one byte value at format code 10.
    #[inline(never)]
    fn luminance(e: &mut Engine<Mmio>, vram: &mut [u8]) {
        let bg = background(0.0, TSP_DELL);
        let v = |x, y, u, v| m3::tex::Vertex {
            x,
            y,
            z: 0.0,
            rhw: 1.0,
            argb: 0xFFFF_FFFF,
            u,
            v,
        };
        for (code, byte, tex2) in [
            (10u32, 0x60u32, 0x8608_4610u32),
            (10, 0xC0, 0x8608_4610),
            (10, 0x60, 0x8608_6610),
        ] {
            rt::check_in();
            for k in 0..16 {
                let _ = store32_volatile(vram, TEX + 4 * k, byte * 0x0101_0101);
            }
            cpu::drain_write_buffer();
            let mut texw = m3::ta::dell_texture(TEX as u32);
            texw[0] = m3::tex::tex0(0, code);
            texw[2] = tex2;
            let tsp = m3::ta::TSP_DELL_TEXTURED;
            let a = m3::tex::triangle(
                ISP_ALWAYS,
                texw,
                tsp,
                &[
                    v(100.0, 100.0, 0.0, 0.0),
                    v(300.0, 100.0, 1.0, 0.0),
                    v(100.0, 300.0, 0.0, 1.0),
                ],
            );
            let b = m3::tex::triangle(
                ISP_ALWAYS,
                texw,
                tsp,
                &[
                    v(300.0, 100.0, 1.0, 0.0),
                    v(300.0, 300.0, 1.0, 1.0),
                    v(100.0, 300.0, 0.0, 1.0),
                ],
            );
            let t = m3::tex::STRIP_GAME;
            let done = scene(e, vram, &[(&bg, STRIP), (&a, t), (&b, t)]);
            log!(
                "  L code {code}, texels 0x{byte:02x}, TEX2 0x{tex2:08x}: done {done}; square (200,200) 0x{:08x}, (120,280) 0x{:08x}",
                px(vram, 200, 200),
                px(vram, 120, 280)
            );
        }
    }

    /// The longest stream [`through_ta`] sends.
    const TA_WORDS: usize = 96;
    /// The most draws in one [`through_ta`].
    const TA_DRAWS: usize = 4;
    /// The most list words [`through_ta`] reads from the first tile.
    const TA_LIST: usize = 6;

    /// What [`through_ta`] did.
    struct TaOut {
        /// The TA said done.
        ta: bool,
        /// The render after it finished.
        done: bool,
        /// The first tile's list words up to its end word ([`m3::END`]
        /// when fewer).
        list: [u32; TA_LIST],
    }

    /// `draws` through the tile accelerator (`rp-mbx3d` version 18's
    /// path: its buffers zeroed, its start table, each draw into its
    /// window followed by a `1` into the end window (each window's offset
    /// counting on, as Dell's driver counts), the scene's end), then, if
    /// the TA said done, the 3D stage on the TA's region array. With
    /// `words`, logs the first tile's list and each object the TA wrote
    /// (`words` of each).
    #[inline(never)]
    fn through_ta(e: &mut Engine<Mmio>, vram: &mut [u8], draws: &[&[u32]], words: usize) -> TaOut {
        let mut out = TaOut {
            ta: false,
            done: false,
            list: [m3::END; TA_LIST],
        };
        for k in 0..TAIL_SIZE / 4 {
            let _ = store32_volatile(vram, TAIL + 4 * k, 0);
        }
        for k in 0..2 * (COLS * ROWS + 1) as usize {
            let _ = store32_volatile(vram, TA_REGIONS + 4 * k, 0);
        }
        cpu::drain_write_buffer();
        let t = m3::ta::start_table(TAIL as u32, TA_REGIONS as u32, W, H, COLS, ROWS);
        let mut w = [(0usize, 0u32); 9 + TA_WORDS + TA_DRAWS + 2];
        w[..9].copy_from_slice(&t);
        let (mut at, mut n, mut ends) = (9, 0, 0);
        for d in draws.iter().take(TA_DRAWS) {
            if n + d.len() > TA_WORDS {
                log!("    the draws exceed {TA_WORDS} words: the rest not sent");
                break;
            }
            for &v in d.iter() {
                w[at] = (m3::ta::PORT - MBX + 4 * n, v);
                at += 1;
                n += 1;
            }
            w[at] = (m3::ta::END_PORT - MBX + 4 * ends, 1);
            at += 1;
            ends += 1;
        }
        let r85c = t[7];
        w[at] = (r85c.0, r85c.1 | m3::ta::SCENE_END);
        w[at + 1] = (m3::ta::PORT - MBX + 4 * n, m3::END);
        out.ta = run(e, &w[..at + 2], m3::ta::DONE, 200_000);
        e.regs.write32(MBX + r85c.0, r85c.1);
        if !out.ta {
            log!("    the TA never said done: no render");
            return out;
        }
        // The first tile the TA put something in, its list, its objects.
        let tile = (0..(COLS * ROWS) as usize)
            .map(|k| TA_REGIONS + 8 * k)
            .find(|&a| load32_volatile(vram, a + 4).unwrap_or(0) != 0)
            .unwrap_or(TA_REGIONS);
        let list = load32_volatile(vram, tile + 4).unwrap_or(0) as usize & m3::ADDR_MASK as usize;
        for (k, slot) in out.list.iter_mut().enumerate() {
            let ptr = load32_volatile(vram, list + 4 * k).unwrap_or(m3::END);
            *slot = ptr;
            if ptr == m3::END {
                break;
            }
            if words == 0 {
                continue;
            }
            let obj = PARAMS + 4 * (ptr & 0x001F_FFFF) as usize;
            let _ = write!(
                RunLog,
                "    TA list word 0x{ptr:08x} (layout {}), object at 0x{obj:x}:",
                (ptr & super::LAYOUT) >> 21
            );
            for j in 0..words {
                let _ = write!(
                    RunLog,
                    " {:08x}",
                    load32_volatile(vram, obj + 4 * j).unwrap_or(0)
                );
            }
            log!("");
        }
        let (mut t3, len) = table();
        for p in &mut t3[..len] {
            if p.0 == 0x608 {
                p.1 = TA_REGIONS as u32;
            }
        }
        out.done = run(e, &t3[..len], RENDER_DONE, 200_000);
        if !out.done {
            e.reset(rt::delay_us);
            let ok = pm_init(e, vram);
            log!("    (render timed out: core reset, memory manager ready again: {ok})");
        }
        out
    }

    /// M: the second texture unit (the performance gate's P5): two flat
    /// 8 x 8 grey textures (code 10), 0x80 in unit 0 and 0x40 in unit 1,
    /// through the TA with the state block's mask bit 3 (unit 1's three
    /// words after unit 0's) and vertices of 9 words (u, v for each unit).
    /// Unit 1 modulating gives about 0x20; ignored, 0x80. A one-unit draw
    /// first (the control). The object the TA wrote is logged: the direct
    /// form's layout for two units.
    #[inline(never)]
    fn two_units(e: &mut Engine<Mmio>, vram: &mut [u8]) {
        for k in 0..16 {
            let _ = store32_volatile(vram, TEX + 4 * k, 0x8080_8080);
            let _ = store32_volatile(vram, TEX_B + 4 * k, 0x4040_4040);
        }
        cpu::drain_write_buffer();
        let tex = |addr: usize, tex2: u32| {
            let mut w = m3::ta::dell_texture(addr as u32);
            w[0] = m3::tex::tex0(0, 10);
            w[2] = tex2;
            w
        };
        // Unit 0 as test L (texture's grey and alpha); unit 1 MODULATE (the
        // GLES table's unit-1 form: bits 11 and 7 set).
        let a = tex(TEX, 0x8608_4610);
        let clip = m3::ta::clip(W, H);
        let corners = [
            (100.0f32, 100.0f32, 0.0f32, 0.0f32),
            (380.0, 100.0, 1.0, 0.0),
            (100.0, 540.0, 0.0, 1.0),
        ];
        // Version 2 (2026-10-22): mask bit 3 alone left the TA reading
        // 7-word vertices. TSP bits 19-17 (1 in Dell's textured draw, 2 in
        // DoomGLES's) and bit 16 (set by the GLES driver from a second
        // field) are the candidates for "texture coordinate sets".
        let dell = m3::ta::TSP_DELL_TEXTURED & !(0xF << 16);
        // Version 2 found TSP bit 16 = a second coordinate set (the TA
        // built two-unit objects, list layout 12) but each such render hung,
        // and every render after the first hang hung too (the recovery, not
        // the case). Version 3: bit 16 with unit 1's combine word in three
        // forms (the GLES table's unit-1 MODULATE, unit 0's form, a unit-1
        // REPLACE), a one-unit control after each, to tell a case's hang
        // from a recovery that didn't.
        let two = dell | 1 << 17 | 1 << 16;
        let control = (
            "one unit (control)",
            1usize,
            m3::ta::TSP_DELL_TEXTURED,
            0u32,
        );
        for (name, units, tsp, tex2b) in [
            control,
            ("two units, unit 1 TEX2 0x86084e10", 2, two, 0x8608_4E10),
            control,
            ("two units, unit 1 TEX2 0x86084610", 2, two, 0x8608_4610),
            control,
            ("two units, unit 1 TEX2 0xd6087e10", 2, two, 0xD608_7E10),
            control,
        ] {
            let b = tex(TEX_B, tex2b);
            rt::check_in();
            let fill = rp_image::need(
                cmd::fill(
                    Format::BPP32,
                    TARGET,
                    STRIDE,
                    (0, 0, W as u16, H as u16),
                    GREY,
                ),
                "a 2D rectangle",
            );
            let _ = e.submit(&fill).and_then(|()| e.finish(vram, 200_000));
            let mut st = [0u32; 48];
            let mut n = 0;
            let mut put = |v: u32| {
                st[n] = v;
                n += 1;
            };
            put(if units == 2 { 0x1000_005F } else { 0x1000_0057 });
            put(tsp);
            put(ISP_ALWAYS);
            a.iter().for_each(|&v| put(v));
            if units == 2 {
                b.iter().for_each(|&v| put(v));
            }
            put(clip);
            put(0);
            put(m3::ta::STRIP);
            for &(x, y, u, v) in &corners {
                for word in [
                    x.to_bits(),
                    y.to_bits(),
                    1.0f32.to_bits(),
                    1.0f32.to_bits(),
                    0xFFFF_FFFF,
                    u.to_bits(),
                    v.to_bits(),
                ] {
                    put(word);
                }
                if units == 2 {
                    put(u.to_bits());
                    put(v.to_bits());
                }
            }
            let TaOut { ta, done, .. } = through_ta(e, vram, &[&st[..n]], 32);
            log!(
                "  M {name} (TSP 0x{tsp:08x}): {n} words, TA done {ta}, render done {done}; inside (150,200) 0x{:08x}, outside (400,600) 0x{:08x}",
                px(vram, 150, 200),
                px(vram, 400, 600)
            );
        }
    }

    /// `lines` on the label (upper case; each cut to the screen's width),
    /// the rest of its text area blank.
    #[inline(never)]
    fn say(vram: &mut [u8], lines: &[&str]) {
        let cell = (WIDTH + 1) * LABEL_SCALE;
        let mut row = [0u32; LABEL_STRIDE / 4];
        for l in 0..LABEL_LINES {
            let text = lines.get(l).copied().unwrap_or("");
            for py in 0..LABEL_LINE_H {
                row.fill(PAPER | PAPER << 16);
                let gy = py / LABEL_SCALE;
                if gy < font::HEIGHT {
                    for (k, ch) in text.chars().take(480 / cell).enumerate() {
                        let bits = font::glyph(ch.to_ascii_uppercase())[gy];
                        for gx in 0..WIDTH {
                            if bits & (1 << (WIDTH - 1 - gx)) == 0 {
                                continue;
                            }
                            for dx in 0..LABEL_SCALE {
                                let x = 8 + k * cell + gx * LABEL_SCALE + dx;
                                let shift = 16 * (x % 2);
                                row[x / 2] = (row[x / 2] & !(0xFFFF << shift)) | INK << shift;
                            }
                        }
                    }
                }
                let y = LABEL_TOP + l * LABEL_LINE_H + py;
                store(vram, LABEL + y * LABEL_STRIDE, &row);
            }
        }
        cpu::drain_write_buffer();
    }

    /// The label plane cleared and shown; the display's registers as they
    /// were.
    fn show_label(regs: &mut Mmio, vram: &mut [u8]) -> [(usize, u32); 5] {
        for k in 0..LABEL_STRIDE * H as usize / 4 {
            let _ = store32_volatile(vram, LABEL + 4 * k, PAPER | PAPER << 16);
        }
        cpu::drain_write_buffer();
        show(
            regs,
            Plane {
                format: PixelFormat::Rgb565,
                width: W as u16,
                height: H as u16,
                offset: LABEL,
                stride: LABEL_STRIDE,
            },
        )
    }

    /// The display's output signature (mbxfb's `DSIG`): 0 when nothing is
    /// sent, and for an all-black picture (run N), which the label never is.
    const DSIG: usize = 0x2184;
    /// `DINTRS` bit 4: the graphics stream's underrun; set, the display
    /// stops sending until it is cleared (run G4's dark display, 2700g.md).
    const GFX_UNDERRUN: u32 = 1 << 4;
    /// A frame at 60 Hz, rounded up.
    const FRAME_US: u32 = 16_700;

    /// The display after a clock change (quality audit C69): the underrun
    /// flag cleared Dell's way (read, written back without it), then the
    /// output signature two frames on and a frame after that. If nothing is
    /// sent, the label plane is set again and read once more. Logged with
    /// `when`; true if the display sends.
    fn display_check(regs: &mut Mmio, when: &str) -> bool {
        let s = rp_drv_mbx_display::ack_events(regs, GFX_UNDERRUN);
        rt::delay_us(2 * FRAME_US);
        let a = regs.read32(DSIG);
        rt::delay_us(FRAME_US);
        let b = regs.read32(DSIG);
        let mut sending = a != 0 && b != 0;
        let _ = write!(
            RunLog,
            "    display {when}: underrun {}, DSIG 0x{a:x} then 0x{b:x}",
            s & GFX_UNDERRUN != 0
        );
        if !sending {
            let _ = show(
                regs,
                Plane {
                    format: PixelFormat::Rgb565,
                    width: W as u16,
                    height: H as u16,
                    offset: LABEL,
                    stride: LABEL_STRIDE,
                },
            );
            let _ = rp_drv_mbx_display::ack_events(regs, GFX_UNDERRUN);
            rt::delay_us(2 * FRAME_US);
            let c = regs.read32(DSIG);
            sending = c != 0;
            let _ = write!(RunLog, "; the label plane set again: DSIG 0x{c:x}");
        }
        log!("; {}", if sending { "sending" } else { "NOT SENDING" });
        sending
    }

    /// Windows Mobile's clock and bus timing back, wherever test G left
    /// them (no state kept, so the crash path can call it): `rp-gpubench`'s
    /// `clock_restore`, which every clock run has used. The PLL is always
    /// written again.
    fn clock_back<B: RegisterBus>(regs: &mut B) {
        let _ = diag::set_cs3_vlio(CS3_LONG.0, CS3_LONG.1, CS3_LONG.2);
        let clk = regs.read32(MBXCLK);
        clock::restore(regs, (clk & 3) | 1 << 2);
        rt::delay_us(1_000);
        if let Some(a) = clock::set_speed(regs, Speed::WM, rt::delay_us)
            && a.rewrites + a.unwritten > 0
        {
            log!(
                "    put back: {} memory-controller writes written again, {} never held",
                a.rewrites,
                a.unwritten
            );
        }
        let _ = diag::set_cs3_vlio(CS3_WM.0, CS3_WM.1, CS3_WM.2);
    }

    /// `speed` (a core PLL and its memory timing) with the cores at divide
    /// 1, as `rp-gpubench`'s `clock_apply` steps to it: the long bus timing,
    /// the cores at divide 2, the PLL and its memory timing, then divide 1
    /// and the bus timing of the clock's tier (Dell's: the long one below
    /// 99 MHz). False if the PLL didn't lock or a memory-controller write
    /// never held (Windows Mobile's clock is then back).
    ///
    /// ADR-0035's rules this follows: 3, CL2 always (callers pass
    /// `Speed::ladder`; G3's first run took Dell's CL3 and rendered nothing
    /// at 133.25 MHz); 4, every memory-controller write read back and the
    /// step failed if one never holds (`clock::set_speed`); 8, every change
    /// followed by the underrun check and Dell's clear (the callers'
    /// `display_check`, quality audit C69).
    fn to_divide_1<B: RegisterBus>(regs: &mut B, speed: Speed) -> bool {
        let _ = diag::set_cs3_vlio(CS3_LONG.0, CS3_LONG.1, CS3_LONG.2);
        let clk = regs.read32(MBXCLK);
        clock::restore(regs, (clk & 3) | 1 << 2);
        rt::delay_us(1_000);
        let ok = match clock::set_speed(regs, speed, rt::delay_us) {
            Some(a) => {
                if a.rewrites + a.unwritten > 0 {
                    log!(
                        "    memory-controller writes: {} written again, {} never held",
                        a.rewrites,
                        a.unwritten
                    );
                }
                a.unwritten == 0
            }
            None => false,
        };
        if !ok {
            clock_back(regs);
            return false;
        }
        let clk = regs.read32(MBXCLK);
        clock::restore(regs, clk & 3);
        rt::delay_us(1_000);
        let cs3 = if speed.khz() < 99_000 {
            CS3_LONG
        } else {
            CS3_WM
        };
        let _ = diag::set_cs3_vlio(cs3.0, cs3.1, cs3.2);
        true
    }

    /// The clock registers as the 2700G holds them.
    fn clock_readback(regs: &mut Mmio) {
        log!(
            "    read back: COREPLL 0x{:x} MBXCLK 0x{:x} LMTYPE 0x{:x} LMTIM 0x{:x} LMREFRESH 0x{:x}",
            regs.read32(clock::COREPLL),
            regs.read32(MBXCLK),
            regs.read32(clock::LMTYPE),
            regs.read32(clock::LMTIM),
            regs.read32(clock::LMREFRESH)
        );
    }

    /// On a crash, first: Windows Mobile's clock and bus timing back.
    fn put_back() {
        if let Some((mut regs, _)) = rp_hal_x51v::devices::emergency_mbx() {
            clock_back(&mut regs);
        }
    }

    /// A clean start (GC2's rule 5): the 3D core reset and its memory
    /// manager set up again.
    fn clean(e: &mut Engine<Mmio>, vram: &mut [u8]) -> bool {
        e.reset(rt::delay_us);
        pm_init(e, vram)
    }

    /// The target filled with [`SENTINEL`] by the CPU (not the 2D engine,
    /// which runs on the clock under test).
    fn blank(vram: &mut [u8]) {
        for k in 0..(W * H) as usize {
            let _ = store32_volatile(vram, TARGET + 4 * k, SENTINEL);
        }
        cpu::drain_write_buffer();
    }

    /// The target held to the picture at `reference`.
    fn compare(vram: &[u8], reference: usize) -> Tally {
        let mut t = Tally::default();
        for i in 0..W * H {
            let at = 4 * i as usize;
            t.add_px(
                (i % W, i / W),
                (W, H),
                load32_volatile(vram, reference + at).unwrap_or(0),
                load32_volatile(vram, TARGET + at).unwrap_or(0xDEAD_BEEF),
            );
        }
        t
    }

    /// The picture at `at` hashed (FNV-1a over its words).
    fn picture_hash(vram: &[u8], at: usize) -> u32 {
        let mut hash = 0x811C_9DC5u32;
        for k in 0..(W * H) as usize {
            let v = load32_volatile(vram, at + 4 * k).unwrap_or(0);
            hash = (hash ^ v).wrapping_mul(0x0100_0193);
        }
        hash
    }

    /// The target copied to [`REF`].
    fn keep_reference(vram: &mut [u8]) {
        for k in 0..(W * H) as usize {
            let v = load32_volatile(vram, TARGET + 4 * k).unwrap_or(0);
            let _ = store32_volatile(vram, REF + 4 * k, v);
        }
        cpu::drain_write_buffer();
    }

    /// Test G's three ways (and a fourth after them): how each draws.
    #[derive(Clone, Copy)]
    enum Way {
        /// Through the TA: Dell's clear, then each triangle its own strip.
        Ta,
        /// Our direct objects: a black background, the two Gouraud
        /// triangles, every list word ours (layout 4).
        Ours,
        /// The same objects with the list word the TA wrote for its
        /// Gouraud object ([`adopt_flags`]).
        OursTaFlags(u32),
        /// Through the TA: the clear, then the quad as one strip of four
        /// (as a GLES program draws a quad).
        TaStrip4,
        /// Test G2: our direct path with the TA's choices taken one at a
        /// time.
        Direct(Form),
        /// Test G3: Dell's sequence through the TA as `glClear` then
        /// `glDrawArrays(GL_TRIANGLE_STRIP)` send it: the colour clear
        /// ([`super::dell_clear_draw`]), then the quad as one strip of four.
        TaDell,
    }

    /// Test G2's direct forms: which of the TA's choices it takes.
    #[derive(Clone, Copy)]
    struct Form {
        name: &'static str,
        /// One four-corner object (layout 5, [`super::quad_object`]), not
        /// two triangles.
        quad: bool,
        /// z 1.0 (the TA's), not 0.
        z1: bool,
        /// What lies under the quad.
        bg: Bg,
        /// Each tile lists only the objects that touch it (the TA's lists),
        /// not every object.
        tiled: bool,
        /// Start-table words written instead of ours (test G4; not with
        /// `tiled`).
        regs: &'static [(usize, u32)],
    }

    /// Backgrounds (all black and opaque but the clear's, which has no
    /// colour): G2 found ours behind the divide-1 errors and the clear's
    /// colourless object free of them; G3 finds which of ours's traits it
    /// is.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Bg {
        /// Ours: one flat triangle (Dell's TSP) to (2048, 2048), z 0.
        Giant,
        /// The same at z 1.0.
        GiantZ1,
        /// The GLES clear's colourless object (layout 0; G2's way 4).
        Clear,
        /// Dell's colour clear by hand: one flat four-corner object over
        /// the screen exactly, z 1.0 ([`super::dell_clear_corners`]).
        DellQuad,
        /// Two flat triangles over the screen exactly, z 0 (the size alone).
        ScreenTris,
        /// One Gouraud triangle to (2048, 2048), every corner black.
        Gouraud,
        /// None.
        Nothing,
    }

    impl Way {
        const fn name(self) -> &'static str {
            match self {
                Self::Ta => "through the TA",
                Self::Ours => "our objects, our layout 4",
                Self::OursTaFlags(_) => "our objects, the TA's list flags",
                Self::TaStrip4 => "through the TA, one strip of four",
                Self::Direct(f) => f.name,
                Self::TaDell => "Dell's clear and strip, through the TA",
            }
        }
    }

    /// Where test G2's per-tile lists go: after the objects, in the
    /// parameter memory (32 bytes a tile: up to seven objects and the end).
    const LISTS: usize = PARAMS + 0x1000;
    const LIST_WORDS: usize = 8;
    const _: () = assert!(LISTS + 4 * LIST_WORDS * (COLS * ROWS) as usize <= PM + PM_SIZE as usize);

    /// Which of [`TRIS`] an object covers (bit k: triangle k); every
    /// tile for [`ALL`].
    const ALL: u8 = 0xFF;

    /// `objs` (words, list flags, cover) rendered with a list for each
    /// tile holding only the objects that touch it ([`super::touches`]);
    /// true if it finished. A timeout resets the core.
    #[inline(never)]
    fn scene_tiled(e: &mut Engine<Mmio>, vram: &mut [u8], objs: &[(&[u32], u32, u8)]) -> bool {
        let mut at = OBJECT;
        let mut word = [m3::END; LIST_WORDS - 1];
        let n = objs.len().min(LIST_WORDS - 1);
        for (k, (w, flags, _)) in objs[..n].iter().enumerate() {
            store(vram, at, w);
            word[k] = flags | ((at - PARAMS) / 4) as u32;
            at += 4 * w.len();
        }
        let p = |k: usize| (QUAD[k].0, QUAD[k].1);
        let total = COLS * ROWS;
        for t in 0..total {
            let (c, r) = (t % COLS, t / COLS);
            let rect = (
                (c * m3::TILE_W) as f32,
                (r * m3::TILE_H) as f32,
                ((c + 1) * m3::TILE_W) as f32,
                ((r + 1) * m3::TILE_H) as f32,
            );
            let mut list = [m3::END; LIST_WORDS];
            let mut len = 0;
            for (k, &(_, _, cover)) in objs[..n].iter().enumerate() {
                let hit = cover == ALL
                    || (0..TRIS.len())
                        .any(|j| cover & 1 << j != 0 && super::touches(TRIS[j].map(p), rect));
                if hit {
                    list[len] = word[k];
                    len += 1;
                }
            }
            let addr = LISTS + 4 * LIST_WORDS * t as usize;
            store(vram, addr, &list);
            let entry = m3::region_entry(c, r, addr as u32, t + 1 == total);
            store(vram, REGIONS + 8 * t as usize, &entry);
        }
        cpu::drain_write_buffer();
        let (t, len) = table();
        let done = run(e, &t[..len], RENDER_DONE, 200_000);
        if !done {
            e.reset(rt::delay_us);
            let ok = pm_init(e, vram);
            log!("  (timed out: core reset, memory manager ready again: {ok})");
        }
        done
    }

    /// Test G2's and G3's direct form `f` drawn (the target already
    /// blanked); with `quad` false, its background alone. With `words`, the
    /// quad's first object, its list word and
    /// the background's logged.
    fn draw_direct(
        e: &mut Engine<Mmio>,
        vram: &mut [u8],
        f: Form,
        words: usize,
        quad_too: bool,
    ) -> bool {
        let z = if f.z1 { 1.0 } else { 0.0 };
        const BLACK: u32 = 0xFF00_0000;
        let giant = |tsp: u32, z: f32| {
            m3::triangle(
                ISP_ALWAYS,
                tsp,
                &[(0.0, 0.0), (2048.0, 0.0), (0.0, 2048.0)].map(|(x, y)| Vertex {
                    x,
                    y,
                    z,
                    rhw: 1.0,
                    argb: BLACK,
                }),
            )
        };
        let screen = |t: [usize; 3]| {
            m3::triangle(
                ISP_ALWAYS,
                TSP_DELL,
                &t.map(|k| Vertex::flat(QUAD[k].0, QUAD[k].1, BLACK)),
            )
        };
        let (flat0, flat1, smooth) = (
            giant(TSP_DELL, 0.0),
            giant(TSP_DELL, 1.0),
            giant(TSP_SMOOTH, 0.0),
        );
        let (half0, half1) = (screen(TRIS[0]), screen(TRIS[1]));
        let clear_bg = super::clear_object(W, H);
        let dell_bg = super::four_corner_object(
            ISP_ALWAYS,
            super::DELL_CLEAR_TSP,
            1.0,
            super::dell_clear_corners(W, H, BLACK),
        );
        let mut objs: [(&[u32], u32, u8); 4] = [(&[], 0, 0); 4];
        let mut n = 0;
        let bgs: [(&[u32], u32, u8); 2] = match f.bg {
            Bg::Giant => [(&flat0, STRIP, ALL), (&[], 0, 0)],
            Bg::GiantZ1 => [(&flat1, STRIP, ALL), (&[], 0, 0)],
            Bg::Clear => [(&clear_bg, super::CLEAR_FLAGS, ALL), (&[], 0, 0)],
            Bg::DellQuad => [(&dell_bg, super::QUAD_FLAGS, ALL), (&[], 0, 0)],
            Bg::ScreenTris => [(&half0, STRIP, 1), (&half1, STRIP, 2)],
            Bg::Gouraud => [(&smooth, STRIP, ALL), (&[], 0, 0)],
            Bg::Nothing => [(&[], 0, 0), (&[], 0, 0)],
        };
        for o in bgs {
            if !o.0.is_empty() {
                objs[n] = o;
                n += 1;
            }
        }
        let tri = |t: [usize; 3]| {
            m3::triangle(
                ISP_ALWAYS,
                TSP_SMOOTH,
                &t.map(|k| Vertex {
                    x: QUAD[k].0,
                    y: QUAD[k].1,
                    z,
                    rhw: 1.0,
                    argb: QUAD[k].2,
                }),
            )
        };
        let (a, b) = (tri(TRIS[0]), tri(TRIS[1]));
        let quad = super::quad_object(ISP_ALWAYS, TSP_SMOOTH, z);
        let first = n;
        if !quad_too {
        } else if f.quad {
            objs[n] = (&quad, super::QUAD_FLAGS, 3);
            n += 1;
        } else {
            objs[n] = (&a, STRIP, 1);
            objs[n + 1] = (&b, STRIP, 2);
            n += 2;
        }
        if words > 0 && first < n {
            let _ = write!(
                RunLog,
                "    list word 0x{:08x} (layout {}), object:",
                objs[first].1,
                (objs[first].1 & super::LAYOUT) >> 21
            );
            for w in objs[first].0 {
                let _ = write!(RunLog, " {w:08x}");
            }
            let _ = write!(RunLog, "; background:");
            for o in &objs[..first] {
                let _ = write!(RunLog, " list word 0x{:08x}, {} words", o.1, o.0.len());
                for w in o.0.iter().take(2) {
                    let _ = write!(RunLog, " {w:08x}");
                }
                let _ = write!(RunLog, ";");
            }
            log!("");
        }
        if f.tiled {
            scene_tiled(e, vram, &objs[..n])
        } else {
            let mut plain = [(&[][..], 0u32); 4];
            for (k, o) in objs[..n].iter().enumerate() {
                plain[k] = (o.0, o.1);
            }
            scene_with(e, vram, &plain[..n], f.regs)
        }
    }

    /// One render of `way` into the target (blanked first). With `words`,
    /// the TA's lists and objects are logged. Whether it finished, the
    /// render's time in microseconds, and the TA's first tile's list.
    fn draw(
        e: &mut Engine<Mmio>,
        vram: &mut [u8],
        way: Way,
        words: usize,
    ) -> (bool, u32, [u32; TA_LIST]) {
        blank(vram);
        let clip = m3::ta::clip(W, H);
        let t0 = rt::ticks();
        match way {
            Way::Ta | Way::TaStrip4 => {
                let clear = m3::ta::clear_stream(W, H);
                let (a, na) = ta_draw(TSP_SMOOTH, ISP_ALWAYS, clip, &TRIS[0]);
                let (b, nb) = ta_draw(TSP_SMOOTH, ISP_ALWAYS, clip, &TRIS[1]);
                let (q, nq) = ta_draw(TSP_SMOOTH, ISP_ALWAYS, clip, &[0, 1, 3, 2]);
                let out = if let Way::Ta = way {
                    through_ta(e, vram, &[&clear[..], &a[..na], &b[..nb]], words)
                } else {
                    through_ta(e, vram, &[&clear[..], &q[..nq]], words)
                };
                (out.ta && out.done, us_since(t0), out.list)
            }
            Way::Ours | Way::OursTaFlags(_) => {
                let flags = match way {
                    Way::OursTaFlags(f) => f,
                    _ => STRIP,
                };
                let bg = m3::triangle(
                    ISP_ALWAYS,
                    TSP_DELL,
                    &[
                        Vertex::flat(0.0, 0.0, 0xFF00_0000),
                        Vertex::flat(2048.0, 0.0, 0xFF00_0000),
                        Vertex::flat(0.0, 2048.0, 0xFF00_0000),
                    ],
                );
                let tri = |t: [usize; 3]| {
                    m3::triangle(
                        ISP_ALWAYS,
                        TSP_SMOOTH,
                        &t.map(|k| Vertex::flat(QUAD[k].0, QUAD[k].1, QUAD[k].2)),
                    )
                };
                let (a, b) = (tri(TRIS[0]), tri(TRIS[1]));
                if words > 0 {
                    let _ = write!(
                        RunLog,
                        "    our list word 0x{flags:08x} (layout {}), object:",
                        (flags & super::LAYOUT) >> 21
                    );
                    for w in a {
                        let _ = write!(RunLog, " {w:08x}");
                    }
                    log!("");
                }
                let done = scene(e, vram, &[(&bg, STRIP), (&a, flags), (&b, flags)]);
                (done, us_since(t0), [m3::END; TA_LIST])
            }
            Way::Direct(f) => {
                let done = draw_direct(e, vram, f, words, true);
                (done, us_since(t0), [m3::END; TA_LIST])
            }
            Way::TaDell => {
                let clear = super::dell_clear_draw(ISP_ALWAYS, W, H, 1.0, 0xFF00_0000);
                let (q, nq) = ta_draw(TSP_SMOOTH, ISP_ALWAYS, clip, &[0, 1, 3, 2]);
                let out = through_ta(e, vram, &[&clear[..], &q[..nq]], words);
                (out.ta && out.done, us_since(t0), out.list)
            }
        }
    }

    /// The list word of the TA's Gouraud object in `list` (its object's
    /// first two words hold [`TSP_SMOOTH`]), if it wrote one.
    fn gouraud_word(vram: &[u8], list: &[u32; TA_LIST]) -> Option<u32> {
        list.iter()
            .take_while(|&&p| p != m3::END)
            .copied()
            .find(|&p| {
                let obj = PARAMS + 4 * (p & 0x001F_FFFF) as usize;
                (0..2).any(|j| load32_volatile(vram, obj + 4 * j) == Some(TSP_SMOOTH))
            })
    }

    /// What one way gave at one setting.
    #[derive(Default)]
    struct Setting {
        /// The clock reached it.
        clocked: bool,
        /// Renders at divide 1 equal to the reference.
        right: u32,
        /// Renders at divide 1 that never finished.
        unfinished: u32,
        /// Every wrong word at divide 1.
        tally: Tally,
        /// Wrong renders whose second read (50 ms later) differed.
        unsettled: u32,
        /// The fastest render at divide 1, microseconds.
        us: u32,
        /// The display sent at every check of this setting (after the
        /// change, after the draws, back at Windows Mobile's clock).
        screen: bool,
        /// The control at Windows Mobile's clock after: equal.
        control: bool,
    }

    /// The most settings a way runs at.
    const SETTINGS: usize = 3;

    /// What one way gave.
    #[derive(Default)]
    struct WayResult {
        /// The reference rendered twice the same.
        repeatable: bool,
        /// The reference's fastest render, microseconds.
        us_wm: u32,
        /// Each setting's result, in the order run.
        at: [Setting; SETTINGS],
    }

    impl WayResult {
        /// Renders right at setting `k`, if it ran.
        fn right(&self, k: usize) -> Option<u32> {
            (self.repeatable && self.at[k].clocked).then_some(self.at[k].right)
        }
    }

    /// One way: its reference at Windows Mobile's clock (rendered twice,
    /// equal; kept), then at each of `settings` (a core PLL with the cores
    /// at divide 1, and its name in MHz) 20 renders held to it and a control
    /// at Windows Mobile's clock. `step` and `steps` label the screen.
    #[inline(never)]
    fn way(
        e: &mut Engine<Mmio>,
        vram: &mut [u8],
        w: Way,
        settings: &[(u32, &str)],
        step: u32,
        steps: u32,
    ) -> (WayResult, [u32; TA_LIST]) {
        let mut r = WayResult::default();
        let mut head = [0u8; 32];
        let head = fmt(&mut head, format_args!("STEP {step} OF {steps}"));
        let mut name = [0u8; 32];
        let name = fmt(&mut name, format_args!("WAY {}: {}", step - 1, w.name()));
        say(vram, &[G_TITLE, head, name, "WM CLOCK: REFERENCE"]);
        log!("{G_NAME} way {} ({}):", step - 1, w.name());
        rt::check_in();
        let _ = display_check(&mut e.regs, "before the reference");
        let ready = clean(e, vram);
        // G3 and G4: the background alone first. The quad covers the whole target,
        // so a background never shows in the reference; its own picture
        // proves it drew (and ways whose backgrounds must look alike are
        // held to each other).
        if let (true, Way::Direct(f)) = (TEST_G3 || TEST_G4, w)
            && f.bg != Bg::Nothing
        {
            blank(vram);
            let done = draw_direct(e, vram, f, 0, false);
            log!(
                "  background alone: rendered {done}, hash 0x{:08x}",
                picture_hash(vram, TARGET)
            );
        }
        let (d1, us1, list) = draw(e, vram, w, 24);
        // G4: the start table's words Dell's way, read back from the core
        // after the render (that they were written, and held).
        if let Way::Direct(f) = w
            && !f.regs.is_empty()
        {
            let _ = write!(RunLog, "  read back after the render:");
            for &(off, _) in f.regs {
                let _ = write!(RunLog, " 0x{off:x} 0x{:08x}", e.regs.read32(MBX + off));
            }
            log!("");
        }
        keep_reference(vram);
        // The reference's hash: ways that must draw the same picture (G2's
        // forms of one quad over one background) are held to each other,
        // not only each to itself.
        log!("  reference hash 0x{:08x}", picture_hash(vram, REF));
        let (d2, us2, _) = draw(e, vram, w, 0);
        let again = compare(vram, REF);
        r.repeatable = d1 && d2 && again.wrong == 0;
        r.us_wm = us1.min(us2);
        log!(
            "  reference at Windows Mobile's clock: memory manager ready {ready}, rendered {d1} then {d2}, the second {} words off the first; {} us",
            again.wrong,
            r.us_wm
        );
        if !r.repeatable {
            // What differs (test G's way 1 differed in every word): five
            // places, the first render's word then the second's.
            let _ = write!(RunLog, "    samples (first, second):");
            for (x, y) in [(2, 2), (240, 320), (477, 2), (2, 637), (477, 637)] {
                let at = 4 * (y * W + x) as usize;
                let _ = write!(
                    RunLog,
                    " ({x}, {y}) 0x{:08x} 0x{:08x};",
                    load32_volatile(vram, REF + at).unwrap_or(0),
                    load32_volatile(vram, TARGET + at).unwrap_or(0)
                );
            }
            log!("");
            log!("  no repeatable reference: nothing at divide 1");
            return (r, list);
        }
        for (k, &(pll, mhz)) in settings.iter().take(SETTINGS).enumerate() {
            let s = &mut r.at[k];
            let mut line = [0u8; 32];
            let line = fmt(&mut line, format_args!("{mhz} MHZ / 1: SETTING"));
            say(vram, &[G_TITLE, head, name, line]);
            s.clocked = to_divide_1(&mut e.regs, Speed::ladder(pll));
            clock_readback(&mut e.regs);
            if !s.clocked {
                log!(
                    "  the clock change was refused (Windows Mobile's clock kept): nothing at divide 1"
                );
                continue;
            }
            let ready = clean(e, vram);
            log!("  at {mhz} / 1: memory manager ready {ready}");
            s.screen = display_check(&mut e.regs, "after the change");
            s.us = u32::MAX;
            for n in 1..=G_DRAWS {
                rt::check_in();
                let mut line = [0u8; 32];
                let line = fmt(&mut line, format_args!("{mhz} MHZ / 1: {n} OF {G_DRAWS}"));
                say(vram, &[G_TITLE, head, name, line]);
                let (done, us, _) = draw(e, vram, w, 0);
                s.us = s.us.min(us);
                if !done {
                    s.unfinished += 1;
                    let _ = clean(e, vram);
                }
                let t = compare(vram, REF);
                if t.wrong == 0 {
                    s.right += 1;
                } else {
                    rt::delay_us(50_000);
                    if compare(vram, REF) != t {
                        s.unsettled += 1;
                    }
                }
                s.tally.merge(&t);
            }
            s.screen &= display_check(&mut e.regs, "after the draws");
            clock_back(&mut e.regs);
            s.screen &= display_check(&mut e.regs, "back at Windows Mobile's clock");
            clock_readback(&mut e.regs);
            let _ = clean(e, vram);
            let (done, _, _) = draw(e, vram, w, 0);
            s.control = done && compare(vram, REF).wrong == 0;
            let t = &s.tally;
            let _ = write!(
                RunLog,
                "  at {mhz} / 1: {} of {G_DRAWS} right ({} unfinished, {} read differently 50 ms later); wrong words {} ({} one step, {} never written, worst step {})",
                s.right, s.unfinished, s.unsettled, t.wrong, t.one_step, t.unwritten, t.max_step
            );
            if let Some((i, want, got)) = t.first {
                let _ = write!(
                    RunLog,
                    ", first at ({}, {}) 0x{got:08x} for 0x{want:08x}",
                    i % W,
                    i / W
                );
            }
            if (TEST_G2 || TEST_G3 || TEST_G4) && t.wrong > 0 {
                let _ = write!(RunLog, "; on the picture's edge {}", t.edge);
                if let Some((i, want, got)) = t.first_inner {
                    let _ = write!(
                        RunLog,
                        ", first inside at ({}, {}) 0x{got:08x} for 0x{want:08x}",
                        i % W,
                        i / W
                    );
                }
            }
            log!(
                "; fastest {} us (Windows Mobile's {} us); control at Windows Mobile's clock after: {}; the screen on throughout: {}",
                s.us,
                r.us_wm,
                if s.control { "right" } else { "WRONG" },
                if s.screen {
                    "yes"
                } else {
                    "NO (a screen-off result)"
                }
            );
        }
        (r, list)
    }

    /// `args` formatted into `buf` (cut at its end).
    fn fmt<'a>(buf: &'a mut [u8], args: core::fmt::Arguments) -> &'a str {
        struct Cut<'b>(&'b mut [u8], usize);
        impl Write for Cut<'_> {
            fn write_str(&mut self, s: &str) -> core::fmt::Result {
                for &b in s.as_bytes() {
                    if self.1 < self.0.len() {
                        self.0[self.1] = b;
                        self.1 += 1;
                    }
                }
                Ok(())
            }
        }
        let mut c = Cut(buf, 0);
        let _ = c.write_fmt(args);
        let n = c.1;
        core::str::from_utf8(&buf[..n]).unwrap_or("")
    }

    /// Test G: the three ways, then the strip of four; each against its
    /// own reference. The summary on the label.
    #[inline(never)]
    fn test_g(e: &mut Engine<Mmio>, vram: &mut [u8]) {
        log!(
            "G: Gouraud at 74.75 MHz / 1, through the TA against our objects (TSP 0x{TSP_SMOOTH:08x}, GC2's ARGB8888 quad)"
        );
        rp_image::on_crash(put_back);
        let at = &super::G3_PLLS[..1];
        let (ta, list) = way(e, vram, Way::Ta, at, 2, 5);
        let (ours, _) = way(e, vram, Way::Ours, at, 3, 5);
        let flags = gouraud_word(vram, &list);
        let mixed = match flags {
            Some(p) => {
                let (f, all) = adopt_flags(STRIP, p & super::FLAGS);
                log!(
                    "G: the TA's Gouraud list word 0x{p:08x} (layout {}); way 3 takes {} (0x{f:08x})",
                    (p & super::LAYOUT) >> 21,
                    if all {
                        "all of its flags"
                    } else {
                        "its layout only (its triangle mask differs from ours)"
                    }
                );
                Some(way(e, vram, Way::OursTaFlags(f), at, 4, 5).0)
            }
            None => {
                log!("G way 3: skipped (the TA wrote no Gouraud object in the first tile)");
                None
            }
        };
        let (strip4, _) = way(e, vram, Way::TaStrip4, at, 5, 5);
        let count = |r: &WayResult| r.right(0).unwrap_or(0);
        log!(
            "G: right at 74.75 / 1 of {G_DRAWS}: through the TA {}, ours {}, ours with the TA's flags {}, the TA's strip of four {}",
            count(&ta),
            count(&ours),
            mixed.as_ref().map_or(0, count),
            count(&strip4)
        );
        let mut l = [[0u8; 32]; 4];
        let [l0, l1, l2, l3] = &mut l;
        let a = fmt(l0, format_args!("TA: {} OF {G_DRAWS} RIGHT", count(&ta)));
        let b = fmt(l1, format_args!("OURS: {} OF {G_DRAWS}", count(&ours)));
        let c = fmt(
            l2,
            format_args!("TA FLAGS: {} OF {G_DRAWS}", mixed.as_ref().map_or(0, count)),
        );
        let d = fmt(
            l3,
            format_args!("STRIP OF 4: {} OF {G_DRAWS}", count(&strip4)),
        );
        say(
            vram,
            &["RP-GLESFEAT TEST G", "DONE: PRESS A BUTTON", a, b, c, d],
        );
    }

    /// Test G2's ways, in order: our form (G's way 2, the control), each
    /// of the TA's four choices alone, all four together, then the TA
    /// itself (last: a TA hang isn't recovered within a run, C53).
    const G2_WAYS: [Way; 8] = {
        const OURS: Form = Form {
            name: "ours (G's way 2)",
            quad: false,
            z1: false,
            bg: Bg::Giant,
            tiled: false,
            regs: &[],
        };
        [
            Way::Direct(OURS),
            Way::Direct(Form {
                name: "one 4-corner object",
                quad: true,
                ..OURS
            }),
            Way::Direct(Form {
                name: "z 1.0",
                z1: true,
                ..OURS
            }),
            Way::Direct(Form {
                name: "the clear underneath",
                bg: Bg::Clear,
                ..OURS
            }),
            Way::Direct(Form {
                name: "per-tile lists",
                tiled: true,
                ..OURS
            }),
            Way::Direct(Form {
                name: "all four: the TA's form",
                quad: true,
                z1: true,
                bg: Bg::Clear,
                tiled: true,
                regs: &[],
            }),
            Way::TaStrip4,
            Way::Ta,
        ]
    };

    /// Test G2: each of [`G2_WAYS`] against its own reference; the summary
    /// on the label.
    #[inline(never)]
    fn test_g2(e: &mut Engine<Mmio>, vram: &mut [u8]) {
        log!(
            "G2: which of the TA's choices makes Gouraud right at 74.75 MHz / 1 (TSP 0x{TSP_SMOOTH:08x}, GC2's ARGB8888 quad)"
        );
        rp_image::on_crash(put_back);
        let steps = G2_WAYS.len() as u32 + 1;
        let mut right = [None; G2_WAYS.len()];
        for (k, &w) in G2_WAYS.iter().enumerate() {
            let (r, _) = way(e, vram, w, &super::G3_PLLS[..1], k as u32 + 2, steps);
            right[k] = r.right(0);
        }
        let _ = write!(RunLog, "G2: right at 74.75 / 1 of {G_DRAWS}:");
        for (k, w) in G2_WAYS.iter().enumerate() {
            match right[k] {
                Some(n) => {
                    let _ = write!(RunLog, " {} {n};", w.name());
                }
                None => {
                    let _ = write!(RunLog, " {} not run;", w.name());
                }
            }
        }
        log!("");
        let mut l = [[0u8; 32]; 8];
        let mut lines: [&str; 10] = [""; 10];
        lines[0] = G_TITLE;
        lines[1] = "DONE: PRESS A BUTTON";
        for (k, (buf, w)) in l.iter_mut().zip(G2_WAYS.iter()).enumerate() {
            lines[2 + k] = match right[k] {
                Some(n) => fmt(buf, format_args!("{}: {n} OF {G_DRAWS}", k + 1)),
                None => fmt(buf, format_args!("{}: NOT RUN", k + 1)),
            };
            let _ = w;
        }
        say(vram, &lines);
    }

    /// Test G3's ways (the quad our two Gouraud triangles unless named):
    /// our background (G2's way 1, the control), Dell's colour clear by
    /// hand, Dell's whole sequence by hand, our background's traits one at a
    /// time against Dell's (its size, its depth), the other backgrounds a
    /// renderer might use (Gouraud, none), then Dell's sequence through the
    /// TA (last: a TA hang isn't recovered within a run, C53).
    const G3_WAYS: [Way; 8] = {
        const OURS: Form = Form {
            name: "our giant flat triangle",
            quad: false,
            z1: false,
            bg: Bg::Giant,
            tiled: false,
            regs: &[],
        };
        [
            Way::Direct(OURS),
            Way::Direct(Form {
                name: "Dell's clear (a flat quad)",
                bg: Bg::DellQuad,
                ..OURS
            }),
            Way::Direct(Form {
                name: "Dell's clear and 4-corner quad",
                bg: Bg::DellQuad,
                quad: true,
                ..OURS
            }),
            Way::Direct(Form {
                name: "flat, screen-sized triangles",
                bg: Bg::ScreenTris,
                ..OURS
            }),
            Way::Direct(Form {
                name: "our giant triangle at z 1.0",
                bg: Bg::GiantZ1,
                ..OURS
            }),
            Way::Direct(Form {
                name: "a Gouraud giant triangle",
                bg: Bg::Gouraud,
                ..OURS
            }),
            Way::Direct(Form {
                name: "no background",
                bg: Bg::Nothing,
                ..OURS
            }),
            Way::TaDell,
        ]
    };

    /// Test G4's ways: our form (the control), G2's colourless clear (the one
    /// form seen right at divide 1), Dell's render table whole (with our
    /// 32-bit target, then with Dell's 16-bit one: Dell's set-up exactly),
    /// then each of Dell's words alone.
    const G4_WAYS: [Way; 10] = {
        const OURS: Form = Form {
            name: "ours",
            quad: false,
            z1: false,
            bg: Bg::Giant,
            tiled: false,
            regs: &[],
        };
        const D: [(usize, u32); 7] = super::DELL_TABLE;
        const ALL16: [(usize, u32); 9] = [
            D[0],
            D[1],
            D[2],
            D[3],
            D[4],
            D[5],
            D[6],
            super::DELL_TARGET16[0],
            super::DELL_TARGET16[1],
        ];
        const fn one(name: &'static str, regs: &'static [(usize, u32)]) -> Way {
            Way::Direct(Form { name, regs, ..OURS })
        }
        [
            Way::Direct(OURS),
            Way::Direct(Form {
                name: "G2's colourless clear",
                bg: Bg::Clear,
                ..OURS
            }),
            one("Dell's table, 32-bit", &D),
            one("Dell's table and RGB565", &ALL16),
            one("Dell's 0x610 alone", &[D[0]]),
            one("Dell's 0x614, 0x618 alone", &[D[1], D[2]]),
            one("Dell's 0x61c alone", &[D[3]]),
            one("Dell's 0x648 alone", &[D[4]]),
            one("Dell's 0x6d8 alone", &[D[5]]),
            one("Dell's 0x6dc alone", &[D[6]]),
        ]
    };

    /// The most ways [`test_settings`] takes.
    const MAX_WAYS: usize = 10;

    /// Test G3 or G4: each of `ways` at each of [`super::G3_PLLS`], against
    /// its own reference; the summary in the log (each screen-off result
    /// marked) and on the label.
    #[inline(never)]
    fn test_settings(e: &mut Engine<Mmio>, vram: &mut [u8], ways: &[Way]) {
        rp_image::on_crash(put_back);
        let ways = &ways[..ways.len().min(MAX_WAYS)];
        let steps = ways.len() as u32 + 1;
        let mut right = [[None; SETTINGS]; MAX_WAYS];
        let mut dark = [[false; SETTINGS]; MAX_WAYS];
        for (k, &w) in ways.iter().enumerate() {
            let (r, _) = way(e, vram, w, &super::G3_PLLS, k as u32 + 2, steps);
            for s in 0..SETTINGS {
                right[k][s] = r.right(s);
                dark[k][s] = r.right(s).is_some() && !r.at[s].screen;
            }
        }
        for (s, (_, mhz)) in super::G3_PLLS.iter().enumerate() {
            let _ = write!(RunLog, "{G_NAME}: right at {mhz} / 1 of {G_DRAWS}:");
            for (k, w) in ways.iter().enumerate() {
                match right[k][s] {
                    Some(n) => {
                        let off = if dark[k][s] { " (screen off)" } else { "" };
                        let _ = write!(RunLog, " {} {n}{off};", w.name());
                    }
                    None => {
                        let _ = write!(RunLog, " {} not run;", w.name());
                    }
                }
            }
            log!("");
        }
        let mut l = [[0u8; 32]; MAX_WAYS];
        let mut lines: [&str; 4 + MAX_WAYS] = [""; 4 + MAX_WAYS];
        lines[0] = G_TITLE;
        lines[1] = "DONE: PRESS A BUTTON";
        lines[2] = "RIGHT OF 20 AT 74/99/133";
        lines[3] = "* = THE SCREEN WENT DARK";
        for (k, buf) in l.iter_mut().enumerate().take(ways.len()) {
            let n = |s: usize| right[k][s].map_or(-1, |v| v as i32);
            let d = |s: usize| if dark[k][s] { "*" } else { "" };
            lines[4 + k] = fmt(
                buf,
                format_args!(
                    "WAY {}: {}{} {}{} {}{}",
                    k + 1,
                    n(0),
                    d(0),
                    n(1),
                    d(1),
                    n(2),
                    d(2)
                ),
            );
        }
        say(vram, &lines[..4 + ways.len()]);
    }

    /// Test G3: the backgrounds.
    fn test_g3(e: &mut Engine<Mmio>, vram: &mut [u8]) {
        log!(
            "G3: which background lets Gouraud draw right at the cores' divide 1, at 74.75, 99.67 and 133.25 MHz (TSP 0x{TSP_SMOOTH:08x}, GC2's ARGB8888 quad; Dell's clear as libGLES_CL draws it)"
        );
        test_settings(e, vram, &G3_WAYS);
    }

    /// Test G4: Dell's render table.
    fn test_g4(e: &mut Engine<Mmio>, vram: &mut [u8]) {
        log!(
            "G4: Dell's render table at the cores' divide 1, whole and word by word, at 74.75, 99.67 and 133.25 MHz (TSP 0x{TSP_SMOOTH:08x}, GC2's quad; Dell's words read by rp-wmprobe at 74.75 / 1)"
        );
        test_settings(e, vram, &G4_WAYS);
    }

    fn main() -> ! {
        rt::init_mmu();
        let Some((regs, vram)) = rp_image::take_mbx() else {
            crash(format_args!("no 2700G"))
        };
        let mut display = Display::new(regs);
        let _ = display.adopt(vram.len());
        let regs = display.into_inner();
        let mut e = Engine::new(regs, FENCE, rt::ticks, 3_250);
        // The label first: the screen says what runs from the first moment.
        let saved = show_label(&mut e.regs, vram);
        let title = if TEST_G {
            G_TITLE
        } else {
            "RP-GLESFEAT A D L M"
        };
        let first = if TEST_G4 {
            "STEP 1 OF 11"
        } else if TEST_G2 || TEST_G3 {
            "STEP 1 OF 9"
        } else {
            "STEP 1 OF 5"
        };
        say(vram, &[title, first, "SETTING UP THE 3D CORE"]);
        let clk0 = e.regs.read32(MBXCLK);
        if TEST_G {
            log!(
                "{MARKER}: test {G_NAME}, Gouraud at the cores' divide 1 (the give-up audit's item 1)"
            );
        } else {
            log!("{MARKER}: alpha test, depth 'less', 8-bit grey textures (ADR-0026)");
        }
        let _ = store32_volatile(vram, FENCE, 0);
        cpu::drain_write_buffer();
        let mut ready = false;
        if e.regs.read32(PLLSTAT) & 4 != 0 {
            rp_drv_mbx_display::clock::restore(&mut e.regs, with_3d(clk0));
            rt::delay_us(1_000);
            e.reset(rt::delay_us);
            ready = pm_init(&mut e, vram);
        }
        log!(
            "MBXCLK 0x{clk0:x} -> 0x{:x}, parameter memory ready: {ready}",
            e.regs.read32(MBXCLK)
        );
        if ready && TEST_G4 {
            test_g4(&mut e, vram);
        } else if ready && TEST_G3 {
            test_g3(&mut e, vram);
        } else if ready && TEST_G2 {
            test_g2(&mut e, vram);
        } else if ready && TEST_G {
            test_g(&mut e, vram);
        } else if ready {
            let fill = rp_image::need(
                cmd::fill(
                    Format::BPP32,
                    TARGET,
                    STRIDE,
                    (0, 0, W as u16, H as u16),
                    GREY,
                ),
                "a 2D rectangle",
            );
            let _ = e.submit(&fill).and_then(|()| e.finish(vram, 200_000));
            say(
                vram,
                &["RP-GLESFEAT A D L M", "STEP 2 OF 5", "A: ALPHA TEST"],
            );
            log!("A: alpha test, red square with vertex alpha 64 over grey");
            alpha(&mut e, vram);
            say(
                vram,
                &["RP-GLESFEAT A D L M", "STEP 3 OF 5", "D: DEPTH LESS"],
            );
            log!("D: depth, red z 0.5 then blue z 0.3 / 0.8, writes on");
            depth(&mut e, vram);
            say(
                vram,
                &[
                    "RP-GLESFEAT A D L M",
                    "STEP 4 OF 5",
                    "L: 8-BIT GREY TEXTURES",
                ],
            );
            log!("L: 8-bit grey textures (code 10)");
            luminance(&mut e, vram);
            say(
                vram,
                &["RP-GLESFEAT A D L M", "STEP 5 OF 5", "M: TWO TEXTURE UNITS"],
            );
            log!("M: the second texture unit through the TA (0x80 x 0x40)");
            two_units(&mut e, vram);
            // The last render on screen until a button.
            let _ = show(
                &mut e.regs,
                Plane {
                    format: PixelFormat::Argb8888,
                    width: W as u16,
                    height: H as u16,
                    offset: TARGET,
                    stride: STRIDE,
                },
            );
        } else {
            say(
                vram,
                &["RP-GLESFEAT", "NO PARAMETER MEMORY", "PRESS A BUTTON"],
            );
        }
        let keypad = arm_keypad();
        log!(
            "{MARKER}: {} - {} until a button",
            if ready { "DONE" } else { "no parameter memory" },
            if TEST_G {
                "the summary shown"
            } else {
                "the last render shown"
            }
        );
        // Test G's summary stays up a minute: long enough to read.
        hold_until_button(keypad, if TEST_G { 60_000_000 } else { 10_000_000 });
        for (off, v) in saved {
            e.regs.write32(off, v);
        }
        e.reset(rt::delay_us);
        rp_drv_mbx_display::clock::restore(&mut e.regs, clk0);
        rp_image::finish()
    }

    /// `plane` scanned out; the display's registers as they were.
    fn show(regs: &mut Mmio, plane: Plane) -> [(usize, u32); 5] {
        let new = plane.registers();
        let saved = new.map(|(off, _)| (off, regs.read32(off)));
        let gamma = regs.read32(dreg::GSCTRL) & (1 << 26);
        for (off, v) in new {
            let v = match off {
                dreg::GSCTRL => v | gamma,
                dreg::GSCADR => Layer::SOLO.scadr(v),
                dreg::GBBASE => Layer::SOLO.bbase(v),
                _ => v,
            };
            regs.write32(off, v);
        }
        saved
    }

    fn arm_keypad() -> Option<Keypad<Mmio>> {
        let kp = rt::take_keypad()?;
        let mut keypad = Keypad::new(kp);
        keypad.enable();
        keypad.acknowledge();
        Some(keypad)
    }

    fn hold_until_button(keypad: Option<Keypad<Mmio>>, limit_us: u32) {
        let Some(mut keypad) = keypad else {
            return;
        };
        let t0 = rt::ticks();
        let mut seen = false;
        while us_since(t0) < limit_us {
            rt::check_in();
            seen |= keypad.acknowledge();
            if seen && keypad.pressed() == Pressed::NONE {
                return;
            }
            rt::delay_us(20_000);
        }
        log!("no button: returning");
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}

#[cfg(test)]
mod tests {
    use super::alpha_word;

    #[test]
    fn the_alpha_word_matches_every_captured_one() {
        // Dell's objects ("always", reference 0) and DoomGLES's textured
        // ones ("always", reference 100), captured on the device.
        assert_eq!(alpha_word(7, 0), 0xE000_0000);
        assert_eq!(alpha_word(7, 100), 0xEC80_0000);
        // Version 23's depth "less" blue square: "alpha < 0".
        assert_eq!(alpha_word(1, 0), 1 << 29);
    }
}

/// Test G's pure parts: the TA draws, way 3's list word, the tally.
#[cfg(test)]
mod g_tests {
    use super::*;
    use rp_drv_mbx3d as m3;

    #[test]
    fn the_constants_agree_with_the_driver_and_dell() {
        // Dell's smooth TSP is Dell's flat one with Gouraud's bit, bit 8
        // clear (a colour per vertex).
        assert_eq!(TSP_SMOOTH, m3::TSP_DELL | m3::TSP_GOURAUD);
        assert_eq!(TSP_SMOOTH & 1 << 8, 0);
        // The untextured state block is the GLES clear's.
        assert_eq!(m3::ta::clear_stream(480, 640)[0], TA_STATE);
        assert_eq!(TA_STRIP, m3::ta::STRIP);
        // Our flat list word carries layout 4, the textured one 8.
        assert_eq!((m3::STRIP_ONE_TRIANGLE & LAYOUT) >> 21, 4);
        assert_eq!((m3::tex::STRIP_GAME & LAYOUT) >> 21, 8);
        assert_eq!(MASK | LAYOUT, FLAGS);
        assert_eq!(MASK & LAYOUT, 0);
    }

    #[test]
    fn a_ta_draw_is_the_state_block_the_strip_and_five_words_a_vertex() {
        let clip = m3::ta::clip(480, 640);
        let (s, n) = ta_draw(TSP_SMOOTH, m3::ISP_ALWAYS, clip, &TRIS[0]);
        assert_eq!(n, 6 + 3 * TA_VERTEX_WORDS);
        assert_eq!(
            s[..6],
            [TA_STATE, TSP_SMOOTH, m3::ISP_ALWAYS, clip, 0, TA_STRIP]
        );
        let one = 1.0f32.to_bits();
        // Top left, top right, bottom left: x, y, z, 1/w, colour.
        assert_eq!(s[6..11], [0, 0, one, one, 0xFF00_0000]);
        assert_eq!(s[11..16], [480.0f32.to_bits(), 0, one, one, 0xFFFF_FFFF]);
        assert_eq!(s[16..21], [0, 640.0f32.to_bits(), one, one, 0xFFFF_E000]);
        assert!(s[n..].iter().all(|&w| w == 0));
        // A strip of four fills the buffer; a fifth corner is never sent.
        assert_eq!(ta_draw(TSP_SMOOTH, 0, clip, &[0, 1, 3, 2]).1, TA_DRAW_MAX);
        assert_eq!(
            ta_draw(TSP_SMOOTH, 0, clip, &[0, 1, 3, 2, 0]).1,
            TA_DRAW_MAX
        );
        assert_eq!(ta_draw(TSP_SMOOTH, 0, clip, &[]).1, 6);
    }

    #[test]
    fn the_two_triangles_cover_the_quad_once() {
        // Every corner used; the shared edge is the diagonal 1-3.
        let mut used = [0; 4];
        for t in TRIS {
            for k in t {
                used[k] += 1;
            }
        }
        assert_eq!(used, [1, 2, 1, 2]);
        // The quad is the whole target.
        assert_eq!((QUAD[2].0, QUAD[2].1), (480.0, 640.0));
    }

    #[test]
    fn way_3_takes_the_tas_flags_and_keeps_our_offset() {
        let ours = m3::STRIP_ONE_TRIANGLE | 0x123;
        // The same triangle mask: every flag of the TA's.
        let (w, all) = adopt_flags(ours, 0x4120_0000 | 0x1_FFFF);
        assert!(all);
        assert_eq!(w, 0x4120_0123);
        // A different mask (two triangles): the layout alone.
        let (w, all) = adopt_flags(ours, 0x6100_0000);
        assert!(!all);
        assert_eq!(w, 0x4100_0123);
        // Layout 0 (the clear's): ours goes to 0, nothing else moves.
        let (w, _) = adopt_flags(ours, 0x8000_0000);
        assert_eq!(w, 0x4000_0123);
        // The TA's own word back: unchanged.
        assert_eq!(adopt_flags(ours, ours), (ours, true));
    }

    #[test]
    fn the_tally_counts_steps_wraps_and_unwritten_words() {
        let mut t = Tally::default();
        t.add(0, 0xFF10_2030, 0xFF10_2030);
        assert_eq!(t, Tally::default());
        // One step in one channel, either way.
        t.add(1, 0xFF10_2030, 0xFF10_2031);
        t.add(2, 0xFF10_2030, 0xFF0F_2030);
        assert_eq!((t.wrong, t.one_step, t.max_step), (2, 2, 1));
        assert_eq!(t.first, Some((1, 0xFF10_2030, 0xFF10_2031)));
        // White wrapped to black in red: 255; alpha counts too.
        t.add(3, 0xFFFF_FFFF, 0xFF00_FFFF);
        t.add(4, 0xFF00_0000, 0xFE00_0000);
        assert_eq!((t.wrong, t.one_step, t.max_step), (4, 3, 255));
        // A word the render never wrote.
        t.add(5, 0xFF00_0000, SENTINEL);
        assert_eq!((t.wrong, t.unwritten), (5, 1));
        // Merged: sums, the larger step, the first kept.
        let mut u = Tally::default();
        u.add(9, 1, 3);
        u.merge(&t);
        assert_eq!(
            (u.wrong, u.one_step, u.unwritten, u.max_step),
            (6, 3, 1, 255)
        );
        assert_eq!(u.first, Some((9, 1, 3)));
        // The larger step kept whichever side holds it.
        let mut w = t;
        let mut small = Tally::default();
        small.add(0, 1, 3);
        w.merge(&small);
        assert_eq!(w.max_step, 255);
        let mut v = Tally::default();
        v.merge(&t);
        assert_eq!(v, t);
    }

    #[test]
    fn the_sentinel_is_no_colour_of_the_quad() {
        assert!(QUAD.iter().all(|&(_, _, c)| c >> 24 == 0xFF));
        assert_ne!(SENTINEL >> 24, 0xFF);
    }
}

/// Test G2's pure parts, held to the words the TA wrote on the X51v
/// (test G, `local/rp-glesfeat-G1.txt`, 2026-10-23).
#[cfg(test)]
mod g2_tests {
    use super::*;
    use rp_drv_mbx3d as m3;

    #[test]
    fn the_quad_object_is_the_tas_strip_of_four_word_for_word() {
        // Way 4's object at 0x626280 (its first 22 words).
        let ta: [u32; QUAD_WORDS] = [
            0xe000_0000,
            0x2260_0e80,
            0,
            0,
            0x43f0_0000,
            0,
            0,
            0x4420_0000,
            0x43f0_0000,
            0x4420_0000,
            0x3f80_0000,
            0x3f80_0000,
            0x3f80_0000,
            0x3f80_0000,
            0x3f80_0000,
            0x3f80_0000,
            0x3f80_0000,
            0x3f80_0000,
            0xff00_0000,
            0xffff_ffff,
            0xffff_e000,
            0xff10_20a0,
        ];
        assert_eq!(quad_object(m3::ISP_ALWAYS, TSP_SMOOTH, 1.0), ta);
        // Its list word 0x60a098a0: two triangles, layout 5.
        assert_eq!(0x60a0_98a0 & FLAGS, QUAD_FLAGS);
        assert_eq!((QUAD_FLAGS & LAYOUT) >> 21, 5);
        // z 0: only the four z words change.
        let z0 = quad_object(m3::ISP_ALWAYS, TSP_SMOOTH, 0.0);
        for (k, (&a, &b)) in z0.iter().zip(ta.iter()).enumerate() {
            assert_eq!(a == b, !(10..14).contains(&k), "word {k}");
        }
    }

    #[test]
    fn the_clear_object_is_the_tas_word_for_word() {
        // Way 4's clear at 0x64ba48 and its list word 0x40012e92.
        let ta = [
            0x2220_6f80,
            0,
            0,
            0x4470_0000,
            0,
            0,
            0x44a0_0000,
            0x3f80_0000,
            0x3f80_0000,
            0x3f80_0000,
        ];
        assert_eq!(clear_object(480, 640), ta);
        assert_eq!(0x4001_2e92 & FLAGS, CLEAR_FLAGS);
        assert_eq!(CLEAR_FLAGS & LAYOUT, 0);
        // The clear stream's TSP is the object's.
        assert_eq!(m3::ta::clear_stream(480, 640)[1], TSP_CLEAR);
    }

    #[test]
    fn the_tas_gouraud_triangle_is_ours_but_its_z() {
        // Way 1's second object at 0x626100, list word 0x40809840.
        let ta = [
            0xe000_0000,
            0x2260_0e80,
            0,
            0,
            0x43f0_0000,
            0,
            0,
            0x4420_0000,
            0x3f80_0000,
            0x3f80_0000,
            0x3f80_0000,
            0x3f80_0000,
            0x3f80_0000,
            0x3f80_0000,
            0xff00_0000,
            0xffff_ffff,
            0xffff_e000,
        ];
        let v = |k: usize, z| m3::Vertex {
            x: QUAD[k].0,
            y: QUAD[k].1,
            z,
            rhw: 1.0,
            argb: QUAD[k].2,
        };
        let ours = |z| m3::triangle(m3::ISP_ALWAYS, TSP_SMOOTH, &TRIS[0].map(|k| v(k, z)));
        assert_eq!(ours(1.0), ta);
        assert_ne!(ours(0.0), ta);
        assert_eq!(0x4080_9840 & FLAGS, m3::STRIP_ONE_TRIANGLE);
    }

    #[test]
    fn a_triangle_touches_the_tiles_it_covers_and_no_others() {
        let tile = |c: u32, r: u32| {
            let (x, y) = ((c * 8) as f32, (r * 16) as f32);
            (x, y, x + 8.0, y + 16.0)
        };
        let p = |k: usize| (QUAD[k].0, QUAD[k].1);
        let (a, b) = (TRIS[0].map(p), TRIS[1].map(p));
        // The corners: each half holds its own.
        assert!(touches(a, tile(0, 0)) && !touches(b, tile(0, 0)));
        assert!(touches(b, tile(59, 39)) && !touches(a, tile(59, 39)));
        // Both on the diagonal (480, 0)-(0, 640): the top-right and
        // bottom-left corners' tiles, and one across its middle.
        for t in [tile(59, 0), tile(0, 39), tile(30, 19)] {
            assert!(touches(a, t) && touches(b, t));
        }
        // Every tile touched by one or the other (the quad covers all).
        for r in 0..40 {
            for c in 0..60 {
                assert!(
                    touches(a, tile(c, r)) || touches(b, tile(c, r)),
                    "tile {c},{r}"
                );
            }
        }
        // Either winding; a tile well outside a small triangle.
        let small = [(10.0, 10.0), (20.0, 10.0), (10.0, 20.0)];
        let flipped = [small[0], small[2], small[1]];
        for t in [small, flipped] {
            assert!(touches(t, (8.0, 8.0, 16.0, 16.0)));
            assert!(!touches(t, (100.0, 100.0, 108.0, 116.0)));
            // Inside the box, beyond the hypotenuse.
            assert!(!touches(t, (18.0, 18.0, 19.0, 19.0)));
            // Wholly inside the triangle (no corner on any edge).
            assert!(touches(t, (11.0, 11.0, 12.0, 12.0)));
            // Thin strips past a corner: no edge separates them, only the
            // bounding box does (one across, one down).
            assert!(!touches(t, (21.0, 0.0, 22.0, 30.0)));
            assert!(!touches(t, (0.0, 21.0, 30.0, 22.0)));
        }
    }

    #[test]
    fn the_tally_places_wrong_pixels_on_the_edge_or_inside() {
        let mut t = Tally::default();
        t.add_px((0, 107), (480, 640), 0, 0x48ee_b980);
        t.add_px((479, 5), (480, 640), 0, 1);
        t.add_px((5, 639), (480, 640), 0, 1);
        t.add_px((5, 0), (480, 640), 0, 1);
        assert_eq!((t.wrong, t.edge, t.first_inner), (4, 4, None));
        t.add_px((38, 8), (480, 640), 0xff17_1714, 0xff17_1717);
        t.add_px((1, 1), (480, 640), 0, 1);
        t.add_px((2, 2), (480, 640), 5, 5);
        assert_eq!((t.wrong, t.edge), (6, 4));
        assert_eq!(
            t.first_inner,
            Some((8 * 480 + 38, 0xff17_1714, 0xff17_1717))
        );
        assert_eq!(t.first, Some((107 * 480, 0, 0x48ee_b980)));
        let mut u = Tally::default();
        u.merge(&t);
        u.merge(&t);
        assert_eq!((u.edge, u.first_inner), (8, t.first_inner));
    }
}

/// Test G3's pure parts: Dell's colour clear as read from `libGLES_CL.dll`
/// (A08) and the settings.
#[cfg(test)]
mod g3_tests {
    use super::*;
    use rp_drv_mbx_display::clock::Speed;
    use rp_drv_mbx3d as m3;

    #[test]
    fn dells_clear_is_an_opaque_flat_quad_over_the_viewport() {
        // The clear's TSP is Dell's flat one, the same as our background's;
        // its state block names TSP, ISP and the bit-6 word (no clip).
        assert_eq!(DELL_CLEAR_TSP, m3::TSP_DELL);
        assert_eq!(DELL_CLEAR_STATE & !TA_STATE, 0);
        assert_eq!(TA_STATE & !DELL_CLEAR_STATE, 1 << 4);
        let s = dell_clear_draw(m3::ISP_ALWAYS, 480, 640, 1.0, 0xFF00_0000);
        assert_eq!(s[..5], [0x1000_0043, 0x2220_0E80, 0xE000_0000, 0, TA_STRIP]);
        let one = 1.0f32.to_bits();
        let (w, h) = (480.0f32.to_bits(), 640.0f32.to_bits());
        // Top left, bottom left, top right, bottom right.
        assert_eq!(s[5..10], [0, 0, one, one, 0xFF00_0000]);
        assert_eq!(s[10..15], [0, h, one, one, 0xFF00_0000]);
        assert_eq!(s[15..20], [w, 0, one, one, 0xFF00_0000]);
        assert_eq!(s[20..25], [w, h, one, one, 0xFF00_0000]);
        // Depth and colour go to every vertex.
        let t = dell_clear_draw(m3::ISP_ALWAYS, 480, 640, 0.25, 0x1234_5678);
        for v in 0..4 {
            assert_eq!(t[5 + 5 * v + 2], 0.25f32.to_bits());
            assert_eq!(t[5 + 5 * v + 4], 0x1234_5678);
        }
    }

    #[test]
    fn a_four_corner_object_takes_its_corners_in_the_order_given() {
        let c = dell_clear_corners(480, 640, 0xFF00_0000);
        let o = four_corner_object(m3::ISP_ALWAYS, DELL_CLEAR_TSP, 1.0, c);
        let f = |v: f32| v.to_bits();
        assert_eq!(
            o[2..10],
            [0, 0, 0, f(640.0), f(480.0), 0, f(480.0), f(640.0)]
        );
        assert!(o[10..18].iter().all(|&w| w == f(1.0)));
        assert!(o[18..22].iter().all(|&w| w == 0xFF00_0000));
        // The quad's object is the same builder in the strip's order.
        assert_eq!(
            quad_object(m3::ISP_ALWAYS, TSP_SMOOTH, 0.0),
            four_corner_object(
                m3::ISP_ALWAYS,
                TSP_SMOOTH,
                0.0,
                STRIP_ORDER.map(|k| QUAD[k])
            )
        );
    }

    #[test]
    fn the_settings_are_the_clocks_named_with_dells_memory_timing() {
        let khz: [u32; 3] = G3_PLLS.map(|(p, _)| Speed::rated(p).khz());
        assert_eq!(khz, [74_750, 99_666, 133_250]);
        // 74.75 is the A75 setting test G ran; 99.67 Windows Mobile's own
        // timing (only the cores' divider differs from Windows Mobile).
        assert_eq!(Speed::rated(G3_PLLS[0].0), Speed::A75);
        assert_eq!(Speed::rated(G3_PLLS[1].0), Speed::WM);
        // CL2 at every setting, as GC2 and the soaks (Dell's rule would
        // give CL3 at 133.25: no 3D picture in run G1 at 108 MHz, none
        // rendered in G3's first run).
        assert_eq!(G_LMTYPE, Speed::WM.lmtype);
        assert_eq!(Speed::A75.lmtype, G_LMTYPE);
        // What the image runs: the clock driver's ADR-0035 ladder.
        for (pll, _) in G3_PLLS {
            assert_eq!(Speed::ladder(pll).lmtype, G_LMTYPE);
        }
        assert_eq!(
            Speed::rated(G3_PLLS[2].0).lmtype,
            0xEC8,
            "Dell's rule, which G3 overrides"
        );
        assert!(
            G3_PLLS
                .windows(2)
                .all(|p| Speed::rated(p[0].0).khz() < Speed::rated(p[1].0).khz())
        );
    }
}

/// Test G4's pure parts: Dell's render table as `rp-wmprobe` read it.
#[cfg(test)]
mod g4_tests {
    use super::*;
    use rp_drv_mbx3d as m3;

    #[test]
    fn dells_table_is_the_probes_reading_and_differs_from_ours_in_every_word() {
        // rp-wmprobe run 2 (local/rp-wmprobe-5.txt), Dell's driver at
        // 74.75 MHz / 1: "run 2 after: MBX 0x610 0x00030001" and so on.
        let probe: [(usize, u32); 9] = [
            (0x610, 0x0003_0001),
            (0x614, 0x1e3c_e500),
            (0x618, 0x1e3c_e500),
            (0x61c, 0x0002_0007),
            (0x648, 0x0000_0001),
            (0x650, 0x0000_0001),
            (0x660, 0x0000_00f0),
            (0x6d8, 0x0900_0000),
            (0x6dc, 0x3f80_0001),
        ];
        for &(off, v) in DELL_TABLE.iter().chain(DELL_TARGET16.iter()) {
            assert!(probe.contains(&(off, v)), "0x{off:x} 0x{v:x}");
        }
        assert_eq!(DELL_TABLE.len() + DELL_TARGET16.len(), probe.len());
        // Every word is one our start table writes (the image replaces it,
        // never adds it), and differs from ours (else it isn't a test).
        let ours = m3::start_table(0, 0, 480, 640, 480 * 4);
        for &(off, v) in DELL_TABLE.iter().chain(DELL_TARGET16.iter()) {
            let mine = ours.iter().find(|p| p.0 == off);
            assert!(mine.is_some(), "0x{off:x} not in our table");
            assert_ne!(mine.map(|p| p.1), Some(v), "0x{off:x} the same as ours");
        }
        // Never the start register.
        assert!(!DELL_TABLE.iter().any(|p| p.0 == m3::START));
    }
}
