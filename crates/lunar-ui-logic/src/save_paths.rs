//! Chemins des dialogues de fichiers : chemin proposé à l'enregistrement
//! (dossier du dernier projet et nom daté) et extension du chemin choisi.
//!
//! Le sélecteur GTK de rfd rend le nom tel que l'opérateur l'a tapé :
//! [`ensure_extension`] complète l'extension du filtre quand le nom n'en a
//! pas (`matrix-export` devient `matrix-export.csv`).

/// Chemin proposé par un dialogue d'enregistrement : `dossier/nom` quand un
/// dossier est connu (dernier projet), le nom seul sinon.
pub fn default_save_path(dir: Option<&str>, file_name: &str) -> String {
    match dir.filter(|dir| !dir.is_empty()) {
        Some(dir) => format!("{dir}/{file_name}"),
        None => file_name.to_owned(),
    }
}

/// Sépare un chemin proposé en dossier (s'il y en a un) et nom de fichier,
/// séparateurs `/` et `\` confondus, pour `set_directory` et
/// `set_file_name` du dialogue.
pub fn split_default_path(path: &str) -> (Option<&str>, &str) {
    match path.rfind(['/', '\\']) {
        Some(0) => (Some(&path[..1]), &path[1..]),
        Some(cut) => (Some(&path[..cut]), &path[cut + 1..]),
        None => (None, path),
    }
}

/// Complète `path` par `.extension` quand son nom de fichier n'en a aucune
/// (`matrix-export` → `matrix-export.csv`). Un nom qui porte déjà une
/// extension, même autre, est respecté : c'est le choix de l'opérateur.
pub fn ensure_extension(path: &str, extension: &str) -> String {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let has_extension = name
        .rfind('.')
        .is_some_and(|dot| dot > 0 && dot + 1 < name.len());
    if name.is_empty() || has_extension || extension.is_empty() {
        return path.to_owned();
    }
    let separator = if path.ends_with('.') { "" } else { "." };
    format!("{path}{separator}{extension}")
}

/// Nom de fichier d'un chemin (ce qui suit le dernier séparateur), pour les
/// notifications.
pub fn file_name_of(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chemin_propose_avec_ou_sans_dossier() {
        assert_eq!(
            default_save_path(Some("/releves"), "20261119_projet.lunar"),
            "/releves/20261119_projet.lunar"
        );
        assert_eq!(
            default_save_path(None, "20261119_projet.lunar"),
            "20261119_projet.lunar"
        );
        assert_eq!(default_save_path(Some(""), "a.lunar"), "a.lunar");
    }

    #[test]
    fn separation_dossier_et_nom() {
        assert_eq!(
            split_default_path("/releves/a.lunar"),
            (Some("/releves"), "a.lunar")
        );
        assert_eq!(
            split_default_path("C:\\releves\\a.lunar"),
            (Some("C:\\releves"), "a.lunar")
        );
        assert_eq!(split_default_path("/a.lunar"), (Some("/"), "a.lunar"));
        assert_eq!(split_default_path("a.lunar"), (None, "a.lunar"));
    }

    #[test]
    fn extension_completee_seulement_si_absente() {
        assert_eq!(
            ensure_extension("/tmp/matrix-export", "csv"),
            "/tmp/matrix-export.csv"
        );
        assert_eq!(ensure_extension("/tmp/export.", "csv"), "/tmp/export.csv");
        assert_eq!(ensure_extension("/tmp/m.csv", "csv"), "/tmp/m.csv");
        assert_eq!(ensure_extension("/tmp/m.CSV", "csv"), "/tmp/m.CSV");
        assert_eq!(ensure_extension("/tmp/m.txt", "csv"), "/tmp/m.txt");
        assert_eq!(
            ensure_extension("/tmp/dossier.v2/archive", "zip"),
            "/tmp/dossier.v2/archive.zip"
        );
        assert_eq!(ensure_extension("/tmp/.cache", "zip"), "/tmp/.cache.zip");
        assert_eq!(ensure_extension("/tmp/", "zip"), "/tmp/");
        assert_eq!(ensure_extension("/tmp/x", ""), "/tmp/x");
    }

    #[test]
    fn nom_de_fichier() {
        assert_eq!(file_name_of("/a/b/projet.lunar"), "projet.lunar");
        assert_eq!(file_name_of("C:\\a\\m.csv"), "m.csv");
        assert_eq!(file_name_of("seul.png"), "seul.png");
    }
}
