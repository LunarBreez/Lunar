//! Emplacement du graphe dans la fenêtre.
//!
//! [`GraphSlot`] est l'adaptateur entre l'application et la vue du graphe
//! ([`crate::graph_view::GraphView`]). Il expose les méthodes du contrat de
//! la vue (`apply_updates`, `load_snapshot`, `reset`, `toggle_force`,
//! `is_force_running`, `node_count`, `export_png`, `confirm_label_edit`) et
//! traduit ses événements en [`GraphSlotEvent`] pour l'application.

use std::path::PathBuf;

use gpui::{App, AppContext, Context, Entity, Subscription, Task, Window};
use lunar_engine::state::graph::{GraphData, GraphUpdate};

use crate::graph_view::{GraphView, GraphViewEvent};

/// Vue du graphe branchée dans la fenêtre.
pub type GraphViewImpl = GraphView;

/// Événement émis par [`GraphViewImpl`].
pub type GraphViewEventImpl = GraphViewEvent;

/// Événement du graphe relayé à l'application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphSlotEvent {
    /// L'opérateur ouvre l'éditeur de label d'un nœud.
    LabelEditRequested {
        /// Nœud visé.
        node_id: String,
        /// Label actuel (vide s'il n'y en a pas).
        current: String,
    },
    /// L'opérateur valide un label : l'application l'enregistre par le
    /// moteur (`add_label`), applique les mises à jour rendues puis appelle
    /// [`GraphSlot::confirm_label_edit`].
    LabelCommitted {
        /// Nœud visé.
        node_id: String,
        /// MAC du nœud.
        mac: String,
        /// IP du nœud.
        ip: String,
        /// Label saisi.
        label: String,
    },
    /// La gravité (ForceAtlas2) démarre ou s'arrête.
    ForceStateChanged(bool),
    /// Bouton « Afficher les labels » de la barre du graphe.
    MatrixLabelsRequested,
}

/// Traduit l'événement de la vue en événement de l'application.
fn slot_event(event: &GraphViewEventImpl) -> GraphSlotEvent {
    match event {
        GraphViewEvent::LabelEditRequested { node_id, current } => {
            GraphSlotEvent::LabelEditRequested {
                node_id: node_id.clone(),
                current: current.clone(),
            }
        }
        GraphViewEvent::LabelCommitted {
            node_id,
            mac,
            ip,
            label,
        } => GraphSlotEvent::LabelCommitted {
            node_id: node_id.clone(),
            mac: mac.clone(),
            ip: ip.clone(),
            label: label.clone(),
        },
        GraphViewEvent::ForceStateChanged(running) => GraphSlotEvent::ForceStateChanged(*running),
        GraphViewEvent::MatrixLabelsRequested => GraphSlotEvent::MatrixLabelsRequested,
    }
}

/// Graphe de la fenêtre : la vue et ses commandes.
pub struct GraphSlot {
    /// Vue du graphe.
    view: Entity<GraphViewImpl>,
}

impl GraphSlot {
    /// Crée la vue du graphe.
    pub fn new(window: &mut Window, cx: &mut App) -> Self {
        Self {
            view: cx.new(|cx| {
                let mut view = GraphViewImpl::new(window, cx);
                view.hide_force_indicator(cx);
                view
            }),
        }
    }

    /// Vue à placer dans l'arbre des éléments.
    pub fn view(&self) -> Entity<GraphViewImpl> {
        self.view.clone()
    }

    /// Applique des mises à jour incrémentales (`graph`, `graphBatch`).
    pub fn apply_updates(&self, updates: Vec<GraphUpdate>, cx: &mut App) {
        self.view
            .update(cx, |view, cx| view.apply_updates(updates, cx));
    }

    /// Remplace le graphe par un instantané (`graphSnapshot`).
    pub fn load_snapshot(&self, data: GraphData, cx: &mut App) {
        self.view
            .update(cx, |view, cx| view.load_snapshot(data, cx));
    }

    /// Vide le graphe (réinitialisation du relevé).
    pub fn reset(&self, cx: &mut App) {
        self.view.update(cx, |view, cx| view.reset(cx));
    }

    /// Active ou coupe la gravité.
    pub fn toggle_force(&self, cx: &mut App) {
        self.view.update(cx, |view, cx| view.toggle_force(cx));
    }

    /// Gravité active.
    pub fn is_force_running(&self, cx: &App) -> bool {
        self.view.read(cx).is_force_running()
    }

    /// Nombre de nœuds affichés.
    pub fn node_count(&self, cx: &App) -> usize {
        self.view.read(cx).model().node_count()
    }

    /// Exporte le graphe en PNG.
    pub fn export_png(&self, path: PathBuf, cx: &mut App) -> Task<Result<(), String>> {
        self.view.update(cx, |view, cx| view.export_png(path, cx))
    }

    /// Termine l'édition de label en cours : `ok` garde le label affiché,
    /// sinon la vue revient à l'ancien (`commit_label_edit` /
    /// `fail_label_edit`).
    pub fn confirm_label_edit(&self, ok: bool, cx: &mut App) {
        self.view
            .update(cx, |view, cx| view.confirm_label_edit(ok, cx));
    }

    /// Relaie les événements du graphe à `on_event`.
    pub fn subscribe<T: 'static>(
        &self,
        cx: &mut Context<T>,
        on_event: impl Fn(&mut T, GraphSlotEvent, &mut Context<T>) + 'static,
    ) -> Subscription {
        cx.subscribe(
            &self.view,
            move |this, _, event: &GraphViewEventImpl, cx| on_event(this, slot_event(event), cx),
        )
    }
}
