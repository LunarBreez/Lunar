//! Vue du graphe réseau sur gpui : dessin, ForceAtlas2, export PNG.
//!
//! Architecture :
//! - [`GraphModel`] (`lunar-ui-logic`) garde l'état d'interface : styles,
//!   survol, sélection, tunnels, labels, bouton « Gravité » ;
//! - [`layout`] fait tourner ForceAtlas2 sur un fil dédié, sur un second
//!   modèle tenu aligné, et publie les positions ;
//! - à chaque image, [`frame`] assemble un instantané (styles du modèle,
//!   positions du fil), [`scene`] en tire les primitives de Sigma pour la
//!   caméra courante ([`camera`]), [`paint`] les dessine avec gpui ;
//! - [`export`] rejoue la même scène hors écran avec tiny-skia (PNG) ;
//! - [`overlays`] : légende, témoin de gravité, boutons de zoom, bandeau.
//!
//! Sans mouvement (gravité coupée ou graphe immobile, pas d'animation de
//! caméra, pas de souris), la vue ne redessine rien et ne réveille rien.
//!
//! La vue n'appelle pas le cœur : elle émet des [`GraphViewEvent`] que
//! l'application traite (enregistrement d'un label, panneau des labels de
//! la matrice) et reçoit ses données par [`GraphView::load_snapshot`] et
//! [`GraphView::apply_updates`].

mod camera;
mod export;
mod font;
mod frame;
mod layout;
mod overlays;
mod paint;
mod scene;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{
    App, Bounds, Context, CursorStyle, DispatchPhase, Entity, EventEmitter, FocusHandle, Focusable,
    FontWeight, Hitbox, HitboxBehavior, KeyBinding, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Render, ScrollWheelEvent, Subscription, Task, Window, actions, canvas,
    div, prelude::*, px, rgb, rgba,
};
use lunar_flows_core::graph::{GraphData, GraphUpdate};
use lunar_ui_logic::graph::{EdgeIndex, GraphModel, LabelEditRequest};

use self::camera::{
    ANIMATE_DURATION, BUTTON_ZOOMING_RATIO, Camera, CameraState, DOUBLE_CLICK_ZOOM_DURATION,
    DOUBLE_CLICK_ZOOMING_RATIO, DRAG_TIMEOUT, DRAGGED_EVENTS_TOLERANCE, Easing, INERTIA_DURATION,
    INERTIA_RATIO, Viewport, WHEEL_DEBOUNCE, WHEEL_ZOOM_DURATION, WHEEL_ZOOMING_RATIO,
};
use self::export::ExportRequest;
use self::frame::GraphFrame;
use self::layout::{LayoutCommand, LayoutWorker};
use self::paint::Painter;
use crate::widgets::button::{Button, ButtonSize};
use crate::widgets::text_input::{TextInput, TextInputEvent};

actions!(
    lunar_graph_view,
    [
        /// Échap : ferme le bandeau du nœud sélectionné.
        ClearSelection
    ]
);

/// Contexte clavier de la vue.
const KEY_CONTEXT: &str = "LunarGraphView";

/// Tolérance du test d'atteinte d'une arête, en pixels autour du trait.
const EDGE_HIT_TOLERANCE: f32 = 2.0;

/// Événements de la vue, traités par l'application.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GraphViewEvent {
    /// Un nœud vient d'être sélectionné : son bandeau et son champ de label
    /// sont ouverts (pour une application qui préfère son propre éditeur).
    LabelEditRequested {
        /// Nœud sélectionné.
        node_id: String,
        /// Label opérateur actuel (vide si aucun).
        current: String,
    },
    /// « Enregistrer » ou Entrée : label appliqué de façon optimiste, à
    /// transmettre au cœur (`add_label(mac, ip, label)`), puis à confirmer
    /// par [`GraphView::confirm_label_edit`] ; les updates rendues par le
    /// cœur passent par [`GraphView::apply_updates`].
    LabelCommitted {
        /// Nœud édité.
        node_id: String,
        /// MAC du nœud.
        mac: String,
        /// IP du nœud.
        ip: String,
        /// Nouveau label, sans blancs de bord.
        label: String,
    },
    /// La gravité a changé d'état (clic, ou arrêt automatique au bout de 20
    /// minutes).
    ForceStateChanged(bool),
    /// « Afficher les labels » : ouvrir le panneau des labels de la matrice.
    MatrixLabelsRequested,
}

/// Mesures de la vue, pour les bancs d'essai.
#[derive(Clone, Copy, Debug, Default)]
pub struct GraphViewStats {
    /// Images dessinées depuis la dernière remise à zéro.
    pub frames: u64,
    /// Cadence moyenne des images, par seconde.
    pub fps: f64,
    /// Pire intervalle entre deux images, en millisecondes.
    pub worst_frame_ms: f64,
    /// Coût processeur moyen d'une image (instantané, scène, dessin), ms.
    pub cpu_ms: f64,
    /// Pire coût processeur d'une image, ms.
    pub worst_cpu_ms: f64,
    /// Part de la construction de la scène dans le coût moyen, ms.
    pub build_ms: f64,
    /// Part de l'émission des primitives gpui dans le coût moyen, ms.
    pub paint_ms: f64,
    /// Dans l'émission : arêtes, ms par image.
    pub edges_ms: f64,
    /// Dans l'émission : nœuds, ms par image.
    pub nodes_ms: f64,
    /// Dans l'émission : labels et calque de survol, ms par image.
    pub labels_ms: f64,
    /// Triangles d'arêtes de la dernière image.
    pub triangles: usize,
    /// Itérations ForceAtlas2 faites par le fil.
    pub fa2_iterations: u64,
    /// Durée moyenne d'une itération ForceAtlas2, ms.
    pub fa2_ms: f64,
    /// Publications de positions par le fil.
    pub fa2_publications: u64,
    /// Le fil itère en ce moment.
    pub fa2_running: bool,
}

/// Horloge des images : cadence et pire intervalle.
#[derive(Clone, Copy, Debug, Default)]
struct FrameClock {
    /// Première image mesurée.
    first: Option<Instant>,
    /// Dernière image.
    last: Option<Instant>,
    /// Images mesurées.
    frames: u64,
    /// Pire intervalle.
    worst: Duration,
    /// Coût processeur cumulé (µs).
    cpu_micros: u64,
    /// Pire coût processeur (µs).
    worst_cpu: u64,
    /// Part construction (µs).
    build_micros: u64,
    /// Part dessin (µs).
    paint_micros: u64,
}

/// Glissé en cours (`MouseCaptor`).
#[derive(Clone, Copy, Debug)]
struct Drag {
    /// Dernière position du pointeur, en pixels de la zone de dessin.
    last: (f32, f32),
    /// Mouvements reçus depuis l'appui.
    moves: u32,
    /// Instant du dernier mouvement.
    last_move: Instant,
    /// Caméra avant le dernier mouvement (inertie).
    previous: CameraState,
    /// Nombre de clics de l'appui (double clic).
    click_count: usize,
}

/// Vue du graphe réseau.
pub struct GraphView {
    /// Focus (Échap).
    focus_handle: FocusHandle,
    /// Indicateur « Gravité » dessiné sur le graphe. L'application le masque :
    /// son bouton « Gravité » de la rangée du haut en tient lieu ; la démo
    /// `graph_demo` le garde.
    force_indicator: bool,
    /// Modèle d'interface.
    model: GraphModel,
    /// Fil ForceAtlas2.
    worker: LayoutWorker,
    /// Génération du graphe (snapshot, reset).
    generation: u64,
    /// Positions copiées depuis le fil.
    positions: Vec<(f32, f32)>,
    /// Version des positions copiées.
    positions_version: Option<u64>,
    /// Le fil a publié depuis la dernière image.
    positions_dirty: bool,
    /// Instantané de la dernière image.
    frame: GraphFrame,
    /// L'instantané doit être recalculé.
    frame_dirty: bool,
    /// La scène doit être reconstruite.
    scene_dirty: bool,
    /// Caméra.
    camera: Camera,
    /// Taille de la zone de dessin à la dernière image.
    size: (f32, f32),
    /// Facteur d'échelle de la fenêtre (export PNG).
    scale_factor: f32,
    /// Peintre partagé avec l'élément de dessin.
    painter: Rc<RefCell<Painter>>,
    /// Glissé en cours.
    drag: Option<Drag>,
    /// Dernier cran de molette : sens et instant.
    last_wheel: Option<(i8, Instant)>,
    /// Arête survolée.
    hovered_edge: Option<EdgeIndex>,
    /// Champ de label du bandeau.
    label_input: Entity<TextInput>,
    /// Saisie reflétée dans le champ.
    synced_label: String,
    /// Édition de label en attente de confirmation du cœur.
    pending_edit: Option<LabelEditRequest>,
    /// Légende dépliée.
    legend_expanded: bool,
    /// Échéance d'arrêt automatique surveillée.
    scheduled_deadline: Option<Instant>,
    /// Minuterie de l'arrêt automatique.
    _auto_stop: Option<Task<()>>,
    /// Réception des réveils du fil.
    _wake: Task<()>,
    /// Abonnements.
    _subscriptions: Vec<Subscription>,
    /// Redessin continu (banc d'essai).
    continuous: bool,
    /// Mesures des images.
    clock: FrameClock,
}

impl EventEmitter<GraphViewEvent> for GraphView {}

impl Focusable for GraphView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// Raccourcis de la vue (une fois par processus).
fn bind_keys(cx: &mut App) {
    static BOUND: std::sync::Once = std::sync::Once::new();
    BOUND.call_once(|| {
        cx.bind_keys([KeyBinding::new("escape", ClearSelection, Some(KEY_CONTEXT))]);
    });
}

/// Graine commune aux deux modèles, tirée de l'horloge : les positions
/// initiales changent d'un lancement à l'autre.
fn fresh_seed() -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    nanos ^ 0x9e37_79b9_7f4a_7c15
}

impl GraphView {
    /// Vue vide, gravité active (état initial).
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let _ = window;
        bind_keys(cx);
        font::register(cx);
        let seed = fresh_seed();

        let (tx, mut rx) = mpsc::unbounded::<()>();
        let worker = LayoutWorker::spawn(seed, move || {
            let _ = tx.unbounded_send(());
        });
        let wake = cx.spawn(async move |this, cx| {
            while rx.next().await.is_some() {
                let alive = this.update(cx, |view, cx| {
                    view.worker.acknowledge_wake();
                    view.positions_dirty = true;
                    cx.notify();
                });
                if alive.is_err() {
                    break;
                }
            }
        });

        let label_input = cx.new(|cx| TextInput::new(cx).placeholder("Entrer un label…"));
        let subscription = cx.subscribe(&label_input, |view, _, event, cx| match event {
            TextInputEvent::Changed(text) => {
                view.synced_label = text.to_string();
                view.model.set_edited_label(text.to_string());
            }
            TextInputEvent::Submitted(text) => {
                view.synced_label = text.to_string();
                view.model.set_edited_label(text.to_string());
                view.save_label(cx);
            }
        });

        Self {
            focus_handle: cx.focus_handle(),
            force_indicator: true,
            model: GraphModel::with_seed(seed),
            worker,
            generation: 0,
            positions: Vec::new(),
            positions_version: None,
            positions_dirty: false,
            frame: GraphFrame::default(),
            frame_dirty: true,
            scene_dirty: true,
            camera: Camera::default(),
            size: (0.0, 0.0),
            scale_factor: 1.0,
            painter: Rc::new(RefCell::new(Painter::default())),
            drag: None,
            last_wheel: None,
            hovered_edge: None,
            label_input,
            synced_label: String::new(),
            pending_edit: None,
            legend_expanded: true,
            scheduled_deadline: None,
            _auto_stop: None,
            _wake: wake,
            _subscriptions: vec![subscription],
            continuous: false,
            clock: FrameClock::default(),
        }
    }

    // === Données ===============================================================

    /// Updates du cœur (capture, labels) : mises en file et appliquées une
    /// fois par image.
    pub fn apply_updates(&mut self, updates: Vec<GraphUpdate>, cx: &mut Context<Self>) {
        if updates.is_empty() {
            return;
        }
        self.model.enqueue_all(updates.iter().cloned());
        self.worker.send(LayoutCommand::Updates(updates));
        cx.notify();
    }

    /// Remplace le graphe par un snapshot du cœur (`get_graph_snapshot`) :
    /// nœuds en couronne que ForceAtlas2 démêle, caméra recadrée.
    pub fn load_snapshot(&mut self, data: GraphData, cx: &mut Context<Self>) {
        self.release_pending_edit();
        let data = Arc::new(data);
        self.model.load_snapshot(&data);
        self.generation += 1;
        self.worker.send(LayoutCommand::Snapshot {
            data,
            generation: self.generation,
        });
        self.after_graph_replaced(cx);
    }

    /// Vide le graphe (nouvelle capture).
    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.release_pending_edit();
        self.model.reset();
        self.generation += 1;
        self.worker.send(LayoutCommand::Reset {
            generation: self.generation,
        });
        self.after_graph_replaced(cx);
    }

    /// Libère une édition de label en attente : le graphe change sous elle.
    fn release_pending_edit(&mut self) {
        if let Some(request) = self.pending_edit.take() {
            self.model.commit_label_edit(request, Vec::new());
        }
    }

    /// Suites communes d'un snapshot et d'un reset.
    fn after_graph_replaced(&mut self, cx: &mut Context<Self>) {
        self.positions.clear();
        self.positions_version = None;
        self.hovered_edge = None;
        self.drag = None;
        self.frame_dirty = true;
        cx.notify();
    }

    /// Modèle d'interface (lecture : totaux, sélection, zoom).
    pub fn model(&self) -> &GraphModel {
        &self.model
    }

    // === Gravité =================================================================

    /// Bouton « Gravité » : coupe ou relance ForceAtlas2 (pour 20 minutes).
    pub fn toggle_force(&mut self, cx: &mut Context<Self>) {
        self.model.toggle_force();
        let on = self.model.force_enabled();
        self.worker.send(LayoutCommand::SetForce(on));
        cx.emit(GraphViewEvent::ForceStateChanged(on));
        cx.notify();
    }

    /// Masque l'indicateur « Gravité » dessiné sur le graphe (l'application
    /// a son propre bouton).
    pub fn hide_force_indicator(&mut self, cx: &mut Context<Self>) {
        self.force_indicator = false;
        cx.notify();
    }

    /// Gravité active (« Gravité: ON »).
    pub fn is_force_running(&self) -> bool {
        self.model.force_enabled()
    }

    /// Relève l'arrêt automatique des 20 minutes.
    fn poll_auto_stop(&mut self, cx: &mut Context<Self>) {
        if self.model.step_layout(0).auto_stopped {
            self.worker.send(LayoutCommand::SetForce(false));
            cx.emit(GraphViewEvent::ForceStateChanged(false));
            cx.notify();
        }
    }

    /// Arme une minuterie à l'échéance d'arrêt automatique si elle a changé.
    fn schedule_auto_stop(&mut self, cx: &mut Context<Self>) {
        let deadline = self.model.layout().auto_stop_deadline();
        if deadline == self.scheduled_deadline {
            return;
        }
        self.scheduled_deadline = deadline;
        self._auto_stop = deadline.map(|deadline| {
            cx.spawn(async move |this, cx| {
                let wait = deadline.saturating_duration_since(Instant::now());
                cx.background_executor()
                    .timer(wait + Duration::from_millis(20))
                    .await;
                let _ = this.update(cx, |view, cx| view.poll_auto_stop(cx));
            })
        });
    }

    // === Export ==================================================================

    /// Exporte le graphe en PNG : rendu hors écran de 2 ×
    /// la vue, fond blanc, tramé sur un fil d'arrière-plan.
    pub fn export_png(&self, path: PathBuf, cx: &mut Context<Self>) -> Task<Result<(), String>> {
        let request = ExportRequest {
            frame: self.frame.clone(),
            width: self.size.0.max(1.0),
            height: self.size.1.max(1.0),
            camera: self.camera.state(),
            pixel_ratio: self.scale_factor,
        };
        cx.background_spawn(async move { export::write_png(&request, &path) })
    }

    // === Caméra ==================================================================

    /// Zoom avant animé (`camera.animatedZoom`).
    pub fn zoom_in(&mut self, cx: &mut Context<Self>) {
        let target = self.camera.state();
        self.camera.animate(
            CameraState {
                ratio: target.ratio / BUTTON_ZOOMING_RATIO,
                ..target
            },
            ANIMATE_DURATION,
            Easing::QuadraticInOut,
            Instant::now(),
        );
        cx.notify();
    }

    /// Zoom arrière animé (`camera.animatedUnzoom`).
    pub fn zoom_out(&mut self, cx: &mut Context<Self>) {
        let target = self.camera.state();
        self.camera.animate(
            CameraState {
                ratio: target.ratio * BUTTON_ZOOMING_RATIO,
                ..target
            },
            ANIMATE_DURATION,
            Easing::QuadraticInOut,
            Instant::now(),
        );
        cx.notify();
    }

    /// Recadrage animé sur tout le graphe (`camera.animatedReset`).
    pub fn reset_camera(&mut self, cx: &mut Context<Self>) {
        self.camera.animate(
            CameraState::default(),
            ANIMATE_DURATION,
            Easing::QuadraticInOut,
            Instant::now(),
        );
        cx.notify();
    }

    // === Labels ==================================================================

    /// « Enregistrer » / Entrée.
    fn save_label(&mut self, cx: &mut Context<Self>) {
        if self.model.is_saving_label() || self.pending_edit.is_some() {
            return;
        }
        let Some(request) = self.model.begin_label_edit() else {
            return;
        };
        cx.emit(GraphViewEvent::LabelCommitted {
            node_id: request.node_id.clone(),
            mac: request.mac.clone(),
            ip: request.ip.clone(),
            label: request.label.clone(),
        });
        self.pending_edit = Some(request);
        self.frame_dirty = true;
        cx.notify();
    }

    /// Réponse du cœur à [`GraphViewEvent::LabelCommitted`] : `ok` garde le
    /// label (les updates du cœur arrivent par [`Self::apply_updates`]),
    /// sinon retour arrière et bandeau restauré.
    pub fn confirm_label_edit(&mut self, ok: bool, cx: &mut Context<Self>) {
        let Some(request) = self.pending_edit.take() else {
            return;
        };
        if ok {
            self.model.commit_label_edit(request, Vec::new());
        } else {
            self.model.fail_label_edit(request);
        }
        self.frame_dirty = true;
        cx.notify();
    }

    /// « Annuler ».
    fn cancel_label_edit(&mut self, cx: &mut Context<Self>) {
        self.model.cancel_edit();
        cx.notify();
    }

    /// Ferme le bandeau du nœud sélectionné.
    fn clear_selection(&mut self, cx: &mut Context<Self>) {
        self.model.clear_selection();
        self.frame_dirty = true;
        cx.notify();
    }

    /// Reflète dans le champ la saisie du modèle (clic, annulation, retour
    /// arrière, label resynchronisé par le cœur).
    fn sync_label_input(&mut self, cx: &mut Context<Self>) {
        if self.model.edited_label() != self.synced_label {
            self.synced_label = self.model.edited_label().to_string();
            let text = self.synced_label.clone();
            self.label_input
                .update(cx, |input, cx| input.set_text(&text, cx));
        }
        let saving = self.model.is_saving_label();
        if self.label_input.read(cx).is_disabled() != saving {
            self.label_input
                .update(cx, |input, cx| input.set_disabled(saving, cx));
        }
    }

    // === Bancs d'essai =============================================================

    /// Redessine à chaque image même sans mouvement (mesure du débit de rendu).
    pub fn set_continuous_redraw(&mut self, continuous: bool, cx: &mut Context<Self>) {
        self.continuous = continuous;
        cx.notify();
    }

    /// Remet les mesures d'images à zéro (après un temps de chauffe).
    pub fn reset_stats(&mut self) {
        self.clock = FrameClock::default();
        self.painter.borrow_mut().stats = paint::PaintStats::default();
    }

    /// Mesures courantes.
    pub fn stats(&self) -> GraphViewStats {
        let c = &self.clock;
        let frames = c.frames;
        let fps = match (c.first, c.last) {
            (Some(a), Some(b)) if frames > 1 => {
                (frames - 1) as f64 / b.duration_since(a).as_secs_f64().max(1e-9)
            }
            _ => 0.0,
        };
        let per = |micros: u64| {
            if frames == 0 {
                0.0
            } else {
                micros as f64 / frames as f64 / 1000.0
            }
        };
        let painter = self.painter.borrow().stats;
        let painter_ms = |micros: u64, frames: u64| {
            if frames == 0 {
                0.0
            } else {
                micros as f64 / frames as f64 / 1000.0
            }
        };
        let worker = self.worker.stats();
        let iterations = worker.iterations.load(Ordering::Relaxed);
        let iteration_micros = worker.iteration_micros.load(Ordering::Relaxed);
        GraphViewStats {
            frames,
            fps,
            worst_frame_ms: c.worst.as_secs_f64() * 1000.0,
            cpu_ms: per(c.cpu_micros),
            worst_cpu_ms: c.worst_cpu as f64 / 1000.0,
            build_ms: per(c.build_micros),
            paint_ms: per(c.paint_micros),
            edges_ms: painter_ms(painter.edges_micros, painter.frames),
            nodes_ms: painter_ms(painter.nodes_micros, painter.frames),
            labels_ms: painter_ms(painter.labels_micros, painter.frames),
            triangles: painter.triangles,
            fa2_iterations: iterations,
            fa2_ms: if iterations == 0 {
                0.0
            } else {
                iteration_micros as f64 / iterations as f64 / 1000.0
            },
            fa2_publications: worker.publications.load(Ordering::Relaxed),
            fa2_running: worker.running.load(Ordering::Relaxed),
        }
    }

    // === Image ======================================================================

    /// Prépare l'image : caméra, positions, instantané, scène.
    fn prepare(&mut self, bounds: Bounds<Pixels>, window: &mut Window) -> u64 {
        let started = Instant::now();
        self.scale_factor = window.scale_factor();
        let size = (f32::from(bounds.size.width), f32::from(bounds.size.height));
        if size != self.size {
            self.size = size;
            self.scene_dirty = true;
        }

        if self.model.take_camera_reset_request() {
            self.camera.animate(
                CameraState::default(),
                ANIMATE_DURATION,
                Easing::QuadraticInOut,
                started,
            );
        }
        if self.camera.tick(started) {
            self.scene_dirty = true;
            if self.model.set_camera_ratio(self.camera.state().ratio) {
                self.frame_dirty = true;
            }
        }

        if self.positions_dirty || self.frame_dirty {
            self.positions_dirty = false;
            let version = self
                .worker
                .read_positions(self.generation, &mut self.positions);
            if version.is_some() && version != self.positions_version {
                self.positions_version = version;
                self.frame_dirty = true;
            }
        }
        if self.frame_dirty {
            self.frame_dirty = false;
            self.scene_dirty = true;
            let positions = self.positions_version.map(|_| self.positions.as_slice());
            self.frame.refresh(&self.model, positions);
        }
        if self.scene_dirty {
            self.scene_dirty = false;
            let viewport = Viewport::new(size.0, size.1, self.frame.bounds, self.camera.state());
            self.painter
                .borrow_mut()
                .build(&self.frame, &viewport, window);
        }
        if self.camera.is_animating() || self.continuous {
            window.request_animation_frame();
        }
        started.elapsed().as_micros() as u64
    }

    /// Image dessinée : mesures.
    fn record_frame(&mut self, prepare_micros: u64, build_before: u64, paint_micros: u64) {
        let now = Instant::now();
        let c = &mut self.clock;
        if let Some(last) = c.last {
            c.worst = c.worst.max(now.duration_since(last));
        }
        c.first.get_or_insert(now);
        c.last = Some(now);
        c.frames += 1;
        let build = self.painter.borrow().stats.build_micros - build_before;
        let cpu = prepare_micros + paint_micros;
        c.cpu_micros += cpu;
        c.worst_cpu = c.worst_cpu.max(cpu);
        c.build_micros += build;
        c.paint_micros += paint_micros;
    }

    /// Point d'un événement en pixels de la zone de dessin.
    fn local(&self, position: gpui::Point<Pixels>) -> (f32, f32) {
        let origin = self.painter.borrow().origin;
        (
            f32::from(position.x - origin.x),
            f32::from(position.y - origin.y),
        )
    }

    /// Projection de la dernière image.
    fn viewport(&self) -> Viewport {
        Viewport::new(
            self.size.0,
            self.size.1,
            self.frame.bounds,
            self.camera.state(),
        )
    }

    // === Souris ======================================================================

    /// Appui : début de glissé (`MouseCaptor.handleDown`).
    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle);
        let p = self.local(event.position);
        self.drag = Some(Drag {
            last: p,
            moves: 0,
            last_move: Instant::now(),
            previous: self.camera.state(),
            click_count: event.click_count,
        });
        cx.notify();
    }

    /// Mouvement : glissé de la caméra, ou survol.
    fn mouse_move(&mut self, event: &MouseMoveEvent, hovered: bool, cx: &mut Context<Self>) {
        let p = self.local(event.position);
        if let Some(mut drag) = self.drag {
            if event.pressed_button != Some(MouseButton::Left) {
                // Relâché hors de la fenêtre.
                self.drag = None;
            } else {
                let viewport = self.viewport();
                let previous = self.camera.state();
                self.camera.set_state(viewport.panned_state(drag.last, p));
                drag.previous = previous;
                drag.last = p;
                drag.moves += 1;
                drag.last_move = Instant::now();
                self.drag = Some(drag);
                self.scene_dirty = true;
                if self.model.set_camera_ratio(self.camera.state().ratio) {
                    self.frame_dirty = true;
                }
                self.update_cursor();
                cx.notify();
                return;
            }
        }
        self.update_hover(hovered.then_some(p), cx);
    }

    /// Relâcher : clic (moins de 3 mouvements), double clic, ou inertie.
    fn mouse_up(&mut self, event: &MouseUpEvent, cx: &mut Context<Self>) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        let p = self.local(event.position);
        let now = Instant::now();
        if drag.moves >= DRAGGED_EVENTS_TOLERANCE {
            // Le graphe a glissé sous le pointeur : le survol sera recalculé
            // au prochain mouvement.
            self.update_hover(None, cx);
            if now.duration_since(drag.last_move) < DRAG_TIMEOUT {
                let s = self.camera.state();
                self.camera.animate(
                    CameraState {
                        x: s.x + INERTIA_RATIO * (s.x - drag.previous.x),
                        y: s.y + INERTIA_RATIO * (s.y - drag.previous.y),
                        ratio: s.ratio,
                    },
                    INERTIA_DURATION,
                    Easing::QuadraticOut,
                    now,
                );
            }
        } else if drag.click_count >= 2 && drag.click_count % 2 == 0 {
            let viewport = self.viewport();
            let ratio = self.camera.state().ratio / DOUBLE_CLICK_ZOOMING_RATIO;
            self.camera.animate(
                viewport.zoomed_state(p.0, p.1, ratio),
                DOUBLE_CLICK_ZOOM_DURATION,
                Easing::QuadraticInOut,
                now,
            );
        } else {
            self.click(p, cx);
        }
        self.update_cursor();
        cx.notify();
    }

    /// Clic sur un nœud, une arête ou le fond.
    fn click(&mut self, p: (f32, f32), cx: &mut Context<Self>) {
        let (node, edge) = {
            let painter = self.painter.borrow();
            let node = painter.scene.node_at(p);
            let edge = if node.is_none() {
                painter.scene.edge_at(p, EDGE_HIT_TOLERANCE)
            } else {
                None
            };
            (node, edge)
        };
        if let Some(index) = node
            .and_then(|n| self.frame.nodes.get(n as usize))
            .map(|n| n.index)
        {
            self.model.click_node(index);
            if let Some(selected) = self.model.selected_node() {
                cx.emit(GraphViewEvent::LabelEditRequested {
                    node_id: selected.id.clone(),
                    current: selected.label.clone(),
                });
            }
        } else if let Some(index) = edge
            .and_then(|e| self.frame.edges.get(e as usize))
            .map(|e| e.index)
        {
            self.model.click_edge(index);
        } else {
            self.model.click_stage();
        }
        self.frame_dirty = true;
    }

    /// Survol au point `p` (absent : pointeur hors de la zone).
    fn update_hover(&mut self, p: Option<(f32, f32)>, cx: &mut Context<Self>) {
        let (node, edge) = match p {
            Some(p) => {
                let painter = self.painter.borrow();
                let node = painter
                    .scene
                    .node_at(p)
                    .and_then(|n| self.frame.nodes.get(n as usize))
                    .map(|n| n.index);
                let edge = if node.is_none() {
                    painter
                        .scene
                        .edge_at(p, EDGE_HIT_TOLERANCE)
                        .and_then(|e| self.frame.edges.get(e as usize))
                        .map(|e| e.index)
                } else {
                    None
                };
                (node, edge)
            }
            None => (None, None),
        };
        let mut changed = false;
        if node != self.model.hovered_node() {
            self.model.hover_node(node);
            changed = true;
        }
        if edge != self.hovered_edge {
            match edge {
                Some(e) => self.model.enter_edge(e),
                None => self.model.leave_edge(),
            }
            self.hovered_edge = edge;
            changed = true;
        }
        if changed {
            self.frame_dirty = true;
            self.update_cursor();
            cx.notify();
        }
    }

    /// Curseur : main fermée pendant un glissé, main au survol d'un nœud ou
    /// d'une arête.
    fn update_cursor(&self) {
        let cursor = if self
            .drag
            .is_some_and(|d| d.moves >= DRAGGED_EVENTS_TOLERANCE)
        {
            CursorStyle::ClosedHand
        } else if self.model.hovered_node().is_some() || self.hovered_edge.is_some() {
            CursorStyle::PointingHand
        } else {
            CursorStyle::Arrow
        };
        self.painter.borrow_mut().cursor = cursor;
    }

    /// Molette : zoom animé autour du pointeur (`MouseCaptor.handleWheel`).
    fn scroll(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let delta = f32::from(event.delta.pixel_delta(px(16.)).y);
        if delta == 0.0 {
            return;
        }
        let direction: i8 = if delta > 0.0 { 1 } else { -1 };
        let now = Instant::now();
        if let Some((last_direction, at)) = self.last_wheel
            && last_direction == direction
            && now.duration_since(at) < WHEEL_DEBOUNCE
        {
            return;
        }
        self.last_wheel = Some((direction, now));
        let p = self.local(event.position);
        let current = self.camera.state();
        let ratio = if direction > 0 {
            current.ratio / WHEEL_ZOOMING_RATIO
        } else {
            current.ratio * WHEEL_ZOOMING_RATIO
        };
        let target = self.viewport().zoomed_state(p.0, p.1, ratio);
        self.camera
            .animate(target, WHEEL_ZOOM_DURATION, Easing::QuadraticOut, now);
        cx.notify();
    }

    // === Éléments ======================================================================

    /// Zone de dessin et ses écouteurs souris.
    fn graph_canvas(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let painter = self.painter.clone();
        let prepaint_entity = entity.clone();
        canvas(
            move |bounds, window, cx| {
                let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
                let build_before = painter_build_micros(&prepaint_entity, cx);
                let prepare = prepaint_entity.update(cx, |view, _| view.prepare(bounds, window));
                (hitbox, prepare, build_before)
            },
            move |bounds, (hitbox, prepare, build_before): (Hitbox, u64, u64), window, cx| {
                let started = Instant::now();
                let cursor = {
                    let mut p = painter.borrow_mut();
                    window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
                        p.paint(bounds, window, cx);
                    });
                    p.cursor
                };
                window.set_cursor_style(cursor, &hitbox);
                let paint = started.elapsed().as_micros() as u64;
                entity.update(cx, |view, _| {
                    view.record_frame(prepare, build_before, paint)
                });
                register_mouse(&entity, &hitbox, window);
            },
        )
        .size_full()
    }

    /// Bandeau bas (`.bottom-info`).
    fn bottom_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let tunnel = self.model.tunnel_info();
        let mut bar = div()
            .flex_none()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_x(px(12.))
            .gap_y(px(8.))
            .px(px(12.))
            .py(px(8.))
            .bg(rgba(0x0f0f0fcc))
            .text_color(rgb(0xeaeaea))
            .text_size(px(13.))
            .border_t_1()
            .border_color(rgb(0x333333))
            .child(format!("Zoom: {}", self.model.zoom_text()))
            .child(overlays::separator())
            .child(div().whitespace_nowrap().child(self.model.counts_label()));
        if !tunnel.is_empty() {
            bar = bar.child(overlays::separator()).child(
                div()
                    .whitespace_nowrap()
                    .text_color(rgb(0x1de9b6))
                    .child(format!("🚇 {tunnel}")),
            );
        }
        let labels_entity = entity.clone();
        bar = bar
            .child(overlays::separator())
            .child(
                Button::new("graph-show-labels", "Afficher les labels")
                    .size(ButtonSize::Small)
                    .tooltip("afficher les labels")
                    .on_click(move |_, _, cx| {
                        labels_entity
                            .update(cx, |_, cx| cx.emit(GraphViewEvent::MatrixLabelsRequested));
                    }),
            )
            .child(overlays::separator());

        if self.model.selected_node_infos().is_empty() {
            return bar.child(
                div()
                    .italic()
                    .opacity(0.7)
                    .child("Clique sur un nœud pour afficher ses informations."),
            );
        }
        let saving = self.model.is_saving_label();
        let save_entity = entity.clone();
        let cancel_entity = entity;
        let edit_row = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .mt(px(6.))
            .mb(px(10.))
            .child("Label :")
            .child(div().min_w(px(220.)).child(self.label_input.clone()))
            .child(
                Button::new(
                    "graph-label-save",
                    if saving {
                        "Enregistrement…"
                    } else {
                        "Enregistrer"
                    },
                )
                .primary()
                .size(ButtonSize::Small)
                .disabled(saving || self.model.selected_node().is_none())
                .tooltip("Valider la modification")
                .on_click(move |_, _, cx| {
                    save_entity.update(cx, |view, cx| view.save_label(cx));
                }),
            )
            .child(
                Button::new("graph-label-cancel", "Annuler")
                    .ghost()
                    .size(ButtonSize::Small)
                    .disabled(saving)
                    .on_click(move |_, _, cx| {
                        cancel_entity.update(cx, |view, cx| view.cancel_label_edit(cx));
                    }),
            );
        let infos = div()
            .flex()
            .flex_wrap()
            .gap(px(10.))
            .mt(px(4.))
            .child("infos :")
            .children(
                self.model
                    .selected_node_infos()
                    .iter()
                    .map(|info| div().opacity(0.95).child(info.clone())),
            );
        bar.child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(
                    div()
                        .font_weight(FontWeight::BOLD)
                        .child("Nœud sélectionné"),
                )
                .child(edit_row)
                .child(infos),
        )
    }
}

/// Durée cumulée de construction de scène du peintre (avant l'image).
fn painter_build_micros(entity: &Entity<GraphView>, cx: &App) -> u64 {
    entity.read(cx).painter.borrow().stats.build_micros
}

/// Écouteurs souris de la zone de dessin pour l'image suivante.
fn register_mouse(entity: &Entity<GraphView>, hitbox: &Hitbox, window: &mut Window) {
    let (e, hb) = (entity.clone(), hitbox.clone());
    window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
        if phase == DispatchPhase::Bubble
            && event.button == MouseButton::Left
            && hb.is_hovered(window)
        {
            e.update(cx, |view, cx| view.mouse_down(event, window, cx));
        }
    });
    let (e, hb) = (entity.clone(), hitbox.clone());
    window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
        if phase == DispatchPhase::Bubble {
            let hovered = hb.is_hovered(window);
            e.update(cx, |view, cx| view.mouse_move(event, hovered, cx));
        }
    });
    let e = entity.clone();
    window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
        if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
            e.update(cx, |view, cx| view.mouse_up(event, cx));
        }
    });
    let (e, hb) = (entity.clone(), hitbox.clone());
    window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
        if phase == DispatchPhase::Bubble && hb.is_hovered(window) {
            e.update(cx, |view, cx| view.scroll(event, cx));
        }
    });
}

impl Render for GraphView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.model.flush() {
            self.frame_dirty = true;
        }
        self.poll_auto_stop(cx);
        self.schedule_auto_stop(cx);
        self.sync_label_input(cx);

        let entity = cx.entity();
        let (zin, zout, zreset, toggle) = (
            entity.clone(),
            entity.clone(),
            entity.clone(),
            entity.clone(),
        );
        let legend = overlays::legend(
            self.legend_expanded,
            Box::new(move |_, _, cx| {
                toggle.update(cx, |view, cx| {
                    view.legend_expanded = !view.legend_expanded;
                    cx.notify();
                });
            }),
        );
        let zoom = overlays::zoom_controls(
            Box::new(move |_, _, cx| zin.update(cx, |view, cx| view.zoom_in(cx))),
            Box::new(move |_, _, cx| zout.update(cx, |view, cx| view.zoom_out(cx))),
            Box::new(move |_, _, cx| zreset.update(cx, |view, cx| view.reset_camera(cx))),
        );

        div()
            .id("graph-view")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|view, _: &ClearSelection, _, cx| view.clear_selection(cx)))
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(rgb(0x111111))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(self.graph_canvas(cx))
                    .children(self.force_indicator.then(|| {
                        overlays::force_indicator(
                            self.model.force_enabled(),
                            self.model.force_button_label(),
                            self.model.force_button_title(),
                        )
                    }))
                    .child(legend)
                    .child(zoom),
            )
            .child(self.bottom_bar(cx))
    }
}
