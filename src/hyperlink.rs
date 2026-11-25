//! Liens cliquables du terminal (logique pure) : détection des URL et
//! validation avant ouverture.
//!
//! Un lien OSC 8 peut afficher un texte et pointer ailleurs : seuls les
//! schémas web et `mailto:` s'ouvrent, jamais `file:` ni un schéma applicatif.

/// Expression (PCRE2) des URL web écrites en clair. Le dernier caractère exclut
/// la ponctuation de fin de phrase et les fermants.
pub const URL_PATTERN: &str = r#"https?://[^\s<>"'`\x00-\x1f]*[^\s<>"'`.,;:!?)\]}\x00-\x1f]"#;

const MAX_URI_LENGTH: usize = 2048;
const ALLOWED_SCHEMES: [&str; 3] = ["http://", "https://", "mailto:"];

/// Retourne l'URI si elle peut être ouverte sans risque, sinon `None`.
pub fn openable_uri(uri: &str) -> Option<&str> {
    let lower = uri.to_ascii_lowercase();
    let allowed = ALLOWED_SCHEMES
        .iter()
        .any(|scheme| lower.starts_with(scheme) && lower.len() > scheme.len());
    let clean = uri.len() <= MAX_URI_LENGTH
        && !uri
            .chars()
            .any(|character| character.is_control() || character.is_whitespace());
    (allowed && clean).then_some(uri)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_and_mail_links_are_openable() {
        assert!(openable_uri("https://example.com/a?b=c").is_some());
        assert!(openable_uri("HTTP://example.com").is_some());
        assert!(openable_uri("mailto:user@example.com").is_some());
    }

    #[test]
    fn dangerous_or_malformed_links_are_refused() {
        for uri in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "ssh://host",
            "https://",
            "https://exa mple.com",
            "https://example.com/\u{1b}[31m",
            "example.com",
            "",
        ] {
            assert!(openable_uri(uri).is_none(), "{uri:?}");
        }
        let long = format!("https://example.com/{}", "a".repeat(MAX_URI_LENGTH));
        assert!(openable_uri(&long).is_none());
    }

    #[test]
    fn url_pattern_is_a_valid_pcre_shape() {
        // Garde-fou : le motif ne contient pas de groupe de capture accidentel.
        assert!(URL_PATTERN.starts_with("https?://"));
        assert!(!URL_PATTERN.contains("(?<"));
    }
}
