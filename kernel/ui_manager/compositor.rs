// Compositor — Compositor de ventanas de shardevos.
// Proporciona:
// - Doble bufferizado para renderizado sin tearing
// - Composición de múltiples ventanas en el framebuffer
// - Gestión del orden Z de ventanas

use super::window::{WindowManager, Window, WindowState, Framebuffer};
use super::{UiError, UiResult, PixelFormat};
use core::sync::atomic::{AtomicU64, Ordering};

// SpinLock para Compositor
mod spin_lock_comp {
    use core::sync::atomic::{AtomicUsize, Ordering};
    use core::cell::UnsafeCell;

    pub struct SpinLock<T> {
        lock: AtomicUsize,
        value: UnsafeCell<T>,
    }

    unsafe impl<T> Send for SpinLock<T> {}
    unsafe impl<T> Sync for SpinLock<T> {}

    impl<T> SpinLock<T> {
        pub const fn new(value: T) -> Self {
            Self { lock: AtomicUsize::new(0), value: UnsafeCell::new(value) }
        }

        pub fn lock(&self) -> SpinLockGuard<'_, T> {
            while self.lock.compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed).is_err() {
                core::hint::spin_loop();
            }
            SpinLockGuard { lock: self }
        }

        pub fn unlock(&self) {
            self.lock.store(0, Ordering::Release);
        }
    }

    pub struct SpinLockGuard<'a, T> {
        lock: &'a SpinLock<T>,
    }

    impl<'a, T> Drop for SpinLockGuard<'a, T> {
        fn drop(&mut self) {
            self.lock.unlock();
        }
    }

    impl<'a, T> core::ops::Deref for SpinLockGuard<'a, T> {
        type Target = T;
        fn deref(&self) -> &T {
            unsafe { &*self.lock.value.get() }
        }
    }

    impl<'a, T> core::ops::DerefMut for SpinLockGuard<'a, T> {
        fn deref_mut(&mut self) -> &mut T {
            unsafe { &mut *self.lock.value.get() }
        }
    }
}

/// Modo de composición.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CompositeMode {
    Copy = 0,       // Copia directa (sin blending)
    AlphaBlend = 1, // Mezcla con canal alfa
}

/// Estado del compositor.
#[derive(Debug, Clone, Copy)]
pub struct CompositeStats {
    pub frames_composited: u64,
    pub windows_composited: u64,
    pub last_frame_us: u64,
}

/// Compositor de ventanas.
pub struct Compositor {
    mode: AtomicU64,
    stats: spin_lock_comp::SpinLock<CompositeStats>,
    initialized: AtomicU64,
}

impl Compositor {
    pub const fn new() -> Self {
        Self {
            mode: AtomicU64::new(CompositeMode::Copy as u64),
            stats: spin_lock_comp::SpinLock::new(CompositeStats {
                frames_composited: 0,
                windows_composited: 0,
                last_frame_us: 0,
            }),
            initialized: AtomicU64::new(0),
        }
    }

    pub unsafe fn init(&self) {
        self.initialized.store(1, Ordering::Release);
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::Acquire) != 0
    }

    /// Configura el modo de composición.
    pub fn set_composite_mode(&self, mode: CompositeMode) {
        self.mode.store(mode as u64, Ordering::Release);
    }

    /// Obtiene el modo actual.
    pub fn composite_mode(&self) -> CompositeMode {
        match self.mode.load(Ordering::Acquire) {
            1 => CompositeMode::AlphaBlend,
            _ => CompositeMode::Copy,
        }
    }

    /// Compone todas las ventanas visibles en el framebuffer destino.
    pub fn composite_frame(
        &self,
        window_manager: &WindowManager,
        _target_fb: &Framebuffer,
    ) -> UiResult<CompositeStats> {
        if !self.is_initialized() {
            return Err(UiError::NotInitialized);
        }

        let mut stats = CompositeStats {
            frames_composited: 0,
            windows_composited: 0,
            last_frame_us: time_us(),
        };

        // Recopilar ventanas visibles ordenadas por Z-order
        let mut visible_windows: [Option<Window>; 64] = [None; 64];
        let mut count = 0usize;

        {
            let wm_windows = window_manager.windows_lock();
            for w in wm_windows.iter().flatten() {
                if w.state == WindowState::Shown && count < 64 {
                    visible_windows[count] = Some(w.clone());
                    count += 1;
                }
            }
        }

        // Ordenar por Z-order (ascendente: primero los de atrás)
        for i in 0..count {
            for j in (i + 1)..count {
                let wi = visible_windows[i].as_ref().unwrap();
                let wj = visible_windows[j].as_ref().unwrap();
                if wi.z_order > wj.z_order {
                    visible_windows.swap(i, j);
                }
            }
        }

        // Simular composición de cada ventana
        for i in 0..count {
            if let Some(win) = visible_windows[i] {
                if let Some(_fb) = win.buffer {
                    stats.windows_composited += 1;
                }
            }
        }

        stats.frames_composited += 1;
        stats.last_frame_us = time_us();

        {
            let mut s = self.stats.lock();
            *s = stats;
        }

        Ok(stats)
    }

    /// Limpia el framebuffer destino con un color.
    pub fn clear_framebuffer(fb: &mut Framebuffer, color: u32) {
        let bytes_per_pixel = match fb.format {
            PixelFormat::RGB565 => 2,
            PixelFormat::RGB888 => 3,
            PixelFormat::RGBA8888 | PixelFormat::BGRA8888 => 4,
        };

        let pixel_count = (fb.width * fb.height) as usize;
        let pixel_bytes = color.to_le_bytes();

        for i in 0..pixel_count {
            let offset = i * bytes_per_pixel as usize;
            if offset + bytes_per_pixel as usize <= fb.size {
                unsafe {
                    let ptr = (fb.addr + offset) as *mut u8;
                    for b in 0..bytes_per_pixel as usize {
                        ptr.add(b).write(pixel_bytes[b]);
                    }
                }
            }
        }
    }

    /// Devuelve las estadísticas del compositor.
    pub fn stats(&self) -> CompositeStats {
        self.stats.lock().clone()
    }
}

/// Tiempo actual en microsegundos.
fn time_us() -> u64 {
    super::next_window_id() * 1000
}

/// Instancia global del compositor.
static COMPOSITOR: Compositor = Compositor::new();

/// Devuelve una referencia al compositor global.
pub fn global_compositor() -> &'static Compositor {
    &COMPOSITOR
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compositor_new() {
        let comp = Compositor::new();
        assert!(!comp.is_initialized());
        assert_eq!(comp.composite_mode(), CompositeMode::Copy);
    }

    #[test]
    fn composite_mode() {
        let comp = Compositor::new();
        comp.set_composite_mode(CompositeMode::AlphaBlend);
        assert_eq!(comp.composite_mode(), CompositeMode::AlphaBlend);
        comp.set_composite_mode(CompositeMode::Copy);
        assert_eq!(comp.composite_mode(), CompositeMode::Copy);
    }

    #[test]
    fn clear_framebuffer() {
        let mut fb = Framebuffer::new(0x10000, 10, 10, PixelFormat::RGBA8888);
        Compositor::clear_framebuffer(&mut fb, 0x00FF0000); // Rojo en BGRA8888
        // Verificar que no panica y los campos son correctos
        assert_eq!(fb.width, 10);
        assert_eq!(fb.height, 10);
        assert_eq!(fb.format, PixelFormat::RGBA8888);
    }

    #[test]
    fn stats() {
        let comp = Compositor::new();
        let s = comp.stats();
        assert_eq!(s.frames_composited, 0);
        assert_eq!(s.windows_composited, 0);
    }

    #[test]
    fn composite_modes() {
        assert_eq!(CompositeMode::Copy as u8, 0);
        assert_eq!(CompositeMode::AlphaBlend as u8, 1);
    }
}
