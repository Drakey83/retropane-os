//! RetroPane bring-up image.
//!
//! The first RetroPane code to run on the X51v: it proves the toolchain,
//! the startup code in `rp-hal-pxa27x::rt`, the UART driver and the 2700G
//! display driver (a color-bar test pattern), in the emulator and on real
//! hardware. `cargo xtask emu` boots it in QEMU, checks for [`MARKER`] and
//! verifies the screen.
//!
//! On the host this builds as an empty program so the workspace's host
//! checks still work.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed once bring-up succeeds; the emulator test looks for it.
pub const MARKER: &str = "RETROPANE-HELLO-OK";

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use core::cell::RefCell;
    use rp_drv_cpld::{Cpld, Delay};
    use rp_drv_mbx_display::{Display, PixelFormat, Plane, X51V_VGA, pattern};

    use rp_drv_pxa_gpio::{Gpio, GpioPin};
    use rp_drv_pxa_keypad::{Keypad, Pressed};
    use rp_drv_pxa_ssp::Ssp;
    use rp_drv_pxa_uart::PxaUart;
    use rp_drv_tsc2046::{Calibration, Tsc2046};
    use rp_hal_pxa27x::rt;

    rp_hal_pxa27x::entry!(main);

    /// Touch screen chip select and pen interrupt (docs/hw/touch.md).
    const TS_CS: u8 = 24;
    const TS_PENIRQ: u8 = 94;

    struct TimerDelay;
    impl Delay for TimerDelay {
        fn delay_us(&mut self, us: u32) {
            rt::delay_us(us);
        }
    }

    fn main() -> ! {
        if let Some(bus) = rt::take_ffuart() {
            let mut uart = PxaUart::new(bus);
            uart.init(115_200);
            let _ = writeln!(uart, "\nRetroPane OS: hello from the Dell Axim X51v");
            let _ = writeln!(uart, "CPU ID: {:#010x}", rt::cpu_id());
            let vga = match rp_hal_x51v::devices::take_cpld() {
                Some(bus) => {
                    let panel = Cpld::new(bus).panel(&mut TimerDelay);
                    let _ = writeln!(uart, "Panel: {}", panel.name());
                    panel.is_vga()
                }
                None => true,
            };
            if !vga {
                let _ = writeln!(
                    uart,
                    "QVGA panel: display setup skipped (not supported yet)"
                );
            } else if let Some((regs, vram)) = rp_hal_x51v::devices::take_mbx() {
                let plane = Plane::fullscreen(&X51V_VGA, PixelFormat::Rgb565);
                pattern::fill_rgb565(
                    &mut vram[plane.offset..],
                    plane.width.into(),
                    plane.height.into(),
                    plane.stride,
                );
                Display::new(regs).enable(&X51V_VGA, &plane);
                let _ = writeln!(
                    uart,
                    "Display: {}x{} test pattern",
                    plane.width, plane.height
                );
            }
            let _ = writeln!(uart, "{}", super::MARKER);

            // Report buttons and touches until the end of time (or the emulator).
            if let (Some(kp), Some(gpio), Some(ssp)) =
                (rt::take_keypad(), rt::take_gpio(), rt::take_ssp1())
            {
                let mut keypad = Keypad::new(kp);
                keypad.enable();

                let gpio = RefCell::new(Gpio::new(gpio));
                gpio.borrow_mut().make_output(TS_CS, true);
                gpio.borrow_mut().make_input(TS_PENIRQ);
                let mut ssp = Ssp::new(ssp);
                ssp.configure_spi(2_600_000);
                let mut touch = Tsc2046::new(
                    ssp,
                    GpioPin::new(&gpio, TS_CS),
                    GpioPin::new(&gpio, TS_PENIRQ),
                );

                let mut last_keys = Pressed::default();
                let mut last_touch = None;
                loop {
                    let keys = keypad.pressed();
                    for (button, down) in last_keys.changes(keys) {
                        let state = if down { "down" } else { "up" };
                        let _ = writeln!(uart, "KEY {} {state}", button.name());
                    }
                    last_keys = keys;

                    let now = touch.sample().map(|t| Calibration::X51V.to_screen(t));
                    if now != last_touch {
                        match now {
                            Some((x, y)) => {
                                let _ = writeln!(uart, "TOUCH {x} {y}");
                            }
                            None => {
                                let _ = writeln!(uart, "TOUCH up");
                            }
                        }
                        last_touch = now;
                    }
                    rt::delay_us(5_000);
                }
            }
        }
        loop {
            core::hint::spin_loop();
        }
    }

    #[panic_handler]
    fn panic(_: &core::panic::PanicInfo) -> ! {
        loop {
            core::hint::spin_loop();
        }
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}
