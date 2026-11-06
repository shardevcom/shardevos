//! Frame allocator basado en bitmap.
//! Cada bit representa un marco de página de 4 KB.
//! No depende de `alloc` (puede usarse antes de inicializar el heap).

use crate::mm::{addr_to_frame, MemError, MemResult};
use core::sync::atomic::{AtomicUsize, Ordering};

/// Número máximo de marcos soportados (inicialmente 1 M de marcos = 4 GB de RAM).
const MAX_FRAMES: usize = 1_048_576;
/// Tamaño del bitmap en palabras `usize` (64 bits cada una).
const BITMAP_WORDS: usize = (MAX_FRAMES + 63) / 64;

/// Bitmap estático: bit = 1 significa marco usado.
static FRAME_BITMAP: [AtomicUsize; BITMAP_WORDS] = [const { AtomicUsize::new(0) }; BITMAP_WORDS];

/// Contador de marcos libres (aproximado, para toma de decisiones rápidas).
static FREE_FRAMES: AtomicUsize = AtomicUsize::new(MAX_FRAMES);

/// Spinlock simple para proteger operaciones de bitmap.
static BITMAP_LOCK: AtomicUsize = AtomicUsize::new(0);

/// Adquiere el spinlock.
fn lock() {
    while BITMAP_LOCK.compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed).is_err() {
        core::hint::spin_loop();
    }
}

/// Libera el spinlock.
fn unlock() {
    BITMAP_LOCK.store(0, Ordering::Release);
}

/// Inicializa el frame allocator.
/// Marca como usados los marcos ocupados por el kernel (desde 0 hasta KERNEL_BASE + tamaño del kernel).
pub unsafe fn init() {
    lock();
    // Marcar los primeros marcos (0 a 1 MB) como usados (reservados para BIOS, bootloader, etc.)
    let reserved_frames = addr_to_frame(crate::mm::KERNEL_BASE);
    for f in 0..=reserved_frames {
        set_bit(f);
    }
    // Ajustar contador de libres
    let mut free = MAX_FRAMES;
    for f in 0..=reserved_frames {
        if is_bit_set(f) {
            free -= 1;
        }
    }
    FREE_FRAMES.store(free, Ordering::Release);
    unlock();
}

/// Asigna un marco de página libre.
/// Devuelve el número de marco o `MemError::FrameExhausted`.
pub fn alloc_frame() -> MemResult<usize> {
    lock();
    for word_idx in 0..BITMAP_WORDS {
        let word = FRAME_BITMAP[word_idx].load(Ordering::Acquire);
        if word == !0usize {
            continue; // palabra completa, todos usados
        }
        // Encontrar el primer bit libre en esta palabra
        for bit in 0..64usize {
            let mask = 1usize << bit;
            if word & mask == 0 {
                let frame = word_idx * 64 + bit;
                if frame < MAX_FRAMES {
                    // Marcar como usado de forma atómica
                    let new_word = word | mask;
                    // Usar compare_exchange para evitar race conditions
                    match FRAME_BITMAP[word_idx].compare_exchange(word, new_word, Ordering::AcqRel, Ordering::Acquire) {
                        Ok(_) => {
                            FREE_FRAMES.fetch_sub(1, Ordering::Release);
                            unlock();
                            return Ok(frame);
                        }
                        Err(_actual) => {
                            // Otro hilo modificó, reintentar desde el principio de esta palabra
                            // Simplemente salimos y reintentamos externamente
                            break;
                        }
                    }
                }
            }
        }
    }
    unlock();
    Err(MemError::FrameExhausted)
}

/// Libera un marco de página previamente asignado.
pub fn dealloc_frame(frame: usize) -> MemResult<()> {
    if frame >= MAX_FRAMES {
        return Err(MemError::InvalidAddress);
    }
    lock();
    clear_bit(frame);
    FREE_FRAMES.fetch_add(1, Ordering::Release);
    unlock();
    Ok(())
}

/// Marca un marco como usado (reservado).
pub fn reserve_frame(frame: usize) -> MemResult<()> {
    if frame >= MAX_FRAMES {
        return Err(MemError::InvalidAddress);
    }
    lock();
    set_bit(frame);
    FREE_FRAMES.fetch_sub(1, Ordering::Release);
    unlock();
    Ok(())
}

/// Consulta si un marco está en uso.
pub fn is_frame_used(frame: usize) -> bool {
    if frame >= MAX_FRAMES {
        return true;
    }
    is_bit_set(frame)
}

/// Devuelve el número de marcos libres aproximado.
pub fn free_frame_count() -> usize {
    FREE_FRAMES.load(Ordering::Acquire)
}

// ---------------------------------------------------------------------------
// Operaciones internas de bitmap
// ---------------------------------------------------------------------------

fn word_index(frame: usize) -> usize {
    frame / 64
}

fn bit_index(frame: usize) -> usize {
    frame % 64
}

fn set_bit(frame: usize) {
    let w = word_index(frame);
    let b = bit_index(frame);
    let mask = 1usize << b;
    FRAME_BITMAP[w].fetch_or(mask, Ordering::AcqRel);
}

fn clear_bit(frame: usize) {
    let w = word_index(frame);
    let b = bit_index(frame);
    let mask = !(1usize << b);
    FRAME_BITMAP[w].fetch_and(mask, Ordering::AcqRel);
}

fn is_bit_set(frame: usize) -> bool {
    let w = word_index(frame);
    let b = bit_index(frame);
    FRAME_BITMAP[w].load(Ordering::Acquire) & (1usize << b) != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitmap_word_count() {
        assert_eq!(BITMAP_WORDS, 16384); // 1M frames / 64
    }

    #[test]
    fn set_and_clear_bit() {
        // Prueba lógica directa sobre AtomicUsize (simulado)
        let word = AtomicUsize::new(0);
        assert!((word.load(Ordering::Relaxed) & 1) == 0);
        word.fetch_or(1, Ordering::Relaxed);
        assert!((word.load(Ordering::Relaxed) & 1) != 0);
        word.fetch_and(!1usize, Ordering::Relaxed);
        assert!((word.load(Ordering::Relaxed) & 1) == 0);
    }
}
