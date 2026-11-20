//! Instrumentation optionnelle des imports PCAP (feature `capture_timing`) :
//! échantillonnage des paquets et journal JSONL des durées par étape.

#[cfg(feature = "capture_timing")]
use log::{error, info};
#[cfg(feature = "capture_timing")]
use std::path::PathBuf;
#[cfg(feature = "capture_timing")]
use std::time::Instant;
#[cfg(feature = "capture_timing")]
use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Write},
    time::{SystemTime, UNIX_EPOCH},
};

/// Paquet retenu par l'échantillonnage de l'import : rang et taux en
/// vigueur, recopiés dans l'entrée `import_packet_timing` correspondante.
#[cfg(feature = "capture_timing")]
#[derive(Clone, Copy)]
pub(super) struct ImportTimingSample {
    /// Rang du paquet parmi ceux passés au traitement (à partir de 1).
    pub(super) seq: u64,
    /// Taux d'échantillonnage en vigueur : un paquet mesuré sur `sample_rate`.
    pub(super) sample_rate: u64,
}

/// Journal JSONL des mesures d'un import PCAP : entrées
/// `import_packet_timing` (paquets échantillonnés), `import_parse_error_timing`,
/// `import_file_timing` et `import_snapshot_timing`, construites par
/// l'appelant en `serde_json::Value`. Même fichier que le journal de capture
/// live (`import_timing_log_path`), ouvert en ajout ; abandonné par
/// `write_timing_or_disable` à la première erreur d'écriture.
#[cfg(feature = "capture_timing")]
pub(super) struct ImportTimingLogger {
    /// Fichier JSONL ouvert en ajout, tamponné ; vidé par `write_value` à sa cadence.
    writer: BufWriter<File>,
    /// Un paquet mesuré sur `sample_rate` ; `LUNAR_IMPORT_TIMING_SAMPLE_RATE`,
    /// sinon `LUNAR_CAPTURE_TIMING_SAMPLE_RATE`, 1 par défaut (tous les paquets).
    sample_rate: u64,
    /// Paquets passés par `next_sample` depuis l'ouverture, échantillonnés ou non.
    packet_seen: u64,
    /// Entrées écrites depuis le dernier vidage du tampon ; vidage forcé à 256.
    pending_flush: u64,
    /// Instant du dernier vidage du tampon ; vidage forcé après 1 s.
    last_flush: Instant,
}

/// Sans la feature `capture_timing`, le journal est le type unité : les
/// appelants manipulent un `Option<()>` toujours `None`.
#[cfg(not(feature = "capture_timing"))]
pub(super) type ImportTimingLogger = ();

#[cfg(feature = "capture_timing")]
impl ImportTimingLogger {
    /// Ouvre (ou crée) le fichier de journal en ajout, crée son répertoire au
    /// besoin et lit le taux d'échantillonnage dans
    /// `LUNAR_IMPORT_TIMING_SAMPLE_RATE`, sinon `LUNAR_CAPTURE_TIMING_SAMPLE_RATE`
    /// (entier > 0, 1 par défaut). Journalise le chemin retenu en `info`.
    pub(super) fn new() -> io::Result<Self> {
        let path = import_timing_log_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let sample_rate = std::env::var("LUNAR_IMPORT_TIMING_SAMPLE_RATE")
            .or_else(|_| std::env::var("LUNAR_CAPTURE_TIMING_SAMPLE_RATE"))
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(1);

        info!(
            "Import timing log enabled: path={} sample_rate={}",
            path.display(),
            sample_rate
        );

        Ok(Self {
            writer: BufWriter::new(file),
            sample_rate,
            packet_seen: 0,
            pending_flush: 0,
            last_flush: Instant::now(),
        })
    }

    /// Compte un paquet et dit s'il est échantillonné : `Some` tous les
    /// `sample_rate` paquets, avec son rang.
    pub(super) fn next_sample(&mut self) -> Option<ImportTimingSample> {
        self.packet_seen = self.packet_seen.saturating_add(1);
        if !self.packet_seen.is_multiple_of(self.sample_rate) {
            return None;
        }

        Some(ImportTimingSample {
            seq: self.packet_seen,
            sample_rate: self.sample_rate,
        })
    }

    /// Sérialise `value` sur une ligne du journal, puis vide le tampon toutes
    /// les 256 entrées ou au plus tard 1 s après le vidage précédent. Une
    /// erreur de sérialisation est rendue comme erreur d'E/S.
    pub(super) fn write_value(&mut self, value: serde_json::Value) -> io::Result<()> {
        serde_json::to_writer(&mut self.writer, &value).map_err(io::Error::other)?;
        self.writer.write_all(b"\n")?;

        self.pending_flush = self.pending_flush.saturating_add(1);
        if self.pending_flush >= 256
            || self.last_flush.elapsed() >= std::time::Duration::from_secs(1)
        {
            self.writer.flush()?;
            self.pending_flush = 0;
            self.last_flush = Instant::now();
        }

        Ok(())
    }
}

/// Écrit une entrée dans le journal ; en cas d'échec d'écriture, le journal
/// est désactivé (mis à `None`) pour ne pas répéter l'erreur à chaque paquet.
#[cfg(feature = "capture_timing")]
pub(super) fn write_timing_or_disable(
    timing_logger: &mut Option<ImportTimingLogger>,
    value: serde_json::Value,
    context: &str,
) {
    if let Some(logger) = timing_logger.as_mut()
        && let Err(e) = logger.write_value(value)
    {
        error!("Import timing log disabled after {context} write error: {e}");
        *timing_logger = None;
    }
}

/// Chemin du journal : `LUNAR_CAPTURE_TIMING_LOG` s'il est défini, sinon
/// `capture-timing-{pid}.jsonl` dans le répertoire de logs de l'application
/// (`$XDG_DATA_HOME/fr.lunar/logs` sous Linux,
/// `%LOCALAPPDATA%\fr.lunar\logs` sous Windows,
/// `~/Library/Logs/fr.lunar` sous macOS). Même règle que la capture live.
/// L'identifiant est [`crate::paths::DEFAULT_APP_ID`].
#[cfg(feature = "capture_timing")]
fn import_timing_log_path() -> PathBuf {
    if let Ok(path) = std::env::var("LUNAR_CAPTURE_TIMING_LOG") {
        return PathBuf::from(path);
    }

    let file_name = format!("capture-timing-{}.jsonl", std::process::id());

    #[cfg(target_os = "linux")]
    {
        let base = std::env::var("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("/"))
                    .join(".local/share")
            });
        base.join("fr.lunar/logs").join(file_name)
    }

    #[cfg(target_os = "windows")]
    {
        return dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("C:\\Users\\Default\\AppData\\Local"))
            .join("fr.lunar\\logs")
            .join(file_name);
    }

    #[cfg(target_os = "macos")]
    {
        return dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/Users/Shared"))
            .join("Library/Logs/fr.lunar")
            .join(file_name);
    }
}

/// Durée écoulée depuis `start`, en ns (tronquée à `u64`).
#[cfg(feature = "capture_timing")]
pub(super) fn elapsed_ns_since(start: Instant) -> u64 {
    start.elapsed().as_nanos() as u64
}

/// Horodatage Unix courant en ns pour le champ `ts_unix_ns` des entrées ;
/// 0 si l'horloge système est antérieure à l'époque.
#[cfg(feature = "capture_timing")]
pub(super) fn now_unix_ns() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default()
}
