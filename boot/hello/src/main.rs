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
    use rp_drv_mbx_display::console::{self, Console};
    use rp_drv_mbx_display::{Display, PixelFormat, Plane, X51V_VGA, pattern};

    use rp_drv_pxa_gpio::{Gpio, GpioPin};
    use rp_drv_pxa_keypad::{Keypad, Pressed};
    use rp_drv_pxa_mmc::PxaMmc;
    use rp_drv_pxa_ssp::Ssp;
    use rp_drv_pxa_uart::PxaUart;
    use rp_drv_sdcard::{SdCard, mbr_partitions};
    use rp_drv_tsc2046::{Calibration, Tsc2046};
    use rp_hal_pxa27x::rt;

    rp_hal_pxa27x::entry!(main);

    /// Input polling rate: buttons and touch are sampled each tick, with the
    /// CPU idle in between.
    const TICK_HZ: u32 = 200;

    fn on_fault(kind: rt::Fault, addr: u32) -> ! {
        let mut uart = PxaUart::new(rt::emergency_uart());
        let _ = writeln!(uart, "\nFAULT {} at {addr:#010x}", kind.name());
        loop {
            core::hint::spin_loop();
        }
    }

    /// Touch screen chip select and pen interrupt (docs/hw/touch.md).
    const TS_CS: u8 = 24;
    const TS_PENIRQ: u8 = 94;

    /// Writes to the UART and, once the display is up, the on-screen console.
    struct Out<U> {
        uart: U,
        con: Option<Console<'static>>,
    }

    impl<U: Write> Write for Out<U> {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            if let Some(con) = self.con.as_mut() {
                let _ = con.write_str(s);
            }
            self.uart.write_str(s)
        }
    }

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
            let mut out = Out { uart, con: None };
            rt::set_fault_handler(on_fault);
            rt::init_mmu();
            let _ = writeln!(out, "\nRetroPane 26: hello from the Dell Axim X51v");
            let _ = writeln!(out, "CPU ID: {:#010x}", rt::cpu_id());
            let mut cpld = rp_hal_x51v::devices::take_cpld().map(Cpld::new);
            let vga = match cpld.as_mut() {
                Some(cpld) => {
                    let panel = cpld.panel(&mut TimerDelay);
                    let _ = writeln!(out, "Panel: {}", panel.name());
                    panel.is_vga()
                }
                None => true,
            };
            if !vga {
                let _ = writeln!(out, "QVGA panel: display setup skipped (not supported yet)");
            } else if let Some((regs, vram)) = rp_hal_x51v::devices::take_mbx() {
                let plane = Plane::fullscreen(&X51V_VGA, PixelFormat::Rgb565);
                pattern::fill_rgb565(
                    &mut vram[plane.offset..],
                    plane.width.into(),
                    plane.height.into(),
                    plane.stride,
                );
                Display::new(regs).enable(&X51V_VGA, &plane);
                // From here on, everything printed also appears on screen: the
                // real X51v has no serial port to read.
                let (x, y, w, h) = console::BOOT_LOG;
                out.con = Some(Console::new(
                    &mut vram[plane.offset..],
                    plane.stride,
                    (x, y),
                    (w, h),
                    console::BOOT_LOG_SCALE,
                    console::BOOT_LOG_COLORS,
                ));
                let _ = writeln!(out, "RetroPane 26");
                let _ = writeln!(
                    out,
                    "Display: {}x{} test pattern",
                    plane.width, plane.height
                );
            }
            // SD card: power it through the CPLD, identify it, read the MBR.
            if let (Some(cpld), Some(mmc)) = (cpld.as_mut(), rt::take_mmc()) {
                cpld.set_sd_power(true);
                rt::delay_us(10_000);
                match SdCard::init(PxaMmc::new(mmc)) {
                    Ok(mut card) => {
                        let kind = if card.is_high_capacity() {
                            "SDHC"
                        } else {
                            "SD"
                        };
                        let _ = writeln!(out, "SD: {kind} card, RCA {:#06x}", card.rca());
                        let mut block = [0u8; rp_drv_sdcard::BLOCK];
                        match card.read(0, &mut block) {
                            Ok(()) => match mbr_partitions(&block) {
                                Some(parts) => {
                                    for (i, p) in parts.iter().enumerate() {
                                        if let Some(p) = p {
                                            let _ = writeln!(
                                                out,
                                                "SD: partition {} type {:#04x} start {} blocks {}",
                                                i + 1,
                                                p.kind,
                                                p.start,
                                                p.blocks
                                            );
                                        }
                                    }
                                }
                                None => {
                                    let _ = writeln!(out, "SD: no partition table");
                                }
                            },
                            Err(e) => {
                                let _ = writeln!(out, "SD: read failed: {}", e.name());
                            }
                        }
                    }
                    Err(e) => {
                        let _ = writeln!(out, "SD: no card ({})", e.name());
                    }
                }
            }
            let _ = writeln!(out, "{}", super::MARKER);

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

                rt::start_tick(TICK_HZ);
                let mut last_keys = Pressed::default();
                let mut last_touch = None;
                loop {
                    let keys = keypad.pressed();
                    for (button, down) in last_keys.changes(keys) {
                        let state = if down { "down" } else { "up" };
                        let _ = writeln!(out, "KEY {} {state}", button.name());
                    }
                    last_keys = keys;

                    let now = touch.sample().map(|t| Calibration::X51V.to_screen(t));
                    if now != last_touch {
                        match now {
                            Some((x, y)) => {
                                let _ = writeln!(out, "TOUCH {x} {y}");
                            }
                            None => {
                                let _ = writeln!(out, "TOUCH up");
                            }
                        }
                        last_touch = now;
                    }
                    // Sleep until the next tick instead of spinning.
                    rt::wait_tick();
                }
            }
        }
        loop {
            core::hint::spin_loop();
        }
    }

    #[panic_handler]
    fn panic(info: &core::panic::PanicInfo) -> ! {
        let mut uart = PxaUart::new(rt::emergency_uart());
        // Location only: formatting the full message would pull in much more
        // of core::fmt (ADR-0005 rule 8).
        match info.location() {
            Some(l) => {
                let _ = writeln!(uart, "\nPANIC at {}:{}", l.file(), l.line());
            }
            None => {
                let _ = writeln!(uart, "\nPANIC");
            }
        }
        loop {
            core::hint::spin_loop();
        }
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}
