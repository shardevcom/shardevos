//! Display — Servidor de display gráfico de shardevos.
//!
//! Proporciona:
//! - Framebuffer en modo gráfico VESA (800x600, 32-bit RGBA)
//! - Dibujo de primitivas (rectángulos, texto, líneas)
//! - Doble buffer para evitar parpadeo
//! - API para renderizar UI (ventanas, botones, texto)

use core::arch::asm;
use core::ptr::{write_volatile, read_volatile};
use alloc::vec::Vec;
use alloc::string::{String, ToString};

// ============================================================================
// Modo de video VESA (800x600 @ 32-bit)
// ============================================================================

const VBE_DISPI_IOPORT_INDEX: u16 = 0x01CE;
const VBE_DISPI_IOPORT_DATA: u16 = 0x01CF;

const VBE_DISPI_INDEX_ID: u16 = 0;
const VBE_DISPI_INDEX_XRES: u16 = 1;
const VBE_DISPI_INDEX_YRES: u16 = 2;
const VBE_DISPI_INDEX_BPP: u16 = 3;
const VBE_DISPI_INDEX_ENABLE: u16 = 4;
const VBE_DISPI_INDEX_BANK: u16 = 5;
const VBE_DISPI_INDEX_VIRT_WIDTH: u16 = 6;
const VBE_DISPI_INDEX_VIRT_HEIGHT: u16 = 7;
const VBE_DISPI_INDEX_X_OFFSET: u16 = 8;
const VBE_DISPI_INDEX_Y_OFFSET: u16 = 9;

const VBE_DISPI_DISABLED: u16 = 0;
const VBE_DISPI_ENABLED: u16 = 1;
const VBE_DISPI_GETCAPS: u16 = 0x01;
const VBE_DISPI_8BIT_DAC: u16 = 0x02;
const VBE_DISPI_LFB_ENABLED: u16 = 0x40;
const VBE_DISPI_NOCLEARMEM: u16 = 0x80;

/// Dirección del framebuffer LFB (Linear Frame Buffer) de VBE.
pub const FB_ADDR: usize = 0xE000_0000;

/// Resolución por defecto.
pub const FB_WIDTH: u32 = 800;
pub const FB_HEIGHT: u32 = 600;
pub const FB_BPP: u32 = 32;

/// Formato de color: BGRA (0xAABBGGRR en little-endian)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C, packed)]
pub struct Color {
    pub b: u8,
    pub g: u8,
    pub r: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { b, g, r, a: 0 }
    }

    pub const BLACK: Color = Color::rgb(0, 0, 0);
    pub const WHITE: Color = Color::rgb(255, 255, 255);
    pub const RED: Color = Color::rgb(255, 0, 0);
    pub const GREEN: Color = Color::rgb(0, 255, 0);
    pub const BLUE: Color = Color::rgb(0, 0, 255);
    pub const YELLOW: Color = Color::rgb(255, 255, 0);
    pub const CYAN: Color = Color::rgb(0, 255, 255);
    pub const MAGENTA: Color = Color::rgb(255, 0, 255);
    pub const GRAY: Color = Color::rgb(128, 128, 128);
    pub const LIGHT_GRAY: Color = Color::rgb(200, 200, 200);
    pub const DARK_GRAY: Color = Color::rgb(64, 64, 64);
}

/// Estado del servidor de display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayState {
    Uninitialized,
    TextMode,    // Modo texto VGA (80x25)
    GraphicsMode, // Modo gráfico VESA
}

/// Información del framebuffer.
pub struct FramebufferInfo {
    pub addr: usize,
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub bpp: u32,
}

/// Servidor de display global.
pub struct DisplayServer {
    pub state: DisplayState,
    pub fb_info: FramebufferInfo,
    pub front_buffer: *mut Color,
    pub back_buffer: *mut Color,
    pub initialized: bool,
}

impl DisplayServer {
    pub const fn new() -> Self {
        Self {
            state: DisplayState::Uninitialized,
            fb_info: FramebufferInfo {
                addr: 0,
                width: 0,
                height: 0,
                pitch: 0,
                bpp: 0,
            },
            front_buffer: core::ptr::null_mut(),
            back_buffer: core::ptr::null_mut(),
            initialized: false,
        }
    }

    /// Inicializa el modo gráfico VESA 800x600@32.
    pub unsafe fn init_graphics(&mut self) -> Result<(), &'static str> {
        // Intentar establecer modo VESA 800x600@32
        self.set_vbe_mode(FB_WIDTH, FB_HEIGHT, FB_BPP)?;

        self.fb_info = FramebufferInfo {
            addr: FB_ADDR,
            width: FB_WIDTH,
            height: FB_HEIGHT,
            pitch: FB_WIDTH * (FB_BPP / 8),
            bpp: FB_BPP,
        };

        self.front_buffer = FB_ADDR as *mut Color;
        // Back buffer en una página de memoria asignada
        let back_size = (FB_WIDTH as usize) * (FB_HEIGHT as usize) * 4;
        let back_pages = (back_size + 4095) / 4096;
        // Usamos una dirección alta para el back buffer (temporal)
        self.back_buffer = (0xF000_0000usize) as *mut Color;

        self.state = DisplayState::GraphicsMode;
        self.initialized = true;

        // Limpiar pantalla
        self.clear(Color::BLACK);

        Ok(())
    }

    /// Establece el modo VESA mediante las extensiones de Bochs/QEMU.
    unsafe fn set_vbe_mode(&mut self, width: u32, height: u32, bpp: u32) -> Result<(), &'static str> {
        // Escribir índice y dato en los puertos VBE de Bochs
        // Esto solo funciona en QEMU con VBE habilitado

        // Habilitar LFB + modo gráfico
        asm!("out dx, ax", in("dx") VBE_DISPI_IOPORT_INDEX, in("ax") (VBE_DISPI_INDEX_ENABLE | VBE_DISPI_LFB_ENABLED), options(nostack));
        asm!("out dx, ax", in("dx") VBE_DISPI_IOPORT_DATA, in("ax") (VBE_DISPI_ENABLED | VBE_DISPI_LFB_ENABLED), options(nostack));

        // Establecer resolución X
        asm!("out dx, ax", in("dx") VBE_DISPI_IOPORT_INDEX, in("ax") VBE_DISPI_INDEX_XRES, options(nostack));
        asm!("out dx, ax", in("dx") VBE_DISPI_IOPORT_DATA, in("ax") width as u16, options(nostack));

        // Establecer resolución Y
        asm!("out dx, ax", in("dx") VBE_DISPI_IOPORT_INDEX, in("ax") VBE_DISPI_INDEX_YRES, options(nostack));
        asm!("out dx, ax", in("dx") VBE_DISPI_IOPORT_DATA, in("ax") height as u16, options(nostack));

        // Establecer BPP
        asm!("out dx, ax", in("dx") VBE_DISPI_IOPORT_INDEX, in("ax") VBE_DISPI_INDEX_BPP, options(nostack));
        asm!("out dx, ax", in("dx") VBE_DISPI_IOPORT_DATA, in("ax") bpp as u16, options(nostack));

        // Verificar que el modo se estableció (leer el ID)
        asm!("out dx, ax", in("dx") VBE_DISPI_IOPORT_INDEX, in("ax") VBE_DISPI_INDEX_ID, options(nostack));
        let id: u16;
        asm!("in ax, dx", out("ax") id, in("dx") VBE_DISPI_IOPORT_DATA, options(nostack));

        if id != 0xB0C5 {
            return Err("VBE no soportado en este entorno");
        }

        Ok(())
    }

    /// Limpia toda la pantalla con un color.
    pub unsafe fn clear(&mut self, color: Color) {
        if !self.initialized {
            return;
        }
        let pixels = (FB_WIDTH as usize) * (FB_HEIGHT as usize);
        for i in 0..pixels {
            write_volatile(self.front_buffer.add(i), color);
        }
    }

    /// Dibuja un píxel en la posición (x, y).
    pub unsafe fn put_pixel(&mut self, x: u32, y: u32, color: Color) {
        if !self.initialized || x >= FB_WIDTH || y >= FB_HEIGHT {
            return;
        }
        let idx = (y as usize) * (FB_WIDTH as usize) + (x as usize);
        write_volatile(self.front_buffer.add(idx), color);
    }

    /// Dibuja un rectángulo relleno.
    pub unsafe fn fill_rect(&mut self, x: u32, y: u32, w: u32, h: u32, color: Color) {
        if !self.initialized {
            return;
        }
        for dy in 0..h {
            for dx in 0..w {
                self.put_pixel(x + dx, y + dy, color);
            }
        }
    }

    /// Dibuja una línea horizontal.
    pub unsafe fn hline(&mut self, x: u32, y: u32, w: u32, color: Color) {
        self.fill_rect(x, y, w, 1, color);
    }

    /// Dibuja una línea vertical.
    pub unsafe fn vline(&mut self, x: u32, y: u32, h: u32, color: Color) {
        self.fill_rect(x, y, 1, h, color);
    }

    /// Dibuja el borde de un rectángulo.
    pub unsafe fn draw_rect(&mut self, x: u32, y: u32, w: u32, h: u32, color: Color) {
        self.hline(x, y, w, color);
        self.hline(x, y + h.saturating_sub(1), w, color);
        self.vline(x, y, h, color);
        self.vline(x + w.saturating_sub(1), y, h, color);
    }

    /// Dibuja un carácter 8x8 simple (bitmap de fuente 5x7 escalado).
    pub unsafe fn draw_char(&mut self, x: u32, y: u32, ch: char, color: Color, bg: Color) {
        if !self.initialized {
            return;
        }

        let font = get_font_char(ch as u8);
        for row in 0..8 {
            let bits = font[row];
            for col in 0..8 {
                let pixel_on = (bits >> (7 - col)) & 1 == 1;
                let px_color = if pixel_on { color } else { bg };
                self.put_pixel(x + col as u32, y + row as u32, px_color);
            }
        }
    }

    /// Dibuja una cadena de texto.
    pub unsafe fn draw_string(&mut self, x: u32, y: u32, text: &str, color: Color, bg: Color) {
        if !self.initialized {
            return;
        }
        let mut cx = x;
        for ch in text.chars() {
            if ch == '\n' {
                cx = x;
                // y += 9; // No implementado para simplicidad
                return;
            }
            self.draw_char(cx, y, ch, color, bg);
            cx += 8;
            if cx + 8 > FB_WIDTH {
                break;
            }
        }
    }

    /// Obtiene una referencia al framebuffer para acceso directo.
    pub fn front_buffer_mut(&mut self) -> &mut [Color] {
        unsafe { core::slice::from_raw_parts_mut(self.front_buffer, (FB_WIDTH as usize) * (FB_HEIGHT as usize)) }
    }

    /// Devuelve la información del framebuffer.
    pub fn info(&self) -> &FramebufferInfo {
        &self.fb_info
    }
}

/// Fuente 8x8 simplificada (bitmaps para caracteres básicos).
fn get_font_char(ch: u8) -> [u8; 8] {
    match ch {
        b'A' => [0x18, 0x3C, 0x66, 0x66, 0x7E, 0x66, 0x66, 0x00],
        b'B' => [0x7C, 0x66, 0x66, 0x7C, 0x66, 0x66, 0x7C, 0x00],
        b'C' => [0x3C, 0x66, 0x60, 0x60, 0x60, 0x66, 0x3C, 0x00],
        b'D' => [0x7C, 0x66, 0x66, 0x66, 0x66, 0x66, 0x7C, 0x00],
        b'E' => [0x7E, 0x60, 0x60, 0x7E, 0x60, 0x60, 0x7E, 0x00],
        b'F' => [0x7E, 0x60, 0x60, 0x7E, 0x60, 0x60, 0x60, 0x00],
        b'G' => [0x3C, 0x66, 0x60, 0x6E, 0x66, 0x66, 0x3C, 0x00],
        b'H' => [0x66, 0x66, 0x66, 0x7E, 0x66, 0x66, 0x66, 0x00],
        b'I' => [0x7E, 0x18, 0x18, 0x18, 0x18, 0x18, 0x7E, 0x00],
        b'J' => [0x1C, 0x0C, 0x0C, 0x0C, 0x0C, 0x6C, 0x38, 0x00],
        b'K' => [0x66, 0x6C, 0x78, 0x70, 0x78, 0x6C, 0x66, 0x00],
        b'L' => [0x60, 0x60, 0x60, 0x60, 0x60, 0x60, 0x7E, 0x00],
        b'M' => [0x66, 0x7E, 0x7E, 0x6E, 0x66, 0x66, 0x66, 0x00],
        b'N' => [0x66, 0x76, 0x7E, 0x7E, 0x6E, 0x66, 0x66, 0x00],
        b'O' => [0x3C, 0x66, 0x66, 0x66, 0x66, 0x66, 0x3C, 0x00],
        b'P' => [0x7C, 0x66, 0x66, 0x7C, 0x60, 0x60, 0x60, 0x00],
        b'Q' => [0x3C, 0x66, 0x66, 0x66, 0x6E, 0x66, 0x3C, 0x0A],
        b'R' => [0x7C, 0x66, 0x66, 0x7C, 0x78, 0x6C, 0x66, 0x00],
        b'S' => [0x3E, 0x60, 0x60, 0x3C, 0x06, 0x06, 0x3C, 0x00],
        b'T' => [0x7E, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x00],
        b'U' => [0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x3C, 0x00],
        b'V' => [0x66, 0x66, 0x66, 0x66, 0x66, 0x3C, 0x18, 0x00],
        b'W' => [0x66, 0x66, 0x66, 0x66, 0x6E, 0x7E, 0x66, 0x00],
        b'X' => [0x66, 0x66, 0x3C, 0x18, 0x3C, 0x66, 0x66, 0x00],
        b'Y' => [0x66, 0x66, 0x66, 0x3C, 0x18, 0x18, 0x18, 0x00],
        b'Z' => [0x7E, 0x06, 0x0C, 0x18, 0x30, 0x60, 0x7E, 0x00],
        b'a' => [0x00, 0x00, 0x3E, 0x06, 0x3E, 0x66, 0x3E, 0x00],
        b'b' => [0x60, 0x60, 0x7C, 0x66, 0x66, 0x66, 0x7C, 0x00],
        b'c' => [0x00, 0x00, 0x3C, 0x60, 0x60, 0x60, 0x3C, 0x00],
        b'd' => [0x06, 0x06, 0x3E, 0x66, 0x66, 0x66, 0x3E, 0x00],
        b'e' => [0x00, 0x00, 0x3C, 0x66, 0x7E, 0x60, 0x3C, 0x00],
        b'f' => [0x0C, 0x18, 0x7C, 0x18, 0x18, 0x18, 0x18, 0x00],
        b'g' => [0x00, 0x00, 0x3E, 0x66, 0x3E, 0x06, 0x3E, 0x00],
        b'h' => [0x60, 0x60, 0x7C, 0x66, 0x66, 0x66, 0x66, 0x00],
        b'i' => [0x18, 0x00, 0x18, 0x18, 0x00, 0x18, 0x18, 0x00],
        b'j' => [0x0C, 0x00, 0x0C, 0x0C, 0x00, 0x6C, 0x38, 0x00],
        b'k' => [0x60, 0x60, 0x6C, 0x78, 0x70, 0x6C, 0x66, 0x00],
        b'l' => [0x18, 0x00, 0x18, 0x18, 0x18, 0x18, 0x18, 0x00],
        b'm' => [0x00, 0x00, 0x7C, 0x6E, 0x66, 0x66, 0x66, 0x00],
        b'n' => [0x00, 0x00, 0x7C, 0x66, 0x66, 0x66, 0x66, 0x00],
        b'o' => [0x00, 0x00, 0x3C, 0x66, 0x66, 0x66, 0x3C, 0x00],
        b'p' => [0x00, 0x00, 0x7C, 0x66, 0x66, 0x7C, 0x60, 0x60],
        b'q' => [0x00, 0x00, 0x3E, 0x66, 0x66, 0x3E, 0x06, 0x06],
        b'r' => [0x00, 0x00, 0x7C, 0x60, 0x60, 0x60, 0x60, 0x00],
        b's' => [0x00, 0x00, 0x3E, 0x06, 0x3E, 0x60, 0x3E, 0x00],
        b't' => [0x18, 0x00, 0x18, 0x18, 0x7C, 0x66, 0x18, 0x00],
        b'u' => [0x00, 0x00, 0x66, 0x66, 0x66, 0x66, 0x3E, 0x00],
        b'v' => [0x00, 0x00, 0x66, 0x66, 0x66, 0x3C, 0x18, 0x00],
        b'w' => [0x00, 0x00, 0x66, 0x66, 0x6E, 0x7E, 0x66, 0x00],
        b'x' => [0x00, 0x00, 0x66, 0x3C, 0x18, 0x3C, 0x66, 0x00],
        b'y' => [0x00, 0x00, 0x66, 0x66, 0x3E, 0x06, 0x3E, 0x00],
        b'z' => [0x00, 0x00, 0x7E, 0x0C, 0x18, 0x30, 0x7E, 0x00],
        b'0' => [0x3C, 0x66, 0x66, 0x66, 0x66, 0x66, 0x3C, 0x00],
        b'1' => [0x18, 0x38, 0x18, 0x18, 0x18, 0x18, 0x7E, 0x00],
        b'2' => [0x3C, 0x66, 0x06, 0x0C, 0x18, 0x30, 0x7E, 0x00],
        b'3' => [0x3C, 0x66, 0x06, 0x1C, 0x06, 0x66, 0x3C, 0x00],
        b'4' => [0x06, 0x0E, 0x1E, 0x36, 0x66, 0x7E, 0x06, 0x00],
        b'5' => [0x7E, 0x60, 0x7C, 0x06, 0x06, 0x66, 0x3C, 0x00],
        b'6' => [0x1C, 0x30, 0x60, 0x7C, 0x66, 0x66, 0x3C, 0x00],
        b'7' => [0x7E, 0x06, 0x0C, 0x18, 0x30, 0x30, 0x30, 0x00],
        b'8' => [0x3C, 0x66, 0x66, 0x3C, 0x66, 0x66, 0x3C, 0x00],
        b'9' => [0x3C, 0x66, 0x66, 0x3E, 0x06, 0x0C, 0x3C, 0x00],
        b' ' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        b'.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x18, 0x00],
        b',' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x18, 0x18, 0x00],
        b':' => [0x00, 0x00, 0x18, 0x00, 0x18, 0x00, 0x00, 0x00],
        b';' => [0x00, 0x00, 0x18, 0x00, 0x18, 0x00, 0x18, 0x00],
        b'!' => [0x18, 0x18, 0x18, 0x18, 0x18, 0x00, 0x18, 0x00],
        b'?' => [0x3C, 0x66, 0x06, 0x0C, 0x18, 0x00, 0x18, 0x00],
        b'-' => [0x00, 0x00, 0x00, 0x7E, 0x00, 0x00, 0x00, 0x00],
        b'_' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x7E, 0x00],
        b'+' => [0x00, 0x18, 0x18, 0x7E, 0x18, 0x18, 0x00, 0x00],
        b'=' => [0x00, 0x00, 0x7E, 0x00, 0x7E, 0x00, 0x00, 0x00],
        b'/' => [0x00, 0x06, 0x0C, 0x18, 0x30, 0x60, 0x00, 0x00],
        b'\\' => [0x00, 0x60, 0x30, 0x18, 0x0C, 0x06, 0x00, 0x00],
        b'|' => [0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x00],
        _ => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
    }
}

/// Instancia global del servidor de display.
static mut DISPLAY: DisplayServer = DisplayServer::new();

/// Devuelve una referencia mutable al display global.
pub fn global_display() -> &'static mut DisplayServer {
    unsafe { &mut DISPLAY }
}
