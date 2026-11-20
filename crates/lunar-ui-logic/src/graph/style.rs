//! Helpers purs de style et de mise en forme du graphe réseau : couleurs,
//! tailles proportionnelles au trafic, labels et courbure des arêtes.
//!
//! Les attributs de rendu sont [`NodeStyle`] et [`EdgeStyle`] ; la géométrie
//! du label d'un nœud est [`node_label_box`].

use lunar_flows_core::graph::{Edge, Node};

use super::protocol_colors::color_for_protocol;
use super::rng::Rng;

// --- Couleurs ---------------------------------------------------------------

/// Couleur RGB 8 bits, prête pour le moteur de rendu.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rgb {
    /// Rouge.
    pub r: u8,
    /// Vert.
    pub g: u8,
    /// Bleu.
    pub b: u8,
}

impl Rgb {
    /// Couleur `#rrggbb` en minuscules.
    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// Lit une couleur `#RRGGBB` ou `#RGB` (voir [`parse_hex`]).
    pub fn from_hex(hex: &str) -> Self {
        parse_hex(hex)
    }
}

/// `parseInt(s, 16)` réduit à ses 32 bits de poids faible (ce que lit
/// `ToInt32` avant les décalages) ; `None` pour `NaN`.
fn parse_int_hex(s: &str) -> Option<u32> {
    let s = s.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{FEFF}');
    let (negative, s) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let s = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    let mut value: u32 = 0;
    let mut any_digit = false;
    for digit in s.chars().map_while(|c| c.to_digit(16)) {
        value = value.wrapping_mul(16).wrapping_add(digit);
        any_digit = true;
    }
    any_digit.then_some(if negative {
        value.wrapping_neg()
    } else {
        value
    })
}

/// Couleur lue : `#RGB` est développé en `#RRGGBB`, puis la valeur
/// hexadécimale est découpée en octets ; une chaîne illisible donne du noir
/// (`parseInt` rend `NaN`, que les décalages ramènent à 0).
pub fn parse_hex(hex: &str) -> Rgb {
    let h = hex.strip_prefix('#').unwrap_or(hex);
    let expanded: String;
    let h = if h.chars().count() == 3 {
        expanded = h.chars().flat_map(|c| [c, c]).collect();
        expanded.as_str()
    } else {
        h
    };
    let v = parse_int_hex(h).unwrap_or(0);
    Rgb {
        r: ((v >> 16) & 255) as u8,
        g: ((v >> 8) & 255) as u8,
        b: (v & 255) as u8,
    }
}

/// Composante tronquée vers zéro puis bornée à l'octet (`Math.trunc`).
fn channel(value: f64) -> u8 {
    // `as` sature : négatif → 0, au-delà de 255 → 255.
    value.trunc() as u8
}

/// Facteur d'assombrissement par défaut de `darken`.
pub const DEFAULT_DARKEN_FACTOR: f64 = 0.2;
/// Facteur d'éclaircissement par défaut de `brighten`.
pub const DEFAULT_BRIGHTEN_FACTOR: f64 = 0.15;

/// Assombrit vers le noir au facteur donné (`darken`).
pub fn darken_rgb(color: Rgb, factor: f64) -> Rgb {
    let scale = |c: u8| channel(f64::from(c) * (1.0 - factor));
    Rgb {
        r: scale(color.r),
        g: scale(color.g),
        b: scale(color.b),
    }
}

/// Éclaircit vers le blanc au facteur donné, sans déborder (`brighten`).
pub fn brighten_rgb(color: Rgb, factor: f64) -> Rgb {
    let scale = |c: u8| channel((f64::from(c) / 255.0 + factor).clamp(0.0, 1.0) * 255.0);
    Rgb {
        r: scale(color.r),
        g: scale(color.g),
        b: scale(color.b),
    }
}

/// `darken(hex, factor)` : couleur `#rrggbb` en minuscules.
pub fn darken(hex: &str, factor: f64) -> String {
    darken_rgb(parse_hex(hex), factor).to_hex()
}

/// `brighten(hex, factor)` : couleur `#rrggbb` en minuscules.
pub fn brighten(hex: &str, factor: f64) -> String {
    brighten_rgb(parse_hex(hex), factor).to_hex()
}

/// Remplissage par défaut d'un nœud sans couleur.
pub const DEFAULT_NODE_COLOR: &str = "#2196F3";

/// Estompage des arêtes pendant la surbrillance d'un tunnel (fond noir).
pub const DIM_EDGE_COLOR: &str = "#2a2a2a";
/// Estompage des nœuds pendant la surbrillance d'un tunnel (fond noir).
pub const DIM_NODE_COLOR: &str = "#3a3a3a";
/// [`DIM_EDGE_COLOR`] décodée.
pub const DIM_EDGE_RGB: Rgb = Rgb {
    r: 0x2a,
    g: 0x2a,
    b: 0x2a,
};
/// [`DIM_NODE_COLOR`] décodée.
pub const DIM_NODE_RGB: Rgb = Rgb {
    r: 0x3a,
    g: 0x3a,
    b: 0x3a,
};

/// Bordure d'alerte : IP observée avec plusieurs MAC (anomalie à investiguer).
pub const MAC_CONFLICT_BORDER_COLOR: &str = "#FF5252";

/// Bordure d'alerte : même IP portée par plusieurs nœuds (VLAN différents).
/// Moins sévère que le conflit de MAC (qui garde priorité) : ambre.
pub const DUPLICATE_IP_BORDER_COLOR: &str = "#FFB300";

/// Bordure d'information : la MAC du nœud porte de nombreuses IP (passerelle
/// probable) — l'IP est vraisemblablement derrière cette MAC. Moins
/// prioritaire que les deux alertes ci-dessus : violet.
pub const GATEWAY_MAC_BORDER_COLOR: &str = "#7E57C2";

/// Facteur d'assombrissement de la bordure ordinaire d'un nœud.
pub const NODE_BORDER_DARKEN_FACTOR: f64 = 0.25;
/// Facteur d'éclaircissement du remplissage d'un nœud survolé.
pub const NODE_HOVER_BRIGHTEN_FACTOR: f64 = 0.18;

/// Épaisseur de la bordure, en fraction du rayon du nœud ; le remplissage
/// occupe le reste.
pub const NODE_BORDER_SIZE_RATIO: f32 = 0.1;

// --- Clés d'arêtes ----------------------------------------------------------

/// Séparateur des composantes d'une clé d'arête.
const EDGE_SEP: &str = "__";

/// Clé d'une arête dans le graphe : `source__cible__protocole`.
pub fn edge_key(source: &str, target: &str, label: &str) -> String {
    let mut key =
        String::with_capacity(source.len() + target.len() + label.len() + 2 * EDGE_SEP.len());
    key.push_str(source);
    key.push_str(EDGE_SEP);
    key.push_str(target);
    key.push_str(EDGE_SEP);
    key.push_str(label);
    key
}

// --- Positionnement ---------------------------------------------------------

/// Rayon par défaut du jitter d'apparition.
pub const DEFAULT_JITTER_RADIUS: f32 = 120.0;

/// Jitter autour d'un point d'ancrage pour les nouveaux nœuds (évite
/// l'empilement) : un point de la couronne `[0.4 r, r]`.
pub fn jitter_around(x: f32, y: f32, radius: f32, rng: &mut Rng) -> (f32, f32) {
    let angle = rng.random_float() * 2.0 * std::f64::consts::PI;
    let r = f64::from(radius) * (0.4 + 0.6 * rng.random_float());
    (
        (f64::from(x) + angle.cos() * r) as f32,
        (f64::from(y) + angle.sin() * r) as f32,
    )
}

// --- Seuils de zoom pour l'affichage des labels d'arêtes (zoom = 1 / ratio) --

/// Zoom à partir duquel le protocole s'affiche sur les arêtes.
pub const EDGE_LABEL_ZOOM: f32 = 1.2;
/// Zoom à partir duquel les ports s'ajoutent au protocole.
pub const PORT_LABEL_ZOOM: f32 = 1.8;

// --- Tailles ----------------------------------------------------------------
// Tailles proportionnelles au trafic, en échelle log : les gros parleurs
// ressortent sans écraser les hôtes discrets.

/// Taille (rayon) minimale d'un nœud.
pub const NODE_SIZE_MIN: f32 = 7.0;
/// Taille (rayon) maximale d'un nœud.
pub const NODE_SIZE_MAX: f32 = 80.0;

/// Taille d'un nœud pour son trafic cumulé : `min(80, 7 + 2·log2(1 + o/2000))`.
pub fn node_size_for(bytes: u64) -> f32 {
    let size = f64::from(NODE_SIZE_MIN) + 2.0 * (1.0 + bytes as f64 / 2000.0).log2();
    size.min(f64::from(NODE_SIZE_MAX)) as f32
}

/// Épaisseur minimale d'une arête.
pub const EDGE_SIZE_MIN: f32 = 1.2;
/// Épaisseur maximale d'une arête.
pub const EDGE_SIZE_MAX: f32 = 7.0;

/// Épaisseur d'une arête pour ses octets : `min(7, 1.2 + 0.55·log2(1 + o/1500))`.
pub fn edge_size_for(bytes: u64) -> f32 {
    let size = f64::from(EDGE_SIZE_MIN) + 0.55 * (1.0 + bytes as f64 / 1500.0).log2();
    size.min(f64::from(EDGE_SIZE_MAX)) as f32
}

/// `n / 1024^puissance` arrondi au demi supérieur à `decimals` décimales,
/// en arithmétique entière : exactement `(n / 1024 ** k).toFixed(d)`, la
/// division par une puissance de deux étant exacte en double.
fn fixed_binary_fraction(bytes: u64, power: u32, decimals: u32) -> String {
    let scale = 10u128.pow(decimals);
    let divisor = 1u128 << (10 * power);
    let scaled = (u128::from(bytes) * scale * 2 + divisor) / (divisor * 2);
    let int_part = scaled / scale;
    let frac_part = scaled % scale;
    format!("{int_part}.{frac_part:0width$}", width = decimals as usize)
}

/// Volume lisible : `o`, `ko`, `Mo` (une décimale) ou `Go` (deux décimales),
/// point décimal comme `toFixed`.
pub fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    if bytes < KIB {
        format!("{bytes} o")
    } else if bytes < KIB.pow(2) {
        format!("{} ko", fixed_binary_fraction(bytes, 1, 1))
    } else if bytes < KIB.pow(3) {
        format!("{} Mo", fixed_binary_fraction(bytes, 2, 1))
    } else {
        format!("{} Go", fixed_binary_fraction(bytes, 3, 2))
    }
}

// --- Courbure des arêtes parallèles (d'après @sigma/edge-curve) ------------

/// Courbure par défaut d'une arête courbe.
pub const DEFAULT_EDGE_CURVATURE: f32 = 0.25;

/// Amplitude de la courbure des arêtes parallèles.
const CURVATURE_AMPLITUDE: f64 = 3.5;

/// Courbure de la `index`-ième arête d'un faisceau d'indice maximal
/// `max_index` ; négative pour un indice négatif (de l'autre côté).
pub fn get_curvature(index: f32, max_index: f32) -> f32 {
    if max_index <= 0.0 {
        return DEFAULT_EDGE_CURVATURE;
    }
    let max_index = f64::from(max_index);
    let max_curvature = CURVATURE_AMPLITUDE
        * (1.0 - (-max_index / CURVATURE_AMPLITUDE).exp())
        * f64::from(DEFAULT_EDGE_CURVATURE);
    (max_curvature * f64::from(index) / max_index) as f32
}

/// Point de contrôle de la courbe de Bézier quadratique d'une arête courbe,
/// comme le shader de `@sigma/edge-curve` : milieu du segment décalé de
/// `courbure × (−dy, dx)`. À calculer dans le même repère que les extrémités
/// (à l'écran, y vers le bas) puis à faire passer par la caméra.
pub fn edge_control_point(source: (f32, f32), target: (f32, f32), curvature: f32) -> (f32, f32) {
    let dx = target.0 - source.0;
    let dy = target.1 - source.1;
    (
        (source.0 + target.0) / 2.0 - dy * curvature,
        (source.1 + target.1) / 2.0 + dx * curvature,
    )
}

// --- Rendu des labels de nœuds ----------------------------------------------

/// Taille de police des labels de nœuds.
pub const NODE_LABEL_SIZE: f32 = 13.0;
/// Couleur du texte des labels de nœuds.
pub const NODE_LABEL_COLOR: &str = "#ffffff";
/// Fond des labels de nœuds (noir à 80 %, `#000000CC`).
pub const NODE_LABEL_BACKGROUND: &str = "#000000CC";
/// Taille de police des labels d'arêtes.
pub const EDGE_LABEL_SIZE: f32 = 11.0;
/// Couleur des labels d'arêtes.
pub const EDGE_LABEL_COLOR: &str = "#E0E0E0";
/// Fond de la zone du graphe.
pub const GRAPH_BACKGROUND: &str = "#000000";

/// Cadre d'un label de nœud, en pixels d'écran.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LabelBox {
    /// Bord gauche du fond.
    pub x: f32,
    /// Bord haut du fond.
    pub y: f32,
    /// Largeur du fond : texte + 10 px.
    pub width: f32,
    /// Hauteur du fond : police + 6 px.
    pub height: f32,
    /// Abscisse du début du texte.
    pub text_x: f32,
    /// Ordonnée de la ligne de base du texte.
    pub text_y: f32,
}

/// Géométrie du label d'un nœud : texte blanc sur fond noir, centré
/// au-dessus du nœud. Sert aussi au survol (un fond blanc serait illisible
/// sur fond noir). Coordonnées d'écran : centre et rayon
/// du nœud déjà passés par la caméra.
pub fn node_label_box(
    center_x: f32,
    center_y: f32,
    node_radius: f32,
    text_width: f32,
    label_size: f32,
) -> LabelBox {
    let width = text_width + 10.0;
    let x = center_x - width / 2.0;
    let y = center_y - node_radius - label_size - 8.0;
    LabelBox {
        x,
        y,
        width,
        height: label_size + 6.0,
        text_x: x + 5.0,
        text_y: y + label_size,
    }
}

// --- Attributs des nœuds et arêtes ------------------------------------------

/// Attributs de style d'un nœud, dérivés du `Node` du cœur.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeStyle {
    /// Nom du nœud, son id à défaut.
    pub name: String,
    /// Première MAC observée.
    pub mac: String,
    /// Toutes les MAC unicast observées.
    pub macs: Vec<String>,
    /// Plusieurs MAC unicast pour une même IP : anomalie (IP partagée, VRRP,
    /// usurpation ARP…) signalée par une bordure d'alerte.
    pub mac_conflict: bool,
    /// IP du nœud (vide pour un nœud L2).
    pub ip: String,
    /// VLAN 802.1Q : il fait partie de la clé du nœud et s'affiche dans le
    /// libellé.
    pub vlan_id: Option<u16>,
    /// Même IP présente sur un autre VLAN (bordure ambre).
    pub duplicate_ip: bool,
    /// MAC partagée par de nombreuses IP (passerelle probable).
    pub gateway_mac: bool,
    /// Label opérateur brut (vide si aucun).
    pub raw_label: String,
    /// Libellé affiché : label, sinon nom, sinon id, suivi de « (VLAN n) ».
    pub label: String,
    /// Couleur de remplissage telle que reçue (`#2196F3` à défaut).
    pub color: String,
    /// Couleur de bordure (alertes, sinon remplissage assombri de 25 %).
    pub border_color: String,
    /// Couleur de remplissage au survol (éclaircie de 0.18).
    pub hover_color: String,
    /// [`Self::color`] décodée.
    pub fill: Rgb,
    /// [`Self::border_color`] décodée.
    pub border: Rgb,
    /// [`Self::hover_color`] décodée.
    pub hover: Rgb,
}

/// Attributs de style d'un nœud.
pub fn node_attributes(node: &Node) -> NodeStyle {
    let color = if node.color.is_empty() {
        DEFAULT_NODE_COLOR.to_string()
    } else {
        node.color.clone()
    };
    let fill = parse_hex(&color);
    let raw_label = node.label.clone().unwrap_or_default();
    let macs = node.macs.clone();
    let mac_conflict = macs.len() > 1;
    let vlan_suffix = node
        .vlan_id
        .map(|id| format!(" (VLAN {id})"))
        .unwrap_or_default();
    let border_color = if mac_conflict {
        MAC_CONFLICT_BORDER_COLOR.to_string()
    } else if node.duplicate_ip {
        DUPLICATE_IP_BORDER_COLOR.to_string()
    } else if node.gateway_mac {
        GATEWAY_MAC_BORDER_COLOR.to_string()
    } else {
        darken_rgb(fill, NODE_BORDER_DARKEN_FACTOR).to_hex()
    };
    let hover = brighten_rgb(fill, NODE_HOVER_BRIGHTEN_FACTOR);
    let base_label = [raw_label.as_str(), node.name.as_str(), node.id.as_str()]
        .into_iter()
        .find(|s| !s.is_empty())
        .unwrap_or("");
    NodeStyle {
        name: if node.name.is_empty() {
            node.id.clone()
        } else {
            node.name.clone()
        },
        mac: node.mac.clone(),
        macs,
        mac_conflict,
        ip: node.ip.clone(),
        vlan_id: node.vlan_id,
        duplicate_ip: node.duplicate_ip,
        gateway_mac: node.gateway_mac,
        label: format!("{base_label}{vlan_suffix}"),
        raw_label,
        border: parse_hex(&border_color),
        border_color,
        hover_color: hover.to_hex(),
        hover,
        fill,
        color,
    }
}

/// Attributs de style d'une arête, dérivés de l'`Edge` du cœur
///.
#[derive(Clone, Debug, PartialEq)]
pub struct EdgeStyle {
    /// Protocole affiché (label de l'arête).
    pub protocol: String,
    /// Port source du premier paquet.
    pub source_port: Option<u16>,
    /// Port destination du premier paquet.
    pub destination_port: Option<u16>,
    /// Tous les ports « service » observés sur l'arête (triés, plafonnés par
    /// le cœur) : plusieurs services entre deux équipements ne sont plus
    /// masqués par le premier couple de ports. Les ports éphémères ne sont
    /// pas listés : `has_dynamic_ports` les signale (rendu « … »).
    pub ports: Vec<u16>,
    /// Trafic observé sur des ports dynamiques uniquement.
    pub has_dynamic_ports: bool,
    /// Trafic observé dans les deux sens.
    pub bidir: bool,
    /// Paquets cumulés.
    pub count: u64,
    /// Octets cumulés.
    pub total_bytes: u64,
    /// Tunnels (encap_id hex) auxquels ce flux participe : le cœur envoie la
    /// liste cumulative, on la remplace donc à chaque update.
    pub encap_ids: Vec<String>,
    /// Couleur du protocole.
    pub color: &'static str,
    /// [`Self::color`] décodée.
    pub rgb: Rgb,
    /// Épaisseur proportionnelle (log) aux octets.
    pub size: f32,
}

impl EdgeStyle {
    /// Champs lus par [`edge_label_for`].
    pub fn label_source(&self) -> EdgeLabelSource<'_> {
        EdgeLabelSource {
            protocol: &self.protocol,
            ports: &self.ports,
            has_dynamic_ports: self.has_dynamic_ports,
            source_port: self.source_port,
            destination_port: self.destination_port,
        }
    }
}

/// Attributs de style d'une arête.
pub fn edge_attributes(edge: &Edge) -> EdgeStyle {
    let color = color_for_protocol(Some(&edge.label));
    EdgeStyle {
        protocol: edge.label.clone(),
        source_port: edge.source_port,
        destination_port: edge.destination_port,
        ports: edge.ports.iter().copied().collect(),
        has_dynamic_ports: edge.has_dynamic_ports,
        bidir: edge.bidir,
        count: edge.count,
        total_bytes: edge.total_bytes,
        encap_ids: edge.encap_ids.clone(),
        color,
        rgb: parse_hex(color),
        size: edge_size_for(edge.total_bytes),
    }
}

// --- Libellé d'arête rendu -------------------------------------------------

/// Champs d'une arête utiles à son libellé.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EdgeLabelSource<'a> {
    /// Protocole.
    pub protocol: &'a str,
    /// Ports « service » triés.
    pub ports: &'a [u16],
    /// Trafic sur ports dynamiques.
    pub has_dynamic_ports: bool,
    /// Port source du premier paquet.
    pub source_port: Option<u16>,
    /// Port destination du premier paquet.
    pub destination_port: Option<u16>,
}

/// Libellé d'arête affiché par le reducer : protocole, plus le détail des
/// ports si affiché à ce niveau de zoom.
pub fn edge_label_for(data: EdgeLabelSource<'_>, port_labels_shown: bool) -> String {
    let protocol = data.protocol;
    if !port_labels_shown {
        return protocol.to_string();
    }
    if !data.ports.is_empty() {
        // Ports « service » de l'arête (les éphémères ne sont pas listés, le
        // cœur les résume par has_dynamic_ports → « … »).
        let ports: Vec<String> = data.ports.iter().map(u16::to_string).collect();
        let dynamic = if data.has_dynamic_ports { ",…" } else { "" };
        return format!("{protocol} :{}{dynamic}", ports.join(","));
    }
    if data.has_dynamic_ports {
        // Uniquement du trafic sur ports dynamiques : signalé sans liste.
        return format!("{protocol} :…");
    }
    if data.source_port.is_some() || data.destination_port.is_some() {
        let port = |p: Option<u16>| p.map(|p| p.to_string()).unwrap_or_default();
        return format!(
            "{protocol} {}→{}",
            port(data.source_port),
            port(data.destination_port)
        );
    }
    protocol.to_string()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// Nœud complet à défauts neutres.
    pub(crate) fn node(id: &str) -> Node {
        Node {
            id: id.to_string(),
            name: id.to_string(),
            color: "#2196F3".to_string(),
            mac: String::new(),
            macs: Vec::new(),
            ip: String::new(),
            vlan_id: None,
            duplicate_ip: false,
            gateway_mac: false,
            label: None,
        }
    }

    /// Arête complète à défauts neutres.
    pub(crate) fn edge(source: &str, target: &str, label: &str) -> Edge {
        Edge {
            id: "e".to_string(),
            source: source.to_string(),
            target: target.to_string(),
            label: label.to_string(),
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

    // --- Couleurs ------------------------------------------------------------

    #[test]
    fn darken_assombrit_vers_le_noir_au_facteur_donne() {
        assert_eq!(darken("#2196F3", 0.25), "#1870b6");
    }

    #[test]
    fn darken_le_noir_reste_noir() {
        assert_eq!(darken("#000000", DEFAULT_DARKEN_FACTOR), "#000000");
    }

    #[test]
    fn darken_facteur_par_defaut_sur_blanc() {
        assert_eq!(darken("#ffffff", DEFAULT_DARKEN_FACTOR), "#cccccc");
    }

    #[test]
    fn brighten_eclaircit_vers_le_blanc_au_facteur_donne() {
        assert_eq!(brighten("#2196F3", DEFAULT_BRIGHTEN_FACTOR), "#47bcff");
    }

    #[test]
    fn brighten_clampe_a_blanc_plutot_que_de_deborder() {
        assert_eq!(brighten("#ffffff", 0.5), "#ffffff");
    }

    #[test]
    fn brighten_le_noir_s_eclaircit_sans_deborder() {
        assert_eq!(brighten("#000000", 0.5), "#7f7f7f");
    }

    #[test]
    fn couleurs_d_estompage_decodees() {
        assert_eq!(parse_hex(DIM_EDGE_COLOR), DIM_EDGE_RGB);
        assert_eq!(parse_hex(DIM_NODE_COLOR), DIM_NODE_RGB);
    }

    #[test]
    fn parse_hex_comme_hex_to_rgb() {
        assert_eq!(parse_hex("#f00"), Rgb { r: 255, g: 0, b: 0 });
        assert_eq!(
            parse_hex("2196F3"),
            Rgb {
                r: 0x21,
                g: 0x96,
                b: 0xF3
            }
        );
        // parseInt s'arrête au premier caractère non hexadécimal.
        assert_eq!(
            parse_hex("#12zz34"),
            Rgb {
                r: 0,
                g: 0,
                b: 0x12
            }
        );
        // NaN → noir.
        assert_eq!(parse_hex("rouge"), Rgb::default());
        assert_eq!(parse_hex(""), Rgb::default());
    }

    // --- Clé d'arête ---------------------------------------------------------

    #[test]
    fn edge_key_concatene_source_cible_et_protocole() {
        assert_eq!(edge_key("a", "b", "TCP"), "a__b__TCP");
    }

    // --- Positionnement ------------------------------------------------------

    #[test]
    fn jitter_around_reste_dans_la_couronne_autour_du_point() {
        let mut rng = Rng::with_seed(1);
        for _ in 0..50 {
            let (x, y) = jitter_around(100.0, 200.0, 120.0, &mut rng);
            let dist = f64::from(x - 100.0).hypot(f64::from(y - 200.0));
            // Tolérance de l'arrondi f32 des coordonnées.
            assert!(
                (120.0 * 0.4 - 1e-3..=120.0 + 1e-3).contains(&dist),
                "dist={dist}"
            );
        }
    }

    // --- Tailles proportionnelles au trafic ----------------------------------

    #[test]
    fn node_size_for_trafic_nul_donne_la_taille_minimale() {
        assert_eq!(node_size_for(0), NODE_SIZE_MIN);
    }

    #[test]
    fn node_size_for_distingue_les_gros_volumes_puis_plafonne() {
        let small = node_size_for(1_000);
        let big = node_size_for(1_000_000);
        let very_big = node_size_for(1_000_000_000);
        let huge = node_size_for(1_000_000_000_000_000_000);

        assert!(small < big, "small={small} big={big}");
        assert!(big < very_big, "big={big} veryBig={very_big}");
        assert!(very_big > 40.0, "veryBig={very_big}");
        assert_eq!(huge, NODE_SIZE_MAX);
    }

    #[test]
    fn edge_size_for_trafic_nul_minimal_plafonne_a_7() {
        assert_eq!(edge_size_for(0), 1.2);
        assert_eq!(edge_size_for(1_000_000_000_000), 7.0);
    }

    // --- Formatage des octets ------------------------------------------------

    #[test]
    fn format_bytes_sous_1024_octets() {
        assert_eq!(format_bytes(0), "0 o");
        assert_eq!(format_bytes(1023), "1023 o");
    }

    #[test]
    fn format_bytes_kilo_octets() {
        assert_eq!(format_bytes(1024), "1.0 ko");
        assert_eq!(format_bytes(2048), "2.0 ko");
    }

    #[test]
    fn format_bytes_mega_octets() {
        assert_eq!(format_bytes(1024 * 1024), "1.0 Mo");
    }

    #[test]
    fn format_bytes_giga_octets_deux_decimales() {
        assert_eq!(format_bytes(2_684_354_560), "2.50 Go");
    }

    #[test]
    fn format_bytes_arrondit_au_demi_superieur_comme_to_fixed() {
        // 1280 / 1024 = 1.25 exactement : toFixed(1) donne « 1.3 ».
        assert_eq!(format_bytes(1280), "1.3 ko");
        assert_eq!(format_bytes(1024 * 1023 + 1023), "1024.0 ko");
    }

    // --- Courbure des arêtes parallèles ---------------------------------------

    #[test]
    fn get_curvature_une_seule_arete_donne_la_courbure_par_defaut() {
        assert_eq!(get_curvature(0.0, 0.0), DEFAULT_EDGE_CURVATURE);
    }

    #[test]
    fn get_curvature_croit_avec_l_index_sous_la_courbure_max() {
        let c1 = get_curvature(1.0, 3.0);
        let c2 = get_curvature(2.0, 3.0);
        let c3 = get_curvature(3.0, 3.0);
        assert!(c1 < c2 && c2 < c3, "c1={c1} c2={c2} c3={c3}");
        assert!(c3 < 3.5 * DEFAULT_EDGE_CURVATURE);
    }

    // --- Attributs ------------------------------------------------------------

    #[test]
    fn node_attributes_couleur_par_defaut_label_retombe_sur_le_nom() {
        let mut n = node("n1");
        n.name = "hote-a".to_string();
        n.color = String::new();
        let attrs = node_attributes(&n);
        assert_eq!(attrs.color, "#2196F3");
        assert_eq!(attrs.label, "hote-a");
        assert_eq!(attrs.raw_label, "");
        assert!(!attrs.mac_conflict);
    }

    #[test]
    fn node_attributes_plusieurs_mac_conflit_signale_par_la_bordure() {
        let mut n = node("n");
        n.macs = vec!["aa:bb:cc:dd:ee:ff".into(), "11:22:33:44:55:66".into()];
        let attrs = node_attributes(&n);
        assert!(attrs.mac_conflict);
        assert_eq!(attrs.border_color, MAC_CONFLICT_BORDER_COLOR);
    }

    #[test]
    fn node_attributes_vlan_dans_le_libelle_et_en_attribut() {
        let mut n = node("n");
        n.name = "192.168.1.10".to_string();
        n.vlan_id = Some(42);
        let attrs = node_attributes(&n);
        assert_eq!(attrs.vlan_id, Some(42));
        assert_eq!(attrs.label, "192.168.1.10 (VLAN 42)");

        n.vlan_id = None;
        let untagged = node_attributes(&n);
        assert_eq!(untagged.vlan_id, None);
        assert_eq!(untagged.label, "192.168.1.10");
    }

    #[test]
    fn node_attributes_mac_passerelle_bordure_violette_derriere_les_alertes() {
        let mut n = node("n");
        n.gateway_mac = true;
        let attrs = node_attributes(&n);
        assert!(attrs.gateway_mac);
        assert_eq!(attrs.border_color, GATEWAY_MAC_BORDER_COLOR);

        let mut with_duplicate = n.clone();
        with_duplicate.duplicate_ip = true;
        assert_eq!(
            node_attributes(&with_duplicate).border_color,
            DUPLICATE_IP_BORDER_COLOR
        );
        let mut with_conflict = n.clone();
        with_conflict.macs = vec!["aa:bb:cc:dd:ee:ff".into(), "11:22:33:44:55:66".into()];
        assert_eq!(
            node_attributes(&with_conflict).border_color,
            MAC_CONFLICT_BORDER_COLOR
        );
        assert!(!node_attributes(&node("n")).gateway_mac);
    }

    #[test]
    fn node_attributes_ip_dupliquee_bordure_ambre_le_conflit_mac_prime() {
        let mut n = node("n");
        n.duplicate_ip = true;
        let attrs = node_attributes(&n);
        assert!(attrs.duplicate_ip);
        assert_eq!(attrs.border_color, DUPLICATE_IP_BORDER_COLOR);

        n.macs = vec!["aa:bb:cc:dd:ee:ff".into(), "11:22:33:44:55:66".into()];
        assert_eq!(node_attributes(&n).border_color, MAC_CONFLICT_BORDER_COLOR);
    }

    #[test]
    fn node_attributes_bordure_et_survol_derives_du_remplissage() {
        let attrs = node_attributes(&node("n"));
        assert_eq!(attrs.border_color, "#1870b6");
        assert_eq!(attrs.hover_color, brighten("#2196F3", 0.18));
        assert_eq!(attrs.border, parse_hex("#1870b6"));
        // Label et nom retombent sur l'id quand le nom est vide.
        let mut anonymous = node("id-1");
        anonymous.name = String::new();
        let attrs = node_attributes(&anonymous);
        assert_eq!(attrs.name, "id-1");
        assert_eq!(attrs.label, "id-1");
    }

    #[test]
    fn edge_attributes_champs_par_defaut_sur_une_arete_minimale() {
        let attrs = edge_attributes(&edge("a", "b", "TCP"));
        assert_eq!(attrs.protocol, "TCP");
        assert!(attrs.ports.is_empty());
        assert!(!attrs.has_dynamic_ports);
        assert!(!attrs.bidir);
        assert!(attrs.encap_ids.is_empty());
        assert_eq!(attrs.size, edge_size_for(0));
        assert_eq!(attrs.color, "#2563EB");
    }

    #[test]
    fn edge_attributes_transmet_ports_et_tunnels_tels_quels() {
        let mut e = edge("a", "b", "TCP");
        e.ports = BTreeSet::from([80, 443]);
        e.has_dynamic_ports = true;
        e.encap_ids = vec!["enc-1".to_string()];
        e.total_bytes = 12345;
        let attrs = edge_attributes(&e);
        assert_eq!(attrs.ports, vec![80, 443]);
        assert!(attrs.has_dynamic_ports);
        assert_eq!(attrs.encap_ids, vec!["enc-1".to_string()]);
        assert_eq!(attrs.total_bytes, 12345);
    }

    // --- Libellé d'arête ------------------------------------------------------

    #[test]
    fn edge_label_for_ports_masques_seulement_le_protocole() {
        let src = EdgeLabelSource {
            protocol: "TCP",
            ports: &[80],
            ..Default::default()
        };
        assert_eq!(edge_label_for(src, false), "TCP");
    }

    #[test]
    fn edge_label_for_liste_de_ports_protocole_et_ports_tries() {
        let src = EdgeLabelSource {
            protocol: "TCP",
            ports: &[80, 443],
            ..Default::default()
        };
        assert_eq!(edge_label_for(src, true), "TCP :80,443");
    }

    #[test]
    fn edge_label_for_ports_dynamiques_meles_suffixe_points() {
        let src = EdgeLabelSource {
            protocol: "TCP",
            ports: &[80],
            has_dynamic_ports: true,
            ..Default::default()
        };
        assert_eq!(edge_label_for(src, true), "TCP :80,…");
    }

    #[test]
    fn edge_label_for_uniquement_dynamiques_pas_de_liste() {
        let src = EdgeLabelSource {
            protocol: "TCP",
            has_dynamic_ports: true,
            ..Default::default()
        };
        assert_eq!(edge_label_for(src, true), "TCP :…");
    }

    #[test]
    fn edge_label_for_ports_source_destination_sans_liste_fleche() {
        let src = EdgeLabelSource {
            protocol: "TCP",
            source_port: Some(51234),
            destination_port: Some(443),
            ..Default::default()
        };
        assert_eq!(edge_label_for(src, true), "TCP 51234→443");
        let only_destination = EdgeLabelSource {
            protocol: "UDP",
            destination_port: Some(53),
            ..Default::default()
        };
        assert_eq!(edge_label_for(only_destination, true), "UDP →53");
    }

    #[test]
    fn edge_label_for_rien_a_montrer_seulement_le_protocole() {
        let src = EdgeLabelSource {
            protocol: "ICMP",
            ..Default::default()
        };
        assert_eq!(edge_label_for(src, true), "ICMP");
    }

    // --- Géométrie -------------------------------------------------------------

    #[test]
    fn node_label_box_centre_au_dessus_du_noeud() {
        let b = node_label_box(100.0, 200.0, 10.0, 50.0, 13.0);
        assert_eq!(
            b,
            LabelBox {
                x: 70.0,
                y: 169.0,
                width: 60.0,
                height: 19.0,
                text_x: 75.0,
                text_y: 182.0
            }
        );
    }

    #[test]
    fn edge_control_point_decale_le_milieu_perpendiculairement() {
        assert_eq!(
            edge_control_point((0.0, 0.0), (10.0, 0.0), 0.25),
            (5.0, 2.5)
        );
        assert_eq!(edge_control_point((0.0, 0.0), (10.0, 0.0), 0.0), (5.0, 0.0));
    }
}
