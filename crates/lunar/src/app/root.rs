//! Fenêtre principale : barre
//! de menus, puis la vue d'analyse (barre d'outils, graphe et panneau
//! latéral, journal de trames, barre de statut) ou la vue d'investigation,
//! les notifications et les dialogues.
//!
//! La racine porte le contexte clavier de la vue
//! ([`crate::app::actions::key_context`]) et les écouteurs de toutes les
//! actions : la barre de menus, la barre d'outils et les raccourcis
//! déclenchent la même action, traitée par
//! [`AppState::run_action`].

use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};

use gpui::{
    AnyElement, AnyView, App, AppContext, ClickEvent, Context, CursorStyle, Entity, ExternalPaths,
    FocusHandle, Focusable, InteractiveElement, IntoElement, ParentElement, Render, Styled,
    Subscription, Window, div, img, prelude::FluentBuilder, px,
};
use lunar_ui_logic::menu::{
    MenuActionId, MenuContext, MenuRowState, MenuView, menu_bar, menu_enabled_state,
    shortcuts_help_text,
};

use crate::app::about::{ABOUT_TITLE, about_message};
use crate::app::actions::{
    ExportGraphPng, MenuActionHandler, ShowMatrixLabels, ToggleGravity, action_for, key_context,
    on_menu_actions,
};
use crate::app::investigation::InvestigationView;
use crate::app::packet_log::render_packet_log;
use crate::app::panels::import_capture::forward_window_drop;
use crate::app::panels::{Panel, Presentation, build_panel_view};
use crate::app::state::{
    AppEvent, AppModal, AppState, QUIT_QUESTION, RECOVERY_QUESTION, RESET_QUESTION,
};
use crate::app::status_bar::{StatusBarData, render_status_bar};
use crate::app::toolbar::render_toolbar;
use crate::assets::{Icon, LOGO_PNG};
use crate::theme::{ActiveTheme, Theme};
use crate::widgets::button::{Button, ButtonSize};
use crate::widgets::menu_bar::{MenuBar, MenuEntry, MenuSection};
use crate::widgets::modal::{Confirm, Modal};
use crate::widgets::root_actions;

/// Rendus de la fenêtre principale, lus par le réveil X11
/// ([`crate::platform::install_x11_wakeup`]).
pub static RENDERS: AtomicU32 = AtomicU32::new(0);

/// Rappel exécuté juste après la première image (smoke test).
pub type FirstFrameCallback = Box<dyn FnOnce(&mut Window, &mut App)>;

/// Sections de la barre de menus pour l'état `context`.
pub fn menu_sections(context: &MenuContext) -> Vec<MenuSection> {
    menu_bar(context)
        .into_iter()
        .map(|section| {
            let items = section
                .rows
                .into_iter()
                .map(|row| match row {
                    MenuRowState::Separator => MenuEntry::Separator,
                    MenuRowState::Item(item) => {
                        let entry =
                            MenuEntry::item(item.id.as_str(), item.label).enabled(item.enabled);
                        match item.shortcut {
                            Some(shortcut) => entry.accelerator(shortcut.display()),
                            None => entry,
                        }
                    }
                })
                .collect();
            MenuSection::new(section.title, items)
        })
        .collect()
}

/// Vue racine de la fenêtre.
pub struct LunarWindow {
    /// Focus racine : sans lui, les raccourcis n'atteignent pas la racine.
    focus_handle: FocusHandle,
    /// État de l'application.
    state: Entity<AppState>,
    /// Barre de menus.
    menu_bar: Entity<MenuBar>,
    /// Vues des panneaux ouverts.
    panel_views: Vec<(Panel, AnyView)>,
    /// Vue d'investigation, tant qu'elle est affichée.
    investigation: Option<Entity<InvestigationView>>,
    /// Rappel de première image, posé au premier rendu.
    first_frame: Option<FirstFrameCallback>,
    /// Abonnements à l'état.
    _subscriptions: Vec<Subscription>,
}

impl Focusable for LunarWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl LunarWindow {
    /// Fenêtre sur `state`.
    pub fn new(
        state: Entity<AppState>,
        first_frame: Option<FirstFrameCallback>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let context = state.read(cx).menu_context();
        let menu_bar = cx.new(|cx| {
            MenuBar::new(menu_sections(&context), cx).on_activate(|id, window, cx| {
                if let Some(action) = MenuActionId::parse(id) {
                    window.dispatch_action(action_for(action), cx);
                }
            })
        });
        let subscriptions = vec![
            cx.observe(&state, |_, _, cx| cx.notify()),
            cx.subscribe_in(&state, window, Self::on_app_event),
        ];
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle);
        Self {
            focus_handle,
            state,
            menu_bar,
            panel_views: Vec::new(),
            investigation: None,
            first_frame,
            _subscriptions: subscriptions,
        }
    }

    /// Réagit aux changements de l'état.
    fn on_app_event(
        &mut self,
        state: &Entity<AppState>,
        event: &AppEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            AppEvent::MenuContextChanged(context) => self.sync_menu(context, cx),
            AppEvent::PanelsChanged => self.sync_panels(window, cx),
            AppEvent::ViewChanged(view) => {
                // La vue d'investigation naît à l'entrée et meurt à la sortie :
                // chaque ouverture relit le relevé.
                self.investigation = match view {
                    MenuView::Investigation => {
                        let state = state.clone();
                        Some(cx.new(|cx| InvestigationView::new(state, window, cx)))
                    }
                    MenuView::Analyse => None,
                };
                if state.read(cx).modal().is_none() {
                    window.focus(&self.focus_handle);
                }
            }
            AppEvent::ModalsChanged => {
                // Dialogue fermé ou vue changée : le focus revient à la racine
                // (Tab, Alt et F10 repartent de là).
                if state.read(cx).modal().is_none() {
                    window.focus(&self.focus_handle);
                }
            }
            AppEvent::Reset | AppEvent::InvestigationAction(_) => {}
        }
    }

    /// Recalcule l'activation des entrées de menu.
    fn sync_menu(&mut self, context: &MenuContext, cx: &mut Context<Self>) {
        let enabled = menu_enabled_state(context);
        self.menu_bar.update(cx, |bar, cx| {
            for id in MenuActionId::all() {
                bar.set_enabled(id.as_str(), enabled[id], cx);
            }
        });
    }

    /// Crée les vues des panneaux ouverts, détruit celles des fermés.
    fn sync_panels(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let open: Vec<Panel> = self.state.read(cx).panels().open_panels().collect();
        let before = self.panel_views.len();
        self.panel_views.retain(|(panel, _)| open.contains(panel));
        // Panneau fermé : son focus disparaît avec lui, la racine le reprend
        // (sinon raccourcis et boutons de la barre d'outils restent sans
        // effet jusqu'au prochain clic).
        if self.panel_views.len() < before && self.state.read(cx).modal().is_none() {
            window.focus(&self.focus_handle);
        }
        for panel in open {
            if !self
                .panel_views
                .iter()
                .any(|(existing, _)| *existing == panel)
            {
                let view = build_panel_view(panel, &self.state, window, cx);
                self.panel_views.push((panel, view));
            }
        }
        cx.notify();
    }

    /// Vue de panneau ouverte présentée de la façon `presentation`.
    fn panel_view(&self, presentation: Presentation) -> Option<AnyView> {
        self.panel_views
            .iter()
            .find(|(panel, _)| panel.presentation() == presentation)
            .map(|(_, view)| view.clone())
    }

    /// Vue d'analyse.
    fn render_analyse(&self, state: &AppState, theme: &Theme, cx: &App) -> AnyElement {
        let graph_area = if state.show_graph() {
            div()
                .relative()
                .size_full()
                .child(state.graph().view())
                .child(render_graph_controls(
                    state.graph().is_force_running(cx),
                    theme,
                ))
                .into_any_element()
        } else {
            div().size_full().bg(theme.bg_content).into_any_element()
        };
        let clear_filter = {
            let state = self.state.clone();
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                state.update(cx, |state, cx| state.clear_filter(cx));
            }
        };
        let status = render_status_bar(
            StatusBarData {
                capture: state.capture(),
                elapsed: state.timer().elapsed(std::time::Instant::now()),
                cpu_usage: state.cpu_usage(),
                channel: state.channel(),
            },
            theme,
            clear_filter,
        );
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(render_toolbar(&state.toolbar_state(), theme))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(div().flex_1().min_w_0().h_full().child(graph_area))
                    .children(self.panel_view(Presentation::Side)),
            )
            .child(render_packet_log(state.packet_log(), theme))
            .child(status)
            .into_any_element()
    }

    /// Dialogue affiché.
    fn render_modal(&self, seq: u64, modal: &AppModal, theme: &Theme) -> AnyElement {
        let id = gpui::ElementId::Name(format!("dialogue-{seq}").into());
        let state = self.state.clone();
        let close = move |_: &(), _: &mut Window, cx: &mut App| {
            state.update(cx, |state, cx| state.close_modal(cx));
        };
        let state = self.state.clone();
        let close_button = move |label: &'static str| {
            let state = state.clone();
            Button::new("dialogue-fermer", label).primary().on_click(
                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                    state.update(cx, |state, cx| state.close_modal(cx));
                },
            )
        };
        let answer = |answer: fn(&mut AppState, bool, &mut Context<AppState>), value: bool| {
            let state = self.state.clone();
            move |_: &(), _: &mut Window, cx: &mut App| {
                state.update(cx, |state, cx| answer(state, value, cx));
            }
        };
        match modal {
            AppModal::About => Modal::new(id, ABOUT_TITLE)
                .width(px(620.))
                .on_close(close.clone())
                .on_confirm(close)
                .child(render_about(theme))
                .footer(close_button("Fermer"))
                .into_any_element(),
            AppModal::Shortcuts => Modal::new(id, "Raccourcis clavier — LUNAR")
                .width(px(560.))
                .on_close(close.clone())
                .on_confirm(close)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .font_family(theme.font_mono.clone())
                        .text_size(theme.text_sm)
                        .children(shortcuts_help_text().lines().map(|line| {
                            // Les lignes de raccourci restent entières ; les
                            // phrases se replient dans la largeur de la modale.
                            div()
                                .min_h(px(16.))
                                .when(line.starts_with(' '), |el| el.whitespace_nowrap())
                                .child(line.to_owned())
                        })),
                )
                .footer(close_button("Fermer"))
                .into_any_element(),
            AppModal::Message { title, message } => Modal::new(id, title.clone())
                .width(px(520.))
                .on_close(close.clone())
                .on_confirm(close)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .children(message.lines().map(|line| div().child(line.to_owned()))),
                )
                .footer(close_button("OK"))
                .into_any_element(),
            AppModal::ConfirmReset => Confirm::new(id, "LUNAR", RESET_QUESTION)
                .confirm_label("Réinitialiser")
                .danger()
                .on_confirm(answer(AppState::answer_reset, true))
                .on_cancel(answer(AppState::answer_reset, false))
                .into_any_element(),
            AppModal::ConfirmQuit => Confirm::new(id, "LUNAR", QUIT_QUESTION)
                .confirm_label("Quitter")
                .danger()
                .on_confirm(answer(AppState::answer_exit, true))
                .on_cancel(answer(AppState::answer_exit, false))
                .into_any_element(),
            AppModal::Recovery { .. } => Confirm::new(id, "LUNAR", RECOVERY_QUESTION)
                .confirm_label("Récupérer")
                .cancel_label("Ignorer")
                .on_confirm(answer(AppState::answer_recovery, true))
                .on_cancel(answer(AppState::answer_recovery, false))
                .into_any_element(),
        }
    }
}

/// Commandes posées sur le graphe, en haut à gauche : export PNG et
/// gravité ; « Afficher les labels » est dans la barre du bas de la vue du
/// graphe.
fn render_graph_controls(force_running: bool, theme: &Theme) -> impl IntoElement + use<> {
    let dispatch = |action: Box<dyn gpui::Action>| {
        move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
            window.dispatch_action(action.boxed_clone(), cx)
        }
    };
    div()
        .absolute()
        .top(theme.space_md)
        .left(theme.space_md)
        .flex()
        .gap(theme.space_sm)
        .child(
            Button::new("graphe-export", "Export PNG")
                .icon(Icon::Download)
                .size(ButtonSize::Small)
                .tooltip("Exporter le graphe en PNG")
                .on_click(dispatch(Box::new(ExportGraphPng))),
        )
        .child(
            Button::new(
                "graphe-gravite",
                if force_running {
                    "Gravité: ON"
                } else {
                    "Gravité: OFF"
                },
            )
            .size(ButtonSize::Small)
            .toggled(force_running)
            .tooltip(if force_running {
                "Désactiver la gravité (elle se coupe seule au bout de 20 minutes)"
            } else {
                "Activer la gravité pour 20 minutes"
            })
            .on_click(dispatch(Box::new(ToggleGravity))),
        )
}

/// Corps du dialogue « À propos » : logo, puis un paragraphe par section,
/// la première ligne d'une section en titre.
fn render_about(theme: &Theme) -> impl IntoElement + use<> {
    let text = about_message();
    let mut paragraphs = text.split("\n\n");
    let headline = paragraphs.next().unwrap_or_default().to_owned();
    div()
        .flex()
        .flex_col()
        .gap(theme.space_lg)
        .child(
            div()
                .flex()
                .items_center()
                .gap(theme.space_lg)
                .child(img(LOGO_PNG).size(px(48.)))
                .child(
                    div()
                        .text_size(theme.text_lg)
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(theme.text_heading)
                        .child(headline),
                ),
        )
        .children(paragraphs.map(|paragraph| {
            let mut lines = paragraph.lines();
            let title = lines.next().unwrap_or_default().to_owned();
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(theme.text_highlight)
                        .child(title),
                )
                .children(lines.map(|line| div().child(line.to_owned())))
        }))
}

impl Render for LunarWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        RENDERS.fetch_add(1, Ordering::Relaxed);
        if let Some(callback) = self.first_frame.take() {
            window.on_next_frame(callback);
        }
        // Panneau ou dialogue fermé avec le focus : sans élément focalisé, les
        // actions partent du nœud racine, sans gestionnaire ; la racine reprend
        // le focus (barre d'outils, raccourcis).
        if window.focused(cx).is_none() {
            window.focus(&self.focus_handle);
        }
        let theme = cx.theme().clone();
        let state = self.state.read(cx);
        let view = state.view();
        let importing = state.capture().is_importing;
        let toasts = state.toasts().clone();
        let content = match view {
            MenuView::Analyse => self.render_analyse(state, &theme, cx),
            MenuView::Investigation => match &self.investigation {
                Some(view) => view.clone().into_any_element(),
                None => div().size_full().bg(theme.bg_content).into_any_element(),
            },
        };
        let modal = state
            .modal()
            .map(|(seq, modal)| self.render_modal(seq, modal, &theme));
        let has_modal = modal.is_some();
        let dialog_panel = match view {
            MenuView::Analyse => self.panel_view(Presentation::Modal),
            MenuView::Investigation => None,
        };

        let root = div()
            .id("lunar")
            .track_focus(&self.focus_handle)
            .key_context(key_context(view))
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.bg_app)
            .text_color(theme.text)
            .text_size(theme.text_md)
            .font_family(theme.font_family.clone())
            .child(self.menu_bar.clone())
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .child(content)
                    // Verrou d'import : la souris n'atteint plus la vue.
                    .when(importing, |d| {
                        d.child(
                            div()
                                .id("verrou-import")
                                .absolute()
                                .inset_0()
                                .occlude()
                                .cursor(CursorStyle::OperationNotAllowed),
                        )
                    }),
            )
            .children(dialog_panel)
            .child(toasts)
            .children(modal);

        let handler: MenuActionHandler = {
            let state = self.state.clone();
            Rc::new(move |id, _window, cx| state.update(cx, |state, cx| state.run_action(id, cx)))
        };
        let root = on_menu_actions(root, handler)
            // Dépôt de fichiers hors du panneau d'import : routé vers lui.
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                forward_window_drop(&this.panel_views, paths, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleGravity, _, cx| {
                this.state.update(cx, |state, cx| state.toggle_gravity(cx))
            }))
            .on_action(cx.listener(|this, _: &ExportGraphPng, _, cx| {
                this.state
                    .update(cx, |state, cx| state.export_graph_png(cx))
            }))
            .on_action(cx.listener(|this, _: &ShowMatrixLabels, _, cx| {
                this.state
                    .update(cx, |state, cx| state.toggle_matrix_labels(cx))
            }));
        root_actions(root, (!has_modal).then_some(&self.menu_bar))
    }
}
