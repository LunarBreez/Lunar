//! Commandes de gestion des labels : le [`LabelStore`] est la source
//! de vérité unique ; chaque mutation resynchronise le miroir de la matrice
//! et les nœuds du graphe (`setup::labels::resync_labels`), notifie le
//! channel de capture s'il existe et retourne les updates graphe à
//! l'appelant (le panneau les applique sans dépendre du channel).

use std::sync::{Arc, Mutex};

use crate::{
    commandes::import::normalize_label_key,
    errors::CaptureStateError,
    events::CaptureEvent,
    setup::labels::resync_labels,
    state::{
        capture::CaptureState,
        flow_matrix::FlowMatrix,
        graph::{GraphData, GraphUpdate},
        labels_list::{LabelStore, PcInfoLabel},
    },
};

/// Ligne de label enrichie de son statut : `observed` indique qu'un endpoint
/// de la matrice courante correspond à la clé (label « actif » vs
/// « dormant » dans l'UI de gestion).
#[derive(Debug, Clone, serde::Serialize)]
pub struct LabelRowStatus {
    /// Adresse MAC de la clé.
    pub mac: String,
    /// Adresse IP de la clé.
    pub ip: String,
    /// Libellé de l'équipement.
    pub label: String,
    /// Vrai si un endpoint de la matrice courante porte cette clé.
    pub observed: bool,
}

/// Resynchronise miroir + graphe après une mutation du store et pousse les
/// updates sur le channel de capture s'il existe. Même ordre de verrouillage
/// que les imports (matrice -> graphe -> store) pour éviter un interblocage.
pub(crate) fn resync_and_notify(
    matrix: &Arc<Mutex<FlowMatrix>>,
    graph: &Arc<Mutex<GraphData>>,
    label_store: &Arc<Mutex<LabelStore>>,
    pcinfo: &Arc<Mutex<PcInfoLabel>>,
    capture_state: &Arc<Mutex<CaptureState>>,
) -> Result<Vec<GraphUpdate>, CaptureStateError> {
    let updates = {
        let mut matrix_guard = matrix.lock()?;
        let mut graph_guard = graph.lock()?;
        let store_guard = label_store.lock()?;
        let pcinfo_guard = pcinfo.lock()?;
        resync_labels(
            &pcinfo_guard,
            &store_guard,
            &mut matrix_guard,
            &mut graph_guard,
        )
    };

    // Best-effort : une erreur d'envoi ne remet pas en cause la mutation,
    // l'appelant reçoit de toute façon les updates dans la réponse.
    // Tous les appelants sont des mutations de labels : le relevé est marqué
    // modifié ici, une fois les verrous de données relâchés.
    let on_event = {
        let mut state = capture_state.lock()?;
        state.mark_dirty();
        state.on_event.clone()
    };
    if let Some(on_event) = on_event {
        for update in &updates {
            if let Err(e) = on_event.send(CaptureEvent::Graph {
                update: update.clone(),
            }) {
                log::error!("Erreur d'envoi du GraphUpdate label: {e}");
            }
        }
    }

    Ok(updates)
}

/// Ajoute ou remplace le label de la clé `(mac, ip)` (édition depuis le
/// graphe ou ajout depuis le panneau de gestion).
#[allow(clippy::too_many_arguments)]
pub fn add_label(
    matrix: &Arc<Mutex<FlowMatrix>>,
    graph: &Arc<Mutex<GraphData>>,
    label_store: &Arc<Mutex<LabelStore>>,
    pcinfo: &Arc<Mutex<PcInfoLabel>>,
    capture_state: &Arc<Mutex<CaptureState>>,
    mac: String,
    ip: String,
    label: String,
) -> Result<Vec<GraphUpdate>, CaptureStateError> {
    let (mac, ip) = normalize_label_key(&mac, &ip)?;
    label_store.lock()?.set(&mac, &ip, label.trim());
    resync_and_notify(matrix, graph, label_store, pcinfo, capture_state)
}

/// Modifie une ligne du store, clé comprise. Refuse si la nouvelle clé
/// écraserait une autre ligne (pas d'écrasement silencieux : l'utilisateur
/// arbitre).
#[allow(clippy::too_many_arguments)]
pub fn update_label(
    matrix: &Arc<Mutex<FlowMatrix>>,
    graph: &Arc<Mutex<GraphData>>,
    label_store: &Arc<Mutex<LabelStore>>,
    pcinfo: &Arc<Mutex<PcInfoLabel>>,
    capture_state: &Arc<Mutex<CaptureState>>,
    old_mac: String,
    old_ip: String,
    mac: String,
    ip: String,
    label: String,
) -> Result<Vec<GraphUpdate>, CaptureStateError> {
    let (mac, ip) = normalize_label_key(&mac, &ip)?;
    label_store
        .lock()?
        .update(&old_mac, &old_ip, &mac, &ip, label.trim())
        .map_err(crate::errors::label::LabelError::EditRejected)?;
    resync_and_notify(matrix, graph, label_store, pcinfo, capture_state)
}

/// Supprime la ligne de clé `(mac, ip)` : l'équipement est désétiqueté
/// (matrice et graphe resynchronisés).
pub fn delete_label(
    matrix: &Arc<Mutex<FlowMatrix>>,
    graph: &Arc<Mutex<GraphData>>,
    label_store: &Arc<Mutex<LabelStore>>,
    pcinfo: &Arc<Mutex<PcInfoLabel>>,
    capture_state: &Arc<Mutex<CaptureState>>,
    mac: String,
    ip: String,
) -> Result<Vec<GraphUpdate>, CaptureStateError> {
    label_store.lock()?.remove(&mac, &ip);
    resync_and_notify(matrix, graph, label_store, pcinfo, capture_state)
}

/// Période du relevé telle que l'interface l'affiche : vide, connue
/// (premier et dernier paquet, RFC 3339 en UTC), ou inconnue quand une
/// matrice importée ne la déclarait pas.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum SurveyPeriodView {
    /// Relevé sans paquet : aucune période.
    Empty,
    /// Période connue : premier et dernier paquet.
    Known {
        /// Horodatage du premier paquet, RFC 3339 en UTC.
        first: String,
        /// Horodatage du dernier paquet, RFC 3339 en UTC.
        last: String,
    },
    /// Matrice importée dont le préambule ne déclarait pas de période.
    Unknown,
}

impl From<lunar_flows_core::lfms::SurveyPeriod> for SurveyPeriodView {
    fn from(period: lunar_flows_core::lfms::SurveyPeriod) -> Self {
        use lunar_flows_core::lfms::{SurveyPeriod, format_timestamp};
        match period {
            SurveyPeriod::Empty => Self::Empty,
            SurveyPeriod::Known(period) => Self::Known {
                first: format_timestamp(period.first),
                last: format_timestamp(period.last),
            },
            SurveyPeriod::Unknown => Self::Unknown,
        }
    }
}

/// Période observée du relevé courant, lue à la demande par la vue
/// d'investigation pour la durée de sa synthèse.
pub fn get_survey_period(
    matrix: &Arc<Mutex<FlowMatrix>>,
) -> Result<SurveyPeriodView, CaptureStateError> {
    Ok(matrix.lock()?.period.into())
}

/// Snapshot complet du graphe courant (nœuds = endpoints, arêtes = flux),
/// indépendamment de la source du relevé (capture live, import PCAP/matrice,
/// projet ouvert). La vue d'investigation lit la matrice de flux **hors** du
/// flux d'événements de capture : elle a besoin d'une lecture à la demande de
/// l'état de référence, pas d'un abonnement au channel. Verrou court : simple
/// clone de l'état, aucune mutation.
pub fn get_graph_snapshot(graph: &Arc<Mutex<GraphData>>) -> Result<GraphData, CaptureStateError> {
    Ok(graph.lock()?.get_all_graph_data())
}

/// Lignes du store avec leur statut observé/dormant : une ligne est
/// « observée » si un endpoint réel de la matrice courante correspond à sa
/// clé (couple exact, ou MAC seule / IP seule pour les labels partiels).
pub fn get_labels_with_status(
    matrix: &Arc<Mutex<FlowMatrix>>,
    label_store: &Arc<Mutex<LabelStore>>,
) -> Result<Vec<LabelRowStatus>, CaptureStateError> {
    let endpoints = matrix.lock()?.observed_endpoints();
    let observed_macs: std::collections::HashSet<&str> =
        endpoints.iter().map(|(mac, _)| mac.as_str()).collect();
    let observed_ips: std::collections::HashSet<&str> =
        endpoints.iter().map(|(_, ip)| ip.as_str()).collect();

    let store = label_store.lock()?;
    Ok(store
        .get()
        .iter()
        .map(|(mac, ip, label)| {
            let observed = match (mac.is_empty(), ip.is_empty()) {
                (false, false) => endpoints.contains(&(mac.clone(), ip.clone())),
                (false, true) => observed_macs.contains(mac.as_str()),
                (true, false) => observed_ips.contains(ip.as_str()),
                (true, true) => false,
            };
            LabelRowStatus {
                mac: mac.clone(),
                ip: ip.clone(),
                label: label.clone(),
                observed,
            }
        })
        .collect())
}

/// Liste des labels connus de la matrice (valeurs seules, pour l'UI).
pub fn get_label_list(matrix: &Arc<Mutex<FlowMatrix>>) -> Result<Vec<String>, CaptureStateError> {
    let guard = matrix.lock()?;
    Ok(guard.get_label_list())
}

/// Labels réellement appliqués à la matrice de flux, au format `(mac, ip, label)`,
/// avec les champs manquants (MAC ou IP) complétés depuis la matrice — même
/// vue que l'export de labels.
pub fn get_matrix_labels(
    matrix: &Arc<Mutex<FlowMatrix>>,
) -> Result<Vec<(String, String, String)>, CaptureStateError> {
    let guard = matrix.lock()?;
    Ok(guard.export_labels())
}
