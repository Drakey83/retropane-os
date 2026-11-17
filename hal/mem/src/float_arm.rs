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
//! `rp_fadd_asm(a, b)`, `rp_fsub_asm(a, b)`, `rp_fmul_asm(a, b)`,
//! `rp_fdiv_asm(a, b)`: IEEE
//! single bits in r0 and r1, the result in r0 (AAPCS, soft-float).
//!
//! `rp_fcmp{eq,lt,le,gt,ge}_asm(a, b)` (kit 70): the compares, 1 or 0,
//! the AEABI's answers (a NaN: 0), for the modules' `__aeabi_fcmp*`.
//!
//! `rp_fdot3_asm(a, b)` (R316, kit 65): C's `(a[0] * b[0] + a[1] * b[1])
//! + a[2] * b[2]` for two `float[3]` (pointers in r0 and r1), bit for bit,
//! in one call: each operand unpacked once, the three products and two
//! sums rounded to nearest even exactly as `rp_fmul_asm` and `rp_fadd_asm`
//! round them, the values kept unpacked between the steps (exponent,
//! significand at the top of a word, sign), no branch on the common path
//! but the rare cases' exits. A zero product is made in place (axial
//! normals have two); anything else off the common path (a subnormal, an
//! infinity, a NaN, a product or sum past 2^126 or below 2^-126) goes the
//! C way, the five calls in C's order, so the answer is always C's. The
//! sum is the C reference's (`ports/xash/include/rp_fdot.h`, kit 64:
//! 10^9 vectors on the PC against IEEE hardware): the significands at bits
//! 30 and down, the smaller shifted with a sticky bit, `clz` normalising
//! a carry, a plain sum and a cancellation alike.

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
    @ A zero times a finite number is the signed zero, here: a game
    @ multiplies by zero often (run 6 on the X51v: 6.4% of a frame went to
    @ the rare road for it). The rest (a subnormal, an infinity, a NaN)
    @ in Rust.
    mov     r2, r0, lsl #1
    mov     r3, r1, lsl #1
    cmp     r2, #0
    cmpne   r3, #0
    bne     rp_fmul_rs
    cmp     r2, #0xFF000000
    cmplo   r3, #0xFF000000
    bhs     rp_fmul_rs
    eor     r0, r0, r1
    and     r0, r0, #0x80000000
    bx      lr
    .size rp_fmul_asm, . - rp_fmul_asm

    .global rp_fdiv_asm
    .type rp_fdiv_asm, %function
    .p2align 2
rp_fdiv_asm:
    @ As `float::div`: the reciprocal of the divisor from a table and two
    @ Newton steps, the quotient from one multiply, made exact by its
    @ remainder (the first guess is never more than one off: a test
    @ checks every divisor significand).
    mov     r2, r0, lsr #23
    and     r2, r2, #0xFF               @ ea
    mov     r3, r1, lsr #23
    and     r3, r3, #0xFF               @ eb
    sub     r12, r2, #1
    cmp     r12, #254
    bhs     .Lfdiv_special
    sub     r12, r3, #1
    cmp     r12, #254
    bhs     .Lfdiv_special
    push    {{r4, r5, r6, r7}}
    eor     r7, r0, r1                  @ the sign at bit 31
    sub     r2, r2, r3
    add     r2, r2, #127                @ e
    mov     r0, r0, lsl #9
    mov     r0, r0, lsr #9
    orr     r0, r0, #0x00800000         @ n
    mov     r1, r1, lsl #9
    mov     r1, r1, lsr #9
    orr     r1, r1, #0x00800000         @ d
    cmp     r0, r1
    movlo   r0, r0, lsl #1
    sublo   r2, r2, #1
    cmp     r1, #0x00800000
    beq     .Lfdiv_pow2
    mov     r3, r1, lsl #8              @ D
    ldr     r12, =RP_FLOAT_RECIP
    mov     r4, r3, lsr #23
    and     r4, r4, #0xFF
    add     r4, r4, r4
    ldrh    r4, [r12, r4]
    mov     r4, r4, lsl #16             @ Y, the first guess
    @ Two Newton steps: Y + Y(1 - DY), 1 - DY in units of 2^-31.
    umull   r5, r6, r3, r4
    rsbs    r5, r6, #0x80000000
    rsbmi   r5, r5, #0
    umull   r6, r12, r4, r5
    mov     r12, r12, lsl #1
    orr     r12, r12, r6, lsr #31
    addpl   r4, r4, r12
    submi   r4, r4, r12
    umull   r5, r6, r3, r4
    rsbs    r5, r6, #0x80000000
    rsbmi   r5, r5, #0
    umull   r6, r12, r4, r5
    mov     r12, r12, lsl #1
    orr     r12, r12, r6, lsr #31
    addpl   r4, r4, r12
    submi   r4, r4, r12
    @ q = n * Y / 2^30, then its remainder (n * 2^25 - q * d).
    umull   r5, r6, r0, r4
    mov     r6, r6, lsl #2
    orr     r6, r6, r5, lsr #30
    mul     r5, r6, r1
    rsb     r5, r5, r0, lsl #25
    cmp     r5, #0
    sublt   r6, r6, #1
    addlt   r5, r5, r1
    cmp     r5, r1
    addge   r6, r6, #1
    subge   r5, r5, r1
5:  @ r6: the 26-bit quotient; r5: its remainder (kept as bit 0).
    cmp     r5, #0
    mov     r5, r6, lsl #6
    orrne   r5, r5, #1
    cmp     r2, #254
    bgt     .Lfdiv_inf
    cmp     r2, #1
    bge     .Lfdiv_round
    rsb     r3, r2, #1
    cmp     r3, #32
    movhs   r5, #1
    bhs     6f
    rsb     r1, r3, #32
    mov     r1, r5, lsl r1
    mov     r5, r5, lsr r3
    cmp     r1, #0
    orrne   r5, r5, #1
6:  mov     r2, #1
.Lfdiv_round:
    and     r1, r7, #0x80000000
    sub     r2, r2, #1
    mov     r0, r5, lsr #8
    add     r0, r0, r2, lsl #23
    and     r3, r5, #0xFF
    cmp     r3, #0x80
    addhi   r0, r0, #1
    bne     7f
    tst     r0, #1
    addne   r0, r0, #1
7:  orr     r0, r0, r1
    pop     {{r4, r5, r6, r7}}
    bx      lr
.Lfdiv_pow2:
    mov     r6, r0, lsl #2
    mov     r5, #0
    b       5b
.Lfdiv_inf:
    and     r0, r7, #0x80000000
    orr     r0, r0, #0x7F000000
    orr     r0, r0, #0x00800000
    pop     {{r4, r5, r6, r7}}
    bx      lr
.Lfdiv_special:
    @ A zero divided by a finite number other than zero is the signed
    @ zero, here; the rest (subnormals, infinities, NaNs, a zero
    @ divisor) in Rust.
    mov     r2, r0, lsl #1
    mov     r3, r1, lsl #1
    cmp     r2, #0
    bne     rp_fdiv_rs
    cmp     r3, #0
    beq     rp_fdiv_rs
    cmp     r3, #0xFF000000
    bhs     rp_fdiv_rs
    eor     r0, r0, r1
    and     r0, r0, #0x80000000
    bx      lr
    .ltorg
    .size rp_fdiv_asm, . - rp_fdiv_asm
    @ ---- rp_fdot3_asm (R316) ----
    @ A product: x = a[off], y = b[off], both normal (else .Lfdot_z<id>);
    @ the result unpacked: E its exponent (1..254), M its significand at
    @ bits 31..8 (low byte zero), S's bit 31 its sign. Uses r2, r3, r12, lr.
    .macro RP_FDOT_MUL id, off, E, M, S
    ldr     r2, [r0, #\off]
    ldr     r3, [r1, #\off]
    mov     \E, r2, lsl #1
    mov     r12, r3, lsl #1
    sub     lr, \E, #0x01000000         @ a normal x: 0 .. 0xFDFFFFFF
    cmp     lr, #0xFE000000
    sublo   lr, r12, #0x01000000
    cmplo   lr, #0xFE000000
    bhs     .Lfdot_z\id                 @ a zero (or a rare case)
    eor     \S, r2, r3
    mov     \E, \E, lsr #24
    add     \E, \E, r12, lsr #24        @ ea + eb
    mov     r2, r2, lsl #8
    orr     r2, r2, #0x80000000
    mov     r3, r3, lsl #8
    orr     r3, r3, #0x80000000
    umull   lr, \M, r2, r3              @ M:lr, the leading one at 63 or 62
    tst     \M, #0x80000000
    subne   \E, \E, #126
    subeq   \E, \E, #127
    moveq   \M, \M, lsl #1
    orreq   \M, \M, lr, lsr #31
    moveq   lr, lr, lsl #1
    cmp     lr, #0
    orrne   \M, \M, #1                  @ the low word, kept as one bit
    sub     r12, \E, #1                 @ before rounding, as C: 1..253
    cmp     r12, #253
    bhs     .Lfdot_slow
    @ To 24 bits, nearest even: + 0x7F + the kept lowest bit; a carry out
    @ of the word is 2.0, one exponent up.
    and     r12, \M, #0x100
    adds    \M, \M, r12, lsr #8
    addscc  \M, \M, #0x7F
    movcs   \M, \M, rrx
    addcs   \E, \E, #1
    bic     \M, \M, #0xFF
.Lfdot_m\id:
    .endm

    @ The rare exit of a product: a zero times a finite number is the
    @ signed zero (E = M = 0); anything else goes the C way.
    .macro RP_FDOT_ZERO id, E, M, S
.Lfdot_z\id:
    cmp     \E, #0                      @ x << 1
    cmpne   r12, #0                     @ y << 1
    bne     .Lfdot_slow
    cmp     \E, #0xFF000000
    cmplo   r12, #0xFF000000
    bhs     .Lfdot_slow
    eor     \S, r2, r3
    mov     \E, #0
    mov     \M, #0
    b       .Lfdot_m\id
    .endm

    @ x + y, x = (r4, r5, r6), y = (r7, r8, r9), into x; the last one
    @ packs the float into r0 and returns. Uses r2, r3, r12.
    .macro RP_FDOT_ADD id, last
    eor     r12, r6, r9                 @ bit 31: the signs differ
    subs    r2, r4, r7                  @ d = ex - ey
    rsbmi   r2, r2, #0
    movmi   r4, r7                      @ the larger exponent's in x
    movmi   r6, r9
    movmi   r3, r5
    movmi   r5, r8
    movmi   r8, r3
    @ Both significands to bit 30, y's d further with the bits that go
    @ kept as one (a d of 31 or more leaves only that bit).
    add     r2, r2, #1
    cmp     r2, #32
    movhi   r2, #32
    rsb     r3, r2, #32
    mov     r3, r8, lsl r3
    mov     r8, r8, lsr r2
    cmp     r3, #0
    orrne   r8, r8, #1
    tst     r12, #0x80000000
    addeq   r5, r8, r5, lsr #1
    rsbsne  r5, r8, r5, lsr #1
    rsbmi   r5, r5, #0                  @ y was the larger: its sign
    eormi   r6, r6, #0x80000000
    clz     r2, r5                      @ a carry (0), none (1), a cancellation
    movs    r5, r5, lsl r2
    beq     .Lfdot_s\id                 @ an exact zero
    rsb     r2, r2, #1
    add     r4, r4, r2
    sub     r12, r4, #1                 @ before rounding: 1..253
    cmp     r12, #253
    bhs     .Lfdot_slow
    .if \last
    @ Rounded into the float: the exponent field e - 1 plus the leading
    @ one, so a carry raises it.
    mov     r0, r5, lsr #8
    add     r0, r0, r12, lsl #23
    and     r3, r5, #0xFF
    and     r2, r0, #1
    add     r3, r3, r2
    cmp     r3, #0x80
    addhi   r0, r0, #1
    and     r6, r6, #0x80000000
    orr     r0, r0, r6
    pop     {{r4, r5, r6, r7, r8, r9, pc}}
    .else
    and     r12, r5, #0x100
    adds    r5, r5, r12, lsr #8
    addscc  r5, r5, #0x7F
    movcs   r5, r5, rrx
    addcs   r4, r4, #1
    bic     r5, r5, #0xFF
.Lfdot_a\id:
    .endif
    .endm

    .global rp_fdot3_asm
    .type rp_fdot3_asm, %function
    .p2align 2
rp_fdot3_asm:
    push    {{r4, r5, r6, r7, r8, r9, lr}}
    RP_FDOT_MUL 1, 0, r4, r5, r6
    RP_FDOT_MUL 2, 4, r7, r8, r9
    RP_FDOT_ADD 1, 0
    RP_FDOT_MUL 3, 8, r7, r8, r9
    RP_FDOT_ADD 2, 1
    @ The rare exits, out of the line.
    RP_FDOT_ZERO 1, r4, r5, r6
    RP_FDOT_ZERO 2, r7, r8, r9
    RP_FDOT_ZERO 3, r7, r8, r9
.Lfdot_s1:
    @ x + y exactly zero: +0 when the signs differ, else their sign.
    tst     r12, #0x80000000
    movne   r6, #0
    mov     r4, #0
    b       .Lfdot_a1
.Lfdot_s2:
    tst     r12, #0x80000000
    movne   r6, #0
    and     r0, r6, #0x80000000
    pop     {{r4, r5, r6, r7, r8, r9, pc}}
.Lfdot_slow:
    @ C's own expression, the five calls in its order (r0, r1 untouched).
    pop     {{r4, r5, r6, r7, r8, r9, lr}}
    push    {{r4, r5, r6, lr}}
    mov     r4, r0
    mov     r5, r1
    ldr     r0, [r4]
    ldr     r1, [r5]
    bl      rp_fmul_asm
    mov     r6, r0
    ldr     r0, [r4, #4]
    ldr     r1, [r5, #4]
    bl      rp_fmul_asm
    mov     r1, r0
    mov     r0, r6
    bl      rp_fadd_asm
    mov     r6, r0
    ldr     r0, [r4, #8]
    ldr     r1, [r5, #8]
    bl      rp_fmul_asm
    mov     r1, r0
    mov     r0, r6
    bl      rp_fadd_asm
    pop     {{r4, r5, r6, pc}}
    .size rp_fdot3_asm, . - rp_fdot3_asm

    @ ---- the compares (kit 70) ----
    @ IEEE single bits in r0, r1; 1 if the relation holds, else 0 (a NaN:
    @ never). Both zeros are equal; otherwise each float's bits made an
    @ ordered signed integer (a negative's magnitude bits flipped), one
    @ signed compare. Common prefix: NaN out, zeros apart.
    .macro RP_FCMP name, zero, cond
    .global \name
    .type \name, %function
    .p2align 2
\name:
    mov     r2, r0, lsl #1
    mov     r3, r1, lsl #1
    cmp     r2, #0xFF000000
    cmpls   r3, #0xFF000000
    movhi   r0, #0                      @ unordered
    bxhi    lr
    orrs    r12, r2, r3
    moveq   r0, #\zero                  @ +-0 and +-0: equal
    bxeq    lr
    mov     r12, r0, asr #31
    eor     r0, r0, r12, lsr #1
    mov     r12, r1, asr #31
    eor     r1, r1, r12, lsr #1
    cmp     r0, r1
    mov     r0, #0
    mov\cond r0, #1
    bx      lr
    .size \name, . - \name
    .endm

    RP_FCMP rp_fcmplt_asm, 0, lt
    RP_FCMP rp_fcmple_asm, 1, le
    RP_FCMP rp_fcmpgt_asm, 0, gt
    RP_FCMP rp_fcmpge_asm, 1, ge
    RP_FCMP rp_fcmpeq_asm, 1, eq
"#
);
