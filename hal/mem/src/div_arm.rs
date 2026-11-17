//! 32-bit integer division in ARM assembly for the XScale (no divide
//! instruction on ARMv5), R306's FPS work after run 51: the division
//! helpers were 3.4% of Half-Life's play (`__udivmodsi4` 765 samples,
//! `__aeabi_idiv` 593, `__aeabi_idivmod` 460, `__aeabi_uidivmod` 137).
//!
//! Iterate, don't imitate (rule 0). The vendor way: Windows Mobile's
//! `coredll` and the ARM C library divide with a shift-subtract loop; the
//! textbook routine Rust's `compiler_builtins` gives this target (what ran
//! in run 51) finds the quotient's length with `clz` and then loops, seven
//! instructions and a taken branch a quotient bit, behind two calls
//! (`__aeabi_idiv` -> `__aeabi_idivmod` -> `__udivmodsi4`). Ours: the same
//! `clz` start, then a jump straight into a 32-step unrolled table of
//! three conditional instructions a bit (`cmp` / `adc` / `subcs`: no
//! branch, no flag lost), the signed forms around the one body with their
//! signs in a register, and the quotient and remainder both in r0 and r1
//! as the ARM EABI asks (`__aeabi_uidivmod`, `__aeabi_idivmod`), so the
//! `%` beside a `/` costs nothing more. About 3 cycles a quotient bit
//! against ~8, and the same results bit for bit (the X51v checks them
//! against the compiler's own routines at every `-rpfloat` run).
//!
//! `rp_udiv_asm(n, d)` -> r0 the quotient, r1 the remainder;
//! `rp_idiv_asm(n, d)` -> the same, signed (C's truncation: the remainder
//! takes the dividend's sign). A zero divisor gives a zero quotient and
//! the dividend as the remainder (C leaves it undefined; nothing traps).

core::arch::global_asm!(
    r#"
    .arm
    .text

    @ The unsigned body: r0 the dividend (becomes the remainder), r1 the
    @ divisor (non-zero, not above r0), r2 the quotient, r3 scratch.
    .macro RP_UDIV_BODY
    clz     r3, r0
    clz     r2, r1
    sub     r3, r2, r3              @ k: the quotient's top bit
    rsbs    r3, r3, #31             @ steps skipped: 31 - k
    addne   r3, r3, r3, lsl #1      @ x 3 instructions a step
    mov     r2, #0
    addne   pc, pc, r3, lsl #2      @ into the table at bit k
    nop
    .set    rp_shift, 32
    .rept   32
    .set    rp_shift, rp_shift - 1
    cmp     r0, r1, lsl #rp_shift
    adc     r2, r2, r2
    subcs   r0, r0, r1, lsl #rp_shift
    .endr
    .endm

    .global rp_udiv_asm
    .type rp_udiv_asm, %function
    .p2align 2
rp_udiv_asm:
    cmp     r1, #0
    beq     .Lrp_udiv_zero
    cmp     r0, r1
    blo     .Lrp_udiv_less
    RP_UDIV_BODY
    mov     r1, r0
    mov     r0, r2
    bx      lr
.Lrp_udiv_less:
    mov     r1, r0
    mov     r0, #0
    bx      lr
.Lrp_udiv_zero:
    mov     r1, r0
    mov     r0, #0
    bx      lr
    .size rp_udiv_asm, . - rp_udiv_asm

    .global rp_idiv_asm
    .type rp_idiv_asm, %function
    .p2align 2
rp_idiv_asm:
    @ r12: bit 31 the dividend's sign (the remainder's), bit 30 the
    @ divisor's; both made positive.
    ands    r12, r0, #0x80000000
    rsbne   r0, r0, #0
    cmp     r1, #0
    beq     .Lrp_idiv_zero
    orrlt   r12, r12, #0x40000000
    rsblt   r1, r1, #0
    cmp     r0, r1
    blo     .Lrp_idiv_less
    RP_UDIV_BODY
    @ the quotient's sign: the two signs differ
    eor     r3, r12, r12, lsl #1
    tst     r3, #0x80000000
    rsbne   r2, r2, #0
    tst     r12, #0x80000000
    rsbne   r1, r0, #0
    moveq   r1, r0
    mov     r0, r2
    bx      lr
.Lrp_idiv_less:
    tst     r12, #0x80000000
    rsbne   r1, r0, #0
    moveq   r1, r0
    mov     r0, #0
    bx      lr
.Lrp_idiv_zero:
    tst     r12, #0x80000000
    rsbne   r1, r0, #0
    moveq   r1, r0
    mov     r0, #0
    bx      lr
    .size rp_idiv_asm, . - rp_idiv_asm
"#
);

/// The ARM EABI names the compiler calls for `/` and `%`, in place of
/// `compiler_builtins`' (exported with the `float` feature only: the game
/// modules' build, as the float routines; every other image keeps the
/// routines it was measured with).
#[cfg(feature = "float")]
core::arch::global_asm!(
    r#"
    .arm
    .text
    .global __aeabi_uidiv
    .type __aeabi_uidiv, %function
    .global __aeabi_uidivmod
    .type __aeabi_uidivmod, %function
    .p2align 2
__aeabi_uidiv:
__aeabi_uidivmod:
    b       rp_udiv_asm
    .size __aeabi_uidiv, . - __aeabi_uidiv

    .global __aeabi_idiv
    .type __aeabi_idiv, %function
    .global __aeabi_idivmod
    .type __aeabi_idivmod, %function
    .p2align 2
__aeabi_idiv:
__aeabi_idivmod:
    b       rp_idiv_asm
    .size __aeabi_idiv, . - __aeabi_idiv
"#
);
