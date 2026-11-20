//! Initialisation du moteur : bannière, snapshot d'environnement dans les
//! logs, labels générés depuis les interfaces réseau, mesure de l'usage CPU.

use log::info;

use crate::paths::AppPaths;

pub mod labels;
pub mod system_info;

/// Journalise un instantané complet de l'environnement au démarrage : hôte,
/// OS, métadonnées de l'application et cible de compilation. Précieux pour
/// diagnostiquer un binaire en production à partir de ses seuls logs.
pub fn log_host_and_app_snapshot(paths: &AppPaths) {
    // ---------- OS / Host ----------
    // Plateforme, famille, arch et extension viennent de `std::env::consts` ;
    // type, version et nom d'hôte de `sysinfo`, la locale des variables
    // d'environnement POSIX (inexistantes sous Windows : « unknown »).
    let platform = std::env::consts::OS;
    let family = std::env::consts::FAMILY;
    let arch = std::env::consts::ARCH;
    let exe_ext = std::env::consts::EXE_EXTENSION;
    let os_type = sysinfo::System::name().unwrap_or_else(|| "unknown".to_string());
    let os_version = sysinfo::System::os_version().unwrap_or_else(|| "unknown".to_string());
    let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
        .unwrap_or_else(|| "unknown".to_string());
    let hostname = sysinfo::System::host_name().unwrap_or_else(|| "unknown".to_string());

    info!("--- Host / OS ---");
    info!("Host platform      : {}", platform);
    info!("Host family        : {}", family);
    info!("Host type          : {}", os_type);
    info!("Host version       : {}", os_version);
    info!("Host arch          : {}", arch);
    info!(
        "Exe extension      : {}",
        if exe_ext.is_empty() {
            "(none)"
        } else {
            exe_ext
        }
    );
    info!("Locale             : {}", locale);
    info!("Hostname           : {}", hostname);

    // Optionnel mais utile pour debug d’exécution
    if let Ok(cwd) = std::env::current_dir() {
        info!("Current dir        : {}", cwd.display());
    }
    if let Ok(exe) = std::env::current_exe() {
        info!("Current exe        : {}", exe.display());
    }

    // ---------- Application (Cargo metadata) ----------

    // Ces env! sont injectées par Cargo au build-time.
    // Attention: env! panique si la variable n’existe pas. option_env! = safe.
    let license: &str = env!("CARGO_PKG_LICENSE");
    let authors: &str = env!("CARGO_PKG_AUTHORS");
    let description: &str = option_env!("CARGO_PKG_DESCRIPTION").unwrap_or("");
    let repository: &str = option_env!("CARGO_PKG_REPOSITORY").unwrap_or("");
    let homepage: &str = option_env!("CARGO_PKG_HOMEPAGE").unwrap_or("");
    let rust_version: &str = option_env!("CARGO_PKG_RUST_VERSION").unwrap_or("");
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };

    // Variables utiles si présentes (selon environnement CI)
    let git_sha = option_env!("GIT_SHA").unwrap_or(""); // à injecter toi-même dans la CI
    let build_time = option_env!("BUILD_TIME").unwrap_or(""); // idem

    info!("--- Application ---");
    info!("App name           : {}", crate::APP_NAME);
    info!("App version        : {}", env!("CARGO_PKG_VERSION"));
    info!("Config dir         : {}", paths.config_dir.display());
    info!("Data dir           : {}", paths.data_dir.display());
    info!("Log dir            : {}", paths.log_dir.display());
    info!("Build profile      : {}", profile);
    info!("License            : {}", license);
    if !authors.is_empty() {
        info!("Authors            : {}", authors);
    }

    if !description.is_empty() {
        info!("Description        : {}", description);
    }
    if !rust_version.is_empty() {
        info!("Rust version req   : {}", rust_version);
    }
    if !homepage.is_empty() {
        info!("Homepage           : {}", homepage);
    }
    if !repository.is_empty() {
        info!("Repository         : {}", repository);
    }
    if !git_sha.is_empty() {
        info!("Git SHA            : {}", git_sha);
    }
    if !build_time.is_empty() {
        info!("Build time         : {}", build_time);
    }

    // ---------- Target / compilation (très utile en cross-build) ----------
    // Beaucoup sont disponibles via env! et cfg!.
    // TARGET n'est pas toujours exposée automatiquement => on la met en option.
    let target = option_env!("TARGET").unwrap_or("unknown"); // selon CI/cargo
    let target_os = std::env::consts::OS;
    let target_arch = std::env::consts::ARCH;

    info!("--- Build / Target ---");
    info!("Target triple      : {}", target);
    info!("Target OS / Arch   : {} / {}", target_os, target_arch);
    info!("Debug assertions   : {}", cfg!(debug_assertions));
    info!(
        "Panic strategy     : {}",
        if cfg!(panic = "abort") {
            "abort"
        } else {
            "unwind"
        }
    );

    // ---------- Platform specific (optionnel) ----------
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        let uid = unsafe { libc::geteuid() };
        let gid = unsafe { libc::getegid() };
        info!("--- Unix details ---");
        info!("EUID / EGID        : {} / {}", uid, gid);

        if let Ok(meta) = std::fs::metadata("/") {
            info!("Root dev/inode     : {}/{}", meta.dev(), meta.ino());
        }
    }

    #[cfg(windows)]
    {
        info!("--- Windows details ---");
        // Sur Windows, on peut aller plus loin via winapi/windows-rs,
        // mais je n’ajoute rien ici pour éviter d’alourdir tes deps.
    }
}

/// Bannière ASCII du démarrage, écrite dans les logs. Elle est rendue sans
/// couleur, l'interface ou le journal décidant de sa mise en forme.
pub fn print_banner() -> String {
    // ASCII art banner
    let banner = r"
  .____                               
  |    |    __ __  ____ _____ _______ 
  |    |   |  |  \/    \\__  \\_  __ \
  |    |___|  |  /   |  \/ __ \|  | \/
  |_______ \____/|___|  (____  /__|   
          \/          \/     \/       
   ";

    banner.to_string()
}
