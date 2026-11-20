//! Gestion des labels : recherche et désignation des lignes du store,
//! vérification d'une saisie avant l'appel au moteur, textes du panneau de
//! gestion et du panneau des labels de la matrice.

use lunar_engine::commandes::flow_matrix::LabelRowStatus;
use lunar_engine::commandes::import::labels::validation::normalize_label_key;
use lunar_engine::errors::CaptureStateError;
use lunar_engine::errors::label::LabelError;

/// Clé d'une ligne du store (`mac|ip`), stable pendant une édition.
pub fn row_key(mac: &str, ip: &str) -> String {
    format!("{mac}|{ip}")
}

/// Désignation lisible d'une ligne pour les noms accessibles des boutons
/// d'action : `mac / ip`, ou « ligne sans MAC ni IP ».
pub fn row_name(mac: &str, ip: &str) -> String {
    let parts: Vec<&str> = [mac, ip].into_iter().filter(|s| !s.is_empty()).collect();
    if parts.is_empty() {
        "ligne sans MAC ni IP".to_owned()
    } else {
        parts.join(" / ")
    }
}

/// Indices des lignes dont la MAC, l'IP ou le label contient `search`, sans
/// tenir compte de la casse ni des blancs de bord ;
/// recherche vide : toutes les lignes.
pub fn filter_status_rows(rows: &[LabelRowStatus], search: &str) -> Vec<usize> {
    let needle = search.trim().to_lowercase();
    rows.iter()
        .enumerate()
        .filter(|(_, row)| {
            needle.is_empty()
                || [&row.mac, &row.ip, &row.label]
                    .iter()
                    .any(|field| field.to_lowercase().contains(&needle))
        })
        .map(|(index, _)| index)
        .collect()
}

/// Sous-titre du panneau, avant et après les pastilles de statut.
pub fn subtitle_parts(count: usize) -> (String, &'static str, &'static str) {
    (
        format!("{count} label(s) — "),
        " observé dans la capture courante, ",
        " dormant. La table est l'état des labels : supprimer une ligne désétiquette l'équipement.",
    )
}

/// Texte d'état vide de la table.
pub fn empty_text(total: usize) -> &'static str {
    if total == 0 {
        "Aucun label. Ajoute une ligne ou importe un fichier."
    } else {
        "Aucun label ne correspond à la recherche."
    }
}

/// Statut d'une ligne, en mots (`aria-label` de la pastille).
pub fn status_text(observed: bool) -> &'static str {
    if observed {
        "Observé dans la capture courante"
    } else {
        "Dormant (aucun endpoint correspondant)"
    }
}

/// Libellé du bouton d'arbitrage du panneau de gestion.
pub fn arbitrate_button(count: usize) -> String {
    format!("Arbitrer {count} conflit(s)")
}

/// Titre de la confirmation de suppression.
pub const DELETE_TITLE: &str = "Supprimer le label";

/// Question de la suppression d'une ligne.
pub fn delete_question(mac: &str, ip: &str, label: &str) -> String {
    format!(
        "Supprimer le label « {label} » ({} / {}) ?\nL'équipement sera désétiqueté.",
        dash(mac),
        dash(ip)
    )
}

/// `-` pour un champ vide.
fn dash(value: &str) -> &str {
    if value.is_empty() { "-" } else { value }
}

/// Erreurs d'une saisie de label, champ par champ.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EntryErrors {
    /// MAC mal formée.
    pub mac: Option<String>,
    /// IP mal formée, ou MAC et IP toutes deux vides.
    pub ip: Option<String>,
    /// Label manquant.
    pub label: Option<String>,
}

impl EntryErrors {
    /// Aucune erreur.
    pub fn is_empty(&self) -> bool {
        self.mac.is_none() && self.ip.is_none() && self.label.is_none()
    }
}

/// Message d'une MAC refusée.
pub const INVALID_MAC: &str = "MAC invalide (aa:bb:cc:dd:ee:ff)";
/// Message d'une IP refusée.
pub const INVALID_IP: &str = "IP invalide (v4 ou v6)";
/// Message d'une clé sans MAC ni IP.
pub const EMPTY_KEY: &str = "MAC ou IP requise";
/// Message d'un label manquant (champ `required` du formulaire d'ajout).
pub const LABEL_REQUIRED: &str = "Label requis";

/// Vérifie une saisie avec les règles du moteur (`normalize_label_key` :
/// MAC dans l'une des formes acceptées, IP v4 ou v6, au moins l'une des
/// deux) avant de l'envoyer ; `require_label` pour l'ajout, dont le champ
/// label est obligatoire. Le moteur revérifie : cette étape ne fait que
/// placer le message sous le bon champ.
pub fn validate_entry(mac: &str, ip: &str, label: &str, require_label: bool) -> EntryErrors {
    let mut errors = EntryErrors::default();
    match normalize_label_key(mac, ip) {
        Ok(_) => {}
        Err(CaptureStateError::Label(LabelError::InvalidMacIpFormat {
            invalid_mac,
            invalid_ip,
        })) => {
            if !invalid_mac.is_empty() {
                errors.mac = Some(INVALID_MAC.to_owned());
            }
            if !invalid_ip.is_empty() {
                errors.ip = Some(INVALID_IP.to_owned());
            }
        }
        Err(_) => errors.ip = Some(EMPTY_KEY.to_owned()),
    }
    if require_label && label.trim().is_empty() {
        errors.label = Some(LABEL_REQUIRED.to_owned());
    }
    errors
}

/// Titre du panneau des labels de la matrice, avec leur nombre.
pub fn matrix_labels_title(count: usize) -> String {
    format!("Labels appliqués à la matrice ({count})")
}

/// Texte du panneau des labels de la matrice sans label.
pub const MATRIX_LABELS_EMPTY: &str = "Aucun label appliqué à la matrice pour le moment.";

/// Texte indicatif des champs de recherche des labels.
pub const SEARCH_PLACEHOLDER: &str = "Rechercher (MAC, IP, label)…";

/// Indices des triplets `(mac, ip, label)` dont un champ contient `search`
/// (blancs de bord ignorés, casse ignorée).
pub fn filter_triplets(rows: &[(String, String, String)], search: &str) -> Vec<usize> {
    let needle = search.trim().to_lowercase();
    rows.iter()
        .enumerate()
        .filter(|(_, (mac, ip, label))| {
            needle.is_empty()
                || [mac, ip, label]
                    .iter()
                    .any(|field| field.to_lowercase().contains(&needle))
        })
        .map(|(index, _)| index)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(mac: &str, ip: &str, label: &str, observed: bool) -> LabelRowStatus {
        LabelRowStatus {
            mac: mac.to_owned(),
            ip: ip.to_owned(),
            label: label.to_owned(),
            observed,
        }
    }

    #[test]
    fn cle_et_designation_d_une_ligne() {
        assert_eq!(row_key("aa", "1.1.1.1"), "aa|1.1.1.1");
        assert_eq!(row_name("aa:bb", "10.0.0.1"), "aa:bb / 10.0.0.1");
        assert_eq!(row_name("aa:bb", ""), "aa:bb");
        assert_eq!(row_name("", ""), "ligne sans MAC ni IP");
    }

    #[test]
    fn recherche_sur_les_trois_champs_sans_casse_ni_blancs() {
        let rows = [
            row("e0:d5:5e:28:9b:d4", "192.168.1.181", "PC Lunar", true),
            row("44:15:24:20:a5:64", "192.168.1.254", "Passerelle", false),
            row("44:15:24:20:a5:64", "", "Box", false),
        ];
        assert_eq!(filter_status_rows(&rows, ""), [0, 1, 2]);
        assert_eq!(filter_status_rows(&rows, "   "), [0, 1, 2]);
        assert_eq!(filter_status_rows(&rows, " lunar "), [0]);
        assert_eq!(filter_status_rows(&rows, "44:15"), [1, 2]);
        assert_eq!(filter_status_rows(&rows, ".254"), [1]);
        assert!(filter_status_rows(&rows, "absent").is_empty());
    }

    #[test]
    fn textes_du_panneau() {
        let (head, observed, dormant) = subtitle_parts(3);
        assert_eq!(head, "3 label(s) — ");
        assert!(observed.contains("observé dans la capture courante"));
        assert!(dormant.contains("supprimer une ligne désétiquette l'équipement"));
        assert_eq!(
            empty_text(0),
            "Aucun label. Ajoute une ligne ou importe un fichier."
        );
        assert_eq!(empty_text(4), "Aucun label ne correspond à la recherche.");
        assert_eq!(status_text(true), "Observé dans la capture courante");
        assert_eq!(status_text(false), "Dormant (aucun endpoint correspondant)");
        assert_eq!(arbitrate_button(2), "Arbitrer 2 conflit(s)");
        assert_eq!(
            delete_question("aa:bb:cc:dd:ee:ff", "", "Box"),
            "Supprimer le label « Box » (aa:bb:cc:dd:ee:ff / -) ?\nL'équipement sera désétiqueté."
        );
    }

    #[test]
    fn saisie_valide_dans_les_formes_acceptees() {
        for mac in [
            "aa:bb:cc:dd:ee:ff",
            "AA-BB-CC-DD-EE-FF",
            "aabb.ccdd.eeff",
            "aabbccddeeff",
            "",
        ] {
            let errors = validate_entry(mac, "10.0.0.1", "poste", true);
            assert!(errors.is_empty(), "{mac} : {errors:?}");
        }
        assert!(validate_entry("aa:bb:cc:dd:ee:ff", "", "poste", true).is_empty());
        assert!(validate_entry("", "fe80::1", "", false).is_empty());
    }

    #[test]
    fn erreurs_placees_sous_le_bon_champ() {
        let errors = validate_entry("aa:bb", "999.1.1.1", "", true);
        assert_eq!(errors.mac.as_deref(), Some(INVALID_MAC));
        assert_eq!(errors.ip.as_deref(), Some(INVALID_IP));
        assert_eq!(errors.label.as_deref(), Some(LABEL_REQUIRED));
        let errors = validate_entry("", "", "poste", true);
        assert_eq!(errors.ip.as_deref(), Some(EMPTY_KEY));
        assert_eq!(errors.mac, None);
        // Édition : le label peut être vidé.
        assert!(validate_entry("aa:bb:cc:dd:ee:ff", "", "  ", false).is_empty());
        assert_eq!(
            validate_entry("aa:bb:cc:dd:ee:ff", "", "  ", true)
                .label
                .as_deref(),
            Some(LABEL_REQUIRED)
        );
    }

    #[test]
    fn labels_de_la_matrice() {
        let rows = vec![
            (
                "aa:bb:cc:dd:ee:01".to_owned(),
                "10.0.0.1".to_owned(),
                "Automate".to_owned(),
            ),
            (
                "aa:bb:cc:dd:ee:02".to_owned(),
                String::new(),
                "IHM".to_owned(),
            ),
        ];
        assert_eq!(filter_triplets(&rows, ""), [0, 1]);
        assert_eq!(filter_triplets(&rows, " ihm"), [1]);
        assert_eq!(filter_triplets(&rows, "10.0"), [0]);
        assert_eq!(matrix_labels_title(2), "Labels appliqués à la matrice (2)");
    }
}
