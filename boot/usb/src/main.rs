//! RetroPane USB link test (ADR-0012 step U1): the X51v as a COM port.
//!
//! Plug the Axim's sync cable (or cradle) into a PC, run this from the SD
//! card, and the PC finds a serial port with its built-in driver. Whatever a
//! terminal sends comes back ("echo"), and the screen shows what the host
//! has done so far: cable detect, bus resets, SETUP packets, the
//! configuration, whether a program has the port open, and bytes each way.
//! `cargo xtask usb` finds the port and talks to it.
//!
//! The top of the screen is the live status (redrawn twice a second); below
//! it, what the host sent.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Sent to the host when a program opens the port.
pub const BANNER: &str = "RetroPane 26 USB console OK\r\n";

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_drv_mbx_display::console::Console;
    use rp_drv_mbx_display::{Display, crash, reg};
    use rp_drv_pxa_gpio::Gpio;
    use rp_drv_pxa_uart::PxaUart;
    use rp_drv_pxa_udc::Udc;
    use rp_drv_pxa_udc::reg::{UP2OCR, UP3OCR};
    use rp_hal_api::RegisterBus;
    use rp_hal_pxa27x::rt;
    use rp_hal_x51v::MBX_VRAM_SIZE;
    use rp_hal_x51v::devices::beacon;

    rp_hal_pxa27x::entry!(main);

    /// D+ pull-up and cable detect (docs/hw/udc.md).
    const USB_PULLUP: u8 = 2;
    const USB_DETECT: u8 = 14;
    /// OS timer ticks (3.25 MHz) between status redraws: 0.5 s.
    const REDRAW: u32 = 1_625_000;
    /// Status area height in pixels (10 rows of 16 px at scale 2).
    const STATUS_H: usize = 10 * 16;
    const COLORS: (u16, u16) = (0xFFFF, 0x0000);
    const STATUS_COLORS: (u16, u16) = (0x0000, 0xFFE0);

    fn on_fault(kind: rt::Fault, addr: u32) -> ! {
        crash(format_args!("FAULT {} at {addr:#010x}", kind.name()))
    }

    #[inline(never)]
    fn crash(msg: core::fmt::Arguments) -> ! {
        let mut uart = PxaUart::new(rt::emergency_uart());
        let _ = writeln!(uart, "\n{msg}");
        if let Some((regs, vram)) = rp_hal_x51v::devices::emergency_mbx() {
            let mut c =
                crash::emergency_console(regs, vram, (0, 576), (480, 64), 2, (0xFFFF, 0xA800));
            let _ = write!(c, "{msg}");
        }
        halt()
    }

    fn main() -> ! {
        // Start-up colours (as in rp-light): a hang leaves the last one on
        // screen. Red: started; green: MMU on; blue: 2700G answered.
        beacon(0xF800, 0xF800);
        let Some(bus) = rt::take_ffuart() else { halt() };
        let mut uart = PxaUart::new(bus);
        uart.init(115_200);
        rt::set_fault_handler(on_fault);
        rt::init_mmu();
        beacon(0x07E0, 0x07E0);

        // The screen Windows Mobile left (as in rp-light), else our own.
        let Some((mut regs, vram)) = rp_hal_x51v::devices::take_mbx() else {
            halt()
        };
        // The first register read: a hang here leaves the green colour up.
        let _ = regs.read32(reg::GSCTRL);
        beacon(0x001F, 0x001F);
        let (plane, _) = Display::new(regs).adopt(MBX_VRAM_SIZE as usize);
        let end = plane.offset + plane.stride * usize::from(plane.height);
        let (top, rest) = vram[plane.offset..end].split_at_mut(plane.stride * STATUS_H);
        let mut log = Console::new(rest, plane.stride, (0, 0), (480, 640 - STATUS_H), 2, COLORS);

        let Some(gpio_bus) = rt::take_gpio() else {
            halt()
        };
        let mut gpio = Gpio::new(gpio_bus);
        gpio.make_input(USB_DETECT);
        // Disconnect first, so the host sees a fresh device (Windows Mobile
        // left the pull-up on), then enable and connect.
        gpio.make_output(USB_PULLUP, false);
        rt::delay_us(200_000);

        let Some(udc_bus) = rt::take_udc() else {
            halt()
        };
        let mut udc = Udc::new(udc_bus);
        // Port control as Windows Mobile left it: if WM used the PXA's
        // internal D+ pull-up, UP2OCR says so (docs/hw/udc.md open question).
        let (up2, up3) = (udc.read_reg(UP2OCR), udc.read_reg(UP3OCR));
        let _ = writeln!(log, "WM left UP2OCR {up2:08X} UP3OCR {up3:08X}");
        match udc.enable() {
            Ok(()) => {
                let _ = writeln!(log, "UDC enabled, connecting");
                let _ = writeln!(uart, "UDC enabled");
            }
            Err(e) => {
                let _ = writeln!(log, "UDC error {e:?}");
                let _ = writeln!(uart, "UDC error");
                halt()
            }
        }
        // Connect: GPIO2 high (confirmed on the device, docs/hw/udc.md).
        gpio.write(USB_PULLUP, true);
        let _ = writeln!(uart, "{}", super::MARKER);

        let mut last_draw = rt::ticks().wrapping_sub(REDRAW);
        let mut was_open = false;
        let mut buf = [0u8; 64];
        loop {
            udc.poll();
            let open = udc.host_open();
            if open && !was_open {
                udc.write(super::BANNER.as_bytes());
                let _ = writeln!(log, "host opened the port");
            }
            was_open = open;
            let n = udc.read(&mut buf);
            if n > 0 {
                udc.write(&buf[..n]);
                for &b in &buf[..n] {
                    let c = if b == b'\r' { '\n' } else { char::from(b) };
                    if c == '\n' || (' '..='~').contains(&c) {
                        log.put(c);
                    }
                }
            }
            if rt::ticks().wrapping_sub(last_draw) >= REDRAW {
                last_draw = rt::ticks();
                let s = udc.stats();
                let mut st = Console::new(
                    &mut *top,
                    plane.stride,
                    (0, 0),
                    (480, STATUS_H),
                    2,
                    STATUS_COLORS,
                );
                let _ = writeln!(st, "RetroPane 26 USB link test");
                let _ = writeln!(
                    st,
                    "cable GPIO14={} GPIO2 high",
                    u8::from(gpio.read(USB_DETECT))
                );
                let _ = writeln!(st, "resets {} setups {}", s.resets, s.setups);
                let _ = writeln!(st, "stalls {} last {:04X}", s.stalls, s.last_request);
                let _ = writeln!(
                    st,
                    "configured {} open {}",
                    yes(udc.configured()),
                    yes(open)
                );
                let _ = writeln!(st, "rx {} tx {}", s.rx_bytes, s.tx_bytes);
                let _ = writeln!(st, "ep0 in {} csr {:03X}", s.ep0_in, s.setup_csr);
                let r = s.recent;
                let _ = writeln!(
                    st,
                    "req {:04X} {:04X} {:04X} {:04X}",
                    r[0], r[1], r[2], r[3]
                );
                let _ = write!(st, "soft-reset to go back");
            }
        }
    }

    fn yes(b: bool) -> &'static str {
        if b { "yes" } else { "no" }
    }

    fn halt() -> ! {
        loop {
            core::hint::spin_loop();
        }
    }

    #[panic_handler]
    fn panic(info: &core::panic::PanicInfo) -> ! {
        match info.location() {
            Some(l) => crash(format_args!("PANIC {}:{}", l.file(), l.line())),
            None => crash(format_args!("PANIC")),
        }
    }
}

/// Printed on serial once the UDC is enabled and connected.
pub const MARKER: &str = "RETROPANE-USB-READY";

#[cfg(not(target_os = "none"))]
fn main() {}
