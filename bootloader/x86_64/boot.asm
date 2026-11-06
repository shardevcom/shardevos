; Shardev OS bootloader x86_64 — Multiboot2
; GRUB carga el kernel en 1 MB y salta al offset 0x0 (start), en modo de 32 bits.

[BITS 32]
section .multiboot2 align=4

; GRUB (Multiboot2 raw) salta al offset 0 de la imagen cargada a 1 MB.
; El salto near salta sobre la cabecera hacia `start`.
    jmp start
    times 3 db 0                   ; rellenar hasta alinear a 4 bytes

; ============================================================================
; Cabecera Multiboot2 — GRUB la busca al inicio de la imagen (alig. 4 bytes)
; ============================================================================
multiboot2_header:
    dd 0xe85250d6                 ; magic
    dd 0                          ; architecture (i386)
    dd multiboot2_header_end - multiboot2_header  ; header length
    dd 0x100000000 - (0xe85250d6 + 0 + (multiboot2_header_end - multiboot2_header)) ; checksum

    ; tag: required end
    dw 0                          ; type
    dw 0                          ; flags
    dd 8                          ; size
multiboot2_header_end:

section .text

; ============================================================================
; Punto de entrada — GRUB salta aquí (Modo Multiboot2: segmentos planos,
; paginación desactivada, CPU en modo protegido de 32 bits).
; ============================================================================
global start
extern kernel_entry               ; definido en kernel/arch/x86_64/entry.rs

start:
    cli
    cld

    ; Paginación desactivada (GRUB en Multiboot2 puede dejarla activa),
    ; para acceder a memoria con direcciones físicas (carga en 1 MB).
    mov eax, cr0
    and eax, ~(1 << 31)       ; PG = 0
    mov cr0, eax

    ; Selector de datos plano (base 0, límite 4 GB) para leer el mensaje y la GDT.
    mov ax, 0x10
    mov ds, ax

    ; Mensaje de arranque por puerto serie COM1.
    lea esi, [boot_msg_start]
    call serial_print_32

    ; Mensaje de arranque en pantalla VGA (texto modo 3, 80x25).
    lea esi, [boot_msg_vga]
    call vga_print_32

    ; Cargar nuestra GDT mínima para modo largo.
    lgdt [gdt_descriptor]

    ; Segmentos de datos y pila = nuestra GDT (0x10, writable, base 0, límite 4 GB).
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax
    mov ss, ax
    mov esp, stack_top

    ; Habilitar PAE.
    mov eax, cr4
    or eax, 1 << 5
    mov cr4, eax

    ; Páginas 4KB, identity map simple (primeros 2 MB).
    mov eax, page_table
    mov cr3, eax

    ; Habilitar modo largo (EFER.LME).
    mov ecx, 0xC0000080
    rdmsr
    or ecx, 1 << 8
    wrmsr

    ; Habilitar paginación y modo protegido.
    mov eax, cr0
    or eax, 1 << 31               ; PG
    or eax, 1 << 0                ; PE
    mov cr0, eax

    ; Saltar a modo largo (64 bits) — segmento de código 0x08 de nuestra GDT.
    ; PRUEBA: cuelgue tras activar paginación (sin modo largo) para aislar.
    jmp .hang32

.hang32:
    hlt
    jmp .hang32

.hang:
    hlt
    jmp .hang

; ============================================================================
; Funciones de impresión (modo 32 bits)
; ============================================================================

; Imprime una cadena terminada en nulo por puerto serie COM1 (0x3F8).
serial_print_32:
    push eax
    push esi
.serial_loop:
    mov al, [esi]
    test al, al
    jz .serial_done
    out 0x3F8, al
    inc esi
    jmp .serial_loop
.serial_done:
    pop esi
    pop eax
    ret

; Inicializa UART COM1 (0x3F8) a 115200 baud, 8N1, sin interrupciones.
uart_init_32:
    push dx
    push ax
    ; DLAB = 1 para acceder al divisor de baudios
    mov dx, 0x3FB
    mov al, 0x80
    out dx, al
    ; Divisor = 1 -> 115200 baud (reloj base 115200 Hz)
    mov dx, 0x3F8
    mov al, 0x01
    out dx, al
    mov dx, 0x3F9
    mov al, 0x00
    out dx, al
    ; DLAB = 0, 8 bits, sin paridad, 1 bit de parada
    mov dx, 0x3FB
    mov al, 0x03
    out dx, al
    ; Habilitar FIFO y limpiarlo
    mov dx, 0x3FC
    mov al, 0x07
    out dx, al
    pop ax
    pop dx
    ret

; Imprime una cadena terminada en nulo en pantalla VGA (memoria 0xB8000, modo texto 3, 80x25).
vga_print_32:
    push eax
    push esi
    push edi
    mov edi, 0xB8000          ; inicio de memoria de video texto
.vga_loop:
    mov al, [esi]
    test al, al
    jz .vga_done
    mov [edi], al             ; carácter
    mov byte [edi + 1], 0x0F  ; atributo: blanco sobre negro
    add edi, 2
    inc esi
    jmp .vga_loop
.vga_done:
    pop edi
    pop esi
    pop eax
    ret

; Mensajes de arranque
boot_msg_start: db "Shardev OS: arrancando start (modo 32 bits)", 13, 10, 0
boot_msg_vga:   db "Shardev OS v0.1.0 - Arrancando...", 0

; ============================================================================
; Modo largo (64 bits)
; ============================================================================
[BITS 64]
long_mode:
    ; Configurar segmentos de datos (en modo largo solo CS importa)
    xor ax, ax
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax
    mov ss, ax

    ; Apuntar stack al tope
    mov rsp, stack_top

    ; Llamar al kernel en Rust (kernel_entry)
    call kernel_entry

    ; Si retorna, loop infinito
.hang:
    hlt
    jmp .hang

; ============================================================================
; Datos (BSS y tablas)
; ============================================================================
section .bss
align 16
stack_bottom:
    resb 65536
stack_top:

section .data
; Tabla GDT (null, code de 64 bits, data)
gdt_start:
    dq 0                          ; null descriptor
gdt_code:
    dw 0xFFFF                     ; limit low
    dw 0                          ; base low
    db 0                          ; base middle
    db 0x9A                       ; access (present, ring 0, code, executable)
    db 0xC0                       ; granularity: G=1 (4KB) + L=1 (modo 64 bits)
    db 0                          ; base high
gdt_data:
    dw 0xFFFF
    dw 0
    db 0
    db 0x92
    db 0xCF
    db 0
gdt_end:

gdt_descriptor:
    dw gdt_end - gdt_start - 1
    dq gdt_start

align 4096
page_table:
    dq 0x00001007                 ; PML4[0] -> PDPT (present, rw)
    times 511 dq 0

align 4096
pdpt:
    dq 0x00002007                 ; PDPT[0] -> PD
    times 511 dq 0

align 4096
pd:
    dq 0x00003007                 ; PD[0] -> PT
    times 511 dq 0

align 4096
pt:
    times 512 dq 0x00000003       ; PT[0..511] -> identidad 2MB (present, rw)
