//! Test image (item 69, the MBX bring-up's first step): reads the MBX 3D
//! core's identification registers and returns.
//!
//! The static analysis of Dell's driver (docs/hw/2700g.md) puts the core's
//! 4 KiB register block at `0x0FFE4000`, 16 KiB into the 2700G register
//! window RetroPane already maps; the driver identifies it the ARM PrimeCell
//! way (bytes at `0xff0`-`0xffc` assembling to `0xB105F00D`, a part number
//! at `0xfe0`/`0xfe4`, else a core ID at `0xf00`/`0xf10`). Reading an
//! address the chip doesn't implement hangs its bus (four WM probes froze
//! that way), so each name goes into the run log before its read: if this
//! hangs and the watchdog resets, the log (which survives) names the
//! register. Reads only, the clock as Windows Mobile left it.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed once everything has been read.
pub const MARKER: &str = "RETROPANE-MBX-ID";

/// The MBX block's offset in the 2700G register window (`0x0FFE0000`).
pub const MBX: usize = 0x4000;

/// What is read, in order: the system block's MBX clock first (known to
/// exist), then the core's identification registers.
pub const READS: [(usize, &str); 9] = [
    (0x0054, "MBXCLK (system)"),
    (MBX + 0xff0, "MBX 0xff0"),
    (MBX + 0xff4, "MBX 0xff4"),
    (MBX + 0xff8, "MBX 0xff8"),
    (MBX + 0xffc, "MBX 0xffc"),
    (MBX + 0xfe0, "MBX 0xfe0"),
    (MBX + 0xfe4, "MBX 0xfe4"),
    (MBX + 0xf00, "MBX 0xf00"),
    (MBX + 0xf10, "MBX 0xf10"),
];

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_hal_api::RegisterBus;
    use rp_hal_pxa27x::rt;
    use rp_hal_x51v::devices::RunLog;

    rp_hal_pxa27x::entry!(main);

    fn main() -> ! {
        rt::set_fault_handler(on_fault);
        rt::init_mmu();
        let Some((mut regs, _vram)) = rp_hal_x51v::devices::take_mbx() else {
            crash(format_args!("no 2700G registers"))
        };
        let mut words = [0u32; super::READS.len()];
        for (i, &(off, name)) in super::READS.iter().enumerate() {
            let _ = writeln!(
                RunLog,
                "reading {name} at 0x{:08x}",
                0x0FFE_0000 + off as u32
            );
            words[i] = regs.read32(off);
            let _ = writeln!(RunLog, "  = 0x{:08x}", words[i]);
        }
        // The PrimeCell cell ID: the low byte of each of 0xff0-0xffc.
        let cell = (1..=4).fold(0u32, |id, k| id | (words[k] & 0xFF) << (8 * (k - 1)));
        let _ = writeln!(
            RunLog,
            "{}: PrimeCell cell ID 0x{cell:08x} ({})",
            super::MARKER,
            if cell == 0xB105_F00D {
                "the MBX block is at 0x0FFE4000"
            } else {
                "not the expected 0xB105F00D"
            }
        );
        rp_hal_x51v::devices::return_to_loader();
        loop {
            core::hint::spin_loop();
        }
    }

    fn on_fault(kind: rt::Fault, addr: u32) -> ! {
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
        crash(format_args!("PANIC {}", info.message()))
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}
