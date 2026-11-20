//! Caméra du graphe : bornes du ratio, seuils de zoom des labels d'arêtes,
//! normalisation de l'emprise du graphe et mise à l'échelle des tailles
//! (tailles d'écran, divisées par la racine carrée du ratio de caméra).

use super::style::{EDGE_LABEL_ZOOM, PORT_LABEL_ZOOM};

/// Ratio de caméra minimal (zoom 5).
pub const MIN_CAMERA_RATIO: f32 = 0.2;
/// Ratio de caméra maximal (zoom 0.1).
pub const MAX_CAMERA_RATIO: f32 = 10.0;

/// Ratio borné à `[MIN_CAMERA_RATIO, MAX_CAMERA_RATIO]` ; un ratio non fini
/// revient à 1.
pub fn clamp_camera_ratio(ratio: f32) -> f32 {
    if ratio.is_finite() {
        ratio.clamp(MIN_CAMERA_RATIO, MAX_CAMERA_RATIO)
    } else {
        1.0
    }
}

/// Zoom affiché pour un ratio de caméra : `zoom = 1 / ratio`.
pub fn zoom_for_ratio(ratio: f32) -> f32 {
    1.0 / ratio
}

/// Labels d'arêtes visibles à un niveau de zoom.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EdgeLabelVisibility {
    /// Protocole affiché (zoom ≥ [`EDGE_LABEL_ZOOM`]).
    pub edge_labels: bool,
    /// Ports ajoutés (zoom ≥ [`PORT_LABEL_ZOOM`]).
    pub port_labels: bool,
}

impl EdgeLabelVisibility {
    /// Visibilité pour `zoom` (= 1 / ratio).
    pub fn for_zoom(zoom: f32) -> Self {
        Self {
            edge_labels: zoom >= EDGE_LABEL_ZOOM,
            port_labels: zoom >= PORT_LABEL_ZOOM,
        }
    }
}

/// Rayon (ou épaisseur) d'écran d'une taille de nœud ou d'arête au ratio de caméra
/// donné : `size / √ratio` — les nœuds grossissent moins vite que le zoom.
pub fn screen_size(size: f32, camera_ratio: f32) -> f32 {
    size / camera_ratio.sqrt()
}

/// Emprise des positions des nœuds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    /// Abscisse minimale.
    pub min_x: f32,
    /// Ordonnée minimale.
    pub min_y: f32,
    /// Abscisse maximale.
    pub max_x: f32,
    /// Ordonnée maximale.
    pub max_y: f32,
}

/// Normalisation : l'emprise du graphe ramenée au carré unité centré en
/// (0.5, 0.5), le plus grand côté valant 1. Elle est recalculée à chaque
/// rafraîchissement, si bien que le cadrage suit le
/// graphe pendant que ForceAtlas2 le déplie ; la caméra (ratio, centre)
/// s'applique ensuite dans cet espace normalisé.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Normalization {
    /// Plus grand côté de l'emprise (1 pour une emprise dégénérée).
    pub ratio: f32,
    /// Centre de l'emprise en x.
    pub dx: f32,
    /// Centre de l'emprise en y.
    pub dy: f32,
}

impl Normalization {
    /// Normalisation d'une emprise ; un
    /// graphe vide ou réduit à un point donne un ratio de 1.
    pub fn from_bounds(bounds: Option<Bounds>) -> Self {
        let Some(b) = bounds else {
            return Self {
                ratio: 1.0,
                dx: 0.0,
                dy: 0.0,
            };
        };
        let mut ratio = (b.max_x - b.min_x).max(b.max_y - b.min_y);
        if ratio == 0.0 || !ratio.is_finite() {
            ratio = 1.0;
        }
        let finite_or_zero = |v: f32| if v.is_nan() { 0.0 } else { v };
        Self {
            ratio,
            dx: finite_or_zero((b.max_x + b.min_x) / 2.0),
            dy: finite_or_zero((b.max_y + b.min_y) / 2.0),
        }
    }

    /// Point monde → espace normalisé.
    pub fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (
            0.5 + (x - self.dx) / self.ratio,
            0.5 + (y - self.dy) / self.ratio,
        )
    }

    /// Espace normalisé → point monde.
    pub fn invert(&self, x: f32, y: f32) -> (f32, f32) {
        (
            self.dx + self.ratio * (x - 0.5),
            self.dy + self.ratio * (y - 0.5),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratio_borne_entre_0_2_et_10() {
        assert_eq!(clamp_camera_ratio(0.01), 0.2);
        assert_eq!(clamp_camera_ratio(50.0), 10.0);
        assert_eq!(clamp_camera_ratio(1.5), 1.5);
        assert_eq!(clamp_camera_ratio(f32::NAN), 1.0);
        assert_eq!(zoom_for_ratio(MIN_CAMERA_RATIO), 5.0);
        assert_eq!(zoom_for_ratio(MAX_CAMERA_RATIO), 0.1);
    }

    #[test]
    fn seuils_des_labels_d_aretes() {
        assert_eq!(
            EdgeLabelVisibility::for_zoom(1.0),
            EdgeLabelVisibility::default()
        );
        assert_eq!(
            EdgeLabelVisibility::for_zoom(1.2),
            EdgeLabelVisibility {
                edge_labels: true,
                port_labels: false
            }
        );
        assert_eq!(
            EdgeLabelVisibility::for_zoom(1.8),
            EdgeLabelVisibility {
                edge_labels: true,
                port_labels: true
            }
        );
    }

    #[test]
    fn tailles_d_ecran_en_racine_du_ratio() {
        assert_eq!(screen_size(10.0, 1.0), 10.0);
        assert_eq!(screen_size(10.0, 4.0), 5.0);
        assert_eq!(screen_size(10.0, 0.25), 20.0);
    }

    #[test]
    fn normalisation_dans_le_carre_unite() {
        let n = Normalization::from_bounds(Some(Bounds {
            min_x: -100.0,
            min_y: 0.0,
            max_x: 100.0,
            max_y: 50.0,
        }));
        assert_eq!(
            n,
            Normalization {
                ratio: 200.0,
                dx: 0.0,
                dy: 25.0
            }
        );
        assert_eq!(n.apply(-100.0, 25.0), (0.0, 0.5));
        assert_eq!(n.apply(100.0, 25.0), (1.0, 0.5));
        assert_eq!(n.invert(0.0, 0.5), (-100.0, 25.0));

        let point = Normalization::from_bounds(Some(Bounds {
            min_x: 3.0,
            min_y: 4.0,
            max_x: 3.0,
            max_y: 4.0,
        }));
        assert_eq!(
            point,
            Normalization {
                ratio: 1.0,
                dx: 3.0,
                dy: 4.0
            }
        );
        assert_eq!(Normalization::from_bounds(None).ratio, 1.0);
    }
}
