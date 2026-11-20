//! Événements poussés à l'interface sur le [`crate::ipc::Channel`], pendant
//! une capture live comme pendant un import. Le contrat sérialisé : noms
//! `camelCase`, forme `{ event, data }`.
//!
//! ## Convention de nommage sérialisée
//!
//! `camelCase` partout — tags d'événement, tags de variante interne
//! (`GraphUpdate`) et champs de struct — sur tout ce qui est *Lunar-owned*
//! (ce module, `StatsPayload`, `CapturedPacketOwned`, `Node`/`Edge`/
//! `GraphUpdate` de `lunar_flows_core`). Seule exception assumée : la
//! couche paquet/flux (`PacketFlow`, `DataLink`, `IpType`, `NetworkProtocol`,
//! `CorruptedLayerKind`…) reste dans la casse imposée par la crate
//! `packet_parser` (`snake_case` pour les champs, `snake_case` ou
//! `PascalCase` selon l'enum pour les tags) — cette dépendance ne se modifie
//! pas ici ; à signaler en amont si une uniformisation complète devient
//! nécessaire.

use serde::Serialize;

use crate::state::{
    capture::capture_handle::messages::capture::CapturedPacketOwned,
    graph::{GraphData, GraphUpdate},
};

/// Version du contrat `CaptureEvent` : à incrémenter à toute évolution
/// non rétro-compatible (variante retirée, champ renommé/retiré, forme JSON
/// changée) — jamais pour un ajout de champ optionnel ou de variante, qui
/// reste sûr à l'exécution : un consommateur journalise (au lieu de
/// planter) tout tag d'événement qu'il ne reconnaît pas encore. Portée par
/// `Started`, émis en tête de toute session (capture live, import PCAP et
/// import de matrice CSV), pour que le frontend la lise dès le début du
/// flux et signale un écart.
pub const CAPTURE_EVENT_PROTOCOL_VERSION: u32 = 1;

/// Événement de capture/import envoyé à l'interface. L'événement possède ses
/// données, sans emprunter l'état verrouillé : il traverse le canal vers un
/// autre thread sans durée de vie à respecter.
///
/// `session_id` identifie la session de capture live émettrice : le frontend
/// ignore les événements d'une session périmée. La valeur 0 signifie « hors
/// session » (imports PCAP/CSV), jamais filtrée.
#[derive(Debug, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "event",
    content = "data"
)]
pub enum CaptureEvent {
    /// La capture (ou la conversion PCAP) démarre, avec ses paramètres.
    /// En capture live, émis seulement une fois l'interface ouverte et le
    /// filtre appliqué : un échec de démarrage ne produit jamais `Started`.
    Started {
        /// Session de capture live émettrice ; 0 pour un import. Seul `Started`
        /// fait avancer la session courante du frontend, qui ignore ensuite tout
        /// événement d'une session antérieure.
        session_id: u64,
        /// Nom de l'interface écoutée (`CaptureConfig::device_name`) ; vide pour
        /// un import.
        device: String,
        /// Taille du buffer pcap, en octets ; 0 pour un import.
        buffer_size: i32,
        /// Capacité du canal capture→processing, en paquets ; 0 pour un import.
        chan_capacity: i32,
        /// Délai de lecture pcap, en millisecondes ; 0 pour un import.
        timeout: i32,
        /// Longueur de capture par paquet (snaplen pcap), en octets ; 0 pour un
        /// import.
        snaplen: i32,
        /// Type de liaison (LINKTYPE/DLT) de la capture ou du fichier importé
        /// (« Ethernet », « Linux cooked v1 »…), affiché dans la barre de
        /// statut. Plusieurs fichiers importés de DLT différents sont joints
        /// par des virgules.
        link_type: String,
        /// Version du contrat IPC, cf. [`CAPTURE_EVENT_PROTOCOL_VERSION`].
        protocol_version: u32,
    },
    /// Compteurs périodiques pour la barre de statut. Émis aussi une dernière
    /// fois après le drainage d'arrêt ou de plafond : le récapitulatif
    /// final où chaque paquet reçu appartient à une catégorie — intégré,
    /// illisible, ou perdu (noyau / interface / application).
    Stats {
        /// Session émettrice ; 0 pour le `Stats` final d'un import.
        session_id: u64,
        /// Paquets reçus par pcap depuis le début de la session (`ps_recv`) ;
        /// en import, paquets ou lignes lus du fichier.
        received: u32,
        /// Paquets perdus par le noyau, buffer pcap plein (`ps_drop`).
        dropped: u32,
        /// Paquets perdus par l'interface elle-même (`ps_ifdrop`).
        if_dropped: u32,
        /// Pertes côté application (pool de buffers épuisé ou canal plein),
        /// en plus des drops kernel remontés par pcap.
        app_dropped: u64,
        /// Rejets du parseur par catégorie fine : capture coupée au
        /// snaplen, type de liaison inconnu du parser, trame malformée sur
        /// le réseau observé. Même partition que l'import de fichier.
        rejected_truncated: u64,
        /// Rejets pour type de liaison inconnu du parser.
        rejected_unsupported_link_type: u64,
        /// Rejets pour trame malformée sur le réseau observé.
        rejected_malformed: u64,
        /// Flux distincts de la matrice (lignes du relevé), affiché comme total
        /// traité dans la barre de statut.
        processed: u32,
    },
    /// Occupation du canal capture→processing (indicateur de backpressure).
    ChannelCapacityPayload {
        /// Session émettrice (capture live uniquement).
        session_id: u64,
        /// Capacité du canal, en paquets (`chan_capacity`).
        channel_size: usize,
        /// Paquets en attente dans le canal au moment de la mesure ; le
        /// frontend en tire le taux d'occupation (`ChannelStatus`).
        current_size: usize,
        /// Vrai quand le canal est saturé, ou qu'un lot de pertes applicatives
        /// vient d'être rapporté : l'indicateur passe en alerte.
        backpressure: bool,
    },
    /// Lot de paquets traités (voir `PACKET_BATCH_MAX` côté processing).
    PacketBatch {
        /// Session émettrice.
        session_id: u64,
        /// Queue du lot traité, limitée à `PACKET_BATCH_UI_MAX` : matrice et
        /// graphe ont déjà intégré tout le lot, seule l'interface est allégée.
        packets: Vec<CapturedPacketOwned>,
    },
    /// Update graphe unitaire (ex. label de nœud modifié après arbitrage).
    Graph {
        /// Mise à jour unitaire, relayée aux abonnés graphe du store.
        update: GraphUpdate,
    },
    /// Updates graphe coalescées sur la fenêtre de batch (voir `GraphUpdateBatch`).
    GraphBatch {
        /// Session émettrice.
        session_id: u64,
        /// Mises à jour accumulées sur la fenêtre de batch, dans l'ordre
        /// d'arrivée, relayées une à une aux abonnés graphe du store.
        updates: Vec<GraphUpdate>,
    },
    /// Fin du pipeline de capture, normale ou sur erreur fatale (ex. pcap).
    /// Sans cet événement, le frontend croirait capturer indéfiniment après
    /// une erreur.
    Stopped {
        /// Session qui s'arrête : le frontend la mémorise comme dernière
        /// session close et repasse `isRunning` à faux.
        session_id: u64,
        /// Motif lisible de l'arrêt (demande de l'opérateur, plafond,
        /// erreur pcap…), affiché en badge dans la barre de statut.
        reason: String,
    },
    /// Fin de traitement d'un fichier importé (PCAP ou matrice CSV), avec le
    /// rapport qualité du fichier : chaque paquet lu appartient à
    /// exactement une catégorie fine, et le total émis est la somme des
    /// catégories (`PcapFileReport::read()`) — l'équation
    /// `lus = intégrés + tronqués + DLT non supportés + malformés`
    /// tient par construction. Affiché dans la barre de statut.
    Finished {
        /// Chemin du fichier importé, repris dans le bilan d'import.
        file_name: String,
        /// Paquets lus (lignes pour une matrice CSV) : somme des quatre
        /// catégories qui suivent.
        packet_total_count: usize,
        /// Paquets parsés et intégrés à la matrice (pour une matrice CSV :
        /// lignes importées).
        integrated_count: usize,
        /// Rejetés parce que la capture était coupée (`caplen < len`) :
        /// snaplen trop court au moment du relevé, pas un défaut du réseau.
        rejected_truncated_count: usize,
        /// Rejetés pour type de liaison inconnu du parser. Zéro attendu :
        /// la garde DLT rejette le fichier entier sinon — le bilan prouve
        /// ce zéro au lieu de le supposer.
        rejected_unsupported_link_type_count: usize,
        /// Rejetés trame complète : malformés sur le réseau observé (0 pour
        /// une matrice CSV : une ligne invalide est une erreur fatale, pas
        /// un skip).
        rejected_malformed_count: usize,
        /// Flux distincts de la matrice après fusion du fichier, affiché comme
        /// total traité.
        matrix_total_count: usize,
    },
    /// Progression d'un import multi-fichiers. `current` et `total` comptent
    /// les paquets pour un PCAP et les lignes pour une matrice CSV ; les index
    /// de fichier commencent à 1 pour être affichés directement dans l'UI.
    ImportProgress {
        /// Fichier en cours de traitement.
        file_name: String,
        /// Rang du fichier dans le lot, à partir de 1.
        file_index: usize,
        /// Nombre de fichiers du lot.
        files_total: usize,
        /// Paquets (ou lignes) déjà traités dans ce fichier ; émis tous les
        /// 1000 paquets et à la fin.
        current: usize,
        /// Paquets (ou lignes) attendus dans ce fichier, dénominateur du
        /// pourcentage affiché.
        total: usize,
    },
    /// Graphe complet, envoyé en fin d'import pour recharger la vue.
    GraphSnapshot {
        /// Graphe complet, relayé aux abonnés snapshot du store, qui
        /// remplacent la vue.
        graph_data: GraphData,
    },
}

/// `Clone` manuel : `GraphData` n'est pas `Clone` dans le cœur (ses index
/// internes ne se dupliquent pas), le snapshot se recopie donc par
/// [`GraphData::get_all_graph_data`], qui rend les mêmes nœuds et arêtes —
/// exactement ce qui est sérialisé.
impl Clone for CaptureEvent {
    fn clone(&self) -> Self {
        match self {
            Self::Started {
                session_id,
                device,
                buffer_size,
                chan_capacity,
                timeout,
                snaplen,
                link_type,
                protocol_version,
            } => Self::Started {
                session_id: *session_id,
                device: device.clone(),
                buffer_size: *buffer_size,
                chan_capacity: *chan_capacity,
                timeout: *timeout,
                snaplen: *snaplen,
                link_type: link_type.clone(),
                protocol_version: *protocol_version,
            },
            Self::Stats {
                session_id,
                received,
                dropped,
                if_dropped,
                app_dropped,
                rejected_truncated,
                rejected_unsupported_link_type,
                rejected_malformed,
                processed,
            } => Self::Stats {
                session_id: *session_id,
                received: *received,
                dropped: *dropped,
                if_dropped: *if_dropped,
                app_dropped: *app_dropped,
                rejected_truncated: *rejected_truncated,
                rejected_unsupported_link_type: *rejected_unsupported_link_type,
                rejected_malformed: *rejected_malformed,
                processed: *processed,
            },
            Self::ChannelCapacityPayload {
                session_id,
                channel_size,
                current_size,
                backpressure,
            } => Self::ChannelCapacityPayload {
                session_id: *session_id,
                channel_size: *channel_size,
                current_size: *current_size,
                backpressure: *backpressure,
            },
            Self::PacketBatch {
                session_id,
                packets,
            } => Self::PacketBatch {
                session_id: *session_id,
                packets: packets.clone(),
            },
            Self::Graph { update } => Self::Graph {
                update: update.clone(),
            },
            Self::GraphBatch {
                session_id,
                updates,
            } => Self::GraphBatch {
                session_id: *session_id,
                updates: updates.clone(),
            },
            Self::Stopped { session_id, reason } => Self::Stopped {
                session_id: *session_id,
                reason: reason.clone(),
            },
            Self::Finished {
                file_name,
                packet_total_count,
                integrated_count,
                rejected_truncated_count,
                rejected_unsupported_link_type_count,
                rejected_malformed_count,
                matrix_total_count,
            } => Self::Finished {
                file_name: file_name.clone(),
                packet_total_count: *packet_total_count,
                integrated_count: *integrated_count,
                rejected_truncated_count: *rejected_truncated_count,
                rejected_unsupported_link_type_count: *rejected_unsupported_link_type_count,
                rejected_malformed_count: *rejected_malformed_count,
                matrix_total_count: *matrix_total_count,
            },
            Self::ImportProgress {
                file_name,
                file_index,
                files_total,
                current,
                total,
            } => Self::ImportProgress {
                file_name: file_name.clone(),
                file_index: *file_index,
                files_total: *files_total,
                current: *current,
                total: *total,
            },
            Self::GraphSnapshot { graph_data } => Self::GraphSnapshot {
                graph_data: graph_data.get_all_graph_data(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    //! Forme JSON des événements possédés : le contrat est `{ event, data }`,
    //! en `camelCase`.
    use super::*;
    use serde_json::json;

    #[test]
    fn started_keeps_its_tag_and_camel_case_fields() {
        let event = CaptureEvent::Started {
            session_id: 3,
            device: "eth0".to_string(),
            buffer_size: 1,
            chan_capacity: 2,
            timeout: 25,
            snaplen: 65536,
            link_type: "Ethernet".to_string(),
            protocol_version: CAPTURE_EVENT_PROTOCOL_VERSION,
        };
        assert_eq!(
            serde_json::to_value(&event).unwrap(),
            json!({
                "event": "started",
                "data": {
                    "sessionId": 3,
                    "device": "eth0",
                    "bufferSize": 1,
                    "chanCapacity": 2,
                    "timeout": 25,
                    "snaplen": 65536,
                    "linkType": "Ethernet",
                    "protocolVersion": CAPTURE_EVENT_PROTOCOL_VERSION,
                },
            })
        );
    }

    #[test]
    fn finished_and_progress_carry_their_file_name() {
        let finished = CaptureEvent::Finished {
            file_name: "relevé.pcap".to_string(),
            packet_total_count: 4,
            integrated_count: 3,
            rejected_truncated_count: 1,
            rejected_unsupported_link_type_count: 0,
            rejected_malformed_count: 0,
            matrix_total_count: 2,
        };
        assert_eq!(
            serde_json::to_value(&finished).unwrap()["event"],
            "finished"
        );
        assert_eq!(
            serde_json::to_value(&finished).unwrap()["data"]["fileName"],
            "relevé.pcap"
        );
        let progress = CaptureEvent::ImportProgress {
            file_name: "relevé.pcap".to_string(),
            file_index: 1,
            files_total: 2,
            current: 0,
            total: 10,
        };
        assert_eq!(
            serde_json::to_value(&progress).unwrap(),
            json!({
                "event": "importProgress",
                "data": {
                    "fileName": "relevé.pcap",
                    "fileIndex": 1,
                    "filesTotal": 2,
                    "current": 0,
                    "total": 10,
                },
            })
        );
    }

    /// Le `Clone` manuel recopie le snapshot par `get_all_graph_data` : le
    /// clone sérialise exactement comme l'original.
    #[test]
    fn a_cloned_graph_snapshot_serializes_like_the_original() {
        let matrix_path = crate::test_support::fixtures().join("20260703_NP_Matrice.csv");
        let rows = lunar_flows_core::csv::read_matrix_rows(&matrix_path).unwrap();
        let matrix =
            lunar_flows_core::csv::matrix_from_rows(&rows, packet_parser::LinkType::ETHERNET)
                .unwrap();
        let event = CaptureEvent::GraphSnapshot {
            graph_data: GraphData::from_flow_matrix(&matrix),
        };
        let original = serde_json::to_value(&event).unwrap();
        assert_eq!(original["event"], "graphSnapshot");
        assert!(
            original["data"]["graphData"]["nodes"]
                .as_object()
                .is_some_and(|nodes| !nodes.is_empty())
        );
        assert_eq!(serde_json::to_value(event.clone()).unwrap(), original);
    }
}
