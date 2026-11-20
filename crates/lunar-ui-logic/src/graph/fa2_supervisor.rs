//! Superviseur ForceAtlas2 réel, réglé pour la taille du graphe.
//!
//! Il suit le superviseur de `graphology-layout-forceatlas2/worker`, sans
//! worker : un appel explicite, [`Fa2Supervisor::step`], que le modèle fait à chaque
//! frame : chaque appel itère puis recopie les positions dans le graphe, comme
//! le superviseur à chaque message du worker. Comme lui, tout ajout de nœud
//! ou d'arête recharge les matrices (`spawnWorker` puis `start`) : masses,
//! forces et convergences repartent de zéro, positions relues du graphe.

use super::force_atlas2::{Fa2Node, Fa2Settings, ForceAtlas2, infer_settings};
use super::force_layout::LayoutSupervisor;
use super::store::GraphStore;

/// Superviseur ForceAtlas2 (`FA2LayoutSupervisor`).
#[derive(Clone, Debug)]
pub struct Fa2Supervisor {
    /// Simulation et ses matrices.
    layout: ForceAtlas2,
    /// Itérations en cours.
    running: bool,
    /// Superviseur détruit.
    killed: bool,
    /// Matrices à recharger depuis le graphe avant la prochaine itération.
    needs_reload: bool,
}

impl Fa2Supervisor {
    /// Superviseur aux réglages donnés, à l'arrêt.
    pub fn new(settings: Fa2Settings) -> Self {
        Self {
            layout: ForceAtlas2::new(settings),
            running: false,
            killed: false,
            needs_reload: true,
        }
    }

    /// Superviseur réglé pour `order` nœuds (`forceAtlas2.inferSettings`).
    pub fn for_order(order: usize) -> Self {
        Self::new(infer_settings(order))
    }

    /// Réglages de la simulation.
    pub fn settings(&self) -> &Fa2Settings {
        self.layout.settings()
    }

    /// Itérations en cours.
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Superviseur détruit.
    pub fn is_killed(&self) -> bool {
        self.killed
    }

    /// Un nœud ou une arête a été ajouté : les matrices seront reconstruites
    ///.
    pub fn graph_changed(&mut self) {
        self.needs_reload = true;
    }

    /// Fait `iterations` itérations si la mise en page tourne, puis recopie
    /// les positions dans `store`. Retourne vrai si des positions ont changé.
    pub fn step(&mut self, store: &mut GraphStore, iterations: usize) -> bool {
        if !self.running || self.killed || iterations == 0 || store.order() == 0 {
            return false;
        }
        if self.needs_reload || self.layout.order() != store.order() {
            self.layout.load(
                store.nodes().iter().map(|n| Fa2Node {
                    x: n.x,
                    y: n.y,
                    size: n.size,
                    fixed: false,
                }),
                store
                    .edges()
                    .iter()
                    .map(|e| (e.source.index(), e.target.index(), 1.0)),
            );
            self.needs_reload = false;
        }
        for _ in 0..iterations {
            self.layout.iterate();
        }
        for (node, (x, y)) in store.nodes.iter_mut().zip(self.layout.positions()) {
            node.x = x;
            node.y = y;
        }
        store.touch_positions();
        true
    }
}

impl LayoutSupervisor for Fa2Supervisor {
    fn start(&mut self) {
        // Un superviseur détruit ne redémarre jamais.
        if self.killed || self.running {
            return;
        }
        // `start` reconstruit toujours les matrices depuis le graphe.
        self.needs_reload = true;
        self.running = true;
    }

    fn stop(&mut self) {
        self.running = false;
    }

    fn kill(&mut self) {
        self.running = false;
        self.killed = true;
        self.layout = ForceAtlas2::new(*self.layout.settings());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::rng::Rng;
    use crate::graph::style::tests::{edge, node};

    fn graph() -> GraphStore {
        let mut g = GraphStore::new(Rng::with_seed(2));
        for id in ["a", "b", "c"] {
            g.upsert_node(&node(id));
        }
        g.upsert_edge(&edge("a", "b", "TCP"));
        g
    }

    fn positions(g: &GraphStore) -> Vec<(f32, f32)> {
        g.nodes().iter().map(|n| (n.x, n.y)).collect()
    }

    #[test]
    fn reglages_inferes_de_l_ordre() {
        assert_eq!(
            *Fa2Supervisor::for_order(3000).settings(),
            infer_settings(3000)
        );
    }

    #[test]
    fn n_itere_que_demarre() {
        let mut g = graph();
        let before = positions(&g);
        let mut sup = Fa2Supervisor::for_order(g.order());
        assert!(!sup.step(&mut g, 1));
        assert_eq!(positions(&g), before);

        sup.start();
        let version = g.positions_version();
        assert!(sup.step(&mut g, 3));
        assert_ne!(positions(&g), before);
        assert_ne!(g.positions_version(), version);

        sup.stop();
        let paused = positions(&g);
        assert!(!sup.step(&mut g, 3));
        assert_eq!(positions(&g), paused);
    }

    #[test]
    fn un_ajout_recharge_les_matrices() {
        let mut g = graph();
        let mut sup = Fa2Supervisor::for_order(g.order());
        sup.start();
        sup.step(&mut g, 1);
        g.upsert_node(&node("d"));
        g.upsert_edge(&edge("d", "a", "UDP"));
        sup.graph_changed();
        assert!(sup.step(&mut g, 1));
        assert_eq!(sup.layout.order(), 4);
        assert!(g.nodes().iter().all(|n| n.x.is_finite() && n.y.is_finite()));
    }

    #[test]
    fn detruit_ne_redemarre_plus() {
        let mut g = graph();
        let mut sup = Fa2Supervisor::for_order(g.order());
        sup.start();
        sup.kill();
        sup.start();
        assert!(!sup.is_running());
        assert!(sup.is_killed());
        assert!(!sup.step(&mut g, 1));
    }
}
