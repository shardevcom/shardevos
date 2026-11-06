//! VFS — Sistema de Archivos Virtual del kernel shardevos.
//!
//! Proporciona:
//! - Abstracción de archivos, directorios e inodos
//! - Montaje de múltiples sistemas de archivos
//! - Operaciones de lectura/escritura/búsqueda unificadas
//! - API para sistemas de archivos subyacentes (shardevfs, etc.)


extern crate alloc;

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

mod shardevfs;

/// Identificador único de archivo.
pub type FileId = u64;

/// Identificador único de inodo.
pub type InodeId = u64;

/// Identificador único de montaje.
pub type MountId = u64;

/// Tipos de nodos del VFS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum VfsNodeType {
    RegularFile = 0,
    Directory = 1,
    Symlink = 2,
    CharacterDevice = 3,
    BlockDevice = 4,
    Socket = 5,
    Fifo = 6,
}

/// Estadísticas de un archivo/directorio.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct VfsStat {
    pub inode_id: InodeId,
    pub node_type: VfsNodeType,
    pub size: u64,
    pub blocks: u64,
    pub atime_us: u64,
    pub mtime_us: u64,
    pub ctime_us: u64,
    pub uid: u32,
    pub gid: u32,
    pub mode: u32,
}

/// Inodo del VFS (metadatos de archivo/directorio).
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct VfsInode {
    pub id: InodeId,
    pub node_type: VfsNodeType,
    pub size: u64,
    pub data_blocks: [u64; 16],  // Bloques de datos directos
    pub atime_us: u64,
    pub mtime_us: u64,
    pub ctime_us: u64,
    pub uid: u32,
    pub gid: u32,
    pub mode: u32,
    pub ref_count: u32,
    pub mount_id: MountId,
}

impl VfsInode {
    pub const fn new(id: InodeId, node_type: VfsNodeType, mount_id: MountId) -> Self {
        Self {
            id,
            node_type,
            size: 0,
            data_blocks: [0; 16],
            atime_us: 0,
            mtime_us: 0,
            ctime_us: 0,
            uid: 0,
            gid: 0,
            mode: 0o644,
            ref_count: 1,
            mount_id,
        }
    }
}

/// Archivo abierto en el VFS.
#[derive(Debug, Clone, Copy)]
pub struct VfsFile {
    pub id: FileId,
    pub inode_id: InodeId,
    pub offset: u64,
    pub flags: u32,       // O_RDONLY, O_WRONLY, O_RDWR, O_CREAT, etc.
    pub mount_id: MountId,
}

const O_RDONLY: u32 = 0x0000;
const O_WRONLY: u32 = 0x0001;
const O_RDWR: u32 = 0x0002;
#[allow(dead_code)]
const O_CREAT: u32 = 0x0040;
#[allow(dead_code)]
const O_TRUNC: u32 = 0x0200;
#[allow(dead_code)]
const O_APPEND: u32 = 0x0400;

/// Directorio del VFS (entrada de entrada de directorio).
#[derive(Debug, Clone)]
#[repr(C)]
pub struct VfsDirEntry {
    pub name: [u8; 64],
    pub inode_id: InodeId,
    pub node_type: VfsNodeType,
}

/// Resultado de operaciones del VFS.
pub type VfsResult<T> = Result<T, VfsError>;

/// Errores del VFS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VfsError {
    NotFound,
    NotADirectory,
    IsADirectory,
    InvalidArgument,
    OutOfMemory,
    PermissionDenied,
    ReadOnly,
    NoSpace,
    NotInitialized,
    AlreadyExists,
    NotMounted,
    IOError,
    DeviceNotFound,
    InvalidLba,
    InvalidBuffer,
    DeviceError,
    InvalidPacket,
}

/// Traits para sistemas de archivos subyacentes.
pub trait Filesystem: Send + Sync {
    fn name(&self) -> &str;
    fn mount(&mut self, mount_point: &str) -> VfsResult<MountId>;
    fn unmount(&mut self, mount_id: MountId) -> VfsResult<()>;
    fn create_inode(&mut self, parent_id: InodeId, name: &str, node_type: VfsNodeType) -> VfsResult<InodeId>;
    fn lookup_inode(&self, parent_id: InodeId, name: &str) -> VfsResult<InodeId>;
    fn read_inode(&self, inode_id: InodeId) -> VfsResult<VfsInode>;
    fn write_inode(&mut self, inode: &VfsInode) -> VfsResult<()>;
    fn read_data(&self, inode_id: InodeId, offset: u64, buf: &mut [u8]) -> VfsResult<usize>;
    fn write_data(&mut self, inode_id: InodeId, offset: u64, buf: &[u8]) -> VfsResult<usize>;
    fn list_directory(&self, inode_id: InodeId) -> VfsResult<Vec<VfsDirEntry>>;
    fn delete_inode(&mut self, parent_id: InodeId, name: &str) -> VfsResult<()>;
    fn get_stat(&self, inode_id: InodeId) -> VfsResult<VfsStat>;
}

// SpinLock simple para VFS
mod spin_lock_vfs {
    use core::sync::atomic::{AtomicUsize, Ordering};
    use core::cell::UnsafeCell;

    pub struct SpinLock<T> {
        lock: AtomicUsize,
        value: UnsafeCell<T>,
    }

    unsafe impl<T> Send for SpinLock<T> {}
    unsafe impl<T> Sync for SpinLock<T> {}

    impl<T> SpinLock<T> {
        pub const fn new(value: T) -> Self {
            Self { lock: AtomicUsize::new(0), value: UnsafeCell::new(value) }
        }

        pub fn lock(&self) -> SpinLockGuard<'_, T> {
            while self.lock.compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed).is_err() {
                core::hint::spin_loop();
            }
            SpinLockGuard { lock: self }
        }

        pub fn unlock(&self) {
            self.lock.store(0, Ordering::Release);
        }
    }

    pub struct SpinLockGuard<'a, T> {
        lock: &'a SpinLock<T>,
    }

    impl<'a, T> Drop for SpinLockGuard<'a, T> {
        fn drop(&mut self) {
            self.lock.unlock();
        }
    }

    impl<'a, T> core::ops::Deref for SpinLockGuard<'a, T> {
        type Target = T;
        fn deref(&self) -> &T {
            unsafe { &*self.lock.value.get() }
        }
    }

    impl<'a, T> core::ops::DerefMut for SpinLockGuard<'a, T> {
        fn deref_mut(&mut self) -> &mut T {
            unsafe { &mut *self.lock.value.get() }
        }
    }
}

/// Tabla de archivos abiertos.
#[derive(Debug, Clone)]
pub struct OpenFileTable {
    files: [Option<VfsFile>; 256],
    count: usize,
}

impl OpenFileTable {
    pub const fn new() -> Self {
        Self { files: [None; 256], count: 0 }
    }

    pub fn insert(&mut self, file: VfsFile) -> VfsResult<FileId> {
        if self.count >= 256 {
            return Err(VfsError::OutOfMemory);
        }
        let id = self.next_id();
        for slot in self.files.iter_mut() {
            if slot.is_none() {
                *slot = Some(VfsFile { id, ..file });
                self.count += 1;
                return Ok(id);
            }
        }
        Err(VfsError::OutOfMemory)
    }

    pub fn get(&self, id: FileId) -> Option<&VfsFile> {
        self.files.iter().find_map(|f| f.as_ref().filter(|f| f.id == id))
    }

    pub fn get_mut(&mut self, id: FileId) -> Option<&mut VfsFile> {
        self.files.iter_mut().find_map(|f| f.as_mut().filter(|f| f.id == id))
    }

    pub fn remove(&mut self, id: FileId) -> VfsResult<()> {
        let idx = self.files.iter().position(|f| f.as_ref().map_or(false, |f| f.id == id));
        match idx {
            Some(i) => {
                self.files[i] = None;
                self.count -= 1;
                Ok(())
            }
            None => Err(VfsError::NotFound),
        }
    }

    fn next_id(&self) -> FileId {
        self.files.iter().filter_map(|f| f.as_ref().map(|f| f.id)).max().unwrap_or(0) + 1
    }

    pub fn count(&self) -> usize {
        self.count
    }
}

/// Tabla de montajes.
#[derive(Debug, Clone)]
pub struct MountTable {
    mounts: [Option<MountEntry>; 64],
    count: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct MountEntry {
    pub id: MountId,
    pub mount_point: [u8; 64],
    pub fs_name: [u8; 32],
    pub root_inode: InodeId,
}

impl MountTable {
    pub const fn new() -> Self {
        Self { mounts: [None; 64], count: 0 }
    }

    pub fn insert(&mut self, entry: MountEntry) -> VfsResult<MountId> {
        if self.count >= 64 {
            return Err(VfsError::OutOfMemory);
        }
        for slot in self.mounts.iter_mut() {
            if slot.is_none() {
                *slot = Some(entry);
                self.count += 1;
                return Ok(entry.id);
            }
        }
        Err(VfsError::OutOfMemory)
    }

    pub fn find_by_path(&self, path: &str) -> Option<&MountEntry> {
        self.mounts.iter().find_map(|m| m.as_ref().filter(|m| {
            let mp = core::str::from_utf8(&m.mount_point).unwrap_or("");
            path.starts_with(mp)
        }))
    }

    pub fn count(&self) -> usize {
        self.count
    }
}

/// Tabla de inodos en memoria.
#[derive(Debug, Clone)]
pub struct InodeTable {
    inodes: [Option<VfsInode>; 1024],
    count: usize,
}

impl InodeTable {
    pub const fn new() -> Self {
        Self { inodes: [None; 1024], count: 0 }
    }

    pub fn insert(&mut self, inode: VfsInode) -> VfsResult<InodeId> {
        if self.count >= 1024 {
            return Err(VfsError::OutOfMemory);
        }
        for slot in self.inodes.iter_mut() {
            if slot.is_none() {
                *slot = Some(inode);
                self.count += 1;
                return Ok(self.inodes.iter().position(|i| i.is_some()).unwrap() as InodeId);
            }
        }
        Err(VfsError::OutOfMemory)
    }

    pub fn get(&self, id: InodeId) -> Option<&VfsInode> {
        self.inodes.get(id as usize).and_then(|i| i.as_ref())
    }

    pub fn get_mut(&mut self, id: InodeId) -> Option<&mut VfsInode> {
        self.inodes.get_mut(id as usize).and_then(|i| i.as_mut())
    }

    pub fn count(&self) -> usize {
        self.count
    }
}

/// Sistema de archivos virtual del kernel.
#[allow(dead_code)]
pub struct Vfs {
    open_files: spin_lock_vfs::SpinLock<OpenFileTable>,
    mounts: spin_lock_vfs::SpinLock<MountTable>,
    inodes: spin_lock_vfs::SpinLock<InodeTable>,
    next_file_id: AtomicU64,
    initialized: AtomicU64,
}

impl Vfs {
    pub const fn new() -> Self {
        Self {
            open_files: spin_lock_vfs::SpinLock::new(OpenFileTable::new()),
            mounts: spin_lock_vfs::SpinLock::new(MountTable::new()),
            inodes: spin_lock_vfs::SpinLock::new(InodeTable::new()),
            next_file_id: AtomicU64::new(1),
            initialized: AtomicU64::new(0),
        }
    }

    pub unsafe fn init(&self) {
        self.initialized.store(1, Ordering::Release);
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::Acquire) != 0
    }

    /// Monta un sistema de archivos en un punto de montaje.
    pub fn mount(&self, fs_name: &str, mount_point: &str, root_inode: InodeId) -> VfsResult<MountId> {
        if !self.is_initialized() {
            return Err(VfsError::NotInitialized);
        }

        let mut mp = [0u8; 64];
        let bytes = mount_point.as_bytes();
        let copy_len = bytes.len().min(64);
        mp[..copy_len].copy_from_slice(&bytes[..copy_len]);

        let mut fs = [0u8; 32];
        let fbytes = fs_name.as_bytes();
        let fcopy = fbytes.len().min(32);
        fs[..fcopy].copy_from_slice(&fbytes[..fcopy]);

        let mut mounts = self.mounts.lock();
        let id = mounts.count() as MountId + 1;
        mounts.insert(MountEntry {
            id,
            mount_point: mp,
            fs_name: fs,
            root_inode,
        })?;

        Ok(id)
    }

    /// Desmonta un sistema de archivos.
    pub fn unmount(&self, mount_id: MountId) -> VfsResult<()> {
        if !self.is_initialized() {
            return Err(VfsError::NotInitialized);
        }

        let mut mounts = self.mounts.lock();
        // Buscar y remover
        let idx = mounts.mounts.iter().position(|m| m.as_ref().map_or(false, |m| m.id == mount_id));
        match idx {
            Some(i) => {
                mounts.mounts[i] = None;
                mounts.count -= 1;
                Ok(())
            }
            None => Err(VfsError::NotMounted),
        }
    }

    /// Crea un archivo o directorio.
    pub fn create(&self, path: &str, node_type: VfsNodeType) -> VfsResult<InodeId> {
        if !self.is_initialized() {
            return Err(VfsError::NotInitialized);
        }

        // Encontrar el montaje correspondiente
        let mounts = self.mounts.lock();
        let mount = mounts.find_by_path(path).ok_or(VfsError::NotMounted)?;
        let root_inode = mount.root_inode;
        let mount_id = mount.id;
        drop(mounts);

        // Crear inodo (delegar al filesystem subyacente en implementación completa)
        // Por ahora, crear un inodo local
        let mut inodes = self.inodes.lock();
        let mut name = [0u8; 64];
        let pbytes = path.as_bytes();
        let pcopy = pbytes.len().min(64);
        name[..pcopy].copy_from_slice(&pbytes[..pcopy]);

        let inode = VfsInode::new(root_inode, node_type, mount_id);
        let inode_id = inodes.insert(inode)?;
        Ok(inode_id)
    }

    /// Abre un archivo existente.
    pub fn open(&self, path: &str, flags: u32) -> VfsResult<FileId> {
        if !self.is_initialized() {
            return Err(VfsError::NotInitialized);
        }

        // Buscar inodo por path (simplificado)
        let inode_id = self.lookup(path)?;

        // Crear entrada en tabla de archivos abiertos
        let mut files = self.open_files.lock();
        let file = VfsFile {
            id: 0, // Se asignará en insert
            inode_id,
            offset: 0,
            flags,
            mount_id: 0,
        };
        let file_id = files.insert(file)?;
        Ok(file_id)
    }

    /// Cierra un archivo abierto.
    pub fn close(&self, file_id: FileId) -> VfsResult<()> {
        if !self.is_initialized() {
            return Err(VfsError::NotInitialized);
        }

        let mut files = self.open_files.lock();
        files.remove(file_id)
    }

    /// Lee de un archivo abierto.
    pub fn read(&self, file_id: FileId, buf: &mut [u8]) -> VfsResult<usize> {
        if !self.is_initialized() {
            return Err(VfsError::NotInitialized);
        }

        let mut files = self.open_files.lock();
        let file = files.get_mut(file_id).ok_or(VfsError::NotFound)?;

        if file.flags & O_RDONLY == 0 && file.flags & O_RDWR == 0 {
            return Err(VfsError::PermissionDenied);
        }

        // Leer del inodo
        let mut inodes = self.inodes.lock();
        let inode = inodes.get_mut(file.inode_id).ok_or(VfsError::NotFound)?;
        let offset = file.offset as usize;
        let end = (offset + buf.len()).min(inode.size as usize);
        if offset >= inode.size as usize {
            return Ok(0);
        }

        let read_len = end - offset;
        let buf_len = buf.len();
        let read_buf = &mut buf[..read_len.min(buf_len)];

        // Simular lectura (en producción: leer del dispositivo de bloque)
        for b in read_buf.iter_mut() {
            *b = 0;
        }

        file.offset += read_len as u64;
        Ok(read_len)
    }

    /// Escribe en un archivo abierto.
    pub fn write(&self, file_id: FileId, buf: &[u8]) -> VfsResult<usize> {
        if !self.is_initialized() {
            return Err(VfsError::NotInitialized);
        }

        let mut files = self.open_files.lock();
        let file = files.get_mut(file_id).ok_or(VfsError::NotFound)?;

        if file.flags & O_WRONLY == 0 && file.flags & O_RDWR == 0 {
            return Err(VfsError::PermissionDenied);
        }

        // Escribir al inodo
        let mut inodes = self.inodes.lock();
        let inode = inodes.get_mut(file.inode_id).ok_or(VfsError::NotFound)?;
        let offset = file.offset as usize;
        let write_len = buf.len();
        let new_size = (offset + write_len).max(inode.size as usize);

        // Simular escritura
        inode.size = new_size as u64;
        inode.mtime_us = time_us();

        file.offset += write_len as u64;
        Ok(write_len)
    }

    /// Busca un inodo por path.
    pub fn lookup(&self, _path: &str) -> VfsResult<InodeId> {
        if !self.is_initialized() {
            return Err(VfsError::NotInitialized);
        }

        // Simplificado: buscar en tabla de inodos
        let inodes = self.inodes.lock();
        for i in 0..inodes.count {
            if let Some(inode) = &inodes.inodes[i] {
                // En implementación completa, comparar rutas
                let _ = inode;
            }
        }
        drop(inodes);

        // Por ahora, devolver un inodo por defecto si existe
        Err(VfsError::NotFound)
    }

    /// Obtiene estadísticas de un inodo.
    pub fn stat(&self, inode_id: InodeId) -> VfsResult<VfsStat> {
        if !self.is_initialized() {
            return Err(VfsError::NotInitialized);
        }

        let inodes = self.inodes.lock();
        let inode = inodes.get(inode_id).ok_or(VfsError::NotFound)?;

        Ok(VfsStat {
            inode_id: inode.id,
            node_type: inode.node_type,
            size: inode.size,
            blocks: (inode.size + 4095) / 4096,
            atime_us: inode.atime_us,
            mtime_us: inode.mtime_us,
            ctime_us: inode.ctime_us,
            uid: inode.uid,
            gid: inode.gid,
            mode: inode.mode,
        })
    }

    /// Elimina un archivo o directorio.
    pub fn unlink(&self, path: &str) -> VfsResult<()> {
        if !self.is_initialized() {
            return Err(VfsError::NotInitialized);
        }

        let inode_id = self.lookup(path)?;

        // Remover de tabla de inodos
        let mut inodes = self.inodes.lock();
        inodes.inodes[inode_id as usize] = None;
        Ok(())
    }

    /// Devuelve el número de archivos abiertos.
    pub fn open_file_count(&self) -> usize {
        self.open_files.lock().count()
    }

    /// Devuelve el número de montajes.
    pub fn mount_count(&self) -> usize {
        self.mounts.lock().count()
    }

    /// Devuelve el número de inodos.
    pub fn inode_count(&self) -> usize {
        self.inodes.lock().count
    }
}

/// Tiempo actual en microsegundos.
fn time_us() -> u64 {
    static mut TIME_COUNTER: u64 = 0;
    unsafe {
        TIME_COUNTER += 1000;
        TIME_COUNTER
    }
}

/// Instancia global del VFS.
static mut VFS: Vfs = Vfs::new();

/// Devuelve una referencia mutable al VFS global.
#[allow(static_mut_refs)]
pub fn global_vfs() -> &'static mut Vfs {
    unsafe { &mut VFS }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vfs_node_types() {
        assert_eq!(VfsNodeType::RegularFile as u8, 0);
        assert_eq!(VfsNodeType::Directory as u8, 1);
        assert_eq!(VfsNodeType::Symlink as u8, 2);
    }

    #[test]
    fn vfs_inode_new() {
        let inode = VfsInode::new(1, VfsNodeType::RegularFile, 0);
        assert_eq!(inode.id, 1);
        assert_eq!(inode.node_type, VfsNodeType::RegularFile);
        assert_eq!(inode.size, 0);
        assert_eq!(inode.ref_count, 1);
    }

    #[test]
    fn vfs_new() {
        let vfs = Vfs::new();
        assert!(!vfs.is_initialized());
        assert_eq!(vfs.open_file_count(), 0);
        assert_eq!(vfs.mount_count(), 0);
    }

    #[test]
    fn vfs_init() {
        let vfs = Vfs::new();
        unsafe { vfs.init(); }
        assert!(vfs.is_initialized());
    }

    #[test]
    fn open_file_table() {
        let mut table = OpenFileTable::new();
        assert_eq!(table.count(), 0);

        let file = VfsFile {
            id: 0,
            inode_id: 1,
            offset: 0,
            flags: O_RDONLY,
            mount_id: 0,
        };
        let id = table.insert(file).unwrap();
        assert_eq!(table.count(), 1);

        let f = table.get(id).unwrap();
        assert_eq!(f.inode_id, 1);

        table.remove(id).unwrap();
        assert_eq!(table.count(), 0);
    }

    #[test]
    fn mount_table() {
        let mut table = MountTable::new();
        assert_eq!(table.count(), 0);

        let mut mp = [0u8; 64];
        mp[0..4].copy_from_slice(b"/mnt");
        let mut fs = [0u8; 32];
        fs[0..6].copy_from_slice(b"ext4\0\0");

        let id = table.insert(MountEntry {
            id: 1,
            mount_point: mp,
            fs_name: fs,
            root_inode: 1,
        }).unwrap();
        assert_eq!(id, 1);
        assert_eq!(table.count(), 1);
    }

    #[test]
    fn inode_table() {
        let mut table = InodeTable::new();
        assert_eq!(table.count(), 0);

        let inode = VfsInode::new(1, VfsNodeType::Directory, 0);
        let id = table.insert(inode).unwrap();
        assert_eq!(table.count(), 1);

        let retrieved = table.get(id).unwrap();
        assert_eq!(retrieved.id, 1);
    }

    #[test]
    fn vfs_errors() {
        assert_eq!(VfsError::NotFound as isize, 0);
        assert_eq!(VfsError::PermissionDenied as isize, 5);
        assert_eq!(VfsError::NotInitialized as isize, 8);
    }

    #[test]
    fn open_flags() {
        assert_eq!(O_RDONLY, 0);
        assert_eq!(O_WRONLY, 1);
        assert_eq!(O_RDWR, 2);
        assert_eq!(O_CREAT, 0x40);
    }
}
