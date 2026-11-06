//! TCP/IP Stack mínimo — ARP, ICMP, IPv4.
//!
//! Proporciona:
//! - ARP: resolución de direcciones IP a MAC
//! - ICMP: echo request/reply (ping)
//! - IPv4: encabezado básico y enrutamiento
//! - Integración con VirtIO-Net para envío/recepción de paquetes

use crate::virtio_net::global_virtio_net;
use core::convert::TryInto;
use core::sync::atomic::{AtomicU64, Ordering};
use alloc::vec::Vec;

// ============================================================================
// Configuración de red del sistema
// ============================================================================

/// Dirección IP del sistema (10.0.0.2 por defecto).
pub static mut IP_ADDR: [u8; 4] = [10, 0, 0, 2];
/// Máscara de subred (255.255.255.0).
pub static mut SUBNET_MASK: [u8; 4] = [255, 255, 255, 0];
/// Puerta de enlace (10.0.0.1).
pub static mut GATEWAY: [u8; 4] = [10, 0, 0, 1];

/// Estado del stack de red.
pub static mut NET_INIT: AtomicU64 = AtomicU64::new(0);

// ============================================================================
// Encabezados de protocolo
// ============================================================================

/// Encabezado Ethernet (14 bytes).
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct EthHeader {
    dst_mac: [u8; 6],
    src_mac: [u8; 6],
    ethertype: u16, // 0x0800 = IPv4, 0x0806 = ARP
}

const ETHERTYPE_IPV4: u16 = 0x0800;
const ETHERTYPE_ARP: u16 = 0x0806;

/// Encabezado ARP (28 bytes para Ethernet/IPv4).
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct ArpHeader {
    htype: u16,      // 1 = Ethernet
    ptype: u16,      // 0x0800 = IPv4
    hlen: u8,        // 6
    plen: u8,        // 4
    opcode: u16,     // 1 = request, 2 = reply
    sender_mac: [u8; 6],
    sender_ip: [u8; 4],
    target_mac: [u8; 6],
    target_ip: [u8; 4],
}

const ARP_OP_REQUEST: u16 = 1;
const ARP_OP_REPLY: u16 = 2;

/// Encabezado IPv4 (mínimo 20 bytes).
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct IpHeader {
    version_ihl: u8,   // 0x45 = IPv4, 20 bytes header
    dscp_ecn: u8,      // 0x00
    total_len: u16,    // longitud total en bytes
    identification: u16,
    flags_fragment: u16,
    ttl: u8,           // 64
    protocol: u8,      // 1 = ICMP, 6 = TCP, 17 = UDP
    header_checksum: u16,
    src_ip: [u8; 4],
    dst_ip: [u8; 4],
}

const IP_PROTO_ICMP: u8 = 1;

/// Encabezado ICMP (8 bytes mínimo).
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct IcmpHeader {
    r#type: u8,        // 8 = echo request, 0 = echo reply
    code: u8,          // 0
    checksum: u16,
    id: u16,
    sequence: u16,
}

const ICMP_ECHO_REQUEST: u8 = 8;
const ICMP_ECHO_REPLY: u8 = 0;

// ============================================================================
// Funciones de utilidad
// ============================================================================

/// Calcula el checksum de Internet (16-bit ones' complement sum).
fn checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;
    while i + 1 < data.len() {
        sum += ((data[i] as u32) << 8) | (data[i + 1] as u32);
        i += 2;
    }
    if i < data.len() {
        sum += (data[i] as u32) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

/// Obtiene la dirección MAC del dispositivo VirtIO-Net.
fn get_our_mac() -> [u8; 6] {
    let net = unsafe { global_virtio_net() };
    net.info().mac_address
}

/// Obtiene la dirección IP del sistema.
fn get_our_ip() -> [u8; 4] {
    unsafe { IP_ADDR }
}

/// Construye un encabezado Ethernet.
fn make_eth_header(dst_mac: &[u8; 6], src_mac: &[u8; 6], ethertype: u16) -> EthHeader {
    EthHeader {
        dst_mac: *dst_mac,
        src_mac: *src_mac,
        ethertype: ethertype.to_be(),
    }
}

/// Construye un encabezado ARP request.
fn make_arp_request(target_ip: &[u8; 4], our_mac: &[u8; 6], our_ip: &[u8; 4]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(28);
    let hdr = ArpHeader {
        htype: 1u16.to_be(),
        ptype: 0x0800u16.to_be(),
        hlen: 6,
        plen: 4,
        opcode: ARP_OP_REQUEST.to_be(),
        sender_mac: *our_mac,
        sender_ip: *our_ip,
        target_mac: [0; 6],
        target_ip: *target_ip,
    };
    unsafe {
        let ptr = &hdr as *const ArpHeader as *const u8;
        for i in 0..28 {
            buf.push(read_volatile(ptr.add(i)));
        }
    }
    buf
}

/// Construye un encabezado ARP reply.
fn make_arp_reply(target_mac: &[u8; 6], target_ip: &[u8; 4], our_mac: &[u8; 6], our_ip: &[u8; 4]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(28);
    let hdr = ArpHeader {
        htype: 1u16.to_be(),
        ptype: 0x0800u16.to_be(),
        hlen: 6,
        plen: 4,
        opcode: ARP_OP_REPLY.to_be(),
        sender_mac: *our_mac,
        sender_ip: *our_ip,
        target_mac: *target_mac,
        target_ip: *target_ip,
    };
    unsafe {
        let ptr = &hdr as *const ArpHeader as *const u8;
        for i in 0..28 {
            buf.push(read_volatile(ptr.add(i)));
        }
    }
    buf
}

/// Construye un encabezado IPv4 en un buffer.
fn make_ip_header(buf: &mut [u8; 20], protocol: u8, src_ip: &[u8; 4], dst_ip: &[u8; 4], payload_len: u16) {
    let total_len = (20 + payload_len as u16).to_be();
    let hdr = IpHeader {
        version_ihl: 0x45,
        dscp_ecn: 0x00,
        total_len,
        identification: 0x0001,
        flags_fragment: 0x0000,
        ttl: 64,
        protocol,
        header_checksum: 0x0000,
        src_ip: *src_ip,
        dst_ip: *dst_ip,
    };
    unsafe {
        let ptr = &hdr as *const IpHeader as *const u8;
        for i in 0..20 {
            buf[i] = read_volatile(ptr.add(i));
        }
    }
    // Calcular checksum del encabezado IP
    let checksum_val = checksum(buf);
    buf[10] = (checksum_val >> 8) as u8;
    buf[11] = (checksum_val & 0xFF) as u8;
}

/// Construye un encabezado ICMP echo.
fn make_icmp_echo(r#type: u8, id: u16, sequence: u16, data: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(8 + data.len());
    let hdr = IcmpHeader {
        r#type,
        code: 0,
        checksum: 0x0000,
        id: id.to_be(),
        sequence: sequence.to_be(),
    };
    unsafe {
        let ptr = &hdr as *const IcmpHeader as *const u8;
        for i in 0..8 {
            buf.push(read_volatile(ptr.add(i)));
        }
    }
    buf.extend_from_slice(data);
    // Calcular checksum ICMP (encabezado + datos)
    let checksum_val = checksum(&buf);
    buf[2] = (checksum_val >> 8) as u8;
    buf[3] = (checksum_val & 0xFF) as u8;
    buf
}

/// Lee un byte de forma volátil.
unsafe fn read_volatile(ptr: *const u8) -> u8 {
    core::ptr::read_volatile(ptr)
}

// ============================================================================
// Envío de paquetes
// ============================================================================

/// Envía un paquete Ethernet crudo.
fn eth_send(dst_mac: &[u8; 6], ethertype: u16, payload: &[u8]) -> Result<(), &'static str> {
    let net = unsafe { global_virtio_net() };
    if net.state() != crate::virtio_net::VirtioNetState::Ready {
        return Err("Red no lista");
    }

    let our_mac = get_our_mac();
    let eth_hdr = make_eth_header(dst_mac, &our_mac, ethertype);

    let mut packet = Vec::with_capacity(14 + payload.len());
    unsafe {
        let ptr = &eth_hdr as *const EthHeader as *const u8;
        for i in 0..14 {
            packet.push(read_volatile(ptr.add(i)));
        }
    }
    packet.extend_from_slice(payload);

    unsafe { net.send_packet(&packet) }
}

/// Envía un paquete IPv4.
pub fn ip_send(protocol: u8, dst_ip: &[u8; 4], payload: &[u8]) -> Result<(), &'static str> {
    let our_ip = get_our_ip();
    let dst_mac = dst_mac_for_ip(dst_ip)?;
    let eth_hdr = make_eth_header(&dst_mac, &get_our_mac(), ETHERTYPE_IPV4);
    let mut ip_hdr_buf = [0u8; 20];
    make_ip_header(&mut ip_hdr_buf, protocol, &our_ip, dst_ip, payload.len() as u16);

    let mut packet = Vec::with_capacity(14 + 20 + payload.len());
    unsafe {
        let ptr = &eth_hdr as *const EthHeader as *const u8;
        for i in 0..14 {
            packet.push(read_volatile(ptr.add(i)));
        }
    }
    unsafe {
        let ptr = ip_hdr_buf.as_ptr() as *const u8;
        for i in 0..20 {
            packet.push(read_volatile(ptr.add(i)));
        }
    }
    packet.extend_from_slice(payload);

    let net = unsafe { global_virtio_net() };
    unsafe { net.send_packet(&packet) }
}

/// Obtiene la MAC de destino para una IP (ARP si es necesario).
fn dst_mac_for_ip(dst_ip: &[u8; 4]) -> Result<[u8; 6], &'static str> {
    // Si está en la misma subred, usar la IP directamente
    let our_ip = get_our_ip();
    let mask = unsafe { SUBNET_MASK };
    let our_net = [our_ip[0] & mask[0], our_ip[1] & mask[1], our_ip[2] & mask[2], our_ip[3] & mask[3]];
    let dst_net = [dst_ip[0] & mask[0], dst_ip[1] & mask[1], dst_ip[2] & mask[2], dst_ip[3] & mask[3]];

    let target_ip = if our_net == dst_net {
        *dst_ip
    } else {
        unsafe { GATEWAY }
    };

    // Enviar ARP request para obtener la MAC
    let our_mac = get_our_mac();
    let arp_payload = make_arp_request(&target_ip, &our_mac, &our_ip);
    let eth_hdr = make_eth_header(&[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF], &our_mac, ETHERTYPE_ARP);

    let mut packet = Vec::with_capacity(14 + 28);
    unsafe {
        let ptr = &eth_hdr as *const EthHeader as *const u8;
        for i in 0..14 {
            packet.push(read_volatile(ptr.add(i)));
        }
    }
    packet.extend_from_slice(&arp_payload);

    let net = unsafe { global_virtio_net() };
    unsafe { net.send_packet(&packet) }?;

    // Esperar ARP reply (polling simple)
    for _ in 0..10000 {
        if let Some(pkt) = unsafe { net.receive_packet() } {
            if pkt.len() >= 14 + 28 {
                // Verificar que es ARP reply
                let opcode = u16::from_be_bytes([pkt[20], pkt[21]]);
                if opcode == ARP_OP_REPLY {
                    let mut src_mac = [0u8; 6];
                    src_mac.copy_from_slice(&pkt[22..28]);
                    return Ok(src_mac);
                }
            }
        }
        for _ in 0..100 {
            core::hint::spin_loop();
        }
    }

    Err("ARP timeout")
}

/// Envía un ICMP echo request (ping).
pub fn icmp_echo_request(dest_ip: &[u8; 4], data: &[u8]) -> Result<(), &'static str> {
    let icmp_payload = make_icmp_echo(ICMP_ECHO_REQUEST, 0x1234, 1, data);
    ip_send(IP_PROTO_ICMP, dest_ip, &icmp_payload)
}

// ============================================================================
// Recepción de paquetes
// ============================================================================

/// Procesa un paquete recibido (llamado desde el bucle principal o interrupción).
pub fn net_receive() -> Result<(), &'static str> {
    let net = unsafe { global_virtio_net() };
    loop {
        match unsafe { net.receive_packet() } {
            Some(pkt) => {
                if pkt.len() < 14 {
                    continue;
                }
                // Verificar ethertype
                let ethertype = u16::from_be_bytes([pkt[12], pkt[13]]);
                match ethertype {
                    ETHERTYPE_ARP => {
                        handle_arp(&pkt)?;
                    }
                    ETHERTYPE_IPV4 => {
                        handle_ip(&pkt)?;
                    }
                    _ => {}
                }
            }
            None => break,
        }
    }
    Ok(())
}

/// Maneja un paquete ARP recibido.
fn handle_arp(pkt: &[u8]) -> Result<(), &'static str> {
    if pkt.len() < 14 + 28 {
        return Ok(());
    }
    let opcode = u16::from_be_bytes([pkt[20], pkt[21]]);
    if opcode != ARP_OP_REQUEST {
        return Ok(());
    }

    // Verificar que el ARP request es para nuestra IP
    let target_ip = &pkt[24..28];
    let our_ip = get_our_ip();
    if target_ip != our_ip {
        return Ok(());
    }

    let sender_mac: [u8; 6] = pkt[22..28].try_into().unwrap();
    let sender_ip: [u8; 4] = pkt[28..32].try_into().unwrap();

    // Enviar ARP reply
    let our_mac = get_our_mac();
    let our_ip_val = get_our_ip();
    let arp_reply = make_arp_reply(&sender_mac, &sender_ip, &our_mac, &our_ip_val);
    let eth_hdr = make_eth_header(&sender_mac, &our_mac, ETHERTYPE_ARP);

    let mut packet = Vec::with_capacity(14 + 28);
    unsafe {
        let ptr = &eth_hdr as *const EthHeader as *const u8;
        for i in 0..14 {
            packet.push(read_volatile(ptr.add(i)));
        }
    }
    packet.extend_from_slice(&arp_reply);

    let net = unsafe { global_virtio_net() };
    unsafe { net.send_packet(&packet) }
}

/// Maneja un paquete IPv4 recibido.
fn handle_ip(pkt: &[u8]) -> Result<(), &'static str> {
    if pkt.len() < 14 + 20 {
        return Ok(());
    }
    let protocol = pkt[23];
    let dst_ip: [u8; 4] = pkt[30..34].try_into().unwrap();
    let our_ip = get_our_ip();

    // Solo procesar paquetes dirigidos a nosotros
    if dst_ip != our_ip {
        return Ok(());
    }

    match protocol {
        IP_PROTO_ICMP => handle_icmp(&pkt[14..]),
        _ => Ok(()),
    }
}

/// Maneja un paquete ICMP recibido.
fn handle_icmp(pkt: &[u8]) -> Result<(), &'static str> {
    if pkt.len() < 8 {
        return Ok(());
    }
    let r#type = pkt[0];
    if r#type != ICMP_ECHO_REQUEST {
        return Ok(());
    }

    let id = u16::from_be_bytes([pkt[4], pkt[5]]);
    let sequence = u16::from_be_bytes([pkt[6], pkt[7]]);
    let data = &pkt[8..];

    // Responder con ICMP echo reply
    let icmp_reply = make_icmp_echo(ICMP_ECHO_REPLY, id, sequence, data);
    let src_ip: [u8; 4] = pkt[26..30].try_into().unwrap(); // src_ip del IP header
    let our_ip = get_our_ip();

    ip_send(IP_PROTO_ICMP, &src_ip, &icmp_reply)
}

// ============================================================================
// Inicialización
// ============================================================================

/// Inicializa el stack de red.
pub unsafe fn init(ip: [u8; 4], subnet: [u8; 4], gateway: [u8; 4]) {
    IP_ADDR = ip;
    SUBNET_MASK = subnet;
    GATEWAY = gateway;
    NET_INIT.store(1, Ordering::Release);
}

/// Verifica si el stack de red está inicializado.
pub fn is_initialized() -> bool {
    unsafe { NET_INIT.load(Ordering::Acquire) != 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_checksum() {
        let data = [0x45, 0x00, 0x00, 0x28];
        let cs = checksum(&data);
        // Verificación básica: el checksum no debe ser 0 para datos no triviales
        assert_ne!(cs, 0x0000);
    }

    #[test]
    fn test_icmp_echo_build() {
        let pkt = make_icmp_echo(ICMP_ECHO_REQUEST, 0x1234, 1, b"test");
        assert_eq!(pkt[0], ICMP_ECHO_REQUEST);
        assert_eq!(pkt[4], 0x12);
        assert_eq!(pkt[5], 0x34);
        assert!(pkt.len() >= 12); // 8 header + 4 data
    }
}
