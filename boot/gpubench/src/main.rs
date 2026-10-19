//! Test image: the GPU facts the 3D compositor proposal needs
//! (docs/design/ui.md, "3D acceleration for the whole UI": the owner votes
//! on options A/B/C once these are in). Under the loader, Windows Mobile's
//! clock (the MBX cores at 49.8 MHz; times follow the clock exactly,
//! `rp-mbx3d` run 26), no board bits, no sound. Results only: nothing here
//! decides anything.
//!
//! **Run A** (the default build):
//!
//! - **G4a, a rectangular texture:** 64 x 32, 32 x 64 and 512 x 1024 (a
//!   480 x 640 layer) RGB565 textures, uploaded in the GLES driver's order
//!   (`tex::twiddle_rect`, static) with TEX0's two size codes apart, drawn
//!   1:1; every texel's position is in its colour, so the log says which
//!   texel each checked pixel showed;
//! - **G1, fill rate:** one full-screen quad, 5 renders each, exact times:
//!   flat, flat translucent, textured point and bilinear, translucent
//!   textured point and bilinear; texel formats (565, 8888, PVRTC 4);
//!   scales (8x magnified, 2:1 shrunk, mipmapped); on the 32-bit screen
//!   picture and a 16-bit RGB565 picture;
//! - **G2, N full-screen layers:** 1 to 8, opaque textured, translucent
//!   textured (point, bilinear), translucent flat, both targets;
//! - **G3, partial frames:** a band (600 tiles), a row (60), an icon (32)
//!   and one tile, under three translucent layers and under one opaque;
//! - **G5, a layer's upload:** a 480 x 640 layer from SDRAM by the CPU in
//!   row order (16 and 32 bits), into a 512 x 1024 texture's order, the
//!   same order made in SDRAM then copied by DMA (with the CPU's free
//!   share), and a 480 x 48 strip both ways;
//! - **W1-W3, in step with the frame event (the owner watches; each runs
//!   until a button, 30 s at most):** W1 whether a screen flip
//!   (`GSCADR`) takes effect at once or at the next frame (a band shows
//!   if at once); W2 a bar moved by full renders into the hidden one of
//!   two screens, flipped at `VEVENT0`; W3 one screen, rendered in four
//!   bands just behind the scan line. Each logs its timings.
//!
//! **Run B** (built with `RP_GPUBENCH_PROBES`): TEX0 words Dell's driver
//! never sets (bits 19-18, the format field's odd codes, bits 8-0), one at
//! a time, on a texture written in row order: whether any reads it as rows
//! (a non-twiddled, any-width texture). A hung render is recovered by the
//! renderer (core reset), and the log line is written before each try.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed with the verdict.
pub const MARKER: &str = "RETROPANE-GPUBENCH";

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_drv_blit2d::{self as cmd, Engine, Format, MBX, reg};
    use rp_drv_mbx_display::clock::MBXCLK;
    use rp_drv_mbx_display::font::{self, WIDTH};
    use rp_drv_mbx_display::{
        Display, PixelFormat as PlaneFormat, Plane, event, layers::Layer, reg as dreg,
    };
    use rp_drv_mbx3d::tex;
    use rp_drv_pxa_keypad::{Keypad, Pressed};
    use rp_drv_pxa_uart::PxaUart;
    use rp_hal_api::{DmaEngine, RegisterBus};
    use rp_hal_mem::{load32_volatile, store32_volatile, words_as_bytes};
    use rp_hal_pxa27x::{Mmio, cpu, rt};
    use rp_hal_x51v::devices::RunLog;
    use rp_mbx3d_render::{
        Clock, Draw, LocalMem, Paint, Picture, PixelFormat, Poll, Renderer, Target, Texture,
        UV_ONE, address, format, untwiddle,
    };
    use rp_vram_map::{self as map, Pool};

    use super::MARKER;

    rp_hal_pxa27x::entry!(main);

    macro_rules! log {
        ($($t:tt)*) => {{ let _ = writeln!(RunLog, $($t)*); }};
    }

    /// Run B: the texture-word probes instead of run A.
    const PROBES: bool = option_env!("RP_GPUBENCH_PROBES").is_some();

    const RENDER_DONE: u32 = 1 << 2;
    const GREY: u32 = 0xFF20_2020;
    const WHITE: u32 = 0xFFFF_FFFF;
    const BLACK: u32 = 0xFF00_0000;
    const BLUE: u32 = 0xFF10_20A0;
    const YELLOW: u32 = 0xFFFF_E000;
    const SCREEN: Picture = Picture::screen(0);
    /// A 16-bit target where the background picture would go (not shown).
    const PIC16: Picture = Picture::packed(map::BACKGROUND.at, 480, 640, PixelFormat::Rgb565);
    const TILES: u32 = 2400;
    /// A frame of the panel (56.46 Hz, `rp-mbx-blend` run 1).
    const FRAME_US: u32 = 17_712;
    /// Watch tests end on a button, or after this long.
    const WATCH_US: u32 = 30_000_000;
    /// The 2700G's local memory as the CPU and DMA see it (CS3).
    const CS3: u32 = 0x0C00_0000;
    /// One DMA transfer: 120 KiB (under the controller's 128 KiB chain).
    const DMA_PIECE: usize = 120 * 1024;

    fn us_since(t0: u32) -> u32 {
        rt::ticks().wrapping_sub(t0) / 13 * 4
    }

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
        resets: u32,
        hangs: u32,
        measured: u32,
        keypad: Option<Keypad<Mmio>>,
        pressed: bool,
    }

    /// Up to eight times, their median, least and most.
    #[derive(Default)]
    struct Times {
        t: [u32; 8],
        n: usize,
    }

    impl Times {
        fn add(&mut self, us: u32) {
            if self.n < self.t.len() {
                self.t[self.n] = us;
                self.n += 1;
            }
        }

        fn median(&self) -> u32 {
            let mut s = self.t;
            s[..self.n].sort_unstable();
            if self.n == 0 { 0 } else { s[self.n / 2] }
        }

        fn min(&self) -> u32 {
            self.t[..self.n].iter().copied().min().unwrap_or(0)
        }

        fn max(&self) -> u32 {
            self.t[..self.n].iter().copied().max().unwrap_or(0)
        }
    }

    impl Rig {
        fn px(&self, x: u32, y: u32) -> u32 {
            self.mem
                .load((SCREEN.at + y * SCREEN.stride + 4 * x) as usize)
                .unwrap_or(0xDEAD_BEEF)
        }

        fn sync_resets(&mut self, r: &Renderer) {
            if r.core_resets() != self.resets {
                // The renderer reset the MBX: the 2D stream restarts.
                self.resets = r.core_resets();
                self.e.after_core_reset();
            }
        }

        /// One frame into `t`: the exact render time (watching the status
        /// bit, as `rp-mbx3d-api` run 2 calibrated), then the renderer's
        /// own poll to the end. `None` if refused, hung or nothing to do.
        fn frame(&mut self, r: &mut Renderer, t: Target) -> Option<u32> {
            rt::check_in();
            let res = r.render(&mut self.e.regs, &mut self.mem, &mut self.clock, t);
            self.sync_resets(r);
            let mut at = match res {
                Ok(Some(at)) => at,
                Ok(None) => return None,
                Err(err) => {
                    log!("    render refused: {err:?}");
                    return None;
                }
            };
            let t0 = self.clock.now_us();
            let mut exact = None;
            while self.clock.now_us().wrapping_sub(t0) < 500_000 {
                if self.e.regs.read32(MBX + reg::STATUS) & RENDER_DONE != 0 {
                    exact = Some(self.clock.now_us().wrapping_sub(t0));
                    break;
                }
            }
            loop {
                while (self.clock.now_us().wrapping_sub(at) as i32) < 0 {}
                match r.poll(&mut self.e.regs, &mut self.mem, &mut self.clock) {
                    Poll::Wait(t) => at = t,
                    Poll::Done(_) => {
                        self.measured += 1;
                        return exact;
                    }
                    p => {
                        self.sync_resets(r);
                        self.hangs += u32::from(p == Poll::Recovered);
                        log!("    {p:?}");
                        return None;
                    }
                }
            }
        }

        /// `reps` full renders of what `r` holds into `t`.
        fn full(&mut self, r: &mut Renderer, t: Target, reps: usize) -> Times {
            let mut times = Times::default();
            for _ in 0..reps {
                r.invalidate_all();
                if let Some(us) = self.frame(r, t) {
                    times.add(us);
                }
            }
            times
        }

        fn set(&mut self, r: &mut Renderer, slot: usize, d: Option<Draw>) {
            if let Err(err) = r.set(&mut self.mem, slot, d.as_ref()) {
                log!("    set {slot}: {err:?}");
            }
        }

        /// Fills a rectangle of the 32-bit picture at `at` with the 2D
        /// engine (and waits).
        fn fill(&mut self, at: u32, rect: (u16, u16, u16, u16), c: u32) {
            let w = cmd::fill(Format::BPP32, at as usize, SCREEN.stride as usize, rect, c);
            let ok = self
                .e
                .submit(&w)
                .and_then(|()| self.e.finish(self.mem.0, 200_000))
                .is_ok();
            if !ok {
                log!("  the 2D fill didn't finish");
            }
        }

        /// `s` in the 5 x 7 font, `scale` times, at `(x, y)` of the 32-bit
        /// picture at `at`.
        fn text(&mut self, at: u32, x: u32, y: u32, s: &str, c: u32, scale: u32) {
            for (k, ch) in s.chars().enumerate() {
                let x0 = x + k as u32 * (WIDTH as u32 + 1) * scale;
                for (gy, &bits) in font::glyph(ch).iter().enumerate() {
                    for gx in 0..WIDTH as u32 {
                        if bits & (1 << (WIDTH as u32 - 1 - gx)) == 0 {
                            continue;
                        }
                        for dy in 0..scale {
                            let py = y + gy as u32 * scale + dy;
                            let row = at + py * SCREEN.stride + 4 * (x0 + gx * scale);
                            let px = [c; 8];
                            self.mem.store(row as usize, &px[..scale.min(8) as usize]);
                        }
                    }
                }
            }
            self.mem.sync();
        }

        /// The keypad's activity forgotten: a watch test starts.
        fn arm_button(&mut self) {
            if let Some(k) = self.keypad.as_mut() {
                k.acknowledge();
            }
            self.pressed = false;
        }

        /// A button pressed and let go since [`Self::arm_button`].
        fn button(&mut self) -> bool {
            let Some(k) = self.keypad.as_mut() else {
                return false;
            };
            self.pressed |= k.acknowledge();
            self.pressed && k.pressed() == Pressed::NONE
        }

        fn line(&mut self) -> u32 {
            self.e.regs.read32(dreg::DVLNUM) & 0xFFF
        }

        /// Waits until the scan line is in `[from, from + span)` (wrapping
        /// at the frame's 645 lines); false after 40 ms (no scan line).
        fn wait_line(&mut self, from: u32, span: u32) -> bool {
            let t0 = rt::ticks();
            while us_since(t0) < 40_000 {
                let l = self.line();
                if (l + 645 - from) % 645 < span {
                    return true;
                }
            }
            false
        }

        /// Clears `DINTRS`'s VEVENT0 (Dell's way: the status written back
        /// without the bit; a 1 written would set the others).
        fn vevent_clear(&mut self) {
            let s = self.e.regs.read32(dreg::DINTRS);
            self.e.regs.write32(dreg::DINTRS, s & !event::VEVENT0);
        }

        /// Waits for VEVENT0 (once a frame, on `DVECTRL`'s line, 643);
        /// false after 40 ms.
        fn vevent_wait(&mut self) -> bool {
            let t0 = rt::ticks();
            while us_since(t0) < 40_000 {
                if self.e.regs.read32(dreg::DINTRS) & event::VEVENT0 != 0 {
                    return true;
                }
            }
            false
        }
    }

    // ---- textures ----------------------------------------------------

    /// Where texel index `i` of a `1 << bu` x `1 << bv` texture sits:
    /// the inverse of `tex::twiddle_rect`.
    const fn untwiddle_rect(i: u32, bu: u32, bv: u32) -> (u32, u32) {
        let short = if bu < bv { bu } else { bv };
        let (x, y) = untwiddle(i & ((1 << (2 * short)) - 1));
        let high = i >> (2 * short);
        if bu > bv {
            (x | high << short, y)
        } else {
            (x, y | high << short)
        }
    }

    /// Writes a `1 << bu` x `1 << bv` texture of 16-bit texels `f(x, y)`
    /// at `at`, in the twiddled order (`levels` mip levels of a square one
    /// after the other, each from `f` at the level's scale).
    fn write16(g: &mut Rig, at: u32, bu: u32, bv: u32, levels: u32, f: &dyn Fn(u32, u32) -> u16) {
        let mut off = 0;
        for k in 0..levels {
            let (lu, lv) = (bu.saturating_sub(k), bv.saturating_sub(k));
            let texels = 1u32 << (lu + lv);
            let words = texels.div_ceil(2);
            let mut buf = [0u32; 64];
            let mut m = 0;
            while m < words {
                let n = (words - m).min(64);
                for (j, w) in buf[..n as usize].iter_mut().enumerate() {
                    let i = 2 * (m + j as u32);
                    let (x0, y0) = untwiddle_rect(i, lu, lv);
                    let lo = u32::from(f(x0 << k, y0 << k));
                    let hi = if i + 1 < texels {
                        let (x1, y1) = untwiddle_rect(i + 1, lu, lv);
                        u32::from(f(x1 << k, y1 << k))
                    } else {
                        0
                    };
                    *w = lo | hi << 16;
                }
                g.mem.store((at + off + 4 * m) as usize, &buf[..n as usize]);
                m += n;
            }
            off += 4 * words;
            rt::check_in();
        }
        g.mem.sync();
    }

    /// A square texture of side `8 << code` from the pool, written by
    /// `f` (16-bit formats), wrapping both ways.
    fn tex16(
        g: &mut Rig,
        code: u8,
        fmt: u32,
        levels: u8,
        f: &dyn Fn(u32, u32) -> u16,
    ) -> Option<Texture> {
        let mut t = Texture {
            at: 0,
            code,
            format: fmt as u8,
            u_mode: address::WRAP as u8,
            v_mode: address::WRAP as u8,
            linear: false,
            levels,
        };
        t.at = g.pool.alloc(t.bytes(), 128)?;
        let bits = u32::from(code) + 3;
        write16(g, t.at, bits, bits, u32::from(levels), f);
        Some(t)
    }

    /// A square 32-bit texture of side `8 << code`, `0xAARRGGBB` from `f`.
    fn tex32(g: &mut Rig, code: u8, f: &dyn Fn(u32, u32) -> u32) -> Option<Texture> {
        let t = Texture {
            at: 0,
            code,
            format: format::ARGB8888 as u8,
            u_mode: address::WRAP as u8,
            v_mode: address::WRAP as u8,
            linear: false,
            levels: 1,
        };
        let at = g.pool.alloc(t.bytes(), 128)?;
        let words = t.side() * t.side();
        let mut buf = [0u32; 64];
        let mut m = 0;
        while m < words {
            let n = (words - m).min(64);
            for (j, w) in buf[..n as usize].iter_mut().enumerate() {
                let (x, y) = untwiddle(m + j as u32);
                *w = f(x, y);
            }
            g.mem.store((at + 4 * m) as usize, &buf[..n as usize]);
            m += n;
        }
        g.mem.sync();
        Some(Texture { at, ..t })
    }

    /// A PVRTC 4 bpp texture of side `8 << code`: blocks of a varied
    /// pattern (any 64 bits are a valid block; only the cost is measured).
    fn texpvrtc(g: &mut Rig, code: u8) -> Option<Texture> {
        let t = Texture {
            at: 0,
            code,
            format: format::PVRTC4 as u8,
            u_mode: address::WRAP as u8,
            v_mode: address::WRAP as u8,
            linear: false,
            levels: 1,
        };
        let at = g.pool.alloc(t.bytes(), 128)?;
        let words = t.bytes() / 4;
        let mut buf = [0u32; 64];
        let mut m = 0;
        while m < words {
            let n = (words - m).min(64);
            for (j, w) in buf[..n as usize].iter_mut().enumerate() {
                let k = m + j as u32;
                *w = if k & 1 == 0 {
                    0x5A5A_5A5A ^ k.wrapping_mul(0x9E37_79B9)
                } else {
                    0x7C1F_83E0 ^ (k << 7)
                };
            }
            g.mem.store((at + 4 * m) as usize, &buf[..n as usize]);
            m += n;
        }
        g.mem.sync();
        Some(Texture { at, ..t })
    }

    /// A busy-looking RGB565 picture: stripes and a ramp.
    fn pattern565(x: u32, y: u32) -> u16 {
        let r = (x >> 4) & 31;
        let gr = ((x + y) >> 3) & 63;
        let b = (y >> 4) & 31;
        (r << 11 | gr << 5 | b) as u16
    }

    /// ARGB4444 at alpha 8/15 (half see-through), the same stripes.
    fn pattern4444(x: u32, y: u32) -> u16 {
        (0x8000 | ((x >> 5) & 15) << 8 | ((y >> 5) & 15) << 4 | ((x ^ y) >> 4) & 15) as u16
    }

    fn pattern8888(x: u32, y: u32) -> u32 {
        0xFF00_0000 | (x & 0xFF) << 16 | ((x + y) & 0xFF) << 8 | (y & 0xFF)
    }

    /// The whole target covered by `tex` (side `side` texels) at `scale`
    /// texels a pixel, in sixteenths (16: 1:1).
    fn cover(tex: Texture, scale: i32, blend: bool) -> Draw {
        let side = tex.side() as i32;
        let u1 = (480 * scale / 16) * (UV_ONE / side);
        let v1 = (640 * scale / 16) * (UV_ONE / side);
        let d = Draw::rect(
            0,
            0,
            480,
            640,
            Paint::Texture {
                tex,
                uv: [(0, 0), (u1, 0), (u1, v1), (0, v1)],
                tint: WHITE,
            },
        );
        if blend { d.over() } else { d }
    }

    fn flat(c: u32, blend: bool) -> Draw {
        let d = Draw::rect(0, 0, 480, 640, Paint::Flat(c));
        if blend { d.over() } else { d }
    }

    fn targets() -> [(&'static str, Target); 2] {
        [
            (
                "32-bit",
                Target {
                    picture: SCREEN,
                    x: 0,
                    y: 0,
                },
            ),
            (
                "16-bit",
                Target {
                    picture: PIC16,
                    x: 0,
                    y: 0,
                },
            ),
        ]
    }

    /// One measurement line: median, spread, per tile, pixels a second.
    fn report(what: &str, t: &Times, tiles: u32) {
        let m = t.median();
        if t.n == 0 {
            log!("  {what}: no render finished");
            return;
        }
        log!(
            "  {what}: {m} us (min {}, max {}, n {}), {} ns a tile, {} Mpx/s",
            t.min(),
            t.max(),
            t.n,
            u64::from(m) * 1000 / u64::from(tiles.max(1)),
            tiles * 128 / m.max(1)
        );
    }

    // ---- G4a: rectangular textures -------------------------------------

    /// A texel whose colour names its place: red the row's high bits,
    /// green the column / `step`, blue the row's low bits.
    fn place565(x: u32, y: u32, step: u32) -> u16 {
        ((((y >> 5) & 31) << 11) | (((x / step) & 63) << 5) | (y & 31)) as u16
    }

    /// Which (column / step, row) a 32-bit pixel of [`place565`] shows.
    fn place_of(px: u32) -> (u32, u32) {
        let r = (px >> 19) & 31;
        let gr = (px >> 10) & 63;
        let b = (px >> 3) & 31;
        (gr, r << 5 | b)
    }

    /// Changes the size codes in a slot's TEX0 words (both triangles):
    /// the renderer builds square ones. False if the slot holds no word
    /// equal to `tex0` (then nothing is written).
    fn patch_tex0(g: &mut Rig, r: &Renderer, slot: usize, tex0: u32, new: u32) -> bool {
        let Some(at) = r.slot_words_at(slot) else {
            return false;
        };
        let mut n = 0;
        for k in 0..64 {
            let a = (at + 4 * k) as usize;
            if g.mem.load(a) == Some(tex0) {
                g.mem.store(a, &[new]);
                n += 1;
            }
        }
        g.mem.sync();
        n > 0
    }

    #[inline(never)]
    fn g4a(g: &mut Rig) {
        log!(
            "G4a: rectangular textures (TEX0 size codes apart, the GLES driver's order), drawn 1:1"
        );
        let Some(mut r) = Renderer::new(&mut g.pool, 480, 640) else {
            log!("  no renderer");
            return;
        };
        let _ = r.set_background(Some(GREY));
        let full = Target {
            picture: SCREEN,
            x: 0,
            y: 0,
        };
        // (bits across, bits down, drawn w, h, column step)
        for (bu, bv, w, h, step) in [
            (6u32, 5u32, 64u32, 32u32, 1u32),
            (5, 6, 32, 64, 1),
            (9, 10, 480, 640, 8),
        ] {
            let bytes = 2u32 << (bu + bv);
            let Some(at) = g.pool.alloc(bytes, 128) else {
                log!("  {}x{}: no memory", 1 << bu, 1 << bv);
                continue;
            };
            write16(g, at, bu, bv, 1, &|x, y| place565(x, y, step));
            let long = bu.max(bv);
            let t = Texture {
                at,
                code: (long - 3) as u8,
                format: format::RGB565 as u8,
                u_mode: address::CLAMP as u8,
                v_mode: address::CLAMP as u8,
                linear: false,
                levels: 1,
            };
            let (x0, y0) = if w == 480 { (0, 0) } else { (64, 64) };
            // The uv: (w / 2^bu, h / 2^bv) of the texture, 1:1.
            let (u1, v1) = (((w << 16) >> bu) as i32, ((h << 16) >> bv) as i32);
            g.set(
                &mut r,
                0,
                Some(Draw::rect(
                    x0 as i32,
                    y0 as i32,
                    w as i32,
                    h as i32,
                    Paint::Texture {
                        tex: t,
                        uv: [(0, 0), (u1, 0), (u1, v1), (0, v1)],
                        tint: WHITE,
                    },
                )),
            );
            // TEX0 as the renderer built it (square), its two size codes
            // set apart (the address modes and format kept).
            let square = t.words()[0];
            let sizes = tex::tex0_rect(bu - 3, bv - 3, 0) & 0x0FF0_0000;
            let rect = (square & !0x0FF0_0000) | sizes;
            if !patch_tex0(g, &r, 0, square, rect) {
                log!(
                    "  {}x{}: TEX0 not found in the slot: skipped",
                    1 << bu,
                    1 << bv
                );
                let _ = g.pool.free(at, bytes);
                continue;
            }
            let us = g.frame(&mut r, full);
            // Checked pixels: a grid, 64 at most.
            let (mut checked, mut right) = (0, 0);
            let mut shown = 0;
            for gy in 0..8 {
                for gx in 0..8 {
                    let (px, py) = (gx * w / 8 + w / 16, gy * h / 8 + h / 16);
                    let got = place_of(g.px(x0 + px, y0 + py));
                    let want = (px / step, py);
                    checked += 1;
                    if got == want {
                        right += 1;
                    } else if shown < 6 {
                        shown += 1;
                        log!(
                            "    pixel ({px},{py}) showed texel (col/{step} {}, row {}), its own is ({}, {})",
                            got.0,
                            got.1,
                            want.0,
                            want.1
                        );
                    }
                }
            }
            log!(
                "  {}x{} (TEX0 0x{rect:08x}): {right} of {checked} checked pixels showed their own texel (render {us:?} us)",
                1 << bu,
                1 << bv
            );
            g.set(&mut r, 0, None);
            let _ = g.frame(&mut r, full);
            let _ = g.pool.free(at, bytes);
        }
        let _ = r.release(&mut g.pool);
    }

    // ---- G1: fill rate --------------------------------------------------

    /// The textures G1-G3 share.
    struct Set {
        t565: Texture,
        t4444: Texture,
        t8888: Texture,
        pvrtc: Texture,
        mip: Texture,
        small: Texture,
    }

    fn textures(g: &mut Rig) -> Option<Set> {
        let t0 = rt::ticks();
        let set = Set {
            t565: tex16(g, 6, format::RGB565, 1, &pattern565)?,
            t4444: tex16(g, 6, format::ARGB4444, 1, &pattern4444)?,
            t8888: tex32(g, 6, &pattern8888)?,
            pvrtc: texpvrtc(g, 6)?,
            mip: tex16(g, 6, format::RGB565, 10, &pattern565)?,
            small: tex16(g, 3, format::RGB565, 1, &pattern565)?,
        };
        log!("  textures made in {} ms", us_since(t0) / 1000);
        Some(set)
    }

    fn release(g: &mut Rig, s: Set) {
        for t in [s.t565, s.t4444, s.t8888, s.pvrtc, s.mip, s.small] {
            let _ = rp_mbx3d_render::release(&mut g.pool, t);
        }
    }

    #[inline(never)]
    fn g1(g: &mut Rig, s: &Set) {
        log!(
            "G1: fill rate, one full-screen quad over the background, 5 renders each (2400 tiles)"
        );
        let Some(mut r) = Renderer::new(&mut g.pool, 480, 640) else {
            return;
        };
        let _ = r.set_background(Some(GREY));
        let lin = |t: Texture| t.with_linear(true);
        let cases: [(&str, Draw); 14] = [
            ("background only", flat(GREY, false)),
            ("flat opaque", flat(0xFFC0_4000, false)),
            ("flat translucent (alpha 0x80)", flat(0x80C0_4000, true)),
            ("565 point 1:1", cover(s.t565, 16, false)),
            ("565 bilinear 1:1", cover(lin(s.t565), 16, false)),
            ("4444 translucent point 1:1", cover(s.t4444, 16, true)),
            (
                "4444 translucent bilinear 1:1",
                cover(lin(s.t4444), 16, true),
            ),
            ("8888 point 1:1", cover(s.t8888, 16, false)),
            ("8888 bilinear 1:1", cover(lin(s.t8888), 16, false)),
            ("PVRTC4 point 1:1", cover(s.pvrtc, 16, false)),
            ("PVRTC4 bilinear 1:1", cover(lin(s.pvrtc), 16, false)),
            (
                "565 bilinear magnified 8x (a 64-texel texture)",
                cover(lin(s.small), 2, false),
            ),
            ("565 bilinear shrunk 2:1", cover(lin(s.t565), 32, false)),
            ("565 mipmapped shrunk 2:1", cover(s.mip, 32, false)),
        ];
        for (name, d) in cases {
            if name == "background only" {
                g.set(&mut r, 0, None);
            } else {
                g.set(&mut r, 0, Some(d));
            }
            for (tname, t) in targets() {
                let times = g.full(&mut r, t, 5);
                let mut what = heapless_name();
                let _ = write!(what, "G1 {name} / {tname}");
                report(what.as_str(), &times, TILES);
            }
        }
        g.set(&mut r, 0, None);
        let _ = r.release(&mut g.pool);
    }

    /// A small fixed string buffer for log labels.
    struct Name {
        b: [u8; 96],
        n: usize,
    }

    impl Name {
        fn as_str(&self) -> &str {
            core::str::from_utf8(&self.b[..self.n]).unwrap_or("?")
        }
    }

    impl Write for Name {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            for &c in s.as_bytes() {
                if self.n < self.b.len() {
                    self.b[self.n] = c;
                    self.n += 1;
                }
            }
            Ok(())
        }
    }

    const fn heapless_name() -> Name {
        Name { b: [0; 96], n: 0 }
    }

    // ---- G2: N full-screen layers ----------------------------------------

    #[inline(never)]
    fn g2(g: &mut Rig, s: &Set) {
        log!("G2: N full-screen layers over the background, 3 renders each (median)");
        let Some(mut r) = Renderer::new(&mut g.pool, 480, 640) else {
            return;
        };
        let _ = r.set_background(Some(GREY));
        for kind in 0..4 {
            let name = [
                "opaque 565 point",
                "translucent 4444 point",
                "translucent 4444 bilinear",
                "translucent flat",
            ][kind];
            for (tname, t) in targets() {
                let mut line = heapless_name();
                let mut fits = 0;
                for n in 1..=8usize {
                    for k in 0..8 {
                        let d = (k < n).then(|| match kind {
                            0 => cover(s.t565, 16, false),
                            1 => cover(s.t4444, 16, true),
                            2 => cover(s.t4444.with_linear(true), 16, true),
                            _ => flat(
                                0x40_00_00_00 | ((0x20 * k as u32) & 0xFF) << 16 | 0x8040,
                                true,
                            ),
                        });
                        // Each layer its own texture offset (8 texels apart).
                        let d = d.map(|mut d| {
                            if let Paint::Texture { uv, .. } = &mut d.paint {
                                for p in uv.iter_mut() {
                                    p.0 += k as i32 * 8 * (UV_ONE / 512);
                                }
                            }
                            d
                        });
                        g.set(&mut r, k, d);
                    }
                    let times = g.full(&mut r, t, 3);
                    let m = times.median();
                    if times.n > 0 && m <= FRAME_US {
                        fits = n;
                    }
                    let _ = write!(line, " N{n} {m}");
                }
                log!(
                    "  G2 {name} / {tname} (us):{} ; up to N={fits} in a {FRAME_US} us frame",
                    line.as_str()
                );
            }
        }
        for k in 0..8 {
            g.set(&mut r, k, None);
        }
        let _ = r.release(&mut g.pool);
    }

    /// A pixel rectangle: x, y, width, height.
    type Rect = (i32, i32, i32, i32);

    // ---- G3: partial frames ------------------------------------------------

    #[inline(never)]
    fn g3(g: &mut Rig, s: &Set) {
        log!("G3: partial frames (only the changed tiles rendered), 5 renders each");
        let Some(mut r) = Renderer::new(&mut g.pool, 480, 640) else {
            return;
        };
        let _ = r.set_background(Some(GREY));
        let regions: [(&str, Rect, u32); 4] = [
            ("band 480x160", (0, 240, 480, 160), 600),
            ("row 480x16", (0, 320, 480, 16), 60),
            ("icon 64x64", (208, 288, 64, 64), 32),
            ("one tile", (240, 320, 8, 16), 1),
        ];
        for scene in 0..2 {
            let sname = ["3 translucent 4444 layers", "1 opaque 565 layer"][scene];
            for k in 0..3 {
                let d = if scene == 0 {
                    Some(cover(s.t4444, 16, true))
                } else {
                    (k == 0).then(|| cover(s.t565, 16, false))
                };
                g.set(&mut r, k, d);
            }
            for (tname, t) in targets() {
                // Everything rendered once into this target.
                r.invalidate_all();
                let _ = g.frame(&mut r, t);
                for (rname, (x, y, w, h), tiles) in regions {
                    let mut times = Times::default();
                    for _ in 0..5 {
                        r.invalidate(x, y, w, h);
                        if let Some(us) = g.frame(&mut r, t) {
                            times.add(us);
                        }
                    }
                    let mut what = heapless_name();
                    let _ = write!(what, "G3 {sname}, {rname} ({tiles} tiles) / {tname}");
                    report(what.as_str(), &times, tiles);
                }
            }
        }
        for k in 0..3 {
            g.set(&mut r, k, None);
        }
        let _ = r.release(&mut g.pool);
    }

    // ---- G5: a layer's upload -----------------------------------------------

    /// A 480 x 640 16-bit layer's texel `(x, y)` from its row-order words
    /// (0 outside it: the 512 x 1024 texture's margin).
    fn src16(rows: &[u32], x: u32, y: u32) -> u32 {
        if x >= 480 || y >= 640 {
            return 0;
        }
        let i = (y * 480 + x) as usize;
        (rows[i / 2] >> (16 * (i & 1))) & 0xFFFF
    }

    /// Word `w` of the 512 x 1024 texture in the twiddled order: the texel
    /// pair `(x, y)` and `(x, y + 1)` (y in the even bits).
    fn twiddled_word(rows: &[u32], w: u32) -> u32 {
        let (x, y) = untwiddle_rect(2 * w, 9, 10);
        src16(rows, x, y) | src16(rows, x, y + 1) << 16
    }

    #[inline(never)]
    fn g5(g: &mut Rig) {
        log!("G5: a 480 x 640 layer's upload (bytes / us = MB/s)");
        let Some(ram) = rp_hal_x51v::devices::take_ram_pool() else {
            log!("  no RAM pool");
            return;
        };
        const L16: usize = 480 * 640 / 2;
        const L32: usize = 480 * 640;
        const TW: usize = 512 * 1024 / 2;
        let (rows16, rest) = ram.split_at_mut(L16);
        let (rows32, rest) = rest.split_at_mut(L32);
        let (staging, _) = rest.split_at_mut(TW);
        for (i, w) in rows16.iter_mut().enumerate() {
            let p = 2 * i as u32;
            *w = u32::from(pattern565(p % 480, p / 480))
                | u32::from(pattern565((p + 1) % 480, (p + 1) / 480)) << 16;
        }
        for (i, w) in rows32.iter_mut().enumerate() {
            *w = pattern8888(i as u32 % 480, i as u32 / 480);
        }
        let (Some(lin), Some(twd)) = (
            g.pool.alloc((L32 * 4) as u32, 128),
            g.pool.alloc((TW * 4) as u32, 128),
        ) else {
            log!("  no 2700G memory");
            return;
        };
        let rate = |what: &str, bytes: usize, us: u32| {
            log!(
                "  {what}: {bytes} bytes in {us} us, {} MB/s",
                bytes as u32 / us.max(1)
            );
        };
        rt::check_in();

        // U1, U2: row order, as one copy.
        for (what, src) in [
            ("U1 row order, 16-bit, CPU", &rows16[..]),
            ("U2 row order, 32-bit, CPU", &rows32[..]),
        ] {
            let bytes = src.len() * 4;
            let t0 = rt::ticks();
            g.mem.0[lin as usize..lin as usize + bytes].copy_from_slice(words_as_bytes(src));
            cpu::drain_write_buffer();
            rate(what, bytes, us_since(t0));
            rt::check_in();
        }

        // U3: into the 512 x 1024 texture's order, the CPU reordering on the
        // way (the destination written in order).
        let t0 = rt::ticks();
        let mut buf = [0u32; 64];
        let mut w = 0;
        while w < TW as u32 {
            for (j, b) in buf.iter_mut().enumerate() {
                *b = twiddled_word(rows16, w + j as u32);
            }
            g.mem.store((twd + 4 * w) as usize, &buf);
            w += 64;
        }
        cpu::drain_write_buffer();
        rate(
            "U3 512x1024 texture order, CPU reordering on the way",
            TW * 4,
            us_since(t0),
        );
        rt::check_in();

        // U4: reordered in SDRAM, then DMA.
        let t0 = rt::ticks();
        for (w, s) in staging.iter_mut().enumerate() {
            *s = twiddled_word(rows16, w as u32);
        }
        let reorder = us_since(t0);
        log!("  U4a reordered into SDRAM: {reorder} us");
        rt::check_in();
        u4_dma(g, staging, twd);
        // The two ways must agree.
        let mut agree = true;
        for k in [0usize, 1, 777, TW / 2, TW - 1] {
            agree &= g.mem.load(twd as usize + 4 * k) == Some(staging[k]);
        }
        log!("  U3 and U4 wrote the same texture: {agree}");

        // U5: a 480 x 48 strip (rows 96-143), both ways.
        let t0 = rt::ticks();
        for y in (96..144u32).step_by(2) {
            for x in 0..480u32 {
                let at = twd + 2 * tex::twiddle_rect(x, y, 9, 10);
                let v = src16(rows16, x, y) | src16(rows16, x, y + 1) << 16;
                let _ = store32_volatile(g.mem.0, at as usize, v);
            }
        }
        cpu::drain_write_buffer();
        rate(
            "U5a strip 480x48 into the texture order",
            480 * 48 * 2,
            us_since(t0),
        );
        let t0 = rt::ticks();
        let (a, b) = (96 * 480 / 2, 144 * 480 / 2);
        g.mem.0[lin as usize + 4 * a..lin as usize + 4 * b]
            .copy_from_slice(words_as_bytes(&rows16[a..b]));
        cpu::drain_write_buffer();
        rate("U5b strip 480x48 in row order", 480 * 48 * 2, us_since(t0));
        let _ = g.pool.free(lin, (L32 * 4) as u32);
        let _ = g.pool.free(twd, (TW * 4) as u32);
    }

    /// The CPU's spin rate, what it gets done while a copy runs.
    fn busy(n: &mut u32) {
        *n = core::hint::black_box(*n + 1);
    }

    fn u4_dma(g: &mut Rig, staging: &[u32], dst: u32) {
        let Some(mut dma) = rt::take_dma(rp_hal_pxa27x::dma_map::SD) else {
            log!("  U4b: no DMA channel");
            return;
        };
        let mut idle = 0u32;
        let t0 = rt::ticks();
        while us_since(t0) < 10_000 {
            busy(&mut idle);
        }
        let src = words_as_bytes(staging);
        let mut spins = 0u32;
        let t0 = rt::ticks();
        for (k, piece) in src.chunks(DMA_PIECE).enumerate() {
            let to = CS3 + dst + (k * DMA_PIECE) as u32;
            if !dma.arm_copy_to_device_memory(piece, to) {
                log!("  U4b: refused at piece {k}");
                return;
            }
            let p0 = rt::ticks();
            loop {
                match dma.done() {
                    Some(true) => break,
                    Some(false) => {
                        log!("  U4b: bus error at piece {k}");
                        return;
                    }
                    None if us_since(p0) > 200_000 => {
                        dma.abort();
                        log!("  U4b: piece {k} not done after 200 ms: stopped");
                        return;
                    }
                    None => busy(&mut spins),
                }
            }
        }
        let us = us_since(t0);
        let free =
            u64::from(spins) * 10_000 * 100 / (u64::from(idle).max(1) * u64::from(us.max(1)));
        log!(
            "  U4b SDRAM to the texture by DMA: {} bytes in {us} us, {} MB/s; the CPU kept {free}% of its idle pace",
            src.len(),
            src.len() as u32 / us.max(1)
        );
        let _ = g;
    }

    // ---- W1-W3: in step with the frame event (the owner watches) ---------

    /// `GSCADR` pointing the graphics plane at the 32-bit picture at `at`.
    fn gscadr(at: u32) -> u32 {
        let plane = Plane {
            format: PlaneFormat::Argb8888,
            width: 480,
            height: 640,
            offset: at as usize,
            stride: 480 * 4,
        };
        let v = plane
            .registers()
            .iter()
            .find(|(o, _)| *o == dreg::GSCADR)
            .map_or(0, |&(_, v)| v);
        Layer::SOLO.scadr(v)
    }

    /// The top 80 lines of both screen pictures: the watch test's words.
    fn prompt(g: &mut Rig, l1: &str, l2: &str) {
        for n in 0..2 {
            let at = Picture::screen(n).at;
            g.fill(at, (0, 0, 480, 80), BLACK);
            g.text(at, 8, 10, l1, WHITE, 3);
            g.text(at, 8, 46, l2, YELLOW, 3);
        }
    }

    #[inline(never)]
    fn w1(g: &mut Rig) {
        log!(
            "W1: a flip at line 320 and back at line 560, every frame: a band shows if a flip takes effect at once"
        );
        let (a, b) = (Picture::screen(0).at, Picture::screen(1).at);
        g.fill(a, (0, 80, 480, 640), BLUE);
        g.fill(b, (0, 80, 480, 640), YELLOW);
        prompt(g, "W1: A YELLOW BAND?", "THEN PRESS A BUTTON");
        g.e.regs.write32(dreg::GSCADR, gscadr(a));
        g.arm_button();
        let t0 = rt::ticks();
        let (mut frames, mut l1max, mut l2max) = (0u32, 0, 0);
        while us_since(t0) < WATCH_US && !g.button() {
            rt::check_in();
            if !g.wait_line(320, 20) {
                log!("  no scan line: stopped");
                break;
            }
            g.e.regs.write32(dreg::GSCADR, gscadr(b));
            l1max = l1max.max(g.line());
            if !g.wait_line(560, 20) {
                break;
            }
            g.e.regs.write32(dreg::GSCADR, gscadr(a));
            l2max = l2max.max(g.line());
            frames += 1;
        }
        log!(
            "  W1: {frames} frames, {} s; writes done by line {l1max} and {l2max} at the latest",
            us_since(t0) / 1_000_000
        );
    }

    #[inline(never)]
    fn w2(g: &mut Rig) {
        log!(
            "W2: double buffered: full renders of a moving bar into the hidden screen, flipped at VEVENT0"
        );
        prompt(g, "W2: TORN, JUMPY, SMOOTH?", "THEN PRESS A BUTTON");
        let Some(mut r) = Renderer::new(&mut g.pool, 480, 560) else {
            return;
        };
        let _ = r.set_background(Some(GREY));
        let tgt = |n: usize| Target {
            picture: Picture::screen(n),
            x: 0,
            y: 80,
        };
        let mut shown = 0usize;
        g.e.regs
            .write32(dreg::GSCADR, gscadr(Picture::screen(0).at));
        g.arm_button();
        let t0 = rt::ticks();
        let mut render = Times::default();
        let (mut frames, mut rmax, mut late, mut lmin, mut lmax) = (0u32, 0u32, 0u32, 999u32, 0u32);
        let mut last_flip = rt::ticks();
        let mut pos = 0i32;
        while us_since(t0) < WATCH_US && !g.button() {
            let back = 1 - shown;
            pos = (pos + 8) % 448;
            g.set(
                &mut r,
                0,
                Some(Draw::rect(pos, 0, 32, 560, Paint::Flat(WHITE))),
            );
            g.set(
                &mut r,
                1,
                Some(Draw::rect(0, 400, 480, 32, Paint::Flat(0xFF40_A040))),
            );
            r.invalidate_all();
            let us = g.frame(&mut r, tgt(back)).unwrap_or(0);
            if render.n < 8 {
                render.add(us);
            }
            rmax = rmax.max(us);
            g.vevent_clear();
            if !g.vevent_wait() {
                log!("  no frame event: stopped");
                break;
            }
            g.e.regs
                .write32(dreg::GSCADR, gscadr(Picture::screen(back).at));
            let l = g.line();
            (lmin, lmax) = (lmin.min(l), lmax.max(l));
            let gap = us_since(last_flip);
            last_flip = rt::ticks();
            if frames > 0 && gap > FRAME_US * 3 / 2 {
                late += 1;
            }
            shown = back;
            frames += 1;
        }
        log!(
            "  W2: {frames} flips in {} s, {late} came a frame or more late; render {} us typical, {rmax} us at most; flips written on lines {lmin}-{lmax}",
            us_since(t0) / 1_000_000,
            render.median()
        );
        g.e.regs
            .write32(dreg::GSCADR, gscadr(Picture::screen(0).at));
        g.set(&mut r, 0, None);
        g.set(&mut r, 1, None);
        let _ = r.release(&mut g.pool);
    }

    #[inline(never)]
    fn w3(g: &mut Rig) {
        log!("W3: one screen, four bands each rendered just after the scan line leaves it");
        prompt(g, "W3: TORN, JUMPY, SMOOTH?", "THEN PRESS A BUTTON");
        let Some(mut r) = Renderer::new(&mut g.pool, 480, 560) else {
            return;
        };
        let _ = r.set_background(Some(GREY));
        let t = Target {
            picture: SCREEN,
            x: 0,
            y: 80,
        };
        g.e.regs.write32(dreg::GSCADR, gscadr(SCREEN.at));
        // Bands in tile rows of the 480 x 560 target (35 rows).
        const ROWS: [i32; 5] = [0, 9, 18, 27, 35];
        r.invalidate_all();
        let _ = g.frame(&mut r, t);
        g.arm_button();
        let t0 = rt::ticks();
        let (mut frames, mut pos) = (0u32, 0i32);
        let mut times = [
            Times::default(),
            Times::default(),
            Times::default(),
            Times::default(),
        ];
        let mut done_line = [0u32; 4];
        let mut overlap = 0u32;
        'run: while us_since(t0) < WATCH_US && !g.button() {
            rt::check_in();
            pos = (pos + 8) % 448;
            for k in 0..4 {
                let (top, bottom) = (16 * ROWS[k], 16 * ROWS[k + 1]);
                // The screen line where the band ends: render once the scan
                // has left it.
                let end = (80 + bottom) as u32;
                let ok = if end >= 640 {
                    g.wait_line(640, 30)
                } else {
                    g.wait_line(end, 40)
                };
                if !ok {
                    log!("  no scan line: stopped");
                    break 'run;
                }
                g.set(
                    &mut r,
                    k,
                    Some(Draw::rect(pos, top, 32, bottom - top, Paint::Flat(WHITE))),
                );
                r.invalidate(0, top, 480, bottom - top);
                let us = g.frame(&mut r, t).unwrap_or(0);
                let l = g.line();
                times[k].add(us);
                done_line[k] = done_line[k].max(l);
                // Done before the scan comes back to the band's top?
                let start = (80 + top) as u32;
                let since_end = (l + 645 - end % 645) % 645;
                if since_end >= (start + 645 - end % 645) % 645 {
                    overlap += 1;
                }
            }
            frames += 1;
        }
        log!(
            "  W3: {frames} frames in {} s; band renders typical {} / {} / {} / {} us; done by lines {:?} at the latest; {overlap} band renders still running when the scan came back to them",
            us_since(t0) / 1_000_000,
            times[0].median(),
            times[1].median(),
            times[2].median(),
            times[3].median(),
            done_line
        );
        for k in 0..4 {
            g.set(&mut r, k, None);
        }
        let _ = r.release(&mut g.pool);
    }

    // ---- Run B: texture-word probes ------------------------------------------

    #[inline(never)]
    fn probes(g: &mut Rig) {
        log!(
            "B: TEX0 words Dell's driver never sets, one at a time, on a 512 x 512 RGB565 texture written in ROW order"
        );
        let Some(at) = g.pool.alloc(512 * 512 * 2, 128) else {
            log!("  no memory");
            return;
        };
        // Row order: word (y * 256 + x / 2), texels place565(x, y, 8).
        let mut buf = [0u32; 256];
        for y in 0..512u32 {
            for (j, w) in buf.iter_mut().enumerate() {
                let x = 2 * j as u32;
                *w = u32::from(place565(x, y, 8)) | u32::from(place565(x + 1, y, 8)) << 16;
            }
            g.mem.store((at + y * 1024) as usize, &buf);
        }
        g.mem.sync();
        let Some(mut r) = Renderer::new(&mut g.pool, 480, 640) else {
            return;
        };
        let _ = r.set_background(Some(GREY));
        let full = Target {
            picture: SCREEN,
            x: 0,
            y: 0,
        };
        let t = Texture {
            at,
            code: 6,
            format: format::RGB565 as u8,
            u_mode: address::CLAMP as u8,
            v_mode: address::CLAMP as u8,
            linear: false,
            levels: 1,
        };
        let base = t.words()[0];
        let quarter = UV_ONE / 4;
        let draw = Draw::rect(
            176,
            256,
            128,
            128,
            Paint::Texture {
                tex: t,
                uv: [(0, 0), (quarter, 0), (quarter, quarter), (0, quarter)],
                tint: WHITE,
            },
        );
        let mut tries: [(u8, u32); 20] = [(0, 0); 20];
        let mut n = 0;
        for v in 1..=3u32 {
            tries[n] = (0, (base & !(3 << 18)) | v << 18);
            n += 1;
        }
        for code in [1u32, 3, 5, 7, 9, 11, 13, 15] {
            tries[n] = (1, (base & !(0x1F << 13)) | code << 13);
            n += 1;
        }
        for b in 0..9u32 {
            tries[n] = (2, (base & !0x1FF) | 1 << b);
            n += 1;
        }
        let kinds = ["bits 19-18", "format code", "bits 8-0"];
        let (mut rows_seen, mut hung) = (0, 0);
        for &(kind, word) in &tries[..n] {
            log!("  try {} TEX0 0x{word:08x}", kinds[kind as usize]);
            g.set(&mut r, 0, None);
            g.set(&mut r, 0, Some(draw));
            if !patch_tex0(g, &r, 0, base, word) {
                log!("    TEX0 not found in the slot: skipped");
                continue;
            }
            let hangs = g.hangs;
            let us = g.frame(&mut r, full);
            if g.hangs != hangs {
                hung += 1;
                log!("    HUNG (the core was reset)");
                continue;
            }
            // 16 pixels: row order shows texel (x, y) at pixel (x, y).
            let (mut as_rows, mut as_twiddled) = (0, 0);
            let mut first = (0, 0, 0);
            for k in 0..16u32 {
                let (px, py) = (4 + 8 * k, 4 + 8 * ((k * 5) % 16));
                let pix = g.px(176 + px, 256 + py);
                let got = place_of(pix);
                as_rows += u32::from(got == (px / 8, py));
                // Twiddled reading of row-order data: texel index
                // twiddle(px, py) taken as a row-order place.
                let i = tex::twiddle(px, py);
                let (tx, ty) = (i % 512, i / 512);
                as_twiddled += u32::from(got == (tx / 8, ty));
                if k == 5 {
                    first = (px, py, pix);
                }
            }
            rows_seen += u32::from(as_rows >= 12);
            log!(
                "    {us:?} us: read as rows {as_rows}/16, as twiddled {as_twiddled}/16; pixel ({},{}) 0x{:08x}",
                first.0,
                first.1,
                first.2
            );
        }
        log!("  B: {n} tries, {rows_seen} read the texture as rows, {hung} hung");
        g.set(&mut r, 0, None);
        let _ = r.release(&mut g.pool);
        let _ = g.pool.free(at, 512 * 512 * 2);
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
            "{MARKER}: run {}, MBXCLK 0x{clk0:x}",
            if PROBES {
                "B (texture-word probes)"
            } else {
                "A (fill, layers, partial frames, rectangles, uploads, frame event)"
            }
        );
        let keypad = rt::take_keypad().map(|kp| {
            let mut k = Keypad::new(kp);
            k.enable();
            k.acknowledge();
            k
        });
        let mut g = Rig {
            e,
            mem: Vram(vram),
            clock: DevClock {
                last: rt::ticks(),
                us: 0,
                rest: 0,
            },
            pool: Pool::new(map::POOL),
            resets: 0,
            hangs: 0,
            measured: 0,
            keypad,
            pressed: false,
        };
        g.fill(SCREEN.at, (0, 0, 480, 640), BLACK);
        let saved = show_screen(&mut g.e.regs);
        if PROBES {
            probes(&mut g);
        } else {
            g4a(&mut g);
            if let Some(set) = textures(&mut g) {
                g1(&mut g, &set);
                g2(&mut g, &set);
                g3(&mut g, &set);
                release(&mut g, set);
            } else {
                log!("  textures: no memory: G1-G3 skipped");
            }
            g5(&mut g);
            w1(&mut g);
            w2(&mut g);
            w3(&mut g);
        }
        log!(
            "pool: {} bytes free of {} (everything given back: {})",
            g.pool.free_bytes(),
            map::POOL.size,
            g.pool.free_bytes() == map::POOL.size
        );
        let (measured, hangs) = (g.measured, g.hangs);
        log!("{MARKER}: done, {measured} renders measured, {hangs} hung renders recovered");
        if let Some(bus) = rt::take_ffuart() {
            let mut uart = PxaUart::new(bus);
            uart.init(115_200);
            let _ = writeln!(uart, "{MARKER}: done, {measured} renders, {hangs} hung");
        }
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
            format: PlaneFormat::Argb8888,
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
