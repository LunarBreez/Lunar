//! Mutations du graphe à partir des données du cœur : upserts de nœuds et
//! d'arêtes, tailles proportionnelles au trafic et courbure des arêtes
//! parallèles.
//!
//! L'indexation des arêtes parallèles suit `indexParallelEdgesIndex`
//! (`@sigma/edge-curve` 3.1.0). Le `match` sur `GraphUpdate` est exhaustif
//! à la compilation.

use lunar_flows_core::graph::{Edge, Node};

use super::store::{EdgeIndex, EdgeKind, GraphEdge, GraphNode, GraphStore, NodeIndex};
use super::style::{
    DEFAULT_JITTER_RADIUS, NODE_SIZE_MIN, NodeStyle, edge_attributes, edge_key, edge_label_for,
    get_curvature, jitter_around, node_attributes, node_size_for,
};

/// Nombre de nœuds échantillonnés pour l'ancrage d'apparition : suffisant
/// pour retomber « raisonnablement près » du cluster existant (ForceAtlas2
/// corrige l'écart ensuite), sans coût O(n) par nouveau nœud — O(n²) cumulé
/// sur une capture longue avec beaucoup d'hôtes.
pub const SPAWN_ANCHOR_SAMPLE_SIZE: usize = 200;

/// Rayon du jitter d'apparition d'un nouveau nœud autour de l'ancrage.
pub const SPAWN_JITTER_RADIUS: f32 = 200.0;

/// Taille d'un nœud de snapshot tant qu'aucun trafic ne l'a dimensionné : la
/// taille par défaut, 2.
pub const UNSIZED_NODE_SIZE: f32 = 2.0;

impl GraphStore {
    /// Barycentre approché des nœuds existants (point d'apparition des
    /// nouveaux), calculé sur les [`SPAWN_ANCHOR_SAMPLE_SIZE`] premiers
    /// nœuds plutôt que sur tout le graphe.
    pub fn spawn_anchor(&self) -> (f32, f32) {
        let sample = &self.nodes[..self.nodes.len().min(SPAWN_ANCHOR_SAMPLE_SIZE)];
        if sample.is_empty() {
            return (0.0, 0.0);
        }
        let (sx, sy) = sample.iter().fold((0.0f64, 0.0f64), |(sx, sy), n| {
            (sx + f64::from(n.x), sy + f64::from(n.y))
        });
        let n = sample.len() as f64;
        ((sx / n) as f32, (sy / n) as f32)
    }

    /// Insère un nœud absent du graphe.
    fn insert_node(
        &mut self,
        id: String,
        style: NodeStyle,
        x: f32,
        y: f32,
        size: f32,
        spawned: bool,
    ) {
        let index = NodeIndex(self.nodes.len() as u32);
        self.node_ids.insert(id.clone(), index);
        self.nodes.push(GraphNode {
            id,
            style,
            x,
            y,
            size,
            traffic_bytes: 0,
            spawned,
            in_edges: Vec::new(),
            out_edges: Vec::new(),
        });
        self.touch_positions();
    }

    /// Ajoute ou met à jour un nœud. Retourne vrai si le nœud a été ajouté :
    /// il apparaît alors près de [`Self::spawn_anchor`], à la taille minimale.
    pub fn upsert_node(&mut self, node: &Node) -> bool {
        if node.id.is_empty() {
            return false;
        }
        let style = node_attributes(node);
        if let Some(index) = self.node_index(&node.id)
            && let Some(existing) = self.node_mut(index)
        {
            existing.style = style;
            return false;
        }
        let (ax, ay) = self.spawn_anchor();
        let (x, y) = jitter_around(ax, ay, SPAWN_JITTER_RADIUS, &mut self.rng);
        self.insert_node(node.id.clone(), style, x, y, NODE_SIZE_MIN, true);
        true
    }

    /// Ajoute un nœud de snapshot à la position donnée, sous l'identifiant
    /// `id` (`node.id` à défaut de la clé du snapshot). Sans taille tant
    /// qu'une arête n'a pas apporté de trafic ([`UNSIZED_NODE_SIZE`]).
    ///
    /// Pour un identifiant déjà présent, le style est mis à jour et la
    /// position d'origine conservée. Retourne vrai si le
    /// nœud a été ajouté.
    pub fn add_snapshot_node(&mut self, id: &str, node: &Node, x: f32, y: f32) -> bool {
        let style = if node.id == id {
            node_attributes(node)
        } else {
            let mut renamed = node.clone();
            renamed.id = id.to_string();
            node_attributes(&renamed)
        };
        if let Some(index) = self.node_index(id)
            && let Some(existing) = self.node_mut(index)
        {
            existing.style = style;
            return false;
        }
        self.insert_node(id.to_string(), style, x, y, UNSIZED_NODE_SIZE, false);
        true
    }

    /// Taille du nœud proportionnelle (log) au trafic cumulé de ses arêtes,
    /// maintenue par delta (`_trafficBytes`) plutôt que par un rebalayage
    /// O(degré) à chaque upsert — O(degré²) cumulé sur un nœud « hub ». Sûr :
    /// les arêtes ne sont jamais retirées individuellement.
    fn apply_node_traffic_delta(&mut self, node: NodeIndex, previous: u64, current: u64) {
        if previous == current {
            return;
        }
        let Some(n) = self.node_mut(node) else { return };
        n.traffic_bytes = n
            .traffic_bytes
            .saturating_add(current)
            .saturating_sub(previous);
        n.size = node_size_for(n.traffic_bytes);
    }

    /// Ajoute ou met à jour une arête. Retourne vrai si l'arête a été ajoutée.
    ///
    /// Une arête orpheline (extrémité absente) est refusée. Un nœud
    /// fraîchement apparu relié à un nœud déjà placé est déplacé à côté de
    /// lui : ForceAtlas2 n'a plus qu'un ajustement local à faire.
    pub fn upsert_edge(&mut self, e: &Edge) -> bool {
        if e.source.is_empty() || e.target.is_empty() {
            return false;
        }
        let (Some(source), Some(target)) = (self.node_index(&e.source), self.node_index(&e.target))
        else {
            return false;
        };

        let key = edge_key(&e.source, &e.target, &e.label);
        let style = edge_attributes(e);
        let current_bytes = style.total_bytes;
        let port_label = edge_label_for(style.label_source(), true);

        if let Some(index) = self.edge_index(&key) {
            // Le cœur envoie un compteur cumulé par arête : le delta face à
            // l'ancienne valeur suffit à tenir la taille des nœuds à jour.
            let Some(existing) = self.edges.get_mut(index.index()) else {
                return false;
            };
            let previous_bytes = existing.style.total_bytes;
            existing.style = style;
            existing.port_label = port_label;
            self.apply_node_traffic_delta(source, previous_bytes, current_bytes);
            self.apply_node_traffic_delta(target, previous_bytes, current_bytes);
            return false;
        }

        let index = EdgeIndex(self.edges.len() as u32);
        self.edge_keys.insert(key.clone(), index);
        self.edges.push(GraphEdge {
            key,
            source,
            target,
            style,
            kind: EdgeKind::Straight,
            curvature: 0.0,
            port_label,
            parallel_index: None,
            parallel_min_index: None,
            parallel_max_index: None,
        });
        if let Some(n) = self.node_mut(source) {
            n.out_edges.push(index);
        }
        if let Some(n) = self.node_mut(target) {
            n.in_edges.push(index);
        }
        self.apply_node_traffic_delta(source, 0, current_bytes);
        self.apply_node_traffic_delta(target, 0, current_bytes);

        let spawned = |store: &Self, i: NodeIndex| store.node(i).is_some_and(|n| n.spawned);
        let (src_spawned, dst_spawned) = (spawned(self, source), spawned(self, target));
        if src_spawned != dst_spawned {
            let (fresh, settled) = if src_spawned {
                (source, target)
            } else {
                (target, source)
            };
            if let Some((sx, sy)) = self.node(settled).map(|n| (n.x, n.y)) {
                let (x, y) = jitter_around(sx, sy, DEFAULT_JITTER_RADIUS, &mut self.rng);
                if let Some(n) = self.node_mut(fresh) {
                    n.x = x;
                    n.y = y;
                    n.spawned = false;
                }
                self.touch_positions();
            }
        }
        true
    }

    /// Recalcule type et courbure des arêtes parallèles (multi-protocoles),
    /// d'après `indexParallelEdgesIndex`, suivi d'un post-traitement.
    pub fn refresh_parallel_edges(&mut self) {
        // Arêtes triées par (source, cible), ordre d'insertion conservé dans
        // chaque faisceau (tri stable).
        let mut bundles = std::mem::take(&mut self.parallel_scratch);
        bundles.clear();
        bundles.extend(
            self.edges
                .iter()
                .enumerate()
                .map(|(i, e)| (e.source.0, e.target.0, i as u32)),
        );
        bundles.sort_by_key(|&(s, t, _)| (s, t));
        let count_of = |s: u32, t: u32| {
            let lo = bundles.partition_point(|&(a, b, _)| (a, b) < (s, t));
            let hi = bundles.partition_point(|&(a, b, _)| (a, b) <= (s, t));
            hi - lo
        };

        let mut start = 0;
        while start < bundles.len() {
            let (s, t, _) = bundles[start];
            let end = start
                + bundles[start..]
                    .iter()
                    .take_while(|&&(a, b, _)| (a, b) == (s, t))
                    .count();
            let directed = end - start;
            // Une boucle a la même clé dans les deux sens.
            let undirected = if s == t {
                directed
            } else {
                directed + count_of(t, s)
            };
            for (i, &(_, _, e)) in bundles[start..end].iter().enumerate() {
                let Some(edge) = self.edges.get_mut(e as usize) else {
                    continue;
                };
                if directed == 1 && undirected == 1 {
                    // Arête seule dans les deux sens.
                    edge.parallel_index = None;
                    edge.parallel_max_index = None;
                } else if directed == 1 {
                    // Seule dans son sens, mais au moins une en sens inverse.
                    edge.parallel_index = Some(1.0);
                    edge.parallel_max_index = Some(1.0);
                } else if directed == undirected {
                    // Plusieurs arêtes, toutes de même sens.
                    let max = (directed - 1) as f32 / 2.0;
                    edge.parallel_index = Some(-max + i as f32);
                    edge.parallel_min_index = Some(-max);
                    edge.parallel_max_index = Some(max);
                } else {
                    // Plusieurs arêtes, dans les deux sens.
                    edge.parallel_index = Some((i + 1) as f32);
                    edge.parallel_max_index = Some(directed as f32);
                }
            }
            start = end;
        }
        self.parallel_scratch = bundles;

        for edge in &mut self.edges {
            let max_index = edge.parallel_max_index.unwrap_or(0.0);
            if edge.parallel_min_index.is_some() {
                // `parallelIndex ? … : …` : 0 (l'arête centrale) reste droite.
                match edge.parallel_index.filter(|&i| i != 0.0 && !i.is_nan()) {
                    Some(index) => {
                        edge.kind = EdgeKind::Curved;
                        edge.curvature = get_curvature(index, max_index);
                    }
                    None => {
                        edge.kind = EdgeKind::Straight;
                        edge.curvature = 0.0;
                    }
                }
            } else if let Some(index) = edge.parallel_index {
                edge.kind = EdgeKind::Curved;
                edge.curvature = get_curvature(index, max_index);
            } else {
                edge.kind = EdgeKind::Straight;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::rng::Rng;
    use super::super::style::tests::{edge, node};
    use super::super::style::{DEFAULT_EDGE_CURVATURE, edge_size_for};
    use super::*;

    fn new_graph() -> GraphStore {
        GraphStore::new(Rng::with_seed(3))
    }

    #[test]
    fn upsert_node_ajoute_un_noeud_avec_ses_attributs_derives() {
        let mut g = new_graph();
        let mut n = node("192.168.1.1");
        n.name = "hote-a".into();
        n.mac = "aa:bb".into();
        n.ip = "192.168.1.1".into();
        let added = g.upsert_node(&n);

        assert!(added);
        assert_eq!(g.order(), 1);
        let attrs = g.node_by_id("192.168.1.1").expect("nœud");
        assert_eq!(attrs.style.name, "hote-a");
        // Sans label utilisateur, le label affiché retombe sur le nom.
        assert_eq!(attrs.style.label, "hote-a");
        assert!(attrs.x.is_finite() && attrs.y.is_finite());
        assert_eq!(attrs.size, NODE_SIZE_MIN);
    }

    #[test]
    fn upsert_node_un_noeud_existant_est_fusionne_pas_duplique() {
        let mut g = new_graph();
        let mut n = node("n1");
        n.name = "avant".into();
        g.upsert_node(&n);
        let (x, y) = g.node_by_id("n1").map(|n| (n.x, n.y)).expect("nœud");
        n.label = Some("mon-pc".into());
        let added = g.upsert_node(&n);

        assert!(!added);
        assert_eq!(g.order(), 1);
        let merged = g.node_by_id("n1").expect("nœud");
        assert_eq!(merged.style.label, "mon-pc");
        // La fusion ne touche ni à la position ni à la taille.
        assert_eq!((merged.x, merged.y), (x, y));
    }

    #[test]
    fn upsert_node_id_manquant_refuse() {
        let mut g = new_graph();
        assert!(!g.upsert_node(&node("")));
        assert_eq!(g.order(), 0);
    }

    #[test]
    fn upsert_node_apparait_pres_de_l_ancrage() {
        let mut g = new_graph();
        g.upsert_node(&node("a"));
        let first = g.node_by_id("a").map(|n| (n.x, n.y)).expect("nœud");
        // Premier nœud : ancrage (0, 0), couronne [80, 200].
        let d = f64::from(first.0).hypot(f64::from(first.1));
        assert!((80.0 - 1e-3..=200.0 + 1e-3).contains(&d), "{d}");
        assert_eq!(g.spawn_anchor(), first);
    }

    #[test]
    fn spawn_anchor_echantillonne_les_200_premiers_noeuds() {
        let mut g = new_graph();
        for i in 0..250 {
            let x = if i < SPAWN_ANCHOR_SAMPLE_SIZE {
                10.0
            } else {
                1000.0
            };
            g.add_snapshot_node(&format!("n{i}"), &node(&format!("n{i}")), x, -4.0);
        }
        assert_eq!(g.spawn_anchor(), (10.0, -4.0));
    }

    #[test]
    fn upsert_edge_refuse_les_aretes_orphelines() {
        let mut g = new_graph();
        g.upsert_node(&node("a"));
        let added = g.upsert_edge(&edge("a", "fantome", "TCP"));

        assert!(!added);
        assert_eq!(g.size(), 0);
        assert!(!g.upsert_edge(&edge("", "a", "TCP")));
    }

    #[test]
    fn upsert_edge_ajoute_puis_fusionne_sur_la_cle() {
        let mut g = new_graph();
        g.upsert_node(&node("a"));
        g.upsert_node(&node("b"));

        let mut e = edge("a", "b", "TCP");
        e.count = 1;
        e.total_bytes = 100;
        let first = g.upsert_edge(&e);
        e.count = 5;
        e.total_bytes = 900;
        let second = g.upsert_edge(&e);

        assert!(first);
        assert!(!second, "même flux -> fusion, pas de doublon");
        assert_eq!(g.size(), 1);
        // Le cœur envoie des compteurs cumulés : la fusion remplace les valeurs.
        let merged = g.edge_by_key("a__b__TCP").expect("arête");
        assert_eq!(merged.style.total_bytes, 900);
        assert_eq!(merged.style.count, 5);
        assert_eq!(merged.style.size, edge_size_for(900));
    }

    #[test]
    fn upsert_edge_deux_protocoles_entre_les_memes_hotes_deux_aretes() {
        let mut g = new_graph();
        g.upsert_node(&node("a"));
        g.upsert_node(&node("b"));
        g.upsert_edge(&edge("a", "b", "TCP"));
        g.upsert_edge(&edge("a", "b", "DNS"));

        assert_eq!(g.size(), 2);
    }

    #[test]
    fn upsert_edge_la_taille_des_noeuds_suit_le_trafic_cumule() {
        let mut g = new_graph();
        g.upsert_node(&node("a"));
        g.upsert_node(&node("b"));
        let mut e = edge("a", "b", "TCP");
        e.total_bytes = 1;
        g.upsert_edge(&e);
        let small = g.node_by_id("a").expect("a").size;

        e.total_bytes = 50_000_000;
        g.upsert_edge(&e);
        let big = g.node_by_id("a").expect("a").size;

        assert!(big > small, "taille {big} devrait dépasser {small}");
        // Delta exact : le trafic du nœud est celui de l'arête, pas la somme.
        assert_eq!(big, node_size_for(50_000_000));
        assert_eq!(
            g.node_by_id("b").expect("b").size,
            node_size_for(50_000_000)
        );
    }

    #[test]
    fn upsert_edge_le_trafic_des_noeuds_cumule_leurs_aretes() {
        let mut g = new_graph();
        for id in ["a", "b", "c"] {
            g.upsert_node(&node(id));
        }
        let mut ab = edge("a", "b", "TCP");
        ab.total_bytes = 3000;
        let mut ac = edge("a", "c", "UDP");
        ac.total_bytes = 5000;
        g.upsert_edge(&ab);
        g.upsert_edge(&ac);
        assert_eq!(g.node_by_id("a").expect("a").size, node_size_for(8000));
        assert_eq!(g.node_by_id("c").expect("c").size, node_size_for(5000));
    }

    #[test]
    fn upsert_edge_le_noeud_apparu_rejoint_son_premier_voisin_place() {
        let mut g = new_graph();
        g.add_snapshot_node("place", &node("place"), 5000.0, 5000.0);
        g.upsert_node(&node("nouveau"));
        g.upsert_edge(&edge("nouveau", "place", "TCP"));

        let fresh = g.node_by_id("nouveau").expect("nœud");
        let d = f64::from(fresh.x - 5000.0).hypot(f64::from(fresh.y - 5000.0));
        assert!((48.0 - 1e-2..=120.0 + 1e-2).contains(&d), "{d}");
        assert!(!fresh.spawned);
        // Le nœud placé ne bouge pas.
        let settled = g.node_by_id("place").expect("nœud");
        assert_eq!((settled.x, settled.y), (5000.0, 5000.0));
    }

    #[test]
    fn upsert_edge_deux_noeuds_apparus_restent_en_place() {
        let mut g = new_graph();
        g.upsert_node(&node("a"));
        g.upsert_node(&node("b"));
        let before: Vec<_> = g.nodes().iter().map(|n| (n.x, n.y)).collect();
        g.upsert_edge(&edge("a", "b", "TCP"));
        let after: Vec<_> = g.nodes().iter().map(|n| (n.x, n.y)).collect();
        assert_eq!(before, after);
        assert!(g.nodes().iter().all(|n| n.spawned));
    }

    #[test]
    fn refresh_parallel_edges_les_aretes_paralleles_deviennent_courbes() {
        let mut g = new_graph();
        g.upsert_node(&node("a"));
        g.upsert_node(&node("b"));
        g.upsert_edge(&edge("a", "b", "TCP"));
        g.upsert_edge(&edge("a", "b", "DNS"));

        g.refresh_parallel_edges();

        let kinds: Vec<_> = g.edges().iter().map(|e| e.kind).collect();
        assert!(kinds.contains(&EdgeKind::Curved), "types: {kinds:?}");
    }

    #[test]
    fn refresh_parallel_edges_comme_index_parallel_edges_index() {
        let mut g = new_graph();
        for id in ["a", "b", "c"] {
            g.upsert_node(&node(id));
        }
        // Arête seule : droite.
        g.upsert_edge(&edge("a", "c", "TCP"));
        // Trois arêtes de même sens : -1, 0, 1 ; la centrale reste droite.
        for proto in ["TCP", "DNS", "TLS"] {
            g.upsert_edge(&edge("a", "b", proto));
        }
        // Une arête en sens inverse de c → a : index 1 / max 1 des deux côtés.
        g.upsert_edge(&edge("c", "b", "UDP"));
        g.upsert_edge(&edge("b", "c", "UDP"));
        g.refresh_parallel_edges();

        let by_key = |k: &str| g.edge_by_key(k).expect(k);
        let alone = by_key("a__c__TCP");
        assert_eq!((alone.kind, alone.curvature), (EdgeKind::Straight, 0.0));

        let max = get_curvature(1.0, 1.0);
        assert_eq!(by_key("a__b__TCP").kind, EdgeKind::Curved);
        assert_eq!(by_key("a__b__TCP").curvature, -max);
        assert_eq!(
            (by_key("a__b__DNS").kind, by_key("a__b__DNS").curvature),
            (EdgeKind::Straight, 0.0)
        );
        assert_eq!(by_key("a__b__TLS").curvature, max);

        for k in ["c__b__UDP", "b__c__UDP"] {
            assert_eq!(by_key(k).kind, EdgeKind::Curved);
            assert_eq!(by_key(k).curvature, get_curvature(1.0, 1.0));
        }

        // Une troisième arête b → c : faisceau à deux sens, index 1 et 2 sur 2.
        g.upsert_edge(&edge("b", "c", "DNS"));
        g.refresh_parallel_edges();
        let by_key = |k: &str| g.edge_by_key(k).expect(k);
        assert_eq!(by_key("b__c__UDP").curvature, get_curvature(1.0, 2.0));
        assert_eq!(by_key("b__c__DNS").curvature, get_curvature(2.0, 2.0));
        assert_eq!(by_key("c__b__UDP").curvature, get_curvature(1.0, 1.0));
    }

    #[test]
    fn refresh_parallel_edges_deux_aretes_de_meme_sens_de_part_et_d_autre() {
        let mut g = new_graph();
        g.upsert_node(&node("a"));
        g.upsert_node(&node("b"));
        g.upsert_edge(&edge("a", "b", "TCP"));
        g.upsert_edge(&edge("a", "b", "DNS"));
        g.refresh_parallel_edges();
        let c: Vec<_> = g.edges().iter().map(|e| e.curvature).collect();
        assert_eq!(c[0], -c[1]);
        assert!(c[1] > 0.0 && c[1] < DEFAULT_EDGE_CURVATURE);
    }

    #[test]
    fn node_edges_entrantes_puis_sortantes_boucle_vue_une_fois() {
        let mut g = new_graph();
        for id in ["a", "b", "c"] {
            g.upsert_node(&node(id));
        }
        g.upsert_edge(&edge("a", "b", "TCP"));
        g.upsert_edge(&edge("c", "a", "DNS"));
        g.upsert_edge(&edge("b", "a", "UDP"));
        g.upsert_edge(&edge("c", "a", "TLS"));
        g.upsert_edge(&edge("a", "a", "ARP"));
        let a = g.node_index("a").expect("a");
        let keys: Vec<_> = g
            .node_edges(a)
            .into_iter()
            .map(|e| g.edge(e).expect("arête").key.clone())
            .collect();
        assert_eq!(
            keys,
            [
                "c__a__TLS",
                "c__a__DNS",
                "b__a__UDP",
                "a__a__ARP",
                "a__b__TCP"
            ]
        );
    }
}
