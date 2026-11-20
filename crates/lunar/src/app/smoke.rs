//! Smoke test de démarrage (`--lunar-smoke-test`) : marqueurs sur stdout,
//! et dans le fichier `LUNAR_SMOKE_LOG_PATH` s'il est demandé.
//!
//! La validation va jusqu'à la première image de la vraie fenêtre (moteur
//! construit, vues rendues), sans autosave ni proposition de récupération.
//! Échec : fenêtre impossible à ouvrir, moteur en erreur ou aucune image en
//! [`FIRST_FRAME_TIMEOUT`].

use std::cell::RefCell;
use std::env;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{App, Application};
use lunar_engine::setup::print_banner;
use lunar_engine::utils::lookup_default_device;
use lunar_engine::{AppPaths, Lunar};

use crate::app::{StartOptions, actions, logs, open_main_window};
use crate::assets::Assets;
use crate::platform::install_x11_wakeup;
use crate::widgets;

/// Argument qui déclenche le smoke test.
pub const SMOKE_ARG: &str = "--lunar-smoke-test";

/// Variable désignant le fichier où dupliquer les marqueurs (vide : stdout
/// seul).
pub const SMOKE_LOG_ENV: &str = "LUNAR_SMOKE_LOG_PATH";

/// Marqueur d'un smoke test réussi, recherché par la CI.
pub const VALIDATION_LOG: &str = "LUNAR_STARTUP_VALIDATION=OK";

/// Marqueur de la première image rendue.
pub const FIRST_FRAME_LOG: &str = "LUNAR_SMOKE_FIRST_FRAME=OK";

/// Délai accordé à la première image.
pub const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(30);

/// Exécute le smoke test et rend le code de sortie (0 : validation OK, 1 :
/// échec journalisé).
pub fn run() -> i32 {
    logs::init(None);
    let logger = match SmokeLogger::new() {
        Ok(logger) => Rc::new(RefCell::new(logger)),
        Err(err) => return fail(&err.to_string()),
    };
    if let Err(err) = preamble(&mut logger.borrow_mut()) {
        return fail(&err);
    }
    let paths = match AppPaths::for_default_app() {
        Ok(paths) => paths,
        Err(err) => return fail(&format!("répertoires de l'application : {err}")),
    };
    let lunar = match Lunar::new(paths) {
        Ok(lunar) => Arc::new(lunar),
        Err(err) => return fail(&format!("moteur : {err}")),
    };

    // Le smoke test se termine depuis la boucle gpui : `Application::run`
    // ne rend pas la main sur toutes les plates-formes.
    Application::new()
        .with_assets(Assets)
        .run(move |cx: &mut App| {
            widgets::init(cx);
            actions::bind_keys(cx);
            let on_frame = Rc::clone(&logger);
            let first_frame = Box::new(move |_: &mut gpui::Window, _: &mut App| {
                let result = {
                    let mut logger = on_frame.borrow_mut();
                    logger
                        .line(FIRST_FRAME_LOG)
                        .and_then(|()| logger.line(VALIDATION_LOG))
                };
                match result {
                    Ok(()) => finish(0),
                    Err(err) => finish(fail(&err)),
                }
            });
            let options = StartOptions {
                recovery: None,
                first_frame: Some(first_frame),
            };
            if let Err(err) = open_main_window(cx, lunar, options) {
                finish(fail(&format!("ouverture de la fenêtre : {err}")));
            }
            install_x11_wakeup(cx, &crate::app::root::RENDERS);
            cx.activate(true);
            cx.spawn(async move |cx| {
                cx.background_executor().timer(FIRST_FRAME_TIMEOUT).await;
                finish(fail(&format!(
                    "aucune image rendue en {} s",
                    FIRST_FRAME_TIMEOUT.as_secs()
                )));
            })
            .detach();
        });
    // Atteint seulement si la boucle s'arrête sans verdict.
    fail("boucle d'événements arrêtée avant la première image")
}

/// Marqueurs de début : bannière, interface par défaut (ou son
/// indisponibilité, qui n'est pas un échec), cible.
fn preamble(logger: &mut SmokeLogger) -> Result<(), String> {
    logger.line("LUNAR_STARTUP_VALIDATION=BEGIN")?;
    logger.line(&print_banner())?;
    match lookup_default_device() {
        Ok(device) => {
            logger.line(&format!("Using device {}", device.name))?;
            logger.line(&format!("LUNAR_SMOKE_DEVICE={}", device.name))?;
        }
        Err(err) => {
            logger.line(&format!("Using device unavailable ({err})"))?;
            logger.line(&format!("LUNAR_SMOKE_DEVICE_UNAVAILABLE={err}"))?;
        }
    }
    logger.line(&format!(
        "LUNAR_SMOKE_TARGET={} {}",
        env::consts::OS,
        env::consts::ARCH
    ))
}

/// Quitte le processus avec `code`, sorties vidées.
fn finish(code: i32) -> ! {
    let _ = io::stdout().flush();
    let _ = io::stderr().flush();
    std::process::exit(code)
}

/// Écrit `LUNAR_STARTUP_VALIDATION=FAILED <err>` sur stderr et, si
/// demandé, à la fin du fichier de marqueurs. Rend le code d'échec.
fn fail(err: &str) -> i32 {
    let message = format!("LUNAR_STARTUP_VALIDATION=FAILED {err}");
    eprintln!("{message}");
    if let Some(path) = smoke_log_path()
        && let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path)
    {
        let _ = writeln!(file, "{message}");
    }
    1
}

/// Chemin du fichier de marqueurs, `None` si la variable est absente ou
/// vide.
fn smoke_log_path() -> Option<PathBuf> {
    env::var_os(SMOKE_LOG_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// Écrit chaque ligne sur stdout et, si demandé, dans le fichier de
/// marqueurs.
struct SmokeLogger {
    /// Fichier de marqueurs (tronqué à l'ouverture).
    file: Option<File>,
}

impl SmokeLogger {
    /// Crée le fichier de marqueurs s'il est demandé.
    fn new() -> io::Result<Self> {
        let file = match smoke_log_path() {
            Some(path) => Some(File::create(path)?),
            None => None,
        };
        Ok(Self { file })
    }

    /// Écrit `message` sur stdout puis dans le fichier ; une erreur
    /// d'écriture dans le fichier fait échouer le smoke test.
    fn line(&mut self, message: &str) -> Result<(), String> {
        println!("{message}");
        if let Some(file) = self.file.as_mut() {
            writeln!(file, "{message}").map_err(|err| err.to_string())?;
        }
        Ok(())
    }
}
