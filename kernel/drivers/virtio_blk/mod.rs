//! VirtIO-Blk — Controlador real de dispositivo de bloque VirtIO.
//!
//! Implementa el protocolo VirtIO 1.0 sobre MMIO para acceder a discos
//! virtuales en QEMU/VirtualBox. Esto permite lectura y escritura reales
//! de bloques desde el dispositivo de bloque emulado.
//!
//! Referencia: https://docs.oasis-open.org/virtio/virtio/v1.1/virtio-v1.1.pdf

use crate::mm::{frame_allocator::{alloc_frame, dealloc_frame}, frame_to_addr, PAGE_SIZE};
use crate::pci::{find_virtio_devices, pci_bar_is_mmio, pci_bar_mmio_addr};
use core::arch::asm;
use core::ptr::{read_volatile, write_volatile, NonNull};
use core::sync::atomic::{AtomicU64, Ordering};
use alloc::vec::Vec;

// ============================================================================
// VirtIO-MMIO registros (offset desde la base MMIO)
// ============================================================================

const VIRTIO_MMIO_MAGIC_VALUE: usize = 0x00;
const VIRTIO_MMIO_VERSION: usize = 0x04;
const VIRTIO_MMIO_DEVICE_ID: usize = 0x08;
const VIRTIO_MMIO_VENDOR_ID: usize = 0x0C;
const VIRTIO_MMIO_DEVICE_FEATURES: usize = 0x10;
const VIRTIO_MMIO_DRIVER_FEATURES: usize = 0x14;
const VIRTIO_MMIO_QUEUE_SEL: usize = 0x14;
const VIRTIO_MMIO_QUEUE_NUM_MAX: usize = 0x18;
const VIRTIO_MMIO_QUEUE_NUM: usize = 0x1C;
const VIRTIO_MMIO_QUEUE_PFN: usize = 0x20;
const VIRTIO_MMIO_QUEUE_READY: usize = 0x24;
const VIRTIO_MMIO_QUEUE_NOTIFY: usize = 0x28;
const VIRTIO_MMIO_INTERRUPT_STATUS: usize = 0x2C;
const VIRTIO_MMIO_INTERRUPT_ACK: usize = 0x30;
const VIRTIO_MMIO_STATUS: usize = 0x34;
const VIRTIO_MMIO_CONFIG_GENERATION: usize = 0x3C;

// ============================================================================
// Bits de estado del dispositivo VirtIO
// ============================================================================

const VIRTIO_CONFIG_STATUS_ACKNOWLEDGE: u32 = 1;
const VIRTIO_CONFIG_STATUS_DRIVER: u32 = 2;
const VIRTIO_CONFIG_STATUS_DRIVER_OK: u32 = 4;
const VIRTIO_CONFIG_STATUS_FEATURES_OK: u32 = 8;
const VIRTIO_CONFIG_STATUS_FAILED: u32 = 128;

// ============================================================================
// Bits de descriptor VirtIO
// ============================================================================

const VIRTQ_DESC_F_NEXT: u16 = 1;
const VIRTQ_DESC_F_WRITE: u16 = 2;

// ============================================================================
// Tipos de solicitud VirtIO-Blk
// ============================================================================

const VIRTIO_BLK_T_IN: u32 = 0;    // Leer desde dispositivo
const VIRTIO_BLK_T_OUT: u32 = 1;   // Escribir al dispositivo
const VIRTIO_BLK_T_FLUSH: u32 = 4; // Vaciar caché

const VIRTIO_BLK_S_OK: u8 = 0;
const VIRTIO_BLK_S_IOERR: u8 = 1;
const VIRTIO_BLK_S_UNSUPP: u8 = 2;

// ============================================================================
// Estructuras de datos VirtIO
// ============================================================================

/// Descriptor de la cola VirtIO (16 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VirtqDesc {
    pub addr: u64,
    pub len: u32,
    pub flags: u16,
    pub next: u16,
}

/// Área disponible de la cola VirtIO.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VirtqAvail {
    pub flags: u16,
    pub idx: u16,
    pub ring: [u16; 256],  // Máximo 256 descriptores
    pub used_event: u16,
}

/// Área usada de la cola VirtIO.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VirtqUsed {
    pub flags: u16,
    pub idx: u16,
    pub ring: [VirtqUsedElem; 256],
}

/// Elemento usado de la cola VirtIO.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VirtqUsedElem {
    pub id: u32,
    pub len: u32,
}

/// Solicitud VirtIO-Blk (encabezado).
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct VirtioBlkReq {
    pub type_: u32,
    pub reserved: u32,
    pub sector: u64,
}

/// Estado de respuesta VirtIO-Blk.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VirtioBlkResp {
    pub status: u8,
}

/// Estado del driver VirtIO-Blk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtioBlkState {
    Uninitialized,
    Acknowledged,
    Driver,
    FeaturesNegotiated,
    Ready,
    Failed,
}

/// Información del dispositivo de bloque VirtIO.
#[derive(Debug, Clone, Copy)]
pub struct VirtioBlkInfo {
    pub device_id: u32,
    pub vendor_id: u32,
    pub queue_size: u16,
    pub capacity_sectors: u64,
    pub sector_size: u32,
}

/// Driver de dispositivo de bloque VirtIO.
pub struct VirtioBlkDriver {
    mmio_base: usize,
    state: VirtioBlkState,
    info: VirtioBlkInfo,
    queue_size: u16,
    desc_table: *mut VirtqDesc,
    avail_ring: *mut VirtqAvail,
    used_ring: *mut VirtqUsed,
    queue_pfn: usize,
    next_desc_idx: u16,
    avail_idx: u16,
    used_idx: u16,
}

impl VirtioBlkDriver {
    /// Crea un nuevo driver VirtIO-Blk no inicializado.
    pub const fn new() -> Self {
        Self {
            mmio_base: 0,
            state: VirtioBlkState::Uninitialized,
            info: VirtioBlkInfo {
                device_id: 0,
                vendor_id: 0,
                queue_size: 0,
                capacity_sectors: 0,
                sector_size: 512,
            },
            queue_size: 0,
            desc_table: core::ptr::null_mut(),
            avail_ring: core::ptr::null_mut(),
            used_ring: core::ptr::null_mut(),
            queue_pfn: 0,
            next_desc_idx: 0,
            avail_idx: 0,
            used_idx: 0,
        }
    }

    /// Lee un registro de 32 bits del dispositivo VirtIO-MMIO.
    unsafe fn mmio_read(&self, offset: usize) -> u32 {
        read_volatile((self.mmio_base + offset) as *const u32)
    }

    /// Escribe un registro de 32 bits al dispositivo VirtIO-MMIO.
    unsafe fn mmio_write(&self, offset: usize, value: u32) {
        write_volatile((self.mmio_base + offset) as *mut u32, value);
    }

    /// Lee un registro de 8 bits del dispositivo VirtIO-MMIO.
    unsafe fn mmio_read8(&self, offset: usize) -> u8 {
        read_volatile((self.mmio_base + offset) as *const u8)
    }

    /// Escribe un registro de 8 bits al dispositivo VirtIO-MMIO.
    unsafe fn mmio_write8(&self, offset: usize, value: u8) {
        write_volatile((self.mmio_base + offset) as *mut u8, value);
    }

    /// Busca e inicializa el primer dispositivo VirtIO-Blk disponible vía PCI.
    pub unsafe fn discover_and_init(&mut self) -> Result<(), &'static str> {
        let devices = find_virtio_devices();
        if devices.is_empty() {
            return Err("No se encontraron dispositivos VirtIO PCI");
        }

        for dev in devices.iter() {
            // Verificar si es un dispositivo de bloque (device ID 0x1001 = VirtIO-Blk)
            // o cualquier dispositivo VirtIO (intentaremos inicializar)
            if dev.device_id & 0xFF == 0x01 || dev.device_id == 0x1001 {
                if pci_bar_is_mmio(dev.bar0) {
                    self.mmio_base = pci_bar_mmio_addr(dev.bar0);
                    return self.init_mmio();
                }
            }
        }

        Err("No se encontró dispositivo VirtIO-Blk con BAR MMIO")
    }

    /// Inicializa el dispositivo VirtIO-MMIO.
    unsafe fn init_mmio(&mut self) -> Result<(), &'static str> {
        // Paso 1: Resetear el dispositivo
        self.mmio_write(VIRTIO_MMIO_STATUS, 0);

        // Paso 2: Verificar magic y version
        let magic = self.mmio_read(VIRTIO_MMIO_MAGIC_VALUE);
        if magic != 0x74726976 {
            return Err("Magic value de VirtIO inválido");
        }
        let version = self.mmio_read(VIRTIO_MMIO_VERSION);
        if version != 2 {
            return Err("Versión de VirtIO no soportada (se requiere 1.0)");
        }

        self.info.device_id = self.mmio_read(VIRTIO_MMIO_DEVICE_ID);
        self.info.vendor_id = self.mmio_read(VIRTIO_MMIO_VENDOR_ID);

        // Paso 3: ACKNOWLEDGE
        self.mmio_write(VIRTIO_MMIO_STATUS, VIRTIO_CONFIG_STATUS_ACKNOWLEDGE);

        // Paso 4: DRIVER
        self.mmio_write(VIRTIO_MMIO_STATUS, VIRTIO_CONFIG_STATUS_DRIVER);

        // Paso 5: Negociar características
        let device_features = self.mmio_read(VIRTIO_MMIO_DEVICE_FEATURES);
        // Solo aceptamos características básicas (no necesitamos ninguna especial)
        self.mmio_write(VIRTIO_MMIO_DRIVER_FEATURES, 0);

        // Paso 6: Configurar la cola 0 (cola de solicitudes de bloque)
        self.queue_size = self.mmio_read(VIRTIO_MMIO_QUEUE_NUM_MAX) as u16;
        if self.queue_size == 0 {
            return Err("El dispositivo no colas de solicitudes");
        }

        // Limitar a 256 descriptores (máximo de nuestra implementación)
        if self.queue_size > 256 {
            self.queue_size = 256;
        }

        self.mmio_write(VIRTIO_MMIO_QUEUE_SEL, 0);

        // Asignar memoria para la cola (descriptor table, avail ring, used ring)
        let desc_size = self.queue_size as usize * core::mem::size_of::<VirtqDesc>();
        let avail_size = core::mem::size_of::<VirtqAvail>();
        let used_size = core::mem::size_of::<VirtqUsed>();
        let queue_mem_size = (desc_size + avail_size + used_size + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);

        let frame = alloc_frame()
            .map_err(|_| "No se pudo asignar marco de página para la cola VirtIO")?;
        let queue_addr = frame_to_addr(frame);
        core::ptr::write_bytes(queue_addr as *mut u8, 0, queue_mem_size);

        self.desc_table = queue_addr as *mut VirtqDesc;
        self.avail_ring = (queue_addr + self.queue_size as usize * core::mem::size_of::<VirtqDesc>()) as *mut VirtqAvail;
        self.used_ring = (queue_addr + self.queue_size as usize * core::mem::size_of::<VirtqDesc>()
            + core::mem::size_of::<VirtqAvail>()) as *mut VirtqUsed;

        // Inicializar disponible ring
        unsafe {
            write_volatile(&mut (*self.avail_ring).flags, 0);
            write_volatile(&mut (*self.avail_ring).idx, 0);
        }
        // Inicializar usado ring
        unsafe {
            write_volatile(&mut (*self.used_ring).flags, 0);
            write_volatile(&mut (*self.used_ring).idx, 0);
        }

        // Asignar el PFN de la cola
        let queue_pfn = queue_addr / PAGE_SIZE;
        self.queue_pfn = queue_pfn;
        self.mmio_write(VIRTIO_MMIO_QUEUE_NUM, self.queue_size as u32);
        self.mmio_write(VIRTIO_MMIO_QUEUE_PFN, queue_pfn as u32);

        // Verificar que la cola está lista
        let queue_ready = self.mmio_read(VIRTIO_MMIO_QUEUE_READY);
        if queue_ready == 0 {
            return Err("La cola VirtIO no está lista");
        }

        // Paso 7: DRIVER_OK
        self.mmio_write(VIRTIO_MMIO_STATUS, VIRTIO_CONFIG_STATUS_DRIVER_OK);

        self.state = VirtioBlkState::Ready;

        // Leer capacidad del dispositivo (config space offset 0)
        self.info.capacity_sectors = self.read_config_u64(0);
        self.info.sector_size = self.read_config_u32(0x16) as u32;
        if self.info.sector_size == 0 {
            self.info.sector_size = 512;
        }

        Ok(())
    }

    /// Lee un valor de 64 bits del espacio de configuración del dispositivo.
    unsafe fn read_config_u64(&self, offset: usize) -> u64 {
        let low = self.mmio_read(0x30 + offset) as u64;
        let high = self.mmio_read(0x30 + offset + 4) as u64;
        low | (high << 32)
    }

    /// Lee un valor de 32 bits del espacio de configuración del dispositivo.
    unsafe fn read_config_u32(&self, offset: usize) -> u32 {
        self.mmio_read(0x30 + offset)
    }

    /// Obtiene el siguiente índice de descriptor libre.
    fn next_desc(&mut self) -> u16 {
        let idx = self.next_desc_idx;
        self.next_desc_idx = (self.next_desc_idx + 1) % self.queue_size;
        idx
    }

    /// Envía una solicitud de lectura al dispositivo VirtIO-Blk.
    pub unsafe fn read_blocks(&mut self, sector: u64, sector_count: u32, buffer: *mut u8) -> Result<(), &'static str> {
        if self.state != VirtioBlkState::Ready {
            return Err("Dispositivo VirtIO-Blk no listo");
        }

        if buffer.is_null() {
            return Err("Buffer nulo");
        }

        // Construir la cadena de descriptores: req (read-only) + data (write-only) + status (write-only)
        let desc_head = self.next_desc();
        let desc_data = self.next_desc();
        let desc_status = self.next_desc();

        // Nota: En una implementación completa, necesitaríamos memoria separada para req, data y status.
        // Aquí usamos el buffer para datos y escribimos req/status en memoria temporal.

        // Asignar memoria para la solicitud y respuesta (en el heap del kernel)
        let req_mem = alloc_frame()
            .map_err(|_| "No se pudo asignar memoria para solicitud VirtIO")?;
        let req_addr_mem = frame_to_addr(req_mem) as *mut VirtioBlkReq;
        let resp_addr_mem = frame_to_addr(req_mem) + core::mem::size_of::<VirtioBlkReq>();

        write_volatile(req_addr_mem, VirtioBlkReq {
            type_: VIRTIO_BLK_T_IN,
            reserved: 0,
            sector,
        });
        write_volatile(resp_addr_mem as *mut VirtioBlkResp, VirtioBlkResp { status: 0xFF });

        // Configurar descriptor de solicitud (read-only)
        write_volatile(&mut (*self.desc_table.add(desc_head as usize)).addr, req_addr_mem as u64);
        write_volatile(&mut (*self.desc_table.add(desc_head as usize)).len, core::mem::size_of::<VirtioBlkReq>() as u32);
        write_volatile(&mut (*self.desc_table.add(desc_head as usize)).flags, 0);
        write_volatile(&mut (*self.desc_table.add(desc_head as usize)).next, desc_data);

        // Configurar descriptor de datos (write-only)
        write_volatile(&mut (*self.desc_table.add(desc_data as usize)).addr, buffer as u64);
        write_volatile(&mut (*self.desc_table.add(desc_data as usize)).len, sector_count as u32 * self.info.sector_size as u32);
        write_volatile(&mut (*self.desc_table.add(desc_data as usize)).flags, VIRTQ_DESC_F_WRITE);
        write_volatile(&mut (*self.desc_table.add(desc_data as usize)).next, desc_status);

        // Configurar descriptor de estado (write-only)
        write_volatile(&mut (*self.desc_table.add(desc_status as usize)).addr, resp_addr_mem as u64);
        write_volatile(&mut (*self.desc_table.add(desc_status as usize)).len, core::mem::size_of::<VirtioBlkResp>() as u32);
        write_volatile(&mut (*self.desc_table.add(desc_status as usize)).flags, VIRTQ_DESC_F_WRITE);
        write_volatile(&mut (*self.desc_table.add(desc_status as usize)).next, 0xFFFF);

        // Agregar al available ring
        let avail_idx = self.avail_idx;
        write_volatile(&mut (*self.avail_ring).ring[avail_idx as usize], desc_head);
        self.avail_idx = (self.avail_idx + 1) % self.queue_size;
        write_volatile(&mut (*self.avail_ring).idx, self.avail_idx);

        // Notificar al dispositivo
        self.mmio_write(VIRTIO_MMIO_QUEUE_NOTIFY, 0);

        // Esperar a que se complete (polling simple)
        let mut timeout = 100000;
        while timeout > 0 {
            let status = read_volatile(resp_addr_mem as *const VirtioBlkResp);
            if status.status != 0xFF {
                if status.status == VIRTIO_BLK_S_OK {
                    // Liberar marco de página temporal
                    dealloc_frame(req_mem);
                    return Ok(());
                } else {
                    dealloc_frame(req_mem);
                    return Err("Error de E/S en VirtIO-Blk");
                }
            }
            timeout -= 1;
            for _ in 0..100 {
                core::hint::spin_loop();
            }
        }

        dealloc_frame(req_mem);
        Err("Tiempo de espera agotado en VirtIO-Blk read")
    }

    /// Envía una solicitud de escritura al dispositivo VirtIO-Blk.
    pub unsafe fn write_blocks(&mut self, sector: u64, sector_count: u32, buffer: *const u8) -> Result<(), &'static str> {
        if self.state != VirtioBlkState::Ready {
            return Err("Dispositivo VirtIO-Blk no listo");
        }

        if buffer.is_null() {
            return Err("Buffer nulo");
        }

        let desc_head = self.next_desc();
        let desc_data = self.next_desc();
        let desc_status = self.next_desc();

        let req_mem = alloc_frame()
            .map_err(|_| "No se pudo asignar memoria para solicitud VirtIO")?;
        let req_addr_mem = frame_to_addr(req_mem) as *mut VirtioBlkReq;
        let resp_addr_mem = frame_to_addr(req_mem) + core::mem::size_of::<VirtioBlkReq>();

        write_volatile(req_addr_mem, VirtioBlkReq {
            type_: VIRTIO_BLK_T_OUT,
            reserved: 0,
            sector,
        });
        write_volatile(resp_addr_mem as *mut VirtioBlkResp, VirtioBlkResp { status: 0xFF });

        // Descriptor de solicitud (read-only)
        write_volatile(&mut (*self.desc_table.add(desc_head as usize)).addr, req_addr_mem as u64);
        write_volatile(&mut (*self.desc_table.add(desc_head as usize)).len, core::mem::size_of::<VirtioBlkReq>() as u32);
        write_volatile(&mut (*self.desc_table.add(desc_head as usize)).flags, 0);
        write_volatile(&mut (*self.desc_table.add(desc_head as usize)).next, desc_data);

        // Descriptor de datos (read-only para el dispositivo, lee del buffer)
        write_volatile(&mut (*self.desc_table.add(desc_data as usize)).addr, buffer as u64);
        write_volatile(&mut (*self.desc_table.add(desc_data as usize)).len, sector_count as u32 * self.info.sector_size as u32);
        write_volatile(&mut (*self.desc_table.add(desc_data as usize)).flags, 0);
        write_volatile(&mut (*self.desc_table.add(desc_data as usize)).next, desc_status);

        // Descriptor de estado (write-only)
        write_volatile(&mut (*self.desc_table.add(desc_status as usize)).addr, resp_addr_mem as u64);
        write_volatile(&mut (*self.desc_table.add(desc_status as usize)).len, core::mem::size_of::<VirtioBlkResp>() as u32);
        write_volatile(&mut (*self.desc_table.add(desc_status as usize)).flags, VIRTQ_DESC_F_WRITE);
        write_volatile(&mut (*self.desc_table.add(desc_status as usize)).next, 0xFFFF);

        // Agregar al available ring
        let avail_idx = self.avail_idx;
        write_volatile(&mut (*self.avail_ring).ring[avail_idx as usize], desc_head);
        self.avail_idx = (self.avail_idx + 1) % self.queue_size;
        write_volatile(&mut (*self.avail_ring).idx, self.avail_idx);

        // Notificar
        self.mmio_write(VIRTIO_MMIO_QUEUE_NOTIFY, 0);

        // Esperar completion
        let mut timeout = 100000;
        while timeout > 0 {
            let status = read_volatile(resp_addr_mem as *const VirtioBlkResp);
            if status.status != 0xFF {
                if status.status == VIRTIO_BLK_S_OK {
                    dealloc_frame(req_mem);
                    return Ok(());
                } else {
                    dealloc_frame(req_mem);
                    return Err("Error de E/S en VirtIO-Blk write");
                }
            }
            timeout -= 1;
            for _ in 0..100 {
                core::hint::spin_loop();
            }
        }

        dealloc_frame(req_mem);
        Err("Tiempo de espera agotado en VirtIO-Blk write")
    }

    /// Devuelve la información del dispositivo.
    pub fn info(&self) -> VirtioBlkInfo {
        self.info
    }

    /// Devuelve el estado del driver.
    pub fn state(&self) -> VirtioBlkState {
        self.state
    }
}

/// Instancia global del driver VirtIO-Blk.
static mut VIRTIO_BLK: VirtioBlkDriver = VirtioBlkDriver::new();

/// Devuelve una referencia mutable al driver VirtIO-Blk global.
pub fn global_virtio_blk() -> &'static mut VirtioBlkDriver {
    unsafe { &mut VIRTIO_BLK }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtio_blk_states() {
        assert_eq!(VirtioBlkState::Uninitialized as usize, 0);
        assert_eq!(VirtioBlkState::Acknowledged as usize, 1);
        assert_eq!(VirtioBlkState::Driver as usize, 2);
        assert_eq!(VirtioBlkState::FeaturesNegotiated as usize, 3);
        assert_eq!(VirtioBlkState::Ready as usize, 4);
        assert_eq!(VirtioBlkState::Failed as usize, 5);
    }

    #[test]
    fn virtio_blk_req_type() {
        assert_eq!(VIRTIO_BLK_T_IN, 0);
        assert_eq!(VIRTIO_BLK_T_OUT, 1);
        assert_eq!(VIRTIO_BLK_T_FLUSH, 4);
    }

    #[test]
    fn virtio_blk_resp_status() {
        assert_eq!(VIRTIO_BLK_S_OK, 0);
        assert_eq!(VIRTIO_BLK_S_IOERR, 1);
        assert_eq!(VIRTIO_BLK_S_UNSUPP, 2);
    }
}
