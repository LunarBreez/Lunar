//! Dialogue modal centré sur un voile, et confirmation OK / Annuler.
//!
//! Surface `#1e1e2e`, bordure `#2d2d50`, rayon 10, voile noir à .7.
//! [`Confirm`] sert à toutes les confirmations de l'application.
//!
//! Comportement :
//! - le dialogue est peint au-dessus de tout (`deferred`), le voile bloque la
//!   souris vers l'arrière-plan ;
//! - à l'ouverture, le focus passe au dialogue ; Tab et Maj+Tab tournent
//!   parmi ses seuls contrôles (piège à focus) ;
//! - Échap appelle `on_close` et rend le focus à l'élément qui l'avait avant ;
//! - Entrée appelle `on_confirm` quand le focus est sur le dialogue lui-même
//!   (un bouton focalisé garde sa propre action).
//!
//! Le parent décide de l'affichage : il ne rend la modale que si elle est
//! ouverte, et la ferme dans `on_close`.

use std::rc::Rc;

use gpui::{
    AnyElement, App, ElementId, FocusHandle, IntoElement, ParentElement, Pixels, SharedString,
    Window, anchored, deferred, div, point, prelude::*, px,
};

use crate::assets::Icon;
use crate::theme::ActiveTheme;
use crate::widgets::button::{Button, ButtonSize};
use crate::widgets::{Cancel, Confirm as ConfirmAction, FocusNext, FocusPrev, MODAL_CONTEXT};

/// Rappel sans argument d'une modale.
pub type ModalHandler = Rc<dyn Fn(&mut Window, &mut App)>;

/// Priorité de peinture des modales parmi les éléments différés.
pub const MODAL_PRIORITY: usize = 10;

/// Nombre maximal de sauts de focus pour retrouver un contrôle du dialogue.
const MAX_FOCUS_HOPS: usize = 256;

/// État conservé d'un rendu à l'autre tant que la modale est affichée.
struct ModalState {
    /// Focus du dialogue.
    focus: FocusHandle,
    /// Focus à rendre à la fermeture par Échap.
    previous: Option<FocusHandle>,
    /// Le focus initial a été posé.
    initialized: bool,
}

/// Dialogue modal.
///
/// ```ignore
/// Modal::new("config", "Configuration")
///     .on_close(cx.listener(|this, _: &(), _, cx| { this.config_open = false; cx.notify() }))
///     .child(…)
///     .footer(Button::new("enregistrer", "Enregistrer").primary())
/// ```
#[derive(IntoElement)]
pub struct Modal {
    /// Identifiant, clé de l'état conservé.
    id: ElementId,
    /// Titre.
    title: SharedString,
    /// Largeur du dialogue.
    width: Pixels,
    /// Corps.
    children: Vec<AnyElement>,
    /// Pied (boutons), aligné à droite.
    footer: Vec<AnyElement>,
    /// Fermeture : Échap, bouton ✕, clic sur le voile si permis.
    on_close: Option<ModalHandler>,
    /// Validation : Entrée quand le dialogue lui-même a le focus.
    on_confirm: Option<ModalHandler>,
    /// Un clic sur le voile ferme la modale.
    close_on_backdrop: bool,
}

impl Modal {
    /// Modale de 480 px de large.
    pub fn new(id: impl Into<ElementId>, title: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            width: px(480.),
            children: Vec::new(),
            footer: Vec::new(),
            on_close: None,
            on_confirm: None,
            close_on_backdrop: false,
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

    /// Rappel de fermeture (Échap, ✕, voile) ; affiche le bouton ✕.
    pub fn on_close(mut self, handler: impl Fn(&(), &mut Window, &mut App) + 'static) -> Self {
        self.on_close = Some(Rc::new(move |window, cx| handler(&(), window, cx)));
        self
    }

    /// Rappel de validation par Entrée.
    pub fn on_confirm(mut self, handler: impl Fn(&(), &mut Window, &mut App) + 'static) -> Self {
        self.on_confirm = Some(Rc::new(move |window, cx| handler(&(), window, cx)));
        self
    }

    /// Un clic sur le voile ferme la modale.
    pub fn close_on_backdrop(mut self, close: bool) -> Self {
        self.close_on_backdrop = close;
        self
    }
}

impl ParentElement for Modal {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

/// Déplace le focus vers le contrôle suivant ou précédent du dialogue,
/// en sautant ceux de l'arrière-plan.
fn cycle_focus(focus: &FocusHandle, forward: bool, window: &mut Window, cx: &mut App) {
    for _ in 0..MAX_FOCUS_HOPS {
        if forward {
            window.focus_next();
        } else {
            window.focus_prev();
        }
        if focus.contains_focused(window, cx) && !focus.is_focused(window) {
            return;
        }
    }
    // Aucun contrôle focalisable dans le dialogue : le focus y reste.
    window.focus(focus);
}

impl RenderOnce for Modal {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.id.clone(), cx, |window, cx| ModalState {
            focus: cx.focus_handle(),
            previous: window.focused(cx),
            initialized: false,
        });
        let (focus, initialized) = {
            let state = state.read(cx);
            (state.focus.clone(), state.initialized)
        };
        if !initialized {
            state.update(cx, |state, _| state.initialized = true);
            let focus = focus.clone();
            window.defer(cx, move |window, _| window.focus(&focus));
        }

        let theme = cx.theme();
        let viewport = window.viewport_size();
        let on_close = self.on_close.clone();
        let close_button = self.on_close.clone().map(|handler| {
            Button::icon_only(
                ElementId::NamedChild(Box::new(self.id.clone()), "fermer".into()),
                Icon::Close,
                "Fermer (Échap)",
            )
            .ghost()
            .size(ButtonSize::Small)
            .on_click(move |_, window, cx| handler(window, cx))
        });

        let dialog = div()
            .id(ElementId::NamedChild(
                Box::new(self.id.clone()),
                "dialogue".into(),
            ))
            .track_focus(&focus)
            .key_context(MODAL_CONTEXT)
            .on_action({
                let state = state.clone();
                move |_: &Cancel, window: &mut Window, cx: &mut App| {
                    if let Some(previous) = state.read(cx).previous.clone() {
                        window.focus(&previous);
                    }
                    if let Some(on_close) = on_close.as_ref() {
                        on_close(window, cx);
                    }
                }
            })
            .on_action({
                let focus = focus.clone();
                let on_confirm = self.on_confirm.clone();
                move |_: &ConfirmAction, window: &mut Window, cx: &mut App| match on_confirm
                    .as_ref()
                {
                    Some(on_confirm) if focus.is_focused(window) => on_confirm(window, cx),
                    _ => cx.propagate(),
                }
            })
            .on_action({
                let focus = focus.clone();
                move |_: &FocusNext, window: &mut Window, cx: &mut App| {
                    cycle_focus(&focus, true, window, cx)
                }
            })
            .on_action({
                let focus = focus.clone();
                move |_: &FocusPrev, window: &mut Window, cx: &mut App| {
                    cycle_focus(&focus, false, window, cx)
                }
            })
            // Un clic dans le dialogue ne doit pas atteindre le voile qui
            // ferme la modale.
            .when(self.close_on_backdrop, |d| {
                d.on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            })
            .flex()
            .flex_col()
            .w(self.width)
            .max_h(viewport.height * 0.9)
            .rounded(theme.radius_xl)
            .border_1()
            .border_color(theme.border)
            .bg(theme.bg_surface)
            .shadow(theme.shadow_modal())
            .text_size(theme.text_md)
            .text_color(theme.text)
            .font_family(theme.font_family.clone())
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_between()
                    .pl(theme.space_xxl)
                    .pr(theme.space_md)
                    .pt(theme.space_lg)
                    .pb(theme.space_md)
                    .child(
                        div()
                            .text_size(theme.text_lg)
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(theme.text_heading)
                            .child(self.title),
                    )
                    .children(close_button),
            )
            .child(
                div()
                    .id(ElementId::NamedChild(
                        Box::new(self.id.clone()),
                        "corps".into(),
                    ))
                    .flex()
                    .flex_col()
                    .flex_1()
                    .gap(theme.space_lg)
                    .px(theme.space_xxl)
                    .py(theme.space_md)
                    .overflow_y_scroll()
                    .children(self.children),
            )
            .when(!self.footer.is_empty(), |d| {
                d.child(
                    div()
                        .flex()
                        .flex_none()
                        .justify_end()
                        .gap(theme.space_md)
                        .px(theme.space_xxl)
                        .pt(theme.space_md)
                        .pb(theme.space_xl)
                        .children(self.footer),
                )
            });

        let backdrop_close = if self.close_on_backdrop {
            self.on_close.clone()
        } else {
            None
        };
        let backdrop = div()
            .id(ElementId::NamedChild(Box::new(self.id), "voile".into()))
            .occlude()
            .w(viewport.width)
            .h(viewport.height)
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.bg_backdrop)
            .when_some(backdrop_close, |d, handler| {
                d.on_mouse_down(gpui::MouseButton::Left, move |_, window, cx| {
                    handler(window, cx)
                })
            })
            .child(dialog);

        deferred(anchored().position(point(px(0.), px(0.))).child(backdrop))
            .with_priority(MODAL_PRIORITY)
    }
}

/// Confirmation OK / Annuler.
///
/// Entrée valide quand aucun bouton n'a le focus, Échap annule.
///
/// ```ignore
/// Confirm::new("reset", "Réinitialiser", "Des données non enregistrées seront perdues.")
///     .confirm_label("Réinitialiser").danger()
///     .on_confirm(cx.listener(|this, _: &(), _, cx| this.reset(cx)))
///     .on_cancel(cx.listener(|this, _: &(), _, cx| this.close_confirm(cx)))
/// ```
#[derive(IntoElement)]
pub struct Confirm {
    /// Identifiant de la modale.
    id: ElementId,
    /// Titre.
    title: SharedString,
    /// Message.
    message: SharedString,
    /// Libellé du bouton de validation (« OK » par défaut).
    confirm_label: SharedString,
    /// Libellé du bouton d'annulation (« Annuler » par défaut).
    cancel_label: SharedString,
    /// Validation destructrice : bouton rouge.
    danger: bool,
    /// Rappel de validation.
    on_confirm: Option<ModalHandler>,
    /// Rappel d'annulation (bouton, ✕, Échap).
    on_cancel: Option<ModalHandler>,
}

impl Confirm {
    /// Confirmation avec titre et message.
    pub fn new(
        id: impl Into<ElementId>,
        title: impl Into<SharedString>,
        message: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            message: message.into(),
            confirm_label: "OK".into(),
            cancel_label: "Annuler".into(),
            danger: false,
            on_confirm: None,
            on_cancel: None,
        }
    }

    /// Libellé du bouton de validation.
    pub fn confirm_label(mut self, label: impl Into<SharedString>) -> Self {
        self.confirm_label = label.into();
        self
    }

    /// Libellé du bouton d'annulation.
    pub fn cancel_label(mut self, label: impl Into<SharedString>) -> Self {
        self.cancel_label = label.into();
        self
    }

    /// Validation destructrice (bouton rouge).
    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }

    /// Rappel de validation.
    pub fn on_confirm(mut self, handler: impl Fn(&(), &mut Window, &mut App) + 'static) -> Self {
        self.on_confirm = Some(Rc::new(move |window, cx| handler(&(), window, cx)));
        self
    }

    /// Rappel d'annulation.
    pub fn on_cancel(mut self, handler: impl Fn(&(), &mut Window, &mut App) + 'static) -> Self {
        self.on_cancel = Some(Rc::new(move |window, cx| handler(&(), window, cx)));
        self
    }
}

impl RenderOnce for Confirm {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let on_confirm = self.on_confirm.clone();
        let on_cancel = self.on_cancel.clone();
        let ok = Button::new(
            ElementId::NamedChild(Box::new(self.id.clone()), "ok".into()),
            self.confirm_label,
        )
        .variant(if self.danger {
            crate::widgets::button::ButtonVariant::Danger
        } else {
            crate::widgets::button::ButtonVariant::Primary
        })
        .on_click({
            let on_confirm = on_confirm.clone();
            move |_, window, cx| {
                if let Some(handler) = on_confirm.as_ref() {
                    handler(window, cx);
                }
            }
        });
        let cancel = Button::new(
            ElementId::NamedChild(Box::new(self.id.clone()), "annuler".into()),
            self.cancel_label,
        )
        .on_click({
            let on_cancel = on_cancel.clone();
            move |_, window, cx| {
                if let Some(handler) = on_cancel.as_ref() {
                    handler(window, cx);
                }
            }
        });
        let mut modal = Modal::new(self.id, self.title)
            .width(px(420.))
            .child(
                div()
                    .text_color(theme.text)
                    .line_height(gpui::relative(1.45))
                    .child(self.message),
            )
            .footer(cancel)
            .footer(ok);
        if let Some(handler) = on_cancel {
            modal = modal.on_close(move |_, window, cx| handler(window, cx));
        }
        if let Some(handler) = on_confirm {
            modal = modal.on_confirm(move |_, window, cx| handler(window, cx));
        }
        modal
    }
}
