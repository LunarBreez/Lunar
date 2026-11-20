//! Messages et payloads du pipeline de capture : paquets, stats partagées et
//! occupation du canal.

use crate::state::capture::capture_handle::threads::packet_buffer::PacketBuffer;

pub mod capture;
pub mod channel;
pub mod stats;

/// Le canal capture→processing ne transporte que des paquets : les stats
/// passent par [`stats::SharedCaptureStats`] pour ne jamais être perdues
/// quand le canal est plein.
pub enum CaptureMessage {
    /// Un paquet copié dans un buffer du pool ; le consommateur le rend au
    /// pool après traitement, l'émetteur immédiatement si l'envoi échoue.
    Packet(PacketBuffer),
}
