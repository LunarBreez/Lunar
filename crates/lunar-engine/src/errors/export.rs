//! Erreurs des commandes d'export (CSV, labels, logs, synthèse HTML).

/// Erreur d'export : chemin manquant, E/S, écriture CSV, verrou empoisonné
/// ou dossier de logs introuvable.
#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    /// Chemin de destination vide ou blanc : refusé avant toute écriture.
    #[error("Chemin de fichier vide")]
    EmptyPath,
    /// Erreur d'E/S à l'écriture du fichier (CSV, page HTML, archive) ou à
    /// la copie des logs.
    #[error("Erreur d’E/S: {0}")]
    Io(#[from] std::io::Error),
    /// Erreur du writer CSV (matrice, labels) ; conversion automatique de
    /// `csv::Error`.
    #[error("Erreur CSV: {0}")]
    Csv(#[from] csv::Error),
    /// Erreur d'écriture d'une archive ZIP : logs de support ou projet
    /// `.lunar`.
    #[error("Erreur d’archive ZIP: {0}")]
    Zip(#[from] zip::result::ZipError),
    /// Verrou de la matrice empoisonné par la panique d'un autre thread ;
    /// porte le message d'origine.
    #[error("the mutex was poisoned")]
    PoisonError(String),
    /// Dossier de logs de l'application (`AppPaths::log_dir`) inexistant
    /// (export des logs de support).
    #[error("Le dossier de logs est introuvable.")]
    LogNotFound,
    /// Cible de la synthèse sans extension `.html`/`.htm` : le CSV jumeau ne
    /// peut pas être nommé.
    #[error("La synthèse doit être enregistrée dans un fichier .html")]
    NotHtml,
    /// Un CSV homonyme existe déjà à côté d'une page de synthèse absente :
    /// il n'est pas écrasé en silence ; porte le chemin de ce CSV.
    #[error("Le fichier {0} existe déjà : choisissez un autre nom pour la synthèse")]
    CsvAlreadyExists(String),
}

/// Représentation sérialisable de [`ExportError`] (forme `{ kind, message }`).
#[derive(serde::Serialize)]
#[serde(tag = "kind", content = "message")]
#[serde(rename_all = "camelCase")]
pub enum ExportErrorKind {
    /// `{ kind: "emptyPath" }`, sans charge utile.
    EmptyPath,
    /// `{ kind: "io", message }` : message de l'erreur d'E/S.
    Io(String),
    /// `{ kind: "csv", message }` : message du writer CSV.
    Csv(String),
    /// `{ kind: "zip", message }` : message de l'erreur d'archive.
    Zip(String),
    /// `{ kind: "poisonError", message }` : message du verrou empoisonné.
    PoisonError(String),
    /// `{ kind: "logNotFound" }`, sans charge utile.
    LogNotFound,
    /// `{ kind: "notHtml" }`, sans charge utile.
    NotHtml,
    /// `{ kind: "csvAlreadyExists", message }` : chemin du CSV existant.
    CsvAlreadyExists(String),
}

impl<T> From<std::sync::PoisonError<T>> for ExportError {
    fn from(err: std::sync::PoisonError<T>) -> Self {
        ExportError::PoisonError(err.to_string())
    }
}
