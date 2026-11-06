//! Heap del kernel con bump allocator simple.
//! Reserva marcos físicos, los mapea al espacio virtual del kernel y gestiona
//! un puntero de asignación lineal (bump pointer). `deallocate` es un no-op
//! en esta fase.

use crate::mm::{frame_to_addr, PAGE_SIZE};
use core::ptr::null_mut;
use core::sync::atomic::{AtomicUsize, Ordering};

/// Tamaño del heap en bytes (4 MB = 1024 marcos).
const HEAP_SIZE: usize = 4 * 1024 * 1024;

/// Dirección base del heap en el espacio de direcciones del kernel.
const HEAP_BASE: usize = 0xFFFF_8000_0000_0000;

/// Puntero de asignación actual (bump pointer).
static HEAP_PTR: AtomicUsize = AtomicUsize::new(HEAP_BASE);

/// Indica si el heap está inicializado.
static HEAP_INIT: AtomicUsize = AtomicUsize::new(0);

/// Dirección final del heap (exclusiva).
const HEAP_END: usize = HEAP_BASE + HEAP_SIZE;

/// Inicializa el heap.
pub unsafe fn init() {
    if HEAP_INIT.load(Ordering::Acquire) != 0 {
        return;
    }

    let frame_count = HEAP_SIZE / PAGE_SIZE;
    let mut start_frame: Option<usize> = None;

    // Asignar marcos para el heap
    for _ in 0..frame_count {
        match crate::mm::frame_allocator::alloc_frame() {
            Ok(f) => {
                if start_frame.is_none() {
                    start_frame = Some(f);
                }
            }
            Err(_) => break,
        }
    }

    let start_frame = match start_frame {
        Some(f) => f,
        None => return, // No se pudo asignar ningún marco
    };

    // Mapear marcos al espacio virtual del heap
    let mut vaddr = HEAP_BASE;
    let mut frame = start_frame;
    let mut mapped = 0usize;

    while mapped < frame_count {
        let paddr = frame_to_addr(frame);
        let _ = crate::mm::paging::map(
            vaddr,
            paddr,
            crate::mm::paging::PAGE_PRESENT | crate::mm::paging::PAGE_WRITABLE,
        );
        vaddr += PAGE_SIZE;
        frame += 1;
        mapped += 1;
    }

    HEAP_PTR.store(HEAP_BASE, Ordering::Release);
    HEAP_INIT.store(1, Ordering::Release);
}

/// Asigna memoria en el heap.
pub fn allocate(size: usize, align: usize) -> *mut u8 {
    if HEAP_INIT.load(Ordering::Acquire) == 0 {
        return null_mut();
    }

    let mut ptr = HEAP_PTR.load(Ordering::Acquire);
    loop {
        let aligned_ptr = (ptr + align - 1) & !(align - 1);
        let next_ptr = aligned_ptr + size;

        if next_ptr > HEAP_END {
            return null_mut();
        }

        match HEAP_PTR.compare_exchange(ptr, next_ptr, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return aligned_ptr as *mut u8,
            Err(current) => ptr = current,
        }
    }
}

/// Libera memoria (no-op en bump allocator).
pub fn deallocate(_ptr: *mut u8, _size: usize, _align: usize) {
    // No-op: bump allocator no libera individualmente
}

/// Bytes asignados aproximados.
pub fn allocated_bytes() -> usize {
    HEAP_PTR.load(Ordering::Acquire) - HEAP_BASE
}

pub fn is_initialized() -> bool {
    HEAP_INIT.load(Ordering::Acquire) != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn align_up_works() {
        assert_eq!((1usize + 8 - 1) & !(8 - 1), 8);
        assert_eq!((8usize + 8 - 1) & !(8 - 1), 8);
        assert_eq!((9usize + 8 - 1) & !(8 - 1), 16);
    }

    #[test]
    fn heap_size_is_4mb() {
        assert_eq!(HEAP_SIZE, 4 * 1024 * 1024);
    }
}
