//! Restauration de session : onglets, divisions et dossiers repliés, dans
//! `$XDG_CONFIG_HOME/leeloo/workspace.json` (0600).
//!
//! Seules des *références* sont enregistrées (terminal local, identifiant d'une
//! session enregistrée, ou VM Azure), jamais une ligne de commande : à la
//! restauration, la commande est reconstruite et validée par le modèle de
//! session. Un fichier modifié à la main ne peut donc pas lancer de programme
//! arbitraire.

use crate::private_file;
use crate::session::AzureVmSession;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const MAX_TABS: usize = 32;
pub const MAX_PANES_PER_TAB: usize = 16;
pub const MAX_SPLIT_DEPTH: usize = 8;
pub const MAX_TITLE_CHARS: usize = 128;
pub const MAX_COLLAPSED_FOLDERS: usize = 256;

/// Ce qui tourne dans un volet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PaneSource {
    /// Shell local de l'utilisateur.
    Local,
    /// Session enregistrée dans `sessions.json`.
    Session { id: u64 },
    /// VM choisie dans le sélecteur Azure.
    Azure { vm: AzureVmSession },
}

/// Disposition des volets d'un onglet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Layout {
    Pane {
        source: PaneSource,
    },
    Split {
        /// `true` : volets l'un au-dessus de l'autre ; `false` : côte à côte.
        vertical: bool,
        first: Box<Layout>,
        second: Box<Layout>,
    },
}

impl Layout {
    pub fn pane_count(&self) -> usize {
        match self {
            Self::Pane { .. } => 1,
            Self::Split { first, second, .. } => first.pane_count() + second.pane_count(),
        }
    }

    pub fn depth(&self) -> usize {
        match self {
            Self::Pane { .. } => 0,
            Self::Split { first, second, .. } => 1 + first.depth().max(second.depth()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabLayout {
    /// Titre choisi par l'utilisateur ; `None` = titre automatique du terminal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub layout: Layout,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Workspace {
    pub tabs: Vec<TabLayout>,
    /// Indice de l'onglet sélectionné.
    pub selected: usize,
    /// Chemins des dossiers repliés dans le volet des sessions.
    pub collapsed_folders: Vec<String>,
}

impl Workspace {
    pub fn default_path() -> Option<PathBuf> {
        private_file::config_dir().map(|directory| directory.join("workspace.json"))
    }

    /// Borne ce qui vient du disque : un fichier démesuré ou trafiqué ne doit
    /// pas ouvrir des centaines de terminaux.
    pub fn sanitize(&mut self) {
        self.tabs.retain(|tab| {
            tab.layout.pane_count() <= MAX_PANES_PER_TAB && tab.layout.depth() <= MAX_SPLIT_DEPTH
        });
        self.tabs.truncate(MAX_TABS);
        for tab in &mut self.tabs {
            tab.title = tab
                .title
                .take()
                .map(|title| {
                    title
                        .chars()
                        .filter(|character| !character.is_control())
                        .take(MAX_TITLE_CHARS)
                        .collect::<String>()
                })
                .filter(|title| !title.trim().is_empty());
        }
        self.selected = self.selected.min(self.tabs.len().saturating_sub(1));
        self.collapsed_folders.truncate(MAX_COLLAPSED_FOLDERS);
    }

    /// Fichier absent = espace vide ; fichier illisible = erreur.
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(content) => serde_json::from_str::<Self>(&content)
                .map(|mut workspace| {
                    workspace.sanitize();
                    workspace
                })
                .map_err(|error| error.to_string()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error.to_string()),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self).map_err(|error| error.to_string())?;
        private_file::write_atomic(path, json.as_bytes()).map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn temporary_directory(label: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "leeloo-workspace-{label}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        directory
    }

    fn pane(source: PaneSource) -> Layout {
        Layout::Pane { source }
    }

    fn split(first: Layout, second: Layout) -> Layout {
        Layout::Split {
            vertical: false,
            first: Box::new(first),
            second: Box::new(second),
        }
    }

    fn tab(layout: Layout) -> TabLayout {
        TabLayout {
            title: None,
            layout,
        }
    }

    #[test]
    fn roundtrip_keeps_layout_and_private_mode() {
        let directory = temporary_directory("roundtrip");
        let path = directory.join("workspace.json");
        let workspace = Workspace {
            tabs: vec![
                tab(pane(PaneSource::Local)),
                TabLayout {
                    title: Some("prod".to_owned()),
                    layout: split(
                        pane(PaneSource::Session { id: 7 }),
                        pane(PaneSource::Azure {
                            vm: AzureVmSession {
                                subscription: Some("sub".to_owned()),
                                resource_group: "rg".to_owned(),
                                vm_name: "vm1".to_owned(),
                                prefer_private_ip: true,
                            },
                        }),
                    ),
                },
            ],
            selected: 1,
            collapsed_folders: vec!["Azure/Dev".to_owned()],
        };
        workspace.save(&path).unwrap();
        assert_eq!(Workspace::load(&path).unwrap(), workspace);
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn missing_file_is_empty_and_corruption_errors() {
        let directory = temporary_directory("missing");
        let path = directory.join("workspace.json");
        assert_eq!(Workspace::load(&path).unwrap(), Workspace::default());
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(&path, "pas du json").unwrap();
        assert!(Workspace::load(&path).is_err());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn stored_data_never_contains_a_command_line() {
        let json = serde_json::to_string(&tab(pane(PaneSource::Session { id: 3 }))).unwrap();
        assert!(!json.contains("argv") && !json.contains("ssh"));
    }

    #[test]
    fn sanitize_bounds_tabs_panes_depth_title_and_selection() {
        let mut deep = pane(PaneSource::Local);
        for _ in 0..=MAX_SPLIT_DEPTH {
            deep = split(deep, pane(PaneSource::Local));
        }
        let mut wide = pane(PaneSource::Local);
        for _ in 0..MAX_PANES_PER_TAB {
            wide = split(wide, pane(PaneSource::Local));
        }
        let mut workspace = Workspace {
            tabs: vec![tab(deep), tab(wide)],
            selected: 5,
            collapsed_folders: Vec::new(),
        };
        workspace.tabs.extend((0..MAX_TABS + 10).map(|_| TabLayout {
            title: Some(format!("a\u{1b}[31m{}", "b".repeat(500))),
            layout: pane(PaneSource::Local),
        }));
        workspace.selected = 10_000;
        workspace.collapsed_folders = vec!["x".to_owned(); MAX_COLLAPSED_FOLDERS + 5];
        workspace.sanitize();

        assert_eq!(workspace.tabs.len(), MAX_TABS);
        assert!(workspace.tabs.iter().all(|t| t.layout.pane_count() == 1));
        assert_eq!(workspace.selected, MAX_TABS - 1);
        assert_eq!(workspace.collapsed_folders.len(), MAX_COLLAPSED_FOLDERS);
        let title = workspace.tabs[0].title.as_deref().unwrap();
        assert!(title.chars().count() <= MAX_TITLE_CHARS);
        assert!(!title.contains('\u{1b}'));
    }

    #[test]
    fn empty_workspace_keeps_selection_at_zero() {
        let mut workspace = Workspace {
            selected: 4,
            ..Workspace::default()
        };
        workspace.sanitize();
        assert_eq!(workspace.selected, 0);
    }
}
