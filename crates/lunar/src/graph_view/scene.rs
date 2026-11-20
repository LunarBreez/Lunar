//! Scène d'une image : les primitives que Sigma 3.0.3 dessinerait, en
//! pixels de la zone de dessin, partagées par le rendu gpui ([`super::paint`])
//! et l'export PNG ([`super::export`]).
//!
//! Ordre des calques, celui de Sigma : arêtes (plan 0 puis plan 1,
//! `zIndex`), labels d'arêtes, nœuds, labels de nœuds, puis calque de survol
//! (label du nœud survolé et du nœud sélectionné, nœud redessiné par-dessus).
//!
//! Géométrie des programmes Sigma :
//! - arête droite (`EdgeArrowProgram`) : trait d'épaisseur
//!   `max(taille / √ratio, 1,7)` arrêté à la base de la flèche ; flèche de
//!   longueur 2,5 × épaisseur et de base 2 × épaisseur, pointe sur la
//!   bordure de la cible ;
//! - arête courbe (`EdgeCurvedArrowProgram`, `@sigma/edge-curve` 3.1.0) :
//!   Bézier quadratique de point de contrôle `milieu + courbure × (dy, −dx)`
//!   à l'écran, même flèche le long de la courbe ;
//! - nœud (`NodeBorderProgram`) : disque de rayon `taille / √ratio`, bordure
//!   de 10 % du rayon.
//!
//! Labels (`LabelGrid`, `renderLabels`, `renderEdgeLabels`) : grille de
//! cellules de 100 px sur la vue à ratio 1, `⌈densité / ratio²⌉` labels par
//! cellule (les plus gros nœuds, puis l'identifiant), seulement si le nœud
//! fait au moins 6 px à l'écran et s'il est à l'écran (marges 150 × 50 px).
//! Un label d'arête s'affiche si ses deux extrémités ont le leur, ou si
//! l'une est survolée ou sélectionnée. Libellés trop longs abrégés par « … »
//! comme `drawStraightEdgeLabel` / `drawCurvedEdgeLabel`.
//!
//! Niveaux de détail sans effet visible : arêtes et nœuds hors de la zone de
//! dessin ignorés, arête plus courte que les rayons de ses extrémités (sous
//! les disques) ignorée, courbes découpées selon leur longueur à l'écran.

use std::sync::Arc;

use lunar_ui_logic::graph::Rgb;
use lunar_ui_logic::graph::style::{EDGE_LABEL_SIZE, NODE_LABEL_SIZE, node_label_box};

use super::camera::{CameraState, MIN_EDGE_THICKNESS, Viewport};
use super::frame::GraphFrame;

/// Côté des cellules de la grille des labels (`labelGridCellSize`).
pub const LABEL_GRID_CELL_SIZE: f32 = 100.0;
/// Densité des labels (`labelDensity`).
pub const LABEL_DENSITY: f32 = 1.0;
/// Taille d'écran minimale d'un nœud dont le label s'affiche
/// (`labelRenderedSizeThreshold`).
pub const LABEL_RENDERED_SIZE_THRESHOLD: f32 = 6.0;
/// Marge horizontale hors écran où un label reste dessiné.
const X_LABEL_MARGIN: f32 = 150.0;
/// Marge verticale hors écran où un label reste dessiné.
const Y_LABEL_MARGIN: f32 = 50.0;
/// Longueur de la flèche rapportée à l'épaisseur (`lengthToThicknessRatio`).
pub const ARROW_LENGTH_RATIO: f32 = 2.5;
/// Demi-base de la flèche rapportée à l'épaisseur
/// (`widenessToThicknessRatio` : base de 2 × épaisseur).
pub const ARROW_WIDENESS_RATIO: f32 = 2.0;
/// Fraction du rayon occupée par la bordure d'un nœud.
pub const NODE_BORDER_RATIO: f32 = lunar_ui_logic::graph::style::NODE_BORDER_SIZE_RATIO;
/// Points de suspension des libellés abrégés.
const ELLIPSIS: char = '…';

/// Point en pixels de la zone de dessin (y vers le bas).
pub type Pt = (f32, f32);

/// Mesure de texte : largeur d'un libellé à une taille de police, avec la
/// police des labels (Liberation Sans, métriques d'Arial).
pub trait TextMeasure {
    /// Largeur de `text` à `font_size` pixels.
    fn width(&mut self, text: &str, font_size: f32) -> f32;
}

/// Tracé du trait d'une arête.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EdgeShape {
    /// Segment droit.
    Segment {
        /// Début (centre de la source).
        from: Pt,
        /// Fin (base de la flèche).
        to: Pt,
    },
    /// Courbe aplatie : `len` points de [`Scene::points`] à partir de `start`.
    Polyline {
        /// Premier point.
        start: u32,
        /// Nombre de points (au moins 2).
        len: u32,
    },
}

/// Arête à dessiner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgePrim {
    /// Rang de l'arête dans l'instantané.
    pub edge: u32,
    /// Couleur.
    pub color: Rgb,
    /// Plan.
    pub z: u8,
    /// Épaisseur du trait, en pixels.
    pub thickness: f32,
    /// Trait (absent si la flèche couvre toute la longueur).
    pub shape: Option<EdgeShape>,
    /// Flèche : pointe, puis les deux coins de la base.
    pub arrow: Option<[Pt; 3]>,
    /// Ligne médiane complète jusqu'à la pointe (test d'atteinte) : début.
    pub hit_from: Pt,
    /// Fin de la ligne médiane (pointe de la flèche).
    pub hit_to: Pt,
}

/// Nœud à dessiner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NodePrim {
    /// Rang du nœud dans l'instantané.
    pub node: u32,
    /// Centre.
    pub center: Pt,
    /// Rayon.
    pub radius: f32,
    /// Remplissage.
    pub fill: Rgb,
    /// Bordure (10 % du rayon).
    pub border: Rgb,
}

/// Label de nœud : fond noir à 80 % et texte blanc (`drawNodeLabel`).
#[derive(Clone, Debug, PartialEq)]
pub struct NodeLabelPrim {
    /// Rang du nœud.
    pub node: u32,
    /// Fond : x, y, largeur, hauteur.
    pub rect: (f32, f32, f32, f32),
    /// Début du texte : x et ligne de base.
    pub text_origin: Pt,
    /// Texte.
    pub text: Arc<str>,
}

/// Placement d'un label d'arête selon Sigma.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EdgeLabelPlacement {
    /// `drawStraightEdgeLabel` : texte tourné de `angle` autour de `center`,
    /// début à `(−largeur/2, offset_y)` dans le repère tourné.
    Straight {
        /// Milieu entre les bordures des deux nœuds.
        center: Pt,
        /// Angle du texte (gardé à l'endroit).
        angle: f32,
        /// Décalage de la ligne de base sous le trait.
        offset_y: f32,
    },
    /// `drawCurvedEdgeLabel` : caractères posés un à un le long de la
    /// courbe décalée `(source, anchor, target)`, à partir du paramètre `t0`.
    Curved {
        /// Début de la courbe décalée.
        source: Pt,
        /// Point de contrôle décalé.
        anchor: Pt,
        /// Fin de la courbe décalée.
        target: Pt,
        /// Paramètre du premier caractère.
        t0: f32,
        /// Longueur de la courbe décalée.
        curve_length: f32,
        /// Courbure (élargissement des caractères : `1 + 0,35 × courbure`).
        curvature: f32,
    },
}

/// Label d'arête.
#[derive(Clone, Debug, PartialEq)]
pub struct EdgeLabelPrim {
    /// Rang de l'arête.
    pub edge: u32,
    /// Texte, abrégé s'il le faut.
    pub text: Arc<str>,
    /// Largeur du texte.
    pub width: f32,
    /// Placement de Sigma (export PNG).
    pub placement: EdgeLabelPlacement,
    /// Centre du texte posé à l'horizontale (rendu gpui, qui ne tourne pas
    /// le texte).
    pub horizontal_center: Pt,
}

/// Primitives d'une image.
#[derive(Clone, Debug, Default)]
pub struct Scene {
    /// Largeur de la zone de dessin.
    pub width: f32,
    /// Hauteur de la zone de dessin.
    pub height: f32,
    /// Arêtes, dans l'ordre de dessin.
    pub edges: Vec<EdgePrim>,
    /// Points des courbes aplaties.
    pub points: Vec<Pt>,
    /// Nœuds, dans l'ordre de dessin.
    pub nodes: Vec<NodePrim>,
    /// Labels d'arêtes.
    pub edge_labels: Vec<EdgeLabelPrim>,
    /// Labels de nœuds retenus par la grille.
    pub node_labels: Vec<NodeLabelPrim>,
    /// Labels du calque de survol.
    pub hover_labels: Vec<NodeLabelPrim>,
    /// Nœuds redessinés au-dessus du calque de survol.
    pub hover_nodes: Vec<NodePrim>,
    /// Centre et rayon d'écran de chaque nœud (`NaN` s'il n'est pas placé).
    pub node_screen: Vec<(f32, f32, f32)>,
    /// Label de chaque nœud affiché à cette image.
    displayed: Vec<bool>,
    /// Candidats de la grille des labels : cellule, rang du nœud.
    grid: Vec<(i64, u32)>,
}

/// Point d'une Bézier quadratique.
pub fn quad_point(t: f32, a: Pt, c: Pt, b: Pt) -> Pt {
    let u = 1.0 - t;
    (
        u * u * a.0 + 2.0 * u * t * c.0 + t * t * b.0,
        u * u * a.1 + 2.0 * u * t * c.1 + t * t * b.1,
    )
}

/// Longueur d'une Bézier quadratique, sur 20 segments (`getCurveLength`).
fn curve_length(a: Pt, c: Pt, b: Pt) -> f32 {
    let mut length = 0.0;
    let mut last = a;
    for i in 1..=20 {
        let p = quad_point(i as f32 / 20.0, a, c, b);
        length += dist(last, p);
        last = p;
    }
    length
}

/// Distance entre deux points.
fn dist(a: Pt, b: Pt) -> f32 {
    (b.0 - a.0).hypot(b.1 - a.1)
}

/// Carré de la distance du point `p` au segment `[a, b]`.
pub fn segment_distance_sq(p: Pt, a: Pt, b: Pt) -> f32 {
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

/// Point de contrôle d'une arête courbe à l'écran (y vers le bas) : celui
/// du shader de `@sigma/edge-curve`, calculé là-bas y vers le haut.
pub fn curve_control_point(s: Pt, t: Pt, curvature: f32) -> Pt {
    let (dx, dy) = (t.0 - s.0, t.1 - s.1);
    (
        (s.0 + t.0) / 2.0 + dy * curvature,
        (s.1 + t.1) / 2.0 - dx * curvature,
    )
}

/// Abrège `label` pour qu'il tienne dans `available` pixels, comme Sigma :
/// « … » ajouté puis caractères retirés un à un ; `None` s'il reste moins de
/// quatre caractères.
fn fit_label(
    label: &Arc<str>,
    available: f32,
    size: f32,
    measure: &mut dyn TextMeasure,
) -> Option<(Arc<str>, f32)> {
    let width = measure.width(label, size);
    if width <= available {
        return Some((label.clone(), width));
    }
    let mut chars: Vec<char> = label.chars().collect();
    chars.push(ELLIPSIS);
    let mut text: String = chars.iter().collect();
    let mut width = measure.width(&text, size);
    while width > available && chars.len() > 1 {
        // `label.slice(0, -2) + ellipsis`.
        let keep = chars.len().saturating_sub(2);
        chars.truncate(keep);
        chars.push(ELLIPSIS);
        text = chars.iter().collect();
        width = measure.width(&text, size);
    }
    if chars.len() < 4 {
        return None;
    }
    Some((Arc::from(text), width))
}

impl Scene {
    /// Construit la scène de `frame` vue par `viewport`.
    pub fn build(
        &mut self,
        frame: &GraphFrame,
        viewport: &Viewport,
        measure: &mut dyn TextMeasure,
    ) {
        self.width = viewport.width;
        self.height = viewport.height;
        self.edges.clear();
        self.points.clear();
        self.nodes.clear();
        self.edge_labels.clear();
        self.node_labels.clear();
        self.hover_labels.clear();
        self.hover_nodes.clear();
        self.node_screen.clear();
        self.displayed.clear();
        self.displayed.resize(frame.nodes.len(), false);

        for node in &frame.nodes {
            if node.visible {
                let (x, y) = viewport.graph_to_viewport(node.x, node.y);
                self.node_screen
                    .push((x, y, viewport.scale_size(node.size)));
            } else {
                self.node_screen.push((f32::NAN, f32::NAN, 0.0));
            }
        }

        for z in [0u8, 1u8] {
            for (i, edge) in frame.edges.iter().enumerate() {
                if edge.z == z && frame.edge_drawable(edge) {
                    self.push_edge(i as u32, frame, viewport);
                }
            }
        }

        for (i, node) in frame.nodes.iter().enumerate() {
            if !node.visible {
                continue;
            }
            let (x, y, r) = self.node_screen[i];
            if x + r < 0.0 || x - r > self.width || y + r < 0.0 || y - r > self.height {
                continue;
            }
            self.nodes.push(NodePrim {
                node: i as u32,
                center: (x, y),
                radius: r,
                fill: node.fill,
                border: node.border,
            });
        }

        self.select_node_labels(frame, viewport, measure);
        self.place_edge_labels(frame, viewport, measure);

        // Calque de survol : nœud survolé, puis nœud sélectionné.
        let mut hover_layer = Vec::with_capacity(2);
        if let Some(h) = frame.hovered {
            hover_layer.push(h);
        }
        if let Some(h) = frame.highlighted
            && frame.hovered != Some(h)
        {
            hover_layer.push(h);
        }
        for n in hover_layer {
            let Some(node) = frame.nodes.get(n as usize) else {
                continue;
            };
            if !node.visible {
                continue;
            }
            let (x, y, r) = self.node_screen[n as usize];
            if let Some(label) = self.node_label(frame, n, measure) {
                self.hover_labels.push(label);
            }
            self.hover_nodes.push(NodePrim {
                node: n,
                center: (x, y),
                radius: r,
                fill: node.fill,
                border: node.border,
            });
        }
    }

    /// Ajoute la géométrie de l'arête `index`.
    fn push_edge(&mut self, index: u32, frame: &GraphFrame, viewport: &Viewport) {
        let edge = &frame.edges[index as usize];
        let (sx, sy, sr) = self.node_screen[edge.source as usize];
        let (tx, ty, tr) = self.node_screen[edge.target as usize];
        let (s, t) = ((sx, sy), (tx, ty));
        let length = dist(s, t);
        // Sous les disques des deux extrémités : invisible.
        if length.is_nan() || length <= sr + tr || length < 0.5 {
            return;
        }
        let thickness = viewport.scale_size(edge.size).max(MIN_EDGE_THICKNESS);
        let head = ARROW_LENGTH_RATIO * thickness;
        let control = edge
            .curved
            .then(|| curve_control_point(s, t, edge.curvature));

        // Hors de la zone de dessin (boîte des points de contrôle).
        let margin = thickness * ARROW_WIDENESS_RATIO;
        let (mut min_x, mut max_x) = (s.0.min(t.0), s.0.max(t.0));
        let (mut min_y, mut max_y) = (s.1.min(t.1), s.1.max(t.1));
        if let Some(c) = control {
            min_x = min_x.min(c.0);
            max_x = max_x.max(c.0);
            min_y = min_y.min(c.1);
            max_y = max_y.max(c.1);
        }
        if max_x + margin < 0.0
            || min_x - margin > self.width
            || max_y + margin < 0.0
            || min_y - margin > self.height
        {
            return;
        }

        let prim = match control {
            None => {
                let (ux, uy) = ((t.0 - s.0) / length, (t.1 - s.1) / length);
                let (nx, ny) = (-uy, ux);
                let tip = (t.0 - ux * tr, t.1 - uy * tr);
                let base = (t.0 - ux * (tr + head), t.1 - uy * (tr + head));
                let half = thickness * ARROW_WIDENESS_RATIO / 2.0;
                let shape =
                    (length > tr + head).then_some(EdgeShape::Segment { from: s, to: base });
                EdgePrim {
                    edge: index,
                    color: edge.color,
                    z: edge.z,
                    thickness,
                    shape,
                    arrow: Some([
                        tip,
                        (base.0 + nx * half, base.1 + ny * half),
                        (base.0 - nx * half, base.1 - ny * half),
                    ]),
                    hit_from: s,
                    hit_to: tip,
                }
            }
            Some(c) => self.curved_edge(index, edge.color, edge.z, thickness, s, c, t, tr),
        };
        self.edges.push(prim);
    }

    /// Arête courbe : courbe aplatie jusqu'à la base de la flèche, flèche
    /// entre les cercles de rayon `r + 2,5 × épaisseur` et `r` autour de la
    /// cible.
    #[allow(clippy::too_many_arguments)]
    fn curved_edge(
        &mut self,
        index: u32,
        color: Rgb,
        z: u8,
        thickness: f32,
        s: Pt,
        c: Pt,
        t: Pt,
        target_radius: f32,
    ) -> EdgePrim {
        let screen_length = dist(s, c) + dist(c, t);
        let segments = ((screen_length / 10.0).ceil() as usize).clamp(4, 32);
        let head_radius = target_radius + ARROW_LENGTH_RATIO * thickness;
        let start = self.points.len() as u32;

        // Points jusqu'à la sortie du cercle de la flèche.
        let mut base: Option<Pt> = None;
        let mut previous = s;
        self.points.push(s);
        for i in 1..=segments {
            let p = quad_point(i as f32 / segments as f32, s, c, t);
            if dist(p, t) < head_radius {
                // Entrée dans le cercle : base de la flèche par interpolation.
                let d0 = dist(previous, t);
                let d1 = dist(p, t);
                let k = if d0 > d1 {
                    ((d0 - head_radius) / (d0 - d1)).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let b = (
                    previous.0 + (p.0 - previous.0) * k,
                    previous.1 + (p.1 - previous.1) * k,
                );
                self.points.push(b);
                base = Some(b);
                break;
            }
            self.points.push(p);
            previous = p;
        }
        let len = self.points.len() as u32 - start;
        let base = base.unwrap_or(t);
        let shape = (len >= 2 && dist(s, base) > 0.0).then_some(EdgeShape::Polyline { start, len });
        if shape.is_none() {
            self.points.truncate(start as usize);
        }

        // Pointe : intersection de la fin de courbe avec le cercle de la
        // cible, approchée par la corde depuis la base.
        let (bx, by) = (t.0 - base.0, t.1 - base.1);
        let bl = bx.hypot(by);
        let arrow = (bl > target_radius && bl > 0.0).then(|| {
            let (ux, uy) = (bx / bl, by / bl);
            let tip = (t.0 - ux * target_radius, t.1 - uy * target_radius);
            let half = thickness * ARROW_WIDENESS_RATIO / 2.0;
            [
                tip,
                (base.0 - uy * half, base.1 + ux * half),
                (base.0 + uy * half, base.1 - ux * half),
            ]
        });
        let hit_to = arrow.map_or(base, |a| a[0]);
        EdgePrim {
            edge: index,
            color,
            z,
            thickness,
            shape,
            arrow,
            hit_from: s,
            hit_to,
        }
    }

    /// Label du nœud `n` (`drawNodeLabel`), s'il en a un.
    fn node_label(
        &self,
        frame: &GraphFrame,
        n: u32,
        measure: &mut dyn TextMeasure,
    ) -> Option<NodeLabelPrim> {
        let text = frame.node_labels.get(n as usize)?.as_ref()?;
        if text.is_empty() {
            return None;
        }
        let (x, y, r) = self.node_screen[n as usize];
        let width = measure.width(text, NODE_LABEL_SIZE);
        let b = node_label_box(x, y, r, width, NODE_LABEL_SIZE);
        Some(NodeLabelPrim {
            node: n,
            rect: (b.x, b.y, b.width, b.height),
            text_origin: (b.text_x, b.text_y),
            text: text.clone(),
        })
    }

    /// Grille des labels de Sigma (`LabelGrid`, `renderLabels`).
    fn select_node_labels(
        &mut self,
        frame: &GraphFrame,
        viewport: &Viewport,
        measure: &mut dyn TextMeasure,
    ) {
        let null_camera = viewport.with_camera(CameraState::default());
        let columns = (self.width / LABEL_GRID_CELL_SIZE).ceil() as i64;
        self.grid.clear();
        for (i, node) in frame.nodes.iter().enumerate() {
            if !node.visible || frame.node_labels[i].is_none() {
                continue;
            }
            let (x, y) = null_camera.graph_to_viewport(node.x, node.y);
            let cx = (x / LABEL_GRID_CELL_SIZE).floor() as i64;
            let cy = (y / LABEL_GRID_CELL_SIZE).floor() as i64;
            self.grid.push((cy * columns + cx, i as u32));
        }
        // Par cellule : taille décroissante puis identifiant croissant
        // (`LabelCandidate.compare`).
        let nodes = &frame.nodes;
        let ids = &frame.node_ids;
        self.grid.sort_unstable_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| {
                    let (sa, sb) = (nodes[a.1 as usize].size, nodes[b.1 as usize].size);
                    sb.total_cmp(&sa)
                })
                .then_with(|| ids[a.1 as usize].cmp(&ids[b.1 as usize]))
        });
        let ratio = viewport.camera.ratio;
        let per_cell = (LABEL_DENSITY / (ratio * ratio)).ceil().max(1.0) as usize;

        let grid = std::mem::take(&mut self.grid);
        let mut current_cell = None;
        let mut taken = 0usize;
        for &(cell, n) in &grid {
            if current_cell != Some(cell) {
                current_cell = Some(cell);
                taken = 0;
            }
            if taken >= per_cell {
                continue;
            }
            taken += 1;
            let (x, y, r) = self.node_screen[n as usize];
            if r < LABEL_RENDERED_SIZE_THRESHOLD {
                continue;
            }
            if x < -X_LABEL_MARGIN
                || x > self.width + X_LABEL_MARGIN
                || y < -Y_LABEL_MARGIN
                || y > self.height + Y_LABEL_MARGIN
            {
                continue;
            }
            self.displayed[n as usize] = true;
            if let Some(label) = self.node_label(frame, n, measure) {
                self.node_labels.push(label);
            }
        }
        self.grid = grid;
    }

    /// Labels d'arêtes (`edgeLabelsToDisplayFromNodes`, `renderEdgeLabels`).
    fn place_edge_labels(
        &mut self,
        frame: &GraphFrame,
        viewport: &Viewport,
        measure: &mut dyn TextMeasure,
    ) {
        if frame.edge_labels.iter().all(Option::is_none) {
            return;
        }
        let special = |n: u32| frame.hovered == Some(n) || frame.highlighted == Some(n);
        for (i, edge) in frame.edges.iter().enumerate() {
            let Some(label) = frame.edge_labels[i].as_ref() else {
                continue;
            };
            if label.is_empty() || !frame.edge_drawable(edge) {
                continue;
            }
            let (src, tgt) = (edge.source, edge.target);
            let worthy = special(src)
                || special(tgt)
                || (self.displayed[src as usize] && self.displayed[tgt as usize]);
            if !worthy {
                continue;
            }
            let (sx, sy, sr) = self.node_screen[src as usize];
            let (tx, ty, tr) = self.node_screen[tgt as usize];
            let size = viewport.scale_size(edge.size);
            let prim = if edge.curved {
                curved_label(
                    i as u32,
                    label,
                    (sx, sy, sr),
                    (tx, ty, tr),
                    size,
                    edge.curvature,
                    measure,
                )
            } else {
                straight_label(i as u32, label, (sx, sy, sr), (tx, ty, tr), size, measure)
            };
            if let Some(prim) = prim {
                let (x, y) = prim.horizontal_center;
                if x >= -prim.width
                    && x <= self.width + prim.width
                    && y >= -20.0
                    && y <= self.height + 20.0
                {
                    self.edge_labels.push(prim);
                }
            }
        }
    }

    /// Nœud sous le point `p` : le calque de survol d'abord, puis le dernier
    /// nœud dessiné.
    pub fn node_at(&self, p: Pt) -> Option<u32> {
        let hit = |n: &&NodePrim| {
            let (dx, dy) = (p.0 - n.center.0, p.1 - n.center.1);
            dx * dx + dy * dy <= n.radius * n.radius
        };
        self.hover_nodes
            .iter()
            .rev()
            .find(hit)
            .or_else(|| self.nodes.iter().rev().find(hit))
            .map(|n| n.node)
    }

    /// Arête sous le point `p`, à `tolerance` pixels près de son trait :
    /// la dernière dessinée (plan le plus haut).
    pub fn edge_at(&self, p: Pt, tolerance: f32) -> Option<u32> {
        self.edges
            .iter()
            .rev()
            .find(|e| {
                let reach = e.thickness / 2.0 + tolerance;
                let reach_sq = reach * reach;
                match e.shape {
                    Some(EdgeShape::Polyline { start, len }) => {
                        let pts = &self.points[start as usize..(start + len) as usize];
                        pts.windows(2)
                            .any(|w| segment_distance_sq(p, w[0], w[1]) <= reach_sq)
                            || e.arrow.is_some_and(|a| {
                                segment_distance_sq(p, pts[pts.len() - 1], a[0]) <= reach_sq
                            })
                    }
                    _ => segment_distance_sq(p, e.hit_from, e.hit_to) <= reach_sq,
                }
            })
            .map(|e| e.edge)
    }
}

/// `drawStraightEdgeLabel`.
fn straight_label(
    edge: u32,
    label: &Arc<str>,
    source: (f32, f32, f32),
    target: (f32, f32, f32),
    edge_size: f32,
    measure: &mut dyn TextMeasure,
) -> Option<EdgeLabelPrim> {
    let (mut sx, mut sy, s_size) = source;
    let (mut tx, mut ty, t_size) = target;
    let (mut dx, mut dy) = (tx - sx, ty - sy);
    let mut d = dx.hypot(dy);
    if d < s_size + t_size {
        return None;
    }
    sx += dx * s_size / d;
    sy += dy * s_size / d;
    tx -= dx * t_size / d;
    ty -= dy * t_size / d;
    let center = ((sx + tx) / 2.0, (sy + ty) / 2.0);
    dx = tx - sx;
    dy = ty - sy;
    d = dx.hypot(dy);
    let (text, width) = fit_label(label, d, EDGE_LABEL_SIZE, measure)?;
    let angle = if dx > 0.0 {
        if dy > 0.0 {
            (dx / d).acos()
        } else {
            (dy / d).asin()
        }
    } else if dy > 0.0 {
        (dx / d).acos() + std::f32::consts::PI
    } else {
        (dx / d).asin() + std::f32::consts::FRAC_PI_2
    };
    let offset_y = edge_size / 2.0 + EDGE_LABEL_SIZE;
    // Centre du texte dans le repère tourné : (0, ligne de base − 0,35 em).
    let local_y = offset_y - 0.35 * EDGE_LABEL_SIZE;
    let (sin, cos) = angle.sin_cos();
    let horizontal_center = (center.0 - sin * local_y, center.1 + cos * local_y);
    Some(EdgeLabelPrim {
        edge,
        text,
        width,
        placement: EdgeLabelPlacement::Straight {
            center,
            angle,
            offset_y,
        },
        horizontal_center,
    })
}

/// `drawCurvedEdgeLabel` (`keepLabelUpright`).
fn curved_label(
    edge: u32,
    label: &Arc<str>,
    source: (f32, f32, f32),
    target: (f32, f32, f32),
    edge_size: f32,
    curvature: f32,
    measure: &mut dyn TextMeasure,
) -> Option<EdgeLabelPrim> {
    // `edgeData[curvatureAttribute] || defaultCurvature`.
    let curvature = if curvature == 0.0 {
        lunar_ui_logic::graph::style::DEFAULT_EDGE_CURVATURE
    } else {
        curvature
    };
    let ltr = source.0 < target.0;
    let (a, b) = if ltr {
        (source, target)
    } else {
        (target, source)
    };
    let (mut sx, mut sy) = (a.0, a.1);
    let (mut tx, mut ty) = (b.0, b.1);
    let (cx, cy) = ((sx + tx) / 2.0, (sy + ty) / 2.0);
    let (diff_x, diff_y) = (tx - sx, ty - sy);
    let diff = diff_x.hypot(diff_y);
    if diff <= 0.0 {
        return None;
    }
    let orientation = if ltr { 1.0 } else { -1.0 };
    let mut ax = cx + diff_y * curvature * orientation;
    let mut ay = cy - diff_x * curvature * orientation;

    let offset = edge_size * 0.7 + 5.0;
    let (sox, soy) = (ay - sy, -(ax - sx));
    let sol = sox.hypot(soy).max(f32::MIN_POSITIVE);
    let (tox, toy) = (ty - ay, -(tx - ax));
    let tol = tox.hypot(toy).max(f32::MIN_POSITIVE);
    sx += offset * sox / sol;
    sy += offset * soy / sol;
    tx += offset * tox / tol;
    ty += offset * toy / tol;
    ax += offset * diff_y / diff;
    ay -= offset * diff_x / diff;

    let (sp, anchor, tp) = ((sx, sy), (ax, ay), (tx, ty));
    let length = curve_length(sp, anchor, tp);
    // Tailles d'écran des extrémités, dans l'ordre d'origine (Sigma).
    if length < source.2 + target.2 {
        return None;
    }
    let available = length - source.2 - target.2;
    let (text, width) = fit_label(label, available, EDGE_LABEL_SIZE, measure)?;
    let t0 = 0.5 - width / length / 2.0;
    let mid = quad_point(0.5, sp, anchor, tp);
    Some(EdgeLabelPrim {
        edge,
        text,
        width,
        placement: EdgeLabelPlacement::Curved {
            source: sp,
            anchor,
            target: tp,
            t0,
            curve_length: length,
            curvature,
        },
        horizontal_center: (mid.0, mid.1 - 0.35 * EDGE_LABEL_SIZE),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::graph_view::frame::{FrameEdge, FrameNode};
    use lunar_ui_logic::graph::GraphModel;
    use lunar_ui_logic::graph::camera::Bounds;

    /// Mesure fixe : 7 px par caractère à 13 px, 6 px à 11 px.
    pub(crate) struct FixedMeasure;

    impl TextMeasure for FixedMeasure {
        fn width(&mut self, text: &str, font_size: f32) -> f32 {
            text.chars().count() as f32 * font_size * 0.55
        }
    }

    /// Instantané fabriqué à la main : nœuds `(x, y, taille, label)`.
    pub(crate) fn frame(nodes: &[(f32, f32, f32, &str)], edges: &[(u32, u32, bool)]) -> GraphFrame {
        // Rangs du modèle obtenus d'un vrai modèle (types opaques).
        let mut model = GraphModel::with_seed(1);
        let ids: Vec<String> = (0..nodes.len()).map(|i| format!("n{i:03}")).collect();
        let core_nodes: Vec<_> = ids
            .iter()
            .map(|id| crate::graph_view::frame::tests::node(id, "#2196F3"))
            .collect();
        let core_edges: Vec<_> = edges
            .iter()
            .map(|&(s, t, _)| {
                crate::graph_view::frame::tests::edge(&ids[s as usize], &ids[t as usize], "TCP", 1)
            })
            .collect();
        model.load_snapshot(&crate::graph_view::frame::tests::snapshot(
            &core_nodes,
            &core_edges,
        ));
        let mut f = GraphFrame::default();
        f.refresh(&model, None);
        let model_edges = f.edges.clone();
        f.nodes = nodes
            .iter()
            .enumerate()
            .map(|(i, &(x, y, size, _))| FrameNode {
                x,
                y,
                size,
                ..f.nodes[i]
            })
            .collect();
        f.node_ids = ids.iter().map(|s| Arc::from(s.as_str())).collect();
        f.node_labels = nodes
            .iter()
            .map(|&(_, _, _, l)| (!l.is_empty()).then(|| Arc::from(l)))
            .collect();
        f.edges = edges
            .iter()
            .zip(model_edges)
            .map(|(&(s, t, curved), e)| FrameEdge {
                source: s,
                target: t,
                size: 1.0,
                curved,
                curvature: if curved { 0.25 } else { 0.0 },
                z: 0,
                ..e
            })
            .collect();
        f.edge_labels = vec![None; edges.len()];
        f.bounds = Some(Bounds {
            min_x: nodes.iter().map(|n| n.0).fold(f32::INFINITY, f32::min),
            min_y: nodes.iter().map(|n| n.1).fold(f32::INFINITY, f32::min),
            max_x: nodes.iter().map(|n| n.0).fold(f32::NEG_INFINITY, f32::max),
            max_y: nodes.iter().map(|n| n.1).fold(f32::NEG_INFINITY, f32::max),
        });
        f
    }

    fn viewport(f: &GraphFrame, ratio: f32) -> Viewport {
        Viewport::new(
            800.0,
            600.0,
            f.bounds,
            CameraState {
                ratio,
                ..CameraState::default()
            },
        )
    }

    #[test]
    fn fleche_droite_comme_sigma() {
        let f = frame(
            &[(-100.0, 0.0, 10.0, "a"), (100.0, 0.0, 10.0, "b")],
            &[(0, 1, false)],
        );
        let mut scene = Scene::default();
        scene.build(&f, &viewport(&f, 1.0), &mut FixedMeasure);
        assert_eq!(scene.edges.len(), 1);
        let e = scene.edges[0];
        // Épaisseur minimale 1,7 px ; flèche 2,5 × plus longue.
        assert_eq!(e.thickness, MIN_EDGE_THICKNESS);
        let (tx, _, tr) = scene.node_screen[1];
        let arrow = e.arrow.expect("flèche");
        assert!(
            (arrow[0].0 - (tx - tr)).abs() < 1e-3,
            "pointe sur la bordure"
        );
        let Some(EdgeShape::Segment { to, .. }) = e.shape else {
            panic!("segment attendu");
        };
        assert!((to.0 - (tx - tr - 2.5 * MIN_EDGE_THICKNESS)).abs() < 1e-3);
        // Base de la flèche : 2 × l'épaisseur.
        assert!((dist(arrow[1], arrow[2]) - 2.0 * MIN_EDGE_THICKNESS).abs() < 1e-3);
        // Test d'atteinte sur le trait.
        let mid = ((scene.node_screen[0].0 + tx) / 2.0, scene.node_screen[0].1);
        assert_eq!(scene.edge_at((mid.0, mid.1 + 1.5), 1.0), Some(0));
        assert_eq!(scene.edge_at((mid.0, mid.1 + 10.0), 1.0), None);
        assert_eq!(scene.node_at((tx, scene.node_screen[1].1)), Some(1));
    }

    #[test]
    fn courbe_aplatie_jusqu_a_la_fleche() {
        let f = frame(
            &[(-100.0, 0.0, 10.0, "a"), (100.0, 0.0, 10.0, "b")],
            &[(0, 1, true)],
        );
        let mut scene = Scene::default();
        scene.build(&f, &viewport(&f, 1.0), &mut FixedMeasure);
        let e = scene.edges[0];
        let Some(EdgeShape::Polyline { start, len }) = e.shape else {
            panic!("courbe attendue");
        };
        let pts = &scene.points[start as usize..(start + len) as usize];
        // De gauche à droite à l'écran : (dy, −dx) × 0,25 relève le
        // sommet (y vers le bas).
        let apex = pts.iter().map(|p| p.1).fold(f32::INFINITY, f32::min);
        assert!(apex < scene.node_screen[0].1 - 50.0, "sommet {apex}");
        let (tx, ty, tr) = scene.node_screen[1];
        let tip = e.arrow.expect("flèche")[0];
        assert!((dist(tip, (tx, ty)) - tr).abs() < 0.5);
        let base = pts[pts.len() - 1];
        assert!((dist(base, (tx, ty)) - (tr + 2.5 * e.thickness)).abs() < 0.5);
    }

    #[test]
    fn grille_des_labels() {
        // Deux nœuds dans la même cellule : seul le plus gros garde son
        // label au ratio 1 ; au ratio 0,5, quatre labels par cellule.
        let f = frame(
            &[
                (0.0, 0.0, 10.0, "petit"),
                (0.1, 0.1, 20.0, "gros"),
                (1000.0, 1000.0, 10.0, "loin"),
                (500.0, 500.0, 2.0, "minuscule"),
            ],
            &[],
        );
        let mut scene = Scene::default();
        scene.build(&f, &viewport(&f, 1.0), &mut FixedMeasure);
        let texts: Vec<_> = scene.node_labels.iter().map(|l| l.text.as_ref()).collect();
        assert!(texts.contains(&"gros") && texts.contains(&"loin"));
        assert!(!texts.contains(&"petit"));
        assert!(!texts.contains(&"minuscule"), "moins de 6 px");

        let mut zoomed = Scene::default();
        let vp = viewport(&f, 0.5).with_camera(CameraState {
            x: 0.0,
            y: 0.0,
            ratio: 0.5,
        });
        zoomed.build(&f, &vp, &mut FixedMeasure);
        let texts: Vec<_> = zoomed.node_labels.iter().map(|l| l.text.as_ref()).collect();
        assert!(texts.contains(&"petit") && texts.contains(&"gros"));

        // Boîte de `drawNodeLabel` : texte + 10 px, police + 6 px.
        let l = &scene.node_labels[0];
        assert_eq!(l.rect.3, NODE_LABEL_SIZE + 6.0);
        assert_eq!(
            l.rect.2,
            FixedMeasure.width(&l.text, NODE_LABEL_SIZE) + 10.0
        );
    }

    #[test]
    fn labels_d_aretes_et_abreviation() {
        let mut f = frame(
            &[(-100.0, 0.0, 10.0, "a"), (100.0, 0.0, 10.0, "b")],
            &[(0, 1, false)],
        );
        // 700 px entre les bordures : 130 caractères (786 px) n'y tiennent pas.
        let long = format!("TCP :{}", "80,443,".repeat(18));
        f.edge_labels = vec![Some(Arc::from(long.as_str()))];
        let mut scene = Scene::default();
        scene.build(&f, &viewport(&f, 1.0), &mut FixedMeasure);
        assert_eq!(scene.edge_labels.len(), 1);
        let label = &scene.edge_labels[0];
        assert!(label.text.ends_with('…'), "{}", label.text);
        let EdgeLabelPlacement::Straight { angle, .. } = label.placement else {
            panic!("droit attendu");
        };
        assert_eq!(angle, 0.0);

        // Sans label sur une extrémité (ni survol, ni sélection), rien.
        f.node_labels[1] = None;
        scene.build(&f, &viewport(&f, 1.0), &mut FixedMeasure);
        assert!(scene.edge_labels.is_empty());
        f.hovered = Some(1);
        scene.build(&f, &viewport(&f, 1.0), &mut FixedMeasure);
        assert_eq!(scene.edge_labels.len(), 1);
    }

    #[test]
    fn calque_de_survol() {
        let mut f = frame(&[(-100.0, 0.0, 10.0, "a"), (100.0, 0.0, 3.0, "b")], &[]);
        f.hovered = Some(1);
        f.highlighted = Some(0);
        let mut scene = Scene::default();
        scene.build(&f, &viewport(&f, 1.0), &mut FixedMeasure);
        // « b » est trop petit pour la grille mais son label s'affiche au
        // survol ; le nœud sélectionné suit.
        let hover: Vec<_> = scene.hover_labels.iter().map(|l| l.text.as_ref()).collect();
        assert_eq!(hover, ["b", "a"]);
        assert_eq!(scene.hover_nodes.len(), 2);
        assert_eq!(scene.node_at(scene.hover_nodes[0].center), Some(1));
    }
}
