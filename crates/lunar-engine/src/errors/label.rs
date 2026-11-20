//! Erreurs de validation des fichiers de labels CSV.

/// Champ invalide : (n° de ligne, valeur fautive, ligne brute).
pub type LabelInvalidField = (usize, String, String);
/// Ligne invalide : (n° de ligne, contenu affiché).
pub type LabelInvalidRow = (usize, String);
/// Conflit entre deux lignes : (ligne A, ligne B, ip, valeur A, valeur B,
/// ligne brute A, ligne brute B).
pub type LabelConflict = (usize, usize, String, String, String, String, String);

/// Erreur bloquante de validation d'un fichier de labels. Le frontend
/// affiche chaque variante dans un dialogue dédié (`ConflictDialog`).
#[derive(Debug, thiserror::Error)]
pub enum LabelError {
    /// Au moins une MAC ou une IP non vide mal formée dans le fichier (ou
    /// dans une clé saisie dans le panneau de gestion) ; les deux listes
    /// peuvent être remplies à la fois.
    #[error("Adresse MAC invalide: {invalid_mac:?} / Adresse IP invalide: {invalid_ip:?}")]
    InvalidMacIpFormat {
        /// MAC fautives : (n° de ligne, valeur, ligne brute).
        invalid_mac: Vec<LabelInvalidField>,
        /// IP fautives : (n° de ligne, valeur, ligne brute).
        invalid_ip: Vec<LabelInvalidField>,
    },
    /// Lignes inexploitables : moins de trois colonnes, erreur du lecteur
    /// CSV, fichier qui est en fait une matrice de flux exportée, ou clé
    /// saisie avec MAC et IP toutes deux vides.
    #[error(
        "Format de fichier invalide. Attendu : au moins mac, ip, label ; colonnes suivantes fusionnées dans le label. Trouvé : {invalid_lines:?}"
    )]
    InvalidRowsFormat {
        /// Lignes rejetées : (n° de ligne, contenu affiché ou diagnostic).
        invalid_lines: Vec<LabelInvalidRow>,
    },
    // Les conflits de labels ne bloquent plus l'import (résolus « premier
    // gagné » et rapportés via LabelImportReport). Variante conservée pour la
    // compatibilité de l'API d'erreur et le futur module d'arbitrage.
    /// Deux lignes donnent, pour une même IP, une MAC ou un label
    /// différents. Plus émise aujourd'hui : les conflits passent par
    /// l'arbitrage (`LabelConflictStore`) ; la variante reste dans le
    /// contrat d'erreurs (`#[allow(dead_code)]`).
    #[error(
        "Conflits détectés : IP -> Mac : {same_ip_diff_mac:?}, IP -> Label : {same_ip_diff_label:?}"
    )]
    #[allow(dead_code)]
    LabelLinesConflicts {
        /// Conflits IP → MAC, un `LabelConflict` par paire de lignes.
        same_ip_diff_mac: Vec<LabelConflict>,
        /// Conflits IP → label, un `LabelConflict` par paire de lignes.
        same_ip_diff_label: Vec<LabelConflict>,
    },
    /// Édition refusée par le store (ligne introuvable, collision de clé) —
    /// panneau de gestion des labels.
    #[error("édition de label refusée : {0}")]
    EditRejected(String),
}

/// Représentation sérialisable de [`LabelError`] (forme `{ kind, message }`).
#[derive(serde::Serialize)]
#[serde(tag = "kind", content = "message")]
#[serde(rename_all = "camelCase")]
pub enum LabelErrorKind {
    /// `{ kind: "invalidMacIpFormat", message: [MAC fautives, IP fautives] }`.
    InvalidMacIpFormat(Vec<LabelInvalidField>, Vec<LabelInvalidField>),
    /// `{ kind: "invalidRowsFormat", message: lignes rejetées }`.
    InvalidRowsFormat(Vec<LabelInvalidRow>),
    /// `{ kind: "labelLinesConflicts", message: [conflits IP → MAC,
    /// conflits IP → label] }`.
    LabelLinesConflicts(Vec<LabelConflict>, Vec<LabelConflict>),
    /// `{ kind: "editRejected", message }` : motif du refus.
    EditRejected(String),
}
