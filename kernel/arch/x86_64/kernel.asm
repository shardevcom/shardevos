; shardevos Kernel - Modo largo x86_64 con pantalla
section .text
global _start

section .rodata
msg1:   db "========================================", 0
msg2:   db "  shardevos Kernel v0.1.0", 0
msg3:   db "  Sistema Operativo Inteligente", 0
msg4:   db "  EN EJECUCION - OK", 0
msg5:   db "========================================", 0

section .text
global _start
_start:
    ; Configurar segmentos de datos
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax
    mov ss, ax

    ; Apuntar stack al tope
    mov rsp, stack_top

    ; Limpiar pantalla (80x25 = 2000 caracteres, cada uno 2 bytes)
    mov rax, 0xB8000
    mov rdi, rax
    mov rcx, 1000           ; 2000 bytes / 2 = 1000 dobles palabras
    mov eax, 0x07200720   ; ' ' con color gris claro sobre negro, dos caracteres
    rep stosd             ; Escribir 4 bytes a la vez (2 caracteres)

    ; Escribir mensajes en pantalla
    lea rsi, [rel msg1]
    call print_string

    call newline

    lea rsi, [rel msg2]
    call print_string

    call newline

    lea rsi, [rel msg3]
    call print_string

    call newline

    mov rsi, msg4
    call print_string

    call newline

    mov rsi, msg5
    call print_string
    call newline

    ; Loop infinito
.hang:
    hlt
    jmp .hang

; Función para imprimir string terminado en 0
print_string:
    push rsi
    push rax
    push rdi
    mov rdi, rax         ; Dirección de pantalla = 0xB8000
    mov rax, 0xB8000
    mov rdi, rax
.print_char:
    mov al, [rsi]        ; Cargar carácter
    test al, al          ; ¿Es cero?
    jz .done             ; Si es cero, terminó
    mov [rdi], al        ; Escribir carácter en pantalla
    mov byte [rdi+1], 0x0F   ; Escribir atributo de color (blanco sobre negro)
    add rdi, 2           ; Avanzar 2 bytes (carácter + atributo)
    inc rsi              ; Avanzar al siguiente carácter
    jmp .print_char
.done:
    pop rax
    pop rsi
    ret

; Nueva línea
newline:
    push rax
    push rdx
    mov rax, rdi
    mov rbx, 160         ; 80 caracteres × 2 bytes = 160 bytes por línea
    mov rdx, rax
    xor rdx, rdx
    div rbx              ; rax / 160 = fila actual, rdx = resto
    mul rbx              ; rax = fila actual * 160
    add rax, 160         ; Avanzar una línea completa
    mov rdi, rax
    pop rdx
    pop rax
    ret

section .bss
align 16
stack_bottom:
    resb 65536
stack_top: