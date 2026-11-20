//! Erreurs de l'application. [`CaptureStateError`] agrège les erreurs de
//! chaque domaine (capture, export, import, labels) et se sérialise vers le
//! frontend sous la forme discriminée `{ kind, message }`. Un échec de
//! livraison d'événement passe par `CaptureError::EventSendError`.

use capture_error::{CaptureError, CaptureErrorKind};
use serde::Serialize;

use crate::errors::{
    export::{ExportError, ExportErrorKind},
    import::{PcapImportError, PcapImportErrorKind},
    label::{LabelError, LabelErrorKind},
};

pub mod capture_error;
pub mod export;
pub mod import;
pub mod label;

/// Erreur agrégée retournée par les commandes de la façade : chaque variante
/// enveloppe l'erreur d'un domaine (IO, verrou empoisonné, capture, export,
/// import PCAP, labels).
#[derive(Debug, thiserror::Error)]
pub enum CaptureStateError {
    /// Erreur d'E/S remontée telle quelle (lecture ou écriture d'un fichier
    /// par une commande) ; reçoit aussi les erreurs du cœur partagé sans
    /// variante dédiée, avec leur message d'origine.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// Verrou de l'état partagé empoisonné par la panique d'un autre thread ;
    /// porte le message du `PoisonError` d'origine.
    #[error("the mutex was poisoned")]
    PoisonError(String),
    /// Transition refusée par la machine d'état de capture (ex. démarrage
    /// pendant qu'une capture tourne déjà).
    #[error("transition de capture refusée : {from} → {to}")]
    InvalidTransition {
        /// Phase courante (ou situation) qui a refusé la transition, telle
        /// qu'affichée (`idle`, `importing`…).
        from: String,
        /// Phase visée par la transition refusée.
        to: String,
    },
    /// Erreur du domaine capture : configuration, interface, ouverture
    /// `pcap`, pipeline (voir `CaptureError`).
    #[error(transparent)]
    Capture(#[from] CaptureError),
    /// Erreur d'une commande d'export (CSV, labels, logs, synthèse, projet).
    #[error(transparent)]
    Export(#[from] ExportError),
    /// Erreur d'import PCAP, de matrice ou d'archive projet.
    #[error(transparent)]
    Import(#[from] PcapImportError),
    /// Erreur de validation ou d'édition des labels.
    #[error(transparent)]
    Label(#[from] LabelError),
}

/// Représentation sérialisable de [`CaptureStateError`] : forme discriminée
/// `{ kind, message }` consommée telle quelle par le frontend.
#[derive(serde::Serialize)]
#[serde(tag = "kind", content = "message")]
#[serde(rename_all = "camelCase")]
pub enum CaptureStateErrorKind {
    /// `{ kind: "io", message }` : message de l'erreur d'E/S.
    Io(String),
    /// `{ kind: "poisonError", message }` : message du verrou empoisonné.
    PoisonError(String),
    /// `{ kind: "invalidTransition", message }` : message complet
    /// « transition de capture refusée : from → to ».
    InvalidTransition(String),
    /// `{ kind: "capture", message }` : l'erreur de capture, elle-même
    /// discriminée `{ kind, message }`.
    Capture(CaptureErrorKind),
    /// `{ kind: "export", message }` : l'erreur d'export, elle-même
    /// discriminée `{ kind, message }`.
    Export(ExportErrorKind),
    /// `{ kind: "import", message }` : l'erreur d'import, elle-même
    /// discriminée `{ kind, message }`.
    Import(PcapImportErrorKind),
    /// `{ kind: "label", message }` : l'erreur de labels, elle-même
    /// discriminée `{ kind, message }`.
    Label(LabelErrorKind),
}

impl Serialize for CaptureStateError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        let kind = match self {
            Self::Io(e) => CaptureStateErrorKind::Io(e.to_string()),
            Self::PoisonError(e) => CaptureStateErrorKind::PoisonError(e.clone()),
            Self::InvalidTransition { .. } => {
                CaptureStateErrorKind::InvalidTransition(self.to_string())
            }
            Self::Capture(e) => {
                // Convert `CaptureError` into `CaptureErrorKind`
                let kind = match e {
                    CaptureError::InvalidConfig(msg) => {
                        CaptureErrorKind::InvalidConfig(msg.clone())
                    }
                    CaptureError::ConfigPersistence(msg) => {
                        CaptureErrorKind::ConfigPersistence(msg.clone())
                    }
                    CaptureError::InterfaceNotFound(msg) => {
                        CaptureErrorKind::InterfaceNotFound(msg.clone())
                    }
                    CaptureError::DeviceListError(e) => {
                        CaptureErrorKind::DeviceListError(e.to_string())
                    }
                    CaptureError::NpcapMissing(msg) => CaptureErrorKind::NpcapMissing(msg.clone()),
                    CaptureError::CaptureInitError(e) => {
                        CaptureErrorKind::CaptureInitError(e.to_string())
                    }
                    CaptureError::ChannelSendError(e) => {
                        CaptureErrorKind::ChannelSendError(e.to_string())
                    }
                    CaptureError::EventSendError(e) => {
                        CaptureErrorKind::EventSendError(e.to_string())
                    }
                    CaptureError::UnsupportedLinkType(label) => {
                        CaptureErrorKind::UnsupportedLinkType(label.clone())
                    }
                    CaptureError::MixedLinkType(message) => {
                        CaptureErrorKind::MixedLinkType(message.clone())
                    } //   CaptureError::FilterError(e) => CaptureErrorKind::FilterError(e.to_string()),
                };
                CaptureStateErrorKind::Capture(kind)
            }
            Self::Export(e) => {
                let kind = match e {
                    ExportError::EmptyPath => ExportErrorKind::EmptyPath,
                    ExportError::Io(e) => ExportErrorKind::Io(e.to_string()),
                    ExportError::Csv(e) => ExportErrorKind::Csv(e.to_string()),
                    ExportError::Zip(e) => ExportErrorKind::Zip(e.to_string()),
                    ExportError::PoisonError(e) => ExportErrorKind::PoisonError(e.clone()),
                    ExportError::LogNotFound => ExportErrorKind::LogNotFound,
                    ExportError::NotHtml => ExportErrorKind::NotHtml,
                    ExportError::CsvAlreadyExists(path) => {
                        ExportErrorKind::CsvAlreadyExists(path.clone())
                    }
                };
                CaptureStateErrorKind::Export(kind)
            }
            Self::Import(e) => {
                let kind = match e {
                    PcapImportError::MissingInput(operation) => {
                        PcapImportErrorKind::MissingInput(operation.clone())
                    }
                    PcapImportError::NpcapMissing(msg) => {
                        PcapImportErrorKind::NpcapMissing(msg.clone())
                    }
                    PcapImportError::OpenFileError(msg, msgg) => {
                        PcapImportErrorKind::OpenFileError(msg.clone(), msgg.clone())
                    }
                    PcapImportError::ReadPacketError(file, msg) => {
                        PcapImportErrorKind::ReadPacketError(file.clone(), msg.clone())
                    }
                    PcapImportError::UnsupportedLinkType(file, label) => {
                        PcapImportErrorKind::UnsupportedLinkType(file.clone(), label.clone())
                    }
                    PcapImportError::Cancelled(operation) => {
                        PcapImportErrorKind::Cancelled(operation.clone())
                    }
                };
                CaptureStateErrorKind::Import(kind)
            }
            Self::Label(e) => {
                let kind = match e {
                    LabelError::InvalidMacIpFormat {
                        invalid_mac,
                        invalid_ip,
                    } => {
                        LabelErrorKind::InvalidMacIpFormat(invalid_mac.clone(), invalid_ip.clone())
                    }
                    LabelError::LabelLinesConflicts {
                        same_ip_diff_mac,
                        same_ip_diff_label,
                    } => LabelErrorKind::LabelLinesConflicts(
                        same_ip_diff_mac.clone(),
                        same_ip_diff_label.clone(),
                    ),
                    LabelError::InvalidRowsFormat { invalid_lines } => {
                        LabelErrorKind::InvalidRowsFormat(invalid_lines.clone())
                    }
                    LabelError::EditRejected(reason) => {
                        LabelErrorKind::EditRejected(reason.clone())
                    }
                };
                CaptureStateErrorKind::Label(kind)
            }
        };
        kind.serialize(serializer)
    }
}

/// Un événement non livré (`?` sur [`crate::ipc::Channel::send`]) est une
/// erreur du domaine capture, sérialisée `capture.eventSendError`.
impl From<crate::ipc::ChannelError> for CaptureStateError {
    fn from(err: crate::ipc::ChannelError) -> Self {
        CaptureStateError::Capture(CaptureError::EventSendError(err))
    }
}

impl<T> From<std::sync::PoisonError<T>> for CaptureStateError {
    fn from(err: std::sync::PoisonError<T>) -> Self {
        CaptureStateError::PoisonError(err.to_string())
    }
}

/// Les erreurs du cœur partagé gardent leur distinction ouverture/lecture
/// (préservée côté front par `PcapImportErrorKind`) ; le reste (CSV invalide,
/// IO…) passe par la variante `Io` avec son message d'origine.
impl From<lunar_flows_core::LunarCoreError> for CaptureStateError {
    fn from(err: lunar_flows_core::LunarCoreError) -> Self {
        use lunar_flows_core::LunarCoreError;
        match err {
            LunarCoreError::PcapOpen { path, message } => CaptureStateError::Import(
                PcapImportError::OpenFileError(path.display().to_string(), message),
            ),
            LunarCoreError::PcapRead { path, message } => CaptureStateError::Import(
                PcapImportError::ReadPacketError(path.display().to_string(), message),
            ),
            LunarCoreError::UnsupportedLinkType { path, label } => CaptureStateError::Import(
                PcapImportError::UnsupportedLinkType(path.display().to_string(), label),
            ),
            LunarCoreError::MissingInput => {
                CaptureStateError::Import(PcapImportError::MissingInput("cet import".to_string()))
            }
            LunarCoreError::Io(e) => CaptureStateError::Io(e),
            other => CaptureStateError::Io(std::io::Error::other(other.to_string())),
        }
    }
}

/// Fidélité JSON du contrat d'erreurs. Il n'y a pas de miroir
/// dupliqué à faire dériver ici : `CaptureStateErrorKind` (et les `XxxKind`
/// par domaine) sont la cible de sérialisation — un seul `match`
/// exhaustif construit l'un depuis l'autre (`impl Serialize for
/// CaptureStateError` ci-dessus), sans possibilité de double définition qui
/// diverge. Le risque restant est plus étroit (le tag `{kind,message}` et le
/// `camelCase` de premier niveau), ce que ces tests couvrent par domaine.
#[cfg(test)]
mod fidelity_tests {
    use super::*;
    use serde_json::json;

    fn assert_json(error: &CaptureStateError, expected: serde_json::Value, label: &str) {
        assert_eq!(
            serde_json::to_value(error).unwrap(),
            expected,
            "{label} : sérialisation divergente du contrat attendu"
        );
    }

    #[test]
    fn io_and_poison_and_invalid_transition_match() {
        assert_json(
            &CaptureStateError::PoisonError("verrou capturé".to_string()),
            json!({"kind": "poisonError", "message": "verrou capturé"}),
            "poisonError",
        );
        assert_json(
            &CaptureStateError::InvalidTransition {
                from: "Idle".to_string(),
                to: "Running".to_string(),
            },
            json!({"kind": "invalidTransition", "message": "transition de capture refusée : Idle → Running"}),
            "invalidTransition",
        );
    }

    #[test]
    fn capture_domain_nests_under_capture_tag() {
        assert_json(
            &CaptureStateError::Capture(CaptureError::InvalidConfig(
                "snaplen invalide".to_string(),
            )),
            json!({"kind": "capture", "message": {"kind": "invalidConfig", "message": "snaplen invalide"}}),
            "capture.invalidConfig",
        );
        assert_json(
            &CaptureStateError::Capture(CaptureError::UnsupportedLinkType("DLT 999".to_string())),
            json!({"kind": "capture", "message": {"kind": "unsupportedLinkType", "message": "DLT 999"}}),
            "capture.unsupportedLinkType",
        );
    }

    #[test]
    fn export_domain_nests_under_export_tag() {
        assert_json(
            &CaptureStateError::Export(ExportError::EmptyPath),
            json!({"kind": "export", "message": {"kind": "emptyPath"}}),
            "export.emptyPath",
        );
        assert_json(
            &CaptureStateError::Export(ExportError::LogNotFound),
            json!({"kind": "export", "message": {"kind": "logNotFound"}}),
            "export.logNotFound",
        );
    }

    #[test]
    fn import_domain_nests_under_import_tag() {
        assert_json(
            &CaptureStateError::Import(PcapImportError::MissingInput("l'import PCAP".to_string())),
            json!({
                "kind": "import",
                "message": {"kind": "missingInput", "message": "l'import PCAP"},
            }),
            "import.missingInput",
        );
        assert_json(
            &CaptureStateError::Import(PcapImportError::OpenFileError(
                "capture.pcap".to_string(),
                "permission refusée".to_string(),
            )),
            json!({
                "kind": "import",
                "message": {"kind": "openFileError", "message": ["capture.pcap", "permission refusée"]},
            }),
            "import.openFileError",
        );
    }

    #[test]
    fn label_domain_nests_under_label_tag() {
        assert_json(
            &CaptureStateError::Label(LabelError::EditRejected("ligne introuvable".to_string())),
            json!({"kind": "label", "message": {"kind": "editRejected", "message": "ligne introuvable"}}),
            "label.editRejected",
        );
        assert_json(
            &CaptureStateError::Label(LabelError::InvalidMacIpFormat {
                invalid_mac: vec![(3, "zz:zz".to_string(), "3,zz:zz,1.2.3.4".to_string())],
                invalid_ip: vec![],
            }),
            json!({
                "kind": "label",
                "message": {
                    "kind": "invalidMacIpFormat",
                    "message": [[[3, "zz:zz", "3,zz:zz,1.2.3.4"]], []],
                },
            }),
            "label.invalidMacIpFormat",
        );
    }
}

/// Conversion `lunar_flows_core::LunarCoreError -> CaptureStateError` : les
/// trois variantes PCAP distinguées (ouverture/lecture/DLT non supporté)
/// gardent leur identité côté front, le reste retombe sur `Io`.
#[cfg(test)]
mod lunar_core_error_conversion_tests {
    use super::*;
    use lunar_flows_core::LunarCoreError;
    use std::path::PathBuf;

    #[test]
    fn pcap_open_becomes_import_open_file_error() {
        let err: CaptureStateError = LunarCoreError::PcapOpen {
            path: PathBuf::from("capture.pcap"),
            message: "permission refusée".to_string(),
        }
        .into();
        match err {
            CaptureStateError::Import(PcapImportError::OpenFileError(path, message)) => {
                assert_eq!(path, "capture.pcap");
                assert_eq!(message, "permission refusée");
            }
            other => panic!("attendu Import(OpenFileError), reçu {other:?}"),
        }
    }

    #[test]
    fn pcap_read_becomes_import_read_packet_error() {
        let err: CaptureStateError = LunarCoreError::PcapRead {
            path: PathBuf::from("capture.pcap"),
            message: "paquet tronqué".to_string(),
        }
        .into();
        match err {
            CaptureStateError::Import(PcapImportError::ReadPacketError(path, message)) => {
                assert_eq!(path, "capture.pcap");
                assert_eq!(message, "paquet tronqué");
            }
            other => panic!("attendu Import(ReadPacketError), reçu {other:?}"),
        }
    }

    #[test]
    fn unsupported_link_type_is_preserved() {
        let err: CaptureStateError = LunarCoreError::UnsupportedLinkType {
            path: PathBuf::from("capture.pcap"),
            label: "DLT_RAW".to_string(),
        }
        .into();
        match err {
            CaptureStateError::Import(PcapImportError::UnsupportedLinkType(path, label)) => {
                assert_eq!(path, "capture.pcap");
                assert_eq!(label, "DLT_RAW");
            }
            other => panic!("attendu Import(UnsupportedLinkType), reçu {other:?}"),
        }
    }

    #[test]
    fn io_variant_passes_through() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "fichier introuvable");
        let err: CaptureStateError = LunarCoreError::Io(io_err).into();
        match err {
            CaptureStateError::Io(e) => assert_eq!(e.kind(), std::io::ErrorKind::NotFound),
            other => panic!("attendu Io, reçu {other:?}"),
        }
    }

    /// L'absence d'entrée garde une identité typée jusqu'au frontend.
    #[test]
    fn missing_input_becomes_typed_import_error() {
        let err: CaptureStateError = LunarCoreError::MissingInput.into();
        match err {
            CaptureStateError::Import(PcapImportError::MissingInput(operation)) => {
                assert_eq!(operation, "cet import");
            }
            other => panic!("attendu Import(MissingInput), reçu {other:?}"),
        }
    }
}
