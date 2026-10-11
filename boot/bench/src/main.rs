//! RetroPane benchmark image.
//!
//! Runs micro-benchmarks timed with the PXA27x OS timer and prints one line
//! per result: `BENCH <name> <ticks> <iterations>`, then `BENCH-DONE`.
//! `cargo xtask bench` boots it in the emulator in deterministic `-icount`
//! mode and compares the results with the committed baseline; on the real X51v
//! the same image gives real-hardware numbers.
//!
//! Benchmarks come in comparison pairs where possible (library vs. naive,
//! current code vs. the hardware's ceiling) so they point at what to improve.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed after the last result.
pub const DONE: &str = "BENCH-DONE";

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;
    use core::hint::black_box;

    use rp_drv_cpld::Cpld;
    use rp_drv_mbx_display::{PixelFormat, Plane, X51V_VGA, fill_rect_rgb565, pattern};
    use rp_drv_pxa_mmc::PxaMmc;
    use rp_drv_pxa_uart::PxaUart;
    use rp_drv_sdcard::{BLOCK, SdCard};
    use rp_hal_pxa27x::{Mmio, rt};

    rp_hal_pxa27x::entry!(main);

    struct Bench {
        uart: PxaUart<Mmio>,
    }

    impl Bench {
        /// Times `iters` runs of `f` and prints the result.
        ///
        /// `f` is called through `dyn` and this function is never inlined:
        /// with a generic closure and a constant `iters`, LLVM unrolled whole
        /// benchmarks (64 iterations x 512 FIFO reads = ~33k instructions,
        /// 180 KB), which both bloats the image and measures code nothing
        /// else would run (ADR-0005 rule 8).
        #[inline(never)]
        fn run(&mut self, name: &str, iters: u32, f: &mut dyn FnMut()) {
            let iters = black_box(iters);
            f(); // warm caches and branch predictors
            let start = rt::ticks();
            for _ in 0..iters {
                f();
            }
            let ticks = rt::ticks().wrapping_sub(start);
            let _ = writeln!(self.uart, "BENCH {name} {ticks} {iters}");
        }
    }

    /// Checks the memcpy family on the real CPU for every size 0..=300 and
    /// every alignment, including overlapping moves and comparisons.
    fn check_mem(buf: &mut [u8]) -> Result<(), (&'static str, usize, usize, usize)> {
        let (src, rest) = buf.split_at_mut(512);
        let dst = &mut rest[..512];
        for (i, v) in src.iter_mut().enumerate() {
            *v = (i * 7 + 3) as u8;
        }
        for n in 0..=300 {
            for so in 0..4 {
                for d in 0..4 {
                    dst.fill(0xEE);
                    dst[d..d + n].copy_from_slice(&src[so..so + n]);
                    let ok = (0..n).all(|i| black_box(dst[d + i]) == src[so + i])
                        && dst[..d].iter().chain(&dst[d + n..]).all(|&b| b == 0xEE);
                    if !ok {
                        return Err(("memcpy", n, so, d));
                    }
                    if dst[d..d + n] != src[so..so + n] {
                        return Err(("memcmp", n, so, d));
                    }
                }
            }
            for d in 0..4 {
                dst.fill(0x11);
                dst[d..d + n].fill(0xA5);
                if !(dst[d..d + n].iter().all(|&b| b == 0xA5)
                    && dst[..d].iter().chain(&dst[d + n..]).all(|&b| b == 0x11))
                {
                    return Err(("memset", n, 0, d));
                }
            }
        }
        for n in 0..=96 {
            for from in 0..8 {
                for to in 0..8 {
                    dst[..200].copy_from_slice(&src[..200]);
                    dst.copy_within(from..from + n, to);
                    let ok = (0..n).all(|i| dst[to + i] == src[from + i]);
                    if !ok {
                        return Err(("memmove", n, from, to));
                    }
                }
            }
        }
        Ok(())
    }

    fn naive_copy(dst: &mut [u8], src: &[u8]) {
        for (d, s) in dst.iter_mut().zip(src) {
            *d = black_box(*s);
        }
    }

    fn main() -> ! {
        let Some(bus) = rt::take_ffuart() else { halt() };
        let mut uart = PxaUart::new(bus);
        uart.init(115_200);
        let _ = writeln!(uart, "\nRetroPane benchmarks (OS timer {} Hz)", rt::TICK_HZ);
        let mut b = Bench { uart };

        b.run("timer_read", 1000, &mut || {
            black_box(rt::ticks());
        });

        if let Some(scratch) = rp_hal_x51v::devices::take_scratch() {
            match check_mem(&mut scratch[..1024]) {
                Ok(()) => {
                    let _ = writeln!(b.uart, "CHECK mem OK");
                }
                Err((what, n, x, y)) => {
                    let _ = writeln!(b.uart, "CHECK mem FAIL {what} n={n} {x} {y}");
                }
            }
            let (a, rest) = scratch.split_at_mut(64 * 1024);
            let bufb = &mut rest[..64 * 1024];
            for (i, v) in a.iter_mut().enumerate() {
                *v = i as u8;
            }
            b.run("memcpy_64k", 20, &mut || {
                bufb.copy_from_slice(a);
                black_box(&bufb);
            });
            b.run("memcpy_naive_64k", 20, &mut || naive_copy(bufb, a));
            b.run("memset_64k", 20, &mut || {
                bufb.fill(black_box(0x5A));
                black_box(&bufb);
            });
        }

        if let Some((_regs, vram)) = rp_hal_x51v::devices::take_mbx() {
            let plane = Plane::fullscreen(&X51V_VGA, PixelFormat::Rgb565);
            let fb =
                &mut vram[plane.offset..plane.offset + plane.stride * usize::from(plane.height)];
            b.run("fb_pattern_vga", 3, &mut || {
                pattern::fill_rgb565(fb, plane.width.into(), plane.height.into(), plane.stride);
            });
            b.run("fb_solid_fill_vga", 3, &mut || {
                fill_rect_rgb565(
                    fb,
                    plane.stride,
                    0,
                    0,
                    plane.width.into(),
                    plane.height.into(),
                    black_box(0x001F),
                );
            });
        }

        if let (Some(cpld), Some(mmc)) = (rp_hal_x51v::devices::take_cpld(), rt::take_mmc()) {
            Cpld::new(cpld).set_sd_power(true);
            rt::delay_us(10_000);
            match SdCard::init(PxaMmc::new(mmc)) {
                Ok(mut card) => {
                    let mut block = [0u8; BLOCK];
                    let mut lba = 0;
                    b.run("sd_read_block", 64, &mut || {
                        let _ = card.read(lba, &mut block);
                        lba = (lba + 1) % 64;
                    });
                }
                Err(e) => {
                    let _ = writeln!(b.uart, "SD unavailable: {e:?}");
                }
            }
        }

        let _ = writeln!(b.uart, "{}", super::DONE);
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
