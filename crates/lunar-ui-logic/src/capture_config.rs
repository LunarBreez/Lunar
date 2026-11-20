//! Panneau de configuration de la capture : valeurs par défaut, choix de
//! l'interface, lecture des champs numériques et charge utile de
//! `config_capture`.
//!
//! - Le choix de l'interface ne dépend pas de l'ordre d'arrivée de
//!   `get_devices_list` et de `get_config_capture` : l'interface configurée
//!   est retenue si la liste la contient, sinon le choix automatique du
//!   sélecteur (première interface connectée, à défaut la première).
//! - Le champ de saisie gpui accepte les lettres, d'où [`parse_numeric`] :
//!   vide vaut 0 (le moteur refuse ensuite la borne), un texte non entier
//!   est refusé avec un message de configuration invalide.

use lunar_engine::dto::{ConnectionStatus, NetDevice};
use lunar_engine::errors::capture_error::CaptureError;
use lunar_engine::state::capture::capture_config::CaptureConfig;

/// Message du moteur pour une interface absente, réutilisé par le panneau
/// quand aucune n'est choisie.
pub const MISSING_INTERFACE: &str = "interface réseau obligatoire";

/// Texte de la liste sans choix.
pub const INTERFACE_PLACEHOLDER: &str = "Choisir une interface...";

/// Champ numérique du panneau, dans l'ordre d'affichage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NumericField {
    /// Taille du buffer noyau pcap, en octets.
    BufferSize,
    /// Capacité du canal capture → traitement, en paquets.
    ChanCapacity,
    /// Délai de lecture pcap, en millisecondes.
    Timeout,
    /// Octets retenus par paquet.
    Snaplen,
}

impl NumericField {
    /// Les quatre champs, dans l'ordre du panneau.
    pub const ALL: [NumericField; 4] = [
        NumericField::BufferSize,
        NumericField::ChanCapacity,
        NumericField::Timeout,
        NumericField::Snaplen,
    ];

    /// Nom du champ de `CaptureConfig`, celui des messages du moteur.
    pub const fn key(self) -> &'static str {
        match self {
            NumericField::BufferSize => "buffer_size",
            NumericField::ChanCapacity => "chan_capacity",
            NumericField::Timeout => "timeout",
            NumericField::Snaplen => "snaplen",
        }
    }

    /// Libellé du champ, au caractère près.
    pub const fn label(self) -> &'static str {
        match self {
            NumericField::BufferSize => "Taille du buffer :",
            NumericField::ChanCapacity => "Nombre de buffers:",
            NumericField::Timeout => "Timeout (ms) :",
            NumericField::Snaplen => "Taille du snaplen :",
        }
    }

    /// Bulle d'aide du bouton « ? » (`data-tooltip`).
    pub const fn help(self) -> &'static str {
        match self {
            NumericField::BufferSize => {
                "Buffer kernel pcap, en octets. Plus il est grand, plus LUNAR absorbe les pics de trafic sans perte côté système."
            }
            NumericField::ChanCapacity => {
                "Capacité du canal interne entre capture et traitement. Trop petit, l'application peut perdre des paquets sous charge; trop grand, elle consomme plus de mémoire."
            }
            NumericField::Timeout => {
                "Délai pcap avant livraison des paquets. Petit, il réduit la latence; plus grand, il favorise le batching."
            }
            NumericField::Snaplen => {
                "Nombre maximum d'octets capturés par paquet. Grand, il garde les paquets complets; petit, il réduit CPU et mémoire mais peut tronquer le décodage."
            }
        }
    }

    /// Valeur proposée avant la lecture de la configuration
    /// (`DEFAULT_CAPTURE_CONFIG`).
    pub const fn default_value(self) -> i32 {
        match self {
            NumericField::BufferSize => 18_000_000,
            NumericField::ChanCapacity => 10_000,
            NumericField::Timeout => 25,
            NumericField::Snaplen => 65_536,
        }
    }

    /// Bornes `min`/`max` de l'`<input type="number">` (celles du moteur).
    pub const fn bounds(self) -> (i32, i32) {
        match self {
            NumericField::BufferSize => (65_536, 536_870_912),
            NumericField::ChanCapacity => (1, 1_000_000),
            NumericField::Timeout => (1, 10_000),
            NumericField::Snaplen => (64, 262_144),
        }
    }

    /// Valeur du champ dans `config`.
    pub const fn value_in(self, config: &CaptureConfig) -> i32 {
        match self {
            NumericField::BufferSize => config.buffer_size,
            NumericField::ChanCapacity => config.chan_capacity,
            NumericField::Timeout => config.timeout,
            NumericField::Snaplen => config.snaplen,
        }
    }
}

/// Libellé d'une interface dans la liste : `nom - description`, ou le nom
/// seul.
pub fn interface_label(device: &NetDevice) -> String {
    match device.desc.as_deref() {
        Some(desc) if !desc.is_empty() => format!("{} - {desc}", device.name),
        _ => device.name.clone(),
    }
}

/// Statut de connexion en français.
pub const fn connection_status_text(status: &ConnectionStatus) -> &'static str {
    match status {
        ConnectionStatus::Connected => "Connecté",
        ConnectionStatus::Disconnected => "Déconnecté",
        ConnectionStatus::Unknown => "Inconnu",
        ConnectionStatus::NotApplicable => "Non applicable",
    }
}

/// Choix automatique du sélecteur : première interface connectée, à défaut
/// la première ; `None` pour une liste vide.
pub fn default_interface(devices: &[NetDevice]) -> Option<usize> {
    devices
        .iter()
        .position(|device| matches!(device.flags.connection_status, ConnectionStatus::Connected))
        .or_else(|| (!devices.is_empty()).then_some(0))
}

/// Interface à afficher : celle de la configuration si la liste la
/// contient, sinon [`default_interface`].
pub fn select_interface(devices: &[NetDevice], configured: &str) -> Option<usize> {
    let configured = configured.trim();
    devices
        .iter()
        .position(|device| !configured.is_empty() && device.name == configured)
        .or_else(|| default_interface(devices))
}

/// Lit un champ numérique (`v-model.number`) : vide vaut 0 comme
/// `Number("")`, sinon un entier signé en base 10.
pub fn parse_numeric(field: NumericField, text: &str) -> Result<i32, CaptureError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(0);
    }
    let digits = trimmed.strip_prefix(['+', '-']).unwrap_or(trimmed);
    let well_formed = !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit());
    match trimmed.parse::<i32>() {
        Ok(value) if well_formed => Ok(value),
        _ => Err(CaptureError::InvalidConfig(format!(
            "{} doit être un nombre entier, reçu « {trimmed} »",
            field.key()
        ))),
    }
}

/// Erreur affichée sous un champ numérique pendant la saisie : seul un
/// texte qui n'est pas un entier est signalé, les bornes restent au moteur.
pub fn numeric_input_error(text: &str) -> Option<&'static str> {
    parse_numeric(NumericField::BufferSize, text)
        .is_err()
        .then_some("Nombre entier attendu")
}

/// Configuration à envoyer à `config_capture` :
/// interface obligatoire (sans espaces de bord), champs dans l'ordre de
/// [`NumericField::ALL`].
pub fn normalize_config(
    device_name: Option<&str>,
    values: [&str; 4],
) -> Result<CaptureConfig, CaptureError> {
    let device_name = device_name.map(str::trim).unwrap_or_default();
    if device_name.is_empty() {
        return Err(CaptureError::InvalidConfig(MISSING_INTERFACE.to_owned()));
    }
    let [buffer, chan, timeout, snaplen] = values;
    Ok(CaptureConfig {
        device_name: device_name.to_owned(),
        buffer_size: parse_numeric(NumericField::BufferSize, buffer)?,
        chan_capacity: parse_numeric(NumericField::ChanCapacity, chan)?,
        timeout: parse_numeric(NumericField::Timeout, timeout)?,
        snaplen: parse_numeric(NumericField::Snaplen, snaplen)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use lunar_engine::dto::{Address, DeviceFlags, IfFlags};

    fn device(name: &str, desc: Option<&str>, status: ConnectionStatus) -> NetDevice {
        NetDevice {
            name: name.to_owned(),
            desc: desc.map(str::to_owned),
            addresses: vec![Address {
                addr: [127, 0, 0, 1].into(),
                netmask: None,
                broadcast_addr: None,
                dst_addr: None,
            }],
            flags: DeviceFlags {
                if_flags: IfFlags { bits: 0 },
                connection_status: status,
            },
        }
    }

    fn devices() -> Vec<NetDevice> {
        vec![
            device("lo", None, ConnectionStatus::NotApplicable),
            device("eth0", Some("Carte Intel"), ConnectionStatus::Disconnected),
            device("eth1", None, ConnectionStatus::Connected),
        ]
    }

    #[test]
    fn libelle_avec_ou_sans_description() {
        let list = devices();
        assert_eq!(interface_label(&list[0]), "lo");
        assert_eq!(interface_label(&list[1]), "eth0 - Carte Intel");
        let empty = device("wlan0", Some(""), ConnectionStatus::Unknown);
        assert_eq!(interface_label(&empty), "wlan0");
    }

    #[test]
    fn statuts_en_francais() {
        assert_eq!(
            connection_status_text(&ConnectionStatus::Connected),
            "Connecté"
        );
        assert_eq!(
            connection_status_text(&ConnectionStatus::Disconnected),
            "Déconnecté"
        );
        assert_eq!(
            connection_status_text(&ConnectionStatus::Unknown),
            "Inconnu"
        );
        assert_eq!(
            connection_status_text(&ConnectionStatus::NotApplicable),
            "Non applicable"
        );
    }

    #[test]
    fn choix_automatique_prefere_une_interface_connectee() {
        assert_eq!(default_interface(&devices()), Some(2));
        let none_connected = vec![
            device("lo", None, ConnectionStatus::NotApplicable),
            device("eth0", None, ConnectionStatus::Disconnected),
        ];
        assert_eq!(default_interface(&none_connected), Some(0));
        assert_eq!(default_interface(&[]), None);
    }

    #[test]
    fn interface_configuree_retenue_si_presente() {
        let list = devices();
        assert_eq!(select_interface(&list, "eth0"), Some(1));
        assert_eq!(select_interface(&list, " lo "), Some(0), "espaces ignorés");
        assert_eq!(
            select_interface(&list, "usb0"),
            Some(2),
            "absente : choix automatique"
        );
        assert_eq!(select_interface(&list, ""), Some(2));
        assert_eq!(select_interface(&[], "eth0"), None);
    }

    #[test]
    fn champ_numerique_vide_vaut_zero() {
        assert_eq!(parse_numeric(NumericField::Timeout, "").ok(), Some(0));
        assert_eq!(parse_numeric(NumericField::Timeout, "  ").ok(), Some(0));
    }

    #[test]
    fn champ_numerique_entier() {
        assert_eq!(
            parse_numeric(NumericField::Snaplen, " 65536 ").ok(),
            Some(65_536)
        );
        assert_eq!(parse_numeric(NumericField::Timeout, "-5").ok(), Some(-5));
        assert_eq!(parse_numeric(NumericField::Timeout, "+7").ok(), Some(7));
    }

    #[test]
    fn champ_numerique_non_entier_refuse() {
        for text in ["1.5", "abc", "1e3", "-", "99999999999", "12 34"] {
            let err = parse_numeric(NumericField::ChanCapacity, text);
            let Err(CaptureError::InvalidConfig(message)) = err else {
                panic!("{text:?} devrait être refusé : {err:?}");
            };
            assert!(message.starts_with("chan_capacity doit être un nombre entier"));
        }
        assert_eq!(numeric_input_error("12a"), Some("Nombre entier attendu"));
        assert_eq!(numeric_input_error(""), None);
        assert_eq!(numeric_input_error("4096"), None);
    }

    #[test]
    fn configuration_normalisee() {
        let config = normalize_config(Some(" eth0 "), ["18000000", "10000", "25", "65536"]);
        let Ok(config) = config else {
            panic!("configuration valide refusée");
        };
        assert_eq!(config.device_name, "eth0");
        assert_eq!(
            NumericField::ALL.map(|field| field.value_in(&config)),
            [18_000_000, 10_000, 25, 65_536]
        );
    }

    #[test]
    fn interface_obligatoire() {
        for device in [None, Some(""), Some("   ")] {
            let err = normalize_config(device, ["1", "1", "1", "64"]);
            assert!(
                matches!(&err, Err(CaptureError::InvalidConfig(m)) if m == MISSING_INTERFACE),
                "{device:?} : {err:?}"
            );
        }
    }

    #[test]
    fn premier_champ_invalide_signale() {
        let err = normalize_config(Some("eth0"), ["18000000", "x", "y", "65536"]);
        assert!(
            matches!(&err, Err(CaptureError::InvalidConfig(m)) if m.starts_with("chan_capacity")),
            "{err:?}"
        );
    }

    #[test]
    fn valeurs_par_defaut_dans_les_bornes() {
        for field in NumericField::ALL {
            let (min, max) = field.bounds();
            assert!((min..=max).contains(&field.default_value()), "{field:?}");
        }
        assert_eq!(NumericField::ChanCapacity.label(), "Nombre de buffers:");
    }
}
