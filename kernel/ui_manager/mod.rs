//! UI Manager — Servidor de Display y Gestor de Interfaz de Usuario de shardevos.
//!
//! Proporciona:
//! - Servidor de display con ventanas y superficies
//! - Driver de GPU/NPU para aceleración gráfica e IA
//! - Compositor de ventanas con doble bufferizado
//! - Integración con el Unified Input Manager

pub mod compositor;
pub mod display_server;
pub mod responsive;
pub mod window;

pub use self::compositor::{Compositor, CompositeMode, CompositeStats, global_compositor};
pub use self::display_server::{DisplayServer, global_display_server};
pub use self::responsive::{Breakpoint, BreakpointRule, DeviceCategory, Orientation, ResponsiveConfig, ResponsiveManager, global_responsive_manager};
pub use self::window::{Framebuffer, Window, WindowId, WindowProperties, WindowState, WindowManager, global_window_manager};

use core::sync::atomic::{AtomicU64, Ordering};

/// Resolución de pantalla.
#[derive(Debug, Clone, Copy)]
pub struct DisplayMode {
    pub width: u32,
    pub height: u32,
    pub refresh_rate: u32,
    pub bits_per_pixel: u32,
}

/// Formato de color del framebuffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    RGB565 = 0,
    RGB888 = 1,
    RGBA8888 = 2,
    BGRA8888 = 3,
}

/// Información de un framebuffer.
#[derive(Debug, Clone, Copy)]
pub struct FramebufferInfo {
    pub addr: usize,
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub format: PixelFormat,
    pub size: u32,
}

/// Estado del servidor de display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayState {
    Uninitialized = 0,
    Initialized = 1,
    Active = 2,
    Suspended = 3,
}

/// Errores del UI Manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiError {
    NotInitialized,
    InvalidDimensions,
    OutOfMemory,
    WindowNotFound,
    DisplayNotAvailable,
    BufferTooSmall,
}

/// Resultado de operaciones del UI Manager.
pub type UiResult<T> = Result<T, UiError>;

/// Contador global de ventanas.
static WINDOW_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Genera un ID único de ventana.
pub fn next_window_id() -> u64 {
    WINDOW_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Instancia global del servidor de display.
static DISPLAY_SERVER: AtomicU64 = AtomicU64::new(0);

/// Devuelve true si el servidor de display está inicializado.
pub fn is_initialized() -> bool {
    DISPLAY_SERVER.load(Ordering::Acquire) != 0
}

/// Inicializa el servidor de display.
pub unsafe fn init() {
    DISPLAY_SERVER.store(1, Ordering::Release);
}
