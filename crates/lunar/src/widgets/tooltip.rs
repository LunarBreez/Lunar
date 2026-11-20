//! Bulle d'aide au survol, pour `StatefulInteractiveElement::tooltip`.
//!
//! Fond `bg_tooltip`, bordure `border_tooltip` du thème.
//!
//! ```ignore
//! div().id("x").tooltip(tooltip("Démarrer (Ctrl+P)"))
//! div().id("y").tooltip(tooltip_with_shortcut("Filtrer", "Ctrl+F"))
//! ```

use gpui::{AnyView, App, Context, IntoElement, Render, SharedString, Window, div, prelude::*, px};

use crate::theme::ActiveTheme;

/// Vue d'une bulle : texte et raccourci facultatif.
pub struct Tooltip {
    /// Texte principal.
    text: SharedString,
    /// Raccourci clavier affiché à droite, atténué.
    shortcut: Option<SharedString>,
}

impl Tooltip {
    /// Crée la vue d'une bulle.
    pub fn new(text: impl Into<SharedString>, shortcut: Option<SharedString>) -> Self {
        Self {
            text: text.into(),
            shortcut,
        }
    }
}

impl Render for Tooltip {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        // Marge autour de la bulle : gpui la place sous le pointeur.
        div().pl(px(10.)).pt(px(14.)).child(
            div()
                .flex()
                .gap(theme.space_md)
                .items_center()
                .max_w(px(320.))
                .px(px(10.))
                .py(px(6.))
                .rounded(theme.radius_md)
                .border_1()
                .border_color(theme.border_tooltip)
                .bg(theme.bg_tooltip)
                .shadow(theme.shadow_popover())
                .text_size(theme.text_sm)
                .text_color(theme.text)
                .font_family(theme.font_family.clone())
                .child(self.text.clone())
                .when_some(self.shortcut.clone(), |d, shortcut| {
                    d.child(div().text_color(theme.text_muted).child(shortcut))
                }),
        )
    }
}

/// Fabrique de bulle simple, à passer à `.tooltip(…)`.
pub fn tooltip(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView {
    let text = text.into();
    move |_, cx| cx.new(|_| Tooltip::new(text.clone(), None)).into()
}

/// Fabrique de bulle avec raccourci clavier, à passer à `.tooltip(…)`.
pub fn tooltip_with_shortcut(
    text: impl Into<SharedString>,
    shortcut: impl Into<SharedString>,
) -> impl Fn(&mut Window, &mut App) -> AnyView {
    let text = text.into();
    let shortcut = shortcut.into();
    move |_, cx| {
        cx.new(|_| Tooltip::new(text.clone(), Some(shortcut.clone())))
            .into()
    }
}
