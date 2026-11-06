//! Driver táctil nativo — Multi-Touch para el Unified Input Manager.
//!
//! Proporciona:
//! - Registro de toques simultáneos (multi-touch) con touch_id
//! - Presión táctil (0-255)
//! - Vectores de arrastre (drag vectors)
//! - Gestos de pellizco (pinch)
//! - Inyección de eventos al InputManager global

use super::{
    InputEvent, InputSource, TouchPoint, DragVector, InputResult, InputError,
    global_manager, time_us,
};
use core::sync::atomic::{AtomicBool, Ordering};

/// Número máximo de toques simultáneos soportados.
const MAX_TOUCH_POINTS: usize = 10;

/// Estado de un punto táctil individual.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct TouchState {
    pub active: bool,
    pub touch_id: u8,
    pub x: i32,
    pub y: i32,
    pub pressure: u8,
    pub last_x: i32,
    pub last_y: i32,
    pub last_pressure: u8,
    pub timestamp_us: u64,
}

impl TouchState {
    const fn inactive() -> Self {
        Self {
            active: false,
            touch_id: 0,
            x: 0,
            y: 0,
            pressure: 0,
            last_x: 0,
            last_y: 0,
            last_pressure: 0,
            timestamp_us: 0,
        }
    }
}

/// Tabla de estado de toques simultáneos.
#[repr(C, align(16))]
pub struct TouchTable {
    points: [TouchState; MAX_TOUCH_POINTS],
    active_count: u8,
}

impl TouchTable {
    pub const fn new() -> Self {
        Self {
            points: [TouchState::inactive(); MAX_TOUCH_POINTS],
            active_count: 0,
        }
    }

    /// Encuentra un punto táctil por touch_id.
    fn find_by_id(&self, touch_id: u8) -> Option<usize> {
        for (i, pt) in self.points.iter().enumerate() {
            if pt.active && pt.touch_id == touch_id {
                return Some(i);
            }
        }
        None
    }

    /// Encuentra un slot libre para un nuevo toque.
    fn find_free_slot(&self) -> Option<usize> {
        for (i, pt) in self.points.iter().enumerate() {
            if !pt.active {
                return Some(i);
            }
        }
        None
    }
}

/// Driver táctil nativo.
pub struct TouchscreenDriver {
    table: spin_lock::SpinLock<TouchTable>,
    initialized: AtomicBool,
}

// SpinLock simple inline para evitar dependencias
mod spin_lock {
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
            Self {
                lock: AtomicUsize::new(0),
                value: UnsafeCell::new(value),
            }
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

impl TouchscreenDriver {
    /// Crea una nueva instancia del driver táctil.
    pub const fn new() -> Self {
        Self {
            table: spin_lock::SpinLock::new(TouchTable::new()),
            initialized: AtomicBool::new(false),
        }
    }

    /// Inicializa el driver táctil.
    pub unsafe fn init(&self) {
        self.initialized.store(true, Ordering::Release);
    }

    /// Verifica si el driver está inicializado.
    pub fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::Acquire)
    }

    /// Registra un evento de pulsación táctil (touch down).
    /// Devuelve el touch_id asignado.
    pub fn touch_down(&self, x: i32, y: i32, pressure: u8) -> InputResult<u8> {
        if !self.is_initialized() {
            return Err(InputError::NotInitialized);
        }

        let mut table = self.table.lock();

        // Encontrar un touch_id libre (1-254, 0 reservado para gestos)
        let touch_id = (1u8..=254u8).find(|id| {
            table.points.iter().all(|pt| !pt.active || pt.touch_id != *id)
        }).ok_or(InputError::QueueFull)?;

        let slot = table.find_free_slot().ok_or(InputError::QueueFull)?;

        let timestamp = time_us();
        table.points[slot] = TouchState {
            active: true,
            touch_id,
            x,
            y,
            pressure,
            last_x: x,
            last_y: y,
            last_pressure: pressure,
            timestamp_us: timestamp,
        };
        table.active_count += 1;

        // Inyectar evento al InputManager
        let event = InputEvent::touch_down(InputSource::TouchScreen, x, y, pressure, touch_id);
        global_manager().inject_event(event)?;

        Ok(touch_id)
    }

    /// Registra un evento de liberación táctil (touch up).
    pub fn touch_up(&self, touch_id: u8) -> InputResult<()> {
        if !self.is_initialized() {
            return Err(InputError::NotInitialized);
        }

        let mut table = self.table.lock();

        let slot = table.find_by_id(touch_id).ok_or(InputError::InvalidArgument)?;
        let pt = table.points[slot];

        let event = InputEvent::touch_up(InputSource::TouchScreen, pt.x, pt.y, touch_id);
        global_manager().inject_event(event)?;

        table.points[slot] = TouchState::inactive();
        if table.active_count > 0 {
            table.active_count -= 1;
        }

        Ok(())
    }

    /// Registra un evento de movimiento táctil (touch move).
    pub fn touch_move(&self, touch_id: u8, x: i32, y: i32, pressure: u8) -> InputResult<()> {
        if !self.is_initialized() {
            return Err(InputError::NotInitialized);
        }

        let mut table = self.table.lock();

        let slot = table.find_by_id(touch_id).ok_or(InputError::InvalidArgument)?;
        let pt = &mut table.points[slot];

        // Calcular vector de arrastre si hubo movimiento significativo
        let dx = x - pt.last_x;
        let dy = y - pt.last_y;
        let has_significant_move = dx.abs() > 2 || dy.abs() > 2;

        pt.last_x = pt.x;
        pt.last_y = pt.y;
        pt.last_pressure = pt.pressure;
        pt.x = x;
        pt.y = y;
        pt.pressure = pressure;
        pt.timestamp_us = time_us();

        // Inyectar evento de movimiento
        let event = InputEvent::touch_move(InputSource::TouchScreen, x, y, pressure, touch_id);
        global_manager().inject_event(event)?;

        // Si hay movimiento significativo, también inyectar un evento de arrastre
        if has_significant_move {
            let drag = DragVector {
                start_x: pt.last_x,
                start_y: pt.last_y,
                end_x: x,
                end_y: y,
                velocity: (dx.abs() + dy.abs()) as i32,
            };
            let drag_event = InputEvent::gesture_swipe(InputSource::TouchScreen, drag);
            global_manager().inject_event(drag_event)?;
        }

        Ok(())
    }

    /// Registra un gesto de pellizco (pinch) con dos puntos de contacto.
    pub fn gesture_pinch(&self, touch_id1: u8, touch_id2: u8, scale: f32) -> InputResult<()> {
        if !self.is_initialized() {
            return Err(InputError::NotInitialized);
        }

        let table = self.table.lock();

        let slot1 = table.find_by_id(touch_id1).ok_or(InputError::InvalidArgument)?;
        let slot2 = table.find_by_id(touch_id2).ok_or(InputError::InvalidArgument)?;

        let p1 = table.points[slot1];
        let p2 = table.points[slot2];

        drop(table);

        let event = InputEvent::gesture_pinch(
            InputSource::TouchScreen,
            TouchPoint { x: p1.x, y: p1.y, pressure: p1.pressure, timestamp_us: p1.timestamp_us },
            TouchPoint { x: p2.x, y: p2.y, pressure: p2.pressure, timestamp_us: p2.timestamp_us },
            scale,
        );
        global_manager().inject_event(event)?;

        Ok(())
    }

    /// Registra un gesto de toque rápido (tap).
    pub fn gesture_tap(&self, x: i32, y: i32) -> InputResult<()> {
        if !self.is_initialized() {
            return Err(InputError::NotInitialized);
        }

        let event = InputEvent::gesture_tap(InputSource::TouchScreen, x, y);
        global_manager().inject_event(event)?;

        Ok(())
    }

    /// Devuelve el número de toques activos actualmente.
    pub fn active_touch_count(&self) -> u8 {
        self.table.lock().active_count
    }

    /// Obtiene el estado de un toque específico.
    pub fn get_touch_state(&self, touch_id: u8) -> Option<TouchState> {
        let table = self.table.lock();
        table.find_by_id(touch_id).map(|i| table.points[i])
    }
}

/// Instancia global del driver táctil.
static TOUCHSCREEN_DRIVER: TouchscreenDriver = TouchscreenDriver::new();

/// Devuelve una referencia al driver táctil global.
pub fn global_touchscreen() -> &'static TouchscreenDriver {
    &TOUCHSCREEN_DRIVER
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touch_state_inactive() {
        let ts = TouchState::inactive();
        assert!(!ts.active);
        assert_eq!(ts.touch_id, 0);
        assert_eq!(ts.x, 0);
        assert_eq!(ts.y, 0);
    }

    #[test]
    fn touch_table_new() {
        let table = TouchTable::new();
        assert_eq!(table.active_count, 0);
        assert!(table.find_by_id(1).is_none());
        assert!(table.find_free_slot().is_some());
    }

    #[test]
    fn touch_table_find_free_slot() {
        let mut table = TouchTable::new();
        // Llenar todos los slots
        for i in 0..MAX_TOUCH_POINTS {
            table.points[i] = TouchState {
                active: true,
                touch_id: i as u8 + 1,
                x: 0, y: 0, pressure: 0,
                last_x: 0, last_y: 0, last_pressure: 0,
                timestamp_us: 0,
            };
        }
        table.active_count = MAX_TOUCH_POINTS as u8;
        assert!(table.find_free_slot().is_none());
    }

    #[test]
    fn touchscreen_driver_new() {
        let driver = TouchscreenDriver::new();
        assert!(!driver.is_initialized());
        assert_eq!(driver.active_touch_count(), 0);
    }
}
