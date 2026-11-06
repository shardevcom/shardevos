# shardevos — Sistema Operativo Inteligente de Kernel Unificado

> **shardevos** es un sistema operativo inteligente de kernel unificado escrito en Rust y Assembly x86_64. Diseñado desde cero para ejecutar binarios heterogéneos (PE/ELF/MachO) nativamente, con un pipeline de entrada unificado (táctil + visión por computadora), motor de IA multimodal local (voz + visión + acciones), y capacidades de autogestión por IA (auto-reparación y planificación adaptativa).

---

## Tabla de Contenidos

- [Características Principales](#características-principales)
- [Arquitectura del Sistema](#arquitectura-del-sistema)
- [Estructura del Proyecto](#estructura-del-proyecto)
- [Fases de Desarrollo](#fases-de-desarrollo)
- [Compilación y Ejecución](#compilación-y-ejecución)
- [Diseño de la Interfaz de Usuario (UI Responsiva)](#diseño-de-la-interfaz-de-usuario-ui-responsiva)
- [Cargador de Binarios Universal](#cargador-de-binarios-universal)
- [Módulo de IA](#módulo-de-ia)
- [Módulo de Autogestión](#módulo-de-autogestión)
- [Roadmap](#roadmap)
- [Contribuir](#contribuir)
- [Licencia](#licencia)

---

## Características Principales

| Característica | Descripción |
|--------------|-------------|
| **Kernel Monolítico Modular** | Kernel en Rust con Assembly x86_64 para bootloader e interrupciones |
| **Cargador de Binarios Heterogéneos** | Ejecuta `.exe` (PE), `.deb`/ELF (Linux), y `.app`/MachO (macOS) nativamente |
| **Gestor de Entrada Unificado** | Fusiona eventos táctiles multi-touch y gestos aéreos 3D desde cámara web |
| **Pipeline Multimodal de IA** | STT (whisper.cpp), TTS (Piper), SLM local (Llama-3-3B/Phi-3) con Function Calling |
| **Autogestión por IA** | Planificación adaptativa de CPU/RAM, auto-reparación de hilos/drivers sin Kernel Panic |
| **UI Responsiva** | Diseño adaptativo para móvil, tablet, desktop y TV con breakpoints dinámicos |
| **VFS y Sistemas de Archivos** | Virtual File System con soporte para sistemas de archivos nativos |
| **Drivers de Dispositivo** | Soporte para dispositivos de bloque y red virtuales |

---

## Arquitectura del Sistema

```
┌─────────────────────────────────────────────────────────────┐
│                    Aplicaciones de Usuario                  │
│  (PE/ELF/MachO binarios ejecutándose nativamente)           │
├─────────────────────────────────────────────────────────────┐
│                    Capa de Syscalls Unificada               │
│  (exit, write, read, open, close, input_read, input_poll)   │
├─────────────────────────────────────────────────────────────┐
│                    AI Engine Multimodal                     │
│  STT (whisper.cpp) │ TTS (Piper) │ SLM (Llama-3-3B/Phi-3)   │
│  Function Calling sobre APIs del SO                         │
├─────────────────────────────────────────────────────────────┐
│              Unified Input Manager                          │
│  Táctil Multi-Touch │ Visión por Computadora (CV)           │
│  Gestos: PINCH, SWIPE, TAP, AIR_3D                          │
├─────────────────────────────────────────────────────────────┐
│              UI Manager (Diseño Responsivo)                 │
│  Mobile │ Tablet │ Desktop │ TV — Breakpoints adaptativos   │
├─────────────────────────────────────────────────────────────┐
│              Módulo de Autogestión                          │
│  AI Scheduler (CPU/RAM predictivo) │ Self-Healing           │
├─────────────────────────────────────────────────────────────┐
│              VFS y Sistemas de Archivos                     │
│  shardevfs nativo │ Frameworks de montaje                    │
├─────────────────────────────────────────────────────────────┐
│              Drivers de Dispositivo                         │
│  Bloque (IDE/SATA/NVMe) │ Red (Ethernet/WiFi/VirtIO)        │
├─────────────────────────────────────────────────────────────┐
│              Kernel Core (Rust + Assembly x86_64)           │
│  GDT/IDT │ Paginación 4 niveles │ Heap │ Interrupciones    │
├─────────────────────────────────────────────────────────────┐
│                    Bootloader (Multiboot2)                  │
└─────────────────────────────────────────────────────────────┘
```

---

## Estructura del Proyecto

```
shardevos/
├── Cargo.toml                    # Manifiesto del crate principal
├── Cargo.lock
├── README.md                     # Este archivo
├── BACKLOG.md                    # Plan de trabajo y tareas
├── README_ARQUITECTURA.md        # Documentación técnica detallada
│
├── binfmt_loader/
│   └── mod.rs                    # Cargador universal PE/ELF/MachO (10 tests)
│
├── bootloader/
│   ├── linker.ld                 # Script de enlazado
│   └── x86_64/
│       └── boot.asm              # GDT, PAE, paginación, modo largo
│
├── build/
│   └── build.sh                  # Script de compilación y QEMU
│
├── kernel/
│   ├── main.rs
│   ├── arch/x86_64/
│   │   ├── entry.rs              # Entry point del kernel
│   │   ├── gdt.rs                # Tabla de descriptores GDT
│   │   ├── idt.rs                # Tabla de interrupciones IDT
│   │   ├── interrupts.rs         # Manejadores de interrupciones + PIC
│   │   ├── isr.asm               # 256 stubs de interrupción
│   │   ├── sched/
│   │   │   ├── mod.rs            # Planificador de tareas
│   │   │   └── ai_scheduler.rs   # AI Scheduler predictivo
│   │   └── self_healing/
│   │       └── mod.rs            # Auto-reparación sin Kernel Panic
│   ├── drivers/
│   │   ├── input/
│   │   │   ├── mod.rs            # Input Manager (táctil + CV)
│   │   │   ├── touchscreen.rs    # Driver táctil multi-touch
│   │   │   ├── camera_cv.rs      # Visión por computadora
│   │   │   └── dispatcher.rs     # Despachador unificado
│   │   ├── gpu_npu/
│   │   │   └── mod.rs            # GPU/NPU acceleration
│   │   ├── block/
│   │   │   └── mod.rs            # Dispositivo de bloque
│   │   └── net/
│   │       └── mod.rs            # Driver de red
│   ├── fs/
│   │   ├── mod.rs                # VFS (Virtual File System)
│   │   └── shardevfs.rs          # Sistema de archivos nativo
│   ├── ai_engine/
│   │   ├── mod.rs                # Motor de IA base
│   │   ├── stt.rs                # Voz a Texto (whisper.cpp)
│   │   ├── tts.rs                # Texto a Voz (Piper)
│   │   └── slm.rs                # SLM con Function Calling
│   └── ui_manager/
│       ├── mod.rs                # Servidor de display
│       ├── window.rs             # Gestor de ventanas
│       ├── display_server.rs     # Servidor de display
│       ├── compositor.rs         # Compositor de ventanas
│       └── responsive.rs         # Diseño responsivo (breakpoints)
│
├── userland/                     # Binarios de usuario (PE/ELF/MachO)
├── assets/
│   └── logo.svg                  # Logo del sistema operativo
├── docs/
└── target/                       # Directorio de compilación (generado)
```

---

## Fases de Desarrollo

| Fase | Nombre | Estado | Progreso |
|------|--------|--------|----------|
| F1 | Especificación y estructura | ✅ Completada | 100% |
| F2 | Subsistema base (arch, mm, syscall) | ✅ Completada | 100% |
| F3 | Unified Input Manager | ✅ Completada | 100% |
| F4 | AI Engine multimodal | ✅ Completada | 100% |
| F5 | UI Manager + GPU/NPU | ✅ Completada | 100% |
| F6 | Self-Healing & AI Scheduler | ✅ Completada | 100% |
| F7 | VFS, sistemas de archivos, drivers | ✅ Completada | 100% |
| F8 | Pruebas, simulación, documentación | 🔄 En progreso | 40% |

### Detalle por Fase

**Fase 1:** Árbol de carpetas, Cargo.toml, binfmt_loader con detección PE/ELF/MachO, 10 tests pasando.

**Fase 2:** GDT, IDT, PIC, manejo de interrupciones, frame allocator (bitmap), paginación 4 niveles, heap bump allocator, capa de syscalls unificada (22 syscalls).

**Fase 3:** Input Manager con detección táctil multi-touch, visión por computadora para gestos aéreos 3D, despachador unificado con priorización y deduplicación, syscalls de entrada (input_read, input_poll).

**Fase 4:** STT (whisper.cpp/Sherpa-ONNX), TTS (Piper), SLM (Llama-3-3B/Phi-3) con Function Calling para sys_read, sys_write, sys_uname.

**Fase 5:** Servidor de display, gestor de ventanas, compositor, driver GPU/NPU, **diseño responsivo** con breakpoints para móvil/tablet/desktop/TV.

**Fase 6:** Planificador round-robin, AI Scheduler predictivo con promedio móvil, auto-reparación de hilos/drivers sin Kernel Panic, captura de fallos de driver.

**Fase 7:** VFS con montaje de sistemas de archivos, shardevfs nativo con inodos y bitmap, driver de dispositivo de bloque, driver de red.

**Fase 8:** Pruebas en QEMU, documentación completa, release v0.1.0.

---

## Compilación y Ejecución

### Requisitos Previos

```bash
# Debian/Ubuntu
sudo apt-get install nasm qemu-system-x86 binutils grub-pc-bin

# Rust (instalar con rustup)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup target add x86_64-unknown-none
```

### Comandos de Construcción

```bash
# Entrar al directorio del proyecto
cd /home/elber/shardevos

# Compilar kernel y bootloader
./build/build.sh build

# Compilar y ejecutar en QEMU
./build/build.sh run

# Compilar y crear imagen ISO booteable
./build/build.sh iso

# Compilar y ejecutar todo
./build/build.sh all

# Limpiar archivos generados
./build/build.sh clean
```

### Opciones del Script

| Comando | Descripción |
|---------|-------------|
| `build` | Compila bootloader + kernel |
| `run` | Compila y ejecuta en QEMU |
| `iso` | Crea imagen ISO booteable con GRUB |
| `all` | build + run |
| `clean` | Limpia archivos generados |

### Salida Esperada

Al ejecutar en QEMU, verás la salida por puerto serie (stdio) con mensajes de inicialización del kernel:

```
[shardevos] Inicializando GDT...
[shardevos] Inicializando IDT...
[shardevos] Inicializando PIC...
[shardevos] Inicializando memoria...
[shardevos] Inicializando syscalls...
[shardevos] Inicializando Input Manager...
[shardevos] Inicializando AI Engine...
[shardevos] Inicializando UI Manager...
[shardevos] Inicializando Self-Healing...
[shardevos] Inicializando VFS...
[shardevos] Inicializando drivers de disco y red...
[shardevos] Kernel listo. Bucle principal iniciado.
```

---

## Diseño de la Interfaz de Usuario (UI Responsiva)

La UI de shardevos es **completamente responsiva** y se adapta automáticamente al dispositivo de visualización:

### Breakpoints Definidos

| Dispositivo | Ancho Mínimo | Columnas | Escala UI | Touch Targets |
|-------------|-------------|----------|-----------|---------------|
| Móvil (XS) | 0px | 1 | 0.8x | Sí |
| Móvil (SM) | 480px | 2 | 0.9x | Sí |
| Tablet (MD) | 768px | 3 | 1.0x | Sí |
| Desktop (LG) | 1024px | 4 | 1.1x | No |
| Escritorio (XL) | 1280px | 6 | 1.2x | No |
| TV (XXL) | 1920px | 8 | 1.5x | No |

### Características

- **Detección automática** de resolución y orientación (portrait/landscape)
- **Ajuste automático** de tamaño de ventanas según el dispositivo
- **Ventanas centradas** automáticamente en la pantalla
- **Barra lateral colapsable** en móviles y tablets pequeños
- **Targets táctiles grandes** en dispositivos táctiles
- **Escala de fuente adaptativa** según el dispositivo

### Ejemplo de Uso

```rust
// Crear una ventana que se adapta automáticamente al dispositivo
let props = WindowProperties {
    title: b"Mi Aplicación",
    x: 100, y: 100,
    width: 800, height: 600,
    visible: true,
    borderless: false,
    min_width_pct: 50,   // 50% del ancho de pantalla
    max_width_pct: 100,  // Hasta 100% del ancho
    min_height_pct: 50,
    max_height_pct: 100,
    responsive: true,    // Habilitar adaptación automática
};

let id = window_manager.create_window_responsive(props).unwrap();
```

---

## Cargador de Binarios Universal (binfmt_loader)

shardevos ejecuta binarios heterogéneos **sin emulación ni máquinas virtuales aisladas**:

| Formato | Extensión | Encabezado (Magic) | Mapeo de Syscalls |
|---------|-----------|-------------------|-------------------|
| Windows PE | `.exe` | `MZ` (0x4D 0x5A) | Win32/NT → API interna |
| Linux ELF | Executables | `\x7fELF` (0x7F 0x45 0x4C 0x46) | POSIX/Linux nativas |
| macOS Mach-O | `.app` | `0xFEEDFACE`, `0xFEEDFACF`, etc. | Mach/BSD |

### Ejemplo de Detección

```rust
use binfmt_loader::{detect_format, BinaryFormat};

let bytes = leer_archivo("programa.exe");
let formato = detect_format(&bytes)?;
// Resultado: BinaryFormat::PE
```

---

## Módulo de IA

### STT (Speech-to-Text)
- **Motor**: whisper.cpp / Sherpa-ONNX
- **Ejecución**: Local, GPU/NPU
- **Latencia**: Ultra-baja

### TTS (Text-to-Speech)
- **Motor**: Piper TTS
- **Ejecución**: Local

### SLM (Small Language Model)
- **Modelos**: Llama-3-3B, Phi-3
- **Function Calling**: Llamada directa a APIs del sistema operativo
- **Funciones soportadas**: `sys_read`, `sys_write`, `sys_uname`

---

## Módulo de Autogestión

### AI Scheduler Predictivo
- Monitorea uso de CPU/RAM por tarea
- Predice necesidades futuras (promedio móvil ponderado)
- Ajusta prioridades de tareas automáticamente

### Self-Healing (Auto-Reparación)
- Detecta violaciones de acceso, llamadas inválidas, fallos de drivers
- Aisla, parchea o reinicia hilos de forma transparente
- **Sin Kernel Panic** ante fallos controlados

---

## Roadmap

| Versión | Características | Fecha Estimada |
|---------|----------------|----------------|
| v0.1.0 | Kernel base, binfmt_loader, syscalls, QEMU boot | Fase 8 |
| v0.2.0 | UI responsiva completa, AI Engine completo | Fase 8-9 |
| v0.3.0 | Self-healing completo, VFS completo | Fase 9-10 |
| v0.4.0 | Drivers de red y bloque completos, AI Scheduler completo | Fase 10-11 |
| v1.0.0 | Release estable, documentación completa, ISO bootable | Fase 12 |

---

## Contribuir

1. Fork el repositorio
2. Crea una rama para tu feature (`git checkout -b feature/nombre`)
3. Commit tus cambios (`git commit -m "feat: descripción"`)
4. Push a la rama (`git push origin feature/nombre`)
5. Abre un Pull Request

### Estándares de Código

- Rust 2021 edition
- `no_std` para código de kernel
- Comentarios en español
- Manejo de errores con `Result`
- `unsafe` delimitado y documentado

---

## Licencia

Este proyecto está bajo la licencia MIT. Ver el archivo `LICENSE` para más detalles.

---

## Contacto

- **Proyecto**: shardevos
- **Descripción**: Sistema Operativo Inteligente de Kernel Unificado
- **Lenguaje**: Rust + Assembly x86_64

---

*Última actualización: Noviembre 2026*