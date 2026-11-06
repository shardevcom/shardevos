//! Driver de cámara web (Visión por Computador) para el Unified Input Manager.
//!
//! Proporciona:
//! - Captura de frames de cámara web (placeholder simulado)
//! - Rastreo de mano 3D (posición X, Y, Z)
//! - Detección de gestos aéreos: pellizco, deslizamiento, toque
//! - Conversión de gestos CV a `InputEvent` unificado (misma estructura que táctil)

use super::{
    InputEvent, InputEventType, InputSource, TouchPoint, DragVector, InputResult, InputError,
    global_manager, time_us,
};
use core::sync::atomic::{AtomicBool, Ordering};

/// Posición de una mano en espacio 3D.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct Hand3D {
    pub x: i32,      // Coordenada horizontal
    pub y: i32,      // Coordenada vertical
    pub z: i32,      // Profundidad (distancia a la cámara)
    pub confidence: u8,  // 0-255, confianza del rastreo
    pub timestamp_us: u64,
}

/// Estado del rastreador de manos.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct HandTracker {
    pub left_hand: Option<Hand3D>,
    pub right_hand: Option<Hand3D>,
    pub gesture_detected: InputEventType,
    pub gesture_confidence: u8,
}

/// Frame de cámara simulado.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct CameraFrame {
    pub width: u32,
    pub height: u32,
    pub timestamp_us: u64,
    pub frame_id: u64,
}

/// Driver de cámara con visión por computador.
pub struct CameraCVDriver {
    tracker: spin_lock_cv::SpinLock<HandTracker>,
    frame_counter: AtomicU64CV,
    initialized: AtomicBool,
}

// SpinLock simple para CV (evitar duplicación de código)
mod spin_lock_cv {
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

/// Contador atómico simple para frames.
struct AtomicU64CV {
    val: core::sync::atomic::AtomicU64,
}

impl AtomicU64CV {
    const fn new(v: u64) -> Self {
        Self { val: core::sync::atomic::AtomicU64::new(v) }
    }

    fn fetch_add(&self, n: u64, _: Ordering) -> u64 {
        self.val.fetch_add(n, core::sync::atomic::Ordering::Relaxed)
    }

    #[allow(dead_code)]
    fn load(&self, _: Ordering) -> u64 {
        self.val.load(core::sync::atomic::Ordering::Relaxed)
    }
}

impl CameraCVDriver {
    /// Crea una nueva instancia del driver de cámara CV.
    pub const fn new() -> Self {
        Self {
            tracker: spin_lock_cv::SpinLock::new(HandTracker {
                left_hand: None,
                right_hand: None,
                gesture_detected: InputEventType::GestureTap,
                gesture_confidence: 0,
            }),
            frame_counter: AtomicU64CV::new(0),
            initialized: AtomicBool::new(false),
        }
    }

    /// Inicializa el driver de cámara CV.
    pub unsafe fn init(&self) {
        self.initialized.store(true, Ordering::Release);
    }

    /// Verifica si el driver está inicializado.
    pub fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::Acquire)
    }

    /// Captura un frame simulado de la cámara.
    /// En una implementación real, esto leería del hardware de la cámara.
    pub fn capture_frame(&self) -> CameraFrame {
        let id = self.frame_counter.fetch_add(1, Ordering::Relaxed);
        CameraFrame {
            width: 640,
            height: 480,
            timestamp_us: time_us(),
            frame_id: id,
        }
    }

    /// Actualiza el estado del rastreador de manos con datos simulados.
    /// En una implementación real, esto procesaría el frame de la cámara
    /// con algoritmos de visión por computador (MediaPipe, OpenCV, etc.).
    pub fn update_hand_tracking(&self, left: Option<(i32, i32, i32)>, right: Option<(i32, i32, i32)>) {
        let mut tracker = self.tracker.lock();

        tracker.left_hand = left.map(|(x, y, z)| Hand3D {
            x, y, z,
            confidence: 200,
            timestamp_us: time_us(),
        });

        tracker.right_hand = right.map(|(x, y, z)| Hand3D {
            x, y, z,
            confidence: 200,
            timestamp_us: time_us(),
        });
    }

    /// Detecta gestos aéreos basados en el estado actual de las manos.
    /// Genera eventos InputEvent unificados para el InputManager.
    pub fn detect_gestures(&self) -> InputResult<()> {
        if !self.is_initialized() {
            return Err(InputError::NotInitialized);
        }

        let tracker = self.tracker.lock();
        let left = tracker.left_hand;
        let right = tracker.right_hand;
        drop(tracker);

        // Gesto de pellizco: ambas manos detectadas con confianza alta y cercanas
        if let (Some(l), Some(r)) = (left, right) {
            if l.confidence > 150 && r.confidence > 150 {
                let dx = (l.x - r.x).abs();
                let dy = (l.y - r.y).abs();
                let distance = (dx * dx + dy * dy) as f32;

                // Si las manos están cerca, es un pellizco
                if distance < 10000.0 {
                    let scale = 1.0 + (distance / 10000.0).min(1.0);
                    let event = InputEvent::gesture_pinch(
                        InputSource::CameraCV,
                        TouchPoint { x: l.x, y: l.y, pressure: l.confidence, timestamp_us: l.timestamp_us },
                        TouchPoint { x: r.x, y: r.y, pressure: r.confidence, timestamp_us: r.timestamp_us },
                        scale,
                    );
                    global_manager().inject_event(event)?;
                    return Ok(());
                }

                // Si las manos están separadas y se mueven, es un swipe
                if distance > 40000.0 {
                    let drag = DragVector {
                        start_x: l.x,
                        start_y: l.y,
                        end_x: r.x,
                        end_y: r.y,
                        velocity: (dx + dy) as i32,
                    };
                    let event = InputEvent::gesture_swipe(InputSource::CameraCV, drag);
                    global_manager().inject_event(event)?;
                    return Ok(());
                }
            }
        }

        // Gesto de tap aéreo: una sola mano con alta confianza en posición estable
        if let Some(h) = left.or(right) {
            if h.confidence > 180 {
                let event = InputEvent::gesture_air3d(
                    InputSource::CameraCV,
                    h.x, h.y, h.z,
                    InputEventType::GestureTap,
                );
                global_manager().inject_event(event)?;
            }
        }

        Ok(())
    }

    /// Procesa un frame completo: capturar, rastrear y detectar gestos.
    pub fn process_frame(&self) -> InputResult<CameraFrame> {
        if !self.is_initialized() {
            return Err(InputError::NotInitialized);
        }

        let frame = self.capture_frame();

        // Simular datos de mano basados en el frame (placeholder)
        // En producción, aquí iría el algoritmo de visión por computador real
        let t = (frame.frame_id as f32 * 0.1) as i32;
        self.update_hand_tracking(
            Some((320 + t * 2, 240 + t, 500)),  // Mano izquierda moviéndose
            Some((320 - t, 240 - t, 520)),      // Mano derecha moviéndose
        );

        self.detect_gestures()?;

        Ok(frame)
    }

    /// Devuelve el estado actual del rastreador de manos.
    pub fn get_tracker_state(&self) -> HandTracker {
        self.tracker.lock().clone()
    }
}

/// Instancia global del driver de cámara CV.
static CAMERA_CV_DRIVER: CameraCVDriver = CameraCVDriver::new();

/// Devuelve una referencia al driver de cámara CV global.
pub fn global_camera_cv() -> &'static CameraCVDriver {
    &CAMERA_CV_DRIVER
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hand3d_default_values() {
        let h = Hand3D { x: 100, y: 200, z: 300, confidence: 255, timestamp_us: 0 };
        assert_eq!(h.x, 100);
        assert_eq!(h.y, 200);
        assert_eq!(h.z, 300);
        assert_eq!(h.confidence, 255);
    }

    #[test]
    fn camera_frame_new() {
        let frame = CameraFrame { width: 640, height: 480, timestamp_us: 0, frame_id: 1 };
        assert_eq!(frame.width, 640);
        assert_eq!(frame.height, 480);
        assert_eq!(frame.frame_id, 1);
    }

    #[test]
    fn camera_driver_new() {
        let driver = CameraCVDriver::new();
        assert!(!driver.is_initialized());
    }

    #[test]
    fn capture_frame_increments_id() {
        let driver = CameraCVDriver::new();
        let f1 = driver.capture_frame();
        let f2 = driver.capture_frame();
        assert_eq!(f2.frame_id, f1.frame_id + 1);
    }

    #[test]
    fn update_hand_tracking() {
        let driver = CameraCVDriver::new();
        driver.update_hand_tracking(Some((100, 200, 300)), Some((400, 500, 600)));
        let state = driver.get_tracker_state();
        assert!(state.left_hand.is_some());
        assert!(state.right_hand.is_some());
        assert_eq!(state.left_hand.unwrap().x, 100);
        assert_eq!(state.right_hand.unwrap().x, 400);
    }

    #[test]
    fn not_initialized_error() {
        let driver = CameraCVDriver::new();
        assert_eq!(driver.detect_gestures(), Err(InputError::NotInitialized));
        assert!(driver.process_frame().is_err());
        assert_eq!(driver.process_frame().unwrap_err(), InputError::NotInitialized);
    }

    #[test]
    fn gesture_air3d_event() {
        let ev = InputEvent::gesture_air3d(InputSource::CameraCV, 150, 250, 400, InputEventType::GestureSwipe);
        assert_eq!(ev.event_type, InputEventType::GestureSwipe);
        assert_eq!(ev.source, InputSource::CameraCV);
        assert_eq!(ev.point.x, 150);
        assert_eq!(ev.point.y, 250);
    }
}
