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

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed with the verdict.
pub const MARKER: &str = "RETROPANE-CLOCKS";

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;
    use core::hint::black_box;

    use rp_bench_suite as suite;
    use rp_drv_mbx_display::{Display, PixelFormat, Plane};
    use rp_drv_pxa_gpio::Gpio;
    use rp_drv_pxa_uart::PxaUart;
    use rp_hal_pxa27x::Mmio;
    use rp_hal_pxa27x::clocks::{self, POINTS, POWER_REGS, Point, Raw};
    use rp_hal_pxa27x::{diag, rt};
    use rp_hal_x51v::devices::RunLog;
    use rp_hal_x51v::vcore::{self, Code};

    use super::MARKER;

    rp_hal_pxa27x::entry!(main);

    macro_rules! log {
        ($($t:tt)*) => {{ let _ = writeln!(RunLog, $($t)*); }};
    }

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

    fn main() -> ! {
        rt::set_fault_handler(on_fault);
        rt::init_mmu();
        let start = clocks::save();
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
        let mut gpio = Gpio::new(gpio);
        let (code0, ours) = vcode(&mut gpio);
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
        let Some((regs, v)) = rp_hal_x51v::devices::take_mbx() else {
            crash(format_args!("no 2700G"))
        };
        let (plane, kept): (Plane, bool) = Display::new(regs).adopt(v.len());
        let (low, high) = v.split_at_mut(rp_vram_map::POOL.at as usize);
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
            rp_vram_map::POOL.at as usize + FRESH + crumb_at
        );
        crumbs.mark(0, start.cccr);
        let mut vram = Some(pool);
        let (work, keep) = scratch.split_at_mut(scratch.len() - KEEP);
        fill(keep, 0x5EED_0001);
        benches("start", work, sram.as_deref_mut(), vram.as_deref_mut());

        // C2: the points that keep the memory clock, lowest risk first.
        let pts = [
            ("turbo off", sp.with_turbo(false)),
            ("416", POINTS[4].1),
            ("312", POINTS[3].1),
            ("208", POINTS[2].1),
            ("520", POINTS[5].1),
            ("624", POINTS[6].1),
        ];
        let mut top = None;
        for (i, (name, p)) in pts.iter().enumerate() {
            rt::check_in();
            log!("C2 {name} MHz");
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
                set_vcode(&mut gpio, need);
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
            set_vcode(&mut gpio, code0);
        }
        g.expect("voltage code as found", vcode(&mut gpio).0 == code0);
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
        rp_hal_x51v::devices::return_to_loader();
        loop {
            core::hint::spin_loop();
        }
    }

    /// The clocks as found, if saved.
    fn put_back() {
        let _ = clocks::restore_saved();
    }

    fn on_fault(kind: rt::Fault, addr: u32) -> ! {
        put_back();
        crash(format_args!("FAULT {} at {addr:#010x}", kind.name()))
    }

    #[inline(never)]
    fn crash(msg: core::fmt::Arguments) -> ! {
        rp_hal_x51v::devices::crash_to_loader(msg);
        loop {
            core::hint::spin_loop();
        }
    }

    #[panic_handler]
    fn panic(info: &core::panic::PanicInfo) -> ! {
        put_back();
        crash(format_args!("PANIC {}", info.message()))
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}
