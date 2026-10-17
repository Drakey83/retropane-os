//! RetroPane screen measurement (docs/design/display-calibration.md): full
//! screen patches on a fixed schedule ([`rp_screen_measure::PATCHES`]) for
//! a colorimeter resting on the screen, read by `cargo xtask measure`.
//!
//! The gamma table is set straight (output = input) for the run, so the
//! readings are the panel's own response; the backlight follows each
//! patch. Home stops early. The display registers, the gamma table and the
//! backlight are put back as found, then it returns to the loader.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed when the image puts everything back.
pub const MARKER: &str = "RETROPANE-SCREEN-MEASURE-OK";

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_drv_mbx_display::layers::{Layer, Snapshot};
    use rp_drv_mbx_display::{Display, PixelFormat, Plane, crash};
    use rp_drv_pxa_keypad::{Button, Keypad};
    use rp_hal_api::RegisterBus;
    use rp_hal_pxa27x::rt;
    use rp_hal_x51v::MBX_VRAM_SIZE;
    use rp_hal_x51v::devices::{self, RunLog};
    use rp_screen_measure::{LEAD_MS, Light, PATCHES, start_ms, straight_table};
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
    const W: usize = 480;
    const H: usize = 640;
    const STRIDE: usize = W * 4;

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

    /// Milliseconds since `t0` (OS timer ticks).
    fn ms_since(t0: u32) -> u32 {
        rt::ticks().wrapping_sub(t0) / (rt::TICK_HZ / 1000)
    }

    fn main() -> ! {
        rt::set_fault_handler(on_fault);
        rt::init_mmu();
        // The schedule counts from here: the loader's reply to the PC came
        // just before the jump.
        let t0 = rt::ticks();
        let (Some((regs, vram)), Some(mut pwm), Some(kp)) =
            (devices::take_mbx(), rt::take_pwm0(), rt::take_keypad())
        else {
            halt()
        };
        let mut display = Display::new(regs);
        let _ = display.adopt(MBX_VRAM_SIZE as usize);
        let before = Snapshot::take(display.regs());
        let gsctrl = display.regs().read32(GSCTRL);
        let found: [u32; 17] = core::array::from_fn(|i| display.regs().read32(GFXGAM0 + 4 * i));
        let light = (pwm.read32(CR), pwm.read32(DCR), pwm.read32(PCR));
        let _ = writeln!(
            RunLog,
            "PWM0 found: CR {:02X} DCR {:03X} PCR {:03X}",
            light.0, light.1, light.2
        );
        let mut keypad = Keypad::new(kp);
        keypad.enable();
        keypad.acknowledge();

        // Black, straight table, then the patches.
        let at = SCREENS[0].at as usize;
        let fb = &mut vram[at..at + STRIDE * H];
        rp_hal_mem::fill32(fb, 0xFF00_0000);
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
        for (i, w) in straight_table().iter().enumerate() {
            display.regs().write32(GFXGAM0 + 4 * i, *w);
        }
        let on = display.regs().read32(GSCTRL) | GSCTRL_GAMMA;
        display.regs().write32(GSCTRL, on);

        let mut stopped = false;
        let wait_until = |until_ms: u32, keypad: &mut Keypad<_>| -> bool {
            while ms_since(t0) < until_ms {
                rt::check_in();
                keypad.acknowledge();
                if keypad.pressed().contains(Button::Home) {
                    return true;
                }
                rt::delay_us(20_000);
            }
            false
        };
        stopped |= wait_until(LEAD_MS, &mut keypad);
        for (i, p) in PATCHES.iter().enumerate() {
            if stopped {
                break;
            }
            let (r, g, b) = p.rgb;
            let word = 0xFF00_0000 | u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b);
            rp_hal_mem::fill32(fb, word);
            match p.light {
                Light::Found => {
                    pwm.write32(CR, light.0);
                    pwm.write32(PCR, light.2);
                    pwm.write32(DCR, light.1);
                }
                Light::Off => {
                    pwm.write32(CR, 0);
                    pwm.write32(PCR, COUNTS - 1);
                    pwm.write32(DCR, 0);
                }
                Light::Eighths(n) => {
                    pwm.write32(CR, 0);
                    pwm.write32(PCR, COUNTS - 1);
                    pwm.write32(DCR, COUNTS * n.min(8) / 8);
                }
            }
            let _ = writeln!(
                RunLog,
                "patch {i} {} at {} ms (planned {})",
                p.name,
                ms_since(t0),
                start_ms(i)
            );
            stopped |= wait_until(start_ms(i) + p.ms, &mut keypad);
        }
        let _ = writeln!(
            RunLog,
            "{} at {} ms",
            if stopped {
                "stopped (Home)"
            } else {
                "all patches shown"
            },
            ms_since(t0)
        );

        // Everything back as found.
        for (i, w) in found.iter().enumerate() {
            display.regs().write32(GFXGAM0 + 4 * i, *w);
        }
        pwm.write32(CR, light.0);
        pwm.write32(PCR, light.2);
        pwm.write32(DCR, light.1);
        before.restore(display.regs());
        display.regs().write32(GSCTRL, gsctrl);
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
