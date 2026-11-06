//! Wrappers de syscalls para espacio de usuario de shardevos (x86_64).
//!
//! Convención x86_64 System V:
//!   rax = número de syscall
//!   rdi, rsi, rdx, r10, r8, r9 = argumentos
//!   retorno en rax (valor negativo codifica error)

#![no_std]

use core::arch::asm;

/// Números de syscall unificados de shardevos (deben coincidir con kernel/syscall/mod.rs).
pub mod nr {
    pub const EXIT: usize = 1;
    pub const WRITE: usize = 2;
    pub const READ: usize = 3;
    pub const OPEN: usize = 4;
    pub const CLOSE: usize = 5;
    pub const STAT: usize = 6;
    pub const VFS_LIST: usize = 23;
    pub const VFS_READ_BY_INODE: usize = 24;
}

/// Errores de syscall (codificados como valor negativo en rax).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(isize)]
pub enum SysError {
    InvalidNr = -1,
    InvalidArgument = -2,
    NotImplemented = -3,
    Denied = -4,
    Fault = -5,
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

/// Escribe `buf` al descriptor `fd` (1=stdout/serial, 2=stderr).
#[inline(always)]
pub fn sys_write(fd: usize, buf: &[u8]) -> Result<usize, SysError> {
    let ret = syscall(nr::WRITE, fd, buf.as_ptr() as usize, buf.len(), 0, 0, 0);
    if ret < 0 {
        Err(unsafe { core::mem::transmute(ret as isize) })
    } else {
        Ok(ret as usize)
    }
}

/// Lee hasta `buf.len()` bytes desde `fd` (0=stdin/serial).
#[inline(always)]
pub fn sys_read(fd: usize, buf: &mut [u8]) -> Result<usize, SysError> {
    let ret = syscall(nr::READ, fd, buf.as_mut_ptr() as usize, buf.len(), 0, 0, 0);
    if ret < 0 {
        Err(unsafe { core::mem::transmute(ret as isize) })
    } else {
        Ok(ret as usize)
    }
}

/// Sale del proceso con `code`.
#[inline(always)]
pub fn sys_exit(code: u8) -> ! {
    syscall(nr::EXIT, code as usize, 0, 0, 0, 0, 0);
    loop {
        core::hint::spin_loop();
    }
}

/// Lista inodos del VFS para indexación.
/// Escribe en `buf`: u64 count, luego [u64 inode_id, u64 size, u8 node_type] por entrada.
/// Devuelve bytes escritos.
#[inline(always)]
pub fn sys_vfs_list(buf: &mut [u8]) -> Result<usize, SysError> {
    let ret = syscall(nr::VFS_LIST, buf.as_mut_ptr() as usize, buf.len(), 0, 0, 0, 0);
    if ret < 0 {
        Err(unsafe { core::mem::transmute(ret as isize) })
    } else {
        Ok(ret as usize)
    }
}

/// Lee datos de un inodo por ID (placeholder: kernel devuelve ceros).
/// arg: inode_id, offset, buf, len.
#[inline(always)]
pub fn sys_vfs_read_by_inode(inode_id: usize, offset: usize, buf: &mut [u8]) -> Result<usize, SysError> {
    let ret = syscall(nr::VFS_READ_BY_INODE, inode_id, offset, buf.as_mut_ptr() as usize, buf.len(), 0, 0);
    if ret < 0 {
        Err(unsafe { core::mem::transmute(ret as isize) })
    } else {
        Ok(ret as usize)
    }
}

/// Imprime una cadena por serial (stdout).
pub fn serial_print(s: &str) {
    let _ = sys_write(1, s.as_bytes());
}

/// Imprime una línea por serial (stdout).
pub fn serial_println(s: &str) {
    let _ = sys_write(1, s.as_bytes());
    let _ = sys_write(1, b"\n");
}

/// Imprime un u64 en hexadecimal por serial.
pub fn serial_print_hex(val: u64) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = [0u8; 18];
    out[0] = b'0';
    out[1] = b'x';
    for i in 0..16 {
        let nibble = ((val >> ((15 - i) * 4)) & 0xF) as usize;
        out[2 + i] = HEX[nibble];
    }
    out[16] = b'\r';
    out[17] = b'\n';
    let _ = sys_write(1, &out);
}

/// Imprime un usize en decimal por serial.
pub fn serial_print_usize(val: usize) {
    if val == 0 {
        serial_println("0");
        return;
    }
    let mut buf = [0u8; 20];
    let mut n = val;
    let mut i = 0;
    while n > 0 {
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    let mut out = [0u8; 22];
    let mut j = 0;
    while i > 0 {
        i -= 1;
        out[j] = buf[i];
        j += 1;
    }
    out[j] = b'\n';
    let _ = sys_write(1, &out[..=j]);
}
