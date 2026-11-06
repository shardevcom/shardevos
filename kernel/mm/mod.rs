//! Gestor de memoria del kernel shardevos.
//!
//! Proporciona:
//! - Frame allocator (bitmap) para marcos de página físicos
//! - Paginación de 4 niveles (x86_64): PML4 → PDPT → PD → PT
//! - Heap del kernel con API `alloc`

pub mod frame_allocator;
pub mod heap;
pub mod paging;

use core::alloc::{GlobalAlloc, Layout};

/// Errores del gestor de memoria.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemError {
    OutOfMemory,
    InvalidAddress,
    AlreadyMapped,
    NotMapped,
    AlignmentError,
    FrameExhausted,
}

/// Resultado del gestor de memoria.
pub type MemResult<T> = core::result::Result<T, MemError>;

/// Tamaño de una página (4 KB).
pub const PAGE_SIZE: usize = 4096;

/// Dirección base del kernel (según linker.ld).
pub const KERNEL_BASE: usize = 0x100000; // 1 MB

/// Límite superior de la memoria física soportada (inicialmente 4 GB).
pub const MAX_PHYS_ADDR: usize = 0xFFFF_F000;

/// Alinea hacia arriba al múltiplo más cercano de PAGE_SIZE.
#[inline]
pub fn align_up(addr: usize) -> usize {
    (addr + PAGE_SIZE - 1) & !(PAGE_SIZE - 1)
}

/// Alinea hacia abajo al múltiplo más cercano de PAGE_SIZE.
#[inline]
pub fn align_down(addr: usize) -> usize {
    addr & !(PAGE_SIZE - 1)
}

/// Convierte una dirección virtual a un número de marco de página.
#[inline]
pub fn addr_to_frame(addr: usize) -> usize {
    addr / PAGE_SIZE
}

/// Convierte un número de marco de página a una dirección física.
#[inline]
pub fn frame_to_addr(frame: usize) -> usize {
    frame * PAGE_SIZE
}

/// Inicializa el subsistema de memoria (frame allocator, paginación, heap).
/// Debe llamarse desde `kernel_entry` después de `interrupts::init()`.
pub unsafe fn init() {
    frame_allocator::init();
    paging::init();
    heap::init();
}

/// Traits para un allocador global simple (placeholder hasta integrar `alloc`).
pub struct KernelAllocator;

unsafe impl GlobalAlloc for KernelAllocator {
    unsafe fn alloc(&self, _layout: Layout) -> *mut u8 {
        heap::allocate(_layout.size(), _layout.align())
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        heap::deallocate(_ptr, _layout.size(), _layout.align());
    }
}

#[global_allocator]
static ALLOCATOR: KernelAllocator = KernelAllocator;
