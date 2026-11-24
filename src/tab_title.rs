//! Nom d'onglet saisi par l'utilisateur (logique pure).

pub const MAX_TAB_TITLE_CHARS: usize = 64;

/// Nettoie un nom d'onglet : caractères de contrôle retirés, espaces de bord
/// supprimés, longueur bornée. `None` si le résultat est vide.
pub fn normalize_tab_title(input: &str) -> Option<String> {
    let cleaned: String = input.chars().filter(|c| !c.is_control()).collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.chars().take(MAX_TAB_TITLE_CHARS).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_and_keeps_inner_spaces() {
        assert_eq!(
            normalize_tab_title("  web dev  ").as_deref(),
            Some("web dev")
        );
    }

    #[test]
    fn empty_or_blank_is_rejected() {
        assert_eq!(normalize_tab_title(""), None);
        assert_eq!(normalize_tab_title(" \t\n "), None);
        assert_eq!(normalize_tab_title("\u{7}\u{1b}"), None);
    }

    #[test]
    fn control_characters_are_stripped() {
        assert_eq!(normalize_tab_title("a\nb\u{1b}c").as_deref(), Some("abc"));
    }

    #[test]
    fn length_is_bounded_by_characters_not_bytes() {
        let long = "é".repeat(200);
        let title = normalize_tab_title(&long).unwrap();
        assert_eq!(title.chars().count(), MAX_TAB_TITLE_CHARS);
    }
}
