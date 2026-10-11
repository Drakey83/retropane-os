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

    @ Stacks: IRQ, abort (its own when the image sizes one: the kernel
    @ resolves page faults there), undefined, then SVC, where we stay.
    @ Interrupts stay masked throughout.
    msr     cpsr_c, #0xD2
    ldr     sp, =__irq_stack_top
    msr     cpsr_c, #0xD7
    ldr     sp, =__abt_stack_top
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
    @ rp_swi_entry (system calls) lives in user.rs.
    @ Aborts: ask rp_abort first. It returns where to resume: the faulting
    @ instruction (a resolved page fault), a fixup (a faulting try_read /
    @ try_write), or 0 to report the fault and stop.
rp_pabt_entry:
    sub     lr, lr, #4
    stmfd   sp!, {{r0-r3, r12, lr}}
    mov     r0, #3
    mov     r1, lr
    bl      rp_abort
    cmp     r0, #0
    strne   r0, [sp, #20]
    ldmfdne sp!, {{r0-r3, r12, pc}}^
    ldmfd   sp!, {{r0-r3, r12, lr}}
    mov     r0, #3
    mov     r1, lr
    b       rp_fault
rp_dabt_entry:
    sub     lr, lr, #8
    stmfd   sp!, {{r0-r3, r12, lr}}
    mov     r0, #4
    mov     r1, lr
    bl      rp_abort
    cmp     r0, #0
    strne   r0, [sp, #20]
    ldmfdne sp!, {{r0-r3, r12, pc}}^
    ldmfd   sp!, {{r0-r3, r12, lr}}
    mov     r0, #4
    mov     r1, lr
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

/// Reports an unrecoverable fault through the fault handler.
pub(crate) fn report_fault(kind: Fault, addr: u32) -> ! {
    // The numbers the exception entries pass to rp_fault.
    let code = match kind {
        Fault::Undefined => 1,
        Fault::Swi => 2,
        Fault::PrefetchAbort => 3,
        Fault::DataAbort => 4,
    };
    rp_fault(code, addr)
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
    let other = pending & !(1 << crate::soc::irq::OST1);
    if other != 0 {
        // SAFETY: we're in the IRQ handler with IRQs masked, and main code
        // only touches DISPATCH with IRQs masked, so this access is exclusive.
        unsafe {
            (*core::ptr::addr_of_mut!(DISPATCH)).handle(&mut intc(), &mut gpio_edges(), other);
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

fn irqs_enabled() -> bool {
    let cpsr: u32;
    // SAFETY: reading CPSR has no side effects.
    unsafe { core::arch::asm!("mrs {}, cpsr", out(reg) cpsr, options(nostack, nomem)) };
    cpsr & 0x80 == 0
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
    ensure_intc();
    ostimer().start(period);
    intc().enable(crate::soc::irq::OST1);
    irq_enable();
}

/// Arms a one-shot alarm at OS-timer value `at` (tickless scheduling): when
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
    irq_restore(was);
    irq_enable();
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
