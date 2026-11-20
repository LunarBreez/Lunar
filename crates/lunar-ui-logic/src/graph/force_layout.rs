//! Contrôleur de la mise en page ForceAtlas2 : (re)création adaptée à la
//! taille du graphe, démarrage/arrêt liés au bouton « Gravité », arrêt
//! automatique au bout de [`FORCE_LAYOUT_MAX_RUN`]. Encapsule le
//! superviseur et l'ordre du graphe vu à sa création, puisque slowDown et
//! Barnes-Hut en dépendent. Le superviseur réel est
//! [`super::fa2_supervisor::Fa2Supervisor`].
//!
//! L'arrêt automatique est une échéance relevée par
//! [`ForceLayoutController::poll`] (appelé à chaque frame), sur une horloge
//! remplaçable dans les tests.

use std::time::{Duration, Instant};

/// Durée au-delà de laquelle la mise en page s'arrête d'elle-même.
/// ForceAtlas2 ne s'arrête jamais seul : sans cette limite, il tournerait
/// indéfiniment.
pub const FORCE_LAYOUT_MAX_RUN: Duration = Duration::from_secs(20 * 60);

/// Ce que le contrôleur attend d'un superviseur ForceAtlas2.
pub trait LayoutSupervisor {
    /// Démarre (ou reprend) les itérations.
    fn start(&mut self);
    /// Suspend les itérations.
    fn stop(&mut self);
    /// Détruit le superviseur : il ne redémarrera plus.
    fn kill(&mut self);
}

/// Fabrique de superviseur, réglé pour l'ordre (nombre de nœuds) du graphe.
pub type SupervisorFactory<S> = Box<dyn FnMut(usize) -> S + Send>;

/// Horloge monotone, remplacée dans les tests.
pub type Clock = Box<dyn Fn() -> Instant + Send>;

/// Contrôleur de la mise en page.
///
/// Les méthodes reçoivent l'ordre courant du graphe (nombre de nœuds).
pub struct ForceLayoutController<S: LayoutSupervisor> {
    /// Crée le superviseur, réglé pour la taille actuelle du graphe.
    create_supervisor: SupervisorFactory<S>,
    /// Horloge de l'arrêt automatique.
    clock: Clock,
    /// Durée de fonctionnement avant l'arrêt automatique.
    max_run: Duration,
    /// Superviseur courant.
    layout: Option<S>,
    /// Ordre du graphe au dernier `inferSettings`.
    last_order: usize,
    /// Échéance de l'arrêt automatique, si armé.
    deadline: Option<Instant>,
}

impl<S: LayoutSupervisor> ForceLayoutController<S> {
    /// Contrôleur à l'horloge système et à l'arrêt automatique par défaut.
    pub fn new(create_supervisor: SupervisorFactory<S>) -> Self {
        Self {
            create_supervisor,
            clock: Box::new(Instant::now),
            max_run: FORCE_LAYOUT_MAX_RUN,
            layout: None,
            last_order: 0,
            deadline: None,
        }
    }

    /// Remplace l'horloge (tests).
    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    /// Remplace la durée avant l'arrêt automatique.
    pub fn with_max_run(mut self, max_run: Duration) -> Self {
        self.max_run = max_run;
        self
    }

    /// Désarme l'arrêt automatique.
    fn disarm(&mut self) {
        self.deadline = None;
    }

    /// Chaque démarrage, ou reprise, repart pour `max_run` : passé ce délai,
    /// [`Self::poll`] arrête la mise en page et le signale.
    fn run(&mut self) {
        self.disarm();
        if let Some(layout) = self.layout.as_mut() {
            layout.start();
        }
        self.deadline = Some((self.clock)() + self.max_run);
    }

    /// Détruit le superviseur courant.
    pub fn kill(&mut self) {
        self.disarm();
        if let Some(mut layout) = self.layout.take() {
            layout.kill();
            self.last_order = 0;
        }
    }

    /// (Re)crée le superviseur pour `order` nœuds et le démarre.
    pub fn start(&mut self, order: usize) {
        // ⚠️ Jamais sur un graphe vide : inferSettings y produit
        // slowDown = 1 + ln(0) = -∞ et fige la simulation pour toujours.
        if order == 0 {
            return;
        }
        self.kill();
        self.last_order = order;
        self.layout = Some((self.create_supervisor)(order));
        self.run();
    }

    /// Suspend la mise en page (bouton « Gravité » sur OFF).
    pub fn stop(&mut self) {
        self.disarm();
        if let Some(layout) = self.layout.as_mut() {
            layout.stop();
        }
    }

    /// Le graphe a assez grandi depuis le dernier inferSettings pour que
    /// slowDown/Barnes-Hut (dépendants de la taille) soient obsolètes.
    fn outgrown(&self, order: usize) -> bool {
        self.layout.is_none() || self.last_order == 0 || order >= self.last_order * 4
    }

    /// Reprend un layout existant, ou le recrée s'il n'y en a pas.
    ///
    /// Le graphe peut avoir grandi pendant la pause (Gravité désactivée :
    /// [`Self::ensure_settings`] n'est alors pas appelé), d'où le même
    /// contrôle plutôt qu'une reprise aveugle de réglages obsolètes.
    pub fn resume(&mut self, order: usize) {
        if self.layout.is_some() && !self.outgrown(order) {
            self.run();
        } else {
            self.start(order);
        }
    }

    /// Recrée le layout si le graphe vient de se peupler ou a beaucoup grandi
    /// (×4) depuis le dernier inferSettings.
    pub fn ensure_settings(&mut self, order: usize) {
        if order == 0 {
            return;
        }
        if self.outgrown(order) {
            self.start(order);
        }
    }

    /// Relève l'échéance de l'arrêt automatique : passée, la mise en page est
    /// suspendue et `poll` retourne vrai (l'interface repasse alors
    /// « Gravité » sur OFF).
    pub fn poll(&mut self) -> bool {
        match self.deadline {
            Some(deadline) if (self.clock)() >= deadline => {
                self.deadline = None;
                if let Some(layout) = self.layout.as_mut() {
                    layout.stop();
                }
                true
            }
            _ => false,
        }
    }

    /// Échéance de l'arrêt automatique en attente.
    pub fn auto_stop_deadline(&self) -> Option<Instant> {
        self.deadline
    }

    /// Superviseur courant.
    pub fn supervisor(&self) -> Option<&S> {
        self.layout.as_ref()
    }

    /// Superviseur courant, modifiable (pour le faire itérer).
    pub fn supervisor_mut(&mut self) -> Option<&mut S> {
        self.layout.as_mut()
    }

    /// Ordre du graphe au dernier inferSettings (0 sans superviseur).
    pub fn last_order(&self) -> usize {
        self.last_order
    }
}

impl<S: LayoutSupervisor> std::fmt::Debug for ForceLayoutController<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ForceLayoutController")
            .field("has_layout", &self.layout.is_some())
            .field("last_order", &self.last_order)
            .field("deadline", &self.deadline)
            .field("max_run", &self.max_run)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Horloge figée, avancée à la main.
    #[derive(Clone)]
    pub(crate) struct FakeClock(pub(crate) Arc<Mutex<Instant>>);

    impl FakeClock {
        pub(crate) fn new() -> Self {
            Self(Arc::new(Mutex::new(Instant::now())))
        }

        pub(crate) fn now(&self) -> Instant {
            *self.0.lock().unwrap()
        }

        pub(crate) fn advance(&self, by: Duration) {
            *self.0.lock().unwrap() += by;
        }

        pub(crate) fn clock(&self) -> Clock {
            let shared = self.clone();
            Box::new(move || shared.now())
        }
    }

    /// Superviseur factice qui journalise ses appels.
    struct FakeSupervisor(Arc<Mutex<Vec<&'static str>>>);

    impl LayoutSupervisor for FakeSupervisor {
        fn start(&mut self) {
            self.0.lock().unwrap().push("start");
        }
        fn stop(&mut self) {
            self.0.lock().unwrap().push("stop");
        }
        fn kill(&mut self) {
            self.0.lock().unwrap().push("kill");
        }
    }

    struct Setup {
        calls: Arc<Mutex<Vec<&'static str>>>,
        clock: FakeClock,
        controller: ForceLayoutController<FakeSupervisor>,
        /// Ordre du graphe (un nœud « a »).
        order: usize,
    }

    impl Setup {
        fn calls(&self) -> Vec<&'static str> {
            self.calls.lock().unwrap().clone()
        }

        /// Fait expirer la minuterie en cours (`timers.elapse()`) ; retourne
        /// le nombre d'arrêts automatiques signalés.
        fn elapse(&mut self) -> usize {
            let Some(deadline) = self.controller.auto_stop_deadline() else {
                return 0;
            };
            let now = self.clock.now();
            if deadline > now {
                self.clock.advance(deadline - now);
            }
            usize::from(self.controller.poll())
        }
    }

    fn setup() -> Setup {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let clock = FakeClock::new();
        let shared = calls.clone();
        let controller =
            ForceLayoutController::new(Box::new(move |_order| FakeSupervisor(shared.clone())))
                .with_clock(clock.clock());
        Setup {
            calls,
            clock,
            controller,
            order: 1,
        }
    }

    #[test]
    fn mise_en_page_arret_automatique_au_bout_de_20_minutes() {
        assert_eq!(FORCE_LAYOUT_MAX_RUN, Duration::from_millis(20 * 60 * 1000));
        let mut s = setup();

        s.controller.start(s.order);
        assert_eq!(s.calls(), ["start"]);
        assert_eq!(
            s.controller.auto_stop_deadline().map(|d| d - s.clock.now()),
            Some(FORCE_LAYOUT_MAX_RUN)
        );

        // Avant l'échéance, rien ne se passe.
        s.clock
            .advance(FORCE_LAYOUT_MAX_RUN - Duration::from_millis(1));
        assert!(!s.controller.poll());

        let auto_stops = s.elapse();
        assert_eq!(s.calls(), ["start", "stop"]);
        assert_eq!(
            auto_stops, 1,
            "l'interface est prévenue pour repasser « Gravité » sur OFF"
        );
        assert_eq!(s.controller.auto_stop_deadline(), None);
    }

    #[test]
    fn mise_en_page_une_relance_repart_pour_20_minutes() {
        let mut s = setup();
        s.controller.start(s.order);
        let mut auto_stops = s.elapse();

        s.controller.resume(s.order);
        assert_eq!(s.calls(), ["start", "stop", "start"]);
        assert!(s.controller.auto_stop_deadline().is_some());
        auto_stops += s.elapse();
        assert_eq!(auto_stops, 2);
    }

    #[test]
    fn mise_en_page_arretee_ou_detruite_a_la_main_plus_d_arret_en_attente() {
        let mut stopped = setup();
        stopped.controller.start(stopped.order);
        stopped.controller.stop();
        assert_eq!(stopped.controller.auto_stop_deadline(), None);
        assert_eq!(stopped.elapse(), 0);

        let mut killed = setup();
        killed.controller.start(killed.order);
        killed.controller.kill();
        assert_eq!(killed.controller.auto_stop_deadline(), None);
        assert_eq!(killed.calls(), ["start", "kill"]);
    }

    #[test]
    fn mise_en_page_rien_ne_demarre_sur_un_graphe_vide() {
        let mut s = setup();
        s.controller.start(0);
        assert!(s.calls().is_empty());
        assert_eq!(s.controller.auto_stop_deadline(), None);
        s.controller.ensure_settings(0);
        assert!(s.calls().is_empty());
    }

    #[test]
    fn mise_en_page_recreee_quand_le_graphe_a_quadruple() {
        let mut s = setup();
        s.controller.ensure_settings(10);
        assert_eq!(s.calls(), ["start"]);
        assert_eq!(s.controller.last_order(), 10);
        // Moins de ×4 : réglages conservés.
        s.controller.ensure_settings(39);
        assert_eq!(s.calls(), ["start"]);
        // ×4 : superviseur détruit puis recréé pour la nouvelle taille.
        s.controller.ensure_settings(40);
        assert_eq!(s.calls(), ["start", "kill", "start"]);
        assert_eq!(s.controller.last_order(), 40);
    }

    #[test]
    fn mise_en_page_la_reprise_recree_un_layout_devenu_trop_petit() {
        let mut s = setup();
        s.controller.start(5);
        s.controller.stop();
        // Le graphe a grandi pendant la pause.
        s.controller.resume(20);
        assert_eq!(s.calls(), ["start", "stop", "kill", "start"]);
        assert_eq!(s.controller.last_order(), 20);
        // Reprise sans layout : création.
        s.controller.kill();
        s.controller.resume(20);
        assert_eq!(
            s.calls(),
            ["start", "stop", "kill", "start", "kill", "start"]
        );
    }
}
