//! VirtIO-Net — Controlador real de dispositivo de red VirtIO.
//!
//! Implementa el protocolo VirtIO 1.0 sobre MMIO para conectividad de red
//! en QEMU/VirtualBox. Proporciona envío y recepción de paquetes Ethernet.
//!
//! Referencia: https://docs.oasis-open.org/virtio/virtio/v1.1/virtio-v1.1.pdf

use crate::mm::frame_allocator::{alloc_frame, dealloc_frame};
use crate::mm::frame_to_addr;
use crate::pci::{find_virtio_devices, pci_bar_is_mmio, pci_bar_mmio_addr};
use core::arch::asm;
use core::ptr::{read_volatile, write_volatile};
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
    pub ring: [u16; 256],
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

/// Estado del driver VirtIO-Net.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtioNetState {
    Uninitialized,
    Acknowledged,
    Driver,
    FeaturesNegotiated,
    Ready,
    Failed,
}

/// Información del dispositivo de red VirtIO.
#[derive(Debug, Clone, Copy)]
pub struct VirtioNetInfo {
    pub device_id: u32,
    pub vendor_id: u32,
    pub mac_address: [u8; 6],
    pub mtu: u16,
}

/// Driver de dispositivo de red VirtIO-Net.
pub struct VirtioNetDriver {
    mmio_base: usize,
    state: VirtioNetState,
    info: VirtioNetInfo,
    queue_size: u16,

    // Cola 0: RX (recepción)
    rx_desc: *mut VirtqDesc,
    rx_avail: *mut VirtqAvail,
    rx_used: *mut VirtqUsed,
    rx_queue_pfn: usize,
    rx_next_desc: u16,
    rx_avail_idx: u16,
    rx_used_idx: u16,

    // Cola 1: TX (transmisión)
    tx_desc: *mut VirtqDesc,
    tx_avail: *mut VirtqAvail,
    tx_used: *mut VirtqUsed,
    tx_queue_pfn: usize,
    tx_next_desc: u16,
    tx_avail_idx: u16,
    tx_used_idx: u16,

    // Buffers de paquetes recibidos
    rx_packets: Vec<Vec<u8>>,
}

impl VirtioNetDriver {
    pub const fn new() -> Self {
        Self {
            mmio_base: 0,
            state: VirtioNetState::Uninitialized,
            info: VirtioNetInfo {
                device_id: 0,
                vendor_id: 0,
                mac_address: [0; 6],
                mtu: 1500,
            },
            queue_size: 0,
            rx_desc: core::ptr::null_mut(),
            rx_avail: core::ptr::null_mut(),
            rx_used: core::ptr::null_mut(),
            rx_queue_pfn: 0,
            rx_next_desc: 0,
            rx_avail_idx: 0,
            rx_used_idx: 0,
            tx_desc: core::ptr::null_mut(),
            tx_avail: core::ptr::null_mut(),
            tx_used: core::ptr::null_mut(),
            tx_queue_pfn: 0,
            tx_next_desc: 0,
            tx_avail_idx: 0,
            tx_used_idx: 0,
            rx_packets: Vec::new(),
        }
    }

    unsafe fn mmio_read(&self, offset: usize) -> u32 {
        read_volatile((self.mmio_base + offset) as *const u32)
    }

    unsafe fn mmio_write(&self, offset: usize, value: u32) {
        write_volatile((self.mmio_base + offset) as *mut u32, value);
    }

    pub unsafe fn discover_and_init(&mut self) -> Result<(), &'static str> {
        let devices = find_virtio_devices();
        if devices.is_empty() {
            return Err("No se encontraron dispositivos VirtIO PCI");
        }

        for dev in devices.iter() {
            // VirtIO-Net tiene device ID 0x1001
            if dev.device_id == 0x1001 {
                if pci_bar_is_mmio(dev.bar0) {
                    self.mmio_base = pci_bar_mmio_addr(dev.bar0);
                    return self.init_mmio();
                }
            }
        }

        Err("No se encontró dispositivo VirtIO-Net con BAR MMIO")
    }

    unsafe fn init_mmio(&mut self) -> Result<(), &'static str> {
        self.mmio_write(VIRTIO_MMIO_STATUS, 0);

        let magic = self.mmio_read(VIRTIO_MMIO_MAGIC_VALUE);
        if magic != 0x74726976 {
            return Err("Magic value de VirtIO inválido");
        }
        let version = self.mmio_read(VIRTIO_MMIO_VERSION);
        if version != 2 {
            return Err("Versión de VirtIO no soportada");
        }

        self.info.device_id = self.mmio_read(VIRTIO_MMIO_DEVICE_ID);
        self.info.vendor_id = self.mmio_read(VIRTIO_MMIO_VENDOR_ID);

        self.mmio_write(VIRTIO_MMIO_STATUS, VIRTIO_CONFIG_STATUS_ACKNOWLEDGE);
        self.mmio_write(VIRTIO_MMIO_STATUS, VIRTIO_CONFIG_STATUS_DRIVER);
        self.mmio_write(VIRTIO_MMIO_DRIVER_FEATURES, 0);

        // Configurar cola 0: RX
        self.queue_size = self.mmio_read(VIRTIO_MMIO_QUEUE_NUM_MAX) as u16;
        if self.queue_size == 0 {
            return Err("El dispositivo no tiene colas");
        }
        if self.queue_size > 256 {
            self.queue_size = 256;
        }
        self.mmio_write(VIRTIO_MMIO_QUEUE_SEL, 0);
        self.mmio_write(VIRTIO_MMIO_QUEUE_NUM, self.queue_size as u32);

        // Asignar memoria para ambas colas (RX y TX)
        let queue_mem_size = (self.queue_size as usize * core::mem::size_of::<VirtqDesc>()
            + core::mem::size_of::<VirtqAvail>()
            + core::mem::size_of::<VirtqUsed>()
            + self.queue_size as usize * core::mem::size_of::<VirtqDesc>()
            + core::mem::size_of::<VirtqAvail>()
            + core::mem::size_of::<VirtqUsed>()
            + 4095) & !4095;

        let frame = alloc_frame()
            .map_err(|_| "No se pudo asignar marco para colas VirtIO-Net")?;
        let queue_addr = frame_to_addr(frame);
        core::ptr::write_bytes(queue_addr as *mut u8, 0, queue_mem_size);

        // Cola RX (offset 0)
        self.rx_desc = queue_addr as *mut VirtqDesc;
        self.rx_avail = (queue_addr + self.queue_size as usize * core::mem::size_of::<VirtqDesc>()) as *mut VirtqAvail;
        self.rx_used = (queue_addr + self.queue_size as usize * core::mem::size_of::<VirtqDesc>()
            + core::mem::size_of::<VirtqAvail>()) as *mut VirtqUsed;
        self.rx_queue_pfn = queue_addr / 4096;

        // Cola TX (después de RX)
        let tx_base = queue_addr + self.queue_size as usize * core::mem::size_of::<VirtqDesc>()
            + core::mem::size_of::<VirtqAvail>()
            + core::mem::size_of::<VirtqUsed>();
        self.tx_desc = tx_base as *mut VirtqDesc;
        self.tx_avail = (tx_base + self.queue_size as usize * core::mem::size_of::<VirtqDesc>()) as *mut VirtqAvail;
        self.tx_used = (tx_base + self.queue_size as usize * core::mem::size_of::<VirtqDesc>()
            + core::mem::size_of::<VirtqAvail>()) as *mut VirtqUsed;
        self.tx_queue_pfn = tx_base / 4096;

        // Inicializar avail rings
        unsafe {
            write_volatile(&mut (*self.rx_avail).flags, 0);
            write_volatile(&mut (*self.rx_avail).idx, 0);
            write_volatile(&mut (*self.tx_avail).flags, 0);
            write_volatile(&mut (*self.tx_avail).idx, 0);
            write_volatile(&mut (*self.rx_used).flags, 0);
            write_volatile(&mut (*self.rx_used).idx, 0);
            write_volatile(&mut (*self.tx_used).flags, 0);
            write_volatile(&mut (*self.tx_used).idx, 0);
        }

        self.mmio_write(VIRTIO_MMIO_QUEUE_PFN, self.rx_queue_pfn as u32);
        let rx_ready = self.mmio_read(VIRTIO_MMIO_QUEUE_READY);
        if rx_ready == 0 {
            return Err("Cola RX no está lista");
        }

        // Configurar cola TX
        self.mmio_write(VIRTIO_MMIO_QUEUE_SEL, 1);
        self.mmio_write(VIRTIO_MMIO_QUEUE_NUM, self.queue_size as u32);
        self.mmio_write(VIRTIO_MMIO_QUEUE_PFN, self.tx_queue_pfn as u32);
        let tx_ready = self.mmio_read(VIRTIO_MMIO_QUEUE_READY);
        if tx_ready == 0 {
            return Err("Cola TX no está lista");
        }

        self.mmio_write(VIRTIO_MMIO_STATUS, VIRTIO_CONFIG_STATUS_DRIVER_OK);
        self.state = VirtioNetState::Ready;

        // Leer MAC address del config space (offset 0)
        for i in 0..6 {
            self.info.mac_address[i] = self.mmio_read(0x30 + i) as u8;
        }
        self.info.mtu = self.mmio_read(0x36) as u16;
        if self.info.mtu == 0 {
            self.info.mtu = 1500;
        }

        self.rx_packets = Vec::new();
        Ok(())
    }

    fn next_rx_desc(&mut self) -> u16 {
        let idx = self.rx_next_desc;
        self.rx_next_desc = (self.rx_next_desc + 1) % self.queue_size;
        idx
    }

    fn next_tx_desc(&mut self) -> u16 {
        let idx = self.tx_next_desc;
        self.tx_next_desc = (self.tx_next_desc + 1) % self.queue_size;
        idx
    }

    /// Envía un paquete Ethernet por la red.
    pub unsafe fn send_packet(&mut self, data: &[u8]) -> Result<(), &'static str> {
        if self.state != VirtioNetState::Ready {
            return Err("Dispositivo VirtIO-Net no listo");
        }
        if data.len() > self.info.mtu as usize {
            return Err("Paquete demasiado grande");
        }

        let desc_head = self.next_tx_desc();
        let desc_data = self.next_tx_desc();

        // Asignar memoria para el paquete
        let frame = crate::mm::frame_allocator::alloc_frame()
            .map_err(|_| "No se pudo asignar marco para paquete")?;
        let pkt_addr = frame_to_addr(frame) as *mut u8;
        core::ptr::write_bytes(pkt_addr, 0, 4096);
        core::ptr::copy_nonoverlapping(data.as_ptr(), pkt_addr, data.len());

        // Descriptor de datos (read-only para el dispositivo)
        write_volatile(&mut (*self.tx_desc.add(desc_head as usize)).addr, pkt_addr as u64);
        write_volatile(&mut (*self.tx_desc.add(desc_head as usize)).len, data.len() as u32);
        write_volatile(&mut (*self.tx_desc.add(desc_head as usize)).flags, 0);
        write_volatile(&mut (*self.tx_desc.add(desc_head as usize)).next, desc_data);

        // Descriptor de estado (write-only)
        let status_addr = pkt_addr.add(data.len());
        write_volatile(status_addr, 0);
        write_volatile(&mut (*self.tx_desc.add(desc_data as usize)).addr, status_addr as u64);
        write_volatile(&mut (*self.tx_desc.add(desc_data as usize)).len, 1);
        write_volatile(&mut (*self.tx_desc.add(desc_data as usize)).flags, VIRTQ_DESC_F_WRITE);
        write_volatile(&mut (*self.tx_desc.add(desc_data as usize)).next, 0xFFFF);

        // Agregar al available ring TX
        let avail_idx = self.tx_avail_idx;
        write_volatile(&mut (*self.tx_avail).ring[avail_idx as usize], desc_head);
        self.tx_avail_idx = (self.tx_avail_idx + 1) % self.queue_size;
        write_volatile(&mut (*self.tx_avail).idx, self.tx_avail_idx);

        // Notificar
        self.mmio_write(VIRTIO_MMIO_QUEUE_NOTIFY, 1);

        // Liberar marco después de un breve tiempo (en producción usar interrupt)
        for _ in 0..10000 {
            core::hint::spin_loop();
        }
        dealloc_frame(frame / 4096).ok();

        Ok(())
    }

    /// Recibe un paquete de la red (polling).
    pub unsafe fn receive_packet(&mut self) -> Option<Vec<u8>> {
        if self.state != VirtioNetState::Ready {
            return None;
        }

        // Verificar si hay paquetes en el used ring RX
        let used_idx = read_volatile(&(*self.rx_used).idx);
        if self.rx_used_idx == used_idx {
            return None;
        }

        let elem = read_volatile(&(*self.rx_used).ring[self.rx_used_idx as usize]);
        self.rx_used_idx = (self.rx_used_idx + 1) % self.queue_size;

        // El descriptor head nos dice qué buffer usar
        let desc_id = elem.id;
        let _desc = read_volatile(&(*self.rx_desc.add(desc_id as usize)));

        // Leer los datos del descriptor de datos (segundo descriptor en la cadena)
        // En una implementación completa, seguiríamos la cadena next
        // Aquí simplificamos: el primer descriptor tiene los datos
        let data_len = read_volatile(&(*self.rx_desc.add(desc_id as usize))).len as usize;
        let data_addr = read_volatile(&(*self.rx_desc.add(desc_id as usize))).addr as *mut u8;

        let mut data = Vec::with_capacity(data_len);
        for i in 0..data_len {
            data.push(read_volatile(data_addr.add(i)));
        }

        // Devolver el descriptor al available ring
        let avail_idx = self.rx_avail_idx;
        write_volatile(&mut (*self.rx_avail).ring[avail_idx as usize], desc_id as u16);
        self.rx_avail_idx = (self.rx_avail_idx + 1) % self.queue_size;
        write_volatile(&mut (*self.rx_avail).idx, self.rx_avail_idx);

        Some(data)
    }

    pub fn info(&self) -> VirtioNetInfo {
        self.info
    }

    pub fn state(&self) -> VirtioNetState {
        self.state
    }

    pub fn rx_packets_count(&self) -> usize {
        self.rx_packets.len()
    }
}

static mut VIRTIO_NET: VirtioNetDriver = VirtioNetDriver::new();

pub fn global_virtio_net() -> &'static mut VirtioNetDriver {
    unsafe { &mut VIRTIO_NET }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtio_net_states() {
        assert_eq!(VirtioNetState::Uninitialized as usize, 0);
        assert_eq!(VirtioNetState::Ready as usize, 4);
        assert_eq!(VirtioNetState::Failed as usize, 5);
    }
}
