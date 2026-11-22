//! Dialogues de fichiers des labels (GTK par rfd, sans bloquer la boucle de
//! rendu) : titres et filtres des imports et exports de labels.

use std::future::Future;
use std::path::Path;

use lunar_ui_logic::file_types::LABEL_EXTENSIONS;
use lunar_ui_logic::winaudit_import::{WINAUDIT_FILTER_NAME, WINAUDIT_SURVEY_EXTENSIONS};

/// Chemin choisi, en texte (les commandes du moteur prennent des `String`).
fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Ajoute `.extension` à un chemin qui n'en a pas : le dialogue GTK rend le
/// nom tel que saisi (le harnais E2E saisit `labels-export` et attend
/// `labels-export.csv`).
pub fn with_default_extension(path: &str, extension: &str) -> String {
    if Path::new(path).extension().is_some() {
        path.to_owned()
    } else {
        format!("{path}.{extension}")
    }
}

/// Choix d'un fichier de labels (CSV ou classeur Excel), `title` facultatif,
/// filtre `filter`.
pub(crate) fn pick_label_file(
    title: Option<&str>,
    filter: &str,
) -> impl Future<Output = Option<String>> + use<> {
    let mut dialog = rfd::AsyncFileDialog::new().add_filter(filter, LABEL_EXTENSIONS);
    if let Some(title) = title {
        dialog = dialog.set_title(title);
    }
    let picked = dialog.pick_file();
    async move { picked.await.map(|file| path_text(file.path())) }
}

/// Choix d'un ou plusieurs relevés WinAudit.
pub(crate) fn pick_winaudit_surveys() -> impl Future<Output = Vec<String>> + use<> {
    let picked = rfd::AsyncFileDialog::new()
        .add_filter(WINAUDIT_FILTER_NAME, WINAUDIT_SURVEY_EXTENSIONS)
        .pick_files();
    async move {
        picked
            .await
            .unwrap_or_default()
            .iter()
            .map(|file| path_text(file.path()))
            .collect()
    }
}

/// Enregistrement d'un CSV : titre, nom proposé ; `.csv` ajouté si le nom
/// saisi n'a pas d'extension.
pub(crate) fn save_csv(
    title: &str,
    file_name: &str,
) -> impl Future<Output = Option<String>> + use<> {
    let picked = rfd::AsyncFileDialog::new()
        .set_title(title)
        .add_filter(".csv", &["csv"])
        .set_file_name(file_name)
        .save_file();
    async move {
        picked
            .await
            .map(|file| with_default_extension(&path_text(file.path()), "csv"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_ajoutee_seulement_si_absente() {
        assert_eq!(
            with_default_extension("/tmp/labels-export", "csv"),
            "/tmp/labels-export.csv"
        );
        assert_eq!(
            with_default_extension("/tmp/20261119_labels.csv", "csv"),
            "/tmp/20261119_labels.csv"
        );
        assert_eq!(
            with_default_extension("/tmp/export.txt", "csv"),
            "/tmp/export.txt"
        );
        assert_eq!(
            with_default_extension("/tmp/dossier.v2/export", "csv"),
            "/tmp/dossier.v2/export.csv"
        );
    }
}
