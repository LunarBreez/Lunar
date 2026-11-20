//! Tableau virtualisé sur `uniform_list` : seules les lignes visibles sont
//! construites, des milliers de lignes défilent sans à-coup.
//!
//! - colonnes : titre, largeur fixe ou proportionnelle, alignement, tri ;
//! - clic sur un en-tête triable : croissant, décroissant, sans tri (flèche) ;
//! - survol et sélection de ligne ; double clic ou Entrée : activation ;
//! - cellule personnalisée facultative (édition de label en place) ;
//! - texte d'état vide ; poignée de défilement déplaçable.
//!
//! Les données viennent d'un [`TableDelegate`], générique sur le type de
//! ligne. Le tri est calculé par le tableau (permutation stable) à partir
//! de [`TableDelegate::compare`].
//!
//! Style : en-tête `#12102b` en `#84c3f7` gras, séparateurs `#171531`,
//! cellules 13 px, marges `6px 10px`.
//!
//! Clavier (tableau focalisé) : haut/bas, Début/Fin, Page préc./suiv.
//! déplacent la sélection, Entrée active la ligne.

use std::cmp::Ordering;
use std::ops::Range;

use gpui::{
    AnyElement, App, Context, CursorStyle, Div, ElementId, EventEmitter, FocusHandle, Focusable,
    IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Render,
    ScrollStrategy, SharedString, Stateful, UniformListScrollHandle, Window, canvas, div, point,
    prelude::*, px, uniform_list,
};

use crate::assets::{Icon, icon};
use crate::theme::{ActiveTheme, Theme};
use crate::widgets::{
    Confirm, MoveDown, MoveFirst, MoveLast, MoveUp, PageDown, PageUp, TABLE_CONTEXT, ellipsis,
};

/// Sens du tri.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortDirection {
    /// Croissant.
    Ascending,
    /// Décroissant.
    Descending,
}

/// Tri courant : colonne et sens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SortState {
    /// Indice de colonne.
    pub column: usize,
    /// Sens.
    pub direction: SortDirection,
}

/// Tri suivant après un clic sur l'en-tête `column` : croissant, puis
/// décroissant, puis sans tri ; une autre colonne repart en croissant.
pub fn cycle_sort(current: Option<SortState>, column: usize) -> Option<SortState> {
    match current {
        Some(SortState {
            column: c,
            direction: SortDirection::Ascending,
        }) if c == column => Some(SortState {
            column,
            direction: SortDirection::Descending,
        }),
        Some(SortState {
            column: c,
            direction: SortDirection::Descending,
        }) if c == column => None,
        _ => Some(SortState {
            column,
            direction: SortDirection::Ascending,
        }),
    }
}

/// Ordre d'affichage des `len` lignes selon le tri ; tri stable (les
/// égalités gardent l'ordre des données), y compris en décroissant.
pub fn sorted_order(
    len: usize,
    sort: Option<SortState>,
    compare: impl Fn(usize, usize, usize) -> Ordering,
) -> Vec<usize> {
    let mut order: Vec<usize> = (0..len).collect();
    if let Some(sort) = sort {
        order.sort_by(|a, b| {
            let ordering = compare(*a, *b, sort.column);
            match sort.direction {
                SortDirection::Ascending => ordering,
                SortDirection::Descending => ordering.reverse(),
            }
        });
    }
    order
}

/// Alignement du contenu d'une colonne.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColumnAlign {
    /// À gauche.
    #[default]
    Left,
    /// Centré.
    Center,
    /// À droite (nombres).
    Right,
}

/// Largeur d'une colonne.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ColumnWidth {
    /// Largeur fixe.
    Fixed(Pixels),
    /// Part de la largeur restante (poids relatif).
    Flex(f32),
}

/// Définition d'une colonne.
#[derive(Clone, Debug)]
pub struct Column {
    /// Titre de l'en-tête.
    pub title: SharedString,
    /// Largeur.
    pub width: ColumnWidth,
    /// Alignement.
    pub align: ColumnAlign,
    /// Triable par clic sur l'en-tête.
    pub sortable: bool,
    /// Police à chasse fixe (adresses, ports).
    pub mono: bool,
}

impl Column {
    /// Colonne de largeur fixe, alignée à gauche, non triable.
    pub fn new(title: impl Into<SharedString>, width: Pixels) -> Self {
        Self {
            title: title.into(),
            width: ColumnWidth::Fixed(width),
            align: ColumnAlign::Left,
            sortable: false,
            mono: false,
        }
    }

    /// Colonne qui prend une part `weight` de la largeur restante.
    pub fn flex(title: impl Into<SharedString>, weight: f32) -> Self {
        Self {
            width: ColumnWidth::Flex(weight),
            ..Self::new(title, px(0.))
        }
    }

    /// Change l'alignement.
    pub fn align(mut self, align: ColumnAlign) -> Self {
        self.align = align;
        self
    }

    /// Aligne à droite (nombres).
    pub fn numeric(self) -> Self {
        self.align(ColumnAlign::Right)
    }

    /// Rend la colonne triable.
    pub fn sortable(mut self) -> Self {
        self.sortable = true;
        self
    }

    /// Police à chasse fixe.
    pub fn mono(mut self) -> Self {
        self.mono = true;
        self
    }
}

/// Source des données d'un tableau.
pub trait TableDelegate: 'static {
    /// Colonnes.
    fn columns(&self) -> &[Column];

    /// Nombre de lignes de données.
    fn row_count(&self) -> usize;

    /// Texte d'une cellule (ligne de données, colonne).
    fn cell_text(&self, row: usize, column: usize) -> SharedString;

    /// Comparaison de deux lignes pour le tri d'une colonne ; par défaut,
    /// ordre lexical des textes. À redéfinir pour les nombres.
    fn compare(&self, a: usize, b: usize, column: usize) -> Ordering {
        self.cell_text(a, column).cmp(&self.cell_text(b, column))
    }

    /// Élément personnalisé d'une cellule (champ d'édition…) ; `None` :
    /// texte de [`Self::cell_text`].
    fn render_cell(
        &self,
        _row: usize,
        _column: usize,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Option<AnyElement> {
        None
    }

    /// Ligne à mettre en alerte.
    fn row_warning(&self, _row: usize) -> bool {
        false
    }
}

/// Événements d'un tableau.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TableEvent {
    /// Sélection changée (ligne de données).
    SelectionChanged(Option<usize>),
    /// Ligne activée par double clic ou Entrée (ligne de données).
    Activated(usize),
    /// Tri changé.
    SortChanged(Option<SortState>),
}

/// Glissement de la poignée de défilement en cours.
#[derive(Clone, Copy, Debug)]
struct ThumbDrag {
    /// Ordonnée du pointeur au début du glissement.
    start_y: Pixels,
    /// Défilement au début du glissement (positif vers le bas).
    start_scroll: Pixels,
}

/// Largeur de la poignée de défilement.
const THUMB_WIDTH: Pixels = px(8.);
/// Hauteur minimale de la poignée.
const THUMB_MIN_HEIGHT: Pixels = px(24.);

/// Tableau, détenu dans une `Entity`.
///
/// ```ignore
/// let table = cx.new(|cx| Table::new(MesFlux::new(), cx).empty_text("Aucun flux"));
/// cx.subscribe(&table, |this, _, event: &TableEvent, cx| { … }).detach();
/// ```
pub struct Table<D: TableDelegate> {
    /// Source des données.
    delegate: D,
    /// Focus du tableau.
    focus_handle: FocusHandle,
    /// Ordre d'affichage : indice affiché → ligne de données.
    order: Vec<usize>,
    /// Inverse de `order` : ligne de données → indice affiché.
    position: Vec<usize>,
    /// Tri courant.
    sort: Option<SortState>,
    /// Ligne de données sélectionnée.
    selected: Option<usize>,
    /// Défilement de la liste.
    scroll: UniformListScrollHandle,
    /// Texte affiché sans ligne.
    empty_text: SharedString,
    /// Glissement de la poignée en cours.
    thumb_drag: Option<ThumbDrag>,
}

impl<D: TableDelegate> EventEmitter<TableEvent> for Table<D> {}

impl<D: TableDelegate> Focusable for Table<D> {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl<D: TableDelegate> Table<D> {
    /// Tableau sur une source de données, sans tri.
    pub fn new(delegate: D, cx: &mut Context<Self>) -> Self {
        let mut table = Self {
            delegate,
            focus_handle: cx.focus_handle().tab_stop(true),
            order: Vec::new(),
            position: Vec::new(),
            sort: None,
            selected: None,
            scroll: UniformListScrollHandle::new(),
            empty_text: "Aucune donnée".into(),
            thumb_drag: None,
        };
        table.recompute_order();
        table
    }

    /// Texte affiché sans ligne.
    pub fn empty_text(mut self, text: impl Into<SharedString>) -> Self {
        self.empty_text = text.into();
        self
    }

    /// Source des données.
    pub fn delegate(&self) -> &D {
        &self.delegate
    }

    /// Source des données, modifiable ; appeler [`Self::refresh`] ensuite si
    /// les lignes ont changé.
    pub fn delegate_mut(&mut self) -> &mut D {
        &mut self.delegate
    }

    /// Recalcule l'ordre après un changement des données.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.recompute_order();
        if self
            .selected
            .is_some_and(|row| row >= self.delegate.row_count())
        {
            self.selected = None;
            cx.emit(TableEvent::SelectionChanged(None));
        }
        cx.notify();
    }

    /// Tri courant.
    pub fn sort(&self) -> Option<SortState> {
        self.sort
    }

    /// Change le tri par programme ; n'émet pas d'événement.
    pub fn set_sort(&mut self, sort: Option<SortState>, cx: &mut Context<Self>) {
        self.sort = sort;
        self.recompute_order();
        cx.notify();
    }

    /// Ligne de données sélectionnée.
    pub fn selected_row(&self) -> Option<usize> {
        self.selected
    }

    /// Sélectionne une ligne de données par programme (et la montre).
    pub fn set_selected_row(&mut self, row: Option<usize>, cx: &mut Context<Self>) {
        self.selected = row.filter(|r| *r < self.delegate.row_count());
        self.reveal_selection();
        cx.notify();
    }

    /// Ordre d'affichage courant (indice affiché → ligne de données).
    pub fn display_order(&self) -> &[usize] {
        &self.order
    }

    /// Recalcule `order` et `position`.
    fn recompute_order(&mut self) {
        let delegate = &self.delegate;
        self.order = sorted_order(delegate.row_count(), self.sort, |a, b, c| {
            delegate.compare(a, b, c)
        });
        self.position = vec![0; self.order.len()];
        for (view_ix, row) in self.order.iter().enumerate() {
            self.position[*row] = view_ix;
        }
    }

    /// Fait défiler jusqu'à la ligne sélectionnée.
    fn reveal_selection(&self) {
        if let Some(view_ix) = self.selected.and_then(|row| self.position.get(row)) {
            self.scroll.scroll_to_item(*view_ix, ScrollStrategy::Top);
        }
    }

    /// Sélectionne l'indice affiché `view_ix`.
    fn select_view_index(&mut self, view_ix: usize, cx: &mut Context<Self>) {
        let Some(row) = self.order.get(view_ix).copied() else {
            return;
        };
        if self.selected != Some(row) {
            self.selected = Some(row);
            cx.emit(TableEvent::SelectionChanged(Some(row)));
        }
        self.reveal_selection();
        cx.notify();
    }

    /// Déplace la sélection de `delta` lignes affichées.
    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let len = self.order.len();
        if len == 0 {
            return;
        }
        let current = self
            .selected
            .and_then(|row| self.position.get(row))
            .copied();
        let target = match current {
            None if delta >= 0 => 0,
            None => len - 1,
            Some(ix) => (ix as isize + delta).clamp(0, len as isize - 1) as usize,
        };
        self.select_view_index(target, cx);
    }

    /// Lignes visibles par page.
    fn page_rows(&self, row_height: Pixels) -> isize {
        let height = self.scroll.0.borrow().base_handle.bounds().size.height;
        ((height / row_height).floor() as isize - 1).max(1)
    }

    /// Clic sur un en-tête triable.
    fn toggle_sort(&mut self, column: usize, cx: &mut Context<Self>) {
        self.sort = cycle_sort(self.sort, column);
        self.recompute_order();
        self.reveal_selection();
        cx.emit(TableEvent::SortChanged(self.sort));
        cx.notify();
    }

    /// Action [`MoveUp`].
    fn on_up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(-1, cx);
    }

    /// Action [`MoveDown`].
    fn on_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(1, cx);
    }

    /// Action [`MoveFirst`].
    fn on_first(&mut self, _: &MoveFirst, _: &mut Window, cx: &mut Context<Self>) {
        self.select_view_index(0, cx);
    }

    /// Action [`MoveLast`].
    fn on_last(&mut self, _: &MoveLast, _: &mut Window, cx: &mut Context<Self>) {
        let last = self.order.len().saturating_sub(1);
        self.select_view_index(last, cx);
    }

    /// Action [`PageUp`].
    fn on_page_up(&mut self, _: &PageUp, _: &mut Window, cx: &mut Context<Self>) {
        let page = self.page_rows(cx.theme().row_height);
        self.move_selection(-page, cx);
    }

    /// Action [`PageDown`].
    fn on_page_down(&mut self, _: &PageDown, _: &mut Window, cx: &mut Context<Self>) {
        let page = self.page_rows(cx.theme().row_height);
        self.move_selection(page, cx);
    }

    /// Action [`Confirm`] : active la ligne sélectionnée.
    fn on_confirm(&mut self, _: &Confirm, _: &mut Window, cx: &mut Context<Self>) {
        match self.selected {
            Some(row) => cx.emit(TableEvent::Activated(row)),
            None => cx.propagate(),
        }
    }

    /// Construit les lignes visibles.
    fn render_rows(
        &mut self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Stateful<Div>> {
        let theme = cx.theme().clone();
        let columns = self.delegate.columns().to_vec();
        let mut rows = Vec::with_capacity(range.len());
        for view_ix in range {
            let Some(row) = self.order.get(view_ix).copied() else {
                continue;
            };
            let selected = self.selected == Some(row);
            let warn = self.delegate.row_warning(row);
            let bg = if selected {
                theme.table_row_selected
            } else if warn {
                theme.table_row_warn
            } else {
                gpui::transparent_black()
            };
            let hover = theme.table_row_hover;
            let mut line = div()
                .id(view_ix)
                .flex()
                .items_center()
                .w_full()
                .h(theme.row_height)
                .border_b_1()
                .border_color(theme.table_row_border)
                .bg(bg)
                .when(!selected, |d| d.hover(move |s| s.bg(hover)))
                .on_click(cx.listener(move |this, event: &gpui::ClickEvent, _, cx| {
                    this.select_view_index(view_ix, cx);
                    if event.click_count() >= 2 {
                        cx.emit(TableEvent::Activated(row));
                    }
                }));
            for (col_ix, column) in columns.iter().enumerate() {
                let custom = self.delegate.render_cell(row, col_ix, window, cx);
                let cell = cell_box(column, &theme).h_full().text_color(theme.text);
                let cell = match custom {
                    Some(element) => cell.child(element),
                    None => cell.child(
                        ellipsis(div().w_full())
                            .map(|d| match column.align {
                                ColumnAlign::Left => d,
                                ColumnAlign::Center => d.text_center(),
                                ColumnAlign::Right => d.text_right(),
                            })
                            .child(self.delegate.cell_text(row, col_ix)),
                    ),
                };
                line = line.child(cell);
            }
            rows.push(line);
        }
        rows
    }

    /// En-tête des colonnes.
    fn render_header(&self, theme: &Theme, cx: &Context<Self>) -> impl IntoElement + use<D> {
        let mut header = div()
            .flex()
            .flex_none()
            .items_center()
            .w_full()
            .h(theme.row_height + px(4.))
            .bg(theme.table_header_bg)
            .border_b_1()
            .border_color(theme.table_header_border)
            .text_color(theme.table_header_text)
            .font_weight(gpui::FontWeight::SEMIBOLD);
        for (col_ix, column) in self.delegate.columns().iter().enumerate() {
            let sorted = self.sort.filter(|s| s.column == col_ix);
            let arrow = match sorted.map(|s| s.direction) {
                Some(SortDirection::Ascending) => Some((Icon::ArrowUp, theme.table_header_text)),
                Some(SortDirection::Descending) => Some((Icon::ArrowDown, theme.table_header_text)),
                None if column.sortable => {
                    Some((Icon::ArrowsSort, theme.table_header_text.opacity(0.35)))
                }
                None => None,
            };
            let content = div()
                .flex()
                .items_center()
                .gap(theme.space_xs)
                .min_w_0()
                .when(column.align == ColumnAlign::Right, |d| d.flex_row_reverse())
                // Titres courts : pas de troncature (avec `ellipsis`, le
                // titre de la colonne proportionnelle tombait à zéro).
                .child(div().flex_none().child(column.title.clone()))
                .when_some(arrow, |d, (kind, color)| {
                    d.child(icon(kind).size(px(13.)).text_color(color))
                });
            let cell = cell_box(column, theme)
                .h_full()
                .id(ElementId::named_usize("en-tete", col_ix))
                .child(content);
            let cell = if column.sortable {
                let focus_ring = theme.focus_ring;
                let hover = theme.table_row_hover;
                cell.tab_index(0)
                    .cursor(CursorStyle::PointingHand)
                    .hover(move |s| s.bg(hover))
                    .focus(move |s| s.border_1().border_color(focus_ring))
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_sort(col_ix, cx)))
            } else {
                cell
            };
            header = header.child(cell);
        }
        header
    }

    /// Poignée de défilement, déplaçable.
    fn render_scrollbar(
        &self,
        theme: &Theme,
        cx: &Context<Self>,
    ) -> Option<impl IntoElement + use<D>> {
        let (offset, viewport) = {
            let state = self.scroll.0.borrow();
            (state.base_handle.offset(), state.base_handle.bounds())
        };
        let view_h = viewport.size.height;
        let content_h = theme.row_height * self.order.len() as f32;
        if view_h <= px(0.) || content_h <= view_h {
            return None;
        }
        let max_scroll = content_h - view_h;
        let thumb_h = (view_h * (view_h / content_h)).max(THUMB_MIN_HEIGHT);
        let scroll = -offset.y;
        let top = (view_h - thumb_h) * (scroll / max_scroll).clamp(0., 1.);
        let dragging = self.thumb_drag.is_some();
        let color = if dragging {
            theme.scrollbar_thumb_hover
        } else {
            theme.scrollbar_thumb
        };
        let hover = theme.scrollbar_thumb_hover;
        let entity = cx.entity();
        let handle = self.scroll.0.borrow().base_handle.clone();
        Some(
            div()
                .id("poignee")
                .absolute()
                .top(top)
                .right(px(2.))
                .w(THUMB_WIDTH)
                .h(thumb_h)
                .rounded(THUMB_WIDTH / 2.)
                .bg(color)
                .hover(move |s| s.bg(hover))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        this.thumb_drag = Some(ThumbDrag {
                            start_y: event.position.y,
                            start_scroll: scroll,
                        });
                        cx.notify();
                    }),
                )
                .child(
                    // Écouteurs de fenêtre : le glissement continue hors de
                    // la poignée (même technique que `data_table.rs`).
                    canvas(
                        |_, _, _| (),
                        move |_, _, window, _| {
                            let entity_move = entity.clone();
                            let handle = handle.clone();
                            window.on_mouse_event(move |event: &MouseMoveEvent, _, _, cx| {
                                let Some(drag) = entity_move.read(cx).thumb_drag else {
                                    return;
                                };
                                if !event.dragging() {
                                    return;
                                }
                                let travel = view_h - thumb_h;
                                if travel <= px(0.) {
                                    return;
                                }
                                let delta = event.position.y - drag.start_y;
                                let scroll = (drag.start_scroll + max_scroll * (delta / travel))
                                    .clamp(px(0.), max_scroll);
                                handle.set_offset(point(px(0.), -scroll));
                                cx.notify(entity_move.entity_id());
                            });
                            let entity_up = entity.clone();
                            window.on_mouse_event(move |_: &MouseUpEvent, _, _, cx| {
                                entity_up.update(cx, |this, cx| {
                                    if this.thumb_drag.take().is_some() {
                                        cx.notify();
                                    }
                                });
                            });
                        },
                    )
                    .size_full(),
                ),
        )
    }
}

/// Boîte d'une cellule : largeur, alignement, marges, police.
fn cell_box(column: &Column, theme: &Theme) -> Div {
    let cell = div()
        .flex()
        .items_center()
        .px(px(10.))
        .overflow_hidden()
        .when(column.mono, |d| d.font_family(theme.font_mono.clone()))
        .map(|d| match column.align {
            ColumnAlign::Left => d.justify_start(),
            ColumnAlign::Center => d.justify_center(),
            ColumnAlign::Right => d.justify_end(),
        });
    match column.width {
        ColumnWidth::Fixed(width) => cell.w(width).flex_none(),
        ColumnWidth::Flex(weight) => {
            let mut cell = cell.flex_basis(px(0.)).min_w_0();
            cell.style().flex_grow = Some(weight);
            cell.style().flex_shrink = Some(1.);
            cell
        }
    }
}

impl<D: TableDelegate> Render for Table<D> {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let count = self.order.len();
        let header = self.render_header(&theme, cx);
        let body = if count == 0 {
            div()
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .text_color(theme.text_muted)
                .child(self.empty_text.clone())
                .into_any_element()
        } else {
            div()
                .relative()
                .flex_1()
                .min_h_0()
                .child(
                    uniform_list(
                        "lignes",
                        count,
                        cx.processor(|this, range: Range<usize>, window, cx| {
                            this.render_rows(range, window, cx)
                        }),
                    )
                    .size_full()
                    .track_scroll(self.scroll.clone()),
                )
                .children(self.render_scrollbar(&theme, cx))
                .into_any_element()
        };
        let focus_ring = theme.focus_ring;
        div()
            .id("tableau")
            .track_focus(&self.focus_handle)
            .key_context(TABLE_CONTEXT)
            .on_action(cx.listener(Self::on_up))
            .on_action(cx.listener(Self::on_down))
            .on_action(cx.listener(Self::on_first))
            .on_action(cx.listener(Self::on_last))
            .on_action(cx.listener(Self::on_page_up))
            .on_action(cx.listener(Self::on_page_down))
            .on_action(cx.listener(Self::on_confirm))
            .flex()
            .flex_col()
            .size_full()
            .overflow_hidden()
            .rounded(theme.radius_md)
            .border_1()
            .border_color(theme.border_card)
            .focus(move |s| s.border_color(focus_ring))
            .bg(theme.table_bg)
            .text_size(theme.text_md)
            .child(header)
            .child(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycle_du_tri() {
        let asc = |column| {
            Some(SortState {
                column,
                direction: SortDirection::Ascending,
            })
        };
        let desc = Some(SortState {
            column: 1,
            direction: SortDirection::Descending,
        });
        assert_eq!(cycle_sort(None, 1), asc(1));
        assert_eq!(cycle_sort(asc(1), 1), desc);
        assert_eq!(cycle_sort(desc, 1), None);
        assert_eq!(cycle_sort(desc, 2), asc(2), "autre colonne : croissant");
        assert_eq!(cycle_sort(asc(0), 2), asc(2));
    }

    #[test]
    fn ordre_stable() {
        let keys = [3, 1, 2, 1, 3];
        let cmp = |a: usize, b: usize, _| keys[a].cmp(&keys[b]);
        assert_eq!(sorted_order(5, None, cmp), vec![0, 1, 2, 3, 4]);
        let asc = Some(SortState {
            column: 0,
            direction: SortDirection::Ascending,
        });
        assert_eq!(sorted_order(5, asc, cmp), vec![1, 3, 2, 0, 4]);
        let desc = Some(SortState {
            column: 0,
            direction: SortDirection::Descending,
        });
        assert_eq!(
            sorted_order(5, desc, cmp),
            vec![0, 4, 2, 1, 3],
            "égalités dans l'ordre des données"
        );
    }
}
