//! Mémoire rendue au système après une réinitialisation du relevé.
//!
//! Un import est traité sur un fil de l'exécuteur de fond de gpui, le graphe
//! sur le fil de rendu et ForceAtlas2 sur le sien. Sous glibc, chaque fil
//! alloue dans son arène : à la réinitialisation, la mémoire est libérée mais
//! reste résidente dans les arènes, que glibc ne rend pas d'elle-même. D'un
//! cycle import → réinitialisation au suivant, le travail tombe sur d'autres
//! fils et d'autres arènes se remplissent : la mémoire résidente montait d'un
//! cycle à l'autre alors que le tas vivant (`mallinfo2`) restait stable
//! (matrice de 1 000 flux, `script/e2e/soak-lunar-memory.sh`).
//!
//! Après « Relevé réinitialisé », [`release_after_reset`] demande donc à
//! l'allocateur de rendre les pages libres de toutes ses arènes
//! (`malloc_trim(0)` : 5 à 9 Mo rendus à chaque réinitialisation dans cette
//! mesure), une fois le fil ForceAtlas2 passé lui aussi à vide, et
//! journalise la mémoire résidente et le tas vivant (`[Mémoire]`, lu par la
//! mesure). Ailleurs que sous Linux glibc, rien n'est fait.

use std::time::Duration;

use gpui::App;
use log::debug;

/// Délai avant de rendre la mémoire : le fil ForceAtlas2 reçoit la
/// réinitialisation par sa file et libère son modèle peu après.
const RELEASE_DELAY: Duration = Duration::from_millis(500);

/// Rend au système, en fond et après [`RELEASE_DELAY`], la mémoire libérée
/// par la réinitialisation du relevé ; journalise la mémoire résidente
/// avant et après.
pub fn release_after_reset(cx: &mut App) {
    let executor = cx.background_executor().clone();
    executor
        .spawn({
            let executor = executor.clone();
            async move {
                executor.timer(RELEASE_DELAY).await;
                let before = resident_kib();
                if release_free_memory() {
                    debug!(
                        "[Mémoire] pages libres rendues au système après la réinitialisation : \
                         VmRSS {} Ko -> {} Ko, allocations vivantes {} Ko",
                        before.unwrap_or(0),
                        resident_kib().unwrap_or(0),
                        allocated_kib().unwrap_or(0)
                    );
                }
            }
        })
        .detach();
}

/// `malloc_trim(0)` : rend les pages libres de toutes les arènes de glibc.
/// Vrai si l'appel a été fait.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn release_free_memory() -> bool {
    // SAFETY: malloc_trim ne prend que le seuil de marge à garder en haut du
    // tas ; il verrouille lui-même chaque arène et peut être appelé de
    // n'importe quel fil.
    unsafe {
        libc::malloc_trim(0);
    }
    true
}

/// Octets alloués et non libérés, toutes arènes et blocs `mmap` compris
/// (`mallinfo2`), en Kio : ce qui reste quand la fragmentation est rendue.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn allocated_kib() -> Option<u64> {
    // SAFETY: mallinfo2 ne fait que lire les compteurs des arènes.
    let info = unsafe { libc::mallinfo2() };
    Some(((info.uordblks + info.hblkhd) / 1024) as u64)
}

/// Sans glibc : compteur indisponible.
#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn allocated_kib() -> Option<u64> {
    None
}

/// Sans glibc : rien à rendre explicitement.
#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn release_free_memory() -> bool {
    false
}

/// Mémoire résidente du processus (VmRSS, en Kio), lue dans
/// `/proc/self/status` ; `None` hors Linux.
fn resident_kib() -> Option<u64> {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| parse_vm_rss(&status))
}

/// Valeur de la ligne `VmRSS:` d'un `/proc/<pid>/status`, en Kio.
fn parse_vm_rss(status: &str) -> Option<u64> {
    status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|value| value.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vmrss_lu_dans_le_statut_du_processus() {
        let status = "Name:\tlunar\nVmPeak:\t  300000 kB\nVmRSS:\t  212364 kB\nThreads:\t62\n";
        assert_eq!(parse_vm_rss(status), Some(212_364));
        assert_eq!(parse_vm_rss("Name:\tlunar\n"), None);
        assert_eq!(parse_vm_rss("VmRSS:\t  n/a kB\n"), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn vmrss_du_processus_courant() {
        assert!(resident_kib().is_some_and(|kib| kib > 0));
    }

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    #[test]
    fn pages_libres_rendues_apres_liberation_sur_un_autre_fil() {
        // Arène d'un autre fil : 64 Mio en blocs de 4 Kio, un bloc sur 64
        // gardé vivant. Le reste, libéré entre des blocs vivants, ne peut pas
        // être rendu en rognant le haut du tas : il reste résident jusqu'à
        // malloc_trim, comme les restes d'un import.
        let pinned = std::thread::spawn(|| {
            let blocks: Vec<Vec<u8>> = (0..16_384).map(|i| vec![i as u8; 4_096]).collect();
            blocks.into_iter().step_by(64).collect::<Vec<_>>()
        })
        .join()
        .unwrap_or_default();
        let before = resident_kib().unwrap_or(0);
        assert!(release_free_memory());
        let after = resident_kib().unwrap_or(u64::MAX);
        assert!(
            after + 32 * 1024 < before,
            "VmRSS {before} Ko -> {after} Ko ({} blocs gardés)",
            pinned.len()
        );
    }
}
