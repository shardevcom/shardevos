; shardevos — ISR stubs x86_64
; Genera wrappers de ensamblador para las 256 entradas de la IDT.
; Cada stub guarda registros, empuja el número de interrupción y salta
; al manejador común en Rust (`isr_common_handler`).

section .text

global isr_stub_start
global isr_stub_end
extern isr_common_handler

; Macro para generar un stub de ISR
%macro ISR_STUB 2
global isr_stub_%1
isr_stub_%1:
    cli
    push 0                ; espacio para error code (0 si no aplica)
    push %1               ; número de interrupción
    jmp isr_common_handler
%endmacro

isr_stub_start:

; Excepciones 0-31 (todas con error code implícito o sin él)
%assign i 0
%rep 32
    ISR_STUB i, 0
%assign i i+1
%endrep

; Interrupciones de hardware 32-255
%assign i 32
%rep 224
    ISR_STUB i, 0
%assign i i+1
%endrep

isr_stub_end:
