//! Kernel threads: saved contexts and the context switch (ADR-0010).
//!
//! The kernel is non-preemptive (ADR-0010 rule 4): a thread gives up the CPU
//! only at a scheduling point (a wait, a sleep, a yield, or the return from
//! an exception), always by calling [`switch`]. So a context is just what the
//! ARM calling convention says a callee must preserve: `r4`-`r11` and the
//! return address, pushed on the thread's own kernel stack with one `push`.
//! Only the stack pointer stays here, indexed by thread number (the
//! scheduler's `Tid`): one word per thread instead of ten (~1 KiB for
//! 255 threads, not 10 KiB), and the kernel never handles raw stack
//! pointers.

use core::ptr::addr_of_mut;

/// Threads the HAL keeps contexts for (matches the scheduler's table; the
/// kernel image checks at build time).
pub const MAX_THREADS: usize = 255;

/// Per thread: its kernel stack pointer while it isn't running (its
/// `r4`-`r11` and return address are pushed just above).
static mut SPS: [u32; MAX_THREADS] = [0; MAX_THREADS];

/// Words `rp_switch` pushes: `r4`-`r11`, `lr`.
const SAVED_WORDS: usize = 9;

/// The word at the bottom of every prepared thread's stack: a thread that
/// went past its stack wrote over it (2026-11-23, Game Mode G7a: a 744-byte
/// `on_syscall` frame held at every nesting level of a migrating call ran
/// a 4 KiB kernel stack into its neighbour, and the emulator showed only
/// garbage contexts, `not a process slot` and re-entered globals, a
/// different one each run).
pub const STACK_GUARD: u32 = 0x57AC_6A4D;

/// Per thread: its stack's lowest word's address (0: not prepared here,
/// such as the boot thread, which has its own guard).
static mut BOTTOMS: [u32; MAX_THREADS] = [0; MAX_THREADS];

/// Words at the bottom of thread `i`'s stack never written since it was
/// prepared (its stack's depth never reached them), counted up to `limit`
/// words; `None` for a thread not prepared here.
#[must_use]
pub fn stack_free_words(i: usize, limit: usize) -> Option<usize> {
    // SAFETY: as in `stack_intact`.
    let at = unsafe { addr_of_mut!(BOTTOMS[i]).read() };
    if at == 0 {
        return None;
    }
    // SAFETY: words of a live stack's memory, from its bottom up, at most
    // `limit` (the caller's stack length).
    Some(
        (0..limit)
            .take_while(|k| unsafe { (at as *const u32).add(*k).read_volatile() } == STACK_GUARD)
            .count(),
    )
}

/// Whether thread `i`'s stack still has its guard word (a thread never
/// prepared here has none to lose).
#[must_use]
pub fn stack_intact(i: usize) -> bool {
    // SAFETY: a plain read of the slot's word, written by `prepare`; the
    // address it holds is the bottom of a stack handed over for good.
    let at = unsafe { addr_of_mut!(BOTTOMS[i]).read() };
    // SAFETY: as above: a word of a live stack's memory.
    at == 0 || unsafe { (at as *const u32).read_volatile() } == STACK_GUARD
}

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
    // SAFETY: as for `SPS` below: this slot's thread isn't running.
    unsafe { addr_of_mut!(BOTTOMS[i]).write(base as u32) };
    let top = (((base + stack.len() * 4) & !7) - base) / 4 - SAVED_WORDS;
    // The whole stack below the first frame: the guard word, so how deep
    // the thread ever went can be read back (`stack_free_words`).
    stack[..top].fill(STACK_GUARD);
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
/// If either index is out of range or they're equal, or either thread's
/// stack has lost its guard word (it overflowed: stop here, before a
/// neighbour's saved context is used).
pub fn switch(from: usize, to: usize) {
    assert!(
        from < MAX_THREADS && to < MAX_THREADS && from != to,
        "bad switch"
    );
    assert!(
        stack_intact(from),
        "kernel stack overflowed (switching away)"
    );
    assert!(stack_intact(to), "kernel stack overflowed (switching to)");
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
