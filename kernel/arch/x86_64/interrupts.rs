use core::arch::asm;

use super::{gdt, idt};

/// Registro de estado de CPU empujado por el hardware en una interrupción.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct InterruptFrame {
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

/// Registro de error opcional (algunas excepciones lo empujan).
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct InterruptInfo {
    pub error_code: u64,
    pub interrupt_number: u64,
}

/// Manejador común llamado desde los stubs de ensamblador.
/// El stack en este punto tiene (de abajo a arriba):
///   [error_code] [interrupt_number] [regs...] [InterruptFrame]
#[no_mangle]
pub extern "C" fn isr_common_handler(error_code: u64, interrupt_number: u64) {
    unsafe {
        // Leer el frame de interrupción desde el stack
        // (después de que el asm haya empujado error_code y número)
        let frame_ptr: *const InterruptFrame;
        asm!("mov {}, rsp", out(reg) frame_ptr, options(nostack));
        let frame = &*frame_ptr;

        // Dispatch basado en el número de interrupción
        match interrupt_number {
            0 => handle_divide_by_zero(frame, error_code),
            1 => handle_debug(frame, error_code),
            2 => handle_nmi(frame, error_code),
            3 => handle_breakpoint(frame, error_code),
            4 => handle_overflow(frame, error_code),
            5 => handle_bad_range(frame, error_code),
            6 => handle_invalid_opcode(frame, error_code),
            7 => handle_device_not_available(frame, error_code),
            8 => handle_double_fault(frame, error_code),
            9 => handle_coprocessor_segment_overrun(frame, error_code),
            10 => handle_invalid_tss(frame, error_code),
            11 => handle_segment_not_present(frame, error_code),
            12 => handle_stack_segment_fault(frame, error_code),
            13 => handle_general_protection_fault(frame, error_code),
            14 => handle_page_fault(frame, error_code),
            15 => handle_reserved(frame, error_code),
            16 => handle_x87_floating_point(frame, error_code),
            17 => handle_alignment_check(frame, error_code),
            18 => handle_machine_check(frame, error_code),
            19 => handle_simd_floating_point(frame, error_code),
            20 => handle_virtualization(frame, error_code),
            32 => handle_timer_tick(frame, error_code),
            33 => handle_keyboard(frame, error_code),
            _ => handle_unhandled(interrupt_number, frame, error_code),
        }

        // Enviar EOI al PIC si es una interrupción de hardware (32-47)
        if interrupt_number >= 32 && interrupt_number < 48 {
            pic_eoi((interrupt_number - 32) as u8);
        }
    }

    // Retornar desde la interrupción
    unsafe {
        asm!("iretq", options(nostack, preserves_flags));
    }
}

// ---------------------------------------------------------------------------
// Manejadores individuales (placeholders)
// ---------------------------------------------------------------------------

fn handle_divide_by_zero(_frame: &InterruptFrame, _error_code: u64) {}
fn handle_debug(_frame: &InterruptFrame, _error_code: u64) {}
fn handle_nmi(_frame: &InterruptFrame, _error_code: u64) {}
fn handle_breakpoint(_frame: &InterruptFrame, _error_code: u64) {}
fn handle_overflow(_frame: &InterruptFrame, _error_code: u64) {}
fn handle_bad_range(_frame: &InterruptFrame, _error_code: u64) {}
fn handle_invalid_opcode(_frame: &InterruptFrame, _error_code: u64) {}
fn handle_device_not_available(_frame: &InterruptFrame, _error_code: u64) {}
fn handle_double_fault(_frame: &InterruptFrame, _error_code: u64) {}
fn handle_coprocessor_segment_overrun(_frame: &InterruptFrame, _error_code: u64) {}
fn handle_invalid_tss(_frame: &InterruptFrame, _error_code: u64) {}
fn handle_segment_not_present(_frame: &InterruptFrame, _error_code: u64) {}
fn handle_stack_segment_fault(_frame: &InterruptFrame, _error_code: u64) {}
fn handle_general_protection_fault(_frame: &InterruptFrame, _error_code: u64) {}
fn handle_page_fault(_frame: &InterruptFrame, _error_code: u64) {}
fn handle_reserved(_frame: &InterruptFrame, _error_code: u64) {}
fn handle_x87_floating_point(_frame: &InterruptFrame, _error_code: u64) {}
fn handle_alignment_check(_frame: &InterruptFrame, _error_code: u64) {}
fn handle_machine_check(_frame: &InterruptFrame, _error_code: u64) {}
fn handle_simd_floating_point(_frame: &InterruptFrame, _error_code: u64) {}
fn handle_virtualization(_frame: &InterruptFrame, _error_code: u64) {}

fn handle_timer_tick(_frame: &InterruptFrame, _error_code: u64) {}
fn handle_keyboard(_frame: &InterruptFrame, _error_code: u64) {}

fn handle_unhandled(_number: u64, _frame: &InterruptFrame, _error_code: u64) {}

// ---------------------------------------------------------------------------
// PIC (Programmable Interrupt Controller)
// ---------------------------------------------------------------------------

const PIC1_COMMAND: u16 = 0x20;
const PIC1_DATA: u16 = 0x21;
const PIC2_COMMAND: u16 = 0xA0;
const PIC2_DATA: u16 = 0xA1;

/// Remapea el PIC legacy para que las interrupciones IRQ0-15 caigan en los
/// vectores 32-47 (evitando colapso con las excepciones 0-31).
pub unsafe fn pic_remap() {
    let _ = inb(PIC1_DATA);
    let _ = inb(PIC2_DATA);

    outb(PIC1_COMMAND, 0x11);
    outb(PIC2_COMMAND, 0x11);
    outb(PIC1_DATA, 0x20);
    outb(PIC2_DATA, 0x28);
    outb(PIC1_DATA, 0x04);
    outb(PIC2_DATA, 0x02);
    outb(PIC1_DATA, 0x01);
    outb(PIC2_DATA, 0x01);

    // Enmascarar todas las IRQ: aun no hay stubs de ISR registrados.
    outb(PIC1_DATA, 0xFF);
    outb(PIC2_DATA, 0xFF);
}

unsafe fn outb(port: u16, value: u8) {
    asm!("out dx, al", in("dx") port, in("al") value, options(nostack));
}

unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    asm!("in al, dx", out("al") value, in("dx") port, options(nostack));
    value
}

/// Envía EOI (End Of Interrupt) al PIC.
unsafe fn pic_eoi(irq: u8) {
    if irq >= 8 {
        outb(PIC2_COMMAND, 0x20);
    }
    outb(PIC1_COMMAND, 0x20);
}

/// Inicializa el subsistema de interrupciones: GDT, IDT, PIC.
pub unsafe fn init() {
    gdt::init();
    idt::init();
    pic_remap();
    asm!("sti", options(nostack));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interrupt_frame_layout() {
        assert_eq!(core::mem::size_of::<InterruptFrame>(), 40);
    }
}
