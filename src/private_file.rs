//! Fichiers de configuration privés : dossier XDG, écriture atomique 0600.

use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

/// `$XDG_CONFIG_HOME/leeloo` (sinon `~/.config/leeloo`).
pub fn config_dir() -> Option<PathBuf> {
    let config_home = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        _ => PathBuf::from(std::env::var_os("HOME")?).join(".config"),
    };
    Some(config_home.join("leeloo"))
}

/// Ancien nom du dossier de configuration (avant le renommage en Leeloo).
const LEGACY_DIR_NAME: &str = "moba-linux";

#[derive(Debug, PartialEq, Eq)]
pub enum Migration {
    /// Rien à reprendre (pas d'ancien dossier, ou le nouveau existe déjà).
    NotNeeded,
    Migrated,
}

/// Reprend l'ancien dossier de configuration : renommage atomique, qui
/// conserve contenu et droits. Ne fait rien si le nouveau dossier existe déjà,
/// pour ne jamais écraser de données.
pub fn migrate_dir(old: &Path, new: &Path) -> std::io::Result<Migration> {
    if new.exists() || !old.is_dir() {
        return Ok(Migration::NotNeeded);
    }
    std::fs::rename(old, new)?;
    Ok(Migration::Migrated)
}

/// À appeler une fois au démarrage, avant tout chargement de configuration.
pub fn migrate_legacy_config_dir() -> std::io::Result<Migration> {
    let Some(new) = config_dir() else {
        return Ok(Migration::NotNeeded);
    };
    match new.parent() {
        Some(parent) => migrate_dir(&parent.join(LEGACY_DIR_NAME), &new),
        None => Ok(Migration::NotNeeded),
    }
}

/// Écriture atomique (fichier temporaire + rename), droits 0600, dossier 0700.
pub fn write_atomic(path: &Path, content: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)?;
    }
    let mut temporary_name = path.as_os_str().to_owned();
    temporary_name.push(".tmp");
    let temporary_path = PathBuf::from(temporary_name);
    let mut temporary_file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temporary_path)?;
    temporary_file.write_all(content)?;
    temporary_file.sync_all()?;
    std::fs::rename(&temporary_path, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn scratch(label: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("leeloo-private-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[test]
    fn legacy_directory_is_moved_with_content_and_permissions() {
        let root = scratch("migrate");
        let old = root.join("moba-linux");
        let new = root.join("leeloo");
        write_atomic(&old.join("sessions.json"), b"{}").unwrap();

        assert_eq!(migrate_dir(&old, &new).unwrap(), Migration::Migrated);
        assert!(!old.exists());
        assert_eq!(std::fs::read(new.join("sessions.json")).unwrap(), b"{}");
        let mode = std::fs::metadata(new.join("sessions.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn existing_new_directory_is_never_overwritten() {
        let root = scratch("keep");
        let old = root.join("moba-linux");
        let new = root.join("leeloo");
        write_atomic(&old.join("sessions.json"), b"old").unwrap();
        write_atomic(&new.join("sessions.json"), b"new").unwrap();

        assert_eq!(migrate_dir(&old, &new).unwrap(), Migration::NotNeeded);
        assert_eq!(std::fs::read(new.join("sessions.json")).unwrap(), b"new");
        assert!(old.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn nothing_to_migrate_without_a_legacy_directory() {
        let root = scratch("none");
        assert_eq!(
            migrate_dir(&root.join("moba-linux"), &root.join("leeloo")).unwrap(),
            Migration::NotNeeded
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
