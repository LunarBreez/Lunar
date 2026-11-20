//! Notifications passagères empilées en bas à droite (information, succès,
//! avertissement, erreur), effacées automatiquement.
//!
//! Couleurs : jetons `toast_*` du thème (information, succès, avertissement,
//! erreur).
//!
//! La file ([`ToastQueue`]) est sans fenêtre et testée seule ; la vue
//! ([`ToastStack`]) programme un minuteur par notification.

use std::time::{Duration, Instant};

use gpui::{
    App, Context, CursorStyle, Hsla, IntoElement, Render, SharedString, Window, anchored, deferred,
    div, point, prelude::*, px,
};

use crate::assets::{Icon, icon};
use crate::theme::{ActiveTheme, Theme};

/// Priorité de peinture des notifications (au-dessus des listes flottantes).
pub const TOAST_PRIORITY: usize = 30;

/// Nombre maximal de notifications affichées ; les plus anciennes partent.
pub const MAX_VISIBLE: usize = 5;

/// Nature d'une notification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    /// Information.
    Info,
    /// Succès.
    Success,
    /// Avertissement.
    Warning,
    /// Erreur.
    Error,
}

impl ToastKind {
    /// Durée d'affichage par défaut : plus longue pour ce qui demande de
    /// l'attention.
    pub fn default_duration(self) -> Duration {
        match self {
            ToastKind::Info | ToastKind::Success => Duration::from_secs(4),
            ToastKind::Warning => Duration::from_secs(6),
            ToastKind::Error => Duration::from_secs(8),
        }
    }

    /// Icône de la notification.
    fn icon(self) -> Icon {
        match self {
            ToastKind::Info => Icon::Info,
            ToastKind::Success => Icon::CircleCheck,
            ToastKind::Warning => Icon::Warning,
            ToastKind::Error => Icon::AlertCircle,
        }
    }

    /// Fond, bordure et couleur d'icône.
    fn colors(self, theme: &Theme) -> (Hsla, Hsla, Hsla) {
        match self {
            ToastKind::Info => (
                theme.toast_info_bg,
                theme.toast_info_border,
                theme.toast_info_border,
            ),
            ToastKind::Success => (theme.toast_success_bg, theme.success, theme.success),
            ToastKind::Warning => (
                theme.toast_warning_bg,
                theme.toast_warning_border,
                theme.warning,
            ),
            ToastKind::Error => (theme.toast_error_bg, theme.danger, theme.danger_text),
        }
    }
}

/// Identifiant d'une notification.
pub type ToastId = u64;

/// Notification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toast {
    /// Identifiant.
    pub id: ToastId,
    /// Nature.
    pub kind: ToastKind,
    /// Message.
    pub message: SharedString,
    /// Échéance d'effacement ; `None` : reste jusqu'à fermeture.
    pub expires_at: Option<Instant>,
}

/// File de notifications, sans fenêtre.
#[derive(Clone, Debug, Default)]
pub struct ToastQueue {
    /// Notifications, de la plus ancienne à la plus récente.
    toasts: Vec<Toast>,
    /// Prochain identifiant.
    next_id: ToastId,
}

impl ToastQueue {
    /// Ajoute une notification ; au-delà de [`MAX_VISIBLE`], la plus ancienne
    /// part.
    pub fn push(
        &mut self,
        kind: ToastKind,
        message: impl Into<SharedString>,
        duration: Option<Duration>,
        now: Instant,
    ) -> ToastId {
        let id = self.next_id;
        self.next_id += 1;
        self.toasts.push(Toast {
            id,
            kind,
            message: message.into(),
            expires_at: duration.map(|d| now + d),
        });
        if self.toasts.len() > MAX_VISIBLE {
            let excess = self.toasts.len() - MAX_VISIBLE;
            self.toasts.drain(..excess);
        }
        id
    }

    /// Retire une notification ; renvoie vrai si elle existait.
    pub fn dismiss(&mut self, id: ToastId) -> bool {
        let before = self.toasts.len();
        self.toasts.retain(|t| t.id != id);
        before != self.toasts.len()
    }

    /// Retire les notifications échues à `now` ; renvoie vrai si la file a
    /// changé.
    pub fn expire(&mut self, now: Instant) -> bool {
        let before = self.toasts.len();
        self.toasts
            .retain(|t| t.expires_at.is_none_or(|deadline| deadline > now));
        before != self.toasts.len()
    }

    /// Notifications affichées.
    pub fn toasts(&self) -> &[Toast] {
        &self.toasts
    }

    /// File vide.
    pub fn is_empty(&self) -> bool {
        self.toasts.is_empty()
    }
}

/// Pile de notifications d'une fenêtre, à rendre comme enfant de la vue
/// racine (elle se place seule en bas à droite, au-dessus de tout).
///
/// ```ignore
/// let toasts = cx.new(|_| ToastStack::new());
/// toasts.update(cx, |t, cx| t.push(ToastKind::Success, "Projet enregistré", cx));
/// ```
#[derive(Default)]
pub struct ToastStack {
    /// File.
    queue: ToastQueue,
}

impl ToastStack {
    /// Pile vide.
    pub fn new() -> Self {
        Self::default()
    }

    /// Affiche une notification pour la durée par défaut de sa nature.
    pub fn push(
        &mut self,
        kind: ToastKind,
        message: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) -> ToastId {
        self.push_for(kind, message, Some(kind.default_duration()), cx)
    }

    /// Affiche une notification pour `duration` (`None` : jusqu'à fermeture).
    pub fn push_for(
        &mut self,
        kind: ToastKind,
        message: impl Into<SharedString>,
        duration: Option<Duration>,
        cx: &mut Context<Self>,
    ) -> ToastId {
        let id = self.queue.push(kind, message, duration, Instant::now());
        if let Some(duration) = duration {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(duration).await;
                let _ = this.update(cx, |this, cx| {
                    if this.queue.expire(Instant::now()) {
                        cx.notify();
                    }
                });
            })
            .detach();
        }
        cx.notify();
        id
    }

    /// Ferme une notification.
    pub fn dismiss(&mut self, id: ToastId, cx: &mut Context<Self>) {
        if self.queue.dismiss(id) {
            cx.notify();
        }
    }

    /// File (lecture seule).
    pub fn queue(&self) -> &ToastQueue {
        &self.queue
    }
}

impl Render for ToastStack {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.queue.is_empty() {
            return div().into_any_element();
        }
        let theme = cx.theme();
        let viewport = window.viewport_size();
        let mut column = div()
            .flex()
            .flex_col()
            .items_end()
            .gap(theme.space_md)
            .font_family(theme.font_family.clone())
            .text_size(theme.text_md);
        for toast in self.queue.toasts() {
            let (bg, border, icon_color) = toast.kind.colors(theme);
            let id = toast.id;
            column = column.child(
                div()
                    .id(("notification", id as usize))
                    .occlude()
                    .flex()
                    .items_start()
                    .gap(theme.space_md)
                    .w(px(340.))
                    .px(theme.space_lg)
                    .py(px(10.))
                    .rounded(theme.radius_lg)
                    .border_1()
                    .border_color(border)
                    .bg(bg)
                    .shadow(theme.shadow_popover())
                    .text_color(theme.text)
                    .child(icon(toast.kind.icon()).size(px(18.)).text_color(icon_color))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .line_height(gpui::relative(1.35))
                            .child(toast.message.clone()),
                    )
                    .child(
                        div()
                            .id(("fermer-notification", id as usize))
                            .flex_none()
                            .rounded(theme.radius_sm)
                            .cursor(CursorStyle::PointingHand)
                            .hover(|s| s.bg(theme.bg_ghost_hover))
                            .child(icon(Icon::Close).size(px(14.)).text_color(theme.text_muted))
                            .on_click(cx.listener(move |this, _, _, cx| this.dismiss(id, cx))),
                    ),
            );
        }
        let margin = theme.space_xl;
        deferred(
            anchored()
                .position(point(viewport.width - margin, viewport.height - margin))
                .anchor(gpui::Corner::BottomRight)
                .child(column),
        )
        .with_priority(TOAST_PRIORITY)
        .into_any_element()
    }
}

/// Raccourci : affiche une notification dans une pile détenue par une vue.
pub fn notify(
    stack: &gpui::Entity<ToastStack>,
    kind: ToastKind,
    message: impl Into<SharedString>,
    cx: &mut App,
) -> ToastId {
    let message = message.into();
    stack.update(cx, |stack, cx| stack.push(kind, message, cx))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn echeances() {
        let t0 = Instant::now();
        let mut q = ToastQueue::default();
        let info = q.push(
            ToastKind::Info,
            "Import terminé",
            Some(Duration::from_secs(4)),
            t0,
        );
        let err = q.push(
            ToastKind::Error,
            "Capture impossible",
            Some(Duration::from_secs(8)),
            t0,
        );
        let sticky = q.push(ToastKind::Warning, "Base figée", None, t0);
        assert!(!q.expire(t0 + Duration::from_secs(3)));
        assert!(
            q.expire(t0 + Duration::from_secs(4)),
            "échue à l'instant exact"
        );
        let ids: Vec<_> = q.toasts().iter().map(|t| t.id).collect();
        assert_eq!(ids, vec![err, sticky]);
        assert!(q.expire(t0 + Duration::from_secs(60)));
        assert_eq!(q.toasts().len(), 1, "la notification sans durée reste");
        assert!(q.dismiss(sticky));
        assert!(!q.dismiss(info), "déjà partie");
        assert!(q.is_empty());
    }

    #[test]
    fn plafond_d_affichage() {
        let t0 = Instant::now();
        let mut q = ToastQueue::default();
        for i in 0..(MAX_VISIBLE + 2) {
            q.push(ToastKind::Info, format!("n{i}"), None, t0);
        }
        assert_eq!(q.toasts().len(), MAX_VISIBLE);
        assert_eq!(
            q.toasts()[0].message.as_ref(),
            "n2",
            "les plus anciennes partent"
        );
    }

    #[test]
    fn durees_par_nature() {
        assert!(ToastKind::Error.default_duration() > ToastKind::Info.default_duration());
    }
}
