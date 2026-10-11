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

    use rp_drv_mbx_display::{Display, PixelFormat, Plane, X51V_VGA, pattern};
    use rp_drv_pxa_uart::PxaUart;
    use rp_hal_pxa27x::rt;

    rp_hal_pxa27x::entry!(main);

    fn main() -> ! {
        if let Some(bus) = rt::take_ffuart() {
            let mut uart = PxaUart::new(bus);
            uart.init(115_200);
            let _ = writeln!(uart, "\nRetroPane OS: hello from the Dell Axim X51v");
            let _ = writeln!(uart, "CPU ID: {:#010x}", rt::cpu_id());
            if let Some((regs, vram)) = rp_hal_x51v::devices::take_mbx() {
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
