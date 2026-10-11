//! Bare-metal runtime for RetroPane images on the PXA27x.
//!
//! Provides `_start`: the first code that runs when the SD launcher (or QEMU)
//! jumps into a RetroPane image in SDRAM. It puts the CPU in a known state,
//! sets up the stack, clears `.bss` and calls the image's `rp_main`.
//!
//! The image must define `rp_main` with the [`entry!`](crate::entry) macro and
//! link with a script that provides `__stack_top`, `__bss_start` and `__bss_end`.

use core::ptr::NonNull;

use crate::{FFUART_BASE, Mmio, UART_WINDOW};

core::arch::global_asm!(
    r#"
    .section .text.start, "ax"
    .global _start
    .arm
_start:
    @ SVC mode, IRQ and FIQ masked. Whatever Windows Mobile (or QEMU) left
    @ behind, we now own the CPU.
    msr     cpsr_c, #0xD3

    @ Stack.
    ldr     sp, =__stack_top

    @ Zero .bss (word-aligned by the linker script).
    ldr     r0, =__bss_start
    ldr     r1, =__bss_end
    mov     r2, #0
1:  cmp     r0, r1
    strlo   r2, [r0], #4
    blo     1b

    bl      rp_main
2:  b       2b
    "#
);

/// Defines the image's entry point. `$path` must be a `fn() -> !`.
#[macro_export]
macro_rules! entry {
    ($path:path) => {
        #[unsafe(export_name = "rp_main")]
        extern "C" fn __rp_main() -> ! {
            let f: fn() -> ! = $path;
            f()
        }
    };
}

/// Single-use guard for the boot-time FFUART handle.
static mut FFUART_TAKEN: bool = false;

/// Hands out the FFUART register bank once.
///
/// Must be called before interrupts are enabled (it's not reentrant). The PXA27x
/// is single-core, and ARMv5 has no atomics, so a plain flag is sufficient here.
pub fn take_ffuart() -> Option<Mmio> {
    // SAFETY: single core, IRQs masked by `_start` and not yet re-enabled, so
    // nothing can race this read-modify-write of the flag.
    let taken = unsafe { core::ptr::addr_of_mut!(FFUART_TAKEN).replace(true) };
    if taken {
        return None;
    }
    let base = NonNull::new(FFUART_BASE as *mut u32)?;
    // SAFETY: FFUART_BASE..+UART_WINDOW is the FFUART register window (PXA27x
    // memory map), identity-mapped with the MMU off, and the flag above
    // guarantees a single owner.
    Some(unsafe { Mmio::new(base, UART_WINDOW) })
}

/// Reads the CPU's Main ID register (CP15 c0).
#[must_use]
pub fn cpu_id() -> u32 {
    let id: u32;
    // SAFETY: reading the ID register has no side effects and is allowed in
    // any privileged mode, which RetroPane always runs in at this point.
    unsafe {
        core::arch::asm!("mrc p15, 0, {0}, c0, c0, 0", out(reg) id, options(nomem, nostack));
    }
    id
}

/// OS timer counter 0 (OSCR0): free-running at 3.25 MHz after reset.
const OSCR0: usize = 0x40A0_0010;
/// OSCR0 ticks per 4 microseconds (3.25 MHz = 13 ticks / 4 µs).
const OSCR_TICKS_PER_US_X4: u32 = 13;

/// Busy-waits at least `us` microseconds using the OS timer.
pub fn delay_us(us: u32) {
    let read = || {
        // SAFETY: OSCR0 is a read-only-in-effect counter register in the
        // PXA27x OS timer block, identity-mapped with the MMU off; reading it
        // has no side effects.
        unsafe { (OSCR0 as *const u32).read_volatile() }
    };
    let ticks = us.saturating_mul(OSCR_TICKS_PER_US_X4).div_ceil(4);
    let start = read();
    while read().wrapping_sub(start) < ticks {
        core::hint::spin_loop();
    }
}
