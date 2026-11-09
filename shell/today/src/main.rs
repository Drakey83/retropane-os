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
    use rp_drv_pxa_mmc::PxaMmc;
    use rp_drv_pxa_ssp::Ssp;
    use rp_drv_pxa_uart::PxaUart;
    use rp_drv_pxa_udc::{Layout as UsbLayout, Udc};
    use rp_drv_sdcard::{BLOCK, Identity, SdCard, mbr_partitions};
    use rp_drv_tsc2046::Tsc2046;
    use rp_fat::AnyVolume;
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
    static DIAL_IMGS: rt::Slot<[rp_draw::Image; rp_shell::station::DIALS.len()]> = rt::Slot::new();
    /// The SD card and the CF card: in `.bss`, not `main`'s frame.
    static CF: rt::Slot<Option<(CfDisk, u32)>> = rt::Slot::new();
    /// The picture store (option C, docs/design/weather-rule0.md): the
    /// pictures the screens show, at their sizes; the shell holds handles
    /// to them (ADR-0021 rule 1), both renderers read them by handle
    /// (ADR-0030 G3), the pixels in SDRAM where the CPU composes (ADR-0022
    /// rule 2).
    static PICS: rt::Slot<rp_draw::pics::Pics<'static>> = rt::Slot::new();

    static CARD: rt::Slot<Option<(Card, u32)>> = rt::Slot::new();
    // The radar's rain cells' descriptions (their pixels are in the RAM pool).
    static RADAR_IMGS: rt::Slot<[rp_draw::Image; 3]> = rt::Slot::new();
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
    /// values: ~2 s of pen-down at ~7 ms a sample, ~14 KB). It was 1150
    /// (~54 KB): with a watched run's screen lines the 64 KiB run log
    /// filled at 130 s and lost the run's end twice (2026-10-31).
    const RECORD: u32 = 300;
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
    /// A build switch's decimal digits (anything else ends the number).
    const fn parse_u64(v: &str) -> u64 {
        let b = v.as_bytes();
        let mut n = 0u64;
        let mut i = 0;
        while i < b.len() && b[i].is_ascii_digit() {
            n = n * 10 + (b[i] - b'0') as u64;
            i += 1;
        }
        n
    }
    /// R188's measurement: the Weather app not composed ahead on Today
    /// (`RP_TODAY_NO_APP_AHEAD`; the bisect named 4fa4dbad, the app
    /// composed ahead for Reveal, the first slow commit). Reveal then opens
    /// without its picture: a measurement only.
    const NO_APP_AHEAD: bool = option_env!("RP_TODAY_NO_APP_AHEAD").is_some();
    /// R188's measurement (the owner's list of 2026-11-09 item 4): the
    /// touch replay held until this many ms after the Today screen first
    /// shows (`RP_TODAY_REPLAY_HOLD_MS`; 0, the ratchet's, starts at once),
    /// so the icon pack's reading and the pictures' making (2.8 s and 1.8 s
    /// on the X51v, run today-0081) are done before the first touch.
    const REPLAY_HOLD_MS: u64 = match option_env!("RP_TODAY_REPLAY_HOLD_MS") {
        Some(v) => parse_u64(v),
        None => 0,
    };
    /// R185's look check (`RP_TODAY_LOOK_CHECK`, sheet
    /// `docs/research/sheets/r185-look-check-unattended.md`): while the
    /// Weather app is up and nothing is in flight, the look the panel
    /// shows (`rp_today::look`, from pixels read back off the display) is
    /// checked against the look chosen, every pass; a mismatch is logged
    /// `LOOK WRONG`.
    const LOOK_CHECK: bool = option_env!("RP_TODAY_LOOK_CHECK").is_some();

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
    // Hourly at 32 bits (4.6 MB), Now and 10 Day at 16, the app and its
    // chrome composed ahead (0.6 MB each): within the 8.4 MB pool.
    const PAGES_QUOTA: u32 = 15 << 19;

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
        // Hourly at 32 bits (ARGB8888): its anti-aliased line stays smooth
        // (at ARGB4444 it banded and dithered: the owner, 2026-10-31). The
        // others at 16: nothing on them needs it, and Hourly's tall picture
        // (4.6 MB) leaves the pool no room for three at 32.
        let wide = tab == 1;
        let h = match if wide {
            gfx.picture_argb8888(pc, W as u16, rows)
        } else {
            gfx.picture(pc, W as u16, rows)
        } {
            Ok(h) => h,
            Err(e) => {
                let _ = writeln!(RunLog, "page {tab}: no picture ({e:?})");
                return None;
            }
        };
        let bytes = cpu.plane();
        // The CPU's plane is the page at 32 bits; a 16-bit page's rows go
        // through the dithered conversion on the way out.
        let bpp = if wide { 4 } else { 2 };
        let mut row = [0u32; W];
        let mut half = [0u8; W * 2];
        let words = W * bpp / 4;
        for y in 0..usize::from(rows) {
            if wide {
                for (k, w) in row.iter_mut().enumerate() {
                    let a = y * W * 4 + 4 * k;
                    *w = u32::from_le_bytes([bytes[a], bytes[a + 1], bytes[a + 2], bytes[a + 3]]);
                }
            } else {
                let r = rp_gfx::Rect::new(0, y as u32, W as u32, 1);
                let _ = rp_gfx::layer::flush(
                    &cpu.layer().view(),
                    PlaneFormat::Argb4444Dithered,
                    &mut half,
                    0,
                    &[r],
                );
                for (k, w) in row[..words].iter_mut().enumerate() {
                    *w = u32::from_le_bytes([
                        half[4 * k],
                        half[4 * k + 1],
                        half[4 * k + 2],
                        half[4 * k + 3],
                    ]);
                }
            }
            if gfx
                .write_picture(pc, h, (y * W * bpp) as u32, &row[..words])
                .is_err()
            {
                let _ = gfx.free_picture(pc, h);
                return None;
            }
            if y % 64 == 0 {
                rt::check_in();
            }
        }
        let _ = writeln!(
            RunLog,
            "page {tab}: its picture (480 x {rows}, {}) drawn by the CPU in {drawn_us} us, written in {} us",
            if wide { "ARGB8888" } else { "ARGB4444" },
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
        /// The scan line the last commit ended on (where its writes
        /// landed).
        line: u32,
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
                u32::from(m.4) | self.line << 16,
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
                line: w[9] >> 16,
            }
        }
    }

    /// Refreshes kept per opening (~1.6 s at 60 Hz).
    const TRACE: usize = 96;

    /// The layer's window over the whole of a screen-sized picture.
    const WHOLE: Window = Window {
        x: 0,
        y: 0,
        w: W as u16,
        h: H as u16,
        row: 0,
        col: 0,
    };

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

    /// The dock's dev relay (R132, quality audit C95: dev-only, TLS at the
    /// PC while developing): the image brings up its own USB serial port
    /// (the loader switched USB off before starting it) and asks the PC's
    /// `cargo xtask net` for the Weather app's questions (settings, place
    /// search, forecasts, the station). Off in the replay (on
    /// with `RP_TODAY_REPLAY_NET`: the link checked unattended), or with
    /// `RP_TODAY_NO_NET`.
    const NET: bool = option_env!("RP_TODAY_NO_NET").is_none()
        && (!REPLAY || option_env!("RP_TODAY_REPLAY_NET").is_some());
    /// The default pack's file (pack 0, VClouds: its 250 px pictures only,
    /// ~1.41 MB), kept packed in RAM even with a card in (the owner's
    /// choice, 2026-10-31): a card pulled mid-run never leaves a gap.
    const DEF_FILE: usize = 1536 * 1024;
    /// A big picture's stream read from a card pack (the largest ~170 KB),
    /// and a pack's table.
    const STREAM_BUF: usize = 256 * 1024;
    /// The picture store's slots (side, count): the shell's sizes
    /// (`rp_shell::Size`), as many of each as the screens show at once
    /// plus a spare, so a pick replaces a picture with no gap (the sheet's
    /// 250 x2, 66 x8, 40 x16; Hourly's 160 x6, not x2: every hour's card
    /// picture is kept, so a pick never waits on a decode, P1).
    /// The big pictures are fitted inside 250 x 176 (the hero's box), so
    /// their slots are that, not square (-148 KB). Hourly's graph is one
    /// picture of its own (its line and band, painted once a forecast).
    const PIC_CLASSES: [(u16, u16, u8); 6] = [
        (rp_shell::Size::Hero.side(), rp_shell::HERO_PIC_H, 2),
        (rp_shell::Size::Mid.side(), rp_shell::Size::Mid.side(), 6),
        (rp_shell::Size::Card.side(), rp_shell::Size::Card.side(), 8),
        (rp_shell::Size::Row.side(), rp_shell::Size::Row.side(), 16),
        (
            rp_shell::weather_app::GRAPH_PIC.0,
            rp_shell::weather_app::GRAPH_PIC.1,
            1,
        ),
        // The open chart's line, its own picture (Hourly's left be).
        (
            rp_shell::chart::CHART_PIC.0,
            rp_shell::chart::CHART_PIC.1,
            1,
        ),
    ];
    /// The keeper's tag while no pack is in yet (apart from every pack).
    const NOT_YET: u8 = 0xFE;

    /// The relay's rain frames kept (RPRN, R184 step 2): two live frames
    /// measured 10,800 and 10,908 bytes (2026-11-06), the relay refuses
    /// any over 240,000; 2 MB holds 13 typical frames many times over and
    /// eight of the largest; compacted when full.
    const RAIN_STORE: usize = 2 << 20;
    /// The radar's frames composed in 2700G memory: 13 views at 16 bits at
    /// most (rain in every band of every frame); usually far less, only
    /// the bands where it rains.
    const RADAR_QUOTA: u32 =
        (rp_weather::radar::PAST * rp_weather::tiles::VIEW_W * 2 * rp_weather::tiles::VIEW_H)
            as u32;

    use rp_today::rain::Composed;

    type Bands = rp_today::rain::BandPool<Handle>;

    /// Frame `c`'s band slots given back.
    fn give_back(pool: &mut Bands, c: &mut Option<Composed>) {
        if let Some(c) = c.take() {
            for s in c.slots.into_iter().flatten() {
                pool.give(s);
            }
        }
    }

    /// The radar's band pictures taken from the pool when Radar opens:
    /// three of 48 bands (every band of 13 frames), each smaller if the
    /// pool can't give it (halved down to 6 bands); logged.
    #[inline(never)]
    fn take_band_pics(gfx: &mut Graphics<'static, Mmio, Vram>, rc: ClientId, pool: &mut Bands) {
        use rp_today::rain::{BAND_H, PICS, SLOTS, SLOTS_PER_PIC};
        let mut want = SLOTS;
        for _ in 0..PICS {
            let mut n = want.min(SLOTS_PER_PIC);
            while n >= 6 {
                if let Ok(h) = gfx.picture_rgb565_rows(rc, n * BAND_H) {
                    pool.add(h, n);
                    break;
                }
                n /= 2;
            }
            if n < 6 {
                break;
            }
            want = want.saturating_sub(n);
            if want == 0 {
                break;
            }
        }
        let _ = writeln!(
            RunLog,
            "radar: {} band slots of {SLOTS} wanted in 2700G memory ({} bytes)",
            pool.total(),
            u32::from(pool.total()) * u32::from(BAND_H) * 960
        );
    }

    /// The scratch a frame is composed in: the view's rain.
    struct RainScratch<'a> {
        view: &'a mut [u32],
    }

    /// A frame being composed, a step a pass (run today-0054: a whole
    /// frame in one pass held the loop 0.56 s): step 0 expands its RPRN
    /// answer, 1 finds the rainy bands, 2 on write a band each.
    #[derive(Clone, Copy)]
    struct Job {
        f: usize,
        step: u8,
        comp: Composed,
        /// When it started, and the time its steps took (decoding, the
        /// rest), us.
        t0: u32,
        decode_us: u32,
        write_us: u32,
        /// Its answer didn't decode (audit C97): never composed as dry;
        /// the shell asks for it again once, then marks it lost.
        bad: bool,
    }

    impl Job {
        fn new(f: usize) -> Self {
            use rp_today::rain::BANDS;
            Self {
                f,
                step: 0,
                comp: Composed {
                    slots: [None; BANDS],
                    spans: [None; BANDS],
                },
                t0: rt::ticks(),
                decode_us: 0,
                write_us: 0,
                bad: false,
            }
        }

        /// Its band slots given back (abandoned).
        fn give_back(self, pool: &mut Bands) {
            give_back(pool, &mut Some(self.comp));
        }
    }

    /// One step of `job` (frame `job.f`'s rain over `map` in `look`):
    /// true once the frame is composed. `NoMemory`: no band slot free for
    /// the band due (the job stays at that step). The caller has waited for
    /// the engine (a slot given back may have been read by the last frame).
    #[allow(clippy::too_many_arguments)]
    #[inline(never)]
    fn job_step(
        gfx: &mut Graphics<'static, Mmio, Vram>,
        rc: ClientId,
        pool: &mut Bands,
        store: &rp_today::rain::Store<'_>,
        job: &mut Job,
        map: &[u32],
        look: rp_weather::maplayers::Look,
        s: &mut RainScratch<'_>,
    ) -> Result<bool, rp_graphics::Error> {
        use rp_today::rain::{BAND_H, BANDS};
        use rp_weather::tiles::VIEW_W;
        let t0 = rt::ticks();
        let f = job.f;
        match usize::from(job.step) {
            0 => {
                // The relay's frame (RPRN), checked as hostile input; a
                // damaged one shows no rain and is said so.
                let got = store
                    .tile(f, 0)
                    .map(|b| rp_weather::rprn::decode(b, s.view));
                if got != Some(Ok(())) {
                    // Never composed as a dry frame (audit C97): the caller
                    // gives it back to the shell, which asks again once.
                    job.bad = true;
                    let _ = writeln!(
                        RunLog,
                        "radar: frame {f} wasn't read ({got:?}): asked again once, never shown dry"
                    );
                    job.decode_us += rt::ticks().wrapping_sub(t0) / 13 * 4;
                    return Ok(true);
                }
                job.decode_us += rt::ticks().wrapping_sub(t0) / 13 * 4;
            }
            1 => job.comp.spans = rp_today::rain::bands_of(s.view),
            k => {
                let b = k - 2;
                if job.comp.spans[b].is_some() {
                    let Some((slot, (h, row))) = pool.take().and_then(|s| Some((s, pool.at(s)?)))
                    else {
                        return Err(rp_graphics::Error::NoMemory);
                    };
                    job.comp.slots[b] = Some(slot);
                    // A row a quarter at a time: a whole row's words (960
                    // bytes) sat on the image's stack (P6).
                    let mut words = [0u32; VIEW_W / 8];
                    for y in 0..BAND_H {
                        for q in 0..4 {
                            let at = usize::from(b as u16 * BAND_H + y) * VIEW_W + q * VIEW_W / 4;
                            rp_today::rain::row565(
                                &map[at..at + VIEW_W / 4],
                                &s.view[at..at + VIEW_W / 4],
                                look,
                                &mut words,
                            );
                            let off = u32::from(row + y) * (VIEW_W as u32 * 2)
                                + q as u32 * (VIEW_W as u32 / 2);
                            gfx.write_picture(rc, h, off, &words)?;
                        }
                    }
                }
                job.write_us += rt::ticks().wrapping_sub(t0) / 13 * 4;
                if b + 1 == BANDS {
                    return Ok(true);
                }
            }
        }
        job.step += 1;
        Ok(false)
    }

    /// The wallpaper's bands pointed at `frame`'s slots (`None`: the map
    /// alone); the areas where the rain differs from the frame shown
    /// before, redrawn there only, returned for the shell.
    #[inline(never)]
    fn show_bands(
        gfx: &mut Graphics<'static, Mmio, Vram>,
        rc: ClientId,
        pool: &Bands,
        frame: Option<&Composed>,
        spans_shown: &mut [Option<(u16, u16)>; rp_today::rain::BANDS],
    ) -> Result<([rp_draw::Rect; rp_today::rain::AREAS], usize), rp_graphics::Error> {
        use rp_today::rain::{BAND_H, BANDS};
        let none = [None; BANDS];
        let spans = frame.map_or(none, |c| c.spans);
        let (areas, n) = rp_today::rain::changed(spans_shown, &spans, rp_today::MAP_TOP);
        let mut bands = [None; BANDS];
        if let Some(c) = frame {
            for (b, s) in bands.iter_mut().zip(c.slots) {
                *b = s.and_then(|s| pool.at(s));
            }
        }
        let nb = if bands.iter().any(Option::is_some) {
            BANDS
        } else {
            0
        };
        gfx.wallpaper_bands(rc, rp_today::MAP_TOP, BAND_H, &bands[..nb], &areas[..n])?;
        *spans_shown = spans;
        Ok((areas, n))
    }

    /// Whose map the radar's layers are (R184).
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    enum MapOf {
        /// The place shown's, drawn by the dock's relay.
        Place,
    }
    /// The layers coloured into `view` in `look` when they are `of`'s and
    /// what is there is another look or another map's (R184, way C); true
    /// if it coloured.
    fn recolour(
        of: MapOf,
        look: Option<rp_weather::maplayers::Look>,
        layers_of: Option<MapOf>,
        coloured: &mut Option<(MapOf, rp_weather::maplayers::Look)>,
        layers: &[u8],
        view: &mut [u32],
    ) -> bool {
        let Some(l) = look.filter(|_| layers_of == Some(of)) else {
            return false;
        };
        if *coloured == Some((of, l)) {
            return false;
        }
        let t0 = rt::ticks();
        rp_weather::maplayers::colour(layers, l, view);
        *coloured = Some((of, l));
        let _ = writeln!(
            RunLog,
            "radar map: {of:?}'s layers coloured {l:?} in {} us",
            rt::ticks().wrapping_sub(t0) / 13 * 4
        );
        true
    }

    /// One pass of the place's kept radar map (R184 step 3): looked for on
    /// the card when the shell asks, a piece read, a layer unpacked, or the
    /// dock's map written (pen up). Each step logged after the card is let
    /// go (a log line mid-read would send the card log's sector).
    #[allow(clippy::too_many_arguments)]
    #[inline(never)]
    fn kept_map_pass(
        shell: &mut rp_shell::Shell,
        kept: &mut KeptMap,
        card: &mut Option<(Card, u32)>,
        card_tried: &mut bool,
        map_file: &mut [u8],
        map_layers: &mut [u8],
        scratch: &mut [u8],
        layers_of: &mut Option<MapOf>,
        coloured: &mut Option<(MapOf, rp_weather::maplayers::Look)>,
        pen_down: bool,
        now_ms: u64,
    ) {
        use rp_weather::mapcard::HEAD;
        use rp_weather::maplayers::{LAYERS, PLANE};
        let us = |t0: u32| rt::ticks().wrapping_sub(t0) / 13 * 4;
        if let Some(key) = shell.radar_map_wanted() {
            if kept.ram == Some(key) {
                // Still in RAM (the same place after a refused frame list).
                shell.radar_map_card(Some(kept.crc));
                shell.radar_map_in();
                let _ = writeln!(RunLog, "radar map: the place's layers still in RAM");
            } else {
                if !*card_tried {
                    *card_tried = true;
                    *card = take_card();
                }
                let t0 = rt::ticks();
                let got = match card.as_mut() {
                    Some((c, start)) => map_head(c, *start, key, map_file),
                    None => Err("no SD card"),
                };
                kept.asked_ms = now_ms;
                kept.read_us = us(t0);
                kept.unpack_us = 0;
                match got {
                    Ok((h, run)) => {
                        shell.radar_map_card(Some(h.crc));
                        // The layers in RAM become this place's.
                        kept.ram = None;
                        kept.key = None;
                        *layers_of = None;
                        *coloured = None;
                        // Straight off the card from the file's start (its
                        // header again, then the map); through the folder
                        // from the map's start if it isn't one run.
                        kept.step = MapStep::Read {
                            key,
                            head: h,
                            run,
                            got: if run.is_some() { 0 } else { HEAD },
                        };
                        let _ = writeln!(
                            RunLog,
                            "radar map: the place's on the card ({} bytes, written at {}, {}): reading it, its header in {} us",
                            h.len,
                            h.at,
                            if run.is_some() {
                                "one run"
                            } else {
                                "in pieces"
                            },
                            kept.read_us
                        );
                    }
                    Err(why) => {
                        shell.radar_map_card(None);
                        let _ = writeln!(
                            RunLog,
                            "radar map: not from the card ({why}, {} us): the dock's",
                            kept.read_us
                        );
                    }
                }
            }
        }
        // A step under way for a place no longer shown stops.
        let current = shell.radar_key();
        let fail = |shell: &mut rp_shell::Shell, kept: &mut KeptMap, why: &str| {
            shell.radar_map_card(None);
            kept.key = None;
            kept.ram = None;
            kept.step = MapStep::Idle;
            let _ = writeln!(RunLog, "radar map: the card's not used ({why}): the dock's");
        };
        match kept.step {
            MapStep::Idle => {}
            MapStep::Read { key, .. } | MapStep::Unpack { key, .. } if key != current => {
                kept.step = MapStep::Idle;
                let _ = writeln!(RunLog, "radar map: the card's read stopped (another place)");
            }
            MapStep::Read {
                key,
                head,
                run,
                got,
            } => {
                let t0 = rt::ticks();
                let n = match (run, card.as_mut()) {
                    (Some(run), Some((c, _))) => map_file
                        .get_mut(got..)
                        .and_then(|out| map_sectors(c, run, (got / 512) as u32, out)),
                    (None, Some((c, start))) => map_file
                        .get_mut(got..HEAD + head.len)
                        .and_then(|out| map_chunk(c, *start, key, got as u64, out)),
                    _ => None,
                };
                kept.read_us += us(t0);
                match n {
                    Some(n) if n > 0 && got + n < HEAD + head.len => {
                        kept.step = MapStep::Read {
                            key,
                            head,
                            run,
                            got: got + n,
                        };
                    }
                    Some(n) if n > 0 => {
                        let t0 = rt::ticks();
                        let ok =
                            rp_weather::mapcard::check_map(&head, &map_file[HEAD..HEAD + head.len]);
                        kept.read_us += us(t0);
                        if ok.is_ok() {
                            kept.key = Some(key);
                            kept.len = head.len;
                            kept.crc = head.crc;
                            kept.step = MapStep::Unpack { key, layer: 0 };
                        } else {
                            fail(shell, kept, "its bytes not its checksum");
                        }
                    }
                    _ => fail(shell, kept, "a piece not readable"),
                }
            }
            MapStep::Unpack { key, layer } => {
                let t0 = rt::ticks();
                let got = rp_weather::maplayers::unpack_layer(
                    &map_file[HEAD..HEAD + kept.len],
                    layer,
                    &mut map_layers[layer * PLANE..(layer + 1) * PLANE],
                );
                kept.unpack_us += us(t0);
                match got {
                    Ok(()) if layer + 1 < LAYERS => {
                        kept.step = MapStep::Unpack {
                            key,
                            layer: layer + 1,
                        };
                    }
                    Ok(()) => {
                        kept.ram = Some(key);
                        kept.step = MapStep::Idle;
                        *layers_of = Some(MapOf::Place);
                        *coloured = None;
                        shell.radar_map_in();
                        let _ = writeln!(
                            RunLog,
                            "radar map: the place's from the card, {} bytes: read in {} us, unpacked in {} us ({LAYERS} passes), in {} ms after Radar asked",
                            kept.len,
                            kept.read_us,
                            kept.unpack_us,
                            now_ms.wrapping_sub(kept.asked_ms)
                        );
                    }
                    Err(e) => {
                        let _ = writeln!(RunLog, "radar map: the card's layer {layer}: {e:?}");
                        fail(shell, kept, "a layer not unpacked");
                    }
                }
            }
            MapStep::Write { key, wait: true } => {
                kept.step = MapStep::Write { key, wait: false };
            }
            MapStep::Write { key, .. } if !pen_down => {
                // The first pass of the write a piece a pass (run r184s3a:
                // one put held the loop 532 ms): the header, the folder if
                // missing and the free run found; nothing linked yet.
                kept.step = MapStep::Idle;
                if kept.key != Some(key) {
                    return;
                }
                let len = kept.len;
                let head = rp_weather::mapcard::head(
                    key,
                    &map_file[HEAD..HEAD + len],
                    shell.radar_newest(),
                );
                map_file[..HEAD].copy_from_slice(&head);
                // The last sector's tail zeroed (the file's length bounds it).
                let end = (HEAD + len).div_ceil(512) * 512;
                map_file[HEAD + len..end].fill(0);
                if !*card_tried {
                    *card_tried = true;
                    *card = take_card();
                }
                let t0 = rt::ticks();
                let got = match card.as_mut() {
                    Some((c, start)) => {
                        map_stage(c, *start, key, HEAD + len, scratch).map_err(|e| e.name())
                    }
                    None => Err("no SD card"),
                };
                let took = us(t0);
                kept.write = (took, took, 1);
                match got {
                    Ok(st) => {
                        kept.step = MapStep::Pieces { key, st, at: 0 };
                        let _ = writeln!(
                            RunLog,
                            "radar map: writing it to the card ({} bytes, {} sectors): its run found in {took} us",
                            HEAD + len,
                            st.sectors
                        );
                    }
                    Err(why) => {
                        let _ =
                            writeln!(RunLog, "radar map: not kept on the card ({why}, {took} us)");
                    }
                }
            }
            MapStep::Pieces { key, st, at } if !pen_down => {
                if kept.key != Some(key) {
                    kept.step = MapStep::Idle;
                    let _ = writeln!(RunLog, "radar map: its write stopped (a new map)");
                    return;
                }
                let n = (st.sectors - at).min(rp_today::kept::PIECE as u32 / 512);
                let from = at as usize * 512;
                let t0 = rt::ticks();
                let got = match card.as_mut() {
                    Some((c, start)) => {
                        map_piece(c, *start, &st, at, &map_file[from..from + n as usize * 512])
                            .map_err(|e| e.name())
                    }
                    None => Err("no SD card"),
                };
                let took = us(t0);
                kept.write = (
                    kept.write.0 + took,
                    kept.write.1.max(took),
                    kept.write.2 + 1,
                );
                match got {
                    Ok(()) if at + n < st.sectors => {
                        kept.step = MapStep::Pieces {
                            key,
                            st,
                            at: at + n,
                        };
                    }
                    Ok(()) => kept.step = MapStep::Commit { key, st },
                    Err(why) => {
                        kept.step = MapStep::Idle;
                        let _ =
                            writeln!(RunLog, "radar map: not kept on the card (a piece: {why})");
                    }
                }
            }
            MapStep::Commit { key, st } if !pen_down => {
                kept.step = MapStep::Idle;
                if kept.key != Some(key) {
                    return;
                }
                let mut saved = [(i32::MIN, i32::MIN); rp_weather::settings::PLACES_MAX];
                for (s, p) in saved.iter_mut().zip(shell.saved_places()) {
                    *s = p;
                }
                let t0 = rt::ticks();
                let got = match card.as_mut() {
                    Some((c, start)) => map_commit(
                        c,
                        *start,
                        key,
                        st,
                        &|k| saved.contains(&(k.lat, k.lon)),
                        scratch,
                    )
                    .map_err(|e| e.name()),
                    None => Err("no SD card"),
                };
                let took = us(t0);
                let (total, worst, passes) = (
                    kept.write.0 + took,
                    kept.write.1.max(took),
                    kept.write.2 + 1,
                );
                match got {
                    Ok(dropped) => {
                        let _ = writeln!(
                            RunLog,
                            "radar map: kept on the card ({} bytes) in {passes} passes, {total} us, the longest pass {worst} us (linked in {took} us){}",
                            kept.len + HEAD,
                            if dropped.is_some() {
                                ", the oldest other place's dropped (the cap)"
                            } else {
                                ""
                            }
                        );
                    }
                    Err(why) => {
                        let _ =
                            writeln!(RunLog, "radar map: not kept on the card ({why}, {took} us)");
                    }
                }
            }
            MapStep::Write { .. } | MapStep::Pieces { .. } | MapStep::Commit { .. } => {}
        }
    }

    /// The Today screen's forecast cards in the run log: each day's
    /// condition and its picture (its key, and whether the store still
    /// holds it), or "none" (R181: a card drawn without its icon said
    /// nothing).
    fn cards_line(shell: &rp_shell::Shell, pics: &rp_draw::pics::Pics<'_>) {
        let _ = write!(RunLog, "today cards:");
        for (id, pic) in shell.today_cards() {
            let _ = match pic {
                Some(p) => write!(
                    RunLog,
                    " {id}:{:#x}{}",
                    p.key(),
                    if pics.is_live(p) { "" } else { " stale" }
                ),
                None => write!(RunLog, " {id}:none"),
            };
        }
        let _ = writeln!(RunLog);
    }

    /// Card file `stem`'s bytes from `off` into `out` (all of it), across
    /// its parts (`<stem>A.PAK`, `B`, ..., each [`rp_today::PACK_PART`]
    /// bytes but the last): one ranged read a part, the volume mounted
    /// once. False if the card, a part or a byte is missing. Nothing is
    /// logged while it reads.
    fn read_span(card: &mut Card, start: u32, stem: [u8; 5], off: u64, out: &mut [u8]) -> bool {
        rp_hal_x51v::cardsink::settle();
        // FAT or exFAT alike (audit C98).
        let Ok(mut vol) = AnyVolume::mount(&mut *card, start) else {
            return false;
        };
        let mut scratch = [0u8; 2 * BLOCK];
        let part_len = rp_today::PACK_PART as u64;
        let mut done = 0usize;
        while done < out.len() {
            let at = off + done as u64;
            let part = at / part_len;
            if part >= 26 {
                return false;
            }
            let name = [
                stem[0],
                stem[1],
                stem[2],
                stem[3],
                stem[4],
                b'A' + part as u8,
                b'.',
                b'P',
                b'A',
                b'K',
            ];
            let Ok(file) = core::str::from_utf8(&name) else {
                return false;
            };
            let in_part = at % part_len;
            let n = (out.len() - done).min((part_len - in_part) as usize);
            let mut got = 0usize;
            let dst = &mut out[done..done + n];
            let ok = vol.read_range(file, in_part, n as u64, &mut scratch, &mut |c| {
                if let Some(o) = dst.get_mut(got..got + c.len()) {
                    o.copy_from_slice(c);
                }
                got += c.len();
            });
            if ok.is_err() || got != n {
                return false;
            }
            done += n;
        }
        true
    }

    /// Pack `n`'s table from the card (two reads: the header, then the
    /// table its count names), into `buf`.
    fn read_index(
        card: &mut Card,
        start: u32,
        n: u8,
        buf: &mut [u8],
    ) -> Option<rp_today::PackIndex> {
        let stem = [b'R', b'P', b'I', b'C', b'0' + n];
        let head = buf.get_mut(..12)?;
        if !read_span(card, start, stem, 0, head) {
            return None;
        }
        let len = rp_today::PackIndex::head_len(head)?;
        let all = buf.get_mut(..len)?;
        if !read_span(card, start, stem, 0, all) {
            return None;
        }
        rp_today::PackIndex::read(all)
    }

    /// The store's pictures made since it last held what the screens show:
    /// logged as one line when it does again.
    #[derive(Default)]
    struct Batch {
        made: u32,
        passes: u32,
        us: u32,
        worst: (u32, u32),
        card_failed: bool,
    }

    /// The default pack's file being read from the card a piece a pass.
    struct FileLoad {
        /// Bytes in so far, the part being read and where in it.
        got: usize,
        part: u8,
        off: u64,
        /// The time its reads took (us).
        read_us: u32,
        /// From the CF card (else the SD card).
        from_cf: bool,
    }
    type Card = SdCard<PxaMmc<Mmio>>;
    type CfDisk = rp_drv_cf::CfCard<Mmio, Mmio>;

    /// The CF card, adopted as the loader left it when its card log is on
    /// (the loader attached it; no power or reset line is touched here:
    /// a card the loader didn't attach is left alone), and its first
    /// partition's start.
    fn take_cf_card() -> Option<(CfDisk, u32)> {
        let sectors = rp_hal_x51v::cardsink::current()
            .map(|s| s.cf_sectors)
            .filter(|&n| n > 0)?;
        let (attr, common) = rp_hal_x51v::devices::take_cf()?;
        let mut cf = rp_drv_cf::CfCard::adopt(attr, common, sectors);
        let mut block = [0u8; BLOCK];
        rp_hal_api::BlockDevice::read_blocks(&mut cf, 0, &mut block).ok()?;
        let start = mbr_partitions(&block)
            .and_then(|p| p[0])
            .map_or(0, |p| p.start);
        Some((cf, start))
    }

    /// The SD card, taken over as the loader left it when its card log is
    /// on (no command sent: the log keeps writing through the same
    /// controller between the image's reads, `cardsink::settle` first),
    /// else started fresh; and its first partition's start.
    fn take_card() -> Option<(Card, u32)> {
        let id = rp_hal_x51v::cardsink::current().map(|s| Identity {
            rca: s.sd.rca as u16,
            high_capacity: s.sd.flags & 1 != 0,
            wide: s.sd.flags & 2 != 0,
            blocks: s.sd.blocks,
        });
        // Taking the controller settles the card log and marks the card
        // taken (the PC re-reads it after the run).
        let mmc = rt::take_mmc()?;
        let mut card = match id {
            Some(id) if id.rca != 0 => SdCard::adopt(PxaMmc::new(mmc), id),
            _ => SdCard::try_init(PxaMmc::new(mmc)).ok()?,
        };
        let mut block = [0u8; BLOCK];
        card.read(0, &mut block).ok()?;
        let start = mbr_partitions(&block)
            .and_then(|p| p[0])
            .map_or(0, |p| p.start);
        Some((card, start))
    }

    /// The card file `stem` from the card's root into `out`: its parts
    /// (`<stem>A.PAK`, `B`, ...: the loader takes files up to its image
    /// area's size, a pack is 1.2-1.6 MB) joined, up to the first missing
    /// or empty one; the length, or `None` (no card, no part, too big).
    /// Icon pack `n` is `RPIC<n>`.
    /// Nothing is logged while it reads (a log line would send the card
    /// log's sector mid-read).
    fn read_pack(card: &mut Card, start: u32, stem: [u8; 5], out: &mut [u8]) -> Option<usize> {
        rp_hal_x51v::cardsink::settle();
        // FAT or exFAT alike (audit C98).
        let mut vol = AnyVolume::mount(&mut *card, start).ok()?;
        let mut scratch = [0u8; 2 * BLOCK];
        let (mut at, mut over) = (0usize, false);
        for part in b'A'..=b'Z' {
            let name = [
                stem[0], stem[1], stem[2], stem[3], stem[4], part, b'.', b'P', b'A', b'K',
            ];
            let file = core::str::from_utf8(&name).ok()?;
            let before = at;
            match vol.read_range(file, 0, u64::MAX, &mut scratch, &mut |chunk| {
                match out.get_mut(at..at + chunk.len()) {
                    Some(o) => o.copy_from_slice(chunk),
                    None => over = true,
                }
                at += chunk.len();
            }) {
                Ok(_) => {}
                Err(rp_fat::Error::NotFound) => break,
                Err(_) => return None,
            }
            if at == before {
                break;
            }
        }
        (!over && at > 0).then_some(at)
    }

    /// The bytes one background read takes from the card (~30 ms on the
    /// X51v: a pack's 1.9 MB read in 1.4-2.2 s, today-0035/36).
    const PACK_CHUNK: u64 = 32 * 1024;

    /// The next piece of card file `stem` (parts `<stem>A.PAK`, `B`, ...)
    /// at `part`/`off` into `out`: the bytes read (0 when that part is
    /// done: the next part from 0), or `None` once there is no next part
    /// (or no card). Nothing is logged while it reads.
    fn read_chunk<D: rp_hal_api::BlockDevice>(
        card: &mut D,
        start: u32,
        stem: [u8; 5],
        part: u8,
        off: u64,
        out: &mut [u8],
    ) -> Option<usize> {
        rp_hal_x51v::cardsink::settle();
        // FAT or exFAT alike (audit C98).
        let mut vol = AnyVolume::mount(&mut *card, start).ok()?;
        let name = [
            stem[0],
            stem[1],
            stem[2],
            stem[3],
            stem[4],
            b'A' + part,
            b'.',
            b'P',
            b'A',
            b'K',
        ];
        let file = core::str::from_utf8(&name).ok()?;
        let mut scratch = [0u8; 2 * BLOCK];
        let mut at = 0usize;
        let mut over = false;
        let len = PACK_CHUNK.min(out.len() as u64);
        let (size, _) = vol
            .read_range(file, off, len, &mut scratch, &mut |c| {
                match out.get_mut(at..at + c.len()) {
                    Some(o) => o.copy_from_slice(c),
                    None => over = true,
                }
                at += c.len();
            })
            .ok()?;
        // An empty part past the first ends the file.
        if part > 0 && off == 0 && size == 0 {
            return None;
        }
        (!over).then_some(at)
    }

    /// The kept radar map's header for `key` (R184 step 3,
    /// [`rp_today::kept::head`]), the card log's controller settled first.
    /// Nothing is logged while it reads.
    fn map_head(
        card: &mut Card,
        start: u32,
        key: rp_weather::mapcard::Key,
        out: &mut [u8],
    ) -> Result<(rp_weather::mapcard::Head, Option<rp_fat::Reserved>), &'static str> {
        rp_hal_x51v::cardsink::settle();
        rp_today::kept::head(&mut *card, start, key, out)
    }

    /// The next piece of `key`'s kept map ([`rp_today::kept::piece`]).
    fn map_chunk(
        card: &mut Card,
        start: u32,
        key: rp_weather::mapcard::Key,
        off: u64,
        out: &mut [u8],
    ) -> Option<usize> {
        rp_hal_x51v::cardsink::settle();
        rp_today::kept::piece(&mut *card, start, key, off, out)
    }

    /// The free run for `key`'s kept map ([`rp_today::kept::stage`]).
    fn map_stage(
        card: &mut Card,
        start: u32,
        key: rp_weather::mapcard::Key,
        bytes: usize,
        scratch: &mut [u8],
    ) -> Result<rp_fat::Staged, rp_fat::Error> {
        rp_hal_x51v::cardsink::settle();
        rp_today::kept::stage(&mut *card, start, key, bytes, scratch)
    }

    /// A piece of a staged kept map ([`rp_today::kept::stage_piece`]).
    fn map_piece(
        card: &mut Card,
        start: u32,
        st: &rp_fat::Staged,
        at: u32,
        data: &[u8],
    ) -> Result<(), rp_fat::Error> {
        rp_hal_x51v::cardsink::settle();
        rp_today::kept::stage_piece(&mut *card, start, st, at, data)
    }

    /// Links a staged kept map and keeps the folder to its cap
    /// ([`rp_today::kept::commit`]). Nothing is logged while it writes.
    fn map_commit(
        card: &mut Card,
        start: u32,
        key: rp_weather::mapcard::Key,
        st: rp_fat::Staged,
        keep: &dyn Fn(rp_weather::mapcard::Key) -> bool,
        scratch: &mut [u8],
    ) -> Result<Option<rp_weather::mapcard::Key>, rp_fat::Error> {
        rp_hal_x51v::cardsink::settle();
        rp_today::kept::commit(&mut *card, start, key, st, keep, scratch)
    }

    /// Sectors of a kept map lying in one run, straight off the card
    /// ([`rp_today::kept::sectors`]).
    fn map_sectors(
        card: &mut Card,
        run: rp_fat::Reserved,
        at: u32,
        out: &mut [u8],
    ) -> Option<usize> {
        rp_hal_x51v::cardsink::settle();
        rp_today::kept::sectors(&mut *card, run, at, out)
    }

    /// Where the place's kept radar map is (R184 step 3).
    #[derive(Clone, Copy)]
    enum MapStep {
        /// Nothing under way.
        Idle,
        /// Read from the card a piece a pass: `got` bytes of the file so far
        /// (its header and map: whole sectors straight off the card when it
        /// lies in one run, `run`; else through the folder, from the map).
        Read {
            key: rp_weather::mapcard::Key,
            head: rp_weather::mapcard::Head,
            run: Option<rp_fat::Reserved>,
            got: usize,
        },
        /// Unpacked a layer a pass (~21 ms each; 167 ms in one pass before).
        Unpack {
            key: rp_weather::mapcard::Key,
            layer: usize,
        },
        /// The dock's map to write to the card: in the pass after it was
        /// shown, the pen up (`wait` the pass between): its run found.
        Write {
            key: rp_weather::mapcard::Key,
            wait: bool,
        },
        /// Written a piece a pass (32 KB) into its run, from sector `at`.
        Pieces {
            key: rp_weather::mapcard::Key,
            st: rp_fat::Staged,
            at: u32,
        },
        /// Linked as the place's kept map (the old one whole until then).
        Commit {
            key: rp_weather::mapcard::Key,
            st: rp_fat::Staged,
        },
    }

    /// The place's radar map: whose bytes the map file holds (after its
    /// header), their length and CRC-32, whose layers are unpacked in RAM,
    /// the step under way, and its times for the log.
    struct KeptMap {
        key: Option<rp_weather::mapcard::Key>,
        len: usize,
        crc: u32,
        ram: Option<rp_weather::mapcard::Key>,
        step: MapStep,
        asked_ms: u64,
        read_us: u32,
        unpack_us: u32,
        /// The write a piece a pass: its time, its longest pass, its passes.
        write: (u32, u32, u32),
    }

    /// The USB D+ pull-up's GPIO (docs/hw/udc.md).
    const USB_PULLUP: u8 = 2;
    /// The largest answer kept (a `better_forecast` is ~30-60 KB).
    const NET_BUF: usize = 256 * 1024;
    /// The scratch a kept radar map's write searches the FAT with (R184
    /// step 3: up to 64 sectors a command).
    const MAP_SCRATCH: usize = 32 * 1024;

    /// The longest a question waits for the relay's answer (a search or
    /// a forecast through the PC is well under a second; a slow network
    /// several).
    const NET_TIMEOUT_MS: u64 = 20_000;

    /// The relay link: the port, the question being sent (line and Save's
    /// record) and how much of it went, whether an answer is awaited and
    /// since when, the answer's header so far, its length and bytes in.
    struct Net {
        udc: Udc<Mmio>,
        out: [u8; 96 + rp_weather::store::LEN],
        out_n: usize,
        out_at: usize,
        waiting: bool,
        asked_ms: u64,
        header: [u8; 32],
        hn: usize,
        len: Option<usize>,
        got: usize,
        /// The tile being fetched: where it goes.
        tile: Option<(Option<u8>, u8, rp_weather::tiles::View)>,
        /// The default icon pack's piece being fetched (option C).
        pack: bool,
    }

    impl Net {
        /// Ready for the next question.
        fn reset(&mut self) {
            self.out_n = 0;
            self.out_at = 0;
            self.waiting = false;
            self.hn = 0;
            self.len = None;
            self.got = 0;
            self.tile = None;
            self.pack = false;
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
        // The image's start, for the start-to-screen time (the sheet's
        // target: under 0.5 s; P4's baseline).
        let t_image = rt::ticks();
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
        let Some(two_d) = TWO_D.put_in(|m| {
            Mbx2dRenderer::new_in(
                m,
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
        let pool_words = pool.len();
        let (pics_px, rest) = pool.split_at_mut(rp_draw::pics::Pics::words(&PIC_CLASSES));
        let (wall_words, rest) = rest.split_at_mut(rp_today::WALLPAPER_BYTES.div_ceil(4));
        let (radar_px, rest) = rest.split_at_mut(rp_today::RADAR_WORDS);
        let (dial_px, rest) = rest.split_at_mut(rp_shell::station::DIAL_WORDS);
        let (map_words, rest) = rest.split_at_mut(rp_weather::maplayers::BYTES.div_ceil(4));
        // The plane path's page drawing: the CPU's layer (ARGB8888) and the
        // picture it is flushed into (ARGB4444), the tallest page's size.
        let (layer_px, rest) = rest.split_at_mut(W * PIC_ROWS);
        let (plane_words, rest) = rest.split_at_mut(W * PIC_ROWS);
        // The openings' trace, in the RAM pool (not the image's RAM budget).
        let (trace_words, rest) = rest.split_at_mut(TRACE * 10);
        // The relay's answer (the dock's dev relay).
        let (net_words, rest) = rest.split_at_mut(NET_BUF / 4);
        // The pictures (option C): the default pack's file, a card pack's
        // picture stream read in, the radar map's file, the filter's rows.
        let (def_words, rest) = rest.split_at_mut(DEF_FILE / 4);
        let def_file: &'static mut [u8] = rp_hal_mem::words_as_bytes_mut(def_words);
        let (stream_words, rest) = rest.split_at_mut(STREAM_BUF / 4);
        let stream_buf: &'static mut [u8] = rp_hal_mem::words_as_bytes_mut(stream_words);
        let (big_px, rest) = rest.split_at_mut(rp_today::BIG_WORDS);
        let (shrink_words, rest) = rest.split_at_mut(rp_today::Maker::shrink_words());
        let shrink_px: &'static mut [i32] = rp_hal_mem::words_as_i32_mut(shrink_words);
        // The real radar (R184 step 2, way C): the map's view, the rain
        // frame being composed, and the relay's frames kept (RPRN, ~11 KB
        // each; eight 32-bit frames took 7.1 MB, and the PNG reader's
        // scratch 0.8 MB more, gone with the PNGs).
        let view_px = rp_weather::tiles::VIEW_W * rp_weather::tiles::VIEW_H;
        let (radar_view_map, rest) = rest.split_at_mut(view_px);
        let (rain_view, rest) = rest.split_at_mut(view_px);
        let (rain_words, rest) = rest.split_at_mut(RAIN_STORE / 4);
        let mut rain_store = rp_today::rain::Store::new(rp_hal_mem::words_as_bytes_mut(rain_words));
        // The place's radar map as a file (R184 step 3): its header and the
        // relay's bytes, read from the card or kept from the dock to write
        // there; and the write's scratch (rp-fat searches the FAT for free
        // clusters in runs: a sector at a time took 3.7 s once, docs/hw/sd.md).
        let (map_file_words, rest) =
            rest.split_at_mut((rp_weather::mapcard::HEAD + rp_weather::mapcard::MAX + 512) / 4);
        let map_file: &'static mut [u8] = rp_hal_mem::words_as_bytes_mut(map_file_words);
        let (map_scratch_words, rest) = rest.split_at_mut(MAP_SCRATCH / 4);
        let map_scratch: &'static mut [u8] = rp_hal_mem::words_as_bytes_mut(map_scratch_words);
        let mut kept = KeptMap {
            key: None,
            len: 0,
            crc: 0,
            ram: None,
            step: MapStep::Idle,
            asked_ms: 0,
            read_us: 0,
            unpack_us: 0,
            write: (0, 0, 0),
        };
        // Weather's RAM (the sheet's target: under 8 MB): every part of the
        // pool this image takes, and the pictures' share (option C).
        let pictures = rp_draw::pics::Pics::words(&PIC_CLASSES) * 4
            + DEF_FILE
            + STREAM_BUF
            + rp_today::BIG_WORDS * 4
            + rp_today::Maker::shrink_words() * 4;
        let _ = writeln!(
            RunLog,
            "ram: the image's parts of the RAM pool {} bytes of {} (the pictures {pictures}: store {}, default pack {DEF_FILE}, card stream {STREAM_BUF}, decode scratch {})",
            (pool_words - rest.len()) * 4,
            pool_words * 4,
            rp_draw::pics::Pics::words(&PIC_CLASSES) * 4,
            (rp_today::BIG_WORDS + rp_today::Maker::shrink_words()) * 4,
        );
        let net_buf = rp_hal_mem::words_as_bytes_mut(net_words);
        let plane_bytes = rp_hal_mem::words_as_bytes_mut(plane_words);
        let wall_bytes =
            &mut rp_hal_mem::words_as_bytes_mut(wall_words)[..rp_today::WALLPAPER_BYTES];
        let map_layers: &'static mut [u8] =
            &mut rp_hal_mem::words_as_bytes_mut(map_words)[..rp_weather::maplayers::BYTES];
        let t0 = rt::ticks();
        if let Err(e) = rp_today::unpack_wallpaper(WALLPAPER_PACKED, wall_bytes) {
            crash(format_args!("wallpaper unpacking: {e:?}"))
        }
        let unpacked_us = rt::ticks().wrapping_sub(t0) / 13 * 4;
        let wallpaper: &'static [u8] = wall_bytes;
        // The radar's base map as layers (R184, rp_weather::maplayers): the
        // place's, drawn by the dock's relay; coloured into the view in the
        // look the app's appearance names, again only when the look or the
        // layers change. Never another place's map (the owner, run
        // today-0055: "it starts out over seattle"): until the place's is
        // in, the radar shows the wallpaper and "Radar loading".
        let mut layers_of: Option<MapOf> = None;
        let mut coloured: Option<(MapOf, rp_weather::maplayers::Look)> = None;
        // The rain frames composed (R184 step 2) are kept with their PNGs in
        // `rain_store` (off the stack); the frame the wallpaper's bands
        // show and its rain's columns; the band slots; the frame whose
        // tiles are coming in.
        let mut banded: Option<usize> = None;
        let mut band_pool = Bands::new();
        let mut bands_tried = false;
        // The frame being composed, a step a pass; whether the background
        // composing waits for room; the pages whose pictures Radar holds.
        let mut radar_job: Option<Job> = None;
        // The living sky's last logged state, when, and its counts then.
        #[allow(clippy::type_complexity)]
        let mut sky_logged: Option<((bool, &'static str, u8, bool), u64, u32, u32)> = None;
        let mut no_band_room = false;
        let mut pages_lent = 0u8;
        let mut spans_shown = [None; rp_today::rain::BANDS];
        let mut filling: Option<usize> = None;
        let mut scratch = RainScratch { view: rain_view };
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
        let Some(pics) = PICS.put_in(|m| rp_draw::pics::Pics::new_in(m, pics_px, &PIC_CLASSES))
        else {
            crash(format_args!("started twice"))
        };
        let pics: &'static rp_draw::pics::Pics<'static> = pics;
        gfx.two_d().set_pics(pics);
        let mut keeper = rp_today::Keeper::new();
        let mut maker = rp_today::Maker::new(big_px, shrink_px).timed(|| rt::ticks() / 13 * 4);
        // The default pack's file: being read, read (its table), or none.
        let mut def_load: Option<FileLoad> = None;
        let mut def_index: Option<rp_today::PackIndex> = None;
        let mut def_failed = false;
        // The same file from the PC through the dock's relay when no card
        // has it: the bytes in so far, and whether the relay can't give it.
        let mut relay_got: Option<usize> = None;
        let mut relay_failed = false;
        // The pack chosen other than the default, and its table.
        let mut chosen: Option<(u8, rp_today::PackIndex)> = None;
        let mut icons = rp_shell::Icons::NONE;
        let t0 = rt::ticks();
        if let Some(imgs) = RADAR_IMGS.put_with(|| [rp_today::NO_IMAGE; 3]) {
            icons.radar = rp_today::radar_into(radar_px, imgs);
        }
        let _ = writeln!(
            RunLog,
            "radar cells drawn in {} us",
            rt::ticks().wrapping_sub(t0) / 13 * 4
        );
        let t0 = rt::ticks();
        let dials =
            match DIAL_IMGS.put_with(|| [rp_today::NO_IMAGE; rp_shell::station::DIALS.len()]) {
                Some(imgs) => rp_today::dials_into(dial_px, imgs),
                None => rp_shell::station::Dials::NONE,
            };
        let _ = writeln!(
            RunLog,
            "station dials painted in {} us",
            rt::ticks().wrapping_sub(t0) / 13 * 4
        );
        let mut pack_shown = 0u8;

        let Some(cf) = CF.put_with(|| None) else {
            crash(format_args!("started twice"))
        };
        let (Some(shell), Some(frame)) = (
            SHELL.put_in(|m| Shell::new_in(m, rp_today::shell_fonts(), icons, true)),
            FRAME.put_in(Frame::new_in),
        ) else {
            crash(format_args!("started twice"))
        };
        shell.set_dials(dials);
        // The Weather settings `cargo xtask load` put on the card (the
        // relay's record): the chosen icon pack and place known before the
        // first picture is made (the relay answers ~15 s in).
        // The PC's time at the load, from the card (R182).
        let card_min: Option<i64>;
        let Some(card) = CARD.put_with(|| None) else {
            crash(format_args!("started twice"))
        };
        let mut card_tried = true;
        *card = take_card();
        let t0 = rt::ticks();
        // The PC's time at the load (R182), else the build's.
        card_min = card
            .as_mut()
            .and_then(|(c, start)| read_pack(c, *start, *b"RPTIM", stream_buf))
            .and_then(|n| clock::from_card(&stream_buf[..n]));
        match card_min {
            Some(m) => {
                shell.set_time(clock::time_at(m));
                let _ = writeln!(RunLog, "clock: the PC's time at the load");
            }
            None => {
                let _ = writeln!(
                    RunLog,
                    "clock: the build's time (no time file on the card: R182)"
                );
            }
        }
        match card
            .as_mut()
            .and_then(|(c, start)| read_pack(c, *start, *b"RPSET", stream_buf))
        {
            Some(n) => {
                shell.settings_from_card(&stream_buf[..n]);
                let _ = writeln!(
                    RunLog,
                    "settings: read from the card ({n} bytes) in {} us: icon pack {}",
                    rt::ticks().wrapping_sub(t0) / 13 * 4,
                    shell.icon_pack()
                );
            }
            None => {
                let _ = writeln!(
                    RunLog,
                    "settings: none on the card (the relay's at its answer)"
                );
            }
        }

        // The plane path: the pages' client and their pictures, drawn once
        // now (rebuilt only when a page's draw list changes).
        let pc = if PLANES {
            gfx.client(PAGES_QUOTA)
        } else {
            None
        };
        // The look check reads the whole screen back: the image, the
        // service's owner here, grants it to its pages' client (audit C99:
        // no client asks for that itself).
        if LOOK_CHECK {
            gfx.grant_readback(pc);
        }
        // The radar's frames (R184 step 2): their own client, so their
        // pictures are counted and given back apart from the pages'.
        let Some(rc) = gfx.client(RADAR_QUOTA) else {
            crash(format_args!("no graphics client for the radar"))
        };
        let Some(cpu_layer) = Layer::new(layer_px, W as u32, PIC_ROWS as u32, W) else {
            crash(format_args!("page layer"))
        };
        let mut cpu = CpuRenderer::new(cpu_layer, plane_bytes, W * 4, PlaneFormat::Argb8888);
        cpu.set_pics(pics);
        let _ = writeln!(
            RunLog,
            "icons: the picture store {} slots, {} bytes; the default pack's file {DEF_FILE} bytes; {} pictures wanted",
            pics.slots(),
            rp_draw::pics::Pics::words(&PIC_CLASSES) * 4,
            shell.wanted().count(),
        );
        // Hourly's graph: one store picture painted once a forecast, drawn
        // the same on the planes and the 2D path.
        let mut graph_key: Option<u32> = None;
        let mut chart_key: Option<u32> = None;
        let t0 = rt::ticks();
        rp_today::keep_graph(shell, pics, &mut graph_key);
        let _ = writeln!(
            RunLog,
            "graph: Hourly's line painted in {} us ({})",
            rt::ticks().wrapping_sub(t0) / 13 * 4,
            if shell.graph().is_some() {
                "in the store"
            } else {
                "no slot"
            },
        );
        let mut pages: [Page; 3] = [None; 3];
        // The look (light or not) each page's picture and the app's chrome
        // picture were made in: a picture of the other look is never shown
        // (the owner, 2026-11-06, run r184s3c: after switching to dark in
        // Weather settings the light Now page showed before its rebuild,
        // "this looks broken"; the page's picture was handed over as built,
        // and a look change asked no page to be rebuilt ahead).
        let mut page_light: [Option<bool>; 3] = [None; 3];
        let mut chrome_light: Option<bool> = None;
        let mut last_light = shell.light();
        if let Some(pc) = pc {
            shell.use_planes(true);
            // The clock first: the Now page's Sunset bar follows it.
            shell.set_time(clock::time_at(card_min.unwrap_or_else(start_minutes)));
            for (tab, p) in pages.iter_mut().enumerate() {
                *p = build_page(gfx, pc, shell, &mut cpu, frame, tab as u8);
                page_light[tab] = Some(shell.light());
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
        // The opening runs on the page layer (no hand-off at its end).
        let mut reveal_on_page = false;
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
        // The opening's picture (the app composed ahead) while the screen
        // plane rests on it after the opening: the very pixels the opening
        // ended on, until something on the app changes or the pen comes
        // down (the owner's run, 2026-11-01: the end's swap to the page's
        // own picture changed every panel at once, ARGB4444 dithered for
        // the RGB565 the opening showed).
        let mut reveal_pic: Option<Handle> = None;
        let mut rest_on_app = false;
        let (mut reveal_t0, mut reveal_c0) = (0u64, 0u32);
        // Openings so far (the glide's alternation).
        let mut opens = 0u32;
        // Movements not given a line of their own (smooth, or a tap's two
        // frames).
        let mut movements_quiet = 0u32;
        let mut batch = Batch::default();
        let mut screen_logged = (rp_shell::Screen::Today, u8::MAX, u8::MAX);
        // The opening's trace: refreshes recorded, the last refresh's
        // count (`DSIG` is published at line 3 of the next), and how many
        // refreshes to keep recording after the opening ends.
        let trace = trace_words;
        let (mut trace_n, mut trace_on, mut trace_tail) = (0usize, false, 0u32);
        // The first refresh sampled after the opening ended (its end's
        // summary, every opening).
        let mut trace_end: Option<usize> = None;
        let mut last_line = 0u32;
        let mut trace_due = false;
        let mut meant = (0u16, 0u16, 0u16, 0u16, 0u16);
        let mut retire: Option<Handle> = None;
        // A page whose build found no room, by its draw list's key: not
        // tried again until that changes (today-0036 retried every pass,
        // 487 times).
        let mut no_room: [Option<u32>; 3] = [None; 3];
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
        let mut net: Option<Net> = None;
        // The dock's dev relay: USB up as a serial port (unplugged first,
        // so the PC sees a fresh device), asked once the PC opens it.
        if NET {
            gpio.borrow_mut().make_output(USB_PULLUP, false);
            rt::delay_us(200_000);
            if let Some(bus) = rt::take_udc() {
                let mut udc = Udc::new(bus);
                udc.set_layout(UsbLayout::Serial);
                match udc.enable() {
                    Ok(()) => {
                        gpio.borrow_mut().write(USB_PULLUP, true);
                        let _ = writeln!(
                            RunLog,
                            "net: USB serial up, waiting for the PC's relay (cargo xtask net)"
                        );
                        net = Some(Net {
                            udc,
                            out: [0; 96 + rp_weather::store::LEN],
                            out_n: 0,
                            out_at: 0,
                            waiting: false,
                            asked_ms: 0,
                            header: [0; 32],
                            hn: 0,
                            len: None,
                            got: 0,
                            tile: None,
                            pack: false,
                        });
                    }
                    Err(e) => {
                        let _ = writeln!(RunLog, "net: USB not enabled ({e:?})");
                    }
                }
            }
        }
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
        let start_min = card_min.unwrap_or_else(start_minutes);
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
        let mut backdrop_shown: Option<rp_weather::maplayers::Look> = None;
        let (mut burst_bg_px, mut burst_rows, mut burst_reused) = (0u64, 0u32, 0u32);
        let mut frames = 0u32;
        // Pictures the renderers drew nothing for (R181): all so far, the
        // CPU renderer's count when last read, the last one said and how
        // many lines (a picture missed every frame says so once).
        let mut pics_missed = 0u32;
        let mut cpu_missed_seen = 0u32;
        let mut missed_said: Option<(rp_draw::pics::Pic, rp_draw::pics::Miss)> = None;
        let mut missed_lines = 0u32;
        let mut overflow_logged = false;
        let mut draw_us_worst = 0u32;
        // When the worst frame was drawn and on what (R171: the budget's
        // failures name their frame).
        let mut draw_worst_at = (0u64, rp_shell::Screen::Today, 0u8, false);
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
        // R185's look check: passes judged right per look (dark, light) and
        // page, wrong, unknown; the look last seen chosen.
        let mut look_seen = [[0u32; 4]; 2];
        let mut look_wrong = 0u32;
        let mut look_unknown = 0u32;
        let mut look_last: Option<bool> = None;
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
            if LOOK_CHECK
                && let Some(pc) = pc
                && matches!(shell.screen(), rp_shell::Screen::Weather)
                && !shell.needs_frame()
                && !gfx.busy()
                && !gfx.commit_pending()
            {
                let want = shell.light();
                let tab = shell.weather_page().0;
                let look = &rp_today::look::CHROME;
                // The page is judged only at rest: while it grows in its
                // content passes under the points (X51v run today-0062: two
                // single passes on Hourly, the dark glass right, the points
                // on its graph); the chrome always.
                let page = if shell.animating() {
                    &[]
                } else {
                    rp_today::look::page_points(tab)
                };
                // The chrome and the page vote apart: a page of the wrong
                // look under the right chrome (run r184s3c) is wrong.
                let mut cpx = [None; rp_today::look::CHROME.len()];
                let mut ppx = [None; 9];
                for (o, p) in cpx.iter_mut().zip(look) {
                    *o = gfx.shown_at(pc, p.x, p.y);
                }
                // A page on the layer is judged only where the layer covers
                // it (a page growing in shows the wallpaper below its
                // window: the emulator's run, 2026-11-06).
                let on_layer = gfx.layer().is_some();
                for (o, p) in ppx.iter_mut().zip(page) {
                    *o = gfx
                        .shown_parts_at(pc, p.x, p.y)
                        .filter(|s| !on_layer || s.over.is_some())
                        .map(|s| s.out);
                }
                let chrome = rp_today::look::verdict(look, &cpx);
                let on_page = rp_today::look::verdict(page, &ppx[..page.len()]);
                // A Weather page the shell has on the layer but the display
                // shows no layer: the page is missing, whatever the pixels
                // say (the X51v's run today-0065: past Radar the bare
                // wallpaper showed, read "light", so only the dark look
                // caught it).
                let missing = !page.is_empty() && shell.page_layer().is_some() && !on_layer;
                let wrong = missing || chrome == Some(!want) || on_page == Some(!want);
                let moving = shell.animating() && !rp_today::look::page_points(tab).is_empty();
                let right = chrome == Some(want) && (page.is_empty() || on_page == Some(want));
                if wrong {
                    look_wrong += 1;
                    if look_wrong <= 8 {
                        let l = |v: &Option<u32>| v.map_or(0, rp_today::look::luma);
                        let (cl, pl) = (cpx.map(|v| l(&v)), ppx.map(|v| l(&v)));
                        let _ = writeln!(
                            RunLog,
                            "LOOK WRONG at {now_ms} ms: page {tab}, the look is {}; the chrome says {chrome:?} {cl:?}, the page {on_page:?} {:?} (Some(true) light){}",
                            if want { "light" } else { "dark" },
                            &pl[..page.len()],
                            if missing {
                                "; no page on the layer"
                            } else {
                                ""
                            },
                        );
                        // Why: each plane's part at the page's first
                        // points, and the layer as the service has it.
                        for p in page.iter().take(3) {
                            let _ = writeln!(
                                RunLog,
                                "  at ({}, {}): {:x?}; the layer {:?}",
                                p.x,
                                p.y,
                                gfx.shown_parts_at(pc, p.x, p.y),
                                gfx.layer()
                            );
                        }
                    }
                } else if moving {
                    // The chrome right, the page not judged while it moves.
                } else if right {
                    look_seen[usize::from(want)][usize::from(tab.min(3))] += 1;
                } else {
                    look_unknown += 1;
                }
                if look_last != Some(want) {
                    look_last = Some(want);
                    let _ = writeln!(
                        RunLog,
                        "look check: the {} look chosen at {now_ms} ms",
                        if want { "light" } else { "dark" }
                    );
                }
            }
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
                if total > SLOW_ANSWER_US && (!REPLAY || LOOK_CHECK) {
                    let _ = writeln!(
                        RunLog,
                        "slow answer at {} ms: {total} us; its frame's CPU {last_frame_us} us, wait {} us, engine {} us; {} glyph atlases begun, {} glyphs and {} pictures uploaded",
                        now_ms, s.wait_us, s.engine_us, s.atlases, s.glyphs, s.images
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
                    Some(_) if now_ms < REPLAY_HOLD_MS => None,
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

            if back && LOOK_CHECK {
                let _ = writeln!(
                    RunLog,
                    "look check: {look_wrong} wrong; right: dark on pages 0-3 {:?}, light on pages 0-3 {:?}; {look_unknown} passes unknown (too few points agreeing)",
                    look_seen[0], look_seen[1]
                );
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
                let _ = writeln!(
                    RunLog,
                    "worst draw: {draw_us_worst} us at {} ms on {:?} page {}{}",
                    draw_worst_at.0,
                    draw_worst_at.1,
                    draw_worst_at.2,
                    if draw_worst_at.3 { ", moving" } else { "" }
                );
                let cm = gfx.commits();
                let _ = writeln!(
                    RunLog,
                    "plane path: {} commits (window moves and hand-offs), {} ending outside the plane window (lines 640-642), the last on line {:?}",
                    cm.done, cm.late, cm.last_line
                );
                let _ = writeln!(
                    RunLog,
                    "gpu frames: worst CPU queueing {queue_worst} us, worst wait for the previous frame and scan-out {wait_worst} us, worst queue-to-flip {engine_worst} us, background rebuilt {rebuilds} times, pictures not drawn {pics_missed}; flips waited for the blanking {}, slipped a refresh {}; copied behind the scan {}, flipped as large {}, flipped with no copy window {}; copy times {:?} us",
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
                let _ = writeln!(
                    RunLog,
                    "movement: {movements_quiet} more at 30 fps or better, or of a frame or two (not logged one by one)"
                );
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
                if let Some(n) = net.as_mut() {
                    n.udc.disable();
                    gpio.borrow_mut().write(USB_PULLUP, false);
                }
                before.restore(&mut gfx.two_d().parts().0.regs);
                rp_image::finish()
            }

            // The pictures (option C, docs/design/weather-rule0.md): the
            // default pack's file read from the card a piece a pass while
            // nothing moves and kept packed in RAM; a pick of another pack
            // reads its table from the card; the store keeps what the
            // screens show, each picture decoded at its size from the pack
            // shown, else from the default (a pulled card: no gap, no box).
            let still = !filter.down() && !shell.animating() && !shell.needs_frame() && !gfx.busy();
            if still && def_index.is_none() && !def_failed {
                if !card_tried {
                    card_tried = true;
                    *card = take_card();
                }
                let l = def_load.get_or_insert(FileLoad {
                    got: 0,
                    part: 0,
                    off: 0,
                    read_us: 0,
                    from_cf: false,
                });
                let t0 = rt::ticks();
                let out = &mut def_file[l.got..];
                let got = if l.from_cf {
                    cf.as_mut()
                        .and_then(|(c, start)| read_chunk(c, *start, *b"RPIC0", l.part, l.off, out))
                } else {
                    card.as_mut()
                        .and_then(|(c, start)| read_chunk(c, *start, *b"RPIC0", l.part, l.off, out))
                };
                l.read_us += rt::ticks().wrapping_sub(t0) / 13 * 4;
                let done = match got {
                    Some(0) => {
                        l.part += 1;
                        l.off = 0;
                        l.part > 25 || l.got == def_file.len()
                    }
                    Some(k) => {
                        l.got += k;
                        l.off += k as u64;
                        false
                    }
                    None => true,
                };
                if done {
                    let (got, from_cf) = (l.got, l.from_cf);
                    def_index = rp_today::PackIndex::read(&def_file[..got])
                        .filter(|i| i.file_len() as usize <= got);
                    def_failed = def_index.is_none();
                    maker.forget();
                    let _ = writeln!(
                        RunLog,
                        "icons: the default pack {} ({got} bytes from the {} card in {} us, kept in RAM)",
                        if def_failed { "not usable" } else { "read" },
                        if from_cf { "CF" } else { "SD" },
                        l.read_us,
                    );
                    def_load = None;
                    if def_failed && !from_cf {
                        // Not on the SD card: the CF card, if the loader
                        // left one attached.
                        *cf = take_cf_card();
                        if cf.is_some() {
                            def_failed = false;
                            def_load = Some(FileLoad {
                                got: 0,
                                part: 0,
                                off: 0,
                                read_us: 0,
                                from_cf: true,
                            });
                        }
                    }
                    if def_failed && NET {
                        // On neither card: the PC through the dock.
                        relay_got = Some(0);
                    }
                }
            }
            if let Some(at) = relay_got
                && !relay_failed
                && !shell.wants(rp_shell::Ask::Pack)
            {
                shell.want_pack(at as u32);
            }
            let want = shell.icon_pack();
            if want != pack_shown {
                chosen = None;
                if want != 0 {
                    if !card_tried {
                        card_tried = true;
                        *card = take_card();
                    }
                    let t0 = rt::ticks();
                    match card
                        .as_mut()
                        .and_then(|(c, start)| read_index(c, *start, want, stream_buf))
                    {
                        Some(i) => chosen = Some((want, i)),
                        None => shell.icon_pack_failed(),
                    }
                    let _ = writeln!(
                        RunLog,
                        "icons: pack {want} {} in {} us",
                        if chosen.is_some() {
                            "chosen: its table read"
                        } else {
                            "not on the card"
                        },
                        rt::ticks().wrapping_sub(t0) / 13 * 4
                    );
                }
                pack_shown = chosen.map_or(0, |(n, _)| n);
                pages_check = true;
                app_check = true;
            }
            if still {
                // A new forecast's hours: Hourly's graph painted again.
                let t0 = rt::ticks();
                if rp_today::keep_graph(shell, pics, &mut graph_key) {
                    pages_check = true;
                    app_check = true;
                    let _ = writeln!(
                        RunLog,
                        "graph: Hourly's line painted again in {} us",
                        rt::ticks().wrapping_sub(t0) / 13 * 4
                    );
                }
                let tag = match chosen {
                    Some((n, _)) => n,
                    None if def_index.is_some() => 0,
                    None => NOT_YET,
                };
                let t_pass = rt::ticks();
                let mut made = 0u32;
                let (worst, card_failed) = (&mut batch.worst, &mut batch.card_failed);
                // One picture a pass: a decode at size is ~70 ms on the
                // X51v (the option C run, 2026-10-31: four a pass held the
                // screen ~0.17 s), so a touch waits at most one.
                while made < 1 {
                    let def_file: &[u8] = def_file;
                    let k = keeper.step(shell, pics, tag, |size, i, p| {
                        let mut pic = None;
                        if let Some((n, idx)) = chosen
                            && !*card_failed
                        {
                            let buf = &mut *stream_buf;
                            let card = &mut *card;
                            let read_failed = core::cell::Cell::new(false);
                            pic = maker.make(n, &idx, size, i, p, |b| {
                                let got = card.as_mut().and_then(|(c, start)| {
                                    let out = buf.get_mut(..b.len as usize)?;
                                    let stem = [b'R', b'P', b'I', b'C', b'0' + n];
                                    read_span(c, *start, stem, u64::from(b.off), out)
                                        .then_some(&*out)
                                });
                                read_failed.set(got.is_none());
                                got
                            });
                            // The card gone mid-run: the rest from the
                            // default file, quietly. Only a read that
                            // failed says so: a full store (the old picture
                            // still in its slot until the keeper frees it)
                            // was taken for a gone card, and every pick
                            // showed the default pack's pictures (the
                            // owner's run, 2026-10-31).
                            *card_failed |= read_failed.get();
                        }
                        if pic.is_none()
                            && (chosen.is_none() || *card_failed)
                            && let Some(idx) = def_index
                        {
                            pic = maker.make(0, &idx, size, i, p, |b| {
                                def_file.get(
                                    b.off as usize..(b.off as usize).checked_add(b.len as usize)?,
                                )
                            });
                        }
                        let (a, b) = maker.last;
                        if pic.is_some() && a + b > worst.0 + worst.1 {
                            *worst = (a, b);
                        }
                        pic
                    });
                    if k == rp_today::Kept::Idle {
                        break;
                    }
                    made += 1;
                    if matches!(k, rp_today::Kept::Made(..)) {
                        batch.made += 1;
                    }
                }
                let spent = rt::ticks().wrapping_sub(t_pass) / 13 * 4;
                if made > 0 {
                    pages_check = true;
                    app_check = true;
                    batch.us += spent;
                    batch.passes += 1;
                } else if batch.passes > 0 {
                    // The batch done (the store holds what the screens
                    // show): one line for it, not one a pass (a line a
                    // pass filled the 64 KiB log, emu-today 2026-10-31).
                    if batch.made > 0 {
                        let _ = writeln!(
                            RunLog,
                            "icons: {} pictures made in {} passes, {} us in all (slowest {} us: inflate {} us, shrink {} us; {} inflates so far; {} of {} slots){}",
                            batch.made,
                            batch.passes,
                            batch.us,
                            batch.worst.0 + batch.worst.1,
                            batch.worst.0,
                            batch.worst.1,
                            maker.inflates,
                            pics.used(),
                            pics.slots(),
                            if batch.card_failed {
                                "; the card failed, the rest from the default"
                            } else {
                                ""
                            },
                        );
                        cards_line(shell, pics);
                    }
                    batch = Batch::default();
                }
            }

            // What the owner sees, by time: each screen and Weather page
            // (the run log read against the eye's answers).
            let seen = (shell.screen(), shell.weather_page().0, shell.icon_pack());
            if seen != screen_logged {
                screen_logged = seen;
                let _ = writeln!(
                    RunLog,
                    "screen: {:?}, page {}, icon pack {} at {now_ms} ms",
                    seen.0, seen.1, seen.2
                );
                if seen.0 == rp_shell::Screen::Today {
                    cards_line(shell, pics);
                }
            }

            // The dock's dev relay: the port polled each pass; the shell's
            // next question sent when none is waiting, its answer read into
            // the RAM pool and handed back whole (weather-settings-build.md
            // "The dock link, version 2").
            if let Some(n) = net.as_mut() {
                n.udc.poll();
                // Closed before an answer (another program had the port,
                // `load` reading the log say): asked again on the next open.
                if n.waiting && !n.udc.host_open() {
                    shell.net_retry();
                    n.reset();
                }
                if !n.waiting
                    && n.udc.configured()
                    && n.udc.host_open()
                    && let Some(q) = shell.next_ask()
                {
                    let line = q.line();
                    n.out[..line.len()].copy_from_slice(line);
                    n.out_n = line.len();
                    if let Some(b) = q.body {
                        n.out[n.out_n..n.out_n + b.len()].copy_from_slice(&b);
                        n.out_n += b.len();
                    }
                    n.out_at = 0;
                    n.tile = q.tile;
                    n.pack = q.ask == rp_shell::Ask::Pack;
                    n.waiting = true;
                    n.asked_ms = now_ms;
                    let _ = writeln!(
                        RunLog,
                        "net: asked {:?} at {now_ms} ms",
                        core::str::from_utf8(q.logged()).unwrap_or("?").trim_end()
                    );
                }
                // What is left of the question, as the port takes it.
                if n.out_at < n.out_n {
                    n.out_at += n.udc.write(&n.out[n.out_at..n.out_n]);
                }
                let mut chunk = [0u8; 64];
                loop {
                    let k = n.udc.read(&mut chunk);
                    if k == 0 {
                        break;
                    }
                    // Nothing counts before a question (a loader command
                    // from the PC on the same port, say).
                    if !n.waiting {
                        continue;
                    }
                    for &b in &chunk[..k] {
                        match n.len {
                            None => {
                                if b == b'\n' {
                                    // "RPNET <status> <length>"
                                    let h = core::str::from_utf8(&n.header[..n.hn]).unwrap_or("");
                                    let mut f = h.split(' ');
                                    let ok = f.next() == Some("RPNET") && f.next() == Some("200");
                                    let len = f.next().and_then(|l| l.parse::<usize>().ok());
                                    match (ok, len) {
                                        (true, Some(l)) if l <= net_buf.len() => n.len = Some(l),
                                        _ => {
                                            relay_failed |= n.pack;
                                            let _ =
                                                writeln!(RunLog, "net: the relay answered {h:?}");
                                            shell.net_answer(false, &[]);
                                            n.reset();
                                        }
                                    }
                                } else if n.hn < n.header.len() {
                                    n.header[n.hn] = b;
                                    n.hn += 1;
                                }
                            }
                            Some(l) if n.got < l => {
                                net_buf[n.got] = b;
                                n.got += 1;
                            }
                            Some(_) => {}
                        }
                    }
                }
                if let Some(l) = n.len
                    && n.got == l
                {
                    let before = *shell.forecast();
                    // A tile: decoded here into the map or its rain frame
                    // (the shell only moves on to the next).
                    if let Some((None, _, _)) = n.tile {
                        use rp_weather::mapcard::HEAD;
                        if l == 0 {
                            // The check of the card's map (R184 step 3): the
                            // relay's is the same.
                            let _ = writeln!(
                                RunLog,
                                "radar map: the card's is the relay's ({} ms to check)",
                                now_ms.wrapping_sub(n.asked_ms)
                            );
                            shell.net_answer(true, &[]);
                        } else {
                            // The place's map: its layers in one answer (R184),
                            // the first for the place or the relay's newer one
                            // (a card read under way stops), kept to write to
                            // the card in a later pass.
                            let key = shell.radar_key();
                            let t0 = rt::ticks();
                            let got = match map_file.get_mut(HEAD..HEAD + l) {
                                Some(f) => {
                                    f.copy_from_slice(&net_buf[..l]);
                                    rp_weather::maplayers::unpack(f, map_layers)
                                }
                                None => Err(rp_weather::maplayers::Error::Room),
                            };
                            let _ = writeln!(
                                RunLog,
                                "radar map: the place's layers, {l} bytes, unpacked {got:?} in {} us",
                                rt::ticks().wrapping_sub(t0) / 13 * 4
                            );
                            layers_of = got.is_ok().then_some(MapOf::Place);
                            coloured = None;
                            if got.is_ok() {
                                kept.key = Some(key);
                                kept.len = l;
                                kept.crc = rp_weather::store::crc32(&map_file[HEAD..HEAD + l]);
                                kept.ram = Some(key);
                                kept.step = MapStep::Write { key, wait: true };
                            } else {
                                kept.key = None;
                                kept.ram = None;
                                kept.step = MapStep::Idle;
                            }
                            shell.net_answer(got.is_ok(), &[]);
                            if got.is_ok() {
                                shell.radar_map_in();
                            }
                        }
                    } else if let Some((Some(f), k, _)) = n.tile {
                        // Kept as it came (R184 step 2): decoded when its
                        // frame is composed. A frame's first tile starts it
                        // afresh.
                        let f = usize::from(f);
                        if filling != Some(f) {
                            rain_store.clear(f);
                            filling = Some(f);
                        }
                        let ok = rain_store.put(f, usize::from(k), &net_buf[..l]);
                        if !ok {
                            let _ = writeln!(
                                RunLog,
                                "radar: frame {f}'s tile {k} ({l} bytes) not kept: the rain's store is full ({} bytes held): asked again once, never shown dry",
                                rain_store.used()
                            );
                        }
                        shell.net_answer(true, &[]);
                        if !ok {
                            // Audit C97: never composed without its rain.
                            rain_store.clear(f);
                            shell.radar_forget(f as u8);
                        }
                    } else if n.pack {
                        // The default pack's next piece, kept where the
                        // card's copy would be; a short piece is its end.
                        let at = relay_got.unwrap_or(0);
                        let fits = def_file.get_mut(at..at + l);
                        let ok = fits.is_some();
                        if let Some(d) = fits {
                            d.copy_from_slice(&net_buf[..l]);
                            relay_got = Some(at + l);
                        }
                        shell.net_answer(ok, &[]);
                        if !ok || l < rp_weather::settings::PACK_CHUNK {
                            let got = relay_got.unwrap_or(0);
                            def_index = rp_today::PackIndex::read(&def_file[..got])
                                .filter(|i| ok && i.file_len() as usize <= got);
                            relay_failed = def_index.is_none();
                            relay_got = None;
                            maker.forget();
                            let _ = writeln!(
                                RunLog,
                                "icons: the default pack {} ({got} bytes from the PC through the dock, kept in RAM)",
                                if relay_failed { "not usable" } else { "read" },
                            );
                        }
                    } else {
                        shell.net_answer(true, &net_buf[..l]);
                        // A radar refresh kept frames by their time (P7).
                        if let Some(m) = shell.take_radar_moves() {
                            // Only the frame-to-row map moves: the kept PNGs
                            // and composed bands stay in their rows; the rows
                            // whose frame went give their band slots back
                            // (the bands first stop reading them).
                            let gone = rain_store.moved(&m);
                            if let Some(j) = radar_job.take() {
                                j.give_back(&mut band_pool);
                            }
                            banded =
                                banded.and_then(|b| m.iter().position(|&j| usize::from(j) == b));
                            if banded.is_none() {
                                if let Ok((a, n)) =
                                    show_bands(gfx, rc, &band_pool, None, &mut spans_shown)
                                {
                                    a[..n].iter().for_each(|&r| shell.redraw(r));
                                }
                            }
                            for r in 0..rp_weather::radar::PAST {
                                if gone & 1 << r != 0 {
                                    give_back(&mut band_pool, &mut rain_store.take_row(r));
                                }
                            }
                            filling = None;
                            let _ = writeln!(
                                RunLog,
                                "radar: {} frames kept by their time, the rest asked",
                                m.iter().filter(|&&j| j != u8::MAX).count()
                            );
                        }
                    }
                    let after = shell.forecast();
                    if *after != before {
                        pages_check = true;
                        app_check = true;
                    }
                    let _ = writeln!(
                        RunLog,
                        "net: {l} bytes answered at {now_ms} ms ({} ms after asking){}",
                        now_ms.wrapping_sub(n.asked_ms),
                        if *after != before {
                            ", the forecast changed"
                        } else {
                            ""
                        }
                    );
                    n.reset();
                }
                // No answer in time: the shell decides what still matters.
                if n.waiting && now_ms.wrapping_sub(n.asked_ms) > NET_TIMEOUT_MS {
                    let _ = writeln!(RunLog, "net: no answer in {NET_TIMEOUT_MS} ms");
                    shell.net_answer(false, &[]);
                    n.reset();
                }
            }

            // A look change (Settings' Appearance, or Auto at sunrise or
            // sunset): every page and the app's pictures are made again
            // ahead, while still, as for any change of their draw lists (run
            // r184s3c: none was asked, and the light page showed after the
            // switch to dark).
            if shell.light() != last_light {
                last_light = shell.light();
                pages_check = true;
                app_check = true;
                let _ = writeln!(
                    RunLog,
                    "look: {} at {now_ms} ms: the pages and the app's pictures made again ahead",
                    if last_light { "light" } else { "dark" }
                );
            }

            // The place's radar map kept on the card (R184 step 3, way A):
            // looked for in the pass Radar asks for it (before the frame
            // list can answer), read a piece a pass beside the rain's asks,
            // unpacked a layer a pass, and the dock's written there once.
            kept_map_pass(
                shell,
                &mut kept,
                card,
                &mut card_tried,
                map_file,
                map_layers,
                map_scratch,
                &mut layers_of,
                &mut coloured,
                filter.down(),
                now_ms,
            );

            // The radar's map under the page while Radar is open, the
            // wallpaper again after (the frame in flight may read it).
            // The real radar once its map is in: the frame shown blended
            // over it.
            // The living sky in the log (the owner, 2026-11-06: "it doesnt
            // do the animation at all"): when it starts or stops and why,
            // and while it runs its steps and moves every 5 s.
            let sky = shell.sky_report();
            let key = (sky.live, sky.why, sky.test, sky.moving);
            if sky_logged.map(|(k, _, _, _)| k) != Some(key) {
                let _ = writeln!(
                    RunLog,
                    "sky: {} (test {} {}; the weather shown {}) at {now_ms} ms",
                    if sky.live { "running" } else { sky.why },
                    sky.test,
                    sky.test_name,
                    if sky.moving {
                        "moves"
                    } else {
                        "has nothing that moves"
                    }
                );
                sky_logged = Some((key, now_ms, sky.steps, sky.moved));
            } else if let Some((k, at, steps, moved)) = sky_logged
                && sky.live
                && now_ms.wrapping_sub(at) >= 5000
            {
                let _ = writeln!(
                    RunLog,
                    "sky: {} steps, {} of them moved something, in {} ms",
                    sky.steps.wrapping_sub(steps),
                    sky.moved.wrapping_sub(moved),
                    now_ms.wrapping_sub(at)
                );
                sky_logged = Some((k, now_ms, sky.steps, sky.moved));
            }
            let real = shell.backdrop().and(shell.radar_real());
            let look = shell.backdrop();
            if real.is_some() {
                // The place's map: its layers coloured in the look shown
                // when either changed (Paper, Ink or Night from the same
                // layers: R184, way C), written once into the wallpaper;
                // the rain frames composed over it once each, a step a
                // pass, into band slots (R184 step 2), a loop step only
                // switching the wallpaper's bands to the frame shown.
                let changed = recolour(
                    MapOf::Place,
                    look,
                    layers_of,
                    &mut coloured,
                    map_layers,
                    radar_view_map,
                );
                let have = shell.radar_have();
                if changed || look != backdrop_shown {
                    // A new map or look: every composed frame is the old
                    // one's, given back (bands off first) and made again.
                    if let Ok((a, n)) = show_bands(gfx, rc, &band_pool, None, &mut spans_shown) {
                        a[..n].iter().for_each(|&r| shell.redraw(r));
                    }
                    banded = None;
                    if let Some(j) = radar_job.take() {
                        j.give_back(&mut band_pool);
                    }
                    for f in 0..rp_weather::radar::PAST {
                        give_back(&mut band_pool, &mut rain_store.take_composed(f));
                    }
                    no_band_room = false;
                    let t0 = rt::ticks();
                    if layers_of != Some(MapOf::Place) {
                        // The place's map failed: dark water, never another
                        // place's map.
                        radar_view_map.fill(0xFF06_0E18);
                        coloured = None;
                    }
                    rp_today::show_radar(gfx.two_d(), radar_view_map, None);
                    backdrop_shown = look;
                    let _ = writeln!(
                        RunLog,
                        "radar: the map into the wallpaper in {} us",
                        rt::ticks().wrapping_sub(t0) / 13 * 4
                    );
                }
                // A frame no longer in (fetched again, or a new list) is
                // given back, and its job dropped.
                if radar_job.is_some_and(|j| have & 1 << j.f == 0)
                    && let Some(j) = radar_job.take()
                {
                    j.give_back(&mut band_pool);
                }
                for f in 0..rp_weather::radar::PAST {
                    if have & 1 << f == 0 && rain_store.composed(f).is_some() {
                        if banded == Some(f)
                            && let Ok((a, n)) =
                                show_bands(gfx, rc, &band_pool, None, &mut spans_shown)
                        {
                            a[..n].iter().for_each(|&r| shell.redraw(r));
                            banded = None;
                        }
                        give_back(&mut band_pool, &mut rain_store.take_composed(f));
                        no_band_room = false;
                    }
                }
                // Room for the frames, once per opening (the owner's way A,
                // 2026-11-06): the pages not on screen give their pictures
                // back while Radar needs the room, Hourly's first (the
                // largest), the others only if that isn't enough; each is
                // made again when Radar closes. Phones and browsers drop
                // what isn't on screen the same way; here the pages' own
                // client gives them back (ADR-0027: no service eviction).
                // Only with the page layer off: a page lent while the layer
                // still showed one stopped the plane path from turning it
                // off, and Now stayed over the radar (run today-0055).
                if !bands_tried && have != 0 && gfx.layer().is_none() && !gfx.commit_pending() {
                    bands_tried = true;
                    while matches!(gfx.poll(&mut gclock), Step::Wait(_)) {}
                    for round in [&[1usize][..], &[0, 2][..]] {
                        if band_pool.total() >= rp_today::rain::SLOTS {
                            break;
                        }
                        if let Some(pc) = pc {
                            for &i in round {
                                if let Some((h, k)) = pages[i].take() {
                                    if gfx.free_picture(pc, h).is_ok() {
                                        pages_lent |= 1 << i;
                                    } else {
                                        pages[i] = Some((h, k));
                                    }
                                }
                            }
                        }
                        for h in band_pool.take_pics().into_iter().flatten() {
                            let _ = gfx.free_picture(rc, h);
                        }
                        take_band_pics(gfx, rc, &mut band_pool);
                    }
                    let _ = writeln!(
                        RunLog,
                        "radar: the pages lent their pictures for it: {pages_lent:#05b} (Hourly 0b010)"
                    );
                }
                // The frame to show composed first; the others behind it,
                // the newest first, while there are slots (never by giving
                // way: run today-0054's frames 1 and 2 took each other's
                // slots 83 times).
                let want = real.flatten().map(usize::from);
                let needed = |f: usize| have & 1 << f != 0 && rain_store.composed(f).is_none();
                if let Some(w) = want.filter(|&w| needed(w))
                    && radar_job.is_some_and(|j| j.f != w)
                    && let Some(j) = radar_job.take()
                {
                    j.give_back(&mut band_pool);
                }
                if radar_job.is_none() {
                    radar_job = want.filter(|&w| needed(w)).map(Job::new).or_else(|| {
                        (0..rp_weather::radar::PAST)
                            .rev()
                            .find(|&f| needed(f) && !no_band_room && band_pool.free() > 0)
                            .map(Job::new)
                    });
                }
                if let (Some(mut job), Some(l)) = (radar_job, look) {
                    while matches!(gfx.poll(&mut gclock), Step::Wait(_)) {}
                    let mut got = job_step(
                        gfx,
                        rc,
                        &mut band_pool,
                        &rain_store,
                        &mut job,
                        radar_view_map,
                        l,
                        &mut scratch,
                    );
                    // No slot for the frame on screen: one composed frame
                    // not shown gives way (one, then the next pass tries
                    // again); for one behind it, it waits for room.
                    if matches!(got, Err(rp_graphics::Error::NoMemory)) {
                        let victim = (Some(job.f) == want)
                            .then(|| {
                                (0..rp_weather::radar::PAST).find(|&j| {
                                    Some(j) != banded
                                        && rain_store
                                            .composed(j)
                                            .is_some_and(|c| c.slots.iter().any(Option::is_some))
                                })
                            })
                            .flatten();
                        match victim {
                            Some(j) => {
                                give_back(&mut band_pool, &mut rain_store.take_composed(j));
                                let _ = writeln!(
                                    RunLog,
                                    "radar: frame {j}'s bands given way for {}",
                                    job.f
                                );
                                got = Ok(false);
                            }
                            None => {
                                no_band_room = true;
                                let _ = writeln!(
                                    RunLog,
                                    "radar: no band slot for frame {} ({} of {} free): it waits",
                                    job.f,
                                    band_pool.free(),
                                    band_pool.total()
                                );
                            }
                        }
                    }
                    match got {
                        Ok(true) if job.bad => {
                            // Unreadable (audit C97): never shown dry.
                            job.give_back(&mut band_pool);
                            rain_store.clear(job.f);
                            radar_job = None;
                            shell.radar_forget(job.f as u8);
                        }
                        Ok(true) => {
                            let bands = job.comp.spans.iter().filter(|b| b.is_some()).count();
                            rain_store.set_composed(job.f, Some(job.comp));
                            radar_job = None;
                            let _ = writeln!(
                                RunLog,
                                "radar: frame {} composed over the map in {} us of steps ({} decoding, {} composing; {} us since it began; {bands} of {} bands rain; {} of {} band slots free; the rain's PNGs {} bytes)",
                                job.f,
                                job.decode_us + job.write_us,
                                job.decode_us,
                                job.write_us,
                                rt::ticks().wrapping_sub(job.t0) / 13 * 4,
                                rp_today::rain::BANDS,
                                band_pool.free(),
                                band_pool.total(),
                                rain_store.used()
                            );
                        }
                        Ok(false) => radar_job = Some(job),
                        Err(e) => {
                            job.give_back(&mut band_pool);
                            radar_job = None;
                            if !matches!(e, rp_graphics::Error::NoMemory) {
                                let _ = writeln!(
                                    RunLog,
                                    "radar: frame {} not composed ({e:?}): asked again once",
                                    job.f
                                );
                                // Audit C97: asked again once, then lost.
                                rain_store.clear(job.f);
                                shell.radar_forget(job.f as u8);
                            }
                        }
                    }
                    // The frame shown with no room to compose it (audit
                    // C97): the radar says the rain is unavailable.
                    let shown_waits = no_band_room
                        && want.is_some_and(|w| {
                            have & 1 << w != 0
                                && rain_store.composed(w).is_none()
                                && radar_job.is_none_or(|j| j.f == w)
                        });
                    shell.set_radar_rain_room(!shown_waits && band_pool.total() > 0);
                }
                let ready = (0..rp_weather::radar::PAST)
                    .filter(|&f| rain_store.composed(f).is_some())
                    .fold(0u16, |m, f| m | 1 << f);
                shell.set_radar_ready(ready);
                // A loop step: the bands switched to the frame shown.
                let target = want.filter(|&f| rain_store.composed(f).is_some());
                if target != banded {
                    let t0 = rt::ticks();
                    let c = target.and_then(|f| rain_store.composed(f));
                    match show_bands(gfx, rc, &band_pool, c.as_ref(), &mut spans_shown) {
                        Ok((a, n)) => {
                            banded = target;
                            a[..n].iter().for_each(|&r| shell.redraw(r));
                            let px: u32 =
                                a[..n].iter().map(|r| u32::from(r.w) * u32::from(r.h)).sum();
                            let _ = writeln!(
                                RunLog,
                                "radar: frame {target:?} shown in {} us ({n} areas, {px} px redrawn)",
                                rt::ticks().wrapping_sub(t0) / 13 * 4,
                            );
                        }
                        Err(e) => {
                            let _ = writeln!(RunLog, "radar: frame {target:?} not shown ({e:?})");
                        }
                    }
                }
            } else {
                bands_tried = false;
                no_band_room = false;
                // The radar's frames given back while it isn't shown (the
                // pages have the pool to themselves again); their PNGs kept;
                // the pages that lent their pictures made again now.
                if band_pool.total() > 0 || radar_job.is_some() {
                    if let Ok((a, n)) = show_bands(gfx, rc, &band_pool, None, &mut spans_shown) {
                        a[..n].iter().for_each(|&r| shell.redraw(r));
                    }
                    banded = None;
                    radar_job = None;
                    for f in 0..rp_weather::radar::PAST {
                        rain_store.set_composed(f, None);
                    }
                    shell.set_radar_ready(0);
                    while matches!(gfx.poll(&mut gclock), Step::Wait(_)) {}
                    for h in band_pool.take_pics().into_iter().flatten() {
                        let _ = gfx.free_picture(rc, h);
                    }
                    let _ = writeln!(RunLog, "radar: its band pictures given back");
                }
                // Only the page about to be shown is made now; the others
                // the radar borrowed are made ahead while still, one a pass
                // (item 4, R172: the X51v's run r184s3d held the loop 465 ms
                // leaving Radar for Weather settings, 212 ms of it Hourly's
                // picture, which settings never shows).
                let now_tab = matches!(shell.screen(), rp_shell::Screen::Weather)
                    .then(|| shell.weather_page().0)
                    .filter(|&t| t < 3 && pages_lent & 1 << t != 0);
                if pages_lent != 0 && now_tab.is_none() {
                    pages_check = true;
                }
                if let (Some(t), Some(pc)) = (now_tab, pc) {
                    let t0 = rt::ticks();
                    let i = usize::from(t);
                    if pages[i].is_none() {
                        pages[i] = build_page(gfx, pc, shell, &mut cpu, frame, t);
                        page_light[i] = Some(shell.light());
                    }
                    let _ = writeln!(
                        RunLog,
                        "radar: the page shown ({t}) of those it borrowed from ({pages_lent:#05b}) made again in {} us{}",
                        rt::ticks().wrapping_sub(t0) / 13 * 4,
                        if pages[i].is_some() {
                            ""
                        } else {
                            ": not made (no room), the plane path off"
                        }
                    );
                    pages_lent &= !(1 << i);
                    pages_check |= pages_lent != 0;
                }
                // No map of the place yet, or Radar closed: the wallpaper.
                let (changed, want) = (false, None);
                if changed || want != backdrop_shown {
                    while matches!(gfx.poll(&mut gclock), Step::Wait(_)) {}
                    let t0 = rt::ticks();
                    rp_today::show_backdrop(
                        gfx.two_d(),
                        wallpaper,
                        want.map(|_| &*radar_view_map),
                        wall,
                    );
                    let _ = writeln!(
                        RunLog,
                        "backdrop: {} in {} us",
                        if want.is_some() {
                            "radar map"
                        } else {
                            "wallpaper"
                        },
                        rt::ticks().wrapping_sub(t0) / 13 * 4
                    );
                    backdrop_shown = want;
                }
            }

            // The plane path: what the page layer shows. A hand-off (the
            // layer on, off, another page, a page redrawn) is a commit with
            // this pass's frame; a move only repoints it, no engine.
            // (the layer, the frame kept aside, this pass's 2D frame drawn
            // with it)
            let mut commit: Option<(Option<LayerState>, bool, bool)> = None;
            // Past Radar, a page it borrowed and that isn't shown is made
            // again ahead while still (below), so it may be missing here;
            // the one to show is made now. The X51v's run today-0065
            // (2026-11-08): this block waited for every page, so leaving
            // Radar for Now (Hourly still lent) put no page back on the
            // layer, the bare wallpaper showing 4 s each time, until
            // Settings; and the rebuild ahead, inside this block, never ran.
            let on_radar =
                shell.weather_page().0 >= 3 && matches!(shell.screen(), rp_shell::Screen::Weather);
            if let Some(pc) = pc
                && !on_radar
                && let Some(v) = shell.page_layer()
            {
                let i = usize::from(v.tab);
                if i < 3 && pages[i].is_none() && pages_lent & 1 << i != 0 {
                    let t0 = rt::ticks();
                    pages[i] = build_page(gfx, pc, shell, &mut cpu, frame, v.tab);
                    page_light[i] = Some(shell.light());
                    if pages[i].is_some() {
                        pages_lent &= !(1 << i);
                    }
                    let _ = writeln!(
                        RunLog,
                        "radar: page {i}, lent to it, made again to be shown in {} us{}",
                        rt::ticks().wrapping_sub(t0) / 13 * 4,
                        if pages[i].is_some() {
                            ""
                        } else {
                            ": not made (no room), the plane path off"
                        }
                    );
                }
            }
            let shown_want = shell.page_layer().map(|v| usize::from(v.tab));
            let pages_ready = pages.iter().enumerate().all(|(i, p)| {
                p.is_some() || (!on_radar && pages_lent & 1 << i != 0 && shown_want != Some(i))
            });
            if let Some(pc) = pc.filter(|_| pages_ready) {
                if !gfx.busy() && !gfx.commit_pending() {
                    // Resting on the opening's picture: handed to the app's
                    // own planes (its chrome drawn, its page on the layer)
                    // once anything on it changes or the pen comes down,
                    // so the change shows with the swap, never on its own.
                    let wake = if !rest_on_app {
                        None
                    } else if filter.down() {
                        Some("the pen down")
                    } else if shell.animating() {
                        Some("the app moving")
                    } else if shell.page_layer().is_none() {
                        Some("no page on the layer")
                    } else if shell.needs_frame() {
                        Some(if shell.only_sky_changed() {
                            "the sky's area changed"
                        } else {
                            "the app changed"
                        })
                    } else {
                        None
                    };
                    if let Some(why) = wake {
                        rest_on_app = false;
                        // The page the app hands over to, built now if it
                        // is out of date (the screen shows the opening's
                        // picture meanwhile, as right as ever): never an
                        // old page swapped in, then rebuilt.
                        if let Some(v) = shell.page_layer() {
                            let i = usize::from(v.tab);
                            let key = page_key(shell, frame, v.tab);
                            if pages[i].is_none_or(|(_, k)| k != key) {
                                if let Some((h, _)) = pages[i].take() {
                                    let _ = gfx.free_picture(pc, h);
                                }
                                pages[i] = build_page(gfx, pc, shell, &mut cpu, frame, v.tab);
                                page_light[i] = Some(shell.light());
                            }
                        }
                        shell.redraw_all();
                        let want = shell.page_layer().and_then(|v| {
                            pages[usize::from(v.tab)].map(|(h, _)| (v.tab, page_state(h, v)))
                        });
                        commit = Some((want.map(|(_, st)| st), false, true));
                        shown_tab = want.map(|(t, _)| t);
                        pages_check = true;
                        let _ = writeln!(
                            RunLog,
                            "rest: {now_ms} ms, the opening's picture handed to the app's planes ({why})"
                        );
                    }
                    if commit.is_some() {
                        // Handed over this pass.
                    } else if let Some((r, shift)) = shell.reveal_plane() {
                        if !gfx.aside_held() && !revealing {
                            // The app drawn whole aside, shown through the
                            // card's window, gliding from where the card is.
                            // (The 2026-10-30 test held every second opening
                            // still; left in, it jumped the app's lines 28 px
                            // in their first refresh: the owner's video,
                            // 2026-11-01, "just as jarring".)
                            opens += 1;
                            shell.set_reveal_glide(true);
                            // Never an old picture: the app composed ahead
                            // is checked against the app now (the owner's
                            // run, 2026-11-01: in the light look the opening
                            // grew the dark app composed two minutes before,
                            // the living sky having kept the Today screen
                            // from ever being still). Stale, it is composed
                            // now, aside.
                            shell.weather_preview(frame);
                            let now_key = rp_today::frame_key(frame, rp_today::KEY_START);
                            frame.clear();
                            let ahead = app_pic.filter(|&(_, k)| k == now_key);
                            let _ = writeln!(
                                RunLog,
                                "open {}: the app composed ahead {}",
                                opens,
                                match (app_pic, ahead) {
                                    (None, _) => "not yet: composed now, aside",
                                    (Some(_), None) => "stale: composed now, aside",
                                    (Some(_), Some(_)) => "matches the app now",
                                }
                            );
                            let (r, shift) = shell.reveal_plane().unwrap_or((r, shift));
                            let _ = writeln!(
                                RunLog,
                                "open {opens}: Reveal with the glide (the app read 28 rows lower, easing up)"
                            );
                            // On the page layer when the app's screen and
                            // page are ready: the screen plane on the app's
                            // screen and the window over its page, in one
                            // commit at the next refresh; the opening then
                            // ends as the app rests, with no hand-off (the
                            // owner's run, 2026-10-31: the end's hand-off
                            // changed the picture twice in 80 ms).
                            // The whole app composed ahead grows from the
                            // card over Today when it is ready, Today kept
                            // round it (the owner's run, 2026-11-01: on the
                            // page layer the first refresh swapped the whole
                            // screen to the app's frame, Today's rows gone
                            // and the header in at once, "still looks
                            // jarring"); the page layer only without it.
                            let on_page = app_pic
                                .is_none()
                                .then_some(chrome_pic)
                                .flatten()
                                .filter(|_| chrome_light == Some(shell.light()))
                                .zip(shell.reveal_page())
                                .and_then(|((ch, _), v)| {
                                    pages[usize::from(v.tab)].map(|(h, _)| (ch, h, v))
                                });
                            if let Some((ch, h, v)) = on_page {
                                if gfx.commit_screen(pc, ch, Some(page_state(h, v))).is_ok() {
                                    revealing = true;
                                    reveal_on_page = true;
                                    reveal_shown = true;
                                    shown_tab = None;
                                    reveal_t0 = now_ms;
                                    reveal_c0 = gfx.commits().done;
                                    trace_n = 0;
                                    trace_end = None;
                                    trace_on = true;
                                    trace_tail = 30;
                                }
                            } else {
                                reveal_pic = ahead.map(|(h, _)| h);
                                commit = Some(match ahead {
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
                            }
                            reveal_t0 = now_ms;
                            reveal_c0 = gfx.commits().done;
                            trace_n = 0;
                            trace_end = None;
                            trace_on = true;
                            trace_tail = 30;
                        }
                    } else if revealing
                        && let Some((ch, _)) =
                            chrome_pic.filter(|_| chrome_light == Some(shell.light()))
                        && let Some(v) = shell.page_layer()
                        && let Some((h, _)) = pages[usize::from(v.tab)]
                    {
                        // Reveal done: the screen plane on the app's screen
                        // composed ahead and its page on the layer, in one
                        // commit at the next refresh, nothing drawn (the
                        // trace, 2026-10-31: drawing that screen during
                        // Reveal froze it two refreshes). The renderer's own
                        // screens come back with the next frame.
                        if reveal_on_page {
                            // Already as it rests: the window to the page's.
                            let _ = gfx.move_layer(pc, page_state(h, v).win);
                            shown_tab = Some(v.tab);
                            screen_on_pic = true;
                        } else if reveal_pic.take().is_some() && gfx.move_layer(pc, WHOLE).is_ok() {
                            // The layer's window over the whole of the
                            // opening's own picture: the same pixels, its
                            // title bar in (below the bar, nothing changes).
                            // The layer stays on (switched off here and on
                            // again at the hand-over, the run of 2026-11-01
                            // showed one wrong refresh). Rests there until
                            // the app changes or is touched.
                            rest_on_app = true;
                            shown_tab = None;
                        } else {
                            let _ = gfx.commit_screen(pc, ch, Some(page_state(h, v)));
                            shown_tab = Some(v.tab);
                            // Kept on the picture until something on the
                            // screen changes (below): nothing drawn now.
                            screen_on_pic = true;
                        }
                        reveal_on_page = false;
                        let n = gfx.commits().done - reveal_c0;
                        let ms = (now_ms - reveal_t0).max(1);
                        let _ = writeln!(
                            RunLog,
                            "open: {now_ms} ms, Reveal done: {n} window moves in {ms} ms (~{} fps); {}",
                            u64::from(n) * 1000 / ms,
                            if rest_on_app {
                                "the screen on the opening's own picture, the layer off, resting"
                            } else {
                                "the screen on its picture composed ahead, the page on the layer, one commit"
                            }
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
                            // Reveal done over the picture composed ahead:
                            // the screen plane rests on it (as above), or
                            // the screen's chrome and the page layer in one
                            // commit (below), its frames counted.
                            if reveal_pic.take().is_some() && gfx.move_layer(pc, WHOLE).is_ok() {
                                rest_on_app = true;
                                shown_tab = None;
                            }
                            let n = gfx.commits().done - reveal_c0;
                            let ms = (now_ms - reveal_t0).max(1);
                            let _ = writeln!(
                                RunLog,
                                "open: {now_ms} ms, Reveal done: {n} window moves in {ms} ms (~{} fps); {}",
                                u64::from(n) * 1000 / ms,
                                if rest_on_app {
                                    "the screen on the opening's own picture, the layer off, resting"
                                } else {
                                    "the screen and the page in one commit next"
                                }
                            );
                            revealing = false;
                            reveal_shown = false;
                        }
                        let want = shell.page_layer();
                        let still = !(shell.animating() || filter.down());
                        if rest_on_app {
                            // Resting on the opening's picture (below).
                        } else if want.map(|v| v.tab) != shown_tab {
                            // The picture as built: nothing is checked or
                            // drawn on the tap (the first device run: 40-70
                            // ms of CPU here); checks run while still.
                            if let Some(v) = want {
                                // A page made in the other look is made
                                // again first (~110 ms once, only when the
                                // ahead-of-time rebuild hasn't reached it):
                                // never shown light in the dark look or the
                                // other way (run r184s3c).
                                let i = usize::from(v.tab);
                                if pages[i].is_some() && page_light[i] != Some(shell.light()) {
                                    let t0 = rt::ticks();
                                    if let Some((h, _)) = pages[i].take() {
                                        let _ = gfx.free_picture(pc, h);
                                    }
                                    pages[i] = build_page(gfx, pc, shell, &mut cpu, frame, v.tab);
                                    page_light[i] = Some(shell.light());
                                    let _ = writeln!(
                                        RunLog,
                                        "page {i}: made again in the look shown before it was handed over ({} us)",
                                        rt::ticks().wrapping_sub(t0) / 13 * 4
                                    );
                                }
                                if let Some((h, _)) = pages[i] {
                                    commit = Some((Some(page_state(h, v)), false, true));
                                    shown_tab = Some(v.tab);
                                }
                            } else {
                                commit = Some((None, false, true));
                                shown_tab = None;
                            }
                            pages_check = true;
                        } else if let Some(v) = want
                            && retire.is_none()
                            && pages[usize::from(v.tab)].is_some()
                            && page_light[usize::from(v.tab)] != Some(shell.light())
                        {
                            // The look changed under the page on the layer
                            // (Auto at sunrise or sunset, or the dock's
                            // settings arriving: the X51v's run today-0065,
                            // 2026-11-08, the relay's settings turned the
                            // look dark and the chrome went dark 84 ms
                            // before the light page was made again): the
                            // page made again now and swapped in with the
                            // chrome's frame, one commit, the old picture
                            // given back after it (R185's class).
                            let i = usize::from(v.tab);
                            let t0 = rt::ticks();
                            let old = pages[i].take();
                            let old_light = page_light[i];
                            pages[i] = build_page(gfx, pc, shell, &mut cpu, frame, v.tab);
                            page_light[i] = Some(shell.light());
                            if let Some((h, _)) = pages[i] {
                                commit = Some((Some(page_state(h, v)), false, true));
                                retire = old.map(|(h, _)| h);
                                let _ = writeln!(
                                    RunLog,
                                    "page {i}: made again in the look shown under the layer, one commit with the chrome ({} us)",
                                    rt::ticks().wrapping_sub(t0) / 13 * 4
                                );
                            } else {
                                pages[i] = old;
                                page_light[i] = old_light;
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
                            // Up at the next refresh (the app composed ahead),
                            // so Reveal's clock runs on from the tap: a
                            // restart here stepped the window back 8 px (the
                            // trace, 2026-10-31).
                            reveal_shown = true;
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
                        if reveal_on_page {
                            if let Some(v) = shell.reveal_page()
                                && let Some((h, _)) = pages[usize::from(v.tab)]
                            {
                                let _ = gfx.move_layer(pc, page_state(h, v).win);
                            }
                        } else if reveal_shown && let Some((r, shift)) = shell.reveal_plane() {
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
                // (Not while the living sky moves: a page's build holds the
                // CPU ~110 ms, the sky stopping 3-4 steps, the owner's run
                // of 2026-11-01, today-0048. A page out of date is built
                // when the app first hands over to it, above.)
                if pages_check
                    && commit.is_none()
                    && !rest_on_app
                    && !shell.animating()
                    && !filter.down()
                    && !shell.needs_frame()
                    && !gfx.busy()
                    && !gfx.commit_pending()
                    && !gfx.aside_held()
                {
                    // The page shown is replaced only while it is on the
                    // layer and no replaced picture still waits to go back:
                    // otherwise its new picture had nowhere to go and was
                    // lost (today-0036: with the place menu open, each pass
                    // built one and kept none, until the pool ran out).
                    let layer_on = shell.page_layer().is_some();
                    let radar_open = shell.weather_page().0 >= 3
                        && matches!(shell.screen(), rp_shell::Screen::Weather);
                    let stale = (0..3u8).find(|&t| {
                        let i = usize::from(t);
                        let shown_now = Some(t) == shown_tab;
                        let lent = pages_lent & 1 << t != 0 && pages[i].is_none() && !radar_open;
                        (!shown_now || (layer_on && retire.is_none()))
                            && (lent
                                || pages[i].is_some_and(|(_, k)| {
                                    let key = page_key(shell, frame, t);
                                    k != key && no_room[i] != Some(key)
                                }))
                    });
                    match stale {
                        // The page shown: a new picture, swapped in by a
                        // commit, the old one given back after it.
                        Some(t) if Some(t) == shown_tab => {
                            let i = usize::from(t);
                            let old = pages[i].take();
                            let old_light = page_light[i];
                            pages[i] = build_page(gfx, pc, shell, &mut cpu, frame, t);
                            page_light[i] = Some(shell.light());
                            match (pages[i], shell.page_layer()) {
                                (Some((h, _)), Some(v)) => {
                                    commit = Some((Some(page_state(h, v)), false, true));
                                    retire = old.map(|(h, _)| h);
                                }
                                // Not shown after all: the new one goes back.
                                (Some((h, _)), None) => {
                                    let _ = gfx.free_picture(pc, h);
                                    pages[i] = old;
                                    page_light[i] = old_light;
                                }
                                (None, _) => {
                                    no_room[i] = Some(page_key(shell, frame, t));
                                    pages[i] = old;
                                    page_light[i] = old_light;
                                }
                            }
                        }
                        Some(t) => {
                            let i = usize::from(t);
                            let was_lent = pages[i].is_none() && pages_lent & 1 << i != 0;
                            let t0 = rt::ticks();
                            if let Some((h, _)) = pages[i].take() {
                                let _ = gfx.free_picture(pc, h);
                            }
                            pages[i] = build_page(gfx, pc, shell, &mut cpu, frame, t);
                            page_light[i] = Some(shell.light());
                            if pages[i].is_none() {
                                no_room[i] = Some(page_key(shell, frame, t));
                            } else if was_lent {
                                pages_lent &= !(1 << i);
                                let _ = writeln!(
                                    RunLog,
                                    "radar: page {i}, lent to it, made again ahead while still in {} us",
                                    rt::ticks().wrapping_sub(t0) / 13 * 4
                                );
                            }
                        }
                        None => pages_check = false,
                    }
                }
                // The app composed ahead while the Today screen is still (its
                // key the draw list's: the header's clock makes it stale),
                // so a tap opens Reveal's window at the next refresh.
                if app_check
                    && !NO_APP_AHEAD
                    && commit.is_none()
                    && shell.screen() == rp_shell::Screen::Today
                    && !shell.animating()
                    && !filter.down()
                    && (!shell.needs_frame() || shell.only_sky_changed())
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
                                Ok(h) => {
                                    chrome_pic = Some((h, key));
                                    chrome_light = Some(shell.light());
                                }
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

            // The screen on a picture composed ahead stays on it until
            // something on the screen changes; then the renderer's own
            // screen, drawn whole, takes over with that change. Handed back
            // at once after the opening (drawn whole, assumed the same
            // pixels), the panel showed two other pictures in 80 ms: the
            // owner's "flash at the end", DSIG 5435b376 -> 31a77120 ->
            // 50804349 at the hand-back while the draw lists matched
            // command for command (the 2D renderer's own drawing differs
            // from the picture's, R135).
            if screen_on_pic
                && shell.needs_frame()
                && commit.is_none()
                && !gfx.busy()
                && !gfx.commit_pending()
            {
                screen_on_pic = false;
                shell.redraw_all();
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
                        line: gfx.commits().last_line.unwrap_or(0xFFF),
                    };
                    trace[trace_n * 10..trace_n * 10 + 10].copy_from_slice(&t.words());
                    trace_n += 1;
                    // After the opening, its tail counts refreshes sampled
                    // (it counted loop passes, ~1 a ms, so every trace
                    // ended at "Reveal done" and the hand-off was never
                    // seen: the run of 2026-10-31).
                    if !revealing && !gfx.aside_held() {
                        if trace_end.is_none() {
                            trace_end = Some(trace_n - 1);
                        }
                        trace_tail = trace_tail.saturating_sub(1);
                    }
                }
                last_line = line;
                if trace_tail == 0 || trace_n == TRACE {
                    trace_on = false;
                    let _ = writeln!(
                        RunLog,
                        "trace of open {opens}: {trace_n} refreshes (ms, DSIG, GSCADR, VSCADR, VSADR, VSCTRL, SSSIZE; the window meant x,y,w,h,row; @ the last commit's line)"
                    );
                    // Every opening's end in one line: from the opening's
                    // last refresh, how many times the picture changed
                    // (none, if the end is seamless) and the refreshes the
                    // samples missed (the loop busy), so a clean count is
                    // known to be whole (the owner, 2026-11-01: "I asked
                    // you to make the logs be able to see it").
                    if let Some(e) = trace_end.filter(|&e| e >= 1 && e < trace_n) {
                        let at = |i: usize| Refresh::from_words(&trace[i * 10..i * 10 + 10]);
                        let last = at(e - 1);
                        let (mut changes, mut missed, mut prev) = (0u32, 0u32, last);
                        let mut first = 0;
                        for i in e..trace_n {
                            let t = at(i);
                            if t.sig != prev.sig {
                                if changes == 0 {
                                    first = t.ms - last.ms;
                                }
                                changes += 1;
                            }
                            missed += (t.ms - prev.ms).saturating_sub(8) / 17;
                            prev = t;
                        }
                        let _ = writeln!(
                            RunLog,
                            "open {opens} end: the picture changed {changes} times in the {} refreshes sampled after the opening's last ({} ms; the first change at +{first} ms), {missed} refreshes not sampled",
                            trace_n - e,
                            prev.ms - last.ms
                        );
                    }
                    // The first opening's refreshes only: every opening's
                    // filled the 64 KiB log by the seventh (the option C
                    // run, 2026-10-31, lost its end).
                    for w in trace[..if opens <= 1 { trace_n * 10 } else { 0 }].chunks(10) {
                        let t = Refresh::from_words(w);
                        let _ = writeln!(
                            RunLog,
                            "T {} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} | {},{},{},{},{} @{}",
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
                            t.meant.4,
                            t.line
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
            if (commit.is_some() || shell.needs_frame() && !revealing && !rest_on_app)
                && !gfx.busy()
                && !gfx.aside_held()
            {
                let t0 = rt::ticks();
                let draw = commit.is_none_or(|(_, _, d)| d);
                // A sky step alone leaves the app composed ahead as it was.
                let sky_only = shell.only_sky_changed();
                if draw {
                    // The chart's line painted before the frame that shows
                    // it (painted only when still, a chart opened over the
                    // last one's line, then swapped: the owner's video,
                    // 2026-11-01, "a fully different line graph").
                    if rp_today::keep_graph(shell, pics, &mut graph_key) {
                        pages_check = true;
                        app_check = true;
                    }
                    rp_today::keep_chart(shell, pics, &mut chart_key);
                    z_shell.run(|| shell.frame(frame));
                    // Commands past the frame's room are dropped: never
                    // quietly (the tests walk every screen with the device's
                    // pictures; this names one they missed).
                    if frame.overflowed() && !overflow_logged {
                        overflow_logged = true;
                        let _ = writeln!(
                            RunLog,
                            "frame {}: full ({} commands), the rest dropped on {:?}",
                            frames + 1,
                            rp_draw::MAX_CMDS,
                            shell.screen()
                        );
                    }
                } else {
                    frame.clear();
                }
                if shell.screen() == rp_shell::Screen::Today && !sky_only {
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
                if us > draw_us_worst {
                    draw_worst_at = (
                        now_ms,
                        shell.screen(),
                        shell.weather_page().0,
                        shell.animating(),
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
                if us > SLOW_FRAME_US
                    && frames > 0
                    && (!REPLAY || LOOK_CHECK)
                    && (!moving || LOOK_CHECK)
                {
                    let _ = writeln!(
                        RunLog,
                        "slow frame {} at {} ms{}: {us} us (queueing {} us, wait {} us, background {} ({} px), rows written {} reused {}, {} rects, {} px dithered, {} command words, {} commands drawn; {} glyph atlases begun ({} evicted), {} glyphs and {} pictures uploaded)",
                        frames + 1,
                        now_ms,
                        if moving { " (moving)" } else { "" },
                        s.queue_us,
                        s.wait_us,
                        s.background,
                        s.background_px,
                        s.rows_written,
                        s.rows_reused,
                        s.rects,
                        s.px_dither,
                        s.words,
                        frame.cmds().count(),
                        s.atlases,
                        s.atlases_evicted,
                        s.glyphs,
                        s.images
                    );
                }
                // Pictures drawn as nothing, by either renderer (R181).
                let cpu_new = cpu.missed.n.wrapping_sub(cpu_missed_seen);
                cpu_missed_seen = cpu.missed.n;
                for (n, last, by) in [
                    (s.pics_missed.n, s.pics_missed.last, "the 2D engine"),
                    (cpu_new, cpu.missed.last, "the CPU"),
                ] {
                    pics_missed = pics_missed.saturating_add(n);
                    if n > 0 && last != missed_said && missed_lines < 24 {
                        missed_said = last;
                        missed_lines += 1;
                        if let Some((pic, why)) = last {
                            let _ = writeln!(
                                RunLog,
                                "pictures not drawn by {by}: {n} in frame {} at {now_ms} ms, the last picture {:#x} ({} x {}): {why:?}; {pics_missed} so far",
                                frames + 1,
                                pic.key(),
                                pic.w,
                                pic.h
                            );
                        }
                    }
                }
                queue_worst = queue_worst.max(s.queue_us);
                wait_worst = wait_worst.max(s.wait_us);
                rebuilds += u32::from(s.background);
                frames += 1;
                if frames == 1 {
                    let _ = writeln!(RunLog, "{} first frame {us} us", rp_today::MARKER);
                    let _ = writeln!(
                        RunLog,
                        "start: the Today screen shown at {} ms after the image began",
                        rt::ticks().wrapping_sub(t_image) / TICKS_MS
                    );
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

            // An answer with nothing left to draw on screen 0 (the layer's
            // alone, or nothing changed) isn't timed: kept, the next frame
            // whenever it came was charged with the wait (the run of
            // 2026-11-01: a "slow answer" of 9.9 s with no touch between).
            if pending.is_some() && !shell.needs_frame() && !gfx.busy() && !gfx.commit_pending() {
                pending = None;
            }

            // A movement over (no frame for a while): its one summary line.
            if let Some((first, last)) = burst
                && now_ms - last > BURST_GAP_MS
                && showing.is_none()
                && !REPLAY
            {
                burst = None;
                let (n, cpu_med, cpu_worst) = burst_cpu.summary();
                let (_, per_med, _) = burst_period.summary();
                // Only the movements worth reading: slower than 30 fps over
                // five frames or more. A line each filled the 64 KiB run log
                // (138 lines, 31 KB, the owner's run of 2026-10-31); the
                // rest are counted, one line at the end.
                let slow = n >= 5 && per_med > 33_333;
                if n > 1 && !slow {
                    movements_quiet += 1;
                }
                if n > 1 && slow {
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
            let net_busy = net.as_ref().is_some_and(|n| n.waiting);
            let wait_ms: u32 = if showing.is_some()
                || gfx.busy()
                || gfx.commit_pending()
                || net_busy
                || radar_job.is_some()
            {
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
            } else if let Some(due) = shell.sky_due_in(now_ms as u32) {
                // The living sky's next step, when it is due.
                due
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
                    rt::sleep_until(rt::ticks().wrapping_add(ticks.max(1)), || false);
                    rt::check_in();
                }
                continue;
            }
            // Armed and slept on as one step (an alarm due at once fired
            // between the two and the loop slept until a key press: the
            // owner's run, 2026-10-31, 24 s frozen).
            rt::sleep_until(rt::ticks().wrapping_add(wait_ticks), || false);
        }
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}
