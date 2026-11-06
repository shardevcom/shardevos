# shardevos — Release v1.1.0

> **shardevos** — Sistema Operativo Inteligente de Kernel Unificado  
> Versión: 1.1.0 (Release estable con persistencia y red)  
> Fecha: Noviembre 2026  
> Arquitectura: Kernel monolítico modular en Rust (`no_std`) + Assembly x86_64  
> Repositorio: git@github.com:shardevcom/shardevos.git  

---

## 1. Resumen

shardevos es un sistema operativo escrito desde cero en Rust y Assembly x86_64, diseñado para ejecutar binarios heterogéneos (PE/ELF/MachO) nativamente, con un pipeline de entrada unificado, motor de IA multimodal local, capacidades de autogestión, UI gráfica responsiva, y un sistema de seguridad multiusuario completo.

Esta release **v1.1.0** añade persistencia de archivos en disco real, conectividad de red (VirtIO-Net + stack TCP/IP), y syscalls de gestión de identidad (setuid/setgid).

---

## 2. Características Principales

| Categoría | Característica | Descripción |
|-----------|---------------|-------------|
| **Kernel** | Monolítico modular | Kernel en Rust `no_std` + Assembly x86_64 para bootloader e interrupciones |
| **Arranque** | Multiboot2 + GRUB | Carga mediante GRUB, modo largo (64 bits), paginación 4 niveles |
| **Memoria** | Paginación con protección | PML4→PDPT→PD→PT, bit U/S para separar kernel de usuario, frame allocator bitmap |
| **Multi-tarea** | Scheduler preemptivo | Round-robin por prioridades, timer PIT a 100 Hz, cambio de contexto con `iretq`, `fork` (syscall 26) |
| **Binarios** | Cargador universal | Ejecuta `.exe` (PE), `.deb`/ELF (Linux), `.app`/MachO (macOS) nativamente sin emulación |
| **Sistema de archivos** | VFS + shardevfs | Path resolution real (`/foo/bar.txt`), open/read/write/close/stat, archivos en RAM y disco |
| **Dispositivo de bloque** | VirtIO-Blk real | Protocolo VirtIO 1.0 sobre MMIO, lectura/escritura de bloques reales en disco QEMU |
| **Syscalls de bloque** | `sys_block_read/write` | Números 27 y 28 para acceso directo a disco desde userspace |
| **Red** | VirtIO-Net real | Protocolo VirtIO 1.0 sobre MMIO, colas RX/TX, envío/recepción de paquetes Ethernet |
| **Stack TCP/IP** | ARP + ICMP | Resolución de direcciones MAC (ARP), echo request/reply (ping), encabezado IPv4 |
| **Interfaz gráfica** | UI Responsiva VESA | Modo gráfico 800x600@32 RGBA, framebuffer LFB, primitivas de dibujo, texto 8x8, barra de título |
| **Búsqueda IA** | Indexador semántico | Indexa archivos por contenido, búsqueda top-5 por similitud de coseno, embeddings placeholder |
| **Seguridad** | Multiusuario | Usuarios/grupos (/etc/passwd, /etc/group), autenticación por contraseña, permisos rwx estilo Unix |
| **Syscalls de seguridad** | `getuid`/`login`/`setuid`/`setgid` | Números 29-35 para gestión de identidad y autenticación |
| **Total de syscalls** | 35 | Desde `exit` (1) hasta `setgid` (35) |

---

## 3. Arquitectura del Sistema

```
┌─────────────────────────────────────────────────────────────┐
│                    Aplicaciones de Usuario                  │
│  (PE/ELF/MachO binarios ejecutándose nativamente)           │
├─────────────────────────────────────────────────────────────┤
│                    Capa de Syscalls Unificada               │
│  (35 syscalls: exit/write/read/open/close/stat/exec/fork/   │
│   block_read/block_write/getuid/geteuid/login/setuid/...)   │
├─────────────────────────────────────────────────────────────┤
│                    Módulo de Seguridad                      │
│  Usuarios/grupos │ Autenticación │ Permisos rwx │ Contextos │
├─────────────────────────────────────────────────────────────┤
│                    Stack TCP/IP                             │
│  ARP (resolución MAC) │ ICMP (ping) │ IPv4 (encabezado)    │
├─────────────────────────────────────────────────────────────┤
│                    Unified Input Manager                    │
│  Táctil Multi-Touch │ Visión por Computadora (CV)           │
├─────────────────────────────────────────────────────────────┤
│                    UI Manager (Diseño Responsivo)           │
│  VESA 800x600@32 │ Framebuffer LFB │ Texto 8x8 │ Ventanas  │
├─────────────────────────────────────────────────────────────┤
│              Módulo de Autogestión                          │
│  AI Scheduler (CPU/RAM predictivo) │ Self-Healing          │
├─────────────────────────────────────────────────────────────┤
│              VFS y Sistemas de Archivos                     │
│  shardevfs nativo │ Path resolution real │ Persistencia en disco │
├─────────────────────────────────────────────────────────────┤
│              Drivers de Dispositivo                         │
│  Bloque (VirtIO-Blk real) │ Red (VirtIO-Net real)          │
├─────────────────────────────────────────────────────────────┤
│              Kernel Core (Rust + Assembly x86_64)           │
│  GDT/IDT │ Paginación 4 niveles │ Heap │ Interrupciones    │
├─────────────────────────────────────────────────────────────┤
│                    Bootloader (Multiboot2)                  │
└─────────────────────────────────────────────────────────────┘
```

---

## 4. Estructura del Proyecto

```
shardevos/
├── Cargo.toml                    # Manifiesto del crate principal
├── Cargo.lock
├── README.md                     # Documentación del proyecto
├── BACKLOG.md                    # Plan de trabajo (Fases 1-16)
├── TEST_RESULTS.md               # Resultados de pruebas de todas las fases
├── RELEASE.md                    # Notas de release (este archivo)
├── LICENSE                       # Licencia MIT
│
├── binfmt_loader/
│   └── mod.rs                    # Cargador universal PE/ELF/MachO (10 tests)
│
├── bootloader/
│   ├── linker.ld                 # Script de enlazado
│   └── x86_64/
│       ├── boot.asm              # GDT, PAE, paginación, modo largo, Multiboot2
│
├── build/
│   └── build.sh                  # Script de compilación y QEMU (build/run/iso/img/all/clean)
│
├── kernel/
│   ├── main.rs                   # Punto de entrada del kernel, UI de arranque VESA
│   ├── arch/x86_64/
│   │   ├── entry.rs              # Entry point del kernel
│   │   ├── gdt.rs                # Tabla de descriptores GDT
│   │   ├── idt.rs                # Tabla de interrupciones IDT
│   │   ├── interrupts.rs         # Manejadores de interrupciones + PIC + PIT timer
│   │   ├── isr.asm               # 256 stubs de interrupción
│   │   ├── switch.asm            # Cambio de contexto, switch_to_user, timer_reschedule
│   │   ├── sched/
│   │   │   ├── mod.rs            # Planificador de tareas (round-robin, fork, context switch)
│   │   │   └── ai_scheduler.rs   # AI Scheduler predictivo
│   │   ├── self_healing/
│   │   │   └── mod.rs            # Auto-reparación sin Kernel Panic
│   │   └── pci/                  # Módulo PCI (enumeración, BARs, dispositivos VirtIO)
│   ├── drivers/
│   │   ├── block/mod.rs          # Driver de dispositivo de bloque (stub + VirtIO-Blk real)
│   │   ├── virtio_blk/mod.rs     # Driver VirtIO-Blk 1.0 sobre MMIO (disco real QEMU)
│   │   ├── virtio_net/mod.rs     # Driver VirtIO-Net 1.0 sobre MMIO (red real QEMU)
│   │   ├── net/                  # Stack de red (ARP, ICMP, IPv4)
│   │   │   └── stack.rs          # Protocolo TCP/IP mínimo
│   │   ├── gpu_npu/mod.rs        # Driver GPU/NPU acceleration
│   │   ├── input/                # Input Manager (táctil + visión por computadora)
│   │   └── net/mod.rs            # Driver de red base (Ethernet/WiFi/VirtIO)
│   ├── fs/
│   │   ├── mod.rs                # VFS + shardevfs (path resolution real, open/read/write/close/stat, persistencia en disco)
│   │   └── ...
│   ├── security/
│   │   ├── mod.rs                # Módulo de seguridad (usuarios + permisos)
│   │   ├── users.rs              # Gestión de usuarios/grupos, autenticación
│   │   └── permissions.rs        # Permisos rwx estilo Unix, check_access, can_chown/chmod
│   ├── syscall/
│   │   ├── mod.rs                # 35 syscalls unificados + dispatch table
│   │   └── syscall_entry.asm     # Stub syscall/sysret
│   ├── ui_manager/
│   │   ├── mod.rs                # Servidor de display, WindowManager, DisplayServer
│   │   ├── display.rs            # Modo gráfico VESA 800x600@32, framebuffer LFB, texto 8x8
│   │   ├── window.rs             # Gestor de ventanas
│   │   ├── display_server.rs     # Servidor de display
│   │   ├── compositor.rs         # Compositor de ventanas
│   │   └── responsive.rs         # Diseño responsivo (breakpoints)
│   ├── ai_engine/
│   │   ├── mod.rs                # Motor de IA base
│   │   ├── stt.rs                # Voz a Texto (whisper.cpp)
│   │   ├── tts.rs                # Texto a Voz (Piper)
│   │   └── slm.rs                # SLM con Function Calling
│   ├── mm/
│   │   ├── mod.rs                # Gestor de memoria (frame allocator, paginación)
│   │   ├── frame_allocator.rs    # Bitmap allocator (1M marcos = 4 GB)
│   │   ├── heap.rs               # Bump allocator 4 MB
│   │   └── paging.rs             # Paginación 4 niveles, protección U/S, create_user_pml4
│   ├── userland_bin.rs           # Binarios de usuario embargados (hello.elf, search.elf)
│   └── userland_loader.rs        # Cargador de binarios de usuario ELF64
│
├── userland/                     # Binarios de usuario (PE/ELF/MachO)
│   ├── hello.asm / hello.elf     # Programa de ejemplo
│   └── search/                   # Indexador y buscador semántico de archivos
│       ├── main.rs               # Indexador + buscador top-5 (coseno)
│       ├── syscalls.rs           # Wrappers de syscalls para userspace
│       └── linker.ld             # Script de enlazado (0x150000)
│
├── assets/
│   └── logo.svg                  # Logo del sistema operativo
│
├── docs/
├── target/                       # Directorio de compilación (generado)
└── build/out/                    # Binarios generados (kernel.elf, ShardevOS.bin, ISO)
```

---

## 5. Compilación y Ejecución

### Requisitos Previos

```bash
# Debian/Ubuntu
sudo apt-get install nasm qemu-system-x86 binutils grub-pc-bin xorriso

# Rust (instalar con rustup)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup target add x86_64-unknown-none
source $HOME/.cargo/env
```

### Comandos de Construcción

```bash
# Entrar al directorio del proyecto
cd /home/elber/shardevos

# Compilar kernel y bootloader
./build/build.sh build

# Compilar y ejecutar en QEMU
./build/build.sh run

# Compilar y crear imagen ISO booteable v1.1.0
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
| `run` | Compila y ejecuta en QEMU (salida serial) |
| `iso` | Crea imagen ISO booteable con GRUB (ShardevOS.iso) |
| `img` | Crea imagen de disquete para VirtualBox |
| `all` | build + iso + img |
| `clean` | Limpia archivos generados |

### Salida Esperada (Arranque)

Al ejecutar en QEMU, verás la salida por puerto serie y la UI gráfica VESA:

```
[shardevos] Inicializando GDT...
[shardevos] Inicializando IDT...
[shardevos] Inicializando PIC...
[shardevos] Inicializando PIT (timer 100Hz)...
[shardevos] Inicializando memoria...
[shardevos] Inicializando syscalls (35)...
[shardevos] Inicializando Input Manager...
[shardevos] Inicializando AI Engine...
[shardevos] Inicializando UI Manager (VESA 800x600@32)...
[shardevos] Inicializando Self-Healing...
[shardevos] Inicializando VFS...
[shardevos] [ OK ] shardevfs: cargado desde disco VirtIO.  (si hay disco QEMU)
[shardevos] [ OK ] Gestor de usuarios inicializado.
[shardevos] [ OK ] VirtIO-Blk dispositivo de bloque detectado.
[shardevos] [ OK ] VirtIO-Net dispositivo de red detectado.
[shardevos] [ OK ] Stack de red inicializado (ARP, ICMP).
[shardevos] Kernel listo. UI gráfica activada.
```

---

## 6. Syscalls Implementados (35 total)

| Número | Nombre | Descripción |
|--------|--------|-------------|
| 1 | `sys_exit` | Terminar proceso |
| 2 | `sys_write` | Escribir a descriptor de archivo |
| 3 | `sys_read` | Leer de descriptor de archivo |
| 4 | `sys_open` | Abrir archivo |
| 5 | `sys_close` | Cerrar archivo |
| 6 | `sys_stat` | Estadísticas de archivo |
| 7 | `sys_mmap` | Mapear memoria |
| 8 | `sys_munmap` | Desmapear memoria |
| 9 | `sys_brk` | Ajustar heap |
| 10 | `sys_clone` | Crear proceso/hilo |
| 11 | `sys_execve` | Ejecutar binario |
| 12 | `sys_wait4` | Esperar proceso hijo |
| 13 | `sys_kill` | Enviar señal |
| 14 | `sys_getpid` | Obtener PID |
| 15 | `sys_time` | Obtener tiempo |
| 16 | `sys_uname` | Información del sistema |
| 17 | `sys_ioctl` | Control de dispositivo |
| 18 | `sys_fcntl` | Control de archivo |
| 19 | `sys_poll` | Esperar eventos |
| 20 | `sys_nanosleep` | Dormir |
| 21 | `sys_input_read` | Leer evento de entrada |
| 22 | `sys_input_poll` | Consultar cola de entrada |
| 23 | `sys_vfs_list` | Listar inodos del VFS |
| 24 | `sys_vfs_read_by_inode` | Leer inodo por ID |
| 25 | `sys_exec` | Cargar y ejecutar ELF desde VFS |
| 26 | `sys_fork` | Crear proceso hijo |
| 27 | `sys_block_read` | Leer bloques desde disco VirtIO |
| 28 | `sys_block_write` | Escribir bloques al disco VirtIO |
| 29 | `sys_getuid` | Obtener UID real |
| 30 | `sys_getgid` | Obtener GID real |
| 31 | `sys_geteuid` | Obtener UID efectivo |
| 32 | `sys_getegid` | Obtener GID efectivo |
| 33 | `sys_login` | Autenticar usuario |
| 34 | `sys_setuid` | Establecer UID efectivo (solo root) |
| 35 | `sys_setgid` | Establecer GID efectivo (solo root) |

---

## 7. Instalación en QEMU y VirtualBox

### QEMU

```bash
# Arrancar desde ISO
qemu-system-x86_64 -cdrom build/ShardevOS.iso -nographic -monitor null -serial stdio -no-reboot -m 256 -vga std -machine q35

# Con disco VirtIO real (para probar persistencia de shardevfs)
qemu-system-x86_64 -cdrom build/ShardevOS.iso -drive file=fat:rw:/tmp/disk,if=virtio -nographic -monitor null -serial stdio -no-reboot -m 256 -vga std -machine q35

# Con red VirtIO (para probar stack TCP/IP)
qemu-system-x86_64 -cdrom build/ShardevOS.iso -netdev user,id=net0 -device virtio-net-pci,netdev=net0 -nographic -monitor null -serial stdio -no-reboot -m 256 -vga std -machine q35
```

### VirtualBox

1. Crear VM con controlador de DVD
2. Montar `build/ShardevOS.iso` como disco óptico
3. Configurar arranque desde disco óptico
4. Arrancar la VM

O usar imagen de disquete:
```bash
./build/build.sh img
# Montar build/ShardevOS.flp como disquete en VirtualBox
```

---

## 8. Limitaciones Conocidas

| Limitación | Descripción | Plan |
|------------|-------------|------|
| Salida serial en QEMU | El puerto serie COM1 no siempre es visible en entornos de prueba headless | Usar modo gráfico VESA (funcional) |
| AI Engine | Módulos integrados pero requieren modelos externos (whisper.cpp, Piper, Llama) para funcionar completamente | Conectar modelos quantizados en userspace |
| setuid/setgid | Implementados pero solo para root; no hay cambio de identidad de usuario no-root | Ampliar a usuarios no-root con verificación |
| syscalls de red para userspace | Stack TCP/IP en kernel pero sin syscalls para usarlo desde userspace | Agregar syscalls socket/connect/send/recv |
| Multiusuario completo | Contexto de seguridad por proceso implementado; aislamiento completo de memoria entre usuarios pendiente | Paginación con espacios de direcciones aislados por usuario |

---

## 9. Roadmap Futuro (v1.2.0+)

| Versión | Características | Fase |
|---------|---------------|------|
| v1.2.0 | Syscalls de red para userspace (socket, connect, send, recv) | F17 |
| v1.3.0 | AI Engine con modelo quantizado real (all-MiniLM-L6-v2 INT8) | F18-F19 |
| v1.4.0 | UI con ventanas, compositor, ratón, gestos táctiles | F20-F21 |
| v2.0.0 | Multiusuario completo con aislamiento de memoria, sandbox de procesos | F22-F24 |

---

## 10. Contacto y Licencia

- **Proyecto**: shardevos — Sistema Operativo Inteligente de Kernel Unificado
- **Descripción**: SO escrito en Rust (`no_std`) + Assembly x86_64 con cargador universal de binarios, UI gráfica, búsqueda semántica con IA, y seguridad multiusuario
- **Lenguaje**: Rust 2021 edition + Assembly x86_64
- **Licencia**: MIT
- **Repositorio**: git@github.com:shardevcom/shardevos.git

---

*Última actualización: Noviembre 2026 — Release v1.1.0*
