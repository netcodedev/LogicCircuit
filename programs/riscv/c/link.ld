/* C programs on SOC_RV32: everything in RAM from address 0. */
ENTRY(_start)
SECTIONS {
    . = 0;
    .text : { *(.text.start) *(.text .text.*) }
    .rodata : { *(.rodata .rodata.* .srodata .srodata.*) }
    .data : { *(.data .data.* .sdata .sdata.*) }
    . = ALIGN(4);
    __bss_start = .;
    .bss : { *(.bss .bss.* .sbss .sbss.* COMMON) }
    . = ALIGN(4);
    __bss_end = .;
}
