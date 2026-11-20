//! Barre de progression déterminée ou indéterminée.
//!
//! Barre d'import : hauteur 8 px, piste blanche à .15, valeur `#4299e1`,
//! rayon 4. La variante fine (3 px) sert à la jauge du canal.

use std::time::Duration;

use gpui::{
    Animation, AnimationExt, App, ElementId, IntoElement, SharedString, Window, div, prelude::*,
    px, relative,
};

use crate::theme::ActiveTheme;

/// Barre de progression.
///
/// ```ignore
/// ProgressBar::determinate("import", 0.42).label("42 %")
/// ProgressBar::indeterminate("lecture")
/// ```
#[derive(IntoElement)]
pub struct ProgressBar {
    /// Identifiant (nécessaire à l'animation indéterminée).
    id: ElementId,
    /// Avancement dans [0, 1] ; `None` pour une barre indéterminée.
    value: Option<f32>,
    /// Texte affiché au-dessus de la barre.
    label: Option<SharedString>,
    /// Variante fine (3 px).
    thin: bool,
}

impl ProgressBar {
    /// Barre déterminée ; `value` est borné à [0, 1].
    pub fn determinate(id: impl Into<ElementId>, value: f32) -> Self {
        Self {
            id: id.into(),
            value: Some(if value.is_finite() {
                value.clamp(0., 1.)
            } else {
                0.
            }),
            label: None,
            thin: false,
        }
    }

    /// Barre indéterminée : un segment parcourt la piste en boucle.
    pub fn indeterminate(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            value: None,
            label: None,
            thin: false,
        }
    }

    /// Texte affiché au-dessus de la barre.
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Variante fine (3 px).
    pub fn thin(mut self) -> Self {
        self.thin = true;
        self
    }
}

/// Largeur relative du segment d'une barre indéterminée.
const SEGMENT: f32 = 0.3;

impl RenderOnce for ProgressBar {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let height = if self.thin { px(3.) } else { px(8.) };
        let fill = theme.progress_fill;
        let track = div()
            .relative()
            .w_full()
            .h(height)
            .rounded(height / 2.)
            .overflow_hidden()
            .bg(theme.progress_track);
        let track = match self.value {
            Some(value) => track
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .h_full()
                        .w(relative(value))
                        .rounded(height / 2.)
                        .bg(fill),
                )
                .into_any_element(),
            None => track
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .h_full()
                        .w(relative(SEGMENT))
                        .rounded(height / 2.)
                        .bg(fill)
                        .with_animation(
                            self.id.clone(),
                            Animation::new(Duration::from_millis(1400)).repeat(),
                            |segment, delta| {
                                segment.left(relative(delta * (1. + SEGMENT) - SEGMENT))
                            },
                        ),
                )
                .into_any_element(),
        };
        div()
            .flex()
            .flex_col()
            .gap(theme.space_xs)
            .w_full()
            .when_some(self.label, |d, label| {
                d.child(
                    div()
                        .text_size(theme.text_sm)
                        .text_color(theme.text_muted)
                        .child(label),
                )
            })
            .child(track)
    }
}
