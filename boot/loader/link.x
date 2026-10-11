/*
 * The resident loader's layout (ADR-0012 U3): like boot/link.x, but linked at
 * its own home, 0xA8400000 (rp_hal_x51v::devices::LOADER_BASE, 1 MiB
 * reserved), so the images it receives can go to 0xA8008000. HaRET still
 * loads it at 0xA8008000: its first instructions (rp_reloc_entry in
 * rp-hal-pxa27x::handoff, position-independent) copy it home and jump to
 * _start there. __load_end marks the end of the bytes to copy.
 */
ENTRY(rp_reloc_entry)
MEMORY
{
    SDRAM (rwx) : ORIGIN = 0xA8400000, LENGTH = 1M
}

SECTIONS
{
    .text : {
        KEEP(*(.text.reloc))
        KEEP(*(.text.start))
        *(.text .text.*)
    } > SDRAM

    .rodata : ALIGN(4) { *(.rodata .rodata.*) } > SDRAM
    /* (faulting instruction, fixup) pairs for rt::try_read / try_write. */
    .rp_fixup : ALIGN(4) {
        __rp_fixup_start = .;
        KEEP(*(.rp_fixup))
        __rp_fixup_end = .;
    } > SDRAM
    .data   : ALIGN(4) { *(.data .data.*) . = ALIGN(4); } > SDRAM
    __load_end = .;

    .bss (NOLOAD) : ALIGN(4) {
        __bss_start = .;
        *(.bss .bss.* COMMON)
        . = ALIGN(4);
        __bss_end = .;
    } > SDRAM

    .stack (NOLOAD) : ALIGN(8) {
        . += 16K;
        __stack_top = .;
        . += 1536;
        __irq_stack_top = .;
        . += 512;
        __exc_stack_top = .;
        /* Abort mode: its own stack when the image needs one (the kernel
         * resolves page faults in abort mode; its build.rs passes
         * --defsym=__abt_stack_size=...). By default 0: shared with the
         * undefined-instruction stack, which only reports and stops. */
        . += DEFINED(__abt_stack_size) ? __abt_stack_size : 0;
        __abt_stack_top = .;
    } > SDRAM

    /DISCARD/ : { *(.ARM.exidx .ARM.exidx.* .ARM.extab.*) }
}
