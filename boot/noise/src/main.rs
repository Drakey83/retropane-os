//! Test image (ADR-0029, "How it will be measured", step 3): measures the
//! noise sources the device's random number generator would draw on, under
//! the loader. Reads only, apart from its result files on the card.
//!
//! 1. **Clocks**: `OSCC` (is the 32.768 kHz oscillator its own crystal,
//!    on and stable?), `CCCR`/`CCSR`/`CLKCFG`, and the core clock measured
//!    with the cycle counter against the 3.25 MHz OS timer.
//! 2. **A, two-crystal jitter**: 1,000,000 samples of the core cycles
//!    between one 32.768 kHz tick and the next (an OS timer channel on the
//!    slow oscillator, the XScale's `CCNT`; how to start a channel on the
//!    32 kHz clock is found first, each way tried for 2 ms and logged).
//!    Its first 1,000 low bytes are appended to `noise-r.bin` as one row of SP 800-90B's restart test
//!    (each run one row, up to 1,000).
//! 3. **B, one-clock jitter**: 1,000,000 timings (cycles) of 64 reads that
//!    all miss the data cache (one cache set, 64 lines for 32 ways), so
//!    each goes to SDRAM.
//! 4. **T, the TSC2046's single-ended inputs**: 1,000,000 12-bit readings
//!    each of `TEMP0`, `VBAT`, `AUX`, `TEMP1`.
//!
//! Each source's raw samples go to the card as little-endian 16-bit values
//! (`noise-a.bin`, `noise-b.bin`, `noise-t0.bin`, `noise-vb.bin`,
//! `noise-ax.bin`, `noise-t1.bin`), for NIST's `ea_non_iid` on the PC; the
//! run log and `noise.txt` carry a first look: the range, the distinct
//! values and the 90B most-common-value min-entropy estimate (one of the
//! tool's ten; the tool decides, this only says whether a source is
//! obviously dead). Interrupts stay off throughout (as the kernel's boot
//! collector would sample, before anything else runs).

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed when the run is done (serial and run log).
pub const MARKER: &str = "RETROPANE-NOISE: done";
/// Samples per source (SP 800-90B asks for at least 1,000,000).
pub const SAMPLES: usize = 1_000_000;
/// Spaced series: ticks between samples (ADR-0029 option 2: phase noise
/// adds up over time).
pub const SPACING: u32 = 16;
/// Samples in the spaced series (~49 s at 16 ticks a sample).
pub const SPACED_SAMPLES: usize = 100_000;
/// Samples of source A per restart-test row.
pub const RESTART_ROW: usize = 1_000;
/// Rows the restart file holds at most (90B: 1,000 restarts).
pub const RESTART_ROWS: usize = 1_000;

/// The first look at a source's samples (SP 800-90B §6.3.1, the most
/// common value estimate), in plain integers: the image has no FPU and no
/// `libm`.
pub mod stats {
    /// What the run log says about one source.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct Summary {
        /// Samples seen.
        pub n: u32,
        /// Smallest and largest sample (whole values).
        pub min: u16,
        /// See `min`.
        pub max: u16,
        /// Distinct values after the mask.
        pub distinct: u32,
        /// How often the most common masked value occurred.
        pub mcv: u32,
        /// The MCV min-entropy estimate, thousandths of a bit per sample.
        pub h_milli: u32,
    }

    /// Summarises `samples` with each value masked by `mask` (`0xFF`: the
    /// low byte, `0xFFF`: a 12-bit reading), counting in `hist`, which must
    /// hold `mask + 1` entries.
    #[must_use]
    pub fn summarise(samples: &[u16], mask: u16, hist: &mut [u32]) -> Summary {
        let bins = usize::from(mask) + 1;
        let hist = &mut hist[..bins];
        hist.fill(0);
        let (mut min, mut max) = (u16::MAX, 0);
        for &s in samples {
            hist[usize::from(s & mask)] += 1;
            min = min.min(s);
            max = max.max(s);
        }
        let n = samples.len() as u32;
        let mcv = hist.iter().copied().max().unwrap_or(0);
        Summary {
            n,
            min: if n == 0 { 0 } else { min },
            max,
            distinct: hist.iter().filter(|&&c| c != 0).count() as u32,
            mcv,
            h_milli: mcv_min_entropy_milli(mcv, n),
        }
    }

    /// SP 800-90B §6.3.1: with `count` of `n` samples the most common value,
    /// `p_u = min(1, p + 2.576 sqrt(p (1 - p) / (n - 1)))` and the estimate
    /// is `-log2(p_u)` bits a sample; returned in thousandths of a bit.
    #[must_use]
    pub fn mcv_min_entropy_milli(count: u32, n: u32) -> u32 {
        if n < 2 || count == 0 {
            return 0;
        }
        const ONE: u128 = 1 << 32;
        // p in 32.32 fixed point.
        let p = u128::from(count) * ONE / u128::from(n);
        // p (1 - p) / (n - 1) in 2^64 units; its square root in 2^32 units.
        let var = p * (ONE - p) / u128::from(n - 1);
        let bound = isqrt(var) * 2576 / 1000;
        let pu = (p + bound).min(ONE);
        // -log2(pu / 2^32) = 32 - log2(pu), log2 in 2^20 units.
        let bits = (32u64 << 20).saturating_sub(log2_fixed20(pu as u64));
        ((bits * 1000 + (1 << 19)) >> 20) as u32
    }

    /// `floor(sqrt(x))`.
    fn isqrt(x: u128) -> u128 {
        if x < 2 {
            return x;
        }
        // Newton's method from a power of two at or above the root.
        let mut r = 1u128 << ((128 - x.leading_zeros()).div_ceil(2));
        loop {
            let next = (r + x / r) / 2;
            if next >= r {
                return r;
            }
            r = next;
        }
    }

    /// `log2(x)` for `x >= 1`, in 2^20 units (20 fraction bits, by repeated
    /// squaring of the mantissa).
    fn log2_fixed20(x: u64) -> u64 {
        if x == 0 {
            return 0;
        }
        let int = 63 - u64::from(x.leading_zeros());
        // The mantissa x / 2^int in [1, 2), 2.62 fixed point (u128 room).
        let mut m = u128::from(x) << (62 - int);
        let mut frac = 0u64;
        for bit in (0..20).rev() {
            m = (m * m) >> 62;
            if m >= 2 << 62 {
                m >>= 1;
                frac |= 1 << bit;
            }
        }
        int << 20 | frac
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn reference(count: u32, n: u32) -> f64 {
            let p = f64::from(count) / f64::from(n);
            let pu = (p + 2.576 * (p * (1.0 - p) / f64::from(n - 1)).sqrt()).min(1.0);
            -pu.log2() * 1000.0
        }

        #[test]
        fn the_estimate_matches_floating_point() {
            for (count, n) in [
                (1_000_000, 1_000_000),
                (500_000, 1_000_000),
                (3_906, 1_000_000),
                (4_000, 1_000_000),
                (250, 1_000),
                (1, 1_000_000),
                (999_999, 1_000_000),
                (1, 2),
            ] {
                let got = f64::from(mcv_min_entropy_milli(count, n));
                let want = reference(count, n);
                assert!((got - want).abs() <= 2.0, "{count}/{n}: {got} vs {want}");
            }
        }

        #[test]
        fn a_dead_source_has_no_entropy_and_a_flat_one_nearly_eight_bits() {
            let mut hist = vec![0u32; 4096];
            let stuck = vec![0x1234u16; 10_000];
            let s = summarise(&stuck, 0xFF, &mut hist);
            assert_eq!((s.distinct, s.mcv, s.h_milli), (1, 10_000, 0));
            assert_eq!((s.min, s.max), (0x1234, 0x1234));
            let flat: Vec<u16> = (0..256_000u32).map(|i| (i % 256) as u16 | 0x4000).collect();
            let s = summarise(&flat, 0xFF, &mut hist);
            assert_eq!(s.distinct, 256);
            assert!((7_800..8_000).contains(&s.h_milli), "{}", s.h_milli);
            // Twelve-bit readings use the whole value.
            let s = summarise(&[1, 2, 4095, 4095], 0xFFF, &mut hist);
            assert_eq!((s.distinct, s.mcv, s.min, s.max), (3, 2, 1, 4095));
        }

        #[test]
        fn nothing_in_is_nothing_out() {
            let mut hist = vec![0u32; 256];
            assert_eq!(summarise(&[], 0xFF, &mut hist), Summary::default());
            assert_eq!(mcv_min_entropy_milli(0, 0), 0);
        }
    }
}

#[cfg(target_os = "none")]
mod image {
    use core::cell::RefCell;
    use core::fmt::Write;

    use rp_drv_cpld::Cpld;
    use rp_drv_pxa_gpio::{Gpio, GpioPin};
    use rp_drv_pxa_mmc::PxaMmc;
    use rp_drv_pxa_ssp::Ssp;
    use rp_drv_pxa_uart::PxaUart;
    use rp_drv_sdcard::{SdCard, mbr_partitions};
    use rp_drv_tsc2046::{Tsc2046, channel, single};
    use rp_fat::AnyVolume;
    use rp_hal_api::{BlockDevice, BlockError};
    use rp_hal_pxa27x::dma::Dma;
    use rp_hal_pxa27x::rt::IrqLine;
    use rp_hal_pxa27x::soc::{OstChannels, irq};
    use rp_hal_pxa27x::{Mmio, diag, rt};
    use rp_hal_x51v::devices::{self, RunLog};

    use super::stats::{self, Summary};
    use super::{MARKER, RESTART_ROW, RESTART_ROWS, SAMPLES, SPACED_SAMPLES, SPACING};

    rp_hal_pxa27x::entry!(main);

    type Sd = SdCard<PxaMmc<Mmio, Dma>>;

    const TS_CS: u8 = 24;
    const TS_PENIRQ: u8 = 94;
    /// Clocks unit registers (PXA27x Developer's Manual, clocks chapter).
    const CCCR: usize = 0x4130_0000;
    const OSCC: usize = 0x4130_0008;
    const CCSR: usize = 0x4130_000C;
    /// Ways to run an OS timer channel on the 32.768 kHz oscillator, tried in
    /// order: (channel, `OMCRx`, counter written before the mode). `OMCRx`
    /// bits 2-0 the resolution (1: 1/32768 s, 2: 1 ms, 4: 1 us), bit 7 its
    /// own counter, bits 6 and 3 as the manual names them (periodic, reset
    /// on match).
    const TIMER_TRIES: &[(u8, u32, bool)] = &[
        (5, 0x81, false),
        (5, 0x81, true),
        (5, 0x01, false),
        (5, 0xC1, false),
        (5, 0x82, false),
        (5, 0x84, false),
        (6, 0x81, false),
        (8, 0x81, false),
        (11, 0x81, false),
    ];
    /// Check in with the loader's deadlines every this many samples.
    const CHECK_IN: usize = 4096;

    /// The scratch region's layout (4 MiB, `devices::take_scratch`).
    const SAMPLE_BYTES: usize = SAMPLES * 2;
    const HIST_AT: usize = SAMPLE_BYTES;
    const FAT_AT: usize = HIST_AT + 16 * 1024;
    const WALK_AT: usize = FAT_AT + 64 * 1024;
    const WALK_BYTES: usize = 64 * 1024;
    const RESTART_AT: usize = WALK_AT + WALK_BYTES;
    const RESTART_BYTES: usize = RESTART_ROW * RESTART_ROWS;
    /// The summary for `noise.txt` (in scratch, not on the 16 KiB stack:
    /// run 1 used 13.5 KiB of it with the summary there).
    const TEXT_AT: usize = RESTART_AT + RESTART_BYTES;
    const TEXT_BYTES: usize = 4096;
    const _: () = assert!(TEXT_AT + TEXT_BYTES <= devices::SCRATCH_SIZE);

    /// The summary for `noise.txt`, written alongside the run log.
    struct Text<'a> {
        buf: &'a mut [u8],
        len: usize,
    }

    impl Write for Text<'_> {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let n = s.len().min(self.buf.len() - self.len);
            self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
            self.len += n;
            RunLog.write_str(s)
        }
    }

    /// The card, keeping the loader's deadlines while it works (a 2 MB file
    /// is thousands of blocks).
    struct Fed<'a>(&'a mut Sd);

    impl BlockDevice for Fed<'_> {
        fn read_blocks(&mut self, lba: u32, buf: &mut [u8]) -> Result<(), BlockError> {
            rt::check_in();
            self.0.read_blocks(lba, buf)
        }
        fn write_blocks(&mut self, lba: u32, buf: &[u8]) -> Result<(), BlockError> {
            rt::check_in();
            self.0.write_blocks(lba, buf)
        }
    }

    fn main() -> ! {
        rt::set_fault_handler(on_fault);
        rt::init_mmu();
        let Some(scratch) = devices::take_scratch() else {
            crash(format_args!("no scratch memory"))
        };
        let (scratch, text) = scratch.split_at_mut(TEXT_AT);
        let mut out = Text {
            buf: &mut text[..TEXT_BYTES],
            len: 0,
        };
        let _ = writeln!(
            out,
            "rp-noise (ADR-0029): noise sources, raw samples to the card"
        );

        // 1. Clocks.
        let reg = |a: usize| rt::try_read(a).unwrap_or(0xDEAD_DEAD);
        let (oscc, cccr, ccsr) = (reg(OSCC), reg(CCCR), reg(CCSR));
        let clkcfg = diag::coprocessors().clkcfg;
        // OSCC bit 0 OOK (oscillator stable), bit 1 OON (oscillator on).
        let own = if oscc & 3 == 3 {
            "on and stable: the 32 kHz clock is its own crystal"
        } else {
            "NOT both on and stable: the 32 kHz clock may come from the 13 MHz crystal"
        };
        let _ = writeln!(
            out,
            "clocks: OSCC {oscc:#010x} (32.768 kHz oscillator {own})"
        );
        let _ = writeln!(
            out,
            "clocks: CCCR {cccr:#010x} CCSR {ccsr:#010x} CLKCFG {clkcfg:#x}"
        );
        rt::cycle_counter_start();
        let (c0, t0) = (rt::cycles(), rt::ticks());
        rt::delay_us(100_000);
        let (c1, t1) = (rt::cycles(), rt::ticks());
        let ticks = u64::from(t1.wrapping_sub(t0)).max(1);
        let khz = u64::from(c1.wrapping_sub(c0)) * u64::from(rt::TICK_HZ / 1000) / ticks;
        let _ = writeln!(
            out,
            "clocks: core measured {}.{:03} MHz (cycle counter over the 3.25 MHz timer)",
            khz / 1000,
            khz % 1000
        );

        // The card, before any sampling, so a missing card is known early.
        let mut card = open_card();
        let _ = writeln!(
            out,
            "card: {}",
            if card.is_some() {
                "ready"
            } else {
                "NOT AVAILABLE: summaries only"
            }
        );

        let (samples, rest) = scratch.split_at_mut(SAMPLE_BYTES);
        let samples = rp_hal_mem::bytes_as_pixels_mut(samples)
            .unwrap_or_else(|| crash(format_args!("scratch not aligned")));
        let (hist_bytes, rest) = rest.split_at_mut(FAT_AT - HIST_AT);
        let hist = rp_hal_mem::bytes_as_words_mut(hist_bytes)
            .unwrap_or_else(|| crash(format_args!("scratch not aligned")));
        let (fat, rest) = rest.split_at_mut(WALK_AT - FAT_AT);
        let (walk, rest) = rest.split_at_mut(WALK_BYTES);
        let restart = rest;

        // 2. A: two-crystal jitter. First find how a channel counts the
        // 32 kHz oscillator on the X51v: runs 1 and 2 set channel 5 as the
        // manual reads (`OMCR5 = 0x81`, counter before or after) and it
        // never moved. Each way is tried for 2 ms; the first that counts at
        // 1/32768 s is used.
        let Some(mut ost) = rt::take_ost_channels() else {
            crash(format_args!("no OS timer channels"))
        };
        let mut found = None;
        for &(ch, mode, count_first) in TIMER_TRIES {
            ost.set_mode(ch, 0);
            if count_first {
                ost.set_count(ch, 0);
                ost.set_mode(ch, mode);
            } else {
                ost.set_mode(ch, mode);
                ost.set_count(ch, 0);
            }
            let read = ost.mode(ch);
            let c0 = ost.count(ch);
            rt::delay_us(2_000);
            let c1 = ost.count(ch);
            ost.set_mode(ch, 0);
            let hz = c1.wrapping_sub(c0).saturating_mul(500);
            let _ = writeln!(
                out,
                "timer: channel {ch} OMCR {mode:#04x} {}: OMCR reads {read:#x}, count {c0} -> {c1} in 2 ms (~{hz} Hz)",
                if count_first {
                    "counter first"
                } else {
                    "mode first"
                },
            );
            // 32,768 Hz is ~65 counts in 2 ms.
            if found.is_none() && mode & 7 == 1 && (40..=100).contains(&c1.wrapping_sub(c0)) {
                found = Some((ch, mode, count_first));
            }
        }
        ost.stop_all();
        if let Some((ch, mode, count_first)) = found {
            if count_first {
                ost.set_count(ch, 0);
                ost.set_mode(ch, mode);
            } else {
                ost.set_mode(ch, mode);
                ost.set_count(ch, 0);
            }
            let _ = writeln!(out, "timer: sampling with channel {ch}, OMCR {mode:#04x}");
            // Run 3 polled the counter: every sample landed on one of two
            // values 504 cycles apart (one register read), a near-regular
            // beat. Now the tick's interrupt is timestamped as the IRQ path
            // is entered (`rt::last_irq_cycles`). The channels share one
            // interrupt line with the run limit (FIQ), so the run limit is
            // handed off for this part; every wait below is bounded, and
            // the hardware watchdog stays armed (`check_in`).
            let limit = rt::run_limit_ms();
            rt::disarm_run_limit();
            let Some(mut line) = rt::take_irq(irq::OST_4_11) else {
                crash(format_args!("OS timer interrupt taken"))
            };
            let tries: [(u32, usize, &str, &str); 2] = [
                (
                    1,
                    SAMPLES,
                    "A interrupt (cycles between 32 kHz tick interrupts)",
                    "noise-ai.bin",
                ),
                (
                    SPACING,
                    SPACED_SAMPLES,
                    "A spaced (cycles across 16 tick interrupts, low 16 bits)",
                    "noise-as.bin",
                ),
            ];
            for (k, n, name, file) in tries {
                let series = &mut samples[..n];
                let start = rt::ticks();
                let missed = sample_ticks(&mut ost, &mut line, ch, k, series);
                let took = rt::ticks().wrapping_sub(start);
                rt::irq_disable();
                ost.disarm_match(ch);
                let sum = stats::summarise(series, 0xFF, hist);
                report(&mut out, name, &sum, took, "low byte");
                let _ = writeln!(out, "  ticks missed and resynchronised: {missed}");
                save(&mut out, card.as_mut(), file, series, fat);
                if k == 1 {
                    save_restart_row(&mut out, card.as_mut(), series, restart, fat);
                }
            }
            if let Some(ms) = limit {
                rt::arm_run_limit(ms);
            }
        } else {
            let _ = writeln!(
                out,
                "A two-crystal: no channel counted at 1/32768 s: skipped"
            );
        }
        ost.stop_all();

        // 3. B: one-clock jitter, 64 reads that all miss the cache. The
        // D-cache is 32 KiB, 32 ways of 32-byte lines: 32 sets, so a 1 KiB
        // stride lands every read in one set, and 64 lines evict each other.
        let start = rt::ticks();
        let mut sink = 0u32;
        for (i, s) in samples.iter_mut().enumerate() {
            let c0 = rt::cycles();
            for k in 0..64 {
                sink = sink.wrapping_add(rp_hal_mem::load32_volatile(walk, k * 1024).unwrap_or(0));
            }
            *s = rt::cycles().wrapping_sub(c0) as u16;
            if i % CHECK_IN == 0 {
                rt::check_in();
            }
        }
        let took = rt::ticks().wrapping_sub(start);
        let sum = stats::summarise(samples, 0xFF, hist);
        report(
            &mut out,
            "B one-clock (cycles for 64 SDRAM reads)",
            &sum,
            took,
            "low byte",
        );
        let _ = writeln!(RunLog, "  (sink {sink:#x})");
        save(&mut out, card.as_mut(), "noise-b.bin", samples, fat);

        // 4. T: the TSC2046's single-ended inputs.
        let (Some(gpio), Some(ssp)) = (rt::take_gpio(), rt::take_ssp1()) else {
            crash(format_args!("no GPIO or SSP"))
        };
        let gpio = RefCell::new(Gpio::new(gpio));
        // As rp-today: Windows Mobile leaves GPIO24 as SSP1's frame signal
        // (docs/hw/touch.md); level and direction first, then a plain GPIO.
        gpio.borrow_mut().make_output(TS_CS, true);
        gpio.borrow_mut().set_function(TS_CS, 0);
        gpio.borrow_mut().make_input(TS_PENIRQ);
        let mut ssp = Ssp::new(ssp);
        ssp.configure_spi(2_600_000);
        let mut touch = Tsc2046::new(
            ssp,
            GpioPin::new(&gpio, TS_CS),
            GpioPin::new(&gpio, TS_PENIRQ),
        );
        for (ch, name, file) in [
            (single::TEMP0, "T TEMP0", "noise-t0.bin"),
            (single::VBAT, "T VBAT", "noise-vb.bin"),
            (single::AUX, "T AUX", "noise-ax.bin"),
            (single::TEMP1, "T TEMP1", "noise-t1.bin"),
        ] {
            // Let the reference and the input settle first.
            for _ in 0..64 {
                touch.read_single(ch);
            }
            let start = rt::ticks();
            for (i, s) in samples.iter_mut().enumerate() {
                *s = touch.read_single(ch);
                if i % CHECK_IN == 0 {
                    rt::check_in();
                }
            }
            let took = rt::ticks().wrapping_sub(start);
            let sum = stats::summarise(samples, 0xFFF, hist);
            report(&mut out, name, &sum, took, "12-bit value");
            save(&mut out, card.as_mut(), file, samples, fat);
        }
        // A last differential conversion powers the converter down and
        // re-arms the pen interrupt, as the touch driver leaves it.
        touch.read(channel::Y);

        if let Some(sd) = card.as_mut() {
            let text = &out.buf[..out.len];
            let ok = mount(sd).and_then(|mut v| v.put("noise.txt", text, fat).map_err(|_| "write"));
            let _ = writeln!(RunLog, "noise.txt: {}", ok.map_or_else(|e| e, |_| "saved"));
        }
        if let Some(uart) = rt::take_ffuart() {
            let mut uart = PxaUart::new(uart);
            uart.init(115_200);
            let _ = writeln!(uart, "{MARKER}");
        }
        let _ = writeln!(RunLog, "{MARKER}");
        devices::return_to_loader();
        loop {
            core::hint::spin_loop();
        }
    }

    /// Fills `out` with the core cycles between every `k`th 32 kHz tick of
    /// channel `ch`, each tick taken as an interrupt and timestamped on
    /// entry. The wait spins on memory only (no peripheral reads, whose
    /// ~500-cycle bus time would put run 3's beat back into the timing),
    /// bounded by the cycle counter. A tick missed (a match already past)
    /// is waited out, resynchronised and not counted; returns how many.
    fn sample_ticks(
        ost: &mut OstChannels<Mmio>,
        line: &mut IrqLine,
        ch: u8,
        k: u32,
        out: &mut [u16],
    ) -> u32 {
        // 10 ms at 624 MHz: hundreds of ticks.
        const TIMEOUT: u32 = 6_240_000;
        let wait = |line: &mut IrqLine| {
            let t0 = rt::cycles();
            while !line.take() {
                if rt::cycles().wrapping_sub(t0) > TIMEOUT {
                    return false;
                }
            }
            true
        };
        let mut missed = 0u32;
        let mut at = 0u32;
        // Start (and restart after a miss) two ticks ahead; the first
        // interrupt only sets the reference.
        let sync = |ost: &mut OstChannels<Mmio>, line: &mut IrqLine, at: &mut u32| -> Option<u32> {
            for _ in 0..4 {
                rt::irq_disable();
                *at = ost.count(ch).wrapping_add(2);
                ost.arm_match(ch, *at);
                line.done();
                rt::irq_enable();
                if wait(line) {
                    return Some(rt::last_irq_cycles());
                }
            }
            None
        };
        let Some(mut prev) = sync(ost, line, &mut at) else {
            return u32::MAX;
        };
        let mut i = 0;
        while i < out.len() {
            rt::irq_disable();
            at = at.wrapping_add(k);
            ost.arm_match(ch, at);
            line.done();
            rt::irq_enable();
            if i % CHECK_IN == 0 {
                rt::check_in();
            }
            if !wait(line) {
                missed += 1;
                if missed > 1000 {
                    break;
                }
                match sync(ost, line, &mut at) {
                    Some(c) => prev = c,
                    None => break,
                }
                continue;
            }
            let c = rt::last_irq_cycles();
            out[i] = c.wrapping_sub(prev) as u16;
            prev = c;
            i += 1;
        }
        missed
    }

    /// One source's line: what it is, how long it took, its first look.
    fn report(out: &mut Text<'_>, name: &str, s: &Summary, ticks: u32, over: &str) {
        let ms = u64::from(ticks) * 1000 / u64::from(rt::TICK_HZ);
        let _ = writeln!(
            out,
            "{name}: {} samples in {}.{:03} s, values {}..{}; {over}: {} distinct, most common {} times, MCV min-entropy {}.{:03} bits/sample{}",
            s.n,
            ms / 1000,
            ms % 1000,
            s.min,
            s.max,
            s.distinct,
            s.mcv,
            s.h_milli / 1000,
            s.h_milli % 1000,
            if s.distinct <= 1 {
                " - DEAD (one value)"
            } else {
                ""
            }
        );
    }

    /// The SD card, as the loader starts it (power through the CPLD, then
    /// the card); `None` if there is none.
    fn open_card() -> Option<Sd> {
        let mut cpld = Cpld::new(devices::take_cpld()?);
        let (mmc, dma) = (rt::take_mmc()?, rt::take_dma(0)?);
        // The CPLD doesn't read back: as the loader, the backlight's enable
        // (sharing a register with the CF reset) set again with SD power.
        cpld.set_backlight_enable(true);
        cpld.set_sd_power(true);
        rt::delay_us(10_000);
        SdCard::try_init(PxaMmc::with_dma(mmc, dma)).ok()
    }

    fn mount(sd: &mut Sd) -> Result<AnyVolume<Fed<'_>>, &'static str> {
        let mut block = [0u8; rp_drv_sdcard::BLOCK];
        sd.read(0, &mut block).map_err(|_| "card read")?;
        let start = mbr_partitions(&block)
            .and_then(|p| p[0])
            .map_or(0, |p| p.start);
        AnyVolume::mount(Fed(sd), start).map_err(|_| "not FAT32 or exFAT")
    }

    /// Writes the samples as little-endian 16-bit values (the CPU's own
    /// order, so the buffer goes as it is).
    fn save(out: &mut Text<'_>, sd: Option<&mut Sd>, name: &str, samples: &[u16], fat: &mut [u8]) {
        let Some(sd) = sd else { return };
        let bytes = rp_hal_mem::pixels_as_bytes(samples);
        let ok = mount(sd).and_then(|mut v| v.put(name, bytes, fat).map_err(|_| "write failed"));
        let _ = writeln!(out, "  {name}: {}", ok.map_or_else(|e| e, |_| "saved"));
    }

    /// Appends this run's first [`RESTART_ROW`] samples of A (low bytes) to
    /// `noise-r.bin`, the restart test's rows, up to [`RESTART_ROWS`].
    fn save_restart_row(
        out: &mut Text<'_>,
        sd: Option<&mut Sd>,
        a: &[u16],
        rows: &mut [u8],
        fat: &mut [u8],
    ) {
        let Some(sd) = sd else { return };
        let Ok(mut vol) = mount(sd) else { return };
        let mut have = 0usize;
        let mut fits = true;
        let _ = vol.read_back("noise-r.bin", fat, &mut |piece| {
            let end = have + piece.len();
            if end <= rows.len() {
                rows[have..end].copy_from_slice(piece);
                have = end;
            } else {
                fits = false;
            }
        });
        // Only whole rows count (a file cut short by hand loses its tail).
        have -= have % RESTART_ROW;
        if !fits || have >= rows.len() {
            let _ = writeln!(out, "  noise-r.bin: full ({RESTART_ROWS} rows): not added");
            return;
        }
        for (d, s) in rows[have..have + RESTART_ROW].iter_mut().zip(a) {
            *d = *s as u8;
        }
        have += RESTART_ROW;
        let ok = vol.put("noise-r.bin", &rows[..have], fat);
        let _ = writeln!(
            out,
            "  noise-r.bin: {}",
            if ok.is_ok() {
                "row added"
            } else {
                "write failed"
            }
        );
        let _ = writeln!(
            out,
            "  restart rows: {} of {RESTART_ROWS}",
            have / RESTART_ROW
        );
    }

    fn on_fault(kind: rt::Fault, addr: u32) -> ! {
        crash(format_args!("FAULT {} at {addr:#010x}", kind.name()))
    }

    #[inline(never)]
    fn crash(msg: core::fmt::Arguments) -> ! {
        let mut uart = PxaUart::new(rt::emergency_uart());
        let _ = writeln!(uart, "{msg}");
        devices::crash_to_loader(msg);
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
