//! Import des alias `Host` de `~/.ssh/config`.
//!
//! Seuls les alias sont importés : `ssh` applique lui-même HostName, User,
//! Port, ProxyJump, Include, Match… au lancement. `Include` n'est pas suivi.

use std::path::PathBuf;

/// Alias `Host` explicites (les motifs `*`, `?`, `!` sont ignorés), sans doublon.
pub fn parse_host_aliases(content: &str) -> Vec<String> {
    let mut aliases: Vec<String> = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((keyword, arguments)) = split_keyword(line) else {
            continue;
        };
        if !keyword.eq_ignore_ascii_case("host") {
            continue;
        }
        for token in arguments.split_whitespace() {
            let alias = token.trim_matches('"');
            if alias.is_empty() || alias.contains(['*', '?', '!']) {
                continue;
            }
            if !aliases.iter().any(|known| known == alias) {
                aliases.push(alias.to_owned());
            }
        }
    }
    aliases
}

/// Sépare « Keyword args » ou « Keyword=args ».
fn split_keyword(line: &str) -> Option<(&str, &str)> {
    let end = line.find(|c: char| c.is_whitespace() || c == '=')?;
    let arguments = line[end..].trim_start_matches(|c: char| c.is_whitespace() || c == '=');
    Some((&line[..end], arguments))
}

pub fn default_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".ssh").join("config"))
}

/// Lit `~/.ssh/config` ; fichier absent = liste vide.
pub fn read_default_aliases() -> std::io::Result<Vec<String>> {
    let Some(path) = default_path() else {
        return Ok(Vec::new());
    };
    match std::fs::read_to_string(path) {
        Ok(content) => Ok(parse_host_aliases(&content)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_explicit_aliases_only() {
        let content = "\
# commentaire
Host *
    ServerAliveInterval 30

Host web-dev web-stg
    HostName 10.0.0.1

host=bastion
  User dummy

Host !secret *.example.com db?
Host \"quoted\"
Match host foo
Host web-dev
";
        assert_eq!(
            parse_host_aliases(content),
            ["web-dev", "web-stg", "bastion", "quoted"]
        );
    }

    #[test]
    fn ignores_other_keywords_and_empty_input() {
        assert!(parse_host_aliases("").is_empty());
        assert!(
            parse_host_aliases("HostName example.com\nHostKeyAlgorithms ssh-ed25519").is_empty()
        );
    }
}
