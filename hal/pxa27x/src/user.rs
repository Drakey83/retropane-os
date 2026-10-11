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
    // The vector page holds 8 `ldr pc, [pc, #24]` slots, then their 8
    // target words: the SWI's target is word 8 + 2.
    let phys = crate::mmu::VECTORS_PHYS + 4 * 10;
    let high = 0xFFFF_0000 + 4 * 10;
    // SAFETY: VECTORS_PHYS is the reserved vector page (identity-mapped
    // RAM, see boot/link.x), written only here and by init_mmu; the word is
    // read by the SWI vector, which can't run concurrently on one core
    // during setup.
    unsafe { (phys as *mut u32).write_volatile(rp_swi_entry as *const () as usize as u32) };
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
