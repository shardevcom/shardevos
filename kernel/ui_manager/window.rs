// Window — Gestor de ventanas del servidor de display de shardevos.
// Proporciona:
// - Creación y gestión de ventanas
// - Posición, tamaño, visibilidad
// - Títulos y buffers de contenido

use super::{UiError, UiResult, next_window_id, PixelFormat};
use super::responsive::global_responsive_manager;
use core::sync::atomic::{AtomicU64, Ordering};

/// Identificador único de ventana.
pub type WindowId = u64;

/// Propiedades de una ventana.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct WindowProperties {
    pub title: [u8; 64],
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub visible: bool,
    pub borderless: bool,
    /// Ancho mínimo permitido (en unidades relativas 0-100 del ancho de pantalla)
    pub min_width_pct: u8,
    /// Ancho máximo permitido (en unidades relativas 0-100 del ancho de pantalla)
    pub max_width_pct: u8,
    /// Alto mínimo permitido (en unidades relativas 0-100 del alto de pantalla)
    pub min_height_pct: u8,
    /// Alto máximo permitido (en unidades relativas 0-100 del alto de pantalla)
    pub max_height_pct: u8,
    /// Si la ventana debe adaptarse automáticamente al dispositivo
    pub responsive: bool,
}

impl Default for WindowProperties {
    fn default() -> Self {
        let mut title = [0u8; 64];
        let t = b"shardevos Window";
        title[..t.len()].copy_from_slice(t);
        Self {
            title,
            x: 100,
            y: 100,
            width: 800,
            height: 600,
            visible: true,
            borderless: false,
            min_width_pct: 50,
            max_width_pct: 100,
            min_height_pct: 50,
            max_height_pct: 100,
            responsive: true,
        }
    }
}

/// Estado de una ventana.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum WindowState {
    Created = 0,
    Shown = 1,
    Hidden = 2,
    Minimized = 3,
    Maximized = 4,
    Closed = 5,
}

/// Una ventana del servidor de display.
#[derive(Debug, Clone, Copy)]
pub struct Window {
    pub id: WindowId,
    pub properties: WindowProperties,
    pub state: WindowState,
    pub buffer: Option<Framebuffer>,
    pub z_order: u32,
    pub created_at_us: u64,
}

/// Framebuffer asociado a una ventana.
#[derive(Debug, Clone, Copy)]
pub struct Framebuffer {
    pub addr: usize,
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub format: PixelFormat,
    pub size: usize,
}

impl Framebuffer {
    pub fn new(addr: usize, width: u32, height: u32, format: PixelFormat) -> Self {
        let bpp = match format {
            PixelFormat::RGB565 => 2,
            PixelFormat::RGB888 => 3,
            PixelFormat::RGBA8888 | PixelFormat::BGRA8888 => 4,
        };
        let pitch = width * bpp;
        let size = (pitch * height) as usize;
        Self { addr, width, height, pitch, format, size }
    }
}

/// Gestor de ventanas.
pub struct WindowManager {
    windows: spin_lock_wm::SpinLock<[Option<Window>; 64]>,
    window_count: AtomicU64,
    next_z_order: AtomicU64,
    initialized: AtomicU64,
    focused_window: AtomicU64,
}

// SpinLock para WindowManager
mod spin_lock_wm {
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

impl WindowManager {
    pub const fn new() -> Self {
        Self {
            windows: spin_lock_wm::SpinLock::new([None; 64]),
            window_count: AtomicU64::new(0),
            next_z_order: AtomicU64::new(0),
            initialized: AtomicU64::new(0),
            focused_window: AtomicU64::new(0),
        }
    }

    pub unsafe fn init(&self) {
        self.initialized.store(1, Ordering::Release);
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::Acquire) != 0
    }

    /// Devuelve la ventana enfocada.
    pub fn focused_window(&self) -> u64 {
        self.focused_window.load(Ordering::Acquire)
    }

    /// Enfoca una ventana.
    pub fn focus_window(&self, id: WindowId) -> UiResult<()> {
        if !self.is_initialized() {
            return Err(UiError::NotInitialized);
        }
        self.focused_window.store(id, Ordering::Release);
        Ok(())
    }

    /// Enfoca la primera ventana visible con mayor orden Z.
    pub fn focus_topmost_visible(&self) -> UiResult<WindowId> {
        if !self.is_initialized() {
            return Err(UiError::NotInitialized);
        }
        let windows = self.windows.lock();
        let mut best: Option<Window> = None;
        let mut best_z = 0u32;
        for window in windows.iter().flatten() {
            if window.state == WindowState::Shown && (best.is_none() || window.z_order > best_z) {
                best = Some(*window);
                best_z = window.z_order;
            }
        }
        let id = best.ok_or(UiError::WindowNotFound)?.id;
        self.focused_window.store(id, Ordering::Release);
        Ok(id)
    }

    /// Crea una nueva ventana.
    pub fn create_window(&self, props: WindowProperties) -> UiResult<WindowId> {
        if !self.is_initialized() {
            return Err(UiError::NotInitialized);
        }

        if props.width == 0 || props.height == 0 {
            return Err(UiError::InvalidDimensions);
        }

        let mut windows = self.windows.lock();
        let id = next_window_id();

        // Encontrar slot libre
        let slot = windows.iter().position(|w| w.is_none()).ok_or(UiError::OutOfMemory)?;

        let z_order = self.next_z_order.fetch_add(1, Ordering::Relaxed) as u32;

        let window = Window {
            id,
            properties: props,
            state: WindowState::Created,
            buffer: None,
            z_order,
            created_at_us: time_us(),
        };

        windows[slot] = Some(window);
        self.window_count.fetch_add(1, Ordering::Release);

        Ok(id)
    }

    /// Muestra una ventana.
    pub fn show_window(&self, id: WindowId) -> UiResult<()> {
        if !self.is_initialized() {
            return Err(UiError::NotInitialized);
        }

        let mut windows = self.windows.lock();
        let window = windows.iter_mut().find(|w| w.as_ref().map(|w| w.id == id).unwrap_or(false))
            .and_then(|w| w.as_mut())
            .ok_or(UiError::WindowNotFound)?;

        window.state = WindowState::Shown;
        Ok(())
    }

    /// Oculta una ventana.
    pub fn hide_window(&self, id: WindowId) -> UiResult<()> {
        if !self.is_initialized() {
            return Err(UiError::NotInitialized);
        }

        let mut windows = self.windows.lock();
        let window = windows.iter_mut().find(|w| w.as_ref().map(|w| w.id == id).unwrap_or(false))
            .and_then(|w| w.as_mut())
            .ok_or(UiError::WindowNotFound)?;

        window.state = WindowState::Hidden;
        Ok(())
    }

    /// Cierra una ventana.
    pub fn close_window(&self, id: WindowId) -> UiResult<()> {
        if !self.is_initialized() {
            return Err(UiError::NotInitialized);
        }

        let mut windows = self.windows.lock();
        let found = windows.iter().position(|w| w.as_ref().map(|w| w.id == id).unwrap_or(false));
        match found {
            Some(slot) => {
                windows[slot] = None;
                self.window_count.fetch_sub(1, Ordering::Release);
                Ok(())
            }
            None => Err(UiError::WindowNotFound),
        }
    }

    /// Obtiene una referencia a una ventana.
    pub fn get_window(&self, id: WindowId) -> UiResult<Window> {
        let windows = self.windows.lock();
        for w in windows.iter() {
            if let Some(win) = w {
                if win.id == id {
                    return Ok(*win);
                }
            }
        }
        Err(UiError::WindowNotFound)
    }

    /// Devuelve el número de ventanas activas.
    pub fn window_count(&self) -> u64 {
        self.window_count.load(Ordering::Acquire)
    }

    /// Devuelve un lock al array de ventanas para iteración segura.
    pub fn windows_lock(&self) -> spin_lock_wm::SpinLockGuard<'_, [Option<Window>; 64]> {
        self.windows.lock()
    }

    /// Asigna un framebuffer a una ventana.
    pub fn set_framebuffer(&self, id: WindowId, fb: Framebuffer) -> UiResult<()> {
        if !self.is_initialized() {
            return Err(UiError::NotInitialized);
        }

        let mut windows = self.windows.lock();
        let window = windows.iter_mut().find(|w| w.as_ref().map(|w| w.id == id).unwrap_or(false))
            .and_then(|w| w.as_mut())
            .ok_or(UiError::WindowNotFound)?;

        window.buffer = Some(fb);
        Ok(())
    }

    /// Calcula las dimensiones responsivas para una ventana según el dispositivo actual.
    pub fn calculate_responsive_size(&self, props: &WindowProperties) -> (u32, u32) {
        let mgr = global_responsive_manager();
        if !mgr.is_initialized() {
            return (props.width, props.height);
        }

        let config = mgr.current_config();
        let screen_w = mgr.screen_width();
        let screen_h = mgr.screen_height();

        // Calcular ancho basado en porcentaje y categoría
        let max_w = screen_w as u32;
        let max_h = screen_h as u32;

        let min_w = (max_w as u32 * props.min_width_pct as u32) / 100;
        let max_w_limited = (max_w as u32 * props.max_width_pct as u32) / 100;
        let min_h = (max_h as u32 * props.min_height_pct as u32) / 100;
        let max_h_limited = (max_h as u32 * props.max_height_pct as u32) / 100;

        // Aplicar factor de escala del dispositivo
        let scale = config.scale_factor;

        let mut calc_w = if props.responsive {
            ((max_w as f32) * scale * 0.8) as u32
        } else {
            props.width
        };
        let mut calc_h = if props.responsive {
            ((max_h as f32) * scale * 0.7) as u32
        } else {
            props.height
        };

        // Clamp entre mínimos y máximos
        calc_w = calc_w.clamp(min_w, max_w_limited);
        calc_h = calc_h.clamp(min_h, max_h_limited);

        // Ajustar según columnas máximas del dispositivo
        let col_width = max_w / config.max_columns as u32;
        if calc_w > col_width * config.max_columns as u32 / 2 {
            calc_w = col_width * (config.max_columns as u32 / 2);
        }

        (calc_w, calc_h)
    }

    /// Crea una ventana con dimensiones ajustadas automáticamente al dispositivo actual.
    pub fn create_window_responsive(&self, mut props: WindowProperties) -> UiResult<WindowId> {
        if !self.is_initialized() {
            return Err(UiError::NotInitialized);
        }

        // Calcular dimensiones responsivas
        let (rw, rh) = self.calculate_responsive_size(&props);
        props.width = rw;
        props.height = rh;

        // Ajustar posición para centrar en pantalla
        let mgr = global_responsive_manager();
        if mgr.is_initialized() && props.responsive {
            let screen_w = mgr.screen_width() as i32;
            let screen_h = mgr.screen_height() as i32;
            props.x = (screen_w - rw as i32) / 2;
            props.y = (screen_h - rh as i32) / 2;
        }

        self.create_window(props)
    }

    /// Ajusta una ventana existente al dispositivo actual.
    pub fn adjust_window_to_device(&self, id: WindowId) -> UiResult<()> {
        if !self.is_initialized() {
            return Err(UiError::NotInitialized);
        }

        let mgr = global_responsive_manager();
        if !mgr.is_initialized() {
            return Err(UiError::NotInitialized);
        }

        let mut windows = self.windows.lock();
        let window = windows.iter_mut().find(|w| w.as_ref().map(|w| w.id == id).unwrap_or(false))
            .and_then(|w| w.as_mut())
            .ok_or(UiError::WindowNotFound)?;

        let props = &window.properties;
        let (rw, rh) = self.calculate_responsive_size(props);

        window.properties.width = rw;
        window.properties.height = rh;

        // Reposicionar al centro
        let screen_w = mgr.screen_width() as i32;
        let screen_h = mgr.screen_height() as i32;
        window.properties.x = (screen_w - rw as i32) / 2;
        window.properties.y = (screen_h - rh as i32) / 2;

        Ok(())
    }
}

/// Tiempo actual en microsegundos.
fn time_us() -> u64 {
    // Placeholder: usar TSC/RTC en producción
    next_window_id() * 1000
}

/// Instancia global del gestor de ventanas.
static WINDOW_MANAGER: WindowManager = WindowManager::new();

/// Devuelve una referencia al gestor de ventanas global.
pub fn global_window_manager() -> &'static WindowManager {
    &WINDOW_MANAGER
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_properties_default() {
        let props = WindowProperties::default();
        assert_eq!(props.width, 800);
        assert_eq!(props.height, 600);
        assert!(props.visible);
    }

    #[test]
    fn window_states() {
        assert_eq!(WindowState::Created as u8, 0);
        assert_eq!(WindowState::Shown as u8, 1);
        assert_eq!(WindowState::Closed as u8, 5);
    }

    #[test]
    fn framebuffer_new() {
        let fb = Framebuffer::new(0x1000, 800, 600, PixelFormat::RGBA8888);
        assert_eq!(fb.width, 800);
        assert_eq!(fb.height, 600);
        assert_eq!(fb.pitch, 800 * 4);
        assert_eq!(fb.size, 800 * 600 * 4);
    }

    #[test]
    fn window_manager_new() {
        let wm = WindowManager::new();
        assert!(!wm.is_initialized());
        assert_eq!(wm.window_count(), 0);
    }

    #[test]
    fn create_and_close_window() {
        let wm = WindowManager::new();
        unsafe { wm.init(); }

        let props = WindowProperties { title: *b"Test\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0", ..Default::default() };
        let id = wm.create_window(props).unwrap();
        assert_eq!(wm.window_count(), 1);

        let window = wm.get_window(id).unwrap();
        assert_eq!(window.id, id);
        assert_eq!(window.state, WindowState::Created);

        wm.show_window(id).unwrap();
        let window = wm.get_window(id).unwrap();
        assert_eq!(window.state, WindowState::Shown);

        wm.close_window(id).unwrap();
        assert_eq!(wm.window_count(), 0);
        match wm.get_window(id) {
            Err(UiError::WindowNotFound) => {},
            _ => panic!("expected WindowNotFound"),
        }
    }

    #[test]
    fn invalid_dimensions() {
        let wm = WindowManager::new();
        unsafe { wm.init(); }
        let props = WindowProperties { width: 0, height: 600, ..Default::default() };
        assert_eq!(wm.create_window(props), Err(UiError::InvalidDimensions));
    }

    #[test]
    fn not_initialized_error() {
        let wm = WindowManager::new();
        assert_eq!(wm.create_window(WindowProperties::default()), Err(UiError::NotInitialized));
    }

    #[test]
    fn set_framebuffer() {
        let wm = WindowManager::new();
        unsafe { wm.init(); }
        let id = wm.create_window(WindowProperties::default()).unwrap();
        let fb = Framebuffer::new(0x2000, 100, 100, PixelFormat::RGB565);
        assert!(wm.set_framebuffer(id, fb).is_ok());
        let window = wm.get_window(id).unwrap();
        assert!(window.buffer.is_some());
        assert_eq!(window.buffer.unwrap().width, 100);
    }
}
