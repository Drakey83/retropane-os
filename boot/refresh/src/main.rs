//! RetroPane refresh-rate test (display calibration step C3,
//! docs/design/display-calibration.md), owner watching: the X51v's frame
//! rate measured at Windows Mobile's display PLL (56 Hz) and at the panel's
//! typical pixel clock (60 Hz, [`rp_refresh::FAST_DISPPLL`]).
//!
//! A 32 bpp picture with a bar moving one step a frame (motion to judge)
//! and the measured rate large. Frames are counted from `DINTRS` VEVENT0
//! over 2 s after every switch, with any under/overrun bits (the display
//! running short of data at the faster clock). The PLL is changed only if
//! it holds exactly Windows Mobile's value; otherwise the test only
//! measures.
//!
//! | Input | Does |
//! |---|---|
//! | D-pad centre | switches 56 Hz / 60 Hz, then measures |
//! | Home | done: the PLL and display put back as found, back to the loader |
//!
//! After 3 minutes with no input it puts everything back by itself.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed when the image puts everything back.
pub const MARKER: &str = "RETROPANE-REFRESH-OK";

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_drv_mbx_display::layers::{Layer, Snapshot};
    use rp_drv_mbx_display::{Display, PixelFormat, Plane, ack_events, event, font};
    use rp_drv_pxa_keypad::{Button, Keypad, Pressed};
    use rp_hal_api::RegisterBus;
    use rp_hal_pxa27x::rt;
    use rp_hal_x51v::MBX_VRAM_SIZE;
    use rp_hal_x51v::devices::RunLog;
    use rp_refresh::{FAST_DISPPLL, WM_DISPPLL, measured_centi_hz, pll_khz};
    use rp_vram_map::bench32::SCREENS;

    rp_image::entry!(main);

    const DISPPLL: usize = 0x03C;
    const PLLSTAT: usize = 0x040;
    /// `PLLSTAT` bit 0: the display PLL locked (docs/hw/2700g.md).
    const DISP_LOCKED: u32 = 1;
    const W: usize = 480;
    const H: usize = 640;
    const STRIDE: usize = W * 4;
    /// The moving bar's band and size.
    const BAR_Y: usize = 300;
    const BAR_H: usize = 240;
    const BAR_W: usize = 24;
    const STEP: usize = 6;
    const IDLE_S: u32 = 180;

    fn put(fb: &mut [u8], x: usize, y: usize, (r, g, b): (u8, u8, u8)) {
        let o = y * STRIDE + x * 4;
        fb[o..o + 4].copy_from_slice(&[b, g, r, 0xFF]);
    }

    fn rect(fb: &mut [u8], x: usize, y: usize, w: usize, h: usize, c: (u8, u8, u8)) {
        for yy in y..y + h {
            for xx in x..(x + w).min(W) {
                put(fb, xx, yy, c);
            }
        }
    }

    /// Text at `(x, y)`, `scale` times the font's size, on black.
    fn text(fb: &mut [u8], x: usize, y: usize, scale: usize, c: (u8, u8, u8), s: &str) {
        let cell = (font::WIDTH + 1) * scale;
        for (i, ch) in s.chars().enumerate() {
            let px = x + i * cell;
            if px + cell > W {
                break;
            }
            let g = font::glyph(ch);
            for gy in 0..(font::HEIGHT + 1) * scale {
                for gx in 0..cell {
                    let (fx, fy) = (gx / scale, gy / scale);
                    let lit = fx < font::WIDTH
                        && fy < font::HEIGHT
                        && g[fy] & (1 << (font::WIDTH - 1 - fx)) != 0;
                    put(fb, px + gx, y + gy, if lit { c } else { (0, 0, 0) });
                }
            }
        }
    }

    /// A short string on the stack.
    struct Line {
        buf: [u8; 40],
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
                buf: [0; 40],
                len: 0,
            }
        }
        fn as_str(&self) -> &str {
            core::str::from_utf8(&self.buf[..self.len]).unwrap_or("?")
        }
    }

    /// The caption: which clock, the measured rate, and the faults seen.
    fn caption(fb: &mut [u8], fast: bool, centi: u32, faults: u32) {
        rect(fb, 0, 0, W, 200, (0, 0, 0));
        let mut l = Line::new();
        let _ = write!(l, "{}.{} Hz", centi / 100, centi / 10 % 10);
        text(fb, 12, 20, 8, (255, 255, 255), l.as_str());
        let mut l = Line::new();
        let _ = write!(
            l,
            "{}",
            if fast {
                "60 Hz clock (panel's)"
            } else {
                "56 Hz clock (Dell's)"
            }
        );
        text(fb, 12, 100, 3, (255, 220, 0), l.as_str());
        let mut l = Line::new();
        if faults == 0 {
            let _ = write!(l, "no picture faults");
        } else {
            let _ = write!(l, "FAULTS {faults:08X}");
        }
        text(
            fb,
            12,
            140,
            2,
            if faults == 0 {
                (150, 255, 150)
            } else {
                (255, 80, 80)
            },
            l.as_str(),
        );
        text(fb, 12, 170, 2, (180, 180, 180), "OK switch  Home done");
    }

    struct Bar {
        x: usize,
    }

    impl Bar {
        /// One step right (wrapping): the old bar's trailing strip erased,
        /// the new leading strip drawn.
        fn step(&mut self, fb: &mut [u8]) {
            let old = self.x;
            self.x = (self.x + STEP) % (W - BAR_W);
            if self.x < old {
                rect(fb, old, BAR_Y, BAR_W, BAR_H, (20, 20, 40));
                rect(fb, self.x, BAR_Y, BAR_W, BAR_H, (255, 255, 255));
            } else {
                rect(fb, old, BAR_Y, STEP, BAR_H, (20, 20, 40));
                rect(fb, old + BAR_W, BAR_Y, STEP, BAR_H, (255, 255, 255));
            }
        }
    }

    fn main() -> ! {
        rt::init_mmu();
        let (Some((regs, vram)), Some(kp)) = (rp_image::take_mbx(), rt::take_keypad()) else {
            rp_image::fail("no display or keypad")
        };
        let mut display = Display::new(regs);
        // Before the takeover: `adopt` moves Windows Mobile's clock to 60 Hz.
        let found = display.regs().read32(DISPPLL);
        let _ = display.adopt(MBX_VRAM_SIZE as usize);
        // The test starts at Dell's clock again, as found.
        if found == WM_DISPPLL {
            display.regs().write32(DISPPLL, found);
        }
        let before = Snapshot::take(display.regs());
        let _ = writeln!(
            RunLog,
            "DISPPLL found {found:03X} ({:?} kHz), PLLSTAT {:X}",
            pll_khz(found),
            display.regs().read32(PLLSTAT)
        );
        let may_switch = found == WM_DISPPLL;
        if !may_switch {
            let _ = writeln!(
                RunLog,
                "DISPPLL is not Windows Mobile's {WM_DISPPLL:03X}: no switch, measuring only"
            );
        }
        let mut keypad = Keypad::new(kp);
        keypad.enable();
        keypad.acknowledge();

        let at = SCREENS[0].at as usize;
        let fb = &mut vram[at..at + STRIDE * H];
        rp_hal_mem::fill32(fb, 0xFF00_0000);
        rect(fb, 0, BAR_Y, W, BAR_H, (20, 20, 40));
        // A grey ramp under the bar: smoothness and any shimmer at 60 Hz.
        for x in 0..W {
            let v = (x * 255 / (W - 1)) as u8;
            rect(fb, x, 580, 1, 50, (v, v, v));
        }
        display.set_graphics(
            &Plane {
                format: PixelFormat::Argb8888,
                width: W as u16,
                height: H as u16,
                offset: at,
                stride: STRIDE,
            },
            Layer::SOLO,
        );
        let mut bar = Bar { x: 0 };
        let ms = rt::TICK_HZ / 1000;

        // Counts frames for 2 s, the bar moving one step each; returns the
        // rate and the under/overrun bits seen.
        let measure = |display: &mut Display<_>, fb: &mut [u8], bar: &mut Bar| -> (u32, u32) {
            let regs = display.regs();
            let _ = ack_events(regs, event::VEVENT0 | event::UNDER_OVERRUN);
            let t0 = rt::ticks();
            let (mut frames, mut faults) = (0u32, 0u32);
            while rt::ticks().wrapping_sub(t0) < 2_000 * ms {
                let s = ack_events(regs, event::VEVENT0 | event::UNDER_OVERRUN);
                faults |= s & event::UNDER_OVERRUN;
                if s & event::VEVENT0 != 0 {
                    frames += 1;
                    bar.step(fb);
                }
                rt::check_in();
                rt::delay_us(300);
            }
            let took = rt::ticks().wrapping_sub(t0) / ms;
            (measured_centi_hz(frames, took), faults)
        };

        let mut fast = false;
        let (centi, faults) = measure(&mut display, fb, &mut bar);
        let _ = writeln!(RunLog, "56 Hz clock: {centi} centi-Hz, faults {faults:08X}");
        caption(fb, fast, centi, faults);

        let mut held = Pressed::NONE;
        let mut last_input = rt::ticks();
        let reason = loop {
            rt::check_in();
            let s = ack_events(display.regs(), event::VEVENT0);
            if s & event::VEVENT0 != 0 {
                bar.step(fb);
            }
            keypad.acknowledge();
            let now = keypad.pressed();
            let mut flip = false;
            let mut done = false;
            for (b, down) in held.changes(now) {
                match b {
                    Button::Enter if down => flip = true,
                    Button::Home if down => done = true,
                    _ => {}
                }
            }
            held = now;
            if done {
                break "Home";
            }
            if rt::ticks().wrapping_sub(last_input) > IDLE_S * rt::TICK_HZ {
                break "no input for 3 minutes";
            }
            if flip {
                last_input = rt::ticks();
                if may_switch {
                    fast = !fast;
                    let v = if fast { FAST_DISPPLL } else { WM_DISPPLL };
                    display.regs().write32(DISPPLL, v);
                    let t0 = rt::ticks();
                    while display.regs().read32(PLLSTAT) & DISP_LOCKED == 0
                        && rt::ticks().wrapping_sub(t0) < 100 * ms
                    {
                        rt::delay_us(100);
                    }
                    let _ = writeln!(
                        RunLog,
                        "DISPPLL {v:03X}: locked after {} us",
                        rt::ticks().wrapping_sub(t0) * 1000 / ms
                    );
                }
                let (centi, faults) = measure(&mut display, fb, &mut bar);
                let _ = writeln!(
                    RunLog,
                    "{} clock: {centi} centi-Hz, faults {faults:08X}",
                    if fast { "60 Hz" } else { "56 Hz" }
                );
                caption(fb, fast, centi, faults);
            }
            rt::delay_us(500);
        };

        // Everything back as found.
        if may_switch {
            display.regs().write32(DISPPLL, found);
            let t0 = rt::ticks();
            while display.regs().read32(PLLSTAT) & DISP_LOCKED == 0
                && rt::ticks().wrapping_sub(t0) < 100 * ms
            {
                rt::delay_us(100);
            }
        }
        before.restore(display.regs());
        let _ = writeln!(
            RunLog,
            "done ({reason}); DISPPLL back to {:03X}",
            display.regs().read32(DISPPLL)
        );
        let _ = writeln!(RunLog, "{}", super::MARKER);
        rp_image::finish()
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}
