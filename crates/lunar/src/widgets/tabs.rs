//! Barre d'onglets.
//!
//! Onglet actif : texte et soulignement de 2 px en `#84c3f7`. Chaque onglet
//! est un arrêt de tabulation, activé par clic, Entrée ou Espace.

use std::rc::Rc;

use gpui::{
    App, CursorStyle, ElementId, IntoElement, MouseButton, SharedString, Window, div, prelude::*,
    px,
};

use crate::theme::ActiveTheme;

/// Rappel de changement d'onglet : reçoit l'indice choisi.
pub type TabHandler = Rc<dyn Fn(usize, &mut Window, &mut App)>;

/// Onglet.
#[derive(Clone, Debug)]
pub struct Tab {
    /// Libellé.
    pub label: SharedString,
    /// Compteur affiché entre parenthèses, comme « Flux (n) ».
    pub count: Option<usize>,
    /// Onglet désactivé.
    pub disabled: bool,
}

impl<T: Into<SharedString>> From<T> for Tab {
    fn from(label: T) -> Self {
        Self {
            label: label.into(),
            count: None,
            disabled: false,
        }
    }
}

impl Tab {
    /// Désactive l'onglet.
    pub fn disabled(mut self) -> Self {
        self.disabled = true;
        self
    }

    /// Onglet avec compteur.
    pub fn with_count(label: impl Into<SharedString>, count: usize) -> Self {
        Self {
            count: Some(count),
            ..Self::from(label)
        }
    }
}

/// Barre d'onglets.
///
/// ```ignore
/// Tabs::new("vues", [Tab::from("Résumé"), Tab::with_count("Flux", 42)], self.tab)
///     .on_change(cx.listener(|this, ix: &usize, _, cx| { this.tab = *ix; cx.notify() }))
/// ```
#[derive(IntoElement)]
pub struct Tabs {
    /// Identifiant de la barre.
    id: ElementId,
    /// Onglets.
    tabs: Vec<Tab>,
    /// Indice de l'onglet actif.
    selected: usize,
    /// Rappel de changement.
    on_change: Option<TabHandler>,
}

impl Tabs {
    /// Barre d'onglets.
    pub fn new<T: Into<Tab>>(
        id: impl Into<ElementId>,
        tabs: impl IntoIterator<Item = T>,
        selected: usize,
    ) -> Self {
        Self {
            id: id.into(),
            tabs: tabs.into_iter().map(Into::into).collect(),
            selected,
            on_change: None,
        }
    }

    /// Rappel appelé avec l'indice de l'onglet choisi.
    pub fn on_change(mut self, handler: impl Fn(&usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(move |ix, window, cx| handler(&ix, window, cx)));
        self
    }
}

impl RenderOnce for Tabs {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let mut strip = div()
            .id(self.id.clone())
            .overflow_x_scroll()
            .flex()
            .items_end()
            .gap(theme.space_xs)
            .px(px(14.))
            .border_b_1()
            .border_color(theme.border_divider);
        for (ix, tab) in self.tabs.into_iter().enumerate() {
            let active = ix == self.selected;
            let text = match tab.count {
                Some(count) => format!("{} ({count})", tab.label),
                None => tab.label.to_string(),
            };
            let color = if active {
                theme.tab_active
            } else {
                theme.tab_text
            };
            let mut item = div()
                .id(ElementId::NamedChild(
                    Box::new(self.id.clone()),
                    format!("onglet-{ix}").into(),
                ))
                .px(px(14.))
                .pt(px(8.))
                .pb(px(6.))
                .border_b_2()
                .border_color(if active {
                    theme.tab_active
                } else {
                    gpui::transparent_black()
                })
                .text_size(theme.text_md)
                .text_color(color)
                .when(active, |d| d.font_weight(gpui::FontWeight::SEMIBOLD))
                .child(text);
            if tab.disabled {
                item = item
                    .opacity(theme.disabled_opacity)
                    .cursor(CursorStyle::OperationNotAllowed);
            } else {
                let hover = theme.text;
                let focus_ring = theme.focus_ring;
                item = item
                    .tab_index(0)
                    .cursor(CursorStyle::PointingHand)
                    .when(!active, |d| d.hover(move |s| s.text_color(hover)))
                    .focus(move |s| s.bg(focus_ring.opacity(0.18)))
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default());
                if let Some(handler) = self.on_change.clone() {
                    item = item.on_click(move |_, window, cx| handler(ix, window, cx));
                }
            }
            strip = strip.child(item);
        }
        strip
    }
}
