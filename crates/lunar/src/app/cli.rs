//! Arguments du binaire :
//! `--version` et `--help` répondent et rendent la main sans ouvrir de
//! fenêtre ni toucher au réseau, un argument inconnu est refusé au lieu de
//! démarrer l'application. Analyse faite dans `main`, avant tout démarrage
//! de gpui.

use crate::app::smoke::SMOKE_ARG;

/// Nom du programme dans les textes de la ligne de commande.
pub const PROGRAM: &str = "lunar";

/// Code de sortie d'un argument refusé.
pub const EXIT_INVALID_ARGUMENT: i32 = 2;

/// Ce que demande la ligne de commande.
#[derive(Debug, PartialEq, Eq)]
pub enum Invocation {
    /// Aucun argument : l'application graphique.
    Gui,
    /// `--lunar-smoke-test` : validation de démarrage (première image).
    SmokeTest,
    /// `--version` / `-V` : affiche la version et rend la main.
    Version,
    /// `--help` / `-h` : affiche l'aide et rend la main.
    Help,
    /// Argument non reconnu, ou combinaison refusée.
    Invalid(String),
}

/// Analyse les arguments, nom du programme exclu.
pub fn parse<I>(args: I) -> Invocation
where
    I: IntoIterator<Item = String>,
{
    let args: Vec<String> = args.into_iter().collect();
    match args.as_slice() {
        [] => Invocation::Gui,
        [arg] => match arg.as_str() {
            "--version" | "-V" => Invocation::Version,
            "--help" | "-h" => Invocation::Help,
            SMOKE_ARG => Invocation::SmokeTest,
            other => Invocation::Invalid(other.to_string()),
        },
        [_, second, ..] => Invocation::Invalid(second.to_string()),
    }
}

/// Texte de `--version` : la version du crate.
pub fn version_text() -> String {
    format!("{PROGRAM} {}", env!("CARGO_PKG_VERSION"))
}

/// Texte de `--help`.
pub fn help_text() -> String {
    format!(
        "{version}
Cartographie passive des flux réseau (interface native GPUI).

Usage :
  {PROGRAM}                          ouvre l'application
  {PROGRAM} --version, -V            affiche la version
  {PROGRAM} --help, -h               affiche cette aide
  {PROGRAM} {SMOKE_ARG}       vérifie le démarrage jusqu'à la première image,
                                   puis rend la main (marqueurs aussi écrits dans
                                   le fichier désigné par LUNAR_SMOKE_LOG_PATH)
",
        version = version_text()
    )
}

/// Message d'un argument refusé.
pub fn invalid_text(arg: &str) -> String {
    format!("{PROGRAM} : argument non reconnu « {arg} » (voir {PROGRAM} --help)")
}

/// Sous Windows, le binaire release n'a pas de console propre
/// (`windows_subsystem = "windows"`) : on se rattache à celle du shell qui
/// l'a lancé pour que `--version`, `--help` et le smoke test s'y affichent.
/// Sans console parente (double-clic), l'appel échoue sans conséquence.
#[cfg(windows)]
pub fn attach_parent_console() {
    /// `ATTACH_PARENT_PROCESS` de l'API Win32.
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        /// Rattache le processus à la console d'un autre processus.
        fn AttachConsole(process_id: u32) -> i32;
    }
    // SAFETY : appel Win32 sans pointeur ni état partagé ; un échec signifie
    // seulement l'absence de console parente.
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

/// Hors Windows, le binaire hérite de la console du shell : rien à faire.
#[cfg(not(windows))]
pub fn attach_parent_console() {}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn sans_argument_l_application_s_ouvre() {
        assert_eq!(parse(args(&[])), Invocation::Gui);
    }

    #[test]
    fn version_et_aide_reconnues_sous_les_deux_formes() {
        assert_eq!(parse(args(&["--version"])), Invocation::Version);
        assert_eq!(parse(args(&["-V"])), Invocation::Version);
        assert_eq!(parse(args(&["--help"])), Invocation::Help);
        assert_eq!(parse(args(&["-h"])), Invocation::Help);
        assert_eq!(parse(args(&["--lunar-smoke-test"])), Invocation::SmokeTest);
    }

    #[test]
    fn arguments_inconnus_ou_en_trop_refuses() {
        assert_eq!(
            parse(args(&["--verbose"])),
            Invocation::Invalid("--verbose".to_string())
        );
        assert_eq!(
            parse(args(&["capture.pcap"])),
            Invocation::Invalid("capture.pcap".to_string())
        );
        assert_eq!(
            parse(args(&["--version", "--help"])),
            Invocation::Invalid("--help".to_string())
        );
        assert!(invalid_text("--verbose").contains("lunar --help"));
        assert!(invalid_text("--verbose").contains("« --verbose »"));
    }

    #[test]
    fn la_version_affichee_est_celle_du_crate() {
        assert_eq!(
            version_text(),
            format!("lunar {}", env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn l_aide_liste_chaque_argument_reconnu() {
        let help = help_text();
        for arg in ["--version", "-V", "--help", "-h", SMOKE_ARG] {
            assert!(help.contains(arg), "{arg} absent de l'aide");
        }
        assert!(help.contains("LUNAR_SMOKE_LOG_PATH"));
        assert!(help.starts_with(&version_text()));
    }
}
