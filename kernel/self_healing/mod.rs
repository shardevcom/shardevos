//! Self-Healing — Auto-reparación del kernel sin Kernel Panic.
//!
//! Proporciona:
//! - Monitoreo autónomo de violaciones de acceso y llamadas inválidas
//! - Detección de fallos de drivers y hilos problemáticos
//! - Aislamiento y reinicio transparente de componentes fallidos
//! - Registro de eventos de autocuración para análisis posterior


use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use super::{TaskId, TaskState};

/// Tipo de evento de autocuración.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum HealEventType {
    PageFault = 0,
    GeneralProtectionFault = 1,
    InvalidOpcode = 2,
    DoubleFault = 3,
    InvalidSyscall = 4,
    DriverHang = 5,
    ThreadCrash = 6,
    MemoryCorruption = 7,
}

/// Acción de autocuración tomada.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum HealAction {
    None = 0,
    IsolateThread = 1,
    RestartThread = 2,
    IsolateDriver = 3,
    RestartDriver = 4,
    KillProcess = 5,
    ResetSubsystem = 6,
}

/// Registro de un evento de autocuración.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct HealEvent {
    pub event_type: HealEventType,
    pub action: HealAction,
    pub task_id: TaskId,
    pub timestamp_us: u64,
    pub error_code: u64,
    pub address: usize,
}

/// Estado del subsistema de autocuración.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SelfHealingState {
    Disabled = 0,
    Monitoring = 1,
    Healing = 2,
}

/// Contador de fallos por tarea.
#[derive(Debug, Clone, Copy)]
pub struct FailureStats {
    pub task_id: TaskId,
    pub page_faults: u64,
    pub general_protection_faults: u64,
    pub invalid_opcodes: u64,
    pub invalid_syscalls: u64,
    pub total_failures: u64,
    pub last_failure_us: u64,
}

/// Módulo de autocuración del kernel.
pub struct SelfHealing {
    pub state: AtomicUsize,
    pub events: spin_lock_sh::SpinLock<[Option<HealEvent>; 256]>,
    pub event_count: AtomicUsize,
    pub failure_stats: spin_lock_sh::SpinLock<[Option<FailureStats>; 256]>,
    pub stats_count: AtomicUsize,
    pub max_failures_before_isolate: u64,
    pub max_failures_before_restart: u64,
    pub initialized: AtomicU64,
    pub total_healings: AtomicU64,
}

// SpinLock para SelfHealing
mod spin_lock_sh {
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

impl SelfHealing {
    pub const fn new() -> Self {
        Self {
            state: AtomicUsize::new(SelfHealingState::Disabled as usize),
            events: spin_lock_sh::SpinLock::new([None; 256]),
            event_count: AtomicUsize::new(0),
            failure_stats: spin_lock_sh::SpinLock::new([None; 256]),
            stats_count: AtomicUsize::new(0),
            max_failures_before_isolate: 3,
            max_failures_before_restart: 5,
            initialized: AtomicU64::new(0),
            total_healings: AtomicU64::new(0),
        }
    }

    pub unsafe fn init(&mut self) {
        self.state.store(SelfHealingState::Monitoring as usize, Ordering::Release);
        self.initialized.store(1, Ordering::Release);
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::Acquire) != 0
    }

    pub fn state(&self) -> SelfHealingState {
        match self.state.load(Ordering::Acquire) {
            0 => SelfHealingState::Disabled,
            1 => SelfHealingState::Monitoring,
            2 => SelfHealingState::Healing,
            _ => SelfHealingState::Disabled,
        }
    }

    /// Registra un evento de fallo y decide la acción de autocuración.
    pub fn record_failure(
        &mut self,
        event_type: HealEventType,
        task_id: TaskId,
        error_code: u64,
        address: usize,
        _description: &str,
    ) -> HealAction {
        if !self.is_initialized() {
            return HealAction::None;
        }

        self.state.store(SelfHealingState::Healing as usize, Ordering::Release);

        // Actualizar estadísticas de fallos para la tarea
        let stats_idx = self.find_or_create_stats(task_id);
        if let Some(stats) = self.failure_stats.lock()[stats_idx].as_mut() {
            match event_type {
                HealEventType::PageFault => stats.page_faults += 1,
                HealEventType::GeneralProtectionFault => stats.general_protection_faults += 1,
                HealEventType::InvalidOpcode => stats.invalid_opcodes += 1,
                HealEventType::InvalidSyscall => stats.invalid_syscalls += 1,
                _ => {}
            }
            stats.total_failures += 1;
            stats.last_failure_us = time_us();
        }

        // Decidir acción basada en el número de fallos
        let action = self.decide_action(task_id, stats_idx);

        // Registrar el evento
        let event_idx = self.event_count.load(Ordering::Acquire);
        if event_idx < 256 {
            self.events.lock()[event_idx] = Some(HealEvent {
                event_type,
                action,
                task_id,
                timestamp_us: time_us(),
                error_code,
                address,
            });
            self.event_count.store(event_idx + 1, Ordering::Release);
        }

        // Incrementar contador de autocuraciones
        self.total_healings.fetch_add(1, Ordering::Release);

        // Ejecutar la acción
        self.execute_action(action, task_id);

        self.state.store(SelfHealingState::Monitoring as usize, Ordering::Release);
        action
    }

    /// Encuentra o crea estadísticas de fallos para una tarea.
    fn find_or_create_stats(&mut self, task_id: TaskId) -> usize {
        // Buscar existente
        let stats = self.failure_stats.lock();
        for i in 0..self.stats_count.load(Ordering::Acquire) {
            if let Some(ref s) = stats[i] {
                if s.task_id == task_id {
                    return i;
                }
            }
        }
        drop(stats);

        // Crear nuevo
        let count = self.stats_count.load(Ordering::Acquire);
        if count < 256 {
            self.failure_stats.lock()[count] = Some(FailureStats {
                task_id,
                page_faults: 0,
                general_protection_faults: 0,
                invalid_opcodes: 0,
                invalid_syscalls: 0,
                total_failures: 0,
                last_failure_us: 0,
            });
            self.stats_count.store(count + 1, Ordering::Release);
            count
        } else {
            0
        }
    }

    /// Decide la acción de autocuración basada en el historial de fallos.
    fn decide_action(&self, _task_id: TaskId, stats_idx: usize) -> HealAction {
        let stats_guard = self.failure_stats.lock();
        let stats = stats_guard[stats_idx].as_ref();
        match stats {
            Some(s) => {
                if s.total_failures >= self.max_failures_before_restart {
                    HealAction::RestartThread
                } else if s.total_failures >= self.max_failures_before_isolate {
                    HealAction::IsolateThread
                } else {
                    HealAction::None
                }
            }
            None => HealAction::None,
        }
    }

    /// Ejecuta la acción de autocuración decidida.
    fn execute_action(&self, action: HealAction, task_id: TaskId) {
        match action {
            HealAction::IsolateThread | HealAction::IsolateDriver => {
                if let Some(sched) = get_scheduler_mut() {
                    let _ = sched.set_task_state(task_id, TaskState::Blocked);
                }
            }
            HealAction::RestartThread | HealAction::RestartDriver => {
                if let Some(sched) = get_scheduler_mut() {
                    let _ = sched.set_task_state(task_id, TaskState::Ready);
                }
            }
            HealAction::KillProcess => {
                if let Some(sched) = get_scheduler_mut() {
                    let _ = sched.set_task_state(task_id, TaskState::Zombie);
                }
            }
            HealAction::ResetSubsystem => {
                // Placeholder para lógica de reinicio de subsistema
            }
            HealAction::None => {}
        }
    }

    /// Maneja una violación de acceso (page fault).
    pub fn handle_page_fault(&mut self, task_id: TaskId, error_code: u64, address: usize) -> HealAction {
        self.record_failure(
            HealEventType::PageFault,
            task_id,
            error_code,
            address,
            "Page fault: acceso a memoria inválida",
        )
    }

    /// Maneja una falla de protección general.
    pub fn handle_general_protection_fault(&mut self, task_id: TaskId, error_code: u64, address: usize) -> HealAction {
        self.record_failure(
            HealEventType::GeneralProtectionFault,
            task_id,
            error_code,
            address,
            "General protection fault: operación privilegiada inválida",
        )
    }

    /// Maneja un opcode inválido.
    pub fn handle_invalid_opcode(&mut self, task_id: TaskId, error_code: u64, address: usize) -> HealAction {
        self.record_failure(
            HealEventType::InvalidOpcode,
            task_id,
            error_code,
            address,
            "Invalid opcode: instrucción no reconocida",
        )
    }

    /// Maneja una doble falla (muy crítica).
    pub fn handle_double_fault(&mut self, task_id: TaskId, error_code: u64, address: usize) -> HealAction {
        self.record_failure(
            HealEventType::DoubleFault,
            task_id,
            error_code,
            address,
            "Double fault: falla durante manejo de otra falla",
        )
    }

    /// Maneja un syscall inválido.
    pub fn handle_invalid_syscall(&mut self, task_id: TaskId, syscall_nr: u64) -> HealAction {
        self.record_failure(
            HealEventType::InvalidSyscall,
            task_id,
            syscall_nr,
            0,
            "Invalid syscall: número de syscall no implementado",
        )
    }

    /// Maneja un cuelgue de driver.
    pub fn handle_driver_hang(&mut self, task_id: TaskId) -> HealAction {
        self.record_failure(
            HealEventType::DriverHang,
            task_id,
            0,
            0,
            "Driver hang: no respondió en el tiempo esperado",
        )
    }

    /// Devuelve todos los eventos de autocuración registrados.
    pub fn events(&self) -> usize {
        self.event_count.load(Ordering::Acquire)
    }

    /// Devuelve el total de autocuraciones realizadas.
    pub fn total_healings(&self) -> u64 {
        self.total_healings.load(Ordering::Acquire)
    }

    /// Configura los umbrales de fallos.
    pub fn set_failure_thresholds(&mut self, isolate: u64, restart: u64) {
        self.max_failures_before_isolate = isolate;
        self.max_failures_before_restart = restart;
    }

    /// Devuelve las estadísticas de fallos para una tarea.
    pub fn get_failure_stats(&self, task_id: TaskId) -> Option<FailureStats> {
        let stats = self.failure_stats.lock();
        for i in 0..self.stats_count.load(Ordering::Acquire) {
            if let Some(ref s) = stats[i] {
                if s.task_id == task_id {
                    return Some(*s);
                }
            }
        }
        None
    }

    // ============================================================================
    // F6-04: Auto-reparación explícita de hilos y drivers
    // ============================================================================

    /// Repara un hilo específico: lo aísla temporalmente y lo vuelve a marcar como listo.
    pub fn repair_thread(&mut self, task_id: TaskId) -> HealAction {
        if !self.is_initialized() {
            return HealAction::None;
        }

        // Primero aislar, luego reiniciar
        if let Some(sched) = get_scheduler_mut() {
            let _ = sched.set_task_state(task_id, TaskState::Blocked);
            let _ = sched.set_task_state(task_id, TaskState::Ready);
        }

        self.record_failure(
            HealEventType::ThreadCrash,
            task_id,
            0,
            0,
            "Thread repair: hilo reiniciado por auto-reparación",
        )
    }

    /// Repara un driver específico: lo aísla y lo reinicia.
    pub fn repair_driver(&mut self, task_id: TaskId) -> HealAction {
        if !self.is_initialized() {
            return HealAction::None;
        }

        if let Some(sched) = get_scheduler_mut() {
            let _ = sched.set_task_state(task_id, TaskState::Blocked);
            let _ = sched.set_task_state(task_id, TaskState::Ready);
        }

        self.record_failure(
            HealEventType::DriverHang,
            task_id,
            0,
            0,
            "Driver repair: driver reiniciado por auto-reparación",
        )
    }

    // ============================================================================
    // F6-05: Captura de fallos de drivers
    // ============================================================================

    /// Captura un fallo específico de driver y registra detalles para diagnóstico.
    pub fn capture_driver_fault(
        &mut self,
        driver_id: TaskId,
        fault_type: HealEventType,
        error_code: u64,
        fault_address: usize,
        driver_name: &str,
    ) -> HealAction {
        if !self.is_initialized() {
            return HealAction::None;
        }

        // Registrar el fallo del driver
        let action = self.record_failure(
            fault_type,
            driver_id,
            error_code,
            fault_address,
            driver_name,
        );

        // Los fallos de driver críticos se isolan inmediatamente
        if matches!(fault_type, HealEventType::DoubleFault | HealEventType::MemoryCorruption) {
            if let Some(sched) = get_scheduler_mut() {
                let _ = sched.set_task_state(driver_id, TaskState::Blocked);
            }
        }

        action
    }
}

/// Instancia global de autocuración.
static mut SELF_HEALING: SelfHealing = SelfHealing::new();

/// Devuelve una referencia mutable al SelfHealing global.
#[allow(static_mut_refs)]
pub fn global_self_healing() -> &'static mut SelfHealing {
    unsafe { &mut SELF_HEALING }
}

/// Tiempo actual en microsegundos.
fn time_us() -> u64 {
    static mut TIME_COUNTER: u64 = 0;
    unsafe {
        TIME_COUNTER += 1000;
        TIME_COUNTER
    }
}

/// Obtiene una referencia mutable al scheduler global.
fn get_scheduler_mut() -> Option<&'static mut super::Scheduler> {
    Some(super::global_scheduler())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heal_states() {
        assert_eq!(SelfHealingState::Disabled as u8, 0);
        assert_eq!(SelfHealingState::Monitoring as u8, 1);
        assert_eq!(SelfHealingState::Healing as u8, 2);
    }

    #[test]
    fn heal_event_types() {
        assert_eq!(HealEventType::PageFault as u8, 0);
        assert_eq!(HealEventType::GeneralProtectionFault as u8, 1);
        assert_eq!(HealEventType::InvalidOpcode as u8, 2);
        assert_eq!(HealEventType::InvalidSyscall as u8, 4);
    }

    #[test]
    fn heal_actions() {
        assert_eq!(HealAction::None as u8, 0);
        assert_eq!(HealAction::IsolateThread as u8, 1);
        assert_eq!(HealAction::RestartThread as u8, 2);
        assert_eq!(HealAction::RestartDriver as u8, 4);
    }

    #[test]
    fn self_healing_new() {
        let sh = SelfHealing::new();
        assert!(!sh.is_initialized());
        assert_eq!(sh.state(), SelfHealingState::Disabled);
        assert_eq!(sh.events(), 0);
        assert_eq!(sh.total_healings(), 0);
    }

    #[test]
    fn self_healing_init() {
        let mut sh = SelfHealing::new();
        unsafe { sh.init(); }
        assert!(sh.is_initialized());
        assert_eq!(sh.state(), SelfHealingState::Monitoring);
    }

    #[test]
    fn record_page_fault() {
        let mut sh = SelfHealing::new();
        unsafe { sh.init(); }

        let action = sh.handle_page_fault(1, 0x6, 0xDEADBEEF);
        assert_eq!(action, HealAction::None);
        assert_eq!(sh.total_healings(), 1);
        assert_eq!(sh.events(), 1);
    }

    #[test]
    fn record_multiple_failures_isolate() {
        let mut sh = SelfHealing::new();
        unsafe { sh.init(); }
        sh.set_failure_thresholds(3, 5);

        for _ in 0..3 {
            sh.handle_page_fault(1, 0x6, 0xDEADBEEF);
        }

        let action = sh.handle_page_fault(1, 0x6, 0xDEADBEEF);
        assert_eq!(action, HealAction::IsolateThread);
    }

    #[test]
    fn record_multiple_failures_restart() {
        let mut sh = SelfHealing::new();
        unsafe { sh.init(); }
        sh.set_failure_thresholds(3, 5);

        for _ in 0..5 {
            sh.handle_page_fault(1, 0x6, 0xDEADBEEF);
        }

        let action = sh.handle_page_fault(1, 0x6, 0xDEADBEEF);
        assert_eq!(action, HealAction::RestartThread);
    }

    #[test]
    fn handle_general_protection_fault() {
        let mut sh = SelfHealing::new();
        unsafe { sh.init(); }
        let action = sh.handle_general_protection_fault(2, 0x0, 0xCAFEBABE);
        assert_eq!(action, HealAction::None);
        assert_eq!(sh.total_healings(), 1);
    }

    #[test]
    fn handle_invalid_opcode() {
        let mut sh = SelfHealing::new();
        unsafe { sh.init(); }
        let action = sh.handle_invalid_opcode(3, 0x0, 0x1234);
        assert_eq!(action, HealAction::None);
    }

    #[test]
    fn handle_double_fault() {
        let mut sh = SelfHealing::new();
        unsafe { sh.init(); }
        let action = sh.handle_double_fault(4, 0x0, 0x5678);
        assert_eq!(action, HealAction::None);
    }

    #[test]
    fn handle_invalid_syscall() {
        let mut sh = SelfHealing::new();
        unsafe { sh.init(); }
        let action = sh.handle_invalid_syscall(5, 999);
        assert_eq!(action, HealAction::None);
    }

    #[test]
    fn handle_driver_hang() {
        let mut sh = SelfHealing::new();
        unsafe { sh.init(); }
        let action = sh.handle_driver_hang(6);
        assert_eq!(action, HealAction::None);
    }

    #[test]
    fn failure_stats() {
        let mut sh = SelfHealing::new();
        unsafe { sh.init(); }
        sh.handle_page_fault(1, 0x6, 0x1000);
        sh.handle_page_fault(1, 0x6, 0x2000);

        let stats = sh.get_failure_stats(1).unwrap();
        assert_eq!(stats.task_id, 1);
        assert_eq!(stats.page_faults, 2);
        assert_eq!(stats.total_failures, 2);
    }

    #[test]
    fn failure_stats_not_found() {
        let mut sh = SelfHealing::new();
        unsafe { sh.init(); }
        let stats = sh.get_failure_stats(999);
        assert!(stats.is_none());
    }

    #[test]
    fn not_initialized() {
        let mut sh = SelfHealing::new();
        let action = sh.handle_page_fault(1, 0x6, 0x1000);
        assert_eq!(action, HealAction::None);
        assert_eq!(sh.total_healings(), 0);
    }

    #[test]
    fn set_thresholds() {
        let mut sh = SelfHealing::new();
        unsafe { sh.init(); }
        sh.set_failure_thresholds(10, 20);
    }
}
