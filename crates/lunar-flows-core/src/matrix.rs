//! Matrice de flux : agrégat central de LUNAR. Chaque flux observé (couple
//! MAC/IP/ports/protocoles) y accumule ses statistiques, ventilées par tunnel
//! (`encap_id`), avec résolution de labels par clé `(mac, ip)` et export /
//! réimport CSV sans perte.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs::File;
use std::net::IpAddr;
use std::time::SystemTime;

use log::info;
use packet_parser::owned::{ApplicationOwned, InternetOwned, PacketFlowOwned, TransportOwned};
use packet_parser::parse::data_link::{ethertype::Ethertype, vlan_tag::VlanTag};
use packet_parser::{IpType, LinkType};

use crate::link::{
    LinkView, into_matrix_flow_identity, matrix_flow_identity, matrix_link_from_text,
};
use serde::{Deserialize, Serialize};

/// Vrai pour les IP « non spécifiées »/broadcast qui ne désignent pas un
/// équipement précis : `0.0.0.0`, `::`, `255.255.255.255`. Elles peuvent être
/// partagées par plusieurs machines et ne doivent donc pas servir de clé de
/// label ni déclencher de conflit.
pub fn is_placeholder_ip(ip: &str) -> bool {
    matches!(ip.trim(), "0.0.0.0" | "::" | "255.255.255.255")
}

/// Vrai pour une MAC de broadcast (`ff:ff:ff:ff:ff:ff`) ou multicast : le bit
/// de poids faible du premier octet vaut 1 (ex. `01:00:5e:*`, `33:33:*`). Ces
/// adresses ne sont pas l'identité d'un équipement.
pub fn is_non_unicast_mac(mac: &str) -> bool {
    mac.split(':')
        .next()
        .and_then(|octet| u8::from_str_radix(octet.trim(), 16).ok())
        .is_some_and(|first| first & 0x01 == 1)
}

/// Vrai pour une MAC qui n'identifie aucun équipement : absente, nulle, ou
/// de groupe (diffusion, multicast). C'est le cas de la cible d'une requête
/// ARP, envoyée en diffusion : seule son IP la désigne.
fn is_unidentified_mac(mac: &str) -> bool {
    let mac = mac.trim();
    mac.is_empty() || mac == "00:00:00:00:00:00" || is_non_unicast_mac(mac)
}

/// Nombre d'IP unicast distinctes (d'une même famille) à partir duquel une
/// MAC est considérée comme **passerelle probable** : routeur, pare-feu,
/// VRRP, NAT… Un poste ordinaire porte une IPv4 et une poignée d'IPv6 ;
/// une passerelle est vue avec les adresses de tout ce qu'elle route. Au-delà
/// de ce seuil, un label posé sur la MAC seule ne s'applique plus aux IP
/// qu'elle porte : il renommerait à tort tout le trafic routé.
pub const GATEWAY_IP_THRESHOLD: usize = 4;

/// IP qui compte comme identité d'un équipement pour le seuil passerelle :
/// unicast, hors non spécifiée, boucle locale, multicast, broadcast et
/// lien-local (fe80::/10, 169.254/16 — plusieurs par hôte, non routées).
pub fn is_countable_unicast_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            !v4.is_unspecified()
                && !v4.is_loopback()
                && !v4.is_multicast()
                && !v4.is_broadcast()
                && !v4.is_link_local()
        }
        IpAddr::V6(v6) => {
            !v6.is_unspecified()
                && !v6.is_loopback()
                && !v6.is_multicast()
                && !v6.is_unicast_link_local()
        }
    }
}

/// Compteurs d'un flux pour un tunnel donné : une entrée par couple
/// (flux, tunnel) dans [`FlowMatrix::matrix`], refusionnées en une
/// seule ligne à l'export.
#[derive(Debug, Clone, Serialize)]
pub struct FlowStats {
    /// Nombre de paquets vus pour ce flux.
    pub count: u64,
    /// Somme des longueurs sur le fil (`len`, pas `caplen`) des paquets
    /// du flux.
    pub total_bytes: u64,
    /// Horodatage du paquet le plus récent : un maximum, jamais la
    /// dernière valeur lue, pour que des PCAP importés hors ordre ne le
    /// fassent pas régresser.
    pub last_seen: SystemTime,
}

/// Flux reconstruit depuis une ligne LFMS et statistiques ventilées par tunnel.
pub type ReconstructedFlowRows = (PacketFlowOwned, Vec<(Option<u64>, FlowStats)>);

/// Sérialise la colonne `encap_id` d'une ligne de matrice :
/// - `""` : flux jamais vu dans un tunnel ;
/// - `id` : un seul tunnel, qui porte tous les paquets de la ligne ;
/// - `id:n|id:n|…` : comptes par tunnel (flux répliqué dans plusieurs
///   tunnels, ou partiellement hors tunnel). La somme des `n` d'un tunnel
///   sur les lignes filles égale le compteur de sa ligne externe.
pub fn format_encap_list(entries: &[(Option<u64>, FlowStats)]) -> String {
    let tunnels: Vec<(u64, u64)> = entries
        .iter()
        .filter_map(|(id, stats)| id.map(|id| (id, stats.count)))
        .collect();

    match tunnels.as_slice() {
        [] => String::new(),
        [(id, _)] if entries.len() == 1 => format!("{id:016x}"),
        _ => tunnels
            .iter()
            .map(|(id, count)| format!("{id:016x}:{count}"))
            .collect::<Vec<_>>()
            .join("|"),
    }
}

/// Chemin inverse de [`format_encap_list`] : redistribue le compteur d'une
/// ligne de CSV entre ses tunnels. Les paquets non couverts par la liste
/// (part hors tunnel) vont sur une entrée `None`. Les octets ne sont pas
/// ventilés par tunnel dans le CSV : ils sont portés par la première entrée,
/// l'export refusionnant de toute façon les entrées d'un même flux.
pub fn parse_encap_list(
    encap: &str,
    count: u64,
    total_bytes: u64,
    last_seen: SystemTime,
) -> Vec<(Option<u64>, FlowStats)> {
    let mut counts: Vec<(Option<u64>, u64)> = Vec::new();

    for element in encap.split('|').map(str::trim).filter(|e| !e.is_empty()) {
        let (id, tunnel_count) = match element.split_once(':') {
            Some((id, n)) => (id, n.trim().parse::<u64>().unwrap_or(0)),
            // `id` sans `:n` : le tunnel porte tous les paquets de la ligne.
            None => (element, count),
        };
        if let Ok(id) = u64::from_str_radix(id.trim(), 16) {
            counts.push((Some(id), tunnel_count));
        }
    }

    let in_tunnels: u64 = counts.iter().map(|(_, n)| *n).sum();
    if counts.is_empty() || in_tunnels < count {
        counts.push((None, count.saturating_sub(in_tunnels)));
    }

    counts
        .into_iter()
        .enumerate()
        .map(|(i, (id, n))| {
            (
                id,
                FlowStats {
                    count: n,
                    total_bytes: if i == 0 { total_bytes } else { 0 },
                    last_seen,
                },
            )
        })
        .collect()
}

/// Séparateur des noms de fichiers dans la colonne `origin` d'une ligne de
/// matrice. Un flux présent dans plusieurs fichiers importés porte tous leurs
/// noms, joints par ce caractère (ex. `site-a.csv|site-b.csv`).
pub const ORIGIN_SEP: char = '|';

/// Découpe une valeur de colonne `origin` en noms de fichiers individuels
/// (vides ignorés).
pub fn parse_origin_list(origin: &str) -> impl Iterator<Item = String> + '_ {
    origin
        .split(ORIGIN_SEP)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
}

/// Matrice de flux : statistiques par flux (ventilées par tunnel), labels
/// résolus par clé `(mac, ip)` et provenance (fichiers d'origine) par flux.
pub struct FlowMatrix {
    /// Une entrée de statistiques par couple (flux, tunnel encapsulant), pour
    /// que la somme des compteurs internes d'un tunnel soit égale au compteur
    /// de sa ligne externe. `None` = paquets vus hors tunnel. À l'export, un
    /// flux redonne UNE ligne : ses tunnels sont sérialisés dans la colonne
    /// `encap_id` (voir `format_encap_list`).
    pub matrix: HashMap<PacketFlowOwned, Vec<(Option<u64>, FlowStats)>>,
    /// Labels par clé `(mac, ip)`. Une partie vide fait un label par IP
    /// seule ou par MAC seule ; la précédence entre les trois formes est
    /// celle de [`FlowMatrix::get_label`].
    pub label: HashMap<(String, String), String>,
    /// Fichier(s) d'origine par flux, alimenté à l'import de matrices CSV. Un
    /// flux fusionné depuis plusieurs fichiers accumule tous leurs noms. Vide
    /// pour les flux issus d'une capture live ou d'un import PCAP. Sérialisé
    /// dans la colonne `origin` par `to_flat_vec`.
    pub origins: HashMap<PacketFlowOwned, BTreeSet<String>>,
    /// Type de liaison du relevé, fixé par la première source (capture,
    /// fichier PCAP ou préambule #LFMS d'une matrice importée) via
    /// `bind_link_type`. Un relevé = un réseau = un DLT :
    /// toute source d'un autre DLT est refusée. `None` = matrice vierge.
    pub link_type: Option<packet_parser::LinkType>,
    /// Contexte du relevé (site, capteur, interface), déclaré par l'opérateur
    /// à l'arrêt de la capture ou à l'enregistrement. Écrit
    /// dans le préambule #LFMS à l'export ; volontairement hors de la clé des
    /// flux — le même paquet peut être vu par plusieurs capteurs.
    pub context: crate::lfms::SurveyContext,
    /// IP unicast distinctes observées par MAC unicast (source ou destination),
    /// maintenu à chaque nouveau flux : seuil passerelle de `get_label`
    /// sans balayage de la matrice à chaque résolution de label.
    unicast_ips_by_mac: HashMap<String, HashSet<IpAddr>>,
    /// Période observée du relevé : premier et dernier paquet vus,
    /// ou inconnue si une matrice importée ne la déclarait pas. Écrite dans
    /// le préambule #LFMS (`first_seen=` / `last_seen=`).
    pub period: crate::lfms::SurveyPeriod,
}

impl Default for FlowMatrix {
    fn default() -> Self {
        Self::new()
    }
}

impl FlowMatrix {
    /// Matrice vierge : aucun flux ni label, type de liaison indéterminé
    /// (fixé par la première source via
    /// [`bind_link_type`](Self::bind_link_type)), contexte par défaut et
    /// période vide.
    pub fn new() -> Self {
        Self {
            matrix: HashMap::new(),
            label: HashMap::new(),
            origins: HashMap::new(),
            link_type: None,
            context: crate::lfms::SurveyContext::default(),
            unicast_ips_by_mac: HashMap::new(),
            period: crate::lfms::SurveyPeriod::Empty,
        }
    }

    /// Fixe le type de liaison du relevé, ou vérifie qu'une nouvelle source
    /// (capture, PCAP, matrice importée) porte bien le même : une fusion ne
    /// concerne que des relevés du même réseau, donc du même DLT.
    /// `origin` identifie la source refusée dans l'erreur.
    pub fn bind_link_type(
        &mut self,
        link_type: packet_parser::LinkType,
        origin: &std::path::Path,
    ) -> crate::Result<()> {
        match self.link_type {
            None => {
                self.link_type = Some(link_type);
                Ok(())
            }
            Some(existing) if existing == link_type => Ok(()),
            Some(existing) => Err(crate::LunarCoreError::MixedLinkTypes {
                path: origin.to_path_buf(),
                found: crate::lfms::link_type_name(link_type),
                expected: crate::lfms::link_type_name(existing),
            }),
        }
    }

    /// Nombre de lignes de la matrice exportée (une par flux distinct).
    pub fn row_count(&self) -> usize {
        self.matrix.len()
    }

    /// Comptabilise un paquet (capture live ou PCAP) : incrémente l'entrée
    /// (flux, tunnel) existante ou la crée, étend la période du relevé et,
    /// pour un flux nouveau, alimente l'index du seuil passerelle. La clé
    /// est l'identité canonique du flux (`matrix_flow_identity`) : pour
    /// SLL/SLL2, les métadonnées du point d'observation n'en font pas
    /// partie.
    pub fn update_flow(&mut self, pkt: &CapturedPacketOwned) {
        let ts = timeval_to_systemtime(pkt.ts_sec, pkt.ts_usec);
        self.period.observe(ts);
        let flow = matrix_flow_identity(&pkt.flow);

        // Ethernet/RAW restent recherchés par référence. Pour SLL/SLL2, la
        // clé canonique ignore les seules métadonnées du point d'observation.
        if let Some(entries) = self.matrix.get_mut(flow.as_ref()) {
            if let Some((_, stats)) = entries.iter_mut().find(|(id, _)| *id == pkt.encap_id) {
                stats.count += 1;
                stats.total_bytes = stats.total_bytes.saturating_add(pkt.len as u64);
                // Max et non dernière valeur : des PCAP importés hors ordre
                // chronologique ne font pas régresser la date.
                if ts > stats.last_seen {
                    stats.last_seen = ts;
                }
            } else {
                entries.push((
                    pkt.encap_id,
                    FlowStats {
                        count: 1,
                        total_bytes: pkt.len as u64,
                        last_seen: ts,
                    },
                ));
            }
        } else {
            let flow = flow.into_owned();
            self.index_flow_endpoints(&flow);
            self.matrix.insert(
                flow,
                vec![(
                    pkt.encap_id,
                    FlowStats {
                        count: 1,
                        total_bytes: pkt.len as u64,
                        last_seen: ts,
                    },
                )],
            );
        }
    }

    /// Enregistre les couples (MAC unicast, IP unicast) d'un flux dans l'index
    /// du seuil passerelle. Appelé une fois par flux, à sa création.
    fn index_flow_endpoints(&mut self, flow: &PacketFlowOwned) {
        let Some(internet) = flow.internet.as_ref() else {
            return;
        };
        let link = LinkView::of(&flow.data_link);
        for (mac, ip) in [
            (&link.source_mac, internet.source_ip),
            (&link.destination_mac, internet.destination_ip),
        ] {
            if let Some(ip) = ip
                && !mac.is_empty()
                && !is_non_unicast_mac(mac)
                && is_countable_unicast_ip(&ip)
            {
                self.unicast_ips_by_mac
                    .entry(mac.clone())
                    .or_default()
                    .insert(ip);
            }
        }
    }

    /// Nombre d'IP unicast distinctes portées par `mac` dans la famille la
    /// plus peuplée (IPv4 ou IPv6, comptées séparément : un hôte double pile
    /// ne franchit pas le seuil par simple addition de ses deux familles).
    pub fn gateway_ip_count(&self, mac: &str) -> usize {
        self.unicast_ips_by_mac
            .get(mac)
            .map(|ips| {
                let v4 = ips.iter().filter(|ip| ip.is_ipv4()).count();
                v4.max(ips.len() - v4)
            })
            .unwrap_or(0)
    }

    /// Vrai quand `mac` porte au moins [`GATEWAY_IP_THRESHOLD`] IP unicast
    /// distinctes d'une même famille : passerelle probable.
    pub fn is_gateway_mac(&self, mac: &str) -> bool {
        self.gateway_ip_count(mac) >= GATEWAY_IP_THRESHOLD
    }

    /// Fusionne une ligne déjà agrégée (import de matrice CSV) : cumule les
    /// compteurs si la ligne (flux, tunnel) existe, la crée sinon.
    pub fn merge_row(&mut self, flow: PacketFlowOwned, encap_id: Option<u64>, stats: FlowStats) {
        let flow = into_matrix_flow_identity(flow);
        if !self.matrix.contains_key(&flow) {
            self.index_flow_endpoints(&flow);
        }
        let entries = self.matrix.entry(flow).or_default();
        if let Some((_, existing)) = entries.iter_mut().find(|(id, _)| *id == encap_id) {
            existing.count += stats.count;
            existing.total_bytes = existing.total_bytes.saturating_add(stats.total_bytes);
            if stats.last_seen > existing.last_seen {
                existing.last_seen = stats.last_seen;
            }
        } else {
            entries.push((encap_id, stats));
        }
    }

    /// Enregistre le(s) fichier(s) d'origine d'un flux (import de matrice CSV).
    /// Les noms s'ajoutent à ceux déjà connus pour ce flux : un même flux vu
    /// dans deux fichiers finit avec les deux noms. Les noms vides sont ignorés
    /// et n'entraînent aucune allocation.
    pub fn add_flow_origins<I>(&mut self, flow: &PacketFlowOwned, origins: I)
    where
        I: IntoIterator<Item = String>,
    {
        let mut origins = origins
            .into_iter()
            .filter(|name| !name.trim().is_empty())
            .peekable();
        if origins.peek().is_none() {
            return;
        }
        let flow = matrix_flow_identity(flow);
        let set = self.origins.entry(flow.into_owned()).or_default();
        set.extend(origins);
    }

    /// Vide la matrice : flux, labels, provenances, index passerelle, type
    /// de liaison et période repartent de zéro. Le contexte du relevé est
    /// conservé.
    pub fn clear(&mut self) {
        self.matrix.clear();
        self.label.clear();
        self.origins.clear();
        self.link_type = None;
        self.unicast_ips_by_mac.clear();
        self.period = crate::lfms::SurveyPeriod::Empty;
    }

    // pub fn print(&self) {
    //     // En-tête
    //     println!(
    //         "{:<4} {:<30} {:<10} {:<12} {:<24}",
    //         "#", "FLOW", "COUNT", "BYTES", "LAST SEEN"
    //     );
    //     println!(
    //         "{:<4} {:<30} {:<10} {:<12} {:<24}",
    //         "-",
    //         "------------------------------",
    //         "----------",
    //         "------------",
    //         "------------------------"
    //     );

    //     let mut count_p = 0;
    //     for (flow, stats) in &self.matrix {
    //         count_p += 1;
    //         // Formatage de la date (timestamp en secondes)
    //         let last_seen = match stats.last_seen.duration_since(std::time::UNIX_EPOCH) {
    //             Ok(dur) => {
    //                 let dt = chrono::NaiveDateTime::from_timestamp_opt(dur.as_secs() as i64, 0)
    //                     .unwrap_or_default();
    //                 dt.format("%Y-%m-%d %H:%M:%S").to_string()
    //             }
    //             Err(_) => "N/A".to_string(),
    //         };

    //         println!(
    //             "{:<4} {:<30} {:<10} {:<12} {:<24}",
    //             count_p,
    //             format!("{}", flow),
    //             stats.count,
    //             stats.total_bytes,
    //             last_seen
    //         );
    //     }
    //     println!("count : {}", count_p);
    // }

    /// Aplatit la matrice en lignes de matrice (format CSV/LFMS) : une
    /// ligne par flux, compteurs par tunnel refusionnés (la ventilation
    /// reste lisible dans `encap_id`), labels résolus par
    /// [`get_label`](Self::get_label) et échappés contre l'injection de
    /// formule, `last_seen` en UTC à la microseconde. Les lignes sont
    /// triées sur tous les champs d'identité : deux exports du même état
    /// donnent le même fichier.
    pub fn to_flat_vec(&self) -> Vec<FlowMatrixRow> {
        let mut rows: Vec<FlowMatrixRow> = self
            .matrix
            .iter()
            .map(|(flow, entries)| {
                // Une seule ligne par flux : les compteurs par tunnel sont
                // refusionnés, la ventilation reste lisible dans `encap_id`.
                let stats = FlowStats {
                    count: entries.iter().map(|(_, s)| s.count).sum(),
                    total_bytes: entries
                        .iter()
                        .fold(0u64, |acc, (_, s)| acc.saturating_add(s.total_bytes)),
                    last_seen: entries
                        .iter()
                        .map(|(_, s)| s.last_seen)
                        .max()
                        .unwrap_or(UNIX_EPOCH),
                };
                let encap_id = format_encap_list(entries);
                // Provenance : noms de fichiers d'origine du flux, triés
                // (BTreeSet) et joints, ou vide hors import de matrice CSV.
                let origin = self
                    .origins
                    .get(flow)
                    .map(|names| {
                        names
                            .iter()
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(&ORIGIN_SEP.to_string())
                    })
                    .unwrap_or_default();
                (flow, encap_id, origin, stats)
            })
            .map(|(flow, encap_id, origin, stats)| {
                let link = LinkView::of(&flow.data_link);
                let ip_source = flow
                    .internet
                    .as_ref()
                    .and_then(|i| i.source_ip)
                    .map(|ip| ip.to_string())
                    .unwrap_or_default();
                let ip_source_type = flow
                    .internet
                    .as_ref()
                    .and_then(|i| i.ip_source_type.clone())
                    .map(|ip| ip.to_string())
                    .unwrap_or_default();
                let label_source = self.get_label(&link.source_mac, &ip_source);

                let ip_destination = flow
                    .internet
                    .as_ref()
                    .and_then(|i| i.destination_ip)
                    .map(|ip| ip.to_string())
                    .unwrap_or_default();
                let ip_destination_type = flow
                    .internet
                    .as_ref()
                    .and_then(|i| i.ip_destination_type.clone())
                    .map(|ip| ip.to_string())
                    .unwrap_or_default();
                let label_destination = self.get_label(&link.destination_mac, &ip_destination);

                // Microsecondes et fuseau explicites : deux captures ne se
                // distinguent plus à la seconde près et la date est
                // interprétable sans convention implicite.
                let last_seen = match stats.last_seen.duration_since(std::time::UNIX_EPOCH) {
                    Ok(dur) => chrono::DateTime::<chrono::Utc>::from_timestamp(
                        dur.as_secs() as i64,
                        dur.subsec_nanos(),
                    )
                    .unwrap_or(chrono::DateTime::<chrono::Utc>::UNIX_EPOCH)
                    .format("%Y-%m-%d %H:%M:%S%.6f UTC")
                    .to_string(),
                    Err(_) => "N/A".into(),
                };

                FlowMatrixRow {
                    mac_source: link.source_mac,
                    mac_destination: link.destination_mac,
                    vlan_id: link.vlan_id,
                    protocol_data_link: link.protocol,
                    ip_source,
                    ip_source_type,
                    label_source: label_source.map(|l| escape_formula_cell(&l)),
                    ip_destination,
                    ip_destination_type,
                    label_destination: label_destination.map(|l| escape_formula_cell(&l)),
                    port_source: flow.transport.as_ref().and_then(|t| t.source_port),
                    port_destination: flow.transport.as_ref().and_then(|t| t.destination_port),
                    protocol_transport: flow.transport.as_ref().map(|t| t.protocol.clone()),
                    application_protocol: flow.application.as_ref().map(|a| a.protocol.clone()),
                    count: stats.count,
                    total_bytes: stats.total_bytes,
                    last_seen,
                    encap_id,
                    origin,
                }
            })
            .collect();

        // Tri stable : l'ordre d'itération du HashMap varie d'une exécution
        // à l'autre, deux exports du même état doivent produire le même
        // fichier.
        rows.sort_by(|a, b| {
            (
                &a.mac_source,
                &a.mac_destination,
                &a.ip_source,
                &a.ip_destination,
                &a.protocol_data_link,
                a.vlan_id,
                a.port_source,
                a.port_destination,
                &a.protocol_transport,
                &a.application_protocol,
                &a.encap_id,
            )
                .cmp(&(
                    &b.mac_source,
                    &b.mac_destination,
                    &b.ip_source,
                    &b.ip_destination,
                    &b.protocol_data_link,
                    b.vlan_id,
                    b.port_source,
                    b.port_destination,
                    &b.protocol_transport,
                    &b.application_protocol,
                    &b.encap_id,
                ))
        });
        rows
    }

    /// Exporte la matrice vers un fichier CSV (écriture atomique), préambule
    /// `#LFMS` en tête (version du format, DLT, contexte et période).
    pub fn export_to_csv(&self, path: String) -> std::io::Result<()> {
        Self::write_rows_to_csv_with_context(
            &self.to_flat_vec(),
            self.link_type,
            &self.context,
            self.period.known(),
            &path,
        )
    }

    /// Écrit des lignes de matrice (snapshot de [`Self::to_flat_vec`]) vers
    /// un fichier CSV. Séparé de l'état pour que les commandes d'export
    /// puissent snapshoter sous verrou court (lignes + `link_type`) et écrire
    /// hors verrou, sans bloquer le pipeline de capture pendant l'I/O disque.
    pub fn write_rows_to_csv(
        rows: &[FlowMatrixRow],
        link_type: Option<packet_parser::LinkType>,
        path: &str,
    ) -> std::io::Result<()> {
        Self::write_rows_to_csv_with_context(
            rows,
            link_type,
            &crate::lfms::SurveyContext::default(),
            None,
            path,
        )
    }

    /// Variante portant le contexte du relevé (site, capteur,
    /// interface) et sa période observée dans le préambule `#LFMS`.
    pub fn write_rows_to_csv_with_context(
        rows: &[FlowMatrixRow],
        link_type: Option<packet_parser::LinkType>,
        context: &crate::lfms::SurveyContext,
        period: Option<crate::lfms::ObservedPeriod>,
        path: &str,
    ) -> std::io::Result<()> {
        let preamble = crate::lfms::format_preamble_with_period(link_type, context, period);
        write_csv_atomically(path, Some(preamble), |wtr| {
            for row in rows {
                // Neutralisation au moment d'écrire, et non à la construction
                // de la ligne. Échapper à la construction laissait
                // chaque nouveau champ non protégé par défaut : `origin`,
                // alimenté par le nom d'un fichier importé, sortait tel quel
                // et redevenait une formule à l'ouverture dans un tableur.
                wtr.serialize(escaped_for_export(row))?;
            }
            Ok(())
        })?;
        info!("✅ Matrice exportée avec succès vers {}", path);
        Ok(())
    }

    /// Construit les lignes de labels prêtes pour l'export au format
    /// `mac, ip, label`.
    ///
    /// Les labels édités à la main pendant la capture sont souvent partiels
    /// (MAC seule ou IP seule). Plutôt que d'étendre chaque label partiel
    /// indépendamment — ce qui produisait des doublons contradictoires pour un
    /// même couple `(mac, ip)` — on émet **un seul label résolu par endpoint
    /// réel** de la matrice, via la même précédence que [`Self::get_label`]
    /// (exact > IP seule > MAC seule). Cela complète les champs manquants sans
    /// jamais créer deux labels pour la même clé.
    ///
    /// Les endpoints non identifiants (IP `0.0.0.0`/`::`, MAC broadcast ou
    /// multicast) sont exclus : ce ne sont pas des équipements et ils
    /// provoquaient de faux conflits au réimport. Les labels stockés qui ne
    /// correspondent à aucun endpoint courant sont conservés (champs
    /// placeholder normalisés à vide).
    ///
    /// Les endpoints eux-mêmes sont exposés par [`Self::observed_endpoints`]
    /// (statut « observé/dormant » d'un label dans l'UI de gestion).
    pub fn export_labels(&self) -> Vec<(String, String, String)> {
        self.export_labels_inner()
    }

    /// Endpoints réels observés dans la matrice : couples (MAC unicast,
    /// IP non-placeholder), triés et dédupliqués.
    pub fn observed_endpoints(&self) -> std::collections::BTreeSet<(String, String)> {
        let mut endpoints = std::collections::BTreeSet::new();
        for flow in self.matrix.keys() {
            let src_ip = flow
                .internet
                .as_ref()
                .and_then(|i| i.source_ip)
                .map(|ip| ip.to_string())
                .unwrap_or_default();
            let dst_ip = flow
                .internet
                .as_ref()
                .and_then(|i| i.destination_ip)
                .map(|ip| ip.to_string())
                .unwrap_or_default();

            let link = LinkView::of(&flow.data_link);
            for (mac, ip) in [
                (&link.source_mac, &src_ip),
                (&link.destination_mac, &dst_ip),
            ] {
                if !mac.is_empty()
                    && !ip.is_empty()
                    && !is_non_unicast_mac(mac)
                    && !is_placeholder_ip(ip)
                {
                    endpoints.insert((mac.clone(), ip.clone()));
                }
            }
        }
        endpoints
    }

    /// Équipements réels (voir [`Self::observed_endpoints`]) qui n'ont
    /// aucun label résolu, avec les paquets et octets des flux où ils
    /// apparaissent, triés par paquets décroissants. Les adresses
    /// de groupe et factices sont exclues : ce ne sont pas des équipements.
    pub fn unlabeled_endpoints(&self) -> Vec<UnlabeledEndpoint> {
        let mut totals: HashMap<(String, String), (u64, u64)> = HashMap::new();
        for (flow, entries) in &self.matrix {
            let (packets, bytes) = entries.iter().fold((0u64, 0u64), |(p, b), (_, stats)| {
                (p + stats.count, b.saturating_add(stats.total_bytes))
            });
            let link = LinkView::of(&flow.data_link);
            let ips = flow.internet.as_ref().map_or((None, None), |internet| {
                (internet.source_ip, internet.destination_ip)
            });
            for (mac, ip) in [(&link.source_mac, ips.0), (&link.destination_mac, ips.1)] {
                let Some(ip) = ip.map(|ip| ip.to_string()) else {
                    continue;
                };
                if mac.is_empty() || is_non_unicast_mac(mac) || is_placeholder_ip(&ip) {
                    continue;
                }
                let entry = totals.entry((mac.clone(), ip)).or_default();
                entry.0 += packets;
                entry.1 = entry.1.saturating_add(bytes);
            }
        }
        let mut endpoints: Vec<UnlabeledEndpoint> = totals
            .into_iter()
            .filter(|((mac, ip), _)| self.get_label(mac, ip).is_none())
            .map(|((mac, ip), (packets, bytes))| UnlabeledEndpoint {
                mac,
                ip,
                packets,
                bytes,
            })
            .collect();
        endpoints.sort_by(|a, b| {
            b.packets
                .cmp(&a.packets)
                .then_with(|| a.mac.cmp(&b.mac))
                .then_with(|| a.ip.cmp(&b.ip))
        });
        endpoints
    }

    /// Implémentation de [`export_labels`](Self::export_labels) : un label
    /// résolu par endpoint observé, puis les labels stockés qu'aucun
    /// endpoint ne couvre (clé complète absente, IP seule ou MAC seule non
    /// couverte), MAC de groupe et IP factices ramenées à vide. Le
    /// `BTreeSet` déduplique et fixe l'ordre de sortie.
    fn export_labels_inner(&self) -> Vec<(String, String, String)> {
        use std::collections::{BTreeSet, HashSet};

        let endpoints = self.observed_endpoints();

        // BTreeSet : déduplication + sortie déterministe.
        let mut rows: BTreeSet<(String, String, String)> = BTreeSet::new();
        let mut covered_ips: HashSet<String> = HashSet::new();
        let mut covered_macs: HashSet<String> = HashSet::new();

        // 1) Un label résolu par endpoint réel (précédence exact > IP > MAC).
        for (mac, ip) in &endpoints {
            if let Some(label) = self.get_label(mac, ip) {
                rows.insert((mac.clone(), ip.clone(), label));
                covered_ips.insert(ip.clone());
                covered_macs.insert(mac.clone());
            }
        }

        // 2) Labels stockés non couverts par un endpoint (équipements absents de
        //    la matrice) : préservés, champs placeholder/non-unicast écartés.
        for ((mac, ip), label) in &self.label {
            let mac_eff = if is_non_unicast_mac(mac) {
                ""
            } else {
                mac.as_str()
            };
            let ip_eff = if is_placeholder_ip(ip) {
                ""
            } else {
                ip.as_str()
            };

            match (mac_eff.is_empty(), ip_eff.is_empty()) {
                // Clé complète absente des endpoints : on la conserve.
                (false, false) => {
                    if !endpoints.contains(&(mac_eff.to_string(), ip_eff.to_string())) {
                        rows.insert((mac_eff.to_string(), ip_eff.to_string(), label.clone()));
                    }
                }
                // Label IP seule : conservé si l'IP n'a pas déjà été couverte.
                (true, false) => {
                    if !covered_ips.contains(ip_eff) {
                        rows.insert((String::new(), ip_eff.to_string(), label.clone()));
                    }
                }
                // Label MAC seule : conservé si la MAC n'a pas déjà été couverte.
                (false, true) => {
                    if !covered_macs.contains(mac_eff) {
                        rows.insert((mac_eff.to_string(), String::new(), label.clone()));
                    }
                }
                // Rien d'identifiant : ignoré.
                (true, true) => {}
            }
        }

        rows.into_iter().collect()
    }

    /// Exporte les labels (complétés depuis la matrice) vers un fichier CSV
    /// `mac,ip,label`, réimportable tel quel (écriture atomique).
    pub fn export_labels_to_csv(&self, path: String) -> std::io::Result<()> {
        Self::write_label_rows_to_csv(&self.export_labels(), &path)
    }

    /// Écrit des lignes de labels (snapshot de [`Self::export_labels`]) vers
    /// un fichier CSV `mac,ip,label`. Même logique de verrou court que
    /// [`Self::write_rows_to_csv`].
    pub fn write_label_rows_to_csv(
        rows: &[(String, String, String)],
        path: &str,
    ) -> std::io::Result<()> {
        // Pas de préambule : le fichier de labels est une annexe du relevé,
        // pas une matrice LFMS.
        write_csv_atomically(path, None, |wtr| {
            wtr.write_record(["mac", "ip", "label"])?;
            // Les trois colonnes sont neutralisées : `add_label` accepte
            // n'importe quelle clé, donc `mac` et `ip` ne sont pas plus
            // sûres que `label` (pour une clé légitime, l'échappement est
            // un no-op — aucune MAC ni IP ne commence par un déclencheur).
            for (mac, ip, label) in rows {
                wtr.write_record([
                    &escape_formula_cell(mac),
                    &escape_formula_cell(ip),
                    &escape_formula_cell(label),
                ])?;
            }
            Ok(())
        })?;
        info!("✅ Labels exportés avec succès vers {}", path);
        Ok(())
    }

    /// Enregistre, ou remplace, le label du couple `(mac, ip)`. Une partie
    /// vide donne un label par IP seule ou par MAC seule. Aucune
    /// normalisation ni validation : l'appelant fournit les formes
    /// canoniques.
    pub fn add_label(&mut self, mac: String, ip: String, label: String) {
        self.label.insert((mac, ip), label);
    }

    /// Label d'un endpoint : couple exact, puis IP seule, puis MAC seule.
    /// Le repli sur la MAC seule est **désactivé** quand la MAC est une
    /// passerelle probable ([`is_gateway_mac`](Self::is_gateway_mac)) : sur
    /// un relevé réel, une MAC de routeur porte des dizaines d'IP et un label
    /// posé dessus renommait tout le trafic routé derrière elle.
    /// Un label exact (MAC, IP) ou par IP seule reste toujours appliqué.
    ///
    /// Quand la MAC observée n'identifie pas l'équipement (cible d'une
    /// requête ARP, vue avec la MAC de diffusion), l'IP suffit : un label
    /// (MAC, IP) de même IP s'applique, s'il est le seul nom de cette IP
    ///. Ce repli passe avant celui sur la MAC seule, qui ne
    /// désignerait ici que la diffusion.
    pub fn get_label(&self, mac: &str, ip: &str) -> Option<String> {
        self.label
            .get(&(mac.to_string(), ip.to_string()))
            .or_else(|| self.label.get(&(String::new(), ip.to_string())))
            .cloned()
            .or_else(|| {
                if is_unidentified_mac(mac) {
                    self.unique_label_of_ip(ip)
                } else {
                    None
                }
            })
            .or_else(|| {
                if self.is_gateway_mac(mac) {
                    None
                } else {
                    self.label.get(&(mac.to_string(), String::new())).cloned()
                }
            })
    }

    /// Nom que portent les labels (MAC, IP) de `ip`, s'ils n'en donnent
    /// qu'un. Plusieurs noms pour une même IP (ponts Docker en 172.17.0.1
    /// sur plusieurs hôtes, par exemple) : aucun n'est choisi.
    fn unique_label_of_ip(&self, ip: &str) -> Option<String> {
        if ip.is_empty() || is_placeholder_ip(ip) {
            return None;
        }
        let mut names = self
            .label
            .iter()
            .filter(|((mac, key_ip), _)| !mac.is_empty() && key_ip == ip)
            .map(|(_, label)| label);
        let first = names.next()?;
        names.all(|other| other == first).then(|| first.clone())
    }

    /// Liste des labels stockés, sans leurs clés, dans l'ordre d'itération
    /// de la table. Trace la table sur la sortie standard : reliquat de
    /// débogage.
    pub fn get_label_list(&self) -> Vec<String> {
        println!("get_label_list");
        println!("{:?}", self.label);
        let debug_label = self.label.values().cloned().collect();
        println!("{:?}", debug_label);
        debug_label
    }

    // pub fn get_all_graph_data(&self) -> Vec<FlowMatrixRow> {
    //     self.to_flat_vec()
    // }
    // pub fn add_label_list(&mut self, list: String) {
    //     self.label.insert((mac, ip), label);
    // }
}

/// Écrit un CSV de façon atomique : fichier temporaire dans le même dossier,
/// flush + fsync, puis renommage. Une erreur disque en cours d'écriture ne
/// laisse jamais un fichier final tronqué. Le `preamble` éventuel est
/// écrit tel quel avant la première ligne CSV (ligne `#LFMS`).
fn write_csv_atomically(
    path: &str,
    preamble: Option<String>,
    write: impl FnOnce(&mut csv::Writer<File>) -> std::io::Result<()>,
) -> std::io::Result<()> {
    use std::io::Write;

    // Temporaire unique : `{path}.tmp` était déterministe, donc deux
    // exports concurrents vers la même destination écrivaient dans le même
    // fichier — et le premier à finir supprimait ou renommait celui de
    // l'autre. Le PID et un compteur suffisent à les séparer ; le fichier
    // reste voisin de la destination pour que le renommage final ne
    // traverse pas deux systèmes de fichiers.
    let tmp_path = {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        format!(
            "{path}.{}.{}.tmp",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        )
    };
    let result = (|| {
        let mut file = File::create(&tmp_path)?;
        if let Some(preamble) = preamble {
            writeln!(file, "{preamble}")?;
        }
        let mut wtr = csv::Writer::from_writer(file);
        write(&mut wtr)?;
        wtr.flush()?;
        let file = wtr
            .into_inner()
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        file.sync_all()?;
        std::fs::rename(&tmp_path, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp_path);
    }
    result
}

/// Caractères qui font interpréter une cellule comme formule par un tableur
/// (liste OWASP de l'injection de formule CSV) : `=`, `+`, `-`, `@`, plus la
/// tabulation et le retour chariot, qu'Excel accepte comme blancs de tête
/// avant un déclencheur.
const FORMULA_TRIGGERS: [char; 6] = ['=', '+', '-', '@', '\t', '\r'];

/// Copie d'une ligne dont **toutes** les cellules textuelles sont neutralisées
/// pour un tableur, juste avant sérialisation.
///
/// Chaque champ est listé explicitement plutôt que traité génériquement : le
/// dérive `Serialize` ne laisse pas de crochet pour intercepter les chaînes.
/// C'est le test `every_text_cell_is_neutralised_on_export` qui garantit
/// qu'un champ ajouté plus tard n'échappe pas à cette liste — il remplit
/// toutes les cellules d'une charge de formule et vérifie qu'aucune ne
/// ressort exécutable.
///
/// [`escape_formula_cell`] est idempotente : une valeur déjà échappée
/// commence par `'`, qui n'est pas un caractère de formule.
fn escaped_for_export(row: &FlowMatrixRow) -> FlowMatrixRow {
    let esc = |v: &str| escape_formula_cell(v);
    let esc_opt = |v: &Option<String>| v.as_deref().map(esc);

    FlowMatrixRow {
        mac_source: esc(&row.mac_source),
        mac_destination: esc(&row.mac_destination),
        vlan_id: row.vlan_id,
        protocol_data_link: esc(&row.protocol_data_link),
        ip_source: esc(&row.ip_source),
        ip_source_type: esc(&row.ip_source_type),
        label_source: esc_opt(&row.label_source),
        ip_destination: esc(&row.ip_destination),
        ip_destination_type: esc(&row.ip_destination_type),
        label_destination: esc_opt(&row.label_destination),
        port_source: row.port_source,
        port_destination: row.port_destination,
        protocol_transport: esc_opt(&row.protocol_transport),
        application_protocol: esc_opt(&row.application_protocol),
        count: row.count,
        total_bytes: row.total_bytes,
        last_seen: esc(&row.last_seen),
        encap_id: esc(&row.encap_id),
        origin: esc(&row.origin),
    }
}

/// Préfixe d'échappement des cellules commençant par un caractère de
/// `FORMULA_TRIGGERS` (`=`, `+`, `-`, `@`, tabulation, retour chariot) :
/// ouvertes dans un tableur, elles seraient sinon interprétées comme du
/// code (injection de formule CSV). Le préfixe `'` est la
/// convention tableur ; [`unescape_formula_cell`] le retire au réimport
/// pour préserver l'aller-retour.
pub fn escape_formula_cell(value: &str) -> String {
    if value.starts_with(FORMULA_TRIGGERS) {
        format!("'{value}")
    } else {
        value.to_string()
    }
}

/// Chemin inverse de [`escape_formula_cell`] : retire le `'` d'échappement
/// devant un caractère de formule. Un `'` légitime (non suivi d'un caractère
/// de formule) est conservé tel quel.
pub fn unescape_formula_cell(value: &str) -> String {
    match value.strip_prefix('\'') {
        Some(rest) if rest.starts_with(FORMULA_TRIGGERS) => rest.to_string(),
        _ => value.to_string(),
    }
}

/// Copie d'une ligne dont chaque champ échappé par [`escaped_for_export`]
/// est dé-échappé, appliquée par la lecture de fichier : l'aller-retour
/// export → réimport restitue la donnée d'origine, pour l'ensemble des
/// colonnes et pas seulement les labels.
pub(crate) fn unescaped_for_import(row: &FlowMatrixRow) -> FlowMatrixRow {
    let unesc = |v: &str| unescape_formula_cell(v);
    let unesc_opt = |v: &Option<String>| v.as_deref().map(unesc);

    FlowMatrixRow {
        mac_source: unesc(&row.mac_source),
        mac_destination: unesc(&row.mac_destination),
        vlan_id: row.vlan_id,
        protocol_data_link: unesc(&row.protocol_data_link),
        ip_source: unesc(&row.ip_source),
        ip_source_type: unesc(&row.ip_source_type),
        label_source: unesc_opt(&row.label_source),
        ip_destination: unesc(&row.ip_destination),
        ip_destination_type: unesc(&row.ip_destination_type),
        label_destination: unesc_opt(&row.label_destination),
        port_source: row.port_source,
        port_destination: row.port_destination,
        protocol_transport: unesc_opt(&row.protocol_transport),
        application_protocol: unesc_opt(&row.application_protocol),
        count: row.count,
        total_bytes: row.total_bytes,
        last_seen: unesc(&row.last_seen),
        encap_id: unesc(&row.encap_id),
        origin: unesc(&row.origin),
    }
}

/// Parse la colonne `last_seen` d'une matrice exportée. Accepte le format
/// courant (microsecondes + fuseau explicite) et l'ancien (secondes, UTC
/// implicite) pour rester compatible avec les matrices déjà exportées.
pub fn parse_last_seen(value: &str) -> Result<SystemTime, String> {
    let normalized = value.trim().trim_end_matches(" UTC");
    for format in ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%d %H:%M:%S"] {
        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(normalized, format) {
            let ts = dt.and_utc();
            let secs = u64::try_from(ts.timestamp())
                .map_err(|_| format!("date antérieure à 1970 : « {value} »"))?;
            return Ok(UNIX_EPOCH + std::time::Duration::new(secs, ts.timestamp_subsec_nanos()));
        }
    }
    Err(format!(
        "date invalide « {value} » (attendu : AAAA-MM-JJ HH:MM:SS[.µs] [UTC])"
    ))
}

/// Équipement réel sans label, avec son trafic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnlabeledEndpoint {
    /// MAC unicast de l'équipement.
    pub mac: String,
    /// IP de l'équipement, jamais factice (`0.0.0.0`, `::`, diffusion).
    pub ip: String,
    /// Paquets de tous les flux où l'équipement apparaît, en source ou
    /// en destination.
    pub packets: u64,
    /// Octets de ces mêmes flux (somme saturante).
    pub bytes: u64,
}

/// Une ligne de matrice telle qu'exportée en CSV (format LFMS) : flux à
/// plat, labels résolus et compteurs cumulés tous tunnels confondus.
/// L'ordre des champs est celui des colonnes du fichier ; la lecture
/// réimporte la même structure.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FlowMatrixRow {
    /// MAC source, vide quand la liaison n'en porte pas (RAW IP, SLL/SLL2).
    pub mac_source: String,
    /// MAC de destination, vide dans les mêmes cas.
    pub mac_destination: String,
    /// Identifiant du VLAN 802.1Q, `None` hors VLAN (pcp/dei non exportés).
    pub vlan_id: Option<u16>,
    /// Nom du protocole de liaison (ethertype : « IPv4 », « ARP »…).
    pub protocol_data_link: String,
    /// IP source, vide pour un flux sans couche 3 (ARP par exemple).
    pub ip_source: String,
    /// Classe de l'IP source (privée, publique, multicast…), vide sans
    /// couche 3.
    pub ip_source_type: String,
    /// Label résolu de l'extrémité source, `None` quand aucun label ne
    /// s'applique.
    pub label_source: Option<String>,
    /// IP de destination, vide pour un flux sans couche 3.
    pub ip_destination: String,
    /// Classe de l'IP de destination, vide sans couche 3.
    pub ip_destination_type: String,
    /// Label résolu de l'extrémité de destination, `None` quand aucun
    /// label ne s'applique.
    pub label_destination: Option<String>,
    /// Port source, `None` sans couche transport à ports.
    pub port_source: Option<u16>,
    /// Port de destination, `None` sans couche transport à ports.
    pub port_destination: Option<u16>,
    /// Protocole de transport (« TCP », « UDP »…), `None` sans couche 4.
    pub protocol_transport: Option<String>,
    /// Protocole applicatif reconnu, `None` s'il n'est pas identifié.
    pub application_protocol: Option<String>,
    /// Paquets du flux, tous tunnels confondus.
    pub count: u64,
    /// Octets du flux, tous tunnels confondus.
    pub total_bytes: u64,
    /// Dernier paquet vu, `AAAA-MM-JJ HH:MM:SS.µµµµµµ UTC` ; `N/A` si la
    /// date n'est pas représentable (antérieure à 1970).
    pub last_seen: String,
    /// Tunnels traversés (extension LFMS) : vide (hors tunnel), `id` hex (un
    /// seul tunnel) ou `id:n|id:n|…` (comptes par tunnel, voir
    /// `format_encap_list`). L'id est partagé par la ligne externe et ses
    /// lignes internes, dans les deux sens du tunnel. `serde(default)` pour
    /// rester compatible avec les matrices exportées avant cette colonne.
    #[serde(default)]
    pub encap_id: String,
    /// Fichier(s) d'origine de la ligne, joints par `|` quand un même flux
    /// provient de plusieurs fichiers fusionnés (ex. `site-a.csv|site-b.csv`).
    /// Vide pour une capture live ou un import PCAP. `serde(default)` pour
    /// rester compatible avec les matrices exportées avant cette colonne.
    #[serde(default)]
    pub origin: String,
}

impl FlowMatrixRow {
    /// Validation stricte d'une ligne importée : une IP invalide ou une date
    /// illisible est rejetée avec un message précis, au lieu d'être
    /// silencieusement dégradée en `None`/epoch. Une IP vide reste
    /// valide (flux sans couche 3) ; `N/A` et vide sont des dates absentes.
    pub fn validate(&self, link_type: LinkType) -> Result<(), String> {
        for (field, value) in [
            ("ip_source", &self.ip_source),
            ("ip_destination", &self.ip_destination),
        ] {
            if !value.is_empty() && value.parse::<IpAddr>().is_err() {
                return Err(format!("{field} invalide : « {value} »"));
            }
        }
        let last_seen = self.last_seen.trim();
        if !last_seen.is_empty() && last_seen != "N/A" {
            parse_last_seen(last_seen).map_err(|e| format!("last_seen : {e}"))?;
        }
        // La projection LFMS de liaison dépend du DLT annoncé par le
        // préambule : Ethernet exige des MAC à 6 octets, cooked accepte une
        // adresse source de 1 à 8 octets, RAW n'en accepte aucune.
        match link_type {
            LinkType::ETHERNET => {
                for (field, value) in [
                    ("mac_source", &self.mac_source),
                    ("mac_destination", &self.mac_destination),
                ] {
                    if !value.is_empty() && crate::link::mac_from_text(value).is_none() {
                        return Err(format!("{field} invalide : « {value} »"));
                    }
                }
                if !self.protocol_data_link.is_empty()
                    && crate::link::ethertype_from_name(&self.protocol_data_link).is_none()
                {
                    return Err(format!(
                        "protocol_data_link inconnu : « {} » (attendu : nom connu ou « Unknown (0x…) »)",
                        self.protocol_data_link
                    ));
                }
            }
            // Loopback BSD (packet_parser 11.2.0) : l'en-tête de famille ne
            // porte aucune adresse, la ligne a la forme de l'IP nu.
            LinkType::RAW | LinkType::NULL => {
                let dlt = crate::lfms::link_type_name(link_type);
                if !self.mac_source.is_empty() || !self.mac_destination.is_empty() {
                    return Err(format!(
                        "un relevé {dlt} ne peut pas porter d'adresse de liaison"
                    ));
                }
                if self.vlan_id.is_some() {
                    return Err(format!("un relevé {dlt} ne peut pas porter de VLAN"));
                }
                if !matches!(self.protocol_data_link.as_str(), "IPv4" | "IPv6") {
                    return Err(format!(
                        "protocol_data_link {dlt} invalide : « {} » (attendu : IPv4 ou IPv6)",
                        self.protocol_data_link
                    ));
                }
            }
            LinkType::LINUX_SLL | LinkType::LINUX_SLL2 => {
                crate::link::cooked_address_from_text(&self.mac_source)?;
                if !self.mac_destination.is_empty() {
                    return Err(
                        "un relevé SLL/SLL2 ne peut pas porter d'adresse destination".to_string(),
                    );
                }
                if self.vlan_id.is_some() {
                    return Err("un relevé SLL/SLL2 ne peut pas porter de VLAN".to_string());
                }
                if crate::link::cooked_protocol_from_text(&self.protocol_data_link).is_none() {
                    return Err(format!(
                        "protocol_data_link cooked invalide : « {} » (attendu : nom connu ou 0xNNNN)",
                        self.protocol_data_link
                    ));
                }
            }
            _ => {
                return Err(format!(
                    "type de liaison non reconstructible : {}",
                    link_type.0
                ));
            }
        }
        Ok(())
    }

    /// Reconstruit le flux et ses statistiques par tunnel depuis une ligne de
    /// CSV exporté (chemin inverse de `to_flat_vec`). Les types d'IP sont
    /// recalculés depuis les adresses ; le VLAN ne conserve que son id
    /// (pcp/dei non exportés).
    pub fn to_flow_and_rows(&self, link_type: LinkType) -> Result<ReconstructedFlowRows, String> {
        let source_ip = self.ip_source.parse::<IpAddr>().ok();
        let destination_ip = self.ip_destination.parse::<IpAddr>().ok();

        let internet = (source_ip.is_some() || destination_ip.is_some()).then(|| InternetOwned {
            source_ip,
            ip_source_type: source_ip.map(|_| IpType::from_ip(&self.ip_source)),
            destination_ip,
            ip_destination_type: destination_ip.map(|_| IpType::from_ip(&self.ip_destination)),
            protocol: self.protocol_data_link.clone(),
        });

        let transport = self
            .protocol_transport
            .as_ref()
            .filter(|p| !p.is_empty())
            .map(|p| TransportOwned {
                source_port: self.port_source,
                destination_port: self.port_destination,
                protocol: p.clone(),
            });

        let application = self
            .application_protocol
            .as_ref()
            .filter(|p| !p.is_empty())
            .map(|p| ApplicationOwned {
                protocol: p.clone(),
            });

        let vlan = self.vlan_id.map(|id| VlanTag {
            id,
            pcp: 0,
            dei: false,
            inner_ethertype: Ethertype(0),
        });

        let flow = PacketFlowOwned {
            data_link: matrix_link_from_text(
                link_type,
                &self.mac_source,
                &self.mac_destination,
                &self.protocol_data_link,
                vlan,
            )?,
            internet,
            transport,
            application,
            // Une ligne CSV ne transporte ni diagnostic de corruption ni
            // tunnel imbriqué : les niveaux d'un tunnel sont déjà des lignes
            // distinctes reliées par `encap_id`.
            corrupted: None,
            inner: None,
        };

        // La validation stricte est faite en amont ([`Self::validate`],
        // avec le numéro de ligne) ; ici le repli epoch ne couvre que les
        // dates « absentes » (vide, N/A).
        let last_seen = parse_last_seen(&self.last_seen).unwrap_or(UNIX_EPOCH);

        let rows = parse_encap_list(&self.encap_id, self.count, self.total_bytes, last_seen);

        Ok((flow, rows))
    }
}

use std::time::UNIX_EPOCH;

use crate::packet::CapturedPacketOwned;

/// Convertit un timestamp pcap (`tv_sec`, `tv_usec`) en `SystemTime`.
///
/// Un fichier hostile peut porter n'importe quoi dans ces champs — un
/// timestamp négatif casté en non signé faisait déborder `SystemTime` et
/// **paniquait** (trouvé par un fuzzer de lecture PCAP). Les valeurs invalides sont écrasées vers `UNIX_EPOCH` : un
/// `last_seen` daté de 1970 est visiblement corrompu sans mentir sur le
/// contenu de la trame, qui reste intégrable.
pub fn timeval_to_systemtime(tv_sec: impl Into<i64>, tv_usec: impl Into<i64>) -> SystemTime {
    let tv_sec = tv_sec.into();
    let tv_usec = tv_usec.into();
    if tv_sec < 0 || !(0..1_000_000).contains(&tv_usec) {
        return UNIX_EPOCH;
    }
    UNIX_EPOCH
        .checked_add(std::time::Duration::new(
            tv_sec as u64,
            (tv_usec as u32) * 1000,
        ))
        .unwrap_or(UNIX_EPOCH)
}

#[cfg(test)]
mod tests {
    use super::{
        FlowMatrix, FlowStats, GATEWAY_IP_THRESHOLD, escape_formula_cell, is_non_unicast_mac,
        is_placeholder_ip, parse_last_seen, unescape_formula_cell,
    };
    use crate::link::ethernet_link_from_text;
    use crate::packet::CapturedPacketOwned;
    use packet_parser::owned::{
        LinkLayerOwned, LinuxSll2LinkOwned, LinuxSllLinkOwned, PacketFlowOwned,
    };
    use packet_parser::{LinkType, LinuxArphrdType, LinuxCookedPacketType};
    use std::time::UNIX_EPOCH;

    /// Deux appels sur le même état produisent exactement les mêmes lignes
    /// dans le même ordre : l'export est déterministe.
    #[test]
    fn to_flat_vec_is_deterministic() {
        let mut matrix = FlowMatrix::new();
        for i in 0..50u8 {
            let mut pkt = sample_packet(100);
            pkt.flow.data_link = ethernet_link_from_text(
                &format!("aa:bb:cc:dd:ee:{i:02x}"),
                "aa:bb:cc:dd:ee:ff",
                "IPv4",
                None,
            );
            matrix.update_flow(&pkt);
        }

        let first: Vec<String> = matrix
            .to_flat_vec()
            .iter()
            .map(|r| format!("{}>{}", r.mac_source, r.mac_destination))
            .collect();
        let second: Vec<String> = matrix
            .to_flat_vec()
            .iter()
            .map(|r| format!("{}>{}", r.mac_source, r.mac_destination))
            .collect();

        assert_eq!(first, second);
        let mut sorted = first.clone();
        sorted.sort();
        assert_eq!(first, sorted, "lignes triées par clé stable");
    }

    /// Une date d'import antérieure ne fait pas régresser last_seen.
    #[test]
    fn update_flow_keeps_max_last_seen() {
        let mut matrix = FlowMatrix::new();
        let mut newer = sample_packet(100);
        newer.ts_sec = 2_000;
        let mut older = sample_packet(100);
        older.ts_sec = 1_000;

        matrix.update_flow(&newer);
        matrix.update_flow(&older);

        let (_, stats) = &matrix.matrix.values().next().unwrap()[0];
        assert_eq!(
            stats.last_seen,
            super::timeval_to_systemtime(2_000, 0),
            "un paquet plus ancien ne fait pas régresser la date"
        );
    }

    /// Un timestamp hostile ne panique jamais (cas trouvé par fuzzing) :
    /// négatif, microsecondes hors plage ou débordement de `SystemTime`
    /// donnent `UNIX_EPOCH` — visiblement corrompu, jamais un crash ni une
    /// date inventée plausible.
    #[test]
    fn hostile_timestamps_never_panic() {
        use std::time::UNIX_EPOCH;
        assert_eq!(super::timeval_to_systemtime(-1i64, 0i64), UNIX_EPOCH);
        assert_eq!(super::timeval_to_systemtime(i64::MIN, 0i64), UNIX_EPOCH);
        assert_eq!(super::timeval_to_systemtime(0i64, -1i64), UNIX_EPOCH);
        assert_eq!(super::timeval_to_systemtime(0i64, 1_000_000i64), UNIX_EPOCH);
        // La borne haute de `SystemTime` dépend de la plateforme (Linux tient
        // i64::MAX secondes) : la propriété garantie est l'absence de panic,
        // pas une valeur précise — `checked_add` encaisse le débordement là
        // où il existe.
        let _ = super::timeval_to_systemtime(i64::MAX, 999_999i64);
        // Un timestamp valide reste exact.
        assert_eq!(
            super::timeval_to_systemtime(2_000i64, 500_000i64),
            UNIX_EPOCH + std::time::Duration::new(2_000, 500_000_000)
        );
    }

    #[test]
    fn formula_cells_are_escaped_and_roundtrip() {
        assert_eq!(escape_formula_cell("=SUM(A1)"), "'=SUM(A1)");
        assert_eq!(escape_formula_cell("@cmd"), "'@cmd");
        assert_eq!(escape_formula_cell("Automate 3"), "Automate 3");
        assert_eq!(unescape_formula_cell("'=SUM(A1)"), "=SUM(A1)");
        assert_eq!(
            unescape_formula_cell("'apostrophe légitime"),
            "'apostrophe légitime"
        );
    }

    /// L'ancien format (secondes, UTC implicite) et le nouveau
    /// (microsecondes + fuseau) sont tous deux acceptés ; l'illisible est
    /// rejeté avec un message.
    #[test]
    fn parse_last_seen_accepts_both_formats_and_rejects_garbage() {
        let old = parse_last_seen("2026-07-11 10:00:00").unwrap();
        let new = parse_last_seen("2026-07-11 10:00:00.123456 UTC").unwrap();
        assert!(new > old);
        assert!(parse_last_seen("pas une date").is_err());
    }

    fn sample_packet(len: u32) -> CapturedPacketOwned {
        CapturedPacketOwned {
            ts_sec: 1_000,
            ts_usec: 0,
            caplen: len,
            len,
            flow: PacketFlowOwned {
                data_link: ethernet_link_from_text(
                    "11:22:33:44:55:66",
                    "aa:bb:cc:dd:ee:ff",
                    "IPv4",
                    None,
                ),
                internet: None,
                transport: None,
                application: None,
                corrupted: None,
                inner: None,
            },
            encap_id: None,
        }
    }
    /// Les champs cooked propres à la sonde ne font pas partie de l'identité
    /// de conversation. Les paquets originaux restent fidèles pour l'IPC,
    /// tandis que compteurs et octets fusionnent dans une seule ligne.
    #[test]
    fn cooked_observation_metadata_does_not_split_matrix_rows() {
        let address = Some(vec![0xaa, 0xbb, 0xcc, 0xdd]);

        let mut sll_a = sample_packet(40);
        sll_a.flow.data_link = LinkLayerOwned::linux_sll(LinuxSllLinkOwned::new(
            LinuxCookedPacketType::OUTGOING,
            LinuxArphrdType::ETHERNET,
            9,
            address.clone(),
            0x0800,
        ));
        let mut sll_b = sample_packet(60);
        sll_b.flow.data_link = LinkLayerOwned::linux_sll(LinuxSllLinkOwned::new(
            LinuxCookedPacketType::HOST,
            LinuxArphrdType::LOOPBACK,
            4,
            address.clone(),
            0x0800,
        ));
        let original_sll_a = sll_a.flow.data_link.clone();

        let mut sll_matrix = FlowMatrix::new();
        sll_matrix.link_type = Some(LinkType::LINUX_SLL);
        sll_matrix.update_flow(&sll_a);
        sll_matrix.update_flow(&sll_b);

        assert_eq!(sll_matrix.row_count(), 1);
        let sll_stats = &sll_matrix.matrix.values().next().unwrap()[0].1;
        assert_eq!((sll_stats.count, sll_stats.total_bytes), (2, 100));
        assert_eq!(
            sll_a.flow.data_link, original_sll_a,
            "la normalisation de clé ne doit pas altérer le paquet IPC"
        );

        let mut sll2_a = sample_packet(70);
        sll2_a.flow.data_link = LinkLayerOwned::linux_sll2(LinuxSll2LinkOwned::new(
            0xabcd,
            7,
            3,
            LinuxArphrdType::ETHERNET,
            LinuxCookedPacketType::OUTGOING,
            8,
            address.clone(),
        ));
        let mut sll2_b = sample_packet(30);
        sll2_b.flow.data_link = LinkLayerOwned::linux_sll2(LinuxSll2LinkOwned::new(
            0xabcd,
            0,
            42,
            LinuxArphrdType::LOOPBACK,
            LinuxCookedPacketType::BROADCAST,
            4,
            address.clone(),
        ));
        let original_sll2_a = sll2_a.flow.data_link.clone();

        let mut sll2_matrix = FlowMatrix::new();
        sll2_matrix.link_type = Some(LinkType::LINUX_SLL2);
        sll2_matrix.update_flow(&sll2_a);
        sll2_matrix.update_flow(&sll2_b);

        assert_eq!(sll2_matrix.row_count(), 1);
        let sll2_stats = &sll2_matrix.matrix.values().next().unwrap()[0].1;
        assert_eq!((sll2_stats.count, sll2_stats.total_bytes), (2, 100));
        assert_eq!(sll2_a.flow.data_link, original_sll2_a);

        let mut other_address = sll2_b.clone();
        other_address.flow.data_link = LinkLayerOwned::linux_sll2(LinuxSll2LinkOwned::new(
            0xabcd,
            0,
            99,
            LinuxArphrdType::ETHERNET,
            LinuxCookedPacketType::HOST,
            4,
            Some(vec![0xaa, 0xbb, 0xcc, 0xee]),
        ));
        sll2_matrix.update_flow(&other_address);
        assert_eq!(
            sll2_matrix.row_count(),
            2,
            "une adresse source différente reste une identité différente"
        );
    }

    #[test]
    fn update_flow_counts_first_packet_bytes_once() {
        let mut matrix = FlowMatrix::new();
        let pkt = sample_packet(100);

        matrix.update_flow(&pkt);

        let entries = matrix.matrix.get(&pkt.flow).expect("flux inséré");
        let (encap_id, stats) = &entries[0];
        assert_eq!(*encap_id, None);
        assert_eq!(stats.count, 1);
        assert_eq!(stats.total_bytes, 100);
    }

    #[test]
    fn update_flow_accumulates_on_existing_flow() {
        let mut matrix = FlowMatrix::new();
        let pkt = sample_packet(100);

        matrix.update_flow(&pkt);
        matrix.update_flow(&pkt);
        matrix.update_flow(&pkt);

        assert_eq!(matrix.row_count(), 1, "un seul flux");
        let entries = matrix.matrix.get(&pkt.flow).expect("flux inséré");
        let (_, stats) = &entries[0];
        assert_eq!(stats.count, 3);
        assert_eq!(stats.total_bytes, 300);
    }

    /// Le même flux interne vu à travers deux tunnels différents garde UNE
    /// seule ligne à l'export ; les compteurs par tunnel restent exacts dans
    /// la colonne `encap_id` (somme des fils d'un tunnel == compteur du père).
    #[test]
    fn update_flow_keeps_one_row_with_per_tunnel_counts() {
        let mut matrix = FlowMatrix::new();
        let mut in_tunnel_a = sample_packet(100);
        in_tunnel_a.encap_id = Some(0xaaaa);
        let mut in_tunnel_b = sample_packet(100);
        in_tunnel_b.encap_id = Some(0xbbbb);

        matrix.update_flow(&in_tunnel_a);
        matrix.update_flow(&in_tunnel_a);
        matrix.update_flow(&in_tunnel_b);

        assert_eq!(matrix.row_count(), 1, "une seule ligne pour ce flux");

        let rows = matrix.to_flat_vec();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].count, 3, "compteur total du flux");
        assert_eq!(
            rows[0].encap_id, "000000000000aaaa:2|000000000000bbbb:1",
            "ventilation par tunnel dans la colonne encap_id"
        );
    }

    /// Un flux dans un seul tunnel exporte l'id nu (sans `:n`), comme la
    /// ligne externe du tunnel : la jointure directe reste possible.
    #[test]
    fn single_tunnel_flow_exports_bare_id() {
        let mut matrix = FlowMatrix::new();
        let mut pkt = sample_packet(100);
        pkt.encap_id = Some(0xaaaa);

        matrix.update_flow(&pkt);
        matrix.update_flow(&pkt);

        let rows = matrix.to_flat_vec();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].encap_id, "000000000000aaaa");
        assert_eq!(rows[0].count, 2);
    }

    /// Aller-retour export -> import de la colonne `encap_id`, dans ses trois
    /// formes : vide, id nu, liste `id:n`.
    #[test]
    fn encap_list_roundtrips_through_format_and_parse() {
        use super::{format_encap_list, parse_encap_list};
        use std::time::UNIX_EPOCH;

        for encap in [
            "",
            "000000000000aaaa",
            "000000000000aaaa:2|000000000000bbbb:1",
        ] {
            let entries = parse_encap_list(encap, 3, 300, UNIX_EPOCH);
            assert_eq!(
                entries.iter().map(|(_, s)| s.count).sum::<u64>(),
                3,
                "le compteur total doit être conservé pour {encap:?}"
            );
            assert_eq!(
                entries.iter().map(|(_, s)| s.total_bytes).sum::<u64>(),
                300,
                "les octets totaux doivent être conservés pour {encap:?}"
            );
            assert_eq!(format_encap_list(&entries), encap, "aller-retour");
        }

        // Liste partielle : 2 paquets sur 3 en tunnel, le reste hors tunnel.
        let entries = parse_encap_list("000000000000aaaa:2", 3, 300, UNIX_EPOCH);
        assert_eq!(entries.len(), 2, "part hors tunnel sur une entrée None");
        assert!(entries.iter().any(|(id, s)| id.is_none() && s.count == 1));
    }

    /// L'import CSV fusionne les doublons tunnel par tunnel.
    #[test]
    fn merge_row_accumulates_per_tunnel() {
        use super::FlowStats;
        use std::time::UNIX_EPOCH;

        let mut matrix = FlowMatrix::new();
        let flow = sample_packet(100).flow;
        let stats = |count: u64| FlowStats {
            count,
            total_bytes: count * 10,
            last_seen: UNIX_EPOCH,
        };

        matrix.merge_row(flow.clone(), Some(0xaaaa), stats(2));
        matrix.merge_row(flow.clone(), Some(0xaaaa), stats(3));
        matrix.merge_row(flow.clone(), Some(0xbbbb), stats(1));

        assert_eq!(matrix.row_count(), 1, "une seule ligne pour ce flux");
        let entries = matrix.matrix.get(&flow).unwrap();
        let merged = entries
            .iter()
            .find(|(id, _)| *id == Some(0xaaaa))
            .map(|(_, s)| s)
            .unwrap();
        assert_eq!(merged.count, 5);
        assert_eq!(merged.total_bytes, 50);
    }

    #[test]
    fn get_label_falls_back_to_ip_only_label() {
        let mut matrix = FlowMatrix::new();
        matrix.add_label(
            String::new(),
            "8.8.8.8".to_string(),
            "google.com".to_string(),
        );

        assert_eq!(
            matrix.get_label("aa:bb:cc:dd:ee:ff", "8.8.8.8"),
            Some("google.com".to_string())
        );
    }

    #[test]
    fn get_label_falls_back_to_mac_only_label() {
        let mut matrix = FlowMatrix::new();
        matrix.add_label(
            "bc:24:11:ff:3e:15".to_string(),
            String::new(),
            "google.com".to_string(),
        );

        assert_eq!(
            matrix.get_label("bc:24:11:ff:3e:15", "192.168.1.105"),
            Some("google.com".to_string())
        );
    }

    /// Seuil passerelle : tant que la MAC porte moins de
    /// `GATEWAY_IP_THRESHOLD` IP unicast, le repli MAC seule s'applique ; au
    /// seuil, il est coupé — le couple exact et l'IP seule restent servis.
    #[test]
    fn get_label_stops_mac_only_fallback_on_gateway_mac() {
        let gateway = "00:04:96:a0:0f:60";
        let mut matrix = FlowMatrix::new();
        matrix.add_label(gateway.to_string(), String::new(), "Routeur".to_string());

        for n in 1..GATEWAY_IP_THRESHOLD {
            matrix.update_flow(&packet_with_ips(
                "10:22:33:44:55:66",
                "192.168.1.10",
                gateway,
                &format!("10.0.0.{n}"),
            ));
            assert_eq!(matrix.gateway_ip_count(gateway), n);
            assert!(
                !matrix.is_gateway_mac(gateway),
                "{n} IP : pas encore une passerelle"
            );
            assert_eq!(
                matrix.get_label(gateway, "10.0.0.250"),
                Some("Routeur".to_string()),
                "{n} IP : le repli MAC seule s'applique encore"
            );
        }

        matrix.update_flow(&packet_with_ips(
            "10:22:33:44:55:66",
            "192.168.1.10",
            gateway,
            &format!("10.0.0.{GATEWAY_IP_THRESHOLD}"),
        ));
        assert!(matrix.is_gateway_mac(gateway));
        assert_eq!(
            matrix.get_label(gateway, "10.0.0.250"),
            None,
            "au seuil, plus de repli MAC seule sur une passerelle"
        );

        // Le couple exact et l'IP seule gardent la priorité, passerelle ou non.
        matrix.add_label(
            gateway.to_string(),
            "10.0.0.1".to_string(),
            "Box".to_string(),
        );
        matrix.add_label(String::new(), "10.0.0.2".to_string(), "Serveur".to_string());
        assert_eq!(
            matrix.get_label(gateway, "10.0.0.1"),
            Some("Box".to_string())
        );
        assert_eq!(
            matrix.get_label(gateway, "10.0.0.2"),
            Some("Serveur".to_string())
        );

        // L'export de labels suit la même règle : la passerelle n'est plus
        // « complétée » avec chacune de ses IP.
        let exported = matrix.export_labels();
        assert!(
            !exported
                .iter()
                .any(|(mac, ip, label)| mac == gateway && ip == "10.0.0.3" && label == "Routeur"),
            "{exported:?}"
        );
    }

    /// Les IP non identifiantes (multicast, broadcast, lien-local, non
    /// spécifiée) ne comptent pas ; IPv4 et IPv6 sont comptées séparément ;
    /// une MAC broadcast/multicast n'est jamais une passerelle ; `clear`
    /// réinitialise le compte ; `merge_row` (import de matrice) alimente
    /// aussi l'index.
    #[test]
    fn gateway_ip_count_ignores_non_identifying_ips_and_counts_families_apart() {
        let mac = "10:22:33:44:55:66";
        let mut matrix = FlowMatrix::new();
        for dst in [
            "224.0.0.251",
            "255.255.255.255",
            "169.254.10.10",
            "0.0.0.0",
            "ff02::1",
            "fe80::1",
        ] {
            matrix.update_flow(&packet_with_ips(
                mac,
                "192.168.1.10",
                "ff:ff:ff:ff:ff:ff",
                dst,
            ));
        }
        assert_eq!(matrix.gateway_ip_count(mac), 1, "seule 192.168.1.10 compte");
        assert_eq!(matrix.gateway_ip_count("ff:ff:ff:ff:ff:ff"), 0);

        // Double pile : 2 IPv4 + 2 IPv6 = max(2, 2), pas 4.
        matrix.update_flow(&packet_with_ips(
            mac,
            "192.168.1.11",
            "aa:bb:cc:dd:ee:ff",
            "192.168.1.20",
        ));
        matrix.update_flow(&packet_with_ips(
            mac,
            "2001:db8::10",
            "aa:bb:cc:dd:ee:ff",
            "2001:db8::20",
        ));
        matrix.update_flow(&packet_with_ips(
            mac,
            "2001:db8::11",
            "aa:bb:cc:dd:ee:ff",
            "2001:db8::20",
        ));
        assert_eq!(matrix.gateway_ip_count(mac), 2);
        assert!(!matrix.is_gateway_mac(mac));

        matrix.clear();
        assert_eq!(matrix.gateway_ip_count(mac), 0);

        // Import de matrice : chaque ligne fusionnée alimente l'index.
        for n in 1..=GATEWAY_IP_THRESHOLD {
            let pkt = packet_with_ips(
                mac,
                &format!("10.1.0.{n}"),
                "aa:bb:cc:dd:ee:ff",
                "10.1.0.200",
            );
            matrix.merge_row(
                pkt.flow.clone(),
                None,
                FlowStats {
                    count: 1,
                    total_bytes: 1,
                    last_seen: UNIX_EPOCH,
                },
            );
        }
        assert!(matrix.is_gateway_mac(mac));
    }

    #[test]
    fn get_label_prefers_exact_mac_ip_label() {
        let mut matrix = FlowMatrix::new();
        matrix.add_label(
            String::new(),
            "8.8.8.8".to_string(),
            "google.com".to_string(),
        );
        matrix.add_label(
            "aa:bb:cc:dd:ee:ff".to_string(),
            "8.8.8.8".to_string(),
            "custom dns".to_string(),
        );

        assert_eq!(
            matrix.get_label("aa:bb:cc:dd:ee:ff", "8.8.8.8"),
            Some("custom dns".to_string())
        );
    }

    /// La cible d'une requête ARP est vue avec la MAC de diffusion,
    /// jamais avec la sienne. Seule son IP la désigne : le label (MAC, IP)
    /// tiré d'un relevé la nomme, s'il est le seul nom de cette IP.
    #[test]
    fn get_label_names_an_unidentified_mac_by_its_ip_when_one_name_carries_it() {
        let mut matrix = FlowMatrix::new();
        matrix.add_label(
            "aa:bb:cc:00:00:01".to_string(),
            "10.0.0.1".to_string(),
            "poste-01".to_string(),
        );
        matrix.add_label(
            "aa:bb:cc:00:00:01".to_string(),
            String::new(),
            "poste-01".to_string(),
        );

        for mac in [
            "ff:ff:ff:ff:ff:ff",
            "00:00:00:00:00:00",
            "",
            "01:00:5e:00:00:01",
        ] {
            assert_eq!(
                matrix.get_label(mac, "10.0.0.1"),
                Some("poste-01".to_string()),
                "MAC {mac:?}"
            );
        }
        // Une autre vraie MAC sur la même IP reste une autre machine.
        assert_eq!(matrix.get_label("aa:bb:cc:00:00:99", "10.0.0.1"), None);
        // Une IP absente du référentiel ne prend aucun nom.
        assert_eq!(matrix.get_label("ff:ff:ff:ff:ff:ff", "10.0.0.2"), None);
        assert_eq!(
            matrix.get_label("ff:ff:ff:ff:ff:ff", "255.255.255.255"),
            None
        );
    }

    #[test]
    fn get_label_does_not_choose_between_two_names_for_one_ip() {
        let mut matrix = FlowMatrix::new();
        // Deux hôtes Docker, chacun avec son pont en 172.17.0.1.
        matrix.add_label(
            "aa:bb:cc:00:00:0a".to_string(),
            "172.17.0.1".to_string(),
            "hote-a".to_string(),
        );
        matrix.add_label(
            "aa:bb:cc:00:00:0b".to_string(),
            "172.17.0.1".to_string(),
            "hote-b".to_string(),
        );
        assert_eq!(matrix.get_label("ff:ff:ff:ff:ff:ff", "172.17.0.1"), None);

        // Le même nom sous deux MAC (carte remplacée entre deux relevés)
        // n'est pas une ambiguïté.
        matrix.add_label(
            "aa:bb:cc:00:00:01".to_string(),
            "10.0.0.1".to_string(),
            "poste-01".to_string(),
        );
        matrix.add_label(
            "aa:bb:cc:00:00:02".to_string(),
            "10.0.0.1".to_string(),
            "poste-01".to_string(),
        );
        assert_eq!(
            matrix.get_label("ff:ff:ff:ff:ff:ff", "10.0.0.1"),
            Some("poste-01".to_string())
        );
    }

    #[test]
    fn get_label_by_ip_for_an_unidentified_mac_keeps_the_existing_precedence() {
        let mut matrix = FlowMatrix::new();
        matrix.add_label(
            "aa:bb:cc:00:00:01".to_string(),
            "10.0.0.1".to_string(),
            "poste-01".to_string(),
        );
        matrix.add_label(String::new(), "10.0.0.1".to_string(), "nom-ip".to_string());
        matrix.add_label(
            "ff:ff:ff:ff:ff:ff".to_string(),
            String::new(),
            "Diffusion".to_string(),
        );

        // L'IP seule garde la priorité sur le nom trouvé par l'IP.
        assert_eq!(
            matrix.get_label("ff:ff:ff:ff:ff:ff", "10.0.0.1"),
            Some("nom-ip".to_string())
        );
        // Une IP sans label garde le repli sur la MAC seule.
        assert_eq!(
            matrix.get_label("ff:ff:ff:ff:ff:ff", "10.0.0.9"),
            Some("Diffusion".to_string())
        );
    }

    /// De bout en bout sur la matrice : « qui a 10.0.0.1 ? » envoyé par la
    /// passerelle en diffusion. La ligne porte le nom de la cible, et l'export
    /// des labels n'invente pas de clé sur la MAC de diffusion.
    #[test]
    fn an_arp_request_row_is_named_after_its_target() {
        let mut matrix = FlowMatrix::new();
        matrix.update_flow(&packet_with_ips(
            "10:22:33:44:55:66",
            "10.0.0.254",
            "ff:ff:ff:ff:ff:ff",
            "10.0.0.1",
        ));
        matrix.add_label(
            "aa:bb:cc:00:00:01".to_string(),
            "10.0.0.1".to_string(),
            "poste-01".to_string(),
        );

        let rows = matrix.to_flat_vec();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label_destination.as_deref(), Some("poste-01"));
        assert_eq!(rows[0].label_source, None);

        let exported = matrix.export_labels();
        assert_eq!(
            exported,
            vec![(
                "aa:bb:cc:00:00:01".to_string(),
                "10.0.0.1".to_string(),
                "poste-01".to_string()
            )],
            "seul le label du référentiel est exporté"
        );
    }

    fn packet_with_ips(
        src_mac: &str,
        src_ip: &str,
        dst_mac: &str,
        dst_ip: &str,
    ) -> CapturedPacketOwned {
        use packet_parser::IpType;
        use packet_parser::owned::InternetOwned;
        use std::net::IpAddr;

        CapturedPacketOwned {
            ts_sec: 1_000,
            ts_usec: 0,
            caplen: 100,
            len: 100,
            flow: PacketFlowOwned {
                data_link: ethernet_link_from_text(src_mac, dst_mac, "IPv4", None),
                internet: Some(InternetOwned {
                    source_ip: Some(src_ip.parse::<IpAddr>().unwrap()),
                    ip_source_type: Some(IpType::from_ip(src_ip)),
                    destination_ip: Some(dst_ip.parse::<IpAddr>().unwrap()),
                    ip_destination_type: Some(IpType::from_ip(dst_ip)),
                    protocol: "IPv4".to_string(),
                }),
                transport: None,
                application: None,
                corrupted: None,
                inner: None,
            },
            encap_id: None,
        }
    }

    #[test]
    fn export_labels_completes_missing_fields_from_matrix() {
        let mut matrix = FlowMatrix::new();
        // MAC unicast valides (premier octet pair -> bit multicast à 0).
        matrix.update_flow(&packet_with_ips(
            "10:22:33:44:55:66",
            "192.168.1.10",
            "aa:bb:cc:dd:ee:ff",
            "192.168.1.20",
        ));

        // Label saisi par IP seule pendant la capture -> MAC complétée.
        matrix.add_label(
            String::new(),
            "192.168.1.10".to_string(),
            "poste-A".to_string(),
        );
        // Label saisi par MAC seule -> IP complétée.
        matrix.add_label(
            "aa:bb:cc:dd:ee:ff".to_string(),
            String::new(),
            "serveur-B".to_string(),
        );

        let rows = matrix.export_labels();

        assert!(rows.contains(&(
            "10:22:33:44:55:66".to_string(),
            "192.168.1.10".to_string(),
            "poste-A".to_string()
        )));
        assert!(rows.contains(&(
            "aa:bb:cc:dd:ee:ff".to_string(),
            "192.168.1.20".to_string(),
            "serveur-B".to_string()
        )));
        assert!(
            rows.iter().all(|(m, i, _)| !m.is_empty() && !i.is_empty()),
            "les champs manquants doivent être complétés : {rows:?}"
        );
    }

    #[test]
    fn export_labels_keeps_partial_label_when_matrix_has_no_match() {
        let mut matrix = FlowMatrix::new();
        // Label par IP seule, mais aucune IP correspondante dans la matrice.
        matrix.add_label(
            String::new(),
            "10.0.0.99".to_string(),
            "inconnu".to_string(),
        );

        let rows = matrix.export_labels();

        assert_eq!(
            rows,
            vec![(
                String::new(),
                "10.0.0.99".to_string(),
                "inconnu".to_string()
            )]
        );
    }

    #[test]
    fn export_labels_emits_single_label_per_endpoint() {
        // Un même endpoint porte un label MAC-seule ET un label IP-seule
        // différents : l'export ne doit produire qu'une seule ligne (précédence),
        // pas deux lignes contradictoires pour la même clé (mac, ip).
        let mut matrix = FlowMatrix::new();
        matrix.update_flow(&packet_with_ips(
            "48:21:0b:41:45:65",
            "192.168.1.182",
            "aa:bb:cc:dd:ee:ff",
            "192.168.1.20",
        ));
        matrix.add_label(
            "48:21:0b:41:45:65".to_string(),
            String::new(),
            "TVD09".to_string(),
        );
        matrix.add_label(
            String::new(),
            "192.168.1.182".to_string(),
            "pc lunar".to_string(),
        );

        let rows = matrix.export_labels();

        let for_endpoint: Vec<_> = rows
            .iter()
            .filter(|(m, i, _)| m == "48:21:0b:41:45:65" && i == "192.168.1.182")
            .collect();
        assert_eq!(
            for_endpoint.len(),
            1,
            "un seul label par couple (mac, ip) : {rows:?}"
        );
    }

    #[test]
    fn export_labels_excludes_placeholder_ip_and_broadcast_mac() {
        let mut matrix = FlowMatrix::new();
        // Endpoint avec IP placeholder et MAC broadcast : ne doit rien produire
        // d'identifiant à l'export.
        matrix.update_flow(&packet_with_ips(
            "48:21:0b:41:45:65",
            "0.0.0.0",
            "ff:ff:ff:ff:ff:ff",
            "100.180.65.40",
        ));
        matrix.add_label(
            "48:21:0b:41:45:65".to_string(),
            "0.0.0.0".to_string(),
            "x".to_string(),
        );

        let rows = matrix.export_labels();

        assert!(
            rows.iter()
                .all(|(m, i, _)| !is_placeholder_ip(i) && !is_non_unicast_mac(m)),
            "ni IP placeholder ni MAC broadcast/multicast : {rows:?}"
        );
    }
}

// #[cfg(test)]
// mod tests {
//     use super::*;
//     use std::time::{SystemTime, Duration};

//     // Dummy PacketFlow pour les tests (adapter selon la vraie signature de PacketFlow)
//     #[derive(Debug, Clone, PartialEq, Eq, Hash)]
//     struct DummyFlow(u8);

//     // Dummy impl PacketFlow<'a> pour les tests (adapter selon la vraie signature de PacketFlow)
//     // Ici, on suppose PacketFlow<'a> = DummyFlow (adapter selon ton code réel)
//     impl<'a> From<&'a DummyFlow> for PacketFlow<'a> {
//         fn from(df: &'a DummyFlow) -> Self {
//             unsafe { std::mem::transmute_copy(df) }
//         }
//     }

//     #[test]
//     fn test_new_flow_matrix() {
//         let matrix: FlowMatrix = FlowMatrix { flows: HashMap::new() };
//         assert_eq!(matrix.flows.len(), 0);
//     }

//     #[test]
//     fn test_update_flow_inserts_and_updates() {
//         let mut matrix = FlowMatrix { flows: HashMap::new() };
//         let now = SystemTime::now();
//         let flow: PacketFlow<'_> = unsafe { std::mem::zeroed() }; // Remplacer par un vrai PacketFlow si possible

//         matrix.update_flow(flow.clone(), 100, timeval_to_systemtime(now));
//         assert_eq!(matrix.flows.len(), 1);
//         let stats = matrix.flows.get(&flow).unwrap();
//         assert_eq!(stats.count, 1);
//         assert_eq!(stats.total_bytes, 100);
//         assert_eq!(stats.last_seen, now);

//         // Update same flow
//         let later = now + Duration::from_secs(10);
//         matrix.update_flow(flow.clone(), 50, timeval_to_systemtime(later));
//         let stats = matrix.flows.get(&flow).unwrap();
//         assert_eq!(stats.count, 2);
//         assert_eq!(stats.total_bytes, 150);
//         assert_eq!(stats.last_seen, later);
//     }
// }

#[cfg(test)]
mod unlabeled_endpoints_tests {
    use super::FlowMatrix;
    use std::path::Path;

    fn sample() -> FlowMatrix {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/20260703_NP_Matrice.csv");
        crate::csv::merge_matrix_files(&[path]).expect("matrice de test")
    }

    #[test]
    fn real_unlabeled_endpoints_sorted_by_packets() {
        let matrix = sample();
        let endpoints = matrix.unlabeled_endpoints();
        assert!(!endpoints.is_empty());
        assert!(
            endpoints
                .windows(2)
                .all(|pair| pair[0].packets >= pair[1].packets),
            "tri par paquets décroissants"
        );
        for endpoint in &endpoints {
            assert!(!endpoint.mac.is_empty() && !endpoint.ip.is_empty());
            assert!(!super::is_non_unicast_mac(&endpoint.mac), "{endpoint:?}");
            assert!(!super::is_placeholder_ip(&endpoint.ip), "{endpoint:?}");
            assert!(matrix.get_label(&endpoint.mac, &endpoint.ip).is_none());
            assert!(endpoint.packets > 0 && endpoint.bytes > 0);
        }
    }

    #[test]
    fn labelling_an_endpoint_removes_it_from_the_list() {
        let mut matrix = sample();
        let first = matrix.unlabeled_endpoints()[0].clone();
        matrix.add_label(first.mac.clone(), first.ip.clone(), "Automate".to_string());
        assert!(
            !matrix
                .unlabeled_endpoints()
                .iter()
                .any(|endpoint| endpoint.mac == first.mac && endpoint.ip == first.ip)
        );
    }
}

#[cfg(test)]
mod formula_injection_tests {
    use super::*;

    const PAYLOAD: &str = "=cmd|'/c calc'!A1";

    fn row_with_every_text_cell_hostile() -> FlowMatrixRow {
        FlowMatrixRow {
            mac_source: PAYLOAD.to_string(),
            mac_destination: PAYLOAD.to_string(),
            vlan_id: Some(7),
            protocol_data_link: PAYLOAD.to_string(),
            ip_source: PAYLOAD.to_string(),
            ip_source_type: PAYLOAD.to_string(),
            label_source: Some(PAYLOAD.to_string()),
            ip_destination: PAYLOAD.to_string(),
            ip_destination_type: PAYLOAD.to_string(),
            label_destination: Some(PAYLOAD.to_string()),
            port_source: Some(1),
            port_destination: Some(2),
            protocol_transport: Some(PAYLOAD.to_string()),
            application_protocol: Some(PAYLOAD.to_string()),
            count: 1,
            total_bytes: 1,
            last_seen: PAYLOAD.to_string(),
            encap_id: PAYLOAD.to_string(),
            origin: PAYLOAD.to_string(),
        }
    }

    /// Garde-fou de couverture : si un champ texte est ajouté à
    /// `FlowMatrixRow` sans être neutralisé dans `escaped_for_export`, sa
    /// cellule ressort exécutable et ce test échoue.
    #[test]
    fn every_text_cell_is_neutralised_on_export() {
        let path = std::env::temp_dir().join(format!(
            "lunar_formula_injection_{}.csv",
            std::process::id()
        ));
        let target = path.to_str().unwrap();
        FlowMatrix::write_rows_to_csv(&[row_with_every_text_cell_hostile()], None, target).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).ok();

        let data_line = content
            .lines()
            .last()
            .expect("le CSV doit porter une ligne de données");

        let mut executable: Vec<&str> = Vec::new();
        for cell in data_line.split(',') {
            // Le lecteur CSV cite une cellule contenant une virgule ; la
            // charge n'en contient pas, le découpage naïf suffit ici.
            let cell = cell.trim_matches('"');
            if cell.starts_with(FORMULA_TRIGGERS) {
                executable.push(cell);
            }
        }

        assert!(
            executable.is_empty(),
            "cellules encore exécutables à l'export : {executable:?}\nligne : {data_line}"
        );
    }

    /// L'aller-retour ne doit pas altérer la donnée : ce qui a été échappé à
    /// l'écriture est retrouvé tel quel à la lecture.
    #[test]
    fn escaping_round_trips_without_altering_the_value() {
        assert_eq!(
            unescape_formula_cell(&escape_formula_cell(PAYLOAD)),
            PAYLOAD
        );
        // Idempotence : ré-échapper une valeur déjà protégée ne l'empile pas.
        let once = escape_formula_cell(PAYLOAD);
        assert_eq!(escape_formula_cell(&once), once);
    }

    /// Garde-fou sur la liste des déclencheurs elle-même : chacun des
    /// caractères connus (liste OWASP, tab et CR compris — Excel ignore les
    /// blancs de tête avant une formule) doit être échappé, et l'échappement
    /// doit s'inverser sans perte. Retirer un caractère de
    /// `FORMULA_TRIGGERS` fait échouer ce test, pas seulement une revue.
    #[test]
    fn every_known_formula_trigger_is_escaped_and_round_trips() {
        for trigger in FORMULA_TRIGGERS {
            let hostile = format!("{trigger}1+cmd");
            let escaped = escape_formula_cell(&hostile);
            assert_eq!(
                escaped,
                format!("'{hostile}"),
                "déclencheur non échappé : {trigger:?}"
            );
            assert_eq!(
                unescape_formula_cell(&escaped),
                hostile,
                "aller-retour altéré pour : {trigger:?}"
            );
        }
    }

    /// L'export des labels neutralise ses trois colonnes : `add_label`
    /// accepte n'importe quelle clé, `mac` et `ip` ne sont donc pas plus
    /// sûres que `label`.
    #[test]
    fn label_rows_neutralise_mac_and_ip_columns_too() {
        let path =
            std::env::temp_dir().join(format!("lunar_label_injection_{}.csv", std::process::id()));
        FlowMatrix::write_label_rows_to_csv(
            &[(
                PAYLOAD.to_string(),
                PAYLOAD.to_string(),
                PAYLOAD.to_string(),
            )],
            path.to_str().unwrap(),
        )
        .unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).ok();

        let data_line = content
            .lines()
            .last()
            .expect("le CSV de labels doit porter une ligne de données");
        let executable: Vec<&str> = data_line
            .split(',')
            .map(|cell| cell.trim_matches('"'))
            .filter(|cell| cell.starts_with(FORMULA_TRIGGERS))
            .collect();
        assert!(
            executable.is_empty(),
            "colonnes de labels encore exécutables : {executable:?}\nligne : {data_line}"
        );
    }

    /// Une valeur anodine ne doit pas être décorée : sinon chaque MAC, IP et
    /// date du fichier gagnerait une apostrophe parasite.
    #[test]
    fn harmless_cells_are_left_untouched() {
        let row = FlowMatrixRow {
            mac_source: "aa:bb:cc:dd:ee:ff".to_string(),
            ip_source: "10.0.0.1".to_string(),
            origin: "site-a.csv".to_string(),
            ..row_with_every_text_cell_hostile()
        };
        let escaped = escaped_for_export(&row);
        assert_eq!(escaped.mac_source, "aa:bb:cc:dd:ee:ff");
        assert_eq!(escaped.ip_source, "10.0.0.1");
        assert_eq!(escaped.origin, "site-a.csv");
    }
}
