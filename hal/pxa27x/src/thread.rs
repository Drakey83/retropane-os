//! Kernel threads: saved contexts and the context switch (ADR-0010).
//!
//! The kernel is non-preemptive (ADR-0010 rule 4): a thread gives up the CPU
//! only at a scheduling point (a wait, a sleep, a yield, or the return from
//! an exception), always by calling [`switch`]. So a context is just what the
//! ARM calling convention says a callee must preserve: `r4`-`r11`, the stack
//! pointer and the return address. Ten words, saved and loaded with one `stm`
//! and one `ldm`.
//!
//! Contexts live here, indexed by thread number (the scheduler's `Tid`), so
//! the kernel never handles raw stack pointers.

use core::ptr::addr_of_mut;

/// Threads the HAL keeps contexts for (matches the scheduler's table).
pub const MAX_THREADS: usize = 128;

/// A saved thread: `r4`-`r11`, `sp`, `lr`.
#[repr(C)]
#[derive(Clone, Copy)]
struct Context([u32; 10]);

static mut CONTEXTS: [Context; MAX_THREADS] = [Context([0; 10]); MAX_THREADS];

core::arch::global_asm!(
    r#"
    .section .text.rp_switch, "ax"
    .global rp_switch
    .arm
    @ rp_switch(from: *mut Context, to: *const Context)
rp_switch:
    stmia   r0, {{r4-r11, sp, lr}}
    ldmia   r1, {{r4-r11, sp, lr}}
    bx      lr

    @ A new thread's first "return": r4 = entry, r5 = argument. Interrupts on
    @ (FIQ stays masked), then the entry function, which never returns.
    .global rp_thread_start
rp_thread_start:
    msr     cpsr_c, #0x53
    mov     r0, r5
    blx     r4
1:  b       1b
    "#
);

unsafe extern "C" {
    fn rp_switch(from: *mut Context, to: *const Context);
    fn rp_thread_start();
}

/// Prepares thread `i` to start running `entry(arg)` on `stack` the first
/// time something switches to it. The stack is handed over for good.
///
/// # Panics
/// If `i` is out of range or the stack is smaller than 256 bytes.
pub fn prepare(i: usize, stack: &'static mut [u32], entry: extern "C" fn(usize) -> !, arg: usize) {
    assert!(i < MAX_THREADS, "no such thread slot");
    assert!(stack.len() >= 64, "stack too small");
    // Full descending stack, 8-byte aligned top (AAPCS).
    let top = (stack.as_mut_ptr() as usize + stack.len() * 4) & !7;
    let mut c = Context([0; 10]);
    c.0[0] = entry as usize as u32; // r4
    c.0[1] = arg as u32; // r5
    c.0[8] = top as u32; // sp
    c.0[9] = rp_thread_start as *const () as usize as u32; // lr
    // SAFETY: slot `i` belongs to a thread that isn't running (it's being
    // created), and the kernel prepares and switches with IRQs masked or from
    // the one running thread, so nothing else touches this slot now.
    unsafe { addr_of_mut!(CONTEXTS[i]).write(c) };
}

/// Saves the running thread's context in slot `from` and resumes thread `to`
/// (which was saved by an earlier `switch`, or set up by [`prepare`]).
/// Returns when something switches back to `from`.
///
/// # Panics
/// If either index is out of range or they're equal.
pub fn switch(from: usize, to: usize) {
    assert!(
        from < MAX_THREADS && to < MAX_THREADS && from != to,
        "bad switch"
    );
    let was = crate::rt::irqs_enabled_now();
    crate::rt::irq_disable();
    // SAFETY: the two slots are distinct and IRQs are masked, so these are
    // the only references; `rp_switch` saves the callee-saved registers, sp
    // and lr of this call into `from` and loads `to`'s, which hold either a
    // previous `switch` call's state (so it returns there) or a `prepare`d
    // start. Every thread runs in SVC mode with its own stack.
    unsafe {
        rp_switch(addr_of_mut!(CONTEXTS[from]), addr_of_mut!(CONTEXTS[to]));
    }
    // Back in `from`'s context: restore its interrupt state.
    if was {
        crate::rt::irq_enable();
    }
}
