//! shardevos-ping — Cliente ICMP echo (ping) para shardevos.
//!
//! Envía un ICMP echo request a una dirección IP y espera respuesta.
//! Uso: ping <ip> [datos]

#![no_std]
#![no_main]

use core::arch::asm;

/// Números de syscall de shardevos.
mod nr {
    pub const NET_PING: usize = 42;
    pub const WRITE: usize = 2;
    pub const EXIT: usize = 1;
}

/// Errores de syscall.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SysError {
    InvalidNr = -1,
    InvalidArgument = -2,
    NotImplemented = -3,
    Denied = -4,
    Fault = -5,
    DeviceNotFound = -6,
    IOError = -7,
    PermissionDenied = -8,
}

/// Manejador de pánico para userspace no_std.
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}

/// Ejecuta un syscall x86_64 `syscall`.
#[inline(always)]
fn syscall(nr: usize, a1: usize, a2: usize, a3: usize, a4: usize, a5: usize, a6: usize) -> isize {
    let ret: isize;
    unsafe {
        asm!(
            "syscall",
            in("rax") nr,
            in("rdi") a1,
            in("rsi") a2,
            in("rdx") a3,
            in("r10") a4,
            in("r8") a5,
            in("r9") a6,
            lateout("rax") ret,
            options(nostack)
        );
    }
    ret
}

/// Escribe un byte al descriptor `fd` (1=stdout/serial).
#[inline(always)]
fn sys_write(fd: usize, buf: &[u8]) -> Result<usize, SysError> {
    let ret = syscall(nr::WRITE, fd, buf.as_ptr() as usize, buf.len(), 0, 0, 0);
    if ret < 0 {
        Err(match ret {
            -1 => SysError::InvalidNr,
            -2 => SysError::InvalidArgument,
            -3 => SysError::NotImplemented,
            -4 => SysError::Denied,
            -5 => SysError::Fault,
            -6 => SysError::DeviceNotFound,
            -7 => SysError::IOError,
            -8 => SysError::PermissionDenied,
            _ => SysError::Fault,
        })
    } else {
        Ok(ret as usize)
    }
}

/// Sale del proceso con `code`.
#[inline(always)]
fn sys_exit(code: u8) -> ! {
    syscall(nr::EXIT, code as usize, 0, 0, 0, 0, 0);
    loop {
        core::hint::spin_loop();
    }
}

/// Envía un ICMP echo request (ping).
/// arg1: ptr a IP destino (4 bytes), arg2: ptr a datos, arg3: longitud de datos.
#[inline(always)]
fn sys_net_ping(ip_ptr: *const u8, data: &[u8]) -> Result<usize, SysError> {
    let ret = syscall(42, ip_ptr as usize, data.as_ptr() as usize, data.len(), 0, 0, 0);
    if ret < 0 {
        Err(match ret {
            -1 => SysError::InvalidNr,
            -2 => SysError::InvalidArgument,
            -3 => SysError::NotImplemented,
            -4 => SysError::Denied,
            -5 => SysError::Fault,
            -6 => SysError::DeviceNotFound,
            -7 => SysError::IOError,
            -8 => SysError::PermissionDenied,
            _ => SysError::Fault,
        })
    } else {
        Ok(ret as usize)
    }
}

/// Imprime una cadena por serial (stdout).
fn serial_print(s: &str) {
    let _ = sys_write(1, s.as_bytes());
}

/// Imprime una línea por serial (stdout).
fn serial_println(s: &str) {
    let _ = sys_write(1, s.as_bytes());
    let _ = sys_write(1, b"\n");
}

/// Imprime un u8 en decimal por serial.
fn serial_print_u8(val: u8) {
    if val == 0 {
        serial_println("0");
        return;
    }
    let mut buf = [0u8; 3];
    let mut n = val;
    let mut i = 0;
    while n > 0 {
        buf[i] = b'0' + (n % 10);
        n /= 10;
        i += 1;
    }
    while i > 0 {
        i -= 1;
        let _ = sys_write(1, &buf[i..i + 1]);
    }
    serial_println("");
}

/// Punto de entrada del programa ping.
#[no_mangle]
pub extern "C" fn _start() -> ! {
    serial_println("shardevos-ping v1.0.0");
    serial_println("");

    // IP destino por defecto: 10.0.0.1 (gateway)
    let dest_ip: [u8; 4] = [10, 0, 0, 1];

    serial_print("Haciendo ping a ");
    for i in 0..4 {
        serial_print_u8(dest_ip[i]);
        if i < 3 {
            serial_print(".");
        }
    }
    serial_println("");

    // Datos del ping
    let payload: [u8; 34] = *b"shardevos-ping-echo-request-data!!";

    // Enviar ping
    match sys_net_ping(dest_ip.as_ptr(), &payload) {
        Ok(bytes) => {
            serial_print("Respuesta recibida: ");
            serial_print_u8(bytes as u8);
            serial_println(" bytes");
        }
        Err(e) => {
            serial_print("Error en ping: ");
            serial_print_u8(e as u8);
            serial_println("");
        }
    }

    serial_println("");
    serial_println("shardevos-ping finalizado.");

    sys_exit(0);
}
