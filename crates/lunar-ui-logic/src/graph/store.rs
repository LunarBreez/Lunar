//! Stockage du graphe affiché : multigraphe orienté réduit aux attributs
//! utiles au rendu.
//!
//! Les nœuds et arêtes vivent dans des vecteurs, dans leur ordre d'insertion ;
//! ils ne sont jamais retirés un par un, seul [`GraphStore::clear`] vide le
//! graphe. Un [`NodeIndex`] ou un
//! [`EdgeIndex`] reste donc valide jusqu'au prochain `clear`.

use std::collections::HashMap;

use super::camera::Bounds;
use super::rng::Rng;
use super::style::{EdgeStyle, NodeStyle};

/// Position d'un nœud dans le stockage (ordre d'insertion).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeIndex(pub(crate) u32);

impl NodeIndex {
    /// Rang du nœud, de 0 à `order() - 1`.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Position d'une arête dans le stockage (ordre d'insertion).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EdgeIndex(pub(crate) u32);

impl EdgeIndex {
    /// Rang de l'arête, de 0 à `size() - 1`.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Programme de rendu d'une arête.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EdgeKind {
    /// Segment droit avec flèche (`EdgeArrowProgram`), le défaut.
    #[default]
    Straight,
    /// Courbe de Bézier quadratique avec flèche (`EdgeCurvedArrowProgram`).
    Curved,
}

/// Nœud du graphe affiché.
#[derive(Clone, Debug)]
pub struct GraphNode {
    /// Identifiant du nœud (id stable du cœur).
    pub id: String,
    /// Attributs de style dérivés du `Node` du cœur.
    pub style: NodeStyle,
    /// Abscisse monde.
    pub x: f32,
    /// Ordonnée monde.
    pub y: f32,
    /// Rayon d'écran (pixels à un ratio de caméra de 1).
    pub size: f32,
    /// Trafic cumulé des arêtes du nœud (attribut interne `_trafficBytes`).
    pub(crate) traffic_bytes: u64,
    /// Nœud fraîchement apparu, pas encore rapproché d'un voisin placé
    /// (attribut interne `_spawned`).
    pub(crate) spawned: bool,
    /// Arêtes entrantes, dans l'ordre d'insertion.
    pub(crate) in_edges: Vec<EdgeIndex>,
    /// Arêtes sortantes, dans l'ordre d'insertion.
    pub(crate) out_edges: Vec<EdgeIndex>,
}

/// Arête du graphe affiché.
#[derive(Clone, Debug)]
pub struct GraphEdge {
    /// Clé `source__cible__protocole` (voir [`super::style::edge_key`]).
    pub key: String,
    /// Nœud source.
    pub source: NodeIndex,
    /// Nœud cible.
    pub target: NodeIndex,
    /// Attributs de style dérivés de l'`Edge` du cœur.
    pub style: EdgeStyle,
    /// Droite ou courbe (arêtes parallèles).
    pub kind: EdgeKind,
    /// Courbure d'une arête courbe (voir [`super::style::edge_control_point`]).
    pub curvature: f32,
    /// Libellé avec ports, précalculé à chaque mise à jour pour ne rien
    /// allouer par frame.
    pub(crate) port_label: String,
    /// Index de l'arête dans son faisceau.
    pub(crate) parallel_index: Option<f32>,
    /// Index minimal du faisceau, posé seulement pour
    /// un faisceau de même sens.
    pub(crate) parallel_min_index: Option<f32>,
    /// Index maximal du faisceau.
    pub(crate) parallel_max_index: Option<f32>,
}

impl GraphEdge {
    /// Libellé avec le détail des ports (zoom ≥ [`super::style::PORT_LABEL_ZOOM`]).
    pub fn port_label(&self) -> &str {
        &self.port_label
    }
}

/// Graphe affiché : nœuds, arêtes, index par identifiant et adjacence.
#[derive(Debug)]
pub struct GraphStore {
    /// Nœuds dans l'ordre d'insertion.
    pub(crate) nodes: Vec<GraphNode>,
    /// Arêtes dans l'ordre d'insertion.
    pub(crate) edges: Vec<GraphEdge>,
    /// Id de nœud → rang.
    pub(crate) node_ids: HashMap<String, NodeIndex>,
    /// Clé d'arête → rang.
    pub(crate) edge_keys: HashMap<String, EdgeIndex>,
    /// Générateur du placement des nouveaux nœuds.
    pub(crate) rng: Rng,
    /// Compteur de modifications des positions : un index spatial construit
    /// à une autre valeur est périmé.
    pub(crate) positions_version: u64,
    /// Tampon réutilisé par `refresh_parallel_edges`.
    pub(crate) parallel_scratch: Vec<(u32, u32, u32)>,
}

impl Default for GraphStore {
    fn default() -> Self {
        Self::new(Rng::from_entropy())
    }
}

impl GraphStore {
    /// Graphe vide, avec ce générateur pour le placement des nœuds.
    pub fn new(rng: Rng) -> Self {
        Self {
            nodes: Vec::new(),
            edges: Vec::new(),
            node_ids: HashMap::new(),
            edge_keys: HashMap::new(),
            rng,
            positions_version: 0,
            parallel_scratch: Vec::new(),
        }
    }

    /// Nombre de nœuds (`graph.order`).
    pub fn order(&self) -> usize {
        self.nodes.len()
    }

    /// Nombre d'arêtes (`graph.size`).
    pub fn size(&self) -> usize {
        self.edges.len()
    }

    /// Vide le graphe (`graph.clear()`) ; les index précédents sont caducs.
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.edges.clear();
        self.node_ids.clear();
        self.edge_keys.clear();
        self.touch_positions();
    }

    /// Rang du nœud d'identifiant `id`.
    pub fn node_index(&self, id: &str) -> Option<NodeIndex> {
        self.node_ids.get(id).copied()
    }

    /// Rang de l'arête de clé `key`.
    pub fn edge_index(&self, key: &str) -> Option<EdgeIndex> {
        self.edge_keys.get(key).copied()
    }

    /// Vrai si un nœud porte cet identifiant (`graph.hasNode`).
    pub fn has_node(&self, id: &str) -> bool {
        self.node_ids.contains_key(id)
    }

    /// Vrai si une arête porte cette clé (`graph.hasEdge`).
    pub fn has_edge(&self, key: &str) -> bool {
        self.edge_keys.contains_key(key)
    }

    /// Nœud de rang `index`.
    pub fn node(&self, index: NodeIndex) -> Option<&GraphNode> {
        self.nodes.get(index.index())
    }

    /// Arête de rang `index`.
    pub fn edge(&self, index: EdgeIndex) -> Option<&GraphEdge> {
        self.edges.get(index.index())
    }

    /// Nœud d'identifiant `id`.
    pub fn node_by_id(&self, id: &str) -> Option<&GraphNode> {
        self.node_index(id).and_then(|i| self.node(i))
    }

    /// Arête de clé `key`.
    pub fn edge_by_key(&self, key: &str) -> Option<&GraphEdge> {
        self.edge_index(key).and_then(|i| self.edge(i))
    }

    /// Nœuds dans l'ordre d'insertion.
    pub fn nodes(&self) -> &[GraphNode] {
        &self.nodes
    }

    /// Arêtes dans l'ordre d'insertion.
    pub fn edges(&self) -> &[GraphEdge] {
        &self.edges
    }

    /// Arêtes touchant `node` : entrantes puis sortantes, regroupées par voisin dans l'ordre de leur
    /// première apparition, la plus récente d'abord dans chaque groupe ; une boucle n'est vue qu'une fois.
    pub fn node_edges(&self, node: NodeIndex) -> Vec<EdgeIndex> {
        let Some(n) = self.node(node) else {
            return Vec::new();
        };
        let mut out = Vec::with_capacity(n.in_edges.len() + n.out_edges.len());
        self.push_grouped_by_neighbor(&n.in_edges, |e| e.source, None, &mut out);
        self.push_grouped_by_neighbor(&n.out_edges, |e| e.target, Some(node), &mut out);
        out
    }

    /// Ajoute `edges` à `out`, regroupées par voisin (la plus récente en
    /// tête) ; `skip` écarte les
    /// boucles déjà vues.
    fn push_grouped_by_neighbor(
        &self,
        edges: &[EdgeIndex],
        neighbor: impl Fn(&GraphEdge) -> NodeIndex,
        skip: Option<NodeIndex>,
        out: &mut Vec<EdgeIndex>,
    ) {
        let mut order: Vec<NodeIndex> = Vec::new();
        let mut groups: HashMap<NodeIndex, Vec<EdgeIndex>> = HashMap::new();
        for &e in edges {
            let Some(edge) = self.edge(e) else { continue };
            let other = neighbor(edge);
            if Some(other) == skip {
                continue;
            }
            groups
                .entry(other)
                .or_insert_with(|| {
                    order.push(other);
                    Vec::new()
                })
                .push(e);
        }
        for other in order {
            if let Some(group) = groups.remove(&other) {
                out.extend(group.into_iter().rev());
            }
        }
    }

    /// Nœud de rang `index`, modifiable.
    pub(crate) fn node_mut(&mut self, index: NodeIndex) -> Option<&mut GraphNode> {
        self.nodes.get_mut(index.index())
    }

    /// Signale un changement de positions (index spatial à reconstruire).
    pub(crate) fn touch_positions(&mut self) {
        self.positions_version = self.positions_version.wrapping_add(1);
    }

    /// Version courante des positions.
    pub fn positions_version(&self) -> u64 {
        self.positions_version
    }

    /// Emprise des positions des nœuds (`None` pour un graphe vide).
    pub fn bounds(&self) -> Option<Bounds> {
        let first = self.nodes.first()?;
        let init = Bounds {
            min_x: first.x,
            min_y: first.y,
            max_x: first.x,
            max_y: first.y,
        };
        Some(self.nodes.iter().fold(init, |b, n| Bounds {
            min_x: b.min_x.min(n.x),
            min_y: b.min_y.min(n.y),
            max_x: b.max_x.max(n.x),
            max_y: b.max_y.max(n.y),
        }))
    }
}
