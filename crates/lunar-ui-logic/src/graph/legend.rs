//! Données pures de la légende. Les couleurs de catégories viennent du cœur
//! ([`super::categories`]) ; les autres valeurs viennent des helpers du rendu
//! ([`super::style`]), afin que la légende et le rendu partagent leurs
//! sources.

use super::categories::{GRAPH_NODE_CATEGORIES, GraphNodeCategory};
use super::format::fr_decimal;
use super::style::{
    DIM_EDGE_COLOR, DIM_NODE_COLOR, DUPLICATE_IP_BORDER_COLOR, EDGE_LABEL_ZOOM, EDGE_SIZE_MAX,
    EDGE_SIZE_MIN, GATEWAY_MAC_BORDER_COLOR, MAC_CONFLICT_BORDER_COLOR, NODE_BORDER_DARKEN_FACTOR,
    NODE_SIZE_MAX, NODE_SIZE_MIN, PORT_LABEL_ZOOM, darken,
};

/// Libellé de légende d'une catégorie de nœud (`NODE_CATEGORY_LABELS`) ; le
/// `match` exhaustif tient lieu du `satisfies Record<GraphNodeCategory, …>`.
pub fn node_category_label(kind: GraphNodeCategory) -> &'static str {
    match kind {
        GraphNodeCategory::Private => "IP privée",
        GraphNodeCategory::Public => "IP publique",
        GraphNodeCategory::Multicast => "IP multicast",
        GraphNodeCategory::Loopback => "Boucle locale",
        GraphNodeCategory::Apipa => "APIPA",
        GraphNodeCategory::LinkLocal => "IPv6 lien-local",
        GraphNodeCategory::Ula => "IPv6 ULA",
        GraphNodeCategory::Documentation => "IP de documentation",
        GraphNodeCategory::Broadcast => "Diffusion (255.255.255.255)",
        GraphNodeCategory::Layer2 => "Couche 2 (MAC)",
    }
}

/// Entrée de la légende des couleurs de nœuds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeColorLegendItem {
    /// Catégorie.
    pub kind: GraphNodeCategory,
    /// Libellé affiché.
    pub label: &'static str,
    /// Remplissage.
    pub color: &'static str,
    /// Bordure ordinaire, celle du rendu (remplissage assombri de 25 %).
    pub border_color: String,
}

/// Légende des couleurs de nœuds (`NODE_COLOR_LEGEND`), dans l'ordre du
/// cœur.
pub fn node_color_legend() -> Vec<NodeColorLegendItem> {
    GRAPH_NODE_CATEGORIES
        .iter()
        .map(|c| NodeColorLegendItem {
            kind: c.kind,
            label: node_category_label(c.kind),
            color: c.color,
            border_color: darken(c.color, NODE_BORDER_DARKEN_FACTOR),
        })
        .collect()
}

/// Bornes d'une échelle de tailles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SizeRange {
    /// Taille minimale.
    pub min: f32,
    /// Taille maximale.
    pub max: f32,
}

/// Constantes de rendu reprises par la légende (`GRAPH_RENDERING_LEGEND`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GraphRenderingLegend {
    /// Rayons des nœuds.
    pub node_size: SizeRange,
    /// Épaisseurs des arêtes.
    pub edge_size: SizeRange,
    /// Zoom d'affichage des protocoles.
    pub edge_label_zoom: f32,
    /// Zoom d'affichage des ports.
    pub port_label_zoom: f32,
    /// Bordure « plusieurs MAC ».
    pub mac_conflict_border_color: &'static str,
    /// Bordure « IP sur plusieurs VLAN ».
    pub duplicate_ip_border_color: &'static str,
    /// Bordure « MAC passerelle ».
    pub gateway_mac_border_color: &'static str,
    /// Arêtes estompées pendant la surbrillance d'un tunnel.
    pub tunnel_dim_edge_color: &'static str,
    /// Nœuds estompés pendant la surbrillance d'un tunnel.
    pub tunnel_dim_node_color: &'static str,
}

/// Constantes de rendu de la légende, partagées avec le moteur de rendu.
pub const GRAPH_RENDERING_LEGEND: GraphRenderingLegend = GraphRenderingLegend {
    node_size: SizeRange {
        min: NODE_SIZE_MIN,
        max: NODE_SIZE_MAX,
    },
    edge_size: SizeRange {
        min: EDGE_SIZE_MIN,
        max: EDGE_SIZE_MAX,
    },
    edge_label_zoom: EDGE_LABEL_ZOOM,
    port_label_zoom: PORT_LABEL_ZOOM,
    mac_conflict_border_color: MAC_CONFLICT_BORDER_COLOR,
    duplicate_ip_border_color: DUPLICATE_IP_BORDER_COLOR,
    gateway_mac_border_color: GATEWAY_MAC_BORDER_COLOR,
    tunnel_dim_edge_color: DIM_EDGE_COLOR,
    tunnel_dim_node_color: DIM_NODE_COLOR,
};

/// Nombre de la légende : `toLocaleString("fr-FR", { maximumFractionDigits:
/// 1 })` (`1.2` → `"1,2"`, `80` → `"80"`).
pub fn format_legend_number(value: f32) -> String {
    // Repasse par la représentation décimale la plus courte du f32 (« 1.2 »)
    // plutôt que par sa valeur binaire exacte (1.2000000476…) : la valeur
    // voulue est 1,2.
    let value: f64 = value.to_string().parse().unwrap_or(f64::from(value));
    fr_decimal(value, 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::style::{node_attributes, tests::node};

    #[test]
    fn legende_noeuds_reprend_toutes_les_categories_et_couleurs_du_coeur() {
        let legend = node_color_legend();
        let pairs: Vec<_> = legend.iter().map(|i| (i.kind, i.color)).collect();
        let expected: Vec<_> = GRAPH_NODE_CATEGORIES
            .iter()
            .map(|c| (c.kind, c.color))
            .collect();
        assert_eq!(pairs, expected);
        // Chaque catégorie a son libellé (le match est exhaustif), sans doublon.
        let mut labels: Vec<_> = legend.iter().map(|i| i.label).collect();
        labels.sort();
        labels.dedup();
        assert_eq!(labels.len(), legend.len());
    }

    #[test]
    fn legende_noeuds_remplissage_et_bordure_correspondent_au_rendu() {
        for item in node_color_legend() {
            let mut n = node(item.kind.kind());
            n.color = item.color.to_string();
            let attrs = node_attributes(&n);
            assert_eq!(attrs.color, item.color);
            assert_eq!(attrs.border_color, item.border_color);
        }
    }

    #[test]
    fn legende_du_rendu_partage_les_constantes_du_rendu() {
        assert_eq!(
            GRAPH_RENDERING_LEGEND,
            GraphRenderingLegend {
                node_size: SizeRange {
                    min: NODE_SIZE_MIN,
                    max: NODE_SIZE_MAX
                },
                edge_size: SizeRange {
                    min: EDGE_SIZE_MIN,
                    max: EDGE_SIZE_MAX
                },
                edge_label_zoom: EDGE_LABEL_ZOOM,
                port_label_zoom: PORT_LABEL_ZOOM,
                mac_conflict_border_color: MAC_CONFLICT_BORDER_COLOR,
                duplicate_ip_border_color: DUPLICATE_IP_BORDER_COLOR,
                gateway_mac_border_color: GATEWAY_MAC_BORDER_COLOR,
                tunnel_dim_edge_color: DIM_EDGE_COLOR,
                tunnel_dim_node_color: DIM_NODE_COLOR,
            }
        );
    }

    #[test]
    fn nombres_de_legende_a_la_francaise() {
        assert_eq!(
            format_legend_number(GRAPH_RENDERING_LEGEND.node_size.min),
            "7"
        );
        assert_eq!(
            format_legend_number(GRAPH_RENDERING_LEGEND.node_size.max),
            "80"
        );
        assert_eq!(
            format_legend_number(GRAPH_RENDERING_LEGEND.edge_size.min),
            "1,2"
        );
        assert_eq!(
            format_legend_number(GRAPH_RENDERING_LEGEND.edge_label_zoom),
            "1,2"
        );
        assert_eq!(
            format_legend_number(GRAPH_RENDERING_LEGEND.port_label_zoom),
            "1,8"
        );
    }
}
