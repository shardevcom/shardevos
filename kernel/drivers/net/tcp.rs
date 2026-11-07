//! TCP Stack — Protocolo de Control de Transmisión para shardevos.
//!
//! Proporciona:
//! - Cabecera TCP y handshake de 3 vías (SYN, SYN-ACK, ACK)
//! - Envío y recepción de segmentos TCP sobre VirtIO-Net
//! - Tabla de conexiones TCP activas
//! - Integración con syscalls NET_CONNECT/NET_SEND/NET_RECV

use crate::net::stack::{icmp_echo_request, ip_send};
use crate::virtio_net::global_virtio_net;
use core::sync::atomic::{AtomicU64, Ordering};
use alloc::vec::Vec;

// ============================================================================
// Cabecera TCP (20 bytes mínimo)
// ============================================================================

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct TcpHeader {
    pub src_port: u16,
    pub dst_port: u16,
    pub seq_num: u32,
    pub ack_num: u32,
    pub data_offset_reserved: u8,  // bits 4-7: data offset, bits 0-3: reserved
    pub flags: u8,                 // FIN=0x01, SYN=0x02, RST=0x04, PSH=0x08, ACK=0x10, URG=0x20
    pub window_size: u16,
    pub checksum: u16,
    pub urgent_ptr: u16,
}

pub const TCP_FLAG_FIN: u8 = 0x01;
pub const TCP_FLAG_SYN: u8 = 0x02;
pub const TCP_FLAG_RST: u8 = 0x04;
pub const TCP_FLAG_PSH: u8 = 0x08;
pub const TCP_FLAG_ACK: u8 = 0x10;
pub const TCP_FLAG_URG: u8 = 0x20;

const IP_PROTO_TCP: u8 = 6;

/// Estado de una conexión TCP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TcpState {
    Closed = 0,
    Listen = 1,
    SynSent = 2,
    SynReceived = 3,
    Established = 4,
    FinWait1 = 5,
    FinWait2 = 6,
    CloseWait = 7,
    Closing = 8,
    LastAck = 9,
    TimeWait = 10,
}

/// Entrada de conexión TCP.
#[derive(Debug, Clone, Copy)]
pub struct TcpConnection {
    pub state: TcpState,
    pub local_ip: [u8; 4],
    pub local_port: u16,
    pub remote_ip: [u8; 4],
    pub remote_port: u16,
    pub seq_num: u32,
    pub ack_num: u32,
    pub window_size: u16,
}

/// Tabla global de conexiones TCP.
const MAX_TCP_CONNECTIONS: usize = 16;

static mut TCP_CONNECTIONS: [TcpConnection; MAX_TCP_CONNECTIONS] = [TcpConnection {
    state: TcpState::Closed,
    local_ip: [0; 4],
    local_port: 0,
    remote_ip: [0; 4],
    remote_port: 0,
    seq_num: 0,
    ack_num: 0,
    window_size: 0,
}; MAX_TCP_CONNECTIONS];

static NEXT_TCP_ID: AtomicU64 = AtomicU64::new(1);

static mut TCP_INIT: AtomicU64 = AtomicU64::new(0);

// ============================================================================
// Funciones de utilidad TCP
// ============================================================================

/// Calcula el checksum pseudo-header + TCP.
fn tcp_checksum(src_ip: &[u8; 4], dst_ip: &[u8; 4], tcp_header: &[u8], payload: &[u8]) -> u16 {
    let mut sum: u32 = 0;

    // Pseudo-header: src_ip (4), dst_ip (4), zero (1), protocol (1), tcp_length (2)
    for i in 0..4 {
        sum += src_ip[i] as u32;
        sum += dst_ip[i] as u32;
    }
    sum += 0; // zero
    sum += IP_PROTO_TCP as u32;
    sum += (tcp_header.len() + payload.len()) as u32;

    // TCP header + payload
    let mut i = 0;
    while i + 1 < tcp_header.len() {
        sum += ((tcp_header[i] as u32) << 8) | (tcp_header[i + 1] as u32);
        i += 2;
    }
    if i < tcp_header.len() {
        sum += (tcp_header[i] as u32) << 8;
    }

    i = 0;
    while i + 1 < payload.len() {
        sum += ((payload[i] as u32) << 8) | (payload[i + 1] as u32);
        i += 2;
    }
    if i < payload.len() {
        sum += (payload[i] as u32) << 8;
    }

    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

/// Construye un encabezado TCP.
pub fn make_tcp_header(src_port: u16, dst_port: u16, seq_num: u32, ack_num: u32, flags: u8, window_size: u16) -> TcpHeader {
    TcpHeader {
        src_port: src_port.to_be(),
        dst_port: dst_port.to_be(),
        seq_num: seq_num.to_be(),
        ack_num: ack_num.to_be(),
        data_offset_reserved: 0x50, // 5 * 4 = 20 bytes header
        flags,
        window_size: window_size.to_be(),
        checksum: 0,
        urgent_ptr: 0,
    }
}

/// Obtiene el siguiente número de secuencia.
fn next_seq_num() -> u32 {
    static mut SEQ: u32 = 1000;
    unsafe {
        SEQ += 1;
        SEQ
    }
}

/// Encuentra una conexión TCP por tupla (local_ip, local_port, remote_ip, remote_port).
fn find_tcp_connection(local_ip: &[u8; 4], local_port: u16, remote_ip: &[u8; 4], remote_port: u16) -> Option<usize> {
    unsafe {
        for i in 0..MAX_TCP_CONNECTIONS {
            let c = TCP_CONNECTIONS[i];
            if c.state != TcpState::Closed
                && c.local_ip == *local_ip
                && c.local_port == local_port
                && c.remote_ip == *remote_ip
                && c.remote_port == remote_port
            {
                return Some(i);
            }
        }
    }
    None
}

/// Encuentra una conexión libre.
fn find_free_tcp_connection() -> Option<usize> {
    unsafe {
        for i in 0..MAX_TCP_CONNECTIONS {
            if TCP_CONNECTIONS[i].state == TcpState::Closed {
                return Some(i);
            }
        }
    }
    None
}

/// Envía un segmento TCP.
pub fn tcp_send_segment(src_ip: &[u8; 4], dst_ip: &[u8; 4], header: &mut TcpHeader, payload: &[u8]) -> Result<(), &'static str> {
    let net = unsafe { global_virtio_net() };
    if net.state() != crate::virtio_net::VirtioNetState::Ready {
        return Err("Red no lista");
    }

    // Calcular checksum TCP con pseudo-header
    let mut tcp_bytes = [0u8; 20];
    unsafe {
        let ptr = header as *const TcpHeader as *const u8;
        for i in 0..20 {
            tcp_bytes[i] = core::ptr::read_volatile(ptr.add(i));
        }
    }
    let checksum_val = tcp_checksum(src_ip, dst_ip, &tcp_bytes, payload);

    // Construir paquete completo: IP header + TCP header + payload
    let mut packet = Vec::with_capacity(20 + 20 + payload.len());

    // IP header
    let ip_hdr_buf = [0u8; 20];
    // Usamos una función auxiliar para construir IP header
    // Por simplicidad, construimos manualmente aquí
    let total_len = (20 + 20 + payload.len()) as u16;
    let ip_hdr = [
        0x45, // version + ihl
        0x00, // dscp + ecn
        (total_len >> 8) as u8, (total_len & 0xFF) as u8, // total length
        0x00, 0x01, // identification
        0x00, 0x00, // flags + fragment
        0x40, // ttl = 64
        IP_PROTO_TCP, // protocol
        0x00, 0x00, // checksum (se calcula abajo)
        src_ip[0], src_ip[1], src_ip[2], src_ip[3],
        dst_ip[0], dst_ip[1], dst_ip[2], dst_ip[3],
    ];
    packet.extend_from_slice(&ip_hdr);

    // TCP header
    unsafe {
        let ptr = header as *const TcpHeader as *const u8;
        for i in 0..20 {
            packet.push(core::ptr::read_volatile(ptr.add(i)));
        }
    }
    // Actualizar checksum en el paquete (offset 16+16+10 = 42 para TCP checksum)
    packet[16 + 16 + 10] = (checksum_val >> 8) as u8;
    packet[16 + 16 + 11] = (checksum_val & 0xFF) as u8;

    // Payload
    packet.extend_from_slice(payload);

    // Calcular checksum IP
    let ip_checksum = crate::net::stack::checksum(&packet[0..20]);
    packet[10] = (ip_checksum >> 8) as u8;
    packet[11] = (ip_checksum & 0xFF) as u8;

    unsafe { net.send_packet(&packet) }
}

/// Inicializa el stack TCP.
pub unsafe fn init() {
    TCP_INIT.store(1, Ordering::Release);
}

/// Verifica si el stack TCP está inicializado.
pub fn is_initialized() -> bool {
    unsafe { TCP_INIT.load(Ordering::Acquire) != 0 }
}

/// Envía un SYN para iniciar una conexión TCP.
pub fn tcp_connect(src_ip: &[u8; 4], src_port: u16, dst_ip: &[u8; 4], dst_port: u16) -> Result<u16, &'static str> {
    let idx = find_free_tcp_connection().ok_or("No hay conexiones TCP disponibles")?;

    let seq_num = next_seq_num();

    let mut header = make_tcp_header(src_port, dst_port, seq_num, 0, TCP_FLAG_SYN, 8192);

    tcp_send_segment(src_ip, dst_ip, &mut header, &[])?;

    // Almacenar la conexión
    unsafe {
        TCP_CONNECTIONS[idx] = TcpConnection {
            state: TcpState::SynSent,
            local_ip: *src_ip,
            local_port: src_port,
            remote_ip: *dst_ip,
            remote_port: dst_port,
            seq_num: seq_num + 1,
            ack_num: 0,
            window_size: 8192,
        };
    }

    Ok(src_port)
}

/// Procesa un segmento TCP entrante.
pub fn tcp_receive(src_ip: &[u8; 4], dst_ip: &[u8; 4], header: &TcpHeader, payload: &[u8]) {
    let src_port = header.src_port;
    let dst_port = header.dst_port;
    let seq_num = header.seq_num;
    let ack_num = header.ack_num;
    let flags = header.flags;

    // Buscar conexión existente
    let conn_idx = find_tcp_connection(dst_ip, dst_port, src_ip, src_port);

    if flags & TCP_FLAG_SYN != 0 && flags & TCP_FLAG_ACK == 0 {
        // SYN entrante - posible nueva conexión o SYN-ACK
        if let Some(idx) = conn_idx {
            unsafe {
                let c = &mut TCP_CONNECTIONS[idx];
                if c.state == TcpState::SynSent {
                    // Recibimos SYN-ACK en respuesta a nuestro SYN
                    c.state = TcpState::Established;
                    c.ack_num = seq_num + 1;
                    c.remote_port = src_port;

                    // Enviar ACK para completar el handshake
                    let mut ack_header = make_tcp_header(dst_port, src_port, c.seq_num, c.ack_num, TCP_FLAG_ACK, c.window_size);
                    let _ = tcp_send_segment(dst_ip, src_ip, &mut ack_header, &[]);
                }
            }
        } else {
            // SYN entrante para nosotros (alguien nos quiere conectar)
            // Responder con SYN-ACK
            let new_seq = next_seq_num();
            let mut syn_ack_header = make_tcp_header(dst_port, src_port, new_seq, seq_num + 1, TCP_FLAG_SYN | TCP_FLAG_ACK, 8192);
            let _ = tcp_send_segment(dst_ip, src_ip, &mut syn_ack_header, &[]);

            // Almacenar como conexión en estado SynReceived
            if let Some(idx) = find_free_tcp_connection() {
                unsafe {
                    TCP_CONNECTIONS[idx] = TcpConnection {
                        state: TcpState::SynReceived,
                        local_ip: *dst_ip,
                        local_port: dst_port,
                        remote_ip: *src_ip,
                        remote_port: src_port,
                        seq_num: new_seq + 1,
                        ack_num: seq_num + 1,
                        window_size: 8192,
                    };
                }
            }
        }
    } else if flags & TCP_FLAG_ACK != 0 && !(flags & TCP_FLAG_SYN != 0) {
        // ACK puro
        if let Some(idx) = conn_idx {
            unsafe {
                let c = &mut TCP_CONNECTIONS[idx];
                if c.state == TcpState::SynSent {
                    c.state = TcpState::Established;
                    c.ack_num = ack_num;
                } else if c.state == TcpState::Established {
                    // Actualizar número de secuencia remoto
                    c.ack_num = ack_num;
                }
            }
        }
    }

    // Si hay datos, procesarlos
    if !payload.is_empty() {
        if let Some(idx) = conn_idx {
            unsafe {
                let c = &mut TCP_CONNECTIONS[idx];
                if c.state == TcpState::Established {
                    c.ack_num = seq_num + payload.len() as u32;
                }
            }
        }
        // Enviar ACK para los datos recibidos
        if let Some(idx) = find_tcp_connection(dst_ip, dst_port, src_ip, src_port) {
            unsafe {
                let c = TCP_CONNECTIONS[idx];
                let mut ack_header = make_tcp_header(dst_port, src_port, c.seq_num, c.ack_num, TCP_FLAG_ACK, c.window_size);
                let _ = tcp_send_segment(dst_ip, src_ip, &mut ack_header, &[]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tcp_states() {
        assert_eq!(TcpState::Closed as u8, 0);
        assert_eq!(TcpState::Listen as u8, 1);
        assert_eq!(TcpState::Established as u8, 4);
    }

    #[test]
    fn test_tcp_flags() {
        assert_eq!(TCP_FLAG_SYN, 0x02);
        assert_eq!(TCP_FLAG_ACK, 0x10);
        assert_eq!(TCP_FLAG_FIN, 0x01);
    }

    #[test]
    fn test_make_tcp_header() {
        let hdr = make_tcp_header(1234, 80, 1000, 0, TCP_FLAG_SYN, 8192);
        assert_eq!(u16::from_be_bytes([hdr.src_port[0], hdr.src_port[1]]), 1234);
        assert_eq!(u16::from_be_bytes([hdr.dst_port[0], hdr.dst_port[1]]), 80);
        assert_eq!(u32::from_be_bytes([hdr.seq_num[0], hdr.seq_num[1], hdr.seq_num[2], hdr.seq_num[3]]), 1000);
        assert_eq!(hdr.flags, TCP_FLAG_SYN);
    }

    #[test]
    fn test_tcp_checksum() {
        let src_ip = [10, 0, 0, 2];
        let dst_ip = [10, 0, 0, 1];
        let hdr = make_tcp_header(1234, 80, 1000, 0, TCP_FLAG_SYN, 8192);
        let mut hdr_bytes = [0u8; 20];
        unsafe {
            let ptr = &hdr as *const TcpHeader as *const u8;
            for i in 0..20 {
                hdr_bytes[i] = core::ptr::read_volatile(ptr.add(i));
            }
        }
        let cs = tcp_checksum(&src_ip, &dst_ip, &hdr_bytes, &[]);
        assert_ne!(cs, 0);
    }
}
