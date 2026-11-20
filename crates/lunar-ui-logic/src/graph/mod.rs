//! Graphe réseau de la vue d'analyse. La logique se teste sans fenêtre.
//!
//! Le point d'entrée est [`GraphModel`] : il reçoit les `GraphUpdate` et les
//! snapshots `GraphData` du cœur, fait tourner ForceAtlas2 et expose à chaque
//! frame ce qu'un moteur de rendu doit dessiner (positions, tailles, couleurs,
//! libellés, courbures), ainsi que les tests d'atteinte et l'état de survol,
//! de sélection et de surbrillance des tunnels.
//!
//! Modules :
//!
//! | Rôle                                                | Module                |
//! |-----------------------------------------------------|-----------------------|
//! | Couleurs, tailles, labels et courbure               | [`style`]             |
//! | Mutations depuis les données du cœur, stockage      | [`sync`], [`store`]   |
//! | Contrôle de la mise en page (bouton « Gravité »)    | [`force_layout`]      |
//! | Superviseur ForceAtlas2                             | [`fa2_supervisor`]    |
//! | Algorithme ForceAtlas2                              | [`force_atlas2`]      |
//! | Famille de tunnel d'une arête                       | [`tunnel`]            |
//! | Bandeau « nœud sélectionné »                        | [`node_info`]         |
//! | Mise à jour optimiste d'un label                    | [`label_edit`]        |
//! | Légende                                             | [`legend`]            |
//! | Totaux du bandeau                                   | [`counts`]            |
//! | Couleurs des protocoles                             | [`protocol_colors`]   |
//! | Catégories de nœuds                                 | [`categories`]        |
//! | Modèle sans fenêtre, caméra                         | [`model`], [`camera`] |
//!
//! Repère : les positions sont en coordonnées « monde », indépendantes des
//! pixels ; le moteur de rendu applique la caméra (voir [`camera`]). Les
//! tailles de nœuds et d'arêtes sont des tailles d'écran
//! (pixels à un ratio de caméra de 1).

pub mod camera;
pub mod categories;
pub mod counts;
pub mod fa2_supervisor;
pub mod force_atlas2;
pub mod force_layout;
mod format;
pub mod label_edit;
pub mod legend;
pub mod model;
pub mod node_info;
pub mod protocol_colors;
pub mod rng;
pub mod spatial;
pub mod store;
pub mod style;
pub mod sync;
pub mod tunnel;

pub use model::{EdgeRender, GraphModel, LabelEditRequest, LayoutStep, NodeRender, SnapshotLoad};
pub use store::{EdgeIndex, EdgeKind, GraphEdge, GraphNode, GraphStore, NodeIndex};
pub use style::Rgb;
