//! Prépare un relevé à récupérer pour les essais visuels : importe des
//! matrices de flux par le moteur, enregistre le projet comme autosave et
//! laisse la sentinelle d'une session interrompue. Au lancement suivant,
//! LUNAR propose de récupérer ce relevé.
//!
//! ```text
//! cargo run --example seed_project -- matrice.csv [autre.csv…]
//! ```
//!
//! Les répertoires sont ceux de l'application (`XDG_DATA_HOME`… respectés) :
//! à lancer avec le même environnement que l'application.

use std::process::ExitCode;

use lunar_engine::commandes::project::{AUTOSAVE_FILE, autosave_dir};
use lunar_engine::ipc::Channel;
use lunar_engine::{AppPaths, Lunar};

/// Nom de la sentinelle de session (`commandes::project`).
const SESSION_LOCK_FILE: &str = "session.lock";

/// Point d'entrée : matrices en arguments.
fn main() -> ExitCode {
    let matrices: Vec<String> = std::env::args().skip(1).collect();
    if matrices.is_empty() {
        eprintln!("usage : seed_project <matrice.csv>…");
        return ExitCode::from(2);
    }
    match seed(matrices) {
        Ok(path) => {
            println!("SEED_AUTOSAVE={path}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("{err}");
            ExitCode::FAILURE
        }
    }
}

/// Importe `matrices`, écrit l'autosave et la sentinelle ; rend le chemin de
/// l'autosave.
fn seed(matrices: Vec<String>) -> Result<String, String> {
    let paths = AppPaths::for_default_app().map_err(|err| err.to_string())?;
    let dir = autosave_dir(&paths.data_dir);
    let lunar = Lunar::new(paths).map_err(|err| err.to_string())?;
    lunar
        .import_matrix_files(matrices, Channel::new(|_| {}))
        .map_err(|err| format!("import des matrices : {err}"))?;
    std::fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
    let autosave = dir.join(AUTOSAVE_FILE).to_string_lossy().into_owned();
    lunar
        .save_project(autosave.clone())
        .map_err(|err| format!("enregistrement : {err}"))?;
    std::fs::write(dir.join(SESSION_LOCK_FILE), "0").map_err(|err| err.to_string())?;
    Ok(autosave)
}
