//! Environnement des processus lancés dans un onglet.
//!
//! L'application peut être démarrée depuis un autre terminal (Ghostty, Kitty…)
//! dont le `TERM` est inconnu des machines distantes (« cannot initialize
//! terminal type xterm-ghostty »). Un onglet est un terminal VTE : on
//! déclare donc `xterm-256color`, compris partout, et on retire les variables
//! qui identifient le terminal parent.

const TERM: &str = "xterm-256color";

/// Variables du terminal parent à ne pas transmettre.
fn is_parent_terminal_variable(key: &str) -> bool {
    matches!(key, "TERM_PROGRAM" | "TERM_PROGRAM_VERSION" | "TERMINFO")
        || key.starts_with("GHOSTTY_")
        || key.starts_with("KITTY_")
        || key.starts_with("WEZTERM_")
        || key.starts_with("ALACRITTY_")
}

/// Liste `CLE=VALEUR` pour le processus fils, à partir de l'environnement donné.
pub fn terminal_environment(variables: impl IntoIterator<Item = (String, String)>) -> Vec<String> {
    let mut environment: Vec<String> = variables
        .into_iter()
        .filter(|(key, _)| key != "TERM" && key != "COLORTERM" && !is_parent_terminal_variable(key))
        .map(|(key, value)| format!("{key}={value}"))
        .collect();
    environment.push(format!("TERM={TERM}"));
    environment.push("COLORTERM=truecolor".to_owned());
    environment
}

#[cfg(test)]
mod tests {
    use super::*;

    fn variables(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn term_is_replaced_and_truecolor_declared() {
        let environment = terminal_environment(variables(&[
            ("TERM", "xterm-ghostty"),
            ("HOME", "/home/dummy"),
        ]));
        assert!(environment.contains(&"TERM=xterm-256color".to_owned()));
        assert!(environment.contains(&"COLORTERM=truecolor".to_owned()));
        assert!(environment.contains(&"HOME=/home/dummy".to_owned()));
        assert_eq!(
            environment
                .iter()
                .filter(|v| v.starts_with("TERM="))
                .count(),
            1
        );
    }

    #[test]
    fn parent_terminal_variables_are_removed() {
        let environment = terminal_environment(variables(&[
            ("GHOSTTY_RESOURCES_DIR", "/usr/share/ghostty"),
            ("KITTY_WINDOW_ID", "1"),
            ("TERM_PROGRAM", "ghostty"),
            ("TERMINFO", "/dummy"),
            ("SSH_AUTH_SOCK", "/run/dummy.sock"),
        ]));
        assert!(environment.iter().all(|v| {
            !v.starts_with("GHOSTTY_")
                && !v.starts_with("KITTY_")
                && !v.starts_with("TERM_PROGRAM")
                && !v.starts_with("TERMINFO")
        }));
        assert!(environment.contains(&"SSH_AUTH_SOCK=/run/dummy.sock".to_owned()));
    }

    #[test]
    fn works_without_term_in_input() {
        let environment = terminal_environment(Vec::new());
        assert_eq!(environment, ["TERM=xterm-256color", "COLORTERM=truecolor"]);
    }
}
