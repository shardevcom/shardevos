//! VFS + shardevfs — Sistema de archivos virtual y filesystem nativo de shardevos.
//!
//! Este módulo combina:
//! - VFS: Abstracción de archivos, path resolution, open/read/write/close
//! - shardevfs: Filesystem nativo basado en bloques con superbloque, inodos, bitmap

use alloc::vec::Vec;
use core::cmp::min;
use core::sync::atomic::{AtomicU64, Ordering};

// ============================================================================
// Tipos básicos del VFS
// ============================================================================

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
    pub data_blocks: [u64; 16],
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
    pub flags: u32,
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

/// Entrada de directorio del VFS.
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

/// Trait para sistemas de archivos subyacentes.
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

// ============================================================================
// SpinLock
// ============================================================================

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

// ============================================================================
// Tablas del VFS
// ============================================================================

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

pub struct MountTable {
    mounts: [Option<MountEntry>; 64],
    count: usize,
}

pub struct MountEntry {
    pub id: MountId,
    pub mount_point: [u8; 64],
    pub fs_name: [u8; 32],
    pub root_inode: InodeId,
}

impl MountEntry {
    pub const fn new(id: MountId, mount_point: [u8; 64], fs_name: [u8; 32], root_inode: InodeId) -> Self {
        Self { id, mount_point, fs_name, root_inode }
    }
}

// MountEntry needs Copy for [None; 64] initialization
impl Copy for MountEntry {}
impl Clone for MountEntry {
    fn clone(&self) -> Self { *self }
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

    pub fn find_root_inode(&self) -> Option<InodeId> {
        self.mounts.iter().find_map(|m| m.as_ref()).map(|m| m.root_inode)
    }

    pub fn count(&self) -> usize {
        self.count
    }
}

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
        let final_id = if inode.id != 0 && inode.id as usize <= 1024 && self.inodes[inode.id as usize - 1].is_none() {
            inode.id
        } else {
            let next = self.inodes.iter().position(|i| i.is_none()).unwrap_or(0) + 1;
            next as InodeId
        };
        let idx = (final_id - 1) as usize;
        self.inodes[idx] = Some(inode);
        self.count += 1;
        Ok(final_id)
    }

    pub fn get(&self, id: InodeId) -> Option<&VfsInode> {
        self.inodes.get(id as usize).and_then(|i| i.as_ref())
    }

    pub fn get_mut(&mut self, id: InodeId) -> Option<&mut VfsInode> {
        self.inodes.get_mut(id as usize).and_then(|i| i.as_mut())
    }

    pub fn entry_at(&self, index: usize) -> Option<&VfsInode> {
        self.inodes.get(index).and_then(|i| i.as_ref())
    }

    pub fn count(&self) -> usize {
        self.count
    }
}

// ============================================================================
// Estructura VFS principal
// ============================================================================

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

    pub fn unmount(&self, mount_id: MountId) -> VfsResult<()> {
        if !self.is_initialized() {
            return Err(VfsError::NotInitialized);
        }

        let mut mounts = self.mounts.lock();
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

    pub fn resolve_path(&self, path: &str) -> VfsResult<InodeId> {
        if !self.is_initialized() {
            return Err(VfsError::NotInitialized);
        }

        let path = path.trim();
        if path.is_empty() || path == "/" {
            let mounts = self.mounts.lock();
            let root = mounts.find_root_inode().ok_or(VfsError::NotMounted)?;
            drop(mounts);
            return Ok(root);
        }

        let mut current_inode: InodeId;
        let mut remaining: &str;

        if path.starts_with('/') {
            let mounts = self.mounts.lock();
            let root = mounts.find_root_inode().ok_or(VfsError::NotMounted)?;
            current_inode = root;
            remaining = path.trim_start_matches('/');
            drop(mounts);
        } else {
            let mounts = self.mounts.lock();
            let root = mounts.find_root_inode().ok_or(VfsError::NotMounted)?;
            current_inode = root;
            remaining = path;
            drop(mounts);
        }

        let components: Vec<&str> = remaining.split('/').filter(|c| !c.is_empty()).collect();

        for comp in components {
            if comp == "." {
                continue;
            }
            if comp == ".." {
                let mounts = self.mounts.lock();
                let root = mounts.find_root_inode().ok_or(VfsError::NotMounted)?;
                current_inode = root;
                drop(mounts);
                continue;
            }

            let shardevfs = global_shardevfs_mut();
            let stat = shardevfs.get_stat(current_inode)?;
            if stat.node_type != VfsNodeType::Directory {
                return Err(VfsError::NotADirectory);
            }

            let found = self.shardevfs_lookup_child(current_inode, comp, shardevfs)?;
            current_inode = found;
        }

        Ok(current_inode)
    }

    fn shardevfs_lookup_child(&self, parent_id: InodeId, name: &str, fs: &Shardevfs) -> VfsResult<InodeId> {
        if parent_id == 1 {
            let entries = fs.list_directory(parent_id)?;
            for entry in entries.iter() {
                let entry_name = core::str::from_utf8(&entry.name).unwrap_or("").trim_end_matches(char::from(0));
                if entry_name == name {
                    return Ok(entry.inode_id);
                }
            }
        }
        Err(VfsError::NotFound)
    }

    pub fn create(&self, path: &str, node_type: VfsNodeType) -> VfsResult<InodeId> {
        if !self.is_initialized() {
            return Err(VfsError::NotInitialized);
        }

        let (parent_path, name) = split_path(path);
        if name.is_empty() {
            return Err(VfsError::InvalidArgument);
        }

        let parent_id = if parent_path.is_empty() || parent_path == "/" {
            let mounts = self.mounts.lock();
            let root = mounts.find_root_inode().ok_or(VfsError::NotMounted)?;
            drop(mounts);
            root
        } else {
            self.resolve_path(parent_path)?
        };

        let shardevfs = global_shardevfs_mut();
        let inode_id = shardevfs.create_inode(parent_id, name, node_type)?;

        let stat = shardevfs.get_stat(inode_id)?;
        let vfs_inode = VfsInode {
            id: stat.inode_id,
            node_type: stat.node_type,
            size: stat.size,
            data_blocks: [0; 16],
            atime_us: stat.atime_us,
            mtime_us: stat.mtime_us,
            ctime_us: stat.ctime_us,
            uid: stat.uid,
            gid: stat.gid,
            mode: stat.mode,
            ref_count: 1,
            mount_id: 1,
        };
        let mut inodes = self.inodes.lock();
        inodes.insert(vfs_inode)?;
        drop(inodes);

        Ok(inode_id)
    }

    pub fn open(&self, path: &str, flags: u32) -> VfsResult<FileId> {
        if !self.is_initialized() {
            return Err(VfsError::NotInitialized);
        }

        let inode_id = self.resolve_path(path)?;

        let mut files = self.open_files.lock();
        let file = VfsFile {
            id: 0,
            inode_id,
            offset: 0,
            flags,
            mount_id: 0,
        };
        let file_id = files.insert(file)?;
        Ok(file_id)
    }

    pub fn close(&self, file_id: FileId) -> VfsResult<()> {
        if !self.is_initialized() {
            return Err(VfsError::NotInitialized);
        }

        let mut files = self.open_files.lock();
        files.remove(file_id)
    }

    pub fn read(&self, file_id: FileId, buf: &mut [u8]) -> VfsResult<usize> {
        if !self.is_initialized() {
            return Err(VfsError::NotInitialized);
        }

        let mut files = self.open_files.lock();
        let file = files.get_mut(file_id).ok_or(VfsError::NotFound)?;

        if file.flags & O_RDONLY == 0 && file.flags & O_RDWR == 0 {
            return Err(VfsError::PermissionDenied);
        }

        let shardevfs = global_shardevfs_mut();
        let stat = shardevfs.get_stat(file.inode_id)?;
        let offset = file.offset as usize;
        if offset >= stat.size as usize {
            return Ok(0);
        }

        let read_len = min(offset + buf.len(), stat.size as usize) - offset;
        let mut read_buf = [0u8; 4096];
        let to_read = min(read_len, read_buf.len());
        let actually_read = shardevfs.read_data(file.inode_id, file.offset, &mut read_buf[..to_read])?;

        buf[..actually_read].copy_from_slice(&read_buf[..actually_read]);
        file.offset += actually_read as u64;
        Ok(actually_read)
    }

    pub fn write(&self, file_id: FileId, buf: &[u8]) -> VfsResult<usize> {
        if !self.is_initialized() {
            return Err(VfsError::NotInitialized);
        }

        let mut files = self.open_files.lock();
        let file = files.get_mut(file_id).ok_or(VfsError::NotFound)?;

        if file.flags & O_WRONLY == 0 && file.flags & O_RDWR == 0 {
            return Err(VfsError::PermissionDenied);
        }

        let shardevfs = global_shardevfs_mut();
        let written = shardevfs.write_data(file.inode_id, file.offset, buf)?;

        file.offset += written as u64;
        Ok(written)
    }

    pub fn lookup(&self, path: &str) -> VfsResult<InodeId> {
        self.resolve_path(path)
    }

    pub fn stat(&self, inode_id: InodeId) -> VfsResult<VfsStat> {
        if !self.is_initialized() {
            return Err(VfsError::NotInitialized);
        }

        let shardevfs = global_shardevfs_mut();
        shardevfs.get_stat(inode_id)
    }

    pub fn unlink(&self, path: &str) -> VfsResult<()> {
        if !self.is_initialized() {
            return Err(VfsError::NotInitialized);
        }

        let (parent_path, name) = split_path(path);
        if name.is_empty() {
            return Err(VfsError::InvalidArgument);
        }

        let parent_id = if parent_path.is_empty() || parent_path == "/" {
            let mounts = self.mounts.lock();
            let root = mounts.find_root_inode().ok_or(VfsError::NotMounted)?;
            drop(mounts);
            root
        } else {
            self.resolve_path(parent_path)?
        };

        let shardevfs = global_shardevfs_mut();
        shardevfs.delete_inode(parent_id, name)
    }

    pub fn open_file_count(&self) -> usize {
        self.open_files.lock().count()
    }

    pub fn mount_count(&self) -> usize {
        self.mounts.lock().count()
    }

    pub fn inode_count(&self) -> usize {
        self.inodes.lock().count
    }

    pub fn inode_entry_at(&self, index: usize) -> Option<VfsInode> {
        self.inodes.lock().entry_at(index).copied()
    }
}

fn split_path(path: &str) -> (&str, &str) {
    let path = path.trim_end_matches('/');
    if path == "/" || path.is_empty() {
        return ("", "");
    }

    if let Some(pos) = path.rfind('/') {
        let parent = &path[..pos];
        let name = &path[pos + 1..];
        if parent.is_empty() {
            ("/", name)
        } else {
            (parent, name)
        }
    } else {
        ("/", path)
    }
}

fn time_us() -> u64 {
    static mut TIME_COUNTER: u64 = 0;
    unsafe {
        TIME_COUNTER += 1000;
        TIME_COUNTER
    }
}

// ============================================================================
// Instancias globales
// ============================================================================

static mut VFS: Vfs = Vfs::new();

#[allow(static_mut_refs)]
pub fn global_vfs() -> &'static mut Vfs {
    unsafe { &mut VFS }
}

pub unsafe fn global_shardevfs_init() {
    global_shardevfs_mut().init();
}

// ============================================================================
// shardevfs — Filesystem nativo integrado
// ============================================================================

const SHARDEVFS_MAGIC: u32 = 0x53484152;
const BLOCK_SIZE: usize = 4096;
const MAX_INODES: usize = 256;
const MAX_DATA_BLOCKS: usize = 1024;

#[derive(Debug, Clone, Copy)]
#[repr(C)]
#[allow(dead_code)]
pub struct Superblock {
    pub magic: u32,
    pub version: u32,
    pub block_size: u32,
    pub max_inodes: u32,
    pub max_data_blocks: u32,
    pub inode_bitmap_block: u32,
    pub data_bitmap_block: u32,
    pub inode_table_block: u32,
    pub data_region_block: u32,
    pub root_inode_id: InodeId,
    pub created_at_us: u64,
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
#[allow(dead_code)]
pub struct DiskDirEntry {
    pub name: [u8; 64],
    pub inode_id: InodeId,
    pub node_type: u8,
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
#[allow(dead_code)]
pub struct DiskInode {
    pub id: InodeId,
    pub node_type: u8,
    pub size: u64,
    pub data_blocks: [u64; 16],
    pub atime_us: u64,
    pub mtime_us: u64,
    pub ctime_us: u64,
    pub uid: u32,
    pub gid: u32,
    pub mode: u32,
}

#[allow(dead_code)]
pub struct Shardevfs {
    superblock: Superblock,
    inode_bitmap: [u8; 32],
    data_bitmap: [u8; 128],
    inodes: [DiskInode; MAX_INODES],
    root_entries: [DiskDirEntry; 64],
    root_entry_count: usize,
    data_store: [u8; 65536],
    initialized: AtomicU64,
}

impl Shardevfs {
    pub const fn new() -> Self {
        Self {
            superblock: Superblock {
                magic: SHARDEVFS_MAGIC,
                version: 1,
                block_size: BLOCK_SIZE as u32,
                max_inodes: MAX_INODES as u32,
                max_data_blocks: MAX_DATA_BLOCKS as u32,
                inode_bitmap_block: 1,
                data_bitmap_block: 2,
                inode_table_block: 3,
                data_region_block: 4,
                root_inode_id: 1,
                created_at_us: 0,
            },
            inode_bitmap: [0; 32],
            data_bitmap: [0; 128],
            inodes: [DiskInode {
                id: 0, node_type: 0, size: 0, data_blocks: [0; 16],
                atime_us: 0, mtime_us: 0, ctime_us: 0,
                uid: 0, gid: 0, mode: 0,
            }; MAX_INODES],
            root_entries: [DiskDirEntry {
                name: [0; 64], inode_id: 0, node_type: 0,
            }; 64],
            root_entry_count: 0,
            data_store: [0; 65536],
            initialized: AtomicU64::new(0),
        }
    }

    pub unsafe fn init(&mut self) {
        self.initialized.store(1, Ordering::Release);
        let root = DiskInode {
            id: 1, node_type: VfsNodeType::Directory as u8, size: 0,
            data_blocks: [0; 16], atime_us: time_us(), mtime_us: time_us(),
            ctime_us: time_us(), uid: 0, gid: 0, mode: 0o755,
        };
        self.inodes[0] = root;
        self.inode_bitmap[0] |= 0x01;
        self.superblock.root_inode_id = 1;
        self.superblock.created_at_us = time_us();
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::Acquire) != 0
    }

    fn alloc_inode(&mut self) -> Result<InodeId, VfsError> {
        for i in 0..MAX_INODES {
            let byte_idx = i / 8;
            let bit_idx = i % 8;
            if self.inode_bitmap[byte_idx] & (1 << bit_idx) == 0 {
                self.inode_bitmap[byte_idx] |= 1 << bit_idx;
                return Ok(i as InodeId + 1);
            }
        }
        Err(VfsError::NoSpace)
    }

    fn free_inode(&mut self, inode_id: InodeId) {
        if inode_id == 0 || inode_id > MAX_INODES as InodeId { return; }
        let idx = (inode_id - 1) as usize;
        let byte_idx = idx / 8;
        let bit_idx = idx % 8;
        self.inode_bitmap[byte_idx] &= !(1 << bit_idx);
    }

    fn alloc_data_block(&mut self) -> Result<u64, VfsError> {
        for i in 0..MAX_DATA_BLOCKS {
            let byte_idx = i / 8;
            let bit_idx = i % 8;
            if self.data_bitmap[byte_idx] & (1 << bit_idx) == 0 {
                self.data_bitmap[byte_idx] |= 1 << bit_idx;
                return Ok(i as u64 + 4);
            }
        }
        Err(VfsError::NoSpace)
    }

    fn find_dir_entry(&self, name: &str) -> Option<&DiskDirEntry> {
        for i in 0..self.root_entry_count {
            let entry_name = core::str::from_utf8(&self.root_entries[i].name).unwrap_or("");
            let entry_name_trimmed = entry_name.trim_end_matches(char::from(0));
            if entry_name_trimmed == name {
                return Some(&self.root_entries[i]);
            }
        }
        None
    }

    fn add_dir_entry(&mut self, name: &str, inode_id: InodeId, node_type: VfsNodeType) -> Result<(), VfsError> {
        if self.root_entry_count >= 64 {
            return Err(VfsError::NoSpace);
        }
        let mut name_arr = [0u8; 64];
        let bytes = name.as_bytes();
        let copy_len = bytes.len().min(64);
        name_arr[..copy_len].copy_from_slice(&bytes[..copy_len]);
        self.root_entries[self.root_entry_count] = DiskDirEntry {
            name: name_arr, inode_id, node_type: node_type as u8,
        };
        self.root_entry_count += 1;
        Ok(())
    }
}

impl Filesystem for Shardevfs {
    fn name(&self) -> &str { "shardevfs" }

    fn mount(&mut self, _mount_point: &str) -> VfsResult<MountId> {
        if !self.is_initialized() { return Err(VfsError::NotInitialized); }
        Ok(1)
    }

    fn unmount(&mut self, _mount_id: MountId) -> VfsResult<()> { Ok(()) }

    fn create_inode(&mut self, _parent_id: InodeId, name: &str, node_type: VfsNodeType) -> VfsResult<InodeId> {
        if !self.is_initialized() { return Err(VfsError::NotInitialized); }
        if self.find_dir_entry(name).is_some() { return Err(VfsError::AlreadyExists); }
        let inode_id = self.alloc_inode()?;
        let disk_inode = DiskInode {
            id: inode_id, node_type: node_type as u8, size: 0,
            data_blocks: [0; 16], atime_us: time_us(), mtime_us: time_us(),
            ctime_us: time_us(), uid: 0, gid: 0,
            mode: match node_type { VfsNodeType::Directory => 0o755, _ => 0o644 },
        };
        self.inodes[inode_id as usize - 1] = disk_inode;
        self.add_dir_entry(name, inode_id, node_type)?;
        Ok(inode_id)
    }

    fn lookup_inode(&self, _parent_id: InodeId, name: &str) -> VfsResult<InodeId> {
        if !self.is_initialized() { return Err(VfsError::NotInitialized); }
        if let Some(entry) = self.find_dir_entry(name) { Ok(entry.inode_id) } else { Err(VfsError::NotFound) }
    }

    fn read_inode(&self, inode_id: InodeId) -> VfsResult<VfsInode> {
        if !self.is_initialized() { return Err(VfsError::NotInitialized); }
        if inode_id == 0 || inode_id > MAX_INODES as InodeId { return Err(VfsError::NotFound); }
        let disk_inode = &self.inodes[inode_id as usize - 1];
        if disk_inode.id == 0 { return Err(VfsError::NotFound); }
        Ok(VfsInode {
            id: disk_inode.id,
            node_type: match disk_inode.node_type { 0=>VfsNodeType::RegularFile, 1=>VfsNodeType::Directory, 2=>VfsNodeType::Symlink, 3=>VfsNodeType::CharacterDevice, 4=>VfsNodeType::BlockDevice, 5=>VfsNodeType::Socket, 6=>VfsNodeType::Fifo, _=>VfsNodeType::RegularFile },
            size: disk_inode.size, data_blocks: disk_inode.data_blocks,
            atime_us: disk_inode.atime_us, mtime_us: disk_inode.mtime_us, ctime_us: disk_inode.ctime_us,
            uid: disk_inode.uid, gid: disk_inode.gid, mode: disk_inode.mode,
            ref_count: 1, mount_id: 1,
        })
    }

    fn write_inode(&mut self, inode: &VfsInode) -> VfsResult<()> {
        if !self.is_initialized() { return Err(VfsError::NotInitialized); }
        if inode.id == 0 || inode.id > MAX_INODES as InodeId { return Err(VfsError::NotFound); }
        let disk_inode = &mut self.inodes[inode.id as usize - 1];
        disk_inode.id = inode.id; disk_inode.node_type = inode.node_type as u8;
        disk_inode.size = inode.size; disk_inode.data_blocks = inode.data_blocks;
        disk_inode.atime_us = inode.atime_us; disk_inode.mtime_us = inode.mtime_us;
        disk_inode.ctime_us = inode.ctime_us; disk_inode.uid = inode.uid;
        disk_inode.gid = inode.gid; disk_inode.mode = inode.mode;
        Ok(())
    }

    fn read_data(&self, inode_id: InodeId, offset: u64, buf: &mut [u8]) -> VfsResult<usize> {
        if !self.is_initialized() { return Err(VfsError::NotInitialized); }
        if inode_id == 0 || inode_id > MAX_INODES as InodeId { return Err(VfsError::NotFound); }
        let disk_inode = &self.inodes[inode_id as usize - 1];
        if disk_inode.id == 0 { return Err(VfsError::NotFound); }
        if disk_inode.node_type != VfsNodeType::RegularFile as u8 { return Err(VfsError::InvalidArgument); }
        let offset = offset as usize;
        if offset >= disk_inode.size as usize { return Ok(0); }
        let read_len = (offset + buf.len()).min(disk_inode.size as usize) - offset;
        buf[..read_len].copy_from_slice(&self.data_store[offset..offset + read_len]);
        Ok(read_len)
    }

    fn write_data(&mut self, inode_id: InodeId, offset: u64, buf: &[u8]) -> VfsResult<usize> {
        if !self.is_initialized() { return Err(VfsError::NotInitialized); }
        if inode_id == 0 || inode_id > MAX_INODES as InodeId { return Err(VfsError::NotFound); }
        {
            let disk_inode = &self.inodes[inode_id as usize - 1];
            if disk_inode.id == 0 { return Err(VfsError::NotFound); }
            if disk_inode.node_type != VfsNodeType::RegularFile as u8 { return Err(VfsError::InvalidArgument); }
        }
        let offset = offset as usize;
        let write_len = buf.len();
        let new_size = (offset + write_len).max(self.inodes[inode_id as usize - 1].size as usize);
        let blocks_needed = (new_size + BLOCK_SIZE - 1) / BLOCK_SIZE;
        let mut new_blocks: [u64; 16] = [0; 16];
        let mut blocks_allocated = 0;
        for i in 0..16 {
            let di = &self.inodes[inode_id as usize - 1];
            if di.data_blocks[i] != 0 {
                new_blocks[i] = di.data_blocks[i];
            } else if blocks_allocated < blocks_needed {
                let block = self.alloc_data_block()?;
                new_blocks[i] = block;
                blocks_allocated += 1;
            }
        }
        self.data_store[offset..offset + write_len].copy_from_slice(buf);
        let disk_inode = &mut self.inodes[inode_id as usize - 1];
        disk_inode.data_blocks = new_blocks;
        disk_inode.size = new_size as u64;
        disk_inode.mtime_us = time_us();
        Ok(write_len)
    }

    fn list_directory(&self, inode_id: InodeId) -> VfsResult<Vec<VfsDirEntry>> {
        if !self.is_initialized() { return Err(VfsError::NotInitialized); }
        if inode_id != 1 { return Err(VfsError::NotADirectory); }
        let mut entries = Vec::new();
        for i in 0..self.root_entry_count {
            entries.push(VfsDirEntry {
                name: self.root_entries[i].name,
                inode_id: self.root_entries[i].inode_id,
                node_type: match self.root_entries[i].node_type {
                    0=>VfsNodeType::RegularFile, 1=>VfsNodeType::Directory, 2=>VfsNodeType::Symlink,
                    3=>VfsNodeType::CharacterDevice, 4=>VfsNodeType::BlockDevice, 5=>VfsNodeType::Socket, 6=>VfsNodeType::Fifo,
                    _=>VfsNodeType::RegularFile,
                },
            });
        }
        Ok(entries)
    }

    fn delete_inode(&mut self, _parent_id: InodeId, name: &str) -> VfsResult<()> {
        if !self.is_initialized() { return Err(VfsError::NotInitialized); }
        let entry_idx = (0..self.root_entry_count).find(|i| {
            let name_str = core::str::from_utf8(&self.root_entries[*i].name).unwrap_or("");
            name_str.trim_end_matches(char::from(0)) == name
        });
        match entry_idx {
            Some(idx) => {
                let inode_id = self.root_entries[idx].inode_id;
                self.free_inode(inode_id);
                self.root_entries[idx] = self.root_entries[self.root_entry_count - 1];
                self.root_entry_count -= 1;
                Ok(())
            }
            None => Err(VfsError::NotFound),
        }
    }

    fn get_stat(&self, inode_id: InodeId) -> VfsResult<VfsStat> {
        if !self.is_initialized() { return Err(VfsError::NotInitialized); }
        if inode_id == 0 || inode_id > MAX_INODES as InodeId { return Err(VfsError::NotFound); }
        let disk_inode = &self.inodes[inode_id as usize - 1];
        if disk_inode.id == 0 { return Err(VfsError::NotFound); }
        Ok(VfsStat {
            inode_id: disk_inode.id,
            node_type: match disk_inode.node_type { 0=>VfsNodeType::RegularFile, 1=>VfsNodeType::Directory, 2=>VfsNodeType::Symlink, 3=>VfsNodeType::CharacterDevice, 4=>VfsNodeType::BlockDevice, 5=>VfsNodeType::Socket, 6=>VfsNodeType::Fifo, _=>VfsNodeType::RegularFile },
            size: disk_inode.size,
            blocks: (disk_inode.size + BLOCK_SIZE as u64 - 1) / BLOCK_SIZE as u64,
            atime_us: disk_inode.atime_us, mtime_us: disk_inode.mtime_us, ctime_us: disk_inode.ctime_us,
            uid: disk_inode.uid, gid: disk_inode.gid, mode: disk_inode.mode,
        })
    }
}

static mut SHARDEVFS: Shardevfs = Shardevfs::new();

#[allow(dead_code)]
pub fn global_shardevfs_mut() -> &'static mut Shardevfs {
    unsafe { &mut SHARDEVFS }
}

// ============================================================================
// Pruebas
// ============================================================================

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
        assert_eq!(vfs.inode_count(), 0);
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
        let file = VfsFile { id: 0, inode_id: 1, offset: 0, flags: O_RDONLY, mount_id: 0 };
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
        let mut mp = [0u8; 64]; mp[0..4].copy_from_slice(b"/mnt");
        let mut fs = [0u8; 32]; fs[0..6].copy_from_slice(b"ext4\0\0");
        let id = table.insert(MountEntry { id: 1, mount_point: mp, fs_name: fs, root_inode: 1 }).unwrap();
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
        assert_eq!(O_RDONLY, 0); assert_eq!(O_WRONLY, 1);
        assert_eq!(O_RDWR, 2); assert_eq!(O_CREAT, 0x40);
    }

    #[test]
    fn split_path_tests() {
        assert_eq!(split_path("/foo/bar.txt"), ("/foo", "bar.txt"));
        assert_eq!(split_path("/"), ("", ""));
        assert_eq!(split_path(""), ("", ""));
        assert_eq!(split_path("file.txt"), ("/", "file.txt"));
        assert_eq!(split_path("/foo/"), ("/foo", ""));
    }

    #[test]
    fn shardevfs_init() {
        let mut fs = Shardevfs::new();
        unsafe { fs.init(); }
        assert!(fs.is_initialized());
    }

    #[test]
    fn shardevfs_create_and_lookup() {
        let mut fs = Shardevfs::new();
        unsafe { fs.init(); }
        let inode_id = fs.create_inode(1, "test.txt", VfsNodeType::RegularFile).unwrap();
        assert_eq!(inode_id, 2);
        let found = fs.lookup_inode(1, "test.txt").unwrap();
        assert_eq!(found, inode_id);
        let result = fs.create_inode(1, "test.txt", VfsNodeType::RegularFile);
        assert_eq!(result, Err(VfsError::AlreadyExists));
    }

    #[test]
    fn shardevfs_read_write() {
        let mut fs = Shardevfs::new();
        unsafe { fs.init(); }
        let inode_id = fs.create_inode(1, "data.bin", VfsNodeType::RegularFile).unwrap();
        let write_data = b"Hello, shardevfs!";
        let written = fs.write_data(inode_id, 0, write_data).unwrap();
        assert_eq!(written, write_data.len());
        let mut read_buf = [0u8; 32];
        let read = fs.read_data(inode_id, 0, &mut read_buf).unwrap();
        assert_eq!(read, write_data.len());
        assert_eq!(&read_buf[..read], write_data);
    }

    #[test]
    fn shardevfs_list_directory() {
        let mut fs = Shardevfs::new();
        unsafe { fs.init(); }
        fs.create_inode(1, "file1.txt", VfsNodeType::RegularFile).unwrap();
        fs.create_inode(1, "file2.txt", VfsNodeType::RegularFile).unwrap();
        fs.create_inode(1, "subdir", VfsNodeType::Directory).unwrap();
        let entries = fs.list_directory(1).unwrap();
        assert_eq!(entries.len(), 3);
    }
}
