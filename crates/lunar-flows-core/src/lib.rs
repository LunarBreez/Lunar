//! Cœur métier de LUNAR : matrices de flux LFMS, CSV, graphe réseau,
//! conversion PCAP et relevés WinAudit.
//!
//! Le crate ne dépend d'aucune interface : le moteur, l'application et les
//! tests le partagent.

pub mod csv;
pub mod graph;
pub mod lfms;
pub mod link;
pub mod matrix;
pub mod packet;
#[cfg(feature = "pcap")]
pub mod pcap;
pub mod report;
pub mod winaudit;
#[cfg(feature = "xlsx")]
pub mod xlsx;

pub use error::{LunarCoreError, Result};

/// Erreurs du cœur : une variante par refus explicite (entrée
/// manquante, CSV invalide, PCAP illisible, type de liaison non pris en
/// charge…), sans jamais produire un résultat partiel en silence.
pub mod error {
    use std::path::PathBuf;

    /// Erreur renvoyée par toutes les opérations du cœur (matrice, CSV,
    /// PCAP, graphe). Le message `Display` est celui montré à l'opérateur.
    #[derive(Debug, thiserror::Error)]
    pub enum LunarCoreError {
        /// Traitement par lots appelé sans aucun fichier d'entrée.
        #[error("at least one input file is required")]
        MissingInput,

        /// Chemin de sortie vide pour un traitement par lots.
        #[error("output path is required")]
        MissingOutput,

        /// Fichier d'entrée annoncé mais absent du disque (chemin en cause).
        #[error("input file does not exist: {0}")]
        MissingInputFile(PathBuf),

        /// Fichier CSV de matrice refusé (en-têtes, préambule LFMS ou ligne
        /// invalide) : le fichier en cause et le motif.
        #[error("{path}: {message}")]
        InvalidCsv {
            /// Fichier CSV en cause.
            path: PathBuf,
            /// Motif du refus, tel qu'affiché à l'opérateur (ligne numérotée).
            message: String,
        },

        /// Une ligne déjà lue ne peut pas être reconstruite dans le DLT
        /// annoncé. Ce garde interne complète la validation numérotée du CSV
        /// et évite tout repli silencieux si l'API est appelée directement.
        #[error("ligne de matrice non reconstructible : {0}")]
        InvalidMatrixRow(String),

        /// Erreur d'entrée/sortie du système (ouverture, lecture, écriture d'un
        /// fichier), transmise telle quelle.
        #[error(transparent)]
        Io(#[from] std::io::Error),

        /// Fichier PCAP impossible à ouvrir (absent, illisible, format non
        /// reconnu par libpcap).
        #[cfg(feature = "pcap")]
        #[error("{path}: {message}")]
        PcapOpen {
            /// Fichier PCAP en cause.
            path: PathBuf,
            /// Motif renvoyé par libpcap.
            message: String,
        },

        /// Erreur de lecture au milieu d'un fichier PCAP (tronqué, corrompu) :
        /// distincte de la fin normale pour ne jamais produire une matrice
        /// silencieusement partielle.
        #[cfg(feature = "pcap")]
        #[error("{path}: {message}")]
        PcapRead {
            /// Fichier PCAP en cause.
            path: PathBuf,
            /// Motif renvoyé par libpcap.
            message: String,
        },

        /// Type de liaison (LINKTYPE/DLT) sans décodeur dans cette version :
        /// refusé avant toute mutation d'état, jamais parsé « comme si »
        /// c'était de l'Ethernet.
        #[cfg(feature = "pcap")]
        #[error("{path}: type de liaison non supporté : {label}")]
        UnsupportedLinkType {
            /// Fichier PCAP en cause.
            path: PathBuf,
            /// Libellé du type de liaison refusé (nom LINKTYPE ou numéro).
            label: String,
        },

        /// Fusion de relevés de types de liaison différents : refusée
        /// explicitement : une fusion ne concerne que des relevés du même
        /// réseau, donc du même DLT.
        #[error(
            "{path}: type de liaison {found} incompatible avec le relevé en cours ({expected}) : \
             une fusion ne concerne que des relevés du même réseau (même type de liaison)"
        )]
        MixedLinkTypes {
            /// Fichier dont le type de liaison diffère du relevé en cours.
            path: PathBuf,
            /// Type de liaison lu dans ce fichier.
            found: String,
            /// Type de liaison du relevé déjà chargé.
            expected: String,
        },

        /// Relevé dont la projection LFMS n'est pas définie dans cette
        /// version (IEEE 802.11 ou LINKTYPE futur) : refusé plutôt que
        /// réimporté sous un autre type de liaison.
        #[error(
            "{path}: relevé {label} non réimportable : aucune projection d'identité LFMS n'est définie pour ce type de liaison"
        )]
        UnreimportableLinkType {
            /// Fichier CSV en cause.
            path: PathBuf,
            /// Libellé du type de liaison sans projection LFMS.
            label: String,
        },
    }

    /// Résultat du cœur : `Ok` ou une `LunarCoreError`.
    pub type Result<T> = std::result::Result<T, LunarCoreError>;
}

/// Contrôles communs aux traitements par lots, avant toute lecture : au
/// moins une entrée, une sortie nommée, et chaque entrée présente sur le
/// disque. Le premier manquement est renvoyé, dans cet ordre.
pub(crate) fn validate_batch_paths(
    inputs: &[std::path::PathBuf],
    output: &std::path::Path,
) -> Result<()> {
    if inputs.is_empty() {
        return Err(LunarCoreError::MissingInput);
    }

    if output.as_os_str().is_empty() {
        return Err(LunarCoreError::MissingOutput);
    }

    for input in inputs {
        if !input.exists() {
            return Err(LunarCoreError::MissingInputFile(input.clone()));
        }
    }

    Ok(())
}
