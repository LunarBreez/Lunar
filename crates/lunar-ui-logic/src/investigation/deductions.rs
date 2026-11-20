//! Déductions à partir de registres publics : préfixe OUI d'une MAC →
//! fabricant, adresses IP/MAC extrêmement répandues → rôle connu. Comme pour
//! les ports (cf. [`super::ports`]), ce sont des **indications probables**,
//! pas des preuves — une MAC peut être aléatoire (bit « localement
//! administré »), une IP publique peut être réattribuée. LUNAR ne conclut
//! pas ; il rapproche l'observé de ce qui est notoire, et laisse l'humain
//! trancher.
//!
//! Les listes sont volontairement **curées** (les cas « extrêmement
//! connus »), pas exhaustives : mieux vaut ne rien déduire que déduire faux.

use super::locale::js_string_to_number;

/// Préfixe OUI (3 premiers octets, MAJUSCULE « XX:XX:XX ») → fabricant.
/// Sous-ensemble notoire, orienté IT et OT/ICS.
fn oui_vendor_of(oui: &str) -> &'static str {
    match oui {
        // Virtualisation / cloud
        "00:05:69" => "VMware",
        "00:0C:29" => "VMware",
        "00:1C:14" => "VMware",
        "00:50:56" => "VMware",
        "00:15:5D" => "Microsoft (Hyper-V)",
        "00:16:3E" => "Xen",
        "52:54:00" => "QEMU / KVM",
        "08:00:27" => "VirtualBox",
        // Réseau
        "00:00:0C" => "Cisco",
        "00:1A:A1" => "Cisco",
        "00:25:B4" => "Cisco",
        "00:15:6D" => "Ubiquiti",
        "24:5A:4C" => "Ubiquiti",
        "78:8A:20" => "Ubiquiti",
        "FC:EC:DA" => "Ubiquiti",
        "00:14:6C" => "Netgear",
        "9C:D3:6D" => "Netgear",
        "00:27:19" => "TP-Link",
        "50:C7:BF" => "TP-Link",
        "EC:08:6B" => "TP-Link",
        "00:18:82" => "Huawei",
        "48:46:FB" => "Huawei",
        // Postes / serveurs
        "00:1B:21" => "Intel",
        "3C:97:0E" => "Intel",
        "8C:16:45" => "Intel",
        "00:14:22" => "Dell",
        "18:03:73" => "Dell",
        "F8:BC:12" => "Dell",
        "00:1B:78" => "HP / HPE",
        "3C:D9:2B" => "HP / HPE",
        "94:57:A5" => "HP / HPE",
        // Apple
        "00:1E:C2" => "Apple",
        "3C:07:54" => "Apple",
        "A4:5E:60" => "Apple",
        "F0:18:98" => "Apple",
        "DC:A9:04" => "Apple",
        "88:66:5A" => "Apple",
        "AC:BC:32" => "Apple",
        // IoT / cartes
        "B8:27:EB" => "Raspberry Pi",
        "DC:A6:32" => "Raspberry Pi",
        "E4:5F:01" => "Raspberry Pi",
        "28:CD:C1" => "Raspberry Pi",
        "24:0A:C4" => "Espressif (ESP32/8266)",
        "30:AE:A4" => "Espressif (ESP32/8266)",
        "5C:CF:7F" => "Espressif (ESP32/8266)",
        "84:F3:EB" => "Espressif (ESP32/8266)",
        "A0:20:A6" => "Espressif (ESP32/8266)",
        "B4:E6:2D" => "Espressif (ESP32/8266)",
        // Grand public
        "00:12:FB" => "Samsung",
        "5C:0A:5B" => "Samsung",
        "34:D2:70" => "Amazon",
        "68:37:E9" => "Amazon",
        "3C:5A:B4" => "Google / Nest",
        "F4:F5:D8" => "Google / Nest",
        // Automatismes / OT-ICS
        "00:0E:8C" => "Siemens",
        "00:1B:1B" => "Siemens",
        "00:1F:F8" => "Siemens",
        "08:00:06" => "Siemens",
        "AC:64:17" => "Siemens",
        "00:00:BC" => "Rockwell / Allen-Bradley",
        "00:1D:9C" => "Rockwell / Allen-Bradley",
        "5C:88:16" => "Rockwell / Allen-Bradley",
        "00:00:54" => "Schneider Electric (Modicon)",
        "00:80:F4" => "Schneider Electric (Telemecanique)",
        "00:A0:45" => "Phoenix Contact",
        "00:01:05" => "Beckhoff",
        "00:30:DE" => "WAGO",
        "00:90:E8" => "Moxa",
        "00:80:63" => "Hirschmann",
        _ => "",
    }
}

/// Adresses IPv4/IPv6 exactes extrêmement connues → rôle.
fn known_ip_exact(ip: &str) -> &'static str {
    match ip {
        "8.8.8.8" => "Google Public DNS",
        "8.8.4.4" => "Google Public DNS",
        "1.1.1.1" => "Cloudflare DNS",
        "1.0.0.1" => "Cloudflare DNS",
        "9.9.9.9" => "Quad9 DNS",
        "149.112.112.112" => "Quad9 DNS",
        "208.67.222.222" => "OpenDNS (Cisco Umbrella)",
        "208.67.220.220" => "OpenDNS (Cisco Umbrella)",
        "4.2.2.1" => "Level3 DNS",
        "4.2.2.2" => "Level3 DNS",
        "255.255.255.255" => "Broadcast limité",
        "0.0.0.0" => "Adresse non spécifiée",
        "224.0.0.1" => "Multicast — tous les hôtes",
        "224.0.0.2" => "Multicast — tous les routeurs",
        "224.0.0.5" => "OSPF",
        "224.0.0.6" => "OSPF (DR)",
        "224.0.0.9" => "RIPv2",
        "224.0.0.18" => "VRRP",
        "224.0.0.22" => "IGMP v3",
        "224.0.0.102" => "HSRP / GLBP",
        "224.0.0.107" => "PTP (peer delay)",
        "224.0.0.251" => "mDNS (Bonjour)",
        "224.0.0.252" => "LLMNR",
        "224.0.1.129" => "PTP (IEEE 1588)",
        "239.0.171.1" => "Valmet DNA (réseau de process)",
        "239.255.0.1" => "RTPS / DDS (découverte)",
        "239.255.255.250" => "SSDP / UPnP",
        "::1" => "Loopback (IPv6)",
        "ff02::1" => "Multicast IPv6 — tous les nœuds",
        "ff02::2" => "Multicast IPv6 — tous les routeurs",
        "ff02::fb" => "mDNS (IPv6)",
        "ff02::1:3" => "LLMNR (IPv6)",
        "ff02::c" => "SSDP (IPv6)",
        "ff02::6b" => "PTP (peer delay, IPv6)",
        "ff0e::181" => "PTP (IEEE 1588, IPv6)",
        _ => "",
    }
}

/// Préfixe OUI « XX:XX:XX » (MAJUSCULE) d'une MAC, ou vide si la MAC est trop
/// courte / invalide. Tolère les séparateurs `:` ou `-` et l'absence de
/// séparateur.
pub fn mac_oui(mac: &str) -> String {
    let hex: Vec<char> = mac
        .chars()
        .filter(char::is_ascii_hexdigit)
        .take(6)
        .collect();
    if hex.len() < 6 {
        return String::new();
    }
    let pair = |index: usize| hex[index..index + 2].iter().collect::<String>();
    format!("{}:{}:{}", pair(0), pair(2), pair(4)).to_ascii_uppercase()
}

/// Préfixes localement administrés dont l'usage est une convention publique :
/// ce ne sont pas des OUI attribués par l'IEEE, mais ils identifient de façon
/// fiable l'hyperviseur qui les génère. Seule exception à la règle « une MAC
/// locale ne donne aucun fabricant ».
const LOCAL_PREFIX_CONVENTIONS: [&str; 1] = ["52:54:00"];

/// Vrai si la MAC n'est pas une adresse universelle unicast : bit U/L (0x02,
/// adresse localement administrée — MAC aléatoire des mobiles, VM,
/// conteneurs) ou bit I/G (0x01, groupe) du premier octet. Ses trois premiers
/// octets ne sont alors PAS un OUI : les rapprocher du registre donnerait un
/// fabricant au hasard.
pub fn is_local_or_group_mac(mac: &str) -> bool {
    let oui = mac_oui(mac);
    !oui.is_empty() && u8::from_str_radix(&oui[..2], 16).is_ok_and(|first| first & 0x03 != 0)
}

/// Fabricant déduit de l'OUI, ou vide si rien à déduire : préfixe hors de la
/// liste curée, ou MAC localement administrée / de groupe (pas d'OUI, sauf
/// convention publique d'hyperviseur).
pub fn oui_vendor(mac: &str) -> &'static str {
    let oui = mac_oui(mac);
    if oui.is_empty() {
        return "";
    }
    if is_local_or_group_mac(mac) && !LOCAL_PREFIX_CONVENTIONS.contains(&oui.as_str()) {
        return "";
    }
    oui_vendor_of(&oui)
}

/// Rôle connu d'une IP (v4/v6), ou vide. Exact d'abord, puis plages
/// notoires (loopback, APIPA, multicast, lien-local).
pub fn known_ip(ip: &str) -> &'static str {
    if ip.is_empty() {
        return "";
    }
    let exact = match known_ip_exact(ip) {
        "" => known_ip_exact(&ip.to_lowercase()),
        found => found,
    };
    if !exact.is_empty() {
        return exact;
    }

    if ip.contains(':') {
        let low = ip.to_lowercase();
        if low.starts_with("ff") {
            return "Multicast (IPv6)";
        }
        if low.starts_with("fe80") {
            return "Lien-local (IPv6)";
        }
        return "";
    }

    // `ip.split(".").map(Number)` : mêmes conversions que JavaScript
    // (« 010 » vaut 10, « » vaut 0), puis entiers de 0 à 255 exigés.
    let octets: Vec<Option<f64>> = ip.split('.').map(js_string_to_number).collect();
    let valid = |octet: &Option<f64>| {
        octet.is_some_and(|value| value.fract() == 0.0 && (0.0..=255.0).contains(&value))
    };
    if octets.len() != 4 || !octets.iter().all(valid) {
        return "";
    }
    let (a, b) = (octets[0].unwrap_or(-1.0), octets[1].unwrap_or(-1.0));
    if a == 127.0 {
        return "Loopback";
    }
    if a == 169.0 && b == 254.0 {
        return "APIPA (auto-config, pas de DHCP)";
    }
    if (224.0..=239.0).contains(&a) {
        return "Multicast IPv4";
    }
    ""
}

/// Rôle connu d'une MAC (broadcast, multicast, protocoles 802.1/Cisco), ou
/// vide.
pub fn known_mac(mac: &str) -> &'static str {
    if mac.is_empty() {
        return "";
    }
    let m = mac.to_lowercase();
    if m == "ff:ff:ff:ff:ff:ff" {
        return "Broadcast Ethernet";
    }
    if m == "00:00:00:00:00:00" {
        return "MAC nulle";
    }
    if m.starts_with("01:00:5e") {
        return "Multicast IPv4";
    }
    if m.starts_with("33:33") {
        return "Multicast IPv6";
    }
    if m == "01:1b:19:00:00:00" {
        return "PTP (IEEE 1588 sur Ethernet)";
    }
    if m.starts_with("01:0c:cd:01") {
        return "GOOSE (IEC 61850)";
    }
    if m.starts_with("01:0c:cd:04") {
        return "Sampled Values (IEC 61850)";
    }
    if m.starts_with("01:80:c2") {
        return "802.1 (STP / LLDP / pause)";
    }
    if m.starts_with("01:00:0c") {
        return "Cisco (CDP / VTP / PVST+)";
    }
    ""
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_oui_prefixe_3_octets_separateurs_toleres() {
        assert_eq!(mac_oui("b8:27:eb:12:34:56"), "B8:27:EB");
        assert_eq!(mac_oui("b827eb123456"), "B8:27:EB");
        assert_eq!(mac_oui("B8-27-EB-12-34-56"), "B8:27:EB");
        assert_eq!(mac_oui("zz"), "");
    }

    #[test]
    fn oui_vendor_fabricants_notoires_it_et_ot() {
        assert_eq!(oui_vendor("b8:27:eb:00:00:01"), "Raspberry Pi");
        assert_eq!(oui_vendor("00:0c:29:aa:bb:cc"), "VMware");
        assert_eq!(oui_vendor("00:0e:8c:11:22:33"), "Siemens");
        assert_eq!(oui_vendor("00:00:bc:44:55:66"), "Rockwell / Allen-Bradley");
        // MAC aléatoire / locale non répertoriée -> aucune déduction.
        assert_eq!(oui_vendor("de:73:c8:36:f6:c5"), "");
    }

    #[test]
    fn oui_vendor_une_mac_localement_administree_ou_de_groupe_ne_donne_aucun_fabricant() {
        // Bit U/L (0x02) : les trois premiers octets ne sont pas un OUI.
        assert!(is_local_or_group_mac("de:73:c8:36:f6:c5"));
        assert!(!is_local_or_group_mac("00:0e:8c:11:22:33"));
        // Même préfixe qu'un fabricant répertorié, bit local en plus (00:0E:8C
        // Siemens -> 02:0E:8C) : le registre ne doit pas être consulté « à côté ».
        assert_eq!(oui_vendor("02:0e:8c:11:22:33"), "");
        // Bit I/G (0x01) : adresse de groupe, jamais un fabricant.
        assert!(is_local_or_group_mac("01:00:5e:00:00:fb"));
        assert_eq!(oui_vendor("01:00:5e:00:00:fb"), "");
        // Seule exception : convention publique d'hyperviseur (préfixe local connu).
        assert_eq!(oui_vendor("52:54:00:12:34:56"), "QEMU / KVM");
        // MAC invalide : ni locale ni fabricant.
        assert!(!is_local_or_group_mac("zz"));
    }

    #[test]
    fn known_ip_ip_publiques_celebres_et_plages_notoires() {
        assert_eq!(known_ip("8.8.8.8"), "Google Public DNS");
        assert_eq!(known_ip("1.1.1.1"), "Cloudflare DNS");
        assert_eq!(known_ip("224.0.0.251"), "mDNS (Bonjour)");
        assert_eq!(known_ip("239.1.2.3"), "Multicast IPv4");
        assert_eq!(
            known_ip("169.254.10.20"),
            "APIPA (auto-config, pas de DHCP)"
        );
        assert_eq!(known_ip("127.0.0.1"), "Loopback");
        assert_eq!(known_ip("fe80::1"), "Lien-local (IPv6)");
        assert_eq!(known_ip("192.168.1.50"), ""); // privé banal, rien à déduire
    }

    #[test]
    fn known_mac_broadcast_multicast_protocoles_8021() {
        assert_eq!(known_mac("ff:ff:ff:ff:ff:ff"), "Broadcast Ethernet");
        assert_eq!(known_mac("01:00:5e:00:00:fb"), "Multicast IPv4");
        assert_eq!(known_mac("33:33:00:00:00:01"), "Multicast IPv6");
        assert_eq!(known_mac("01:80:c2:00:00:00"), "802.1 (STP / LLDP / pause)");
        assert_eq!(known_mac("e0:d3:62:6a:a2:5a"), ""); // MAC unicast banale
    }

    #[test]
    fn known_ip_octets_lus_comme_number() {
        // Conversions de `Number` : « 0x7f » vaut 127, « 1e2 » vaut 100.
        assert_eq!(known_ip("0x7f.0.0.1"), "Loopback");
        assert_eq!(known_ip("127.0.0"), "");
        assert_eq!(known_ip("127.0.0.256"), "");
        assert_eq!(known_ip("127.0.0.1.5"), "");
    }
}
