//! Interface native de LUNAR sur gpui 0.2.2 : thème, ressources embarquées,
//! composants réutilisables et contournements de plateforme.
//!
//! - [`theme`] : jetons du thème sombre ;
//! - [`assets`] : icônes et images embarquées ;
//! - [`widgets`] : boutons, champs, listes, tableaux, menus, modales… ;
//! - [`graph_view`] : vue du graphe réseau (rendu, ForceAtlas2, export PNG) ;
//! - [`platform`] : contournements X11 / Wayland de gpui 0.2.2 ;
//! - [`app`] : coquille de l'application (état, pont moteur, fenêtre,
//!   actions, journal, ligne de commande).

pub mod app;
pub mod assets;
pub mod graph_view;
pub mod platform;
pub mod theme;
pub mod widgets;
