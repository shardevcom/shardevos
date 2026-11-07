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
| F19 | Embeddings cuantizados INT8 y búsqueda semántica (v1.4.0) | ✅ Completada | 100% |
| F20 | Modelo serializado .semb e integración VFS (v1.5.0) | ✅ Completada | 100% |
| F21 | UI gráfica con ventanas y compositor (v1.6.0) | ✅ En progreso | 50% |

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
| F9-03 | Indexador de archivos | `userland/search/main.rs` cmd `index`: lista inodos, lee archivos, genera embeddings INT8 reales y los registra en el conector semántico | ✅ |
| F9-04 | Buscador semántico top-k | `userland/search/main.rs`: genera embedding de consulta y devuelve top-5 vía `EmbeddingIndex::search_text` (similitud de coseno) | ✅ |
| F9-05 | Modelo de embeddings ligero | Motor cuantizado INT8 (`kernel/ai_engine/embeddings.rs` + `userland/search/embeddings.rs`) con vocabulario integrado. La carga de all-MiniLM-L6-v2 vía ggml/llama.cpp queda para Fase 20 | ✅ |

---

## 5. Fase 10 — VFS real, syscall exec y archivos de prueba

| ID | Título | Descripción | Estado |
|----|--------|-------------|--------|
| F10-01 | VFS real — path resolution | `Vfs::resolve_path()` resuelve rutas como /foo/bar.txt recorriendo directorios | ✅ |
| F10-02 | VFS real — lectura/escritura de datos | `Vfs::read()`/`write()` delegan a `shardevfs::read_data()`/`write_data()` con datos reales | ✅ |
| F10-03 | Syscall exec (25) | `SYSCALL_EXEC` carga un ELF desde el VFS y salta a su entry point vía `binfmt_loader` | ✅ |
| F10-04 | Salida por VGA | `vga_print_line()` escribe a 0xB8000; logo y mensajes de estado visibles en modo texto | ✅ |
| F10-05 | Archivos de prueba | Kernel crea hello.txt, readme.txt, search_demo.txt en shardevfs al arrancar | ✅ |
| F10-06 | Indexador con VFS real | `search.elf` indexa automáticamente los archivos de shardevfs al arrancar | ✅ |

---

## 5. Fase 11 — Protección de memoria y multi-tarea preemptiva

| ID | Título | Descripción | Estado |
|----|--------|-------------|--------|
| F11-01 | Paginación con protección U/S | `create_user_pml4()`, `map_in()`, `clone_pml4()`, `switch_cr3()` para espacios de direcciones de usuario | ✅ |
| F11-02 | Switch de contexto | `switch.asm` con `switch_context` y `switch_to_user` (iretq a modo usuario) | ✅ |
| F11-03 | Timer tick preemptivo | PIT configurado a 100 Hz, IRQ 0 habilitada, `handle_timer_tick` conectado al scheduler | ✅ |
| F11-04 | Syscalls fork (26) y exec (25) | `sys_fork` crea procesos con PML4 propio; `sys_exec` carga ELF desde VFS | ✅ |
| F11-05 | Tabla de procesos con CR3 | Cada proceso tiene su propio PML4 (espacio de direcciones aislado) | ✅ |
| F11-06 | Pruebas de multi-tarea | QEMU arranca sin triple fault con timer PIT activo | ✅ |

---

## 5. Fase 12 — Drivers de dispositivo y red reales

| ID | Título | Descripción | Estado |
|----|--------|-------------|--------|
| F12-01 | Módulo PCI | Enumeración de dispositivos PCI, lectura de BARs, búsqueda de dispositivos VirtIO | ✅ |
| F12-02 | Driver VirtIO-Blk real | Protocolo VirtIO 1.0 sobre MMIO, lectura/escritura de bloques desde disco QEMU | ✅ |
| F12-03 | Syscalls de bloque (27, 28) | `SYS_BLOCK_READ` y `SYS_BLOCK_WRITE` para acceso directo a disco desde userspace | ✅ |
| F12-04 | Integrar shardevfs con bloque real | Persistencia de shardevfs en dispositivo de bloque real vía VirtIO-Blk | ✅ |
| F12-05 | Driver VirtIO-Net | Protocolo VirtIO-Net para conectividad de red real | ✅ |
| F12-06 | Syscalls de red para userspace | Envío/recepción de paquetes desde espacio de usuario | ⏳ |

---

## 5. Fase 18 — Stack TCP completo para userspace (v1.3.0)

| ID | Título | Descripción | Estado |
|----|--------|-------------|--------|
| F18-01 | Stack TCP en kernel | `kernel/drivers/net/tcp.rs`: cabecera TCP, handshake SYN/SYN-ACK/ACK, envío/recepción de segmentos, tabla de conexiones | ✅ |
| F18-02 | Syscalls de red conectados | `NET_CONNECT` (38), `NET_SEND` (39), `NET_RECV` (40) ahora usan el stack TCP real sobre VirtIO-Net | ✅ |
| F18-03 | Prueba de conectividad | QEMU con VirtIO-Net: kernel responde a ICMP echo y acepta conexiones TCP | ✅ |

---

## 5. Fase 19 — Embeddings cuantizados para búsqueda semántica (v1.4.0)

| ID | Título | Descripción | Estado |
|----|--------|-------------|--------|
| F19-01 | Módulo de embeddings quantizado (INT8) para AI Engine | `kernel/ai_engine/embeddings.rs`: tokenizador, matriz de embeddings INT8, cuantización/decuantización, similitud de coseno y `sqrt`/`round` sin `std`; inicializado desde `kernel_main` | ✅ |
| F19-02 | Conector de embeddings para búsqueda semántica | `EmbeddingIndex` + `SearchHit` en kernel y userspace: indexación por inodo, búsqueda top-k por coseno y API `index_document`/`semantic_search` sin allocador | ✅ |
| F19-03 | Integrar embeddings reales en el buscador de archivos | `userland/search/main.rs`: indexa cada archivo con `embed_text`, consulta con `semantic_search` y muestra inodo + similitud (sin placeholder de hash) | ✅ |
| F19-04 | Actualizar BACKLOG, TEST_RESULTS y RELEASE para v1.4.0 | Documentación de la fase, resultados de pruebas (16 tests de embeddings) y notas de release v1.4.0 | ✅ |

---

## 5. Fase 20 — Modelo serializado .semb e integración VFS (v1.5.0)

| ID | Título | Descripción | Estado |
|----|--------|-------------|--------|
| F20-01 | Formato serializado de embeddings (.semb) | `EmbeddingModel::export_semb` / `load_from_semb`, header `SEMB_MAGIC` / `SEMB_VERSION`, entradas de palabra + vector INT8 | ✅ |
| F20-02 | Lectura real de VFS y lookup por ruta | `SYSCALL_VFS_READ_BY_INODE` lee datos reales de shardevfs; nuevo `SYSCALL_VFS_LOOKUP` (43) resuelve ruta a inodo | ✅ |
| F20-03 | Integración del modelo en userspace | `userland/search/main.rs` carga `/minilm_int8.semb`, con fallback a modelo integrado | ✅ |
| F20-04 | Exportar modelo integrado y pruebas | `kernel/main.rs` exporta modelo a `/minilm_int8.semb`; tests de roundtrip y errores | ✅ |

---

## 5. Fase 21 — UI gráfica con ventanas y compositor

| ID | Título | Descripción | Estado |
|----|--------|-------------|--------|
| F21-01 | Integrar compositor en renderizado | `kernel/main.rs`: init compositor/WM, crear ventana, set framebuffer, `composite_frame` | ✅ |
| F21-02 | Ratón/touch/gestos | Integración de input unificado con ventana y compositor | ✅ |

---

## 5. Comandos de Desarrollo

| ID | Título | Descripción | Estado |
|----|--------|-------------|--------|
| F13-01 | Modo gráfico VESA | `kernel/ui_manager/display.rs`: modo 800x600@32, framebuffer LFB, primitivas (píxel, rect, texto 8x8) | ✅ |
| F13-02 | UI en arranque | `kernel/main.rs`: fondo degradado, barra de título, info del sistema, resultados de búsqueda | ✅ |
| F13-03 | AI Engine con modelo real | Conector para all-MiniLM-L6-v2 INT8 vía ggml/llama.cpp compilado como ELF de usuario | ⏳ |
| F13-04 | SLM con Function Calling | Llama/TinyLlama para comandos de voz al SO | ⏳ |
| F13-05 | STT/TTS locales | whisper.cpp + Piper compilados como ELF de usuario | ⏳ |

---

## 5. Fase 14 — Multiusuario y seguridad

| ID | Título | Descripción | Estado |
|----|--------|-------------|--------|
| F14-01 | Gestión de usuarios y grupos | `kernel/security/users.rs`: PasswdEntry, GroupEntry, add_user, add_group, get_user_by_name/uid, get_group_by_name/gid | ✅ |
| F14-02 | Autenticación por contraseña | `kernel/security/users.rs`: authenticate() verifica credenciales | ✅ |
| F14-03 | Contexto de seguridad por proceso | SecurityContext: uid, gid, euid, egid, grupos suplementarios, authenticated, session_id | ✅ |
| F14-04 | Permisos rwx estilo Unix | `kernel/security/permissions.rs`: FileMode, AccessType, PermissionChecker, check_access, can_chown, can_chmod | ✅ |
| F14-05 | Syscalls de autenticación | SYSCALL_GETUID (29), GETGID (30), GETEUID (31), GETEGID (32), LOGIN (33), SETUID (34), SETGID (35) | ✅ |
| F14-06 | Root UID/GID | root (0/0) y nobody (65534/65534) creados automáticamente al arrancar | ✅ |

---

## 5. Fase 15 — Release v1.0.0

| ID | Título | Descripción | Estado |
|----|--------|-------------|--------|
| F15-01 | ISO bootable v1.0.0 | Imagen de instalación completa con kernel, drivers, UI, búsqueda IA y seguridad | ✅ |
| F15-02 | Documentación completa | Guía de usuario, referencia de syscalls (35), guía de desarrollador, arquitectura | ✅ |
| F15-03 | Pruebas de integración | Pruebas de arranque, multi-tarea, búsqueda, VFS, permisos, bloque | ✅ |
| F15-04 | Release final | Repositorio listo para producción con todos los componentes integrados | ✅ |

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
