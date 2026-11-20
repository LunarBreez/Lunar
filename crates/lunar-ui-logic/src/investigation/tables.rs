//! Agrégations de la vue d'investigation : transforment le snapshot du graphe
//! (nœuds = endpoints, arêtes = flux) en lignes de tableau et indicateurs
//! synthétiques. Fonctions pures et sans dépendance de rendu — la vue ne fait
//! qu'afficher, trier et filtrer ce que ces fonctions calculent.
//!
//! La période du relevé est dans [`super::period`]. Les compteurs sont des
//! entiers : `u64` pour les paquets et octets (sommes saturantes, comme le
//! cœur), `usize` pour les dénombrements.

use std::collections::{HashMap, HashSet};
use std::hash::Hash;

use lunar_flows_core::graph::{Edge, Node};

use super::deductions::{known_ip, known_mac, mac_oui, oui_vendor};
use super::glossary::normalize_token;
use super::graph_view::GraphView;
use super::locale::{compare_fr, format_decimal, js_trim, to_fixed};
use super::ports::port_service;
use super::protocol_traits::{Confidentiality, Usage, protocol_traits, usage_of};

/// Ligne du tableau des flux (une par arête du graphe).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlowRow {
    /// Clé de l'arête dans la map du graphe.
    pub id: String,
    /// `Node.id` de la source.
    pub source_id: String,
    /// `Node.id` de la cible.
    pub target_id: String,
    /// Nom lisible de la source : label, sinon IP, sinon MAC, sinon id.
    pub source: String,
    /// Nom lisible de la cible.
    pub target: String,
    /// Protocole porté par l'arête (ex. TCP:443, ARP…).
    pub protocol: String,
    /// Ports observés, résumés (« 443 », « 80, 443 », « dynamiques »…).
    pub ports: String,
    /// Vrai si du trafic a été vu dans les deux sens.
    pub bidir: bool,
    /// Paquets cumulés.
    pub packets: u64,
    /// Octets cumulés.
    pub bytes: u64,
}

/// Ligne du tableau des endpoints (une par nœud du graphe).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EndpointRow {
    /// `Node.id`.
    pub id: String,
    /// Nom lisible : label, sinon IP, sinon MAC, sinon nom ou id.
    pub name: String,
    /// IP brute (vide si absente) : valeur envoyée au backend pour poser un
    /// label.
    pub ip: String,
    /// MAC brute portée par le nœud (clé d'étiquetage). Peut être une adresse
    /// non-unicast (broadcast d'une requête ARP) : ne pas afficher telle quelle.
    pub mac: String,
    /// MAC à afficher : première MAC unicast réellement observée, sinon vide
    /// (« — » côté vue). Un hôte seulement vu comme cible ARP n'a pas de vraie
    /// MAC connue — on ne montre pas le broadcast à sa place.
    pub mac_display: String,
    /// Toutes les MAC unicast observées pour cette IP (⚠ si plusieurs).
    pub macs: Vec<String>,
    /// VLAN 802.1Q du nœud.
    pub vlan: Option<u16>,
    /// Label posé, sans blancs autour ; vide si aucun.
    pub label: String,
    /// Vrai quand la même IP apparaît sur un autre VLAN (anomalie).
    pub duplicate_ip: bool,
    /// Nombre de flux incidents.
    pub degree: usize,
    /// Flux où l'endpoint émet : il est la source de l'arête, ou l'arête est
    /// bidirectionnelle. Un flux bidirectionnel compte dans les deux sens, donc
    /// `flows_out + flows_in` peut dépasser `degree`. Le volume, lui, n'est pas
    /// ventilé par sens : l'arête ne porte qu'un cumul (`count`, `total_bytes`).
    pub flows_out: usize,
    /// Flux où l'endpoint reçoit : cible de l'arête, ou arête bidirectionnelle.
    pub flows_in: usize,
    /// Paquets des flux incidents.
    pub packets: u64,
    /// Octets des flux incidents.
    pub bytes: u64,
}

/// Indicateurs synthétiques du relevé courant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct InvestigationStats {
    /// Endpoints reliés par au moins un flux.
    pub endpoints: usize,
    /// Flux.
    pub flows: usize,
    /// Paquets.
    pub packets: u64,
    /// Octets.
    pub bytes: u64,
    /// Nombre de protocoles distincts observés sur les flux.
    pub protocols: usize,
    /// Nombre d'endpoints dont l'IP est portée par plusieurs VLAN.
    pub duplicate_ips: usize,
}

/// Compteur qui garde l'ordre de première insertion, comme une `Map`
/// JavaScript : les tris stables qui suivent départagent ainsi les égalités
/// par ordre d'apparition.
struct OrderedTally<K, V> {
    /// Entrées dans l'ordre de première insertion.
    entries: Vec<(K, V)>,
    /// Clé → position dans `entries`.
    index: HashMap<K, usize>,
}

impl<K: Clone + Eq + Hash, V> OrderedTally<K, V> {
    /// Compteur vide.
    fn new() -> Self {
        Self {
            entries: Vec::new(),
            index: HashMap::new(),
        }
    }

    /// Valeur de `key`, créée par `init` à la première rencontre.
    fn entry(&mut self, key: K, init: impl FnOnce() -> V) -> &mut V {
        let position = match self.index.get(&key) {
            Some(&position) => position,
            None => {
                self.index.insert(key.clone(), self.entries.len());
                self.entries.push((key, init()));
                self.entries.len() - 1
            }
        };
        &mut self.entries[position].1
    }
}

/// Index des nœuds par leur `id` (et non par la clé de la map, qui est la clé
/// d'identité `(VLAN, IP/MAC)`). Les arêtes référencent `Node.id`, un hash
/// stable distinct de la clé de la map : sans cet index, la map ne trouve
/// jamais le nœud. À id égal, le dernier nœud parcouru l'emporte.
pub fn index_nodes_by_id<'a>(graph: &GraphView<'a>) -> HashMap<&'a str, &'a Node> {
    graph
        .nodes()
        .iter()
        .map(|&(_, node)| (node.id.as_str(), node))
        .collect()
}

/// Protocole affiché d'un flux : le label de l'arête, « — » si absent. Source
/// unique de la valeur affichée, filtrée et agrégée (pas de divergence).
pub fn edge_protocol(edge: &Edge) -> &str {
    if edge.label.is_empty() {
        "—"
    } else {
        &edge.label
    }
}

/// Nom lisible d'un endpoint : label utilisateur, sinon IP, sinon MAC, sinon
/// nom, sinon id.
pub fn endpoint_name(node: Option<&Node>, fallback_id: &str) -> String {
    let Some(node) = node else {
        return fallback_id.to_string();
    };
    if let Some(label) = node
        .label
        .as_deref()
        .map(js_trim)
        .filter(|label| !label.is_empty())
    {
        return label.to_string();
    }
    for candidate in [&node.ip, &node.mac, &node.name] {
        if !candidate.is_empty() {
            return candidate.clone();
        }
    }
    fallback_id.to_string()
}

/// Résumé lisible des ports d'un flux, sans inventer d'ordre : ports triés.
pub fn summarize_ports(edge: &Edge) -> String {
    if edge.has_dynamic_ports {
        return "dynamiques".to_string();
    }
    if edge.ports.is_empty() {
        return "—".to_string();
    }
    // `BTreeSet` : déjà triés par numéro.
    edge.ports
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Lignes du tableau des flux, une par arête, source/cible résolues en noms.
pub fn build_flow_rows(graph: &GraphView<'_>) -> Vec<FlowRow> {
    let by_id = index_nodes_by_id(graph);
    graph
        .edges()
        .iter()
        .map(|&(id, edge)| FlowRow {
            id: id.to_string(),
            source_id: edge.source.clone(),
            target_id: edge.target.clone(),
            source: endpoint_name(by_id.get(edge.source.as_str()).copied(), &edge.source),
            target: endpoint_name(by_id.get(edge.target.as_str()).copied(), &edge.target),
            protocol: edge_protocol(edge).to_string(),
            ports: summarize_ports(edge),
            bidir: edge.bidir,
            packets: edge.count,
            bytes: edge.total_bytes,
        })
        .collect()
}

/// Degré, sens et trafic d'un nœud, cumulés sur ses arêtes.
#[derive(Clone, Copy, Default)]
struct Incidence {
    /// Flux incidents.
    degree: usize,
    /// Flux émis.
    flows_out: usize,
    /// Flux reçus.
    flows_in: usize,
    /// Paquets cumulés.
    packets: u64,
    /// Octets cumulés.
    bytes: u64,
}

/// Lignes du tableau des endpoints, degré et trafic dérivés des arêtes
/// incidentes.
pub fn build_endpoint_rows(graph: &GraphView<'_>) -> Vec<EndpointRow> {
    // Trafic et degré par nœud : une seule passe sur les arêtes plutôt qu'un
    // balayage par nœud (évite un coût quadratique sur les gros relevés). Le
    // trafic est agrégé par `Node.id` (ce que portent les arêtes), pas par la
    // clé de la map des nœuds.
    let mut incidence: HashMap<&str, Incidence> = HashMap::new();
    for &(_, edge) in graph.edges() {
        for endpoint in [edge.source.as_str(), edge.target.as_str()] {
            let entry = incidence.entry(endpoint).or_default();
            entry.degree += 1;
            entry.packets = entry.packets.saturating_add(edge.count);
            entry.bytes = entry.bytes.saturating_add(edge.total_bytes);
        }
        incidence.entry(edge.source.as_str()).or_default().flows_out += 1;
        incidence.entry(edge.target.as_str()).or_default().flows_in += 1;
        // Une boucle (source = cible) est déjà comptée dans les deux sens.
        if edge.bidir && edge.source != edge.target {
            incidence.entry(edge.target.as_str()).or_default().flows_out += 1;
            incidence.entry(edge.source.as_str()).or_default().flows_in += 1;
        }
    }

    // `ip`/`mac` restent bruts (chaîne vide si absents) : ce sont les valeurs
    // envoyées au backend pour poser un label — l'affichage « — » est fait
    // par la vue, pas stocké ici.
    graph
        .nodes()
        .iter()
        .map(|&(_, node)| {
            let traffic = incidence.get(node.id.as_str()).copied().unwrap_or_default();
            EndpointRow {
                id: node.id.clone(),
                name: endpoint_name(Some(node), &node.id),
                ip: node.ip.clone(),
                mac: node.mac.clone(),
                // `macs` exclut déjà le broadcast/non-unicast côté backend : sa
                // première entrée est la vraie MAC à montrer ; vide = MAC
                // encore inconnue.
                mac_display: node.macs.first().cloned().unwrap_or_default(),
                macs: node.macs.clone(),
                vlan: node.vlan_id,
                label: node.label.as_deref().map(js_trim).unwrap_or("").to_string(),
                duplicate_ip: node.duplicate_ip,
                degree: traffic.degree,
                flows_out: traffic.flows_out,
                flows_in: traffic.flows_in,
                packets: traffic.packets,
                bytes: traffic.bytes,
            }
        })
        .collect()
}

/// Ligne du tableau des ports : un port « service » observé et son service
/// probable. `flows` = nombre de conversations où ce port apparaît ;
/// `packets` et `bytes` cumulent le trafic de ces conversations
/// (approximation : une conversation peut porter plusieurs ports, on n'en
/// scinde pas le trafic).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortRow {
    /// Numéro de port.
    pub port: u16,
    /// Service probable (vide si non répertorié).
    pub service: &'static str,
    /// Conversations où le port apparaît.
    pub flows: usize,
    /// Paquets de ces conversations.
    pub packets: u64,
    /// Octets de ces conversations.
    pub bytes: u64,
}

/// Agrège les ports « service » présents sur les arêtes (ceux retenus par le
/// cœur : port bas de chaque flux, hors plage dynamique). Trié par nombre de
/// flux décroissant, puis par numéro de port.
pub fn build_port_rows(graph: &GraphView<'_>) -> Vec<PortRow> {
    let mut rows: HashMap<u16, PortRow> = HashMap::new();
    for &(_, edge) in graph.edges() {
        for &port in &edge.ports {
            let row = rows.entry(port).or_insert_with(|| PortRow {
                port,
                service: port_service(port),
                flows: 0,
                packets: 0,
                bytes: 0,
            });
            row.flows += 1;
            row.packets = row.packets.saturating_add(edge.count);
            row.bytes = row.bytes.saturating_add(edge.total_bytes);
        }
    }
    let mut rows: Vec<PortRow> = rows.into_values().collect();
    rows.sort_by(|a, b| b.flows.cmp(&a.flows).then(a.port.cmp(&b.port)));
    rows
}

/// Jetons normalisés présents dans le relevé, pour surligner les entrées de
/// glossaire pertinentes (onglet Glossaire) : protocoles des flux, plus
/// quelques signaux structurels (VLAN, tunnels). L'appariement final vit dans
/// [`super::glossary::match_glossary`].
pub fn observed_glossary_tokens(graph: &GraphView<'_>) -> HashSet<String> {
    let mut tokens = HashSet::new();
    for &(_, edge) in graph.edges() {
        if !edge.label.is_empty() {
            tokens.insert(normalize_token(&edge.label));
        }
        if !edge.encap_ids.is_empty() {
            tokens.insert(normalize_token("tunnel"));
            tokens.insert(normalize_token("capwap"));
        }
    }
    if graph.nodes().iter().any(|(_, node)| node.vlan_id.is_some()) {
        tokens.insert(normalize_token("vlan"));
    }
    tokens.remove("");
    tokens
}

/// Catégorie d'une déduction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DeductionKind {
    /// « Port » : service probable d'un port.
    Port,
    /// « Fabricant (OUI) » : fabricant d'après le préfixe d'une MAC.
    Oui,
    /// « Adresse connue » : IP ou MAC notoire.
    KnownAddress,
}

impl DeductionKind {
    /// Libellé affiché.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Port => "Port",
            Self::Oui => "Fabricant (OUI)",
            Self::KnownAddress => "Adresse connue",
        }
    }

    /// Rang d'affichage par défaut (ports, fabricants, adresses).
    const fn order(self) -> u8 {
        match self {
            Self::Port => 0,
            Self::Oui => 1,
            Self::KnownAddress => 2,
        }
    }
}

/// Ligne de l'onglet Déduction : un élément observé, ce qu'on en déduit d'un
/// registre public, et le nombre d'occurrences (flux pour un port, endpoints
/// pour un OUI ou une adresse). Toujours une **probabilité**, pas une preuve.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeductionRow {
    /// Catégorie.
    pub kind: DeductionKind,
    /// Élément observé (port, OUI « XX:XX:XX », IP ou MAC).
    pub observed: String,
    /// Ce qu'en dit le registre.
    pub deduction: &'static str,
    /// Occurrences.
    pub count: usize,
}

/// Rassemble tout ce que LUNAR peut inférer de registres publics : services
/// probables des ports, fabricants (OUI des MAC), IP/MAC notoires. Ne liste
/// que ce qui a effectivement été reconnu — un port/OUI/adresse inconnu
/// n'apparaît pas (ce n'est pas une déduction).
pub fn build_deduction_rows(graph: &GraphView<'_>) -> Vec<DeductionRow> {
    let mut rows: Vec<DeductionRow> = build_port_rows(graph)
        .into_iter()
        .filter(|port| !port.service.is_empty())
        .map(|port| DeductionRow {
            kind: DeductionKind::Port,
            observed: port.port.to_string(),
            deduction: port.service,
            count: port.flows,
        })
        .collect();

    let endpoints: Vec<EndpointRow> = build_endpoint_rows(graph)
        .into_iter()
        .filter(|row| row.degree > 0)
        .collect();

    // Fabricants : un préfixe OUI distinct par endpoint, compté sur les
    // endpoints.
    let mut oui_counts: OrderedTally<String, (&'static str, usize)> = OrderedTally::new();
    for endpoint in &endpoints {
        let mut seen = HashSet::new();
        for mac in &endpoint.macs {
            let oui = mac_oui(mac);
            let vendor = oui_vendor(mac);
            if oui.is_empty() || vendor.is_empty() || !seen.insert(oui.clone()) {
                continue;
            }
            oui_counts.entry(oui, || (vendor, 0)).1 += 1;
        }
    }
    rows.extend(
        oui_counts
            .entries
            .into_iter()
            .map(|(oui, (vendor, count))| DeductionRow {
                kind: DeductionKind::Oui,
                observed: oui,
                deduction: vendor,
                count,
            }),
    );

    // Adresses notoires (IP puis MAC brute), comptées sur les endpoints.
    let mut addr_counts: OrderedTally<String, (&'static str, usize)> = OrderedTally::new();
    for endpoint in &endpoints {
        for (observed, deduction) in [
            (&endpoint.ip, known_ip(&endpoint.ip)),
            (&endpoint.mac, known_mac(&endpoint.mac)),
        ] {
            if observed.is_empty() || deduction.is_empty() {
                continue;
            }
            addr_counts.entry(observed.clone(), || (deduction, 0)).1 += 1;
        }
    }
    rows.extend(
        addr_counts
            .entries
            .into_iter()
            .map(|(observed, (deduction, count))| DeductionRow {
                kind: DeductionKind::KnownAddress,
                observed,
                deduction,
                count,
            }),
    );

    rows.sort_by(|a, b| {
        a.kind
            .order()
            .cmp(&b.kind.order())
            .then(b.count.cmp(&a.count))
            .then_with(|| compare_fr(&a.observed, &b.observed))
    });
    rows
}

/// Indicateurs synthétiques dérivés du snapshot. Les endpoints comptés sont
/// ceux effectivement reliés par un flux : sur un graphe filtré (cf.
/// [`filter_graph_by_protocols`]), un nœud sans flux visible n'est plus
/// compté.
pub fn compute_stats(graph: &GraphView<'_>) -> InvestigationStats {
    let mut active = HashSet::new();
    let mut protocols = HashSet::new();
    let (mut packets, mut bytes) = (0_u64, 0_u64);
    for &(_, edge) in graph.edges() {
        active.insert(edge.source.as_str());
        active.insert(edge.target.as_str());
        protocols.insert(edge_protocol(edge));
        packets = packets.saturating_add(edge.count);
        bytes = bytes.saturating_add(edge.total_bytes);
    }
    let by_id = index_nodes_by_id(graph);
    let duplicate_ips = active
        .iter()
        .filter(|id| by_id.get(*id).is_some_and(|node| node.duplicate_ip))
        .count();
    InvestigationStats {
        endpoints: active.len(),
        flows: graph.edges().len(),
        packets,
        bytes,
        protocols: protocols.len(),
        duplicate_ips,
    }
}

/// Un protocole du filtre et son nombre de flux.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProtocolCount {
    /// Protocole affiché (cf. [`edge_protocol`]).
    pub protocol: String,
    /// Flux de ce protocole.
    pub flows: usize,
}

/// Répartition des flux par protocole, triée du plus fréquent au moins
/// fréquent (à égalité, ordre alphabétique). Alimente les cases à cocher du
/// filtre, avec le nombre de flux par protocole.
pub fn protocol_breakdown(graph: &GraphView<'_>) -> Vec<ProtocolCount> {
    let mut counts: OrderedTally<&str, usize> = OrderedTally::new();
    for &(_, edge) in graph.edges() {
        *counts.entry(edge_protocol(edge), || 0) += 1;
    }
    let mut rows: Vec<ProtocolCount> = counts
        .entries
        .into_iter()
        .map(|(protocol, flows)| ProtocolCount {
            protocol: protocol.to_string(),
            flows,
        })
        .collect();
    rows.sort_by(|a, b| {
        b.flows
            .cmp(&a.flows)
            .then_with(|| compare_fr(&a.protocol, &b.protocol))
    });
    rows
}

/// Sous-graphe ne gardant que les flux dont le protocole est autorisé. Les
/// nœuds sont conservés tels quels : c'est aux vues de n'afficher que les
/// endpoints encore reliés (degré > 0). Ne copie aucun nœud ni arête.
pub fn filter_graph_by_protocols<'a>(
    graph: &GraphView<'a>,
    allowed: &HashSet<String>,
) -> GraphView<'a> {
    graph.retain_edges(|edge| allowed.contains(edge_protocol(edge)))
}

/// Flux, paquets et octets cumulés d'une part de la synthèse.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Totals {
    /// Flux.
    pub flows: usize,
    /// Paquets.
    pub packets: u64,
    /// Octets.
    pub bytes: u64,
}

impl Totals {
    /// Ajoute un flux et son trafic.
    fn add_edge(&mut self, edge: &Edge) {
        self.flows += 1;
        self.packets = self.packets.saturating_add(edge.count);
        self.bytes = self.bytes.saturating_add(edge.total_bytes);
    }
}

/// Une part de la répartition par protocole : flux, paquets et octets
/// cumulés.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProtocolShare {
    /// Protocole affiché.
    pub protocol: String,
    /// Flux.
    pub flows: usize,
    /// Paquets.
    pub packets: u64,
    /// Octets.
    pub bytes: u64,
}

/// Part des échanges selon la portée : « Internet » si une extrémité est une
/// IP publique (catégorie du nœud, fournie par le backend), « interne »
/// sinon. Un fait d'adressage, pas une interprétation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ScopeShares {
    /// Flux sans extrémité publique.
    pub internal: Totals,
    /// Flux dont une extrémité est une IP publique.
    pub internet: Totals,
}

/// Part d'un statut de chiffrement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConfidentialityShare {
    /// Statut.
    pub kind: Confidentiality,
    /// Flux, paquets et octets du statut.
    pub totals: Totals,
}

/// Part d'un usage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UsageShare {
    /// Usage.
    pub usage: Usage,
    /// Flux, paquets et octets de l'usage.
    pub totals: Totals,
}

/// Portée, chiffrement et usages d'un graphe (cf. [`build_trait_shares`]).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct TraitShares {
    /// Interne / Internet.
    pub scope: ScopeShares,
    /// Toujours les trois statuts, dans l'ordre Chiffré, En clair,
    /// Indéterminé.
    pub confidentiality: Vec<ConfidentialityShare>,
    /// Usages présents, par volume décroissant.
    pub usages: Vec<UsageShare>,
}

/// Synthèse « résumé » d'un relevé : répartition par protocole (par volume),
/// plus gros émetteurs, plus gros flux, nombre d'endpoints encore à
/// étiqueter, portée interne / Internet, et deux indications tirées de la
/// table fixe des protocoles ([`super::protocol_traits`]) : chiffrement et
/// usage. Aucune n'est déduite d'un port ni d'un volume.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct InvestigationSummary {
    /// Parts par protocole, par volume décroissant.
    pub protocols: Vec<ProtocolShare>,
    /// Endpoints les plus actifs (par volume).
    pub top_endpoints: Vec<EndpointRow>,
    /// Flux les plus volumineux.
    pub top_flows: Vec<FlowRow>,
    /// Endpoints reliés (degré > 0) sans label posé.
    pub unlabeled: usize,
    /// Interne / Internet.
    pub scope: ScopeShares,
    /// Toujours les trois statuts, dans l'ordre Chiffré, En clair,
    /// Indéterminé (vide seulement dans la synthèse vide du chargement).
    pub confidentiality: Vec<ConfidentialityShare>,
    /// Usages présents, par volume décroissant.
    pub usages: Vec<UsageShare>,
}

/// Catégorie d'adresse d'un nœud, telle que le backend la colore.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GraphNodeCategory {
    /// IP privée.
    Private,
    /// IP publique.
    Public,
    /// Multicast.
    Multicast,
    /// Boucle locale.
    Loopback,
    /// APIPA (169.254/16).
    Apipa,
    /// Lien-local.
    LinkLocal,
    /// ULA IPv6.
    Ula,
    /// Plage de documentation (et couleur par défaut du backend).
    Documentation,
    /// Diffusion limitée.
    Broadcast,
    /// Nœud de couche 2 (MAC seule).
    Layer2,
}

/// Couleur de chaque catégorie, observée en faisant passer chaque catégorie
/// par le graphe du cœur.
pub const GRAPH_NODE_CATEGORIES: [(GraphNodeCategory, &str); 10] = [
    (GraphNodeCategory::Private, "#8BC34A"),
    (GraphNodeCategory::Public, "#2196F3"),
    (GraphNodeCategory::Multicast, "#FFC107"),
    (GraphNodeCategory::Loopback, "#E53935"),
    (GraphNodeCategory::Apipa, "#FF9800"),
    (GraphNodeCategory::LinkLocal, "#FF5722"),
    (GraphNodeCategory::Ula, "#9C27B0"),
    (GraphNodeCategory::Documentation, "#9E9E9E"),
    (GraphNodeCategory::Broadcast, "#EC407A"),
    (GraphNodeCategory::Layer2, "#00BCD4"),
];

/// Catégorie d'adresse d'un nœud, lue dans sa couleur : le backend colore
/// chaque nœud selon le type de son IP (table générée et testée côté Rust).
pub fn node_category(node: Option<&Node>) -> Option<GraphNodeCategory> {
    let node = node?;
    GRAPH_NODE_CATEGORIES
        .iter()
        .find(|(_, color)| color.eq_ignore_ascii_case(&node.color))
        .map(|&(kind, _)| kind)
}

/// Ordre fixe des statuts de chiffrement.
const CONFIDENTIALITY_ORDER: [Confidentiality; 3] = [
    Confidentiality::Encrypted,
    Confidentiality::Cleartext,
    Confidentiality::Undetermined,
];

/// Portée, chiffrement et usages d'un graphe, en une passe sur les arêtes.
pub fn build_trait_shares(graph: &GraphView<'_>) -> TraitShares {
    let nodes = index_nodes_by_id(graph);
    let mut scope = ScopeShares::default();
    let mut confidentiality: HashMap<Confidentiality, Totals> = HashMap::new();
    let mut usages: OrderedTally<Usage, Totals> = OrderedTally::new();
    for &(_, edge) in graph.edges() {
        let internet = [&edge.source, &edge.target].iter().any(|id| {
            node_category(nodes.get(id.as_str()).copied()) == Some(GraphNodeCategory::Public)
        });
        if internet {
            &mut scope.internet
        } else {
            &mut scope.internal
        }
        .add_edge(edge);

        let protocol = edge_protocol(edge);
        confidentiality
            .entry(protocol_traits(protocol).confidentiality)
            .or_default()
            .add_edge(edge);
        usages
            .entry(usage_of(protocol), Totals::default)
            .add_edge(edge);
    }
    let mut usages: Vec<UsageShare> = usages
        .entries
        .into_iter()
        .map(|(usage, totals)| UsageShare { usage, totals })
        .collect();
    usages.sort_by(|a, b| {
        b.totals
            .bytes
            .cmp(&a.totals.bytes)
            .then_with(|| compare_fr(a.usage.label(), b.usage.label()))
    });
    TraitShares {
        scope,
        confidentiality: CONFIDENTIALITY_ORDER
            .iter()
            .map(|&kind| ConfidentialityShare {
                kind,
                totals: confidentiality.get(&kind).copied().unwrap_or_default(),
            })
            .collect(),
        usages,
    }
}

/// Tri stable par octets décroissants, puis par nom (ordre français).
fn sort_by_bytes_then_name<T>(
    rows: &mut [T],
    bytes: impl Fn(&T) -> u64,
    name: impl Fn(&T) -> &str,
) {
    rows.sort_by(|a, b| {
        bytes(b)
            .cmp(&bytes(a))
            .then_with(|| compare_fr(name(a), name(b)))
    });
}

/// Nombre de lignes des classements de la synthèse par défaut.
pub const DEFAULT_TOP_N: usize = 5;

/// Construit la synthèse du relevé. `top_n` borne les classements
/// ([`DEFAULT_TOP_N`] dans la vue).
pub fn build_summary(graph: &GraphView<'_>, top_n: usize) -> InvestigationSummary {
    let mut shares: OrderedTally<&str, ProtocolShare> = OrderedTally::new();
    for &(_, edge) in graph.edges() {
        let protocol = edge_protocol(edge);
        let share = shares.entry(protocol, || ProtocolShare {
            protocol: protocol.to_string(),
            flows: 0,
            packets: 0,
            bytes: 0,
        });
        share.flows += 1;
        share.packets = share.packets.saturating_add(edge.count);
        share.bytes = share.bytes.saturating_add(edge.total_bytes);
    }
    let mut protocols: Vec<ProtocolShare> =
        shares.entries.into_iter().map(|(_, share)| share).collect();
    sort_by_bytes_then_name(&mut protocols, |share| share.bytes, |share| &share.protocol);

    let endpoints: Vec<EndpointRow> = build_endpoint_rows(graph)
        .into_iter()
        .filter(|row| row.degree > 0)
        .collect();
    let unlabeled = endpoints.iter().filter(|row| row.label.is_empty()).count();

    let mut top_endpoints = endpoints;
    sort_by_bytes_then_name(&mut top_endpoints, |row| row.bytes, |row| &row.name);
    top_endpoints.truncate(top_n);

    let mut top_flows = build_flow_rows(graph);
    sort_by_bytes_then_name(&mut top_flows, |row| row.bytes, |row| &row.source);
    top_flows.truncate(top_n);

    let TraitShares {
        scope,
        confidentiality,
        usages,
    } = build_trait_shares(graph);
    InvestigationSummary {
        protocols,
        top_endpoints,
        top_flows,
        unlabeled,
        scope,
        confidentiality,
        usages,
    }
}

/// Unités de [`format_bytes`], en base 1024.
const BYTE_UNITS: [&str; 5] = ["o", "Kio", "Mio", "Gio", "Tio"];

/// Octets en unité lisible (base 1024), pour l'affichage des tableaux et
/// KPIs : « 512 o », « 1.5 Kio », « 120 Mio » (une décimale sous 100, point
/// décimal comme `toFixed`).
pub fn format_bytes(bytes: u64) -> String {
    if bytes == 0 {
        return "0 o".to_string();
    }
    // `Math.floor(Math.log(bytes) / Math.log(1024))`, calculé en entiers :
    // même résultat aux puissances exactes de 1024, sans
    // dépendre de la précision du logarithme.
    let mut exponent = 0_usize;
    let mut rest = bytes;
    while rest >= 1024 && exponent < BYTE_UNITS.len() - 1 {
        rest /= 1024;
        exponent += 1;
    }
    if exponent == 0 {
        return format!("{bytes} o");
    }
    // Conversion du nombre JavaScript ; diviser par une puissance de deux
    // est exact.
    let value = bytes as f64 / 1024_f64.powi(i32::try_from(exponent).unwrap_or(0));
    let rounded = to_fixed(value, if value >= 100.0 { 0 } else { 1 });
    format!("{rounded} {}", BYTE_UNITS[exponent])
}

/// Part d'un total, arrondie au dixième (« < 0,1 % » pour une part infime,
/// « 0 % » sans total), la même pour la vue et la page de synthèse.
pub fn percent_of(part: u64, total: u64) -> String {
    if total == 0 {
        return "0 %".to_string();
    }
    #[allow(clippy::cast_precision_loss)]
    let share = part as f64 / total as f64 * 100.0;
    if share > 0.0 && share < 0.1 {
        return "< 0,1 %".to_string();
    }
    format!("{} %", format_decimal(share, 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::investigation::test_support::{edge, graph, node, ports, s};
    use lunar_flows_core::graph::GraphData;

    /// Deux endpoints, deux flux : de quoi exercer degré, trafic et
    /// protocoles.
    fn sample() -> GraphData {
        graph(
            vec![
                (
                    "a",
                    Node {
                        id: s("a"),
                        ip: s("192.168.1.1"),
                        mac: s("aa:aa"),
                        label: Some(s("Automate")),
                        ..node()
                    },
                ),
                (
                    "b",
                    Node {
                        id: s("b"),
                        ip: s("192.168.1.2"),
                        mac: s("bb:bb"),
                        duplicate_ip: true,
                        ..node()
                    },
                ),
            ],
            vec![
                (
                    "e1",
                    Edge {
                        id: s("e1"),
                        label: s("Modbus"),
                        ports: ports(&[502]),
                        count: 10,
                        total_bytes: 1000,
                        bidir: true,
                        ..edge()
                    },
                ),
                (
                    "e2",
                    Edge {
                        id: s("e2"),
                        label: s("DNS"),
                        ports: ports(&[53]),
                        count: 5,
                        total_bytes: 200,
                        ..edge()
                    },
                ),
            ],
        )
    }

    fn allowed(protocols: &[&str]) -> HashSet<String> {
        protocols.iter().map(|protocol| s(protocol)).collect()
    }

    #[test]
    fn endpoint_name_priorite_label_ip_mac_id() {
        let full = Node {
            label: Some(s("L")),
            ip: s("1.1.1.1"),
            mac: s("m"),
            ..node()
        };
        assert_eq!(endpoint_name(Some(&full), "id"), "L");
        assert_eq!(
            endpoint_name(
                Some(&Node {
                    ip: s("1.1.1.1"),
                    mac: s("m"),
                    ..node()
                }),
                "id"
            ),
            "1.1.1.1"
        );
        assert_eq!(
            endpoint_name(
                Some(&Node {
                    mac: s("m"),
                    ..node()
                }),
                "id"
            ),
            "m"
        );
        assert_eq!(endpoint_name(None, "id"), "id");
    }

    #[test]
    fn summarize_ports_dynamiques_vide_tries() {
        assert_eq!(
            summarize_ports(&Edge {
                has_dynamic_ports: true,
                ports: ports(&[1]),
                ..edge()
            }),
            "dynamiques"
        );
        assert_eq!(
            summarize_ports(&Edge {
                ports: ports(&[]),
                ..edge()
            }),
            "—"
        );
        assert_eq!(
            summarize_ports(&Edge {
                ports: ports(&[443, 80]),
                ..edge()
            }),
            "80, 443"
        );
    }

    #[test]
    fn build_flow_rows_resout_les_noms_de_source_et_cible() {
        let sample = sample();
        let rows = build_flow_rows(&GraphView::new(&sample));
        assert_eq!(rows.len(), 2);
        let modbus = rows
            .iter()
            .find(|row| row.protocol == "Modbus")
            .expect("flux Modbus");
        assert_eq!(modbus.source, "Automate");
        assert_eq!(modbus.target, "192.168.1.2");
        assert_eq!(modbus.ports, "502");
        assert!(modbus.bidir);
    }

    #[test]
    fn build_flow_rows_build_endpoint_rows_resolution_via_node_id() {
        // Reproduit le modèle réel : la map `nodes` est indexée par la clé
        // d'identité (IP/MAC), tandis que les arêtes référencent `Node.id` (un
        // hash distinct). La résolution doit passer par l'id, pas par la clé
        // de map.
        let g = graph(
            vec![
                (
                    "192.168.1.1",
                    Node {
                        id: s("hashA"),
                        ip: s("192.168.1.1"),
                        label: Some(s("Automate")),
                        ..node()
                    },
                ),
                (
                    "192.168.1.2",
                    Node {
                        id: s("hashB"),
                        ip: s("192.168.1.2"),
                        ..node()
                    },
                ),
            ],
            vec![(
                "e",
                Edge {
                    id: s("e"),
                    source: s("hashA"),
                    target: s("hashB"),
                    label: s("Modbus"),
                    count: 4,
                    total_bytes: 400,
                    ..edge()
                },
            )],
        );
        let view = GraphView::new(&g);
        let flows = build_flow_rows(&view);
        assert_eq!(flows[0].source, "Automate");
        assert_eq!(flows[0].target, "192.168.1.2");

        let rows = build_endpoint_rows(&view);
        let a = rows
            .iter()
            .find(|row| row.id == "hashA")
            .expect("endpoint hashA");
        assert_eq!(a.degree, 1);
        assert_eq!(a.packets, 4);
        assert_eq!(a.bytes, 400);
        // Le comptage d'endpoints des KPIs doit correspondre (2, pas 0).
        assert_eq!(compute_stats(&view).endpoints, 2);
    }

    #[test]
    fn build_endpoint_rows_mac_display_masque_le_broadcast() {
        let g = graph(
            vec![
                // Hôte seulement vu comme cible ARP : mac brute = broadcast,
                // macs vide.
                (
                    "target",
                    Node {
                        id: s("t"),
                        ip: s("192.168.1.60"),
                        mac: s("ff:ff:ff:ff:ff:ff"),
                        macs: vec![],
                        ..node()
                    },
                ),
                // Hôte vu émettre : vraie MAC unicast.
                (
                    "real",
                    Node {
                        id: s("r"),
                        ip: s("192.168.1.1"),
                        mac: s("e0:d3:62:6a:a2:5a"),
                        macs: vec![s("e0:d3:62:6a:a2:5a")],
                        ..node()
                    },
                ),
            ],
            vec![(
                "e",
                Edge {
                    source: s("t"),
                    target: s("r"),
                    ..edge()
                },
            )],
        );
        let rows = build_endpoint_rows(&GraphView::new(&g));
        let t = rows.iter().find(|row| row.id == "t").expect("endpoint t");
        let r = rows.iter().find(|row| row.id == "r").expect("endpoint r");
        assert_eq!(t.mac_display, ""); // affiché « — »
        assert_eq!(t.mac, "ff:ff:ff:ff:ff:ff"); // brute conservée (clé de label)
        assert_eq!(r.mac_display, "e0:d3:62:6a:a2:5a");
    }

    #[test]
    fn build_endpoint_rows_degre_et_trafic_derives_des_aretes() {
        let sample = sample();
        let rows = build_endpoint_rows(&GraphView::new(&sample));
        let a = rows.iter().find(|row| row.id == "a").expect("endpoint a");
        assert_eq!(a.degree, 2);
        assert_eq!(a.packets, 15);
        assert_eq!(a.bytes, 1200);
        let b = rows.iter().find(|row| row.id == "b").expect("endpoint b");
        assert!(b.duplicate_ip);
    }

    #[test]
    fn compute_stats_endpoints_flux_protocoles_distincts_ip_dupliquees() {
        let sample = sample();
        assert_eq!(
            compute_stats(&GraphView::new(&sample)),
            InvestigationStats {
                endpoints: 2,
                flows: 2,
                packets: 15,
                bytes: 1200,
                protocols: 2,
                duplicate_ips: 1
            }
        );
    }

    #[test]
    fn compute_stats_releve_vide() {
        let empty = GraphData::new();
        assert_eq!(
            compute_stats(&GraphView::new(&empty)),
            InvestigationStats::default()
        );
    }

    #[test]
    fn edge_protocol_label_ou_tiret_si_absent() {
        assert_eq!(
            edge_protocol(&Edge {
                label: s("Modbus"),
                ..edge()
            }),
            "Modbus"
        );
        assert_eq!(
            edge_protocol(&Edge {
                label: s(""),
                ..edge()
            }),
            "—"
        );
    }

    #[test]
    fn protocol_breakdown_trie_par_nombre_de_flux_decroissant() {
        let g = graph(
            vec![],
            vec![
                (
                    "e1",
                    Edge {
                        label: s("ARP"),
                        ..edge()
                    },
                ),
                (
                    "e2",
                    Edge {
                        label: s("ARP"),
                        ..edge()
                    },
                ),
                (
                    "e3",
                    Edge {
                        label: s("Modbus"),
                        ..edge()
                    },
                ),
            ],
        );
        assert_eq!(
            protocol_breakdown(&GraphView::new(&g)),
            vec![
                ProtocolCount {
                    protocol: s("ARP"),
                    flows: 2
                },
                ProtocolCount {
                    protocol: s("Modbus"),
                    flows: 1
                }
            ]
        );
    }

    #[test]
    fn filter_graph_by_protocols_ne_garde_que_les_protocoles_autorises() {
        let sample = sample();
        let filtered = filter_graph_by_protocols(&GraphView::new(&sample), &allowed(&["Modbus"]));
        let edges: Vec<_> = filtered.edges().iter().map(|(key, _)| *key).collect();
        assert_eq!(edges, vec!["e1"]);
        // Les nœuds sont conservés ; c'est la vue qui masque les endpoints
        // isolés.
        let nodes: Vec<_> = filtered.nodes().iter().map(|(key, _)| *key).collect();
        assert_eq!(nodes, vec!["a", "b"]);
    }

    #[test]
    fn compute_stats_suit_le_filtre() {
        // En ne gardant que DNS, seul le flux e2 (a↔b) subsiste : 1 flux, 2
        // endpoints encore reliés, ses paquets/octets, 1 protocole.
        let sample = sample();
        let filtered = filter_graph_by_protocols(&GraphView::new(&sample), &allowed(&["DNS"]));
        assert_eq!(
            compute_stats(&filtered),
            InvestigationStats {
                endpoints: 2,
                flows: 1,
                packets: 5,
                bytes: 200,
                protocols: 1,
                duplicate_ips: 1
            }
        );
    }

    #[test]
    fn compute_stats_aucun_protocole_autorise_releve_vide() {
        let sample = sample();
        let filtered = filter_graph_by_protocols(&GraphView::new(&sample), &HashSet::new());
        assert_eq!(compute_stats(&filtered).endpoints, 0);
        assert_eq!(compute_stats(&filtered).flows, 0);
    }

    #[test]
    fn build_summary_protocoles_par_volume_tops_bornes_a_etiqueter() {
        let g = graph(
            vec![
                (
                    "1",
                    Node {
                        id: s("a"),
                        ip: s("10.0.0.1"),
                        label: Some(s("Auto")),
                        ..node()
                    },
                ),
                (
                    "2",
                    Node {
                        id: s("b"),
                        ip: s("10.0.0.2"),
                        ..node()
                    },
                ),
                (
                    "3",
                    Node {
                        id: s("c"),
                        ip: s("10.0.0.3"),
                        ..node()
                    },
                ),
            ],
            vec![
                (
                    "e1",
                    Edge {
                        id: s("e1"),
                        label: s("Modbus"),
                        count: 10,
                        total_bytes: 1000,
                        ..edge()
                    },
                ),
                (
                    "e2",
                    Edge {
                        id: s("e2"),
                        target: s("c"),
                        label: s("DNS"),
                        count: 2,
                        total_bytes: 50,
                        ..edge()
                    },
                ),
                (
                    "e3",
                    Edge {
                        id: s("e3"),
                        source: s("b"),
                        target: s("c"),
                        label: s("Modbus"),
                        count: 1,
                        total_bytes: 20,
                        ..edge()
                    },
                ),
            ],
        );
        let summary = build_summary(&GraphView::new(&g), 2);
        let protocols: Vec<_> = summary
            .protocols
            .iter()
            .map(|share| share.protocol.as_str())
            .collect();
        assert_eq!(protocols, vec!["Modbus", "DNS"]);
        assert_eq!(
            summary.protocols[0],
            ProtocolShare {
                protocol: s("Modbus"),
                flows: 2,
                packets: 11,
                bytes: 1020
            }
        );
        assert_eq!(summary.top_endpoints.len(), 2);
        assert_eq!(summary.top_endpoints[0].id, "a"); // 1050 octets incidents
        assert_eq!(summary.top_flows.len(), 2);
        assert_eq!(summary.top_flows[0].id, "e1");
        assert_eq!(summary.unlabeled, 2); // b et c sans label ; a étiqueté
    }

    #[test]
    fn build_port_rows_agrege_les_ports_service_service_probable_tri() {
        let g = graph(
            vec![],
            vec![
                (
                    "e1",
                    Edge {
                        id: s("e1"),
                        ports: ports(&[502]),
                        count: 10,
                        total_bytes: 1000,
                        ..edge()
                    },
                ),
                (
                    "e2",
                    Edge {
                        id: s("e2"),
                        ports: ports(&[502]),
                        count: 5,
                        total_bytes: 500,
                        ..edge()
                    },
                ),
                (
                    "e3",
                    Edge {
                        id: s("e3"),
                        ports: ports(&[53, 138]),
                        count: 2,
                        total_bytes: 200,
                        ..edge()
                    },
                ),
                (
                    "arp",
                    Edge {
                        id: s("arp"),
                        ports: ports(&[]),
                        count: 1,
                        total_bytes: 60,
                        ..edge()
                    },
                ),
            ],
        );
        let rows = build_port_rows(&GraphView::new(&g));
        // 502 en tête (2 flux), puis 53 et 138 (1 flux) triés par numéro.
        assert_eq!(
            rows.iter().map(|row| row.port).collect::<Vec<_>>(),
            vec![502, 53, 138]
        );
        assert_eq!(
            rows[0],
            PortRow {
                port: 502,
                service: "Modbus/TCP",
                flows: 2,
                packets: 15,
                bytes: 1500
            }
        );
        assert_eq!(
            rows.iter()
                .find(|row| row.port == 138)
                .map(|row| row.service),
            Some("NetBIOS (datagramme)")
        );
    }

    #[test]
    fn build_deduction_rows_ports_oui_et_adresses_connues_reunis() {
        let g = graph(
            vec![
                (
                    "pi",
                    Node {
                        id: s("pi"),
                        ip: s("192.168.1.50"),
                        mac: s("b8:27:eb:00:00:01"),
                        macs: vec![s("b8:27:eb:00:00:01")],
                        ..node()
                    },
                ),
                (
                    "dns",
                    Node {
                        id: s("dns"),
                        ip: s("8.8.8.8"),
                        mac: s("e0:d3:62:00:00:01"),
                        macs: vec![s("e0:d3:62:00:00:01")],
                        ..node()
                    },
                ),
                (
                    "bcast",
                    Node {
                        id: s("bc"),
                        ip: s("192.168.1.60"),
                        mac: s("ff:ff:ff:ff:ff:ff"),
                        macs: vec![],
                        ..node()
                    },
                ),
            ],
            vec![
                (
                    "e1",
                    Edge {
                        id: s("e1"),
                        source: s("pi"),
                        target: s("dns"),
                        ports: ports(&[53]),
                        count: 3,
                        ..edge()
                    },
                ),
                (
                    "e2",
                    Edge {
                        id: s("e2"),
                        source: s("pi"),
                        target: s("bc"),
                        ports: ports(&[]),
                        count: 1,
                        ..edge()
                    },
                ),
            ],
        );
        let rows = build_deduction_rows(&GraphView::new(&g));
        let find = |predicate: &dyn Fn(&DeductionRow) -> bool| {
            rows.iter().find(|row| predicate(row)).cloned()
        };
        assert_eq!(
            find(&|row| row.kind == DeductionKind::Port).map(|row| row.deduction),
            Some("DNS")
        );
        assert_eq!(
            find(&|row| row.deduction == "Raspberry Pi").map(|row| row.observed),
            Some(s("B8:27:EB"))
        );
        assert_eq!(
            find(&|row| row.observed == "8.8.8.8").map(|row| row.deduction),
            Some("Google Public DNS")
        );
        assert_eq!(
            find(&|row| row.observed == "ff:ff:ff:ff:ff:ff").map(|row| row.deduction),
            Some("Broadcast Ethernet")
        );
    }

    #[test]
    fn observed_glossary_tokens_protocoles_des_flux_vlan_tunnel() {
        let g = graph(
            vec![
                (
                    "a",
                    Node {
                        id: s("a"),
                        vlan_id: Some(10),
                        ..node()
                    },
                ),
                (
                    "b",
                    Node {
                        id: s("b"),
                        ..node()
                    },
                ),
            ],
            vec![
                (
                    "e1",
                    Edge {
                        id: s("e1"),
                        label: s("TLS"),
                        ..edge()
                    },
                ),
                (
                    "e2",
                    Edge {
                        id: s("e2"),
                        label: s("Modbus/TCP"),
                        encap_ids: vec![s("abc")],
                        ..edge()
                    },
                ),
            ],
        );
        let tokens = observed_glossary_tokens(&GraphView::new(&g));
        assert!(tokens.contains("tls"));
        assert!(tokens.contains("modbustcp"));
        assert!(tokens.contains("vlan")); // nœud a a un vlan_id
        assert!(tokens.contains("tunnel")); // e2 a un encapId
        assert!(!tokens.contains(""));
    }

    #[test]
    fn format_bytes_unites_base_1024() {
        assert_eq!(format_bytes(0), "0 o");
        assert_eq!(format_bytes(512), "512 o");
        assert_eq!(format_bytes(1024), "1.0 Kio");
        assert_eq!(format_bytes(1536), "1.5 Kio");
        assert_eq!(format_bytes(1_048_576), "1.0 Mio");
    }

    #[test]
    fn build_endpoint_rows_flux_sortants_et_entrants_par_endpoint() {
        let g = graph(
            vec![
                (
                    "a",
                    Node {
                        id: s("a"),
                        ip: s("10.0.0.1"),
                        ..node()
                    },
                ),
                (
                    "b",
                    Node {
                        id: s("b"),
                        ip: s("10.0.0.2"),
                        ..node()
                    },
                ),
                (
                    "c",
                    Node {
                        id: s("c"),
                        ip: s("10.0.0.3"),
                        ..node()
                    },
                ),
            ],
            vec![
                // a -> b à sens unique ; b <-> c bidirectionnel.
                (
                    "e1",
                    Edge {
                        id: s("e1"),
                        bidir: false,
                        ..edge()
                    },
                ),
                (
                    "e2",
                    Edge {
                        id: s("e2"),
                        source: s("b"),
                        target: s("c"),
                        bidir: true,
                        ..edge()
                    },
                ),
            ],
        );
        let rows: HashMap<String, EndpointRow> = build_endpoint_rows(&GraphView::new(&g))
            .into_iter()
            .map(|row| (row.id.clone(), row))
            .collect();
        let flows = |id: &str| (rows[id].flows_out, rows[id].flows_in);
        assert_eq!(flows("a"), (1, 0));
        // b reçoit de a, et échange dans les deux sens avec c.
        assert_eq!(flows("b"), (1, 2));
        assert_eq!(flows("c"), (1, 1));
        // Le degré reste le nombre de flux incidents, sans double compte.
        assert_eq!(rows["b"].degree, 2);
    }

    #[test]
    fn percent_of_dixieme_et_part_infime() {
        assert_eq!(percent_of(1, 0), "0 %");
        assert_eq!(percent_of(0, 10), "0 %");
        assert_eq!(percent_of(1, 10_000), "< 0,1 %");
        assert_eq!(percent_of(1, 3), "33,3 %");
        assert_eq!(percent_of(10, 10), "100 %");
    }
}
