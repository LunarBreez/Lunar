//! Modèle du graphe réseau, sans fenêtre : file d'updates, chargement de
//! snapshot, reducers de survol/sélection/tunnel, zoom, bouton « Gravité »,
//! édition de label. Ni rendu ni appel au moteur.
//!
//! Un moteur de rendu l'utilise ainsi, à chaque frame :
//!
//! 1. [`GraphModel::enqueue`] reçoit les `GraphUpdate` au fil de l'eau ;
//!    [`GraphModel::flush`] les applique une fois par frame ;
//! 2. [`GraphModel::step_layout`] fait avancer ForceAtlas2 et relève l'arrêt
//!    automatique ;
//! 3. [`GraphModel::nodes_render`] et [`GraphModel::edges_render`] donnent,
//!    sans allocation, ce qu'il faut dessiner, reducers appliqués ;
//! 4. les événements pointeur passent par [`GraphModel::node_at`] et
//!    [`GraphModel::edge_at`], puis par les méthodes `hover_node`,
//!    `click_node`, `enter_edge`…
//!
//! Les appels au moteur (`get_graph_snapshot`, `add_label`) restent à
//! l'appelant : [`GraphModel::restore_snapshot`] reçoit le snapshot obtenu,
//! [`GraphModel::begin_label_edit`] rend la requête à envoyer au cœur et
//! [`GraphModel::commit_label_edit`] / [`GraphModel::fail_label_edit`] en
//! appliquent l'issue.

use lunar_flows_core::graph::{GraphData, GraphUpdate};

use super::camera::{Bounds, EdgeLabelVisibility, zoom_for_ratio};
use super::counts::graph_counts_label;
use super::fa2_supervisor::Fa2Supervisor;
use super::force_layout::{Clock, ForceLayoutController};
use super::format::{js_trim, to_precision};
use super::label_edit::{LabelRollback, apply_optimistic_label};
use super::node_info::{NodeSnapshot, build_node_infos, node_snapshot};
use super::rng::Rng;
use super::spatial::SpatialGrid;
use super::store::{EdgeIndex, EdgeKind, GraphStore, NodeIndex};
use super::style::{DIM_EDGE_RGB, DIM_NODE_RGB, Rgb, edge_control_point};
use super::tunnel::{TunnelHighlight, compute_tunnel_highlight, edge_has_tunnel};

/// Rayon de la couronne initiale d'un snapshot de `n` nœuds :
/// `100 + √n × 40`.
pub fn snapshot_ring_radius(n: usize) -> f64 {
    100.0 + (n as f64).sqrt() * 40.0
}

/// Amplitude totale du jitter de la couronne initiale (± 15).
pub const SNAPSHOT_RING_JITTER: f64 = 30.0;

/// Segments de l'approximation d'une arête courbe pour le test d'atteinte.
const CURVE_HIT_SEGMENTS: usize = 16;

/// Élément ajouté par une update : nœud ou arête.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Added {
    /// Un nœud.
    Node,
    /// Une arête.
    Edge,
}

/// Bilan d'un chargement de snapshot, à journaliser.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SnapshotLoad {
    /// Nœuds du snapshot.
    pub nodes: usize,
    /// Arêtes du snapshot.
    pub edges: usize,
    /// Arêtes sans source ou sans cible, ignorées.
    pub invalid_edges: usize,
    /// Arêtes dont une extrémité est absente, ignorées.
    pub orphan_edges: usize,
}

/// Bilan d'un pas de mise en page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LayoutStep {
    /// L'arrêt automatique vient de couper la gravité.
    pub auto_stopped: bool,
    /// Des positions ont changé.
    pub moved: bool,
}

/// Requête d'édition de label à transmettre au cœur (`add_label`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LabelEditRequest {
    /// Nœud édité.
    pub node_id: String,
    /// MAC du nœud.
    pub mac: String,
    /// IP du nœud.
    pub ip: String,
    /// Nouveau label, sans blancs de bord.
    pub label: String,
    /// Retour arrière si le cœur refuse.
    rollback: LabelRollback,
    /// Sélection d'avant l'édition, restaurée en cas d'échec.
    previous: NodeSnapshot,
}

/// Nœud à dessiner, reducer appliqué.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NodeRender<'a> {
    /// Rang du nœud.
    pub index: NodeIndex,
    /// Identifiant.
    pub id: &'a str,
    /// Abscisse monde.
    pub x: f32,
    /// Ordonnée monde.
    pub y: f32,
    /// Rayon d'écran (voir [`super::camera::screen_size`]).
    pub size: f32,
    /// Remplissage : estompé hors famille de tunnel, éclairci au survol.
    pub fill: Rgb,
    /// Bordure (épaisseur [`super::style::NODE_BORDER_SIZE_RATIO`]).
    pub border: Rgb,
    /// Libellé, absent pour un nœud estompé.
    pub label: Option<&'a str>,
    /// Nœud survolé (son label est alors dessiné en surimpression).
    pub hovered: bool,
    /// Nœud sélectionné (`highlighted` : label toujours dessiné).
    pub highlighted: bool,
    /// Nœud estompé (hors de la famille de tunnel surlignée).
    pub dimmed: bool,
}

/// Arête à dessiner, reducer appliqué.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeRender<'a> {
    /// Rang de l'arête.
    pub index: EdgeIndex,
    /// Nœud source.
    pub source: NodeIndex,
    /// Nœud cible (porte la flèche).
    pub target: NodeIndex,
    /// Position monde de la source.
    pub source_pos: (f32, f32),
    /// Position monde de la cible.
    pub target_pos: (f32, f32),
    /// Rayon d'écran de la cible (la flèche s'arrête à sa bordure).
    pub target_size: f32,
    /// Épaisseur d'écran, +1 dans une famille de tunnel surlignée.
    pub size: f32,
    /// Couleur du protocole, ou estompée.
    pub color: Rgb,
    /// Droite ou courbe.
    pub kind: EdgeKind,
    /// Courbure d'une arête courbe (voir [`edge_control_point`]).
    pub curvature: f32,
    /// Plan de dessin : 1 pour la famille surlignée, 0 sinon.
    pub z_index: u8,
    /// Arête estompée.
    pub dimmed: bool,
    /// Libellé à ce niveau de zoom.
    pub label: Option<&'a str>,
}

/// Modèle du graphe réseau, tenu par la vue du graphe.
pub struct GraphModel {
    /// Graphe affiché.
    store: GraphStore,
    /// Contrôleur ForceAtlas2.
    layout: ForceLayoutController<Fa2Supervisor>,
    /// Bouton « Gravité ».
    force_enabled: bool,
    /// Zoom courant (`1 / ratio`).
    zoom: f32,
    /// Seuils de labels d'arêtes franchis.
    labels: EdgeLabelVisibility,
    /// Nœud survolé.
    hovered_node: Option<NodeIndex>,
    /// Famille de tunnel surlignée (survol ou épinglage d'une arête).
    tunnel: Option<TunnelHighlight>,
    /// Arête épinglée au clic : la surbrillance reste après le survol.
    pinned_tunnel_edge: Option<EdgeIndex>,
    /// Nœud sélectionné.
    selected: Option<NodeIndex>,
    /// Snapshot du nœud sélectionné (bandeau bas).
    selected_node: Option<NodeSnapshot>,
    /// Lignes d'infos du nœud sélectionné.
    selected_node_infos: Vec<String>,
    /// Saisie du champ de label.
    edited_label: String,
    /// Enregistrement d'un label en cours.
    is_saving_label: bool,
    /// Updates en attente du prochain `flush`.
    queue: Vec<GraphUpdate>,
    /// Recadrage demandé après un chargement.
    camera_reset_requested: bool,
    /// Index spatial des nœuds.
    grid: SpatialGrid,
    /// Version des positions indexées.
    grid_version: Option<u64>,
}

impl Default for GraphModel {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for GraphModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GraphModel")
            .field("order", &self.store.order())
            .field("size", &self.store.size())
            .field("force_enabled", &self.force_enabled)
            .field("zoom", &self.zoom)
            .field("pending_updates", &self.queue.len())
            .finish_non_exhaustive()
    }
}

impl GraphModel {
    /// Modèle vide, placement aléatoire et horloge système.
    pub fn new() -> Self {
        Self::with_rng_and_clock(Rng::from_entropy(), Box::new(std::time::Instant::now))
    }

    /// Modèle vide au placement déterministe (graine fixe).
    pub fn with_seed(seed: u64) -> Self {
        Self::with_rng_and_clock(Rng::with_seed(seed), Box::new(std::time::Instant::now))
    }

    /// Modèle vide avec ce générateur et cette horloge (arrêt automatique).
    pub fn with_rng_and_clock(rng: Rng, clock: Clock) -> Self {
        let layout =
            ForceLayoutController::new(Box::new(Fa2Supervisor::for_order)).with_clock(clock);
        Self {
            store: GraphStore::new(rng),
            layout,
            force_enabled: true,
            zoom: 1.0,
            labels: EdgeLabelVisibility::default(),
            hovered_node: None,
            tunnel: None,
            pinned_tunnel_edge: None,
            selected: None,
            selected_node: None,
            selected_node_infos: Vec::new(),
            edited_label: String::new(),
            is_saving_label: false,
            queue: Vec::new(),
            camera_reset_requested: false,
            grid: SpatialGrid::default(),
            grid_version: None,
        }
    }

    /// Graphe affiché.
    pub fn store(&self) -> &GraphStore {
        &self.store
    }

    // === Updates ============================================================

    /// Met une update en file pour le prochain [`Self::flush`].
    pub fn enqueue(&mut self, update: GraphUpdate) {
        self.queue.push(update);
    }

    /// Met plusieurs updates en file.
    pub fn enqueue_all(&mut self, updates: impl IntoIterator<Item = GraphUpdate>) {
        self.queue.extend(updates);
    }

    /// Updates en attente.
    pub fn pending_updates(&self) -> usize {
        self.queue.len()
    }

    /// Applique la file (une fois par frame). Retourne vrai si
    /// la file n'était pas vide.
    pub fn flush(&mut self) -> bool {
        if self.queue.is_empty() {
            return false;
        }
        let queue = std::mem::take(&mut self.queue);
        let mut added_nodes = false;
        let mut added_edges = false;
        for update in &queue {
            match self.apply_update(update) {
                Some(Added::Node) => added_nodes = true,
                Some(Added::Edge) => added_edges = true,
                None => {}
            }
        }
        // Rend son tampon à la file (vidée entre-temps : rien ne l'alimente
        // pendant l'application).
        self.queue = queue;
        self.queue.clear();

        // Le superviseur ForceAtlas2 suit les ajouts (rechargement des
        // matrices) ; il reste à recalculer la courbure des arêtes parallèles.
        if (added_nodes || added_edges)
            && let Some(supervisor) = self.layout.supervisor_mut()
        {
            supervisor.graph_changed();
        }
        if added_edges {
            self.store.refresh_parallel_edges();
        }
        self.ensure_layout_settings();
        true
    }

    /// Applique une update ; indique ce qui a été ajouté.
    fn apply_update(&mut self, update: &GraphUpdate) -> Option<Added> {
        match update {
            GraphUpdate::NewNode(node) => self.store.upsert_node(node).then_some(Added::Node),
            GraphUpdate::NodeUpdated(node) => {
                let added = self.store.upsert_node(node);
                self.refresh_selected_node(&node.id);
                added.then_some(Added::Node)
            }
            GraphUpdate::NewEdge(edge) | GraphUpdate::EdgeUpdated(edge) => {
                self.store.upsert_edge(edge).then_some(Added::Edge)
            }
        }
    }

    /// Resynchronise le bandeau bas si le nœud mis à jour est sélectionné.
    fn refresh_selected_node(&mut self, node_id: &str) {
        if node_id.is_empty() || self.selected_node_id() != Some(node_id) {
            return;
        }
        let Some(snapshot) = node_snapshot(&self.store, node_id) else {
            return;
        };
        self.edited_label = snapshot.label.clone();
        self.selected_node_infos = build_node_infos(&self.store, node_id);
        self.selected_node = Some(snapshot);
    }

    // === Snapshot et réinitialisation ========================================

    /// Vide le graphe : mise en page détruite, sélection et
    /// surbrillance effacées, updates en attente abandonnées (sinon un update
    /// mis en file avant le reset serait rejoué sur le graphe rechargé).
    pub fn reset(&mut self) {
        self.layout.kill();
        self.store.clear();
        self.clear_selection();
        self.unpin_tunnel_highlight();
        // Les rangs vont être réattribués : un survol périmé désignerait un
        // autre nœud.
        self.hovered_node = None;
        self.queue.clear();
    }

    /// Recharge complètement le graphe depuis un snapshot : nœuds en couronne (rayon
    /// [`snapshot_ring_radius`], jitter ±15) que ForceAtlas2 démêle ensuite,
    /// arêtes orphelines ignorées, mise en page recréée pour la nouvelle
    /// taille et recadrage demandé.
    ///
    /// Les entrées sont prises par clé croissante (l'ordre d'un `HashMap` du
    /// cœur est arbitraire).
    pub fn load_snapshot(&mut self, snapshot: &GraphData) -> SnapshotLoad {
        self.reset();

        let mut node_entries: Vec<_> = snapshot.nodes.iter().collect();
        node_entries.sort_unstable_by(|a, b| a.0.cmp(b.0));
        let mut edge_entries: Vec<_> = snapshot.edges.iter().collect();
        edge_entries.sort_unstable_by(|a, b| a.0.cmp(b.0));

        let n = node_entries.len().max(1);
        let radius = snapshot_ring_radius(n);
        for (i, (key, node)) in node_entries.iter().enumerate() {
            let id = if node.id.is_empty() {
                key.as_str()
            } else {
                node.id.as_str()
            };
            let angle = 2.0 * std::f64::consts::PI * i as f64 / n as f64;
            let jx = (self.store.rng.random_float() - 0.5) * SNAPSHOT_RING_JITTER;
            let jy = (self.store.rng.random_float() - 0.5) * SNAPSHOT_RING_JITTER;
            let x = (angle.cos() * radius + jx) as f32;
            let y = (angle.sin() * radius + jy) as f32;
            self.store.add_snapshot_node(id, node, x, y);
        }

        let mut report = SnapshotLoad {
            nodes: node_entries.len(),
            edges: edge_entries.len(),
            ..SnapshotLoad::default()
        };
        for (_, edge) in edge_entries {
            if edge.source.is_empty() || edge.target.is_empty() {
                report.invalid_edges += 1;
                continue;
            }
            if !self.store.has_node(&edge.source) || !self.store.has_node(&edge.target) {
                report.orphan_edges += 1;
                continue;
            }
            self.store.upsert_edge(edge);
        }

        self.store.refresh_parallel_edges();

        // Mise en page recréée avec des réglages adaptés à la taille du graphe.
        self.layout.start(self.store.order());
        if !self.force_enabled {
            self.layout.stop();
        }
        self.camera_reset_requested = true;
        report
    }

    /// Restaure le graphe détenu par le cœur au (re)montage de la vue
    /// (résultat de `get_graph_snapshot`) : un snapshot
    /// vide ne remplace rien.
    pub fn restore_snapshot(&mut self, snapshot: &GraphData) -> Option<SnapshotLoad> {
        if snapshot.nodes.is_empty() {
            return None;
        }
        Some(self.load_snapshot(snapshot))
    }

    /// Recadrage demandé depuis le dernier appel (après un chargement de
    /// snapshot, `camera.animatedReset()`).
    pub fn take_camera_reset_request(&mut self) -> bool {
        std::mem::take(&mut self.camera_reset_requested)
    }

    /// Emprise des nœuds (cadrage de la caméra, voir
    /// [`super::camera::Normalization`]).
    pub fn bounds(&self) -> Option<Bounds> {
        self.store.bounds()
    }

    // === Totaux ===============================================================

    /// Nombre de nœuds affichés.
    pub fn node_count(&self) -> usize {
        self.store.order()
    }

    /// Nombre d'arêtes affichées.
    pub fn edge_count(&self) -> usize {
        self.store.size()
    }

    /// Totaux du bandeau : « 49 nœuds · 71 arêtes ». Le graphe ne
    /// change que par `reset`, `load_snapshot` et `flush` : le libellé suit.
    pub fn counts_label(&self) -> String {
        graph_counts_label(self.node_count(), self.edge_count())
    }

    // === Mise en page =========================================================

    /// Bouton « Gravité » actif.
    pub fn force_enabled(&self) -> bool {
        self.force_enabled
    }

    /// ForceAtlas2 itère en ce moment.
    pub fn layout_running(&self) -> bool {
        self.layout
            .supervisor()
            .is_some_and(Fa2Supervisor::is_running)
    }

    /// Contrôleur de la mise en page (lecture).
    pub fn layout(&self) -> &ForceLayoutController<Fa2Supervisor> {
        &self.layout
    }

    /// Recrée la mise en page si le graphe a assez grandi, gravité active
    /// seulement.
    fn ensure_layout_settings(&mut self) {
        if self.force_enabled {
            self.layout.ensure_settings(self.store.order());
        }
    }

    /// Clic sur « Gravité » : reprise (réglages revus si le graphe a grandi)
    /// ou arrêt.
    pub fn toggle_force(&mut self) {
        self.force_enabled = !self.force_enabled;
        if self.force_enabled {
            self.layout.resume(self.store.order());
        } else {
            self.layout.stop();
        }
    }

    /// Libellé du bouton « Gravité ».
    pub fn force_button_label(&self) -> &'static str {
        if self.force_enabled {
            "Gravité: ON"
        } else {
            "Gravité: OFF"
        }
    }

    /// Infobulle du bouton « Gravité ».
    pub fn force_button_title(&self) -> &'static str {
        if self.force_enabled {
            "Désactiver la gravité (elle se coupe seule au bout de 20 minutes)"
        } else {
            "Activer la gravité pour 20 minutes"
        }
    }

    /// Un pas de mise en page : relève l'arrêt automatique — au bout de 20
    /// minutes la gravité se coupe comme par un clic, un clic la relance pour
    /// 20 minutes de plus — puis fait `iterations` itérations de
    /// ForceAtlas2 si elle tourne.
    pub fn step_layout(&mut self, iterations: usize) -> LayoutStep {
        let auto_stopped = self.layout.poll();
        if auto_stopped {
            self.force_enabled = false;
        }
        let moved = self
            .layout
            .supervisor_mut()
            .is_some_and(|supervisor| supervisor.step(&mut self.store, iterations));
        LayoutStep {
            auto_stopped,
            moved,
        }
    }

    // === Zoom ===================================================================

    /// Le ratio de caméra a changé (`camera.on("updated")`) ; retourne vrai si
    /// l'affichage des labels d'arêtes change.
    pub fn set_camera_ratio(&mut self, ratio: f32) -> bool {
        self.set_zoom(zoom_for_ratio(ratio))
    }

    /// Nouveau zoom ; retourne vrai si un seuil de labels
    /// d'arêtes a été franchi (rafraîchissement nécessaire).
    pub fn set_zoom(&mut self, zoom: f32) -> bool {
        self.zoom = zoom;
        let labels = EdgeLabelVisibility::for_zoom(zoom);
        let changed = labels != self.labels;
        self.labels = labels;
        changed
    }

    /// Zoom courant.
    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    /// Zoom du bandeau, `zoomLevel.toPrecision(2)` (« 1.0 », « 0.50 »).
    pub fn zoom_text(&self) -> String {
        to_precision(f64::from(self.zoom), 2)
    }

    /// Protocoles affichés sur les arêtes.
    pub fn edge_labels_shown(&self) -> bool {
        self.labels.edge_labels
    }

    /// Ports affichés sur les arêtes.
    pub fn port_labels_shown(&self) -> bool {
        self.labels.port_labels
    }

    // === Survol, clics et tunnels ================================================

    /// Survol d'un nœud (entrée ou sortie).
    pub fn hover_node(&mut self, node: Option<NodeIndex>) {
        self.hovered_node = node.filter(|n| n.index() < self.store.order());
    }

    /// Nœud survolé.
    pub fn hovered_node(&self) -> Option<NodeIndex> {
        self.hovered_node
    }

    /// Clic sur un nœud : sélection et bandeau bas.
    pub fn click_node(&mut self, node: NodeIndex) {
        let Some(id) = self.store.node(node).map(|n| n.id.clone()) else {
            return;
        };
        let Some(snapshot) = node_snapshot(&self.store, &id) else {
            return;
        };
        self.selected = Some(node);
        self.edited_label = snapshot.label.clone();
        self.selected_node_infos = build_node_infos(&self.store, &id);
        self.selected_node = Some(snapshot);
    }

    /// Clic sur le fond : désélection et libération de l'épinglage.
    pub fn click_stage(&mut self) {
        self.clear_selection();
        self.unpin_tunnel_highlight();
    }

    /// Surligne la famille de tunnel(s) de `edge` ; vrai si elle en a une.
    fn apply_tunnel_highlight(&mut self, edge: EdgeIndex) -> bool {
        match compute_tunnel_highlight(&self.store, edge, self.pinned_tunnel_edge) {
            Some(highlight) => {
                self.tunnel = Some(highlight);
                true
            }
            None => false,
        }
    }

    /// Survol d'une arête.
    pub fn enter_edge(&mut self, edge: EdgeIndex) {
        if !self.apply_tunnel_highlight(edge) {
            self.clear_tunnel_highlight();
        }
    }

    /// Fin de survol d'une arête : revient à la famille épinglée s'il y en a
    /// une, sinon efface.
    pub fn leave_edge(&mut self) {
        self.clear_tunnel_highlight();
    }

    /// Clic sur une arête : épingle sa famille de tunnel, re-clic = libère
    ///.
    pub fn click_edge(&mut self, edge: EdgeIndex) {
        if self.pinned_tunnel_edge == Some(edge) {
            self.pinned_tunnel_edge = None;
            self.clear_tunnel_highlight();
            return;
        }
        if edge_has_tunnel(&self.store, edge) {
            self.pinned_tunnel_edge = Some(edge);
            self.apply_tunnel_highlight(edge);
        } else {
            self.pinned_tunnel_edge = None;
            self.clear_tunnel_highlight();
        }
    }

    /// Revient à la famille épinglée, sinon efface la surbrillance.
    fn clear_tunnel_highlight(&mut self) {
        if let Some(pinned) = self.pinned_tunnel_edge
            && self.apply_tunnel_highlight(pinned)
        {
            return;
        }
        self.pinned_tunnel_edge = None;
        self.tunnel = None;
    }

    /// Efface tout, épinglage compris (clic sur le fond, reset).
    fn unpin_tunnel_highlight(&mut self) {
        self.pinned_tunnel_edge = None;
        self.clear_tunnel_highlight();
    }

    /// Famille de tunnel surlignée.
    pub fn tunnel_highlight(&self) -> Option<&TunnelHighlight> {
        self.tunnel.as_ref()
    }

    /// Libellé du bandeau tunnel (vide sans surbrillance).
    pub fn tunnel_info(&self) -> &str {
        self.tunnel.as_ref().map_or("", TunnelHighlight::info)
    }

    /// Arête épinglée.
    pub fn pinned_tunnel_edge(&self) -> Option<EdgeIndex> {
        self.pinned_tunnel_edge
    }

    // === Sélection et édition de label ============================================

    /// Rang du nœud sélectionné.
    pub fn selected_node_index(&self) -> Option<NodeIndex> {
        self.selected
    }

    /// Identifiant du nœud sélectionné.
    fn selected_node_id(&self) -> Option<&str> {
        self.selected
            .and_then(|i| self.store.node(i))
            .map(|n| n.id.as_str())
    }

    /// Snapshot du nœud sélectionné.
    pub fn selected_node(&self) -> Option<&NodeSnapshot> {
        self.selected_node.as_ref()
    }

    /// Lignes d'infos du nœud sélectionné (vide sans sélection).
    pub fn selected_node_infos(&self) -> &[String] {
        &self.selected_node_infos
    }

    /// Saisie du champ de label.
    pub fn edited_label(&self) -> &str {
        &self.edited_label
    }

    /// Modifie la saisie du champ de label.
    pub fn set_edited_label(&mut self, label: impl Into<String>) {
        self.edited_label = label.into();
    }

    /// Enregistrement d'un label en cours (bouton désactivé).
    pub fn is_saving_label(&self) -> bool {
        self.is_saving_label
    }

    /// Ferme le bandeau du nœud sélectionné (Échap).
    pub fn clear_selection(&mut self) {
        self.selected_node_infos.clear();
        self.selected_node = None;
        self.selected = None;
        self.edited_label.clear();
    }

    /// « Annuler » : la saisie revient au label du nœud.
    pub fn cancel_edit(&mut self) {
        let Some(id) = self.selected_node_id().map(str::to_string) else {
            return;
        };
        if let Some(selected) = &self.selected_node {
            self.edited_label = selected.label.clone();
            self.selected_node_infos = build_node_infos(&self.store, &id);
        }
    }

    /// « Enregistrer » (ou Entrée) : applique le label saisi de façon
    /// optimiste et rend la requête `add_label` à transmettre au cœur
    ///. `None` sans nœud sélectionné.
    pub fn begin_label_edit(&mut self) -> Option<LabelEditRequest> {
        let previous = self.selected_node.clone()?;
        let node_id = self.selected_node_id()?.to_string();
        let new_label = js_trim(&self.edited_label).to_string();
        let (mac, ip) = {
            let node = self.store.node_by_id(&node_id)?;
            (node.style.mac.clone(), node.style.ip.clone())
        };

        // Mise à jour immédiate de l'interface, avec retour arrière si le
        // cœur refuse.
        let rollback = apply_optimistic_label(&mut self.store, &node_id, &new_label)?;
        if let Some(selected) = self.selected_node.as_mut() {
            selected.label = new_label.clone();
        }
        self.selected_node_infos = build_node_infos(&self.store, &node_id);
        self.is_saving_label = true;
        Some(LabelEditRequest {
            node_id,
            mac,
            ip,
            label: new_label,
            rollback,
            previous,
        })
    }

    /// Le cœur a accepté le label : ses updates (tous les labels
    /// resynchronisés) passent par la file comme celles de la capture.
    pub fn commit_label_edit(&mut self, _request: LabelEditRequest, updates: Vec<GraphUpdate>) {
        self.enqueue_all(updates);
        self.is_saving_label = false;
    }

    /// Le cœur a refusé le label : retour arrière, et bandeau restauré si le
    /// nœud est toujours sélectionné. L'appelant affiche l'erreur.
    pub fn fail_label_edit(&mut self, request: LabelEditRequest) {
        let LabelEditRequest {
            node_id,
            rollback,
            previous,
            ..
        } = request;
        rollback.rollback(&mut self.store);
        if self.selected_node_id() == Some(node_id.as_str()) {
            self.edited_label = previous.label.clone();
            self.selected_node = Some(previous);
            self.selected_node_infos = build_node_infos(&self.store, &node_id);
        }
        self.is_saving_label = false;
    }

    // === Rendu ======================================================================

    /// Nœud à dessiner, reducer appliqué.
    pub fn node_render(&self, index: NodeIndex) -> Option<NodeRender<'_>> {
        let node = self.store.node(index)?;
        let dimmed = self
            .tunnel
            .as_ref()
            .is_some_and(|t| !t.contains_node(index));
        let hovered = self.hovered_node == Some(index);
        let mut fill = node.style.fill;
        let mut label = Some(node.style.label.as_str()).filter(|l| !l.is_empty());
        if dimmed {
            fill = DIM_NODE_RGB;
            label = None;
        }
        if hovered {
            fill = node.style.hover;
        }
        Some(NodeRender {
            index,
            id: &node.id,
            x: node.x,
            y: node.y,
            size: node.size,
            fill,
            border: node.style.border,
            label,
            hovered,
            highlighted: self.selected == Some(index),
            dimmed,
        })
    }

    /// Nœuds à dessiner, dans l'ordre d'insertion.
    pub fn nodes_render(&self) -> impl Iterator<Item = NodeRender<'_>> + '_ {
        (0..self.store.order()).filter_map(|i| self.node_render(NodeIndex(i as u32)))
    }

    /// Arête à dessiner, reducer appliqué.
    pub fn edge_render(&self, index: EdgeIndex) -> Option<EdgeRender<'_>> {
        let edge = self.store.edge(index)?;
        let source = self.store.node(edge.source)?;
        let target = self.store.node(edge.target)?;
        let mut color = edge.style.rgb;
        let mut size = edge.style.size;
        let mut z_index = 0;
        let mut dimmed = false;
        if let Some(tunnel) = &self.tunnel {
            if tunnel.contains_edge(index) {
                z_index = 1;
                // `(data.size || 1) + 1`.
                let base = if size > 0.0 { size } else { 1.0 };
                size = base + 1.0;
            } else {
                dimmed = true;
                color = DIM_EDGE_RGB;
            }
        }
        let label = if !self.labels.edge_labels || dimmed {
            None
        } else if self.labels.port_labels {
            Some(edge.port_label.as_str())
        } else {
            Some(edge.style.protocol.as_str())
        }
        .filter(|l| !l.is_empty());
        Some(EdgeRender {
            index,
            source: edge.source,
            target: edge.target,
            source_pos: (source.x, source.y),
            target_pos: (target.x, target.y),
            target_size: target.size,
            size,
            color,
            kind: edge.kind,
            curvature: edge.curvature,
            z_index,
            dimmed,
            label,
        })
    }

    /// Arêtes à dessiner, dans l'ordre d'insertion (le rendu trie ensuite
    /// par `z_index`).
    pub fn edges_render(&self) -> impl Iterator<Item = EdgeRender<'_>> + '_ {
        (0..self.store.size()).filter_map(|i| self.edge_render(EdgeIndex(i as u32)))
    }

    // === Tests d'atteinte ===============================================================

    /// Nœud sous le point monde `(x, y)` ; `size_to_world` convertit une
    /// taille d'écran en unités monde à la caméra courante. Parmi plusieurs
    /// nœuds, le dernier inséré (dessiné au-dessus). L'index spatial est
    /// reconstruit si les positions ont changé.
    pub fn node_at(&mut self, x: f32, y: f32, size_to_world: f32) -> Option<NodeIndex> {
        let version = self.store.positions_version();
        if self.grid_version != Some(version) {
            self.grid.rebuild(self.store.nodes());
            self.grid_version = Some(version);
        }
        self.grid.node_at(x, y, size_to_world)
    }

    /// Arête sous le point monde `(x, y)`, à `tolerance` unités monde près de
    /// son trait (épaisseur comprise). Parmi plusieurs, celle du plan le plus
    /// haut puis la dernière insérée, comme au dessin. À tester
    /// après [`Self::node_at`] : les nœuds sont dessinés par-dessus.
    pub fn edge_at(&self, x: f32, y: f32, size_to_world: f32, tolerance: f32) -> Option<EdgeIndex> {
        let mut best: Option<(u8, EdgeIndex)> = None;
        for edge in self.edges_render() {
            if edge.source == edge.target {
                continue;
            }
            let reach = edge.size * size_to_world / 2.0 + tolerance;
            let (s, t) = (edge.source_pos, edge.target_pos);
            let hit = match edge.kind {
                EdgeKind::Straight => segment_distance_sq((x, y), s, t) <= reach * reach,
                EdgeKind::Curved => {
                    let c = edge_control_point(s, t, edge.curvature);
                    curve_distance_sq((x, y), s, c, t) <= reach * reach
                }
            };
            if hit && best.is_none_or(|(z, i)| (edge.z_index, edge.index) > (z, i)) {
                best = Some((edge.z_index, edge.index));
            }
        }
        best.map(|(_, i)| i)
    }
}

/// Carré de la distance du point `p` au segment `[a, b]`.
fn segment_distance_sq(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (abx, aby) = (b.0 - a.0, b.1 - a.1);
    let (apx, apy) = (p.0 - a.0, p.1 - a.1);
    let len_sq = abx * abx + aby * aby;
    let t = if len_sq > 0.0 {
        ((apx * abx + apy * aby) / len_sq).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (dx, dy) = (apx - t * abx, apy - t * aby);
    dx * dx + dy * dy
}

/// Carré de la distance du point `p` à la courbe de Bézier quadratique
/// `(a, c, b)`, approchée par [`CURVE_HIT_SEGMENTS`] segments.
fn curve_distance_sq(p: (f32, f32), a: (f32, f32), c: (f32, f32), b: (f32, f32)) -> f32 {
    let at = |t: f32| {
        let u = 1.0 - t;
        (
            u * u * a.0 + 2.0 * u * t * c.0 + t * t * b.0,
            u * u * a.1 + 2.0 * u * t * c.1 + t * t * b.1,
        )
    };
    let mut best = f32::INFINITY;
    let mut previous = a;
    for i in 1..=CURVE_HIT_SEGMENTS {
        let next = at(i as f32 / CURVE_HIT_SEGMENTS as f32);
        best = best.min(segment_distance_sq(p, previous, next));
        previous = next;
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::force_layout::FORCE_LAYOUT_MAX_RUN;
    use crate::graph::force_layout::tests::FakeClock;
    use crate::graph::style::tests::{edge, node};
    use crate::graph::style::{NODE_SIZE_MIN, brighten_rgb, parse_hex};
    use crate::graph::sync::UNSIZED_NODE_SIZE;
    use lunar_flows_core::graph::{Edge, Node};

    fn model() -> (GraphModel, FakeClock) {
        let clock = FakeClock::new();
        (
            GraphModel::with_rng_and_clock(Rng::with_seed(9), clock.clock()),
            clock,
        )
    }

    fn snapshot(nodes: &[Node], edges: &[Edge]) -> GraphData {
        let mut data = GraphData::new();
        for n in nodes {
            data.nodes.insert(format!("key-{}", n.id), n.clone());
        }
        for (i, e) in edges.iter().enumerate() {
            data.edges.insert(format!("edge-{i:04}"), e.clone());
        }
        data
    }

    fn tunnel_edge(source: &str, target: &str, label: &str, encap: &str) -> Edge {
        let mut e = edge(source, target, label);
        e.encap_ids = vec![encap.to_string()];
        e
    }

    /// Graphe chargé : capwapA ↔ capwapB (tunnel enc-1), client → server (dans
    /// le tunnel), autre → client (hors tunnel).
    fn loaded() -> (GraphModel, FakeClock) {
        let (mut m, clock) = model();
        let nodes: Vec<_> = ["capwapA", "capwapB", "client", "server", "autre"]
            .into_iter()
            .map(node)
            .collect();
        let edges = [
            tunnel_edge("capwapA", "capwapB", "CAPWAP", "enc-1"),
            tunnel_edge("client", "server", "TCP", "enc-1"),
            edge("autre", "client", "UDP"),
        ];
        m.load_snapshot(&snapshot(&nodes, &edges));
        (m, clock)
    }

    fn e(m: &GraphModel, key: &str) -> EdgeIndex {
        m.store().edge_index(key).expect(key)
    }

    fn n(m: &GraphModel, id: &str) -> NodeIndex {
        m.store().node_index(id).expect(id)
    }

    // --- Snapshot --------------------------------------------------------------

    #[test]
    fn snapshot_place_les_noeuds_en_couronne() {
        let (mut m, _) = model();
        let nodes: Vec<_> = (0..16).map(|i| node(&format!("n{i:02}"))).collect();
        let report = m.load_snapshot(&snapshot(&nodes, &[]));
        assert_eq!(
            report,
            SnapshotLoad {
                nodes: 16,
                edges: 0,
                ..SnapshotLoad::default()
            }
        );

        let radius = snapshot_ring_radius(16);
        assert_eq!(radius, 260.0);
        for (i, node) in m.store().nodes().iter().enumerate() {
            let angle = 2.0 * std::f64::consts::PI * i as f64 / 16.0;
            let (ex, ey) = (angle.cos() * radius, angle.sin() * radius);
            assert!(
                (f64::from(node.x) - ex).abs() <= 15.0 + 1e-3,
                "{i}: x={}",
                node.x
            );
            assert!(
                (f64::from(node.y) - ey).abs() <= 15.0 + 1e-3,
                "{i}: y={}",
                node.y
            );
            // Pas d'attribut size sur un nœud de snapshot sans trafic.
            assert_eq!(node.size, UNSIZED_NODE_SIZE);
        }
        assert!(m.take_camera_reset_request());
        assert!(!m.take_camera_reset_request());
    }

    #[test]
    fn snapshot_id_du_noeud_sinon_cle_et_aretes_ignorees() {
        let (mut m, _) = model();
        let mut data = snapshot(
            &[node("a"), node("b")],
            &[
                edge("a", "b", "TCP"),
                edge("a", "fantome", "TCP"),
                edge("", "b", "TCP"),
            ],
        );
        data.nodes.insert("cle-sans-id".into(), node(""));
        let report = m.load_snapshot(&data);

        assert_eq!(
            report,
            SnapshotLoad {
                nodes: 3,
                edges: 3,
                invalid_edges: 1,
                orphan_edges: 1
            }
        );
        assert!(m.store().has_node("cle-sans-id"));
        assert_eq!(
            m.store()
                .node_by_id("cle-sans-id")
                .expect("nœud")
                .style
                .label,
            "cle-sans-id"
        );
        assert_eq!(m.edge_count(), 1);
        assert_eq!(m.counts_label(), "3 nœuds · 1 arête");
    }

    #[test]
    fn snapshot_vide_ne_remplace_pas_le_graphe() {
        let (mut m, _) = loaded();
        assert_eq!(m.restore_snapshot(&GraphData::new()), None);
        assert_eq!(m.node_count(), 5);
        assert!(m.restore_snapshot(&snapshot(&[node("x")], &[])).is_some());
        assert_eq!(m.node_count(), 1);
    }

    #[test]
    fn snapshot_redemarre_la_mise_en_page_sauf_gravite_coupee() {
        let (mut m, _) = loaded();
        assert!(m.layout_running());
        assert_eq!(m.layout().last_order(), 5);

        m.toggle_force();
        assert!(!m.layout_running());
        m.load_snapshot(&snapshot(&[node("a"), node("b")], &[]));
        assert!(!m.layout_running());
        assert!(m.layout().supervisor().is_some());
        assert_eq!(m.layout().auto_stop_deadline(), None);
    }

    // --- File d'updates -----------------------------------------------------------

    #[test]
    fn updates_appliquees_au_flush_seulement() {
        let (mut m, _) = model();
        m.enqueue(GraphUpdate::NewNode(node("a")));
        m.enqueue(GraphUpdate::NewNode(node("b")));
        m.enqueue(GraphUpdate::NewEdge(edge("a", "b", "TCP")));
        assert_eq!(m.node_count(), 0);
        assert_eq!(m.pending_updates(), 3);

        assert!(m.flush());
        assert_eq!(m.pending_updates(), 0);
        assert_eq!(m.counts_label(), "2 nœuds · 1 arête");
        assert!(!m.flush(), "file vide");
        // Premier peuplement : la mise en page démarre (gravité active).
        assert!(m.layout_running());
        assert_eq!(m.layout().last_order(), 2);
    }

    #[test]
    fn flush_recalcule_les_aretes_paralleles() {
        let (mut m, _) = model();
        m.enqueue_all([
            GraphUpdate::NewNode(node("a")),
            GraphUpdate::NewNode(node("b")),
            GraphUpdate::NewEdge(edge("a", "b", "TCP")),
            GraphUpdate::NewEdge(edge("a", "b", "DNS")),
        ]);
        m.flush();
        assert!(m.store().edges().iter().all(|e| e.kind == EdgeKind::Curved));
    }

    #[test]
    fn flush_recree_la_mise_en_page_quand_le_graphe_quadruple() {
        let (mut m, _) = model();
        m.enqueue(GraphUpdate::NewNode(node("n0")));
        m.flush();
        assert_eq!(m.layout().last_order(), 1);
        m.enqueue_all((1..3).map(|i| GraphUpdate::NewNode(node(&format!("n{i}")))));
        m.flush();
        assert_eq!(m.layout().last_order(), 1, "×3 : réglages conservés");
        m.enqueue(GraphUpdate::NewNode(node("n3")));
        m.flush();
        assert_eq!(m.layout().last_order(), 4);
        // Gravité coupée : pas de recréation (ensureLayoutSettings gardé).
        m.toggle_force();
        m.enqueue_all((4..20).map(|i| GraphUpdate::NewNode(node(&format!("n{i}")))));
        m.flush();
        assert_eq!(m.layout().last_order(), 4);
        // La reprise revoit les réglages.
        m.toggle_force();
        assert_eq!(m.layout().last_order(), 20);
    }

    #[test]
    fn reset_vide_la_file_et_le_graphe() {
        let (mut m, _) = loaded();
        m.enqueue(GraphUpdate::NewNode(node("tardif")));
        m.click_node(n(&m, "client"));
        m.click_edge(e(&m, "client__server__TCP"));
        m.hover_node(Some(n(&m, "autre")));
        m.reset();

        assert_eq!(m.pending_updates(), 0);
        assert_eq!(m.counts_label(), "0 nœud · 0 arête");
        assert_eq!(m.selected_node_index(), None);
        assert_eq!(m.pinned_tunnel_edge(), None);
        assert_eq!(m.tunnel_info(), "");
        assert_eq!(m.hovered_node(), None);
        assert!(m.layout().supervisor().is_none());
        // Un update mis en file avant le reset n'est pas rejoué.
        assert!(!m.flush());
        assert_eq!(m.node_count(), 0);
    }

    #[test]
    fn totaux_rafraichis_par_reset_chargement_et_flush() {
        // Les trois seules sources de modification du graphe.
        let (mut m, _) = model();
        assert_eq!(m.counts_label(), "0 nœud · 0 arête");
        m.load_snapshot(&snapshot(&[node("a"), node("b")], &[edge("a", "b", "TCP")]));
        assert_eq!(m.counts_label(), "2 nœuds · 1 arête");
        m.enqueue(GraphUpdate::NewNode(node("c")));
        assert_eq!(m.counts_label(), "2 nœuds · 1 arête", "rien avant le flush");
        m.flush();
        assert_eq!(m.counts_label(), "3 nœuds · 1 arête");
        m.reset();
        assert_eq!(m.counts_label(), "0 nœud · 0 arête");
    }

    #[test]
    fn node_updated_resynchronise_le_bandeau_du_noeud_selectionne() {
        let (mut m, _) = loaded();
        m.click_node(n(&m, "client"));
        m.set_edited_label("saisie en cours");
        let mut renamed = node("client");
        renamed.label = Some("Poste client".into());
        m.enqueue(GraphUpdate::NodeUpdated(renamed));
        m.flush();

        assert_eq!(
            m.selected_node().map(|s| s.label.as_str()),
            Some("Poste client")
        );
        assert_eq!(m.edited_label(), "Poste client");
        assert!(
            m.selected_node_infos()
                .contains(&"Label: Poste client".to_string())
        );

        // Un autre nœud mis à jour ne touche pas au bandeau.
        m.set_edited_label("autre saisie");
        m.enqueue(GraphUpdate::NodeUpdated(node("server")));
        m.flush();
        assert_eq!(m.edited_label(), "autre saisie");
    }

    #[test]
    fn un_nouveau_noeud_relie_rejoint_son_voisin() {
        let (mut m, _) = loaded();
        let (sx, sy) = {
            let s = m.store().node_by_id("server").expect("server");
            (s.x, s.y)
        };
        m.enqueue(GraphUpdate::NewNode(node("nouveau")));
        m.enqueue(GraphUpdate::NewEdge(edge("nouveau", "server", "TCP")));
        m.flush();
        let fresh = m.store().node_by_id("nouveau").expect("nouveau");
        assert_eq!(fresh.size, NODE_SIZE_MIN);
        let d = f64::from(fresh.x - sx).hypot(f64::from(fresh.y - sy));
        assert!(d <= 120.0 + 1e-2, "{d}");
    }

    // --- Mise en page ----------------------------------------------------------------

    #[test]
    fn la_gravite_deplie_le_graphe_et_s_arrete_au_bout_de_20_minutes() {
        let (mut m, clock) = loaded();
        let before: Vec<_> = m.store().nodes().iter().map(|n| (n.x, n.y)).collect();
        let step = m.step_layout(5);
        assert_eq!(
            step,
            LayoutStep {
                auto_stopped: false,
                moved: true
            }
        );
        let after: Vec<_> = m.store().nodes().iter().map(|n| (n.x, n.y)).collect();
        assert_ne!(before, after);

        clock.advance(FORCE_LAYOUT_MAX_RUN);
        let step = m.step_layout(5);
        // La vue du graphe repasse « Gravité » sur OFF à l'arrêt automatique.
        assert_eq!(
            step,
            LayoutStep {
                auto_stopped: true,
                moved: false
            }
        );
        assert!(!m.force_enabled());
        assert_eq!(m.force_button_label(), "Gravité: OFF");
        assert_eq!(m.force_button_title(), "Activer la gravité pour 20 minutes");
        assert!(!m.layout_running());

        // Un clic relance pour 20 minutes.
        m.toggle_force();
        assert!(m.force_enabled());
        assert_eq!(m.force_button_label(), "Gravité: ON");
        assert_eq!(
            m.layout().auto_stop_deadline().map(|d| d - clock.now()),
            Some(FORCE_LAYOUT_MAX_RUN)
        );
        assert!(m.step_layout(1).moved);
    }

    #[test]
    fn rien_ne_tourne_sur_un_graphe_vide() {
        let (mut m, _) = model();
        m.toggle_force();
        m.toggle_force();
        assert_eq!(m.step_layout(10), LayoutStep::default());
        assert!(m.layout().supervisor().is_none());
    }

    // --- Zoom ------------------------------------------------------------------------------

    #[test]
    fn zoom_et_seuils_des_labels_d_aretes() {
        let (mut m, _) = loaded();
        assert_eq!(m.zoom_text(), "1.0");
        let tcp = e(&m, "client__server__TCP");
        assert_eq!(m.edge_render(tcp).expect("arête").label, None);

        assert!(!m.set_camera_ratio(0.9), "zoom 1.11 : aucun seuil franchi");
        assert!(m.set_camera_ratio(0.8), "zoom 1.25");
        assert!(m.edge_labels_shown() && !m.port_labels_shown());
        assert_eq!(m.edge_render(tcp).expect("arête").label, Some("TCP"));

        assert!(m.set_zoom(2.0));
        assert_eq!(m.zoom_text(), "2.0");
        assert!(m.port_labels_shown());
        assert_eq!(m.edge_render(tcp).expect("arête").label, Some("TCP"));
        let mut with_ports = edge("client", "server", "TCP");
        with_ports.ports = [443].into();
        with_ports.encap_ids = vec!["enc-1".into()];
        m.enqueue(GraphUpdate::EdgeUpdated(with_ports));
        m.flush();
        assert_eq!(m.edge_render(tcp).expect("arête").label, Some("TCP :443"));

        assert!(m.set_camera_ratio(4.0));
        assert_eq!(m.zoom_text(), "0.25");
        assert!(!m.edge_labels_shown());
    }

    // --- Reducers ----------------------------------------------------------------------------

    #[test]
    fn reducer_noeud_survol_selection() {
        let (mut m, _) = loaded();
        let client = n(&m, "client");
        let base = m.node_render(client).expect("nœud");
        assert_eq!(base.fill, parse_hex("#2196F3"));
        assert_eq!(base.label, Some("client"));
        assert!(!base.hovered && !base.highlighted && !base.dimmed);

        m.hover_node(Some(client));
        let hovered = m.node_render(client).expect("nœud");
        assert_eq!(hovered.fill, brighten_rgb(parse_hex("#2196F3"), 0.18));
        assert!(hovered.hovered);
        m.hover_node(None);

        m.click_node(client);
        assert!(m.node_render(client).expect("nœud").highlighted);
        m.click_stage();
        assert!(!m.node_render(client).expect("nœud").highlighted);
    }

    #[test]
    fn reducers_surbrillance_d_un_tunnel() {
        let (mut m, _) = loaded();
        m.set_zoom(1.5);
        m.enter_edge(e(&m, "capwapA__capwapB__CAPWAP"));
        assert_eq!(m.tunnel_info(), "tunnel enc-1… — 2 flux liés");

        let autre = m.node_render(n(&m, "autre")).expect("nœud");
        assert!(autre.dimmed);
        assert_eq!(autre.fill, DIM_NODE_RGB);
        assert_eq!(autre.label, None);
        // La bordure n'est pas estompée (le reducer ne touche qu'à color).
        assert_eq!(
            autre.border,
            m.store().node_by_id("autre").expect("autre").style.border
        );
        let client = m.node_render(n(&m, "client")).expect("nœud");
        assert!(!client.dimmed && client.label.is_some());

        let outside = m.edge_render(e(&m, "autre__client__UDP")).expect("arête");
        assert!(outside.dimmed);
        assert_eq!(
            (outside.color, outside.z_index, outside.label),
            (DIM_EDGE_RGB, 0, None)
        );
        let inner = m.edge_render(e(&m, "client__server__TCP")).expect("arête");
        let base = m
            .store()
            .edge_by_key("client__server__TCP")
            .expect("arête")
            .style
            .size;
        assert_eq!(
            (inner.z_index, inner.size, inner.label),
            (1, base + 1.0, Some("TCP"))
        );

        // Un nœud estompé survolé prend sa couleur de survol, sans label.
        m.hover_node(Some(n(&m, "autre")));
        let hovered = m.node_render(n(&m, "autre")).expect("nœud");
        let hover = m.store().node_by_id("autre").expect("autre").style.hover;
        assert_eq!((hovered.fill, hovered.label), (hover, None));

        m.leave_edge();
        assert_eq!(m.tunnel_info(), "");
        assert!(m.edges_render().all(|e| !e.dimmed && e.z_index == 0));
    }

    #[test]
    fn epinglage_d_un_tunnel() {
        let (mut m, _) = loaded();
        let tunnel = e(&m, "capwapA__capwapB__CAPWAP");
        let outside = e(&m, "autre__client__UDP");

        m.click_edge(tunnel);
        assert_eq!(m.pinned_tunnel_edge(), Some(tunnel));
        assert_eq!(m.tunnel_info(), "📌 tunnel enc-1… — 2 flux liés");

        // Survol d'une arête hors tunnel puis départ : retour à l'épinglée.
        m.enter_edge(outside);
        assert_eq!(m.tunnel_info(), "📌 tunnel enc-1… — 2 flux liés");
        m.enter_edge(e(&m, "client__server__TCP"));
        assert_eq!(m.tunnel_info(), "tunnel enc-1… — 2 flux liés");
        m.leave_edge();
        assert_eq!(m.tunnel_info(), "📌 tunnel enc-1… — 2 flux liés");

        // Re-clic : libère.
        m.click_edge(tunnel);
        assert_eq!(m.pinned_tunnel_edge(), None);
        assert_eq!(m.tunnel_info(), "");

        // Clic sur une arête sans tunnel : rien d'épinglé.
        m.click_edge(tunnel);
        m.click_edge(outside);
        assert_eq!(m.pinned_tunnel_edge(), None);
        assert_eq!(m.tunnel_info(), "");

        // Clic sur le fond : libère aussi.
        m.click_edge(tunnel);
        m.click_stage();
        assert_eq!(m.pinned_tunnel_edge(), None);
        assert!(m.tunnel_highlight().is_none());
    }

    // --- Édition de label -----------------------------------------------------------------------

    #[test]
    fn edition_de_label_optimiste_puis_validee() {
        let (mut m, _) = loaded();
        let client = n(&m, "client");
        assert_eq!(m.begin_label_edit(), None, "aucune sélection");
        m.click_node(client);
        assert_eq!(m.edited_label(), "");
        m.set_edited_label("  Poste 12  ");

        let request = m.begin_label_edit().expect("requête");
        assert_eq!(request.label, "Poste 12");
        assert_eq!(request.node_id, "client");
        assert!(m.is_saving_label());
        assert_eq!(
            m.store().node(client).expect("nœud").style.label,
            "Poste 12"
        );
        assert_eq!(
            m.selected_node().map(|s| s.label.as_str()),
            Some("Poste 12")
        );
        assert!(
            m.selected_node_infos()
                .contains(&"Label: Poste 12".to_string())
        );

        let mut confirmed = node("client");
        confirmed.label = Some("Poste 12".into());
        m.commit_label_edit(request, vec![GraphUpdate::NodeUpdated(confirmed)]);
        assert!(!m.is_saving_label());
        assert_eq!(m.pending_updates(), 1);
        m.flush();
        assert_eq!(
            m.store().node(client).expect("nœud").style.raw_label,
            "Poste 12"
        );
    }

    #[test]
    fn edition_de_label_refusee_retour_arriere() {
        let (mut m, _) = loaded();
        let client = n(&m, "client");
        m.click_node(client);
        m.set_edited_label("Refusé");
        let request = m.begin_label_edit().expect("requête");

        m.fail_label_edit(request);
        assert!(!m.is_saving_label());
        let style = &m.store().node(client).expect("nœud").style;
        assert_eq!(
            (style.raw_label.as_str(), style.label.as_str()),
            ("", "client")
        );
        assert_eq!(m.selected_node().map(|s| s.label.as_str()), Some(""));
        assert_eq!(m.edited_label(), "");
        assert!(m.selected_node_infos().contains(&"Label: N/A".to_string()));
    }

    #[test]
    fn annuler_et_echap() {
        let (mut m, _) = loaded();
        let mut labelled = node("client");
        labelled.label = Some("PC".into());
        m.enqueue(GraphUpdate::NodeUpdated(labelled));
        m.flush();
        m.click_node(n(&m, "client"));
        m.set_edited_label("brouillon");
        m.cancel_edit();
        assert_eq!(m.edited_label(), "PC");
        m.clear_selection();
        assert_eq!(m.selected_node(), None);
        assert!(m.selected_node_infos().is_empty());
        assert_eq!(m.edited_label(), "");
    }

    // --- Tests d'atteinte ---------------------------------------------------------------------------

    #[test]
    fn atteinte_d_un_noeud_et_d_une_arete() {
        let (mut m, _) = model();
        m.load_snapshot(&snapshot(
            &[node("a"), node("b"), node("c")],
            &[
                edge("a", "b", "TCP"),
                edge("b", "c", "UDP"),
                edge("b", "c", "DNS"),
            ],
        ));
        m.toggle_force();
        // Positions forcées pour un test lisible.
        for (id, (x, y)) in [
            ("a", (0.0, 0.0)),
            ("b", (100.0, 0.0)),
            ("c", (100.0, 100.0)),
        ] {
            let i = n(&m, id);
            let node = m.store.node_mut(i).expect("nœud");
            node.x = x;
            node.y = y;
        }
        m.store.touch_positions();

        assert_eq!(m.node_at(1.0, 1.0, 1.0), Some(n(&m, "a")));
        assert_eq!(m.node_at(50.0, 0.0, 1.0), None);

        // Sur le segment a → b.
        assert_eq!(m.edge_at(50.0, 0.5, 1.0, 0.0), Some(e(&m, "a__b__TCP")));
        assert_eq!(m.edge_at(50.0, 10.0, 1.0, 0.0), None);
        assert_eq!(m.edge_at(50.0, 10.0, 1.0, 10.0), Some(e(&m, "a__b__TCP")));

        // Faisceau b → c : deux courbes de part et d'autre de la droite.
        let udp = m.store().edge_by_key("b__c__UDP").expect("udp");
        let dns = m.store().edge_by_key("b__c__DNS").expect("dns");
        assert_eq!((udp.kind, dns.kind), (EdgeKind::Curved, EdgeKind::Curved));
        let apex = |c: f32| edge_control_point((100.0, 0.0), (100.0, 100.0), c);
        // Le sommet de la courbe est à mi-chemin du point de contrôle.
        let (ux, uy) = apex(udp.curvature);
        let (dx, dy) = apex(dns.curvature);
        let mid = |p: f32, q: f32| (p + q) / 2.0;
        assert_eq!(
            m.edge_at(mid(ux, 100.0), mid(uy, 50.0), 1.0, 0.5),
            Some(e(&m, "b__c__UDP"))
        );
        assert_eq!(
            m.edge_at(mid(dx, 100.0), mid(dy, 50.0), 1.0, 0.5),
            Some(e(&m, "b__c__DNS"))
        );
    }

    #[test]
    fn atteinte_suit_les_positions_de_la_mise_en_page() {
        let (mut m, _) = loaded();
        for _ in 0..5 {
            m.step_layout(3);
            for node in m.store().nodes().to_vec() {
                let i = n(&m, &node.id);
                assert_eq!(
                    m.node_at(node.x, node.y, 1.0).map(|hit| hit >= i),
                    Some(true)
                );
            }
        }
    }

    #[test]
    fn rendu_sans_allocation_d_etiquettes() {
        // Les libellés rendus empruntent le stockage : même adresse à chaque frame.
        let (mut m, _) = loaded();
        m.set_zoom(2.0);
        let i = e(&m, "client__server__TCP");
        let a = m.edge_render(i).and_then(|r| r.label).map(str::as_ptr);
        let b = m.edge_render(i).and_then(|r| r.label).map(str::as_ptr);
        assert_eq!(a, b);
        assert_eq!(m.nodes_render().count(), 5);
        assert_eq!(m.edges_render().count(), 3);
    }
}
