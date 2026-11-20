//! Journal des dernières trames reçues, sous le graphe : douze colonnes,
//! les 5 derniers paquets des lots `PacketBatch`.
//!
//! Les cellules sont calculées depuis le JSON du contrat
//! (`CapturedPacketOwned` sérialisé), avec « - » pour une valeur absente
//! et une discrimination par `link_kind`. La pompe d'événements regroupe les
//! lots avant de redessiner, ce qui lisse l'affichage.

use std::collections::VecDeque;

use chrono::{Local, TimeZone};
use gpui::{IntoElement, ParentElement, SharedString, Styled, div, prelude::FluentBuilder, px};
use lunar_engine::state::capture::capture_handle::messages::capture::CapturedPacketOwned;
use serde_json::Value;

use crate::theme::Theme;
use crate::widgets::ellipsis;

/// Lignes affichées (`MAX_LOG_ROWS`).
pub const MAX_LOG_ROWS: usize = 5;

/// En-têtes des colonnes.
pub const COLUMNS: [&str; 12] = [
    "MAC S",
    "MAC D",
    "Vlan",
    "Protocol",
    "IP S",
    "IP D",
    "Transport",
    "Port S",
    "Port D",
    "Application",
    "Taille",
    "Heure",
];

/// Hauteur du journal (`.trames { height: 190px }`).
pub const PACKET_LOG_HEIGHT: f32 = 190.;

/// Une ligne du journal, cellules dans l'ordre de [`COLUMNS`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PacketRow {
    /// Cellules formatées.
    pub cells: [SharedString; 12],
}

/// Valeur affichée : « - » pour une valeur absente, nulle ou vide.
fn format_value(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => "-".to_owned(),
        Some(Value::String(text)) if text.is_empty() => "-".to_owned(),
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
    }
}

/// Adresse matérielle SLL/SLL2, sérialisée en tableau d'octets.
fn format_hw_address(value: Option<&Value>) -> Option<Value> {
    let bytes = value?.as_array()?;
    if bytes.is_empty() {
        return None;
    }
    let text = bytes
        .iter()
        .map(|byte| format!("{:02x}", byte.as_u64().unwrap_or(0)))
        .collect::<Vec<_>>()
        .join(":");
    Some(Value::String(text))
}

/// Protocole réseau générique d'un lien sans EtherType.
fn format_network_protocol(protocol: Option<&Value>) -> Option<Value> {
    let protocol = protocol?;
    let label = match protocol.get("kind")?.as_str()? {
        "ipv4" => "IPv4".to_owned(),
        "ipv6" => "IPv6".to_owned(),
        "arp" => "ARP".to_owned(),
        "profinet" => "Profinet".to_owned(),
        "ptp" => "PTP".to_owned(),
        "goose" => "GOOSE".to_owned(),
        "lldp" => "LLDP".to_owned(),
        "other" => format!("0x{:04X}", protocol.get("value")?.as_u64()?),
        _ => return None,
    };
    Some(Value::String(label))
}

/// MAC source et destination.
fn link_addresses(data_link: &Value) -> (Option<Value>, Option<Value>) {
    let details = data_link.get("link_details");
    let field = |name: &str| details.and_then(|d| d.get(name)).cloned();
    match data_link.get("link_kind").and_then(Value::as_str) {
        Some("ethernet" | "ieee80211") => (field("source_mac"), field("destination_mac")),
        Some("linux_sll" | "linux_sll2") => (
            format_hw_address(details.and_then(|d| d.get("source_address"))),
            None,
        ),
        _ => (None, None),
    }
}

/// VLAN : seul Ethernet en porte un.
fn link_vlan(data_link: &Value) -> Option<Value> {
    if data_link.get("link_kind").and_then(Value::as_str) != Some("ethernet") {
        return None;
    }
    data_link
        .get("link_details")?
        .get("vlan")?
        .get("id")
        .cloned()
}

/// EtherType, protocole SNAP ou protocole réseau du lien.
fn link_protocol(data_link: &Value) -> Option<Value> {
    let details = data_link.get("link_details");
    match data_link.get("link_kind").and_then(Value::as_str)? {
        "ethernet" => details?.get("ethertype").cloned(),
        "ieee80211" => details?.get("snap_protocol").cloned(),
        "linux_sll" | "linux_sll2" | "raw_ip" => {
            format_network_protocol(data_link.get("network_protocol"))
        }
        _ => None,
    }
}

/// `TCP`/`UDP` en capitales, le reste tel quel.
fn transport(value: Option<&Value>) -> Option<Value> {
    let text = value?.as_str()?;
    if text.is_empty() {
        return None;
    }
    let normalized = match text.trim().to_lowercase().as_str() {
        "tcp" => "TCP".to_owned(),
        "udp" => "UDP".to_owned(),
        _ => text.to_owned(),
    };
    Some(Value::String(normalized))
}

/// Heure locale `HH:MM:SS.mmm` de l'horodatage de capture.
pub fn format_timestamp(sec: Option<i64>, usec: Option<i64>) -> String {
    let (Some(sec), Some(usec)) = (sec, usec) else {
        return "-".to_owned();
    };
    let millis = sec
        .saturating_mul(1000)
        .saturating_add(usec.div_euclid(1000));
    let Some(date) = Local.timestamp_millis_opt(millis).single() else {
        return "-".to_owned();
    };
    let ms = usec.rem_euclid(1_000_000) / 1000;
    format!("{}.{ms:03}", date.format("%H:%M:%S"))
}

/// Ligne depuis le JSON d'un paquet.
pub fn row_from_json(packet: &Value) -> PacketRow {
    let null = Value::Null;
    let flow = packet.get("flow").unwrap_or(&null);
    let data_link = flow.get("data_link").unwrap_or(&null);
    let (source_mac, destination_mac) = link_addresses(data_link);
    let cell = |value: Option<&Value>| SharedString::from(format_value(value));
    PacketRow {
        cells: [
            cell(source_mac.as_ref()),
            cell(destination_mac.as_ref()),
            cell(link_vlan(data_link).as_ref()),
            cell(link_protocol(data_link).as_ref()),
            cell(flow.get("source_ip")),
            cell(flow.get("destination_ip")),
            cell(transport(flow.get("protocol_transport")).as_ref()),
            cell(flow.get("source_port")),
            cell(flow.get("destination_port")),
            cell(flow.get("application_protocol")),
            cell(packet.get("len")),
            format_timestamp(
                packet.get("tsSec").and_then(Value::as_i64),
                packet.get("tsUsec").and_then(Value::as_i64),
            )
            .into(),
        ],
    }
}

/// Ligne d'un paquet du moteur.
pub fn row_from_packet(packet: &CapturedPacketOwned) -> PacketRow {
    match serde_json::to_value(packet) {
        Ok(value) => row_from_json(&value),
        Err(_) => row_from_json(&Value::Null),
    }
}

/// Les 5 dernières lignes.
#[derive(Debug, Default, Clone)]
pub struct PacketLog {
    /// Lignes, la plus ancienne en tête.
    rows: VecDeque<PacketRow>,
}

impl PacketLog {
    /// Journal vide.
    pub fn new() -> Self {
        Self::default()
    }

    /// Ajoute un lot : seuls ses 5 derniers paquets sont formatés.
    pub fn push_batch(&mut self, packets: &[CapturedPacketOwned]) {
        let start = packets.len().saturating_sub(MAX_LOG_ROWS);
        for packet in &packets[start..] {
            self.rows.push_back(row_from_packet(packet));
        }
        while self.rows.len() > MAX_LOG_ROWS {
            self.rows.pop_front();
        }
    }

    /// Vide le journal (événement `reset`).
    pub fn clear(&mut self) {
        self.rows.clear();
    }

    /// Lignes, la plus ancienne en tête.
    pub fn rows(&self) -> impl Iterator<Item = &PacketRow> {
        self.rows.iter()
    }

    /// Nombre de lignes.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Journal vide.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// Rendu : en-tête et lignes, colonnes de largeur égale, texte centré et
/// tronqué (`table-layout: fixed`).
pub fn render_packet_log(log: &PacketLog, theme: &Theme) -> impl IntoElement + use<> {
    let color = theme.text_highlight;
    let row = |cells: Vec<SharedString>, bold: bool| {
        div()
            .flex()
            .w_full()
            .children(cells.into_iter().map(move |text| {
                ellipsis(
                    div()
                        .flex_1()
                        .min_w_0()
                        .px(px(8.))
                        .py(px(6.))
                        .text_center()
                        .when(bold, |d| d.font_weight(gpui::FontWeight::BOLD))
                        .child(text),
                )
            }))
    };
    let header: Vec<SharedString> = COLUMNS.iter().map(|c| SharedString::from(*c)).collect();
    div()
        .flex()
        .flex_col()
        .flex_none()
        .h(px(PACKET_LOG_HEIGHT))
        .w_full()
        .overflow_hidden()
        .bg(gpui::black())
        .font_family(theme.font_mono.clone())
        .text_size(theme.text_md)
        .text_color(color)
        .child(row(header, true))
        .children(log.rows().map(|r| row(r.cells.to_vec(), false)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cells(row: &PacketRow) -> Vec<&str> {
        row.cells.iter().map(|c| c.as_ref()).collect()
    }

    #[test]
    fn trame_ethernet_complete() {
        let packet = json!({
            "tsSec": 1, "tsUsec": 2_345, "caplen": 74, "len": 74,
            "flow": {
                "data_link": {
                    "link_type": 1, "network_protocol": {"kind": "ipv4"},
                    "link_kind": "ethernet",
                    "link_details": {
                        "destination_mac": "aa:bb:cc:dd:ee:ff",
                        "source_mac": "11:22:33:44:55:66",
                        "ethertype": "IPv4",
                        "vlan": {"id": 2474, "pcp": 0, "dei": false}
                    }
                },
                "source_ip": "192.168.1.10", "destination_ip": "8.8.8.8",
                "protocol_internet": "TCP",
                "source_port": 12345, "destination_port": 80,
                "protocol_transport": "tcp", "application_protocol": "HTTP"
            }
        });
        let row = row_from_json(&packet);
        let got = cells(&row);
        assert_eq!(
            &got[..11],
            [
                "11:22:33:44:55:66",
                "aa:bb:cc:dd:ee:ff",
                "2474",
                "IPv4",
                "192.168.1.10",
                "8.8.8.8",
                "TCP",
                "12345",
                "80",
                "HTTP",
                "74",
            ]
        );
        assert!(got[11].ends_with(".002"), "millisecondes : {}", got[11]);
    }

    #[test]
    fn lien_sll_adresse_cuite_et_protocole_generique() {
        let packet = json!({
            "tsSec": 0, "tsUsec": 0, "len": 60,
            "flow": {
                "data_link": {
                    "link_type": 113, "network_protocol": {"kind": "ipv6"},
                    "link_kind": "linux_sll",
                    "link_details": {"source_address": [224, 213, 94, 40, 155, 212]}
                }
            }
        });
        let row = row_from_json(&packet);
        let got = cells(&row);
        assert_eq!(got[0], "e0:d5:5e:28:9b:d4");
        assert_eq!(got[1], "-");
        assert_eq!(got[2], "-", "pas de VLAN hors Ethernet");
        assert_eq!(got[3], "IPv6");
        assert_eq!(got[4], "-", "flux sans couche IP");
        assert_eq!(got[6], "-", "flux sans transport");
    }

    #[test]
    fn raw_ip_protocole_inconnu_en_hexadecimal() {
        let packet = json!({
            "len": 10,
            "flow": {
                "data_link": {
                    "link_type": 101, "network_protocol": {"kind": "other", "value": 0x88b5},
                    "link_kind": "raw_ip", "link_details": {"ip_version": 4}
                },
                "protocol_transport": "SCTP", "application_protocol": ""
            }
        });
        let row = row_from_json(&packet);
        let got = cells(&row);
        assert_eq!(&got[..4], ["-", "-", "-", "0x88B5"]);
        assert_eq!(got[6], "SCTP", "transport inconnu laissé tel quel");
        assert_eq!(got[9], "-", "application vide");
        assert_eq!(got[11], "-", "horodatage absent");
    }

    #[test]
    fn horodatage_local_avec_millisecondes() {
        let text = format_timestamp(Some(1_700_000_000), Some(123_456));
        let expected = Local
            .timestamp_millis_opt(1_700_000_000_123)
            .single()
            .map(|d| format!("{}.123", d.format("%H:%M:%S")));
        assert_eq!(Some(text), expected);
        assert_eq!(format_timestamp(None, Some(1)), "-");
    }

    #[test]
    fn journal_borne_aux_cinq_derniers() {
        use lunar_flows_core::packet::CapturedPacket;
        use packet_parser::PacketFlow;
        const FRAME: [u8; 42] = [
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0x08, 0x00,
            0x45, 0x00, 0x00, 0x1c, 0x00, 0x01, 0x00, 0x00, 0x40, 0x11, 0x00, 0x0a, 0xc0, 0xa8,
            0x00, 0x01, 0xc0, 0xa8, 0x00, 0x02, 0x00, 0x35, 0x00, 0x35, 0x00, 0x08, 0x00, 0x00,
        ];
        let flow = PacketFlow::try_from(FRAME.as_slice()).expect("trame UDP");
        let packet = CapturedPacket {
            ts_sec: 1,
            ts_usec: 2,
            caplen: 42,
            len: 42,
            flow,
        }
        .to_owned_packet();

        let mut log = PacketLog::new();
        log.push_batch(&vec![packet.clone(); 3]);
        assert_eq!(log.len(), 3);
        log.push_batch(&vec![packet.clone(); 12]);
        assert_eq!(log.len(), MAX_LOG_ROWS);
        let Some(first) = log.rows().next() else {
            panic!("journal vide");
        };
        let got = cells(first);
        assert_eq!(
            &got[..11],
            [
                "66:77:88:99:aa:bb",
                "00:11:22:33:44:55",
                "-",
                "IPv4",
                "192.168.0.1",
                "192.168.0.2",
                "UDP",
                "53",
                "53",
                got[9],
                "42",
            ]
        );
        log.clear();
        assert!(log.is_empty());
    }
}
