//! Panneau de filtre BPF : presets rapides,
//! couches, protocoles, adresses, ports, expression brute, aperçu éditable
//! de l'expression générée, appliquée par `set_filter`.
//!
//! La construction et la validation sont celles de
//! [`lunar_ui_logic::bpf::FilterForm`] : l'aperçu suit les champs tant
//! qu'il n'est pas édité à la main (badge « édition manuelle »), « ↺ Sync
//! auto » le recale. « Appliquer » enregistre le filtre : actif tout de
//! suite hors capture, en attente du prochain démarrage pendant une capture
//! (promu par le store à l'événement `Started`). « Supprimer » et
//! « Réinitialiser » effacent le filtre côté moteur, « Annuler » rend au
//! moteur le filtre actif à la place du filtre en attente.
//!
//! Mise en page : une colonne, le panneau latéral faisant 520 px ; seules
//! les cartes « Couche / Types » et « Protocoles », étroites, restent côte
//! à côte. L'expression brute est sur une ligne, l'aperçu sur trois.

use gpui::{
    AppContext, ClickEvent, Context, Entity, FocusHandle, InteractiveElement, IntoElement,
    ParentElement, Render, StatefulInteractiveElement, Styled, Subscription, Window, div,
    prelude::FluentBuilder, px, rgb, rgba,
};
use log::error;
use lunar_ui_logic::bpf::{BpfFormState, BpfPreset, Direction, FieldError, FilterForm, FilterSlot};

use crate::app::panels::Panel;
use crate::app::panels::keys::{SIDE_PANEL_CONTEXT, ensure_side_panel_keys};
use crate::app::state::AppState;
use crate::theme::{ActiveTheme, Theme};
use crate::widgets::button::{Button, ButtonSize};
use crate::widgets::checkbox::Checkbox;
use crate::widgets::panel::Panel as PanelFrame;
use crate::widgets::select::{Select, SelectEvent};
use crate::widgets::text_input::{TextInput, TextInputEvent};
use crate::widgets::{Cancel, ellipsis};

/// Titres des cartes (`.card h3 { color: #7070a0 }`).
const CARD_TITLE: u32 = 0x70_70_a0;
/// Libellés des champs (`.field label { color: #8888b0 }`).
const FIELD_LABEL: u32 = 0x88_88_b0;
/// Code des cases à cocher (`.row code`).
const CODE_TEXT: u32 = 0x64_83_c1;
/// Fond du code des cases à cocher.
const CODE_BG: u32 = 0x1a_1a_2e;
/// Indications (`.hint { color: #50507a }`).
const HINT_TEXT: u32 = 0x50_50_7a;
/// Code des indications (`.hint code`).
const HINT_CODE: u32 = 0x50_70_b0;
/// Expression du filtre actif ou en attente (`.active-filter-expr`).
const FILTER_EXPR: u32 = 0x8a_b4_ff;
/// Libellé du filtre actif (`.active-filter-label`).
const ACTIVE_LABEL: u32 = 0x4a_7c_ff;
/// Libellé du filtre en attente et badge « édition manuelle ».
const PENDING_LABEL: u32 = 0xf0_a0_30;
/// Fond des presets (`.chip`).
const CHIP_BG: u32 = 0x1e_1e_30;
/// Texte des presets.
const CHIP_TEXT: u32 = 0x83_83_b2;
/// Texte des presets survolés.
const CHIP_TEXT_HOVER: u32 = 0xc0_c0_f0;

/// Largeur des listes « Direction » : carte moins le libellé de 110 px.
const DIRECTION_WIDTH: f32 = 336.;

/// Champ texte du formulaire, dans l'ordre des entités de saisie.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextField {
    /// Inclure hôte.
    IncludeHost,
    /// Exclure hôte.
    ExcludeHost,
    /// Inclure réseau.
    IncludeNet,
    /// Exclure réseau.
    ExcludeNet,
    /// Inclure port(s).
    PortInclude,
    /// Exclure port(s).
    PortExclude,
    /// Plage de ports.
    PortRange,
    /// Expression BPF brute.
    Raw,
}

impl TextField {
    /// Tous les champs.
    const ALL: [TextField; 8] = [
        TextField::IncludeHost,
        TextField::ExcludeHost,
        TextField::IncludeNet,
        TextField::ExcludeNet,
        TextField::PortInclude,
        TextField::PortExclude,
        TextField::PortRange,
        TextField::Raw,
    ];

    /// Libellé de la ligne (vide pour l'expression brute, sans libellé).
    const fn label(self) -> &'static str {
        match self {
            TextField::IncludeHost => "Inclure hôte",
            TextField::ExcludeHost => "Exclure hôte",
            TextField::IncludeNet => "Inclure réseau",
            TextField::ExcludeNet => "Exclure réseau",
            TextField::PortInclude => "Inclure port(s)",
            TextField::PortExclude => "Exclure port(s)",
            TextField::PortRange => "Plage",
            TextField::Raw => "",
        }
    }

    /// Texte indicatif.
    const fn placeholder(self) -> &'static str {
        match self {
            TextField::IncludeHost => "ex: 192.168.1.42",
            TextField::ExcludeHost => "ex: 10.0.0.1",
            TextField::IncludeNet => "ex: 10.0.0.0/8",
            TextField::ExcludeNet => "ex: 192.168.0.0/16",
            TextField::PortInclude => "ex: 80,443,22",
            TextField::PortExclude => "ex: 25,21",
            TextField::PortRange => "ex: 10000-20000",
            TextField::Raw => "ex: tcp[13] & 0x02 != 0 and tcp[13] & 0x10 = 0",
        }
    }

    /// Valeur dans le formulaire.
    fn value(self, form: &BpfFormState) -> &str {
        match self {
            TextField::IncludeHost => &form.ip.include_host,
            TextField::ExcludeHost => &form.ip.exclude_host,
            TextField::IncludeNet => &form.ip.include_net,
            TextField::ExcludeNet => &form.ip.exclude_net,
            TextField::PortInclude => &form.ports.include,
            TextField::PortExclude => &form.ports.exclude,
            TextField::PortRange => &form.ports.range,
            TextField::Raw => &form.advanced_raw,
        }
    }

    /// Valeur à modifier.
    fn value_mut(self, form: &mut BpfFormState) -> &mut String {
        match self {
            TextField::IncludeHost => &mut form.ip.include_host,
            TextField::ExcludeHost => &mut form.ip.exclude_host,
            TextField::IncludeNet => &mut form.ip.include_net,
            TextField::ExcludeNet => &mut form.ip.exclude_net,
            TextField::PortInclude => &mut form.ports.include,
            TextField::PortExclude => &mut form.ports.exclude,
            TextField::PortRange => &mut form.ports.range,
            TextField::Raw => &mut form.advanced_raw,
        }
    }
}

/// Case à cocher du formulaire.
struct Toggle {
    /// Identifiant de l'élément.
    id: &'static str,
    /// Libellé.
    label: &'static str,
    /// Clause BPF affichée en code après le libellé.
    code: Option<&'static str>,
    /// Lecture de l'état.
    get: fn(&BpfFormState) -> bool,
    /// Écriture de l'état.
    set: fn(&mut BpfFormState, bool),
}

/// Cases « Couche / Types ».
fn layer_toggles() -> [Toggle; 4] {
    [
        Toggle {
            id: "filtre-vlan",
            label: "Trafic VLAN (802.1Q)",
            code: None,
            get: |s| s.opt.vlan,
            set: |s, on| s.opt.vlan = on,
        },
        Toggle {
            id: "filtre-ipv4",
            label: "Uniquement IPv4",
            code: Some("ip"),
            get: |s| s.opt.only_ip4,
            set: |s, on| s.opt.only_ip4 = on,
        },
        Toggle {
            id: "filtre-sans-ipv6",
            label: "Exclure IPv6",
            code: Some("not ip6"),
            get: |s| s.opt.exclude_ipv6,
            set: |s, on| s.opt.exclude_ipv6 = on,
        },
        Toggle {
            id: "filtre-sans-arp",
            label: "Exclure ARP",
            code: Some("not arp"),
            get: |s| s.opt.exclude_arp,
            set: |s, on| s.opt.exclude_arp = on,
        },
    ]
}

/// Cases « Protocoles ».
fn proto_toggles() -> [Toggle; 4] {
    [
        Toggle {
            id: "filtre-tcp",
            label: "TCP",
            code: None,
            get: |s| s.proto.tcp,
            set: |s, on| s.proto.tcp = on,
        },
        Toggle {
            id: "filtre-udp",
            label: "UDP",
            code: None,
            get: |s| s.proto.udp,
            set: |s, on| s.proto.udp = on,
        },
        Toggle {
            id: "filtre-icmp",
            label: "ICMP",
            code: None,
            get: |s| s.proto.icmp,
            set: |s, on| s.proto.icmp = on,
        },
        Toggle {
            id: "filtre-icmp6",
            label: "ICMPv6",
            code: None,
            get: |s| s.proto.icmp6,
            set: |s, on| s.proto.icmp6 = on,
        },
    ]
}

/// Indice d'une direction dans la liste « Direction ».
fn direction_index(direction: Direction) -> usize {
    Direction::ALL
        .iter()
        .position(|candidate| *candidate == direction)
        .unwrap_or(0)
}

/// Panneau de filtre BPF.
pub struct FilterPanel {
    /// État de l'application.
    state: Entity<AppState>,
    /// Focus du panneau (Échap).
    focus_handle: FocusHandle,
    /// Formulaire et aperçu.
    form: FilterForm,
    /// Champs texte, dans l'ordre de [`TextField::ALL`].
    inputs: [Entity<TextInput>; 8],
    /// Zone d'aperçu, éditable.
    preview: Entity<TextInput>,
    /// Direction des adresses.
    ip_direction: Entity<Select>,
    /// Direction des ports.
    port_direction: Entity<Select>,
    /// Abonnements aux champs et à l'état.
    _subscriptions: Vec<Subscription>,
}

impl FilterPanel {
    /// Panneau ouvert sur un formulaire vierge, focalisé.
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        ensure_side_panel_keys(cx);
        let inputs = TextField::ALL.map(|field| {
            cx.new(|cx| {
                let input = TextInput::new(cx).placeholder(field.placeholder());
                if field == TextField::Raw {
                    input.mono()
                } else {
                    input
                }
            })
        });
        let preview = cx.new(|cx| {
            TextInput::multiline(3, cx)
                .placeholder("Le filtre BPF généré apparaît ici…")
                .mono()
        });
        let direction_select = |cx: &mut Context<Self>| {
            cx.new(|cx| {
                Select::new(Direction::ALL.map(Direction::label), cx)
                    .selected(Some(0))
                    .width(px(DIRECTION_WIDTH))
            })
        };
        let ip_direction = direction_select(cx);
        let port_direction = direction_select(cx);

        let mut subscriptions = Vec::new();
        for (field, input) in TextField::ALL.into_iter().zip(&inputs) {
            subscriptions.push(
                cx.subscribe(input, move |this, _, event: &TextInputEvent, cx| {
                    if let TextInputEvent::Changed(text) = event {
                        *field.value_mut(this.form.form_mut()) = text.to_string();
                        this.form_changed(cx);
                    }
                }),
            );
        }
        subscriptions.push(
            cx.subscribe(&preview, |this, _, event: &TextInputEvent, cx| {
                if let TextInputEvent::Changed(text) = event {
                    this.form.edit_preview(text.to_string());
                    cx.notify();
                }
            }),
        );
        subscriptions.push(
            cx.subscribe(&ip_direction, |this, _, SelectEvent::Changed(ix), cx| {
                this.form.form_mut().ip.direction = Direction::ALL[(*ix).min(2)];
                this.form_changed(cx);
            }),
        );
        subscriptions.push(cx.subscribe(
            &port_direction,
            |this, _, SelectEvent::Changed(ix), cx| {
                this.form.form_mut().ports.direction = Direction::ALL[(*ix).min(2)];
                this.form_changed(cx);
            },
        ));
        // Filtres actif et en attente affichés en tête.
        subscriptions.push(cx.observe(&state, |_, _, cx| cx.notify()));

        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle);
        Self {
            state,
            focus_handle,
            form: FilterForm::new(),
            inputs,
            preview,
            ip_direction,
            port_direction,
            _subscriptions: subscriptions,
        }
    }

    /// Ferme le panneau.
    fn close(&mut self, cx: &mut Context<Self>) {
        self.state
            .update(cx, |state, cx| state.close_panel(Panel::Filter, cx));
    }

    /// Un champ a changé : l'aperçu automatique suit.
    fn form_changed(&mut self, cx: &mut Context<Self>) {
        if !self.form.is_manual_preview() {
            let text = self.form.preview_text();
            self.preview.update(cx, |preview, cx| {
                if preview.text() != text {
                    preview.set_text(&text, cx);
                }
            });
        }
        cx.notify();
    }

    /// Recopie le formulaire dans les champs (preset, réinitialisation).
    fn push_form(&mut self, cx: &mut Context<Self>) {
        let form = self.form.form().clone();
        for (field, input) in TextField::ALL.into_iter().zip(&self.inputs) {
            let text = field.value(&form).to_owned();
            input.update(cx, |input, cx| {
                if input.text() != text {
                    input.set_text(&text, cx);
                }
            });
        }
        let ip = direction_index(form.ip.direction);
        let ports = direction_index(form.ports.direction);
        self.ip_direction
            .update(cx, |select, cx| select.set_selected(Some(ip), cx));
        self.port_direction
            .update(cx, |select, cx| select.set_selected(Some(ports), cx));
        let text = self.form.preview_text();
        self.preview
            .update(cx, |preview, cx| preview.set_text(&text, cx));
        cx.notify();
    }

    /// Preset rapide : formulaire vierge puis champs du preset.
    fn apply_preset(&mut self, preset: BpfPreset, cx: &mut Context<Self>) {
        self.form.apply_preset(preset);
        self.push_form(cx);
    }

    /// « ↺ Sync auto ».
    fn sync_preview(&mut self, cx: &mut Context<Self>) {
        self.form.sync_preview();
        let text = self.form.preview_text();
        self.preview
            .update(cx, |preview, cx| preview.set_text(&text, cx));
        cx.notify();
    }

    /// « Appliquer » : filtre enregistré côté moteur, actif ou en attente
    /// selon la capture, puis fermeture (`apply`).
    fn apply(&mut self, cx: &mut Context<Self>) {
        let Some(filter) = self.form.filter_to_apply() else {
            return;
        };
        let bridge = self.state.read(cx).bridge().clone();
        let sent = filter.clone();
        bridge
            .run(
                cx,
                move |lunar| lunar.set_filter(sent),
                move |this, result, cx| match result {
                    Ok(()) => {
                        this.state.update(cx, |state, cx| {
                            let slot = FilterSlot::for_capture(state.capture().is_running);
                            state.update_capture(
                                |capture| match slot {
                                    FilterSlot::Active => capture.set_active_filter(filter),
                                    FilterSlot::Pending => capture.set_pending_filter(filter),
                                },
                                cx,
                            );
                        });
                        this.close(cx);
                    }
                    Err(err) => error!("set_filter failed: {err}"),
                },
            )
            .detach();
    }

    /// « Réinitialiser » et « Supprimer » : formulaire vierge, filtre
    /// effacé côté moteur.
    fn reset_all(&mut self, cx: &mut Context<Self>) {
        self.form.reset();
        self.push_form(cx);
        let bridge = self.state.read(cx).bridge().clone();
        bridge
            .run(
                cx,
                |lunar| lunar.set_filter(String::new()),
                |this, result, cx| match result {
                    Ok(()) => this.state.update(cx, |state, cx| {
                        state.update_capture(
                            |capture| {
                                capture.set_active_filter("");
                                capture.set_pending_filter("");
                            },
                            cx,
                        )
                    }),
                    Err(err) => error!("clear filter failed: {err}"),
                },
            )
            .detach();
    }

    /// « Annuler » le filtre en attente : le moteur reprend le filtre actif.
    fn cancel_pending(&mut self, cx: &mut Context<Self>) {
        let active = self.state.read(cx).capture().active_filter.clone();
        let bridge = self.state.read(cx).bridge().clone();
        bridge
            .run(
                cx,
                move |lunar| lunar.set_filter(active),
                |this, result, cx| match result {
                    Ok(()) => this.state.update(cx, |state, cx| {
                        state.update_capture(|capture| capture.set_pending_filter(""), cx)
                    }),
                    Err(err) => error!("cancel pending failed: {err}"),
                },
            )
            .detach();
    }

    /// Case à cocher du formulaire.
    fn render_toggle(&self, toggle: Toggle, theme: &Theme, cx: &Context<Self>) -> gpui::Div {
        let set = toggle.set;
        let checkbox = Checkbox::new(toggle.id, toggle.label, (toggle.get)(self.form.form()))
            .on_change(cx.listener(move |this, checked: &bool, _, cx| {
                set(this.form.form_mut(), *checked);
                this.form_changed(cx);
            }));
        div()
            .flex()
            .items_center()
            .gap(px(3.))
            .child(checkbox)
            .when_some(toggle.code, |row, code| {
                row.child(
                    div()
                        .flex()
                        .items_center()
                        .text_size(theme.text_md)
                        .text_color(theme.text)
                        .child("(")
                        .child(code_chip(code, theme))
                        .child(")"),
                )
            })
    }

    /// Ligne libellé + champ (`.field`).
    fn render_field(&self, field: TextField, theme: &Theme) -> gpui::Div {
        let index = TextField::ALL
            .iter()
            .position(|candidate| *candidate == field)
            .unwrap_or(0);
        field_row(field.label(), theme)
            .child(div().flex_1().min_w_0().child(self.inputs[index].clone()))
    }
}

/// Clause BPF en code (`.row code`).
fn code_chip(code: &'static str, theme: &Theme) -> impl IntoElement + use<> {
    div()
        .px(px(4.))
        .rounded(px(3.))
        .bg(rgb(CODE_BG))
        .font_family(theme.font_mono.clone())
        .text_size(theme.text_xs)
        .text_color(rgb(CODE_TEXT))
        .child(code)
}

/// Ligne d'un champ : libellé de 110 px puis contrôle (`.field`).
fn field_row(label: &'static str, theme: &Theme) -> gpui::Div {
    div().flex().items_center().gap(theme.space_md).child(
        div()
            .flex_none()
            .w(px(110.))
            .text_size(theme.text_md)
            .text_color(rgb(FIELD_LABEL))
            .child(label),
    )
}

/// Carte titrée (`.card` et son `h3`).
fn card(title: &'static str, theme: &Theme) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(theme.space_md)
        .p(px(14.))
        .rounded(theme.radius_xl)
        .border_1()
        .border_color(theme.border)
        .bg(theme.bg_raised)
        .child(card_title(title, theme))
}

/// Titre d'une carte, en capitales.
fn card_title(title: &'static str, theme: &Theme) -> gpui::Div {
    div()
        .text_size(theme.text_xs)
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(rgb(CARD_TITLE))
        .child(title)
}

/// Indication sous un groupe de champs (`.hint`).
fn hint(theme: &Theme) -> gpui::Div {
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .text_size(theme.text_sm)
        .text_color(rgb(HINT_TEXT))
}

/// Erreurs de saisie (`.errors`) ; rien sans erreur.
fn errors(errors: Vec<FieldError>, theme: &Theme) -> Option<gpui::Div> {
    (!errors.is_empty()).then(|| {
        div()
            .flex()
            .flex_col()
            .gap(px(3.))
            .text_size(theme.text_sm)
            .text_color(theme.danger_text)
            .children(errors.into_iter().map(|e| div().child(format!("• {e}"))))
    })
}

/// Bandeau du filtre actif ou en attente.
fn filter_bar(
    label: &'static str,
    label_color: u32,
    bg: u32,
    border: u32,
    expression: String,
    button: Button,
    theme: &Theme,
) -> impl IntoElement + use<> {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .px(theme.space_lg)
        .py(theme.space_xs)
        .rounded(theme.radius_lg)
        .border_1()
        .border_color(rgba(border))
        .bg(rgba(bg))
        .child(
            div()
                .flex_none()
                .text_size(theme.text_xs)
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgb(label_color))
                .child(label),
        )
        .child(ellipsis(
            div()
                .flex_1()
                .min_w_0()
                .font_family(theme.font_mono.clone())
                .text_size(theme.text_sm)
                .text_color(rgb(FILTER_EXPR))
                .child(expression),
        ))
        .child(button)
}

impl Render for FilterPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let capture = self.state.read(cx).capture();
        let active = capture.active_filter.clone();
        let pending = capture.pending_filter.clone();

        let active_bar = (!active.is_empty()).then(|| {
            filter_bar(
                "FILTRE ACTIF",
                ACTIVE_LABEL,
                0x4a7c_ff14,
                0x4a7c_ff4d,
                active,
                Button::new("filtre-supprimer", "Supprimer")
                    .ghost()
                    .size(ButtonSize::Small)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.reset_all(cx))),
                theme,
            )
        });
        let pending_bar = (!pending.is_empty()).then(|| {
            filter_bar(
                "PROCHAIN DÉMARRAGE",
                PENDING_LABEL,
                0xf0a0_3012,
                0xf0a0_3047,
                pending,
                Button::new("filtre-annuler-attente", "Annuler")
                    .ghost()
                    .size(ButtonSize::Small)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.cancel_pending(cx))),
                theme,
            )
        });

        let presets = card("PRESETS RAPIDES", theme).child(
            div()
                .flex()
                .flex_wrap()
                .gap(theme.space_md)
                .children(BpfPreset::ALL.map(|preset| {
                    div()
                        .id(gpui::ElementId::Name(
                            format!("preset-{}", preset.key()).into(),
                        ))
                        .px(px(13.))
                        .py(px(5.))
                        .rounded_full()
                        .border_1()
                        .border_color(theme.border)
                        .bg(rgb(CHIP_BG))
                        .text_size(theme.text_sm)
                        .text_color(rgb(CHIP_TEXT))
                        .cursor_pointer()
                        .tab_index(0)
                        .hover(|s| {
                            s.bg(theme.bg_hover)
                                .border_color(theme.focus_ring)
                                .text_color(rgb(CHIP_TEXT_HOVER))
                        })
                        .focus(|s| s.border_color(theme.focus_ring))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.apply_preset(preset, cx)
                        }))
                        .child(preset.label())
                })),
        );

        let [vlan, ipv4, no_ipv6, no_arp] = layer_toggles();
        let layers = card("COUCHE / TYPES", theme)
            .flex_1()
            .min_w_0()
            .gap(px(6.))
            .child(self.render_toggle(vlan, theme, cx))
            .child(self.render_toggle(ipv4, theme, cx))
            .child(self.render_toggle(no_ipv6, theme, cx))
            .child(self.render_toggle(no_arp, theme, cx));
        let [tcp, udp, icmp, icmp6] = proto_toggles();
        let proto_row = |a, b| {
            div()
                .flex()
                .child(div().flex_1().child(self.render_toggle(a, theme, cx)))
                .child(div().flex_1().child(self.render_toggle(b, theme, cx)))
        };
        let protocols = card("PROTOCOLES", theme)
            .flex_1()
            .min_w_0()
            .gap(px(6.))
            .child(proto_row(tcp, udp))
            .child(proto_row(icmp, icmp6))
            .child(hint(theme).child("Vide = pas de restriction de protocole."));

        let addresses = card("ADRESSES IP", theme)
            .child(self.render_field(TextField::IncludeHost, theme))
            .child(self.render_field(TextField::ExcludeHost, theme))
            .child(self.render_field(TextField::IncludeNet, theme))
            .child(self.render_field(TextField::ExcludeNet, theme))
            .child(field_row("Direction", theme).child(self.ip_direction.clone()))
            .children(errors(self.form.ip_errors(), theme));

        let ports = card("PORTS", theme)
            .child(self.render_field(TextField::PortInclude, theme))
            .child(self.render_field(TextField::PortExclude, theme))
            .child(self.render_field(TextField::PortRange, theme))
            .child(field_row("Direction", theme).child(self.port_direction.clone()))
            .child(hint(theme).child("S'applique principalement à TCP/UDP."))
            .children(errors(self.form.port_errors(), theme));

        let raw_index = TextField::ALL.len() - 1;
        let raw = card("EXPRESSION BPF BRUTE", theme)
            .child(self.inputs[raw_index].clone())
            .child(
                hint(theme)
                    .child("Concaténé à la fin du filtre généré avec ")
                    .child(
                        div()
                            .font_family(theme.font_mono.clone())
                            .text_color(rgb(HINT_CODE))
                            .child("and"),
                    )
                    .child("."),
            );

        let manual = self.form.is_manual_preview();
        let preview_header = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(card_title("APERÇU", theme))
            .when(manual, |header| {
                header.child(
                    div()
                        .px(px(7.))
                        .py(px(2.))
                        .rounded(theme.radius_sm)
                        .border_1()
                        .border_color(rgba(0xf0a0_3040))
                        .bg(rgba(0xf0a0_301a))
                        .text_size(theme.text_xs)
                        .text_color(rgb(PENDING_LABEL))
                        .child("édition manuelle"),
                )
            });
        let actions = div()
            .flex()
            .flex_wrap()
            .gap(theme.space_md)
            .mt(px(2.))
            .child(
                Button::new("filtre-appliquer", "Appliquer")
                    .primary()
                    .disabled(!self.form.can_apply())
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.apply(cx))),
            )
            .child(
                Button::new("filtre-reinitialiser", "Réinitialiser")
                    .ghost()
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.reset_all(cx))),
            )
            .when(manual, |row| {
                row.child(
                    Button::new("filtre-sync", "↺ Sync auto")
                        .ghost()
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.sync_preview(cx))),
                )
            });
        let preview = div()
            .flex()
            .flex_col()
            .gap(theme.space_md)
            .p(px(14.))
            .rounded(theme.radius_xl)
            .border_1()
            .border_color(theme.border)
            .bg(theme.bg_raised)
            .child(preview_header)
            .child(self.preview.clone())
            .children(errors(self.form.errors(), theme))
            .child(actions);

        let body = div()
            .flex()
            .flex_col()
            .gap(theme.space_lg)
            .children(active_bar)
            .children(pending_bar)
            .child(presets)
            .child(
                div()
                    .flex()
                    .gap(theme.space_lg)
                    .child(layers)
                    .child(protocols),
            )
            .child(addresses)
            .child(ports)
            .child(raw)
            .child(preview);

        div()
            .id("panneau-filtre-racine")
            .flex()
            .flex_none()
            .h_full()
            .track_focus(&self.focus_handle)
            .key_context(SIDE_PANEL_CONTEXT)
            .on_action(cx.listener(|this, _: &Cancel, _, cx| this.close(cx)))
            .child(
                PanelFrame::new("panneau-filtre", Panel::Filter.title())
                    .width(Panel::Filter.width())
                    .on_close(cx.listener(|this, _: &ClickEvent, _, cx| this.close(cx)))
                    .child(body),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn champs_lus_et_ecrits_au_meme_endroit() {
        let mut form = BpfFormState::default();
        for (index, field) in TextField::ALL.into_iter().enumerate() {
            *field.value_mut(&mut form) = format!("valeur {index}");
        }
        for (index, field) in TextField::ALL.into_iter().enumerate() {
            assert_eq!(field.value(&form), format!("valeur {index}"), "{field:?}");
        }
        assert_eq!(form.advanced_raw, "valeur 7");
        assert_eq!(form.ports.range, "valeur 6");
    }

    #[test]
    fn cases_lues_et_ecrites_au_meme_endroit() {
        for toggle in layer_toggles().into_iter().chain(proto_toggles()) {
            let mut form = BpfFormState::default();
            (toggle.set)(&mut form, true);
            assert!((toggle.get)(&form), "{}", toggle.id);
            assert_ne!(form, BpfFormState::default(), "{}", toggle.id);
        }
    }

    #[test]
    fn indice_des_directions() {
        for (index, direction) in Direction::ALL.into_iter().enumerate() {
            assert_eq!(direction_index(direction), index);
        }
    }
}
