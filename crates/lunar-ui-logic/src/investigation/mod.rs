//! Vue d'investigation et page de synthèse HTML.
//!
//! | Module | Rôle |
//! |---|---|
//! | [`tables`], [`period`] | tableaux, KPI et période du relevé |
//! | [`view`] | logique de la vue (onglets, filtres, tri) |
//! | [`synthesis_html`] | page de synthèse HTML |
//! | [`glossary`] | glossaire embarqué |
//! | [`deductions`] | déductions OUI et adresses connues |
//! | [`ports`] | services probables par port |
//! | [`protocol_traits`] | usages et chiffrement par protocole |
//! | [`protocol_colors`] | couleurs de protocole |
//! | [`dates`] | dates au sens de `Date` JavaScript |
//! | [`locale`] | `localeCompare`, `toLocaleString("fr-FR")`, `toFixed` |
//! | [`graph_view`] | ordre d'itération du graphe (cf. ce module) |
//!
//! Les tests de `tests/golden.rs` comparent la page de synthèse et les
//! tableaux, à l'octet, à des fichiers de référence figés.

pub mod dates;
pub mod deductions;
pub mod glossary;
pub mod graph_view;
pub mod locale;
pub mod period;
pub mod ports;
/// Couleurs de protocole : la table unique du graphe.
pub use crate::graph::protocol_colors;
pub mod protocol_traits;
pub mod synthesis_html;
pub mod tables;
pub mod view;

#[cfg(test)]
mod test_support;

pub use dates::{EpochMillis, FixedZone, LocalZone};
pub use graph_view::GraphView;
pub use period::{PeriodSummary, SurveyPeriodView};
pub use synthesis_html::{SynthesisExport, render_synthesis_html};
pub use view::{InvestigationModel, ViewState};
