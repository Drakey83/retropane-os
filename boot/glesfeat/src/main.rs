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

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_drv_blit2d::{self as cmd, Engine, Format, MBX, reg};
    use rp_drv_mbx_display::clock::with_3d;
    use rp_drv_mbx_display::{Display, PixelFormat, Plane, layers::Layer, reg as dreg};
    use rp_drv_mbx3d::{self as m3, ISP_ALWAYS, PM_READY, RENDER_DONE, TSP_DELL, Vertex};
    use rp_drv_pxa_keypad::{Keypad, Pressed};
    use rp_hal_api::RegisterBus;
    use rp_hal_mem::{load32_volatile, store32_volatile};
    use rp_hal_pxa27x::{Mmio, cpu, rt};
    use rp_hal_x51v::devices::RunLog;

    use super::{MARKER, TSP_ALPHA_TEST, alpha_word};

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

    fn main() -> ! {
        rt::init_mmu();
        let Some((regs, vram)) = rp_image::take_mbx() else {
            crash(format_args!("no 2700G"))
        };
        let mut display = Display::new(regs);
        let _ = display.adopt(vram.len());
        let regs = display.into_inner();
        let mut e = Engine::new(regs, FENCE, rt::ticks, 3_250);
        let clk0 = e.regs.read32(MBXCLK);
        log!("{MARKER}: alpha test, depth 'less', 8-bit grey textures (ADR-0026)");
        let _ = store32_volatile(vram, FENCE, 0);
        cpu::drain_write_buffer();
        let mut ready = false;
        if e.regs.read32(PLLSTAT) & 4 != 0 {
            e.regs.write32(MBXCLK, with_3d(clk0));
            rt::delay_us(1_000);
            e.reset(rt::delay_us);
            ready = pm_init(&mut e, vram);
        }
        log!(
            "MBXCLK 0x{clk0:x} -> 0x{:x}, parameter memory ready: {ready}",
            e.regs.read32(MBXCLK)
        );
        if ready {
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
            log!("A: alpha test, red square with vertex alpha 64 over grey");
            alpha(&mut e, vram);
            log!("D: depth, red z 0.5 then blue z 0.3 / 0.8, writes on");
            depth(&mut e, vram);
            log!("L: 8-bit grey textures (code 10)");
            luminance(&mut e, vram);
        }
        let saved = show_target(&mut e.regs);
        let keypad = arm_keypad();
        log!(
            "{MARKER}: {} - the last render shown until a button",
            if ready { "DONE" } else { "no parameter memory" }
        );
        hold_until_button(keypad, 10_000_000);
        for (off, v) in saved {
            e.regs.write32(off, v);
        }
        e.reset(rt::delay_us);
        e.regs.write32(MBXCLK, clk0);
        rp_image::finish()
    }

    fn show_target(regs: &mut Mmio) -> [(usize, u32); 5] {
        let plane = Plane {
            format: PixelFormat::Argb8888,
            width: W as u16,
            height: H as u16,
            offset: TARGET,
            stride: STRIDE,
        };
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
