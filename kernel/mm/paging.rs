//! Paginación de 4 niveles para x86_64.
//! Estructura: CR3 → PML4 → PDPT → PD → PT → página física de 4 KB.
//! Cada tabla tiene 512 entradas de 8 bytes (u64).

use crate::mm::{PAGE_SIZE, align_down, frame_to_addr, MemError, MemResult};
use core::arch::asm;
use core::sync::atomic::{AtomicUsize, Ordering};

/// Número de entradas por tabla de paginación.
const PT_ENTRIES: usize = 512;

/// Bits de las entradas de página (PTE/PDE/PDPTE/PML4E).
pub const PAGE_PRESENT: u64 = 1 << 0;
pub const PAGE_WRITABLE: u64 = 1 << 1;
pub const PAGE_USER: u64 = 1 << 2;
pub const PAGE_WRITE_THROUGH: u64 = 1 << 3;
pub const PAGE_CACHE_DISABLE: u64 = 1 << 4;
pub const PAGE_ACCESSED: u64 = 1 << 5;
pub const PAGE_DIRTY: u64 = 1 << 6;
pub const PAGE_HUGE: u64 = 1 << 7; // solo en PD/PDPT para páginas de 2 MB/1 GB

/// Índices de los niveles de paginación para una dirección virtual de 48 bits.
#[derive(Debug, Clone, Copy)]
pub struct PageIndices {
    pub pml4: usize,
    pub pdpt: usize,
    pub pd: usize,
    pub pt: usize,
}

impl PageIndices {
    pub fn from_vaddr(vaddr: usize) -> Self {
        Self {
            pml4: (vaddr >> 39) & 0x1FF,
            pdpt: (vaddr >> 30) & 0x1FF,
            pd:  (vaddr >> 21) & 0x1FF,
            pt:  (vaddr >> 12) & 0x1FF,
        }
    }
}

/// Una tabla de paginación (512 entradas de 8 bytes = 4 KB).
#[repr(C, align(4096))]
pub struct PageTable {
    entries: [u64; PT_ENTRIES],
}

impl PageTable {
    pub const fn new() -> Self {
        Self { entries: [0; PT_ENTRIES] }
    }

    pub fn entry(&self, index: usize) -> u64 {
        self.entries[index]
    }

    pub fn set_entry(&mut self, index: usize, value: u64) {
        self.entries[index] = value;
    }
}

/// Puntero al PML4 actual (CR3).
static mut PML4_ADDR: usize = 0;

/// Indica si la paginación está inicializada.
static PAGING_INIT: AtomicUsize = AtomicUsize::new(0);

/// Inicializa las tablas de paginación.
pub unsafe fn init() {
    if PAGING_INIT.load(Ordering::Acquire) != 0 {
        return;
    }

    // Leer CR3 actual
    let mut cr3: usize;
    asm!("mov {}, cr3", out(reg) cr3, options(nostack));
    PML4_ADDR = cr3;

    // Si CR3 es 0, asignar un nuevo PML4
    if cr3 == 0 {
        match crate::mm::frame_allocator::alloc_frame() {
            Ok(frame) => {
                let addr = frame_to_addr(frame);
                core::ptr::write_bytes(addr as *mut u8, 0, PAGE_SIZE);
                PML4_ADDR = addr;
                asm!("mov cr3, {}", in(reg) addr, options(nostack));
            }
            Err(_) => return,
        }
    }

    // Identity map para los primeros 2 MB
    identity_map_range(0, 2 * 1024 * 1024);

    PAGING_INIT.store(1, Ordering::Release);
}

/// Realiza identity mapping de un rango [start, end).
unsafe fn identity_map_range(start: usize, end: usize) {
    let mut addr = align_down(start);
    while addr < end {
        let _ = map(addr, addr, PAGE_PRESENT | PAGE_WRITABLE);
        addr += PAGE_SIZE;
    }
}

/// Mapea una dirección virtual a una dirección física.
pub unsafe fn map(vaddr: usize, paddr: usize, flags: u64) -> MemResult<()> {
    if PAGING_INIT.load(Ordering::Acquire) == 0 {
        return Err(MemError::InvalidAddress);
    }

    let vaddr = align_down(vaddr);
    let paddr = align_down(paddr);
    let indices = PageIndices::from_vaddr(vaddr);

    let pml4 = &mut *(PML4_ADDR as *mut PageTable);

    // Nivel PDPT
    let pdpt_addr = get_or_create_table(pml4, indices.pml4)?;
    let pdpt = &mut *(pdpt_addr as *mut PageTable);

    // Nivel PD
    let pd_addr = get_or_create_table(pdpt, indices.pdpt)?;
    let pd = &mut *(pd_addr as *mut PageTable);

    // Nivel PT
    let pt_addr = get_or_create_table(pd, indices.pd)?;
    let pt = &mut *(pt_addr as *mut PageTable);

    // Verificar si ya está mapeado
    if pt.entries[indices.pt] & PAGE_PRESENT != 0 {
        return Err(MemError::AlreadyMapped);
    }

    // Escribir entrada: paddr (u64) | flags
    pt.entries[indices.pt] = paddr as u64 | flags | PAGE_PRESENT;

    flush_tlb_single(vaddr);
    Ok(())
}

/// Desmapea una dirección virtual.
pub unsafe fn unmap(vaddr: usize) -> MemResult<()> {
    if PAGING_INIT.load(Ordering::Acquire) == 0 {
        return Err(MemError::InvalidAddress);
    }

    let vaddr = align_down(vaddr);
    let indices = PageIndices::from_vaddr(vaddr);

    let pml4 = &mut *(PML4_ADDR as *mut PageTable);
    let pml4e = pml4.entries[indices.pml4];
    if pml4e & PAGE_PRESENT == 0 {
        return Err(MemError::NotMapped);
    }
    let pdpt = &mut *((pml4e & !0xFFF) as *mut PageTable);

    let pdpte = pdpt.entries[indices.pdpt];
    if pdpte & PAGE_PRESENT == 0 {
        return Err(MemError::NotMapped);
    }
    let pd = &mut *((pdpte & !0xFFF) as *mut PageTable);

    let pde = pd.entries[indices.pd];
    if pde & PAGE_PRESENT == 0 {
        return Err(MemError::NotMapped);
    }
    let pt = &mut *((pde & !0xFFF) as *mut PageTable);

    let pte = pt.entries[indices.pt];
    if pte & PAGE_PRESENT == 0 {
        return Err(MemError::NotMapped);
    }

    pt.entries[indices.pt] = 0;
    flush_tlb_single(vaddr);
    Ok(())
}

/// Traduce una dirección virtual a física.
pub unsafe fn translate(vaddr: usize) -> MemResult<usize> {
    if PAGING_INIT.load(Ordering::Acquire) == 0 {
        return Err(MemError::InvalidAddress);
    }

    let vaddr = align_down(vaddr);
    let indices = PageIndices::from_vaddr(vaddr);

    let pml4 = &*(PML4_ADDR as *const PageTable);
    let pml4e = pml4.entries[indices.pml4];
    if pml4e & PAGE_PRESENT == 0 {
        return Err(MemError::NotMapped);
    }
    let pdpt = &*((pml4e & !0xFFF) as *const PageTable);

    let pdpte = pdpt.entries[indices.pdpt];
    if pdpte & PAGE_PRESENT == 0 {
        return Err(MemError::NotMapped);
    }
    let pd = &*((pdpte & !0xFFF) as *const PageTable);

    let pde = pd.entries[indices.pd];
    if pde & PAGE_PRESENT == 0 {
        return Err(MemError::NotMapped);
    }

    // Página grande (2 MB)
    if pde & PAGE_HUGE != 0 {
        let base = pde & !0x1FFFFF;
        let offset = (vaddr & 0x1FFFFF) as u64;
        return Ok((base | offset) as usize);
    }

    let pt = &*((pde & !0xFFF) as *const PageTable);
    let pte = pt.entries[indices.pt];
    if pte & PAGE_PRESENT == 0 {
        return Err(MemError::NotMapped);
    }

    let phys_base = pte & !0xFFF;
    let offset = (vaddr & 0xFFF) as u64;
    Ok((phys_base | offset) as usize)
}

/// Obtiene la dirección de una tabla de nivel inferior o crea una nueva.
unsafe fn get_or_create_table(table: &mut PageTable, index: usize) -> MemResult<usize> {
    let entry = table.entries[index];
    if entry & PAGE_PRESENT != 0 {
        return Ok((entry & !0xFFF) as usize);
    }

    match crate::mm::frame_allocator::alloc_frame() {
        Ok(frame) => {
            let addr = frame_to_addr(frame);
            core::ptr::write_bytes(addr as *mut u8, 0, PAGE_SIZE);
            table.entries[index] = addr as u64 | PAGE_PRESENT | PAGE_WRITABLE;
            Ok(addr)
        }
        Err(_) => Err(MemError::FrameExhausted),
    }
}

/// Invalida una entrada del TLB para una dirección virtual.
unsafe fn flush_tlb_single(vaddr: usize) {
    asm!("invlpg [{}]", in(reg) vaddr, options(nostack));
}

/// Devuelve la dirección del PML4 actual (CR3).
pub fn cr3() -> usize {
    unsafe { PML4_ADDR }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_indices_split() {
        let vaddr = 0xFFFF_F800_0000_1000usize;
        let idx = PageIndices::from_vaddr(vaddr);
        assert_eq!(idx.pml4, 0x1FF);
        assert_eq!(idx.pdpt, 0x1FF);
        assert_eq!(idx.pd, 0x1FF);
        assert_eq!(idx.pt, 0x100);
    }

    #[test]
    fn align_functions() {
        assert_eq!(align_down(8193), 8192);
        assert_eq!(align_down(4096), 4096);
    }

    #[test]
    fn frame_conversion() {
        assert_eq!(addr_to_frame(0x1000), 1);
        assert_eq!(frame_to_addr(1), 0x1000);
    }

    #[test]
    fn page_table_size() {
        assert_eq!(core::mem::size_of::<PageTable>(), 4096);
    }
}
