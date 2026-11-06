use core::arch::asm;

/// Un segmento de descritor GDT (8 bytes).
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct SegmentDescriptor {
    limit_low: u16,
    base_low: u16,
    base_middle: u8,
    access: u8,
    granularity: u8,
    base_high: u8,
}

impl SegmentDescriptor {
    const fn null() -> Self {
        Self { limit_low: 0, base_low: 0, base_middle: 0, access: 0, granularity: 0, base_high: 0 }
    }

    const fn code() -> Self {
        Self {
            limit_low: 0xFFFF,
            base_low: 0,
            base_middle: 0,
            access: 0x9A,
            granularity: 0xC0,
            base_high: 0,
        }
    }

    const fn data() -> Self {
        Self {
            limit_low: 0xFFFF,
            base_low: 0,
            base_middle: 0,
            access: 0x92,
            granularity: 0xCF,
            base_high: 0,
        }
    }
}

/// Tabla de descriptores GDT (3 entradas: null, code, data).
#[repr(C)]
#[repr(align(16))]
pub struct Gdt {
    null: SegmentDescriptor,
    code: SegmentDescriptor,
    data: SegmentDescriptor,
}

impl Gdt {
    pub const fn new() -> Self {
        Self {
            null: SegmentDescriptor::null(),
            code: SegmentDescriptor::code(),
            data: SegmentDescriptor::data(),
        }
    }

    /// Selectores de segmento (offsets en la GDT).
    pub const KERNEL_CODE: u16 = 0x08;
    pub const KERNEL_DATA: u16 = 0x10;

    pub fn load(&'static self) {
        unsafe {
            let ptr: *const GdtPointer = &GdtPointer::from(self as *const Gdt);
            asm!("lgdt [{}]", in(reg) ptr, options(nostack, preserves_flags));
        }
    }
}

/// Puntero GDT (10 bytes): limit (16) + base (64).
#[repr(C, packed)]
pub struct GdtPointer {
    limit: u16,
    base: u64,
}

impl GdtPointer {
    fn from(gdt: *const Gdt) -> Self {
        Self {
            limit: core::mem::size_of::<Gdt>() as u16 - 1,
            base: gdt as u64,
        }
    }
}

static GDT: Gdt = Gdt::new();

/// Instala la GDT y recarga los registros de segmento.
pub fn init() {
    GDT.load();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gdt_size_is_24_bytes() {
        assert_eq!(core::mem::size_of::<Gdt>(), 24);
    }

    #[test]
    fn gdt_pointer_size_is_10_bytes() {
        assert_eq!(core::mem::size_of::<GdtPointer>(), 10);
    }
}
