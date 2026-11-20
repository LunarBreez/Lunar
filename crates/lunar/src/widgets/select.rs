//! Liste déroulante : bouton et liste flottante.
//!
//! Sombre comme les champs de saisie (`#12121e`, bordure `#2d2d50`, focus
//! `#4a7cff`), avec un chevron. Options désactivées grisées et sautées.
//!
//! Clavier, bouton focalisé : Entrée, Espace, flèches ouvrent la liste ;
//! liste ouverte : flèches, Début, Fin, Page préc./suiv. déplacent la
//! surbrillance, Entrée ou Espace choisissent, Échap ferme. Un clic hors de
//! la liste la ferme.

use gpui::{
    App, Context, CursorStyle, EventEmitter, FocusHandle, Focusable, IntoElement, MouseButton,
    Pixels, Render, ScrollHandle, SharedString, Window, anchored, deferred, div, point, prelude::*,
    px,
};

use crate::assets::{Icon, icon};
use crate::theme::ActiveTheme;
use crate::widgets::{
    Cancel, Confirm, MoveDown, MoveFirst, MoveLast, MoveUp, PageDown, PageUp, SELECT_CONTEXT,
    ellipsis, nav,
};

/// Priorité de peinture des listes flottantes (au-dessus des modales).
pub const POPOVER_PRIORITY: usize = 20;

/// Nombre d'options sautées par Page préc. / Page suiv.
const PAGE: usize = 8;

/// Option d'une liste déroulante.
#[derive(Clone, Debug)]
pub struct SelectOption {
    /// Libellé.
    pub label: SharedString,
    /// Détail affiché à droite, atténué (adresse d'une interface…).
    pub detail: Option<SharedString>,
    /// Icône à gauche.
    pub icon: Option<Icon>,
    /// Option désactivée.
    pub disabled: bool,
}

impl SelectOption {
    /// Option active.
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            detail: None,
            icon: None,
            disabled: false,
        }
    }

    /// Ajoute un détail à droite.
    pub fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Ajoute une icône.
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Désactive l'option.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl<T: Into<SharedString>> From<T> for SelectOption {
    fn from(label: T) -> Self {
        Self::new(label)
    }
}

/// Événement d'une liste déroulante.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SelectEvent {
    /// L'utilisateur a choisi l'option d'indice donné.
    Changed(usize),
}

/// État d'ouverture et de surbrillance, sans fenêtre.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SelectState {
    /// Liste ouverte.
    pub open: bool,
    /// Option en surbrillance.
    pub highlighted: Option<usize>,
}

impl SelectState {
    /// Ouvre la liste, surbrillance sur l'option choisie ou la première
    /// active.
    pub fn open(&mut self, enabled: &[bool], selected: Option<usize>) {
        self.open = true;
        self.highlighted = selected
            .filter(|ix| enabled.get(*ix).copied().unwrap_or(false))
            .or_else(|| nav::first(enabled));
    }

    /// Ferme la liste.
    pub fn close(&mut self) {
        self.open = false;
    }

    /// Déplace la surbrillance d'un cran (sans boucler).
    pub fn step(&mut self, enabled: &[bool], forward: bool) {
        self.highlighted = nav::step(enabled, self.highlighted, forward, false);
    }

    /// Surbrillance sur la première ou la dernière option active.
    pub fn jump(&mut self, enabled: &[bool], to_end: bool) {
        self.highlighted = if to_end {
            nav::last(enabled)
        } else {
            nav::first(enabled)
        };
    }

    /// Saut d'une page.
    pub fn page(&mut self, enabled: &[bool], forward: bool) {
        self.highlighted = nav::page(enabled, self.highlighted, forward, PAGE)
            .or_else(|| nav::step(enabled, None, forward, false));
    }

    /// Valide la surbrillance : ferme et renvoie l'option choisie.
    pub fn confirm(&mut self, enabled: &[bool]) -> Option<usize> {
        let choice = self
            .highlighted
            .filter(|ix| enabled.get(*ix).copied().unwrap_or(false));
        self.open = false;
        choice
    }
}

/// Liste déroulante, détenue dans une `Entity`.
///
/// ```ignore
/// let iface = cx.new(|cx| Select::new(["eth0", "wlan0"], cx).placeholder("Interface"));
/// cx.subscribe(&iface, |this, _, SelectEvent::Changed(ix), cx| { … }).detach();
/// ```
pub struct Select {
    /// Focus du bouton.
    focus_handle: FocusHandle,
    /// Options.
    options: Vec<SelectOption>,
    /// Option choisie.
    selected: Option<usize>,
    /// Texte du bouton sans choix.
    placeholder: SharedString,
    /// Liste désactivée.
    disabled: bool,
    /// Ouverture et surbrillance.
    state: SelectState,
    /// Largeur du bouton et de la liste.
    width: Pixels,
    /// Défilement de la liste ouverte.
    scroll: ScrollHandle,
}

impl EventEmitter<SelectEvent> for Select {}

impl Focusable for Select {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Select {
    /// Liste d'options, sans choix initial, 240 px de large.
    pub fn new<O: Into<SelectOption>>(
        options: impl IntoIterator<Item = O>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            focus_handle: cx.focus_handle().tab_stop(true),
            options: options.into_iter().map(Into::into).collect(),
            selected: None,
            placeholder: "Choisir…".into(),
            disabled: false,
            state: SelectState::default(),
            width: px(240.),
            scroll: ScrollHandle::new(),
        }
    }

    /// Texte affiché sans choix.
    pub fn placeholder(mut self, text: impl Into<SharedString>) -> Self {
        self.placeholder = text.into();
        self
    }

    /// Choix initial.
    pub fn selected(mut self, index: Option<usize>) -> Self {
        self.selected = index.filter(|ix| *ix < self.options.len());
        self
    }

    /// Largeur.
    pub fn width(mut self, width: Pixels) -> Self {
        self.width = width;
        self
    }

    /// Désactive la liste.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Option choisie.
    pub fn selected_index(&self) -> Option<usize> {
        self.selected
    }

    /// Options.
    pub fn options(&self) -> &[SelectOption] {
        &self.options
    }

    /// Liste ouverte.
    pub fn is_open(&self) -> bool {
        self.state.open
    }

    /// Choisit une option par programme ; n'émet pas d'événement.
    pub fn set_selected(&mut self, index: Option<usize>, cx: &mut Context<Self>) {
        self.selected = index.filter(|ix| *ix < self.options.len());
        cx.notify();
    }

    /// Remplace les options ; le choix est conservé s'il existe encore.
    pub fn set_options<O: Into<SelectOption>>(
        &mut self,
        options: impl IntoIterator<Item = O>,
        cx: &mut Context<Self>,
    ) {
        self.options = options.into_iter().map(Into::into).collect();
        self.selected = self.selected.filter(|ix| *ix < self.options.len());
        self.state.close();
        cx.notify();
    }

    /// Active ou désactive la liste.
    pub fn set_disabled(&mut self, disabled: bool, cx: &mut Context<Self>) {
        self.disabled = disabled;
        if disabled {
            self.state.close();
        }
        cx.notify();
    }

    /// Ouvre la liste par programme (galerie, essais).
    pub fn open(&mut self, cx: &mut Context<Self>) {
        if !self.disabled {
            let enabled = self.enabled();
            self.state.open(&enabled, self.selected);
            self.scroll_to_highlight();
            cx.notify();
        }
    }

    /// Ferme la liste.
    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.state.close();
        cx.notify();
    }

    /// Options actives.
    fn enabled(&self) -> Vec<bool> {
        self.options.iter().map(|o| !o.disabled).collect()
    }

    /// Fait défiler la liste jusqu'à la surbrillance.
    fn scroll_to_highlight(&self) {
        if let Some(ix) = self.state.highlighted {
            self.scroll.scroll_to_item(ix);
        }
    }

    /// Choisit une option et émet l'événement.
    fn choose(&mut self, index: usize, cx: &mut Context<Self>) {
        self.state.close();
        if self.selected != Some(index) {
            self.selected = Some(index);
            cx.emit(SelectEvent::Changed(index));
        }
        cx.notify();
    }

    /// Action [`MoveDown`].
    fn on_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        let enabled = self.enabled();
        if self.state.open {
            self.state.step(&enabled, true);
        } else {
            self.state.open(&enabled, self.selected);
        }
        self.scroll_to_highlight();
        cx.notify();
    }

    /// Action [`MoveUp`].
    fn on_up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        let enabled = self.enabled();
        if self.state.open {
            self.state.step(&enabled, false);
        } else {
            self.state.open(&enabled, self.selected);
        }
        self.scroll_to_highlight();
        cx.notify();
    }

    /// Action [`MoveFirst`].
    fn on_first(&mut self, _: &MoveFirst, _: &mut Window, cx: &mut Context<Self>) {
        if self.state.open {
            self.state.jump(&self.enabled(), false);
            self.scroll_to_highlight();
            cx.notify();
        } else {
            cx.propagate();
        }
    }

    /// Action [`MoveLast`].
    fn on_last(&mut self, _: &MoveLast, _: &mut Window, cx: &mut Context<Self>) {
        if self.state.open {
            self.state.jump(&self.enabled(), true);
            self.scroll_to_highlight();
            cx.notify();
        } else {
            cx.propagate();
        }
    }

    /// Action [`PageUp`].
    fn on_page_up(&mut self, _: &PageUp, _: &mut Window, cx: &mut Context<Self>) {
        if self.state.open {
            self.state.page(&self.enabled(), false);
            self.scroll_to_highlight();
            cx.notify();
        } else {
            cx.propagate();
        }
    }

    /// Action [`PageDown`].
    fn on_page_down(&mut self, _: &PageDown, _: &mut Window, cx: &mut Context<Self>) {
        if self.state.open {
            self.state.page(&self.enabled(), true);
            self.scroll_to_highlight();
            cx.notify();
        } else {
            cx.propagate();
        }
    }

    /// Action [`Confirm`].
    fn on_confirm(&mut self, _: &Confirm, _: &mut Window, cx: &mut Context<Self>) {
        let enabled = self.enabled();
        if self.state.open {
            match self.state.confirm(&enabled) {
                Some(ix) => self.choose(ix, cx),
                None => cx.notify(),
            }
        } else {
            self.state.open(&enabled, self.selected);
            self.scroll_to_highlight();
            cx.notify();
        }
    }

    /// Action [`Cancel`] : ferme la liste, sinon laisse remonter (modale).
    fn on_cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        if self.state.open {
            self.close(cx);
        } else {
            cx.propagate();
        }
    }
}

impl Render for Select {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // La liste se ferme quand le focus part ailleurs (Tab, clic).
        if self.state.open && !self.focus_handle.is_focused(window) {
            self.state.close();
        }
        let theme = cx.theme();
        let focused = !self.disabled && self.focus_handle.is_focused(window);
        let open = self.state.open;
        let current = self.selected.and_then(|ix| self.options.get(ix));
        let (label, label_color) = match current {
            Some(option) => (option.label.clone(), theme.text_field),
            None => (self.placeholder.clone(), theme.text_placeholder),
        };
        let border = if focused || open {
            theme.focus_ring
        } else {
            theme.border
        };
        let hover_border = theme.border_strong;
        let width = self.width;
        let height = theme.control_height;

        let mut trigger = div()
            .id("bouton")
            .flex()
            .items_center()
            .gap(theme.space_md)
            .w(width)
            .h(height)
            .pl(px(10.))
            .pr(px(8.))
            .rounded(theme.radius_md)
            .border_1()
            .border_color(border)
            .bg(theme.bg_field)
            .text_size(theme.text_md)
            .text_color(label_color)
            .when_some(current.and_then(|o| o.icon), |d, i| {
                d.child(icon(i).text_color(theme.text_muted))
            })
            .child(ellipsis(div().flex_1().min_w_0()).child(label))
            .child(
                icon(if open {
                    Icon::ChevronUp
                } else {
                    Icon::ChevronDown
                })
                .size(px(14.))
                .text_color(theme.text_muted),
            );

        if self.disabled {
            trigger = trigger
                .opacity(theme.disabled_opacity)
                .cursor(CursorStyle::OperationNotAllowed);
        } else {
            trigger = trigger
                .track_focus(&self.focus_handle)
                .key_context(SELECT_CONTEXT)
                .cursor(CursorStyle::PointingHand)
                .when(!focused && !open, |d| {
                    d.hover(move |s| s.border_color(hover_border))
                })
                .on_action(cx.listener(Self::on_down))
                .on_action(cx.listener(Self::on_up))
                .on_action(cx.listener(Self::on_first))
                .on_action(cx.listener(Self::on_last))
                .on_action(cx.listener(Self::on_page_up))
                .on_action(cx.listener(Self::on_page_down))
                .on_action(cx.listener(Self::on_confirm))
                .on_action(cx.listener(Self::on_cancel))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        if this.state.open {
                            this.close(cx);
                        } else {
                            this.open(cx);
                        }
                    }),
                );
        }

        let mut root = div().relative().flex_none().w(width).child(trigger);
        if open {
            let viewport = window.viewport_size();
            let highlighted = self.state.highlighted;
            let selected = self.selected;
            // `occlude` : le capteur de clics placé sous la liste ne doit
            // pas recevoir l'appui sur une option (il fermerait la liste
            // avant le relâchement, et le choix serait perdu).
            let mut list = div()
                .id("liste")
                .occlude()
                .track_scroll(&self.scroll)
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .w(width)
                .max_h(px(280.))
                .py(theme.space_xs)
                .rounded(theme.radius_md)
                .border_1()
                .border_color(theme.border_strong)
                .bg(theme.bg_raised)
                .shadow(theme.shadow_popover())
                .text_size(theme.text_md)
                .font_family(theme.font_family.clone());
            for (ix, option) in self.options.iter().enumerate() {
                let is_highlighted = highlighted == Some(ix);
                let is_selected = selected == Some(ix);
                let mut row = div()
                    .id(ix)
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(theme.space_md)
                    .h(px(28.))
                    .px(px(10.))
                    .text_color(if option.disabled {
                        theme.text_placeholder
                    } else {
                        theme.text
                    })
                    .when(is_highlighted, |d| d.bg(theme.bg_hover))
                    .child(div().w(px(14.)).flex_none().when(is_selected, |d| {
                        d.child(icon(Icon::Check).size(px(14.)).text_color(theme.accent))
                    }))
                    .when_some(option.icon, |d, i| {
                        d.child(icon(i).text_color(theme.text_muted))
                    })
                    .child(ellipsis(div().flex_1().min_w_0()).child(option.label.clone()))
                    .when_some(option.detail.clone(), |d, detail| {
                        d.child(
                            div()
                                .text_size(theme.text_sm)
                                .text_color(theme.text_muted)
                                .font_family(theme.font_mono.clone())
                                .child(detail),
                        )
                    });
                if option.disabled {
                    row = row.cursor(CursorStyle::OperationNotAllowed);
                } else {
                    row = row
                        .cursor(CursorStyle::PointingHand)
                        .on_mouse_move(cx.listener(move |this, _, _, cx| {
                            if this.state.highlighted != Some(ix) {
                                this.state.highlighted = Some(ix);
                                cx.notify();
                            }
                        }))
                        .on_click(cx.listener(move |this, _, _, cx| this.choose(ix, cx)));
                }
                list = list.child(row);
            }
            if self.options.is_empty() {
                list = list.child(
                    div()
                        .px(px(10.))
                        .py(theme.space_sm)
                        .text_color(theme.text_muted)
                        .child("Aucune option"),
                );
            }
            // Capteur de clics plein écran sous la liste : un clic dehors
            // ferme sans atteindre l'arrière-plan.
            let catcher = div()
                .id("hors-liste")
                .occlude()
                .w(viewport.width)
                .h(viewport.height)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| this.close(cx)),
                );
            root = root
                .child(
                    deferred(anchored().position(point(px(0.), px(0.))).child(catcher))
                        .with_priority(POPOVER_PRIORITY),
                )
                // Ancrage sous le bouton : `anchored` part de la position
                // de son parent, placé juste sous le bouton.
                .child(
                    div().absolute().top(height + px(4.)).left_0().child(
                        deferred(anchored().snap_to_window_with_margin(px(8.)).child(list))
                            .with_priority(POPOVER_PRIORITY + 1),
                    ),
                );
        }
        root
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ouverture_sur_le_choix_courant() {
        let enabled = [true, false, true];
        let mut state = SelectState::default();
        state.open(&enabled, Some(2));
        assert!(state.open);
        assert_eq!(state.highlighted, Some(2));
        state.open(&enabled, Some(1));
        assert_eq!(
            state.highlighted,
            Some(0),
            "choix désactivé : première active"
        );
    }

    #[test]
    fn fleches_et_validation() {
        let enabled = [true, false, true, true];
        let mut state = SelectState::default();
        state.open(&enabled, None);
        state.step(&enabled, true);
        assert_eq!(state.highlighted, Some(2), "saute l'option désactivée");
        state.step(&enabled, true);
        state.step(&enabled, true);
        assert_eq!(state.highlighted, Some(3), "s'arrête à la dernière");
        state.jump(&enabled, false);
        assert_eq!(state.highlighted, Some(0));
        state.page(&enabled, true);
        assert_eq!(state.highlighted, Some(3));
        assert_eq!(state.confirm(&enabled), Some(3));
        assert!(!state.open);
    }

    #[test]
    fn rien_a_valider() {
        let enabled = [false, false];
        let mut state = SelectState::default();
        state.open(&enabled, None);
        assert_eq!(state.highlighted, None);
        assert_eq!(state.confirm(&enabled), None);
    }
}
