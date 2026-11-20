//! Références figées : la page de synthèse, les tableaux de la vue
//! d'investigation et les formats fr-FR doivent rester identiques, à
//! l'octet, aux fichiers de `tests/golden/`.
//!
//! Chaque fixture `<nom>.input.json` a sa page `<nom>.html` et ses tableaux
//! `<nom>.tables.json` ; `locale.json` porte les formats fr-FR (V8/ICU), la
//! collation, la normalisation, les dates et les tables fixes.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "binaire de test : ses fonctions d'aide doivent faire échouer le test sur une fixture invalide"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use lunar_flows_core::graph::{Edge, GraphData, Node};
use lunar_ui_logic::investigation::dates::{
    EpochMillis, FixedZone, compact_date, format_long_medium, format_short_medium, parse_js_date,
    to_iso_string,
};
use lunar_ui_logic::investigation::deductions::{
    is_local_or_group_mac, known_ip, known_mac, mac_oui, oui_vendor,
};
use lunar_ui_logic::investigation::glossary::{
    GLOSSARY, entry_tokens, match_glossary, normalize_token,
};
use lunar_ui_logic::investigation::locale::{
    compare_fr, format_decimal, format_integer, js_number_to_string, to_fixed,
};
use lunar_ui_logic::investigation::period::{PeriodSummary, format_duration, summarize_period};
use lunar_ui_logic::investigation::ports::port_service;
use lunar_ui_logic::investigation::protocol_colors::color_for_protocol;
use lunar_ui_logic::investigation::protocol_traits::{is_classified, protocol_traits, usage_of};
use lunar_ui_logic::investigation::tables::{
    DEFAULT_TOP_N, DeductionRow, EndpointRow, FlowRow, InvestigationStats, InvestigationSummary,
    PortRow, Totals, TraitShares, build_deduction_rows, build_endpoint_rows, build_flow_rows,
    build_port_rows, build_summary, build_trait_shares, compute_stats, filter_graph_by_protocols,
    format_bytes, observed_glossary_tokens, percent_of, protocol_breakdown,
};
use lunar_ui_logic::investigation::view::{
    InvestigationRow, SortKey, bar_width, filter_rows, sort_rows, wiki_entries,
};
use lunar_ui_logic::investigation::{
    GraphView, SurveyPeriodView, SynthesisExport, render_synthesis_html,
};
use serde_json::{Map, Value, json};

/// Dossier des fichiers de référence.
fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

/// Fixtures `*.input.json`, par nom.
fn fixtures() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(golden_dir())
        .expect("dossier golden")
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter_map(|name| name.strip_suffix(".input.json").map(str::to_string))
        .collect();
    names.sort();
    assert!(names.len() >= 3, "fixtures manquantes : {names:?}");
    names
}

fn read(name: &str) -> String {
    std::fs::read_to_string(golden_dir().join(name))
        .unwrap_or_else(|error| panic!("{name} : {error}"))
}

fn read_json(name: &str) -> Value {
    serde_json::from_str(&read(name)).unwrap_or_else(|error| panic!("{name} : {error}"))
}

fn text(value: &Value) -> String {
    value
        .as_str()
        .unwrap_or_else(|| panic!("chaîne attendue : {value}"))
        .to_string()
}

fn optional_text(value: &Value) -> Option<String> {
    value.as_str().map(str::to_string)
}

fn number(value: &Value) -> u64 {
    value
        .as_u64()
        .unwrap_or_else(|| panic!("entier attendu : {value}"))
}

fn port(value: &Value) -> u16 {
    u16::try_from(number(value)).expect("port")
}

fn flag(value: &Value) -> bool {
    value
        .as_bool()
        .unwrap_or_else(|| panic!("booléen attendu : {value}"))
}

fn texts(value: &Value) -> Vec<String> {
    value
        .as_array()
        .expect("tableau")
        .iter()
        .map(text)
        .collect()
}

/// Nœud du cœur depuis sa forme JSON.
fn node_from_json(value: &Value) -> Node {
    Node {
        id: text(&value["id"]),
        name: text(&value["name"]),
        color: text(&value["color"]),
        mac: text(&value["mac"]),
        macs: texts(&value["macs"]),
        ip: text(&value["ip"]),
        vlan_id: value["vlan_id"]
            .as_u64()
            .map(|vlan| u16::try_from(vlan).expect("VLAN")),
        duplicate_ip: flag(&value["duplicate_ip"]),
        gateway_mac: flag(&value["gateway_mac"]),
        label: optional_text(&value["label"]),
    }
}

/// Arête du cœur depuis sa forme JSON (clés camelCase).
fn edge_from_json(value: &Value) -> Edge {
    Edge {
        id: text(&value["id"]),
        source: text(&value["source"]),
        target: text(&value["target"]),
        label: text(&value["label"]),
        source_port: value["sourcePort"]
            .as_u64()
            .map(|port| u16::try_from(port).expect("port")),
        destination_port: value["destinationPort"]
            .as_u64()
            .map(|port| u16::try_from(port).expect("port")),
        ports: value["ports"]
            .as_array()
            .expect("ports")
            .iter()
            .map(port)
            .collect::<BTreeSet<_>>(),
        has_dynamic_ports: flag(&value["hasDynamicPorts"]),
        bidir: flag(&value["bidir"]),
        count: number(&value["count"]),
        total_bytes: number(&value["totalBytes"]),
        encap_ids: texts(&value["encapIds"]),
    }
}

fn graph_from_json(value: &Value) -> GraphData {
    let mut graph = GraphData::new();
    for (key, node) in value["nodes"].as_object().expect("nœuds") {
        graph.nodes.insert(key.clone(), node_from_json(node));
    }
    for (key, edge) in value["edges"].as_object().expect("arêtes") {
        graph.edges.insert(key.clone(), edge_from_json(edge));
    }
    graph
}

fn period_from_json(value: &Value) -> SurveyPeriodView {
    match value["state"].as_str() {
        Some("known") => SurveyPeriodView::Known {
            first: text(&value["first"]),
            last: text(&value["last"]),
        },
        Some("unknown") => SurveyPeriodView::Unknown,
        _ => SurveyPeriodView::Empty,
    }
}

/// Fixture : export, instant de génération et fuseau.
struct Fixture {
    export: SynthesisExport,
    generated_at: EpochMillis,
    zone: FixedZone,
}

fn fixture(name: &str) -> Fixture {
    let input = read_json(&format!("{name}.input.json"));
    let offset =
        i32::try_from(input["offsetSeconds"].as_i64().expect("offsetSeconds")).expect("décalage");
    Fixture {
        export: SynthesisExport {
            graph: graph_from_json(&input["graph"]),
            csv_file_name: text(&input["csvFileName"]),
            csv_sha256: text(&input["csvSha256"]),
            csv_bytes: number(&input["csvBytes"]),
            link_type: optional_text(&input["linkType"]),
            site: optional_text(&input["site"]),
            sensor: optional_text(&input["sensor"]),
            interface: optional_text(&input["interface"]),
            app_name: "LUNAR".to_string(),
            app_version: text(&input["appVersion"]),
            period: period_from_json(&input["period"]),
        },
        generated_at: parse_js_date(&text(&input["generatedAt"])).expect("generatedAt"),
        zone: FixedZone::new(offset),
    }
}

/// Compare deux textes et, s'ils diffèrent, montre la première ligne
/// divergente.
fn assert_same_text(name: &str, expected: &str, actual: &str) {
    if expected == actual {
        return;
    }
    let (expected_lines, actual_lines): (Vec<_>, Vec<_>) =
        (expected.lines().collect(), actual.lines().collect());
    let line = expected_lines
        .iter()
        .zip(&actual_lines)
        .position(|(a, b)| a != b)
        .unwrap_or(expected_lines.len().min(actual_lines.len()));
    panic!(
        "{name} : diffère ligne {}\n  attendu : {:?}\n  obtenu  : {:?}\n(longueurs {} / {})",
        line + 1,
        expected_lines.get(line),
        actual_lines.get(line),
        expected.len(),
        actual.len()
    );
}

/// Compare deux valeurs JSON, clé par clé pour un objet, avec le chemin de
/// la première différence.
fn assert_same_json(path: &str, expected: &Value, actual: &Value) {
    match (expected, actual) {
        (Value::Object(expected), Value::Object(actual)) => {
            let keys: BTreeSet<_> = expected.keys().chain(actual.keys()).collect();
            for key in keys {
                assert_same_json(
                    &format!("{path}.{key}"),
                    expected.get(key).unwrap_or(&Value::Null),
                    actual.get(key).unwrap_or(&Value::Null),
                );
            }
        }
        (Value::Array(expected_items), Value::Array(actual_items))
            if expected_items.len() == actual_items.len() =>
        {
            for (index, (expected, actual)) in expected_items.iter().zip(actual_items).enumerate() {
                assert_same_json(&format!("{path}[{index}]"), expected, actual);
            }
        }
        _ => assert_eq!(expected, actual, "{path}"),
    }
}

// --- Page de synthèse ----------------------------------------------------

#[test]
fn page_de_synthese_identique_aux_references() {
    for name in fixtures() {
        let fixture = fixture(&name);
        let html = render_synthesis_html(&fixture.export, fixture.generated_at, &fixture.zone);
        assert_same_text(
            &format!("{name}.html"),
            &read(&format!("{name}.html")),
            &html,
        );
    }
}

// --- Tableaux de la vue ----------------------------------------------------

fn flow_json(row: &FlowRow) -> Value {
    json!({
        "id": row.id, "sourceId": row.source_id, "targetId": row.target_id, "source": row.source,
        "target": row.target, "protocol": row.protocol, "ports": row.ports, "bidir": row.bidir,
        "packets": row.packets, "bytes": row.bytes,
    })
}

fn endpoint_json(row: &EndpointRow) -> Value {
    json!({
        "id": row.id, "name": row.name, "ip": row.ip, "mac": row.mac, "macDisplay": row.mac_display,
        "macs": row.macs, "vlan": row.vlan, "label": row.label, "duplicateIp": row.duplicate_ip,
        "degree": row.degree, "flowsOut": row.flows_out, "flowsIn": row.flows_in, "packets": row.packets,
        "bytes": row.bytes,
    })
}

fn port_json(row: &PortRow) -> Value {
    json!({ "port": row.port, "service": row.service, "flows": row.flows, "packets": row.packets, "bytes": row.bytes })
}

fn deduction_json(row: &DeductionRow) -> Value {
    json!({ "kind": row.kind.label(), "observed": row.observed, "deduction": row.deduction, "count": row.count })
}

fn stats_json(stats: &InvestigationStats) -> Value {
    json!({
        "endpoints": stats.endpoints, "flows": stats.flows, "packets": stats.packets, "bytes": stats.bytes,
        "protocols": stats.protocols, "duplicateIps": stats.duplicate_ips,
    })
}

fn totals_json(totals: &Totals) -> Value {
    json!({ "flows": totals.flows, "packets": totals.packets, "bytes": totals.bytes })
}

fn with_totals(mut base: Map<String, Value>, totals: &Totals) -> Value {
    if let Value::Object(totals) = totals_json(totals) {
        base.extend(totals);
    }
    Value::Object(base)
}

fn traits_json(
    scope: &lunar_ui_logic::investigation::tables::ScopeShares,
    shares: &TraitShares,
) -> Value {
    json!({
        "scope": { "internal": totals_json(&scope.internal), "internet": totals_json(&scope.internet) },
        "confidentiality": shares.confidentiality.iter().map(|part| {
            let mut base = Map::new();
            base.insert("kind".into(), json!(part.kind.label()));
            with_totals(base, &part.totals)
        }).collect::<Vec<_>>(),
        "usages": shares.usages.iter().map(|part| {
            let mut base = Map::new();
            base.insert("usage".into(), json!(part.usage.label()));
            with_totals(base, &part.totals)
        }).collect::<Vec<_>>(),
    })
}

fn summary_json(summary: &InvestigationSummary) -> Value {
    let shares = TraitShares {
        scope: summary.scope,
        confidentiality: summary.confidentiality.clone(),
        usages: summary.usages.clone(),
    };
    let mut value = traits_json(&summary.scope, &shares);
    let object = value.as_object_mut().expect("objet");
    object.insert(
        "protocols".into(),
        summary
            .protocols
            .iter()
            .map(|share| json!({ "protocol": share.protocol, "flows": share.flows, "packets": share.packets, "bytes": share.bytes }))
            .collect(),
    );
    object.insert(
        "topEndpoints".into(),
        summary.top_endpoints.iter().map(endpoint_json).collect(),
    );
    object.insert(
        "topFlows".into(),
        summary.top_flows.iter().map(flow_json).collect(),
    );
    object.insert("unlabeled".into(), json!(summary.unlabeled));
    value
}

const QUERIES: [&str; 13] = [
    "", "true", "null", "192.168", "10.0.1", "modbus", "  DNS ", "tls", "ff:ff", "doublon", "é",
    "aa000", "—",
];
const WIKI_QUERIES: [&str; 6] = ["", "modbus", "réseau", "tcp", "vlan", "zz"];

fn sorts<T: InvestigationRow + Clone>(rows: &[T], id: impl Fn(&T) -> String) -> Value {
    let mut out = Map::new();
    for key in SortKey::ALL {
        for ascending in [true, false] {
            let sorted = sort_rows(rows.to_vec(), key, ascending);
            out.insert(
                format!("{}:{}", key.name(), if ascending { "asc" } else { "desc" }),
                sorted.iter().map(&id).collect(),
            );
        }
    }
    Value::Object(out)
}

fn searches<T: InvestigationRow + Clone>(rows: &[T], id: impl Fn(&T) -> String) -> Value {
    QUERIES
        .iter()
        .map(|query| {
            (
                (*query).to_string(),
                filter_rows(rows, query).iter().map(&id).collect(),
            )
        })
        .collect::<Map<_, _>>()
        .into()
}

fn tables_json(graph: &GraphView<'_>) -> Map<String, Value> {
    let flows = build_flow_rows(graph);
    let endpoints: Vec<EndpointRow> = build_endpoint_rows(graph)
        .into_iter()
        .filter(|row| row.degree > 0)
        .collect();
    let deductions = build_deduction_rows(graph);
    let present = match_glossary(&observed_glossary_tokens(graph));
    let stats = compute_stats(graph);
    let summary = build_summary(graph, DEFAULT_TOP_N);
    let mut tokens: Vec<String> = observed_glossary_tokens(graph).into_iter().collect();
    tokens.sort();
    let share_bars = |parts: Vec<u64>| -> Value {
        parts
            .iter()
            .map(|&bytes| {
                json!([
                    percent_of(bytes, stats.bytes),
                    bar_width(bytes, stats.bytes)
                ])
            })
            .collect()
    };
    let max_protocol = summary.protocols.first().map_or(0, |share| share.bytes);
    let trait_shares = build_trait_shares(graph);
    let mut wiki = Map::new();
    for all in [false, true] {
        for query in WIKI_QUERIES {
            let base = if all {
                GLOSSARY.iter().collect()
            } else {
                present.clone()
            };
            wiki.insert(
                format!("{}:{query}", if all { "all" } else { "present" }),
                wiki_entries(base, query)
                    .iter()
                    .map(|entry| json!(entry.term))
                    .collect(),
            );
        }
    }
    let deduction_id = |row: &DeductionRow| format!("{}:{}", row.kind.label(), row.observed);
    let object = json!({
        "stats": stats_json(&stats),
        "protocolBreakdown": protocol_breakdown(graph).iter().map(|row| json!({ "protocol": row.protocol, "flows": row.flows })).collect::<Vec<_>>(),
        "flowRows": flows.iter().map(flow_json).collect::<Vec<_>>(),
        "endpointRows": build_endpoint_rows(graph).iter().map(endpoint_json).collect::<Vec<_>>(),
        "portRows": build_port_rows(graph).iter().map(port_json).collect::<Vec<_>>(),
        "deductionRows": deductions.iter().map(deduction_json).collect::<Vec<_>>(),
        "summary": summary_json(&summary),
        "summaryBars": {
            "protocols": summary.protocols.iter().map(|share| bar_width(share.bytes, max_protocol)).collect::<Vec<_>>(),
            "scope": share_bars(vec![summary.scope.internal.bytes, summary.scope.internet.bytes]),
            "confidentiality": share_bars(summary.confidentiality.iter().map(|part| part.totals.bytes).collect()),
            "usages": share_bars(summary.usages.iter().map(|part| part.totals.bytes).collect()),
        },
        "traitShares": traits_json(&trait_shares.scope, &trait_shares),
        "glossaryTokens": tokens,
        "presentGlossary": present.iter().map(|entry| entry.term).collect::<Vec<_>>(),
        "wiki": wiki,
        "flowSorts": sorts(&flows, |row| row.id.clone()),
        "endpointSorts": sorts(&endpoints, |row| row.id.clone()),
        "deductionSorts": sorts(&deductions, deduction_id),
        "flowSearches": searches(&flows, |row| row.id.clone()),
        "endpointSearches": searches(&endpoints, |row| row.id.clone()),
        "deductionSearches": searches(&deductions, deduction_id),
    });
    match object {
        Value::Object(map) => map,
        _ => unreachable!(),
    }
}

#[test]
fn tableaux_de_la_vue_identiques_aux_references() {
    for name in fixtures() {
        let fixture = fixture(&name);
        let graph = GraphView::new(&fixture.export.graph);
        let options = protocol_breakdown(&graph);
        let hidden: Vec<String> = options
            .iter()
            .take(1)
            .map(|option| option.protocol.clone())
            .collect();
        let allowed = options
            .iter()
            .skip(1)
            .map(|option| option.protocol.clone())
            .collect();
        let filtered = filter_graph_by_protocols(&graph, &allowed);
        let mut filtered_tables = Map::new();
        filtered_tables.insert("hidden".into(), json!(hidden));
        filtered_tables.extend(tables_json(&filtered));
        let period = match summarize_period(&fixture.export.period) {
            PeriodSummary::Known {
                first,
                last,
                duration,
            } => {
                json!({ "state": "known", "first": first.map(|at| at.0), "last": last.map(|at| at.0), "duration": duration })
            }
            PeriodSummary::Unknown => json!({ "state": "unknown" }),
            PeriodSummary::Empty => json!({ "state": "empty" }),
        };
        let actual =
            json!({ "full": tables_json(&graph), "filtered": filtered_tables, "period": period });
        assert_same_json(&name, &read_json(&format!("{name}.tables.json")), &actual);
    }
}

// --- Formats fr-FR, collation, normalisation, dates, tables fixes ----------

fn as_f64(value: &Value) -> f64 {
    value
        .as_f64()
        .unwrap_or_else(|| panic!("nombre attendu : {value}"))
}

fn pairs<'a>(locale: &'a Value, key: &str) -> &'a Vec<Value> {
    locale[key]
        .as_array()
        .unwrap_or_else(|| panic!("{key} absent"))
}

#[test]
fn formats_fr_fr_identiques_aux_references() {
    let locale = read_json("locale.json");
    for item in pairs(&locale, "formatBytes") {
        assert_eq!(
            format_bytes(number(&item[0])),
            text(&item[1]),
            "formatBytes({})",
            item[0]
        );
    }
    for item in pairs(&locale, "formatDuration") {
        assert_eq!(
            format_duration(as_f64(&item[0])),
            text(&item[1]),
            "formatDuration({})",
            item[0]
        );
    }
    for item in pairs(&locale, "percentOf") {
        assert_eq!(
            percent_of(number(&item[0]), number(&item[1])),
            text(&item[2]),
            "percentOf({}, {})",
            item[0],
            item[1]
        );
    }
    for item in pairs(&locale, "barWidth") {
        assert_eq!(
            bar_width(number(&item[0]), number(&item[1])),
            text(&item[2]),
            "barWidth({}, {})",
            item[0],
            item[1]
        );
    }
    for item in pairs(&locale, "decimals") {
        assert_eq!(
            format_decimal(as_f64(&item[0]), 1),
            text(&item[1]),
            "toLocaleString({})",
            item[0]
        );
    }
    for item in pairs(&locale, "integers") {
        assert_eq!(
            format_integer(number(&item[0])),
            text(&item[1]),
            "toLocaleString({})",
            item[0]
        );
    }
    for item in pairs(&locale, "toFixed") {
        let digits = usize::try_from(number(&item[1])).expect("décimales");
        assert_eq!(
            to_fixed(as_f64(&item[0]), digits),
            text(&item[2]),
            "({}).toFixed({digits})",
            item[0]
        );
    }
    for item in pairs(&locale, "numbers") {
        assert_eq!(
            js_number_to_string(as_f64(&item[0])),
            text(&item[1]),
            "String({})",
            item[0]
        );
    }
}

#[test]
fn collation_identique_a_locale_compare_fr() {
    let locale = read_json("locale.json");
    let corpus = texts(&locale["collation"]["corpus"]);
    let mut sorted = corpus.clone();
    sorted.sort_by(|a, b| compare_fr(a, b));
    assert_eq!(sorted, texts(&locale["collation"]["sorted"]));
    let signs = texts(&locale["collation"]["signs"]);
    for (a, line) in corpus.iter().zip(&signs) {
        let actual: String = corpus
            .iter()
            .map(|b| match compare_fr(a, b) {
                std::cmp::Ordering::Less => '<',
                std::cmp::Ordering::Equal => '=',
                std::cmp::Ordering::Greater => '>',
            })
            .collect();
        assert_eq!(&actual, line, "localeCompare({a:?}, …)");
    }
}

#[test]
fn normalize_token_identique_sur_tout_unicode() {
    let locale = read_json("locale.json");
    let expected: BTreeMap<String, String> = locale["normalizeToken"]["codePoints"]
        .as_object()
        .expect("codePoints")
        .iter()
        .map(|(cp, token)| (cp.clone(), text(token)))
        .collect();
    let actual: BTreeMap<String, String> = (0..=0x10FFFF_u32)
        .filter_map(char::from_u32)
        .filter_map(|character| {
            let token = normalize_token(&character.to_string());
            (!token.is_empty()).then(|| (format!("{:x}", u32::from(character)), token))
        })
        .collect();
    assert_eq!(actual, expected);
    for item in locale["normalizeToken"]["strings"]
        .as_array()
        .expect("strings")
    {
        assert_eq!(
            normalize_token(&text(&item[0])),
            text(&item[1]),
            "normalizeToken({})",
            item[0]
        );
    }
    let tokens: Vec<Value> = GLOSSARY
        .iter()
        .map(|entry| json!(entry_tokens(entry)))
        .collect();
    assert_eq!(Value::Array(tokens), locale["entryTokens"]);
}

#[test]
fn dates_identiques_a_date_javascript() {
    let locale = read_json("locale.json");
    // Références relevées sous TZ=Etc/GMT-2 (UTC+2).
    let zone = FixedZone::new(7200);
    for item in locale["dates"]["parse"].as_array().expect("parse") {
        assert_eq!(
            parse_js_date(&text(&item[0])).map(|at| at.0),
            item[1].as_i64(),
            "new Date({})",
            item[0]
        );
    }
    for item in locale["dates"]["format"].as_array().expect("format") {
        let at = item[0]
            .as_i64()
            .map(EpochMillis)
            .or_else(|| Some(EpochMillis(as_f64(&item[0]) as i64)));
        let at = at.filter(|at| at.0.abs() <= 8_640_000_000_000_000);
        assert_eq!(
            at.map(to_iso_string),
            optional_text(&item[1]),
            "toISOString({})",
            item[0]
        );
        assert_eq!(
            format_short_medium(at, &zone),
            text(&item[2]),
            "short/medium({})",
            item[0]
        );
        assert_eq!(
            format_long_medium(at, &zone),
            text(&item[3]),
            "long/medium({})",
            item[0]
        );
        assert_eq!(
            at.map(|at| compact_date(at, &zone)),
            optional_text(&item[4]),
            "getCurrentDate({})",
            item[0]
        );
    }
}

#[test]
fn tables_fixes_identiques_aux_references() {
    let locale = read_json("locale.json");
    for item in pairs(&locale, "macOui") {
        let mac = text(&item[0]);
        assert_eq!(mac_oui(&mac), text(&item[1]), "macOui({mac})");
        assert_eq!(
            is_local_or_group_mac(&mac),
            flag(&item[2]),
            "isLocalOrGroupMac({mac})"
        );
        assert_eq!(oui_vendor(&mac), text(&item[3]), "ouiVendor({mac})");
        assert_eq!(known_mac(&mac), text(&item[4]), "knownMac({mac})");
    }
    for item in pairs(&locale, "knownIp") {
        assert_eq!(
            known_ip(&text(&item[0])),
            text(&item[1]),
            "knownIp({})",
            item[0]
        );
    }
    let services: BTreeMap<u16, String> = locale["portService"]
        .as_object()
        .expect("portService")
        .iter()
        .map(|(port, service)| (port.parse().expect("port"), text(service)))
        .collect();
    let actual: BTreeMap<u16, String> = (0..=u16::MAX)
        .filter(|&port| !port_service(port).is_empty())
        .map(|port| (port, port_service(port).to_string()))
        .collect();
    assert_eq!(actual, services);
    for item in pairs(&locale, "protocols") {
        let label = text(&item[0]);
        let traits = protocol_traits(&label);
        assert_eq!(
            traits.usage.map(|usage| usage.label().to_string()),
            optional_text(&item[1]),
            "usage({label})"
        );
        assert_eq!(
            traits.confidentiality.label(),
            text(&item[2]),
            "confidentiality({label})"
        );
        assert_eq!(usage_of(&label).label(), text(&item[3]), "usageOf({label})");
        assert_eq!(
            is_classified(&label),
            flag(&item[4]),
            "isClassified({label})"
        );
        assert_eq!(
            color_for_protocol(Some(&label)),
            text(&item[5]),
            "colorForProtocol({label})"
        );
    }
}
