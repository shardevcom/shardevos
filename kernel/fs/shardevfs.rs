//! shardevfs — Sistema de archivos nativo de shardevos.
//!
//! Implementa el trait Filesystem del VFS con un sistema de archivos simple
//! basado en bloques, con:
//! - Superbloque con metadatos del filesystem
//! - Tabla de inodos
//! - Bitmap de bloques libres
//! - Directorio raíz
//! - Operaciones de create, lookup, read, write, delete


use super::{
    Filesystem, VfsError, VfsResult, VfsInode, VfsDirEntry, VfsNodeType,
    InodeId, MountId, VfsStat,
};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

/// Magic number del filesystem shardevfs.
#[allow(dead_code)]
const SHARDEVFS_MAGIC: u32 = 0x53484152; // "SHAR"

/// Tamaño de bloque en bytes.
#[allow(dead_code)]
const BLOCK_SIZE: usize = 4096;

/// Número máximo de inodos.
#[allow(dead_code)]
const MAX_INODES: usize = 256;

/// Número máximo de bloques de datos.
#[allow(dead_code)]
const MAX_DATA_BLOCKS: usize = 1024;

/// Superbloque del filesystem.
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

/// Entrada de directorio en disco.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
#[allow(dead_code)]
pub struct DiskDirEntry {
    pub name: [u8; 64],
    pub inode_id: InodeId,
    pub node_type: u8,
}

/// Inodo en disco.
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

/// Motor del filesystem shardevfs.
#[allow(dead_code)]
pub struct Shardevfs {
    superblock: Superblock,
    inode_bitmap: [u8; 32],  // 256 bits = 256 inodos
    data_bitmap: [u8; 128],  // 1024 bits = 1024 bloques
    inodes: [DiskInode; MAX_INODES],
    root_entries: [DiskDirEntry; 64],
    root_entry_count: usize,
    data_store: [u8; 65536],  // 64KB de almacenamiento de datos
    initialized: AtomicU64,
}

#[allow(dead_code)]
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
                id: 0,
                node_type: 0,
                size: 0,
                data_blocks: [0; 16],
                atime_us: 0,
                mtime_us: 0,
                ctime_us: 0,
                uid: 0,
                gid: 0,
                mode: 0,
            }; MAX_INODES],
            root_entries: [DiskDirEntry {
                name: [0; 64],
                inode_id: 0,
                node_type: 0,
            }; 64],
            root_entry_count: 0,
            data_store: [0; 65536],
            initialized: AtomicU64::new(0),
        }
    }

    pub unsafe fn init(&mut self) {
        self.initialized.store(1, Ordering::Release);

        // Crear inodo raíz (directorio /)
        let root = DiskInode {
            id: 1,
            node_type: VfsNodeType::Directory as u8,
            size: 0,
            data_blocks: [0; 16],
            atime_us: time_us(),
            mtime_us: time_us(),
            ctime_us: time_us(),
            uid: 0,
            gid: 0,
            mode: 0o755,
        };
        self.inodes[0] = root;
        self.inode_bitmap[0] |= 0x01; // Marcar inodo 1 como usado
        self.superblock.root_inode_id = 1;
        self.superblock.created_at_us = time_us();
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::Acquire) != 0
    }

    /// Asigna un nuevo inodo.
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

    /// Libera un inodo.
    fn free_inode(&mut self, inode_id: InodeId) {
        if inode_id == 0 || inode_id > MAX_INODES as InodeId {
            return;
        }
        let idx = (inode_id - 1) as usize;
        let byte_idx = idx / 8;
        let bit_idx = idx % 8;
        self.inode_bitmap[byte_idx] &= !(1 << bit_idx);
    }

    /// Asigna un bloque de datos.
    fn alloc_data_block(&mut self) -> Result<u64, VfsError> {
        for i in 0..MAX_DATA_BLOCKS {
            let byte_idx = i / 8;
            let bit_idx = i % 8;
            if self.data_bitmap[byte_idx] & (1 << bit_idx) == 0 {
                self.data_bitmap[byte_idx] |= 1 << bit_idx;
                return Ok(i as u64 + 4); // Bloques de datos empiezan en el bloque 4
            }
        }
        Err(VfsError::NoSpace)
    }

    /// Busca una entrada de directorio por nombre.
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

    /// Agrega una entrada al directorio raíz.
    fn add_dir_entry(&mut self, name: &str, inode_id: InodeId, node_type: VfsNodeType) -> Result<(), VfsError> {
        if self.root_entry_count >= 64 {
            return Err(VfsError::NoSpace);
        }

        let mut name_arr = [0u8; 64];
        let bytes = name.as_bytes();
        let copy_len = bytes.len().min(64);
        name_arr[..copy_len].copy_from_slice(&bytes[..copy_len]);

        self.root_entries[self.root_entry_count] = DiskDirEntry {
            name: name_arr,
            inode_id,
            node_type: node_type as u8,
        };
        self.root_entry_count += 1;
        Ok(())
    }
}

impl Filesystem for Shardevfs {
    fn name(&self) -> &str {
        "shardevfs"
    }

    fn mount(&mut self, _mount_point: &str) -> VfsResult<MountId> {
        if !self.is_initialized() {
            return Err(VfsError::NotInitialized);
        }
        Ok(1)
    }

    fn unmount(&mut self, _mount_id: MountId) -> VfsResult<()> {
        Ok(())
    }

    fn create_inode(&mut self, _parent_id: InodeId, name: &str, node_type: VfsNodeType) -> VfsResult<InodeId> {
        if !self.is_initialized() {
            return Err(VfsError::NotInitialized);
        }

        // Verificar que no exista ya
        if self.find_dir_entry(name).is_some() {
            return Err(VfsError::AlreadyExists);
        }

        // Asignar nuevo inodo
        let inode_id = self.alloc_inode()?;

        // Crear inodo en disco
        let disk_inode = DiskInode {
            id: inode_id,
            node_type: node_type as u8,
            size: 0,
            data_blocks: [0; 16],
            atime_us: time_us(),
            mtime_us: time_us(),
            ctime_us: time_us(),
            uid: 0,
            gid: 0,
            mode: match node_type {
                VfsNodeType::Directory => 0o755,
                _ => 0o644,
            },
        };
        self.inodes[inode_id as usize - 1] = disk_inode;

        // Agregar entrada al directorio raíz
        self.add_dir_entry(name, inode_id, node_type)?;

        Ok(inode_id)
    }

    fn lookup_inode(&self, _parent_id: InodeId, name: &str) -> VfsResult<InodeId> {
        if !self.is_initialized() {
            return Err(VfsError::NotInitialized);
        }

        if let Some(entry) = self.find_dir_entry(name) {
            Ok(entry.inode_id)
        } else {
            Err(VfsError::NotFound)
        }
    }

    fn read_inode(&self, inode_id: InodeId) -> VfsResult<VfsInode> {
        if !self.is_initialized() {
            return Err(VfsError::NotInitialized);
        }

        if inode_id == 0 || inode_id > MAX_INODES as InodeId {
            return Err(VfsError::NotFound);
        }

        let disk_inode = &self.inodes[inode_id as usize - 1];
        if disk_inode.id == 0 {
            return Err(VfsError::NotFound);
        }

        Ok(VfsInode {
            id: disk_inode.id,
            node_type: match disk_inode.node_type {
                0 => VfsNodeType::RegularFile,
                1 => VfsNodeType::Directory,
                2 => VfsNodeType::Symlink,
                3 => VfsNodeType::CharacterDevice,
                4 => VfsNodeType::BlockDevice,
                5 => VfsNodeType::Socket,
                6 => VfsNodeType::Fifo,
                _ => VfsNodeType::RegularFile,
            },
            size: disk_inode.size,
            data_blocks: disk_inode.data_blocks,
            atime_us: disk_inode.atime_us,
            mtime_us: disk_inode.mtime_us,
            ctime_us: disk_inode.ctime_us,
            uid: disk_inode.uid,
            gid: disk_inode.gid,
            mode: disk_inode.mode,
            ref_count: 1,
            mount_id: 1,
        })
    }

    fn write_inode(&mut self, inode: &VfsInode) -> VfsResult<()> {
        if !self.is_initialized() {
            return Err(VfsError::NotInitialized);
        }

        if inode.id == 0 || inode.id > MAX_INODES as InodeId {
            return Err(VfsError::NotFound);
        }

        let disk_inode = &mut self.inodes[inode.id as usize - 1];
        disk_inode.id = inode.id;
        disk_inode.node_type = inode.node_type as u8;
        disk_inode.size = inode.size;
        disk_inode.data_blocks = inode.data_blocks;
        disk_inode.atime_us = inode.atime_us;
        disk_inode.mtime_us = inode.mtime_us;
        disk_inode.ctime_us = inode.ctime_us;
        disk_inode.uid = inode.uid;
        disk_inode.gid = inode.gid;
        disk_inode.mode = inode.mode;

        Ok(())
    }

    fn read_data(&self, inode_id: InodeId, offset: u64, buf: &mut [u8]) -> VfsResult<usize> {
        if !self.is_initialized() {
            return Err(VfsError::NotInitialized);
        }

        if inode_id == 0 || inode_id > MAX_INODES as InodeId {
            return Err(VfsError::NotFound);
        }

        let disk_inode = &self.inodes[inode_id as usize - 1];
        if disk_inode.id == 0 {
            return Err(VfsError::NotFound);
        }

        if disk_inode.node_type != VfsNodeType::RegularFile as u8 {
            return Err(VfsError::InvalidArgument);
        }

        let offset = offset as usize;
        if offset >= disk_inode.size as usize {
            return Ok(0);
        }

        let read_len = (offset + buf.len()).min(disk_inode.size as usize) - offset;

        // Leer datos del data_store
        buf[..read_len].copy_from_slice(&self.data_store[offset..offset + read_len]);

        Ok(read_len)
    }

    fn write_data(&mut self, inode_id: InodeId, offset: u64, buf: &[u8]) -> VfsResult<usize> {
        if !self.is_initialized() {
            return Err(VfsError::NotInitialized);
        }

        if inode_id == 0 || inode_id > MAX_INODES as InodeId {
            return Err(VfsError::NotFound);
        }

        // Verificar inodo antes de tomar préstamo mutable
        {
            let disk_inode = &self.inodes[inode_id as usize - 1];
            if disk_inode.id == 0 {
                return Err(VfsError::NotFound);
            }
            if disk_inode.node_type != VfsNodeType::RegularFile as u8 {
                return Err(VfsError::InvalidArgument);
            }
        }

        let offset = offset as usize;
        let write_len = buf.len();
        let new_size = (offset + write_len).max({
            let di = &self.inodes[inode_id as usize - 1];
            di.size as usize
        });

        // Asignar bloques de datos si es necesario (primero, sin mantener préstamo del inodo)
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

        // Escribir datos en el data_store
        self.data_store[offset..offset + write_len].copy_from_slice(buf);

        // Ahora actualizar el inodo
        let disk_inode = &mut self.inodes[inode_id as usize - 1];
        disk_inode.data_blocks = new_blocks;
        disk_inode.size = new_size as u64;
        disk_inode.mtime_us = time_us();

        Ok(write_len)
    }

    fn list_directory(&self, inode_id: InodeId) -> VfsResult<Vec<VfsDirEntry>> {
        if !self.is_initialized() {
            return Err(VfsError::NotInitialized);
        }

        if inode_id != 1 {
            return Err(VfsError::NotADirectory);
        }

        let mut entries = Vec::new();
        for i in 0..self.root_entry_count {
            entries.push(VfsDirEntry {
                name: self.root_entries[i].name,
                inode_id: self.root_entries[i].inode_id,
                node_type: match self.root_entries[i].node_type {
                    0 => VfsNodeType::RegularFile,
                    1 => VfsNodeType::Directory,
                    2 => VfsNodeType::Symlink,
                    3 => VfsNodeType::CharacterDevice,
                    4 => VfsNodeType::BlockDevice,
                    5 => VfsNodeType::Socket,
                    6 => VfsNodeType::Fifo,
                    _ => VfsNodeType::RegularFile,
                },
            });
        }

        Ok(entries)
    }

    fn delete_inode(&mut self, _parent_id: InodeId, name: &str) -> VfsResult<()> {
        if !self.is_initialized() {
            return Err(VfsError::NotInitialized);
        }

        // Buscar la entrada
        let entry_idx = (0..self.root_entry_count)
            .find(|i| {
                let entry = &self.root_entries[*i];
                let name_str = core::str::from_utf8(&entry.name).unwrap_or("");
                name_str.trim_end_matches(char::from(0)) == name
            });

        match entry_idx {
            Some(idx) => {
                let inode_id = self.root_entries[idx].inode_id;

                // Liberar inodo
                self.free_inode(inode_id);

                // Remover entrada del directorio
                self.root_entries[idx] = self.root_entries[self.root_entry_count - 1];
                self.root_entry_count -= 1;

                Ok(())
            }
            None => Err(VfsError::NotFound),
        }
    }

    fn get_stat(&self, inode_id: InodeId) -> VfsResult<VfsStat> {
        if !self.is_initialized() {
            return Err(VfsError::NotInitialized);
        }

        if inode_id == 0 || inode_id > MAX_INODES as InodeId {
            return Err(VfsError::NotFound);
        }

        let disk_inode = &self.inodes[inode_id as usize - 1];
        if disk_inode.id == 0 {
            return Err(VfsError::NotFound);
        }

        Ok(VfsStat {
            inode_id: disk_inode.id,
            node_type: match disk_inode.node_type {
                0 => VfsNodeType::RegularFile,
                1 => VfsNodeType::Directory,
                2 => VfsNodeType::Symlink,
                3 => VfsNodeType::CharacterDevice,
                4 => VfsNodeType::BlockDevice,
                5 => VfsNodeType::Socket,
                6 => VfsNodeType::Fifo,
                _ => VfsNodeType::RegularFile,
            },
            size: disk_inode.size,
            blocks: (disk_inode.size + BLOCK_SIZE as u64 - 1) / BLOCK_SIZE as u64,
            atime_us: disk_inode.atime_us,
            mtime_us: disk_inode.mtime_us,
            ctime_us: disk_inode.ctime_us,
            uid: disk_inode.uid,
            gid: disk_inode.gid,
            mode: disk_inode.mode,
        })
    }
}

/// Tiempo actual en microsegundos.
#[allow(dead_code)]
fn time_us() -> u64 {
    static mut TIME_COUNTER: u64 = 0;
    unsafe {
        TIME_COUNTER += 1000;
        TIME_COUNTER
    }
}

/// Instancia global del filesystem shardevfs.
#[allow(dead_code)]
static mut SHARDEVFS: Shardevfs = Shardevfs::new();

/// Devuelve una referencia mutable al filesystem global.
#[allow(static_mut_refs)]
#[allow(dead_code)]
pub fn global_shardevfs() -> &'static mut Shardevfs {
    unsafe { &mut SHARDEVFS }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fs_new() {
        let fs = Shardevfs::new();
        assert!(!fs.is_initialized());
        assert_eq!(fs.name(), "shardevfs");
    }

    #[test]
    fn fs_init() {
        let mut fs = Shardevfs::new();
        unsafe { fs.init(); }
        assert!(fs.is_initialized());
    }

    #[test]
    fn fs_mount() {
        let mut fs = Shardevfs::new();
        unsafe { fs.init(); }
        let id = fs.mount("/mnt").unwrap();
        assert_eq!(id, 1);
    }

    #[test]
    fn fs_create_and_lookup() {
        let mut fs = Shardevfs::new();
        unsafe { fs.init(); }

        let inode_id = fs.create_inode(1, "test.txt", VfsNodeType::RegularFile).unwrap();
        assert_eq!(inode_id, 2); // El inodo 1 es el raíz

        let found = fs.lookup_inode(1, "test.txt").unwrap();
        assert_eq!(found, inode_id);

        // Intentar crear duplicado
        let result = fs.create_inode(1, "test.txt", VfsNodeType::RegularFile);
        assert_eq!(result, Err(VfsError::AlreadyExists));
    }

    #[test]
    fn fs_lookup_not_found() {
        let mut fs = Shardevfs::new();
        unsafe { fs.init(); }
        let result = fs.lookup_inode(1, "nonexistent.txt");
        assert_eq!(result, Err(VfsError::NotFound));
    }

    #[test]
    fn fs_read_write_inode() {
        let mut fs = Shardevfs::new();
        unsafe { fs.init(); }

        let inode_id = fs.create_inode(1, "data.bin", VfsNodeType::RegularFile).unwrap();

        // Leer inodo
        let inode = fs.read_inode(inode_id).unwrap();
        assert_eq!(inode.id, inode_id);
        assert_eq!(inode.size, 0);

        // Escribir datos
        let write_data = b"Hello, shardevfs!";
        let written = fs.write_data(inode_id, 0, write_data).unwrap();
        assert_eq!(written, write_data.len());

        // Leer inodo actualizado
        let inode = fs.read_inode(inode_id).unwrap();
        assert_eq!(inode.size, write_data.len() as u64);

        // Leer datos
        let mut read_buf = [0u8; 32];
        let read = fs.read_data(inode_id, 0, &mut read_buf).unwrap();
        assert_eq!(read, write_data.len());
        assert_eq!(&read_buf[..read], write_data);
    }

    #[test]
    fn fs_list_directory() {
        let mut fs = Shardevfs::new();
        unsafe { fs.init(); }

        fs.create_inode(1, "file1.txt", VfsNodeType::RegularFile).unwrap();
        fs.create_inode(1, "file2.txt", VfsNodeType::RegularFile).unwrap();
        fs.create_inode(1, "subdir", VfsNodeType::Directory).unwrap();

        let entries = fs.list_directory(1).unwrap();
        assert_eq!(entries.len(), 3);

        let names: Vec<&str> = entries.iter()
            .map(|e| core::str::from_utf8(&e.name).unwrap_or("").trim_end_matches(char::from(0)))
            .collect();
        assert!(names.contains(&"file1.txt"));
        assert!(names.contains(&"file2.txt"));
        assert!(names.contains(&"subdir"));
    }

    #[test]
    fn fs_delete_inode() {
        let mut fs = Shardevfs::new();
        unsafe { fs.init(); }

        fs.create_inode(1, "temp.txt", VfsNodeType::RegularFile).unwrap();
        assert!(fs.lookup_inode(1, "temp.txt").is_ok());

        fs.delete_inode(1, "temp.txt").unwrap();
        assert_eq!(fs.lookup_inode(1, "temp.txt"), Err(VfsError::NotFound));
    }

    #[test]
    fn fs_get_stat() {
        let mut fs = Shardevfs::new();
        unsafe { fs.init(); }

        let inode_id = fs.create_inode(1, "stat_test.txt", VfsNodeType::RegularFile).unwrap();
        let stat = fs.get_stat(inode_id).unwrap();
        assert_eq!(stat.inode_id, inode_id);
        assert_eq!(stat.node_type, VfsNodeType::RegularFile);
        assert_eq!(stat.size, 0);
    }

    #[test]
    fn fs_not_initialized() {
        let mut fs = Shardevfs::new();
        let result = fs.create_inode(1, "test.txt", VfsNodeType::RegularFile);
        assert_eq!(result, Err(VfsError::NotInitialized));
    }
}
