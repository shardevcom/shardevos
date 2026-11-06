//! PCI — Enumeración y configuración del bus PCI de shardevos.
//!
//! Proporciona:
//! - Lectura/escritura de registros de configuración PCI (puertos 0xCF8/0xCFC)
//! - Enumeración de dispositivos en el bus
//! - Búsqueda de dispositivos VirtIO (vendor 0x1AF4)
//! - Asignación de recursos MMIO y puertos I/O

use core::arch::asm;
use alloc::vec::Vec;

/// Puerto de dirección de configuración PCI.
const PCI_CONFIG_ADDRESS: u16 = 0xCF8;
/// Puerto de datos de configuración PCI.
const PCI_CONFIG_DATA: u16 = 0xCFC;

/// Número máximo de buses PCI.
const PCI_MAX_BUSES: u16 = 256;
/// Número máximo de dispositivos por bus.
const PCI_MAX_DEVICES: u8 = 32;
/// Número máximo de funciones por dispositivo.
const PCI_MAX_FUNCTIONS: u8 = 8;

/// ID de vendor de Red Hat / VirtIO.
pub const PCI_VENDOR_VIRTIO: u16 = 0x1AF4;
/// ID de dispositivo VirtIO (rango 0x1000-0x107F para dispositivos net/block/etc).
pub const PCI_DEVICE_VIRTIO_BASE: u16 = 0x1000;

/// Clase de dispositivo: controlador de almacenamiento masivo.
pub const PCI_CLASS_STORAGE: u8 = 0x01;
/// Subclase: controlador NVM Express.
pub const PCI_SUBCLASS_NVME: u8 = 0x08;
/// Clase de dispositivo: controlador de red.
pub const PCI_CLASS_NETWORK: u8 = 0x02;
/// Subclase: Ethernet.
pub const PCI_SUBCLASS_ETHERNET: u8 = 0x00;

/// Identificador de un dispositivo PCI encontrado.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct PciDevice {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub class_code: u8,
    pub subclass: u8,
    pub prog_if: u8,
    pub revision_id: u8,
    pub bar0: u32,
    pub bar1: u32,
    pub bar2: u32,
    pub bar3: u32,
    pub bar4: u32,
    pub bar5: u32,
    pub irq_line: u8,
}

/// Lee un registro de 32 bits de configuración PCI.
unsafe fn pci_read_config_dword(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    let address: u32 = 0x80000000
        | ((bus as u32) << 16)
        | ((device as u32) << 11)
        | ((function as u32) << 8)
        | (offset as u32 & 0xFC);

    asm!("out dx, eax", in("dx") PCI_CONFIG_ADDRESS, in("eax") address, options(nostack));
    let value: u32;
    asm!("in eax, dx", out("eax") value, in("dx") PCI_CONFIG_DATA, options(nostack));
    value
}

/// Escribe un registro de 32 bits de configuración PCI.
unsafe fn pci_write_config_dword(bus: u8, device: u8, function: u8, offset: u8, value: u32) {
    let address: u32 = 0x80000000
        | ((bus as u32) << 16)
        | ((device as u32) << 11)
        | ((function as u32) << 8)
        | (offset as u32 & 0xFC);

    asm!("out dx, eax", in("dx") PCI_CONFIG_ADDRESS, in("eax") address, options(nostack));
    asm!("out dx, eax", in("dx") PCI_CONFIG_DATA, in("eax") value, options(nostack));
}

/// Lee una palabra de 16 bits de configuración PCI.
unsafe fn pci_read_config_word(bus: u8, device: u8, function: u8, offset: u8) -> u16 {
    let value = pci_read_config_dword(bus, device, function, offset & 0xFC);
    let shift = ((offset & 0x02) * 8) as u32;
    ((value >> shift) & 0xFFFF) as u16
}

/// Lee un byte de configuración PCI.
unsafe fn pci_read_config_byte(bus: u8, device: u8, function: u8, offset: u8) -> u8 {
    let value = pci_read_config_dword(bus, device, function, offset & 0xFC);
    let shift = ((offset & 0x03) * 8) as u32;
    ((value >> shift) & 0xFF) as u8
}

/// Verifica si una función existe (vendor ID != 0xFFFF).
fn pci_function_exists(bus: u8, device: u8, function: u8) -> bool {
    unsafe { pci_read_config_word(bus, device, function, 0) != 0xFFFF }
}

/// Obtiene el tipo de cabecera (bit 7 = multifunción).
unsafe fn pci_get_header_type(bus: u8, device: u8, function: u8) -> u8 {
    pci_read_config_byte(bus, device, function, 0x0E) & 0x7F
}

/// Habilita el acceso a memoria e I/O en el registro de comando.
unsafe fn pci_enable_bus_mastering(bus: u8, device: u8, function: u8) {
    let mut cmd = pci_read_config_word(bus, device, function, 0x04);
    cmd |= 0x0006; // Bit 1: Memory space, Bit 2: Bus master
    pci_write_config_dword(bus, device, function, 0x04, cmd as u32);
}

/// Lee la dirección base de un BAR (puerto 0x10-0x24).
unsafe fn pci_read_bar(bus: u8, device: u8, function: u8, bar_index: u8) -> u32 {
    pci_read_config_dword(bus, device, function, 0x10 + bar_index * 4)
}

/// Enumerar todos los dispositivos PCI y devolver una lista de los encontrados.
pub fn enumerate_pci_devices() -> Vec<PciDevice> {
    let mut devices: Vec<PciDevice> = Vec::new();

    for bus in 0..PCI_MAX_BUSES {
        for device in 0..PCI_MAX_DEVICES {
            if !pci_function_exists(bus as u8, device, 0) {
                continue;
            }

            let header_type = unsafe { pci_get_header_type(bus as u8, device, 0) };
            let is_multi_function = (header_type & 0x80) != 0;
            let max_function = if is_multi_function { PCI_MAX_FUNCTIONS } else { 1 };

            for function in 0..max_function {
                if function > 0 && !pci_function_exists(bus as u8, device, function) {
                    continue;
                }

                let vendor_id = unsafe { pci_read_config_word(bus as u8, device, function, 0) };
                let device_id = unsafe { pci_read_config_word(bus as u8, device, function, 2) };
                let class_code = unsafe { pci_read_config_byte(bus as u8, device, function, 0x0B) };
                let subclass = unsafe { pci_read_config_byte(bus as u8, device, function, 0x0A) };
                let prog_if = unsafe { pci_read_config_byte(bus as u8, device, function, 0x09) };
                let revision_id = unsafe { pci_read_config_byte(bus as u8, device, function, 0x08) };
                let irq_line = unsafe { pci_read_config_byte(bus as u8, device, function, 0x3C) };

                // Leer BARs (0-5)
                let mut bars = [0u32; 6];
                for i in 0..6 {
                    bars[i] = unsafe { pci_read_bar(bus as u8, device, function, i as u8) };
                }

                // Habilitar bus mastering para dispositivos VirtIO
                if vendor_id == PCI_VENDOR_VIRTIO {
                    unsafe { pci_enable_bus_mastering(bus as u8, device, function); }
                }

                devices.push(PciDevice {
                    bus: bus as u8,
                    device,
                    function,
                    vendor_id,
                    device_id,
                    class_code,
                    subclass,
                    prog_if,
                    revision_id,
                    bar0: bars[0],
                    bar1: bars[1],
                    bar2: bars[2],
                    bar3: bars[3],
                    bar4: bars[4],
                    bar5: bars[5],
                    irq_line,
                });
            }
        }
    }

    devices
}

/// Busca dispositivos VirtIO en el bus PCI.
pub fn find_virtio_devices() -> Vec<PciDevice> {
    let all = enumerate_pci_devices();
    all.into_iter()
        .filter(|d| d.vendor_id == PCI_VENDOR_VIRTIO)
        .collect()
}

/// Verifica si un BAR es de tipo MMIO (bit 0 = 0) o I/O (bit 0 = 1).
pub fn pci_bar_is_mmio(bar: u32) -> bool {
    (bar & 1) == 0
}

/// Obtiene la dirección base de un BAR MMIO (máscara de bits bajos).
pub fn pci_bar_mmio_addr(bar: u32) -> usize {
    (bar & 0xFFFFFFF0) as usize
}

/// Obtiene la dirección base de un BAR I/O (máscara de bits bajos).
pub fn pci_bar_io_addr(bar: u32) -> u16 {
    (bar & 0xFFFC) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pci_virtio_vendor() {
        assert_eq!(PCI_VENDOR_VIRTIO, 0x1AF4);
    }

    #[test]
    fn pci_bar_is_mmio_test() {
        // Bit 0 = 0 es MMIO
        assert!(pci_bar_is_mmio(0x1234_0000));
        // Bit 0 = 1 es I/O
        assert!(!pci_bar_is_mmio(0x1234_0001));
    }

    #[test]
    fn pci_bar_mmio_addr_test() {
        let bar = 0xFED0_0000;
        assert_eq!(pci_bar_mmio_addr(bar), 0xFED0_0000);
    }

    #[test]
    fn pci_bar_io_addr_test() {
        let bar = 0x0000_3001;
        assert_eq!(pci_bar_io_addr(bar), 0x3000);
    }
}
