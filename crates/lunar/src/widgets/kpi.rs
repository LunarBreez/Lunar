//! Tuile d'indicateur : grande valeur, libellé
//! en capitales et texte secondaire facultatif.
//!
//! Fond `#12102b`, bordure `#262a44`, rayon 8, valeur 20 px grasse en
//! `#84c3f7`, libellé 11 px en capitales à opacité .7 ; la variante
//! d'alerte passe la bordure en `#b5651d` et la valeur en
//! `#ffb74d`.

use gpui::{App, IntoElement, SharedString, Window, div, prelude::*, px};

use crate::theme::ActiveTheme;

/// Tuile d'indicateur.
///
/// ```ignore
/// Kpi::new("Flux", "1 284").detail("dont 12 nouveaux")
/// Kpi::new("Sans label", "37").warn(true)
/// ```
#[derive(IntoElement)]
pub struct Kpi {
    /// Libellé (affiché en capitales).
    label: SharedString,
    /// Valeur.
    value: SharedString,
    /// Texte secondaire sous le libellé.
    sub: Option<SharedString>,
    /// Variante d'alerte.
    warn: bool,
}

impl Kpi {
    /// Tuile avec libellé et valeur.
    pub fn new(label: impl Into<SharedString>, value: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            sub: None,
            warn: false,
        }
    }

    /// Texte secondaire.
    pub fn detail(mut self, sub: impl Into<SharedString>) -> Self {
        self.sub = Some(sub.into());
        self
    }

    /// Variante d'alerte.
    pub fn warn(mut self, warn: bool) -> Self {
        self.warn = warn;
        self
    }
}

impl RenderOnce for Kpi {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let (border, value_color) = if self.warn {
            (theme.kpi_warn_border, theme.kpi_warn_value)
        } else {
            (theme.border_card, theme.text_highlight)
        };
        div()
            .flex()
            .flex_col()
            .flex_none()
            .min_w(px(96.))
            .px(theme.space_xl)
            .py(theme.space_md)
            .gap(px(2.))
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(border)
            .bg(theme.kpi_bg)
            .child(
                div()
                    .text_size(theme.text_kpi)
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(value_color)
                    .child(self.value),
            )
            .child(
                div()
                    .text_size(theme.text_xs)
                    .text_color(theme.text.opacity(0.7))
                    .child(self.label.to_uppercase()),
            )
            .when_some(self.sub, |d, sub| {
                d.child(
                    div()
                        .text_size(theme.text_xs)
                        .text_color(theme.text_muted)
                        .child(sub),
                )
            })
    }
}
