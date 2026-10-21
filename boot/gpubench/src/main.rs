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
//!
//! **Run C** (`RP_GPUBENCH_PLANES`, the owner watches each step until a
//! button): the video plane for ADR-0022 point 3's glass-over-a-pan rule.
//! W4 whether a `VSCADR` repoint takes effect at once (a band shows); W5 a
//! list stored as UYVY (the video plane's formats are all YUV) panned by
//! `VSCADR` under per-pixel-alpha glass, its text against the same row in
//! RGB at the bottom; W6 the planes swapped: the list in RGB panned by
//! `GSCADR` under the video plane as a tint at one global alpha.
//!
//! **Run D** (`RP_GPUBENCH_VFMT`, the owner watches): an RGB565 test card on
//! the video plane at each unnamed format code 0-8, colour conversion off,
//! the chroma bases pointed at zeroed memory first (mbxfb: a planar format
//! with them unset hangs the chip); which show the card right.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed with the verdict.
pub const MARKER: &str = "RETROPANE-GPUBENCH";

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_drv_blit2d::{self as cmd, Engine, Format, MBX, reg};
    use rp_drv_mbx_display::clock::{self, MBXCLK, Speed as ClockSpeed};
    use rp_drv_mbx_display::font::{self, WIDTH};
    use rp_drv_mbx_display::{
        Display, PixelFormat as PlaneFormat, Plane, event,
        layers::{Blend, Layer, Slot, Snapshot, VideoFormat, VideoPlane, reg as lreg},
        reg as dreg, scanout,
    };
    use rp_drv_mbx3d::tex;
    use rp_drv_pxa_keypad::{Keypad, Pressed};
    use rp_drv_pxa_uart::PxaUart;
    use rp_hal_api::{DmaEngine, RegisterBus};
    use rp_hal_mem::{load32_volatile, store32_volatile, words_as_bytes};
    use rp_hal_pxa27x::diag;
    use rp_hal_pxa27x::{Mmio, cpu, rt};
    use rp_hal_x51v::devices::KeptRunLog;
    use rp_mbx3d_render::{
        Clock, Draw, LocalMem, Paint, Picture, PixelFormat, Poll, Renderer, Target, Texture,
        UV_ONE, address, format, untwiddle,
    };
    use rp_vram_map::{self as map, Pool};

    use super::MARKER;

    rp_hal_pxa27x::entry!(main);

    macro_rules! log {
        ($($t:tt)*) => {{ let _ = writeln!(KeptRunLog, $($t)*); }};
    }

    /// Run B: the texture-word probes instead of run A.
    const PROBES: bool = option_env!("RP_GPUBENCH_PROBES").is_some();
    /// Run C: the video plane's repoint and pans (the owner watches).
    const PLANES: bool = option_env!("RP_GPUBENCH_PLANES").is_some();
    /// Run D: the video plane's undocumented format codes (the owner
    /// watches).
    const VFMT: bool = option_env!("RP_GPUBENCH_VFMT").is_some();
    /// Run E: faster uploads (the owner's 2x).
    const UPLOAD: bool = option_env!("RP_GPUBENCH_UPLOAD").is_some();
    /// Run F: text as 4 bpp coverage strips.
    const TEXT: bool = option_env!("RP_GPUBENCH_TEXT").is_some();
    /// Run G: the clocks (the cores at divide 1, the system clock past
    /// 100 MHz).
    const CLOCK: bool = option_env!("RP_GPUBENCH_CLOCK").is_some();
    /// Run K: only report a log kept in SRAM by an earlier run that didn't
    /// return (every run reports one; this one does nothing else).
    const KEPT: bool = option_env!("RP_GPUBENCH_KEPT").is_some();
    /// Run R: the display found dark (run G4 stopped scan-out while every
    /// render landed right, and it stayed dark at the loader): every
    /// register read as found, then restart steps one at a time, each
    /// shown as a card for the owner to say which one brought it back.
    const REVIVE: bool = option_env!("RP_GPUBENCH_REVIVE").is_some();
    /// Run S: every named 2700G register below the MBX block read, nothing
    /// written (run R's display stayed dark with a running sync generator:
    /// this snapshot, dark and again after a reset, is diffed).
    const SNAPSHOT: bool = option_env!("RP_GPUBENCH_SNAPSHOT").is_some();
    /// Run U: the dark display reproduced and cleared. Runs R and S found
    /// it: the graphics stream's underrun flag (`DINTRS` bit 4) set and the
    /// output signature (`DSIG`) 0, the sync generator running. U holds
    /// the system clock on the 13 MHz reference for 1, 3, 10 and 30 ms
    /// (what a clock change does), and after each clears the flag Dell's
    /// way, the owner watching cards.
    const UNDERRUN: bool = option_env!("RP_GPUBENCH_UNDERRUN").is_some();
    /// Run V: which work, at which clock, makes the display's fetch run
    /// short (run U ruled out the reference clock, CL3 and the lock traces;
    /// G4 went dark at its first step, the cores at divide 1 at 99.67 MHz).
    /// Built on run G's set-up.
    const LOAD: bool = option_env!("RP_GPUBENCH_LOAD").is_some();
    /// Run X: the dark display's one difference from a working one (run S,
    /// dark against healthy: only `DINTRS` bit 4 set and `DSIG` 0 among
    /// 146 registers) cleared, nothing else touched: the flag cleared
    /// Dell's way, then a card.
    const CLEAR: bool = option_env!("RP_GPUBENCH_CLEAR").is_some();
    /// Run L: the recovery ladder for the dark display (run X: the flag
    /// cleared, then the sync generator, the plane and the whole display
    /// set-up restarted, and still nothing sent). Rungs from the gentlest,
    /// each one Linux mbxfb's resume or set-up also touches, stopping at
    /// the first after which the display sends (`DSIG` nonzero).
    const LADDER: bool = option_env!("RP_GPUBENCH_LADDER").is_some();
    /// Run Z: the 2700G reset (`SYSRST`) and rebuilt from a working one's
    /// registers (run S on the X51v after a soft reset), in Linux mbxfb's
    /// order: clocks, the local memory, the system block, the display. Run L
    /// found no clock or power rung that brings the dark display back.
    const REBUILD: bool = option_env!("RP_GPUBENCH_REBUILD").is_some();
    /// Run N: the start-up watched. G5 found the display dark from its very
    /// start, before any clock changed (`DSIG` 0, no underrun; run Z
    /// brought it back). The display's flags and output signature read
    /// after every start-up stage, then G5's opening up to its first clock
    /// change (its memory set-up, the reference check at Windows Mobile's
    /// clock), with cards the owner reports: one drawn by the CPU, the
    /// others by the 2D engine as G5's are; then G5's first clock changes,
    /// its four lock traces, each followed by a CPU card and a read.
    const START: bool = option_env!("RP_GPUBENCH_START").is_some();
    /// Run Q: what ends the slow state. Since run H2's watchdog reset the
    /// chip runs slow at Windows Mobile's clock (cores / 2: 3D 37.9 ms
    /// against 34.3); run Y's restarts at that clock (`LMPWR` alone,
    /// `LMTYPE` CL3 and back, self-refresh, `SYSCFG` 0) and G5's CL2 steps
    /// left it so, and G4c's first 108.33 MHz CL3 step with the memory
    /// restarted ended it. Three steps of rising reach, Windows Mobile's
    /// clock measured before and after each: 108.33 MHz CL2; the memory
    /// restarted at Windows Mobile's clock; G4c's step.
    const SLOW: bool = option_env!("RP_GPUBENCH_SLOW").is_some();
    /// Run Y: the rescue measured. Run G4 after a Z rebuild (`runG4c`) ran
    /// every 2D fill and 3D render right, but its first pass ran slower
    /// than the same steps before any rebuild (the 3D render at 99.67 MHz,
    /// cores / 1: 25.7 ms against 20.9; reads 6 MB/s against 8) until its
    /// first step that restarted the memory (`LMPWR` and dummy reads after
    /// `LMTYPE` changed to CL3); from then on every step ran at G4's
    /// speeds. Y measures at Windows Mobile's clock before the rebuild,
    /// after it, and after each of three memory restarts, gentlest first,
    /// then with `SYSCFG` put back.
    const RESCUE: bool = option_env!("RP_GPUBENCH_RESCUE").is_some();
    /// Run H: the card log's proof against the reset that lost every
    /// earlier log: a bus the 2700G never answers (runs G2 and Z froze it;
    /// the hardware watchdog reset the device into Dell's prompt, which
    /// clears the SDRAM and SRAM copies). 40 numbered lines, then a read of
    /// the system block's `0x12C` (no register there: it froze Windows
    /// Mobile each time, docs/hw/2700g.md). Every line must be on the card.
    const FREEZE: bool = option_env!("RP_GPUBENCH_FREEZE").is_some();
    /// Run H fires the hardware watchdog (on the device); the emulator's
    /// check builds it with `RP_GPUBENCH_NO_WATCHDOG` too and the run
    /// returns.
    const FIRE_WATCHDOG: bool = option_env!("RP_GPUBENCH_NO_WATCHDOG").is_none();

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
    /// A frame of the panel at 60 Hz, as `Display::adopt` sets it
    /// (`DISPPLL` `0xFC5`: 25.19 MHz over the 648 x 645 frame). Runs A-D on
    /// 2026-10-19 used 17,712 (Windows Mobile's 56.46 Hz) by mistake: G2's
    /// "fits" and W2's "late" thresholds were 6% loose.
    const FRAME_US: u32 = 16_594;
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

    // ---- Run C: the video plane (the owner watches) --------------------------
    //
    // ADR-0022 point 3, the translucent-layer-over-a-pan rule: does a
    // `VSCADR` repoint take effect at once (W4), does a YUV picture pan
    // smoothly under per-pixel-alpha glass and is its text good enough
    // (W5), and does an RGB pan on the graphics plane under a global-alpha
    // video-plane tint work (W6, the planes swapped).

    /// Whether pixel `(x, y)` is lit in `s` drawn at `(x0, y0)` in the
    /// 5 x 7 font, `scale` times.
    fn in_text(x: u32, y: u32, s: &str, x0: u32, y0: u32, scale: u32) -> bool {
        let cell = (WIDTH as u32 + 1) * scale;
        if x < x0 || y < y0 || y >= y0 + 7 * scale {
            return false;
        }
        let k = ((x - x0) / cell) as usize;
        let Some(ch) = s.chars().nth(k) else {
            return false;
        };
        let gx = (x - x0) % cell / scale;
        let gy = ((y - y0) / scale) as usize;
        gx < WIDTH as u32 && font::glyph(ch)[gy] & (1 << (WIDTH as u32 - 1 - gx)) != 0
    }

    /// A list to scroll through: 48-pixel rows, each with its number in
    /// black, a red and a blue word (colour detail: where YUV's halved
    /// chroma shows) and a thin green line; `0x00RRGGBB`.
    fn list_rgb(x: u32, y: u32) -> u32 {
        let row = y / 48;
        let top = row * 48;
        let mut label = heapless_name();
        let _ = write!(label, "ROW {row:02}");
        if in_text(x, y, label.as_str(), 16, top + 14, 3) {
            return 0x00_10_10_10;
        }
        if in_text(x, y, "RED", 200, top + 14, 3) {
            return 0x00_D0_10_10;
        }
        if in_text(x, y, "BLUE", 300, top + 14, 3) {
            return 0x00_10_30_D0;
        }
        if y == top + 47 {
            return 0x00_20_A0_40;
        }
        if row.is_multiple_of(2) {
            0x00_F4_F4_F4
        } else {
            0x00_DC_DC_E4
        }
    }

    /// Writes `w` x `h` UYVY pixels (stride `w * 2`) at `at` from `f`
    /// (`0x00RRGGBB`), each pair's chroma averaged.
    fn write_uyvy(g: &mut Rig, at: u32, w: u32, h: u32, f: &dyn Fn(u32, u32) -> u32) {
        let mut row = [0u32; 240];
        for y in 0..h {
            for (k, word) in row[..(w / 2) as usize].iter_mut().enumerate() {
                let x = 2 * k as u32;
                let yuv = |c: u32| scanout::rgb_to_yuv((c >> 16) as u8, (c >> 8) as u8, c as u8);
                let (y0, u0, v0) = yuv(f(x, y));
                let (y1, u1, v1) = yuv(f(x + 1, y));
                let u = (u32::from(u0) + u32::from(u1)).div_ceil(2);
                let v = (u32::from(v0) + u32::from(v1)).div_ceil(2);
                *word = u | u32::from(y0) << 8 | v << 16 | u32::from(y1) << 24;
            }
            g.mem
                .store((at + y * w * 2) as usize, &row[..(w / 2) as usize]);
            if y.is_multiple_of(64) {
                rt::check_in();
            }
        }
        g.mem.sync();
    }

    /// Writes `w` x `h` ARGB8888 pixels (stride `w * 4`) at `at` from `f`.
    fn write_argb(g: &mut Rig, at: u32, w: u32, h: u32, f: &dyn Fn(u32, u32) -> u32) {
        let mut row = [0u32; 480];
        for y in 0..h {
            for (x, p) in row[..w as usize].iter_mut().enumerate() {
                *p = f(x as u32, y);
            }
            g.mem.store((at + y * w * 4) as usize, &row[..w as usize]);
            if y.is_multiple_of(64) {
                rt::check_in();
            }
        }
        g.mem.sync();
    }

    /// The video plane's registers for a 480 x 640 UYVY picture at `at`
    /// (its lines `stride` bytes apart), as `layer`.
    fn video_regs(at: u32, stride: u32, layer: Layer) -> [(usize, u32); 9] {
        let mut p = VideoPlane::new(VideoFormat::Uyvy, 480, 640, at as usize);
        p.stride = stride as usize;
        p.registers(layer, false)
    }

    fn vscadr_of(regs: &[(usize, u32); 9]) -> u32 {
        regs.iter()
            .find(|(o, _)| *o == lreg::VSCADR)
            .map_or(0, |&(_, v)| v)
    }

    /// The graphics plane on the 32-bit picture at `at` (480 x 640, lines
    /// `stride` bytes apart), as `layer`.
    fn graphics_on(g: &mut Rig, at: u32, stride: u32, layer: Layer) {
        let plane = Plane {
            format: PlaneFormat::Argb8888,
            width: 480,
            height: 640,
            offset: at as usize,
            stride: stride as usize,
        };
        let gamma = g.e.regs.read32(dreg::GSCTRL) & (1 << 26);
        for (off, v) in plane.registers() {
            let v = match off {
                dreg::GSCTRL => v | gamma,
                dreg::GSCADR => layer.scadr(v),
                dreg::GBBASE => layer.bbase(v),
                _ => v,
            };
            g.e.regs.write32(off, v);
        }
    }

    fn video_on(g: &mut Rig, regs: &[(usize, u32); 9]) {
        g.e.regs.modify(lreg::VOVRCLK, rp_hal_api::Field::bit(0), 1);
        for &(off, v) in regs {
            g.e.regs.write32(off, v);
        }
    }

    /// `GSCADR` for the graphics plane at `at` as `layer`.
    fn gscadr_as(at: u32, stride: u32, layer: Layer) -> u32 {
        let plane = Plane {
            format: PlaneFormat::Argb8888,
            width: 480,
            height: 640,
            offset: at as usize,
            stride: stride as usize,
        };
        plane
            .registers()
            .iter()
            .find(|(o, _)| *o == dreg::GSCADR)
            .map_or(0, |&(_, v)| layer.scadr(v))
    }

    /// A glass layer: clear, the prompt's opaque black band on top (its
    /// words in white and yellow), a reference strip at the bottom (the
    /// list's own row in RGB, to compare text against), and a sheet in the
    /// middle: a blue-grey tint at alpha 0x90 with soft edges.
    fn glass(x: u32, y: u32, l1: &str, l2: &str) -> u32 {
        if y < 80 {
            if in_text(x, y, l1, 8, 10, 3) {
                return 0xFF_FF_FF_FF;
            }
            if in_text(x, y, l2, 8, 46, 3) {
                return 0xFF_FF_E0_00;
            }
            return 0xFF_00_00_00;
        }
        if y >= 592 {
            // Row 1 of the list (its stripes and words), in RGB.
            return 0xFF00_0000 | list_rgb(x, y - 592 + 48);
        }
        let (x0, y0, x1, y1) = (60u32, 200u32, 420u32, 440u32);
        if x < x0 || x >= x1 || y < y0 || y >= y1 {
            return 0;
        }
        let edge = (x - x0).min(x1 - 1 - x).min(y - y0).min(y1 - 1 - y);
        let a = if edge < 16 { 0x90 * edge / 16 } else { 0x90 };
        a << 24 | 0x40_50_70
    }

    /// Runs `step` once a frame (at VEVENT0) until a button or the cap.
    /// Returns frames and whether the frame event stopped coming.
    fn each_frame(g: &mut Rig, step: &mut dyn FnMut(&mut Rig, u32)) -> (u32, bool) {
        g.arm_button();
        let t0 = rt::ticks();
        let mut n = 0;
        while us_since(t0) < WATCH_US && !g.button() {
            rt::check_in();
            g.vevent_clear();
            if !g.vevent_wait() {
                return (n, true);
            }
            step(g, n);
            n += 1;
        }
        (n, false)
    }

    #[inline(never)]
    fn planes(g: &mut Rig) {
        log!(
            "C: the video plane: repoint (W4), a YUV pan under glass (W5), an RGB pan under a video-plane tint (W6)"
        );
        const VSTRIDE: u32 = 960;
        const GSTRIDE: u32 = 480 * 4;
        let (Some(vid), Some(rgb)) = (
            g.pool.alloc(VSTRIDE * 1280, 0x1000),
            g.pool.alloc(GSTRIDE * 1280, 0x1000),
        ) else {
            log!("  no 2700G memory");
            return;
        };
        let under = Layer {
            slot: Slot::Bottom,
            blend: Blend::Opaque,
        };
        let glass_layer = Layer {
            slot: Slot::Middle,
            blend: Blend::PixelAlpha,
        };

        // W4: two flat pictures, blue over yellow, in one tall buffer.
        log!(
            "W4: VSCADR to the yellow half at line 320, back at 560, every frame: a band shows if a repoint takes effect at once"
        );
        write_uyvy(g, vid, 480, 1280, &|_, y| {
            if y < 640 {
                0x00_10_20_A0
            } else {
                0x00_FF_E0_00
            }
        });
        write_argb(g, SCREEN.at, 480, 640, &|x, y| {
            glass(x, y, "W4: A YELLOW BAND?", "THEN PRESS A BUTTON")
        });
        // No sheet or strip for W4: only the prompt.
        write_argb(g, SCREEN.at + 80 * GSTRIDE, 480, 560, &|_, _| 0);
        let a = video_regs(vid, VSTRIDE, under);
        let b = vscadr_of(&video_regs(vid + 640 * VSTRIDE, VSTRIDE, under));
        video_on(g, &a);
        graphics_on(g, SCREEN.at, GSTRIDE, glass_layer);
        g.arm_button();
        let t0 = rt::ticks();
        let (mut frames, mut late1, mut late2) = (0u32, 0u32, 0u32);
        while us_since(t0) < WATCH_US && !g.button() {
            rt::check_in();
            if !g.wait_line(320, 20) {
                log!("  no scan line: stopped");
                break;
            }
            g.e.regs.write32(lreg::VSCADR, b);
            late1 = late1.max(g.line());
            if !g.wait_line(560, 20) {
                break;
            }
            g.e.regs.write32(lreg::VSCADR, vscadr_of(&a));
            late2 = late2.max(g.line());
            frames += 1;
        }
        log!(
            "  W4: {frames} frames, {} s; writes done by line {late1} and {late2} at the latest",
            us_since(t0) / 1_000_000
        );

        // W5: the list in UYVY, panned under the glass.
        log!(
            "W5: a YUV list panned 4 lines a frame by VSCADR at VEVENT0, under per-pixel-alpha glass; the bottom strip is the same row in RGB"
        );
        let t0 = rt::ticks();
        write_uyvy(g, vid, 480, 1280, &list_rgb);
        let made = us_since(t0);
        log!(
            "  the list converted RGB -> UYVY by the CPU: 480 x 1280 in {} ms",
            made / 1000
        );
        write_argb(g, SCREEN.at, 480, 640, &|x, y| {
            glass(x, y, "W5: TORN, JUMPY, SMOOTH?", "TEXT VS BOTTOM STRIP?")
        });
        let (frames, stopped) = each_frame(g, &mut |g, n| {
            let pos = (n * 4) % 640;
            let v = vscadr_of(&video_regs(vid + pos * VSTRIDE, VSTRIDE, under));
            g.e.regs.write32(lreg::VSCADR, v);
        });
        log!(
            "  W5: {frames} frames{}",
            if stopped {
                ", the frame event stopped"
            } else {
                ""
            }
        );

        // W6: planes swapped: the list in RGB on the graphics plane, panned
        // by GSCADR; the video plane on top, a flat tint at global alpha.
        log!(
            "W6: an RGB list panned 4 lines a frame by GSCADR at VEVENT0, under the video plane as a tint at global alpha 0x90 (its top band the prompt)"
        );
        let t0 = rt::ticks();
        write_argb(g, rgb, 480, 1280, &|x, y| 0xFF00_0000 | list_rgb(x, y));
        log!("  the RGB list drawn: {} ms", us_since(t0) / 1000);
        write_uyvy(g, vid, 480, 640, &|x, y| {
            if y < 80 {
                if in_text(x, y, "W6: TORN, JUMPY, SMOOTH?", 8, 10, 3)
                    || in_text(x, y, "THEN PRESS A BUTTON", 8, 46, 3)
                {
                    return 0x00_FF_FF_FF;
                }
                return 0;
            }
            0x00_20_30_70
        });
        let tint = Layer {
            slot: Slot::Middle,
            blend: Blend::Global(0x90),
        };
        let content = Layer {
            slot: Slot::Bottom,
            blend: Blend::Opaque,
        };
        g.e.regs.write32(lreg::VSCADR, 0);
        graphics_on(g, rgb, GSTRIDE, content);
        video_on(g, &video_regs(vid, VSTRIDE, tint));
        let (frames, stopped) = each_frame(g, &mut |g, n| {
            let pos = (n * 4) % 640;
            g.e.regs.write32(
                dreg::GSCADR,
                gscadr_as(rgb + pos * GSTRIDE, GSTRIDE, content),
            );
        });
        log!(
            "  W6: {frames} frames{}",
            if stopped {
                ", the frame event stopped"
            } else {
                ""
            }
        );
        let _ = g.pool.free(vid, VSTRIDE * 1280);
        let _ = g.pool.free(rgb, GSTRIDE * 1280);
    }

    // ---- Run D: the video plane's undocumented format codes -------------------

    #[inline(never)]
    fn vformats(g: &mut Rig) {
        log!(
            "D: VSCTRL format codes 0-8 (only 9 and 0xC-0xF are named) on an RGB565 test card, colour conversion off, one at a time"
        );
        const STRIDE: u32 = 960;
        let (Some(card), Some(chroma)) = (
            g.pool.alloc(STRIDE * 640, 0x1000),
            g.pool.alloc(STRIDE * 640, 0x1000),
        ) else {
            log!("  no 2700G memory");
            return;
        };
        // The card: eight colour bars over the top half, a grey ramp, the
        // words "RGB565" in the bottom half; RGB565 pixels.
        const BARS: [u32; 8] = [
            0xFFFFFF, 0xFFFF00, 0x00FFFF, 0x00FF00, 0xFF00FF, 0xFF0000, 0x0000FF, 0x000000,
        ];
        let px = |x: u32, y: u32| -> u32 {
            let c = if y < 320 {
                BARS[(x / 60) as usize]
            } else if y < 480 {
                let v = x * 255 / 479;
                v << 16 | v << 8 | v
            } else if in_text(x, y, "RGB565", 60, 530, 8) {
                0xFFFFFF
            } else {
                0x202020
            };
            ((c >> 19) & 31) << 11 | ((c >> 10) & 63) << 5 | ((c >> 3) & 31)
        };
        let mut row = [0u32; 240];
        for y in 0..640 {
            for (k, w) in row.iter_mut().enumerate() {
                let x = 2 * k as u32;
                *w = px(x, y) | px(x + 1, y) << 16;
            }
            g.mem.store((card + y * STRIDE) as usize, &row);
        }
        // The chroma bases point at zeroed memory (mbxfb: a planar format
        // with them unset hangs the chip).
        let zero = [0u32; 240];
        for y in 0..640 {
            g.mem.store((chroma + y * STRIDE) as usize, &zero);
        }
        g.mem.sync();
        let under = Layer {
            slot: Slot::Bottom,
            blend: Blend::Opaque,
        };
        let glass_layer = Layer {
            slot: Slot::Middle,
            blend: Blend::PixelAlpha,
        };
        let base = video_regs(card, STRIDE, under);
        g.e.regs.write32(lreg::VSCADR, 0);
        g.e.regs.write32(lreg::VUBASE, (chroma / 8) | 1 << 31);
        g.e.regs.write32(lreg::VVBASE, chroma / 8);
        for code in 0..9u32 {
            let mut l1 = heapless_name();
            let _ = write!(l1, "CODE {code}: A TEST CARD?");
            write_argb(g, SCREEN.at, 480, 640, &|x, y| {
                if y < 80 {
                    glass(x, y, l1.as_str(), "THEN PRESS A BUTTON")
                } else {
                    0
                }
            });
            graphics_on(g, SCREEN.at, 480 * 4, glass_layer);
            log!("  try code {code}");
            for &(off, v) in &base {
                let v = match off {
                    // The format, colour conversion off; size kept.
                    lreg::VSCTRL => (v & !(0xF << 27) & !(1 << 25)) | code << 27,
                    lreg::VUBASE => (chroma / 8) | 1 << 31,
                    lreg::VVBASE => chroma / 8,
                    _ => v,
                };
                if off == lreg::VSCADR {
                    g.e.regs.modify(lreg::VOVRCLK, rp_hal_api::Field::bit(0), 1);
                }
                g.e.regs.write32(off, v);
            }
            let found = g.e.regs.read32(lreg::VSCTRL);
            g.arm_button();
            let t0 = rt::ticks();
            while us_since(t0) < 15_000_000 && !g.button() {
                rt::check_in();
                rt::delay_us(20_000);
            }
            log!(
                "    VSCTRL read back 0x{found:08x}; shown {} ms",
                us_since(t0) / 1000
            );
            g.e.regs.write32(lreg::VSCADR, 0);
        }
        let _ = g.pool.free(card, STRIDE * 640);
        let _ = g.pool.free(chroma, STRIDE * 640);
    }

    // ---- Run E: faster uploads (the owner's 2x, 2026-10-19) -------------------
    //
    // The bus to the 2700G is paced by the 2700G (rp-cs3bus: ~33 MB/s
    // writes whatever MSC1 says), so the 2x comes from around it: E1 the
    // texture reorder made fast and overlapped with the DMA (G5 did 37 ms
    // reordering then 34 ms of DMA, one after the other); E2 8-bit layers
    // (half the bytes) and their draw cost; E3 the bytes PVRTC moves; E4 a
    // screen of text as 2D-engine commands from a 4 bpp coverage atlas
    // instead of pixels.

    /// The 64 texels of an aligned 8 x 8 block in twiddled order, as
    /// offsets in a 480-wide row-order source (texels): word k of the block
    /// is texels 2k and 2k + 1 (one above the other: y is the low bit).
    const BLOCK8: [u16; 64] = {
        let mut t = [0u16; 64];
        let mut i = 0;
        while i < 64 {
            let (x, y) = untwiddle(i as u32);
            t[i] = (y * 480 + x) as u16;
            i += 1;
        }
        t
    };

    /// Words of the 512 x 1024 texture (two 16-bit texels each).
    const TW: usize = 512 * 1024 / 2;
    /// One DMA piece: 120 KiB (the controller's chain limit, as G5).
    const PIECE_WORDS: usize = DMA_PIECE / 4;

    /// Block `b` (64 texels, 32 words) of the 512 x 1024 texture from the
    /// 480 x 640 row-order layer `px`, into `out` (32 words): the fast
    /// reorder, a table instead of bit work per texel.
    fn block(px: &[u16], b: usize, out: &mut [u32]) {
        let (x, y) = untwiddle_rect((b as u32) << 6, 9, 10);
        if x >= 480 || y >= 640 {
            out.fill(0);
            return;
        }
        let base = (y * 480 + x) as usize;
        for (k, w) in out.iter_mut().enumerate() {
            let lo = px[base + BLOCK8[2 * k] as usize];
            let hi = px[base + BLOCK8[2 * k + 1] as usize];
            *w = u32::from(lo) | u32::from(hi) << 16;
        }
    }

    /// Waits for the DMA in flight; false on a bus error or 200 ms.
    fn dma_wait(dma: &mut rp_hal_pxa27x::dma::Dma) -> bool {
        let t0 = rt::ticks();
        loop {
            match dma.done() {
                Some(true) => return true,
                Some(false) => return false,
                None if us_since(t0) > 200_000 => {
                    dma.abort();
                    return false;
                }
                None => {}
            }
        }
    }

    #[inline(never)]
    fn uploads(g: &mut Rig) {
        log!(
            "E: faster uploads: the bus is the 2700G's (~33 MB/s), so less waiting and fewer bytes"
        );
        let Some(ram) = rp_hal_x51v::devices::take_ram_pool() else {
            log!("  no RAM pool");
            return;
        };
        let Some(mut dma) = rt::take_dma(rp_hal_pxa27x::dma_map::SD) else {
            log!("  no DMA channel");
            return;
        };
        let (rows, rest) = ram.split_at_mut(480 * 640 / 2);
        let (staging, _) = rest.split_at_mut(TW);
        for (i, w) in rows.iter_mut().enumerate() {
            let p = 2 * i as u32;
            *w = u32::from(pattern565(p % 480, p / 480))
                | u32::from(pattern565((p + 1) % 480, (p + 1) / 480)) << 16;
        }
        let Some(px) = rp_hal_mem::bytes_as_pixels_mut(rp_hal_mem::words_as_bytes_mut(rows)) else {
            log!("  layer not 16-bit aligned");
            return;
        };
        let px: &[u16] = px;
        let Some(twd) = g.pool.alloc((TW * 4) as u32, 128) else {
            log!("  no 2700G memory");
            return;
        };
        let rate = |what: &str, bytes: usize, us: u32| {
            log!(
                "  {what}: {bytes} bytes in {us} us, {} MB/s",
                bytes as u32 / us.max(1)
            );
        };

        // E1a: the fast reorder alone (SDRAM to SDRAM).
        let t0 = rt::ticks();
        for b in 0..TW / 32 {
            block(px, b, &mut staging[32 * b..32 * b + 32]);
        }
        let reorder = us_since(t0);
        log!(
            "  E1a fast reorder of the 512x1024 texture in SDRAM: {reorder} us (G5's plain one: 37,144 us)"
        );
        rt::check_in();
        // It must agree with G5's reference order.
        let agree = [0usize, 1, 777, 31 * 1024 + 5, TW / 2, TW - 1]
            .iter()
            .all(|&w| staging[w] == ref_word(px, w as u32));
        log!("  E1a agrees with the reference order: {agree}");

        // E1b: reorder then DMA, one after the other (G5's way, fast reorder).
        let t0 = rt::ticks();
        for b in 0..TW / 32 {
            block(px, b, &mut staging[32 * b..32 * b + 32]);
        }
        let mut ok = true;
        for (k, piece) in words_as_bytes(staging).chunks(DMA_PIECE).enumerate() {
            ok &= dma.arm_copy_to_device_memory(piece, CS3 + twd + (k * DMA_PIECE) as u32)
                && dma_wait(&mut dma);
        }
        rate("E1b fast reorder, then DMA", TW * 4, us_since(t0));
        rt::check_in();

        // E1c: overlapped: each piece reordered while the last one moves.
        let t0 = rt::ticks();
        let (mut waited, mut busy) = (0u32, false);
        for k in 0..TW.div_ceil(PIECE_WORDS) {
            let (a, b) = (k * PIECE_WORDS, ((k + 1) * PIECE_WORDS).min(TW));
            for blk in a / 32..b / 32 {
                block(px, blk, &mut staging[32 * blk..32 * blk + 32]);
            }
            if busy {
                let w0 = rt::ticks();
                ok &= dma_wait(&mut dma);
                waited += us_since(w0);
            }
            ok &= dma.arm_copy_to_device_memory(
                words_as_bytes(&staging[a..b]),
                CS3 + twd + (a * 4) as u32,
            );
            busy = true;
        }
        ok &= dma_wait(&mut dma);
        let total = us_since(t0);
        rate("E1c fast reorder overlapped with DMA", TW * 4, total);
        log!(
            "  E1c the CPU waited for the DMA {waited} us of {total} (free for other work then); DMA ok {ok}"
        );
        let landed = [0usize, 1, 777, TW / 2, TW - 1]
            .iter()
            .all(|&w| g.mem.load(twd as usize + 4 * w) == Some(staging[w]));
        log!("  E1c the texture in 2700G memory is right: {landed}");
        rt::check_in();

        // E1d: the fast reorder written straight to 2700G memory by the CPU.
        let t0 = rt::ticks();
        let mut buf = [0u32; 32];
        for b in 0..TW / 32 {
            block(px, b, &mut buf);
            g.mem.store(twd as usize + 128 * b, &buf);
        }
        cpu::drain_write_buffer();
        rate(
            "E1d fast reorder written by the CPU straight in",
            TW * 4,
            us_since(t0),
        );
        rt::check_in();

        // E2/E3: fewer bytes. A layer's texture at 8 bits (code 10) and as
        // PVRTC 4 / 2 bpp: the bytes DMA moves for each.
        for (what, bytes) in [
            ("E2 8-bit layer (512x1024, half the bytes)", TW * 2),
            ("E3 PVRTC 4 bpp layer (512x1024, a quarter)", TW),
            ("E3 PVRTC 2 bpp layer (an eighth)", TW / 2),
        ] {
            let t0 = rt::ticks();
            let mut ok = true;
            for (k, piece) in words_as_bytes(&staging[..bytes / 4])
                .chunks(DMA_PIECE)
                .enumerate()
            {
                ok &= dma.arm_copy_to_device_memory(piece, CS3 + twd + (k * DMA_PIECE) as u32)
                    && dma_wait(&mut dma);
            }
            rate(what, bytes, us_since(t0));
            if !ok {
                log!("    DMA refused or failed");
            }
            rt::check_in();
        }
        let _ = g.pool.free(twd, (TW * 4) as u32);
        e2_draw(g);
        e4_text(g);
    }

    /// Word `w` of the texture the plain way (G5's order, bit by bit): the
    /// reference the fast reorder must match.
    fn ref_word(px: &[u16], w: u32) -> u32 {
        let (x, y) = untwiddle_rect(2 * w, 9, 10);
        let at = |x: u32, y: u32| {
            if x < 480 && y < 640 {
                u32::from(px[(y * 480 + x) as usize])
            } else {
                0
            }
        };
        at(x, y) | at(x, y + 1) << 16
    }

    /// The draw cost of an 8-bit layer (code 10 grey, measured drawing by
    /// rp-glesfeat L), full screen 1:1, opaque and with its texels as alpha
    /// (TEX2 0x86084610, glesfeat L), against G1's 565 (27.2 ms).
    #[inline(never)]
    fn e2_draw(g: &mut Rig) {
        let t = Texture {
            at: 0,
            code: 6,
            format: 10,
            u_mode: address::WRAP as u8,
            v_mode: address::WRAP as u8,
            linear: false,
            levels: 1,
        };
        let bytes = 512 * 512;
        let Some(at) = g.pool.alloc(bytes, 128) else {
            log!("  E2 draw: no memory");
            return;
        };
        let t = Texture { at, ..t };
        // Grey texels, twiddled bytes: a stripe pattern.
        let mut buf = [0u32; 64];
        let mut w = 0u32;
        while w < bytes / 4 {
            for (j, word) in buf.iter_mut().enumerate() {
                let i = 4 * (w + j as u32);
                let mut v = 0;
                for k in 0..4 {
                    let (x, y) = untwiddle(i + k);
                    v |= (((x ^ y) >> 3 & 0xFF) | 0x20) << (8 * k);
                }
                *word = v;
            }
            g.mem.store((at + 4 * w) as usize, &buf);
            w += 64;
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
        g.set(&mut r, 0, Some(cover(t, 16, false)));
        let times = g.full(&mut r, full, 5);
        report("E2 8-bit layer drawn, opaque, 1:1 / 32-bit", &times, TILES);
        g.set(&mut r, 0, Some(cover(t, 16, true)));
        let alpha = patch_tex0(g, &r, 0, t.words()[2], 0x8608_4610);
        let times = g.full(&mut r, full, 5);
        report(
            if alpha {
                "E2 8-bit layer drawn, its texels as alpha, 1:1 / 32-bit"
            } else {
                "E2 8-bit layer, alpha word not found: opaque again"
            },
            &times,
            TILES,
        );
        g.set(&mut r, 0, None);
        let _ = r.release(&mut g.pool);
        let _ = g.pool.free(at, bytes);
    }

    /// E4: a screen of text (37 lines of 38 characters, the 5 x 7 font at
    /// 2x in 12 x 16 cells) drawn by the 2D engine from a 4 bpp coverage
    /// atlas already in 2700G memory (Dell's text path: blend 0x09100000,
    /// header bit 17, high nibble first: rp-mbx-blend B1), against the
    /// same screen as RGB565 pixels from the CPU (G5: 18.5 ms, 600 KiB).
    #[inline(never)]
    fn e4_text(g: &mut Rig) {
        const CELL_W: u32 = 12;
        const CELL_H: u32 = 16;
        const GLYPHS: u32 = 95;
        /// Glyphs sit 16 px (8 bytes) apart in the atlas, so every glyph's
        /// source address is a whole word.
        const ATLAS_CELL: u32 = 16;
        const ATLAS_STRIDE: u32 = 768; // 95 x 16 px at 4 bpp, rounded up
        let atlas = map::GLYPHS.at;
        // The atlas: glyph c (0x20 + k) in cell k, lit pixels coverage 15
        // (cx past 12 stays clear: in_text draws 12 px wide at 2x).
        let mut row = [0u32; (ATLAS_STRIDE / 4) as usize];
        let mut lit_per_glyph = [0u32; GLYPHS as usize];
        for y in 0..CELL_H {
            for (i, word) in row.iter_mut().enumerate() {
                let mut v = 0u32;
                for b in 0..4u32 {
                    let mut byte = 0u32;
                    for half in 0..2u32 {
                        let x = 8 * i as u32 + 2 * b + half;
                        let (k, cx) = (x / ATLAS_CELL, x % ATLAS_CELL);
                        let ch = char::from_u32(0x20 + k).unwrap_or(' ');
                        let mut utf8 = [0u8; 4];
                        let on = k < GLYPHS && in_text(cx, y, ch.encode_utf8(&mut utf8), 1, 1, 2);
                        if on {
                            lit_per_glyph[k as usize] += 1;
                        }
                        // High nibble first (B1).
                        byte |= (u32::from(on) * 0xF) << (4 * (1 - half));
                    }
                    v |= byte << (8 * b);
                }
                *word = v;
            }
            g.mem.store((atlas + y * ATLAS_STRIDE) as usize, &row);
        }
        g.mem.sync();

        // The target: an RGB565 picture (PIC16), black.
        let w = cmd::fill(
            Format::RGB565,
            PIC16.at as usize,
            PIC16.stride as usize,
            (0, 0, 480, 640),
            0,
        );
        let _ = g.e.submit(&w).and_then(|()| g.e.finish(g.mem.0, 200_000));

        // The text: line l, column c shows character (l * 7 + c) % 94 + '!'.
        let (lines, cols) = (37u32, 38u32);
        let t0 = rt::ticks();
        let mut words_sent = 0usize;
        let mut lit = 0u32;
        let _ = g.e.submit(&cmd::blend(cmd::BLEND_DELL_TEXT));
        words_sent += 2;
        for l in 0..lines {
            for c in 0..cols {
                let k = (l * 7 + c) % 94 + 1;
                lit += lit_per_glyph[k as usize];
                let (x, y) = ((4 + c * CELL_W) as u16, (8 + l * CELL_H + 1) as u16);
                let (cmdw, n) = cmd::blit(
                    (Format::RGB565, PIC16.at as usize, PIC16.stride as i32),
                    (
                        Format::COVERAGE4,
                        (atlas + k * ATLAS_CELL / 2) as usize,
                        ATLAS_STRIDE as i32,
                    ),
                    (0, 0),
                    (x, y, x + CELL_W as u16, y + CELL_H as u16),
                    cmd::ROP_SRCCOPY,
                    cmd::OP_BLEND,
                    0x00FF_FFFF,
                );
                if g.e.submit(&cmdw[..n]).is_err() {
                    log!("  E4: the 2D engine stopped taking commands");
                    return;
                }
                words_sent += n;
            }
        }
        let _ = g.e.submit(&cmd::blend(0));
        words_sent += 2;
        let queued = us_since(t0);
        let done = g.e.finish(g.mem.0, 500_000).is_ok();
        let total = us_since(t0);
        // Check: white pixels in the picture = the lit coverage drawn.
        let mut white = 0u32;
        for i in 0..480 * 640 / 2 {
            let v = g.mem.load(PIC16.at as usize + 4 * i).unwrap_or(0);
            white += u32::from(v & 0xFFFF == 0xFFFF) + u32::from(v >> 16 == 0xFFFF);
        }
        log!(
            "  E4 a screen of text ({} glyphs) by 2D commands from a 4 bpp atlas: {} bytes over the bus (RGB565 pixels: 614400), queued in {queued} us, drawn in {total} us (finished {done}); white pixels {white}, lit in the glyphs drawn {lit}",
            lines * cols,
            words_sent * 4
        );
    }
    // ---- Run F: text as 4 bpp coverage strips (2026-10-20) -------------------
    //
    // E4 drew a screen of text as one 2D command a glyph: 11x fewer bytes
    // than pixels, but no faster (~14 us of engine time a command). Here the
    // CPU draws the text as 4 bpp coverage (a quarter of RGB565's bytes) and
    // the 2D engine colours and blends it onto the RGB565 picture: one
    // command a line (37), or one for the whole text area. F0 is the same
    // screen drawn by the CPU as RGB565 pixels and copied, the baseline.

    const F_LINES: u32 = 37;
    const F_COLS: u32 = 38;
    /// A line: 480 px x 16 rows at 4 bpp.
    const F_ROW_BYTES: usize = 240;
    const F_LINE_BYTES: usize = F_ROW_BYTES * 16;
    const F_TEXT_BYTES: usize = F_LINE_BYTES * F_LINES as usize;
    /// A glyph's cell in the table: 12 x 16 px at 4 bpp, 6 bytes a row.
    const F_GLYPH_BYTES: usize = 6 * 16;

    /// The character at line `l`, column `c` (E4's text): glyph index k
    /// (character 0x20 + k).
    const fn f_glyph(l: u32, c: u32) -> u32 {
        (l * 7 + c) % 94 + 1
    }

    #[inline(never)]
    fn text_strips(g: &mut Rig) {
        log!(
            "F: a screen of text (37 x 38 glyphs) as 4 bpp coverage strips the 2D engine colours, against RGB565 pixels"
        );
        let Some(ram) = rp_hal_x51v::devices::take_ram_pool() else {
            log!("  no RAM pool");
            return;
        };
        let bytes = rp_hal_mem::words_as_bytes_mut(ram);
        let (table, rest) = bytes.split_at_mut(95 * F_GLYPH_BYTES);
        let (strips, rest) = rest.split_at_mut(F_TEXT_BYTES);
        let (px565, _) = rest.split_at_mut(480 * 640 * 2);

        // The glyph table: each glyph's 12 x 16 cell (the 5 x 7 font at 2x,
        // as E4), 4 bpp, high nibble first; its lit pixels counted.
        let mut lit_of = [0u32; 95];
        for (k, cell) in table
            .as_chunks_mut::<F_GLYPH_BYTES>()
            .0
            .iter_mut()
            .enumerate()
        {
            let ch = char::from_u32(0x20 + k as u32).unwrap_or(' ');
            let mut utf8 = [0u8; 4];
            let s = ch.encode_utf8(&mut utf8);
            for y in 0..16u32 {
                for b in 0..6u32 {
                    let mut byte = 0u8;
                    for half in 0..2u32 {
                        if in_text(2 * b + half, y, s, 1, 1, 2) {
                            byte |= 0xF << (4 * (1 - half));
                            lit_of[k] += 1;
                        }
                    }
                    cell[(y * 6 + b) as usize] = byte;
                }
            }
        }
        let mut lit = 0u32;
        for l in 0..F_LINES {
            for c in 0..F_COLS {
                lit += lit_of[f_glyph(l, c) as usize];
            }
        }
        rt::check_in();

        let black = |g: &mut Rig| {
            let w = cmd::fill(
                Format::RGB565,
                PIC16.at as usize,
                PIC16.stride as usize,
                (0, 0, 480, 640),
                0,
            );
            let _ = g.e.submit(&w).and_then(|()| g.e.finish(g.mem.0, 200_000));
        };
        let white = |g: &Rig| -> u32 {
            let mut n = 0;
            for i in 0..480 * 640 / 2 {
                let v = g.mem.load(PIC16.at as usize + 4 * i).unwrap_or(0);
                n += u32::from(v & 0xFFFF == 0xFFFF) + u32::from(v >> 16 == 0xFFFF);
            }
            n
        };

        // F0: the CPU draws RGB565 pixels in SDRAM (black, white text), then
        // copies the whole picture (600 KiB).
        let t0 = rt::ticks();
        px565.fill(0);
        for l in 0..F_LINES {
            for c in 0..F_COLS {
                let cell = &table[f_glyph(l, c) as usize * F_GLYPH_BYTES..][..F_GLYPH_BYTES];
                let (x0, y0) = (4 + c * 12, 8 + l * 16);
                for y in 0..16u32 {
                    for x in 0..12u32 {
                        let nib = cell[(y * 6 + x / 2) as usize] >> (4 * (1 - (x & 1)));
                        if nib & 0xF != 0 {
                            let at = (((y0 + y) * 480 + x0 + x) * 2) as usize;
                            px565[at] = 0xFF;
                            px565[at + 1] = 0xFF;
                        }
                    }
                }
            }
        }
        let drawn = us_since(t0);
        g.mem.0[PIC16.at as usize..PIC16.at as usize + px565.len()].copy_from_slice(px565);
        cpu::drain_write_buffer();
        let total = us_since(t0);
        let w0 = white(g);
        log!(
            "  F0 RGB565 pixels: drawn {drawn} us, copied {} us, {total} us in all; 614400 bytes over the bus; white {w0}, lit {lit}",
            total - drawn
        );
        rt::check_in();

        // F1-F2: the CPU draws 4 bpp strips (whole bytes: every glyph starts
        // on an even pixel), then copies them (142 KiB).
        let t0 = rt::ticks();
        strips.fill(0);
        for l in 0..F_LINES {
            for c in 0..F_COLS {
                let cell = &table[f_glyph(l, c) as usize * F_GLYPH_BYTES..][..F_GLYPH_BYTES];
                let x0 = (4 + c * 12) as usize / 2;
                for y in 0..16usize {
                    let row = l as usize * F_LINE_BYTES + y * F_ROW_BYTES + x0;
                    strips[row..row + 6].copy_from_slice(&cell[y * 6..y * 6 + 6]);
                }
            }
        }
        let raster = us_since(t0);
        let at = map::GLYPHS.at as usize;
        g.mem.0[at..at + F_TEXT_BYTES].copy_from_slice(strips);
        cpu::drain_write_buffer();
        let upload = us_since(t0) - raster;
        log!(
            "  F1 4 bpp strips drawn by the CPU: {raster} us; F2 copied: {upload} us ({F_TEXT_BYTES} bytes over the bus)"
        );
        rt::check_in();

        // F3a: one 2D command a line; F3b: one for the whole text area.
        for (name, per_line) in [
            ("F3a one 2D command a line (37)", true),
            ("F3b one 2D command for all of it", false),
        ] {
            black(g);
            let t0 = rt::ticks();
            let _ = g.e.submit(&cmd::blend(cmd::BLEND_DELL_TEXT));
            let (n, rows) = if per_line {
                (F_LINES, 16u32)
            } else {
                (1, 16 * F_LINES)
            };
            let mut ok = true;
            for k in 0..n {
                let y0 = (8 + k * rows) as u16;
                let (w, len) = cmd::blit(
                    (Format::RGB565, PIC16.at as usize, PIC16.stride as i32),
                    (
                        Format::COVERAGE4,
                        at + (k * rows) as usize * F_ROW_BYTES,
                        F_ROW_BYTES as i32,
                    ),
                    (0, 0),
                    (0, y0, 480, y0 + rows as u16),
                    cmd::ROP_SRCCOPY,
                    cmd::OP_BLEND,
                    0x00FF_FFFF,
                );
                ok &= g.e.submit(&w[..len]).is_ok();
            }
            let _ = g.e.submit(&cmd::blend(0));
            let done = g.e.finish(g.mem.0, 500_000).is_ok();
            let us = us_since(t0);
            let w = white(g);
            log!(
                "  {name}: {us} us (finished {}), so strips in all {} us; white {w}, lit {lit}",
                ok && done,
                raster + upload + us
            );
            rt::check_in();
        }

        // The words of 2700G memory's picture that differ from F0's (the
        // reference, still in SDRAM).
        let differ = |g: &Rig, reference: &[u8]| -> u32 {
            let mut n = 0;
            for (i, w) in reference.as_chunks::<4>().0.iter().enumerate() {
                if g.mem.load(PIC16.at as usize + 4 * i) != Some(u32::from_le_bytes(*w)) {
                    n += 1;
                }
            }
            n
        };

        // F4: a pre-coloured RGB565 atlas (white on black, the edges baked
        // in: a solid background's text) kept in 2700G memory, every glyph a
        // plain 2D copy (no blend), after a 2D fill of the background.
        const ATLAS565_STRIDE: usize = 2304; // 95 x 12 px x 2 bytes, rounded up
        let a565 = map::GLYPHS.at as usize + F_TEXT_BYTES.next_multiple_of(4096);
        let t0 = rt::ticks();
        let mut row = [0u32; ATLAS565_STRIDE / 4];
        for y in 0..16usize {
            for (i, word) in row.iter_mut().enumerate() {
                let mut v = 0u32;
                for half in 0..2usize {
                    let x = 2 * i + half;
                    let (k, cx) = (x / 12, x % 12);
                    if k < 95 {
                        let nib = table[k * F_GLYPH_BYTES + y * 6 + cx / 2] >> (4 * (1 - (cx & 1)));
                        if nib & 0xF != 0 {
                            v |= 0xFFFF << (16 * half);
                        }
                    }
                }
                *word = v;
            }
            g.mem.store(a565 + y * ATLAS565_STRIDE, &row);
        }
        g.mem.sync();
        log!(
            "  F4 the coloured atlas made once: {} us ({} bytes)",
            us_since(t0),
            16 * ATLAS565_STRIDE
        );
        let t0 = rt::ticks();
        let w = cmd::fill(
            Format::RGB565,
            PIC16.at as usize,
            PIC16.stride as usize,
            (0, 0, 480, 640),
            0,
        );
        let mut ok = g.e.submit(&w).is_ok();
        let mut sent = w.len();
        for l in 0..F_LINES {
            for c in 0..F_COLS {
                let k = f_glyph(l, c) as u16;
                let (x, y) = ((4 + c * 12) as u16, (8 + l * 16) as u16);
                let (cw, n) = cmd::copy(
                    Format::RGB565,
                    (PIC16.at as usize, PIC16.stride as usize),
                    (a565, ATLAS565_STRIDE),
                    (k * 12, 0),
                    (x, y, x + 12, y + 16),
                    0,
                );
                ok &= g.e.submit(&cw[..n]).is_ok();
                sent += n;
            }
        }
        let queued = us_since(t0);
        let done = g.e.finish(g.mem.0, 500_000).is_ok();
        let us = us_since(t0);
        log!(
            "  F4 fill + a plain 2D copy a glyph from the coloured atlas: {us} us in all (the CPU queueing {queued}; finished {}); {} bytes over the bus; words differing from F0: {}",
            ok && done,
            sent * 4,
            differ(g, px565)
        );
        rt::check_in();

        // F5: the engine fills the background (and is waited for), then the
        // CPU writes only the words that hold ink.
        let t0 = rt::ticks();
        let w = cmd::fill(
            Format::RGB565,
            PIC16.at as usize,
            PIC16.stride as usize,
            (0, 0, 480, 640),
            0,
        );
        let _ = g.e.submit(&w).and_then(|()| g.e.finish(g.mem.0, 200_000));
        let filled = us_since(t0);
        let mut words = 0u32;
        for (i, w) in px565.as_chunks::<4>().0.iter().enumerate() {
            let v = u32::from_le_bytes(*w);
            if v != 0 {
                let _ = store32_volatile(g.mem.0, PIC16.at as usize + 4 * i, v);
                words += 1;
            }
        }
        cpu::drain_write_buffer();
        let us = us_since(t0);
        log!(
            "  F5 fill ({filled} us) + the CPU writing only the ink: {us} us in all; {words} words of ink ({} bytes over the bus); words differing from F0: {}",
            4 * words,
            differ(g, px565)
        );
        rt::check_in();

        // F6: F4 with leaner commands. a: F4's full commands, sent in
        // batches; b: the surfaces once, then origin, header, colour and
        // rectangle a glyph; c: b without the colour word (OP_NO_COLOUR).
        // Whether the engine keeps the surfaces between operations is what
        // b and c find out: each picture is compared with F0's, word for
        // word, and a wedged engine is reset.
        let [d0, d1] = cmd::dest(Format::RGB565, PIC16.at as usize, PIC16.stride as i32);
        let [s0, s1] = cmd::source(Format::RGB565, a565, ATLAS565_STRIDE as i32);
        for (name, lean, colour) in [
            ("F6a full commands, batched", false, true),
            ("F6b surfaces once, 5 words a glyph", true, true),
            (
                "F6c surfaces once, 4 words a glyph (no colour word)",
                true,
                false,
            ),
        ] {
            let t0 = rt::ticks();
            let w = cmd::fill(
                Format::RGB565,
                PIC16.at as usize,
                PIC16.stride as usize,
                (0, 0, 480, 640),
                0,
            );
            let mut ok = g.e.submit(&w).is_ok();
            let mut sent = w.len();
            let mut batch = [0u32; 320];
            let mut n = 0usize;
            if lean {
                batch[..4].copy_from_slice(&[d0, d1, s0, s1]);
                n = 4;
            }
            for l in 0..F_LINES {
                for c in 0..F_COLS {
                    let k = f_glyph(l, c) as u16;
                    let (x, y) = ((4 + c * 12) as u16, (8 + l * 16) as u16);
                    if lean {
                        let flags = if colour { 0 } else { cmd::OP_NO_COLOUR };
                        batch[n] = cmd::origin(k * 12, 0);
                        batch[n + 1] = 0x8000_0000 | cmd::ROP_SRCCOPY | flags;
                        n += 2;
                        if colour {
                            batch[n] = 0;
                            n += 1;
                        }
                        batch[n] = cmd::xy(x, y);
                        batch[n + 1] = cmd::xy(x + 12, y + 16);
                        n += 2;
                    } else {
                        let (cw, len) = cmd::copy(
                            Format::RGB565,
                            (PIC16.at as usize, PIC16.stride as usize),
                            (a565, ATLAS565_STRIDE),
                            (k * 12, 0),
                            (x, y, x + 12, y + 16),
                            0,
                        );
                        batch[n..n + len].copy_from_slice(&cw[..len]);
                        n += len;
                    }
                    if n > batch.len() - 12 {
                        ok &= g.e.submit(&batch[..n]).is_ok();
                        sent += n;
                        n = 0;
                    }
                }
            }
            batch[n] = cmd::FLUSH;
            n += 1;
            ok &= g.e.submit(&batch[..n]).is_ok();
            sent += n;
            let queued = us_since(t0);
            let done = g.e.finish(g.mem.0, 500_000).is_ok();
            let us = us_since(t0);
            let bad = differ(g, px565);
            log!(
                "  {name}: {us} us in all (the CPU queueing {queued}; finished {}); {} bytes over the bus; words differing from F0: {bad}",
                ok && done,
                sent * 4
            );
            if !(ok && done) {
                g.e.reset(rt::delay_us);
                log!("    the 2D engine was reset");
            }
            rt::check_in();
        }
    }

    // ---- Run G: the clocks (docs/research/2700g-full-potential.md P1, P2) ----
    //
    // G2 (2026-10-20), after G1 (`local/rp-gpubench-runG1.txt`): the same
    // step passed and later failed, and the 2700G stayed broken after its
    // registers were put back, so no failure could be pinned on a clock.
    // G2 traces the PLL's lock bit, reads every clock register back, runs
    // each step twice, every step rising and then in reverse, checks
    // Windows Mobile's clock after every step (a carried-over failure shows
    // there, and stops the run if a second restore doesn't clear it), tries
    // Dell's CL3 rule against CL2 past 100 MHz, and shows each step on the
    // screen in large letters for the owner watching. Windows Mobile's clock
    // and bus timing go back at the end and from the fault handler.

    /// CS3 timing (RDF, RDN, RRR): Windows Mobile's (Dell's tier for 99 MHz
    /// and up) and Dell's tier for 25-99 MHz.
    const CS3_WM: (u32, u32, u32) = (8, 4, 2);
    const CS3_LONG: (u32, u32, u32) = (13, 9, 7);

    /// A clock step: the core PLL (13 MHz x M / (N x 2^P)), the cores'
    /// divider (1 or 2) and the local memory's CAS latency (`None`: Dell's
    /// rule, CL3 above 100 MHz; run G1's 108 MHz step with CL3 drew no 3D
    /// picture where `rp-mbx3d` run 27 drew right at 133 MHz with CL2, so
    /// G2 tries both).
    #[derive(Clone, Copy)]
    struct ClockStep {
        m: u32,
        n: u32,
        p: u32,
        div: u32,
        cl3: Option<bool>,
        /// After the PLL and timing: `LMPWR` written active again and a
        /// few reads, as Linux mbxfb's set-up does after `LMTYPE` ("enable
        /// memory controller", then dummy reads). Whether the controller
        /// re-sends the SDRAM's mode (its CAS latency) only then is the
        /// question: Dell's routine writes no `LMPWR`, and on the X51v it
        /// never chooses CL3 (its clock is clamped to 100 MHz).
        restart: bool,
    }

    impl ClockStep {
        const fn pll(self) -> u32 {
            self.m << 7 | self.n << 4 | self.p << 1 | 1
        }

        const fn khz(self) -> u32 {
            ClockSpeed::rated(self.pll()).khz()
        }

        /// The PLL and the memory timing for it.
        const fn speed(self) -> ClockSpeed {
            let mut s = ClockSpeed::rated(self.pll());
            match self.cl3 {
                Some(true) => s.lmtype = 0xEC8,
                Some(false) => s.lmtype = 0xAC8,
                None => {}
            }
            s
        }

        /// The CAS latency the step runs with.
        const fn cl(self) -> u32 {
            self.speed().lmtype >> 10 & 7
        }
    }

    const WM_STEP: ClockStep = ClockStep {
        m: 23,
        n: 3,
        p: 0,
        div: 2,
        cl3: None,
        restart: false,
    };

    /// The local memory's power control (Linux mbxfb: 0 active, 1
    /// self-refresh, 3 deep power-down).
    const LMPWR: usize = 0x1008;

    /// The clock and memory registers as they read now.
    fn clock_now<B: RegisterBus>(regs: &mut B) -> ClockSpeed {
        ClockSpeed {
            pll: regs.read32(clock::COREPLL),
            lmtype: regs.read32(clock::LMTYPE),
            lmtim: regs.read32(clock::LMTIM),
            lmrefresh: regs.read32(clock::LMREFRESH),
        }
    }

    /// Windows Mobile's clock and bus timing back, wherever run G left
    /// them (no state kept, so the fault handler can call it). The PLL is
    /// always written again, even when it reads as Windows Mobile's: run
    /// G1 put the registers back and the 2700G still didn't work.
    fn clock_restore<B: RegisterBus>(regs: &mut B) {
        let _ = diag::set_cs3_vlio(CS3_LONG.0, CS3_LONG.1, CS3_LONG.2);
        let clk = regs.read32(MBXCLK);
        regs.write32(MBXCLK, (clk & !0xC) | 1 << 2);
        rt::delay_us(1_000);
        if let Some(a) = clock::set_speed(regs, ClockSpeed::WM, rt::delay_us)
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

    /// Puts `s` in place: the long bus timing first (safe at any clock),
    /// the cores at divide 2, the PLL and its memory timing (Dell's way;
    /// skipped when they already read as `s`'s, so a divider-only step
    /// changes nothing else), then the step's divider and the bus timing of
    /// its tier. The PLL's lock time, or `None` (Windows Mobile's speed is
    /// then back).
    fn clock_apply(g: &mut Rig, s: ClockStep) -> Option<u32> {
        let _ = diag::set_cs3_vlio(CS3_LONG.0, CS3_LONG.1, CS3_LONG.2);
        let clk = g.e.regs.read32(MBXCLK);
        g.e.regs.write32(MBXCLK, (clk & !0xC) | 1 << 2);
        rt::delay_us(1_000);
        let lock = if clock_now(&mut g.e.regs) == s.speed() {
            Some(0)
        } else {
            match clock::set_speed(&mut g.e.regs, s.speed(), rt::delay_us) {
                Some(a) => {
                    if a.rewrites + a.unwritten > 0 {
                        log!(
                            "    memory-controller writes: {} written again, {} never held",
                            a.rewrites,
                            a.unwritten
                        );
                    }
                    (a.unwritten == 0).then_some(a.lock_us)
                }
                None => None,
            }
        };
        if s.restart {
            g.e.regs.write32(LMPWR, 0);
            rt::delay_us(100);
            for k in 0..8 {
                let _ = g.mem.load(4 * k);
            }
        }
        let clk = g.e.regs.read32(MBXCLK);
        g.e.regs.write32(MBXCLK, (clk & !0xC) | (s.div - 1) << 2);
        rt::delay_us(1_000);
        let cs3 = if s.khz() < 99_000 { CS3_LONG } else { CS3_WM };
        let _ = diag::set_cs3_vlio(cs3.0, cs3.1, cs3.2);
        lock
    }

    /// Every clock register read back and logged (what the 2700G holds,
    /// not what was written).
    fn clock_readback(g: &mut Rig) {
        let r = &mut g.e.regs;
        log!(
            "    read back: SYSCLKSRC {} COREPLL 0x{:x} PLLSTAT 0x{:x} MBXCLK 0x{:x} LMTYPE 0x{:x} LMTIM 0x{:x} LMREFRESH 0x{:x}",
            r.read32(clock::SYSCLKSRC),
            r.read32(clock::COREPLL),
            r.read32(clock::PLLSTAT),
            r.read32(MBXCLK),
            r.read32(clock::LMTYPE),
            r.read32(clock::LMTIM),
            r.read32(clock::LMREFRESH)
        );
    }

    /// The PLL's lock bit traced across a change to `to` (the system clock
    /// on the reference meanwhile, as `set_speed` does): `PLLSTAT` before,
    /// at once after the write, and every change in the next 2 ms with its
    /// time. Settles whether the lock bit follows a relock (V9 of the
    /// verify list: run 27 and G1 read it set at the first poll).
    fn lock_trace(g: &mut Rig, to: ClockSpeed, name: &str) {
        let r = &mut g.e.regs;
        r.write32(clock::SYSCLKSRC, 0);
        rt::delay_us(250);
        // Each memory-controller write waited and read back (run U2: back
        // to back, the traces' last writes were lost).
        for (reg, v) in [
            (clock::LMTIM, to.lmtim),
            (clock::LMREFRESH, to.lmrefresh),
            (clock::LMTYPE, to.lmtype),
        ] {
            for _ in 0..3 {
                r.write32(reg, v);
                rt::delay_us(1_000);
                if r.read32(reg) == v {
                    break;
                }
            }
        }
        let before = r.read32(clock::PLLSTAT);
        r.write32(clock::COREPLL, to.pll);
        let first = r.read32(clock::PLLSTAT);
        let t0 = rt::ticks();
        let mut last = first;
        let mut changes = heapless_name();
        let mut samples = 0u32;
        while us_since(t0) < 2_000 {
            let v = r.read32(clock::PLLSTAT);
            samples += 1;
            if v != last {
                let _ = write!(changes, " {}us:0x{v:x}", us_since(t0));
                last = v;
            }
        }
        rt::delay_us(2_000);
        r.write32(clock::SYSCLKSRC, 2);
        rt::delay_us(250);
        log!(
            "  lock trace {name}: PLLSTAT 0x{before:x} before, 0x{first:x} at once after the write, {samples} reads in 2 ms, changes:{}",
            if changes.n == 0 {
                " none"
            } else {
                changes.as_str()
            }
        );
    }

    /// A full-screen card in large letters for the owner watching, held
    /// `secs` seconds.
    fn clock_card(g: &mut Rig, lines: [&str; 3], bg: u32, secs: u32) {
        g.fill(SCREEN.at, (0, 0, 480, 640), bg);
        for (k, l) in lines.iter().enumerate() {
            g.text(SCREEN.at, 16, 200 + 80 * k as u32, l, 0xFFFF_FFFF, 4);
        }
        cpu::drain_write_buffer();
        for _ in 0..secs * 10 {
            rt::delay_us(100_000);
            rt::check_in();
        }
    }

    /// Where run G's memory sits: the texture, the flush target, the
    /// pattern region (2700G memory); the flush source and the reference
    /// picture (SDRAM).
    struct ClockRig<'a> {
        tex: Texture,
        /// The texture's words as uploaded (read back at Windows Mobile's
        /// clock): run G3's controls failed on a texture damaged by an
        /// earlier step, uploaded once; now each check counts the damage
        /// and repairs it before rendering.
        tex_copy: &'a [u32],
        flush_at: u32,
        pat_at: u32,
        pat_bytes: u32,
        src: &'a [u32],
        reference: &'a mut [u32],
    }

    /// One step's results.
    #[derive(Default)]
    struct ClockResult {
        flush_us: u32,
        read_us: u32,
        bus_wrong: u32,
        mem_wrong: u32,
        fill_us: u32,
        fill_wrong: u32,
        /// The 2D fill didn't finish in 200 ms (G1 counted this as
        /// `u32::MAX` wrong words, which wrapped).
        fill_hung: bool,
        /// Texture words found changed since the last check (repaired).
        tex_damaged: u32,
        /// The display's fetch had run short (`DINTRS` bit 4, cleared when
        /// read) before this check began (since the last read: the clock
        /// change into it), and during it (G4's dark screen, never
        /// reproduced alone by runs U and V).
        short_before: bool,
        short_during: bool,
        /// Parts stopped at their time limit ([`PART_US`]): bit 0 the
        /// flush, 1 the read-back, 2 the memory pattern.
        slow: u32,
        render_us: u32,
        render_wrong: u32,
        first_diff: Option<(u32, u32, u32)>,
    }

    impl ClockResult {
        fn good(&self) -> bool {
            self.bus_wrong + self.mem_wrong + self.fill_wrong + self.render_wrong == 0
                && !self.fill_hung
                && self.slow == 0
                && self.tex_damaged == 0
                && self.render_us != 0
        }
    }

    /// The pattern word at byte offset `a` for `seed` (odd seeds inverted).
    fn clock_pat(a: u32, seed: u32) -> u32 {
        let w = a.wrapping_mul(0x9E37_79B9) ^ seed.wrapping_mul(0x85EB_CA6B);
        if seed & 1 == 1 { !w } else { w }
    }

    /// Each part of a check stops here (run G2 never came back from a step
    /// whose bus ran at a fraction of its speed: every loop is bounded by
    /// time, not only by count).
    const PART_US: u32 = 4_000_000;

    /// Every test at the current clock. `take_reference`: the 3D picture
    /// becomes the reference (Windows Mobile's clock, first).
    fn clock_check(g: &mut Rig, c: &mut ClockRig, take_reference: bool, seed: u32) -> ClockResult {
        let mut r = ClockResult {
            short_before: underrun_take(&mut g.e.regs),
            ..ClockResult::default()
        };
        // The bus: the flush and every word of it; 256 KiB read back.
        let t0 = rt::ticks();
        let src = words_as_bytes(c.src);
        for (k, chunk) in src.chunks(64 * 1024).enumerate() {
            let at = c.flush_at as usize + k * 64 * 1024;
            g.mem.0[at..at + chunk.len()].copy_from_slice(chunk);
            if us_since(t0) > PART_US {
                r.slow |= 1;
                break;
            }
        }
        cpu::drain_write_buffer();
        r.flush_us = us_since(t0);
        rt::check_in();
        let t0 = rt::ticks();
        let mut sum = 0u32;
        for k in 0..256 * 1024 / 4 {
            let v = g.mem.load(c.flush_at as usize + 4 * k).unwrap_or(0);
            sum = sum.wrapping_add(v);
            r.bus_wrong += u32::from(v != c.src[k]);
            if k % 1024 == 1023 && us_since(t0) > PART_US {
                r.slow |= 2;
                break;
            }
        }
        r.read_us = us_since(t0);
        let _ = core::hint::black_box(sum);
        for k in 256 * 1024 / 4..c.src.len() {
            r.bus_wrong += u32::from(g.mem.load(c.flush_at as usize + 4 * k) != Some(c.src[k]));
            if k % 1024 == 1023 && us_since(t0) > PART_US {
                r.slow |= 2;
                break;
            }
        }
        rt::check_in();
        // Memory: a pattern and its inverse over the pattern region (only
        // what was written in time is read back).
        let t0 = rt::ticks();
        'pattern: for s in [seed, seed + 1] {
            let mut buf = [0u32; 64];
            let mut a = 0;
            while a < c.pat_bytes {
                for (j, w) in buf.iter_mut().enumerate() {
                    *w = clock_pat(a + 4 * j as u32, s);
                }
                g.mem.store((c.pat_at + a) as usize, &buf);
                a += 256;
                if a % (64 * 1024) == 0 && us_since(t0) > PART_US {
                    r.slow |= 4;
                    break;
                }
            }
            let written = a;
            g.mem.sync();
            rt::check_in();
            let mut a = 0;
            while a < written {
                r.mem_wrong +=
                    u32::from(g.mem.load((c.pat_at + a) as usize) != Some(clock_pat(a, s)));
                a += 4;
                if a % (64 * 1024) == 0 {
                    rt::check_in();
                    if us_since(t0) > PART_US {
                        r.slow |= 4;
                        break 'pattern;
                    }
                }
            }
            if r.slow & 4 != 0 {
                break;
            }
        }
        // The 2D engine: a full-screen fill, timed, 4096 words checked.
        let colour = clock_pat(seed, 0x2D) | 0xFF00_0000;
        let w = cmd::fill(
            Format::BPP32,
            SCREEN.at as usize,
            SCREEN.stride as usize,
            (0, 0, 480, 640),
            colour,
        );
        let t0 = rt::ticks();
        let ok =
            g.e.submit(&w)
                .and_then(|()| g.e.finish(g.mem.0, 200_000))
                .is_ok();
        r.fill_us = us_since(t0);
        r.fill_hung = !ok;
        for k in 0..4096u32 {
            let at = SCREEN.at + (k * 7919 % (480 * 640)) * 4;
            r.fill_wrong += u32::from(g.mem.load(at as usize) != Some(colour));
        }
        rt::check_in();
        // The texture: damage since the last check counted, then repaired.
        let mut buf = [0u32; 64];
        for (k, chunk) in c.tex_copy.chunks(64).enumerate() {
            let at = c.tex.at as usize + 256 * k;
            let mut bad = false;
            for (j, &w) in chunk.iter().enumerate() {
                bad |= g.mem.load(at + 4 * j) != Some(w);
                r.tex_damaged += u32::from(g.mem.load(at + 4 * j) != Some(w));
            }
            if bad {
                buf[..chunk.len()].copy_from_slice(chunk);
                g.mem.store(at, &buf[..chunk.len()]);
            }
        }
        g.mem.sync();
        rt::check_in();
        // The 3D core: a scene of a textured layer, a translucent flat one
        // and a square, rendered whole three times; the picture against the
        // reference, word for word.
        if let Some(mut rr) = Renderer::new(&mut g.pool, 480, 640) {
            let _ = rr.set_background(Some(GREY));
            g.set(&mut rr, 0, Some(cover(c.tex, 16, false)));
            g.set(&mut rr, 1, Some(flat(0x8040_A0C0, true)));
            g.set(
                &mut rr,
                2,
                Some(Draw::rect(100, 200, 96, 96, Paint::Flat(0xFFFF_2000))),
            );
            let full = Target {
                picture: SCREEN,
                x: 0,
                y: 0,
            };
            let times = g.full(&mut rr, full, 3);
            r.render_us = times.min();
            for (k, want) in c.reference.iter_mut().enumerate() {
                let got = g.mem.load(SCREEN.at as usize + 4 * k).unwrap_or(0);
                if take_reference {
                    *want = got;
                } else if got != *want {
                    r.render_wrong += 1;
                    if r.first_diff.is_none() {
                        r.first_diff = Some((k as u32, got, *want));
                    }
                }
            }
            for k in 0..3 {
                g.set(&mut rr, k, None);
            }
            let _ = rr.release(&mut g.pool);
        }
        r.short_during = underrun_take(&mut g.e.regs);
        rt::check_in();
        r
    }

    /// The display's underrun flag, cleared Dell's way; whether it was set.
    fn underrun_take(regs: &mut Mmio) -> bool {
        let v = regs.read32(DINTRS);
        if v & GFX_UNDERRUN != 0 {
            regs.write32(DINTRS, v & !GFX_UNDERRUN);
            true
        } else {
            false
        }
    }

    /// Run V (see [`LOAD`]): at each clock, each kind of work alone, the
    /// underrun flag cleared before and read after.
    fn underrun_load(g: &mut Rig, c: &mut ClockRig) {
        log!("V: which work at which clock makes the display's fetch run short (DINTRS bit 4)");
        clock_card(
            g,
            ["V: LOAD TEST", "NO NEED TO", "WATCH CLOSELY"],
            0xFF20_2040,
            2,
        );
        let step = |m, n, p, div, cl3| ClockStep {
            m,
            n,
            p,
            div,
            cl3,
            restart: false,
        };
        let settings = [
            WM_STEP,
            step(23, 3, 0, 1, None),        // 99.67, the cores / 1 (G4: dark)
            step(23, 4, 0, 1, None),        // 74.75 / 1
            step(41, 4, 0, 2, Some(false)), // 133.25 CL2 / 2
        ];
        let mut seed = 0x400;
        for s in settings {
            let lock = clock_apply(g, s);
            let mut l = heapless_name();
            let _ = write!(
                l,
                "  {}.{:02} MHz, cores / {}:",
                s.khz() / 1000,
                s.khz() % 1000 / 10,
                s.div
            );
            if lock.is_none() {
                log!("{} the clock didn't take", l.as_str());
                clock_restore(&mut g.e.regs);
                continue;
            }
            let _ = underrun_take(&mut g.e.regs);
            // The bus: the 1.2 MB flush, five times.
            for _ in 0..5 {
                let at = c.flush_at as usize;
                g.mem.0[at..at + c.src.len() * 4].copy_from_slice(words_as_bytes(c.src));
            }
            cpu::drain_write_buffer();
            let _ = write!(l, " flush x5 {}", underrun_take(&mut g.e.regs));
            // The 2D engine: 20 full-screen fills.
            for k in 0..20u32 {
                let w = cmd::fill(
                    Format::BPP32,
                    SCREEN.at as usize,
                    SCREEN.stride as usize,
                    (0, 0, 480, 640),
                    0xFF00_0000 | (k * 0x0001_0101),
                );
                let _ = g.e.submit(&w).and_then(|()| g.e.finish(g.mem.0, 200_000));
            }
            let _ = write!(l, ", 2D fill x20 {}", underrun_take(&mut g.e.regs));
            // Everything: the clock check (memory pattern, 2D, 3D x3).
            let r = clock_check(g, c, false, seed);
            seed += 2;
            let _ = write!(
                l,
                ", whole check {} ({})",
                underrun_take(&mut g.e.regs),
                if r.good() { "GOOD" } else { "BAD" }
            );
            log!("{} (true: the display ran short)", l.as_str());
            rt::check_in();
            clock_restore(&mut g.e.regs);
            let _ = underrun_take(&mut g.e.regs);
        }
        clock_card(g, ["V DONE", "BACK TO NORMAL", "LOG SENT"], 0xFF10_6020, 3);
    }

    /// Windows Mobile's clock checked (after a restore): GOOD, or BAD
    /// logged with the registers, restored once more and checked again.
    fn clock_control(g: &mut Rig, c: &mut ClockRig, seed: u32, after: &str) -> bool {
        let r = clock_check(g, c, false, seed);
        if r.good() {
            log!("    control at 99.67 MHz {after}: GOOD");
            return true;
        }
        log!("    control at 99.67 MHz {after}: BAD");
        clock_log(WM_STEP, Some(0), &r);
        clock_readback(g);
        clock_restore(&mut g.e.regs);
        rt::delay_us(100_000);
        let r = clock_check(g, c, false, seed + 0x1_0000);
        clock_log(WM_STEP, Some(0), &r);
        log!(
            "    control again after a second restore: {}",
            if r.good() { "GOOD" } else { "BAD" }
        );
        if r.short_before || r.short_during {
            log!(
                "    the display ran short (DINTRS bit 4, cleared):{}{}",
                if r.short_before {
                    " before the check (the clock change into it)"
                } else {
                    ""
                },
                if r.short_during {
                    " during the check"
                } else {
                    ""
                }
            );
        }
        if r.tex_damaged != 0 {
            log!(
                "    the texture: {} words changed since the last check (repaired)",
                r.tex_damaged
            );
        }
        if r.slow != 0 {
            log!(
                "    stopped at {} s: {}{}{}",
                PART_US / 1_000_000,
                if r.slow & 1 != 0 { "the flush " } else { "" },
                if r.slow & 2 != 0 {
                    "the read-back "
                } else {
                    ""
                },
                if r.slow & 4 != 0 {
                    "the memory pattern"
                } else {
                    ""
                }
            );
        }
        r.good()
    }

    fn clock_log(s: ClockStep, lock: Option<u32>, r: &ClockResult) {
        let mhz = s.khz() / 1000;
        let mbs = |bytes: u32, us: u32| bytes / us.max(1);
        log!(
            "  G {}.{:02} MHz CL{}{}, cores / {} = {} MHz: lock {lock:?} us | flush {} us {} MB/s, read 256K {} us {} MB/s, bus wrong {} | memory wrong {} | 2D fill {} us wrong {}{} | 3D {} us wrong {} | {}",
            mhz,
            s.khz() % 1000 / 10,
            s.cl(),
            if s.restart { " restarted" } else { "" },
            s.div,
            s.khz() / s.div / 1000,
            r.flush_us,
            mbs(1_228_800, r.flush_us),
            r.read_us,
            mbs(262_144, r.read_us),
            r.bus_wrong,
            r.mem_wrong,
            r.fill_us,
            r.fill_wrong,
            if r.fill_hung { " HUNG" } else { "" },
            r.render_us,
            r.render_wrong,
            if r.good() { "GOOD" } else { "BAD" }
        );
        if let Some((k, got, want)) = r.first_diff {
            log!(
                "    first 3D difference at ({},{}): 0x{got:08x}, want 0x{want:08x}",
                k % 480,
                k / 480
            );
        }
    }

    /// Run G's memory (see [`ClockRig`]): the flush source and the
    /// reference picture in the SDRAM pool, the texture, the flush target
    /// and the pattern region in 2700G memory. `None` (logged) if any is
    /// missing.
    fn clock_rig(g: &mut Rig) -> Option<ClockRig<'static>> {
        let Some(ram) = rp_hal_x51v::devices::take_ram_pool() else {
            log!("  no RAM pool");
            return None;
        };
        let (src, rest) = ram.split_at_mut(480 * 640);
        let (reference, rest) = rest.split_at_mut(480 * 640);
        for (i, w) in src.iter_mut().enumerate() {
            *w = clock_pat(4 * i as u32, 0xF1A5);
        }
        let Some(tex) = tex16(g, 6, format::RGB565, 1, &pattern565) else {
            log!("  no memory for the texture");
            return None;
        };
        let (Some(flush_at), Some(pat_at)) =
            (g.pool.alloc(480 * 640 * 4, 128), g.pool.alloc(4 << 20, 128))
        else {
            log!("  no 2700G memory");
            return None;
        };
        let (tex_copy, _) = rest.split_at_mut(tex.bytes() as usize / 4);
        for (k, w) in tex_copy.iter_mut().enumerate() {
            *w = g.mem.load(tex.at as usize + 4 * k).unwrap_or(0);
        }
        Some(ClockRig {
            tex,
            tex_copy,
            flush_at,
            pat_at,
            pat_bytes: 4 << 20,
            src,
            reference,
        })
    }

    /// Gives [`clock_rig`]'s 2700G memory back.
    fn clock_rig_free(g: &mut Rig, c: ClockRig) {
        let _ = g.pool.free(c.flush_at, 480 * 640 * 4);
        let _ = g.pool.free(c.pat_at, c.pat_bytes);
        let _ = rp_mbx3d_render::release(&mut g.pool, c.tex);
    }

    #[inline(never)]
    fn clocks(g: &mut Rig) {
        log!(
            "G5: the clocks: the lock bit traced; the cores at divide 1 and 2 up to 133.25 MHz with CL2; each step twice, rising then in reverse, every part time-limited, the display's underrun read around every check, Windows Mobile's clock checked after each, a 60 s soak at 99.67 MHz with the cores at divide 1"
        );
        let found = (
            g.e.regs.read32(clock::COREPLL),
            g.e.regs.read32(MBXCLK),
            diag::msc1(),
        );
        log!(
            "  as found: COREPLL 0x{:x}, MBXCLK 0x{:x}, MSC1 0x{:08x}",
            found.0,
            found.1,
            found.2
        );
        if found.0 != ClockSpeed::WM.pll || found.2 != 0x2484_2481 {
            log!("  not Windows Mobile's clock and bus timing: nothing changed");
            return;
        }
        let Some(mut c) = clock_rig(g) else {
            return;
        };

        // Windows Mobile's clock: the reference.
        let base = clock_check(g, &mut c, true, 2);
        clock_log(WM_STEP, Some(0), &base);
        if !base.good() {
            log!("  the reference step itself is BAD: stopped");
            return;
        }

        if LOAD {
            underrun_load(g, &mut c);
        } else {
            // The lock bit traced across a big change and a small one, each
            // way; then Windows Mobile's clock checked again.
            let a75 = ClockStep { n: 4, ..WM_STEP }.speed();
            let f108 = ClockStep { m: 25, ..WM_STEP }.speed();
            clock_card(
                g,
                ["LOCK TRACE", "4 PLL CHANGES", "TESTING"],
                0xFF20_2040,
                2,
            );
            lock_trace(g, a75, "99.67 -> 74.75 MHz");
            lock_trace(g, ClockSpeed::WM, "74.75 -> 99.67 MHz");
            lock_trace(g, f108, "99.67 -> 108.33 MHz");
            lock_trace(g, ClockSpeed::WM, "108.33 -> 99.67 MHz");
            let mut healthy = clock_control(g, &mut c, 3, "after the lock traces");

            // Each step twice, every step in pass 1 and again in reverse in
            // pass 2 (a failure that depends on the step before shows as a
            // difference); Windows Mobile's clock checked after every step (a
            // failure carried over shows there). Nothing stops at a BAD step:
            // only a control that stays BAD stops the run.
            // G2 on the device: steps 1-6 GOOD (the owner saw them green), then
            // 108.33 MHz CL3 never came back. CL3 last in each pass now, the
            // memory-restart variant first.
            let step = |m, n, p, div, cl3, restart| ClockStep {
                m,
                n,
                p,
                div,
                cl3,
                restart,
            };
            let steps = [
                step(23, 3, 0, 1, None, false),        // 99.67, the divider alone
                step(23, 3, 1, 1, None, false),        // 49.83 /1 (G1: GOOD, then BAD)
                step(23, 4, 0, 2, None, false),        // 74.75 /2: the PLL change alone
                step(23, 4, 0, 1, None, false),        // 74.75 /1 (G1: BAD)
                step(20, 4, 0, 1, None, false),        // 65.00 /1
                step(25, 3, 0, 2, Some(false), false), // 108.33 CL2 (G2: GOOD)
                step(27, 3, 0, 2, Some(false), false), // 117.00 CL2
                step(41, 4, 0, 2, Some(false), false), // 133.25 CL2 (run 27: right)
                // G5: CL3 dropped (G4: the 3D core wrong in all 24 checks,
                // with or without the memory restart: settled); the cores
                // at divide 1 past 100 MHz, new.
                step(25, 3, 0, 1, Some(false), false), // 108.33 CL2 /1
                step(27, 3, 0, 1, Some(false), false), // 117.00 CL2 /1
                step(41, 4, 0, 1, Some(false), false), // 133.25 CL2 /1
            ];
            let total = 2 * steps.len();
            let mut seed = 4;
            let mut k = 0;
            'passes: for pass in 1..=2u32 {
                log!(
                    "  pass {pass}: {}",
                    if pass == 1 {
                        "rising"
                    } else {
                        "the same steps in reverse"
                    }
                );
                for i in 0..steps.len() {
                    let s = if pass == 1 {
                        steps[i]
                    } else {
                        steps[steps.len() - 1 - i]
                    };
                    k += 1;
                    let (mut l1, mut l2, mut l3) =
                        (heapless_name(), heapless_name(), heapless_name());
                    let _ = write!(l1, "STEP {k} OF {total}");
                    let _ = write!(
                        l2,
                        "{}.{:02} MHZ CORES/{}",
                        s.khz() / 1000,
                        s.khz() % 1000 / 10,
                        s.div
                    );
                    let _ = write!(
                        l3,
                        "CL{}{} TESTING",
                        s.cl(),
                        if s.restart { " RESTART" } else { "" }
                    );
                    clock_card(g, [l1.as_str(), l2.as_str(), l3.as_str()], 0xFF20_2040, 2);
                    let lock = clock_apply(g, s);
                    clock_readback(g);
                    let mut good = 0;
                    for _ in 0..2 {
                        let r = if lock.is_some() {
                            clock_check(g, &mut c, false, seed)
                        } else {
                            ClockResult::default()
                        };
                        seed += 2;
                        clock_log(s, lock, &r);
                        good += u32::from(r.good());
                    }
                    clock_restore(&mut g.e.regs);
                    let mut done = heapless_name();
                    let _ = write!(done, "step {k}");
                    healthy = clock_control(g, &mut c, seed, done.as_str());
                    seed += 2;
                    let mut l3 = heapless_name();
                    let _ = write!(
                        l3,
                        "{}",
                        if good == 2 {
                            "GOOD"
                        } else if good == 0 {
                            "BAD"
                        } else {
                            "MIXED"
                        }
                    );
                    let bg = if good == 2 { 0xFF10_6020 } else { 0xFF80_2010 };
                    clock_card(g, [l1.as_str(), l2.as_str(), l3.as_str()], bg, 2);
                    if !healthy {
                        break 'passes;
                    }
                }
            }
            // The soak: 60 s of checks at the best setting so far (the
            // cores at divide 1, the memory at Windows Mobile's clock).
            if healthy {
                let s = step(23, 3, 0, 1, None, false);
                clock_card(
                    g,
                    ["SOAK 60 S", "99.67 MHZ CORES/1", "TESTING"],
                    0xFF20_2040,
                    2,
                );
                let lock = clock_apply(g, s);
                let t0 = rt::ticks();
                let (mut rounds, mut bad, mut short) = (0u32, 0u32, 0u32);
                while lock.is_some() && us_since(t0) < 60_000_000 {
                    let r = clock_check(g, &mut c, false, seed);
                    seed += 2;
                    rounds += 1;
                    short += u32::from(r.short_before || r.short_during);
                    if !r.good() {
                        bad += 1;
                        clock_log(s, lock, &r);
                    }
                }
                clock_restore(&mut g.e.regs);
                log!(
                    "  soak at 99.67 MHz, cores / 1: {rounds} rounds in {} s, {bad} BAD, the display short in {short}",
                    us_since(t0) / 1_000_000
                );
                healthy = clock_control(g, &mut c, seed, "after the soak");
            }
            if healthy {
                clock_card(
                    g,
                    ["RUN G5 DONE", "BACK TO NORMAL", "LOG SENT"],
                    0xFF10_6020,
                    3,
                );
            } else {
                log!("  STOPPED: Windows Mobile's clock stayed BAD after a step");
                clock_card(
                    g,
                    ["RUN G5 STOPPED", "GPU NOT RECOVERED", "SOFT RESET NEEDED"],
                    0xFF80_2010,
                    3,
                );
            }
        }
        let after = (
            g.e.regs.read32(clock::COREPLL),
            g.e.regs.read32(MBXCLK),
            diag::msc1(),
        );
        log!(
            "  put back: COREPLL 0x{:x}, MBXCLK 0x{:x}, MSC1 0x{:08x}",
            after.0,
            after.1,
            after.2
        );
        clock_rig_free(g, c);
    }

    fn main() -> ! {
        rt::set_fault_handler(on_fault);
        rt::init_mmu();
        let Some((mut regs, vram)) = rp_hal_x51v::devices::take_mbx() else {
            crash(format_args!("no 2700G"))
        };
        if START {
            start_watch(&mut regs, "N0 the 2700G handed out");
        }
        // A log an earlier run kept in the internal SRAM (it hung and the
        // device was reset: docs/hw/memory-map.md), reported; then every run
        // keeps its own there until it returns (run Z's log was lost to a
        // watchdog reset with only the clock runs keeping one: the class
        // fixed, every run keeps it, every return clears it).
        let opened = rp_hal_x51v::devices::kept_open(|k, copy| match copy {
            Ok(text) => {
                log!(
                    "SRAM copy {k}: a kept log from an earlier run ({} bytes):",
                    text.len()
                );
                log!("{text}");
                log!("SRAM copy {k}: end of the kept log");
            }
            Err(why) => log!("SRAM copy {k}: {why}"),
        });
        if opened {
            rp_hal_x51v::devices::kept_start();
            log!("{MARKER}: the log kept in SRAM from here");
        }
        if START {
            start_watch(&mut regs, "N1 the SRAM log opened");
        }
        if SNAPSHOT {
            snapshot(regs, vram);
        }
        if UNDERRUN {
            underrun(regs, vram);
        }
        if CLEAR {
            clear_underrun(regs, vram);
        }
        if LADDER {
            ladder(regs, vram);
        }
        if REBUILD {
            rebuild(regs, vram);
        }
        if REVIVE {
            revive(regs, vram);
        }
        let mut display = Display::new(regs);
        let _ = display.adopt(vram.len());
        let mut regs = display.into_inner();
        if START {
            start_watch(&mut regs, "N2 the display adopted");
        }
        let mut e = Engine::new(regs, map::FENCE.at as usize, rt::ticks, 3_250);
        if START {
            start_watch(&mut e.regs, "N3 the 2D engine taken");
        }
        let clk0 = e.regs.read32(MBXCLK);
        let _ = store32_volatile(vram, map::FENCE.at as usize, 0);
        cpu::drain_write_buffer();
        log!(
            "{MARKER}: run {}, MBXCLK 0x{clk0:x}",
            if PROBES {
                "B (texture-word probes)"
            } else if PLANES {
                "C (the video plane: repoint, YUV pan under glass, RGB pan under a tint)"
            } else if VFMT {
                "D (the video plane's undocumented format codes)"
            } else if UPLOAD {
                "E (faster uploads: overlapped reorder and DMA, 8-bit, PVRTC, text by commands)"
            } else if TEXT {
                "F (text as 4 bpp coverage strips, against RGB565 pixels)"
            } else if KEPT {
                "K (a log kept in SRAM by an earlier run, only)"
            } else if FREEZE {
                "H (the card log against a frozen bus: 40 lines, then a read the 2700G never answers)"
            } else if RESCUE {
                "Y (the rescue measured: Windows Mobile's clock checked before run Z's rebuild, after it, after each of three memory restarts and with SYSCFG put back)"
            } else if LOAD {
                "V (which work at which clock makes the display's fetch run short)"
            } else if SLOW {
                "Q (what ends the slow state: 108.33 MHz CL2, the memory restarted at Windows Mobile's clock, then G4c's 108.33 MHz CL3 with the restart, Windows Mobile's clock measured around each)"
            } else if START {
                "N (the start-up watched: the display read after every stage, then G5's opening, cards to report)"
            } else if CLOCK {
                "G5 (the clocks: lock traces, divide 1 and 2 to 133.25 MHz with CL2, the display's underrun watched, a 60 s soak, memory-controller writes checked, the texture checked and repaired, time limits, controls after each step, the log kept in SRAM)"
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
        if START {
            start_watch(&mut g.e.regs, "N4 the keypad taken, the fence written");
        }
        g.fill(SCREEN.at, (0, 0, 480, 640), BLACK);
        if START {
            start_watch(
                &mut g.e.regs,
                "N5 the screen buffer filled black (not shown yet)",
            );
        }
        // Every plane register as found (the video plane, the cursor, the
        // background colour, the graphics plane), put back at the end.
        let planes0 = Snapshot::take(&mut g.e.regs);
        let saved = show_screen(&mut g.e.regs);
        if START {
            start_watch(&mut g.e.regs, "N6 the screen buffer shown (black)");
        }
        if PROBES {
            probes(&mut g);
        } else if PLANES {
            planes(&mut g);
        } else if VFMT {
            vformats(&mut g);
        } else if UPLOAD {
            uploads(&mut g);
        } else if TEXT {
            text_strips(&mut g);
        } else if KEPT {
            // Only the kept log, reported above.
        } else if FREEZE {
            freeze(&mut g);
        } else if RESCUE {
            rescue(&mut g);
            rp_hal_x51v::devices::kept_stop();
        } else if SLOW {
            slow_state(&mut g);
            rp_hal_x51v::devices::kept_stop();
        } else if START {
            start_opening(&mut g);
            rp_hal_x51v::devices::kept_stop();
        } else if CLOCK || LOAD {
            clocks(&mut g);
            rp_hal_x51v::devices::kept_stop();
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
        planes0.restore(&mut g.e.regs);
        g.e.reset(rt::delay_us);
        g.e.regs.write32(MBXCLK, clk0);
        back_to_loader();
        loop {
            core::hint::spin_loop();
        }
    }

    // ---- Run S: a read-only snapshot of the 2700G (2026-10-20) -------------

    /// Every register Linux mbxfb names below the MBX block (its header,
    /// `local/re/mbxfb_regs.h`), the write-only resets and clears left out.
    const SNAPSHOT_REGS: [(&str, usize); 146] = [
        ("SYSCFG", 0x0000),
        ("PFBASE", 0x0004),
        ("PFCEIL", 0x0008),
        ("POLLFLAG", 0x000C),
        ("NINTPW", 0x0014),
        ("MINTENABLE", 0x0018),
        ("MINTSTAT", 0x001C),
        ("SINTENABLE", 0x0020),
        ("SINTSTAT", 0x0024),
        ("SYSCLKSRC", 0x002C),
        ("PIXCLKSRC", 0x0030),
        ("CLKSLEEP", 0x0034),
        ("COREPLL", 0x0038),
        ("DISPPLL", 0x003C),
        ("PLLSTAT", 0x0040),
        ("VOVRCLK", 0x0044),
        ("PIXCLK", 0x0048),
        ("MEMCLK", 0x004C),
        ("M24CLK", 0x0050),
        ("MBXCLK", 0x0054),
        ("SDCLK", 0x0058),
        ("PIXCLKDIV", 0x005C),
        ("LCD_CONFIG", 0x0060),
        ("ODFBPWR", 0x0064),
        ("ODFBSTAT", 0x0068),
        ("GPIOCGF", 0x006C),
        ("GPIOHI", 0x0070),
        ("GPIOLO", 0x0074),
        ("GPIOSTAT", 0x0078),
        ("PWMCFG", 0x0204),
        ("PWM0DIV", 0x0210),
        ("PWM0DUTY", 0x0214),
        ("PWM0PER", 0x0218),
        ("PWM1DIV", 0x0220),
        ("PWM1DUTY", 0x0224),
        ("PWM1PER", 0x0228),
        ("ID", 0x0FF0),
        ("LMCFG", 0x1004),
        ("LMPWR", 0x1008),
        ("LMPWRSTAT", 0x100C),
        ("LMCEMR", 0x1010),
        ("LMTYPE", 0x1014),
        ("LMTIM", 0x1018),
        ("LMREFRESH", 0x101C),
        ("LMPROTMIN", 0x1020),
        ("LMPROTMAX", 0x1024),
        ("LMPROTCFG", 0x1028),
        ("LMPROTERR", 0x102C),
        ("GSCTRL", 0x2000),
        ("VSCTRL", 0x2004),
        ("GBBASE", 0x2020),
        ("VBBASE", 0x2024),
        ("GDRCTRL", 0x2040),
        ("VCMSK", 0x2044),
        ("GSCADR", 0x2060),
        ("VSCADR", 0x2064),
        ("VUBASE", 0x2084),
        ("VVBASE", 0x20A4),
        ("GSADR", 0x20C0),
        ("VSADR", 0x20C4),
        ("HCCTRL", 0x2100),
        ("HCSIZE", 0x2110),
        ("HCPOS", 0x2120),
        ("HCBADR", 0x2130),
        ("HCCKMSK", 0x2140),
        ("GPLUT", 0x2150),
        ("DSCTRL", 0x2154),
        ("DHT01", 0x2158),
        ("DHT02", 0x215C),
        ("DHT03", 0x2160),
        ("DVT01", 0x2164),
        ("DVT02", 0x2168),
        ("DVT03", 0x216C),
        ("DBCOL", 0x2170),
        ("BGCOLOR", 0x2174),
        ("DINTRS", 0x2178),
        ("DINTRE", 0x217C),
        ("DINTRCNT", 0x2180),
        ("DSIG", 0x2184),
        ("DMCTRL", 0x2188),
        ("CLIPCTRL", 0x218C),
        ("SPOCTRL", 0x2190),
        ("SVCTRL", 0x2194),
        ("VSCOEFF0", 0x2198),
        ("VSCOEFF1", 0x219C),
        ("VSCOEFF2", 0x21A0),
        ("VSCOEFF3", 0x21A4),
        ("VSCOEFF4", 0x21A8),
        ("SHCTRL", 0x21B0),
        ("HSCOEFF0", 0x21B4),
        ("HSCOEFF1", 0x21B8),
        ("HSCOEFF2", 0x21BC),
        ("HSCOEFF3", 0x21C0),
        ("HSCOEFF4", 0x21C4),
        ("HSCOEFF5", 0x21C8),
        ("HSCOEFF6", 0x21CC),
        ("HSCOEFF7", 0x21D0),
        ("HSCOEFF8", 0x21D4),
        ("SSSIZE", 0x21D8),
        ("VIDGAM0", 0x2200),
        ("VIDGAM1", 0x2204),
        ("VIDGAM2", 0x2208),
        ("VIDGAM3", 0x220C),
        ("VIDGAM4", 0x2210),
        ("VIDGAM5", 0x2214),
        ("VIDGAM6", 0x2218),
        ("VIDGAM7", 0x221C),
        ("VIDGAM8", 0x2220),
        ("VIDGAM9", 0x2224),
        ("VIDGAM10", 0x2228),
        ("VIDGAM11", 0x222C),
        ("VIDGAM12", 0x2230),
        ("VIDGAM13", 0x2234),
        ("VIDGAM14", 0x2238),
        ("VIDGAM15", 0x223C),
        ("VIDGAM16", 0x2240),
        ("GFXGAM0", 0x2250),
        ("GFXGAM1", 0x2254),
        ("GFXGAM2", 0x2258),
        ("GFXGAM3", 0x225C),
        ("GFXGAM4", 0x2260),
        ("GFXGAM5", 0x2264),
        ("GFXGAM6", 0x2268),
        ("GFXGAM7", 0x226C),
        ("GFXGAM8", 0x2270),
        ("GFXGAM9", 0x2274),
        ("GFXGAM10", 0x2278),
        ("GFXGAM11", 0x227C),
        ("GFXGAM12", 0x2280),
        ("GFXGAM13", 0x2284),
        ("GFXGAM14", 0x2288),
        ("GFXGAM15", 0x228C),
        ("GFXGAM16", 0x2290),
        ("DLSTS", 0x2300),
        ("DLLCTRL", 0x2304),
        ("DVLNUM", 0x2308),
        ("DUCTRL", 0x230C),
        ("DVECTRL", 0x2310),
        ("DHDET", 0x2314),
        ("DVDET", 0x2318),
        ("DODMSK", 0x231C),
        ("CSC01", 0x2330),
        ("CSC02", 0x2334),
        ("CSC03", 0x2338),
        ("CSC04", 0x233C),
        ("CSC05", 0x2340),
    ];

    /// Run S: [`SNAPSHOT_REGS`] read and logged, the line counter sampled,
    /// the first words of the scanned-out picture read; nothing written.
    fn snapshot(regs: Mmio, vram: &mut [u8]) -> ! {
        let mut regs = regs;
        dump_all(
            &mut regs,
            "S: every named register below the MBX block, read only",
        );
        revive_lines(&mut regs);
        let mut words = heapless_name();
        for k in 0..6 {
            let at = 640 * 960 / 2 + 480 + 4 * k;
            let _ = write!(words, " 0x{:08x}", load32_volatile(vram, at).unwrap_or(0));
        }
        log!("  the picture at offset 0, mid-screen:{}", words.as_str());
        log!("{MARKER}: done (run S)");
        back_to_loader();
        loop {
            core::hint::spin_loop();
        }
    }

    // ---- Run U: the underrun reproduced and cleared (2026-10-20) ----------

    /// `DINTRS` bit 4: the graphics stream ran short (Linux mbxfb's
    /// `STR1_UR`).
    const GFX_UNDERRUN: u32 = 1 << 4;
    const DINTRS: usize = 0x2178;
    const DSIG: usize = 0x2184;

    /// The flag and the output signature now, and the signature 20 ms on.
    fn underrun_state(regs: &mut Mmio) -> (u32, u32, u32) {
        let (flags, sig) = (regs.read32(DINTRS), regs.read32(DSIG));
        rt::delay_us(20_000);
        (flags, sig, regs.read32(DSIG))
    }

    /// Held 4 s as it is (the owner sees whether it went dark), then the
    /// underrun flag cleared Dell's way if it's set, and logged.
    fn underrun_hold_and_clear(regs: &mut Mmio, n: u32) {
        for _ in 0..40 {
            rt::delay_us(100_000);
            rt::check_in();
        }
        let v = regs.read32(DINTRS);
        if v & GFX_UNDERRUN != 0 {
            regs.write32(DINTRS, v & !GFX_UNDERRUN);
            rt::delay_us(40_000);
            let (f, s0, s1) = underrun_state(regs);
            log!(
                "U{n}: the flag cleared (Dell's way): DINTRS 0x{f:x}, DSIG 0x{s0:x} then 0x{s1:x}"
            );
        }
    }

    /// Run X (see [`CLEAR`]): nothing but the flag, then the plane as found
    /// gets a card (drawn by the CPU into the picture already scanned out).
    fn clear_underrun(regs: Mmio, vram: &mut [u8]) -> ! {
        let mut regs = regs;
        let (f, s0, s1) = underrun_state(&mut regs);
        log!("X: as found: DINTRS 0x{f:x}, DSIG 0x{s0:x} then 0x{s1:x}");
        let v = regs.read32(DINTRS);
        regs.write32(DINTRS, v & !GFX_UNDERRUN);
        rt::delay_us(40_000);
        let (f, s0, s1) = underrun_state(&mut regs);
        log!("X: the flag cleared (Dell's way): DINTRS 0x{f:x}, DSIG 0x{s0:x} then 0x{s1:x}");
        let plane = Plane::decode(
            regs.read32(dreg::GSCTRL),
            regs.read32(dreg::GSCADR),
            regs.read32(dreg::GSADR),
            vram.len(),
        );
        if let Some(p) = plane {
            card_text(vram, &p, "X: CLEARED", 0xFF10_6020, 4);
            let (f, s0, s1) = underrun_state(&mut regs);
            log!("X: after the card: DINTRS 0x{f:x}, DSIG 0x{s0:x} then 0x{s1:x}");
            // Run X1 on the device: the flag cleared, DSIG stayed 0. With
            // the flag clear, each restart alone (run R tried them with it
            // set), DSIG read after each: nonzero is the display sending.
            let steps: [(&str, &str); 3] = [
                ("X1: the sync generator off and on", "X1: SYNC"),
                ("X2: the graphics plane off and on", "X2: PLANE"),
                ("X3: the whole display set-up", "X3: SETUP"),
            ];
            for (k, (what, card)) in steps.into_iter().enumerate() {
                let _ = underrun_take(&mut regs);
                match k {
                    0 => {
                        let v = regs.read32(dreg::DSCTRL);
                        regs.write32(dreg::DSCTRL, v & !(1 << 31));
                        rt::delay_us(20_000);
                        regs.write32(dreg::DSCTRL, v | 1 << 31);
                    }
                    1 => {
                        let v = regs.read32(dreg::GSCADR);
                        regs.write32(dreg::GSCADR, v & !(1 << 31));
                        rt::delay_us(20_000);
                        regs.write32(dreg::GSCADR, v | 1 << 31);
                    }
                    _ => {
                        let mut d = Display::new(regs);
                        d.enable(&rp_drv_mbx_display::X51V_VGA, &p);
                        regs = d.into_inner();
                    }
                }
                rt::delay_us(40_000);
                let (f, s0, s1) = underrun_state(&mut regs);
                log!(
                    "{what}: DINTRS 0x{f:x}, DSIG 0x{s0:x} then 0x{s1:x} (sending: {})",
                    s0 | s1 != 0
                );
                card_text(vram, &p, card, 0xFF10_6020, 4);
            }
        } else {
            log!("X: no plane decoded: no card");
        }
        log!("{MARKER}: done (run X)");
        back_to_loader();
        loop {
            core::hint::spin_loop();
        }
    }

    /// Waits up to `ms` for `reg & mask == want`; whether it got there.
    fn wait_for(regs: &mut Mmio, reg: usize, mask: u32, want: u32, ms: u32) -> bool {
        for _ in 0..ms * 10 {
            if regs.read32(reg) & mask == want {
                return true;
            }
            rt::delay_us(100);
        }
        false
    }

    /// A working X51v's 2700G (run S, `local/rp-gpubench-runS1-healthy.txt`,
    /// 2026-10-20, after a soft reset into Windows Mobile and the loader):
    /// name, offset, value.
    const HEALTHY: [(&str, usize, u32); 146] = [
        ("SYSCFG", 0x0000, 0x0),
        ("PFBASE", 0x0004, 0x0),
        ("PFCEIL", 0x0008, 0x3FDFFE0),
        ("POLLFLAG", 0x000C, 0x0),
        ("NINTPW", 0x0014, 0x1C),
        ("MINTENABLE", 0x0018, 0x800F),
        ("MINTSTAT", 0x001C, 0x0),
        ("SINTENABLE", 0x0020, 0x0),
        ("SINTSTAT", 0x0024, 0x10),
        ("SYSCLKSRC", 0x002C, 0x2),
        ("PIXCLKSRC", 0x0030, 0x1),
        ("CLKSLEEP", 0x0034, 0x0),
        ("COREPLL", 0x0038, 0xBB1),
        ("DISPPLL", 0x003C, 0xFC5),
        ("PLLSTAT", 0x0040, 0x7),
        ("VOVRCLK", 0x0044, 0x0),
        ("PIXCLK", 0x0048, 0x1),
        ("MEMCLK", 0x004C, 0x1),
        ("M24CLK", 0x0050, 0x2),
        ("MBXCLK", 0x0054, 0x5),
        ("SDCLK", 0x0058, 0x1),
        ("PIXCLKDIV", 0x005C, 0x1),
        ("LCD_CONFIG", 0x0060, 0x2EFA0000),
        ("ODFBPWR", 0x0064, 0x0),
        ("ODFBSTAT", 0x0068, 0x4),
        ("GPIOCGF", 0x006C, 0x1),
        ("GPIOHI", 0x0070, 0x0),
        ("GPIOLO", 0x0074, 0x0),
        ("GPIOSTAT", 0x0078, 0x0),
        ("PWMCFG", 0x0204, 0x3),
        ("PWM0DIV", 0x0210, 0x0),
        ("PWM0DUTY", 0x0214, 0x0),
        ("PWM0PER", 0x0218, 0x4),
        ("PWM1DIV", 0x0220, 0x0),
        ("PWM1DUTY", 0x0224, 0x0),
        ("PWM1PER", 0x0228, 0x4),
        ("ID", 0x0FF0, 0x1727189),
        ("LMCFG", 0x1004, 0x27),
        ("LMPWR", 0x1008, 0x0),
        ("LMPWRSTAT", 0x100C, 0x0),
        ("LMCEMR", 0x1010, 0x0),
        ("LMTYPE", 0x1014, 0xAC8),
        ("LMTIM", 0x1018, 0x73392),
        ("LMREFRESH", 0x101C, 0x615),
        ("LMPROTMIN", 0x1020, 0x0),
        ("LMPROTMAX", 0x1024, 0x3FDFFFC),
        ("LMPROTCFG", 0x1028, 0x0),
        ("LMPROTERR", 0x102C, 0x0),
        ("GSCTRL", 0x2000, 0x3C0EFA7F),
        ("VSCTRL", 0x2004, 0x600780F),
        ("GBBASE", 0x2020, 0x0),
        ("VBBASE", 0x2024, 0x0),
        ("GDRCTRL", 0x2040, 0xFFFFFF),
        ("VCMSK", 0x2044, 0xFFFFFF),
        ("GSCADR", 0x2060, 0x80000000),
        ("VSCADR", 0x2064, 0x1000000),
        ("VUBASE", 0x2084, 0x0),
        ("VVBASE", 0x20A4, 0x0),
        ("GSADR", 0x20C0, 0xEC00000),
        ("VSADR", 0x20C4, 0x0),
        ("HCCTRL", 0x2100, 0x0),
        ("HCSIZE", 0x2110, 0x40000000),
        ("HCPOS", 0x2120, 0x0),
        ("HCBADR", 0x2130, 0x0),
        ("HCCKMSK", 0x2140, 0xFFFFFF),
        ("GPLUT", 0x2150, 0x0),
        ("DSCTRL", 0x2154, 0x8000002A),
        ("DHT01", 0x2158, 0x400288),
        ("DHT02", 0x215C, 0x500050),
        ("DHT03", 0x2160, 0x2300230),
        ("DVT01", 0x2164, 0x20284),
        ("DVT02", 0x2168, 0x30003),
        ("DVT03", 0x216C, 0x2830283),
        ("DBCOL", 0x2170, 0x0),
        ("BGCOLOR", 0x2174, 0x0),
        ("DINTRS", 0x2178, 0x5),
        ("DINTRE", 0x217C, 0x0),
        ("DINTRCNT", 0x2180, 0x0),
        ("DSIG", 0x2184, 0xFE6D32F),
        ("DMCTRL", 0x2188, 0x10303010),
        ("CLIPCTRL", 0x218C, 0x0),
        ("SPOCTRL", 0x2190, 0xC8030000),
        ("SVCTRL", 0x2194, 0x0),
        ("VSCOEFF0", 0x2198, 0x0),
        ("VSCOEFF1", 0x219C, 0x0),
        ("VSCOEFF2", 0x21A0, 0x0),
        ("VSCOEFF3", 0x21A4, 0x0),
        ("VSCOEFF4", 0x21A8, 0x0),
        ("SHCTRL", 0x21B0, 0x20000000),
        ("HSCOEFF0", 0x21B4, 0x0),
        ("HSCOEFF1", 0x21B8, 0x0),
        ("HSCOEFF2", 0x21BC, 0x0),
        ("HSCOEFF3", 0x21C0, 0x0),
        ("HSCOEFF4", 0x21C4, 0x0),
        ("HSCOEFF5", 0x21C8, 0x0),
        ("HSCOEFF6", 0x21CC, 0x0),
        ("HSCOEFF7", 0x21D0, 0x0),
        ("HSCOEFF8", 0x21D4, 0x0),
        ("SSSIZE", 0x21D8, 0x0),
        ("VIDGAM0", 0x2200, 0x0),
        ("VIDGAM1", 0x2204, 0x101010),
        ("VIDGAM2", 0x2208, 0x202020),
        ("VIDGAM3", 0x220C, 0x303030),
        ("VIDGAM4", 0x2210, 0x404040),
        ("VIDGAM5", 0x2214, 0x505050),
        ("VIDGAM6", 0x2218, 0x606060),
        ("VIDGAM7", 0x221C, 0x6F6F6F),
        ("VIDGAM8", 0x2220, 0x808080),
        ("VIDGAM9", 0x2224, 0x909090),
        ("VIDGAM10", 0x2228, 0xA0A0A0),
        ("VIDGAM11", 0x222C, 0xB0B0B0),
        ("VIDGAM12", 0x2230, 0xC0C0C0),
        ("VIDGAM13", 0x2234, 0xCFCFCF),
        ("VIDGAM14", 0x2238, 0xDFDFDF),
        ("VIDGAM15", 0x223C, 0xF0F0F0),
        ("VIDGAM16", 0x2240, 0xFFFFFF),
        ("GFXGAM0", 0x2250, 0x0),
        ("GFXGAM1", 0x2254, 0xF0F0F),
        ("GFXGAM2", 0x2258, 0x1F1F1F),
        ("GFXGAM3", 0x225C, 0x2F2F2F),
        ("GFXGAM4", 0x2260, 0x3F3F3F),
        ("GFXGAM5", 0x2264, 0x4F4F4F),
        ("GFXGAM6", 0x2268, 0x5F5F5F),
        ("GFXGAM7", 0x226C, 0x6F6F6F),
        ("GFXGAM8", 0x2270, 0x7F7F7F),
        ("GFXGAM9", 0x2274, 0x8E8E8E),
        ("GFXGAM10", 0x2278, 0x9E9E9E),
        ("GFXGAM11", 0x227C, 0xAEAEAE),
        ("GFXGAM12", 0x2280, 0xBEBEBE),
        ("GFXGAM13", 0x2284, 0xCECECE),
        ("GFXGAM14", 0x2288, 0xDEDEDE),
        ("GFXGAM15", 0x228C, 0xEEEEEE),
        ("GFXGAM16", 0x2290, 0xFDFDFD),
        ("DLSTS", 0x2300, 0x800000),
        ("DLLCTRL", 0x2304, 0x0),
        ("DVLNUM", 0x2308, 0xE2),
        ("DUCTRL", 0x230C, 0x0),
        ("DVECTRL", 0x2310, 0x2830002),
        ("DHDET", 0x2314, 0x500230),
        ("DVDET", 0x2318, 0x30283),
        ("DODMSK", 0x231C, 0x0),
        ("CSC01", 0x2330, 0x12A),
        ("CSC02", 0x2334, 0x95199),
        ("CSC03", 0x2338, 0x39879C),
        ("CSC04", 0x233C, 0x10292A),
        ("CSC05", 0x2340, 0x0),
    ];

    /// The healthy value of `name`.
    fn healthy(name: &str) -> u32 {
        HEALTHY.iter().find(|r| r.0 == name).map_or(0, |r| r.2)
    }

    /// Every [`SNAPSHOT_REGS`] register logged, four to a line.
    fn dump_all(regs: &mut Mmio, title: &str) {
        log!("{title}");
        let mut line = heapless_name();
        for (k, (name, off)) in SNAPSHOT_REGS.iter().enumerate() {
            let _ = write!(line, " {name} 0x{:x}", regs.read32(*off));
            if k % 4 == 3 {
                log!(" {}", line.as_str());
                line = heapless_name();
            }
        }
        if line.n != 0 {
            log!(" {}", line.as_str());
        }
    }

    /// Writes the healthy value of `name` and waits 1 ms (as Linux mbxfb
    /// writes its set-up); no read.
    fn write_healthy(regs: &mut Mmio, name: &str) {
        if let Some(&(_, off, v)) = HEALTHY.iter().find(|r| r.0 == name) {
            regs.write32(off, v);
        }
        rt::delay_us(1_000);
    }

    /// Writes the healthy value of each of `names`, 1 ms after each (as
    /// Linux mbxfb writes its set-up), and reads it back; logs any that
    /// read differently. The number that did.
    fn put_healthy(regs: &mut Mmio, names: &[&str]) -> u32 {
        let mut off_by = 0;
        for name in names {
            let Some(&(_, off, v)) = HEALTHY.iter().find(|r| r.0 == *name) else {
                log!("  {name}: not in the healthy snapshot");
                continue;
            };
            regs.write32(off, v);
            rt::delay_us(1_000);
            let got = regs.read32(off);
            if got != v {
                off_by += 1;
                log!("  {name}: wrote 0x{v:x}, reads 0x{got:x}");
            }
        }
        off_by
    }

    /// Blinks the backlight `n` times (off 0.4 s, back 0.4 s), then 1.5 s
    /// lit: the owner counts the last group to name the phase a freeze
    /// came in. The PXA's PWM0 duty register (`0x4`), put back as found.
    fn blink(pwm: &mut Option<Mmio>, n: u32) {
        let Some(p) = pwm else { return };
        let duty = p.read32(0x4);
        let wait = |ms: u32| {
            for _ in 0..ms / 50 {
                rt::delay_us(50_000);
                rt::check_in();
            }
        };
        for _ in 0..n {
            p.write32(0x4, 0);
            wait(400);
            p.write32(0x4, duty);
            wait(400);
        }
        wait(1_500);
    }

    /// Run Z (see [`REBUILD`]).
    fn rebuild(regs: Mmio, vram: &mut [u8]) -> ! {
        let mut regs = regs;
        // The backlight (the PXA's PWM0, not the 2700G) blinks the phase
        // number before each phase: run Z's second try froze after the
        // reset and neither the SDRAM log nor the SRAM copy survived the
        // watchdog reset (Dell's boot clears both), so the owner counts the
        // last group of blinks instead.
        let mut pwm = rt::take_pwm0();
        let (f, s0, s1) = underrun_state(&mut regs);
        log!("Z: as found: DINTRS 0x{f:x}, DSIG 0x{s0:x} then 0x{s1:x}");
        rebuild_chip(&mut regs, vram, &mut pwm);
        dump_all(&mut regs, "Z7: every register after the rebuild");
        // The cold state (what a flash boot finds) is left for a later run,
        // once a rebuild is proven: reading it froze the first try.
        log!("{MARKER}: done (run Z)");
        back_to_loader();
        loop {
            core::hint::spin_loop();
        }
    }

    /// Run Z's reset and rebuild (Z1-Z6), the card drawn on the plane
    /// after; the backlight blinks each phase's number first. Proven on the
    /// X51v (runs Z4, Z5): the display sends again. The local memory's
    /// contents are lost (the picture, textures, the fence word).
    fn rebuild_chip(regs: &mut Mmio, vram: &mut [u8], pwm: &mut Option<Mmio>) {
        // Z1: the reset. Run Z's first try read registers at once after it
        // (to log the cold state) and froze (the watchdog reset): Linux
        // mbxfb only writes, 1 ms apart, until the clocks and the memory
        // run. So here: writes only, in its order, until Z3's end; each
        // phase noted in the kept log first (a freeze shows where).
        blink(pwm, 1);
        log!("Z1: SYSRST next (writes only from here to Z3)");
        regs.write32(0x010, 1);
        rt::delay_us(10_000);
        // Linux mbxfb's next write after the reset: "setup a timeout, raise
        // drive strength". Without a timeout, an access the reset 2700G
        // never answers holds the bus (RDY) for good: the suspect for the
        // freeze. Kept to the end (Z4 doesn't put WM's 0 back).
        blink(pwm, 2);
        log!("Z1b: SYSCFG 0xffffff0c (Linux's timeout and drive strength) next");
        regs.write32(0x000, 0xFFFF_FF0C);
        rt::delay_us(1_000);
        blink(pwm, 3);
        log!("Z2: the clocks next");
        for name in ["SYSCLKSRC", "PIXCLKSRC", "CLKSLEEP", "COREPLL", "DISPPLL"] {
            write_healthy(regs, name);
        }
        rt::delay_us(10_000);
        for name in [
            "VOVRCLK",
            "PIXCLK",
            "MEMCLK",
            "M24CLK",
            "MBXCLK",
            "SDCLK",
            "PIXCLKDIV",
        ] {
            write_healthy(regs, name);
        }
        blink(pwm, 4);
        log!("Z3: the local memory next (Linux's setup_memc order)");
        for name in ["LMCFG", "LMPWR", "LMTIM", "LMREFRESH", "LMTYPE", "LMPWR"] {
            write_healthy(regs, name);
        }
        blink(pwm, 5);
        log!("Z3: first reads after the reset next (PLLSTAT)");
        log!(
            "Z3: PLLSTAT 0x{:x}, LMPWRSTAT 0x{:x}",
            regs.read32(0x040),
            regs.read32(0x100C)
        );
        for k in 0..16 {
            let _ = load32_volatile(vram, 4 * k);
        }
        let mut off_by = put_healthy(
            regs,
            &[
                "SYSCLKSRC",
                "PIXCLKSRC",
                "CLKSLEEP",
                "COREPLL",
                "DISPPLL",
                "VOVRCLK",
                "PIXCLK",
                "MEMCLK",
                "M24CLK",
                "MBXCLK",
                "SDCLK",
                "PIXCLKDIV",
                "LMCFG",
                "LMPWR",
                "LMTIM",
                "LMREFRESH",
                "LMTYPE",
                "LMCEMR",
                "LMPROTMIN",
                "LMPROTMAX",
                "LMPROTCFG",
            ],
        );
        log!("Z3: the clocks and memory read back: {off_by} differ");
        blink(pwm, 6);
        log!("Z4: the system block next");
        // Z4: the system block's other settings (SYSCFG stays Linux's).
        off_by += put_healthy(
            regs,
            &[
                "PFBASE",
                "PFCEIL",
                "NINTPW",
                "MINTENABLE",
                "SINTENABLE",
                "LCD_CONFIG",
                "ODFBPWR",
                "GPIOCGF",
                "GPIOHI",
                "GPIOLO",
                "PWMCFG",
                "PWM0DIV",
                "PWM0DUTY",
                "PWM0PER",
                "PWM1DIV",
                "PWM1DUTY",
                "PWM1PER",
            ],
        );
        blink(pwm, 7);
        log!("Z5: the display next");
        // Z5: the display, every setting but the status and counters; the
        // sync generator last. DHDET and DVDET are the display-enable
        // window (run Z's third try skipped them as status: they read 0
        // after the reset and nothing was drawn; healthy 0x500230 and
        // 0x30283, the panel's 80-560 x 3-643).
        let skip = ["DINTRS", "DSIG", "DVLNUM", "DLSTS", "DSCTRL"];
        let mut display = [""; 128];
        let mut nd = 0;
        for &(name, off, _) in &HEALTHY {
            if (0x2000..0x3000).contains(&off) && !skip.contains(&name) && nd < display.len() {
                display[nd] = name;
                nd += 1;
            }
        }
        off_by += put_healthy(regs, &display[..nd]);
        let ds = healthy("DSCTRL");
        regs.write32(dreg::DSCTRL, ds & !(1 << 31));
        rt::delay_us(1_000);
        regs.write32(dreg::DSCTRL, ds);
        rt::delay_us(40_000);
        log!("Z5: the display: {nd} registers; {off_by} writes in all read back differently");
        let _ = underrun_take(regs);
        let (f, s0, s1) = underrun_state(regs);
        log!(
            "Z6: DINTRS 0x{f:x}, DSIG 0x{s0:x} then 0x{s1:x} (sending: {})",
            s0 | s1 != 0
        );
        if let Some(p) = Plane::decode(
            regs.read32(dreg::GSCTRL),
            regs.read32(dreg::GSCADR),
            regs.read32(dreg::GSADR),
            vram.len(),
        ) {
            blink(pwm, 8);
            card_text(vram, &p, "Z: BACK", 0xFF10_6020, 6);
            let (f, s0, s1) = underrun_state(regs);
            log!("Z6: after the card: DINTRS 0x{f:x}, DSIG 0x{s0:x} then 0x{s1:x}");
        }
    }

    // ---- Run Q: what ends the slow state (2026-10-20) ------------------------

    /// Windows Mobile's clock checked twice, logged under `when`; the 3D
    /// render time of the second (the first after a change warms caches).
    fn slow_measure(g: &mut Rig, c: &mut ClockRig, seed: &mut u32, when: &str) -> u32 {
        let mut r3d = 0;
        for _ in 0..2 {
            let r = clock_check(g, c, false, *seed);
            *seed += 2;
            log!("  Q {when}:");
            clock_log(WM_STEP, Some(0), &r);
            r3d = r.render_us;
        }
        r3d
    }

    /// Run Q (see [`SLOW`]).
    #[inline(never)]
    fn slow_state(g: &mut Rig) {
        let found = (g.e.regs.read32(clock::COREPLL), diag::msc1());
        if found.0 != ClockSpeed::WM.pll || found.1 != 0x2484_2481 {
            log!("Q: not Windows Mobile's clock and bus timing: nothing changed");
            return;
        }
        let Some(mut c) = clock_rig(g) else {
            log!("Q: no memory for the set-up: stopped");
            return;
        };
        let base = clock_check(g, &mut c, true, 2);
        clock_log(WM_STEP, Some(0), &base);
        if !base.good() {
            log!("Q: the reference itself is BAD: stopped");
            clock_rig_free(g, c);
            return;
        }
        let mut seed = 4;
        clock_card(
            g,
            ["RUN Q: SLOW STATE", "MEASURING", "STEP 1 OF 4"],
            0xFF20_2040,
            2,
        );
        let before = slow_measure(g, &mut c, &mut seed, "before any step");
        let steps = [
            (
                ClockStep {
                    m: 25,
                    cl3: Some(false),
                    ..WM_STEP
                },
                "after 108.33 MHz CL2",
                "108.33 MHZ CL2",
            ),
            (
                ClockStep {
                    restart: true,
                    ..WM_STEP
                },
                "after the memory restarted at Windows Mobile's clock",
                "MEMORY RESTART",
            ),
            (
                ClockStep {
                    m: 25,
                    cl3: Some(true),
                    restart: true,
                    ..WM_STEP
                },
                "after G4c's 108.33 MHz CL3 with the restart",
                "108.33 MHZ CL3",
            ),
        ];
        let mut after = [0u32; 3];
        for (k, (s, when, label)) in steps.into_iter().enumerate() {
            let mut l3 = heapless_name();
            let _ = write!(l3, "STEP {} OF 4", k + 2);
            clock_card(g, ["RUN Q: SLOW STATE", label, l3.as_str()], 0xFF20_2040, 2);
            let lock = clock_apply(g, s);
            clock_readback(g);
            let r = if lock.is_some() {
                clock_check(g, &mut c, false, seed)
            } else {
                ClockResult::default()
            };
            seed += 2;
            clock_log(s, lock, &r);
            clock_restore(&mut g.e.regs);
            rt::delay_us(10_000);
            after[k] = slow_measure(g, &mut c, &mut seed, when);
        }
        log!(
            "Q: the 3D render at Windows Mobile's clock (cores / 2; slow 37.9 ms, normal 34.3 ms): before {} us; after 108.33 CL2 {} us; after the memory restart {} us; after 108.33 CL3 with the restart {} us",
            before,
            after[0],
            after[1],
            after[2]
        );
        let ok = clock_control(g, &mut c, seed, "at the end");
        clock_card(
            g,
            [
                "RUN Q DONE",
                if ok { "BACK TO NORMAL" } else { "CONTROL BAD" },
                "LOG SENT",
            ],
            if ok { 0xFF10_6020 } else { 0xFF80_2010 },
            3,
        );
        clock_rig_free(g, c);
    }

    // ---- Run N: the start-up watched (2026-10-20) ---------------------------

    /// The display as it is now, logged under `stage`: the flags, the output
    /// signature now and 20 ms on (a frame is 17.7 ms: a display that sends
    /// a picture that isn't all black has a nonzero signature that holds
    /// still while the picture does), the graphics plane's registers and
    /// the sync generator.
    fn start_watch(regs: &mut Mmio, stage: &str) {
        let (f, s0, s1) = underrun_state(regs);
        log!(
            "{stage}: DINTRS 0x{f:x}, DSIG 0x{s0:x} then 0x{s1:x}; GSCTRL 0x{:x} GSCADR 0x{:x} GSADR 0x{:x} DSCTRL 0x{:x}; MBXCLK 0x{:x}",
            regs.read32(dreg::GSCTRL),
            regs.read32(dreg::GSCADR),
            regs.read32(dreg::GSADR),
            regs.read32(dreg::DSCTRL),
            regs.read32(MBXCLK),
        );
    }

    /// Run N's second half: cards and G5's opening (its memory set-up and
    /// the reference check at Windows Mobile's clock), the display read
    /// after each; nothing past it (no clock is changed).
    #[inline(never)]
    fn start_opening(g: &mut Rig) {
        // A card drawn by the CPU into the plane on screen: if this one is
        // dark, the display isn't sending, whatever draws it.
        if let Some(p) = Plane::decode(
            g.e.regs.read32(dreg::GSCTRL),
            g.e.regs.read32(dreg::GSCADR),
            g.e.regs.read32(dreg::GSADR),
            g.mem.0.len(),
        ) {
            card_text(g.mem.0, &p, "N CARD 1", 0xFF10_6020, 4);
        }
        start_watch(&mut g.e.regs, "N7 card 1 (the CPU's, green) shown 4 s");
        clock_card(
            g,
            ["N CARD 2", "THE 2D ENGINE", "AS G5 DRAWS"],
            0xFF20_2040,
            4,
        );
        start_watch(&mut g.e.regs, "N8 card 2 (the 2D engine's, blue) shown 4 s");
        let Some(mut c) = clock_rig(g) else {
            log!("N: no memory for G5's set-up: stopped");
            return;
        };
        start_watch(&mut g.e.regs, "N9 G5's memory set-up done");
        let base = clock_check(g, &mut c, true, 2);
        clock_log(WM_STEP, Some(0), &base);
        start_watch(
            &mut g.e.regs,
            "N10 G5's reference check at Windows Mobile's clock done",
        );
        clock_card(
            g,
            ["N CARD 3", "AFTER THE CHECK", "TELL ME"],
            0xFF60_2010,
            4,
        );
        start_watch(&mut g.e.regs, "N11 card 3 (the 2D engine's, red) shown 4 s");
        // G5's next part, its first clock changes: the lock traces (99.67 ->
        // 74.75 -> 99.67 -> 108.33 -> 99.67 MHz). After each a card drawn by
        // the CPU (never black: run N found an all-black picture reads `DSIG`
        // 0), then the display read: a stopped display reads 0 under it.
        if base.good() {
            let a75 = ClockStep { n: 4, ..WM_STEP }.speed();
            let f108 = ClockStep { m: 25, ..WM_STEP }.speed();
            let traces = [
                (a75, "99.67 -> 74.75 MHz", "N TRACE 1"),
                (ClockSpeed::WM, "74.75 -> 99.67 MHz", "N TRACE 2"),
                (f108, "99.67 -> 108.33 MHz", "N TRACE 3"),
                (ClockSpeed::WM, "108.33 -> 99.67 MHz", "N TRACE 4"),
            ];
            for (k, (speed, what, label)) in traces.into_iter().enumerate() {
                lock_trace(g, speed, what);
                if let Some(p) = Plane::decode(
                    g.e.regs.read32(dreg::GSCTRL),
                    g.e.regs.read32(dreg::GSCADR),
                    g.e.regs.read32(dreg::GSADR),
                    g.mem.0.len(),
                ) {
                    card_text(g.mem.0, &p, label, 0xFF60_6010, 2);
                }
                let mut stage = heapless_name();
                let _ = write!(stage, "N{} after the lock trace {what}, a CPU card", 12 + k);
                start_watch(&mut g.e.regs, stage.as_str());
            }
        }
        clock_rig_free(g, c);
        log!(
            "N: done; the owner reports which cards showed (1 green, 2 blue, 3 red, traces yellow)"
        );
    }

    /// Run H (see [`FREEZE`]).
    #[inline(never)]
    fn freeze(g: &mut Rig) {
        /// The system block's `0x12C`: no register (the MBX core's status
        /// is at `0x412C`); reading it froze the bus under Windows Mobile.
        const NOTHING: usize = 0x12C;
        log!(
            "H: the card log against a frozen bus: 40 numbered lines, then a read of 0x{NOTHING:x} the 2700G never answers; the hardware watchdog resets the device; every line must be on the card"
        );
        clock_card(
            g,
            ["RUN H: FREEZE TEST", "SCREEN FREEZES", "THEN IT RESETS"],
            0xFF80_2010,
            3,
        );
        for k in 1..=40 {
            log!("H: line {k} of 40 before the freeze");
            rt::delay_us(50_000);
            rt::check_in();
        }
        log!("H: reading 0x{NOTHING:x} next (it froze Windows Mobile's bus)");
        let v = g.e.regs.read32(NOTHING);
        // Run H1 on the X51v (2026-10-20): it read 0 under the loader, no
        // freeze. So the reset that clears the SDRAM and SRAM logs is
        // brought on directly: the PXA's hardware watchdog (OS timer match
        // 3), the reset a frozen bus ends in, into Dell's prompt.
        log!("H: the read returned 0x{v:x}: no freeze");
        log!(
            "H: the hardware watchdog fires in 1 s (nothing feeds it from here); the device resets into Dell's prompt"
        );
        if FIRE_WATCHDOG {
            rt::watchdog().feed(3_250_000);
            loop {
                core::hint::spin_loop();
            }
        }
        log!("H: the watchdog left unarmed (an emulator run: RP_GPUBENCH_NO_WATCHDOG)");
    }

    /// Run Y (see [`RESCUE`]).
    #[inline(never)]
    fn rescue(g: &mut Rig) {
        const SYSCFG: usize = 0x000;
        const LMPWR: usize = 0x1008;
        const LMPWRSTAT: usize = 0x100C;
        const LMTYPE: usize = 0x1014;
        /// `LMTYPE`'s CAS latency field (bits 12-10).
        const CL_BITS: u32 = 7 << 10;
        log!(
            "Y: the rescue measured: Windows Mobile's clock checked before run Z's rebuild (Y0), after it (Y1), after each memory restart (Y2-Y4), and with SYSCFG put back (Y5)"
        );
        let found = (g.e.regs.read32(clock::COREPLL), diag::msc1());
        if found.0 != ClockSpeed::WM.pll || found.1 != 0x2484_2481 {
            log!(
                "  not Windows Mobile's clock and bus timing (COREPLL 0x{:x}, MSC1 0x{:08x}): nothing changed",
                found.0,
                found.1
            );
            return;
        }
        let Some(mut c) = clock_rig(g) else {
            return;
        };
        clock_card(
            g,
            ["RUN Y: RESCUE", "WHITE, THEN BACK", "STEP 1 OF 5"],
            0xFF20_2040,
            3,
        );
        let base = clock_check(g, &mut c, true, 2);
        rescue_log(g, "Y0: before the rebuild (the reference)", &base, &base);
        if !base.good() {
            log!("  the reference itself is BAD: stopped");
            clock_rig_free(g, c);
            return;
        }
        let mut pwm = rt::take_pwm0();
        rebuild_chip(&mut g.e.regs, g.mem.0, &mut pwm);
        // The reset lost the local memory: the 2D engine's window starts
        // again, the fence word is unknown, the texture is uploaded again;
        // the plane goes back to the check's picture.
        g.e.after_core_reset();
        let _ = store32_volatile(g.mem.0, map::FENCE.at as usize, 0);
        let mut buf = [0u32; 64];
        for (k, chunk) in c.tex_copy.chunks(64).enumerate() {
            buf[..chunk.len()].copy_from_slice(chunk);
            g.mem
                .store(c.tex.at as usize + 256 * k, &buf[..chunk.len()]);
        }
        g.mem.sync();
        let _ = show_screen(&mut g.e.regs);
        let r = clock_check(g, &mut c, false, 4);
        rescue_log(g, "Y1: after the rebuild", &r, &base);
        // The restarts, each measured; each stays done for the next.
        let rungs: [(&str, [&str; 3]); 4] = [
            (
                "Y2: LMPWR written active and 8 dummy reads (G4's restart, alone)",
                ["Y: STEP 2 OF 5", "MEMORY RESTART", "TESTING"],
            ),
            (
                "Y3: LMTYPE to CL3 and back to CL2, then LMPWR active and dummy reads (G4's first restarted step and the restore after it, at one clock)",
                ["Y: STEP 3 OF 5", "MEMORY MODE", "TESTING"],
            ),
            (
                "Y4: the local memory to self-refresh and back (run L's rung 4)",
                ["Y: STEP 4 OF 5", "SELF-REFRESH", "TESTING"],
            ),
            (
                "Y5: SYSCFG back to Windows Mobile's 0 (Linux's timeout was needed only after the reset)",
                ["Y: STEP 5 OF 5", "SYSCFG BACK", "TESTING"],
            ),
        ];
        let restart = |g: &mut Rig| {
            g.e.regs.write32(LMPWR, 0);
            rt::delay_us(100);
            for k in 0..8 {
                let _ = g.mem.load(4 * k);
            }
        };
        for (k, (what, card)) in rungs.into_iter().enumerate() {
            clock_card(g, card, 0xFF20_2040, 2);
            log!("{what}");
            match k {
                0 => restart(g),
                1 => {
                    let t = g.e.regs.read32(LMTYPE);
                    g.e.regs.write32(LMTYPE, (t & !CL_BITS) | 3 << 10);
                    rt::delay_us(1_000);
                    g.e.regs.write32(LMTYPE, t);
                    rt::delay_us(1_000);
                    restart(g);
                    log!("  LMTYPE 0x{:x} (was 0x{t:x})", g.e.regs.read32(LMTYPE));
                }
                2 => {
                    g.e.regs.write32(LMPWR, 1);
                    let sr = wait_for(&mut g.e.regs, LMPWRSTAT, 3, 1, 100);
                    rt::delay_us(1_000);
                    g.e.regs.write32(LMPWR, 0);
                    let act = wait_for(&mut g.e.regs, LMPWRSTAT, 3, 0, 100);
                    log!("  self-refresh entered: {sr}, active again: {act}");
                }
                _ => {
                    g.e.regs.write32(SYSCFG, 0);
                    rt::delay_us(1_000);
                }
            }
            let r = clock_check(g, &mut c, false, 6 + 2 * k as u32);
            rescue_log(g, what.split(':').next().unwrap_or(what), &r, &base);
        }
        clock_card(
            g,
            ["RUN Y DONE", "BACK TO NORMAL", "LOG SENT"],
            0xFF10_6020,
            3,
        );
        clock_rig_free(g, c);
    }

    /// One run Y check: its speed against the reference (100 = as fast;
    /// the bus read, the 2D fill, the 3D render), the memory controller's
    /// state, then the clock check's own line.
    fn rescue_log(g: &mut Rig, what: &str, r: &ClockResult, base: &ClockResult) {
        let pct = |now: u32, then: u32| then * 100 / now.max(1);
        log!(
            "{what}: speed against Y0 (100 = as fast): read {}, 2D {}, 3D {}; SYSCFG 0x{:x}, LMPWRSTAT 0x{:x}, LMTYPE 0x{:x}{}",
            pct(r.read_us, base.read_us),
            pct(r.fill_us, base.fill_us),
            pct(r.render_us, base.render_us),
            g.e.regs.read32(0x000),
            g.e.regs.read32(0x100C),
            g.e.regs.read32(0x1014),
            if r.tex_damaged != 0 {
                " (the texture was damaged: repaired)"
            } else {
                ""
            }
        );
        clock_log(WM_STEP, Some(0), r);
    }

    /// Run L (see [`LADDER`]).
    fn ladder(regs: Mmio, vram: &mut [u8]) -> ! {
        const PIXCLK: usize = 0x048;
        const PIXCLKSRC: usize = 0x030;
        const LMPWRSTAT: usize = 0x100C;
        const ODFBPWR: usize = 0x064;
        const ODFBSTAT: usize = 0x068;
        const MEMCLK: usize = 0x04C;
        let mut regs = regs;
        let (f, s0, s1) = underrun_state(&mut regs);
        log!("L: as found: DINTRS 0x{f:x}, DSIG 0x{s0:x} then 0x{s1:x}");
        let plane = Plane::decode(
            regs.read32(dreg::GSCTRL),
            regs.read32(dreg::GSCADR),
            regs.read32(dreg::GSADR),
            vram.len(),
        );
        let rungs: [(&str, &str); 6] = [
            ("L1: the pixel clock off and on (PIXCLK)", "L1: PIXCLK"),
            (
                "L2: the pixel clock's source to the reference and back (PIXCLKSRC)",
                "L2: PIXSRC",
            ),
            (
                "L3: the display PLL off and on (DISPPLL bit 0)",
                "L3: DISPPLL",
            ),
            (
                "L4: the local memory to self-refresh and back (LMPWR, as Linux suspends it)",
                "L4: SELFREF",
            ),
            (
                "L5: the on-die frame buffer to sleep and back (ODFBPWR)",
                "L5: ODFB",
            ),
            ("L6: the memory clock off for 1 ms (MEMCLK)", "L6: MEMCLK"),
        ];
        let mut recovered = None;
        for (k, (what, card)) in rungs.into_iter().enumerate() {
            match k {
                0 => {
                    regs.write32(PIXCLK, 0);
                    rt::delay_us(1_000);
                    regs.write32(PIXCLK, 1);
                }
                1 => {
                    let v = regs.read32(PIXCLKSRC);
                    regs.write32(PIXCLKSRC, 0);
                    rt::delay_us(1_000);
                    regs.write32(PIXCLKSRC, v);
                }
                2 => {
                    let v = regs.read32(dreg::DISPPLL);
                    regs.write32(dreg::DISPPLL, v & !1);
                    rt::delay_us(1_000);
                    regs.write32(dreg::DISPPLL, v | 1);
                    let locked = wait_for(&mut regs, dreg::PLLSTAT, 1, 1, 50);
                    log!("  the display PLL locked again: {locked}");
                }
                3 => {
                    regs.write32(0x1008, 1);
                    let sr = wait_for(&mut regs, LMPWRSTAT, 3, 1, 100);
                    rt::delay_us(1_000);
                    regs.write32(0x1008, 0);
                    let act = wait_for(&mut regs, LMPWRSTAT, 3, 0, 100);
                    log!("  self-refresh entered: {sr}, active again: {act}");
                }
                4 => {
                    let v = regs.read32(ODFBPWR);
                    regs.write32(ODFBPWR, (v & !3) | 2);
                    let sl = wait_for(&mut regs, ODFBSTAT, 7, 2, 100);
                    rt::delay_us(1_000);
                    regs.write32(ODFBPWR, v);
                    let act = wait_for(&mut regs, ODFBSTAT, 7, 4, 100);
                    log!(
                        "  ODFBPWR 0x{v:x}: asleep {sl}, active again {act} (ODFBSTAT 0x{:x})",
                        regs.read32(ODFBSTAT)
                    );
                }
                _ => {
                    regs.write32(MEMCLK, 0);
                    rt::delay_us(1_000);
                    regs.write32(MEMCLK, 1);
                }
            }
            rt::delay_us(1_000);
            let _ = underrun_take(&mut regs);
            rt::delay_us(40_000);
            let (f, s0, s1) = underrun_state(&mut regs);
            let sending = s0 | s1 != 0;
            log!("{what}: DINTRS 0x{f:x}, DSIG 0x{s0:x} then 0x{s1:x} (sending: {sending})");
            if let Some(p) = &plane {
                card_text(vram, p, card, 0xFF10_6020, 4);
            }
            if sending {
                recovered = Some(k + 1);
                break;
            }
        }
        match recovered {
            Some(n) => log!("L: the display sends again after rung {n}"),
            None => log!("L: no rung brought the display back"),
        }
        log!("{MARKER}: done (run L)");
        back_to_loader();
        loop {
            core::hint::spin_loop();
        }
    }

    /// Run U (see [`UNDERRUN`]).
    fn underrun(regs: Mmio, vram: &mut [u8]) -> ! {
        let mut d = Display::new(regs);
        let (plane, _) = d.adopt(vram.len());
        let mut regs = d.into_inner();
        let (f, s0, s1) = underrun_state(&mut regs);
        log!("U: as found: DINTRS 0x{f:x}, DSIG 0x{s0:x} then 0x{s1:x}");
        card_text(vram, &plane, "U: START", 0xFF20_2080, 4);
        for (k, ms) in [1u32, 3, 10, 30].into_iter().enumerate() {
            let n = k + 1;
            let mut l = heapless_name();
            let _ = write!(l, "U{n}: {ms} MS");
            card_text(vram, &plane, l.as_str(), 0xFF10_6020, 3);
            regs.write32(clock::SYSCLKSRC, 0);
            rt::delay_us(ms * 1_000);
            regs.write32(clock::SYSCLKSRC, 2);
            rt::delay_us(20_000);
            let (f, s0, s1) = underrun_state(&mut regs);
            let dark = f & GFX_UNDERRUN != 0;
            log!(
                "U{n}: {ms} ms on the reference clock: DINTRS 0x{f:x} (underrun {dark}), DSIG 0x{s0:x} then 0x{s1:x}"
            );
            // Held 4 s as it is: the owner sees whether it went dark.
            for _ in 0..40 {
                rt::delay_us(100_000);
                rt::check_in();
            }
            if dark {
                let v = regs.read32(DINTRS);
                regs.write32(DINTRS, v & !GFX_UNDERRUN);
                rt::delay_us(40_000);
                let (f, s0, s1) = underrun_state(&mut regs);
                log!(
                    "U{n}: the flag cleared (Dell's way): DINTRS 0x{f:x}, DSIG 0x{s0:x} then 0x{s1:x}"
                );
            }
            let mut l = heapless_name();
            let _ = write!(l, "U{n} BACK");
            card_text(vram, &plane, l.as_str(), 0xFF80_6010, 4);
        }
        // U5-U6 (run U2, after U1 on the device: 30 ms on the reference
        // clock alone set no underrun): what else G4's lock traces did.
        // U5: CL3 at Windows Mobile's clock for 100 ms (written the
        // checked way): CL3 breaks the 3D core's reads (G4), maybe the
        // display's too. U6: the lock traces as G4 ran them.
        card_text(vram, &plane, "U5: CL3", 0xFF10_6020, 3);
        let wm = ClockSpeed::WM;
        let _ = clock::set_speed(
            &mut regs,
            ClockSpeed {
                lmtype: 0xEC8,
                ..wm
            },
            rt::delay_us,
        );
        rt::delay_us(100_000);
        let (f, s0, s1) = underrun_state(&mut regs);
        let _ = clock::set_speed(&mut regs, wm, rt::delay_us);
        log!(
            "U5: CL3 at 99.67 MHz for 100 ms: DINTRS 0x{f:x} (underrun {}), DSIG 0x{s0:x} then 0x{s1:x}; LMTYPE now 0x{:x}",
            f & GFX_UNDERRUN != 0,
            regs.read32(clock::LMTYPE)
        );
        underrun_hold_and_clear(&mut regs, 5);
        card_text(vram, &plane, "U5 BACK", 0xFF80_6010, 4);
        card_text(vram, &plane, "U6: TRACES", 0xFF10_6020, 3);
        let a75 = ClockStep { n: 4, ..WM_STEP }.speed();
        let f108 = ClockStep { m: 25, ..WM_STEP }.speed();
        for to in [a75, wm, f108, wm] {
            regs.write32(clock::SYSCLKSRC, 0);
            rt::delay_us(250);
            regs.write32(clock::LMTIM, to.lmtim);
            regs.write32(clock::LMREFRESH, to.lmrefresh);
            regs.write32(clock::LMTYPE, to.lmtype);
            regs.write32(clock::COREPLL, to.pll);
            rt::delay_us(4_000);
            regs.write32(clock::SYSCLKSRC, 2);
            rt::delay_us(250);
        }
        rt::delay_us(20_000);
        let (f, s0, s1) = underrun_state(&mut regs);
        log!(
            "U6: G4's lock traces: DINTRS 0x{f:x} (underrun {}), DSIG 0x{s0:x} then 0x{s1:x}; LMTYPE 0x{:x} LMTIM 0x{:x} LMREFRESH 0x{:x} COREPLL 0x{:x}",
            f & GFX_UNDERRUN != 0,
            regs.read32(clock::LMTYPE),
            regs.read32(clock::LMTIM),
            regs.read32(clock::LMREFRESH),
            regs.read32(clock::COREPLL)
        );
        let _ = clock::set_speed(&mut regs, wm, rt::delay_us);
        underrun_hold_and_clear(&mut regs, 6);
        card_text(vram, &plane, "U6 BACK", 0xFF80_6010, 4);
        let (f, s0, s1) = underrun_state(&mut regs);
        log!("U: at the end: DINTRS 0x{f:x}, DSIG 0x{s0:x} then 0x{s1:x}");
        card_text(vram, &plane, "U: DONE", 0xFF20_2080, 3);
        log!("{MARKER}: done (run U)");
        back_to_loader();
        loop {
            core::hint::spin_loop();
        }
    }

    // ---- Run R: the dark display (2026-10-20, after run G4) ---------------

    /// Every register Dell's driver saves, the PLLs and the system clock's
    /// source, and the display's line counter sampled 8 times 0.5 ms apart
    /// (moving: the sync generator runs).
    fn revive_dump(regs: &mut Mmio, when: &str) {
        log!("R: {when}");
        let extra: [(&str, usize); 5] = [
            ("SYSCLKSRC", 0x02C),
            ("COREPLL", 0x038),
            ("DISPPLL", 0x03C),
            ("PLLSTAT", 0x040),
            ("DINTRS", 0x2178),
        ];
        let all = rp_drv_mbx_display::SAVED_REGS.iter().chain(extra.iter());
        let mut line = heapless_name();
        for (k, (name, off)) in all.enumerate() {
            let _ = write!(line, " {name} 0x{:x}", regs.read32(*off));
            if k % 4 == 3 {
                log!(" {}", line.as_str());
                line = heapless_name();
            }
        }
        if line.n != 0 {
            log!(" {}", line.as_str());
        }
        revive_lines(regs);
    }

    fn revive_lines(regs: &mut Mmio) {
        let mut line = heapless_name();
        for _ in 0..8 {
            let _ = write!(line, " {}", regs.read32(dreg::DVLNUM));
            rt::delay_us(500);
        }
        log!(
            "  line counter every 0.5 ms:{}; DSCTRL 0x{:x}, DINTRS 0x{:x}",
            line.as_str(),
            regs.read32(dreg::DSCTRL),
            regs.read32(0x2178)
        );
    }

    /// A full-screen card in `plane`: `bg`, and "REVIVE n" in large white
    /// letters, drawn by the CPU (the 2D engine isn't trusted here); held
    /// 8 s.
    fn revive_card(vram: &mut [u8], plane: &Plane, n: u32, bg: u32) {
        let mut label = heapless_name();
        let _ = write!(label, "REVIVE {n}");
        card_text(vram, plane, label.as_str(), bg, 8);
    }

    /// A full-screen card in `plane` drawn by the CPU: `bg`, and `text` in
    /// large white letters; held `secs` seconds.
    fn card_text(vram: &mut [u8], plane: &Plane, text: &str, bg: u32, secs: u32) {
        let bpp = if plane.format == PlaneFormat::Rgb565 {
            2
        } else {
            4
        };
        let px = |c: u32| -> u32 {
            if bpp == 2 {
                let v = ((c >> 8) & 0xF800) | ((c >> 5) & 0x07E0) | ((c >> 3) & 0x001F);
                v | v << 16
            } else {
                c
            }
        };
        let scale = if text.len() > 9 { 4 } else { 8 };
        for y in 0..u32::from(plane.height) {
            for x in (0..u32::from(plane.width)).step_by(4 / bpp) {
                let lit = in_text(x, y, text, 24, 280, scale);
                let at = plane.offset + y as usize * plane.stride + x as usize * bpp;
                let _ = store32_volatile(vram, at, px(if lit { 0xFFFF_FFFF } else { bg }));
            }
        }
        cpu::drain_write_buffer();
        for _ in 0..secs * 10 {
            rt::delay_us(100_000);
            rt::check_in();
        }
    }

    /// Run R: the registers as found (before anything is touched), then
    /// one restart step at a time, each followed by a card the owner
    /// watches for (the first number seen names the step that worked):
    /// 0 nothing (only the adopt every image does), 1 the sync generator
    /// off and on, 2 the display PLL written again, 3 the whole display
    /// set-up again, 4 the system clock and memory timing put back
    /// (checked) and the display set up again.
    fn revive(regs: Mmio, vram: &mut [u8]) -> ! {
        let mut regs = regs;
        revive_dump(&mut regs, "as found, before anything is touched");
        let mut d = Display::new(regs);
        let (plane, kept) = d.adopt(vram.len());
        let mut regs = d.into_inner();
        log!(
            "R0: adopted (plane kept: {kept}; offset 0x{:x}, stride {}, {:?})",
            plane.offset,
            plane.stride,
            plane.format
        );
        revive_lines(&mut regs);
        revive_card(vram, &plane, 0, 0xFF80_1010);

        log!("R1: the sync generator off and on");
        let v = regs.read32(dreg::DSCTRL);
        regs.write32(dreg::DSCTRL, v & !(1 << 31));
        rt::delay_us(20_000);
        regs.write32(dreg::DSCTRL, v | 1 << 31);
        revive_lines(&mut regs);
        revive_card(vram, &plane, 1, 0xFF10_6020);

        log!("R2: the display PLL written again (60 Hz)");
        regs.write32(dreg::DISPPLL, rp_drv_mbx_display::pll::SIXTY);
        rt::delay_us(1_000);
        revive_lines(&mut regs);
        revive_card(vram, &plane, 2, 0xFF20_2080);

        log!("R3: the whole display set-up again");
        let mut d = Display::new(regs);
        d.enable(&rp_drv_mbx_display::X51V_VGA, &plane);
        let mut regs = d.into_inner();
        revive_lines(&mut regs);
        revive_card(vram, &plane, 3, 0xFF80_6010);

        log!("R4: the system clock and memory timing put back (checked), the display set up again");
        let a = clock::set_speed(&mut regs, ClockSpeed::WM, rt::delay_us);
        log!("  set_speed: {a:?}");
        let mut d = Display::new(regs);
        d.enable(&rp_drv_mbx_display::X51V_VGA, &plane);
        let mut regs = d.into_inner();
        revive_dump(&mut regs, "after R4");
        revive_card(vram, &plane, 4, 0xFF60_1080);
        log!("{MARKER}: done (run R)");
        back_to_loader();
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

    /// Every clean return: the SRAM copy cleared, back to the loader (which
    /// closes the card log: its closing line marks a run that came back).
    fn back_to_loader() {
        rp_hal_x51v::devices::kept_stop();
        rp_hal_x51v::devices::return_to_loader();
    }

    fn on_fault(kind: rt::Fault, addr: u32) -> ! {
        crash(format_args!("FAULT {} at {addr:#010x}", kind.name()))
    }

    #[inline(never)]
    fn crash(msg: core::fmt::Arguments) -> ! {
        // Run G: Windows Mobile's clock and bus timing back first.
        if CLOCK && let Some((mut regs, _)) = rp_hal_x51v::devices::emergency_mbx() {
            clock_restore(&mut regs);
        }
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
