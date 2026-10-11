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
    use rp_hal_pxa27x::soc::irq;

    rp_hal_pxa27x::entry!(main);

    /// Sampling rate while a button or the pen is held. Otherwise the tick is
    /// off and the CPU sleeps until an interrupt.
    const TICK_HZ: u32 = 200;

    fn on_fault(kind: rt::Fault, addr: u32) -> ! {
        let mut out = crash_out();
        let _ = writeln!(out, "\nFAULT {} at {addr:#010x}", kind.name());
        loop {
            core::hint::spin_loop();
        }
    }

    /// Crash output: the UART plus, if the display is up, a red box at the
    /// bottom of the screen (the real X51v has no serial port).
    fn crash_out() -> Out<PxaUart<rp_hal_pxa27x::Mmio>> {
        let con = rp_hal_x51v::devices::emergency_vram().map(|vram| {
            let plane = Plane::fullscreen(&X51V_VGA, PixelFormat::Rgb565);
            let (x, y, w, h) = console::BOOT_LOG;
            Console::new(
                &mut vram[plane.offset..],
                plane.stride,
                (x, y + h - 64),
                (w, 64),
                console::BOOT_LOG_SCALE,
                CRASH_COLORS,
            )
        });
        Out {
            uart: PxaUart::new(rt::emergency_uart()),
            con,
        }
    }

    /// Crash box colors (RGB565): white on red.
    const CRASH_COLORS: (u16, u16) = (0xFFFF, 0xA800);

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
            let _ = writeln!(out, "\nRetroPane 26 on the Axim X51v");
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
                let _ = writeln!(out, "QVGA panel: not supported yet");
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
                    // Integers print as u32 only: one formatter per kind for
                    // the whole image (ADR-0005 rule 8).
                    "Display: {}x{} test pattern",
                    u32::from(plane.width),
                    u32::from(plane.height)
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
                        let _ =
                            writeln!(out, "SD: {kind} card, RCA {:#06x}", u32::from(card.rca()));
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
                                                u32::from(p.kind),
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
            if !out.uart.is_alive() {
                // Expected on a real X51v (no serial port fitted): say so
                // rather than dropping output silently.
                let _ = writeln!(out, "UART: none, screen only");
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

                // Interrupt-driven (ADR-0007): asleep until a button or the
                // pen wakes us; the tick runs only while something is held.
                let (Some(mut keys_irq), Some(mut pen_irq)) = (
                    rt::take_irq(irq::KEYPAD),
                    rt::take_gpio_irq(TS_PENIRQ, rt::Edge::Falling),
                ) else {
                    let _ = writeln!(out, "IRQ lines unavailable");
                    halt()
                };
                let mut last_keys = Pressed::default();
                let mut last_touch = None;
                let mut last_unclaimed = 0;
                let mut ticking = false;
                loop {
                    // Interrupts that fired with no owner were masked; say so.
                    let unclaimed = rt::unclaimed_irqs();
                    if unclaimed != last_unclaimed {
                        // Sources 32-39, then 0-31 (u32 formatting only).
                        let _ = writeln!(
                            out,
                            "IRQ unclaimed, masked: {:#x}:{:08x}",
                            (unclaimed >> 32) as u32,
                            unclaimed as u32
                        );
                        last_unclaimed = unclaimed;
                    }

                    // Buttons: acknowledge the controller before unmasking.
                    let key_event = keys_irq.take();
                    if key_event {
                        keypad.acknowledge();
                    }
                    let keys = keypad.pressed();
                    for (button, down) in last_keys.changes(keys) {
                        let state = if down { "down" } else { "up" };
                        let _ = writeln!(out, "KEY {} {state}", button.name());
                    }
                    last_keys = keys;
                    if key_event {
                        keys_irq.done();
                    }

                    // Touch: pen-down interrupt starts sampling; the pen line
                    // stays disarmed while we poll.
                    let pen_event = pen_irq.take();
                    let now = if pen_event || last_touch.is_some() || touch.pen_down() {
                        touch.sample().map(|t| Calibration::X51V.to_screen(t))
                    } else {
                        None
                    };
                    if now != last_touch {
                        match now {
                            Some((x, y)) => {
                                // As u32: one integer formatter for the whole image.
                                let _ = writeln!(out, "TOUCH {} {}", u32::from(x), u32::from(y));
                            }
                            None => {
                                let _ = writeln!(out, "TOUCH up");
                            }
                        }
                        last_touch = now;
                    }

                    let active =
                        last_touch.is_some() || keys != Pressed::default() || touch.pen_down();
                    if active {
                        if !ticking {
                            rt::start_tick(TICK_HZ);
                            ticking = true;
                        }
                        rt::wait_tick();
                    } else {
                        if ticking {
                            rt::stop_tick();
                            ticking = false;
                        }
                        // Re-arm pen-down, then re-check the level: an edge
                        // while it was disarmed isn't remembered.
                        pen_irq.done();
                        if !touch.pen_down() {
                            rt::wait_any_signal();
                        }
                    }
                }
            }
        }
        loop {
            core::hint::spin_loop();
        }
    }

    fn halt() -> ! {
        loop {
            core::hint::spin_loop();
        }
    }

    #[panic_handler]
    fn panic(info: &core::panic::PanicInfo) -> ! {
        let mut uart = crash_out();
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
