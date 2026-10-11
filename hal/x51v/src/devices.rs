//! Safe, single-owner access to X51v board devices for bare-metal images.
//!
//! Like `rp_hal_pxa27x::rt::take_ffuart`, each device can be taken once, before
//! interrupts are enabled, with the MMU off (physical = virtual).

use core::ptr::NonNull;

use rp_hal_pxa27x::Mmio;

use crate::{MBX_REG_OFFSET, MBX_REG_SIZE, MBX_VRAM_SIZE, region};

static mut MBX_TAKEN: bool = false;
static mut CPLD_TAKEN: bool = false;
static mut SCRATCH_TAKEN: bool = false;

/// Hands out the Intel 2700G once: its register block and its local memory.
pub fn take_mbx() -> Option<(Mmio, &'static mut [u8])> {
    // SAFETY: single core, IRQs masked by `_start` and not re-enabled yet, so
    // nothing can race this read-modify-write of the flag.
    let taken = unsafe { core::ptr::addr_of_mut!(MBX_TAKEN).replace(true) };
    if taken {
        return None;
    }
    let cs3 = region("cs3_2700g")?.base;
    let regs = NonNull::new((cs3 + MBX_REG_OFFSET) as *mut u32)?;
    // SAFETY: the register block is a device window on CS3 (docs/hw/2700g.md),
    // identity-mapped with the MMU off, and the flag guarantees one owner.
    let regs = unsafe { Mmio::new(regs, MBX_REG_SIZE as usize) };
    // SAFETY: CS3 starts with the 2700G's 16 MiB local memory, which nothing
    // else in RetroPane references; the flag guarantees this is the only slice.
    let vram = unsafe { core::slice::from_raw_parts_mut(cs3 as *mut u8, MBX_VRAM_SIZE as usize) };
    Some((regs, vram))
}

/// The 2700G's local memory for crash reporting only, once [`take_mbx`] has
/// handed it out (so the display is known to be set up). The real X51v has
/// no serial port, so faults and panics are drawn on screen.
#[must_use]
pub fn emergency_vram() -> Option<&'static mut [u8]> {
    // SAFETY: a plain read of the flag; single core.
    if !unsafe { core::ptr::addr_of!(MBX_TAKEN).read() } {
        return None;
    }
    let cs3 = region("cs3_2700g")?.base;
    // SAFETY: the same 16 MiB of 2700G memory as `take_mbx`. This second
    // slice aliases the owner's, which is acceptable only because it's used
    // on paths that never return to the owner (fault and panic handlers),
    // like `rt::emergency_uart`.
    Some(unsafe { core::slice::from_raw_parts_mut(cs3 as *mut u8, MBX_VRAM_SIZE as usize) })
}

/// Hands out the board CPLD's register window once.
pub fn take_cpld() -> Option<Mmio> {
    // SAFETY: single core, IRQs masked by `_start` and not re-enabled yet.
    let taken = unsafe { core::ptr::addr_of_mut!(CPLD_TAKEN).replace(true) };
    if taken {
        return None;
    }
    let base = NonNull::new(region("cs4_cpld")?.base as *mut u32)?;
    // SAFETY: CS4 holds the CPLD's 0x20-byte register window (docs/hw/cpld.md),
    // identity-mapped with the MMU off; the flag guarantees one owner.
    Some(unsafe { Mmio::new(base, 0x20) })
}

/// Start of the SDRAM scratch region handed out by [`take_scratch`]: 8 MiB into
/// SDRAM, far above bare-metal images (linked at `0xA8008000`, a few tens of KiB).
pub const SCRATCH_BASE: usize = 0xA880_0000;
/// Size of the scratch region.
pub const SCRATCH_SIZE: usize = 1024 * 1024;

/// Hands out 1 MiB of otherwise unused SDRAM once, for buffers in bare-metal
/// images (benchmarks, bring-up) that shouldn't grow the image itself.
pub fn take_scratch() -> Option<&'static mut [u8]> {
    // SAFETY: single core, IRQs masked by `_start` and not re-enabled yet.
    let taken = unsafe { core::ptr::addr_of_mut!(SCRATCH_TAKEN).replace(true) };
    if taken {
        return None;
    }
    // SAFETY: SCRATCH_BASE..+SCRATCH_SIZE is SDRAM (64 MiB from 0xA8000000)
    // that no bare-metal image's sections or stack reach (see boot/link.x),
    // identity-mapped with the MMU off; the flag guarantees one owner.
    Some(unsafe { core::slice::from_raw_parts_mut(SCRATCH_BASE as *mut u8, SCRATCH_SIZE) })
}
