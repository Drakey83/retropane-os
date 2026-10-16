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
        let mut sd_buf: &mut [u8] = &mut [];
        let mut gfx_work: &mut [u8] = &mut [];
        let mut layer_work: &mut [u8] = &mut [];
        if let Some(scratch) = rp_hal_x51v::devices::take_scratch() {
            suite::memory(&mut r, scratch);
            suite::lz4(&mut r, &mut scratch[256 * 1024..]);
            suite::chase(&mut r, "chase_sdram_256k", &mut scratch[..256 * 1024]);
            // The first MiB less 128 KiB for the pixel paths, those 128
            // KiB for SD (256-block reads; the suites above stay below
            // them), the rest for the interface layer.
            let (work, tail) = scratch.split_at_mut(1024 * 1024 - 128 * 1024);
            let (sd, rest) = tail.split_at_mut(128 * 1024);
            gfx_work = work;
            sd_buf = sd;
            layer_work = rest;
        }
        if let Some(sram) = diag::take_sram() {
            suite::chase(&mut r, "chase_sram_256k", sram);
        }
        suite::tlb(&mut r, "tlb_fit_16", "tlb_spill_60");
        suite::crypto(&mut r, sd_buf);
        if let Some((_regs, vram)) = rp_hal_x51v::devices::take_mbx() {
            let plane = Plane::fullscreen(&X51V_VGA, PixelFormat::Rgb565);
            let end = plane.offset + plane.stride * usize::from(plane.height);
            suite::gfx(
                &mut r,
                gfx_work,
                Some((&mut vram[plane.offset..end], &plane)),
            );
            suite::framebuffer(&mut r, &mut vram[plane.offset..end], &plane);
            // An off-screen graphics plane from 1 MiB up (the screen is at 0).
            suite::layer(&mut r, layer_work, vram.get_mut(0x10_0000..0x24_0000));
        } else {
            suite::gfx(&mut r, gfx_work, None);
            suite::layer(&mut r, layer_work, None);
        }
        if let (Some(cpld), Some(mmc), Some(dma)) = (
            rp_hal_x51v::devices::take_cpld(),
            rt::take_mmc(),
            rt::take_dma(rp_hal_pxa27x::dma_map::SD),
        ) {
            Cpld::new(cpld).set_sd_power(true);
            rt::delay_us(10_000);
            match SdCard::init(PxaMmc::with_dma(mmc, dma)) {
                Ok(mut card) => {
                    suite::sd(&mut r, &mut card, sd_buf);
                    sd_dma_past_chain(&mut r, &mut card, layer_work);
                }
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

    /// A 1 MiB read, past the 128 KiB the DMA chain once held (it fell
    /// back to the FIFO above that: docs/research/storage-speed.md), must
    /// go by DMA in one transfer and read what the FIFO reads.
    fn sd_dma_past_chain(
        r: &mut suite::Runner,
        card: &mut SdCard<PxaMmc<Mmio, rp_hal_pxa27x::dma::Dma>>,
        work: &mut [u8],
    ) {
        const LEN: usize = 1024 * 1024;
        let Some((dma, rest)) = work.get_mut(..2 * LEN).map(|w| w.split_at_mut(LEN)) else {
            r.check("sd_dma_past_chain", false, "no buffer");
            return;
        };
        dma.fill(0xA5);
        let before = card.host().dma_transfers();
        let by_dma = card.read_many(0, dma).is_ok();
        let transfers = card.host().dma_transfers() - before;
        card.host_mut().set_dma(false, false);
        let by_fifo = card.read_many(0, rest).is_ok();
        card.host_mut().set_dma(true, true);
        let ok = by_dma && by_fifo && transfers == 1 && *dma == *rest;
        r.check(
            "sd_dma_past_chain",
            ok,
            if ok {
                ""
            } else {
                "1 MiB by DMA: not one transfer, or differs from the FIFO"
            },
        );
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
