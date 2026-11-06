//! Unified Input Manager — Gestor de Entrada Unificado de shardevos.
//!
//! Abstrae y fusiona eventos de dos fuentes principales:
//! 1. Drivers táctiles nativos (multi-touch)
//! 2. Visión por computador (cámara web, gestos aéreos 3D)
//!
//! Ambos tipos de evento se convierten a la misma estructura `InputEvent`.


pub mod touchscreen;
pub mod camera_cv;
pub mod dispatcher;

use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// Spinlock simple para entornos `no_std` sin dependencias externas.
struct SpinLock<T> {
    lock: AtomicUsize,
    value: core::cell::UnsafeCell<T>,
}

unsafe impl<T> Send for SpinLock<T> {}
unsafe impl<T> Sync for SpinLock<T> {}

impl<T> SpinLock<T> {
    const fn new(value: T) -> Self {
        Self {
            lock: AtomicUsize::new(0),
            value: core::cell::UnsafeCell::new(value),
        }
    }

    fn lock(&self) -> SpinLockGuard<'_, T> {
        while self.lock.compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed).is_err() {
            core::hint::spin_loop();
        }
        SpinLockGuard { lock: self }
    }

    fn unlock(&self) {
        self.lock.store(0, Ordering::Release);
    }
}

struct SpinLockGuard<'a, T> {
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

/// Tamaño máximo de la cola de eventos de entrada.
const INPUT_QUEUE_SIZE: usize = 256;

/// Tipos de eventos de entrada soportados.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum InputEventType {
    /// Evento de toque táctil (pulsación)
    TouchDown = 0,
    /// Evento de liberación táctil
    TouchUp = 1,
    /// Evento de movimiento táctil
    TouchMove = 2,
    /// Gesto de pellizco (pinch)
    GesturePinch = 3,
    /// Gesto de deslizamiento (swipe)
    GestureSwipe = 4,
    /// Gesto de toque rápido (tap)
    GestureTap = 5,
    /// Evento de presión táctil
    TouchPressure = 6,
    /// Evento de gestos aéreos 3D (mano)
    GestureAir3D = 7,
    /// Evento de desplazamiento (scroll)
    Scroll = 8,
}

/// Fuente del evento de entrada.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum InputSource {
    /// Pantalla táctil nativa
    TouchScreen = 0,
    /// Cámara web (visión por computador)
    CameraCV = 1,
    /// Teclado (futuro)
    Keyboard = 2,
    /// Ratón (futuro)
    Mouse = 3,
}

/// Punto de coordenadas 2D con presión.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct TouchPoint {
    pub x: i32,
    pub y: i32,
    pub pressure: u8,      // 0-255
    pub timestamp_us: u64,
}

/// Vector de arrastre (para gestos de deslizamiento).
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct DragVector {
    pub start_x: i32,
    pub start_y: i32,
    pub end_x: i32,
    pub end_y: i32,
    pub velocity: i32,
}

/// Estructura unificada de evento de entrada.
/// Tanto eventos táctiles como gestos de CV usan esta misma estructura.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct InputEvent {
    pub event_type: InputEventType,
    pub source: InputSource,
    pub timestamp_us: u64,
    pub sequence_id: u64,
    /// Punto principal (dedo, mano, cursor)
    pub point: TouchPoint,
    /// Segundo punto (para multitouch, pellizco, etc.)
    pub point2: Option<TouchPoint>,
    /// Vector de arrastre (para swipe, drag)
    pub drag: Option<DragVector>,
    /// Escala de pellizco (1.0 = sin cambio, <1.0 = acercar, >1.0 = alejar)
    pub pinch_scale: f32,
    /// Rotación en grados (para gestos de rotación)
    pub rotation_deg: f32,
    /// Identificador de sesion/touch (para distinguir dedos simultáneos)
    pub touch_id: u8,
}

/// Resultado de operaciones del input manager.
pub type InputResult<T> = Result<T, InputError>;

/// Errores del Unified Input Manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputError {
    QueueFull,
    QueueEmpty,
    InvalidSource,
    InvalidEvent,
    InvalidArgument,
    DriverNotFound,
    NotInitialized,
}

/// Generador atómico de IDs de secuencia para eventos.
static SEQ_ID: AtomicU64 = AtomicU64::new(1);

/// Cola circular de eventos de entrada.
struct InputQueue {
    buffer: [Option<InputEvent>; INPUT_QUEUE_SIZE],
    head: usize,
    tail: usize,
    count: usize,
}

impl InputQueue {
    const fn new() -> Self {
        Self {
            buffer: [None; INPUT_QUEUE_SIZE],
            head: 0,
            tail: 0,
            count: 0,
        }
    }

    fn push(&mut self, event: InputEvent) -> InputResult<()> {
        if self.count >= INPUT_QUEUE_SIZE {
            return Err(InputError::QueueFull);
        }
        self.buffer[self.tail] = Some(event);
        self.tail = (self.tail + 1) % INPUT_QUEUE_SIZE;
        self.count += 1;
        Ok(())
    }

    fn pop(&mut self) -> InputResult<InputEvent> {
        if self.count == 0 {
            return Err(InputError::QueueEmpty);
        }
        let event = self.buffer[self.head].take().ok_or(InputError::QueueEmpty)?;
        self.head = (self.head + 1) % INPUT_QUEUE_SIZE;
        self.count -= 1;
        Ok(event)
    }

    #[allow(dead_code)]
    fn is_empty(&self) -> bool {
        self.count == 0
    }

    fn len(&self) -> usize {
        self.count
    }
}

/// Manejador de eventos (callback) registrado por los consumidores.
type InputEventHandler = fn(InputEvent);

/// Gestor principal de entrada unificado.
pub struct InputManager {
    queue: SpinLock<InputQueue>,
    handlers: SpinLock<[Option<InputEventHandler>; 16]>,
    handler_count: SpinLock<usize>,
    initialized: AtomicU64,
}

impl InputManager {
    /// Crea una nueva instancia del InputManager.
    pub const fn new() -> Self {
        Self {
            queue: SpinLock::new(InputQueue::new()),
            handlers: SpinLock::new([None; 16]),
            handler_count: SpinLock::new(0),
            initialized: AtomicU64::new(0),
        }
    }

    /// Inicializa el gestor de entrada.
    pub unsafe fn init(&self) {
        self.initialized.store(1, Ordering::Release);
    }

    /// Verifica si el gestor está inicializado.
    pub fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::Acquire) != 0
    }

    /// Inyecta un evento de entrada en la cola.
    pub fn inject_event(&self, event: InputEvent) -> InputResult<()> {
        if !self.is_initialized() {
            return Err(InputError::NotInitialized);
        }
        self.queue.lock().push(event)
    }

    /// Obtiene el siguiente evento de la cola (bloqueante no, retorna inmediatamente).
    pub fn poll_event(&self) -> InputResult<InputEvent> {
        if !self.is_initialized() {
            return Err(InputError::NotInitialized);
        }
        self.queue.lock().pop()
    }

    /// Devuelve el número de eventos pendientes en la cola.
    pub fn len(&self) -> usize {
        self.queue.lock().len()
    }

    /// Registra un manejador de eventos (callback).
    pub fn register_handler(&self, handler: InputEventHandler) -> InputResult<usize> {
        let mut handlers = self.handlers.lock();
        let mut count = self.handler_count.lock();
        if *count >= 16 {
            return Err(InputError::QueueFull);
        }
        let idx = *count;
        handlers[idx] = Some(handler);
        *count += 1;
        Ok(idx)
    }

    /// Despacha un evento a todos los manejadores registrados.
    fn dispatch(&self, event: InputEvent) {
        let handlers = self.handlers.lock();
        let count = self.handler_count.lock();
        for i in 0..*count {
            if let Some(h) = handlers[i] {
                h(event);
            }
        }
    }

    /// Procesa la cola: despacha todos los eventos pendientes a los manejadores.
    pub fn process_queue(&self) {
        loop {
            match self.queue.lock().pop() {
                Ok(event) => self.dispatch(event),
                Err(InputError::QueueEmpty) => break,
                Err(_) => break,
            }
        }
    }
}

/// Instancia global del gestor de entrada.
static INPUT_MANAGER: InputManager = InputManager::new();

/// Devuelve una referencia al gestor global.
pub fn global_manager() -> &'static InputManager {
    &INPUT_MANAGER
}

// ---------------------------------------------------------------------------
// Constructores de eventos de ayuda
// ---------------------------------------------------------------------------

impl InputEvent {
    /// Crea un evento de pulsación táctil.
    pub fn touch_down(source: InputSource, x: i32, y: i32, pressure: u8, touch_id: u8) -> Self {
        Self {
            event_type: InputEventType::TouchDown,
            source,
            timestamp_us: time_us(),
            sequence_id: SEQ_ID.fetch_add(1, Ordering::Relaxed),
            point: TouchPoint { x, y, pressure, timestamp_us: time_us() },
            point2: None,
            drag: None,
            pinch_scale: 1.0,
            rotation_deg: 0.0,
            touch_id,
        }
    }

    /// Crea un evento de liberación táctil.
    pub fn touch_up(source: InputSource, x: i32, y: i32, touch_id: u8) -> Self {
        Self {
            event_type: InputEventType::TouchUp,
            source,
            timestamp_us: time_us(),
            sequence_id: SEQ_ID.fetch_add(1, Ordering::Relaxed),
            point: TouchPoint { x, y, pressure: 0, timestamp_us: time_us() },
            point2: None,
            drag: None,
            pinch_scale: 1.0,
            rotation_deg: 0.0,
            touch_id,
        }
    }

    /// Crea un evento de movimiento táctil.
    pub fn touch_move(source: InputSource, x: i32, y: i32, pressure: u8, touch_id: u8) -> Self {
        Self {
            event_type: InputEventType::TouchMove,
            source,
            timestamp_us: time_us(),
            sequence_id: SEQ_ID.fetch_add(1, Ordering::Relaxed),
            point: TouchPoint { x, y, pressure, timestamp_us: time_us() },
            point2: None,
            drag: None,
            pinch_scale: 1.0,
            rotation_deg: 0.0,
            touch_id,
        }
    }

    /// Crea un gesto de pellizco (pinch).
    pub fn gesture_pinch(source: InputSource, p1: TouchPoint, p2: TouchPoint, scale: f32) -> Self {
        Self {
            event_type: InputEventType::GesturePinch,
            source,
            timestamp_us: time_us(),
            sequence_id: SEQ_ID.fetch_add(1, Ordering::Relaxed),
            point: p1,
            point2: Some(p2),
            drag: None,
            pinch_scale: scale,
            rotation_deg: 0.0,
            touch_id: 0,
        }
    }

    /// Crea un gesto de deslizamiento (swipe).
    pub fn gesture_swipe(source: InputSource, drag: DragVector) -> Self {
        Self {
            event_type: InputEventType::GestureSwipe,
            source,
            timestamp_us: time_us(),
            sequence_id: SEQ_ID.fetch_add(1, Ordering::Relaxed),
            point: TouchPoint { x: drag.start_x, y: drag.start_y, pressure: 0, timestamp_us: time_us() },
            point2: None,
            drag: Some(drag),
            pinch_scale: 1.0,
            rotation_deg: 0.0,
            touch_id: 0,
        }
    }

    /// Crea un gesto de toque rápido (tap).
    pub fn gesture_tap(source: InputSource, x: i32, y: i32) -> Self {
        Self {
            event_type: InputEventType::GestureTap,
            source,
            timestamp_us: time_us(),
            sequence_id: SEQ_ID.fetch_add(1, Ordering::Relaxed),
            point: TouchPoint { x, y, pressure: 255, timestamp_us: time_us() },
            point2: None,
            drag: None,
            pinch_scale: 1.0,
            rotation_deg: 0.0,
            touch_id: 0,
        }
    }

    /// Crea un evento de gesto aéreo 3D (desde cámara CV).
    pub fn gesture_air3d(source: InputSource, x: i32, y: i32, z: i32, gesture_type: InputEventType) -> Self {
        let _ = z; // El Z se usa internamente por el driver CV, aquí se omite para simplificar
        Self {
            event_type: gesture_type,
            source,
            timestamp_us: time_us(),
            sequence_id: SEQ_ID.fetch_add(1, Ordering::Relaxed),
            point: TouchPoint { x, y, pressure: 128, timestamp_us: time_us() },
            point2: None,
            drag: None,
            pinch_scale: 1.0,
            rotation_deg: 0.0,
            touch_id: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Utilidades internas
// ---------------------------------------------------------------------------

/// Obtiene el tiempo actual en microsegundos (placeholder, usar RTC/HPET en producción).
fn time_us() -> u64 {
    // Placeholder: retornar un valor basado en un contador simple.
    // En una implementación real, leer desde el TSC o RTC.
    SEQ_ID.load(Ordering::Relaxed) * 1000
}

impl Default for InputManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_event_types() {
        assert_eq!(InputEventType::TouchDown as u8, 0);
        assert_eq!(InputEventType::GesturePinch as u8, 3);
        assert_eq!(InputEventType::GestureSwipe as u8, 4);
        assert_eq!(InputEventType::GestureTap as u8, 5);
    }

    #[test]
    fn input_sources() {
        assert_eq!(InputSource::TouchScreen as u8, 0);
        assert_eq!(InputSource::CameraCV as u8, 1);
    }

    #[test]
    fn touch_down_event() {
        let ev = InputEvent::touch_down(InputSource::TouchScreen, 100, 200, 150, 1);
        assert_eq!(ev.event_type, InputEventType::TouchDown);
        assert_eq!(ev.source, InputSource::TouchScreen);
        assert_eq!(ev.point.x, 100);
        assert_eq!(ev.point.y, 200);
        assert_eq!(ev.point.pressure, 150);
        assert_eq!(ev.touch_id, 1);
    }

    #[test]
    fn gesture_pinch_event() {
        let p1 = TouchPoint { x: 50, y: 50, pressure: 100, timestamp_us: 0 };
        let p2 = TouchPoint { x: 150, y: 150, pressure: 100, timestamp_us: 0 };
        let ev = InputEvent::gesture_pinch(InputSource::TouchScreen, p1, p2, 1.5);
        assert_eq!(ev.event_type, InputEventType::GesturePinch);
        assert!(ev.point2.is_some());
        assert!((ev.pinch_scale - 1.5).abs() < 0.001);
    }

    #[test]
    fn gesture_swipe_event() {
        let drag = DragVector { start_x: 0, start_y: 0, end_x: 200, end_y: 0, velocity: 500 };
        let ev = InputEvent::gesture_swipe(InputSource::TouchScreen, drag);
        assert_eq!(ev.event_type, InputEventType::GestureSwipe);
        assert!(ev.drag.is_some());
        assert_eq!(ev.drag.unwrap().end_x, 200);
    }

    #[test]
    fn gesture_tap_event() {
        let ev = InputEvent::gesture_tap(InputSource::CameraCV, 300, 400);
        assert_eq!(ev.event_type, InputEventType::GestureTap);
        assert_eq!(ev.source, InputSource::CameraCV);
        assert_eq!(ev.point.x, 300);
        assert_eq!(ev.point.y, 400);
    }

    #[test]
    fn gesture_air3d_event() {
        let ev = InputEvent::gesture_air3d(InputSource::CameraCV, 100, 200, 50, InputEventType::GestureSwipe);
        assert_eq!(ev.event_type, InputEventType::GestureSwipe);
        assert_eq!(ev.source, InputSource::CameraCV);
    }

    #[test]
    fn queue_push_pop() {
        let mgr = InputManager::new();
        unsafe { mgr.init(); }

        let ev1 = InputEvent::touch_down(InputSource::TouchScreen, 10, 10, 100, 0);
        let ev2 = InputEvent::touch_up(InputSource::TouchScreen, 10, 10, 0);

        assert!(mgr.inject_event(ev1).is_ok());
        assert!(mgr.inject_event(ev2).is_ok());
        assert_eq!(mgr.queue.lock().len(), 2);

        let popped = mgr.poll_event().unwrap();
        assert_eq!(popped.event_type, InputEventType::TouchDown);
        assert_eq!(mgr.queue.lock().len(), 1);

        let popped2 = mgr.poll_event().unwrap();
        assert_eq!(popped2.event_type, InputEventType::TouchUp);

        assert_eq!(mgr.poll_event().unwrap_err(), InputError::QueueEmpty);
    }

    #[test]
    fn queue_full() {
        let mgr = InputManager::new();
        unsafe { mgr.init(); }

        for i in 0..INPUT_QUEUE_SIZE {
            let ev = InputEvent::touch_down(InputSource::TouchScreen, i as i32, 0, 100, 0);
            assert!(mgr.inject_event(ev).is_ok());
        }

        let ev = InputEvent::touch_down(InputSource::TouchScreen, 999, 999, 100, 0);
        assert_eq!(mgr.inject_event(ev), Err(InputError::QueueFull));
    }

    #[test]
    fn not_initialized_error() {
        let mgr = InputManager::new();
        let ev = InputEvent::touch_down(InputSource::TouchScreen, 0, 0, 100, 0);
        assert_eq!(mgr.inject_event(ev), Err(InputError::NotInitialized));
        assert_eq!(mgr.poll_event().unwrap_err(), InputError::NotInitialized);
    }
}
