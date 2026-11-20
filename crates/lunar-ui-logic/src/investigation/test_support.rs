//! Fabriques de nœuds et d'arêtes des tests.

use std::collections::BTreeSet;

use lunar_flows_core::graph::{Edge, GraphData, Node};

/// Libellés de protocole applicatif que le parser peut émettre (règles de
/// `parse/dispatch/rules.rs`, packet_parser 11.8.2) : un libellé ajouté ici
/// fait échouer les tests du glossaire et de la table des usages tant qu'il
/// n'est ni défini ni classé.
pub const PARSER_APPLICATION_LABELS: [&str; 45] = [
    "AMS",
    "ASTERIX",
    "BACnet/IP",
    "Bitcoin",
    "COTP",
    "DHCP",
    "DHCPv6",
    "DIS",
    "DNP3",
    "DNS",
    "ESP",
    "EtherNet/IP",
    "FTP",
    "GIOP",
    "HTTP",
    "IEC 104",
    "IKE",
    "LLMNR",
    "mDNS",
    "ModbusTCP",
    "MQTT",
    "NAT-keepalive",
    "NBNS",
    "NBSS",
    "NNTP",
    "NTP",
    "OPC UA",
    "OpenVPN",
    "PostgreSQL",
    "PTP",
    "QUIC",
    "RADIUS",
    "RDP",
    "RTPS",
    "S7Comm",
    "SMTP",
    "SNMP",
    "SRVLOC",
    "SSDP",
    "SSH",
    "STANAG 5066",
    "TLS",
    "UMAS",
    "Valmet DNA",
    "WireGuard",
];

/// Libellés de tunnel (`parse/tunnel/mod.rs`), qui deviennent aussi le
/// protocole affiché d'un flux encapsulant.
pub const PARSER_TUNNEL_LABELS: [&str; 6] =
    ["CAPWAP", "VXLAN", "Geneve", "GTP-U", "GRE", "IP-in-IP"];

/// `String` depuis un littéral.
pub fn s(value: &str) -> String {
    value.to_string()
}

/// Nœud par défaut des tests (`id: "n"`, couleur `#000`…).
pub fn node() -> Node {
    Node {
        id: s("n"),
        name: String::new(),
        color: s("#000"),
        mac: String::new(),
        macs: Vec::new(),
        ip: String::new(),
        vlan_id: None,
        duplicate_ip: false,
        gateway_mac: false,
        label: None,
    }
}

/// Arête par défaut des tests (`a` → `b`, TCP, sans trafic).
pub fn edge() -> Edge {
    Edge {
        id: s("e"),
        source: s("a"),
        target: s("b"),
        label: s("TCP"),
        source_port: None,
        destination_port: None,
        ports: BTreeSet::new(),
        has_dynamic_ports: false,
        bidir: false,
        count: 0,
        total_bytes: 0,
        encap_ids: Vec::new(),
    }
}

/// Ports d'une arête.
pub fn ports(values: &[u16]) -> BTreeSet<u16> {
    values.iter().copied().collect()
}

/// Graphe de nœuds et d'arêtes indexés par leur clé de map.
pub fn graph(nodes: Vec<(&str, Node)>, edges: Vec<(&str, Edge)>) -> GraphData {
    let mut graph = GraphData::new();
    graph
        .nodes
        .extend(nodes.into_iter().map(|(key, node)| (s(key), node)));
    graph
        .edges
        .extend(edges.into_iter().map(|(key, edge)| (s(key), edge)));
    graph
}
