/// Punto de entrada del kernel (llamado desde el bootloader Assembly).
/// Salta al kernel Rust principal (`kernel_main` en crate root).
#[no_mangle]
pub extern "C" fn kernel_entry() -> ! {
    // Llamar al kernel principal en Rust (definido en kernel/main.rs)
    crate::kernel_main();
}
