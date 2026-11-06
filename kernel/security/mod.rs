//! Security — Sistema de autenticación y permisos de shardevos.
//!
//! Proporciona:
//! - Gestión de usuarios y grupos (/etc/passwd, /etc/group)
//! - Autenticación por contraseña
//! - Verificación de permisos rwx estilo Unix
//! - Contexto de seguridad por proceso

pub mod users;
pub mod permissions;

pub use self::users::{
    UserManager, PasswdEntry, GroupEntry, SecurityContext, SecurityError,
    ROOT_UID, ROOT_GID, NOBODY_UID, NOBODY_GID,
    global_user_manager, init_user_manager,
};
pub use self::permissions::{
    FileMode, AccessType, PermissionChecker, PermissionError,
    PermissionResult,
};
