/*
 * RetroPane bare-metal image layout (shared by boot/hello and boot/bench).
 *
 * Loaded at 0xA8008000, the address HaRET (the SD launcher) and QEMU both use:
 * 32 KiB into SDRAM, which on the X51v starts at 0xA8000000 (SDCS2, see
 * docs/hw/memory-map.md). HaRET leaves its boot tags at 0xA8000100. Reserved below and
 * above (rp-hal-pxa27x::mmu): the bring-up page table at 0xA8004000 (16 KiB)
 * and the exception vectors at 0xA80F0000; images must stay below 0xA80F0000.
 * 0xA80F8000-0xA8100000 (32 KiB) is reserved for the whole-D-cache clean
 * (rp-hal-pxa27x::cpu::CLEAN_REGION) and never holds data.
 * The bring-up MMU maps everything 1:1, so virtual = physical.
 */
ENTRY(_start)

MEMORY
{
    SDRAM (rwx) : ORIGIN = 0xA8008000, LENGTH = 64M - 32K
}

SECTIONS
{
    .text : {
        KEEP(*(.text.start))
        *(.text .text.*)
    } > SDRAM

    .rodata : ALIGN(4) { *(.rodata .rodata.*) } > SDRAM
    .data   : ALIGN(4) { *(.data .data.*) } > SDRAM

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
