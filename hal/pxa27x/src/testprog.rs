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
//! 8. follows two absolute addresses (`RELOC_LITERAL`'s, to the image's
//!    `.data`, and the pointer there, to `.rdata`) to [`MAGIC`]: right only
//!    if the loader relocated both;
//! 9. reports: `SYS_DONE` with the last sum, both errors, its word, the two
//!    nested results in `r4`, `r5` and the word it reached in `r6`.
//!
//! The server, given the arguments in `r1`, `r2` and a nesting count in
//! `r3`: with a count, calls itself (handle 0, its own port) with the
//! count less one and returns the result plus one (errors unchanged);
//! else faults if `r1`, `r2` are both zero (a load from an unmapped
//! address), else writes `0x2222_2222` at `DATA` and returns their sum.
//!
//! The same server code also runs as a supervised *host* (ADR-0016 step
//! 3), and a third program is the *service manager*: called with `OPEN` in
//! `r1`, it grants its caller (whose slot the kernel puts in `r0`) a
//! `CALL` handle to the host and returns the handle's number. Client 0
//! (its role is `r0 >> 8` at start) then, before step 5: opens the host
//! through the manager, calls it (7), makes it fault (the host restarts),
//! calls through the old handle (`Stale`), opens it again and calls (7),
//! keeping the five results at `RESTART_RESULTS`; then asks the manager
//! to `SPAWN` a worker twice (keeping the two slots it got after them).
//!
//! The *worker* (a process the manager starts with `SYS_SPAWN`) reads its
//! data word (a new process's memory must be zero), leaves a mark there
//! for the next process in its slot not to see, checks that a handle it was
//! never given is refused, and exits with [`WORKER_OK`] if both held.

/// System calls the programs make (`r7`).
pub const SYS_CALL: u32 = 1;
/// Return from a call (server).
pub const SYS_RETURN: u32 = 2;
/// The client's report.
pub const SYS_DONE: u32 = 3;
/// Hand a handle on (`r0` handle, `r1` slot, `r2` rights).
pub const SYS_GRANT: u32 = 4;
/// The manager's operation: open the host service.
pub const OPEN: u32 = 1;
/// The manager's operation: start a worker process.
pub const SPAWN: u32 = 2;
/// The manager's operation: open the display host (its handle 2; handle 1
/// is its own port).
pub const OPEN_DISPLAY: u32 = 3;
/// End the calling process (`r0` exit code).
pub const SYS_EXIT: u32 = 5;
/// Start a process from the image (`r0` entry offset in its code; the
/// service manager only).
pub const SYS_SPAWN: u32 = 6;
/// A worker's exit code when its checks held.
pub const WORKER_OK: u32 = 0x600D;
/// Where client 0 keeps its restart test's five results (its own slot).
pub const RESTART_RESULTS: u32 = DATA + 16;
/// Where each program keeps its word (in its own slot).
pub const DATA: u32 = 0x0002_0000;
/// Calls the client makes for timing.
pub const ROUNDS: u32 = 1000;
/// The image the programs are packaged in (by the kernel's test) prefers
/// this base, with `.text` at RVA `0x1000`, `.rdata` at `0x2000` and
/// `.data` at `0x3000`.
pub const PREFERRED_BASE: u32 = 0x0001_0000;
/// The word at the start of `.rdata` the client must reach.
pub const MAGIC: u32 = 0x7E57_DA7A;

core::arch::global_asm!(
    r#"
    .section .rodata.rp_user, "a"
    .arm
    .balign 4
    .global rp_user_page
    .global rp_ipc_client
    .global rp_ipc_server
    .global rp_ipc_manager
    .global rp_ipc_worker
rp_user_page:
rp_ipc_client:
    and     r4, r0, #0xFF           @ the handle
    ldr     r5, =0x00020000
    lsr     r12, r0, #8             @ our role (a register: threads share memory)
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
    cmp     r12, #0                 @ client 0: the supervised host
    bne     8f
    mov     r0, #1                  @ the manager: open the host
    mov     r1, #1
    mov     r7, #1
    swi     #0
    str     r0, [r5, #16]
    mov     r6, r0
    mov     r1, #3                  @ 3 + 4
    mov     r2, #4
    mov     r3, #0
    mov     r7, #1
    swi     #0
    str     r0, [r5, #20]
    mov     r0, r6                  @ make it fault: it restarts
    mov     r1, #0
    mov     r2, #0
    mov     r7, #1
    swi     #0
    str     r0, [r5, #24]
    mov     r0, r6                  @ the old handle: stale now
    mov     r1, #3
    mov     r2, #4
    mov     r7, #1
    swi     #0
    str     r0, [r5, #28]
    mov     r0, #1                  @ ask the manager again
    mov     r1, #1
    mov     r7, #1
    swi     #0
    mov     r1, #3                  @ and call the new instance
    mov     r2, #4
    mov     r3, #0
    mov     r7, #1
    swi     #0
    str     r0, [r5, #32]
    mov     r0, #1                  @ the manager: start a worker
    mov     r1, #2
    mov     r7, #1
    swi     #0
    str     r0, [r5, #36]
    mov     r0, #1                  @ and another, once that one ended
    mov     r1, #2
    mov     r7, #1
    swi     #0
    str     r0, [r5, #40]
8:  ldr     r6, [r5]                @ our word, untouched
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
    mov     r3, r6                  @ our word
    ldr     r6, 9f                  @ &.data: an absolute address
    ldr     r6, [r6]                @ .data's pointer to .rdata
    ldr     r6, [r6]                @ MAGIC, if both were relocated
    mov     r4, r9
    mov     r5, r12
    mov     r0, r8
    mov     r1, r10
    mov     r2, r11
    mov     r7, #3                  @ SYS_DONE
    swi     #0
2:  b       2b
    .global rp_ipc_reloc
rp_ipc_reloc:
9:  .word   0x00013000              @ PREFERRED_BASE + .data's RVA (relocated)

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

rp_ipc_manager:                     @ r0 = the caller's slot, r1 = operation
    cmp     r1, #1                  @ OPEN: our handle 0 (the host)
    moveq   r2, #0
    beq     4f
    cmp     r1, #3                  @ OPEN_DISPLAY: our handle 2 (1 is
    moveq   r2, #2                  @ our own port)
    beq     4f
    cmp     r1, #2                  @ SPAWN
    mvnne   r0, #2                  @ else an error (0xFFFFFFFD)
    bne     5f
    ldr     r0, =(rp_ipc_worker - rp_user_page)
    mov     r7, #6                  @ SYS_SPAWN: its slot comes back
    swi     #0
    b       5f
4:  mov     r1, r0                  @ to the caller
    mov     r0, r2                  @ the handle asked for
    mov     r2, #1                  @ CALL only
    mov     r7, #4                  @ SYS_GRANT
    swi     #0
5:  mov     r7, #2                  @ SYS_RETURN
    swi     #0
6:  b       6b

rp_ipc_worker:                      @ r0 = a handle to the manager
    ldr     r5, =0x00020000
    ldr     r6, [r5]                @ a new process's memory reads zero
    ldr     r1, =0x33333333
    str     r1, [r5]                @ a mark the next process mustn't see
    mov     r0, #5                  @ a handle we were never given
    mov     r7, #1
    swi     #0
    cmn     r0, #256                @ carry set: refused, as it must be
    ldr     r0, =0x600D
    movcc   r0, #1
    cmp     r6, #0
    movne   r0, #2
    mov     r7, #5                  @ SYS_EXIT
    swi     #0
7:  b       7b
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
    static rp_ipc_manager: u8;
    static rp_ipc_reloc: u8;
}

/// The programs' code, as words.
pub struct Programs {
    /// The code (position independent, but for one relocated literal).
    pub code: &'static [u32],
    /// The client's offset in it.
    pub client: u32,
    /// The server's offset in it (also the supervised host's).
    pub server: u32,
    /// The service manager's offset in it.
    pub manager: u32,
    /// The offset of the literal that needs a base relocation (`HIGHLOW`).
    pub reloc: u32,
}

/// The programs' code and the offsets the image builder needs.
#[must_use]
pub fn ipc_programs() -> Programs {
    let start = core::ptr::addr_of!(rp_user_page);
    let (end, client, server, manager, reloc) = (
        core::ptr::addr_of!(rp_user_page_end) as usize,
        core::ptr::addr_of!(rp_ipc_client) as u32,
        core::ptr::addr_of!(rp_ipc_server) as u32,
        core::ptr::addr_of!(rp_ipc_manager) as u32,
        core::ptr::addr_of!(rp_ipc_reloc) as u32,
    );
    let words = (end - start as usize) / 4;
    // SAFETY: `rp_user_page..rp_user_page_end` is the assembled block
    // above: word aligned, a whole number of words, read-only, in the
    // image for its whole life.
    let code = unsafe { core::slice::from_raw_parts(start, words) };
    let at = start as u32;
    Programs {
        code,
        client: client - at,
        server: server - at,
        manager: manager - at,
        reloc: reloc - at,
    }
}
