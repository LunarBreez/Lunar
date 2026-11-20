//! Table fixe des usages et du chiffrement par protocole.
//!
//! Des déductions bornées, jamais des conclusions : l'usage
//! et le chiffrement d'un flux ne se déduisent QUE du protocole que le parser
//! a reconnu (libellé de l'arête : applicatif, sinon transport, réseau ou
//! liaison). Jamais d'un numéro de port, jamais d'un volume. Ce qui n'est pas
//! dans la table reste « non classé » et « indéterminé ».
//!
//! Conséquences assumées : un flux seulement étiqueté TCP ou UDP est
//! « indéterminé » (son contenu n'est pas reconnu) ; une session TLS est
//! « chiffrée » mais son usage reste « non classé » (on ne sait pas ce
//! qu'elle transporte). La table est relisible comme le glossaire, et un test
//! échoue si un protocole émis par le parser n'y figure pas.

use std::collections::HashMap;
use std::sync::LazyLock;

use super::glossary::normalize_token;

/// Catégorie d'usage d'un protocole.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum UsageCategory {
    /// « Web ».
    Web,
    /// « Noms et découverte ».
    NamesAndDiscovery,
    /// « Infrastructure réseau ».
    NetworkInfrastructure,
    /// « Administration et supervision ».
    AdministrationAndMonitoring,
    /// « Fichiers et bases de données ».
    FilesAndDatabases,
    /// « Messagerie et publication ».
    MessagingAndPublishing,
    /// « Industriel et opérationnel ».
    IndustrialAndOperational,
    /// « Tunnels et VPN ».
    TunnelsAndVpn,
}

impl UsageCategory {
    /// Libellé affiché de la catégorie.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Web => "Web",
            Self::NamesAndDiscovery => "Noms et découverte",
            Self::NetworkInfrastructure => "Infrastructure réseau",
            Self::AdministrationAndMonitoring => "Administration et supervision",
            Self::FilesAndDatabases => "Fichiers et bases de données",
            Self::MessagingAndPublishing => "Messagerie et publication",
            Self::IndustrialAndOperational => "Industriel et opérationnel",
            Self::TunnelsAndVpn => "Tunnels et VPN",
        }
    }
}

/// Les catégories d'usage, dans l'ordre de `USAGE_CATEGORIES`.
pub const USAGE_CATEGORIES: [UsageCategory; 8] = [
    UsageCategory::Web,
    UsageCategory::NamesAndDiscovery,
    UsageCategory::NetworkInfrastructure,
    UsageCategory::AdministrationAndMonitoring,
    UsageCategory::FilesAndDatabases,
    UsageCategory::MessagingAndPublishing,
    UsageCategory::IndustrialAndOperational,
    UsageCategory::TunnelsAndVpn,
];

/// Usage d'un flux : une catégorie de la table, ou « non classé ».
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Usage {
    /// Catégorie de la table.
    Category(UsageCategory),
    /// « Non classé » : protocole absent de la table, ou qui ne dit rien de
    /// l'usage.
    Unclassified,
}

impl Usage {
    /// Libellé affiché de l'usage.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Category(category) => category.label(),
            Self::Unclassified => "Non classé",
        }
    }
}

/// Chiffrement d'un flux, tiré du seul protocole reconnu.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Confidentiality {
    /// « Chiffré ».
    Encrypted,
    /// « En clair ».
    Cleartext,
    /// « Indéterminé ».
    Undetermined,
}

impl Confidentiality {
    /// Libellé affiché du statut.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Encrypted => "Chiffré",
            Self::Cleartext => "En clair",
            Self::Undetermined => "Indéterminé",
        }
    }
}

/// Ce que dit la table d'un protocole.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProtocolTraits {
    /// `None` : protocole connu, mais sans usage déterminable.
    pub usage: Option<UsageCategory>,
    /// Chiffrement du protocole.
    pub confidentiality: Confidentiality,
}

/// Ce que dit la table sur un protocole qu'elle ne contient pas.
pub const UNCLASSIFIED: ProtocolTraits = ProtocolTraits {
    usage: None,
    confidentiality: Confidentiality::Undetermined,
};

/// Raccourci de la table : chiffré.
const C: Confidentiality = Confidentiality::Encrypted;
/// Raccourci de la table : en clair.
const P: Confidentiality = Confidentiality::Cleartext;
/// Raccourci de la table : indéterminé.
const I: Confidentiality = Confidentiality::Undetermined;

/// Raccourci des catégories d'usage dans la table.
use UsageCategory as U;

/// Libellés tels que le parser les émet, normalisés pour la recherche.
const TABLE: &[(&str, Option<UsageCategory>, Confidentiality)] = &[
    // Applicatif (règles de `parse/dispatch.rs`)
    ("HTTP", Some(U::Web), P),
    ("QUIC", None, C),
    ("TLS", None, C),
    ("SSH", Some(U::AdministrationAndMonitoring), C),
    ("SNMP", Some(U::AdministrationAndMonitoring), P),
    ("DNS", Some(U::NamesAndDiscovery), P),
    ("mDNS", Some(U::NamesAndDiscovery), P),
    ("LLMNR", Some(U::NamesAndDiscovery), P),
    ("NBNS", Some(U::NamesAndDiscovery), P),
    ("SSDP", Some(U::NamesAndDiscovery), P),
    ("SRVLOC", Some(U::NamesAndDiscovery), P),
    ("DHCP", Some(U::NetworkInfrastructure), P),
    ("DHCPv6", Some(U::NetworkInfrastructure), P),
    ("NTP", Some(U::NetworkInfrastructure), P),
    ("PTP", Some(U::NetworkInfrastructure), P),
    ("FTP", Some(U::FilesAndDatabases), P),
    ("NBSS", Some(U::FilesAndDatabases), I),
    ("PostgreSQL", Some(U::FilesAndDatabases), P),
    ("SMTP", Some(U::MessagingAndPublishing), P),
    ("NNTP", Some(U::MessagingAndPublishing), P),
    ("MQTT", Some(U::MessagingAndPublishing), P),
    ("ModbusTCP", Some(U::IndustrialAndOperational), P),
    ("UMAS", Some(U::IndustrialAndOperational), P),
    ("S7Comm", Some(U::IndustrialAndOperational), P),
    ("COTP", Some(U::IndustrialAndOperational), P),
    ("EtherNet/IP", Some(U::IndustrialAndOperational), P),
    ("AMS", Some(U::IndustrialAndOperational), P),
    ("OPC UA", Some(U::IndustrialAndOperational), I),
    ("ASTERIX", Some(U::IndustrialAndOperational), P),
    ("OpenVPN", Some(U::TunnelsAndVpn), C),
    // packet_parser 11.6.0
    ("BACnet/IP", Some(U::IndustrialAndOperational), P),
    ("DNP3", Some(U::IndustrialAndOperational), P),
    ("IEC 104", Some(U::IndustrialAndOperational), P),
    ("RTPS", Some(U::IndustrialAndOperational), P),
    ("DIS", Some(U::IndustrialAndOperational), P),
    ("STANAG 5066", Some(U::IndustrialAndOperational), P),
    ("RADIUS", Some(U::AdministrationAndMonitoring), P),
    ("RDP", Some(U::AdministrationAndMonitoring), I),
    ("IKE", Some(U::TunnelsAndVpn), I),
    ("NAT-keepalive", Some(U::TunnelsAndVpn), P),
    ("WireGuard", Some(U::TunnelsAndVpn), C),
    // packet_parser 11.8.0
    ("Valmet DNA", Some(U::IndustrialAndOperational), P),
    ("GIOP", None, P),
    ("Bitcoin", None, P),
    // Tunnels (`parse/tunnel/mod.rs`) : le contenu est un autre niveau.
    ("CAPWAP", Some(U::TunnelsAndVpn), I),
    ("VXLAN", Some(U::TunnelsAndVpn), I),
    ("Geneve", Some(U::TunnelsAndVpn), I),
    ("GTP-U", Some(U::TunnelsAndVpn), I),
    ("GRE", Some(U::TunnelsAndVpn), I),
    ("IP-in-IP", Some(U::TunnelsAndVpn), I),
    ("L2TP", Some(U::TunnelsAndVpn), I),
    ("ESP", Some(U::TunnelsAndVpn), C),
    ("AH", Some(U::TunnelsAndVpn), P),
    // Transport et réseau : le contenu n'est pas reconnu.
    ("TCP", None, I),
    ("UDP", None, I),
    ("SCTP", None, I),
    ("IPv4", None, I),
    ("IPv6", None, I),
    ("ICMP", Some(U::NetworkInfrastructure), P),
    ("ICMPv6", Some(U::NetworkInfrastructure), P),
    ("IGMP", Some(U::NetworkInfrastructure), P),
    ("PIM", Some(U::NetworkInfrastructure), P),
    ("VRRP", Some(U::NetworkInfrastructure), P),
    ("OSPFIGP", Some(U::NetworkInfrastructure), P),
    ("EIGRP", Some(U::NetworkInfrastructure), P),
    // Liaison (noms d'EtherType)
    ("ARP", Some(U::NetworkInfrastructure), P),
    ("Rarp", Some(U::NetworkInfrastructure), P),
    ("LLDP", Some(U::NamesAndDiscovery), P),
    ("GOOSE", Some(U::IndustrialAndOperational), P),
    ("Profinet", Some(U::IndustrialAndOperational), P),
    ("MRP", Some(U::NetworkInfrastructure), P),
    ("cfm", Some(U::NetworkInfrastructure), P),
    ("Pppoe Discovery Stage", Some(U::NetworkInfrastructure), P),
];

/// Table indexée par jeton normalisé (une entrée ultérieure remplacerait
/// une antérieure, comme la `Map` d'origine ; un test interdit les doublons).
static BY_TOKEN: LazyLock<HashMap<String, ProtocolTraits>> = LazyLock::new(|| {
    TABLE
        .iter()
        .map(|&(label, usage, confidentiality)| {
            (
                normalize_token(label),
                ProtocolTraits {
                    usage,
                    confidentiality,
                },
            )
        })
        .collect()
});

/// Libellés couverts par la table, dans l'ordre de la table.
pub fn classified_labels() -> impl Iterator<Item = &'static str> {
    TABLE.iter().map(|&(label, _, _)| label)
}

/// Usage et chiffrement d'un libellé de protocole (casse et séparateurs
/// ignorés). Un libellé absent de la table : non classé, indéterminé.
pub fn protocol_traits(label: &str) -> ProtocolTraits {
    BY_TOKEN
        .get(&normalize_token(label))
        .copied()
        .unwrap_or(UNCLASSIFIED)
}

/// Vrai si le libellé figure explicitement dans la table.
pub fn is_classified(label: &str) -> bool {
    BY_TOKEN.contains_key(&normalize_token(label))
}

/// Usage affiché d'un libellé.
pub fn usage_of(label: &str) -> Usage {
    protocol_traits(label)
        .usage
        .map_or(Usage::Unclassified, Usage::Category)
}

#[cfg(test)]
mod tests {
    // La table, et les parts de la synthèse qui en dérivent. Règle
    // centrale : usage et
    // chiffrement viennent du seul protocole reconnu, jamais du port.

    use super::*;
    use crate::investigation::graph_view::GraphView;
    use crate::investigation::tables::{
        GRAPH_NODE_CATEGORIES, GraphNodeCategory, Totals, build_summary, build_trait_shares,
        node_category,
    };
    use crate::investigation::test_support::{
        PARSER_APPLICATION_LABELS, PARSER_TUNNEL_LABELS, graph, ports, s,
    };
    use lunar_flows_core::graph::{Edge, GraphData, Node};

    fn color(kind: GraphNodeCategory) -> String {
        GRAPH_NODE_CATEGORIES
            .iter()
            .find(|(category, _)| *category == kind)
            .map(|(_, color)| s(color))
            .unwrap_or_default()
    }

    fn node(id: &str, kind: GraphNodeCategory) -> Node {
        Node {
            id: s(id),
            name: s(id),
            color: color(kind),
            mac: String::new(),
            macs: Vec::new(),
            ip: s(id),
            vlan_id: None,
            duplicate_ip: false,
            gateway_mac: false,
            label: None,
        }
    }

    fn edge(
        id: &str,
        source: &str,
        target: &str,
        label: &str,
        bytes: u64,
        port: Option<u16>,
    ) -> Edge {
        Edge {
            id: s(id),
            source: s(source),
            target: s(target),
            label: s(label),
            source_port: None,
            destination_port: port,
            ports: ports(port.as_slice()),
            has_dynamic_ports: false,
            bidir: false,
            count: 1,
            total_bytes: bytes,
            encap_ids: Vec::new(),
        }
    }

    #[test]
    fn table_tout_protocole_emis_par_le_parser_y_figure_explicitement() {
        let missing: Vec<_> = PARSER_APPLICATION_LABELS
            .iter()
            .chain(PARSER_TUNNEL_LABELS.iter())
            .filter(|label| !is_classified(label))
            .collect();
        assert!(missing.is_empty(), "{missing:?}");
    }

    #[test]
    fn table_aucun_doublon_categories_connues() {
        let tokens: Vec<String> = classified_labels()
            .map(|label| {
                label
                    .to_lowercase()
                    .chars()
                    .filter(char::is_ascii_alphanumeric)
                    .collect()
            })
            .collect();
        let unique: std::collections::HashSet<_> = tokens.iter().collect();
        assert_eq!(unique.len(), tokens.len());
        for label in classified_labels() {
            let usage = protocol_traits(label).usage;
            assert!(
                usage.is_none_or(|usage| USAGE_CATEGORIES.contains(&usage)),
                "{label}"
            );
        }
    }

    #[test]
    fn table_le_chiffrement_vient_du_protocole_reconnu_pas_du_port() {
        assert_eq!(
            protocol_traits("TLS").confidentiality,
            Confidentiality::Encrypted
        );
        assert_eq!(
            protocol_traits("SSH").confidentiality,
            Confidentiality::Encrypted
        );
        assert_eq!(
            protocol_traits("HTTP").confidentiality,
            Confidentiality::Cleartext
        );
        assert_eq!(
            protocol_traits("ModbusTCP").confidentiality,
            Confidentiality::Cleartext
        );
        // Contenu non reconnu : indéterminé, quel que soit le port.
        assert_eq!(
            protocol_traits("TCP").confidentiality,
            Confidentiality::Undetermined
        );
        assert_eq!(
            protocol_traits("UDP").confidentiality,
            Confidentiality::Undetermined
        );
        // TLS est chiffré, mais ne dit rien de l'usage.
        assert_eq!(usage_of("TLS"), Usage::Unclassified);
        assert_eq!(usage_of("HTTP"), Usage::Category(UsageCategory::Web));
        // Casse et séparateurs ignorés, inconnu = non classé et indéterminé.
        assert_eq!(
            usage_of("modbus/tcp"),
            Usage::Category(UsageCategory::IndustrialAndOperational)
        );
        assert_eq!(
            protocol_traits("Unknown (0x88b5)"),
            ProtocolTraits {
                usage: None,
                confidentiality: Confidentiality::Undetermined
            }
        );
        assert_eq!(usage_of("—"), Usage::Unclassified);
    }

    #[test]
    fn synthese_un_flux_tcp_sur_le_port_443_reste_indetermine_et_non_classe() {
        let g = graph(
            vec![
                ("a", node("a", GraphNodeCategory::Private)),
                ("b", node("b", GraphNodeCategory::Public)),
            ],
            vec![("e", edge("e", "a", "b", "TCP", 1000, Some(443)))],
        );
        let shares = build_trait_shares(&GraphView::new(&g));
        let parts: Vec<_> = shares
            .confidentiality
            .iter()
            .map(|part| (part.kind.label(), part.totals.bytes))
            .collect();
        assert_eq!(
            parts,
            vec![("Chiffré", 0), ("En clair", 0), ("Indéterminé", 1000)]
        );
        assert_eq!(
            shares
                .usages
                .iter()
                .map(|part| part.usage)
                .collect::<Vec<_>>(),
            vec![Usage::Unclassified]
        );
    }

    #[test]
    fn synthese_portee_interne_internet_d_apres_la_categorie_des_noeuds() {
        let g = graph(
            vec![
                ("lan1", node("lan1", GraphNodeCategory::Private)),
                ("lan2", node("lan2", GraphNodeCategory::Private)),
                ("web", node("web", GraphNodeCategory::Public)),
                ("mcast", node("mcast", GraphNodeCategory::Multicast)),
            ],
            vec![
                ("e1", edge("e1", "lan1", "web", "TLS", 7000, Some(443))),
                ("e2", edge("e2", "web", "lan2", "HTTP", 2000, Some(80))),
                (
                    "e3",
                    edge("e3", "lan1", "lan2", "ModbusTCP", 500, Some(502)),
                ),
                ("e4", edge("e4", "lan1", "mcast", "SSDP", 100, None)),
            ],
        );
        assert_eq!(
            node_category(g.nodes.get("web")),
            Some(GraphNodeCategory::Public)
        );
        assert_eq!(node_category(None), None);
        let summary = build_summary(&GraphView::new(&g), 5);
        assert_eq!(
            summary.scope.internet,
            Totals {
                flows: 2,
                packets: 2,
                bytes: 9000
            }
        );
        assert_eq!(
            summary.scope.internal,
            Totals {
                flows: 2,
                packets: 2,
                bytes: 600
            }
        );
        assert_eq!(
            summary
                .confidentiality
                .iter()
                .map(|part| (part.kind.label(), part.totals.bytes))
                .collect::<Vec<_>>(),
            vec![("Chiffré", 7000), ("En clair", 2600), ("Indéterminé", 0)]
        );
        assert_eq!(
            summary
                .usages
                .iter()
                .map(|part| (part.usage.label(), part.totals.bytes))
                .collect::<Vec<_>>(),
            vec![
                ("Non classé", 7000),
                ("Web", 2000),
                ("Industriel et opérationnel", 500),
                ("Noms et découverte", 100)
            ]
        );
        // Chaque répartition couvre tout le volume.
        let total = 9600;
        assert_eq!(
            summary
                .confidentiality
                .iter()
                .map(|part| part.totals.bytes)
                .sum::<u64>(),
            total
        );
        assert_eq!(
            summary
                .usages
                .iter()
                .map(|part| part.totals.bytes)
                .sum::<u64>(),
            total
        );
    }

    #[test]
    fn synthese_releve_vide_trois_statuts_a_zero_aucun_usage() {
        let empty = GraphData::new();
        let shares = build_trait_shares(&GraphView::new(&empty));
        assert_eq!(
            shares
                .confidentiality
                .iter()
                .map(|part| part.totals.bytes)
                .collect::<Vec<_>>(),
            vec![0, 0, 0]
        );
        assert!(shares.usages.is_empty());
        assert_eq!(shares.scope.internet.flows + shares.scope.internal.flows, 0);
    }
}
