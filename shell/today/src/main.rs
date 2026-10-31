//! Test image (ADR-0021): the device side of `rp-today` (see the library's
//! documentation): input, the tickless loop, the latency log.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

#[cfg(target_os = "none")]
mod image {
    use core::cell::RefCell;
    use core::fmt::Write;

    use rp_draw::Frame;
    use rp_draw::Renderer as _;
    use rp_drv_mbx_display::Display;
    use rp_drv_mbx_display::layers::Snapshot;
    use rp_drv_pxa_gpio::{Gpio, GpioPin};
    use rp_drv_pxa_keypad::Keypad;
    use rp_drv_pxa_keypad::keys::{Keys, Kind, Timing};
    use rp_drv_pxa_ssp::Ssp;
    use rp_drv_pxa_uart::PxaUart;
    use rp_drv_tsc2046::Tsc2046;
    use rp_gfx::layer::{Layer, PlaneFormat};
    use rp_gfx::render::CpuRenderer;
    use rp_graphics::{ClientId, Graphics, Handle, LayerState, Source, Step, Window};
    use rp_hal_mem::{load32_volatile, store32_volatile};
    use rp_hal_pxa27x::soc::irq;
    use rp_hal_pxa27x::{Mmio, cpu, pmu, rt};
    use rp_hal_x51v::MBX_VRAM_SIZE;
    use rp_hal_x51v::devices::RunLog;
    use rp_mbx_render::{Layout, LocalMem, Mbx2dRenderer};
    use rp_mbx3d_render::Clock;
    use rp_shell::{Action, Input, Key, Shell, Time};
    use rp_touch_cal::{Calibration, Filter, Pen, Sample};
    use rp_vram_map::{POOL, Pool};

    use rp_today::{Depth, H, Latency, W, WallScratch, clock};

    use rp_image::crash;

    rp_image::entry!(main);

    /// Microseconds from OSCR0 (3.25 MHz), kept running across its wrap:
    /// the graphics service's clock.
    struct TickClock {
        last: u32,
        us: u32,
        rest: u32,
    }

    impl Clock for TickClock {
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

    // The image's long-lived objects, in `.bss` (counted by the build's
    // budget) instead of `main`'s frame: as locals they made it 22 KiB deep
    // and an older build overflowed its stack (2026-10-17).
    static TWO_D: rt::Slot<Mbx2dRenderer<Mmio, Vram>> = rt::Slot::new();
    static GFX: rt::Slot<Graphics<'static, Mmio, Vram>> = rt::Slot::new();
    static SHELL: rt::Slot<Shell> = rt::Slot::new();
    static FRAME: rt::Slot<Frame> = rt::Slot::new();
    /// Touch to flush, then its parts: landing to verdict, frame + queueing,
    /// queue to flip; then the loop's period while the pen is down (the
    /// replay's: the first 256 of its ~530 count towards the median).
    static LATENCY: rt::Slot<[Latency; 5]> = rt::Slot::new();
    /// One movement's frames (an entrance, a drag, a fling: frames less
    /// than [`BURST_GAP_MS`] apart): CPU, queue to flip, and the period
    /// between frames, summed up in one run-log line when it ends (the
    /// first device run's per-frame lines filled the 64 KiB log).
    static BURST: rt::Slot<[Latency; 3]> = rt::Slot::new();
    /// The wallpaper's conversion rows (the 16-bit comparison's dither).
    static WALL: rt::Slot<WallScratch> = rt::Slot::new();
    // The weather pictures' descriptions (their pixels are in the RAM pool).
    static ICON_IMGS: rt::Slot<[rp_draw::Image; rp_today::ICON_COUNT]> = rt::Slot::new();
    // The radar's rain cells' descriptions (their pixels are in the RAM pool).
    static RADAR_IMGS: rt::Slot<[rp_draw::Image; 3]> = rt::Slot::new();
    /// The radar's base map, packed by the build (the Night look; Ink is
    /// made from it), unpacked at start into the RAM pool.
    static MAP_PACKED: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/radar.zz"));
    /// The wallpaper this image was built with (`RP_TODAY_WALLPAPER`).
    const WALL_NAME: &str = env!("RP_TODAY_WALLPAPER_NAME");

    /// Touch screen chip select and pen interrupt (docs/hw/touch.md).
    const TS_CS: u8 = 24;
    const TS_PENIRQ: u8 = 94;
    /// OS timer ticks per millisecond.
    const TICKS_MS: u32 = 3250;
    /// Frames further apart than this end a movement (its summary line).
    const BURST_GAP_MS: u64 = 250;
    /// An answer to a touch slower than this gets its own log line.
    const SLOW_ANSWER_US: u32 = 40_000;
    /// A frame (after the first, which builds the background: ~54 ms)
    /// taking more CPU than this gets its own log line.
    const SLOW_FRAME_US: u32 = 10_000;
    /// Samples the recorder keeps (~47 bytes each with the early settling
    /// values, in the 64 KiB run log: ~8 s of pen-down at ~7 ms a sample).
    const RECORD: u32 = 1150;
    /// The sleep between samples while a touch is in progress: 5 ms, with
    /// ~2 ms samples (X settles 32 conversions, not 256) a sample every ~7
    /// ms, the rate the filter's per-sample thresholds were proven at on
    /// the device (2026-10-14: 1 ms sleeps and ~6 ms samples; no tap
    /// pressed far). Shorter samples first, a faster rate only once the
    /// filter is checked at it (docs/hw/touch.md, "Faster samples").
    const PEN_DOWN_SLEEP_MS: u32 = 5;

    /// The touch replay (`RP_TODAY_REPLAY`, `rp_today::replay`): the
    /// recording instead of the pen, its numbers on a `TR` line for the
    /// ratchet, the run ending by itself. The recorder is off in it (its
    /// lines go to the card as they are logged, a sector write each:
    /// quality audit C58, C59), unless `RP_TODAY_REPLAY_RECORD` keeps it
    /// on to measure what those lines cost.
    const REPLAY: bool = option_env!("RP_TODAY_REPLAY").is_some();
    const REPLAY_RECORD: bool = option_env!("RP_TODAY_REPLAY_RECORD").is_some();

    /// The profile (`RP_TODAY_PROFILE`, with the replay; R81 item 1, the
    /// owner's "measure first"): the performance monitor around each hot
    /// path ([`Zone`]), where its cycles go (work, waiting on data, full
    /// write buffers, instruction fetch) and why (misses a thousand
    /// instructions), one `PROFILE` line a zone at the replay's end. Off:
    /// no counter is touched.
    const PROFILE: bool = option_env!("RP_TODAY_PROFILE").is_some();

    /// A profiled hot path: each span counted under the next of
    /// [`pmu::SETS`] in turn (three counters, six events: multiplexed over
    /// the replay's repeated work). Spans never nest ([`pmu::start`] resets
    /// the counters).
    struct Zone {
        tally: pmu::Tally,
        next: usize,
    }

    impl Zone {
        const fn new() -> Self {
            Self {
                tally: pmu::Tally {
                    spans: [0; 2],
                    cycles: [0; 2],
                    events: [[0; 3]; 2],
                },
                next: 0,
            }
        }

        /// `f`, counted when [`PROFILE`] is on.
        fn run<R>(&mut self, f: impl FnOnce() -> R) -> R {
            if !PROFILE {
                return f();
            }
            let set = self.next % pmu::SETS.len();
            self.next += 1;
            let span = pmu::start(pmu::SETS[set]);
            let r = f();
            self.tally.add(set, pmu::read(span));
            r
        }
    }

    /// The wallpaper, packed by the build (sheet today-image-room.md: 460
    /// KB of planes in ~140 KB); unpacked at start into the RAM pool.
    static WALLPAPER_PACKED: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/wallpaper.zz"));

    /// The plane path (sheet weather-gpu-movement.md, "The plane path
    /// build, as built"; ADR-0034 C4 as changed, L2): the Weather pages on
    /// the page layer, moved by repointing the video plane in the blanking.
    /// `RP_TODAY_2D` builds the 2D renderer's way instead, to compare.
    const PLANES: bool = option_env!("RP_TODAY_2D").is_none();
    /// The tallest page picture's rows (Hourly's, 1,332).
    const PIC_ROWS: usize = rp_shell::picture_rows(1) as usize;
    /// The pages' 2700G memory: three pictures and one being rebuilt (~3.9
    /// MiB of the pool's ~8.4).
    const PAGES_QUOTA: u32 = 6 << 20;

    /// A page's picture and the key of the draw list it was drawn from.
    type Page = Option<(Handle, u32)>;

    /// Page `tab`'s draw list's key, band by band (nothing drawn).
    fn page_key(shell: &mut Shell, f: &mut Frame, tab: u8) -> u32 {
        let rows = rp_shell::picture_rows(tab);
        let mut key = rp_today::KEY_START;
        let mut top = 0;
        while top < rows {
            let n = rp_today::PAGE_BAND.min(rows - top);
            shell.page_band(tab, f, top, n);
            key = rp_today::frame_key(f, key);
            top += n;
        }
        key
    }

    /// Page `tab` drawn once by the CPU, band by band, into its layer in
    /// SDRAM (straight alpha) and out as ARGB4444 with the shipped dither
    /// (V3), then written into a new picture of the pages' client: the
    /// picture and its draw list's key; the times logged.
    fn build_page(
        gfx: &mut Graphics<'static, Mmio, Vram>,
        pc: ClientId,
        shell: &mut Shell,
        cpu: &mut CpuRenderer<'_>,
        f: &mut Frame,
        tab: u8,
    ) -> Page {
        let t0 = rt::ticks();
        let rows = rp_shell::picture_rows(tab);
        let mut key = rp_today::KEY_START;
        let mut top = 0;
        while top < rows {
            let n = rp_today::PAGE_BAND.min(rows - top);
            shell.page_band(tab, f, top, n);
            key = rp_today::frame_key(f, key);
            let _ = cpu.draw(f);
            top += n;
            rt::check_in();
        }
        let drawn_us = rt::ticks().wrapping_sub(t0) / 13 * 4;
        let t1 = rt::ticks();
        let h = match gfx.picture(pc, W as u16, rows) {
            Ok(h) => h,
            Err(e) => {
                let _ = writeln!(RunLog, "page {tab}: no picture ({e:?})");
                return None;
            }
        };
        let bytes = cpu.plane();
        let mut row = [0u32; W / 2];
        for y in 0..usize::from(rows) {
            for (k, w) in row.iter_mut().enumerate() {
                let a = y * W * 2 + 4 * k;
                *w = u32::from_le_bytes([bytes[a], bytes[a + 1], bytes[a + 2], bytes[a + 3]]);
            }
            if gfx.write_picture(pc, h, (y * W * 2) as u32, &row).is_err() {
                let _ = gfx.free_picture(pc, h);
                return None;
            }
            if y % 64 == 0 {
                rt::check_in();
            }
        }
        let _ = writeln!(
            RunLog,
            "page {tab}: its picture (480 x {rows}, ARGB4444) drawn by the CPU in {drawn_us} us, written in {} us",
            rt::ticks().wrapping_sub(t1) / 13 * 4
        );
        Some((h, key))
    }

    /// The layer showing page picture `h` as the shell's view `v` has it.
    fn page_state(h: Handle, v: rp_shell::PageView) -> LayerState {
        LayerState {
            src: Source::Picture(h),
            win: Window {
                x: v.win.x,
                y: v.win.y,
                w: v.win.w,
                h: v.win.h,
                row: v.row,
                col: 0,
            },
        }
    }

    /// One refresh of an opening, as the display had it: the time, the
    /// display's signature of the picture it scanned out (`DSIG`), the plane
    /// registers read back (`GSCADR`, `VSCADR`, `VSADR`, `VSCTRL`,
    /// `SSSIZE`), and the window the code meant for it (x, y, w, h, row).
    #[derive(Clone, Copy, Default)]
    struct Refresh {
        ms: u32,
        sig: u32,
        regs: [u32; 5],
        meant: (u16, u16, u16, u16, u16),
    }

    impl Refresh {
        /// As ten words (kept in the RAM pool).
        fn words(&self) -> [u32; 10] {
            let m = self.meant;
            [
                self.ms,
                self.sig,
                self.regs[0],
                self.regs[1],
                self.regs[2],
                self.regs[3],
                self.regs[4],
                u32::from(m.0) | u32::from(m.1) << 16,
                u32::from(m.2) | u32::from(m.3) << 16,
                u32::from(m.4),
            ]
        }

        /// Back from [`Self::words`].
        fn from_words(w: &[u32]) -> Self {
            let lo = |v: u32| (v & 0xFFFF) as u16;
            Self {
                ms: w[0],
                sig: w[1],
                regs: [w[2], w[3], w[4], w[5], w[6]],
                meant: (lo(w[7]), lo(w[7] >> 16), lo(w[8]), lo(w[8] >> 16), lo(w[9])),
            }
        }
    }

    /// Refreshes kept per opening (~1.6 s at 60 Hz).
    const TRACE: usize = 96;

    /// Reveal's window over the app composed aside.
    fn reveal_state(r: rp_draw::Rect, shift: u16) -> LayerState {
        let (x, y, w, h, row, col) = rp_today::reveal_window(r, shift);
        LayerState {
            src: Source::Aside,
            win: Window {
                x,
                y,
                w,
                h,
                row,
                col,
            },
        }
    }

    /// Start of the clock, in local minutes since 1970 (from the build).
    fn start_minutes() -> i64 {
        let utc: i64 = env!("RP_TODAY_UTC_MIN").parse().unwrap_or(0);
        let off: i64 = env!("RP_TODAY_OFFSET_MIN").parse().unwrap_or(0);
        utc + off
    }

    /// Windows Mobile's virtual keys for the D-pad and Home (docs/hw/keypad.md).
    fn key_of(vk: u8) -> Option<Key> {
        Some(match vk {
            0x26 => Key::Up,
            0x28 => Key::Down,
            0x25 => Key::Left,
            0x27 => Key::Right,
            0x0D => Key::Enter,
            0xC4 => Key::Home,
            _ => return None,
        })
    }

    fn main() -> ! {
        rt::init_mmu();
        let Some((regs, vram)) = rp_image::take_mbx() else {
            crash(format_args!("no display"))
        };
        let mut d = Display::new(regs);
        let _ = d.adopt(MBX_VRAM_SIZE as usize);
        let before = Snapshot::take(d.regs());
        // R105: the state the picture is shown through, for the owner's
        // look check against the decided one (display-calibration.md,
        // 2026-10-22: the S-curve 1.75, saturation 1.2x): the build's
        // look, the display's gamma switch and table, the backlight.
        {
            use rp_drv_mbx_display::reg;
            use rp_hal_api::RegisterBus as _;
            let r = d.regs();
            let gsctrl = r.read32(reg::GSCTRL);
            let mut gam = [0u32; 17];
            for (k, g) in gam.iter_mut().enumerate() {
                *g = r.read32(reg::GFXGAM0 + 4 * k);
            }
            let pwm = rt::take_pwm0().map(|mut p| (p.read32(0), p.read32(4), p.read32(8)));
            let _ = writeln!(
                RunLog,
                "look: {} (curve 1.75, saturation {}/{}); GSCTRL {gsctrl:08X}, gamma table {}; GFXGAM {gam:06X?}; backlight PWM0 (CR, DCR, PCR) {pwm:X?}",
                if rp_draw::look::ON {
                    "the default picture"
                } else {
                    "Original (RP_PICTURE_ORIGINAL)"
                },
                rp_draw::look::SAT_NUM,
                rp_draw::look::SAT_DEN,
                if gsctrl & 1 << 26 != 0 { "on" } else { "off" },
            );
        }

        // ADR-0023: the 2D engine draws; no CPU layer in SDRAM.
        let Some(two_d) = TWO_D.put_with(|| {
            Mbx2dRenderer::new(
                d.into_inner(),
                Vram(vram),
                Layout::X51V,
                rt::ticks,
                TICKS_MS,
            )
        }) else {
            crash(format_args!("started twice"))
        };
        let Some(wall) = WALL.put_with(WallScratch::new) else {
            crash(format_args!("started twice"))
        };
        // The RAM pool (SDRAM outside the image area, unused by this image
        // otherwise): the weather pictures, then the wallpaper's planes.
        let Some(pool) = rp_hal_x51v::devices::take_ram_pool() else {
            crash(format_args!("no RAM pool"))
        };
        let (icon_px, rest) = pool.split_at_mut(rp_today::ICON_WORDS);
        let (wall_words, rest) = rest.split_at_mut(rp_today::WALLPAPER_BYTES.div_ceil(4));
        let (radar_px, rest) = rest.split_at_mut(rp_today::RADAR_WORDS);
        let (map_words, rest) = rest.split_at_mut(rp_today::MAP_BYTES.div_ceil(4));
        // The plane path's page drawing: the CPU's layer (ARGB8888) and the
        // picture it is flushed into (ARGB4444), the tallest page's size.
        let (layer_px, rest) = rest.split_at_mut(W * PIC_ROWS);
        let (plane_words, rest) = rest.split_at_mut(W * PIC_ROWS / 2);
        // The openings' trace, in the RAM pool (not the image's RAM budget).
        let (trace_words, _) = rest.split_at_mut(TRACE * 10);
        let plane_bytes = rp_hal_mem::words_as_bytes_mut(plane_words);
        let wall_bytes =
            &mut rp_hal_mem::words_as_bytes_mut(wall_words)[..rp_today::WALLPAPER_BYTES];
        let map_bytes = &mut rp_hal_mem::words_as_bytes_mut(map_words)[..rp_today::MAP_BYTES];
        let t0 = rt::ticks();
        if let Err(e) = rp_today::unpack_wallpaper(WALLPAPER_PACKED, wall_bytes) {
            crash(format_args!("wallpaper unpacking: {e:?}"))
        }
        let unpacked_us = rt::ticks().wrapping_sub(t0) / 13 * 4;
        let wallpaper: &'static [u8] = wall_bytes;
        // The radar's map: a damaged one leaves the radar on the wallpaper.
        let t0 = rt::ticks();
        let radar_map: Option<&'static [u8]> = match rp_today::unpack_map(MAP_PACKED, map_bytes) {
            Ok(()) => Some(map_bytes),
            Err(e) => {
                let _ = writeln!(RunLog, "radar map unpacking: {e:?}");
                None
            }
        };
        let _ = writeln!(
            RunLog,
            "radar map unpacked: {} -> {} bytes in {} us",
            MAP_PACKED.len(),
            rp_today::MAP_BYTES,
            rt::ticks().wrapping_sub(t0) / 13 * 4
        );
        let _ = writeln!(
            RunLog,
            "wallpaper unpacked: {} -> {} bytes in {unpacked_us} us",
            WALLPAPER_PACKED.len(),
            wallpaper.len()
        );
        let t0 = rt::ticks();
        rp_today::show_gpu(two_d, wallpaper, rt::delay_us, wall);
        let shown_us = rt::ticks().wrapping_sub(t0) / 13 * 4;
        // The 16-bit comparison's fact: how the engine widens a 16-bit
        // source into 32 bits (in the pool, which this 2D-only image leaves
        // unused).
        match rp_today::probe_widen(two_d, POOL.at as usize) {
            Some(w) => {
                let _ = writeln!(
                    RunLog,
                    "probe: the engine widens RGB565 to 32 bpp: by repeating the top bits {}, by zeros {}, either {}, neither {} (first: {:?})",
                    w.replicated, w.zero_filled, w.either, w.other, w.first_other
                );
            }
            None => {
                let _ = writeln!(RunLog, "probe: the engine didn't finish the 16-bit copy");
            }
        }
        // The graphics service (ADR-0030) owns the engines from here; 2D
        // only, so no 3D state is lent (ThreeD: ~5 KiB).
        let Some(gfx) = GFX.put_with(|| Graphics::new(two_d, Pool::new(POOL), None)) else {
            crash(format_args!("started twice"))
        };
        let mut gclock = TickClock {
            last: rt::ticks(),
            us: 0,
            rest: 0,
        };
        let _ = writeln!(
            RunLog,
            "gpu: wallpaper ({WALL_NAME}) converted to 32 bpp and shown in {shown_us} us"
        );
        // The comparison's frame after a depth switch: its time is logged.
        let mut compare_frame = false;
        // The pictures unpacked once into the RAM pool (SDRAM outside the
        // image area, unused by this image): R129, the lean rule.
        let mut icons = match ICON_IMGS.put_with(|| [rp_today::NO_IMAGE; rp_today::ICON_COUNT]) {
            Some(imgs) => rp_today::icons_into(icon_px, imgs),
            None => rp_shell::Icons::NONE,
        };
        let t0 = rt::ticks();
        if let Some(imgs) = RADAR_IMGS.put_with(|| [rp_today::NO_IMAGE; 3]) {
            icons.radar = rp_today::radar_into(radar_px, imgs);
        }
        let _ = writeln!(
            RunLog,
            "radar cells drawn in {} us",
            rt::ticks().wrapping_sub(t0) / 13 * 4
        );
        let (Some(shell), Some(frame)) = (
            SHELL.put_with(|| Shell::new(rp_today::shell_fonts(), icons, true)),
            FRAME.put_with(Frame::new),
        ) else {
            crash(format_args!("started twice"))
        };

        // The plane path: the pages' client and their pictures, drawn once
        // now (rebuilt only when a page's draw list changes).
        let pc = if PLANES {
            gfx.client(PAGES_QUOTA)
        } else {
            None
        };
        let Some(cpu_layer) = Layer::new(layer_px, W as u32, PIC_ROWS as u32, W) else {
            crash(format_args!("page layer"))
        };
        let mut cpu =
            CpuRenderer::new(cpu_layer, plane_bytes, W * 2, PlaneFormat::Argb4444Dithered);
        let mut pages: [Page; 3] = [None; 3];
        if let Some(pc) = pc {
            shell.use_planes(true);
            // The clock first: the Now page's Sunset bar follows it.
            shell.set_time(clock::time_at(start_minutes()));
            for (tab, p) in pages.iter_mut().enumerate() {
                *p = build_page(gfx, pc, shell, &mut cpu, frame, tab as u8);
            }
            if pages.iter().any(Option::is_none) {
                // Without every picture, the 2D way throughout.
                shell.use_planes(false);
                let _ = writeln!(RunLog, "plane path: off (a page picture failed)");
            }
        }
        let _ = writeln!(
            RunLog,
            "plane path: {}",
            if PLANES && pages.iter().all(Option::is_some) {
                "on (the Weather pages on the page layer)"
            } else {
                "off (the 2D renderer draws every frame)"
            }
        );
        // The page the layer shows, a Reveal under way, a picture replaced
        // and given back once nothing shows it.
        let mut shown_tab: Option<u8> = None;
        let mut revealing = false;
        // Reveal's window has shown (its clock restarted then).
        let mut reveal_shown = false;
        // The app's screen as Reveal leaves it, composed ahead (its picture
        // and key), and whether the screen plane shows it now (handed back
        // to the renderer by the next frame).
        let mut chrome_pic: Option<(Handle, u32)> = None;
        let mut screen_on_pic = false;
        // The app composed ahead (its picture and key), whether to check it,
        // and the open under way's start (ms, commits).
        let mut app_pic: Option<(Handle, u32)> = None;
        let mut app_check = true;
        let (mut reveal_t0, mut reveal_c0) = (0u64, 0u32);
        // Openings so far (the glide's alternation).
        let mut opens = 0u32;
        // The opening's trace: refreshes recorded, the last refresh's
        // count (`DSIG` is published at line 3 of the next), and how many
        // refreshes to keep recording after the opening ends.
        let trace = trace_words;
        let (mut trace_n, mut trace_on, mut trace_tail) = (0usize, false, 0u32);
        let mut last_line = 0u32;
        let mut trace_due = false;
        let mut meant = (0u16, 0u16, 0u16, 0u16, 0u16);
        let mut retire: Option<Handle> = None;
        // The pages' draw lists to check against their pictures (a new
        // minute), rebuilt ahead while the screen is still.
        let mut pages_check = false;

        // Input: the keypad and the touch screen, interrupt-driven.
        let (Some(kp), Some(gpio), Some(ssp)) =
            (rt::take_keypad(), rt::take_gpio(), rt::take_ssp1())
        else {
            crash(format_args!("no keypad, GPIO or SSP"))
        };
        let mut keypad = Keypad::new(kp);
        keypad.enable();
        let gpio = RefCell::new(Gpio::new(gpio));
        // Windows Mobile leaves GPIO24 as SSP1's frame signal (function 2,
        // docs/hw/touch.md): then our chip-select writes do nothing and the
        // frame pulses per byte, so every reading is 0 (all taps at the top
        // left). Level and direction first, then a plain GPIO.
        let cs_found = gpio.borrow_mut().function(TS_CS);
        gpio.borrow_mut().make_output(TS_CS, true);
        gpio.borrow_mut().set_function(TS_CS, 0);
        gpio.borrow_mut().make_input(TS_PENIRQ);
        let mut ssp = Ssp::new(ssp);
        ssp.configure_spi(2_600_000);
        let mut touch = Tsc2046::new(
            ssp,
            GpioPin::new(&gpio, TS_CS),
            GpioPin::new(&gpio, TS_PENIRQ),
        );
        let (Some(mut keys_irq), Some(mut pen_irq)) = (
            rt::take_irq(irq::KEYPAD),
            rt::take_gpio_irq(TS_PENIRQ, rt::Edge::Falling),
        ) else {
            crash(format_args!("no IRQs"))
        };
        let mut key_events = Keys::new(Timing::DEFAULT);
        let _ = writeln!(
            RunLog,
            "touch: GPIO24 function {cs_found} as found, now a plain GPIO (chip select)"
        );
        // Samples recorded into the run log so far (see RECORD).
        let mut recorded = 0u32;
        // The pen interrupt fired since the pen was last up.
        let mut irq_seen = false;
        // Until the align screen runs: the rough default from the owner's
        // first taps (no settings store yet to keep a better one).
        let mut cal = Calibration::X51V_DEFAULT;

        // Time: the OS timer (3.25 MHz, wraps every 22 minutes) summed into
        // milliseconds since start.
        let mut last_ticks = rt::ticks();
        let mut sub_ms = 0u32;
        let mut now_ms = 0u64;
        let start_min = start_minutes();
        let mut minute_shown = -1i64;

        // Touch to flush, and the parts of a tap's answer, for their medians.
        let Some([latency, settle, queueing, to_flip, period]) = LATENCY.put_with(|| {
            [
                Latency::new(),
                Latency::new(),
                Latency::new(),
                Latency::new(),
                Latency::new(),
            ]
        }) else {
            crash(format_args!("started twice"))
        };
        let Some([burst_cpu, burst_engine, burst_period]) =
            BURST.put_with(|| [Latency::new(), Latency::new(), Latency::new()])
        else {
            crash(format_args!("started twice"))
        };
        // The movement in progress: when its first and last frames were
        // queued (ms), and its background pixels and rows written.
        let mut burst: Option<(u64, u64)> = None;
        // What is under the Weather app's page: the wallpaper (None) or the
        // radar map (Some(ink)).
        let mut backdrop_shown: Option<bool> = None;
        let (mut burst_bg_px, mut burst_rows, mut burst_reused) = (0u64, 0u32, 0u32);
        let mut frames = 0u32;
        let mut draw_us_worst = 0u32;
        // The CPU time of the frame last queued (for the slow-answer line).
        let mut last_frame_us = 0u32;
        // Samples to what the person meant (rp_touch_cal::Filter).
        let mut filter = Filter::new();
        // When the pen landed (OS timer): latency counts from the touch.
        let mut landed_ticks: Option<u32> = None;
        // Times the pen-detect line said "down" with no pressure behind it
        // (diagnostics: the line recovering slowly after a sample).
        let mut phantom = 0u32;
        // Touches found by the pen interrupt, and by the idle check.
        let (mut found_by_irq, mut found_by_poll) = (0u32, 0u32);
        let mut pending: Option<u32> = None; // when the input being answered was read
        // An answered input whose frame is with the engine, not yet shown.
        let mut showing: Option<u32> = None;
        // R115's chase (the owner, 2026-10-30: "2 more test to chase the
        // 7"): each 2D frame flipped for want of a copy window, kept for
        // the summary (no card write mid-run, C59): the scan line, its
        // rectangles (top, rows, pixels), the copies' band.
        let mut windowless = [(0u32, [(0u16, 0u16, 0u32); 3], 0usize, None::<(u32, u32)>); 16];
        let mut windowless_n = 0usize;
        let mut no_window_seen = 0u32;
        // When the frame in flight next wants a poll (the service's clock,
        // us): its flip's deadline, or the engine's next look (R115).
        let mut flip_due: Option<u32> = None;
        // The GPU renderer's costs (rp_mbx_render::FrameStats), worst seen.
        let (mut queue_worst, mut wait_worst, mut engine_worst, mut rebuilds) =
            (0u32, 0u32, 0u32, 0u32);
        // The replay: its steps, when the pause before the next touch ends
        // (ms), whether a touch is in progress, the last sample's time.
        let mut replay = rp_today::replay::steps(rp_today::replay::DATA).peekable();
        let mut replay_until: Option<u64> = None;
        let mut replay_touching = false;
        let mut replay_last: Option<u32> = None;

        // The profile's zones (see PROFILE).
        let (mut z_touch, mut z_filter, mut z_shell, mut z_compositor) =
            (Zone::new(), Zone::new(), Zone::new(), Zone::new());
        loop {
            rt::check_in();
            let t = rt::ticks();
            sub_ms += t.wrapping_sub(last_ticks);
            last_ticks = t;
            now_ms += u64::from(sub_ms / TICKS_MS);
            sub_ms %= TICKS_MS;

            // A frame the engine finished goes on the screen; the latency
            // counts to then (the flip), not to the end of queueing. First
            // thing after waking (R115): a flip waiting for the blanking is
            // due now, before this pass's pen sample (~2 ms) could carry the
            // scan past it (today-0013: touch to flush +4.3 ms median with
            // the poll after the sample).
            let step = gfx.poll(&mut gclock);
            flip_due = match step {
                Step::Wait(t) => Some(t),
                _ => None,
            };
            let nw = gfx.starts().flips_no_window;
            if nw != no_window_seen {
                no_window_seen = nw;
                if windowless_n < windowless.len() {
                    let line = gfx.starts().last_no_window_line.unwrap_or(0);
                    let band = gfx.copy_times();
                    let drawn = gfx.two_d().drawn();
                    let mut rects = [(0u16, 0u16, 0u32); 3];
                    for (o, r) in rects.iter_mut().zip(drawn) {
                        *o = (r.y, r.h, u32::from(r.w) * u32::from(r.h));
                    }
                    windowless[windowless_n] = (line, rects, drawn.len(), band);
                    windowless_n += 1;
                }
            }
            if flip_due.is_none()
                && let Some(at) = showing.take()
            {
                let total = rt::ticks().wrapping_sub(at) / 13 * 4;
                latency.add(total);
                let s = gfx.two_d().last;
                // A slow answer, taken apart (2026-10-14: one of 57 took
                // 101 ms on the device, every part's worst far less).
                // Not in the replay: a line mid-run is a card write in the loop (C59).
                if total > SLOW_ANSWER_US && !REPLAY {
                    let _ = writeln!(
                        RunLog,
                        "slow answer at {} ms: {total} us; its frame's CPU {last_frame_us} us, wait {} us, engine {} us",
                        now_ms, s.wait_us, s.engine_us
                    );
                }
                engine_worst = engine_worst.max(s.engine_us);
                to_flip.add(s.engine_us);
                if burst.is_some() {
                    burst_engine.add(s.engine_us);
                }
            }

            // The clock: once a minute.
            let minute = start_min + (now_ms / 60_000) as i64;
            if minute != minute_shown {
                minute_shown = minute;
                let time: Time = clock::time_at(minute);
                shell.set_time(time);
                pages_check = true;
                app_check = true;
            }
            // Movement by itself (an entrance, a fling, a rebound) marks
            // what changed; it is drawn below, not counted as an answer.
            shell.tick(now_ms as u32);

            // Buttons, as Windows Mobile programs see them.
            // The scan acknowledges the controller and takes its activity
            // latch with the held set, so a tap between two looks is
            // counted (key_events.missed), not lost.
            let key_event = keys_irq.take();
            let scan = keypad.scan();
            let pressed = scan.held;
            let mut back = false;
            key_events.update(rt::ticks() / TICKS_MS, scan, &mut |e| {
                if matches!(e.kind, Kind::Down | Kind::Repeat)
                    && let Some(k) = key_of(e.vk)
                    && shell.input(Input::Key(k)) == Action::BackToLoader
                {
                    back = true;
                }
            });
            if key_event {
                keys_irq.done();
            }

            // The pen: down and up are what the shell acts on.
            let pen_event = pen_irq.take() && !REPLAY;
            let read_at = rt::ticks();
            let (traced, raw) = if REPLAY {
                // A touch's samples one a pass, as the pen gives them; a
                // pause before the next touch, the pen up.
                let step = match replay.peek() {
                    Some(st) if st.wait_ms > 0 && !replay_touching => {
                        let until = *replay_until.get_or_insert(now_ms + u64::from(st.wait_ms));
                        if now_ms >= until {
                            replay_until = None;
                            replay.next()
                        } else {
                            None
                        }
                    }
                    Some(_) => replay.next(),
                    None => None,
                };
                match step {
                    Some(st) => {
                        // The real sample's work, its values not used.
                        let _ = z_touch.run(|| touch.convert_traced());
                        if replay_touching && let Some(last) = replay_last {
                            period.add(read_at.wrapping_sub(last) / 13 * 4);
                        }
                        replay_last = Some(read_at);
                        replay_touching = st.sample.is_some();
                        let raw = st.sample.map(|s| rp_drv_tsc2046::RawTouch {
                            x: s.x,
                            y: s.y,
                            z1: s.z1,
                            z2: 0,
                        });
                        (None, raw)
                    }
                    None => (None, None),
                }
            } else {
                let traced = if pen_event || filter.down() || touch.pen_down() {
                    z_touch.run(|| touch.sample_traced())
                } else {
                    None
                };
                (traced, traced.map(|(s, _)| s))
            };
            let sampled_at = rt::ticks();
            if raw.is_some() && !filter.down() {
                landed_ticks = Some(read_at);
            }
            irq_seen |= pen_event;
            // Whether a touch was in progress before this sample: its end
            // is recorded even when nothing was reported.
            let was_down = filter.down();
            let ev = z_filter.run(|| {
                filter.sample(
                    raw.map(|s| Sample {
                        x: s.x,
                        y: s.y,
                        z1: s.z1,
                    }),
                    (now_ms & 0xFFFF_FFFF) as u32,
                )
            });
            // Which found this touch: the pen interrupt, or the idle check
            // of the line (diagnostics: touch went dead after a hold on the
            // device, as if the interrupt were missed). Counted once a
            // touch, when it lands (2026-10-14: counted per sample before
            // the landing, the light-pressure samples of 39 touches read as
            // 382 touches found).
            if !was_down && filter.down() {
                if irq_seen {
                    found_by_irq += 1;
                } else {
                    found_by_poll += 1;
                }
            }
            if raw.is_none() {
                irq_seen = false;
            }
            let map = |x: u16, y: u16| cal.to_screen(x, y, (W as u16, H as u16));
            // The recorder (2026-10-13): every sample while the pen is down,
            // with the filter's verdict, into the run log, so the filter is
            // tuned on the PC against the owner's real taps instead of on
            // the device by guesswork. Up to RECORD samples (~25 s). The
            // sample that ends a touch is recorded whether or not the touch
            // was reported (2026-10-14: a lift before the press left no
            // line, so the replay ran two touches together).
            if (!REPLAY || REPLAY_RECORD)
                && (raw.is_some() || ev != Pen::Nothing || was_down)
                && recorded < RECORD
            {
                recorded += 1;
                let what = match ev {
                    Pen::Nothing => 'n',
                    Pen::Down(..) => 'D',
                    Pen::Move(..) => 'M',
                    Pen::Hold(..) => 'H',
                    Pen::Up(..) => 'U',
                };
                let ms = (now_ms & 0xFFFF_FFFF) as u32;
                match traced {
                    // The replay's (`RP_TODAY_REPLAY_RECORD`): as long a
                    // line as a recording's, from the replayed values.
                    _ if REPLAY => {
                        let _ = match raw {
                            Some(s) => writeln!(
                                RunLog,
                                "R {ms} {} {} {} 0 {what} 0 0 0 0 0",
                                s.x, s.y, s.z1
                            ),
                            None => writeln!(RunLog, "R {ms} - - - - {what}"),
                        };
                    }
                    // Then X and Y had they settled 16 conversions (32
                    // kept; X's 32/64/128 columns 0 now that it settles
                    // 32): how short a sample could be, from recordings.
                    Some((s, t)) => {
                        let [x16, x32, x64, x128] = t.x.early;
                        let _ = writeln!(
                            RunLog,
                            "R {ms} {} {} {} {} {what} {x16} {x32} {x64} {x128} {}",
                            s.x, s.y, s.z1, s.z2, t.y.early[0]
                        );
                    }
                    None => {
                        let _ = writeln!(RunLog, "R {ms} - - - - {what}");
                    }
                }
            }
            // The align screen takes stable raw samples; everything else,
            // points.
            let act = if shell.wants_raw() {
                match ev {
                    Pen::Down(rx, ry) | Pen::Move(rx, ry) | Pen::Hold(rx, ry) => {
                        shell.input(Input::Raw { rx, ry })
                    }
                    Pen::Up(..) => shell.input(Input::RawUp),
                    Pen::Nothing => Action::Nothing,
                }
            } else {
                match ev {
                    Pen::Down(rx, ry) => {
                        let (x, y) = map(rx, ry);
                        shell.input(Input::PenDown { x, y })
                    }
                    Pen::Up(rx, ry) => {
                        let (x, y) = map(rx, ry);
                        shell.input(Input::PenUp { x, y })
                    }
                    // Lists follow the pen; swipes are measured as it goes.
                    Pen::Move(rx, ry) => {
                        let (x, y) = map(rx, ry);
                        shell.input(Input::PenMove { x, y })
                    }
                    Pen::Hold(..) | Pen::Nothing => Action::Nothing,
                }
            };
            // Latency: the first answer to a touch counts from the touch
            // itself (the landing and the filter included); anything later
            // in the same touch (a hold, a move, the lift) from its sample.
            let since = match ev {
                Pen::Down(..) => landed_ticks.unwrap_or(read_at),
                _ => read_at,
            };
            // The touch's share: from the landing to the filter's verdict.
            if matches!(ev, Pen::Down(..)) {
                settle.add(read_at.wrapping_sub(since) / 13 * 4);
            }
            if matches!(ev, Pen::Up(..)) || raw.is_none() {
                landed_ticks = None;
            }
            match act {
                Action::BackToLoader => back = true,
                Action::Calibrated(c) => {
                    cal = c;
                    let _ = writeln!(
                        RunLog,
                        "calibrated: a {} b {} c {} d {} e {} f {} (screen = (a rx + b ry + c) / 65536, (d rx + e ry + f) / 65536)",
                        c.a, c.b, c.c, c.d, c.e, c.f
                    );
                    pending.get_or_insert(since);
                }
                Action::Redraw => {
                    pending.get_or_insert(since);
                }
                Action::WallpaperDepth { dithered } => {
                    // The frame in flight may be reading the wallpaper.
                    while matches!(gfx.poll(&mut gclock), Step::Wait(_)) {}
                    let t0 = rt::ticks();
                    let depth = if dithered { Depth::Rgb565 } else { Depth::Full };
                    rp_today::load_wallpaper(gfx.two_d(), wallpaper, depth, wall);
                    gfx.two_d().wallpaper_changed();
                    let _ = writeln!(
                        RunLog,
                        "compare: {WALL_NAME} at {}, {}: converted in {} us",
                        // Since ADR-0023's L1 the wallpaper is kept as
                        // RGB565: the other side is plain 16 bits (quality
                        // audit C48), no longer 32.
                        if dithered {
                            "16-bit dithered"
                        } else {
                            "16-bit plain (truncated)"
                        },
                        if shell.wallpaper_only() {
                            "wallpaper only"
                        } else {
                            "Today screen"
                        },
                        rt::ticks().wrapping_sub(t0) / 13 * 4
                    );
                    compare_frame = true;
                    pending.get_or_insert(since);
                }
                Action::Nothing => {}
            }

            // The replay's end: every step taken, every answer on the screen.
            if REPLAY
                && replay.peek().is_none()
                && showing.is_none()
                && pending.is_none()
                && !shell.needs_frame()
            {
                let (n, median, worst) = latency.summary();
                let (sn, smedian, sworst) = settle.summary();
                let (_, qmedian, qworst) = queueing.summary();
                let (_, fmedian, fworst) = to_flip.summary();
                let (pn, pmedian, pworst) = period.summary();
                let _ = writeln!(
                    RunLog,
                    // Every number named in full: the ratchet names each
                    // by the words before it (`cargo xtask perf`).
                    "TR touch replay{}: touch to flush median Some({median}), touch to flush worst Some({worst}); \
                     landing to verdict median Some({smedian}), landing to verdict worst Some({sworst}); \
                     frame and queueing median Some({qmedian}), frame and queueing worst Some({qworst}); \
                     queue to flip median Some({fmedian}), queue to flip worst Some({fworst}); \
                     pen-down period median Some({pmedian}), pen-down period worst Some({pworst}) \
                     (us; {n} answered, {sn} presses, {pn} periods)",
                    if REPLAY_RECORD {
                        " with the recorder"
                    } else {
                        ""
                    }
                );
                back = true;
            }

            if back {
                let (n, median, worst) = latency.summary();
                let (stalls, fifo_full) = (gfx.two_d().stalls, gfx.two_d().fifo_full);
                let _ = writeln!(
                    RunLog,
                    "touch to flush: {n} answered, median {median} us, worst {worst} us \
                     (promise: 50 ms); {frames} frames, worst draw {draw_us_worst} us; pen line down without pressure {phantom} times; \
                     touches found by the interrupt {found_by_irq}, by the idle check {found_by_poll}; engine stalls {} (hangs, each a reset), \
                     frames dropped on a full FIFO {}; button taps missed between scans {}",
                    stalls,
                    fifo_full,
                    key_events.missed()
                );
                let cm = gfx.commits();
                let _ = writeln!(
                    RunLog,
                    "plane path: {} commits (window moves and hand-offs), {} ending outside the plane window (lines 643-2), the last on line {:?}",
                    cm.done, cm.late, cm.last_line
                );
                let _ = writeln!(
                    RunLog,
                    "gpu frames: worst CPU queueing {queue_worst} us, worst wait for the previous frame and scan-out {wait_worst} us, worst queue-to-flip {engine_worst} us, background rebuilt {rebuilds} times; flips waited for the blanking {}, slipped a refresh {}; copied behind the scan {}, flipped as large {}, flipped with no copy window {}; copy times {:?} us",
                    gfx.starts().flips_waited,
                    gfx.starts().flips_slipped,
                    gfx.starts().two_d_behind,
                    gfx.starts().flips_large,
                    gfx.starts().flips_no_window,
                    gfx.copy_times()
                );
                for (line, rects, n, band) in &windowless[..windowless_n] {
                    let _ = writeln!(
                        RunLog,
                        "windowless copy: at line {line}, {n} rectangles (top, rows, pixels) {:?}, the copies' band {band:?} us",
                        &rects[..(*n).min(3)]
                    );
                }
                // Typical values (medians) of each part of a tap's answer.
                for (name, l) in [
                    ("touch landing to the filter's verdict", &mut *settle),
                    ("shell frame + CPU queueing", &mut *queueing),
                    ("queue to flip (engine, polled every ms)", &mut *to_flip),
                ] {
                    let (n, median, worst) = l.summary();
                    let _ = writeln!(
                        RunLog,
                        "  {name}: {n} times, median {median} us, worst {worst} us"
                    );
                }
                if PROFILE {
                    for (name, z) in [
                        ("touch sample (SPI)", &z_touch),
                        ("touch filter", &z_filter),
                        ("shell frame (layout and draw list)", &z_shell),
                        ("compositor (2D queueing)", &z_compositor),
                    ] {
                        let _ = writeln!(RunLog, "PROFILE {name}: {}", z.tally.split());
                    }
                }
                before.restore(&mut gfx.two_d().parts().0.regs);
                rp_image::finish()
            }

            // The radar's map under the page while Radar is open, the
            // wallpaper again after (the frame in flight may read it).
            let want = shell.backdrop().filter(|_| radar_map.is_some());
            if want != backdrop_shown {
                while matches!(gfx.poll(&mut gclock), Step::Wait(_)) {}
                let t0 = rt::ticks();
                rp_today::show_backdrop(
                    gfx.two_d(),
                    wallpaper,
                    want.zip(radar_map).map(|(ink, m)| (m, ink)),
                    wall,
                );
                let _ = writeln!(
                    RunLog,
                    "backdrop: {} in {} us",
                    match want {
                        Some(true) => "radar map, Ink",
                        Some(false) => "radar map, Night",
                        None => "wallpaper",
                    },
                    rt::ticks().wrapping_sub(t0) / 13 * 4
                );
                backdrop_shown = want;
            }

            // The plane path: what the page layer shows. A hand-off (the
            // layer on, off, another page, a page redrawn) is a commit with
            // this pass's frame; a move only repoints it, no engine.
            // (the layer, the frame kept aside, this pass's 2D frame drawn
            // with it)
            let mut commit: Option<(Option<LayerState>, bool, bool)> = None;
            if let Some(pc) = pc.filter(|_| pages.iter().all(Option::is_some)) {
                if !gfx.busy() && !gfx.commit_pending() {
                    if let Some((r, shift)) = shell.reveal_plane() {
                        if !gfx.aside_held() && !revealing {
                            // The app drawn whole aside, shown through the
                            // card's window. Opens alternate the glide (odd)
                            // and a still app (even), to tell apart the
                            // glide on this panel and the hand-over (the
                            // owner's run, 2026-10-30).
                            opens += 1;
                            shell.set_reveal_glide(opens % 2 == 1);
                            let (r, shift) = shell.reveal_plane().unwrap_or((r, shift));
                            let _ = writeln!(
                                RunLog,
                                "open {opens}: Reveal {}",
                                if opens % 2 == 1 {
                                    "with the glide (the app read 52 rows lower, easing up)"
                                } else {
                                    "held still (the window grows, the app in place)"
                                }
                            );
                            commit = Some(match app_pic {
                                // Composed ahead: the window at the next
                                // refresh, no 2D work on the tap.
                                Some((h, _)) => {
                                    let mut st = reveal_state(r, shift);
                                    st.src = Source::Picture(h);
                                    (Some(st), false, false)
                                }
                                // Not ready: composed now, aside.
                                None => (Some(reveal_state(r, shift)), true, true),
                            });
                            reveal_t0 = now_ms;
                            reveal_c0 = gfx.commits().done;
                            trace_n = 0;
                            trace_on = true;
                            trace_tail = 30;
                        }
                    } else if revealing
                        && let Some((ch, _)) = chrome_pic
                        && let Some(v) = shell.page_layer()
                        && let Some((h, _)) = pages[usize::from(v.tab)]
                    {
                        // Reveal done: the screen plane on the app's screen
                        // composed ahead and its page on the layer, in one
                        // commit at the next refresh, nothing drawn (the
                        // trace, 2026-10-31: drawing that screen during
                        // Reveal froze it two refreshes). The renderer's own
                        // screens come back with the next frame.
                        let _ = gfx.commit_screen(pc, ch, Some(page_state(h, v)));
                        shown_tab = Some(v.tab);
                        screen_on_pic = true;
                        shell.redraw_all();
                        let n = gfx.commits().done - reveal_c0;
                        let ms = (now_ms - reveal_t0).max(1);
                        let _ = writeln!(
                            RunLog,
                            "open: {now_ms} ms, Reveal done: {n} window moves in {ms} ms (~{} fps); the screen on its picture composed ahead, the page on the layer, one commit",
                            u64::from(n) * 1000 / ms
                        );
                        revealing = false;
                        reveal_shown = false;
                    } else if gfx.aside_held() {
                        // Reveal done: the app takes the screen (the same
                        // pixels), then its page goes onto the layer.
                        let _ = gfx.show_aside(pc, None);
                        let _ = writeln!(
                            RunLog,
                            "open: {now_ms} ms, Reveal done: the app handed to the screen"
                        );
                        revealing = false;
                        reveal_shown = false;
                        shown_tab = None;
                    } else {
                        if revealing {
                            // Reveal done over the picture composed ahead: the
                            // screen's chrome and the page layer in one commit
                            // (below), its frames counted.
                            let n = gfx.commits().done - reveal_c0;
                            let ms = (now_ms - reveal_t0).max(1);
                            let _ = writeln!(
                                RunLog,
                                "open: {now_ms} ms, Reveal done: {n} window moves in {ms} ms (~{} fps); the screen and the page in one commit next",
                                u64::from(n) * 1000 / ms
                            );
                            revealing = false;
                            reveal_shown = false;
                        }
                        let want = shell.page_layer();
                        let still = !(shell.animating() || filter.down());
                        if want.map(|v| v.tab) != shown_tab {
                            // The picture as built: nothing is checked or
                            // drawn on the tap (the first device run: 40-70
                            // ms of CPU here); checks run while still.
                            if let Some(v) = want {
                                if let Some((h, _)) = pages[usize::from(v.tab)] {
                                    commit = Some((Some(page_state(h, v)), false, true));
                                    shown_tab = Some(v.tab);
                                }
                            } else {
                                commit = Some((None, false, true));
                                shown_tab = None;
                            }
                            pages_check = true;
                        } else if want.is_some() && shell.needs_frame() && still {
                            // Something changed on screen: the pictures are
                            // checked once it is still.
                            pages_check = true;
                        }
                    }
                }
                // Moves: the window follows (staged; the latest wins).
                if commit.is_none() {
                    if shell.reveal_plane().is_some() {
                        if revealing && gfx.layer().is_some() && !reveal_shown {
                            // The app's window is up: Reveal's clock starts
                            // now, so it grows from the card rather than
                            // jumping in partway (the composing took ~100 ms
                            // of its 540 on the first device run).
                            reveal_shown = true;
                            shell.restart_entrance(now_ms as u32);
                            let _ = writeln!(
                                RunLog,
                                "open: {now_ms} ms, the app composed aside and its window up ({} commits so far)",
                                gfx.commits().done
                            );
                        }
                        // Only once its window is up: moves staged while the
                        // app was still composing showed it part-open, then
                        // the restart snapped it back to the card (the
                        // owner's second run, today-0022, his photos).
                        if reveal_shown && let Some((r, shift)) = shell.reveal_plane() {
                            let _ = gfx.move_layer(pc, reveal_state(r, shift).win);
                        }
                    } else if let Some(v) = shell.page_layer()
                        && shown_tab == Some(v.tab)
                        && let Some((h, _)) = pages[usize::from(v.tab)]
                    {
                        let st = page_state(h, v);
                        if gfx.layer().map(|l| l.win) != Some(st.win) {
                            let _ = gfx.move_layer(pc, st.win);
                        }
                    }
                }
                // Ahead of need, while still: a page not shown whose draw
                // list changed (the clock's minute) is rebuilt, one a pass,
                // so opening or turning to it never waits for a build.
                if pages_check
                    && commit.is_none()
                    && !shell.animating()
                    && !filter.down()
                    && !shell.needs_frame()
                    && !gfx.busy()
                    && !gfx.commit_pending()
                    && !gfx.aside_held()
                {
                    let stale = (0..3u8).find(|&t| {
                        pages[usize::from(t)].is_some_and(|(_, k)| k != page_key(shell, frame, t))
                    });
                    match stale {
                        // The page shown: a new picture, swapped in by a
                        // commit, the old one given back after it.
                        Some(t) if Some(t) == shown_tab => {
                            let i = usize::from(t);
                            let old = pages[i].take();
                            pages[i] = build_page(gfx, pc, shell, &mut cpu, frame, t);
                            match (pages[i], shell.page_layer()) {
                                (Some((h, _)), Some(v)) => {
                                    commit = Some((Some(page_state(h, v)), false, true));
                                    retire = old.map(|(h, _)| h);
                                }
                                _ => pages[i] = old,
                            }
                        }
                        Some(t) => {
                            let i = usize::from(t);
                            if let Some((h, _)) = pages[i].take() {
                                let _ = gfx.free_picture(pc, h);
                            }
                            pages[i] = build_page(gfx, pc, shell, &mut cpu, frame, t);
                        }
                        None => pages_check = false,
                    }
                }
                // The app composed ahead while the Today screen is still (its
                // key the draw list's: the header's clock makes it stale),
                // so a tap opens Reveal's window at the next refresh.
                if app_check
                    && commit.is_none()
                    && shell.screen() == rp_shell::Screen::Today
                    && !shell.animating()
                    && !filter.down()
                    && !shell.needs_frame()
                    && !gfx.busy()
                    && !gfx.commit_pending()
                    && !gfx.aside_held()
                {
                    app_check = false;
                    shell.weather_preview(frame);
                    let key = rp_today::frame_key(frame, rp_today::KEY_START);
                    if app_pic.is_none_or(|(_, k)| k != key) {
                        let h = match app_pic {
                            Some((h, _)) => Ok(h),
                            None => gfx.picture_rgb565(pc),
                        };
                        match h.and_then(|h| gfx.compose_into(pc, frame, h, &mut gclock).map(|_| h))
                        {
                            Ok(h) => app_pic = Some((h, key)),
                            Err(e) => {
                                let _ = writeln!(RunLog, "app picture: not composed ({e:?})");
                            }
                        }
                        // The app's screen (its chrome) on a later pass.
                        app_check = true;
                    } else {
                        // Then the app's screen as Reveal leaves it (the
                        // same frame reused: one more would cost the image
                        // its RAM budget).
                        shell.weather_chrome_preview(frame);
                        let key = rp_today::frame_key(frame, rp_today::KEY_START);
                        if chrome_pic.is_none_or(|(_, k)| k != key) {
                            let h = match chrome_pic {
                                Some((h, _)) => Ok(h),
                                None => gfx.picture_rgb565(pc),
                            };
                            match h.and_then(|h| {
                                gfx.compose_into(pc, frame, h, &mut gclock).map(|_| h)
                            }) {
                                Ok(h) => chrome_pic = Some((h, key)),
                                Err(e) => {
                                    let _ =
                                        writeln!(RunLog, "chrome picture: not composed ({e:?})");
                                }
                            }
                        }
                    }
                }
                // A replaced picture goes back once nothing shows it.
                if let Some(h) = retire
                    && !gfx.commit_pending()
                    && gfx.free_picture(pc, h).is_ok()
                {
                    retire = None;
                }
            }

            // The screen on a picture composed ahead: the next frame hands it
            // back to the renderer's own screen (drawn whole, flipped, the
            // layer kept), the same pixels.
            if screen_on_pic && commit.is_none() && !gfx.busy() && !gfx.commit_pending() {
                screen_on_pic = false;
                commit = Some((gfx.layer(), false, true));
            }

            // The opening's trace: once a refresh, after its signature is
            // published (line 3 of the next), what the display showed.
            if trace_on {
                use rp_hal_api::RegisterBus as _;
                let regs = &mut gfx.two_d().parts().0.regs;
                let line = regs.read32(rp_drv_mbx_display::reg::DVLNUM) & 0xFFF;
                // A new refresh began since the last look (the line went
                // back): sampled once its signature is published (line 3).
                if line < last_line {
                    trace_due = true;
                }
                let published = line >= rp_drv_mbx_display::sig::PUBLISHED_LINE;
                if trace_due && published && trace_n < TRACE {
                    trace_due = false;
                    let rd = |r: &mut Mmio, o: usize| r.read32(o);
                    let t = Refresh {
                        ms: now_ms as u32,
                        sig: rd(regs, rp_drv_mbx_display::sig::DSIG),
                        regs: [
                            rd(regs, rp_drv_mbx_display::reg::GSCADR),
                            rd(regs, rp_drv_mbx_display::layers::reg::VSCADR),
                            rd(regs, rp_drv_mbx_display::layers::reg::VSADR),
                            rd(regs, rp_drv_mbx_display::layers::reg::VSCTRL),
                            rd(regs, rp_drv_mbx_display::layers::reg::SSSIZE),
                        ],
                        meant,
                    };
                    trace[trace_n * 10..trace_n * 10 + 10].copy_from_slice(&t.words());
                    trace_n += 1;
                }
                last_line = line;
                if !revealing && !gfx.aside_held() {
                    trace_tail = trace_tail.saturating_sub(u32::from(published && trace_n > 0));
                }
                if trace_tail == 0 || trace_n == TRACE {
                    trace_on = false;
                    let _ = writeln!(
                        RunLog,
                        "trace of open {opens}: {trace_n} refreshes (ms, DSIG, GSCADR, VSCADR, VSADR, VSCTRL, SSSIZE; the window meant x,y,w,h,row)"
                    );
                    for w in trace[..trace_n * 10].chunks(10) {
                        let t = Refresh::from_words(w);
                        let _ = writeln!(
                            RunLog,
                            "T {} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} | {},{},{},{},{}",
                            t.ms,
                            t.sig,
                            t.regs[0],
                            t.regs[1],
                            t.regs[2],
                            t.regs[3],
                            t.regs[4],
                            t.meant.0,
                            t.meant.1,
                            t.meant.2,
                            t.meant.3,
                            t.meant.4
                        );
                    }
                }
            }
            if let Some(l) = gfx.layer() {
                meant = (l.win.x, l.win.y, l.win.w, l.win.h, l.win.row);
            } else {
                meant = (0, 0, 0, 0, 0);
            }

            // Draw what changed, timing it. One job at a time: with a frame
            // still in flight (its flip waiting for the blanking, R115) the
            // new one waits a pass, the pen sampled meanwhile, rather than
            // the CPU spinning until the flip. Nothing while a frame is held
            // aside (Reveal): it waits for the hand-over.
            if (commit.is_some() || shell.needs_frame() && !revealing)
                && !gfx.busy()
                && !gfx.aside_held()
            {
                let t0 = rt::ticks();
                let draw = commit.is_none_or(|(_, _, d)| d);
                if draw {
                    z_shell.run(|| shell.frame(frame));
                } else {
                    frame.clear();
                }
                if shell.screen() == rp_shell::Screen::Today {
                    app_check = true;
                }
                let _ = z_compositor.run(|| match (commit, pc) {
                    (Some((layer, aside, _)), Some(pc)) => {
                        let r = gfx.frame_commit(pc, frame, layer, aside, &mut gclock);
                        if r.is_ok() && layer.is_some_and(|l| matches!(l.src, Source::Picture(_))) && shell.reveal_plane().is_some() {
                            revealing = true;
                        }
                        if r.is_ok() && aside {
                            revealing = true;
                        }
                        let cm = gfx.commits();
                        let _ = writeln!(
                            RunLog,
                            "hand-off: {now_ms} ms, {} (the last commit on line {:?}, {} late so far)",
                            match (layer.map(|l| l.src), aside) {
                                (_, true) => "the app being composed aside for Reveal",
                                (Some(Source::Picture(_)), _) => "screen 0's chrome + a page on the layer",
                                _ => "the layer off, the page drawn on screen 0",
                            },
                            cm.last_line,
                            cm.late
                        );
                        if let Err(e) = r {
                            let _ = writeln!(RunLog, "plane path: a commit refused ({e:?})");
                        }
                        r
                    }
                    _ => gfx.frame(frame, &mut gclock),
                });
                let t1 = rt::ticks();
                let us = t1.wrapping_sub(t0) / 13 * 4;
                if core::mem::take(&mut compare_frame) {
                    let _ = writeln!(
                        RunLog,
                        "compare: the whole screen redrawn over it: CPU {us} us"
                    );
                }
                draw_us_worst = draw_us_worst.max(us);
                queueing.add(us);
                last_frame_us = us;
                let s = gfx.two_d().last;
                // The movement this frame belongs to (or starts).
                burst = match burst {
                    Some((first, last)) if now_ms - last <= BURST_GAP_MS => {
                        burst_period.add(((now_ms - last) * 1000) as u32);
                        Some((first, now_ms))
                    }
                    _ => {
                        *burst_cpu = Latency::new();
                        *burst_engine = Latency::new();
                        *burst_period = Latency::new();
                        (burst_bg_px, burst_rows, burst_reused) = (0, 0, 0);
                        Some((now_ms, now_ms))
                    }
                };
                burst_cpu.add(us);
                burst_bg_px += u64::from(s.background_px);
                burst_rows += s.rows_written;
                burst_reused += s.rows_reused;
                let moving = shell.animating() || burst_cpu.summary().0 > 1;
                if us > SLOW_FRAME_US && frames > 0 && !REPLAY && !moving {
                    let _ = writeln!(
                        RunLog,
                        "slow frame {} at {} ms: {us} us (queueing {} us, wait {} us, background {})",
                        frames + 1,
                        now_ms,
                        s.queue_us,
                        s.wait_us,
                        s.background
                    );
                }
                queue_worst = queue_worst.max(s.queue_us);
                wait_worst = wait_worst.max(s.wait_us);
                rebuilds += u32::from(s.background);
                frames += 1;
                if frames == 1 {
                    let _ = writeln!(RunLog, "{} first frame {us} us", rp_today::MARKER);
                    // The emulator check waits for this on the serial port.
                    let (stalls, fifo_full) = (gfx.two_d().stalls, gfx.two_d().fifo_full);
                    let _ = writeln!(
                        PxaUart::new(rt::emergency_uart()),
                        "{} (engine stalls {stalls}, FIFO full {fifo_full})",
                        rp_today::MARKER,
                    );
                }
                if let Some(at) = pending.take() {
                    // Answered by this frame: counted when it shows.
                    showing = Some(showing.unwrap_or(at).min(at));
                }
            }

            // A movement over (no frame for a while): its one summary line.
            if let Some((first, last)) = burst
                && now_ms - last > BURST_GAP_MS
                && showing.is_none()
                && !REPLAY
            {
                burst = None;
                let (n, cpu_med, cpu_worst) = burst_cpu.summary();
                if n > 1 {
                    let (_, eng_med, eng_worst) = burst_engine.summary();
                    let (_, per_med, per_worst) = burst_period.summary();
                    let fps10 = if per_med > 0 { 10_000_000 / per_med } else { 0 };
                    let _ = writeln!(
                        RunLog,
                        "movement at {first} ms: {n} frames in {} ms, period median {per_med} us (~{}.{} fps), worst {per_worst} us; CPU median {cpu_med} us, worst {cpu_worst} us; queue to flip median {eng_med} us, worst {eng_worst} us; background rebuilt {burst_bg_px} px, rows written {burst_rows}, reused {burst_reused}",
                        last - first,
                        fps10 / 10,
                        fps10 % 10
                    );
                }
            }

            // Sleep until the next reason to wake.
            // Never a loop without a pause (2026-10-13: sampling back to
            // back while the pen-detect line read "down" hung touch on the
            // device; sampling holds that line low, so it never recovered).
            // While a touch is in progress, or the line says so: every
            // PEN_DOWN_SLEEP_MS (5 ms was the build the owner called "much
            // better" on 2026-10-13, 1 ms "much worse" with the filter of
            // then). Pressure decides whether a touch is in progress.
            if !filter.down() && raw.is_none() && touch.pen_down() {
                phantom = phantom.saturating_add(1);
            }
            let wait_ms: u32 = if showing.is_some() || gfx.busy() || gfx.commit_pending() {
                // A frame is with the engine or waiting for the blanking:
                // flip it as soon as it's due.
                1
            } else if shell.animating() {
                // A fling, a rebound or an entrance: the next frame as soon
                // as the last is on the screen.
                1
            } else if REPLAY && !replay_touching {
                // The pause before the next touch.
                replay_until.map_or(1, |u| u.saturating_sub(now_ms).clamp(1, 50) as u32)
            } else if filter.down() || touch.pen_down() || replay_touching {
                PEN_DOWN_SLEEP_MS
            } else if pressed != rp_drv_pxa_keypad::Pressed::NONE {
                20
            } else if shell.notice().is_some() {
                100
            } else {
                // The next minute, but look at the pen line every 50 ms
                // too: a touch must never depend on one interrupt alone
                // (on the device touch went dead after a hold, as if one
                // were missed; the counts in the run log will say).
                (60_000 - (now_ms % 60_000) as u32).clamp(1, 50)
            };
            if !filter.down() {
                // Re-arm pen-down, then re-check the level: an edge while it
                // was disarmed isn't remembered.
                pen_irq.done();
            }
            // Never past the next check-in: sleeping through the loader's
            // run limit returns us to it (or worse, the watchdog resets).
            let wait_ms = wait_ms.min(rt::check_in_every_ms()).max(1);
            let mut wait_ticks = wait_ms * TICKS_MS;
            // The frame in flight's own time, if sooner (R115: woken at its
            // flip's deadline, not up to a millisecond after).
            if let Some(due) = flip_due.filter(|_| gfx.busy() || gfx.commit_pending()) {
                let until = due.wrapping_sub(gclock.now_us()) as i32;
                wait_ticks = wait_ticks
                    .min((until.max(1) as u32).saturating_mul(13) / 4)
                    .max(1);
            }
            // While a touch is in progress the pen is sampled at its proven
            // rate (PEN_DOWN_SLEEP_MS after the last sample: the filter's
            // thresholds were proven at it, docs/hw/touch.md), however often
            // the display wakes the loop. Commits due sooner are written
            // here without a sample (the first device run of the plane
            // path: samples 2-4 ms apart, strokes broken into short
            // touches, drags up and swipes lost).
            if !REPLAY && (filter.down() || touch.pen_down()) {
                let pen_at = sampled_at.wrapping_add(PEN_DOWN_SLEEP_MS * TICKS_MS);
                loop {
                    let left = pen_at.wrapping_sub(rt::ticks()) as i32;
                    if left <= 0 {
                        break;
                    }
                    let mut ticks = left as u32;
                    if gfx.busy() || gfx.commit_pending() {
                        if let Step::Wait(t) = gfx.poll(&mut gclock) {
                            let until = t.wrapping_sub(gclock.now_us()) as i32;
                            ticks = ticks.min((until.max(1) as u32).saturating_mul(13) / 4);
                        }
                    }
                    if rt::set_alarm(rt::ticks().wrapping_add(ticks.max(1))) {
                        rt::wait_any_signal();
                    }
                    rt::check_in();
                }
                continue;
            }
            if rt::set_alarm(rt::ticks().wrapping_add(wait_ticks)) {
                rt::wait_any_signal();
            }
        }
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}
