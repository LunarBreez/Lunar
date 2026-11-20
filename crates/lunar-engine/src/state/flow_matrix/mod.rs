//! Matrice de flux : réexport du cœur `lunar-flows-core`, source unique du
//! domaine. Le moteur n'est
//! qu'un adaptateur : commandes de la façade → fonctions du cœur →
//! événements UI.

pub use lunar_flows_core::matrix::*;
