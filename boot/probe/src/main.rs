//! RetroPane hardware probe: the SD-card image for real X51v bring-up.
//!
//! The X51v has no serial port, so everything RetroPane needs to learn from a
//! real device is shown on screen, in pages a phone photo can capture, each
//! line ending in a checksum ([`report`]):
//!
//! 1. **System:** CPU and coprocessor state, clocks, memory controller,
//!    panel, SD card.
//! 2. **WM state:** interrupt, power, backlight and GPIO configuration as
//!    Windows Mobile left it.
//! 3. **2700G:** the registers Dell's driver saves across suspend (the
//!    display timing and clock encodings in docs/hw/2700g.md's open
//!    questions).
//! 4. **Bench:** the shared benchmark suite, including SRAM vs SDRAM latency
//!    and TLB-miss cost with the page table in SDRAM and in SRAM (ADR-0006).
//!
//! Hardware state is captured first, before RetroPane changes anything. The
//! screen keeps the framebuffer Windows Mobile configured when it can decode
//! it, and only reprograms the display if not, so a wrong timing guess can't
//! blank the report. Everything is mirrored to the serial port, where
//! `cargo xtask emu` verifies every line's checksum.
//!
//! D-pad right/down/centre or a tap: next page. Left/up: previous page.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

// On the host only the tests use the report (the host `main` is empty).
#[cfg_attr(not(target_os = "none"), allow(dead_code))]
mod report;

/// Printed once the report is complete.
pub const MARKER: &str = "RETROPANE-PROBE-DONE";

/// Which build produced a photo: set by `cargo xtask sdkit` (`RP_BUILD_ID`).
pub const BUILD: &str = match option_env!("RP_BUILD_ID") {
    Some(b) => b,
    None => "dev",
};

#[cfg(target_os = "none")]
mod image {
    use core::cell::RefCell;
    use core::fmt::Write;

    use super::report::{self, PAGES, Report};
    use rp_bench_suite as suite;
    use rp_drv_cpld::{Cpld, Delay};
    use rp_drv_mbx_display::console::Console;
    use rp_drv_mbx_display::{Display, PixelFormat, Plane, X51V_VGA, reg};
    use rp_drv_pxa_gpio::{Gpio, GpioPin};
    use rp_drv_pxa_keypad::{Button, Keypad, Pressed};
    use rp_drv_pxa_mmc::PxaMmc;
    use rp_drv_pxa_ssp::Ssp;
    use rp_drv_pxa_uart::PxaUart;
    use rp_drv_sdcard::{SdCard, mbr_partitions};
    use rp_drv_tsc2046::Tsc2046;
    use rp_hal_api::RegisterBus;
    use rp_hal_pxa27x::{Mmio, diag, rt};
    use rp_hal_x51v::MBX_VRAM_SIZE;

    rp_hal_pxa27x::entry!(main);

    /// The 2700G registers Dell's driver saves and restores across suspend
    /// (docs/hw/2700g.md): its own list of essential, safely readable state.
    const GPU_REGS: &[(&str, usize)] = &[
        ("SYSCFG", 0x000),
        ("PIXCLK", 0x048),
        ("MEMCLK", 0x04C),
        ("M24CLK", 0x050),
        ("MBXCLK", 0x054),
        ("SDCLK", 0x058),
        ("PIXCLKDIV", 0x05C),
        ("LCDCONFIG", 0x060),
        ("LMCFG", 0x1004),
        ("LMPWR", 0x1008),
        ("LMCEMR", 0x1010),
        ("LMTYPE", 0x1014),
        ("LMTIM", 0x1018),
        ("LMREFRESH", 0x101C),
        ("LMPROTMIN", 0x1020),
        ("LMPROTMAX", 0x1024),
        ("LMPROTCFG", 0x1028),
        ("GSCTRL", 0x2000),
        ("GBBASE", 0x2020),
        ("GDRCTRL", 0x2040),
        ("GSCADR", 0x2060),
        ("GSADR", 0x20C0),
        ("GPLUT", 0x2150),
        ("VSCTRL", 0x2004),
        ("VBBASE", 0x2024),
        ("VCMSK", 0x2044),
        ("VSCADR", 0x2064),
        ("VUBASE", 0x2084),
        ("VSADR", 0x20C4),
        ("BGCOLOR", 0x2174),
        ("HCCTRL", 0x2100),
        ("HCSIZE", 0x2110),
        ("HCPOS", 0x2120),
        ("HCBADR", 0x2130),
        ("HCCKMSK", 0x2140),
        ("DSCTRL", 0x2154),
        ("DUCTRL", 0x230C),
        ("DHT01", 0x2158),
        ("DHT02", 0x215C),
        ("DHT03", 0x2160),
        ("DVT01", 0x2164),
        ("DVT02", 0x2168),
        ("DVT03", 0x216C),
        ("DVECTRL", 0x2310),
        ("DHDET", 0x2314),
        ("DVDET", 0x2318),
        ("DODMSK", 0x231C),
        ("DBCOL", 0x2170),
        ("DSIG", 0x2184),
        ("DVLNUM", 0x2308),
        ("DMCTRL", 0x2188),
        ("DINTRE", 0x217C),
        ("DINTRCNT", 0x2180),
        ("DLLCTRL", 0x2304),
        ("CLIPCTRL", 0x218C),
        ("SPOCTRL", 0x2190),
        ("SVCTRL", 0x2194),
        ("SHCTRL", 0x21B0),
        ("SSSIZE", 0x21D8),
    ];

    /// Light grey on black: highest contrast for a photo.
    const COLORS: (u16, u16) = (0xFFFF, 0x0000);
    const SCALE: usize = 2;
    const TS_CS: u8 = 24;
    const TS_PENIRQ: u8 = 94;
    const MAX_RESULTS: usize = 24;

    struct TimerDelay;
    impl Delay for TimerDelay {
        fn delay_us(&mut self, us: u32) {
            rt::delay_us(us);
        }
    }

    /// Benchmark results kept for the report.
    struct Results {
        bench: [(&'static str, u32, u32); MAX_RESULTS],
        n: usize,
        checks: [(&'static str, bool, [u8; 12]); 8],
        nc: usize,
    }

    impl suite::Sink for Results {
        fn result(&mut self, name: &'static str, ticks: u32, iters: u32) {
            if self.n < MAX_RESULTS {
                self.bench[self.n] = (name, ticks, iters);
                self.n += 1;
            }
        }
        fn check(&mut self, name: &'static str, ok: bool, detail: &str) {
            if self.nc < self.checks.len() {
                let mut d = [b' '; 12];
                for (o, b) in d.iter_mut().zip(detail.bytes()) {
                    *o = b;
                }
                self.checks[self.nc] = (name, ok, d);
                self.nc += 1;
            }
        }
    }

    /// Progress lines while the probe runs: if it stops, the photo shows
    /// where.
    struct Progress<'a> {
        uart: &'a mut PxaUart<Mmio>,
        con: Option<Console<'a>>,
    }

    impl Write for Progress<'_> {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            if let Some(c) = self.con.as_mut() {
                let _ = c.write_str(s);
            }
            self.uart.write_str(s)
        }
    }

    fn on_fault(kind: rt::Fault, addr: u32) -> ! {
        crash(format_args!("FAULT {} at {addr:#010x}", kind.name()))
    }

    /// Faults and panics: the message on serial and in a red box on screen.
    /// One copy of this code for both (ADR-0005 rule 8).
    #[inline(never)]
    fn crash(msg: core::fmt::Arguments) -> ! {
        let mut uart = PxaUart::new(rt::emergency_uart());
        let _ = writeln!(uart, "\n{msg}");
        if let Some(vram) = rp_hal_x51v::devices::emergency_vram() {
            let plane = Plane::fullscreen(&X51V_VGA, PixelFormat::Rgb565);
            let mut c = Console::new(
                &mut vram[plane.offset..],
                plane.stride,
                (0, 576),
                (480, 64),
                SCALE,
                (0xFFFF, 0xA800),
            );
            let _ = write!(c, "{msg}");
        }
        halt()
    }

    fn main() -> ! {
        // 1. Capture the hardware exactly as Windows Mobile (and HaRET) left
        // it, before RetroPane changes anything.
        let cp = diag::coprocessors();
        let mut soc = [0u32; diag::COUNT];
        diag::snapshot(&mut soc);

        let Some(bus) = rt::take_ffuart() else { halt() };
        let mut uart = PxaUart::new(bus);
        uart.init(115_200);
        rt::set_fault_handler(on_fault);
        rt::init_mmu();

        // 2. The 2700G: read its state, then find a framebuffer to draw in.
        let mut gpu = [0u32; GPU_REGS.len()];
        let mut display_note = "no 2700G";
        let Some((mut regs, vram)) = rp_hal_x51v::devices::take_mbx() else {
            let _ = writeln!(uart, "probe: no 2700G");
            halt()
        };
        for (v, (_, off)) in gpu.iter_mut().zip(GPU_REGS) {
            *v = regs.read32(*off);
        }
        let kept = Plane::decode(
            regs.read32(reg::GSCTRL),
            regs.read32(reg::GSCADR),
            regs.read32(reg::GSADR),
            MBX_VRAM_SIZE as usize,
        )
        .filter(|p| p.format == PixelFormat::Rgb565 && p.width == 480 && p.height == 640);
        let plane = match kept {
            Some(p) => {
                display_note = "kept WM plane";
                p
            }
            None => {
                let p = Plane::fullscreen(&X51V_VGA, PixelFormat::Rgb565);
                Display::new(regs).enable(&X51V_VGA, &p);
                if display_note == "no 2700G" {
                    display_note = "reprogrammed";
                }
                p
            }
        };
        // Visible screen below `split`, an off-screen buffer for the fill
        // benchmarks above it.
        let screen_end = plane.offset + plane.stride * usize::from(plane.height);
        let split = screen_end.max(0x20_0000).next_multiple_of(0x1_0000);
        let (fb_all, offscreen) = vram.split_at_mut(split);
        let fb = &mut fb_all[plane.offset..screen_end];

        // Scratch RAM: the report's storage at the end, benchmarks before it.
        let Some(scratch) = rp_hal_x51v::devices::take_scratch() else {
            halt()
        };
        let (bench_ram, report_ram) = scratch.split_at_mut(scratch.len() - report::STORAGE);

        let mut results = Results {
            bench: [("", 0, 0); MAX_RESULTS],
            n: 0,
            checks: [("", false, [b' '; 12]); 8],
            nc: 0,
        };
        let mut sd_lines = [[b' '; 37]; 5];
        let mut panel_name = "unknown";

        {
            let mut log = Progress {
                uart: &mut uart,
                con: Some(Console::new(
                    &mut *fb,
                    plane.stride,
                    (0, 0),
                    (480, 640),
                    SCALE,
                    COLORS,
                )),
            };
            let _ = writeln!(log, "RetroPane 26 probe {}", super::BUILD);
            let _ = writeln!(log, "display: {display_note}");

            let mut cpld = rp_hal_x51v::devices::take_cpld().map(Cpld::new);
            if let Some(c) = cpld.as_mut() {
                panel_name = c.panel(&mut TimerDelay).name();
            }
            let _ = writeln!(log, "panel: {panel_name}");

            let _ = writeln!(log, "SD...");
            let mut card = None;
            if let (Some(c), Some(mmc)) = (cpld.as_mut(), rt::take_mmc()) {
                c.set_sd_power(true);
                rt::delay_us(10_000);
                match SdCard::init(PxaMmc::new(mmc)) {
                    Ok(mut k) => {
                        let kind = if k.is_high_capacity() { "SDHC" } else { "SD" };
                        fmt_into(
                            &mut sd_lines[0],
                            format_args!("SD {kind} RCA {:04X}", k.rca()),
                        );
                        let mut block = [0u8; rp_drv_sdcard::BLOCK];
                        if k.read(0, &mut block).is_ok()
                            && let Some(parts) = mbr_partitions(&block)
                        {
                            for (i, p) in parts.iter().enumerate() {
                                if let Some(p) = p {
                                    fmt_into(
                                        &mut sd_lines[1 + i],
                                        format_args!(
                                            "P{} type {:02X} start {} n {}",
                                            i + 1,
                                            p.kind,
                                            p.start,
                                            p.blocks
                                        ),
                                    );
                                }
                            }
                        }
                        card = Some(k);
                    }
                    Err(e) => fmt_into(&mut sd_lines[0], format_args!("SD none ({})", e.name())),
                }
            }

            let _ = writeln!(log, "benchmarks...");
            let mut r = suite::Runner::new(&mut results);
            suite::timer(&mut r);
            suite::frames(&mut r);
            suite::memory(&mut r, bench_ram);
            suite::lz4(&mut r, &mut bench_ram[256 * 1024..]);
            suite::chase(&mut r, "chase_sdram_256k", &mut bench_ram[..256 * 1024]);
            suite::tlb(&mut r, "tlb_fit_16", "tlb_spill_60");
            if let Some(sram) = diag::take_sram() {
                suite::chase(&mut r, "chase_sram_256k", sram);
                // ADR-0006 rule 1: the same TLB test with the page table in
                // on-chip SRAM. If the numbers match, table walks are served
                // from the cache anyway.
                diag::move_l1_table(&mut sram[..16 * 1024]);
                suite::tlb(&mut r, "tlb_fit_16_l1sram", "tlb_spill_60_l1sram");
            }
            if offscreen.len() >= screen_end - plane.offset {
                suite::framebuffer(&mut r, &mut offscreen[..screen_end - plane.offset], &plane);
            }
            if let Some(k) = card.as_mut() {
                suite::sd(&mut r, k);
            }
            let _ = writeln!(log, "done");
        }

        // 3. Build the report.
        let mut rep = Report::new(report_ram);
        rep.line(format_args!("1 SYSTEM  build {}", super::BUILD));
        rep.pairs(&[
            ("CP15 ID", cp.id),
            ("CONTROL", cp.control),
            ("AUXCTRL", cp.aux),
            ("TTB", cp.ttb),
            ("DACR", cp.dacr),
            ("PID", cp.pid),
            ("CPAR", cp.cpar),
            ("CLKCFG", cp.clkcfg),
        ]);
        rep.on_page(1);
        rep.line(format_args!("2 WM STATE (as left)"));
        let mut i = 0;
        for (title, regs) in diag::GROUPS {
            let page = match *title {
                "CLOCKS" | "MEMORY" => 0,
                _ => 1,
            };
            rep.on_page(page);
            rep.line(format_args!("-- {title}"));
            let mut pairs = [("", 0u32); 12];
            for (k, reg) in regs.iter().enumerate() {
                pairs[k] = (reg.name, soc[i]);
                i += 1;
            }
            rep.pairs(&pairs[..regs.len()]);
            if *title == "CLOCKS" {
                let ccsr = soc[3];
                let l = ccsr & 0x1F;
                let n2 = (ccsr >> 7) & 0xF;
                rep.line(format_args!(
                    "run {} MHz turbo {} MHz T{} B{}",
                    13 * l,
                    13 * l * n2 / 2,
                    cp.clkcfg & 1,
                    (cp.clkcfg >> 3) & 1
                ));
            }
        }
        rep.on_page(0);
        rep.line(format_args!("panel {panel_name}"));
        rep.line(format_args!("display {display_note} @{:06X}", plane.offset));
        for l in sd_lines.iter().filter(|l| l.iter().any(|&b| b != b' ')) {
            rep.line(format_args!("{}", Ascii(l)));
        }
        rep.line(format_args!(
            "UART {}",
            if uart.is_alive() { "alive" } else { "dead" }
        ));

        rep.on_page(2);
        rep.line(format_args!("3 2700G (as left by WM)"));
        let mut pairs = [("", 0u32); GPU_REGS.len()];
        for (k, (name, _)) in GPU_REGS.iter().enumerate() {
            pairs[k] = (name, gpu[k]);
        }
        rep.pairs(&pairs);

        rep.on_page(3);
        rep.line(format_args!("4 BENCH ns/iteration"));
        for (name, ticks, iters) in &results.bench[..results.n] {
            // 3.25 MHz ticks: 1 tick = 4000/13 ns. Exact in u32 (no 64-bit
            // division or formatting code in the image): split ticks per
            // iteration into whole and remainder before scaling.
            let it = (*iters).max(1);
            let (q, r) = (ticks / it, ticks % it);
            let ns = q * 4000 / 13 + r * 4000 / 13 / it;
            rep.line(format_args!("{name:<24} {ns:>11}"));
        }
        for (name, ok, detail) in &results.checks[..results.nc] {
            let d = Ascii(detail);
            let v = if *ok { "OK" } else { "FAIL" };
            rep.line(format_args!("check {name} {v} {d}"));
        }
        let lost = rep.overflow;
        if lost > 0 {
            rep.on_page(3);
            rep.line(format_args!("OVERFLOW {lost} lines lost"));
        }

        // Serial copy of every page (the emulator test checks the sums).
        for p in 0..PAGES {
            for l in rep.page_lines(p) {
                let _ = writeln!(uart, "P{}|{}", p + 1, Ascii(l));
            }
        }
        let _ = writeln!(uart, "{}", super::MARKER);

        // 4. Show pages; buttons and taps turn them.
        let mut page = 0;
        let unclaimed = rt::unclaimed_irqs();
        show(&rep, page, fb, plane.stride, unclaimed);
        let (Some(kp), Some(gpio), Some(ssp)) =
            (rt::take_keypad(), rt::take_gpio(), rt::take_ssp1())
        else {
            halt()
        };
        let mut keypad = Keypad::new(kp);
        keypad.enable();
        let gpio = RefCell::new(Gpio::new(gpio));
        gpio.borrow_mut().make_output(TS_CS, true);
        gpio.borrow_mut().make_input(TS_PENIRQ);
        let mut ssp = Ssp::new(ssp);
        ssp.configure_spi(2_600_000);
        let mut touch = Tsc2046::new(
            ssp,
            GpioPin::new(&gpio, TS_CS),
            GpioPin::new(&gpio, TS_PENIRQ),
        );
        rt::start_tick(100);
        let mut last_keys = Pressed::default();
        let mut was_touched = false;
        let mut shown_irqs = unclaimed;
        loop {
            let keys = keypad.pressed();
            let mut step: i32 = 0;
            for (b, down) in last_keys.changes(keys) {
                if down {
                    step = match b {
                        Button::Right | Button::Down | Button::Enter => 1,
                        Button::Left | Button::Up => -1,
                        _ => step,
                    };
                }
            }
            last_keys = keys;
            let touched = touch.sample().is_some();
            if touched && !was_touched {
                step = 1;
            }
            was_touched = touched;
            let irqs = rt::unclaimed_irqs();
            if step != 0 || irqs != shown_irqs {
                page = (page as i32 + step).rem_euclid(PAGES as i32) as usize;
                shown_irqs = irqs;
                show(&rep, page, fb, plane.stride, irqs);
            }
            rt::wait_tick();
        }
    }

    /// Draws page `p` and a footer.
    fn show(rep: &Report, p: usize, fb: &mut [u8], stride: usize, irqs: u64) {
        let mut c = Console::new(fb, stride, (0, 0), (480, 640), SCALE, COLORS);
        for l in rep.page_lines(p) {
            let _ = write!(c, "{}\n", Ascii(l));
        }
        let (_, rows) = c.size();
        for _ in rep.page_lines(p).count()..rows - 1 {
            let _ = c.write_str("\n");
        }
        let _ = write!(
            c,
            "pg {}/{PAGES} <> tap  IRQ {:X}:{:08X}",
            p + 1,
            (irqs >> 32) as u32,
            irqs as u32
        );
    }

    /// Shows ASCII bytes as text without UTF-8 validation (report text is
    /// ASCII by construction; validating it would add code to the image).
    struct Ascii<'a>(&'a [u8]);

    impl core::fmt::Display for Ascii<'_> {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            for &b in self.0 {
                f.write_char(b as char)?;
            }
            Ok(())
        }
    }

    /// Formats into a fixed line buffer (cut if too long).
    fn fmt_into(buf: &mut [u8; 37], args: core::fmt::Arguments) {
        struct W<'a>(&'a mut [u8; 37], usize);
        impl Write for W<'_> {
            fn write_str(&mut self, s: &str) -> core::fmt::Result {
                for b in s.bytes() {
                    if self.1 < self.0.len() {
                        self.0[self.1] = b;
                        self.1 += 1;
                    }
                }
                Ok(())
            }
        }
        let _ = W(buf, 0).write_fmt(args);
    }

    fn halt() -> ! {
        loop {
            core::hint::spin_loop();
        }
    }

    #[panic_handler]
    fn panic(info: &core::panic::PanicInfo) -> ! {
        match info.location() {
            Some(l) => crash(format_args!("PANIC {}:{}", l.file(), l.line())),
            None => crash(format_args!("PANIC")),
        }
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}
