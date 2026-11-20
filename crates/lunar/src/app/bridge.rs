//! Pont entre l'interface gpui et le moteur [`Lunar`].
//!
//! Les méthodes de `Lunar` sont bloquantes (imports, ouverture de projet,
//! arrêt de capture) : [`Bridge::call`] les exécute sur l'exécuteur de fond
//! de gpui et rend une [`Task`] ; [`Bridge::run`] (et [`Bridge::run_in`],
//! avec la fenêtre) ramène en plus le résultat sur le thread de rendu, dans
//! l'entité appelante.
//!
//! Les événements du moteur ([`CaptureEvent`]) partent sur un
//! [`Channel`] : [`Bridge::store_channel`] en crée un étiqueté dont les
//! événements atteignent le store de capture (pompe de
//! [`crate::app::state::AppState`]). Créer un nouveau canal du store
//! détache le précédent :
//! ses retardataires sont ignorés. [`Bridge::detached_channel`] rend un
//! canal dont les événements sont jetés (arrêt de capture à la fermeture).
//!
//! Le rappel d'un `Channel` peut s'exécuter sous les verrous du moteur : il
//! ne fait qu'empiler l'événement dans une file non bornée, jamais d'appel
//! au moteur.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use gpui::{Context, Task, Window};
use lunar_engine::Lunar;
use lunar_engine::events::CaptureEvent;
use lunar_engine::ipc::{Channel, ChannelError};

/// Étiquette du canal émetteur d'un événement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChannelTag(u64);

/// Étiquette des canaux détachés dès leur création.
const DETACHED: ChannelTag = ChannelTag(0);

/// Événement du moteur et canal qui l'a porté.
#[derive(Debug)]
pub struct TaggedEvent {
    /// Canal émetteur.
    pub tag: ChannelTag,
    /// Événement.
    pub event: CaptureEvent,
}

/// Accès partagé au moteur, clonable d'une vue à l'autre.
#[derive(Clone)]
pub struct Bridge {
    /// Moteur.
    lunar: Arc<Lunar>,
    /// File des événements vers la pompe du thread de rendu.
    events: UnboundedSender<TaggedEvent>,
    /// Dernière étiquette attribuée.
    last_tag: Rc<Cell<u64>>,
    /// Étiquette du canal courant du store (0 : aucun).
    store_tag: Rc<Cell<u64>>,
}

impl Bridge {
    /// Pont et récepteur des événements, à confier à la pompe.
    pub fn new(lunar: Arc<Lunar>) -> (Self, UnboundedReceiver<TaggedEvent>) {
        let (events, receiver) = unbounded();
        (
            Self {
                lunar,
                events,
                last_tag: Rc::new(Cell::new(0)),
                store_tag: Rc::new(Cell::new(0)),
            },
            receiver,
        )
    }

    /// Moteur, pour un appel non bloquant (lecture d'un chemin…).
    pub fn lunar(&self) -> &Arc<Lunar> {
        &self.lunar
    }

    /// Canal portant `tag` vers la pompe.
    fn channel(&self, tag: ChannelTag) -> Channel<CaptureEvent> {
        let events = self.events.clone();
        Channel::new_fallible(move |event| {
            events
                .unbounded_send(TaggedEvent { tag, event })
                .map_err(|_| ChannelError::new("interface fermée"))
        })
    }

    /// Nouveau canal du store : ses événements alimentent le store de
    /// capture et le graphe ; le canal précédent est détaché.
    pub fn store_channel(&self) -> Channel<CaptureEvent> {
        let tag = self.last_tag.get() + 1;
        self.last_tag.set(tag);
        self.store_tag.set(tag);
        self.channel(ChannelTag(tag))
    }

    /// Canal courant du store, sans en ouvrir de nouveau
    /// : l'arrêt d'une capture y publie
    /// `Stopped` et le dernier bilan. Sans canal du store, il est détaché.
    pub fn current_store_channel(&self) -> Channel<CaptureEvent> {
        self.channel(ChannelTag(self.store_tag.get()))
    }

    /// Canal dont les événements sont ignorés.
    pub fn detached_channel(&self) -> Channel<CaptureEvent> {
        self.channel(DETACHED)
    }

    /// Vrai si `tag` est le canal courant du store.
    pub fn is_store_tag(&self, tag: ChannelTag) -> bool {
        tag != DETACHED && tag.0 == self.store_tag.get()
    }

    /// Exécute `work` sur l'exécuteur de fond.
    pub fn call<T, R>(
        &self,
        cx: &mut Context<T>,
        work: impl FnOnce(&Lunar) -> R + Send + 'static,
    ) -> Task<R>
    where
        T: 'static,
        R: Send + 'static,
    {
        let lunar = Arc::clone(&self.lunar);
        cx.background_executor().spawn(async move { work(&lunar) })
    }

    /// Exécute `work` en fond puis `done` dans l'entité appelante, sur le
    /// thread de rendu. Entité disparue entre-temps : `done` n'est pas
    /// appelé. La tâche rendue peut être détachée.
    pub fn run<T, R>(
        &self,
        cx: &mut Context<T>,
        work: impl FnOnce(&Lunar) -> R + Send + 'static,
        done: impl FnOnce(&mut T, R, &mut Context<T>) + 'static,
    ) -> Task<()>
    where
        T: 'static,
        R: Send + 'static,
    {
        let task = self.call(cx, work);
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| done(this, result, cx));
        })
    }

    /// [`Self::run`] avec la fenêtre dans `done` (focus, modale…).
    pub fn run_in<T, R>(
        &self,
        window: &Window,
        cx: &mut Context<T>,
        work: impl FnOnce(&Lunar) -> R + Send + 'static,
        done: impl FnOnce(&mut T, R, &mut Window, &mut Context<T>) + 'static,
    ) -> Task<()>
    where
        T: 'static,
        R: Send + 'static,
    {
        let task = self.call(cx, work);
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| done(this, result, window, cx));
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    fn bridge() -> (Bridge, UnboundedReceiver<TaggedEvent>, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let lunar = Lunar::new(lunar_engine::AppPaths::under(dir.path())).expect("moteur");
        let (bridge, receiver) = Bridge::new(Arc::new(lunar));
        (bridge, receiver, dir)
    }

    fn progress() -> CaptureEvent {
        CaptureEvent::ImportProgress {
            file_name: "a.pcap".into(),
            file_index: 1,
            files_total: 1,
            current: 1,
            total: 2,
        }
    }

    #[test]
    fn un_nouveau_canal_du_store_detache_le_precedent() {
        let (bridge, mut receiver, _dir) = bridge();
        let first = bridge.store_channel();
        let second = bridge.store_channel();
        first.send(progress()).expect("envoi");
        second.send(progress()).expect("envoi");
        bridge.detached_channel().send(progress()).expect("envoi");
        let tags: Vec<bool> = futures::executor::block_on(async {
            let mut tags = Vec::new();
            for _ in 0..3 {
                if let Some(tagged) = receiver.next().await {
                    tags.push(bridge.is_store_tag(tagged.tag));
                }
            }
            tags
        });
        assert_eq!(tags, [false, true, false]);
    }

    #[test]
    fn canal_courant_du_store_garde_son_etiquette() {
        let (bridge, mut receiver, _dir) = bridge();
        bridge
            .current_store_channel()
            .send(progress())
            .expect("envoi");
        let _start = bridge.store_channel();
        bridge
            .current_store_channel()
            .send(progress())
            .expect("envoi");
        let tags: Vec<bool> = futures::executor::block_on(async {
            let mut tags = Vec::new();
            for _ in 0..2 {
                if let Some(tagged) = receiver.next().await {
                    tags.push(bridge.is_store_tag(tagged.tag));
                }
            }
            tags
        });
        assert_eq!(tags, [false, true], "sans canal du store, détaché");
    }

    #[test]
    fn aucun_canal_du_store_au_depart() {
        let (bridge, _receiver, _dir) = bridge();
        assert!(!bridge.is_store_tag(ChannelTag(0)));
        assert!(!bridge.is_store_tag(ChannelTag(1)));
    }

    #[test]
    fn interface_fermee_signalee_au_moteur() {
        let (bridge, receiver, _dir) = bridge();
        drop(receiver);
        let err = bridge.store_channel().send(progress());
        assert!(err.is_err(), "l'envoi vers une interface fermée échoue");
    }
}
