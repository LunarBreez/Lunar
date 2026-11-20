//! Point d'entrée de LUNAR : analyse d'abord la ligne de commande
//! ([`lunar::app::cli`]). Sans argument, ouvre l'application ; avec
//! `--lunar-smoke-test`, vérifie le démarrage jusqu'à la première image ;
//! `--version` et `--help` répondent sans rien démarrer.

// Pas de console supplémentaire sous Windows en release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use lunar::app::cli::{self, Invocation};
use lunar::app::{run_gui, smoke};
use lunar::platform::force_x11_backend;

fn main() {
    // Avant tout thread : modifie l'environnement.
    force_x11_backend();
    match cli::parse(std::env::args().skip(1)) {
        Invocation::Gui => std::process::exit(run_gui()),
        Invocation::SmokeTest => {
            cli::attach_parent_console();
            std::process::exit(smoke::run())
        }
        Invocation::Version => {
            cli::attach_parent_console();
            println!("{}", cli::version_text());
        }
        Invocation::Help => {
            cli::attach_parent_console();
            print!("{}", cli::help_text());
        }
        Invocation::Invalid(arg) => {
            cli::attach_parent_console();
            eprintln!("{}", cli::invalid_text(&arg));
            std::process::exit(cli::EXIT_INVALID_ARGUMENT)
        }
    }
}
