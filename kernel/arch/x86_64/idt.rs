use core::arch::asm;

use super::gdt;

/// Entrada de la IDT (descriptor de puerta de interrupción, 16 bytes).
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct IdtEntry {
    offset_low: u16,
    selector: u16,
    ist: u8,
    type_attr: u8,
    offset_middle: u16,
    offset_high: u32,
    reserved: u32,
}

impl IdtEntry {
    const fn missing() -> Self {
        Self {
            offset_low: 0,
            selector: 0,
            ist: 0,
            type_attr: 0,
            offset_middle: 0,
            offset_high: 0,
            reserved: 0,
        }
    }

    /// Crea una entrada para un manejador de interrupción.
    pub const fn handler(handler_addr: u64, selector: u16, type_attr: u8) -> Self {
        Self {
            offset_low: (handler_addr & 0xFFFF) as u16,
            selector,
            ist: 0,
            type_attr,
            offset_middle: ((handler_addr >> 16) & 0xFFFF) as u16,
            offset_high: ((handler_addr >> 32) & 0xFFFFFFFF) as u32,
            reserved: 0,
        }
    }

    pub const PRESENT: u8 = 0x80;
    pub const RING0: u8 = 0x00;
    pub const INTERRUPT_GATE_64: u8 = 0x0E;
}

const IDT_ENTRIES: usize = 256;

/// Tabla de descriptores de interrupción (256 entradas).
#[repr(C)]
#[repr(align(16))]
pub struct Idt {
    entries: [IdtEntry; IDT_ENTRIES],
}

impl Idt {
    pub const fn new() -> Self {
        let entries = [IdtEntry::missing(); IDT_ENTRIES];
        Self { entries }
    }

    pub fn set_entry(&mut self, index: usize, entry: IdtEntry) {
        if index < IDT_ENTRIES {
            self.entries[index] = entry;
        }
    }

    pub fn entries(&self) -> &[IdtEntry; IDT_ENTRIES] {
        &self.entries
    }
}

/// Puntero IDT (10 bytes): limit (16) + base (64).
#[repr(C, packed)]
pub struct IdtPointer {
    limit: u16,
    base: u64,
}

impl IdtPointer {
    fn from(idt: *const Idt) -> Self {
        Self {
            limit: (core::mem::size_of::<Idt>() - 1) as u16,
            base: idt as u64,
        }
    }
}

/// Tabla de interrupciones del kernel.
static mut IDT: Idt = Idt::new();

/// Registra un manejador de interrupción en un índice específico.
pub fn set_handler(index: usize, handler_addr: u64, type_attr: u8) {
    unsafe {
        let idt_ptr: *mut Idt = &raw mut IDT;
        (*idt_ptr).set_entry(
            index,
            IdtEntry::handler(
                handler_addr,
                gdt::Gdt::KERNEL_CODE,
                type_attr | IdtEntry::PRESENT | IdtEntry::RING0,
            ),
        );
    }
}

/// Carga la IDT en el procesador (`lidt`).
pub unsafe fn load() {
    let ptr = IdtPointer::from(&raw const IDT);
    let ptr_ref: *const IdtPointer = &ptr;
    asm!("lidt [{}]", in(reg) ptr_ref, options(nostack, preserves_flags));
}

/// Instala la IDT completa (vacia, con handlers por defecto).
pub fn init() {
    unsafe {
        // Inicializar todas las entradas con un handler por defecto (stub)
        let idt_ptr: *mut Idt = &raw mut IDT;
        for i in 0..IDT_ENTRIES {
            (*idt_ptr).set_entry(
                i,
                IdtEntry::handler(
                    default_isr as *const () as u64,
                    gdt::Gdt::KERNEL_CODE,
                    IdtEntry::PRESENT | IdtEntry::RING0 | IdtEntry::INTERRUPT_GATE_64,
                ),
            );
        }
        load();
    }
}

/// Manejador por defecto para interrupciones no implementadas.
extern "C" fn default_isr() {
    // Loop infinito (placeholder)
    loop {
        core::hint::spin_loop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idt_size_is_4096_bytes() {
        assert_eq!(core::mem::size_of::<Idt>(), 256 * 16);
    }

    #[test]
    fn idt_pointer_size_is_10_bytes() {
        assert_eq!(core::mem::size_of::<IdtPointer>(), 10);
    }

    #[test]
    fn set_handler_writes_entry() {
        let mut idt = Idt::new();
        idt.set_entry(0, IdtEntry::handler(0x12345678, 0x08, 0x8E));
        let e = idt.entries()[0];
        assert_eq!(e.offset_low, 0x5678);
        assert_eq!(e.selector, 0x08);
    }
}
