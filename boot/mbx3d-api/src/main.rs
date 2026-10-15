//! Test image (ADR-0026): RetroPane's own 3D renderer (`rp-mbx3d-render`)
//! on the device, under the loader. Every result `rp-mbx3d` versions 19-23
//! found with hand-written words, again through the renderer's API, each
//! checked against the value the device gave then (the host model gives
//! the same: `cargo test -p rp-mbx3d-render`):
//!
//! - **S1 (T1, dirty tiles):** the whole screen, then one tile, a 10 x 10
//!   block and a 600-tile band, unlisted tiles untouched; a frame with no
//!   change renders nothing; a small square moved five times, each frame's
//!   cost logged (tiles, lists written, words to 2700G memory, register
//!   writes, render time against the prediction, status reads);
//! - **S2 (T3, sub-rectangles):** a 32 x 64 renderer drawing into the
//!   picture at (0,0), (0,100), (64,100), (8,100), (1,100);
//! - **S3 (T5, blending):** alpha 0x80 over red `0xBF7F007F`, an ARGB4444
//!   texture at alpha 8/15 `0xBF770087`, additive `0xFFFF00FF`;
//! - **S4 (T6, textures):** the 8 x 8 checker texel-exact, and wrap,
//!   mirror and clamp past u = 1;
//! - **S5 (T7, depth):** larger z nearer with "greater" (run 3: "less" moved
//!   to run 3's S8, which showed it never passes and, in a partial render,
//!   hides an "always" quad below: the renderer now refuses it, checked);
//! - **S6 Gouraud** corners, logged;
//! - **S7 the 3D clock gate:** off after 100 ms idle, on again for the next
//!   frame, which still draws.
//!
//! The renderer waits by its predicted render time and reads the status
//! once (ADR-0026 D3); this image busy-waits for that time (no scheduler
//! here). Run 2: the background writes depth (run 1's S5 failed where
//! nothing had: a tile's depth isn't cleared), and each frame's exact render
//! time is measured beside the prediction (now counting quads a tile), to
//! calibrate it. Unattended: the last picture stays 10 s or until a button; the
//! plane, the core and the clock are put back.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed with the verdict.
pub const MARKER: &str = "RETROPANE-MBX3D-API";

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_drv_blit2d::{self as cmd, Engine, Format, MBX, reg};
    use rp_drv_mbx_display::clock::{MBXCLK, has_3d};
    use rp_drv_mbx_display::{Display, PixelFormat, Plane, layers::Layer, reg as dreg};
    use rp_drv_pxa_keypad::{Keypad, Pressed};
    use rp_drv_pxa_uart::PxaUart;
    use rp_hal_api::RegisterBus;
    use rp_hal_mem::{load32_volatile, store32_volatile};
    use rp_hal_pxa27x::{Mmio, cpu, rt};
    use rp_hal_x51v::devices::RunLog;
    const RENDER_DONE: u32 = 1 << 2;
    use rp_mbx3d_render::{
        self as r3d, Clock, Depth, Draw, Error, FrameStats, LocalMem, Paint, Picture, Poll,
        Renderer, Target, UV_ONE, address, depth, factor, format,
    };
    use rp_vram_map::{self as map, Pool};

    use super::MARKER;

    rp_hal_pxa27x::entry!(main);

    macro_rules! log {
        ($($t:tt)*) => {{ let _ = writeln!(RunLog, $($t)*); }};
    }

    const GREY: u32 = 0xFF20_2020;
    const RED: u32 = 0xFFFF_0000;
    const GREEN: u32 = 0xFF00_FF00;
    const BLUE: u32 = 0xFF00_00FF;
    const WHITE: u32 = 0xFFFF_FFFF;
    const BLACK: u32 = 0xFF00_0000;
    const SCREEN: Picture = Picture::screen(0);
    const FULL: Target = Target {
        picture: SCREEN,
        x: 0,
        y: 0,
    };

    /// 2700G local memory through the CPU's window onto it.
    struct Vram(&'static mut [u8]);

    impl LocalMem for Vram {
        fn store(&mut self, at: usize, words: &[u32]) {
            for (k, &w) in words.iter().enumerate() {
                let _ = store32_volatile(self.0, at + 4 * k, w);
            }
        }

        fn load(&self, at: usize) -> Option<u32> {
            load32_volatile(self.0, at)
        }

        fn sync(&mut self) {
            cpu::drain_write_buffer();
        }
    }

    /// Microseconds from OSCR0 (3.25 MHz), kept running across its wrap.
    struct DevClock {
        last: u32,
        us: u32,
        rest: u32,
    }

    impl Clock for DevClock {
        fn now_us(&mut self) -> u32 {
            let t = rt::ticks();
            let d = u64::from(t.wrapping_sub(self.last)) * 4 + u64::from(self.rest);
            self.last = t;
            self.us = self.us.wrapping_add((d / 13) as u32);
            self.rest = (d % 13) as u32;
            self.us
        }

        fn delay_us(&mut self, us: u32) {
            rt::delay_us(us);
        }
    }

    /// Everything a stage needs.
    struct Rig {
        e: Engine<Mmio>,
        mem: Vram,
        clock: DevClock,
        pool: Pool,
        checks: u32,
        bad: u32,
        resets: u32,
    }

    impl Rig {
        fn px(&self, x: u32, y: u32) -> u32 {
            self.mem
                .load((SCREEN.at + y * SCREEN.stride + 4 * x) as usize)
                .unwrap_or(0xDEAD_BEEF)
        }

        /// Pixel `(x, y)` is `want`; logged either way.
        fn check(&mut self, what: &str, x: u32, y: u32, want: u32) -> bool {
            let got = self.px(x, y);
            self.checks += 1;
            if got == want {
                log!("    ok   {what} ({x},{y}) 0x{got:08x}");
                true
            } else {
                self.bad += 1;
                log!("    BAD  {what} ({x},{y}) 0x{got:08x}, want 0x{want:08x}");
                false
            }
        }

        fn expect(&mut self, what: &str, ok: bool) {
            self.checks += 1;
            if ok {
                log!("    ok   {what}");
            } else {
                self.bad += 1;
                log!("    BAD  {what}");
            }
        }

        /// Fills the screen picture with the 2D engine (and waits).
        fn clear(&mut self, c: u32) {
            let w = cmd::fill(
                Format::BPP32,
                SCREEN.at as usize,
                SCREEN.stride as usize,
                (0, 0, SCREEN.w, SCREEN.h),
                c,
            );
            let ok = self
                .e
                .submit(&w)
                .and_then(|()| self.e.finish(self.mem.0, 200_000))
                .is_ok();
            if !ok {
                log!("  the 2D clear didn't finish");
            }
        }

        #[inline(never)]
        fn set(&mut self, r: &mut Renderer, slot: usize, d: Option<Draw>) {
            if let Err(err) = r.set(&mut self.mem, slot, d.as_ref()) {
                log!("  set {slot}: {err:?}");
            }
        }

        /// One frame into `t`: render, wait for the predicted time, poll
        /// (counting status reads) until done. Logs its cost.
        fn frame(&mut self, r: &mut Renderer, t: Target, what: &str) -> Option<FrameStats> {
            rt::check_in();
            let res = r.render(&mut self.e.regs, &mut self.mem, &mut self.clock, t);
            if r.core_resets() != self.resets {
                // The renderer reset the MBX: the 2D stream restarts.
                self.resets = r.core_resets();
                self.e.after_core_reset();
            }
            let mut at = match res {
                Ok(Some(at)) => at,
                Ok(None) => {
                    log!("  {what}: nothing to render");
                    return None;
                }
                Err(err) => {
                    log!("  {what}: {err:?}");
                    return None;
                }
            };
            // Calibration (run 2): the exact finish, by watching the status
            // bit (it stays set until the renderer's poll clears it). The
            // renderer itself reads it once at the predicted time.
            let t0 = self.clock.now_us();
            let mut exact = None;
            while self.clock.now_us().wrapping_sub(t0) < 200_000 {
                if self.e.regs.read32(MBX + reg::STATUS) & RENDER_DONE != 0 {
                    exact = Some(self.clock.now_us().wrapping_sub(t0));
                    break;
                }
            }
            let mut polls = 0;
            loop {
                while (self.clock.now_us().wrapping_sub(at) as i32) < 0 {}
                polls += 1;
                match r.poll(&mut self.e.regs, &mut self.mem, &mut self.clock) {
                    Poll::Wait(t) => at = t,
                    Poll::Done(s) => {
                        log!(
                            "  {what}: {} tiles, {} quads listed, {} lists written, {} words to memory, {} register writes; exact {exact:?} us, predicted {}",
                            s.tiles,
                            s.quads,
                            s.lists,
                            s.mem_words,
                            s.reg_writes,
                            s.predicted_us
                        );
                        return Some(s);
                    }
                    p => {
                        if r.core_resets() != self.resets {
                            self.resets = r.core_resets();
                            self.e.after_core_reset();
                        }
                        log!("  {what}: {p:?} after {polls} poll(s)");
                        return None;
                    }
                }
            }
        }
    }

    fn rect(x: i32, y: i32, w: i32, h: i32, c: u32) -> Draw {
        Draw::rect(x, y, w, h, Paint::Flat(c))
    }

    /// S1 (T1): dirty tiles.
    #[inline(never)]
    fn s1(g: &mut Rig) {
        log!("S1 (T1): only changed tiles rendered, unlisted ones untouched");
        let Some(mut r) = Renderer::new(&mut g.pool, 480, 640) else {
            log!("  no renderer");
            return;
        };
        let _ = r.set_background(Some(GREY));
        let s = g.frame(&mut r, FULL, "whole screen");
        g.expect("2400 tiles", s.is_some_and(|s| s.tiles == 2400));
        g.check("grey", 0, 0, GREY);
        g.check("grey", 479, 639, GREY);

        g.set(&mut r, 0, Some(rect(160, 320, 8, 16, BLUE)));
        let s = g.frame(&mut r, FULL, "tile (20,20)");
        g.expect("1 tile", s.is_some_and(|s| s.tiles == 1));
        g.check("tile corner", 160, 320, BLUE);
        g.check("tile corner", 167, 335, BLUE);
        g.check("right neighbour", 168, 328, GREY);
        g.check("neighbour below", 164, 336, GREY);

        g.set(&mut r, 0, Some(rect(120, 240, 80, 160, BLUE)));
        let s = g.frame(&mut r, FULL, "10 x 10 from (15,15)");
        g.expect("100 tiles", s.is_some_and(|s| s.tiles == 100));
        g.check("block corner", 120, 240, BLUE);
        g.check("block corner", 199, 399, BLUE);
        g.check("right of it", 200, 300, GREY);
        g.check("below it", 150, 400, GREY);

        g.set(&mut r, 0, Some(rect(0, 240, 480, 160, BLUE)));
        let s = g.frame(&mut r, FULL, "rows 15-24");
        g.expect("600 tiles", s.is_some_and(|s| s.tiles == 600));
        g.check("band", 5, 245, BLUE);
        g.check("above", 5, 239, GREY);
        g.check("below", 5, 400, GREY);

        let res = r.render(&mut g.e.regs, &mut g.mem, &mut g.clock, FULL);
        g.expect("no change: nothing rendered", res == Ok(None));

        log!("  a 16 x 16 square moved 8 px a frame:");
        g.set(&mut r, 0, None);
        g.set(&mut r, 1, Some(rect(40, 40, 16, 16, RED)));
        g.frame(&mut r, FULL, "band gone, square in");
        for k in 1..=5 {
            g.set(&mut r, 1, Some(rect(40 + 8 * k, 40, 16, 16, RED)));
            g.frame(&mut r, FULL, "moved");
        }
        g.check("square", 80, 40, RED);
        g.check("left behind", 79, 40, GREY);
        g.check("square's right edge", 95, 55, RED);
        g.check("past it", 96, 55, GREY);
        let _ = r.release(&mut g.pool);
    }

    /// S2 (T3): a sub-rectangle target at pixel offsets.
    #[inline(never)]
    fn s2(g: &mut Rig) {
        log!("S2 (T3): a 32 x 64 target inside the picture, stride kept");
        for (x, y) in [(0u16, 0u16), (0, 100), (64, 100), (8, 100), (1, 100)] {
            g.clear(GREY);
            let Some(mut r) = Renderer::new(&mut g.pool, 32, 64) else {
                log!("  no renderer");
                return;
            };
            let _ = r.set_background(Some(BLUE));
            let s = g.frame(
                &mut r,
                Target {
                    picture: SCREEN,
                    x,
                    y,
                },
                "block",
            );
            g.expect("16 tiles", s.is_some_and(|s| s.tiles == 16));
            let (x, y) = (u32::from(x), u32::from(y));
            g.check("corner", x, y, BLUE);
            g.check("corner", x + 31, y + 63, BLUE);
            g.check("right of it", x + 32, y, GREY);
            g.check("below it", x, y + 64, GREY);
            if x > 0 {
                g.check("left of it", x - 1, y, GREY);
            }
            if (x, y) != (0, 0) {
                g.check("picture origin", 0, 0, GREY);
            }
            let _ = r.release(&mut g.pool);
        }
    }

    /// S3 (T5): blending.
    #[inline(never)]
    fn s3(g: &mut Rig) {
        log!("S3 (T5): blending (TSP bit 13, factors in bits 31-26)");
        let Some(mut r) = Renderer::new(&mut g.pool, 480, 640) else {
            return;
        };
        let _ = r.set_background(Some(RED));
        g.set(
            &mut r,
            0,
            Some(rect(100, 100, 200, 200, 0x8000_00FF).over()),
        );
        let tex = r3d::upload(&mut g.pool, &mut g.mem, 0, format::ARGB4444, &[0x800F; 64]);
        if let Some(t) = tex {
            g.set(
                &mut r,
                1,
                Some(Draw::rect(320, 100, 64, 64, Paint::texture(t)).over()),
            );
        }
        g.frame(&mut r, FULL, "alpha over red");
        g.check("blue at alpha 0x80", 200, 200, 0xBF7F_007F);
        g.check("ARGB4444 at alpha 8/15", 350, 130, 0xBF77_0087);
        g.set(
            &mut r,
            0,
            Some(Draw {
                blend: Some((factor::ONE, factor::ONE)),
                ..rect(100, 100, 200, 200, BLUE)
            }),
        );
        g.frame(&mut r, FULL, "additive");
        g.check("red + blue", 200, 200, 0xFFFF_00FF);
        if let Some(t) = tex {
            let _ = r3d::release(&mut g.pool, t);
        }
        let _ = r.release(&mut g.pool);
    }

    /// S4 (T6): textures.
    #[inline(never)]
    fn s4(g: &mut Rig) {
        log!("S4 (T6): twiddled textures texel-exact, address modes past u = 1");
        let Some(mut r) = Renderer::new(&mut g.pool, 480, 640) else {
            return;
        };
        let _ = r.set_background(Some(BLACK));
        // The checker: red/white, texel (7,0) green; 8 px a texel.
        let mut checker = [0u16; 64];
        for (i, t) in checker.iter_mut().enumerate() {
            *t = if (i % 8 + i / 8) % 2 == 0 {
                0xF800
            } else {
                0xFFFF
            };
        }
        checker[7] = 0x07E0;
        // Columns of pure colours (no channel-widening question).
        const COLS: [u16; 8] = [
            0x0000, 0x001F, 0x07E0, 0x07FF, 0xF800, 0xF81F, 0xFFE0, 0xFFFF,
        ];
        const SHOWN: [u32; 8] = [
            0xFF00_0000,
            0xFF00_00FF,
            0xFF00_FF00,
            0xFF00_FFFF,
            0xFFFF_0000,
            0xFFFF_00FF,
            0xFFFF_FF00,
            0xFFFF_FFFF,
        ];
        let mut columns = [0u16; 64];
        for (i, t) in columns.iter_mut().enumerate() {
            *t = COLS[i % 8];
        }
        let a = r3d::upload(&mut g.pool, &mut g.mem, 0, format::RGB565, &checker);
        let b = r3d::upload(&mut g.pool, &mut g.mem, 0, format::RGB565, &columns);
        let (Some(a), Some(b)) = (a, b) else {
            log!("  no texture memory");
            return;
        };
        g.set(
            &mut r,
            0,
            Some(Draw::rect(100, 100, 64, 64, Paint::texture(a))),
        );
        let modes = [
            ("wrap", address::WRAP, 0usize, 7usize),
            ("mirror", address::MIRROR, 7, 0),
            ("clamp", address::CLAMP, 7, 7),
        ];
        let uv = [(0, 0), (2 * UV_ONE, 0), (2 * UV_ONE, UV_ONE), (0, UV_ONE)];
        for (k, &(_, mode, _, _)) in modes.iter().enumerate() {
            let tex = b.with_modes(mode, address::CLAMP);
            let paint = Paint::Texture {
                tex,
                uv,
                tint: WHITE,
            };
            g.set(
                &mut r,
                1 + k,
                Some(Draw::rect(0, 200 + 80 * k as i32, 128, 64, paint)),
            );
        }
        g.frame(&mut r, FULL, "textures");
        g.check("texel (0,0)", 100, 100, RED);
        g.check("texel (0,0) far corner", 107, 107, RED);
        g.check("texel (1,0)", 108, 100, WHITE);
        g.check("texel (0,1)", 100, 108, WHITE);
        g.check("texel (7,0)", 156, 100, GREEN);
        g.check("texel (7,0) far corner", 163, 107, GREEN);
        g.check("texel (7,1)", 163, 108, RED);
        g.check("past the texture", 164, 100, BLACK);
        for (k, &(name, _, after, end)) in modes.iter().enumerate() {
            let y = 210 + 80 * k as u32;
            log!("  {name}:");
            g.check("column 0", 4, y, SHOWN[0]);
            g.check("column 7", 60, y, SHOWN[7]);
            g.check("just past u = 1", 68, y, SHOWN[after]);
            g.check("near u = 2", 124, y, SHOWN[end]);
        }
        let _ = r3d::release(&mut g.pool, a);
        let _ = r3d::release(&mut g.pool, b);
        let _ = r.release(&mut g.pool);
    }

    /// S5 (T7): depth.
    #[inline(never)]
    fn s5(g: &mut Rig) {
        log!("S5 (T7): depth, larger z nearer; red z 0.5 writing, blue over it");
        let Some(mut r) = Renderer::new(&mut g.pool, 480, 640) else {
            return;
        };
        let _ = r.set_background(Some(GREY));
        let z = |tenths: u32| (tenths * 65536 / 10) as u16;
        g.set(
            &mut r,
            0,
            Some(rect(100, 100, 200, 200, RED).with_depth(Depth {
                mode: depth::ALWAYS,
                write: true,
                z: z(5),
            })),
        );
        for (name, mode, zb, overlap, alone) in [
            ("greater, z 0.8", depth::GREATER, 8, BLUE, BLUE),
            ("greater, z 0.3", depth::GREATER, 3, RED, BLUE),
            ("greater or equal, z 0.3", depth::GEQUAL, 3, RED, BLUE),
        ] {
            g.set(
                &mut r,
                1,
                Some(rect(200, 200, 200, 200, BLUE).with_depth(Depth {
                    mode,
                    write: true,
                    z: z(zb),
                })),
            );
            g.frame(&mut r, FULL, name);
            g.check("overlap", 250, 250, overlap);
            g.check("blue alone", 350, 350, alone);
            g.check("red alone", 150, 150, RED);
        }
        let less = rect(200, 200, 200, 200, BLUE).with_depth(Depth {
            mode: depth::LESS,
            write: true,
            z: z(3),
        });
        let refused = r.set(&mut g.mem, 1, Some(&less)) == Err(Error::Unsupported);
        g.expect("\"less\" refused (run 3: broken on the device)", refused);
        let _ = r.release(&mut g.pool);
    }

    /// S6 Gouraud, S7 the clock gate; leaves the Gouraud square on screen.
    #[inline(never)]
    fn s6_s7(g: &mut Rig) {
        log!("S6: Gouraud quad, corners red/green/blue/white (logged)");
        let Some(mut r) = Renderer::new(&mut g.pool, 480, 640) else {
            return;
        };
        let _ = r.set_background(Some(GREY));
        let c = [RED, GREEN, BLUE, WHITE];
        g.set(
            &mut r,
            0,
            Some(Draw::rect(112, 192, 256, 256, Paint::Gouraud(c))),
        );
        g.frame(&mut r, FULL, "Gouraud");
        log!(
            "  corners 0x{:08x} 0x{:08x} 0x{:08x} 0x{:08x}, middle 0x{:08x}",
            g.px(112, 192),
            g.px(367, 192),
            g.px(367, 447),
            g.px(112, 447),
            g.px(240, 320)
        );

        log!("S7: the 3D clock gate: off after 100 ms idle, on for the next frame");
        let now = g.clock.now_us();
        let off = r.idle_due().is_some_and(|due| {
            while (g.clock.now_us().wrapping_sub(due) as i32) < 0 {
                rt::check_in();
            }
            r.idle(&mut g.e.regs, due)
        });
        let clk = g.e.regs.read32(MBXCLK);
        log!(
            "  idle {} ms: switched off {off}, MBXCLK 0x{clk:x}",
            g.clock.now_us().wrapping_sub(now) / 1000
        );
        g.expect("3D clock off when idle", off && !has_3d(clk));
        g.set(&mut r, 1, Some(rect(208, 288, 64, 64, BLACK)));
        g.frame(&mut r, FULL, "clock back on");
        let clk = g.e.regs.read32(MBXCLK);
        g.expect("3D clock on again", has_3d(clk));
        g.check("drawn after it", 240, 320, BLACK);
        let _ = r.release(&mut g.pool);
    }

    fn main() -> ! {
        rt::set_fault_handler(on_fault);
        rt::init_mmu();
        let Some((regs, vram)) = rp_hal_x51v::devices::take_mbx() else {
            crash(format_args!("no 2700G"))
        };
        let mut display = Display::new(regs);
        let _ = display.adopt(vram.len());
        let regs = display.into_inner();
        let mut e = Engine::new(regs, map::FENCE.at as usize, rt::ticks, 3_250);
        let clk0 = e.regs.read32(MBXCLK);
        let _ = store32_volatile(vram, map::FENCE.at as usize, 0);
        cpu::drain_write_buffer();
        log!(
            "{MARKER}: RetroPane's own 3D renderer through its API (ADR-0026), run 4 (depth modes the device broke refused), MBXCLK 0x{clk0:x}"
        );
        let mut g = Rig {
            e,
            mem: Vram(vram),
            clock: DevClock {
                last: rt::ticks(),
                us: 0,
                rest: 0,
            },
            pool: Pool::new(map::POOL),
            checks: 0,
            bad: 0,
            resets: 0,
        };
        g.clear(BLACK);
        let saved = show_screen(&mut g.e.regs);
        s1(&mut g);
        s2(&mut g);
        s3(&mut g);
        s4(&mut g);
        s5(&mut g);
        s6_s7(&mut g);
        log!(
            "pool: {} bytes free of {} (everything given back: {})",
            g.pool.free_bytes(),
            map::POOL.size,
            g.pool.free_bytes() == map::POOL.size
        );

        let keypad = arm_keypad();
        let (checks, bad) = (g.checks, g.bad);
        log!("{MARKER}: {checks} checks, {bad} BAD - the Gouraud square shown until a button");
        if let Some(bus) = rt::take_ffuart() {
            let mut uart = PxaUart::new(bus);
            uart.init(115_200);
            let _ = writeln!(uart, "{MARKER}: {checks} checks, {bad} BAD");
        }
        hold_until_button(keypad, 10_000_000);
        for (off, v) in saved {
            g.e.regs.write32(off, v);
        }
        g.e.reset(rt::delay_us);
        g.e.regs.write32(MBXCLK, clk0);
        rp_hal_x51v::devices::return_to_loader();
        loop {
            core::hint::spin_loop();
        }
    }

    /// Points the graphics plane at the screen picture (opaque); returns
    /// the registers as found.
    fn show_screen(regs: &mut Mmio) -> [(usize, u32); 5] {
        let plane = Plane {
            format: PixelFormat::Argb8888,
            width: SCREEN.w,
            height: SCREEN.h,
            offset: SCREEN.at as usize,
            stride: SCREEN.stride as usize,
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

    /// The keypad, enabled with earlier activity discarded.
    fn arm_keypad() -> Option<Keypad<Mmio>> {
        let kp = rt::take_keypad()?;
        let mut keypad = Keypad::new(kp);
        keypad.enable();
        keypad.acknowledge();
        Some(keypad)
    }

    /// Holds until a button is pressed and released (`limit_us` at most).
    fn hold_until_button(keypad: Option<Keypad<Mmio>>, limit_us: u32) {
        let Some(mut keypad) = keypad else {
            return;
        };
        let t0 = rt::ticks();
        let mut seen = false;
        while rt::ticks().wrapping_sub(t0) / 13 * 4 < limit_us {
            rt::check_in();
            seen |= keypad.acknowledge();
            if seen && keypad.pressed() == Pressed::NONE {
                return;
            }
            rt::delay_us(20_000);
        }
        log!("no button: returning");
    }

    fn on_fault(kind: rt::Fault, addr: u32) -> ! {
        crash(format_args!("FAULT {} at {addr:#010x}", kind.name()))
    }

    #[inline(never)]
    fn crash(msg: core::fmt::Arguments) -> ! {
        rp_hal_x51v::devices::crash_to_loader(msg);
        loop {
            core::hint::spin_loop();
        }
    }

    #[panic_handler]
    fn panic(info: &core::panic::PanicInfo) -> ! {
        crash(format_args!("PANIC {}", info.message()))
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}
