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

    @ Stacks: IRQ, then abort/undefined (shared; they only report and stop),
    @ then SVC, where we stay. Interrupts stay masked throughout.
    msr     cpsr_c, #0xD2
    ldr     sp, =__irq_stack_top
    msr     cpsr_c, #0xD7
    ldr     sp, =__exc_stack_top
    msr     cpsr_c, #0xDB
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
    .word   _start, rp_undef_entry, rp_swi_entry, rp_pabt_entry
    .word   rp_dabt_entry, rp_undef_entry, rp_irq_entry, rp_undef_entry

    @ IRQ: save the caller-saved registers (AAPCS; 6 words keeps the stack
    @ 8-byte aligned), dispatch in Rust, return restoring CPSR from SPSR.
rp_irq_entry:
    sub     lr, lr, #4
    stmfd   sp!, {{r0-r3, r12, lr}}
    bl      rp_irq
    ldmfd   sp!, {{r0-r3, r12, pc}}^

    @ Faults: report kind and faulting address, then stop.
rp_undef_entry:
    mov     r0, #1
    sub     r1, lr, #4
    b       rp_fault
rp_swi_entry:
    mov     r0, #2
    sub     r1, lr, #4
    b       rp_fault
rp_pabt_entry:
    mov     r0, #3
    sub     r1, lr, #4
    b       rp_fault
rp_dabt_entry:
    mov     r0, #4
    sub     r1, lr, #8
    b       rp_fault
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
/// Single-use guard for the keypad controller.
static mut KEYPAD_TAKEN: bool = false;
/// Single-use guard for the GPIO controller.
static mut GPIO_TAKEN: bool = false;
/// Single-use guard for SSP1.
static mut SSP1_TAKEN: bool = false;
/// Single-use guard for the MMC controller.
static mut MMC_TAKEN: bool = false;

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
    let base = NonNull::new(FFUART_BASE as *mut u32)?;
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
    unsafe { Mmio::new(NonNull::new_unchecked(FFUART_BASE as *mut u32), UART_WINDOW) }
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

/// OS timer frequency.
pub const TICK_HZ: u32 = 3_250_000;

/// The free-running OS timer (3.25 MHz), for timing and benchmarks.
#[inline]
#[must_use]
pub fn ticks() -> u32 {
    // SAFETY: OSCR0 is a counter register in the PXA27x OS timer block,
    // identity-mapped with the MMU off; reading it has no side effects.
    unsafe { (OSCR0 as *const u32).read_volatile() }
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
    let cken = CKEN as *mut u32;
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
    let base = NonNull::new(KEYPAD_BASE as *mut u32)?;
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
    let base = NonNull::new(0x40E0_0000 as *mut u32)?;
    // SAFETY: the GPIO controller's register window (PXA27x memory map, up to
    // 0x130 used), identity-mapped; the flag guarantees one owner.
    Some(unsafe { Mmio::new(base, 0x200) })
}

/// Hands out SSP1 once, with its clock enabled.
pub fn take_ssp1() -> Option<Mmio> {
    // SAFETY: single core, IRQs masked by `_start` and not yet re-enabled.
    let taken = unsafe { core::ptr::addr_of_mut!(SSP1_TAKEN).replace(true) };
    if taken {
        return None;
    }
    enable_clock(CKEN_SSP1);
    let base = NonNull::new(0x4100_0000 as *mut u32)?;
    // SAFETY: SSP1's register window (PXA27x memory map), identity-mapped;
    // the flag guarantees one owner.
    Some(unsafe { Mmio::new(base, 0x40) })
}

/// CKEN bit for the MMC/SD controller.
pub const CKEN_MMC: u32 = 12;

/// Hands out the MMC/SD controller once, with its clock enabled.
pub fn take_mmc() -> Option<Mmio> {
    // SAFETY: single core, IRQs masked by `_start` and not yet re-enabled.
    let taken = unsafe { core::ptr::addr_of_mut!(MMC_TAKEN).replace(true) };
    if taken {
        return None;
    }
    enable_clock(CKEN_MMC);
    let base = NonNull::new(0x4110_0000 as *mut u32)?;
    // SAFETY: the MMC controller's register window (PXA27x memory map, up to
    // 0x50 used), identity-mapped; the flag guarantees one owner.
    Some(unsafe { Mmio::new(base, 0x100) })
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
        }
    }
}

static mut FAULT_HANDLER: Option<fn(Fault, u32) -> !> = None;
static mut TICK_PERIOD: u32 = 0;
static mut TICKS: u32 = 0;
/// See [`unclaimed_irqs`].
static mut UNCLAIMED: u64 = 0;

const INTC_BASE: usize = 0x40D0_0000;
const OST_BASE: usize = 0x40A0_0000;

fn intc() -> crate::soc::Intc<Mmio> {
    // SAFETY: the interrupt controller's register window; the HAL is its only
    // user (images reach it through start_tick / the IRQ path).
    crate::soc::Intc(unsafe { Mmio::new(NonNull::new_unchecked(INTC_BASE as *mut u32), 0x100) })
}

fn ostimer() -> crate::soc::OsTimer<Mmio> {
    // SAFETY: the OS timer's register window; the HAL is its only writer.
    crate::soc::OsTimer(unsafe { Mmio::new(NonNull::new_unchecked(OST_BASE as *mut u32), 0x20) })
}

/// Sets what runs on an unexpected exception (gets the kind and the faulting
/// instruction's address). Call before [`init_mmu`].
pub fn set_fault_handler(f: fn(Fault, u32) -> !) {
    // SAFETY: single core with IRQs masked during setup, so nothing reads the
    // handler while it's being written.
    unsafe { core::ptr::addr_of_mut!(FAULT_HANDLER).write(Some(f)) }
}

#[unsafe(no_mangle)]
extern "C" fn rp_fault(kind: u32, addr: u32) -> ! {
    let kind = match kind {
        2 => Fault::Swi,
        3 => Fault::PrefetchAbort,
        4 => Fault::DataAbort,
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

#[unsafe(no_mangle)]
extern "C" fn rp_irq() {
    let pending = intc().pending();
    if pending & (1 << crate::soc::irq::OST1) != 0 {
        // SAFETY: the IRQ handler is the only writer of TICKS (main code only
        // reads it), and TICK_PERIOD is set before the tick interrupt is enabled.
        unsafe {
            ostimer().rearm(core::ptr::addr_of!(TICK_PERIOD).read());
            let t = core::ptr::addr_of_mut!(TICKS);
            t.write_volatile(t.read_volatile().wrapping_add(1));
        }
    }
    // Anything else isn't claimed by a driver yet: mask it so it can't storm,
    // and record it, so it's visible rather than silently dropped
    // ([`unclaimed_irqs`]).
    let other = pending & !(1 << crate::soc::irq::OST1);
    if other != 0 {
        // SAFETY: only this handler writes UNCLAIMED; main code reads it with
        // a single aligned volatile access per half.
        unsafe {
            let u = core::ptr::addr_of_mut!(UNCLAIMED);
            u.write_volatile(u.read_volatile() | other);
        }
        for n in 0..40 {
            if other & (1 << n) != 0 {
                intc().disable(n);
            }
        }
    }
}

/// Interrupt sources that fired with no owner and were masked (bit n = IRQ
/// n). On the real X51v these are sources Windows Mobile left enabled.
#[must_use]
pub fn unclaimed_irqs() -> u64 {
    // SAFETY: written only by the IRQ handler; a torn read between the two
    // halves can only miss a bit that is reported on the next read.
    unsafe { core::ptr::addr_of!(UNCLAIMED).read_volatile() }
}

/// Turns on the bring-up MMU: identity map ([`crate::mmu::X51V_BRINGUP`]),
/// I/D caches, write buffer, branch prediction and high vectors.
pub fn init_mmu() {
    unsafe extern "C" {
        static rp_vectors: [u32; 16];
    }
    // SAFETY: interrupts are masked and the MMU is off. L1_TABLE (16 KiB) and
    // VECTORS_PHYS (64 bytes) are reserved SDRAM below/above every image
    // (boot/link.x), so writing them clobbers nothing. The table maps the code
    // currently running at its own address, so execution continues seamlessly.
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
            "orr {t}, {t}, {bits}",         // M C W Z I V
            "mcr p15, 0, {t}, c1, c0, 0",
            "mrc p15, 0, {t}, c2, c0, 0",   // XScale CPWAIT
            "mov {t}, {t}",
            "sub pc, pc, #4",
            t = out(reg) _,
            ttb = in(reg) crate::mmu::L1_TABLE,
            bits = in(reg) (1u32 << 0) | (1 << 2) | (1 << 3) | (1 << 11) | (1 << 12) | (1 << 13),
            options(nostack),
        );
    }
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
pub fn irq_disable() {
    // SAFETY: setting the I bit in CPSR only defers interrupts.
    unsafe {
        core::arch::asm!("mrs {t}, cpsr", "orr {t}, {t}, #0x80", "msr cpsr_c, {t}", t = out(reg) _, options(nostack));
    }
}

/// Unmasks IRQs at the CPU.
#[inline]
pub fn irq_enable() {
    // SAFETY: the vectors and IRQ stack are set up by `_start` / `init_mmu`.
    unsafe {
        core::arch::asm!("mrs {t}, cpsr", "bic {t}, {t}, #0x80", "msr cpsr_c, {t}", t = out(reg) _, options(nostack));
    }
}

/// Enters idle mode until an unmasked interrupt is pending (PWRMODE = 1).
#[inline]
pub fn idle() {
    // SAFETY: idle stops the core clock until an interrupt; state is kept.
    unsafe { core::arch::asm!("mcr p14, 0, {m}, c7, c0, 0", m = in(reg) 1u32, options(nostack)) }
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
    let mut i = intc();
    i.reset();
    ostimer().start(period);
    i.enable(crate::soc::irq::OST1);
    irq_enable();
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
