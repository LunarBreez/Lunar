//! Test différentiel bout-en-bout PCAP -> matrice LFMS.
//!
//! La fixture est un échantillon déterministe de `The-Ultimate-PCAP` dont
//! l'inventaire indépendant TShark est committé à côté du PCAP. Le CSV de
//! référence fige ensuite chaque ligne produite (identité, compteurs, octets
//! et date), pas seulement les totaux globaux.

#![cfg(feature = "pcap")]
#![allow(clippy::expect_used, clippy::panic)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use lunar_flows_core::LunarCoreError;
use lunar_flows_core::csv::merge_matrix_files;
use lunar_flows_core::graph::GraphData;
use lunar_flows_core::link::ethertype_from_name;
use lunar_flows_core::matrix::{FlowMatrix, FlowMatrixRow, parse_last_seen};
use lunar_flows_core::pcap::append_pcap_file;
use packet_parser::LinkType;
use serde::Deserialize;
use sha2::{Digest, Sha256};

const EXPECTED_PACKETS: usize = 328;
const EXPECTED_BYTES: u64 = 96_622;
const EXPECTED_ROWS: usize = 229;
const EXPECTED_OUTER_ROWS: usize = 222;
const EXPECTED_INNER_FLOWS: usize = 7;
const EXPECTED_COMMON_PACKETS: u64 = 313;
const EXPECTED_COMMON_BYTES: u64 = 94_091;
const EXPECTED_COMMON_FLOWS: usize = 204;

fn ultimate_fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/pcaps/import/ultimate_ethernet_sample")
}

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pcaps/import/pcap_tshark_corpus")
}

fn read_expected_rows(path: &Path) -> Vec<FlowMatrixRow> {
    let mut reader = csv::ReaderBuilder::new()
        .comment(Some(b'#'))
        .from_path(path)
        .expect("ouverture de la matrice oracle");
    reader
        .deserialize()
        .collect::<Result<Vec<_>, _>>()
        .expect("désérialisation de la matrice oracle")
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct CommonFlowKey {
    mac_source: String,
    mac_destination: String,
    vlan_id: Option<u16>,
    ether_type: u16,
    ip_source: String,
    ip_destination: String,
    ip_protocol: Option<u8>,
    port_source: Option<u16>,
    port_destination: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommonFlowStats {
    count: u64,
    total_bytes: u64,
    last_seen_us: u64,
}

#[derive(Debug, Deserialize)]
struct RawCommonFlowRow {
    mac_source: String,
    mac_destination: String,
    vlan_id: String,
    ether_type: u16,
    ip_source: String,
    ip_destination: String,
    ip_protocol: String,
    port_source: String,
    port_destination: String,
    count: u64,
    total_bytes: u64,
    last_seen_us: u64,
}

fn transport_protocol_number(name: Option<&str>) -> Option<u8> {
    match name {
        Some("ICMP") => Some(1),
        Some("IGMP") => Some(2),
        Some("IPv4") => Some(4),
        Some("TCP") | Some("Tcp") => Some(6),
        Some("UDP") => Some(17),
        Some("IPv6") => Some(41),
        Some("GRE") => Some(47),
        Some("ESP") => Some(50),
        Some("ICMPv6") => Some(58),
        Some("EIGRP") => Some(88),
        Some("OSPFIGP") => Some(89),
        Some("L2TP") => Some(115),
        Some(other) => panic!("protocole de transport sans numéro oracle : {other}"),
        None => None,
    }
}

/// Niveau interne d'un tunnel IP-in-IP, 6in4 ou GRE : depuis packet_parser
/// 10.4.0, chaque niveau pelé devient une ligne à part entière, jointe à sa
/// ligne externe par `encap_id`. Sur une capture Ethernet, ces niveaux sont
/// de l'IP nu — sans trame, donc sans MAC. Les tunnels à trame interne
/// (CAPWAP data) sont couverts séparément par le corpus multi-DLT.
fn is_ip_tunnel_inner(row: &FlowMatrixRow) -> bool {
    !row.encap_id.is_empty() && row.mac_source.is_empty()
}

fn common_projection(rows: &[FlowMatrixRow]) -> BTreeMap<CommonFlowKey, CommonFlowStats> {
    let mut projected: BTreeMap<CommonFlowKey, CommonFlowStats> = BTreeMap::new();
    for row in rows {
        let ether_type = ethertype_from_name(&row.protocol_data_link)
            .unwrap_or_else(|| panic!("EtherType LFMS inconnu : {}", row.protocol_data_link))
            .0;
        // Même domaine que l'oracle : les longueurs IEEE 802.3 et les VLAN
        // empilés ne sont pas représentables sans perte dans LFMS v1.
        if ether_type < 0x0600 {
            continue;
        }
        let key = CommonFlowKey {
            mac_source: row.mac_source.clone(),
            mac_destination: row.mac_destination.clone(),
            vlan_id: row.vlan_id,
            ether_type,
            ip_source: row.ip_source.clone(),
            ip_destination: row.ip_destination.clone(),
            ip_protocol: transport_protocol_number(row.protocol_transport.as_deref()),
            port_source: row.port_source,
            port_destination: row.port_destination,
        };
        let last_seen_us = parse_last_seen(&row.last_seen)
            .expect("timestamp LFMS valide")
            .duration_since(UNIX_EPOCH)
            .expect("timestamp postérieur à epoch")
            .as_micros() as u64;
        let stats = projected.entry(key).or_insert(CommonFlowStats {
            count: 0,
            total_bytes: 0,
            last_seen_us: 0,
        });
        stats.count += row.count;
        stats.total_bytes += row.total_bytes;
        stats.last_seen_us = stats.last_seen_us.max(last_seen_us);
    }
    projected
}

fn read_tshark_common_flows(path: &Path) -> BTreeMap<CommonFlowKey, CommonFlowStats> {
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(b'\t')
        .comment(Some(b'#'))
        .from_path(path)
        .expect("ouverture de l'oracle TShark");
    reader
        .deserialize::<RawCommonFlowRow>()
        .map(|row| {
            let row = row.expect("ligne TShark commune valide");
            (
                CommonFlowKey {
                    mac_source: row.mac_source,
                    mac_destination: row.mac_destination,
                    vlan_id: (!row.vlan_id.is_empty())
                        .then(|| row.vlan_id.parse().expect("VLAN TShark valide")),
                    ether_type: row.ether_type,
                    ip_source: row.ip_source,
                    ip_destination: row.ip_destination,
                    ip_protocol: (!row.ip_protocol.is_empty())
                        .then(|| row.ip_protocol.parse().expect("protocole IP TShark valide")),
                    port_source: (!row.port_source.is_empty())
                        .then(|| row.port_source.parse().expect("port source TShark valide")),
                    port_destination: (!row.port_destination.is_empty()).then(|| {
                        row.port_destination
                            .parse()
                            .expect("port destination TShark valide")
                    }),
                },
                CommonFlowStats {
                    count: row.count,
                    total_bytes: row.total_bytes,
                    last_seen_us: row.last_seen_us,
                },
            )
        })
        .collect()
}

#[derive(Debug)]
struct OracleMetadata {
    capture_sha256: String,
    included_packets: u64,
    included_bytes: u64,
}

fn read_oracle_metadata(path: &Path) -> OracleMetadata {
    let content = std::fs::read_to_string(path).expect("lecture des métadonnées oracle");
    let value = |name: &str| {
        content
            .lines()
            .find_map(|line| line.strip_prefix(&format!("# {name}=")))
            .unwrap_or_else(|| panic!("métadonnée {name} absente de {}", path.display()))
    };
    OracleMetadata {
        capture_sha256: value("capture_sha256").to_string(),
        included_packets: value("included_packets")
            .parse()
            .expect("compte de paquets oracle"),
        included_bytes: value("included_bytes")
            .parse()
            .expect("compte d'octets oracle"),
    }
}

fn assert_oracle_hash(capture: &Path, oracle: &Path) {
    let expected = read_oracle_metadata(oracle).capture_sha256;
    let actual = format!(
        "{:x}",
        Sha256::digest(std::fs::read(capture).expect("lecture de la capture"))
    );
    assert_eq!(
        actual,
        expected,
        "oracle TShark périmé pour {} : régénérer {}",
        capture.display(),
        oracle.display()
    );
}

fn assert_common_flows_equal(
    label: &str,
    actual: &BTreeMap<CommonFlowKey, CommonFlowStats>,
    expected: &BTreeMap<CommonFlowKey, CommonFlowStats>,
) {
    if actual == expected {
        return;
    }
    let keys: BTreeSet<_> = actual.keys().chain(expected.keys()).collect();
    let differences = keys
        .into_iter()
        .filter_map(|key| {
            let actual = actual.get(key);
            let expected = expected.get(key);
            (actual != expected)
                .then(|| format!("{key:?}\n  LUNAR={actual:?}\n  TShark={expected:?}"))
        })
        .take(20)
        .collect::<Vec<_>>()
        .join("\n");
    panic!("écart LUNAR/TShark pour {label} :\n{differences}");
}

fn assert_deterministic_round_trip(matrix: &FlowMatrix, label: &str) {
    let base = std::env::temp_dir().join(format!(
        "lunar_pcap_accuracy_{label}_{}",
        std::process::id()
    ));
    let first = base.with_extension("first.csv");
    let second = base.with_extension("second.csv");
    matrix
        .export_to_csv(first.to_string_lossy().into_owned())
        .expect("premier export");
    matrix
        .export_to_csv(second.to_string_lossy().into_owned())
        .expect("second export");
    assert_eq!(
        std::fs::read(&first).expect("lecture premier export"),
        std::fs::read(&second).expect("lecture second export"),
        "export non déterministe pour {label}"
    );

    let reimported = merge_matrix_files(std::slice::from_ref(&first)).expect("réimport LFMS");
    let mut actual = reimported.to_flat_vec();
    let mut expected = matrix.to_flat_vec();
    for row in actual.iter_mut().chain(expected.iter_mut()) {
        row.origin.clear();
    }
    assert_eq!(
        actual, expected,
        "aller-retour LFMS avec perte pour {label}"
    );
    let _ = std::fs::remove_file(first);
    let _ = std::fs::remove_file(second);
}

#[test]
fn ultimate_ethernet_sample_matches_lfms_snapshot_flow_by_flow() {
    let fixtures = ultimate_fixture_dir();
    let capture = fixtures.join("ultimate_ethernet_sample.pcap");
    let expected_csv = fixtures.join("ultimate_ethernet_sample.csv");

    let mut matrix = FlowMatrix::new();
    let report = append_pcap_file(&mut matrix, &capture).expect("conversion de la fixture");

    // Ces deux agrégats viennent directement de l'inventaire TShark committé
    // dans ultimate_ethernet_sample.tshark.md.
    assert_eq!(report.read(), EXPECTED_PACKETS);
    assert_eq!(report.decoded, EXPECTED_PACKETS);
    assert_eq!(report.rejected(), 0);
    assert_eq!(matrix.link_type, Some(LinkType::ETHERNET));

    let actual = matrix.to_flat_vec();
    let expected = read_expected_rows(&expected_csv);
    assert_eq!(actual.len(), EXPECTED_ROWS);
    assert_eq!(actual, expected, "écart de matrice flux par flux");

    // Les niveaux internes de tunnel ne comptent pas deux fois le même
    // volume : seules les lignes externes (une par trame) totalisent les
    // trames et les octets de TShark.
    let (inner, outer): (Vec<_>, Vec<_>) = actual.iter().partition(|row| is_ip_tunnel_inner(row));
    assert_eq!(outer.len(), EXPECTED_OUTER_ROWS);
    assert_eq!(
        outer.iter().map(|row| row.count).sum::<u64>(),
        EXPECTED_PACKETS as u64,
        "la somme des compteurs externes doit rester égale aux trames TShark"
    );
    assert_eq!(
        outer.iter().map(|row| row.total_bytes).sum::<u64>(),
        EXPECTED_BYTES,
        "la somme des octets externes doit rester égale à frame.len dans TShark"
    );
    let outer_encap_ids = outer
        .iter()
        .map(|row| row.encap_id.as_str())
        .filter(|id| !id.is_empty())
        .collect::<BTreeSet<_>>();
    for row in &inner {
        assert!(
            outer_encap_ids.contains(row.encap_id.as_str()),
            "ligne interne sans ligne externe joignable par encap_id : {row:?}"
        );
    }
}

/// Les indicateurs de la vue d'investigation sont des sommes sur
/// l'instantané du graphe (`get_graph_snapshot`). Ils doivent valoir exactement
/// ce que donne la matrice exportée pour le même relevé : le graphe agrège les
/// lignes par relation, il ne doit ni en perdre ni en compter deux fois.
#[test]
fn ultimate_ethernet_sample_graph_totals_equal_exported_matrix_totals() {
    let capture = ultimate_fixture_dir().join("ultimate_ethernet_sample.pcap");
    let mut matrix = FlowMatrix::new();
    append_pcap_file(&mut matrix, &capture).expect("conversion de la fixture");

    let rows = matrix.to_flat_vec();
    let matrix_packets: u64 = rows.iter().map(|row| row.count).sum();
    let matrix_bytes: u64 = rows.iter().map(|row| row.total_bytes).sum();

    let graph = GraphData::from_flow_matrix(&matrix).get_all_graph_data();
    let graph_packets: u64 = graph.edges.values().map(|edge| edge.count).sum();
    let graph_bytes: u64 = graph.edges.values().map(|edge| edge.total_bytes).sum();

    assert_eq!(
        graph_packets, matrix_packets,
        "paquets : graphe ≠ matrice exportée"
    );
    assert_eq!(
        graph_bytes, matrix_bytes,
        "volume : graphe ≠ matrice exportée"
    );

    // Les niveaux internes de tunnel sont des lignes à part entière de la
    // matrice : ces totaux dépassent donc le nombre de trames de la capture.
    // C'est la matrice exportée, pas TShark, qui fait référence ici.
    assert!(matrix_packets >= EXPECTED_PACKETS as u64);

    // Toute arête relie deux nœuds présents dans l'instantané.
    let node_ids: BTreeSet<&str> = graph.nodes.values().map(|node| node.id.as_str()).collect();
    for edge in graph.edges.values() {
        assert!(
            node_ids.contains(edge.source.as_str()),
            "source orpheline : {}",
            edge.id
        );
        assert!(
            node_ids.contains(edge.target.as_str()),
            "cible orpheline : {}",
            edge.id
        );
    }
}

#[test]
fn ultimate_ethernet_sample_matches_tshark_on_every_shared_field() {
    let fixtures = ultimate_fixture_dir();
    let capture = fixtures.join("ultimate_ethernet_sample.pcap");
    let oracle = fixtures.join("ultimate_ethernet_sample.flows.tsv");
    let inner_oracle = fixtures.join("ultimate_ethernet_sample.inner.flows.tsv");
    assert_oracle_hash(&capture, &oracle);
    assert_oracle_hash(&capture, &inner_oracle);

    let mut matrix = FlowMatrix::new();
    append_pcap_file(&mut matrix, &capture).expect("conversion de la fixture");

    // L'oracle `outer` projette la première occurrence des champs IP de
    // chaque trame ; l'oracle `ip_inner` projette la dernière, pour les
    // seules trames dont le transport externe est un tunnel IP.
    let (inner, outer): (Vec<_>, Vec<_>) = matrix
        .to_flat_vec()
        .into_iter()
        .partition(is_ip_tunnel_inner);
    let actual = common_projection(&outer);
    let expected = read_tshark_common_flows(&oracle);
    assert_eq!(expected.len(), EXPECTED_COMMON_FLOWS);
    assert_eq!(
        expected.values().map(|stats| stats.count).sum::<u64>(),
        EXPECTED_COMMON_PACKETS
    );
    assert_eq!(
        expected
            .values()
            .map(|stats| stats.total_bytes)
            .sum::<u64>(),
        EXPECTED_COMMON_BYTES
    );
    assert_common_flows_equal("ultimate_ethernet_sample", &actual, &expected);

    let expected_inner = read_tshark_common_flows(&inner_oracle);
    assert_eq!(expected_inner.len(), EXPECTED_INNER_FLOWS);
    assert_common_flows_equal(
        "ultimate_ethernet_sample/inner",
        &common_projection(&inner),
        &expected_inner,
    );
    // Les niveaux pelés (GRE, IP-in-IP, 6in4) n'ont pas de MAC : l'aller-retour
    // LFMS ne doit pas leur en inventer.
    assert_deterministic_round_trip(&matrix, "ultimate_ethernet_sample");
}

#[test]
fn ultimate_ethernet_sample_export_is_byte_for_byte_deterministic() {
    let fixtures = ultimate_fixture_dir();
    let capture = fixtures.join("ultimate_ethernet_sample.pcap");
    let expected_csv = fixtures.join("ultimate_ethernet_sample.csv");
    let output = std::env::temp_dir().join(format!(
        "lunar_ultimate_ethernet_sample_{}.csv",
        std::process::id()
    ));

    let mut matrix = FlowMatrix::new();
    append_pcap_file(&mut matrix, &capture).expect("conversion de la fixture");
    matrix
        .export_to_csv(output.to_string_lossy().into_owned())
        .expect("export déterministe");

    let actual = std::fs::read(&output).expect("lecture de l'export");
    let expected = std::fs::read(&expected_csv).expect("lecture du snapshot");
    let _ = std::fs::remove_file(&output);
    assert_eq!(actual, expected, "le CSV LFMS a dérivé octet par octet");
}

#[test]
fn multi_dlt_corpus_matches_tshark_flow_by_flow() {
    let fixtures = corpus_dir();
    for (name, expected_link_type) in [
        ("linux_sll", LinkType::LINUX_SLL),
        ("linux_sll2", LinkType::LINUX_SLL2),
        ("raw_ip", LinkType::RAW),
        ("vlan", LinkType::ETHERNET),
        ("industrial_ethernet", LinkType::ETHERNET),
        ("capwap_radius", LinkType::ETHERNET),
        ("capwap_management", LinkType::ETHERNET),
        // Double tag 802.1Q (QinQ) : vlan_id = tag interne, couche 3 atteinte
        // (packet_parser 10.5.0). Depuis packet_parser 11.6.0, la
        // session PPPoE est pelée comme un tag quand PPP porte IPv4 ou IPv6 :
        // l'oracle (`ppp.protocol`) et LUNAR voient les mêmes flux IP.
        ("qinq_pppoe", LinkType::ETHERNET),
        // Loopback BSD (LINKTYPE_NULL), décodé depuis packet_parser 11.2.0
        // : l'UDP forgé qui marquait la frontière, puis une vraie
        // session OPC UA (corpus nDPI).
        ("loopback_null", LinkType::NULL),
        ("opcua_loopback", LinkType::NULL),
        // ASTERIX (packet_parser 11.3.0) : flux UDP multicast,
        // étiquette vérifiée à part dans `asterix_captures_are_labelled…`.
        ("asterix_cat048_multicast", LinkType::ETHERNET),
        ("asterix_cat034_048", LinkType::ETHERNET),
        // PTP (packet_parser 11.5.0) : UDP 319/320 en IPv4 et en IPv6, et
        // couche 2 (EtherType 0x88F7), dont l'aller-retour LFMS exerce le
        // réimport ; étiquettes vérifiées dans `ptp_captures_are_labelled…`.
        ("ptp_udp_l2", LinkType::ETHERNET),
        ("ptp_ipv6", LinkType::ETHERNET),
        // packet_parser 11.6.0 : dix captures des nouveaux décodeurs, flux
        // par flux contre TShark ; étiquettes vérifiées dans
        // `packet_parser_11_6_captures_are_labelled_exactly_like_tshark`.
        // GOOSE est une couche 2 (EtherType 0x88B8) dont l'aller-retour LFMS
        // exerce le réimport ; RADIUS et RTPS sont des captures loopback BSD.
        ("dnp3", LinkType::ETHERNET),
        ("iec104", LinkType::ETHERNET),
        ("bacnet_ip", LinkType::ETHERNET),
        ("goose_l2", LinkType::ETHERNET),
        ("radius", LinkType::NULL),
        ("ike_v1_main_mode", LinkType::ETHERNET),
        ("wireguard", LinkType::ETHERNET),
        ("rtps_cyclonedds", LinkType::NULL),
        ("dis", LinkType::ETHERNET),
        ("stanag5066_expedited", LinkType::ETHERNET),
    ] {
        let capture = fixtures.join(format!("{name}.pcap"));
        let oracle = fixtures.join(format!("{name}.flows.tsv"));
        assert_oracle_hash(&capture, &oracle);
        let metadata = read_oracle_metadata(&oracle);

        let mut matrix = FlowMatrix::new();
        let report = append_pcap_file(&mut matrix, &capture)
            .unwrap_or_else(|error| panic!("conversion de {name}: {error}"));
        assert_eq!(report.read() as u64, metadata.included_packets, "{name}");
        assert_eq!(report.decoded, report.read(), "{name}");
        assert_eq!(report.rejected(), 0, "{name}");
        assert_eq!(matrix.link_type, Some(expected_link_type), "{name}");

        let actual = common_projection(&matrix.to_flat_vec());
        let expected = read_tshark_common_flows(&oracle);
        assert_eq!(
            expected
                .values()
                .map(|stats| stats.total_bytes)
                .sum::<u64>(),
            metadata.included_bytes,
            "métadonnées d'oracle incohérentes pour {name}"
        );
        assert_common_flows_equal(name, &actual, &expected);
        assert_deterministic_round_trip(&matrix, name);
    }
}

#[test]
fn capwap_data_matches_outer_and_inner_tshark_flows_and_encap_id() {
    let fixtures = corpus_dir();
    let capture = fixtures.join("capwap_data.pcap");
    let outer_oracle = fixtures.join("capwap_data.flows.tsv");
    let inner_oracle = fixtures.join("capwap_data.inner.flows.tsv");
    assert_oracle_hash(&capture, &outer_oracle);
    assert_oracle_hash(&capture, &inner_oracle);

    let mut matrix = FlowMatrix::new();
    let report = append_pcap_file(&mut matrix, &capture).expect("conversion CAPWAP");
    assert_eq!(
        (report.read(), report.decoded, report.rejected()),
        (2, 2, 0)
    );

    let rows = matrix.to_flat_vec();
    assert_eq!(rows.len(), 4, "deux niveaux par trame CAPWAP");
    let encap_ids = rows
        .iter()
        .map(|row| {
            assert!(!row.encap_id.is_empty(), "encap_id CAPWAP absent");
            row.encap_id.as_str()
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        encap_ids.len(),
        1,
        "les deux sens et niveaux sont joignables"
    );
    assert!(
        encap_ids.iter().all(|id| id.len() == 16),
        "encap_id hexadécimal stable sur 64 bits"
    );

    let outer_rows = rows
        .iter()
        .filter(|row| row.application_protocol.as_deref() == Some("CAPWAP"))
        .cloned()
        .collect::<Vec<_>>();
    let inner_rows = rows
        .iter()
        .filter(|row| row.application_protocol.as_deref() != Some("CAPWAP"))
        .cloned()
        .collect::<Vec<_>>();
    assert_common_flows_equal(
        "capwap_data/outer",
        &common_projection(&outer_rows),
        &read_tshark_common_flows(&outer_oracle),
    );
    assert_common_flows_equal(
        "capwap_data/inner",
        &common_projection(&inner_rows),
        &read_tshark_common_flows(&inner_oracle),
    );
    assert_deterministic_round_trip(&matrix, "capwap_data");
}

/// GTP-U (packet_parser 11.2.0) : chaque G-PDU donne une ligne
/// externe UDP 2152 et une ligne pour le paquet IP transporté — sans trame,
/// donc sans MAC —, jointes par l'`encap_id` de leur sens. Les fixtures ne
/// portent que des datagrammes externes non fragmentés : TShark les
/// réassemblerait, packet_parser non.
#[test]
fn gtpu_matches_outer_and_inner_tshark_flows_and_encap_id() {
    let fixtures = corpus_dir();
    // `tunnel_pair_id` hache aussi les MAC des extrémités : les deux sens d'un
    // tunnel ne partagent leur identifiant que si le chemin de niveau 2 est
    // symétrique. Dans gtpu_gn, chaque sens sort par sa propre passerelle
    // (MAC virtuelle HSRP 00:00:0c:07:ac:… d'un côté, VRRP 00:00:5e:00:01:…
    // de l'autre) : un identifiant par sens. gtpu_ipv6 n'a qu'un sens.
    for (name, packets, tunnel_ids) in [("gtpu_gn", 32_usize, 2_usize), ("gtpu_ipv6", 2, 1)] {
        let capture = fixtures.join(format!("{name}.pcap"));
        let outer_oracle = fixtures.join(format!("{name}.flows.tsv"));
        let inner_oracle = fixtures.join(format!("{name}.inner.flows.tsv"));
        assert_oracle_hash(&capture, &outer_oracle);
        assert_oracle_hash(&capture, &inner_oracle);

        let mut matrix = FlowMatrix::new();
        let report = append_pcap_file(&mut matrix, &capture)
            .unwrap_or_else(|error| panic!("conversion de {name}: {error}"));
        assert_eq!(
            (report.read(), report.decoded, report.rejected()),
            (packets, packets, 0),
            "{name}"
        );

        let (inner_rows, outer_rows): (Vec<_>, Vec<_>) = matrix
            .to_flat_vec()
            .into_iter()
            .partition(is_ip_tunnel_inner);
        assert!(
            outer_rows
                .iter()
                .all(|row| row.application_protocol.as_deref() == Some("GTP-U")),
            "{name} : toute ligne externe est un G-PDU"
        );
        let encap_ids = |rows: &[FlowMatrixRow]| {
            rows.iter()
                .map(|row| row.encap_id.clone())
                .collect::<BTreeSet<_>>()
        };
        let outer_ids = encap_ids(&outer_rows);
        assert_eq!(
            encap_ids(&inner_rows),
            outer_ids,
            "{name} : chaque niveau interne est joint à la ligne externe de son sens"
        );
        assert_eq!(outer_ids.len(), tunnel_ids, "{name}");
        assert!(
            outer_ids.iter().all(|id| id.len() == 16),
            "{name} : encap_id hexadécimal stable sur 64 bits"
        );

        assert_common_flows_equal(
            &format!("{name}/outer"),
            &common_projection(&outer_rows),
            &read_tshark_common_flows(&outer_oracle),
        );
        assert_common_flows_equal(
            &format!("{name}/inner"),
            &common_projection(&inner_rows),
            &read_tshark_common_flows(&inner_oracle),
        );
        assert_deterministic_round_trip(&matrix, name);
    }
}

#[test]
fn asterix_captures_are_labelled_exactly_like_tshark() {
    // packet_parser 11.3.0 reconnaît ASTERIX par sa seule
    // structure, sans port ni magic ; TShark ne le dissèque que sur
    // `-d udp.port==…,asterix`. Les oracles ne portant pas d'étiquette
    // applicative, on fige ici que chaque flux de ces deux captures est
    // ASTERIX : les 303 trames (203 + 100) que `tshark -Y asterix` voit, et
    // aucune autre.
    let fixtures = corpus_dir();
    for (name, expected_packets) in [
        ("asterix_cat048_multicast", 203_u64),
        ("asterix_cat034_048", 100),
    ] {
        let mut matrix = FlowMatrix::new();
        let report = append_pcap_file(&mut matrix, &fixtures.join(format!("{name}.pcap")))
            .unwrap_or_else(|error| panic!("conversion de {name}: {error}"));
        assert_eq!(report.read() as u64, expected_packets, "{name}");
        assert_eq!(report.rejected(), 0, "{name}");

        let rows = matrix.to_flat_vec();
        let labels: Vec<Option<&str>> = rows
            .iter()
            .map(|row| row.application_protocol.as_deref())
            .collect();
        assert!(
            labels.iter().all(|label| *label == Some("ASTERIX")),
            "{name}: étiquettes {labels:?}"
        );
        assert_eq!(
            rows.iter().map(|row| row.count).sum::<u64>(),
            expected_packets,
            "{name}"
        );
        assert!(
            rows.iter()
                .all(|row| row.protocol_transport.as_deref() == Some("UDP")),
            "{name}: ASTERIX ne voyage ici qu'en UDP"
        );
    }
}

#[test]
fn ptp_captures_are_labelled_exactly_like_tshark() {
    // packet_parser 11.5.0 reconnaît PTP sur UDP 319/320 (étiquette
    // applicative) et directement sur Ethernet (EtherType 0x88F7, couche
    // liaison « PTP », sans IP ni transport). Comptes de `tshark -Y` :
    // `ptp && udp` et `ptp && !udp`. Les oracles ne portant pas
    // d'étiquette applicative, on fige ici que chaque trame PTP est
    // étiquetée, et aucune autre.
    let fixtures = corpus_dir();
    for (name, expected_udp, expected_layer2) in
        [("ptp_udp_l2", 25_u64, 14_u64), ("ptp_ipv6", 14, 0)]
    {
        let mut matrix = FlowMatrix::new();
        let report = append_pcap_file(&mut matrix, &fixtures.join(format!("{name}.pcap")))
            .unwrap_or_else(|error| panic!("conversion de {name}: {error}"));
        assert_eq!(
            report.read() as u64,
            expected_udp + expected_layer2,
            "{name}"
        );
        assert_eq!(report.rejected(), 0, "{name}");

        let rows = matrix.to_flat_vec();
        let udp: Vec<_> = rows
            .iter()
            .filter(|row| row.protocol_transport.as_deref() == Some("UDP"))
            .collect();
        assert!(
            udp.iter()
                .all(|row| row.application_protocol.as_deref() == Some("PTP")),
            "{name}: étiquettes UDP {:?}",
            udp.iter()
                .map(|row| row.application_protocol.as_deref())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            udp.iter().map(|row| row.count).sum::<u64>(),
            expected_udp,
            "{name}: trames PTP sur UDP"
        );

        let layer2: Vec<_> = rows
            .iter()
            .filter(|row| row.protocol_transport.is_none())
            .collect();
        assert!(
            layer2.iter().all(|row| row.protocol_data_link == "PTP"
                && row.ip_source.is_empty()
                && row.ip_destination.is_empty()),
            "{name}: lignes de couche 2 {layer2:?}"
        );
        assert_eq!(
            layer2.iter().map(|row| row.count).sum::<u64>(),
            expected_layer2,
            "{name}: trames PTP en couche 2"
        );
    }
}

#[test]
fn packet_parser_11_6_captures_are_labelled_exactly_like_tshark() {
    // packet_parser 11.6.0 : chaque capture porte exactement les trames que
    // le filtre TShark correspondant voit (`dnp3`, `iec60870_104`, `bvlc`,
    // `radius`, `isakmp`, `wg`, `rtps`, `dis`, `s5066sis`), et aucune autre
    // étiquette applicative n'apparaît : le reste est de l'ouverture ou de
    // l'acquittement TCP sans charge (`None`) ou une charge non reconnue
    // (`Unknown`). Les oracles ne portant pas d'étiquette, les comptes sont
    // figés ici.
    let fixtures = corpus_dir();
    for (name, label, expected_labelled, expected_frames) in [
        ("dnp3", "DNP3", 115_u64, 214_u64),
        ("iec104", "IEC 104", 64, 105),
        ("bacnet_ip", "BACnet/IP", 23, 23),
        ("radius", "RADIUS", 19, 19),
        ("ike_v1_main_mode", "IKE", 12, 12),
        ("wireguard", "WireGuard", 22, 22),
        ("rtps_cyclonedds", "RTPS", 87, 87),
        ("dis", "DIS", 253, 253),
        ("stanag5066_expedited", "STANAG 5066", 13, 13),
    ] {
        let mut matrix = FlowMatrix::new();
        let report = append_pcap_file(&mut matrix, &fixtures.join(format!("{name}.pcap")))
            .unwrap_or_else(|error| panic!("conversion de {name}: {error}"));
        assert_eq!(report.read() as u64, expected_frames, "{name}");
        assert_eq!(report.rejected(), 0, "{name}");

        let rows = matrix.to_flat_vec();
        assert_eq!(
            rows.iter()
                .filter(|row| row.application_protocol.as_deref() == Some(label))
                .map(|row| row.count)
                .sum::<u64>(),
            expected_labelled,
            "{name}: trames étiquetées {label}"
        );
        let foreign: Vec<_> = rows
            .iter()
            .filter_map(|row| row.application_protocol.as_deref())
            .filter(|other| *other != label && *other != "Unknown")
            .collect();
        assert!(
            foreign.is_empty(),
            "{name}: étiquettes inattendues {foreign:?}"
        );
    }

    // GOOSE (IEC 61850-8-1) voyage directement sur Ethernet, EtherType
    // 0x88B8 : couche liaison « GOOSE », sans IP, transport ni étiquette
    // applicative. `tshark -Y goose` voit les 8 trames.
    let mut matrix = FlowMatrix::new();
    let report = append_pcap_file(&mut matrix, &fixtures.join("goose_l2.pcap"))
        .expect("conversion de goose_l2");
    assert_eq!(report.read(), 8);
    assert_eq!(report.rejected(), 0);
    let rows = matrix.to_flat_vec();
    assert!(
        rows.iter().all(|row| row.protocol_data_link == "GOOSE"
            && row.ip_source.is_empty()
            && row.ip_destination.is_empty()
            && row.protocol_transport.is_none()
            && row.application_protocol.is_none()),
        "goose_l2: lignes {rows:?}"
    );
    assert_eq!(rows.iter().map(|row| row.count).sum::<u64>(), 8);
}

#[test]
fn unsupported_ieee80211_capture_is_refused_before_matrix_mutation() {
    let fixtures = corpus_dir();
    let capture = fixtures.join("unsupported_ieee80211.pcapng");
    assert_oracle_hash(&capture, &fixtures.join("unsupported_ieee80211.flows.tsv"));
    let mut matrix = FlowMatrix::new();
    let error = append_pcap_file(&mut matrix, &capture).expect_err("DLT 802.11 refusé");
    assert!(matches!(error, LunarCoreError::UnsupportedLinkType { .. }));
    assert_eq!(matrix.row_count(), 0);
}
