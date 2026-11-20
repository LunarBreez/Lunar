//! Dialogue des erreurs bloquantes d'un import de labels : MAC/IP
//! invalides, lignes mal formées ou
//! conflits entre lignes, détaillés ligne par ligne et plafonnés à dix par
//! catégorie.

use gpui::{
    ClickEvent, Context, Entity, FocusHandle, IntoElement, ParentElement, Render, Styled, Window,
    div, px,
};
use lunar_ui_logic::label_import::{EXPECTED_FORMAT, IssueReport, IssueSection, issue_report};

use crate::app::labels::session::LabelSession;
use crate::app::panels::Panel;
use crate::app::state::AppState;
use crate::assets::{Icon, icon};
use crate::theme::{ActiveTheme, Theme};
use crate::widgets::button::Button;
use crate::widgets::modal::Modal;

/// Ce dialogue.
const PANEL: Panel = Panel::Conflicts;

/// Dialogue des conflits d'import.
pub struct ConflictDialog {
    /// État de l'application.
    state: Entity<AppState>,
    /// Contenu, figé à l'ouverture (`None` : rien à montrer).
    report: Option<IssueReport>,
    /// Focus rendu à la fermeture : le panneau parent.
    return_focus: Option<FocusHandle>,
}

impl ConflictDialog {
    /// Dialogue sur l'erreur déposée dans la session.
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let session = LabelSession::global(cx);
        let report = session
            .read(cx)
            .issues()
            .map(|issues| issue_report(issues, false));
        Self {
            state,
            report,
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
}

/// Une section : titre puis entrées sur fond sombre, défilantes.
fn render_section(section: &IssueSection, theme: &Theme) -> impl IntoElement + use<> {
    div()
        .flex()
        .flex_col()
        .gap(theme.space_sm)
        .child(
            div()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(theme.text_heading)
                .child(section.title),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(theme.space_sm)
                .p(theme.space_md)
                .rounded(theme.radius_md)
                .bg(theme.bg_raised)
                .font_family(theme.font_mono.clone())
                .text_size(theme.text_sm)
                .children(section.entries.iter().map(|entry| {
                    let mut lines = entry.iter();
                    div()
                        .flex()
                        .flex_col()
                        .children(lines.next().map(|head| div().child(head.clone())))
                        .children(lines.map(|line| {
                            div()
                                .pl(px(24.))
                                .text_color(theme.text_muted)
                                .child(line.clone())
                        }))
                }))
                .children(section.truncated.clone().map(|line| {
                    div()
                        .pl(px(24.))
                        .italic()
                        .text_color(theme.text_muted)
                        .child(line)
                })),
        )
}

impl Render for ConflictDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let body = match &self.report {
            None => div()
                .text_color(theme.text_muted)
                .child("Aucune erreur à afficher."),
            Some(report) => div()
                .flex()
                .gap(theme.space_xl)
                .child(
                    div()
                        .flex_none()
                        .pt(theme.space_md)
                        .child(icon(Icon::Warning).size(px(64.)).text_color(theme.warning)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .gap(theme.space_lg)
                        .child(
                            div()
                                .px(theme.space_md)
                                .py(theme.space_sm)
                                .rounded(theme.radius_sm)
                                .border_1()
                                .border_color(theme.danger)
                                .font_weight(gpui::FontWeight::BOLD)
                                .child(report.total.clone()),
                        )
                        .children(
                            report
                                .sections
                                .iter()
                                .map(|section| render_section(section, &theme)),
                        )
                        .children(report.show_expected_format.then(|| {
                            div()
                                .text_size(theme.text_sm)
                                .text_color(theme.text_muted)
                                .child(EXPECTED_FORMAT)
                        })),
                ),
        };
        let title = self
            .report
            .as_ref()
            .map_or(PANEL.title(), |report| report.title);
        Modal::new("dialogue-conflits-labels", title)
            .width(PANEL.width())
            .on_close(cx.listener(|this, _: &(), window, cx| this.close(window, cx)))
            .on_confirm(cx.listener(|this, _: &(), window, cx| this.close(window, cx)))
            .child(body)
            .footer(
                Button::new("conflits-fermer", "Fermer").primary().on_click(
                    cx.listener(|this, _: &ClickEvent, window, cx| this.close(window, cx)),
                ),
            )
    }
}
