//! Test image for the crash path: started by the resident loader, it says
//! hello and crashes on purpose. The crash goes the way every image's crash
//! goes (`rp-image`'s crash path, quality audit C38): the serial line, the
//! crash box on screen, the run log, back to the loader, to the PC as a
//! crash, and onto the card as `crashlog.txt`. `cargo xtask emu` checks all
//! of it.
//!
//! It panics, unless built with one of the fault switches, which make it
//! fault instead (MMU on, so the bring-up map's unmapped 0x80000000 aborts):
//! `RP_CRASH_DATA` a data abort (a read there), `RP_CRASH_PREFETCH` a
//! prefetch abort (a jump there), `RP_CRASH_UNDEFINED` an undefined
//! instruction. `emu-crash` runs all four.
//!
//! Before crashing it moves the visible plane to offset 0 of the 2700G's
//! memory, where Windows Mobile leaves it on a real X51v (the emulator
//! starts from reset, so the loader set one up at `FRAMEBUFFER_OFFSET`):
//! the crash box must show on that plane, not land off screen.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed before the deliberate crash.
pub const MARKER: &str = "RETROPANE-CRASH-TEST";

/// An address nothing is mapped at in the bring-up map.
pub const UNMAPPED: usize = 0x8000_0000;

/// The crash this build makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `panic!` (the default).
    Panic,
    /// A read of [`UNMAPPED`].
    DataAbort,
    /// A jump to [`UNMAPPED`].
    PrefetchAbort,
    /// `UDF #0`.
    Undefined,
}

/// Built with `RP_CRASH_BOARD` (the emulator's check of quality audit C39):
/// before crashing, it changes every kind of board state the loader keeps
/// (`rp_hal_x51v::board`): the amplifier and a `0x1C` bit, the backlight
/// (dark), a gamma word and the gamma enable, `MBXCLK`'s divider and
/// `M24CLK`. The loader must put each back. For the emulator: on the
/// device it would switch the speaker amplifier (an owned bit) for a
/// moment.
pub const BOARD: bool = option_env!("RP_CRASH_BOARD").is_some();

/// The crash this build makes, from the build switches (the first set wins,
/// in the order data, prefetch, undefined).
pub const KIND: Kind = if option_env!("RP_CRASH_DATA").is_some() {
    Kind::DataAbort
} else if option_env!("RP_CRASH_PREFETCH").is_some() {
    Kind::PrefetchAbort
} else if option_env!("RP_CRASH_UNDEFINED").is_some() {
    Kind::Undefined
} else {
    Kind::Panic
};

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_drv_mbx_display::{Display, PixelFormat, Plane, X51V_VGA};
    use rp_drv_pxa_uart::PxaUart;
    use rp_hal_pxa27x::rt;

    use super::{BOARD, KIND, Kind, MARKER, UNMAPPED};

    /// `RP_CRASH_BOARD`: every kind of kept board state changed (see
    /// [`BOARD`]); `regs` the 2700G's window.
    fn change_board(regs: &mut rp_hal_pxa27x::Mmio) {
        use rp_hal_api::RegisterBus;
        if let Some(bus) = rp_hal_x51v::devices::take_cpld() {
            let (mut c, _) = rp_drv_cpld::Cpld::adopt(bus.0, bus.1);
            let (a, u) = (c.shadow(0x14), c.shadow(0x1C));
            if a & 2 == 0 {
                c.set(0x14, 2)
            } else {
                c.clear(0x14, 2)
            }
            if u & 4 == 0 {
                c.set(0x1C, 4)
            } else {
                c.clear(0x1C, 4)
            }
        }
        if let Some(mut p) = rt::take_pwm0() {
            p.write32(rp_drv_pxa_pwm::reg::DCR, 0);
        }
        {
            use rp_drv_mbx_display::{clock::MBXCLK, reg};
            let g = regs.read32(reg::GFXGAM0);
            regs.write32(reg::GFXGAM0, g ^ 0x0001_0101);
            let s = regs.read32(reg::GSCTRL);
            regs.write32(reg::GSCTRL, s ^ (1 << 26));
            let m = regs.read32(MBXCLK);
            rp_drv_mbx_display::clock::restore(regs, m ^ (1 << 2));
            let v = regs.read32(rp_drv_mved::M24CLK);
            regs.write32(rp_drv_mved::M24CLK, v ^ (1 << 1));
        }
        let _ = writeln!(
            rp_hal_x51v::devices::RunLog,
            "{MARKER}: the board changed (amplifier, 0x1C bit 2, backlight dark, gamma, MBXCLK, M24CLK)"
        );
    }

    rp_image::entry!(main);

    fn main() -> ! {
        if let Some(bus) = rt::take_ffuart() {
            let mut uart = PxaUart::new(bus);
            uart.init(115_200);
            let _ = writeln!(uart, "{MARKER}");
        }
        let _ = writeln!(
            rp_hal_x51v::devices::RunLog,
            "{MARKER}: crashing on purpose ({KIND:?})"
        );
        if KIND != Kind::Panic {
            rt::init_mmu();
        }
        if let Some((mut regs, _vram)) = rp_image::take_mbx() {
            if BOARD {
                change_board(&mut regs);
            }
            let wm = Plane {
                offset: 0,
                ..Plane::fullscreen(&X51V_VGA, PixelFormat::Rgb565)
            };
            Display::new(regs).enable(&X51V_VGA, &wm);
        }
        match KIND {
            Kind::Panic => panic!("deliberate test panic"),
            Kind::DataAbort => {
                let _ = core::hint::black_box(rt::fault_probe(core::hint::black_box(UNMAPPED)));
                rp_image::fail("the data abort probe didn't fault")
            }
            Kind::PrefetchAbort => rt::prefetch_probe(UNMAPPED),
            Kind::Undefined => rt::undefined_probe(),
        }
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}
