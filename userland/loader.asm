; Cargador de arranque simple para floppy
[BITS 16]
[ORG 0x7C00]

start:
    cli
    cld
    xor ax, ax
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov sp, 0x7C00

    ; Imprimir 'S' en serial COM1
    mov bl, 'S'
    mov dx, 0x3F8
.wait_serial:
    in al, dx
    test al, 0x20
    jz .wait_serial
    mov al, bl
    out dx, al

    ; Imprimir 'A' en VGA
    mov ax, 0xB800
    mov es, ax
    mov byte [es:0x00], 'A'
    mov byte [es:0x01], 0x0F

    hlt
    jmp $

times 510 - ($ - $$) db 0
dw 0xAA55
