//! Barre d'outils de la vue d'analyse : boutons dans un ordre fixe, règles
//! de grisage, bulles en français.
//! Chaque bouton déclenche l'action de menu correspondante
//! ([`crate::app::actions`]) : la barre, le menu et le clavier suivent le
//! même chemin.

use gpui::{App, IntoElement, ParentElement, Styled, Window, div, px};
use lunar_ui_logic::menu::MenuActionId;

use crate::app::actions::action_for;
use crate::app::panels::{ImportMode, Panel};
use crate::assets::Icon;
use crate::theme::Theme;
use crate::widgets::button::{Button, ButtonSize};

/// Hauteur de la barre (`.top-bar { height: 40px }`).
pub const TOOLBAR_HEIGHT: f32 = 40.;

/// État dont dépendent les boutons : capture en cours, panneau ouvert,
/// relevé chargé.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ToolbarState {
    /// Capture en cours.
    pub running: bool,
    /// La matrice contient des données.
    pub has_data: bool,
    /// Panneau de la barre d'outils ouvert.
    pub open_panel: Option<Panel>,
}

/// Un bouton de la barre.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolbarButton {
    /// Action déclenchée.
    pub action: MenuActionId,
    /// Icône.
    pub icon: Icon,
    /// Bulle (sans le raccourci).
    pub title: &'static str,
    /// Grisé.
    pub disabled: bool,
    /// Panneau du bouton ouvert.
    pub toggled: bool,
}

/// Boutons dans l'ordre de la barre, grisés selon l'état.
pub fn toolbar_buttons(state: &ToolbarState) -> Vec<ToolbarButton> {
    use MenuActionId as A;
    let running = state.running;
    let panel_open = state.open_panel.is_some();
    let open = |panel: Panel| state.open_panel == Some(panel);
    let button = |action, icon, title, disabled, toggled| ToolbarButton {
        action,
        icon,
        title,
        disabled,
        toggled,
    };
    vec![
        button(
            A::StartCapture,
            Icon::Play,
            "Démarrer",
            running || panel_open,
            false,
        ),
        button(A::StopCapture, Icon::Stop, "Arrêter", !running, false),
        button(
            A::ResetCapture,
            Icon::Refresh,
            "Réinitialiser",
            panel_open,
            false,
        ),
        button(
            A::OpenConfig,
            Icon::Gear,
            "Config",
            running,
            open(Panel::Config),
        ),
        button(A::ExportCsv, Icon::FileExport, "Sauvegarder", false, false),
        button(
            A::SaveProject,
            Icon::Save,
            "Enregistrer le projet (.lunar)",
            false,
            false,
        ),
        button(
            A::OpenProject,
            Icon::FolderOpen,
            "Ouvrir un projet (.lunar)",
            running || state.has_data,
            false,
        ),
        button(
            A::ExportLabels,
            Icon::Tag,
            "Exporter les labels",
            false,
            false,
        ),
        button(
            A::OpenLabels,
            Icon::Tags,
            "Gérer les labels",
            running,
            open(Panel::Labels),
        ),
        button(
            A::OpenCaptureFile,
            Icon::FileImport,
            "Ouvrir un fichier Pcap ou une matrice de flux",
            running || state.has_data,
            open(Panel::Import(ImportMode::Capture)),
        ),
        button(
            A::ImportLabels,
            Icon::FileSpreadsheet,
            "Importer un fichier de label",
            running,
            open(Panel::Import(ImportMode::Labels)),
        ),
        button(A::Quit, Icon::Quit, "Quitter", false, false),
        button(A::ExportLogs, Icon::Logs, "Logs", false, false),
        button(
            A::OpenFilter,
            Icon::Filter,
            "Filtrer",
            running,
            open(Panel::Filter),
        ),
        button(
            A::OpenInvestigation,
            Icon::Microscope,
            "Investigation",
            running || panel_open,
            false,
        ),
    ]
}

/// Bulle : titre et raccourci éventuel (« Démarrer (Ctrl+P) »).
pub fn tooltip_text(button: &ToolbarButton) -> String {
    match button.action.shortcut() {
        Some(shortcut) => format!("{} ({})", button.title, shortcut.display()),
        None => button.title.to_owned(),
    }
}

/// Barre d'outils : fond `#070416`, boutons espacés de 8 px.
pub fn render_toolbar(state: &ToolbarState, theme: &Theme) -> impl IntoElement + use<> {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.))
        .h(px(TOOLBAR_HEIGHT))
        .px(px(10.))
        .bg(theme.bg_bar)
        .border_b_1()
        .border_color(theme.border_divider)
        .children(toolbar_buttons(state).into_iter().map(|button| {
            let action = button.action;
            Button::icon_only(
                gpui::ElementId::Name(format!("outil-{}", action.as_str()).into()),
                button.icon,
                tooltip_text(&button),
            )
            .ghost()
            .size(ButtonSize::Toolbar)
            .disabled(button.disabled)
            .toggled(button.toggled)
            .on_click(move |_, window: &mut Window, cx: &mut App| {
                window.dispatch_action(action_for(action), cx)
            })
        }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disabled(state: &ToolbarState) -> Vec<MenuActionId> {
        toolbar_buttons(state)
            .into_iter()
            .filter(|b| b.disabled)
            .map(|b| b.action)
            .collect()
    }

    #[test]
    fn ordre_de_topbar() {
        use MenuActionId as A;
        let order: Vec<MenuActionId> = toolbar_buttons(&ToolbarState::default())
            .into_iter()
            .map(|b| b.action)
            .collect();
        assert_eq!(
            order,
            [
                A::StartCapture,
                A::StopCapture,
                A::ResetCapture,
                A::OpenConfig,
                A::ExportCsv,
                A::SaveProject,
                A::OpenProject,
                A::ExportLabels,
                A::OpenLabels,
                A::OpenCaptureFile,
                A::ImportLabels,
                A::Quit,
                A::ExportLogs,
                A::OpenFilter,
                A::OpenInvestigation,
            ]
        );
    }

    #[test]
    fn au_repos_seul_arreter_est_grise() {
        assert_eq!(
            disabled(&ToolbarState::default()),
            [MenuActionId::StopCapture]
        );
    }

    #[test]
    fn capture_en_cours() {
        use MenuActionId as A;
        let state = ToolbarState {
            running: true,
            ..ToolbarState::default()
        };
        assert_eq!(
            disabled(&state),
            [
                A::StartCapture,
                A::OpenConfig,
                A::OpenProject,
                A::OpenLabels,
                A::OpenCaptureFile,
                A::ImportLabels,
                A::OpenFilter,
                A::OpenInvestigation,
            ]
        );
    }

    #[test]
    fn panneau_ouvert_et_releve_charge() {
        use MenuActionId as A;
        let state = ToolbarState {
            running: false,
            has_data: true,
            open_panel: Some(Panel::Filter),
        };
        assert_eq!(
            disabled(&state),
            [
                A::StartCapture,
                A::StopCapture,
                A::ResetCapture,
                A::OpenProject,
                A::OpenCaptureFile,
                A::OpenInvestigation,
            ]
        );
        let toggled: Vec<MenuActionId> = toolbar_buttons(&state)
            .into_iter()
            .filter(|b| b.toggled)
            .map(|b| b.action)
            .collect();
        assert_eq!(toggled, [A::OpenFilter]);
    }

    #[test]
    fn bulles_avec_raccourci() {
        let buttons = toolbar_buttons(&ToolbarState::default());
        let titles: Vec<String> = buttons.iter().map(tooltip_text).collect();
        assert_eq!(titles[0], "Démarrer (Ctrl+P)");
        assert_eq!(titles[3], "Config (Ctrl+,)");
        assert_eq!(titles[5], "Enregistrer le projet (.lunar) (Ctrl+Shift+S)");
        assert_eq!(titles[14], "Investigation (Ctrl+I)");
    }
}
