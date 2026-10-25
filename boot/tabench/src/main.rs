//! Test image (the performance gate's P4, docs/design/p4-ta-speed.md step
//! 4): the tile accelerator against our CPU-built lists, under the loader
//! at the clock it leaves (Windows Mobile's, 99.67 MHz / 2: ADR-0035 as
//! decided for P4's first run), into a 16-bit target off screen (ADR-0034:
//! the display keeps reading the safe region's screen at `0x0`, where the
//! step is shown from the first moment).
//!
//! Six content sets (100, 1,000 and 5,000 textured quads of 16 x 16 and of
//! 64 x 64 pixels, scattered by a fixed sequence), five frames of each way:
//!
//! 1. **the CPU binner** (`rp-mbx3d-render::binner`): the first frame bins
//!    and renders, the four after it render the same scene again;
//! 2. **the TA fed by CPU stores as Dell's GLES driver feeds it:** 7-word
//!    vertices, the state block and a strip of four a quad;
//! 3. **state once,** the strips after it;
//! 4. **state once and one triangle list** for the batch;
//! 5. **the same list moved by DMA** in pieces the FIFO has room for, the
//!    CPU free meanwhile;
//! 6. **Dell's form with `0x85C`'s scene-end bit off for the body** (audit
//!    C78, reminder R37).
//!
//! Logged for each: the CPU's time building or feeding the frame, the TA's
//! time, the render's, the words to the chip, the FIFO's deepest level
//! (ADR-0026 T8's evidence), the parameter memory used; every frame's
//! picture hashed against the way's first (repeatable) and the first
//! against the binner's (the same picture?). A TA that never says done is
//! recorded and the run moves on (Q2 A; C53: after a TA hang the TA may
//! stay hung, which the later TA ways then show); a render past 2 s resets
//! the core. The 6-word vertex isn't tried: which state word selects it is
//! unread (R38), and a guessed one could hang the TA for the ways after it.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed with the verdict.
pub const MARKER: &str = "RETROPANE-TABENCH";

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    #[path = "../isolate.rs"]
    mod isolate;

    /// Run 6 instead of run 1 (`RP_TABENCH_ISOLATE=1`; device build
    /// `RP_TABENCH_ISOLATE=1 cargo xtask codegen-variant rp-tabench isolate`).
    const ISOLATE: bool = option_env!("RP_TABENCH_ISOLATE").is_some();

    use rp_drv_blit2d::{MBX, reg, reset_core};
    use rp_drv_mbx_display::clock::{self, MBXCLK, with_3d};
    use rp_drv_mbx_display::font;
    use rp_drv_mbx_display::{Display, PixelFormat as PlaneFormat, Plane};
    use rp_drv_mbx3d::{self as m3, ta};
    use rp_drv_pxa_keypad::{Keypad, Pressed};
    use rp_hal_api::{DmaEngine, RegisterBus};
    use rp_hal_mem::words_as_bytes_mut;
    use rp_hal_mem::{bytes_as_pixels_mut, load32_volatile, store32_volatile, words_as_bytes};
    use rp_hal_pxa27x::{Mmio, cpu, rt};
    use rp_hal_x51v::devices::RunLog;
    use rp_mbx3d_render::binner::{self, Binned, Scratch};
    use rp_mbx3d_render::{
        Draw, LocalMem, Paint, Picture, PixelFormat, Target, Texture, fixed, format, upload,
    };
    use rp_vram_map::{POOL, Pool};

    use super::MARKER;

    use rp_image::crash;

    rp_image::entry!(main);

    macro_rules! log {
        ($($t:tt)*) => {{ let _ = writeln!(RunLog, $($t)*); }};
    }

    const W: u32 = 480;
    const H: u32 = 640;
    /// The off-screen target: 480 x 640 RGB565, outside the safe region.
    const TARGET: u32 = 0x0020_0000;
    const PICTURE: Picture = Picture::packed(TARGET, W as u16, H as u16, PixelFormat::Rgb565);
    const FULL: Target = Target {
        picture: PICTURE,
        x: 0,
        y: 0,
    };
    /// Words of the target.
    const TARGET_WORDS: usize = (W * H / 2) as usize;
    /// The parameter memory manager's block (16 KiB aligned).
    const PM_TABLE: u32 = 0x000F_C000;
    /// The parameter memory: 4 MiB, below the manager's 8 MiB reach.
    const PM: u32 = 0x0030_0000;
    const PM_SIZE: u32 = 0x0040_0000;
    const _: () = assert!((PM + PM_SIZE) as usize <= m3::PM_REACH);
    /// The binner's region array.
    const REGIONS: u32 = 0x0070_0000;
    /// The TA's tail buffer (128 KiB aligned) and region array.
    const TAIL: u32 = 0x0072_0000;
    const TA_REGIONS: u32 = 0x0074_0000;
    /// The TA window's physical address (the register window `0x0FFE0000`
    /// plus [`ta::PORT`]), for the DMA.
    const TA_WINDOW: u32 = 0x0FFE_0000 + ta::PORT as u32;

    const BIN: binner::Layout = binner::Layout {
        params: PM,
        params_size: PM_SIZE,
        regions: REGIONS,
        w: W as u16,
        h: H as u16,
    };
    /// The 3D stage on the TA's region array.
    const FROM_TA: binner::Layout = binner::Layout {
        regions: TA_REGIONS,
        ..BIN
    };
    const TA: ta::Layout = ta::Layout {
        tail: TAIL,
        tail_bytes: 0x1_0000,
        regions: TA_REGIONS,
        width: W,
        height: H,
    };

    /// The content sets: (quads, side in pixels).
    const SETS: [(usize, i32); 6] = [
        (100, 16),
        (1_000, 16),
        (5_000, 16),
        (100, 64),
        (1_000, 64),
        (5_000, 64),
    ];
    const MOST: usize = 5_000;
    const FRAMES: usize = 5;
    /// Binner list entries the largest set can need (5,000 64 x 64 quads:
    /// up to 9 x 5 tiles each).
    const ENTRIES: usize = MOST * 45;
    /// The staging buffer for the DMA way: the largest list stream.
    const STAGING: usize = (9 + MOST * 6 * ta::TEX_VERTEX_WORDS + 1).next_multiple_of(8);

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Way {
        Binner,
        TaDell,
        TaOnce,
        TaList,
        TaDma,
        TaDellBit7Off,
    }

    const WAYS: [Way; 6] = [
        Way::Binner,
        Way::TaDell,
        Way::TaOnce,
        Way::TaList,
        Way::TaDma,
        Way::TaDellBit7Off,
    ];

    impl Way {
        const fn name(self) -> &'static str {
            match self {
                Self::Binner => "CPU BINNER",
                Self::TaDell => "TA, DELL'S FEED",
                Self::TaOnce => "TA, STATE ONCE",
                Self::TaList => "TA, ONE LIST",
                Self::TaDma => "TA, LIST BY DMA",
                Self::TaDellBit7Off => "TA, DELL, 85C BIT 7 OFF",
            }
        }

        const fn shape(self) -> ta::Shape {
            match self {
                Self::TaOnce => ta::Shape {
                    batching: ta::Batching::Once,
                    ..ta::Shape::DELL
                },
                Self::TaList | Self::TaDma => ta::Shape {
                    batching: ta::Batching::Once,
                    primitive: ta::Primitive::List,
                    ..ta::Shape::DELL
                },
                _ => ta::Shape::DELL,
            }
        }
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

    fn us_since(t0: u32) -> u32 {
        rt::ticks().wrapping_sub(t0) / 13 * 4
    }

    /// How long the TA has from its start to done, the feeding included
    /// (5,000 quads are ~0.8 MB of stream: ~25 ms of bus).
    const TA_LIMIT_US: u32 = 1_000_000;

    const CLOCK: ta::Clock = ta::Clock {
        now: rt::ticks,
        ticks_per_ms: rt::TICK_HZ / 1000,
    };

    /// One frame's numbers.
    #[derive(Clone, Copy, Default)]
    struct Frame {
        /// The CPU's time building (binner) or feeding (TA) the frame.
        cpu_us: u32,
        /// Of it, the CPU free while the DMA moved words.
        free_us: u32,
        /// The TA's time to done (TA ways).
        ta_us: u32,
        /// The render's time; 0 if it never finished.
        render_us: u32,
        /// Words to the chip (objects, lists and regions; or the stream).
        words: u32,
        /// The FIFO's deepest level, its status reads (TA ways).
        level_max: u32,
        level_reads: u32,
        /// The TA said done (always true for the binner).
        ta_done: bool,
        /// The TA's FIFO stayed full (its status).
        stuck: Option<u32>,
        hash: u32,
    }

    struct Rig {
        regs: Mmio,
        mem: Vram,
        screen: Plane,
        tex: Texture,
        /// The SDRAM pool's pieces.
        starts: &'static mut [u32],
        entries: &'static mut [u16],
        objects: &'static mut [u32],
        reference: &'static mut [u32],
        staging: &'static mut [u32],
        /// Each quad's top left corner, packed ([`pack`]).
        xy: &'static mut [u32],
        /// The set's quads' side.
        side: i32,
        dma: Option<rp_hal_pxa27x::dma::Dma>,
        /// Renders past 2 s (core resets), TA ways that never said done.
        render_hangs: u32,
        ta_hangs: u32,
    }

    impl Rig {
        /// The core reset and the parameter memory manager set up again;
        /// true when it said ready. Always both (run 2: set up again without
        /// a reset, the manager handed the TA pages outside its range; our
        /// renderer's `bring_up` and `rp-glesfeat`'s clean start reset first
        /// too).
        fn pm_init(&mut self) -> bool {
            reset_core(&mut self.regs, rt::delay_us);
            let zeros = [0u32; 64];
            for k in 0..16 * 1024 / 256 {
                self.mem.store((PM_TABLE + 256 * k) as usize, &zeros);
            }
            self.mem.sync();
            let en = self.regs.read32(MBX + reg::ENABLE);
            self.regs.write32(MBX + reg::ENABLE, en | m3::PM_READY);
            self.regs.write32(MBX + reg::CLEAR, m3::PM_READY);
            for (off, v) in m3::pm_init_table(PM, PM_SIZE, PM_TABLE) {
                self.regs.write32(MBX + off, v);
            }
            let t0 = rt::ticks();
            let mut ok = false;
            while us_since(t0) < 100_000 {
                if self.regs.read32(MBX + reg::STATUS) & m3::PM_READY != 0 {
                    ok = true;
                    break;
                }
            }
            self.regs.write32(MBX + reg::CLEAR, m3::PM_READY);
            self.regs.write32(MBX + reg::ENABLE, en);
            ok
        }

        /// The 3D stage on `l`'s region array into the target; its time,
        /// or 0 if it didn't finish in 2 s (then the core is reset and set
        /// up again).
        fn render(&mut self, l: &binner::Layout) -> u32 {
            let en = self.regs.read32(MBX + reg::ENABLE);
            self.regs.write32(MBX + reg::CLEAR, m3::RENDER_DONE);
            self.regs.write32(MBX + reg::ENABLE, en | m3::RENDER_DONE);
            let t0 = rt::ticks();
            for (off, v) in binner::start_table(l, FULL) {
                self.regs.write32(MBX + off, v);
            }
            let mut us = 0;
            while us_since(t0) < 2_000_000 {
                if self.regs.read32(MBX + reg::STATUS) & m3::RENDER_DONE != 0 {
                    us = us_since(t0).max(1);
                    break;
                }
            }
            self.regs.write32(MBX + reg::CLEAR, m3::RENDER_DONE);
            self.regs.write32(MBX + reg::ENABLE, en);
            if us == 0 {
                self.render_hangs += 1;
                reset_core(&mut self.regs, rt::delay_us);
                let ok = self.pm_init();
                log!("    render past 2 s: core reset, memory manager ready again: {ok}");
            }
            us
        }

        /// FNV-1a of the target's words.
        fn hash(&self) -> u32 {
            let mut h = 0x811C_9DC5u32;
            for k in 0..TARGET_WORDS {
                let w = self.mem.load(TARGET as usize + 4 * k).unwrap_or(0);
                h = (h ^ w).wrapping_mul(0x0100_0193);
            }
            h
        }

        /// The target kept as the set's reference.
        fn keep(&mut self) {
            for k in 0..TARGET_WORDS {
                self.reference[k] = self.mem.load(TARGET as usize + 4 * k).unwrap_or(0);
            }
        }

        /// Words of the target unlike the kept reference.
        fn off(&self) -> u32 {
            (0..TARGET_WORDS)
                .filter(|&k| {
                    self.mem.load(TARGET as usize + 4 * k).unwrap_or(0) != self.reference[k]
                })
                .count() as u32
        }

        fn bin(&mut self, n: usize) -> Result<Binned, rp_mbx3d_render::Error> {
            let (xy, side, tex) = (&*self.xy, self.side, self.tex);
            binner::bin_with(
                &mut self.mem,
                &BIN,
                Some(BLACK),
                n,
                |k| draw(xy[k], side, tex),
                Scratch {
                    starts: &mut *self.starts,
                    entries: &mut *self.entries,
                    objects: &mut *self.objects,
                },
            )
        }

        /// One TA frame of `quads` in `way`: our background first (a black
        /// strip over the whole target: run 4 found the GLES clear's object
        /// drawing nothing and blanking every tile it lies in, audit C79),
        /// then the quads; the 3D stage on the TA's regions
        /// if it said done.
        fn ta_frame(&mut self, way: Way, n: usize) -> Frame {
            let mut f = Frame::default();
            // Every frame from a fresh memory manager: the TA's pages
            // given back (5,000 64 x 64 quads take most of it).
            if !self.pm_init() {
                log!("    memory manager not ready");
            }
            let _ = ta::clear(self.mem.0, &TA);
            cpu::drain_write_buffer();
            let state = ta::State {
                tsp: ta::TSP_DELL_TEXTURED,
                isp: m3::ISP_ALWAYS,
                tex: self.tex.words(),
                clip: ta::clip(W, H),
            };
            let shape = way.shape();
            let clear = background_strip();
            let side = self.side;
            let t0 = rt::ticks();
            let mut free_us = 0;
            let fed = if way == Way::TaDma {
                // The list's words staged in SDRAM first (the CPU's work
                // the DMA frees it from comes after: the feeding).
                let mut staged = 0;
                let xy = &*self.xy;
                for item in ta::quads_with(state, shape, n, |k| ta_quad(xy[k], side)) {
                    if let ta::Item::Word(w) = item {
                        self.staging[staged] = w;
                        staged += 1;
                    }
                }
                let built = us_since(t0);
                let t1 = rt::ticks();
                let (fed, free) = self.feed_dma(&clear, staged);
                free_us = free;
                f.cpu_us = built + us_since(t1);
                fed
            } else {
                let mut feed = ta::Feed::start(&mut self.regs, &TA, CLOCK, TA_LIMIT_US);
                if way == Way::TaDellBit7Off {
                    feed.scene_end_in_body(false);
                }
                let _ = ta::draws(&[&clear])
                    .chain(ta::quads_with(state, shape, n, |k| {
                        ta_quad(self.xy[k], side)
                    }))
                    .all(|i| feed.put(i));
                let fed = feed.finish();
                f.cpu_us = fed.feed_us;
                fed
            };
            f.free_us = free_us;
            f.ta_us = fed.ta_us;
            f.words = fed.words;
            f.level_max = fed.level_max;
            f.level_reads = fed.level_reads;
            f.ta_done = fed.done;
            f.stuck = fed.stuck;
            if fed.done {
                f.render_us = self.render(&FROM_TA);
            } else {
                self.ta_hangs += 1;
            }
            f
        }

        /// The background by the CPU, then the staged list's `n` words by DMA in
        /// pieces of whole 32-byte lines no larger than the FIFO's room or
        /// the window's rest (the CPU takes 8 words across the window's
        /// wrap), its end by the CPU. Returns the feed and the CPU's free
        /// time while the DMA moved words.
        fn feed_dma(&mut self, clear: &[u32], n: usize) -> (ta::Fed, u32) {
            let mut free = 0;
            let Some(mut dma) = self.dma.take() else {
                log!("    no DMA channel");
                return (ta::Fed::default(), 0);
            };
            let mut feed = ta::Feed::start(&mut self.regs, &TA, CLOCK, TA_LIMIT_US);
            let mut ok = ta::draws(&[clear]).all(|i| feed.put(i));
            let mut at = 0;
            // When the FIFO last had room for a piece.
            let mut roomy = rt::ticks();
            while ok && at < n {
                let window = (ta::PORT_SIZE - feed.window_offset()) / 4;
                let left = n - at;
                if window < 8 || left < 8 {
                    // Across the wrap or the last few: 8 (or the rest) by
                    // the CPU, keeping the staging's place on whole lines.
                    for &w in &self.staging[at..at + left.min(8)] {
                        ok &= feed.put(ta::Item::Word(w));
                    }
                    at += left.min(8);
                    continue;
                }
                let room = feed.room() as usize;
                let piece = room.min(window).min(left) & !7;
                if piece == 0 {
                    if us_since(roomy) > 200_000 {
                        log!(
                            "    the FIFO had no room for a piece for 200 ms: stopped at word {at}"
                        );
                        ok = false;
                    }
                    continue;
                }
                roomy = rt::ticks();
                let src = &words_as_bytes(&self.staging[at..at + piece]);
                let dst = TA_WINDOW + feed.window_offset() as u32;
                if !dma.arm_copy_to_device_memory(src, dst) {
                    log!("    the DMA refused a piece at word {at}");
                    ok = false;
                    break;
                }
                let t0 = rt::ticks();
                let done = loop {
                    match dma.done() {
                        Some(r) => break r,
                        None if us_since(t0) > 200_000 => {
                            dma.abort();
                            log!("    a DMA piece not done after 200 ms: stopped");
                            break false;
                        }
                        None => {}
                    }
                };
                free += us_since(t0);
                if !done {
                    ok = false;
                    break;
                }
                feed.advanced(piece as u32);
                at += piece;
            }
            if ok {
                ok = feed.put(ta::Item::End);
            }
            let _ = ok;
            let fed = feed.finish();
            self.dma = Some(dma);
            (fed, free)
        }
    }

    const BLACK: u32 = 0xFF00_0000;

    /// Our TA background: one strip of four over the whole target, black,
    /// untextured (`rp-glesfeat` G2's form: the state block `0x10000053`
    /// with Dell's smooth TSP, x, y, z 1.0, 1/w 1.0 and a colour a corner),
    /// in place of the GLES clear, whose colourless object the 3D stage drew
    /// nothing from in run 4 (audit C79). The binner's background is the
    /// same black.
    fn background_strip() -> [u32; 26] {
        let mut s = [0u32; 26];
        s[..6].copy_from_slice(&[
            0x1000_0053,
            0x2260_0E80,
            m3::ISP_ALWAYS,
            ta::clip(W, H),
            0,
            ta::STRIP,
        ]);
        let one = 1.0f32.to_bits();
        let f = |v: i32| fixed::f32_bits(v, 0);
        for (k, (x, y)) in [(0, 0), (W as i32, 0), (0, H as i32), (W as i32, H as i32)]
            .into_iter()
            .enumerate()
        {
            s[6 + 5 * k..11 + 5 * k].copy_from_slice(&[f(x), f(y), one, one, BLACK]);
        }
        s
    }

    /// A quad's top left corner packed in a word: `x + 64`, `y + 64` (the
    /// quads reach up to half their side past the screen's edges).
    const fn pack(x: i32, y: i32) -> u32 {
        (x + 64) as u32 | ((y + 64) as u32) << 16
    }

    const fn unpack(p: u32) -> (i32, i32) {
        ((p & 0xFFFF) as i32 - 64, (p >> 16) as i32 - 64)
    }

    /// `n` quads of `side` pixels scattered by a fixed sequence (some past
    /// the screen's edges): 4 bytes each, the quads themselves made as
    /// they are wanted ([`draw`], [`ta_quad`]).
    fn scatter(xy: &mut [u32], n: usize, side: i32) {
        let mut s = 0x2545_F491u32;
        let mut next = |m: u32| {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (s >> 8) % m
        };
        for p in &mut xy[..n] {
            let x = next(W + side as u32) as i32 - side / 2;
            let y = next(H + side as u32) as i32 - side / 2;
            *p = pack(x, y);
        }
    }

    /// The binner's quad at `p`: the whole texture, untinted.
    fn draw(p: u32, side: i32, tex: Texture) -> Draw {
        let (x, y) = unpack(p);
        Draw::rect(x, y, side, side, Paint::texture(tex))
    }

    /// The TA's quad at `p`, the same corners (whole pixels made floats by
    /// bit operations: no soft-float).
    fn ta_quad(p: u32, side: i32) -> ta::Quad {
        let (x, y) = unpack(p);
        let f = |v: i32| f32::from_bits(fixed::f32_bits(v, 0));
        ta::Quad {
            x0: f(x),
            y0: f(y),
            x1: f(x + side),
            y1: f(y + side),
            z: f(0),
            u0: f(0),
            v0: f(0),
            u1: f(1),
            v1: f(1),
            argb: 0xFFFF_FFFF,
        }
    }

    /// Two lines in a dark band across the top of the loader's screen (the
    /// safe region's), white text at 3x (`rp-mved`'s label).
    fn label(vram: &mut [u8], screen: &Plane, lines: [&str; 2]) {
        const BAND: u32 = 60;
        const SCALE: u32 = 3;
        let bpp = screen.format.bytes() as u32;
        if !matches!(screen.format, PlaneFormat::Rgb565 | PlaneFormat::Argb8888)
            || screen.offset + screen.stride * BAND as usize > vram.len()
        {
            return;
        }
        let (bg, fg) = if bpp == 2 {
            (0x1082_1082, 0xFFFF_FFFF)
        } else {
            (0xFF10_1020, 0xFFFF_FFFF)
        };
        let at = |x: u32, y: u32| screen.offset + y as usize * screen.stride + (x * bpp) as usize;
        for y in 0..BAND {
            for x in (0..u32::from(screen.width)).step_by((4 / bpp) as usize) {
                let _ = store32_volatile(vram, at(x, y), bg);
            }
        }
        for (k, text) in lines.iter().enumerate() {
            let y0 = 6 + 28 * k as u32;
            for (i, ch) in text.chars().enumerate() {
                let x0 = 12 + i as u32 * (font::WIDTH as u32 + 1) * SCALE;
                if x0 + font::WIDTH as u32 * SCALE > u32::from(screen.width) {
                    break;
                }
                for (gy, &bits) in font::glyph(ch).iter().enumerate() {
                    for gx in 0..font::WIDTH as u32 {
                        if bits & (1 << (font::WIDTH as u32 - 1 - gx)) == 0 {
                            continue;
                        }
                        for dy in 0..SCALE {
                            let y = y0 + gy as u32 * SCALE + dy;
                            let x = x0 + gx * SCALE;
                            if bpp == 2 {
                                let x = x & !1;
                                for w in 0..2 {
                                    let _ = store32_volatile(vram, at(x + 2 * w, y), fg);
                                }
                            } else {
                                for dx in 0..SCALE {
                                    let _ = store32_volatile(vram, at(x + dx, y), fg);
                                }
                            }
                        }
                    }
                }
            }
        }
        cpu::drain_write_buffer();
    }

    /// A formatting buffer on the stack.
    struct Line<'a>(&'a mut [u8], usize);

    impl Write for Line<'_> {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let n = s.len().min(self.0.len() - self.1);
            self.0[self.1..self.1 + n].copy_from_slice(&s.as_bytes()[..n]);
            self.1 += n;
            Ok(())
        }
    }

    /// "STEP n OF 6: <quads> x <side> PX" and the way.
    fn step(g: &mut Rig, n: usize, quads: usize, side: i32, way: Way) {
        let mut buf = [0u8; 40];
        let len = {
            let mut l = Line(&mut buf, 0);
            let _ = write!(l, "STEP {n} OF {}: {quads} OF {side} PX", SETS.len());
            l.1
        };
        let line = core::str::from_utf8(&buf[..len]).unwrap_or("STEP");
        let screen = g.screen;
        label(g.mem.0, &screen, [line, way.name()]);
    }

    /// Run 6's label: "STEP n OF m" and the way.
    fn step_line(g: &mut Rig, n: usize, m: usize, what: &str) {
        let mut buf = [0u8; 40];
        let len = {
            let mut l = Line(&mut buf, 0);
            let _ = write!(l, "RUN 6, STEP {n} OF {m}:");
            l.1
        };
        let line = core::str::from_utf8(&buf[..len]).unwrap_or("STEP");
        let screen = g.screen;
        label(g.mem.0, &screen, [line, what]);
    }

    /// The pool's pieces: binner scratch, the reference picture, the DMA
    /// staging (whole cache lines).
    /// The SDRAM pool's pieces.
    struct Sdram {
        starts: &'static mut [u32],
        entries: &'static mut [u16],
        objects: &'static mut [u32],
        reference: &'static mut [u32],
        staging: &'static mut [u32],
        xy: &'static mut [u32],
    }

    /// The pool cut into its pieces (the staging first: whole cache lines).
    fn sdram() -> Option<Sdram> {
        let ram = rp_hal_x51v::devices::take_ram_pool()?;
        let (staging, rest) = ram.split_at_mut(STAGING);
        // Room for a 32-bit picture (run 2's control).
        let (reference, rest) = rest.split_at_mut(TARGET_WORDS * 2);
        let (starts, rest) = rest.split_at_mut(2408);
        let (objects, rest) = rest.split_at_mut(MOST);
        let (xy, rest) = rest.split_at_mut(MOST);
        let (entries, _) = rest.split_at_mut(ENTRIES / 2);
        let entries = bytes_as_pixels_mut(words_as_bytes_mut(entries))?;
        Some(Sdram {
            starts,
            entries,
            objects,
            reference,
            staging,
            xy,
        })
    }

    /// A 64 x 64 RGB565 texture: a gradient with a checker, so a wrong
    /// texel or a shifted quad changes the picture.
    fn texture(mem: &mut Vram, buf: &mut [u32]) -> Option<Texture> {
        let texels = bytes_as_pixels_mut(words_as_bytes_mut(&mut buf[..64 * 64 / 2]))?;
        for (i, t) in texels.iter_mut().enumerate() {
            let (x, y) = ((i % 64) as u16, (i / 64) as u16);
            let check = if (x / 8 + y / 8) % 2 == 0 { 0x0010 } else { 0 };
            *t = (x >> 1) << 11 | y << 5 | check;
        }
        upload(&mut Pool::new(POOL), mem, 3, format::RGB565, texels)
    }

    fn report(way: Way, frames: &[Frame], first_off: Option<u32>) {
        let f = frames[0];
        let same = frames[1..]
            .iter()
            .filter(|x| x.hash == f.hash && x.render_us != 0)
            .count();
        let (mut rmin, mut rmax) = (u32::MAX, 0);
        let (mut cmin, mut cmax) = (u32::MAX, 0);
        for x in frames {
            if x.render_us != 0 {
                rmin = rmin.min(x.render_us);
                rmax = rmax.max(x.render_us);
            }
            cmin = cmin.min(x.cpu_us);
            cmax = cmax.max(x.cpu_us);
        }
        log!(
            "  {}: TA done {}{}; CPU {} us first, {cmin}-{cmax} us all (free during DMA {} us); TA {} us; render {} us first, {rmin}-{rmax} us; {} words ({} bytes); FIFO deepest {} words, {} status reads; frames 2-5 as the first: {same} of {}; first frame's words unlike the binner's: {}",
            way.name(),
            frames.iter().filter(|x| x.ta_done).count(),
            f.stuck.map_or(Default::default(), |_| " (FIFO stuck)"),
            f.cpu_us,
            f.free_us,
            f.ta_us,
            f.render_us,
            f.words,
            4 * f.words,
            f.level_max,
            f.level_reads,
            frames.len() - 1,
            first_off.map_or(-1, |o| o as i64),
        );
    }

    fn main() -> ! {
        rt::init_mmu();
        let Some((regs, vram)) = rp_image::take_mbx() else {
            crash(format_args!("no 2700G"))
        };
        let mut display = Display::new(regs);
        let (screen, _) = display.adopt(vram.len());
        label(
            vram,
            &screen,
            ["RP-TABENCH: P4, TA VS CPU LISTS", "STEP 0 OF 6: SETTING UP"],
        );
        let mut regs = display.into_inner();
        let clk0 = regs.read32(MBXCLK);
        log!(
            "{MARKER}: P4, the tile accelerator against the CPU binner (docs/design/p4-ta-speed.md), MBXCLK 0x{clk0:x}, at the loader's clock, into a 16-bit target off screen"
        );
        let Some(ram) = sdram() else {
            crash(format_args!("no SDRAM pool"))
        };
        clock::restore(&mut regs, with_3d(clk0));
        rt::delay_us(1_000);
        let mut mem = Vram(vram);
        let Some(tex) = texture(&mut mem, ram.staging) else {
            crash(format_args!("no room for the texture"))
        };
        let mut g = Rig {
            regs,
            mem,
            screen,
            tex,
            starts: ram.starts,
            entries: ram.entries,
            objects: ram.objects,
            reference: ram.reference,
            staging: ram.staging,
            xy: ram.xy,
            side: 16,
            dma: rt::take_dma(rp_hal_pxa27x::dma_map::SD),
            render_hangs: 0,
            ta_hangs: 0,
        };
        let ready = g.pm_init();
        log!(
            "memory manager ready: {ready}; DMA channel: {}",
            g.dma.is_some()
        );
        if ISOLATE {
            log!(
                "RUN 6 (audit C81): why TA scenes' renders hang, the memory manager's set-up and the waits"
            );
            let (right, all) = isolate::run6(&mut g);
            let (rh, th) = (g.render_hangs, g.ta_hangs);
            log!(
                "{MARKER}: DONE - run 6: {right} of {all} TA frames like the binner's, {rh} renders past 2 s, {th} TA frames never done"
            );
            finish(g, screen, clk0);
        }
        background_check(&mut g);
        for (s, &(n, side)) in SETS.iter().enumerate() {
            scatter(g.xy, n, side);
            g.side = side;
            log!(
                "SET {} of {}: {n} textured quads of {side} x {side}",
                s + 1,
                SETS.len()
            );
            let mut binned = false;
            for way in WAYS {
                step(&mut g, s + 1, n, side, way);
                let mut frames = [Frame::default(); FRAMES];
                let mut first_off = None;
                for (k, fr) in frames.iter_mut().enumerate() {
                    rt::check_in();
                    *fr = if way == Way::Binner {
                        let mut f = Frame {
                            ta_done: true,
                            ..Frame::default()
                        };
                        if k == 0 {
                            let t0 = rt::ticks();
                            match g.bin(n) {
                                Ok(b) => {
                                    f.cpu_us = us_since(t0);
                                    f.words = b.words();
                                    log!(
                                        "    binned: {} entries, longest list {} quads, {} object words, {} list words, {} region words, parameter memory {} bytes",
                                        b.entries,
                                        b.most,
                                        b.object_words,
                                        b.list_words,
                                        b.region_words,
                                        b.params_used
                                    );
                                    binned = true;
                                }
                                Err(e) => log!("    binner: {e:?}"),
                            }
                        }
                        if binned {
                            f.render_us = g.render(&BIN);
                        }
                        f
                    } else {
                        g.ta_frame(way, n)
                    };
                    fr.hash = g.hash();
                    if k == 0 {
                        if way == Way::Binner {
                            g.keep();
                        } else if binned && fr.render_us != 0 {
                            first_off = Some(g.off());
                        }
                    }
                }
                report(way, &frames, first_off);
            }
        }
        let (rh, th) = (g.render_hangs, g.ta_hangs);
        log!("{MARKER}: DONE - {rh} renders past 2 s, {th} TA frames never done");
        finish(g, screen, clk0);
    }

    /// Our TA background alone, once: the target's words not black after
    /// it (0 wanted: every TA way's picture stands on it).
    fn background_check(g: &mut Rig) {
        if !g.pm_init() {
            log!("    memory manager not ready");
        }
        let _ = ta::clear(g.mem.0, &TA);
        cpu::drain_write_buffer();
        let mut feed = ta::Feed::start(&mut g.regs, &TA, CLOCK, TA_LIMIT_US);
        let _ = ta::draws(&[&background_strip()]).all(|i| feed.put(i));
        let fed = feed.finish();
        let us = if fed.done { g.render(&FROM_TA) } else { 0 };
        let not_black = (0..TARGET_WORDS)
            .filter(|&k| g.mem.load(TARGET as usize + 4 * k).unwrap_or(1) != 0)
            .count();
        log!(
            "TA background alone (a black strip, in place of the GLES clear: audit C79): TA done {}, render {us} us, {not_black} of {TARGET_WORDS} words not black",
            fed.done
        );
    }

    /// The results screen until a button (10 s at most), the core and the
    /// clock put back, back to the loader.
    fn finish(mut g: Rig, screen: Plane, clk0: u32) -> ! {
        label(g.mem.0, &screen, ["RP-TABENCH: DONE", "RESULTS IN THE LOG"]);
        let keypad = arm_keypad();
        hold_until_button(keypad, 10_000_000);
        reset_core(&mut g.regs, rt::delay_us);
        clock::restore(&mut g.regs, clk0);
        rp_image::finish()
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
