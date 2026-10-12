//! User mode and system calls (ADR-0010 rules 5-6).
//!
//! A system call is `SWI` with the call number in `r7` and arguments in
//! `r0`-`r3`, so the kernel never reads the instruction from user memory.
//! Entry saves the whole user context as a [`Frame`] on the running thread's
//! kernel stack (the SVC stack, which is per thread: ADR-0010 rule 4) and
//! calls the kernel's handler with it; the handler may block and switch
//! threads, since every thread's frame stays on its own stack. Exit restores
//! the frame and returns to user mode.

/// The user context saved on a system call.
#[repr(C)]
pub struct Frame {
    /// `r0`-`r12`: `r0`-`r3` are arguments (and `r0` the result), `r7` the
    /// call number.
    pub r: [u32; 13],
    /// User stack pointer.
    pub sp: u32,
    /// User link register.
    pub lr: u32,
    /// Where the user resumes (the instruction after the `SWI`).
    pub pc: u32,
    /// The user's CPSR (mode bits `0x10` for user mode).
    pub cpsr: u32,
}

impl Frame {
    /// The call number.
    #[must_use]
    pub fn number(&self) -> u32 {
        self.r[7]
    }

    /// Argument `i` (0-3).
    #[must_use]
    pub fn arg(&self, i: usize) -> u32 {
        self.r[i]
    }

    /// Sets the value returned to the caller.
    pub fn set_result(&mut self, v: u32) {
        self.r[0] = v;
    }

    /// Whether the call came from user mode.
    #[must_use]
    pub fn from_user(&self) -> bool {
        self.cpsr & 0x1F == 0x10
    }
}

core::arch::global_asm!(
    r#"
    .section .text.rp_user, "ax"
    .arm
    .global rp_swi_entry
    @ SWI: SVC mode, lr = the user's next instruction, spsr = its CPSR.
rp_swi_entry:
    sub     sp, sp, #68                 @ a Frame: 17 words
    stmia   sp, {{r0-r12}}
    add     r8, sp, #52
    stmia   r8, {{sp, lr}}^             @ user sp and lr
    nop                                 @ no banked access right after ^
    mrs     r9, spsr
    str     lr, [sp, #60]               @ pc
    str     r9, [sp, #64]               @ cpsr
    mov     r0, sp
    msr     cpsr_c, #0x53               @ IRQs on while the kernel works
    bl      rp_syscall
    @ Back to user mode from the Frame at sp (system calls, user faults).
rp_user_return:
    msr     cpsr_c, #0xD3               @ IRQs off while restoring
    ldr     r9, [sp, #64]
    msr     spsr_cxsf, r9
    ldr     lr, [sp, #60]
    add     r8, sp, #52
    ldmia   r8, {{sp, lr}}^
    nop
    ldmia   sp, {{r0-r12}}
    add     sp, sp, #68
    movs    pc, lr

    @ A prefetch or data abort taken in user mode (from rt's abort entries):
    @ abort mode, r0 = kind (3 prefetch, 4 data), and on the abort stack the
    @ user's r0-r3, r12 and the faulting pc. Build the same Frame a system
    @ call does, on the thread's kernel stack, and handle the fault there in
    @ the thread's own context, where it may block (ADR-0010 rule 5).
    .global rp_uabt_entry
rp_uabt_entry:
    mrs     r1, spsr                    @ the user's CPSR
    mov     r2, sp                      @ the six saved words
    add     sp, sp, #24                 @ copied below, before anything can
                                        @ reuse the abort stack (IRQs stay off)
    msr     cpsr_c, #0xD3               @ SVC: the thread's kernel stack
    sub     sp, sp, #68
    add     r3, sp, #16
    stmia   r3, {{r4-r11}}              @ r4-r11 are still the user's
    ldmia   r2, {{r4-r9}}               @ user r0-r3, r12, pc
    stmia   sp, {{r4-r7}}
    str     r8, [sp, #48]
    str     r9, [sp, #60]
    str     r1, [sp, #64]
    add     r8, sp, #52
    stmia   r8, {{sp, lr}}^             @ user sp and lr
    nop
    mov     r1, r0
    mov     r0, sp
    msr     cpsr_c, #0x53               @ IRQs on while the kernel works
    bl      rp_user_fault
    b       rp_user_return              @ resume at the (fixed) instruction

    @ rp_enter_user(pc, user_sp, arg): first entry to user mode, never returns.
    .global rp_enter_user
rp_enter_user:
    msr     cpsr_c, #0xDF               @ SYS mode (user registers), IRQs off
    mov     sp, r1
    mov     lr, #0
    msr     cpsr_c, #0xD3               @ back to SVC
    mov     lr, r0
    mov     r0, #0x50                   @ user mode, IRQs on, FIQ masked
    msr     spsr_cxsf, r0
    mov     r0, r2
    mov     r1, #0
    mov     r2, #0
    mov     r3, #0
    mov     r4, #0
    mov     r5, #0
    mov     r6, #0
    mov     r7, #0
    mov     r8, #0
    mov     r9, #0
    mov     r10, #0
    mov     r11, #0
    mov     r12, #0
    movs    pc, lr
    "#
);

static mut HANDLER: Option<fn(&mut Frame)> = None;

unsafe extern "C" {
    fn rp_swi_entry();
}

/// Sets the kernel's system-call handler and routes the SWI vector to the
/// system-call entry (until then it reports a fault, and images that never
/// call this don't link the entry at all). Call after `rt::init_mmu`.
pub fn set_syscall_handler(f: fn(&mut Frame)) {
    // SAFETY: set during setup, before any thread can make a system call.
    unsafe { core::ptr::addr_of_mut!(HANDLER).write(Some(f)) }
    set_vector(2, rp_swi_entry as *const () as usize as u32);
}

/// Points exception vector `slot` (0 reset .. 7 FIQ) at `entry`. The vector
/// page holds 8 `ldr pc, [pc, #24]` slots, then their 8 target words, so
/// optional entries (system calls, lazy iWMMXt) are linked only into images
/// that install them.
pub(crate) fn set_vector(slot: u32, entry: u32) {
    let phys = crate::mmu::VECTORS_PHYS + 4 * (8 + slot);
    let high = 0xFFFF_0000 + 4 * (8 + slot);
    // SAFETY: VECTORS_PHYS is the reserved vector page (identity-mapped
    // RAM, see boot/link.x), written only by init_mmu and during setup; the
    // word is read only by that exception's vector, which doesn't run
    // concurrently on one core during setup.
    unsafe { (phys as *mut u32).write_volatile(entry) };
    // VIVT: the same word is also cached under the high-vector address.
    // Write it back through this alias and drop both aliases' lines.
    crate::cpu::clean_invalidate_range(phys, 4);
    crate::cpu::clean_invalidate_range(high, 4);
}

#[unsafe(no_mangle)]
extern "C" fn rp_syscall(frame: *mut Frame) {
    // SAFETY: `frame` points at the Frame the SWI entry just built on this
    // thread's kernel stack; nothing else refers to it until we return.
    let frame = unsafe { &mut *frame };
    // SAFETY: written once during setup.
    match unsafe { core::ptr::addr_of!(HANDLER).read() } {
        Some(f) => f(frame),
        None => crate::rt::report_fault(crate::rt::Fault::Swi, frame.pc.wrapping_sub(4)),
    }
}

/// Handles a fault from user mode: the user context, which fault, and the
/// faulting address. It may fix the cause and return (the instruction runs
/// again), or end the thread and never return.
pub type FaultHandler = fn(&mut Frame, crate::rt::Fault, u32);

static mut FAULT_HANDLER: Option<FaultHandler> = None;

unsafe extern "C" {
    fn rp_uabt_entry();
}

/// Sets the kernel's handler for faults taken in user mode and routes such
/// aborts to it (in the faulting thread's context, on its kernel stack).
/// Until then, and for faults in kernel mode, aborts are reported as usual.
pub fn set_user_fault_handler(f: FaultHandler) {
    // SAFETY: set during setup, before any user thread runs.
    unsafe { core::ptr::addr_of_mut!(FAULT_HANDLER).write(Some(f)) }
    crate::rt::set_user_abort_entry(rp_uabt_entry as *const () as usize as u32);
}

#[unsafe(no_mangle)]
extern "C" fn rp_user_fault(frame: *mut Frame, kind: u32) {
    // SAFETY: `frame` points at the Frame the user-abort entry just built on
    // this thread's kernel stack; nothing else refers to it until we return.
    let frame = unsafe { &mut *frame };
    let (fault, addr) = if kind == 3 {
        (crate::rt::Fault::PrefetchAbort, frame.pc)
    } else {
        (crate::rt::Fault::DataAbort, crate::cpu::fault_registers().1)
    };
    // SAFETY: written once during setup.
    match unsafe { core::ptr::addr_of!(FAULT_HANDLER).read() } {
        Some(f) => f(frame, fault, addr),
        None => crate::rt::report_fault(fault, addr),
    }
}

/// Reads a word as user code (user side): for tests of the user-fault path
/// (an address the user may not access faults like any user access).
#[must_use]
#[cfg_attr(target_arch = "arm", instruction_set(arm::a32))]
pub fn user_read(addr: u32) -> u32 {
    let v: u32;
    // SAFETY: a plain load. In user mode the MMU checks it; a bad address
    // raises an abort the kernel's user-fault handler deals with.
    unsafe {
        core::arch::asm!(
            "ldr {v}, [{a}]",
            v = out(reg) v,
            a = in(reg) addr,
            options(nostack, readonly)
        );
    }
    v
}

/// Writes `value` to `addr` (user side: in user mode the MMU checks it, and
/// a refused write raises an abort the kernel's user-fault handler gets).
#[cfg_attr(target_arch = "arm", instruction_set(arm::a32))]
pub fn user_write(addr: u32, value: u32) {
    // SAFETY: a plain store, checked by the MMU in user mode as above.
    unsafe {
        core::arch::asm!(
            "str {v}, [{a}]",
            v = in(reg) value,
            a = in(reg) addr,
            options(nostack)
        );
    }
}

/// Writes media register `wR0` (user side; with lazy switching, a thread's
/// first media instruction traps once and then runs).
#[cfg_attr(target_arch = "arm", instruction_set(arm::a32))]
pub fn wmmx_write(v: u64) {
    // SAFETY: TMCRR wR0 (MCRR to coprocessor 0) only changes wR0.
    unsafe {
        core::arch::asm!(
            "mcrr p0, #0, {lo}, {hi}, c0",
            lo = in(reg) v as u32,
            hi = in(reg) (v >> 32) as u32,
            options(nomem, nostack),
        )
    };
}

/// Reads media register `wR0` (user side).
#[must_use]
#[cfg_attr(target_arch = "arm", instruction_set(arm::a32))]
pub fn wmmx_read() -> u64 {
    let (lo, hi): (u32, u32);
    // SAFETY: TMRRC from wR0 (MRRC from coprocessor 0) only reads it.
    unsafe {
        core::arch::asm!(
            "mrrc p0, #0, {lo}, {hi}, c0",
            lo = out(reg) lo,
            hi = out(reg) hi,
            options(nomem, nostack),
        )
    };
    u64::from(hi) << 32 | u64::from(lo)
}

unsafe extern "C" {
    fn rp_enter_user(pc: u32, sp: u32, arg: u32) -> !;
}

/// Leaves the kernel for good: continues in user mode at `entry(arg)` on
/// the user stack `stack` (memory the user may access). The calling kernel
/// thread's stack becomes its kernel stack for system calls.
pub fn enter_user(entry: extern "C" fn(usize) -> !, stack: &'static mut [u32], arg: usize) -> ! {
    let top = (stack.as_mut_ptr() as usize + stack.len() * 4) & !7;
    // SAFETY: the user stack is handed over for good; entry is code the user
    // may execute (the caller maps it so); the SVC stack pointer stays at
    // this thread's kernel stack for later SWIs.
    unsafe { rp_enter_user(entry as usize as u32, top as u32, arg as u32) }
}

/// Makes system call `number` with arguments `a0`-`a2` (user side).
#[cfg_attr(target_arch = "arm", instruction_set(arm::a32))]
pub fn syscall(number: u32, a0: u32, a1: u32, a2: u32) -> u32 {
    let r: u32;
    // SAFETY: SWI enters the kernel, which may only change r0 (the result)
    // and returns to the next instruction with every other register intact.
    unsafe {
        core::arch::asm!(
            "swi #0",
            inlateout("r0") a0 => r,
            in("r1") a1,
            in("r2") a2,
            in("r7") number,
            options(nostack),
        );
    }
    r
}
