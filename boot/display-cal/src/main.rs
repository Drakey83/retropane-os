//! RetroPane display calibration (docs/design/display-calibration.md): the
//! 2700G's gamma table, the backlight and saturation dialled in by eye on
//! the real panel, as a guided test.
//!
//! The picture ([`rp_display_cal::pixel`]) is drawn into a 32 bpp plane.
//! Six steps, one dial each ([`rp_display_cal::Dials`], 1-50, 25 =
//! unchanged): Black, White, Mid, Warm, Bright, Sat; then a seventh,
//! "Looks", flipping between ready-made settings made from the owner's
//! picks (`PRESETS`: Original, Accurate, Everyday, Dark games) and the
//! three tables measured with the colorimeter (`MEASURED`). Moving to another step
//! puts every dial back to 25 except that step's own pick (the owner's
//! request after run 4, where a reset to the wrong value spoilt the rest).
//!
//! | Input | Does |
//! |---|---|
//! | D-pad right / left | this step's dial up / down by 1 (held: repeats); last step: next / previous look |
//! | D-pad down / up | next / previous step, the others back to 25 |
//! | D-pad centre | compare: flips between this step and the original |
//! | Home | done: everything put back, back to the loader |
//!
//! The run log gets each step's pick when the step is left, and all picks
//! with their table at the end. After 10 minutes with no input it puts
//! everything back and returns. Display registers, the gamma table and the
//! backlight are restored as found.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed when the image puts everything back.
pub const MARKER: &str = "RETROPANE-DISPLAY-CAL-OK";

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_display_cal::{
        CAPTION_H, DIAL_NAMES, DIALS, Dials, H, LOOKS, NEUTRAL, PRESET_START, W, backlight,
        look_dials, look_name, look_table, pixel_sat, table,
    };
    use rp_drv_mbx_display::layers::{Layer, Snapshot};
    use rp_drv_mbx_display::scanout::GAMMA_ENTRIES;
    use rp_drv_mbx_display::{Display, PixelFormat, Plane, font};
    use rp_drv_pxa_keypad::keys::{Keys, Kind, Timing};
    use rp_drv_pxa_keypad::{Button, Keypad, Pressed};
    use rp_hal_api::RegisterBus;
    use rp_hal_pxa27x::rt;
    use rp_hal_x51v::MBX_VRAM_SIZE;
    use rp_hal_x51v::devices::RunLog;
    use rp_vram_map::bench32::SCREENS;

    rp_image::entry!(main);

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
    /// The steps: one per dial, then all together.
    const STEPS: usize = DIALS + 1;
    const STRIDE: usize = W as usize * 4;

    /// Writes one pixel (XRGB8888) of the 32 bpp screen.
    fn put(fb: &mut [u8], x: u32, y: u32, (r, g, b): (u8, u8, u8)) {
        let o = y as usize * STRIDE + x as usize * 4;
        fb[o..o + 4].copy_from_slice(&[b, g, r, 0xFF]);
    }

    /// The picture below the caption at saturation `sat`.
    fn draw(fb: &mut [u8], sat: u32) {
        for y in CAPTION_H..H {
            for x in 0..W {
                put(fb, x, y, pixel_sat(x, y, sat));
            }
            rt::check_in();
        }
    }

    /// Text at `(x, y)`, `scale` times the font's size, on black.
    fn text(fb: &mut [u8], x: u32, y: u32, scale: u32, colour: (u8, u8, u8), s: &str) {
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

    /// The caption: the step and its dial (or every pick, last step), and
    /// the controls; red with "ORIGINAL" while compare shows the original.
    fn caption(fb: &mut [u8], step: usize, picks: Dials, look: usize, original: bool) {
        for y in 0..CAPTION_H {
            for x in 0..W {
                put(fb, x, y, (0, 0, 0));
            }
        }
        let mut top = Line::new();
        let mut low = Line::new();
        if step < DIALS {
            let _ = write!(
                top,
                "{}/{STEPS} {} {}",
                step + 1,
                DIAL_NAMES[step],
                picks.get(step)
            );
        } else {
            let _ = write!(top, "{STEPS}/{STEPS} {}", look_name(look));
        }
        if original {
            let _ = write!(low, "ORIGINAL shown - OK to go back");
        } else if step < DIALS {
            let _ = write!(low, "<> adjust (25 = none)  v next  ^ back");
        } else {
            for i in 0..DIALS {
                let _ = write!(low, "{}{} ", &DIAL_NAMES[i][..2], look_dials(look).get(i));
            }
        }
        let colour = if original {
            (255, 60, 60)
        } else {
            (255, 220, 0)
        };
        text(fb, 6, 2, 3, colour, top.as_str());
        text(fb, 6, 32, 2, (200, 200, 200), low.as_str());
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

    /// What step `step` shows: its own pick with every other dial at 25,
    /// or look `look` for the last step; everything at 25 when comparing.
    fn shown(step: usize, picks: Dials, look: usize, original: bool) -> Dials {
        if original {
            Dials::STOCK
        } else if step < DIALS {
            Dials::STOCK.moved(step, picks.get(step) as i32 - NEUTRAL as i32)
        } else {
            look_dials(look)
        }
    }

    fn main() -> ! {
        rt::init_mmu();
        let (Some((regs, vram)), Some(mut pwm)) = (rp_image::take_mbx(), rt::take_pwm0()) else {
            rp_image::fail("no display or backlight PWM")
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
        // The found duty in 432ths, for the Bright dial.
        let found_duty = (light.1 & 0x3FF).min(light.2 + 1) * COUNTS / (light.2 + 1);

        // The picture, once, into screen 0 (rp-vram-map), then shown.
        let at = SCREENS[0].at as usize;
        let fb = &mut vram[at..at + STRIDE * H as usize];
        let t0 = rt::ticks();
        draw(fb, NEUTRAL);
        let (mut step, mut picks, mut original) = (0usize, Dials::STOCK, false);
        let mut look = PRESET_START;
        caption(fb, step, picks, look, original);
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
            rp_image::fail("no keypad")
        };
        let mut keypad = Keypad::new(kp);
        keypad.enable();
        keypad.acknowledge();
        let mut keys = Keys::new(Timing::DEFAULT);
        let ms = rt::TICK_HZ / 1000;
        let _ = writeln!(RunLog, "ready: step 1 {}", DIAL_NAMES[0]);

        let mut last_input = rt::ticks();
        let mut changes = 0u32;
        // The saturation the picture was last drawn at.
        let mut drawn_sat = NEUTRAL;
        let reason = loop {
            rt::check_in();
            rt::delay_us(10_000);
            let now = rt::ticks();
            if now.wrapping_sub(last_input) > IDLE_S * rt::TICK_HZ {
                break "no input for 10 minutes";
            }
            // This round's input: a dial step, a step change, compare, done.
            let (mut by, mut go, mut flip, mut done) = (0i32, 0i32, false, false);
            let scan = keypad.scan();
            let pressed = scan.held;
            keys.update(now / ms, scan, &mut |e| {
                let press = matches!(e.kind, Kind::Down);
                let step = press || matches!(e.kind, Kind::Repeat);
                match e.button {
                    Button::Right if step => by += 1,
                    Button::Left if step => by -= 1,
                    Button::Down if press => go += 1,
                    Button::Up if press => go -= 1,
                    Button::Enter if press => flip = !flip,
                    Button::Home if press => done = true,
                    _ => {}
                }
            });
            if done {
                break "Home";
            }
            if by != 0 || go != 0 || flip {
                last_input = now;
                changes += 1;
                if by != 0 && step < DIALS {
                    picks = picks.moved(step, by);
                    original = false;
                } else if by != 0 {
                    look = (look as i32 + by).rem_euclid(LOOKS as i32) as usize;
                    original = false;
                    let _ = writeln!(RunLog, "look {}", look_name(look));
                }
                if go != 0 {
                    if step < DIALS {
                        let _ = writeln!(
                            RunLog,
                            "step {} {}: {}",
                            step + 1,
                            DIAL_NAMES[step],
                            picks.get(step)
                        );
                    }
                    step = (step as i32 + go).clamp(0, STEPS as i32 - 1) as usize;
                    original = false;
                    let _ = writeln!(RunLog, "at {} ms: step {}", now / ms, step + 1);
                }
                original ^= flip;
                let d = shown(step, picks, look, original);
                // The last step's looks bring their own tables.
                let t = if step == DIALS && !original {
                    look_table(look, &found)
                } else {
                    table(d, &found)
                };
                write_gamma(display.regs(), &t);
                if d.bright == NEUTRAL {
                    pwm.write32(CR, light.0);
                    pwm.write32(PCR, light.2);
                    pwm.write32(DCR, light.1);
                } else {
                    pwm.write32(CR, 0);
                    pwm.write32(PCR, COUNTS - 1);
                    pwm.write32(DCR, backlight(d.bright, found_duty, COUNTS));
                }
                caption(fb, step, picks, look, original);
            }
            // Saturation redraws the picture, once the keys are let go.
            let want = shown(step, picks, look, original).sat;
            if want != drawn_sat && pressed == Pressed::NONE {
                draw(fb, want);
                drawn_sat = want;
            }
        };

        // Every pick, then everything back as found.
        let _ = write!(RunLog, "done ({reason}) after {changes} changes; picks:");
        for (i, name) in DIAL_NAMES.iter().enumerate() {
            let _ = write!(RunLog, " {name} {}", picks.get(i));
        }
        let _ = writeln!(RunLog);
        log_table("picks' table", &table(picks, &found));
        let _ = writeln!(RunLog, "last look: {}", look_name(look));
        write_gamma(display.regs(), &found);
        pwm.write32(CR, light.0);
        pwm.write32(PCR, light.2);
        pwm.write32(DCR, light.1);
        // The planes as found (GSCTRL's gamma bit with them).
        before.restore(display.regs());
        let _ = writeln!(RunLog, "{}", super::MARKER);
        rp_image::finish()
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}
