//! shardevos-search — Indexador y buscador semántico de archivos en espacio de usuario.
//!
//! Modos:
//!   index  — lista inodos del VFS, lee cada archivo, genera embedding y guarda índice en memoria
//!   search — lee una consulta por serial, genera su embedding y devuelve top-k por coseno
//!
//! Usa el módulo de embeddings quantizado (INT8) para generar vectores semánticos reales
//! en lugar del placeholder basado en hash FNV.

#![no_std]
#![no_main]

mod syscalls;
mod embeddings;
use syscalls::*;
use embeddings::*;

const READ_CHUNK: usize = 4096;
const LIST_BUF: usize = 65536;
const MODEL_BUF_LEN: usize = 65536;

/// Buffer estático para el modelo serializado .semb (evita alloc en no_std).
static mut MODEL_BUF: [u8; MODEL_BUF_LEN] = [0u8; MODEL_BUF_LEN];

const MODEL_PATH_LEN: usize = 17;
static MODEL_PATH_BYTES: [u8; MODEL_PATH_LEN] = *b"/minilm_int8.semb";

/// Devuelve la ruta del modelo serializado de embeddings.
fn model_path() -> &'static [u8] {
    &MODEL_PATH_BYTES
}

/// Punto de entrada del kernel (llamado por el cargador ELF).
#[no_mangle]
pub extern "C" fn _start() -> ! {
    serial_println("shardevos-search v0.2.0");
    serial_println("Inicializando motor de embeddings...");
    let mut model_loaded = false;
    if load_external_model() {
        model_loaded = true;
    }
    if !model_loaded {
        unsafe {
            embeddings::init_embedding_model();
        }
    }
    serial_println(if model_loaded {
        "Modelo .semb cargado."
    } else {
        "Usando modelo integrado."
    });

    // Indexar automáticamente al arrancar (modo demo; sin entrada serial requerida)
    cmd_index();

    // Búsqueda de demostración usando el conector semántico
    serial_println("Busqueda demo: 'archivo'");
    serial_println("Resultados (top-5):");
    let mut hits = [SearchHit::empty(); 5];
    let found = semantic_search("archivo", &mut hits);
    for hit in hits.iter().take(found) {
        serial_print_hex(hit.inode_id);
        serial_print("  sim=");
        serial_print_f32(hit.similarity);
    }

    serial_println("Busqueda completada.");
    sys_exit(0);
}

/// Carga el modelo serializado (.semb) desde el VFS. Devuelve true si fue exitoso.
fn load_external_model() -> bool {
    unsafe {
        let inode = match sys_vfs_lookup(model_path()) {
            Ok(inode_id) => inode_id,
            Err(e) => {
                let _ = e;
                return false;
            }
        };

        let mut total: usize = 0;
        let mut chunk = [0u8; READ_CHUNK];
        while total < MODEL_BUF_LEN {
            let n = match sys_vfs_read_by_inode(inode, total, &mut chunk) {
                Ok(n) => n,
                Err(e) => {
                    let _ = e;
                    return false;
                }
            };
            if n == 0 {
                break;
            }
            MODEL_BUF[total..total + n].copy_from_slice(&chunk[..n]);
            total += n;
            if n < chunk.len() {
                break;
            }
        }

        let result = crate::embeddings::load_embedding_model_from_bytes(&MODEL_BUF[..total]);
        result.is_ok()
    }
}

/// Comando index: lista inodos, lee archivos, genera embeddings.
fn cmd_index() {
    let mut list_buf = [0u8; LIST_BUF];
    match sys_vfs_list(&mut list_buf) {
        Ok(written) => {
            if written < 8 {
                serial_println("VFS_LIST devolvió menos de 8 bytes");
                return;
            }
            let count = unsafe { core::ptr::read_volatile(list_buf.as_ptr() as *const u64) } as usize;
            serial_print_usize(count);
            serial_println(" inodos en el VFS");

            let mut offset = 8usize;
            let mut indexed = 0usize;
            for _ in 0..count {
                if offset + 17 > written {
                    break;
                }
                unsafe {
                    let inode_id = core::ptr::read_volatile(list_buf.as_ptr().add(offset) as *const u64);
                    let size = core::ptr::read_volatile(list_buf.as_ptr().add(offset + 8) as *const u64);
                    let node_type = core::ptr::read_volatile(list_buf.as_ptr().add(offset + 16) as *const u8);
                    offset += 17;

                    if node_type != 0 {
                        // 0 = RegularFile; saltar directorios y otros
                        continue;
                    }

                    // Leer contenido del archivo (primer chunk para embedding)
                    let mut chunk = [0u8; READ_CHUNK];
                    let mut total_read = 0usize;
                    let mut file_offset = 0usize;
                    let size_usize = size as usize;
                    let limit = size_usize.min(65536); // limitar a 64 KB por archivo

                    let mut embedded = false;
                    while total_read < limit {
                        let to_read = (limit - total_read).min(READ_CHUNK);
                        match sys_vfs_read_by_inode(inode_id as usize, file_offset, &mut chunk[..to_read]) {
                            Ok(r) if r > 0 => {
                                if !embedded {
                                    // Generar embedding real y registrarlo en el conector semántico
                                    if let Ok(text) = core::str::from_utf8(&chunk[..r]) {
                                        if index_document(inode_id, text) {
                                            indexed += 1;
                                        }
                                    }
                                    embedded = true;
                                }
                                total_read += r;
                                file_offset += r;
                                if r < to_read {
                                    break;
                                }
                            }
                            _ => break,
                        }
                    }
                }
            }
            serial_print_usize(indexed);
            serial_println(" archivos indexados");
        }
        Err(e) => {
            serial_println("Error en VFS_LIST: ");
            serial_print_hex(e as isize as u64);
        }
    }
}

/// Imprime un f32 con 2 decimales por serial.
fn serial_print_f32(val: f32) {
    let mut v = (val * 100.0) as i32;
    let neg = v < 0;
    if neg {
        v = -v;
    }
    let int_part = v / 100;
    let dec_part = v % 100;
    if neg {
        let _ = sys_write(1, b"-");
    }
    serial_print_usize(int_part as usize);
    let _ = sys_write(1, b".");
    if dec_part < 10 {
        let _ = sys_write(1, b"0");
    }
    serial_print_usize(dec_part as usize);
    serial_println("");
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

/// Imprime una cadena por serial (stdout).
pub fn serial_print(s: &str) {
    let _ = sys_write(1, s.as_bytes());
}

/// Imprime una línea por serial (stdout).
pub fn serial_println(s: &str) {
    let _ = sys_write(1, s.as_bytes());
    let _ = sys_write(1, b"\n");
}

/// Punto de entrada del kernel (llamado por el cargador ELF).
#[no_mangle]
pub extern "C" fn _start_kernel() -> ! {
    serial_println("shardevos-search v0.2.0");
    serial_println("Inicializando motor de embeddings...");
    unsafe {
        embeddings::init_embedding_model();
    }
    serial_println("Modelo de embeddings listo.");

    // Indexar automáticamente al arrancar
    cmd_index();

    serial_println("Indexador listo.");
    sys_exit(0);
}

/// Panic handler para userspace no_std.
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    serial_println("PANIC en userspace search");
    loop {
        core::hint::spin_loop();
    }
}
