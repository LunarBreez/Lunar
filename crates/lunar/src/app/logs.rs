//! Journal de l'application : format des lignes et politique de fichiers.
//!
//! - Ligne : `[AAAA-MM-JJ][HH:MM:SS][cible][NIVEAU] message`, horodatage UTC,
//!   cible = module Rust émetteur.
//! - Destinations : la sortie standard, `lunar.log` et
//!   `DR_LUNAR_<date locale>_<heure locale>.log` dans le répertoire des
//!   journaux ([`lunar_engine::AppPaths::log_dir`], que `export_logs`
//!   archive).
//! - Rotation au démarrage seulement : un fichier de plus de 500 000 octets
//!   est renommé `<nom>_<AAAA-MM-JJ_HH-MM-SS>.log`, au plus 5 fichiers
//!   tournés gardés par nom.
//! - Rétention entre sessions : au plus 30 fichiers `DR_LUNAR_*`,
//!   les plus anciens supprimés ([`prune_old_logs`]).
//!
//! Filtrage : `RUST_LOG` s'il est défini (par exemple `info,lunar=debug`),
//! sinon [`DEFAULT_FILTER`]. Les dépendances sont bornées à Warn (gpui et le
//! parseur à Info) pour garder le journal lisible : le moteur de rendu écrit
//! des dizaines de lignes Info au démarrage.
//!
//! Le harnais E2E lit la sortie standard et l'erreur du processus, pas les
//! fichiers : les lignes y sont écrites à l'identique.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{Local, NaiveDateTime, Utc};
use log::{Level, Log, Metadata, Record, info, warn};

/// Filtre par défaut, sans `RUST_LOG`.
pub const DEFAULT_FILTER: &str = "warn,gpui=info,packet_parser=info,lunar=debug,\
     lunar_engine=debug,lunar_ui_logic=debug,lunar_flows_core=debug";

/// Nom du journal principal (le nom du produit).
pub const MAIN_LOG_NAME: &str = "lunar";

/// Préfixe du journal de session.
pub const SESSION_LOG_PREFIX: &str = "DR_LUNAR_";

/// Taille au-delà de laquelle un journal est tourné au démarrage.
pub const MAX_FILE_SIZE: u64 = 500_000;

/// Fichiers tournés gardés par nom.
pub const KEEP_ROTATED: usize = 5;

/// Journaux de session gardés entre sessions.
pub const MAX_SESSION_LOGS: usize = 30;

/// Ligne de journal : `[AAAA-MM-JJ][HH:MM:SS][cible][NIVEAU] message`.
pub fn format_line(
    utc: NaiveDateTime,
    target: &str,
    level: Level,
    message: &std::fmt::Arguments<'_>,
) -> String {
    format!(
        "[{}][{}][{target}][{level}] {message}",
        utc.format("%Y-%m-%d"),
        utc.format("%H:%M:%S")
    )
}

/// Nom (sans extension) du journal de session ouvert à `local`.
pub fn session_log_name(local: NaiveDateTime) -> String {
    format!(
        "{SESSION_LOG_PREFIX}{}_{}",
        local.format("%Y-%m-%d"),
        local.format("%H-%M-%S")
    )
}

/// Rotation de démarrage de `<dir>/<file_name>.log` : au-delà de `max_size`
/// octets, les tournés les plus anciens sont supprimés pour n'en garder que
/// `keep - 1`, puis le fichier devient `<file_name>_<utc>.log`.
pub fn rotate_on_startup(
    dir: &Path,
    file_name: &str,
    max_size: u64,
    keep: usize,
    utc: NaiveDateTime,
) -> io::Result<()> {
    let path = dir.join(format!("{file_name}.log"));
    if !path.exists() || fs::metadata(&path)?.len() <= max_size {
        return Ok(());
    }
    let mut rotated: Vec<(PathBuf, String)> = fs::read_dir(dir)?
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            let name = path.file_name()?.to_string_lossy().into_owned();
            let date = name
                .strip_prefix(file_name)?
                .strip_prefix('_')?
                .strip_suffix(".log")?
                .to_owned();
            Some((path, date))
        })
        .collect();
    rotated.sort_by(|a, b| a.1.cmp(&b.1));
    let keep = keep.max(1);
    if rotated.len() > keep - 1 {
        rotated.truncate(rotated.len() + 1 - keep);
        for (old, _) in rotated {
            fs::remove_file(old)?;
        }
    }
    let to = dir.join(format!(
        "{file_name}_{}.log",
        utc.format("%Y-%m-%d_%H-%M-%S")
    ));
    if to.is_file() {
        // Même seconde : très improbable, l'ancien tourné devient `.bak`.
        let mut bak = to.clone().into_os_string();
        bak.push(".bak");
        fs::rename(&to, bak)?;
    }
    fs::rename(&path, to)
}

/// Supprime les journaux de session les plus anciens au-delà de
/// [`MAX_SESSION_LOGS`]. Le nommage `DR_LUNAR_AAAA-MM-JJ_HH-MM-SS` rend le
/// tri lexical chronologique. Best-effort : une erreur n'empêche pas le
/// démarrage. Rend les fichiers supprimés.
pub fn prune_old_logs(log_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(log_dir) else {
        return Vec::new();
    };
    let mut logs: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with(SESSION_LOG_PREFIX))
        })
        .collect();
    if logs.len() <= MAX_SESSION_LOGS {
        return Vec::new();
    }
    logs.sort();
    let excess = logs.len() - MAX_SESSION_LOGS;
    let mut removed = Vec::new();
    for path in logs.into_iter().take(excess) {
        match fs::remove_file(&path) {
            Ok(()) => {
                info!("Rétention logs : {} supprimé", path.display());
                removed.push(path);
            }
            Err(e) => warn!(
                "Rétention logs : suppression de {} impossible : {e}",
                path.display()
            ),
        }
    }
    removed
}

/// Journal : filtre, sortie standard et fichiers.
struct LunarLogger {
    /// Filtre `RUST_LOG`.
    filter: env_filter::Filter,
    /// Journaux ouverts en ajout.
    files: Vec<Mutex<File>>,
}

impl Log for LunarLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        self.filter.enabled(metadata)
    }

    fn log(&self, record: &Record<'_>) {
        if !self.filter.matches(record) {
            return;
        }
        let mut line = format_line(
            Utc::now().naive_utc(),
            record.target(),
            record.level(),
            record.args(),
        );
        line.push('\n');
        // Sortie standard fermée (binaire Windows sans console) : ignoré.
        let _ = io::stdout().lock().write_all(line.as_bytes());
        for file in &self.files {
            if let Ok(mut file) = file.lock() {
                let _ = file.write_all(line.as_bytes());
            }
        }
    }

    fn flush(&self) {
        let _ = io::stdout().lock().flush();
        for file in &self.files {
            if let Ok(mut file) = file.lock() {
                let _ = file.flush();
            }
        }
    }
}

/// Ouvre (en ajout) `<dir>/<name>.log` après la rotation de démarrage.
fn open_log_file(dir: &Path, name: &str) -> io::Result<File> {
    rotate_on_startup(
        dir,
        name,
        MAX_FILE_SIZE,
        KEEP_ROTATED,
        Utc::now().naive_utc(),
    )?;
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(format!("{name}.log")))
}

/// Installe le journal. `log_dir` à `None` : sortie standard seule (smoke
/// test). Un répertoire inutilisable n'empêche pas le démarrage : le
/// journal se replie sur la sortie standard et le signale.
pub fn init(log_dir: Option<&Path>) {
    let spec = std::env::var("RUST_LOG")
        .ok()
        .filter(|spec| !spec.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_FILTER.to_owned());
    let filter = env_filter::Builder::new().parse(&spec).build();
    let mut files = Vec::new();
    let mut failures = Vec::new();
    if let Some(dir) = log_dir {
        match fs::create_dir_all(dir) {
            Ok(()) => {
                let session = session_log_name(Local::now().naive_local());
                for name in [MAIN_LOG_NAME, session.as_str()] {
                    match open_log_file(dir, name) {
                        Ok(file) => files.push(Mutex::new(file)),
                        Err(err) => failures.push(format!("{name}.log : {err}")),
                    }
                }
            }
            Err(err) => failures.push(format!("{} : {err}", dir.display())),
        }
    }
    let max_level = filter.filter();
    let logger = LunarLogger { filter, files };
    if log::set_boxed_logger(Box::new(logger)).is_ok() {
        log::set_max_level(max_level);
    }
    for failure in failures {
        warn!("Journal fichier indisponible ({failure}) : sortie standard seule");
    }
    if let Some(dir) = log_dir {
        info!("Journaux : {}", dir.display());
        prune_old_logs(dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn at(h: u32, mi: u32, s: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 11, 19)
            .and_then(|d| d.and_hms_opt(h, mi, s))
            .expect("date valide")
    }

    #[test]
    fn ligne_au_format_attendu() {
        let line = format_line(
            at(9, 5, 7),
            "lunar::app::state",
            Level::Info,
            &format_args!("Capture démarrée : {}", true),
        );
        assert_eq!(
            line,
            "[2026-11-19][09:05:07][lunar::app::state][INFO] Capture démarrée : true"
        );
    }

    #[test]
    fn nom_du_journal_de_session() {
        assert_eq!(
            session_log_name(at(14, 3, 12)),
            "DR_LUNAR_2026-11-19_14-03-12"
        );
    }

    #[test]
    fn petit_fichier_non_tourne() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("lunar.log"), "court").expect("écriture");
        rotate_on_startup(dir.path(), "lunar", 10, 5, at(1, 2, 3)).expect("rotation");
        assert!(dir.path().join("lunar.log").exists());
    }

    #[test]
    fn gros_fichier_tourne_et_tournes_bornes() {
        let dir = tempfile::tempdir().expect("tempdir");
        for day in 10..16 {
            fs::write(
                dir.path().join(format!("lunar_2026-11-{day}_00-00-00.log")),
                "ancien",
            )
            .expect("écriture");
        }
        fs::write(dir.path().join("lunar.log"), "x".repeat(20)).expect("écriture");
        rotate_on_startup(dir.path(), "lunar", 10, 5, at(1, 2, 3)).expect("rotation");
        assert!(!dir.path().join("lunar.log").exists());
        assert!(dir.path().join("lunar_2026-11-19_01-02-03.log").exists());
        let mut names: Vec<String> = fs::read_dir(dir.path())
            .expect("lecture")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        // 4 anciens gardés + le nouveau tourné : 5 au total.
        assert_eq!(
            names,
            [
                "lunar_2026-11-12_00-00-00.log",
                "lunar_2026-11-13_00-00-00.log",
                "lunar_2026-11-14_00-00-00.log",
                "lunar_2026-11-15_00-00-00.log",
                "lunar_2026-11-19_01-02-03.log",
            ]
        );
    }

    #[test]
    fn la_retention_ne_touche_pas_aux_autres_noms() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("lunar.log"), "").expect("écriture");
        fs::write(dir.path().join("autre_2026-01-01_00-00-00.log"), "").expect("écriture");
        for i in 0..(MAX_SESSION_LOGS + 3) {
            fs::write(
                dir.path()
                    .join(format!("DR_LUNAR_2026-11-{:02}_00-00-00.log", i + 1)),
                "",
            )
            .expect("écriture");
        }
        let removed = prune_old_logs(dir.path());
        let removed: Vec<String> = removed
            .iter()
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect();
        assert_eq!(
            removed,
            [
                "DR_LUNAR_2026-11-01_00-00-00.log",
                "DR_LUNAR_2026-11-02_00-00-00.log",
                "DR_LUNAR_2026-11-03_00-00-00.log",
            ]
        );
        assert!(dir.path().join("lunar.log").exists());
        assert!(dir.path().join("autre_2026-01-01_00-00-00.log").exists());
        assert!(dir.path().join("DR_LUNAR_2026-11-33_00-00-00.log").exists());
    }

    #[test]
    fn retention_sous_le_plafond() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("DR_LUNAR_2026-11-01_00-00-00.log"), "").expect("écriture");
        assert!(prune_old_logs(dir.path()).is_empty());
        assert!(prune_old_logs(&dir.path().join("absent")).is_empty());
    }
}
