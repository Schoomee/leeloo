//! Navigation au clavier entre volets d'un onglet (logique pure).
//!
//! Les volets sont décrits par leur rectangle ; on cherche le voisin le plus
//! proche dans la direction demandée, parmi ceux qui lui font face.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaneRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// Tolérance (pixels) : séparateurs et arrondis de mise en page.
const EPSILON: f64 = 1.0;

/// Indice du volet voisin de `current` dans `direction`, s'il existe.
///
/// Un voisin doit se trouver au-delà du bord concerné et partager une partie du
/// bord opposé. Le plus proche gagne ; à distance égale, le plus en face.
pub fn neighbor(rects: &[PaneRect], current: usize, direction: Direction) -> Option<usize> {
    let from = *rects.get(current)?;
    let (from_start, from_end, from_cross_start, from_cross_end) = axes(from, direction);
    rects
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != current)
        .filter_map(|(index, rect)| {
            let (start, end, cross_start, cross_end) = axes(*rect, direction);
            let gap = match direction {
                Direction::Right | Direction::Down => start - from_end,
                Direction::Left | Direction::Up => from_start - end,
            };
            let overlap = from_cross_end.min(cross_end) - from_cross_start.max(cross_start);
            (gap >= -EPSILON && overlap > EPSILON).then_some((index, gap.max(0.0), overlap))
        })
        .min_by(|a, b| {
            a.1.total_cmp(&b.1)
                .then_with(|| b.2.total_cmp(&a.2))
                .then_with(|| a.0.cmp(&b.0))
        })
        .map(|(index, _, _)| index)
}

/// (début, fin, début transversal, fin transversale) selon l'axe de déplacement.
fn axes(rect: PaneRect, direction: Direction) -> (f64, f64, f64, f64) {
    match direction {
        Direction::Left | Direction::Right => {
            (rect.x, rect.x + rect.width, rect.y, rect.y + rect.height)
        }
        Direction::Up | Direction::Down => {
            (rect.y, rect.y + rect.height, rect.x, rect.x + rect.width)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, width: f64, height: f64) -> PaneRect {
        PaneRect {
            x,
            y,
            width,
            height,
        }
    }

    /// 0 occupe la gauche ; 1 (haut) et 2 (bas) se partagent la droite.
    fn layout() -> Vec<PaneRect> {
        vec![
            rect(0.0, 0.0, 400.0, 600.0),
            rect(400.0, 0.0, 400.0, 300.0),
            rect(400.0, 300.0, 400.0, 300.0),
        ]
    }

    #[test]
    fn moves_between_columns_and_rows() {
        let panes = layout();
        assert_eq!(neighbor(&panes, 0, Direction::Right), Some(1));
        assert_eq!(neighbor(&panes, 1, Direction::Left), Some(0));
        assert_eq!(neighbor(&panes, 2, Direction::Left), Some(0));
        assert_eq!(neighbor(&panes, 1, Direction::Down), Some(2));
        assert_eq!(neighbor(&panes, 2, Direction::Up), Some(1));
    }

    #[test]
    fn no_neighbor_at_the_edges() {
        let panes = layout();
        assert_eq!(neighbor(&panes, 0, Direction::Left), None);
        assert_eq!(neighbor(&panes, 0, Direction::Up), None);
        assert_eq!(neighbor(&panes, 1, Direction::Up), None);
        assert_eq!(neighbor(&panes, 2, Direction::Down), None);
        assert_eq!(neighbor(&panes, 2, Direction::Right), None);
    }

    #[test]
    fn ignores_panes_that_do_not_face_each_other() {
        // 1 est en haut à droite, 2 en bas à gauche : aucun bord commun.
        let panes = vec![
            rect(0.0, 0.0, 100.0, 100.0),
            rect(100.0, 0.0, 100.0, 100.0),
            rect(0.0, 100.0, 100.0, 100.0),
        ];
        assert_eq!(neighbor(&panes, 1, Direction::Down), None);
        assert_eq!(neighbor(&panes, 2, Direction::Right), None);
    }

    #[test]
    fn prefers_the_nearest_then_the_best_facing() {
        let panes = vec![
            rect(0.0, 0.0, 100.0, 300.0),
            rect(100.0, 0.0, 100.0, 100.0),
            rect(100.0, 100.0, 100.0, 200.0),
        ];
        // Même distance : le volet 2 fait davantage face au volet 0.
        assert_eq!(neighbor(&panes, 0, Direction::Right), Some(2));
    }

    #[test]
    fn invalid_index_or_single_pane_gives_none() {
        let panes = layout();
        assert_eq!(neighbor(&panes, 9, Direction::Left), None);
        assert_eq!(neighbor(&panes[..1], 0, Direction::Right), None);
    }
}
