//! Panneaux de la vue d'analyse : lequel est ouvert, règles d'exclusivité,
//! et point de branchement des corps de panneau.
//!
//! Règles :
//! - un seul panneau principal à la fois (configuration, import de
//!   captures, import de labels, filtre, gestion des labels, labels de la
//!   matrice) : en ouvrir un ferme l'autre, le rouvrir le ferme ;
//! - les dialogues imbriqués n'existent qu'avec leur parent : conflits
//!   d'import (dans l'import de labels), arbitrage (dans l'import de labels
//!   ou la gestion des labels) ; fermer le parent les ferme ;
//! - les gardes qui dépendent de la capture (capture en cours, verrou
//!   d'import, relevé déjà chargé) sont dans
//!   [`crate::app::state::AppState::toggle_panel`].
//!
//! Brancher un vrai panneau : remplacer son bras dans [`build_panel_view`]
//! par la construction de sa vue. La vue reçoit l'`Entity<AppState>` ; elle
//! dessine son propre cadre ([`crate::widgets::panel::Panel`] pour un
//! panneau latéral, [`crate::widgets::modal::Modal`] pour un dialogue, voir
//! [`Panel::presentation`]) et se ferme par
//! [`crate::app::state::AppState::close_panel`]. Elle est créée à
//! l'ouverture et détruite à la fermeture.

pub mod config;
pub mod filter;
pub mod import_capture;
pub mod keys;

use gpui::{AnyView, App, AppContext, Entity, Pixels, Window, px};

use crate::app::state::AppState;

/// Ce qu'importe le panneau d'import.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImportMode {
    /// Captures PCAP/PCAPNG ou matrices de flux.
    Capture,
    /// Fichier de labels.
    Labels,
}

/// Panneau ou dialogue de la vue d'analyse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Panel {
    /// Configuration de la capture.
    Config,
    /// Import.
    Import(ImportMode),
    /// Filtre BPF.
    Filter,
    /// Gestion des labels.
    Labels,
    /// Conflits d'import de labels.
    Conflicts,
    /// Arbitrage des conflits de labels.
    Arbitration,
    /// Labels appliqués à la matrice, ouvert depuis le graphe.
    MatrixLabels,
}

/// Façon dont un panneau est présenté.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presentation {
    /// Panneau latéral droit, à côté du graphe.
    Side,
    /// Dialogue modal centré.
    Modal,
}

impl Panel {
    /// Titre affiché en tête du panneau.
    pub fn title(self) -> &'static str {
        match self {
            Panel::Config => "Configuration Capture",
            Panel::Import(ImportMode::Capture) => "Import de captures ou de matrices",
            Panel::Import(ImportMode::Labels) => "Import de labels",
            Panel::Filter => "Filtre BPF",
            Panel::Labels => "Gestion des labels",
            Panel::Conflicts => "Conflits d'import de labels",
            Panel::Arbitration => "Arbitrage des conflits de labels",
            Panel::MatrixLabels => "Labels appliqués à la matrice",
        }
    }

    /// Identifiant stable (éléments gpui, journaux).
    pub fn key(self) -> &'static str {
        match self {
            Panel::Config => "config",
            Panel::Import(ImportMode::Capture) => "import-captures",
            Panel::Import(ImportMode::Labels) => "import-labels",
            Panel::Filter => "filtre",
            Panel::Labels => "labels",
            Panel::Conflicts => "conflits",
            Panel::Arbitration => "arbitrage",
            Panel::MatrixLabels => "labels-matrice",
        }
    }

    /// Présentation : les panneaux de la barre d'outils sont latéraux, les
    /// dialogues et les labels de la matrice sont modaux.
    pub fn presentation(self) -> Presentation {
        match self {
            Panel::Config | Panel::Import(_) | Panel::Filter | Panel::Labels => Presentation::Side,
            Panel::Conflicts | Panel::Arbitration | Panel::MatrixLabels => Presentation::Modal,
        }
    }

    /// Largeur du panneau latéral ou du dialogue.
    pub fn width(self) -> Pixels {
        match self {
            Panel::Config => px(400.),
            Panel::Import(_) => px(480.),
            Panel::Filter => px(520.),
            Panel::Labels => px(720.),
            Panel::Conflicts | Panel::Arbitration => px(720.),
            Panel::MatrixLabels => px(640.),
        }
    }

    /// Panneau ouvert depuis la barre d'outils.
    pub fn is_toolbar_panel(self) -> bool {
        matches!(
            self,
            Panel::Config | Panel::Import(_) | Panel::Filter | Panel::Labels
        )
    }

    /// Dialogue imbriqué dans un panneau parent.
    pub fn is_dialog(self) -> bool {
        matches!(self, Panel::Conflicts | Panel::Arbitration)
    }

    /// Parents possibles d'un dialogue imbriqué.
    pub fn parents(self) -> &'static [Panel] {
        match self {
            Panel::Conflicts => &[Panel::Import(ImportMode::Labels)],
            Panel::Arbitration => &[Panel::Import(ImportMode::Labels), Panel::Labels],
            _ => &[],
        }
    }
}

/// Panneaux ouverts : un principal, éventuellement un dialogue imbriqué.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PanelState {
    /// Panneau principal.
    main: Option<Panel>,
    /// Dialogue imbriqué dans le panneau principal.
    dialog: Option<Panel>,
}

impl PanelState {
    /// Panneau principal ouvert.
    pub fn main(&self) -> Option<Panel> {
        self.main
    }

    /// Dialogue imbriqué ouvert.
    pub fn dialog(&self) -> Option<Panel> {
        self.dialog
    }

    /// `panel` est ouvert.
    pub fn is_open(&self, panel: Panel) -> bool {
        self.main == Some(panel) || self.dialog == Some(panel)
    }

    /// Un panneau de la barre d'outils est ouvert :
    /// c'est ce qui grise Démarrer, Réinitialiser et Investigation.
    pub fn toolbar_panel_open(&self) -> bool {
        self.main.is_some_and(Panel::is_toolbar_panel)
    }

    /// Aucun panneau ouvert.
    pub fn is_empty(&self) -> bool {
        self.main.is_none() && self.dialog.is_none()
    }

    /// Panneaux ouverts, le principal d'abord.
    pub fn open_panels(&self) -> impl Iterator<Item = Panel> {
        self.main.into_iter().chain(self.dialog)
    }

    /// Ouvre ou ferme `panel` comme les boutons de la barre d'outils : un
    /// panneau principal remplace l'autre, rouvrir le même le ferme ; un
    /// dialogue imbriqué suit [`Self::open_dialog`]. Rend vrai si `panel`
    /// est ouvert ensuite.
    pub fn toggle(&mut self, panel: Panel) -> bool {
        if self.is_open(panel) {
            self.close(panel);
            return false;
        }
        if panel.is_dialog() {
            return self.open_dialog(panel);
        }
        self.main = Some(panel);
        self.dialog = None;
        true
    }

    /// Ouvre un dialogue imbriqué ; refusé (faux) sans parent ouvert.
    pub fn open_dialog(&mut self, dialog: Panel) -> bool {
        if !dialog.is_dialog()
            || !self
                .main
                .is_some_and(|main| dialog.parents().contains(&main))
        {
            return false;
        }
        self.dialog = Some(dialog);
        true
    }

    /// Ferme `panel` (et son dialogue imbriqué s'il est principal). Rend vrai
    /// si quelque chose a changé.
    pub fn close(&mut self, panel: Panel) -> bool {
        if self.main == Some(panel) {
            self.main = None;
            self.dialog = None;
            true
        } else if self.dialog == Some(panel) {
            self.dialog = None;
            true
        } else {
            false
        }
    }

    /// Ferme tout. Rend vrai si quelque chose a changé.
    pub fn close_all(&mut self) -> bool {
        let changed = !self.is_empty();
        *self = Self::default();
        changed
    }
}

/// Construit la vue d'un panneau qui s'ouvre : **le point de branchement
/// des panneaux**. Chaque bras rend la vue de son panneau.
///
/// ```ignore
/// Panel::Config => cx.new(|cx| ConfigPanel::new(state.clone(), window, cx)).into(),
/// ```
pub fn build_panel_view(
    panel: Panel,
    state: &Entity<AppState>,
    window: &mut Window,
    cx: &mut App,
) -> AnyView {
    use crate::app::labels;
    match panel {
        Panel::Config => cx
            .new(|cx| config::ConfigPanel::new(state.clone(), window, cx))
            .into(),
        Panel::Filter => cx
            .new(|cx| filter::FilterPanel::new(state.clone(), window, cx))
            .into(),
        Panel::Import(ImportMode::Capture) => cx
            .new(|cx| import_capture::CaptureImportPanel::new(state, window, cx))
            .into(),
        Panel::Import(ImportMode::Labels) => cx
            .new(|cx| labels::import::LabelImportPanel::new(state.clone(), window, cx))
            .into(),
        Panel::Labels => cx
            .new(|cx| labels::manage::LabelsPanel::new(state.clone(), window, cx))
            .into(),
        Panel::Conflicts => cx
            .new(|cx| labels::conflicts::ConflictDialog::new(state.clone(), window, cx))
            .into(),
        Panel::Arbitration => cx
            .new(|cx| labels::arbitration::ArbitrationDialog::new(state.clone(), window, cx))
            .into(),
        Panel::MatrixLabels => cx
            .new(|cx| labels::matrix::MatrixLabelsPanel::new(state.clone(), window, cx))
            .into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LABELS_IMPORT: Panel = Panel::Import(ImportMode::Labels);

    #[test]
    fn un_seul_panneau_principal() {
        let mut state = PanelState::default();
        assert!(state.toggle(Panel::Config));
        assert!(state.toggle(Panel::Filter));
        assert_eq!(state.main(), Some(Panel::Filter));
        assert!(!state.is_open(Panel::Config));
        assert!(state.toolbar_panel_open());
    }

    #[test]
    fn rouvrir_ferme() {
        let mut state = PanelState::default();
        state.toggle(Panel::Labels);
        assert!(!state.toggle(Panel::Labels));
        assert!(state.is_empty());
        assert!(!state.toolbar_panel_open());
    }

    #[test]
    fn deux_modes_d_import_distincts() {
        let mut state = PanelState::default();
        state.toggle(Panel::Import(ImportMode::Capture));
        assert!(
            state.toggle(LABELS_IMPORT),
            "l'autre mode remplace le premier"
        );
        assert_eq!(state.main(), Some(LABELS_IMPORT));
    }

    #[test]
    fn dialogue_imbrique_seulement_avec_son_parent() {
        let mut state = PanelState::default();
        assert!(!state.open_dialog(Panel::Conflicts), "sans parent");
        state.toggle(Panel::Config);
        assert!(!state.open_dialog(Panel::Conflicts), "mauvais parent");
        state.toggle(LABELS_IMPORT);
        assert!(state.open_dialog(Panel::Conflicts));
        assert!(
            state.open_dialog(Panel::Arbitration),
            "l'arbitrage remplace les conflits"
        );
        assert_eq!(state.dialog(), Some(Panel::Arbitration));
        assert_eq!(
            state.open_panels().collect::<Vec<_>>(),
            [LABELS_IMPORT, Panel::Arbitration]
        );
    }

    #[test]
    fn fermer_le_parent_ferme_le_dialogue() {
        let mut state = PanelState::default();
        state.toggle(Panel::Labels);
        state.open_dialog(Panel::Arbitration);
        assert!(state.close(Panel::Labels));
        assert!(state.is_empty());
    }

    #[test]
    fn fermer_le_dialogue_garde_le_parent() {
        let mut state = PanelState::default();
        state.toggle(Panel::Labels);
        state.toggle(Panel::Arbitration);
        assert!(!state.toggle(Panel::Arbitration));
        assert_eq!(state.main(), Some(Panel::Labels));
        assert_eq!(state.dialog(), None);
    }

    #[test]
    fn changer_de_panneau_ferme_le_dialogue() {
        let mut state = PanelState::default();
        state.toggle(LABELS_IMPORT);
        state.open_dialog(Panel::Conflicts);
        state.toggle(Panel::Filter);
        assert_eq!(state.dialog(), None);
    }

    #[test]
    fn labels_de_la_matrice_hors_barre_d_outils() {
        let mut state = PanelState::default();
        state.toggle(Panel::Config);
        assert!(
            state.toggle(Panel::MatrixLabels),
            "remplace le panneau ouvert"
        );
        assert!(!state.toolbar_panel_open(), "ne grise pas Démarrer");
        assert_eq!(Panel::MatrixLabels.presentation(), Presentation::Modal);
        assert!(!state.close(Panel::Config));
        assert!(state.close_all());
        assert!(!state.close_all());
    }

    #[test]
    fn titres_des_panneaux() {
        assert_eq!(Panel::Config.title(), "Configuration Capture");
        assert_eq!(Panel::Filter.title(), "Filtre BPF");
        assert_eq!(LABELS_IMPORT.title(), "Import de labels");
    }
}
