//! Palettes de couleurs du terminal (logique pure).

/// Identifiant de la palette « par défaut » : suit le thème clair/sombre.
pub const DEFAULT_PALETTE_ID: &str = "default";

#[derive(Debug, PartialEq, Eq)]
pub struct Palette {
    pub id: &'static str,
    pub name: &'static str,
    pub foreground: &'static str,
    pub background: &'static str,
    /// 16 couleurs ANSI (0-7 normales, 8-15 vives), au format `#rrggbb`.
    pub colors: [&'static str; 16],
}

/// Couleurs effectives d'un terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Colors {
    pub foreground: &'static str,
    pub background: &'static str,
    /// `None` : palette ANSI par défaut de VTE.
    pub ansi: Option<&'static [&'static str; 16]>,
}

pub const PALETTES: &[Palette] = &[
    Palette {
        id: "tango",
        name: "Tango",
        foreground: "#d3d7cf",
        background: "#2e3436",
        colors: [
            "#2e3436", "#cc0000", "#4e9a06", "#c4a000", "#3465a4", "#75507b", "#06989a", "#d3d7cf",
            "#555753", "#ef2929", "#8ae234", "#fce94f", "#729fcf", "#ad7fa8", "#34e2e2", "#eeeeec",
        ],
    },
    Palette {
        id: "solarized-dark",
        name: "Solarized sombre",
        foreground: "#839496",
        background: "#002b36",
        colors: SOLARIZED_COLORS,
    },
    Palette {
        id: "solarized-light",
        name: "Solarized clair",
        foreground: "#657b83",
        background: "#fdf6e3",
        colors: SOLARIZED_COLORS,
    },
    Palette {
        id: "dracula",
        name: "Dracula",
        foreground: "#f8f8f2",
        background: "#282a36",
        colors: [
            "#21222c", "#ff5555", "#50fa7b", "#f1fa8c", "#bd93f9", "#ff79c6", "#8be9fd", "#f8f8f2",
            "#6272a4", "#ff6e6e", "#69ff94", "#ffffa5", "#d6acff", "#ff92df", "#a4ffff", "#ffffff",
        ],
    },
    Palette {
        id: "nord",
        name: "Nord",
        foreground: "#d8dee9",
        background: "#2e3440",
        colors: [
            "#3b4252", "#bf616a", "#a3be8c", "#ebcb8b", "#81a1c1", "#b48ead", "#88c0d0", "#e5e9f0",
            "#4c566a", "#bf616a", "#a3be8c", "#ebcb8b", "#81a1c1", "#b48ead", "#8fbcbb", "#eceff4",
        ],
    },
    Palette {
        id: "gruvbox-dark",
        name: "Gruvbox sombre",
        foreground: "#ebdbb2",
        background: "#282828",
        colors: [
            "#282828", "#cc241d", "#98971a", "#d79921", "#458588", "#b16286", "#689d6a", "#a89984",
            "#928374", "#fb4934", "#b8bb26", "#fabd2f", "#83a598", "#d3869b", "#8ec07c", "#ebdbb2",
        ],
    },
    Palette {
        id: "one-dark",
        name: "One Dark",
        foreground: "#abb2bf",
        background: "#282c34",
        colors: [
            "#282c34", "#e06c75", "#98c379", "#e5c07b", "#61afef", "#c678dd", "#56b6c2", "#abb2bf",
            "#5c6370", "#e06c75", "#98c379", "#e5c07b", "#61afef", "#c678dd", "#56b6c2", "#ffffff",
        ],
    },
];

/// Couleurs proposées pour repérer une session (nom, `#rrggbb`).
pub const SESSION_COLORS: &[(&str, &str)] = &[
    ("Rouge", "#e01b24"),
    ("Orange", "#ff7800"),
    ("Jaune", "#f5c211"),
    ("Vert", "#33d17a"),
    ("Bleu", "#3584e4"),
    ("Violet", "#9141ac"),
    ("Marron", "#986a44"),
    ("Gris", "#9a9996"),
];

const SOLARIZED_COLORS: [&str; 16] = [
    "#073642", "#dc322f", "#859900", "#b58900", "#268bd2", "#d33682", "#2aa198", "#eee8d5",
    "#002b36", "#cb4b16", "#586e75", "#657b83", "#839496", "#6c71c4", "#93a1a1", "#fdf6e3",
];

/// Palette nommée ; `None` pour « default » ou un identifiant inconnu.
pub fn by_id(id: &str) -> Option<&'static Palette> {
    PALETTES.iter().find(|palette| palette.id == id)
}

/// Couleurs à appliquer : palette nommée, sinon valeurs par défaut du thème.
pub fn resolve(palette: Option<&'static Palette>, dark: bool) -> Colors {
    match palette {
        Some(palette) => Colors {
            foreground: palette.foreground,
            background: palette.background,
            ansi: Some(&palette.colors),
        },
        None if dark => Colors {
            foreground: "#d4d4d4",
            background: "#1e1e1e",
            ansi: None,
        },
        None => Colors {
            foreground: "#1e1e1e",
            background: "#ffffff",
            ansi: None,
        },
    }
}

/// `#rrggbb` → composantes.
pub fn parse_hex(hex: &str) -> Option<(u8, u8, u8)> {
    let digits = hex.strip_prefix('#').filter(|digits| digits.len() == 6)?;
    let channel = |range: std::ops::Range<usize>| u8::from_str_radix(&digits[range], 16).ok();
    Some((channel(0..2)?, channel(2..4)?, channel(4..6)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_never_the_default_id() {
        for (index, palette) in PALETTES.iter().enumerate() {
            assert_ne!(palette.id, DEFAULT_PALETTE_ID);
            assert!(
                PALETTES[index + 1..]
                    .iter()
                    .all(|other| other.id != palette.id)
            );
            assert_eq!(by_id(palette.id), Some(palette));
        }
        assert_eq!(by_id(DEFAULT_PALETTE_ID), None);
        assert_eq!(by_id("inconnue"), None);
    }

    #[test]
    fn every_color_is_valid_hex() {
        for palette in PALETTES {
            for hex in palette
                .colors
                .iter()
                .chain([&palette.foreground, &palette.background])
            {
                assert!(parse_hex(hex).is_some(), "{}: {hex}", palette.id);
            }
        }
    }

    #[test]
    fn session_colors_are_valid_and_unique() {
        for (index, (name, hex)) in SESSION_COLORS.iter().enumerate() {
            assert!(parse_hex(hex).is_some(), "{name}");
            assert!(
                SESSION_COLORS[index + 1..]
                    .iter()
                    .all(|(_, other)| other != hex)
            );
        }
    }

    #[test]
    fn parse_hex_rejects_malformed_values() {
        assert_eq!(parse_hex("#1e1e1e"), Some((30, 30, 30)));
        for invalid in ["1e1e1e", "#1e1e1", "#gggggg", "", "#1e1e1e1e"] {
            assert_eq!(parse_hex(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn default_colors_follow_the_theme_and_named_palettes_do_not() {
        let dark = resolve(None, true);
        let light = resolve(None, false);
        assert_ne!(dark.background, light.background);
        assert!(dark.ansi.is_none());

        let nord = resolve(by_id("nord"), false);
        assert_eq!(nord, resolve(by_id("nord"), true));
        assert_eq!(nord.background, "#2e3440");
        assert!(nord.ansi.is_some());
    }
}
