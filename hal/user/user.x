/*
 * RetroPane user process image layout (ADR-0009, ADR-0016): linked at the
 * PE base the loader prefers, one section per page-aligned group so each
 * page gets its section's rights (W^X), and turned into a PE file by
 * `cargo xtask` (relocations from --emit-relocs, so the kernel can load it
 * at any base).
 *
 *   0x10000           headers (the PE file's own; nothing here)
 *   0x11000  .text    code (read + execute)
 *   next page .rdata  constants (read)
 *   next page .data   data, then .bss (read + write; .bss reads as zero)
 */
ENTRY(_start)

SECTIONS
{
    . = 0x00011000;
    .text : {
        KEEP(*(.text._start))
        *(.text .text.*)
    }

    . = ALIGN(4096);
    .rdata : { *(.rodata .rodata.*) }

    . = ALIGN(4096);
    .data : {
        *(.data .data.*)
        . = ALIGN(4);
        __bss_start = .;
        *(.bss .bss.* COMMON)
        __bss_end = .;
    }

    /DISCARD/ : { *(.ARM.exidx*) *(.ARM.extab*) *(.comment) }
}
