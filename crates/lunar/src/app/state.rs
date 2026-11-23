//! État de l'application : moteur, vue affichée, panneaux, verrou d'import,
//! store de capture, notifications et dialogues.
//!
//! [`AppState`] est une entité gpui unique, créée avec la fenêtre. Elle
//! porte l'état de la capture et de sa configuration, celui de la vue
//! d'analyse et de la barre d'outils, et les dialogues de l'application.
//!
//! Pour un module de l'interface (panneau, vue d'investigation) :
//! - lire l'état : `state.read(cx).capture()`, `.panels()`, `.view()`… ;
//!   suivre ses changements : `cx.observe(&state, …)` (tout changement) ou
//!   `cx.subscribe(&state, …)` ([`AppEvent`]) ;
//! - appeler le moteur : `state.read(cx).bridge().clone()` puis
//!   [`Bridge::run`] depuis l'entité du module ; une opération exclusive
//!   (import, export) passe par [`AppState::run_locked`], qui tient le
//!   verrou global ;
//! - notifier : [`AppState::toast`] ; montrer une erreur du moteur :
//!   [`AppState::show_error`].

mod live_capture;

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;

use futures::StreamExt;
use futures::channel::mpsc::UnboundedReceiver;
use gpui::{AppContext, Context, Entity, EventEmitter, SharedString, Subscription, Task, Window};
use log::{debug, error, info, warn};
use lunar_engine::Lunar;
use lunar_engine::errors::CaptureStateError;
use lunar_engine::setup::system_info::{CPU_SAMPLE_PERIOD, CpuSampler};
use lunar_engine::state::graph::GraphUpdate;
use lunar_ui_logic::capture_store::{CaptureStore, StoreEffect};
use lunar_ui_logic::errors::{ErrorMessage, capture_error_message};
use lunar_ui_logic::investigation::view::{PARTIAL_SURVEY_TITLE, partial_survey_message};
use lunar_ui_logic::live_capture::LiveCapture;
use lunar_ui_logic::menu::{MenuActionId, MenuContext, MenuView, is_action_enabled};

use crate::app::about::{CHANGELOG_TITLE, changelog_message};
use crate::app::bridge::{Bridge, TaggedEvent};
use crate::app::graph_slot::{GraphSlot, GraphSlotEvent};
use crate::app::packet_log::PacketLog;
use crate::app::panels::{ImportMode, Panel, PanelState};
use crate::app::status_bar::{CaptureTimer, ChannelGauge};
use crate::app::toolbar::ToolbarState;
use crate::widgets::toast::{ToastKind, ToastStack};

/// Message du verrou d'import.
pub const BUSY_MESSAGE: &str =
    "Une opération d'importation ou de sauvegarde est déjà en cours. Veuillez patienter.";

/// Question de la réinitialisation d'un relevé non enregistré.
pub const RESET_QUESTION: &str =
    "La réinitialisation effacera le relevé non enregistré.\nContinuer ?";

/// Question de la fermeture avec un relevé non enregistré.
pub const QUIT_QUESTION: &str =
    "Des modifications non enregistrées seront perdues.\nQuitter quand même ?";

/// Question de la récupération après un arrêt brutal.
pub const RECOVERY_QUESTION: &str = "La session précédente ne s'est pas fermée correctement.\nRécupérer le dernier relevé sauvegardé automatiquement ?";

/// Événements traités par passe de la pompe, avant de redessiner.
pub const EVENT_BATCH_MAX: usize = 256;

/// Dialogue de l'application (question ou message).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppModal {
    /// À propos de LUNAR.
    About,
    /// Raccourcis clavier (F1).
    Shortcuts,
    /// Message d'information ou d'erreur.
    Message {
        /// Titre.
        title: SharedString,
        /// Texte.
        message: SharedString,
    },
    /// Confirmation de la réinitialisation d'un relevé non enregistré.
    ConfirmReset,
    /// Confirmation de la fermeture avec un relevé non enregistré.
    ConfirmQuit,
    /// Récupération de l'autosave d'une session interrompue.
    Recovery {
        /// Autosave proposé.
        path: String,
    },
}

/// Changement notable de l'état, pour les vues qui s'y abonnent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppEvent {
    /// L'état d'activation des entrées de menu a changé.
    MenuContextChanged(MenuContext),
    /// Un panneau s'est ouvert ou fermé.
    PanelsChanged,
    /// La vue affichée a changé.
    ViewChanged(MenuView),
    /// Un dialogue s'est ouvert ou fermé.
    ModalsChanged,
    /// Le relevé a été réinitialisé.
    Reset,
    /// Action de menu propre à la vue d'investigation (export de synthèse,
    /// labels à compléter, rafraîchissement), exécutée par la vue.
    InvestigationAction(MenuActionId),
}

/// Indicateurs de la vue d'investigation, tenus par son module.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InvestigationFlags {
    /// Chargement du relevé en cours.
    pub loading: bool,
    /// Export en cours.
    pub exporting: bool,
}

/// État de l'application.
pub struct AppState {
    /// Accès au moteur.
    bridge: Bridge,
    /// Vue affichée.
    view: MenuView,
    /// Panneaux ouverts.
    panels: PanelState,
    /// Store de capture : capture en cours, verrou d'import, filtre,
    /// compteurs, bilan, présence de données.
    capture: CaptureStore,
    /// Démarrage ou arrêt de la capture live en vol.
    live: LiveCapture,
    /// Le relevé serait perdu sans enregistrement (dernier relevé connu ;
    /// les confirmations interrogent toujours le moteur).
    session_dirty: bool,
    /// Graphe affiché.
    show_graph: bool,
    /// Indicateurs de la vue d'investigation.
    investigation: InvestigationFlags,
    /// Jauge du canal de capture.
    channel: ChannelGauge,
    /// Usage CPU, relevé chaque seconde.
    cpu_usage: f32,
    /// Durée de la capture.
    timer: CaptureTimer,
    /// Dernières trames reçues.
    packet_log: PacketLog,
    /// Graphe.
    graph: GraphSlot,
    /// Notifications.
    toasts: Entity<ToastStack>,
    /// Dialogues en attente, le premier affiché, avec leur numéro.
    modals: VecDeque<(u64, AppModal)>,
    /// Numéro du dernier dialogue ouvert.
    modal_seq: u64,
    /// Fermeture engagée.
    exit_in_progress: bool,
    /// Dernier état d'activation des menus annoncé.
    menu_context: MenuContext,
    /// Pompe d'événements et relevé CPU.
    _tasks: Vec<Task<()>>,
    /// Abonnement aux événements du graphe.
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<AppEvent> for AppState {}

impl AppState {
    /// État initial : pont vers `lunar`, pompe d'événements, relevé CPU,
    /// graphe, configuration de capture et présence de données lues.
    pub fn new(lunar: Arc<Lunar>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (bridge, receiver) = Bridge::new(lunar);
        let toasts = cx.new(|_| ToastStack::new());
        let graph = GraphSlot::new(window, cx);
        let graph_events = graph.subscribe(cx, |this: &mut Self, event, cx| {
            this.on_graph_event(event, cx)
        });
        let tasks = vec![
            Self::spawn_event_pump(receiver, cx),
            Self::spawn_cpu_poll(cx),
        ];
        let mut this = Self {
            bridge,
            view: MenuView::Analyse,
            panels: PanelState::default(),
            capture: CaptureStore::new(),
            live: LiveCapture::new(),
            session_dirty: false,
            show_graph: true,
            investigation: InvestigationFlags::default(),
            channel: ChannelGauge::default(),
            cpu_usage: 0.0,
            timer: CaptureTimer::default(),
            packet_log: PacketLog::new(),
            graph,
            toasts,
            modals: VecDeque::new(),
            modal_seq: 0,
            exit_in_progress: false,
            menu_context: MenuContext::default(),
            _tasks: tasks,
            _subscriptions: vec![graph_events],
        };
        this.menu_context = this.compute_menu_context();
        this.load_capture_config(cx);
        this.refresh_flags(cx);
        this
    }

    // --- Lecture -------------------------------------------------------

    /// Accès au moteur.
    pub fn bridge(&self) -> &Bridge {
        &self.bridge
    }

    /// Vue affichée.
    pub fn view(&self) -> MenuView {
        self.view
    }

    /// Panneaux ouverts.
    pub fn panels(&self) -> &PanelState {
        &self.panels
    }

    /// Store de capture.
    pub fn capture(&self) -> &CaptureStore {
        &self.capture
    }

    /// Dernier état « relevé non enregistré » connu.
    pub fn session_dirty(&self) -> bool {
        self.session_dirty
    }

    /// Graphe affiché.
    pub fn show_graph(&self) -> bool {
        self.show_graph
    }

    /// Graphe.
    pub fn graph(&self) -> &GraphSlot {
        &self.graph
    }

    /// Jauge du canal.
    pub fn channel(&self) -> ChannelGauge {
        self.channel
    }

    /// Usage CPU.
    pub fn cpu_usage(&self) -> f32 {
        self.cpu_usage
    }

    /// Durée de capture.
    pub fn timer(&self) -> &CaptureTimer {
        &self.timer
    }

    /// Dernières trames.
    pub fn packet_log(&self) -> &PacketLog {
        &self.packet_log
    }

    /// Pile des notifications (à rendre dans la racine).
    pub fn toasts(&self) -> &Entity<ToastStack> {
        &self.toasts
    }

    /// Dialogue affiché et son numéro.
    pub fn modal(&self) -> Option<(u64, &AppModal)> {
        self.modals.front().map(|(seq, modal)| (*seq, modal))
    }

    /// État d'activation des menus.
    pub fn menu_context(&self) -> MenuContext {
        self.menu_context
    }

    /// État de la barre d'outils.
    pub fn toolbar_state(&self) -> ToolbarState {
        ToolbarState {
            running: self.capture.is_running,
            has_data: self.capture.has_data,
            open_panel: self.panels.main().filter(|panel| panel.is_toolbar_panel()),
        }
    }

    /// Calcule l'état d'activation des menus.
    fn compute_menu_context(&self) -> MenuContext {
        MenuContext {
            view: self.view,
            running: self.capture.is_running,
            starting: self.capture.is_starting,
            importing: self.capture.is_importing,
            has_data: self.capture.has_data,
            panel_open: self.panels.toolbar_panel_open(),
            loading: self.investigation.loading,
            exporting: self.investigation.exporting,
        }
    }

    // --- Modifications -------------------------------------------------

    /// À appeler après toute modification : durée de capture, état des
    /// menus (annoncé s'il change), redessin.
    fn changed(&mut self, cx: &mut Context<Self>) {
        self.timer.sync(self.capture.is_running, Instant::now());
        let menu_context = self.compute_menu_context();
        if menu_context != self.menu_context {
            self.menu_context = menu_context;
            cx.emit(AppEvent::MenuContextChanged(menu_context));
        }
        cx.notify();
    }

    /// Modifie le store de capture (filtre en attente, démarrage…).
    pub fn update_capture(
        &mut self,
        update: impl FnOnce(&mut CaptureStore),
        cx: &mut Context<Self>,
    ) {
        update(&mut self.capture);
        self.changed(cx);
    }

    /// Indicateurs de la vue d'investigation.
    pub fn set_investigation_flags(&mut self, flags: InvestigationFlags, cx: &mut Context<Self>) {
        self.investigation = flags;
        self.changed(cx);
    }

    /// Affiche une notification.
    pub fn toast(&self, kind: ToastKind, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        let message = message.into();
        self.toasts.update(cx, |toasts, cx| {
            toasts.push(kind, message, cx);
        });
    }

    /// Verrou d'import tenu : journalisé et signalé.
    fn busy(&self, cx: &mut Context<Self>) {
        info!("{BUSY_MESSAGE}");
        self.toast(ToastKind::Info, BUSY_MESSAGE, cx);
    }

    /// Montre une erreur du moteur.
    pub fn show_error(&mut self, err: &CaptureStateError, cx: &mut Context<Self>) {
        self.show_error_message(capture_error_message(err), cx);
    }

    /// Montre un message d'erreur déjà formaté, journalisé au niveau erreur.
    pub fn show_error_message(&mut self, message: ErrorMessage, cx: &mut Context<Self>) {
        error!("{}", message.log_line());
        self.show_message(message.title, message.message, cx);
    }

    /// Ouvre un message.
    pub fn show_message(
        &mut self,
        title: impl Into<SharedString>,
        message: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.push_modal(
            AppModal::Message {
                title: title.into(),
                message: message.into(),
            },
            cx,
        );
    }

    /// Ajoute un dialogue en file (pas de doublon).
    pub fn push_modal(&mut self, modal: AppModal, cx: &mut Context<Self>) {
        if self.modals.iter().any(|(_, queued)| *queued == modal) {
            return;
        }
        self.modal_seq += 1;
        // La confirmation de fermeture passe devant : c'est elle que
        // l'opérateur attend après avoir fermé la fenêtre.
        if modal == AppModal::ConfirmQuit {
            self.modals.push_front((self.modal_seq, modal));
        } else {
            self.modals.push_back((self.modal_seq, modal));
        }
        cx.emit(AppEvent::ModalsChanged);
        self.changed(cx);
    }

    /// Ferme le dialogue affiché.
    pub fn close_modal(&mut self, cx: &mut Context<Self>) {
        if self.modals.pop_front().is_some() {
            cx.emit(AppEvent::ModalsChanged);
            self.changed(cx);
        }
    }

    /// Retire le dialogue affiché s'il vérifie `is`.
    fn take_modal(
        &mut self,
        is: impl Fn(&AppModal) -> bool,
        cx: &mut Context<Self>,
    ) -> Option<AppModal> {
        if !self.modals.front().is_some_and(|(_, modal)| is(modal)) {
            return None;
        }
        let modal = self.modals.pop_front().map(|(_, modal)| modal);
        cx.emit(AppEvent::ModalsChanged);
        self.changed(cx);
        modal
    }

    /// Relit la présence de données et l'état
    /// « non enregistré ».
    pub fn refresh_flags(&mut self, cx: &mut Context<Self>) {
        self.bridge
            .run(
                cx,
                |lunar| (lunar.is_matrix_empty(), lunar.is_session_dirty()),
                |this, (empty, dirty), cx| {
                    match empty {
                        Ok(empty) => this.capture.set_matrix_empty(empty),
                        Err(err) => warn!("is_matrix_empty a échoué : {err}"),
                    }
                    this.session_dirty = dirty.unwrap_or(true);
                    this.changed(cx);
                },
            )
            .detach();
    }

    /// Prévient si le relevé qui vient d'être chargé (import de matrices,
    /// ouverture de projet) provient d'un export partiel.
    pub fn warn_if_partial(&mut self, cx: &mut Context<Self>) {
        self.bridge
            .run(
                cx,
                |lunar| lunar.partial_filters(),
                |this, filters, cx| match filters {
                    Ok(filters) => {
                        if let Some(message) = partial_survey_message(&filters) {
                            warn!("Relevé partiel, filtres : {}", filters.join(" | "));
                            this.show_message(PARTIAL_SURVEY_TITLE, message, cx);
                        }
                    }
                    Err(err) => warn!("partial_filters a échoué : {err}"),
                },
            )
            .detach();
    }

    /// Lit l'interface configurée.
    fn load_capture_config(&mut self, cx: &mut Context<Self>) {
        self.bridge
            .run(
                cx,
                |lunar| lunar.get_config_capture(),
                |this, config, cx| match config {
                    Ok(config) => {
                        this.capture.set_interface(config.device_name);
                        this.changed(cx);
                    }
                    Err(err) => this.show_error(&err, cx),
                },
            )
            .detach();
    }

    /// Exécute une opération exclusive (import, export, ouverture de
    /// projet) sous le verrou global : refusée si le
    /// verrou est déjà tenu, il est rendu avant `done`. Rend faux si
    /// l'opération est refusée.
    pub fn run_locked<R: Send + 'static>(
        &mut self,
        cx: &mut Context<Self>,
        work: impl FnOnce(&Lunar) -> R + Send + 'static,
        done: impl FnOnce(&mut Self, R, &mut Context<Self>) + 'static,
    ) -> bool {
        if self.capture.is_importing {
            self.busy(cx);
            return false;
        }
        self.capture.is_importing = true;
        self.changed(cx);
        self.bridge
            .run(cx, work, move |this, result, cx| {
                this.capture.is_importing = false;
                done(this, result, cx);
                this.changed(cx);
            })
            .detach();
        true
    }

    // --- Événements du moteur ------------------------------------------

    /// Pompe : attend les événements sans scrutation, les traite par lots
    /// sur le thread de rendu.
    fn spawn_event_pump(
        mut receiver: UnboundedReceiver<TaggedEvent>,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        cx.spawn(async move |this, cx| {
            while let Some(first) = receiver.next().await {
                let mut batch = vec![first];
                while batch.len() < EVENT_BATCH_MAX {
                    match receiver.try_recv() {
                        Ok(event) => batch.push(event),
                        Err(_) => break,
                    }
                }
                if this
                    .update(cx, |state, cx| state.handle_events(batch, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
    }

    /// Traite un lot d'événements : ceux d'un canal détaché sont ignorés,
    /// les autres passent par le store puis vont au graphe, au journal de
    /// trames et à la jauge du canal.
    fn handle_events(&mut self, batch: Vec<TaggedEvent>, cx: &mut Context<Self>) {
        let mut updates: Vec<GraphUpdate> = Vec::new();
        for TaggedEvent { tag, event } in batch {
            if !self.bridge.is_store_tag(tag) {
                continue;
            }
            let Some(effect) = self.capture.handle_event(event) else {
                continue;
            };
            match effect {
                StoreEffect::Packets(packets) => self.packet_log.push_batch(&packets),
                StoreEffect::GraphUpdates(batch) => updates.extend(batch),
                StoreEffect::GraphSnapshot(data) => {
                    if !updates.is_empty() {
                        self.graph.apply_updates(std::mem::take(&mut updates), cx);
                    }
                    self.graph.load_snapshot(data, cx);
                }
                StoreEffect::ChannelCapacity {
                    channel_size,
                    current_size,
                    backpressure,
                } => {
                    self.channel =
                        ChannelGauge::from_payload(channel_size, current_size, backpressure)
                }
                StoreEffect::Started
                | StoreEffect::Stats
                | StoreEffect::Stopped { .. }
                | StoreEffect::Finished
                | StoreEffect::ImportProgress => {}
            }
        }
        if !updates.is_empty() {
            self.graph.apply_updates(updates, cx);
        }
        self.changed(cx);
    }

    /// Relevé CPU chaque seconde.
    fn spawn_cpu_poll(cx: &mut Context<Self>) -> Task<()> {
        info!("[Cpu] Listener registered");
        cx.spawn(async move |this, cx| {
            let mut sampler = CpuSampler::new();
            loop {
                cx.background_executor().timer(CPU_SAMPLE_PERIOD).await;
                let (back, usage) = cx
                    .background_executor()
                    .spawn(async move {
                        let usage = sampler.sample().cpu_usage;
                        (sampler, usage)
                    })
                    .await;
                sampler = back;
                let alive = this.update(cx, |state, cx| {
                    state.cpu_usage = usage;
                    cx.notify();
                });
                if alive.is_err() {
                    break;
                }
            }
        })
    }

    /// Événement du graphe.
    fn on_graph_event(&mut self, event: GraphSlotEvent, cx: &mut Context<Self>) {
        match event {
            GraphSlotEvent::LabelEditRequested { node_id, current } => {
                // La vue du graphe ouvre son propre champ de label.
                debug!("[Graphe] édition du label de {node_id} demandée (actuel : {current:?})");
            }
            GraphSlotEvent::LabelCommitted {
                node_id,
                mac,
                ip,
                label,
            } => crate::app::labels::commit_graph_label(self, node_id, mac, ip, label, cx),
            GraphSlotEvent::ForceStateChanged(running) => {
                debug!(
                    "[Graphe] gravité {}",
                    if running { "active" } else { "coupée" }
                );
                self.changed(cx);
            }
            GraphSlotEvent::MatrixLabelsRequested => self.toggle_matrix_labels(cx),
        }
    }

    // --- Actions -------------------------------------------------------

    /// Exécute une action de menu, d'où qu'elle vienne (menu, barre
    /// d'outils, clavier). Ignorée sous un dialogue ou si l'entrée est
    /// grisée ; refusée avec [`BUSY_MESSAGE`] si seul le verrou d'import la
    /// grise.
    pub fn run_action(&mut self, id: MenuActionId, cx: &mut Context<Self>) {
        if !self.modals.is_empty() {
            debug!("[Menu] {id} ignoré : un dialogue est ouvert");
            return;
        }
        info!("[Menu] action {id}");
        let context = self.compute_menu_context();
        if !is_action_enabled(id, &context) {
            let unlocked = MenuContext {
                importing: false,
                ..context
            };
            if context.importing && is_action_enabled(id, &unlocked) {
                self.busy(cx);
            } else {
                debug!("[Menu] {id} inactif dans l'état courant");
            }
            return;
        }
        use MenuActionId as A;
        match id {
            A::OpenInvestigation => {
                info!("[Barre d'outils] Bouton investigation cliqué");
                self.set_view(MenuView::Investigation, cx);
            }
            A::BackToAnalyse => self.set_view(MenuView::Analyse, cx),
            A::OpenConfig => {
                info!("[Barre d'outils] Bouton config cliqué");
                self.toggle_panel(Panel::Config, cx);
            }
            A::OpenFilter => {
                info!("[Barre d'outils] Bouton filter cliqué");
                self.toggle_panel(Panel::Filter, cx);
            }
            A::OpenLabels => self.toggle_panel(Panel::Labels, cx),
            A::OpenCaptureFile => {
                info!("[Barre d'outils] Bouton open cliqué");
                self.toggle_panel(Panel::Import(ImportMode::Capture), cx);
            }
            A::ImportLabels => {
                info!("[Barre d'outils] Bouton open cliqué");
                self.toggle_panel(Panel::Import(ImportMode::Labels), cx);
            }
            A::ToggleGraph => self.toggle_graph(cx),
            A::ResetCapture => self.request_reset(cx),
            A::StartCapture => self.start_capture(cx),
            A::StopCapture => self.stop_capture(cx),
            A::Shortcuts => self.push_modal(AppModal::Shortcuts, cx),
            A::About => self.push_modal(AppModal::About, cx),
            A::Quit => {
                info!("Fermeture demandée");
                self.request_exit(cx);
            }
            A::ExportSynthesis
            | A::ExportLabelsToComplete
            | A::ExportFilteredFlows
            | A::ReloadSurvey => cx.emit(AppEvent::InvestigationAction(id)),
            A::OpenProject => self.open_project_dialog(cx),
            A::SaveProject => self.save_project_dialog(cx),
            A::ExportCsv => self.export_csv_dialog(cx),
            A::ExportLogs => self.export_logs_dialog(cx),
            A::ExportLabels => crate::app::labels::export_labels(self, cx),
            A::Changelog => self.show_message(CHANGELOG_TITLE, changelog_message(), cx),
        }
    }

    /// Change de vue ; les panneaux se ferment (la vue d'investigation
    /// occupe toute la fenêtre).
    pub fn set_view(&mut self, view: MenuView, cx: &mut Context<Self>) {
        if self.view == view {
            return;
        }
        if self.panels.close_all() {
            cx.emit(AppEvent::PanelsChanged);
        }
        self.view = view;
        if view == MenuView::Analyse
            && let Some(line) = self.capture.restoration_log()
        {
            // Retour dans la vue d'analyse : la barre d'état rejournalise
            // la restauration (le harnais E2E l'attend).
            info!("{line}");
        }
        if view == MenuView::Analyse {
            // De même pour le graphe, qui reste vivant pendant
            // l'investigation : le harnais E2E compte ses nœuds.
            info!(
                "[Graphe] Restauration au montage : {} nœuds",
                self.graph.node_count(cx)
            );
        }
        info!(
            "[Vue] {}",
            match view {
                MenuView::Analyse => "analyse",
                MenuView::Investigation => "investigation",
            }
        );
        cx.emit(AppEvent::ViewChanged(view));
        self.changed(cx);
    }

    /// Ouvre ou ferme un panneau : refusé pendant
    /// une capture ou quand il est bloqué (import de captures avec un
    /// relevé chargé), signalé occupé sous le verrou d'import.
    pub fn toggle_panel(&mut self, panel: Panel, cx: &mut Context<Self>) {
        if panel.is_toolbar_panel() {
            let blocked = panel == Panel::Import(ImportMode::Capture) && self.capture.has_data;
            if self.capture.is_running || blocked {
                debug!(
                    "[Panneaux] {} refusé (capture en cours ou relevé chargé)",
                    panel.key()
                );
                return;
            }
        }
        if self.capture.is_importing {
            self.busy(cx);
            return;
        }
        let opened = self.panels.toggle(panel);
        debug!(
            "[Panneaux] {} {}",
            panel.key(),
            if opened { "ouvert" } else { "fermé" }
        );
        cx.emit(AppEvent::PanelsChanged);
        self.changed(cx);
    }

    /// Ouvre un dialogue imbriqué dans le panneau ouvert.
    pub fn open_dialog(&mut self, dialog: Panel, cx: &mut Context<Self>) {
        if self.panels.open_dialog(dialog) {
            debug!("[Panneaux] {} ouvert", dialog.key());
            cx.emit(AppEvent::PanelsChanged);
            self.changed(cx);
        }
    }

    /// Ferme un panneau (bouton de fermeture du panneau).
    pub fn close_panel(&mut self, panel: Panel, cx: &mut Context<Self>) {
        if self.panels.close(panel) {
            debug!("[Panneaux] {} fermé", panel.key());
            cx.emit(AppEvent::PanelsChanged);
            self.changed(cx);
        }
    }

    /// Affiche ou masque le graphe (`toggle-graph`) ; la vue du graphe reste
    /// vivante, ses labels de matrice se ferment.
    fn toggle_graph(&mut self, cx: &mut Context<Self>) {
        self.show_graph = !self.show_graph;
        if !self.show_graph && self.panels.close(Panel::MatrixLabels) {
            cx.emit(AppEvent::PanelsChanged);
        }
        self.changed(cx);
    }

    /// Ouvre ou ferme les labels de la matrice (bouton du graphe).
    pub fn toggle_matrix_labels(&mut self, cx: &mut Context<Self>) {
        if self.view != MenuView::Analyse || !self.show_graph || !self.modals.is_empty() {
            return;
        }
        self.toggle_panel(Panel::MatrixLabels, cx);
    }

    /// Active ou coupe la gravité du graphe.
    pub fn toggle_gravity(&mut self, cx: &mut Context<Self>) {
        if self.view != MenuView::Analyse || !self.show_graph || !self.modals.is_empty() {
            return;
        }
        self.graph.toggle_force(cx);
        info!(
            "[Graphe] gravité {}",
            if self.graph.is_force_running(cx) {
                "activée"
            } else {
                "coupée"
            }
        );
        cx.notify();
    }

    /// Export PNG du graphe (dialogue d'enregistrement, voir
    /// [`crate::app::offline`]).
    pub fn export_graph_png(&mut self, cx: &mut Context<Self>) {
        if self.modals.is_empty() {
            self.export_graph_png_dialog(cx);
        }
    }

    /// Efface le filtre actif (bouton ✕ de la barre de statut).
    pub fn clear_filter(&mut self, cx: &mut Context<Self>) {
        self.bridge
            .run(
                cx,
                |lunar| lunar.set_filter(String::new()),
                |this, result, cx| match result {
                    Ok(()) => {
                        this.capture.set_active_filter("");
                        this.changed(cx);
                    }
                    Err(err) => error!("clear filter failed: {err}"),
                },
            )
            .detach();
    }

    // --- Réinitialisation ----------------------------------------------

    /// Réinitialisation demandée : confirmée si le relevé n'est pas
    /// enregistré.
    fn request_reset(&mut self, cx: &mut Context<Self>) {
        info!("reset");
        self.bridge
            .run(
                cx,
                |lunar| lunar.is_session_dirty(),
                |this, dirty, cx| {
                    if dirty.unwrap_or(true) {
                        this.push_modal(AppModal::ConfirmReset, cx);
                    } else {
                        this.reset(cx);
                    }
                },
            )
            .detach();
    }

    /// Réponse à la confirmation de réinitialisation.
    pub fn answer_reset(&mut self, confirmed: bool, cx: &mut Context<Self>) {
        if self
            .take_modal(|modal| *modal == AppModal::ConfirmReset, cx)
            .is_none()
        {
            return;
        }
        if confirmed {
            self.reset(cx);
        } else {
            info!("reset annulé par l'utilisateur");
        }
    }

    /// Vide le relevé : moteur, compteurs, journal de trames, graphe.
    fn reset(&mut self, cx: &mut Context<Self>) {
        self.bridge
            .run(
                cx,
                |lunar| lunar.reset_capture(),
                |this, result, cx| match result {
                    Ok(()) => {
                        this.capture.reset_counters();
                        this.packet_log.clear();
                        this.channel = ChannelGauge::default();
                        this.graph.reset(cx);
                        info!("Relevé réinitialisé");
                        crate::app::memory::release_after_reset(cx);
                        cx.emit(AppEvent::Reset);
                        this.refresh_flags(cx);
                        this.changed(cx);
                    }
                    Err(err) => this.show_error(&err, cx),
                },
            )
            .detach();
    }

    // --- Fermeture -----------------------------------------------------

    /// Fermeture de la fenêtre demandée par le système.
    pub fn on_close_requested(&mut self, cx: &mut Context<Self>) {
        info!("close requested");
        self.request_exit(cx);
    }

    /// Fermeture demandée : confirmée si le relevé
    /// n'est pas enregistré.
    pub fn request_exit(&mut self, cx: &mut Context<Self>) {
        if self.exit_in_progress {
            debug!("Fermeture déjà en cours");
            return;
        }
        self.exit_in_progress = true;
        info!("Fermeture demandee");
        self.bridge
            .run(
                cx,
                |lunar| lunar.is_session_dirty(),
                |this, dirty, cx| {
                    if dirty.unwrap_or(true) {
                        this.push_modal(AppModal::ConfirmQuit, cx);
                    } else {
                        this.exit_now(cx);
                    }
                },
            )
            .detach();
    }

    /// Réponse à la confirmation de fermeture.
    pub fn answer_exit(&mut self, confirmed: bool, cx: &mut Context<Self>) {
        if self
            .take_modal(|modal| *modal == AppModal::ConfirmQuit, cx)
            .is_none()
        {
            return;
        }
        if confirmed {
            self.exit_now(cx);
        } else {
            info!("Fermeture annulee");
            self.exit_in_progress = false;
        }
    }

    /// Arrête la capture (au mieux) et l'autosave, puis quitte.
    fn exit_now(&mut self, cx: &mut Context<Self>) {
        info!("Fermeture confirmee");
        let channel = self.bridge.detached_channel();
        self.bridge
            .run(
                cx,
                move |lunar| {
                    if let Err(err) = lunar.stop_capture(channel) {
                        error!("Arrêt de la capture avant fermeture: {err}");
                    }
                    if let Err(err) = lunar.stop_autosave() {
                        error!("Arrêt de l'autosave avant fermeture: {err}");
                    }
                },
                |_, (), cx| cx.quit(),
            )
            .detach();
    }

    // --- Récupération --------------------------------------------------

    /// Propose de récupérer l'autosave d'une session interrompue.
    pub fn offer_recovery(&mut self, path: String, cx: &mut Context<Self>) {
        info!("Autosave d'une session interrompue : {path}");
        self.push_modal(AppModal::Recovery { path }, cx);
    }

    /// Réponse à la proposition de récupération : le projet
    /// est rouvert sur le canal du store, son instantané recharge le graphe.
    pub fn answer_recovery(&mut self, accept: bool, cx: &mut Context<Self>) {
        let Some(AppModal::Recovery { path }) =
            self.take_modal(|modal| matches!(modal, AppModal::Recovery { .. }), cx)
        else {
            return;
        };
        if !accept {
            info!("Récupération de l'autosave refusée");
            return;
        }
        let channel = crate::app::offline::project_channel(self.bridge.store_channel(), &path);
        self.run_locked(
            cx,
            move |lunar| lunar.open_project(path, channel),
            |this, result, cx| match result {
                Ok(()) => {
                    info!("Relevé récupéré depuis l'autosave");
                    this.refresh_flags(cx);
                    this.warn_if_partial(cx);
                }
                Err(err) => {
                    error!("Récupération impossible: {err}");
                    this.show_error(&err, cx);
                }
            },
        );
    }
}
