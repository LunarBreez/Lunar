//! Fil ForceAtlas2 : les itérations tournent hors du fil d'interface et
//! publient leurs positions dans un tampon que le rendu copie (quelques
//! dizaines de Ko).
//!
//! Le fil détient son propre [`GraphModel`] (« modèle de mise en page »),
//! qui reçoit exactement les mêmes mutations que le modèle d'interface :
//! updates, snapshots, réinitialisations, bouton « Gravité ». Les deux
//! modèles partent de la même graine : rangs des nœuds, positions initiales
//! (couronne, jitter d'apparition) et courbures sont identiques, et le
//! modèle d'interface garde survol, sélection, tunnels et libellés sans
//! jamais attendre une itération (12 ms sur 5 000 nœuds en Barnes-Hut).
//!
//! Le modèle d'interface reste l'autorité de l'arrêt automatique des
//! 20 minutes : l'horloge du modèle de mise en page est figée, il
//! ne s'arrête que sur ordre ([`LayoutCommand::SetForce`]).
//!
//! Cadence : une itération à la fois, au plus une toutes les
//! [`MIN_ITERATION_PERIOD`] (les petits graphes ne monopolisent pas un cœur
//! à des centaines de milliers d'itérations par seconde), positions publiées
//! au plus toutes les [`PUBLISH_PERIOD`], puis réveil du fil d'interface
//! (coalescé : un seul réveil en attente à la fois).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use lunar_flows_core::graph::{GraphData, GraphUpdate};
use lunar_ui_logic::graph::GraphModel;
use lunar_ui_logic::graph::rng::Rng;

/// Période minimale d'une itération (plafond de 1 000 itérations/s).
pub const MIN_ITERATION_PERIOD: Duration = Duration::from_millis(1);

/// Période minimale entre deux publications de positions.
pub const PUBLISH_PERIOD: Duration = Duration::from_millis(8);

/// Ordre envoyé au fil de mise en page.
pub enum LayoutCommand {
    /// Updates du cœur, dans l'ordre reçu par le modèle d'interface.
    Updates(Vec<GraphUpdate>),
    /// Rechargement complet (`load_snapshot`), nouvelle génération.
    Snapshot {
        /// Snapshot du cœur, partagé avec le modèle d'interface.
        data: Arc<GraphData>,
        /// Génération du graphe chargé.
        generation: u64,
    },
    /// Graphe vidé (`reset`), nouvelle génération.
    Reset {
        /// Génération du graphe vide.
        generation: u64,
    },
    /// État voulu du bouton « Gravité ».
    SetForce(bool),
}

/// Positions publiées par le fil.
#[derive(Clone, Debug, Default)]
pub struct Published {
    /// Génération du graphe dont viennent les positions.
    pub generation: u64,
    /// Numéro de publication, croissant.
    pub version: u64,
    /// Position de chaque nœud (rang = `NodeIndex::index`).
    pub positions: Vec<(f32, f32)>,
}

/// Mesures du fil, lues par la démo et le banc d'essai.
#[derive(Debug, Default)]
pub struct LayoutStats {
    /// Itérations faites depuis le démarrage.
    pub iterations: AtomicU64,
    /// Durée cumulée des itérations, en microsecondes.
    pub iteration_micros: AtomicU64,
    /// Publications faites.
    pub publications: AtomicU64,
    /// Le fil itère en ce moment.
    pub running: AtomicBool,
}

/// État partagé entre le fil et l'interface.
struct Shared {
    /// Dernières positions publiées.
    published: Mutex<Published>,
    /// Un réveil est déjà en attente côté interface.
    wake_pending: AtomicBool,
    /// Réveil du fil d'interface.
    wake: Box<dyn Fn() + Send + Sync>,
    /// Mesures.
    stats: LayoutStats,
}

impl Shared {
    /// Réveille l'interface si aucun réveil n'est en attente.
    fn wake(&self) {
        if !self.wake_pending.swap(true, Ordering::AcqRel) {
            (self.wake)();
        }
    }
}

/// Poignée du fil de mise en page ; l'arrête à la destruction.
pub struct LayoutWorker {
    /// File des ordres (fermée à la destruction : le fil s'arrête).
    tx: Option<Sender<LayoutCommand>>,
    /// État partagé.
    shared: Arc<Shared>,
    /// Fil.
    thread: Option<JoinHandle<()>>,
}

impl LayoutWorker {
    /// Lance le fil ; `seed` est la graine commune aux deux modèles, `wake`
    /// réveille l'interface après une publication.
    pub fn spawn(seed: u64, wake: impl Fn() + Send + Sync + 'static) -> Self {
        let (tx, rx) = mpsc::channel();
        let shared = Arc::new(Shared {
            published: Mutex::new(Published::default()),
            wake_pending: AtomicBool::new(false),
            wake: Box::new(wake),
            stats: LayoutStats::default(),
        });
        let thread_shared = shared.clone();
        let thread = std::thread::Builder::new()
            .name("lunar-fa2".into())
            .spawn(move || run(rx, &thread_shared, seed))
            .map_err(|err| log::error!("fil ForceAtlas2 impossible à lancer : {err}"))
            .ok();
        Self {
            tx: Some(tx),
            shared,
            thread,
        }
    }

    /// Envoie un ordre au fil.
    pub fn send(&self, command: LayoutCommand) {
        if let Some(tx) = &self.tx
            && tx.send(command).is_err()
        {
            log::warn!("fil ForceAtlas2 arrêté : ordre ignoré");
        }
    }

    /// L'interface a traité le réveil : le prochain sera transmis.
    pub fn acknowledge_wake(&self) {
        self.shared.wake_pending.store(false, Ordering::Release);
    }

    /// Copie dans `out` les positions publiées pour la génération
    /// `generation` ; retourne leur numéro de publication, ou `None` si le
    /// fil n'a encore rien publié pour ce graphe.
    pub fn read_positions(&self, generation: u64, out: &mut Vec<(f32, f32)>) -> Option<u64> {
        let published = self
            .shared
            .published
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if published.generation != generation || published.version == 0 {
            return None;
        }
        out.clear();
        out.extend_from_slice(&published.positions);
        Some(published.version)
    }

    /// Mesures du fil.
    pub fn stats(&self) -> &LayoutStats {
        &self.shared.stats
    }
}

impl Drop for LayoutWorker {
    fn drop(&mut self) {
        // Fermer la file réveille le fil s'il attend, et l'arrête.
        self.tx = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Modèle de mise en page : graine commune, horloge figée (l'arrêt
/// automatique est décidé par le modèle d'interface).
pub fn layout_model(seed: u64) -> GraphModel {
    let frozen = Instant::now();
    GraphModel::with_rng_and_clock(Rng::with_seed(seed), Box::new(move || frozen))
}

/// Applique un ordre au modèle de mise en page ; retourne la nouvelle
/// génération s'il en ouvre une. Les updates sont mises en file comme dans
/// le modèle d'interface (qui les applique une fois par image) : l'appelant
/// appelle `flush` après une rafale d'ordres, et un `reset` abandonne la
/// file des deux côtés.
pub fn apply_command(model: &mut GraphModel, command: LayoutCommand) -> Option<u64> {
    match command {
        LayoutCommand::Updates(updates) => {
            model.enqueue_all(updates);
            None
        }
        LayoutCommand::Snapshot { data, generation } => {
            model.load_snapshot(&data);
            Some(generation)
        }
        LayoutCommand::Reset { generation } => {
            model.reset();
            Some(generation)
        }
        LayoutCommand::SetForce(enabled) => {
            if model.force_enabled() != enabled {
                model.toggle_force();
            }
            None
        }
    }
}

/// Boucle du fil.
fn run(rx: Receiver<LayoutCommand>, shared: &Shared, seed: u64) {
    let mut model = layout_model(seed);
    let mut generation = 0u64;
    let mut version = 0u64;
    let mut dirty = false;
    let mut last_publish: Option<Instant> = None;
    loop {
        // Ordres : en attente bloquante quand rien ne tourne.
        let running = model.layout_running();
        shared.stats.running.store(running, Ordering::Relaxed);
        let first = if running {
            match rx.try_recv() {
                Ok(command) => Some(command),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => return,
            }
        } else if dirty {
            // Publication en retard (cadence) : attente bornée.
            match rx.recv_timeout(PUBLISH_PERIOD) {
                Ok(command) => Some(command),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        } else {
            match rx.recv() {
                Ok(command) => Some(command),
                Err(_) => return,
            }
        };
        if let Some(first) = first {
            for command in std::iter::once(first).chain(rx.try_iter()) {
                if let Some(g) = apply_command(&mut model, command) {
                    generation = g;
                }
                dirty = true;
            }
            model.flush();
        }

        if model.layout_running() {
            let started = Instant::now();
            if model.step_layout(1).moved {
                dirty = true;
            }
            let took = started.elapsed();
            shared.stats.iterations.fetch_add(1, Ordering::Relaxed);
            shared
                .stats
                .iteration_micros
                .fetch_add(took.as_micros() as u64, Ordering::Relaxed);
            if took < MIN_ITERATION_PERIOD {
                std::thread::sleep(MIN_ITERATION_PERIOD - took);
            }
        }

        let due = last_publish.is_none_or(|t| t.elapsed() >= PUBLISH_PERIOD);
        if dirty && due {
            version += 1;
            {
                let mut published = shared
                    .published
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                published.generation = generation;
                published.version = version;
                published.positions.clear();
                published
                    .positions
                    .extend(model.store().nodes().iter().map(|n| (n.x, n.y)));
            }
            shared.stats.publications.fetch_add(1, Ordering::Relaxed);
            last_publish = Some(Instant::now());
            dirty = false;
            shared.wake();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph_view::frame::tests::{edge, node, snapshot};

    /// Attend une publication satisfaisant `pred` (2 s au plus).
    fn wait_for(worker: &LayoutWorker, generation: u64, pred: impl Fn(&[(f32, f32)]) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut out = Vec::new();
        while Instant::now() < deadline {
            if worker.read_positions(generation, &mut out).is_some() && pred(&out) {
                return;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        panic!("aucune publication attendue (génération {generation})");
    }

    #[test]
    fn les_deux_modeles_restent_alignes() {
        // Mêmes mutations, même graine : mêmes rangs et mêmes positions tant
        // que ForceAtlas2 n'a pas tourné.
        let data = Arc::new(snapshot(
            &[node("a", "#ff0000"), node("b", "#00ff00")],
            &[edge("a", "b", "TCP", 5000)],
        ));
        let mut ui = GraphModel::with_seed(7);
        let mut layout = layout_model(7);
        ui.load_snapshot(&data);
        apply_command(
            &mut layout,
            LayoutCommand::Snapshot {
                data: data.clone(),
                generation: 1,
            },
        );
        let updates = vec![
            GraphUpdate::NewNode(node("c", "#0000ff")),
            GraphUpdate::NewEdge(edge("c", "a", "UDP", 100)),
        ];
        ui.enqueue_all(updates.clone());
        ui.flush();
        apply_command(&mut layout, LayoutCommand::Updates(updates));
        layout.flush();
        let ids = |m: &GraphModel| -> Vec<(String, f32, f32)> {
            m.store()
                .nodes()
                .iter()
                .map(|n| (n.id.clone(), n.x, n.y))
                .collect()
        };
        assert_eq!(ids(&ui), ids(&layout));
        // Horloge figée : la gravité ne se coupe jamais seule.
        assert!(layout.force_enabled());
        apply_command(&mut layout, LayoutCommand::SetForce(false));
        assert!(!layout.force_enabled() && !layout.layout_running());
    }

    #[test]
    fn le_fil_itere_publie_et_reveille() {
        let wakes = Arc::new(AtomicU64::new(0));
        let counter = wakes.clone();
        let worker = LayoutWorker::spawn(3, move || {
            counter.fetch_add(1, Ordering::Relaxed);
        });
        let data = Arc::new(snapshot(
            &[
                node("a", "#ff0000"),
                node("b", "#00ff00"),
                node("c", "#0000ff"),
            ],
            &[edge("a", "b", "TCP", 5000), edge("b", "c", "TCP", 5000)],
        ));
        let mut ui = GraphModel::with_seed(3);
        ui.load_snapshot(&data);
        let initial: Vec<_> = ui.store().nodes().iter().map(|n| (n.x, n.y)).collect();
        worker.send(LayoutCommand::Snapshot {
            data,
            generation: 1,
        });
        // Les positions bougent (gravité active par défaut).
        wait_for(&worker, 1, |p| p.len() == 3 && p != initial.as_slice());
        assert!(wakes.load(Ordering::Relaxed) >= 1);
        assert!(worker.stats().iterations.load(Ordering::Relaxed) > 0);
        // Une génération périmée n'est pas lue.
        assert_eq!(worker.read_positions(0, &mut Vec::new()), None);

        // Gravité coupée : le fil s'arrête et publie une dernière fois.
        worker.send(LayoutCommand::SetForce(false));
        let deadline = Instant::now() + Duration::from_secs(2);
        while worker.stats().running.load(Ordering::Relaxed) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(!worker.stats().running.load(Ordering::Relaxed));
        let mut a = Vec::new();
        let mut b = Vec::new();
        worker.acknowledge_wake();
        std::thread::sleep(Duration::from_millis(30));
        worker.read_positions(1, &mut a);
        std::thread::sleep(Duration::from_millis(30));
        worker.read_positions(1, &mut b);
        assert_eq!(a, b, "plus rien ne bouge");

        // Réinitialisation : nouvelle génération, graphe vide.
        worker.send(LayoutCommand::Reset { generation: 2 });
        wait_for(&worker, 2, <[(f32, f32)]>::is_empty);
    }
}
