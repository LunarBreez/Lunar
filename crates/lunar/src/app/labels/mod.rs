//! Labels : gestion du store de labels, import d'un fichier CSV et de
//! relevés WinAudit, dialogue des conflits d'import, arbitrage, labels de la
//! matrice, export du fichier de labels et édition depuis le graphe.
//!
//! La logique pure (textes, recherche, vérification d'une saisie, candidats
//! d'un conflit, bilans) est dans `lunar-ui-logic` (`label_management`,
//! `label_import`, `label_conflicts`, `winaudit_import`).
//!
//! Les dialogues imbriqués (conflits, arbitrage) sont des vues à part,
//! construites par [`crate::app::panels::build_panel_view`] : ce qu'ils
//! montrent passe par [`session::LabelSession`], partagé avec leur panneau
//! parent.
//!
//! Les mutations du store rendent leurs mises à jour du graphe : elles vont
//! au graphe par [`crate::app::graph_slot::GraphSlot::apply_updates`], sans
//! dépendre du canal de capture. Les imports passent, eux, par le canal du
//! store ([`crate::app::bridge::Bridge::store_channel`]).

pub mod arbitration;
pub mod conflicts;
mod dialogs;
pub mod import;
pub mod manage;
pub mod matrix;
mod rows;
pub mod session;

use gpui::{App, Context, Entity};
use log::{debug, error, info};
use lunar_engine::events::CaptureEvent;
use lunar_engine::ipc::Channel;
use lunar_engine::state::graph::GraphUpdate;
use lunar_ui_logic::file_names::{LocalDateTime, labels_csv_name};

use crate::app::state::{AppState, BUSY_MESSAGE};
use crate::widgets::toast::ToastKind;

pub use dialogs::with_default_extension;

/// Date et heure locales, pour les noms de fichiers proposés.
pub(crate) fn local_now() -> LocalDateTime {
    use chrono::{Datelike, Timelike};
    let now = chrono::Local::now();
    // Champs bornés par chrono : mois 1–12, jour 1–31, heure 0–23…
    let narrow = |value: u32| u8::try_from(value).unwrap_or(u8::MAX);
    LocalDateTime::new(
        now.year(),
        narrow(now.month()),
        narrow(now.day()),
        narrow(now.hour()),
        narrow(now.minute()),
        narrow(now.second()),
    )
}

/// Prend le verrou d'import pour une opération de
/// labels qui commence par un dialogue de fichiers ; refusé et signalé s'il
/// est déjà tenu. À rendre par [`release_import_lock`].
pub(crate) fn take_import_lock(state: &Entity<AppState>, cx: &mut App) -> bool {
    if state.read(cx).capture().is_importing {
        info!("{BUSY_MESSAGE}");
        state.update(cx, |state, cx| {
            state.toast(ToastKind::Info, BUSY_MESSAGE, cx)
        });
        return false;
    }
    state.update(cx, |state, cx| {
        state.update_capture(|capture| capture.is_importing = true, cx)
    });
    true
}

/// Rend le verrou d'import pris par [`take_import_lock`].
pub(crate) fn release_import_lock(state: &Entity<AppState>, cx: &mut App) {
    state.update(cx, |state, cx| {
        state.update_capture(|capture| capture.is_importing = false, cx)
    });
}

/// Canal d'un import de labels : celui du store hors capture ; pendant une
/// capture, un canal détaché (le moteur passe alors par celui de la
/// capture, qu'il ne faut pas détacher).
pub(crate) fn import_channel(state: &Entity<AppState>, cx: &App) -> Channel<CaptureEvent> {
    let state = state.read(cx);
    if state.capture().is_running {
        state.bridge().detached_channel()
    } else {
        state.bridge().store_channel()
    }
}

/// Applique au graphe les mises à jour rendues par une mutation du store,
/// puis relit l'état « non enregistré ».
pub(crate) fn apply_label_updates(
    state: &Entity<AppState>,
    updates: Vec<GraphUpdate>,
    cx: &mut App,
) {
    state.update(cx, |state, cx| {
        if !updates.is_empty() {
            debug!(
                "[Labels] {} mise(s) à jour transmise(s) au graphe",
                updates.len()
            );
            state.graph().apply_updates(updates, cx);
        }
        state.refresh_flags(cx);
    });
}

/// Export du fichier de labels (barre d'outils et menu Fichier) : dialogue
/// d'enregistrement daté puis
/// `export_label_file`, sous le verrou d'import.
pub fn export_labels(state: &mut AppState, cx: &mut Context<AppState>) {
    info!("Export des labels");
    if state.capture().is_importing {
        info!("{BUSY_MESSAGE}");
        state.toast(ToastKind::Info, BUSY_MESSAGE, cx);
        return;
    }
    state.update_capture(|capture| capture.is_importing = true, cx);
    let dialog = dialogs::save_csv("Exporter les labels", &labels_csv_name(&local_now()));
    cx.spawn(async move |this, cx| {
        let path = dialog.await;
        let _ = this.update(cx, |state, cx| {
            state.update_capture(|capture| capture.is_importing = false, cx);
            let Some(path) = path else {
                info!("Aucun chemin sélectionné");
                return;
            };
            state.run_locked(
                cx,
                move |lunar| lunar.export_label_file(path),
                |state, result, cx| match result {
                    Ok(()) => info!("Labels exportés"),
                    Err(err) => {
                        error!("Erreur export labels: {err}");
                        state.show_error(&err, cx);
                    }
                },
            );
        });
    })
    .detach();
}

/// Label saisi dans le graphe : `add_label(mac, ip,
/// label)`, puis la réponse à la vue du graphe — mises à jour appliquées et
/// label gardé, ou retour arrière et erreur affichée.
pub fn commit_graph_label(
    state: &mut AppState,
    node_id: String,
    mac: String,
    ip: String,
    label: String,
    cx: &mut Context<AppState>,
) {
    info!("[Graphe] label de {node_id} : {label:?}");
    state
        .bridge()
        .clone()
        .run(
            cx,
            move |lunar| lunar.add_label(mac, ip, label),
            |state, result, cx| match result {
                Ok(updates) => {
                    state.graph().apply_updates(updates, cx);
                    state.graph().confirm_label_edit(true, cx);
                    state.refresh_flags(cx);
                }
                Err(err) => {
                    state.graph().confirm_label_edit(false, cx);
                    state.show_error(&err, cx);
                }
            },
        )
        .detach();
}
