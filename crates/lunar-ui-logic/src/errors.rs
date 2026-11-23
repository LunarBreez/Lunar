//! Traduction des erreurs du moteur en messages français lisibles : titre et
//! texte du dialogue d'erreur, ligne de journal.
//!
//! L'erreur arrive typée ([`CaptureStateError`]) : le `match` est exhaustif
//! et une variante ajoutée au moteur casse la compilation. Reste un filet :
//! [`unexpected_error_message`], pour une erreur qui ne vient pas du moteur
//! (tâche interrompue, dialogue de fichiers en échec…).
//!
//! Les messages s'appuient sur l'affichage (`Display`) des erreurs
//! imbriquées.

use lunar_engine::errors::CaptureStateError;
use lunar_engine::errors::capture_error::CaptureError;
use lunar_engine::errors::export::ExportError;
use lunar_engine::errors::import::PcapImportError;
use lunar_engine::errors::label::{LabelConflict, LabelError, LabelInvalidField, LabelInvalidRow};

/// Message prêt à afficher : titre du dialogue et texte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorMessage {
    /// Titre du dialogue (« Erreur Capture (import) »).
    pub title: String,
    /// Texte, sur plusieurs lignes au besoin.
    pub message: String,
}

impl ErrorMessage {
    /// Ligne écrite dans le journal, au niveau erreur
    /// (`Erreur Capture (kind) : message`).
    pub fn log_line(&self) -> String {
        format!("{} : {}", self.title, self.message)
    }
}

/// Étiquette `kind` de l'erreur, telle que la sérialisait le contrat.
pub fn error_kind(err: &CaptureStateError) -> &'static str {
    match err {
        CaptureStateError::Io(_) => "io",
        CaptureStateError::PoisonError(_) => "poisonError",
        CaptureStateError::InvalidTransition { .. } => "invalidTransition",
        CaptureStateError::Capture(_) => "capture",
        CaptureStateError::Export(_) => "export",
        CaptureStateError::Import(_) => "import",
        CaptureStateError::Label(_) => "label",
    }
}

/// Titre et texte du dialogue d'une erreur du moteur
///.
pub fn capture_error_message(err: &CaptureStateError) -> ErrorMessage {
    let message = match err {
        CaptureStateError::Io(io) => format!("Erreur IO : {io}"),
        CaptureStateError::PoisonError(detail) => format!("Erreur verrou : {detail}"),
        CaptureStateError::InvalidTransition { .. } => format!("Opération refusée : {err}"),
        CaptureStateError::Capture(capture) => capture_error_text(capture),
        CaptureStateError::Export(export) => handle_export_error(export),
        CaptureStateError::Import(import) => handle_import_error(import),
        CaptureStateError::Label(label) => handle_label_error(label),
    };
    ErrorMessage {
        title: format!("Erreur Capture ({})", error_kind(err)),
        message,
    }
}

/// Dernier filet : erreur hors du contrat du moteur, montrée telle quelle.
pub fn unexpected_error_message(detail: &str) -> ErrorMessage {
    ErrorMessage {
        title: "Erreur Capture (inattendue)".to_owned(),
        message: format!("Erreur inattendue : {detail}"),
    }
}

/// Texte d'une erreur du domaine capture (`kind: "capture"`).
pub fn capture_error_text(err: &CaptureError) -> String {
    match err {
        CaptureError::InvalidConfig(message) => format!("Configuration invalide : {message}"),
        CaptureError::ConfigPersistence(message) => {
            format!("Erreur persistance configuration : {message}")
        }
        CaptureError::InterfaceNotFound(message) => format!("Interface non trouvée : {message}"),
        CaptureError::DeviceListError(source) => format!(
            "Erreur récupération device : {source}.\nEssayez : sudo setcap cap_net_raw=eip nom_du_binaire."
        ),
        CaptureError::NpcapMissing(message) => message.clone(),
        CaptureError::CaptureInitError(source) => {
            format!("Erreur initialisation capture : {source}")
        }
        CaptureError::ChannelSendError(source) => format!("Erreur envoi canal capture : {source}"),
        CaptureError::EventSendError(source) => {
            format!("Erreur envoi evenement capture : {source}")
        }
        CaptureError::UnsupportedLinkType(label) => format!(
            "Type de liaison non supporté par cette version : {label}.\nLa capture n'a pas démarré."
        ),
        CaptureError::MixedLinkType(message) => format!(
            "{message}\nExportez ou réinitialisez le relevé en cours avant de capturer sur cette interface."
        ),
    }
}

/// Texte d'une erreur d'export.
pub fn handle_export_error(err: &ExportError) -> String {
    match err {
        ExportError::EmptyPath => "Aucun chemin de fichier fourni pour l'export.".to_owned(),
        ExportError::Io(source) => format!("Erreur d'écriture du fichier exporté : {source}"),
        ExportError::Csv(source) => format!("Erreur d'écriture CSV : {source}"),
        ExportError::Zip(source) => format!("Erreur d'écriture de l'archive ZIP : {source}"),
        ExportError::PoisonError(detail) => format!("Erreur verrou pendant l'export : {detail}"),
        ExportError::LogNotFound => "Le dossier de logs est introuvable.".to_owned(),
        ExportError::NotHtml => {
            "La synthèse doit être enregistrée dans un fichier .html.".to_owned()
        }
        ExportError::CsvAlreadyExists(path) => {
            format!("Le fichier {path} existe déjà : choisissez un autre nom pour la synthèse.")
        }
        ExportError::MissingFilter => {
            "Aucun filtre actif : exportez la matrice complète (Fichier › Exporter la matrice)."
                .to_owned()
        }
    }
}

/// Message de Npcap absent quand `err`, renvoyée telle quelle par
/// `get_devices_list`, est `NpcapMissing`.
pub fn npcap_missing_message(err: &CaptureError) -> Option<&str> {
    match err {
        CaptureError::NpcapMissing(message) => Some(message),
        _ => None,
    }
}

/// Texte d'une erreur d'import.
pub fn handle_import_error(err: &PcapImportError) -> String {
    match err {
        PcapImportError::MissingInput(operation) => {
            format!("Aucun fichier sélectionné pour {operation}. Le relevé courant est inchangé.")
        }
        PcapImportError::NpcapMissing(message) => {
            format!("{message}\nLe relevé courant est inchangé.")
        }
        PcapImportError::OpenFileError(file, message) => {
            format!("Impossible d'ouvrir le fichier {file} : {message}")
        }
        PcapImportError::ReadPacketError(file, message) => format!(
            "Erreur de lecture dans {file} : {message}.\nL'import a été annulé, la matrice courante est inchangée."
        ),
        PcapImportError::UnsupportedLinkType(file, label) => format!(
            "Type de liaison non supporté dans {file} : {label}.\nL'import a été annulé, la matrice courante est inchangée."
        ),
        PcapImportError::Cancelled(operation) => {
            format!("Import annulé pendant {operation}.\nLe relevé courant est inchangé.")
        }
    }
}

/// Vrai si l'erreur est l'annulation d'import demandée par l'opérateur :
/// une issue normale à notifier comme telle, pas un défaut
///.
pub fn is_import_cancellation(err: &CaptureStateError) -> bool {
    matches!(
        err,
        CaptureStateError::Import(PcapImportError::Cancelled(_))
    )
}

/// Liste plafonnée : au-delà de `max` entrées, le détail noie le diagnostic
///.
pub fn cap_list<T>(items: &[T], format: impl Fn(&T) -> String, max: usize) -> String {
    let shown: Vec<String> = items.iter().take(max).map(format).collect();
    let shown = shown.join("\n");
    let rest = items.len().saturating_sub(max);
    if rest == 0 {
        return shown;
    }
    let plural = if rest > 1 { "s" } else { "" };
    format!("{shown}\n… et {rest} autre{plural}")
}

/// Plafond par défaut de [`cap_list`].
pub const CAP_LIST_MAX: usize = 8;

/// `ligne N: valeur | ligne brute`.
fn format_invalid_field((line, value, row): &LabelInvalidField) -> String {
    format!("ligne {line}: {value} | {row}")
}

/// `lignes A/B - ip : mac de référence <-> mac`.
fn format_mac_conflict((line_a, line_b, ip, ref_mac, mac, ..): &LabelConflict) -> String {
    format!("lignes {line_a}/{line_b} - {ip} : {ref_mac} <-> {mac}")
}

/// `lignes A/B - ip : label de référence <-> label`.
fn format_label_conflict((line_a, line_b, ip, ref_label, label, ..): &LabelConflict) -> String {
    format!("lignes {line_a}/{line_b} - {ip} : {ref_label} <-> {label}")
}

/// `ligne N: valeur`.
fn format_invalid_row((line, value): &LabelInvalidRow) -> String {
    format!("ligne {line}: {value}")
}

/// Texte d'une erreur de labels.
pub fn handle_label_error(err: &LabelError) -> String {
    match err {
        LabelError::InvalidMacIpFormat {
            invalid_mac,
            invalid_ip,
        } => {
            let mut parts = Vec::new();
            if !invalid_mac.is_empty() {
                parts.push(format!(
                    "MAC invalides ({}) :\n{}",
                    invalid_mac.len(),
                    cap_list(invalid_mac, format_invalid_field, CAP_LIST_MAX)
                ));
            }
            if !invalid_ip.is_empty() {
                parts.push(format!(
                    "IP invalides ({}) :\n{}",
                    invalid_ip.len(),
                    cap_list(invalid_ip, format_invalid_field, CAP_LIST_MAX)
                ));
            }
            format!("Formats invalides.\n{}", parts.join("\n\n"))
        }
        LabelError::LabelLinesConflicts {
            same_ip_diff_mac,
            same_ip_diff_label,
        } => {
            let mut parts = Vec::new();
            if !same_ip_diff_mac.is_empty() {
                parts.push(format!(
                    "même IP, MAC différent ({}) :\n{}",
                    same_ip_diff_mac.len(),
                    cap_list(same_ip_diff_mac, format_mac_conflict, CAP_LIST_MAX)
                ));
            }
            if !same_ip_diff_label.is_empty() {
                parts.push(format!(
                    "même IP, label différent ({}) :\n{}",
                    same_ip_diff_label.len(),
                    cap_list(same_ip_diff_label, format_label_conflict, CAP_LIST_MAX)
                ));
            }
            format!(
                "Conflits dans les lignes de labels.\n{}\n<Importation impossible>",
                parts.join("\n\n")
            )
        }
        LabelError::InvalidRowsFormat { invalid_lines } => format!(
            "Format de fichier invalide. Attendu au moins \"mac, ip, label\" (séparateur virgule, point-virgule ou tabulation ; colonnes suivantes ajoutées au label).\n{}",
            cap_list(invalid_lines, format_invalid_row, CAP_LIST_MAX)
        ),
        LabelError::EditRejected(reason) => format!("Édition refusée : {reason}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lunar_engine::ipc::ChannelError;

    fn io(message: &str) -> std::io::Error {
        std::io::Error::other(message.to_owned())
    }

    // --- handleExportError ---------------------------------------------

    #[test]
    fn export_chemin_vide() {
        assert_eq!(
            handle_export_error(&ExportError::EmptyPath),
            "Aucun chemin de fichier fourni pour l'export."
        );
    }

    #[test]
    fn export_io_et_verrou_incluent_le_message_du_moteur() {
        assert_eq!(
            handle_export_error(&ExportError::Io(io("disque plein"))),
            "Erreur d'écriture du fichier exporté : disque plein"
        );
        assert_eq!(
            handle_export_error(&ExportError::PoisonError("verrou empoisonné".into())),
            "Erreur verrou pendant l'export : verrou empoisonné"
        );
        assert_eq!(
            handle_export_error(&ExportError::CsvAlreadyExists("a.csv".into())),
            "Le fichier a.csv existe déjà : choisissez un autre nom pour la synthèse."
        );
        assert_eq!(
            handle_export_error(&ExportError::NotHtml),
            "La synthèse doit être enregistrée dans un fichier .html."
        );
    }

    #[test]
    fn export_log_not_found() {
        assert_eq!(
            handle_export_error(&ExportError::LogNotFound),
            "Le dossier de logs est introuvable."
        );
    }

    // --- handleImportError ---------------------------------------------

    #[test]
    fn import_entree_vide_explique_que_le_releve_est_preserve() {
        let msg = handle_import_error(&PcapImportError::MissingInput("l'import PCAP".into()));
        assert!(msg.contains("Aucun fichier sélectionné"), "{msg}");
        assert!(msg.contains("relevé courant est inchangé"), "{msg}");
    }

    #[test]
    fn import_open_file_error_inclut_fichier_et_detail() {
        let err =
            PcapImportError::OpenFileError("capture.pcap".into(), "permission refusée".into());
        assert_eq!(
            handle_import_error(&err),
            "Impossible d'ouvrir le fichier capture.pcap : permission refusée"
        );
    }

    #[test]
    fn import_read_packet_error_signale_l_annulation() {
        let err = PcapImportError::ReadPacketError("capture.pcap".into(), "paquet tronqué".into());
        let msg = handle_import_error(&err);
        assert!(msg.contains("Erreur de lecture dans capture.pcap"), "{msg}");
        assert!(msg.contains("matrice courante est inchangée"), "{msg}");
    }

    #[test]
    fn import_unsupported_link_type() {
        let err = PcapImportError::UnsupportedLinkType("capture.pcap".into(), "DLT_RAW".into());
        let msg = handle_import_error(&err);
        assert!(
            msg.contains("Type de liaison non supporté dans capture.pcap : DLT_RAW"),
            "{msg}"
        );
    }

    #[test]
    fn import_cancelled_explique_que_le_releve_est_preserve() {
        let msg = handle_import_error(&PcapImportError::Cancelled("capture.pcap".into()));
        assert!(msg.contains("Import annulé pendant capture.pcap"), "{msg}");
        assert!(msg.contains("relevé courant est inchangé"), "{msg}");
    }

    // --- isImportCancellation ------------------------------------------

    #[test]
    fn annulation_reconnue() {
        let err = CaptureStateError::Import(PcapImportError::Cancelled("capture.pcap".into()));
        assert!(is_import_cancellation(&err));
    }

    #[test]
    fn autres_erreurs_d_import_refusees() {
        let err = CaptureStateError::Import(PcapImportError::MissingInput("l'import PCAP".into()));
        assert!(!is_import_cancellation(&err));
        assert!(!is_import_cancellation(&CaptureStateError::PoisonError(
            "cancelled".into()
        )));
    }

    // --- capList -------------------------------------------------------

    #[test]
    fn cap_list_sous_le_plafond() {
        assert_eq!(cap_list(&[1, 2, 3], |n| format!("#{n}"), 8), "#1\n#2\n#3");
    }

    #[test]
    fn cap_list_exactement_au_plafond() {
        assert_eq!(cap_list(&[1, 2, 3], |n| format!("#{n}"), 3), "#1\n#2\n#3");
    }

    #[test]
    fn cap_list_au_dela_singulier() {
        assert_eq!(
            cap_list(&[1, 2, 3], |n| format!("#{n}"), 2),
            "#1\n#2\n… et 1 autre"
        );
    }

    #[test]
    fn cap_list_au_dela_pluriel() {
        assert_eq!(
            cap_list(&[1, 2, 3, 4, 5], |n| format!("#{n}"), 2),
            "#1\n#2\n… et 3 autres"
        );
    }

    // --- handleLabelerror ----------------------------------------------

    #[test]
    fn labels_mac_et_ip_invalides_listees() {
        let err = LabelError::InvalidMacIpFormat {
            invalid_mac: vec![(2, "pas-une-mac".into(), "pas-une-mac,1.2.3.4,pc".into())],
            invalid_ip: vec![(
                3,
                "999.1.1.1".into(),
                "aa:bb:cc:dd:ee:ff,999.1.1.1,pc".into(),
            )],
        };
        let msg = handle_label_error(&err);
        assert!(msg.contains("MAC invalides (1)"), "{msg}");
        assert!(msg.contains("IP invalides (1)"), "{msg}");
        assert!(msg.contains("ligne 2: pas-une-mac"), "{msg}");
        assert!(msg.contains("ligne 3: 999.1.1.1"), "{msg}");
    }

    #[test]
    fn labels_seule_la_section_non_vide_apparait() {
        let err = LabelError::InvalidMacIpFormat {
            invalid_mac: vec![],
            invalid_ip: vec![(1, "1.2.3.4.5".into(), "raw".into())],
        };
        let msg = handle_label_error(&err);
        assert!(!msg.contains("MAC invalides"), "{msg}");
        assert!(msg.contains("IP invalides (1)"), "{msg}");
    }

    #[test]
    fn labels_conflits_listes() {
        let err = LabelError::LabelLinesConflicts {
            same_ip_diff_mac: vec![],
            same_ip_diff_label: vec![(
                1,
                2,
                "1.2.3.4".into(),
                "pc-a".into(),
                "pc-b".into(),
                "ligne1".into(),
                "ligne2".into(),
            )],
        };
        let msg = handle_label_error(&err);
        assert!(msg.contains("même IP, label différent (1)"), "{msg}");
        assert!(msg.contains("pc-a <-> pc-b"), "{msg}");
        assert!(msg.contains("<Importation impossible>"), "{msg}");
    }

    #[test]
    fn labels_lignes_mal_formees() {
        let err = LabelError::InvalidRowsFormat {
            invalid_lines: vec![(4, "une,seule".into())],
        };
        assert!(handle_label_error(&err).contains("ligne 4: une,seule"));
    }

    #[test]
    fn labels_edition_refusee() {
        assert_eq!(
            handle_label_error(&LabelError::EditRejected("clé déjà utilisée".into())),
            "Édition refusée : clé déjà utilisée"
        );
    }

    #[test]
    fn labels_liste_plafonnee() {
        let invalid_lines: Vec<LabelInvalidRow> = (1..=10).map(|n| (n, "x".into())).collect();
        let msg = handle_label_error(&LabelError::InvalidRowsFormat { invalid_lines });
        assert!(msg.ends_with("… et 2 autres"), "{msg}");
    }

    // --- displayCaptureError -------------------------------------------

    #[test]
    fn erreur_inattendue_affichee_telle_quelle() {
        let msg = unexpected_error_message("panne backend");
        assert_eq!(msg.title, "Erreur Capture (inattendue)");
        assert_eq!(msg.message, "Erreur inattendue : panne backend");
        assert_eq!(
            msg.log_line(),
            "Erreur Capture (inattendue) : Erreur inattendue : panne backend"
        );
    }

    #[test]
    fn titres_par_domaine() {
        let cases = [
            (CaptureStateError::Io(io("x")), "Erreur Capture (io)"),
            (
                CaptureStateError::PoisonError("x".into()),
                "Erreur Capture (poisonError)",
            ),
            (
                CaptureStateError::InvalidTransition {
                    from: "idle".into(),
                    to: "stopping".into(),
                },
                "Erreur Capture (invalidTransition)",
            ),
            (
                CaptureStateError::Export(ExportError::EmptyPath),
                "Erreur Capture (export)",
            ),
            (
                CaptureStateError::Label(LabelError::EditRejected("x".into())),
                "Erreur Capture (label)",
            ),
        ];
        for (err, title) in cases {
            assert_eq!(capture_error_message(&err).title, title);
        }
    }

    #[test]
    fn messages_de_premier_niveau() {
        assert_eq!(
            capture_error_message(&CaptureStateError::Io(io("disque plein"))).message,
            "Erreur IO : disque plein"
        );
        assert_eq!(
            capture_error_message(&CaptureStateError::PoisonError("verrou".into())).message,
            "Erreur verrou : verrou"
        );
        assert_eq!(
            capture_error_message(&CaptureStateError::InvalidTransition {
                from: "importing".into(),
                to: "running".into(),
            })
            .message,
            "Opération refusée : transition de capture refusée : importing → running"
        );
    }

    #[test]
    fn erreurs_de_capture() {
        let cases = [
            (
                CaptureError::InvalidConfig("snaplen".into()),
                "Configuration invalide : snaplen",
            ),
            (
                CaptureError::InterfaceNotFound("eth9".into()),
                "Interface non trouvée : eth9",
            ),
            (
                CaptureError::EventSendError(ChannelError::new("fermé")),
                "Erreur envoi evenement capture : fermé",
            ),
            (
                CaptureError::UnsupportedLinkType("DLT 147".into()),
                "Type de liaison non supporté par cette version : DLT 147.\nLa capture n'a pas démarré.",
            ),
            (
                CaptureError::MixedLinkType("Relevé Ethernet.".into()),
                "Relevé Ethernet.\nExportez ou réinitialisez le relevé en cours avant de capturer sur cette interface.",
            ),
        ];
        for (err, expected) in cases {
            let wrapped = CaptureStateError::Capture(err);
            let msg = capture_error_message(&wrapped);
            assert_eq!(msg.title, "Erreur Capture (capture)");
            assert_eq!(msg.message, expected);
        }
    }

    // --- Npcap absent -------------------------------------------------

    const NPCAP_MESSAGE: &str = "Npcap n'est pas installé sur ce poste.";

    #[test]
    fn npcap_absent_reconnu() {
        let err = CaptureError::NpcapMissing(NPCAP_MESSAGE.into());
        assert_eq!(npcap_missing_message(&err), Some(NPCAP_MESSAGE));
        assert_eq!(capture_error_text(&err), NPCAP_MESSAGE);
    }

    #[test]
    fn npcap_autres_erreurs() {
        assert_eq!(
            npcap_missing_message(&CaptureError::InvalidConfig("x".into())),
            None
        );
    }

    #[test]
    fn import_npcap_absent_releve_preserve() {
        assert_eq!(
            handle_import_error(&PcapImportError::NpcapMissing(NPCAP_MESSAGE.into())),
            format!("{NPCAP_MESSAGE}\nLe relevé courant est inchangé.")
        );
    }
}
