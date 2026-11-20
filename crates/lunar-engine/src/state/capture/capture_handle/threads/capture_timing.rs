//! Instrumentation de la capture live (feature `capture_timing`) :
//! échantillonnage des paquets, journal JSONL des durées par étape du
//! pipeline, timing IPC des batches et résumé de fin de run.

use log::info;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Write},
    path::PathBuf,
    sync::atomic::Ordering,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use packet_parser::{LinkType, PacketFlow, timing::ParseTiming};

use super::packet_buffer::PacketBufferPool;
use super::processing::{PACKET_BATCH_INTERVAL_MS, PACKET_BATCH_MAX};

/// Rang de la prochaine session de capture instrumentée dans ce processus ;
/// donne le suffixe `-runNN` de l'identifiant de run (`capture_timing_run_id`).
static CAPTURE_TIMING_RUN_COUNTER: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);

/// Parse un paquet avec le décodeur chronométré de `packet_parser` et rend
/// le flux avec la durée de chaque couche (L2/L3/L4/L7 et total, en ns).
/// Le thread de traitement ne l'emploie que pour les paquets échantillonnés.
pub(super) fn parse_packet_flow_with_timing(
    link_type: LinkType,
    bytes: &[u8],
) -> Result<(PacketFlow<'_>, ParseTiming), packet_parser::ParseError> {
    let mut timing = ParseTiming::default();
    let flow = packet_parser::parse::parse_timed(link_type, bytes, &mut timing)?;
    Ok((flow, timing))
}

/// Paquet retenu par l'échantillonnage : rang et taux en vigueur, recopiés
/// dans l'entrée `capture_pipeline_timing` correspondante.
#[derive(Clone, Copy)]
pub(super) struct CaptureTimingSample {
    /// Rang du paquet parmi tous ceux passés au thread de traitement (à partir de 1).
    pub(super) seq: u64,
    /// Taux d'échantillonnage en vigueur : un paquet mesuré sur `sample_rate`.
    pub(super) sample_rate: u64,
}

/// Durées d'un paquet échantillonné à chaque étape du pipeline de traitement,
/// toutes en ns ; sérialisées par `CaptureTimingLogger::write_pipeline` dans
/// une entrée `capture_pipeline_timing` du journal JSONL.
#[derive(Default)]
pub(super) struct CapturePipelineTiming {
    /// Octets capturés du paquet (en-tête pcap).
    pub(super) caplen: u32,
    /// Longueur du paquet sur le fil, en octets (en-tête pcap).
    pub(super) len: u32,
    /// Décodage de la couche liaison (ns), mesuré par `packet_parser`.
    pub(super) parse_l2_ns: u64,
    /// Décodage de la couche réseau (ns).
    pub(super) parse_l3_ns: u64,
    /// Décodage de la couche transport (ns).
    pub(super) parse_l4_ns: u64,
    /// Décodage de la couche applicative (ns).
    pub(super) parse_l7_ns: u64,
    /// Parsing complet du paquet (ns), toutes couches confondues.
    pub(super) parse_total_ns: u64,
    /// Conversion du paquet parsé en paquets possédés, un par niveau de tunnel (ns).
    pub(super) packet_owned_ns: u64,
    /// Recherche des étiquettes source et destination dans la matrice (ns),
    /// cumulée sur les niveaux de tunnel.
    pub(super) label_lookup_ns: u64,
    /// Intégration des flux dans la matrice (ns), cumulée sur les niveaux de tunnel.
    pub(super) matrix_update_ns: u64,
    /// Intégration dans le graphe (ns), cumulée sur les niveaux de tunnel.
    pub(super) graph_update_ns: u64,
    /// Mise en batch des updates graphe et, si `GRAPH_BATCH_MAX` est atteint,
    /// envoi IPC de ce batch (ns).
    pub(super) graph_ipc_ns: u64,
    /// Nombre d'updates graphe produites par ce paquet, tous niveaux confondus.
    pub(super) graph_updates: usize,
    /// Envois IPC du batch graphe en échec pour ce paquet. Toujours 0 : un
    /// échec arrête le thread de traitement avant l'écriture de l'entrée.
    pub(super) graph_ipc_failures: usize,
    /// Traitement complet du paquet (ns), du parsing à la mise en batch graphe.
    pub(super) pipeline_total_ns: u64,
}

/// Journal JSONL des mesures d'une session de capture live : une entrée
/// `capture_pipeline_timing` par paquet échantillonné, une entrée
/// `capture_packet_batch_ipc_timing` par batch de paquets envoyé au front et
/// une entrée `capture_run_summary` à l'arrêt. Fichier donné par
/// `capture_timing_log_path`, ouvert en ajout. Détenu par le `PacketWorker`
/// du thread de traitement, qui l'abandonne à la première erreur d'écriture.
pub(super) struct CaptureTimingLogger {
    /// Fichier JSONL ouvert en ajout, tamponné ; vidé par `flush_if_due`.
    writer: BufWriter<File>,
    /// Identifiant de la session, recopié dans chaque entrée (`capture_timing_run_id`).
    run_id: String,
    /// Un paquet mesuré sur `sample_rate` ; `LUNAR_CAPTURE_TIMING_SAMPLE_RATE`, 100 par défaut.
    sample_rate: u64,
    /// Paquets passés par `next_sample` depuis l'ouverture, échantillonnés ou non.
    seen: u64,
    /// Batches de paquets envoyés au front depuis l'ouverture (`batch_seq` des entrées).
    batch_seen: u64,
    /// Horodatage Unix (ns) du premier batch envoyé ; borne basse de la durée
    /// active du résumé.
    batch_first_ts_unix_ns: Option<u128>,
    /// Horodatage Unix (ns) du dernier batch envoyé ; borne haute de la durée
    /// active du résumé.
    batch_last_ts_unix_ns: Option<u128>,
    /// Paquets cumulés de tous les batches envoyés (`packet_total` du résumé).
    batch_packet_total: u64,
    /// Batches partis pleins, à `PACKET_BATCH_MAX` paquets (`full_batch_count` du résumé).
    batch_full_total: u64,
    /// Somme des durées d'envoi IPC des batches (ns), pour la moyenne du résumé.
    batch_ipc_total_ns: u128,
    /// Durée d'envoi IPC de chaque batch (ns), triée à l'arrêt pour les p95 et p99 du résumé.
    batch_ipc_values: Vec<u64>,
    /// Vrai une fois l'entrée `capture_run_summary` écrite : un second appel
    /// ne fait que vider le tampon.
    summary_written: bool,
    /// Entrées écrites depuis le dernier vidage du tampon ; vidage forcé à 256.
    pending_flush: u64,
    /// Instant du dernier vidage du tampon ; vidage forcé après 1 s.
    last_flush: Instant,
}

impl CaptureTimingLogger {
    /// Ouvre (ou crée) le fichier de journal en ajout, crée son répertoire au
    /// besoin, lit le taux d'échantillonnage dans
    /// `LUNAR_CAPTURE_TIMING_SAMPLE_RATE` (entier > 0, 100 par défaut) et tire
    /// l'identifiant de run. Journalise le chemin retenu en `info`.
    pub(super) fn new() -> io::Result<Self> {
        let path = capture_timing_log_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let sample_rate = std::env::var("LUNAR_CAPTURE_TIMING_SAMPLE_RATE")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(100);
        let run_id = capture_timing_run_id();

        info!(
            "Capture timing log enabled: path={} run_id={} sample_rate={}",
            path.display(),
            run_id,
            sample_rate
        );

        Ok(Self {
            writer: BufWriter::new(file),
            run_id,
            sample_rate,
            seen: 0,
            batch_seen: 0,
            batch_first_ts_unix_ns: None,
            batch_last_ts_unix_ns: None,
            batch_packet_total: 0,
            batch_full_total: 0,
            batch_ipc_total_ns: 0,
            batch_ipc_values: Vec::new(),
            summary_written: false,
            pending_flush: 0,
            last_flush: Instant::now(),
        })
    }

    /// Compte un paquet et dit s'il est échantillonné : `Some` tous les
    /// `sample_rate` paquets, avec son rang. Appelé pour chaque paquet avant
    /// le parsing, pour ne chronométrer que les paquets retenus.
    pub(super) fn next_sample(&mut self) -> Option<CaptureTimingSample> {
        self.seen = self.seen.saturating_add(1);
        if !self.seen.is_multiple_of(self.sample_rate) {
            return None;
        }

        Some(CaptureTimingSample {
            seq: self.seen,
            sample_rate: self.sample_rate,
        })
    }

    /// Écrit l'entrée `capture_pipeline_timing` d'un paquet échantillonné
    /// (horodatage Unix en ns, identifiant de run, rang, taux et toutes les
    /// durées de `timing`), puis vide le tampon s'il est dû.
    pub(super) fn write_pipeline(
        &mut self,
        sample: CaptureTimingSample,
        timing: CapturePipelineTiming,
    ) -> io::Result<()> {
        let ts_unix_ns = unix_now_ns();

        writeln!(
            self.writer,
            "{{\"event\":\"capture_pipeline_timing\",\"ts_unix_ns\":{},\"run_id\":\"{}\",\"seq\":{},\"sample_rate\":{},\"caplen\":{},\"len\":{},\"parse_l2_ns\":{},\"parse_l3_ns\":{},\"parse_l4_ns\":{},\"parse_l7_ns\":{},\"parse_total_ns\":{},\"packet_owned_ns\":{},\"label_lookup_ns\":{},\"matrix_update_ns\":{},\"graph_update_ns\":{},\"graph_ipc_ns\":{},\"graph_updates\":{},\"graph_ipc_failures\":{},\"pipeline_total_ns\":{}}}",
            ts_unix_ns,
            self.run_id,
            sample.seq,
            sample.sample_rate,
            timing.caplen,
            timing.len,
            timing.parse_l2_ns,
            timing.parse_l3_ns,
            timing.parse_l4_ns,
            timing.parse_l7_ns,
            timing.parse_total_ns,
            timing.packet_owned_ns,
            timing.label_lookup_ns,
            timing.matrix_update_ns,
            timing.graph_update_ns,
            timing.graph_ipc_ns,
            timing.graph_updates,
            timing.graph_ipc_failures,
            timing.pipeline_total_ns
        )?;

        self.flush_if_due()
    }

    /// Écrit l'entrée `capture_packet_batch_ipc_timing` d'un batch de paquets
    /// envoyé au front : nombre de paquets, seuils `PACKET_BATCH_MAX` et
    /// `PACKET_BATCH_INTERVAL_MS`, batch plein ou non, durée de l'envoi IPC
    /// (ns) et succès. Met à jour les cumuls du résumé d'arrêt.
    pub(super) fn write_packet_batch_ipc(
        &mut self,
        batch_len: usize,
        ipc_ns: u64,
        ok: bool,
    ) -> io::Result<()> {
        self.batch_seen = self.batch_seen.saturating_add(1);
        let ts_unix_ns = unix_now_ns();
        let batch_full = usize::from(batch_len >= PACKET_BATCH_MAX);

        self.batch_first_ts_unix_ns.get_or_insert(ts_unix_ns);
        self.batch_last_ts_unix_ns = Some(ts_unix_ns);
        self.batch_packet_total = self
            .batch_packet_total
            .saturating_add(batch_len.try_into().unwrap_or(u64::MAX));
        self.batch_full_total = self
            .batch_full_total
            .saturating_add(batch_full.try_into().unwrap_or(0));
        self.batch_ipc_total_ns = self.batch_ipc_total_ns.saturating_add(ipc_ns as u128);
        self.batch_ipc_values.push(ipc_ns);

        writeln!(
            self.writer,
            "{{\"event\":\"capture_packet_batch_ipc_timing\",\"ts_unix_ns\":{},\"run_id\":\"{}\",\"batch_seq\":{},\"batch_len\":{},\"batch_max\":{},\"batch_interval_ms\":{},\"batch_full\":{},\"ipc_ns\":{},\"ok\":{}}}",
            ts_unix_ns,
            self.run_id,
            self.batch_seen,
            batch_len,
            PACKET_BATCH_MAX,
            PACKET_BATCH_INTERVAL_MS,
            batch_full,
            ipc_ns,
            ok
        )?;

        self.flush_if_due()
    }

    /// Compte une entrée écrite et vide le tampon toutes les 256 entrées ou
    /// au plus tard 1 s après le vidage précédent : borne la perte en cas
    /// d'arrêt brutal sans payer un `flush` par entrée.
    fn flush_if_due(&mut self) -> io::Result<()> {
        self.pending_flush = self.pending_flush.saturating_add(1);
        if self.pending_flush >= 256 || self.last_flush.elapsed() >= Duration::from_secs(1) {
            self.writer.flush()?;
            self.pending_flush = 0;
            self.last_flush = Instant::now();
        }

        Ok(())
    }

    /// Écrit l'entrée `capture_run_summary` et vide le tampon : paquets et
    /// batches envoyés, batches pleins, débit moyen (paquets/s entre le
    /// premier et le dernier batch), durée IPC moyenne, p95 et p99 des
    /// batches (ns) et état du pool de tampons. Idempotent : un second appel
    /// ne fait que vider le tampon.
    pub(super) fn write_run_summary(&mut self, buffer_pool: &PacketBufferPool) -> io::Result<()> {
        if self.summary_written {
            return self.writer.flush();
        }

        let ts_unix_ns = unix_now_ns();
        let batch_count = self.batch_seen;
        let active_duration_ns = match (self.batch_first_ts_unix_ns, self.batch_last_ts_unix_ns) {
            (Some(first), Some(last)) => last.saturating_sub(first) as u64,
            _ => 0,
        };
        let avg_packets_per_second = if active_duration_ns > 0 {
            (self.batch_packet_total as f64 * 1_000_000_000f64) / active_duration_ns as f64
        } else {
            0.0
        };
        let packet_batch_ipc_avg_ns = if batch_count > 0 {
            (self.batch_ipc_total_ns / batch_count as u128) as u64
        } else {
            0
        };
        let mut sorted_ipc = self.batch_ipc_values.clone();
        sorted_ipc.sort_unstable();
        let packet_batch_ipc_p95_ns = percentile_ns(&sorted_ipc, 0.95);
        let packet_batch_ipc_p99_ns = percentile_ns(&sorted_ipc, 0.99);

        let pool_stats = buffer_pool.stats();
        writeln!(
            self.writer,
            "{{\"event\":\"capture_run_summary\",\"ts_unix_ns\":{},\"run_id\":\"{}\",\"packet_total\":{},\"avg_packets_per_second\":{:.3},\"batch_count\":{},\"batch_max\":{},\"batch_interval_ms\":{},\"full_batch_count\":{},\"packet_batch_ipc_avg_ns\":{},\"packet_batch_ipc_p95_ns\":{},\"packet_batch_ipc_p99_ns\":{},\"active_duration_ns\":{},\"pool_small_allocated\":{},\"pool_large_allocated\":{},\"pool_allocated_bytes\":{},\"pool_exhausted\":{}}}",
            ts_unix_ns,
            self.run_id,
            self.batch_packet_total,
            avg_packets_per_second,
            batch_count,
            PACKET_BATCH_MAX,
            PACKET_BATCH_INTERVAL_MS,
            self.batch_full_total,
            packet_batch_ipc_avg_ns,
            packet_batch_ipc_p95_ns,
            packet_batch_ipc_p99_ns,
            active_duration_ns,
            pool_stats.small_allocated,
            pool_stats.large_allocated,
            buffer_pool.allocated_bytes(),
            pool_stats.exhausted
        )?;
        self.summary_written = true;
        self.writer.flush()
    }
}

/// Durée écoulée depuis `start`, en ns (tronquée à `u64`).
pub(super) fn elapsed_ns_since(start: Instant) -> u64 {
    start.elapsed().as_nanos() as u64
}

/// Horodatage Unix courant en ns pour le champ `ts_unix_ns` des entrées ;
/// 0 si l'horloge système est antérieure à l'époque.
fn unix_now_ns() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default()
}

/// Identifiant de run inscrit dans chaque entrée : préfixe
/// `LUNAR_CAPTURE_TIMING_RUN_ID` (sinon `capture-{pid}-{horodatage ns}`),
/// réduit aux caractères `[A-Za-z0-9-_.]`, suivi de `-runNN` où NN est le
/// rang de la session dans le processus (`CAPTURE_TIMING_RUN_COUNTER`).
fn capture_timing_run_id() -> String {
    let run_index = CAPTURE_TIMING_RUN_COUNTER.fetch_add(1, Ordering::Relaxed);
    let raw_prefix = std::env::var("LUNAR_CAPTURE_TIMING_RUN_ID")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| format!("capture-{}-{}", std::process::id(), unix_now_ns()));

    let sanitized_prefix: String = raw_prefix
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();

    format!("{sanitized_prefix}-run{run_index:02}")
}

/// Valeur au quantile `quantile` (0,95 pour le p95) d'une série déjà triée,
/// par rang arrondi au supérieur ; 0 sur série vide.
fn percentile_ns(sorted_values: &[u64], quantile: f64) -> u64 {
    if sorted_values.is_empty() {
        return 0;
    }

    let last_index = sorted_values.len() - 1;
    let index = (last_index as f64 * quantile).ceil() as usize;
    sorted_values[index.min(last_index)]
}

/// Chemin du journal : `LUNAR_CAPTURE_TIMING_LOG` s'il est défini, sinon
/// `capture-timing-{pid}.jsonl` dans le répertoire de logs de l'application
/// (`$XDG_DATA_HOME/fr.lunar/logs` sous Linux,
/// `%LOCALAPPDATA%\fr.lunar\logs` sous Windows,
/// `~/Library/Logs/fr.lunar` sous macOS). Même fichier que le journal
/// d'import. L'identifiant est [`crate::paths::DEFAULT_APP_ID`].
fn capture_timing_log_path() -> PathBuf {
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
        dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("C:\\Users\\Default\\AppData\\Local"))
            .join("fr.lunar\\logs")
            .join(file_name)
    }

    #[cfg(target_os = "macos")]
    {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/Users/Shared"))
            .join("Library/Logs/fr.lunar")
            .join(file_name)
    }
}
