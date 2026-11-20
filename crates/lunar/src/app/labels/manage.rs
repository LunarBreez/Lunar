//! Panneau de gestion des labels : table éditable
//! du store — source de vérité unique — avec recherche, ajout, édition en
//! place et suppression, statut observé/dormant par ligne, arbitrage des
//! conflits, import et export. Chaque mutation rend ses mises à jour du
//! graphe, appliquées sans dépendre du canal de capture, puis la table est
//! relue.

use gpui::{
    AnyElement, App, AppContext, ClickEvent, Context, ElementId, Entity, FocusHandle, Focusable,
    HighlightStyle, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, StyledText, Subscription, WeakEntity, Window, div,
    prelude::FluentBuilder, px,
};
use log::{error, info};
use lunar_engine::commandes::flow_matrix::LabelRowStatus;
use lunar_engine::commandes::import::labels::conflicts::LabelConflictReport;
use lunar_engine::errors::CaptureStateError;
use lunar_engine::state::graph::GraphUpdate;
use lunar_ui_logic::label_conflicts::dash_if_empty;
use lunar_ui_logic::label_management::{
    DELETE_TITLE, SEARCH_PLACEHOLDER, arbitrate_button, delete_question, empty_text,
    filter_status_rows, row_key, row_name, status_text, subtitle_parts, validate_entry,
};

use crate::app::labels::session::{LabelSession, LabelSessionEvent};
use crate::app::labels::{
    apply_label_updates, dialogs, export_labels, import_channel, release_import_lock,
    take_import_lock,
};
use crate::app::panels::Panel;
use crate::app::state::AppState;
use crate::assets::Icon;
use crate::theme::ActiveTheme;
use crate::widgets::button::{Button, ButtonSize};
use crate::widgets::modal::Confirm;
use crate::widgets::panel::Panel as PanelFrame;
use crate::widgets::table::{Column, ColumnAlign, Table, TableDelegate};
use crate::widgets::text_input::{TextInput, TextInputEvent};
use crate::widgets::tooltip::tooltip;
use crate::widgets::{Cancel, MODAL_CONTEXT};

/// Ce panneau.
const PANEL: Panel = Panel::Labels;

/// Colonne des boutons d'action.
const ACTIONS_COLUMN: usize = 4;

/// Champs d'une saisie : MAC, IP, label.
#[derive(Clone)]
struct EntryInputs {
    /// MAC.
    mac: Entity<TextInput>,
    /// IP.
    ip: Entity<TextInput>,
    /// Label.
    label: Entity<TextInput>,
}

impl EntryInputs {
    /// Trois champs, compacts pour une cellule de tableau.
    fn new(compact: bool, cx: &mut App) -> Self {
        let field = |placeholder: &'static str, mono: bool, cx: &mut App| {
            cx.new(|cx| {
                let input = TextInput::new(cx).placeholder(placeholder);
                let input = if mono { input.mono() } else { input };
                if compact { input.compact() } else { input }
            })
        };
        Self {
            mac: field("MAC (aa:bb:cc:dd:ee:ff)", true, cx),
            ip: field("IP (v4 ou v6)", true, cx),
            label: field("Label", false, cx),
        }
    }

    /// Les trois champs.
    fn all(&self) -> [&Entity<TextInput>; 3] {
        [&self.mac, &self.ip, &self.label]
    }

    /// Textes saisis : MAC, IP, label.
    fn texts(&self, cx: &App) -> (String, String, String) {
        (
            self.mac.read(cx).text().to_owned(),
            self.ip.read(cx).text().to_owned(),
            self.label.read(cx).text().to_owned(),
        )
    }

    /// Remplace les textes et efface les erreurs.
    fn set(&self, mac: &str, ip: &str, label: &str, cx: &mut App) {
        for (input, text) in self.all().into_iter().zip([mac, ip, label]) {
            input.update(cx, |input, cx| {
                input.set_text(text, cx);
                input.set_error(None, cx);
            });
        }
    }

    /// Place les erreurs de la saisie sous leurs champs ; rend vrai si la
    /// saisie est valide.
    fn check(&self, require_label: bool, cx: &mut App) -> bool {
        let (mac, ip, label) = self.texts(cx);
        let errors = validate_entry(&mac, &ip, &label, require_label);
        for (input, error) in self.all().into_iter().zip([
            errors.mac.clone(),
            errors.ip.clone(),
            errors.label.clone(),
        ]) {
            input.update(cx, |input, cx| input.set_error(error.map(Into::into), cx));
        }
        errors.is_empty()
    }
}

/// Lignes de la table : store, filtre, ligne en édition.
pub struct ManagedRows {
    /// Colonnes : statut, MAC, IP, label, actions.
    columns: Vec<Column>,
    /// Lignes du store avec leur statut.
    rows: Vec<LabelRowStatus>,
    /// Indices des lignes qui passent la recherche.
    visible: Vec<usize>,
    /// Recherche courante.
    search: String,
    /// Clé de la ligne en édition.
    editing: Option<String>,
    /// Champs d'édition en place.
    editors: EntryInputs,
    /// Panneau, pour les boutons des lignes.
    panel: WeakEntity<LabelsPanel>,
    /// Mutation en cours : boutons grisés.
    busy: bool,
}

impl ManagedRows {
    /// Ligne visible `row`.
    fn row(&self, row: usize) -> Option<&LabelRowStatus> {
        self.visible
            .get(row)
            .and_then(|index| self.rows.get(*index))
    }

    /// Recalcule les lignes visibles.
    fn refilter(&mut self) {
        self.visible = filter_status_rows(&self.rows, &self.search);
    }

    /// Ligne de clé `key`.
    fn find(&self, key: &str) -> Option<&LabelRowStatus> {
        self.rows
            .iter()
            .find(|row| row_key(&row.mac, &row.ip) == key)
    }

    /// Bouton d'action d'une ligne, relayé au panneau.
    fn action_button(
        &self,
        id: String,
        icon: Icon,
        tooltip: String,
        disabled: bool,
        action: impl Fn(&mut LabelsPanel, &mut Window, &mut Context<LabelsPanel>) + 'static,
    ) -> Button {
        let panel = self.panel.clone();
        Button::icon_only(ElementId::Name(id.into()), icon, tooltip)
            .ghost()
            .size(ButtonSize::Small)
            .disabled(disabled)
            .on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                let _ = panel.update(cx, |panel, cx| action(panel, window, cx));
            })
    }
}

impl TableDelegate for ManagedRows {
    fn columns(&self) -> &[Column] {
        &self.columns
    }

    fn row_count(&self) -> usize {
        self.visible.len()
    }

    fn cell_text(&self, row: usize, column: usize) -> gpui::SharedString {
        let Some(line) = self.row(row) else {
            return Default::default();
        };
        match column {
            0 => if line.observed { "1" } else { "0" }.into(),
            1 => dash_if_empty(&line.mac).to_owned().into(),
            2 => dash_if_empty(&line.ip).to_owned().into(),
            3 => dash_if_empty(&line.label).to_owned().into(),
            _ => Default::default(),
        }
    }

    fn render_cell(
        &self,
        row: usize,
        column: usize,
        _window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        let line = self.row(row)?;
        let key = row_key(&line.mac, &line.ip);
        let editing = self.editing.as_deref() == Some(key.as_str());
        let theme = cx.theme();
        if column == 0 {
            let color = if line.observed {
                theme.success
            } else {
                theme.text_muted
            };
            return Some(
                div()
                    .id(ElementId::Name(format!("labels-statut-{key}").into()))
                    .w_full()
                    .text_center()
                    .text_color(color)
                    .tooltip(tooltip(status_text(line.observed)))
                    .child("●")
                    .into_any_element(),
            );
        }
        if editing && (1..ACTIONS_COLUMN).contains(&column) {
            let input = match column {
                1 => &self.editors.mac,
                2 => &self.editors.ip,
                _ => &self.editors.label,
            };
            return Some(div().w_full().child(input.clone()).into_any_element());
        }
        if column != ACTIONS_COLUMN {
            return None;
        }
        let name = row_name(&line.mac, &line.ip);
        let buttons = if editing {
            [
                self.action_button(
                    format!("labels-enregistrer-{key}"),
                    Icon::Check,
                    format!("Enregistrer {name} (Entrée)"),
                    self.busy,
                    |panel, _, cx| panel.save_edit(cx),
                ),
                self.action_button(
                    format!("labels-annuler-{key}"),
                    Icon::Undo,
                    format!("Annuler l'édition de {name}"),
                    false,
                    |panel, _, cx| panel.cancel_edit(cx),
                ),
            ]
        } else {
            let edit_key = key.clone();
            let delete_key = key.clone();
            [
                self.action_button(
                    format!("labels-editer-{key}"),
                    Icon::Pencil,
                    format!("Éditer {name}"),
                    false,
                    move |panel, window, cx| panel.start_edit(&edit_key, window, cx),
                ),
                self.action_button(
                    format!("labels-supprimer-{key}"),
                    Icon::Trash,
                    format!("Supprimer {name}"),
                    self.busy,
                    move |panel, _, cx| panel.request_delete(&delete_key, cx),
                ),
            ]
        };
        Some(
            div()
                .flex()
                .w_full()
                .justify_end()
                .gap(theme.space_xs)
                .children(buttons)
                .into_any_element(),
        )
    }
}

/// Panneau de gestion des labels.
pub struct LabelsPanel {
    /// État de l'application.
    state: Entity<AppState>,
    /// Données partagées avec le dialogue d'arbitrage.
    session: Entity<LabelSession>,
    /// Focus du panneau (Échap).
    focus: FocusHandle,
    /// Recherche.
    search: Entity<TextInput>,
    /// Formulaire d'ajout.
    draft: EntryInputs,
    /// Table des labels.
    table: Entity<Table<ManagedRows>>,
    /// Conflits en attente d'arbitrage.
    conflicts: Vec<LabelConflictReport>,
    /// Mutation, import ou export en cours.
    busy: bool,
    /// Ligne dont la suppression attend confirmation.
    confirm_delete: Option<LabelRowStatus>,
    /// Erreur de la saisie en place, affichée au-dessus de la table.
    edit_error: Option<String>,
    /// Abonnements (champs, session).
    _subscriptions: Vec<Subscription>,
}

impl Focusable for LabelsPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl LabelsPanel {
    /// Panneau ouvert : lignes et conflits lus, focus dans la recherche.
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let session = LabelSession::global(cx);
        let search = cx.new(|cx| TextInput::new(cx).placeholder(SEARCH_PLACEHOLDER));
        let draft = EntryInputs::new(false, cx);
        let editors = EntryInputs::new(true, cx);
        let panel = cx.entity().downgrade();
        let table = cx.new(|cx| {
            Table::new(
                ManagedRows {
                    columns: vec![
                        Column::new("•", px(28.))
                            .align(ColumnAlign::Center)
                            .sortable(),
                        Column::flex("MAC", 1.2).mono().sortable(),
                        Column::flex("IP", 1.2).mono().sortable(),
                        Column::flex("Label", 1.5).sortable(),
                        Column::new("", px(68.)),
                    ],
                    rows: Vec::new(),
                    visible: Vec::new(),
                    search: String::new(),
                    editing: None,
                    editors: editors.clone(),
                    panel,
                    busy: false,
                },
                cx,
            )
        });
        let mut subscriptions = vec![
            cx.subscribe(&search, |this, _, event: &TextInputEvent, cx| {
                if let TextInputEvent::Changed(text) = event {
                    this.table.update(cx, |table, cx| {
                        let rows = table.delegate_mut();
                        rows.search = text.to_string();
                        rows.refilter();
                        table.refresh(cx);
                    });
                }
            }),
            cx.subscribe(
                &session,
                |this, _, event: &LabelSessionEvent, cx| match event {
                    LabelSessionEvent::Resolved => this.load(cx),
                },
            ),
        ];
        for input in draft.all() {
            subscriptions.push(
                cx.subscribe(
                    input,
                    |this, input, event: &TextInputEvent, cx| match event {
                        TextInputEvent::Changed(_) => {
                            input.update(cx, |input, cx| input.set_error(None, cx))
                        }
                        TextInputEvent::Submitted(_) => this.add_row(cx),
                    },
                ),
            );
        }
        for input in editors.all() {
            subscriptions.push(cx.subscribe(
                input,
                |this, _, event: &TextInputEvent, cx| match event {
                    TextInputEvent::Submitted(_) => this.save_edit(cx),
                    TextInputEvent::Changed(_) => {
                        if this.edit_error.take().is_some() {
                            cx.notify();
                        }
                    }
                },
            ));
        }
        search.read(cx).focus(window);
        let mut this = Self {
            state,
            session,
            focus: cx.focus_handle(),
            search,
            draft,
            table,
            conflicts: Vec::new(),
            busy: false,
            confirm_delete: None,
            edit_error: None,
            _subscriptions: subscriptions,
        };
        this.load(cx);
        this
    }

    /// Champs d'édition en place.
    fn editors(&self, cx: &App) -> EntryInputs {
        self.table.read(cx).delegate().editors.clone()
    }

    /// Ligne en édition.
    fn editing_row(&self, cx: &App) -> Option<LabelRowStatus> {
        let rows = self.table.read(cx).delegate();
        rows.editing
            .as_deref()
            .and_then(|key| rows.find(key))
            .cloned()
    }

    /// Indique une mutation en cours (boutons des lignes grisés).
    fn set_busy(&mut self, busy: bool, cx: &mut Context<Self>) {
        self.busy = busy;
        self.table.update(cx, |table, cx| {
            table.delegate_mut().busy = busy;
            cx.notify();
        });
        cx.notify();
    }

    /// Lit les lignes du store et les conflits en attente (`load`).
    fn load(&mut self, cx: &mut Context<Self>) {
        let bridge = self.state.read(cx).bridge().clone();
        bridge
            .run(
                cx,
                |lunar| (lunar.get_labels_with_status(), lunar.get_label_conflicts()),
                |this, loaded, cx| this.on_loaded(loaded, cx),
            )
            .detach();
    }

    /// Lignes et conflits lus.
    fn on_loaded(
        &mut self,
        loaded: (
            Result<Vec<LabelRowStatus>, CaptureStateError>,
            Result<Vec<LabelConflictReport>, CaptureStateError>,
        ),
        cx: &mut Context<Self>,
    ) {
        match loaded {
            (Ok(rows), Ok(conflicts)) => {
                self.conflicts = conflicts;
                self.table.update(cx, |table, cx| {
                    let delegate = table.delegate_mut();
                    delegate.rows = rows;
                    delegate.refilter();
                    table.refresh(cx);
                });
            }
            (Err(err), _) | (_, Err(err)) => {
                error!("Chargement des labels impossible: {err}");
                self.state
                    .update(cx, |state, cx| state.show_error(&err, cx));
            }
        }
        cx.notify();
    }

    /// Mutation réussie : mises à jour au graphe, puis lignes relues.
    fn after_mutation(&mut self, updates: Vec<GraphUpdate>, cx: &mut Context<Self>) {
        apply_label_updates(&self.state, updates, cx);
        self.load(cx);
    }

    /// Lance une mutation du store : `work` en fond, `Ok` → mises à jour
    /// appliquées puis `on_ok`, erreur → message.
    fn mutate(
        &mut self,
        cx: &mut Context<Self>,
        work: impl FnOnce(&lunar_engine::Lunar) -> Result<Vec<GraphUpdate>, CaptureStateError>
        + Send
        + 'static,
        on_ok: impl FnOnce(&mut Self, &mut Context<Self>) + 'static,
    ) {
        if self.busy {
            return;
        }
        self.set_busy(true, cx);
        let bridge = self.state.read(cx).bridge().clone();
        bridge
            .run(cx, work, move |this, result, cx| {
                this.set_busy(false, cx);
                match result {
                    Ok(updates) => {
                        on_ok(this, cx);
                        this.after_mutation(updates, cx);
                    }
                    Err(err) => this
                        .state
                        .update(cx, |state, cx| state.show_error(&err, cx)),
                }
            })
            .detach();
    }

    /// « Ajouter » : saisie vérifiée puis `add_label`.
    fn add_row(&mut self, cx: &mut Context<Self>) {
        if self.busy || !self.draft.check(true, cx) {
            return;
        }
        let (mac, ip, label) = self.draft.texts(cx);
        self.mutate(
            cx,
            move |lunar| lunar.add_label(mac, ip, label),
            |this, cx| this.draft.set("", "", "", cx),
        );
    }

    /// ✏️ : la ligne `key` passe en édition.
    fn start_edit(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.table.read(cx).delegate().find(key).cloned() else {
            return;
        };
        let editors = self.editors(cx);
        editors.set(&row.mac, &row.ip, &row.label, cx);
        self.edit_error = None;
        self.table.update(cx, |table, cx| {
            table.delegate_mut().editing = Some(key.to_owned());
            cx.notify();
        });
        editors.label.read(cx).focus(window);
        cx.notify();
    }

    /// ↩️ : édition abandonnée.
    fn cancel_edit(&mut self, cx: &mut Context<Self>) {
        self.edit_error = None;
        self.table.update(cx, |table, cx| {
            table.delegate_mut().editing = None;
            cx.notify();
        });
        cx.notify();
    }

    /// ✅ ou Entrée : saisie vérifiée puis `update_label` (clé comprise).
    fn save_edit(&mut self, cx: &mut Context<Self>) {
        let Some(row) = self.editing_row(cx) else {
            return;
        };
        if self.busy {
            return;
        }
        let (mac, ip, label) = self.editors(cx).texts(cx);
        let errors = validate_entry(&mac, &ip, &label, false);
        if !errors.is_empty() {
            self.edit_error = [errors.mac, errors.ip, errors.label]
                .into_iter()
                .flatten()
                .reduce(|a, b| format!("{a} ; {b}"));
            cx.notify();
            return;
        }
        self.mutate(
            cx,
            move |lunar| lunar.update_label(row.mac, row.ip, mac, ip, label),
            |this, cx| this.cancel_edit(cx),
        );
    }

    /// 🗑️ : confirmation demandée.
    fn request_delete(&mut self, key: &str, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.confirm_delete = self.table.read(cx).delegate().find(key).cloned();
        cx.notify();
    }

    /// Suppression confirmée : l'équipement est désétiqueté.
    fn delete_confirmed(&mut self, cx: &mut Context<Self>) {
        let Some(row) = self.confirm_delete.take() else {
            return;
        };
        self.mutate(
            cx,
            move |lunar| lunar.delete_label(row.mac, row.ip),
            |_, _| {},
        );
        cx.notify();
    }

    /// « Arbitrer » : conflits en attente présentés à l'arbitrage.
    fn open_arbitration(&mut self, cx: &mut Context<Self>) {
        let conflicts = self.conflicts.clone();
        self.session
            .update(cx, |session, _| session.set_arbitration(conflicts));
        self.state
            .update(cx, |state, cx| state.open_dialog(Panel::Arbitration, cx));
    }

    /// « Importer » : un fichier de labels remplace le store ; l'arbitrage
    /// s'ouvre s'il reste des conflits.
    fn import_file(&mut self, cx: &mut Context<Self>) {
        if self.busy || !take_import_lock(&self.state, cx) {
            return;
        }
        self.set_busy(true, cx);
        let dialog = dialogs::pick_csv(Some("Importer un fichier de labels"), "Labels CSV");
        let state = self.state.clone();
        cx.spawn(async move |this, cx| {
            let path = dialog.await;
            let channel = match path {
                Some(_) => cx.update(|cx| import_channel(&state, cx)).ok(),
                None => None,
            };
            let (Some(path), Some(channel)) = (path, channel) else {
                let _ = cx.update(|cx| release_import_lock(&state, cx));
                let _ = this.update(cx, |this, cx| this.set_busy(false, cx));
                return;
            };
            let Ok(bridge) = cx.update(|cx| state.read(cx).bridge().clone()) else {
                return;
            };
            let lunar = bridge.lunar().clone();
            let loaded = cx
                .background_executor()
                .spawn(async move {
                    let imported = lunar.import_label_file(path, channel);
                    let loaded = (lunar.get_labels_with_status(), lunar.get_label_conflicts());
                    (imported, loaded)
                })
                .await;
            let _ = cx.update(|cx| release_import_lock(&state, cx));
            let _ = this.update(cx, |this, cx| {
                this.set_busy(false, cx);
                let (imported, loaded) = loaded;
                match imported {
                    Ok(_) => info!("Labels importés depuis le panneau de gestion"),
                    Err(err) => this
                        .state
                        .update(cx, |state, cx| state.show_error(&err, cx)),
                }
                this.on_loaded(loaded, cx);
                this.state.update(cx, |state, cx| state.refresh_flags(cx));
                if !this.conflicts.is_empty() {
                    this.open_arbitration(cx);
                }
            });
        })
        .detach();
    }

    /// « Exporter » : export du fichier de labels (même chemin que le menu).
    fn export_file(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.state.update(cx, export_labels);
    }

    /// Échap : annule l'édition en cours, sinon ferme le panneau.
    fn on_cancel(&mut self, cx: &mut Context<Self>) {
        if self.table.read(cx).delegate().editing.is_some() {
            self.cancel_edit(cx);
        } else {
            self.close(cx);
        }
    }

    /// Ferme le panneau.
    fn close(&mut self, cx: &mut Context<Self>) {
        self.state
            .update(cx, |state, cx| state.close_panel(PANEL, cx));
    }

    /// Sous-titre : nombre de labels et légende des pastilles.
    fn render_subtitle(&self, total: usize, cx: &App) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let (head, observed, dormant) = subtitle_parts(total);
        let mut text = head;
        let first = text.len()..text.len() + "●".len();
        text.push('●');
        text.push_str(observed);
        let second = text.len()..text.len() + "●".len();
        text.push('●');
        text.push_str(dormant);
        let highlights = [
            (
                first,
                HighlightStyle {
                    color: Some(theme.success),
                    ..Default::default()
                },
            ),
            (
                second,
                HighlightStyle {
                    color: Some(theme.text_muted),
                    ..Default::default()
                },
            ),
        ];
        div()
            .text_size(theme.text_sm)
            .text_color(theme.text)
            .child(StyledText::new(text).with_highlights(highlights))
    }
}

impl Render for LabelsPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let (total, visible) = {
            let rows = self.table.read(cx).delegate();
            (rows.rows.len(), rows.visible.len())
        };
        let busy = self.busy;
        let toolbar = div()
            .flex()
            .items_center()
            .gap(theme.space_sm)
            .child(div().flex_1().min_w_0().child(self.search.clone()))
            .when(!self.conflicts.is_empty(), |d| {
                d.child(
                    Button::new("labels-arbitrer", arbitrate_button(self.conflicts.len()))
                        .icon(Icon::Scale)
                        .on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| this.open_arbitration(cx)),
                        ),
                )
            })
            .child(
                Button::new("labels-importer", "Importer")
                    .icon(Icon::Upload)
                    .disabled(busy)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.import_file(cx))),
            )
            .child(
                Button::new("labels-exporter", "Exporter")
                    .icon(Icon::FileExport)
                    .disabled(busy || total == 0)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.export_file(cx))),
            );
        let add_form = div()
            .flex()
            .items_start()
            .gap(theme.space_sm)
            .child(div().flex_1().min_w_0().child(self.draft.mac.clone()))
            .child(div().flex_1().min_w_0().child(self.draft.ip.clone()))
            .child(div().flex_1().min_w_0().child(self.draft.label.clone()))
            .child(
                Button::new("labels-ajouter", "Ajouter")
                    .icon(Icon::Plus)
                    .primary()
                    .disabled(busy)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.add_row(cx))),
            );
        let table = if visible == 0 {
            div()
                .py(theme.space_lg)
                .italic()
                .text_color(theme.text_muted)
                .child(empty_text(total))
                .into_any_element()
        } else {
            div()
                .flex_1()
                .min_h(px(240.))
                .child(self.table.clone())
                .into_any_element()
        };
        let frame = PanelFrame::new("panneau-labels", PANEL.title())
            .width(PANEL.width())
            .on_close(cx.listener(|this, _: &ClickEvent, _, cx| this.close(cx)))
            .child(self.render_subtitle(total, cx))
            .child(toolbar)
            .child(add_form)
            .children(self.edit_error.clone().map(|message| {
                div()
                    .text_size(theme.text_sm)
                    .text_color(theme.danger_text)
                    .child(message)
            }))
            .child(table);
        let confirm = self.confirm_delete.as_ref().map(|row| {
            Confirm::new(
                "labels-confirmer-suppression",
                DELETE_TITLE,
                delete_question(&row.mac, &row.ip, &row.label),
            )
            .confirm_label("Supprimer")
            .danger()
            .on_confirm(cx.listener(|this, _: &(), _, cx| this.delete_confirmed(cx)))
            .on_cancel(cx.listener(|this, _: &(), _, cx| {
                this.confirm_delete = None;
                cx.notify();
            }))
        });
        div()
            .id("gestion-labels")
            .track_focus(&self.focus)
            .key_context(MODAL_CONTEXT)
            .on_action(cx.listener(|this, _: &Cancel, _, cx| this.on_cancel(cx)))
            .flex_none()
            .h_full()
            .child(frame)
            .children(confirm)
    }
}
