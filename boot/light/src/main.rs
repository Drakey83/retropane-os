//! RetroPane first-light image: where does startup stop on the real X51v?
//!
//! The probe ran on the device but drew nothing after HaRET's "Turning off
//! MMU...", and the X51v has no serial port. This image repeats the probe's
//! startup, step for step, and after each step fills the whole screen with a
//! colour ([`rp_hal_x51v::devices::beacon`] paints all of the 2700G's memory,
//! so it shows whatever plane the display is scanning out). The colour left
//! on screen says how far it got:
//!
//! | Colour  | Reached                                               |
//! |---------|-------------------------------------------------------|
//! | red     | `main` (MMU off, HaRET's handoff worked)              |
//! | yellow  | SoC state snapshot                                    |
//! | green   | FFUART clocked, set up and written                    |
//! | cyan    | fault handler + MMU on                                |
//! | blue    | 2700G taken, its registers read                       |
//! | white   | display plane chosen, scratch RAM taken: text follows |
//!
//! A fault paints stripes: magenta alternating with the colour of the last
//! step reached. On success, black text on white lists what it found.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed once every step passed.
pub const MARKER: &str = "RETROPANE-LIGHT-OK";

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_drv_cpld::{Cpld, Delay};
    use rp_drv_mbx_display::console::Console;
    use rp_drv_mbx_display::{Display, crash, reg};
    use rp_drv_pxa_uart::PxaUart;
    use rp_hal_api::RegisterBus;
    use rp_hal_pxa27x::diag;
    use rp_hal_pxa27x::rt::{self, Global};
    use rp_hal_x51v::MBX_VRAM_SIZE;
    use rp_hal_x51v::devices::beacon;

    rp_hal_pxa27x::entry!(main);

    // Step colours (RGB565).
    const RED: u16 = 0xF800;
    const YELLOW: u16 = 0xFFE0;
    const GREEN: u16 = 0x07E0;
    const CYAN: u16 = 0x07FF;
    const BLUE: u16 = 0x001F;
    const WHITE: u16 = 0xFFFF;
    const MAGENTA: u16 = 0xF81F;

    /// The last step's colour, for the fault stripes.
    static STEP: Global<u16> = Global::new(0);

    fn step(colour: u16) {
        STEP.with(|s| *s = colour);
        beacon(colour, colour);
    }

    struct TimerDelay;
    impl Delay for TimerDelay {
        fn delay_us(&mut self, us: u32) {
            rt::delay_us(us);
        }
    }

    fn on_fault(kind: rt::Fault, addr: u32) -> ! {
        crash(format_args!("FAULT {} at {addr:#010x}", kind.name()))
    }

    /// Stripes, then the message on serial and (if the 2700G was taken) as
    /// text at the top of a full-screen plane.
    #[inline(never)]
    fn crash(msg: core::fmt::Arguments) -> ! {
        let last = STEP.with(|s| *s);
        beacon(MAGENTA, last);
        let mut uart = PxaUart::new(rt::emergency_uart());
        let _ = writeln!(uart, "\n{msg}");
        if let Some((regs, vram)) = rp_hal_x51v::devices::emergency_mbx() {
            let mut c =
                crash::emergency_console(regs, vram, (0, 0), (480, 64), 2, (0xFFFF, 0x0000));
            let _ = write!(c, "{msg}");
        }
        // Started by the resident loader: the message goes to the PC too.
        if rp_hal_x51v::devices::loader_present() {
            rt::delay_us(3_000_000); // time to read the screen
        }
        rp_hal_x51v::devices::crash_to_loader(msg);
        halt()
    }

    fn main() -> ! {
        step(RED);

        // The probe's order exactly: state first, before anything changes it.
        let cp = diag::coprocessors();
        let mut soc = [0u32; diag::COUNT];
        diag::snapshot(&mut soc);
        step(YELLOW);

        let Some(bus) = rt::take_ffuart() else { halt() };
        let mut uart = PxaUart::new(bus);
        uart.init(115_200);
        let _ = writeln!(uart, "\nRetroPane 26 first light");
        step(GREEN);

        rt::set_fault_handler(on_fault);
        rt::init_mmu();
        step(CYAN);

        let Some((mut regs, vram)) = rp_hal_x51v::devices::take_mbx() else {
            halt()
        };
        let (gsctrl, gscadr, gsadr) = (
            regs.read32(reg::GSCTRL),
            regs.read32(reg::GSCADR),
            regs.read32(reg::GSADR),
        );
        step(BLUE);

        let (plane, kept) = Display::new(regs).adopt(MBX_VRAM_SIZE as usize);
        let note = if kept {
            "kept WM plane"
        } else {
            "reprogrammed"
        };
        let scratch = rp_hal_x51v::devices::take_scratch().is_some();
        step(WHITE);

        let end = plane.offset + plane.stride * usize::from(plane.height);
        let mut con = Console::new(
            &mut vram[plane.offset..end],
            plane.stride,
            (0, 0),
            (480, 640),
            2,
            (0x0000, WHITE),
        );
        let panel = rp_hal_x51v::devices::take_cpld()
            .map(|(b, t)| Cpld::adopt(b, t).0)
            .map_or("no CPLD", |mut c| c.panel(&mut TimerDelay).name());
        let mut out = Tee(&mut uart, &mut con);
        let _ = writeln!(out, "RetroPane 26 first light");
        let _ = writeln!(out, "all steps OK");
        let _ = writeln!(out, "display: {note} @{:06X}", plane.offset);
        if let Some(h) = rp_hal_x51v::devices::mbx_handoff() {
            let _ = writeln!(
                out,
                "2700G at {} us; DMA {:08X} stop {} us",
                h.at * 4 / 13,
                h.found.dma_running,
                h.stopped.waited * 4 / 13
            );
        }
        let _ = writeln!(out, "GSCTRL  {gsctrl:08X}");
        let _ = writeln!(out, "GSCADR  {gscadr:08X}");
        let _ = writeln!(out, "GSADR   {gsadr:08X}");
        let _ = writeln!(out, "panel: {panel}");
        let _ = writeln!(out, "scratch: {}", if scratch { "ok" } else { "MISSING" });
        let _ = writeln!(out, "CPU ID  {:08X}", rt::cpu_id());
        let _ = writeln!(out, "CP15 c1 {:08X}", cp.control);
        let _ = writeln!(
            out,
            "serial: {}",
            if out.0.is_alive() { "alive" } else { "dead" }
        );
        let _ = writeln!(out, "\nSoft-reset to go back.");
        let _ = writeln!(out.0, "{}", super::MARKER);
        let _ = writeln!(rp_hal_x51v::devices::RunLog, "{}", super::MARKER);
        let _ = soc;
        // Started by the resident loader: show the screen briefly, then go
        // back so it can send the run log to the PC.
        if rp_hal_x51v::devices::loader_present() {
            rt::delay_us(2_000_000);
            rp_hal_x51v::devices::return_to_loader();
        }
        halt()
    }

    /// Writes to the UART and the screen.
    struct Tee<'a, 'b>(&'a mut PxaUart<rp_hal_pxa27x::Mmio>, &'a mut Console<'b>);

    impl Write for Tee<'_, '_> {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let _ = self.1.write_str(s);
            rp_hal_x51v::devices::log_write(s.as_bytes());
            self.0.write_str(s)
        }
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
