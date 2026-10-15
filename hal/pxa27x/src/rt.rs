//! Bare-metal runtime for RetroPane images on the PXA27x.
//!
//! Provides `_start`: the first code that runs when the SD launcher (or QEMU)
//! jumps into a RetroPane image in SDRAM. It puts the CPU in a known state,
//! sets up the stack, clears `.bss` and calls the image's `rp_main`.
//!
//! The image must define `rp_main` with the [`entry!`](crate::entry) macro and
//! link with a script that provides `__stack_top`, `__irq_stack_top`,
//! `__exc_stack_top`, `__bss_start` and `__bss_end` (see `boot/link.x`).
//!
//! It also provides the bring-up MMU ([`init_mmu`]: caches, write buffer,
//! high vectors), exception handling and a periodic tick with idle
//! ([`start_tick`], [`wait_tick`]), so images never busy-wait.

use core::ptr::NonNull;

use rp_hal_api::RegisterBus;

use crate::devmap::va;
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

    @ ...and the memory: stop what Windows Mobile left running that writes
    @ RAM by itself, before we use any of it. All 32 DMA channels (DCSRn:
    @ RUN off), and the USB device controller (its driver's transfers).
    @ On the real X51v, images hung at random early points only while the
    @ Axim was docked with USB active.
    @ The channels that were running are kept for `rt::handoff` (r4),
    @ read only with the DMA clock on (r5: CKEN as found; without it a
    @ channel can't run). The UDC is only stopped, not read: a read of a
    @ block that isn't there faults, and nothing catches a fault yet (the
    @ emulator has no UDC unless asked for one). The writes stay
    @ unconditional, as before.
    ldr     r0, =0x41300004
    ldr     r5, [r0]
    mov     r4, #0
    ldr     r0, =0x40000000
    mov     r1, #0
    mov     r2, #1
0:  tst     r5, #0x01000000
    beq     4f
    ldr     r3, [r0]
    tst     r3, #0x80000000
    orrne   r4, r4, r2
4:  str     r1, [r0], #4
    movs    r2, r2, lsl #1
    bne     0b
    ldr     r0, =0x40600000
    str     r1, [r0]

    @ Stacks: IRQ, abort (its own when the image sizes one: the kernel
    @ resolves page faults there), undefined, then SVC, where we stay.
    @ Interrupts stay masked throughout.
    msr     cpsr_c, #0xD2
    ldr     sp, =__irq_stack_top
    msr     cpsr_c, #0xD7
    ldr     sp, =__abt_stack_top
    msr     cpsr_c, #0xDB
    ldr     sp, =__exc_stack_top
    @ FIQ: the soft run limit only (terminal: back to the loader), so it
    @ can share the exception stack.
    msr     cpsr_c, #0xD1
    ldr     sp, =__exc_stack_top
    msr     cpsr_c, #0xD3
    ldr     sp, =__stack_top

    @ Zero .bss (word-aligned by the linker script).
    ldr     r0, =__bss_start
    ldr     r1, =__bss_end
    mov     r2, #0
1:  cmp     r0, r1
    strlo   r2, [r0], #4
    blo     1b
    ldr     r0, =rp_handoff_dma
    str     r4, [r0]

    @ Paint the SVC stack below us, so `stack_used` can find how deep it
    @ went (a silent overflow into .bss lost a glyph atlas, 2026-10-14).
    ldr     r0, =__stack_bottom
    ldr     r1, =0x5741434B
    sub     r3, sp, #16
3:  cmp     r0, r3
    strlo   r1, [r0], #4
    blo     3b

    bl      rp_main
2:  b       2b

    @ Exception vectors, copied to the high-vector page by init_mmu. Each slot
    @ loads its handler's absolute address from the word 32 bytes later.
    .global rp_vectors
    .balign 4
rp_vectors:
    ldr     pc, [pc, #24]
    ldr     pc, [pc, #24]
    ldr     pc, [pc, #24]
    ldr     pc, [pc, #24]
    ldr     pc, [pc, #24]
    ldr     pc, [pc, #24]
    ldr     pc, [pc, #24]
    ldr     pc, [pc, #24]
    .word   _start, rp_undef_entry, rp_swi_fault, rp_pabt_entry
    .word   rp_dabt_entry, rp_undef_entry, rp_irq_entry, rp_fiq_entry

    @ FIQ: only the soft run limit is routed here (`arm_run_limit`). It
    @ never returns: the program goes back to its starter.
rp_fiq_entry:
    bl      rp_fiq

    @ IRQ: move straight to SVC mode and save a full frame on the
    @ interrupted context's SVC stack (a thread's kernel stack), so the
    @ kernel can switch threads before returning to user mode (ADR-0010).
    @ The IRQ stack only holds 3 scratch words. Frame (18 words): r0-r12,
    @ user sp, user lr, svc lr, pc, cpsr.
rp_irq_entry:
    sub     lr, lr, #4
    stmdb   sp, {{r0, lr}}              @ scratch: [sp-8] = r0, [sp-4] = pc
    mrs     lr, spsr
    str     lr, [sp, #-12]              @ scratch: [sp-12] = cpsr
    sub     r0, sp, #12
    msr     cpsr_c, #0x93               @ SVC, IRQ masked (FIQ open: run limit)
    sub     sp, sp, #72
    stmib   sp, {{r1-r12}}              @ r1-r12 at +4..+48
    ldmia   r0, {{r1, r2, r3}}          @ cpsr, r0, pc
    str     r2, [sp]
    str     lr, [sp, #60]               @ svc lr (live if the kernel was interrupted)
    str     r3, [sp, #64]
    str     r1, [sp, #68]
    add     r0, sp, #52
    stmia   r0, {{sp, lr}}^             @ user sp and lr
    nop
    mov     r0, sp
    bl      rp_irq_frame
    ldr     r0, [sp, #68]
    msr     spsr_cxsf, r0
    add     r0, sp, #52
    ldmia   r0, {{sp, lr}}^
    nop
    add     lr, sp, #60                 @ lr -> {{svc lr, pc}}
    ldmia   sp, {{r0-r12}}
    add     sp, sp, #72                 @ IRQs are masked: the frame can't be overwritten
    ldmia   lr, {{lr, pc}}^             @ svc lr, pc; CPSR = SPSR

    @ Faults: report kind and faulting address, then stop.
    @ Undefined instruction: report and stop. (Lazy iWMMXt switching patches
    @ its own entry into the vector page, iwmmxt.rs, and falls back here.)
    .global rp_undef_entry
rp_undef_entry:
    mrs     r0, spsr
    ldr     r1, =rp_fault_spsr
    str     r0, [r1]
    mov     r0, #1
    sub     r1, lr, #4
    b       rp_fault
    @ SWI with no system-call handler: report and stop. user.rs patches the
    @ real entry (rp_swi_entry) into the vector page when a handler is set,
    @ so images without system calls don't link it.
rp_swi_fault:
    mov     r0, #2
    sub     r1, lr, #4
    b       rp_fault
    @ Aborts: from user mode, the user-abort entry (user.rs, installed at
    @ run time through rp_user_abort_entry) takes over with r0 = kind and
    @ the six words just pushed: the fault is handled in the thread's own
    @ context (ADR-0010 rule 5). Otherwise ask rp_abort: it returns where
    @ to resume (the faulting instruction for a resolved page fault, a
    @ try_read / try_write fixup) or 0 to report the fault and stop.
rp_pabt_entry:
    sub     lr, lr, #4
    stmfd   sp!, {{r0-r3, r12, lr}}
    mov     r0, #3
    b       2f
rp_dabt_entry:
    sub     lr, lr, #8
    stmfd   sp!, {{r0-r3, r12, lr}}
    mov     r0, #4
2:  mrs     r1, spsr
    and     r1, r1, #0x1F
    cmp     r1, #0x10
    bne     1f
    ldr     r1, =rp_user_abort_entry
    ldr     r1, [r1]
    cmp     r1, #0
    bxne    r1
1:  str     r0, [sp, #-4]!              @ keep the kind across the call
    mov     r1, lr
    bl      rp_abort
    ldr     r12, [sp], #4
    cmp     r0, #0
    strne   r0, [sp, #20]
    ldmfdne sp!, {{r0-r3, r12, pc}}^
    mov     r0, r12                     @ unresolved: report and stop
    ldr     r1, [sp, #20]               @ the faulting pc
    b       rp_fault
    .ltorg
    "#
);

/// Where an abort taken in user mode goes (0: like any other abort). Set by
/// `user::set_user_fault_handler`, so images without user mode don't link
/// the user-abort entry.
#[unsafe(export_name = "rp_user_abort_entry")]
static mut USER_ABORT_ENTRY: u32 = 0;

/// Routes aborts from user mode to `entry` (see `USER_ABORT_ENTRY`).
pub(crate) fn set_user_abort_entry(entry: u32) {
    // SAFETY: a plain word read only by the abort entries; set during setup.
    unsafe { core::ptr::addr_of_mut!(USER_ABORT_ENTRY).write_volatile(entry) }
}

/// Defines the image's entry point. `$path` must be a `fn() -> !`. The
/// moment it starts is recorded for [`since_start`] (boot-time budgets).
#[macro_export]
macro_rules! entry {
    ($path:path) => {
        #[unsafe(export_name = "rp_main")]
        extern "C" fn __rp_main() -> ! {
            $crate::rt::mark_start();
            let f: fn() -> ! = $path;
            f()
        }
    };
}

/// The OS timer when the image's code started.
static mut START: u32 = 0;

/// Time since the image started, past the OS timer's wrap ([`uptime_ms`]).
static mut UPTIME: crate::elapsed::Elapsed = crate::elapsed::Elapsed::starting_at(0);

/// DMA channels found running at the image's first instruction (one bit
/// each), recorded by `_start` before it stopped them.
#[unsafe(export_name = "rp_handoff_dma")]
static mut HANDOFF_DMA: u32 = 0;

/// What the previous owner of the CPU (Windows Mobile through HaRET, or a
/// RetroPane image through the loader) left running that writes memory by
/// itself. `_start` stops all of it before anything else; this says what
/// it found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Handoff {
    /// DMA channels that were running (bit n = channel n).
    pub dma_running: u32,
}

/// What `_start` found running (see [`Handoff`]).
#[must_use]
pub fn handoff() -> Handoff {
    // SAFETY: plain words written once by `_start` before `rp_main`.
    unsafe {
        Handoff {
            dma_running: core::ptr::addr_of!(HANDOFF_DMA).read(),
        }
    }
}

/// How waiting for the hand-off's DMA to stop went ([`wait_handoff_dma`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DmaStopped {
    /// OS timer ticks spent waiting.
    pub waited: u32,
    /// Channels that still hadn't stopped when the wait gave up.
    pub stuck: u32,
}

/// DMA channel status registers (`DCSRn`, 4 bytes apart).
const DCSR0: usize = 0x4000_0000;
/// `DCSR.STOPINTR`: the channel is stopped (also set after reset).
const DCSR_STOPPED: u32 = 1 << 3;

/// Waits until every DMA channel `_start` found running reports stopped,
/// at most `timeout_us`. Clearing `RUN` doesn't stop a channel at once: it
/// finishes the transfer in flight first. This is the condition the first
/// reads of a shared bus device (the 2700G) wait for after the hand-off.
pub fn wait_handoff_dma(timeout_us: u32) -> DmaStopped {
    let running = handoff().dma_running;
    let start = ticks();
    let limit = timeout_us.saturating_mul(OSCR_TICKS_PER_US_X4).div_ceil(4);
    let mut stuck = running;
    while stuck != 0 {
        let mut left = 0;
        for ch in 0..32 {
            if stuck & (1 << ch) != 0 {
                // SAFETY: DCSRn of a channel that was running, so the DMA
                // clock is on (`_start` read it with CKEN checked); reading
                // a status register has no side effects.
                let s = unsafe { (va(DCSR0 + ch * 4) as *const u32).read_volatile() };
                if s & DCSR_STOPPED == 0 {
                    left |= 1 << ch;
                }
            }
        }
        stuck = left;
        if stuck == 0 || ticks().wrapping_sub(start) >= limit {
            break;
        }
    }
    DmaStopped {
        waited: ticks().wrapping_sub(start),
        stuck,
    }
}

/// Records now as the image's start (called by [`entry!`]).
#[doc(hidden)]
pub fn mark_start() {
    // SAFETY: written once, before anything else runs.
    unsafe {
        let now = ticks();
        core::ptr::addr_of_mut!(START).write(now);
        core::ptr::addr_of_mut!(UPTIME).write(crate::elapsed::Elapsed::starting_at(now));
    }
}

/// Milliseconds since the image started, counting smoothly through the OS
/// timer's 22-minute wrap and wrapping only at 2^32 ms (49.7 days), so
/// `uptime_ms().wrapping_sub(then)` is right for any span under that. For
/// a long-running program's deadlines and windows: `ticks() / 3250` drops
/// back to 0 every 22 minutes instead. It must be called at least once per
/// 22 minutes (a main loop calls it every pass); not from interrupt handlers.
#[must_use]
#[inline(never)]
pub fn uptime_ms() -> u32 {
    // SAFETY: single-threaded main-line use only (documented); the value is
    // a plain struct updated in place.
    let ms = unsafe { (*core::ptr::addr_of_mut!(UPTIME)).ms_at(ticks()) };
    // Keeping the low 32 bits is the wrap described above.
    ms as u32
}

/// OS timer ticks (3.25 MHz) since the image's code started: boot time.
#[must_use]
pub fn since_start() -> u32 {
    // SAFETY: written once at entry; read-only after.
    ticks().wrapping_sub(unsafe { core::ptr::addr_of!(START).read() })
}

/// Single-use guard for the boot-time FFUART handle.
static mut FFUART_TAKEN: bool = false;
/// Single-use guard for the keypad controller.
static mut KEYPAD_TAKEN: bool = false;
/// Single-use guard for the GPIO controller.
static mut GPIO_TAKEN: bool = false;
/// Single-use guard for SSP1.
static mut SSP1_TAKEN: bool = false;
/// Single-use guard for the MMC controller.
static mut MMC_TAKEN: bool = false;
/// Single-use guard for PWM0.
static mut PWM0_TAKEN: bool = false;
/// Single-use guards for the DMA channels (one bit each).
static mut DMA_TAKEN: u32 = 0;

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
    // Windows Mobile may have left the FFUART's clock off (the X51v has no
    // serial port fitted), and an unclocked UART never reports TX space.
    enable_clock(CKEN_FFUART);
    let base = NonNull::new(va(FFUART_BASE) as *mut u32)?;
    // SAFETY: FFUART_BASE..+UART_WINDOW is the FFUART register window (PXA27x
    // memory map), identity-mapped with the MMU off, and the flag above
    // guarantees a single owner.
    Some(unsafe { Mmio::new(base, UART_WINDOW) })
}

/// An FFUART handle for fault and panic reporting only, usable even after
/// [`take_ffuart`] handed out the normal one. Output may interleave with the
/// owner's; that's acceptable when the system is stopping anyway.
#[must_use]
pub fn emergency_uart() -> Mmio {
    // SAFETY: the FFUART window is valid MMIO; this is only used on paths that
    // never return to the code holding the normal handle (faults, panics).
    unsafe {
        Mmio::new(
            NonNull::new_unchecked(va(FFUART_BASE) as *mut u32),
            UART_WINDOW,
        )
    }
}

/// Reads the CPU's Main ID register (CP15 c0).
#[must_use]
#[cfg_attr(target_arch = "arm", instruction_set(arm::a32))]
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

/// OS timer frequency.
pub const TICK_HZ: u32 = crate::elapsed::TICK_HZ;

pub use crate::elapsed::Elapsed;

impl Elapsed {
    /// Starts counting now. For any span that can exceed a few minutes:
    /// `ticks().wrapping_sub(t0)` wraps after 22 minutes (see [`crate::elapsed`]).
    #[must_use]
    pub fn start() -> Self {
        Self::starting_at(ticks())
    }

    /// Milliseconds since [`Elapsed::start`]; read at least once per 22 minutes.
    pub fn ms(&mut self) -> u64 {
        self.ms_at(ticks())
    }
}

/// The free-running OS timer (3.25 MHz), for timing and benchmarks.
#[inline]
#[must_use]
pub fn ticks() -> u32 {
    // SAFETY: OSCR0 is a counter register in the PXA27x OS timer block,
    // identity-mapped with the MMU off; reading it has no side effects.
    unsafe { (va(OSCR0) as *const u32).read_volatile() }
}

/// Busy-waits at least `us` microseconds using the OS timer.
pub fn delay_us(us: u32) {
    let wait = us.saturating_mul(OSCR_TICKS_PER_US_X4).div_ceil(4);
    let start = ticks();
    while ticks().wrapping_sub(start) < wait {
        core::hint::spin_loop();
    }
}

/// Clock manager: clock enable register (CKEN).
const CKEN: usize = 0x4130_0004;
/// CKEN bit for the keypad controller.
pub const CKEN_KEYPAD: u32 = 19;
/// Keypad controller base.
const KEYPAD_BASE: usize = 0x4150_0000;

/// `CKEN` bit for the FFUART.
pub const CKEN_FFUART: u32 = 6;

/// Enables a peripheral clock (`CKEN` bit `bit`).
pub fn enable_clock(bit: u32) {
    let cken = va(CKEN) as *mut u32;
    // SAFETY: CKEN is the clock manager's clock enable register, identity-
    // mapped with the MMU off; single core with IRQs masked, so this
    // read-modify-write can't race.
    unsafe { cken.write_volatile(cken.read_volatile() | (1 << bit)) }
}

/// Hands out the keypad controller once, with its clock enabled.
pub fn take_keypad() -> Option<Mmio> {
    // SAFETY: single core, IRQs masked by `_start` and not yet re-enabled.
    let taken = unsafe { core::ptr::addr_of_mut!(KEYPAD_TAKEN).replace(true) };
    if taken {
        return None;
    }
    enable_clock(CKEN_KEYPAD);
    let base = NonNull::new(va(KEYPAD_BASE) as *mut u32)?;
    // SAFETY: the keypad controller's register window (PXA27x memory map,
    // 0x4C bytes used), identity-mapped; the flag guarantees one owner.
    Some(unsafe { Mmio::new(base, 0x100) })
}

/// CKEN bit for SSP1.
pub const CKEN_SSP1: u32 = 23;

/// Hands out the GPIO controller once.
pub fn take_gpio() -> Option<Mmio> {
    // SAFETY: single core, IRQs masked by `_start` and not yet re-enabled.
    let taken = unsafe { core::ptr::addr_of_mut!(GPIO_TAKEN).replace(true) };
    if taken {
        return None;
    }
    let base = NonNull::new(va(0x40E0_0000) as *mut u32)?;
    // SAFETY: the GPIO controller's register window (PXA27x memory map, up to
    // 0x130 used), identity-mapped; the flag guarantees one owner.
    Some(unsafe { Mmio::new(base, 0x200) })
}

/// Single-use guards for the I2C and I2S controllers.
static mut I2C_TAKEN: bool = false;
static mut I2S_TAKEN: bool = false;
/// CKEN bit for the I2C controller.
pub const CKEN_I2C: u32 = 14;
/// CKEN bit for the I2S controller.
pub const CKEN_I2S: u32 = 8;

/// Hands out the (standard) I2C controller once, with its clock enabled.
pub fn take_i2c() -> Option<Mmio> {
    // SAFETY: single core, IRQs masked by `_start` and not yet re-enabled.
    let taken = unsafe { core::ptr::addr_of_mut!(I2C_TAKEN).replace(true) };
    if taken {
        return None;
    }
    enable_clock(CKEN_I2C);
    let base = NonNull::new(va(0x4030_1680) as *mut u32)?;
    // SAFETY: the I2C controller's registers (PXA27x memory map: IBMR at
    // 0x40301680, ISAR the last at +0x20), identity-mapped; the flag
    // guarantees one owner.
    Some(unsafe { Mmio::new(base, 0x24) })
}

/// Hands out the I2S controller once, with its clock enabled.
pub fn take_i2s() -> Option<Mmio> {
    // SAFETY: single core, IRQs masked by `_start` and not yet re-enabled.
    let taken = unsafe { core::ptr::addr_of_mut!(I2S_TAKEN).replace(true) };
    if taken {
        return None;
    }
    enable_clock(CKEN_I2S);
    let base = NonNull::new(va(0x4040_0000) as *mut u32)?;
    // SAFETY: the I2S controller's registers (PXA27x memory map, SADR the
    // last at +0x80), identity-mapped; the flag guarantees one owner.
    Some(unsafe { Mmio::new(base, 0x84) })
}

/// Hands out SSP1 once, with its clock enabled.
pub fn take_ssp1() -> Option<Mmio> {
    // SAFETY: single core, IRQs masked by `_start` and not yet re-enabled.
    let taken = unsafe { core::ptr::addr_of_mut!(SSP1_TAKEN).replace(true) };
    if taken {
        return None;
    }
    enable_clock(CKEN_SSP1);
    let base = NonNull::new(va(0x4100_0000) as *mut u32)?;
    // SAFETY: SSP1's register window (PXA27x memory map), identity-mapped;
    // the flag guarantees one owner.
    Some(unsafe { Mmio::new(base, 0x40) })
}

/// CKEN bit for the MMC/SD controller.
pub const CKEN_MMC: u32 = 12;

/// Hands out the USB device controller once, with its clock enabled.
pub fn take_udc() -> Option<Mmio> {
    // SAFETY: single core, IRQs masked by `_start` and not yet re-enabled.
    let taken = unsafe { core::ptr::addr_of_mut!(UDC_TAKEN).replace(true) };
    if taken {
        return None;
    }
    enable_clock(CKEN_UDC);
    let base = NonNull::new(va(0x4060_0000) as *mut u32)?;
    // SAFETY: the USB device controller's register window (PXA27x memory
    // map; registers up to 0x45C, docs/hw/udc.md), identity-mapped; the flag
    // guarantees one owner.
    Some(unsafe { Mmio::new(base, 0x1000) })
}

/// Single-use guard for the USB device controller.
static mut UDC_TAKEN: bool = false;
/// CKEN bit for the USB device controller.
pub const CKEN_UDC: u32 = 11;

/// Hands out the USB host controller (OHCI, base `0x4C000000`) once, with
/// its clock enabled. On the X51v its port 2 shares the USB connector's
/// transceiver with the device controller (`UP2OCR.HXS`, docs/hw/udc.md).
pub fn take_uhc() -> Option<Mmio> {
    // SAFETY: single core, IRQs masked by `_start` and not yet re-enabled.
    let taken = unsafe { core::ptr::addr_of_mut!(UHC_TAKEN).replace(true) };
    if taken {
        return None;
    }
    enable_clock(CKEN_UHC);
    let base = NonNull::new(va(0x4C00_0000) as *mut u32)?;
    // SAFETY: the USB host controller's register window (PXA27x memory map:
    // the OHCI registers, then UHCHR at 0x64 and the PXA27x additions up to
    // 0x6C), identity-mapped; the flag guarantees one owner.
    Some(unsafe { Mmio::new(base, 0x100) })
}

/// Single-use guard for the USB host controller.
static mut UHC_TAKEN: bool = false;
/// CKEN bit for the USB host controller.
pub const CKEN_UHC: u32 = 10;

/// Hands out the MMC/SD controller once, with its clock enabled.
pub fn take_mmc() -> Option<Mmio> {
    // SAFETY: single core, IRQs masked by `_start` and not yet re-enabled.
    let taken = unsafe { core::ptr::addr_of_mut!(MMC_TAKEN).replace(true) };
    if taken {
        return None;
    }
    enable_clock(CKEN_MMC);
    let base = NonNull::new(va(0x4110_0000) as *mut u32)?;
    // SAFETY: the MMC controller's register window (PXA27x memory map, up to
    // 0x50 used), identity-mapped; the flag guarantees one owner.
    Some(unsafe { Mmio::new(base, 0x100) })
}

/// CKEN bit for the DMA controller.
pub const CKEN_DMA: u32 = 24;

/// Hands out DMA channel `ch` (one `crate::dma_map` lists) once
/// (`_start` stopped every channel), with the controller's clock enabled.
pub fn take_dma(ch: u32) -> Option<crate::dma::Dma> {
    if crate::dma_map::descriptors(ch) == 0 {
        return None;
    }
    // SAFETY: single core, IRQs masked by `_start` and not yet re-enabled.
    let taken = unsafe { &mut *core::ptr::addr_of_mut!(DMA_TAKEN) };
    if *taken & (1 << ch) != 0 {
        return None;
    }
    *taken |= 1 << ch;
    enable_clock(CKEN_DMA);
    Some(crate::dma::Dma::new(ch))
}

/// CKEN bit for PWM0 (and PWM2).
pub const CKEN_PWM0: u32 = 0;

/// Hands out PWM0 (the X51v's backlight) once, with its clock enabled.
pub fn take_pwm0() -> Option<Mmio> {
    // SAFETY: single core, IRQs masked by `_start` and not yet re-enabled.
    let taken = unsafe { core::ptr::addr_of_mut!(PWM0_TAKEN).replace(true) };
    if taken {
        return None;
    }
    enable_clock(CKEN_PWM0);
    let base = NonNull::new(va(0x40B0_0000) as *mut u32)?;
    // SAFETY: PWM0's three registers (PXA27x memory map, 0x10 bytes; PWM2
    // follows at +0x10 and is not handed out), identity-mapped; the flag
    // guarantees one owner.
    Some(unsafe { Mmio::new(base, 0x10) })
}

// --------------------------------------------------------------------------
// MMU, exceptions, tick and idle
// --------------------------------------------------------------------------

/// What went wrong, for the fault handler.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// Undefined instruction (or an unexpected vector).
    Undefined,
    /// SWI with no handler yet.
    Swi,
    /// Prefetch abort.
    PrefetchAbort,
    /// Data abort.
    DataAbort,
    /// The program ran past the run limit its starter set (the soft
    /// watchdog, [`arm_run_limit`]) without checking in.
    RunLimit,
}

impl Fault {
    /// A short fixed name (no `Debug` formatting needed).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Undefined => "Undefined",
            Self::Swi => "Swi",
            Self::PrefetchAbort => "PrefetchAbort",
            Self::DataAbort => "DataAbort",
            Self::RunLimit => "RunLimit",
        }
    }
}

static mut FAULT_HANDLER: Option<fn(Fault, u32) -> !> = None;
static mut TICK_PERIOD: u32 = 0;
static mut TICKS: u32 = 0;
/// OS timer match 1 is a one-shot alarm (tickless) rather than the tick.
static mut ALARM_MODE: bool = false;
/// Interrupt claims and signals (ADR-0007). Written by the IRQ handler and,
/// with IRQs masked, by main code.
static mut DISPATCH: crate::soc::Dispatch = crate::soc::Dispatch::new();
/// Whether the interrupt controller has been reset into a known state.
static mut INTC_READY: bool = false;

const INTC_BASE: usize = 0x40D0_0000;
const OST_BASE: usize = 0x40A0_0000;

fn intc() -> crate::soc::Intc<Mmio> {
    // SAFETY: the interrupt controller's register window; the HAL is its only
    // user (images reach it through start_tick / the IRQ path).
    crate::soc::Intc(unsafe { Mmio::new(NonNull::new_unchecked(va(INTC_BASE) as *mut u32), 0x100) })
}

/// The watchdog (OS timer match 3). Not single-owner: the loader feeds it
/// while it waits, and an image it started may feed it too; each feed is
/// two writes that only move the deadline forward.
#[must_use]
pub fn watchdog() -> crate::soc::Watchdog<Mmio> {
    // SAFETY: the OS timer's register window; the watchdog touches only
    // match 3 and OWER, which nothing else uses.
    crate::soc::Watchdog(unsafe {
        Mmio::new(NonNull::new_unchecked(va(OST_BASE) as *mut u32), 0x20)
    })
}

static mut SPARE_TIMER_TAKEN: bool = false;

/// Hands out OS timer match channel 2 once (its interrupt: `soc::irq::OST2`),
/// quiesced: whatever an earlier image left armed or pending is cleared, so
/// take it before claiming its interrupt line.
pub fn take_spare_timer() -> Option<crate::soc::SpareTimer<Mmio>> {
    // SAFETY: single core; IRQs masked by the caller's setup, as for the
    // other single-owner devices.
    let taken = unsafe { core::ptr::addr_of_mut!(SPARE_TIMER_TAKEN).replace(true) };
    if taken {
        return None;
    }
    // SAFETY: the OS timer's register window. The spare timer touches only
    // channel 2's match, status and enable bits (read-modify-write of OIER
    // with IRQs masked by its owner's callers), the alarm only channel 1's.
    let mut t = crate::soc::SpareTimer(unsafe {
        Mmio::new(NonNull::new_unchecked(va(OST_BASE) as *mut u32), 0x20)
    });
    t.quiesce();
    Some(t)
}

/// The whole OS timer block (channels 0-11), for the run limit.
fn ost_block() -> Mmio {
    // SAFETY: the OS timer's register window, channels 0-11 (0xE0 bytes).
    // The run limit touches only channel 4's registers and bit 4 of the
    // shared status and enable registers (read-modify-write with IRQs
    // masked, or from the IRQ handler).
    unsafe { Mmio::new(NonNull::new_unchecked(va(OST_BASE) as *mut u32), 0xE0) }
}

/// Arms the *soft watchdog*: if the program started next doesn't check in
/// ([`keep_alive`]) within `ms` milliseconds, an interrupt makes it fault
/// with [`Fault::RunLimit`], and its fault handler takes it back to its
/// starter (the loader) with a crash report, instead of the hardware
/// watchdog resetting the device (which lands the X51v on Dell's factory
/// prompt). OS timer channel 4 keeps its settings across the hand-off, so
/// every image built on this runtime is covered without doing anything.
/// The hardware watchdog stays as the backstop, for a program that hangs
/// with interrupts off.
pub fn arm_run_limit(ms: u32) {
    use crate::soc::ost_reg::{OIER, OMCR4, OSCR4, OSMR4, OSSR};
    let was = irqs_enabled_now();
    irq_disable();
    let mut t = ost_block();
    let enabled = t.read32(OIER);
    t.write32(OIER, enabled & !(1 << 4));
    t.write32(OSSR, 1 << 4);
    t.write32(OMCR4, 2); // 1 ms ticks
    t.write32(OSMR4, ms.max(1));
    t.write32(OSCR4, 0);
    let enabled = t.read32(OIER);
    t.write32(OIER, enabled | 1 << 4);
    route_run_limit();
    if was {
        irq_enable();
    }
}

/// Turns the soft watchdog off (the program is back, or never started).
pub fn disarm_run_limit() {
    use crate::soc::ost_reg::{OIER, OMCR4, OSSR};
    let was = irqs_enabled_now();
    irq_disable();
    let mut t = ost_block();
    let enabled = t.read32(OIER);
    t.write32(OIER, enabled & !(1 << 4));
    t.write32(OMCR4, 0);
    t.write32(OSSR, 1 << 4);
    let mut i = intc();
    i.disable(crate::soc::irq::OST_4_11);
    i.route_fiq(crate::soc::irq::OST_4_11, false);
    if was {
        irq_enable();
    }
}

/// Routes the run limit's interrupt to FIQ and unmasks it. FIQ, not IRQ:
/// a program spinning with interrupts off (IRQ masked) is still stopped,
/// since RetroPane code never masks FIQ once its vectors are in place.
fn route_run_limit() {
    let mut i = intc();
    i.route_fiq(crate::soc::irq::OST_4_11, true);
    i.enable(crate::soc::irq::OST_4_11);
}

/// The FIQ handler: the run limit expired (the only FIQ source). Faults
/// with [`Fault::RunLimit`]; the program's fault handler takes it back to
/// its starter.
#[unsafe(no_mangle)]
extern "C" fn rp_fiq() -> ! {
    let expired = ost_block().read32(crate::soc::ost_reg::OSSR) & 1 << 4 != 0;
    disarm_run_limit();
    report_fault(
        if expired {
            Fault::RunLimit
        } else {
            Fault::Undefined
        },
        0,
    )
}

/// Whether a soft run limit is armed.
#[must_use]
pub fn run_limit_armed() -> bool {
    ost_block().read32(crate::soc::ost_reg::OIER) & 1 << 4 != 0
}

/// The armed run limit's length in milliseconds, if one is armed.
#[must_use]
pub fn run_limit_ms() -> Option<u32> {
    run_limit_armed().then(|| ost_block().read32(crate::soc::ost_reg::OSMR4))
}

/// How far past the soft run limit the hardware watchdog is fed by
/// [`check_in`]: the soft limit must always fire first (it returns to the
/// loader with a report; the watchdog's reset lands the X51v on Dell's
/// factory prompt), so the watchdog is only the backstop for a program
/// hung with interrupts off. As the loader arms them at launch.
pub const WATCHDOG_MARGIN_MS: u32 = 30_000;

/// The longest a program under a starter's deadlines may go without
/// [`check_in`]: half the run limit (or of the watchdog's margin when no
/// run limit is armed). Programs that sleep cap their sleeps to it.
#[must_use]
pub fn check_in_every_ms() -> u32 {
    run_limit_ms().unwrap_or(WATCHDOG_MARGIN_MS) / 2
}

/// Checks in with both of the starter's deadlines: the soft run limit starts
/// again, and the hardware watchdog (if armed) is fed for the run limit
/// plus [`WATCHDOG_MARGIN_MS`], keeping the soft limit first. The one way
/// images check in (2026-10-13: images feeding the watchdog only as long as
/// the run limit let it win the race, and an idle image reset the device).
pub fn check_in() {
    keep_alive();
    let mut wd = watchdog();
    if wd.armed() {
        let ms = run_limit_ms()
            .unwrap_or(0)
            .saturating_add(WATCHDOG_MARGIN_MS);
        wd.feed(ms.saturating_mul(TICK_HZ / 1000));
    }
}

/// Checks in with the soft watchdog: the run limit starts again from now
/// (nothing happens if none is armed). Long-running programs call it as
/// they make progress, as they feed the hardware watchdog.
pub fn keep_alive() {
    if run_limit_armed() {
        ost_block().write32(crate::soc::ost_reg::OSCR4, 0);
    }
}

fn ostimer() -> crate::soc::OsTimer<Mmio> {
    // SAFETY: the OS timer's register window; the HAL is its only writer.
    crate::soc::OsTimer(unsafe {
        Mmio::new(NonNull::new_unchecked(va(OST_BASE) as *mut u32), 0x20)
    })
}

/// Sets what runs on an unexpected exception (gets the kind and the faulting
/// instruction's address). Call before [`init_mmu`].
pub fn set_fault_handler(f: fn(Fault, u32) -> !) {
    // SAFETY: single core with IRQs masked during setup, so nothing reads the
    // handler while it's being written.
    unsafe { core::ptr::addr_of_mut!(FAULT_HANDLER).write(Some(f)) }
}

/// What an abort handler is told.
#[derive(Clone, Copy)]
pub struct Abort {
    /// Data abort (true) or prefetch abort (false).
    pub data: bool,
    /// Address of the faulting instruction.
    pub pc: u32,
    /// Data aborts: the Fault Status Register.
    pub fsr: u32,
    /// Data aborts: the faulting (modified virtual) address. Prefetch aborts:
    /// the instruction's address.
    pub addr: u32,
}

static mut ABORT_HANDLER: Option<fn(Abort) -> bool> = None;

/// Sets the abort handler: called (in abort mode, IRQs masked) for every
/// data and prefetch abort. Returning true means "resolved": the faulting
/// instruction runs again. False reports it as a fault (the fault handler).
/// The image must size an abort stack for it (`__abt_stack_size`, see
/// `boot/link.x`).
pub fn set_abort_handler(f: fn(Abort) -> bool) {
    // SAFETY: called during setup, before any abort it could race with.
    unsafe { core::ptr::addr_of_mut!(ABORT_HANDLER).write(Some(f)) }
}

#[unsafe(no_mangle)]
extern "C" fn rp_abort(kind: u32, pc: u32) -> u32 {
    let data = kind == 4;
    // SAFETY: written once during setup.
    if let Some(f) = unsafe { core::ptr::addr_of!(ABORT_HANDLER).read() } {
        let (fsr, addr) = if data {
            crate::cpu::fault_registers()
        } else {
            (0, pc)
        };
        if f(Abort {
            data,
            pc,
            fsr,
            addr,
        }) {
            return pc; // resolved: retry the instruction
        }
    }
    // Not resolvable: a try_read/try_write resumes at its fixup.
    fixup_for(pc).unwrap_or(0)
}

/// What `_start` paints the SVC stack with.
const STACK_PAINT: u32 = 0x5741_434B;

/// The SVC stack: (bytes the deepest call reached, bytes it has), from the
/// paint `_start` laid down. Deepest = the lowest word no longer painted;
/// all of it used means it probably overflowed into `.bss` below.
#[must_use]
pub fn stack_used() -> (usize, usize) {
    unsafe extern "C" {
        static __stack_bottom: [u32; 0];
        static __stack_top: [u32; 0];
    }
    // SAFETY: the linker script reserves the SVC stack between these two
    // symbols (boot/link.x); words below the current frame are only read.
    unsafe {
        let bottom = core::ptr::addr_of!(__stack_bottom).cast::<u32>();
        let top = core::ptr::addr_of!(__stack_top).cast::<u32>();
        let size = top as usize - bottom as usize;
        let mut p = bottom;
        while p < top && p.read_volatile() == STACK_PAINT {
            p = p.add(1);
        }
        (top as usize - p as usize, size)
    }
}

/// The fixup address registered for a faulting instruction at `pc`.
fn fixup_for(pc: u32) -> Option<u32> {
    unsafe extern "C" {
        static __rp_fixup_start: [u32; 0];
        static __rp_fixup_end: [u32; 0];
    }
    // SAFETY: the linker places every (fault pc, fixup pc) word pair from
    // `.rp_fixup` between these two symbols (boot/link.x); read-only data.
    unsafe {
        let mut p = core::ptr::addr_of!(__rp_fixup_start).cast::<u32>();
        let end = core::ptr::addr_of!(__rp_fixup_end).cast::<u32>();
        while p < end {
            if p.read() == pc {
                return Some(p.add(1).read());
            }
            p = p.add(2);
        }
    }
    None
}

/// Reads the word at `addr` (any address, user or kernel), returning `None`
/// instead of faulting if it can't be read: an unmapped page the VM can't
/// resolve, another process's domain, no permission. For the kernel touching
/// memory it can't trust (system-call pointers, protection tests).
#[must_use]
#[cfg_attr(target_arch = "arm", instruction_set(arm::a32))]
pub fn try_read(addr: usize) -> Option<u32> {
    let (value, failed): (u32, u32);
    // SAFETY: a single load; if it aborts and the VM can't resolve it, the
    // abort path resumes at label 2 (registered in .rp_fixup), which only
    // sets the failure flag. No memory is written.
    unsafe {
        core::arch::asm!(
            "mov {f}, #0",
            "1: ldr {v}, [{a}]",
            "b 3f",
            "2: mov {f}, #1",
            "mov {v}, #0",
            "3:",
            ".pushsection .rp_fixup, \"a\"",
            ".word 1b, 2b",
            ".popsection",
            a = in(reg) addr,
            v = out(reg) value,
            f = out(reg) failed,
            options(nostack, readonly),
        );
    }
    (failed == 0).then_some(value)
}

/// Writes `value` to `addr`, returning false instead of faulting if it can't
/// be written (see [`try_read`]).
#[must_use]
#[cfg_attr(target_arch = "arm", instruction_set(arm::a32))]
pub fn try_write(addr: usize, value: u32) -> bool {
    let failed: u32;
    // SAFETY: a single store to an address the caller chose to probe; if it
    // aborts and can't be resolved, the fixup at label 2 sets the flag.
    unsafe {
        core::arch::asm!(
            "mov {f}, #0",
            "1: str {v}, [{a}]",
            "b 3f",
            "2: mov {f}, #1",
            "3:",
            ".pushsection .rp_fixup, \"a\"",
            ".word 1b, 2b",
            ".popsection",
            a = in(reg) addr,
            v = in(reg) value,
            f = out(reg) failed,
            options(nostack),
        );
    }
    failed == 0
}

/// A global value for bare-metal images, used safely: [`with`](Self::with)
/// masks IRQs for the duration (one core, so that's exclusive), and
/// re-entering it (say, from an abort taken while it's held) panics instead
/// of aliasing. Lets images keep kernel state in `static`s without `unsafe`.
pub struct Global<T> {
    value: core::cell::UnsafeCell<T>,
    busy: core::cell::Cell<bool>,
}

// SAFETY: every access goes through `with`, which masks IRQs and refuses
// re-entry, so on this single-core CPU no two contexts ever hold the value.
unsafe impl<T: Send> Sync for Global<T> {}

impl<T> Global<T> {
    /// A global holding `value` (usually zero-initialised, so it's `.bss`).
    pub const fn new(value: T) -> Self {
        Self {
            value: core::cell::UnsafeCell::new(value),
            busy: core::cell::Cell::new(false),
        }
    }

    /// Runs `f` with exclusive access to the value.
    ///
    /// # Panics
    /// If called again while already inside `with` on the same global.
    pub fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        let was = irq_save();
        assert!(!self.busy.replace(true), "Global re-entered");
        // SAFETY: IRQs are masked and `busy` was false, so this is the only
        // live reference (see the Sync impl).
        let r = f(unsafe { &mut *self.value.get() });
        self.busy.set(false);
        irq_restore(was);
        r
    }
}

/// One long-lived object of an image, kept in `.bss` instead of on the
/// stack: the build's zeroed-RAM budget counts it, where a local of `main`
/// was found only by an overflow at run time (rp-today's `main` held 22 KiB
/// of objects in its frame, 2026-10-17).
///
/// **Filled once per image start, never re-initialised in place.**
/// [`put_with`](Self::put_with) fills the slot and lends the object for the
/// rest of the run; a second fill returns `None` (the caller crashes loudly)
/// and never hands back, or quietly overwrites, the first. The slot is fresh
/// at every start because nothing re-enters a running image: the loader
/// starts an image only after receiving and checking it
/// (`handoff::launch`), `_start` zeroes `.bss` before `rp_main`, and a soft
/// reset or the watchdog restarts the whole device. Anything that one day
/// re-enters an image (or restarts a service in its own process) without
/// that hits the refusal at start-up, not stale state.
///
/// In-flight state: the object is never dropped (nor were the locals of a
/// `main` that never returns), so hardware it drives must be stopped by the
/// image before it returns to the loader. Hardware the next image inherits
/// is that image's to check: the 2700G isn't reset between images
/// (docs/hw/2700g.md).
pub struct Slot<T> {
    filled: core::cell::Cell<bool>,
    value: core::cell::UnsafeCell<core::mem::MaybeUninit<T>>,
}

// SAFETY: the claim (`filled`) is taken with IRQs masked on this
// single-core CPU, so exactly one context ever gets the reference; the
// object is made in that context and stays there (`&'static mut T` is only
// `Send` when `T` is), so `T` itself needn't be `Send`.
unsafe impl<T> Sync for Slot<T> {}

impl<T> Slot<T> {
    /// An empty slot (all zero: it lives in `.bss`).
    #[allow(clippy::new_without_default)]
    pub const fn new() -> Self {
        Self {
            filled: core::cell::Cell::new(false),
            value: core::cell::UnsafeCell::new(core::mem::MaybeUninit::uninit()),
        }
    }

    /// Makes the object with `make` straight into the slot and lends it for
    /// the rest of the run; `None` if the slot was filled already this
    /// start. Not inlined, so any temporary `make` needs is in this call's
    /// frame, gone once it returns, not in the caller's for the whole run.
    #[inline(never)]
    pub fn put_with(&'static self, make: impl FnOnce() -> T) -> Option<&'static mut T> {
        let was = irq_save();
        let filled = self.filled.replace(true);
        irq_restore(was);
        if filled {
            return None;
        }
        // SAFETY: `filled` went false -> true under masked IRQs just now, and
        // nothing ever sets it back, so this is the only reference made.
        let value = unsafe { &mut *self.value.get() };
        Some(value.write(make()))
    }
}

/// The SPSR at the last undefined-instruction exception (bring-up).
#[unsafe(export_name = "rp_fault_spsr")]
static mut FAULT_SPSR: u32 = 0;

/// The CPSR of the code that hit the last undefined-instruction fault
/// (mode bits, and bit 5 for Thumb state).
#[must_use]
pub fn fault_spsr() -> u32 {
    // SAFETY: a plain word, written by the exception entry before the
    // handler that reads it runs.
    unsafe { core::ptr::addr_of!(FAULT_SPSR).read_volatile() }
}

/// Reports an unrecoverable fault through the fault handler.
pub(crate) fn report_fault(kind: Fault, addr: u32) -> ! {
    // The numbers the exception entries pass to rp_fault.
    let code = match kind {
        Fault::Undefined => 1,
        Fault::Swi => 2,
        Fault::PrefetchAbort => 3,
        Fault::DataAbort => 4,
        Fault::RunLimit => 5,
    };
    rp_fault(code, addr)
}

#[unsafe(no_mangle)]
extern "C" fn rp_fault(kind: u32, addr: u32) -> ! {
    let kind = match kind {
        2 => Fault::Swi,
        3 => Fault::PrefetchAbort,
        4 => Fault::DataAbort,
        5 => Fault::RunLimit,
        _ => Fault::Undefined,
    };
    // SAFETY: written once during setup, before any exception can occur.
    if let Some(f) = unsafe { core::ptr::addr_of!(FAULT_HANDLER).read() } {
        f(kind, addr);
    }
    loop {
        core::hint::spin_loop();
    }
}

static mut PREEMPT_HOOK: Option<fn()> = None;
static mut SIGNAL_HOOK: Option<fn(u32)> = None;

/// Sets what runs when a claimed interrupt fires (after the first-level
/// handler masked and recorded it): the kernel's chance to wake the thread
/// waiting for it, e.g. by setting the event that thread waits on, so an
/// `IrqLine` is a waitable object (ADR-0010 rule 7). Gets the line's id
/// ([`IrqLine::id`]); runs in the IRQ handler, IRQs masked, and must not
/// switch threads (rescheduling happens on the way out: the preempt hook,
/// or the idle loop).
pub fn set_signal_hook(f: fn(u32)) {
    // SAFETY: set during setup, before the lines it serves are claimed.
    unsafe { core::ptr::addr_of_mut!(SIGNAL_HOOK).write(Some(f)) }
}

/// Sets what runs after an interrupt taken in user mode, before returning to
/// it: the kernel's chance to preempt the thread (quantum over, a more
/// urgent thread woken). Runs in SVC mode on the thread's kernel stack with
/// IRQs masked, and may switch threads. Interrupts taken in kernel mode don't
/// call it then (the kernel is non-preemptive, ADR-0010 rule 4): they leave
/// it owed, and it runs when that system call or user fault returns to user
/// mode, so a quantum that ends inside the kernel isn't lost.
pub fn set_preempt_hook(f: fn()) {
    // SAFETY: set during setup, before interrupts from user mode can occur.
    unsafe { core::ptr::addr_of_mut!(PREEMPT_HOOK).write(Some(f)) }
}

#[unsafe(no_mangle)]
extern "C" fn rp_irq_frame(frame: *const u32) {
    rp_irq();
    // SAFETY: `frame` is the 18-word frame the IRQ entry built; word 17 is
    // the interrupted CPSR.
    let cpsr = unsafe { frame.add(17).read() };
    // SAFETY: written once during setup.
    if let Some(h) = unsafe { core::ptr::addr_of!(PREEMPT_HOOK).read() } {
        if cpsr & 0x1F == 0x10 {
            h();
        } else {
            // The kernel isn't preemptible: the hook runs on its way back
            // to user mode instead (a quantum that ends during a system
            // call must not be lost). SAFETY: IRQs are masked here, and the
            // flag is otherwise touched only with IRQs masked.
            unsafe { core::ptr::addr_of_mut!(RESCHED_PENDING).write(true) };
        }
    }
}

/// An interrupt arrived while the kernel ran: the preempt hook is owed on
/// the next return to user mode.
static mut RESCHED_PENDING: bool = false;

/// On the way back to user mode (end of a system call or user fault): runs
/// the preempt hook if an interrupt asked for it while the kernel ran.
pub(crate) fn preempt_if_pending() {
    let was = irqs_enabled_now();
    irq_disable();
    // SAFETY: IRQs are masked, so the IRQ handler can't touch the flag now.
    let pending = unsafe { core::ptr::addr_of_mut!(RESCHED_PENDING).replace(false) };
    if pending {
        // SAFETY: written once during setup.
        if let Some(h) = unsafe { core::ptr::addr_of!(PREEMPT_HOOK).read() } {
            h();
        }
    }
    if was {
        irq_enable();
    }
}

fn rp_irq() {
    let pending = intc().pending();
    // The soft watchdog: the program ran past its limit. Disarm (one shot)
    // and fault, which its handler turns into a return to the loader.
    if pending & (1 << crate::soc::irq::OST_4_11) != 0
        && ost_block().read32(crate::soc::ost_reg::OSSR) & 1 << 4 != 0
    {
        disarm_run_limit();
        report_fault(Fault::RunLimit, 0);
    }
    if pending & (1 << crate::soc::irq::OST1) != 0 {
        // SAFETY: the IRQ handler is the only writer of TICKS (main code only
        // reads it), and TICK_PERIOD is set before the tick interrupt is enabled.
        unsafe {
            if core::ptr::addr_of!(ALARM_MODE).read() {
                ostimer().stop(); // one-shot: acknowledged and disarmed
            } else {
                ostimer().rearm(core::ptr::addr_of!(TICK_PERIOD).read());
            }
            let t = core::ptr::addr_of_mut!(TICKS);
            t.write_volatile(t.read_volatile().wrapping_add(1));
        }
    }
    // Everything else: mask and signal its owner, or mask and record it as
    // unclaimed (ADR-0007). No driver code runs here.
    let other = pending & !(1 << crate::soc::irq::OST1 | 1 << crate::soc::irq::OST_4_11);
    if other != 0 {
        // SAFETY: we're in the IRQ handler with IRQs masked, and main code
        // only touches DISPATCH with IRQs masked, so this access is exclusive.
        unsafe {
            let hook = core::ptr::addr_of!(SIGNAL_HOOK).read();
            (*core::ptr::addr_of_mut!(DISPATCH)).handle(
                &mut intc(),
                &mut gpio_edges(),
                other,
                &mut |line| {
                    if let Some(h) = hook {
                        h(line);
                    }
                },
            );
        }
    }
}

/// Interrupt sources that fired with no owner and were masked (bit n = IRQ
/// n). On the real X51v these are sources Windows Mobile left enabled.
#[must_use]
pub fn unclaimed_irqs() -> u64 {
    with_dispatch(|d| d.unclaimed)
}

/// GPIO edges that fired with no owner (bit n of word b = GPIO 32b + n);
/// their edge detection is now off.
#[must_use]
pub fn unclaimed_gpio_irqs() -> [u32; 4] {
    with_dispatch(|d| d.unclaimed_gpio)
}

/// Masks IRQs, returning whether they were enabled (for [`irq_restore`]).
/// Not generic, so critical sections cost one call, not a copy per caller.
#[inline(never)]
fn irq_save() -> bool {
    let was = irqs_enabled();
    irq_disable();
    was
}

fn irq_restore(was_enabled: bool) {
    if was_enabled {
        irq_enable();
    }
}

/// Runs `f` on the interrupt dispatch state with IRQs masked.
fn with_dispatch<T>(f: impl FnOnce(&mut crate::soc::Dispatch) -> T) -> T {
    let was = irq_save();
    // SAFETY: IRQs are masked, so the IRQ handler (the only other user) can't
    // run until we're done.
    let r = f(unsafe { &mut *core::ptr::addr_of_mut!(DISPATCH) });
    irq_restore(was);
    r
}

fn gpio_edges() -> crate::soc::GpioEdges<Mmio> {
    // SAFETY: the GPIO controller's window. The interrupt layer uses only the
    // edge registers (GRER, GFER, GEDR), which GPIO drivers never touch
    // (ADR-0007 rule 7), so sharing the window with `take_gpio`'s owner is
    // safe on one core.
    crate::soc::GpioEdges(unsafe {
        Mmio::new(NonNull::new_unchecked(va(0x40E0_0000) as *mut u32), 0x200)
    })
}

/// Resets the interrupt controller once (everything masked, IRQ not FIQ, only
/// unmasked sources wake from idle).
fn ensure_intc() {
    // SAFETY: called from main code; the flag is only touched here.
    let ready = unsafe { core::ptr::addr_of_mut!(INTC_READY).replace(true) };
    if !ready {
        intc().reset();
        // A run limit armed by whoever started this program stays live.
        if run_limit_armed() {
            route_run_limit();
        }
    }
}

pub use crate::soc::Edge;

/// An interrupt this code owns (ADR-0007): the first-level handler masks it
/// and signals it; the owner services its device, then calls
/// [`done`](Self::done) to unmask it.
pub struct IrqLine {
    src: Src,
}

#[derive(Clone, Copy)]
enum Src {
    Intc(u32),
    Gpio(u8, Edge),
}

/// Claims interrupt-controller source `n` (`soc::irq`), unmasked and ready.
/// `None` if it's taken, out of range, or a GPIO summary source.
pub fn take_irq(n: u32) -> Option<IrqLine> {
    ensure_intc();
    if !with_dispatch(|d| d.claim(n)) {
        return None;
    }
    let line = IrqLine { src: Src::Intc(n) };
    with_dispatch(|_| intc().enable(n));
    irq_enable();
    Some(line)
}

/// Claims GPIO `pin`'s edge interrupt, armed for `edge`. `None` if taken.
pub fn take_gpio_irq(pin: u8, edge: Edge) -> Option<IrqLine> {
    ensure_intc();
    if !with_dispatch(|d| d.claim_gpio(pin)) {
        return None;
    }
    with_dispatch(|_| {
        let mut i = intc();
        for n in [
            crate::soc::irq::GPIO0,
            crate::soc::irq::GPIO1,
            crate::soc::irq::GPIO_X,
        ] {
            i.enable(n);
        }
        gpio_edges().arm(pin, edge);
    });
    irq_enable();
    Some(IrqLine {
        src: Src::Gpio(pin, edge),
    })
}

impl IrqLine {
    /// The id the signal hook gets for this line: the controller source
    /// number, or `soc::gpio_line(pin)` for a GPIO edge.
    #[must_use]
    pub fn id(&self) -> u32 {
        match self.src {
            Src::Intc(n) => n,
            Src::Gpio(pin, _) => crate::soc::gpio_line(pin),
        }
    }

    /// Consumes a pending signal: true if the interrupt fired since the last
    /// call. The source stays masked until [`done`](Self::done).
    #[inline(never)]
    pub fn take(&mut self) -> bool {
        let src = self.src;
        with_dispatch(|d| match src {
            Src::Intc(n) => d.take_signal(n),
            Src::Gpio(pin, _) => d.take_gpio_signal(pin),
        })
    }

    /// Sleeps (idle mode) until the interrupt fires, consuming the signal.
    pub fn wait(&mut self) {
        loop {
            // Check-then-idle with IRQs masked: a signal can't slip in between.
            irq_disable();
            if self.take() {
                irq_enable();
                return;
            }
            idle();
            irq_enable();
        }
    }

    /// Unmasks the interrupt again (CE's `InterruptDone`). For a GPIO, edge
    /// detection is re-armed; an edge while it was off isn't remembered, so
    /// owners re-check the pin's level after this.
    #[inline(never)]
    pub fn done(&mut self) {
        let was = irq_save();
        match self.src {
            Src::Intc(n) => intc().enable(n),
            Src::Gpio(pin, edge) => gpio_edges().arm(pin, edge),
        }
        irq_restore(was);
    }
}

/// Sleeps (idle mode) until the tick advances or any claimed interrupt is
/// signalled. Consumes nothing: check the lines with [`IrqLine::take`].
pub fn wait_any_signal() {
    let start = tick_count();
    loop {
        irq_disable();
        // SAFETY: IRQs are masked (exclusive access to DISPATCH).
        let pending = unsafe { (*core::ptr::addr_of!(DISPATCH)).any_signalled() };
        if pending || tick_count() != start {
            irq_enable();
            return;
        }
        idle();
        irq_enable();
    }
}

/// Whether IRQs are enabled at the CPU (for code that must restore it).
pub(crate) fn irqs_enabled_now() -> bool {
    irqs_enabled()
}

#[cfg_attr(target_arch = "arm", instruction_set(arm::a32))]
fn irqs_enabled() -> bool {
    let cpsr: u32;
    // SAFETY: reading CPSR has no side effects.
    unsafe { core::arch::asm!("mrs {}, cpsr", out(reg) cpsr, options(nostack, nomem)) };
    cpsr & 0x80 == 0
}

/// Turns on the bring-up MMU: identity map ([`crate::mmu::X51V_BRINGUP`]),
/// I/D caches, write buffer, branch prediction and high vectors.
#[cfg_attr(target_arch = "arm", instruction_set(arm::a32))]
pub fn init_mmu() {
    unsafe extern "C" {
        static rp_vectors: [u32; 16];
    }
    // SAFETY: interrupts are masked and the MMU is off. L1_TABLE (16 KiB) and
    // VECTORS_PHYS (64 bytes) are reserved SDRAM below/above every image
    // (boot/link.x), so writing them clobbers nothing. The table maps the code
    // currently running at its own address, so execution continues seamlessly.
    let ctl: u32;
    unsafe {
        let vectors = core::ptr::addr_of!(rp_vectors).cast::<u32>();
        core::ptr::copy_nonoverlapping(vectors, crate::mmu::VECTORS_PHYS as *mut u32, 16);
        let table = &mut *(crate::mmu::L1_TABLE as *mut [u32; 4096]);
        crate::mmu::build(table, crate::mmu::X51V_BRINGUP);
        core::arch::asm!(
            "mov {t}, #0",
            "mcr p15, 0, {t}, c7, c7, 0",   // invalidate I/D caches and BTB
            "mcr p15, 0, {t}, c8, c7, 0",   // invalidate TLBs
            "mcr p15, 0, {t}, c7, c10, 4",  // drain write buffer
            "mcr p15, 0, {ttb}, c2, c0, 0", // translation table base
            "mov {t}, #1",
            "mcr p15, 0, {t}, c3, c0, 0",   // domain 0 = client
            "mrc p15, 0, {t}, c1, c0, 0",
            "orr {ctl}, {t}, {bits}",       // M C W Z I V
            t = out(reg) _,
            ctl = out(reg) ctl,
            ttb = in(reg) crate::mmu::L1_TABLE,
            bits = in(reg) (1u32 << 0) | (1 << 2) | (1 << 3) | (1 << 11) | (1 << 12) | (1 << 13),
            options(nostack),
        );
        // The enable itself from one aligned cache line (see rp_mmu_on).
        rp_mmu_on(ctl);
    }
    // Vectors in place: a run limit armed by this program's starter may now
    // stop it, even with IRQs masked (FIQ is never masked again).
    if run_limit_armed() {
        fiq_enable();
    }
}

/// Unmasks FIQ (CPSR F bit) in the current mode.
#[cfg_attr(target_arch = "arm", instruction_set(arm::a32))]
fn fiq_enable() {
    // SAFETY: clears only the F bit; the FIQ vector is installed (callers).
    unsafe {
        core::arch::asm!("mrs {t}, cpsr", "bic {t}, {t}, #0x40", "msr cpsr_c, {t}", t = out(reg) _, options(nostack));
    }
}

/// Single-use guard for the KData page.
static mut KDATA_TAKEN: bool = false;

/// Hands out the KData page (ADR-0010 rule 8) once, zeroed: splits the
/// bring-up map's top MiB into pages ([`crate::mmu::high_pages`]), so the
/// page at [`crate::mmu::KDATA_VIRT`] is user read-only and the vector page
/// becomes kernel-only. The kernel writes the page through that one user
/// address (privileged writes pass a user-read-only page), so a VIVT cache
/// never holds a second alias and no write needs cache maintenance. Call
/// after [`init_mmu`], during setup; images that never call it keep the
/// plain section.
#[inline(never)]
pub fn take_kdata_page() -> Option<&'static mut [u32; 1024]> {
    // SAFETY: single core, called during setup: nothing races the flag.
    let taken = unsafe { core::ptr::addr_of_mut!(KDATA_TAKEN).replace(true) };
    if taken {
        return None;
    }
    // Lines cached under the old mapping of this address go first.
    crate::cpu::clean_invalidate_range(crate::mmu::KDATA_VIRT, 4096);
    // SAFETY: HIGH_L2 and L1_TABLE are reserved SDRAM, identity-mapped
    // (boot/link.x), used by nothing else. The new entry maps the vector
    // page to the same physical page as the section did, so an exception
    // taken while the TLB still holds the old entry finds its vectors.
    unsafe {
        let l2 = &mut *(crate::mmu::HIGH_L2 as *mut [u32; 256]);
        crate::mmu::high_pages(l2);
        // (An opaque length: LLVM unrolled the 32-line loop to 380 bytes.)
        crate::cpu::clean_range(crate::mmu::HIGH_L2, core::hint::black_box(1024));
        let l1 = (crate::mmu::L1_TABLE as *mut u32).add(crate::mmu::HIGH_L1_INDEX);
        l1.write_volatile(crate::mmu::coarse(crate::mmu::HIGH_L2));
        crate::cpu::clean_range(l1 as u32, 4);
    }
    crate::cpu::invalidate_tlb_all();
    // SAFETY: KDATA_VIRT now maps KDATA_PHYS, reserved for it, read/write
    // for the kernel; the flag makes this the only reference.
    let page = unsafe { &mut *(crate::mmu::KDATA_VIRT as *mut [u32; 1024]) };
    page.fill(0);
    Some(page)
}

/// Maps the 1 MiB at `virt` to SDRAM at `phys` in the bring-up map (normal
/// memory, as the RAM sections), for self-tests of addresses no bring-up
/// region covers: the native process slots (`0xC0000000`-`0xFDFFFFFF`,
/// `rp_kernel_mm::layout`). Refuses (false) anything but a MiB-aligned
/// `virt` in that range that maps nothing yet, and a MiB of SDRAM. The
/// caller keeps its view coherent: the same frame at two virtual addresses
/// in the VIVT cache (clean before reading through the other).
pub fn map_test_section(virt: u32, phys: u32) -> bool {
    const MIB: u32 = 1 << 20;
    let native = (0xC000_0000..0xFE00_0000).contains(&virt);
    let ram = (0xA800_0000..0xAC00_0000).contains(&phys);
    if !native || !ram || virt % MIB != 0 || phys % MIB != 0 {
        return false;
    }
    let index = (virt >> 20) as usize;
    // SAFETY: L1_TABLE is the bring-up table (reserved SDRAM, identity
    // mapped, boot/link.x); one word of it, for an address the bring-up
    // map leaves unmapped (checked), so nothing that runs moves.
    unsafe {
        let entry = (crate::mmu::L1_TABLE as *mut u32).add(index);
        if entry.read_volatile() != 0 {
            return false;
        }
        entry.write_volatile(crate::mmu::section(phys, crate::mmu::Memory::Normal));
        crate::cpu::clean_range(entry as u32, 4);
    }
    crate::cpu::invalidate_tlb_all();
    true
}

core::arch::global_asm!(
    r#"
    .section .text.rp_mmu, "ax"
    .arm
    .global rp_mmu_on
    @ rp_mmu_on(control): writes CP15 control (MMU and caches on) and
    @ returns. The write, the CPWAIT and what follows it must share one
    @ 32-byte cache line: measured on the X51v, a build whose CPWAIT branch
    @ ended a line took an undefined-instruction fault on the first word of
    @ the next line (docs/hw/README.md). Linux keeps its __turn_mmu_on
    @ aligned for the same reason.
    .balign 32
rp_mmu_on:
    mcr     p15, 0, r0, c1, c0, 0
    mrc     p15, 0, r1, c2, c0, 0      @ CPWAIT
    mov     r1, r1
    sub     pc, pc, #4
    bx      lr
    "#
);

unsafe extern "C" {
    fn rp_mmu_on(control: u32);
}

/// Reads the word at `addr` to exercise the fault path: an unmapped address
/// raises a data abort, which the runtime reports through the fault handler
/// and never returns. Only for self-tests on addresses with no read side
/// effects (unmapped space, plain RAM); never point it at device FIFOs.
#[must_use]
pub fn fault_probe(addr: usize) -> u32 {
    // SAFETY: bare metal with the bring-up map: the read either hits mapped
    // memory (the caller promises no read side effects there) or aborts into
    // the fault handler, which doesn't return.
    unsafe { (addr as *const u32).read_volatile() }
}

/// Masks IRQs at the CPU.
#[inline]
#[cfg_attr(target_arch = "arm", instruction_set(arm::a32))]
pub fn irq_disable() {
    // SAFETY: setting the I bit in CPSR only defers interrupts.
    unsafe {
        core::arch::asm!("mrs {t}, cpsr", "orr {t}, {t}, #0x80", "msr cpsr_c, {t}", t = out(reg) _, options(nostack));
    }
}

/// Unmasks IRQs at the CPU.
#[inline]
#[cfg_attr(target_arch = "arm", instruction_set(arm::a32))]
pub fn irq_enable() {
    // SAFETY: the vectors and IRQ stack are set up by `_start` / `init_mmu`.
    unsafe {
        core::arch::asm!("mrs {t}, cpsr", "bic {t}, {t}, #0x80", "msr cpsr_c, {t}", t = out(reg) _, options(nostack));
    }
}

/// Enters idle mode until an unmasked interrupt is pending (PWRMODE = 1).
#[inline]
#[cfg_attr(target_arch = "arm", instruction_set(arm::a32))]
pub fn idle() {
    // SAFETY: idle stops the core clock until an interrupt; state is kept.
    unsafe { core::arch::asm!("mcr p14, 0, {m}, c7, c0, 0", m = in(reg) 1u32, options(nostack)) }
}

/// The idle loop's sleep: unless `ready()` (checked with IRQs masked, so a
/// wake-up can't slip in between the check and the sleep), sleeps until any
/// unmasked interrupt is pending, then lets it be taken. Unlike
/// [`wait_tick`], a device interrupt ends the sleep too.
pub fn idle_until_interrupt(ready: impl FnOnce() -> bool) {
    let was = irq_save();
    if !ready() {
        idle();
    }
    irq_restore(was);
    irq_enable();
}

/// Starts a periodic tick at `hz` on OS timer match 1 and enables IRQs.
/// Requires [`init_mmu`] (for the vectors) first.
pub fn start_tick(hz: u32) {
    let period = TICK_HZ / hz.clamp(1, TICK_HZ);
    // SAFETY: IRQs are still masked; the handler reads these only after the
    // tick interrupt is enabled below.
    unsafe {
        core::ptr::addr_of_mut!(TICK_PERIOD).write(period);
        core::ptr::addr_of_mut!(TICKS).write_volatile(0);
    }
    ensure_intc();
    ostimer().start(period);
    intc().enable(crate::soc::irq::OST1);
    irq_enable();
}

/// Arms a one-shot alarm at OS-timer value `at` (tickless scheduling). IRQs
/// must be enabled for it to wake anything (threads start with them on). When
/// it fires, [`tick_count`] advances, so [`wait_tick`] and
/// [`wait_any_signal`] return. Replaces the periodic tick. Returns false if
/// `at` has already passed; the caller should act now instead of sleeping.
pub fn set_alarm(at: u32) -> bool {
    ensure_intc();
    // SAFETY: written with IRQs masked (below) before the interrupt it
    // affects is enabled; read only by the IRQ handler.
    let was = irq_save();
    unsafe { core::ptr::addr_of_mut!(ALARM_MODE).write(true) };
    let armed = ostimer().alarm(at);
    intc().enable(crate::soc::irq::OST1);
    // Leaves the CPU's IRQ state as it was: this is also called from the
    // preempt hook, which must return with IRQs masked.
    irq_restore(was);
    armed
}

/// Stops the periodic tick (so an idle system sleeps until a real event).
pub fn stop_tick() {
    with_dispatch(|_| {
        intc().disable(crate::soc::irq::OST1);
        ostimer().stop();
    });
}

/// Ticks since [`start_tick`].
#[inline]
#[must_use]
pub fn tick_count() -> u32 {
    // SAFETY: a single aligned word written only by the IRQ handler.
    unsafe { core::ptr::addr_of!(TICKS).read_volatile() }
}

/// Sleeps (idle mode) until the next tick, then returns the tick count.
pub fn wait_tick() -> u32 {
    let start = tick_count();
    loop {
        // Check-then-idle with IRQs masked, so a tick can't slip in between
        // the check and the idle and leave us asleep.
        irq_disable();
        if tick_count() != start {
            irq_enable();
            return tick_count();
        }
        idle();
        irq_enable();
    }
}
