//! Couleurs des protocoles dans le graphe réseau : palette fixe pour les
//! protocoles connus, couleur dérivée par hachage stable pour les autres.
//! Alimente aussi la légende du graphe.

use super::format::js_trim as trim_js;

/// Couleur des protocoles absents ou inconnus.
pub const UNKNOWN_PROTOCOL_COLOR: &str = "#94A3B8";

/// Palette de repli, indexée par le hachage de la clé du protocole.
pub const HASH_FALLBACK_COLORS: [&str; 10] = [
    "#22D3EE", "#A78BFA", "#34D399", "#F472B6", "#FBBF24", "#60A5FA", "#FB7185", "#C084FC",
    "#2DD4BF", "#F97316",
];

/// Couleur exacte d'une clé normalisée (voir [`protocol_color_key`]).
///
/// Les labels émis par packet_parser sont normalisés : « EtherNet/IP »,
/// « OPC UA », « Q-in-Q » ou « MPLS Unicast » restent des clés stables.
fn exact_color(key: &str) -> Option<&'static str> {
    Some(match key {
        // Protocoles applicatifs
        "http" => "#0EA5E9",
        "dhcp" => "#FBBF24",
        "dhcpv6" => "#F59E0B",
        "ntp" => "#F59E0B",
        "ams" => "#A855F7",
        "bitcoin" => "#F7931A",
        "opcua" => "#7C3AED",
        // ASTERIX (surveillance aérienne) : bleu ciel, à l'écart des teintes OT.
        "asterix" => "#0369A1",
        "ethernetip" => "#D97706",
        "dns" => "#EF4444",
        "mqtt" => "#22C55E",
        "postgresql" => "#38BDF8",
        "snmp" => "#10B981",
        "tls" => "#3B82F6",
        "s7comm" => "#E11D48",
        "cotp" => "#F97316",
        "copt" => "#F97316",
        "giop" => "#8B5CF6",
        "srvloc" => "#14B8A6",
        "srvlock" => "#14B8A6",
        "modbustcp" => "#DC2626",
        // Même protocole sur UDP : même teinte.
        "modbusudp" => "#DC2626",
        // UMAS (Schneider) voyage dans une enveloppe Modbus et est reconnu avant
        // elle : teinte voisine du Modbus, distincte pour que la bascule se voie.
        "umas" => "#B91C1C",
        "quiq" => "#06B6D4",
        "quic" => "#06B6D4",

        // Protocoles Internet et liaison / Ethertypes
        "arp" => "#FACC15",
        "rarp" => "#FACC15",
        "aarp" => "#FDBA74",
        "ipv4" => "#F97316",
        "minipv4" => "#FB923C",
        "ipv6" => "#A855F7",
        "profinet" => "#22C55E",
        "profinetrt" => "#22C55E",
        "vlantaggedframe" => "#84CC16",
        "qinq" => "#A3E635",
        "pbridge" => "#65A30D",
        "lldp" => "#38BDF8",
        "mrp" => "#4ADE80",
        "ptp" => "#F59E0B",
        "trill" => "#2DD4BF",
        "decnet" => "#A78BFA",
        "appletalk" => "#FB7185",
        "ipx" => "#C084FC",
        "qnx" => "#94A3B8",
        "cobtranet" => "#34D399",
        "cfm" => "#9CA3AF",
        "mplsunicast" => "#60A5FA",
        "mplsmulticast" => "#818CF8",
        "pppoediscoverystage" => "#F472B6",
        "pppoesessionstage" => "#EC4899",

        // Transport courant / en-têtes suivants IPv6
        "tcp" => "#2563EB",
        "udp" => "#0EA5E9",
        "udplite" => "#38BDF8",
        "icmp" => "#F43F5E",
        "icmpv6" => "#FB7185",
        "igmp" => "#A3E635",
        "pim" => "#84CC16",
        "vrrp" => "#EAB308",
        "egp" => "#84CC16",
        "igp" => "#84CC16",
        "ospfigp" => "#84CC16",
        "eigrp" => "#84CC16",
        "gre" => "#64748B",
        "ipip" => "#7DD3FC",
        "etherip" => "#7DD3FC",
        "encap" => "#7DD3FC",
        "l2tp" => "#7DD3FC",
        "mplsinip" => "#60A5FA",
        "esp" => "#6366F1",
        "ah" => "#8B5CF6",
        "ipcomp" => "#A78BFA",
        "sctp" => "#06B6D4",
        "dccp" => "#0284C7",
        "rsvp" => "#F59E0B",
        "rsvpe2eignore" => "#F59E0B",
        _ => return None,
    })
}

/// Couleur IPv6, partagée par ses en-têtes d'extension.
const IPV6_COLOR: &str = "#A855F7";

/// En-têtes d'extension IPv6, colorés comme IPv6.
fn is_ipv6_extension(key: &str) -> bool {
    matches!(
        key,
        "ipv6hopbyhop"
            | "ipv6routingheader"
            | "ipv6fragmentheader"
            | "ipv6destinationoptions"
            | "ipv6mobilityheader"
            | "nonextheader"
            | "hopopt"
    )
}

/// Protocoles de routage, colorés en vert.
fn is_routing_protocol(key: &str) -> bool {
    matches!(
        key,
        "bbnrccmon"
            | "cbt"
            | "dcnmeas"
            | "dsr"
            | "idpr"
            | "idprcmtp"
            | "idrp"
            | "isisoveripv4"
            | "larp"
            | "manet"
            | "mtp"
            | "narp"
            | "pnni"
            | "sdrp"
            | "sunnd"
            | "xnsidp"
    )
}

/// Protocoles de sécurité, colorés en violet.
fn is_security_protocol(key: &str) -> bool {
    matches!(
        key,
        "anyprivateencryptionscheme"
            | "hip"
            | "rohc"
            | "securevmtp"
            | "shim6"
            | "skip"
            | "tlsp"
            | "wesp"
    )
}

/// Clé normalisée d'un label : minuscules, sans ce qui n'est ni lettre ASCII
/// ni chiffre (`"EtherNet/IP"` → `"ethernetip"`).
pub fn protocol_color_key(label: &str) -> String {
    trim_js(label)
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        .collect()
}

/// Couleur de repli stable : hachage 32 bits signé façon `String.hashCode`
/// de Java (rebouclage volontaire), modulo la palette.
fn fallback_color_for_key(key: &str) -> &'static str {
    let mut hash: i32 = 0;
    // La clé est ASCII : ses octets sont ses unités UTF-16.
    for unit in key.encode_utf16() {
        hash = hash.wrapping_mul(31).wrapping_add(i32::from(unit));
    }
    let index = (hash.unsigned_abs() as usize) % HASH_FALLBACK_COLORS.len();
    HASH_FALLBACK_COLORS[index]
}

/// Couleur d'un protocole d'après son label (`None` ou vide : inconnu).
pub fn color_for_protocol(label: Option<&str>) -> &'static str {
    let raw = label.map(trim_js).unwrap_or("");
    if raw.is_empty() {
        return UNKNOWN_PROTOCOL_COLOR;
    }

    let key = protocol_color_key(raw);
    if key.is_empty() || key.starts_with("unknown") {
        return UNKNOWN_PROTOCOL_COLOR;
    }

    if let Some(exact) = exact_color(&key) {
        return exact;
    }
    if is_ipv6_extension(&key) || key.starts_with("ipv6") {
        return IPV6_COLOR;
    }
    if is_routing_protocol(&key) {
        return "#84CC16";
    }
    if is_security_protocol(&key) {
        return "#8B5CF6";
    }
    if key.contains("mpls") {
        return "#60A5FA";
    }
    if key.contains("ip") && (key.contains("inip") || key.contains("over")) {
        return "#7DD3FC";
    }

    fallback_color_for_key(&key)
}

/// Entrée de la légende des couleurs de protocoles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProtocolColorLegendItem {
    /// Libellé affiché dans la légende.
    pub label: &'static str,
    /// Couleur de l'arête correspondante.
    pub color: &'static str,
}

/// Libellé de légende et label de protocole dont il prend la couleur.
const PROTOCOL_LEGEND_SOURCES: [(&str, Option<&str>); 16] = [
    ("TCP", Some("Tcp")),
    ("UDP", Some("UDP")),
    ("DNS", Some("DNS")),
    ("TLS", Some("TLS")),
    ("QUIC", Some("QUIQ")),
    ("ARP", Some("ARP")),
    ("IPv4", Some("IPv4")),
    ("IPv6", Some("IPv6")),
    ("Profinet", Some("Profinet")),
    ("OPC UA", Some("OPC UA")),
    ("S7Comm", Some("S7Comm")),
    ("ModbusTCP", Some("ModbusTCP")),
    ("UMAS", Some("UMAS")),
    ("LLDP", Some("LLDP")),
    ("Tunnel", Some("GRE")),
    ("Inconnu", None),
];

/// Légende des couleurs de protocoles (`PROTOCOL_COLOR_LEGEND`), calculée
/// par [`color_for_protocol`] comme le rendu.
pub fn protocol_color_legend() -> [ProtocolColorLegendItem; 16] {
    PROTOCOL_LEGEND_SOURCES.map(|(label, source)| ProtocolColorLegendItem {
        label,
        color: color_for_protocol(source),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn color(label: &str) -> &'static str {
        color_for_protocol(Some(label))
    }

    #[test]
    fn correspondance_exacte_insensible_a_la_casse() {
        assert_eq!(color("tcp"), color("TCP"));
        assert_eq!(color("TCP"), "#2563EB");
    }

    #[test]
    fn normalise_ponctuation_et_espaces() {
        assert_eq!(color("EtherNet/IP"), color("ethernetip"));
        assert_eq!(color("OPC UA"), color("opcua"));
    }

    #[test]
    fn vide_ou_absent_donne_la_couleur_inconnue() {
        assert_eq!(color(""), UNKNOWN_PROTOCOL_COLOR);
        // Protocole absent : None.
        assert_eq!(color_for_protocol(None), UNKNOWN_PROTOCOL_COLOR);
        assert_eq!(color("   "), UNKNOWN_PROTOCOL_COLOR);
    }

    #[test]
    fn unknown_et_variantes_donnent_la_couleur_inconnue() {
        assert_eq!(color("Unknown"), UNKNOWN_PROTOCOL_COLOR);
        assert_eq!(color("UnknownProtocol"), UNKNOWN_PROTOCOL_COLOR);
    }

    #[test]
    fn extensions_ipv6_retombent_sur_la_couleur_ipv6() {
        let ipv6_color = color("IPv6");
        assert_eq!(color("IPv6HopByHop"), ipv6_color);
        assert_eq!(color("HopOpt"), ipv6_color);
        assert_eq!(color("ipv6-quelquechose-inconnu"), ipv6_color);
    }

    #[test]
    fn protocoles_de_routage_connus() {
        assert_eq!(color("DSR"), "#84CC16");
        assert_eq!(color("PNNI"), "#84CC16");
    }

    #[test]
    fn protocoles_de_securite_connus() {
        assert_eq!(color("HIP"), "#8B5CF6");
        assert_eq!(color("SKIP"), "#8B5CF6");
    }

    #[test]
    fn heuristique_mpls() {
        assert_eq!(color("MplsInIp"), "#60A5FA");
    }

    #[test]
    fn heuristique_encapsulation_ip_in_ip_ou_over() {
        assert_eq!(color("SomethingOverIp"), "#7DD3FC");
    }

    #[test]
    fn protocole_totalement_inconnu_fallback_par_hachage_stable() {
        let a = color("ProtocoleMaisonXYZ");
        let b = color("ProtocoleMaisonXYZ");
        assert_eq!(a, b, "même entrée -> même couleur (déterministe)");

        let c = color("AutreProtocoleInconnu123");
        // Pas de garantie de distinction (collisions possibles), mais la
        // couleur reste dans la palette de repli, jamais la couleur « inconnu ».
        assert_ne!(a, UNKNOWN_PROTOCOL_COLOR);
        assert_ne!(c, UNKNOWN_PROTOCOL_COLOR);
        assert!(HASH_FALLBACK_COLORS.contains(&a));
        assert!(HASH_FALLBACK_COLORS.contains(&c));
    }

    #[test]
    fn hachage_identique_a_string_hashcode() {
        // Hachage de référence : `(hash * 31 + code) | 0` sur des entiers
        // signés 32 bits, puis `Math.abs(hash) % 10`.
        // "abc".hashCode() = 96354 → index 4.
        assert_eq!(fallback_color_for_key("abc"), HASH_FALLBACK_COLORS[4]);
        // Débordement 32 bits signé : "polygenelubricants" = i32::MIN, dont
        // Math.abs vaut 2147483648 → index 8 (sans panique en Rust).
        assert_eq!(
            fallback_color_for_key("polygenelubricants"),
            HASH_FALLBACK_COLORS[8]
        );
    }

    #[test]
    fn legende_toutes_les_entrees_ont_une_couleur_definie() {
        let legend = protocol_color_legend();
        assert!(!legend.is_empty());
        for item in legend {
            assert!(
                item.color.starts_with('#'),
                "{}: {}",
                item.label,
                item.color
            );
        }
        assert_eq!(
            legend[4],
            ProtocolColorLegendItem {
                label: "QUIC",
                color: "#06B6D4"
            }
        );
        assert_eq!(legend[15].color, UNKNOWN_PROTOCOL_COLOR);
    }
}
