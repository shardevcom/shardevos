#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use crate::fs::Filesystem;
use crate::ui_manager::display::Color;
use crate::ui_manager::global_gfx_display;

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
pub mod security;
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

    // Inicializar gestor de usuarios y grupos
    unsafe {
        crate::security::init_user_manager();
        crate::vga_print_line("  [ OK ] Gestor de usuarios inicializado.", 21, 0x0A);
    }

    // Inicializar modo gráfico VESA para la UI
    unsafe {
        let display = global_gfx_display();
        match display.init_graphics() {
            Ok(()) => {
                // Fondo degradado oscuro
                for y in 0..600 {
                    for x in 0..800 {
                        let intensity = ((y as u32) * 255 / 600) as u8;
                        display.put_pixel(x, y, Color::rgb(intensity / 3, intensity / 2, intensity));
                    }
                }

                // Barra de título
                display.fill_rect(0, 0, 800, 40, Color::rgb(30, 60, 120));
                display.draw_string(20, 10, "shardevOS v0.1.0 - Sistema Operativo Inteligente", Color::WHITE, Color::rgb(30, 60, 120));
                display.draw_string(20, 30, "Rust + Assembly x86_64  |  Busqueda semantica de archivos  |  Motor IA local", Color::LIGHT_GRAY, Color::rgb(30, 60, 120));

                // Información del sistema
                display.draw_string(20, 60, "Estado: KERNEL INICIALIZADO", Color::GREEN, Color::rgb(0, 0, 0));
                display.draw_string(20, 75, "VFS: shardevfs montado en /", Color::WHITE, Color::rgb(0, 0, 0));
                display.draw_string(20, 90, "Syscalls: 28 implementados", Color::WHITE, Color::rgb(0, 0, 0));
                display.draw_string(20, 105, "Scheduler: Preemptivo (PIT 100Hz)", Color::WHITE, Color::rgb(0, 0, 0));
                display.draw_string(20, 120, "Paginacion: 4 niveles con proteccion US", Color::WHITE, Color::rgb(0, 0, 0));

                // Seccion de busqueda
                display.fill_rect(20, 140, 760, 2, Color::rgb(100, 100, 100));
                display.draw_string(20, 150, "Archivos indexados en shardevfs:", Color::YELLOW, Color::rgb(0, 0, 0));
                display.draw_string(40, 165, "  /hello.txt - Prueba de indexacion semantica", Color::WHITE, Color::rgb(0, 0, 0));
                display.draw_string(40, 180, "  /readme.txt - Informacion del sistema", Color::WHITE, Color::rgb(0, 0, 0));
                display.draw_string(40, 195, "  /search_demo.txt - Buscador semantico", Color::WHITE, Color::rgb(0, 0, 0));

                // Resultado de búsqueda demo
                display.draw_string(20, 220, "Busqueda demo: 'archivo'", Color::CYAN, Color::rgb(0, 0, 0));
                display.draw_string(40, 235, "  [1] /hello.txt  similitud: 0.85", Color::GREEN, Color::rgb(0, 0, 0));
                display.draw_string(40, 250, "  [2] /readme.txt  similitud: 0.45", Color::GREEN, Color::rgb(0, 0, 0));

                // Barra inferior
                display.fill_rect(0, 560, 800, 40, Color::rgb(30, 60, 120));
                display.draw_string(20, 570, "shardevOS - Sistema Operativo Inteligente v0.1.0", Color::WHITE, Color::rgb(30, 60, 120));
                display.draw_string(400, 570, "UI Grafica: Modo VESA 800x600@32", Color::LIGHT_GRAY, Color::rgb(30, 60, 120));
            }
            Err(e) => {
                // Fallback a modo texto si VBE no disponible
                let _ = e;
                crate::vga_print_line("  [ WARN ] Modo grafico no disponible, usando texto.", 17, 0x0C);
            }
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
