//! Case à cocher et interrupteur.
//!
//! Case de 14 × 14, cochée en `#4a7cff`. Les deux composants sont
//! focalisables (Tab) et basculent à Espace ou Entrée.

use std::rc::Rc;

use gpui::{
    App, CursorStyle, ElementId, IntoElement, MouseButton, SharedString, Window, div, prelude::*,
    px,
};

use crate::assets::{Icon, icon};
use crate::theme::ActiveTheme;

/// Rappel de changement d'état : reçoit le nouvel état.
pub type ToggleHandler = Rc<dyn Fn(bool, &mut Window, &mut App)>;

/// Case à cocher avec libellé.
///
/// ```ignore
/// Checkbox::new("ipv4", "IPv4 seulement", self.ipv4)
///     .on_change(cx.listener(|this, checked: &bool, _, cx| { this.ipv4 = *checked; cx.notify() }))
/// ```
#[derive(IntoElement)]
pub struct Checkbox {
    /// Identifiant de l'élément.
    id: ElementId,
    /// Libellé à droite de la case.
    label: Option<SharedString>,
    /// État coché.
    checked: bool,
    /// Désactivée.
    disabled: bool,
    /// Rappel de changement.
    on_change: Option<ToggleHandler>,
}

impl Checkbox {
    /// Case à cocher avec libellé.
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>, checked: bool) -> Self {
        Self {
            id: id.into(),
            label: Some(label.into()),
            checked,
            disabled: false,
            on_change: None,
        }
    }

    /// Case sans libellé (cellule de tableau).
    pub fn bare(id: impl Into<ElementId>, checked: bool) -> Self {
        Self {
            label: None,
            ..Self::new(id, "", checked)
        }
    }

    /// Désactive la case.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Rappel appelé avec le nouvel état.
    pub fn on_change(mut self, handler: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(move |value, window, cx| {
            handler(&value, window, cx)
        }));
        self
    }
}

impl RenderOnce for Checkbox {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let checked = self.checked;
        let accent = theme.focus_ring;
        let group: SharedString = format!("case-{:?}", self.id).into();
        let box_border = if checked { accent } else { theme.border_strong };
        let check_box = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(16.))
            .rounded(px(3.))
            .border_1()
            .border_color(box_border)
            .bg(if checked { accent } else { theme.bg_field })
            .when(!self.disabled, |d| {
                d.group_hover(group.clone(), |s| s.border_color(accent))
            })
            .when(checked, |d| {
                d.child(
                    icon(Icon::Check)
                        .size(px(12.))
                        .text_color(theme.text_on_fill),
                )
            });

        let row = div()
            .id(self.id)
            .group(group)
            .flex()
            .items_center()
            .gap(theme.space_md)
            .p(px(2.))
            .rounded(theme.radius_sm)
            .border_1()
            .border_color(gpui::transparent_black())
            .text_size(theme.text_md)
            .text_color(theme.text)
            .child(check_box)
            .when_some(self.label, |d, label| d.child(label));

        if self.disabled {
            return row
                .opacity(theme.disabled_opacity)
                .cursor(CursorStyle::OperationNotAllowed);
        }
        let focus_ring = theme.focus_ring;
        let mut row = row
            .tab_index(0)
            .cursor(CursorStyle::PointingHand)
            .focus(move |s| s.border_color(focus_ring))
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default());
        if let Some(handler) = self.on_change {
            row = row.on_click(move |_, window, cx| handler(!checked, window, cx));
        }
        row
    }
}

/// Interrupteur marche/arrêt avec libellé.
#[derive(IntoElement)]
pub struct Switch {
    /// Identifiant de l'élément.
    id: ElementId,
    /// Libellé à droite.
    label: Option<SharedString>,
    /// État activé.
    on: bool,
    /// Désactivé.
    disabled: bool,
    /// Rappel de changement.
    on_change: Option<ToggleHandler>,
}

impl Switch {
    /// Interrupteur avec libellé.
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>, on: bool) -> Self {
        Self {
            id: id.into(),
            label: Some(label.into()),
            on,
            disabled: false,
            on_change: None,
        }
    }

    /// Désactive l'interrupteur.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Rappel appelé avec le nouvel état.
    pub fn on_change(mut self, handler: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(move |value, window, cx| {
            handler(&value, window, cx)
        }));
        self
    }
}

impl RenderOnce for Switch {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let on = self.on;
        let track = div()
            .flex()
            .flex_none()
            .items_center()
            .w(px(30.))
            .h(px(16.))
            .px(px(2.))
            .rounded_full()
            .bg(if on {
                theme.accent
            } else {
                theme.button_bg_hover
            })
            .when(on, |d| d.justify_end())
            .child(div().size(px(12.)).rounded_full().bg(if on {
                theme.bg_surface
            } else {
                theme.text_muted
            }));
        let row = div()
            .id(self.id)
            .flex()
            .items_center()
            .gap(theme.space_md)
            .p(px(2.))
            .rounded(theme.radius_sm)
            .border_1()
            .border_color(gpui::transparent_black())
            .text_size(theme.text_md)
            .text_color(theme.text)
            .child(track)
            .when_some(self.label, |d, label| d.child(label));
        if self.disabled {
            return row
                .opacity(theme.disabled_opacity)
                .cursor(CursorStyle::OperationNotAllowed);
        }
        let focus_ring = theme.focus_ring;
        let mut row = row
            .tab_index(0)
            .cursor(CursorStyle::PointingHand)
            .focus(move |s| s.border_color(focus_ring))
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default());
        if let Some(handler) = self.on_change {
            row = row.on_click(move |_, window, cx| handler(!on, window, cx));
        }
        row
    }
}
