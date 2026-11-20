//! Panneau de configuration de la capture : interface, taille du buffer,
//! nombre de buffers, timeout et snaplen, persistés par le moteur
//! (`config_capture`).
//!
//! À l'ouverture, la liste des interfaces (`get_devices_list`) et la
//! configuration courante (`get_config_capture`) sont lues ; Npcap absent
//! sous Windows remplace la liste par son explication.
//! « Sauvegarder » valide, persiste, met à jour l'interface de la barre de
//! statut et ferme le panneau ; une erreur du moteur s'affiche dans le
//! dialogue d'erreur. Échap ou « Fermer » ferment sans enregistrer.
//!
//! Les règles des panneaux empêchent d'ouvrir celui-ci pendant une
//! capture ; s'il l'est quand même (course), ses champs sont grisés.
//!
//! Un bouton « Actualiser » relit la liste des interfaces sans fermer le
//! panneau (carte USB branchée après l'ouverture). Le sélecteur suit le
//! thème sombre.

use gpui::{
    AppContext, ClickEvent, Context, Entity, FocusHandle, Hsla, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Window,
    div, px, rgb,
};
use log::info;
use lunar_engine::dto::{ConnectionStatus, NetDevice};
use lunar_engine::errors::CaptureStateError;
use lunar_engine::state::capture::capture_config::CaptureConfig;
use lunar_ui_logic::capture_config::{
    INTERFACE_PLACEHOLDER, NumericField, connection_status_text, interface_label, normalize_config,
    numeric_input_error, select_interface,
};
use lunar_ui_logic::errors::npcap_missing_message;

use crate::app::panels::Panel;
use crate::app::panels::keys::{SIDE_PANEL_CONTEXT, ensure_side_panel_keys};
use crate::app::state::AppState;
use crate::assets::Icon;
use crate::theme::{ActiveTheme, Theme};
use crate::widgets::button::Button;
use crate::widgets::panel::Panel as PanelFrame;
use crate::widgets::select::{Select, SelectEvent, SelectOption};
use crate::widgets::text_input::TextInput;
use crate::widgets::tooltip::tooltip;
use crate::widgets::{Cancel, ellipsis};

/// Largeur de la liste des interfaces : corps du panneau moins le bouton
/// « Actualiser » et l'écart.
const SELECT_WIDTH: f32 = 328.;

/// Texte de l'alerte Npcap (`.config-alert { color: #ffe2b0 }`).
const ALERT_TEXT: u32 = 0xff_e2_b0;

/// Fond du bouton d'aide (`.help-tooltip { background-color: #303744 }`).
const HELP_BG: u32 = 0x30_37_44;

/// Bordure du bouton d'aide (`#8a8a8a`).
const HELP_BORDER: u32 = 0x8a_8a_8a;

/// Panneau de configuration de la capture.
pub struct ConfigPanel {
    /// État de l'application.
    state: Entity<AppState>,
    /// Focus du panneau (Échap).
    focus_handle: FocusHandle,
    /// Interfaces remontées par le moteur.
    devices: Vec<NetDevice>,
    /// Explication affichée à la place de la liste quand Npcap manque.
    npcap_missing: Option<SharedString>,
    /// Liste des interfaces.
    interface: Entity<Select>,
    /// Interface choisie.
    selected: Option<usize>,
    /// Interface de la configuration lue.
    device_name: String,
    /// Configuration lue : le choix d'interface en tient compte.
    config_loaded: bool,
    /// Champs numériques, dans l'ordre de [`NumericField::ALL`].
    fields: [Entity<TextInput>; 4],
    /// Enregistrement en cours.
    saving: bool,
    /// Capture en cours : champs grisés.
    running: bool,
    /// Abonnements à la liste et à l'état.
    _subscriptions: Vec<Subscription>,
}

impl ConfigPanel {
    /// Panneau ouvert : focalisé, liste et configuration demandées au
    /// moteur.
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        ensure_side_panel_keys(cx);
        let running = state.read(cx).capture().is_running;
        let interface = cx.new(|cx| {
            Select::new(Vec::<SelectOption>::new(), cx)
                .placeholder(INTERFACE_PLACEHOLDER)
                .width(px(SELECT_WIDTH))
                .disabled(true)
        });
        let fields = NumericField::ALL.map(|field| {
            cx.new(|cx| {
                TextInput::new(cx)
                    .with_text(&field.default_value().to_string())
                    .validator(|text| numeric_input_error(text).map(SharedString::from))
                    .disabled(running)
            })
        });
        let subscriptions = vec![
            cx.subscribe(&interface, |this, _, SelectEvent::Changed(ix), cx| {
                this.selected = Some(*ix);
                cx.notify();
            }),
            cx.observe(&state, |this, state, cx| {
                let running = state.read(cx).capture().is_running;
                if running != this.running {
                    this.running = running;
                    this.sync_disabled(cx);
                }
            }),
        ];
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle);
        let mut this = Self {
            state,
            focus_handle,
            devices: Vec::new(),
            npcap_missing: None,
            interface,
            selected: None,
            device_name: String::new(),
            config_loaded: false,
            fields,
            saving: false,
            running,
            _subscriptions: subscriptions,
        };
        this.load_interfaces(cx);
        this.load_config(cx);
        this
    }

    /// Ferme le panneau.
    fn close(&mut self, cx: &mut Context<Self>) {
        self.state
            .update(cx, |state, cx| state.close_panel(Panel::Config, cx));
    }

    /// Montre une erreur du moteur dans le dialogue d'erreur.
    fn show_error(&self, err: &CaptureStateError, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.show_error(err, cx));
    }

    /// Grise ou dégrise les champs selon la capture.
    fn sync_disabled(&mut self, cx: &mut Context<Self>) {
        let running = self.running;
        let no_device = self.devices.is_empty();
        for field in &self.fields {
            field.update(cx, |input, cx| input.set_disabled(running, cx));
        }
        self.interface.update(cx, |select, cx| {
            select.set_disabled(running || no_device, cx)
        });
        cx.notify();
    }

    /// Lit la liste des interfaces.
    fn load_interfaces(&mut self, cx: &mut Context<Self>) {
        let bridge = self.state.read(cx).bridge().clone();
        bridge
            .run(
                cx,
                |lunar| lunar.get_devices_list(),
                |this, result, cx| match result {
                    Ok(devices) => {
                        this.npcap_missing = None;
                        this.set_devices(devices, cx);
                    }
                    Err(err) => {
                        if let Some(message) = npcap_missing_message(&err) {
                            this.npcap_missing = Some(message.to_owned().into());
                            cx.notify();
                            return;
                        }
                        // Erreur nue de `get_devices_list` : enveloppée pour
                        // être traduite comme les autres erreurs de capture.
                        this.show_error(&CaptureStateError::Capture(err), cx);
                    }
                },
            )
            .detach();
    }

    /// Nouvelle liste d'interfaces.
    fn set_devices(&mut self, devices: Vec<NetDevice>, cx: &mut Context<Self>) {
        let options: Vec<SelectOption> = devices
            .iter()
            .map(|device| SelectOption::new(interface_label(device)))
            .collect();
        self.devices = devices;
        self.interface
            .update(cx, |select, cx| select.set_options(options, cx));
        self.sync_selection(cx);
        self.sync_disabled(cx);
    }

    /// Interface affichée : celle de la configuration si la liste la
    /// contient, sinon le choix automatique du sélecteur. Tant que la
    /// configuration n'est pas lue, le choix automatique s'applique.
    fn sync_selection(&mut self, cx: &mut Context<Self>) {
        let configured = if self.config_loaded {
            self.device_name.as_str()
        } else {
            ""
        };
        let selected = select_interface(&self.devices, configured);
        self.selected = selected;
        self.interface
            .update(cx, |select, cx| select.set_selected(selected, cx));
        cx.notify();
    }

    /// Lit la configuration courante.
    fn load_config(&mut self, cx: &mut Context<Self>) {
        let bridge = self.state.read(cx).bridge().clone();
        bridge
            .run(
                cx,
                |lunar| lunar.get_config_capture(),
                |this, result, cx| match result {
                    Ok(config) => {
                        info!(
                            "[Configuration] config={}",
                            serde_json::to_string(&config).unwrap_or_default()
                        );
                        this.apply_config(&config, cx);
                    }
                    Err(err) => this.show_error(&err, cx),
                },
            )
            .detach();
    }

    /// Affiche `config` et la reporte dans le store (interface de la barre
    /// de statut).
    fn apply_config(&mut self, config: &CaptureConfig, cx: &mut Context<Self>) {
        self.device_name = config.device_name.clone();
        self.config_loaded = true;
        for (field, input) in NumericField::ALL.iter().zip(&self.fields) {
            let text = field.value_in(config).to_string();
            input.update(cx, |input, cx| input.set_text(&text, cx));
        }
        let interface = config.device_name.clone();
        self.state.update(cx, |state, cx| {
            state.update_capture(|capture| capture.set_interface(interface), cx)
        });
        if !self.devices.is_empty() {
            self.sync_selection(cx);
        }
        cx.notify();
    }

    /// « Sauvegarder » : valide, persiste, ferme (`save`).
    fn save(&mut self, cx: &mut Context<Self>) {
        if self.saving || self.running {
            return;
        }
        let device = self
            .selected
            .and_then(|ix| self.devices.get(ix))
            .map(|device| device.name.clone());
        let texts = self
            .fields
            .each_ref()
            .map(|input| input.read(cx).text().to_owned());
        let payload =
            match normalize_config(device.as_deref(), texts.each_ref().map(String::as_str)) {
                Ok(payload) => payload,
                Err(err) => {
                    self.show_error(&CaptureStateError::Capture(err), cx);
                    return;
                }
            };
        self.saving = true;
        cx.notify();
        let bridge = self.state.read(cx).bridge().clone();
        bridge
            .run(
                cx,
                move |lunar| {
                    lunar.config_capture(
                        payload.device_name,
                        payload.buffer_size,
                        payload.chan_capacity,
                        payload.timeout,
                        payload.snaplen,
                    )
                },
                |this, result, cx| {
                    this.saving = false;
                    match result {
                        Ok(config) => {
                            this.apply_config(&config, cx);
                            this.close(cx);
                        }
                        Err(err) => {
                            this.show_error(&err, cx);
                            cx.notify();
                        }
                    }
                },
            )
            .detach();
    }

    /// Sélecteur d'interface, bouton « Actualiser » et détails de
    /// l'interface choisie.
    fn render_interface(&self, theme: &Theme, cx: &Context<Self>) -> impl IntoElement + use<> {
        let refresh = Button::icon_only(
            "config-actualiser",
            Icon::Refresh,
            "Actualiser la liste des interfaces",
        )
        .ghost()
        .disabled(self.running)
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.load_interfaces(cx)));
        let details = self
            .selected
            .and_then(|ix| self.devices.get(ix))
            .map(|device| render_device_details(device, theme));
        div()
            .flex()
            .flex_col()
            .gap(theme.space_md)
            .child(item_label(theme).child("Interface réseau"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(theme.space_md)
                    .child(self.interface.clone())
                    .child(refresh),
            )
            .children(details)
    }
}

/// Libellé d'un réglage (`.config-item label`).
fn item_label(theme: &Theme) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(theme.space_sm)
        .font_weight(gpui::FontWeight::BOLD)
        .text_color(theme.text)
}

/// Bouton « ? » et sa bulle d'aide (`.help-tooltip`).
fn help_button(field: NumericField, theme: &Theme) -> impl IntoElement + use<> {
    div()
        .id(gpui::ElementId::Name(
            format!("aide-{}", field.key()).into(),
        ))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(18.))
        .rounded_full()
        .border_1()
        .border_color(rgb(HELP_BORDER))
        .bg(rgb(HELP_BG))
        .text_size(theme.text_sm)
        .font_weight(gpui::FontWeight::NORMAL)
        .text_color(gpui::white())
        .tooltip(tooltip(field.help()))
        .child("?")
}

/// Couleur du statut de connexion (`.status-connected`…), éclaircie pour
/// le fond sombre.
fn status_color(status: &ConnectionStatus, theme: &Theme) -> Hsla {
    match status {
        ConnectionStatus::Connected => theme.success,
        ConnectionStatus::Disconnected => theme.danger_text,
        ConnectionStatus::Unknown | ConnectionStatus::NotApplicable => theme.text_muted,
    }
}

/// Détails de l'interface choisie : description, adresses, statut
/// (`.interface-details`).
fn render_device_details(device: &NetDevice, theme: &Theme) -> impl IntoElement + use<> {
    let detail_label = |text: &'static str| {
        div()
            .text_size(theme.text_xs)
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(theme.text_muted)
            .child(text)
    };
    let item = || div().flex().flex_col().gap(theme.space_xs);
    let mut details = div()
        .flex()
        .flex_col()
        .gap(theme.space_md)
        .p(theme.space_lg)
        .rounded(theme.radius_md)
        .border_1()
        .border_color(theme.border)
        .bg(theme.bg_raised)
        .text_size(theme.text_md);
    if let Some(desc) = device.desc.as_deref().filter(|desc| !desc.is_empty()) {
        details = details.child(item().child(detail_label("DESCRIPTION :")).child(ellipsis(
            div().text_color(theme.text_field).child(desc.to_owned()),
        )));
    }
    if !device.addresses.is_empty() {
        details =
            details.child(
                item().child(detail_label("ADRESSES :")).child(
                    div().flex().flex_wrap().gap(theme.space_xs).children(
                        device.addresses.iter().map(|address| {
                            div()
                                .px(theme.space_sm)
                                .py(px(2.))
                                .rounded(theme.radius_sm)
                                .bg(theme.bg_field)
                                .font_family(theme.font_mono.clone())
                                .text_size(theme.text_xs)
                                .text_color(theme.text_field)
                                .child(address.addr.to_string())
                        }),
                    ),
                ),
            );
    }
    let status = &device.flags.connection_status;
    details.child(
        item().child(detail_label("STATUT :")).child(
            div()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(status_color(status, theme))
                .child(connection_status_text(status)),
        ),
    )
}

impl Render for ConfigPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let alert = self.npcap_missing.clone().map(|message| {
            div()
                .px(theme.space_lg)
                .py(px(10.))
                .rounded(theme.radius_lg)
                .border_1()
                .border_color(theme.toast_warning_border)
                .bg(theme.toast_warning_bg)
                .text_color(rgb(ALERT_TEXT))
                .children(
                    message
                        .lines()
                        .map(|line| div().child(line.to_owned()))
                        .collect::<Vec<_>>(),
                )
        });
        let items = NumericField::ALL
            .iter()
            .zip(&self.fields)
            .map(|(&field, input)| {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(5.))
                    .child(
                        item_label(theme)
                            .child(field.label())
                            .child(help_button(field, theme)),
                    )
                    .child(input.clone())
            })
            .collect::<Vec<_>>();
        let actions = div()
            .flex()
            .justify_between()
            .gap(theme.space_md)
            .mt(px(10.))
            .child(
                Button::new("config-sauvegarder", "Sauvegarder")
                    .primary()
                    .disabled(self.selected.is_none() || self.saving || self.running)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.save(cx))),
            )
            .child(
                Button::new("config-fermer", "Fermer")
                    .danger()
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.close(cx))),
            );
        let body = div()
            .flex()
            .flex_col()
            .gap(px(15.))
            .children(alert)
            .child(self.render_interface(theme, cx))
            .children(items)
            .child(actions);
        div()
            .id("panneau-config-racine")
            .flex()
            .flex_none()
            .h_full()
            .track_focus(&self.focus_handle)
            .key_context(SIDE_PANEL_CONTEXT)
            .on_action(cx.listener(|this, _: &Cancel, _, cx| this.close(cx)))
            .child(
                PanelFrame::new("panneau-config", Panel::Config.title())
                    .width(Panel::Config.width())
                    .on_close(cx.listener(|this, _: &ClickEvent, _, cx| this.close(cx)))
                    .child(body),
            )
    }
}
