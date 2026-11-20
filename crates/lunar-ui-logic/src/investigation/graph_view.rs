//! Vue en lecture seule d'un graphe du cœur, dans un ordre d'itération
//! déterministe.
//!
//! L'ordre d'un `HashMap` est arbitraire ; il départagerait pourtant les
//! égalités des tris stables (flux de même volume, endpoints de même nom…).
//! [`GraphView::new`] range nœuds et arêtes par clé de map croissante : même
//! graphe, mêmes tableaux, même page. [`GraphView::from_entries`] garde un
//! ordre donné (celui d'insertion d'un objet JavaScript, par exemple).

use std::collections::HashMap;

use lunar_flows_core::graph::{Edge, GraphData, Node};

/// Nœuds et arêtes d'un graphe, empruntés, avec leur clé de map.
#[derive(Clone, Debug, Default)]
pub struct GraphView<'a> {
    /// Nœuds, avec leur clé d'identité (`(VLAN, IP)` ou `(VLAN, MAC)`).
    nodes: Vec<(&'a str, &'a Node)>,
    /// Arêtes, avec leur clé `a_id:b_id:protocole`.
    edges: Vec<(&'a str, &'a Edge)>,
}

impl<'a> GraphView<'a> {
    /// Vue d'un graphe complet, nœuds et arêtes par clé croissante.
    pub fn new(graph: &'a GraphData) -> Self {
        Self::from_maps(&graph.nodes, &graph.edges)
    }

    /// Vue de deux maps (nœuds, arêtes), par clé croissante.
    pub fn from_maps(nodes: &'a HashMap<String, Node>, edges: &'a HashMap<String, Edge>) -> Self {
        let mut nodes: Vec<_> = nodes
            .iter()
            .map(|(key, node)| (key.as_str(), node))
            .collect();
        let mut edges: Vec<_> = edges
            .iter()
            .map(|(key, edge)| (key.as_str(), edge))
            .collect();
        nodes.sort_unstable_by(|a, b| a.0.cmp(b.0));
        edges.sort_unstable_by(|a, b| a.0.cmp(b.0));
        Self { nodes, edges }
    }

    /// Vue dans l'ordre donné, sans tri.
    pub fn from_entries(
        nodes: impl IntoIterator<Item = (&'a str, &'a Node)>,
        edges: impl IntoIterator<Item = (&'a str, &'a Edge)>,
    ) -> Self {
        Self {
            nodes: nodes.into_iter().collect(),
            edges: edges.into_iter().collect(),
        }
    }

    /// Nœuds, avec leur clé de map.
    pub fn nodes(&self) -> &[(&'a str, &'a Node)] {
        &self.nodes
    }

    /// Arêtes, avec leur clé de map.
    pub fn edges(&self) -> &[(&'a str, &'a Edge)] {
        &self.edges
    }

    /// Même vue, restreinte aux arêtes retenues par `keep` (nœuds inchangés).
    pub fn retain_edges(&self, keep: impl Fn(&Edge) -> bool) -> Self {
        Self {
            nodes: self.nodes.clone(),
            edges: self
                .edges
                .iter()
                .copied()
                .filter(|(_, edge)| keep(edge))
                .collect(),
        }
    }
}
