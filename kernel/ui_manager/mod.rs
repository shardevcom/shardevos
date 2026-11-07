//! UI Manager — Servidor de Display y Gestor de Interfaz de Usuario de shardevos.
//!
//! Proporciona:
//! - Servidor de display con ventanas y superficies
//! - Driver de GPU/NPU para aceleración gráfica e IA
//! - Compositor de ventanas con doble bufferizado
//! - Integración con el Unified Input Manager

pub mod compositor;
pub mod display;
pub mod display_server;
pub mod responsive;
pub mod window;

pub use self::compositor::{Compositor, CompositeMode, CompositeStats, global_compositor};
pub use self::display::{Color as GfxColor, DisplayServer as GfxDisplay, FramebufferInfo as GfxFramebufferInfo, global_display as global_gfx_display};
pub use self::display_server::{DisplayServer, global_display_server};
pub use self::responsive::{Breakpoint, BreakpointRule, DeviceCategory, Orientation, ResponsiveConfig, ResponsiveManager, global_responsive_manager};
pub use self::window::{Framebuffer, Window, WindowId, WindowProperties, WindowState, WindowManager, global_window_manager};

use crate::input::{global_manager, InputEvent, InputEventType, InputSource, TouchPoint};
use core::sync::atomic::{AtomicU64, Ordering};

/// Contador global de eventos de input procesados por el UI Manager.
static UI_INPUT_EVENTS: AtomicU64 = AtomicU64::new(0);

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

/// Procesa un evento de input unificado (touch/gesture/mouse) para el UI.
pub fn handle_ui_input_event(event: InputEvent) {
    UI_INPUT_EVENTS.fetch_add(1, Ordering::Relaxed);
    if matches!(event.event_type, InputEventType::TouchDown | InputEventType::GestureTap) {
        let _ = global_window_manager().focus_topmost_visible();
    }
    if matches!(event.event_type, InputEventType::Scroll | InputEventType::GestureSwipe) {
        let focused = global_window_manager().focused_window();
        let delta = event.drag.map(|d| d.end_y - d.start_y).unwrap_or(0);
        let _ = global_window_manager().scroll_window(focused, delta);
    }
}

/// Devuelve el último contador de eventos de input procesados por el UI.
pub fn ui_input_events_last() -> u64 {
    UI_INPUT_EVENTS.load(Ordering::Acquire)
}

/// Inicializa el input manager y registra el handler de UI.
pub unsafe fn init_input() -> bool {
    let manager = global_manager();
    if !manager.is_initialized() {
        manager.init();
    }
    manager.register_handler(handle_ui_input_event).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_input_event_counter_increments() {
        let before = ui_input_events_last();
        let event = InputEvent {
            event_type: InputEventType::Scroll,
            source: InputSource::Mouse,
            timestamp_us: 0,
            sequence_id: 0,
            point: TouchPoint { x: 0, y: 0, pressure: 0, timestamp_us: 0 },
            point2: None,
            drag: None,
            pinch_scale: 0.0,
            rotation_deg: 0.0,
            touch_id: 0,
        };
        handle_ui_input_event(event);
        assert!(ui_input_events_last() > before);
    }
}
