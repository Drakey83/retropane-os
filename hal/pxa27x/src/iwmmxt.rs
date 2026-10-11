//! Lazy iWMMXt context switching (ADR-0010 rule 3).
//!
//! The XScale's media unit has 16 64-bit registers plus control registers:
//! 152 bytes a thread would otherwise save and load on every switch. Most
//! threads never use it, so access is switched off (CP15 `CPAR` bits 0-1)
//! for every thread but the unit's current owner. Another thread's first
//! media instruction is undefined: [`rp_iwmmxt_trap`] saves the owner's
//! registers, loads the new thread's, makes it the owner and runs the
//! instruction again. A thread that never touches the unit costs nothing.
//!
//! State lives in a small pool ([`SLOTS`] threads at once, about 1.2 KiB),
//! not a slot for each of the 128 possible threads (19 KiB): memory first.
//!
//! The saturation flags (`wCSSF`) and arithmetic flags (`wCASF`) can only be
//! cleared, not set, by software, so they are restored as far as the
//! hardware allows (as in other kernels); the 16 data registers and the four
//! general control registers (`wCGR0`-`3`) are restored exactly.

use crate::thread::MAX_THREADS;

/// Threads that may hold media state at the same time.
pub const SLOTS: usize = 8;

/// One thread's media state (layout used by the assembly below).
#[repr(C)]
#[derive(Clone, Copy)]
struct State {
    wr: [u64; 16],
    cgr: [u32; 4],
    cssf: u32,
    casf: u32,
}

const NONE: usize = usize::MAX;

static mut POOL: [State; SLOTS] = [State {
    wr: [0; 16],
    cgr: [0; 4],
    cssf: 0,
    casf: 0,
}; SLOTS];
/// Per thread: its pool slot + 1 (0: none yet).
static mut SLOT_OF: [u8; MAX_THREADS] = [0; MAX_THREADS];
/// Bit `k` set: pool slot `k` is in use.
static mut SLOTS_USED: u8 = 0;
/// The thread whose state is in the unit now.
static mut OWNER: usize = NONE;
/// The running thread (kept by `thread::switch`).
static mut CURRENT: usize = 0;
static mut ENABLED: bool = false;
static mut TRAPS: u32 = 0;

core::arch::global_asm!(
    r#"
    .section .text.rp_iwmmxt, "ax"
    .arm
    @ The undefined-instruction vector while lazy switching is on: ask
    @ rp_iwmmxt_trap; a media instruction it switched in runs again,
    @ anything else goes to the normal entry (report and stop).
    .global rp_iwmmxt_undef
rp_iwmmxt_undef:
    stmfd   sp!, {{r0-r3, r12, lr}}
    sub     r0, lr, #4                  @ the instruction (ARM state)
    bl      rp_iwmmxt_trap
    cmp     r0, #0
    ldmfd   sp!, {{r0-r3, r12, lr}}
    subsne  pc, lr, #4
    b       rp_undef_entry

    @ rp_iwmmxt_save(state: *mut State)
    .global rp_iwmmxt_save
rp_iwmmxt_save:
    mrrc    p0, #0, r2, r3, c0
    str     r2, [r0, #0]
    str     r3, [r0, #4]
    mrrc    p0, #0, r2, r3, c1
    str     r2, [r0, #8]
    str     r3, [r0, #12]
    mrrc    p0, #0, r2, r3, c2
    str     r2, [r0, #16]
    str     r3, [r0, #20]
    mrrc    p0, #0, r2, r3, c3
    str     r2, [r0, #24]
    str     r3, [r0, #28]
    mrrc    p0, #0, r2, r3, c4
    str     r2, [r0, #32]
    str     r3, [r0, #36]
    mrrc    p0, #0, r2, r3, c5
    str     r2, [r0, #40]
    str     r3, [r0, #44]
    mrrc    p0, #0, r2, r3, c6
    str     r2, [r0, #48]
    str     r3, [r0, #52]
    mrrc    p0, #0, r2, r3, c7
    str     r2, [r0, #56]
    str     r3, [r0, #60]
    mrrc    p0, #0, r2, r3, c8
    str     r2, [r0, #64]
    str     r3, [r0, #68]
    mrrc    p0, #0, r2, r3, c9
    str     r2, [r0, #72]
    str     r3, [r0, #76]
    mrrc    p0, #0, r2, r3, c10
    str     r2, [r0, #80]
    str     r3, [r0, #84]
    mrrc    p0, #0, r2, r3, c11
    str     r2, [r0, #88]
    str     r3, [r0, #92]
    mrrc    p0, #0, r2, r3, c12
    str     r2, [r0, #96]
    str     r3, [r0, #100]
    mrrc    p0, #0, r2, r3, c13
    str     r2, [r0, #104]
    str     r3, [r0, #108]
    mrrc    p0, #0, r2, r3, c14
    str     r2, [r0, #112]
    str     r3, [r0, #116]
    mrrc    p0, #0, r2, r3, c15
    str     r2, [r0, #120]
    str     r3, [r0, #124]
    mrc     p1, #0, r2, c8, c0, #0
    str     r2, [r0, #128]
    mrc     p1, #0, r2, c9, c0, #0
    str     r2, [r0, #132]
    mrc     p1, #0, r2, c10, c0, #0
    str     r2, [r0, #136]
    mrc     p1, #0, r2, c11, c0, #0
    str     r2, [r0, #140]
    mrc     p1, #0, r2, c2, c0, #0
    str     r2, [r0, #144]
    mrc     p1, #0, r2, c3, c0, #0
    str     r2, [r0, #148]
    bx      lr

    @ rp_iwmmxt_load(state: *const State)
    .global rp_iwmmxt_load
rp_iwmmxt_load:
    ldr     r2, [r0, #0]
    ldr     r3, [r0, #4]
    mcrr    p0, #0, r2, r3, c0
    ldr     r2, [r0, #8]
    ldr     r3, [r0, #12]
    mcrr    p0, #0, r2, r3, c1
    ldr     r2, [r0, #16]
    ldr     r3, [r0, #20]
    mcrr    p0, #0, r2, r3, c2
    ldr     r2, [r0, #24]
    ldr     r3, [r0, #28]
    mcrr    p0, #0, r2, r3, c3
    ldr     r2, [r0, #32]
    ldr     r3, [r0, #36]
    mcrr    p0, #0, r2, r3, c4
    ldr     r2, [r0, #40]
    ldr     r3, [r0, #44]
    mcrr    p0, #0, r2, r3, c5
    ldr     r2, [r0, #48]
    ldr     r3, [r0, #52]
    mcrr    p0, #0, r2, r3, c6
    ldr     r2, [r0, #56]
    ldr     r3, [r0, #60]
    mcrr    p0, #0, r2, r3, c7
    ldr     r2, [r0, #64]
    ldr     r3, [r0, #68]
    mcrr    p0, #0, r2, r3, c8
    ldr     r2, [r0, #72]
    ldr     r3, [r0, #76]
    mcrr    p0, #0, r2, r3, c9
    ldr     r2, [r0, #80]
    ldr     r3, [r0, #84]
    mcrr    p0, #0, r2, r3, c10
    ldr     r2, [r0, #88]
    ldr     r3, [r0, #92]
    mcrr    p0, #0, r2, r3, c11
    ldr     r2, [r0, #96]
    ldr     r3, [r0, #100]
    mcrr    p0, #0, r2, r3, c12
    ldr     r2, [r0, #104]
    ldr     r3, [r0, #108]
    mcrr    p0, #0, r2, r3, c13
    ldr     r2, [r0, #112]
    ldr     r3, [r0, #116]
    mcrr    p0, #0, r2, r3, c14
    ldr     r2, [r0, #120]
    ldr     r3, [r0, #124]
    mcrr    p0, #0, r2, r3, c15
    ldr     r2, [r0, #128]
    mcr     p1, #0, r2, c8, c0, #0
    ldr     r2, [r0, #132]
    mcr     p1, #0, r2, c9, c0, #0
    ldr     r2, [r0, #136]
    mcr     p1, #0, r2, c10, c0, #0
    ldr     r2, [r0, #140]
    mcr     p1, #0, r2, c11, c0, #0
    ldr     r2, [r0, #144]
    mcr     p1, #0, r2, c2, c0, #0
    ldr     r2, [r0, #148]
    mcr     p1, #0, r2, c3, c0, #0
    bx      lr
    "#
);

unsafe extern "C" {
    fn rp_iwmmxt_undef();
    fn rp_iwmmxt_save(state: *mut State);
    fn rp_iwmmxt_load(state: *const State);
}

fn cpar() -> u32 {
    let v: u32;
    // SAFETY: reading CPAR has no side effects.
    unsafe {
        core::arch::asm!("mrc p15, 0, {v}, c15, c1, 0", v = out(reg) v, options(nomem, nostack))
    };
    v
}

/// Media-unit access on or off for the running thread.
fn set_access(on: bool) {
    let old = cpar();
    let new = if on { old | 3 } else { old & !3 };
    if new != old {
        // SAFETY: CPAR bits 0-1 gate coprocessors 0 and 1 (the media unit)
        // only; CPWAIT makes the change take effect before the next
        // instruction.
        unsafe {
            core::arch::asm!(
                "mcr p15, 0, {v}, c15, c1, 0",
                "mrc p15, 0, {t}, c2, c0, 0",
                "mov {t}, {t}",
                "sub pc, pc, #4",
                v = in(reg) new,
                t = out(reg) _,
                options(nomem, nostack),
            )
        };
    }
}

/// Switches the media unit to lazy switching: off for everyone until a
/// thread's first media instruction traps.
pub fn enable_lazy() {
    // SAFETY: setup, before threads use the unit; single core.
    unsafe {
        core::ptr::addr_of_mut!(ENABLED).write(true);
        core::ptr::addr_of_mut!(OWNER).write(NONE);
    }
    set_access(false);
    crate::user::set_vector(1, rp_iwmmxt_undef as *const () as usize as u32);
}

/// Called by `thread::switch` for the thread about to run.
pub(crate) fn on_switch(to: usize) {
    // SAFETY: plain words, touched only by the running thread with IRQs
    // masked around a switch (single core).
    unsafe {
        core::ptr::addr_of_mut!(CURRENT).write(to);
        if core::ptr::addr_of!(ENABLED).read() {
            set_access(core::ptr::addr_of!(OWNER).read() == to);
        }
    }
}

/// Thread `i` exited: its media state is dropped and its slot freed.
pub fn release(i: usize) {
    if i >= MAX_THREADS {
        return;
    }
    // SAFETY: as in `on_switch`.
    unsafe {
        let s = core::ptr::addr_of!(SLOT_OF[i]).read();
        if s != 0 {
            let used = core::ptr::addr_of_mut!(SLOTS_USED);
            used.write(used.read() & !(1 << (s - 1)));
        }
        core::ptr::addr_of_mut!(SLOT_OF[i]).write(0);
        if core::ptr::addr_of!(OWNER).read() == i {
            core::ptr::addr_of_mut!(OWNER).write(NONE);
        }
    }
}

/// How many first-use traps there were (each one a lazy switch).
#[must_use]
pub fn traps() -> u32 {
    // SAFETY: a plain read.
    unsafe { core::ptr::addr_of!(TRAPS).read() }
}

/// Is `insn` a coprocessor 0 or 1 instruction (the media unit)?
fn is_media(insn: u32) -> bool {
    let class = (insn >> 24) & 0xF;
    (0xC..=0xE).contains(&class) && (insn >> 8) & 0xF <= 1 && insn >> 28 != 0xF
}

/// The undefined-instruction hook: `pc` is the instruction that trapped.
/// Returns whether it was a lazy media switch (then it runs again).
#[unsafe(no_mangle)]
extern "C" fn rp_iwmmxt_trap(pc: u32) -> bool {
    // SAFETY: `pc` is the address of the instruction that just trapped, so
    // it's mapped and readable (identity-mapped code in the bring-up map).
    let insn = unsafe { (pc as *const u32).read_volatile() };
    // SAFETY: the undefined-instruction handler runs with IRQs masked on one
    // core; nothing else touches these statics meanwhile.
    unsafe {
        let cur = core::ptr::addr_of!(CURRENT).read();
        let owner = core::ptr::addr_of!(OWNER).read();
        if !is_media(insn) || owner == cur || cur >= MAX_THREADS {
            return false; // a genuinely undefined instruction
        }
        set_access(true);
        if owner != NONE {
            let s = core::ptr::addr_of!(SLOT_OF[owner]).read();
            if s != 0 {
                rp_iwmmxt_save(core::ptr::addr_of_mut!(POOL[usize::from(s) - 1]));
            }
        }
        let mut s = core::ptr::addr_of!(SLOT_OF[cur]).read();
        if s == 0 {
            let used = core::ptr::addr_of_mut!(SLOTS_USED);
            let free = (!used.read()).trailing_zeros() as usize;
            if free >= SLOTS {
                set_access(false);
                return false; // every slot taken: reported as a fault
            }
            used.write(used.read() | 1 << free);
            // A thread's first use starts from zeroed registers.
            core::ptr::write_bytes(core::ptr::addr_of_mut!(POOL[free]), 0, 1);
            s = free as u8 + 1;
            core::ptr::addr_of_mut!(SLOT_OF[cur]).write(s);
        }
        rp_iwmmxt_load(core::ptr::addr_of!(POOL[usize::from(s) - 1]));
        core::ptr::addr_of_mut!(OWNER).write(cur);
        core::ptr::addr_of_mut!(TRAPS).write(core::ptr::addr_of!(TRAPS).read() + 1);
    }
    true
}
