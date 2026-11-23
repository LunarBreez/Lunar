//! Travail hors ligne lancé de la barre d'outils et du menu Fichier :
//! ouvrir et enregistrer un projet `.lunar`, exporter la matrice en CSV,
//! l'archive de support et le graphe en PNG.
//!
//! Chaque opération pose le verrou global d'import
//! **avant** son dialogue de fichiers et ferme le panneau ouvert ; le verrou
//! est rendu avant la suite (rafraîchissement, notification), même en cas
//! d'erreur ou d'annulation. L'export PNG, lui, ne prend pas le verrou (il
//! ne touche pas au moteur). Les lignes de journal sont stables (le harnais
//! E2E les lit) ; une erreur du moteur s'affiche dans le dialogue d'erreur,
//! une réussite dans une notification.
//!
//! Projets récents : `ui-preferences.json` dans le dossier de configuration
//! du moteur ([`AppState::ui_preferences`]). Les préférences ne bloquent
//! jamais : une erreur est tracée et l'opération continue.

use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::{Datelike, Local, Timelike};
use gpui::Context;
use log::{error, info, warn};
use lunar_engine::Lunar;
use lunar_engine::events::CaptureEvent;
use lunar_engine::ipc::Channel;
use lunar_ui_logic::file_names::{
    LocalDateTime, flow_matrix_csv_name, network_graph_png_name, project_file_name,
    support_archive_name,
};
use lunar_ui_logic::import_files::relabel_import_event;
use lunar_ui_logic::recent_projects::{UI_PREFERENCES_FILE, UiPreferences};
use lunar_ui_logic::save_paths::{default_save_path, file_name_of};

use crate::app::file_dialog::{
    CSV_FILTER, PNG_FILTER, PROJECT_FILTER, ZIP_FILTER, pick_files, pick_save_path,
};
use crate::app::state::{AppState, BUSY_MESSAGE};
use crate::widgets::toast::ToastKind;

/// Date et heure locales, pour les noms de fichiers proposés.
pub fn local_now() -> LocalDateTime {
    let now = Local::now();
    LocalDateTime::new(
        now.year(),
        now.month() as u8,
        now.day() as u8,
        now.hour() as u8,
        now.minute() as u8,
        now.second() as u8,
    )
}

/// Canal d'une ouverture de projet : celui du store, dont les événements
/// d'import sont rapportés au fichier `.lunar` (et non au CSV extrait dans
/// un dossier temporaire, que le badge d'import afficherait sinon).
pub fn project_channel(store: Channel<CaptureEvent>, project: &str) -> Channel<CaptureEvent> {
    let project = project.to_owned();
    Channel::new_fallible(move |event| store.send(relabel_import_event(event, &project)))
}

impl AppState {
    /// Préférences d'interface (projets récents).
    pub fn ui_preferences(&self) -> UiPreferences {
        UiPreferences::new(
            self.bridge()
                .lunar()
                .paths()
                .config_dir
                .join(UI_PREFERENCES_FILE),
        )
    }

    /// Dossier du projet le plus récent.
    fn last_project_dir(&self) -> Option<String> {
        self.ui_preferences()
            .last_project_dir()
            .unwrap_or_else(|err| {
                warn!("recents illisibles: {err}");
                None
            })
    }

    /// Enregistre `path` en tête des projets récents.
    fn record_recent_project(&self, path: &str) {
        if let Err(err) = self.ui_preferences().record_recent_project(path) {
            warn!("recents non enregistrés: {err}");
        }
    }

    /// Pose le verrou global d'import, sans fermer de panneau. Refusé (et
    /// signalé) s'il est déjà tenu.
    pub fn hold_import_lock(&mut self, cx: &mut Context<Self>) -> bool {
        if self.capture().is_importing {
            info!("{BUSY_MESSAGE}");
            self.toast(ToastKind::Info, BUSY_MESSAGE, cx);
            return false;
        }
        self.update_capture(|capture| capture.is_importing = true, cx);
        true
    }

    /// Rend le verrou global d'import.
    pub fn release_import_lock(&mut self, cx: &mut Context<Self>) {
        self.update_capture(|capture| capture.is_importing = false, cx);
    }

    /// Opération sous le verrou d'import : verrou posé, panneau de la barre
    /// d'outils fermé, puis `dialog` ; si un chemin est choisi, `prepare`
    /// s'exécute sur le thread de rendu (canal d'événements…) puis `work` en
    /// fond sur le moteur. Le verrou est rendu avant `done`, et aussitôt si le dialogue
    /// est annulé (`cancelled` est alors journalisé).
    fn with_import_lock<P: Send + 'static, R: Send + 'static>(
        &mut self,
        cx: &mut Context<Self>,
        dialog: impl Future<Output = Option<String>> + 'static,
        cancelled: &'static str,
        prepare: impl FnOnce(&mut Self, &str, &mut Context<Self>) -> P + 'static,
        work: impl FnOnce(&Lunar, String, P) -> R + Send + 'static,
        done: impl FnOnce(&mut Self, String, R, &mut Context<Self>) + 'static,
    ) {
        if !self.hold_import_lock(cx) {
            return;
        }
        if let Some(panel) = self
            .panels()
            .main()
            .filter(|panel| panel.is_toolbar_panel())
        {
            self.close_panel(panel, cx);
        }
        let lunar = Arc::clone(self.bridge().lunar());
        cx.spawn(async move |this, cx| {
            let Some(path) = dialog.await else {
                info!("{cancelled}");
                let _ = this.update(cx, |this, cx| this.release_import_lock(cx));
                return;
            };
            let Ok(prepared) = this.update(cx, |this, cx| prepare(this, &path, cx)) else {
                return;
            };
            let chosen = path.clone();
            let result = cx
                .background_executor()
                .spawn(async move { work(&lunar, chosen, prepared) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.release_import_lock(cx);
                done(this, path, result, cx);
            });
        })
        .detach();
    }

    /// Ouvrir un projet : le projet remplace le
    /// relevé, son instantané recharge le graphe.
    pub fn open_project_dialog(&mut self, cx: &mut Context<Self>) {
        info!("Ouverture d'un projet");
        let directory = self.last_project_dir();
        let dialog = pick_files(
            Some("Ouvrir un projet"),
            PROJECT_FILTER,
            false,
            directory.as_deref(),
        );
        let dialog = async move { dialog.await.and_then(|paths| paths.into_iter().next()) };
        self.with_import_lock(
            cx,
            dialog,
            "Aucun projet sélectionné",
            |this, path, cx| {
                // Canal neuf par ouverture, créé au retour du dialogue ; la
                // progression précédente est oubliée.
                this.update_capture(|capture| capture.clear_import_progress(), cx);
                project_channel(this.bridge().store_channel(), path)
            },
            |lunar, path, channel| lunar.open_project(path, channel),
            |this, path, result, cx| match result {
                Ok(()) => {
                    this.record_recent_project(&path);
                    this.refresh_flags(cx);
                    this.warn_if_partial(cx);
                    info!("Projet ouvert");
                    this.toast(
                        ToastKind::Success,
                        format!("Projet ouvert : {}", file_name_of(&path)),
                        cx,
                    );
                }
                Err(err) => {
                    error!("Erreur ouverture projet: {err}");
                    this.show_error(&err, cx);
                }
            },
        );
    }

    /// Enregistrer le projet, proposé dans le
    /// dossier du projet le plus récent.
    pub fn save_project_dialog(&mut self, cx: &mut Context<Self>) {
        info!("Enregistrement du projet");
        let directory = self.last_project_dir();
        let default_path =
            default_save_path(directory.as_deref(), &project_file_name(&local_now()));
        let dialog = pick_save_path(Some("Enregistrer le projet"), PROJECT_FILTER, &default_path);
        self.with_import_lock(
            cx,
            dialog,
            "Aucun chemin sélectionné",
            |_, _, _| (),
            |lunar, path, ()| lunar.save_project(path),
            |this, path, result, cx| match result {
                Ok(()) => {
                    this.record_recent_project(&path);
                    info!("Projet enregistré");
                    this.refresh_flags(cx);
                    this.toast(
                        ToastKind::Success,
                        format!("Projet enregistré : {}", file_name_of(&path)),
                        cx,
                    );
                }
                Err(err) => {
                    error!("Erreur enregistrement projet: {err}");
                    this.show_error(&err, cx);
                }
            },
        );
    }

    /// Sauvegarder la matrice de flux en CSV.
    pub fn export_csv_dialog(&mut self, cx: &mut Context<Self>) {
        info!("Save as csv");
        let dialog = pick_save_path(
            Some("Sauvegarder la matrice de flux"),
            CSV_FILTER,
            &flow_matrix_csv_name(&local_now()),
        );
        self.with_import_lock(
            cx,
            dialog,
            "Aucun chemin sélectionné",
            |_, _, _| (),
            |lunar, path, ()| lunar.export_csv(path),
            |this, path, result, cx| match result {
                Ok(()) => {
                    // Réponse vide d'`export_csv`, sérialisée en JSON.
                    info!("response: null");
                    this.toast(
                        ToastKind::Success,
                        format!("Matrice exportée : {}", file_name_of(&path)),
                        cx,
                    );
                }
                Err(err) => {
                    let json = serde_json::to_string(&err).unwrap_or_else(|_| err.to_string());
                    error!("Erreur sauvegarde csv: {json}");
                    // Sans ce message, un export raté (disque plein, droits,
                    // fichier ouvert ailleurs) passerait inaperçu.
                    this.show_error(&err, cx);
                }
            },
        );
    }

    /// Exporter les journaux en archive de support datée.
    pub fn export_logs_dialog(&mut self, cx: &mut Context<Self>) {
        info!("export logs");
        let dialog = pick_save_path(
            Some("Sauvegarder les logs"),
            ZIP_FILTER,
            &support_archive_name(&local_now()),
        );
        self.with_import_lock(
            cx,
            dialog,
            "Aucun chemin de fichier sélectionné",
            |_, _, _| (),
            |lunar, path, ()| lunar.export_logs(path),
            |this, path, result, cx| match result {
                // Le moteur rend un message de réussite, pas le chemin.
                Ok(response) => {
                    let json =
                        serde_json::to_string(&response).unwrap_or_else(|_| response.clone());
                    info!("Sauvegarde terminée: {json}");
                    this.toast(
                        ToastKind::Success,
                        format!("Journaux exportés : {}", file_name_of(&path)),
                        cx,
                    );
                }
                Err(err) => {
                    error!("Erreur export logs: {err}");
                    this.show_error(&err, cx);
                }
            },
        );
    }

    /// Exporter le graphe affiché en PNG, hors verrou d'import.
    pub fn export_graph_png_dialog(&mut self, cx: &mut Context<Self>) {
        let dialog = pick_save_path(None, PNG_FILTER, &network_graph_png_name(&local_now()));
        cx.spawn(async move |this, cx| {
            let Some(path) = dialog.await else {
                return;
            };
            let Ok(export) = this.update(cx, |this, cx| {
                this.graph().export_png(PathBuf::from(&path), cx)
            }) else {
                return;
            };
            let result = export.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(()) => {
                    info!("PNG exporté dans {path}");
                    this.toast(
                        ToastKind::Success,
                        format!("Graphe exporté : {}", file_name_of(&path)),
                        cx,
                    );
                }
                Err(err) => {
                    error!("Export PNG impossible vers {path} : {err}");
                    this.toast(
                        ToastKind::Error,
                        format!("Export PNG impossible : {err}"),
                        cx,
                    );
                }
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn le_canal_de_projet_rapporte_les_evenements_au_fichier_lunar() {
        let received: Arc<Mutex<Vec<CaptureEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&received);
        let store = Channel::new(move |event| {
            if let Ok(mut events) = sink.lock() {
                events.push(event);
            }
        });
        let channel = project_channel(store, "/releves/audit.lunar");
        channel
            .send(CaptureEvent::Finished {
                file_name: "/tmp/extrait/matrice.csv".into(),
                packet_total_count: 3,
                integrated_count: 3,
                rejected_truncated_count: 0,
                rejected_unsupported_link_type_count: 0,
                rejected_malformed_count: 0,
                matrix_total_count: 3,
            })
            .expect("envoi");
        let events = received.lock().expect("verrou");
        assert!(matches!(
            events.as_slice(),
            [CaptureEvent::Finished { file_name, .. }] if file_name == "/releves/audit.lunar"
        ));
    }

    #[test]
    fn date_locale_plausible() {
        let now = local_now();
        assert!(now.year >= 2024);
        assert!((1..=12).contains(&now.month));
        assert!((1..=31).contains(&now.day));
    }
}
