//! Composants réutilisables de l'interface LUNAR, stylés par [`crate::theme`].
//!
//! Deux familles :
//! - composants sans état, construits à chaque rendu (`RenderOnce`) :
//!   [`button::Button`], [`checkbox::Checkbox`], [`checkbox::Switch`],
//!   [`radio::RadioGroup`], [`tabs::Tabs`], [`progress::ProgressBar`],
//!   [`kpi::Kpi`], [`panel::Panel`], [`modal::Modal`] ;
//! - composants à état, détenus dans une `Entity` par la vue parente et
//!   signalant leurs changements par `EventEmitter` :
//!   [`text_input::TextInput`], [`select::Select`], [`table::Table`],
//!   [`menu_bar::MenuBar`], [`toast::ToastStack`].
//!
//! [`init`] installe le thème et les raccourcis clavier ; la vue racine de
//! chaque fenêtre passe son élément racine à [`root_actions`] pour la
//! navigation au clavier (Tab, Maj+Tab, Alt / F10).

pub mod button;
pub mod checkbox;
pub mod kpi;
pub mod menu_bar;
pub mod modal;
pub mod nav;
pub mod panel;
pub mod progress;
pub mod radio;
pub mod select;
pub mod table;
pub mod tabs;
pub mod text_input;
pub mod toast;
pub mod tooltip;

use gpui::{App, Entity, InteractiveElement, KeyBinding, Styled, Window, actions};

use crate::widgets::menu_bar::MenuBar;

actions!(
    lunar_widgets,
    [
        /// Passe au contrôle suivant (Tab).
        FocusNext,
        /// Revient au contrôle précédent (Maj+Tab).
        FocusPrev,
        /// Ouvre la barre de menus au clavier (Alt seul, F10).
        OpenMenuBar,
        /// Élément précédent d'une liste (flèche haut).
        MoveUp,
        /// Élément suivant d'une liste (flèche bas).
        MoveDown,
        /// Section précédente (flèche gauche).
        MoveLeft,
        /// Section suivante (flèche droite).
        MoveRight,
        /// Premier élément (Début).
        MoveFirst,
        /// Dernier élément (Fin).
        MoveLast,
        /// Page précédente (Page préc.).
        PageUp,
        /// Page suivante (Page suiv.).
        PageDown,
        /// Valide l'élément en surbrillance (Entrée, Espace).
        Confirm,
        /// Ferme la liste, le menu ou la modale (Échap).
        Cancel,
    ]
);

/// Contexte clavier des listes déroulantes.
pub const SELECT_CONTEXT: &str = "LunarSelect";
/// Contexte clavier de la barre de menus.
pub const MENU_BAR_CONTEXT: &str = "LunarMenuBar";
/// Contexte clavier des tableaux.
pub const TABLE_CONTEXT: &str = "LunarTable";
/// Contexte clavier des modales.
pub const MODAL_CONTEXT: &str = "LunarModal";

/// Installe le thème LUNAR et les raccourcis clavier des composants.
///
/// À appeler une fois, au démarrage de l'application, avant d'ouvrir une
/// fenêtre.
pub fn init(cx: &mut App) {
    crate::theme::init(cx);
    cx.bind_keys([
        KeyBinding::new("tab", FocusNext, None),
        KeyBinding::new("shift-tab", FocusPrev, None),
        KeyBinding::new("alt", OpenMenuBar, None),
        KeyBinding::new("f10", OpenMenuBar, None),
        KeyBinding::new("escape", Cancel, Some(MODAL_CONTEXT)),
        KeyBinding::new("enter", Confirm, Some(MODAL_CONTEXT)),
    ]);
    let list_contexts = [SELECT_CONTEXT, MENU_BAR_CONTEXT, TABLE_CONTEXT];
    for context in list_contexts {
        cx.bind_keys([
            KeyBinding::new("up", MoveUp, Some(context)),
            KeyBinding::new("down", MoveDown, Some(context)),
            KeyBinding::new("home", MoveFirst, Some(context)),
            KeyBinding::new("end", MoveLast, Some(context)),
            KeyBinding::new("pageup", PageUp, Some(context)),
            KeyBinding::new("pagedown", PageDown, Some(context)),
            KeyBinding::new("enter", Confirm, Some(context)),
            KeyBinding::new("escape", Cancel, Some(context)),
        ]);
    }
    cx.bind_keys([
        KeyBinding::new("space", Confirm, Some(SELECT_CONTEXT)),
        KeyBinding::new("space", Confirm, Some(MENU_BAR_CONTEXT)),
        KeyBinding::new("left", MoveLeft, Some(MENU_BAR_CONTEXT)),
        KeyBinding::new("right", MoveRight, Some(MENU_BAR_CONTEXT)),
    ]);
    text_input::bind_keys(cx);
}

/// Texte sur une ligne, tronqué avec des points de suspension.
///
/// Remplace `truncate()` : en `whitespace_nowrap`, gpui 0.2.2 garde la
/// première mesure du texte (souvent sans largeur disponible, pendant le
/// calcul des tailles intrinsèques des flex) et n'ajoute jamais « … ». En
/// retour à la ligne normal limité à une ligne, la mesure est refaite quand
/// la largeur change et la troncature s'applique.
pub fn ellipsis<E: Styled>(element: E) -> E {
    element.overflow_hidden().text_ellipsis().line_clamp(1)
}

/// Branche la navigation clavier globale sur l'élément racine d'une fenêtre.
///
/// - Tab / Maj+Tab parcourent les contrôles focalisables ;
/// - Alt seul ou F10 ouvrent la barre de menus, si `menu_bar` est fourni.
///
/// Les actions remontent du contrôle focalisé vers la racine : c'est donc
/// la racine qui doit les traiter (gpui 0.2.2 n'a pas d'écouteur d'action
/// de fenêtre indépendant du focus).
///
/// La racine doit aussi suivre un `FocusHandle` (`track_focus`) focalisé au
/// démarrage : sans focus, gpui distribue les touches au seul nœud racine
/// de la vue, et Alt, F10 ou Tab n'atteignent jamais ces écouteurs.
pub fn root_actions<E: InteractiveElement>(element: E, menu_bar: Option<&Entity<MenuBar>>) -> E {
    let element = element
        .on_action(|_: &FocusNext, window: &mut Window, _: &mut App| window.focus_next())
        .on_action(|_: &FocusPrev, window: &mut Window, _: &mut App| window.focus_prev());
    match menu_bar {
        Some(menu_bar) => {
            let menu_bar = menu_bar.clone();
            element.on_action(move |_: &OpenMenuBar, window: &mut Window, cx: &mut App| {
                menu_bar.update(cx, |menu, cx| menu.open_from_keyboard(window, cx));
            })
        }
        None => element,
    }
}
