//! Chargement de Npcap sous Windows.
//!
//! `wpcap.dll` est liée en chargement différé (`/DELAYLOAD`, voir `build.rs`).
//! Liée en import implicite, elle devait se trouver dans le chemin de
//! recherche des DLL, où seul le mode « WinPcap API-compatible » de Npcap la
//! place : sans ce mode, ou sans Npcap, Windows refuserait de lancer
//! l'application (0xC0000135, aucune fenêtre, aucune explication).
//!
//! [`ensure_available`] charge `Packet.dll` puis `wpcap.dll` par chemin
//! complet depuis `System32\Npcap`, où Npcap les installe dans tous les
//! modes. Le chargement différé retrouve ensuite le module déjà chargé par
//! son nom. Tout appel à libpcap doit être précédé de cette vérification : si
//! Npcap est absent, un appel différé lèverait une exception et arrêterait
//! LUNAR.

/// Message affiché quand Npcap est introuvable.
pub const MISSING_MESSAGE: &str = "Npcap n'est pas installé sur ce poste. LUNAR en a besoin pour \
capturer le trafic et pour lire les fichiers PCAP. Installez Npcap depuis https://npcap.com, \
puis relancez LUNAR. Les matrices et les labels restent utilisables sans Npcap.";

/// Vérifie que libpcap est utilisable, en chargeant Npcap au premier appel
/// sous Windows. Retourne [`MISSING_MESSAGE`] si Npcap est introuvable.
pub fn ensure_available() -> Result<(), String> {
    if imp::available() {
        Ok(())
    } else {
        Err(MISSING_MESSAGE.to_string())
    }
}

/// Implémentation Windows : charge `Packet.dll` puis `wpcap.dll` depuis
/// `System32\Npcap` par chemin complet, une seule fois par processus.
#[cfg(windows)]
mod imp {
    use std::{
        ffi::c_void,
        os::windows::ffi::OsStrExt,
        path::{Path, PathBuf},
        sync::OnceLock,
    };

    // Dépendances cherchées dans le dossier de la DLL chargée, puis dans
    // System32 ; jamais dans le dossier courant ni dans le PATH.
    /// Drapeau `LoadLibraryExW` : résoudre les dépendances dans le dossier
    /// de la DLL chargée (`Packet.dll` à côté de `wpcap.dll`).
    const LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR: u32 = 0x0000_0100;
    /// Drapeau `LoadLibraryExW` : résoudre les dépendances dans `System32`.
    const LOAD_LIBRARY_SEARCH_SYSTEM32: u32 = 0x0000_0800;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetSystemDirectoryW(buffer: *mut u16, size: u32) -> u32;
        fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
    }

    /// Résultat du chargement, figé au premier appel : les DLL restent
    /// chargées pour toute la durée du processus, inutile de réessayer.
    static LOADED: OnceLock<bool> = OnceLock::new();

    /// Vrai si Npcap a pu être chargé (tentative faite au premier appel).
    pub fn available() -> bool {
        *LOADED.get_or_init(load)
    }

    /// Charge `Packet.dll` puis `wpcap.dll` depuis `System32\Npcap` ; faux si
    /// le dossier système est introuvable ou si l'une des deux DLL manque.
    fn load() -> bool {
        let Some(dir) = system_directory().map(|system| system.join("Npcap")) else {
            log::error!("Npcap : dossier système introuvable");
            return false;
        };
        // Packet.dll d'abord : wpcap.dll en dépend. Les modules restent
        // chargés pour toute la durée du processus.
        let loaded = ["Packet.dll", "wpcap.dll"]
            .iter()
            .all(|dll| load_library(&dir.join(dll)));
        if loaded {
            log::info!("Npcap chargé depuis {}", dir.display());
        } else {
            log::warn!("Npcap introuvable dans {}", dir.display());
        }
        loaded
    }

    /// Dossier `System32` réel (`GetSystemDirectoryW`), ou `None` si l'appel
    /// échoue ou si le chemin dépasse 260 caractères UTF-16.
    fn system_directory() -> Option<PathBuf> {
        let mut buffer = vec![0u16; 260];
        // SAFETY : le tampon est valide et sa taille est passée à l'appel.
        let len = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) };
        if len == 0 || len as usize >= buffer.len() {
            return None;
        }
        buffer.truncate(len as usize);
        Some(PathBuf::from(String::from_utf16_lossy(&buffer)))
    }

    /// Charge la DLL `path` par chemin complet, dépendances limitées à son
    /// dossier et à `System32` ; vrai si le module est chargé.
    fn load_library(path: &Path) -> bool {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY : `wide` est une chaîne UTF-16 terminée par zéro, vivante
        // pendant l'appel ; aucun autre pointeur n'est passé.
        let module = unsafe {
            LoadLibraryExW(
                wide.as_ptr(),
                std::ptr::null_mut(),
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
        };
        !module.is_null()
    }
}

/// Implémentation hors Windows : rien à charger, la vérification est toujours positive.
#[cfg(not(windows))]
mod imp {
    /// Hors Windows, libpcap est une dépendance du paquet : toujours liée.
    pub fn available() -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(windows))]
    #[test]
    fn libpcap_is_always_available_outside_windows() {
        assert_eq!(ensure_available(), Ok(()));
    }

    #[test]
    fn missing_message_tells_what_to_install_and_what_still_works() {
        assert!(MISSING_MESSAGE.contains("https://npcap.com"));
        assert!(MISSING_MESSAGE.contains("relancez LUNAR"));
        assert!(MISSING_MESSAGE.contains("matrices et les labels restent utilisables"));
        // Le mode WinPcap n'est plus un prérequis : ne pas le réclamer.
        assert!(!MISSING_MESSAGE.contains("WinPcap"));
    }
}
