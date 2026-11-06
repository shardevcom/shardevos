// Display Server — Servidor principal de visualización de shardevos.
// Coordina el framebuffer, el modo de display y la comunicación con
// el hardware de video (GPU/NPU).

use super::{DisplayMode, DisplayState, FramebufferInfo, UiError, UiResult};
use super::responsive::global_responsive_manager;
use core::sync::atomic::{AtomicU64, Ordering};

// SpinLock para DisplayServer
mod spin_lock_ds {
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

/// Servidor de display.
pub struct DisplayServer {
    mode: spin_lock_ds::SpinLock<DisplayMode>,
    state: AtomicU64,
    framebuffer: spin_lock_ds::SpinLock<Option<FramebufferInfo>>,
    initialized: AtomicU64,
}

impl DisplayServer {
    pub const fn new() -> Self {
        Self {
            mode: spin_lock_ds::SpinLock::new(DisplayMode {
                width: 800,
                height: 600,
                refresh_rate: 60,
                bits_per_pixel: 32,
            }),
            state: AtomicU64::new(DisplayState::Uninitialized as u64),
            framebuffer: spin_lock_ds::SpinLock::new(None),
            initialized: AtomicU64::new(0),
        }
    }

    pub unsafe fn init(&self) {
        self.initialized.store(1, Ordering::Release);
        self.state.store(DisplayState::Initialized as u64, Ordering::Release);
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::Acquire) != 0
    }

    pub fn state(&self) -> DisplayState {
        match self.state.load(Ordering::Acquire) {
            0 => DisplayState::Uninitialized,
            1 => DisplayState::Initialized,
            2 => DisplayState::Active,
            3 => DisplayState::Suspended,
            _ => DisplayState::Uninitialized,
        }
    }

    /// Configura el modo de display.
    pub fn set_mode(&self, mode: DisplayMode) -> UiResult<()> {
        if !self.is_initialized() {
            return Err(UiError::NotInitialized);
        }

        if mode.width == 0 || mode.height == 0 {
            return Err(UiError::InvalidDimensions);
        }

        let mut m = self.mode.lock();
        *m = mode;

        // Actualizar el gestor responsivo con el nuevo tamaño de pantalla
        let r_mgr = global_responsive_manager();
        if r_mgr.is_initialized() {
            r_mgr.set_screen_size(mode.width as u16, mode.height as u16);
        }

        Ok(())
    }

    /// Obtiene el modo actual.
    pub fn current_mode(&self) -> DisplayMode {
        self.mode.lock().clone()
    }

    /// Activa el display.
    pub fn activate(&self) -> UiResult<()> {
        if !self.is_initialized() {
            return Err(UiError::NotInitialized);
        }
        self.state.store(DisplayState::Active as u64, Ordering::Release);
        Ok(())
    }

    /// Suspende el display.
    pub fn suspend(&self) -> UiResult<()> {
        if !self.is_initialized() {
            return Err(UiError::NotInitialized);
        }
        self.state.store(DisplayState::Suspended as u64, Ordering::Release);
        Ok(())
    }

    /// Configura el framebuffer.
    pub fn set_framebuffer(&self, info: FramebufferInfo) -> UiResult<()> {
        if !self.is_initialized() {
            return Err(UiError::NotInitialized);
        }

        let mut fb = self.framebuffer.lock();
        *fb = Some(info);
        Ok(())
    }

    /// Obtiene la información del framebuffer actual.
    pub fn get_framebuffer(&self) -> UiResult<FramebufferInfo> {
        let fb = self.framebuffer.lock();
        fb.ok_or(UiError::DisplayNotAvailable)
    }

    /// Devuelve true si hay un framebuffer configurado.
    pub fn has_framebuffer(&self) -> bool {
        self.framebuffer.lock().is_some()
    }
}

/// Instancia global del servidor de display.
static DISPLAY_SERVER: DisplayServer = DisplayServer::new();

/// Devuelve una referencia al servidor de display global.
pub fn global_display_server() -> &'static DisplayServer {
    &DISPLAY_SERVER
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_server_new() {
        let ds = DisplayServer::new();
        assert!(!ds.is_initialized());
        assert_eq!(ds.state(), DisplayState::Uninitialized);
    }

    #[test]
    fn display_server_init() {
        let ds = DisplayServer::new();
        unsafe { ds.init(); }
        assert!(ds.is_initialized());
        assert_eq!(ds.state(), DisplayState::Initialized);
    }

    #[test]
    fn set_and_get_mode() {
        let ds = DisplayServer::new();
        unsafe { ds.init(); }

        let mode = DisplayMode { width: 1920, height: 1080, refresh_rate: 144, bits_per_pixel: 32 };
        assert!(ds.set_mode(mode).is_ok());
        let current = ds.current_mode();
        assert_eq!(current.width, 1920);
        assert_eq!(current.refresh_rate, 144);
    }

    #[test]
    fn invalid_mode() {
        let ds = DisplayServer::new();
        unsafe { ds.init(); }
        let mode = DisplayMode { width: 0, height: 600, refresh_rate: 60, bits_per_pixel: 32 };
        assert_eq!(ds.set_mode(mode), Err(UiError::InvalidDimensions));
    }

    #[test]
    fn activate_suspend() {
        let ds = DisplayServer::new();
        unsafe { ds.init(); }

        assert!(ds.activate().is_ok());
        assert_eq!(ds.state(), DisplayState::Active);

        assert!(ds.suspend().is_ok());
        assert_eq!(ds.state(), DisplayState::Suspended);
    }

    #[test]
    fn framebuffer_ops() {
        let ds = DisplayServer::new();
        unsafe { ds.init(); }

        assert!(!ds.has_framebuffer());

        let info = FramebufferInfo {
            addr: 0xA0000,
            width: 800,
            height: 600,
            pitch: 3200,
            format: PixelFormat::RGBA8888,
            size: 800 * 600 * 4,
        };
        assert!(ds.set_framebuffer(info).is_ok());
        assert!(ds.has_framebuffer());

        let fb = ds.get_framebuffer().unwrap();
        assert_eq!(fb.addr, 0xA0000);
        assert_eq!(fb.width, 800);
        assert_eq!(fb.format, PixelFormat::RGBA8888);
    }

    #[test]
    fn not_initialized_errors() {
        let ds = DisplayServer::new();
        assert_eq!(ds.set_mode(DisplayMode::default()), Err(UiError::NotInitialized));
        assert_eq!(ds.activate(), Err(UiError::NotInitialized));
        assert_eq!(ds.get_framebuffer(), Err(UiError::NotInitialized));
    }
}

impl Default for DisplayMode {
    fn default() -> Self {
        Self { width: 800, height: 600, refresh_rate: 60, bits_per_pixel: 32 }
    }
}
