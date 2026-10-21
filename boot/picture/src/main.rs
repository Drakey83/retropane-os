//! Test image: puts a 480 x 640 picture on the X51v's screen for
//! [`SHOW_SECONDS`], then goes back to the resident loader. For looking at UI
//! concepts on the real panel (its colours, size and sharpness) before any UI
//! code exists. The picture comes from `local/concept/picture.rgb565` at build
//! time (see `build.rs`); it is never committed.
//!
//! Stays well inside the loader's 30 s run limit, checks in while it waits,
//! and never touches flash or the cards.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed once the picture is on screen.
pub const MARKER: &str = "RETROPANE-PICTURE-SHOWN";
/// How long the picture stays up.
pub const SHOW_SECONDS: u32 = 20;

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_drv_mbx_display::Display;
    use rp_drv_pxa_uart::PxaUart;
    use rp_hal_pxa27x::rt;
    use rp_hal_x51v::MBX_VRAM_SIZE;

    use rp_image::crash;

    rp_image::entry!(main);

    static PICTURE: &[u8; 480 * 640 * 2] =
        include_bytes!(concat!(env!("OUT_DIR"), "/picture.rgb565"));

    fn main() -> ! {
        rt::init_mmu();
        let Some((regs, vram)) = rp_image::take_mbx() else {
            crash(format_args!("no display"))
        };
        // Keep the plane already scanning out (no flicker) or set one up.
        let (plane, _) = Display::new(regs).adopt(MBX_VRAM_SIZE as usize);
        for (y, row) in PICTURE.as_chunks::<{ 480 * 2 }>().0.iter().enumerate() {
            let at = plane.offset + y * plane.stride;
            vram[at..at + row.len()].copy_from_slice(row);
        }
        if let Some(bus) = rt::take_ffuart() {
            let mut uart = PxaUart::new(bus);
            uart.init(115_200);
            let _ = writeln!(uart, "{}", super::MARKER);
        }
        let _ = writeln!(rp_hal_x51v::devices::RunLog, "{}", super::MARKER);
        for _ in 0..super::SHOW_SECONDS * 10 {
            rt::delay_us(100_000);
            rt::keep_alive();
        }
        rp_image::finish()
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}
