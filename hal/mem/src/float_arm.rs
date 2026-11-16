//! The float routines of [`crate::float`] in ARM assembly for the XScale
//! (docs/design/apps-and-packs.md, "Floating point in the ports", layer 2:
//! "hand-written ARM assembly routines ... the first measurement is how
//! far behind an ARM assembly version it is"). The same results bit for
//! bit (the Rust ones are the reference the PC checks against its own
//! IEEE hardware; these are checked against them and against the
//! compiler's generic routines on the X51v, `-rpfloat`).
//!
//! What the CPU gives that a generic routine does not use: the significand
//! kept at the top of a word, so an add's carry is the carry flag and comes
//! back in with one `rrx`; `clz` normalises a cancellation in one step;
//! conditional instructions in place of branches for the rounding and the
//! subnormal cases; `umull` makes the whole 48-bit product at once; and
//! nothing on the stack but two registers.
//!
//! `rp_fadd_asm(a, b)`, `rp_fsub_asm(a, b)`, `rp_fmul_asm(a, b)`: IEEE
//! single bits in r0 and r1, the result in r0 (AAPCS, soft-float).

core::arch::global_asm!(
    r#"
    .arm
    .text

    .global rp_fsub_asm
    .type rp_fsub_asm, %function
    .global rp_fadd_asm
    .type rp_fadd_asm, %function
    .p2align 2
rp_fsub_asm:
    eor     r1, r1, #0x80000000
rp_fadd_asm:
    @ The larger magnitude in r0 (x), the other in r1 (y); r2, r3 their
    @ magnitudes shifted left by one (the exponent in the top byte).
    mov     r2, r0, lsl #1
    mov     r3, r1, lsl #1
    cmp     r2, r3
    bhs     1f
    mov     r12, r0
    mov     r0, r1
    mov     r1, r12
    mov     r12, r2
    mov     r2, r3
    mov     r3, r12
1:  mov     r12, r2, lsr #24            @ r12 = ex
    cmp     r12, #255
    beq     .Lfadd_special
    push    {{r4, r5}}
    @ The significands at the top of a word: the leading one at bit 31
    @ (none for a subnormal, which takes the smallest normal's scale).
    mov     r4, r0, lsl #8
    cmp     r12, #0
    orrne   r4, r4, #0x80000000
    moveq   r12, #1
    movs    r3, r3, lsr #24             @ r3 = ey
    mov     r5, r1, lsl #8
    orrne   r5, r5, #0x80000000
    moveq   r3, #1
    sub     r3, r12, r3                 @ r3 = d = ex - ey
    cmp     r3, #25
    bhi     .Lfadd_x                    @ y below half x's spacing
    @ y lined up with x; a bit that goes kept as bit 0.
    rsb     r2, r3, #32
    mov     r2, r5, lsl r2              @ (a shift of 32 is 0: d = 0)
    mov     r5, r5, lsr r3
    cmp     r2, #0
    orrne   r5, r5, #1
    teq     r0, r1
    bmi     .Lfadd_sub
    adds    r4, r4, r5
    bcc     .Lfadd_round
    movs    r4, r4, rrx                 @ the carry back in at the top
    orrcs   r4, r4, #1                  @ the bit that went, kept
    add     r12, r12, #1
    cmp     r12, #255
    bne     .Lfadd_round
    and     r0, r0, #0x80000000         @ beyond the largest: infinity
    orr     r0, r0, #0x7F000000
    orr     r0, r0, #0x00800000
    pop     {{r4, r5}}
    bx      lr
.Lfadd_sub:
    subs    r4, r4, r5
    beq     .Lfadd_zero
    clz     r2, r4
    mov     r4, r4, lsl r2
    subs    r12, r12, r2
    bgt     .Lfadd_round
    @ Subnormal: back to the smallest normal's scale.
    rsb     r3, r12, #1
    rsb     r2, r3, #32
    mov     r2, r4, lsl r2
    mov     r4, r4, lsr r3
    cmp     r2, #0
    orrne   r4, r4, #1
    mov     r12, #1
.Lfadd_round:
    @ r4: the leading one at bit 31 (or none), 23 bits, then the 8 the
    @ rounding reads. The exponent field is e - 1 plus the leading one,
    @ so a rounding that carries out raises it (to infinity at 255).
    and     r1, r0, #0x80000000
    sub     r12, r12, #1
    mov     r2, r4, lsr #8
    add     r2, r2, r12, lsl #23
    and     r3, r4, #0xFF
    cmp     r3, #0x80
    addhi   r2, r2, #1
    bne     2f
    tst     r2, #1
    addne   r2, r2, #1
2:  orr     r0, r2, r1
    pop     {{r4, r5}}
    bx      lr
.Lfadd_x:
    pop     {{r4, r5}}
    bx      lr
.Lfadd_zero:
    mov     r0, #0                      @ x - x is +0
    pop     {{r4, r5}}
    bx      lr
.Lfadd_special:
    @ x is an infinity or a NaN (no registers saved yet).
    cmp     r2, #0xFF000000
    orrhi   r0, r0, #0x00400000         @ a NaN, quiet
    bxhi    lr
    cmp     r3, #0xFF000000
    bxne    lr
    teq     r0, r1
    bxpl    lr
    mov     r0, #0x7F000000             @ infinity minus infinity
    orr     r0, r0, #0x00C00000
    bx      lr
    .size rp_fadd_asm, . - rp_fadd_asm

    .global rp_fmul_asm
    .type rp_fmul_asm, %function
    .p2align 2
rp_fmul_asm:
    mov     r2, r0, lsr #23
    and     r2, r2, #0xFF               @ ea
    mov     r3, r1, lsr #23
    and     r3, r3, #0xFF               @ eb
    sub     r12, r2, #1
    cmp     r12, #254
    bhs     .Lfmul_special
    sub     r12, r3, #1
    cmp     r12, #254
    bhs     .Lfmul_special
    push    {{r4, r5}}
    eor     r12, r0, r1                 @ the sign at bit 31
    mov     r0, r0, lsl #8
    orr     r0, r0, #0x80000000
    mov     r1, r1, lsl #8
    orr     r1, r1, #0x80000000
    umull   r4, r5, r0, r1              @ r5:r4, the leading one at 63 or 62
    add     r2, r2, r3
    tst     r5, #0x80000000
    subne   r2, r2, #126
    subeq   r2, r2, #127
    moveq   r5, r5, lsl #1
    orreq   r5, r5, r4, lsr #31
    moveq   r4, r4, lsl #1
    cmp     r4, #0
    orrne   r5, r5, #1                  @ the low word, kept as one bit
    cmp     r2, #254
    bgt     .Lfmul_inf
    cmp     r2, #1
    bge     .Lfmul_round
    @ Subnormal or zero: shifted to the smallest normal's scale.
    rsb     r3, r2, #1
    cmp     r3, #32
    movhs   r5, #1                      @ all of it below: kept as one bit
    bhs     3f
    rsb     r1, r3, #32
    mov     r1, r5, lsl r1
    mov     r5, r5, lsr r3
    cmp     r1, #0
    orrne   r5, r5, #1
3:  mov     r2, #1
.Lfmul_round:
    and     r1, r12, #0x80000000
    sub     r2, r2, #1
    mov     r0, r5, lsr #8
    add     r0, r0, r2, lsl #23
    and     r3, r5, #0xFF
    cmp     r3, #0x80
    addhi   r0, r0, #1
    bne     4f
    tst     r0, #1
    addne   r0, r0, #1
4:  orr     r0, r0, r1
    pop     {{r4, r5}}
    bx      lr
.Lfmul_inf:
    and     r0, r12, #0x80000000
    orr     r0, r0, #0x7F000000
    orr     r0, r0, #0x00800000
    pop     {{r4, r5}}
    bx      lr
.Lfmul_special:
    @ A zero, a subnormal, an infinity or a NaN: rare, in Rust.
    b       rp_fmul_rs
    .size rp_fmul_asm, . - rp_fmul_asm
"#
);
