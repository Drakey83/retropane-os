/*
 * RetroPane bare-metal image layout (shared by boot/hello and boot/bench).
 *
 * Loaded at 0xA0008000, the address the SD launcher and QEMU both use (the
 * traditional ARM kernel load address, 32 KiB into SDRAM). Reserved below and
 * above (rp-hal-pxa27x::mmu): the bring-up page table at 0xA0004000 (16 KiB)
 * and the exception vectors at 0xA00F0000; images must stay below 0xA00F0000.
 * The bring-up MMU maps everything 1:1, so virtual = physical.
 */
ENTRY(_start)

MEMORY
{
    SDRAM (rwx) : ORIGIN = 0xA0008000, LENGTH = 64M - 32K
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
    } > SDRAM

    /DISCARD/ : { *(.ARM.exidx .ARM.exidx.* .ARM.extab.*) }
}
