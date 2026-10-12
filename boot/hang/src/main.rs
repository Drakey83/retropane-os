//! Test image for the soft run limit (overnight item 55): started by the
//! resident loader, it says hello, then spins forever with interrupts off
//! and never checks in. The loader's soft watchdog must bring
//! it back with a crash report, without the hardware reset (which on the
//! X51v lands on Dell's factory-restore prompt). `cargo xtask emu` checks it.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed before it starts hanging.
pub const MARKER: &str = "RETROPANE-HANG-TEST";

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

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
            "{}: running forever on purpose",
            super::MARKER
        );
        rt::set_fault_handler(on_fault);
        rt::init_mmu();
        // Interrupts on, then off again, and spin: the hardest case (what
        // rp-bench did on the device), which only the FIQ-routed run limit
        // can stop. Never returns, never checks in.
        rt::start_tick(100);
        rt::irq_disable();
        loop {
            core::hint::spin_loop();
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
