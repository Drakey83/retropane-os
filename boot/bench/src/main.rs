//! RetroPane benchmark image.
//!
//! Runs the shared benchmark suite (`rp-bench-suite`) and prints one line per
//! result: `BENCH <name> <ticks> <iterations>` (OS-timer ticks, 3.25 MHz),
//! self-checks as `CHECK <name> OK|FAIL`, then `BENCH-DONE`. `cargo xtask
//! bench` boots it in the emulator in deterministic `-icount` mode and
//! compares the results with the committed baseline. On the real X51v, the
//! probe image (`rp-probe`) runs the same suite and shows it on screen.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed after the last result.
pub const DONE: &str = "BENCH-DONE";

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;
    use core::hint::black_box;

    use rp_bench_suite as suite;
    use rp_drv_cpld::Cpld;
    use rp_drv_mbx_display::{PixelFormat, Plane, X51V_VGA};
    use rp_drv_pxa_mmc::PxaMmc;
    use rp_drv_pxa_uart::PxaUart;
    use rp_drv_sdcard::SdCard;
    use rp_hal_pxa27x::{Mmio, diag, rt};

    rp_hal_pxa27x::entry!(main);

    /// Prints results on the serial port.
    struct Serial(PxaUart<Mmio>);

    impl suite::Sink for Serial {
        fn result(&mut self, name: &'static str, ticks: u32, iters: u32) {
            let _ = writeln!(self.0, "BENCH {name} {ticks} {iters}");
        }
        fn check(&mut self, name: &'static str, ok: bool, detail: &str) {
            let verdict = if ok { "OK" } else { "FAIL" };
            let _ = writeln!(self.0, "CHECK {name} {verdict} {detail}");
        }
    }

    fn main() -> ! {
        let Some(bus) = rt::take_ffuart() else { halt() };
        let mut uart = PxaUart::new(bus);
        uart.init(115_200);
        // Benchmark the configuration RetroPane really runs: MMU and caches on.
        rt::set_fault_handler(on_fault);
        rt::init_mmu();
        let _ = writeln!(uart, "\nRetroPane benchmarks (OS timer {} Hz)", rt::TICK_HZ);
        let mut out = Serial(uart);
        let mut r = suite::Runner::new(&mut out);

        suite::timer(&mut r);
        suite::frames(&mut r);
        if let Some(scratch) = rp_hal_x51v::devices::take_scratch() {
            suite::memory(&mut r, scratch);
            suite::lz4(&mut r, &mut scratch[256 * 1024..]);
            suite::chase(&mut r, "chase_sdram_256k", &mut scratch[..256 * 1024]);
        }
        if let Some(sram) = diag::take_sram() {
            suite::chase(&mut r, "chase_sram_256k", sram);
        }
        suite::tlb(&mut r, "tlb_fit_16", "tlb_spill_60");
        if let Some((_regs, vram)) = rp_hal_x51v::devices::take_mbx() {
            let plane = Plane::fullscreen(&X51V_VGA, PixelFormat::Rgb565);
            let end = plane.offset + plane.stride * usize::from(plane.height);
            suite::framebuffer(&mut r, &mut vram[plane.offset..end], &plane);
        }
        if let (Some(cpld), Some(mmc)) = (rp_hal_x51v::devices::take_cpld(), rt::take_mmc()) {
            Cpld::new(cpld).set_sd_power(true);
            rt::delay_us(10_000);
            match SdCard::init(PxaMmc::new(mmc)) {
                Ok(mut card) => suite::sd(&mut r, &mut card),
                Err(e) => r.check("sd", false, e.name()),
            }
        }

        let _ = writeln!(out.0, "{}", super::DONE);

        // Fault-path self-test: 0x80000000 is unmapped in the bring-up map, so
        // this read must end in a reported data abort, not silence.
        let _ = black_box(rt::fault_probe(black_box(0x8000_0000)));
        let _ = writeln!(out.0, "FAULT-PROBE no fault (unexpected)");
        halt()
    }

    fn on_fault(kind: rt::Fault, addr: u32) -> ! {
        let mut uart = PxaUart::new(rt::emergency_uart());
        let _ = writeln!(uart, "FAULT {} at {addr:#010x}", kind.name());
        halt()
    }

    fn halt() -> ! {
        loop {
            core::hint::spin_loop();
        }
    }

    #[panic_handler]
    fn panic(_: &core::panic::PanicInfo) -> ! {
        halt()
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}
