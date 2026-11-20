//! Barre de menus d'application dessinée par gpui (Linux n'a pas de barre
//! native dans gpui 0.2.2).
//!
//! Sections à menus déroulants, séparateurs, raccourci aligné à droite,
//! entrées désactivées. Clavier : Alt seul ou F10 ouvrent la première
//! section (voir [`crate::widgets::root_actions`]), flèches gauche/droite
//! changent de section, haut/bas d'entrée, Début/Fin, Entrée ou Espace
//! activent, Échap ferme et rend le focus. Souris : clic sur un titre,
//! survol d'un autre titre menu ouvert, clic sur une entrée, clic dehors
//! pour fermer.
//!
//! Une entrée activée émet [`MenuBarEvent::Activated`] et appelle le rappel
//! de [`MenuBar::on_activate`] avec son identifiant d'action.
//!
//! Style : fond de la barre d'outils (`#070416`), séparateur `#252526`,
//! menus en `#252535` comme les listes déroulantes.

use std::rc::Rc;

use gpui::{
    App, Bounds, Context, CursorStyle, EventEmitter, FocusHandle, Focusable, IntoElement,
    MouseButton, Pixels, Render, SharedString, Window, anchored, canvas, deferred, div, point,
    prelude::*, px,
};

use crate::theme::ActiveTheme;
use crate::widgets::select::POPOVER_PRIORITY;
use crate::widgets::{
    Cancel, Confirm, MENU_BAR_CONTEXT, MoveDown, MoveFirst, MoveLast, MoveLeft, MoveRight, MoveUp,
    nav,
};

/// Entrée d'un menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuEntry {
    /// Entrée activable.
    Item {
        /// Identifiant d'action transmis à l'activation.
        id: &'static str,
        /// Libellé.
        label: SharedString,
        /// Raccourci affiché à droite (« Ctrl+O »).
        accelerator: Option<String>,
        /// Entrée active.
        enabled: bool,
    },
    /// Trait de séparation.
    Separator,
}

impl MenuEntry {
    /// Entrée active sans raccourci.
    pub fn item(id: &'static str, label: impl Into<SharedString>) -> Self {
        MenuEntry::Item {
            id,
            label: label.into(),
            accelerator: None,
            enabled: true,
        }
    }

    /// Ajoute le raccourci affiché.
    pub fn accelerator(mut self, text: impl Into<String>) -> Self {
        if let MenuEntry::Item { accelerator, .. } = &mut self {
            *accelerator = Some(text.into());
        }
        self
    }

    /// Active ou désactive l'entrée.
    pub fn enabled(mut self, value: bool) -> Self {
        if let MenuEntry::Item { enabled, .. } = &mut self {
            *enabled = value;
        }
        self
    }

    /// L'entrée est activable (ni séparateur ni désactivée).
    pub fn is_enabled(&self) -> bool {
        matches!(self, MenuEntry::Item { enabled: true, .. })
    }

    /// Identifiant d'action d'une entrée.
    pub fn id(&self) -> Option<&'static str> {
        match self {
            MenuEntry::Item { id, .. } => Some(id),
            MenuEntry::Separator => None,
        }
    }
}

/// Section de la barre : titre et entrées.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuSection {
    /// Titre (« Fichier »).
    pub label: SharedString,
    /// Entrées.
    pub items: Vec<MenuEntry>,
}

impl MenuSection {
    /// Section avec titre et entrées.
    pub fn new(label: impl Into<SharedString>, items: Vec<MenuEntry>) -> Self {
        Self {
            label: label.into(),
            items,
        }
    }

    /// Entrées activables.
    fn enabled(&self) -> Vec<bool> {
        self.items.iter().map(MenuEntry::is_enabled).collect()
    }
}

/// Événement de la barre de menus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuBarEvent {
    /// Une entrée a été activée.
    Activated(&'static str),
}

/// Section ouverte et entrée en surbrillance, sans fenêtre.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MenuBarState {
    /// Section ouverte.
    pub open: Option<usize>,
    /// Entrée en surbrillance dans la section ouverte.
    pub highlighted: Option<usize>,
}

impl MenuBarState {
    /// Ouvre une section ; `highlight_first` met la première entrée active
    /// en surbrillance (ouverture au clavier).
    pub fn open_section(&mut self, sections: &[MenuSection], ix: usize, highlight_first: bool) {
        let Some(section) = sections.get(ix) else {
            return;
        };
        self.open = Some(ix);
        self.highlighted = if highlight_first {
            nav::first(&section.enabled())
        } else {
            None
        };
    }

    /// Section voisine, en boucle ; garde la surbrillance au clavier.
    pub fn next_section(&mut self, sections: &[MenuSection], forward: bool) {
        if sections.is_empty() {
            return;
        }
        let current = self.open.unwrap_or(0);
        let next = if forward {
            (current + 1) % sections.len()
        } else {
            (current + sections.len() - 1) % sections.len()
        };
        self.open_section(sections, next, true);
    }

    /// Entrée voisine dans la section ouverte, en boucle.
    pub fn move_item(&mut self, sections: &[MenuSection], forward: bool) {
        if let Some(section) = self.open.and_then(|ix| sections.get(ix)) {
            self.highlighted = nav::step(&section.enabled(), self.highlighted, forward, true);
        }
    }

    /// Première ou dernière entrée active.
    pub fn jump(&mut self, sections: &[MenuSection], to_end: bool) {
        if let Some(section) = self.open.and_then(|ix| sections.get(ix)) {
            let enabled = section.enabled();
            self.highlighted = if to_end {
                nav::last(&enabled)
            } else {
                nav::first(&enabled)
            };
        }
    }

    /// Active l'entrée en surbrillance : ferme et renvoie son identifiant.
    pub fn activate(&mut self, sections: &[MenuSection]) -> Option<&'static str> {
        let entry = self
            .open
            .and_then(|ix| sections.get(ix))
            .zip(self.highlighted)
            .and_then(|(section, item)| section.items.get(item))
            .filter(|entry| entry.is_enabled())?;
        let id = entry.id();
        self.close();
        id
    }

    /// Ferme la section ouverte.
    pub fn close(&mut self) {
        self.open = None;
        self.highlighted = None;
    }
}

/// Rappel d'activation : reçoit l'identifiant d'action.
pub type ActivateHandler = Rc<dyn Fn(&'static str, &mut Window, &mut App)>;

/// Barre de menus, détenue dans une `Entity`.
///
/// ```ignore
/// let menu = cx.new(|cx| MenuBar::new(vec![
///     MenuSection::new("Fichier", vec![
///         MenuEntry::item("ouvrir", "Ouvrir…").accelerator("Ctrl+O"),
///         MenuEntry::Separator,
///         MenuEntry::item("quitter", "Quitter").accelerator("Ctrl+Q"),
///     ]),
/// ], cx));
/// ```
pub struct MenuBar {
    /// Focus de la barre pendant la navigation.
    focus_handle: FocusHandle,
    /// Sections.
    sections: Vec<MenuSection>,
    /// Ouverture et surbrillance.
    state: MenuBarState,
    /// Focus à rendre à la fermeture.
    previous_focus: Option<FocusHandle>,
    /// Rappel d'activation.
    on_activate: Option<ActivateHandler>,
    /// Rectangle de la barre au dernier rendu (zone exclue du capteur de
    /// clics extérieurs).
    bar_bounds: Bounds<Pixels>,
}

impl EventEmitter<MenuBarEvent> for MenuBar {}

impl Focusable for MenuBar {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl MenuBar {
    /// Barre avec ses sections.
    pub fn new(sections: Vec<MenuSection>, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            sections,
            state: MenuBarState::default(),
            previous_focus: None,
            on_activate: None,
            bar_bounds: Bounds::default(),
        }
    }

    /// Rappel appelé avec l'identifiant de l'entrée activée.
    pub fn on_activate(
        mut self,
        handler: impl Fn(&'static str, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_activate = Some(Rc::new(handler));
        self
    }

    /// Sections.
    pub fn sections(&self) -> &[MenuSection] {
        &self.sections
    }

    /// Remplace les sections (état des entrées changé…).
    pub fn set_sections(&mut self, sections: Vec<MenuSection>, cx: &mut Context<Self>) {
        self.sections = sections;
        self.state.close();
        cx.notify();
    }

    /// Active ou désactive toutes les entrées d'identifiant `id`.
    pub fn set_enabled(&mut self, id: &str, value: bool, cx: &mut Context<Self>) {
        for section in &mut self.sections {
            for entry in &mut section.items {
                if let MenuEntry::Item {
                    id: entry_id,
                    enabled,
                    ..
                } = entry
                    && *entry_id == id
                {
                    *enabled = value;
                }
            }
        }
        cx.notify();
    }

    /// Section ouverte.
    pub fn open_section_index(&self) -> Option<usize> {
        self.state.open
    }

    /// Alt / F10 : ouvre la première section avec sa première entrée en
    /// surbrillance, ou ferme la barre si elle est ouverte.
    pub fn open_from_keyboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.open.is_some() {
            self.close(window, cx);
            return;
        }
        self.open_section(0, true, window, cx);
    }

    /// Ouvre une section et prend le focus.
    pub fn open_section(
        &mut self,
        ix: usize,
        highlight_first: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.state.open.is_none() && !self.focus_handle.is_focused(window) {
            self.previous_focus = window.focused(cx);
        }
        self.state.open_section(&self.sections, ix, highlight_first);
        window.focus(&self.focus_handle);
        cx.notify();
    }

    /// Ferme la barre et rend le focus.
    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.state.close();
        match self.previous_focus.take() {
            Some(previous) => window.focus(&previous),
            None => window.blur(),
        }
        cx.notify();
    }

    /// Active l'entrée en surbrillance.
    fn activate_highlighted(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.state.activate(&self.sections) {
            self.close(window, cx);
            cx.emit(MenuBarEvent::Activated(id));
            if let Some(handler) = self.on_activate.clone() {
                handler(id, window, cx);
            }
        }
    }

    /// Action [`MoveLeft`].
    fn on_left(&mut self, _: &MoveLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.state.next_section(&self.sections, false);
        cx.notify();
    }

    /// Action [`MoveRight`].
    fn on_right(&mut self, _: &MoveRight, _: &mut Window, cx: &mut Context<Self>) {
        self.state.next_section(&self.sections, true);
        cx.notify();
    }

    /// Action [`MoveUp`].
    fn on_up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        self.state.move_item(&self.sections, false);
        cx.notify();
    }

    /// Action [`MoveDown`].
    fn on_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        self.state.move_item(&self.sections, true);
        cx.notify();
    }

    /// Action [`MoveFirst`].
    fn on_first(&mut self, _: &MoveFirst, _: &mut Window, cx: &mut Context<Self>) {
        self.state.jump(&self.sections, false);
        cx.notify();
    }

    /// Action [`MoveLast`].
    fn on_last(&mut self, _: &MoveLast, _: &mut Window, cx: &mut Context<Self>) {
        self.state.jump(&self.sections, true);
        cx.notify();
    }

    /// Action [`Confirm`].
    fn on_confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        self.activate_highlighted(window, cx);
    }

    /// Action [`Cancel`].
    fn on_cancel(&mut self, _: &Cancel, window: &mut Window, cx: &mut Context<Self>) {
        self.close(window, cx);
    }
}

impl Render for MenuBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Focus parti ailleurs (clic dans le contenu, Tab) : le menu se ferme.
        if self.state.open.is_some() && !self.focus_handle.is_focused(window) {
            self.state.close();
            self.previous_focus = None;
        }
        let theme = cx.theme();
        let bar_height = theme.menu_bar_height;
        let open = self.state.open;
        let entity = cx.entity();

        let mut bar = div()
            .id("barre-de-menus")
            .track_focus(&self.focus_handle)
            .key_context(MENU_BAR_CONTEXT)
            .on_action(cx.listener(Self::on_left))
            .on_action(cx.listener(Self::on_right))
            .on_action(cx.listener(Self::on_up))
            .on_action(cx.listener(Self::on_down))
            .on_action(cx.listener(Self::on_first))
            .on_action(cx.listener(Self::on_last))
            .on_action(cx.listener(Self::on_confirm))
            .on_action(cx.listener(Self::on_cancel))
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .w_full()
            .h(bar_height)
            .px(theme.space_xs)
            .bg(theme.bg_bar)
            .border_b_1()
            .border_color(theme.border_divider)
            .text_size(theme.text_md)
            .text_color(theme.text)
            .font_family(theme.font_family.clone())
            // Clic sur une zone vide de la barre : fermeture.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    if this.state.open.is_some() {
                        this.close(window, cx);
                    }
                }),
            )
            .child(
                canvas(
                    move |bounds, _, cx| {
                        entity.update(cx, |this, _| this.bar_bounds = bounds);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            );

        for (ix, section) in self.sections.iter().enumerate() {
            let is_open = open == Some(ix);
            let hover_bg = theme.bg_ghost_hover;
            let title = div()
                .id(("titre-menu", ix))
                .flex()
                .items_center()
                .h(bar_height - px(6.))
                .px(px(10.))
                .rounded(theme.radius_sm)
                .cursor(CursorStyle::PointingHand)
                .when(is_open, |d| d.bg(theme.bg_hover))
                .when(!is_open, |d| d.hover(move |s| s.bg(hover_bg)))
                .child(section.label.clone())
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        if this.state.open == Some(ix) {
                            this.close(window, cx);
                        } else {
                            this.open_section(ix, false, window, cx);
                        }
                    }),
                )
                .on_mouse_move(cx.listener(move |this, _, window, cx| {
                    if this.state.open.is_some() && this.state.open != Some(ix) {
                        this.open_section(ix, false, window, cx);
                    }
                }));
            let mut wrapper = div().relative().child(title);
            if is_open {
                wrapper = wrapper.child(
                    div().absolute().top(bar_height - px(3.)).left_0().child(
                        deferred(
                            anchored()
                                .snap_to_window_with_margin(px(8.))
                                .child(self.render_dropdown(ix, cx)),
                        )
                        .with_priority(POPOVER_PRIORITY + 1),
                    ),
                );
            }
            bar = bar.child(wrapper);
        }

        if open.is_some() {
            // Capteur de clics extérieurs, au-dessus et au-dessous de la
            // barre, pour laisser le survol des titres changer de section.
            let viewport = window.viewport_size();
            let bounds = self.bar_bounds;
            let above = bounds.top().max(px(0.));
            let below = bounds.bottom().max(px(0.));
            let close = |id: &'static str, top: Pixels, height: Pixels, cx: &Context<Self>| {
                deferred(
                    anchored().position(point(px(0.), top)).child(
                        div()
                            .id(id)
                            .occlude()
                            .w(viewport.width)
                            .h(height.max(px(0.)))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, window, cx| this.close(window, cx)),
                            ),
                    ),
                )
                .with_priority(POPOVER_PRIORITY)
            };
            bar = bar
                .child(close("hors-menu-haut", px(0.), above, cx))
                .child(close("hors-menu-bas", below, viewport.height - below, cx));
        }
        bar
    }
}

impl MenuBar {
    /// Menu déroulant de la section `ix`.
    fn render_dropdown(&self, ix: usize, cx: &Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let highlighted = self.state.highlighted;
        let mut menu = div()
            .id("menu-deroulant")
            .occlude()
            .flex()
            .flex_col()
            .min_w(px(240.))
            .py(theme.space_xs)
            .rounded(theme.radius_md)
            .border_1()
            .border_color(theme.border_strong)
            .bg(theme.bg_raised)
            .shadow(theme.shadow_popover())
            .text_size(theme.text_md);
        let Some(section) = self.sections.get(ix) else {
            return menu;
        };
        for (item_ix, entry) in section.items.iter().enumerate() {
            match entry {
                MenuEntry::Separator => {
                    menu = menu.child(
                        div()
                            .h(px(1.))
                            .mx(theme.space_md)
                            .my(theme.space_xs)
                            .bg(theme.border),
                    );
                }
                MenuEntry::Item {
                    label,
                    accelerator,
                    enabled,
                    ..
                } => {
                    let enabled = *enabled;
                    let is_highlighted = highlighted == Some(item_ix);
                    let mut row = div()
                        .id(("entree-menu", item_ix))
                        .flex()
                        .items_center()
                        .gap(theme.space_xxl)
                        .h(px(28.))
                        .px(theme.space_lg)
                        .mx(theme.space_xs)
                        .rounded(theme.radius_sm)
                        .text_color(if enabled {
                            theme.text
                        } else {
                            theme.text_placeholder
                        })
                        .when(is_highlighted && enabled, |d| d.bg(theme.bg_hover))
                        .child(div().flex_1().child(label.clone()))
                        .when_some(accelerator.clone(), |d, text| {
                            d.child(
                                div()
                                    .text_size(theme.text_sm)
                                    .text_color(if enabled {
                                        theme.text_muted
                                    } else {
                                        theme.text_placeholder
                                    })
                                    .child(text),
                            )
                        });
                    if enabled {
                        row = row
                            .cursor(CursorStyle::PointingHand)
                            .on_mouse_move(cx.listener(move |this, _, _, cx| {
                                if this.state.highlighted != Some(item_ix) {
                                    this.state.highlighted = Some(item_ix);
                                    cx.notify();
                                }
                            }))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.state.highlighted = Some(item_ix);
                                this.activate_highlighted(window, cx);
                            }));
                    } else {
                        row = row.cursor(CursorStyle::OperationNotAllowed);
                    }
                    menu = menu.child(row);
                }
            }
        }
        menu
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sections() -> Vec<MenuSection> {
        vec![
            MenuSection::new(
                "Fichier",
                vec![
                    MenuEntry::item("ouvrir", "Ouvrir…").accelerator("Ctrl+O"),
                    MenuEntry::Separator,
                    MenuEntry::item("exporter", "Exporter").enabled(false),
                    MenuEntry::item("quitter", "Quitter"),
                ],
            ),
            MenuSection::new("Capture", vec![MenuEntry::item("demarrer", "Démarrer")]),
            MenuSection::new("Aide", vec![]),
        ]
    }

    #[test]
    fn navigation_dans_une_section() {
        let s = sections();
        let mut state = MenuBarState::default();
        state.open_section(&s, 0, true);
        assert_eq!(state.highlighted, Some(0));
        state.move_item(&s, true);
        assert_eq!(
            state.highlighted,
            Some(3),
            "saute séparateur et entrée inactive"
        );
        state.move_item(&s, true);
        assert_eq!(state.highlighted, Some(0), "boucle");
        state.move_item(&s, false);
        assert_eq!(state.highlighted, Some(3));
        state.jump(&s, false);
        assert_eq!(state.highlighted, Some(0));
    }

    #[test]
    fn changement_de_section() {
        let s = sections();
        let mut state = MenuBarState::default();
        state.open_section(&s, 0, true);
        state.next_section(&s, false);
        assert_eq!(state.open, Some(2), "boucle vers la dernière");
        assert_eq!(state.highlighted, None, "section vide");
        state.next_section(&s, true);
        state.next_section(&s, true);
        assert_eq!(state.open, Some(1));
        assert_eq!(state.highlighted, Some(0));
    }

    #[test]
    fn activation() {
        let s = sections();
        let mut state = MenuBarState::default();
        state.open_section(&s, 0, false);
        assert_eq!(state.activate(&s), None, "rien en surbrillance");
        state.highlighted = Some(2);
        assert_eq!(state.activate(&s), None, "entrée désactivée");
        state.highlighted = Some(3);
        assert_eq!(state.activate(&s), Some("quitter"));
        assert_eq!(state.open, None);
    }

    #[test]
    fn constructeurs() {
        let entry = MenuEntry::item("x", "X").accelerator("F1").enabled(false);
        assert!(!entry.is_enabled());
        assert_eq!(entry.id(), Some("x"));
        assert!(!MenuEntry::Separator.is_enabled());
        assert_eq!(MenuEntry::Separator.accelerator("F2"), MenuEntry::Separator);
    }
}
