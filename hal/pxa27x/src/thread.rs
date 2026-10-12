//! Kernel threads: saved contexts and the context switch (ADR-0010).
//!
//! The kernel is non-preemptive (ADR-0010 rule 4): a thread gives up the CPU
//! only at a scheduling point (a wait, a sleep, a yield, or the return from
//! an exception), always by calling [`switch`]. So a context is just what the
//! ARM calling convention says a callee must preserve: `r4`-`r11` and the
//! return address, pushed on the thread's own kernel stack with one `push`.
//! Only the stack pointer stays here, indexed by thread number (the
//! scheduler's `Tid`): one word per thread instead of ten (512 bytes for
//! 128 threads, not 5 KiB), and the kernel never handles raw stack
//! pointers.

use core::ptr::addr_of_mut;

/// Threads the HAL keeps contexts for (matches the scheduler's table).
pub const MAX_THREADS: usize = 128;

/// Per thread: its kernel stack pointer while it isn't running (its
/// `r4`-`r11` and return address are pushed just above).
static mut SPS: [u32; MAX_THREADS] = [0; MAX_THREADS];

/// Words `rp_switch` pushes: `r4`-`r11`, `lr`.
const SAVED_WORDS: usize = 9;

core::arch::global_asm!(
    r#"
    .section .text.rp_switch, "ax"
    .global rp_switch
    .arm
    @ rp_switch(from_sp: *mut u32, to_sp: *const u32)
rp_switch:
    push    {{r4-r11, lr}}
    str     sp, [r0]
    ldr     sp, [r1]
    pop     {{r4-r11, pc}}              @ interworks: returns to ARM or Thumb

    @ A new thread's first "return": r4 = entry, r5 = argument. Interrupts on
    @ (FIQ stays masked), then the entry function, which never returns.
    .global rp_thread_start
rp_thread_start:
    msr     cpsr_c, #0x13              @ IRQs on; FIQ open (run limit)
    mov     r0, r5
    blx     r4
1:  b       1b
    "#
);

unsafe extern "C" {
    fn rp_switch(from_sp: *mut u32, to_sp: *const u32);
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
    // Full descending stack, 8-byte aligned top (AAPCS): the first switch
    // pops the saved words and leaves sp at the top, aligned.
    let base = stack.as_ptr() as usize;
    let top = (((base + stack.len() * 4) & !7) - base) / 4 - SAVED_WORDS;
    let frame = &mut stack[top..top + SAVED_WORDS];
    frame.fill(0);
    frame[0] = entry as usize as u32; // r4
    frame[1] = arg as u32; // r5
    frame[8] = rp_thread_start as *const () as usize as u32; // lr -> pc
    let sp = frame.as_ptr() as usize as u32;
    // SAFETY: slot `i` belongs to a thread that isn't running (it's being
    // created), and the kernel prepares and switches with IRQs masked or from
    // the one running thread, so nothing else touches this slot now.
    unsafe { addr_of_mut!(SPS[i]).write(sp) };
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
    // Media-unit access follows its owner (lazy iWMMXt switching).
    crate::iwmmxt::on_switch(to);
    // SAFETY: the two slots are distinct and IRQs are masked, so these are
    // the only references; `rp_switch` pushes this call's callee-saved
    // registers and return address on this thread's stack, keeps sp in
    // `from`'s slot and resumes `to`'s stack, which holds either a previous
    // `switch` call's words (so it returns there) or a `prepare`d start.
    // Every thread runs in SVC mode with its own stack.
    unsafe {
        rp_switch(addr_of_mut!(SPS[from]), addr_of_mut!(SPS[to]));
    }
    // Back in `from`'s context: restore its interrupt state.
    if was {
        crate::rt::irq_enable();
    }
}
