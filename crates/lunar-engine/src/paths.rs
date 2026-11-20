//! Répertoires de l'application : configuration, données et journaux.

use std::path::PathBuf;

use crate::errors::CaptureStateError;

/// Identifiant d'application par défaut : il nomme les répertoires créés
/// sous les dossiers de configuration, de données et de journaux de
/// l'utilisateur.
pub const DEFAULT_APP_ID: &str = "fr.lunar";

/// Répertoires où le moteur lit et écrit ses fichiers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    /// Configuration persistée (`capture_config.json`).
    pub config_dir: PathBuf,
    /// Données de session (`autosave/`, `session.lock`).
    pub data_dir: PathBuf,
    /// Journaux exportés par `export_logs`.
    pub log_dir: PathBuf,
}

impl AppPaths {
    /// Répertoires de l'identifiant [`DEFAULT_APP_ID`].
    pub fn for_default_app() -> Result<Self, CaptureStateError> {
        Self::for_app_id(DEFAULT_APP_ID)
    }

    /// Répertoires d'une application d'identifiant `app_id` :
    /// `config_dir()/app_id`, `data_dir()/app_id`, et pour les
    /// journaux `~/Library/Logs/app_id` sous macOS,
    /// `data_local_dir()/app_id/logs` ailleurs. Rien n'est créé sur le
    /// disque ; échoue seulement quand le répertoire personnel est
    /// introuvable.
    pub fn for_app_id(app_id: &str) -> Result<Self, CaptureStateError> {
        let base = |dir: Option<PathBuf>, what: &str| {
            dir.map(|dir| dir.join(app_id)).ok_or_else(|| {
                CaptureStateError::Io(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("répertoire {what} de l'utilisateur introuvable"),
                ))
            })
        };
        #[cfg(target_os = "macos")]
        let log_dir = base(
            dirs::home_dir().map(|home| home.join("Library/Logs")),
            "des journaux",
        )?;
        #[cfg(not(target_os = "macos"))]
        let log_dir = base(dirs::data_local_dir(), "des journaux")?.join("logs");
        Ok(Self {
            config_dir: base(dirs::config_dir(), "de configuration")?,
            data_dir: base(dirs::data_dir(), "de données")?,
            log_dir,
        })
    }

    /// Les trois répertoires sous une même racine (`config`, `data`, `logs`) :
    /// pratique pour les tests et les installations portables.
    pub fn under(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            config_dir: root.join("config"),
            data_dir: root.join("data"),
            log_dir: root.join("logs"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_id_names_every_directory() {
        let Ok(paths) = AppPaths::for_app_id("fr.lunar.test") else {
            // Pas de répertoire personnel (conteneur minimal) : rien à vérifier.
            return;
        };
        for dir in [&paths.config_dir, &paths.data_dir, &paths.log_dir] {
            assert!(
                dir.components()
                    .any(|part| part.as_os_str() == "fr.lunar.test"),
                "{} doit contenir l'identifiant",
                dir.display()
            );
        }
    }

    #[test]
    fn under_keeps_the_three_directories_apart() {
        let paths = AppPaths::under("/racine");
        assert_eq!(paths.config_dir, PathBuf::from("/racine/config"));
        assert_eq!(paths.data_dir, PathBuf::from("/racine/data"));
        assert_eq!(paths.log_dir, PathBuf::from("/racine/logs"));
    }
}
