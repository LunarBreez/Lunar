//! Projets récents et dossier du dernier projet, conservés dans le fichier
//! de préférences d'interface `ui-preferences.json` (objet JSON, clé
//! `recentProjects`), dont le chemin est fourni par l'appelant.
//!
//! Préférences d'interface uniquement : jamais bloquant. Chaque opération
//! rend une erreur que l'appelant trace avant de continuer (liste vide,
//! dossier non prérempli). La configuration de capture, elle, reste sur la
//! persistance atomique du moteur.

use std::path::{Path, PathBuf};
use std::{fmt, fs, io};

use serde_json::{Map, Value};

/// Nombre maximal de projets récents conservés.
pub const MAX_RECENT_PROJECTS: usize = 10;

/// Nom du fichier de préférences d'interface.
pub const UI_PREFERENCES_FILE: &str = "ui-preferences.json";

/// Clé de la liste des projets récents dans le fichier de préférences.
pub const KEY_RECENT_PROJECTS: &str = "recentProjects";

/// Ajoute `path` en tête de liste, dédupliqué, plafonné à
/// [`MAX_RECENT_PROJECTS`] entrées.
pub fn push_recent(list: &[String], path: &str) -> Vec<String> {
    push_recent_capped(list, path, MAX_RECENT_PROJECTS)
}

/// Ajoute `path` en tête de liste, dédupliqué, plafonné à `max` entrées.
pub fn push_recent_capped(list: &[String], path: &str, max: usize) -> Vec<String> {
    std::iter::once(path)
        .chain(
            list.iter()
                .map(String::as_str)
                .filter(|entry| *entry != path),
        )
        .take(max)
        .map(str::to_owned)
        .collect()
}

/// Dossier parent d'un chemin, séparateurs `/` et `\` confondus ; `None` si
/// le chemin n'a pas de parent exploitable (pas de séparateur, ou seulement
/// en tête : `/audit.lunar`).
pub fn dirname_of(path: &str) -> Option<&str> {
    match path.rfind(['/', '\\']) {
        Some(cut) if cut > 0 => Some(&path[..cut]),
        _ => None,
    }
}

/// Erreur de lecture ou d'écriture des préférences d'interface.
#[derive(Debug)]
pub enum PreferencesError {
    /// Lecture ou écriture du fichier impossible.
    Io(io::Error),
    /// Contenu qui n'est pas du JSON valide.
    Json(serde_json::Error),
    /// JSON valide mais de forme inattendue.
    Format(&'static str),
}

impl fmt::Display for PreferencesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PreferencesError::Io(err) => write!(f, "préférences d'interface inaccessibles : {err}"),
            PreferencesError::Json(err) => write!(f, "préférences d'interface illisibles : {err}"),
            PreferencesError::Format(what) => {
                write!(f, "préférences d'interface mal formées : {what}")
            }
        }
    }
}

impl std::error::Error for PreferencesError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            PreferencesError::Io(err) => Some(err),
            PreferencesError::Json(err) => Some(err),
            PreferencesError::Format(_) => None,
        }
    }
}

impl From<io::Error> for PreferencesError {
    fn from(err: io::Error) -> Self {
        PreferencesError::Io(err)
    }
}

impl From<serde_json::Error> for PreferencesError {
    fn from(err: serde_json::Error) -> Self {
        PreferencesError::Json(err)
    }
}

/// Fichier de préférences d'interface (objet JSON). Les clés inconnues sont
/// conservées à l'écriture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiPreferences {
    /// Chemin du fichier JSON.
    path: PathBuf,
}

impl UiPreferences {
    /// Préférences stockées dans `path` (le fichier peut ne pas exister).
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Chemin du fichier.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Projets récents, du plus récent au plus ancien ; liste vide si le
    /// fichier ou la clé n'existe pas encore.
    pub fn recent_projects(&self) -> Result<Vec<String>, PreferencesError> {
        recents_in(&self.load()?)
    }

    /// Enregistre un projet ouvert ou sauvegardé en tête des récents et
    /// rend la nouvelle liste. Un fichier illisible n'est pas écrasé.
    pub fn record_recent_project(&self, project: &str) -> Result<Vec<String>, PreferencesError> {
        let mut prefs = self.load()?;
        let recents = push_recent(&recents_in(&prefs)?, project);
        prefs.insert(
            KEY_RECENT_PROJECTS.to_owned(),
            Value::Array(recents.iter().cloned().map(Value::String).collect()),
        );
        self.save(&prefs)?;
        Ok(recents)
    }

    /// Dossier du projet le plus récent, pour préremplir les dialogues.
    pub fn last_project_dir(&self) -> Result<Option<String>, PreferencesError> {
        let recents = self.recent_projects()?;
        Ok(recents
            .first()
            .and_then(|latest| dirname_of(latest))
            .map(str::to_owned))
    }

    /// Contenu du fichier ; objet vide si le fichier n'existe pas.
    fn load(&self) -> Result<Map<String, Value>, PreferencesError> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Map::new()),
            Err(err) => return Err(err.into()),
        };
        match serde_json::from_slice(&bytes)? {
            Value::Object(map) => Ok(map),
            _ => Err(PreferencesError::Format("objet JSON attendu")),
        }
    }

    /// Écrit le fichier par remplacement (fichier temporaire voisin puis
    /// renommage) : une coupure n'en laisse jamais un à moitié écrit.
    fn save(&self, prefs: &Map<String, Value>) -> Result<(), PreferencesError> {
        if let Some(dir) = self.path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
            fs::create_dir_all(dir)?;
        }
        let mut tmp = self.path.clone().into_os_string();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        fs::write(&tmp, serde_json::to_vec_pretty(prefs)?)?;
        fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

/// Liste des récents lue dans les préférences ; vide si la clé est absente.
fn recents_in(prefs: &Map<String, Value>) -> Result<Vec<String>, PreferencesError> {
    match prefs.get(KEY_RECENT_PROJECTS) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::to_owned)
                    .ok_or(PreferencesError::Format("chemin de projet non textuel"))
            })
            .collect(),
        Some(_) => Err(PreferencesError::Format(
            "liste de projets récents attendue",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Liste de chemins possédés.
    fn owned(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|p| (*p).to_owned()).collect()
    }

    // --- Liste des projets récents ---

    #[test]
    fn ajoute_en_tete_d_une_liste_vide() {
        assert_eq!(push_recent(&[], "/a/p1.lunar"), ["/a/p1.lunar"]);
    }

    #[test]
    fn le_plus_recent_passe_devant() {
        assert_eq!(
            push_recent(&owned(&["/a/p1.lunar"]), "/a/p2.lunar"),
            ["/a/p2.lunar", "/a/p1.lunar"]
        );
    }

    #[test]
    fn un_chemin_deja_present_remonte_sans_doublon() {
        assert_eq!(
            push_recent(&owned(&["/a/p2.lunar", "/a/p1.lunar"]), "/a/p1.lunar"),
            ["/a/p1.lunar", "/a/p2.lunar"]
        );
    }

    #[test]
    fn la_liste_est_plafonnee() {
        let full: Vec<String> = (0..MAX_RECENT_PROJECTS)
            .map(|i| format!("/a/p{i}.lunar"))
            .collect();
        let result = push_recent(&full, "/a/nouveau.lunar");
        assert_eq!(result.len(), MAX_RECENT_PROJECTS);
        assert_eq!(result[0], "/a/nouveau.lunar");
        let oldest = format!("/a/p{}.lunar", MAX_RECENT_PROJECTS - 1);
        assert!(!result.contains(&oldest), "le plus ancien est évincé");
    }

    #[test]
    fn dirname_chemins_posix_et_windows() {
        assert_eq!(dirname_of("/home/user/audit.lunar"), Some("/home/user"));
        assert_eq!(dirname_of("C:\\relevés\\audit.lunar"), Some("C:\\relevés"));
        assert_eq!(dirname_of("audit.lunar"), None);
        assert_eq!(dirname_of("/audit.lunar"), None);
    }

    // --- Fichier de préférences ---

    #[test]
    fn fichier_absent_liste_vide_et_pas_de_dossier() {
        let dir = tempfile::tempdir().unwrap();
        let prefs = UiPreferences::new(dir.path().join(UI_PREFERENCES_FILE));
        assert_eq!(prefs.recent_projects().unwrap(), Vec::<String>::new());
        assert_eq!(prefs.last_project_dir().unwrap(), None);
    }

    #[test]
    fn enregistrement_relu_et_dossier_du_plus_recent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sous-dossier").join(UI_PREFERENCES_FILE);
        let prefs = UiPreferences::new(&path);
        prefs.record_recent_project("/releves/a.lunar").unwrap();
        let list = prefs.record_recent_project("/audits/b.lunar").unwrap();
        assert_eq!(list, ["/audits/b.lunar", "/releves/a.lunar"]);

        let reopened = UiPreferences::new(&path);
        assert_eq!(reopened.recent_projects().unwrap(), list);
        assert_eq!(
            reopened.last_project_dir().unwrap().as_deref(),
            Some("/audits")
        );
        assert!(
            !path.with_extension("json.tmp").exists(),
            "pas de fichier temporaire laissé"
        );
    }

    #[test]
    fn format_du_plugin_store_et_cles_inconnues_conservees() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(UI_PREFERENCES_FILE);
        fs::write(
            &path,
            r#"{"recentProjects":["/x/ancien.lunar"],"autreCle":{"a":1}}"#,
        )
        .unwrap();
        let prefs = UiPreferences::new(&path);
        prefs.record_recent_project("/x/nouveau.lunar").unwrap();
        let written: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            written,
            serde_json::json!({
                "recentProjects": ["/x/nouveau.lunar", "/x/ancien.lunar"],
                "autreCle": {"a": 1},
            })
        );
    }

    #[test]
    fn fichier_illisible_signale_et_jamais_ecrase() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(UI_PREFERENCES_FILE);
        fs::write(&path, "pas du json").unwrap();
        let prefs = UiPreferences::new(&path);
        assert!(matches!(
            prefs.recent_projects(),
            Err(PreferencesError::Json(_))
        ));
        assert!(prefs.record_recent_project("/x/p.lunar").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "pas du json");
    }

    #[test]
    fn forme_inattendue_signalee() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(UI_PREFERENCES_FILE);
        let prefs = UiPreferences::new(&path);
        for content in [
            "[1, 2]",
            r#"{"recentProjects": "/x"}"#,
            r#"{"recentProjects": [1]}"#,
        ] {
            fs::write(&path, content).unwrap();
            let err = prefs.recent_projects().unwrap_err();
            assert!(
                matches!(err, PreferencesError::Format(_)),
                "{content} : {err}"
            );
        }
    }
}
