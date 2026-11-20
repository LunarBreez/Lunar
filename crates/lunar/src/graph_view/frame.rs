//! Instantané de rendu : ce que le graphe doit dessiner à une image donnée,
//! en coordonnées monde, reducers de `GraphModel` appliqués.
//!
//! Le modèle d'interface (survol, sélection, tunnels, libellés) fournit les
//! styles ; les positions viennent du fil ForceAtlas2 ([`super::layout`])
//! quand elles concernent le même graphe (même génération). Un nœud que le
//! fil n'a pas encore placé est masqué, avec ses arêtes, jusqu'à la
//! publication suivante (une image au plus).
//!
//! L'instantané ne dépend ni de gpui ni du verrou du fil : le dessin à
//! l'écran, le test d'atteinte et l'export PNG le lisent sans bloquer la
//! mise en page. Les libellés sont partagés (`Arc<str>`) et réutilisés d'une
//! image à l'autre tant qu'ils ne changent pas.

use std::sync::Arc;

use lunar_ui_logic::graph::camera::Bounds;
use lunar_ui_logic::graph::{EdgeIndex, EdgeKind, GraphModel, NodeIndex, Rgb};

/// Nœud à dessiner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameNode {
    /// Rang du nœud dans le modèle.
    pub index: NodeIndex,
    /// Abscisse monde.
    pub x: f32,
    /// Ordonnée monde.
    pub y: f32,
    /// Taille Sigma (rayon d'écran au ratio 1).
    pub size: f32,
    /// Remplissage (survol, estompage appliqués).
    pub fill: Rgb,
    /// Bordure.
    pub border: Rgb,
    /// Nœud survolé.
    pub hovered: bool,
    /// Nœud sélectionné.
    pub highlighted: bool,
    /// Nœud estompé (hors de la famille de tunnel surlignée).
    pub dimmed: bool,
    /// Position connue : un nœud pas encore placé n'est pas dessiné.
    pub visible: bool,
}

/// Arête à dessiner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameEdge {
    /// Rang de l'arête dans le modèle.
    pub index: EdgeIndex,
    /// Rang du nœud source dans [`GraphFrame::nodes`].
    pub source: u32,
    /// Rang du nœud cible (porte la flèche).
    pub target: u32,
    /// Épaisseur Sigma (+1 dans une famille de tunnel surlignée).
    pub size: f32,
    /// Couleur (protocole ou estompage).
    pub color: Rgb,
    /// Arête courbe (faisceau d'arêtes parallèles).
    pub curved: bool,
    /// Courbure d'une arête courbe.
    pub curvature: f32,
    /// Plan : 1 pour la famille surlignée, dessinée au-dessus.
    pub z: u8,
}

/// Instantané complet d'une image.
#[derive(Clone, Debug, Default)]
pub struct GraphFrame {
    /// Nœuds, dans l'ordre d'insertion (rang = `NodeIndex::index`).
    pub nodes: Vec<FrameNode>,
    /// Identifiants des nœuds (départage des labels de même taille).
    pub node_ids: Vec<Arc<str>>,
    /// Libellé affichable de chaque nœud (absent s'il est estompé).
    pub node_labels: Vec<Option<Arc<str>>>,
    /// Arêtes, dans l'ordre d'insertion.
    pub edges: Vec<FrameEdge>,
    /// Libellé de chaque arête à ce niveau de zoom.
    pub edge_labels: Vec<Option<Arc<str>>>,
    /// Nœud survolé.
    pub hovered: Option<u32>,
    /// Nœud sélectionné.
    pub highlighted: Option<u32>,
    /// Emprise des nœuds visibles (cadrage `autoRescale`).
    pub bounds: Option<Bounds>,
}

/// Réutilise le libellé partagé `slot` s'il vaut déjà `text`.
fn reuse_label(slot: Option<&Option<Arc<str>>>, text: Option<&str>) -> Option<Arc<str>> {
    let text = text?;
    match slot {
        Some(Some(previous)) if previous.as_ref() == text => Some(previous.clone()),
        _ => Some(Arc::from(text)),
    }
}

impl GraphFrame {
    /// Recalcule l'instantané depuis le modèle d'interface. `positions`
    /// donne les positions du fil ForceAtlas2 pour ce graphe (rang =
    /// `NodeIndex::index`) ; `None` garde celles du modèle (graphe pas encore
    /// reçu par le fil, positions initiales identiques).
    pub fn refresh(&mut self, model: &GraphModel, positions: Option<&[(f32, f32)]>) {
        let previous_ids = std::mem::take(&mut self.node_ids);
        let previous_labels = std::mem::take(&mut self.node_labels);
        let previous_edge_labels = std::mem::take(&mut self.edge_labels);
        self.nodes.clear();
        self.edges.clear();
        self.hovered = None;
        self.highlighted = None;

        let mut bounds: Option<Bounds> = None;
        for (i, node) in model.nodes_render().enumerate() {
            let (x, y, visible) = match positions {
                Some(p) => match p.get(i) {
                    Some(&(x, y)) => (x, y, x.is_finite() && y.is_finite()),
                    None => (node.x, node.y, false),
                },
                None => (node.x, node.y, true),
            };
            if visible {
                bounds = Some(match bounds {
                    None => Bounds {
                        min_x: x,
                        min_y: y,
                        max_x: x,
                        max_y: y,
                    },
                    Some(b) => Bounds {
                        min_x: b.min_x.min(x),
                        min_y: b.min_y.min(y),
                        max_x: b.max_x.max(x),
                        max_y: b.max_y.max(y),
                    },
                });
            }
            if node.hovered {
                self.hovered = Some(i as u32);
            }
            if node.highlighted {
                self.highlighted = Some(i as u32);
            }
            self.nodes.push(FrameNode {
                index: node.index,
                x,
                y,
                size: node.size,
                fill: node.fill,
                border: node.border,
                hovered: node.hovered,
                highlighted: node.highlighted,
                dimmed: node.dimmed,
                visible,
            });
            let id = match previous_ids.get(i) {
                Some(previous) if previous.as_ref() == node.id => previous.clone(),
                _ => Arc::from(node.id),
            };
            self.node_ids.push(id);
            self.node_labels
                .push(reuse_label(previous_labels.get(i), node.label));
        }
        self.bounds = bounds;

        for (i, edge) in model.edges_render().enumerate() {
            let source = edge.source.index();
            let target = edge.target.index();
            let shown = |n: usize| self.nodes.get(n).is_some_and(|n| n.visible);
            if !shown(source) || !shown(target) {
                // Gardée dans l'instantané (rangs alignés sur le modèle) mais
                // jamais dessinée : extrémité pas encore placée.
                self.edges.push(FrameEdge {
                    index: edge.index,
                    source: source as u32,
                    target: target as u32,
                    size: 0.0,
                    color: edge.color,
                    curved: false,
                    curvature: 0.0,
                    z: edge.z_index,
                });
                self.edge_labels.push(None);
                continue;
            }
            self.edges.push(FrameEdge {
                index: edge.index,
                source: source as u32,
                target: target as u32,
                size: edge.size,
                color: edge.color,
                curved: edge.kind == EdgeKind::Curved,
                curvature: edge.curvature,
                z: edge.z_index,
            });
            self.edge_labels
                .push(reuse_label(previous_edge_labels.get(i), edge.label));
        }
    }

    /// L'arête `edge` est dessinable (ses deux extrémités sont placées).
    pub fn edge_drawable(&self, edge: &FrameEdge) -> bool {
        edge.size > 0.0
            && edge.source != edge.target
            && self
                .nodes
                .get(edge.source as usize)
                .is_some_and(|n| n.visible)
            && self
                .nodes
                .get(edge.target as usize)
                .is_some_and(|n| n.visible)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use lunar_flows_core::graph::{Edge, GraphData, Node};

    /// Nœud de test (couleur et libellé fixés).
    pub(crate) fn node(id: &str, color: &'static str) -> Node {
        let mut n = Node::new(
            id,
            id.to_string(),
            String::new(),
            color,
            String::new(),
            None,
            None,
        );
        n.id = id.to_string();
        n
    }

    /// Arête de test.
    pub(crate) fn edge(source: &str, target: &str, protocol: &str, bytes: u64) -> Edge {
        Edge::new(
            &format!("{source}:{target}:{protocol}"),
            source.to_string(),
            target.to_string(),
        )
        .with_label(protocol.to_string())
        .with_traffic(1, bytes)
    }

    /// Snapshot de test.
    pub(crate) fn snapshot(nodes: &[Node], edges: &[Edge]) -> GraphData {
        let mut data = GraphData::new();
        for n in nodes {
            data.nodes.insert(n.id.clone(), n.clone());
        }
        for (i, e) in edges.iter().enumerate() {
            data.edges.insert(format!("e{i:04}"), e.clone());
        }
        data
    }

    #[test]
    fn positions_du_fil_et_noeuds_pas_encore_places() {
        let mut model = GraphModel::with_seed(1);
        model.load_snapshot(&snapshot(
            &[
                node("a", "#ff0000"),
                node("b", "#00ff00"),
                node("c", "#0000ff"),
            ],
            &[edge("a", "b", "TCP", 10_000), edge("b", "c", "UDP", 10_000)],
        ));
        let mut frame = GraphFrame::default();

        frame.refresh(&model, None);
        assert_eq!(frame.nodes.len(), 3);
        assert!(frame.nodes.iter().all(|n| n.visible));
        assert_eq!(frame.node_labels[0].as_deref(), Some("a"));
        assert!(frame.edges.iter().all(|e| frame.edge_drawable(e)));

        // Le fil n'a placé que deux nœuds : le troisième et son arête
        // attendent.
        frame.refresh(&model, Some(&[(1.0, 2.0), (3.0, 4.0)]));
        assert_eq!((frame.nodes[0].x, frame.nodes[0].y), (1.0, 2.0));
        assert!(!frame.nodes[2].visible);
        assert!(frame.edge_drawable(&frame.edges[0]));
        assert!(!frame.edge_drawable(&frame.edges[1]));
        let b = frame.bounds.expect("emprise");
        assert_eq!((b.min_x, b.max_y), (1.0, 4.0));
    }

    #[test]
    fn libelles_reutilises_d_une_image_a_l_autre() {
        let mut model = GraphModel::with_seed(1);
        model.load_snapshot(&snapshot(&[node("a", "#ff0000")], &[]));
        let mut frame = GraphFrame::default();
        frame.refresh(&model, None);
        let first = frame.node_labels[0].clone().expect("libellé");
        frame.refresh(&model, None);
        let second = frame.node_labels[0].clone().expect("libellé");
        assert!(Arc::ptr_eq(&first, &second));
    }
}
