//! ForceAtlas2 (Jacomy et al., 2014), d'après
//! `graphology-layout-forceatlas2` 0.10.1 (`iterate.js`, `helpers.js`,
//! `defaults.js`, `inferSettings` d'`index.js`), Barnes-Hut compris.
//!
//! L'algorithme est suivi pas à pas, y compris ses particularités : les
//! sous-régions du quadtree sont centrées à ± la *demi-taille* de leur parent,
//! la masse du nœud qui déclenche une subdivision n'est pas ajoutée à la région
//! subdivisée, et un nœud qui retombe trois fois de suite dans le même
//! quadrant qu'un autre n'est pas inséré. Les positions et calculs sont en
//! `f32` (graphology stocke en `Float32Array` mais calcule en double) : les
//! trajectoires diffèrent dans les derniers bits, pas dans la méthode.

/// Réglages de ForceAtlas2 (`defaults.js` pour les valeurs par défaut).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fa2Settings {
    /// Attraction logarithmique (mode LinLog).
    pub lin_log_mode: bool,
    /// Attraction divisée par la masse de la source (hubs écartés).
    pub outbound_attraction_distribution: bool,
    /// Anticollision fondée sur la taille des nœuds.
    pub adjust_sizes: bool,
    /// Exposant appliqué au poids des arêtes.
    pub edge_weight_influence: f32,
    /// Coefficient de répulsion (`scalingRatio`).
    pub scaling_ratio: f32,
    /// Gravité indépendante de la distance au centre.
    pub strong_gravity_mode: bool,
    /// Intensité de la gravité.
    pub gravity: f32,
    /// Diviseur des déplacements : plus grand, plus lent et plus stable.
    pub slow_down: f32,
    /// Répulsion approchée par Barnes-Hut (O(n log n)).
    pub barnes_hut_optimize: bool,
    /// Seuil θ de Barnes-Hut.
    pub barnes_hut_theta: f32,
}

impl Default for Fa2Settings {
    fn default() -> Self {
        Self {
            lin_log_mode: false,
            outbound_attraction_distribution: false,
            adjust_sizes: false,
            edge_weight_influence: 1.0,
            scaling_ratio: 1.0,
            strong_gravity_mode: false,
            gravity: 1.0,
            slow_down: 1.0,
            barnes_hut_optimize: false,
            barnes_hut_theta: 0.5,
        }
    }
}

/// Réglages « raisonnables » pour un graphe de `order` nœuds
/// (`forceAtlas2.inferSettings`), fusionnés avec les défauts comme le fait le
/// superviseur : Barnes-Hut au-delà de 2000 nœuds, gravité forte 0.05,
/// répulsion 10, ralentissement `1 + ln(order)`.
///
/// ⚠️ Sur un graphe vide, `slow_down` vaut `-∞` et fige la simulation : le
/// contrôleur ([`super::force_layout`]) ne démarre jamais sur un graphe vide.
pub fn infer_settings(order: usize) -> Fa2Settings {
    Fa2Settings {
        barnes_hut_optimize: order > 2000,
        strong_gravity_mode: true,
        gravity: 0.05,
        scaling_ratio: 10.0,
        slow_down: (1.0 + (order as f64).ln()) as f32,
        ..Fa2Settings::default()
    }
}

/// Plafond de force du mode anticollision (`MAX_FORCE`).
const MAX_FORCE: f32 = 10.0;

/// Tentatives de subdivision quand deux nœuds tombent dans le même quadrant.
const SUBDIVISION_ATTEMPTS: u32 = 3;

/// Absence de région ou de nœud (le `-1` des matrices JavaScript).
const NONE: u32 = u32::MAX;

/// Ligne de la `NodeMatrix` : un nœud et ses grandeurs de simulation.
#[derive(Clone, Copy, Debug)]
struct Body {
    /// Abscisse.
    x: f32,
    /// Ordonnée.
    y: f32,
    /// Force accumulée en x pendant l'itération.
    dx: f32,
    /// Force accumulée en y pendant l'itération.
    dy: f32,
    /// Force en x de l'itération précédente.
    old_dx: f32,
    /// Force en y de l'itération précédente.
    old_dy: f32,
    /// Masse : 1 + degré pondéré.
    mass: f32,
    /// Convergence locale (vitesse adaptative).
    convergence: f32,
    /// Taille du nœud (anticollision).
    size: f32,
    /// Nœud figé : sa position ne bouge pas.
    fixed: bool,
}

/// Ligne de l'`EdgeMatrix` : une arête pondérée.
#[derive(Clone, Copy, Debug)]
struct Spring {
    /// Rang du nœud source.
    source: u32,
    /// Rang du nœud cible.
    target: u32,
    /// Poids de l'arête.
    weight: f32,
}

/// Ligne de la `RegionMatrix` : une région du quadtree de Barnes-Hut.
#[derive(Clone, Copy, Debug)]
struct Region {
    /// Nœud de la feuille, ou [`NONE`].
    node: u32,
    /// Centre géométrique en x.
    center_x: f32,
    /// Centre géométrique en y.
    center_y: f32,
    /// Taille de la région.
    size: f32,
    /// Région sœur suivante dans le parcours, ou [`NONE`].
    next_sibling: u32,
    /// Première des quatre sous-régions, ou [`NONE`].
    first_child: u32,
    /// Masse cumulée.
    mass: f32,
    /// Centre de masse en x.
    mass_center_x: f32,
    /// Centre de masse en y.
    mass_center_y: f32,
}

impl Region {
    /// Région vide de centre et taille donnés.
    fn empty(center_x: f32, center_y: f32, size: f32, next_sibling: u32) -> Self {
        Self {
            node: NONE,
            center_x,
            center_y,
            size,
            next_sibling,
            first_child: NONE,
            mass: 0.0,
            mass_center_x: 0.0,
            mass_center_y: 0.0,
        }
    }

    /// Sous-région de `self` (subdivisée) où tombe le point `(x, y)` :
    /// haut-gauche, bas-gauche, haut-droite, bas-droite (repère écran).
    fn quadrant(&self, x: f32, y: f32) -> u32 {
        let offset = match (x < self.center_x, y < self.center_y) {
            (true, true) => 0,
            (true, false) => 1,
            (false, true) => 2,
            (false, false) => 3,
        };
        self.first_child + offset
    }
}

/// Nœud chargé dans la simulation (`graphToByteArrays`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fa2Node {
    /// Abscisse initiale.
    pub x: f32,
    /// Ordonnée initiale.
    pub y: f32,
    /// Taille (`attr.size || 1`).
    pub size: f32,
    /// Nœud figé.
    pub fixed: bool,
}

/// État de la simulation : matrices de nœuds, d'arêtes et de régions,
/// réutilisées d'une itération à l'autre (aucune allocation en régime
/// établi).
#[derive(Clone, Debug)]
pub struct ForceAtlas2 {
    /// Réglages de la simulation.
    settings: Fa2Settings,
    /// Nœuds, dans l'ordre de chargement.
    bodies: Vec<Body>,
    /// Arêtes.
    springs: Vec<Spring>,
    /// Quadtree de Barnes-Hut, reconstruit à chaque itération.
    regions: Vec<Region>,
}

impl ForceAtlas2 {
    /// Simulation vide avec ces réglages.
    pub fn new(settings: Fa2Settings) -> Self {
        Self {
            settings,
            bodies: Vec::new(),
            springs: Vec::new(),
            regions: Vec::new(),
        }
    }

    /// Réglages courants.
    pub fn settings(&self) -> &Fa2Settings {
        &self.settings
    }

    /// (Re)charge les matrices (`graphToByteArrays`) : forces et convergence
    /// repartent de zéro, la masse d'un nœud vaut 1 plus son degré pondéré.
    /// Les arêtes `(source, cible, poids)` désignent les nœuds par leur rang ;
    /// une arête hors bornes est ignorée.
    pub fn load(
        &mut self,
        nodes: impl IntoIterator<Item = Fa2Node>,
        edges: impl IntoIterator<Item = (usize, usize, f32)>,
    ) {
        self.bodies.clear();
        self.bodies.extend(nodes.into_iter().map(|n| Body {
            x: n.x,
            y: n.y,
            dx: 0.0,
            dy: 0.0,
            old_dx: 0.0,
            old_dy: 0.0,
            mass: 1.0,
            convergence: 1.0,
            size: if n.size == 0.0 || n.size.is_nan() {
                1.0
            } else {
                n.size
            },
            fixed: n.fixed,
        }));
        self.springs.clear();
        let order = self.bodies.len();
        for (source, target, weight) in edges {
            if source >= order || target >= order {
                continue;
            }
            self.bodies[source].mass += weight;
            self.bodies[target].mass += weight;
            self.springs.push(Spring {
                source: source as u32,
                target: target as u32,
                weight,
            });
        }
    }

    /// Nombre de nœuds chargés.
    pub fn order(&self) -> usize {
        self.bodies.len()
    }

    /// Position du nœud de rang `index`.
    pub fn position(&self, index: usize) -> Option<(f32, f32)> {
        self.bodies.get(index).map(|b| (b.x, b.y))
    }

    /// Positions de tous les nœuds, dans l'ordre de chargement.
    pub fn positions(&self) -> impl ExactSizeIterator<Item = (f32, f32)> + '_ {
        self.bodies.iter().map(|b| (b.x, b.y))
    }

    /// Une itération de l'algorithme (`iterate`).
    pub fn iterate(&mut self) {
        let s = self.settings;

        // 1) Initialisation : forces de l'itération précédente mises de côté.
        for b in &mut self.bodies {
            b.old_dx = b.dx;
            b.old_dy = b.dy;
            b.dx = 0.0;
            b.dy = 0.0;
        }

        // Compensation de l'attraction distribuée : masse moyenne.
        let outbound_compensation = if s.outbound_attraction_distribution {
            let total: f32 = self.bodies.iter().map(|b| b.mass).sum();
            total / self.bodies.len() as f32
        } else {
            0.0
        };

        // 1 bis) Quadtree de Barnes-Hut, puis 2) répulsion.
        if s.barnes_hut_optimize {
            self.build_region_tree();
            self.apply_barnes_hut_repulsion();
        } else {
            self.apply_square_repulsion();
        }

        self.apply_gravity();
        self.apply_attraction(outbound_compensation);
        self.apply_forces();
    }

    /// Construit le quadtree de Barnes-Hut sur les positions courantes.
    fn build_region_tree(&mut self) {
        let bodies = &self.bodies;
        let regions = &mut self.regions;
        regions.clear();

        let (mut min_x, mut max_x) = (f32::INFINITY, f32::NEG_INFINITY);
        let (mut min_y, mut max_y) = (f32::INFINITY, f32::NEG_INFINITY);
        for b in bodies {
            min_x = min_x.min(b.x);
            max_x = max_x.max(b.x);
            min_y = min_y.min(b.y);
            max_y = max_y.max(b.y);
        }
        // Emprise rendue carrée : c'est un quadtree.
        let (dx, dy) = (max_x - min_x, max_y - min_y);
        if dx > dy {
            min_y -= (dx - dy) / 2.0;
            max_y = min_y + dx;
        } else {
            min_x -= (dy - dx) / 2.0;
            max_x = min_x + dy;
        }

        regions.push(Region::empty(
            (min_x + max_x) / 2.0,
            (min_y + max_y) / 2.0,
            (max_x - min_x).max(max_y - min_y),
            NONE,
        ));

        for (n, body) in bodies.iter().enumerate() {
            let n = n as u32;
            let mut r = 0usize;
            let mut attempts = SUBDIVISION_ATTEMPTS;
            loop {
                let region = regions[r];
                if region.first_child != NONE {
                    // Sous-régions : on descend vers une feuille, en mettant à
                    // jour masse et centre de masse des régions traversées.
                    let q = region.quadrant(body.x, body.y);
                    let total = region.mass + body.mass;
                    let reg = &mut regions[r];
                    reg.mass_center_x =
                        (region.mass_center_x * region.mass + body.x * body.mass) / total;
                    reg.mass_center_y =
                        (region.mass_center_y * region.mass + body.y * body.mass) / total;
                    reg.mass += body.mass;
                    r = q as usize;
                    continue;
                }

                // Feuille vide : le nœud s'y range.
                if region.node == NONE {
                    regions[r].node = n;
                    break;
                }

                // Feuille occupée : subdivision en quatre sous-régions
                // (centrées à ± la demi-taille, comme graphology).
                let first = regions.len() as u32;
                let w = region.size / 2.0;
                let (cx, cy) = (region.center_x, region.center_y);
                regions.push(Region::empty(cx - w, cy - w, w, first + 1));
                regions.push(Region::empty(cx - w, cy + w, w, first + 2));
                regions.push(Region::empty(cx + w, cy - w, w, first + 3));
                regions.push(Region::empty(cx + w, cy + w, w, region.next_sibling));

                let old = region.node as usize;
                let old_body = bodies[old];
                let reg = &mut regions[r];
                reg.first_child = first;
                let q = reg.quadrant(old_body.x, old_body.y);
                // L'ancien nœud descend dans son quadrant ; la région prend sa
                // masse (et seulement la sienne).
                reg.mass = old_body.mass;
                reg.mass_center_x = old_body.x;
                reg.mass_center_y = old_body.y;
                reg.node = NONE;
                let q2 = reg.quadrant(body.x, body.y);
                regions[q as usize].node = old as u32;

                if q == q2 {
                    // Même quadrant : on recommence un cran plus bas, au plus
                    // SUBDIVISION_ATTEMPTS fois, puis on abandonne ce nœud.
                    if attempts > 0 {
                        attempts -= 1;
                        r = q as usize;
                        continue;
                    }
                    break;
                }
                regions[q2 as usize].node = n;
                break;
            }
        }
    }

    /// Répulsion approchée par le quadtree.
    fn apply_barnes_hut_repulsion(&mut self) {
        let coefficient = self.settings.scaling_ratio;
        let theta_squared = self.settings.barnes_hut_theta * self.settings.barnes_hut_theta;
        let adjust_sizes = self.settings.adjust_sizes;
        let regions = &self.regions;
        let bodies = &mut self.bodies;

        for n in 0..bodies.len() {
            let body = bodies[n];
            let (mut fx, mut fy) = (body.dx, body.dy);
            let mut r = 0u32;
            loop {
                let region = regions[r as usize];
                let other_mass;
                let (x_dist, y_dist, distance);
                if region.first_child != NONE {
                    // Test de Barnes-Hut : la région est-elle assez loin pour
                    // être traitée comme un seul corps ?
                    let d = (body.x - region.mass_center_x).powi(2)
                        + (body.y - region.mass_center_y).powi(2);
                    let size = region.size;
                    // Écrit comme en JavaScript : un rapport NaN (région vide
                    // de taille nulle, à distance nulle) fait descendre.
                    let far_enough = (4.0 * size * size) / d < theta_squared;
                    if !far_enough {
                        // Trop proche : on regarde les sous-régions.
                        r = region.first_child;
                        continue;
                    }
                    x_dist = body.x - region.mass_center_x;
                    y_dist = body.y - region.mass_center_y;
                    distance = d;
                    other_mass = region.mass;
                } else {
                    // Feuille : répulsion avec son nœud, s'il y en a un autre.
                    let rn = region.node;
                    if rn == NONE || rn as usize == n {
                        r = region.next_sibling;
                        if r == NONE {
                            break;
                        }
                        continue;
                    }
                    let other = bodies[rn as usize];
                    x_dist = body.x - other.x;
                    y_dist = body.y - other.y;
                    distance = x_dist * x_dist + y_dist * y_dist;
                    other_mass = other.mass;
                }

                if distance > 0.0 {
                    // Répulsion linéaire (anticollision comprise : graphology
                    // y applique la même formule en Barnes-Hut).
                    let factor = coefficient * body.mass * other_mass / distance;
                    fx += x_dist * factor;
                    fy += y_dist * factor;
                } else if adjust_sizes && distance < 0.0 {
                    let factor = -coefficient * body.mass * other_mass / distance.sqrt();
                    fx += x_dist * factor;
                    fy += y_dist * factor;
                }

                // Région traitée : on passe à la sœur suivante.
                r = region.next_sibling;
                if r == NONE {
                    break;
                }
            }
            bodies[n].dx = fx;
            bodies[n].dy = fy;
        }
    }

    /// Répulsion exacte, toutes paires (O(n²)).
    fn apply_square_repulsion(&mut self) {
        let coefficient = self.settings.scaling_ratio;
        let adjust_sizes = self.settings.adjust_sizes;
        let bodies = &mut self.bodies;
        for n1 in 0..bodies.len() {
            let (head, tail) = bodies.split_at_mut(n1);
            let b1 = &mut tail[0];
            for b2 in head.iter_mut() {
                let x_dist = b1.x - b2.x;
                let y_dist = b1.y - b2.y;
                let factor = if adjust_sizes {
                    // Répulsion linéaire anticollision.
                    let distance = (x_dist * x_dist + y_dist * y_dist).sqrt() - b1.size - b2.size;
                    if distance > 0.0 {
                        coefficient * b1.mass * b2.mass / distance / distance
                    } else if distance < 0.0 {
                        100.0 * coefficient * b1.mass * b2.mass
                    } else {
                        continue;
                    }
                } else {
                    // Répulsion linéaire.
                    let distance = (x_dist * x_dist + y_dist * y_dist).sqrt();
                    if distance > 0.0 {
                        coefficient * b1.mass * b2.mass / distance / distance
                    } else {
                        continue;
                    }
                };
                b1.dx += x_dist * factor;
                b1.dy += y_dist * factor;
                b2.dx -= x_dist * factor;
                b2.dy -= y_dist * factor;
            }
        }
    }

    /// Gravité vers l'origine, forte (indépendante de la distance) ou non.
    fn apply_gravity(&mut self) {
        let s = self.settings;
        let g = s.gravity / s.scaling_ratio;
        let coefficient = s.scaling_ratio;
        for b in &mut self.bodies {
            let (x_dist, y_dist) = (b.x, b.y);
            let distance = (x_dist.powi(2) + y_dist.powi(2)).sqrt();
            let factor = if distance > 0.0 {
                if s.strong_gravity_mode {
                    coefficient * b.mass * g
                } else {
                    coefficient * b.mass * g / distance
                }
            } else {
                0.0
            };
            b.dx -= x_dist * factor;
            b.dy -= y_dist * factor;
        }
    }

    /// Attraction le long des arêtes.
    fn apply_attraction(&mut self, outbound_compensation: f32) {
        let s = self.settings;
        let coefficient = if s.outbound_attraction_distribution {
            outbound_compensation
        } else {
            1.0
        };
        let bodies = &mut self.bodies;
        for spring in &self.springs {
            let (n1, n2) = (spring.source as usize, spring.target as usize);
            let ewc = spring.weight.powf(s.edge_weight_influence);
            let (b1, b2) = (bodies[n1], bodies[n2]);
            let x_dist = b1.x - b2.x;
            let y_dist = b1.y - b2.y;
            let raw = (x_dist.powi(2) + y_dist.powi(2)).sqrt();

            let (distance, factor) = if s.adjust_sizes {
                let distance = raw - b1.size - b2.size;
                let factor = match (s.lin_log_mode, s.outbound_attraction_distribution) {
                    // LinLog anticollision, distribuée.
                    (true, true) => -coefficient * ewc * (1.0 + distance).ln() / distance / b1.mass,
                    // LinLog anticollision.
                    (true, false) => -coefficient * ewc * (1.0 + distance).ln() / distance,
                    // Linéaire anticollision, distribuée.
                    (false, true) => -coefficient * ewc / b1.mass,
                    // Linéaire anticollision.
                    (false, false) => -coefficient * ewc,
                };
                (distance, factor)
            } else if s.lin_log_mode {
                let factor = if s.outbound_attraction_distribution {
                    // LinLog distribuée.
                    -coefficient * ewc * (1.0 + raw).ln() / raw / b1.mass
                } else {
                    // LinLog.
                    -coefficient * ewc * (1.0 + raw).ln() / raw
                };
                (raw, factor)
            } else if s.outbound_attraction_distribution {
                // Linéaire distribuée par la masse ; distance forcée à 1.
                (1.0, -coefficient * ewc / b1.mass)
            } else {
                // Linéaire ; distance forcée à 1.
                (1.0, -coefficient * ewc)
            };

            if distance > 0.0 {
                bodies[n1].dx += x_dist * factor;
                bodies[n1].dy += y_dist * factor;
                bodies[n2].dx -= x_dist * factor;
                bodies[n2].dy -= y_dist * factor;
            }
        }
    }

    /// Applique les forces : vitesse adaptative par nœud (oscillation contre
    /// traction), freinée par `slow_down`.
    fn apply_forces(&mut self) {
        let s = self.settings;
        for b in self.bodies.iter_mut().filter(|b| !b.fixed) {
            if s.adjust_sizes {
                let force = (b.dx.powi(2) + b.dy.powi(2)).sqrt();
                if force > MAX_FORCE {
                    b.dx = b.dx * MAX_FORCE / force;
                    b.dy = b.dy * MAX_FORCE / force;
                }
            }

            let swinging = b.mass
                * ((b.old_dx - b.dx) * (b.old_dx - b.dx) + (b.old_dy - b.dy) * (b.old_dy - b.dy))
                    .sqrt();
            let traction = ((b.old_dx + b.dx) * (b.old_dx + b.dx)
                + (b.old_dy + b.dy) * (b.old_dy + b.dy))
                .sqrt()
                / 2.0;

            let node_speed = if s.adjust_sizes {
                0.1 * (1.0 + traction).ln() / (1.0 + swinging.sqrt())
            } else {
                let speed = b.convergence * (1.0 + traction).ln() / (1.0 + swinging.sqrt());
                // `Math.min(1, …)` : NaN se propage comme en JavaScript.
                let convergence =
                    (speed * (b.dx.powi(2) + b.dy.powi(2)) / (1.0 + swinging.sqrt())).sqrt();
                b.convergence = if convergence > 1.0 { 1.0 } else { convergence };
                speed
            };

            b.x += b.dx * (node_speed / s.slow_down);
            b.y += b.dy * (node_speed / s.slow_down);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::rng::Rng;

    /// Graphe aléatoire de `n` nœuds et `m` arêtes, positions dans un carré.
    fn random_graph(n: usize, m: usize, seed: u64) -> (Vec<Fa2Node>, Vec<(usize, usize, f32)>) {
        let mut rng = Rng::with_seed(seed);
        let nodes = (0..n)
            .map(|_| Fa2Node {
                x: (rng.random_float() * 1000.0 - 500.0) as f32,
                y: (rng.random_float() * 1000.0 - 500.0) as f32,
                size: 7.0,
                fixed: false,
            })
            .collect();
        let edges = (0..m)
            .map(|_| {
                let a = (rng.next_u64() % n as u64) as usize;
                let b = (rng.next_u64() % n as u64) as usize;
                (a, b, 1.0)
            })
            .collect();
        (nodes, edges)
    }

    fn spread(layout: &ForceAtlas2) -> f32 {
        layout
            .positions()
            .map(|(x, y)| (x * x + y * y).sqrt())
            .fold(0.0, f32::max)
    }

    #[test]
    fn infer_settings_comme_graphology() {
        let s = infer_settings(100);
        assert!(!s.barnes_hut_optimize);
        assert!(s.strong_gravity_mode);
        assert_eq!(s.gravity, 0.05);
        assert_eq!(s.scaling_ratio, 10.0);
        assert_eq!(s.slow_down, (1.0 + 100f64.ln()) as f32);
        // Défauts conservés pour le reste.
        assert!(!s.lin_log_mode && !s.adjust_sizes && !s.outbound_attraction_distribution);
        assert_eq!(s.edge_weight_influence, 1.0);
        assert_eq!(s.barnes_hut_theta, 0.5);

        assert!(!infer_settings(2000).barnes_hut_optimize);
        assert!(infer_settings(2001).barnes_hut_optimize);
        // Le piège évité par le contrôleur : graphe vide → slowDown = -∞.
        assert_eq!(infer_settings(0).slow_down, f32::NEG_INFINITY);
    }

    #[test]
    fn la_masse_vaut_un_plus_le_degre() {
        let mut layout = ForceAtlas2::new(Fa2Settings::default());
        let node = Fa2Node {
            x: 0.0,
            y: 0.0,
            size: 0.0,
            fixed: false,
        };
        layout.load(
            [node; 3],
            [(0, 1, 1.0), (0, 2, 1.0), (2, 2, 1.0), (0, 9, 1.0)],
        );
        let masses: Vec<_> = layout.bodies.iter().map(|b| b.mass).collect();
        assert_eq!(masses, [3.0, 2.0, 4.0]);
        assert_eq!(layout.springs.len(), 3, "arête hors bornes ignorée");
        // `attr.size || 1`.
        assert!(layout.bodies.iter().all(|b| b.size == 1.0));
    }

    #[test]
    fn deux_noeuds_relies_trouvent_un_equilibre() {
        // Attraction linéaire (distance) contre répulsion (k / distance) :
        // l'équilibre est à distance √(k·m1·m2) quelle que soit la gravité
        // faible ; les deux nœuds restent symétriques par rapport à l'origine.
        let mut layout = ForceAtlas2::new(infer_settings(2));
        layout.load(
            [
                Fa2Node {
                    x: -300.0,
                    y: 0.0,
                    size: 1.0,
                    fixed: false,
                },
                Fa2Node {
                    x: 300.0,
                    y: 0.0,
                    size: 1.0,
                    fixed: false,
                },
            ],
            [(0, 1, 1.0)],
        );
        for _ in 0..2000 {
            layout.iterate();
        }
        let (a, b) = (
            layout.position(0).expect("a"),
            layout.position(1).expect("b"),
        );
        assert!((a.0 + b.0).abs() < 1e-2 && a.1.abs() < 1e-3, "{a:?} {b:?}");
        let distance = b.0 - a.0;
        assert!(distance > 1.0 && distance < 600.0, "{distance}");
    }

    #[test]
    fn un_noeud_fige_ne_bouge_pas() {
        let mut layout = ForceAtlas2::new(infer_settings(2));
        layout.load(
            [
                Fa2Node {
                    x: 0.0,
                    y: 0.0,
                    size: 1.0,
                    fixed: true,
                },
                Fa2Node {
                    x: 50.0,
                    y: 0.0,
                    size: 1.0,
                    fixed: false,
                },
            ],
            [(0, 1, 1.0)],
        );
        layout.iterate();
        assert_eq!(layout.position(0), Some((0.0, 0.0)));
        assert_ne!(layout.position(1), Some((50.0, 0.0)));
    }

    #[test]
    fn barnes_hut_approche_la_repulsion_exacte() {
        let (nodes, edges) = random_graph(300, 600, 11);
        let mut exact = ForceAtlas2::new(Fa2Settings {
            barnes_hut_optimize: false,
            ..infer_settings(300)
        });
        let mut approx = ForceAtlas2::new(Fa2Settings {
            barnes_hut_optimize: true,
            ..infer_settings(300)
        });
        exact.load(nodes.iter().copied(), edges.iter().copied());
        approx.load(nodes.iter().copied(), edges.iter().copied());
        exact.iterate();
        approx.iterate();
        // Même direction de déplacement pour l'immense majorité des nœuds.
        let mut agree = 0;
        for (i, n) in nodes.iter().enumerate() {
            let (ex, ey) = exact.position(i).expect("exact");
            let (ax, ay) = approx.position(i).expect("approx");
            let dot = (ex - n.x) * (ax - n.x) + (ey - n.y) * (ay - n.y);
            if dot > 0.0 {
                agree += 1;
            }
        }
        assert!(agree > nodes.len() * 9 / 10, "{agree}/{}", nodes.len());
    }

    #[test]
    fn la_disposition_reste_finie_et_bornee() {
        for barnes_hut in [false, true] {
            let (nodes, edges) = random_graph(500, 1000, 5);
            let mut layout = ForceAtlas2::new(Fa2Settings {
                barnes_hut_optimize: barnes_hut,
                ..infer_settings(500)
            });
            layout.load(nodes, edges);
            for _ in 0..200 {
                layout.iterate();
            }
            assert!(
                layout
                    .positions()
                    .all(|(x, y)| x.is_finite() && y.is_finite())
            );
            assert!(spread(&layout) < 1e5, "{}", spread(&layout));
        }
    }

    #[test]
    fn noeuds_superposes_sans_nan() {
        // Positions identiques : la répulsion est nulle à distance nulle, le
        // quadtree abandonne après ses trois subdivisions ; rien ne diverge.
        for barnes_hut in [false, true] {
            let mut layout = ForceAtlas2::new(Fa2Settings {
                barnes_hut_optimize: barnes_hut,
                ..infer_settings(3)
            });
            layout.load(
                [Fa2Node {
                    x: 1.0,
                    y: 1.0,
                    size: 1.0,
                    fixed: false,
                }; 3],
                [(0, 1, 1.0)],
            );
            for _ in 0..10 {
                layout.iterate();
            }
            assert!(
                layout
                    .positions()
                    .all(|(x, y)| x.is_finite() && y.is_finite())
            );
        }
    }

    #[test]
    fn tous_les_modes_restent_finis() {
        let (nodes, edges) = random_graph(200, 400, 9);
        for adjust_sizes in [false, true] {
            for lin_log_mode in [false, true] {
                for outbound in [false, true] {
                    for strong_gravity_mode in [false, true] {
                        let mut layout = ForceAtlas2::new(Fa2Settings {
                            adjust_sizes,
                            lin_log_mode,
                            outbound_attraction_distribution: outbound,
                            strong_gravity_mode,
                            ..Fa2Settings::default()
                        });
                        layout.load(nodes.iter().copied(), edges.iter().copied());
                        for _ in 0..20 {
                            layout.iterate();
                        }
                        assert!(
                            layout
                                .positions()
                                .all(|(x, y)| x.is_finite() && y.is_finite())
                        );
                    }
                }
            }
        }
    }

    /// Mesure d'une itération sur 5 000 nœuds / 10 000 arêtes (Barnes-Hut,
    /// réglages inférés). `cargo test --release -p lunar-ui-logic --
    /// --ignored --nocapture fa2_benchmark`.
    #[test]
    #[ignore]
    fn fa2_benchmark_5000_noeuds_10000_aretes() {
        let (nodes, edges) = random_graph(5000, 10_000, 1);
        let mut layout = ForceAtlas2::new(infer_settings(5000));
        layout.load(nodes, edges);
        // Échauffement : les premières itérations dispersent le nuage.
        for _ in 0..20 {
            layout.iterate();
        }
        let rounds = 200;
        let start = std::time::Instant::now();
        for _ in 0..rounds {
            layout.iterate();
        }
        let per_iteration = start.elapsed() / rounds;
        println!(
            "ForceAtlas2 5000 nœuds / 10000 arêtes : {per_iteration:?} par itération (Barnes-Hut)"
        );

        let (nodes, edges) = random_graph(2000, 4000, 1);
        let mut layout = ForceAtlas2::new(infer_settings(2000));
        layout.load(nodes, edges);
        let start = std::time::Instant::now();
        for _ in 0..50 {
            layout.iterate();
        }
        println!(
            "ForceAtlas2 2000 nœuds / 4000 arêtes : {:?} par itération (répulsion exacte)",
            start.elapsed() / 50
        );
    }
}
