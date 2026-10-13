//! Test image (ADR-0021): the device side of `rp-today` (see the library's
//! documentation): input, the tickless loop, the latency log.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

#[cfg(target_os = "none")]
mod image {
    use core::cell::RefCell;
    use core::fmt::Write;

    use rp_draw::{Frame, Renderer};
    use rp_drv_mbx_display::Display;
    use rp_drv_mbx_display::layers::Snapshot;
    use rp_drv_pxa_gpio::{Gpio, GpioPin};
    use rp_drv_pxa_keypad::Keypad;
    use rp_drv_pxa_keypad::keys::{Keys, Kind, Timing};
    use rp_drv_pxa_ssp::Ssp;
    use rp_drv_pxa_uart::PxaUart;
    use rp_drv_tsc2046::{Calibration, Tsc2046};
    use rp_gfx::layer::{Layer, PlaneFormat};
    use rp_gfx::render::CpuRenderer;
    use rp_hal_pxa27x::rt;
    use rp_hal_pxa27x::soc::irq;
    use rp_hal_x51v::MBX_VRAM_SIZE;
    use rp_hal_x51v::devices::RunLog;
    use rp_shell::{Action, Input, Key, Shell, Time};

    use rp_today::{H, Latency, W, clock};

    rp_hal_pxa27x::entry!(main);

    /// Touch screen chip select and pen interrupt (docs/hw/touch.md).
    const TS_CS: u8 = 24;
    const TS_PENIRQ: u8 = 94;
    /// OS timer ticks per millisecond.
    const TICKS_MS: u32 = 3250;

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

        let gfx = rp_today::show(&mut d, vram, WALLPAPER);

        let Some(work) = rp_hal_x51v::devices::take_scratch() else {
            crash(format_args!("no scratch RAM"))
        };
        let Some(layer) = Layer::in_bytes(work, W as u32, H as u32) else {
            crash(format_args!("scratch too small for the layer"))
        };
        let plane = &mut vram[gfx.offset..gfx.offset + gfx.stride * H];
        let mut renderer =
            CpuRenderer::new(layer, plane, gfx.stride, PlaneFormat::Argb4444Dithered);
        let mut shell = Shell::new(rp_today::shell_fonts(), true);
        let mut frame = Frame::new();

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
        let mut raw_logged = 0u8;

        // Time: the OS timer (3.25 MHz, wraps every 22 minutes) summed into
        // milliseconds since start.
        let mut last_ticks = rt::ticks();
        let mut sub_ms = 0u32;
        let mut now_ms = 0u64;
        let start_min = start_minutes();
        let mut minute_shown = -1i64;

        let mut latency = Latency::new();
        let mut frames = 0u32;
        let mut draw_us_worst = 0u32;
        let mut pen: Option<(u16, u16)> = None;
        let mut pending: Option<u32> = None; // when the input being answered was read

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
            let key_event = keys_irq.take();
            if key_event {
                keypad.acknowledge();
            }
            let pressed = keypad.pressed();
            let mut back = false;
            key_events.update(rt::ticks() / TICKS_MS, pressed, &mut |e| {
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
            let traced = if pen_event || pen.is_some() || touch.pen_down() {
                touch.sample_traced()
            } else {
                None
            };
            let raw = traced.map(|(s, _)| s);
            let now = raw.map(|s| Calibration::X51V.to_screen(s));
            // The first touches' raw readings, for the calibration (never
            // checked on the device before this image).
            if let (None, Some((s, t)), Some((x, y))) = (pen, traced, now)
                && raw_logged < 12
            {
                raw_logged += 1;
                let _ = writeln!(
                    RunLog,
                    "touch {raw_logged}: x {} ({} {} {} {}) y {} ({} {} {} {}) z1 {} z2 {} -> screen {} {}",
                    s.x,
                    t.x[0],
                    t.x[1],
                    t.x[2],
                    t.x[3],
                    s.y,
                    t.y[0],
                    t.y[1],
                    t.y[2],
                    t.y[3],
                    s.z1,
                    s.z2,
                    x,
                    y
                );
            }
            match (pen, now) {
                (None, Some((x, y))) => {
                    if shell.input(Input::PenDown { x, y }) != Action::Nothing {
                        pending.get_or_insert(read_at);
                    }
                }
                (Some((x, y)), None) => match shell.input(Input::PenUp { x, y }) {
                    Action::BackToLoader => back = true,
                    Action::Nothing => {}
                    Action::Redraw => {
                        pending.get_or_insert(read_at);
                    }
                },
                _ => {}
            }
            if now.is_some() {
                pen = now;
            } else if pen.is_some() {
                pen = None;
            }

            if back {
                let (n, median, worst) = latency.summary();
                let _ = writeln!(
                    RunLog,
                    "touch to flush: {n} answered, median {median} us, worst {worst} us \
                     (promise: 50 ms); {frames} frames, worst draw {draw_us_worst} us"
                );
                before.restore(d.regs());
                rp_hal_x51v::devices::return_to_loader();
                loop {
                    core::hint::spin_loop();
                }
            }

            // Draw what changed, timing it.
            if shell.needs_frame() {
                let t0 = rt::ticks();
                shell.frame(&mut frame);
                renderer.draw(&frame);
                let t1 = rt::ticks();
                let us = t1.wrapping_sub(t0) / 13 * 4;
                draw_us_worst = draw_us_worst.max(us);
                frames += 1;
                if frames == 1 {
                    let _ = writeln!(RunLog, "{} first frame {us} us", rp_today::MARKER);
                    // The emulator check waits for this on the serial port.
                    let _ = writeln!(PxaUart::new(rt::emergency_uart()), "{}", rp_today::MARKER);
                }
                if let Some(at) = pending.take() {
                    latency.add(t1.wrapping_sub(at) / 13 * 4);
                }
            }

            // Sleep until the next reason to wake.
            let wait_ms: u32 = if pen.is_some() || touch.pen_down() {
                5
            } else if pressed != rp_drv_pxa_keypad::Pressed::NONE {
                20
            } else if shell.notice().is_some() {
                100
            } else {
                // The next minute.
                (60_000 - (now_ms % 60_000) as u32).max(1)
            };
            if pen.is_none() {
                // Re-arm pen-down, then re-check the level: an edge while it
                // was disarmed isn't remembered.
                pen_irq.done();
            }
            if pen.is_none() && touch.pen_down() {
                continue;
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
