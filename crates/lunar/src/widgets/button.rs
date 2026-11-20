//! Bouton : principal, secondaire, danger, discret ; icône et libellé,
//! icône seule avec bulle, désactivé, basculé (bascules de barre d'outils).
//!
//! Au clavier : focalisable par Tab, activé par Entrée ou Espace (gpui
//! déclenche `on_click` au relâchement de ces touches). Un clic souris ne
//! prend pas le focus, pour que l'anneau de mise au point ne s'affiche qu'au
//! clavier (gpui 0.2.2 n'a pas d'équivalent de `:focus-visible`).

use std::rc::Rc;

use gpui::{
    App, ClickEvent, CursorStyle, ElementId, IntoElement, MouseButton, SharedString, Window, div,
    prelude::*, px,
};

use crate::assets::{Icon, icon};
use crate::theme::ActiveTheme;
use crate::widgets::tooltip::tooltip;

/// Rappel de clic d'un bouton.
pub type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// Variante visuelle d'un bouton.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonVariant {
    /// Action principale d'un dialogue.
    Primary,
    /// Action ordinaire.
    #[default]
    Secondary,
    /// Action destructrice.
    Danger,
    /// Bouton discret, sans fond, des barres d'outils.
    Ghost,
}

/// Taille d'un bouton.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonSize {
    /// Hauteur des contrôles du thème (30 px).
    #[default]
    Medium,
    /// Hauteur réduite (24 px), pour les lignes de tableau et les en-têtes.
    Small,
    /// Grand bouton d'icône de barre d'outils (34 px, icône 20 px).
    Toolbar,
}

/// Bouton, construit à chaque rendu.
///
/// ```ignore
/// Button::new("demarrer", "Démarrer").icon(Icon::Play).primary()
///     .on_click(|_, _, _| println!("clic"))
/// Button::icon_only("arreter", Icon::Stop, "Arrêter (Ctrl+Maj+P)").ghost()
/// ```
#[derive(IntoElement)]
pub struct Button {
    /// Identifiant de l'élément (stable d'un rendu à l'autre).
    id: ElementId,
    /// Libellé ; absent pour un bouton d'icône seule.
    label: Option<SharedString>,
    /// Icône à gauche du libellé.
    icon: Option<Icon>,
    /// Variante visuelle.
    variant: ButtonVariant,
    /// Taille.
    size: ButtonSize,
    /// Désactivé : ni clic, ni focus.
    disabled: bool,
    /// État basculé (enfoncé) d'une bascule.
    toggled: bool,
    /// Texte de la bulle au survol.
    tooltip: Option<SharedString>,
    /// Occupe toute la largeur disponible.
    full_width: bool,
    /// Rappel de clic.
    on_click: Option<ClickHandler>,
}

impl Button {
    /// Bouton avec libellé.
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: Some(label.into()),
            icon: None,
            variant: ButtonVariant::default(),
            size: ButtonSize::default(),
            disabled: false,
            toggled: false,
            tooltip: None,
            full_width: false,
            on_click: None,
        }
    }

    /// Bouton d'icône seule ; `tooltip` sert de bulle et décrit l'action.
    pub fn icon_only(
        id: impl Into<ElementId>,
        icon: Icon,
        tooltip: impl Into<SharedString>,
    ) -> Self {
        Self {
            label: None,
            icon: Some(icon),
            tooltip: Some(tooltip.into()),
            ..Self::new(id, "")
        }
    }

    /// Ajoute une icône à gauche du libellé.
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Choisit la variante.
    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }

    /// Variante principale.
    pub fn primary(self) -> Self {
        self.variant(ButtonVariant::Primary)
    }

    /// Variante danger.
    pub fn danger(self) -> Self {
        self.variant(ButtonVariant::Danger)
    }

    /// Variante discrète.
    pub fn ghost(self) -> Self {
        self.variant(ButtonVariant::Ghost)
    }

    /// Choisit la taille.
    pub fn size(mut self, size: ButtonSize) -> Self {
        self.size = size;
        self
    }

    /// Désactive le bouton.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Affiche l'état basculé (bascule enfoncée).
    pub fn toggled(mut self, toggled: bool) -> Self {
        self.toggled = toggled;
        self
    }

    /// Bulle affichée au survol.
    pub fn tooltip(mut self, text: impl Into<SharedString>) -> Self {
        self.tooltip = Some(text.into());
        self
    }

    /// Occupe toute la largeur disponible.
    pub fn full_width(mut self) -> Self {
        self.full_width = true;
        self
    }

    /// Rappel appelé au clic, ou à Entrée / Espace quand le bouton a le focus.
    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }
}

/// Couleurs d'un bouton pour une variante et un état.
struct Palette {
    /// Fond au repos.
    bg: gpui::Hsla,
    /// Fond survolé.
    bg_hover: gpui::Hsla,
    /// Bordure au repos.
    border: gpui::Hsla,
    /// Bordure survolée.
    border_hover: gpui::Hsla,
    /// Texte et icône au repos.
    text: gpui::Hsla,
    /// Texte et icône survolés.
    text_hover: gpui::Hsla,
}

impl RenderOnce for Button {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let transparent = gpui::transparent_black();
        let mut palette = match self.variant {
            ButtonVariant::Primary => Palette {
                bg: theme.primary,
                bg_hover: theme.primary_hover,
                border: theme.primary,
                border_hover: theme.primary_hover,
                text: theme.text_on_fill,
                text_hover: theme.text_on_fill,
            },
            ButtonVariant::Secondary => Palette {
                bg: theme.button_bg,
                bg_hover: theme.button_bg_hover,
                border: theme.border,
                border_hover: theme.border_strong,
                text: theme.button_text,
                text_hover: theme.button_text_hover,
            },
            ButtonVariant::Danger => Palette {
                bg: theme.danger,
                bg_hover: theme.danger_hover,
                border: theme.danger,
                border_hover: theme.danger_hover,
                text: theme.text_on_fill,
                text_hover: theme.text_on_fill,
            },
            ButtonVariant::Ghost => Palette {
                bg: transparent,
                bg_hover: theme.bg_ghost_hover,
                border: transparent,
                border_hover: transparent,
                text: theme.text,
                text_hover: theme.text,
            },
        };
        if self.toggled {
            palette.bg = theme.accent_soft;
            palette.bg_hover = theme.accent_soft;
            palette.border = theme.accent;
            palette.border_hover = theme.accent;
            palette.text = theme.accent;
            palette.text_hover = theme.accent;
        }
        let icon_only = self.label.is_none();
        let (height, icon_size, pad_x) = match self.size {
            ButtonSize::Medium => (theme.control_height, px(16.), px(14.)),
            ButtonSize::Small => (px(24.), px(14.), px(8.)),
            ButtonSize::Toolbar => (px(34.), px(20.), px(8.)),
        };
        let group: SharedString = format!("bouton-{:?}", self.id).into();
        // Sur un bouton plein, la bordure de focus passe en blanc : le bleu
        // de l'anneau se confondrait avec `primary`.
        let focus_ring = match self.variant {
            ButtonVariant::Primary | ButtonVariant::Danger => theme.text_on_fill,
            ButtonVariant::Secondary | ButtonVariant::Ghost => theme.focus_ring,
        };
        let focus_shadow = theme.shadow_focus();
        let radius = if self.size == ButtonSize::Toolbar {
            theme.radius_sm
        } else {
            theme.radius_md
        };

        let mut button = div()
            .id(self.id.clone())
            .group(group.clone())
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .gap(theme.space_sm)
            .h(height)
            .when(icon_only, |b| b.w(height))
            .when(!icon_only, |b| b.px(pad_x))
            .when(self.full_width, |b| b.w_full())
            .rounded(radius)
            .border_1()
            .border_color(palette.border)
            .bg(palette.bg)
            .text_color(palette.text)
            .text_size(theme.text_md)
            .font_weight(if self.variant == ButtonVariant::Primary {
                gpui::FontWeight::SEMIBOLD
            } else {
                gpui::FontWeight::NORMAL
            })
            .whitespace_nowrap();

        if let Some(icon_kind) = self.icon {
            let text = palette.text;
            let text_hover = palette.text_hover;
            button = button.child(
                icon(icon_kind)
                    .size(icon_size)
                    .text_color(text)
                    .when(!self.disabled, |svg| {
                        svg.group_hover(group.clone(), |s| s.text_color(text_hover))
                    }),
            );
        }
        if let Some(label) = self.label.clone() {
            button = button.child(label);
        }
        if let Some(text) = self.tooltip.clone() {
            button = button.tooltip(tooltip(text));
        }

        if self.disabled {
            button
                .opacity(theme.disabled_opacity)
                .cursor(CursorStyle::OperationNotAllowed)
        } else {
            let bg_hover = palette.bg_hover;
            let border_hover = palette.border_hover;
            let text_hover = palette.text_hover;
            button = button
                .tab_index(0)
                .cursor(CursorStyle::PointingHand)
                .hover(move |s| {
                    s.bg(bg_hover)
                        .border_color(border_hover)
                        .text_color(text_hover)
                })
                .focus(move |s| s.border_color(focus_ring).shadow(focus_shadow.clone()))
                // Un clic souris ne déplace pas le focus (voir l'en-tête).
                .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default());
            if let Some(handler) = self.on_click {
                button = button.on_click(move |event, window, cx| handler(event, window, cx));
            }
            button
        }
    }
}
