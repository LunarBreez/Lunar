//! Import de relevés WinAudit : extension des relevés, réglage par défaut et
//! bilan affiché à l'opérateur, une phrase par constat. L'appel au moteur est
//! `Lunar::import_winaudit_surveys(paths, exclude_virtual, on_event)`.

use lunar_engine::commandes::import::labels::winaudit::WinAuditImportReport;

/// Extension des relevés WinAudit.
pub const WINAUDIT_SURVEY_EXTENSIONS: &[&str] = &["csv"];

/// Nom du filtre du dialogue de fichiers.
pub const WINAUDIT_FILTER_NAME: &str = "Relevé WinAudit";

/// Adaptateurs virtuels écartés par défaut, comme dans `winaudit2labels`.
pub const DEFAULT_EXCLUDE_VIRTUAL: bool = true;

/// Libellé de la case qui écarte les adaptateurs virtuels.
pub const EXCLUDE_VIRTUAL_LABEL: &str =
    "Écarter les adaptateurs virtuels des relevés (MAC localement administrée)";

/// Ligne de journal au lancement d'un import (`import_winaudit_surveys: N
/// relevé(s)`).
pub fn import_start_log(count: usize) -> String {
    format!("import_winaudit_surveys: {count} relevé(s)")
}

/// Nom de fichier d'un chemin, séparateurs `/` et `\` (chemins Windows des
/// relevés) ; le chemin entier s'il finit par un séparateur.
pub fn file_name(path: &str) -> &str {
    match path.rsplit(['/', '\\']).next() {
        Some(name) if !name.is_empty() => name,
        _ => path,
    }
}

/// Noms de fichiers séparés par des virgules.
fn file_names(paths: &[String]) -> String {
    paths
        .iter()
        .map(|path| file_name(path))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Bilan d'un import, une phrase par constat : ce qui a été lu, ce qui entre
/// dans le référentiel, puis tout ce qui a été écarté, pour que rien ne
/// disparaisse sans que l'utilisateur le voie.
pub fn summarize_winaudit_import(report: &WinAuditImportReport) -> Vec<String> {
    let mut lines = vec![
        format!(
            "{} relevé(s) lu(s), {} machine(s), {} adaptateur(s) : {} label(s) tiré(s) des relevés (une ligne par adresse, plus une ligne de repli à IP vide par interface).",
            report.files_read, report.machines, report.adapters, report.labels_found
        ),
        format!(
            "{} ajouté(s) au référentiel, {} déjà présent(s) à l'identique.",
            report.added, report.unchanged
        ),
    ];
    if report.new_conflicts > 0 {
        lines.push(format!(
            "{} désaccord(s) à arbitrer : le label en place reste appliqué jusqu'à ton choix.",
            report.new_conflicts
        ));
    }
    // Juste après les comptes : c'est ce qui explique un import sans label.
    if !report.files_without_network_section.is_empty() {
        lines.push(format!(
            "{} relevé(s) sans rubrique 2600 (collecte antérieure à l'inventaire réseau), aucun label possible : {}.",
            report.files_without_network_section.len(),
            file_names(&report.files_without_network_section)
        ));
    }
    if report.skipped_virtual > 0 {
        lines.push(format!(
            "{} adaptateur(s) virtuel(s) écarté(s) (MAC localement administrée : ponts Docker, veth, virbr) — décocher « Écarter les adaptateurs virtuels » pour les garder.",
            report.skipped_virtual
        ));
    } else if report.virtual_adapters > 0 {
        lines.push(format!(
            "{} adaptateur(s) virtuel(s) gardé(s) (MAC localement administrée : ponts Docker, veth, virbr) — case « Écarter les adaptateurs virtuels » pour les retirer.",
            report.virtual_adapters
        ));
    }
    if report.skipped_loopback > 0 {
        lines.push(format!(
            "{} bouclage(s) écarté(s) : leur MAC nulle est la même sur toutes les machines.",
            report.skipped_loopback
        ));
    }
    if report.skipped_without_mac > 0 {
        lines.push(format!(
            "{} adaptateur(s) sans MAC ignoré(s).",
            report.skipped_without_mac
        ));
    }
    if report.skipped_group_mac > 0 {
        lines.push(format!(
            "{} adaptateur(s) à MAC de groupe écarté(s) (multicast, diffusion).",
            report.skipped_group_mac
        ));
    }
    if report.skipped_addresses > 0 {
        lines.push(format!(
            "{} adresse(s) écartée(s) : boucle locale, non spécifiée, multicast, diffusion ou illisible.",
            report.skipped_addresses
        ));
    }
    if !report.files_without_identifier.is_empty() {
        lines.push(format!(
            "{} relevé(s) sans identifiant SALP dans le nom de fichier, label réduit au nom de machine : {}.",
            report.files_without_identifier.len(),
            file_names(&report.files_without_identifier)
        ));
    }
    if report.malformed_rows > 0 {
        lines.push(format!(
            "{} ligne(s) illisible(s) ignorée(s).",
            report.malformed_rows
        ));
    }
    if !report.invalid.is_empty() {
        lines.push(format!(
            "Adresse(s) refusée(s) : {}.",
            report.invalid.join(", ")
        ));
    }
    for (path, reason) in &report.files_rejected {
        lines.push(format!("Relevé écarté — {} : {reason}", file_name(path)));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bilan de référence des tests.
    fn report() -> WinAuditImportReport {
        WinAuditImportReport {
            files_read: 2,
            files_rejected: Vec::new(),
            machines: 2,
            adapters: 5,
            virtual_adapters: 0,
            skipped_virtual: 0,
            skipped_loopback: 0,
            skipped_without_mac: 0,
            skipped_group_mac: 0,
            skipped_addresses: 0,
            files_without_identifier: Vec::new(),
            files_without_network_section: Vec::new(),
            malformed_rows: 0,
            labels_found: 4,
            added: 3,
            unchanged: 1,
            invalid: Vec::new(),
            applied: 3,
            new_conflicts: 0,
            conflicts: Vec::new(),
        }
    }

    #[test]
    fn le_bilan_dit_ce_qui_est_entre_et_ce_qui_attend() {
        let report = WinAuditImportReport {
            new_conflicts: 1,
            ..report()
        };
        assert_eq!(
            summarize_winaudit_import(&report),
            [
                "2 relevé(s) lu(s), 2 machine(s), 5 adaptateur(s) : 4 label(s) tiré(s) des relevés (une ligne par adresse, plus une ligne de repli à IP vide par interface).",
                "3 ajouté(s) au référentiel, 1 déjà présent(s) à l'identique.",
                "1 désaccord(s) à arbitrer : le label en place reste appliqué jusqu'à ton choix.",
            ]
        );
    }

    #[test]
    fn rien_de_ce_qui_est_ecarte_ne_disparait_sans_etre_dit() {
        let report = WinAuditImportReport {
            virtual_adapters: 17,
            skipped_loopback: 1,
            skipped_without_mac: 2,
            skipped_group_mac: 1,
            skipped_addresses: 3,
            files_without_identifier: vec!["/releves/poste-libre.csv".into()],
            malformed_rows: 1,
            invalid: vec!["38:f3:ab:cf:ba:01 / fe80::1%eth0".into()],
            files_rejected: vec![(
                "C:\\releves\\vide.csv".into(),
                "aucune rubrique WinAudit reconnue (300 ou 2600)".into(),
            )],
            ..report()
        };
        let text = summarize_winaudit_import(&report).join("\n");
        // Adaptateurs virtuels gardés : le bilan dit comment les retirer.
        assert!(
            text.contains("17 adaptateur(s) virtuel(s) gardé(s)"),
            "{text}"
        );
        assert!(text.contains("Écarter les adaptateurs virtuels"));
        assert!(text.contains("1 bouclage(s) écarté(s)"));
        assert!(text.contains("2 adaptateur(s) sans MAC"));
        assert!(text.contains("1 ligne(s) illisible(s)"));
        assert!(text.contains("1 adaptateur(s) à MAC de groupe écarté(s)"));
        assert!(text.contains("3 adresse(s) écartée(s)"));
        // Relevé hors convention de nommage : label sans préfixe SALP, et on le dit.
        assert!(text.contains(
            "1 relevé(s) sans identifiant SALP dans le nom de fichier, label réduit au nom de machine : poste-libre.csv."
        ));
        assert!(text.contains("Adresse(s) refusée(s) : 38:f3:ab:cf:ba:01 / fe80::1%eth0."));
        // Chemin Windows réduit au nom du fichier.
        assert!(text.contains("Relevé écarté — vide.csv : aucune rubrique WinAudit reconnue"));
    }

    #[test]
    fn un_lot_sans_rubrique_2600_dit_pourquoi_il_ne_donne_aucun_label() {
        let report = WinAuditImportReport {
            adapters: 0,
            labels_found: 0,
            added: 0,
            unchanged: 0,
            applied: 0,
            files_without_network_section: vec![
                "/releves/20260901_NP_CONF_TST-PC-000002.csv".into(),
                "C:\\releves\\20260901_NP_CONF_TST-PC-000003.csv".into(),
            ],
            ..report()
        };
        assert_eq!(
            summarize_winaudit_import(&report),
            [
                "2 relevé(s) lu(s), 2 machine(s), 0 adaptateur(s) : 0 label(s) tiré(s) des relevés (une ligne par adresse, plus une ligne de repli à IP vide par interface).",
                "0 ajouté(s) au référentiel, 0 déjà présent(s) à l'identique.",
                "2 relevé(s) sans rubrique 2600 (collecte antérieure à l'inventaire réseau), aucun label possible : 20260901_NP_CONF_TST-PC-000002.csv, 20260901_NP_CONF_TST-PC-000003.csv.",
            ]
        );
    }

    #[test]
    fn adaptateurs_virtuels_ecartes_et_le_bilan_dit_comment_les_garder() {
        let report = WinAuditImportReport {
            virtual_adapters: 3,
            skipped_virtual: 3,
            ..report()
        };
        let text = summarize_winaudit_import(&report).join("\n");
        assert!(text.contains("3 adaptateur(s) virtuel(s) écarté(s)"));
        assert!(text.contains("décocher « Écarter les adaptateurs virtuels » pour les garder"));
        assert!(!text.contains("gardé(s)"));
    }

    #[test]
    fn adaptateurs_virtuels_ecartes_par_defaut_comme_winaudit2labels() {
        // Mêmes réglages par défaut que `winaudit2labels`.
        const { assert!(DEFAULT_EXCLUDE_VIRTUAL) };
        assert_eq!(WINAUDIT_SURVEY_EXTENSIONS, ["csv"]);
        assert_eq!(WINAUDIT_FILTER_NAME, "Relevé WinAudit");
    }

    // --- Compléments ---

    #[test]
    fn nom_de_fichier_des_chemins_unix_et_windows() {
        assert_eq!(file_name("/a/b/poste.csv"), "poste.csv");
        assert_eq!(file_name("C:\\releves\\poste.csv"), "poste.csv");
        assert_eq!(file_name("poste.csv"), "poste.csv");
        assert_eq!(file_name("/a/b/"), "/a/b/");
        assert_eq!(import_start_log(2), "import_winaudit_surveys: 2 relevé(s)");
    }
}
