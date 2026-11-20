//! Monitoring système : usage CPU moyen, relevé à la demande, et
//! énumération des interfaces réseau locales.
//!
//! L'interface interroge un [`CpuSampler`] à son propre rythme (une fois par
//! seconde pour la barre de statut) : pas de thread, pas de runtime
//! asynchrone.

use std::time::{Duration, Instant};

use serde::Serialize;
use sysinfo::System;

/// Période de rafraîchissement de l'usage CPU attendue par la barre de
/// statut.
pub const CPU_SAMPLE_PERIOD: Duration = Duration::from_secs(1);

/// Relevé d'usage CPU moyen, en %.
#[derive(Debug, Serialize, Clone)]
pub struct SystemInfo {
    /// Usage CPU moyen sur tous les cœurs, en pourcentage.
    pub cpu_usage: f32,
}

/// Échantillonneur d'usage CPU, à interroger périodiquement.
///
/// `System` appartient au seul échantillonneur : aucun verrou à prendre,
/// donc ni empoisonnement ni chemin d'erreur qui bouclerait sans pause.
/// L'usage CPU de `sysinfo` est un écart entre deux relevés : la
/// première mesure n'a de sens qu'après [`sysinfo::MINIMUM_CPU_UPDATE_INTERVAL`].
pub struct CpuSampler {
    /// État `sysinfo`, rafraîchi à chaque relevé.
    sys: System,
    /// Instant du dernier rafraîchissement.
    last_refresh: Instant,
    /// Dernière valeur mesurée, rendue telle quelle entre deux relevés trop
    /// rapprochés.
    last: SystemInfo,
}

impl CpuSampler {
    /// Échantillonneur prêt : un premier relevé sert de référence.
    pub fn new() -> Self {
        let mut sys = System::new();
        sys.refresh_cpu_usage();
        Self {
            sys,
            last_refresh: Instant::now(),
            last: SystemInfo { cpu_usage: 0.0 },
        }
    }

    /// Usage CPU moyen depuis le relevé précédent. Appelé plus souvent que
    /// [`sysinfo::MINIMUM_CPU_UPDATE_INTERVAL`], rend la dernière valeur au
    /// lieu d'un écart non significatif.
    pub fn sample(&mut self) -> SystemInfo {
        if self.last_refresh.elapsed() >= sysinfo::MINIMUM_CPU_UPDATE_INTERVAL {
            self.sys.refresh_cpu_usage();
            self.last_refresh = Instant::now();
            self.last = SystemInfo {
                cpu_usage: average_cpu_usage(self.sys.cpus().iter().map(|cpu| cpu.cpu_usage())),
            };
        }
        self.last.clone()
    }
}

impl Default for CpuSampler {
    fn default() -> Self {
        Self::new()
    }
}

/// Moyenne des usages par cœur ; 0 si aucun cœur n'est connu.
fn average_cpu_usage(per_core: impl ExactSizeIterator<Item = f32>) -> f32 {
    let count = per_core.len();
    if count == 0 {
        0.0
    } else {
        per_core.sum::<f32>() / count as f32
    }
}

/// Interfaces réseau locales (via `netdev`), journalisées au passage.
pub fn get_interfaces() -> Vec<netdev::Interface> {
    let interfaces = netdev::get_interfaces();

    for iface in &interfaces {
        print!("Interface: {}", iface.name);

        // MAC
        if let Some(mac) = iface.mac_addr {
            print!(", Mac: {}", mac);
        } else {
            print!(", Mac: none");
        }

        // IPv4
        for ipv4 in &iface.ipv4 {
            print!(", IPv4: {}", ipv4);
        }

        // IPv6
        for ipv6 in &iface.ipv6 {
            print!(", IPv6: {}", ipv6);
        }
        println!();
    }
    interfaces.clone()
}

#[cfg(test)]
mod tests {
    use super::{CpuSampler, average_cpu_usage};

    #[test]
    fn sampler_reports_a_percentage() {
        let mut sampler = CpuSampler::new();
        std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
        let usage = sampler.sample().cpu_usage;
        assert!(
            (0.0..=100.0).contains(&usage),
            "usage hors bornes : {usage}"
        );
        // Relevé immédiat : la dernière valeur est rendue telle quelle.
        assert_eq!(sampler.sample().cpu_usage, usage);
    }

    #[test]
    fn average_of_no_core_is_zero() {
        assert_eq!(average_cpu_usage(std::iter::empty()), 0.0);
    }

    #[test]
    fn average_spans_every_core() {
        assert_eq!(average_cpu_usage([10.0, 20.0, 60.0].into_iter()), 30.0);
    }
}
