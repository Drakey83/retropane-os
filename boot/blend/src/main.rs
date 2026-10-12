//! Test image: plane blending at scan-out (overnight item 60). The
//! wallpaper goes in the 2700G's video plane (YUV), a glass panel with text
//! in the graphics plane with per-pixel alpha, and the display controller
//! mixes them as it scans out. Five stages ([`blendtest`]), each labelled
//! on screen and held [`STAGE_SECONDS`] so the owner can photograph it:
//! A video 4:2:2 + UI ARGB4444, B 4:2:0 + ARGB4444, C 4:2:0 + ARGB8888,
//! D as C with the video gamma bit off, E 4:2:0 + dithered ARGB4444. It
//! checks in with the hardware watchdog, since all five outlast the
//! deadline the loader sets at launch.
//!
//! The run log gets the display registers as found and, per stage, read
//! back after programming (do they hold what was written?). Every register
//! it changes is put back before it returns, and the loader takes the
//! screen with `Display::adopt` anyway, so even a crash here leaves the
//! loader's screen visible. Uses 2700G memory from 1 MiB up only; never
//! touches flash or the cards.
//!
//! [`blendtest`]: rp_drv_mbx_display::blendtest

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed (serial and run log) with the stage letter once a stage is up.
pub const MARKER: &str = "RETROPANE-BLEND-";
/// How long each stage stays up, unless the build sets `RP_BLEND_SECONDS`.
pub const STAGE_SECONDS: u32 = 15;
/// Which stages to show (letters), unless the build sets `RP_BLEND_STAGES`
/// (e.g. `RP_BLEND_STAGES=CDE RP_BLEND_SECONDS=20 cargo xtask load
/// rp-blend` to look at a few for longer).
pub const STAGES: &str = match option_env!("RP_BLEND_STAGES") {
    Some(s) => s,
    None => "ABCDE",
};

/// The seconds per stage this build uses.
#[must_use]
pub fn stage_seconds() -> u32 {
    option_env!("RP_BLEND_SECONDS")
        .and_then(|s| s.parse().ok())
        .filter(|&n| (1..=60).contains(&n))
        .unwrap_or(STAGE_SECONDS)
}

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_drv_mbx_display::blendtest::{self, Stage};
    use rp_drv_mbx_display::layers::{self, Snapshot};
    use rp_drv_mbx_display::{Display, reg};
    use rp_drv_pxa_uart::PxaUart;
    use rp_hal_api::RegisterBus;
    use rp_hal_pxa27x::rt;
    use rp_hal_x51v::MBX_VRAM_SIZE;
    use rp_hal_x51v::devices::RunLog;

    rp_hal_pxa27x::entry!(main);

    /// Registers worth recording, as found and per stage.
    const SHOWN: [(&str, usize); 6] = [
        ("VOVRCLK", layers::reg::VOVRCLK),
        ("VSCTRL", layers::reg::VSCTRL),
        ("VSCADR", layers::reg::VSCADR),
        ("VUBASE", layers::reg::VUBASE),
        ("GSCTRL", reg::GSCTRL),
        ("GSCADR", reg::GSCADR),
    ];

    fn record<B: RegisterBus>(regs: &mut B, what: &str) {
        let _ = write!(RunLog, "{what}:");
        for (name, off) in SHOWN {
            let _ = write!(RunLog, " {name} {:08X}", regs.read32(off));
        }
        let _ = writeln!(RunLog);
    }

    fn main() -> ! {
        rt::set_fault_handler(on_fault);
        rt::init_mmu();
        let Some((regs, vram)) = rp_hal_x51v::devices::take_mbx() else {
            crash(format_args!("no display"))
        };
        let mut uart = rt::take_ffuart().map(|bus| {
            let mut u = PxaUart::new(bus);
            u.init(115_200);
            u
        });
        let mut d = Display::new(regs);
        record(d.regs(), "found");
        // Panel timing and an opaque graphics plane, as every image does;
        // that state is what goes back at the end.
        let _ = d.adopt(MBX_VRAM_SIZE as usize);
        let before = Snapshot::take(d.regs());

        for stage in Stage::ALL {
            blendtest::draw(stage, vram);
        }
        for stage in Stage::ALL
            .into_iter()
            .filter(|s| super::STAGES.contains(s.letter()))
        {
            blendtest::program(&mut d, stage);
            record(d.regs(), stage.label()[0]);
            let c = stage.letter();
            if let Some(u) = uart.as_mut() {
                let _ = writeln!(u, "{}{c}", super::MARKER);
            }
            let _ = writeln!(RunLog, "{}{c}", super::MARKER);
            for _ in 0..super::stage_seconds() * 10 {
                rt::delay_us(100_000);
                check_in();
            }
        }
        before.restore(d.regs());
        record(d.regs(), "restored");
        rp_hal_x51v::devices::return_to_loader();
        loop {
            core::hint::spin_loop();
        }
    }

    /// Checks in with the soft run limit and, when the loader armed it, the
    /// hardware watchdog (its deadline is fixed at launch, and all the
    /// stages together run longer than that).
    fn check_in() {
        /// OS timer ticks per second.
        const TICK_1S: u32 = 3_250_000;
        rt::keep_alive();
        let mut wd = rt::watchdog();
        if wd.armed() {
            wd.feed(30 * TICK_1S);
        }
    }

    fn on_fault(kind: rt::Fault, addr: u32) -> ! {
        crash(format_args!("FAULT {} at {addr:#010x}", kind.name()))
    }

    #[inline(never)]
    fn crash(msg: core::fmt::Arguments) -> ! {
        let mut uart = PxaUart::new(rt::emergency_uart());
        let _ = writeln!(uart, "{msg}");
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
