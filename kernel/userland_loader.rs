extern crate alloc;

use alloc::format;

fn read_u16(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}

fn read_u32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

fn read_u64(b: &[u8], off: usize) -> u64 {
    u64::from_le_bytes([
        b[off], b[off + 1], b[off + 2], b[off + 3],
        b[off + 4], b[off + 5], b[off + 6], b[off + 7],
    ])
}

/// Carga y ejecuta un binario ELF64 embebido en el kernel.
pub fn load_and_run_userland(bytes: &[u8]) -> Result<(), &'static str> {
    // Cabecera ELF64
    if bytes.len() < 64 || &bytes[0..4] != b"\x7fELF" {
        return Err("No es un binario ELF válido");
    }
    if bytes[4] != 2 || bytes[5] != 1 {
        return Err("Se esperaba ELF64 little-endian");
    }

    let entry_point = read_u64(bytes, 24) as usize;
    let e_phoff = read_u64(bytes, 32) as usize;
    let e_phentsize = read_u16(bytes, 54) as usize;
    let e_phnum = read_u16(bytes, 56) as usize;

    if e_phnum == 0 || e_phentsize < 56 || e_phoff + e_phnum * e_phentsize > bytes.len() {
        return Err("Cabecera de programa inválida");
    }

    crate::vga_print_line("shardev OS: Cargando binario de usuario ELF64...", 18, 0x0F);
    crate::vga_print_line(&format!("shardev OS: Entry point en 0x{:x}", entry_point), 19, 0x0F);

    // Cargar cada segmento PT_LOAD en su dirección virtual (identidad < 2MB)
    let mut loaded = 0usize;
    for i in 0..e_phnum {
        let ph = e_phoff + i * e_phentsize;
        let p_type = read_u32(bytes, ph);
        if p_type != 1 {
            continue; // solo PT_LOAD
        }

        let p_offset = read_u64(bytes, ph + 8) as usize;
        let p_vaddr = read_u64(bytes, ph + 16) as usize;
        let p_filesz = read_u64(bytes, ph + 32) as usize;
        let p_memsz = read_u64(bytes, ph + 40) as usize;

        if p_offset + p_filesz > bytes.len() {
            return Err("Segmento fuera del archivo");
        }

        unsafe {
            core::ptr::copy_nonoverlapping(
                bytes.as_ptr().add(p_offset),
                p_vaddr as *mut u8,
                p_filesz,
            );
            // Rellenar la parte .bss (memsz > filesz) con ceros
            if p_memsz > p_filesz {
                core::ptr::write_bytes((p_vaddr + p_filesz) as *mut u8, 0, p_memsz - p_filesz);
            }
        }

        loaded += 1;
        crate::vga_print_line(
            &format!(
                "shardev OS: LOAD 0x{:x} -> 0x{:x} ({} bytes)",
                p_offset, p_vaddr, p_filesz
            ),
            20,
            0x0F,
        );
    }

    if loaded == 0 {
        return Err("El ELF no contiene segmentos PT_LOAD");
    }

    crate::vga_print_line("shardev OS: Saltando al programa de usuario...", 21, 0x0F);

    let user_pml4 = unsafe { crate::mm::paging::create_user_pml4() }
        .map_err(|_| "No se pudo crear el PML4 de usuario")?;
    for i in 0..e_phnum {
        let ph = e_phoff + i * e_phentsize;
        let p_type = read_u32(bytes, ph);
        if p_type != 1 {
            continue;
        }
        let p_offset = read_u64(bytes, ph + 8) as usize;
        let p_vaddr = read_u64(bytes, ph + 16) as usize;
        let p_filesz = read_u64(bytes, ph + 32) as usize;
        let p_memsz = read_u64(bytes, ph + 40) as usize;
        unsafe {
            crate::mm::paging::map_in(
                user_pml4,
                p_vaddr,
                p_vaddr,
                crate::mm::paging::PAGE_PRESENT | crate::mm::paging::PAGE_WRITABLE,
            );
            if p_memsz > p_filesz {
                crate::mm::paging::map_in(
                    user_pml4,
                    p_vaddr + p_filesz,
                    p_vaddr + p_filesz,
                    crate::mm::paging::PAGE_PRESENT | crate::mm::paging::PAGE_WRITABLE,
                );
            }
        }
    }

    const USER_STACK_VADDR: usize = 0x300000;
    const USER_STACK_SIZE: usize = 0x10000;
    let stack_pages = USER_STACK_SIZE / crate::mm::PAGE_SIZE;
    for i in 0..stack_pages {
        let frame = unsafe { crate::mm::frame_allocator::alloc_frame() }
            .map_err(|_| "No se pudo reservar stack de usuario")?;
        unsafe {
            crate::mm::paging::map_in(
                user_pml4,
                USER_STACK_VADDR + (i * crate::mm::PAGE_SIZE),
                crate::mm::frame_to_addr(frame),
                crate::mm::paging::PAGE_PRESENT | crate::mm::paging::PAGE_WRITABLE,
            );
        }
    }

    unsafe {
        crate::syscall::set_current_security_ctx(crate::security::SecurityContext::new_unauthenticated());
        crate::sched::run_user_program(entry_point, USER_STACK_VADDR + USER_STACK_SIZE, user_pml4);
    }
}
