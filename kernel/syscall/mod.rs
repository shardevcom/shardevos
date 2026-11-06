//! Capa de syscalls unificada del kernel shardevos.
//!
//! Proporciona:
//! - Números de syscall unificados (independientes del formato del binario)
//! - Manejador de entrada de syscalls (x86_64: instrucción `syscall`)
//! - Dispatch table y traducciones para PE (Win32), ELF (Linux), MachO (macOS)
//! - API de registro de manejadores por número de syscall

use core::arch::asm;

// Acceso al Unified Input Manager a través del crate root
use crate::input::global_manager;
use crate::input::{InputError as InputMgrError, InputEvent as InputMgrEvent};

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
    NotImplemented,
    Denied,
    Fault,
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
    unsafe {
        match SYSCALL_TABLE[args.nr as usize] {
            Some(handler) => handler(args),
            None => Err(SyscallError::InvalidNr),
        }
    }
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
