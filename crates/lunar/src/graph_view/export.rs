//! Export PNG hors écran, sur le modèle de `@sigma/export-image` 3.0.0 :
//! rendu temporaire de 2 × la taille de la vue (pixels
//! logiques), même caméra et mêmes reducers, × le facteur d'échelle de
//! l'écran, sur fond blanc, tous les calques (arêtes, nœuds, labels
//! d'arêtes, labels, calque de survol). Les tailles en pixels des nœuds et
//! des arêtes ne doublent pas : la même caméra cadre le graphe sur une zone
//! deux fois plus grande, et la grille des labels couvre cette zone. Le
//! nœud survolé garde sa couleur de survol (reducer) mais pas son label (le
//! rendu temporaire n'a pas de souris) ; le nœud sélectionné garde le sien.
//! La légende et les bandeaux ne font pas partie de l'image.
//!
//! Le tramage est fait par tiny-skia, sur un fil d'arrière-plan, avec la
//! police embarquée : contours des glyphes de `ttf-parser`, labels d'arêtes
//! tournés le long de l'arête (droits) ou posés caractère par caractère sur
//! la courbe (courbes), comme Sigma.

use std::collections::HashMap;
use std::path::Path;

use lunar_ui_logic::graph::Rgb;
use lunar_ui_logic::graph::style::{EDGE_LABEL_SIZE, NODE_LABEL_SIZE};
use tiny_skia::{
    Color, FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Rect, Stroke, Transform,
};

use super::camera::{CameraState, Viewport};
use super::font::{FontMeasure, LabelFont, label_font};
use super::frame::GraphFrame;
use super::scene::{
    EdgeLabelPlacement, EdgeShape, NODE_BORDER_RATIO, NodeLabelPrim, NodePrim, Scene, quad_point,
};

/// Agrandissement de la zone exportée (`width: W * 2, height: H * 2`).
pub const EXPORT_SCALE: f32 = 2.0;

/// Côté maximal de l'image, en pixels (garde-fou mémoire : 16 384² × 4 o).
const MAX_SIDE: u32 = 16_384;

/// Ce qu'il faut pour rendre l'image : instantané pris sur la vue.
#[derive(Clone, Debug)]
pub struct ExportRequest {
    /// Instantané du graphe au moment de la demande.
    pub frame: GraphFrame,
    /// Largeur de la vue, en pixels logiques.
    pub width: f32,
    /// Hauteur de la vue, en pixels logiques.
    pub height: f32,
    /// Caméra de la vue.
    pub camera: CameraState,
    /// Facteur d'échelle de l'écran (`devicePixelRatio`).
    pub pixel_ratio: f32,
}

/// Couleur tiny-skia d'une couleur du modèle.
fn color(c: Rgb) -> Color {
    Color::from_rgba8(c.r, c.g, c.b, 255)
}

/// Pinceau plein, lissé.
fn paint(c: Color) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(c);
    p.anti_alias = true;
    p
}

/// Adaptateur des contours `ttf-parser` vers un chemin tiny-skia.
struct OutlineToPath(PathBuilder);

impl ttf_parser::OutlineBuilder for OutlineToPath {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to(x, y);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.0.quad_to(x1, y1, x, y);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.0.cubic_to(x1, y1, x2, y2, x, y);
    }
    fn close(&mut self) {
        self.0.close();
    }
}

/// Glyphes déjà tracés, en unités de la police.
struct Glyphs<'f> {
    /// Police.
    font: &'f LabelFont,
    /// Contour de chaque caractère (absent : espace ou glyphe manquant).
    paths: HashMap<char, Option<tiny_skia::Path>>,
}

impl<'f> Glyphs<'f> {
    /// Cache vide.
    fn new(font: &'f LabelFont) -> Self {
        Self {
            font,
            paths: HashMap::new(),
        }
    }

    /// Dessine `c` avec sa ligne de base à l'origine de `at`, à `size` px.
    fn draw_char(&mut self, pixmap: &mut Pixmap, c: char, size: f32, at: Transform, ink: &Paint) {
        let font = self.font;
        let path = self.paths.entry(c).or_insert_with(|| {
            let mut builder = OutlineToPath(PathBuilder::new());
            if font.outline(c, &mut builder) {
                builder.0.finish()
            } else {
                None
            }
        });
        if let Some(path) = path {
            let k = size / font.units_per_em();
            pixmap.fill_path(path, ink, FillRule::Winding, at.pre_scale(k, -k), None);
        }
    }

    /// Dessine `text` sur une ligne de base partant de l'origine de `at`.
    fn draw_text(
        &mut self,
        pixmap: &mut Pixmap,
        text: &str,
        size: f32,
        at: Transform,
        ink: &Paint,
    ) {
        let mut pen = 0.0;
        for c in text.chars() {
            self.draw_char(pixmap, c, size, at.pre_translate(pen, 0.0), ink);
            pen += self.font.advance(c) * size;
        }
    }
}

/// Rend l'image de `request`.
pub fn render(request: &ExportRequest) -> Result<Pixmap, String> {
    let font = label_font().ok_or("police des labels indisponible")?;
    let width = request.width.max(1.0) * EXPORT_SCALE;
    let height = request.height.max(1.0) * EXPORT_SCALE;
    let ratio = if request.pixel_ratio.is_finite() && request.pixel_ratio > 0.0 {
        request.pixel_ratio
    } else {
        1.0
    };
    let px_width = (width * ratio).round() as u32;
    let px_height = (height * ratio).round() as u32;
    if px_width > MAX_SIDE || px_height > MAX_SIDE {
        return Err(format!(
            "image trop grande ({px_width} × {px_height} pixels, {MAX_SIDE} au plus)"
        ));
    }
    let mut pixmap =
        Pixmap::new(px_width.max(1), px_height.max(1)).ok_or("image impossible à allouer")?;
    pixmap.fill(Color::WHITE);

    // Pas de souris sur le rendu temporaire : pas de calque de survol pour
    // le nœud survolé (sa couleur de survol, elle, vient du reducer).
    let mut frame = request.frame.clone();
    frame.hovered = None;
    let viewport = Viewport::new(width, height, frame.bounds, request.camera);
    let mut scene = Scene::default();
    scene.build(&frame, &viewport, &mut FontMeasure);

    let base = Transform::from_scale(ratio, ratio);
    let mut glyphs = Glyphs::new(font);
    // Ordre des calques de Sigma : edges, edgeLabels, nodes, labels,
    // hovers, hoverNodes.
    draw_edges(&mut pixmap, &scene, base);
    draw_edge_labels(&mut pixmap, &scene, base, &mut glyphs);
    for node in &scene.nodes {
        draw_node(&mut pixmap, node, base);
    }
    for label in &scene.node_labels {
        draw_node_label(&mut pixmap, label, base, &mut glyphs);
    }
    for label in &scene.hover_labels {
        draw_node_label(&mut pixmap, label, base, &mut glyphs);
    }
    for node in &scene.hover_nodes {
        draw_node(&mut pixmap, node, base);
    }
    Ok(pixmap)
}

/// Rend l'image de `request` et l'écrit en PNG dans `path`.
pub fn write_png(request: &ExportRequest, path: &Path) -> Result<(), String> {
    let pixmap = render(request)?;
    let bytes = pixmap
        .encode_png()
        .map_err(|err| format!("encodage PNG impossible : {err}"))?;
    std::fs::write(path, bytes)
        .map_err(|err| format!("écriture de {} impossible : {err}", path.display()))
}

/// Arêtes : traits à bouts carrés, flèches pleines.
fn draw_edges(pixmap: &mut Pixmap, scene: &Scene, base: Transform) {
    let mut stroke = Stroke {
        line_cap: LineCap::Butt,
        line_join: LineJoin::Round,
        ..Stroke::default()
    };
    for edge in &scene.edges {
        let ink = paint(color(edge.color));
        let mut pb = PathBuilder::new();
        match edge.shape {
            Some(EdgeShape::Segment { from, to }) => {
                pb.move_to(from.0, from.1);
                pb.line_to(to.0, to.1);
            }
            Some(EdgeShape::Polyline { start, len }) => {
                let pts = &scene.points[start as usize..(start + len) as usize];
                if let Some((first, rest)) = pts.split_first() {
                    pb.move_to(first.0, first.1);
                    for p in rest {
                        pb.line_to(p.0, p.1);
                    }
                }
            }
            None => {}
        }
        if let Some(path) = pb.finish() {
            stroke.width = edge.thickness;
            pixmap.stroke_path(&path, &ink, &stroke, base, None);
        }
        if let Some([a, b, c]) = edge.arrow {
            let mut pb = PathBuilder::new();
            pb.move_to(a.0, a.1);
            pb.line_to(b.0, b.1);
            pb.line_to(c.0, c.1);
            pb.close();
            if let Some(path) = pb.finish() {
                pixmap.fill_path(&path, &ink, FillRule::Winding, base, None);
            }
        }
    }
}

/// Disque bordé d'un nœud.
fn draw_node(pixmap: &mut Pixmap, node: &NodePrim, base: Transform) {
    let (x, y) = node.center;
    if let Some(path) = PathBuilder::from_circle(x, y, node.radius) {
        pixmap.fill_path(
            &path,
            &paint(color(node.border)),
            FillRule::Winding,
            base,
            None,
        );
    }
    let inner = node.radius * (1.0 - NODE_BORDER_RATIO);
    if let Some(path) = PathBuilder::from_circle(x, y, inner) {
        pixmap.fill_path(
            &path,
            &paint(color(node.fill)),
            FillRule::Winding,
            base,
            None,
        );
    }
}

/// Fond `#000000CC` et texte blanc d'un label de nœud.
fn draw_node_label(
    pixmap: &mut Pixmap,
    label: &NodeLabelPrim,
    base: Transform,
    glyphs: &mut Glyphs,
) {
    let (x, y, w, h) = label.rect;
    if let Some(rect) = Rect::from_xywh(x, y, w, h) {
        pixmap.fill_rect(rect, &paint(Color::from_rgba8(0, 0, 0, 0xcc)), base, None);
    }
    let ink = paint(Color::WHITE);
    let (tx, ty) = label.text_origin;
    glyphs.draw_text(
        pixmap,
        &label.text,
        NODE_LABEL_SIZE,
        base.pre_translate(tx, ty),
        &ink,
    );
}

/// Labels d'arêtes placés comme Sigma.
fn draw_edge_labels(pixmap: &mut Pixmap, scene: &Scene, base: Transform, glyphs: &mut Glyphs) {
    let ink = paint(Color::from_rgba8(0xe0, 0xe0, 0xe0, 255));
    let size = EDGE_LABEL_SIZE;
    for label in &scene.edge_labels {
        match label.placement {
            EdgeLabelPlacement::Straight {
                center,
                angle,
                offset_y,
            } => {
                let at = base
                    .pre_translate(center.0, center.1)
                    .pre_rotate(angle.to_degrees())
                    .pre_translate(-label.width / 2.0, offset_y);
                glyphs.draw_text(pixmap, &label.text, size, at, &ink);
            }
            EdgeLabelPlacement::Curved {
                source,
                anchor,
                target,
                t0,
                curve_length,
                curvature,
            } => {
                let mut t = t0;
                let font = glyphs.font;
                for c in label.text.chars() {
                    let p = quad_point(t, source, anchor, target);
                    let tx =
                        2.0 * (1.0 - t) * (anchor.0 - source.0) + 2.0 * t * (target.0 - anchor.0);
                    let ty =
                        2.0 * (1.0 - t) * (anchor.1 - source.1) + 2.0 * t * (target.1 - anchor.1);
                    let at = base
                        .pre_translate(p.0, p.1)
                        .pre_rotate(ty.atan2(tx).to_degrees());
                    glyphs.draw_char(pixmap, c, size, at, &ink);
                    let advance = font.advance(c) * size * (1.0 + curvature * 0.35);
                    t += advance / curve_length.max(f32::MIN_POSITIVE);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph_view::frame::tests::{edge, node, snapshot};
    use lunar_ui_logic::graph::GraphModel;

    #[test]
    fn export_taille_fond_et_couleur_des_noeuds() {
        let mut model = GraphModel::with_seed(5);
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
        let request = ExportRequest {
            frame: frame.clone(),
            width: 300.0,
            height: 200.0,
            camera: CameraState::default(),
            pixel_ratio: 1.0,
        };
        let pixmap = render(&request).expect("rendu");
        assert_eq!((pixmap.width(), pixmap.height()), (600, 400));
        let at = |x: f32, y: f32| {
            let p = pixmap
                .pixel(x.round() as u32, y.round() as u32)
                .expect("pixel")
                .demultiply();
            (p.red(), p.green(), p.blue(), p.alpha())
        };
        // Fond blanc dans un coin.
        assert_eq!(at(1.0, 1.0), (255, 255, 255, 255));
        // Centre de chaque nœud : sa couleur de remplissage.
        let viewport = Viewport::new(600.0, 400.0, frame.bounds, CameraState::default());
        for n in &frame.nodes {
            let (x, y) = viewport.graph_to_viewport(n.x, n.y);
            let (r, g, b, _) = at(x, y);
            assert_eq!(
                (r, g, b),
                (n.fill.r, n.fill.g, n.fill.b),
                "nœud en ({x}, {y})"
            );
        }

        // PNG écrit et relisible.
        let dir = tempfile::tempdir().expect("dossier temporaire");
        let path = dir.path().join("graphe.png");
        write_png(&request, &path).expect("écriture");
        let bytes = std::fs::read(&path).expect("lecture");
        assert_eq!(&bytes[1..4], b"PNG");
        let decoded = Pixmap::decode_png(&bytes).expect("décodage");
        assert_eq!((decoded.width(), decoded.height()), (600, 400));
    }

    #[test]
    fn facteur_d_echelle_de_l_ecran() {
        let request = ExportRequest {
            frame: GraphFrame::default(),
            width: 100.0,
            height: 50.0,
            camera: CameraState::default(),
            pixel_ratio: 1.5,
        };
        let pixmap = render(&request).expect("rendu");
        assert_eq!((pixmap.width(), pixmap.height()), (300, 150));
    }
}
