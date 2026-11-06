//! Usuarios y grupos — Sistema de autenticación y permisos de shardevos.
//!
//! Proporciona:
//! - Gestión de usuarios (UID, nombre, contraseña hash, shell, directorio home)
//! - Gestión de grupos (GID, nombre, miembros)
//! - Archivos /etc/passwd y /etc/group
//! - Autenticación de usuarios (login por contraseña)
//! - Contexto de seguridad por proceso (UID/GID efectivo, grupos suplementarios)

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

/// Identificador de usuario.
pub type Uid = u32;
/// Identificador de grupo.
pub type Gid = u32;

/// UID del usuario root.
pub const ROOT_UID: Uid = 0;
/// GID del grupo root.
pub const ROOT_GID: Gid = 0;

/// UID para usuarios no autenticados (nobody).
pub const NOBODY_UID: Uid = 65534;
/// GID para el grupo nobody.
pub const NOBODY_GID: Gid = 65534;

/// Entrada de un usuario (análoga a /etc/passwd).
#[derive(Debug, Clone)]
pub struct PasswdEntry {
    pub username: String,
    pub password_hash: String,  // Hash SHA-256 de la contraseña
    pub uid: Uid,
    pub gid: Gid,
    pub gecos: String,          // Información del usuario (nombre completo, etc.)
    pub home_dir: String,
    pub shell: String,
}

/// Entrada de un grupo (análoga a /etc/group).
#[derive(Debug, Clone)]
pub struct GroupEntry {
    pub group_name: String,
    pub password: String,
    pub gid: Gid,
    pub members: Vec<String>,
}

/// Resultado de operaciones de seguridad.
pub type SecurityResult<T> = Result<T, SecurityError>;

/// Errores de seguridad y autenticación.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityError {
    UserNotFound,
    GroupNotFound,
    InvalidPassword,
    UserAlreadyExists,
    GroupAlreadyExists,
    PermissionDenied,
    InvalidArgument,
    NotAuthenticated,
    TooManyUsers,
    TooManyGroups,
}

/// Contexto de seguridad de un proceso.
/// Cada proceso tiene un UID/GID efectivo y grupos suplementarios.
#[derive(Debug, Clone, Copy)]
pub struct SecurityContext {
    pub uid: Uid,
    pub gid: Gid,
    pub euid: Uid,       // UID efectivo (para setuid)
    pub egid: Gid,       // GID efectivo
    pub supplementary_gids: [Gid; 16],
    pub supplementary_count: usize,
    pub authenticated: bool,
    pub session_id: u64,
}

impl SecurityContext {
    /// Crea un contexto de seguridad para un usuario no autenticado (nobody).
    pub fn new_unauthenticated() -> Self {
        Self {
            uid: NOBODY_UID,
            gid: NOBODY_GID,
            euid: NOBODY_UID,
            egid: NOBODY_GID,
            supplementary_gids: [NOBODY_GID; 16],
            supplementary_count: 0,
            authenticated: false,
            session_id: 0,
        }
    }

    /// Crea un contexto de seguridad para el usuario root.
    pub fn new_root() -> Self {
        Self {
            uid: ROOT_UID,
            gid: ROOT_GID,
            euid: ROOT_UID,
            egid: ROOT_GID,
            supplementary_gids: [ROOT_GID; 16],
            supplementary_count: 1,
            authenticated: true,
            session_id: 1,
        }
    }

    /// Verifica si el contexto tiene privilegios de root.
    pub fn is_root(&self) -> bool {
        self.euid == ROOT_UID
    }

    /// Verifica si el contexto pertenece a un grupo dado.
    pub fn in_group(&self, gid: Gid) -> bool {
        if self.egid == gid {
            return true;
        }
        for i in 0..self.supplementary_count {
            if self.supplementary_gids[i] == gid {
                return true;
            }
        }
        false
    }
}

/// Gestor de usuarios y grupos.
pub struct UserManager {
    users: spin_lock_security::SpinLock<Vec<PasswdEntry>>,
    groups: spin_lock_security::SpinLock<Vec<GroupEntry>>,
    next_uid: AtomicU64,
    next_gid: AtomicU64,
    initialized: AtomicU64,
}

impl UserManager {
    pub const fn new() -> Self {
        Self {
            users: spin_lock_security::SpinLock::new(Vec::new()),
            groups: spin_lock_security::SpinLock::new(Vec::new()),
            next_uid: AtomicU64::new(1000),
            next_gid: AtomicU64::new(1000),
            initialized: AtomicU64::new(0),
        }
    }

    pub unsafe fn init(&self) {
        self.initialized.store(1, Ordering::Release);

        // Crear usuario root
        let _ = self.add_user(PasswdEntry {
            username: "root".to_string(),
            password_hash: "".to_string(),  // Root sin contraseña por defecto (inseguro, solo demo)
            uid: ROOT_UID,
            gid: ROOT_GID,
            gecos: "Administrador del sistema".to_string(),
            home_dir: "/root".to_string(),
            shell: "/bin/sh".to_string(),
        });

        // Crear grupo root
        let _ = self.add_group(GroupEntry {
            group_name: "root".to_string(),
            password: "".to_string(),
            gid: ROOT_GID,
            members: vec!["root".to_string()],
        });

        // Crear usuario nobody
        let _ = self.add_user(PasswdEntry {
            username: "nobody".to_string(),
            password_hash: "".to_string(),
            uid: NOBODY_UID,
            gid: NOBODY_GID,
            gecos: "Usuario no autenticado".to_string(),
            home_dir: "/nonexistent".to_string(),
            shell: "/bin/false".to_string(),
        });

        // Crear grupo nobody
        let _ = self.add_group(GroupEntry {
            group_name: "nobody".to_string(),
            password: "".to_string(),
            gid: NOBODY_GID,
            members: vec!["nobody".to_string()],
        });
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::Acquire) != 0
    }

    /// Agrega un usuario al sistema.
    pub fn add_user(&self, entry: PasswdEntry) -> SecurityResult<Uid> {
        if !self.is_initialized() {
            return Err(SecurityError::InvalidArgument);
        }

        let users = self.users.lock();
        // Verificar que no exista ya
        if users.iter().any(|u| u.username == entry.username) {
            return Err(SecurityError::UserAlreadyExists);
        }
        drop(users);

        let uid = self.next_uid.fetch_add(1, Ordering::Relaxed) as Uid;
        let mut entry = entry;
        entry.uid = uid;

        self.users.lock().push(entry);
        Ok(uid)
    }

    /// Agrega un grupo al sistema.
    pub fn add_group(&self, entry: GroupEntry) -> SecurityResult<Gid> {
        if !self.is_initialized() {
            return Err(SecurityError::InvalidArgument);
        }

        let groups = self.groups.lock();
        if groups.iter().any(|g| g.group_name == entry.group_name) {
            return Err(SecurityError::GroupAlreadyExists);
        }
        drop(groups);

        let gid = self.next_gid.fetch_add(1, Ordering::Relaxed) as Gid;
        let mut entry = entry;
        entry.gid = gid;

        self.groups.lock().push(entry);
        Ok(gid)
    }

    /// Busca un usuario por nombre.
    pub fn get_user_by_name(&self, username: &str) -> Option<PasswdEntry> {
        let users = self.users.lock();
        users.iter().find(|u| u.username == username).cloned()
    }

    /// Busca un usuario por UID.
    pub fn get_user_by_uid(&self, uid: Uid) -> Option<PasswdEntry> {
        let users = self.users.lock();
        users.iter().find(|u| u.uid == uid).cloned()
    }

    /// Busca un grupo por nombre.
    pub fn get_group_by_name(&self, group_name: &str) -> Option<GroupEntry> {
        let groups = self.groups.lock();
        groups.iter().find(|g| g.group_name == group_name).cloned()
    }

    /// Busca un grupo por GID.
    pub fn get_group_by_gid(&self, gid: Gid) -> Option<GroupEntry> {
        let groups = self.groups.lock();
        groups.iter().find(|g| g.gid == gid).cloned()
    }

    /// Autentica un usuario (verifica contraseña).
    /// En esta implementación demo, la contraseña se compara directamente con el hash almacenado.
    pub fn authenticate(&self, username: &str, password: &str) -> SecurityResult<(Uid, Gid)> {
        if !self.is_initialized() {
            return Err(SecurityError::InvalidArgument);
        }

        let user = self.get_user_by_name(username).ok_or(SecurityError::UserNotFound)?;

        // Verificación simple: comparar contraseña (en producción usar hash real)
        if user.password_hash.is_empty() || user.password_hash == password {
            Ok((user.uid, user.gid))
        } else {
            Err(SecurityError::InvalidPassword)
        }
    }

    /// Obtiene el número total de usuarios.
    pub fn user_count(&self) -> usize {
        self.users.lock().len()
    }

    /// Obtiene el número total de grupos.
    pub fn group_count(&self) -> usize {
        self.groups.lock().len()
    }
}

/// SpinLock para el gestor de seguridad.
mod spin_lock_security {
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

/// Instancia global del gestor de usuarios.
static mut USER_MANAGER: UserManager = UserManager::new();

/// Devuelve una referencia mutable al gestor de usuarios global.
pub fn global_user_manager() -> &'static mut UserManager {
    unsafe { &mut USER_MANAGER }
}

/// Inicializa el gestor de usuarios.
pub unsafe fn init_user_manager() {
    USER_MANAGER.init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_manager_init() {
        let um = UserManager::new();
        assert!(!um.is_initialized());
        unsafe { um.init(); }
        assert!(um.is_initialized());
    }

    #[test]
    fn test_add_and_get_user() {
        let um = UserManager::new();
        unsafe { um.init(); }

        let uid = um.add_user(PasswdEntry {
            username: "testuser".to_string(),
            password_hash: "testpass".to_string(),
            uid: 1000,
            gid: 1000,
            gecos: "Usuario de prueba".to_string(),
            home_dir: "/home/testuser".to_string(),
            shell: "/bin/sh".to_string(),
        }).unwrap();

        assert_eq!(uid, 1000);

        let user = um.get_user_by_name("testuser").unwrap();
        assert_eq!(user.uid, 1000);
        assert_eq!(user.username, "testuser");

        let user_by_uid = um.get_user_by_uid(1000).unwrap();
        assert_eq!(user_by_uid.username, "testuser");
    }

    #[test]
    fn test_add_and_get_group() {
        let um = UserManager::new();
        unsafe { um.init(); }

        let gid = um.add_group(GroupEntry {
            group_name: "testgroup".to_string(),
            password: "".to_string(),
            gid: 2000,
            members: vec!["user1".to_string(), "user2".to_string()],
        }).unwrap();

        assert_eq!(gid, 2000);

        let group = um.get_group_by_name("testgroup").unwrap();
        assert_eq!(group.gid, 2000);
        assert_eq!(group.members.len(), 2);
    }

    #[test]
    fn test_authenticate() {
        let um = UserManager::new();
        unsafe { um.init(); }

        let _ = um.add_user(PasswdEntry {
            username: "alice".to_string(),
            password_hash: "secret123".to_string(),
            uid: 1001,
            gid: 1001,
            gecos: "Alice".to_string(),
            home_dir: "/home/alice".to_string(),
            shell: "/bin/sh".to_string(),
        });

        // Contraseña correcta
        let result = um.authenticate("alice", "secret123");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), (1001, 1001));

        // Contraseña incorrecta
        let result = um.authenticate("alice", "wrongpass");
        assert_eq!(result, Err(SecurityError::InvalidPassword));

        // Usuario no existe
        let result = um.authenticate("bob", "pass");
        assert_eq!(result, Err(SecurityError::UserNotFound));
    }

    #[test]
    fn test_security_context() {
        let ctx = SecurityContext::new_root();
        assert!(ctx.is_root());
        assert!(ctx.authenticated);
        assert_eq!(ctx.uid, ROOT_UID);

        let ctx2 = SecurityContext::new_unauthenticated();
        assert!(!ctx2.is_root());
        assert!(!ctx2.authenticated);
        assert_eq!(ctx2.uid, NOBODY_UID);
    }

    #[test]
    fn test_user_already_exists() {
        let um = UserManager::new();
        unsafe { um.init(); }

        let _ = um.add_user(PasswdEntry {
            username: "dup".to_string(),
            password_hash: "p".to_string(),
            uid: 5000,
            gid: 5000,
            gecos: "Dup".to_string(),
            home_dir: "/home/dup".to_string(),
            shell: "/bin/sh".to_string(),
        });

        let result = um.add_user(PasswdEntry {
            username: "dup".to_string(),
            password_hash: "p2".to_string(),
            uid: 5001,
            gid: 5001,
            gecos: "Dup2".to_string(),
            home_dir: "/home/dup2".to_string(),
            shell: "/bin/sh".to_string(),
        });
        assert_eq!(result, Err(SecurityError::UserAlreadyExists));
    }
}
