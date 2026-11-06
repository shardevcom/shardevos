//! shardevos-search — Indexador y buscador semántico de archivos en espacio de usuario.
//!
//! Modos:
//!   index  — lista inodos del VFS, lee cada archivo, genera embedding y guarda índice en memoria
//!   search — lee una consulta por serial, genera su embedding y devuelve top-k por coseno
//!
//! Nota: el embedding aquí es un placeholder ligero (hash sobre contenido). Para producción,
//! reemplazar `embed()` por una llamada a un modelo de embeddings quantizado
//! (ej. all-MiniLM-L6-v2 INT8 vía ggml/llama.cpp compilado como binario ELF de usuario).

#![no_std]
#![no_main]

#[path = "syscalls.rs"]
mod syscalls;
use syscalls::*;

const MAX_FILES: usize = 256;
const EMB_DIM: usize = 64;
const READ_CHUNK: usize = 4096;
const LIST_BUF: usize = 65536;

#[derive(Copy, Clone)]
#[repr(C)]
struct IndexEntry {
    inode_id: u64,
    size: u64,
    embedding: [f32; EMB_DIM],
}

static mut INDEX: [Option<IndexEntry>; MAX_FILES] = [None; MAX_FILES];
static mut INDEX_COUNT: usize = 0;

/// Panic handler para userspace no_std.
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    serial_println("PANIC en userspace search");
    loop {
        core::hint::spin_loop();
    }
}

/// Punto de entrada (llamado por el cargador ELF del kernel).
#[no_mangle]
pub extern "C" fn _start() -> ! {
    serial_println("shardevos-search v0.1.0");

    // Indexar automáticamente al arrancar (modo demo; sin entrada serial requerida)
    cmd_index();

    // Luego hacer una búsqueda de demostración con consulta "demo"
    serial_println("Busqueda demo: 'demo'");
    let query = b"demo";
    let query_emb = embed(query);
    serial_println("Resultados (top-5):");
    let mut best: [(f32, usize); 5] = [(-1.0, 0); 5];
    let count = unsafe { INDEX_COUNT };
    for i in 0..count {
        if let Some(entry) = unsafe { INDEX[i] } {
            let sim = cosine(&query_emb, &entry.embedding);
            for j in 0..5 {
                if sim > best[j].0 {
                    for k in (1..=j).rev() {
                        best[k] = best[k - 1];
                    }
                    best[j] = (sim, i);
                    break;
                }
            }
        }
    }
    for j in 0..5 {
        if best[j].0 > 0.0 {
            if let Some(entry) = unsafe { INDEX[best[j].1] } {
                serial_print_hex(entry.inode_id);
                serial_print("  sim=");
                serial_print_f32(best[j].0);
                serial_print("  size=");
                serial_print_usize(entry.size as usize);
            }
        }
    }

    serial_println("Busqueda completada.");
    sys_exit(0);
}

/// Genera un embedding placeholder (64 f32) a partir del contenido.
/// Para producción: llamar a un modelo de embeddings real (ggml/llama.cpp).
fn embed(content: &[u8]) -> [f32; EMB_DIM] {
    let mut emb = [0.0f32; EMB_DIM];
    if content.is_empty() {
        return emb;
    }
    let mut h: u64 = 1469598103934665603; // FNV offset
    for &b in content {
        h ^= b as u64;
        h = h.wrapping_mul(1099511628211); // FNV prime
        for i in 0..EMB_DIM {
            let bit = ((h >> (i % 64)) & 1) as f32;
            emb[i] += bit * (b as f32 / 255.0);
        }
    }
    // Normalizar L2
    let mut norm: f32 = 0.0;
    for &v in emb.iter() {
        norm += v * v;
    }
    norm = sqrt_f32(norm);
    if norm > 0.0 {
        for v in emb.iter_mut() {
            *v /= norm;
        }
    }
    emb
}

/// Similitud de coseno entre dos vectores ya normalizados.
fn cosine(a: &[f32; EMB_DIM], b: &[f32; EMB_DIM]) -> f32 {
    let mut s: f32 = 0.0;
    for i in 0..EMB_DIM {
        s += a[i] * b[i];
    }
    s
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

                    // Leer contenido del archivo en chunks (primer chunk para embedding placeholder)
                    let mut chunk = [0u8; READ_CHUNK];
                    let mut total_read = 0usize;
                    let mut file_offset = 0usize;
                    let size_usize = size as usize;
                    let limit = size_usize.min(65536); // limitar a 64 KB por archivo

                    while total_read < limit {
                        let to_read = (limit - total_read).min(READ_CHUNK);
                        match sys_vfs_read_by_inode(inode_id as usize, file_offset, &mut chunk[..to_read]) {
                            Ok(r) if r > 0 => {
                                // Copiar al contenido acumulado (buffer fijo)
                                let end = (total_read + r).min(65536);
                                // Aquí no podemos acumular en Vec; usamos embed incremental
                                // Para simplicidad, embed solo el primer chunk
                                if total_read == 0 {
                                    // embed del primer chunk (placeholder)
                                    let emb = embed(&chunk[..r]);
                                    if unsafe { INDEX_COUNT } < MAX_FILES {
                                        unsafe {
                                            INDEX[INDEX_COUNT] = Some(IndexEntry {
                                                inode_id,
                                                size,
                                                embedding: emb,
                                            });
                                            INDEX_COUNT += 1;
                                        }
                                        indexed += 1;
                                    }
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

/// Comando search: lee consulta, busca top-k por coseno.
fn cmd_search() {
    serial_println("Ingrese consulta:");
    let mut query_buf = [0u8; 256];
    let n = match sys_read(0, &mut query_buf) {
        Ok(n) => n,
        _ => 0,
    };
    if n == 0 {
        serial_println("Consulta vacía");
        return;
    }
    let query = &query_buf[..n];
    let query_emb = embed(query);

    // Buscar top-5
    let mut best: [(f32, usize); 5] = [(-1.0, 0); 5];
    let count = unsafe { INDEX_COUNT };
    for i in 0..count {
        if let Some(entry) = unsafe { INDEX[i] } {
            let sim = cosine(&query_emb, &entry.embedding);
            // Insertar en best ordenado
            for j in 0..5 {
                if sim > best[j].0 {
                    for k in (1..=j).rev() {
                        best[k] = best[k - 1];
                    }
                    best[j] = (sim, i);
                    break;
                }
            }
        }
    }

    serial_println("Resultados (top-5):");
    for j in 0..5 {
        if best[j].0 > 0.0 {
            if let Some(entry) = unsafe { INDEX[best[j].1] } {
                serial_print_hex(entry.inode_id);
                serial_print("  sim=");
                serial_print_f32(best[j].0);
                serial_print("  size=");
                serial_print_usize(entry.size as usize);
            }
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

/// Raíz cuadrada aproximada (Newton-Raphson) para no_std sin f32::sqrt.
fn sqrt_f32(mut x: f32) -> f32 {
    if x == 0.0 {
        return 0.0;
    }
    if x < 0.0 {
        return 0.0;
    }
    let mut g = x / 2.0;
    if g == 0.0 {
        g = 1.0;
    }
    for _ in 0..8 {
        g = (g + x / g) / 2.0;
    }
    g
}
