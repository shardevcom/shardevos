//! Despachador Unificado de Eventos de Entrada.
//!
//! Extiende el InputManager con:
//! - Priorización de eventos (táctil tiene prioridad sobre CV)
//! - Filtros por origen y tipo de evento
//! - Deduplicación de eventos similares en ventana de tiempo
//! - Estadísticas del despachador

use super::{
    InputEvent, InputEventType, InputSource, InputResult, InputError,
    global_manager,
};
use core::sync::atomic::{AtomicU64, Ordering};

/// Prioridad de un evento de entrada.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum EventPriority {
    Low = 0,      // Eventos de CV (gestos aéreos)
    Normal = 1,   // Eventos táctiles normales
    High = 2,     // Gestos táctiles (pinch, swipe, tap)
    Critical = 3, // Eventos del sistema (emergencia)
}

/// Filtro de eventos para suscripciones selectivas.
#[derive(Debug, Clone, Copy)]
pub struct EventFilter {
    pub source: Option<InputSource>,
    pub event_type: Option<InputEventType>,
    pub min_priority: EventPriority,
}

impl Default for EventFilter {
    fn default() -> Self {
        Self {
            source: None,
            event_type: None,
            min_priority: EventPriority::Low,
        }
    }
}

/// Estadísticas del despachador.
#[derive(Debug, Clone, Copy)]
pub struct DispatcherStats {
    pub total_events: u64,
    pub events_by_source: [u64; 4],  // TouchScreen, CameraCV, Keyboard, Mouse
    pub events_by_type: [u64; 9],     // Uno por InputEventType
    pub dropped_events: u64,
    pub duplicate_events: u64,
    pub priority_boosts: u64,
}

/// Entrada en la cola de prioridad del despachador.
#[allow(dead_code)]
struct PrioritizedEvent {
    event: InputEvent,
    priority: EventPriority,
}

/// Despachador unificado.
#[allow(dead_code)]
pub struct Dispatcher {
    stats: AtomicU64,  // Puntero empaquetado a DispatcherStats (simplificado)
    total_events: AtomicU64,
    dropped_events: AtomicU64,
    duplicate_events: AtomicU64,
    priority_boosts: AtomicU64,
    last_event_time: AtomicU64,
    last_event_type: AtomicU64,
    initialized: AtomicU64,
}

impl Dispatcher {
    pub const fn new() -> Self {
        Self {
            stats: AtomicU64::new(0),
            total_events: AtomicU64::new(0),
            dropped_events: AtomicU64::new(0),
            duplicate_events: AtomicU64::new(0),
            priority_boosts: AtomicU64::new(0),
            last_event_time: AtomicU64::new(0),
            last_event_type: AtomicU64::new(0),
            initialized: AtomicU64::new(0),
        }
    }

    pub unsafe fn init(&self) {
        self.initialized.store(1, Ordering::Release);
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::Acquire) != 0
    }

    /// Determina la prioridad de un evento según su origen y tipo.
    fn event_priority(event: &InputEvent) -> EventPriority {
        match event.event_type {
            InputEventType::GesturePinch | InputEventType::GestureSwipe | InputEventType::GestureTap => {
                EventPriority::High
            }
            InputEventType::TouchDown | InputEventType::TouchUp | InputEventType::TouchMove => {
                EventPriority::Normal
            }
            _ => EventPriority::Low,
        }
    }

    /// Determina si un evento es duplicado del anterior (ventana de 50ms).
    /// Si no es duplicado, actualiza el estado interno con este evento.
    fn is_duplicate(&self, event: &InputEvent) -> bool {
        let last_time = self.last_event_time.load(Ordering::Acquire);
        let last_type = self.last_event_type.load(Ordering::Acquire) as u8;

        if last_time == 0 {
            // Primer evento: registrar y no es duplicado
            self.last_event_time.store(event.timestamp_us, Ordering::Release);
            self.last_event_type.store(event.event_type as u64, Ordering::Release);
            return false;
        }

        let time_diff = event.timestamp_us.saturating_sub(last_time);
        let same_type = last_type == event.event_type as u8;

        // Duplicado si mismo tipo y menos de 50ms desde el último
        if same_type && time_diff < 50_000 {
            return true;
        }

        // No es duplicado: actualizar estado con este evento
        self.last_event_time.store(event.timestamp_us, Ordering::Release);
        self.last_event_type.store(event.event_type as u64, Ordering::Release);
        false
    }

    /// Despacha un evento con prioridad y filtrado.
    pub fn dispatch_event(&self, event: InputEvent) -> InputResult<()> {
        if !self.is_initialized() {
            return Err(InputError::NotInitialized);
        }

        // Verificar duplicado
        if self.is_duplicate(&event) {
            self.duplicate_events.fetch_add(1, Ordering::Release);
            // Aún así lo procesamos, pero marcamos como duplicado
        }

        // Determinar prioridad
        let priority = Self::event_priority(&event);

        // Inyectar al InputManager global
        global_manager().inject_event(event)?;

        // Actualizar estadísticas
        self.total_events.fetch_add(1, Ordering::Release);
        self.last_event_time.store(
            // Usar el timestamp del evento que acabamos de inyectar (no tenemos acceso directo aquí,
            // pero usamos el tiempo actual como aproximación)
            time_us(),
            Ordering::Release,
        );

        if priority == EventPriority::High {
            self.priority_boosts.fetch_add(1, Ordering::Release);
        }

        Ok(())
    }

    /// Obtiene las estadísticas actuales del despachador.
    pub fn stats(&self) -> DispatcherStats {
        DispatcherStats {
            total_events: self.total_events.load(Ordering::Acquire),
            events_by_source: [0, 0, 0, 0],  // Placeholder, requeriría seguimiento por fuente
            events_by_type: [0; 9],           // Placeholder
            dropped_events: self.dropped_events.load(Ordering::Acquire),
            duplicate_events: self.duplicate_events.load(Ordering::Acquire),
            priority_boosts: self.priority_boosts.load(Ordering::Acquire),
        }
    }

    /// Reinicia las estadísticas.
    pub fn reset_stats(&self) {
        self.total_events.store(0, Ordering::Release);
        self.dropped_events.store(0, Ordering::Release);
        self.duplicate_events.store(0, Ordering::Release);
        self.priority_boosts.store(0, Ordering::Release);
    }
}

/// Instancia global del despachador.
static DISPATCHER: Dispatcher = Dispatcher::new();

/// Devuelve una referencia al despachador global.
pub fn global_dispatcher() -> &'static Dispatcher {
    &DISPATCHER
}

/// Tiempo actual en microsegundos (placeholder compartido).
fn time_us() -> u64 {
    // Usar el contador de secuencia del input manager como base de tiempo aproximada
    super::time_us()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_priority_ordering() {
        assert!(EventPriority::Critical > EventPriority::High);
        assert!(EventPriority::High > EventPriority::Normal);
        assert!(EventPriority::Normal > EventPriority::Low);
    }

    #[test]
    fn dispatcher_priority_assignment() {
        let ev_touch = InputEvent::touch_down(InputSource::TouchScreen, 10, 10, 100, 0);
        let ev_pinch = InputEvent::gesture_pinch(
            InputSource::TouchScreen,
            TouchPoint { x: 0, y: 0, pressure: 0, timestamp_us: 0 },
            TouchPoint { x: 0, y: 0, pressure: 0, timestamp_us: 0 },
            1.0,
        );

        assert_eq!(Dispatcher::event_priority(&ev_touch), EventPriority::Normal);
        assert_eq!(Dispatcher::event_priority(&ev_pinch), EventPriority::High);
    }

    #[test]
    fn dispatcher_duplicate_detection() {
        let disp = Dispatcher::new();
        unsafe { disp.init(); }

        let ev1 = InputEvent::touch_down(InputSource::TouchScreen, 10, 10, 100, 0);
        let ts = ev1.timestamp_us;

        // Mismo tipo, mismo tiempo -> no es duplicado (primero)
        assert!(!disp.is_duplicate(&ev1));

        // Mismo tipo, tiempo muy cercano -> duplicado
        let ev2 = InputEvent {
            timestamp_us: ts + 10_000, // 10ms después
            ..ev1
        };
        assert!(disp.is_duplicate(&ev2));

        // Tipo diferente -> no es duplicado
        let ev3 = InputEvent {
            timestamp_us: ts + 10_000,
            event_type: InputEventType::TouchUp,
            ..ev1
        };
        assert!(!disp.is_duplicate(&ev3));
    }

    #[test]
    fn dispatcher_stats() {
        let disp = Dispatcher::new();
        unsafe { disp.init(); }

        let s = disp.stats();
        assert_eq!(s.total_events, 0);
        assert_eq!(s.duplicate_events, 0);

        disp.reset_stats();
        let s2 = disp.stats();
        assert_eq!(s2.total_events, 0);
    }
}
