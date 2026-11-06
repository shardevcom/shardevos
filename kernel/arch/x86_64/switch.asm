; switch_context.asm — Cambio de contexto de tareas para shardevos x86_64
; TaskContext offsets (desde la definición en Rust):
;   rip=0x00, rsp=0x08, rflags=0x10, rax=0x18, rbx=0x20, rcx=0x28, rdx=0x30
;   rsi=0x38, rdi=0x40, rbp=0x48, r8=0x50, r9=0x58, r10=0x60, r11=0x68
;   r12=0x70, r13=0x78, r14=0x80, r15=0x88, cs=0x90, ss=0x98, ds=0xA0
;   es=0xA8, fs=0xB0, gs=0xB8, cr3=0xC0

[bits 64]
section .text

; ============================================================================
; switch_context(from_ctx_ptr, to_ctx_ptr)
;   rdi = puntero a TaskContext de origen (guardar aquí)
;   rsi = puntero a TaskContext de destino (cargar desde aquí)
; Guarda registros en from_ctx, carga de to_ctx, y retorna.
; ============================================================================
global switch_context
switch_context:
    push rbp
    push rbx
    push r12
    push r13
    push r14
    push r15

    ; Guardar rsp del hilo actual
    mov [rdi + 0x08], rsp

    ; Guardar registros de propósito general (los que preservamos en pila)
    mov [rdi + 0x20], rbx
    mov [rdi + 0x50], r12
    mov [rdi + 0x58], r13
    mov [rdi + 0x60], r14
    mov [rdi + 0x68], r15

    ; Guardar rflags
    pushfq
    pop rax
    mov [rdi + 0x10], rax

    ; Guardar segmentos y CR3
    mov ax, cs
    mov [rdi + 0x90], rax
    mov ax, ss
    mov [rdi + 0x98], rax
    mov ax, ds
    mov [rdi + 0xA0], rax
    mov ax, es
    mov [rdi + 0xA8], rax
    mov ax, fs
    mov [rdi + 0xB0], rax
    mov ax, gs
    mov [rdi + 0xB8], rax
    mov rax, cr3
    mov [rdi + 0xC0], rax

    ; --- Cargar contexto de destino ---
    mov rax, [rsi + 0xC0]   ; cr3
    mov cr3, rax

    ; Cargar segmentos
    mov rax, [rsi + 0x90]   ; cs (no se puede cargar directamente, se hace con ret lejano o iret)
    ; cs y ss se cargan al retornar con iretq o far ret
    ; Para simplicidad, no cambiamos cs/ss en switch_context (mismo nivel de privilegio)
    ; Los segmentos ds, es, fs, gs sí se pueden cambiar:
    mov rax, [rsi + 0xA0]   ; ds
    mov cx, ax
    mov ds, cx
    mov rax, [rsi + 0xA8]   ; es
    mov cx, ax
    mov es, cx
    mov rax, [rsi + 0xB0]   ; fs
    mov cx, ax
    mov fs, cx
    mov rax, [rsi + 0xB8]   ; gs
    mov cx, ax
    mov gs, cx

    ; Cargar registros de propósito general
    mov rbx, [rsi + 0x20]
    mov r12, [rsi + 0x50]
    mov r13, [rsi + 0x58]
    mov r14, [rsi + 0x60]
    mov r15, [rsi + 0x68]

    ; Cargar rflags
    mov rax, [rsi + 0x10]
    push rax
    popfq

    ; Cargar rsp
    mov rsp, [rsi + 0x08]

    ; Restaurar registros preservados
    pop r15
    pop r14
    pop r13
    pop r12
    pop rbx
    pop rbp

    ; Retornar: el siguiente hilo continúa desde aquí
    ; rip se restauró al crear el hilo (thread_entry_stub o el rip guardado)
    ret

; ============================================================================
; timer_reschedule(current_frame_ptr, error_code)
;   rdi = puntero al InterruptFrame actual (para guardar)
;   rsi = error_code (no usado)
; ============================================================================
global timer_reschedule
timer_reschedule:
    push rax
    push rbx
    push rcx
    push rdx
    push rsi
    push rdi
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15

    ; rdi = current_frame_ptr (InterruptFrame en el stack del ISR)
    ; rsi = next TaskContext* (o null para mantener el actual)
    ; El frame ya fue guardado por Rust antes de llamar a esta función.
    ; Solo cargamos el siguiente contexto y hacemos iretq.

    test rsi, rsi
    jz .use_current

    ; Cargar SS del siguiente hilo para iretq
    mov rax, [rsi + 0x98]   ; ss offset = 0x98
    push rax

    ; Cargar RSP del siguiente hilo
    mov rax, [rsi + 0x08]   ; rsp offset = 0x08
    push rax

    ; Cargar RFLAGS del siguiente hilo
    mov rax, [rsi + 0x10]   ; rflags offset = 0x10
    push rax

    ; Cargar CS del siguiente hilo
    mov rax, [rsi + 0x90]   ; cs offset = 0x90
    push rax

    ; Cargar RIP del siguiente hilo
    mov rax, [rsi + 0x00]   ; rip offset = 0x00
    push rax

    jmp .do_iret

.use_current:
    ; No hay siguiente tarea, restaurar el frame actual (en rdi)
    mov rax, [rdi + 0x20]   ; ss (offset 32 en InterruptFrame)
    push rax
    mov rax, [rdi + 0x18]   ; rsp (offset 24)
    push rax
    mov rax, [rdi + 0x10]   ; rflags (offset 16)
    push rax
    mov rax, [rdi + 0x08]   ; cs (offset 8)
    push rax
    mov rax, [rdi + 0x00]   ; rip (offset 0)
    push rax

.do_iret:
    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rdi
    pop rsi
    pop rdx
    pop rcx
    pop rbx
    pop rax

    iretq

    hlt
    jmp $

; ============================================================================
; switch_to_user(entry_point, user_rsp, user_cr3)
;   rdi = entry point (rip en modo usuario)
;   rsi = user stack pointer (rsp)
;   rdx = user CR3 (espacio de direcciones de usuario)
; ============================================================================
global switch_to_user
switch_to_user:
    mov cr3, rdx

    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax
    mov ss, ax

    push 0x2B           ; SS de usuario (ring 3)
    push rsi            ; RSP de usuario
    pushfq
    pop rax
    or rax, 0x200       ; Habilitar IF
    push rax            ; RFLAGS
    push 0x23           ; CS de usuario (ring 3)
    push rdi            ; RIP (entry point)

    iretq

    hlt
    jmp $
