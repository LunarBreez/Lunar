//! Caméra et cadrage du graphe, sur le modèle de Sigma 3.0.3.
//!
//! Sigma passe par trois repères : les positions « monde » du graphe, le
//! carré unité où `autoRescale` ramène l'emprise des nœuds (« graphe cadré »,
//! [`Normalization`]), puis les pixels de la zone de dessin, à travers la
//! caméra `(x, y, ratio)` et la matrice de `matrixFromCamera` (marge de
//! scène de 30 px, rapport de correction de `getCorrectionRatio`). L'axe y
//! du monde monte, celui de l'écran descend.
//!
//! Les animations reprennent celles du `MouseCaptor` et de la `Camera` de
//! Sigma : molette (×1,7 en 250 ms, `quadraticOut`), double clic (×2,2 en
//! 200 ms), inertie au relâcher d'un glissé (×3 du dernier déplacement en
//! 200 ms), boutons de zoom (×1,5 en 150 ms, `quadraticInOut`) et recadrage
//! (`animatedReset`). Rien ici ne dépend de gpui.

use std::time::{Duration, Instant};

use lunar_ui_logic::graph::camera::{Bounds, Normalization, clamp_camera_ratio};

/// Marge de scène autour du graphe cadré (`stagePadding`).
pub const STAGE_PADDING: f32 = 30.0;
/// Facteur de zoom d'un cran de molette (`zoomingRatio`).
pub const WHEEL_ZOOMING_RATIO: f32 = 1.7;
/// Durée de l'animation d'un cran de molette (`zoomDuration`).
pub const WHEEL_ZOOM_DURATION: Duration = Duration::from_millis(250);
/// Deux crans de même sens plus rapprochés que `zoomDuration / 5` n'en
/// font qu'un (filtre du `MouseCaptor`).
pub const WHEEL_DEBOUNCE: Duration = Duration::from_millis(50);
/// Facteur de zoom du double clic (`doubleClickZoomingRatio`).
pub const DOUBLE_CLICK_ZOOMING_RATIO: f32 = 2.2;
/// Durée du zoom au double clic (`doubleClickZoomingDuration`).
pub const DOUBLE_CLICK_ZOOM_DURATION: Duration = Duration::from_millis(200);
/// Facteur de `camera.animatedZoom` / `animatedUnzoom`
/// (`DEFAULT_ZOOMING_RATIO`), utilisé par les boutons de zoom.
pub const BUTTON_ZOOMING_RATIO: f32 = 1.5;
/// Durée par défaut de `camera.animate` (`ANIMATE_DEFAULTS`).
pub const ANIMATE_DURATION: Duration = Duration::from_millis(150);
/// Inertie au relâcher d'un glissé : dernier déplacement × 3 (`inertiaRatio`).
pub const INERTIA_RATIO: f32 = 3.0;
/// Durée de l'inertie (`inertiaDuration`).
pub const INERTIA_DURATION: Duration = Duration::from_millis(200);
/// Au-delà de ce délai sans mouvement, le relâcher n'a plus d'inertie
/// (`dragTimeout`).
pub const DRAG_TIMEOUT: Duration = Duration::from_millis(100);
/// En dessous de ce nombre de mouvements souris, un appui relâché est un
/// clic (`draggedEventsTolerance`).
pub const DRAGGED_EVENTS_TOLERANCE: u32 = 3;
/// Épaisseur minimale d'une arête à l'écran (`minEdgeThickness`).
pub const MIN_EDGE_THICKNESS: f32 = 1.7;

/// État de la caméra (angle toujours nul : la vue ne tourne pas).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraState {
    /// Centre de la vue en x, dans le graphe cadré.
    pub x: f32,
    /// Centre de la vue en y, dans le graphe cadré.
    pub y: f32,
    /// Ratio : 1 cadre tout le graphe, plus petit agrandit (zoom = 1 / ratio).
    pub ratio: f32,
}

impl Default for CameraState {
    /// État de `animatedReset` : graphe entier centré.
    fn default() -> Self {
        Self {
            x: 0.5,
            y: 0.5,
            ratio: 1.0,
        }
    }
}

impl CameraState {
    /// Même état, ratio borné à `[0,2 ; 10]` (`getBoundedRatio`).
    pub fn bounded(self) -> Self {
        Self {
            ratio: clamp_camera_ratio(self.ratio),
            ..self
        }
    }
}

/// Courbe d'une animation de caméra (`easings` de Sigma).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Easing {
    /// Décélération (`quadraticOut`) : molette, inertie.
    QuadraticOut,
    /// Accélération puis décélération (`quadraticInOut`) : boutons, double
    /// clic, recadrage.
    QuadraticInOut,
}

impl Easing {
    /// Avancement de la courbe pour un temps normalisé `t` ∈ [0, 1].
    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Easing::QuadraticOut => t * (2.0 - t),
            Easing::QuadraticInOut => {
                let t2 = t * 2.0;
                if t2 < 1.0 {
                    0.5 * t2 * t2
                } else {
                    let u = t2 - 1.0;
                    -0.5 * (u * (u - 2.0) - 1.0)
                }
            }
        }
    }
}

/// Animation de caméra en cours.
#[derive(Clone, Copy, Debug)]
struct Animation {
    /// État de départ.
    from: CameraState,
    /// État d'arrivée.
    to: CameraState,
    /// Début.
    start: Instant,
    /// Durée.
    duration: Duration,
    /// Courbe.
    easing: Easing,
}

/// Caméra : état courant et animation éventuelle (`Camera` de Sigma).
#[derive(Clone, Debug, Default)]
pub struct Camera {
    /// État courant.
    state: CameraState,
    /// Animation en cours.
    animation: Option<Animation>,
}

impl Camera {
    /// État courant.
    pub fn state(&self) -> CameraState {
        self.state
    }

    /// Pose un état, ratio borné ; interrompt l'animation en cours.
    pub fn set_state(&mut self, state: CameraState) {
        self.animation = None;
        self.state = state.bounded();
    }

    /// Anime la caméra vers `to` (ratio borné) à partir de l'état courant.
    pub fn animate(&mut self, to: CameraState, duration: Duration, easing: Easing, now: Instant) {
        self.animation = Some(Animation {
            from: self.state,
            to: to.bounded(),
            start: now,
            duration,
            easing,
        });
    }

    /// Une animation est en cours.
    pub fn is_animating(&self) -> bool {
        self.animation.is_some()
    }

    /// Fait avancer l'animation jusqu'à `now` ; retourne vrai si l'état a
    /// changé.
    pub fn tick(&mut self, now: Instant) -> bool {
        let Some(anim) = self.animation else {
            return false;
        };
        let elapsed = now.saturating_duration_since(anim.start);
        let t = if anim.duration.is_zero() {
            1.0
        } else {
            elapsed.as_secs_f32() / anim.duration.as_secs_f32()
        };
        let before = self.state;
        if t >= 1.0 {
            self.state = anim.to;
            self.animation = None;
        } else {
            let k = anim.easing.apply(t);
            let lerp = |a: f32, b: f32| a + (b - a) * k;
            self.state = CameraState {
                x: lerp(anim.from.x, anim.to.x),
                y: lerp(anim.from.y, anim.to.y),
                ratio: lerp(anim.from.ratio, anim.to.ratio),
            };
        }
        self.state != before
    }
}

/// `getCorrectionRatio` : compense l'écart de proportions entre la zone de
/// dessin et l'emprise du graphe, pour que le graphe touche les bords.
fn correction_ratio(width: f32, height: f32, graph_width: f32, graph_height: f32) -> f32 {
    let viewport_ratio = height / width;
    let graph_ratio = graph_height / graph_width;
    if (viewport_ratio < 1.0 && graph_ratio > 1.0) || (viewport_ratio > 1.0 && graph_ratio < 1.0) {
        return 1.0;
    }
    graph_ratio
        .max(1.0 / graph_ratio)
        .min((1.0 / viewport_ratio).max(viewport_ratio))
}

/// Projection monde → pixels pour une zone de dessin, une emprise et un
/// état de caméra (`framedGraphToViewport` après `normalizationFunction`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    /// Largeur de la zone de dessin, en pixels logiques.
    pub width: f32,
    /// Hauteur de la zone de dessin, en pixels logiques.
    pub height: f32,
    /// Caméra appliquée.
    pub camera: CameraState,
    /// Normalisation de l'emprise (autoRescale).
    norm: Normalization,
    /// Pixels par unité du graphe cadré à ratio 1 (`(min(l, h) − 2·marge) ×
    /// correction`).
    unit: f32,
    /// Facteur des tailles d'écran : `1 / √ratio`.
    size_factor: f32,
    /// Emprise retenue, pour recalculer une autre caméra.
    bounds: Option<Bounds>,
}

impl Viewport {
    /// Projection d'une zone `width × height` sur l'emprise `bounds` des
    /// nœuds, vue par `camera`.
    pub fn new(width: f32, height: f32, bounds: Option<Bounds>, camera: CameraState) -> Self {
        let width = width.max(1.0);
        let height = height.max(1.0);
        let (graph_width, graph_height) = bounds.map_or((1.0, 1.0), |b| {
            let w = b.max_x - b.min_x;
            let h = b.max_y - b.min_y;
            (
                if w > 0.0 && w.is_finite() { w } else { 1.0 },
                if h > 0.0 && h.is_finite() { h } else { 1.0 },
            )
        });
        let smallest = (width.min(height) - 2.0 * STAGE_PADDING).max(1.0);
        let unit = smallest * correction_ratio(width, height, graph_width, graph_height);
        let ratio = camera.ratio.max(f32::MIN_POSITIVE);
        Self {
            width,
            height,
            camera,
            norm: Normalization::from_bounds(bounds),
            unit,
            size_factor: 1.0 / ratio.sqrt(),
            bounds,
        }
    }

    /// Même zone et même emprise vues par une autre caméra.
    pub fn with_camera(&self, camera: CameraState) -> Self {
        Self::new(self.width, self.height, self.bounds, camera)
    }

    /// Pixels par unité du graphe cadré, au ratio courant.
    fn scale(&self) -> f32 {
        self.unit / self.camera.ratio
    }

    /// Point du graphe cadré → pixels.
    pub fn framed_to_viewport(&self, x: f32, y: f32) -> (f32, f32) {
        let s = self.scale();
        (
            self.width / 2.0 + (x - self.camera.x) * s,
            self.height / 2.0 - (y - self.camera.y) * s,
        )
    }

    /// Pixels → point du graphe cadré (`viewportToFramedGraph`).
    pub fn viewport_to_framed(&self, x: f32, y: f32) -> (f32, f32) {
        let s = self.scale();
        (
            self.camera.x + (x - self.width / 2.0) / s,
            self.camera.y - (y - self.height / 2.0) / s,
        )
    }

    /// Point monde → pixels.
    pub fn graph_to_viewport(&self, x: f32, y: f32) -> (f32, f32) {
        let (nx, ny) = self.norm.apply(x, y);
        self.framed_to_viewport(nx, ny)
    }

    /// Pixels → point monde.
    #[cfg(test)]
    pub fn viewport_to_graph(&self, x: f32, y: f32) -> (f32, f32) {
        let (nx, ny) = self.viewport_to_framed(x, y);
        self.norm.invert(nx, ny)
    }

    /// Taille Sigma (rayon de nœud, épaisseur d'arête) → pixels :
    /// `size / √ratio` (`scaleSize`, tailles de référence « screen »).
    pub fn scale_size(&self, size: f32) -> f32 {
        size * self.size_factor
    }

    /// État de caméra qui zoome au ratio `new_ratio` en gardant fixe le point
    /// d'écran `(x, y)` (`getViewportZoomedState`).
    pub fn zoomed_state(&self, x: f32, y: f32, new_ratio: f32) -> CameraState {
        let new_ratio = clamp_camera_ratio(new_ratio);
        let ratio_diff = new_ratio / self.camera.ratio;
        let (mx, my) = self.viewport_to_framed(x, y);
        let (cx, cy) = self.viewport_to_framed(self.width / 2.0, self.height / 2.0);
        CameraState {
            x: (mx - cx) * (1.0 - ratio_diff) + self.camera.x,
            y: (my - cy) * (1.0 - ratio_diff) + self.camera.y,
            ratio: new_ratio,
        }
    }

    /// État de caméra après un glissé de `from` à `to` (pixels) : le point
    /// saisi suit le pointeur (`handleMove` du `MouseCaptor`).
    pub fn panned_state(&self, from: (f32, f32), to: (f32, f32)) -> CameraState {
        let (lx, ly) = self.viewport_to_framed(from.0, from.1);
        let (mx, my) = self.viewport_to_framed(to.0, to.1);
        CameraState {
            x: self.camera.x + (lx - mx),
            y: self.camera.y + (ly - my),
            ratio: self.camera.ratio,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds(min_x: f32, min_y: f32, max_x: f32, max_y: f32) -> Option<Bounds> {
        Some(Bounds {
            min_x,
            min_y,
            max_x,
            max_y,
        })
    }

    fn close(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3
    }

    #[test]
    fn cadrage_comme_sigma() {
        // 800 × 600, graphe carré : le plus petit côté moins les marges
        // (540 px) couvre l'emprise ; y monte dans le monde.
        let vp = Viewport::new(
            800.0,
            600.0,
            bounds(-100.0, -100.0, 100.0, 100.0),
            CameraState::default(),
        );
        assert!(close(vp.graph_to_viewport(0.0, 0.0), (400.0, 300.0)));
        assert!(close(vp.graph_to_viewport(-100.0, 0.0), (130.0, 300.0)));
        assert!(close(vp.graph_to_viewport(100.0, 100.0), (670.0, 30.0)));
        assert!(close(vp.viewport_to_graph(670.0, 30.0), (100.0, 100.0)));
    }

    #[test]
    fn rapport_de_correction() {
        // Zone large, graphe large : les grands côtés se touchent.
        assert_eq!(correction_ratio(800.0, 400.0, 400.0, 100.0), 2.0);
        // Orientations opposées : pas de correction.
        assert_eq!(correction_ratio(800.0, 400.0, 100.0, 400.0), 1.0);
    }

    #[test]
    fn tailles_d_ecran() {
        let vp = Viewport::new(
            100.0,
            100.0,
            None,
            CameraState {
                ratio: 4.0,
                ..CameraState::default()
            },
        );
        assert_eq!(vp.scale_size(10.0), 5.0);
    }

    #[test]
    fn zoom_autour_du_pointeur() {
        let vp = Viewport::new(
            800.0,
            600.0,
            bounds(0.0, 0.0, 10.0, 10.0),
            CameraState::default(),
        );
        let anchor = (600.0, 150.0);
        let world = vp.viewport_to_graph(anchor.0, anchor.1);
        let zoomed = vp.with_camera(vp.zoomed_state(anchor.0, anchor.1, 0.5));
        assert!(close(zoomed.graph_to_viewport(world.0, world.1), anchor));
        // Ratio borné.
        assert_eq!(vp.zoomed_state(0.0, 0.0, 0.01).ratio, 0.2);
    }

    #[test]
    fn glisse_le_point_saisi_suit_le_pointeur() {
        let vp = Viewport::new(
            800.0,
            600.0,
            bounds(0.0, 0.0, 10.0, 10.0),
            CameraState::default(),
        );
        let world = vp.viewport_to_graph(100.0, 100.0);
        let panned = vp.with_camera(vp.panned_state((100.0, 100.0), (180.0, 60.0)));
        assert!(close(
            panned.graph_to_viewport(world.0, world.1),
            (180.0, 60.0)
        ));
    }

    #[test]
    fn animation_et_courbes() {
        assert_eq!(Easing::QuadraticOut.apply(0.5), 0.75);
        assert_eq!(Easing::QuadraticInOut.apply(0.5), 0.5);
        assert_eq!(Easing::QuadraticInOut.apply(1.0), 1.0);

        let mut camera = Camera::default();
        let t0 = Instant::now();
        let to = CameraState {
            ratio: 0.5,
            ..CameraState::default()
        };
        camera.animate(to, Duration::from_millis(100), Easing::QuadraticOut, t0);
        assert!(camera.is_animating());
        assert!(camera.tick(t0 + Duration::from_millis(50)));
        assert!((camera.state().ratio - 0.625).abs() < 1e-4);
        assert!(camera.tick(t0 + Duration::from_millis(150)));
        assert_eq!(camera.state(), to);
        assert!(!camera.is_animating());
        assert!(!camera.tick(t0 + Duration::from_millis(200)));
    }
}
