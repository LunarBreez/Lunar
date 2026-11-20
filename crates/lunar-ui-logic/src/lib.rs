//! Logique d'interface de LUNAR, testable sans fenêtre.
//!
//! Aucun type GPUI ici : style et synchronisation du graphe, ForceAtlas2,
//! investigation, synthèse HTML, filtre BPF, menus.

pub mod bpf;
pub mod capture_config;
pub mod capture_store;
pub mod errors;
pub mod file_names;
pub mod file_types;
pub mod graph;
pub mod import_files;
pub mod import_lifecycle;
pub mod import_progress;
pub mod investigation;
pub mod label_conflicts;
pub mod label_import;
pub mod label_management;
pub mod label_search;
pub mod live_capture;
pub mod menu;
pub mod recent_projects;
pub mod save_paths;
pub mod winaudit_import;
