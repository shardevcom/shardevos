//! Network Driver — Controlador de red de shardevos.
//!
//! Proporciona:
//! - Gestión de interfaces de red (Ethernet, WiFi, etc.)
//! - Configuración de direcciones MAC e IP
//! - Envío y recepción de paquetes de red
//! - Stack TCP/IP mínimo (ARP, ICMP, TCP, UDP)
//! - Integración con el VFS y el kernel


extern crate alloc;

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// Errores del driver de red.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetError {
    NotInitialized,
    DeviceNotFound,
    InvalidLba,
    InvalidBuffer,
    InvalidPacket,
    ReadOnly,
    OutOfMemory,
    DeviceBusy,
    DeviceError,
}

/// Resultado de operaciones del driver de red.
pub type NetResult<T> = Result<T, NetError>;

/// Identificador único de dispositivo de red.
pub type NetDeviceId = u64;

pub mod stack;
#[allow(dead_code)]
const MTU: usize = 1500;

/// Tamaño de una dirección MAC (6 bytes).
const MAC_ADDR_LEN: usize = 6;

/// Tamaño de una dirección IPv4 (4 bytes).
const IP_ADDR_LEN: usize = 4;

/// Número máximo de interfaces de red.
const MAX_NET_DEVICES: usize = 8;

/// Número máximo de paquetes en la cola de recepción.
const MAX_RX_QUEUE: usize = 32;

/// Número máximo de paquetes en la cola de transmisión.
const MAX_TX_QUEUE: usize = 32;

/// Tipos de dispositivos de red soportados.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum NetDeviceType {
    Unknown = 0,
    Ethernet = 1,
    WiFi = 2,
    Virtual = 3,
    Loopback = 4,
}

/// Estado de una interfaz de red.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum NetDeviceState {
    Down = 0,
    Up = 1,
    Connecting = 2,
    Disconnected = 3,
    Error = 4,
}

/// Dirección MAC (48 bits).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct MacAddr {
    pub bytes: [u8; MAC_ADDR_LEN],
}

/// Dirección IPv4 (32 bits).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct IpAddr {
    pub bytes: [u8; IP_ADDR_LEN],
}

/// Información de un dispositivo de red.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct NetDeviceInfo {
    pub id: NetDeviceId,
    pub device_type: NetDeviceType,
    pub mac_address: MacAddr,
    pub ip_address: IpAddr,
    pub subnet_mask: IpAddr,
    pub gateway: IpAddr,
    pub mtu: u16,
    pub state: NetDeviceState,
    pub link_up: bool,
    pub tx_bytes: u64,
    pub rx_bytes: u64,
}

/// Tipo de protocolo Ethernet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum EtherType {
    IPv4 = 0x0800,
    ARP = 0x0806,
    IPv6 = 0x86DD,
}

/// Tipo de protocolo IP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum IpProtocol {
    ICMP = 1,
    TCP = 6,
    UDP = 17,
}

/// Resultado de una operación de red.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum NetOpResult {
    Success = 0,
    Error = 1,
    DeviceNotFound = 2,
    InvalidAddress = 3,
    InvalidPacket = 4,
    DeviceBusy = 5,
    DeviceError = 6,
    NoMemory = 7,
}

/// Paquete de red genérico.
#[derive(Debug, Clone)]
pub struct NetPacket {
    pub data: Vec<u8>,
    pub device_id: NetDeviceId,
    pub timestamp_us: u64,
}

/// Estadísticas de una interfaz de red.
#[derive(Debug, Clone, Copy)]
pub struct NetDeviceStats {
    pub device_id: NetDeviceId,
    pub packets_sent: u64,
    pub packets_received: u64,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub errors: u64,
    pub drops: u64,
}

/// Driver de red.
#[allow(dead_code)]
pub struct NetDeviceDriver {
    devices: spin_lock_net::SpinLock<[Option<NetDeviceInfo>; MAX_NET_DEVICES]>,
    device_count: AtomicUsize,
    rx_queue: spin_lock_net::SpinLock<[Option<u64>; MAX_RX_QUEUE]>,
    tx_queue: spin_lock_net::SpinLock<[Option<u64>; MAX_TX_QUEUE]>,
    rx_count: AtomicUsize,
    tx_count: AtomicUsize,
    next_device_id: AtomicU64,
    next_packet_id: AtomicU64,
    stats: spin_lock_net::SpinLock<[Option<NetDeviceStats>; MAX_NET_DEVICES]>,
    initialized: AtomicU64,
}

// SpinLock para el driver de red
mod spin_lock_net {
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

impl NetDeviceDriver {
    pub const fn new() -> Self {
        Self {
            devices: spin_lock_net::SpinLock::new([None; MAX_NET_DEVICES]),
            device_count: AtomicUsize::new(0),
            rx_queue: spin_lock_net::SpinLock::new([None; MAX_RX_QUEUE]),
            tx_queue: spin_lock_net::SpinLock::new([None; MAX_TX_QUEUE]),
            rx_count: AtomicUsize::new(0),
            tx_count: AtomicUsize::new(0),
            next_device_id: AtomicU64::new(1),
            next_packet_id: AtomicU64::new(1),
            stats: spin_lock_net::SpinLock::new([None; MAX_NET_DEVICES]),
            initialized: AtomicU64::new(0),
        }
    }

    pub unsafe fn init(&self) {
        self.initialized.store(1, Ordering::Release);
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::Acquire) != 0
    }

    /// Registra una nueva interfaz de red.
    pub fn register_device(
        &self,
        device_type: NetDeviceType,
        mac_address: MacAddr,
        ip_address: IpAddr,
        subnet_mask: IpAddr,
        gateway: IpAddr,
        mtu: u16,
    ) -> NetResult<NetDeviceId> {
        if !self.is_initialized() {
            return Err(NetError::NotInitialized);
        }

        let mut devices = self.devices.lock();
        let id = (devices.iter().filter(|d| d.is_some()).count() + 1) as NetDeviceId;

        if id > MAX_NET_DEVICES as NetDeviceId {
            return Err(NetError::OutOfMemory);
        }

        // Encontrar slot libre
        for slot in devices.iter_mut() {
            if slot.is_none() {
                *slot = Some(NetDeviceInfo {
                    id,
                    device_type,
                    mac_address,
                    ip_address,
                    subnet_mask,
                    gateway,
                    mtu,
                    state: NetDeviceState::Up,
                    link_up: true,
                    tx_bytes: 0,
                    rx_bytes: 0,
                });
                self.device_count.fetch_add(1, Ordering::Release);

                // Inicializar estadísticas
                let mut stats = self.stats.lock();
                stats[id as usize - 1] = Some(NetDeviceStats {
                    device_id: id,
                    packets_sent: 0,
                    packets_received: 0,
                    bytes_sent: 0,
                    bytes_received: 0,
                    errors: 0,
                    drops: 0,
                });
                drop(stats);

                return Ok(id);
            }
        }

        Err(NetError::OutOfMemory)
    }

    /// Desregistra una interfaz de red.
    pub fn unregister_device(&self, device_id: NetDeviceId) -> NetResult<()> {
        if !self.is_initialized() {
            return Err(NetError::NotInitialized);
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
            None => Err(NetError::DeviceNotFound),
        }
    }

    /// Obtiene información de un dispositivo de red.
    pub fn get_device_info(&self, device_id: NetDeviceId) -> NetResult<NetDeviceInfo> {
        if !self.is_initialized() {
            return Err(NetError::NotInitialized);
        }

        let devices = self.devices.lock();
        for d in devices.iter() {
            if let Some(info) = d {
                if info.id == device_id {
                    return Ok(*info);
                }
            }
        }
        Err(NetError::DeviceNotFound)
    }

    /// Envía un paquete de red.
    pub fn send_packet(
        &self,
        device_id: NetDeviceId,
        data: &[u8],
    ) -> NetResult<NetOpResult> {
        if !self.is_initialized() {
            return Err(NetError::NotInitialized);
        }

        // Verificar que el dispositivo existe y está activo
        let devices = self.devices.lock();
        let device = devices.iter().find_map(|d| d.as_ref().filter(|d| d.id == device_id));
        let device = match device {
            Some(d) => *d,
            None => return Err(NetError::DeviceNotFound),
        };
        drop(devices);

        if !device.link_up {
            return Err(NetError::DeviceError);
        }

        if data.len() > device.mtu as usize {
            return Err(NetError::InvalidPacket);
        }

        // Crear solicitud de transmisión (solo ID)
        let packet_id = self.next_packet_id.fetch_add(1, Ordering::Relaxed);

        {
            let mut queue = self.tx_queue.lock();
            for slot in queue.iter_mut() {
                if slot.is_none() {
                    *slot = Some(packet_id);
                    self.tx_count.fetch_add(1, Ordering::Release);
                    break;
                }
            }
        }

        // Actualizar estadísticas
        self.update_stats(device_id, false, data.len() as u64, NetOpResult::Success);

        Ok(NetOpResult::Success)
    }

    /// Recibe un paquete de red (simulado).
    pub fn receive_packet(&self, device_id: NetDeviceId, data: &[u8]) -> NetResult<NetOpResult> {
        if !self.is_initialized() {
            return Err(NetError::NotInitialized);
        }

        // Verificar que el dispositivo existe
        let devices = self.devices.lock();
        let device = devices.iter().find_map(|d| d.as_ref().filter(|d| d.id == device_id));
        let _device = match device {
            Some(d) => *d,
            None => return Err(NetError::DeviceNotFound),
        };
        drop(devices);

        if data.len() > 1500 {
            return Err(NetError::InvalidPacket);
        }

        // Crear solicitud de recepción (almacenar device_id en la cola)
        {
            let mut queue = self.rx_queue.lock();
            for slot in queue.iter_mut() {
                if slot.is_none() {
                    *slot = Some(device_id);
                    self.rx_count.fetch_add(1, Ordering::Release);
                    break;
                }
            }
        }

        // Actualizar estadísticas
        self.update_stats(device_id, true, data.len() as u64, NetOpResult::Success);

        Ok(NetOpResult::Success)
    }

    /// Recibe el siguiente paquete de la cola de recepción.
    pub fn recv_packet(&self) -> NetResult<NetPacket> {
        if !self.is_initialized() {
            return Err(NetError::NotInitialized);
        }

        let mut queue = self.rx_queue.lock();
        for slot in queue.iter_mut() {
            if let Some(packet_id) = slot.take() {
                self.rx_count.fetch_sub(1, Ordering::Release);
                // Devolver un paquete con el ID y datos vacíos (simulado)
                return Ok(NetPacket {
                    data: Vec::new(),
                    device_id: packet_id,
                    timestamp_us: time_us(),
                });
            }
        }

        Err(NetError::DeviceNotFound)
    }

    /// Actualiza las estadísticas de un dispositivo.
    fn update_stats(&self, device_id: NetDeviceId, is_rx: bool, bytes: u64, result: NetOpResult) {
        let mut stats = self.stats.lock();
        if let Some(stat) = stats.iter_mut().find_map(|s| s.as_mut().filter(|s| s.device_id == device_id)) {
            if is_rx {
                stat.packets_received += 1;
                stat.bytes_received += bytes;
            } else {
                stat.packets_sent += 1;
                stat.bytes_sent += bytes;
            }

            if result != NetOpResult::Success {
                stat.errors += 1;
            }
        }
    }

    /// Envía todas las solicitudes de transmisión pendientes.
    pub fn flush_tx_queue(&self) -> NetResult<u64> {
        if !self.is_initialized() {
            return Err(NetError::NotInitialized);
        }

        let mut executed = 0u64;
        {
            let mut queue = self.tx_queue.lock();
            for slot in queue.iter_mut() {
                if slot.is_some() {
                    slot.take();
                    executed += 1;
                }
            }
            self.tx_count.fetch_sub(executed as usize, Ordering::Release);
        }

        Ok(executed)
    }

    /// Devuelve el número de dispositivos registrados.
    pub fn device_count(&self) -> usize {
        self.device_count.load(Ordering::Acquire)
    }

    /// Devuelve el número de paquetes pendientes en recepción.
    pub fn rx_pending(&self) -> usize {
        self.rx_count.load(Ordering::Acquire)
    }

    /// Devuelve el número de paquetes pendientes en transmisión.
    pub fn tx_pending(&self) -> usize {
        self.tx_count.load(Ordering::Acquire)
    }

    /// Obtiene las estadísticas de un dispositivo.
    pub fn get_stats(&self, device_id: NetDeviceId) -> NetResult<NetDeviceStats> {
        if !self.is_initialized() {
            return Err(NetError::NotInitialized);
        }

        let stats = self.stats.lock();
        for s in stats.iter() {
            if let Some(st) = s {
                if st.device_id == device_id {
                    return Ok(*st);
                }
            }
        }
        Err(NetError::DeviceNotFound)
    }

    /// Cambia el estado de un dispositivo de red.
    pub fn set_device_state(&self, device_id: NetDeviceId, state: NetDeviceState) -> NetResult<()> {
        if !self.is_initialized() {
            return Err(NetError::NotInitialized);
        }

        let mut devices = self.devices.lock();
        for d in devices.iter_mut() {
            if let Some(info) = d {
                if info.id == device_id {
                    info.state = state;
                    info.link_up = state == NetDeviceState::Up;
                    return Ok(());
                }
            }
        }
        Err(NetError::DeviceNotFound)
    }
}

/// Instancia global del driver de red.
static mut NET_DRIVER: NetDeviceDriver = NetDeviceDriver::new();

/// Devuelve una referencia mutable al driver de red global.
#[allow(static_mut_refs)]
pub fn global_net_driver() -> &'static mut NetDeviceDriver {
    unsafe { &mut NET_DRIVER }
}

/// Tiempo actual en microsegundos.
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
    fn net_device_types() {
        assert_eq!(NetDeviceType::Unknown as u8, 0);
        assert_eq!(NetDeviceType::Ethernet as u8, 1);
        assert_eq!(NetDeviceType::WiFi as u8, 2);
        assert_eq!(NetDeviceType::Virtual as u8, 3);
        assert_eq!(NetDeviceType::Loopback as u8, 4);
    }

    #[test]
    fn net_device_states() {
        assert_eq!(NetDeviceState::Down as u8, 0);
        assert_eq!(NetDeviceState::Up as u8, 1);
        assert_eq!(NetDeviceState::Connecting as u8, 2);
        assert_eq!(NetDeviceState::Disconnected as u8, 3);
        assert_eq!(NetDeviceState::Error as u8, 4);
    }

    #[test]
    fn ether_types() {
        assert_eq!(EtherType::IPv4 as u16, 0x0800);
        assert_eq!(EtherType::ARP as u16, 0x0806);
        assert_eq!(EtherType::IPv6 as u16, 0x86DD);
    }

    #[test]
    fn ip_protocols() {
        assert_eq!(IpProtocol::ICMP as u8, 1);
        assert_eq!(IpProtocol::TCP as u8, 6);
        assert_eq!(IpProtocol::UDP as u8, 17);
    }

    #[test]
    fn net_op_results() {
        assert_eq!(NetOpResult::Success as u8, 0);
        assert_eq!(NetOpResult::Error as u8, 1);
        assert_eq!(NetOpResult::DeviceNotFound as u8, 2);
        assert_eq!(NetOpResult::InvalidAddress as u8, 3);
        assert_eq!(NetOpResult::InvalidPacket as u8, 4);
    }

    #[test]
    fn block_request_status() {
        // RequestStatus no existe en el módulo de red - probado en block driver
    }

    #[test]
    fn driver_new() {
        let driver = NetDeviceDriver::new();
        assert!(!driver.is_initialized());
        assert_eq!(driver.device_count(), 0);
        assert_eq!(driver.rx_pending(), 0);
        assert_eq!(driver.tx_pending(), 0);
    }

    #[test]
    fn driver_init() {
        let driver = NetDeviceDriver::new();
        unsafe { driver.init(); }
        assert!(driver.is_initialized());
    }

    #[test]
    fn register_device() {
        let driver = NetDeviceDriver::new();
        unsafe { driver.init(); }

        let mac = MacAddr { bytes: [0x00, 0x11, 0x22, 0x33, 0x44, 0x55] };
        let ip = IpAddr { bytes: [192, 168, 1, 10] };
        let mask = IpAddr { bytes: [255, 255, 255, 0] };
        let gateway = IpAddr { bytes: [192, 168, 1, 1] };

        let id = driver.register_device(
            NetDeviceType::Ethernet,
            mac,
            ip,
            mask,
            gateway,
            1500,
        ).unwrap();

        assert_eq!(id, 1);
        assert_eq!(driver.device_count(), 1);

        let info = driver.get_device_info(id).unwrap();
        assert_eq!(info.device_type, NetDeviceType::Ethernet);
        assert_eq!(info.mac_address, mac);
        assert_eq!(info.ip_address, ip);
        assert_eq!(info.mtu, 1500);
        assert_eq!(info.state, NetDeviceState::Up);
        assert!(info.link_up);
    }

    #[test]
    fn register_multiple_devices() {
        let driver = NetDeviceDriver::new();
        unsafe { driver.init(); }

        let mac1 = MacAddr { bytes: [0x00, 0x00, 0x00, 0x00, 0x00, 0x01] };
        let mac2 = MacAddr { bytes: [0x00, 0x00, 0x00, 0x00, 0x00, 0x02] };
        let ip1 = IpAddr { bytes: [10, 0, 0, 1] };
        let ip2 = IpAddr { bytes: [10, 0, 0, 2] };
        let mask = IpAddr { bytes: [255, 0, 0, 0] };
        let gateway = IpAddr { bytes: [10, 0, 0, 254] };

        driver.register_device(NetDeviceType::Ethernet, mac1, ip1, mask, gateway, 1500).unwrap();
        driver.register_device(NetDeviceType::WiFi, mac2, ip2, mask, gateway, 1500).unwrap();

        assert_eq!(driver.device_count(), 2);
    }

    #[test]
    fn send_receive_packet() {
        let driver = NetDeviceDriver::new();
        unsafe { driver.init(); }

        let mac = MacAddr { bytes: [0x00, 0x11, 0x22, 0x33, 0x44, 0x55] };
        let ip = IpAddr { bytes: [192, 168, 1, 10] };
        let mask = IpAddr { bytes: [255, 255, 255, 0] };
        let gateway = IpAddr { bytes: [192, 168, 1, 1] };

        let id = driver.register_device(
            NetDeviceType::Ethernet,
            mac,
            ip,
            mask,
            gateway,
            1500,
        ).unwrap();

        let packet_data = b"Hello, network!";
        let result = driver.send_packet(id, packet_data);
        assert_eq!(result, Ok(NetOpResult::Success));

        // Recibir un paquete
        let rx_data = b"Response from server";
        let result = driver.receive_packet(id, rx_data);
        assert_eq!(result, Ok(NetOpResult::Success));

        // Recibir el paquete de la cola - verificar device_id correcto
        let received = driver.recv_packet().unwrap();
        assert_eq!(received.device_id, id);
    }

    #[test]
    fn send_invalid_packet_size() {
        let driver = NetDeviceDriver::new();
        unsafe { driver.init(); }

        let mac = MacAddr { bytes: [0x00, 0x11, 0x22, 0x33, 0x44, 0x55] };
        let ip = IpAddr { bytes: [192, 168, 1, 10] };
        let mask = IpAddr { bytes: [255, 255, 255, 0] };
        let gateway = IpAddr { bytes: [192, 168, 1, 1] };

        let id = driver.register_device(
            NetDeviceType::Ethernet,
            mac,
            ip,
            mask,
            gateway,
            100, // MTU muy pequeño
        ).unwrap();

        let large_packet = [0u8; 2000];
        let result = driver.send_packet(id, &large_packet);
        assert_eq!(result, Err(NetError::InvalidPacket));
    }

    #[test]
    fn device_not_found() {
        let driver = NetDeviceDriver::new();
        unsafe { driver.init(); }

        let mut buf = [0u8; 512];
        let result = driver.send_packet(999, &buf);
        assert_eq!(result, Err(NetError::DeviceNotFound));

        let result = driver.get_device_info(999);
        assert_eq!(result, Err(NetError::DeviceNotFound));
    }

    #[test]
    fn unregister_device() {
        let driver = NetDeviceDriver::new();
        unsafe { driver.init(); }

        let mac = MacAddr { bytes: [0x00, 0x11, 0x22, 0x33, 0x44, 0x55] };
        let ip = IpAddr { bytes: [192, 168, 1, 10] };
        let mask = IpAddr { bytes: [255, 255, 255, 0] };
        let gateway = IpAddr { bytes: [192, 168, 1, 1] };

        let id = driver.register_device(
            NetDeviceType::Ethernet,
            mac,
            ip,
            mask,
            gateway,
            1500,
        ).unwrap();

        assert_eq!(driver.device_count(), 1);

        driver.unregister_device(id).unwrap();
        assert_eq!(driver.device_count(), 0);

        let result = driver.get_device_info(id);
        assert_eq!(result, Err(NetError::DeviceNotFound));
    }

    #[test]
    fn flush_tx_queue() {
        let driver = NetDeviceDriver::new();
        unsafe { driver.init(); }

        let mac = MacAddr { bytes: [0x00, 0x11, 0x22, 0x33, 0x44, 0x55] };
        let ip = IpAddr { bytes: [192, 168, 1, 10] };
        let mask = IpAddr { bytes: [255, 255, 255, 0] };
        let gateway = IpAddr { bytes: [192, 168, 1, 1] };

        let id = driver.register_device(
            NetDeviceType::Ethernet,
            mac,
            ip,
            mask,
            gateway,
            1500,
        ).unwrap();

        let packet1 = b"Packet 1";
        let packet2 = b"Packet 2";

        driver.send_packet(id, packet1).unwrap();
        driver.send_packet(id, packet2).unwrap();

        assert_eq!(driver.tx_pending(), 2);

        let executed = driver.flush_tx_queue().unwrap();
        assert_eq!(executed, 2);
        assert_eq!(driver.tx_pending(), 0);
    }

    #[test]
    fn get_stats() {
        let driver = NetDeviceDriver::new();
        unsafe { driver.init(); }

        let mac = MacAddr { bytes: [0x00, 0x11, 0x22, 0x33, 0x44, 0x55] };
        let ip = IpAddr { bytes: [192, 168, 1, 10] };
        let mask = IpAddr { bytes: [255, 255, 255, 0] };
        let gateway = IpAddr { bytes: [192, 168, 1, 1] };

        let id = driver.register_device(
            NetDeviceType::Ethernet,
            mac,
            ip,
            mask,
            gateway,
            1500,
        ).unwrap();

        let packet = b"Test packet";
        driver.send_packet(id, packet).unwrap();
        driver.receive_packet(id, packet).unwrap();

        let stats = driver.get_stats(id).unwrap();
        assert_eq!(stats.device_id, id);
        assert_eq!(stats.packets_sent, 1);
        assert_eq!(stats.packets_received, 1);
        assert_eq!(stats.bytes_sent, packet.len() as u64);
        assert_eq!(stats.bytes_received, packet.len() as u64);
    }

    #[test]
    fn set_device_state() {
        let driver = NetDeviceDriver::new();
        unsafe { driver.init(); }

        let mac = MacAddr { bytes: [0x00, 0x11, 0x22, 0x33, 0x44, 0x55] };
        let ip = IpAddr { bytes: [192, 168, 1, 10] };
        let mask = IpAddr { bytes: [255, 255, 255, 0] };
        let gateway = IpAddr { bytes: [192, 168, 1, 1] };

        let id = driver.register_device(
            NetDeviceType::Ethernet,
            mac,
            ip,
            mask,
            gateway,
            1500,
        ).unwrap();

        assert!(driver.get_device_info(id).unwrap().link_up);

        driver.set_device_state(id, NetDeviceState::Down).unwrap();
        let info = driver.get_device_info(id).unwrap();
        assert_eq!(info.state, NetDeviceState::Down);
        assert!(!info.link_up);

        driver.set_device_state(id, NetDeviceState::Up).unwrap();
        let info = driver.get_device_info(id).unwrap();
        assert_eq!(info.state, NetDeviceState::Up);
        assert!(info.link_up);
    }

    #[test]
    fn not_initialized() {
        let driver = NetDeviceDriver::new();
        let mac = MacAddr { bytes: [0x00, 0x00, 0x00, 0x00, 0x00, 0x01] };
        let ip = IpAddr { bytes: [10, 0, 0, 1] };
        let mask = IpAddr { bytes: [255, 0, 0, 0] };
        let gateway = IpAddr { bytes: [10, 0, 0, 254] };

        assert_eq!(
            driver.register_device(NetDeviceType::Ethernet, mac, ip, mask, gateway, 1500),
            Err(NetError::NotInitialized)
        );
        assert_eq!(driver.device_count(), 0);
    }
}
