//! Navigation entre onglets (logique pure).

/// Position (à partir de 0) de l'onglet demandé par un raccourci Alt+1…9.
/// `requested` est le chiffre tapé (1 = premier onglet) ; `None` s'il n'existe pas.
pub fn nth_tab_index(requested: i32, page_count: i32) -> Option<i32> {
    (1..=page_count)
        .contains(&requested)
        .then_some(requested - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits_map_to_existing_tabs_only() {
        assert_eq!(nth_tab_index(1, 3), Some(0));
        assert_eq!(nth_tab_index(3, 3), Some(2));
        assert_eq!(nth_tab_index(4, 3), None);
        assert_eq!(nth_tab_index(9, 0), None);
    }

    #[test]
    fn invalid_digits_are_rejected() {
        assert_eq!(nth_tab_index(0, 5), None);
        assert_eq!(nth_tab_index(-1, 5), None);
    }
}
