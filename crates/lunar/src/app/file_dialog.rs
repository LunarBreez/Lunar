//! Dialogues de fichiers natifs (rfd, sélecteur GTK) : ouvrir un ou
//! plusieurs fichiers, choisir où enregistrer.
//!
//! Les dialogues sont asynchrones (rfd fait tourner GTK sur son propre
//! thread) : le futur rendu s'attend dans une tâche gpui sans bloquer la
//! boucle. Les chemins sont rendus en `String`, comme les commandes du
//! moteur les attendent. L'enregistrement complète l'extension du filtre
//! quand l'opérateur tape un nom sans extension
//! ([`lunar_ui_logic::save_paths::ensure_extension`]).

use std::future::Future;
use std::path::Path;

use lunar_ui_logic::file_types::{MATRIX_EXTENSIONS, PCAP_EXTENSIONS};
use lunar_ui_logic::save_paths::{ensure_extension, split_default_path};

/// Filtre d'un dialogue : libellé et extensions sans point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileFilter {
    /// Libellé affiché dans la liste des filtres.
    pub name: &'static str,
    /// Extensions acceptées.
    pub extensions: &'static [&'static str],
}

/// Captures réseau.
pub const CAPTURE_FILTER: FileFilter = FileFilter {
    name: "Capture File",
    extensions: PCAP_EXTENSIONS,
};

/// Matrices de flux.
pub const MATRIX_FILTER: FileFilter = FileFilter {
    name: "Matrice CSV ou XLSX",
    extensions: MATRIX_EXTENSIONS,
};

/// Projets LUNAR (ouvrir, enregistrer).
pub const PROJECT_FILTER: FileFilter = FileFilter {
    name: "Projet LUNAR (.lunar)",
    extensions: &["lunar"],
};

/// Export de la matrice.
pub const CSV_FILTER: FileFilter = FileFilter {
    name: ".csv",
    extensions: &["csv"],
};

/// Archive de support.
pub const ZIP_FILTER: FileFilter = FileFilter {
    name: ".zip",
    extensions: &["zip"],
};

/// Export PNG du graphe.
pub const PNG_FILTER: FileFilter = FileFilter {
    name: "PNG File",
    extensions: &["png"],
};

/// Chemin rendu par rfd, en texte.
fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Dialogue d'ouverture : un fichier (`multiple` faux) ou plusieurs.
/// `directory` préremplit le dossier affiché. Rend `None` si l'opérateur
/// annule.
pub fn pick_files(
    title: Option<&str>,
    filter: FileFilter,
    multiple: bool,
    directory: Option<&str>,
) -> impl Future<Output = Option<Vec<String>>> + 'static {
    let mut dialog = rfd::AsyncFileDialog::new().add_filter(filter.name, filter.extensions);
    if let Some(title) = title {
        dialog = dialog.set_title(title);
    }
    if let Some(directory) = directory.filter(|dir| !dir.is_empty()) {
        dialog = dialog.set_directory(directory);
    }
    async move {
        if multiple {
            let files = dialog.pick_files().await?;
            let paths: Vec<String> = files.iter().map(|f| path_text(f.path())).collect();
            (!paths.is_empty()).then_some(paths)
        } else {
            let file = dialog.pick_file().await?;
            Some(vec![path_text(file.path())])
        }
    }
}

/// Dialogue d'enregistrement proposant `default_path` (`dossier/nom` ou nom
/// seul). Le chemin choisi reçoit la première extension du filtre s'il n'en
/// a pas. Rend `None` si l'opérateur annule.
pub fn pick_save_path(
    title: Option<&str>,
    filter: FileFilter,
    default_path: &str,
) -> impl Future<Output = Option<String>> + 'static {
    let (directory, file_name) = split_default_path(default_path);
    let mut dialog = rfd::AsyncFileDialog::new()
        .add_filter(filter.name, filter.extensions)
        .set_can_create_directories(true)
        .set_file_name(file_name);
    if let Some(title) = title {
        dialog = dialog.set_title(title);
    }
    if let Some(directory) = directory {
        dialog = dialog.set_directory(directory);
    }
    let extension = filter.extensions.first().copied().unwrap_or_default();
    async move {
        let file = dialog.save_file().await?;
        Some(ensure_extension(&path_text(file.path()), extension))
    }
}
