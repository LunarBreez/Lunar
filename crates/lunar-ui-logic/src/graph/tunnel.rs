//! Calcul de la famille de tunnel(s) d'une arête : si elle participe à un ou
//! plusieurs tunnels (`encap_ids` non vide), la (les) arête(s) externe(s) du
//! tunnel ET les flux internes qu'il transporte forment la famille à
//! surligner. Fonctionne dans les deux sens : le CAPWAP montre son contenu,
//! un flux interne montre par quel(s) tunnel(s) il est passé.
//!
//! Les ensembles d'identifiants sont des masques indexés par rang, lus en
//! O(1) par le rendu de chaque frame.

use super::store::{EdgeIndex, GraphStore, NodeIndex};

/// Famille de tunnel(s) surlignée : arêtes et nœuds conservés, le reste est
/// estompé.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TunnelHighlight {
    /// Arête de rang i dans la famille.
    edges: Vec<bool>,
    /// Nœud de rang i extrémité d'une arête de la famille.
    nodes: Vec<bool>,
    /// Nombre d'arêtes de la famille.
    edge_count: usize,
    /// Libellé du bandeau : « tunnel abcd1234… — 2 flux liés ».
    info: String,
}

impl TunnelHighlight {
    /// L'arête fait-elle partie de la famille ? Une arête ajoutée après le
    /// calcul n'en fait pas partie.
    pub fn contains_edge(&self, edge: EdgeIndex) -> bool {
        self.edges.get(edge.index()).copied().unwrap_or(false)
    }

    /// Le nœud est-il une extrémité d'une arête de la famille ?
    pub fn contains_node(&self, node: NodeIndex) -> bool {
        self.nodes.get(node.index()).copied().unwrap_or(false)
    }

    /// Arêtes de la famille, par rang croissant.
    pub fn edges(&self) -> impl Iterator<Item = EdgeIndex> + '_ {
        self.edges
            .iter()
            .enumerate()
            .filter(|&(_, &on)| on)
            .map(|(i, _)| EdgeIndex(i as u32))
    }

    /// Nœuds de la famille, par rang croissant.
    pub fn nodes(&self) -> impl Iterator<Item = NodeIndex> + '_ {
        self.nodes
            .iter()
            .enumerate()
            .filter(|&(_, &on)| on)
            .map(|(i, _)| NodeIndex(i as u32))
    }

    /// Nombre d'arêtes (flux) de la famille.
    pub fn edge_count(&self) -> usize {
        self.edge_count
    }

    /// Libellé du bandeau.
    pub fn info(&self) -> &str {
        &self.info
    }
}

/// Famille de tunnel(s) de `edge`, ou `None` si elle n'en a pas
///. `pinned` marque le libellé d'un 📌 quand
/// l'arête est l'arête épinglée.
pub fn compute_tunnel_highlight(
    store: &GraphStore,
    edge: EdgeIndex,
    pinned: Option<EdgeIndex>,
) -> Option<TunnelHighlight> {
    let ids = &store.edge(edge)?.style.encap_ids;
    let first = ids.first()?;

    let mut edges = vec![false; store.size()];
    let mut nodes = vec![false; store.order()];
    let mut edge_count = 0;
    for (i, e) in store.edges().iter().enumerate() {
        if !e.style.encap_ids.iter().any(|id| ids.contains(id)) {
            continue;
        }
        edges[i] = true;
        edge_count += 1;
        nodes[e.source.index()] = true;
        nodes[e.target.index()] = true;
    }

    let id_label = if ids.len() == 1 {
        // `ids[0].slice(0, 8)` : les ids sont hexadécimaux, un caractère
        // vaut une unité UTF-16.
        let short: String = first.chars().take(8).collect();
        format!("tunnel {short}…")
    } else {
        format!("{} tunnels", ids.len())
    };
    let pin = if pinned == Some(edge) { "📌 " } else { "" };
    Some(TunnelHighlight {
        edges,
        nodes,
        edge_count,
        info: format!("{pin}{id_label} — {edge_count} flux liés"),
    })
}

/// Une arête participe-t-elle à au moins un tunnel (donc épinglable) ?
pub fn edge_has_tunnel(store: &GraphStore, edge: EdgeIndex) -> bool {
    store
        .edge(edge)
        .is_some_and(|e| !e.style.encap_ids.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::rng::Rng;
    use crate::graph::style::tests::{edge, node};

    fn new_graph() -> GraphStore {
        let mut g = GraphStore::new(Rng::with_seed(4));
        for id in ["capwapA", "capwapB", "client", "server", "autre"] {
            g.upsert_node(&node(id));
        }
        // Arête externe du tunnel (CAPWAP), et le flux interne qu'elle
        // transporte, partagent le même encap_id.
        let mut tunnel = edge("capwapA", "capwapB", "CAPWAP");
        tunnel.encap_ids = vec!["enc-1".into()];
        let mut inner = edge("client", "server", "TCP");
        inner.encap_ids = vec!["enc-1".into()];
        g.upsert_edge(&tunnel);
        g.upsert_edge(&inner);
        g.upsert_edge(&edge("autre", "client", "UDP"));
        g
    }

    fn e(g: &GraphStore, key: &str) -> EdgeIndex {
        g.edge_index(key).expect(key)
    }

    fn edge_keys(g: &GraphStore, h: &TunnelHighlight) -> Vec<String> {
        h.edges()
            .map(|i| g.edge(i).expect("arête").key.clone())
            .collect()
    }

    fn node_ids(g: &GraphStore, h: &TunnelHighlight) -> Vec<String> {
        h.nodes()
            .map(|i| g.node(i).expect("nœud").id.clone())
            .collect()
    }

    #[test]
    fn arete_inexistante_none() {
        let g = new_graph();
        assert_eq!(compute_tunnel_highlight(&g, EdgeIndex(99), None), None);
    }

    #[test]
    fn arete_sans_tunnel_none() {
        let g = new_graph();
        assert_eq!(
            compute_tunnel_highlight(&g, e(&g, "autre__client__UDP"), None),
            None
        );
    }

    #[test]
    fn survol_du_capwap_montre_le_flux_interne() {
        let g = new_graph();
        let result =
            compute_tunnel_highlight(&g, e(&g, "capwapA__capwapB__CAPWAP"), None).expect("famille");

        assert_eq!(
            edge_keys(&g, &result),
            ["capwapA__capwapB__CAPWAP", "client__server__TCP"]
        );
        assert_eq!(
            node_ids(&g, &result),
            ["capwapA", "capwapB", "client", "server"]
        );
        assert_eq!(result.info(), "tunnel enc-1… — 2 flux liés");
        assert!(!result.contains_node(g.node_index("autre").expect("autre")));
    }

    #[test]
    fn survol_du_flux_interne_montre_le_tunnel_sens_inverse() {
        let g = new_graph();
        let result =
            compute_tunnel_highlight(&g, e(&g, "client__server__TCP"), None).expect("famille");

        assert_eq!(
            edge_keys(&g, &result),
            ["capwapA__capwapB__CAPWAP", "client__server__TCP"]
        );
    }

    #[test]
    fn arete_epinglee_marqueur_dans_le_libelle() {
        let g = new_graph();
        let tunnel = e(&g, "capwapA__capwapB__CAPWAP");
        let result = compute_tunnel_highlight(&g, tunnel, Some(tunnel)).expect("famille");

        assert!(result.info().starts_with("📌 "), "{}", result.info());
    }

    #[test]
    fn plusieurs_tunnels_libelle_au_pluriel() {
        let mut g = GraphStore::new(Rng::with_seed(4));
        g.upsert_node(&node("a"));
        g.upsert_node(&node("b"));
        let mut both = edge("a", "b", "GRE");
        both.encap_ids = vec!["enc-1".into(), "enc-2".into()];
        g.upsert_edge(&both);

        let result = compute_tunnel_highlight(&g, e(&g, "a__b__GRE"), None).expect("famille");

        assert_eq!(result.info(), "2 tunnels — 1 flux liés");
    }

    #[test]
    fn identifiant_long_tronque_a_8_caracteres() {
        let mut g = GraphStore::new(Rng::with_seed(4));
        g.upsert_node(&node("a"));
        g.upsert_node(&node("b"));
        let mut long = edge("a", "b", "CAPWAP");
        long.encap_ids = vec!["0123456789abcdef".into()];
        g.upsert_edge(&long);
        let result = compute_tunnel_highlight(&g, e(&g, "a__b__CAPWAP"), None).expect("famille");
        assert_eq!(result.info(), "tunnel 01234567… — 1 flux liés");
    }

    #[test]
    fn edge_has_tunnel_vrai_si_encap_ids_non_vide() {
        let g = new_graph();
        assert!(edge_has_tunnel(&g, e(&g, "capwapA__capwapB__CAPWAP")));
    }

    #[test]
    fn edge_has_tunnel_faux_si_encap_ids_vide() {
        let g = new_graph();
        assert!(!edge_has_tunnel(&g, e(&g, "autre__client__UDP")));
    }

    #[test]
    fn edge_has_tunnel_faux_si_l_arete_n_existe_pas() {
        let g = new_graph();
        assert!(!edge_has_tunnel(&g, EdgeIndex(99)));
    }
}
