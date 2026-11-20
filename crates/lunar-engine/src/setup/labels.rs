//! Labels générés au démarrage : le poste LUNAR s'auto-étiquette (« pc
//! lunar ») à partir de ses interfaces réseau, et fournit les primitives de
//! parsing d'une ligne de label CSV réutilisées par les imports.

use std::sync::{Arc, Mutex};

use crate::{
    errors::CaptureStateError,
    state::{
        flow_matrix::FlowMatrix,
        graph::{GraphData, GraphUpdate},
        labels_list::{LabelStore, PcInfoLabel},
    },
};

/// Reconstruit le miroir de labels de la matrice depuis les sources de
/// vérité : labels « pc lunar » d'abord, lignes du [`LabelStore`] ensuite
/// (l'utilisateur gagne à clé égale). Le miroir est entièrement remplacé :
/// un label supprimé du store disparaît de la matrice.
pub fn resync_labels_into_matrix(
    pcinfo: &PcInfoLabel,
    store: &LabelStore,
    matrix: &mut FlowMatrix,
) {
    matrix.label.clear();
    for line in pcinfo.get_label() {
        if let Some((mac, ip, label)) = parse_label_row(line) {
            matrix.add_label(mac, ip, label);
        }
    }
    for (mac, ip, label) in store.get() {
        matrix.add_label(mac.clone(), ip.clone(), label.clone());
    }
}

/// Resynchronisation complète après une mutation de labels : miroir de la
/// matrice puis rafraîchissement des nœuds du graphe. Retourne les updates
/// graphe à pousser au frontend (channel live et/ou réponse de commande).
pub fn resync_labels(
    pcinfo: &PcInfoLabel,
    store: &LabelStore,
    matrix: &mut FlowMatrix,
    graph: &mut GraphData,
) -> Vec<GraphUpdate> {
    resync_labels_into_matrix(pcinfo, store, matrix);
    graph.refresh_labels(|mac, ip| matrix.get_label(mac, ip))
}

/// Enregistre une ligne de label « pc lunar » pour chaque couple
/// (MAC, IP) des interfaces réseau locales : la machine de capture est ainsi
/// identifiable dans la matrice et le graphe.
pub fn create_labels_from_network_interfaces(
    interfaces: Vec<netdev::Interface>,
    pcinfo: &Arc<Mutex<PcInfoLabel>>,
) -> Result<(), CaptureStateError> {
    let mut pcinfo = pcinfo.lock()?;
    const LABEL_NAME: &str = "pc lunar";

    for interface in interfaces {
        let Some(mac_addr) = interface.mac_addr else {
            continue;
        };

        let mac_addr = mac_addr.to_string();

        for ipv4 in interface.ipv4_addrs() {
            pcinfo.push(format!("{mac_addr},{ipv4},{LABEL_NAME}"));
        }

        for ipv6 in interface.ipv6_addrs() {
            pcinfo.push(format!("{mac_addr},{ipv6},{LABEL_NAME}"));
        }
    }

    Ok(())
}

/// Recopie les labels « pc lunar » mémorisés au démarrage dans la matrice de
/// flux (appelé à chaque démarrage de capture).
pub fn update_labels_in_state(
    pcinfo: &Arc<Mutex<PcInfoLabel>>,
    state_label: &mut FlowMatrix,
) -> Result<(), CaptureStateError> {
    let pcinfo = pcinfo.lock()?;

    for label in pcinfo.get_label() {
        let Some((mac, ip, label_name)) = parse_label_row(label) else {
            continue;
        };

        state_label.add_label(mac.to_string(), ip, label_name);
    }

    Ok(())
}

/// Parse une ligne CSV `mac,ip,label…` en tuple `(mac, ip, label)`.
/// Voir [`parse_label_fields`] pour les règles de normalisation.
pub fn parse_label_row(row: &str) -> Option<(String, String, String)> {
    parse_label_fields(row.split(','))
}

/// Normalise les champs d'une ligne de label : au moins 3 colonnes, au moins
/// une adresse (MAC ou IP) non vide, notation CIDR ramenée à l'adresse seule,
/// colonnes surnuméraires fusionnées dans le label. Retourne `None` pour une
/// ligne inexploitable, et pour une ligne **sans label** : un fichier « à
/// compléter » se réimporte à moitié rempli sans étiqueter
/// « Label? » les équipements restés vides.
pub fn parse_label_fields<I, S>(fields: I) -> Option<(String, String, String)>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let parts: Vec<String> = fields
        .into_iter()
        .map(|value| clean_csv_field(value.as_ref()).to_string())
        .collect();

    if parts.len() < 3 {
        return None;
    }

    let mac = &parts[0];
    let ip = &parts[1];
    let label_parts = &parts[2..];

    // Une ligne doit porter au moins une adresse (MAC ou IP) pour être exploitable.
    if mac.is_empty() && ip.is_empty() {
        return None;
    }

    // Une IP en notation CIDR ("192.168.1.0/24") est ramenée à sa seule adresse.
    let ip = ip.split('/').next().unwrap_or(ip);
    let label = label_parts
        .iter()
        .filter(|value| !value.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    if label.is_empty() {
        return None;
    }

    Some((mac.to_string(), ip.to_string(), label))
}

/// Vrai pour une ligne qui désigne un équipement (au moins une adresse) sans
/// lui donner de label : ignorée à l'import, et comptée dans son bilan.
pub fn label_row_is_blank<I, S>(fields: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let parts: Vec<String> = fields
        .into_iter()
        .map(|value| clean_csv_field(value.as_ref()).to_string())
        .collect();
    parts.len() >= 3
        && !(parts[0].is_empty() && parts[1].is_empty())
        && parts[2..].iter().all(String::is_empty)
}

/// Nettoie un champ CSV : espaces et guillemets d'encadrement retirés.
pub fn clean_csv_field(value: &str) -> &str {
    value.trim().trim_matches('"')
}

/// Ramène une adresse MAC à la forme canonique produite par le parser de
/// paquets (`aa:bb:cc:dd:ee:ff`) depuis les écritures rencontrées dans les
/// fichiers importés : `AA:BB:CC:DD:EE:FF`, `aa-bb-cc-dd-ee-ff` (Windows),
/// `aabb.ccdd.eeff` (Cisco) et `aabbccddeeff` (exports bruts d'inventaire).
/// Retourne `None` si, séparateurs retirés, il ne reste pas exactement
/// 12 chiffres hexadécimaux : la valeur est alors rapportée comme MAC
/// invalide par la validation d'import.
pub fn canonical_mac(value: &str) -> Option<String> {
    let digits: Vec<u8> = value
        .bytes()
        .filter(|byte| !matches!(byte, b':' | b'-' | b'.'))
        .collect();
    if digits.len() != 12 || !digits.iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    let mut mac = String::with_capacity(17);
    for (index, pair) in digits.chunks(2).enumerate() {
        if index > 0 {
            mac.push(':');
        }
        mac.push(pair[0].to_ascii_lowercase() as char);
        mac.push(pair[1].to_ascii_lowercase() as char);
    }
    Some(mac)
}

#[cfg(test)]
mod tests {
    use super::{
        canonical_mac, create_labels_from_network_interfaces, parse_label_row, resync_labels,
        resync_labels_into_matrix,
    };
    use crate::state::{
        flow_matrix::FlowMatrix,
        graph::{GraphData, GraphUpdate, Node},
        labels_list::{LabelStore, PcInfoLabel},
    };
    use netdev::Interface;
    use netdev::ipnet::{Ipv4Net, Ipv6Net};
    use std::net::{Ipv4Addr, Ipv6Addr};
    use std::sync::{Arc, Mutex};

    /// Lignes « pc lunar » produites par la vraie fonction de démarrage.
    fn build_label_rows(interfaces: Vec<netdev::Interface>) -> Vec<String> {
        let pcinfo = Arc::new(Mutex::new(PcInfoLabel::new()));
        create_labels_from_network_interfaces(interfaces, &pcinfo).unwrap();
        pcinfo.lock().unwrap().get_label().clone()
    }

    #[test]
    fn creates_one_row_per_ip_address() {
        let mut interface = Interface::dummy();
        interface.mac_addr = Some("aa:bb:cc:dd:ee:ff".parse().unwrap());
        interface.ipv4 = vec![Ipv4Net::new(Ipv4Addr::new(192, 168, 1, 10), 24).unwrap()];
        interface.ipv6 =
            vec![Ipv6Net::new("2001:db8::10".parse::<Ipv6Addr>().unwrap(), 64).unwrap()];

        let labels = build_label_rows(vec![interface]);

        assert_eq!(
            labels,
            vec![
                "aa:bb:cc:dd:ee:ff,192.168.1.10,pc lunar".to_string(),
                "aa:bb:cc:dd:ee:ff,2001:db8::10,pc lunar".to_string(),
            ]
        );
    }

    #[test]
    fn skips_interfaces_without_mac_address() {
        let mut interface = Interface::dummy();
        interface.ipv4 = vec![Ipv4Net::new(Ipv4Addr::new(10, 0, 0, 5), 24).unwrap()];

        let labels = build_label_rows(vec![interface]);

        assert!(labels.is_empty());
    }

    /// Les écritures courantes d'une MAC (majuscules, tirets Windows,
    /// points Cisco, sans séparateur) convergent vers la forme du parser ;
    /// tout ce qui n'est pas 12 chiffres hexadécimaux est refusé.
    #[test]
    fn canonical_mac_accepts_usual_writings_and_rejects_the_rest() {
        for written in [
            "aa:bb:cc:dd:ee:ff",
            "AA:BB:CC:DD:EE:FF",
            "aa-bb-cc-dd-ee-ff",
            "AA-BB-CC-DD-EE-FF",
            "aabb.ccdd.eeff",
            "aabbccddeeff",
            "AABBCCDDEEFF",
        ] {
            assert_eq!(
                canonical_mac(written).as_deref(),
                Some("aa:bb:cc:dd:ee:ff"),
                "{written}"
            );
        }
        for rejected in [
            "",
            "aabbccddeef",
            "aabbccddeeff0",
            "aabbccddeefg",
            "aa:bb:cc:dd:e:ff",
            "pas-une-mac",
            "192.168.1.10",
        ] {
            assert_eq!(canonical_mac(rejected), None, "{rejected}");
        }
    }

    #[test]
    fn parses_label_row_into_mac_ip_and_label() {
        let parsed = parse_label_row("aa:bb:cc:dd:ee:ff,192.168.1.10,pc lunar");

        assert_eq!(
            parsed,
            Some((
                "aa:bb:cc:dd:ee:ff".to_string(),
                "192.168.1.10".to_string(),
                "pc lunar".to_string(),
            ))
        );
    }

    #[test]
    fn rejects_invalid_label_rows() {
        assert_eq!(parse_label_row("aa:bb:cc:dd:ee:ff,192.168.1.10"), None);
        assert_eq!(parse_label_row(",,pc lunar"), None);
    }

    #[test]
    fn parses_ip_only_label_row() {
        let parsed = parse_label_row(",8.8.8.8,google.com");

        assert_eq!(
            parsed,
            Some((
                String::new(),
                "8.8.8.8".to_string(),
                "google.com".to_string(),
            ))
        );
    }

    #[test]
    fn parses_quoted_ip_only_label_row() {
        let parsed = parse_label_row(",\"8.8.8.8\", \"google.com\"");

        assert_eq!(
            parsed,
            Some((
                String::new(),
                "8.8.8.8".to_string(),
                "google.com".to_string(),
            ))
        );
    }

    #[test]
    fn parses_trailing_empty_column() {
        let parsed = parse_label_row("aa:bb:cc:dd:ee:ff,192.168.1.10,pc lunar,");

        assert_eq!(
            parsed,
            Some((
                "aa:bb:cc:dd:ee:ff".to_string(),
                "192.168.1.10".to_string(),
                "pc lunar".to_string(),
            ))
        );
    }

    #[test]
    fn merges_extra_columns_into_label() {
        let parsed = parse_label_row(",8.8.8.8,google,public dns");

        assert_eq!(
            parsed,
            Some((
                String::new(),
                "8.8.8.8".to_string(),
                "google public dns".to_string(),
            ))
        );
    }

    #[test]
    fn store_row_overrides_pcinfo_row_at_the_same_key() {
        let mut pcinfo = PcInfoLabel::new();
        pcinfo.push("aa:bb:cc:dd:ee:ff,192.168.1.1,pc lunar".to_string());
        let mut store = LabelStore::new();
        store.add((
            "aa:bb:cc:dd:ee:ff".to_string(),
            "192.168.1.1".to_string(),
            "Poste de l'opérateur".to_string(),
        ));
        let mut matrix = FlowMatrix::new();

        resync_labels_into_matrix(&pcinfo, &store, &mut matrix);

        // Le store (l'utilisateur) est rejoué après pcinfo : il gagne à clé égale.
        assert_eq!(
            matrix.get_label("aa:bb:cc:dd:ee:ff", "192.168.1.1"),
            Some("Poste de l'opérateur".to_string())
        );
    }

    #[test]
    fn pcinfo_only_rows_survive_the_resync() {
        let mut pcinfo = PcInfoLabel::new();
        pcinfo.push("aa:bb:cc:dd:ee:ff,192.168.1.1,pc lunar".to_string());
        let store = LabelStore::new();
        let mut matrix = FlowMatrix::new();

        resync_labels_into_matrix(&pcinfo, &store, &mut matrix);

        assert_eq!(
            matrix.get_label("aa:bb:cc:dd:ee:ff", "192.168.1.1"),
            Some("pc lunar".to_string())
        );
    }

    /// Garde de non-régression : le miroir est entièrement remplacé à
    /// chaque resync, donc un label retiré du store disparaît vraiment de la
    /// matrice (l'ancienne sémantique « un import ne désétiquette jamais »
    /// est abandonnée).
    #[test]
    fn a_label_removed_from_the_store_disappears_from_the_matrix() {
        let pcinfo = PcInfoLabel::new();
        let mut store = LabelStore::new();
        store.add((
            "aa:bb:cc:dd:ee:ff".to_string(),
            "192.168.1.1".to_string(),
            "mon-pc".to_string(),
        ));
        let mut matrix = FlowMatrix::new();
        resync_labels_into_matrix(&pcinfo, &store, &mut matrix);
        assert_eq!(
            matrix.get_label("aa:bb:cc:dd:ee:ff", "192.168.1.1"),
            Some("mon-pc".to_string())
        );

        // Le label disparaît du store (arbitrage / suppression) puis on resynchronise.
        let store = LabelStore::new();
        resync_labels_into_matrix(&pcinfo, &store, &mut matrix);

        assert_eq!(
            matrix.get_label("aa:bb:cc:dd:ee:ff", "192.168.1.1"),
            None,
            "le miroir doit refléter la disparition, pas la conserver"
        );
    }

    #[test]
    fn resync_labels_returns_a_node_updated_for_the_new_label() {
        let pcinfo = PcInfoLabel::new();
        let mut store = LabelStore::new();
        store.add((
            "aa:bb:cc:dd:ee:ff".to_string(),
            "192.168.1.1".to_string(),
            "mon-pc".to_string(),
        ));
        let mut matrix = FlowMatrix::new();
        let mut graph = GraphData::new();
        graph.nodes.insert(
            "192.168.1.1".to_string(),
            Node::new(
                "192.168.1.1",
                "192.168.1.1".to_string(),
                "aa:bb:cc:dd:ee:ff".to_string(),
                "#2196F3",
                "192.168.1.1".to_string(),
                None,
                None,
            ),
        );

        let updates = resync_labels(&pcinfo, &store, &mut matrix, &mut graph);

        assert_eq!(updates.len(), 1);
        match &updates[0] {
            GraphUpdate::NodeUpdated(node) => assert_eq!(node.label, Some("mon-pc".to_string())),
            other => panic!("attendu NodeUpdated, reçu {other:?}"),
        }
    }
}
