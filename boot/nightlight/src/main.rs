//! RetroPane night-light experiment (item 76): how dim can the X51v's
//! screen go, without flicker or whine? For the owner's eyes and ears.
//!
//! Each numbered step holds for [`STEP_S`] seconds with its number large
//! on screen above a grey ramp. Windows Mobile's backlight runs at prescale
//! 1 and 216 counts (30.09 kHz); RetroPane's at prescale 0 and 432 counts,
//! the same frequency with steps half the size. Below the light's dimmest
//! step, the picture itself is dimmed by the display's graphics gamma table
//! (`GFXGAM0`-`16`, read first; only scaled if its entries look like 8-bit
//! grey ramps, else those steps are skipped and the raw table is logged).
//! Gamma found off (`GSCTRL` bit 26: the emulator's cold display; Windows
//! Mobile leaves it on) is switched on for the steps, with the table as
//! found, and off again at the end.
//!
//! | Step | What | The question |
//! |---|---|---|
//! | 1 | Windows Mobile's dimmest: 1 of 216 | reference |
//! | 2 | 2 of 432 | the same light as 1? |
//! | 3 | 1 of 432 | dimmer than 2, still lit? |
//! | 4 | 1 of 432, picture at 1/2 | dimmer still, picture fine? |
//! | 5 | 1 of 432, picture at 1/4 | as 4 |
//! | 6 | 0 and 1 of 432 alternating every 0.5 ms | flicker or whine? |
//! | 7 | 216 of 432 (half) | whine? (ear to the case) |
//! | 8 | 108 of 216 (half, Windows Mobile's cycle) | whine? (reference) |
//! | 9 | 1 of 432, picture at 1/8 (the design's darkest) | still readable in the dark? |
//!
//! Everything is put back as found at the end.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed once every step ran.
pub const MARKER: &str = "RETROPANE-NIGHTLIGHT-OK";

/// Seconds each step holds.
pub const STEP_S: u32 = 6;

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_drv_mbx_display::console::Console;
    use rp_drv_mbx_display::scanout::scale_gamma;
    use rp_drv_mbx_display::{Display, fill_rect_rgb565};
    use rp_hal_api::RegisterBus;
    use rp_hal_pxa27x::rt;
    use rp_hal_x51v::MBX_VRAM_SIZE;
    use rp_hal_x51v::devices;

    rp_image::entry!(main);

    /// PWM registers (rp-drv-pxa-pwm's `reg`).
    const CR: usize = 0x0;
    const DCR: usize = 0x4;
    const PCR: usize = 0x8;
    /// The graphics gamma table: 17 words from here (names from Intel's
    /// 2700G headers), and `GSCTRL`'s gamma enable.
    const GFXGAM0: usize = 0x2250;
    const GAMMA_WORDS: usize = 17;
    const GSCTRL: usize = 0x2000;
    const GSCTRL_GAMMA: u32 = 1 << 26;

    /// Whether `t` looks like 8-bit grey ramps (`0x00LLLLLL`, never
    /// falling): the only form this test dares to scale.
    fn grey_ramps(t: &[u32; GAMMA_WORDS]) -> bool {
        let lanes_equal = t.iter().all(|&w| {
            let b = w.to_le_bytes();
            b[3] == 0 && b[0] == b[1] && b[1] == b[2]
        });
        lanes_equal && t.windows(2).all(|p| p[1] & 0xFF >= p[0] & 0xFF) && t[16] & 0xFF > 0
    }

    struct Step {
        label: &'static str,
        prescale: u32,
        counts: u32,
        duty: u32,
        /// The picture's scale in 1/256ths (256: untouched).
        picture: u32,
        /// Alternate the duty with 0 every 0.5 ms.
        dither: bool,
    }

    const STEPS: [Step; 9] = [
        Step {
            label: "WM dimmest 1/216",
            prescale: 1,
            counts: 216,
            duty: 1,
            picture: 256,
            dither: false,
        },
        Step {
            label: "2/432: same as 1?",
            prescale: 0,
            counts: 432,
            duty: 2,
            picture: 256,
            dither: false,
        },
        Step {
            label: "1/432: dimmer, lit?",
            prescale: 0,
            counts: 432,
            duty: 1,
            picture: 256,
            dither: false,
        },
        Step {
            label: "1/432, picture 1/2",
            prescale: 0,
            counts: 432,
            duty: 1,
            picture: 128,
            dither: false,
        },
        Step {
            label: "1/432, picture 1/4",
            prescale: 0,
            counts: 432,
            duty: 1,
            picture: 64,
            dither: false,
        },
        Step {
            label: "0-1/432 at 1 kHz",
            prescale: 0,
            counts: 432,
            duty: 1,
            picture: 256,
            dither: true,
        },
        Step {
            label: "216/432: whine?",
            prescale: 0,
            counts: 432,
            duty: 216,
            picture: 256,
            dither: false,
        },
        Step {
            label: "108/216 (WM): whine?",
            prescale: 1,
            counts: 216,
            duty: 108,
            picture: 256,
            dither: false,
        },
        Step {
            label: "1/432, picture 1/8",
            prescale: 0,
            counts: 432,
            duty: 1,
            picture: 32,
            dither: false,
        },
    ];

    fn main() -> ! {
        rt::init_mmu();
        let (Some((regs, vram)), Some(mut pwm)) = (rp_image::take_mbx(), rt::take_pwm0()) else {
            rp_image::fail("no display or backlight PWM")
        };
        let mut display = Display::new(regs);
        let (plane, _) = display.adopt(MBX_VRAM_SIZE as usize);
        let stride = plane.stride;
        let fb = &mut vram[plane.offset..plane.offset + stride * usize::from(plane.height)];
        let mut log = devices::RunLog;

        // As found.
        let found = (pwm.read32(CR), pwm.read32(DCR), pwm.read32(PCR));
        let gsctrl = display.regs().read32(GSCTRL);
        let table: [u32; GAMMA_WORDS] =
            core::array::from_fn(|i| display.regs().read32(GFXGAM0 + 4 * i));
        let _ = writeln!(
            log,
            "PWM0 found: CR {:02X} DCR {:03X} PCR {:03X}; GSCTRL {gsctrl:08X}",
            found.0, found.1, found.2
        );
        let _ = write!(log, "GFXGAM:");
        for w in &table {
            let _ = write!(log, " {w:08X}");
        }
        let _ = writeln!(log);
        let gamma_ok = grey_ramps(&table);
        if !gamma_ok {
            let _ = writeln!(log, "gamma: not grey ramps: picture steps skipped");
        } else if gsctrl & GSCTRL_GAMMA == 0 {
            let _ = writeln!(log, "gamma: off, switched on for the steps");
            display.regs().write32(GSCTRL, gsctrl | GSCTRL_GAMMA);
        }

        // The ramp: 16 greys across the lower part of the screen.
        for i in 0..16usize {
            let g = (i * 255 / 15) as u16;
            let c = (g >> 3) << 11 | (g >> 2) << 5 | g >> 3;
            fill_rect_rgb565(fb, stride, i * 30, 400, 30, 240, c);
        }

        for (n, step) in STEPS.iter().enumerate() {
            let skipped = step.picture < 256 && !gamma_ok;
            {
                let mut big = Console::new(fb, stride, (0, 0), (480, 240), 12, (0xFFFF, 0x0000));
                let _ = write!(big, "{}", n + 1);
            }
            {
                let mut cap = Console::new(fb, stride, (0, 240), (480, 160), 3, (0xFFFF, 0x0000));
                let _ = write!(cap, "{}", if skipped { "(skipped)" } else { step.label });
            }
            let _ = writeln!(
                log,
                "step {}: {}{}",
                n + 1,
                step.label,
                if skipped { " (skipped)" } else { "" }
            );
            if skipped {
                continue;
            }
            pwm.write32(CR, step.prescale);
            pwm.write32(PCR, step.counts - 1);
            pwm.write32(DCR, step.duty);
            if gamma_ok {
                let t = scale_gamma(&table, step.picture);
                for (i, w) in t.iter().enumerate() {
                    display.regs().write32(GFXGAM0 + 4 * i, *w);
                }
            }
            let start = rt::ticks();
            let mut on = true;
            while rt::ticks().wrapping_sub(start) < super::STEP_S * rt::TICK_HZ {
                rt::check_in();
                if step.dither {
                    on = !on;
                    pwm.write32(DCR, u32::from(on) * step.duty);
                    rt::delay_us(500);
                } else {
                    rt::delay_us(50_000);
                }
            }
        }

        // Back as found.
        pwm.write32(CR, found.0);
        pwm.write32(PCR, found.2);
        pwm.write32(DCR, found.1);
        for (i, w) in table.iter().enumerate() {
            display.regs().write32(GFXGAM0 + 4 * i, *w);
        }
        display.regs().write32(GSCTRL, gsctrl);
        {
            let mut cap = Console::new(fb, stride, (0, 240), (480, 160), 3, (0xFFFF, 0x0000));
            let _ = write!(cap, "done: put back");
        }
        let _ = writeln!(log, "{}", super::MARKER);
        if devices::loader_present() {
            rt::delay_us(1_000_000);
        }
        rp_image::finish()
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}
