//! Tableaux de la vue d'investigation (flux, endpoints, déductions) sur
//! [`Table`] : les lignes arrivent déjà filtrées et triées par
//! `lunar_ui_logic::investigation::view` ; le tableau ne retrie jamais (sa
//! comparaison rend l'égalité) et sa flèche de tri ne fait que montrer le
//! tri de la vue, partagé par les trois tableaux.
//!
//! Cellules particulières : infobulles, étiquette du
//! type de déduction (`.tag-P`, `.tag-F`, `.tag-A`) et champ de label des
//! endpoints, édité en place par un seul [`TextInput`] partagé.

use std::cmp::Ordering;
use std::rc::Rc;

use gpui::{
    AnyElement, App, CursorStyle, ElementId, Entity, Hsla, IntoElement, SharedString, Window, div,
    prelude::*, px, rgb, rgba,
};
use lunar_ui_logic::investigation::tables::{DeductionKind, DeductionRow, EndpointRow, FlowRow};
use lunar_ui_logic::investigation::view::{
    ColumnDef, InvestigationRow, LABEL_PLACEHOLDER, SortKey, ViewState, deduction_cell,
    endpoint_cell, endpoint_cell_title, flow_cell, flow_cell_title,
};

use crate::widgets::ellipsis;
use crate::widgets::table::{Column, ColumnAlign, SortDirection, SortState, Table, TableDelegate};
use crate::widgets::text_input::TextInput;
use crate::widgets::tooltip::tooltip;

/// Rappel d'ouverture de l'édition du label d'un endpoint (id du nœud).
pub type EditHandler = Rc<dyn Fn(String, &mut Window, &mut App)>;

/// Édition en place des labels (tableau des endpoints seulement).
pub struct LabelEditor {
    /// Champ partagé, affiché dans la ligne en cours d'édition.
    pub editor: Entity<TextInput>,
    /// Id de l'endpoint en cours d'édition.
    pub editing: Option<String>,
    /// Id de l'endpoint dont le label est en cours d'enregistrement (champ
    /// grisé).
    pub saving: Option<String>,
    /// Ouvre l'édition d'un endpoint.
    pub on_edit: EditHandler,
}

/// Ligne affichable d'un tableau de la vue.
pub trait CellRow: InvestigationRow + Clone + 'static {
    /// Identité stable de la ligne.
    fn row_key(&self) -> String;

    /// Texte de la cellule de la colonne `key`.
    fn cell(&self, key: SortKey) -> String;

    /// Infobulle de la cellule (`title`).
    fn title(&self, _key: SortKey) -> Option<String> {
        None
    }

    /// Ligne en alerte (`tr.warn`).
    fn warning(&self) -> bool {
        false
    }

    /// Cellule dessinée à part ; `None` : texte de [`Self::cell`].
    fn custom_cell(
        &self,
        _key: SortKey,
        _row_ix: usize,
        _label: Option<&LabelEditor>,
    ) -> Option<AnyElement> {
        None
    }
}

impl CellRow for FlowRow {
    fn row_key(&self) -> String {
        self.id.clone()
    }

    fn cell(&self, key: SortKey) -> String {
        flow_cell(self, key)
    }

    fn title(&self, key: SortKey) -> Option<String> {
        flow_cell_title(self, key).map(str::to_string)
    }
}

impl CellRow for EndpointRow {
    fn row_key(&self) -> String {
        self.id.clone()
    }

    fn cell(&self, key: SortKey) -> String {
        endpoint_cell(self, key)
    }

    fn title(&self, key: SortKey) -> Option<String> {
        endpoint_cell_title(self, key)
    }

    fn warning(&self) -> bool {
        self.duplicate_ip
    }

    fn custom_cell(
        &self,
        key: SortKey,
        row_ix: usize,
        label: Option<&LabelEditor>,
    ) -> Option<AnyElement> {
        if key != SortKey::Label {
            return None;
        }
        let label = label?;
        Some(label_cell(self, row_ix, label))
    }
}

impl CellRow for DeductionRow {
    fn row_key(&self) -> String {
        format!("{}:{}", self.kind.label(), self.observed)
    }

    fn cell(&self, key: SortKey) -> String {
        deduction_cell(self, key)
    }

    fn custom_cell(
        &self,
        key: SortKey,
        _row_ix: usize,
        _label: Option<&LabelEditor>,
    ) -> Option<AnyElement> {
        (key == SortKey::Kind).then(|| kind_tag(self.kind).into_any_element())
    }
}

/// Couleurs d'une étiquette de type : texte, bordure, fond.
fn tag_colors(kind: DeductionKind) -> (Hsla, Hsla, Hsla) {
    // `.tag-P`, `.tag-F`, `.tag-A` (initiale du libellé).
    match kind {
        DeductionKind::Port => (
            rgb(0x84c3f7).into(),
            rgb(0x2c4a6b).into(),
            rgba(0x2196f31f).into(),
        ),
        DeductionKind::Oui => (
            rgb(0xb39ddb).into(),
            rgb(0x4a3d6b).into(),
            rgba(0x7c4dff1f).into(),
        ),
        DeductionKind::KnownAddress => (
            rgb(0xffcc80).into(),
            rgb(0x6b5326).into(),
            rgba(0xff98001f).into(),
        ),
    }
}

/// Étiquette du type d'une déduction (`span.tag`).
fn kind_tag(kind: DeductionKind) -> impl IntoElement {
    let (text, border, bg) = tag_colors(kind);
    div()
        .flex_none()
        .px(px(8.))
        .py(px(1.))
        .rounded(px(10.))
        .border_1()
        .border_color(border)
        .bg(bg)
        .text_size(px(11.))
        .text_color(text)
        .whitespace_nowrap()
        .child(kind.label())
}

/// Champ de label d'un endpoint : le champ partagé pendant l'édition,
/// sinon une zone qui en a l'apparence (`.label-input`) et ouvre l'édition
/// au clic.
fn label_cell(row: &EndpointRow, row_ix: usize, label: &LabelEditor) -> AnyElement {
    if label.editing.as_deref() == Some(row.id.as_str()) {
        return div()
            .w_full()
            .min_w(px(140.))
            .child(label.editor.clone())
            .into_any_element();
    }
    let saving = label.saving.as_deref() == Some(row.id.as_str());
    let (text, color): (SharedString, Hsla) = if row.label.is_empty() {
        (LABEL_PLACEHOLDER.into(), rgb(0x5a6b82).into())
    } else {
        (row.label.clone().into(), rgb(0xcfe3ff).into())
    };
    let hover_border: Hsla = rgb(0x333b52).into();
    let field = div()
        .id(ElementId::named_usize("label", row_ix))
        .w_full()
        .min_w(px(140.))
        .px(px(6.))
        .py(px(2.))
        .rounded(px(4.))
        .border_1()
        .border_color(gpui::transparent_black())
        .bg(rgb(0x05030f))
        .text_color(color)
        .child(ellipsis(div()).child(text));
    if saving {
        return field.opacity(0.5).into_any_element();
    }
    let on_edit = Rc::clone(&label.on_edit);
    let id = row.id.clone();
    field
        .cursor(CursorStyle::IBeam)
        .hover(move |s| s.border_color(hover_border))
        .on_click(move |_, window, cx| on_edit(id.clone(), window, cx))
        .into_any_element()
}

/// Colonne du tableau pour une colonne de la vue : largeur et alignement
/// d'après le contenu (le tableau HTML se dimensionnait seul).
pub fn column_for(def: &ColumnDef) -> Column {
    let fixed = |width: f32| Column::new(def.label, px(width));
    let flex = |weight: f32| Column::flex(def.label, weight);
    let column = match def.key {
        SortKey::Source | SortKey::Target => flex(1.0),
        SortKey::Protocol => fixed(170.),
        SortKey::Ports => flex(0.8),
        SortKey::Bidir => fixed(64.),
        SortKey::Name => flex(1.2),
        SortKey::Ip => fixed(150.),
        SortKey::Mac => fixed(190.),
        SortKey::Vlan => fixed(70.),
        SortKey::Label => flex(1.0),
        SortKey::Kind => fixed(170.),
        SortKey::Observed => flex(1.0),
        SortKey::Deduction => flex(2.0),
        SortKey::Degree => fixed(70.).numeric(),
        SortKey::FlowsOut | SortKey::FlowsIn => fixed(96.).numeric(),
        SortKey::Count => fixed(120.).numeric(),
        SortKey::Packets | SortKey::Bytes => fixed(110.).numeric(),
    };
    column.sortable()
}

/// Source de données d'un tableau de la vue.
pub struct RowTable<R: CellRow> {
    /// Colonnes affichées.
    columns: Vec<Column>,
    /// Clé de chaque colonne affichée.
    keys: Vec<SortKey>,
    /// Lignes, dans l'ordre d'affichage.
    rows: Vec<R>,
    /// Édition des labels (endpoints).
    pub label: Option<LabelEditor>,
}

impl<R: CellRow> RowTable<R> {
    /// Tableau vide.
    pub fn new(label: Option<LabelEditor>) -> Self {
        Self {
            columns: Vec::new(),
            keys: Vec::new(),
            rows: Vec::new(),
            label,
        }
    }

    /// Clés des colonnes affichées.
    pub fn keys(&self) -> &[SortKey] {
        &self.keys
    }

    /// Lignes affichées.
    pub fn rows(&self) -> &[R] {
        &self.rows
    }

    /// Ligne d'identité `key`.
    pub fn find(&self, key: &str) -> Option<&R> {
        self.rows.iter().find(|row| row.row_key() == key)
    }
}

impl<R: CellRow> TableDelegate for RowTable<R> {
    fn columns(&self) -> &[Column] {
        &self.columns
    }

    fn row_count(&self) -> usize {
        self.rows.len()
    }

    fn cell_text(&self, row: usize, column: usize) -> SharedString {
        match (self.rows.get(row), self.keys.get(column)) {
            (Some(row), Some(key)) => row.cell(*key).into(),
            _ => SharedString::default(),
        }
    }

    fn compare(&self, _a: usize, _b: usize, _column: usize) -> Ordering {
        // Lignes déjà triées par la vue : le tri stable garde leur ordre.
        Ordering::Equal
    }

    fn render_cell(
        &self,
        row: usize,
        column: usize,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Option<AnyElement> {
        let (line, key) = (self.rows.get(row)?, *self.keys.get(column)?);
        if let Some(custom) = line.custom_cell(key, row, self.label.as_ref()) {
            return Some(custom);
        }
        let title = line.title(key)?;
        Some(
            ellipsis(div().id(ElementId::named_usize("cellule", column)).w_full())
                .when(
                    self.columns
                        .get(column)
                        .is_some_and(|c| c.align == ColumnAlign::Right),
                    |d| d.text_right(),
                )
                .tooltip(tooltip(title))
                .child(line.cell(key))
                .into_any_element(),
        )
    }

    fn row_warning(&self, row: usize) -> bool {
        self.rows.get(row).is_some_and(CellRow::warning)
    }
}

/// Tri du tableau qui montre celui de la vue : colonne de la clé de tri si
/// elle est affichée, sens de la vue.
pub fn table_sort(keys: &[SortKey], state: &ViewState) -> Option<SortState> {
    keys.iter()
        .position(|key| *key == state.sort_key)
        .map(|column| SortState {
            column,
            direction: if state.sort_asc {
                SortDirection::Ascending
            } else {
                SortDirection::Descending
            },
        })
}

/// Colonne dont l'en-tête vient d'être cliqué, d'après le tri que le
/// tableau propose (`proposed`) : le tableau cycle croissant, décroissant,
/// sans tri ; « sans tri » ne survient qu'après un décroissant sur la
/// colonne triée, donc sur la clé de tri de la vue.
pub fn clicked_key(keys: &[SortKey], proposed: Option<SortState>, current: SortKey) -> SortKey {
    proposed
        .and_then(|sort| keys.get(sort.column).copied())
        .unwrap_or(current)
}

/// Colonnes visibles de `defs` (filtre « Colonnes »).
pub fn visible_columns(defs: &[ColumnDef], state: &ViewState) -> Vec<ColumnDef> {
    defs.iter()
        .copied()
        .filter(|def| state.is_column_visible(def.key))
        .collect()
}

/// Remplace colonnes et lignes d'un tableau, montre le tri de la vue et
/// garde la sélection sur la même ligne quand elle est encore affichée.
pub fn update_table<R: CellRow>(
    table: &Entity<Table<RowTable<R>>>,
    defs: &[ColumnDef],
    rows: Vec<R>,
    state: &ViewState,
    cx: &mut App,
) {
    table.update(cx, |table, cx| {
        let selected_key = table
            .selected_row()
            .and_then(|row| table.delegate().rows.get(row))
            .map(CellRow::row_key);
        let previous = table.selected_row();
        let delegate = table.delegate_mut();
        let defs = visible_columns(defs, state);
        delegate.columns = defs.iter().map(column_for).collect();
        delegate.keys = defs.iter().map(|def| def.key).collect();
        delegate.rows = rows;
        let selected =
            selected_key.and_then(|key| delegate.rows.iter().position(|row| row.row_key() == key));
        let sort = table_sort(&table.delegate().keys, state);
        table.set_sort(sort, cx);
        if selected != previous {
            table.set_selected_row(selected, cx);
        }
        table.refresh(cx);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use lunar_ui_logic::investigation::view::{ENDPOINT_COLUMNS, FLOW_COLUMNS};

    #[test]
    fn tri_montre_par_le_tableau() {
        let keys: Vec<SortKey> = FLOW_COLUMNS.iter().map(|c| c.key).collect();
        let mut state = ViewState::default();
        // Par défaut : volume décroissant, dernière colonne des flux.
        assert_eq!(
            table_sort(&keys, &state),
            Some(SortState {
                column: 6,
                direction: SortDirection::Descending
            })
        );
        state.sort_by(SortKey::Source);
        assert_eq!(
            table_sort(&keys, &state),
            Some(SortState {
                column: 0,
                direction: SortDirection::Ascending
            })
        );
        // Clé d'un autre tableau : aucune flèche.
        state.sort_by(SortKey::Label);
        assert_eq!(table_sort(&keys, &state), None);
    }

    #[test]
    fn en_tete_clique() {
        let keys: Vec<SortKey> = ENDPOINT_COLUMNS.iter().map(|c| c.key).collect();
        let proposed = Some(SortState {
            column: 4,
            direction: SortDirection::Ascending,
        });
        assert_eq!(clicked_key(&keys, proposed, SortKey::Bytes), SortKey::Label);
        // Retour « sans tri » du tableau : c'était la colonne triée.
        assert_eq!(clicked_key(&keys, None, SortKey::Bytes), SortKey::Bytes);
    }

    #[test]
    fn clic_sur_en_tete_alterne_croissant_et_decroissant() {
        // Le cycle du tableau, rapporté à `ViewState::sort_by`, alterne
        // croissant et décroissant.
        let keys: Vec<SortKey> = FLOW_COLUMNS.iter().map(|c| c.key).collect();
        let mut state = ViewState::default();
        let click = |state: &mut ViewState, column: usize| {
            let proposed = crate::widgets::table::cycle_sort(table_sort(&keys, state), column);
            let key = clicked_key(&keys, proposed, state.sort_key);
            state.sort_by(key);
        };
        click(&mut state, 6); // Volume, déjà trié décroissant
        assert_eq!((state.sort_key, state.sort_asc), (SortKey::Bytes, true));
        click(&mut state, 6);
        assert_eq!((state.sort_key, state.sort_asc), (SortKey::Bytes, false));
        click(&mut state, 6);
        assert_eq!((state.sort_key, state.sort_asc), (SortKey::Bytes, true));
        click(&mut state, 5); // Paquets : mesure, décroissant d'abord
        assert_eq!((state.sort_key, state.sort_asc), (SortKey::Packets, false));
        click(&mut state, 2); // Protocole : texte, croissant d'abord
        assert_eq!((state.sort_key, state.sort_asc), (SortKey::Protocol, true));
    }

    #[test]
    fn colonnes_masquees() {
        let mut state = ViewState::default();
        state.toggle_column(SortKey::Ports);
        state.toggle_column(SortKey::Bytes);
        let keys: Vec<SortKey> = visible_columns(&FLOW_COLUMNS, &state)
            .iter()
            .map(|c| c.key)
            .collect();
        assert_eq!(
            keys,
            [
                SortKey::Source,
                SortKey::Target,
                SortKey::Protocol,
                SortKey::Bidir,
                SortKey::Packets
            ]
        );
        // La colonne triée masquée : plus de flèche.
        assert_eq!(table_sort(&keys, &state), None);
    }

    #[test]
    fn colonnes_numeriques_a_droite() {
        for def in FLOW_COLUMNS.iter().chain(ENDPOINT_COLUMNS.iter()) {
            let column = column_for(def);
            assert!(column.sortable, "{} triable", def.label);
            assert_eq!(
                column.align == ColumnAlign::Right,
                def.key.is_measure(),
                "{}",
                def.label
            );
        }
    }
}
