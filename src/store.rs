//! Stockage des sessions : `$XDG_CONFIG_HOME/leeloo/sessions.json` (0600).

use crate::private_file;
use crate::session::{AzureVmSession, Session, SessionKind, SshSession, ValidationError};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Path, PathBuf};

const FORMAT_VERSION: u32 = 1;

#[derive(Debug)]
pub enum StoreError {
    Io(std::io::Error),
    Parse(serde_json::Error),
    UnsupportedVersion(u32),
}

impl fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "erreur d'accès au fichier : {error}"),
            Self::Parse(error) => write!(formatter, "fichier de sessions invalide : {error}"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "version de format non supportée : {version}")
            }
        }
    }
}

impl std::error::Error for StoreError {}

impl From<std::io::Error> for StoreError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Serialize, Deserialize)]
struct FileFormat {
    version: u32,
    sessions: Vec<Session>,
    /// Dossiers explicites (y compris vides). Absent des anciens fichiers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    folders: Vec<String>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SessionStore {
    sessions: Vec<Session>,
    /// Dossiers créés explicitement ; les dossiers de sessions sont déduits.
    folders: Vec<String>,
}

/// Séparateur de niveaux dans un chemin de dossier (« Azure/Dev »).
pub const FOLDER_SEPARATOR: char = '/';
const MAX_FOLDER_DEPTH: usize = 6;
const MAX_FOLDER_NAME_CHARS: usize = 64;
/// Ancien nom du dossier d'import, qui contenait des `/` : il devenait
/// une arborescence « ~ › .ssh › config ».
const LEGACY_IMPORT_FOLDER: &str = "~/.ssh/config";
pub const IMPORT_FOLDER: &str = "SSH config";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FolderError {
    Empty,
    TooDeep,
    NameTooLong,
    InvalidName,
    AlreadyExists,
    NotFound,
    /// Déplacement d'un dossier dans lui-même ou dans l'un de ses sous-dossiers.
    InvalidMove,
}

impl fmt::Display for FolderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "le nom du dossier est vide",
            Self::TooDeep => "trop de niveaux de dossiers",
            Self::NameTooLong => "nom de dossier trop long",
            Self::InvalidName => "nom de dossier invalide",
            Self::AlreadyExists => "ce dossier existe déjà",
            Self::NotFound => "dossier introuvable",
            Self::InvalidMove => "un dossier ne peut pas être placé dans lui-même",
        })
    }
}

impl std::error::Error for FolderError {}

/// Parent d'un chemin (`None` pour un dossier de premier niveau).
pub fn parent_folder(path: &str) -> Option<&str> {
    path.rsplit_once(FOLDER_SEPARATOR).map(|(parent, _)| parent)
}

/// Dernier segment d'un chemin.
pub fn folder_name(path: &str) -> &str {
    path.rsplit_once(FOLDER_SEPARATOR)
        .map_or(path, |(_, name)| name)
}

/// Vrai si `path` est `ancestor` ou se trouve dessous.
fn is_within(path: &str, ancestor: &str) -> bool {
    path == ancestor
        || path
            .strip_prefix(ancestor)
            .is_some_and(|rest| rest.starts_with(FOLDER_SEPARATOR))
}

/// Remplace le préfixe de chemin `old` par `new` (`path` est `old` ou dessous).
fn replace_prefix(path: &str, old: &str, new: &str) -> String {
    format!("{new}{}", &path[old.len()..])
}

/// Valide les segments d'un nom de dossier (longueur, caractères).
fn validate_segments(segments: &[&str]) -> Result<(), FolderError> {
    for segment in segments {
        if segment.chars().count() > MAX_FOLDER_NAME_CHARS {
            return Err(FolderError::NameTooLong);
        }
        if segment.chars().any(char::is_control) || *segment == "." || *segment == ".." {
            return Err(FolderError::InvalidName);
        }
    }
    Ok(())
}

impl SessionStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn default_path() -> Option<PathBuf> {
        private_file::config_dir().map(|directory| directory.join("sessions.json"))
    }

    /// Fichier absent = magasin vide.
    pub fn load(path: &Path) -> Result<Self, StoreError> {
        let content = match std::fs::read_to_string(path) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Self::new()),
            Err(error) => return Err(error.into()),
        };
        let file: FileFormat = serde_json::from_str(&content).map_err(StoreError::Parse)?;
        if file.version != FORMAT_VERSION {
            return Err(StoreError::UnsupportedVersion(file.version));
        }
        let mut store = Self {
            sessions: file.sessions,
            folders: file.folders,
        };
        store.migrate_legacy_import_folder();
        Ok(store)
    }

    fn migrate_legacy_import_folder(&mut self) {
        let rename = |folder: &mut String| {
            if is_within(folder, LEGACY_IMPORT_FOLDER) {
                *folder = folder.replacen(LEGACY_IMPORT_FOLDER, IMPORT_FOLDER, 1);
            }
        };
        for session in &mut self.sessions {
            if let Some(folder) = session.folder.as_mut() {
                rename(folder);
            }
        }
        self.folders.iter_mut().for_each(rename);
    }

    /// Écriture atomique, droits 0600, dossier 0700.
    pub fn save(&self, path: &Path) -> Result<(), StoreError> {
        let file = FileFormat {
            version: FORMAT_VERSION,
            sessions: self.sessions.clone(),
            folders: self.folders.clone(),
        };
        let json = serde_json::to_string_pretty(&file).map_err(StoreError::Parse)?;
        private_file::write_atomic(path, json.as_bytes())?;
        Ok(())
    }

    pub fn sessions(&self) -> &[Session] {
        &self.sessions
    }

    pub fn get(&self, id: u64) -> Option<&Session> {
        self.sessions.iter().find(|session| session.id == id)
    }

    fn next_id(&self) -> u64 {
        self.sessions
            .iter()
            .map(|s| s.id)
            .max()
            .map_or(1, |max| max + 1)
    }

    pub fn add(
        &mut self,
        name: &str,
        folder: Option<&str>,
        kind: SessionKind,
    ) -> Result<u64, ValidationError> {
        self.add_session(Session {
            id: 0,
            name: name.to_owned(),
            folder: folder.map(str::to_owned),
            color: None,
            palette: None,
            kind,
        })
    }

    /// Ajoute une session complète (couleur et palette comprises) : l'identifiant
    /// est attribué par le magasin, nom et dossier sont normalisés.
    pub fn add_session(&mut self, mut session: Session) -> Result<u64, ValidationError> {
        session.id = self.next_id();
        session.name = session.name.trim().to_owned();
        session.folder = normalize_folder(session.folder.as_deref());
        session.validate()?;
        let id = session.id;
        self.sessions.push(session);
        Ok(id)
    }

    /// Ajoute une VM Azure ; `Ok(None)` si la même VM (subscription, groupe,
    /// nom, sans tenir compte de la casse) est déjà enregistrée.
    pub fn add_azure_vm(
        &mut self,
        name: &str,
        folder: Option<&str>,
        vm: AzureVmSession,
    ) -> Result<Option<u64>, ValidationError> {
        let same = |known: &AzureVmSession| {
            known.vm_name.eq_ignore_ascii_case(&vm.vm_name)
                && known
                    .resource_group
                    .eq_ignore_ascii_case(&vm.resource_group)
                && known
                    .subscription
                    .as_deref()
                    .unwrap_or("")
                    .eq_ignore_ascii_case(vm.subscription.as_deref().unwrap_or(""))
        };
        let already_known = self
            .sessions
            .iter()
            .any(|session| matches!(&session.kind, SessionKind::AzureVm(known) if same(known)));
        if already_known {
            return Ok(None);
        }
        self.add(name, folder, SessionKind::AzureVm(vm)).map(Some)
    }

    /// Remplace la session de même `id`. `Ok(false)` si elle n'existe pas.
    pub fn update(&mut self, mut session: Session) -> Result<bool, ValidationError> {
        session.name = session.name.trim().to_owned();
        session.folder = normalize_folder(session.folder.as_deref());
        session.validate()?;
        match self.sessions.iter_mut().find(|s| s.id == session.id) {
            Some(existing) => {
                *existing = session;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    pub fn remove(&mut self, id: u64) -> bool {
        let before = self.sessions.len();
        self.sessions.retain(|session| session.id != id);
        self.sessions.len() != before
    }

    /// Tous les dossiers (explicites, de sessions et leurs ancêtres), sans
    /// doublon, triés sans tenir compte de la casse.
    pub fn folders(&self) -> Vec<String> {
        let mut all: Vec<String> = Vec::new();
        let declared = self
            .folders
            .iter()
            .cloned()
            .chain(self.sessions.iter().filter_map(|s| s.folder.clone()));
        for path in declared {
            let mut current = String::new();
            for segment in path.split(FOLDER_SEPARATOR) {
                if !current.is_empty() {
                    current.push(FOLDER_SEPARATOR);
                }
                current.push_str(segment);
                if !all.contains(&current) {
                    all.push(current.clone());
                }
            }
        }
        all.sort_by_key(|folder| folder.to_lowercase());
        all
    }

    /// Sous-dossiers directs de `parent` (`None` = racine), chemins complets.
    pub fn child_folders(&self, parent: Option<&str>) -> Vec<String> {
        self.folders()
            .into_iter()
            .filter(|path| parent_folder(path) == parent)
            .collect()
    }

    /// Nombre de sessions dans le dossier et ses sous-dossiers.
    pub fn sessions_in_subtree(&self, folder: &str) -> usize {
        self.sessions
            .iter()
            .filter(|session| {
                session
                    .folder
                    .as_deref()
                    .is_some_and(|path| is_within(path, folder))
            })
            .count()
    }

    /// Crée un dossier sous `parent` (`None` = racine). `name` peut contenir
    /// des `/` pour créer plusieurs niveaux d'un coup. Retourne le chemin créé.
    pub fn create_folder(
        &mut self,
        parent: Option<&str>,
        name: &str,
    ) -> Result<String, FolderError> {
        let mut segments: Vec<&str> = parent
            .map(|parent| parent.split(FOLDER_SEPARATOR).collect())
            .unwrap_or_default();
        let new_segments: Vec<&str> = name
            .split(FOLDER_SEPARATOR)
            .map(str::trim)
            .filter(|segment| !segment.is_empty())
            .collect();
        if new_segments.is_empty() {
            return Err(FolderError::Empty);
        }
        validate_segments(&new_segments)?;
        segments.extend(new_segments);
        if segments.len() > MAX_FOLDER_DEPTH {
            return Err(FolderError::TooDeep);
        }
        let path = segments.join("/");
        if self
            .folders()
            .iter()
            .any(|known| known.eq_ignore_ascii_case(&path))
        {
            return Err(FolderError::AlreadyExists);
        }
        self.folders.push(path.clone());
        Ok(path)
    }

    /// Renomme le dernier niveau d'un dossier ; sous-dossiers et sessions suivent.
    pub fn rename_folder(&mut self, path: &str, new_name: &str) -> Result<String, FolderError> {
        let new_name = new_name.trim();
        if new_name.is_empty() {
            return Err(FolderError::Empty);
        }
        if new_name.contains(FOLDER_SEPARATOR) {
            return Err(FolderError::InvalidName);
        }
        let new_path = match parent_folder(path) {
            Some(parent) => format!("{parent}{FOLDER_SEPARATOR}{new_name}"),
            None => new_name.to_owned(),
        };
        self.relocate_folder(path, new_path)
    }

    /// Déplace un dossier sous `new_parent` (`None` = racine) en gardant son nom.
    pub fn move_folder(
        &mut self,
        path: &str,
        new_parent: Option<&str>,
    ) -> Result<String, FolderError> {
        let name = folder_name(path);
        let new_path = match new_parent {
            Some(parent) => format!("{parent}{FOLDER_SEPARATOR}{name}"),
            None => name.to_owned(),
        };
        self.relocate_folder(path, new_path)
    }

    /// Destinations possibles pour `path` : tous les dossiers hors de son
    /// propre sous-arbre (la racine est proposée à part, par `None`).
    pub fn move_destinations(&self, path: &str) -> Vec<String> {
        self.folders()
            .into_iter()
            .filter(|candidate| !is_within(candidate, path))
            .collect()
    }

    fn relocate_folder(&mut self, old: &str, new: String) -> Result<String, FolderError> {
        let known = self.folders();
        if !known.iter().any(|folder| folder == old) {
            return Err(FolderError::NotFound);
        }
        if new == old {
            return Ok(new);
        }
        // Le nouveau chemin ne peut pas être dans l'ancien sous-arbre.
        if is_within(&new, old) {
            return Err(FolderError::InvalidMove);
        }
        let segments: Vec<&str> = new.split(FOLDER_SEPARATOR).collect();
        validate_segments(&segments)?;
        // Profondeur du sous-arbre déplacé une fois à sa nouvelle place.
        let deepest = known
            .iter()
            .filter(|folder| is_within(folder, old))
            .map(|folder| {
                folder.split(FOLDER_SEPARATOR).count() - old.split(FOLDER_SEPARATOR).count()
            })
            .max()
            .unwrap_or(0);
        if segments.len() + deepest > MAX_FOLDER_DEPTH {
            return Err(FolderError::TooDeep);
        }
        // Conflit avec un autre dossier (une simple correction de casse du
        // même dossier reste permise).
        let conflict = known
            .iter()
            .any(|folder| !is_within(folder, old) && folder.eq_ignore_ascii_case(&new));
        if conflict {
            return Err(FolderError::AlreadyExists);
        }

        for session in &mut self.sessions {
            if let Some(folder) = session.folder.as_mut()
                && is_within(folder, old)
            {
                *folder = replace_prefix(folder, old, &new);
            }
        }
        let mut explicit: Vec<String> = self
            .folders
            .iter()
            .map(|folder| {
                if is_within(folder, old) {
                    replace_prefix(folder, old, &new)
                } else {
                    folder.clone()
                }
            })
            .collect();
        // Un dossier uniquement implicite (ancêtre de sessions) devient explicite
        // à sa nouvelle place, pour ne pas disparaître s'il se vide plus tard.
        if !explicit.contains(&new) {
            explicit.push(new.clone());
        }
        explicit.sort();
        explicit.dedup();
        self.folders = explicit;
        Ok(new)
    }

    /// Supprime un dossier avec ses sous-dossiers et toutes les sessions qu'ils
    /// contiennent. Retourne le nombre de sessions supprimées.
    pub fn delete_folder(&mut self, path: &str) -> Result<usize, FolderError> {
        if !self.folders().iter().any(|known| known == path) {
            return Err(FolderError::NotFound);
        }
        let before = self.sessions.len();
        self.sessions.retain(|session| {
            !session
                .folder
                .as_deref()
                .is_some_and(|folder| is_within(folder, path))
        });
        self.folders.retain(|known| !is_within(known, path));
        Ok(before - self.sessions.len())
    }

    /// Sessions d'un dossier (`None` = racine), triées par nom.
    pub fn in_folder(&self, folder: Option<&str>) -> Vec<&Session> {
        let mut sessions: Vec<&Session> = self
            .sessions
            .iter()
            .filter(|session| session.folder.as_deref() == folder)
            .collect();
        sessions.sort_by_key(|session| session.name.to_lowercase());
        sessions
    }

    /// Ajoute une session SSH par alias absent du magasin ; ignore les alias
    /// invalides. Retourne le nombre de sessions ajoutées.
    pub fn import_ssh_aliases(&mut self, aliases: &[String], folder: Option<&str>) -> usize {
        let mut added = 0;
        for alias in aliases {
            let already_known = self.sessions.iter().any(|session| {
                matches!(&session.kind, SessionKind::Ssh(ssh)
                    if ssh.host == *alias && ssh.user.is_none() && ssh.port.is_none())
            });
            if already_known {
                continue;
            }
            let kind = SessionKind::Ssh(SshSession {
                host: alias.clone(),
                user: None,
                port: None,
                identity_file: None,
                jump_host: None,
            });
            if self.add(alias, folder, kind).is_ok() {
                added += 1;
            }
        }
        added
    }
}

/// Chemin de dossier normalisé : segments rognés, vides retirés.
fn normalize_folder(folder: Option<&str>) -> Option<String> {
    let path: Vec<&str> = folder?
        .split(FOLDER_SEPARATOR)
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect();
    (!path.is_empty()).then(|| path.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::AzureVmSession;
    use std::os::unix::fs::PermissionsExt;

    fn ssh_kind(host: &str) -> SessionKind {
        SessionKind::Ssh(SshSession {
            host: host.to_owned(),
            user: None,
            port: None,
            identity_file: None,
            jump_host: None,
        })
    }

    fn temporary_directory(label: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "leeloo-test-{label}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        directory
    }

    #[test]
    fn add_assigns_increasing_ids_and_validates() {
        let mut store = SessionStore::new();
        let first = store.add("a", None, ssh_kind("a.example.com")).unwrap();
        let second = store.add("b", None, ssh_kind("b.example.com")).unwrap();
        assert_eq!((first, second), (1, 2));
        assert!(store.add("", None, ssh_kind("c.example.com")).is_err());
        assert!(store.add("d", None, ssh_kind("-oEvil")).is_err());
        assert_eq!(store.sessions().len(), 2);
        store.remove(first);
        assert_eq!(store.add("e", None, ssh_kind("e.example.com")).unwrap(), 3);
    }

    #[test]
    fn add_azure_vm_skips_duplicates_ignoring_case() {
        let vm = |resource_group: &str| AzureVmSession {
            subscription: Some("sub-a".into()),
            resource_group: resource_group.into(),
            vm_name: "dummy-vm".into(),
            prefer_private_ip: false,
        };
        let mut store = SessionStore::new();
        assert_eq!(
            store
                .add_azure_vm("dummy-vm", Some("Azure"), vm("dummy-rg"))
                .unwrap(),
            Some(1)
        );
        assert_eq!(
            store
                .add_azure_vm("dummy-vm", Some("Azure"), vm("DUMMY-RG"))
                .unwrap(),
            None
        );
        assert!(
            store
                .add_azure_vm("dummy-vm", None, vm("other-rg"))
                .unwrap()
                .is_some()
        );
        assert!(store.add_azure_vm("", None, vm("third-rg")).is_err());
        assert_eq!(store.sessions().len(), 2);
    }

    #[test]
    fn add_session_keeps_color_and_palette_and_assigns_the_id() {
        let mut store = SessionStore::new();
        let session = Session {
            id: 99,
            name: " prod ".into(),
            folder: Some("/Prod/".into()),
            color: Some("#e01b24".into()),
            palette: Some("nord".into()),
            kind: ssh_kind("prod.example.com"),
        };
        let id = store.add_session(session.clone()).unwrap();
        assert_eq!(id, 1);
        let stored = store.get(id).unwrap();
        assert_eq!(stored.name, "prod");
        assert_eq!(stored.folder.as_deref(), Some("Prod"));
        assert_eq!(stored.color.as_deref(), Some("#e01b24"));
        assert_eq!(stored.palette.as_deref(), Some("nord"));

        let mut invalid = session;
        invalid.color = Some("red".into());
        assert!(store.add_session(invalid).is_err());
        assert_eq!(store.sessions().len(), 1);
    }

    #[test]
    fn folders_include_ancestors_and_empty_folders() {
        let mut store = SessionStore::new();
        store
            .add("web", Some("Azure / Dev"), ssh_kind("web.example.com"))
            .unwrap();
        store.create_folder(None, "Web").unwrap();
        store.create_folder(Some("Azure"), "Prod").unwrap();
        assert_eq!(store.folders(), ["Azure", "Azure/Dev", "Azure/Prod", "Web"]);
        assert_eq!(store.child_folders(None), ["Azure", "Web"]);
        assert_eq!(
            store.child_folders(Some("Azure")),
            ["Azure/Dev", "Azure/Prod"]
        );
        assert!(store.child_folders(Some("Web")).is_empty());
        assert_eq!(store.sessions_in_subtree("Azure"), 1);
        assert_eq!(store.sessions_in_subtree("Azure/Prod"), 0);
        assert_eq!(store.in_folder(Some("Azure/Dev")).len(), 1);
    }

    #[test]
    fn create_folder_validates_names_depth_and_duplicates() {
        let mut store = SessionStore::new();
        assert_eq!(store.create_folder(None, "  A / B ").unwrap(), "A/B");
        assert_eq!(
            store.create_folder(None, "a/b"),
            Err(FolderError::AlreadyExists)
        );
        assert_eq!(store.create_folder(None, " / "), Err(FolderError::Empty));
        assert_eq!(
            store.create_folder(None, ".."),
            Err(FolderError::InvalidName)
        );
        assert_eq!(
            store.create_folder(None, "a\nb"),
            Err(FolderError::InvalidName)
        );
        assert_eq!(
            store.create_folder(None, &"x".repeat(65)),
            Err(FolderError::NameTooLong)
        );
        assert_eq!(
            store.create_folder(None, "1/2/3/4/5/6/7"),
            Err(FolderError::TooDeep)
        );
        assert_eq!(store.create_folder(Some("A/B"), "C").unwrap(), "A/B/C");
    }

    #[test]
    fn delete_folder_removes_subfolders_and_their_sessions_only() {
        let mut store = SessionStore::new();
        store.create_folder(None, "Azure/Empty").unwrap();
        store
            .add("dev", Some("Azure/Dev"), ssh_kind("dev.example.com"))
            .unwrap();
        store
            .add("prod", Some("Azure/Prod"), ssh_kind("prod.example.com"))
            .unwrap();
        store
            .add("azure-root", Some("Azure"), ssh_kind("root.example.com"))
            .unwrap();
        store
            .add("other", Some("Azure2"), ssh_kind("other.example.com"))
            .unwrap();
        store
            .add("loose", None, ssh_kind("loose.example.com"))
            .unwrap();

        assert_eq!(store.delete_folder("Azure/Dev"), Ok(1));
        assert_eq!(
            store.folders(),
            ["Azure", "Azure/Empty", "Azure/Prod", "Azure2"]
        );

        // « Azure » ne doit pas emporter « Azure2 » (préfixe de texte, pas de chemin).
        assert_eq!(store.delete_folder("Azure"), Ok(2));
        assert_eq!(store.folders(), ["Azure2"]);
        let names: Vec<_> = store.sessions().iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["other", "loose"]);

        assert_eq!(store.delete_folder("Nope"), Err(FolderError::NotFound));
        assert_eq!(store.delete_folder("Azure2"), Ok(1));
        assert_eq!(store.delete_folder("Azure2"), Err(FolderError::NotFound));
    }

    fn store_with_tree() -> SessionStore {
        let mut store = SessionStore::new();
        store.create_folder(None, "Azure/Empty").unwrap();
        store
            .add("dev", Some("Azure/Dev"), ssh_kind("dev.example.com"))
            .unwrap();
        store
            .add("web", Some("Azure/Dev/Web"), ssh_kind("web.example.com"))
            .unwrap();
        store
            .add("other", Some("Azure2"), ssh_kind("other.example.com"))
            .unwrap();
        store
    }

    #[test]
    fn rename_folder_moves_subtree_sessions_and_empty_folders() {
        let mut store = store_with_tree();
        assert_eq!(store.rename_folder("Azure", "Cloud").unwrap(), "Cloud");
        assert_eq!(
            store.folders(),
            [
                "Azure2",
                "Cloud",
                "Cloud/Dev",
                "Cloud/Dev/Web",
                "Cloud/Empty"
            ]
        );
        let dev = store.sessions().iter().find(|s| s.name == "dev").unwrap();
        assert_eq!(dev.folder.as_deref(), Some("Cloud/Dev"));
        // « Azure2 » n'est pas un sous-dossier de « Azure ».
        let other = store.sessions().iter().find(|s| s.name == "other").unwrap();
        assert_eq!(other.folder.as_deref(), Some("Azure2"));
    }

    #[test]
    fn rename_folder_validates_and_allows_case_only_change() {
        let mut store = store_with_tree();
        assert_eq!(
            store.rename_folder("Azure/Dev", "  "),
            Err(FolderError::Empty)
        );
        assert_eq!(
            store.rename_folder("Azure/Dev", "a/b"),
            Err(FolderError::InvalidName)
        );
        assert_eq!(
            store.rename_folder("Azure/Dev", ".."),
            Err(FolderError::InvalidName)
        );
        assert_eq!(
            store.rename_folder("Azure/Dev", "empty"),
            Err(FolderError::AlreadyExists)
        );
        assert_eq!(store.rename_folder("Nope", "x"), Err(FolderError::NotFound));
        assert_eq!(
            store.rename_folder("Azure/Dev", "Dev").unwrap(),
            "Azure/Dev"
        );
        assert_eq!(
            store.rename_folder("Azure/Dev", "dev").unwrap(),
            "Azure/dev"
        );
        assert!(store.folders().contains(&"Azure/dev/Web".to_owned()));
    }

    #[test]
    fn move_folder_to_another_parent_or_to_the_root() {
        let mut store = store_with_tree();
        assert_eq!(
            store.move_folder("Azure/Dev", Some("Azure2")).unwrap(),
            "Azure2/Dev"
        );
        assert_eq!(
            store.folders(),
            [
                "Azure",
                "Azure/Empty",
                "Azure2",
                "Azure2/Dev",
                "Azure2/Dev/Web"
            ]
        );
        assert_eq!(store.move_folder("Azure2/Dev/Web", None).unwrap(), "Web");
        let web = store.sessions().iter().find(|s| s.name == "web").unwrap();
        assert_eq!(web.folder.as_deref(), Some("Web"));
        // Déplacer là où il est déjà ne change rien.
        assert_eq!(store.move_folder("Web", None).unwrap(), "Web");
    }

    #[test]
    fn move_folder_rejects_cycles_conflicts_and_excess_depth() {
        let mut store = store_with_tree();
        assert_eq!(
            store.move_folder("Azure", Some("Azure/Dev")),
            Err(FolderError::InvalidMove)
        );
        assert_eq!(
            store.move_folder("Azure", Some("Azure")),
            Err(FolderError::InvalidMove)
        );
        store.create_folder(Some("Azure2"), "Dev").unwrap();
        assert_eq!(
            store.move_folder("Azure/Dev", Some("Azure2")),
            Err(FolderError::AlreadyExists)
        );
        store.create_folder(None, "1/2/3/4").unwrap();
        // « Azure/Dev/Web » a 2 niveaux sous « Azure/Dev » : 4 + 1 + 1 > 6 ne passe pas.
        store.create_folder(Some("1/2/3/4"), "5").unwrap();
        assert_eq!(
            store.move_folder("Azure/Dev", Some("1/2/3/4/5")),
            Err(FolderError::TooDeep)
        );
        assert_eq!(store.move_folder("Nope", None), Err(FolderError::NotFound));
    }

    #[test]
    fn move_destinations_exclude_the_folder_and_its_descendants() {
        let store = store_with_tree();
        assert_eq!(
            store.move_destinations("Azure/Dev"),
            ["Azure", "Azure/Empty", "Azure2"]
        );
        assert_eq!(
            store.move_destinations("Azure2"),
            ["Azure", "Azure/Dev", "Azure/Dev/Web", "Azure/Empty"]
        );
    }

    #[test]
    fn a_moved_implicit_folder_stays_after_its_sessions_are_removed() {
        let mut store = SessionStore::new();
        let id = store
            .add("web", Some("A/B"), ssh_kind("web.example.com"))
            .unwrap();
        assert_eq!(store.move_folder("A/B", None).unwrap(), "B");
        store.remove(id);
        assert_eq!(store.folders(), ["B"]);
    }

    #[test]
    fn empty_folders_survive_save_and_load_and_old_files_still_load() {
        let directory = temporary_directory("folders");
        let path = directory.join("sessions.json");
        let mut store = SessionStore::new();
        store.create_folder(None, "Azure/Dev").unwrap();
        store.save(&path).unwrap();
        assert_eq!(
            SessionStore::load(&path).unwrap().folders(),
            ["Azure", "Azure/Dev"]
        );

        std::fs::write(&path, r#"{"version": 1, "sessions": []}"#).unwrap();
        assert!(SessionStore::load(&path).unwrap().folders().is_empty());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn legacy_import_folder_is_renamed_on_load() {
        let directory = temporary_directory("legacy");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("sessions.json");
        std::fs::write(
            &path,
            r#"{"version": 1, "sessions": [
                {"id": 1, "name": "web", "folder": "~/.ssh/config", "type": "ssh", "host": "web"}
            ]}"#,
        )
        .unwrap();
        let store = SessionStore::load(&path).unwrap();
        assert_eq!(store.folders(), [IMPORT_FOLDER]);
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn path_helpers() {
        assert_eq!(parent_folder("A/B/C"), Some("A/B"));
        assert_eq!(parent_folder("A"), None);
        assert_eq!(folder_name("A/B/C"), "C");
        assert_eq!(folder_name("A"), "A");
    }

    #[test]
    fn update_and_remove() {
        let mut store = SessionStore::new();
        let id = store.add("a", None, ssh_kind("a.example.com")).unwrap();
        let mut session = store.get(id).unwrap().clone();
        session.name = "  renamed ".into();
        assert!(store.update(session.clone()).unwrap());
        assert_eq!(store.get(id).unwrap().name, "renamed");

        session.id = 99;
        assert!(!store.update(session).unwrap());
        assert!(store.remove(id));
        assert!(!store.remove(id));
    }

    #[test]
    fn folders_are_normalized_sorted_and_deduplicated() {
        let mut store = SessionStore::new();
        store
            .add("1", Some("/Prod/"), ssh_kind("h1.example.com"))
            .unwrap();
        store
            .add("2", Some("dev"), ssh_kind("h2.example.com"))
            .unwrap();
        store
            .add("3", Some("Prod"), ssh_kind("h3.example.com"))
            .unwrap();
        store
            .add("4", Some("  "), ssh_kind("h4.example.com"))
            .unwrap();
        assert_eq!(store.folders(), ["dev", "Prod"]);
        assert_eq!(store.in_folder(Some("Prod")).len(), 2);
        assert_eq!(store.in_folder(None).len(), 1);
    }

    #[test]
    fn save_then_load_roundtrip_with_private_permissions() {
        let directory = temporary_directory("roundtrip");
        let path = directory.join("sessions.json");
        let mut store = SessionStore::new();
        store
            .add("web", Some("Prod"), ssh_kind("web.example.com"))
            .unwrap();
        store
            .add(
                "vm",
                None,
                SessionKind::AzureVm(AzureVmSession {
                    subscription: None,
                    resource_group: "dummy-rg".into(),
                    vm_name: "dummy-vm".into(),
                    prefer_private_ip: false,
                }),
            )
            .unwrap();
        store.save(&path).unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(SessionStore::load(&path).unwrap(), store);
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn load_missing_file_gives_empty_store() {
        let path = temporary_directory("missing").join("sessions.json");
        assert_eq!(SessionStore::load(&path).unwrap(), SessionStore::new());
    }

    #[test]
    fn load_rejects_corrupt_and_future_files() {
        let directory = temporary_directory("invalid");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("sessions.json");

        std::fs::write(&path, "pas du json").unwrap();
        assert!(matches!(
            SessionStore::load(&path),
            Err(StoreError::Parse(_))
        ));

        std::fs::write(&path, r#"{"version": 2, "sessions": []}"#).unwrap();
        assert!(matches!(
            SessionStore::load(&path),
            Err(StoreError::UnsupportedVersion(2))
        ));
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn import_skips_known_and_invalid_aliases() {
        let mut store = SessionStore::new();
        store.add("web", None, ssh_kind("web")).unwrap();
        let aliases = ["web", "db", "-oEvil"].map(str::to_owned);
        assert_eq!(store.import_ssh_aliases(&aliases, Some("Importées")), 1);
        assert_eq!(store.import_ssh_aliases(&aliases, Some("Importées")), 0);
        assert_eq!(store.in_folder(Some("Importées")).len(), 1);
    }
}
