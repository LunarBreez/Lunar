//! Mise à jour optimiste du label d'un nœud : l'interface reflète
//! immédiatement la saisie, mais si le cœur refuse la mutation, le rollback
//! retourné doit être appliqué pour ne pas laisser affiché un label qui
//! n'existe pas. Le retour arrière est la valeur [`LabelRollback`].

use super::store::GraphStore;

/// Attributs d'origine à restaurer si la mutation échoue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LabelRollback {
    /// Nœud modifié.
    node_id: String,
    /// Label brut d'origine.
    raw_label: String,
    /// Libellé affiché d'origine.
    label: String,
}

impl LabelRollback {
    /// Restaure les attributs d'origine ; sans effet si le nœud a disparu
    /// (graphe vidé entre-temps).
    pub fn rollback(self, store: &mut GraphStore) {
        let Some(index) = store.node_index(&self.node_id) else {
            return;
        };
        if let Some(node) = store.node_mut(index) {
            node.style.raw_label = self.raw_label;
            node.style.label = self.label;
        }
    }
}

/// Applique `new_label` au nœud : label brut, et libellé affiché (le nom du
/// nœud si le label est vide). Retourne de quoi revenir en arrière, ou
/// `None` si le nœud n'existe pas.
///
/// Le libellé optimiste ne porte pas le suffixe
/// « (VLAN n) » : la mise à jour renvoyée par le cœur le rétablit.
pub fn apply_optimistic_label(
    store: &mut GraphStore,
    node_id: &str,
    new_label: &str,
) -> Option<LabelRollback> {
    let index = store.node_index(node_id)?;
    let node = store.node_mut(index)?;
    let previous = LabelRollback {
        node_id: node_id.to_string(),
        raw_label: node.style.raw_label.clone(),
        label: node.style.label.clone(),
    };
    let shown = [new_label, node.style.name.as_str(), node_id]
        .into_iter()
        .find(|s| !s.is_empty())
        .unwrap_or("")
        .to_string();
    node.style.raw_label = new_label.to_string();
    node.style.label = shown;
    Some(previous)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::rng::Rng;
    use crate::graph::style::tests::node;

    fn new_graph() -> GraphStore {
        let mut g = GraphStore::new(Rng::with_seed(6));
        let mut n = node("192.168.1.1");
        n.name = "hote-a".into();
        n.ip = "192.168.1.1".into();
        n.label = Some("Ancien label".into());
        g.upsert_node(&n);
        g
    }

    fn labels(g: &GraphStore) -> (String, String) {
        let n = g.node_by_id("192.168.1.1").expect("nœud");
        (n.style.raw_label.clone(), n.style.label.clone())
    }

    #[test]
    fn applique_le_nouveau_label_immediatement() {
        let mut g = new_graph();
        apply_optimistic_label(&mut g, "192.168.1.1", "Nouveau");

        assert_eq!(labels(&g), ("Nouveau".into(), "Nouveau".into()));
    }

    #[test]
    fn label_vide_retombe_sur_le_nom_du_noeud() {
        let mut g = new_graph();
        apply_optimistic_label(&mut g, "192.168.1.1", "");

        assert_eq!(labels(&g), (String::new(), "hote-a".into()));
    }

    #[test]
    fn rollback_restaure_les_attributs_d_origine() {
        let mut g = new_graph();
        let rollback = apply_optimistic_label(&mut g, "192.168.1.1", "Nouveau").expect("nœud");

        rollback.rollback(&mut g);

        assert_eq!(labels(&g), ("Ancien label".into(), "Ancien label".into()));
    }

    #[test]
    fn rollback_sans_effet_si_le_noeud_a_disparu() {
        let mut g = new_graph();
        let rollback = apply_optimistic_label(&mut g, "192.168.1.1", "Nouveau").expect("nœud");

        // Les nœuds ne disparaissent qu'avec le graphe entier (reset).
        g.clear();
        rollback.rollback(&mut g);

        assert!(!g.has_node("192.168.1.1"));
    }

    #[test]
    fn noeud_inexistant_aucun_rollback() {
        let mut g = new_graph();
        assert_eq!(apply_optimistic_label(&mut g, "fantome", "x"), None);
    }
}
