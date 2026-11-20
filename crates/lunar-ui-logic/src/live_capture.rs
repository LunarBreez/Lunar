//! Démarrage et arrêt de la capture live : gardes et application des
//! réponses du moteur au store de capture.
//!
//! L'appel au moteur (`start_capture`, `stop_capture`) reste côté
//! interface : [`LiveCapture::begin_start`] dit s'il faut l'engager,
//! [`LiveCapture::finish_start`] applique sa réponse, de même pour l'arrêt.
//!
//! La monotonie de l'arrêt (une réponse `Running` tardive ne ranime pas une
//! session déjà arrêtée) et l'abandon des sessions périmées sont ceux de
//! [`CaptureStore::update_status`].
//!
//! Un second arrêt demandé pendant que le premier attend le moteur est
//! ignoré : relayé, le moteur le refuserait (`Opération refusée`) et
//! l'interface se croirait arrêtée avant la fin du drainage.

use log::{debug, info};
use lunar_engine::errors::CaptureStateError;
use lunar_engine::state::capture::capture_status::CaptureStatus;

use crate::capture_store::CaptureStore;

/// Démarrage refusé avant tout appel au moteur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartRefusal {
    /// Un panneau de la barre d'outils est ouvert.
    PanelOpen,
    /// Le verrou d'import ou d'export est tenu.
    Importing,
    /// Une capture tourne déjà.
    Running,
    /// Un démarrage attend déjà le moteur (double-clic).
    Starting,
}

/// Arrêt refusé avant tout appel au moteur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopRefusal {
    /// Aucune capture en cours.
    NotRunning,
    /// Le verrou d'import ou d'export est tenu.
    Importing,
    /// Un arrêt attend déjà le moteur.
    Stopping,
}

/// Démarrage ou arrêt en vol.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LiveCapture {
    /// Un arrêt attend la réponse du moteur.
    stopping: bool,
}

impl LiveCapture {
    /// Aucun arrêt en vol.
    pub fn new() -> Self {
        Self::default()
    }

    /// Un arrêt attend la réponse du moteur.
    pub fn is_stopping(&self) -> bool {
        self.stopping
    }

    /// Gardes du démarrage ; accepté, `is_starting` est posé tout de suite
    /// pour qu'un double-clic n'engage pas deux démarrages.
    pub fn begin_start(
        &mut self,
        store: &mut CaptureStore,
        panel_open: bool,
    ) -> Result<(), StartRefusal> {
        let refusal = if panel_open {
            Some(StartRefusal::PanelOpen)
        } else if store.is_importing {
            Some(StartRefusal::Importing)
        } else if store.is_running {
            Some(StartRefusal::Running)
        } else if store.is_starting {
            Some(StartRefusal::Starting)
        } else {
            None
        };
        if let Some(refusal) = refusal {
            debug!("[Capture] démarrage ignoré : {refusal:?}");
            return Err(refusal);
        }
        store.is_starting = true;
        Ok(())
    }

    /// Réponse de `start_capture` : statut appliqué et journalisé
    /// (« Capture démarrée : true »), l'erreur rendue pour être montrée.
    pub fn finish_start(
        &mut self,
        store: &mut CaptureStore,
        result: Result<CaptureStatus, CaptureStateError>,
    ) -> Result<(), CaptureStateError> {
        store.is_starting = false;
        let status = result?;
        store.apply_status(&status);
        info!("Capture démarrée : {}", store.is_running);
        Ok(())
    }

    /// Gardes de l'arrêt ; accepté, l'arrêt est marqué en vol.
    pub fn begin_stop(&mut self, store: &CaptureStore) -> Result<(), StopRefusal> {
        let refusal = if !store.is_running {
            Some(StopRefusal::NotRunning)
        } else if store.is_importing {
            Some(StopRefusal::Importing)
        } else if self.stopping {
            Some(StopRefusal::Stopping)
        } else {
            None
        };
        if let Some(refusal) = refusal {
            debug!("[Capture] arrêt ignoré : {refusal:?}");
            return Err(refusal);
        }
        self.stopping = true;
        Ok(())
    }

    /// Réponse de `stop_capture` : statut appliqué et journalisé
    /// (« Capture arrêtée : false »). En erreur, la capture est tout de même
    /// tenue pour arrêtée : le moteur a joint ses threads et normalisé son
    /// statut avant de signaler l'échec (envoi de `Stopped`…).
    pub fn finish_stop(
        &mut self,
        store: &mut CaptureStore,
        result: Result<CaptureStatus, CaptureStateError>,
    ) -> Result<(), CaptureStateError> {
        self.stopping = false;
        match result {
            Ok(status) => {
                store.apply_status(&status);
                info!("Capture arrêtée : {}", store.is_running);
                Ok(())
            }
            Err(err) => {
                store.update_status(false, None);
                Err(err)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lunar_engine::events::CaptureEvent;
    use lunar_engine::state::capture::capture_status::CapturePhase;

    fn status(is_running: bool, session_id: u64) -> CaptureStatus {
        CaptureStatus {
            is_running,
            phase: if is_running {
                CapturePhase::Running
            } else {
                CapturePhase::Idle
            },
            session_id,
        }
    }

    fn refused() -> CaptureStateError {
        CaptureStateError::InvalidTransition {
            from: "running".to_owned(),
            to: "starting".to_owned(),
        }
    }

    fn stopped(session_id: u64) -> CaptureEvent {
        CaptureEvent::Stopped {
            session_id,
            reason: "interface coupée".to_owned(),
        }
    }

    #[test]
    fn demarrage_puis_arret() {
        let mut store = CaptureStore::new();
        let mut live = LiveCapture::new();
        assert_eq!(live.begin_start(&mut store, false), Ok(()));
        assert!(store.is_starting);
        assert!(live.finish_start(&mut store, Ok(status(true, 1))).is_ok());
        assert!(store.is_running && !store.is_starting);
        assert_eq!(store.session_id, 1);

        assert_eq!(live.begin_stop(&store), Ok(()));
        assert!(live.is_stopping());
        assert!(live.finish_stop(&mut store, Ok(status(false, 1))).is_ok());
        assert!(!store.is_running && !live.is_stopping());
    }

    #[test]
    fn double_clic_sur_demarrer() {
        let mut store = CaptureStore::new();
        let mut live = LiveCapture::new();
        assert_eq!(live.begin_start(&mut store, false), Ok(()));
        assert_eq!(
            live.begin_start(&mut store, false),
            Err(StartRefusal::Starting)
        );
    }

    #[test]
    fn demarrage_refuse_par_les_gardes() {
        let mut live = LiveCapture::new();
        let mut store = CaptureStore::new();
        assert_eq!(
            live.begin_start(&mut store, true),
            Err(StartRefusal::PanelOpen)
        );
        store.is_importing = true;
        assert_eq!(
            live.begin_start(&mut store, false),
            Err(StartRefusal::Importing)
        );
        store.is_importing = false;
        store.is_running = true;
        assert_eq!(
            live.begin_start(&mut store, false),
            Err(StartRefusal::Running)
        );
        assert!(!store.is_starting, "aucun refus ne pose is_starting");
    }

    #[test]
    fn echec_du_demarrage() {
        let mut store = CaptureStore::new();
        let mut live = LiveCapture::new();
        assert!(live.begin_start(&mut store, false).is_ok());
        assert!(live.finish_start(&mut store, Err(refused())).is_err());
        assert!(!store.is_starting && !store.is_running);
        assert!(
            live.begin_start(&mut store, false).is_ok(),
            "un nouvel essai est possible"
        );
    }

    #[test]
    fn arret_du_moteur_avant_la_reponse_du_demarrage() {
        // Le moteur s'arrête seul (erreur pcap) : `Stopped` arrive avant la
        // réponse `Running` de `start_capture`, qui ne doit pas ranimer la
        // session.
        let mut store = CaptureStore::new();
        let mut live = LiveCapture::new();
        assert!(live.begin_start(&mut store, false).is_ok());
        store.session_id = 3;
        assert!(store.handle_event(stopped(3)).is_some());
        assert!(live.finish_start(&mut store, Ok(status(true, 3))).is_ok());
        assert!(!store.is_running, "statut Running tardif ignoré");
        assert_eq!(store.last_stop_reason.as_deref(), Some("interface coupée"));
    }

    #[test]
    fn arret_en_erreur_tenu_pour_arrete() {
        let mut store = CaptureStore::new();
        let mut live = LiveCapture::new();
        store.update_status(true, Some(2));
        assert!(live.begin_stop(&store).is_ok());
        assert!(live.finish_stop(&mut store, Err(refused())).is_err());
        assert!(!store.is_running);
        assert!(!live.is_stopping());
        assert_eq!(store.session_id, 2, "la session reste connue");
    }

    #[test]
    fn arret_refuse_par_les_gardes() {
        let mut store = CaptureStore::new();
        let mut live = LiveCapture::new();
        assert_eq!(live.begin_stop(&store), Err(StopRefusal::NotRunning));
        store.update_status(true, Some(1));
        store.is_importing = true;
        assert_eq!(live.begin_stop(&store), Err(StopRefusal::Importing));
        store.is_importing = false;
        assert!(live.begin_stop(&store).is_ok());
        assert_eq!(
            live.begin_stop(&store),
            Err(StopRefusal::Stopping),
            "second arrêt pendant le premier"
        );
    }

    #[test]
    fn statut_d_une_session_perimee_ignore() {
        let mut store = CaptureStore::new();
        let mut live = LiveCapture::new();
        store.update_status(true, Some(5));
        assert!(live.begin_stop(&store).is_ok());
        // Réponse d'une session plus ancienne : le store la rejette.
        assert!(live.finish_stop(&mut store, Ok(status(false, 4))).is_ok());
        assert!(store.is_running);
        assert_eq!(store.session_id, 5);
    }

    #[test]
    fn filtre_en_attente_promu_au_demarrage() {
        let mut store = CaptureStore::new();
        let mut live = LiveCapture::new();
        store.set_active_filter("icmp");
        store.set_pending_filter("udp");
        assert!(live.begin_start(&mut store, false).is_ok());
        let started = CaptureEvent::Started {
            session_id: 1,
            device: "lo".to_owned(),
            buffer_size: 1,
            chan_capacity: 1,
            timeout: 1,
            snaplen: 64,
            link_type: "EN10MB".to_owned(),
            protocol_version: crate::capture_store::EXPECTED_PROTOCOL_VERSION,
        };
        assert!(store.handle_event(started).is_some());
        assert!(live.finish_start(&mut store, Ok(status(true, 1))).is_ok());
        assert_eq!(store.active_filter, "udp");
        assert!(store.pending_filter.is_empty());
    }
}
