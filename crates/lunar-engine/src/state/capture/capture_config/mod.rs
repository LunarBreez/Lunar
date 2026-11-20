//! Configuration de la capture (interface, buffers, timeout, snaplen) :
//! validation par bornes et persistance JSON dans le dossier de
//! configuration de l'application.

use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{errors::capture_error::CaptureError, utils::return_device_lookup};
use serde::{Deserialize, Serialize};

/// Nom du fichier JSON de persistance, dans le dossier de configuration
/// de l'application.
const CONFIG_FILE_NAME: &str = "capture_config.json";

/// Taille minimale du buffer pcap, en octets (64 Kio).
const MIN_BUFFER_SIZE: i32 = 64 * 1024;
/// Taille maximale du buffer pcap, en octets (512 Mio).
const MAX_BUFFER_SIZE: i32 = 512 * 1024 * 1024;
/// Capacité minimale du canal capture → traitement, en paquets.
const MIN_CHAN_CAPACITY: i32 = 1;
/// Capacité maximale du canal capture → traitement, en paquets.
const MAX_CHAN_CAPACITY: i32 = 1_000_000;
/// Timeout de lecture pcap minimal, en millisecondes.
const MIN_TIMEOUT_MS: i32 = 1;
/// Timeout de lecture pcap maximal, en millisecondes (10 s).
const MAX_TIMEOUT_MS: i32 = 10_000;
/// Longueur de capture minimale par paquet, en octets.
const MIN_SNAPLEN: i32 = 64;
/// Longueur de capture maximale par paquet, en octets (256 Kio).
const MAX_SNAPLEN: i32 = 262_144;
/// Budget mémoire du pool de buffers de capture : `(chan_capacity + 2) ×
/// snaplen` octets sont réservables en vol. Sans ce plafond, les bornes
/// individuelles autorisent ~244 Gio théoriques.
const MAX_POOL_BYTES: i64 = 1024 * 1024 * 1024;

/// Paramètres d'une capture : interface et réglages pcap. Toujours passer
/// par [`CaptureConfig::setup`]/`validate` avant usage (bornes `MIN_*`/`MAX_*`).
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct CaptureConfig {
    /// Nom pcap de l'interface à ouvrir ; vide = configuration invalide.
    pub device_name: String,
    /// Taille du buffer noyau pcap, en octets.
    pub buffer_size: i32,
    /// Capacité du canal borné capture → traitement, en paquets ; dimensionne
    /// aussi le pool de buffers (`chan_capacity + 2`).
    pub chan_capacity: i32,
    /// Timeout de lecture pcap, en millisecondes ; le thread de traitement en
    /// dérive aussi la borne de silence du drainage à l'arrêt.
    pub timeout: i32,
    /// Longueur maximale retenue par paquet, en octets ; taille de chaque
    /// buffer du pool.
    pub snaplen: i32,
}

impl CaptureConfig {
    /// Configuration de départ : interface par défaut de pcap et réglages
    /// éprouvés (18 Mo, 10 000 paquets, 25 ms, 65 536 o). Non validée :
    /// `device_name` est vide si aucune interface n'est disponible.
    ///
    /// Méthode propre et non trait `Default` : elle interroge pcap, ce qu'un
    /// `Default` ne laisserait pas deviner.
    #[allow(clippy::should_implement_trait)]
    pub fn default() -> Self {
        let device_name = return_device_lookup();
        Self {
            device_name,
            buffer_size: 18_000_000,
            chan_capacity: 10_000,
            timeout: 25,
            snaplen: 65536,
        }
    }
    /// Remplace la configuration par les valeurs reçues (commande
    /// `config_capture`) après les avoir validées. En cas de refus, l'ancienne
    /// configuration est conservée intacte.
    pub fn setup(
        &mut self,
        device_name: String,
        buffer_size: i32,
        chan_capacity: i32,
        timeout: i32,
        snaplen: i32,
    ) -> Result<(), CaptureError> {
        let next = Self {
            device_name: device_name.trim().to_string(),
            buffer_size,
            chan_capacity,
            timeout,
            snaplen,
        };
        next.validate()?;
        *self = next;
        Ok(())
    }

    /// Vérifie l'interface (non vide), les bornes `MIN_*`/`MAX_*` de chaque
    /// champ, puis le budget mémoire du pool (`MAX_POOL_BYTES`). La première
    /// violation rencontrée donne `CaptureError::InvalidConfig`.
    pub fn validate(&self) -> Result<(), CaptureError> {
        if self.device_name.trim().is_empty() {
            return Err(CaptureError::InvalidConfig(
                "interface réseau obligatoire".to_string(),
            ));
        }
        validate_range(
            "buffer_size",
            self.buffer_size,
            MIN_BUFFER_SIZE,
            MAX_BUFFER_SIZE,
        )?;
        validate_range(
            "chan_capacity",
            self.chan_capacity,
            MIN_CHAN_CAPACITY,
            MAX_CHAN_CAPACITY,
        )?;
        validate_range("timeout", self.timeout, MIN_TIMEOUT_MS, MAX_TIMEOUT_MS)?;
        validate_range("snaplen", self.snaplen, MIN_SNAPLEN, MAX_SNAPLEN)?;

        // Budget global du pool de buffers (miroir du dimensionnement de
        // CaptureHandle::start : chan_capacity + 2 buffers de snaplen octets).
        let pool_bytes = (self.chan_capacity as i64 + 2) * self.snaplen as i64;
        if pool_bytes > MAX_POOL_BYTES {
            return Err(CaptureError::InvalidConfig(format!(
                "chan_capacity × snaplen réserverait {pool_bytes} octets de buffers ; \
                 le budget maximum est de {MAX_POOL_BYTES} octets (1 Gio)"
            )));
        }
        Ok(())
    }

    /// Relit la configuration persistée au démarrage de l'application.
    /// `Ok(None)` si le fichier n'existe pas ; un JSON invalide ou une
    /// configuration hors bornes est une erreur, que l'appelant ignore avec
    /// trace pour garder les valeurs par défaut.
    pub fn load_persisted(config_dir: &Path) -> Result<Option<Self>, CaptureError> {
        let path = persisted_config_path(config_dir);
        if !path.exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(&path).map_err(|err| {
            CaptureError::ConfigPersistence(format!("lecture {} impossible: {err}", path.display()))
        })?;
        let config: Self = serde_json::from_str(&content).map_err(|err| {
            CaptureError::ConfigPersistence(format!("JSON invalide dans {}: {err}", path.display()))
        })?;
        config.validate()?;
        Ok(Some(config))
    }

    /// Persiste la configuration (après validation) dans le dossier de
    /// configuration de l'application. Écrite par `config_capture` et à
    /// l'ouverture d'un projet ; relue au prochain démarrage.
    pub fn save_persisted(&self, config_dir: &Path) -> Result<(), CaptureError> {
        self.validate()?;
        let path = persisted_config_path(config_dir);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                CaptureError::ConfigPersistence(format!(
                    "création {} impossible: {err}",
                    parent.display()
                ))
            })?;
        }

        let content = serde_json::to_string_pretty(self).map_err(|err| {
            CaptureError::ConfigPersistence(format!("sérialisation JSON impossible: {err}"))
        })?;
        // Écriture atomique (temporaire + rename), même discipline que les
        // exports CSV : une coupure en pleine écriture ne doit jamais laisser
        // un JSON tronqué qui ferait échouer la restauration au prochain
        // démarrage.
        let tmp_path = path.with_extension("json.tmp");
        fs::write(&tmp_path, content).map_err(|err| {
            CaptureError::ConfigPersistence(format!(
                "écriture {} impossible: {err}",
                tmp_path.display()
            ))
        })?;
        fs::rename(&tmp_path, &path).map_err(|err| {
            let _ = fs::remove_file(&tmp_path);
            CaptureError::ConfigPersistence(format!(
                "renommage vers {} impossible: {err}",
                path.display()
            ))
        })
    }
}

/// Chemin du fichier de persistance : dossier de configuration de
/// l'application (`AppPaths::config_dir`) + `CONFIG_FILE_NAME`.
fn persisted_config_path(config_dir: &Path) -> PathBuf {
    config_dir.join(CONFIG_FILE_NAME)
}

/// Refuse `value` hors de `[min, max]` avec un message qui nomme `field`
/// et les bornes.
fn validate_range(field: &str, value: i32, min: i32, max: i32) -> Result<(), CaptureError> {
    if !(min..=max).contains(&value) {
        return Err(CaptureError::InvalidConfig(format!(
            "{field} doit être compris entre {min} et {max}, reçu {value}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_config() -> CaptureConfig {
        CaptureConfig {
            device_name: "eth0".to_string(),
            buffer_size: MIN_BUFFER_SIZE,
            chan_capacity: MIN_CHAN_CAPACITY,
            timeout: MIN_TIMEOUT_MS,
            snaplen: MIN_SNAPLEN,
        }
    }

    #[test]
    fn validation_rejects_empty_interface() {
        let mut config = valid_config();
        config.device_name.clear();

        assert!(matches!(
            config.validate(),
            Err(CaptureError::InvalidConfig(_))
        ));
    }

    #[test]
    fn validation_rejects_negative_channel_capacity() {
        let mut config = valid_config();
        config.chan_capacity = -1;

        assert!(matches!(
            config.validate(),
            Err(CaptureError::InvalidConfig(_))
        ));
    }

    #[test]
    fn setup_keeps_previous_config_when_invalid() {
        let mut config = valid_config();
        let previous = config.clone();

        let result = config.setup("eth1".to_string(), 1, 10, 10, 1500);

        assert!(matches!(result, Err(CaptureError::InvalidConfig(_))));
        assert_eq!(config.device_name, previous.device_name);
        assert_eq!(config.buffer_size, previous.buffer_size);
    }

    /// Chaque borne individuelle est respectée mais le produit dépasse le
    /// budget mémoire du pool : la configuration est refusée.
    #[test]
    fn validation_rejects_pool_budget_overflow() {
        let mut config = valid_config();
        config.chan_capacity = MAX_CHAN_CAPACITY; // 1 000 000 buffers
        config.snaplen = MAX_SNAPLEN; // × 262 144 o ≈ 244 Gio

        assert!(matches!(
            config.validate(),
            Err(CaptureError::InvalidConfig(_))
        ));
    }

    #[test]
    fn validation_accepts_default_pool_budget() {
        let mut config = CaptureConfig::default();
        config.device_name = "eth0".to_string();
        config.validate().expect("config par défaut sous le budget");
    }

    #[test]
    fn setup_accepts_valid_config() {
        let mut config = valid_config();

        config
            .setup("eth1".to_string(), 131_072, 2, 10, 1500)
            .expect("valid config");

        assert_eq!(config.device_name, "eth1");
        assert_eq!(config.buffer_size, 131_072);
        assert_eq!(config.chan_capacity, 2);
        assert_eq!(config.timeout, 10);
        assert_eq!(config.snaplen, 1500);
    }
}
