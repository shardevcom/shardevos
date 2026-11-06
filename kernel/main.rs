#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use crate::fs::Filesystem;

pub mod ai_engine;
#[path = "arch/x86_64/mod.rs"]
pub mod arch;
#[path = "drivers/block/mod.rs"]
pub mod block;
#[path = "drivers/gpu_npu/mod.rs"]
pub mod gpu_npu;
#[path = "drivers/input/mod.rs"]
pub mod input;
#[path = "drivers/net/mod.rs"]
pub mod net;
pub mod fs;
pub mod mm;
#[path = "drivers/pci/mod.rs"]
pub mod pci;
pub mod sched;
pub mod syscall;
pub mod ui_manager;
pub mod userland_bin;
pub mod userland_loader;
#[path = "drivers/virtio_blk/mod.rs"]
pub mod virtio_blk;

pub use ui_manager::{
    DisplayMode, DisplayState, FramebufferInfo, PixelFormat, UiError, UiResult, next_window_id,
};

// Escribe una línea con color específico en pantalla VGA.
pub fn vga_print_line(line: &str, row: usize, color: u8) {
    if row >= 25 {
        return;
    }
    let vga: *mut u8 = 0xB8000 as *mut u8;
    let row_offset = row * 80 * 2;
    let mut col: usize = 0;
    for byte in line.bytes() {
        let pos = row_offset + col * 2;
        if pos >= 80 * 25 * 2 {
            break;
        }
        unsafe {
            core::ptr::write_volatile(vga.add(pos), byte);
            core::ptr::write_volatile(vga.add(pos + 1), color);
        }
        col += 1;
    }
}

// Logo ASCII de Shardev OS con bordes y colores.
const LOGO: [&str; 8] = [
    "╔════════════════════════════════════════════════╗",
    "║                                               ║",
    "║  ██████╗ ███████╗██╗   ██╗████████╗██████╗  ║",
    "║  ██╔══██╗██╔════╝╚██╗ ██╔╝╚══██╔══╝██╔══██╗ ║",
    "║  ██████╔╝█████╗   ╚████╔╝    ██║   ██████╔╝ ║",
    "║  ██╔═══╝ ██╔══╝    ╚██╔╝     ██║   ██╔══██╗ ║",
    "║  ███████╗███████╗   ██║      ██║   ██████╔╝ ║",
    "║  ╚══════╝╚══════╝   ╚═╝      ╚═╝   ╚══════╝  ║",
];

// Línea de información del sistema.
const INFO_LINES: [&str; 4] = [
    "║  Shardev OS - Sistema Operativo Inteligente v0.1.0 ║",
    "║  Escrito en Rust (no_std) + Assembly x86_64          ║",
    "║  Cargador universal: PE / ELF / MachO                ║",
    "║  Arranque completado correctamente                   ║",
];

// Punto de entrada lógico del kernel (llamado desde arch/x86_64/entry.rs).
#[no_mangle]
pub extern "C" fn kernel_main() -> ! {
    // Inicializar subsistema de interrupciones (GDT, IDT, PIC)
    unsafe {
        crate::arch::interrupts::init();
    }

    // Inicializar subsistema de syscalls
    unsafe {
        crate::syscall::init();
    }

    // Inicializar VFS y montar shardevfs en la raíz (/)
    unsafe {
        crate::fs::global_vfs().init();
        crate::fs::global_shardevfs_init();
        let _ = crate::fs::global_vfs().mount("shardevfs", "/", 1);

        // Crear archivos de prueba en shardevfs para indexación/búsqueda
        let fs = crate::fs::global_shardevfs_mut();
        let _ = fs.create_inode(1, "hello.txt", crate::fs::VfsNodeType::RegularFile);
        let _ = fs.write_data(2, 0, b"Hello, ShardevOS! Este es un archivo de prueba para indexacion y busqueda semantica.");
        let _ = fs.create_inode(1, "readme.txt", crate::fs::VfsNodeType::RegularFile);
        let _ = fs.write_data(3, 0, b"ShardevOS - Sistema Operativo Inteligente v0.1.0. Escrito en Rust + Assembly x86_64.");
        let _ = fs.create_inode(1, "search_demo.txt", crate::fs::VfsNodeType::RegularFile);
        let _ = fs.write_data(4, 0, b"Buscador semantico de archivos con indexacion vectorial y similitud de coseno.");
    }

    // Intentar inicializar el driver de bloque VirtIO (disco real en QEMU)
    unsafe {
        let blk = crate::virtio_blk::global_virtio_blk();
        match blk.discover_and_init() {
            Ok(()) => {
                crate::vga_print_line("  [ OK ] VirtIO-Blk dispositivo de bloque detectado.", 17, 0x0A);
                let info = blk.info();
                crate::vga_print_line("  [ INFO ] Capacidad: sectores", 18, 0x0F);
                // Mostrar capacidad en hex
                let cap_hex = format!("0x{:x}", info.capacity_sectors);
                crate::vga_print_line(&cap_hex, 19, 0x0F);
            }
            Err(e) => {
                crate::vga_print_line("  [ WARN ] No se encontro dispositivo VirtIO-Blk.", 17, 0x0C);
                let _ = e;
            }
        }
    }

    // Dibujar borde superior y logo con color azul claro (0x1F)
    vga_print_line(LOGO[0], 1, 0x1F);
    for i in 1..LOGO.len() {
        if i <= 7 {
            // Líneas del logo ASCII con color azul claro
            vga_print_line(LOGO[i], 1 + i, 0x1F);
        }
    }

    // Líneas de información con color blanco (0x0F)
    for i in 0..INFO_LINES.len() {
        vga_print_line(INFO_LINES[i], 10 + i, 0x0F);
    }

    // Borde inferior con color azul claro
    vga_print_line(LOGO[0], 14, 0x1F);

    // Mensaje de estado en la parte inferior con color verde (0x0A)
    vga_print_line("  [ OK ] Shardev OS inicializado. Sistema listo.", 16, 0x0A);

    // Cargar y ejecutar programa de usuario (binfmt_loader)
    vga_print_line("  [ INFO ] Cargando programa de usuario...", 18, 0x0F);
    match crate::userland_loader::load_and_run_userland(crate::userland_bin::USERLAND_SEARCH_ELF) {
        Ok(()) => {
            vga_print_line("  [ OK ] Programa de usuario ejecutado.", 19, 0x0A);
        }
        Err(e) => {
            vga_print_line("  [ ERR ] Error cargando usuario.", 19, 0x0C);
            vga_print_line(e, 20, 0x0C);
        }
    }

    loop {
        core::hint::spin_loop();
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
