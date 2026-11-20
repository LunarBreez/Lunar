//! Vue d'investigation :
//! lecture approfondie de la matrice de flux courante, hors du flux
//! d'événements de capture.
//!
//! La vue interroge le moteur (`get_graph_snapshot`, `get_survey_period`)
//! pour l'état de référence, puis présente des indicateurs synthétiques et
//! des onglets (Résumé, Flux, Endpoints, Déduction, Glossaire) triables,
//! filtrables et recherchables. Toute la logique (tri, recherche, filtres,
//! indicateurs, textes) est celle de [`lunar_ui_logic::investigation`] :
//! cette vue ne fait qu'afficher [`InvestigationModel::derive`] et appeler
//! les méthodes de [`ViewState`] (`InvestigationModel::state`).
//!
//! Le relevé s'exporte en page HTML inerte accompagnée de sa matrice CSV et
//! de l'empreinte de celle-ci (Ctrl+S, [`render_synthesis_html`]) ; les
//! équipements sans label s'exportent dans un fichier de labels à compléter
//! (Ctrl+E). Les actions de menu de la vue (`ExportSynthesis`,
//! `ExportLabelsToComplete`, `ReloadSurvey`) arrivent par
//! [`AppEvent::InvestigationAction`].
//!
//! Créée à l'entrée dans la vue, détruite à la sortie : chaque ouverture
//! relit le relevé.

mod render;
pub mod support;
pub mod tables;

use std::path::PathBuf;
use std::rc::Rc;

use gpui::{App, AppContext, Context, Entity, Focusable, Subscription, Window};
use log::{error, info};
use lunar_engine::errors::CaptureStateError;
use lunar_engine::state::graph::GraphUpdate;
use lunar_ui_logic::file_names::{labels_to_complete_csv_name, synthesis_html_name};
use lunar_ui_logic::investigation::tables::{DeductionRow, EndpointRow, FlowRow};
use lunar_ui_logic::investigation::view::{
    DEDUCTION_COLUMNS, ENDPOINT_COLUMNS, ExportOutcome, FLOW_COLUMNS, InvestigationDerived,
    LABELS_DIALOG_FILTER, LABELS_DIALOG_TITLE, LABELS_EXPORT_ERROR, LabelCommand,
    NO_DEDUCTION_TEXT, SAVE_LABEL_ERROR, SEARCH_PLACEHOLDER, SYNTHESIS_DIALOG_FILTER,
    SYNTHESIS_DIALOG_TITLE, SYNTHESIS_EXPORT_ERROR, SortKey, label_command, labels_notice,
    synthesis_notice,
};
use lunar_ui_logic::investigation::{
    EpochMillis, InvestigationModel, ViewState, render_synthesis_html,
};
use lunar_ui_logic::menu::MenuActionId;

use crate::app::bridge::Bridge;
use crate::app::investigation::support::{
    SystemZone, local_date_time, survey_period, synthesis_export, with_extension,
};
use crate::app::investigation::tables::{LabelEditor, RowTable, clicked_key, update_table};
use crate::app::state::{AppEvent, AppState, InvestigationFlags};
use crate::widgets::table::{Table, TableEvent};
use crate::widgets::text_input::{TextInput, TextInputEvent};

/// Tableau d'un type de ligne de la vue.
type RowTableEntity<R> = Entity<Table<RowTable<R>>>;

/// Page de synthèse écrite : de quoi rendre compte de l'export.
struct SynthesisWritten {
    /// Chemin de la page.
    html_path: String,
    /// Nom du CSV écrit à côté.
    csv_file_name: String,
    /// Empreinte du CSV.
    csv_sha256: String,
}

/// Échec d'un export.
enum ExportFailure {
    /// Le moteur a refusé (message d'erreur à montrer).
    Engine(CaptureStateError),
    /// L'écriture de la page a échoué.
    Write(std::io::Error),
}

impl std::fmt::Display for ExportFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Engine(err) => write!(f, "{err}"),
            Self::Write(err) => write!(f, "écriture de la page : {err}"),
        }
    }
}

/// Vue d'investigation.
pub struct InvestigationView {
    /// État de l'application (verrou d'import, indicateurs de menu, graphe).
    app: Entity<AppState>,
    /// Accès au moteur.
    bridge: Bridge,
    /// Relevé chargé et état de la vue.
    model: InvestigationModel,
    /// Valeurs affichées, recalculées à chaque changement ([`Self::recompute`]).
    derived: InvestigationDerived,
    /// Recherche plein texte.
    search: Entity<TextInput>,
    /// Champ d'édition en place des labels.
    editor: Entity<TextInput>,
    /// Tableau des flux.
    flows: RowTableEntity<FlowRow>,
    /// Tableau des endpoints.
    endpoints: RowTableEntity<EndpointRow>,
    /// Tableau des déductions.
    deductions: RowTableEntity<DeductionRow>,
    /// Derniers indicateurs transmis à l'état de l'application.
    flags: Option<InvestigationFlags>,
    /// Abonnements (état, tableaux, champs).
    _subscriptions: Vec<Subscription>,
}

impl InvestigationView {
    /// Vue sur l'état `app` ; le relevé est lu aussitôt.
    pub fn new(app: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let bridge = app.read(cx).bridge().clone();
        let search = cx.new(|cx| TextInput::new(cx).placeholder(SEARCH_PLACEHOLDER));
        let editor = cx.new(|cx| {
            TextInput::new(cx)
                .compact()
                .placeholder(lunar_ui_logic::investigation::view::LABEL_PLACEHOLDER)
        });
        let weak = cx.weak_entity();
        let on_edit = Rc::new(move |id: String, window: &mut Window, cx: &mut App| {
            let _ = weak.update(cx, |view, cx| view.start_edit(id, window, cx));
        });
        let label = LabelEditor {
            editor: editor.clone(),
            editing: None,
            saving: None,
            on_edit,
        };
        let flows = cx.new(|cx| Table::new(RowTable::new(None), cx).empty_text(""));
        let endpoints = cx.new(|cx| Table::new(RowTable::new(Some(label)), cx).empty_text(""));
        let deductions =
            cx.new(|cx| Table::new(RowTable::new(None), cx).empty_text(NO_DEDUCTION_TEXT));

        let editor_focus = editor.focus_handle(cx);
        let subscriptions = vec![
            cx.subscribe_in(&app, window, Self::on_app_event),
            cx.subscribe(&search, |this, _, event: &TextInputEvent, cx| {
                if let TextInputEvent::Changed(text) = event {
                    this.model.state.query = text.to_string();
                    this.recompute(cx);
                }
            }),
            cx.subscribe_in(
                &editor,
                window,
                |this, _, event: &TextInputEvent, window, cx| {
                    if let TextInputEvent::Submitted(_) = event {
                        // Entrée : le champ perd le focus et le label part
                        // (`@keyup.enter` → `blur()` → `@change`).
                        this.commit_edit(cx);
                        window.focus(&this.endpoints.focus_handle(cx));
                    }
                },
            ),
            cx.on_blur(&editor_focus, window, |this, _, cx| this.commit_edit(cx)),
            cx.subscribe(&flows, |this, table, event: &TableEvent, cx| {
                if let TableEvent::SortChanged(sort) = event {
                    let keys = table.read(cx).delegate().keys().to_vec();
                    this.header_clicked(clicked_key(&keys, *sort, this.model.state.sort_key), cx);
                }
            }),
            cx.subscribe_in(
                &endpoints,
                window,
                |this, table, event: &TableEvent, window, cx| match event {
                    TableEvent::SortChanged(sort) => {
                        let keys = table.read(cx).delegate().keys().to_vec();
                        this.header_clicked(
                            clicked_key(&keys, *sort, this.model.state.sort_key),
                            cx,
                        );
                    }
                    // Double clic ou Entrée sur une ligne : édition du label.
                    TableEvent::Activated(row) => {
                        let id = table
                            .read(cx)
                            .delegate()
                            .rows()
                            .get(*row)
                            .map(|row: &EndpointRow| row.id.clone());
                        if let Some(id) = id {
                            this.start_edit(id, window, cx);
                        }
                    }
                    TableEvent::SelectionChanged(_) => {}
                },
            ),
            cx.subscribe(&deductions, |this, table, event: &TableEvent, cx| {
                if let TableEvent::SortChanged(sort) = event {
                    let keys = table.read(cx).delegate().keys().to_vec();
                    this.header_clicked(clicked_key(&keys, *sort, this.model.state.sort_key), cx);
                }
            }),
        ];

        let model = InvestigationModel::default();
        let derived = model.derive(&SystemZone);
        let mut view = Self {
            app,
            bridge,
            model,
            derived,
            search,
            editor,
            flows,
            endpoints,
            deductions,
            flags: None,
            _subscriptions: subscriptions,
        };
        view.reload(cx);
        view
    }

    /// Actions de menu propres à la vue ; les autres sont
    /// traitées par [`AppState::run_action`].
    fn on_app_event(
        &mut self,
        _: &Entity<AppState>,
        event: &AppEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let AppEvent::InvestigationAction(id) = event {
            match id {
                MenuActionId::ExportSynthesis => self.export_synthesis(cx),
                MenuActionId::ExportLabelsToComplete => self.export_labels_to_complete(cx),
                MenuActionId::ReloadSurvey => self.reload(cx),
                _ => {}
            }
        }
    }

    /// Modifie l'état de la vue puis recalcule l'affichage.
    fn update_state(&mut self, update: impl FnOnce(&mut ViewState), cx: &mut Context<Self>) {
        update(&mut self.model.state);
        self.recompute(cx);
    }

    /// Recalcule ce qu'affiche la vue, alimente les tableaux et transmet
    /// les indicateurs de chargement et d'export aux menus.
    fn recompute(&mut self, cx: &mut Context<Self>) {
        self.derived = self.model.derive(&SystemZone);
        let state = &self.model.state;
        let flows = std::mem::take(&mut self.derived.visible_flows);
        let endpoints = std::mem::take(&mut self.derived.visible_endpoints);
        let deductions = std::mem::take(&mut self.derived.visible_deductions);
        update_table(&self.flows, &FLOW_COLUMNS, flows, state, cx);
        update_table(&self.endpoints, &ENDPOINT_COLUMNS, endpoints, state, cx);
        update_table(&self.deductions, &DEDUCTION_COLUMNS, deductions, state, cx);
        let saving = state.saving_label.clone();
        self.endpoints.update(cx, |table, cx| {
            if let Some(label) = table.delegate_mut().label.as_mut() {
                label.saving = saving;
            }
            cx.notify();
        });
        let flags = InvestigationFlags {
            loading: state.loading,
            exporting: state.exporting,
        };
        if self.flags != Some(flags) {
            self.flags = Some(flags);
            self.app
                .update(cx, |app, cx| app.set_investigation_flags(flags, cx));
        }
        cx.notify();
    }

    /// Clic sur un en-tête de tableau : tri de la vue sur `key`.
    fn header_clicked(&mut self, key: SortKey, cx: &mut Context<Self>) {
        self.update_state(|state| state.sort_by(key), cx);
    }

    /// Verrou d'import de l'application.
    fn importing(&self, cx: &App) -> bool {
        self.app.read(cx).capture().is_importing
    }

    /// Pose ou rend le verrou d'import (`store.isImporting`).
    fn set_import_lock(app: &Entity<AppState>, locked: bool, cx: &mut App) {
        app.update(cx, |app, cx| {
            app.update_capture(|capture| capture.is_importing = locked, cx)
        });
    }

    // --- Relevé ---------------------------------------------------------

    /// Relit le relevé (`reload`) : graphe et période ensemble.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.update_state(ViewState::begin_reload, cx);
        self.bridge
            .run(
                cx,
                |lunar| (lunar.get_graph_snapshot(), lunar.get_survey_period()),
                |this, (graph, period), cx| {
                    match (graph, period) {
                        (Ok(graph), Ok(period)) => {
                            this.model.load(graph, survey_period(period));
                            info!("[Investigation] snapshot chargé");
                        }
                        (Err(err), _) | (_, Err(err)) => {
                            this.model.state.finish_reload(false);
                            error!("[Investigation] get_graph_snapshot: {err}");
                        }
                    }
                    this.recompute(cx);
                },
            )
            .detach();
    }

    // --- Labels ---------------------------------------------------------

    /// Ouvre l'édition du label de l'endpoint `id` dans sa ligne.
    fn start_edit(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.model.state.saving_label.as_deref() == Some(id.as_str()) {
            return;
        }
        // Une autre ligne en cours d'édition part d'abord (blur).
        self.commit_edit(cx);
        let current = self
            .endpoints
            .read(cx)
            .delegate()
            .find(&id)
            .map(|row| row.label.clone());
        let Some(current) = current else {
            return;
        };
        self.endpoints.update(cx, |table, cx| {
            if let Some(label) = table.delegate_mut().label.as_mut() {
                label.editing = Some(id);
            }
            cx.notify();
        });
        self.editor
            .update(cx, |editor, cx| editor.set_text(&current, cx));
        self.editor.read(cx).focus(window);
    }

    /// Ferme l'édition en cours et enregistre le label s'il a changé
    /// : `add_label`, ou `delete_label` si le champ est vidé.
    fn commit_edit(&mut self, cx: &mut Context<Self>) {
        let editing = self.endpoints.update(cx, |table, cx| {
            let editing = table
                .delegate_mut()
                .label
                .as_mut()
                .and_then(|label| label.editing.take());
            cx.notify();
            editing
        });
        let Some(id) = editing else {
            return;
        };
        let value = self.editor.read(cx).text().to_string();
        let command = self
            .endpoints
            .read(cx)
            .delegate()
            .find(&id)
            .and_then(|row| label_command(row, &value));
        let Some(command) = command else {
            return;
        };
        self.update_state(|state| state.saving_label = Some(id), cx);
        self.bridge
            .run(
                cx,
                move |lunar| {
                    let updates = match command {
                        LabelCommand::Add { mac, ip, label } => lunar.add_label(mac, ip, label),
                        LabelCommand::Delete { mac, ip } => lunar.delete_label(mac, ip),
                    }?;
                    Ok::<_, CaptureStateError>((updates, lunar.get_graph_snapshot()?))
                },
                |this, result: Result<(Vec<GraphUpdate>, _), _>, cx| {
                    match result {
                        Ok((updates, graph)) => {
                            // Le graphe principal reste cohérent, puis la vue
                            // relit le snapshot pour refléter le label.
                            this.app.update(cx, |app, cx| {
                                app.graph().apply_updates(updates, cx);
                                app.refresh_flags(cx);
                            });
                            this.model.graph = graph;
                        }
                        Err(err) => {
                            this.model.state.error = SAVE_LABEL_ERROR.to_string();
                            error!("[Investigation] enregistrement du label : {err}");
                        }
                    }
                    this.update_state(|state| state.saving_label = None, cx);
                },
            )
            .detach();
    }

    // --- Exports --------------------------------------------------------

    /// Exporte le relevé complet, filtres ignorés : le
    /// moteur écrit la matrice CSV à côté de la page et rend son empreinte
    /// avec le graphe du même instant ; la page est composée ici puis
    /// écrite. Verrou d'import tenu de bout en bout.
    pub fn export_synthesis(&mut self, cx: &mut Context<Self>) {
        let importing = self.importing(cx);
        let mut accepted = false;
        self.update_state(|state| accepted = state.begin_export(importing), cx);
        if !accepted {
            return;
        }
        let app = self.app.clone();
        Self::set_import_lock(&app, true, cx);
        let now = local_date_time(EpochMillis::now(), &SystemZone);
        let (filter_name, extensions) = SYNTHESIS_DIALOG_FILTER;
        let dialog = rfd::AsyncFileDialog::new()
            .set_title(SYNTHESIS_DIALOG_TITLE)
            .add_filter(filter_name, extensions)
            .set_file_name(synthesis_html_name(&now))
            .save_file();
        let lunar = self.bridge.lunar().clone();
        cx.spawn(async move |this, cx| {
            let outcome = match dialog.await {
                None => {
                    info!("[Investigation] export de synthèse annulé");
                    ExportOutcome::Cancelled
                }
                Some(file) => {
                    let path = with_extension(file.path(), "html", &["html", "htm"]);
                    let written = cx
                        .background_executor()
                        .spawn(async move { write_synthesis(&lunar, path) })
                        .await;
                    match written {
                        Ok(done) => {
                            info!(
                                "[Investigation] synthèse exportée : {}, matrice {}, SHA-256 {}",
                                done.html_path, done.csv_file_name, done.csv_sha256
                            );
                            ExportOutcome::Done(synthesis_notice(
                                &done.html_path,
                                &done.csv_file_name,
                                &done.csv_sha256,
                            ))
                        }
                        Err(err) => {
                            error!("[Investigation] export de synthèse : {err}");
                            if let ExportFailure::Engine(err) = &err {
                                let _ = app.update(cx, |app, cx| app.show_error(err, cx));
                            }
                            ExportOutcome::Failed(SYNTHESIS_EXPORT_ERROR)
                        }
                    }
                }
            };
            Self::finish_export(&app, &this, outcome, cx);
        })
        .detach();
    }

    /// Exporte le fichier de labels à compléter : équipements sans label en
    /// tête, puis les labels en place.
    pub fn export_labels_to_complete(&mut self, cx: &mut Context<Self>) {
        let importing = self.importing(cx);
        let mut accepted = false;
        self.update_state(|state| accepted = state.begin_export(importing), cx);
        if !accepted {
            return;
        }
        let app = self.app.clone();
        Self::set_import_lock(&app, true, cx);
        let now = local_date_time(EpochMillis::now(), &SystemZone);
        let (filter_name, extensions) = LABELS_DIALOG_FILTER;
        let dialog = rfd::AsyncFileDialog::new()
            .set_title(LABELS_DIALOG_TITLE)
            .add_filter(filter_name, extensions)
            .set_file_name(labels_to_complete_csv_name(&now))
            .save_file();
        let lunar = self.bridge.lunar().clone();
        cx.spawn(async move |this, cx| {
            let outcome = match dialog.await {
                None => {
                    info!("[Investigation] export des labels à compléter annulé");
                    ExportOutcome::Cancelled
                }
                Some(file) => {
                    let path = path_text(with_extension(file.path(), "csv", &["csv"]));
                    let target = path.clone();
                    let count = cx
                        .background_executor()
                        .spawn(async move { lunar.export_labels_to_complete(target) })
                        .await;
                    match count {
                        Ok(count) => {
                            info!(
                                "[Investigation] labels à compléter : {count} équipement(s) sans label exportés vers {path}"
                            );
                            ExportOutcome::Done(labels_notice(
                                u64::try_from(count).unwrap_or(u64::MAX),
                                &path,
                            ))
                        }
                        Err(err) => {
                            error!("[Investigation] export des labels à compléter : {err}");
                            let _ = app.update(cx, |app, cx| app.show_error(&err, cx));
                            ExportOutcome::Failed(LABELS_EXPORT_ERROR)
                        }
                    }
                }
            };
            Self::finish_export(&app, &this, outcome, cx);
        })
        .detach();
    }

    /// Fin d'un export : verrou rendu (même si la vue a été quittée
    /// entre-temps), compte rendu dans la vue.
    fn finish_export(
        app: &Entity<AppState>,
        this: &gpui::WeakEntity<Self>,
        outcome: ExportOutcome,
        cx: &mut gpui::AsyncApp,
    ) {
        let _ = cx.update(|cx| Self::set_import_lock(app, false, cx));
        let _ = this.update(cx, |this, cx| {
            this.update_state(|state| state.finish_export(outcome), cx)
        });
    }
}

/// Chemin en texte, comme les commandes du moteur le prennent.
fn path_text(path: PathBuf) -> String {
    path.to_string_lossy().into_owned()
}

/// Écrit le CSV (moteur) puis la page de synthèse à côté.
fn write_synthesis(
    lunar: &lunar_engine::Lunar,
    path: PathBuf,
) -> Result<SynthesisWritten, ExportFailure> {
    let html_path = path_text(path);
    let export = lunar
        .export_synthesis_matrix(html_path.clone())
        .map_err(ExportFailure::Engine)?;
    let export = synthesis_export(export);
    let html = render_synthesis_html(&export, EpochMillis::now(), &SystemZone);
    std::fs::write(&html_path, html.as_bytes()).map_err(ExportFailure::Write)?;
    Ok(SynthesisWritten {
        html_path,
        csv_file_name: export.csv_file_name,
        csv_sha256: export.csv_sha256,
    })
}
