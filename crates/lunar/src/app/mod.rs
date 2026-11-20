//! Coquille de l'application LUNAR : état, pont vers le moteur,
//! fenêtre principale, actions et raccourcis, journal, ligne de commande.
//!
//! Démarrage ([`run_gui`]) : journal ([`logs`]), moteur (`Lunar::new`) et
//! autosave, puis la fenêtre « LUNAR » 1280×800 maximisée
//! ([`open_main_window`]). Si la session précédente s'est mal terminée, la
//! récupération de son autosave est proposée. Fermer la fenêtre ou
//! Quitter passe par la confirmation d'un relevé non enregistré, puis
//! arrête la capture et l'autosave.
//!
//! Brancher un module de l'interface :
//! - un panneau : son bras dans [`panels::build_panel_view`] ;
//! - le moteur : [`bridge::Bridge::run`] depuis l'entité du module, ou
//!   [`state::AppState::run_locked`] pour une opération exclusive ;
//! - l'état : `Entity<AppState>` (lecture, `observe`, `subscribe` sur
//!   [`state::AppEvent`]) ;
//! - les notifications : [`state::AppState::toast`] ;
//! - le graphe : [`graph_slot`] (un seul fichier à changer).

pub mod about;
pub mod actions;
pub mod bridge;
pub mod cli;
pub mod file_dialog;
pub mod graph_slot;
pub mod investigation;
pub mod labels;
pub mod logs;
pub mod memory;
pub mod offline;
pub mod packet_log;
pub mod panels;
pub mod root;
pub mod smoke;
pub mod state;
pub mod status_bar;
pub mod toolbar;

use std::sync::Arc;

use gpui::{
    App, AppContext, Application, Bounds, TitlebarOptions, WindowBounds, WindowHandle,
    WindowOptions, px, size,
};
use log::{error, info};
use lunar_engine::{AppPaths, Lunar};

use crate::app::root::{FirstFrameCallback, LunarWindow, RENDERS};
use crate::app::state::AppState;
use crate::assets::Assets;
use crate::platform::install_x11_wakeup;
use crate::widgets;

/// Titre de la fenêtre.
pub const WINDOW_TITLE: &str = "LUNAR";

/// Taille de la fenêtre avant maximisation.
pub const WINDOW_SIZE: (f32, f32) = (1280., 800.);

/// Options d'ouverture de la fenêtre principale.
#[derive(Default)]
pub struct StartOptions {
    /// Autosave d'une session interrompue à proposer.
    pub recovery: Option<String>,
    /// Rappel après la première image (smoke test).
    pub first_frame: Option<FirstFrameCallback>,
}

/// Ouvre la fenêtre principale sur `lunar`.
pub fn open_main_window(
    cx: &mut App,
    lunar: Arc<Lunar>,
    options: StartOptions,
) -> Result<WindowHandle<LunarWindow>, String> {
    let bounds = Bounds::centered(None, size(px(WINDOW_SIZE.0), px(WINDOW_SIZE.1)), cx);
    let StartOptions {
        recovery,
        first_frame,
    } = options;
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Maximized(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some(WINDOW_TITLE.into()),
                ..Default::default()
            }),
            app_id: Some(lunar_engine::paths::DEFAULT_APP_ID.to_owned()),
            window_min_size: Some(size(px(800.), px(500.))),
            ..Default::default()
        },
        move |window, cx| {
            let state = cx.new(|cx| AppState::new(lunar, window, cx));
            if let Some(path) = recovery {
                state.update(cx, |state, cx| state.offer_recovery(path, cx));
            }
            let on_close = state.clone();
            window.on_window_should_close(cx, move |_, cx| {
                on_close.update(cx, |state, cx| state.on_close_requested(cx));
                // La fermeture passe par la confirmation : c'est elle qui
                // quitte l'application.
                false
            });
            cx.new(|cx| LunarWindow::new(state, first_frame, window, cx))
        },
    )
    .map_err(|err| format!("{err:#}"))
}

/// Application graphique ; rend le code de sortie du processus.
pub fn run_gui() -> i32 {
    let paths = match AppPaths::for_default_app() {
        Ok(paths) => paths,
        Err(err) => {
            logs::init(None);
            error!("Répertoires de l'application introuvables : {err}");
            return 1;
        }
    };
    logs::init(Some(&paths.log_dir));
    let lunar = match Lunar::new(paths) {
        Ok(lunar) => Arc::new(lunar),
        Err(err) => {
            error!("Démarrage du moteur impossible : {err}");
            return 1;
        }
    };
    if let Err(err) = lunar.start_autosave() {
        error!("Autosave indisponible : {err}");
    }
    let recovery = lunar.get_recovery_offer();
    let engine = Arc::clone(&lunar);
    Application::new()
        .with_assets(Assets)
        .run(move |cx: &mut App| {
            widgets::init(cx);
            actions::bind_keys(cx);
            let on_quit = Arc::clone(&engine);
            cx.on_app_quit(move |_| {
                // Filet : la fermeture normale a déjà arrêté l'autosave.
                if let Err(err) = on_quit.stop_autosave() {
                    error!("Arrêt de l'autosave impossible : {err}");
                }
                async {}
            })
            .detach();
            let options = StartOptions {
                recovery,
                first_frame: None,
            };
            if let Err(err) = open_main_window(cx, engine, options) {
                error!("Ouverture de la fenêtre impossible : {err}");
                cx.quit();
                return;
            }
            install_x11_wakeup(cx, &RENDERS);
            cx.activate(true);
        });
    if let Err(err) = lunar.stop_autosave() {
        error!("Arrêt de l'autosave impossible : {err}");
    }
    info!("LUNAR fermé");
    0
}
