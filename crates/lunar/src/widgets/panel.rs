//! Cadre de panneau latéral : barre de titre, bouton de fermeture, corps
//! défilant, pied facultatif.
//!
//! Les panneaux (filtre, labels, configuration) s'ouvrent à droite du
//! graphe. Surface `#1e1e2e`, bordure `#2d2d50`.

use std::rc::Rc;

use gpui::{
    AnyElement, App, ClickEvent, ElementId, IntoElement, ParentElement, Pixels, SharedString,
    Window, div, prelude::*, px,
};

use crate::assets::Icon;
use crate::theme::ActiveTheme;
use crate::widgets::button::{Button, ButtonSize, ClickHandler};

/// Panneau latéral.
///
/// ```ignore
/// Panel::new("filtre", "Filtres")
///     .on_close(cx.listener(|this, _, _, cx| { this.panel = None; cx.notify() }))
///     .child(…)
///     .footer(Button::new("appliquer", "Appliquer").primary())
/// ```
#[derive(IntoElement)]
pub struct Panel {
    /// Identifiant du panneau.
    id: ElementId,
    /// Titre.
    title: SharedString,
    /// Largeur.
    width: Pixels,
    /// Contenu du corps.
    children: Vec<AnyElement>,
    /// Pied (boutons d'action), aligné à droite.
    footer: Vec<AnyElement>,
    /// Rappel du bouton de fermeture ; sans rappel, pas de bouton.
    on_close: Option<ClickHandler>,
}

impl Panel {
    /// Panneau de 360 px de large.
    pub fn new(id: impl Into<ElementId>, title: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            width: px(360.),
            children: Vec::new(),
            footer: Vec::new(),
            on_close: None,
        }
    }

    /// Change la largeur.
    pub fn width(mut self, width: Pixels) -> Self {
        self.width = width;
        self
    }

    /// Ajoute un élément au pied.
    pub fn footer(mut self, element: impl IntoElement) -> Self {
        self.footer.push(element.into_any_element());
        self
    }

    /// Affiche le bouton de fermeture et son rappel.
    pub fn on_close(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_close = Some(Rc::new(handler));
        self
    }
}

impl ParentElement for Panel {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for Panel {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let close = self.on_close.map(|handler| {
            Button::icon_only(
                ElementId::NamedChild(Box::new(self.id.clone()), "fermer".into()),
                Icon::Close,
                "Fermer (Échap)",
            )
            .ghost()
            .size(ButtonSize::Small)
            .on_click(move |event, window, cx| handler(event, window, cx))
        });
        div()
            .id(self.id.clone())
            .flex()
            .flex_col()
            .flex_none()
            .w(self.width)
            .h_full()
            .bg(theme.bg_surface)
            .border_l_1()
            .border_color(theme.border)
            .shadow(theme.shadow_popover())
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_between()
                    .h(px(40.))
                    .pl(theme.space_xl)
                    .pr(theme.space_md)
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .text_size(theme.text_lg)
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(theme.text_heading)
                            .child(self.title),
                    )
                    .children(close),
            )
            .child(
                div()
                    .id(ElementId::NamedChild(Box::new(self.id), "corps".into()))
                    .flex()
                    .flex_col()
                    .flex_1()
                    .gap(theme.space_lg)
                    .p(theme.space_xl)
                    .overflow_y_scroll()
                    .text_size(theme.text_md)
                    .text_color(theme.text)
                    .children(self.children),
            )
            .when(!self.footer.is_empty(), |d| {
                d.child(
                    div()
                        .flex()
                        .flex_none()
                        .justify_end()
                        .gap(theme.space_md)
                        .p(theme.space_lg)
                        .border_t_1()
                        .border_color(theme.border)
                        .children(self.footer),
                )
            })
    }
}
