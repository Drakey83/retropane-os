//! Test image (ADR-0031, audit item 67): RetroPane takes the PXA270's
//! clocks. Under the loader:
//!
//! - **C1, as found:** `CCCR`, `CCSR`, `CLKCFG`, `MDREFR`, `MDCNFG`, the
//!   power manager's set-up (voltage sequencer, power I2C, sleep, wake-up),
//!   the point decoded, and the core clock really running, counted by the
//!   CP14 cycle counter against the OS timer.
//! - **C2, the operating points that keep the memory clock at 208 MHz**,
//!   lowest risk first (run 1 never returned: the likeliest stop is its
//!   first step, 520 -> 104 MHz, the one that also halves the memory
//!   clock): turbo off (no relock), 416, 312 and 208 MHz (relocks, memory
//!   clock unchanged), 520 again, then 624 MHz. The memory-clock changes
//!   (104, 156 and the manual's 624) wait for later runs, one a run. Each
//!   step first paints a stripe on the screen and writes a breadcrumb into
//!   2700G memory (`RPCK`, the step, the `CCCR` aimed at), so a run that
//!   never returns still shows where it stopped. The core voltage code (GPIO3/4/38, `rp_hal_x51v::vcore`)
//!   is never lowered here; a point needing a higher code than the one
//!   found gets it raised first (then the settle wait), only if all three
//!   pins are plain GPIO outputs: switched to, the core clock counted,
//!   `CCSR` checked, 1 s
//!   dwelt, 3 MiB of SDRAM written before the first switch checked
//!   (refresh kept through every change) and 256 KiB written and checked
//!   at the point, then benchmarks: `memcpy`/`memset`, SDRAM and SRAM
//!   pointer chases, 256 KiB written to the 2700G.
//! - **C3, switch costs:** turbo off/on at the top allowed point (no PLL
//!   relock) and a relock to 208 MHz (memory clock unchanged), each timed
//!   over many switches.
//! - The starting clocks put back exactly (and on any fault before the
//!   crash report), checked, then the starting voltage code (after the
//!   clock, as any slow-down; a fault leaves the higher code: safe).
//!
//! **Stage C2b** (`RP_CLOCKS_OC`, the owner's go 2026-10-30 on
//! `docs/research/sheets/c2-cpu-overclock-test1.md`): instead of C2 and C3,
//! the core past Intel's 624 MHz by the turbo multiplier alone (L 16 x 2N 7
//! and 8: 728 and 832 MHz; run, bus and memory clocks as at 624, the
//! voltage code never changed), 624 first as the control. Each step:
//! whether the PLL took it (`CCSR`), the core counted, then [`OC_SECS`] of
//! known-answer integer work, SDRAM patterns and cache sweeps, the touch
//! controller's `TEMP0` before and after; the first error ends the stage.
//! Dell's 624 is put back after each step and at the end.
//!
//! **Run F** (`RP_CLOCKS_FAULT`, quality audit C31) changes no clock: it
//! writes `CCCR` asking for the same point with turbo at 1x (2N 2, a lower
//! core clock and the same memory clock, in case anything applies it),
//! checks the core still runs at `CCSR`'s point, saves the clocks in that
//! state, then reads an unmapped address. The crash path's put-back
//! (`clocks::restore_saved`) must keep the point running (C31's bug chose
//! `CCCR`'s) and leave `CCCR` naming it (C42), checked before the crash
//! report and the loader.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed with the verdict.
pub const MARKER: &str = "RETROPANE-CLOCKS";

#[cfg(target_os = "none")]
mod image {
    use core::cell::RefCell;
    use core::fmt::Write;
    use core::hint::black_box;

    use rp_bench_suite as suite;
    use rp_drv_mbx_display::{Display, PixelFormat, Plane};
    use rp_drv_pxa_gpio::{Gpio, GpioPin};
    use rp_drv_pxa_ssp::Ssp;
    use rp_drv_pxa_uart::PxaUart;
    use rp_drv_tsc2046::{Tsc2046, channel, single};
    use rp_hal_pxa27x::Mmio;
    use rp_hal_pxa27x::clocks::{self, POINTS, POWER_REGS, Point, Raw};
    use rp_hal_pxa27x::{diag, rt};
    use rp_hal_x51v::devices::RunLog;
    use rp_hal_x51v::vcore::{self, Code};

    use super::MARKER;

    use rp_image::crash;

    rp_image::entry!(main);

    macro_rules! log {
        ($($t:tt)*) => {{ let _ = writeln!(RunLog, $($t)*); }};
    }

    /// Run F instead (see the crate's docs).
    const FAULT: bool = option_env!("RP_CLOCKS_FAULT").is_some();
    /// Stage C2b instead of C2 and C3 (see the crate's docs).
    const OC: bool = option_env!("RP_CLOCKS_OC").is_some();
    /// Seconds of checks at each C2b step (the emulator's check: 1).
    const OC_SECS: u32 = if option_env!("RP_CLOCKS_OC_SHORT").is_some() {
        1
    } else {
        60
    };
    /// C2b reads the touch controller's `TEMP0` (`RP_CLOCKS_OC_TEMP`): off
    /// since C2b runs 1-2 hung at or right after the first read, before
    /// any clock change (R123).
    const OC_TEMP: bool = option_env!("RP_CLOCKS_OC_TEMP").is_some();
    /// The touch controller's chip select and pen line (touch.md).
    const TS_CS: u8 = 24;
    const TS_PENIRQ: u8 = 94;
    /// The touch controller as a thermometer.
    type Touch<'a> = Tsc2046<Ssp<Mmio>, GpioPin<'a, Mmio>, GpioPin<'a, Mmio>>;

    /// Run F: the clocks as found (`CCCR`, `CCSR`, `CLKCFG`) and the core
    /// counted, for the put-back's checks; checks and BADs before the
    /// fault.
    static F_FOUND: rt::Global<[u32; 4]> = rt::Global::new([0; 4]);
    static F_CHECKS: rt::Global<[u32; 2]> = rt::Global::new([0; 2]);

    /// Read by run F to fault (nothing is mapped there).
    const UNMAPPED: usize = 0x8000_0000;

    /// Bytes of SDRAM written before the first switch and checked after each.
    const KEEP: usize = 3 * 1024 * 1024;
    /// Bytes written and checked at each point.
    const FRESH: usize = 256 * 1024;

    /// Breadcrumb magic in 2700G memory: `RPCK`.
    const CRUMB_MAGIC: u32 = u32::from_le_bytes(*b"RPCK");

    /// Where each step is shown and recorded, even if the run never returns:
    /// a stripe on the screen (RGB565 plane) and three words at the end of
    /// the 2700G pool (2700G memory isn't cleared between images; whether a
    /// watchdog reset clears it is itself worth knowing).
    struct Crumbs<'a> {
        screen: Option<(&'a mut [u8], usize)>,
        word: &'a mut [u8],
    }

    impl Crumbs<'_> {
        fn mark(&mut self, step: u32, cccr: u32) {
            for (k, w) in [CRUMB_MAGIC, step, cccr].into_iter().enumerate() {
                self.word[4 * k..4 * k + 4].copy_from_slice(&w.to_le_bytes());
            }
            const COLOURS: [u16; 8] = [
                0xF800, 0xFFE0, 0x07E0, 0x07FF, 0x001F, 0xF81F, 0xFFFF, 0xFC00,
            ];
            if let Some((px, stride)) = self.screen.as_mut() {
                let c = COLOURS[step as usize % COLOURS.len()].to_le_bytes();
                let top = 40 * step as usize + 8;
                for y in top..top + 24 {
                    if let Some(row) = px.get_mut(y * *stride..y * *stride + 480 * 2) {
                        for p in row.chunks_exact_mut(2) {
                            p.copy_from_slice(&c);
                        }
                    }
                }
            }
        }
    }

    impl Crumbs<'_> {
        /// C2b's progress: thin stripe `n` (10 px apart from row 300, in
        /// turn white and orange), so the stripes counted on the screen say
        /// which sub-step a hang stopped at, without the card log (C2b runs
        /// 1-2 kept only its first sector).
        fn tick(&mut self, n: u32) {
            if let Some((px, stride)) = self.screen.as_mut() {
                let c = if n % 2 == 0 { 0xFFFFu16 } else { 0xFC00 }.to_le_bytes();
                let top = 300 + 10 * n as usize;
                for y in top..top + 6 {
                    if let Some(row) = px.get_mut(y * *stride..y * *stride + 480 * 2) {
                        for p in row.chunks_exact_mut(2) {
                            p.copy_from_slice(&c);
                        }
                    }
                }
            }
        }
    }

    /// Benchmark results to the run log, tagged with the point.
    struct Sink(&'static str);

    impl suite::Sink for Sink {
        fn result(&mut self, name: &'static str, ticks: u32, iters: u32) {
            // ns per iteration from 3.25 MHz ticks: ticks x 4000 / 13.
            let ns = u64::from(ticks) * 4000 / 13 / u64::from(iters.max(1));
            log!("  BENCH {} {name} {ticks} {iters} = {ns} ns", self.0);
        }
        fn check(&mut self, name: &'static str, ok: bool, detail: &str) {
            log!(
                "  CHECK {} {name} {} {detail}",
                self.0,
                if ok { "OK" } else { "BAD" }
            );
        }
    }

    struct Rig {
        checks: u32,
        bad: u32,
    }

    impl Rig {
        fn expect(&mut self, what: &str, ok: bool) {
            self.checks += 1;
            if !ok {
                self.bad += 1;
            }
            log!("  {} {what}", if ok { "ok " } else { "BAD" });
        }
    }

    fn word(i: usize, seed: u32) -> u32 {
        (i as u32).wrapping_mul(0x9E37_79B9) ^ seed
    }

    fn fill(buf: &mut [u8], seed: u32) {
        for (i, w) in buf.chunks_exact_mut(4).enumerate() {
            w.copy_from_slice(&word(i, seed).to_le_bytes());
        }
    }

    /// Words that differ from the pattern.
    fn verify(buf: &[u8], seed: u32) -> u32 {
        let mut bad = 0;
        for (i, w) in buf.chunks_exact(4).enumerate() {
            if u32::from_le_bytes([w[0], w[1], w[2], w[3]]) != word(i, seed) {
                bad += 1;
            }
        }
        bad
    }

    fn log_raw(tag: &str, r: Raw) {
        let p = r.point();
        log!(
            "{tag}: CCCR 0x{:08x} CCSR 0x{:08x} CLKCFG 0x{:x} MDREFR 0x{:08x} MDCNFG 0x{:08x}",
            r.cccr,
            r.ccsr,
            r.clkcfg,
            r.mdrefr,
            r.mdcnfg
        );
        log!(
            "  L {} 2N {} A {} B {} T {} HT {}: run {} turbo {} core {} bus {} mem {} SDCLK {} kHz, DRI {} (rule {}), needs {} mV",
            p.l,
            p.n2,
            u8::from(p.a),
            u8::from(p.b),
            u8::from(p.turbo),
            u8::from(p.half),
            p.run_khz(),
            p.turbo_khz(),
            p.core_khz(),
            p.bus_khz(),
            p.mem_khz(),
            p.sdclk_khz(),
            r.mdrefr & clocks::MDREFR_DRI,
            clocks::dri(p.mem_khz(), clocks::sdram_rows(r.mdcnfg)),
            p.min_mv()
        );
    }

    /// The voltage code on the pins now, and whether all three are plain
    /// GPIO outputs (so RetroPane may drive them).
    fn vcode(gpio: &mut Gpio<Mmio>) -> (Code, bool) {
        let [a, b, c] = vcore::PINS.map(|p| gpio.read(p));
        let ours = vcore::PINS
            .iter()
            .all(|&p| gpio.function(p) == 0 && gpio.is_output(p));
        (Code::from_levels(a, b, c), ours)
    }

    fn set_vcode(gpio: &mut Gpio<Mmio>, to: Code) {
        for (p, high) in vcore::PINS.into_iter().zip(to.levels()) {
            gpio.write(p, high);
        }
    }

    /// Times `n` round trips `a` -> `b` -> `a`; returns ticks.
    fn time_switches(a: Point, b: Point, n: u32) -> u32 {
        let t0 = rt::ticks();
        for _ in 0..n {
            clocks::change(b);
            clocks::change(a);
        }
        rt::ticks().wrapping_sub(t0)
    }

    fn benches(
        tag: &'static str,
        scratch: &mut [u8],
        sram: Option<&mut [u8]>,
        vram: Option<&mut [u8]>,
    ) {
        let mut sink = Sink(tag);
        let mut r = suite::Runner::new(&mut sink);
        suite::memory(&mut r, &mut scratch[..128 * 1024]);
        suite::chase(&mut r, "chase_sdram_256k", &mut scratch[..256 * 1024]);
        if let Some(s) = sram {
            suite::chase(&mut r, "chase_sram_256k", s);
        }
        let mut n = 0u32;
        r.run("alu_loop_1000", 100, &mut || {
            for i in 0..1000u32 {
                n = black_box(n.wrapping_add(i));
            }
        });
        if let Some(v) = vram {
            let src = &scratch[..FRESH];
            r.run("vram_write_256k", 4, &mut || {
                v[..FRESH].copy_from_slice(src);
                black_box(&v);
            });
        }
    }

    /// Known-answer integer work (shifts, multiplies, a 64-bit product,
    /// rotates): the same `seed` gives the same answer at any clock.
    #[inline(never)]
    fn int_work(seed: u32) -> u32 {
        let (mut x, mut acc) = (seed | 1, 0u32);
        for i in 0..200_000u32 {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            acc = acc.wrapping_mul(0x0100_0193).wrapping_add(x ^ i);
            acc = acc.rotate_left(7) ^ ((u64::from(x) * u64::from(acc)) >> 32) as u32;
        }
        black_box(acc)
    }

    /// `buf` read a cache line (32 bytes) at a time, four times over (the
    /// D-cache filled and evicted): the sum of the lines' first words.
    #[inline(never)]
    fn sweep(buf: &[u8]) -> u32 {
        let mut sum = 0u32;
        for _ in 0..4 {
            for line in buf.chunks_exact(32) {
                sum = sum.wrapping_add(u32::from_le_bytes([line[0], line[1], line[2], line[3]]));
            }
            black_box(&buf);
        }
        sum
    }

    /// [`sweep`]'s answer for `bytes` filled by [`fill`] with `seed`,
    /// worked out from the pattern rather than read.
    fn sweep_want(bytes: usize, seed: u32) -> u32 {
        let one = (0..bytes / 4)
            .step_by(8)
            .fold(0u32, |a, i| a.wrapping_add(word(i, seed)));
        one.wrapping_mul(4)
    }

    /// The touch controller borrowed as a thermometer (as `rp-gpubench`'s
    /// warm runs: GPIO24 a plain GPIO, the pen line an input). `None`: no
    /// SSP to borrow.
    fn c2b_touch(gpio: &RefCell<Gpio<Mmio>>) -> Option<Touch<'_>> {
        let ssp = rt::take_ssp1()?;
        gpio.borrow_mut().make_output(TS_CS, true);
        gpio.borrow_mut().set_function(TS_CS, 0);
        gpio.borrow_mut().make_input(TS_PENIRQ);
        let mut ssp = Ssp::new(ssp);
        ssp.configure_spi(2_600_000);
        Some(Tsc2046::new(
            ssp,
            GpioPin::new(gpio, TS_CS),
            GpioPin::new(gpio, TS_PENIRQ),
        ))
    }

    /// `TEMP0`, 64 conversions after 64 to settle, averaged (the chip's
    /// diode: it falls as the board warms).
    fn temp0(t: &mut Touch<'_>) -> u32 {
        for _ in 0..64 {
            t.read_single(single::TEMP0);
        }
        let sum: u32 = (0..64)
            .map(|_| u32::from(t.read_single(single::TEMP0)))
            .sum();
        // A differential conversion re-arms the pen line, as the touch
        // driver leaves it.
        t.read(channel::Y);
        sum / 64
    }

    /// Stage C2b (see the crate's docs): the core past 624 MHz by the turbo
    /// multiplier alone, 624 first as the control; the first error ends it.
    /// `keep` holds [`fill`]'s pattern with seed `0x5EED_0001`. The
    /// voltage code is raised to Dell's own for 624 MHz if Windows Mobile
    /// left a lower one (as C2 does; never past it), and returned for the
    /// put-back.
    #[inline(never)]
    #[allow(clippy::too_many_lines)]
    fn c2b(
        g: &mut Rig,
        gpio: &RefCell<Gpio<Mmio>>,
        (mut code, ours): (Code, bool),
        crumbs: &mut Crumbs<'_>,
        (work, keep): (&mut [u8], &[u8]),
    ) -> Code {
        log!(
            "C2b (the owner's go 2026-10-30): the core past 624 MHz by the turbo multiplier (L 16, run, bus and memory as at 624), {OC_SECS} s of checks a step; the voltage code 0b{:03b} as found, never past Dell's for 624",
            code.0
        );
        let top = Code::for_turbo_khz(624_000);
        if !code.covers(top) {
            if !ours {
                log!(
                    "  skipped: the voltage code is below Dell's for 624 MHz and the pins aren't ours"
                );
                return code;
            }
            set_vcode(&mut gpio.borrow_mut(), top);
            rt::delay_us(vcore::RAISE_SETTLE_US);
            code = top;
            log!(
                "  voltage code raised to Dell's for 624 MHz, 0b{:03b}",
                top.0
            );
        }
        crumbs.tick(0);
        rp_hal_x51v::cardsink::flush();
        crumbs.tick(1);
        let mut touch = if OC_TEMP { c2b_touch(gpio) } else { None };
        crumbs.tick(2);
        // The answers, worked out at the clock as found (624).
        let refs = [int_work(1), int_work(2), int_work(3), int_work(4)];
        crumbs.tick(3);
        let at624 = POINTS[6].1;
        let steps = [
            ("624 (the control)", at624),
            ("728", Point { n2: 7, ..at624 }),
            ("832", Point { n2: 8, ..at624 }),
        ];
        let bytes = (1024 * 1024).min(work.len()) & !31;
        for (i, (name, p)) in steps.into_iter().enumerate() {
            rt::check_in();
            log!("C2b {name} MHz");
            // On the card before the change: a hang keeps this line (run
            // 1 lost every line after the first sector).
            rp_hal_x51v::cardsink::flush();
            let base = 4 + 5 * i as u32;
            crumbs.tick(base);
            let before = touch.as_mut().map(temp0);
            let ok = clocks::change_past_rating(p);
            crumbs.tick(base + 1);
            let now = clocks::read();
            log_raw("  now", now);
            if !ok {
                g.expect("the change planned", false);
                break;
            }
            if now.point() != p {
                log!(
                    "  the PLL didn't take 2N {} (CCSR shows 2N {}): the step skipped",
                    p.n2,
                    now.point().n2
                );
                clocks::change(at624);
                continue;
            }
            let khz = clocks::measure_core_khz(20);
            crumbs.tick(base + 2);
            let speed = khz.abs_diff(p.core_khz()) <= p.core_khz() / 50;
            log!("  core counted {khz} kHz");
            g.expect("core clock counted (2%)", speed);
            let t0 = rt::ticks();
            let (mut rounds, mut errors) = (0u32, 0u32);
            let mut first = None;
            while rt::ticks().wrapping_sub(t0) < OC_SECS * 3_250_000 {
                rt::check_in();
                let k = (rounds % 4) as usize;
                if int_work(k as u32 + 1) != refs[k] {
                    errors += 1;
                    first.get_or_insert("integer");
                }
                let seed = 0xC2B0_0000 | rounds;
                fill(&mut work[..bytes], seed);
                if verify(&work[..bytes], seed) != 0 {
                    errors += 1;
                    first.get_or_insert("SDRAM pattern");
                }
                if sweep(&work[..bytes / 4]) != sweep_want(bytes / 4, seed) {
                    errors += 1;
                    first.get_or_insert("cache sweep");
                }
                rounds += 1;
                if errors != 0 {
                    break;
                }
            }
            let secs = rt::ticks().wrapping_sub(t0) / 3_250_000;
            crumbs.tick(base + 3);
            let lost = verify(keep, 0x5EED_0001);
            let after = touch.as_mut().map(temp0);
            log!(
                "  {rounds} rounds in {secs} s: errors {errors} (first: {first:?}); kept 3 MiB: {lost} words differ; TEMP0 {before:?} -> {after:?} (falls as it warms)"
            );
            g.expect("every check right", errors == 0 && lost == 0);
            clocks::change(at624);
            crumbs.tick(base + 4);
            if errors != 0 || lost != 0 || !speed {
                log!("  the first error: the stage ends here");
                break;
            }
        }
        let back = clocks::read();
        g.expect("back at 624 after the stage", back.point() == at624);
        code
    }

    fn main() -> ! {
        rt::init_mmu();
        if FAULT {
            fault_path()
        }
        let start = clocks::save();
        // A crash puts the clocks back first (rp-image's crash order).
        rp_image::on_crash(put_back);
        let sp = start.point();
        log!("{MARKER}: RetroPane takes the PXA270's clocks (ADR-0031), version 3");
        // The banner and the verdict also go to the serial port (the
        // emulator's check waits for them there; on the device a bounded
        // write to an unclocked UART).
        let mut uart = rt::take_ffuart().map(|bus| {
            let mut u = PxaUart::new(bus);
            u.init(115_200);
            let _ = writeln!(u, "{MARKER}: version 3");
            u
        });
        log_raw("C1 as found", start);
        let power = clocks::power_snapshot();
        for ((name, _), v) in POWER_REGS.iter().zip(power) {
            log!("  {name} 0x{v:08x}");
        }
        let Some(gpio) = rt::take_gpio() else {
            crash(format_args!("no GPIO"))
        };
        let gpio = RefCell::new(Gpio::new(gpio));
        let (code0, ours) = vcode(&mut gpio.borrow_mut());
        log!(
            "  voltage code 0b{:03b} (Dell's label {:?} mV), pins plain outputs: {ours}; Dell's code for this point 0b{:03b}",
            code0.0,
            code0.dell_label_mv(),
            Code::for_turbo_khz(sp.turbo_khz()).0
        );
        let mut code = code0;
        let mut g = Rig { checks: 0, bad: 0 };
        let khz = clocks::measure_core_khz(20);
        log!("  core counted {khz} kHz (decoded {})", sp.core_khz());
        g.expect(
            "cycle counter agrees with the decoded core clock (2%)",
            khz.abs_diff(sp.core_khz()) <= sp.core_khz() / 50,
        );

        let Some(scratch) = rp_hal_x51v::devices::take_scratch() else {
            crash(format_args!("no scratch"))
        };
        let mut sram = diag::take_sram();
        let Some((regs, v)) = rp_image::take_mbx() else {
            crash(format_args!("no 2700G"))
        };
        let (plane, kept): (Plane, bool) = Display::new(regs).adopt(v.len());
        let (low, high) = v.split_at_mut(rp_vram_map::bench32::POOL.at as usize);
        let (pool, rest) = high.split_at_mut(FRESH);
        let crumb_at = rest.len() - 16;
        let screen_end = plane.offset + plane.stride * usize::from(plane.height);
        let mut crumbs = Crumbs {
            screen: (plane.format == PixelFormat::Rgb565 && screen_end <= low.len())
                .then(|| (&mut low[plane.offset..screen_end], plane.stride)),
            word: &mut rest[crumb_at..],
        };
        log!(
            "  screen plane {} at 0x{:x} ({}); breadcrumbs at 2700G 0x{:x}",
            if kept { "kept" } else { "set up" },
            plane.offset,
            if crumbs.screen.is_some() {
                "stripes"
            } else {
                "no stripes"
            },
            rp_vram_map::bench32::POOL.at as usize + FRESH + crumb_at
        );
        crumbs.mark(0, start.cccr);
        let mut vram = Some(pool);
        let (work, keep) = scratch.split_at_mut(scratch.len() - KEEP);
        fill(keep, 0x5EED_0001);
        benches("start", work, sram.as_deref_mut(), vram.as_deref_mut());

        if OC {
            code = c2b(&mut g, &gpio, (code, ours), &mut crumbs, (work, &*keep));
        }

        // C2: the points that keep the memory clock, lowest risk first.
        let pts = if OC {
            [None; 6]
        } else {
            [
                Some(("turbo off", sp.with_turbo(false))),
                Some(("416", POINTS[4].1)),
                Some(("312", POINTS[3].1)),
                Some(("208", POINTS[2].1)),
                Some(("520", POINTS[5].1)),
                Some(("624", POINTS[6].1)),
            ]
        };
        let mut top = None;
        for (i, (name, p)) in pts
            .iter()
            .enumerate()
            .filter_map(|(i, x)| x.map(|x| (i, x)))
        {
            let p = &p;
            rt::check_in();
            log!("C2 {name} MHz");
            rp_hal_x51v::cardsink::flush();
            if p.mem_khz() != sp.mem_khz() {
                log!("  skipped (changes the memory clock: a later run)");
                continue;
            }
            crumbs.mark(i as u32 + 1, p.cccr(start.cccr));
            let need = Code::for_turbo_khz(p.turbo_khz());
            if !code.covers(need) {
                if !ours {
                    log!("  skipped (needs code 0b{:03b}, pins not ours)", need.0);
                    continue;
                }
                set_vcode(&mut gpio.borrow_mut(), need);
                rt::delay_us(vcore::RAISE_SETTLE_US);
                code = need;
                log!("  voltage code raised to 0b{:03b}", need.0);
            }
            let t0 = rt::ticks();
            let ok = clocks::change(*p);
            let took = rt::ticks().wrapping_sub(t0);
            let now = clocks::read();
            log_raw("  now", now);
            g.expect("change accepted", ok);
            g.expect("CCSR and CLKCFG show the point", now.point() == *p);
            let khz = clocks::measure_core_khz(20);
            log!("  core counted {khz} kHz; change took {took} ticks");
            g.expect(
                "core clock counted (2%)",
                khz.abs_diff(p.core_khz()) <= p.core_khz() / 50,
            );
            for _ in 0..4 {
                rt::check_in();
                rt::delay_us(250_000);
            }
            let lost = verify(keep, 0x5EED_0001);
            log!("  kept 3 MiB: {lost} words differ");
            g.expect("SDRAM kept across the change and 1 s here", lost == 0);
            let seed = 0xF00D_0000 | i as u32;
            fill(&mut work[..FRESH], seed);
            g.expect(
                "256 KiB written and read back here",
                verify(&work[..FRESH], seed) == 0,
            );
            benches(name, work, sram.as_deref_mut(), vram.as_deref_mut());
            if p.turbo {
                top = Some(*p);
            }
        }

        // C3: switch costs at the top allowed point.
        if let Some(t) = top {
            rt::check_in();
            clocks::change(t);
            let off = t.with_turbo(false);
            let n = 1000;
            let ticks = time_switches(t, off, n);
            let khz = clocks::measure_core_khz(20);
            log!(
                "C3 turbo {} <-> {} kHz: {n} round trips {ticks} ticks = {} ns a switch; core after {khz} kHz",
                t.core_khz(),
                off.core_khz(),
                u64::from(ticks) * 4000 / 13 / u64::from(2 * n)
            );
            g.expect(
                "turbo back on after the switches",
                khz.abs_diff(t.core_khz()) <= t.core_khz() / 50,
            );
            crumbs.mark(7, POINTS[2].1.cccr(start.cccr));
            let low = POINTS[2].1;
            let n = 100;
            let ticks = time_switches(t, low, n);
            log!(
                "C3 relock {} <-> {} kHz: {n} round trips {ticks} ticks = {} ns a change",
                t.core_khz(),
                low.core_khz(),
                u64::from(ticks) * 4000 / 13 / u64::from(2 * n)
            );
            let lost = verify(keep, 0x5EED_0001);
            g.expect("SDRAM kept across the switch storms", lost == 0);
        }

        // Back as found.
        rt::check_in();
        crumbs.mark(8, start.cccr);
        g.expect("put back", clocks::restore(start));
        if code != code0 {
            set_vcode(&mut gpio.borrow_mut(), code0);
        }
        g.expect(
            "voltage code as found",
            vcode(&mut gpio.borrow_mut()).0 == code0,
        );
        let end = clocks::read();
        log_raw("end", end);
        g.expect(
            "registers as found",
            end.cccr == start.cccr
                && (end.clkcfg ^ start.clkcfg) & !clocks::clkcfg::F == 0
                && end.mdrefr == start.mdrefr,
        );
        let khz = clocks::measure_core_khz(20);
        log!("  core counted {khz} kHz");
        let (checks, bad) = (g.checks, g.bad);
        log!("{MARKER}: {checks} checks, {bad} BAD");
        if let Some(u) = uart.as_mut() {
            let _ = writeln!(u, "{MARKER}: {checks} checks, {bad} BAD");
        }
        rp_image::finish()
    }

    /// The clocks as found, if saved.
    fn put_back() {
        let _ = clocks::restore_saved();
    }

    /// Run F (see the crate's docs): never returns; the crash path ends it.
    fn fault_path() -> ! {
        rp_image::on_crash(fault_put_back);
        log!(
            "{MARKER}: run F, the fault path's put-back with CCCR asking for another point than the one running (quality audit C31), version 3"
        );
        if let Some(bus) = rt::take_ffuart() {
            let mut u = PxaUart::new(bus);
            u.init(115_200);
            let _ = writeln!(u, "{MARKER}: run F");
        }
        let mut g = Rig { checks: 0, bad: 0 };
        let found = clocks::read();
        log_raw("F1 as found", found);
        let fp = found.point();
        let khz0 = clocks::measure_core_khz(20);
        log!("  core counted {khz0} kHz (decoded {})", fp.core_khz());
        F_FOUND.with(|f| *f = [found.cccr, found.ccsr, found.clkcfg, khz0]);
        if Point::decode(found.cccr, found.cccr, found.clkcfg) != fp {
            rp_image::fail("CCCR and CCSR disagree as found: run F needs them agreeing");
        }
        // The same point with turbo at 1x: a lower core clock, the same
        // memory clock and bus, so nothing is at risk if anything applies it.
        let asked = Point { n2: 2, ..fp };
        if fp.n2 == 2 || !asked.legal() {
            rp_image::fail("no lower turbo multiplier to ask for");
        }
        g.expect("CCCR written (no frequency change)", clocks::ask(asked));
        let now = clocks::read();
        log_raw("F2 CCCR asks for 2N 2", now);
        log!(
            "  the point CCCR names: core {} kHz (what restore chose before C31's fix)",
            Point::decode(now.cccr, now.cccr, now.clkcfg).core_khz()
        );
        g.expect("CCSR unchanged", now.ccsr == found.ccsr);
        g.expect(
            "CCCR now names the asked point",
            Point::decode(now.cccr, now.cccr, now.clkcfg) == asked,
        );
        g.expect("the running point unchanged", now.point() == fp);
        let khz = clocks::measure_core_khz(20);
        log!("  core counted {khz} kHz");
        g.expect(
            "the core runs as found (2%): a CCCR write alone changes nothing",
            khz.abs_diff(khz0) <= khz0 / 50,
        );
        let saved = clocks::save();
        log!(
            "F3 saved with CCCR 0x{:08x} and CCSR 0x{:08x}: restore will put back core {} kHz",
            saved.cccr,
            saved.ccsr,
            saved.point().core_khz()
        );
        g.expect("the saved point is the one running", saved.point() == fp);
        F_CHECKS.with(|c| *c = [g.checks, g.bad]);
        log!(
            "F4 a data abort (a read of 0x{UNMAPPED:08x}): the crash path puts the clocks back first"
        );
        let _ = black_box(rt::fault_probe(black_box(UNMAPPED)));
        crash(format_args!("the read of 0x{UNMAPPED:08x} didn't fault"))
    }

    /// Run F's put-back, first in the crash path: the saved clocks put
    /// back, then checked.
    fn fault_put_back() {
        let ok = clocks::restore_saved();
        let [cccr, ccsr, clkcfg, khz0] = F_FOUND.with(|f| *f);
        let fp = Point::decode(cccr, ccsr, clkcfg);
        let [checks, bad] = F_CHECKS.with(|c| *c);
        let mut g = Rig { checks, bad };
        let after = clocks::read();
        log_raw("F5 put back by the crash path", after);
        let khz = clocks::measure_core_khz(20);
        log!("  core counted {khz} kHz");
        g.expect("restore_saved accepted", ok);
        g.expect("the point running is the one found", after.point() == fp);
        g.expect("CCSR as found", after.ccsr == ccsr);
        g.expect(
            "the core runs as found (2%)",
            khz.abs_diff(khz0) <= khz0 / 50,
        );
        g.expect(
            "CCCR names the point running (C42: no request left armed)",
            Point::decode(after.cccr, after.cccr, after.clkcfg) == fp,
        );
        let end = clocks::read();
        log_raw("F6 end", end);
        g.expect("CCCR as found", end.cccr == cccr);
        g.expect("CCSR as found", end.ccsr == ccsr);
        let (checks, bad) = (g.checks, g.bad);
        log!(
            "{MARKER}: run F: {checks} checks, {bad} BAD (the crash report follows: it is the run's end)"
        );
        let _ = writeln!(
            PxaUart::new(rt::emergency_uart()),
            "{MARKER}: run F: {checks} checks, {bad} BAD"
        );
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}
