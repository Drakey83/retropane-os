//! Test image (ADR-0021): the device side of `rp-today` (see the library's
//! documentation): input, the tickless loop, the latency log.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

#[cfg(target_os = "none")]
mod image {
    use core::cell::RefCell;
    use core::fmt::Write;

    use rp_draw::Frame;
    use rp_drv_mbx_display::Display;
    use rp_drv_mbx_display::layers::Snapshot;
    use rp_drv_pxa_gpio::{Gpio, GpioPin};
    use rp_drv_pxa_keypad::Keypad;
    use rp_drv_pxa_keypad::keys::{Keys, Kind, Timing};
    use rp_drv_pxa_ssp::Ssp;
    use rp_drv_pxa_uart::PxaUart;
    use rp_drv_tsc2046::Tsc2046;
    use rp_graphics::{Graphics, Step};
    use rp_hal_mem::{load32_volatile, store32_volatile};
    use rp_hal_pxa27x::soc::irq;
    use rp_hal_pxa27x::{Mmio, cpu, rt};
    use rp_hal_x51v::MBX_VRAM_SIZE;
    use rp_hal_x51v::devices::RunLog;
    use rp_mbx_render::{Layout, LocalMem, Mbx2dRenderer};
    use rp_mbx3d_render::Clock;
    use rp_shell::{Action, Input, Key, Shell, Time};
    use rp_touch_cal::{Calibration, Filter, Pen, Sample};
    use rp_vram_map::{POOL, Pool};

    use rp_today::{Depth, H, Latency, W, WallScratch, clock};

    rp_hal_pxa27x::entry!(main);

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
    /// queue to flip.
    static LATENCY: rt::Slot<[Latency; 4]> = rt::Slot::new();
    /// The wallpaper's conversion rows (the 16-bit comparison's dither).
    static WALL: rt::Slot<WallScratch> = rt::Slot::new();
    /// The wallpaper this image was built with (`RP_TODAY_WALLPAPER`).
    const WALL_NAME: &str = env!("RP_TODAY_WALLPAPER_NAME");

    /// Touch screen chip select and pen interrupt (docs/hw/touch.md).
    const TS_CS: u8 = 24;
    const TS_PENIRQ: u8 = 94;
    /// OS timer ticks per millisecond.
    const TICKS_MS: u32 = 3250;
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

    static WALLPAPER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/wallpaper.yuv"));

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
        rt::set_fault_handler(on_fault);
        rt::init_mmu();
        let Some((regs, vram)) = rp_hal_x51v::devices::take_mbx() else {
            crash(format_args!("no display"))
        };
        let mut d = Display::new(regs);
        let _ = d.adopt(MBX_VRAM_SIZE as usize);
        let before = Snapshot::take(d.regs());

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
        let t0 = rt::ticks();
        rp_today::show_gpu(two_d, WALLPAPER, rt::delay_us, wall);
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
        let (Some(shell), Some(frame)) = (
            SHELL.put_with(|| Shell::new(rp_today::shell_fonts(), rp_today::icons(), true)),
            FRAME.put_with(Frame::new),
        ) else {
            crash(format_args!("started twice"))
        };

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
        let Some([latency, settle, queueing, to_flip]) = LATENCY.put_with(|| {
            [
                Latency::new(),
                Latency::new(),
                Latency::new(),
                Latency::new(),
            ]
        }) else {
            crash(format_args!("started twice"))
        };
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
        // The GPU renderer's costs (rp_mbx_render::FrameStats), worst seen.
        let (mut queue_worst, mut wait_worst, mut engine_worst, mut rebuilds) =
            (0u32, 0u32, 0u32, 0u32);

        loop {
            rt::check_in();
            let t = rt::ticks();
            sub_ms += t.wrapping_sub(last_ticks);
            last_ticks = t;
            now_ms += u64::from(sub_ms / TICKS_MS);
            sub_ms %= TICKS_MS;

            // The clock: once a minute.
            let minute = start_min + (now_ms / 60_000) as i64;
            if minute != minute_shown {
                minute_shown = minute;
                let time: Time = clock::time_at(minute);
                shell.set_time(time);
            }
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
            let pen_event = pen_irq.take();
            let read_at = rt::ticks();
            let traced = if pen_event || filter.down() || touch.pen_down() {
                touch.sample_traced()
            } else {
                None
            };
            let raw = traced.map(|(s, _)| s);
            if raw.is_some() && !filter.down() {
                landed_ticks = Some(read_at);
            }
            irq_seen |= pen_event;
            // Whether a touch was in progress before this sample: its end
            // is recorded even when nothing was reported.
            let was_down = filter.down();
            let ev = filter.sample(
                raw.map(|s| Sample {
                    x: s.x,
                    y: s.y,
                    z1: s.z1,
                }),
                (now_ms & 0xFFFF_FFFF) as u32,
            );
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
            if (raw.is_some() || ev != Pen::Nothing || was_down) && recorded < RECORD {
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
                    _ => Action::Nothing,
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
                Action::WallpaperDepth { sixteen } => {
                    // The frame in flight may be reading the wallpaper.
                    while matches!(gfx.poll(&mut gclock), Step::Wait(_)) {}
                    let t0 = rt::ticks();
                    let depth = if sixteen { Depth::Rgb565 } else { Depth::Full };
                    rp_today::load_wallpaper(gfx.two_d(), WALLPAPER, depth, wall);
                    gfx.two_d().wallpaper_changed();
                    let _ = writeln!(
                        RunLog,
                        "compare: {WALL_NAME} at {}, {}: converted in {} us",
                        if sixteen { "16-bit" } else { "32-bit" },
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
                    "gpu frames: worst CPU queueing {queue_worst} us, worst wait for the previous frame and scan-out {wait_worst} us, worst queue-to-flip {engine_worst} us, background rebuilt {rebuilds} times"
                );
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
                before.restore(&mut gfx.two_d().parts().0.regs);
                rp_hal_x51v::devices::return_to_loader();
                loop {
                    core::hint::spin_loop();
                }
            }

            // A frame the engine finished goes on the screen; the latency
            // counts to then (the flip), not to the end of queueing.
            if !matches!(gfx.poll(&mut gclock), Step::Wait(_))
                && let Some(at) = showing.take()
            {
                let total = rt::ticks().wrapping_sub(at) / 13 * 4;
                latency.add(total);
                let s = gfx.two_d().last;
                // A slow answer, taken apart (2026-10-14: one of 57 took
                // 101 ms on the device, every part's worst far less).
                if total > SLOW_ANSWER_US {
                    let _ = writeln!(
                        RunLog,
                        "slow answer at {} ms: {total} us; its frame's CPU {last_frame_us} us, wait {} us, engine {} us",
                        now_ms, s.wait_us, s.engine_us
                    );
                }
                engine_worst = engine_worst.max(s.engine_us);
                to_flip.add(s.engine_us);
            }

            // Draw what changed, timing it.
            if shell.needs_frame() {
                let t0 = rt::ticks();
                shell.frame(frame);
                // One job at a time: the frame in flight finishes first (as
                // the 2D renderer's own draw waited for it).
                while matches!(gfx.poll(&mut gclock), Step::Wait(_)) {}
                let _ = gfx.frame(frame, &mut gclock);
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
                if us > SLOW_FRAME_US && frames > 0 {
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
            let wait_ms: u32 = if showing.is_some() {
                // A frame is with the engine: flip it as soon as it's done.
                1
            } else if filter.down() || touch.pen_down() {
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
            if rt::set_alarm(rt::ticks().wrapping_add(wait_ms * TICKS_MS)) {
                rt::wait_any_signal();
            }
        }
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
