//! Dessin de la [`Scene`] avec gpui.
//!
//! - Arêtes : triangles émis directement (`Path::push_triangle`, sans
//!   tessellation lyon) et regroupés en un chemin par couple (plan, couleur),
//!   soit une vingtaine de chemins par image quel que soit le nombre
//!   d'arêtes ; les couleurs sont opaques, les recouvrements ne foncent pas.
//! - Nœuds : un quad arrondi par nœud, bordure de 10 % du rayon.
//! - Texte : lignes mises en forme par `shape_line` (police embarquée) et
//!   gardées d'une image à l'autre tant qu'elles servent ; la mesure de la
//!   scène et le dessin partagent la même mise en forme.
//!
//! Les labels d'arêtes sont posés à l'horizontale, centrés sous le milieu de
//! l'arête : gpui 0.2.2 ne sait pas tourner un glyphe (l'export PNG, lui,
//! les tourne comme Sigma).

use std::collections::HashMap;
use std::sync::Arc;

use gpui::{
    App, BorderStyle, Bounds, CursorStyle, Hsla, Path, Pixels, Point, Rgba, ShapedLine,
    SharedString, TextRun, Window, WindowTextSystem, font, point, px, quad, size,
};
use lunar_ui_logic::graph::Rgb;
use lunar_ui_logic::graph::style::{EDGE_LABEL_SIZE, NODE_LABEL_SIZE};

use super::font::LABEL_FONT_FAMILY;
use super::scene::{EdgeShape, NODE_BORDER_RATIO, NodeLabelPrim, NodePrim, Pt, Scene, TextMeasure};

/// Fond de la zone de dessin (noir).
const BACKGROUND: Rgb = Rgb { r: 0, g: 0, b: 0 };
/// Couleur du texte des labels de nœuds (`labelColor`).
const NODE_LABEL_TEXT: u32 = 0xffffff;
/// Couleur du texte des labels d'arêtes (`edgeLabelColor`).
const EDGE_LABEL_TEXT: u32 = 0xe0e0e0;
/// Fond des labels de nœuds, `#000000CC`.
const NODE_LABEL_BACKGROUND: Rgba = Rgba {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 204.0 / 255.0,
};

/// Couleur gpui d'une couleur du modèle.
pub fn hsla(c: Rgb) -> Hsla {
    Rgba {
        r: f32::from(c.r) / 255.0,
        g: f32::from(c.g) / 255.0,
        b: f32::from(c.b) / 255.0,
        a: 1.0,
    }
    .into()
}

/// Famille de lignes de texte : taille et couleur.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum TextKind {
    /// Label de nœud, 13 px blanc.
    Node,
    /// Label d'arête, 11 px #E0E0E0.
    Edge,
}

impl TextKind {
    /// Famille d'après la taille de police demandée par la scène.
    fn for_size(font_size: f32) -> Self {
        if (font_size - EDGE_LABEL_SIZE).abs() < 0.01 {
            TextKind::Edge
        } else {
            TextKind::Node
        }
    }

    /// Taille de police.
    fn font_size(self) -> f32 {
        match self {
            TextKind::Node => NODE_LABEL_SIZE,
            TextKind::Edge => EDGE_LABEL_SIZE,
        }
    }

    /// Couleur du texte.
    fn color(self) -> Hsla {
        gpui::rgb(match self {
            TextKind::Node => NODE_LABEL_TEXT,
            TextKind::Edge => EDGE_LABEL_TEXT,
        })
        .into()
    }
}

/// Lignes mises en forme, sur deux générations : une ligne qui ne sert plus
/// pendant une image entière est oubliée.
#[derive(Default)]
struct TextCache {
    /// Lignes servies à cette image.
    current: HashMap<(TextKind, SharedString), ShapedLine>,
    /// Lignes de l'image précédente.
    previous: HashMap<(TextKind, SharedString), ShapedLine>,
}

impl TextCache {
    /// Ligne de `text`, mise en forme si besoin.
    fn line(&mut self, system: &WindowTextSystem, kind: TextKind, text: &str) -> &ShapedLine {
        let key = (kind, SharedString::from(text.to_string()));
        if !self.current.contains_key(&key) {
            let line = match self.previous.remove(&key) {
                Some(line) => line,
                None => {
                    let run = TextRun {
                        len: text.len(),
                        font: font(LABEL_FONT_FAMILY),
                        color: kind.color(),
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    };
                    system.shape_line(key.1.clone(), px(kind.font_size()), &[run], None)
                }
            };
            self.current.insert(key.clone(), line);
        }
        &self.current[&key]
    }

    /// Fin d'image : les lignes non servies passent à la génération oubliée.
    fn end_frame(&mut self) {
        self.previous = std::mem::take(&mut self.current);
    }
}

/// Mesure par la mise en forme de gpui (celle qui sera dessinée).
struct GpuiMeasure<'a> {
    /// Système de texte de la fenêtre.
    system: &'a WindowTextSystem,
    /// Lignes mises en forme.
    cache: &'a mut TextCache,
}

impl TextMeasure for GpuiMeasure<'_> {
    fn width(&mut self, text: &str, font_size: f32) -> f32 {
        f32::from(
            self.cache
                .line(self.system, TextKind::for_size(font_size), text)
                .width,
        )
    }
}

/// Triangles d'un couple (plan, couleur).
struct Bucket {
    /// Couleur.
    color: Rgb,
    /// Triangles, en pixels de la zone de dessin.
    triangles: Vec<[Pt; 3]>,
}

/// Mesures du dessin, cumulées sur la vie de la vue.
#[derive(Clone, Copy, Debug, Default)]
pub struct PaintStats {
    /// Images dessinées.
    pub frames: u64,
    /// Temps cumulé de construction de la scène (µs).
    pub build_micros: u64,
    /// Temps cumulé d'émission des primitives gpui (µs).
    pub paint_micros: u64,
    /// Pire temps construction + émission (µs).
    pub worst_micros: u64,
    /// Part des arêtes dans l'émission (µs).
    pub edges_micros: u64,
    /// Part des nœuds dans l'émission (µs).
    pub nodes_micros: u64,
    /// Part des labels et du calque de survol dans l'émission (µs).
    pub labels_micros: u64,
    /// Triangles d'arêtes de la dernière image.
    pub triangles: usize,
}

/// Peintre de la vue : dernière scène, cache de texte, seaux de triangles.
pub struct Painter {
    /// Scène de la dernière image (test d'atteinte des événements souris).
    pub scene: Scene,
    /// Coin haut gauche de la zone de dessin, en coordonnées de fenêtre.
    pub origin: Point<Pixels>,
    /// Curseur à afficher au-dessus de la zone de dessin.
    pub cursor: CursorStyle,
    /// Mesures.
    pub stats: PaintStats,
    /// Durée de la dernière construction (µs).
    last_build: u64,
    /// Lignes de texte mises en forme.
    text: TextCache,
    /// Seaux de triangles, réutilisés d'une image à l'autre.
    buckets: Vec<Bucket>,
    /// Rang du seau de chaque couple (plan, couleur).
    bucket_index: HashMap<(u8, Rgb), usize>,
    /// Seaux à dessiner, dans l'ordre.
    draw_order: Vec<usize>,
}

impl Default for Painter {
    fn default() -> Self {
        Self {
            scene: Scene::default(),
            origin: Point::default(),
            cursor: CursorStyle::Arrow,
            stats: PaintStats::default(),
            last_build: 0,
            text: TextCache::default(),
            buckets: Vec::new(),
            bucket_index: HashMap::new(),
            draw_order: Vec::new(),
        }
    }
}

/// Deux triangles d'un trait épais de `a` à `b`.
fn push_segment(out: &mut Vec<[Pt; 3]>, a: Pt, b: Pt, half: f32) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = dx.hypot(dy);
    if len <= f32::EPSILON {
        return;
    }
    let (nx, ny) = (-dy / len * half, dx / len * half);
    let a1 = (a.0 + nx, a.1 + ny);
    let a2 = (a.0 - nx, a.1 - ny);
    let b1 = (b.0 + nx, b.1 + ny);
    let b2 = (b.0 - nx, b.1 - ny);
    out.push([a1, a2, b2]);
    out.push([a1, b2, b1]);
}

/// Bande de triangles d'une polyligne épaisse, joints en onglet (bornés).
fn push_polyline(out: &mut Vec<[Pt; 3]>, pts: &[Pt], half: f32) {
    if pts.len() < 2 {
        return;
    }
    let normal = |a: Pt, b: Pt| -> Option<Pt> {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len = dx.hypot(dy);
        (len > f32::EPSILON).then(|| (-dy / len, dx / len))
    };
    let mut previous: Option<(Pt, Pt)> = None;
    for i in 0..pts.len() {
        let before = (i > 0).then(|| normal(pts[i - 1], pts[i])).flatten();
        let after = (i + 1 < pts.len())
            .then(|| normal(pts[i], pts[i + 1]))
            .flatten();
        let n = match (before, after) {
            (Some(a), Some(b)) => {
                let (sx, sy) = (a.0 + b.0, a.1 + b.1);
                let sl = sx.hypot(sy);
                if sl <= f32::EPSILON {
                    a
                } else {
                    let m = (sx / sl, sy / sl);
                    // Onglet : 1 / cos(demi-angle), borné à 2.
                    let k = 1.0 / (m.0 * a.0 + m.1 * a.1).max(0.5);
                    (m.0 * k, m.1 * k)
                }
            }
            (Some(a), None) | (None, Some(a)) => a,
            (None, None) => continue,
        };
        let p = pts[i];
        let left = (p.0 + n.0 * half, p.1 + n.1 * half);
        let right = (p.0 - n.0 * half, p.1 - n.1 * half);
        if let Some((pl, pr)) = previous {
            out.push([pl, pr, right]);
            out.push([pl, right, left]);
        }
        previous = Some((left, right));
    }
}

impl Painter {
    /// Construit la scène de l'image avec la mise en forme de la fenêtre.
    pub fn build(
        &mut self,
        frame: &super::frame::GraphFrame,
        viewport: &super::camera::Viewport,
        window: &Window,
    ) {
        let started = std::time::Instant::now();
        let system = window.text_system().clone();
        let mut measure = GpuiMeasure {
            system: &system,
            cache: &mut self.text,
        };
        self.scene.build(frame, viewport, &mut measure);

        // Triangles des arêtes, par couple (plan, couleur), dans l'ordre de
        // la scène (plan 0 puis plan 1).
        for bucket in &mut self.buckets {
            bucket.triangles.clear();
        }
        let mut order = std::mem::take(&mut self.draw_order);
        order.clear();
        for edge in &self.scene.edges {
            let key = (edge.z, edge.color);
            let index = match self.bucket_index.get(&key) {
                Some(&i) => i,
                None => {
                    self.buckets.push(Bucket {
                        color: edge.color,
                        triangles: Vec::new(),
                    });
                    let i = self.buckets.len() - 1;
                    self.bucket_index.insert(key, i);
                    i
                }
            };
            if self.buckets[index].triangles.is_empty() {
                order.push(index);
            }
            let half = edge.thickness / 2.0;
            let out = &mut self.buckets[index].triangles;
            match edge.shape {
                Some(EdgeShape::Segment { from, to }) => push_segment(out, from, to, half),
                Some(EdgeShape::Polyline { start, len }) => {
                    let pts = &self.scene.points[start as usize..(start + len) as usize];
                    push_polyline(out, pts, half);
                }
                None => {}
            }
            if let Some(arrow) = edge.arrow {
                out.push(arrow);
            }
        }
        // Ordre de dessin : première apparition, donc plan 0 puis plan 1
        // (la scène range les arêtes par plan).
        self.draw_order = order;
        self.last_build = started.elapsed().as_micros() as u64;
        self.stats.build_micros += self.last_build;
    }

    /// Dessine la dernière scène dans `bounds`.
    ///
    /// Les nœuds, les labels et le calque de survol de Sigma deviennent des
    /// calques gpui (`paint_layer`) : leurs primitives partagent un rang de
    /// dessin, gardent leur ordre d'émission (tri stable) et évitent l'arbre
    /// des recouvrements de gpui, qui coûtait 4 µs par nœud sur 5 000 nœuds
    /// (22 ms par image).
    pub fn paint(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        let started = std::time::Instant::now();
        let o = bounds.origin;
        self.origin = o;

        // Arêtes hors calque : chaque chemin reçoit son propre rang, si bien
        // que gpui les recopie depuis sa texture intermédiaire en une seule
        // fois (rectangle englobant) au lieu d'une copie par chemin.
        let mut triangles = 0;
        window.paint_quad(gpui::fill(bounds, hsla(BACKGROUND)));
        let at = |p: Pt| point(o.x + px(p.0), o.y + px(p.1));
        let st = point(0.0, 1.0);
        for &i in &self.draw_order {
            let bucket = &self.buckets[i];
            let Some(first) = bucket.triangles.first() else {
                continue;
            };
            let mut path = Path::new(at(first[0]));
            for t in &bucket.triangles {
                path.push_triangle((at(t[0]), at(t[1]), at(t[2])), (st, st, st));
            }
            triangles += bucket.triangles.len();
            window.paint_path(path, hsla(bucket.color));
        }
        let edges_done = std::time::Instant::now();

        // Ordre des calques de Sigma : edges, edgeLabels, nodes, labels,
        // hovers, hoverNodes.
        let system = window.text_system().clone();
        window.paint_layer(bounds, |window| {
            for label in &self.scene.edge_labels {
                let line = self.text.line(&system, TextKind::Edge, &label.text).clone();
                let (cx_, cy) = label.horizontal_center;
                let baseline = cy + 0.35 * EDGE_LABEL_SIZE;
                let start = (cx_ - f32::from(line.width) / 2.0, baseline);
                paint_line(&line, start, o, window, cx);
            }
        });
        let edge_labels_done = std::time::Instant::now();

        window.paint_layer(bounds, |window| {
            for node in &self.scene.nodes {
                paint_node(node, o, window);
            }
        });
        let nodes_done = std::time::Instant::now();

        // Labels puis calque de survol : dans un calque, les fonds (quads)
        // passent sous tous les textes (sprites).
        for labels in [&self.scene.node_labels, &self.scene.hover_labels] {
            window.paint_layer(bounds, |window| {
                for label in labels {
                    paint_node_label(&mut self.text, label, &system, o, window, cx);
                }
            });
        }
        window.paint_layer(bounds, |window| {
            for node in &self.scene.hover_nodes {
                paint_node(node, o, window);
            }
        });

        self.text.end_frame();
        let micros = |d: std::time::Duration| d.as_micros() as u64;
        self.stats.edges_micros += micros(edges_done - started);
        self.stats.nodes_micros += micros(nodes_done - edge_labels_done);
        self.stats.labels_micros +=
            micros(edge_labels_done - edges_done) + micros(nodes_done.elapsed());
        let paint = started.elapsed().as_micros() as u64;
        self.stats.paint_micros += paint;
        self.stats.worst_micros = self.stats.worst_micros.max(self.last_build + paint);
        self.stats.frames += 1;
        self.stats.triangles = triangles;
    }
}

/// Fond `#000000CC` puis texte blanc (`drawNodeLabel`).
fn paint_node_label(
    text: &mut TextCache,
    label: &NodeLabelPrim,
    system: &Arc<WindowTextSystem>,
    o: Point<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let (x, y, w, h) = label.rect;
    window.paint_quad(gpui::fill(
        Bounds::new(point(o.x + px(x), o.y + px(y)), size(px(w), px(h))),
        NODE_LABEL_BACKGROUND,
    ));
    let line = text.line(system, TextKind::Node, &label.text).clone();
    paint_line(&line, label.text_origin, o, window, cx);
}

/// Disque bordé d'un nœud (`NodeBorderProgram`).
fn paint_node(node: &NodePrim, o: Point<Pixels>, window: &mut Window) {
    let r = node.radius;
    let bounds = Bounds::new(
        point(o.x + px(node.center.0 - r), o.y + px(node.center.1 - r)),
        size(px(2.0 * r), px(2.0 * r)),
    );
    window.paint_quad(quad(
        bounds,
        px(r),
        hsla(node.fill),
        px(r * NODE_BORDER_RATIO),
        hsla(node.border),
        BorderStyle::Solid,
    ));
}

/// Dessine une ligne de texte dont la ligne de base commence en `start`.
fn paint_line(line: &ShapedLine, start: Pt, o: Point<Pixels>, window: &mut Window, cx: &mut App) {
    let line_height = line.ascent + line.descent;
    let origin = point(o.x + px(start.0), o.y + px(start.1) - line.ascent);
    if let Err(err) = line.paint(origin, line_height, window, cx) {
        log::debug!("label non dessiné : {err:#}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traits_epais() {
        let mut out = Vec::new();
        push_segment(&mut out, (0.0, 0.0), (10.0, 0.0), 1.0);
        assert_eq!(out.len(), 2);
        assert!(out.iter().flatten().all(|p| p.1.abs() == 1.0));
        out.clear();
        push_polyline(&mut out, &[(0.0, 0.0), (10.0, 0.0), (20.0, 10.0)], 1.0);
        assert_eq!(out.len(), 4);
        // Onglet : le joint reste à une demi-épaisseur des deux segments.
        let joint = out[1][1];
        assert!((joint.0 - 10.0).abs() < 1.0 && (joint.1.abs() - 1.0).abs() < 0.5);
    }
}
