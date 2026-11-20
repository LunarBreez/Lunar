//! Catégories de nœuds et leurs couleurs, telles que le cœur les attribue
//! (`color_of` et la couleur L2 de `lunar-flows-core/src/graph.rs`).
//!
//! `color_of` étant privé au cœur, la table est recopiée ici, dans le même
//! ordre.

/// Catégorie d'un nœud : type d'IP (`IpType` hors `Unknown`) ou nœud de
/// couche 2 (MAC seule).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GraphNodeCategory {
    /// IP privée (RFC 1918).
    Private,
    /// IP publique.
    Public,
    /// IP multicast.
    Multicast,
    /// Boucle locale.
    Loopback,
    /// APIPA (169.254.0.0/16).
    Apipa,
    /// IPv6 lien-local.
    LinkLocal,
    /// IPv6 ULA.
    Ula,
    /// IP de documentation.
    Documentation,
    /// Diffusion 255.255.255.255.
    Broadcast,
    /// Nœud de couche 2, identifié par sa MAC.
    Layer2,
}

impl GraphNodeCategory {
    /// Nom de la variante, identique à la clé `kind` du fichier généré.
    pub fn kind(self) -> &'static str {
        match self {
            Self::Private => "Private",
            Self::Public => "Public",
            Self::Multicast => "Multicast",
            Self::Loopback => "Loopback",
            Self::Apipa => "Apipa",
            Self::LinkLocal => "LinkLocal",
            Self::Ula => "Ula",
            Self::Documentation => "Documentation",
            Self::Broadcast => "Broadcast",
            Self::Layer2 => "Layer2",
        }
    }
}

/// Catégorie et couleur de remplissage associée.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GraphNodeCategoryColor {
    /// Catégorie du nœud.
    pub kind: GraphNodeCategory,
    /// Couleur `#RRGGBB` posée par le cœur.
    pub color: &'static str,
}

/// Couleurs observées en faisant passer chaque catégorie par `GraphData`
/// (`GRAPH_NODE_CATEGORIES`), dans l'ordre du fichier généré.
pub const GRAPH_NODE_CATEGORIES: [GraphNodeCategoryColor; 10] = [
    GraphNodeCategoryColor {
        kind: GraphNodeCategory::Private,
        color: "#8BC34A",
    },
    GraphNodeCategoryColor {
        kind: GraphNodeCategory::Public,
        color: "#2196F3",
    },
    GraphNodeCategoryColor {
        kind: GraphNodeCategory::Multicast,
        color: "#FFC107",
    },
    GraphNodeCategoryColor {
        kind: GraphNodeCategory::Loopback,
        color: "#E53935",
    },
    GraphNodeCategoryColor {
        kind: GraphNodeCategory::Apipa,
        color: "#FF9800",
    },
    GraphNodeCategoryColor {
        kind: GraphNodeCategory::LinkLocal,
        color: "#FF5722",
    },
    GraphNodeCategoryColor {
        kind: GraphNodeCategory::Ula,
        color: "#9C27B0",
    },
    GraphNodeCategoryColor {
        kind: GraphNodeCategory::Documentation,
        color: "#9E9E9E",
    },
    GraphNodeCategoryColor {
        kind: GraphNodeCategory::Broadcast,
        color: "#EC407A",
    },
    GraphNodeCategoryColor {
        kind: GraphNodeCategory::Layer2,
        color: "#00BCD4",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn une_entree_par_categorie_sans_doublon() {
        let mut kinds: Vec<_> = GRAPH_NODE_CATEGORIES.iter().map(|c| c.kind).collect();
        kinds.sort();
        kinds.dedup();
        assert_eq!(kinds.len(), GRAPH_NODE_CATEGORIES.len());
    }
}
