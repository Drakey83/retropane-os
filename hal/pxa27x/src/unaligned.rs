//! The ARM EABI's unaligned-access helpers (RTABI 4.3.4: `__aeabi_uread4`,
//! `__aeabi_uwrite4`, `__aeabi_uread8`, `__aeabi_uwrite8`), which LLVM
//! calls from code built at minimum size on a core without unaligned
//! loads (ARMv5): one call in place of four byte loads and three shifted
//! ORs at each site. Rust's `compiler_builtins` doesn't have them; the
//! card code built for size (rp-fat's `#[optimize(size)]`, R208) needs
//! them, in every image that links it, and every X51v image links this
//! crate's runtime. Weak: a copy elsewhere never clashes. Little-endian,
//! a byte at a time, any address; only r0-r3 and r12 touched, as the EABI
//! allows.

core::arch::global_asm!(
    r#"
    .arm
    .section .text.rp_unaligned,"ax",%progbits

    .weak __aeabi_uread4
    .type __aeabi_uread4, %function
    .p2align 2
__aeabi_uread4:                         @ r0 = address -> r0 the word
    ldrb    r1, [r0]
    ldrb    r2, [r0, #1]
    ldrb    r3, [r0, #2]
    ldrb    r12, [r0, #3]
    orr     r0, r1, r2, lsl #8
    orr     r0, r0, r3, lsl #16
    orr     r0, r0, r12, lsl #24
    bx      lr
    .size __aeabi_uread4, . - __aeabi_uread4

    .weak __aeabi_uwrite4
    .type __aeabi_uwrite4, %function
    .p2align 2
__aeabi_uwrite4:                        @ r0 = value, r1 = address -> r0 the value
    strb    r0, [r1]
    mov     r2, r0, lsr #8
    strb    r2, [r1, #1]
    mov     r2, r0, lsr #16
    strb    r2, [r1, #2]
    mov     r2, r0, lsr #24
    strb    r2, [r1, #3]
    bx      lr
    .size __aeabi_uwrite4, . - __aeabi_uwrite4

    .weak __aeabi_uread8
    .type __aeabi_uread8, %function
    .p2align 2
__aeabi_uread8:                         @ r0 = address -> r0:r1 the double word
    mov     r12, r0
    ldrb    r0, [r12]
    ldrb    r1, [r12, #1]
    ldrb    r2, [r12, #2]
    ldrb    r3, [r12, #3]
    orr     r0, r0, r1, lsl #8
    orr     r0, r0, r2, lsl #16
    orr     r0, r0, r3, lsl #24
    ldrb    r1, [r12, #4]
    ldrb    r2, [r12, #5]
    ldrb    r3, [r12, #6]
    orr     r1, r1, r2, lsl #8
    ldrb    r2, [r12, #7]
    orr     r1, r1, r3, lsl #16
    orr     r1, r1, r2, lsl #24
    bx      lr
    .size __aeabi_uread8, . - __aeabi_uread8

    .weak __aeabi_uwrite8
    .type __aeabi_uwrite8, %function
    .p2align 2
__aeabi_uwrite8:                        @ r0:r1 = value, r2 = address -> r0:r1 the value
    strb    r0, [r2]
    mov     r3, r0, lsr #8
    strb    r3, [r2, #1]
    mov     r3, r0, lsr #16
    strb    r3, [r2, #2]
    mov     r3, r0, lsr #24
    strb    r3, [r2, #3]
    strb    r1, [r2, #4]
    mov     r3, r1, lsr #8
    strb    r3, [r2, #5]
    mov     r3, r1, lsr #16
    strb    r3, [r2, #6]
    mov     r3, r1, lsr #24
    strb    r3, [r2, #7]
    bx      lr
    .size __aeabi_uwrite8, . - __aeabi_uwrite8
"#
);
