//! Canal d'événements vers l'interface.
//!
//! Le moteur pousse des [`crate::events::CaptureEvent`] possédés ; l'interface
//! fournit la fonction de livraison (file vers la boucle GPUI, enregistreur de
//! test…). Un [`Channel`] se clone à bas coût (les clones partagent la même
//! livraison et le même identifiant) et se transmet entre threads : le
//! pipeline de capture le garde pour toute la session.
//!
//! Aucune sérialisation JSON à l'envoi : la valeur est livrée telle quelle.
//! Les tests qui vérifient la forme JSON la reconstruisent avec
//! `serde_json::to_value`.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

/// Échec de livraison d'un événement : le destinataire n'écoute plus (fenêtre
/// fermée, récepteur abandonné). Retournée par [`Channel::send`] ; convertie
/// en `CaptureError::EventSendError`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ChannelError {
    /// Motif lisible de l'échec, repris dans le message d'erreur.
    message: String,
}

impl ChannelError {
    /// Erreur de livraison portant `message`.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// Motif de l'échec.
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// Fonction de livraison partagée par tous les clones d'un même canal.
type Deliver<T> = dyn Fn(T) -> Result<(), ChannelError> + Send + Sync;

/// Prochain identifiant attribué à un canal créé (distingue deux
/// abonnements).
static NEXT_CHANNEL_ID: AtomicU32 = AtomicU32::new(1);

/// Canal typé vers l'interface. Voir le module.
pub struct Channel<T> {
    /// Identifiant unique du canal, partagé par ses clones.
    id: u32,
    /// Livraison fournie à la création.
    deliver: Arc<Deliver<T>>,
}

impl<T> Clone for Channel<T> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            deliver: Arc::clone(&self.deliver),
        }
    }
}

impl<T> fmt::Debug for Channel<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Channel").field("id", &self.id).finish()
    }
}

impl<T> Channel<T> {
    /// Canal dont la livraison n'échoue jamais : `f` reçoit chaque valeur.
    ///
    /// `f` est appelée sur le thread émetteur (commande, thread de
    /// traitement de la capture), parfois pendant que le moteur tient ses
    /// verrous de données. Elle doit donc être brève et ne jamais rappeler la
    /// façade : pousser la valeur dans une
    /// file (voir [`Self::from_sender`]) et la traiter ailleurs.
    pub fn new(f: impl Fn(T) + Send + Sync + 'static) -> Self {
        Self::new_fallible(move |value| {
            f(value);
            Ok(())
        })
    }

    /// Canal dont la livraison peut échouer : une erreur de `f` remonte à
    /// l'émetteur par [`Self::send`].
    pub fn new_fallible(f: impl Fn(T) -> Result<(), ChannelError> + Send + Sync + 'static) -> Self {
        Self {
            id: NEXT_CHANNEL_ID.fetch_add(1, Ordering::Relaxed),
            deliver: Arc::new(f),
        }
    }

    /// Identifiant du canal, identique pour tous ses clones.
    pub fn id(&self) -> u32 {
        self.id
    }

    /// Livre `value` au destinataire.
    pub fn send(&self, value: T) -> Result<(), ChannelError> {
        (self.deliver)(value)
    }
}

impl<T: Send + 'static> Channel<T> {
    /// Canal qui pousse chaque valeur dans une file crossbeam : l'interface
    /// la vide depuis son propre thread. La livraison échoue quand le
    /// récepteur a été abandonné.
    pub fn from_sender(sender: crossbeam::channel::Sender<T>) -> Self {
        Self::new_fallible(move |value| {
            sender
                .send(value)
                .map_err(|_| ChannelError::new("le destinataire des événements est fermé"))
        })
    }
}

/// Enregistreurs pour les tests : ils relèvent les événements reçus.
#[cfg(test)]
pub(crate) mod test_support {
    use super::Channel;
    use std::sync::{Arc, Mutex};

    /// Événements reçus, sous leur forme JSON.
    pub(crate) type Recorded = Arc<Mutex<Vec<serde_json::Value>>>;

    /// Canal qui enregistre chaque valeur sérialisée en JSON.
    pub(crate) fn json_recorder<T: serde::Serialize>() -> (Channel<T>, Recorded) {
        let recorded: Recorded = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&recorded);
        let channel = Channel::new(move |value: T| {
            sink.lock()
                .unwrap()
                .push(serde_json::to_value(&value).unwrap());
        });
        (channel, recorded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clones_share_id_and_delivery() {
        let (channel, recorded) = test_support::json_recorder::<u32>();
        let clone = channel.clone();
        assert_eq!(channel.id(), clone.id());
        channel.send(1).unwrap();
        clone.send(2).unwrap();
        assert_eq!(*recorded.lock().unwrap(), vec![1, 2]);
        assert_ne!(Channel::<u32>::new(|_| {}).id(), channel.id());
    }

    #[test]
    fn closed_sender_reports_a_channel_error() {
        let (tx, rx) = crossbeam::channel::unbounded::<u32>();
        let channel = Channel::from_sender(tx);
        channel.send(7).unwrap();
        assert_eq!(rx.recv().unwrap(), 7);
        drop(rx);
        assert!(channel.send(8).is_err());
    }
}
