//! Préférences de l'application : `$XDG_CONFIG_HOME/leeloo/settings.json`.

use crate::palette::{self, DEFAULT_PALETTE_ID};
use crate::private_file;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const MIN_OPACITY_PERCENT: u8 = 30;
pub const MAX_OPACITY_PERCENT: u8 = 100;
pub const MIN_SIDEBAR_WIDTH: i32 = 200;
pub const MAX_SIDEBAR_WIDTH: i32 = 700;
pub const DEFAULT_SIDEBAR_WIDTH: i32 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    /// Suit le thème du système.
    #[default]
    System,
    Light,
    Dark,
}

impl Theme {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "system" => Some(Self::System),
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Copie automatiquement dans le presse-papiers ce qui est sélectionné.
    pub copy_on_select: bool,
    pub theme: Theme,
    /// Opacité du fond du terminal, de 30 à 100 %.
    pub opacity_percent: u8,
    /// Identifiant de palette du terminal (`default` = suit le thème).
    pub palette: String,
    /// Panneau des sessions affiché.
    pub sidebar_visible: bool,
    /// Largeur du panneau des sessions, en pixels (200 à 700).
    pub sidebar_width: i32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            copy_on_select: true,
            theme: Theme::System,
            opacity_percent: MAX_OPACITY_PERCENT,
            palette: DEFAULT_PALETTE_ID.to_owned(),
            sidebar_visible: true,
            sidebar_width: DEFAULT_SIDEBAR_WIDTH,
        }
    }
}

/// Borne la largeur du panneau : ni inutilisable, ni envahissant.
pub fn clamp_sidebar_width(value: i32) -> i32 {
    value.clamp(MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH)
}

/// Borne l'opacité : un fond quasi invisible rendrait le terminal illisible.
pub fn clamp_opacity_percent(value: u8) -> u8 {
    value.clamp(MIN_OPACITY_PERCENT, MAX_OPACITY_PERCENT)
}

impl Settings {
    pub fn default_path() -> Option<PathBuf> {
        private_file::config_dir().map(|directory| directory.join("settings.json"))
    }

    /// Fichier absent = réglages par défaut ; fichier illisible = erreur.
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(content) => serde_json::from_str::<Self>(&content)
                .map(|mut settings| {
                    settings.opacity_percent = clamp_opacity_percent(settings.opacity_percent);
                    settings.sidebar_width = clamp_sidebar_width(settings.sidebar_width);
                    // Palette supprimée ou inconnue : retour au défaut, sans erreur.
                    if palette::by_id(&settings.palette).is_none() {
                        settings.palette = DEFAULT_PALETTE_ID.to_owned();
                    }
                    settings
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
            "leeloo-settings-{label}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        directory
    }

    #[test]
    fn defaults_enable_copy_on_select() {
        assert!(Settings::default().copy_on_select);
    }

    #[test]
    fn defaults_are_system_theme_and_opaque() {
        let settings = Settings::default();
        assert_eq!(settings.theme, Theme::System);
        assert_eq!(settings.opacity_percent, 100);
    }

    #[test]
    fn unknown_palette_falls_back_to_default_and_sidebar_defaults_visible() {
        let directory = temporary_directory("palette");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("settings.json");
        std::fs::write(&path, r#"{"palette": "disparue"}"#).unwrap();
        let settings = Settings::load(&path).unwrap();
        assert_eq!(settings.palette, DEFAULT_PALETTE_ID);
        assert!(settings.sidebar_visible);
        std::fs::write(&path, r#"{"palette": "dracula"}"#).unwrap();
        assert_eq!(Settings::load(&path).unwrap().palette, "dracula");
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn sidebar_width_is_clamped_on_load_and_defaults_when_absent() {
        assert_eq!(clamp_sidebar_width(-5), MIN_SIDEBAR_WIDTH);
        assert_eq!(clamp_sidebar_width(5000), MAX_SIDEBAR_WIDTH);
        assert_eq!(clamp_sidebar_width(420), 420);
        let directory = temporary_directory("sidebar");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("settings.json");
        std::fs::write(&path, r#"{"sidebar_width": 9999}"#).unwrap();
        assert_eq!(
            Settings::load(&path).unwrap().sidebar_width,
            MAX_SIDEBAR_WIDTH
        );
        std::fs::write(&path, r#"{"sidebar_width": 1}"#).unwrap();
        assert_eq!(
            Settings::load(&path).unwrap().sidebar_width,
            MIN_SIDEBAR_WIDTH
        );
        std::fs::write(&path, "{}").unwrap();
        assert_eq!(
            Settings::load(&path).unwrap().sidebar_width,
            DEFAULT_SIDEBAR_WIDTH
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn theme_names_roundtrip() {
        for theme in [Theme::System, Theme::Light, Theme::Dark] {
            assert_eq!(Theme::from_name(theme.as_str()), Some(theme));
        }
        assert_eq!(Theme::from_name("solarized"), None);
    }

    #[test]
    fn opacity_is_clamped_on_load_and_by_helper() {
        assert_eq!(clamp_opacity_percent(0), 30);
        assert_eq!(clamp_opacity_percent(250), 100);
        assert_eq!(clamp_opacity_percent(75), 75);
        let directory = temporary_directory("opacity");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("settings.json");
        std::fs::write(&path, r#"{"theme": "dark", "opacity_percent": 5}"#).unwrap();
        let settings = Settings::load(&path).unwrap();
        assert_eq!(
            (settings.theme, settings.opacity_percent),
            (Theme::Dark, 30)
        );
        std::fs::write(&path, r#"{"theme": "inconnu"}"#).unwrap();
        assert!(Settings::load(&path).is_err());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn missing_file_gives_defaults() {
        let path = temporary_directory("missing").join("settings.json");
        assert_eq!(Settings::load(&path).unwrap(), Settings::default());
    }

    #[test]
    fn roundtrip_keeps_value_and_private_mode() {
        let directory = temporary_directory("roundtrip");
        let path = directory.join("settings.json");
        let settings = Settings {
            copy_on_select: false,
            theme: Theme::Light,
            opacity_percent: 80,
            palette: "nord".to_owned(),
            sidebar_visible: false,
            sidebar_width: 450,
        };
        settings.save(&path).unwrap();
        assert_eq!(Settings::load(&path).unwrap(), settings);
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn unknown_or_missing_fields_fall_back_and_corruption_errors() {
        let directory = temporary_directory("partial");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("settings.json");
        std::fs::write(&path, "{}").unwrap();
        assert_eq!(Settings::load(&path).unwrap(), Settings::default());
        std::fs::write(&path, "pas du json").unwrap();
        assert!(Settings::load(&path).is_err());
        let _ = std::fs::remove_dir_all(&directory);
    }
}
