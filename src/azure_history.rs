//! Favoris et connexions récentes du sélecteur de VM Azure (logique pure).
//!
//! Stockage : `$XDG_CONFIG_HOME/leeloo/azure.json` (0600).

use crate::azure::VmRow;
use crate::private_file;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Nombre de connexions mémorisées.
pub const MAX_RECENT_STORED: usize = 10;
/// Nombre de connexions récentes affichées en tête de liste.
pub const MAX_RECENT_SHOWN: usize = 5;

/// Identité d'une VM, comparée sans tenir compte de la casse (Azure renvoie
/// les noms de groupes de ressources en minuscules).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VmKey {
    subscription_id: String,
    resource_group: String,
    name: String,
}

impl VmKey {
    pub fn from_row(row: &VmRow) -> Self {
        Self {
            subscription_id: row.subscription_id.to_lowercase(),
            resource_group: row.resource_group.to_lowercase(),
            name: row.name.to_lowercase(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Favorites,
    Recent,
    Others,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct History {
    favorites: Vec<VmKey>,
    /// Plus récente en premier.
    recent: Vec<VmKey>,
}

impl History {
    pub fn default_path() -> Option<PathBuf> {
        private_file::config_dir().map(|directory| directory.join("azure.json"))
    }

    /// Fichier absent = historique vide ; fichier illisible = erreur.
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(content) => serde_json::from_str(&content).map_err(|error| error.to_string()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error.to_string()),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self).map_err(|error| error.to_string())?;
        private_file::write_atomic(path, json.as_bytes()).map_err(|error| error.to_string())
    }

    pub fn is_favorite(&self, key: &VmKey) -> bool {
        self.favorites.contains(key)
    }

    /// Ajoute ou retire des favoris ; retourne vrai si la VM est maintenant favorite.
    pub fn toggle_favorite(&mut self, key: VmKey) -> bool {
        match self.favorites.iter().position(|known| *known == key) {
            Some(position) => {
                self.favorites.remove(position);
                false
            }
            None => {
                self.favorites.push(key);
                true
            }
        }
    }

    /// Mémorise une connexion : la VM passe en tête, sans doublon.
    pub fn record_connection(&mut self, key: VmKey) {
        self.recent.retain(|known| *known != key);
        self.recent.insert(0, key);
        self.recent.truncate(MAX_RECENT_STORED);
    }

    /// Ordre d'affichage : favoris, puis connexions récentes (non favorites,
    /// plus récente d'abord), puis le reste trié par subscription, resource
    /// group (sans tenir compte de la casse) et nom.
    pub fn order(&self, rows: Vec<VmRow>) -> Vec<(Group, VmRow)> {
        let sort_key = |row: &VmRow| {
            (
                row.subscription_id.clone(),
                row.resource_group.to_lowercase(),
                row.name.to_lowercase(),
            )
        };
        let mut favorites = Vec::new();
        let mut recent: Vec<(usize, VmRow)> = Vec::new();
        let mut others = Vec::new();
        for row in rows {
            let key = VmKey::from_row(&row);
            if self.is_favorite(&key) {
                favorites.push(row);
            } else if let Some(rank) = self.recent.iter().position(|known| *known == key)
                && rank < MAX_RECENT_SHOWN
            {
                recent.push((rank, row));
            } else {
                others.push(row);
            }
        }
        favorites.sort_by_key(sort_key);
        recent.sort_by_key(|(rank, _)| *rank);
        others.sort_by_key(sort_key);

        favorites
            .into_iter()
            .map(|row| (Group::Favorites, row))
            .chain(recent.into_iter().map(|(_, row)| (Group::Recent, row)))
            .chain(others.into_iter().map(|row| (Group::Others, row)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::os::unix::fs::PermissionsExt;

    fn vm(resource_group: &str, name: &str) -> VmRow {
        VmRow {
            subscription_id: "sub-a".to_owned(),
            resource_group: resource_group.to_owned(),
            name: name.to_owned(),
            location: "francecentral".to_owned(),
            power: None,
            tags: HashMap::new(),
        }
    }

    fn rows() -> Vec<VmRow> {
        vec![
            vm("zeta-rg", "vm-z"),
            vm("Alpha-rg", "vm-a"),
            vm("beta-rg", "vm-b"),
            vm("beta-rg", "vm-c"),
        ]
    }

    fn names(ordered: &[(Group, VmRow)]) -> Vec<(&str, &str)> {
        ordered
            .iter()
            .map(|(group, row)| {
                let tag = match group {
                    Group::Favorites => "fav",
                    Group::Recent => "rec",
                    Group::Others => "all",
                };
                (tag, row.name.as_str())
            })
            .collect()
    }

    #[test]
    fn without_history_rows_are_sorted_by_resource_group_then_name() {
        let ordered = History::default().order(rows());
        assert_eq!(
            names(&ordered),
            [
                ("all", "vm-a"),
                ("all", "vm-b"),
                ("all", "vm-c"),
                ("all", "vm-z")
            ]
        );
    }

    #[test]
    fn favorites_then_recent_then_rest() {
        let mut history = History::default();
        history.toggle_favorite(VmKey::from_row(&vm("zeta-rg", "vm-z")));
        history.record_connection(VmKey::from_row(&vm("beta-rg", "vm-b")));
        history.record_connection(VmKey::from_row(&vm("beta-rg", "vm-c")));
        let ordered = history.order(rows());
        assert_eq!(
            names(&ordered),
            [
                ("fav", "vm-z"),
                ("rec", "vm-c"),
                ("rec", "vm-b"),
                ("all", "vm-a")
            ]
        );
    }

    #[test]
    fn a_favorite_is_not_repeated_in_recent_and_keys_ignore_case() {
        let mut history = History::default();
        history.toggle_favorite(VmKey::from_row(&vm("alpha-rg", "VM-A")));
        history.record_connection(VmKey::from_row(&vm("ALPHA-RG", "vm-a")));
        let ordered = history.order(rows());
        assert_eq!(
            ordered.iter().filter(|(_, row)| row.name == "vm-a").count(),
            1
        );
        assert_eq!(ordered[0].0, Group::Favorites);
    }

    #[test]
    fn toggle_favorite_adds_then_removes() {
        let mut history = History::default();
        let key = VmKey::from_row(&vm("rg", "vm"));
        assert!(history.toggle_favorite(key.clone()));
        assert!(history.is_favorite(&key));
        assert!(!history.toggle_favorite(key.clone()));
        assert!(!history.is_favorite(&key));
    }

    #[test]
    fn recent_is_deduplicated_bounded_and_only_the_top_ones_are_shown() {
        let mut history = History::default();
        for index in 0..15 {
            history.record_connection(VmKey::from_row(&vm("rg", &format!("vm-{index}"))));
        }
        history.record_connection(VmKey::from_row(&vm("rg", "vm-14")));
        assert_eq!(history.recent.len(), MAX_RECENT_STORED);
        assert_eq!(history.recent[0], VmKey::from_row(&vm("rg", "vm-14")));

        let all: Vec<VmRow> = (0..15)
            .map(|index| vm("rg", &format!("vm-{index}")))
            .collect();
        let ordered = history.order(all);
        let shown = ordered
            .iter()
            .filter(|(group, _)| *group == Group::Recent)
            .count();
        assert_eq!(shown, MAX_RECENT_SHOWN);
    }

    #[test]
    fn unknown_vms_in_history_are_ignored() {
        let mut history = History::default();
        history.toggle_favorite(VmKey::from_row(&vm("gone-rg", "gone")));
        assert_eq!(history.order(rows()).len(), 4);
    }

    #[test]
    fn save_load_roundtrip_with_private_mode_and_missing_file() {
        let directory = std::env::temp_dir().join(format!("leeloo-history-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let path = directory.join("azure.json");
        assert_eq!(History::load(&path).unwrap(), History::default());

        let mut history = History::default();
        history.toggle_favorite(VmKey::from_row(&vm("rg", "vm")));
        history.record_connection(VmKey::from_row(&vm("rg", "vm2")));
        history.save(&path).unwrap();
        assert_eq!(History::load(&path).unwrap(), history);
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);

        std::fs::write(&path, "pas du json").unwrap();
        assert!(History::load(&path).is_err());
        let _ = std::fs::remove_dir_all(&directory);
    }
}
