//! Calculs dérivés de la progression d'import affichée dans l'overlay de
//! conversion : pourcentage global toutes pièces et nom du fichier en cours.

/// Progression d'import : les champs de l'événement `ImportProgress` du
/// moteur (`CaptureEvent::ImportProgress`, mêmes types).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ImportProgress {
    /// Chemin du fichier en cours, tel qu'émis par le moteur.
    pub file_name: String,
    /// Rang du fichier en cours, à partir de 1.
    pub file_index: u64,
    /// Nombre de fichiers de l'import.
    pub files_total: u64,
    /// Avancement dans le fichier en cours (paquets ou lignes).
    pub current: u64,
    /// Total du fichier en cours, 0 si inconnu.
    pub total: u64,
}

/// Pourcentage global de l'import, borné à `0..=100`. Sans progression ou
/// sans fichier annoncé : 0. Un fichier de taille inconnue (`total` à 0)
/// compte comme déjà consommé plutôt que de diviser par zéro.
pub fn progress_percent_of(progress: Option<&ImportProgress>) -> f64 {
    let Some(progress) = progress.filter(|p| p.files_total > 0) else {
        return 0.0;
    };
    let file_ratio = if progress.total > 0 {
        (progress.current as f64 / progress.total as f64).clamp(0.0, 1.0)
    } else {
        1.0
    };
    let global_ratio =
        (progress.file_index as f64 - 1.0 + file_ratio) / progress.files_total as f64;
    (global_ratio * 100.0).clamp(0.0, 100.0)
}

/// Nom du fichier en cours, sans son dossier (séparateurs `/` et `\`
/// confondus) ; chaîne vide sans progression.
pub fn progress_file_name_of(progress: Option<&ImportProgress>) -> &str {
    progress.map_or("", |p| {
        p.file_name
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(&p.file_name)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Progression d'un fichier unique `capture.pcap` au début.
    fn progress() -> ImportProgress {
        ImportProgress {
            file_name: "capture.pcap".to_owned(),
            file_index: 1,
            files_total: 1,
            current: 0,
            total: 100,
        }
    }

    #[test]
    fn sans_progression_0() {
        assert_eq!(progress_percent_of(None), 0.0);
    }

    #[test]
    fn files_total_nul_0() {
        let p = ImportProgress {
            files_total: 0,
            ..progress()
        };
        assert_eq!(progress_percent_of(Some(&p)), 0.0);
    }

    #[test]
    fn un_seul_fichier_a_mi_parcours() {
        let p = ImportProgress {
            current: 50,
            total: 100,
            ..progress()
        };
        assert_eq!(progress_percent_of(Some(&p)), 50.0);
    }

    #[test]
    fn fichier_termine_sur_un_import_multi_fichiers() {
        // Fichier 1/2 terminé : la moitié de l'import.
        let p = ImportProgress {
            file_index: 1,
            files_total: 2,
            current: 100,
            total: 100,
            ..progress()
        };
        assert_eq!(progress_percent_of(Some(&p)), 50.0);
    }

    #[test]
    fn total_nul_traite_comme_un_fichier_deja_complet() {
        let p = ImportProgress {
            file_index: 1,
            files_total: 2,
            current: 0,
            total: 0,
            ..progress()
        };
        assert_eq!(progress_percent_of(Some(&p)), 50.0);
    }

    #[test]
    fn borne_entre_0_et_100() {
        // Les champs étant non signés comme dans l'événement du moteur, la
        // borne basse est atteinte par un rang de fichier à 0 (ratio global
        // négatif).
        let below = ImportProgress {
            file_index: 0,
            current: 10,
            total: 100,
            ..progress()
        };
        assert_eq!(progress_percent_of(Some(&below)), 0.0);
        let above = ImportProgress {
            current: 1000,
            total: 100,
            ..progress()
        };
        assert_eq!(progress_percent_of(Some(&above)), 100.0);
        let beyond = ImportProgress {
            file_index: 5,
            files_total: 2,
            ..progress()
        };
        assert_eq!(progress_percent_of(Some(&beyond)), 100.0);
    }

    #[test]
    fn nom_de_fichier_sans_progression_vide() {
        assert_eq!(progress_file_name_of(None), "");
    }

    #[test]
    fn ne_garde_que_le_nom_de_fichier() {
        let posix = ImportProgress {
            file_name: "/home/user/capture.pcap".to_owned(),
            ..progress()
        };
        assert_eq!(progress_file_name_of(Some(&posix)), "capture.pcap");
        let windows = ImportProgress {
            file_name: "C:\\captures\\reseau.pcap".to_owned(),
            ..progress()
        };
        assert_eq!(progress_file_name_of(Some(&windows)), "reseau.pcap");
    }

    #[test]
    fn sans_separateur_renvoie_le_nom_tel_quel() {
        assert_eq!(progress_file_name_of(Some(&progress())), "capture.pcap");
    }
}
