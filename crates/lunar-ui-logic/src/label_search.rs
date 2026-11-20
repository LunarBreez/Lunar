//! Filtrage plein texte de la table des labels importés : recherche
//! insensible à la casse sur les champs MAC, IP et label de chaque ligne.

/// Lignes dont au moins un champ (MAC, IP, label) contient `search`, sans
/// tenir compte de la casse, dans l'ordre d'origine. Recherche vide : toutes
/// les lignes.
pub fn filter_label_rows<'a, T: AsRef<str>>(rows: &'a [[T; 3]], search: &str) -> Vec<&'a [T; 3]> {
    let needle = search.to_lowercase();
    rows.iter()
        .filter(|row| {
            row.iter()
                .any(|field| field.as_ref().to_lowercase().contains(&needle))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Table de labels des tests.
    const ROWS: [[&str; 3]; 3] = [
        ["aa:bb:cc:dd:ee:ff", "192.168.1.1", "PC Lunar"],
        ["11:22:33:44:55:66", "192.168.1.2", "Imprimante"],
        ["77:88:99:aa:bb:cc", "10.0.0.5", "Passerelle"],
    ];

    #[test]
    fn recherche_vide_renvoie_tout() {
        let all: Vec<_> = ROWS.iter().collect();
        assert_eq!(filter_label_rows(&ROWS, ""), all);
    }

    #[test]
    fn filtre_sur_le_label() {
        assert_eq!(filter_label_rows(&ROWS, "lunar"), [&ROWS[0]]);
    }

    #[test]
    fn filtre_sur_l_ip() {
        assert_eq!(filter_label_rows(&ROWS, "10.0.0"), [&ROWS[2]]);
    }

    #[test]
    fn filtre_sur_la_mac_insensible_a_la_casse() {
        assert_eq!(filter_label_rows(&ROWS, "AA:BB:CC:DD"), [&ROWS[0]]);
    }

    #[test]
    fn aucune_correspondance_tableau_vide() {
        assert!(filter_label_rows(&ROWS, "introuvable").is_empty());
    }

    // --- Compléments ---

    #[test]
    fn lignes_possedees_et_casse_des_accents() {
        let rows = vec![[
            "00:00:00:00:00:01".to_owned(),
            "10.1.1.1".to_owned(),
            "Équipement ÉTÉ".to_owned(),
        ]];
        assert_eq!(filter_label_rows(&rows, "été").len(), 1);
    }
}
