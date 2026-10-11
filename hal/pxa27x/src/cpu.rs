//! XScale MMU, cache and TLB control for the kernel's VM (ADR-0008), and
//! recoverable aborts.
//!
//! Cache and TLB operations are the XScale Core Developer's Manual set: the
//! XScale has no "clean and invalidate D line" or "clean entire D-cache"
//! operation, so lines are cleaned then invalidated, and the whole D-cache is
//! cleaned with the manual's method: allocating every line of a reserved,
//! cacheable 32 KiB region (evicting, and so writing back, everything else),
//! then invalidating. Addresses are modified virtual addresses (FCSE).

/// D-cache line size.
pub const LINE: u32 = 32;
/// D-cache size (32-way, 32 sets of 32-byte lines).
pub const DCACHE_SIZE: u32 = 32 * 1024;
/// The reserved region the whole-cache clean allocates lines in: the last
/// 32 KiB of SDRAM's first MiB, kept free by `boot/link.x` (images end below
/// the vector page at 0xA80F0000).
pub const CLEAN_REGION: u32 = 0xA80F_8000;

/// CPWAIT: makes a CP15 change take effect before the next instruction.
#[inline(always)]
fn cpwait() {
    // SAFETY: reads a CP15 register and waits on it; no state changes.
    unsafe {
        core::arch::asm!(
            "mrc p15, 0, {t}, c2, c0, 0",
            "mov {t}, {t}",
            "sub pc, pc, #4",
            t = out(reg) _,
            options(nostack)
        );
    }
}

/// Writes back and discards the D-cache lines, and discards the I-cache
/// lines, of `mva..mva + len`.
pub fn clean_invalidate_range(mva: u32, len: u32) {
    let mut line = mva & !(LINE - 1);
    let end = mva.saturating_add(len);
    while line < end {
        // SAFETY: cache maintenance by address has no effect on program
        // state other than making memory (not cache) hold the data.
        unsafe {
            core::arch::asm!(
                "mcr p15, 0, {a}, c7, c10, 1", // clean D line
                "mcr p15, 0, {a}, c7, c6, 1",  // invalidate D line
                "mcr p15, 0, {a}, c7, c5, 1",  // invalidate I line
                a = in(reg) line,
                options(nostack)
            );
        }
        line += LINE;
    }
    drain_write_buffer();
}

/// Writes back (without discarding) the D-cache lines of `va..va + len`:
/// for memory the kernel wrote through one address that the MMU or another
/// address will read.
pub fn clean_range(va: u32, len: u32) {
    let mut line = va & !(LINE - 1);
    let end = va.saturating_add(len);
    while line < end {
        // SAFETY: as in `clean_invalidate_range`.
        unsafe { core::arch::asm!("mcr p15, 0, {}, c7, c10, 1", in(reg) line, options(nostack)) };
        line += LINE;
    }
    drain_write_buffer();
}

/// Writes back and discards the whole D-cache, and discards the I-cache and
/// branch target buffer.
pub fn clean_invalidate_all() {
    let mut line = CLEAN_REGION;
    while line < CLEAN_REGION + DCACHE_SIZE {
        // SAFETY: line allocate claims a cache line for the reserved region
        // without reading memory, evicting (writing back) whatever it held.
        // The region is never used for data, so its own lines are junk and
        // are discarded by the invalidate below.
        unsafe { core::arch::asm!("mcr p15, 0, {}, c7, c2, 5", in(reg) line, options(nostack)) };
        line += LINE;
    }
    // SAFETY: everything dirty has been written back; invalidating loses
    // nothing.
    unsafe {
        core::arch::asm!(
            "mov {t}, #0",
            "mcr p15, 0, {t}, c7, c10, 4", // drain write buffer
            "mcr p15, 0, {t}, c7, c6, 0",  // invalidate D-cache
            "mcr p15, 0, {t}, c7, c5, 0",  // invalidate I-cache and BTB
            t = out(reg) _,
            options(nostack)
        );
    }
    cpwait();
}

/// Waits for buffered writes to reach memory.
pub fn drain_write_buffer() {
    // SAFETY: draining the write buffer only waits.
    unsafe { core::arch::asm!("mcr p15, 0, {}, c7, c10, 4", in(reg) 0u32, options(nostack)) };
}

/// Discards the instruction and data TLB entries for `mva`.
pub fn invalidate_tlb_entry(mva: u32) {
    // SAFETY: TLB entries are refetched from the page tables on demand.
    unsafe {
        core::arch::asm!(
            "mcr p15, 0, {a}, c8, c5, 1", // I TLB entry
            "mcr p15, 0, {a}, c8, c6, 1", // D TLB entry
            a = in(reg) mva,
            options(nostack)
        );
    }
    cpwait();
}

/// Discards every TLB entry.
pub fn invalidate_tlb_all() {
    // SAFETY: as in `invalidate_tlb_entry`.
    unsafe { core::arch::asm!("mcr p15, 0, {}, c8, c7, 0", in(reg) 0u32, options(nostack)) };
    cpwait();
}

/// Makes the MMU use the first-level table at physical `table` (16 KiB
/// aligned). The caller must have written the table back to memory
/// ([`clean_range`]) and it must map the code running now at its current
/// address.
pub fn switch_table(table: u32) {
    assert!(
        table.is_multiple_of(16 * 1024),
        "L1 tables are 16 KiB aligned"
    );
    // SAFETY: per the contract above, the new table maps the running code
    // identically, so execution continues; stale TLB entries are discarded.
    unsafe {
        core::arch::asm!(
            "mcr p15, 0, {z}, c7, c10, 4", // drain write buffer
            "mcr p15, 0, {t}, c2, c0, 0",  // TTB
            "mcr p15, 0, {z}, c8, c7, 0",  // invalidate TLBs
            t = in(reg) table,
            z = in(reg) 0u32,
            options(nostack)
        );
    }
    cpwait();
}

/// Sets the Domain Access Control Register.
pub fn set_dacr(dacr: u32) {
    // SAFETY: changes which domains are accessible; the caller's own code and
    // stack are in domain 0, which RetroPane always keeps accessible.
    unsafe { core::arch::asm!("mcr p15, 0, {}, c3, c0, 0", in(reg) dacr, options(nostack)) };
    cpwait();
}

/// Sets the FCSE process ID (0-127): addresses below 32 MiB now mean slot
/// `pid`. No cache or TLB maintenance is needed: that's the point of FCSE.
///
/// # Panics
/// If `pid` is 128 or more.
pub fn set_pid(pid: u32) {
    assert!(pid < 128, "the FCSE PID is 7 bits");
    // SAFETY: the running kernel lives above 32 MiB, so relocating the bottom
    // slot doesn't move it.
    unsafe { core::arch::asm!("mcr p15, 0, {}, c13, c0, 0", in(reg) pid << 25, options(nostack)) };
    cpwait();
}

/// Reads the Fault Status and Fault Address registers (after a data abort).
#[must_use]
pub fn fault_registers() -> (u32, u32) {
    let (fsr, far): (u32, u32);
    // SAFETY: reading FSR and FAR has no side effects.
    unsafe {
        core::arch::asm!(
            "mrc p15, 0, {s}, c5, c0, 0",
            "mrc p15, 0, {a}, c6, c0, 0",
            s = out(reg) fsr,
            a = out(reg) far,
            options(nostack, nomem)
        );
    }
    (fsr, far)
}

/// Writes `value` to `addr` with a plain store: for kernel self-tests that
/// touch memory through the page tables (it may fault, and the abort handler
/// decides what happens).
pub fn poke(addr: usize, value: u32) {
    // SAFETY: test-only access through the MMU: the store either hits mapped
    // writable memory the caller owns, or aborts into the abort handler.
    unsafe { (addr as *mut u32).write_volatile(value) }
}
