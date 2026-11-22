//! Actions gpui de l'application : une par entrée de menu
//! ([`MenuActionId`]), déclenchées indifféremment par la barre de menus, la
//! barre d'outils et le clavier, plus les commandes du graphe.
//!
//! Les raccourcis sont ceux de `lunar_ui_logic::menu`, liés au contexte
//! clavier de la vue qui porte l'action ([`ANALYSE_CONTEXT`] ou
//! [`INVESTIGATION_CONTEXT`], posé sur la racine de la fenêtre) : deux
//! actions de vues différentes partagent ainsi un raccourci (Ctrl+S,
//! Ctrl+E, Ctrl+I). Les actions de toutes les vues sont liées dans les deux
//! contextes.

use std::rc::Rc;

use gpui::{Action, App, InteractiveElement, KeyBinding, Window, actions};
use lunar_ui_logic::menu::{ActionView, MenuActionId, MenuView};

/// Contexte clavier de la vue d'analyse.
pub const ANALYSE_CONTEXT: &str = "LunarAnalyse";

/// Contexte clavier de la vue d'investigation.
pub const INVESTIGATION_CONTEXT: &str = "LunarInvestigation";

/// Rappel commun des actions de menu.
pub type MenuActionHandler = Rc<dyn Fn(MenuActionId, &mut Window, &mut App)>;

/// Déclare une action par variante de [`MenuActionId`] (même nom) et les
/// correspondances identifiant ↔ action.
macro_rules! menu_actions {
    ($($(#[$doc:meta])* $name:ident),* $(,)?) => {
        actions!(lunar, [$($(#[$doc])* $name),*]);

        /// Action gpui d'une entrée de menu.
        pub fn action_for(id: MenuActionId) -> Box<dyn Action> {
            match id {
                $(MenuActionId::$name => Box::new($name),)*
            }
        }

        /// Raccourci `keystroke` de l'entrée `id` dans `context`.
        fn binding(id: MenuActionId, keystroke: &str, context: &str) -> KeyBinding {
            match id {
                $(MenuActionId::$name => KeyBinding::new(keystroke, $name, Some(context)),)*
            }
        }

        /// Branche `handler` sur chaque action de menu de `element`.
        pub fn on_menu_actions<E: InteractiveElement>(element: E, handler: MenuActionHandler) -> E {
            $(
                let element = {
                    let handler = Rc::clone(&handler);
                    element.on_action(move |_: &$name, window: &mut Window, cx: &mut App| {
                        handler(MenuActionId::$name, window, cx)
                    })
                };
            )*
            element
        }
    };
}

menu_actions!(
    /// Ouvrir un projet `.lunar`.
    OpenProject,
    /// Enregistrer le projet `.lunar`.
    SaveProject,
    /// Ouvrir un fichier PCAP ou une matrice de flux.
    OpenCaptureFile,
    /// Importer un fichier de labels.
    ImportLabels,
    /// Sauvegarder la matrice (CSV).
    ExportCsv,
    /// Exporter les labels.
    ExportLabels,
    /// Exporter les journaux.
    ExportLogs,
    /// Quitter.
    Quit,
    /// Démarrer la capture.
    StartCapture,
    /// Arrêter la capture.
    StopCapture,
    /// Réinitialiser le relevé.
    ResetCapture,
    /// Configuration de la capture.
    OpenConfig,
    /// Filtre BPF.
    OpenFilter,
    /// Gestion des labels.
    OpenLabels,
    /// Afficher ou masquer le graphe.
    ToggleGraph,
    /// Vue d'investigation.
    OpenInvestigation,
    /// Retour à la vue d'analyse.
    BackToAnalyse,
    /// Exporter la synthèse HTML.
    ExportSynthesis,
    /// Exporter les labels à compléter.
    ExportLabelsToComplete,
    /// Exporter les seuls flux affichés.
    ExportFilteredFlows,
    /// Recharger le relevé (investigation).
    ReloadSurvey,
    /// Raccourcis clavier (F1).
    Shortcuts,
    /// À propos de LUNAR.
    About,
    /// Notes de version.
    Changelog,
);

actions!(
    lunar,
    [
        /// Active ou coupe la gravité (simulation de forces) du graphe.
        ToggleGravity,
        /// Exporte le graphe en PNG.
        ExportGraphPng,
        /// Ouvre ou ferme les labels appliqués à la matrice (bouton « Afficher
        /// les labels » du graphe).
        ShowMatrixLabels,
    ]
);

/// Contexte clavier de la racine pour `view`.
pub fn key_context(view: MenuView) -> &'static str {
    match view {
        MenuView::Analyse => ANALYSE_CONTEXT,
        MenuView::Investigation => INVESTIGATION_CONTEXT,
    }
}

/// Contextes où lier une action de la vue `view`.
fn contexts(view: ActionView) -> &'static [&'static str] {
    match view {
        ActionView::Any => &[ANALYSE_CONTEXT, INVESTIGATION_CONTEXT],
        ActionView::Analyse => &[ANALYSE_CONTEXT],
        ActionView::Investigation => &[INVESTIGATION_CONTEXT],
    }
}

/// Raccourcis de toutes les actions de menu.
pub fn key_bindings() -> Vec<KeyBinding> {
    MenuActionId::all()
        .filter_map(|id| {
            id.shortcut()
                .map(|shortcut| (id, shortcut.to_gpui_keystroke()))
        })
        .flat_map(|(id, keystroke)| {
            contexts(id.view())
                .iter()
                .map(move |context| binding(id, &keystroke, context))
        })
        .collect()
}

/// Installe les raccourcis de l'application (après [`crate::widgets::init`]).
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys(key_bindings());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn une_action_par_entree_de_menu() {
        for id in MenuActionId::all() {
            let action = action_for(id);
            // Nom gpui « lunar::OpenProject » : même nom que la variante.
            let expected = format!("lunar::{id:?}");
            assert_eq!(action.name(), expected);
        }
    }

    #[test]
    fn raccourcis_lies_par_vue() {
        let bindings = key_bindings();
        let with_shortcut = MenuActionId::all()
            .filter(|id| id.shortcut().is_some())
            .map(|id| contexts(id.view()).len())
            .sum::<usize>();
        assert_eq!(bindings.len(), with_shortcut);
        // F1 (Raccourcis) partout ; Ctrl+S propre à chaque vue.
        let f1 = bindings
            .iter()
            .filter(|b| b.action().name() == "lunar::Shortcuts")
            .count();
        assert_eq!(f1, 2);
        let ctrl_s: Vec<&str> = bindings
            .iter()
            .filter(|b| {
                b.keystrokes()
                    .first()
                    .is_some_and(|k| k.unparse() == "ctrl-s")
            })
            .map(|b| b.action().name())
            .collect();
        assert_eq!(ctrl_s, ["lunar::ExportCsv", "lunar::ExportSynthesis"]);
    }

    #[test]
    fn contexte_par_vue() {
        assert_eq!(key_context(MenuView::Analyse), ANALYSE_CONTEXT);
        assert_eq!(key_context(MenuView::Investigation), INVESTIGATION_CONTEXT);
    }
}
