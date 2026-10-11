/*
 * RetroPane bring-up image layout.
 *
 * Loaded at 0xA0008000, the address the SD launcher and QEMU both use (the
 * traditional ARM kernel load address, 32 KiB into SDRAM). MMU off, so
 * virtual = physical.
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
        . += 64K;
        __stack_top = .;
    } > SDRAM

    /DISCARD/ : { *(.ARM.exidx .ARM.exidx.* .ARM.extab.*) }
}
