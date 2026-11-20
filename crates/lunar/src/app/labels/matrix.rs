//! Labels appliqués à la matrice, ouvert depuis le graphe : triplets (MAC, IP, label) lus à l'ouverture, recherche plein
//! texte. Un clic sur le voile ou Échap le ferme.

use gpui::{
    AppContext, Context, Entity, FocusHandle, IntoElement, ParentElement, Render, Styled,
    Subscription, Window, div, px,
};
use log::error;
use lunar_ui_logic::label_management::{
    MATRIX_LABELS_EMPTY, SEARCH_PLACEHOLDER, matrix_labels_title,
};

use crate::app::labels::rows::TripletRows;
use crate::app::panels::Panel;
use crate::app::state::AppState;
use crate::theme::ActiveTheme;
use crate::widgets::modal::Modal;
use crate::widgets::table::Table;
use crate::widgets::text_input::{TextInput, TextInputEvent};

/// Ce panneau.
const PANEL: Panel = Panel::MatrixLabels;

/// Panneau des labels de la matrice.
pub struct MatrixLabelsPanel {
    /// État de l'application.
    state: Entity<AppState>,
    /// Recherche.
    search: Entity<TextInput>,
    /// Labels.
    table: Entity<Table<TripletRows>>,
    /// Focus rendu à la fermeture.
    return_focus: Option<FocusHandle>,
    /// Focus à placer dans la recherche au premier rendu.
    focus_search: bool,
    /// Abonnement à la recherche.
    _subscription: Subscription,
}

impl MatrixLabelsPanel {
    /// Panneau ouvert : labels lus, focus dans la recherche.
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| TextInput::new(cx).placeholder(SEARCH_PLACEHOLDER));
        let table = cx.new(|cx| {
            Table::new(TripletRows::new(), cx)
                .empty_text("Aucun label ne correspond à la recherche.")
        });
        let subscription = cx.subscribe(&search, |this, _, event: &TextInputEvent, cx| {
            if let TextInputEvent::Changed(text) = event {
                this.table.update(cx, |table, cx| {
                    table.delegate_mut().set_search(text);
                    table.refresh(cx);
                });
            }
        });
        let return_focus = window.focused(cx);
        let bridge = state.read(cx).bridge().clone();
        bridge
            .run(
                cx,
                |lunar| lunar.get_matrix_labels(),
                |this, labels, cx| {
                    let labels = labels.unwrap_or_else(|err| {
                        error!("Erreur get_matrix_labels: {err}");
                        Vec::new()
                    });
                    this.table.update(cx, |table, cx| {
                        table.delegate_mut().set_rows(labels);
                        table.refresh(cx);
                    });
                    cx.notify();
                },
            )
            .detach();
        Self {
            state,
            search,
            table,
            return_focus,
            focus_search: true,
            _subscription: subscription,
        }
    }

    /// Ferme le panneau et rend le focus à son détenteur précédent.
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(focus) = &self.return_focus {
            window.focus(focus);
        }
        self.state
            .update(cx, |state, cx| state.close_panel(PANEL, cx));
    }
}

impl Render for MatrixLabelsPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.focus_search) {
            // La modale prend le focus par un `defer` de son premier rendu ;
            // un second `defer`, mis en file après le sien, le donne à la
            // recherche.
            let search = self.search.clone();
            window.defer(cx, move |window, cx| {
                window.defer(cx, move |window, cx| search.read(cx).focus(window));
            });
        }
        let theme = cx.theme().clone();
        let total = self.table.read(cx).delegate().total();
        let body = if total == 0 {
            div()
                .py(theme.space_lg)
                .text_center()
                .text_color(theme.text_muted)
                .child(MATRIX_LABELS_EMPTY)
                .into_any_element()
        } else {
            div()
                .h(px(420.))
                .child(self.table.clone())
                .into_any_element()
        };
        Modal::new("panneau-labels-matrice", matrix_labels_title(total))
            .width(PANEL.width())
            .close_on_backdrop(true)
            .on_close(cx.listener(|this, _: &(), window, cx| this.close(window, cx)))
            .child(self.search.clone())
            .child(body)
    }
}
