//! DTO sérialisables : miroirs des types `pcap::Device` et
//! associés, exposés à l'interface par la commande `get_devices_list`.

use std::net::IpAddr;

use pcap::{
    Address as PcapAddress, ConnectionStatus as PcapConnectionStatus, Device,
    DeviceFlags as PcapDeviceFlags, IfFlags as PcapIfFlags,
};
use serde::Serialize;

/// Interface réseau telle que présentée au frontend.
#[derive(Debug, Serialize)]
pub struct NetDevice {
    /// Nom système de l'interface, clé passée à la capture (`eth0`,
    /// `\Device\NPF_{…}` sous Windows) ; champ `name`.
    pub name: String,
    /// Description lisible fournie par le pilote (surtout sous Windows),
    /// `null` sinon ; champ `desc`.
    pub desc: Option<String>,
    /// Adresses configurées sur l'interface, tableau vide si aucune ;
    /// champ `addresses`.
    pub addresses: Vec<Address>,
    /// Drapeaux bruts et état de connexion ; champ `flags`.
    pub flags: DeviceFlags,
}

/// Adresse portée par une interface (IP, masque, broadcast, destination).
#[derive(Debug, Serialize)]
pub struct Address {
    /// Adresse IPv4 ou IPv6, sérialisée en texte ; champ `addr`.
    pub addr: IpAddr,
    /// Masque de sous-réseau, `null` si inconnu ; champ `netmask`.
    pub netmask: Option<IpAddr>,
    /// Adresse de diffusion, `null` si absente (IPv6, point à point) ;
    /// champ `broadcast_addr`.
    pub broadcast_addr: Option<IpAddr>,
    /// Adresse du pair sur une liaison point à point, `null` sinon ;
    /// champ `dst_addr`.
    pub dst_addr: Option<IpAddr>,
}

/// Drapeaux d'une interface : bits bruts + état de connexion.
#[derive(Debug, Serialize)]
pub struct DeviceFlags {
    /// Bits bruts `PCAP_IF_*` (loopback, up, running, wireless) ;
    /// champ `if_flags`.
    pub if_flags: IfFlags,
    /// État de connexion, sérialisé par le nom de la variante
    /// (`"Connected"`…) ; champ `connection_status`.
    pub connection_status: ConnectionStatus,
}

/// `IfFlags` de pcap est un bitflag interne.
/// On sérialise la valeur brute pour l’UI.
/// (Tu pourras exposer des booléens dérivés si besoin.)
#[derive(Debug, Serialize)]
pub struct IfFlags {
    /// Valeur brute du bitflag `pcap::IfFlags` (`u32`) ; champ `bits`.
    pub bits: u32,
}

/// État de connexion d'une interface, tel que remonté par pcap.
#[derive(Debug, Serialize)]
pub enum ConnectionStatus {
    /// Le système ne renseigne pas l'état (`"Unknown"`).
    Unknown,
    /// Interface connectée (`"Connected"`) : seul état retenu par le
    /// sélecteur d'interfaces du frontend.
    Connected,
    /// Interface sans liaison (`"Disconnected"`).
    Disconnected,
    /// État sans objet pour cette interface, ex. loopback
    /// (`"NotApplicable"`).
    NotApplicable,
}

/// ====== Conversions depuis pcap::* vers nos DTO ======
impl From<PcapAddress> for Address {
    fn from(a: PcapAddress) -> Self {
        Address {
            addr: a.addr,
            netmask: a.netmask,
            broadcast_addr: a.broadcast_addr,
            dst_addr: a.dst_addr,
        }
    }
}

impl From<PcapIfFlags> for IfFlags {
    fn from(f: PcapIfFlags) -> Self {
        IfFlags { bits: f.bits() }
    }
}

impl From<PcapConnectionStatus> for ConnectionStatus {
    fn from(s: PcapConnectionStatus) -> Self {
        match s {
            PcapConnectionStatus::Unknown => ConnectionStatus::Unknown,
            PcapConnectionStatus::Connected => ConnectionStatus::Connected,
            PcapConnectionStatus::Disconnected => ConnectionStatus::Disconnected,
            PcapConnectionStatus::NotApplicable => ConnectionStatus::NotApplicable,
        }
    }
}

impl From<PcapDeviceFlags> for DeviceFlags {
    fn from(df: PcapDeviceFlags) -> Self {
        DeviceFlags {
            if_flags: df.if_flags.into(),
            connection_status: df.connection_status.into(),
        }
    }
}

impl From<Device> for NetDevice {
    fn from(d: Device) -> Self {
        NetDevice {
            name: d.name,
            desc: d.desc,
            addresses: d.addresses.into_iter().map(Address::from).collect(),
            flags: d.flags.into(),
        }
    }
}

// TODO : RUST DOC
// #[derive(Debug, Error)]
// pub enum PcapError {
//     #[error("Impossible de lister les interfaces réseau")]
//     DeviceListError(#[from] pcap::Error),
// }

#[cfg(test)]
mod tests {
    use super::*;

    /// Garde contre une variante `pcap::ConnectionStatus` ajoutée en amont
    /// sans son pendant dans le DTO : chaque variante doit se mapper vers
    /// elle-même, pas retomber silencieusement sur `Unknown`.
    #[test]
    fn connection_status_maps_each_variant_to_its_own_dto_variant() {
        assert!(matches!(
            ConnectionStatus::from(PcapConnectionStatus::Unknown),
            ConnectionStatus::Unknown
        ));
        assert!(matches!(
            ConnectionStatus::from(PcapConnectionStatus::Connected),
            ConnectionStatus::Connected
        ));
        assert!(matches!(
            ConnectionStatus::from(PcapConnectionStatus::Disconnected),
            ConnectionStatus::Disconnected
        ));
        assert!(matches!(
            ConnectionStatus::from(PcapConnectionStatus::NotApplicable),
            ConnectionStatus::NotApplicable
        ));
    }

    #[test]
    fn if_flags_carries_the_raw_bits_through() {
        let flags: IfFlags = PcapIfFlags::from_bits_truncate(0b101).into();
        assert_eq!(flags.bits, 0b101);
    }
}
