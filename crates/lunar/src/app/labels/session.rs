//! Données partagées entre un panneau de labels et ses dialogues imbriqués.
//!
//! Chaque dialogue est une vue à part (voir
//! [`crate::app::panels::Panel::is_dialog`]) : le panneau dépose dans
//! [`LabelSession`] ce que le dialogue affichera, puis l'ouvre ; le dialogue
//! annonce l'arbitrage appliqué par [`LabelSessionEvent`].

use gpui::{App, AppContext, Entity, EventEmitter, Global};
use lunar_engine::commandes::import::labels::conflicts::LabelConflictReport;
use lunar_ui_logic::label_import::LabelFileIssues;

/// Événement de la session de labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelSessionEvent {
    /// Les arbitrages ont été appliqués (`@resolved`) : le panneau parent
    /// relit ses lignes.
    Resolved,
}

/// Ce que montrent les dialogues imbriqués des panneaux de labels.
#[derive(Debug, Default)]
pub struct LabelSession {
    /// Conflits présentés à l'arbitrage.
    arbitration: Vec<LabelConflictReport>,
    /// Erreur bloquante du dernier import, pour le dialogue des conflits.
    issues: Option<LabelFileIssues>,
}

impl EventEmitter<LabelSessionEvent> for LabelSession {}

/// Session unique de l'application.
struct GlobalLabelSession(Entity<LabelSession>);

impl Global for GlobalLabelSession {}

impl LabelSession {
    /// Session de l'application, créée au premier appel.
    pub fn global(cx: &mut App) -> Entity<Self> {
        if let Some(session) = cx.try_global::<GlobalLabelSession>() {
            return session.0.clone();
        }
        let session = cx.new(|_| Self::default());
        cx.set_global(GlobalLabelSession(session.clone()));
        session
    }

    /// Conflits à présenter à l'arbitrage.
    pub fn arbitration(&self) -> &[LabelConflictReport] {
        &self.arbitration
    }

    /// Dépose les conflits que le dialogue d'arbitrage présentera.
    pub fn set_arbitration(&mut self, conflicts: Vec<LabelConflictReport>) {
        self.arbitration = conflicts;
    }

    /// Erreur bloquante à montrer.
    pub fn issues(&self) -> Option<&LabelFileIssues> {
        self.issues.as_ref()
    }

    /// Dépose l'erreur que le dialogue des conflits montrera.
    pub fn set_issues(&mut self, issues: Option<LabelFileIssues>) {
        self.issues = issues;
    }
}
