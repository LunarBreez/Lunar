//! Groupe de boutons radio (choix exclusif).
//!
//! Même style que la case à cocher (accent `#4a7cff`), en forme ronde.
//! Chaque option est un
//! arrêt de tabulation, choisie par clic, Espace ou Entrée.

use std::rc::Rc;

use gpui::{
    App, CursorStyle, ElementId, IntoElement, MouseButton, SharedString, Window, div, prelude::*,
    px,
};

use crate::theme::ActiveTheme;

/// Rappel de choix : reçoit l'indice de l'option choisie.
pub type SelectHandler = Rc<dyn Fn(usize, &mut Window, &mut App)>;

/// Option d'un groupe radio.
#[derive(Clone, Debug)]
pub struct RadioOption {
    /// Libellé.
    pub label: SharedString,
    /// Option désactivée.
    pub disabled: bool,
}

impl<T: Into<SharedString>> From<T> for RadioOption {
    fn from(label: T) -> Self {
        Self {
            label: label.into(),
            disabled: false,
        }
    }
}

/// Groupe de boutons radio, vertical par défaut.
///
/// ```ignore
/// RadioGroup::new("mode", ["Fichier PCAP", "Matrice de flux"], Some(self.mode))
///     .on_change(cx.listener(|this, ix: &usize, _, cx| { this.mode = *ix; cx.notify() }))
/// ```
#[derive(IntoElement)]
pub struct RadioGroup {
    /// Identifiant du groupe ; chaque option en dérive le sien.
    id: ElementId,
    /// Options.
    options: Vec<RadioOption>,
    /// Indice choisi.
    selected: Option<usize>,
    /// Options alignées horizontalement.
    horizontal: bool,
    /// Tout le groupe désactivé.
    disabled: bool,
    /// Rappel de choix.
    on_change: Option<SelectHandler>,
}

impl RadioGroup {
    /// Groupe d'options.
    pub fn new<O: Into<RadioOption>>(
        id: impl Into<ElementId>,
        options: impl IntoIterator<Item = O>,
        selected: Option<usize>,
    ) -> Self {
        Self {
            id: id.into(),
            options: options.into_iter().map(Into::into).collect(),
            selected,
            horizontal: false,
            disabled: false,
            on_change: None,
        }
    }

    /// Désactive une option.
    pub fn disable_option(mut self, index: usize) -> Self {
        if let Some(option) = self.options.get_mut(index) {
            option.disabled = true;
        }
        self
    }

    /// Aligne les options horizontalement.
    pub fn horizontal(mut self) -> Self {
        self.horizontal = true;
        self
    }

    /// Désactive tout le groupe.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Rappel appelé avec l'indice de l'option choisie.
    pub fn on_change(mut self, handler: impl Fn(&usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(move |ix, window, cx| handler(&ix, window, cx)));
        self
    }
}

impl RenderOnce for RadioGroup {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let accent = theme.focus_ring;
        let mut group = div()
            .flex()
            .when(!self.horizontal, |d| d.flex_col())
            .when(self.horizontal, |d| d.flex_wrap())
            .gap(if self.horizontal {
                theme.space_lg
            } else {
                theme.space_xs
            });
        for (ix, option) in self.options.into_iter().enumerate() {
            let checked = self.selected == Some(ix);
            let disabled = self.disabled || option.disabled;
            let hover_group: SharedString = format!("radio-{:?}-{ix}", self.id).into();
            let dot = div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(16.))
                .rounded_full()
                .border_1()
                .border_color(if checked { accent } else { theme.border_strong })
                .bg(theme.bg_field)
                .when(!disabled, |d| {
                    d.group_hover(hover_group.clone(), |s| s.border_color(accent))
                })
                .when(checked, |d| {
                    d.child(div().size(px(8.)).rounded_full().bg(accent))
                });
            let row = div()
                .id(ElementId::NamedChild(
                    Box::new(self.id.clone()),
                    format!("option-{ix}").into(),
                ))
                .group(hover_group)
                .flex()
                .items_center()
                .gap(theme.space_md)
                .p(px(2.))
                .rounded(theme.radius_sm)
                .border_1()
                .border_color(gpui::transparent_black())
                .text_size(theme.text_md)
                .text_color(theme.text)
                .child(dot)
                .child(option.label);
            let row = if disabled {
                row.opacity(theme.disabled_opacity)
                    .cursor(CursorStyle::OperationNotAllowed)
            } else {
                let focus_ring = theme.focus_ring;
                let mut row = row
                    .tab_index(0)
                    .cursor(CursorStyle::PointingHand)
                    .focus(move |s| s.border_color(focus_ring))
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default());
                if let Some(handler) = self.on_change.clone() {
                    row = row.on_click(move |_, window, cx| handler(ix, window, cx));
                }
                row
            };
            group = group.child(row);
        }
        group
    }
}
