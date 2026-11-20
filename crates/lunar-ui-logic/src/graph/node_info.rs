//! Données du bandeau « nœud sélectionné » : snapshot pour l'édition de
//! label, et résumé texte (degré, trafic, protocoles…).

use super::store::GraphStore;
use super::style::format_bytes;

/// Snapshot des attributs d'un nœud (`NodeData`), utilisé par le bandeau bas
/// et l'édition de label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeSnapshot {
    /// Identifiant du nœud.
    pub id: String,
    /// Nom du nœud.
    pub name: String,
    /// Première MAC observée.
    pub mac: String,
    /// IP du nœud.
    pub ip: String,
    /// Couleur de remplissage.
    pub color: String,
    /// Label opérateur brut (vide si aucun).
    pub label: String,
}

/// Snapshot du nœud `node_id`, ou `None` s'il n'existe pas.
pub fn node_snapshot(store: &GraphStore, node_id: &str) -> Option<NodeSnapshot> {
    let n = store.node_by_id(node_id)?;
    Some(NodeSnapshot {
        id: node_id.to_string(),
        name: n.style.name.clone(),
        mac: n.style.mac.clone(),
        ip: n.style.ip.clone(),
        color: n.style.color.clone(),
        label: n.style.raw_label.clone(),
    })
}

/// Résumé texte des infos d'un nœud pour le bandeau bas : identité, MAC,
/// IP, VLAN, couleur, puis degré, trafic et protocoles agrégés sur ses
/// arêtes.
pub fn build_node_infos(store: &GraphStore, node_id: &str) -> Vec<String> {
    let Some(index) = store.node_index(node_id) else {
        return vec!["Nœud introuvable".to_string()];
    };
    let Some(n) = store.node(index) else {
        return vec!["Nœud introuvable".to_string()];
    };

    let mut protocols: Vec<&str> = Vec::new();
    let mut degree = 0usize;
    let mut packets = 0u64;
    let mut bytes = 0u64;
    for e in store.node_edges(index) {
        let Some(edge) = store.edge(e) else { continue };
        degree += 1;
        let protocol = edge.style.protocol.as_str();
        if !protocol.is_empty() && !protocols.contains(&protocol) {
            protocols.push(protocol);
        }
        packets = packets.saturating_add(edge.style.count);
        bytes = bytes.saturating_add(edge.style.total_bytes);
    }

    let s = &n.style;
    // Plusieurs MAC observées pour cette IP : anomalie mise en évidence.
    // MAC portée par de nombreuses IP : passerelle probable, le
    // label de la MAC seule ne s'applique pas à ce nœud.
    let gateway_info = if s.gateway_mac {
        " (passerelle probable : de nombreuses IP derrière)"
    } else {
        ""
    };
    let mac_info = if s.mac_conflict {
        format!("⚠ MACs multiples ({}): {}", s.macs.len(), s.macs.join(", "))
    } else {
        format!("MAC: {}{gateway_info}", s.mac)
    };

    // Identité contextualisée : le VLAN fait partie de la clé du
    // nœud ; une IP portée par un autre nœud (autre VLAN) est une anomalie.
    let ip_info = if s.duplicate_ip {
        format!("⚠ IP: {} (aussi présente sur un autre VLAN)", s.ip)
    } else {
        format!("IP: {}", s.ip)
    };

    let vlan = s
        .vlan_id
        .map(|v| v.to_string())
        .unwrap_or_else(|| "—".to_string());
    let label = if s.raw_label.is_empty() {
        "N/A"
    } else {
        s.raw_label.as_str()
    };
    let protocols = if protocols.is_empty() {
        "—".to_string()
    } else {
        protocols.join(", ")
    };

    vec![
        format!("ID: {node_id}"),
        format!("Nom: {}", s.name),
        format!("Label: {label}"),
        mac_info,
        ip_info,
        format!("VLAN: {vlan}"),
        format!("Couleur: {}", s.color),
        format!("Degré: {degree}"),
        format!("Trafic: {} ({packets} paquets)", format_bytes(bytes)),
        format!("Protocoles: {protocols}"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::rng::Rng;
    use crate::graph::style::tests::{edge, node};

    fn new_graph() -> GraphStore {
        let mut g = GraphStore::new(Rng::with_seed(5));
        let mut a = node("192.168.1.1");
        a.name = "hote-a".into();
        a.mac = "aa:bb:cc:dd:ee:ff".into();
        a.macs = vec!["aa:bb:cc:dd:ee:ff".into()];
        a.ip = "192.168.1.1".into();
        a.label = Some("PC Lunar".into());
        a.color = "#2196F3".into();
        g.upsert_node(&a);

        let mut b = node("192.168.1.2");
        b.name = "hote-b".into();
        b.color = "#000".into();
        g.upsert_node(&b);

        let mut c = node("192.168.1.3");
        c.name = "hote-c".into();
        c.mac = "11:11:11:11:11:11".into();
        c.macs = vec!["11:11:11:11:11:11".into(), "22:22:22:22:22:22".into()];
        c.ip = "192.168.1.3".into();
        c.color = "#f00".into();
        g.upsert_node(&c);

        // Nœud tagué 802.1Q dont l'IP existe aussi sur un autre VLAN.
        let mut d = node("vlan42:192.168.1.4");
        d.name = "192.168.1.4".into();
        d.mac = "33:33:33:33:33:33".into();
        d.macs = vec!["33:33:33:33:33:33".into()];
        d.ip = "192.168.1.4".into();
        d.vlan_id = Some(42);
        d.duplicate_ip = true;
        d.color = "#0f0".into();
        g.upsert_node(&d);
        g
    }

    fn joined(infos: &[String]) -> String {
        infos.join(" | ")
    }

    #[test]
    fn node_snapshot_noeud_inexistant_none() {
        let g = new_graph();
        assert_eq!(node_snapshot(&g, "fantome"), None);
    }

    #[test]
    fn node_snapshot_mappe_les_attributs_vers_node_data() {
        let g = new_graph();
        assert_eq!(
            node_snapshot(&g, "192.168.1.1"),
            Some(NodeSnapshot {
                id: "192.168.1.1".into(),
                name: "hote-a".into(),
                mac: "aa:bb:cc:dd:ee:ff".into(),
                ip: "192.168.1.1".into(),
                color: "#2196F3".into(),
                label: "PC Lunar".into(),
            })
        );
    }

    #[test]
    fn build_node_infos_noeud_inexistant_message_d_erreur() {
        let g = new_graph();
        assert_eq!(build_node_infos(&g, "fantome"), ["Nœud introuvable"]);
    }

    #[test]
    fn build_node_infos_agrege_degre_trafic_et_protocoles() {
        let mut g = new_graph();
        let mut e1 = edge("192.168.1.1", "192.168.1.2", "TCP");
        e1.count = 10;
        e1.total_bytes = 1000;
        let mut e2 = edge("192.168.1.2", "192.168.1.1", "DNS");
        e2.count = 2;
        e2.total_bytes = 200;
        g.upsert_edge(&e1);
        g.upsert_edge(&e2);

        let infos = build_node_infos(&g, "192.168.1.1");

        assert!(
            infos.contains(&"Degré: 2".to_string()),
            "{}",
            joined(&infos)
        );
        assert!(
            infos
                .iter()
                .any(|l| l.contains("Protocoles:") && l.contains("TCP") && l.contains("DNS")),
            "{}",
            joined(&infos)
        );
        assert!(
            infos
                .iter()
                .any(|l| l.starts_with("Trafic:") && l.contains("12 paquets")),
            "{}",
            joined(&infos)
        );
        // Ordre d'adjacence : entrantes (DNS) puis sortantes (TCP).
        assert!(
            infos.contains(&"Protocoles: DNS, TCP".to_string()),
            "{}",
            joined(&infos)
        );
        assert!(
            infos.contains(&"Trafic: 1.2 ko (12 paquets)".to_string()),
            "{}",
            joined(&infos)
        );
    }

    #[test]
    fn build_node_infos_mac_passerelle_mention_sur_la_ligne_mac() {
        let mut g = new_graph();
        let mut n = node("10.0.0.7");
        n.mac = "00:04:96:a0:0f:60".into();
        n.macs = vec!["00:04:96:a0:0f:60".into()];
        n.gateway_mac = true;
        n.ip = "10.0.0.7".into();
        n.color = "#000".into();
        g.upsert_node(&n);
        let infos = build_node_infos(&g, "10.0.0.7");
        assert!(
            infos.contains(
                &"MAC: 00:04:96:a0:0f:60 (passerelle probable : de nombreuses IP derrière)"
                    .to_string()
            ),
            "{}",
            joined(&infos)
        );
        assert!(
            build_node_infos(&g, "192.168.1.1").contains(&"MAC: aa:bb:cc:dd:ee:ff".to_string())
        );
    }

    #[test]
    fn build_node_infos_noeud_isole_pas_d_arete_pas_de_protocole() {
        let g = new_graph();
        let infos = build_node_infos(&g, "192.168.1.2");

        assert!(
            infos.contains(&"Degré: 0".to_string()),
            "{}",
            joined(&infos)
        );
        assert!(
            infos.iter().any(|l| l == "Protocoles: —"),
            "{}",
            joined(&infos)
        );
        assert!(
            infos.contains(&"Label: N/A".to_string()),
            "{}",
            joined(&infos)
        );
    }

    #[test]
    fn build_node_infos_conflit_de_mac_signale() {
        let g = new_graph();
        let infos = build_node_infos(&g, "192.168.1.3");

        assert!(
            infos.iter().any(|l| l.starts_with("⚠ MACs multiples (2)")),
            "{}",
            joined(&infos)
        );
        assert!(
            infos.contains(
                &"⚠ MACs multiples (2): 11:11:11:11:11:11, 22:22:22:22:22:22".to_string()
            )
        );
    }

    #[test]
    fn build_node_infos_vlan_affiche_ip_dupliquee_signalee() {
        let g = new_graph();
        let infos = build_node_infos(&g, "vlan42:192.168.1.4");

        assert!(
            infos.contains(&"VLAN: 42".to_string()),
            "{}",
            joined(&infos)
        );
        assert!(
            infos.contains(&"⚠ IP: 192.168.1.4 (aussi présente sur un autre VLAN)".to_string()),
            "{}",
            joined(&infos)
        );
    }

    #[test]
    fn build_node_infos_hors_trame_taguee_vlan_absent() {
        let g = new_graph();
        let infos = build_node_infos(&g, "192.168.1.1");

        assert!(infos.contains(&"VLAN: —".to_string()), "{}", joined(&infos));
        assert!(
            infos.contains(&"IP: 192.168.1.1".to_string()),
            "{}",
            joined(&infos)
        );
    }

    #[test]
    fn build_node_infos_lignes_dans_l_ordre_du_bandeau() {
        let g = new_graph();
        assert_eq!(
            build_node_infos(&g, "192.168.1.1"),
            [
                "ID: 192.168.1.1",
                "Nom: hote-a",
                "Label: PC Lunar",
                "MAC: aa:bb:cc:dd:ee:ff",
                "IP: 192.168.1.1",
                "VLAN: —",
                "Couleur: #2196F3",
                "Degré: 0",
                "Trafic: 0 o (0 paquets)",
                "Protocoles: —",
            ]
        );
    }
}
