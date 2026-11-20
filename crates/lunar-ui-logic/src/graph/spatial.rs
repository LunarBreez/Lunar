//! Index spatial des nœuds pour le test d'atteinte (nœud sous le pointeur).
//!
//! Une grille uniforme sur les centres des nœuds trouve l'élément survolé
//! sans GPU, en ne testant que les cellules voisines du point. Les tampons sont
//! réutilisés d'une reconstruction à l'autre.

use super::store::{GraphNode, NodeIndex};

/// Nœud rangé dans la grille.
#[derive(Clone, Copy, Debug)]
struct Entry {
    /// Rang du nœud.
    index: u32,
    /// Abscisse monde.
    x: f32,
    /// Ordonnée monde.
    y: f32,
    /// Rayon d'écran (taille du nœud).
    size: f32,
}

/// Grille uniforme sur les centres des nœuds, d'environ deux nœuds par
/// cellule.
#[derive(Clone, Debug, Default)]
pub struct SpatialGrid {
    /// Abscisse du bord gauche de la grille.
    min_x: f32,
    /// Ordonnée du bord haut de la grille.
    min_y: f32,
    /// Côté d'une cellule.
    cell: f32,
    /// Colonnes.
    cols: usize,
    /// Lignes.
    rows: usize,
    /// Début de chaque cellule dans `entries` (`cols × rows + 1` bornes).
    cell_start: Vec<u32>,
    /// Curseurs de remplissage, réutilisés.
    cursor: Vec<u32>,
    /// Nœuds triés par cellule.
    entries: Vec<Entry>,
    /// Plus grande taille de nœud.
    max_size: f32,
}

impl SpatialGrid {
    /// Cellule (colonne, ligne) d'un point, bornée à la grille.
    fn cell_of(&self, x: f32, y: f32) -> (usize, usize) {
        let clamp = |v: f32, n: usize| {
            if v.is_nan() || v < 0.0 {
                0
            } else {
                (v as usize).min(n.saturating_sub(1))
            }
        };
        (
            clamp((x - self.min_x) / self.cell, self.cols),
            clamp((y - self.min_y) / self.cell, self.rows),
        )
    }

    /// Reconstruit la grille sur les positions et tailles des nœuds.
    pub fn rebuild(&mut self, nodes: &[GraphNode]) {
        self.entries.clear();
        self.cell_start.clear();
        self.cols = 0;
        self.rows = 0;
        self.max_size = 0.0;
        let Some(first) = nodes.first() else { return };

        let (mut min_x, mut min_y, mut max_x, mut max_y) = (first.x, first.y, first.x, first.y);
        for n in nodes {
            min_x = min_x.min(n.x);
            min_y = min_y.min(n.y);
            max_x = max_x.max(n.x);
            max_y = max_y.max(n.y);
            self.max_size = self.max_size.max(n.size);
        }
        let (w, h) = (max_x - min_x, max_y - min_y);
        let target = (nodes.len() / 2).max(1) as f32;
        let extent = w.max(h);
        // Environ deux nœuds par cellule, et jamais plus de `target + 1`
        // cellules sur un côté (emprise très allongée).
        let mut cell = ((w * h) / target).sqrt().max(extent / target);
        if !cell.is_finite() || cell <= 0.0 {
            cell = 1.0;
        }
        self.min_x = min_x;
        self.min_y = min_y;
        self.cell = cell;
        self.cols = (w / cell) as usize + 1;
        self.rows = (h / cell) as usize + 1;
        let cells = self.cols * self.rows;

        // Tri par comptage : effectifs, bornes cumulées, puis placement.
        self.cell_start.resize(cells + 1, 0);
        for n in nodes {
            let (cx, cy) = self.cell_of(n.x, n.y);
            self.cell_start[cy * self.cols + cx + 1] += 1;
        }
        for i in 0..cells {
            self.cell_start[i + 1] += self.cell_start[i];
        }
        self.cursor.clear();
        self.cursor.extend_from_slice(&self.cell_start[..cells]);
        self.entries.resize(
            nodes.len(),
            Entry {
                index: 0,
                x: 0.0,
                y: 0.0,
                size: 0.0,
            },
        );
        for (i, n) in nodes.iter().enumerate() {
            let (cx, cy) = self.cell_of(n.x, n.y);
            let slot = &mut self.cursor[cy * self.cols + cx];
            self.entries[*slot as usize] = Entry {
                index: i as u32,
                x: n.x,
                y: n.y,
                size: n.size,
            };
            *slot += 1;
        }
    }

    /// Nœud dont le disque (rayon `size × size_to_world`) contient le point ;
    /// parmi plusieurs, le dernier inséré (dessiné au-dessus).
    pub fn node_at(&self, x: f32, y: f32, size_to_world: f32) -> Option<NodeIndex> {
        if self.entries.is_empty() {
            return None;
        }
        let reach = self.max_size * size_to_world;
        let (x0, y0) = self.cell_of(x - reach, y - reach);
        let (x1, y1) = self.cell_of(x + reach, y + reach);
        let mut best: Option<u32> = None;
        for cy in y0..=y1 {
            let row = cy * self.cols;
            let start = self.cell_start[row + x0] as usize;
            let end = self.cell_start[row + x1 + 1] as usize;
            for e in &self.entries[start..end] {
                let r = e.size * size_to_world;
                let (dx, dy) = (e.x - x, e.y - y);
                if dx * dx + dy * dy <= r * r && best.is_none_or(|b| e.index > b) {
                    best = Some(e.index);
                }
            }
        }
        best.map(NodeIndex)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::rng::Rng;
    use crate::graph::store::GraphStore;
    use crate::graph::style::tests::node;

    fn store(points: &[(f32, f32)]) -> GraphStore {
        let mut g = GraphStore::new(Rng::with_seed(8));
        for (i, &(x, y)) in points.iter().enumerate() {
            let id = format!("n{i}");
            g.add_snapshot_node(&id, &node(&id), x, y);
        }
        g
    }

    /// Recherche exhaustive de référence.
    fn brute_force(g: &GraphStore, x: f32, y: f32, k: f32) -> Option<NodeIndex> {
        g.nodes()
            .iter()
            .enumerate()
            .filter(|(_, n)| (n.x - x).powi(2) + (n.y - y).powi(2) <= (n.size * k).powi(2))
            .map(|(i, _)| NodeIndex(i as u32))
            .next_back()
    }

    #[test]
    fn grille_vide_aucun_noeud() {
        let mut grid = SpatialGrid::default();
        grid.rebuild(&[]);
        assert_eq!(grid.node_at(0.0, 0.0, 1.0), None);
    }

    #[test]
    fn noeud_sous_le_point_dernier_insere_au_dessus() {
        // Taille des nœuds de snapshot : 2.
        let g = store(&[(0.0, 0.0), (1.0, 0.0), (100.0, 100.0)]);
        let mut grid = SpatialGrid::default();
        grid.rebuild(g.nodes());
        assert_eq!(grid.node_at(0.5, 0.0, 1.0), Some(NodeIndex(1)));
        assert_eq!(grid.node_at(-1.5, 0.0, 1.0), Some(NodeIndex(0)));
        assert_eq!(grid.node_at(101.0, 100.0, 1.0), Some(NodeIndex(2)));
        assert_eq!(grid.node_at(50.0, 50.0, 1.0), None);
        // Zoom arrière : un rayon monde plus grand.
        assert_eq!(grid.node_at(110.0, 100.0, 10.0), Some(NodeIndex(2)));
    }

    #[test]
    fn identique_a_la_recherche_exhaustive() {
        let mut rng = Rng::with_seed(12);
        let points: Vec<_> = (0..2000)
            .map(|_| {
                (
                    (rng.random_float() * 4000.0) as f32,
                    (rng.random_float() * 300.0) as f32,
                )
            })
            .collect();
        let g = store(&points);
        let mut grid = SpatialGrid::default();
        grid.rebuild(g.nodes());
        for _ in 0..2000 {
            let x = (rng.random_float() * 4200.0 - 100.0) as f32;
            let y = (rng.random_float() * 500.0 - 100.0) as f32;
            for k in [0.5, 1.0, 8.0] {
                assert_eq!(
                    grid.node_at(x, y, k),
                    brute_force(&g, x, y, k),
                    "({x}, {y}) ×{k}"
                );
            }
        }
    }

    #[test]
    fn emprise_degeneree() {
        let g = store(&[(5.0, 5.0), (5.0, 5.0), (5.0, 5.0)]);
        let mut grid = SpatialGrid::default();
        grid.rebuild(g.nodes());
        assert_eq!(grid.node_at(5.0, 6.0, 1.0), Some(NodeIndex(2)));
    }
}
