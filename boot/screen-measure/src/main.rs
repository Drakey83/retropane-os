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
    use rp_screen_measure::Schedule;
    use rp_screen_measure::program::{PROGRAM_BYTES, StepLight, placeholder};
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

    /// The program room: the PC writes a measurement program here in a copy
    /// of the image before sending it (rp_screen_measure::program). Empty:
    /// the built-in schedule.
    static PROGRAM: [u8; PROGRAM_BYTES] = placeholder();

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

    fn main() -> ! {
        rt::set_fault_handler(on_fault);
        rt::init_mmu();
        // The schedule counts from here: the loader's reply to the PC came
        // just before the jump.
        // A run lasts longer than the timer's 22-minute wrap: `Elapsed`, never
        // a 32-bit tick difference (that froze run 1 of the profile at 22 min).
        let mut clock = rt::Elapsed::start();
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
        // Read through an opaque reference: the room's contents are written
        // after the build, so the compiler mustn't fold them in.
        let room: &[u8] = core::hint::black_box(&PROGRAM[..]);
        let schedule = Schedule::from_room(room);
        let mut table = 0;
        for (j, w) in rp_screen_measure::straight_table().iter().enumerate() {
            display.regs().write32(GFXGAM0 + 4 * j, *w);
        }
        let on = display.regs().read32(GSCTRL) | GSCTRL_GAMMA;
        display.regs().write32(GSCTRL, on);

        let mut stopped = false;
        let wait_until = |until_ms: u32, keypad: &mut Keypad<_>, clock: &mut rt::Elapsed| -> bool {
            while clock.ms() < u64::from(until_ms) {
                rt::check_in();
                keypad.acknowledge();
                if keypad.pressed().contains(Button::Home) {
                    return true;
                }
                rt::delay_us(20_000);
            }
            false
        };
        let mut refused = false;
        match schedule {
            Err(e) => {
                let _ = writeln!(RunLog, "program refused: {}", e.name());
                refused = true;
            }
            Ok(schedule) => {
                let _ = writeln!(
                    RunLog,
                    "{} steps, {}",
                    schedule.len(),
                    if matches!(schedule, Schedule::BuiltIn) {
                        "built-in"
                    } else {
                        "program"
                    }
                );
                let mut at_ms = schedule.lead_ms();
                stopped |= wait_until(at_ms, &mut keypad, &mut clock);
                for i in 0..schedule.len() {
                    if stopped {
                        break;
                    }
                    let s = schedule.step(i);
                    let (r, g, b) = s.rgb;
                    let word = 0xFF00_0000 | u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b);
                    rp_hal_mem::fill32(fb, word);
                    if s.table != table {
                        for (j, w) in schedule.table(s.table).iter().enumerate() {
                            display.regs().write32(GFXGAM0 + 4 * j, *w);
                        }
                        table = s.table;
                    }
                    match s.light {
                        StepLight::Found => {
                            pwm.write32(CR, light.0);
                            pwm.write32(PCR, light.2);
                            pwm.write32(DCR, light.1);
                        }
                        StepLight::Counts(n) => {
                            pwm.write32(CR, 0);
                            pwm.write32(PCR, COUNTS - 1);
                            pwm.write32(DCR, u32::from(n).min(COUNTS));
                        }
                    }
                    let _ = writeln!(RunLog, "step {i} at {} ms (planned {at_ms})", clock.ms());
                    at_ms += s.ms;
                    stopped |= wait_until(at_ms, &mut keypad, &mut clock);
                }
            }
        }
        let _ = writeln!(
            RunLog,
            "{} at {} ms",
            if refused {
                "nothing shown"
            } else if stopped {
                "stopped (Home)"
            } else {
                "all patches shown"
            },
            clock.ms()
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
