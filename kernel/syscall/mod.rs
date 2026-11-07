//! Capa de syscalls unificada del kernel shardevos.
//!
//! Proporciona:
//! - Números de syscall unificados (independientes del formato del binario)
//! - Manejador de entrada de syscalls (x86_64: instrucción `syscall`)
//! - Dispatch table y traducciones para PE (Win32), ELF (Linux), MachO (macOS)
//! - API de registro de manejadores por número de syscall

use core::arch::asm;
use core::convert::TryInto;
use core::sync::atomic::{AtomicU64, Ordering};

// Acceso al Unified Input Manager y VFS a través del crate root
use crate::fs::{global_vfs, Filesystem, VfsNodeType};
use crate::fs::global_shardevfs_mut;
use crate::input::global_manager;
use crate::input::{InputError as InputMgrError, InputEvent as InputMgrEvent};
use crate::sched::{create_process_with_address_space, global_scheduler, TaskPriority};
use crate::security::{
    global_user_manager, init_user_manager, SecurityContext, ROOT_UID, ROOT_GID,
    NOBODY_UID, NOBODY_GID,
};
use crate::net::stack::{icmp_echo_request, ip_send, get_our_ip, is_initialized as net_is_initialized};
use crate::net::tcp::{tcp_connect, tcp_receive, tcp_send_segment, TcpConnection, TCP_FLAG_ACK, TCP_FLAG_SYN, TCP_FLAG_PSH, make_tcp_header, TcpHeader, is_initialized as tcp_is_initialized};
use crate::net::{EtherType, IpProtocol};
use crate::userland_loader::load_and_run_userland;
use crate::virtio_blk::global_virtio_blk;
use crate::virtio_net::global_virtio_net;

// Contexto de seguridad global del proceso actual (solo un hilo en esta etapa)
static mut CURRENT_SECURITY_CTX: SecurityContext = SecurityContext {
    uid: NOBODY_UID,
    gid: NOBODY_GID,
    euid: NOBODY_UID,
    egid: NOBODY_GID,
    supplementary_gids: [NOBODY_GID; 16],
    supplementary_count: 0,
    authenticated: false,
    session_id: 0,
};

/// Devuelve el contexto de seguridad del proceso actual.
pub fn current_security_ctx() -> SecurityContext {
    unsafe { CURRENT_SECURITY_CTX }
}

/// Establece el contexto de seguridad del proceso actual.
pub fn set_current_security_ctx(ctx: SecurityContext) {
    unsafe { CURRENT_SECURITY_CTX = ctx; }
}

// Stub de ensamblador que escribe los MSRs de syscall/sysret (IA32_LSTAR/LSS/EFLAGS)
extern "C" {
    fn syscall_setup_msr();
}

/// Número de syscall (unificado).
pub type SyscallNr = u64;

/// Argumentos de syscall (hasta 6, según convenciones x86_64 System V / Windows).
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct SyscallArgs {
    pub nr: SyscallNr,
    pub arg1: usize,
    pub arg2: usize,
    pub arg3: usize,
    pub arg4: usize,
    pub arg5: usize,
    pub arg6: usize,
}

/// Resultado de syscall.
pub type SyscallResult = Result<usize, SyscallError>;

/// Errores de syscall.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyscallError {
    InvalidNr,
    InvalidArgument,
    OutOfMemory,
    NotImplemented,
    Denied,
    Fault,
    DeviceNotFound,
    IOError,
    PermissionDenied,
}

/// Función manejadora de syscall.
type SyscallHandler = fn(SyscallArgs) -> SyscallResult;

/// Tabla de dispatch de syscalls (máximo 4096 entradas).
const MAX_SYSCALLS: usize = 4096;

static mut SYSCALL_TABLE: [Option<SyscallHandler>; MAX_SYSCALLS] = [None; MAX_SYSCALLS];

/// Indica si el subsistema de syscalls está inicializado.
static SYSCALL_INIT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

// ============================================================================
// Números de syscall unificados (shardevos)
// ============================================================================

pub const SYSCALL_EXIT: SyscallNr = 1;
pub const SYSCALL_WRITE: SyscallNr = 2;
pub const SYSCALL_READ: SyscallNr = 3;
pub const SYSCALL_OPEN: SyscallNr = 4;
pub const SYSCALL_CLOSE: SyscallNr = 5;
pub const SYSCALL_STAT: SyscallNr = 6;
pub const SYSCALL_MMAP: SyscallNr = 7;
pub const SYSCALL_MUNMAP: SyscallNr = 8;
pub const SYSCALL_BRK: SyscallNr = 9;
pub const SYSCALL_CLONE: SyscallNr = 10;
pub const SYSCALL_EXECVE: SyscallNr = 11;
pub const SYSCALL_WAIT4: SyscallNr = 12;
pub const SYSCALL_KILL: SyscallNr = 13;
pub const SYSCALL_GETPID: SyscallNr = 14;
pub const SYSCALL_TIME: SyscallNr = 15;
pub const SYSCALL_UNAME: SyscallNr = 16;
pub const SYSCALL_IOCTL: SyscallNr = 17;
pub const SYSCALL_FCNTL: SyscallNr = 18;
pub const SYSCALL_POLL: SyscallNr = 19;
pub const SYSCALL_NANOSLEEP: SyscallNr = 20;
pub const SYSCALL_INPUT_READ: SyscallNr = 21;
pub const SYSCALL_INPUT_POLL: SyscallNr = 22;
pub const SYSCALL_VFS_LIST: SyscallNr = 23;
pub const SYSCALL_VFS_READ_BY_INODE: SyscallNr = 24;
pub const SYSCALL_EXEC: SyscallNr = 25;
pub const SYSCALL_FORK: SyscallNr = 26;
pub const SYSCALL_BLOCK_READ: SyscallNr = 27;
pub const SYSCALL_BLOCK_WRITE: SyscallNr = 28;
pub const SYSCALL_GETUID: SyscallNr = 29;
pub const SYSCALL_GETGID: SyscallNr = 30;
pub const SYSCALL_GETEUID: SyscallNr = 31;
pub const SYSCALL_GETEGID: SyscallNr = 32;
pub const SYSCALL_LOGIN: SyscallNr = 33;
pub const SYSCALL_SETUID: SyscallNr = 34;
pub const SYSCALL_SETGID: SyscallNr = 35;
pub const SYSCALL_NET_SOCKET: SyscallNr = 36;
pub const SYSCALL_NET_BIND: SyscallNr = 37;
pub const SYSCALL_NET_CONNECT: SyscallNr = 38;
pub const SYSCALL_NET_SEND: SyscallNr = 39;
pub const SYSCALL_NET_RECV: SyscallNr = 40;
pub const SYSCALL_NET_CLOSE: SyscallNr = 41;
pub const SYSCALL_NET_PING: SyscallNr = 42;
pub const SYSCALL_VFS_LOOKUP: SyscallNr = 43;

// ============================================================================
// Traducción de syscalls de formatos heterogéneos
// ============================================================================

/// Origen del binario que emite el syscall.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOrigin {
    Native,     // Binario nativo de shardevos
    PE,         // Windows PE (Win32/NT API)
    ELF,        // Linux ELF (POSIX syscalls)
    MachO,      // macOS Mach-O (BSD/Mach syscalls)
}

/// Traduce un número de syscall de origen externo al número unificado de shardevos.
/// Devuelve None si no hay traducción disponible.
pub fn translate_syscall(origin: BinaryOrigin, foreign_nr: u64) -> Option<SyscallNr> {
    match origin {
        BinaryOrigin::Native => Some(foreign_nr),
        BinaryOrigin::PE => translate_pe_syscall(foreign_nr),
        BinaryOrigin::ELF => translate_elf_syscall(foreign_nr),
        BinaryOrigin::MachO => translate_macho_syscall(foreign_nr),
    }
}

/// Traducción Win32/NT -> shardevos (subset básico).
fn translate_pe_syscall(nr: u64) -> Option<SyscallNr> {
    match nr {
        // Win32: ExitProcess -> shardevos exit
        0x1 => Some(SYSCALL_EXIT),   // aproximado
        // Win32: WriteFile -> shardevos write
        0x2 => Some(SYSCALL_WRITE),
        // Win32: ReadFile -> shardevos read
        0x3 => Some(SYSCALL_READ),
        // Win32: CreateFile -> shardevos open
        0x4 => Some(SYSCALL_OPEN),
        // Win32: CloseHandle -> shardevos close
        0x5 => Some(SYSCALL_CLOSE),
        _ => None,
    }
}

/// Traducción Linux ELF -> shardevos (subset básico).
fn translate_elf_syscall(nr: u64) -> Option<SyscallNr> {
    match nr {
        1  => Some(SYSCALL_WRITE),   // Linux write
        3  => Some(SYSCALL_READ),    // Linux read
        5  => Some(SYSCALL_OPEN),    // Linux open
        6  => Some(SYSCALL_CLOSE),   // Linux close
        60 => Some(SYSCALL_EXIT),    // Linux exit_group/exit
        9  => Some(SYSCALL_MMAP),    // Linux mmap
        11 => Some(SYSCALL_EXECVE),  // Linux execve
        _  => None,
    }
}

/// Traducción macOS MachO -> shardevos (subset básico).
fn translate_macho_syscall(nr: u64) -> Option<SyscallNr> {
    match nr {
        1  => Some(SYSCALL_EXIT),    // macOS exit
        4  => Some(SYSCALL_WRITE),   // macOS write
        3  => Some(SYSCALL_READ),    // macOS read
        5  => Some(SYSCALL_OPEN),    // macOS open
        6  => Some(SYSCALL_CLOSE),   // macOS close
        _  => None,
    }
}

// ============================================================================
// Registro y dispatch de syscalls
// ============================================================================

/// Registra un manejador para un número de syscall.
pub fn register(nr: SyscallNr, handler: SyscallHandler) -> Result<(), SyscallError> {
    if nr as usize >= MAX_SYSCALLS {
        return Err(SyscallError::InvalidNr);
    }
    unsafe {
        SYSCALL_TABLE[nr as usize] = Some(handler);
    }
    Ok(())
}

/// Ejecuta un syscall buscando el manejador correspondiente.
pub fn dispatch(args: SyscallArgs) -> SyscallResult {
    if SYSCALL_INIT.load(core::sync::atomic::Ordering::Acquire) == 0 {
        return Err(SyscallError::NotImplemented);
    }
    if is_privileged_syscall(args.nr) {
        let ctx = current_security_ctx();
        if !ctx.authenticated {
            return Err(SyscallError::PermissionDenied);
        }
    }
    unsafe {
        match SYSCALL_TABLE[args.nr as usize] {
            Some(handler) => handler(args),
            None => Err(SyscallError::InvalidNr),
        }
    }
}

fn is_privileged_syscall(nr: SyscallNr) -> bool {
    matches!(nr, SYSCALL_SETUID | SYSCALL_SETGID)
}

/// Manejador por defecto para syscalls no implementados.
#[allow(dead_code)]
fn default_handler(_args: SyscallArgs) -> SyscallResult {
    Err(SyscallError::NotImplemented)
}

// ============================================================================
// Manejadores de syscalls implementados (placeholders)
// ============================================================================

fn sys_exit(args: SyscallArgs) -> SyscallResult {
    let code = args.arg1 as u8;
    // Apagar el sistema o terminar el hilo actual (placeholder)
    let _ = code;
    // Detener la CPU - el hilo actual ha terminado
    unsafe {
        core::arch::asm!("hlt", options(nostack));
    }
    // Nunca retorna
    loop {
        core::hint::spin_loop();
    }
}

fn sys_write(args: SyscallArgs) -> SyscallResult {
    let fd = args.arg1;
    let buf = args.arg2 as *const u8;
    let count = args.arg3;

    // Escribir a través del driver de salida serial (stdout)
    // Por ahora, escribimos byte a byte al puerto serie COM1 (0x3F8)
    if fd == 1 || fd == 2 {
        unsafe {
            for i in 0..count {
                let byte = core::ptr::read_volatile(buf.add(i));
                serial_write(byte);
            }
        }
        Ok(count)
    } else {
        Err(SyscallError::InvalidArgument)
    }
}

fn sys_read(args: SyscallArgs) -> SyscallResult {
    let fd = args.arg1;
    let buf = args.arg2 as *mut u8;
    let count = args.arg3;

    if buf.is_null() || count == 0 {
        return Err(SyscallError::InvalidArgument);
    }

    // Leer desde puerto serie COM1 (stdin)
    if fd == 0 {
        let mut read_count: usize = 0;
        unsafe {
            for i in 0..count {
                let byte = serial_read_byte()?;
                core::ptr::write_volatile(buf.add(i), byte);
                read_count += 1;
                // Si es newline, terminar lectura
                if byte == b'\n' || byte == b'\r' {
                    break;
                }
            }
        }
        Ok(read_count)
    } else {
        Err(SyscallError::InvalidArgument)
    }
}

/// Lee un byte del puerto serie COM1.
unsafe fn serial_read_byte() -> Result<u8, SyscallError> {
    let port: u16 = 0x3F8;
    let lsr_port = port + 5;
    // Esperar a que haya datos disponibles (bit 0 del LSR = Data Ready)
    for _ in 0..100000 {
        let lsr: u8;
        asm!("in al, dx", out("al") lsr, in("dx") lsr_port, options(nostack));
        if lsr & 0x01 != 0 {
            let byte: u8;
            asm!("in al, dx", out("al") byte, in("dx") port, options(nostack));
            return Ok(byte);
        }
    }
    Err(SyscallError::Fault)
}

fn sys_getpid(_args: SyscallArgs) -> SyscallResult {
    // Retornar PID del hilo actual (placeholder: 1 para el kernel)
    Ok(1)
}

fn sys_time(_args: SyscallArgs) -> SyscallResult {
    // Retornar tiempo desde el boot en segundos (placeholder)
    Ok(0)
}

fn sys_nanosleep(args: SyscallArgs) -> SyscallResult {
    let nanos = args.arg1;
    // Spin simple por nanos segundos (placeholder muy aproximado)
    let _ = nanos;
    for _ in 0..1_000_000 {
        core::hint::spin_loop();
    }
    Ok(0)
}

fn sys_input_read(args: SyscallArgs) -> SyscallResult {
    let buf_ptr = args.arg1 as *mut u8;
    let buf_size = args.arg2;

    if buf_ptr.is_null() || buf_size < core::mem::size_of::<InputMgrEvent>() {
        return Err(SyscallError::InvalidArgument);
    }

    let mgr = global_manager();
    match mgr.poll_event() {
        Ok(event) => {
            // Copiar el evento al buffer del usuario
            let event_size = core::mem::size_of::<InputMgrEvent>();
            unsafe {
                core::ptr::copy_nonoverlapping(
                    &event as *const InputMgrEvent as *const u8,
                    buf_ptr,
                    event_size.min(buf_size),
                );
            }
            Ok(event_size)
        }
        Err(InputMgrError::QueueEmpty) => Ok(0),
        Err(_) => Err(SyscallError::Fault),
    }
}

fn sys_input_poll(_args: SyscallArgs) -> SyscallResult {
    let mgr = global_manager();
    if mgr.len() > 0 {
        Ok(1)
    } else {
        Ok(0)
    }
}

/// Lista inodos del VFS (para indexación de archivos).
/// arg1: unused; arg2: ptr a buffer de salida; arg3: tamaño del buffer.
/// Formato de salida: u64 count, luego [u64 inode_id, u64 size, u8 node_type] repetido.
fn sys_vfs_list(args: SyscallArgs) -> SyscallResult {
    let buf = args.arg2 as *mut u8;
    let count = args.arg3;
    if buf.is_null() || count < 8 {
        return Err(SyscallError::InvalidArgument);
    }
    let vfs = global_vfs();
    let total = vfs.inode_count();
    let mut written: usize = 0;
    // Escribir count
    if written + 8 <= count {
        unsafe {
            core::ptr::write_volatile(buf.add(written) as *mut u64, total as u64);
        }
        written += 8;
    }
    let max_entries = ((count - written) / 17).min(total as usize);
    for i in 0..max_entries {
        if let Some(inode) = vfs.inode_entry_at(i) {
            if written + 17 > count {
                break;
            }
            unsafe {
                core::ptr::write_volatile(buf.add(written) as *mut u64, inode.id);
                core::ptr::write_volatile(buf.add(written + 8) as *mut u64, inode.size);
                core::ptr::write_volatile(buf.add(written + 16) as *mut u8, inode.node_type as u8);
            }
            written += 17;
        }
    }
    Ok(written)
}

/// Lee datos de un inodo por ID (placeholder: devuelve metadatos + ceros).
/// arg1: inode_id; arg2: offset; arg3: ptr a buffer; arg4: count.
fn sys_vfs_read_by_inode(args: SyscallArgs) -> SyscallResult {
    let inode_id = args.arg1;
    let offset = args.arg2;
    let buf = args.arg3 as *mut u8;
    let count = args.arg4;
    if buf.is_null() || count == 0 {
        return Err(SyscallError::InvalidArgument);
    }
    let vfs = global_vfs();
    let stat = match vfs.stat(inode_id as u64) {
        Ok(s) => s,
        Err(_) => return Err(SyscallError::InvalidArgument),
    };
    if stat.node_type != VfsNodeType::RegularFile {
        return Err(SyscallError::InvalidArgument);
    }
    let start = offset as usize;
    let end = (start + count).min(stat.size as usize);
    let read_len = if start >= stat.size as usize { 0 } else { end - start };
    if read_len == 0 {
        return Ok(0);
    }

    // Lectura real desde shardevfs (datos en RAM + write-through a disco)
    let shardevfs = global_shardevfs_mut();
    let dst = unsafe { core::slice::from_raw_parts_mut(buf, read_len) };
    match shardevfs.read_data(inode_id as u64, offset as u64, dst) {
        Ok(n) => Ok(n),
        Err(_) => Err(SyscallError::InvalidArgument),
    }
}

/// Resuelve una ruta del VFS a un inodo.
/// arg1: ptr al path; arg2: longitud del path.
fn sys_vfs_lookup(args: SyscallArgs) -> SyscallResult {
    let path_ptr = args.arg1 as *const u8;
    let path_len = args.arg2;
    if path_ptr.is_null() || path_len == 0 || path_len > 256 {
        return Err(SyscallError::InvalidArgument);
    }
    let path_bytes = unsafe { core::slice::from_raw_parts(path_ptr, path_len) };
    let path = core::str::from_utf8(path_bytes).map_err(|_| SyscallError::InvalidArgument)?;
    let vfs = global_vfs();
    match vfs.lookup(path) {
        Ok(inode_id) => Ok(inode_id as usize),
        Err(_) => Err(SyscallError::InvalidArgument),
    }
}

/// Ejecuta un binario ELF desde el VFS.
/// arg1: ptr a path en memoria; arg2: longitud del path.
/// Carga el ELF y salta a su entry point (no retorna en caso de éxito).
fn sys_exec(args: SyscallArgs) -> SyscallResult {
    let path_ptr = args.arg1 as *const u8;
    let path_len = args.arg2;
    if path_ptr.is_null() || path_len == 0 || path_len > 256 {
        return Err(SyscallError::InvalidArgument);
    }

    // Copiar el path desde memoria de usuario
    let path_bytes = unsafe { core::slice::from_raw_parts(path_ptr, path_len) };
    let path_str = core::str::from_utf8(path_bytes).map_err(|_| SyscallError::InvalidArgument)?;

    // Resolver path en el VFS
    let vfs = global_vfs();
    let inode_id = vfs.lookup(path_str).map_err(|_| SyscallError::InvalidArgument)?;

    // Obtener tamaño del archivo
    let stat = vfs.stat(inode_id).map_err(|_| SyscallError::InvalidArgument)?;
    if stat.node_type != VfsNodeType::RegularFile {
        return Err(SyscallError::InvalidArgument);
    }
    let file_size = stat.size as usize;
    if file_size == 0 || file_size > 65536 {
        return Err(SyscallError::InvalidArgument);
    }

    // Leer contenido completo del archivo
    let mut file_buf = [0u8; 65536];
    let mut offset: usize = 0;
    let shardevfs = global_shardevfs_mut();
    while offset < file_size {
        let to_read = (file_size - offset).min(4096);
        let read = shardevfs.read_data(inode_id, offset as u64, &mut file_buf[offset..offset + to_read]).map_err(|_| SyscallError::Fault)?;
        if read == 0 {
            break;
        }
        offset += read;
    }

    // Ejecutar el binario ELF (no retorna)
    load_and_run_userland(&file_buf[..offset]);

    // Si por alguna razón retorna, hacer loop
    loop {
        core::hint::spin_loop();
    }
}

/// Fork: crea un nuevo proceso clonando el espacio de direcciones actual.
/// Retorna el PID del hijo al padre, 0 al hijo.
fn sys_fork(_args: SyscallArgs) -> SyscallResult {
    // Crear nuevo proceso con su propio PML4
    let (pid, pml4_addr) = create_process_with_address_space(
        unsafe { &mut *(global_scheduler() as *mut _) },
        "forked_child",
        0,
    ).map_err(|_| SyscallError::Fault)?;

    let mut sched: &mut crate::sched::Scheduler = unsafe { &mut *(global_scheduler() as *mut _) };
    let _tid = sched.create_thread_with_address_space(
        pid,
        0x1000,
        0x20000,
        0x1000,
        TaskPriority::Normal,
        pml4_addr,
    ).map_err(|_| SyscallError::Fault)?;

    unsafe {
        crate::syscall::set_current_security_ctx(crate::security::SecurityContext::new_unauthenticated());
    }

    Ok(pid as usize)
}

// ============================================================================
// Syscalls de red para userspace
// ============================================================================

/// Tipo de socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketType {
    Stream = 1,   // TCP (placeholder)
    Datagram = 2, // UDP / ICMP raw
}

/// Estado de un socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketState {
    Free = 0,
    Allocated = 1,
    Bound = 2,
    Connected = 3,
}

/// Entrada de socket abierta.
#[derive(Debug, Clone, Copy)]
pub struct SocketEntry {
    pub state: SocketState,
    pub sock_type: SocketType,
    pub local_port: u16,
    pub remote_ip: [u8; 4],
    pub remote_port: u16,
}

/// Tabla global de sockets (máximo 16 sockets concurrentes).
const MAX_SOCKETS: usize = 16;

static mut SOCKET_TABLE: [SocketEntry; MAX_SOCKETS] = [SocketEntry {
    state: SocketState::Free,
    sock_type: SocketType::Datagram,
    local_port: 0,
    remote_ip: [0; 4],
    remote_port: 0,
}; MAX_SOCKETS];

static NEXT_SOCKET_ID: AtomicU64 = AtomicU64::new(1);

/// Encuentra un socket libre en la tabla.
fn find_free_socket() -> Result<usize, SyscallError> {
    for i in 0..MAX_SOCKETS {
        if unsafe { SOCKET_TABLE[i].state } == SocketState::Free {
            return Ok(i);
        }
    }
    Err(SyscallError::OutOfMemory)
}

/// Lee bloques desde el dispositivo de bloque VirtIO.
/// arg1: sector (LBA), arg2: sector_count, arg3: ptr a buffer, arg4: tamaño del buffer en bytes.
fn sys_block_read(args: SyscallArgs) -> SyscallResult {
    let sector = args.arg1 as u64;
    let sector_count = args.arg2 as u32;
    let buffer = args.arg3 as *mut u8;
    let buf_size = args.arg4;

    if buffer.is_null() || buf_size == 0 {
        return Err(SyscallError::InvalidArgument);
    }

    let blk = unsafe { global_virtio_blk() };
    if blk.state() != crate::virtio_blk::VirtioBlkState::Ready {
        return Err(SyscallError::DeviceNotFound);
    }

    let bytes_needed = sector_count as u64 * blk.info().sector_size as u64;
    if buf_size < bytes_needed as usize {
        return Err(SyscallError::InvalidArgument);
    }

    unsafe {
        blk.read_blocks(sector, sector_count, buffer)
            .map_err(|_| SyscallError::IOError)?;
    }

    Ok(bytes_needed as usize)
}

/// Escribe bloques al dispositivo de bloque VirtIO.
/// arg1: sector (LBA), arg2: sector_count, arg3: ptr a buffer, arg4: tamaño del buffer en bytes.
fn sys_block_write(args: SyscallArgs) -> SyscallResult {
    let sector = args.arg1 as u64;
    let sector_count = args.arg2 as u32;
    let buffer = args.arg3 as *const u8;
    let buf_size = args.arg4;

    if buffer.is_null() || buf_size == 0 {
        return Err(SyscallError::InvalidArgument);
    }

    let blk = unsafe { global_virtio_blk() };
    if blk.state() != crate::virtio_blk::VirtioBlkState::Ready {
        return Err(SyscallError::DeviceNotFound);
    }

    let bytes_needed = sector_count as u64 * blk.info().sector_size as u64;
    if buf_size < bytes_needed as usize {
        return Err(SyscallError::InvalidArgument);
    }

    unsafe {
        blk.write_blocks(sector, sector_count, buffer)
            .map_err(|_| SyscallError::IOError)?;
    }

    Ok(bytes_needed as usize)
}

/// net_socket: crea un nuevo socket de red.
/// arg1: tipo de socket (1=Stream, 2=Datagram).
fn sys_net_socket(args: SyscallArgs) -> SyscallResult {
    let sock_type = match args.arg1 {
        1 => SocketType::Stream,
        2 => SocketType::Datagram,
        _ => return Err(SyscallError::InvalidArgument),
    };

    let idx = find_free_socket()?;
    let socket_id = NEXT_SOCKET_ID.fetch_add(1, Ordering::Relaxed) as i64;

    unsafe {
        SOCKET_TABLE[idx] = SocketEntry {
            state: SocketState::Allocated,
            sock_type,
            local_port: 0,
            remote_ip: [0; 4],
            remote_port: 0,
        };
    }

    Ok(socket_id as usize)
}

/// net_bind: asocia un socket a un puerto local.
/// arg1: socket_id, arg2: puerto local.
fn sys_net_bind(args: SyscallArgs) -> SyscallResult {
    let socket_id = args.arg1 as usize;
    let port = args.arg2 as u16;

    let idx = socket_id.wrapping_sub(1);
    if idx >= MAX_SOCKETS {
        return Err(SyscallError::InvalidArgument);
    }

    unsafe {
        let entry = &mut SOCKET_TABLE[idx];
        if entry.state == SocketState::Free {
            return Err(SyscallError::InvalidArgument);
        }
        entry.local_port = port;
        entry.state = SocketState::Bound;
    }

    Ok(0)
}

/// net_connect: conecta un socket a una dirección remota.
/// arg1: socket_id, arg2: ptr a IP remota (4 bytes), arg3: puerto remoto.
fn sys_net_connect(args: SyscallArgs) -> SyscallResult {
    let socket_id = args.arg1 as usize;
    let ip_ptr = args.arg2 as *const u8;
    let port = args.arg3 as u16;

    if ip_ptr.is_null() {
        return Err(SyscallError::InvalidArgument);
    }

    let idx = socket_id.wrapping_sub(1);
    if idx >= MAX_SOCKETS {
        return Err(SyscallError::InvalidArgument);
    }

    let remote_ip = unsafe { core::slice::from_raw_parts(ip_ptr, 4) };
    let mut rip = [0u8; 4];
    rip.copy_from_slice(remote_ip);

    let local_ip = get_our_ip();

    // Encontrar un puerto local disponible
    let local_port = NEXT_SOCKET_ID.fetch_add(1, Ordering::Relaxed) as u16 + 1024;

    // Realizar handshake TCP
    tcp_connect(&local_ip, local_port, &rip, port).map_err(|_| SyscallError::IOError)?;

    unsafe {
        let entry = &mut SOCKET_TABLE[idx];
        if entry.state == SocketState::Free {
            return Err(SyscallError::InvalidArgument);
        }
        entry.remote_ip = rip;
        entry.remote_port = port;
        entry.local_port = local_port;
        entry.state = SocketState::Connected;
    }

    Ok(0)
}

/// net_send: envía datos por un socket.
/// arg1: socket_id, arg2: ptr a datos, arg3: longitud de datos.
fn sys_net_send(args: SyscallArgs) -> SyscallResult {
    let socket_id = args.arg1 as usize;
    let data_ptr = args.arg2 as *const u8;
    let data_len = args.arg3;

    if data_ptr.is_null() || data_len == 0 {
        return Err(SyscallError::InvalidArgument);
    }

    let idx = socket_id.wrapping_sub(1);
    if idx >= MAX_SOCKETS {
        return Err(SyscallError::InvalidArgument);
    }

    let entry = unsafe { SOCKET_TABLE[idx] };
    if entry.state == SocketState::Free {
        return Err(SyscallError::InvalidArgument);
    }

    let local_ip = get_our_ip();
    let remote_ip = entry.remote_ip;
    let local_port = entry.local_port;
    let remote_port = entry.remote_port;

    let data = unsafe { core::slice::from_raw_parts(data_ptr, data_len) };

    // Enviar como segmento TCP
    let mut tcp_header = make_tcp_header(local_port, remote_port, 0, 0, TCP_FLAG_PSH | TCP_FLAG_ACK, 8192);
    tcp_send_segment(&local_ip, &remote_ip, &mut tcp_header, data).map_err(|_| SyscallError::IOError)?;

    Ok(data_len)
}

/// net_recv: recibe datos de un socket.
/// arg1: socket_id, arg2: ptr a buffer, arg3: tamaño del buffer.
fn sys_net_recv(args: SyscallArgs) -> SyscallResult {
    let socket_id = args.arg1 as usize;
    let buf_ptr = args.arg2 as *mut u8;
    let buf_size = args.arg3;

    if buf_ptr.is_null() || buf_size == 0 {
        return Err(SyscallError::InvalidArgument);
    }

    let idx = socket_id.wrapping_sub(1);
    if idx >= MAX_SOCKETS {
        return Err(SyscallError::InvalidArgument);
    }

    let entry = unsafe { SOCKET_TABLE[idx] };
    if entry.state == SocketState::Free {
        return Err(SyscallError::InvalidArgument);
    }

    let local_port = entry.local_port;
    let remote_port = entry.remote_port;
    let remote_ip = entry.remote_ip;
    let local_ip = get_our_ip();

    // Recibir paquete de la red
    let net = unsafe { global_virtio_net() };
    if net.state() != crate::virtio_net::VirtioNetState::Ready {
        return Err(SyscallError::DeviceNotFound);
    }

    // Intentar recibir un paquete (no bloqueante)
    match unsafe { net.receive_packet() } {
        Some(pkt) => {
            // Verificar que sea un paquete Ethernet con al menos 14 bytes
            if pkt.len() < 14 {
                return Ok(0);
            }

            // Verificar ethertype == IPv4 (0x0800)
            let ethertype = u16::from_be_bytes([pkt[12], pkt[13]]);
            if ethertype != EtherType::IPv4 as u16 {
                return Ok(0);
            }

            // Verificar que hay al menos IP header (20 bytes)
            if pkt.len() < 14 + 20 {
                return Ok(0);
            }

            // Verificar protocolo IP == TCP (6)
            let ip_protocol = pkt[23];
            if ip_protocol != IpProtocol::TCP as u8 {
                return Ok(0);
            }

            // Extraer direcciones IP y puertos TCP
            let src_ip: [u8; 4] = pkt[26..30].try_into().unwrap();
            let dst_ip: [u8; 4] = pkt[30..34].try_into().unwrap();
            let tcp_src_port = u16::from_be_bytes([pkt[34], pkt[35]]);
            let tcp_dst_port = u16::from_be_bytes([pkt[36], pkt[37]]);

            // Verificar que el paquete esté dirigido a nuestro socket
            if tcp_dst_port != local_port || tcp_src_port != remote_port {
                return Ok(0);
            }

            // Verificar que el destino sea nuestra IP
            if dst_ip != local_ip {
                return Ok(0);
            }

            // Extraer el payload TCP (después de IP header + TCP header)
            let tcp_header_offset = 14 + 20; // Ethernet + IP
            let data_offset = ((pkt[tcp_header_offset + 12] >> 4) as usize) * 4; // Data offset en palabras de 32 bits
            let tcp_header_total = tcp_header_offset + data_offset;
            let payload_start = tcp_header_total;

            if pkt.len() < payload_start {
                return Ok(0);
            }

            let payload = &pkt[payload_start..];

            // Procesar el paquete TCP para actualizar el estado de la conexión
            let header = TcpHeader {
                src_port: u16::from_be_bytes([pkt[34], pkt[35]]),
                dst_port: u16::from_be_bytes([pkt[36], pkt[37]]),
                seq_num: u32::from_be_bytes([pkt[38], pkt[39], pkt[40], pkt[41]]),
                ack_num: u32::from_be_bytes([pkt[42], pkt[43], pkt[44], pkt[45]]),
                data_offset_reserved: pkt[46],
                flags: pkt[47],
                window_size: u16::from_be_bytes([pkt[48], pkt[49]]),
                checksum: u16::from_be_bytes([pkt[50], pkt[51]]),
                urgent_ptr: u16::from_be_bytes([pkt[52], pkt[53]]),
            };
            tcp_receive(&dst_ip, &src_ip, &header, payload);

            // Copiar el payload al buffer del usuario
            let copy_len = payload.len().min(buf_size);
            unsafe {
                core::ptr::copy_nonoverlapping(payload.as_ptr(), buf_ptr, copy_len);
            }
            Ok(copy_len)
        }
        None => Ok(0), // No hay datos disponibles
    }
}

/// net_close: cierra un socket.
/// arg1: socket_id.
fn sys_net_close(args: SyscallArgs) -> SyscallResult {
    let socket_id = args.arg1 as usize;
    let idx = socket_id.wrapping_sub(1);

    if idx >= MAX_SOCKETS {
        return Err(SyscallError::InvalidArgument);
    }

    unsafe {
        SOCKET_TABLE[idx] = SocketEntry {
            state: SocketState::Free,
            sock_type: SocketType::Datagram,
            local_port: 0,
            remote_ip: [0; 4],
            remote_port: 0,
        };
    }

    Ok(0)
}

/// net_ping: envía un ICMP echo request a una dirección IP.
/// arg1: ptr a IP destino (4 bytes), arg2: ptr a datos, arg3: longitud de datos.
fn sys_net_ping(args: SyscallArgs) -> SyscallResult {
    let ip_ptr = args.arg1 as *const u8;
    let data_ptr = args.arg2 as *const u8;
    let data_len = args.arg3;

    if ip_ptr.is_null() {
        return Err(SyscallError::InvalidArgument);
    }

    let ip_bytes = unsafe { core::slice::from_raw_parts(ip_ptr, 4) };
    let mut dest_ip = [0u8; 4];
    dest_ip.copy_from_slice(ip_bytes);

    let data = if data_ptr.is_null() || data_len == 0 {
        b"ping".as_slice()
    } else {
        unsafe { core::slice::from_raw_parts(data_ptr, data_len) }
    };

    icmp_echo_request(&dest_ip, data).map_err(|_| SyscallError::IOError)?;

    Ok(0)
}

/// getuid: devuelve el UID real del proceso actual.
fn sys_getuid(_args: SyscallArgs) -> SyscallResult {
    Ok(current_security_ctx().uid as usize)
}

/// getgid: devuelve el GID real del proceso actual.
fn sys_getgid(_args: SyscallArgs) -> SyscallResult {
    Ok(current_security_ctx().gid as usize)
}

/// geteuid: devuelve el UID efectivo del proceso actual.
fn sys_geteuid(_args: SyscallArgs) -> SyscallResult {
    Ok(current_security_ctx().euid as usize)
}

/// getegid: devuelve el GID efectivo del proceso actual.
fn sys_getegid(_args: SyscallArgs) -> SyscallResult {
    Ok(current_security_ctx().egid as usize)
}

/// login: autentica al usuario con nombre de usuario y contraseña.
/// arg1: ptr a nombre de usuario, arg2: longitud del nombre, arg3: ptr a contraseña, arg4: longitud de la contraseña.
fn sys_login(args: SyscallArgs) -> SyscallResult {
    let user_ptr = args.arg1 as *const u8;
    let user_len = args.arg2 as usize;
    let pass_ptr = args.arg3 as *const u8;
    let pass_len = args.arg4 as usize;

    if user_ptr.is_null() || user_len == 0 || user_len > 256 {
        return Err(SyscallError::InvalidArgument);
    }
    if pass_ptr.is_null() || pass_len == 0 || pass_len > 256 {
        return Err(SyscallError::InvalidArgument);
    }

    let username = unsafe { core::str::from_utf8(core::slice::from_raw_parts(user_ptr, user_len)) }
        .map_err(|_| SyscallError::InvalidArgument)?;
    let password = unsafe { core::str::from_utf8(core::slice::from_raw_parts(pass_ptr, pass_len)) }
        .map_err(|_| SyscallError::InvalidArgument)?;

    let um = unsafe { global_user_manager() };
    match um.authenticate(username, password) {
        Ok((uid, gid)) => {
            // Establecer el contexto de seguridad autenticado
            let mut ctx = SecurityContext::new_unauthenticated();
            ctx.uid = uid;
            ctx.gid = gid;
            ctx.euid = uid;
            ctx.egid = gid;
            ctx.authenticated = true;
            ctx.session_id = 1;
            set_current_security_ctx(ctx);
            Ok(uid as usize)
        }
        Err(_) => Err(SyscallError::InvalidArgument),
    }
}

/// setuid: establece el UID efectivo (solo root puede cambiar a cualquier UID).
/// arg1: nuevo UID.
fn sys_setuid(args: SyscallArgs) -> SyscallResult {
    let new_uid = args.arg1 as u32;
    let ctx = current_security_ctx();
    if ctx.euid != ROOT_UID {
        return Err(SyscallError::PermissionDenied);
    }
    let mut new_ctx = ctx;
    new_ctx.euid = new_uid;
    set_current_security_ctx(new_ctx);
    Ok(0)
}

/// setgid: establece el GID efectivo (solo root puede cambiar a cualquier GID).
/// arg1: nuevo GID.
fn sys_setgid(args: SyscallArgs) -> SyscallResult {
    let new_gid = args.arg1 as u32;
    let ctx = current_security_ctx();
    if ctx.euid != ROOT_UID {
        return Err(SyscallError::PermissionDenied);
    }
    let mut new_ctx = ctx;
    new_ctx.egid = new_gid;
    set_current_security_ctx(new_ctx);
    Ok(0)
}

// ============================================================================
// Entrada de syscall desde modo usuario (x86_64)
// ============================================================================

/// Punto de entrada llamado desde el wrapper de ensamblador `syscall_entry`.
/// Los registros x86_64 para syscall son:
///   rax = número de syscall
///   rdi, rsi, rdx, r10, r8, r9 = argumentos
///   rcx = return address, r11 = rflags
///
/// Devuelve el resultado en rax.
#[no_mangle]
pub extern "C" fn syscall_handler(
    nr: SyscallNr,
    arg1: usize,
    arg2: usize,
    arg3: usize,
    arg4: usize,
    arg5: usize,
    arg6: usize,
) -> usize {
    let args = SyscallArgs { nr, arg1, arg2, arg3, arg4, arg5, arg6 };

    match dispatch(args) {
        Ok(val) => val,
        Err(e) => {
            // Retornar valor negativo codificando el error (convención Linux-like)
            -(e as isize) as usize
        }
    }
}

/// Escribe un byte al puerto serie COM1.
unsafe fn serial_write(byte: u8) {
    let port: u16 = 0x3F8;
    let lsr_port = port + 5;
    // Esperar a que el THR esté vacío (bit 5 del LSR)
    for _ in 0..1000 {
        let lsr: u8;
        asm!("in al, dx", out("al") lsr, in("dx") lsr_port, options(nostack));
        if lsr & 0x20 != 0 {
            break;
        }
    }
    asm!("out dx, al", in("dx") port, in("al") byte, options(nostack));
}

/// Inicializa el subsistema de syscalls.
/// - Registra manejadores por defecto
/// - Configura el IDT para la interrupción de syscall (vector 0x80) y/o MSR_LSTAR
pub unsafe fn init() {
    if SYSCALL_INIT.load(core::sync::atomic::Ordering::Acquire) != 0 {
        return;
    }

    // Registrar manejadores integrados
    let _ = register(SYSCALL_EXIT, sys_exit);
    let _ = register(SYSCALL_WRITE, sys_write);
    let _ = register(SYSCALL_READ, sys_read);
    let _ = register(SYSCALL_GETPID, sys_getpid);
    let _ = register(SYSCALL_TIME, sys_time);
    let _ = register(SYSCALL_NANOSLEEP, sys_nanosleep);
    let _ = register(SYSCALL_INPUT_READ, sys_input_read);
    let _ = register(SYSCALL_INPUT_POLL, sys_input_poll);
    let _ = register(SYSCALL_VFS_LIST, sys_vfs_list);
    let _ = register(SYSCALL_VFS_READ_BY_INODE, sys_vfs_read_by_inode);
    let _ = register(SYSCALL_VFS_LOOKUP, sys_vfs_lookup);
    let _ = register(SYSCALL_EXEC, sys_exec);
    let _ = register(SYSCALL_FORK, sys_fork);
    let _ = register(SYSCALL_BLOCK_READ, sys_block_read);
    let _ = register(SYSCALL_BLOCK_WRITE, sys_block_write);
    let _ = register(SYSCALL_GETUID, sys_getuid);
    let _ = register(SYSCALL_GETGID, sys_getgid);
    let _ = register(SYSCALL_GETEUID, sys_geteuid);
    let _ = register(SYSCALL_GETEGID, sys_getegid);
    let _ = register(SYSCALL_LOGIN, sys_login);
    let _ = register(SYSCALL_SETUID, sys_setuid);
    let _ = register(SYSCALL_SETGID, sys_setgid);
    let _ = register(SYSCALL_NET_SOCKET, sys_net_socket);
    let _ = register(SYSCALL_NET_BIND, sys_net_bind);
    let _ = register(SYSCALL_NET_CONNECT, sys_net_connect);
    let _ = register(SYSCALL_NET_SEND, sys_net_send);
    let _ = register(SYSCALL_NET_RECV, sys_net_recv);
    let _ = register(SYSCALL_NET_CLOSE, sys_net_close);
    let _ = register(SYSCALL_NET_PING, sys_net_ping);

    // Configurar el MSR LSTAR para la instrucción syscall/sysret (x86_64)
    // LSTAR = dirección del entry point de syscall en modo kernel
    // Usamos el vector 0x80 (int 0x80) como alternativa para compatibilidad
    setup_syscall_msr();

    SYSCALL_INIT.store(1, core::sync::atomic::Ordering::Release);
}

/// Configura el MSR IA32_LSTAR (0xC0000082) para syscall/sysret.
unsafe fn setup_syscall_msr() {
    // Delegar al stub de ensamblador que escribe los MSRs de syscall/sysret.
    syscall_setup_msr();
}

// ============================================================================
// Registro en el IDT para int 0x80 (syscall legacy)
// ============================================================================

/// Registra el manejador de syscall en la entrada 0x80 de la IDT.
pub fn register_idt_handler() {
    // La entrada 0x80 de la IDT ya está configurada con default_isr.
    // Aquí se sobrescribiría con el manejador de syscall específico.
    // Se deja como placeholder para integrar con idt::set_handler.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translate_elf_write() {
        assert_eq!(translate_syscall(BinaryOrigin::ELF, 1), Some(SYSCALL_WRITE));
    }

    #[test]
    fn translate_pe_exit() {
        assert_eq!(translate_syscall(BinaryOrigin::PE, 0x1), Some(SYSCALL_EXIT));
    }

    #[test]
    fn translate_macho_exit() {
        assert_eq!(translate_syscall(BinaryOrigin::MachO, 1), Some(SYSCALL_EXIT));
    }

    #[test]
    fn translate_native() {
        assert_eq!(translate_syscall(BinaryOrigin::Native, 42), Some(42));
    }

    #[test]
    fn translate_unknown() {
        assert_eq!(translate_syscall(BinaryOrigin::PE, 999), None);
    }

    #[test]
    fn syscall_error_codes() {
        assert_eq!(SyscallError::InvalidNr as isize, 0);
        assert_eq!(SyscallError::NotImplemented as isize, 2);
    }
}
