//! Two tiny user programs for the kernel image's IPC test (ADR-0016): a
//! client and a server. The kernel copies their bytes into a frame of its
//! own and maps that into two process slots as user code, as a loader
//! would: no page of the kernel's image is ever mapped for user mode, and
//! the image carries ~180 bytes instead of a page-aligned page (which cost
//! 7.6 KB with its alignment padding). Written in assembly, position
//! independent, so they call nothing in the kernel.
//!
//! The client, given a handle in `r0`:
//! 1. writes `0x1111_1111` at `DATA` (in its own slot);
//! 2. calls the server `ROUNDS` times with 20 and 22 (each must give 42);
//! 3. calls with 0, 0, which makes the server fault (an error comes back);
//! 4. calls through handle 9, which it doesn't have (an error comes back);
//! 5. reads `DATA` back (still its own `0x1111_1111`, though the server
//!    wrote `0x2222_2222` at the same address in its slot);
//! 6. calls with 20, 22 and a nesting count of 3: the server calls itself
//!    3 more times (4 calls deep, the limit) and each level adds 1: 45;
//! 7. the same with 4: the fifth level is refused (`TooDeep`), and the
//!    error comes back up unchanged;
//! 8. reports: `SYS_DONE` with the last sum, both errors, its word, and
//!    the two nested results in `r4`, `r5`.
//!
//! The server, given the arguments in `r1`, `r2` and a nesting count in
//! `r3`: with a count, calls itself (handle 0, its own port) with the
//! count less one and returns the result plus one (errors unchanged);
//! else faults if `r1`, `r2` are both zero (a load from an unmapped
//! address), else writes `0x2222_2222` at `DATA` and returns their sum.

/// System calls the programs make (`r7`).
pub const SYS_CALL: u32 = 1;
/// Return from a call (server).
pub const SYS_RETURN: u32 = 2;
/// The client's report.
pub const SYS_DONE: u32 = 3;
/// Where each program keeps its word (in its own slot).
pub const DATA: u32 = 0x0002_0000;
/// Calls the client makes for timing.
pub const ROUNDS: u32 = 1000;

core::arch::global_asm!(
    r#"
    .section .rodata.rp_user, "a"
    .arm
    .balign 4
    .global rp_user_page
    .global rp_ipc_client
    .global rp_ipc_server
rp_user_page:
rp_ipc_client:
    mov     r4, r0                  @ the handle
    ldr     r5, =0x00020000
    ldr     r6, =0x11111111
    str     r6, [r5]
    ldr     r9, =1000
1:  mov     r0, r4
    mov     r1, #20
    mov     r2, #22
    mov     r3, #0                  @ no nesting
    mov     r7, #1                  @ SYS_CALL
    swi     #0
    mov     r8, r0                  @ the sum
    subs    r9, r9, #1
    bne     1b
    mov     r0, r4                  @ make the server fault
    mov     r1, #0
    mov     r2, #0
    mov     r7, #1
    swi     #0
    mov     r10, r0                 @ the error
    mov     r0, #9                  @ a handle we don't have
    mov     r1, #1
    mov     r2, #1
    mov     r7, #1
    swi     #0
    mov     r11, r0                 @ the error
    ldr     r6, [r5]                @ our word, untouched
    mov     r0, r4                  @ nested: 4 calls deep
    mov     r1, #20
    mov     r2, #22
    mov     r3, #3
    mov     r7, #1
    swi     #0
    mov     r9, r0
    mov     r0, r4                  @ nested one level too deep
    mov     r1, #20
    mov     r2, #22
    mov     r3, #4
    mov     r7, #1
    swi     #0
    mov     r12, r0
    mov     r4, r9
    mov     r5, r12
    mov     r0, r8
    mov     r1, r10
    mov     r2, r11
    mov     r3, r6
    mov     r7, #3                  @ SYS_DONE
    swi     #0
2:  b       2b

rp_ipc_server:
    cmp     r3, #0
    beq     4f
    mov     r0, #0                  @ our own port
    sub     r3, r3, #1
    mov     r7, #1                  @ SYS_CALL (r1, r2 unchanged)
    swi     #0
    cmn     r0, #256                @ an error (0xFFFFFFxx) goes back as is
    addcc   r0, r0, #1
    mov     r7, #2                  @ SYS_RETURN
    swi     #0
4:  orrs    r3, r1, r2
    ldreq   r3, =0xDEAD0000         @ both zero: fault
    ldreq   r3, [r3]
    ldr     r5, =0x00020000
    ldr     r6, =0x22222222
    str     r6, [r5]
    add     r0, r1, r2
    mov     r7, #2                  @ SYS_RETURN
    swi     #0
3:  b       3b
    .ltorg
    .balign 4
    .global rp_user_page_end
rp_user_page_end:
    .text
"#
);

unsafe extern "C" {
    static rp_user_page: u32;
    static rp_user_page_end: u32;
    static rp_ipc_client: u8;
    static rp_ipc_server: u8;
}

/// The programs as words (to copy into a user frame), and the client's and
/// server's offsets in them.
#[must_use]
pub fn ipc_programs() -> (&'static [u32], u32, u32) {
    let start = core::ptr::addr_of!(rp_user_page);
    let (end, client, server) = (
        core::ptr::addr_of!(rp_user_page_end) as usize,
        core::ptr::addr_of!(rp_ipc_client) as u32,
        core::ptr::addr_of!(rp_ipc_server) as u32,
    );
    let words = (end - start as usize) / 4;
    // SAFETY: `rp_user_page..rp_user_page_end` is the assembled block
    // above: word aligned, a whole number of words, read-only, in the
    // image for its whole life.
    let code = unsafe { core::slice::from_raw_parts(start, words) };
    (code, client - start as u32, server - start as u32)
}
