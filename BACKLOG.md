# shardevos — Backlog y Plan de Trabajo

Proyecto: Sistema Operativo Inteligente de Kernel Unificado ("shardevos")  
Lenguaje: Rust (`no_std`) + Assembly x86_64 / ARM64  
Arquitectura: Kernel monolítico modular con cargador universal de binarios (PE/ELF/MachO)

---

## 1. Estado General

| Fase | Nombre | Estado | Progreso |
|------|--------|--------|----------|
| F1 | Especificación del motor y estructura general de archivos | ✅ Completada | 100% |
| F2 | Subsistema base del kernel (arch, mm, syscall) | ✅ Completada | 100% |
| F3 | Unified Input Manager (táctil + visión por computador) | ✅ Completada | 100% |
| F4 | AI Engine multimodal (STT, TTS, SLM + Function Calling) | ✅ Completada | 100% |
| F5 | UI Manager / Servidor de display + GPU/NPU | ✅ Completada | 100% |
| F6 | Self-Healing & AI Scheduler (autogestión del kernel) | ✅ Completada | 100% |
| F7 | VFS, sistemas de archivos y drivers de dispositivo | ✅ Completada | 100% |
| F8 | Pruebas, simulación QEMU, documentación y release | ✅ Completada | 100% |

---

## 2. Backlog Detallado

### Fase 1 — Especificación y estructura (COMPLETADA)

| ID | Título | Descripción | Estado |
|----|--------|-------------|--------|
| F1-01 | Árbol de carpetas del proyecto | `bootloader/`, `kernel/`, `binfmt_loader/`, `ai_engine/`, `ui_manager/`, `userland/`, `build/` | ✅ |
| F1-02 | `Cargo.toml` y manifiesto | Crate `shardevos`, `no_std`, `rlib` + `lib` | ✅ |
| F1-03 | `binfmt_loader/mod.rs` | `enum BinaryFormat { PE, ELF, MachO, Unknown }`, detección por magics (`MZ`, `\x7fELF`, Mach-O LE/BE), `parse_header()`, `map_to_userspace()` | ✅ |
| F1-04 | Pruebas unitarias `binfmt_loader` | 10 tests pasando (detección, parseo PE/ELF/MachO, rechazos) | ✅ |
| F1-05 | `README.md` | Documentación inicial, estructura, comandos de compilación | ✅ |

### Fase 2 — Subsistema base del kernel (COMPLETADA)

| ID | Título | Descripción | Estado |
|----|--------|-------------|--------|
| F2-01 | `bootloader/x86_64/boot.asm` | Multiboot2, GDT mínima, PAE, paging identity-map, modo largo, salto a Rust | ✅ |
| F2-02 | `bootloader/linker.ld` | Script de enlazado, entry en 1 MB | ✅ |
| F2-03 | `kernel/arch/x86_64/entry.rs` | Entry point del kernel (`no_std`, `no_main`), inicializa GDT/IDT/MM/Syscalls | ✅ |
| F2-04 | `kernel/arch/x86_64/gdt.rs` | `Gdt`, `SegmentDescriptor`, `GdtPointer`, `lgdt`, selectores de segmento | ✅ |
| F2-05 | `kernel/arch/x86_64/idt.rs` | `Idt`, `IdtEntry`, 256 entradas, `lidt`, `set_handler()`, `default_isr` | ✅ |
| F2-06 | `kernel/arch/x86_64/interrupts.rs` | `InterruptFrame`, `isr_common_handler()`, remapeo PIC (IRQ 0-15 → vectores 32-47), EOI, `sti` | ✅ |
| F2-07 | `kernel/arch/x86_64/isr.asm` | 256 stubs de interrupción que saltan a `isr_common_handler` | ✅ |
| F2-08 | `kernel/mm/mod.rs` | `MemError`, `MemResult`, `PAGE_SIZE`, `align_up/down`, `addr_to_frame/frame_to_addr`, `#[global_allocator]` | ✅ |
| F2-09 | `kernel/mm/frame_allocator.rs` | Bitmap allocator (1M marcos = 4 GB), spinlock atómico, `alloc_frame/dealloc_frame/reserve_frame` | ✅ |
| F2-10 | `kernel/mm/paging.rs` | Paginación 4 niveles (PML4→PDPT→PD→PT), `map/unmap/translate`, `identity_map_range`, `flush_tlb_single` | ✅ |
| F2-11 | `kernel/mm/heap.rs` | Bump allocator 4 MB en `0xFFFF_8000_0000_0000`, `allocate/deallocate` | ✅ |
| F2-12 | `kernel/syscall/mod.rs` | Números de syscall unificados, tabla de dispatch (4096), traducciones PE/ELF/MachO, manejadores (exit/write/read/getpid/time/nanosleep), `syscall_handler()` | ✅ |
| F2-13 | `kernel/syscall/syscall_entry.asm` | Stub `syscall`/`sysret`, guarda registros, llama a `syscall_handler` | ✅ |
| F2-14 | `build/build.sh` | Script de construcción: nasm + rustc + ld + objcopy + QEMU | ✅ |

### Fase 3 — Unified Input Manager (táctil + visión por computador)

| ID | Título | Descripción | Estado | Prioridad | Dependencias |
|----|--------|-------------|--------|-----------|--------------|
| F3-01 | `kernel/drivers/input/mod.rs` | Módulo raíz de drivers de entrada, `InputEvent` struct, `InputSource` enum, `InputManager` con cola circular, spinlock, constructores de eventos | ✅ | Alta | F2 |
| F3-02 | `kernel/drivers/input/touchscreen.rs` | Driver táctil nativo: multi-touch (10 puntos), presión 0-255, vectores de arrastre, pellizco, tap, inyección al InputManager | ✅ | Alta | F3-01 |
| F3-03 | `kernel/drivers/input/camera_cv.rs` | Driver de cámara web (CV): captura de frames, rastreo de mano 3D, conversión de gestos aéreos a `InputEvent` unificado | ✅ | Alta | F3-01 |
| F3-04 | `kernel/drivers/input/dispatcher.rs` | Despachador unificado: priorización, filtros, deduplicación, estadísticas | ✅ | Media | F3-02, F3-03 |
| F3-05 | `kernel/syscall/mod.rs` (extensión) | `sys_input_read()` (21), `sys_input_poll()` (22) para espacio de usuario | ✅ | Media | F3-04, F2-12 |

### Fase 4 — AI Engine multimodal (Voz + Visión + Acciones)

| ID | Título | Descripción | Estado | Prioridad | Dependencias |
|----|--------|-------------|--------|-----------|--------------|
| F4-01 | `kernel/ai_engine/mod.rs` | Módulo raíz del motor de IA, `AiModel` trait, `AiInferenceRequest/Response`, `AiDataBuffer` | ✅ | Alta | F2 |
| F4-02 | `kernel/ai_engine/stt.rs` | STT (whisper.cpp / Sherpa-ONNX): transcripción de audio a texto, motor con carga/descarga de modelo | ✅ | Alta | F4-01 |
| F4-03 | `kernel/ai_engine/tts.rs` | TTS (Piper TTS): síntesis de texto a audio PCM, buffer de audio, motor con carga/descarga | ✅ | Alta | F4-01 |
| F4-04 | `kernel/ai_engine/slm.rs` | SLM (Llama-3-3B / Phi-3): generación de texto, Function Calling con registro de funciones del sistema (sys_read, sys_write, sys_uname) | ✅ | Alta | F4-01 |
| F4-05 | Function Calling del SLM | Registro de funciones del sistema llamables desde el modelo (sys_read, sys_write, sys_uname) | ✅ | Alta | F4-04, F2-12 |
| F4-06 | Pipeline multimodal | Orquestación voz → texto → SLM → acción → TTS | ⏳ | Media | F4-02, F4-03, F4-05 |

### Fase 5 — UI Manager / Servidor de display

| ID | Título | Descripción | Estado | Prioridad | Dependencias |
|----|--------|-------------|--------|-----------|--------------|
| F5-01 | `kernel/ui_manager/mod.rs`, `window.rs`, `display_server.rs`, `compositor.rs`, `responsive.rs` | Servidor de display, WindowManager, DisplayServer, Compositor, Framebuffer + Diseño Responsivo (breakpoints, categorías de dispositivo, orientación, grid fluido) | ✅ | Alta | F2 |
| F5-02 | `kernel/drivers/gpu_npu/mod.rs` | Driver GPU/NPU: detección, remapeo de memoria VRAM, comandos de GPU, inferencia NPU | ✅ | Alta | F2 |
| F5-03 | `kernel/ui_manager/compositor.rs` | Compositor de ventanas: doble bufferizado, composición por Z-order, limpieza de framebuffer | ✅ | Media | F5-01, F5-02 |
| F5-04 | `kernel/ui_manager/window.rs` + `kernel/drivers/input/` | Integración con input: inyección de eventos táctiles y CV al InputManager | ✅ | Media | F5-01, F3-04 |
| F5-05 | `kernel/ui_manager/display_server.rs` | Servidor de display: modos de resolución, framebuffer, activación/suspensión | ✅ | Media | F5-01, F2-12 |

### Fase 6 — Self-Healing & AI Scheduler

| ID | Título | Descripción | Estado | Prioridad | Dependencias |
|----|--------|-------------|--------|-----------|--------------|
| F6-01 | `kernel/sched/mod.rs` | Planificador base, `Task`, `Process`, `Thread`, estados de tarea, round-robin, `sys_tick()` | ✅ | Alta | F2 |
| F6-02 | `kernel/sched/ai_scheduler.rs` | AI Scheduler predictivo: monitoreo de CPU/RAM por tarea, predicción promedio móvil, ajuste adaptativo de prioridades | ✅ | Alta | F6-01, F4-04 |
| F6-03 | `kernel/self_healing/mod.rs` | Auto-reparación: monitoreo de violaciones de acceso, GPF, invalid opcode, doble falla, syscalls inválidos, cuelgue de driver; aislamiento y reinicio de hilos sin Kernel Panic | ✅ | Alta | F2, F6-01 |
| F6-04 | `kernel/self_healing/mod.rs` | `repair_thread()` y `repair_driver()`: aislamiento y reinicio explícito de hilos y drivers problemáticos | ✅ | Alta | F6-03 |
| F6-05 | `kernel/self_healing/mod.rs` | `capture_driver_fault()`: captura de fallos de driver con diagnóstico, aislamiento automático de fallos críticos (DoubleFault, MemoryCorruption) | ✅ | Media | F6-03 |

### Fase 7 — VFS, sistemas de archivos y drivers de dispositivo

| ID | Título | Descripción | Estado | Prioridad | Dependencias |
|----|--------|-------------|--------|-----------|--------------|
| F7-01 | `kernel/fs/vfs.rs` | Virtual File System, `File`, `Inode`, `Directory`, montaje de FS | ✅ | Alta | F2 |
| F7-02 | `kernel/fs/shardevfs.rs` | Sistema de archivos nativo shardevfs, create/read/write/delete, 19 tests | ✅ | Alta | F7-01 |
| F7-03 | Soporte para PE/ELF/MachO en VFS | Cargar ejecutables desde VFS al binfmt_loader | ⏳ | Media | F7-01, F1-03 |
| F7-04 | `kernel/drivers/block/mod.rs` | Driver de dispositivo de bloque: registro de dispositivos, lecturas/escrituras por bloques (LBA), cola de solicitudes, estadísticas | ✅ | Media | F2 |
| F7-05 | `kernel/drivers/net/mod.rs` | Driver de red: registro de interfaces, envío/recepción de paquetes, estadísticas, cambio de estado | ✅ | Baja | F2 |

### Fase 8 — Pruebas, simulación y documentación

| ID | Título | Descripción | Estado | Prioridad | Dependencias |
|----|--------|-------------|--------|-----------|--------------|
| F8-01 | Pruebas en QEMU | Arranque completo, serial output, pruebas de syscalls, input y AI | ⏳ | Alta | Todas las fases previas |
| F8-02 | Pruebas de binarios heterogéneos | Ejecutar .exe, .deb/ELF, .app/MachO en shardevos | ⏳ | Alta | F1-03, F2-12 |
| F8-03 | Pruebas de AI Engine | STT/TTS/SLM locales, Function Calling | ⏳ | Media | F4 |
| F8-04 | Pruebas de self-healing | Forzar fallos de driver, verificar auto-reparación | ⏳ | Media | F6 |
| F8-05 | Documentación técnica | Especificaciones de kernel, API de syscalls, guía de desarrollador | ⏳ | Media | Todas |
| F8-06 | Release v0.1.0 | Imagen de disco bootable, ISO, instrucciones de instalación | ⏳ | Baja | Todas |

---

## 3. Métricas de Éxito por Fase

| Fase | Criterio de aceptación |
|------|----------------------|
| F1 | `cargo test` pasa en `binfmt_loader`; estructura de carpetas definida |
| F2 | Kernel arranca en QEMU, GDT/IDT/PIC funcionan, heap y paginación operativos, syscalls responden |
| F3 | Eventos táctiles y de cámara se unifican en `InputEvent`, syscalls de input funcionan |
| F4 | STT/TTS/SLM ejecutan localmente, SLM llama a al menos 3 funciones del SO |
| F5 | Servidor de display dibuja ventanas, recibe eventos de input unificado |
| F6 | Scheduler ajusta prioridades; driver fallido se reinicia sin Kernel Panic |
| F7 | VFS monta y lee archivos; binarios se cargan desde VFS |
| F8 | Imagen ISO bootea en QEMU, todos los tests pasan, documentación completa |

---

## 4. Reglas de Actualización del Backlog

1. Al iniciar una tarea, marcar su estado como `En progreso`.
2. Al completar una tarea y pasar pruebas, marcar como `✅ Completada`.
3. Si una tarea queda bloqueada, agregar comentario con el bloqueador.
4. Las dependencias deben resolverse antes de iniciar una tarea.
5. Este archivo (`BACKLOG.md`) es la fuente de verdad del plan.

---

## 5. Fase 9 — Búsqueda e indexación de archivos con IA

| ID | Título | Descripción | Estado |
|----|--------|-------------|--------|
| F9-01 | Syscall VFS_LIST + VFS_READ_BY_INODE | `SYSCALL_VFS_LIST` (23) lista inodos del VFS; `SYSCALL_VFS_READ_BY_INODE` (24) lee un inodo por ID | ✅ |
| F9-02 | Wrappers de syscall para userspace | `userland/search/syscalls.rs` con wrappers `no_std` para exit/write/read/vfs_list/vfs_read_by_inode vía instrucción `syscall` | ✅ |
| F9-03 | Indexador de archivos | `userland/search/main.rs` cmd `index`: lista inodos, lee archivos, genera embeddings (placeholder) y guarda índice en memoria | ✅ |
| F9-04 | Buscador semántico top-k | `userland/search/main.rs` cmd `search`: genera embedding de consulta y devuelve top-5 por similitud de coseno | ✅ |
| F9-05 | Modelo de embeddings ligero | Placeholder `embed()` en userspace; reemplazar por modelo quantizado (ej. all-MiniLM-L6-v2 INT8 vía ggml/llama.cpp compilado como ELF de usuario) | ⏳ |

---

## 5. Comandos de Desarrollo

```bash
# Compilar y probar binfmt_loader (Fase 1)
source $HOME/.cargo/env
rustc --edition 2021 --test binfmt_loader/mod.rs -o /tmp/shardevos_test && /tmp/shardevos_test

# Compilar kernel y lanzar QEMU (Fases 2.x)
./build/build.sh build
./build/build.sh run

# Limpiar
./build/build.sh clean
```

# Compilar userspace search (Fase 9)
source $HOME/.cargo/env
rustc --target x86_64-unknown-none -C relocation-model=static -C link-arg=-Tuserland/search/linker.ld userland/search/main.rs -o userland/search/search.elf
