//! Représentations d'un paquet dans le pipeline : réexport du cœur
//! `lunar-flows-core` (source unique du domaine).
//! `CapturedPacket::to_owned_packets` y déplie les tunnels en
//! niveaux de flux reliés par `encap_id`.

pub use lunar_flows_core::packet::*;
