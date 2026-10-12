//! Test image for the crash path: started by the resident loader, it says
//! hello and panics on purpose. The panic goes the way every image's crash
//! goes ([`rp_hal_x51v::devices::crash_to_loader`]): into the run log, back
//! to the loader, to the PC as a crash, and onto the card as
//! `crashlog.txt`. `cargo xtask emu` checks all of it.
//!
//! Before panicking it moves the visible plane to offset 0 of the 2700G's
//! memory, where Windows Mobile leaves it on a real X51v (the emulator
//! starts from reset, so the loader set one up at `FRAMEBUFFER_OFFSET`),
//! and draws the crash box every image draws
//! ([`rp_drv_mbx_display::crash::emergency_console`]): the box must show
//! on that plane, not land off screen.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed before the deliberate panic.
pub const MARKER: &str = "RETROPANE-CRASH-TEST";

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_drv_mbx_display::{Display, PixelFormat, Plane, X51V_VGA, crash};
    use rp_drv_pxa_uart::PxaUart;
    use rp_hal_pxa27x::rt;

    rp_hal_pxa27x::entry!(main);

    fn main() -> ! {
        if let Some(bus) = rt::take_ffuart() {
            let mut uart = PxaUart::new(bus);
            uart.init(115_200);
            let _ = writeln!(uart, "{}", super::MARKER);
        }
        let _ = writeln!(
            rp_hal_x51v::devices::RunLog,
            "{}: panicking on purpose",
            super::MARKER
        );
        rt::set_fault_handler(on_fault);
        if let Some((regs, _vram)) = rp_hal_x51v::devices::take_mbx() {
            let wm = Plane {
                offset: 0,
                ..Plane::fullscreen(&X51V_VGA, PixelFormat::Rgb565)
            };
            Display::new(regs).enable(&X51V_VGA, &wm);
        }
        panic!("deliberate test panic");
    }

    fn on_fault(kind: rt::Fault, addr: u32) -> ! {
        crash(format_args!("FAULT {} at {addr:#010x}", kind.name()))
    }

    #[inline(never)]
    fn crash(msg: core::fmt::Arguments) -> ! {
        let mut uart = PxaUart::new(rt::emergency_uart());
        let _ = writeln!(uart, "{msg}");
        if let Some((regs, vram)) = rp_hal_x51v::devices::emergency_mbx() {
            let mut c =
                crash::emergency_console(regs, vram, (0, 576), (480, 64), 2, (0xFFFF, 0xA800));
            let _ = write!(c, "{msg}");
        }
        // Long enough for `cargo xtask emu` to see the box before the
        // loader's own crash screen replaces it.
        if rp_hal_x51v::devices::loader_present() {
            rt::delay_us(3_000_000);
        }
        rp_hal_x51v::devices::crash_to_loader(msg);
        loop {
            core::hint::spin_loop();
        }
    }

    #[panic_handler]
    fn panic(info: &core::panic::PanicInfo) -> ! {
        crash(format_args!("PANIC {}", info.message()))
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}
