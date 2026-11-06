; shardev OS - Programa de usuario de prueba (modo largo 64 bits)
; Imprime un mensaje en VGA texto y luego hace syscall de salida

[BITS 64]

section .text
global _start

_start:
    ; Escribir 'U' en VGA texto (0xB8000) - modo largo usa direcciones directas
    mov rax, 0xB8000
    mov byte [rax], 'U'
    mov byte [rax + 1], 0x0F  ; blanco sobre negro

    ; Escribir mensaje completo en VGA
    mov rsi, msg
    mov rdi, 0xB8000 + 2      ; segunda posición
    mov ecx, msg_len
.write_loop:
    mov al, [rsi]
    test al, al
    jz .done_write
    mov [rdi], al
    mov byte [rdi + 1], 0x0F
    add rdi, 2
    inc rsi
    dec ecx
    jnz .write_loop
.done_write:

    ; Syscall: exit (nr=1)
    mov rax, 1          ; SYSCALL_EXIT
    mov rdi, 0          ; código de salida 0
    syscall

    ; Si volvemos, hlt
    hlt
    jmp $

section .rodata
msg:   db "Hello from userland! Running via binfmt_loader.", 0
msg_len equ $ - msg
