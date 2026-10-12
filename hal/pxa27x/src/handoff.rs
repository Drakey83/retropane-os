//! Handing the CPU from one image to another (ADR-0012 step U3).
//!
//! - [`launch`] starts an image the way HaRET does: interrupts off, data
//!   cache cleaned, MMU and caches off, jump to its first instruction with
//!   Linux boot registers (`r0` = 0, `r1` = machine type, `r2` = boot tags).
//! - `rp_reloc_entry` is the resident loader's first instruction. HaRET puts
//!   every image at `0xA8008000`, but the loader must live elsewhere so the
//!   images it receives can go there. Linked at its home address, it starts
//!   with this position-independent copy of itself and then jumps to the
//!   normal `_start` at home. Only the loader's link script keeps it
//!   (`KEEP(*(.text.reloc))`); every other image discards it. Bytes 4..12
//!   are [`LOADER_SIGNATURE`], so an image can tell the loader is resident.

use crate::cpu;

core::arch::global_asm!(
    r#"
    .section .text.reloc, "ax"
    .global rp_reloc_entry
    .weak __load_end
    .arm
rp_reloc_entry:
    b       0f
    .ascii  "RPLOADER"             @ +4: images check this before returning here
0:  msr     cpsr_c, #0xD3          @ SVC, IRQ and FIQ masked
    adr     r0, rp_reloc_entry     @ where HaRET (or launch) put us
    ldr     r1, =rp_reloc_entry    @ where we're linked
    cmp     r0, r1
    beq     2f                     @ already home (re-entered by launch)
    ldr     r2, =__load_end        @ end of the loaded bytes (.data end)
1:  ldr     r3, [r0], #4
    str     r3, [r1], #4
    cmp     r1, r2
    blo     1b
    @ Copied: a magenta band across the top of the screen (bring-up: with
    @ no serial port, this is the only sign the relocation ran).
    ldr     r0, =0x0C000000
    ldr     r1, =0xF81FF81F
    mov     r2, #0x4000
3:  str     r1, [r0], #4
    subs    r2, r2, #1
    bne     3b
    mov     r3, #0
    mcr     p15, 0, r3, c7, c5, 0  @ invalidate the I-cache and BTB
2:  ldr     pc, =_start
    .ltorg
"#
);

/// At the loader's home + 4 while it is resident.
pub const LOADER_SIGNATURE: [u8; 8] = *b"RPLOADER";

/// Linux's machine number for the Axim X50/X51 family, as HaRET passes it
/// (shown on the device as "mach 00000153").
pub const MACH_AXIM_X50: u32 = 0x153;
/// Where HaRET leaves the boot tags (32 bytes past the 256th byte of RAM).
pub const BOOT_TAGS: u32 = 0xA800_0100;

/// Starts the image whose first instruction is at `entry` (a physical
/// address in identity-mapped SDRAM), as HaRET would: everything this
/// image changed in the CPU's memory system is put back, so the next image
/// sees the same state a cold HaRET boot gives it.
#[cfg_attr(target_arch = "arm", instruction_set(arm::a32))]
pub fn launch(entry: u32) -> ! {
    // Interrupts off first: nothing may run between the clean and the jump.
    // SAFETY: masking interrupts in CPSR has no memory effects.
    unsafe { core::arch::asm!("msr cpsr_c, #0xD3", options(nomem, nostack)) };
    // Dirty lines (the image just written) must reach SDRAM before the
    // caches go off.
    cpu::clean_invalidate_all();
    cpu::drain_write_buffer();
    // SAFETY: this code and `entry` are identity-mapped, so turning the MMU
    // off continues at the same addresses; the D-cache was just cleaned, so
    // invalidating it loses nothing; `entry` is an image's first instruction
    // (the caller's contract) and never returns.
    unsafe { rp_mmu_off_jump(entry, MACH_AXIM_X50, BOOT_TAGS) }
}

core::arch::global_asm!(
    r#"
    .section .text.rp_launch, "ax"
    .arm
    .global rp_mmu_off_jump
    @ rp_mmu_off_jump(entry, mach, tags): caches and MMU off, then jump with
    @ Linux boot registers. The control write, the CPWAIT and what follows
    @ share one aligned 32-byte line (the hazard rt::rp_mmu_on describes).
rp_mmu_off_jump:
    mov     r12, r0
    mov     r3, #0
    mcr     p15, 0, r3, c7, c7, 0      @ invalidate I/D caches and BTB
    mcr     p15, 0, r3, c8, c7, 0      @ invalidate TLBs
    mrc     p15, 0, r3, c1, c0, 0
    bic     r3, r3, #0x1000            @ I-cache off
    bic     r3, r3, #0x0005            @ MMU and D-cache off
    .balignl 32, 0xe1a00000            @ pad with `mov r0, r0`
    mcr     p15, 0, r3, c1, c0, 0
    mrc     p15, 0, r3, c2, c0, 0      @ CPWAIT
    mov     r3, r3
    sub     pc, pc, #4
    mov     r0, #0
    bx      r12
    "#
);

unsafe extern "C" {
    fn rp_mmu_off_jump(entry: u32, mach: u32, tags: u32) -> !;
}
