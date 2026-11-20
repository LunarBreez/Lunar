//! Erreurs du démarrage et du pipeline de capture réseau.

use crossbeam::channel::TrySendError;
use thiserror::Error;

use crate::state::capture::capture_handle::messages::CaptureMessage;

/// Erreur survenue à la configuration, à l'initialisation ou pendant la
/// capture (canal interne ou IPC).
#[derive(Debug, Error)]
pub enum CaptureError {
    /// Paramètre de capture hors bornes, budget de buffers dépassé ou
    /// interface vide (validation de `CaptureConfig`) ; porte le message
    /// explicatif.
    #[error("Configuration de capture invalide : {0}")]
    InvalidConfig(String),

    /// Lecture, décodage JSON ou écriture du fichier de configuration
    /// persistée impossible ; le message donne le chemin et la cause.
    #[error("Erreur de persistance de la configuration : {0}")]
    ConfigPersistence(String),

    /// Nom d'interface absent de la liste `pcap` au démarrage ; porte le
    /// nom demandé.
    #[error("Interface réseau introuvable : {0}")]
    InterfaceNotFound(String),

    /// Erreur `pcap` à l'énumération des interfaces, mais aussi à
    /// l'ouverture de la capture ou à la pose du filtre BPF (conversion
    /// automatique de toute `pcap::Error`).
    #[error("Erreur lors de la récupération de la liste des interfaces : {0}")]
    DeviceListError(#[from] pcap::Error),

    /// Npcap introuvable sous Windows : capture impossible, mais
    /// LUNAR reste utilisable pour les matrices et les labels.
    #[error("{0}")]
    NpcapMissing(String),

    /// Erreur d'E/S à l'initialisation de la capture (conversion automatique
    /// de `std::io::Error`) ; aucun point d'émission direct dans le pipeline
    /// actuel.
    #[error("Erreur lors de l'initialisation de la capture : {0}")]
    CaptureInitError(#[from] std::io::Error),

    /// Échec d'envoi sur le canal interne capture → traitement (plein ou
    /// fermé). Le thread de capture traite lui-même ces deux cas (compteur
    /// de pertes, fin de thread) ; la variante reste pour la conversion
    /// automatique.
    #[error("Erreur lors de l'envoi via le canal : {0}")]
    ChannelSendError(#[from] TrySendError<CaptureMessage>),

    /// Échec d'envoi d'un événement à l'interface (ex. `Started` au
    /// démarrage, qui annule alors le DLT fixé) ; porte l'erreur du canal.
    #[error("Erreur lors de l'envoi de l'evenement : {0}")]
    EventSendError(#[from] crate::ipc::ChannelError),

    /// Type de liaison de l'interface sans décodeur dans cette version :
    /// refusé au démarrage, avant tout événement `Started` (un DLT non
    /// supporté ne doit jamais être parsé comme de l'Ethernet).
    #[error("Type de liaison non supporté par cette version : {0}")]
    UnsupportedLinkType(String),

    /// Interface d'un autre type de liaison que le relevé en cours : capture
    /// refusée (un relevé = un réseau = un DLT, arbitrage du 14/07/2026).
    #[error("{0}")]
    MixedLinkType(String),
    // #[error("Erreur lors de l'application du filtre : {0}")]
    // FilterError(String),
}

/// Représentation sérialisable de [`CaptureError`] (forme `{ kind, message }`).
#[derive(serde::Serialize)]
#[serde(tag = "kind", content = "message")]
#[serde(rename_all = "camelCase")]
pub enum CaptureErrorKind {
    /// `{ kind: "invalidConfig", message }` : message de validation.
    InvalidConfig(String),
    /// `{ kind: "configPersistence", message }` : chemin et cause.
    ConfigPersistence(String),
    /// `{ kind: "interfaceNotFound", message }` : nom de l'interface demandée.
    InterfaceNotFound(String),
    /// `{ kind: "deviceListError", message }` : message de l'erreur `pcap`.
    DeviceListError(String),
    /// `{ kind: "npcapMissing", message }` : message d'installation de
    /// Npcap.
    NpcapMissing(String),
    /// `{ kind: "captureInitError", message }` : message de l'erreur d'E/S.
    CaptureInitError(String),
    /// `{ kind: "channelSendError", message }` : message de l'erreur de canal.
    ChannelSendError(String),
    /// `{ kind: "eventSendError", message }` : message de l'erreur du canal.
    EventSendError(String),
    /// `{ kind: "unsupportedLinkType", message }` : libellé du DLT refusé.
    UnsupportedLinkType(String),
    /// `{ kind: "mixedLinkType", message }` : message du conflit de DLT avec
    /// le relevé en cours.
    MixedLinkType(String),
    // FilterError(String),
}

impl serde::Serialize for CaptureError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        let kind = match self {
            Self::InvalidConfig(msg) => CaptureErrorKind::InvalidConfig(msg.clone()),
            Self::ConfigPersistence(msg) => CaptureErrorKind::ConfigPersistence(msg.clone()),
            Self::InterfaceNotFound(msg) => CaptureErrorKind::InterfaceNotFound(msg.clone()),
            Self::DeviceListError(e) => CaptureErrorKind::DeviceListError(e.to_string()),
            Self::NpcapMissing(msg) => CaptureErrorKind::NpcapMissing(msg.clone()),
            Self::CaptureInitError(e) => CaptureErrorKind::CaptureInitError(e.to_string()),
            Self::ChannelSendError(e) => CaptureErrorKind::ChannelSendError(e.to_string()),
            Self::EventSendError(e) => CaptureErrorKind::EventSendError(e.to_string()),
            Self::UnsupportedLinkType(label) => {
                CaptureErrorKind::UnsupportedLinkType(label.clone())
            }
            Self::MixedLinkType(message) => CaptureErrorKind::MixedLinkType(message.clone()),
            // Self::FilterError(e) => CaptureErrorKind::FilterError(e.to_string()),
        };
        kind.serialize(serializer)
    }
}
