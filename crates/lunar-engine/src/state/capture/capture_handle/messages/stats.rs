//! Statistiques de capture partagées entre threads (atomiques, hors canal de
//! données) et payload `Stats` émis au frontend.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crate::ipc::Channel;
use serde::Serialize;

use crate::events::CaptureEvent;

/// Dernières stats pcap connues, partagées entre le thread de capture (écrit)
/// et le thread de processing (lit sur timer). Hors du canal de données pour
/// rester fiables sous backpressure — un canal plein perdrait précisément les
/// stats au moment où elles sont les plus utiles.
#[derive(Debug, Default)]
pub struct SharedCaptureStats {
    /// Paquets reçus par pcap (`ps_recv`) au dernier sondage, cumul depuis l'ouverture.
    received: AtomicU32,
    /// Paquets perdus par le noyau, tampon de capture plein (`ps_drop`).
    dropped: AtomicU32,
    /// Paquets perdus par l'interface ou son pilote (`ps_ifdrop`).
    if_dropped: AtomicU32,
}

impl SharedCaptureStats {
    /// Publie le dernier relevé pcap. Appelé par le thread de capture à chaque
    /// sondage (toutes les 250 ms) et une dernière fois avant sa sortie, pour
    /// que le récapitulatif final ne lise pas des compteurs périmés.
    pub fn store(&self, stat: pcap::Stat) {
        self.received.store(stat.received, Ordering::Relaxed);
        self.dropped.store(stat.dropped, Ordering::Relaxed);
        self.if_dropped.store(stat.if_dropped, Ordering::Relaxed);
    }

    /// Snapshot courant ; `app_dropped` est complété par l'appelant.
    pub fn load(&self) -> StatTriple {
        StatTriple {
            received: self.received.load(Ordering::Relaxed),
            dropped: self.dropped.load(Ordering::Relaxed),
            if_dropped: self.if_dropped.load(Ordering::Relaxed),
            app_dropped: 0,
        }
    }
}

/// Pertes de paquets côté application (en plus des drops kernel de pcap) :
/// pool de buffers épuisé, canal capture→processing plein, ou paquet accepté
/// abandonné après une erreur fatale d'intégrité. Incrémentés par les workers,
/// lus par le thread de processing pour les stats.
#[derive(Debug, Default)]
pub struct AppDropCounters {
    /// Paquets refusés faute de buffer dans le pool (budget d'allocation épuisé).
    pub no_buffer: AtomicU64,
    /// Paquets refusés parce que le canal capture→processing était plein.
    pub channel_full: AtomicU64,
    /// Paquets déjà acceptés dans le canal mais abandonnés après une erreur
    /// d'intégrité : retenter matrice/graphe serait incorrect, mais ils ne
    /// doivent pas disparaître du récapitulatif final.
    processing_fatal: AtomicU64,
}

impl AppDropCounters {
    /// Compte un paquet refusé faute de buffer dans le pool.
    #[inline]
    pub fn add_no_buffer(&self) {
        self.no_buffer.fetch_add(1, Ordering::Relaxed);
    }

    /// Compte un paquet refusé par un canal plein.
    #[inline]
    pub fn add_channel_full(&self) {
        self.channel_full.fetch_add(1, Ordering::Relaxed);
    }

    /// Ajoute `count` paquets acceptés puis abandonnés après une erreur fatale
    /// d'intégrité : le paquet qui a révélé le poison plus le reste du canal.
    #[inline]
    pub fn add_processing_fatal(&self, count: u64) {
        self.processing_fatal.fetch_add(count, Ordering::Relaxed);
    }

    /// Total des pertes applicatives (pool + canal + arrêt fatal).
    #[inline]
    pub fn total(&self) -> u64 {
        self.no_buffer.load(Ordering::Relaxed)
            + self.channel_full.load(Ordering::Relaxed)
            + self.processing_fatal.load(Ordering::Relaxed)
    }
}

/// Compteurs pcap d'un relevé (reçus, perdus noyau, perdus interface),
/// complétés des pertes applicatives. Les trois premiers sont les cumuls
/// `u32` rendus par pcap ; `app_dropped` est le total de `AppDropCounters`,
/// à zéro tant que l'appelant ne l'a pas renseigné.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StatTriple {
    /// Paquets reçus par pcap depuis l'ouverture de la capture.
    pub received: u32,
    /// Paquets perdus par le noyau (tampon de capture plein).
    pub dropped: u32,
    /// Paquets perdus par l'interface ou son pilote.
    pub if_dropped: u32,
    /// Pertes côté application : pool épuisé, canal plein, arrêt fatal.
    pub app_dropped: u64,
}

impl From<pcap::Stat> for StatTriple {
    fn from(s: pcap::Stat) -> Self {
        Self {
            received: s.received,
            dropped: s.dropped,
            if_dropped: s.if_dropped,
            app_dropped: 0,
        }
    }
}

/// Photographie complète des compteurs de capture à un instant donné : stats
/// pcap + pertes applicatives ([`StatTriple`]) et erreurs de parsing. C'est
/// l'unité de déduplication des événements `Stats`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StatsSnapshot {
    /// Compteurs pcap et pertes applicatives relevés au même instant.
    pub triple: StatTriple,
    /// Rejets du parseur par catégorie fine, même partition que
    /// l'import de fichier : capture coupée au snaplen, type de liaison
    /// inconnu, trame malformée sur le réseau observé.
    pub rejected_truncated: u64,
    /// Rejets pour type de liaison sans décodeur.
    pub rejected_unsupported_link_type: u64,
    /// Rejets pour trame malformée.
    pub rejected_malformed: u64,
    /// Flux distincts de la matrice (lignes du relevé).
    pub processed: u32,
}

/// Données de l'événement `Stats` tel que le reçoit le frontend : un
/// [`StatsSnapshot`] mis à plat et estampillé par la session émettrice.
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsPayload {
    /// Session de capture émettrice, reprise dans chaque événement.
    pub session_id: u64,
    /// Paquets reçus par pcap.
    pub received: u32,
    /// Paquets perdus par le noyau.
    pub dropped: u32,
    /// Paquets perdus par l'interface ou son pilote.
    pub if_dropped: u32,
    /// Pertes applicatives (pool épuisé, canal plein, arrêt fatal).
    pub app_dropped: u64,
    /// Rejets : capture coupée au snaplen.
    pub rejected_truncated: u64,
    /// Rejets : type de liaison sans décodeur.
    pub rejected_unsupported_link_type: u64,
    /// Rejets : trame malformée.
    pub rejected_malformed: u64,
    /// Flux distincts de la matrice (lignes du relevé).
    pub processed: u32,
}

impl StatsPayload {
    /// Met à plat `snapshot` pour la session `session_id`.
    #[inline]
    pub fn new(snapshot: StatsSnapshot, session_id: u64) -> Self {
        Self {
            session_id,
            received: snapshot.triple.received,
            dropped: snapshot.triple.dropped,
            if_dropped: snapshot.triple.if_dropped,
            app_dropped: snapshot.triple.app_dropped,
            rejected_truncated: snapshot.rejected_truncated,
            rejected_unsupported_link_type: snapshot.rejected_unsupported_link_type,
            rejected_malformed: snapshot.rejected_malformed,
            processed: snapshot.processed,
        }
    }

    /// Envoie immédiatement le payload (aucune déduplication ici).
    #[inline]
    pub fn send(&self, ch: &Channel<CaptureEvent>) -> Result<(), crate::ipc::ChannelError> {
        ch.send(CaptureEvent::Stats {
            session_id: self.session_id,
            received: self.received,
            dropped: self.dropped,
            if_dropped: self.if_dropped,
            app_dropped: self.app_dropped,
            rejected_truncated: self.rejected_truncated,
            rejected_unsupported_link_type: self.rejected_unsupported_link_type,
            rejected_malformed: self.rejected_malformed,
            processed: self.processed,
        })
    }

    /// N'envoie que si le snapshot diffère du dernier émis. Tous les
    /// compteurs participent à la déduplication : une matrice qui grandit
    /// sans nouvelle perte comme un paquet illisible de plus doivent
    /// réémettre.
    #[inline]
    pub fn maybe_send(
        last: &mut Option<StatsSnapshot>,
        current: StatsSnapshot,
        session_id: u64,
        ch: &Channel<CaptureEvent>,
    ) -> Result<(), crate::ipc::ChannelError> {
        if last.as_ref() == Some(&current) {
            return Ok(());
        }
        *last = Some(current);
        StatsPayload::new(current, session_id).send(ch)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    /// Canal de test qui compte les événements envoyés.
    fn counting_channel() -> (Channel<CaptureEvent>, Arc<AtomicUsize>) {
        let sent = Arc::new(AtomicUsize::new(0));
        let counter = sent.clone();
        let ch = Channel::new(move |_| {
            counter.fetch_add(1, Ordering::Relaxed);
        });
        (ch, sent)
    }

    /// Tous les compteurs participent à la déduplication : un snapshot
    /// identique n'est pas réémis, mais un paquet illisible ou intégré de
    /// plus doit réémettre — sinon le récapitulatif serait invisible.
    #[test]
    fn maybe_send_deduplicates_on_the_full_snapshot() {
        let (ch, sent) = counting_channel();
        let mut last = None;
        let mut snapshot = StatsSnapshot {
            triple: StatTriple {
                received: 10,
                ..StatTriple::default()
            },
            rejected_truncated: 0,
            rejected_unsupported_link_type: 0,
            rejected_malformed: 0,
            processed: 4,
        };

        StatsPayload::maybe_send(&mut last, snapshot, 1, &ch).unwrap();
        StatsPayload::maybe_send(&mut last, snapshot, 1, &ch).unwrap();
        assert_eq!(
            sent.load(Ordering::Relaxed),
            1,
            "snapshot identique dédupliqué"
        );

        snapshot.rejected_malformed += 1;
        StatsPayload::maybe_send(&mut last, snapshot, 1, &ch).unwrap();
        assert_eq!(
            sent.load(Ordering::Relaxed),
            2,
            "un paquet illisible de plus doit réémettre"
        );
    }
}
