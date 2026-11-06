//! Scheduler — Planificador de tareas del kernel shardevos.
//!
//! Proporciona:
//! - Estructuras de Task, Process y Thread
//! - Estados de tarea (Running, Ready, Blocked, Zombie, etc.)
//! - Planificador round-robin básico
//! - Creación y gestión de tareas
//! - Contexto de ejecución para cambio de tareas


use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use core::ptr::NonNull;

/// Identificador único de tarea.
pub type TaskId = u64;

/// Identificador único de proceso.
pub type ProcessId = u64;

/// Estados de una tarea (task).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TaskState {
    New = 0,
    Ready = 1,
    Running = 2,
    Blocked = 3,
    Zombie = 4,
    Terminated = 5,
}

/// Prioridad de una tarea.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum TaskPriority {
    Idle = 0,
    Low = 1,
    Normal = 2,
    High = 3,
    Realtime = 4,
}

/// Contexto de CPU de una tarea.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct TaskContext {
    pub rip: u64,
    pub rsp: u64,
    pub rflags: u64,
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub cs: u64,
    pub ss: u64,
    pub ds: u64,
    pub es: u64,
    pub fs: u64,
    pub gs: u64,
    pub cr3: u64,
}

impl TaskContext {
    pub fn new() -> Self {
        Self {
            rip: 0, rsp: 0, rflags: 0x00000002,
            rax: 0, rbx: 0, rcx: 0, rdx: 0,
            rsi: 0, rdi: 0, rbp: 0,
            r8: 0, r9: 0, r10: 0, r11: 0,
            r12: 0, r13: 0, r14: 0, r15: 0,
            cs: 0x08, ss: 0x10, ds: 0x10, es: 0x10, fs: 0x10, gs: 0x10,
            cr3: 0,
        }
    }
}

/// Estadísticas de una tarea.
#[derive(Debug, Clone, Copy)]
pub struct TaskStats {
    pub ticks_executed: u64,
    pub context_switches: u64,
    pub total_runtime_us: u64,
    pub last_run_us: u64,
}

/// Hilo de ejecución.
pub struct Thread {
    pub id: TaskId,
    pub state: AtomicUsize,
    pub priority: TaskPriority,
    pub context: TaskContext,
    pub stack_base: usize,
    pub stack_size: usize,
    pub stats: TaskStats,
    pub process_id: ProcessId,
    pub next: Option<NonNull<Thread>>,
    pub prev: Option<NonNull<Thread>>,
}

impl Thread {
    pub fn new(id: TaskId, pid: ProcessId, priority: TaskPriority) -> Self {
        Self {
            id,
            state: AtomicUsize::new(TaskState::New as usize),
            priority,
            context: TaskContext::new(),
            stack_base: 0,
            stack_size: 0,
            stats: TaskStats {
                ticks_executed: 0,
                context_switches: 0,
                total_runtime_us: 0,
                last_run_us: 0,
            },
            process_id: pid,
            next: None,
            prev: None,
        }
    }

    pub fn state(&self) -> TaskState {
        match self.state.load(Ordering::Acquire) {
            0 => TaskState::New,
            1 => TaskState::Ready,
            2 => TaskState::Running,
            3 => TaskState::Blocked,
            4 => TaskState::Zombie,
            5 => TaskState::Terminated,
            _ => TaskState::New,
        }
    }

    pub fn set_state(&self, state: TaskState) {
        self.state.store(state as usize, Ordering::Release);
    }
}

/// Proceso.
pub struct Process {
    pub id: ProcessId,
    pub name: [u8; 32],
    pub state: AtomicUsize,
    pub priority: TaskPriority,
    pub cr3: usize,
    pub threads: spin_lock_proc::SpinLock<[Option<NonNull<Thread>>; 16]>,
    pub thread_count: AtomicUsize,
    pub parent_id: ProcessId,
    pub created_at_us: u64,
}

// SpinLock para Process
mod spin_lock_proc {
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

impl Process {
    pub fn new(id: ProcessId, name: &str, parent_id: ProcessId) -> Self {
        let mut name_arr = [0u8; 32];
        let bytes = name.as_bytes();
        let copy_len = bytes.len().min(32);
        name_arr[..copy_len].copy_from_slice(&bytes[..copy_len]);

        Self {
            id,
            name: name_arr,
            state: AtomicUsize::new(TaskState::New as usize),
            priority: TaskPriority::Normal,
            cr3: 0,
            threads: spin_lock_proc::SpinLock::new([None; 16]),
            thread_count: AtomicUsize::new(0),
            parent_id,
            created_at_us: time_us(),
        }
    }

    pub fn add_thread(&self, _thread: NonNull<Thread>) -> bool {
        if self.thread_count.load(Ordering::Acquire) >= 16 {
            return false;
        }
        self.thread_count.fetch_add(1, Ordering::Release);
        true
    }

    pub fn thread_count(&self) -> usize {
        self.thread_count.load(Ordering::Acquire)
    }
}

/// Cola de tareas listas para ejecutar.
pub struct ReadyQueue {
    counts: [usize; 5],
}

impl ReadyQueue {
    pub const fn new() -> Self {
        Self { counts: [0; 5] }
    }

    pub fn enqueue(&mut self, _task_id: TaskId, priority: TaskPriority) {
        let prio_idx = priority as usize;
        if prio_idx < 5 {
            self.counts[prio_idx] += 1;
        }
    }

    pub fn dequeue(&mut self) -> Option<TaskId> {
        for prio_idx in (0..5).rev() {
            if self.counts[prio_idx] > 0 {
                self.counts[prio_idx] -= 1;
                return Some(0); // Placeholder
            }
        }
        None
    }

    pub fn is_empty(&self) -> bool {
        self.counts.iter().all(|c| *c == 0)
    }
}

/// Planificador de tareas del kernel.
pub struct Scheduler {
    ready_queue: spin_lock_sched::SpinLock<ReadyQueue>,
    process_count: AtomicUsize,
    task_count: AtomicUsize,
    current_task: AtomicU64,
    next_task_id: AtomicU64,
    next_process_id: AtomicU64,
    initialized: AtomicU64,
    total_ticks: AtomicU64,
}

// SpinLock para Scheduler
mod spin_lock_sched {
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

impl Scheduler {
    pub fn new() -> Self {
        Self {
            ready_queue: spin_lock_sched::SpinLock::new(ReadyQueue::new()),
            process_count: AtomicUsize::new(0),
            task_count: AtomicUsize::new(0),
            current_task: AtomicU64::new(0),
            next_task_id: AtomicU64::new(1),
            next_process_id: AtomicU64::new(1),
            initialized: AtomicU64::new(0),
            total_ticks: AtomicU64::new(0),
        }
    }

    pub unsafe fn init(&mut self) {
        self.initialized.store(1, Ordering::Release);
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::Acquire) != 0
    }

    pub fn create_process(&mut self, _name: &str, parent_id: ProcessId) -> Result<ProcessId, SchedulerError> {
        if !self.is_initialized() {
            return Err(SchedulerError::NotInitialized);
        }
        let _ = parent_id;
        let pid = self.next_process_id.fetch_add(1, Ordering::Relaxed);
        self.process_count.fetch_add(1, Ordering::Release);
        Ok(pid)
    }

    #[allow(static_mut_refs)]
pub fn create_thread(&mut self, pid: ProcessId, entry_point: usize, stack: usize, stack_size: usize, priority: TaskPriority) -> Result<TaskId, SchedulerError> {
        if !self.is_initialized() {
            return Err(SchedulerError::NotInitialized);
        }
        let tid = self.next_task_id.fetch_add(1, Ordering::Relaxed);
        let slot = unsafe { THREAD_ALLOCATED.fetch_add(1, Ordering::Relaxed) };
        if slot >= 256 {
            return Err(SchedulerError::OutOfMemory);
        }
        let mut context = TaskContext::new();
        context.rip = entry_point as u64;
        context.rsp = (stack + stack_size) as u64;
        context.cr3 = 0;
        let mut thread = Thread::new(tid, pid, priority);
        thread.context = context;
        thread.stack_base = stack;
        thread.stack_size = stack_size;
        thread.set_state(TaskState::Ready);

        // Almacenar en el almacenamiento estático de hilos
        unsafe {
            THREAD_STORAGE[slot] = Some(thread);
        }

        self.task_count.fetch_add(1, Ordering::Release);
        {
            let mut rq = self.ready_queue.lock();
            rq.enqueue(tid, priority);
        }
        Ok(tid)
    }

    pub fn current_task_id(&self) -> TaskId {
        self.current_task.load(Ordering::Acquire)
    }

    pub fn set_current_task(&self, tid: TaskId) {
        self.current_task.store(tid, Ordering::Release);
    }

    #[allow(static_mut_refs)]
pub fn get_task(&self, tid: TaskId) -> Option<&Thread> {
        let storage = unsafe { &THREAD_STORAGE };
        for slot in storage.iter() {
            if let Some(ref thread) = slot {
                if thread.id == tid {
                    return Some(thread);
                }
            }
        }
        None
    }

    #[allow(static_mut_refs)]
pub fn get_task_mut(&mut self, tid: TaskId) -> Option<&mut Thread> {
        let storage = unsafe { &mut THREAD_STORAGE };
        for slot in storage.iter_mut() {
            if let Some(ref mut thread) = slot {
                if thread.id == tid {
                    return Some(thread);
                }
            }
        }
        None
    }

    pub fn set_task_state(&mut self, tid: TaskId, state: TaskState) -> Result<(), SchedulerError> {
        match self.get_task_mut(tid) {
            Some(thread) => {
                thread.set_state(state);
                Ok(())
            }
            None => Err(SchedulerError::TaskNotFound),
        }
    }

    pub fn block_current(&mut self) -> Result<(), SchedulerError> {
        if !self.is_initialized() {
            return Err(SchedulerError::NotInitialized);
        }
        let current = self.current_task_id();
        self.set_task_state(current, TaskState::Blocked)
    }

    pub fn unblock_task(&mut self, tid: TaskId) -> Result<(), SchedulerError> {
        self.set_task_state(tid, TaskState::Ready)
    }

    pub fn terminate_task(&mut self, tid: TaskId) -> Result<(), SchedulerError> {
        self.set_task_state(tid, TaskState::Zombie)
    }

    pub fn tick(&mut self) {
        if !self.is_initialized() {
            return;
        }
        self.total_ticks.fetch_add(1, Ordering::Release);
        let current = self.current_task_id();
        if current == 0 {
            return;
        }
        if let Some(thread) = self.get_task_mut(current) {
            thread.stats.ticks_executed += 1;
        }
    }

    pub fn schedule(&mut self) -> Option<TaskId> {
        if !self.is_initialized() {
            return None;
        }
        let mut rq = self.ready_queue.lock();
        let next_tid = rq.dequeue();
        drop(rq);
        if let Some(tid) = next_tid {
            let current = self.current_task_id();
            if current != 0 {
                if let Some(thread) = self.get_task_mut(current) {
                    thread.stats.context_switches += 1;
                    thread.set_state(TaskState::Ready);
                }
            }
            self.set_task_state(tid, TaskState::Running).ok();
            self.set_current_task(tid);
            if let Some(thread) = self.get_task_mut(tid) {
                thread.stats.last_run_us = time_us();
                thread.stats.context_switches += 1;
            }
            Some(tid)
        } else {
            None
        }
    }

    pub fn task_count(&self) -> usize {
        self.task_count.load(Ordering::Acquire)
    }

    pub fn total_ticks(&self) -> u64 {
        self.total_ticks.load(Ordering::Acquire)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerError {
    NotInitialized,
    TaskNotFound,
    OutOfMemory,
    InvalidArgument,
}

static mut SCHEDULER: Scheduler = Scheduler {
    ready_queue: spin_lock_sched::SpinLock::new(ReadyQueue::new()),
    process_count: AtomicUsize::new(0),
    task_count: AtomicUsize::new(0),
    current_task: AtomicU64::new(0),
    next_task_id: AtomicU64::new(1),
    next_process_id: AtomicU64::new(1),
    initialized: AtomicU64::new(0),
    total_ticks: AtomicU64::new(0),
};

/// Almacenamiento estático de hilos (evita necesidad de alloc/Box).
static mut THREAD_STORAGE: [Option<Thread>; 256] = unsafe { core::mem::zeroed() };

/// Contador de asignación de hilos.
static mut THREAD_ALLOCATED: AtomicUsize = AtomicUsize::new(0);

#[allow(static_mut_refs)]
pub fn global_scheduler() -> &'static mut Scheduler {
    unsafe { &mut SCHEDULER }
}

fn time_us() -> u64 {
    static mut TIME_COUNTER: u64 = 0;
    unsafe {
        TIME_COUNTER += 1000;
        TIME_COUNTER
    }
}

pub mod ai_scheduler;
#[path = "../self_healing/mod.rs"]
pub mod self_healing;

#[cfg(test)]mod tests {
    use super::*;

    #[test]
    fn task_states() {
        assert_eq!(TaskState::New as u8, 0);
        assert_eq!(TaskState::Ready as u8, 1);
        assert_eq!(TaskState::Running as u8, 2);
        assert_eq!(TaskState::Blocked as u8, 3);
        assert_eq!(TaskState::Zombie as u8, 4);
        assert_eq!(TaskState::Terminated as u8, 5);
    }

    #[test]
    fn task_priorities() {
        assert!(TaskPriority::Realtime > TaskPriority::High);
        assert!(TaskPriority::High > TaskPriority::Normal);
        assert!(TaskPriority::Normal > TaskPriority::Low);
        assert!(TaskPriority::Low > TaskPriority::Idle);
    }

    #[test]
    fn task_context_new() {
        let ctx = TaskContext::new();
        assert_eq!(ctx.rip, 0);
        assert_eq!(ctx.rsp, 0);
        assert_eq!(ctx.rflags, 0x00000002);
        assert_eq!(ctx.cs, 0x08);
        assert_eq!(ctx.ss, 0x10);
    }

    #[test]
    fn thread_new() {
        let thread = Thread::new(1, 1, TaskPriority::Normal);
        assert_eq!(thread.id, 1);
        assert_eq!(thread.process_id, 1);
        assert_eq!(thread.state(), TaskState::New);
        assert_eq!(thread.priority, TaskPriority::Normal);
    }

    #[test]
    fn thread_set_state() {
        let thread = Thread::new(1, 1, TaskPriority::Normal);
        thread.set_state(TaskState::Running);
        assert_eq!(thread.state(), TaskState::Running);
        thread.set_state(TaskState::Blocked);
        assert_eq!(thread.state(), TaskState::Blocked);
    }

    #[test]
    fn process_new() {
        let proc = Process::new(1, "test_proc", 0);
        assert_eq!(proc.id, 1);
        assert_eq!(proc.parent_id, 0);
        assert_eq!(proc.thread_count(), 0);
        assert_eq!(proc.cr3, 0);
    }

    #[test]
    fn scheduler_new() {
        let sched = Scheduler::new();
        assert!(!sched.is_initialized());
        assert_eq!(sched.task_count(), 0);
        assert_eq!(sched.current_task_id(), 0);
    }

    #[test]
    fn scheduler_init() {
        let mut sched = Scheduler::new();
        unsafe { sched.init(); }
        assert!(sched.is_initialized());
    }

    #[test]
    fn scheduler_create_process() {
        let mut sched = Scheduler::new();
        unsafe { sched.init(); }
        let pid = sched.create_process("test", 0).unwrap();
        assert_eq!(pid, 1);
        let pid2 = sched.create_process("test2", pid).unwrap();
        assert_eq!(pid2, 2);
    }

    #[test]
    fn scheduler_create_thread() {
        let mut sched = Scheduler::new();
        unsafe { sched.init(); }
        let pid = sched.create_process("test", 0).unwrap();
        let tid = sched.create_thread(pid, 0x1000, 0x20000, 0x1000, TaskPriority::Normal).unwrap();
        assert_eq!(tid, 1);
        assert_eq!(sched.task_count(), 1);
    }

    #[test]
    fn scheduler_block_unblock() {
        let mut sched = Scheduler::new();
        unsafe { sched.init(); }
        let pid = sched.create_process("test", 0).unwrap();
        let tid = sched.create_thread(pid, 0x1000, 0x20000, 0x1000, TaskPriority::Normal).unwrap();
        sched.set_current_task(tid);
        assert!(sched.block_current().is_ok());
        let thread = sched.get_task(tid).unwrap();
        assert_eq!(thread.state(), TaskState::Blocked);
        assert!(sched.unblock_task(tid).is_ok());
        let thread = sched.get_task(tid).unwrap();
        assert_eq!(thread.state(), TaskState::Ready);
    }

    #[test]
    fn scheduler_terminate() {
        let mut sched = Scheduler::new();
        unsafe { sched.init(); }
        let pid = sched.create_process("test", 0).unwrap();
        let tid = sched.create_thread(pid, 0x1000, 0x20000, 0x1000, TaskPriority::Normal).unwrap();
        assert!(sched.terminate_task(tid).is_ok());
        let thread = sched.get_task(tid).unwrap();
        assert_eq!(thread.state(), TaskState::Zombie);
    }

    #[test]
    fn scheduler_tick() {
        let mut sched = Scheduler::new();
        unsafe { sched.init(); }
        let pid = sched.create_process("test", 0).unwrap();
        let tid = sched.create_thread(pid, 0x1000, 0x20000, 0x1000, TaskPriority::Normal).unwrap();
        sched.set_current_task(tid);
        sched.tick();
        assert_eq!(sched.total_ticks(), 1);
        let thread = sched.get_task_mut(tid).unwrap();
        assert_eq!(thread.stats.ticks_executed, 1);
    }

    #[test]
    fn scheduler_error_not_initialized() {
        let mut sched = Scheduler::new();
        assert_eq!(sched.create_process("test", 0), Err(SchedulerError::NotInitialized));
        assert_eq!(sched.create_thread(1, 0x1000, 0x2000, 0x1000, TaskPriority::Normal), Err(SchedulerError::NotInitialized));
        assert_eq!(sched.block_current(), Err(SchedulerError::NotInitialized));
    }

    #[test]
    fn scheduler_task_not_found() {
        let mut sched = Scheduler::new();
        unsafe { sched.init(); }
        assert_eq!(sched.set_task_state(999, TaskState::Ready), Err(SchedulerError::TaskNotFound));
    }
}
