; shardevos — syscall entry stub x86_64
; Guarda registros, llama a syscall_handler y retorna con sysret.
; Convención x86_64 System V:
;   rax = syscall number
;   rdi, rsi, rdx, r10, r8, r9 = argumentos
;   rcx = RIP de retorno, r11 = RFLAGS

section .text
global syscall_entry
global syscall_setup_msr
extern syscall_handler

section .bss
align 16
global syscall_stack_top
syscall_stack_top:
    resq 1

section .text

; Configura los MSRs para la instruccion `syscall`/`sysret` (x86_64):
;   IA32_LSTAR (0xC0000082) = direccion del stub de syscall
;   IA32_LSS  (0xC0000083) = stack de syscall
;   IA32_EFLAGS (0xC0000084) = mascara de flags para sysret
syscall_setup_msr:
    ; IA32_STAR (0xC0000081): CS/SS base para SYSCALL = 0x08 (bits 47:32 en EDX)
    mov eax, 0x00000000
    mov edx, 0x00000008
    mov ecx, 0xC0000081
    wrmsr
    ; IA32_LSTAR (0xC0000082) = direccion del stub de entrada
    mov eax, syscall_entry
    mov edx, 0x0
    mov ecx, 0xC0000082
    wrmsr
    ; IA32_LSS (0xC0000083) = stack de syscall (ring0->ring0 no lo usa)
    mov eax, syscall_stack_top
    mov edx, 0x0
    mov ecx, 0xC0000083
    wrmsr
    ; IA32_FMASK (0xC0000084): limpiar IF durante el handler (0x200)
    mov eax, 0x200
    mov edx, 0x0
    mov ecx, 0xC0000084
    wrmsr
    ret

syscall_entry:
    ; Entrada via SYSCALL (64 bits):
    ;   rax = nr ; rdi,rsi,rdx,r10,r8,r9 = arg1..arg6
    ;   rcx = RIP de retorno ; r11 = RFLAGS
    push rcx
    push r11
    push rdi
    push rsi
    push rdx
    push r8
    push r9
    push r10

    ; Guardar nr en r11 (ya salvado en pila) y arg6 (r9 original) en rax
    mov r11, rax
    mov rax, [rsp + 8]

    ; Reordenar a ABI SysV: syscall_handler(nr, a1..a6)
    mov rdi, r11            ; nr
    mov rsi, [rsp + 40]     ; arg1 (rdi original)
    mov rdx, [rsp + 32]     ; arg2 (rsi original)
    mov rcx, [rsp + 24]     ; arg3 (rdx original)
    mov r8,  [rsp + 0]      ; arg4 (r10 original)
    mov r9,  [rsp + 16]     ; arg5 (r8 original)

    push rax                ; arg6 (7o parametro: va en la pila)
    call syscall_handler
    add rsp, 8

    ; rax = valor de retorno; restaurar registros salvados
    pop r10
    pop r9
    pop r8
    pop rdx
    pop rsi
    pop rdi
    pop r11
    pop rcx

    ; Volver al "usuario" (hoy ring 0): restaurar RFLAGS y saltar al RIP guardado.
    ; (Se usa jmp en vez de sysret porque el GDT actual no tiene descriptores DPL3.)
    push r11
    popfq
    jmp rcx
