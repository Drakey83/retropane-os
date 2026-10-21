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
//!
//! **Run P** (`RP_GPUBENCH_TARGETS`, unattended; P3 of
//! docs/research/2700g-full-potential.md): the 3D core's target format
//! (`0x650`) at every code 0-15 and with each of its bits 4-31 added to
//! the known ones (bits 4-11 on code 1, 4-31 on code 6), one try each: a
//! 32 x 32 texture whose every texel names its place, drawn 1:1 into a
//! target with 4 KiB of guard memory before it and 56 KiB after (all of
//! it a known pattern first), read back and decoded (bytes a pixel,
//! channel order, rows or twiddled or tiles: a twiddled target would be
//! render-to-texture). The code goes in by a register bus that rewrites
//! the renderer's `0x650` write, so the OS's renderer has no way to set
//! one. A hung try is recovered by the renderer, and the log line is
//! written before each.
//!
//! **Run P2** (`RP_GPUBENCH_TARGETS2`, unattended; run P's follow-up):
//! P2a codes 0-7 into a 16 x 16 target at a 96-byte stride and a 32 x 16
//! one at 256 bytes (whether any shape twiddles); P2b a 64 x 64 grey ramp
//! at codes 0-15, every pixel against its level truncated to each
//! channel's bits: the dither matrix per layout (bit 3), over an 8 x 8
//! cell; P2c four strips at alpha 0, 0x55, 0xaa and 0xff drawn as they are
//! and blended, codes 0-7: what each layout stores for alpha; P2d each bit
//! of the render table's unexplained words (`0x61C`, `0x644`, `0x648`,
//! `0x64C`, `0x6D0`, `0x6D8`) flipped alone, decoded as in run P: whether
//! any bit twiddles the target (render-to-texture; Dell's code has none,
//! P19).
//!
//! **Run P2W** (`RP_GPUBENCH_TARGETS_WATCH`, the owner watches four
//! pictures, each until a button): colour bars and ramps rendered 24-bit
//! (code 4) and shown as RGB888 (display format 6), then RGB565 plain,
//! RGB565 dithered (code 9) and ARGB8888: whether the 24-bit picture shows
//! right and which ramps look smooth.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed with the verdict.
pub const MARKER: &str = "RETROPANE-GPUBENCH";

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_draw::{Cmd, Frame, Rect as DrawRect, Renderer as _};
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
    use rp_mbx_render::{Layout, Mbx2dRenderer};
    use rp_mbx3d_render::{
        Clock, Draw, LocalMem, Paint, Picture, PixelFormat, Poll, Renderer, Target, Texture,
        UV_ONE, address, factor, format, untwiddle,
    };
    use rp_vram_map::{self as map, Pool};

    use super::MARKER;

    use rp_image::crash;

    rp_image::entry!(main);

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
    /// Run Q: what moves the chip from 3D 37.9 ms at Windows Mobile's
    /// clock (cores / 2) to the 34.3 ms G4b and G4c reached (written as
    /// "what ends the slow state": the log re-read of 2026-10-21 found
    /// 37.9 ms is where every run starts, 2700g.md). Three steps of rising
    /// reach, Windows Mobile's clock measured before and after each:
    /// 108.33 MHz CL2; the memory restarted at Windows Mobile's clock; a
    /// 108.33 MHz CL3 step with the restart. None moved it.
    const SLOW: bool = option_env!("RP_GPUBENCH_SLOW").is_some();
    /// Run W: the fingerprint of the state the device is in (the hunt for
    /// what gives G4b's and G4c's faster 3D): every whitelisted PXA
    /// register, the coprocessors,
    /// every named 2700G register, then the speed at Windows Mobile's clock
    /// measured three times, the MBX core reset (as hang recovery does),
    /// and three times again; then W2, G4c's ending (65 and 74.75 MHz with
    /// the cores at divide 1, Windows Mobile's clock put back: the only
    /// way the faster state, 3D 34.3 against 37.9 ms, has come so far),
    /// measured and every 2700G register dumped again to diff. Run in each
    /// state (now, after a soft reset, after a full power-off).
    const FINGERPRINT: bool = option_env!("RP_GPUBENCH_FINGERPRINT").is_some();
    /// Run T: what the display's fetch costs (run W: the 32-bit scan-out
    /// slows 3D ~10%), the speed at Windows Mobile's clock with the plane
    /// at 32 bits, 16 bits, stopped and 32 again; then `0x644`'s
    /// orientation bits mapped pixel by pixel. Unattended.
    const FETCH: bool = option_env!("RP_GPUBENCH_FETCH").is_some();
    /// Run T3: what `0x644`'s averaging write-out costs (anti-aliasing's
    /// price): one full-screen render timed at each of its settings.
    /// Unattended.
    const RESOLVE: bool = option_env!("RP_GPUBENCH_RESOLVE").is_some();
    /// Run P2X: P2W's white specks (in memory after the render, in the
    /// black bar of the 24- and 32-bit pictures): the picture rendered 5
    /// times per format over a marker fill, the bars checked and every
    /// pixel compared between renders (the same places: logic; others:
    /// a glitch). Unattended.
    const SPECKS: bool = option_env!("RP_GPUBENCH_SPECKS").is_some();
    /// Run P2Y: what P2X's errors need: one render read back three times
    /// (the CPU's reads), then 32-bit renders with the screen at 32 bits,
    /// 16 bits and stopped (the display's fetch). Unattended.
    const SPECKS2: bool = option_env!("RP_GPUBENCH_SPECKS2").is_some();
    /// Run P2L: ADR-0023's verification (the owner's vote B, 2026-10-21;
    /// quality audit C28): P2W's picture rendered at 32 bits
    /// [`P2L_RENDERS`] times while the screen is scanned out as dithered
    /// RGB565, each checked as in P2X and against the first, between two
    /// controls of [`P2L_CONTROLS`] renders with the screen at 32 bits (P2Y
    /// found ~40-57 wrong pixels a render there: the count is only worth
    /// something if the controls find them). Unattended, ~10 minutes.
    const LONG_COUNT: bool = option_env!("RP_GPUBENCH_LONG_COUNT").is_some();
    /// Run FF (quality audit C37): the 2D FIFO full. FF1 fills it for real
    /// (300 full-screen fills queued at once: the level reads full, a busy
    /// engine isn't refused); FF2 runs the 2D renderer on the engine with
    /// the FIFO's level read as full for 150 ms while a frame is queued:
    /// the frame is dropped, counted apart from hangs, not shown, and the
    /// next one is whole. Unattended, under a minute.
    const FIFO_FULL: bool = option_env!("RP_GPUBENCH_FIFO_FULL").is_some();
    /// Run P: the 3D core's target formats (`0x650`), every code read back
    /// and decoded.
    const TARGETS: bool = option_env!("RP_GPUBENCH_TARGETS").is_some();
    /// Run P2: run P's follow-up (dither, alpha, two more target shapes).
    const TARGETS2: bool = option_env!("RP_GPUBENCH_TARGETS2").is_some();
    /// Run P2W: a rendered 24-bit picture and dithered 16 bits on screen
    /// (the owner watches).
    const TARGETS_WATCH: bool = option_env!("RP_GPUBENCH_TARGETS_WATCH").is_some();
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

    // ---- Runs P, P2, P2W: the 3D core's target formats (P3) -------------------

    /// A target the target-format runs draw into: its size in pixels and its
    /// picture's stride in bytes.
    #[derive(Clone, Copy)]
    struct PGeom {
        w: u32,
        h: u32,
        stride: u32,
    }

    impl PGeom {
        /// The bytes its rows cover at the stride.
        const fn bytes(self) -> u32 {
            self.h * self.stride
        }
    }

    /// Run P's target: 32 x 32 (4 x 2 tiles) at a 32-bit picture's stride.
    const P_SQUARE: PGeom = PGeom {
        w: 32,
        h: 32,
        stride: 128,
    };
    /// Run P2's other two: 16 x 16 (2 tiles) at a 96-byte stride (not a
    /// 32-bit row's 64), and 32 x 16 (4 tiles in a row) at 256 bytes.
    const P_SMALL: PGeom = PGeom {
        w: 16,
        h: 16,
        stride: 96,
    };
    const P_WIDE: PGeom = PGeom {
        w: 32,
        h: 16,
        stride: 256,
    };
    /// Run P2's dither ramp: 64 x 64 (8 x 4 tiles), 32-bit rows.
    const P_RAMP: PGeom = PGeom {
        w: 64,
        h: 64,
        stride: 256,
    };
    /// Guard memory before the target, and the whole area (both guards and
    /// the target), bytes.
    const P_BEFORE: u32 = 4096;
    const P_AREA: u32 = 64 * 1024;
    /// What the area holds before each try.
    const P_FILL: u32 = 0x5A5A_5A5A;

    /// The texel at `(x, y)`: red `x * 8`, green `y * 8`, blue `0xA0` (the
    /// place in each channel's top five bits, so it survives 16-bit
    /// formats; blue tells the channel order and the format's own bits).
    const fn p_texel(x: u32, y: u32) -> u32 {
        0xFF00_00A0 | (x << 3) << 16 | (y << 3) << 8
    }

    /// Run P2b's grey level at `(x, y)`: 64 + one step a 8 x 8 block (64 to
    /// 127 over the 64 x 64 target), so every place in an 8 x 8 dither
    /// matrix sees every level, and so every fraction of a 4-, 5- and 6-bit
    /// step.
    const fn p_level(x: u32, y: u32) -> u32 {
        64 + x / 8 + 8 * (y / 8)
    }

    const fn p_grey(x: u32, y: u32) -> u32 {
        let v = p_level(x, y);
        0xFF00_0000 | v << 16 | v << 8 | v
    }

    /// The renderer's register writes with `0x650` replaced by `code`, and
    /// one more MBX register by a value of the run's (P2d) if `extra` names
    /// one: only this image can set what the renderer has no name for.
    struct CodeBus<'a> {
        bus: &'a mut Mmio,
        code: u32,
        extra: Option<(usize, u32)>,
    }

    impl RegisterBus for CodeBus<'_> {
        fn read32(&mut self, off: usize) -> u32 {
            self.bus.read32(off)
        }

        fn write32(&mut self, off: usize, v: u32) {
            let v = match self.extra {
                _ if off == MBX + 0x650 => self.code,
                Some((reg, x)) if off == MBX + reg => x,
                _ => v,
            };
            self.bus.write32(off, v);
        }
    }

    /// One pixel layout the runs try: its name, bytes a pixel, the bits of
    /// the place it drops, and the place a value names (`None` if its blue
    /// isn't the texel's), shifted right by those bits.
    struct PLayout {
        name: &'static str,
        bytes: u32,
        shift: u32,
        place: fn(u32) -> Option<(u32, u32)>,
    }

    fn p_8888(v: u32) -> Option<(u32, u32)> {
        (v & 0xFF == 0xA0).then_some(((v >> 19) & 31, (v >> 11) & 31))
    }

    fn p_8888_bgr(v: u32) -> Option<(u32, u32)> {
        ((v >> 16) & 0xFF == 0xA0).then_some(((v >> 3) & 31, (v >> 11) & 31))
    }

    fn p_565(v: u32) -> Option<(u32, u32)> {
        (v & 31 == 20).then_some(((v >> 11) & 31, (v >> 6) & 31))
    }

    fn p_565_bgr(v: u32) -> Option<(u32, u32)> {
        ((v >> 11) & 31 == 20).then_some((v & 31, (v >> 6) & 31))
    }

    fn p_1555(v: u32) -> Option<(u32, u32)> {
        (v & 31 == 20).then_some(((v >> 10) & 31, (v >> 5) & 31))
    }

    fn p_4444(v: u32) -> Option<(u32, u32)> {
        (v & 15 == 10).then_some(((v >> 8) & 15, (v >> 4) & 15))
    }

    fn p_332(v: u32) -> Option<(u32, u32)> {
        (v & 3 == 2).then_some(((v >> 5) & 7, (v >> 2) & 7))
    }

    const P_LAYOUTS: [PLayout; 8] = [
        PLayout {
            name: "8888",
            bytes: 4,
            shift: 0,
            place: p_8888,
        },
        PLayout {
            name: "8888 BGR",
            bytes: 4,
            shift: 0,
            place: p_8888_bgr,
        },
        PLayout {
            name: "888",
            bytes: 3,
            shift: 0,
            place: p_8888,
        },
        PLayout {
            name: "565",
            bytes: 2,
            shift: 0,
            place: p_565,
        },
        PLayout {
            name: "565 BGR",
            bytes: 2,
            shift: 0,
            place: p_565_bgr,
        },
        PLayout {
            name: "1555",
            bytes: 2,
            shift: 0,
            place: p_1555,
        },
        PLayout {
            name: "4444",
            bytes: 2,
            shift: 1,
            place: p_4444,
        },
        PLayout {
            name: "332",
            bytes: 1,
            shift: 2,
            place: p_332,
        },
    ];

    /// The pixel orders the runs try.
    const P_ORDERS: [&str; 5] = [
        "rows (the picture's stride)",
        "rows (packed)",
        "twiddled (texture order)",
        "twiddled (x on the low bits)",
        "8 x 16 tiles, rows inside",
    ];

    /// Where pixel `(x, y)` of `gm` sits in `order` for pixels of `bytes`:
    /// its byte offset from the target. Texture order is the GLES driver's
    /// (`tex::twiddle_rect`: the shorter side twiddled, the longer side's
    /// bits above).
    fn p_offset(order: usize, x: u32, y: u32, bytes: u32, gm: PGeom) -> u32 {
        let (bu, bv) = (gm.w.trailing_zeros(), gm.h.trailing_zeros());
        match order {
            0 => y * gm.stride + x * bytes,
            1 => (y * gm.w + x) * bytes,
            2 => tex::twiddle_rect(x, y, bu, bv) * bytes,
            3 => tex::twiddle_rect(y, x, bv, bu) * bytes,
            _ => {
                let tile = (y / 16) * (gm.w / 8) + x / 8;
                (tile * 128 + (y % 16) * 8 + x % 8) * bytes
            }
        }
    }

    /// The `bytes` bytes at `off` of the target at `at`, little-endian,
    /// read from 2700G memory (a copy would cost the 16 KiB stack).
    fn p_value(mem: &Vram, at: u32, off: u32, bytes: u32) -> u32 {
        // A whole aligned word is one load (the same value the byte fold
        // gives, at a quarter of the reads: run P2L reads 600 MB).
        if bytes == 4 && off.is_multiple_of(4) {
            return mem.load((at + off) as usize).unwrap_or(0);
        }
        (0..bytes).fold(0, |v, k| {
            let o = off + k;
            let w = mem.load((at + (o & !3)) as usize).unwrap_or(0);
            v | ((w >> (8 * (o % 4))) & 0xFF) << (8 * k)
        })
    }

    /// How many of `gm`'s pixels, read as `l` in `order`, name their own
    /// place. Each pixel's offset comes from its place (run P's first
    /// decoder walked offsets instead and missed two rows in three of
    /// 24-bit pixels at a 128-byte stride: 352/1024 on the X51v).
    fn p_score(mem: &Vram, at: u32, l: &PLayout, order: usize, gm: PGeom) -> u32 {
        let mut score = 0;
        for y in 0..gm.h {
            for x in 0..gm.w {
                let off = p_offset(order, x, y, l.bytes, gm);
                score += u32::from(
                    (l.place)(p_value(mem, at, off, l.bytes)) == Some((x >> l.shift, y >> l.shift)),
                );
            }
        }
        score
    }

    /// Renders what `r` holds into `t` with `0x650` = `code`: whether it
    /// finished (false: refused, or hung and recovered).
    fn p_frame(
        g: &mut Rig,
        r: &mut Renderer,
        t: Target,
        code: u32,
        extra: Option<(usize, u32)>,
    ) -> bool {
        rt::check_in();
        // The renderer writes only changed registers: the code (and the
        // extra register) go in first, and the bus replaces the renderer's
        // own if it writes them.
        g.e.regs.write32(MBX + 0x650, code);
        if let Some((reg, v)) = extra {
            g.e.regs.write32(MBX + reg, v);
        }
        r.invalidate_all();
        let res = r.render(
            &mut CodeBus {
                bus: &mut g.e.regs,
                code,
                extra,
            },
            &mut g.mem,
            &mut g.clock,
            t,
        );
        let mut at = match res {
            Ok(Some(at)) => at,
            Ok(None) => return false,
            Err(err) => {
                log!("    render refused: {err:?}");
                return false;
            }
        };
        loop {
            while (g.clock.now_us().wrapping_sub(at) as i32) < 0 {}
            let p = r.poll(
                &mut CodeBus {
                    bus: &mut g.e.regs,
                    code,
                    extra,
                },
                &mut g.mem,
                &mut g.clock,
            );
            match p {
                Poll::Wait(next) => at = next,
                Poll::Done(_) => {
                    g.measured += 1;
                    return true;
                }
                p => {
                    g.sync_resets(r);
                    g.hangs += u32::from(p == Poll::Recovered);
                    log!("    {p:?}");
                    return false;
                }
            }
        }
    }

    /// A square ARGB8888 texture of side `8 << code`, `texel(x, y)` each,
    /// written twiddled a word at a time straight into 2700G memory (no
    /// buffer: the 16 KiB stack overflowed with one, run P's first
    /// emulator check).
    fn p_texture(g: &mut Rig, code: u8, texel: fn(u32, u32) -> u32) -> Option<Texture> {
        let side = 8u32 << code;
        let at = g.pool.alloc(side * side * 4, 128)?;
        for i in 0..side * side {
            let (x, y) = untwiddle(i);
            g.mem.store((at + 4 * i) as usize, &[texel(x, y)]);
        }
        g.mem.sync();
        Some(Texture {
            at,
            code,
            format: format::ARGB8888 as u8,
            u_mode: address::CLAMP as u8,
            v_mode: address::CLAMP as u8,
            linear: false,
            levels: 1,
        })
    }

    /// Gives back a texture from [`p_texture`].
    fn p_texture_free(g: &mut Rig, t: Texture) {
        let side = 8u32 << t.code;
        let _ = g.pool.free(t.at, side * side * 4);
    }

    /// `gm`'s target in the area at `area`, after the guard.
    const fn p_target(area: u32, gm: PGeom) -> Target {
        Target {
            picture: Picture {
                at: area + P_BEFORE,
                stride: gm.stride,
                w: gm.w as u16,
                h: gm.h as u16,
                format: PixelFormat::Argb8888,
            },
            x: 0,
            y: 0,
        }
    }

    /// The area filled with [`P_FILL`].
    fn p_fill(g: &mut Rig, area: u32) {
        let fill = [P_FILL; 64];
        for off in (0..P_AREA).step_by(4 * fill.len()) {
            g.mem.store((area + off) as usize, &fill);
        }
        g.mem.sync();
    }

    /// How a try ended.
    #[derive(PartialEq, Eq)]
    enum PTry {
        /// Hung, the core reset by the renderer.
        Hung,
        /// Refused or nothing rendered.
        Missed,
        /// Rendered: nothing in the area changed.
        Silent,
        /// Rendered: whether 9 in 10 pixels were in place in some reading.
        Done(bool),
    }

    /// One try of `code` into `gm`'s target (with `r` holding the placed
    /// texture drawn 1:1): the area filled, rendered, what changed counted
    /// (before the target, in its rows, after it), the best three readings
    /// and the raw words logged.
    fn p_try(g: &mut Rig, r: &mut Renderer, area: u32, gm: PGeom, code: u32) -> PTry {
        p_try_with(g, r, area, gm, code, None, false)
    }

    /// [`p_try`] with an extra register set (P2d), `quiet`: one line for
    /// the try (what changed and the best reading), no raw words.
    fn p_try_with(
        g: &mut Rig,
        r: &mut Renderer,
        area: u32,
        gm: PGeom,
        code: u32,
        extra: Option<(usize, u32)>,
        quiet: bool,
    ) -> PTry {
        p_fill(g, area);
        let hangs = g.hangs;
        let ok = p_frame(g, r, p_target(area, gm), code, extra);
        let back = g.e.regs.read32(MBX + 0x650);
        if g.hangs != hangs {
            log!("    HUNG (the core was reset); 0x650 reads 0x{back:x}");
            return PTry::Hung;
        }
        if !ok {
            log!("    not rendered; 0x650 reads 0x{back:x}");
            return PTry::Missed;
        }
        let (mut before, mut inside, mut after) = (0u32, 0u32, 0u32);
        let (mut first, mut last) = (None, 0u32);
        for off in (0..P_AREA).step_by(4) {
            if g.mem.load((area + off) as usize).unwrap_or(0) != P_FILL {
                if off < P_BEFORE {
                    before += 1;
                } else if off < P_BEFORE + gm.bytes() {
                    inside += 1;
                } else {
                    after += 1;
                }
                first.get_or_insert(off);
                last = off;
            }
        }
        let rel = |o: u32| o as i32 - P_BEFORE as i32;
        if !quiet {
            log!(
                "    done; 0x650 reads 0x{back:x}; words changed: {before} before, {inside} in the target's {} bytes, {after} after; first at {}, last at {} (bytes from the target)",
                gm.bytes(),
                first.map_or(0, rel),
                rel(last)
            );
        }
        if first.is_none() {
            if quiet {
                log!("    wrote nothing");
            }
            return PTry::Silent;
        }
        let at = area + P_BEFORE;
        let mut best = [(0u32, 0usize, 0usize); 3];
        for (li, l) in P_LAYOUTS.iter().enumerate() {
            for order in 0..P_ORDERS.len() {
                let score = p_score(&g.mem, at, l, order, gm);
                if score > best[2].0 {
                    best[2] = (score, li, order);
                    best.sort_unstable_by(|a, b| b.0.cmp(&a.0));
                }
            }
        }
        if quiet {
            let (score, li, order) = best[0];
            log!(
                "    changed {before}/{inside}/{after} (before/in/after); best {} {}: {score}/{}",
                P_LAYOUTS[li].name,
                P_ORDERS[order],
                gm.w * gm.h
            );
            return PTry::Done(score >= gm.w * gm.h * 9 / 10 && order == 0 && li == 0);
        }
        for &(score, li, order) in best.iter().filter(|b| b.0 > 0) {
            log!(
                "    {} {}: {score}/{} pixels in place",
                P_LAYOUTS[li].name,
                P_ORDERS[order],
                gm.w * gm.h
            );
        }
        // The raw words: the target's first 8, and the first 4 of the next
        // three rows at the stride.
        let word = |o: u32| g.mem.load((at + o) as usize).unwrap_or(0xDEAD_BEEF);
        log!(
            "    words: {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x}",
            word(0),
            word(4),
            word(8),
            word(12),
            word(16),
            word(20),
            word(24),
            word(28)
        );
        for line in 1..4 {
            let o = line * gm.stride;
            log!(
                "    line {line}: {:08x} {:08x} {:08x} {:08x}",
                word(o),
                word(o + 4),
                word(o + 8),
                word(o + 12)
            );
        }
        PTry::Done(best[0].0 >= gm.w * gm.h * 9 / 10)
    }

    /// `r` (`gm`'s size) set to hold the placed texture `tex` drawn 1:1
    /// (its top left `gm.w` x `gm.h` texels). The caller builds `r` in
    /// place: a renderer returned by value costs its size again on the
    /// stack.
    fn p_setup(g: &mut Rig, r: &mut Renderer, gm: PGeom, tex: Texture) {
        let _ = r.set_background(Some(BLACK));
        let (u, v) = (
            (i64::from(UV_ONE) * i64::from(gm.w) / 32) as i32,
            (i64::from(UV_ONE) * i64::from(gm.h) / 32) as i32,
        );
        let draw = Draw::rect(
            0,
            0,
            gm.w as i32,
            gm.h as i32,
            Paint::Texture {
                tex,
                uv: [(0, 0), (u, 0), (u, v), (0, v)],
                tint: WHITE,
            },
        );
        g.set(r, 0, Some(draw));
    }

    /// The codes run P tries: 0-15, then bits 4-11 added to 1 (RGB565) and
    /// bits 4-31 added to 6 (ARGB8888).
    fn p_tries() -> ([u32; 52], usize) {
        let mut tries = [0u32; 52];
        let mut n = 0;
        for code in 0..16 {
            tries[n] = code;
            n += 1;
        }
        for b in 4..12 {
            tries[n] = 1 | 1 << b;
            n += 1;
        }
        for b in 4..32 {
            tries[n] = 6 | 1 << b;
            n += 1;
        }
        (tries, n)
    }

    /// Run P's on-screen label for try `k` of `n`.
    fn p_card(g: &mut Rig, run: &str, k: usize, n: usize, code: u32) {
        let (mut l2, mut l3) = (heapless_name(), heapless_name());
        let _ = write!(l2, "TRY {} OF {n}", k + 1);
        let _ = write!(l3, "CODE 0X{code:X}");
        clock_card(g, [run, l2.as_str(), l3.as_str()], 0xFF20_4020, 0);
    }

    #[inline(never)]
    fn target_formats(g: &mut Rig) {
        log!(
            "P3: render targets: 0x650 codes 0-15, then bits 4-11 added to 1 and 4-31 added to 6; a 32 x 32 texture of placed texels (red x * 8, green y * 8, blue 0xa0) drawn 1:1 into a target at a 32-bit picture's stride (128 bytes), 4 KiB of guard before it, 56 KiB after"
        );
        let Some(tex) = p_texture(g, 2, p_texel) else {
            log!("  no memory for the texture");
            return;
        };
        let Some(area) = g.pool.alloc(P_AREA, 4096) else {
            log!("  no memory for the target");
            p_texture_free(g, tex);
            return;
        };
        let Some(mut r) = Renderer::new(&mut g.pool, P_SQUARE.w as u16, P_SQUARE.h as u16) else {
            log!("  no renderer");
            let _ = g.pool.free(area, P_AREA);
            p_texture_free(g, tex);
            return;
        };
        p_setup(g, &mut r, P_SQUARE, tex);
        let (tries, n) = p_tries();
        let (mut hung, mut decoded, mut silent) = (0, 0, 0);
        for (k, &code) in tries[..n].iter().enumerate() {
            p_card(g, "RUN P TARGETS", k, n, code);
            log!("  P3 try {} 0x650 0x{code:08x}", k + 1);
            match p_try(g, &mut r, area, P_SQUARE, code) {
                PTry::Hung => hung += 1,
                PTry::Silent => silent += 1,
                PTry::Done(d) => decoded += u32::from(d),
                PTry::Missed => {}
            }
        }
        // The renderer's own code back for anything after.
        g.e.regs.write32(MBX + 0x650, PixelFormat::Argb8888.code());
        log!(
            "P3: {n} tries, {decoded} decoded (9 in 10 pixels in place at least), {hung} hung, {silent} wrote nothing"
        );
        g.set(&mut r, 0, None);
        let _ = r.release(&mut g.pool);
        let _ = g.pool.free(area, P_AREA);
        p_texture_free(g, tex);
        clock_card(g, ["RUN P TARGETS", "DONE", ""], 0xFF20_4020, 3);
    }

    // ---- Run P2: dither, alpha, other target shapes --------------------------

    /// Bytes a pixel and each channel's bits for layout code `c` (bits 2-0
    /// of `0x650`, run P), and the channels of value `v`: red, green, blue,
    /// alpha (0 bits where the layout has none).
    fn p2_channels(c: u32, v: u32) -> (u32, [u32; 4], [u32; 4]) {
        match c & 7 {
            0 | 3 => (
                2,
                [5, 5, 5, 1],
                [(v >> 10) & 31, (v >> 5) & 31, v & 31, (v >> 15) & 1],
            ),
            1 => (2, [5, 6, 5, 0], [(v >> 11) & 31, (v >> 5) & 63, v & 31, 0]),
            2 | 7 => (
                2,
                [4, 4, 4, 4],
                [(v >> 8) & 15, (v >> 4) & 15, v & 15, (v >> 12) & 15],
            ),
            4 => (
                3,
                [8, 8, 8, 0],
                [(v >> 16) & 255, (v >> 8) & 255, v & 255, 0],
            ),
            _ => (
                4,
                [8, 8, 8, 8],
                [(v >> 16) & 255, (v >> 8) & 255, v & 255, v >> 24],
            ),
        }
    }

    /// Run P2b, one code into the ramp: for each of red, green and blue,
    /// each pixel's value against the grey level truncated to the
    /// channel's bits: how many rounded up by one, how many did anything
    /// else, and the threshold matrix over an 8 x 8 cell (the least
    /// fraction of a step, in the channel's own units below its bit, that
    /// rounded up there; `-` never), with the places where a larger
    /// fraction stayed down (not a threshold) counted.
    fn p2_dither(g: &mut Rig, area: u32, code: u32) {
        let at = area + P_BEFORE;
        let names = ["red", "green", "blue"];
        for ch in 0..3 {
            let (bytes, bits, _) = p2_channels(code, 0);
            let lost = 8 - bits[ch];
            let mut thr = [255u32; 64];
            let mut down = [0u32; 64];
            let (mut ups, mut odd) = (0u32, 0u32);
            for y in 0..P_RAMP.h {
                for x in 0..P_RAMP.w {
                    let v = p_value(&g.mem, at, y * P_RAMP.stride + x * bytes, bytes);
                    let got = p2_channels(code, v).2[ch];
                    let lv = p_level(x, y);
                    let (trunc, frac) = (lv >> lost, lv & ((1 << lost) - 1));
                    let cell = ((y % 8) * 8 + x % 8) as usize;
                    if got == trunc + 1 {
                        ups += 1;
                        thr[cell] = thr[cell].min(frac);
                    } else if got == trunc {
                        down[cell] = down[cell].max(frac + 1);
                    } else {
                        odd += 1;
                    }
                }
            }
            let mixed = (0..64)
                .filter(|&k| thr[k] != 255 && down[k] > thr[k])
                .count();
            log!(
                "    {} ({} bits): {ups} rounded up, {odd} other, {mixed} of 64 places not a threshold",
                names[ch],
                bits[ch]
            );
            if ups == 0 {
                continue;
            }
            for row in 0..8 {
                let mut line = heapless_name();
                for k in 0..8 {
                    let t = thr[row * 8 + k];
                    let _ = if t == 255 {
                        write!(line, "  -")
                    } else {
                        write!(line, " {t:2}")
                    };
                }
                log!("     {}", line.as_str());
            }
        }
    }

    /// Run P2c's strips: their alpha (the colour `0x__C08040`).
    const P2_ALPHAS: [u32; 4] = [0x00, 0x55, 0xAA, 0xFF];

    /// P2a, one shape: codes 0-7 into `gm` (its own frame: one renderer
    /// on the stack at a time). Returns (tries, hung).
    #[inline(never)]
    fn p2_shape(g: &mut Rig, area: u32, name: &str, gm: PGeom, tex: Texture) -> (u32, u32) {
        let Some(mut r) = Renderer::new(&mut g.pool, gm.w as u16, gm.h as u16) else {
            log!("  no renderer");
            return (0, 0);
        };
        p_setup(g, &mut r, gm, tex);
        let (mut decoded, mut hung) = (0, 0);
        for code in 0..8u32 {
            p_card(g, "RUN P2 SHAPES", code as usize, 8, code);
            log!("  P2a {name} code {code}");
            match p_try(g, &mut r, area, gm, code) {
                PTry::Hung => hung += 1,
                PTry::Done(d) => decoded += u32::from(d),
                _ => {}
            }
        }
        log!("  P2a {name}: {decoded} of 8 decoded");
        g.set(&mut r, 0, None);
        let _ = r.release(&mut g.pool);
        (8, hung)
    }

    /// P2b: the ramp at codes 0-15. Returns (tries, hung).
    #[inline(never)]
    fn p2_ramp(g: &mut Rig, area: u32) -> (u32, u32) {
        let Some(tex) = p_texture(g, 3, p_grey) else {
            log!("  no memory for the texture");
            return (0, 0);
        };
        let Some(mut r) = Renderer::new(&mut g.pool, 64, 64) else {
            log!("  no renderer");
            p_texture_free(g, tex);
            return (0, 0);
        };
        let _ = r.set_background(Some(BLACK));
        g.set(
            &mut r,
            0,
            Some(Draw::rect(0, 0, 64, 64, Paint::texture(tex))),
        );
        let mut hung = 0;
        for code in 0..16u32 {
            p_card(g, "RUN P2 DITHER", code as usize, 16, code);
            log!("  P2b code {code}");
            p_fill(g, area);
            let hangs = g.hangs;
            let ok = p_frame(g, &mut r, p_target(area, P_RAMP), code, None);
            if g.hangs != hangs {
                hung += 1;
                log!("    HUNG (the core was reset)");
                continue;
            }
            if !ok {
                log!("    not rendered");
                continue;
            }
            p2_dither(g, area, code);
        }
        g.set(&mut r, 0, None);
        let _ = r.release(&mut g.pool);
        p_texture_free(g, tex);
        (16, hung)
    }

    /// P2c: the alpha strips at codes 0-7. Returns (tries, hung).
    #[inline(never)]
    fn p2_alpha(g: &mut Rig, area: u32) -> (u32, u32) {
        let Some(mut r) = Renderer::new(&mut g.pool, 32, 32) else {
            log!("  no renderer");
            return (0, 0);
        };
        let _ = r.set_background(Some(0));
        for (k, &a) in P2_ALPHAS.iter().enumerate() {
            let c = a << 24 | 0x00C0_8040;
            let x = 8 * k as i32;
            g.set(&mut r, k, Some(Draw::rect(x, 0, 8, 16, Paint::Flat(c))));
            g.set(
                &mut r,
                4 + k,
                Some(Draw {
                    blend: Some((factor::SRC_ALPHA, factor::INV_SRC_ALPHA)),
                    ..Draw::rect(x, 16, 8, 16, Paint::Flat(c))
                }),
            );
        }
        let mut hung = 0;
        for code in 0..8u32 {
            p_card(g, "RUN P2 ALPHA", code as usize, 8, code);
            p_fill(g, area);
            let hangs = g.hangs;
            let ok = p_frame(g, &mut r, p_target(area, P_SQUARE), code, None);
            if g.hangs != hangs {
                hung += 1;
                log!("  P2c code {code}: HUNG (the core was reset)");
                continue;
            }
            if !ok {
                log!("  P2c code {code}: not rendered");
                continue;
            }
            p2_alpha_line(g, area, code);
        }
        for k in 0..8 {
            g.set(&mut r, k, None);
        }
        let _ = r.release(&mut g.pool);
        (8, hung)
    }

    /// P2c's log line for `code`: each strip's pixel value at rows 8 (as
    /// drawn) and 24 (blended), and the alpha field of each.
    fn p2_alpha_line(g: &Rig, area: u32, code: u32) {
        let (bytes, _, _) = p2_channels(code, 0);
        let px = |x: u32, y: u32| {
            p_value(
                &g.mem,
                area + P_BEFORE,
                y * P_SQUARE.stride + x * bytes,
                bytes,
            )
        };
        let top: [u32; 4] = core::array::from_fn(|k| px(4 + 8 * k as u32, 8));
        let low: [u32; 4] = core::array::from_fn(|k| px(4 + 8 * k as u32, 24));
        let a = |v: u32| p2_channels(code, v).2[3];
        log!(
            "  P2c code {code}: as drawn {:x} {:x} {:x} {:x}; blended {:x} {:x} {:x} {:x}; alpha field as drawn {} {} {} {}, blended {} {} {} {}",
            top[0],
            top[1],
            top[2],
            top[3],
            low[0],
            low[1],
            low[2],
            low[3],
            a(top[0]),
            a(top[1]),
            a(top[2]),
            a(top[3]),
            a(low[0]),
            a(low[1]),
            a(low[2]),
            a(low[3])
        );
    }

    /// P2d's registers: the render table's words with no known meaning,
    /// and what the renderer leaves in each (`m3::start_table`; `0x64C`
    /// and `0x6D0` it never writes: Dell's GLES writes 0 and 1).
    const P2D_REGS: [(usize, u32); 6] = [
        (0x61C, 0x0002_0004),
        (0x644, 0),
        (0x648, 0),
        (0x64C, 0),
        (0x6D0, 0),
        (0x6D8, 0x0100_0000),
    ];

    /// P2d: each bit of each of [`P2D_REGS`] flipped alone (code 6, the
    /// placed texture 1:1 into run P's target), one line a try: whether any
    /// bit changes the target's order (twiddled: render-to-texture) or
    /// anything else. The register is put back after each try. Returns
    /// (tries, hung, tries that read other than ARGB8888 rows).
    #[inline(never)]
    fn p2_bits(g: &mut Rig, area: u32, tex: Texture) -> (u32, u32, u32) {
        let Some(mut r) = Renderer::new(&mut g.pool, P_SQUARE.w as u16, P_SQUARE.h as u16) else {
            log!("  no renderer");
            return (0, 0, 0);
        };
        p_setup(g, &mut r, P_SQUARE, tex);
        let (mut tries, mut hung, mut other) = (0, 0, 0);
        for (ri, &(reg, base)) in P2D_REGS.iter().enumerate() {
            for bit in 0..32u32 {
                let v = base ^ 1 << bit;
                p_card(g, "RUN P2 BITS", ri * 32 + bit as usize, 6 * 32, v);
                log!("  P2d 0x{reg:03x} = 0x{v:08x} (bit {bit})");
                tries += 1;
                match p_try_with(g, &mut r, area, P_SQUARE, 6, Some((reg, v)), true) {
                    PTry::Hung => hung += 1,
                    PTry::Done(true) => {}
                    _ => other += 1,
                }
                g.e.regs.write32(MBX + reg, base);
            }
        }
        g.set(&mut r, 0, None);
        let _ = r.release(&mut g.pool);
        (tries, hung, other)
    }

    #[inline(never)]
    fn target_formats2(g: &mut Rig) {
        let Some(area) = g.pool.alloc(P_AREA, 4096) else {
            log!("P2: no memory for the target");
            return;
        };
        let (mut tries, mut hung) = (0u32, 0u32);
        log!(
            "P2a: target shapes: codes 0-7 into 16 x 16 at a 96-byte stride and 32 x 16 at 256 bytes, the placed texture 1:1"
        );
        let tex = p_texture(g, 2, p_texel);
        if let Some(tex) = tex {
            for (name, gm) in [("16 x 16", P_SMALL), ("32 x 16", P_WIDE)] {
                let (t, h) = p2_shape(g, area, name, gm, tex);
                tries += t;
                hung += h;
            }
        } else {
            log!("  no memory for the texture");
        }
        log!(
            "P2b: dither: a 64 x 64 grey ramp (level 64 + x / 8 + 8 * (y / 8): every level 64-127 at every place of an 8 x 8 cell), codes 0-15; thresholds in each channel's own fraction of a step (0 to 2^(8 - bits) - 1)"
        );
        let (t, h) = p2_ramp(g, area);
        tries += t;
        hung += h;
        log!(
            "P2c: alpha: four strips 0x__c08040 at alpha 0x00, 0x55, 0xaa, 0xff over a background of 0x00000000, drawn as they are (rows 0-15) and blended by their alpha (rows 16-31); codes 0-7; each strip's pixel value at rows 8 and 24"
        );
        let (t, h) = p2_alpha(g, area);
        tries += t;
        hung += h;
        log!(
            "P2d: render-table bits: each bit of 0x61c, 0x644, 0x648, 0x64c, 0x6d0 and 0x6d8 flipped alone from what the renderer leaves (0x20004, 0, 0, 0, 0, 0x1000000), code 6, run P's placed texture into its 32 x 32 target; one line a try"
        );
        if let Some(tex) = tex {
            let (t, h, o) = p2_bits(g, area, tex);
            tries += t;
            hung += h;
            log!("  P2d: {t} tries, {h} hung, {o} not plain ARGB8888 rows");
            p_texture_free(g, tex);
        }
        g.e.regs.write32(MBX + 0x650, PixelFormat::Argb8888.code());
        log!("P2: {tries} tries, {hung} hung");
        let _ = g.pool.free(area, P_AREA);
        clock_card(g, ["RUN P2", "DONE", ""], 0xFF20_4020, 3);
    }

    // ---- Run P2W: a 24-bit picture and dithered 16 bits on screen -----------

    /// Run P2W's picture: eight colour bars, then ramps from black to
    /// white, red, green and blue, across the screen.
    fn p2w_scene(g: &mut Rig, r: &mut Renderer) {
        const BARS: [u32; 8] = [
            0xFFFF_FFFF,
            0xFFFF_FF00,
            0xFF00_FFFF,
            0xFF00_FF00,
            0xFFFF_00FF,
            0xFFFF_0000,
            0xFF00_00FF,
            0xFF00_0000,
        ];
        for (k, &c) in BARS.iter().enumerate() {
            g.set(
                r,
                k,
                Some(Draw::rect(60 * k as i32, 0, 60, 320, Paint::Flat(c))),
            );
        }
        for (k, &c) in [0xFFFF_FFFFu32, 0xFFFF_0000, 0xFF00_FF00, 0xFF00_00FF]
            .iter()
            .enumerate()
        {
            let y = 320 + 80 * k as i32;
            g.set(
                r,
                8 + k,
                Some(Draw::rect(
                    0,
                    y,
                    480,
                    80,
                    Paint::Gouraud([BLACK, c, c, BLACK]),
                )),
            );
        }
    }

    /// The graphics plane on `at` (480 x 640, `stride` bytes a row) in
    /// display format `fmt` (`GSCTRL` bits 30-27: run D's codes, 6 RGB888
    /// included, which the display driver doesn't name).
    fn p2w_show(regs: &mut Mmio, at: u32, stride: u32, fmt: u32) {
        let plane = Plane {
            format: PlaneFormat::Argb8888,
            width: 480,
            height: 640,
            offset: at as usize,
            stride: stride as usize,
        };
        let gamma = regs.read32(dreg::GSCTRL) & (1 << 26);
        for (off, v) in plane.registers() {
            let v = match off {
                dreg::GSCTRL => (v & !(0xF << 27)) | fmt << 27 | gamma,
                dreg::GSCADR => Layer::SOLO.scadr(v),
                dreg::GBBASE => Layer::SOLO.bbase(v),
                _ => v,
            };
            regs.write32(off, v);
        }
    }

    /// The black bar (x 420-479, y 0-319) of P2W's picture at `at` read
    /// back from memory: every pixel that isn't black counted, the first
    /// four logged. The owner saw white specks there on screen in the 24-
    /// and 32-bit pictures (more in 32), none in the 16-bit ones: in memory
    /// (the render or the memory) or added at scan-out?
    fn p2w_bar(g: &Rig, at: u32, stride: u32, bytes: u32, when: &str) {
        let (mut n, mut first) = (0u32, heapless_name());
        for y in 0..320 {
            for x in 420..480 {
                let v = p_value(&g.mem, at, y * stride + x * bytes, bytes);
                let rgb = if bytes == 4 { v & 0x00FF_FFFF } else { v };
                if rgb != 0 {
                    if n < 4 {
                        let _ = write!(first, " ({x},{y}) 0x{v:x}");
                    }
                    n += 1;
                }
            }
        }
        log!(
            "    the black bar {when}: {n} of 19200 pixels not black{}",
            first.as_str()
        );
    }

    #[inline(never)]
    fn target_watch(g: &mut Rig) {
        log!(
            "P2W: the owner watches four pictures, each until a button (30 s at most): 1 rendered 24-bit (0x650 4) shown as RGB888 (GSCTRL format 6, 1920-byte rows); 2 RGB565 (0x650 1); 3 RGB565 dithered (0x650 9); 4 ARGB8888 (0x650 6)"
        );
        let Some(mut r) = Renderer::new(&mut g.pool, 480, 640) else {
            log!("  no renderer");
            return;
        };
        let _ = r.set_background(Some(BLACK));
        p2w_scene(g, &mut r);
        let at = map::BACKGROUND.at;
        let steps: [(&str, u32, u32, u32); 4] = [
            ("24-BIT", 4, 1920, 6),
            ("16-BIT PLAIN", 1, 960, PlaneFormat::Rgb565 as u32),
            ("16-BIT DITHERED", 9, 960, PlaneFormat::Rgb565 as u32),
            ("32-BIT", 6, 1920, PlaneFormat::Argb8888 as u32),
        ];
        for (k, &(name, code, stride, fmt)) in steps.iter().enumerate() {
            let mut l1 = heapless_name();
            let _ = write!(l1, "STEP {} OF 4", k + 1);
            clock_card(g, [l1.as_str(), name, "BUTTON: NEXT"], 0xFF20_2040, 3);
            let bytes = if code == 4 {
                3
            } else if stride == 960 {
                2
            } else {
                4
            };
            let t = Target {
                picture: Picture {
                    at,
                    stride,
                    w: 480,
                    h: 640,
                    format: if stride == 960 {
                        PixelFormat::Rgb565
                    } else {
                        PixelFormat::Argb8888
                    },
                },
                x: 0,
                y: 0,
            };
            let t0 = g.clock.now_us();
            let ok = p_frame(g, &mut r, t, code, None);
            let us = g.clock.now_us().wrapping_sub(t0);
            log!(
                "  P2W step {}: {name} (0x650 {code}, {bytes} bytes a pixel, {stride}-byte rows, display format {fmt}): rendered {ok} in {us} us",
                k + 1
            );
            p2w_bar(g, at, stride, bytes, "after the render");
            p2w_show(&mut g.e.regs, at, stride, fmt);
            log!(
                "    planes: GSCTRL 0x{:08x} GBBASE 0x{:08x} GDRCTRL 0x{:08x} VSCTRL 0x{:08x} VSCADR 0x{:08x} VBBASE 0x{:08x} HCCTRL 0x{:08x}",
                g.e.regs.read32(dreg::GSCTRL),
                g.e.regs.read32(dreg::GBBASE),
                g.e.regs.read32(dreg::GDRCTRL),
                g.e.regs.read32(lreg::VSCTRL),
                g.e.regs.read32(lreg::VSCADR),
                g.e.regs.read32(lreg::VBBASE),
                g.e.regs.read32(lreg::HCCTRL)
            );
            // A second before a press counts: in the owner's third run a
            // press meant for one picture also ended the next at once.
            rt::delay_us(1_000_000);
            g.arm_button();
            let w0 = rt::ticks();
            while us_since(w0) < WATCH_US && !g.button() {
                rt::check_in();
            }
            log!(
                "    shown {} s{}",
                us_since(w0) / 1_000_000,
                if us_since(w0) < WATCH_US {
                    ", a button"
                } else {
                    ", no button (30 s)"
                }
            );
            p2w_bar(g, at, stride, bytes, "after it was shown");
            let _ = show_screen(&mut g.e.regs);
        }
        g.e.regs.write32(MBX + 0x650, PixelFormat::Argb8888.code());
        for k in 0..12 {
            g.set(&mut r, k, None);
        }
        let _ = r.release(&mut g.pool);
        log!("P2W: done");
        clock_card(g, ["RUN P2W", "DONE", ""], 0xFF20_2040, 3);
    }

    // ---- Run T: the display's fetch against the engines, and 0x644 ---------

    /// T2: one try of `0x644` = `v` (code 6, run P's placed texture into
    /// its target), the whole mapping logged: for each row, which words
    /// changed (bit k: pixel k) and the place eight of its pixels show.
    fn t_orient(g: &mut Rig, r: &mut Renderer, area: u32, v: u32) {
        log!("  T2 0x644 = {v}");
        let _ = p_try_with(g, r, area, P_SQUARE, 6, Some((0x644, v)), false);
        g.e.regs.write32(MBX + 0x644, 0);
        let at = area + P_BEFORE;
        for y in 0..P_SQUARE.h {
            let mut changed = 0u32;
            let mut line = heapless_name();
            for x in 0..P_SQUARE.w {
                let w = g
                    .mem
                    .load((at + y * P_SQUARE.stride + 4 * x) as usize)
                    .unwrap_or(0);
                if w != P_FILL {
                    changed |= 1 << x;
                }
                if x % 4 == 0 {
                    let _ = match p_8888(w) {
                        Some((px, py)) => write!(line, " {px:2},{py:2}"),
                        None if w == P_FILL => write!(line, "    --"),
                        None => write!(line, "    ??"),
                    };
                }
            }
            log!("    row {y:2}: changed 0x{changed:08x};{}", line.as_str());
        }
    }

    /// Run T (see [`FETCH`]).
    #[inline(never)]
    fn fetch(g: &mut Rig) {
        log!(
            "T1: the display's fetch: the speed at Windows Mobile's clock with the graphics plane at 32 bits, 16 bits (the same memory read as RGB565: half the bytes), stopped (GSCADR bit 31 clear), and 32 bits again; DINTRS after each"
        );
        let found = (g.e.regs.read32(clock::COREPLL), diag::msc1());
        if found.0 != ClockSpeed::WM.pll || found.1 != 0x2484_2481 {
            log!("T1: not Windows Mobile's clock and bus timing: no speed measured");
        } else if let Some(mut c) = clock_rig(g) {
            let base = clock_check(g, &mut c, true, 2);
            clock_log(WM_STEP, Some(0), &base);
            let configs: [(&str, Option<(u32, u32)>); 4] = [
                ("32-bit plane", Some((1920, PlaneFormat::Argb8888 as u32))),
                ("16-bit plane", Some((960, PlaneFormat::Rgb565 as u32))),
                ("plane stopped", None),
                (
                    "32-bit plane again",
                    Some((1920, PlaneFormat::Argb8888 as u32)),
                ),
            ];
            let mut seed = 4;
            for (name, cfg) in configs {
                match cfg {
                    Some((stride, fmt)) => p2w_show(&mut g.e.regs, SCREEN.at, stride, fmt),
                    None => {
                        let v = g.e.regs.read32(dreg::GSCADR);
                        g.e.regs.write32(dreg::GSCADR, v & !(1 << 31));
                    }
                }
                rt::delay_us(50_000);
                let mut r3d = [0u32; 3];
                let mut read = [0u32; 3];
                for k in 0..3 {
                    let r = clock_check(g, &mut c, false, seed);
                    seed += 2;
                    clock_log(WM_STEP, Some(0), &r);
                    r3d[k] = r.render_us;
                    read[k] = r.read_us;
                }
                log!(
                    "  T1 {name}: 3D {} / {} / {} us, read {} / {} / {} us; DINTRS 0x{:x}",
                    r3d[0],
                    r3d[1],
                    r3d[2],
                    read[0],
                    read[1],
                    read[2],
                    g.e.regs.read32(dreg::DINTRS)
                );
            }
            let _ = show_screen(&mut g.e.regs);
            clock_rig_free(g, c);
        } else {
            log!("T1: no memory for the speed test");
        }
        log!(
            "T2: 0x644 bits 0 and 1 (run P2 found each moves the output): 1, 2 and 3, code 6, run P's placed texture into its 32 x 32 target; per row the words changed and the place eight pixels show (x,y; -- untouched, ?? not a placed texel)"
        );
        let Some(area) = g.pool.alloc(P_AREA, 4096) else {
            log!("  no memory for the target");
            return;
        };
        if let Some(tex) = p_texture(g, 2, p_texel) {
            t_orient_all(g, area, tex);
            p_texture_free(g, tex);
        } else {
            log!("  no memory for the texture");
        }
        g.e.regs.write32(MBX + 0x650, PixelFormat::Argb8888.code());
        let _ = g.pool.free(area, P_AREA);
        clock_card(g, ["RUN T", "DONE", ""], 0xFF10_6020, 3);
    }

    /// T2's three tries in their own frame (one renderer on the stack).
    #[inline(never)]
    fn t_orient_all(g: &mut Rig, area: u32, tex: Texture) {
        let Some(mut r) = Renderer::new(&mut g.pool, P_SQUARE.w as u16, P_SQUARE.h as u16) else {
            log!("  no renderer");
            return;
        };
        p_setup(g, &mut r, P_SQUARE, tex);
        for v in 1..=3 {
            p_card(g, "RUN T 0X644", v as usize - 1, 3, v);
            t_orient(g, &mut r, area, v);
        }
        g.set(&mut r, 0, None);
        let _ = r.release(&mut g.pool);
    }

    /// Run T3 (see [`RESOLVE`]): a full-screen render (2400 tiles: a
    /// textured quad over a Gouraud background) timed five times at each
    /// `0x644` (0 plain, 1 rows halved, 2 columns, 3 both) into the
    /// background picture; the exact time from the status bit as
    /// [`Rig::frame`] takes it.
    #[inline(never)]
    fn resolve(g: &mut Rig) {
        log!(
            "T3: the write-out downsample's cost: one full-screen render (2400 tiles), 5 times at each 0x644 (0, 1, 2, 3), code 6, into the background picture"
        );
        let Some(tex) = p_texture(g, 2, p_texel) else {
            log!("  no memory for the texture");
            return;
        };
        let Some(mut r) = Renderer::new(&mut g.pool, 480, 640) else {
            log!("  no renderer");
            p_texture_free(g, tex);
            return;
        };
        let _ = r.set_background(Some(BLACK));
        g.set(
            &mut r,
            0,
            Some(Draw::rect(
                0,
                0,
                480,
                640,
                Paint::Gouraud([BLACK, WHITE, BLUE, YELLOW]),
            )),
        );
        g.set(
            &mut r,
            1,
            Some(Draw::rect(40, 40, 400, 560, Paint::texture(tex))),
        );
        let t = Target {
            picture: Picture::screen(0),
            x: 0,
            y: 0,
        };
        let t = Target {
            picture: Picture {
                at: map::BACKGROUND.at,
                ..t.picture
            },
            ..t
        };
        let mut medians = [0u32; 4];
        for v in 0..4u32 {
            p_card(g, "RUN T3 RESOLVE", v as usize, 4, v);
            let mut times = Times::default();
            for _ in 0..5 {
                g.e.regs.write32(MBX + 0x644, v);
                r.invalidate_all();
                let res = r.render(
                    &mut CodeBus {
                        bus: &mut g.e.regs,
                        code: 6,
                        extra: Some((0x644, v)),
                    },
                    &mut g.mem,
                    &mut g.clock,
                    t,
                );
                if res.is_err() {
                    log!("    render refused: {res:?}");
                    continue;
                }
                let t0 = g.clock.now_us();
                let mut exact = None;
                while g.clock.now_us().wrapping_sub(t0) < 500_000 {
                    if g.e.regs.read32(MBX + reg::STATUS) & RENDER_DONE != 0 {
                        exact = Some(g.clock.now_us().wrapping_sub(t0));
                        break;
                    }
                }
                loop {
                    match r.poll(&mut g.e.regs, &mut g.mem, &mut g.clock) {
                        Poll::Wait(_) => {}
                        Poll::Done(_) => break,
                        p => {
                            g.sync_resets(&r);
                            log!("    {p:?}");
                            break;
                        }
                    }
                }
                if let Some(us) = exact {
                    times.add(us);
                    g.measured += 1;
                }
            }
            medians[v as usize] = times.median();
            log!(
                "  T3 0x644 = {v}: {} us median ({} - {} us, {} renders)",
                times.median(),
                times.min(),
                times.max(),
                times.n
            );
        }
        g.e.regs.write32(MBX + 0x644, 0);
        g.e.regs.write32(MBX + 0x650, PixelFormat::Argb8888.code());
        log!(
            "T3: plain {} us; rows halved {} us; columns halved {} us; both {} us",
            medians[0],
            medians[1],
            medians[2],
            medians[3]
        );
        for k in 0..2 {
            g.set(&mut r, k, None);
        }
        let _ = r.release(&mut g.pool);
        p_texture_free(g, tex);
        clock_card(g, ["RUN T3", "DONE", ""], 0xFF10_6020, 3);
    }

    /// P2X's marker: the picture is filled with it before each render, so
    /// a pixel the render never wrote shows it.
    const P2X_FILL: u32 = 0xFF12_3456;

    /// The colour P2W's bar at `x` (y below 320) should hold, as `code`'s
    /// layout stores it (red, green, blue only).
    fn p2x_bar(code: u32, x: u32) -> u32 {
        const BARS: [u32; 8] = [
            0xFFFFFF, 0xFFFF00, 0x00FFFF, 0x00FF00, 0xFF00FF, 0xFF0000, 0x0000FF, 0x000000,
        ];
        let c = BARS[(x / 60) as usize % 8];
        if code & 7 == 1 {
            ((c >> 8) & 0xF800) | ((c >> 5) & 0x07E0) | ((c >> 3) & 0x001F)
        } else {
            c
        }
    }

    /// One P2X check: its render, and what was found.
    struct P2xCount {
        /// The render finished.
        ok: bool,
        /// Bar pixels white, never written (the marker), other wrong.
        white: u32,
        unwritten: u32,
        wrong: u32,
        /// Pixels differing from the earlier render.
        differ: u32,
        /// Pixels with either fault (a pixel can have both).
        bad: u32,
        /// The first six of them, `(x,y) value`.
        list: Name,
    }

    /// P2X, one render into `at` with `code`: the bars (y below 320)
    /// against their colours, and if `other` is given, every pixel against
    /// the same picture rendered before at `other`.
    fn p2x_count(
        g: &mut Rig,
        r: &mut Renderer,
        at: u32,
        stride: u32,
        bytes: u32,
        code: u32,
        other: Option<u32>,
    ) -> P2xCount {
        g.fill(at, (0, 0, 480, 640), P2X_FILL);
        let t = Target {
            picture: Picture {
                at,
                stride,
                w: 480,
                h: 640,
                format: if stride == 960 {
                    PixelFormat::Rgb565
                } else {
                    PixelFormat::Argb8888
                },
            },
            x: 0,
            y: 0,
        };
        let ok = p_frame(g, r, t, code, None);
        let mask = if bytes == 2 { 0xFFFF } else { 0x00FF_FFFF };
        let fill = if bytes == 2 {
            P2X_FILL & 0xFFFF
        } else {
            P2X_FILL & mask
        };
        let mut c = P2xCount {
            ok,
            white: 0,
            unwritten: 0,
            wrong: 0,
            differ: 0,
            bad: 0,
            list: heapless_name(),
        };
        for y in 0..640u32 {
            for x in 0..480u32 {
                let off = y * stride + x * bytes;
                let v = p_value(&g.mem, at, off, bytes) & mask;
                let mut bad = false;
                if y < 320 && v != p2x_bar(code, x) {
                    bad = true;
                    if v == mask {
                        c.white += 1;
                    } else if v == fill || (bytes == 2 && v == (P2X_FILL >> 16) & 0xFFFF) {
                        c.unwritten += 1;
                    } else {
                        c.wrong += 1;
                    }
                }
                if let Some(o) = other
                    && p_value(&g.mem, o, off, bytes) & mask != v
                {
                    c.differ += 1;
                    bad = true;
                }
                if bad {
                    if c.bad < 6 {
                        let _ = write!(c.list, " ({x},{y}) 0x{v:x}");
                    }
                    c.bad += 1;
                }
            }
        }
        c
    }

    /// [`p2x_count`], logged (render `k`): counts and the first six of
    /// each kind.
    #[allow(clippy::too_many_arguments)]
    fn p2x_check(
        g: &mut Rig,
        r: &mut Renderer,
        at: u32,
        stride: u32,
        bytes: u32,
        code: u32,
        other: Option<u32>,
        k: u32,
    ) {
        let c = p2x_count(g, r, at, stride, bytes, code, other);
        let listed = c.bad > 0;
        log!(
            "  P2X code {code} render {k} ({}): bars: {} white, {} never written, {} other wrong; {} against render 1{}",
            c.ok,
            c.white,
            c.unwritten,
            c.wrong,
            if other.is_some() { c.differ } else { 0 },
            if listed { ";" } else { "" }
        );
        if listed {
            log!("   {}", c.list.as_str());
        }
    }

    /// Run P2X (see [`SPECKS`]).
    #[inline(never)]
    fn specks(g: &mut Rig) {
        log!(
            "P2X: P2W's picture rendered 5 times in each of 32-bit (code 6), 24-bit (code 4) and RGB565 (code 1): render 1 into the background picture, renders 2-5 into the second screen; each over a marker fill (0xff123456); the bars checked against their colours and every pixel against render 1"
        );
        let Some(mut r) = Renderer::new(&mut g.pool, 480, 640) else {
            log!("  no renderer");
            return;
        };
        let _ = r.set_background(Some(BLACK));
        p2w_scene(g, &mut r);
        let (a, b) = (map::BACKGROUND.at, map::SCREENS[1].at);
        for (code, stride, bytes) in [(6u32, 1920u32, 4u32), (4, 1920, 3), (1, 960, 2)] {
            p_card(g, "RUN P2X SPECKS", code as usize, 6, code);
            p2x_check(g, &mut r, a, stride, bytes, code, None, 1);
            for k in 2..=5 {
                p2x_check(g, &mut r, b, stride, bytes, code, Some(a), k);
            }
        }
        g.e.regs.write32(MBX + 0x650, PixelFormat::Argb8888.code());
        for k in 0..12 {
            g.set(&mut r, k, None);
        }
        let _ = r.release(&mut g.pool);
        log!("P2X: done");
        clock_card(g, ["RUN P2X", "DONE", ""], 0xFF10_6020, 3);
    }

    /// Run P2Y (see [`SPECKS2`]).
    #[inline(never)]
    fn specks2(g: &mut Rig) {
        log!(
            "P2Y: what P2X's errors need. Y1: one 32-bit render read back three times, word by word (the CPU's reads); Y2: P2W's picture rendered 5 times at 32 bits (code 6) with the screen shown at 32 bits, at 16 bits (the same memory as RGB565) and stopped (GSCADR bit 31 clear), checked as in P2X"
        );
        let Some(mut r) = Renderer::new(&mut g.pool, 480, 640) else {
            log!("  no renderer");
            return;
        };
        let _ = r.set_background(Some(BLACK));
        p2w_scene(g, &mut r);
        let (a, b) = (map::BACKGROUND.at, map::SCREENS[1].at);
        // Y1: the reads.
        p_card(g, "RUN P2Y READS", 0, 4, 6);
        p2x_check(g, &mut r, a, 1920, 4, 6, None, 1);
        let words = 480 * 640;
        let mut differ = [0u32; 2];
        for (pass, d) in differ.iter_mut().enumerate() {
            let mut list = heapless_name();
            for k in 0..words {
                let o = (a + 4 * k) as usize;
                let (x, y) = (g.mem.load(o), g.mem.load(o));
                if x != y {
                    if *d < 4 {
                        let _ = write!(list, " word {k}: {x:x?} then {y:x?}");
                    }
                    *d += 1;
                }
            }
            log!(
                "  Y1 pass {}: {} of {words} words read differently twice in a row{}",
                pass + 1,
                *d,
                list.as_str()
            );
        }
        // Y2: the display's fetch.
        let configs: [(&str, Option<(u32, u32)>); 3] = [
            (
                "the screen at 32 bits",
                Some((1920, PlaneFormat::Argb8888 as u32)),
            ),
            (
                "the screen at 16 bits",
                Some((960, PlaneFormat::Rgb565 as u32)),
            ),
            ("the screen stopped", None),
        ];
        for (ci, (name, cfg)) in configs.into_iter().enumerate() {
            p_card(g, "RUN P2Y FETCH", ci + 1, 4, ci as u32);
            match cfg {
                Some((stride, fmt)) => p2w_show(&mut g.e.regs, SCREEN.at, stride, fmt),
                None => {
                    let v = g.e.regs.read32(dreg::GSCADR);
                    g.e.regs.write32(dreg::GSCADR, v & !(1 << 31));
                }
            }
            rt::delay_us(50_000);
            log!("  Y2 {name}:");
            p2x_check(g, &mut r, a, 1920, 4, 6, None, 1);
            for k in 2..=5 {
                p2x_check(g, &mut r, b, 1920, 4, 6, Some(a), k);
            }
            let _ = show_screen(&mut g.e.regs);
        }
        g.e.regs.write32(MBX + 0x650, PixelFormat::Argb8888.code());
        for k in 0..12 {
            g.set(&mut r, k, None);
        }
        let _ = r.release(&mut g.pool);
        log!("P2Y: done");
        clock_card(g, ["RUN P2Y", "DONE", ""], 0xFF10_6020, 3);
    }

    /// Run P2L's renders with the screen as dithered RGB565.
    const P2L_RENDERS: u32 = 1000;
    /// Run P2L's control renders (the screen at 32 bits), before and after.
    const P2L_CONTROLS: u32 = 5;

    /// Run P2L's tally of one phase.
    #[derive(Default)]
    struct P2lTally {
        renders: u32,
        unfinished: u32,
        /// Renders with any wrong or differing pixel.
        bad_renders: u32,
        white: u32,
        unwritten: u32,
        wrong: u32,
        differ: u32,
        bad: u32,
    }

    impl P2lTally {
        fn add(&mut self, c: &P2xCount) {
            self.renders += 1;
            self.unfinished += u32::from(!c.ok);
            self.bad_renders += u32::from(c.bad > 0);
            self.white += c.white;
            self.unwritten += c.unwritten;
            self.wrong += c.wrong;
            self.differ += c.differ;
            self.bad += c.bad;
        }

        fn log(&self, name: &str, us: u32) {
            log!(
                "  P2L {name}: {} renders ({} unfinished) in {} ms: {} with a wrong pixel; {} pixels wrong or differing (bars: {} white, {} never written, {} other wrong; {} against the first render)",
                self.renders,
                self.unfinished,
                us / 1000,
                self.bad_renders,
                self.bad,
                self.white,
                self.unwritten,
                self.wrong,
                self.differ
            );
        }
    }

    /// Run P2L's line on the RGB565 screen (its bottom band): `s` in 4x
    /// letters over black, each pixel pair one word.
    fn p2l_label(g: &mut Rig, s: &str) {
        // Rows 560-639 of the RGB565 screen are rows 280-319 of the 32-bit
        // picture the 2D fill sees (960 bytes a row against 1920).
        g.fill(SCREEN.at, (0, 280, 480, 40), 0);
        let white = 0xFFFF_FFFFu32;
        for (k, ch) in s.chars().enumerate() {
            let x0 = 16 + k as u32 * (WIDTH as u32 + 1) * 4;
            for (gy, &bits) in font::glyph(ch).iter().enumerate() {
                for gx in 0..WIDTH as u32 {
                    if bits & (1 << (WIDTH as u32 - 1 - gx)) == 0 {
                        continue;
                    }
                    for dy in 0..4 {
                        let y = 580 + gy as u32 * 4 + dy;
                        let at = SCREEN.at + y * 960 + 2 * (x0 + gx * 4);
                        g.mem.store(at as usize, &[white, white]);
                    }
                }
            }
        }
        g.mem.sync();
    }

    /// Per-render lines P2L logs a phase at most (the run log sent over USB
    /// filled at render 400 in run 1; the card log keeps every line, but a
    /// line per render says nothing a count doesn't).
    const P2L_LISTED: u32 = 20;

    /// A 32-bit picture's words hashed (FNV-1a): whether anything wrote it.
    fn p2l_hash(g: &Rig, at: u32) -> u32 {
        (0..480 * 640u32).fold(0x811C_9DC5, |h, k| {
            (h ^ g.mem.load((at + 4 * k) as usize).unwrap_or(0)).wrapping_mul(0x0100_0193)
        })
    }

    /// P2L's control: [`P2L_CONTROLS`] renders compared with the first,
    /// the screen at 32 bits.
    fn p2l_control(g: &mut Rig, r: &mut Renderer, a: u32, b: u32, name: &str) -> P2lTally {
        let _ = show_screen(&mut g.e.regs);
        rt::delay_us(50_000);
        let mut t = P2lTally::default();
        let t0 = rt::ticks();
        for k in 0..P2L_CONTROLS {
            let c = p2x_count(g, r, b, 1920, 4, 6, Some(a));
            if c.bad > 0 {
                log!(
                    "   {name} render {}: {} pixels:{}",
                    k + 1,
                    c.bad,
                    c.list.as_str()
                );
            }
            t.add(&c);
        }
        t.log(name, us_since(t0));
        t
    }

    /// Run P2L (see [`LONG_COUNT`]).
    #[inline(never)]
    fn long_count(g: &mut Rig) {
        log!(
            "P2L: ADR-0023's long count. P2W's picture rendered at 32 bits (code 6) into the background picture once (the first render), then into the second screen, each checked as in P2X and compared with the first: {P2L_CONTROLS} with the screen at 32 bits (a control: P2Y found errors there), {P2L_RENDERS} with the screen as dithered RGB565 (P2W's picture, 0x650 9, 960-byte rows) against the first made again under it, {P2L_CONTROLS} at 32 bits again; version 2"
        );
        clock_card(
            g,
            ["RUN P2L", "1000 RENDERS", "ABOUT 10 MIN"],
            0xFF10_6020,
            3,
        );
        let Some(mut r) = Renderer::new(&mut g.pool, 480, 640) else {
            log!("  no renderer");
            return;
        };
        let _ = r.set_background(Some(BLACK));
        p2w_scene(g, &mut r);
        let (a, b) = (map::BACKGROUND.at, map::SCREENS[1].at);
        let first = p2x_count(g, &mut r, a, 1920, 4, 6, None);
        log!(
            "  P2L first render ({}), the screen at 32 bits: bars: {} white, {} never written, {} other wrong{}",
            first.ok,
            first.white,
            first.unwritten,
            first.wrong,
            first.list.as_str()
        );
        let before = p2l_control(g, &mut r, a, b, "control before (the screen at 32 bits)");
        // The screen: P2W's picture as dithered RGB565, the run's line under
        // it.
        let s = Target {
            picture: Picture {
                at: SCREEN.at,
                stride: 960,
                w: 480,
                h: 640,
                format: PixelFormat::Rgb565,
            },
            x: 0,
            y: 0,
        };
        // Run 1 (2026-10-21) found the first render black by the 16-bit
        // phase: each step below hashes it, so the log names the one that
        // writes it.
        let h0 = p2l_hash(g, a);
        log!("  the first render's hash after the control: 0x{h0:08x}");
        let shown = p_frame(g, &mut r, s, 9, None);
        log!(
            "  after the RGB565 screen render ({shown}): the first render's hash 0x{:08x}, the screen's 0x{:08x}",
            p2l_hash(g, a),
            p2l_hash(g, SCREEN.at)
        );
        p2l_label(g, "P2L COUNTING");
        log!(
            "  after the screen's line: the first render's hash 0x{:08x}",
            p2l_hash(g, a)
        );
        p2w_show(&mut g.e.regs, SCREEN.at, 960, PlaneFormat::Rgb565 as u32);
        log!("  the screen: P2W's picture dithered to RGB565 ({shown}), scanned out at 16 bits");
        rt::delay_us(50_000);
        log!(
            "  after the plane switch: the first render's hash 0x{:08x}",
            p2l_hash(g, a)
        );
        // The reference again, under the 16-bit screen; hashed, and checked
        // every 100 renders.
        let again = p2x_count(g, &mut r, a, 1920, 4, 6, None);
        let ref_hash = p2l_hash(g, a);
        log!(
            "  P2L reference made again ({}), the screen at 16 bits: bars: {} white, {} never written, {} other wrong{}; hash 0x{ref_hash:08x}",
            again.ok,
            again.white,
            again.unwritten,
            again.wrong,
            again.list.as_str()
        );
        let mut main = P2lTally::default();
        let (mut listed, mut changed) = (0u32, 0u32);
        let t0 = rt::ticks();
        for k in 1..=P2L_RENDERS {
            let c = p2x_count(g, &mut r, b, 1920, 4, 6, Some(a));
            if (c.bad > 0 || !c.ok) && listed < P2L_LISTED {
                listed += 1;
                log!(
                    "   16-bit screen render {k} ({}): {} pixels:{}",
                    c.ok,
                    c.bad,
                    c.list.as_str()
                );
            }
            main.add(&c);
            if k % 100 == 0 {
                main.log("so far, the screen at 16 bits", us_since(t0));
                let h = p2l_hash(g, a);
                if h != ref_hash {
                    changed += 1;
                    log!("   the reference changed by render {k}: hash 0x{h:08x}");
                }
            }
            if k % 25 == 0 {
                let mut l = heapless_name();
                let _ = write!(l, "{k} OF {P2L_RENDERS} BAD {}", main.bad_renders);
                p2l_label(g, l.as_str());
            }
        }
        if main.bad_renders > listed {
            log!(
                "   ({} more renders with a wrong pixel not listed)",
                main.bad_renders - listed
            );
        }
        log!(
            "  the reference: changed at {changed} of {} checks",
            P2L_RENDERS / 100
        );
        main.log("the screen at 16 bits", us_since(t0));
        let after = p2l_control(g, &mut r, a, b, "control after (the screen at 32 bits)");
        log!(
            "P2L: {} of {} renders under the 16-bit screen had a wrong pixel ({} pixels); the controls at 32 bits: {} of {} before, {} of {} after",
            main.bad_renders,
            main.renders,
            main.bad,
            before.bad_renders,
            before.renders,
            after.bad_renders,
            after.renders
        );
        g.e.regs.write32(MBX + 0x650, PixelFormat::Argb8888.code());
        for k in 0..12 {
            g.set(&mut r, k, None);
        }
        let _ = r.release(&mut g.pool);
        log!("P2L: done");
        let mut l2 = heapless_name();
        let _ = write!(l2, "BAD {} OF {}", main.bad_renders, main.renders);
        clock_card(g, ["RUN P2L DONE", l2.as_str(), ""], 0xFF10_6020, 5);
    }

    // ---- Run FF: a full 2D FIFO (quality audit C37) --------------------------

    /// The register window with the 2D FIFO's level read as full until a
    /// tick: run FF2's stand-in for an engine that stops taking commands
    /// (no known way makes the real one stop short of its clock, `MBXCLK`
    /// 0, whose effect on reads of the MBX block is unmeasured).
    struct FullFifo<'a> {
        regs: &'a mut Mmio,
        /// Status reads answer full until this `rt::ticks` value.
        until: Option<u32>,
        /// Status reads answered full.
        faked: u32,
    }

    impl RegisterBus for FullFifo<'_> {
        fn read32(&mut self, offset: usize) -> u32 {
            let v = self.regs.read32(offset);
            if offset == MBX + reg::STATUS
                && let Some(t) = self.until
            {
                if (rt::ticks().wrapping_sub(t) as i32) < 0 {
                    self.faked += 1;
                    // 511 words used (bits 16-23 and 30): no room.
                    return v | 0x00FF_0000 | 1 << 30;
                }
                self.until = None;
            }
            v
        }

        fn write32(&mut self, offset: usize, value: u32) {
            self.regs.write32(offset, value);
        }
    }

    /// The rig's 2700G memory lent to the 2D renderer.
    struct VramRef<'a>(&'a mut Vram);

    impl LocalMem for VramRef<'_> {
        fn store(&mut self, at: usize, words: &[u32]) {
            self.0.store(at, words);
        }

        fn load(&self, at: usize) -> Option<u32> {
            self.0.load(at)
        }

        fn sync(&mut self) {
            self.0.sync();
        }
    }

    type Ff2d<'a> = Mbx2dRenderer<FullFifo<'a>, VramRef<'a>>;

    /// Run FF2's frame (7 KiB: not on the stack).
    static FF_FRAME: rt::Slot<Frame> = rt::Slot::new();

    /// Run FF2's picture: the host test's shapes, moved by `shift`.
    fn ff_scene(f: &mut Frame, shift: u16) {
        f.clear();
        f.push(Cmd::Fill {
            r: DrawRect::new(0, 0, 480, 40),
            color: 0xFF20_3040,
        });
        f.push(Cmd::Fill {
            r: DrawRect::new(10 + shift, 60, 200, 120),
            color: 0x8040_8080,
        });
        f.push(Cmd::HGradient {
            r: DrawRect::new(20, 200, 300, 50),
            left: 0x10FF_FFFF,
            right: 0xC000_0040,
        });
        f.push(Cmd::HGradient {
            r: DrawRect::new(20, 260, 300, 10),
            left: 0xFF00_0000,
            right: 0xFFFF_FFFF,
        });
        f.push(Cmd::Outline {
            r: DrawRect::new(5 + shift, 55, 210, 130),
            color: 0x60FF_FFFF,
        });
        f.push(Cmd::Fill {
            r: DrawRect::new(60, 320 + 2 * shift, 360, 200),
            color: 0xC0A0_4010,
        });
        f.damage(DrawRect::new(0, 0, 480, 640));
    }

    /// Run FF2's wallpaper: every channel moving.
    fn ff_wall(x: u32, y: u32) -> u32 {
        0xFF00_0000 | ((x * 255 / 479) << 16) | ((y * 255 / 639) << 8) | ((x + y) & 0xFF)
    }

    /// Polls `r` until its last frame is on the screen (1 s at most).
    fn ff_settle(r: &mut Ff2d<'_>) -> bool {
        let t0 = rt::ticks();
        while !r.poll() {
            if us_since(t0) > 1_000_000 {
                return false;
            }
        }
        true
    }

    /// The picture on screen: where (`GSCADR`'s) and its words' hash.
    fn ff_shown(r: &mut Ff2d<'_>) -> (u32, u32) {
        let (e, m) = r.parts();
        let at = (e.regs.read32(dreg::GSCADR) & 0x7F_FFFF) * 16;
        let mut h = 0x811C_9DC5u32;
        for k in 0..480 * 640 {
            h = (h ^ m.load((at + 4 * k) as usize).unwrap_or(0)).wrapping_mul(0x0100_0193);
        }
        (at, h)
    }

    /// Run FF1: the FIFO filled for real.
    #[inline(never)]
    fn ff1(g: &mut Rig) {
        const FILLS: u32 = 300;
        log!(
            "FF1: the FIFO filled for real: {FILLS} full-screen fills queued as fast as the CPU can (7 words each; the FIFO holds {} words), the level read before each, the waits for room timed",
            cmd::FIFO_WORDS
        );
        clock_card(g, ["RUN FF", "STEP 1 OF 2", "FIFO FILLING"], 0xFF20_2040, 2);
        let (mut min_free, mut waited, mut longest, mut refused) = (u32::MAX, 0u32, 0u32, 0u32);
        let t_all = rt::ticks();
        for k in 0..FILLS {
            let c = if k % 2 == 0 { 0xFF40_2020 } else { 0xFF20_2040 };
            let w = cmd::fill(
                Format::BPP32,
                SCREEN.at as usize,
                SCREEN.stride as usize,
                (0, 0, 480, 640),
                c,
            );
            min_free = min_free.min(cmd::free_bytes(g.e.status()));
            let t0 = rt::ticks();
            let r = g.e.submit(&w);
            let us = us_since(t0);
            if us > 100 {
                waited += 1;
            }
            longest = longest.max(us);
            if let Err(e) = r {
                log!("  FF1 fill {k}: refused ({e})");
                refused += 1;
                break;
            }
            if k % 32 == 0 {
                rt::check_in();
            }
        }
        let queued_us = us_since(t_all);
        let done = g.e.finish(g.mem.0, 10_000_000);
        log!(
            "  FF1: {FILLS} fills queued in {} ms, the last done {} ms later ({}); least room seen {} bytes of {}; {waited} fills waited for room, the longest {longest} us; {refused} refused (FIFO full 100 ms)",
            queued_us / 1000,
            done.map_or(0, |us| us / 1000),
            if done.is_ok() {
                "the fence landed"
            } else {
                "the fence never landed"
            },
            min_free,
            4 * cmd::FIFO_WORDS
        );
        log!(
            "  FF1: the FIFO read as full {}: a busy engine {} (each command waited less than 100 ms)",
            if min_free < 28 {
                "(less room than one fill)"
            } else {
                "never"
            },
            if refused == 0 && longest < 100_000 {
                "isn't taken for a hung one"
            } else {
                "was refused"
            }
        );
    }

    /// Run FF2: the 2D renderer on the real engine, the FIFO read as full.
    #[inline(never)]
    fn ff2(g: &mut Rig) {
        log!(
            "FF2: the 2D renderer (ADR-0023) on the engine, its FIFO read as full for 150 ms from the start of frame B: B must be dropped (fifo_full 1, no stall), not shown, and B drawn next whole and as B drawn after A without a drop"
        );
        clock_card(
            g,
            ["RUN FF", "STEP 2 OF 2", "DROPPED FRAME"],
            0xFF20_2040,
            2,
        );
        let Some(f) = FF_FRAME.put_with(Frame::new) else {
            log!("  FF2: started twice");
            return;
        };
        // One window offset for the two streams: each starts after a reset.
        g.e.reset(rt::delay_us);
        let _ = store32_volatile(g.mem.0, map::FENCE.at as usize, 0);
        cpu::drain_write_buffer();
        let (mut checks, mut bad) = (0u32, 0u32);
        let mut expect = |what: &str, ok: bool| {
            checks += 1;
            bad += u32::from(!ok);
            log!("  {} {what}", if ok { "ok " } else { "BAD" });
        };
        let faked = {
            let mut r: Ff2d<'_> = Mbx2dRenderer::new(
                FullFifo {
                    regs: &mut g.e.regs,
                    until: None,
                    faked: 0,
                },
                VramRef(&mut g.mem),
                Layout::X51V,
                rt::ticks,
                3_250,
            );
            let mut row = [0u32; 480];
            for y in 0..640u32 {
                for (x, p) in row.iter_mut().enumerate() {
                    *p = ff_wall(x as u32, y);
                }
                r.wallpaper_row(y as u16, &row);
            }
            r.show(rt::delay_us);
            // A.
            ff_scene(f, 0);
            let _ = r.draw(f);
            expect("frame A on screen", ff_settle(&mut r));
            let a = ff_shown(&mut r);
            expect(
                "A: nothing dropped, no stall",
                r.fifo_full == 0 && r.stalls == 0,
            );
            // B, the FIFO read as full for 150 ms (a command waits 100).
            ff_scene(f, 40);
            r.parts().0.regs.until = Some(rt::ticks().wrapping_add(150 * 3_250));
            let t0 = rt::ticks();
            let _ = r.draw(f);
            let settled = ff_settle(&mut r);
            let us = us_since(t0);
            let faked = r.parts().0.regs.faked;
            log!(
                "  FF2 frame B: {us} us to settle, {faked} status reads answered full; fifo_full {} (this frame {}), stalls {}",
                r.fifo_full,
                r.last.fifo_full,
                r.stalls
            );
            expect("B's fence landed (no reset)", settled);
            expect(
                "B dropped: counted once, apart from stalls",
                r.fifo_full == 1 && r.last.fifo_full == 1 && r.stalls == 0,
            );
            expect(
                "the dropped frame isn't shown (A still on screen)",
                ff_shown(&mut r) == a,
            );
            // B again: whole.
            let _ = r.draw(f);
            expect("B drawn again is on screen", ff_settle(&mut r));
            let b1 = ff_shown(&mut r);
            expect(
                "B again: nothing dropped this time, no stall",
                r.fifo_full == 1 && r.last.fifo_full == 0 && r.stalls == 0,
            );
            expect("B differs from A", b1.1 != a.1);
            // Its pixels kept in the pool (the 3D pool, unused here).
            let keep = map::POOL.at;
            {
                let (_, m) = r.parts();
                for k in 0..480 * 640u32 {
                    let v = m.load((b1.0 + 4 * k) as usize).unwrap_or(0);
                    m.store((keep + 4 * k) as usize, &[v]);
                }
                m.sync();
            }
            // The control: A, then B, no drop.
            ff_scene(f, 0);
            let _ = r.draw(f);
            let ok_a = ff_settle(&mut r);
            ff_scene(f, 40);
            let _ = r.draw(f);
            let ok_b = ff_settle(&mut r);
            expect("the control's A and B on screen", ok_a && ok_b);
            let b2 = ff_shown(&mut r);
            let (mut differ, mut worst) = (0u32, 0u32);
            {
                let (_, m) = r.parts();
                for k in 0..480 * 640u32 {
                    let x = m.load((b2.0 + 4 * k) as usize).unwrap_or(0);
                    let y = m.load((keep + 4 * k) as usize).unwrap_or(0);
                    if (x ^ y) & 0x00FF_FFFF != 0 {
                        differ += 1;
                        for s in [0, 8, 16] {
                            worst = worst.max(((x >> s) & 0xFF).abs_diff((y >> s) & 0xFF));
                        }
                    }
                }
            }
            log!(
                "  FF2 B after the drop against B drawn after A: {differ} pixels differ, by {worst} at most a channel"
            );
            expect(
                "B after the drop is B (within 2 a channel, the host test's bound)",
                worst <= 2,
            );
            expect(
                "the control dropped nothing",
                r.fifo_full == 1 && r.stalls == 0,
            );
            faked
        };
        // The rig's stream again, after a reset.
        g.e.reset(rt::delay_us);
        let _ = store32_volatile(g.mem.0, map::FENCE.at as usize, 0);
        cpu::drain_write_buffer();
        let _ = show_screen(&mut g.e.regs);
        log!("FF: FF2 {checks} checks, {bad} BAD ({faked} status reads faked)");
        let mut l2 = heapless_name();
        let _ = write!(l2, "{bad} BAD OF {checks}");
        clock_card(g, ["RUN FF DONE", l2.as_str(), ""], 0xFF10_6020, 5);
    }

    /// Run FF (see [`FIFO_FULL`]).
    fn fifo_full(g: &mut Rig) {
        ff1(g);
        ff2(g);
        log!("FF: done");
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
        rp_image::on_crash(put_back);
        rt::init_mmu();
        let Some((mut regs, vram)) = rp_image::take_mbx() else {
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
        // The CPU's own clocks and SDRAM timing as this run found them
        // (asked when the 37.9 ms 3D at Windows Mobile's clock was taken
        // for a slow state: is it the PXA's clock Windows Mobile chose at
        // its boot? It wasn't, run K), and the core clock counted.
        {
            let raw = rp_hal_pxa27x::clocks::read();
            log!(
                "PXA clocks: CCCR 0x{:08x} CCSR 0x{:08x} CLKCFG 0x{:x} MDREFR 0x{:08x} MDCNFG 0x{:08x}; core {} kHz counted",
                raw.cccr,
                raw.ccsr,
                raw.clkcfg,
                raw.mdrefr,
                raw.mdcnfg,
                rp_hal_pxa27x::clocks::measure_core_khz(10)
            );
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
            } else if TARGETS2 {
                "P2 (the target formats' follow-up: two more shapes, the dither matrix per layout, what each layout stores for alpha)"
            } else if TARGETS_WATCH {
                "P2W (watched: a rendered 24-bit picture shown as RGB888, RGB565 plain and dithered, ARGB8888)"
            } else if TARGETS {
                "P (the 3D core's target formats: 0x650 codes 0-15 and bits 4-31, each read back and decoded)"
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
            } else if FIFO_FULL {
                "FF (the 2D FIFO full: filled for real, then the 2D renderer with the level read as full)"
            } else if LONG_COUNT {
                "P2L (ADR-0023's long count: 32-bit renders under a dithered RGB565 screen, between 32-bit controls)"
            } else if SPECKS2 {
                "P2Y (what P2X's errors need: the CPU's reads, the display's fetch at 32 bits, 16 bits and stopped)"
            } else if SPECKS {
                "P2X (P2W's white specks: the picture rendered 5 times a format, checked and compared)"
            } else if RESOLVE {
                "T3 (the write-out downsample's cost: a full-screen render at each 0x644)"
            } else if FETCH {
                "T (the display's fetch against the engines: the plane at 32 bits, 16 bits, stopped; then 0x644's orientation bits mapped)"
            } else if FINGERPRINT {
                "W (the fingerprint: every PXA and 2700G register named, then the speed at Windows Mobile's clock)"
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
        } else if TARGETS2 {
            target_formats2(&mut g);
        } else if TARGETS_WATCH {
            target_watch(&mut g);
        } else if TARGETS {
            target_formats(&mut g);
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
        } else if FIFO_FULL {
            fifo_full(&mut g);
        } else if LONG_COUNT {
            long_count(&mut g);
        } else if SPECKS2 {
            specks2(&mut g);
        } else if SPECKS {
            specks(&mut g);
        } else if RESOLVE {
            resolve(&mut g);
        } else if FETCH {
            fetch(&mut g);
        } else if FINGERPRINT {
            fingerprint(&mut g);
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
        back_to_loader()
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
        back_to_loader()
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
        back_to_loader()
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
        back_to_loader()
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

    /// Run W (see [`FINGERPRINT`]).
    #[inline(never)]
    fn fingerprint(g: &mut Rig) {
        log!(
            "W: the fingerprint: every whitelisted PXA register, the CPU's coprocessor state, every named 2700G register, then the speed at Windows Mobile's clock, an MBX core reset and the speed again, then W2: G4c's ending (65 and 74.75 MHz with the cores at divide 1, Windows Mobile's clock put back), the speed and every 2700G register again"
        );
        let mut v = [0u32; diag::COUNT];
        diag::snapshot(&mut v);
        let mut i = 0;
        for (group, regs) in diag::GROUPS {
            let mut line = heapless_name();
            let _ = write!(line, "W PXA {group}:");
            for reg in *regs {
                if line.n > 80 {
                    log!("{}", line.as_str());
                    line = heapless_name();
                    let _ = write!(line, "W PXA {group}:");
                }
                let _ = write!(line, " {} 0x{:x}", reg.name, v[i]);
                i += 1;
            }
            log!("{}", line.as_str());
        }
        let cp = diag::coprocessors();
        log!(
            "W CPU: ID 0x{:08x} control 0x{:08x} aux 0x{:x} CPAR 0x{:x} CLKCFG 0x{:x}",
            cp.id,
            cp.control,
            cp.aux,
            cp.cpar,
            cp.clkcfg
        );
        dump_all(&mut g.e.regs, "W: every named 2700G register");
        let found = (g.e.regs.read32(clock::COREPLL), diag::msc1());
        if found.0 != ClockSpeed::WM.pll || found.1 != 0x2484_2481 {
            log!(
                "W: not Windows Mobile's clock and bus timing (COREPLL 0x{:x}, MSC1 0x{:x}): no speed measured",
                found.0,
                found.1
            );
            return;
        }
        let Some(mut c) = clock_rig(g) else {
            log!("W: no memory for the speed test");
            return;
        };
        clock_card(g, ["RUN W", "FINGERPRINT", "MEASURING"], 0xFF20_2040, 1);
        let base = clock_check(g, &mut c, true, 2);
        clock_log(WM_STEP, Some(0), &base);
        // Then the whole MBX core reset, as the renderer's hang recovery
        // does (`reset_core`): runs G4b and G4c ran faster at this clock
        // only after a recovered 2D hang (3D 34.3 against 37.9 ms, the read
        // 31.8 against 38.1, the 2D fill 6.18 against 6.54): something
        // in the MBX block using memory in the state the run starts in?
        let mut r3d = [[0u32; 3]; 2];
        let mut read = [[0u32; 3]; 2];
        for pass in 0..2 {
            if pass == 1 {
                let (st, en) = (
                    g.e.regs.read32(MBX + reg::STATUS),
                    g.e.regs.read32(MBX + reg::ENABLE),
                );
                g.e.reset(rt::delay_us);
                log!(
                    "W: the MBX core reset (status 0x{st:08x} enable 0x{en:08x} before; status 0x{:08x} enable 0x{:08x} after)",
                    g.e.regs.read32(MBX + reg::STATUS),
                    g.e.regs.read32(MBX + reg::ENABLE)
                );
            }
            for k in 0..3 {
                let r = clock_check(g, &mut c, false, 4 + 2 * (3 * pass + k) as u32);
                clock_log(WM_STEP, Some(0), &r);
                r3d[pass][k] = r.render_us;
                read[pass][k] = r.read_us;
            }
        }
        log!(
            "W: at Windows Mobile's clock (cores / 2; every run starts at 37.9 ms; 34.3 ms seen only after G4c's ending): 3D {} / {} / {} us and read {} / {} / {} us as found; after the MBX core reset 3D {} / {} / {} us and read {} / {} / {} us",
            r3d[0][0],
            r3d[0][1],
            r3d[0][2],
            read[0][0],
            read[0][1],
            read[0][2],
            r3d[1][0],
            r3d[1][1],
            r3d[1][2],
            read[1][0],
            read[1][1],
            read[1][2]
        );
        // W2: G4c's ending, the only way the faster state has come so far
        // (G4b and G4c: a step with the cores at divide 1 at 74.75 MHz or
        // below, then Windows Mobile's clock put back, the first control
        // BAD with a 2D hang, then 3D 34.3 ms with every register read
        // back as Windows Mobile's). Every named register dumped after, to
        // diff against the dump above.
        clock_card(
            g,
            ["RUN W", "G4C'S ENDING", "65 + 74.75 MHZ / 1"],
            0xFF20_2040,
            1,
        );
        let mut seed = 0x100;
        for s in [
            ClockStep {
                m: 20,
                n: 4,
                p: 0,
                div: 1,
                cl3: None,
                restart: false,
            },
            ClockStep {
                m: 23,
                n: 4,
                p: 0,
                div: 1,
                cl3: None,
                restart: false,
            },
        ] {
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
            let _ = clock_control(g, &mut c, seed, "after the step");
            seed += 2;
        }
        let mut after = [[0u32; 2]; 3];
        for a in &mut after {
            let r = clock_check(g, &mut c, false, seed);
            seed += 2;
            clock_log(WM_STEP, Some(0), &r);
            *a = [r.render_us, r.read_us];
        }
        clock_readback(g);
        log!(
            "W2: after G4c's ending at Windows Mobile's clock: 3D {} / {} / {} us, read {} / {} / {} us",
            after[0][0],
            after[1][0],
            after[2][0],
            after[0][1],
            after[1][1],
            after[2][1]
        );
        dump_all(
            &mut g.e.regs,
            "W2: every named 2700G register after G4c's ending",
        );
        clock_rig_free(g, c);
        clock_card(g, ["RUN W", "DONE", ""], 0xFF10_6020, 3);
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
            "Q: the 3D render at Windows Mobile's clock (cores / 2; every run starts at 37.9 ms; 34.3 ms seen only after G4c's ending): before {} us; after 108.33 CL2 {} us; after the memory restart {} us; after 108.33 CL3 with the restart {} us",
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
        back_to_loader()
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
        back_to_loader()
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
        back_to_loader()
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

    /// On a crash, first (rp-image's crash order): run G's Windows Mobile
    /// clock and bus timing back.
    fn put_back() {
        if CLOCK && let Some((mut regs, _)) = rp_hal_x51v::devices::emergency_mbx() {
            clock_restore(&mut regs);
        }
    }

    /// Every clean return: the SRAM copy cleared, back to the loader (which
    /// closes the card log: its closing line marks a run that came back).
    fn back_to_loader() -> ! {
        rp_hal_x51v::devices::kept_stop();
        rp_image::finish()
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}
