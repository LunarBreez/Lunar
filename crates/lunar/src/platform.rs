//! Contournements de plateforme indispensables à gpui 0.2.2.
//!
//! Voir `docs/gpui-plateforme.md` : sans [`install_x11_wakeup`], une fenêtre
//! ouverte sur un serveur X sans gestionnaire de fenêtres (Xvfb nu) n'est
//! plus jamais redessinée après la première image ; sans
//! [`force_x11_backend`], le backend Wayland natif cesse de présenter des
//! images sur un graphe lourd.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use gpui::App;

/// Intervalle entre deux réveils X11.
const WAKEUP_PERIOD: Duration = Duration::from_millis(100);

/// Nombre maximal de réveils (2 min au total).
///
/// La première image peut demander plusieurs secondes en rendu logiciel à
/// froid (compilation des pipelines par lavapipe) : 8 s sur un runner GitHub
/// à 4 cœurs. Si les réveils s'arrêtent avant que la boucle de
/// rafraîchissement de gpui ait démarré, la fenêtre reste figée pour de bon
/// (première version : 10 s, dépassés en CI).
const WAKEUP_ATTEMPTS: u32 = 1_200;

/// Installe le réveil X11 de [`x11_wakeup`] sur l'application.
///
/// La vue principale incrémente `renders` à chaque rendu. On réveille la
/// boucle d'événements toutes les 100 ms jusqu'à ce que la vue ait été
/// redessinée par la boucle de rafraîchissement (deuxième rendu), au plus
/// 2 min ([`WAKEUP_ATTEMPTS`]).
pub fn install_x11_wakeup(cx: &mut App, renders: &'static AtomicU32) {
    cx.spawn(async move |cx| {
        for _ in 0..WAKEUP_ATTEMPTS {
            cx.background_executor().timer(WAKEUP_PERIOD).await;
            if renders.load(Ordering::Relaxed) >= 2 {
                return;
            }
            x11_wakeup();
        }
        log::warn!(
            "[X11] boucle de rafraîchissement toujours arrêtée après {} réveils",
            WAKEUP_ATTEMPTS
        );
    })
    .detach();
}

/// Réveille la boucle d'événements X11 de gpui 0.2.2.
///
/// gpui ne lit la socket X que lorsqu'elle devient lisible. Les événements
/// reçus pendant une attente de réponse (MapNotify, Expose à l'ouverture)
/// restent dans la file XCB. Sans gestionnaire de fenêtres (Xvfb nu), plus
/// rien n'arrive ensuite, et la boucle de rafraîchissement ne démarre jamais.
/// Un PropertyNotify envoyé par une seconde connexion, pendant que gpui
/// attend, rend la socket lisible et fait traiter toute la file. gpui 0.2.2
/// n'expose pas l'identifiant X de ses fenêtres (`window_handle` non
/// implémenté) : on les retrouve par `_NET_WM_PID`.
#[cfg(target_os = "linux")]
pub fn x11_wakeup() {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, PropMode};
    use x11rb::wrapper::ConnectionExt as _;
    let Ok((conn, screen)) = x11rb::connect(None) else {
        return;
    };
    let root = conn.setup().roots[screen].root;
    let intern = |name: &[u8]| {
        conn.intern_atom(false, name)
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|r| r.atom)
    };
    let (Some(pid_atom), Some(wake_atom)) = (intern(b"_NET_WM_PID"), intern(b"_LUNAR_WAKEUP"))
    else {
        return;
    };
    let Some(tree) = conn.query_tree(root).ok().and_then(|c| c.reply().ok()) else {
        return;
    };
    let me = std::process::id();
    for child in tree.children {
        let pid = conn
            .get_property(false, child, pid_atom, AtomEnum::CARDINAL, 0, 1)
            .ok()
            .and_then(|c| c.reply().ok())
            .and_then(|r| r.value32().and_then(|mut v| v.next()));
        if pid == Some(me) {
            let _ =
                conn.change_property8(PropMode::REPLACE, child, wake_atom, AtomEnum::STRING, b"1");
        }
    }
    let _ = conn.flush();
}

/// Réveil X11 : sans objet hors Linux.
#[cfg(not(target_os = "linux"))]
pub fn x11_wakeup() {}

/// Force le backend X11 (XWayland sous une session Wayland).
///
/// Le backend Wayland de gpui 0.2.2 ne présente plus d'image après la
/// première sur un graphe lourd (5 000 nœuds, 10 000 arêtes), alors que X11
/// tient 26 i/s sur le même poste (mesure du 19/11/2026).
///
/// À appeler au tout début de `main`, avant la création de tout thread.
pub fn force_x11_backend() {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() && std::env::var_os("DISPLAY").is_some() {
        // SAFETY: appelé au tout début de `main`, avant la création de tout
        // thread ; aucune lecture concurrente de l'environnement n'est possible.
        unsafe { std::env::remove_var("WAYLAND_DISPLAY") };
    }
}
