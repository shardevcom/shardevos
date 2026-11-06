//! AI Scheduler Predictivo — Optimización de CPU/RAM basada en patrones de uso.
//!
//! Proporciona:
//! - Monitoreo de uso de recursos por tarea (CPU %, RAM bytes)
//! - Historial de uso para predicción
//! - Predicción de necesidades futuras (promedio móvil ponderado)
//! - Ajuste adaptativo de prioridades de tareas
//! - Integración con el Scheduler base


use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use super::{Scheduler, TaskId, TaskPriority};

/// Métricas de recursos de una tarea en un momento dado.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct ResourceSample {
    pub timestamp_us: u64,
    pub cpu_ticks: u64,
    pub mem_bytes: u64,
    pub task_id: TaskId,
}

/// Historial de muestras de recursos para una tarea.
#[derive(Debug, Clone, Copy)]
pub struct ResourceHistory {
    pub task_id: TaskId,
    pub samples: [ResourceSample; 64],
    pub sample_count: usize,
    pub sample_index: usize,
}

/// Predicción de recursos para una tarea.
#[derive(Debug, Clone, Copy)]
pub struct ResourcePrediction {
    pub task_id: TaskId,
    pub predicted_cpu_ticks: u64,
    pub predicted_mem_bytes: u64,
    pub confidence: f32, // 0.0 - 1.0
}

/// Estado del AI Scheduler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AISchedulerState {
    Disabled = 0,
    Monitoring = 1,
    Predicting = 2,
    Optimizing = 3,
}

/// AI Scheduler Predictivo.
pub struct AIScheduler {
    pub state: AtomicUsize,
    pub histories: [Option<ResourceHistory>; 256],
    pub history_count: AtomicUsize,
    pub predictions: [Option<ResourcePrediction>; 256],
    pub prediction_count: AtomicUsize,
    pub optimization_interval_us: u64,
    pub last_optimization_us: AtomicU64,
    pub cpu_threshold_high: u64,
    pub cpu_threshold_low: u64,
    pub mem_threshold_high: u64,
    pub mem_threshold_low: u64,
    pub initialized: AtomicU64,
}

// SpinLock para AIScheduler
mod spin_lock_ai {
    use core::sync::atomic::{AtomicUsize, Ordering};
    use core::cell::UnsafeCell;

    #[allow(dead_code)]
pub struct SpinLock<T> {
        lock: AtomicUsize,
        value: UnsafeCell<T>,
    }

    unsafe impl<T> Send for SpinLock<T> {}
    unsafe impl<T> Sync for SpinLock<T> {}

    #[allow(dead_code)]
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

    #[allow(dead_code)]
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

impl AIScheduler {
    pub const fn new() -> Self {
        Self {
            state: AtomicUsize::new(AISchedulerState::Disabled as usize),
            histories: [None; 256],
            history_count: AtomicUsize::new(0),
            predictions: [None; 256],
            prediction_count: AtomicUsize::new(0),
            optimization_interval_us: 100_000, // 100ms
            last_optimization_us: AtomicU64::new(0),
            cpu_threshold_high: 80, // 80%
            cpu_threshold_low: 20,  // 20%
            mem_threshold_high: 80, // 80%
            mem_threshold_low: 20,  // 20%
            initialized: AtomicU64::new(0),
        }
    }

    pub unsafe fn init(&mut self) {
        self.state.store(AISchedulerState::Monitoring as usize, Ordering::Release);
        self.initialized.store(1, Ordering::Release);
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::Acquire) != 0
    }

    pub fn state(&self) -> AISchedulerState {
        match self.state.load(Ordering::Acquire) {
            0 => AISchedulerState::Disabled,
            1 => AISchedulerState::Monitoring,
            2 => AISchedulerState::Predicting,
            3 => AISchedulerState::Optimizing,
            _ => AISchedulerState::Disabled,
        }
    }

    /// Registra una muestra de recursos para una tarea.
    pub fn record_sample(&mut self, task_id: TaskId, cpu_ticks: u64, mem_bytes: u64, timestamp_us: u64) {
        if !self.is_initialized() {
            return;
        }

        let _ = timestamp_us;

        // Buscar o crear historial para esta tarea
        let history_idx = self.find_or_create_history(task_id);

        if let Some(history) = self.histories[history_idx].as_mut() {
            let idx = history.sample_index % 64;
            history.samples[idx] = ResourceSample {
                timestamp_us,
                cpu_ticks,
                mem_bytes,
                task_id,
            };
            history.sample_index += 1;
            if history.sample_count < 64 {
                history.sample_count += 1;
            }
        }
    }

    /// Encuentra o crea un historial para una tarea.
    fn find_or_create_history(&mut self, task_id: TaskId) -> usize {
        // Buscar existente
        for i in 0..self.history_count.load(Ordering::Acquire) {
            if let Some(ref history) = self.histories[i] {
                if history.task_id == task_id {
                    return i;
                }
            }
        }

        // Crear nuevo
        let count = self.history_count.load(Ordering::Acquire);
        if count < 256 {
            self.histories[count] = Some(ResourceHistory {
                task_id,
                samples: [ResourceSample {
                    timestamp_us: 0,
                    cpu_ticks: 0,
                    mem_bytes: 0,
                    task_id,
                }; 64],
                sample_count: 0,
                sample_index: 0,
            });
            self.history_count.store(count + 1, Ordering::Release);
            count
        } else {
            0 // Lleno, devolver primero
        }
    }

    /// Predice los recursos necesarios para una tarea usando promedio móvil.
    pub fn predict_resources(&mut self, task_id: TaskId) -> Option<ResourcePrediction> {
        if !self.is_initialized() {
            return None;
        }

        // Buscar historial
        let history_idx = (0..self.history_count.load(Ordering::Acquire)).find(|i| {
            self.histories[*i].as_ref().map_or(false, |h| h.task_id == task_id)
        })?;

        let history = self.histories[history_idx].as_ref()?;
        if history.sample_count == 0 {
            return None;
        }

        // Calcular promedio móvil ponderado (más peso a muestras recientes)
        let mut cpu_sum: u64 = 0;
        let mut mem_sum: u64 = 0;
        let mut total_weight: u64 = 0;

        let count = history.sample_count.min(64);
        for i in 0..count {
            let idx = (history.sample_index + i) % 64;
            let sample = history.samples[idx];
            let weight = (i + 1) as u64; // Más peso a muestras recientes
            cpu_sum += sample.cpu_ticks * weight;
            mem_sum += sample.mem_bytes * weight;
            total_weight += weight;
        }

        let predicted_cpu = if total_weight > 0 {
            cpu_sum / total_weight
        } else {
            0
        };

        let predicted_mem = if total_weight > 0 {
            mem_sum / total_weight
        } else {
            0
        };

        // Confianza basada en número de muestras
        let confidence = if count >= 10 {
            0.9
        } else if count >= 5 {
            0.7
        } else if count >= 3 {
            0.5
        } else {
            0.3
        };

        Some(ResourcePrediction {
            task_id,
            predicted_cpu_ticks: predicted_cpu,
            predicted_mem_bytes: predicted_mem,
            confidence,
        })
    }

    /// Ejecuta una ciclo de optimización: predicir y ajustar prioridades.
    pub fn optimize(&mut self, scheduler: &mut Scheduler, current_time_us: u64) {
        if !self.is_initialized() {
            return;
        }

        let last = self.last_optimization_us.load(Ordering::Acquire);
        if current_time_us - last < self.optimization_interval_us {
            return;
        }

        self.state.store(AISchedulerState::Optimizing as usize, Ordering::Release);

        // Generar predicciones para todas las tareas con historial
        for i in 0..self.history_count.load(Ordering::Acquire) {
            if let Some(ref history) = self.histories[i] {
                if let Some(prediction) = self.predict_resources(history.task_id) {
                    // Almacenar predicción
                    let pred_count = self.prediction_count.load(Ordering::Acquire);
                    if pred_count < 256 {
                        self.predictions[pred_count] = Some(prediction);
                        self.prediction_count.store(pred_count + 1, Ordering::Release);
                    }

                    // Ajustar prioridad basada en predicción
                    self.adjust_priority(scheduler, &prediction);
                }
            }
        }

        self.last_optimization_us.store(current_time_us, Ordering::Release);
        self.state.store(AISchedulerState::Monitoring as usize, Ordering::Release);
    }

    /// Ajusta la prioridad de una tarea basada en su predicción de recursos.
    fn adjust_priority(&self, scheduler: &mut Scheduler, prediction: &ResourcePrediction) {
        if prediction.confidence < 0.5 {
            return; // No suficiente confianza
        }

        let task = match scheduler.get_task_mut(prediction.task_id) {
            Some(t) => t,
            None => return,
        };

        let current_priority = task.priority;
        let new_priority = if prediction.predicted_cpu_ticks > self.cpu_threshold_high {
            // Alta carga prevista: subir prioridad para dar más CPU
            match current_priority {
                TaskPriority::Idle => TaskPriority::Low,
                TaskPriority::Low => TaskPriority::Normal,
                TaskPriority::Normal => TaskPriority::High,
                TaskPriority::High => TaskPriority::Realtime,
                TaskPriority::Realtime => TaskPriority::Realtime,
            }
        } else if prediction.predicted_cpu_ticks < self.cpu_threshold_low {
            // Baja carga prevista: bajar prioridad
            match current_priority {
                TaskPriority::Realtime => TaskPriority::High,
                TaskPriority::High => TaskPriority::Normal,
                TaskPriority::Normal => TaskPriority::Low,
                TaskPriority::Low => TaskPriority::Idle,
                TaskPriority::Idle => TaskPriority::Idle,
            }
        } else {
            current_priority
        };

        if new_priority != current_priority {
            task.priority = new_priority;
        }
    }

    /// Configura los umbrales de CPU.
    pub fn set_cpu_thresholds(&mut self, high: u64, low: u64) {
        self.cpu_threshold_high = high;
        self.cpu_threshold_low = low;
    }

    /// Configura los umbrales de memoria.
    pub fn set_mem_thresholds(&mut self, high: u64, low: u64) {
        self.mem_threshold_high = high;
        self.mem_threshold_low = low;
    }

    /// Configura el intervalo de optimización.
    pub fn set_optimization_interval(&mut self, interval_us: u64) {
        self.optimization_interval_us = interval_us;
    }

    /// Devuelve el número de historias de recursos.
    pub fn history_count(&self) -> usize {
        self.history_count.load(Ordering::Acquire)
    }

    /// Devuelve el número de predicciones activas.
    pub fn prediction_count(&self) -> usize {
        self.prediction_count.load(Ordering::Acquire)
    }
}

/// Instancia global del AI Scheduler.
static mut AI_SCHEDULER: AIScheduler = AIScheduler::new();

/// Devuelve una referencia mutable al AI Scheduler global.
#[allow(static_mut_refs)]
pub fn global_ai_scheduler() -> &'static mut AIScheduler {
    unsafe { &mut AI_SCHEDULER }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ai_states() {
        assert_eq!(AISchedulerState::Disabled as u8, 0);
        assert_eq!(AISchedulerState::Monitoring as u8, 1);
        assert_eq!(AISchedulerState::Predicting as u8, 2);
        assert_eq!(AISchedulerState::Optimizing as u8, 3);
    }

    #[test]
    fn ai_scheduler_new() {
        let scheduler = AIScheduler::new();
        assert!(!scheduler.is_initialized());
        assert_eq!(scheduler.state(), AISchedulerState::Disabled);
        assert_eq!(scheduler.history_count(), 0);
        assert_eq!(scheduler.prediction_count(), 0);
    }

    #[test]
    fn ai_scheduler_init() {
        let mut scheduler = AIScheduler::new();
        unsafe { scheduler.init(); }
        assert!(scheduler.is_initialized());
        assert_eq!(scheduler.state(), AISchedulerState::Monitoring);
    }

    #[test]
    fn resource_sample_default() {
        let sample = ResourceSample {
            timestamp_us: 1000,
            cpu_ticks: 500,
            mem_bytes: 1024,
            task_id: 1,
        };
        assert_eq!(sample.task_id, 1);
        assert_eq!(sample.cpu_ticks, 500);
        assert_eq!(sample.mem_bytes, 1024);
    }

    #[test]
    fn resource_history_new() {
        let history = ResourceHistory {
            task_id: 1,
            samples: [ResourceSample {
                timestamp_us: 0,
                cpu_ticks: 0,
                mem_bytes: 0,
                task_id: 1,
            }; 64],
            sample_count: 0,
            sample_index: 0,
        };
        assert_eq!(history.task_id, 1);
        assert_eq!(history.sample_count, 0);
    }

    #[test]
    fn record_and_predict() {
        let mut scheduler = AIScheduler::new();
        unsafe { scheduler.init(); }

        // Registrar varias muestras con valores altos para evitar división entera = 0
        for i in 0..10 {
            scheduler.record_sample(
                1,
                1000 + i * 100, // cpu_ticks creciente
                10000 + i * 1000,
                i * 10000,
            );
        }

        // Predecir
        let prediction = scheduler.predict_resources(1);
        assert!(prediction.is_some());
        let pred = prediction.unwrap();
        assert_eq!(pred.task_id, 1);
        assert!(pred.predicted_cpu_ticks > 0);
        assert!(pred.predicted_mem_bytes > 0);
        assert!(pred.confidence > 0.0);
    }

    #[test]
    fn predict_no_history() {
        let mut scheduler = AIScheduler::new();
        unsafe { scheduler.init(); }
        let prediction = scheduler.predict_resources(999);
        assert!(prediction.is_none());
    }

    #[test]
    fn record_sample_not_initialized() {
        let mut scheduler = AIScheduler::new();
        // No llamar init()
        scheduler.record_sample(1, 100, 1000, 0); // No debe panic
    }

    #[test]
    fn resource_prediction_default() {
        let pred = ResourcePrediction {
            task_id: 1,
            predicted_cpu_ticks: 500,
            predicted_mem_bytes: 2048,
            confidence: 0.85,
        };
        assert_eq!(pred.task_id, 1);
        assert!((pred.confidence - 0.85).abs() < 0.001);
    }

    #[test]
    fn thresholds() {
        let mut scheduler = AIScheduler::new();
        unsafe { scheduler.init(); }
        scheduler.set_cpu_thresholds(90, 10);
        scheduler.set_mem_thresholds(85, 15);
        // Los umbrales se establecen correctamente (sin panic)
    }

    #[test]
    fn optimize_cycle() {
        let mut scheduler = AIScheduler::new();
        unsafe { scheduler.init(); }

        // Registrar muestras para una tarea
        for i in 0..10 {
            scheduler.record_sample(1, 2000 + i * 50, 20000 + i * 500, i * 5000);
        }

        let mut kernel_sched = super::Scheduler::new();
        unsafe { kernel_sched.init(); }

        // Crear una tarea para probar el ajuste
        let _pid = kernel_sched.create_process("test", 0).unwrap();
        let _tid = kernel_sched.create_thread(1, 0x1000, 0x20000, 0x1000, TaskPriority::Normal).unwrap();

        // Ejecutar optimización
        scheduler.optimize(&mut kernel_sched, 50000);

        // Verificar que se generaron predicciones
        assert!(scheduler.prediction_count() > 0);
    }
}
