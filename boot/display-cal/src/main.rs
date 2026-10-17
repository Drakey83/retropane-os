//! RetroPane display calibration, step C1 (docs/design/display-calibration.md):
//! the 2700G's gamma table judged by eye on the real panel.
//!
//! The picture ([`rp_display_cal::pixel`]) is drawn once into a 32 bpp
//! plane; the controls change only the gamma table, so every curve is seen
//! over the same pixels:
//!
//! | Input | Does |
//! |---|---|
//! | Tap, or D-pad right / left | next / previous curve |
//! | D-pad up / down | the curve's strength (0-10) |
//! | D-pad centre | A/B: flips between the curve and Stock |
//! | Home | done: everything put back, back to the loader |
//!
//! Every change goes to the run log, and the last choice with its table.
//! After 10 minutes with no input it puts everything back and returns.
//! Display registers and the gamma table are restored as found.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed when the image puts everything back.
pub const MARKER: &str = "RETROPANE-DISPLAY-CAL-OK";

#[cfg(target_os = "none")]
mod image {
    use core::cell::RefCell;
    use core::fmt::Write;

    use rp_display_cal::{CAPTION_H, Curve, H, STRENGTH_MAX, STRENGTH_START, W, pixel, table};
    use rp_drv_mbx_display::layers::{Layer, Snapshot};
    use rp_drv_mbx_display::scanout::GAMMA_ENTRIES;
    use rp_drv_mbx_display::{Display, PixelFormat, Plane, crash, font};
    use rp_drv_pxa_gpio::{Gpio, GpioPin};
    use rp_drv_pxa_keypad::{Button, Keypad, Pressed};
    use rp_drv_pxa_ssp::Ssp;
    use rp_drv_tsc2046::Tsc2046;
    use rp_hal_api::RegisterBus;
    use rp_hal_pxa27x::rt;
    use rp_hal_x51v::MBX_VRAM_SIZE;
    use rp_hal_x51v::devices::{self, RunLog};
    use rp_vram_map::SCREENS;

    rp_hal_pxa27x::entry!(main);

    const GFXGAM0: usize = 0x2250;
    const GSCTRL: usize = 0x2000;
    const GSCTRL_GAMMA: u32 = 1 << 26;
    /// Touch screen chip select and pen interrupt (docs/hw/touch.md).
    const TS_CS: u8 = 24;
    const TS_PENIRQ: u8 = 94;
    /// No input for this long: put back and return.
    const IDLE_S: u32 = 600;
    /// A tap counts once the pen has been up at least this long.
    const LIFT_MS: u32 = 150;
    const STRIDE: usize = W as usize * 4;

    fn on_fault(kind: rt::Fault, addr: u32) -> ! {
        crash_out(format_args!("FAULT {} at {addr:#010x}", kind.name()))
    }

    #[inline(never)]
    fn crash_out(msg: core::fmt::Arguments) -> ! {
        if let Some((regs, vram)) = devices::emergency_mbx() {
            let mut c =
                crash::emergency_console(regs, vram, (0, 0), (480, 64), 2, (0xFFFF, 0x0000));
            let _ = write!(c, "{msg}");
        }
        devices::crash_to_loader(msg);
        halt()
    }

    /// Writes one pixel (XRGB8888) of the 32 bpp screen.
    fn put(fb: &mut [u8], x: u32, y: u32, (r, g, b): (u8, u8, u8)) {
        let o = y as usize * STRIDE + x as usize * 4;
        fb[o..o + 4].copy_from_slice(&[b, g, r, 0xFF]);
    }

    /// Text at `(x, y)`, `scale` times the font's size, on black.
    fn text(fb: &mut [u8], x: u32, y: u32, scale: u32, colour: (u8, u8, u8), s: &str) {
        let cell = (font::WIDTH as u32 + 1) * scale;
        for (i, c) in s.chars().enumerate() {
            let px = x + i as u32 * cell;
            if px + cell > W {
                return;
            }
            let g = font::glyph(c);
            for gy in 0..(font::HEIGHT as u32 + 1) * scale {
                for gx in 0..cell {
                    let (fx, fy) = (gx / scale, gy / scale);
                    let lit = fx < font::WIDTH as u32
                        && (fy as usize) < font::HEIGHT
                        && g[fy as usize] & (1 << (font::WIDTH as u32 - 1 - fx)) != 0;
                    put(fb, px + gx, y + gy, if lit { colour } else { (0, 0, 0) });
                }
            }
        }
    }

    /// A short string on the stack.
    struct Line {
        buf: [u8; 48],
        len: usize,
    }

    impl Write for Line {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let n = s.len().min(self.buf.len() - self.len);
            self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
            self.len += n;
            Ok(())
        }
    }

    impl Line {
        fn new() -> Self {
            Self {
                buf: [0; 48],
                len: 0,
            }
        }
        fn as_str(&self) -> &str {
            core::str::from_utf8(&self.buf[..self.len]).unwrap_or("?")
        }
    }

    /// The caption: the curve, its strength, and whether A/B shows Stock.
    fn caption(fb: &mut [u8], curve: Curve, strength: u32, showing_stock: bool) {
        for y in 0..CAPTION_H {
            for x in 0..W {
                put(fb, x, y, (0, 0, 0));
            }
        }
        let mut l = Line::new();
        let _ = write!(l, "{} ", curve.name());
        if curve != Curve::Stock {
            let _ = write!(l, "{strength}/{STRENGTH_MAX}");
        }
        if showing_stock {
            let _ = write!(l, "  [B: Stock]");
        }
        text(fb, 6, 4, 3, (255, 255, 255), l.as_str());
        text(
            fb,
            6,
            32,
            2,
            (150, 150, 150),
            "Tap/<> curve  ^v strength  OK A/B",
        );
    }

    fn write_gamma<B: RegisterBus>(regs: &mut B, t: &[u32; GAMMA_ENTRIES]) {
        for (i, w) in t.iter().enumerate() {
            regs.write32(GFXGAM0 + 4 * i, *w);
        }
    }

    fn log_table(what: &str, t: &[u32; GAMMA_ENTRIES]) {
        let _ = write!(RunLog, "{what}:");
        for w in t {
            let _ = write!(RunLog, " {w:06X}");
        }
        let _ = writeln!(RunLog);
    }

    fn main() -> ! {
        rt::set_fault_handler(on_fault);
        rt::init_mmu();
        let Some((regs, vram)) = devices::take_mbx() else {
            halt()
        };
        let mut display = Display::new(regs);
        let _ = display.adopt(MBX_VRAM_SIZE as usize);
        let before = Snapshot::take(display.regs());
        let gsctrl = display.regs().read32(GSCTRL);
        let found: [u32; GAMMA_ENTRIES] =
            core::array::from_fn(|i| display.regs().read32(GFXGAM0 + 4 * i));
        let _ = writeln!(RunLog, "GSCTRL {gsctrl:08X}");
        log_table("gamma found", &found);

        // The picture, once, into screen 0 (rp-vram-map), then shown.
        let at = SCREENS[0].at as usize;
        let fb = &mut vram[at..at + STRIDE * H as usize];
        let t0 = rt::ticks();
        for y in CAPTION_H..H {
            for x in 0..W {
                put(fb, x, y, pixel(x, y));
            }
            rt::check_in();
        }
        let (mut curve, mut strength, mut ab) = (Curve::Stock, STRENGTH_START, false);
        caption(fb, curve, strength, ab);
        let _ = writeln!(
            RunLog,
            "picture drawn in {} ms",
            rt::ticks().wrapping_sub(t0) / (rt::TICK_HZ / 1000)
        );
        let plane = Plane {
            format: PixelFormat::Argb8888,
            width: W as u16,
            height: H as u16,
            offset: at,
            stride: STRIDE,
        };
        display.set_graphics(&plane, Layer::SOLO);
        if gsctrl & GSCTRL_GAMMA == 0 {
            let _ = writeln!(RunLog, "gamma: off, switched on");
            let on = display.regs().read32(GSCTRL) | GSCTRL_GAMMA;
            display.regs().write32(GSCTRL, on);
        }

        // Input: buttons polled; the touch screen's pen sampled for taps.
        let (Some(kp), Some(gpio), Some(ssp)) =
            (rt::take_keypad(), rt::take_gpio(), rt::take_ssp1())
        else {
            crash_out(format_args!("no keypad, GPIO or SSP"))
        };
        let mut keypad = Keypad::new(kp);
        keypad.enable();
        keypad.acknowledge();
        let gpio = RefCell::new(Gpio::new(gpio));
        // Windows Mobile leaves GPIO24 as SSP1's frame signal: level and
        // direction first, then a plain GPIO (as rp-today).
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
        let _ = writeln!(RunLog, "ready: {} {strength}", curve.name());

        let mut held = Pressed::NONE;
        let mut pen_up_ms = LIFT_MS;
        let mut last_input = rt::ticks();
        let mut changes = 0u32;
        let ms = rt::TICK_HZ / 1000;
        let reason = loop {
            rt::check_in();
            rt::delay_us(20_000);
            if rt::ticks().wrapping_sub(last_input) > IDLE_S * rt::TICK_HZ {
                break "no input for 10 minutes";
            }
            // What changed this round: a curve or strength step, A/B, done.
            let (mut next, mut back, mut up, mut down, mut flip, mut done) =
                (false, false, false, false, false, false);
            keypad.acknowledge();
            let now = keypad.pressed();
            for (b, pressed) in held.changes(now) {
                if !pressed {
                    continue;
                }
                match b {
                    Button::Right => next = true,
                    Button::Left => back = true,
                    Button::Up => up = true,
                    Button::Down => down = true,
                    Button::Enter => flip = true,
                    Button::Home => done = true,
                    _ => {}
                }
            }
            held = now;
            match touch.sample() {
                Some(_) => {
                    if pen_up_ms >= LIFT_MS {
                        next = true;
                    }
                    pen_up_ms = 0;
                }
                None => pen_up_ms = pen_up_ms.saturating_add(20),
            }
            if done {
                break "Home";
            }
            if !(next || back || up || down || flip) {
                continue;
            }
            last_input = rt::ticks();
            changes += 1;
            if next || back {
                curve = curve.step(back);
                strength = STRENGTH_START;
                ab = false;
            }
            if up {
                strength = (strength + 1).min(STRENGTH_MAX);
            }
            if down {
                strength = strength.saturating_sub(1);
            }
            if flip {
                ab = !ab;
            }
            let t = if ab {
                found
            } else {
                table(curve, strength, &found)
            };
            write_gamma(display.regs(), &t);
            caption(fb, curve, strength, ab);
            let _ = writeln!(
                RunLog,
                "at {} ms: {} {strength}{}",
                last_input / ms,
                curve.name(),
                if ab { " (B: Stock shown)" } else { "" }
            );
        };

        // The last choice, then everything back as found.
        let _ = writeln!(
            RunLog,
            "done ({reason}) after {changes} changes; last: {} {strength}{}",
            curve.name(),
            if ab { " (B: Stock shown)" } else { "" }
        );
        log_table("last curve's table", &table(curve, strength, &found));
        write_gamma(display.regs(), &found);
        // The planes as found (GSCTRL's gamma bit with them).
        before.restore(display.regs());
        let _ = writeln!(RunLog, "{}", super::MARKER);
        if devices::loader_present() {
            devices::return_to_loader();
        }
        halt()
    }

    fn halt() -> ! {
        loop {
            core::hint::spin_loop();
        }
    }

    #[panic_handler]
    fn panic(info: &core::panic::PanicInfo) -> ! {
        match info.location() {
            Some(l) => crash_out(format_args!("PANIC {}:{}", l.file(), l.line())),
            None => crash_out(format_args!("PANIC")),
        }
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}
