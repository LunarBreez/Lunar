//! Projet LUNAR (`.lunar`) : un relevé complet dans un seul fichier.
//!
//! Le format est une archive ZIP contenant des entrées à noms fixes :
//! - `manifest.json` — version de schéma, version de l'application, DLT et
//!   comptages, pour valider avant toute mutation et migrer plus tard ;
//! - `matrice.csv` — la matrice au format LFMS (même writer
//!   déterministe que l'export CSV, préambule `#LFMS` inclus) ;
//! - `labels.csv` — les labels résolus (même writer que l'export labels) ;
//! - `capture.json` — configuration de capture et filtre BPF de la session.
//!
//! L'ouverture ne réécrit aucune logique d'état : elle compose les chemins
//! d'import existants (`import_matrix_files` pour le swap transactionnel
//! matrice+graphe, `import_label_file` pour le remplacement du store de
//! labels et la resynchronisation), qui portent déjà l'exclusion mutuelle
//! avec la capture et l'émission des événements vers l'interface.
//!
//! La version d'application inscrite au manifest est celle de ce moteur
//! (`CARGO_PKG_VERSION`). L'autosave et la sentinelle de session démarrent
//! par [`SessionPersistence::start`] et s'arrêtent par
//! [`SessionPersistence::stop`].

use std::{
    fs::{self, File},
    io::{self, ErrorKind, Read, Write, copy},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use crossbeam::channel::{RecvTimeoutError, Sender, bounded};
use log::{info, warn};
use serde::{Deserialize, Serialize};
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

use crate::ipc::Channel;

use crate::{
    errors::{CaptureStateError, export::ExportError, import::PcapImportError},
    events::CaptureEvent,
    state::{
        capture::{CaptureState, capture_config::CaptureConfig},
        flow_matrix::FlowMatrix,
        graph::GraphData,
        labels_list::{LabelStore, PcInfoLabel},
    },
};

/// Version du schéma de projet. À incrémenter à chaque évolution du contenu
/// de l'archive, avec un test de migration depuis chaque version antérieure.
/// v2 : contexte de relevé (site, capteur, interface).
pub const PROJECT_SCHEMA_VERSION: u32 = 2;

/// Nom de l'entrée du manifest JSON dans l'archive.
const MANIFEST_ENTRY: &str = "manifest.json";
/// Nom de l'entrée de la matrice CSV (format LFMS) dans l'archive.
const MATRIX_ENTRY: &str = "matrice.csv";
/// Nom de l'entrée des labels CSV ; toujours écrite, mais tolérée absente à l'ouverture.
const LABELS_ENTRY: &str = "labels.csv";
/// Nom de l'entrée des réglages de session (JSON) ; ignorée si absente ou invalide à l'ouverture.
const CAPTURE_ENTRY: &str = "capture.json";

/// Taille décompressée maximale d'une entrée JSON (manifest, réglages de
/// session), qui pèse quelques Ko en pratique.
const MAX_JSON_ENTRY_BYTES: u64 = 1024 * 1024;
/// Taille décompressée maximale de la matrice et des labels. Une ligne de
/// matrice pèse environ 150 octets : le plafond de la capture live
/// (250 000 flux) tient dans 40 Mo, et 256 Mio laissent la place d'un
/// relevé importé environ six fois plus grand. Sans borne, une archive de
/// quelques Mo pouvait se décompresser en plusieurs Go.
const MAX_CSV_ENTRY_BYTES: u64 = 256 * 1024 * 1024;
/// Nombre de noms essayés pour un dossier temporaire avant d'abandonner.
const TEMP_DIR_ATTEMPTS: u32 = 16;

/// Miroir sérialisable de [`lunar_flows_core::lfms::SurveyContext`], qui ne
/// dérive pas `serde` (le cœur écrit le préambule LFMS lui-même, il n'a pas
/// besoin de JSON). Le manifest, lui, est du JSON.
///
/// La conversion depuis le type du cœur est **exhaustive par
/// destructuration** : ajouter un champ à
/// `SurveyContext` casse la compilation ici au lieu de le perdre en silence.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct ManifestSurveyContext {
    /// Site du relevé.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site: Option<String>,
    /// Capteur (poste de relevé).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensor: Option<String>,
    /// Interface réseau écoutée.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interface: Option<String>,
}

impl From<&lunar_flows_core::lfms::SurveyContext> for ManifestSurveyContext {
    fn from(context: &lunar_flows_core::lfms::SurveyContext) -> Self {
        // Destructuration exhaustive volontaire : voir le commentaire du type.
        let lunar_flows_core::lfms::SurveyContext {
            site,
            sensor,
            interface,
        } = context;
        Self {
            site: site.clone(),
            sensor: sensor.clone(),
            interface: interface.clone(),
        }
    }
}

impl From<&ManifestSurveyContext> for lunar_flows_core::lfms::SurveyContext {
    fn from(context: &ManifestSurveyContext) -> Self {
        let ManifestSurveyContext {
            site,
            sensor,
            interface,
        } = context;
        Self {
            site: site.clone(),
            sensor: sensor.clone(),
            interface: interface.clone(),
        }
    }
}

/// Carte d'identité du projet, écrite en tête d'archive et validée avant
/// toute mutation de l'état à l'ouverture.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ProjectManifest {
    /// Version du schéma de l'archive, comparée à `PROJECT_SCHEMA_VERSION` à l'ouverture.
    pub schema_version: u32,
    /// Version de LUNAR qui a écrit le projet (information, pas un critère de refus).
    pub app_version: String,
    /// Version du format LFMS de la matrice embarquée (`lunar_flows_core::lfms::LFMS_VERSION`).
    pub lfms_version: String,
    /// Nom du DLT du relevé (`None` : relevé vide, DLT pas encore fixé).
    pub dlt: Option<String>,
    /// Nombre de lignes de la matrice embarquée (journalisé, repris par l'autosave).
    pub matrix_rows: usize,
    /// Nombre de lignes de labels embarquées.
    pub label_rows: usize,
    /// Date d'enregistrement, en secondes depuis l'epoch Unix (UTC).
    pub saved_at_epoch_secs: u64,
    /// Contexte du relevé (schéma v2). `#[serde(default)]` : un
    /// manifest v1 ne porte pas la clé et se lit avec un contexte vide —
    /// c'est toute la migration v1 → v2, aucune donnée n'est inventée.
    #[serde(default)]
    pub survey_context: ManifestSurveyContext,
}

/// Réglages de session embarqués dans le projet. `CaptureConfig` est
/// revalidée à l'ouverture (bornes `MIN_*`/`MAX_*`) : un projet édité à la
/// main ne peut pas injecter une configuration hors bornes. Le filtre, lui,
/// n'est qu'une trace : il n'est pas réappliqué à l'ouverture.
#[derive(Serialize, Deserialize, Debug, Clone)]
struct ProjectCaptureSettings {
    /// Configuration de capture de la session, revalidée à l'ouverture.
    config: CaptureConfig,
    /// Filtre BPF de la session à l'enregistrement, `None` si aucun. Écrit
    /// pour mémoire, jamais restauré dans la session à l'ouverture.
    filter: Option<String>,
}

/// Version d'application inscrite dans le manifest des projets écrits ici.
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Enregistre le relevé courant comme projet `.lunar` à `path`.
pub fn save_project(
    matrice: &Arc<Mutex<FlowMatrix>>,
    capture_state: &Arc<Mutex<CaptureState>>,
    path: String,
) -> Result<(), CaptureStateError> {
    if path.trim().is_empty() {
        return Err(CaptureStateError::Export(ExportError::EmptyPath));
    }

    let (manifest, revision) =
        write_snapshot(APP_VERSION, matrice, capture_state, Path::new(&path))?;
    // Enregistrement explicite de l'utilisateur : blanchit le suivi dirty
    // jusqu'à la révision snapshotée (l'autosave, lui, ne blanchit pas).
    capture_state.lock()?.mark_clean_at(revision);
    info!(
        "✅ Projet enregistré vers {path} ({} flux, {} labels)",
        manifest.matrix_rows, manifest.label_rows
    );
    Ok(())
}

/// Snapshot du relevé sous verrous courts puis écriture de l'archive vers
/// `dest` (même discipline que `export_csv` : toute l'I/O hors verrou).
/// Retourne le manifest écrit et la révision au moment du snapshot. Ne
/// touche PAS au suivi dirty : l'appelant décide.
fn write_snapshot(
    app_version: &str,
    matrice: &Arc<Mutex<FlowMatrix>>,
    capture_state: &Arc<Mutex<CaptureState>>,
    dest: &Path,
) -> Result<(ProjectManifest, u64), CaptureStateError> {
    let (rows, link_type, labels, context, period) = {
        let matrix = matrice.lock()?;
        (
            matrix.to_flat_vec(),
            matrix.link_type,
            matrix.export_labels(),
            matrix.context.clone(),
            matrix.period.known(),
        )
    };
    // La révision est capturée avec le snapshot : des modifications arrivées
    // pendant l'écriture resteront comptées comme non enregistrées.
    let (settings, revision) = {
        let state = capture_state.lock()?;
        (
            ProjectCaptureSettings {
                config: state.config.clone(),
                filter: state.filter.clone(),
            },
            state.current_revision(),
        )
    };

    let manifest = ProjectManifest {
        schema_version: PROJECT_SCHEMA_VERSION,
        app_version: app_version.to_string(),
        lfms_version: lunar_flows_core::lfms::LFMS_VERSION.to_string(),
        dlt: link_type.map(lunar_flows_core::lfms::link_type_name),
        matrix_rows: rows.len(),
        label_rows: labels.len(),
        saved_at_epoch_secs: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        survey_context: (&context).into(),
    };

    write_project_archive(
        dest, &manifest, &rows, link_type, &context, period, &labels, &settings,
    )?;
    Ok((manifest, revision))
}

/// Ouvre un projet `.lunar` : valide le manifest puis REMPLACE le relevé
/// courant (matrice, graphe, labels, config) par celui du projet. La
/// configuration de capture du projet, si elle est valide, est persistée
/// dans `config_dir`.
#[allow(clippy::too_many_arguments)]
pub fn open_project(
    config_dir: &Path,
    matrice: &Arc<Mutex<FlowMatrix>>,
    graph: &Arc<Mutex<GraphData>>,
    label_store: &Arc<Mutex<LabelStore>>,
    pcinfo: &Arc<Mutex<PcInfoLabel>>,
    capture_state: &Arc<Mutex<CaptureState>>,
    conflict_store: &Arc<Mutex<super::import::LabelConflictStore>>,
    on_event: Channel<CaptureEvent>,
    path: String,
) -> Result<(), CaptureStateError> {
    let extracted = extract_project_archive(Path::new(&path))?;

    // Le swap transactionnel matrice+graphe, l'exclusion avec la capture et
    // les événements émis sont ceux de l'import de matrice existant.
    super::import::import_matrix_files(
        vec![extracted.matrix_csv.to_string_lossy().into_owned()],
        matrice,
        graph,
        label_store,
        capture_state,
        on_event.clone(),
    )?;

    // Les labels du projet remplacent le store (source de vérité unique),
    // appliqués après la matrice pour que la resynchronisation porte sur le
    // relevé fraîchement chargé.
    if let Some(labels_csv) = &extracted.labels_csv {
        super::import::import_label_file(
            labels_csv.to_string_lossy().into_owned(),
            label_store,
            pcinfo,
            matrice,
            graph,
            capture_state,
            conflict_store,
            on_event,
        )?;
    }

    // Le contexte de relevé est rétabli APRÈS les imports : le swap
    // transactionnel de `import_matrix_files` remplace la matrice entière,
    // contexte compris. Le manifest fait foi pour le projet ; un projet v1
    // n'en porte pas et laisse le contexte vide (migration v1 → v2).
    matrice.lock()?.context = (&extracted.manifest.survey_context).into();

    // La configuration embarquée est revalidée ; une config invalide (projet
    // édité à la main, bornes qui ont changé) n'empêche pas l'ouverture du
    // relevé — elle est simplement ignorée avec trace.
    if let Some(settings) = extracted.capture {
        // Le filtre BPF du projet n'est JAMAIS appliqué à la session :
        // l'interface n'affiche que le filtre posé par
        // l'opérateur (`set_filter`), aucune commande ne lui renvoie celui du
        // backend. Un projet reçu d'un tiers aurait donc pu rendre la capture
        // suivante aveugle à un hôte sans que rien ne l'indique. Il reste
        // enregistré dans le projet comme trace des conditions du relevé.
        if let Some(filter) = settings.filter.as_deref().filter(|f| !f.trim().is_empty()) {
            info!("[open_project] filtre de capture du projet non appliqué : {filter:?}");
        }
        match settings.config.validate() {
            Ok(()) => {
                capture_state.lock()?.config = settings.config.clone();
                if let Err(err) = settings.config.save_persisted(config_dir) {
                    warn!("[open_project] persistance de la config impossible: {err:?}");
                }
            }
            Err(err) => warn!("[open_project] config du projet ignorée: {err:?}"),
        }
    }

    // L'état courant correspond exactement au fichier projet : rien à
    // perdre (les imports ci-dessus ont marqué dirty, on annule ici).
    capture_state.lock()?.mark_clean();

    info!(
        "✅ Projet ouvert depuis {path} (schéma v{}, app {})",
        extracted.manifest.schema_version, extracted.manifest.app_version
    );
    Ok(())
}

/// Vrai si des données seraient perdues en quittant ou réinitialisant sans
/// enregistrer. Consommé par les confirmations du frontend.
pub fn is_session_dirty(
    capture_state: &Arc<Mutex<CaptureState>>,
) -> Result<bool, CaptureStateError> {
    Ok(capture_state.lock()?.is_dirty())
}

// --- Autosave et récupération après crash ---

/// Nom de l'archive d'autosave, réécrite en place (atomiquement) par le
/// thread d'autosave tant que le relevé change.
pub const AUTOSAVE_FILE: &str = "autosave.lunar";
/// Sentinelle posée au démarrage et retirée à la fermeture propre : encore
/// présente au démarrage suivant, elle signe un crash.
const SESSION_LOCK_FILE: &str = "session.lock";
/// Cadence de l'autosave. Le tick ne fait rien si la révision n'a pas bougé
/// ou si un import est en cours.
pub const AUTOSAVE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

/// Offre de récupération figée au démarrage, AVANT la pose de la sentinelle
/// de la session courante (sinon elle se détecterait elle-même).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecoveryOffer(Option<PathBuf>);

impl RecoveryOffer {
    /// Offre de la session précédente, d'après le dossier d'autosave de
    /// `data_dir` : à calculer avant [`SessionPersistence::start`].
    pub fn detect(data_dir: &Path) -> Self {
        let offer = detect_recovery(&autosave_dir(data_dir));
        if offer.is_some() {
            info!("[autosave] session précédente non fermée : récupération proposée");
        }
        Self(offer)
    }
}

/// Chemin de l'autosave de la session précédente si elle s'est terminée par
/// un crash, sinon `None`. Le frontend propose alors la récupération.
pub fn get_recovery_offer(offer: &RecoveryOffer) -> Option<String> {
    offer
        .0
        .as_ref()
        .map(|path| path.to_string_lossy().into_owned())
}

/// Dossier des fichiers d'autosave/sentinelle de l'application, sous son
/// dossier de données.
pub fn autosave_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("autosave")
}

/// Une sentinelle laissée par une session morte + un autosave lisible =
/// récupération à proposer. Fonction pure sur le contenu du dossier.
fn detect_recovery(dir: &Path) -> Option<PathBuf> {
    let autosave = dir.join(AUTOSAVE_FILE);
    (dir.join(SESSION_LOCK_FILE).exists() && autosave.is_file()).then_some(autosave)
}

/// Persistance de session en cours : sentinelle posée et thread
/// d'autosave lancé.
///
/// [`Self::stop`] est la fermeture propre : thread arrêté puis sentinelle
/// retirée. Un simple `drop` arrête le thread mais laisse la sentinelle,
/// comme un crash : une sortie qui n'a pas dit « fermeture propre » propose
/// la récupération au démarrage suivant.
#[derive(Debug)]
pub struct SessionPersistence {
    /// Dossier d'autosave (autosave et sentinelle).
    dir: PathBuf,
    /// Fermé (abandonné) pour réveiller et arrêter le thread d'autosave.
    stop: Option<Sender<()>>,
    /// Thread d'autosave, joint à l'arrêt.
    thread: Option<std::thread::JoinHandle<()>>,
}

impl SessionPersistence {
    /// Crée le dossier d'autosave de `data_dir`, pose la sentinelle de cette
    /// session et démarre le thread d'autosave (cadence
    /// [`AUTOSAVE_INTERVAL`]). Échoue seulement si le dossier ne peut pas être
    /// créé ; une sentinelle non posée est seulement tracée.
    pub fn start(
        data_dir: &Path,
        matrice: Arc<Mutex<FlowMatrix>>,
        capture_state: Arc<Mutex<CaptureState>>,
    ) -> Result<Self, CaptureStateError> {
        Self::start_with_interval(data_dir, matrice, capture_state, AUTOSAVE_INTERVAL)
    }

    /// [`Self::start`] avec une cadence choisie (tests).
    fn start_with_interval(
        data_dir: &Path,
        matrice: Arc<Mutex<FlowMatrix>>,
        capture_state: Arc<Mutex<CaptureState>>,
        interval: std::time::Duration,
    ) -> Result<Self, CaptureStateError> {
        let dir = autosave_dir(data_dir);
        fs::create_dir_all(&dir).inspect_err(|err| {
            warn!("[autosave] création {} impossible: {err}", dir.display());
        })?;
        if let Err(err) = fs::write(dir.join(SESSION_LOCK_FILE), std::process::id().to_string()) {
            warn!("[autosave] sentinelle non posée: {err}");
        }
        let (stop, stopped) = bounded::<()>(0);
        let thread = spawn_autosaver(
            matrice,
            capture_state,
            dir.join(AUTOSAVE_FILE),
            interval,
            stopped,
        );
        Ok(Self {
            dir,
            stop: Some(stop),
            thread: Some(thread),
        })
    }

    /// Fermeture propre : arrête le thread d'autosave puis retire la
    /// sentinelle, le prochain démarrage ne proposera pas de récupération.
    pub fn stop(mut self) {
        self.halt();
        let _ = fs::remove_file(self.dir.join(SESSION_LOCK_FILE));
    }

    /// Arrête et joint le thread d'autosave (idempotent).
    fn halt(&mut self) {
        drop(self.stop.take());
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            warn!("[autosave] le thread d'autosave a paniqué");
        }
    }
}

impl Drop for SessionPersistence {
    fn drop(&mut self) {
        self.halt();
    }
}

/// Thread d'autosave : réécrit l'archive d'autosave quand la révision a
/// bougé depuis la dernière écriture, jamais pendant un import (le prochain
/// tick verra l'état stable d'après). Best-effort : une erreur est tracée
/// et le tick suivant réessaie. S'arrête dès que `stopped` est fermé.
fn spawn_autosaver(
    matrice: Arc<Mutex<FlowMatrix>>,
    capture_state: Arc<Mutex<CaptureState>>,
    autosave_path: PathBuf,
    interval: std::time::Duration,
    stopped: crossbeam::channel::Receiver<()>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut last_written: u64 = 0;
        // Un tick par expiration du délai ; la fermeture du canal (ou un
        // message) termine la boucle.
        while let Err(RecvTimeoutError::Timeout) = stopped.recv_timeout(interval) {
            let (revision, importing) = match capture_state.lock() {
                Ok(state) => (
                    state.current_revision(),
                    state.phase == crate::state::capture::capture_status::CapturePhase::Importing,
                ),
                Err(err) => {
                    warn!("[autosave] verrou empoisonné, tick ignoré: {err}");
                    continue;
                }
            };
            if revision == last_written || importing {
                continue;
            }

            match write_snapshot(APP_VERSION, &matrice, &capture_state, &autosave_path) {
                Ok((manifest, snapshot_revision)) => {
                    last_written = snapshot_revision;
                    info!(
                        "[autosave] relevé sauvegardé ({} flux, révision {snapshot_revision})",
                        manifest.matrix_rows
                    );
                }
                Err(err) => warn!("[autosave] écriture échouée, retenté au prochain tick: {err:?}"),
            }
        }
    })
}

/// Écrit l'archive projet vers `dest` : entrées composées en staging via les
/// writers CSV existants, archive assemblée en `.part` puis renommée — une
/// écriture interrompue ne laisse jamais d'archive partielle au chemin final.
#[allow(clippy::too_many_arguments)]
fn write_project_archive(
    dest: &Path,
    manifest: &ProjectManifest,
    rows: &[crate::state::flow_matrix::FlowMatrixRow],
    link_type: Option<packet_parser::LinkType>,
    context: &lunar_flows_core::lfms::SurveyContext,
    period: Option<lunar_flows_core::lfms::ObservedPeriod>,
    labels: &[(String, String, String)],
    settings: &ProjectCaptureSettings,
) -> Result<(), CaptureStateError> {
    if let Some(parent) = dest.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }

    let staging = unique_temp_dir("save")?;
    let result = (|| -> Result<(), CaptureStateError> {
        let matrix_csv = staging.join(MATRIX_ENTRY);
        // Le contexte et la période du relevé voyagent dans le préambule
        // `#LFMS` de la matrice embarquée, pas seulement dans le manifest : la
        // matrice extraite seule de l'archive reste qualifiée, et la période
        // revient à l'ouverture du projet (réimport de cette matrice).
        FlowMatrix::write_rows_to_csv_with_context(
            rows,
            link_type,
            context,
            period,
            &matrix_csv.to_string_lossy(),
        )?;
        let labels_csv = staging.join(LABELS_ENTRY);
        FlowMatrix::write_label_rows_to_csv(labels, &labels_csv.to_string_lossy())?;

        let manifest_json = serde_json::to_string_pretty(manifest)
            .map_err(|err| ExportError::Io(std::io::Error::other(err)))?;
        let capture_json = serde_json::to_string_pretty(settings)
            .map_err(|err| ExportError::Io(std::io::Error::other(err)))?;

        let tmp_dest = dest.with_extension("part");
        let write = (|| -> Result<(), CaptureStateError> {
            let file = File::create(&tmp_dest)?;
            let mut zip = ZipWriter::new(file);
            let options =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

            for (name, content) in [
                (MANIFEST_ENTRY, manifest_json),
                (CAPTURE_ENTRY, capture_json),
            ] {
                zip.start_file(name, options).map_err(ExportError::Zip)?;
                zip.write_all(content.as_bytes())?;
            }
            for (name, path) in [(MATRIX_ENTRY, &matrix_csv), (LABELS_ENTRY, &labels_csv)] {
                zip.start_file(name, options).map_err(ExportError::Zip)?;
                let mut src = File::open(path)?;
                copy(&mut src, &mut zip)?;
            }
            zip.finish().map_err(ExportError::Zip)?;
            Ok(())
        })();
        if let Err(err) = write {
            let _ = fs::remove_file(&tmp_dest);
            return Err(err);
        }
        fs::rename(&tmp_dest, dest)?;
        Ok(())
    })();
    remove_temp_dir(&staging);
    result
}

/// Contenu extrait d'une archive projet, prêt à passer aux chemins d'import.
/// Le dossier temporaire vit tant que la valeur existe.
#[derive(Debug)]
struct ExtractedProject {
    /// Manifest lu et validé (version de schéma supportée).
    manifest: ProjectManifest,
    /// Matrice extraite dans le dossier temporaire, toujours présente.
    matrix_csv: PathBuf,
    /// Labels extraits, `None` si l'archive n'en contient pas.
    labels_csv: Option<PathBuf>,
    /// Réglages de session, `None` si l'entrée est absente ou invalide.
    capture: Option<ProjectCaptureSettings>,
    /// Supprime le dossier temporaire à la sortie de portée, succès ou non.
    _temp_dir: TempDirGuard,
}

/// Dossier temporaire d'extraction, supprimé au `drop` quelle que soit l'issue de l'ouverture.
#[derive(Debug)]
struct TempDirGuard(PathBuf);

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        remove_temp_dir(&self.0);
    }
}

/// Ouvre et valide une archive projet : manifest lisible, version de schéma
/// supportée, matrice présente. Seules les entrées à noms fixes sont
/// extraites — aucun nom d'entrée de l'archive n'est utilisé comme chemin,
/// ce qui exclut toute traversée (« zip slip »).
fn extract_project_archive(src: &Path) -> Result<ExtractedProject, CaptureStateError> {
    let open_error = |message: String| {
        CaptureStateError::Import(PcapImportError::OpenFileError(
            src.to_string_lossy().into_owned(),
            message,
        ))
    };

    let file = File::open(src).map_err(|err| open_error(format!("ouverture impossible: {err}")))?;
    let mut archive = ZipArchive::new(file)
        .map_err(|err| open_error(format!("ce n'est pas une archive projet valide: {err}")))?;

    let manifest_text = read_entry(&mut archive, MANIFEST_ENTRY, MAX_JSON_ENTRY_BYTES)
        .map_err(|err| open_error(format!("{MANIFEST_ENTRY} illisible: {err}")))?
        .ok_or_else(|| open_error(format!("{MANIFEST_ENTRY} absent — pas un projet LUNAR")))?;
    let manifest: ProjectManifest = serde_json::from_str(&manifest_text)
        .map_err(|err| open_error(format!("{MANIFEST_ENTRY} invalide: {err}")))?;
    if manifest.schema_version > PROJECT_SCHEMA_VERSION {
        return Err(open_error(format!(
            "projet au schéma v{} (écrit par la version {}), cette version de LUNAR ne lit que v{} au plus — \
             mettez LUNAR à jour pour l'ouvrir",
            manifest.schema_version, manifest.app_version, PROJECT_SCHEMA_VERSION
        )));
    }

    let matrix_text = read_entry(&mut archive, MATRIX_ENTRY, MAX_CSV_ENTRY_BYTES)
        .map_err(|err| open_error(format!("{MATRIX_ENTRY} illisible: {err}")))?
        .ok_or_else(|| open_error(format!("{MATRIX_ENTRY} absent de l'archive")))?;
    let labels_text = read_entry(&mut archive, LABELS_ENTRY, MAX_CSV_ENTRY_BYTES)
        .map_err(|err| open_error(format!("{LABELS_ENTRY} illisible: {err}")))?;
    let capture = read_entry(&mut archive, CAPTURE_ENTRY, MAX_JSON_ENTRY_BYTES)
        .map_err(|err| open_error(format!("{CAPTURE_ENTRY} illisible: {err}")))?
        .and_then(|text| match serde_json::from_str(&text) {
            Ok(settings) => Some(settings),
            Err(err) => {
                warn!("[open_project] {CAPTURE_ENTRY} invalide, ignoré: {err}");
                None
            }
        });

    // Le garde est posé dès la création : toute sortie anticipée ci-dessous
    // supprime le dossier.
    let temp_dir = TempDirGuard(
        unique_temp_dir("open")
            .map_err(|err| open_error(format!("dossier temporaire impossible à créer: {err}")))?,
    );
    let matrix_csv = temp_dir.0.join(MATRIX_ENTRY);
    fs::write(&matrix_csv, matrix_text)?;
    let labels_csv = match labels_text {
        Some(text) => {
            let path = temp_dir.0.join(LABELS_ENTRY);
            fs::write(&path, text)?;
            Some(path)
        }
        None => None,
    };

    Ok(ExtractedProject {
        manifest,
        matrix_csv,
        labels_csv,
        capture,
        _temp_dir: temp_dir,
    })
}

/// Lit une entrée de l'archive par son nom fixe ; `None` si absente. Une
/// entrée de plus de `max_bytes` décompressés est refusée.
fn read_entry(
    archive: &mut ZipArchive<File>,
    name: &str,
    max_bytes: u64,
) -> Result<Option<String>, zip::result::ZipError> {
    match archive.by_name(name) {
        Ok(entry) => {
            // Taille annoncée par l'archive : refus sans rien décompresser.
            if entry.size() > max_bytes {
                return Err(zip::result::ZipError::Io(entry_too_large(max_bytes)));
            }
            // L'annonce peut mentir : la lecture elle-même reste bornée.
            read_bounded(entry, max_bytes)
                .map(Some)
                .map_err(zip::result::ZipError::Io)
        }
        Err(zip::result::ZipError::FileNotFound) => Ok(None),
        Err(err) => Err(err),
    }
}

/// Lit au plus `max_bytes` octets de `reader` en texte UTF-8 ; au-delà,
/// erreur. Lit en octets puis décode : une coupure au milieu d'un caractère
/// multi-octets ne doit pas passer pour un fichier mal encodé.
fn read_bounded(reader: impl Read, max_bytes: u64) -> io::Result<String> {
    let mut bytes = Vec::new();
    reader
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max_bytes {
        return Err(entry_too_large(max_bytes));
    }
    String::from_utf8(bytes).map_err(|err| io::Error::new(ErrorKind::InvalidData, err))
}

/// Erreur d'une entrée qui dépasse sa taille décompressée maximale.
fn entry_too_large(max_bytes: u64) -> io::Error {
    const MIB: u64 = 1024 * 1024;
    let limit = if max_bytes >= MIB {
        format!("{} Mio", max_bytes / MIB)
    } else {
        format!("{max_bytes} octets")
    };
    io::Error::new(
        ErrorKind::InvalidData,
        format!("taille décompressée supérieure au maximum admis ({limit})"),
    )
}

/// Dossier temporaire privé, propre à cet appel : deux sauvegardes
/// concurrentes ne partagent jamais leur staging, et un autre compte du
/// poste ne peut ni le préparer à l'avance ni lire son contenu.
/// Le nom porte un suffixe imprévisible ; un nom déjà pris (y compris par un
/// lien symbolique) n'est jamais réutilisé : un autre est essayé.
fn unique_temp_dir(label: &str) -> io::Result<PathBuf> {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let base = std::env::temp_dir();
    for _ in 0..TEMP_DIR_ATTEMPTS {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = base.join(format!(
            "lunar_project_{label}_{}_{n}_{:016x}",
            std::process::id(),
            unpredictable_suffix(n)
        ));
        match create_private_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(err) if err.kind() == ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }
    Err(io::Error::new(
        ErrorKind::AlreadyExists,
        format!(
            "aucun nom de dossier temporaire libre après {TEMP_DIR_ATTEMPTS} essais dans {}",
            base.display()
        ),
    ))
}

/// 64 bits imprévisibles sans dépendance supplémentaire : les clés de
/// `RandomState` viennent du générateur aléatoire du système.
fn unpredictable_suffix(n: u32) -> u64 {
    use std::hash::BuildHasher;
    std::collections::hash_map::RandomState::new().hash_one((std::process::id(), n))
}

/// Crée `dir` de façon exclusive : échec `AlreadyExists` si le chemin existe
/// déjà, dossier ou lien symbolique. Droits `0700` sous Unix ; sous Windows,
/// le dossier temporaire de l'utilisateur est déjà propre à son compte.
fn create_private_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new().mode(0o700).create(dir)
    }
    #[cfg(not(unix))]
    {
        fs::DirBuilder::new().create(dir)
    }
}

/// Supprime un dossier temporaire. Un échec est tracé, sans masquer l'erreur
/// principale de l'appelant : les fichiers du relevé ne restent pas sur le
/// disque à l'insu de l'opérateur.
fn remove_temp_dir(dir: &Path) {
    if let Err(err) = fs::remove_dir_all(dir)
        && err.kind() != ErrorKind::NotFound
    {
        warn!(
            "[project] dossier temporaire {} non supprimé: {err}",
            dir.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commandes::import::import_matrix_files;
    use crate::state::flow_matrix::FlowMatrixRow;

    /// Aller-retour joué via les vraies commandes : un relevé importé
    /// puis enregistré comme projet se rouvre avec une matrice identique
    /// après un « redémarrage » (état vidé), et le suivi dirty suit chaque
    /// étape (import → modifié, save/open → propre).
    #[test]
    fn round_trip_save_open_restores_the_same_survey() {
        let matrix = Arc::new(Mutex::new(FlowMatrix::new()));
        let graph = Arc::new(Mutex::new(crate::state::graph::GraphData::new()));
        let labels = Arc::new(Mutex::new(LabelStore::new()));
        let pcinfo = Arc::new(Mutex::new(PcInfoLabel::new()));
        let capture_state = Arc::new(Mutex::new(CaptureState::new()));
        let conflicts = Arc::new(Mutex::new(
            crate::commandes::import::LabelConflictStore::default(),
        ));

        // Interface nommée : la configuration du projet est alors valide et
        // doit être persistée à l'ouverture (dans un dossier de test).
        capture_state.lock().unwrap().config.device_name = "eth0".to_string();
        let config_dir = unique_temp_dir("test_e2e_config").unwrap();
        let read_dirty = || -> bool { is_session_dirty(&capture_state).unwrap() };

        assert!(!read_dirty(), "état initial propre");

        // 1) Relevé peuplé par le VRAI chemin d'import.
        let source = crate::test_support::fixtures().join("20260703_NP_Matrice.csv");
        import_matrix_files(
            vec![source.to_string_lossy().into_owned()],
            &matrix,
            &graph,
            &labels,
            &capture_state,
            Channel::new(|_| {}),
        )
        .expect("l'import de la matrice de test doit réussir");

        let rows_before = matrix.lock().unwrap().to_flat_vec();
        assert!(
            !rows_before.is_empty(),
            "la fixture doit peupler la matrice"
        );
        let nodes_before = graph.lock().unwrap().nodes.len();
        assert!(read_dirty(), "un import marque le relevé modifié");

        // 2) Enregistrement du projet.
        let dir = unique_temp_dir("test_e2e").unwrap();
        let project_path = dir.join("relevé de test.lunar");
        save_project(
            &matrix,
            &capture_state,
            project_path.to_string_lossy().into_owned(),
        )
        .expect("save_project doit réussir");
        assert!(!read_dirty(), "un enregistrement blanchit le relevé");

        // 3) « Redémarrage » : état vidé puis projet rouvert.
        matrix.lock().unwrap().clear();
        graph.lock().unwrap().clear();
        labels.lock().unwrap().clear();
        open_project(
            &config_dir,
            &matrix,
            &graph,
            &labels,
            &pcinfo,
            &capture_state,
            &conflicts,
            Channel::new(|_| {}),
            project_path.to_string_lossy().into_owned(),
        )
        .expect("open_project doit réussir");

        assert_eq!(
            matrix.lock().unwrap().to_flat_vec(),
            rows_before,
            "la matrice rouverte doit être identique à celle enregistrée"
        );
        assert_eq!(
            graph.lock().unwrap().nodes.len(),
            nodes_before,
            "le graphe est reconstruit avec les mêmes nœuds"
        );
        assert!(
            !read_dirty(),
            "un projet fraîchement ouvert n'a rien à perdre"
        );
        assert!(
            CaptureConfig::load_persisted(&config_dir)
                .unwrap()
                .is_some(),
            "la configuration valide du projet est persistée"
        );

        fs::remove_dir_all(&dir).ok();
        fs::remove_dir_all(&config_dir).ok();
    }

    /// Le filtre BPF d'un projet ne doit jamais remplacer celui de
    /// la session. L'interface n'affiche que le filtre posé par l'opérateur :
    /// un filtre venu du fichier s'appliquerait à la capture suivante sans
    /// être visible. Joué par la vraie commande, avec et sans filtre de
    /// session déjà posé.
    #[test]
    fn opening_a_project_never_changes_the_session_filter() {
        let capture_state = Arc::new(Mutex::new(CaptureState::new()));
        let matrix = Arc::new(Mutex::new(FlowMatrix::new()));
        let graph = Arc::new(Mutex::new(crate::state::graph::GraphData::new()));
        let labels = Arc::new(Mutex::new(LabelStore::new()));
        let pcinfo = Arc::new(Mutex::new(PcInfoLabel::new()));
        let conflicts = Arc::new(Mutex::new(
            crate::commandes::import::LabelConflictStore::default(),
        ));
        let config_dir = unique_temp_dir("test_filter_not_restored_config").unwrap();

        // Le projet de test porte le filtre « udp » (`sample_settings`).
        let dir = unique_temp_dir("test_filter_not_restored").unwrap();
        let project_path = dir.join("projet.lunar");
        write_sample_project(&project_path, PROJECT_SCHEMA_VERSION);
        assert_eq!(
            extract_project_archive(&project_path)
                .unwrap()
                .capture
                .unwrap()
                .filter
                .as_deref(),
            Some("udp"),
            "le projet de test doit porter un filtre, sinon le test ne prouve rien"
        );

        let open = || {
            open_project(
                &config_dir,
                &matrix,
                &graph,
                &labels,
                &pcinfo,
                &capture_state,
                &conflicts,
                Channel::new(|_| {}),
                project_path.to_string_lossy().into_owned(),
            )
            .expect("open_project doit réussir");
        };

        open();
        assert_eq!(
            capture_state.lock().unwrap().filter,
            None,
            "sans filtre de session, l'ouverture d'un projet n'en installe aucun"
        );

        capture_state.lock().unwrap().filter = Some("tcp port 502".to_string());
        open();
        assert_eq!(
            capture_state.lock().unwrap().filter.as_deref(),
            Some("tcp port 502"),
            "le filtre posé par l'opérateur survit à l'ouverture d'un projet"
        );

        fs::remove_dir_all(&dir).ok();
        fs::remove_dir_all(&config_dir).ok();
    }

    fn sample_row(port_destination: u16) -> FlowMatrixRow {
        FlowMatrixRow {
            mac_source: "aa:bb:cc:00:00:01".to_string(),
            mac_destination: "aa:bb:cc:00:00:11".to_string(),
            vlan_id: None,
            protocol_data_link: "IPv4".to_string(),
            ip_source: "192.168.50.1".to_string(),
            ip_source_type: "Privée".to_string(),
            label_source: None,
            ip_destination: "192.168.50.11".to_string(),
            ip_destination_type: "Privée".to_string(),
            label_destination: Some("capteur".to_string()),
            port_source: Some(5247),
            port_destination: Some(port_destination),
            protocol_transport: Some("UDP".to_string()),
            application_protocol: Some("Unknown".to_string()),
            count: 1,
            total_bytes: 101,
            last_seen: "2024-07-03 09:46:40.400000 UTC".to_string(),
            encap_id: String::new(),
            origin: String::new(),
        }
    }

    fn sample_settings() -> ProjectCaptureSettings {
        ProjectCaptureSettings {
            config: CaptureConfig {
                device_name: "eth0".to_string(),
                buffer_size: 18_000_000,
                chan_capacity: 10_000,
                timeout: 25,
                snaplen: 65536,
            },
            filter: Some("udp".to_string()),
        }
    }

    fn sample_context() -> lunar_flows_core::lfms::SurveyContext {
        lunar_flows_core::lfms::SurveyContext {
            site: Some("Usine Nord — atelier 3".to_string()),
            sensor: Some("sonde=A 100%".to_string()),
            interface: Some("eth0".to_string()),
        }
    }

    fn sample_manifest(schema_version: u32) -> ProjectManifest {
        sample_manifest_with_context(schema_version, &sample_context())
    }

    fn sample_manifest_with_context(
        schema_version: u32,
        context: &lunar_flows_core::lfms::SurveyContext,
    ) -> ProjectManifest {
        ProjectManifest {
            schema_version,
            app_version: "test".to_string(),
            lfms_version: lunar_flows_core::lfms::LFMS_VERSION.to_string(),
            dlt: Some("ETHERNET".to_string()),
            matrix_rows: 2,
            label_rows: 1,
            saved_at_epoch_secs: 1_754_000_000,
            survey_context: context.into(),
        }
    }

    fn sample_rows_and_labels() -> (Vec<FlowMatrixRow>, Vec<(String, String, String)>) {
        (
            vec![sample_row(40000), sample_row(40001)],
            vec![(
                "aa:bb:cc:00:00:11".to_string(),
                "192.168.50.11".to_string(),
                "capteur".to_string(),
            )],
        )
    }

    fn write_sample_project(dest: &Path, schema_version: u32) {
        write_sample_project_with_context(dest, schema_version, &sample_context());
    }

    fn write_sample_project_with_context(
        dest: &Path,
        schema_version: u32,
        context: &lunar_flows_core::lfms::SurveyContext,
    ) {
        let link_type = lunar_flows_core::lfms::link_type_from_text("ETHERNET");
        let (rows, labels) = sample_rows_and_labels();
        write_project_archive(
            dest,
            &sample_manifest_with_context(schema_version, context),
            &rows,
            link_type,
            context,
            None,
            &labels,
            &sample_settings(),
        )
        .unwrap();
    }

    #[test]
    fn round_trip_preserves_manifest_matrix_labels_and_settings() {
        let dir = unique_temp_dir("test_roundtrip").unwrap();
        let dest = dir.join("relevé projet.lunar");

        write_sample_project(&dest, PROJECT_SCHEMA_VERSION);

        assert!(dest.is_file(), "l'archive doit exister au chemin final");
        assert!(
            !dest.with_extension("part").exists(),
            "aucun fichier temporaire .part ne doit rester après une sauvegarde réussie"
        );

        let extracted = extract_project_archive(&dest).unwrap();
        assert_eq!(extracted.manifest, sample_manifest(PROJECT_SCHEMA_VERSION));

        let matrix_text = fs::read_to_string(&extracted.matrix_csv).unwrap();
        // La version vient de la crate : un bump de LFMS_VERSION ne doit pas
        // exiger de retoucher ce test, seulement de le voir rester vrai.
        let expected_preamble = format!(
            "#LFMS version={} dlt=ETHERNET",
            lunar_flows_core::lfms::LFMS_VERSION
        );
        assert!(
            matrix_text.starts_with(&expected_preamble),
            "la matrice embarquée doit porter le préambule LFMS, trouvé: {}",
            matrix_text.lines().next().unwrap_or_default()
        );
        assert_eq!(matrix_text.matches("192.168.50.11").count(), 2);

        let labels_text = fs::read_to_string(extracted.labels_csv.as_ref().unwrap()).unwrap();
        assert!(labels_text.contains("aa:bb:cc:00:00:11"));
        assert!(labels_text.contains("capteur"));

        let settings = extracted.capture.as_ref().unwrap();
        assert_eq!(settings.config.device_name, "eth0");
        assert_eq!(settings.filter.as_deref(), Some("udp"));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn future_schema_version_is_refused_before_any_mutation() {
        let dir = unique_temp_dir("test_future_schema").unwrap();
        let dest = dir.join("futur.lunar");

        write_sample_project(&dest, PROJECT_SCHEMA_VERSION + 1);

        let err = extract_project_archive(&dest).unwrap_err();
        let message = format!("{err:?}");
        assert!(
            message.contains("mettez LUNAR à jour"),
            "le refus doit expliquer la marche à suivre, trouvé: {message}"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn garbage_file_is_refused_as_invalid_archive() {
        let dir = unique_temp_dir("test_garbage").unwrap();
        let dest = dir.join("pas-un-zip.lunar");
        fs::write(&dest, b"ceci n'est pas une archive").unwrap();

        let err = extract_project_archive(&dest).unwrap_err();
        assert!(matches!(
            err,
            CaptureStateError::Import(PcapImportError::OpenFileError(_, _))
        ));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn archive_without_matrix_entry_is_refused() {
        let dir = unique_temp_dir("test_no_matrix").unwrap();
        let dest = dir.join("sans-matrice.lunar");

        let file = File::create(&dest).unwrap();
        let mut zip = ZipWriter::new(file);
        let options = SimpleFileOptions::default();
        zip.start_file(MANIFEST_ENTRY, options).unwrap();
        zip.write_all(
            serde_json::to_string(&sample_manifest(PROJECT_SCHEMA_VERSION))
                .unwrap()
                .as_bytes(),
        )
        .unwrap();
        zip.finish().unwrap();

        let err = extract_project_archive(&dest).unwrap_err();
        let message = format!("{err:?}");
        assert!(
            message.contains(MATRIX_ENTRY),
            "le refus doit nommer l'entrée manquante, trouvé: {message}"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn oversized_manifest_is_refused_with_the_entry_named() {
        // Le manifest est la première entrée lue, avec sa borne réelle
        // (1 Mio). Des espaces : très compressibles, comme une bombe de
        // décompression.
        let dir = unique_temp_dir("test_oversized_manifest").unwrap();
        let dest = dir.join("manifest-geant.lunar");

        let file = File::create(&dest).unwrap();
        let mut zip = ZipWriter::new(file);
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        zip.start_file(MANIFEST_ENTRY, options).unwrap();
        zip.write_all(&vec![b' '; (MAX_JSON_ENTRY_BYTES + 1) as usize])
            .unwrap();
        zip.finish().unwrap();

        let message = format!("{:?}", extract_project_archive(&dest).unwrap_err());
        assert!(
            message.contains(MANIFEST_ENTRY) && message.contains("maximum admis (1 Mio)"),
            "refus attendu sur la taille du manifest, trouvé: {message}"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_entry_is_read_up_to_its_bound_and_not_beyond() {
        let dir = unique_temp_dir("test_entry_bound").unwrap();
        let dest = dir.join("borne.lunar");

        let file = File::create(&dest).unwrap();
        let mut zip = ZipWriter::new(file);
        zip.start_file(MATRIX_ENTRY, SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&[b'a'; 2048]).unwrap();
        zip.finish().unwrap();

        let mut archive = ZipArchive::new(File::open(&dest).unwrap()).unwrap();
        assert!(read_entry(&mut archive, MATRIX_ENTRY, 2047).is_err());
        assert_eq!(
            read_entry(&mut archive, MATRIX_ENTRY, 2048)
                .unwrap()
                .map(|text| text.len()),
            Some(2048)
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn bounded_read_stops_on_an_endless_stream() {
        // Une taille annoncée mensongère revient à ce cas : la lecture
        // s'arrête à la borne au lieu de tout décompresser.
        let err = read_bounded(io::repeat(b'a'), 1024).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidData);
        assert!(err.to_string().contains("1024 octets"), "{err}");
        assert_eq!(read_bounded(&b"abc"[..], 3).unwrap(), "abc");
    }

    #[test]
    fn temp_dirs_are_distinct_and_private() {
        let first = unique_temp_dir("test_private").unwrap();
        let second = unique_temp_dir("test_private").unwrap();
        assert_ne!(first, second);
        assert!(first.is_dir() && second.is_dir());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for dir in [&first, &second] {
                let mode = fs::metadata(dir).unwrap().permissions().mode() & 0o777;
                assert_eq!(mode, 0o700, "{} créé en {mode:o}", dir.display());
            }
        }

        fs::remove_dir_all(&first).ok();
        fs::remove_dir_all(&second).ok();
    }

    #[test]
    fn a_prepared_path_is_never_reused() {
        // Un chemin préparé à l'avance (dossier, ou lien symbolique vers un
        // dossier choisi par un autre compte) est refusé, jamais partagé.
        let root = unique_temp_dir("test_prepared").unwrap();
        let existing = root.join("deja-la");
        fs::create_dir(&existing).unwrap();
        assert_eq!(
            create_private_dir(&existing).unwrap_err().kind(),
            ErrorKind::AlreadyExists
        );

        #[cfg(unix)]
        {
            let target = root.join("cible");
            fs::create_dir(&target).unwrap();
            let link = root.join("lien");
            std::os::unix::fs::symlink(&target, &link).unwrap();
            assert_eq!(
                create_private_dir(&link).unwrap_err().kind(),
                ErrorKind::AlreadyExists
            );
            assert_eq!(
                fs::read_dir(&target).unwrap().count(),
                0,
                "la cible du lien ne doit pas être touchée"
            );
        }

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn recovery_needs_both_sentinel_and_autosave() {
        let dir = unique_temp_dir("test_recovery").unwrap();

        assert!(
            detect_recovery(&dir).is_none(),
            "dossier vide : rien à récupérer"
        );

        fs::write(dir.join(SESSION_LOCK_FILE), b"42").unwrap();
        assert!(
            detect_recovery(&dir).is_none(),
            "sentinelle sans autosave : rien à proposer"
        );

        fs::write(dir.join(AUTOSAVE_FILE), b"zip").unwrap();
        assert_eq!(
            detect_recovery(&dir),
            Some(dir.join(AUTOSAVE_FILE)),
            "sentinelle + autosave : récupération proposée"
        );

        fs::remove_file(dir.join(SESSION_LOCK_FILE)).unwrap();
        assert!(
            detect_recovery(&dir).is_none(),
            "fermeture propre (sentinelle retirée) : pas de récupération"
        );

        fs::remove_dir_all(&dir).ok();
    }

    /// Migration v1 → v2.
    ///
    /// L'archive est forgée **telle que la v1 l'écrivait réellement** : un
    /// manifest JSON sans la clé `survey_context` et une matrice au préambule
    /// `#LFMS version=1`. Reconstruire le fixture avec le writer courant ne
    /// prouverait rien — il produirait déjà du v2.
    #[test]
    fn a_v1_project_opens_under_v2_with_an_empty_context() {
        let dir = unique_temp_dir("test_migration_v1").unwrap();
        let dest = dir.join("relevé v1.lunar");

        let manifest_v1 = serde_json::json!({
            "schema_version": 1,
            "app_version": "4.9.0",
            "lfms_version": "1",
            "dlt": "ETHERNET",
            "matrix_rows": 2,
            "label_rows": 1,
            "saved_at_epoch_secs": 1_754_000_000u64,
        });
        assert!(
            manifest_v1.get("survey_context").is_none(),
            "le fixture doit être un vrai manifest v1, sans la clé de contexte"
        );

        // Matrice v1 : le préambule est le littéral qu'écrivait la version
        // précédente, mais l'en-tête de colonnes vient du writer réel — un
        // en-tête recopié à la main mentirait sur le schéma dès qu'il
        // évoluerait, et le fixture ne prouverait plus rien.
        let (rows, _) = sample_rows_and_labels();
        let real_csv = dir.join("v1_source.csv");
        FlowMatrix::write_rows_to_csv(
            &rows,
            lunar_flows_core::lfms::link_type_from_text("ETHERNET"),
            &real_csv.to_string_lossy(),
        )
        .unwrap();
        let written = fs::read_to_string(&real_csv).unwrap();
        let body = written
            .split_once('\n')
            .expect("le writer émet un préambule puis le corps")
            .1;
        let matrix_v1 = format!("#LFMS version=1 dlt=ETHERNET\n{body}");

        let file = File::create(&dest).unwrap();
        let mut zip = ZipWriter::new(file);
        let options = SimpleFileOptions::default();
        zip.start_file(MANIFEST_ENTRY, options).unwrap();
        zip.write_all(manifest_v1.to_string().as_bytes()).unwrap();
        zip.start_file(MATRIX_ENTRY, options).unwrap();
        zip.write_all(matrix_v1.as_bytes()).unwrap();
        zip.finish().unwrap();

        let extracted = extract_project_archive(&dest).unwrap();

        assert_eq!(
            extracted.manifest.schema_version, 1,
            "la version lue est celle du fichier, pas celle du code"
        );
        assert_eq!(
            extracted.manifest.survey_context,
            ManifestSurveyContext::default(),
            "un projet v1 s'ouvre avec un contexte vide — aucune métadonnée inventée"
        );
        let restored: lunar_flows_core::lfms::SurveyContext =
            (&extracted.manifest.survey_context).into();
        assert!(
            restored.is_empty(),
            "le contexte rétabli dans la matrice doit être vide, pas rempli de chaînes vides"
        );

        // Les lignes du relevé v1 se relisent : le fixture est bien un projet
        // ouvrable, pas seulement un manifest bien formé.
        let rows_read = lunar_flows_core::csv::read_matrix_rows(&extracted.matrix_csv).unwrap();
        assert_eq!(
            rows_read.len(),
            rows.len(),
            "un projet v1 doit rendre toutes ses lignes sous le schéma v2"
        );

        fs::remove_dir_all(&dir).ok();
    }

    /// Le contexte survit à l'aller-retour, dans le manifest **et** dans le
    /// préambule de la matrice embarquée : une matrice extraite seule de
    /// l'archive reste qualifiée. Le texte libre (espaces, tiret cadratin,
    /// `=`, `%`) traverse sans rendre le préambule ambigu.
    #[test]
    fn survey_context_survives_the_project_round_trip() {
        let dir = unique_temp_dir("test_context_roundtrip").unwrap();
        let dest = dir.join("relevé qualifié.lunar");
        write_sample_project(&dest, PROJECT_SCHEMA_VERSION);

        let extracted = extract_project_archive(&dest).unwrap();
        let from_manifest: lunar_flows_core::lfms::SurveyContext =
            (&extracted.manifest.survey_context).into();
        assert_eq!(
            from_manifest,
            sample_context(),
            "le manifest doit rendre le contexte exactement tel qu'il a été saisi"
        );

        let preamble = lunar_flows_core::lfms::read_preamble(&extracted.matrix_csv)
            .unwrap()
            .expect("la matrice embarquée doit porter un préambule");
        assert_eq!(
            preamble.context,
            sample_context(),
            "le préambule LFMS de la matrice doit porter le même contexte que le manifest"
        );
        assert_eq!(preamble.version, lunar_flows_core::lfms::LFMS_VERSION);

        fs::remove_dir_all(&dir).ok();
    }

    /// Un projet écrit sans contexte reste indiscernable d'un relevé non
    /// qualifié : pas de clés vides dans le manifest, pas de contexte dans le
    /// préambule.
    #[test]
    fn a_project_without_context_stays_clean() {
        let dir = unique_temp_dir("test_context_absent").unwrap();
        let dest = dir.join("sans contexte.lunar");
        let empty = lunar_flows_core::lfms::SurveyContext::default();
        write_sample_project_with_context(&dest, PROJECT_SCHEMA_VERSION, &empty);

        let extracted = extract_project_archive(&dest).unwrap();
        assert_eq!(
            extracted.manifest.survey_context,
            ManifestSurveyContext::default()
        );

        let manifest_json =
            serde_json::to_value(&extracted.manifest).expect("manifest sérialisable");
        let context_json = &manifest_json["survey_context"];
        assert!(
            context_json.as_object().is_some_and(|map| map.is_empty()),
            "un contexte vide ne doit émettre aucune clé, trouvé: {context_json}"
        );

        let preamble = lunar_flows_core::lfms::read_preamble(&extracted.matrix_csv)
            .unwrap()
            .expect("préambule présent");
        assert!(
            preamble.context.is_empty(),
            "aucun contexte ne doit apparaître dans le préambule"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_labels_and_capture_entries_are_tolerated() {
        let dir = unique_temp_dir("test_minimal").unwrap();
        let dest = dir.join("minimal.lunar");

        let file = File::create(&dest).unwrap();
        let mut zip = ZipWriter::new(file);
        let options = SimpleFileOptions::default();
        zip.start_file(MANIFEST_ENTRY, options).unwrap();
        zip.write_all(
            serde_json::to_string(&sample_manifest(PROJECT_SCHEMA_VERSION))
                .unwrap()
                .as_bytes(),
        )
        .unwrap();
        zip.start_file(MATRIX_ENTRY, options).unwrap();
        zip.write_all(lunar_flows_core::lfms::format_preamble(None).as_bytes())
            .unwrap();
        zip.finish().unwrap();

        let extracted = extract_project_archive(&dest).unwrap();
        assert!(extracted.labels_csv.is_none());
        assert!(extracted.capture.is_none());

        fs::remove_dir_all(&dir).ok();
    }

    /// Attend qu'une condition devienne vraie, dans une borne généreuse.
    ///
    /// Délai large : les CSV de l'archive sont écrits par le cœur avec
    /// flush + `fsync` (`lunar-flows-core`, `matrix.rs`), ce qui prend
    /// plusieurs secondes sur un disque chargé (2 s mesurées pendant des
    /// builds en parallèle, le 19/11) ; un test qui réussit n'attend pas plus
    /// que nécessaire.
    fn wait_until(condition: impl Fn() -> bool) -> bool {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while std::time::Instant::now() < deadline {
            if condition() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        condition()
    }

    /// L'autosave se démarre et s'arrête explicitement. Un relevé
    /// modifié est écrit en projet lisible, puis la fermeture propre arrête
    /// le thread et retire la sentinelle — plus de récupération proposée.
    #[test]
    fn autosave_writes_a_modified_survey_and_stop_removes_the_sentinel() {
        let data_dir = unique_temp_dir("test_autosave_cycle").unwrap();
        let matrix = Arc::new(Mutex::new(FlowMatrix::new()));
        let capture_state = Arc::new(Mutex::new(CaptureState::new()));
        capture_state.lock().unwrap().mark_dirty();

        let session = SessionPersistence::start_with_interval(
            &data_dir,
            Arc::clone(&matrix),
            Arc::clone(&capture_state),
            std::time::Duration::from_millis(10),
        )
        .unwrap();
        let dir = autosave_dir(&data_dir);
        assert!(dir.join(SESSION_LOCK_FILE).is_file(), "sentinelle posée");
        assert!(
            wait_until(|| dir.join(AUTOSAVE_FILE).is_file()),
            "le relevé modifié doit être autosauvegardé"
        );
        assert!(
            wait_until(|| extract_project_archive(&dir.join(AUTOSAVE_FILE)).is_ok()),
            "l'autosave est un projet lisible"
        );
        assert_eq!(
            RecoveryOffer::detect(&data_dir),
            RecoveryOffer(Some(dir.join(AUTOSAVE_FILE))),
            "tant que la session tourne, un crash laisserait la récupération"
        );

        session.stop();
        assert!(
            !dir.join(SESSION_LOCK_FILE).exists(),
            "fermeture propre : sentinelle retirée"
        );
        assert_eq!(RecoveryOffer::detect(&data_dir), RecoveryOffer(None));

        fs::remove_dir_all(&data_dir).ok();
    }

    /// Un abandon sans fermeture propre (drop) arrête le thread mais garde
    /// la sentinelle : le démarrage suivant propose la récupération.
    #[test]
    fn dropping_the_session_without_stop_keeps_the_recovery_offer() {
        let data_dir = unique_temp_dir("test_autosave_drop").unwrap();
        let session = SessionPersistence::start(
            &data_dir,
            Arc::new(Mutex::new(FlowMatrix::new())),
            Arc::new(Mutex::new(CaptureState::new())),
        )
        .unwrap();
        let dir = autosave_dir(&data_dir);
        fs::write(dir.join(AUTOSAVE_FILE), b"zip").unwrap();

        drop(session);

        assert!(dir.join(SESSION_LOCK_FILE).is_file());
        assert_eq!(
            get_recovery_offer(&RecoveryOffer::detect(&data_dir)),
            Some(dir.join(AUTOSAVE_FILE).to_string_lossy().into_owned())
        );

        fs::remove_dir_all(&data_dir).ok();
    }
}
