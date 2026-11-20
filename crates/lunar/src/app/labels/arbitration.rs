//! Dialogue d'arbitrage des conflits de labels :
//! pour chaque clé (MAC, IP) qui a reçu plusieurs labels, l'opérateur
//! choisit celui à garder ; tous les choix partent en un seul appel
//! `resolve_label_conflicts` (transactionnel).

use gpui::{
    ClickEvent, Context, Entity, FocusHandle, InteractiveElement, IntoElement, ParentElement,
    Render, StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px,
};
use log::{error, info};
use lunar_engine::commandes::import::labels::conflicts::LabelConflictReport;
use lunar_ui_logic::label_conflicts::{
    ConflictCandidate, arbitration_log, arbitration_subtitle, candidate_origin, candidate_text,
    conflict_candidates, conflict_key_text, default_choices, label_updates_between, resolutions,
};

use crate::app::labels::apply_label_updates;
use crate::app::labels::session::{LabelSession, LabelSessionEvent};
use crate::app::panels::Panel;
use crate::app::state::AppState;
use crate::theme::{ActiveTheme, Theme};
use crate::widgets::button::Button;
use crate::widgets::modal::Modal;

/// Ce dialogue.
const PANEL: Panel = Panel::Arbitration;

/// Dialogue d'arbitrage.
pub struct ArbitrationDialog {
    /// État de l'application.
    state: Entity<AppState>,
    /// Données partagées avec le panneau parent.
    session: Entity<LabelSession>,
    /// Conflits présentés, figés à l'ouverture.
    conflicts: Vec<LabelConflictReport>,
    /// Candidats de chaque conflit.
    candidates: Vec<Vec<ConflictCandidate>>,
    /// Label choisi par conflit (défaut : le label gardé).
    choices: Vec<String>,
    /// Application en cours (`resolving`).
    resolving: bool,
    /// Focus rendu à la fermeture : le panneau parent.
    return_focus: Option<FocusHandle>,
}

impl ArbitrationDialog {
    /// Dialogue sur les conflits déposés dans la session.
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let session = LabelSession::global(cx);
        let conflicts = session.read(cx).arbitration().to_vec();
        let candidates = conflicts.iter().map(conflict_candidates).collect();
        let choices = default_choices(&conflicts);
        Self {
            state,
            session,
            conflicts,
            candidates,
            choices,
            resolving: false,
            return_focus: window.focused(cx),
        }
    }

    /// Ferme le dialogue et rend le focus au panneau parent.
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(focus) = &self.return_focus {
            window.focus(focus);
        }
        self.state
            .update(cx, |state, cx| state.close_panel(PANEL, cx));
    }

    /// Fermeture (croix, Fermer, Échap), refusée pendant l'application :
    /// l'opérateur attend la réponse de `resolve_label_conflicts`.
    fn request_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.resolving {
            self.close(window, cx);
        }
    }

    /// Choisit `label` pour le conflit `index`.
    fn choose(&mut self, index: usize, label: String, cx: &mut Context<Self>) {
        if self.resolving {
            return;
        }
        if let Some(choice) = self.choices.get_mut(index) {
            *choice = label;
            cx.notify();
        }
    }

    /// « Appliquer » : tous les arbitrages en un appel, puis les labels
    /// changés vont au graphe et le parent relit ses lignes.
    fn apply_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.resolving {
            return;
        }
        let window = window.window_handle();
        self.resolving = true;
        cx.notify();
        let sent = resolutions(&self.conflicts, &self.choices);
        let count = self.conflicts.len();
        let bridge = self.state.read(cx).bridge().clone();
        bridge
            .run(
                cx,
                move |lunar| {
                    let before = lunar.get_graph_snapshot();
                    let result = lunar.resolve_label_conflicts(sent);
                    let updates = match (&result, before, lunar.get_graph_snapshot()) {
                        (Ok(_), Ok(before), Ok(after)) => label_updates_between(&before, &after),
                        _ => Vec::new(),
                    };
                    result.map(|_| updates)
                },
                move |this, result, cx| {
                    this.resolving = false;
                    match result {
                        Ok(updates) => {
                            info!("{}", arbitration_log(count));
                            apply_label_updates(&this.state, updates, cx);
                            this.session.update(cx, |session, cx| {
                                session.set_arbitration(Vec::new());
                                cx.emit(LabelSessionEvent::Resolved);
                            });
                            if let Some(focus) = this.return_focus.clone() {
                                // Fenêtre fermée entre-temps : plus de focus à rendre.
                                let _ = window.update(cx, |_, window, _| window.focus(&focus));
                            }
                            this.state
                                .update(cx, |state, cx| state.close_panel(PANEL, cx));
                        }
                        Err(err) => {
                            error!("Erreur arbitrage: {err}");
                            this.state
                                .update(cx, |state, cx| state.show_error(&err, cx));
                        }
                    }
                    cx.notify();
                },
            )
            .detach();
    }

    /// Carte d'un conflit : sa clé et ses candidats.
    fn render_card(
        &self,
        index: usize,
        conflict: &LabelConflictReport,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let chosen = self.choices.get(index).cloned().unwrap_or_default();
        let candidates = self.candidates.get(index).cloned().unwrap_or_default();
        div()
            .flex()
            .flex_col()
            .gap(theme.space_xs)
            .p(theme.space_md)
            .rounded(theme.radius_md)
            .border_1()
            .border_color(theme.border_card)
            .bg(theme.bg_raised)
            .child(
                div()
                    .mb(theme.space_xs)
                    .font_family(theme.font_mono.clone())
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(conflict_key_text(conflict)),
            )
            .children(
                candidates
                    .into_iter()
                    .enumerate()
                    .map(|(position, candidate)| {
                        let selected = candidate.label == chosen;
                        let label = candidate.label.clone();
                        let hover = theme.bg_hover;
                        let dot = div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .size(px(14.))
                            .rounded_full()
                            .border_1()
                            .border_color(if selected {
                                theme.focus_ring
                            } else {
                                theme.border_strong
                            })
                            .when(selected, |d| {
                                d.child(div().size(px(6.)).rounded_full().bg(theme.focus_ring))
                            });
                        div()
                            .id(gpui::ElementId::Name(
                                format!("arbitrage-{index}-{position}").into(),
                            ))
                            .flex()
                            .items_center()
                            .gap(theme.space_md)
                            .px(theme.space_sm)
                            .py(theme.space_xs)
                            .rounded(theme.radius_sm)
                            .cursor_pointer()
                            .when(selected, |d| d.bg(theme.accent_soft))
                            .when(!selected, |d| d.hover(move |s| s.bg(hover)))
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.choose(index, label.clone(), cx)
                            }))
                            .child(dot)
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(candidate_text(&candidate).to_owned()),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .text_size(theme.text_sm)
                                    .text_color(theme.text_muted)
                                    .child(candidate_origin(&candidate)),
                            )
                    }),
            )
    }
}

impl Render for ArbitrationDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let cards: Vec<_> = self
            .conflicts
            .clone()
            .iter()
            .enumerate()
            .map(|(index, conflict)| self.render_card(index, conflict, &theme, cx))
            .collect();
        let resolving = self.resolving;
        Modal::new("dialogue-arbitrage-labels", PANEL.title())
            .width(PANEL.width())
            .on_close(cx.listener(|this, _: &(), window, cx| this.request_close(window, cx)))
            .child(
                div()
                    .text_color(theme.text_muted)
                    .child(arbitration_subtitle(self.conflicts.len())),
            )
            .children(cards)
            .footer(
                Button::new("arbitrage-fermer", "Fermer")
                    .disabled(resolving)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.request_close(window, cx)
                    })),
            )
            .footer(
                Button::new(
                    "arbitrage-appliquer",
                    if resolving {
                        "Application…"
                    } else {
                        "Appliquer"
                    },
                )
                .primary()
                .disabled(resolving)
                .on_click(
                    cx.listener(|this, _: &ClickEvent, window, cx| this.apply_all(window, cx)),
                ),
            )
    }
}
