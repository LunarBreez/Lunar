//! Panneau d'import en mode labels :
//! fichier de labels CSV (remplace le store), relevés WinAudit (s'ajoutent
//! au store), bilan du dernier import WinAudit, accès à l'arbitrage, table du
//! contenu importé avec recherche et remise à zéro, dépôt de fichier.
//!
//! Une erreur bloquante du fichier (MAC/IP invalides, lignes mal formées)
//! ouvre le dialogue des conflits ; des doublons de clé ouvrent l'arbitrage.

use gpui::{
    App, AppContext, ClickEvent, Context, Entity, ExternalPaths, FocusHandle, Focusable,
    InteractiveElement, IntoElement, ParentElement, Render, StatefulInteractiveElement, Styled,
    Subscription, Window, div, prelude::FluentBuilder, px,
};
use log::{error, info};
use lunar_engine::Lunar;
use lunar_engine::errors::CaptureStateError;
use lunar_ui_logic::file_types::{
    DroppedPathsRouting, ImportMode as DropMode, route_dropped_paths,
};
use lunar_ui_logic::label_import::{
    FORMAT_HINT, LABEL_FILE_FILTER, WINAUDIT_HINT, arbitrate_button, classify_label_import_error,
    import_report_log, import_start_log, skipped_note,
};
use lunar_ui_logic::label_management::SEARCH_PLACEHOLDER;
use lunar_ui_logic::winaudit_import::{
    DEFAULT_EXCLUDE_VIRTUAL, EXCLUDE_VIRTUAL_LABEL, import_start_log as winaudit_start_log,
    summarize_winaudit_import,
};

use crate::app::labels::rows::TripletRows;
use crate::app::labels::session::{LabelSession, LabelSessionEvent};
use crate::app::labels::{
    apply_label_updates, dialogs, import_channel, release_import_lock, take_import_lock,
};
use crate::app::panels::{ImportMode, Panel};
use crate::app::state::AppState;
use crate::assets::Icon;
use crate::theme::ActiveTheme;
use crate::widgets::button::Button;
use crate::widgets::checkbox::Checkbox;
use crate::widgets::modal::Confirm;
use crate::widgets::panel::Panel as PanelFrame;
use crate::widgets::table::Table;
use crate::widgets::text_input::{TextInput, TextInputEvent};
use crate::widgets::{Cancel, MODAL_CONTEXT};

/// Ce panneau.
const PANEL: Panel = Panel::Import(ImportMode::Labels);

/// Question de la remise à zéro du contenu importé.
pub const CLEAR_QUESTION: &str =
    "Vider le référentiel de labels ?\nTous les équipements seront désétiquetés.";

/// Panneau d'import de labels.
pub struct LabelImportPanel {
    /// État de l'application.
    state: Entity<AppState>,
    /// Données partagées avec les dialogues imbriqués.
    session: Entity<LabelSession>,
    /// Focus du panneau (Échap).
    focus: FocusHandle,
    /// Recherche dans le contenu importé.
    search: Entity<TextInput>,
    /// Contenu importé.
    table: Entity<Table<TripletRows>>,
    /// Import en cours.
    converting: bool,
    /// Écarter les adaptateurs virtuels des relevés WinAudit.
    exclude_virtual: bool,
    /// Bilan du dernier import WinAudit.
    winaudit_summary: Vec<String>,
    /// Conflits en attente d'arbitrage.
    pending_conflicts: usize,
    /// Lignes sans label du dernier import, ignorées.
    skipped_without_label: usize,
    /// Confirmation de la remise à zéro affichée.
    confirm_clear: bool,
    /// Abonnements (recherche, session).
    _subscriptions: Vec<Subscription>,
}

impl Focusable for LabelImportPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl LabelImportPanel {
    /// Panneau ouvert : focus posé, conflits en attente comptés.
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let session = LabelSession::global(cx);
        let search = cx.new(|cx| TextInput::new(cx).placeholder(SEARCH_PLACEHOLDER));
        let table = cx.new(|cx| Table::new(TripletRows::new(), cx).empty_text("Aucun résultat"));
        let subscriptions = vec![
            cx.subscribe(&search, |this, _, event: &TextInputEvent, cx| {
                if let TextInputEvent::Changed(text) = event {
                    this.table.update(cx, |table, cx| {
                        table.delegate_mut().set_search(text);
                        table.refresh(cx);
                    });
                }
            }),
            cx.subscribe(
                &session,
                |this, _, event: &LabelSessionEvent, cx| match event {
                    LabelSessionEvent::Resolved => this.on_arbitration_resolved(cx),
                },
            ),
        ];
        let focus = cx.focus_handle();
        window.focus(&focus);
        let mut this = Self {
            state,
            session,
            focus,
            search,
            table,
            converting: false,
            exclude_virtual: DEFAULT_EXCLUDE_VIRTUAL,
            winaudit_summary: Vec::new(),
            pending_conflicts: 0,
            skipped_without_label: 0,
            confirm_clear: false,
            _subscriptions: subscriptions,
        };
        this.count_pending_conflicts(cx);
        this
    }

    /// Conflits d'un import précédent en attente (au montage).
    fn count_pending_conflicts(&mut self, cx: &mut Context<Self>) {
        let bridge = self.state.read(cx).bridge().clone();
        bridge
            .run(
                cx,
                |lunar| lunar.get_label_conflicts(),
                |this, conflicts, cx| {
                    if let Ok(conflicts) = conflicts {
                        this.pending_conflicts = conflicts.len();
                        cx.notify();
                    }
                },
            )
            .detach();
    }

    /// Ferme le panneau (refusé pendant un import).
    fn close(&mut self, cx: &mut Context<Self>) {
        if self.converting {
            return;
        }
        self.state
            .update(cx, |state, cx| state.close_panel(PANEL, cx));
    }

    /// Remplace les lignes de la table.
    fn set_rows(&mut self, rows: Vec<(String, String, String)>, cx: &mut Context<Self>) {
        self.table.update(cx, |table, cx| {
            table.delegate_mut().set_rows(rows);
            table.refresh(cx);
        });
    }

    /// Exécute un import sous le verrou déjà pris : `work` en fond, puis le
    /// verrou rendu (même si le panneau a disparu), puis `done`. L'interface
    /// est libérée avant le rafraîchissement de la table : un échec de
    /// lecture ne bloque pas le panneau.
    fn run_import<R: Send + 'static>(
        &mut self,
        cx: &mut Context<Self>,
        work: impl FnOnce(&Lunar) -> R + Send + 'static,
        done: impl FnOnce(&mut Self, R, &mut Context<Self>) + 'static,
    ) {
        self.converting = true;
        cx.notify();
        let state = self.state.clone();
        let bridge = state.read(cx).bridge().clone();
        let task = bridge.call(cx, work);
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = cx.update(|cx| release_import_lock(&state, cx));
            let _ = this.update(cx, |this, cx| {
                this.converting = false;
                done(this, result, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// « Ajouter un fichier » : un seul fichier de labels, qui remplace le
    /// store.
    fn add_csv_file(&mut self, cx: &mut Context<Self>) {
        if self.converting || !take_import_lock(&self.state, cx) {
            return;
        }
        let dialog = dialogs::pick_label_file(None, LABEL_FILE_FILTER);
        let state = self.state.clone();
        cx.spawn(async move |this, cx| {
            let path = dialog.await;
            let handled = this.update(cx, |this, cx| match path {
                Some(path) => this.import_label_file(path, cx),
                None => release_import_lock(&this.state, cx),
            });
            if handled.is_err() {
                let _ = cx.update(|cx| release_import_lock(&state, cx));
            }
        })
        .detach();
    }

    /// Importe un fichier de labels, verrou tenu.
    fn import_label_file(&mut self, path: String, cx: &mut Context<Self>) {
        if path.is_empty() {
            release_import_lock(&self.state, cx);
            return;
        }
        info!("{}", import_start_log(&path));
        self.session
            .update(cx, |session, _| session.set_issues(None));
        let channel = import_channel(&self.state, cx);
        self.run_import(
            cx,
            move |lunar| {
                // Les conflits ne bloquent pas : le moteur garde le premier
                // label par clé (mac, ip) et rend les doublons à arbitrer.
                let report = lunar.import_label_file(path, channel);
                (report, lunar.get_label_rows())
            },
            |this, (report, rows), cx| {
                match report {
                    Ok(report) => {
                        info!("{}", import_report_log(&report));
                        this.skipped_without_label = report.skipped_without_label;
                        this.pending_conflicts = report.conflicts.len();
                        if !report.conflicts.is_empty() {
                            this.open_arbitration_with(report.conflicts, cx);
                        }
                    }
                    Err(err) => this.show_label_file_issues(&err, cx),
                }
                this.after_refresh(rows, cx);
                this.state.update(cx, |state, cx| state.refresh_flags(cx));
            },
        );
    }

    /// Relevés WinAudit, un lot à la fois.
    fn add_winaudit_surveys(&mut self, cx: &mut Context<Self>) {
        if self.converting || !take_import_lock(&self.state, cx) {
            return;
        }
        let dialog = dialogs::pick_winaudit_surveys();
        let state = self.state.clone();
        cx.spawn(async move |this, cx| {
            let paths = dialog.await;
            let handled = this.update(cx, |this, cx| this.import_winaudit_surveys(paths, cx));
            if handled.is_err() {
                let _ = cx.update(|cx| release_import_lock(&state, cx));
            }
        })
        .detach();
    }

    /// Importe des relevés WinAudit, verrou tenu : les labels s'ajoutent à
    /// ceux en place, un désaccord part à l'arbitrage, ouvert aussitôt.
    fn import_winaudit_surveys(&mut self, paths: Vec<String>, cx: &mut Context<Self>) {
        if paths.is_empty() {
            release_import_lock(&self.state, cx);
            return;
        }
        info!("{}", winaudit_start_log(paths.len()));
        self.winaudit_summary.clear();
        let exclude_virtual = self.exclude_virtual;
        let channel = import_channel(&self.state, cx);
        self.run_import(
            cx,
            move |lunar| {
                let report = lunar.import_winaudit_surveys(paths, exclude_virtual, channel);
                (report, lunar.get_label_rows())
            },
            |this, (report, rows), cx| {
                match report {
                    Ok(report) => {
                        this.winaudit_summary = summarize_winaudit_import(&report);
                        this.pending_conflicts = report.conflicts.len();
                        if !report.conflicts.is_empty() {
                            this.open_arbitration_with(report.conflicts, cx);
                        }
                    }
                    Err(err) => this
                        .state
                        .update(cx, |state, cx| state.show_error(&err, cx)),
                }
                this.after_refresh(rows, cx);
                this.state.update(cx, |state, cx| state.refresh_flags(cx));
            },
        );
    }

    /// Lignes relues après un import ou une remise à zéro.
    fn after_refresh(
        &mut self,
        rows: Result<Vec<(String, String, String)>, CaptureStateError>,
        cx: &mut Context<Self>,
    ) {
        match rows {
            Ok(rows) => self.set_rows(rows, cx),
            Err(err) => self
                .state
                .update(cx, |state, cx| state.show_error(&err, cx)),
        }
    }

    /// Erreur bloquante d'un import de labels : dialogue des conflits avec
    /// le détail, ou message générique.
    fn show_label_file_issues(&mut self, err: &CaptureStateError, cx: &mut Context<Self>) {
        match classify_label_import_error(err) {
            Some(issues) => {
                error!("Import de labels refusé : {err}");
                self.session
                    .update(cx, |session, _| session.set_issues(Some(issues)));
                self.state
                    .update(cx, |state, cx| state.open_dialog(Panel::Conflicts, cx));
            }
            None => self.state.update(cx, |state, cx| state.show_error(err, cx)),
        }
    }

    /// Dépose `conflicts` pour l'arbitrage et l'ouvre.
    fn open_arbitration_with(
        &mut self,
        conflicts: Vec<lunar_engine::commandes::import::labels::conflicts::LabelConflictReport>,
        cx: &mut Context<Self>,
    ) {
        self.session
            .update(cx, |session, _| session.set_arbitration(conflicts));
        self.state
            .update(cx, |state, cx| state.open_dialog(Panel::Arbitration, cx));
    }

    /// « Arbitrer les conflits » : conflits en attente relus puis arbitrage.
    fn open_arbitration(&mut self, cx: &mut Context<Self>) {
        if self.converting {
            return;
        }
        let bridge = self.state.read(cx).bridge().clone();
        bridge
            .run(
                cx,
                |lunar| lunar.get_label_conflicts(),
                |this, conflicts, cx| match conflicts {
                    Ok(conflicts) => {
                        this.pending_conflicts = conflicts.len();
                        if !conflicts.is_empty() {
                            this.open_arbitration_with(conflicts, cx);
                        }
                        cx.notify();
                    }
                    Err(err) => this
                        .state
                        .update(cx, |state, cx| state.show_error(&err, cx)),
                },
            )
            .detach();
    }

    /// Arbitrage appliqué : plus rien en attente, table relue.
    fn on_arbitration_resolved(&mut self, cx: &mut Context<Self>) {
        self.pending_conflicts = 0;
        let bridge = self.state.read(cx).bridge().clone();
        bridge
            .run(
                cx,
                |lunar| lunar.get_label_rows(),
                |this, rows, cx| this.after_refresh(rows, cx),
            )
            .detach();
        cx.notify();
    }

    /// Remise à zéro confirmée : store vidé, graphe désétiqueté, table relue.
    fn clear_label_store(&mut self, cx: &mut Context<Self>) {
        self.confirm_clear = false;
        let bridge = self.state.read(cx).bridge().clone();
        bridge
            .run(
                cx,
                |lunar| (lunar.clear_label_store(), lunar.get_label_rows()),
                |this, (cleared, rows), cx| {
                    match cleared {
                        Ok(updates) => {
                            info!("Référentiel de labels vidé");
                            apply_label_updates(&this.state, updates, cx);
                        }
                        Err(err) => this
                            .state
                            .update(cx, |state, cx| state.show_error(&err, cx)),
                    }
                    this.after_refresh(rows, cx);
                    cx.notify();
                },
            )
            .detach();
        cx.notify();
    }

    /// Fichiers déposés sur le panneau : le premier `.csv` ou `.xlsx` est
    /// importé.
    fn on_drop(&mut self, paths: &ExternalPaths, cx: &mut Context<Self>) {
        if self.converting {
            return;
        }
        let paths: Vec<String> = paths
            .paths()
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect();
        match route_dropped_paths(&paths, DropMode::Csv) {
            DroppedPathsRouting::Csv { path } => {
                if take_import_lock(&self.state, cx) {
                    self.import_label_file(path, cx);
                }
            }
            DroppedPathsRouting::Files { .. } | DroppedPathsRouting::Ignored => {
                info!("drop ignoré : aucun fichier .csv ou .xlsx");
            }
        }
    }

    /// Boutons d'import, option WinAudit, bilan, arbitrage.
    fn render_actions(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme().clone();
        let converting = self.converting;
        div()
            .flex()
            .flex_col()
            .gap(theme.space_md)
            .child(
                Button::new("labels-ajouter-fichier", "Ajouter un fichier")
                    .icon(Icon::FileSpreadsheet)
                    .full_width()
                    .disabled(converting)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.add_csv_file(cx))),
            )
            .child(
                Button::new("labels-winaudit", "Importer des relevés WinAudit")
                    .icon(Icon::FileImport)
                    .full_width()
                    .disabled(converting)
                    .on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| this.add_winaudit_surveys(cx)),
                    ),
            )
            .child(
                // Case nue et libellé à part : le libellé, long, revient à la
                // ligne dans la largeur du panneau.
                div()
                    .flex()
                    .items_start()
                    .gap(theme.space_sm)
                    .w_full()
                    .text_size(theme.text_sm)
                    .child(
                        Checkbox::bare("labels-ecarter-virtuels", self.exclude_virtual)
                            .disabled(converting)
                            .on_change(cx.listener(|this, checked: &bool, _, cx| {
                                this.exclude_virtual = *checked;
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .id("labels-ecarter-virtuels-libelle")
                            .flex_1()
                            .min_w_0()
                            .pt(px(2.))
                            .when(!converting, |d| d.cursor_pointer())
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                if !this.converting {
                                    this.exclude_virtual = !this.exclude_virtual;
                                    cx.notify();
                                }
                            }))
                            .child(EXCLUDE_VIRTUAL_LABEL),
                    ),
            )
            .when(!self.winaudit_summary.is_empty(), |d| {
                d.child(
                    div()
                        .id("labels-bilan-winaudit")
                        .flex()
                        .flex_col()
                        .gap(theme.space_xs)
                        .p(theme.space_md)
                        .rounded(theme.radius_md)
                        .bg(theme.bg_raised)
                        .text_size(theme.text_sm)
                        .children(self.winaudit_summary.iter().map(|line| {
                            div()
                                .flex()
                                .gap(theme.space_sm)
                                .child(div().flex_none().child("•"))
                                .child(div().flex_1().min_w_0().child(line.clone()))
                        })),
                )
            })
            .when(self.pending_conflicts > 0, |d| {
                d.child(
                    Button::new("labels-arbitrer", arbitrate_button(self.pending_conflicts))
                        .icon(Icon::Scale)
                        .full_width()
                        .disabled(converting)
                        .on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| this.open_arbitration(cx)),
                        ),
                )
            })
    }

    /// Contenu importé : titre, recherche, remise à zéro et table.
    fn render_content(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme().clone();
        let total = self.table.read(cx).delegate().total();
        if total == 0 {
            return div()
                .text_color(theme.text_muted)
                .child("Aucun label importé pour le moment")
                .into_any_element();
        }
        let title = div()
            .flex()
            .items_baseline()
            .gap(theme.space_xs)
            .child(
                div()
                    .text_size(theme.text_lg)
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(theme.text_heading)
                    .child("Contenu importé"),
            )
            .children(skipped_note(self.skipped_without_label).map(|note| {
                div()
                    .text_size(theme.text_sm)
                    .text_color(theme.text_muted)
                    .child(note)
            }));
        div()
            .flex()
            .flex_col()
            .gap(theme.space_md)
            .child(title)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(theme.space_sm)
                    .child(div().flex_1().min_w_0().child(self.search.clone()))
                    .child(
                        Button::icon_only(
                            "labels-reinitialiser",
                            Icon::Refresh,
                            "Réinitialiser le contenu importé",
                        )
                        .ghost()
                        .disabled(self.converting)
                        .on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                this.confirm_clear = true;
                                cx.notify();
                            },
                        )),
                    ),
            )
            .child(div().h(px(280.)).child(self.table.clone()))
            .into_any_element()
    }
}

impl Render for LabelImportPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let hints = div()
            .flex()
            .flex_col()
            .gap(theme.space_md)
            .text_size(theme.text_sm)
            .text_color(theme.text_muted)
            .child(FORMAT_HINT)
            .child(WINAUDIT_HINT);
        let frame = PanelFrame::new("panneau-import-labels", PANEL.title())
            .width(PANEL.width())
            .on_close(cx.listener(|this, _: &ClickEvent, _, cx| this.close(cx)))
            .child(self.render_actions(cx))
            .child(self.render_content(cx))
            .child(hints);
        let overlay = self.converting.then(|| {
            div()
                .id("labels-conversion")
                .absolute()
                .inset_0()
                .occlude()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme.bg_backdrop)
                .text_color(theme.text_heading)
                .child("Conversion en cours…")
        });
        let confirm = self.confirm_clear.then(|| {
            Confirm::new(
                "labels-confirmer-vidage",
                "Réinitialiser le contenu",
                CLEAR_QUESTION,
            )
            .confirm_label("Vider")
            .danger()
            .on_confirm(cx.listener(|this, _: &(), _, cx| this.clear_label_store(cx)))
            .on_cancel(cx.listener(|this, _: &(), _, cx| {
                this.confirm_clear = false;
                cx.notify();
            }))
        });
        let accent = theme.accent;
        div()
            .id("import-labels")
            .track_focus(&self.focus)
            .key_context(MODAL_CONTEXT)
            .on_action(cx.listener(|this, _: &Cancel, _, cx| this.close(cx)))
            .relative()
            .flex_none()
            .h_full()
            .border_1()
            .border_color(gpui::transparent_black())
            .drag_over::<ExternalPaths>(move |style, _, _, _| style.border_color(accent))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| this.on_drop(paths, cx)))
            .child(frame)
            .children(overlay)
            .children(confirm)
    }
}
