//! RetroPane hand-over diagnostics (items 72 and 77): what a program finds
//! when it takes the X51v over, measured instead of assumed.
//!
//! 1. What `_start` found running (DMA channels, the UDC), how long those
//!    channels took to stop, and when the 2700G was handed out: the fixed
//!    500 ms "settle" this replaced (`devices::take_mbx`).
//! 2. The CPLD's shared shadow table at `0xA8001320` as found, before this
//!    program adopts it: Windows Mobile's own (its ten slots) until a
//!    RetroPane program has adopted it, RetroPane's after. Then what the
//!    adoption decided and the shadow it gave.
//! 3. The LCD-type readout (`0x1E`): the raw value with its enable off, at
//!    once after the enable, and after the waits Dell (1 ms) and AxDroid
//!    (1 ms twice) use.
//!
//! Nothing is switched: the only CPLD writes are the readout's enable on and
//! off, through the adopted shadow (so `0x1E`'s other bits are kept).

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed once everything was reported.
pub const MARKER: &str = "RETROPANE-HANDOVER-OK";

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_drv_cpld::{Cpld, handover, reg};
    use rp_drv_mbx_display::console::Console;
    use rp_drv_mbx_display::{Display, crash};
    use rp_hal_pxa27x::rt;
    use rp_hal_x51v::MBX_VRAM_SIZE;
    use rp_hal_x51v::devices;

    rp_hal_pxa27x::entry!(main);

    fn on_fault(kind: rt::Fault, addr: u32) -> ! {
        crash_out(format_args!("FAULT {} at {addr:#010x}", kind.name()))
    }

    #[inline(never)]
    fn crash_out(msg: core::fmt::Arguments) -> ! {
        if let Some((regs, vram)) = devices::emergency_mbx() {
            let mut c =
                crash::emergency_console(regs, vram, (0, 0), (480, 64), 2, (0xFFFF, 0x0000));
            let _ = write!(c, "{msg}");
        }
        devices::crash_to_loader(msg);
        halt()
    }

    /// Microseconds from OS timer ticks.
    fn us(ticks: u32) -> u32 {
        ticks * 4 / 13
    }

    fn main() -> ! {
        rt::set_fault_handler(on_fault);
        rt::init_mmu();
        let Some((regs, vram)) = devices::take_mbx() else {
            halt()
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
        let _ = writeln!(out, "RetroPane 26 hand-over");

        // 1. The hand-off.
        if let Some(h) = devices::mbx_handoff() {
            let _ = writeln!(
                out,
                "handoff: DMA running {:08X}, stopped in {} us (stuck {:08X}), 2700G at {} us",
                h.found.dma_running,
                us(h.stopped.waited),
                h.stopped.stuck,
                us(h.at),
            );
        }

        // 2. The shadow table, raw, then adopted.
        let Some((bus, table)) = devices::take_cpld() else {
            let _ = writeln!(out, "CPLD unavailable");
            finish()
        };
        let _ = write!(out, "table @{:08X}:", handover::TABLE_PA);
        for w in table.iter() {
            let _ = write!(out, " {w:08X}");
        }
        let _ = writeln!(out);
        let (mut cpld, source) = Cpld::adopt(bus, table);
        let _ = write!(out, "shadow from {}:", source.name());
        for k in 0..rp_drv_cpld::REGS {
            let _ = write!(out, " {:02X}={:04X}", k * 2, cpld.shadow(k * 2));
        }
        let _ = writeln!(out);

        // 3. The LCD-type readout: off, at once, after 1 ms, after 2 ms.
        let off = cpld.read(reg::LCD_ID);
        let mut reads = [0u16; 3];
        for (r, wait) in reads.iter_mut().zip([0, 1000, 2000]) {
            cpld.set(reg::LCD_ID, 0x0008);
            rt::delay_us(wait);
            *r = cpld.read(reg::LCD_ID);
            cpld.clear(reg::LCD_ID, 0x0008);
            rt::delay_us(5000);
        }
        let _ = writeln!(
            out,
            "LCD id: enable off {off:04X}; on, read at 0 us {:04X}, 1 ms {:04X}, 2 ms {:04X}",
            reads[0], reads[1], reads[2]
        );
        finish()
    }

    fn finish() -> ! {
        let _ = writeln!(devices::RunLog, "{}", super::MARKER);
        if devices::loader_present() {
            rt::delay_us(3_000_000);
            devices::return_to_loader();
        }
        halt()
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

    fn halt() -> ! {
        loop {
            core::hint::spin_loop();
        }
    }

    #[panic_handler]
    fn panic(info: &core::panic::PanicInfo) -> ! {
        match info.location() {
            Some(l) => crash_out(format_args!("PANIC {}:{}", l.file(), l.line())),
            None => crash_out(format_args!("PANIC")),
        }
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}
