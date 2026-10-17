//! RetroPane display calibration (docs/design/display-calibration.md): the
//! 2700G's gamma table and the backlight dialled in by eye on the real
//! panel.
//!
//! The picture ([`rp_display_cal::pixel`]) is drawn once into a 32 bpp
//! plane; the dials change only the gamma table (and PWM0 for Bright), so
//! every setting is seen over the same pixels. Five dials, each 1-50
//! ([`rp_display_cal::Dials`]): Black, White, Mid, Warm, Bright.
//!
//! | Input | Does |
//! |---|---|
//! | D-pad up / down | picks the dial (yellow in the caption) |
//! | D-pad right / left | moves it by 1 (held: repeats) |
//! | D-pad centre | compare: flips between the dials and the original |
//! | Home | done: everything put back, back to the loader |
//!
//! The run log gets the dials once they settle (1 s after a change), and
//! the last setting with its table. After 10 minutes with no input it puts
//! everything back and returns. Display registers, the gamma table and the
//! backlight are restored as found.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed when the image puts everything back.
pub const MARKER: &str = "RETROPANE-DISPLAY-CAL-OK";

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_display_cal::{CAPTION_H, DIAL_MAX, DIAL_NAMES, DIALS, Dials, H, W, pixel, table};
    use rp_drv_mbx_display::layers::{Layer, Snapshot};
    use rp_drv_mbx_display::scanout::GAMMA_ENTRIES;
    use rp_drv_mbx_display::{Display, PixelFormat, Plane, crash, font};
    use rp_drv_pxa_keypad::keys::{Keys, Kind, Timing};
    use rp_drv_pxa_keypad::{Button, Keypad};
    use rp_hal_api::RegisterBus;
    use rp_hal_pxa27x::rt;
    use rp_hal_x51v::MBX_VRAM_SIZE;
    use rp_hal_x51v::devices::{self, RunLog};
    use rp_vram_map::SCREENS;

    rp_hal_pxa27x::entry!(main);

    const GFXGAM0: usize = 0x2250;
    const GSCTRL: usize = 0x2000;
    const GSCTRL_GAMMA: u32 = 1 << 26;
    /// PWM registers (rp-drv-pxa-pwm's `reg`).
    const CR: usize = 0x0;
    const DCR: usize = 0x4;
    const PCR: usize = 0x8;
    /// RetroPane's backlight cycle: prescale 0, 432 counts (30 kHz).
    const COUNTS: u32 = 432;
    /// No input for this long: put back and return.
    const IDLE_S: u32 = 600;
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

    /// Text at `(x, y)`, `scale` times the font's size, on black; returns
    /// where the next character would go.
    fn text(fb: &mut [u8], x: u32, y: u32, scale: u32, colour: (u8, u8, u8), s: &str) -> u32 {
        let cell = (font::WIDTH as u32 + 1) * scale;
        let mut px = x;
        for c in s.chars() {
            if px + cell > W {
                break;
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
            px += cell;
        }
        px
    }

    /// A short string on the stack.
    struct Line {
        buf: [u8; 16],
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
                buf: [0; 16],
                len: 0,
            }
        }
        fn as_str(&self) -> &str {
            core::str::from_utf8(&self.buf[..self.len]).unwrap_or("?")
        }
    }

    /// The caption: all five dials on two lines, the chosen one in yellow;
    /// red with "ORIG" while compare shows the original.
    fn caption(fb: &mut [u8], d: Dials, chosen: usize, original: bool) {
        for y in 0..CAPTION_H {
            for x in 0..W {
                put(fb, x, y, (0, 0, 0));
            }
        }
        let (mut x, mut y) = (6, 2);
        for (i, name) in DIAL_NAMES.iter().enumerate() {
            if i == 3 {
                (x, y) = (6, 27);
            }
            let colour = if original {
                (255, 60, 60)
            } else if i == chosen {
                (255, 220, 0)
            } else {
                (255, 255, 255)
            };
            let mut l = Line::new();
            let _ = write!(l, "{name} {}  ", d.get(i));
            x = text(fb, x, y, 3, colour, l.as_str());
        }
        if original {
            text(fb, x, y, 3, (255, 60, 60), "ORIG");
        }
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

    fn log_dials(what: core::fmt::Arguments, d: Dials) {
        let _ = write!(RunLog, "{what}:");
        for (i, name) in DIAL_NAMES.iter().enumerate() {
            let _ = write!(RunLog, " {name} {}", d.get(i));
        }
        let _ = writeln!(RunLog);
    }

    fn main() -> ! {
        rt::set_fault_handler(on_fault);
        rt::init_mmu();
        let (Some((regs, vram)), Some(mut pwm)) = (devices::take_mbx(), rt::take_pwm0()) else {
            halt()
        };
        let mut display = Display::new(regs);
        let _ = display.adopt(MBX_VRAM_SIZE as usize);
        let before = Snapshot::take(display.regs());
        let gsctrl = display.regs().read32(GSCTRL);
        let found: [u32; GAMMA_ENTRIES] =
            core::array::from_fn(|i| display.regs().read32(GFXGAM0 + 4 * i));
        let light = (pwm.read32(CR), pwm.read32(DCR), pwm.read32(PCR));
        let _ = writeln!(
            RunLog,
            "GSCTRL {gsctrl:08X}; PWM0 found: CR {:02X} DCR {:03X} PCR {:03X}",
            light.0, light.1, light.2
        );
        log_table("gamma found", &found);
        // Bright starts at the light as found (in 50ths), and the light is
        // left alone until that dial moves.
        let duty = (light.1 & 0x3FF).min(light.2 + 1);
        let bright = (duty * DIAL_MAX + (light.2 + 1) / 2) / (light.2 + 1);
        let mut d = Dials {
            bright: bright.clamp(1, DIAL_MAX),
            ..Dials::STOCK
        };
        let mut bright_moved = false;

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
        let (mut chosen, mut original) = (0usize, false);
        caption(fb, d, chosen, original);
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

        let Some(kp) = rt::take_keypad() else {
            crash_out(format_args!("no keypad"))
        };
        let mut keypad = Keypad::new(kp);
        keypad.enable();
        keypad.acknowledge();
        let mut keys = Keys::new(Timing::DEFAULT);
        let ms = rt::TICK_HZ / 1000;
        log_dials(format_args!("ready"), d);

        let mut last_input = rt::ticks();
        // A change not yet in the log (logged once the dials settle).
        let mut unlogged = false;
        let mut changes = 0u32;
        let reason = loop {
            rt::check_in();
            rt::delay_us(10_000);
            let now = rt::ticks();
            if now.wrapping_sub(last_input) > IDLE_S * rt::TICK_HZ {
                break "no input for 10 minutes";
            }
            if unlogged && now.wrapping_sub(last_input) > rt::TICK_HZ {
                unlogged = false;
                let what = if original { " (original shown)" } else { "" };
                log_dials(format_args!("at {} ms{what}", last_input / ms), d);
            }
            // This round's input: a dial step, a dial pick, compare, done.
            let (mut by, mut pick, mut flip, mut done) = (0i32, 0i32, false, false);
            keypad.acknowledge();
            keys.update(now / ms, keypad.pressed(), &mut |e| {
                let press = matches!(e.kind, Kind::Down);
                let step = press || matches!(e.kind, Kind::Repeat);
                match e.button {
                    Button::Right if step => by += 1,
                    Button::Left if step => by -= 1,
                    Button::Down if press => pick += 1,
                    Button::Up if press => pick -= 1,
                    Button::Enter if press => flip = !flip,
                    Button::Home if press => done = true,
                    _ => {}
                }
            });
            if done {
                break "Home";
            }
            if by == 0 && pick == 0 && !flip {
                continue;
            }
            last_input = now;
            unlogged = true;
            changes += 1;
            chosen = (chosen as i32 + pick).rem_euclid(DIALS as i32) as usize;
            if by != 0 {
                d = d.moved(chosen, by);
                // A dial moved: compare shows the dials again.
                original = false;
                bright_moved |= chosen == 4;
            }
            original ^= flip;
            let shown = if original { Dials::STOCK } else { d };
            write_gamma(display.regs(), &table(shown, &found));
            if bright_moved {
                let level = if original {
                    light
                } else {
                    (0, d.bright * COUNTS / DIAL_MAX, COUNTS - 1)
                };
                pwm.write32(CR, level.0);
                pwm.write32(PCR, level.2);
                pwm.write32(DCR, level.1);
            }
            caption(fb, d, chosen, original);
        };

        // The last setting, then everything back as found.
        let _ = writeln!(RunLog, "done ({reason}) after {changes} changes");
        log_dials(format_args!("last"), d);
        log_table("last table", &table(d, &found));
        write_gamma(display.regs(), &found);
        pwm.write32(CR, light.0);
        pwm.write32(PCR, light.2);
        pwm.write32(DCR, light.1);
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
