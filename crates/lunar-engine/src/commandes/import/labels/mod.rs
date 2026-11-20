//! Import des fichiers de labels CSV : lecture, validation de format,
//! déduplication « premier gagné », arbitrage des conflits et application
//! des labels au store, à la matrice et au graphe.
//!
//! Découpé en : [`csv`] (lecture/parsing), [`validation`] (format MAC/IP),
//! [`conflicts`] (déduplication et types de conflit), [`commands`] (les
//! commandes qui orchestrent le tout) et [`winaudit`] (import de
//! relevés WinAudit, qui ajoute ses labels au lieu de remplacer le store).

pub mod commands;
pub mod conflicts;
pub mod csv;
pub mod validation;
pub mod winaudit;

pub use commands::{
    clear_label_store, copy_labels_to_matrix, get_label_conflicts, get_label_rows,
    import_label_file, labels_to_matrix, resolve_label_conflicts,
};
pub use conflicts::LabelConflictStore;
pub use validation::normalize_label_key;
pub use winaudit::import_winaudit_surveys;

#[cfg(test)]
mod tests {
    use super::commands::copy_labels_to_matrix;
    use super::conflicts::dedup_labels_first_wins;
    use super::csv::{is_header_row, read_label_rows, verif_label_rows_format};
    use super::validation::verif_mac_ip_format;
    use crate::state::{flow_matrix::FlowMatrix, labels_list::LabelStore};

    /// La fixture VAE Forge utilise les noms de colonnes explicites
    /// `adresse_mac,adresse_ip,label`. Elle doit suivre la même chaîne que la
    /// commande d'import et produire 18 labels basés uniquement sur l'IP.
    #[test]
    fn forge_ip_address_label_file_imports() {
        let label_path = crate::test_support::fixtures()
            .join("Adresses_IP_Forge.csv")
            .to_str()
            .unwrap()
            .to_string();

        verif_label_rows_format(label_path.clone()).unwrap();
        verif_mac_ip_format(label_path.clone()).unwrap();

        let mut rows = read_label_rows(&label_path).unwrap();
        assert!(rows.first().is_some_and(is_header_row));
        rows.remove(0);

        let (rows, conflicts) = dedup_labels_first_wins(rows);
        assert!(conflicts.is_empty());
        assert_eq!(rows.len(), 18);

        let mut label_store = LabelStore::new();
        for row in rows {
            label_store.add(row.into_tuple());
        }

        assert_eq!(label_store.get().len(), 18);
        assert_eq!(
            label_store.get().first(),
            Some(&(String::new(), "192.168.1.60".into(), "Bind9".into()))
        );
        assert_eq!(
            label_store.get().last(),
            Some(&(String::new(), "192.168.1.200".into(), "PF_Tests".into()))
        );

        let mut matrix = FlowMatrix::new();
        copy_labels_to_matrix(&label_store, &mut matrix).unwrap();
        assert_eq!(
            matrix.get_label("aa:bb:cc:dd:ee:ff", "192.168.1.150"),
            Some("Forge".into())
        );
    }

    /// Rejoue la chaîne complète de `import_label_file` sur les fichiers réels
    /// de `fixtures/` : vérifications, écart de l'en-tête, remplissage du
    /// store puis résolution des labels comme le ferait la matrice de flux.
    #[test]
    fn import_real_label_file_resolves_labels_for_matrix_hosts() {
        let label_path = crate::test_support::fixtures()
            .join("20260703_NP_Labels.csv")
            .to_str()
            .unwrap()
            .to_string();

        verif_label_rows_format(label_path.clone()).unwrap();
        verif_mac_ip_format(label_path.clone()).unwrap();
        super::conflicts::verif_labels_conflicts(label_path.clone()).unwrap();

        let read = super::csv::read_label_rows_report(&label_path).unwrap();
        // La ligne `,2606:50c0:8002::154,` n'a pas de label : ignorée et
        // comptée, elle ne devient plus « Label? ».
        assert_eq!(read.without_label, 1);
        let mut rows = read.rows;
        if rows.first().is_some_and(is_header_row) {
            rows.remove(0);
        }

        let mut label_store = LabelStore::new();
        for row in rows {
            label_store.add(row.into_tuple());
        }
        assert_eq!(
            label_store.get().len(),
            8,
            "l'en-tête et la ligne sans label doivent être écartés"
        );

        let mut matrix = FlowMatrix::new();
        copy_labels_to_matrix(&label_store, &mut matrix).unwrap();

        // Correspondance exacte MAC + IP.
        assert_eq!(
            matrix.get_label("e0:d5:5e:28:9b:d4", "192.168.1.181"),
            Some(String::from("PC Lunar"))
        );
        assert_eq!(
            matrix.get_label("44:15:24:20:a5:64", "192.168.1.254"),
            Some(String::from("Passerelle"))
        );
        // Repli sur l'IP seule, prioritaire sur la MAC seule.
        assert_eq!(
            matrix.get_label("e0:d5:5e:28:9b:d4", "2001:861:3fc7:9b00:e09f:22b:19f7:888e"),
            Some(String::from("PC Lunar public"))
        );
        // Repli sur la MAC seule quand l'IP est inconnue du fichier.
        assert_eq!(
            matrix.get_label("e0:d5:5e:28:9b:d4", "fe80::b861:e852:5fa3:d3e5"),
            Some(String::from("Machine de capture"))
        );
        // La notation CIDR du fichier est ramenée à l'adresse seule.
        assert_eq!(
            matrix.get_label("44:15:24:20:a5:64", "2600:1901:0:3084::"),
            Some(String::from("Serveur GCP (CIDR)"))
        );
        // Une ligne sans label ne pose aucun label : l'hôte n'est
        // nommé que si une autre règle le couvre, jamais « Label? ».
        assert_ne!(
            matrix.get_label("44:15:24:20:a5:64", "2606:50c0:8002::154"),
            Some(String::from("Label?"))
        );
        // Hôte absent du fichier de labels : aucun label.
        assert_eq!(matrix.get_label("44:15:24:20:a5:64", "2607:6bc0::10"), None);
    }

    /// Le fichier « à compléter » réimporté tel quel redonne
    /// exactement les labels en place, et ses lignes vides sont ignorées et
    /// comptées — aucun « Label? », aucun label perdu.
    #[test]
    fn file_to_complete_round_trips_without_alteration() {
        let fixtures = crate::test_support::fixtures();
        let mut matrix =
            lunar_flows_core::csv::merge_matrix_files(&[fixtures.join("20260703_NP_Matrice.csv")])
                .unwrap();
        let first = matrix.unlabeled_endpoints()[0].clone();
        matrix.add_label(first.mac.clone(), first.ip.clone(), "Automate".to_string());

        let (rows, unlabeled) = crate::commandes::export::labels::rows_to_complete(&matrix);
        assert_eq!(unlabeled, matrix.unlabeled_endpoints().len());
        assert!(unlabeled > 0);
        assert!(
            rows[..unlabeled]
                .iter()
                .all(|(_, _, label)| label.is_empty())
        );

        let dir = std::env::temp_dir().join(format!("lunar_a_completer_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("labels-a-completer.csv");
        FlowMatrix::write_label_rows_to_csv(&rows, &path.to_string_lossy()).unwrap();

        let read = super::csv::read_label_rows_report(&path.to_string_lossy()).unwrap();
        assert_eq!(read.without_label, unlabeled);
        let mut reimported: Vec<(String, String, String)> = read
            .rows
            .into_iter()
            .filter(|row| !is_header_row(row))
            .map(|row| row.into_tuple())
            .collect();
        reimported.sort();
        let mut expected = matrix.export_labels();
        expected.sort();
        assert_eq!(
            reimported, expected,
            "les labels en place reviennent intacts"
        );
        assert!(reimported.iter().all(|(_, _, label)| label != "Label?"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
