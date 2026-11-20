//! Noms de fichiers proposés par les dialogues d'enregistrement : archive de
//! support horodatée et exports datés (matrice, labels, projet, graphe,
//! synthèse).
//!
//! L'heure est un paramètre : l'appelant lit l'horloge locale, les fonctions
//! restent testables.

/// Date et heure locales, champs tels qu'affichés (mois et jour à partir de 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalDateTime {
    /// Année (`2026`).
    pub year: i32,
    /// Mois, 1 à 12.
    pub month: u8,
    /// Jour du mois, 1 à 31.
    pub day: u8,
    /// Heure, 0 à 23.
    pub hour: u8,
    /// Minute, 0 à 59.
    pub minute: u8,
    /// Seconde, 0 à 59.
    pub second: u8,
}

impl LocalDateTime {
    /// Date et heure à partir de leurs champs (mois et jour à partir de 1).
    pub const fn new(year: i32, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> Self {
        Self {
            year,
            month,
            day,
            hour,
            minute,
            second,
        }
    }
}

/// Date compacte `AAAAMMJJ`, préfixe des exports datés.
pub fn compact_date(now: &LocalDateTime) -> String {
    format!("{}{:02}{:02}", now.year, now.month, now.day)
}

/// Nom d'archive de support daté, en heure locale :
/// `lunar-support-AAAAMMJJ-HHMMSS.zip`.
///
/// Un nom fixe (`lunar-logs.zip`) faisait viser le même fichier à deux
/// exports successifs, donc écraser silencieusement le précédent, et rien
/// n'indiquait à la réception de quand datait l'archive. L'heure locale
/// plutôt qu'UTC parce que ce nom est lu par la personne qui exporte, pour
/// le rapprocher de l'heure à laquelle elle a vu l'incident.
pub fn support_archive_name(now: &LocalDateTime) -> String {
    format!(
        "lunar-support-{}-{:02}{:02}{:02}.zip",
        compact_date(now),
        now.hour,
        now.minute,
        now.second
    )
}

/// Export de la matrice de flux : `AAAAMMJJ_DR_Matrice.csv`.
pub fn flow_matrix_csv_name(now: &LocalDateTime) -> String {
    format!("{}_DR_Matrice.csv", compact_date(now))
}

/// Export des labels (barre d'outils et panneau des labels) :
/// `AAAAMMJJ_labels.csv`.
pub fn labels_csv_name(now: &LocalDateTime) -> String {
    format!("{}_labels.csv", compact_date(now))
}

/// Enregistrement du projet : `AAAAMMJJ_projet.lunar`, proposé dans le
/// dossier du projet le plus récent quand il y en a un
/// ([`crate::recent_projects::UiPreferences::last_project_dir`]).
pub fn project_file_name(now: &LocalDateTime) -> String {
    format!("{}_projet.lunar", compact_date(now))
}

/// Export PNG du graphe affiché : `AAAAMMJJ_network_graph_DR_Matrice.png`.
pub fn network_graph_png_name(now: &LocalDateTime) -> String {
    format!("{}_network_graph_DR_Matrice.png", compact_date(now))
}

/// Export de la synthèse (vue d'investigation) : `AAAAMMJJ_DR_Synthese.html`.
pub fn synthesis_html_name(now: &LocalDateTime) -> String {
    format!("{}_DR_Synthese.html", compact_date(now))
}

/// Export des labels à compléter (vue d'investigation) :
/// `AAAAMMJJ_DR_labels_a_completer.csv`.
pub fn labels_to_complete_csv_name(now: &LocalDateTime) -> String {
    format!("{}_DR_labels_a_completer.csv", compact_date(now))
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Archive de support ---

    #[test]
    fn horodatage_complet_et_extension_zip() {
        let name = support_archive_name(&LocalDateTime::new(2026, 8, 9, 14, 5, 3));
        assert_eq!(name, "lunar-support-20260809-140503.zip");
    }

    #[test]
    fn mois_jour_et_heures_sur_deux_chiffres() {
        // Un `1` au lieu de `01` casserait le tri alphabétique des archives,
        // la seule chose dont dispose quelqu'un qui en reçoit plusieurs.
        let name = support_archive_name(&LocalDateTime::new(2026, 1, 2, 3, 4, 5));
        assert_eq!(name, "lunar-support-20260102-030405.zip");
    }

    #[test]
    fn deux_exports_a_une_seconde_d_ecart_different() {
        let first = support_archive_name(&LocalDateTime::new(2026, 8, 9, 14, 5, 3));
        let second = support_archive_name(&LocalDateTime::new(2026, 8, 9, 14, 5, 4));
        assert_ne!(first, second);
    }

    // --- Noms datés des exports ---

    #[test]
    fn date_compacte_sur_huit_chiffres() {
        assert_eq!(
            compact_date(&LocalDateTime::new(2024, 3, 10, 0, 0, 0)),
            "20240310"
        );
        assert_eq!(
            compact_date(&LocalDateTime::new(2026, 12, 31, 23, 59, 59)),
            "20261231"
        );
    }

    #[test]
    fn noms_des_exports_dates() {
        let now = LocalDateTime::new(2026, 11, 9, 8, 0, 0);
        assert_eq!(flow_matrix_csv_name(&now), "20261109_DR_Matrice.csv");
        assert_eq!(labels_csv_name(&now), "20261109_labels.csv");
        assert_eq!(project_file_name(&now), "20261109_projet.lunar");
        assert_eq!(
            network_graph_png_name(&now),
            "20261109_network_graph_DR_Matrice.png"
        );
        assert_eq!(synthesis_html_name(&now), "20261109_DR_Synthese.html");
        assert_eq!(
            labels_to_complete_csv_name(&now),
            "20261109_DR_labels_a_completer.csv"
        );
    }
}
