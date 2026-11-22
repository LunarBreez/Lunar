//! Extensions reconnues par le panneau d'import et routage des fichiers
//! déposés (glisser-déposer) selon le mode du panneau.

use std::collections::HashSet;

/// Extensions des captures (insensibles à la casse).
pub const PCAP_EXTENSIONS: &[&str] = &["pcap", "pcapng", "cap"];

/// Extensions des matrices de flux (insensibles à la casse).
pub const MATRIX_EXTENSIONS: &[&str] = &["csv", "xlsx"];

/// Extensions des fichiers de labels (insensibles à la casse) : CSV, ou
/// classeur Excel aux mêmes colonnes.
pub const LABEL_EXTENSIONS: &[&str] = &["csv", "xlsx"];

/// Mode du panneau d'import.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImportMode {
    /// Import d'un fichier de labels (`.csv`, `.xlsx`).
    Csv,
    /// Import de captures et de matrices de flux.
    Pcap,
}

/// Ce qui suit le dernier `.` du chemin, en minuscules ; le chemin entier
/// s'il n'a pas de point (même règle que `path.split('.').pop()`).
fn extension_of(path: &str) -> String {
    path.rsplit('.').next().unwrap_or_default().to_lowercase()
}

/// Ajoute des chemins à une liste en écartant ceux déjà présents : un même
/// fichier resélectionné ou redéposé n'apparaît qu'une fois dans la liste
/// affichée. L'ordre d'arrivée est préservé.
///
/// Déduplication par chaîne uniquement, sans normalisation (casse, chemin
/// relatif, lien symbolique) : la garantie de fond — un fichier n'est jamais
/// compté deux fois dans la matrice — est tenue par le moteur, qui
/// canonicalise avant l'import.
pub fn append_unique_paths<E: AsRef<str>, I: AsRef<str>>(
    existing: &[E],
    incoming: &[I],
) -> Vec<String> {
    let mut seen: HashSet<&str> = HashSet::new();
    existing
        .iter()
        .map(AsRef::as_ref)
        .chain(incoming.iter().map(AsRef::as_ref))
        .filter(|path| seen.insert(path))
        .map(str::to_owned)
        .collect()
}

/// Répartition des chemins déposés sur le panneau.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DroppedPathsRouting {
    /// Mode labels : le premier `.csv` ou `.xlsx` déposé.
    Csv {
        /// Chemin retenu.
        path: String,
    },
    /// Mode import : captures et matrices, dans l'ordre de dépôt.
    Files {
        /// Captures (`.pcap`, `.pcapng`, `.cap`).
        pcaps: Vec<String>,
        /// Matrices (`.csv`, `.xlsx`).
        matrices: Vec<String>,
    },
    /// Rien d'exploitable dans ce mode.
    Ignored,
}

/// Répartit les chemins déposés selon le mode du panneau : un seul fichier
/// de labels (`.csv`, `.xlsx`) en mode labels (le premier trouvé), ou des captures et des matrices en
/// mode import. Les autres fichiers sont écartés.
pub fn route_dropped_paths(paths: &[impl AsRef<str>], mode: ImportMode) -> DroppedPathsRouting {
    let paths = paths.iter().map(AsRef::as_ref);
    match mode {
        ImportMode::Csv => paths
            .into_iter()
            .find(|path| LABEL_EXTENSIONS.contains(&extension_of(path).as_str()))
            .map_or(DroppedPathsRouting::Ignored, |path| {
                DroppedPathsRouting::Csv {
                    path: path.to_owned(),
                }
            }),
        ImportMode::Pcap => {
            let mut pcaps = Vec::new();
            let mut matrices = Vec::new();
            for path in paths {
                let extension = extension_of(path);
                if PCAP_EXTENSIONS.contains(&extension.as_str()) {
                    pcaps.push(path.to_owned());
                } else if MATRIX_EXTENSIONS.contains(&extension.as_str()) {
                    matrices.push(path.to_owned());
                }
            }
            if pcaps.is_empty() && matrices.is_empty() {
                DroppedPathsRouting::Ignored
            } else {
                DroppedPathsRouting::Files { pcaps, matrices }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn un_classeur_de_labels_depose_est_retenu_comme_un_csv() {
        assert_eq!(
            route_dropped_paths(&["notes.txt", "Labels.XLSX", "autre.csv"], ImportMode::Csv),
            DroppedPathsRouting::Csv {
                path: "Labels.XLSX".to_owned()
            }
        );
        assert_eq!(
            route_dropped_paths(&["labels.xls", "labels.ods"], ImportMode::Csv),
            DroppedPathsRouting::Ignored
        );
    }

    /// Liste de chemins possédés.
    fn owned(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|p| (*p).to_owned()).collect()
    }

    // --- Extensions et routage ---

    #[test]
    fn mode_csv_premier_csv_retenu() {
        let result =
            route_dropped_paths(&["notes.txt", "labels.csv", "autre.csv"], ImportMode::Csv);
        assert_eq!(
            result,
            DroppedPathsRouting::Csv {
                path: "labels.csv".to_owned()
            }
        );
    }

    #[test]
    fn mode_csv_aucun_csv_ignore() {
        let result = route_dropped_paths(&["capture.pcap", "notes.txt"], ImportMode::Csv);
        assert_eq!(result, DroppedPathsRouting::Ignored);
    }

    #[test]
    fn mode_csv_insensible_a_la_casse() {
        let result = route_dropped_paths(&["Labels.CSV"], ImportMode::Csv);
        assert_eq!(
            result,
            DroppedPathsRouting::Csv {
                path: "Labels.CSV".to_owned()
            }
        );
    }

    #[test]
    fn mode_pcap_separe_captures_et_matrices() {
        let result = route_dropped_paths(
            &[
                "a.pcap",
                "b.pcapng",
                "c.cap",
                "m1.csv",
                "m2.xlsx",
                "notes.txt",
            ],
            ImportMode::Pcap,
        );
        assert_eq!(
            result,
            DroppedPathsRouting::Files {
                pcaps: owned(&["a.pcap", "b.pcapng", "c.cap"]),
                matrices: owned(&["m1.csv", "m2.xlsx"]),
            }
        );
    }

    #[test]
    fn mode_pcap_rien_de_pertinent_ignore() {
        let result = route_dropped_paths(&["notes.txt", "readme.md"], ImportMode::Pcap);
        assert_eq!(result, DroppedPathsRouting::Ignored);
    }

    #[test]
    fn mode_pcap_seulement_des_matrices() {
        let result = route_dropped_paths(&["m.csv"], ImportMode::Pcap);
        assert_eq!(
            result,
            DroppedPathsRouting::Files {
                pcaps: vec![],
                matrices: owned(&["m.csv"]),
            }
        );
    }

    #[test]
    fn append_ecarte_les_chemins_deja_listes() {
        let result = append_unique_paths(
            &["/tmp/a.pcap", "/tmp/b.pcap"],
            &["/tmp/b.pcap", "/tmp/c.pcap"],
        );
        assert_eq!(result, ["/tmp/a.pcap", "/tmp/b.pcap", "/tmp/c.pcap"]);
    }

    #[test]
    fn append_dedoublonne_aussi_au_sein_de_la_selection() {
        let result = append_unique_paths::<&str, &str>(&[], &["/tmp/a.pcap", "/tmp/a.pcap"]);
        assert_eq!(result, ["/tmp/a.pcap"]);
    }

    #[test]
    fn append_preserve_l_ordre_d_arrivee() {
        let result = append_unique_paths(&["/x/1.csv"], &["/x/2.csv", "/x/1.csv", "/x/3.csv"]);
        assert_eq!(result, ["/x/1.csv", "/x/2.csv", "/x/3.csv"]);
    }

    #[test]
    fn append_chemins_distincts_conserves_sans_normalisation() {
        let result = append_unique_paths(&["/tmp/a.pcap"], &["/tmp/A.pcap"]);
        assert_eq!(result, ["/tmp/a.pcap", "/tmp/A.pcap"]);
    }

    // --- Compléments ---

    #[test]
    fn extension_apres_le_dernier_point() {
        assert_eq!(extension_of("/data/relevé.v2.PCAPNG"), "pcapng");
        assert_eq!(extension_of("sans-extension"), "sans-extension");
        assert_eq!(extension_of("/dossier.csv/fichier"), "csv/fichier");
        assert_eq!(extension_of("fin."), "");
    }
}
