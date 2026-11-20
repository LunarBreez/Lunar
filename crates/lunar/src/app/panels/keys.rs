//! Clavier des panneaux latéraux : Échap ferme le panneau.
//!
//! Le panneau pose [`SIDE_PANEL_CONTEXT`] sur sa racine, la focalise à
//! l'ouverture et traite
//! [`crate::widgets::Cancel`]. Une liste déroulante ouverte garde la main :
//! Échap la ferme d'abord.

use gpui::{App, Global, KeyBinding};

use crate::widgets::Cancel;

/// Contexte clavier des panneaux latéraux.
pub const SIDE_PANEL_CONTEXT: &str = "LunarSidePanel";

/// Marque : raccourcis des panneaux déjà installés.
struct SidePanelKeys;

impl Global for SidePanelKeys {}

/// Installe Échap → [`Cancel`] dans [`SIDE_PANEL_CONTEXT`], une seule fois.
pub fn ensure_side_panel_keys(cx: &mut App) {
    if cx.has_global::<SidePanelKeys>() {
        return;
    }
    cx.bind_keys([KeyBinding::new("escape", Cancel, Some(SIDE_PANEL_CONTEXT))]);
    cx.set_global(SidePanelKeys);
}
