//! Vue LUNAR de la couche liaison multi-LINKTYPE de `packet_parser` 7.
//!
//! Depuis packet_parser 7, `PacketFlowOwned::data_link` est un
//! [`LinkLayerOwned`] qui représente fidèlement Ethernet, RAW IP, Linux
//! SLL/SLL2 et IEEE 802.11 — sans inventer de MAC ni d'EtherType. La matrice,
//! le graphe et l'export CSV de LUNAR raisonnent en champs textuels ; ce
//! module centralise la projection typé → texte ([`LinkView`]) et son inverse
//! pour le réimport CSV ([`matrix_link_from_text`]), qui doit rester exact
//! sur toute identité produite par un export LUNAR.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::OnceLock;

use packet_parser::owned::{
    DataLinkOwned, LinkLayerOwned, LinkLayerOwnedKind, LinuxSll2LinkOwned, LinuxSllLinkOwned,
    PacketFlowOwned,
};
use packet_parser::parse::data_link::ethertype::Ethertype;
use packet_parser::parse::data_link::vlan_tag::VlanTag;
use packet_parser::{LinkType, LinuxArphrdType, LinuxCookedPacketType, MacAddress};

/// Projection textuelle de la couche liaison, telle que la matrice, le graphe
/// et le CSV la consomment. Pour Ethernet, chaque champ est identique octet
/// pour octet à l'ancien `DataLinkOwned` textuel de packet_parser 6.
#[derive(Debug, Clone)]
pub struct LinkView {
    /// MAC source en hexadécimal séparé par `:` ; vide si le LINKTYPE n'en
    /// porte pas (RAW IP) ou si l'en-tête cooked n'en exposait aucune.
    pub source_mac: String,
    /// MAC destination, même forme ; toujours vide pour RAW IP et SLL/SLL2
    /// (adresse source unique).
    pub destination_mac: String,
    /// Nom du protocole de liaison (ex-`ethertype` : « IPv4 », « ARP »,
    /// « Unknown (0xABCD) »…).
    pub protocol: String,
    /// Identifiant du VLAN 802.1Q (tag interne) ; `None` hors trame Ethernet
    /// taguée.
    pub vlan_id: Option<u16>,
}

impl LinkView {
    /// Projette la couche liaison en champs textuels. Les LINKTYPE sans
    /// adresse (RAW IP) ou à adresse source unique (SLL/SLL2) produisent des
    /// MAC vides plutôt qu'inventées : les consommateurs traitent déjà la MAC
    /// vide (labels, endpoints observés).
    pub fn of(link: &LinkLayerOwned) -> Self {
        match link.kind() {
            LinkLayerOwnedKind::Ethernet(frame) => Self {
                source_mac: frame.source_mac.to_string(),
                destination_mac: frame.destination_mac.to_string(),
                protocol: frame.ethertype.name(),
                vlan_id: frame.vlan.as_ref().map(|v| v.id),
            },
            LinkLayerOwnedKind::Ieee80211(frame) => Self {
                source_mac: frame.source_mac.to_string(),
                destination_mac: frame.destination_mac.to_string(),
                protocol: frame.snap_protocol.name(),
                vlan_id: None,
            },
            LinkLayerOwnedKind::LinuxSll(details) => Self {
                source_mac: details
                    .source_address
                    .as_deref()
                    .map(format_hw_address)
                    .unwrap_or_default(),
                destination_mac: String::new(),
                protocol: link.network_protocol().to_string(),
                vlan_id: None,
            },
            LinkLayerOwnedKind::LinuxSll2(details) => Self {
                source_mac: details
                    .source_address
                    .as_deref()
                    .map(format_hw_address)
                    .unwrap_or_default(),
                destination_mac: String::new(),
                protocol: link.network_protocol().to_string(),
                vlan_id: None,
            },
            // RAW IP et variants futurs (`#[non_exhaustive]`) : pas d'adresse
            // de liaison réelle.
            _ => Self {
                source_mac: String::new(),
                destination_mac: String::new(),
                protocol: link.network_protocol().to_string(),
                vlan_id: None,
            },
        }
    }
}

/// Adresse matérielle en hexadécimal séparé par `:` (même forme que l'ancien
/// affichage MAC), quelle que soit sa longueur (SLL n'impose pas 6 octets).
fn format_hw_address(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// Parse l'adresse matérielle variable d'un en-tête Linux cooked. Le slot
/// SLL/SLL2 contient au plus huit octets ; une adresse vide signifie que
/// l'en-tête n'en exposait aucune. Cette forme est volontairement distincte
/// de [`mac_from_text`], qui impose les six octets d'une MAC Ethernet.
pub fn cooked_address_from_text(text: &str) -> Result<Option<Vec<u8>>, String> {
    if text.is_empty() {
        return Ok(None);
    }

    let octets: Vec<&str> = text.split(':').collect();
    if !(1..=8).contains(&octets.len()) {
        return Err(format!(
            "adresse cooked invalide : « {text} » (attendu : 1 à 8 octets hexadécimaux)"
        ));
    }

    octets
        .into_iter()
        .map(|octet| {
            if octet.len() != 2 {
                return Err(format!(
                    "adresse cooked invalide : « {text} » (chaque octet doit avoir deux chiffres)"
                ));
            }
            u8::from_str_radix(octet, 16)
                .map_err(|_| format!("adresse cooked invalide : « {text} »"))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

/// Parse une MAC textuelle `aa:bb:cc:dd:ee:ff`. `None` si la forme est
/// invalide — la validation d'import s'appuie dessus pour rejeter la ligne
/// avec un message précis au lieu de dégrader silencieusement.
pub fn mac_from_text(text: &str) -> Option<MacAddress> {
    MacAddress::try_from(text.to_string()).ok()
}

/// Inverse exact de [`Ethertype::name`] : nom connu (« IPv4 », « LLDP »…) ou
/// forme « Unknown (0xABCD) ». `None` pour toute autre chaîne.
pub fn ethertype_from_name(name: &str) -> Option<Ethertype> {
    if let Some(hex) = name
        .strip_prefix("Unknown (0x")
        .and_then(|s| s.strip_suffix(')'))
    {
        return u16::from_str_radix(hex, 16).ok().map(Ethertype);
    }

    // Table inverse construite en balayant `static_name` sur tout l'espace
    // u16 : aucune duplication de la table amont, donc aucune dérive possible.
    static NAMES: OnceLock<HashMap<&'static str, u16>> = OnceLock::new();
    let names = NAMES.get_or_init(|| {
        let mut map = HashMap::new();
        for code in 0..=u16::MAX {
            if let Some(name) = Ethertype(code).static_name() {
                map.entry(name).or_insert(code);
            }
        }
        map
    });
    names.get(name).map(|&code| Ethertype(code))
}

/// Inverse de l'affichage du protocole SLL/SLL2. Les protocoles reconnus ont
/// un nom (`IPv4`, `ARP`…) ; les autres sont affichés par `packet_parser`
/// sous la forme numérique `0xABCD`. La forme Ethernet historique
/// `Unknown (0xABCD)` reste aussi acceptée pour la compatibilité.
pub fn cooked_protocol_from_text(name: &str) -> Option<u16> {
    if let Some(hex) = name.strip_prefix("0x") {
        return (hex.len() == 4)
            .then(|| u16::from_str_radix(hex, 16).ok())
            .flatten();
    }
    ethertype_from_name(name).map(|protocol| protocol.0)
}

/// Construit la représentation canonique de l'identité LFMS cooked. Les
/// détails propres au point d'observation (direction, ARPHRD, longueur
/// déclarée, index d'interface et champ réservé) reçoivent des sentinelles
/// stables et ne segmentent donc pas la matrice. Seuls l'adresse réellement
/// exposée, le protocole et le LINKTYPE restent identifiants.
fn canonical_cooked_link(
    link_type: LinkType,
    source_address: Option<Vec<u8>>,
    protocol: u16,
) -> LinkLayerOwned {
    let address_length = source_address.as_ref().map_or(0, Vec::len);
    if link_type == LinkType::LINUX_SLL {
        LinkLayerOwned::linux_sll(LinuxSllLinkOwned::new(
            LinuxCookedPacketType::HOST,
            LinuxArphrdType(0),
            address_length as u16,
            source_address,
            protocol,
        ))
    } else {
        debug_assert_eq!(link_type, LinkType::LINUX_SLL2);
        LinkLayerOwned::linux_sll2(LinuxSll2LinkOwned::new(
            protocol,
            0,
            0,
            LinuxArphrdType(0),
            LinuxCookedPacketType::HOST,
            address_length as u8,
            source_address,
        ))
    }
}

/// Retourne une clé de matrice normalisée sans modifier le paquet source.
/// Ethernet et RAW restent empruntés sans allocation ; un flux SLL/SLL2 est
/// cloné seulement si ses métadonnées d'observation diffèrent de la forme
/// canonique. Le paquet IPC garde ainsi l'en-tête cooked fidèle reçu du
/// parseur, tandis que la matrice fusionne les observations multi-sondes.
pub fn matrix_flow_identity(flow: &PacketFlowOwned) -> Cow<'_, PacketFlowOwned> {
    let canonical_link = match flow.data_link.kind() {
        LinkLayerOwnedKind::LinuxSll(details) => canonical_cooked_link(
            LinkType::LINUX_SLL,
            details.source_address.clone(),
            details.protocol,
        ),
        LinkLayerOwnedKind::LinuxSll2(details) => canonical_cooked_link(
            LinkType::LINUX_SLL2,
            details.source_address.clone(),
            details.protocol,
        ),
        _ => return Cow::Borrowed(flow),
    };

    if canonical_link == flow.data_link {
        Cow::Borrowed(flow)
    } else {
        let mut canonical = flow.clone();
        canonical.data_link = canonical_link;
        Cow::Owned(canonical)
    }
}

/// Variante possédée de [`matrix_flow_identity`], utilisée par l'import CSV
/// qui possède déjà son flux et peut donc le normaliser sans clone complet.
pub fn into_matrix_flow_identity(mut flow: PacketFlowOwned) -> PacketFlowOwned {
    let canonical_link = match flow.data_link.kind() {
        LinkLayerOwnedKind::LinuxSll(details) => Some(canonical_cooked_link(
            LinkType::LINUX_SLL,
            details.source_address.clone(),
            details.protocol,
        )),
        LinkLayerOwnedKind::LinuxSll2(details) => Some(canonical_cooked_link(
            LinkType::LINUX_SLL2,
            details.source_address.clone(),
            details.protocol,
        )),
        _ => None,
    };
    if let Some(canonical_link) = canonical_link {
        flow.data_link = canonical_link;
    }
    flow
}

/// Reconstruit une couche Ethernet depuis les champs textuels d'une ligne CSV
/// (chemin historique, conservé pour les appelants et tests Ethernet).
///
/// - deux MAC vides avec un protocole IPv4/IPv6 et sans VLAN redeviennent un
///   lien RAW IP (export d'une capture RAW réimporté à l'identique) ;
/// - sinon la ligne redevient Ethernet. Une MAC vide devient l'adresse nulle
///   et un protocole inconnu `Ethertype(0)` : ces cas sont rejetés en amont
///   par [`crate::matrix::FlowMatrixRow::validate`], les replis n'existent
///   que pour rester infaillible ici.
///
pub fn ethernet_link_from_text(
    source_mac: &str,
    destination_mac: &str,
    protocol: &str,
    vlan: Option<VlanTag>,
) -> LinkLayerOwned {
    if source_mac.is_empty() && destination_mac.is_empty() && vlan.is_none() {
        match protocol {
            "IPv4" => return LinkLayerOwned::raw_ipv4(),
            "IPv6" => return LinkLayerOwned::raw_ipv6(),
            _ => {}
        }
    }

    LinkLayerOwned::ethernet(DataLinkOwned {
        destination_mac: mac_from_text(destination_mac).unwrap_or(MacAddress([0; 6])),
        source_mac: mac_from_text(source_mac).unwrap_or(MacAddress([0; 6])),
        ethertype: ethertype_from_name(protocol).unwrap_or(Ethertype(0)),
        vlan,
        // Une ligne de matrice ne porte qu'un VLAN (le tag interne, cf. LFMS) :
        // la pile QinQ complète de la trame d'origine n'est pas sérialisée, on
        // ne l'invente pas à la relecture.
        vlan_stack: Vec::new(),
    })
}
/// En-têtes IP minimaux, sans charge utile : IPv4 127.0.0.1 → 127.0.0.1
/// (protocole 253, réservé aux expérimentations) et IPv6 ::1 → ::1 (en-tête
/// suivant 59 : aucun).
const MINIMAL_IPV4: [u8; 20] = [
    0x45, 0, 0, 20, 0, 0, 0x40, 0, 64, 253, 0x3b, 0xeb, 127, 0, 0, 1, 127, 0, 0, 1,
];
/// En-tête IPv6 minimal ::1 → ::1, en-tête suivant 59 (aucun), sans
/// charge utile.
const MINIMAL_IPV6: [u8; 40] = [
    0x60, 0, 0, 0, 0, 0, 59, 64, //
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, //
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1,
];
/// En-têtes de famille loopback BSD, en little-endian : `AF_INET` vaut 2
/// partout, 30 est l'`AF_INET6` de Darwin.
const NULL_AF_INET: [u8; 4] = [2, 0, 0, 0];
/// En-tête loopback BSD pour IPv6 : `AF_INET6` de Darwin (30), en
/// little-endian.
const NULL_AF_INET6: [u8; 4] = [30, 0, 0, 0];

/// Couche IP brute telle que `packet_parser` la marque sous `link_type` :
/// LINKTYPE_NULL pour le loopback BSD (11.2.0), LINKTYPE_IPV4/IPV6 pour les
/// niveaux pelés d'un tunnel. L'API publique ne construit une couche IP brute
/// que sous LINKTYPE_RAW, alors qu'une clé réimportée doit rester dans le
/// vrai DLT : on la demande au parseur lui-même, sur un en-tête IP minimal,
/// une fois par couple. `RawIpLinkOwned` ne retient que la version, si bien
/// que tout paquet de même version donne cette même couche.
fn parsed_raw_ip_link(link_type: LinkType, protocol: &str) -> Result<LinkLayerOwned, String> {
    type Cell = OnceLock<Result<LinkLayerOwned, String>>;
    static NULL_V4: Cell = OnceLock::new();
    static NULL_V6: Cell = OnceLock::new();
    static PEELED_V4: Cell = OnceLock::new();
    static PEELED_V6: Cell = OnceLock::new();
    let (cell, family, packet): (&Cell, &[u8], &[u8]) = match (link_type, protocol) {
        (LinkType::NULL, "IPv4") => (&NULL_V4, &NULL_AF_INET, &MINIMAL_IPV4),
        (LinkType::NULL, "IPv6") => (&NULL_V6, &NULL_AF_INET6, &MINIMAL_IPV6),
        (LinkType::IPV4, "IPv4") => (&PEELED_V4, &[], &MINIMAL_IPV4),
        (LinkType::IPV6, "IPv6") => (&PEELED_V6, &[], &MINIMAL_IPV6),
        _ => {
            return Err(format!(
                "protocole {} invalide : « {protocol} » (attendu : IPv4 ou IPv6)",
                crate::lfms::link_type_name(link_type)
            ));
        }
    };
    cell.get_or_init(|| {
        let frame = [family, packet].concat();
        packet_parser::parse::parse(link_type, &frame)
            .map(|flow| flow.to_owned_flow().data_link)
            .map_err(|error| format!("couche IP de référence indécodable : {error}"))
    })
    .clone()
}

/// Inverse DLT-aware de [`LinkView::of`] pour une ligne de matrice LFMS.
/// L'identité Ethernet/RAW est reconstruite comme auparavant ; SLL/SLL2
/// utilisent la forme canonique de [`matrix_flow_identity`], sans inventer
/// ni sérialiser les métadonnées locales du point d'observation.
pub fn matrix_link_from_text(
    link_type: LinkType,
    source_address: &str,
    destination_address: &str,
    protocol: &str,
    vlan: Option<VlanTag>,
) -> Result<LinkLayerOwned, String> {
    match link_type {
        // Niveau pelé d'un tunnel IP (IP-in-IP, 6in4, GRE, GTP-U) : de l'IP
        // nu, sans trame, que packet_parser marque LINKTYPE_IPV4/IPV6. Le
        // reconstruire en Ethernet lui inventait des MAC 00:00:00:00:00:00 à
        // l'aller-retour LFMS.
        LinkType::ETHERNET
            if source_address.is_empty()
                && destination_address.is_empty()
                && vlan.is_none()
                && matches!(protocol, "IPv4" | "IPv6") =>
        {
            let peeled = if protocol == "IPv4" {
                LinkType::IPV4
            } else {
                LinkType::IPV6
            };
            parsed_raw_ip_link(peeled, protocol)
        }
        LinkType::ETHERNET => Ok(LinkLayerOwned::ethernet(DataLinkOwned {
            destination_mac: mac_from_text(destination_address).unwrap_or(MacAddress([0; 6])),
            source_mac: mac_from_text(source_address).unwrap_or(MacAddress([0; 6])),
            ethertype: ethertype_from_name(protocol).unwrap_or(Ethertype(0)),
            vlan,
            vlan_stack: Vec::new(),
        })),
        LinkType::RAW => match protocol {
            "IPv4" => Ok(LinkLayerOwned::raw_ipv4()),
            "IPv6" => Ok(LinkLayerOwned::raw_ipv6()),
            _ => Err(format!(
                "protocole RAW invalide : « {protocol} » (attendu : IPv4 ou IPv6)"
            )),
        },
        LinkType::NULL => parsed_raw_ip_link(LinkType::NULL, protocol),
        LinkType::LINUX_SLL | LinkType::LINUX_SLL2 => {
            let source_address = cooked_address_from_text(source_address)?;
            let protocol = cooked_protocol_from_text(protocol).ok_or_else(|| {
                format!(
                    "protocole cooked invalide : « {protocol} » (attendu : nom connu ou 0xNNNN)"
                )
            })?;
            Ok(canonical_cooked_link(link_type, source_address, protocol))
        }
        _ => Err(format!(
            "type de liaison non reconstructible : {}",
            link_type.0
        )),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// L'aller-retour texte → lien → texte est exact pour une ligne Ethernet.
    #[test]
    fn ethernet_round_trip_is_exact() {
        let link = ethernet_link_from_text("aa:bb:cc:dd:ee:ff", "11:22:33:44:55:66", "IPv4", None);
        let view = LinkView::of(&link);
        assert_eq!(view.source_mac, "aa:bb:cc:dd:ee:ff");
        assert_eq!(view.destination_mac, "11:22:33:44:55:66");
        assert_eq!(view.protocol, "IPv4");
        assert_eq!(view.vlan_id, None);
    }

    /// Tout nom produit par `Ethertype::name` doit être inversible, y compris
    /// la forme « Unknown (0x…) » : c'est la garantie de réimport CSV.
    #[test]
    fn every_ethertype_name_is_invertible() {
        for code in [0x0000u16, 0x0800, 0x86DD, 0x0806, 0x88CC, 0x8892, 0xABCD] {
            let name = Ethertype(code).name();
            assert_eq!(
                ethertype_from_name(&name),
                Some(Ethertype(code)),
                "nom non inversible : {name}"
            );
        }
    }

    /// Une ligne RAW IP (MACs vides, protocole IPv4/IPv6) se réimporte sous
    /// sa forme RAW, identique à une capture RAW live.
    #[test]
    fn raw_ip_round_trip_keeps_raw_kind() {
        let link = ethernet_link_from_text("", "", "IPv4", None);
        assert_eq!(link, LinkLayerOwned::raw_ipv4());
        let view = LinkView::of(&link);
        assert_eq!(view.source_mac, "");
        assert_eq!(view.destination_mac, "");
        assert_eq!(view.protocol, "IPv4");
    }

    /// Une adresse cooked n'est pas forcément une MAC Ethernet : sa longueur
    /// variable et un protocole inconnu doivent traverser le CSV à l'identique.
    #[test]
    fn cooked_text_round_trip_accepts_variable_address_and_unknown_protocol() {
        assert_eq!(
            cooked_address_from_text("01:02:03:04").unwrap(),
            Some(vec![1, 2, 3, 4])
        );
        assert!(cooked_address_from_text("00:01:02:03:04:05:06:07:08").is_err());
        assert_eq!(cooked_protocol_from_text("0xABCD"), Some(0xabcd));

        for link_type in [LinkType::LINUX_SLL, LinkType::LINUX_SLL2] {
            let link = matrix_link_from_text(link_type, "01:02:03:04", "", "0xABCD", None).unwrap();
            let view = LinkView::of(&link);
            assert_eq!(link.link_type(), link_type);
            assert_eq!(view.source_mac, "01:02:03:04");
            assert_eq!(view.destination_mac, "");
            assert_eq!(view.protocol, "0xABCD");
            assert_eq!(view.vlan_id, None);
        }
    }
    #[test]
    fn invalid_mac_is_rejected() {
        assert!(mac_from_text("pas-une-mac").is_none());
        assert!(mac_from_text("").is_none());
        assert!(mac_from_text("aa:bb:cc:dd:ee:ff").is_some());
    }

    /// Une ligne loopback réimportée retrouve exactement la couche d'un
    /// paquet loopback capturé : même forme IP brute, même DLT,
    /// donc même clé de matrice et fusion possible avec une nouvelle capture.
    #[test]
    fn null_loopback_rows_rebuild_the_captured_link_layer() {
        // UDP 127.0.0.1:12345 → 127.0.0.1:12346, famille en big-endian cette
        // fois : l'ordre d'octets de l'en-tête ne change pas la couche.
        let mut frame = vec![0, 0, 0, 2];
        frame.extend_from_slice(&[
            0x45, 0, 0, 28, 0, 1, 0, 0, 64, 17, 0x7c, 0xce, 127, 0, 0, 1, 127, 0, 0, 1,
        ]);
        frame.extend_from_slice(&[0x30, 0x39, 0x30, 0x3a, 0, 8, 0, 0]);
        let captured = packet_parser::parse::parse(LinkType::NULL, &frame)
            .unwrap()
            .to_owned_flow()
            .data_link;

        let rebuilt = matrix_link_from_text(LinkType::NULL, "", "", "IPv4", None).unwrap();
        assert_eq!(rebuilt, captured);
        assert_eq!(rebuilt.link_type(), LinkType::NULL);
        let view = LinkView::of(&rebuilt);
        assert_eq!(
            (view.source_mac.as_str(), view.protocol.as_str()),
            ("", "IPv4")
        );

        let ipv6 = matrix_link_from_text(LinkType::NULL, "", "", "IPv6", None).unwrap();
        assert_eq!(ipv6.link_type(), LinkType::NULL);
        assert_eq!(LinkView::of(&ipv6).protocol, "IPv6");
        assert!(matrix_link_from_text(LinkType::NULL, "", "", "ARP", None).is_err());
    }

    /// Dans une matrice Ethernet, une ligne IP sans MAC ni VLAN est un niveau
    /// pelé de tunnel : elle se reconstruit en IP brute LINKTYPE_IPV4/IPV6,
    /// comme packet_parser la produit, et non en trame à MAC nulles.
    #[test]
    fn peeled_tunnel_rows_rebuild_as_raw_ip_without_inventing_macs() {
        let ipv4_packet = [
            0x45, 0, 0, 28, 0, 1, 0, 0, 64, 17, 0x7c, 0xce, 127, 0, 0, 1, 127, 0, 0, 1, //
            0x30, 0x39, 0x30, 0x3a, 0, 8, 0, 0,
        ];
        let peeled = packet_parser::parse::parse(LinkType::IPV4, &ipv4_packet)
            .unwrap()
            .to_owned_flow()
            .data_link;

        let rebuilt = matrix_link_from_text(LinkType::ETHERNET, "", "", "IPv4", None).unwrap();
        assert_eq!(rebuilt, peeled);
        let view = LinkView::of(&rebuilt);
        assert_eq!(
            (view.source_mac.as_str(), view.destination_mac.as_str()),
            ("", "")
        );
        let ipv6 = matrix_link_from_text(LinkType::ETHERNET, "", "", "IPv6", None).unwrap();
        assert_eq!(ipv6.link_type(), LinkType::IPV6);

        // Une vraie trame garde sa reconstruction Ethernet.
        let framed =
            matrix_link_from_text(LinkType::ETHERNET, "aa:bb:cc:dd:ee:ff", "", "IPv4", None)
                .unwrap();
        assert_eq!(framed.link_type(), LinkType::ETHERNET);
        let arp = matrix_link_from_text(LinkType::ETHERNET, "", "", "ARP", None).unwrap();
        assert_eq!(arp.link_type(), LinkType::ETHERNET);
    }
}
