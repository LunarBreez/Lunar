//! Import d'un fichier de labels CSV : classification des erreurs
//! bloquantes pour le dialogue des conflits, textes de ce dialogue, ligne de
//! journal du bilan et aides du panneau d'import.

use lunar_engine::commandes::import::labels::conflicts::LabelImportReport;
use lunar_engine::errors::CaptureStateError;
use lunar_engine::errors::label::{LabelConflict, LabelError, LabelInvalidField, LabelInvalidRow};

/// Erreur bloquante d'un import de labels, telle que le dialogue des
/// conflits l'affiche (`LabelFileIssues`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LabelFileIssues {
    /// MAC ou IP mal formées.
    InvalidMacIpFormat {
        /// MAC fautives : (ligne, valeur, ligne brute).
        invalid_mac: Vec<LabelInvalidField>,
        /// IP fautives : (ligne, valeur, ligne brute).
        invalid_ip: Vec<LabelInvalidField>,
    },
    /// Lignes en conflit (même IP, MAC ou label différents).
    LabelLinesConflicts {
        /// Même IP, MAC différente.
        same_ip_diff_mac: Vec<LabelConflict>,
        /// Même IP, label différent.
        same_ip_diff_label: Vec<LabelConflict>,
    },
    /// Lignes inexploitables.
    InvalidRowsFormat {
        /// Lignes rejetées : (ligne, contenu).
        invalid_lines: Vec<LabelInvalidRow>,
    },
}

/// Erreur d'import propre aux fichiers de labels, à montrer dans le
/// dialogue des conflits ; `None` pour toute autre erreur, à montrer par le
/// message générique. Une édition refusée ne
/// vient pas d'un fichier : elle n'a pas de dialogue dédié.
pub fn classify_label_import_error(err: &CaptureStateError) -> Option<LabelFileIssues> {
    let CaptureStateError::Label(label) = err else {
        return None;
    };
    match label {
        LabelError::InvalidMacIpFormat {
            invalid_mac,
            invalid_ip,
        } => Some(LabelFileIssues::InvalidMacIpFormat {
            invalid_mac: invalid_mac.clone(),
            invalid_ip: invalid_ip.clone(),
        }),
        LabelError::LabelLinesConflicts {
            same_ip_diff_mac,
            same_ip_diff_label,
        } => Some(LabelFileIssues::LabelLinesConflicts {
            same_ip_diff_mac: same_ip_diff_mac.clone(),
            same_ip_diff_label: same_ip_diff_label.clone(),
        }),
        LabelError::InvalidRowsFormat { invalid_lines } => {
            Some(LabelFileIssues::InvalidRowsFormat {
                invalid_lines: invalid_lines.clone(),
            })
        }
        LabelError::EditRejected(_) => None,
    }
}

/// Plafond d'erreurs détaillées par catégorie : au-delà (mauvais fichier,
/// encodage…), la liste complète noie le diagnostic — le compteur « … et N
/// autres » suffit.
pub const MAX_SHOWN: usize = 10;

/// Format attendu, rappelé sous les erreurs de format.
pub const EXPECTED_FORMAT: &str = "Format attendu : mac, ip, label — CSV à séparateur virgule, point-virgule ou tabulation (détecté), ou première feuille d'un classeur .xlsx ; les colonnes suivantes sont ajoutées au label";

/// Aide du format sous le panneau d'import.
pub const FORMAT_HINT: &str = "Format : mac, ip, label — CSV à séparateur virgule, point-virgule ou tabulation (détecté), ou première feuille d'un classeur .xlsx ; les colonnes suivantes sont ajoutées au label ; MAC acceptée en aa:bb:cc:dd:ee:ff, aa-bb-cc-dd-ee-ff, aabb.ccdd.eeff ou aabbccddeeff";

/// Aide des relevés WinAudit sous le panneau d'import.
pub const WINAUDIT_HINT: &str = "Relevés WinAudit : fichiers .csv au format WinAudit, un ou plusieurs à la fois. Label : identifiant SALP du nom de fichier et nom de machine, pour chaque adresse et, en repli, pour la MAC seule. Ces labels s'ajoutent à ceux en place ; un désaccord part à l'arbitrage.";

/// Filtre du dialogue d'ouverture d'un fichier de labels.
pub const LABEL_FILE_FILTER: &str = "Label File";

/// Ligne de journal au lancement d'un import (`import_label_file: chemin`,
/// attendue par le harnais E2E).
pub fn import_start_log(path: &str) -> String {
    format!("import_label_file: {path}")
}

/// Ligne de journal du bilan d'un import réussi.
pub fn import_report_log(report: &LabelImportReport) -> String {
    format!(
        "réponse invoke: {} label(s), {} conflit(s) résolu(s), {} ligne(s) sans label ignorée(s)",
        report.applied,
        report.conflicts.len(),
        report.skipped_without_label
    )
}

/// Note de la table importée : lignes sans label écartées, vide
/// s'il n'y en a pas.
pub fn skipped_note(skipped_without_label: usize) -> Option<String> {
    (skipped_without_label > 0)
        .then(|| format!(" · {skipped_without_label} ligne(s) sans label ignorée(s)"))
}

/// Libellé du bouton d'arbitrage du panneau d'import.
pub fn arbitrate_button(pending: usize) -> String {
    format!("Arbitrer les conflits ({pending})")
}

/// Section du dialogue des conflits : titre et entrées, chaque entrée sur
/// une ou plusieurs lignes (la première est l'en-tête de l'entrée).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueSection {
    /// Titre (« MAC invalides »…).
    pub title: &'static str,
    /// Entrées affichées, plafonnées à [`MAX_SHOWN`] pour les erreurs de
    /// format.
    pub entries: Vec<Vec<String>>,
    /// Ligne « … et N autre(s) » quand la liste est tronquée.
    pub truncated: Option<String>,
}

/// Contenu du dialogue des conflits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueReport {
    /// Titre principal.
    pub title: &'static str,
    /// Compte des erreurs (`Total : N erreur(s)…`).
    pub total: String,
    /// Sections non vides, dans l'ordre du dialogue.
    pub sections: Vec<IssueSection>,
    /// Le rappel du format attendu s'affiche.
    pub show_expected_format: bool,
}

/// `s` au pluriel.
fn plural(count: usize) -> &'static str {
    if count > 1 { "s" } else { "" }
}

/// `-` pour un champ vide.
fn dash(value: &str) -> &str {
    if value.is_empty() { "-" } else { value }
}

/// Ligne « … et N autre(s) » d'une liste plafonnée ; `hint` complète la
/// phrase pour les erreurs de MAC et d'IP.
fn truncation(total: usize, hint: bool) -> Option<String> {
    let more = total.checked_sub(MAX_SHOWN).filter(|more| *more > 0)?;
    let suffix = if hint {
        " (corrigez les premières : souvent la même cause)"
    } else {
        ""
    };
    Some(format!("… et {more} autre{}{suffix}", plural(more)))
}

/// Entrées d'un conflit entre lignes (`'ip'(IP) — lignes A / B:` puis le
/// détail de chaque ligne).
fn conflict_entries(conflicts: &[LabelConflict], field: &str) -> Vec<Vec<String>> {
    conflicts
        .iter()
        .map(|(line_a, line_b, ip, value_a, value_b, row_a, row_b)| {
            vec![
                format!("'{ip}'(IP) — lignes {line_a} / {line_b}:"),
                format!("ligne {line_a} {field}: '{}'", dash(value_a)),
                format!("ligne {line_a} complète : '{row_a}'"),
                format!("ligne {line_b} {field}: '{}'", dash(value_b)),
                format!("ligne {line_b} complète : '{row_b}'"),
            ]
        })
        .collect()
}

/// Entrées de champs invalides, plafonnées.
fn field_entries(fields: &[LabelInvalidField], field: &str) -> Vec<Vec<String>> {
    fields
        .iter()
        .take(MAX_SHOWN)
        .map(|(line, value, row)| {
            vec![
                format!("ligne {line} — {field}: '{value}'"),
                format!("ligne complète : '{row}'"),
            ]
        })
        .collect()
}

/// Contenu du dialogue des conflits. `resolved` :
/// conflits déjà résolus « premier gardé », affichés pour information.
pub fn issue_report(issues: &LabelFileIssues, resolved: bool) -> IssueReport {
    let mut sections = Vec::new();
    let (title, total, show_expected_format) = match issues {
        LabelFileIssues::LabelLinesConflicts {
            same_ip_diff_mac,
            same_ip_diff_label,
        } => {
            for (list, title, field) in [
                (same_ip_diff_mac, "Conflits IP -> MAC", "MAC"),
                (same_ip_diff_label, "Conflits IP -> Label", "Label"),
            ] {
                if !list.is_empty() {
                    sections.push(IssueSection {
                        title,
                        entries: conflict_entries(list, field),
                        truncated: None,
                    });
                }
            }
            let title = if resolved {
                "Conflits de labels résolus"
            } else {
                "Conflits détectés"
            };
            (
                title,
                same_ip_diff_mac.len() + same_ip_diff_label.len(),
                false,
            )
        }
        LabelFileIssues::InvalidMacIpFormat {
            invalid_mac,
            invalid_ip,
        } => {
            for (list, title, field) in [
                (invalid_mac, "MAC invalides", "MAC"),
                (invalid_ip, "IP invalides", "IP"),
            ] {
                if !list.is_empty() {
                    sections.push(IssueSection {
                        title,
                        entries: field_entries(list, field),
                        truncated: truncation(list.len(), true),
                    });
                }
            }
            (
                "MAC/IP invalide(s) détectée(s)",
                invalid_mac.len() + invalid_ip.len(),
                true,
            )
        }
        LabelFileIssues::InvalidRowsFormat { invalid_lines } => {
            sections.push(IssueSection {
                title: "Lignes concernées :",
                entries: invalid_lines
                    .iter()
                    .take(MAX_SHOWN)
                    .map(|(line, value)| vec![format!("ligne {line} : '{value}'")])
                    .collect(),
                truncated: truncation(invalid_lines.len(), false),
            });
            ("Format de fichier invalide", invalid_lines.len(), true)
        }
    };
    let total = if resolved {
        format!(
            "{total} doublon{s} de label résolu{s} automatiquement (premier gardé). À arbitrer si besoin.",
            s = plural(total)
        )
    } else {
        format!(
            "Total : {total} erreur{s} détectée{s} dans le CSV",
            s = plural(total)
        )
    };
    IssueReport {
        title,
        total,
        sections,
        show_expected_format,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn label_err(err: LabelError) -> CaptureStateError {
        CaptureStateError::Label(err)
    }

    // --- Classification et bilan ---

    #[test]
    fn erreur_non_label_rend_none() {
        let io = CaptureStateError::Io(std::io::Error::other("disque plein"));
        assert_eq!(classify_label_import_error(&io), None);
    }

    #[test]
    fn formats_mac_ip_invalides() {
        let invalid_mac = vec![(3, "aa:bb".to_owned(), "aa:bb,1.2.3.4,pc".to_owned())];
        let invalid_ip = vec![(
            5,
            "999.1.1.1".to_owned(),
            "aa:bb:cc:dd:ee:ff,999.1.1.1,pc".to_owned(),
        )];
        let err = label_err(LabelError::InvalidMacIpFormat {
            invalid_mac: invalid_mac.clone(),
            invalid_ip: invalid_ip.clone(),
        });
        assert_eq!(
            classify_label_import_error(&err),
            Some(LabelFileIssues::InvalidMacIpFormat {
                invalid_mac,
                invalid_ip
            })
        );
    }

    #[test]
    fn conflits_entre_lignes() {
        let conflict = vec![(
            1,
            2,
            "1.2.3.4".to_owned(),
            "pc-a".to_owned(),
            "pc-b".to_owned(),
            "ligne1".to_owned(),
            "ligne2".to_owned(),
        )];
        let err = label_err(LabelError::LabelLinesConflicts {
            same_ip_diff_mac: Vec::new(),
            same_ip_diff_label: conflict.clone(),
        });
        assert_eq!(
            classify_label_import_error(&err),
            Some(LabelFileIssues::LabelLinesConflicts {
                same_ip_diff_mac: Vec::new(),
                same_ip_diff_label: conflict,
            })
        );
    }

    #[test]
    fn lignes_malformees() {
        let invalid_lines = vec![(4, "une,seule".to_owned())];
        let err = label_err(LabelError::InvalidRowsFormat {
            invalid_lines: invalid_lines.clone(),
        });
        assert_eq!(
            classify_label_import_error(&err),
            Some(LabelFileIssues::InvalidRowsFormat { invalid_lines })
        );
    }

    #[test]
    fn edition_refusee_sans_dialogue_dedie() {
        // Équivalent du « sous-type de label inconnu » : le contrat typé ne
        // laisse passer qu'EditRejected, qui n'est pas une erreur de fichier.
        let err = label_err(LabelError::EditRejected("clé prise".into()));
        assert_eq!(classify_label_import_error(&err), None);
    }

    // --- Dialogue des conflits ---

    #[test]
    fn mac_et_ip_invalides_plafonnees_avec_le_compteur() {
        let invalid_mac: Vec<LabelInvalidField> = (1..=12)
            .map(|n| (n, format!("mac{n}"), format!("mac{n},1.1.1.1,pc")))
            .collect();
        let invalid_ip = vec![(20, "999.1.1.1".to_owned(), "row".to_owned())];
        let report = issue_report(
            &LabelFileIssues::InvalidMacIpFormat {
                invalid_mac,
                invalid_ip,
            },
            false,
        );
        assert_eq!(report.title, "MAC/IP invalide(s) détectée(s)");
        assert_eq!(report.total, "Total : 13 erreurs détectées dans le CSV");
        assert!(report.show_expected_format);
        assert_eq!(report.sections.len(), 2);
        let macs = &report.sections[0];
        assert_eq!(macs.title, "MAC invalides");
        assert_eq!(macs.entries.len(), MAX_SHOWN);
        assert_eq!(
            macs.entries[0],
            [
                "ligne 1 — MAC: 'mac1'",
                "ligne complète : 'mac1,1.1.1.1,pc'"
            ]
        );
        assert_eq!(
            macs.truncated.as_deref(),
            Some("… et 2 autres (corrigez les premières : souvent la même cause)")
        );
        let ips = &report.sections[1];
        assert_eq!(ips.title, "IP invalides");
        assert_eq!(ips.entries[0][0], "ligne 20 — IP: '999.1.1.1'");
        assert_eq!(ips.truncated, None);
    }

    #[test]
    fn lignes_mal_formees_au_singulier() {
        let invalid_lines: Vec<LabelInvalidRow> = (1..=11).map(|n| (n, "x".to_owned())).collect();
        let report = issue_report(&LabelFileIssues::InvalidRowsFormat { invalid_lines }, false);
        assert_eq!(report.title, "Format de fichier invalide");
        assert_eq!(report.sections[0].title, "Lignes concernées :");
        assert_eq!(report.sections[0].entries[0], ["ligne 1 : 'x'"]);
        assert_eq!(
            report.sections[0].truncated.as_deref(),
            Some("… et 1 autre")
        );
        let one = issue_report(
            &LabelFileIssues::InvalidRowsFormat {
                invalid_lines: vec![(1, "x".into())],
            },
            false,
        );
        assert_eq!(one.total, "Total : 1 erreur détectée dans le CSV");
    }

    #[test]
    fn conflits_entre_lignes_detailles_et_resolus() {
        let issues = LabelFileIssues::LabelLinesConflicts {
            same_ip_diff_mac: vec![(
                2,
                5,
                "10.0.0.1".into(),
                String::new(),
                "aa:bb:cc:dd:ee:ff".into(),
                "row2".into(),
                "row5".into(),
            )],
            same_ip_diff_label: Vec::new(),
        };
        let report = issue_report(&issues, false);
        assert_eq!(report.title, "Conflits détectés");
        assert!(!report.show_expected_format);
        assert_eq!(report.sections.len(), 1);
        assert_eq!(
            report.sections[0].entries[0],
            [
                "'10.0.0.1'(IP) — lignes 2 / 5:",
                "ligne 2 MAC: '-'",
                "ligne 2 complète : 'row2'",
                "ligne 5 MAC: 'aa:bb:cc:dd:ee:ff'",
                "ligne 5 complète : 'row5'",
            ]
        );
        let resolved = issue_report(&issues, true);
        assert_eq!(resolved.title, "Conflits de labels résolus");
        assert_eq!(
            resolved.total,
            "1 doublon de label résolu automatiquement (premier gardé). À arbitrer si besoin."
        );
    }

    // --- Panneau d'import ---

    #[test]
    fn journal_et_notes_de_l_import() {
        let report = LabelImportReport {
            applied: 4,
            conflicts: Vec::new(),
            skipped_without_label: 2,
        };
        assert_eq!(
            import_report_log(&report),
            "réponse invoke: 4 label(s), 0 conflit(s) résolu(s), 2 ligne(s) sans label ignorée(s)"
        );
        assert_eq!(
            import_start_log("/tmp/labels.csv"),
            "import_label_file: /tmp/labels.csv"
        );
        assert_eq!(skipped_note(0), None);
        assert_eq!(
            skipped_note(2).as_deref(),
            Some(" · 2 ligne(s) sans label ignorée(s)")
        );
        assert_eq!(arbitrate_button(3), "Arbitrer les conflits (3)");
    }
}
