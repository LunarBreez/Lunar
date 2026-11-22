//! Moteur de LUNAR : capture en direct, imports PCAP, matrice et labels,
//! projet `.lunar`, exports.
//!
//! L'interface passe par la façade [`Lunar`], une méthode par opération ;
//! les événements lui sont livrés, possédés, par un [`ipc::Channel`].
//!
//! - Les états partagés sont des `Arc<Mutex<…>>`, verrouillés dans un ordre
//!   global (voir [`Lunar`]) ;
//! - [`AppPaths`] donne les répertoires ; les états sont passés
//!   explicitement ;
//! - [`Lunar::new`] prépare le moteur ; l'autosave et la sentinelle de
//!   session passent par [`Lunar::start_autosave`] / [`Lunar::stop_autosave`] ;
//! - l'usage CPU vient de [`setup::system_info::CpuSampler`], interrogé par
//!   l'interface.

use std::sync::{Arc, Mutex};

use log::{error, info};

use crate::{
    commandes::{
        export::synthesis::SynthesisExport,
        flow_matrix::{LabelRowStatus, SurveyPeriodView},
        import::{
            LabelConflictStore,
            labels::{
                conflicts::{LabelConflictReport, LabelImportReport, LabelResolution},
                winaudit::WinAuditImportReport,
            },
        },
        project::{RecoveryOffer, SessionPersistence},
    },
    dto::NetDevice,
    errors::{CaptureStateError, capture_error::CaptureError},
    events::CaptureEvent,
    ipc::Channel,
    state::{
        capture::{CaptureState, capture_config::CaptureConfig, capture_status::CaptureStatus},
        flow_matrix::FlowMatrix,
        graph::{GraphData, GraphUpdate},
        labels_list::{LabelStore, PcInfoLabel},
    },
};

pub mod commandes;
pub mod dto;
pub mod errors;
pub mod events;
pub mod ipc;
pub mod npcap;
pub mod paths;
pub mod setup;
pub mod state;
pub mod utils;

/// Nom du produit, écrit dans le journal, la synthèse et « À propos ».
pub const APP_NAME: &str = "LUNAR";

pub use paths::AppPaths;
// Exposé pour les mesures du pool de buffers hors du crate.
pub use state::capture::capture_handle::threads::packet_buffer;

/// Façade du moteur : possède l'état partagé du relevé et expose une
/// méthode par opération de l'interface.
///
/// Les méthodes sont bloquantes (imports, ouverture de projet, arrêt de
/// capture) : l'interface les appelle hors de son thread de rendu. `Lunar`
/// est `Send + Sync` et se partage derrière un `Arc`. Les commandes qui
/// émettent des événements prennent un `Channel<CaptureEvent>` : c'est par
/// lui que partent `Started`, la progression, les snapshots de graphe…
///
/// Ordre global des verrous : `CaptureState`
/// n'est jamais tenu pendant une jointure ni avec un état de données ; les
/// données se verrouillent dans l'ordre matrice → graphe → labels →
/// labels « pc lunar » → stores auxiliaires.
pub struct Lunar {
    /// Répertoires de configuration, de données et de journaux.
    paths: AppPaths,
    /// Machine d'état de la capture, configuration, filtre, suivi dirty.
    capture_state: Arc<Mutex<CaptureState>>,
    /// Matrice de flux du relevé courant.
    flow_matrix: Arc<Mutex<FlowMatrix>>,
    /// Graphe du relevé courant.
    graph: Arc<Mutex<GraphData>>,
    /// Labels « pc lunar » générés au démarrage depuis les interfaces locales.
    pcinfo: Arc<Mutex<PcInfoLabel>>,
    /// Store de labels, source de vérité unique.
    label_store: Arc<Mutex<LabelStore>>,
    /// Conflits de labels en attente d'arbitrage.
    label_conflicts: Arc<Mutex<LabelConflictStore>>,
    /// Offre de récupération figée à la construction.
    recovery_offer: RecoveryOffer,
    /// Autosave et sentinelle de session, une fois démarrés.
    session: Mutex<Option<SessionPersistence>>,
}

// `Lunar` doit pouvoir être partagé entre le thread de rendu et les tâches
// de fond de l'interface.
const _: () = {
    /// Vérifié à la compilation.
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Lunar>();
};

impl Lunar {
    /// Construit le moteur : bannière et instantané d'environnement dans les logs,
    /// configuration de capture persistée relue, offre de récupération
    /// calculée (AVANT toute pose de sentinelle), labels « pc lunar » tirés
    /// des interfaces réseau locales.
    ///
    /// Le moniteur CPU est à part ([`setup::system_info::CpuSampler`]).
    /// L'autosave ne démarre qu'avec [`Self::start_autosave`].
    pub fn new(paths: AppPaths) -> Result<Self, CaptureStateError> {
        info!("{}", setup::print_banner());
        setup::log_host_and_app_snapshot(&paths);

        let lunar = Self {
            capture_state: Arc::new(Mutex::new(CaptureState::new())),
            flow_matrix: Arc::new(Mutex::new(FlowMatrix::new())),
            graph: Arc::new(Mutex::new(GraphData::new())),
            pcinfo: Arc::new(Mutex::new(PcInfoLabel::new())),
            label_store: Arc::new(Mutex::new(LabelStore::new())),
            label_conflicts: Arc::new(Mutex::new(LabelConflictStore::default())),
            recovery_offer: RecoveryOffer::detect(&paths.data_dir),
            session: Mutex::new(None),
            paths,
        };
        lunar.restore_persisted_capture_config();

        let interfaces = setup::system_info::get_interfaces();
        setup::labels::create_labels_from_network_interfaces(interfaces, &lunar.pcinfo)?;
        Ok(lunar)
    }

    /// Recharge la configuration de capture persistée, si présente.
    fn restore_persisted_capture_config(&self) {
        match CaptureConfig::load_persisted(&self.paths.config_dir) {
            Ok(Some(config)) => match self.capture_state.lock() {
                Ok(mut state) => {
                    info!("Configuration capture chargée depuis le disque");
                    state.config = config;
                }
                Err(err) => error!("Impossible de charger la configuration capture: {err}"),
            },
            Ok(None) => {}
            Err(err) => error!("Configuration capture persistée ignorée: {err}"),
        }
    }

    /// Répertoires du moteur.
    pub fn paths(&self) -> &AppPaths {
        &self.paths
    }

    /// Pose la sentinelle de session et démarre l'autosave. Sans effet s'ils
    /// tournent déjà.
    pub fn start_autosave(&self) -> Result<(), CaptureStateError> {
        let mut session = self.session.lock()?;
        if session.is_none() {
            *session = Some(SessionPersistence::start(
                &self.paths.data_dir,
                Arc::clone(&self.flow_matrix),
                Arc::clone(&self.capture_state),
            )?);
        }
        Ok(())
    }

    /// Fermeture propre : arrête
    /// l'autosave et retire la sentinelle, le prochain démarrage ne
    /// proposera pas de récupération. Sans effet si l'autosave n'a pas été
    /// démarré. Un `Lunar` simplement abandonné arrête le thread mais laisse
    /// la sentinelle, comme un crash.
    pub fn stop_autosave(&self) -> Result<(), CaptureStateError> {
        let session = self.session.lock()?.take();
        if let Some(session) = session {
            session.stop();
        }
        Ok(())
    }

    // --- Capture live ---

    /// Démarre une capture live.
    pub fn start_capture(
        &self,
        on_event: Channel<CaptureEvent>,
    ) -> Result<CaptureStatus, CaptureStateError> {
        commandes::net_capture::start_capture(
            &self.capture_state,
            on_event,
            &self.flow_matrix,
            &self.graph,
            &self.label_store,
            &self.pcinfo,
        )
    }

    /// Arrête la capture en cours.
    pub fn stop_capture(
        &self,
        on_event: Channel<CaptureEvent>,
    ) -> Result<CaptureStatus, CaptureStateError> {
        commandes::net_capture::stop_capture(&self.capture_state, on_event)
    }

    /// Valide, applique et persiste une configuration de capture.
    pub fn config_capture(
        &self,
        device_name: String,
        buffer_size: i32,
        chan_capacity: i32,
        timeout: i32,
        snaplen: i32,
    ) -> Result<CaptureConfig, CaptureStateError> {
        commandes::net_capture::config_capture(
            &self.capture_state,
            &self.paths.config_dir,
            device_name,
            buffer_size,
            chan_capacity,
            timeout,
            snaplen,
        )
    }

    /// Configuration de capture courante.
    pub fn get_config_capture(&self) -> Result<CaptureConfig, CaptureStateError> {
        commandes::net_capture::get_config_capture(&self.capture_state)
    }

    /// Vide la matrice, le graphe et le contexte du relevé.
    pub fn reset_capture(&self) -> Result<(), CaptureStateError> {
        commandes::net_capture::reset_capture(&self.flow_matrix, &self.graph, &self.capture_state)
    }

    /// Enregistre le filtre BPF du prochain démarrage.
    pub fn set_filter(&self, filter: String) -> Result<(), CaptureStateError> {
        commandes::net_capture::set_filter(&self.capture_state, filter)
    }

    /// Interfaces réseau visibles par pcap.
    pub fn get_devices_list(&self) -> Result<Vec<NetDevice>, CaptureError> {
        commandes::net_interface::get_devices_list()
    }

    // --- Imports ---

    /// Remplace le relevé par la conversion des fichiers PCAP.
    pub fn convert_from_pcap_list(
        &self,
        pcap_paths: Vec<String>,
        on_event: Channel<CaptureEvent>,
    ) -> Result<(), CaptureStateError> {
        commandes::import::convert_from_pcap_list(
            &self.flow_matrix,
            &self.graph,
            &self.label_store,
            &self.capture_state,
            pcap_paths,
            on_event,
        )
    }

    /// Demande l'annulation de l'import PCAP en cours.
    pub fn cancel_import(&self) -> Result<bool, CaptureStateError> {
        commandes::import::cancel_import(&self.capture_state)
    }

    /// Remplace le relevé par la fusion des matrices CSV/XLSX.
    pub fn import_matrix_files(
        &self,
        incoming_file_paths: Vec<String>,
        on_event: Channel<CaptureEvent>,
    ) -> Result<(), CaptureStateError> {
        commandes::import::import_matrix_files(
            incoming_file_paths,
            &self.flow_matrix,
            &self.graph,
            &self.label_store,
            &self.capture_state,
            on_event,
        )
    }

    /// Variante à un seul fichier.
    pub fn import_matrix_file(
        &self,
        incoming_file_path: String,
        on_event: Channel<CaptureEvent>,
    ) -> Result<(), CaptureStateError> {
        commandes::import::import_matrix_file(
            incoming_file_path,
            &self.flow_matrix,
            &self.graph,
            &self.label_store,
            &self.capture_state,
            on_event,
        )
    }

    /// Vrai si la matrice courante n'a aucune ligne.
    pub fn is_matrix_empty(&self) -> Result<bool, CaptureStateError> {
        commandes::import::is_matrix_empty(&self.flow_matrix)
    }

    // --- Exports ---

    /// Exporte la matrice en CSV.
    pub fn export_csv(&self, path: String) -> Result<(), CaptureStateError> {
        commandes::export::csv::export_csv(&self.flow_matrix, path)
    }

    /// Exporte en CSV les seuls flux des arêtes `edge_keys` du graphe (clés
    /// de la map des arêtes, celles des lignes du tableau des flux) ;
    /// `description` décrit le filtre et part dans le préambule du fichier.
    /// Retourne le nombre de lignes écrites.
    pub fn export_csv_filtered(
        &self,
        path: String,
        edge_keys: &std::collections::HashSet<String>,
        description: &str,
    ) -> Result<usize, CaptureStateError> {
        commandes::export::csv::export_csv_filtered(&self.flow_matrix, path, edge_keys, description)
    }

    /// Filtres des exports partiels dont le relevé courant est issu ; vide
    /// si rien n'indique qu'il soit incomplet.
    pub fn partial_filters(&self) -> Result<Vec<String>, CaptureStateError> {
        Ok(self.flow_matrix.lock()?.partial.iter().cloned().collect())
    }

    /// Écrit le CSV voisin d'une synthèse HTML et rend de quoi composer la
    /// page.
    pub fn export_synthesis_matrix(
        &self,
        html_path: String,
    ) -> Result<SynthesisExport, CaptureStateError> {
        commandes::export::synthesis::export_synthesis_matrix(
            &self.flow_matrix,
            &self.graph,
            html_path,
        )
    }

    /// Exporte les labels courants en CSV.
    pub fn export_label_file(&self, path: String) -> Result<(), CaptureStateError> {
        commandes::export::labels::export_label_file(&self.flow_matrix, path)
    }

    /// Exporte le fichier de labels « à compléter ».
    pub fn export_labels_to_complete(&self, path: String) -> Result<usize, CaptureStateError> {
        commandes::export::labels::export_labels_to_complete(&self.flow_matrix, path)
    }

    /// Archive les journaux de [`AppPaths::log_dir`] en ZIP.
    pub fn export_logs(&self, destination: String) -> Result<String, CaptureStateError> {
        commandes::export::logs::export_logs(&self.paths.log_dir, destination)
    }

    // --- Labels ---

    /// Contenu du store de labels.
    pub fn get_label_rows(&self) -> Result<Vec<(String, String, String)>, CaptureStateError> {
        commandes::import::get_label_rows(&self.label_store)
    }

    /// Vide le store de labels et resynchronise.
    pub fn clear_label_store(&self) -> Result<Vec<GraphUpdate>, CaptureStateError> {
        commandes::import::clear_label_store(
            &self.label_store,
            &self.pcinfo,
            &self.flow_matrix,
            &self.graph,
            &self.capture_state,
        )
    }

    /// Remplace le store par un fichier de labels CSV.
    pub fn import_label_file(
        &self,
        incoming_file_path: String,
        on_event: Channel<CaptureEvent>,
    ) -> Result<LabelImportReport, CaptureStateError> {
        commandes::import::import_label_file(
            incoming_file_path,
            &self.label_store,
            &self.pcinfo,
            &self.flow_matrix,
            &self.graph,
            &self.capture_state,
            &self.label_conflicts,
            on_event,
        )
    }

    /// Conflits de labels en attente.
    pub fn get_label_conflicts(&self) -> Result<Vec<LabelConflictReport>, CaptureStateError> {
        commandes::import::get_label_conflicts(&self.label_conflicts)
    }

    /// Applique des arbitrages de conflits.
    pub fn resolve_label_conflicts(
        &self,
        resolutions: Vec<LabelResolution>,
    ) -> Result<Vec<LabelConflictReport>, CaptureStateError> {
        commandes::import::resolve_label_conflicts(
            resolutions,
            &self.label_store,
            &self.flow_matrix,
            &self.graph,
            &self.capture_state,
            &self.label_conflicts,
        )
    }

    /// Ajoute au store les labels de relevés WinAudit.
    pub fn import_winaudit_surveys(
        &self,
        paths: Vec<String>,
        exclude_virtual: bool,
        on_event: Channel<CaptureEvent>,
    ) -> Result<WinAuditImportReport, CaptureStateError> {
        commandes::import::import_winaudit_surveys(
            paths,
            exclude_virtual,
            &self.label_store,
            &self.pcinfo,
            &self.flow_matrix,
            &self.graph,
            &self.capture_state,
            &self.label_conflicts,
            on_event,
        )
    }

    /// Ajoute ou remplace le label de `(mac, ip)`.
    pub fn add_label(
        &self,
        mac: String,
        ip: String,
        label: String,
    ) -> Result<Vec<GraphUpdate>, CaptureStateError> {
        commandes::flow_matrix::add_label(
            &self.flow_matrix,
            &self.graph,
            &self.label_store,
            &self.pcinfo,
            &self.capture_state,
            mac,
            ip,
            label,
        )
    }

    /// Modifie une ligne du store, clé comprise.
    pub fn update_label(
        &self,
        old_mac: String,
        old_ip: String,
        mac: String,
        ip: String,
        label: String,
    ) -> Result<Vec<GraphUpdate>, CaptureStateError> {
        commandes::flow_matrix::update_label(
            &self.flow_matrix,
            &self.graph,
            &self.label_store,
            &self.pcinfo,
            &self.capture_state,
            old_mac,
            old_ip,
            mac,
            ip,
            label,
        )
    }

    /// Supprime la ligne `(mac, ip)` du store.
    pub fn delete_label(
        &self,
        mac: String,
        ip: String,
    ) -> Result<Vec<GraphUpdate>, CaptureStateError> {
        commandes::flow_matrix::delete_label(
            &self.flow_matrix,
            &self.graph,
            &self.label_store,
            &self.pcinfo,
            &self.capture_state,
            mac,
            ip,
        )
    }

    /// Lignes du store avec leur statut observé/dormant.
    pub fn get_labels_with_status(&self) -> Result<Vec<LabelRowStatus>, CaptureStateError> {
        commandes::flow_matrix::get_labels_with_status(&self.flow_matrix, &self.label_store)
    }

    /// Labels connus de la matrice.
    pub fn get_label_list(&self) -> Result<Vec<String>, CaptureStateError> {
        commandes::flow_matrix::get_label_list(&self.flow_matrix)
    }

    /// Labels appliqués à la matrice, complétés.
    pub fn get_matrix_labels(&self) -> Result<Vec<(String, String, String)>, CaptureStateError> {
        commandes::flow_matrix::get_matrix_labels(&self.flow_matrix)
    }

    // --- Vues ---

    /// Snapshot complet du graphe.
    pub fn get_graph_snapshot(&self) -> Result<GraphData, CaptureStateError> {
        commandes::flow_matrix::get_graph_snapshot(&self.graph)
    }

    /// Période observée du relevé.
    pub fn get_survey_period(&self) -> Result<SurveyPeriodView, CaptureStateError> {
        commandes::flow_matrix::get_survey_period(&self.flow_matrix)
    }

    // --- Projet ---

    /// Enregistre le relevé comme projet `.lunar`.
    pub fn save_project(&self, path: String) -> Result<(), CaptureStateError> {
        commandes::project::save_project(&self.flow_matrix, &self.capture_state, path)
    }

    /// Remplace le relevé par un projet `.lunar`.
    pub fn open_project(
        &self,
        path: String,
        on_event: Channel<CaptureEvent>,
    ) -> Result<(), CaptureStateError> {
        commandes::project::open_project(
            &self.paths.config_dir,
            &self.flow_matrix,
            &self.graph,
            &self.label_store,
            &self.pcinfo,
            &self.capture_state,
            &self.label_conflicts,
            on_event,
            path,
        )
    }

    /// Vrai si des données seraient perdues sans enregistrement.
    pub fn is_session_dirty(&self) -> Result<bool, CaptureStateError> {
        commandes::project::is_session_dirty(&self.capture_state)
    }

    /// Autosave de la session précédente si elle s'est terminée par un crash.
    pub fn get_recovery_offer(&self) -> Option<String> {
        commandes::project::get_recovery_offer(&self.recovery_offer)
    }
}

/// Aides partagées par les tests du crate.
#[cfg(test)]
pub(crate) mod test_support {
    use std::path::{Path, PathBuf};

    /// Fichiers de test du dépôt (`fixtures/`).
    pub(crate) fn fixtures() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
    }
}

#[cfg(test)]
mod tests {
    //! Tests de la façade : les commandes y sont jouées comme l'interface les
    //! appellera, sur des répertoires temporaires.
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Racine temporaire propre à un test, supprimée à la fin.
    struct TempRoot(PathBuf);

    impl TempRoot {
        /// Crée une racine unique pour `label`.
        fn new(label: &str) -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let path = std::env::temp_dir().join(format!(
                "lunar_engine_{label}_{}_{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        /// Répertoires du moteur sous cette racine.
        fn paths(&self) -> AppPaths {
            AppPaths::under(&self.0)
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn lunar_is_send_and_sync_and_starts_clean() {
        let root = TempRoot::new("clean");
        let lunar = Lunar::new(root.paths()).unwrap();
        assert!(!lunar.is_session_dirty().unwrap());
        assert!(lunar.is_matrix_empty().unwrap());
        assert_eq!(lunar.get_recovery_offer(), None);
        assert_eq!(lunar.paths(), &root.paths());
        let shared = Arc::new(lunar);
        let from_thread = Arc::clone(&shared);
        std::thread::spawn(move || from_thread.get_label_rows().unwrap())
            .join()
            .unwrap();
    }

    /// Parcours complet par la façade : import de matrice (événements
    /// compris), enregistrement, remise à zéro, réouverture identique.
    #[test]
    fn import_save_reset_open_through_the_facade() {
        let root = TempRoot::new("round_trip");
        let lunar = Lunar::new(root.paths()).unwrap();
        let source = test_support::fixtures().join("20260703_NP_Matrice.csv");

        let (on_event, events) = ipc::test_support::json_recorder();
        lunar
            .import_matrix_files(vec![source.to_string_lossy().into_owned()], on_event)
            .unwrap();
        let tags: Vec<String> = events
            .lock()
            .unwrap()
            .iter()
            .map(|event| event["event"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            tags,
            [
                "importProgress",
                "started",
                "graphSnapshot",
                "finished",
                "stats"
            ],
            "séquence d'événements d'un import de matrice"
        );
        assert!(lunar.is_session_dirty().unwrap());
        let rows_before = lunar.flow_matrix.lock().unwrap().to_flat_vec();
        assert_eq!(rows_before.len(), 30);
        let nodes_before = lunar.get_graph_snapshot().unwrap().nodes.len();

        let project = root.0.join("relevé.lunar");
        lunar
            .save_project(project.to_string_lossy().into_owned())
            .unwrap();
        assert!(!lunar.is_session_dirty().unwrap());

        lunar.reset_capture().unwrap();
        assert!(lunar.is_matrix_empty().unwrap());

        lunar
            .open_project(project.to_string_lossy().into_owned(), Channel::new(|_| {}))
            .unwrap();
        assert_eq!(lunar.flow_matrix.lock().unwrap().to_flat_vec(), rows_before);
        assert_eq!(
            lunar.get_graph_snapshot().unwrap().nodes.len(),
            nodes_before
        );
        assert!(!lunar.is_session_dirty().unwrap());
    }

    /// Le manifest d'un projet écrit par le moteur suit le schéma v2, avec la
    /// version d'application du moteur.
    #[test]
    fn saved_project_keeps_the_v2_format() {
        let root = TempRoot::new("format");
        let lunar = Lunar::new(root.paths()).unwrap();
        let project = root.0.join("format.lunar");
        lunar
            .save_project(project.to_string_lossy().into_owned())
            .unwrap();

        let mut archive = zip::ZipArchive::new(std::fs::File::open(&project).unwrap()).unwrap();
        let names: Vec<String> = archive
            .file_names()
            .map(|name| name.unwrap().into_owned())
            .collect();
        for entry in ["manifest.json", "matrice.csv", "labels.csv", "capture.json"] {
            assert!(names.iter().any(|name| name == entry), "entrée {entry}");
        }
        let manifest: serde_json::Value =
            serde_json::from_reader(archive.by_name("manifest.json").unwrap()).unwrap();
        assert_eq!(manifest["schema_version"], 2);
        assert_eq!(manifest["app_version"], env!("CARGO_PKG_VERSION"));
        let keys: Vec<&str> = manifest
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys.len(), 8, "champs du manifest v2 : {keys:?}");
    }

    /// `config_capture` persiste dans `config_dir`, que le moteur suivant
    /// relit à la construction (le `restore_persisted_capture_config` du setup).
    #[test]
    fn capture_config_survives_a_restart() {
        let root = TempRoot::new("config");
        let lunar = Lunar::new(root.paths()).unwrap();
        lunar
            .config_capture("eth-test".to_string(), 1_048_576, 512, 30, 2048)
            .unwrap();
        drop(lunar);

        let restarted = Lunar::new(root.paths()).unwrap();
        let config = restarted.get_config_capture().unwrap();
        assert_eq!(config.device_name, "eth-test");
        assert_eq!(config.chan_capacity, 512);
        assert_eq!(config.snaplen, 2048);

        let refused = restarted
            .config_capture(String::new(), 1_048_576, 512, 30, 2048)
            .expect_err("une interface vide est refusée");
        assert_eq!(serde_json::to_value(&refused).unwrap()["kind"], "capture");
    }

    /// L'autosave ne démarre qu'à la demande ; la fermeture propre retire la
    /// sentinelle, un abandon la laisse et le démarrage suivant propose la
    /// récupération.
    #[test]
    fn autosave_lifecycle_through_the_facade() {
        let root = TempRoot::new("autosave");
        let paths = root.paths();
        let lock = commandes::project::autosave_dir(&paths.data_dir).join("session.lock");

        let lunar = Lunar::new(paths.clone()).unwrap();
        assert!(!lock.exists(), "rien n'est posé avant start_autosave");
        lunar.start_autosave().unwrap();
        lunar.start_autosave().unwrap();
        assert!(lock.is_file());
        lunar.stop_autosave().unwrap();
        assert!(!lock.exists());
        lunar.stop_autosave().unwrap();

        lunar.start_autosave().unwrap();
        let autosave = commandes::project::autosave_dir(&paths.data_dir)
            .join(commandes::project::AUTOSAVE_FILE);
        std::fs::write(&autosave, b"zip").unwrap();
        drop(lunar);
        assert!(lock.is_file(), "un abandon laisse la sentinelle");

        let restarted = Lunar::new(paths).unwrap();
        assert_eq!(
            restarted.get_recovery_offer(),
            Some(autosave.to_string_lossy().into_owned())
        );
    }

    #[test]
    fn label_commands_through_the_facade() {
        let root = TempRoot::new("labels");
        let lunar = Lunar::new(root.paths()).unwrap();
        let source = test_support::fixtures().join("20260703_NP_Matrice.csv");
        lunar
            .import_matrix_files(
                vec![source.to_string_lossy().into_owned()],
                Channel::new(|_| {}),
            )
            .unwrap();
        let (mac, ip) = lunar
            .flow_matrix
            .lock()
            .unwrap()
            .observed_endpoints()
            .into_iter()
            .find(|(mac, ip)| !mac.is_empty() && !ip.is_empty())
            .unwrap();

        let updates = lunar
            .add_label(mac.clone(), ip.clone(), "automate".to_string())
            .unwrap();
        assert!(!updates.is_empty(), "le nœud de l'équipement est renommé");
        assert!(
            lunar
                .get_labels_with_status()
                .unwrap()
                .iter()
                .any(|row| row.label == "automate" && row.observed)
        );
        assert!(
            lunar
                .get_label_list()
                .unwrap()
                .contains(&"automate".to_string())
        );

        lunar
            .update_label(
                mac.clone(),
                ip.clone(),
                mac.clone(),
                ip.clone(),
                "automate 2".to_string(),
            )
            .unwrap();
        assert!(
            lunar
                .get_label_rows()
                .unwrap()
                .iter()
                .any(|(_, _, label)| label == "automate 2")
        );
        lunar.delete_label(mac, ip).unwrap();
        assert!(
            !lunar
                .get_label_rows()
                .unwrap()
                .iter()
                .any(|(_, _, label)| label.starts_with("automate"))
        );
    }

    /// `export_logs` archive `AppPaths::log_dir` ; sans ce dossier, l'erreur
    /// garde son `kind` structuré.
    #[test]
    fn export_logs_reads_the_log_dir() {
        let root = TempRoot::new("logs");
        let paths = root.paths();
        let lunar = Lunar::new(paths.clone()).unwrap();
        let destination = root.0.join("support.zip");

        let missing = lunar
            .export_logs(destination.to_string_lossy().into_owned())
            .expect_err("pas de dossier de journaux");
        assert_eq!(
            serde_json::to_value(&missing).unwrap(),
            serde_json::json!({"kind": "export", "message": {"kind": "logNotFound"}})
        );

        std::fs::create_dir_all(&paths.log_dir).unwrap();
        std::fs::write(paths.log_dir.join("lunar.log"), "ligne\n").unwrap();
        lunar
            .export_logs(destination.to_string_lossy().into_owned())
            .unwrap();
        let archive = zip::ZipArchive::new(std::fs::File::open(&destination).unwrap()).unwrap();
        assert!(
            archive
                .file_names()
                .any(|name| name.unwrap().ends_with("lunar.log"))
        );
    }
}
