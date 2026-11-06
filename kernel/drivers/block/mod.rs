//! Block Device Driver — Controlador de dispositivo de bloque de shardevos.
//!
//! Proporciona:
//! - Lectura y escritura de bloques de dispositivos de almacenamiento
//! - Gestión de colas de solicitudes de E/S
//! - Soporte para disco virtual (simulado para QEMU)
//! - Integración con el VFS a través de operaciones de bloque


extern crate alloc;

use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use core::ptr::NonNull;

/// Identificador único de dispositivo de bloque.
pub type BlockDeviceId = u64;

/// Tamaño de bloque en bytes (estándar 512 bytes para dispositivos de bloque).
const BLOCK_SIZE: usize = 512;

/// Número máximo de dispositivos de bloque soportados.
const MAX_BLOCK_DEVICES: usize = 8;

/// Número máximo de solicitudes en la cola.
const MAX_BLOCK_REQUESTS: usize = 64;

/// Tipos de dispositivos de bloque soportados.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum BlockDeviceType {
    Unknown = 0,
    IDE = 1,
    SATA = 2,
    SCSI = 3,
    NVMe = 4,
    VirtIO = 5,
    RamDisk = 6,
    Virtual = 7,
}

/// Estado de un dispositivo de bloque.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum BlockDeviceState {
    Uninitialized = 0,
    Initialized = 1,
    Ready = 2,
    Error = 3,
    Offline = 4,
}

/// Información de un dispositivo de bloque.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct BlockDeviceInfo {
    pub id: BlockDeviceId,
    pub device_type: BlockDeviceType,
    pub sector_size: u32,
    pub total_sectors: u64,
    pub total_bytes: u64,
    pub read_only: bool,
    pub removable: bool,
}

/// Dirección de bloque (LBA - Logical Block Addressing).
pub type Lba = u64;

/// Tipo de operación de bloque.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum BlockOpType {
    Read = 0,
    Write = 1,
    Flush = 2,
}

/// Solicitud de operación de bloque.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct BlockRequest {
    pub id: u64,
    pub device_id: BlockDeviceId,
    pub op_type: BlockOpType,
    pub lba: Lba,
    pub sector_count: u32,
    pub buffer: *mut u8,
    pub callback: Option<extern "C" fn(u64, BlockOpResult)>,
    pub user_data: *mut u8,
}

/// Resultado de una operación de bloque.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum BlockOpResult {
    Success = 0,
    Error = 1,
    DeviceNotFound = 2,
    InvalidLba = 3,
    InvalidBuffer = 4,
    DeviceBusy = 5,
    DeviceError = 6,
}

/// Estadísticas de un dispositivo de bloque.
#[derive(Debug, Clone, Copy)]
pub struct BlockDeviceStats {
    pub device_id: BlockDeviceId,
    pub read_operations: u64,
    pub write_operations: u64,
    pub read_sectors: u64,
    pub write_sectors: u64,
    pub read_bytes: u64,
    pub write_bytes: u64,
    pub error_count: u64,
    pub last_error: BlockOpResult,
}

/// Estado de la cola de solicitudes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum RequestStatus {
    Pending = 0,
    Processing = 1,
    Completed = 2,
    Failed = 3,
}

/// Errores del driver de bloque.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockError {
    NotInitialized,
    DeviceNotFound,
    InvalidLba,
    InvalidBuffer,
    ReadOnly,
    OutOfMemory,
    DeviceBusy,
    DeviceError,
}

/// Resultado de operaciones del driver de bloque.
pub type BlockResult<T> = Result<T, BlockError>;

/// Nodo de la cola de solicitudes de bloque.
#[allow(dead_code)]
struct BlockRequestNode {
    request: BlockRequest,
    status: RequestStatus,
    next: Option<NonNull<BlockRequestNode>>,
}

/// Driver de dispositivo de bloque.
pub struct BlockDeviceDriver {
    devices: spin_lock_block::SpinLock<[Option<BlockDeviceInfo>; MAX_BLOCK_DEVICES]>,
    device_count: AtomicUsize,
    request_queue: spin_lock_block::SpinLock<[Option<BlockRequest>; MAX_BLOCK_REQUESTS]>,
    request_count: AtomicUsize,
    next_request_id: AtomicU64,
    stats: spin_lock_block::SpinLock<[Option<BlockDeviceStats>; MAX_BLOCK_DEVICES]>,
    initialized: AtomicU64,
}

// SpinLock para el driver de bloque
mod spin_lock_block {
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

impl BlockDeviceDriver {
    pub const fn new() -> Self {
        Self {
            devices: spin_lock_block::SpinLock::new([None; MAX_BLOCK_DEVICES]),
            device_count: AtomicUsize::new(0),
            request_queue: spin_lock_block::SpinLock::new([None; MAX_BLOCK_REQUESTS]),
            request_count: AtomicUsize::new(0),
            next_request_id: AtomicU64::new(1),
            stats: spin_lock_block::SpinLock::new([None; MAX_BLOCK_DEVICES]),
            initialized: AtomicU64::new(0),
        }
    }

    pub unsafe fn init(&self) {
        self.initialized.store(1, Ordering::Release);
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::Acquire) != 0
    }

    /// Registra un nuevo dispositivo de bloque.
    pub fn register_device(&self, device_type: BlockDeviceType, sector_size: u32, total_sectors: u64, read_only: bool, removable: bool) -> BlockResult<BlockDeviceId> {
        if !self.is_initialized() {
            return Err(BlockError::NotInitialized);
        }

        let mut devices = self.devices.lock();
        let id = (devices.iter().filter(|d| d.is_some()).count() + 1) as BlockDeviceId;

        if id > MAX_BLOCK_DEVICES as BlockDeviceId {
            return Err(BlockError::OutOfMemory);
        }

        let total_bytes = total_sectors * sector_size as u64;

        // Encontrar slot libre
        for slot in devices.iter_mut() {
            if slot.is_none() {
                *slot = Some(BlockDeviceInfo {
                    id,
                    device_type,
                    sector_size,
                    total_sectors,
                    total_bytes,
                    read_only,
                    removable,
                });
                self.device_count.fetch_add(1, Ordering::Release);

                // Inicializar estadísticas
                let mut stats = self.stats.lock();
                stats[id as usize - 1] = Some(BlockDeviceStats {
                    device_id: id,
                    read_operations: 0,
                    write_operations: 0,
                    read_sectors: 0,
                    write_sectors: 0,
                    read_bytes: 0,
                    write_bytes: 0,
                    error_count: 0,
                    last_error: BlockOpResult::Success,
                });
                drop(stats);

                return Ok(id);
            }
        }

        Err(BlockError::OutOfMemory)
    }

    /// Desregistra un dispositivo de bloque.
    pub fn unregister_device(&self, device_id: BlockDeviceId) -> BlockResult<()> {
        if !self.is_initialized() {
            return Err(BlockError::NotInitialized);
        }

        let mut devices = self.devices.lock();
        let idx = devices.iter().position(|d| d.as_ref().map_or(false, |d| d.id == device_id));
        match idx {
            Some(i) => {
                devices[i] = None;
                self.device_count.fetch_sub(1, Ordering::Release);

                // Limpiar estadísticas
                let mut stats = self.stats.lock();
                stats[i] = None;
                drop(stats);

                Ok(())
            }
            None => Err(BlockError::DeviceNotFound),
        }
    }

    /// Obtiene información de un dispositivo de bloque.
    pub fn get_device_info(&self, device_id: BlockDeviceId) -> BlockResult<BlockDeviceInfo> {
        if !self.is_initialized() {
            return Err(BlockError::NotInitialized);
        }

        let devices = self.devices.lock();
        for d in devices.iter() {
            if let Some(info) = d {
                if info.id == device_id {
                    return Ok(*info);
                }
            }
        }
        Err(BlockError::DeviceNotFound)
    }

    /// Envía una solicitud de lectura al dispositivo de bloque.
    pub fn read_blocks(&self, device_id: BlockDeviceId, lba: Lba, sector_count: u32, buffer: *mut u8) -> BlockResult<BlockOpResult> {
        if !self.is_initialized() {
            return Err(BlockError::NotInitialized);
        }

        // Verificar que el dispositivo existe
        let devices = self.devices.lock();
        let device = devices.iter().find_map(|d| d.as_ref().filter(|d| d.id == device_id));
        let device = match device {
            Some(d) => *d,
            None => return Err(BlockError::DeviceNotFound),
        };
        drop(devices);

        // Validar LBA
        if lba + sector_count as u64 > device.total_sectors {
            return Err(BlockError::InvalidLba);
        }

        // Validar buffer
        if buffer.is_null() {
            return Err(BlockError::InvalidBuffer);
        }

        // Crear solicitud
        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let request = BlockRequest {
            id: request_id,
            device_id,
            op_type: BlockOpType::Read,
            lba,
            sector_count,
            buffer,
            callback: None,
            user_data: core::ptr::null_mut(),
        };

        // Agregar a la cola
        {
            let mut queue = self.request_queue.lock();
            for slot in queue.iter_mut() {
                if slot.is_none() {
                    *slot = Some(request);
                    self.request_count.fetch_add(1, Ordering::Release);
                    break;
                }
            }
        }

        // Simular lectura (en producción: acceder al dispositivo real)
        for i in 0..(sector_count as usize * BLOCK_SIZE) {
            unsafe {
                buffer.add(i).write(0);
            }
        }

        // Actualizar estadísticas
        self.update_stats(device_id, true, sector_count as u64, BlockOpResult::Success);

        Ok(BlockOpResult::Success)
    }

    /// Envía una solicitud de escritura al dispositivo de bloque.
    pub fn write_blocks(&self, device_id: BlockDeviceId, lba: Lba, sector_count: u32, buffer: *const u8) -> BlockResult<BlockOpResult> {
        if !self.is_initialized() {
            return Err(BlockError::NotInitialized);
        }

        // Verificar que el dispositivo existe y no es solo lectura
        let devices = self.devices.lock();
        let device = devices.iter().find_map(|d| d.as_ref().filter(|d| d.id == device_id));
        let device = match device {
            Some(d) => *d,
            None => return Err(BlockError::DeviceNotFound),
        };
        drop(devices);

        if device.read_only {
            return Err(BlockError::ReadOnly);
        }

        // Validar LBA
        if lba + sector_count as u64 > device.total_sectors {
            return Err(BlockError::InvalidLba);
        }

        // Validar buffer
        if buffer.is_null() {
            return Err(BlockError::InvalidBuffer);
        }

        // Crear solicitud
        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let request = BlockRequest {
            id: request_id,
            device_id,
            op_type: BlockOpType::Write,
            lba,
            sector_count,
            buffer: buffer as *mut u8,
            callback: None,
            user_data: core::ptr::null_mut(),
        };

        // Agregar a la cola
        {
            let mut queue = self.request_queue.lock();
            for slot in queue.iter_mut() {
                if slot.is_none() {
                    *slot = Some(request);
                    self.request_count.fetch_add(1, Ordering::Release);
                    break;
                }
            }
        }

        // Simular escritura
        // En producción: escribir al dispositivo real

        // Actualizar estadísticas
        self.update_stats(device_id, false, sector_count as u64, BlockOpResult::Success);

        Ok(BlockOpResult::Success)
    }

    /// Actualiza las estadísticas de un dispositivo.
    fn update_stats(&self, device_id: BlockDeviceId, is_read: bool, sectors: u64, result: BlockOpResult) {
        let mut stats = self.stats.lock();
        if let Some(stat) = stats.iter_mut().find_map(|s| s.as_mut().filter(|s| s.device_id == device_id)) {
            if is_read {
                stat.read_operations += 1;
                stat.read_sectors += sectors;
                stat.read_bytes += sectors * BLOCK_SIZE as u64;
            } else {
                stat.write_operations += 1;
                stat.write_sectors += sectors;
                stat.write_bytes += sectors * BLOCK_SIZE as u64;
            }

            if result != BlockOpResult::Success {
                stat.error_count += 1;
                stat.last_error = result;
            }
        }
    }

    /// Ejecuta todas las solicitudes pendientes en la cola.
    pub fn flush_queue(&self) -> BlockResult<u64> {
        if !self.is_initialized() {
            return Err(BlockError::NotInitialized);
        }

        let mut executed = 0u64;
        {
            let mut queue = self.request_queue.lock();
            for slot in queue.iter_mut() {
                if slot.is_some() {
                    slot.take();
                    executed += 1;
                }
            }
            // Decrementar el contador de solicitudes pendientes
            self.request_count.fetch_sub(executed as usize, Ordering::Release);
        }

        Ok(executed)
    }

    /// Devuelve el número de dispositivos registrados.
    pub fn device_count(&self) -> usize {
        self.device_count.load(Ordering::Acquire)
    }

    /// Devuelve el número de solicitudes pendientes.
    pub fn pending_requests(&self) -> usize {
        self.request_count.load(Ordering::Acquire)
    }

    /// Obtiene las estadísticas de un dispositivo.
    pub fn get_stats(&self, device_id: BlockDeviceId) -> BlockResult<BlockDeviceStats> {
        if !self.is_initialized() {
            return Err(BlockError::NotInitialized);
        }

        let stats = self.stats.lock();
        for s in stats.iter() {
            if let Some(st) = s {
                if st.device_id == device_id {
                    return Ok(*st);
                }
            }
        }
        Err(BlockError::DeviceNotFound)
    }
}

/// Instancia global del driver de bloque.
static mut BLOCK_DRIVER: BlockDeviceDriver = BlockDeviceDriver::new();

/// Devuelve una referencia mutable al driver de bloque global.
#[allow(static_mut_refs)]
pub fn global_block_driver() -> &'static mut BlockDeviceDriver {
    unsafe { &mut BLOCK_DRIVER }
}

/// Tiempo actual en microsegundos.
#[allow(dead_code)]
fn time_us() -> u64 {
    static mut TIME_COUNTER: u64 = 0;
    unsafe {
        TIME_COUNTER += 1000;
        TIME_COUNTER
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_device_types() {
        assert_eq!(BlockDeviceType::Unknown as u8, 0);
        assert_eq!(BlockDeviceType::IDE as u8, 1);
        assert_eq!(BlockDeviceType::SATA as u8, 2);
        assert_eq!(BlockDeviceType::SCSI as u8, 3);
        assert_eq!(BlockDeviceType::NVMe as u8, 4);
        assert_eq!(BlockDeviceType::VirtIO as u8, 5);
        assert_eq!(BlockDeviceType::RamDisk as u8, 6);
        assert_eq!(BlockDeviceType::Virtual as u8, 7);
    }

    #[test]
    fn block_device_states() {
        assert_eq!(BlockDeviceState::Uninitialized as u8, 0);
        assert_eq!(BlockDeviceState::Initialized as u8, 1);
        assert_eq!(BlockDeviceState::Ready as u8, 2);
        assert_eq!(BlockDeviceState::Error as u8, 3);
        assert_eq!(BlockDeviceState::Offline as u8, 4);
    }

    #[test]
    fn block_op_types() {
        assert_eq!(BlockOpType::Read as u8, 0);
        assert_eq!(BlockOpType::Write as u8, 1);
        assert_eq!(BlockOpType::Flush as u8, 2);
    }

    #[test]
    fn block_op_results() {
        assert_eq!(BlockOpResult::Success as u8, 0);
        assert_eq!(BlockOpResult::Error as u8, 1);
        assert_eq!(BlockOpResult::DeviceNotFound as u8, 2);
        assert_eq!(BlockOpResult::InvalidLba as u8, 3);
        assert_eq!(BlockOpResult::InvalidBuffer as u8, 4);
        assert_eq!(BlockOpResult::DeviceBusy as u8, 5);
        assert_eq!(BlockOpResult::DeviceError as u8, 6);
    }

    #[test]
    fn block_request_status() {
        assert_eq!(RequestStatus::Pending as u8, 0);
        assert_eq!(RequestStatus::Processing as u8, 1);
        assert_eq!(RequestStatus::Completed as u8, 2);
        assert_eq!(RequestStatus::Failed as u8, 3);
    }

    #[test]
    fn block_errors() {
        assert_eq!(BlockError::NotInitialized as isize, 0);
        assert_eq!(BlockError::DeviceNotFound as isize, 1);
        assert_eq!(BlockError::InvalidLba as isize, 2);
        assert_eq!(BlockError::InvalidBuffer as isize, 3);
        assert_eq!(BlockError::ReadOnly as isize, 4);
        assert_eq!(BlockError::OutOfMemory as isize, 5);
        assert_eq!(BlockError::DeviceBusy as isize, 6);
        assert_eq!(BlockError::DeviceError as isize, 7);
    }

    #[test]
    fn driver_new() {
        let driver = BlockDeviceDriver::new();
        assert!(!driver.is_initialized());
        assert_eq!(driver.device_count(), 0);
        assert_eq!(driver.pending_requests(), 0);
    }

    #[test]
    fn driver_init() {
        let driver = BlockDeviceDriver::new();
        unsafe { driver.init(); }
        assert!(driver.is_initialized());
    }

    #[test]
    fn register_device() {
        let driver = BlockDeviceDriver::new();
        unsafe { driver.init(); }

        let id = driver.register_device(
            BlockDeviceType::VirtIO,
            512,
            1024 * 1024, // 512 MB
            false,
            false,
        ).unwrap();

        assert_eq!(id, 1);
        assert_eq!(driver.device_count(), 1);

        let info = driver.get_device_info(id).unwrap();
        assert_eq!(info.device_type, BlockDeviceType::VirtIO);
        assert_eq!(info.sector_size, 512);
        assert_eq!(info.total_sectors, 1024 * 1024);
        assert_eq!(info.total_bytes, 1024 * 1024 * 512);
        assert!(!info.read_only);
    }

    #[test]
    fn register_readonly_device() {
        let driver = BlockDeviceDriver::new();
        unsafe { driver.init(); }

        let id = driver.register_device(
            BlockDeviceType::IDE,
            512,
            100 * 1024 * 1024 / 512,
            true,
            true,
        ).unwrap();

        let info = driver.get_device_info(id).unwrap();
        assert!(info.read_only);
        assert!(info.removable);
    }

    #[test]
    fn register_multiple_devices() {
        let driver = BlockDeviceDriver::new();
        unsafe { driver.init(); }

        driver.register_device(BlockDeviceType::SATA, 512, 1000, false, false).unwrap();
        driver.register_device(BlockDeviceType::NVMe, 4096, 500, false, false).unwrap();
        driver.register_device(BlockDeviceType::SCSI, 512, 2000, false, false).unwrap();

        assert_eq!(driver.device_count(), 3);
    }

    #[test]
    fn read_write_blocks() {
        let driver = BlockDeviceDriver::new();
        unsafe { driver.init(); }

        let id = driver.register_device(
            BlockDeviceType::VirtIO,
            512,
            1024,
            false,
            false,
        ).unwrap();

        let mut write_buf = [0xAAu8; 512];
        let mut read_buf = [0u8; 512];

        // Escribir un bloque
        let result = driver.write_blocks(id, 0, 1, write_buf.as_ptr());
        assert_eq!(result, Ok(BlockOpResult::Success));

        // Leer el bloque
        let result = driver.read_blocks(id, 0, 1, read_buf.as_mut_ptr());
        assert_eq!(result, Ok(BlockOpResult::Success));

        // Verificar que las operaciones no fallen
        assert_eq!(result, Ok(BlockOpResult::Success));
    }

    #[test]
    fn read_invalid_lba() {
        let driver = BlockDeviceDriver::new();
        unsafe { driver.init(); }

        let id = driver.register_device(
            BlockDeviceType::VirtIO,
            512,
            100,
            false,
            false,
        ).unwrap();

        let mut buf = [0u8; 512];
        // LBA fuera de rango
        let result = driver.read_blocks(id, 200, 1, buf.as_mut_ptr());
        assert_eq!(result, Err(BlockError::InvalidLba));
    }

    #[test]
    fn write_readonly_device() {
        let driver = BlockDeviceDriver::new();
        unsafe { driver.init(); }

        let id = driver.register_device(
            BlockDeviceType::IDE,
            512,
            100,
            true,
            false,
        ).unwrap();

        let mut buf = [0u8; 512];
        let result = driver.write_blocks(id, 0, 1, buf.as_ptr());
        assert_eq!(result, Err(BlockError::ReadOnly));
    }

    #[test]
    fn device_not_found() {
        let driver = BlockDeviceDriver::new();
        unsafe { driver.init(); }

        let mut buf = [0u8; 512];
        let result = driver.read_blocks(999, 0, 1, buf.as_mut_ptr());
        assert_eq!(result, Err(BlockError::DeviceNotFound));

        let result = driver.get_device_info(999);
        assert_eq!(result, Err(BlockError::DeviceNotFound));
    }

    #[test]
    fn unregister_device() {
        let driver = BlockDeviceDriver::new();
        unsafe { driver.init(); }

        let id = driver.register_device(
            BlockDeviceType::VirtIO,
            512,
            100,
            false,
            false,
        ).unwrap();

        assert_eq!(driver.device_count(), 1);

        driver.unregister_device(id).unwrap();
        assert_eq!(driver.device_count(), 0);

        // Intentar acceder al dispositivo eliminado
        let result = driver.get_device_info(id);
        assert_eq!(result, Err(BlockError::DeviceNotFound));
    }

    #[test]
    fn flush_queue() {
        let driver = BlockDeviceDriver::new();
        unsafe { driver.init(); }

        let id = driver.register_device(
            BlockDeviceType::VirtIO,
            512,
            100,
            false,
            false,
        ).unwrap();

        let mut buf = [0u8; 512];
        driver.read_blocks(id, 0, 1, buf.as_mut_ptr()).unwrap();
        driver.read_blocks(id, 1, 1, buf.as_mut_ptr()).unwrap();

        assert_eq!(driver.pending_requests(), 2);

        let executed = driver.flush_queue().unwrap();
        assert_eq!(executed, 2);
        assert_eq!(driver.pending_requests(), 0);
    }

    #[test]
    fn get_stats() {
        let driver = BlockDeviceDriver::new();
        unsafe { driver.init(); }

        let id = driver.register_device(
            BlockDeviceType::VirtIO,
            512,
            100,
            false,
            false,
        ).unwrap();

        let mut buf = [0u8; 512];
        driver.read_blocks(id, 0, 1, buf.as_mut_ptr()).unwrap();
        driver.write_blocks(id, 1, 1, buf.as_ptr()).unwrap();

        let stats = driver.get_stats(id).unwrap();
        assert_eq!(stats.device_id, id);
        assert_eq!(stats.read_operations, 1);
        assert_eq!(stats.write_operations, 1);
        assert_eq!(stats.read_sectors, 1);
        assert_eq!(stats.write_sectors, 1);
    }

    #[test]
    fn not_initialized() {
        let driver = BlockDeviceDriver::new();
        assert_eq!(driver.register_device(BlockDeviceType::VirtIO, 512, 100, false, false), Err(BlockError::NotInitialized));
        assert_eq!(driver.device_count(), 0);
    }
}
