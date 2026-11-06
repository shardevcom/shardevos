//! Driver GPU/NPU — Aceleración Gráfica e IA de shardevos.
//!
//! Proporciona:
//! - Inicialización de GPU/NPU
//! - Comandos de renderizado acelerado
//! - Ejecución de modelos de IA vía NPU
//! - Gestión de memoria de video (VRAM)

use super::{UiError, UiResult};
use core::sync::atomic::{AtomicU64, Ordering};

// SpinLock para GPUNPUDriver
mod spin_lock_gpu {
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

/// Información de la GPU detectada.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct GpuInfo {
    pub vendor_id: u16,
    pub device_id: u16,
    pub vram_size: usize,
    pub max_resolution_width: u32,
    pub max_resolution_height: u32,
    pub supports_compute: bool,
    pub supports_npu: bool,
}

/// Información de la NPU detectada.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct NpuInfo {
    pub present: bool,
    pub cores: u32,
    pub max_frequency_mhz: u32,
    pub supports_int8: bool,
    pub supports_fp16: bool,
}

/// Comandos de GPU soportados.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum GpuCommand {
    Clear = 0,
    Blit = 1,
    DrawRectangle = 2,
    DrawLine = 3,
    ComputeDispatch = 4,
    NpuInference = 5,
}

/// Estado del driver GPU/NPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum GpuState {
    Uninitialized = 0,
    Initialized = 1,
    Active = 2,
    Error = 3,
}

/// Driver GPU/NPU.
pub struct GpuNpuDriver {
    gpu_info: spin_lock_gpu::SpinLock<Option<GpuInfo>>,
    npu_info: spin_lock_gpu::SpinLock<Option<NpuInfo>>,
    state: AtomicU64,
    vram_allocator: spin_lock_gpu::SpinLock<VramAllocator>,
    command_queue: spin_lock_gpu::SpinLock<[Option<GpuCommandEntry>; 256]>,
    command_count: AtomicU64,
    initialized: AtomicU64,
}

/// Entrada de la cola de comandos GPU.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct GpuCommandEntry {
    pub command: GpuCommand,
    pub param1: u64,
    pub param2: u64,
    pub param3: u64,
}

/// Alocador de VRAM simple (bump allocator).
#[derive(Debug, Clone, Copy)]
pub struct VramAllocator {
    base_addr: usize,
    size: usize,
    offset: usize,
}

impl VramAllocator {
    pub const fn new() -> Self {
        Self { base_addr: 0, size: 0, offset: 0 }
    }

    pub fn init(&mut self, base_addr: usize, size: usize) {
        self.base_addr = base_addr;
        self.size = size;
        self.offset = 0;
    }

    pub fn allocate(&mut self, size: usize, align: usize) -> Result<usize, UiError> {
        let aligned = (self.offset + align - 1) & !(align - 1);
        if aligned + size > self.size {
            return Err(UiError::OutOfMemory);
        }
        let addr = self.base_addr + aligned;
        self.offset = aligned + size;
        Ok(addr)
    }

    pub fn remaining(&self) -> usize {
        self.size.saturating_sub(self.offset)
    }
}

impl GpuNpuDriver {
    pub const fn new() -> Self {
        Self {
            gpu_info: spin_lock_gpu::SpinLock::new(None),
            npu_info: spin_lock_gpu::SpinLock::new(None),
            state: AtomicU64::new(GpuState::Uninitialized as u64),
            vram_allocator: spin_lock_gpu::SpinLock::new(VramAllocator::new()),
            command_queue: spin_lock_gpu::SpinLock::new([None; 256]),
            command_count: AtomicU64::new(0),
            initialized: AtomicU64::new(0),
        }
    }

    pub unsafe fn init(&self) {
        self.initialized.store(1, Ordering::Release);
        self.state.store(GpuState::Initialized as u64, Ordering::Release);
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::Acquire) != 0
    }

    pub fn state(&self) -> GpuState {
        match self.state.load(Ordering::Acquire) {
            0 => GpuState::Uninitialized,
            1 => GpuState::Initialized,
            2 => GpuState::Active,
            3 => GpuState::Error,
            _ => GpuState::Uninitialized,
        }
    }

    /// Detecta y configura la GPU.
    pub fn detect_gpu(&self) -> UiResult<GpuInfo> {
        if !self.is_initialized() {
            return Err(UiError::NotInitialized);
        }

        // Simular detección de GPU (en producción: leer PCI, registers, etc.)
        let info = GpuInfo {
            vendor_id: 0x10DE, // NVIDIA placeholder
            device_id: 0x1C82,
            vram_size: 4 * 1024 * 1024 * 1024, // 4 GB
            max_resolution_width: 7680,
            max_resolution_height: 4320,
            supports_compute: true,
            supports_npu: true,
        };

        {
            let mut gi = self.gpu_info.lock();
            *gi = Some(info.clone());
        }

        // Inicializar alocador de VRAM
        {
            let mut va = self.vram_allocator.lock();
            va.init(info.vram_size, 4096);
        }

        self.state.store(GpuState::Active as u64, Ordering::Release);
        Ok(info)
    }

    /// Detecta y configura la NPU.
    pub fn detect_npu(&self) -> UiResult<NpuInfo> {
        if !self.is_initialized() {
            return Err(UiError::NotInitialized);
        }

        let info = NpuInfo {
            present: true,
            cores: 8,
            max_frequency_mhz: 1500,
            supports_int8: true,
            supports_fp16: true,
        };

        {
            let mut ni = self.npu_info.lock();
            *ni = Some(info.clone());
        }

        Ok(info)
    }

    /// Asigna memoria en VRAM.
    pub fn allocate_vram(&self, size: usize, align: usize) -> UiResult<usize> {
        if !self.is_initialized() {
            return Err(UiError::NotInitialized);
        }

        let mut va = self.vram_allocator.lock();
        va.allocate(size, align)
    }

    /// Envía un comando a la GPU.
    pub fn submit_command(&self, cmd: GpuCommandEntry) -> UiResult<u64> {
        if !self.is_initialized() {
            return Err(UiError::NotInitialized);
        }

        let mut queue = self.command_queue.lock();
        let id = self.command_count.fetch_add(1, Ordering::Release);

        // Encontrar slot libre
        let slot = queue.iter().position(|c| c.is_none()).ok_or(UiError::OutOfMemory)?;
        queue[slot] = Some(cmd);

        Ok(id)
    }

    /// Ejecuta todos los comandos pendientes (flush).
    pub fn flush_commands(&self) -> UiResult<u64> {
        if !self.is_initialized() {
            return Err(UiError::NotInitialized);
        }

        let mut executed = 0u64;
        {
            let mut queue = self.command_queue.lock();
            for entry in queue.iter_mut() {
                if entry.is_some() {
                    let _cmd = entry.take().unwrap();
                    executed += 1;
                }
            }
        }

        Ok(executed)
    }

    /// Ejecuta inferencia de IA vía NPU.
    pub fn npu_inference(&self, _model_addr: usize, _input_addr: usize, input_size: usize, _output_addr: usize, _output_size: usize) -> UiResult<u64> {
        if !self.is_initialized() {
            return Err(UiError::NotInitialized);
        }

        let npu = self.npu_info.lock();
        if npu.is_none() || !npu.as_ref().unwrap().present {
            return Err(UiError::DisplayNotAvailable);
        }

        // Simular inferencia NPU
        let latency_us = (input_size as u64 / 1024).max(1);

        Ok(latency_us)
    }

    /// Devuelve información de la GPU.
    pub fn gpu_info(&self) -> UiResult<GpuInfo> {
        let gi = self.gpu_info.lock();
        gi.ok_or(UiError::NotInitialized)
    }

    /// Devuelve información de la NPU.
    pub fn npu_info(&self) -> UiResult<NpuInfo> {
        let ni = self.npu_info.lock();
        ni.ok_or(UiError::NotInitialized)
    }

    /// Devuelve la VRAM restante.
    pub fn vram_remaining(&self) -> usize {
        self.vram_allocator.lock().remaining()
    }
}

/// Instancia global del driver GPU/NPU.
static GPU_NPU_DRIVER: GpuNpuDriver = GpuNpuDriver::new();

/// Devuelve una referencia al driver GPU/NPU global.
pub fn global_gpu_npu_driver() -> &'static GpuNpuDriver {
    &GPU_NPU_DRIVER
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_states() {
        assert_eq!(GpuState::Uninitialized as u8, 0);
        assert_eq!(GpuState::Active as u8, 2);
        assert_eq!(GpuState::Error as u8, 3);
    }

    #[test]
    fn gpu_commands() {
        assert_eq!(GpuCommand::Clear as u8, 0);
        assert_eq!(GpuCommand::Blit as u8, 1);
        assert_eq!(GpuCommand::NpuInference as u8, 5);
    }

    #[test]
    fn gpu_driver_new() {
        let driver = GpuNpuDriver::new();
        assert!(!driver.is_initialized());
        assert_eq!(driver.state(), GpuState::Uninitialized);
    }

    #[test]
    fn detect_gpu() {
        let driver = GpuNpuDriver::new();
        unsafe { driver.init(); }
        let info = driver.detect_gpu().unwrap();
        assert_eq!(driver.state(), GpuState::Active);
        assert!(info.supports_compute);
        assert!(info.supports_npu);
        assert_eq!(info.vram_size, 4 * 1024 * 1024 * 1024);
    }

    #[test]
    fn detect_npu() {
        let driver = GpuNpuDriver::new();
        unsafe { driver.init(); }
        let info = driver.detect_npu().unwrap();
        assert!(info.present);
        assert_eq!(info.cores, 8);
        assert!(info.supports_int8);
    }

    #[test]
    fn allocate_vram() {
        let driver = GpuNpuDriver::new();
        unsafe { driver.init(); }
        driver.detect_gpu().unwrap();

        let addr1 = driver.allocate_vram(1024, 4096).unwrap();
        let addr2 = driver.allocate_vram(2048, 4096).unwrap();
        assert!(addr2 > addr1);
        assert!(driver.vram_remaining() < 4 * 1024 * 1024 * 1024);
    }

    #[test]
    fn submit_and_flush_commands() {
        let driver = GpuNpuDriver::new();
        unsafe { driver.init(); }

        let cmd = GpuCommandEntry {
            command: GpuCommand::Clear,
            param1: 0,
            param2: 0,
            param3: 0,
        };
        let id = driver.submit_command(cmd).unwrap();
        assert_eq!(id, 0);

        let executed = driver.flush_commands().unwrap();
        assert_eq!(executed, 1);
    }

    #[test]
    fn npu_inference() {
        let driver = GpuNpuDriver::new();
        unsafe { driver.init(); }
        driver.detect_npu().unwrap();

        let latency = driver.npu_inference(0x1000, 0x2000, 1024, 0x3000, 512).unwrap();
        assert!(latency > 0);
    }

    #[test]
    fn vram_allocator() {
        let mut va = VramAllocator::new();
        va.init(0x8000_0000, 1024 * 1024);

        let a1 = va.allocate(256, 64).unwrap();
        let a2 = va.allocate(512, 64).unwrap();
        assert!(a2 > a1);
        assert_eq!(va.remaining(), 1024 * 1024 - 256 - 512);
    }

    #[test]
    fn not_initialized_errors() {
        let driver = GpuNpuDriver::new();
        assert_eq!(driver.detect_gpu(), Err(UiError::NotInitialized));
        assert_eq!(driver.allocate_vram(1024, 64), Err(UiError::NotInitialized));
    }
}
