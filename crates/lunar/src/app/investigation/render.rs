//! Rendu de la vue d'investigation : barre d'en-tête et filtre, messages,
//! indicateurs, onglets, résumé, tableaux et glossaire.

use gpui::{
    AnyElement, App, ClickEvent, Context, CursorStyle, ElementId, FontWeight, Hsla, MouseButton,
    Render, SharedString, Window, anchored, deferred, div, point, prelude::*, px, relative, rgb,
};
use lunar_ui_logic::investigation::glossary::GlossaryEntry;
use lunar_ui_logic::investigation::view::{
    BodyState, EMPTY_SURVEY_HINT, EMPTY_SURVEY_TEXT, LOADING_TEXT, SummaryCard, SummaryLine, TABS,
    Tab, WIKI_HINT, summary_cards, summary_flags, wiki_all_label, wiki_present_label,
};
use lunar_ui_logic::menu::MenuActionId;

use crate::app::actions::action_for;
use crate::app::investigation::InvestigationView;
use crate::app::investigation::support::css_hex;
use crate::assets::{Icon, icon};
use crate::theme::{ActiveTheme, Theme};
use crate::widgets::button::Button;
use crate::widgets::checkbox::Checkbox;
use crate::widgets::ellipsis;
use crate::widgets::kpi::Kpi;
use crate::widgets::select::POPOVER_PRIORITY;
use crate::widgets::tabs::{Tab as TabItem, Tabs};
use crate::widgets::tooltip::tooltip;

/// Texte de la vue (`.investigation { color }`).
fn text_color() -> Hsla {
    rgb(0xcfe3ff).into()
}

/// Message d'erreur (`.inv-error`).
fn error_color() -> Hsla {
    rgb(0xff8a80).into()
}

/// Compte rendu d'export (`.inv-notice`).
fn notice_color() -> Hsla {
    rgb(0x9ccc65).into()
}

/// Fond des barres du résumé (`.sum-bar`).
fn track_color() -> Hsla {
    rgb(0x05030f).into()
}

/// Bordure des boutons et du filtre (`.inv-btn`, `.filter-pop`).
fn control_border() -> Hsla {
    rgb(0x333b52).into()
}

/// Survol d'une ligne du filtre (`.filter-item:hover`).
fn item_hover() -> Hsla {
    rgb(0x171531).into()
}

/// Avertissement (IP dupliquées, endpoints à étiqueter).
fn warn_color() -> Hsla {
    rgb(0xffb74d).into()
}

/// Marge latérale de la vue.
const GUTTER: f32 = 14.;

impl Render for InvestigationView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let state = &self.model.state;
        let message = if !state.error.is_empty() {
            Some((state.error.clone(), error_color()))
        } else if !state.notice.is_empty() {
            Some((state.notice.clone(), notice_color()))
        } else {
            None
        };
        let body = self.render_body(&theme, cx);
        div()
            .id("investigation")
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.bg_content)
            .text_color(text_color())
            .text_size(theme.text_md)
            .child(self.render_header(&theme, window, cx))
            .when_some(message, |d, (text, color)| {
                d.child(
                    div()
                        .flex_none()
                        .mx(px(GUTTER))
                        .mt(px(8.))
                        .text_color(color)
                        .child(text),
                )
            })
            .child(self.render_kpis(&theme))
            .child(self.render_tabs(cx))
            .child(div().flex().flex_col().flex_1().min_h_0().child(body))
    }
}

impl InvestigationView {
    /// Barre d'en-tête (`.inv-bar`) : retour, titre, rafraîchir, exports,
    /// filtre et recherche.
    fn render_header(
        &self,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let state = &self.model.state;
        let busy = state.loading || state.exporting;
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(12.))
            .h(px(48.))
            .px(px(GUTTER))
            .bg(theme.bg_bar)
            .border_b_1()
            .border_color(theme.border_divider)
            .child(
                Button::new("investigation-retour", "Retour")
                    .icon(Icon::ArrowLeft)
                    .tooltip("Retour à l'analyse")
                    .on_click(|_: &ClickEvent, window: &mut Window, cx: &mut App| {
                        window.dispatch_action(action_for(MenuActionId::BackToAnalyse), cx)
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(6.))
                    .text_size(theme.text_lg)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.text_highlight)
                    .child(
                        icon(Icon::Microscope)
                            .size(px(18.))
                            .text_color(theme.text_highlight),
                    )
                    .child("Investigation"),
            )
            .child(
                Button::new("investigation-rafraichir", "Rafraîchir")
                    .icon(Icon::Refresh)
                    .disabled(state.loading)
                    .tooltip("Rafraîchir")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.reload(cx))),
            )
            .child(
                Button::new("investigation-exporter", "Exporter")
                    .icon(Icon::FileExport)
                    .disabled(busy)
                    .tooltip("Exporter le relevé complet : page HTML lisible sans LUNAR et matrice CSV, avec son empreinte (Ctrl+S)")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.export_synthesis(cx))),
            )
            .child(
                Button::new("investigation-a-completer", "À compléter")
                    .icon(Icon::Tags)
                    .disabled(busy)
                    .tooltip("Exporter les équipements sans label, par paquets décroissants, dans un fichier de labels à compléter puis réimporter (les labels en place y sont repris)")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.export_labels_to_complete(cx)
                    })),
            )
            .child(self.render_filter(theme, window, cx))
            // Un espaceur plutôt que `margin-left: auto` : taffy 0.9 perd les
            // `gap` d'une ligne dès qu'une marge automatique y figure.
            .child(div().flex_1())
            .child(div().w(px(300.)).flex_none().child(self.search.clone()))
    }

    /// Bouton « Filtre » et son volet (`.filter-wrap`).
    fn render_filter(
        &self,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let state = &self.model.state;
        let derived = &self.derived;
        let label = derived
            .filter_label
            .trim_start_matches('⚙')
            .trim_start()
            .to_string();
        let button = Button::new("investigation-filtre", label)
            .icon(Icon::Filter)
            .toggled(!state.all_protocols_visible())
            .disabled(derived.protocol_options.is_empty())
            .tooltip("Filtrer par protocole")
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.update_state(|state| state.filter_open = !state.filter_open, cx)
            }));
        let mut root = div().relative().flex_none().child(button);
        if !state.filter_open {
            return root;
        }
        let viewport = window.viewport_size();
        // Clic hors du volet : il se ferme (`.filter-backdrop`).
        let catcher = div()
            .id("investigation-filtre-dehors")
            .occlude()
            .w(viewport.width)
            .h(viewport.height)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.update_state(|state| state.filter_open = false, cx)
                }),
            );
        root = root
            .child(
                deferred(anchored().position(point(px(0.), px(0.))).child(catcher))
                    .with_priority(POPOVER_PRIORITY),
            )
            .child(
                div()
                    .absolute()
                    .top(theme.control_height + px(6.))
                    .left_0()
                    .child(
                        deferred(
                            anchored()
                                .snap_to_window_with_margin(px(8.))
                                .child(self.render_filter_popover(theme, cx)),
                        )
                        .with_priority(POPOVER_PRIORITY + 1),
                    ),
            );
        root
    }

    /// Volet du filtre (`.filter-pop`) : protocoles, endpoints sans label,
    /// colonnes du tableau courant.
    fn render_filter_popover(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let state = &self.model.state;
        let derived = &self.derived;
        let mut pop = div()
            .id("investigation-filtre-volet")
            .occlude()
            .flex()
            .flex_col()
            .min_w(px(220.))
            .max_h(px(360.))
            .overflow_y_scroll()
            .p(px(8.))
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(control_border())
            .bg(theme.kpi_bg)
            .shadow(theme.shadow_popover())
            .text_size(theme.text_md)
            .text_color(text_color())
            .font_family(theme.font_family.clone());

        let all = cx.listener(|this, _: &ClickEvent, _, cx| {
            let options = this.derived.protocol_options.clone();
            this.update_state(|state| state.set_all_protocols(true, &options), cx)
        });
        let none = cx.listener(|this, _: &ClickEvent, _, cx| {
            let options = this.derived.protocol_options.clone();
            this.update_state(|state| state.set_all_protocols(false, &options), cx)
        });
        pop = pop.child(
            filter_head(theme, "Protocoles")
                .child(div().flex_1())
                .child(filter_link("filtre-tout", "Tout", theme, all))
                .child(filter_link("filtre-aucun", "Aucun", theme, none)),
        );
        for (ix, option) in derived.protocol_options.iter().enumerate() {
            let protocol = option.protocol.clone();
            let on_change = cx.listener(move |this, _: &bool, _, cx| {
                let protocol = protocol.clone();
                this.update_state(|state| state.toggle_protocol(&protocol), cx)
            });
            pop = pop.child(filter_item(
                Checkbox::new(
                    ElementId::named_usize("filtre-protocole", ix),
                    option.protocol.clone(),
                    state.is_protocol_visible(&option.protocol),
                )
                .on_change(on_change),
                Some(option.flows.to_string()),
            ));
        }

        if state.tab == Tab::Endpoints {
            pop = pop
                .child(filter_head(theme, "Endpoints"))
                .child(filter_item(
                    Checkbox::new(
                        "filtre-sans-label",
                        "Sans label uniquement",
                        state.only_unlabeled,
                    )
                    .on_change(cx.listener(|this, checked: &bool, _, cx| {
                        let checked = *checked;
                        this.update_state(|state| state.only_unlabeled = checked, cx)
                    })),
                    Some(derived.summary.unlabeled.to_string()),
                ));
        }

        let columns = state.column_options();
        if !columns.is_empty() {
            pop = pop.child(filter_head(theme, "Colonnes"));
            for (ix, column) in columns.iter().enumerate() {
                let key = column.key;
                pop = pop.child(filter_item(
                    Checkbox::new(
                        ElementId::named_usize("filtre-colonne", ix),
                        column.label,
                        state.is_column_visible(key),
                    )
                    .on_change(cx.listener(move |this, _: &bool, _, cx| {
                        this.update_state(|state| state.toggle_column(key), cx)
                    })),
                    None,
                ));
            }
        }
        pop.into_any_element()
    }

    /// Bandeau d'indicateurs (`.kpis`).
    fn render_kpis(&self, theme: &Theme) -> impl IntoElement + use<> {
        let mut row = div()
            .flex()
            .flex_none()
            .flex_wrap()
            .gap(px(10.))
            .px(px(GUTTER))
            .py(theme.space_lg);
        for (ix, kpi) in self.derived.kpis.iter().enumerate() {
            let tile = Kpi::new(kpi.label, kpi.value.clone()).warn(kpi.warn);
            row = row.child(match kpi.title.clone() {
                Some(title) => div()
                    .id(ElementId::named_usize("kpi", ix))
                    .tooltip(tooltip(title))
                    .child(tile)
                    .into_any_element(),
                None => tile.into_any_element(),
            });
        }
        row
    }

    /// Barre d'onglets (`.tabs`).
    fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let selected = TABS
            .iter()
            .position(|tab| *tab == self.model.state.tab)
            .unwrap_or(0);
        let labels: Vec<TabItem> = self
            .derived
            .tabs
            .iter()
            .map(|(_, label)| TabItem::from(label.clone()))
            .collect();
        div().flex_none().child(
            Tabs::new("investigation-onglets", labels, selected).on_change(cx.listener(
                |this, ix: &usize, _, cx| {
                    if let Some(tab) = TABS.get(*ix).copied() {
                        this.update_state(|state| state.tab = tab, cx)
                    }
                },
            )),
        )
    }

    /// Corps de la vue sous les onglets.
    fn render_body(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        match self.derived.body {
            BodyState::Loading => empty_text(LOADING_TEXT).into_any_element(),
            BodyState::EmptySurvey => {
                // « (Le Glossaire reste consultable hors ligne.) », le mot
                // « Glossaire » en gras.
                let hint = match EMPTY_SURVEY_HINT.split_once("Glossaire") {
                    Some((before, after)) => div()
                        .flex()
                        .child(before.to_string())
                        .child(div().font_weight(FontWeight::BOLD).child("Glossaire"))
                        .child(after.to_string()),
                    None => div().child(EMPTY_SURVEY_HINT),
                };
                div()
                    .flex()
                    .flex_col()
                    .px(px(GUTTER))
                    .py(px(24.))
                    .opacity(0.7)
                    .child(EMPTY_SURVEY_TEXT)
                    .child(hint)
                    .into_any_element()
            }
            BodyState::Tab(Tab::Summary) => self.render_summary(theme, cx),
            BodyState::Tab(Tab::Flows) => table_area(self.flows.clone()),
            BodyState::Tab(Tab::Endpoints) => table_area(self.endpoints.clone()),
            BodyState::Tab(Tab::Deduction) => table_area(self.deductions.clone()),
            BodyState::Tab(Tab::Wiki) => self.render_wiki(theme, cx),
        }
    }

    /// Onglet Résumé (`.summary`) : cartes à barres proportionnelles, puis
    /// les alertes.
    fn render_summary(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let flags = summary_flags(&self.derived);
        let mut column = scroll_column("investigation-resume");
        for (ix, card) in summary_cards(&self.derived).into_iter().enumerate() {
            column = column.child(summary_card(ix, card, theme));
        }
        let unlabeled = flags.unlabeled;
        let flag = |count: usize, text: &'static str| {
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .opacity(0.85)
                .child(
                    div()
                        .text_size(theme.text_lg)
                        .font_weight(FontWeight::BOLD)
                        .text_color(if count > 0 {
                            warn_color()
                        } else {
                            theme.text_highlight
                        })
                        .child(count.to_string()),
                )
                .child(text)
        };
        column = column.child(
            card_box(theme)
                .flex()
                .flex_wrap()
                .gap(px(24.))
                .child(flag(flags.duplicate_ips, "IP dupliquée(s)"))
                .child(
                    flag(unlabeled, "endpoint(s) à étiqueter").when(unlabeled > 0, |d| {
                        d.child(filter_link(
                            "resume-etiqueter",
                            "étiqueter →",
                            theme,
                            cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.update_state(|state| state.show_unlabeled(), cx)
                            }),
                        ))
                    }),
                ),
        );
        column.into_any_element()
    }

    /// Onglet Glossaire : bascule « présents » / « tout », puis les fiches.
    fn render_wiki(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let show_all = self.model.state.wiki_show_all;
        let toolbar = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(10.))
            .mb(px(4.))
            .child(
                Button::new(
                    "glossaire-presents",
                    wiki_present_label(self.derived.present_glossary.len()),
                )
                .toggled(!show_all)
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.update_state(|state| state.wiki_show_all = false, cx)
                })),
            )
            .child(
                Button::new("glossaire-tout", wiki_all_label())
                    .toggled(show_all)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.update_state(|state| state.wiki_show_all = true, cx)
                    })),
            )
            .child(
                div()
                    .text_size(theme.text_sm)
                    .opacity(0.55)
                    .child(WIKI_HINT),
            );
        let mut column = scroll_column("investigation-glossaire").child(toolbar);
        if let Some(text) = self.derived.wiki_empty_text.clone() {
            column = column.child(empty_text(text));
        }
        for entry in &self.derived.wiki_entries {
            column = column.child(wiki_card(entry, theme));
        }
        column.into_any_element()
    }
}

/// Colonne défilante du résumé et du glossaire.
fn scroll_column(id: &'static str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .gap(px(12.))
        .px(px(GUTTER))
        .pt(px(4.))
        .pb(px(GUTTER))
}

/// Texte d'un corps vide (`.inv-empty`).
fn empty_text(text: impl Into<SharedString>) -> impl IntoElement {
    div()
        .px(px(GUTTER))
        .py(px(24.))
        .opacity(0.7)
        .child(text.into())
}

/// Zone d'un tableau (`.table-scroll`).
fn table_area(table: impl IntoElement) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .mx(px(GUTTER))
        .mb(px(GUTTER))
        .child(table)
        .into_any_element()
}

/// Carte (`.sum-card`, `.wiki-card`).
fn card_box(theme: &Theme) -> gpui::Div {
    div()
        .flex_none()
        .rounded(theme.radius_lg)
        .border_1()
        .border_color(theme.border_card)
        .bg(theme.kpi_bg)
        .px(px(14.))
        .py(theme.space_lg)
}

/// Carte du résumé : titre, sous-titre, lignes.
fn summary_card(ix: usize, card: SummaryCard, theme: &Theme) -> impl IntoElement {
    let mut body = card_box(theme).flex().flex_col().child(
        div()
            .flex()
            .flex_wrap()
            .items_baseline()
            .gap(px(6.))
            .mb(px(10.))
            .text_size(theme.text_md)
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.text_highlight)
                    .child(card.title.to_uppercase()),
            )
            .child(div().opacity(0.5).child(card.subtitle)),
    );
    let empty = card.lines.is_empty();
    for (line_ix, line) in card.lines.into_iter().enumerate() {
        body = body.child(summary_line(ix, line_ix, line, theme));
    }
    if empty && let Some(text) = card.empty_text {
        body = body.child(div().opacity(0.5).child(text));
    }
    body
}

/// Ligne d'une carte du résumé (`.sum-row`).
fn summary_line(card: usize, ix: usize, line: SummaryLine, theme: &Theme) -> impl IntoElement {
    let swatch = |color: &'static str| {
        div()
            .flex_none()
            .size(px(10.))
            .rounded(px(2.))
            .bg(css_hex(color).unwrap_or(theme.text_highlight))
    };
    // La répartition par protocole met la pastille avant le nom, les flux
    // après.
    let swatch_first = line.name_title.is_none();
    let name = ellipsis(
        div()
            .id(ElementId::NamedInteger(
                format!("resume-{card}").into(),
                u64::try_from(ix).unwrap_or(u64::MAX),
            ))
            .flex_none()
            .w(relative(0.34)),
    )
    .when_some(line.name_title.clone(), |d, title| {
        d.tooltip(tooltip(title))
    })
    .child(line.name.clone());
    let fill = line
        .bar_color
        .and_then(css_hex)
        .unwrap_or(theme.text_highlight);
    let percent = (line.bar_percent / 100.0).clamp(0.0, 1.0) as f32;
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .py(px(3.))
        .when_some(line.swatch.filter(|_| swatch_first), |d, color| {
            d.child(swatch(color))
        })
        .child(name)
        .when_some(line.swatch.filter(|_| !swatch_first), |d, color| {
            d.child(swatch(color))
        })
        .child(
            div()
                .flex_1()
                .h(px(8.))
                .rounded(px(4.))
                .overflow_hidden()
                .bg(track_color())
                .child(div().h_full().w(relative(percent)).rounded(px(4.)).bg(fill)),
        )
        .child(
            div()
                .flex_none()
                .min_w(px(72.))
                .text_right()
                .child(line.value),
        )
        .child(
            div()
                .flex_none()
                .min_w(px(76.))
                .text_right()
                .opacity(0.6)
                .text_size(theme.text_sm)
                .child(line.meta),
        )
}

/// Fiche du glossaire (`.wiki-card`).
fn wiki_card(entry: &GlossaryEntry, theme: &Theme) -> impl IntoElement {
    card_box(theme)
        .py(px(10.))
        .child(
            div()
                .flex()
                .items_baseline()
                .gap(px(10.))
                .mb(px(4.))
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(entry.term),
                )
                .child(
                    div()
                        .px(px(8.))
                        .py(px(1.))
                        .rounded(px(10.))
                        .border_1()
                        .border_color(rgb(0x2c4a6b))
                        .text_size(theme.text_xs)
                        .text_color(theme.text_highlight)
                        .child(entry.category.label().to_uppercase()),
                ),
        )
        .child(
            div()
                .line_height(relative(1.5))
                .opacity(0.9)
                .child(entry.definition_for(lunar_engine::APP_NAME)),
        )
}

/// En-tête d'une section du filtre (`.filter-head`).
fn filter_head(theme: &Theme, title: &'static str) -> gpui::Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.))
        .px(px(6.))
        .pt(px(4.))
        .pb(px(8.))
        .mb(px(4.))
        .border_b_1()
        .border_color(theme.border_card)
        .text_size(theme.text_sm)
        .opacity(0.85)
        .child(title.to_uppercase())
}

/// Lien du filtre et du résumé (`.filter-link`).
fn filter_link(
    id: &'static str,
    label: &'static str,
    theme: &Theme,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .flex_none()
        .text_size(theme.text_sm)
        .text_color(theme.text_highlight)
        .cursor(CursorStyle::PointingHand)
        .hover(|s| s.underline())
        .on_click(on_click)
        .child(label)
}

/// Ligne du filtre (`.filter-item`) : case, et compteur à droite.
fn filter_item(checkbox: Checkbox, count: Option<String>) -> impl IntoElement {
    let hover = item_hover();
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.))
        .px(px(4.))
        .py(px(1.))
        .rounded(px(4.))
        .hover(move |s| s.bg(hover))
        .child(div().flex_1().min_w_0().child(checkbox))
        .when_some(count, |d, count| {
            d.child(
                div()
                    .flex_none()
                    .opacity(0.6)
                    .text_size(px(12.))
                    .child(count),
            )
        })
}
