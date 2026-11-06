ENTRY(start)
SECTIONS
{
    . = 1M;

    .multiboot2 ALIGN(1K) : {
        KEEP(*(.multiboot2))
    }

    .text ALIGN(1K) : {
        *(.text)
    }

    .rodata ALIGN(1K) : {
        *(.rodata)
    }

    .data ALIGN(1K) : {
        *(.data)
    }

    .bss ALIGN(1K) : {
        *(.bss)
    }

    /DISCARD/ : {
        *(.eh_frame)
        *(.eh_frame_hdr)
    }
}
