//! Store central de la capture : état courant (capture en cours, verrou
//! d'import, filtre, compteurs, bilan d'import) et tri des événements du
//! moteur.
//!
//! - Pas de listes d'abonnés : [`CaptureStore::handle_event`] met l'état à
//!   jour puis rend un [`StoreEffect`] que l'appelant relaie (graphe,
//!   journal de paquets, jauge du canal).
//! - Le canal d'événements n'est pas tenu ici : la coquille gpui filtre les
//!   canaux détachés avant d'appeler le store.
//! - L'appel au moteur `is_matrix_empty` reste côté interface :
//!   [`CaptureStore::set_matrix_empty`] applique le résultat.
//! - Le `match` sur [`CaptureEvent`] est exhaustif : une variante ajoutée
//!   au moteur casse la compilation.

use log::{info, warn};
use lunar_engine::events::CaptureEvent;
use lunar_engine::state::capture::capture_handle::messages::capture::CapturedPacketOwned;
use lunar_engine::state::capture::capture_status::CaptureStatus;
use lunar_engine::state::graph::{GraphData, GraphUpdate};

use crate::import_progress::ImportProgress;

/// Version du contrat d'événements attendue par cette interface, comparée à
/// celle de `Started` (voir `lunar_engine::events::CAPTURE_EVENT_PROTOCOL_VERSION`).
/// Un écart ne bloque rien : il est journalisé.
pub const EXPECTED_PROTOCOL_VERSION: u32 = 1;

/// Compteurs affichés par la barre de statut, en capture comme en import
/// (`StatsView`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StatsView {
    /// Trames reçues par la carte réseau (ou lues du fichier importé).
    pub received: u64,
    /// Trames perdues côté noyau.
    pub dropped: u64,
    /// Trames perdues au niveau de l'interface.
    pub if_dropped: u64,
    /// Trames perdues côté application (pool ou canal saturé).
    pub app_dropped: u64,
    /// Rejets du parseur : capture coupée au snaplen.
    pub rejected_truncated: u64,
    /// Rejets du parseur : type de liaison inconnu.
    pub rejected_unsupported_link_type: u64,
    /// Rejets du parseur : trame malformée.
    pub rejected_malformed: u64,
    /// Flux distincts de la matrice.
    pub processed: u64,
}

impl StatsView {
    /// Total des rejets du parseur, affiché à côté de 🧩 dans la barre
    /// d'état.
    pub fn rejected_total(&self) -> u64 {
        self.rejected_truncated + self.rejected_unsupported_link_type + self.rejected_malformed
    }

    /// Détail des rejets, bulle du compteur 🧩.
    pub fn rejected_detail(&self) -> String {
        format!(
            "Rejets du parseur : {} tronqués, {} DLT non supportés, {} malformés",
            self.rejected_truncated, self.rejected_unsupported_link_type, self.rejected_malformed
        )
    }
}

/// Bilan du dernier import, catégories fines comprises.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LastImport {
    /// Chemin du fichier importé.
    pub file_name: String,
    /// Paquets (ou lignes) intégrés à la matrice.
    pub integrated_count: usize,
    /// Paquets (ou lignes) lus.
    pub total_count: usize,
    /// Rejetés : capture coupée au snaplen.
    pub truncated_count: usize,
    /// Rejetés : type de liaison inconnu.
    pub unsupported_link_type_count: usize,
    /// Rejetés : trame malformée.
    pub malformed_count: usize,
}

impl LastImport {
    /// Total des rejets.
    pub fn rejected_total(&self) -> usize {
        self.truncated_count + self.unsupported_link_type_count + self.malformed_count
    }

    /// Texte du badge d'import de la barre de statut.
    pub fn badge_text(&self) -> String {
        let rejected = self.rejected_total();
        let suffix = match rejected {
            0 => String::new(),
            1 => ", 1 rejetée".to_owned(),
            n => format!(", {n} rejetées"),
        };
        format!(
            "Import : {} ({} lignes{suffix})",
            self.file_name, self.total_count
        )
    }

    /// Équation du bilan, bulle du badge.
    pub fn report_detail(&self) -> String {
        format!(
            "{} : {} lus = {} intégrés + {} tronqués + {} DLT non supportés + {} malformés",
            self.file_name,
            self.total_count,
            self.integrated_count,
            self.truncated_count,
            self.unsupported_link_type_count,
            self.malformed_count
        )
    }
}

/// Occupation du canal capture → traitement, en pourcentage arrondi borné à
/// 100.
pub fn channel_occupancy_percent(channel_size: usize, current_size: usize) -> u32 {
    if channel_size == 0 {
        return 0;
    }
    let ratio = (current_size as f64 * 100.0 / channel_size as f64).min(100.0);
    ratio.round() as u32
}

/// Ce que l'appelant doit relayer après un événement accepté : l'état du
/// store est déjà à jour.
#[derive(Debug)]
pub enum StoreEffect {
    /// `Started` traité (filtre en attente promu, type de liaison connu).
    Started,
    /// `Stats` traité : compteurs à jour.
    Stats,
    /// `Stopped` traité : capture arrêtée, motif retenu.
    Stopped {
        /// Motif de l'arrêt.
        reason: String,
    },
    /// `Finished` traité : bilan d'import retenu.
    Finished,
    /// `ImportProgress` traité : progression à jour.
    ImportProgress,
    /// Occupation du canal, pour la jauge de la barre de statut.
    ChannelCapacity {
        /// Capacité du canal, en paquets.
        channel_size: usize,
        /// Paquets en attente.
        current_size: usize,
        /// Canal saturé.
        backpressure: bool,
    },
    /// Lot de paquets pour le journal du bas.
    Packets(Vec<CapturedPacketOwned>),
    /// Mises à jour du graphe, dans l'ordre.
    GraphUpdates(Vec<GraphUpdate>),
    /// Graphe complet à recharger.
    GraphSnapshot(GraphData),
}

/// Nom sérialisé (`event`) d'un événement, tel que l'écrivent les traces.
pub fn event_name(event: &CaptureEvent) -> &'static str {
    match event {
        CaptureEvent::Started { .. } => "started",
        CaptureEvent::Stats { .. } => "stats",
        CaptureEvent::ChannelCapacityPayload { .. } => "channelCapacityPayload",
        CaptureEvent::PacketBatch { .. } => "packetBatch",
        CaptureEvent::Graph { .. } => "graph",
        CaptureEvent::GraphBatch { .. } => "graphBatch",
        CaptureEvent::Stopped { .. } => "stopped",
        CaptureEvent::Finished { .. } => "finished",
        CaptureEvent::ImportProgress { .. } => "importProgress",
        CaptureEvent::GraphSnapshot { .. } => "graphSnapshot",
    }
}

/// Session émettrice d'un événement ; `None` pour les événements sans
/// session (graphe unitaire, imports).
fn event_session(event: &CaptureEvent) -> Option<u64> {
    match event {
        CaptureEvent::Started { session_id, .. }
        | CaptureEvent::Stats { session_id, .. }
        | CaptureEvent::ChannelCapacityPayload { session_id, .. }
        | CaptureEvent::PacketBatch { session_id, .. }
        | CaptureEvent::GraphBatch { session_id, .. }
        | CaptureEvent::Stopped { session_id, .. } => Some(*session_id),
        CaptureEvent::Graph { .. }
        | CaptureEvent::Finished { .. }
        | CaptureEvent::ImportProgress { .. }
        | CaptureEvent::GraphSnapshot { .. } => None,
    }
}

/// État de la capture et configuration affichée (réduite au nom
/// d'interface).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureStore {
    /// Capture live en cours.
    pub is_running: bool,
    /// Démarrage en cours (`start_capture` pas encore revenu) : bloque un
    /// second démarrage (double-clic).
    pub is_starting: bool,
    /// Session de capture live courante (0 = aucune). Les événements d'une
    /// autre session sont périmés.
    pub session_id: u64,
    /// Plus grande session dont l'arrêt a été observé : rend l'état terminal
    /// monotone face à une réponse `Running` tardive.
    pub last_stopped_session_id: u64,
    /// Affichage de la matrice.
    pub show_matrice: bool,
    /// Verrou global d'import ou d'export.
    pub is_importing: bool,
    /// Progression de l'import en cours.
    pub import_progress: Option<ImportProgress>,
    /// La matrice contient des données.
    pub has_data: bool,
    /// Filtre BPF appliqué à la capture courante.
    pub active_filter: String,
    /// Filtre BPF enregistré pour le prochain démarrage.
    pub pending_filter: String,
    /// Type de liaison (LINKTYPE/DLT) de la capture ou des fichiers importés
    /// (vide : inconnu).
    pub link_type: String,
    /// Compteurs de la barre de statut. Ils vivent ici pour
    /// survivre au passage par la vue d'investigation.
    pub live_stats: StatsView,
    /// Bilan du dernier import.
    pub last_import: Option<LastImport>,
    /// Motif du dernier arrêt de capture.
    pub last_stop_reason: Option<String>,
    /// Interface de capture configurée (`captureConfig.interface`).
    pub interface: String,
}

impl Default for CaptureStore {
    fn default() -> Self {
        Self {
            is_running: false,
            is_starting: false,
            session_id: 0,
            last_stopped_session_id: 0,
            show_matrice: true,
            is_importing: false,
            import_progress: None,
            has_data: false,
            active_filter: String::new(),
            pending_filter: String::new(),
            link_type: String::new(),
            live_stats: StatsView::default(),
            last_import: None,
            last_stop_reason: None,
            interface: String::new(),
        }
    }
}

impl CaptureStore {
    /// Store neuf.
    pub fn new() -> Self {
        Self::default()
    }

    /// Applique le statut rendu par `start_capture` / `stop_capture`
    ///. `session_id` à `None` ou 0 : statut sans session.
    pub fn update_status(&mut self, is_running: bool, session_id: Option<u64>) {
        if let Some(sid) = session_id.filter(|sid| *sid > 0) {
            if sid < self.session_id {
                warn!(
                    "[CaptureStore] statut d'une session périmée ignoré ({sid} < {})",
                    self.session_id
                );
                return;
            }
            if is_running && sid <= self.last_stopped_session_id {
                warn!(
                    "[CaptureStore] statut Running tardif ignoré pour la session déjà arrêtée {sid}"
                );
                return;
            }
            self.session_id = sid;
        }
        self.is_running = is_running;
    }

    /// [`Self::update_status`] depuis le statut du moteur.
    pub fn apply_status(&mut self, status: &CaptureStatus) {
        self.update_status(status.is_running, Some(status.session_id));
    }

    /// Bascule l'affichage de la matrice.
    pub fn toggle_view(&mut self) {
        self.show_matrice = !self.show_matrice;
    }

    /// Filtre appliqué.
    pub fn set_active_filter(&mut self, filter: impl Into<String>) {
        self.active_filter = filter.into();
    }

    /// Oublie la progression d'import.
    pub fn clear_import_progress(&mut self) {
        self.import_progress = None;
    }

    /// Filtre du prochain démarrage.
    pub fn set_pending_filter(&mut self, filter: impl Into<String>) {
        self.pending_filter = filter.into();
    }

    /// Résultat de `is_matrix_empty`.
    pub fn set_matrix_empty(&mut self, empty: bool) {
        self.has_data = !empty;
    }

    /// Interface de la configuration de capture (`captureConfig.updateConfig`).
    pub fn set_interface(&mut self, interface: impl Into<String>) {
        self.interface = interface.into();
    }

    /// Filtrage de session : un événement d'une
    /// autre session que la courante est périmé. Effet de bord assumé : un
    /// `started` valide fait avancer la session courante.
    pub fn should_process(&mut self, event: &CaptureEvent) -> bool {
        let Some(sid) = event_session(event).filter(|sid| *sid > 0) else {
            return true;
        };
        if matches!(event, CaptureEvent::Started { .. }) {
            if sid < self.session_id {
                warn!(
                    "[CaptureStore] started d'une session périmée ignoré ({sid} < {})",
                    self.session_id
                );
                return false;
            }
            self.session_id = sid;
            return true;
        }
        if sid != self.session_id {
            warn!(
                "[CaptureStore] {} d'une session périmée ignoré ({sid} ≠ {})",
                event_name(event),
                self.session_id
            );
            return false;
        }
        true
    }

    /// Traite un événement du moteur : filtre de session, mise à jour de
    /// l'état, puis ce qu'il faut relayer. `None` : événement périmé, ignoré.
    pub fn handle_event(&mut self, event: CaptureEvent) -> Option<StoreEffect> {
        if !self.should_process(&event) {
            return None;
        }
        Some(self.dispatch(event))
    }

    /// Mise à jour de l'état par variante.
    fn dispatch(&mut self, event: CaptureEvent) -> StoreEffect {
        match event {
            CaptureEvent::Started {
                link_type,
                protocol_version,
                ..
            } => {
                if protocol_version != EXPECTED_PROTOCOL_VERSION {
                    warn!(
                        "[CaptureStore] version du contrat d'événements inattendue : reçu {protocol_version}, attendu {EXPECTED_PROTOCOL_VERSION} — interface et moteur probablement désynchronisés"
                    );
                }
                if !self.pending_filter.is_empty() {
                    self.active_filter = std::mem::take(&mut self.pending_filter);
                }
                self.link_type = link_type;
                self.last_stop_reason = None;
                StoreEffect::Started
            }
            CaptureEvent::Stats {
                received,
                dropped,
                if_dropped,
                app_dropped,
                rejected_truncated,
                rejected_unsupported_link_type,
                rejected_malformed,
                processed,
                ..
            } => {
                self.live_stats = StatsView {
                    received: u64::from(received),
                    dropped: u64::from(dropped),
                    if_dropped: u64::from(if_dropped),
                    app_dropped,
                    rejected_truncated,
                    rejected_unsupported_link_type,
                    rejected_malformed,
                    processed: u64::from(processed),
                };
                StoreEffect::Stats
            }
            CaptureEvent::ChannelCapacityPayload {
                channel_size,
                current_size,
                backpressure,
                ..
            } => StoreEffect::ChannelCapacity {
                channel_size,
                current_size,
                backpressure,
            },
            CaptureEvent::PacketBatch { packets, .. } => {
                // Pas d'événement `packet` unitaire : c'est ici que la
                // présence de données est détectée.
                if !packets.is_empty() && !self.has_data {
                    self.has_data = true;
                }
                StoreEffect::Packets(packets)
            }
            CaptureEvent::Graph { update } => StoreEffect::GraphUpdates(vec![update]),
            CaptureEvent::GraphBatch { updates, .. } => StoreEffect::GraphUpdates(updates),
            CaptureEvent::Stopped { session_id, reason } => {
                info!("[CaptureStore] Capture arrêtée : {reason}");
                // Arrêt initié par le moteur (erreur pcap…) : sans cette mise à
                // jour, l'interface croirait capturer indéfiniment.
                self.last_stopped_session_id = self.last_stopped_session_id.max(session_id);
                self.is_running = false;
                self.last_stop_reason = Some(reason.clone());
                StoreEffect::Stopped { reason }
            }
            CaptureEvent::Finished {
                file_name,
                packet_total_count,
                integrated_count,
                rejected_truncated_count,
                rejected_unsupported_link_type_count,
                rejected_malformed_count,
                matrix_total_count,
            } => {
                self.live_stats = StatsView {
                    received: packet_total_count as u64,
                    processed: matrix_total_count as u64,
                    rejected_truncated: rejected_truncated_count as u64,
                    rejected_unsupported_link_type: rejected_unsupported_link_type_count as u64,
                    rejected_malformed: rejected_malformed_count as u64,
                    ..self.live_stats
                };
                self.last_import = Some(LastImport {
                    file_name,
                    integrated_count,
                    total_count: packet_total_count,
                    truncated_count: rejected_truncated_count,
                    unsupported_link_type_count: rejected_unsupported_link_type_count,
                    malformed_count: rejected_malformed_count,
                });
                StoreEffect::Finished
            }
            CaptureEvent::ImportProgress {
                file_name,
                file_index,
                files_total,
                current,
                total,
            } => {
                self.import_progress = Some(ImportProgress {
                    file_name,
                    file_index: file_index as u64,
                    files_total: files_total as u64,
                    current: current as u64,
                    total: total as u64,
                });
                StoreEffect::ImportProgress
            }
            CaptureEvent::GraphSnapshot { graph_data } => StoreEffect::GraphSnapshot(graph_data),
        }
    }

    /// Remise à zéro du bilan, sur l'événement global `reset`
    ///.
    pub fn reset_counters(&mut self) {
        self.live_stats = StatsView::default();
        self.last_import = None;
        self.last_stop_reason = None;
    }

    /// Ligne de trace écrite au retour dans la vue d'analyse quand le bilan a
    /// été conservé ; `None` s'il n'y a rien à
    /// restaurer.
    pub fn restoration_log(&self) -> Option<String> {
        let stats = &self.live_stats;
        if stats.received == 0 && stats.processed == 0 && self.last_import.is_none() {
            return None;
        }
        let import = self
            .last_import
            .as_ref()
            .map(|last| format!(", import {}", last.file_name))
            .unwrap_or_default();
        Some(format!(
            "[Barre d'état] Restauration au montage : {} reçus, {} flux{import}",
            stats.received, stats.processed
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Fixtures : événements tels que le moteur les produit.

    fn started(session_id: u64) -> CaptureEvent {
        CaptureEvent::Started {
            session_id,
            device: "eth0".into(),
            buffer_size: 65536,
            chan_capacity: 1024,
            timeout: 100,
            snaplen: 262_144,
            link_type: "Ethernet".into(),
            protocol_version: 1,
        }
    }

    fn stats(session_id: u64, received: u32) -> CaptureEvent {
        CaptureEvent::Stats {
            session_id,
            received,
            dropped: 0,
            if_dropped: 0,
            app_dropped: 0,
            rejected_truncated: 2,
            rejected_unsupported_link_type: 0,
            rejected_malformed: 1,
            processed: 3,
        }
    }

    fn stopped(session_id: u64, reason: &str) -> CaptureEvent {
        CaptureEvent::Stopped {
            session_id,
            reason: reason.into(),
        }
    }

    fn finished() -> CaptureEvent {
        CaptureEvent::Finished {
            file_name: "capture.pcap".into(),
            packet_total_count: 100,
            integrated_count: 95,
            rejected_truncated_count: 3,
            rejected_unsupported_link_type_count: 1,
            rejected_malformed_count: 1,
            matrix_total_count: 40,
        }
    }

    fn packet_batch(session_id: u64, count: usize) -> CaptureEvent {
        CaptureEvent::PacketBatch {
            session_id,
            packets: vec![sample_packet(); count],
        }
    }

    /// Paquet Ethernet → IPv4 → UDP réel (trame des tests de
    /// `lunar-flows-core`), parsé comme le fait le pipeline de capture.
    fn sample_packet() -> CapturedPacketOwned {
        use lunar_flows_core::packet::CapturedPacket;
        use packet_parser::PacketFlow;
        const FRAME: [u8; 42] = [
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0x08, 0x00,
            0x45, 0x00, 0x00, 0x1c, 0x00, 0x01, 0x00, 0x00, 0x40, 0x11, 0x00, 0x0a, 0xc0, 0xa8,
            0x00, 0x01, 0xc0, 0xa8, 0x00, 0x02, 0x00, 0x35, 0x00, 0x35, 0x00, 0x08, 0x00, 0x00,
        ];
        let flow = PacketFlow::try_from(FRAME.as_slice()).expect("trame UDP");
        let packet = CapturedPacket {
            ts_sec: 1,
            ts_usec: 2,
            caplen: 42,
            len: 42,
            flow,
        };
        packet.to_owned_packet()
    }

    fn graph_update() -> GraphUpdate {
        GraphUpdate::NodeUpdated(lunar_flows_core::graph::Node {
            id: "node-1".into(),
            name: "192.168.1.10".into(),
            color: "#ff0000".into(),
            mac: "aa:bb:cc:dd:ee:ff".into(),
            macs: vec!["aa:bb:cc:dd:ee:ff".into()],
            ip: "192.168.1.10".into(),
            vlan_id: None,
            duplicate_ip: false,
            gateway_mac: false,
            label: Some("Serveur".into()),
        })
    }

    #[test]
    fn refresh_has_data_applique_is_matrix_empty() {
        let mut store = CaptureStore::new();
        assert!(!store.has_data);
        store.set_matrix_empty(false);
        assert!(store.has_data, "matrice non vide");
        store.set_matrix_empty(true);
        assert!(!store.has_data, "matrice vide");
    }

    #[test]
    fn graph_batch_redistribue_update_par_update() {
        let mut store = CaptureStore::new();
        // sessionId 0 = hors session : passe le filtre de session.
        let effect = store.handle_event(CaptureEvent::GraphBatch {
            session_id: 0,
            updates: vec![graph_update(), graph_update()],
        });
        match effect {
            Some(StoreEffect::GraphUpdates(updates)) => assert_eq!(updates.len(), 2),
            other => panic!("effet inattendu : {other:?}"),
        }
        let single = store.handle_event(CaptureEvent::Graph {
            update: graph_update(),
        });
        assert!(matches!(single, Some(StoreEffect::GraphUpdates(u)) if u.len() == 1));
    }

    #[test]
    fn graph_snapshot_atteint_les_abonnes() {
        let mut store = CaptureStore::new();
        let effect = store.handle_event(CaptureEvent::GraphSnapshot {
            graph_data: GraphData::new(),
        });
        assert!(matches!(effect, Some(StoreEffect::GraphSnapshot(_))));
    }

    #[test]
    fn import_progress_met_a_jour_la_progression() {
        let mut store = CaptureStore::new();
        store.handle_event(CaptureEvent::ImportProgress {
            file_name: "capture.pcap".into(),
            file_index: 2,
            files_total: 5,
            current: 1000,
            total: 48_350,
        });
        assert_eq!(
            store.import_progress,
            Some(ImportProgress {
                file_name: "capture.pcap".into(),
                file_index: 2,
                files_total: 5,
                current: 1000,
                total: 48_350,
            })
        );
        store.clear_import_progress();
        assert_eq!(store.import_progress, None);
    }

    #[test]
    fn started_promeut_le_filtre_en_attente() {
        let mut store = CaptureStore::new();
        store.set_active_filter("tcp");
        store.set_pending_filter("udp");
        store.handle_event(started(7));
        assert_eq!(store.active_filter, "udp");
        assert_eq!(store.pending_filter, "");
        assert_eq!(store.link_type, "Ethernet");
    }

    #[test]
    fn started_sans_filtre_en_attente_garde_le_filtre_actif() {
        let mut store = CaptureStore::new();
        store.set_active_filter("tcp");
        store.handle_event(started(1));
        assert_eq!(store.active_filter, "tcp");
    }

    #[test]
    fn packet_batch_signale_des_donnees_et_relaie_le_lot() {
        let mut store = CaptureStore::new();
        let effect = store.handle_event(packet_batch(0, 2));
        match effect {
            Some(StoreEffect::Packets(packets)) => assert_eq!(packets.len(), 2),
            other => panic!("effet inattendu : {other:?}"),
        }
        assert!(store.has_data);
        let mut empty = CaptureStore::new();
        empty.handle_event(packet_batch(0, 0));
        assert!(!empty.has_data, "un lot vide ne signale pas de données");
    }

    #[test]
    fn les_evenements_d_une_session_perimee_sont_ignores() {
        let mut store = CaptureStore::new();
        // La session 2 est la session courante.
        store.handle_event(started(2));
        assert_eq!(store.session_id, 2);
        store.is_running = true;

        // Retardataires de la session 1 : filtrés.
        assert!(store.handle_event(packet_batch(1, 1)).is_none());
        assert!(store.handle_event(stopped(1, "vieux pipeline")).is_none());
        assert!(
            store.is_running,
            "un stopped périmé n'arrête pas la session courante"
        );

        // Ceux de la session courante passent, et les hors session (0).
        assert!(store.handle_event(packet_batch(2, 1)).is_some());
        assert!(store.handle_event(packet_batch(0, 1)).is_some());

        store.handle_event(stopped(2, "arrêt demandé"));
        assert!(!store.is_running);
    }

    #[test]
    fn started_d_une_session_anterieure_est_ignore() {
        let mut store = CaptureStore::new();
        store.handle_event(started(5));
        assert!(store.handle_event(started(4)).is_none());
        assert_eq!(store.session_id, 5);
    }

    #[test]
    fn update_status_reprend_le_session_id_du_moteur() {
        let mut store = CaptureStore::new();
        store.update_status(true, Some(7));
        assert!(store.is_running);
        assert_eq!(store.session_id, 7);
        // L'arrêt ne remet pas l'id à zéro : il filtre encore les
        // retardataires de la session close.
        store.update_status(false, None);
        assert!(!store.is_running);
        assert_eq!(store.session_id, 7);
    }

    #[test]
    fn un_running_tardif_ne_ressuscite_pas_une_session_arretee() {
        let mut store = CaptureStore::new();
        store.handle_event(started(7));
        store.handle_event(stopped(7, "erreur pcap"));
        assert!(!store.is_running);
        assert_eq!(store.last_stopped_session_id, 7);

        // La réponse de `start_capture` peut arriver après l'événement
        // terminal : elle est ignorée.
        store.update_status(true, Some(7));
        assert!(!store.is_running, "la réponse Running périmée est ignorée");
        assert_eq!(store.session_id, 7);

        // Une génération plus récente reste autorisée.
        store.update_status(true, Some(8));
        assert!(store.is_running);
        assert_eq!(store.session_id, 8);

        // Une réponse plus ancienne ne fait pas régresser l'id.
        store.update_status(true, Some(6));
        assert!(store.is_running);
        assert_eq!(store.session_id, 8);
    }

    #[test]
    fn apply_status_traite_la_session_zero_comme_hors_session() {
        let mut store = CaptureStore::new();
        store.update_status(true, Some(3));
        store.apply_status(&CaptureStatus {
            is_running: false,
            phase: lunar_engine::state::capture::capture_status::CapturePhase::Idle,
            session_id: 0,
        });
        assert!(!store.is_running);
        assert_eq!(store.session_id, 3);
    }

    #[test]
    fn stats_alimente_les_compteurs() {
        let mut store = CaptureStore::new();
        store.handle_event(stats(0, 1));
        assert_eq!(store.live_stats.received, 1);
        assert_eq!(store.live_stats.processed, 3);
        assert_eq!(store.live_stats.rejected_total(), 3);
        store.handle_event(stats(0, 2));
        assert_eq!(store.live_stats.received, 2);
    }

    // Le bilan vit dans le store, il survit au passage par la vue
    // d'investigation.
    #[test]
    fn bilan_d_import_retenu_apres_finished() {
        let mut store = CaptureStore::new();
        store.handle_event(finished());
        assert_eq!(store.live_stats.received, 100);
        assert_eq!(store.live_stats.processed, 40);
        assert_eq!(store.live_stats.rejected_truncated, 3);
        assert_eq!(
            store.last_import,
            Some(LastImport {
                file_name: "capture.pcap".into(),
                integrated_count: 95,
                total_count: 100,
                truncated_count: 3,
                unsupported_link_type_count: 1,
                malformed_count: 1,
            })
        );
    }

    #[test]
    fn bilan_d_import_continue_d_etre_alimente() {
        let mut store = CaptureStore::new();
        store.handle_event(finished());
        // Sans abonné (barre de statut démontée), le bilan reste et suit les
        // compteurs.
        store.handle_event(stats(0, 777));
        assert_eq!(store.live_stats.received, 777);
        assert_eq!(
            store.last_import.as_ref().map(|l| l.file_name.as_str()),
            Some("capture.pcap")
        );
    }

    #[test]
    fn raison_d_arret_effacee_au_demarrage_suivant() {
        let mut store = CaptureStore::new();
        store.handle_event(started(3));
        store.handle_event(stopped(3, "erreur pcap"));
        assert_eq!(store.last_stop_reason.as_deref(), Some("erreur pcap"));
        store.handle_event(started(4));
        assert_eq!(store.last_stop_reason, None);
    }

    #[test]
    fn reset_counters_remet_le_bilan_a_zero() {
        let mut store = CaptureStore::new();
        store.handle_event(finished());
        store.reset_counters();
        assert_eq!(store.live_stats, StatsView::default());
        assert_eq!(store.last_import, None);
        assert_eq!(store.last_stop_reason, None);
    }

    #[test]
    fn badge_et_equation_du_bilan() {
        let mut store = CaptureStore::new();
        store.handle_event(finished());
        let Some(last) = store.last_import.clone() else {
            panic!("bilan absent");
        };
        assert_eq!(
            last.badge_text(),
            "Import : capture.pcap (100 lignes, 5 rejetées)"
        );
        assert_eq!(
            last.report_detail(),
            "capture.pcap : 100 lus = 95 intégrés + 3 tronqués + 1 DLT non supportés + 1 malformés"
        );
        let single = LastImport {
            malformed_count: 1,
            ..LastImport::default()
        };
        assert!(single.badge_text().ends_with("lignes, 1 rejetée)"));
        assert!(LastImport::default().badge_text().ends_with("(0 lignes)"));
    }

    #[test]
    fn trace_de_restauration() {
        let mut store = CaptureStore::new();
        assert_eq!(store.restoration_log(), None, "rien au premier démarrage");
        store.handle_event(finished());
        assert_eq!(
            store.restoration_log().as_deref(),
            Some(
                "[Barre d'état] Restauration au montage : 100 reçus, 40 flux, import capture.pcap"
            )
        );
    }

    #[test]
    fn occupation_du_canal() {
        assert_eq!(channel_occupancy_percent(1024, 512), 50);
        assert_eq!(channel_occupancy_percent(0, 5), 0, "canal de taille nulle");
        assert_eq!(channel_occupancy_percent(3, 1), 33);
        assert_eq!(channel_occupancy_percent(10, 20), 100, "borné à 100");
    }

    #[test]
    fn noms_d_evenement() {
        assert_eq!(event_name(&started(1)), "started");
        assert_eq!(event_name(&finished()), "finished");
        assert_eq!(
            event_name(&CaptureEvent::ChannelCapacityPayload {
                session_id: 1,
                channel_size: 1,
                current_size: 0,
                backpressure: false
            }),
            "channelCapacityPayload"
        );
    }
}
