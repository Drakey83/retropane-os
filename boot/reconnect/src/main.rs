//! USB reconnect experiment (ADR-0012): which signal makes the PC see the
//! Axim leave and come back?
//!
//! On the device, the PC never noticed the loader disconnecting (GPIO2 low,
//! UDC off) and reconnecting: every run needed a cable replug. This image,
//! started by the loader, enables the UDC and then tries each candidate
//! "disconnect for a second, reconnect" action in turn, counting the bus
//! resets and SETUP packets the PC sends in the three seconds after. A
//! candidate that makes the PC re-enumerate is the connect switch. Results
//! go to the screen and the run log (`cargo xtask log` after a replug).
//!
//! Candidates: GPIO2 (AxDroid's guess, already seen not to work, as the
//! control), GPIO2 as an input, the UDC's internal D+ pull-up (UP2OCR
//! DPPUE), the D+ pull-down (UP2OCR DPPDE), CPLD 0x1C bit 3 (AxDroid sets
//! it before registering USB: "USB power?"), the UDC off and on, and CPLD
//! bit 3 with the UDC off and on.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed on serial when every candidate has been tried.
pub const MARKER: &str = "RETROPANE-RECONNECT-DONE";

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_drv_cpld::{Cpld, handover};
    use rp_drv_mbx_display::Display;
    use rp_drv_mbx_display::console::Console;
    use rp_drv_pxa_gpio::Gpio;
    use rp_drv_pxa_uart::PxaUart;
    use rp_drv_pxa_udc::Udc;
    use rp_drv_pxa_udc::reg::{DPPUE, HXOE, UP2OCR};
    use rp_hal_pxa27x::Mmio;
    use rp_hal_pxa27x::rt;
    use rp_hal_x51v::MBX_VRAM_SIZE;
    use rp_hal_x51v::devices::{self, RunLog};

    rp_hal_pxa27x::entry!(main);

    const USB_PULLUP: u8 = 2;
    /// UP2OCR: D+ pull-down enable.
    const DPPDE: u32 = 1 << 2;
    /// CPLD register AxDroid sets at boot ("USB power?": bit 3) and the
    /// bits it sets there (bits 1-2 at boot, bit 3 before USB).
    const CPLD_USB_REG: usize = 0x1C;
    const CPLD_BOOT_BITS: u16 = 0x0006;
    const CPLD_USB_BIT: u16 = 0x0008;
    const TICK_1S: u32 = 3_250_000;

    struct Rig {
        gpio: Gpio<Mmio>,
        udc: Udc<Mmio>,
        cpld: Cpld<Mmio, handover::Table<'static>>,
    }

    enum Act {
        Gpio2Low,
        Gpio2Input,
        PullupOff,
        PulldownOn,
        CpldOff,
        UdcOff,
        CpldAndUdcOff,
    }

    const CANDIDATES: [(&str, Act); 7] = [
        ("GPIO2 low/high", Act::Gpio2Low),
        ("GPIO2 input/high", Act::Gpio2Input),
        ("UP2OCR DPPUE off/on", Act::PullupOff),
        ("UP2OCR DPPDE on/off", Act::PulldownOn),
        ("CPLD 1C bit3 off/on", Act::CpldOff),
        ("UDC off/on", Act::UdcOff),
        ("CPLD bit3 + UDC off/on", Act::CpldAndUdcOff),
    ];

    impl Rig {
        fn up2(&mut self) -> u32 {
            self.udc.read_reg(UP2OCR)
        }

        /// The "disconnect" half of a candidate.
        fn off(&mut self, a: &Act) {
            match a {
                Act::Gpio2Low => self.gpio.write(USB_PULLUP, false),
                Act::Gpio2Input => self.gpio.make_input(USB_PULLUP),
                Act::PullupOff => {
                    let v = self.up2();
                    self.udc.write_reg(UP2OCR, v & !DPPUE);
                }
                Act::PulldownOn => {
                    let v = self.up2();
                    self.udc.write_reg(UP2OCR, v | DPPDE);
                }
                Act::CpldOff => self.cpld.clear(CPLD_USB_REG, CPLD_USB_BIT),
                Act::UdcOff => self.udc.disable(),
                Act::CpldAndUdcOff => {
                    self.cpld.clear(CPLD_USB_REG, CPLD_USB_BIT);
                    self.udc.disable();
                }
            }
        }

        /// The "reconnect" half.
        fn on(&mut self, a: &Act) {
            match a {
                Act::Gpio2Low => self.gpio.write(USB_PULLUP, true),
                Act::Gpio2Input => self.gpio.make_output(USB_PULLUP, true),
                Act::PullupOff => {
                    let v = self.up2();
                    self.udc.write_reg(UP2OCR, v | DPPUE | HXOE);
                }
                Act::PulldownOn => {
                    let v = self.up2();
                    self.udc.write_reg(UP2OCR, v & !DPPDE);
                }
                Act::CpldOff => self.cpld.set(CPLD_USB_REG, CPLD_USB_BIT),
                Act::UdcOff => {
                    let _ = self.udc.enable();
                }
                Act::CpldAndUdcOff => {
                    self.cpld.set(CPLD_USB_REG, CPLD_USB_BIT);
                    let _ = self.udc.enable();
                }
            }
        }
    }

    fn on_fault(kind: rt::Fault, addr: u32) -> ! {
        let mut uart = PxaUart::new(rt::emergency_uart());
        let _ = writeln!(uart, "\nFAULT {} at {addr:#010x}", kind.name());
        let _ = writeln!(RunLog, "FAULT {} at {addr:#010x}", kind.name());
        devices::return_to_loader();
        halt()
    }

    fn main() -> ! {
        let Some(bus) = rt::take_ffuart() else { halt() };
        let mut uart = PxaUart::new(bus);
        uart.init(115_200);
        rt::set_fault_handler(on_fault);
        rt::init_mmu();

        let Some((regs, vram)) = devices::take_mbx() else {
            halt()
        };
        let (plane, _) = Display::new(regs).adopt(MBX_VRAM_SIZE as usize);
        let end = plane.offset + plane.stride * usize::from(plane.height);
        let mut con = Console::new(
            &mut vram[plane.offset..end],
            plane.stride,
            (0, 0),
            (480, 640),
            2,
            (0xFFFF, 0x0000),
        );
        let mut out = Tee(&mut uart, &mut con);
        let _ = writeln!(out, "RetroPane 26 USB reconnect experiment");

        let (Some(gpio_bus), Some(udc_bus), Some(cpld_bus)) =
            (rt::take_gpio(), rt::take_udc(), devices::take_cpld())
        else {
            let _ = writeln!(out, "devices unavailable");
            halt()
        };
        let mut rig = Rig {
            gpio: Gpio::new(gpio_bus),
            udc: Udc::new(udc_bus),
            cpld: Cpld::adopt(cpld_bus.0, cpld_bus.1).0,
        };
        rig.gpio.make_output(USB_PULLUP, true);
        // AxDroid's boot bits + USB bit, on top of what the shared shadow
        // says is latched (`handover`), so clearing the USB bit keeps the rest.
        rig.cpld.set(CPLD_USB_REG, CPLD_BOOT_BITS | CPLD_USB_BIT);
        if rig.udc.enable().is_err() {
            let _ = writeln!(out, "UDC enable failed");
            halt()
        }
        let _ = writeln!(out, "UDC on, UP2OCR {:08X}", rig.up2());

        for (name, act) in &CANDIDATES {
            // Settle, then measure what the PC does on its own first.
            let before = settle(&mut rig, TICK_1S);
            rig.off(act);
            settle(&mut rig, TICK_1S);
            rig.on(act);
            let after = settle(&mut rig, 3 * TICK_1S);
            let (r, s) = (
                after.resets.wrapping_sub(before.resets),
                after.setups.wrapping_sub(before.setups),
            );
            let _ = writeln!(
                out,
                "{name}: resets {r} setups {s} configured {}",
                u8::from(rig.udc.configured())
            );
        }
        let _ = writeln!(out, "{}", super::MARKER);
        rt::delay_us(3_000_000);
        devices::return_to_loader();
        halt()
    }

    /// Polls the UDC for `ticks`, then returns its counters. Feeds the
    /// watchdog meanwhile: the loader arms it with a 30 s run limit, and this
    /// experiment runs for about 40 s (the first run on the device was reset
    /// by it mid-way, into Dell's factory-restore prompt).
    fn settle(rig: &mut Rig, ticks: u32) -> rp_drv_pxa_udc::Stats {
        let t = rt::ticks();
        while rt::ticks().wrapping_sub(t) < ticks {
            // Both deadlines, the soft run limit first (the loader's first
            // line of defence).
            rt::check_in();
            rig.udc.poll();
        }
        rig.udc.stats()
    }

    /// Writes to the UART, the screen and the run log.
    struct Tee<'a, 'b>(&'a mut PxaUart<Mmio>, &'a mut Console<'b>);

    impl Write for Tee<'_, '_> {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let _ = self.1.write_str(s);
            devices::log_write(s.as_bytes());
            self.0.write_str(s)
        }
    }

    fn halt() -> ! {
        loop {
            core::hint::spin_loop();
        }
    }

    #[panic_handler]
    fn panic(_: &core::panic::PanicInfo) -> ! {
        let _ = writeln!(RunLog, "PANIC");
        devices::return_to_loader();
        halt()
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}
