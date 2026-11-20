//! Totaux affichés dans le bandeau du graphe : nœuds et arêtes du
//! graphe affiché, accordés en nombre (« 0 nœud », « 1 nœud », « 2 nœuds »)
//! et séparés par milliers à la française (« 12 345 »).

use super::format::fr_integer;

/// Nombre formaté suivi du nom accordé.
fn counted(count: usize, singular: &str, plural: &str) -> String {
    let noun = if count > 1 { plural } else { singular };
    format!("{} {noun}", fr_integer(count as u64))
}

/// « 12 345 nœuds · 2 arêtes ».
pub fn graph_counts_label(nodes: usize, edges: usize) -> String {
    format!(
        "{} · {}",
        counted(nodes, "nœud", "nœuds"),
        counted(edges, "arête", "arêtes")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn totaux_du_graphe_accord_en_nombre() {
        assert_eq!(graph_counts_label(0, 0), "0 nœud · 0 arête");
        assert_eq!(graph_counts_label(1, 1), "1 nœud · 1 arête");
        assert_eq!(graph_counts_label(49, 71), "49 nœuds · 71 arêtes");
    }

    #[test]
    fn totaux_du_graphe_milliers_separes_a_la_francaise() {
        // Espace fine insécable (U+202F) entre les milliers, comme fr-FR.
        assert_eq!(
            graph_counts_label(12345, 2),
            "12\u{202F}345 nœuds · 2 arêtes"
        );
    }
}
