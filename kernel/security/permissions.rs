//! Permisos de archivos — Verificación de acceso rwx estilo Unix.
//!
//! Proporciona:
//! - Modos de permiso (lectura, escritura, ejecución) para propietario/grupo/otros
//! - Verificación de acceso basada en UID/GID del proceso y propietario del archivo
//! - Soporte para bit setuid/setgid/sticky

use crate::security::users::{SecurityContext, ROOT_UID};

/// Bits de permiso de archivo (estilo Unix rwx).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileMode {
    pub bits: u16,
}

impl FileMode {
    pub const fn from_bits(bits: u16) -> Self {
        Self { bits }
    }

    /// Propietario puede leer.
    pub const fn owner_read(&self) -> bool {
        (self.bits & 0o400) != 0
    }

    /// Propietario puede escribir.
    pub const fn owner_write(&self) -> bool {
        (self.bits & 0o200) != 0
    }

    /// Propietario puede ejecutar.
    pub const fn owner_execute(&self) -> bool {
        (self.bits & 0o100) != 0
    }

    /// Grupo puede leer.
    pub const fn group_read(&self) -> bool {
        (self.bits & 0o040) != 0
    }

    /// Grupo puede escribir.
    pub const fn group_write(&self) -> bool {
        (self.bits & 0o020) != 0
    }

    /// Grupo puede ejecutar.
    pub const fn group_execute(&self) -> bool {
        (self.bits & 0o010) != 0
    }

    /// Otros pueden leer.
    pub const fn other_read(&self) -> bool {
        (self.bits & 0o004) != 0
    }

    /// Otros pueden escribir.
    pub const fn other_write(&self) -> bool {
        (self.bits & 0o002) != 0
    }

    /// Otros pueden ejecutar.
    pub const fn other_execute(&self) -> bool {
        (self.bits & 0o001) != 0
    }

    /// Bit setuid.
    pub const fn setuid(&self) -> bool {
        (self.bits & 0o4000) != 0
    }

    /// Bit setgid.
    pub const fn setgid(&self) -> bool {
        (self.bits & 0o2000) != 0
    }

    /// Bit sticky.
    pub const fn sticky(&self) -> bool {
        (self.bits & 0o1000) != 0
    }

    /// Crea un modo con permisos por defecto (0o644 para archivos, 0o755 para directorios).
    pub const fn default_file() -> Self {
        Self { bits: 0o644 }
    }

    pub const fn default_directory() -> Self {
        Self { bits: 0o755 }
    }

    /// Crea un modo desde componentes individuales.
    pub const fn from_components(
        owner_r: bool, owner_w: bool, owner_x: bool,
        group_r: bool, group_w: bool, group_x: bool,
        other_r: bool, other_w: bool, other_x: bool,
    ) -> Self {
        let mut bits: u16 = 0;
        if owner_r { bits |= 0o400; }
        if owner_w { bits |= 0o200; }
        if owner_x { bits |= 0o100; }
        if group_r { bits |= 0o040; }
        if group_w { bits |= 0o020; }
        if group_x { bits |= 0o010; }
        if other_r { bits |= 0o004; }
        if other_w { bits |= 0o002; }
        if other_x { bits |= 0o001; }
        Self { bits }
    }
}

/// Tipo de acceso solicitado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessType {
    Read,
    Write,
    Execute,
}

/// Resultado de verificación de permisos.
pub type PermissionResult = Result<(), PermissionError>;

/// Errores de permisos.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionError {
    PermissionDenied,
    InvalidMode,
    OwnerMismatch,
}

/// Verificador de permisos.
pub struct PermissionChecker;

impl PermissionChecker {
    /// Verifica si un contexto de seguridad puede acceder a un archivo con el modo dado.
    ///
    /// Reglas:
    /// 1. Root (euid == 0) siempre tiene acceso.
    /// 2. Si el UID del proceso coincide con el propietario del archivo, usar permisos de propietario.
    /// 3. Si el GID del proceso (o grupos suplementarios) coincide con el grupo del archivo, usar permisos de grupo.
    /// 4. De lo contrario, usar permisos de otros.
    ///
    /// Para setuid: si el archivo tiene bit setuid, el proceso se ejecuta como el propietario del archivo.
    pub fn check_access(
        ctx: &SecurityContext,
        file_owner_uid: u32,
        file_owner_gid: u32,
        file_mode: FileMode,
        access: AccessType,
    ) -> PermissionResult {
        // Root tiene acceso a todo
        if ctx.is_root() {
            return Ok(());
        }

        // Determinar qué conjunto de permisos usar
        let (can_read, can_write, can_execute) = if ctx.uid == file_owner_uid {
            // Permisos de propietario
            (
                file_mode.owner_read(),
                file_mode.owner_write(),
                file_mode.owner_execute(),
            )
        } else if ctx.in_group(file_owner_gid) {
            // Permisos de grupo
            (
                file_mode.group_read(),
                file_mode.group_write(),
                file_mode.group_execute(),
            )
        } else {
            // Permisos de otros
            (
                file_mode.other_read(),
                file_mode.other_write(),
                file_mode.other_execute(),
            )
        };

        match access {
            AccessType::Read => {
                if can_read {
                    Ok(())
                } else {
                    Err(PermissionError::PermissionDenied)
                }
            }
            AccessType::Write => {
                if can_write {
                    Ok(())
                } else {
                    Err(PermissionError::PermissionDenied)
                }
            }
            AccessType::Execute => {
                if can_execute {
                    Ok(())
                } else {
                    Err(PermissionError::PermissionDenied)
                }
            }
        }
    }

    /// Verifica si un usuario puede crear un archivo en un directorio.
    /// Requiere permiso de escritura en el directorio.
    /// Para el bit sticky: solo el propietario del archivo, el propietario del directorio o root pueden eliminar/renombrar.
    pub fn can_create_in_directory(
        ctx: &SecurityContext,
        dir_owner_uid: u32,
        dir_owner_gid: u32,
        dir_mode: FileMode,
    ) -> PermissionResult {
        Self::check_access(ctx, dir_owner_uid, dir_owner_gid, dir_mode, AccessType::Write)
    }

    /// Verifica si un usuario puede eliminar un archivo de un directorio (considerando sticky bit).
    pub fn can_delete_from_directory(
        ctx: &SecurityContext,
        file_owner_uid: u32,
        dir_owner_uid: u32,
        dir_owner_gid: u32,
        dir_mode: FileMode,
    ) -> PermissionResult {
        // Root puede eliminar todo
        if ctx.is_root() {
            return Ok(());
        }

        // Si el bit sticky está activo, solo el propietario del archivo, el propietario del directorio o root puede eliminar
        if dir_mode.sticky() {
            if ctx.uid == file_owner_uid {
                return Ok(());
            }
            if ctx.uid == dir_owner_uid {
                return Ok(());
            }
            return Err(PermissionError::PermissionDenied);
        }

        // Sin sticky: necesita escritura en el directorio
        Self::check_access(ctx, dir_owner_uid, dir_owner_gid, dir_mode, AccessType::Write)
    }

    /// Verifica si un contexto puede cambiar el propietario de un archivo (chown).
    /// Solo root puede hacer chown.
    pub fn can_chown(ctx: &SecurityContext) -> PermissionResult {
        if ctx.is_root() {
            Ok(())
        } else {
            Err(PermissionError::PermissionDenied)
        }
    }

    /// Verifica si un contexto puede cambiar los permisos de un archivo (chmod).
    /// Solo root o el propietario del archivo puede hacer chmod.
    pub fn can_chmod(ctx: &SecurityContext, file_owner_uid: u32) -> PermissionResult {
        if ctx.is_root() || ctx.uid == file_owner_uid {
            Ok(())
        } else {
            Err(PermissionError::PermissionDenied)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_mode_bits() {
        let mode = FileMode::from_bits(0o755);
        assert!(mode.owner_read());
        assert!(mode.owner_write());
        assert!(mode.owner_execute());
        assert!(mode.group_read());
        assert!(!mode.group_write());
        assert!(mode.group_execute());
        assert!(mode.other_read());
        assert!(!mode.other_write());
        assert!(mode.other_execute());
    }

    #[test]
    fn test_file_mode_setuid_setgid_sticky() {
        let mode = FileMode::from_bits(0o6755);
        assert!(mode.setuid());
        assert!(mode.setgid());
        assert!(mode.sticky());
    }

    #[test]
    fn test_root_access() {
        let ctx = super::users::SecurityContext::new_root();
        let mode = FileMode::from_bits(0o000); // Sin permisos para nadie

        // Root puede leer aunque no haya permisos
        assert!(PermissionChecker::check_access(&ctx, 1000, 1000, mode, AccessType::Read).is_ok());
        assert!(PermissionChecker::check_access(&ctx, 1000, 1000, mode, AccessType::Write).is_ok());
        assert!(PermissionChecker::check_access(&ctx, 1000, 1000, mode, AccessType::Execute).is_ok());
    }

    #[test]
    fn test_owner_access() {
        let ctx = super::users::SecurityContext {
            uid: 1000,
            gid: 1000,
            euid: 1000,
            egid: 1000,
            supplementary_gids: [0; 16],
            supplementary_count: 0,
            authenticated: true,
            session_id: 1,
        };

        // Propietario con permisos de lectura
        let mode = FileMode::from_bits(0o400);
        assert!(PermissionChecker::check_access(&ctx, 1000, 2000, mode, AccessType::Read).is_ok());
        assert!(PermissionChecker::check_access(&ctx, 1000, 2000, mode, AccessType::Write).is_err());

        // Propietario sin permisos
        let mode = FileMode::from_bits(0o000);
        assert!(PermissionChecker::check_access(&ctx, 1000, 2000, mode, AccessType::Read).is_err());
    }

    #[test]
    fn test_group_access() {
        let ctx = super::users::SecurityContext {
            uid: 2000,
            gid: 1000,
            euid: 2000,
            egid: 1000,
            supplementary_gids: [0; 16],
            supplementary_count: 0,
            authenticated: true,
            session_id: 1,
        };

        // No es propietario pero está en el grupo
        let mode = FileMode::from_bits(0o040);
        assert!(PermissionChecker::check_access(&ctx, 1000, 1000, mode, AccessType::Read).is_ok());
        assert!(PermissionChecker::check_access(&ctx, 1000, 1000, mode, AccessType::Write).is_err());
    }

    #[test]
    fn test_other_access() {
        let ctx = super::users::SecurityContext {
            uid: 3000,
            gid: 3000,
            euid: 3000,
            egid: 3000,
            supplementary_gids: [0; 16],
            supplementary_count: 0,
            authenticated: true,
            session_id: 1,
        };

        // No es propietario ni está en el grupo, permisos de otros
        let mode = FileMode::from_bits(0o004);
        assert!(PermissionChecker::check_access(&ctx, 1000, 1000, mode, AccessType::Read).is_ok());
        assert!(PermissionChecker::check_access(&ctx, 1000, 1000, mode, AccessType::Write).is_err());
    }

    #[test]
    fn test_sticky_bit_delete() {
        let ctx = super::users::SecurityContext {
            uid: 2000,
            gid: 2000,
            euid: 2000,
            egid: 2000,
            supplementary_gids: [0; 16],
            supplementary_count: 0,
            authenticated: true,
            session_id: 1,
        };

        // Directorio con sticky bit, ctx no es propietario del archivo ni del directorio
        let dir_mode = FileMode::from_bits(0o1777); // sticky + todos los permisos
        let result = PermissionChecker::can_delete_from_directory(&ctx, 1000, 1000, 1000, dir_mode);
        assert_eq!(result, Err(PermissionError::PermissionDenied));

        // ctx es propietario del archivo
        let result = PermissionChecker::can_delete_from_directory(&ctx, 2000, 1000, 1000, dir_mode);
        assert!(result.is_ok());
    }

    #[test]
    fn test_can_chown_only_root() {
        let root_ctx = super::users::SecurityContext::new_root();
        assert!(PermissionChecker::can_chown(&root_ctx).is_ok());

        let user_ctx = super::users::SecurityContext {
            uid: 1000,
            gid: 1000,
            euid: 1000,
            egid: 1000,
            supplementary_gids: [0; 16],
            supplementary_count: 0,
            authenticated: true,
            session_id: 1,
        };
        assert_eq!(PermissionChecker::can_chown(&user_ctx), Err(PermissionError::PermissionDenied));
    }

    #[test]
    fn test_can_chmod_owner_or_root() {
        let root_ctx = super::users::SecurityContext::new_root();
        assert!(PermissionChecker::can_chmod(&root_ctx, 1000).is_ok());

        let owner_ctx = super::users::SecurityContext {
            uid: 1000,
            gid: 1000,
            euid: 1000,
            egid: 1000,
            supplementary_gids: [0; 16],
            supplementary_count: 0,
            authenticated: true,
            session_id: 1,
        };
        assert!(PermissionChecker::can_chmod(&owner_ctx, 1000).is_ok());

        let other_ctx = super::users::SecurityContext {
            uid: 2000,
            gid: 2000,
            euid: 2000,
            egid: 2000,
            supplementary_gids: [0; 16],
            supplementary_count: 0,
            authenticated: true,
            session_id: 1,
        };
        assert_eq!(PermissionChecker::can_chmod(&other_ctx, 1000), Err(PermissionError::PermissionDenied));
    }
}
