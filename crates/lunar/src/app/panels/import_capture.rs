//! Panneau d'import de captures et de matrices.
//!
//! Deux listes : captures réseau (`.pcap`, `.pcapng`,
//! `.cap`, converties par `convert_from_pcap_list`) et matrices de flux
//! (`.csv`, `.xlsx`, fusionnées par `import_matrix_files`). Les fichiers
//! s'ajoutent par le dialogue (verrou d'import tenu pendant qu'il est
//! ouvert) ou par glisser-déposer sur le panneau ou ailleurs dans la fenêtre
//! ([`forward_window_drop`]) ; une liste ne contient jamais deux fois le
//! même chemin. « Ouvrir » lance l'import sous le verrou global
//! ([`AppState::run_locked`]) : l'overlay de conversion suit la progression
//! du moteur (`ImportProgress`), chaque fichier affiche son statut et sa
//! durée, et l'import de captures s'annule (`cancel_import`). Le graphe se
//! recharge par l'instantané (`GraphSnapshot`) que le moteur émet à la fin
//! de l'import, sur le canal du store.
//!
//! Comportements :
//! - un fichier se retire de sa liste un par un ;
//! - après un import de captures annulé ou en échec, la liste reste affichée
//!   avec le statut de chaque fichier ;
//! - l'overlay passe au-dessus du verrou d'import de la fenêtre : le bouton
//!   d'annulation reste cliquable.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use gpui::{
    Animation, AnimationExt, AnyView, App, ClickEvent, Context, ElementId, Entity, ExternalPaths,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Transformation, WeakEntity, Window, deferred,
    div, percentage, prelude::FluentBuilder, px,
};
use log::info;
use lunar_engine::errors::CaptureStateError;
use lunar_engine::events::CaptureEvent;
use lunar_engine::ipc::Channel;
use lunar_ui_logic::errors::is_import_cancellation;
use lunar_ui_logic::file_types::{
    DroppedPathsRouting, ImportMode as DropMode, route_dropped_paths,
};
use lunar_ui_logic::import_files::{
    FileEvent, FileRun, ImportFiles, ImportKind, ImportRun, RunOutcome, file_event,
    file_status_text, progress_label,
};
use lunar_ui_logic::import_progress::progress_percent_of;
use lunar_ui_logic::save_paths::file_name_of;

use crate::app::file_dialog::{CAPTURE_FILTER, MATRIX_FILTER, pick_files};
use crate::app::panels::{ImportMode, Panel};
use crate::app::state::AppState;
use crate::assets::{Icon, icon};
use crate::theme::{ActiveTheme, Theme};
use crate::widgets::button::{Button, ButtonSize};
use crate::widgets::ellipsis;
use crate::widgets::panel::Panel as PanelFrame;
use crate::widgets::progress::ProgressBar;
use crate::widgets::tooltip::tooltip;

/// Ce panneau.
const PANEL: Panel = Panel::Import(ImportMode::Capture);

/// Indication affichée pendant un glisser-déposer.
pub const DROP_HINT: &str =
    "Déposez des captures (.pcap, .pcapng, .cap) ou des matrices (.csv, .xlsx)";

/// Titre du message d'annulation.
pub const CANCELLED_TITLE: &str = "Import annulé";

/// Message d'annulation : l'import est transactionnel.
pub const CANCELLED_MESSAGE: &str = "Import annulé. Le relevé courant est inchangé.";

/// Priorité de peinture de l'overlay : au-dessus du verrou d'import de la
/// fenêtre, sous les dialogues, menus et notifications.
const OVERLAY_PRIORITY: usize = 1;

/// Texte du bouton d'ajout de la liste `kind`.
fn add_label(kind: ImportKind) -> &'static str {
    match kind {
        ImportKind::Captures => "Ajouter des fichiers .pcap",
        ImportKind::Matrices => "Ajouter une ou plusieurs matrices (CSV ou XLSX)",
    }
}

/// Clé stable de la liste `kind` (identifiants d'éléments).
fn kind_key(kind: ImportKind) -> &'static str {
    match kind {
        ImportKind::Captures => "captures",
        ImportKind::Matrices => "matrices",
    }
}

/// Canal de l'import : chaque événement part vers le store (`store`) ; ceux
/// qui concernent un fichier sont aussi datés et copiés vers le suivi du
/// panneau.
fn tracking_channel(
    store: Channel<CaptureEvent>,
    files: UnboundedSender<FileEvent>,
) -> Channel<CaptureEvent> {
    Channel::new_fallible(move |event| {
        if let Some(file) = file_event(&event, Instant::now()) {
            // Panneau fermé entre-temps : seul le suivi est perdu.
            let _ = files.unbounded_send(file);
        }
        store.send(event)
    })
}

/// Panneau d'import de captures et de matrices.
pub struct CaptureImportPanel {
    /// État de l'application.
    state: WeakEntity<AppState>,
    /// Captures et matrices à importer.
    files: ImportFiles,
    /// Import en cours et sa nature.
    converting: Option<ImportKind>,
    /// Annulation demandée.
    cancel_requested: bool,
    /// Suivi par fichier du dernier import.
    run: Option<ImportRun>,
    /// Événements de fichier de l'import en cours.
    events: Option<UnboundedReceiver<FileEvent>>,
    /// Suivi de l'état (progression de l'import).
    _subscriptions: Vec<Subscription>,
}

impl CaptureImportPanel {
    /// Panneau vide sur `state`.
    pub fn new(state: &Entity<AppState>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            state: state.downgrade(),
            files: ImportFiles::new(),
            converting: None,
            cancel_requested: false,
            run: None,
            events: None,
            _subscriptions: vec![cx.observe(state, |_, _, cx| cx.notify())],
        }
    }

    /// Fichiers listés (essais, diagnostic).
    pub fn files(&self) -> &ImportFiles {
        &self.files
    }

    /// Ferme le panneau, sauf pendant un import (bouton ❌ grisé).
    fn close(&mut self, cx: &mut Context<Self>) {
        if self.converting.is_some() {
            return;
        }
        let _ = self
            .state
            .update(cx, |state, cx| state.close_panel(PANEL, cx));
    }

    /// Les listes ont changé : le suivi du dernier import ne vaut plus.
    fn files_changed(&mut self, cx: &mut Context<Self>) {
        self.run = None;
        cx.notify();
    }

    /// Ajoute des fichiers par le dialogue, sous le verrou
    /// d'import.
    fn add_files(&mut self, kind: ImportKind, cx: &mut Context<Self>) {
        if self.converting.is_some() {
            return;
        }
        let locked = self
            .state
            .update(cx, |state, cx| state.hold_import_lock(cx))
            .unwrap_or(false);
        if !locked {
            return;
        }
        let filter = match kind {
            ImportKind::Captures => CAPTURE_FILTER,
            ImportKind::Matrices => MATRIX_FILTER,
        };
        let dialog = pick_files(None, filter, true, None);
        let state = self.state.clone();
        cx.spawn(async move |this, cx| {
            let picked = dialog.await;
            // Verrou rendu même si le panneau a disparu entre-temps.
            let _ = state.update(cx, |state, cx| state.release_import_lock(cx));
            if let Some(paths) = picked {
                let _ = this.update(cx, |this, cx| {
                    if this.files.add(kind, &paths) {
                        this.files_changed(cx);
                    }
                });
            }
        })
        .detach();
    }

    /// Fichiers déposés sur le panneau ou la fenêtre.
    pub fn handle_dropped_paths(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        if self.converting.is_some() {
            return;
        }
        let paths: Vec<String> = paths
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect();
        let routing = route_dropped_paths(&paths, DropMode::Pcap);
        if routing == DroppedPathsRouting::Ignored {
            info!("drop ignoré : ni capture réseau ni matrice CSV/XLSX");
            return;
        }
        if self.files.add_dropped(&routing) {
            self.files_changed(cx);
        }
    }

    /// Retire un fichier de sa liste.
    fn remove(&mut self, kind: ImportKind, index: usize, cx: &mut Context<Self>) {
        if self.converting.is_none() && self.files.remove(kind, index).is_some() {
            self.files_changed(cx);
        }
    }

    /// Vide une liste (« Effacer »).
    fn clear(&mut self, kind: ImportKind, cx: &mut Context<Self>) {
        if self.converting.is_none() {
            self.files.clear(kind);
            self.files_changed(cx);
        }
    }

    /// Lance l'import de la liste `kind`.
    fn start_import(&mut self, kind: ImportKind, cx: &mut Context<Self>) {
        if self.converting.is_some() {
            return;
        }
        let paths = self.files.list(kind).to_vec();
        if paths.is_empty() {
            return;
        }
        let Some(state) = self.state.upgrade() else {
            return;
        };
        let (sender, receiver) = unbounded();
        let panel = cx.entity().downgrade();
        let started_at = Instant::now();
        let work_paths = paths.clone();
        let started = state.update(cx, |state, cx| {
            // Canal neuf par import. Pendant une capture live (matrices
            // seulement), le moteur passe par le canal de la capture : le
            // canal du store n'est pas remplacé.
            let store = if kind == ImportKind::Matrices && state.capture().is_running {
                state.bridge().detached_channel()
            } else {
                state.bridge().store_channel()
            };
            let channel = tracking_channel(store, sender);
            match kind {
                ImportKind::Captures => {
                    info!("convert_from_pcap_list : {}", work_paths.join(","));
                }
                ImportKind::Matrices => {
                    info!("import_matrix_files: {}", work_paths.join(", "));
                }
            }
            state.update_capture(|capture| capture.clear_import_progress(), cx);
            state.run_locked(
                cx,
                move |lunar| match kind {
                    ImportKind::Captures => lunar.convert_from_pcap_list(work_paths, channel),
                    ImportKind::Matrices => lunar.import_matrix_files(work_paths, channel),
                },
                move |state, result, cx| import_done(state, panel, kind, result, cx),
            )
        });
        if started {
            self.converting = Some(kind);
            self.cancel_requested = false;
            self.run = Some(ImportRun::start(kind, &paths, started_at));
            self.events = Some(receiver);
            cx.notify();
        }
    }

    /// Demande l'annulation coopérative de l'import de captures
    /// : le moteur draine le fichier courant sans
    /// l'analyser et n'ouvre pas les suivants.
    fn cancel_import(&mut self, cx: &mut Context<Self>) {
        if self.converting != Some(ImportKind::Captures) || self.cancel_requested {
            return;
        }
        let Some(state) = self.state.upgrade() else {
            return;
        };
        self.cancel_requested = true;
        cx.notify();
        let bridge = state.read(cx).bridge().clone();
        bridge
            .run(
                cx,
                |lunar| lunar.cancel_import(),
                |this, result, cx| {
                    match result {
                        Ok(true) => {}
                        Ok(false) => {
                            // Course avec la fin d'import : son issue normale
                            // reprend la main.
                            info!("cancel_import : aucun import actif, demande ignorée");
                            this.cancel_requested = false;
                        }
                        Err(err) => {
                            this.cancel_requested = false;
                            let _ = this
                                .state
                                .update(cx, |state, cx| state.show_error(&err, cx));
                        }
                    }
                    cx.notify();
                },
            )
            .detach();
    }

    /// Reporte les événements de fichier reçus dans le suivi.
    fn drain_events(&mut self) {
        let (Some(run), Some(events)) = (self.run.as_mut(), self.events.as_mut()) else {
            return;
        };
        while let Ok(event) = events.try_recv() {
            run.apply(&event);
        }
    }

    /// Issue de l'import, le verrou déjà rendu.
    fn import_finished(&mut self, outcome: RunOutcome, cx: &mut Context<Self>) {
        self.drain_events();
        let now = Instant::now();
        if let Some(run) = self.run.as_mut() {
            run.finish(outcome, now);
            for file in run.files() {
                let status = file_status_text(file, false, now);
                if !status.is_empty() {
                    info!("[Import] {} : {status}", file.path);
                }
            }
        }
        self.events = None;
        self.cancel_requested = false;
        if outcome == RunOutcome::Success
            && let Some(kind) = self.converting
        {
            self.files.clear(kind);
        }
        self.converting = None;
        cx.notify();
    }

    /// Suivi du fichier `path` dans le dernier import de la liste `kind`.
    fn file_run(&self, kind: ImportKind, index: usize) -> Option<&FileRun> {
        self.run
            .as_ref()
            .filter(|run| run.kind() == kind)
            .and_then(|run| run.files().get(index))
    }

    /// Ligne d'un fichier : nom, dossier, statut, bouton de retrait.
    fn render_file_row(
        &self,
        kind: ImportKind,
        index: usize,
        path: &str,
        theme: &Theme,
        now: Instant,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let busy = self.converting.is_some();
        let status = self
            .file_run(kind, index)
            .map(|file| file_status_text(file, busy, now))
            .unwrap_or_default();
        let name = file_name_of(path).to_owned();
        let directory = path
            .strip_suffix(name.as_str())
            .unwrap_or_default()
            .trim_end_matches(['/', '\\'])
            .to_owned();
        let row_id = ElementId::Name(format!("import-{}-{index}", kind_key(kind)).into());
        div()
            .id(row_id)
            .flex()
            .items_center()
            .gap(theme.space_md)
            .px(theme.space_md)
            .py(theme.space_sm)
            .rounded(theme.radius_sm)
            .bg(theme.bg_field)
            .tooltip(tooltip(path.to_owned()))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(ellipsis(
                        div()
                            .font_family(theme.font_mono.clone())
                            .text_size(theme.text_sm)
                            .child(name),
                    ))
                    .when(!directory.is_empty(), |d| {
                        d.child(ellipsis(
                            div()
                                .text_size(theme.text_xs)
                                .text_color(theme.text_muted)
                                .child(directory),
                        ))
                    }),
            )
            .when(!status.is_empty(), |d| {
                d.child(
                    div()
                        .flex_none()
                        .text_size(theme.text_xs)
                        .text_color(theme.text_muted)
                        .child(status),
                )
            })
            .child(
                Button::icon_only(
                    ElementId::Name(format!("retirer-{}-{index}", kind_key(kind)).into()),
                    Icon::Close,
                    "Retirer de la liste",
                )
                .ghost()
                .size(ButtonSize::Small)
                .disabled(busy)
                .on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| this.remove(kind, index, cx)),
                ),
            )
    }

    /// Section d'une liste : ajout, effacement, fichiers, ouverture.
    fn render_section(
        &self,
        kind: ImportKind,
        theme: &Theme,
        now: Instant,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let busy = self.converting.is_some();
        let files = self.files.list(kind).to_vec();
        let key = kind_key(kind);
        let add_icon = match kind {
            ImportKind::Captures => Icon::FileImport,
            ImportKind::Matrices => Icon::FileSpreadsheet,
        };
        let rows: Vec<_> = files
            .iter()
            .enumerate()
            .map(|(index, path)| self.render_file_row(kind, index, path, theme, now, cx))
            .collect();
        div()
            .flex()
            .flex_col()
            .gap(theme.space_md)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_center()
                    .gap(theme.space_md)
                    .child(
                        Button::new(
                            ElementId::Name(format!("ajouter-{key}").into()),
                            add_label(kind),
                        )
                        .icon(add_icon)
                        .disabled(busy)
                        .on_click(
                            cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.add_files(kind, cx)
                            }),
                        ),
                    )
                    .when(!files.is_empty(), |d| {
                        d.child(
                            Button::new(
                                ElementId::Name(format!("effacer-{key}").into()),
                                "Effacer",
                            )
                            .danger()
                            .disabled(busy)
                            .on_click(
                                cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    this.clear(kind, cx)
                                }),
                            ),
                        )
                    }),
            )
            .when(!files.is_empty(), |d| {
                d.child(
                    div()
                        .id(ElementId::Name(format!("liste-{key}").into()))
                        .flex()
                        .flex_col()
                        .gap(theme.space_xs)
                        .max_h(px(250.))
                        .overflow_y_scroll()
                        .children(rows),
                )
                .child(
                    div().flex().justify_center().child(
                        Button::new(ElementId::Name(format!("ouvrir-{key}").into()), "Ouvrir")
                            .primary()
                            .disabled(busy)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.start_import(kind, cx)
                            })),
                    ),
                )
            })
    }

    /// Overlay de conversion : roue, progression, fichiers, annulation.
    fn render_overlay(
        &self,
        theme: &Theme,
        now: Instant,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let progress = self
            .state
            .upgrade()
            .and_then(|state| state.read(cx).capture().import_progress.clone());
        let files: Vec<(String, String)> = self
            .run
            .as_ref()
            .map(|run| {
                run.files()
                    .iter()
                    .map(|file| {
                        (
                            file_name_of(&file.path).to_owned(),
                            file_status_text(file, true, now),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let cancellable = self.converting == Some(ImportKind::Captures);
        let cancel_label = if self.cancel_requested {
            "Annulation…"
        } else {
            "Annuler l'import"
        };
        div()
            .id("import-conversion")
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(theme.space_lg)
            .p(theme.space_xl)
            .bg(gpui::rgba(0x0000_00cc))
            .child(
                icon(Icon::Refresh)
                    .size(px(40.))
                    .text_color(gpui::rgb(0x42_99_e1))
                    .with_animation(
                        "import-roue",
                        Animation::new(Duration::from_secs(1)).repeat(),
                        |svg, delta| {
                            svg.with_transformation(Transformation::rotate(percentage(delta)))
                        },
                    ),
            )
            .child(
                div()
                    .text_size(theme.text_lg)
                    .text_color(gpui::white())
                    .child("Conversion en cours…"),
            )
            .when_some(progress, |d, progress| {
                let percent = progress_percent_of(Some(&progress));
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(theme.space_sm)
                        .w(px(380.))
                        .max_w_full()
                        .child(ProgressBar::determinate(
                            "import-progression",
                            (percent / 100.) as f32,
                        ))
                        .child(ellipsis(
                            div()
                                .text_center()
                                .font_family(theme.font_mono.clone())
                                .text_size(theme.text_sm)
                                .text_color(gpui::rgba(0xffff_ffd9))
                                .child(progress_label(&progress)),
                        )),
                )
            })
            .when(files.len() > 1, |d| {
                d.child(
                    div()
                        .id("import-conversion-fichiers")
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .w(px(380.))
                        .max_w_full()
                        .max_h(px(160.))
                        .overflow_y_scroll()
                        .text_size(theme.text_xs)
                        .children(files.into_iter().map(|(name, status)| {
                            div()
                                .flex()
                                .gap(theme.space_md)
                                .child(ellipsis(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .font_family(theme.font_mono.clone())
                                        .child(name),
                                ))
                                .child(div().flex_none().text_color(theme.text_muted).child(status))
                        })),
                )
            })
            .when(cancellable, |d| {
                d.child(
                    Button::new("annuler-import", cancel_label)
                        .danger()
                        .disabled(self.cancel_requested)
                        .on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| this.cancel_import(cx)),
                        ),
                )
            })
    }
}

/// Fin d'un import lancé par le panneau, dans l'état de l'application, le
/// verrou déjà rendu : messages, suivi du panneau, fermeture après une
/// réussite, puis rafraîchissement de la présence de données, dans cet
/// ordre.
fn import_done(
    state: &mut AppState,
    panel: WeakEntity<CaptureImportPanel>,
    kind: ImportKind,
    result: Result<(), CaptureStateError>,
    cx: &mut Context<AppState>,
) {
    state.update_capture(|capture| capture.clear_import_progress(), cx);
    let outcome = match &result {
        Ok(()) => {
            info!("réponse invoke");
            RunOutcome::Success
        }
        Err(err) if kind == ImportKind::Captures && is_import_cancellation(err) => {
            // Issue normale, notifiée comme telle : l'import est
            // transactionnel, le relevé courant est intact.
            info!("import PCAP annulé par l'opérateur ; relevé courant inchangé");
            state.show_message(CANCELLED_TITLE, CANCELLED_MESSAGE, cx);
            RunOutcome::Cancelled
        }
        Err(err) => {
            state.show_error(err, cx);
            RunOutcome::Failed
        }
    };
    let _ = panel.update(cx, |panel, cx| panel.import_finished(outcome, cx));
    if outcome == RunOutcome::Success {
        state.close_panel(PANEL, cx);
        if kind == ImportKind::Matrices {
            state.warn_if_partial(cx);
        }
    }
    state.refresh_flags(cx);
}

/// Dépôt de fichiers ailleurs que sur le panneau : tant que le panneau est
/// ouvert, la fenêtre entière accepte le
/// dépôt. Rend vrai si un panneau d'import de captures l'a reçu.
pub fn forward_window_drop(
    views: &[(Panel, AnyView)],
    paths: &ExternalPaths,
    cx: &mut App,
) -> bool {
    let Some(view) = views
        .iter()
        .filter(|(panel, _)| *panel == PANEL)
        .find_map(|(_, view)| view.clone().downcast::<CaptureImportPanel>().ok())
    else {
        return false;
    };
    view.update(cx, |panel, cx| {
        panel.handle_dropped_paths(paths.paths(), cx)
    });
    true
}

impl Render for CaptureImportPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.drain_events();
        let theme = cx.theme().clone();
        let now = Instant::now();
        let busy = self.converting.is_some();
        let body = div()
            .flex()
            .flex_col()
            .gap(theme.space_lg)
            .child(self.render_section(ImportKind::Captures, &theme, now, cx))
            .child(div().h(px(1.)).mx(theme.space_md).bg(gpui::rgba(0xf5f5_f54d)))
            .child(self.render_section(ImportKind::Matrices, &theme, now, cx))
            .child(
                div()
                    .text_size(theme.text_xs)
                    .text_color(theme.text_muted)
                    .child(SharedString::from(
                        "Glisser-déposer : captures (.pcap, .pcapng, .cap) et matrices (.csv, .xlsx), \
                         n'importe où dans la fenêtre.",
                    )),
            );
        let frame = PanelFrame::new("panneau-import-captures", PANEL.title())
            .width(PANEL.width())
            .on_close(cx.listener(|this, _: &ClickEvent, _, cx| this.close(cx)))
            .child(body);
        let hint = div()
            .absolute()
            .inset_0()
            .invisible()
            .group_drag_over::<ExternalPaths>("import-depot", |style| style.visible())
            // gpui 0.2.2 n'applique les styles de survol de dépôt qu'aux
            // éléments qui ont leur propre hitbox ; un écouteur vide en crée
            // une, sans bloquer la souris pour le panneau en dessous.
            .on_mouse_move(|_, _, _| {})
            .flex()
            .items_center()
            .justify_center()
            .p(theme.space_xl)
            .bg(gpui::rgba(0x1e1e_2eeb))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(theme.space_md)
                    .p(theme.space_xl)
                    .rounded(theme.radius_lg)
                    .border_2()
                    .border_dashed()
                    .border_color(gpui::rgb(0x25_96_be))
                    .child(icon(Icon::Download).size(px(40.)).text_color(theme.text))
                    .child(div().text_center().child(DROP_HINT)),
            );
        div()
            .id("import-captures")
            .group("import-depot")
            .relative()
            .flex_none()
            .h_full()
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.handle_dropped_paths(paths.paths(), cx)
            }))
            .child(frame)
            .child(hint)
            .when(busy, |d| {
                d.child(
                    deferred(self.render_overlay(&theme, now, cx)).with_priority(OVERLAY_PRIORITY),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn le_canal_suivi_copie_les_evenements_de_fichier() {
        let received: Arc<Mutex<Vec<CaptureEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&received);
        let store = Channel::new(move |event| {
            if let Ok(mut events) = sink.lock() {
                events.push(event);
            }
        });
        let (sender, mut receiver) = unbounded();
        let channel = tracking_channel(store, sender);
        channel
            .send(CaptureEvent::ImportProgress {
                file_name: "/a.pcap".into(),
                file_index: 1,
                files_total: 1,
                current: 1,
                total: 2,
            })
            .expect("envoi");
        channel
            .send(CaptureEvent::Stopped {
                session_id: 0,
                reason: "fin".into(),
            })
            .expect("envoi");
        assert_eq!(
            received.lock().expect("verrou").len(),
            2,
            "tout va au store"
        );
        assert!(matches!(
            receiver.try_recv(),
            Ok(FileEvent::Progress { file_index: 1, .. })
        ));
        assert!(
            receiver.try_recv().is_err(),
            "seuls les événements de fichier"
        );
    }

    #[test]
    fn libelles_des_listes() {
        assert_eq!(
            add_label(ImportKind::Captures),
            "Ajouter des fichiers .pcap"
        );
        assert_ne!(
            kind_key(ImportKind::Captures),
            kind_key(ImportKind::Matrices)
        );
    }
}
