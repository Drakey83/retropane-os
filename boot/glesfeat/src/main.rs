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

    /// `o` added to this one (the first wrong word kept if set).
    pub fn merge(&mut self, o: &Self) {
        self.wrong += o.wrong;
        self.one_step += o.one_step;
        self.unwritten += o.unwritten;
        self.max_step = self.max_step.max(o.max_step);
        if self.first.is_none() {
            self.first = o.first;
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
    const TEST_G: bool = option_env!("RP_GLESFEAT_G").is_some();
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
    const LABEL_LINES: usize = 8;
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
        let (t, len) = table();
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

    /// 74.75 MHz with the cores at divide 1 (`Speed::A75`: `COREPLL`
    /// `0xBC1`, the memory's timing for it), as `rp-gpubench`'s
    /// `clock_apply` steps to it: the long bus timing, the cores at divide
    /// 2, the PLL and its memory timing, then divide 1. False if the PLL
    /// didn't lock or a memory-controller write never held (Windows
    /// Mobile's clock is then back).
    fn to_divide_1<B: RegisterBus>(regs: &mut B) -> bool {
        let _ = diag::set_cs3_vlio(CS3_LONG.0, CS3_LONG.1, CS3_LONG.2);
        let clk = regs.read32(MBXCLK);
        clock::restore(regs, (clk & 3) | 1 << 2);
        rt::delay_us(1_000);
        let ok = match clock::set_speed(regs, Speed::A75, rt::delay_us) {
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
            t.add(
                i,
                load32_volatile(vram, reference + at).unwrap_or(0),
                load32_volatile(vram, TARGET + at).unwrap_or(0xDEAD_BEEF),
            );
        }
        t
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
    }

    impl Way {
        const fn name(self) -> &'static str {
            match self {
                Self::Ta => "through the TA",
                Self::Ours => "our objects, our layout 4",
                Self::OursTaFlags(_) => "our objects, the TA's list flags",
                Self::TaStrip4 => "through the TA, one strip of four",
            }
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

    /// What one way gave.
    #[derive(Default)]
    struct WayResult {
        /// The reference rendered twice the same.
        repeatable: bool,
        /// The clock reached 74.75 / 1.
        clocked: bool,
        /// Renders at divide 1 equal to the reference.
        right: u32,
        /// Renders at divide 1 that never finished.
        unfinished: u32,
        /// Every wrong word at divide 1.
        tally: Tally,
        /// Wrong renders whose second read (50 ms later) differed.
        unsettled: u32,
        /// Renders' times at Windows Mobile's clock and at divide 1 (the
        /// fastest), microseconds.
        us_wm: u32,
        us_d1: u32,
        /// The control at Windows Mobile's clock after: equal.
        control: bool,
    }

    /// One way: its reference at Windows Mobile's clock (rendered twice,
    /// equal; kept), 20 renders at 74.75 / 1 held to it, then a control at
    /// Windows Mobile's clock. `step` and `steps` label the screen.
    #[inline(never)]
    fn way(
        e: &mut Engine<Mmio>,
        vram: &mut [u8],
        w: Way,
        step: u32,
        steps: u32,
    ) -> (WayResult, [u32; TA_LIST]) {
        let mut r = WayResult::default();
        let mut head = [0u8; 32];
        let head = fmt(&mut head, format_args!("STEP {step} OF {steps}"));
        let mut name = [0u8; 32];
        let name = fmt(&mut name, format_args!("WAY {}: {}", step - 1, w.name()));
        say(
            vram,
            &["RP-GLESFEAT TEST G", head, name, "WM CLOCK: REFERENCE"],
        );
        log!("G way {} ({}):", step - 1, w.name());
        rt::check_in();
        let ready = clean(e, vram);
        let (d1, us1, list) = draw(e, vram, w, 24);
        keep_reference(vram);
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
            log!("  no repeatable reference: nothing at divide 1");
            return (r, list);
        }
        say(
            vram,
            &["RP-GLESFEAT TEST G", head, name, "74.75 MHZ / 1: SETTING"],
        );
        r.clocked = to_divide_1(&mut e.regs);
        clock_readback(&mut e.regs);
        if !r.clocked {
            log!(
                "  the clock change was refused (Windows Mobile's clock kept): nothing at divide 1"
            );
            return (r, list);
        }
        let ready = clean(e, vram);
        log!("  at 74.75 / 1: memory manager ready {ready}");
        r.us_d1 = u32::MAX;
        for n in 1..=G_DRAWS {
            rt::check_in();
            let mut line = [0u8; 32];
            let line = fmt(&mut line, format_args!("74.75 MHZ / 1: {n} OF {G_DRAWS}"));
            say(vram, &["RP-GLESFEAT TEST G", head, name, line]);
            let (done, us, _) = draw(e, vram, w, 0);
            r.us_d1 = r.us_d1.min(us);
            if !done {
                r.unfinished += 1;
                let _ = clean(e, vram);
            }
            let t = compare(vram, REF);
            if t.wrong == 0 {
                r.right += 1;
            } else {
                rt::delay_us(50_000);
                if compare(vram, REF) != t {
                    r.unsettled += 1;
                }
            }
            r.tally.merge(&t);
        }
        clock_back(&mut e.regs);
        clock_readback(&mut e.regs);
        let _ = clean(e, vram);
        let (done, _, _) = draw(e, vram, w, 0);
        r.control = done && compare(vram, REF).wrong == 0;
        let t = &r.tally;
        let _ = write!(
            RunLog,
            "  at 74.75 / 1: {} of {G_DRAWS} right ({} unfinished, {} read differently 50 ms later); wrong words {} ({} one step, {} never written, worst step {})",
            r.right, r.unfinished, r.unsettled, t.wrong, t.one_step, t.unwritten, t.max_step
        );
        if let Some((i, want, got)) = t.first {
            let _ = write!(
                RunLog,
                ", first at ({}, {}) 0x{got:08x} for 0x{want:08x}",
                i % W,
                i / W
            );
        }
        log!(
            "; fastest {} us (Windows Mobile's {} us); control at Windows Mobile's clock after: {}",
            r.us_d1,
            r.us_wm,
            if r.control { "right" } else { "WRONG" }
        );
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
        let (ta, list) = way(e, vram, Way::Ta, 2, 5);
        let (ours, _) = way(e, vram, Way::Ours, 3, 5);
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
                Some(way(e, vram, Way::OursTaFlags(f), 4, 5).0)
            }
            None => {
                log!("G way 3: skipped (the TA wrote no Gouraud object in the first tile)");
                None
            }
        };
        let (strip4, _) = way(e, vram, Way::TaStrip4, 5, 5);
        let count = |r: &WayResult| if r.clocked { r.right } else { 0 };
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
            "RP-GLESFEAT TEST G"
        } else {
            "RP-GLESFEAT A D L M"
        };
        say(vram, &[title, "STEP 1 OF 5", "SETTING UP THE 3D CORE"]);
        let clk0 = e.regs.read32(MBXCLK);
        if TEST_G {
            log!("{MARKER}: test G, Gouraud at the cores' divide 1 (the give-up audit's item 1)");
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
        if ready && TEST_G {
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
