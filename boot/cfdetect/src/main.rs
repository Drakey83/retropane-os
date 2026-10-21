//! CompactFlash card detect with the slot unpowered (docs/hw/cf.md, the
//! re-insert finding of 2026-10-16): the loader switches the slot off when
//! the card leaves, and a card pushed back in was then never seen: GPIO13
//! stayed "no card". Which detect signal still works unpowered decides the
//! fix (detect for free, or power the slot briefly to look).
//!
//! 1. The slot as found (powered by Windows Mobile or the loader): GPIO13
//!    (detect, low = card in), GPIO84 (READY / slot power) and CPLD `0x04`
//!    bit 1 (set = no card).
//! 2. The slot switched off, then 30 s of watching every 5 ms while the owner
//!    pulls the card out (first 15 s) and pushes it back in (next 15 s);
//!    every change of any signal is logged with its time.
//! 3. The slot powered again: whether GPIO13 sees the card once it has
//!    power (within 1 s).
//!
//! Nothing else is switched; the slot is left powered.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed once everything was reported.
pub const MARKER: &str = "RETROPANE-CFDETECT-OK";

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_drv_cpld::{Cpld, Delay, handover, reg};
    use rp_drv_mbx_display::Display;
    use rp_drv_mbx_display::console::Console;
    use rp_drv_pxa_gpio::Gpio;
    use rp_hal_pxa27x::{Mmio, rt};
    use rp_hal_x51v::MBX_VRAM_SIZE;
    use rp_hal_x51v::devices;

    rp_image::entry!(main);

    /// The CPLD, with the shadow table every program shares.
    type BoardCpld = Cpld<Mmio, handover::Table<'static>>;

    /// Card detect, low = card in (docs/hw/cf.md).
    const CF_DETECT: u8 = 13;
    /// READY / slot power, high while the slot is powered.
    const CF_READY: u8 = 84;
    /// CPLD `0x04` bit 1: set = no card.
    const CF_ABSENT: u16 = 0x0002;
    /// Sampling period.
    const SAMPLE_US: u32 = 5_000;
    /// Each half of the watch (pull out, push in), in seconds.
    const HALF_S: u32 = 15;
    /// OS timer ticks per millisecond (3.25 MHz).
    const TICKS_PER_MS: u32 = 3_250;
    /// At most this many changes are logged (a bouncing line can't flood).
    const MAX_CHANGES: u32 = 40;

    struct BusyWait;

    impl Delay for BusyWait {
        fn delay_us(&mut self, us: u32) {
            rt::delay_us(us);
        }
    }

    /// One reading of the three signals: GPIO13, GPIO84, CPLD `0x04` bit 1.
    type Sample = (bool, bool, bool);

    fn sample(gpio: &mut Gpio<Mmio>, cpld: &mut BoardCpld) -> Sample {
        (
            gpio.read(CF_DETECT),
            gpio.read(CF_READY),
            cpld.read(reg::STATUS04) & CF_ABSENT != 0,
        )
    }

    fn card(absent: bool) -> &'static str {
        if absent { "no card" } else { "card" }
    }

    fn show(out: &mut impl Write, at_ms: u32, what: &str, s: Sample) {
        let _ = writeln!(
            out,
            "{at_ms:>6} ms {what}: GPIO13 {} ({}), GPIO84 {}, CPLD04 b1 {} ({})",
            u8::from(s.0),
            card(s.0),
            u8::from(s.1),
            u8::from(s.2),
            card(s.2),
        );
    }

    fn ms_since(start: u32) -> u32 {
        rt::ticks().wrapping_sub(start) / TICKS_PER_MS
    }

    fn main() -> ! {
        rt::init_mmu();
        let Some((regs, vram)) = rp_image::take_mbx() else {
            rp_image::fail("no display")
        };
        let (plane, _) = Display::new(regs).adopt(MBX_VRAM_SIZE as usize);
        let end = plane.offset + plane.stride * usize::from(plane.height);
        let mut con = Console::new(
            &mut vram[plane.offset..end],
            plane.stride,
            (0, 0),
            (480, 640),
            2,
            (0xFFFF, 0x0000),
        );
        let mut out = Tee(&mut con);
        let _ = writeln!(out, "RetroPane 26 CF detect");
        let (Some(gpio_bus), Some((bus, table))) = (rt::take_gpio(), devices::take_cpld()) else {
            let _ = writeln!(out, "GPIO or CPLD unavailable");
            rp_image::fail("GPIO or CPLD unavailable")
        };
        let mut gpio = Gpio::new(gpio_bus);
        gpio.make_input(CF_DETECT);
        gpio.make_input(CF_READY);
        let (mut cpld, _) = Cpld::adopt(bus, table);
        let start = rt::ticks();

        // 1. As found.
        let s = sample(&mut gpio, &mut cpld);
        show(&mut out, ms_since(start), "as found", s);

        // 2. Slot off, then watch a pull-out and a push-in.
        cpld.set_cf_power(false, &mut BusyWait);
        rt::delay_us(100_000);
        let mut last = sample(&mut gpio, &mut cpld);
        show(&mut out, ms_since(start), "slot off", last);
        let _ = writeln!(out, "PULL THE CF CARD OUT NOW ({HALF_S} s)");
        let mut asked_in = false;
        let watch = rt::ticks();
        let mut changes = 0u32;
        while ms_since(watch) < 2 * HALF_S * 1000 {
            if !asked_in && ms_since(watch) >= HALF_S * 1000 {
                let _ = writeln!(out, "PUSH IT BACK IN NOW ({HALF_S} s)");
                asked_in = true;
            }
            let s = sample(&mut gpio, &mut cpld);
            if s != last && changes < MAX_CHANGES {
                show(&mut out, ms_since(start), "change", s);
                changes += 1;
            }
            last = s;
            // Still working: the loader's run limit and watchdog.
            rt::check_in();
            rt::delay_us(SAMPLE_US);
        }
        show(&mut out, ms_since(start), "end, slot off", last);

        // 3. Power back on: does GPIO13 see the card once powered?
        cpld.set_cf_power(true, &mut BusyWait);
        let on_at = rt::ticks();
        let mut s = sample(&mut gpio, &mut cpld);
        while ms_since(on_at) < 1000 {
            let n = sample(&mut gpio, &mut cpld);
            if n != s {
                show(&mut out, ms_since(start), "powered, change", n);
                s = n;
            }
            rt::delay_us(SAMPLE_US);
        }
        show(&mut out, ms_since(start), "powered 1 s", s);
        let _ = writeln!(out, "changes logged: {changes}");
        finish()
    }

    fn finish() -> ! {
        let _ = writeln!(devices::RunLog, "{}", super::MARKER);
        if devices::loader_present() {
            rt::delay_us(3_000_000);
        }
        rp_image::finish()
    }

    /// Writes to the screen and the run log.
    struct Tee<'a, 'b>(&'a mut Console<'b>);

    impl Write for Tee<'_, '_> {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let _ = self.0.write_str(s);
            devices::log_write(s.as_bytes());
            Ok(())
        }
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}
