//! Galerie des composants LUNAR : chaque composant dans ses états,
//! dont un tableau de 10 000 lignes. Sert aux captures d'écran.
//!
//! `cargo run --release -p lunar --example gallery -- [options]`
//!
//! Options (états figés pour les captures) :
//! - `--menu` : ouvre la première section de la barre de menus ;
//! - `--select` : ouvre la liste des interfaces ;
//! - `--modal` : ouvre la modale de configuration ;
//! - `--confirm` : ouvre la confirmation de réinitialisation ;
//! - `--toasts` : affiche une notification de chaque nature, sans échéance ;
//! - `--focus` : donne le focus au champ d'adresse IP ;
//! - `--edit` : ouvre l'édition en place d'un label du tableau.

use std::cmp::Ordering;
use std::sync::atomic::{AtomicU32, Ordering as AtomicOrdering};

use gpui::{
    AnyElement, App, Application, Bounds, ClickEvent, Context, Entity, FocusHandle, Focusable,
    IntoElement, Render, SharedString, TitlebarOptions, Window, WindowBounds, WindowOptions, div,
    img, prelude::*, px, size,
};
use lunar::assets::{Assets, Icon, LOGO_PNG, icon};
use lunar::platform::{force_x11_backend, install_x11_wakeup};
use lunar::theme::{ActiveTheme, Theme};
use lunar::widgets::button::{Button, ButtonSize};
use lunar::widgets::checkbox::{Checkbox, Switch};
use lunar::widgets::kpi::Kpi;
use lunar::widgets::menu_bar::{MenuBar, MenuBarEvent, MenuEntry, MenuSection};
use lunar::widgets::modal::{Confirm, Modal};
use lunar::widgets::panel::Panel;
use lunar::widgets::progress::ProgressBar;
use lunar::widgets::radio::RadioGroup;
use lunar::widgets::select::{Select, SelectEvent, SelectOption};
use lunar::widgets::table::{Column, Table, TableDelegate, TableEvent};
use lunar::widgets::tabs::{Tab, Tabs};
use lunar::widgets::text_input::{TextInput, TextInputEvent};
use lunar::widgets::toast::{ToastKind, ToastStack};
use lunar::widgets::tooltip::tooltip_with_shortcut;
use lunar::widgets::{self, Cancel, root_actions};

/// Nombre de rendus de la vue, lu par le réveil X11.
static RENDERS: AtomicU32 = AtomicU32::new(0);

/// Nombre de lignes du tableau de démonstration.
const ROWS: usize = 10_000;

/// Colonne des labels, éditable en place.
const LABEL_COLUMN: usize = 6;

/// États figés demandés sur la ligne de commande.
#[derive(Clone, Copy, Debug, Default)]
struct Args {
    /// Barre de menus ouverte.
    menu: bool,
    /// Liste des interfaces ouverte.
    select: bool,
    /// Modale ouverte.
    modal: bool,
    /// Confirmation ouverte.
    confirm: bool,
    /// Notifications sans échéance.
    toasts: bool,
    /// Focus sur le champ d'adresse.
    focus: bool,
    /// Édition en place d'un label.
    edit: bool,
}

impl Args {
    /// Lit les options.
    fn parse() -> Self {
        let mut args = Self::default();
        for arg in std::env::args().skip(1) {
            match arg.as_str() {
                "--menu" => args.menu = true,
                "--select" => args.select = true,
                "--modal" => args.modal = true,
                "--confirm" => args.confirm = true,
                "--toasts" => args.toasts = true,
                "--focus" => args.focus = true,
                "--edit" => args.edit = true,
                other => eprintln!("option inconnue ignorée : {other}"),
            }
        }
        args
    }
}

/// Générateur pseudo-aléatoire déterministe (xorshift).
struct Rng(u64);

impl Rng {
    /// Tire un entier.
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// Flux de démonstration.
struct Flow {
    /// Adresse source.
    src: SharedString,
    /// Adresse destination.
    dst: SharedString,
    /// Protocole.
    proto: &'static str,
    /// Port de destination.
    port: u16,
    /// Paquets.
    packets: u64,
    /// Octets.
    bytes: u64,
    /// Label de la destination.
    label: Option<SharedString>,
}

/// Source du tableau : 10 000 flux, label éditable en place.
struct Flows {
    /// Colonnes.
    columns: Vec<Column>,
    /// Lignes.
    rows: Vec<Flow>,
    /// Ligne dont le label est en cours d'édition.
    editing: Option<usize>,
    /// Champ d'édition du label.
    editor: Entity<TextInput>,
}

impl Flows {
    /// Génère les flux.
    fn new(editor: Entity<TextInput>) -> Self {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let protos = [
            ("Modbus/TCP", 502),
            ("S7comm", 102),
            ("DNS", 53),
            ("HTTPS", 443),
            ("NTP", 123),
            ("OPC UA", 4840),
            ("ARP", 0),
        ];
        let labels = ["Automate A1", "IHM salle 2", "Serveur SCADA", "Historian"];
        let rows = (0..ROWS)
            .map(|i| {
                let (proto, port) = protos[(rng.next() % protos.len() as u64) as usize];
                let host =
                    |r: u64| format!("10.{}.{}.{}", (r >> 16) % 4, (r >> 8) % 32, r % 254 + 1);
                let packets = rng.next() % 250_000 + 1;
                Flow {
                    src: host(rng.next()).into(),
                    dst: host(rng.next()).into(),
                    proto,
                    port,
                    packets,
                    bytes: packets * (60 + rng.next() % 1400),
                    label: (i % 7 != 3).then(|| labels[i % labels.len()].into()),
                }
            })
            .collect();
        Self {
            columns: vec![
                Column::new("Source", px(112.)).sortable().mono(),
                Column::new("Destination", px(126.)).sortable().mono(),
                Column::new("Protocole", px(104.)).sortable(),
                Column::new("Port", px(68.)).numeric().sortable().mono(),
                Column::new("Paquets", px(92.)).numeric().sortable(),
                Column::new("Octets", px(98.)).numeric().sortable(),
                Column::flex("Label", 1.).sortable(),
            ],
            rows,
            editing: None,
            editor,
        }
    }
}

/// Nombre avec espaces fines de milliers, à la française.
fn group_digits(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push('\u{202f}');
        }
        out.push(c);
    }
    out
}

impl TableDelegate for Flows {
    fn columns(&self) -> &[Column] {
        &self.columns
    }

    fn row_count(&self) -> usize {
        self.rows.len()
    }

    fn cell_text(&self, row: usize, column: usize) -> SharedString {
        let Some(flow) = self.rows.get(row) else {
            return SharedString::default();
        };
        match column {
            0 => flow.src.clone(),
            1 => flow.dst.clone(),
            2 => flow.proto.into(),
            3 => {
                if flow.port == 0 {
                    "—".into()
                } else {
                    flow.port.to_string().into()
                }
            }
            4 => group_digits(flow.packets).into(),
            5 => group_digits(flow.bytes).into(),
            _ => flow.label.clone().unwrap_or_else(|| "à compléter".into()),
        }
    }

    fn compare(&self, a: usize, b: usize, column: usize) -> Ordering {
        let (Some(fa), Some(fb)) = (self.rows.get(a), self.rows.get(b)) else {
            return Ordering::Equal;
        };
        match column {
            3 => fa.port.cmp(&fb.port),
            4 => fa.packets.cmp(&fb.packets),
            5 => fa.bytes.cmp(&fb.bytes),
            _ => self.cell_text(a, column).cmp(&self.cell_text(b, column)),
        }
    }

    fn render_cell(
        &self,
        row: usize,
        column: usize,
        _window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        if column != LABEL_COLUMN {
            return None;
        }
        if self.editing == Some(row) {
            return Some(self.editor.clone().into_any_element());
        }
        let flow = self.rows.get(row)?;
        if flow.label.is_some() {
            return None;
        }
        let theme = cx.theme();
        Some(
            div()
                .truncate()
                .italic()
                .text_color(theme.text_placeholder)
                .child("à compléter")
                .into_any_element(),
        )
    }

    fn row_warning(&self, row: usize) -> bool {
        self.rows.get(row).is_some_and(|f| f.label.is_none())
    }
}

/// Vue de la galerie.
struct Gallery {
    /// Focus racine.
    focus_handle: FocusHandle,
    /// Barre de menus.
    menu_bar: Entity<MenuBar>,
    /// Notifications.
    toasts: Entity<ToastStack>,
    /// Champ simple.
    name: Entity<TextInput>,
    /// Champ validé (adresse IP).
    address: Entity<TextInput>,
    /// Champ désactivé.
    disabled_input: Entity<TextInput>,
    /// Expression BPF sur plusieurs lignes.
    bpf: Entity<TextInput>,
    /// Liste des interfaces.
    interface: Entity<Select>,
    /// Liste des protocoles.
    protocol: Entity<Select>,
    /// Liste désactivée.
    disabled_select: Entity<Select>,
    /// Protocole du panneau latéral.
    panel_protocol: Entity<Select>,
    /// Expression BPF du panneau latéral.
    panel_bpf: Entity<TextInput>,
    /// Interface de la modale.
    modal_interface: Entity<Select>,
    /// Filtre de la modale.
    modal_filter: Entity<TextInput>,
    /// Tableau des flux.
    table: Entity<Table<Flows>>,
    /// Cases à cocher.
    ipv4_only: bool,
    /// Case « Web ».
    web: bool,
    /// Interrupteur de gravité.
    gravity: bool,
    /// Bascule de filtre de la barre d'outils.
    filter_on: bool,
    /// Choix radio.
    mode: usize,
    /// Onglet actif.
    tab: usize,
    /// Modale ouverte.
    modal_open: bool,
    /// Confirmation ouverte.
    confirm_open: bool,
    /// Dernier événement reçu, affiché dans la barre d'état.
    last_event: SharedString,
}

impl Focusable for Gallery {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// Sections de la barre de menus de démonstration (celles de l'application).
fn menu_sections() -> Vec<MenuSection> {
    vec![
        MenuSection::new(
            "Fichier",
            vec![
                MenuEntry::item("ouvrir-pcap", "Ouvrir un PCAP ou une matrice…")
                    .accelerator("Ctrl+O"),
                MenuEntry::item("ouvrir-projet", "Ouvrir un projet (.lunar)…"),
                MenuEntry::item("enregistrer-projet", "Enregistrer le projet (.lunar)"),
                MenuEntry::Separator,
                MenuEntry::item("exporter-matrice", "Sauvegarder la matrice (CSV)")
                    .accelerator("Ctrl+S"),
                MenuEntry::item("exporter-labels", "Exporter les labels"),
                MenuEntry::item("exporter-journaux", "Exporter les journaux").accelerator("Ctrl+L"),
                MenuEntry::Separator,
                MenuEntry::item("quitter", "Quitter").accelerator("Ctrl+Q"),
            ],
        ),
        MenuSection::new(
            "Capture",
            vec![
                MenuEntry::item("demarrer", "Démarrer").accelerator("Ctrl+P"),
                MenuEntry::item("arreter", "Arrêter")
                    .accelerator("Ctrl+Maj+P")
                    .enabled(false),
                MenuEntry::item("reinitialiser", "Réinitialiser").accelerator("Ctrl+Maj+R"),
                MenuEntry::Separator,
                MenuEntry::item("configuration", "Configuration…").accelerator("Ctrl+,"),
            ],
        ),
        MenuSection::new(
            "Analyse",
            vec![
                MenuEntry::item("filtrer", "Filtrer…").accelerator("Ctrl+F"),
                MenuEntry::item("labels", "Gérer les labels…"),
                MenuEntry::item("investigation", "Investigation").accelerator("Ctrl+I"),
            ],
        ),
        MenuSection::new(
            "Aide",
            vec![MenuEntry::item("a-propos", "À propos de LUNAR")],
        ),
    ]
}

impl Gallery {
    /// Construit la galerie et ses composants à état.
    fn new(args: Args, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let menu_bar = cx.new(|cx| MenuBar::new(menu_sections(), cx));
        cx.subscribe(&menu_bar, |this, _, MenuBarEvent::Activated(id), cx| {
            this.last_event = format!("Menu : {id}").into();
            match *id {
                "configuration" => this.modal_open = true,
                "reinitialiser" => this.confirm_open = true,
                _ => {}
            }
            cx.notify();
        })
        .detach();
        let toasts = cx.new(|_| ToastStack::new());

        let name = cx.new(|cx| TextInput::new(cx).placeholder("Nom de l'équipement"));
        let address = cx.new(|cx| {
            TextInput::new(cx)
                .placeholder("Adresse IPv4")
                .mono()
                .with_text("192.168.1.300")
                .validator(|text| {
                    (!text.is_empty() && text.parse::<std::net::Ipv4Addr>().is_err())
                        .then(|| "Adresse IPv4 invalide".into())
                })
        });
        let disabled_input = cx.new(|cx| {
            TextInput::new(cx)
                .with_text("Capture en cours : champ verrouillé")
                .disabled(true)
        });
        let bpf = cx.new(|cx| {
            TextInput::multiline(3, cx)
                .placeholder("Expression BPF brute")
                .mono()
                .with_text("tcp port 502\nor (udp and port 47808)\nand not arp")
        });
        for input in [&name, &address, &bpf] {
            cx.subscribe(input, |this, _, event: &TextInputEvent, cx| {
                this.last_event = match event {
                    TextInputEvent::Changed(text) => format!("Saisie : {text}"),
                    TextInputEvent::Submitted(text) => format!("Validé : {text}"),
                }
                .into();
                cx.notify();
            })
            .detach();
        }

        let interface = cx.new(|cx| {
            Select::new(
                [
                    SelectOption::new("enp0s31f6")
                        .icon(Icon::Cpu)
                        .detail("10.0.0.12"),
                    SelectOption::new("wlp0s20f3")
                        .icon(Icon::Network)
                        .detail("192.168.1.20"),
                    SelectOption::new("docker0")
                        .icon(Icon::Network)
                        .detail("172.17.0.1")
                        .disabled(true),
                    SelectOption::new("lo").icon(Icon::Cpu).detail("127.0.0.1"),
                    SelectOption::new("any").detail("toutes"),
                ],
                cx,
            )
            .placeholder("Interface de capture")
            .selected(Some(0))
            .width(px(300.))
        });
        let protocol = cx.new(|cx| {
            Select::new(["Tous", "Modbus/TCP", "S7comm", "DNP3", "OPC UA"], cx)
                .placeholder("Protocole")
                .width(px(180.))
        });
        let disabled_select = cx.new(|cx| {
            Select::new(["eth1"], cx)
                .selected(Some(0))
                .disabled(true)
                .width(px(180.))
        });
        for select in [&interface, &protocol] {
            cx.subscribe(select, |this, select, SelectEvent::Changed(ix), cx| {
                let label = select
                    .read(cx)
                    .options()
                    .get(*ix)
                    .map(|o| o.label.clone())
                    .unwrap_or_default();
                this.last_event = format!("Choix : {label}").into();
                cx.notify();
            })
            .detach();
        }

        let panel_protocol = cx.new(|cx| {
            Select::new(["Tous", "Modbus/TCP", "S7comm"], cx)
                .selected(Some(1))
                .width(px(268.))
        });
        let panel_bpf = cx.new(|cx| {
            TextInput::multiline(2, cx)
                .placeholder("Expression BPF brute")
                .mono()
        });
        let modal_interface = cx.new(|cx| {
            Select::new(["enp0s31f6 (10.0.0.12)", "wlp0s20f3 (192.168.1.20)"], cx)
                .selected(Some(0))
                .width(px(410.))
        });
        let modal_filter = cx.new(|cx| TextInput::new(cx).mono().with_text("not arp and not ip6"));

        let editor = cx.new(|cx| TextInput::new(cx).compact().placeholder("Label"));
        let table =
            cx.new(|cx| Table::new(Flows::new(editor.clone()), cx).empty_text("Aucun flux"));
        cx.subscribe_in(
            &table,
            window,
            |this, table, event: &TableEvent, window, cx| {
                match event {
                    TableEvent::SelectionChanged(row) => {
                        this.last_event = match row {
                            Some(row) => format!("Ligne {row} sélectionnée"),
                            None => "Aucune sélection".to_string(),
                        }
                        .into();
                    }
                    TableEvent::Activated(row) => {
                        this.start_edit(table, *row, window, cx);
                    }
                    TableEvent::SortChanged(sort) => {
                        this.last_event = format!("Tri : {sort:?}").into();
                    }
                }
                cx.notify();
            },
        )
        .detach();
        cx.subscribe(&editor, |this, _, event: &TextInputEvent, cx| {
            if let TextInputEvent::Submitted(text) = event {
                let table = this.table.clone();
                table.update(cx, |table, cx| {
                    let flows = table.delegate_mut();
                    if let Some(row) = flows.editing.take()
                        && let Some(flow) = flows.rows.get_mut(row)
                    {
                        flow.label = (!text.trim().is_empty()).then(|| text.clone());
                    }
                    table.refresh(cx);
                });
                this.last_event = format!("Label enregistré : {text}").into();
                cx.notify();
            }
        })
        .detach();

        let gallery = Self {
            focus_handle: cx.focus_handle(),
            menu_bar,
            toasts,
            name,
            address,
            disabled_input,
            bpf,
            interface,
            protocol,
            disabled_select,
            panel_protocol,
            panel_bpf,
            modal_interface,
            modal_filter,
            table,
            ipv4_only: true,
            web: false,
            gravity: true,
            filter_on: true,
            mode: 0,
            tab: 1,
            modal_open: args.modal,
            confirm_open: args.confirm,
            last_event: "Prêt".into(),
        };
        gallery.apply_args(args, window, cx);
        gallery
    }

    /// Applique les états figés de la ligne de commande.
    fn apply_args(&self, args: Args, window: &mut Window, cx: &mut Context<Self>) {
        // Sans focus, les touches n'atteignent pas les écouteurs de la racine.
        window.focus(&self.focus_handle);
        if args.toasts {
            self.toasts.update(cx, |t, cx| {
                t.push_for(
                    ToastKind::Info,
                    "Import de 3 fichiers PCAP en cours.",
                    None,
                    cx,
                );
                t.push_for(ToastKind::Success, "Projet enregistré.", None, cx);
                t.push_for(
                    ToastKind::Warning,
                    "Base de vulnérabilités figée depuis 14 jours.",
                    None,
                    cx,
                );
                t.push_for(
                    ToastKind::Error,
                    "Capture impossible : Npcap réservé aux administrateurs.",
                    None,
                    cx,
                );
            });
        }
        if args.select {
            self.interface.update(cx, |s, cx| s.open(cx));
            window.focus(&self.interface.focus_handle(cx));
        }
        if args.focus {
            self.address.read(cx).focus(window);
        }
        if args.menu {
            self.menu_bar
                .update(cx, |m, cx| m.open_section(0, true, window, cx));
        }
        if args.edit {
            let table = self.table.clone();
            table.update(cx, |t, cx| t.set_selected_row(Some(3), cx));
            let editor = table.read(cx).delegate().editor.clone();
            table.update(cx, |t, cx| {
                t.delegate_mut().editing = Some(3);
                t.refresh(cx);
            });
            editor.update(cx, |e, cx| e.set_text("Automate de secours", cx));
            editor.read(cx).focus(window);
        }
    }

    /// Ouvre l'édition en place du label d'une ligne.
    fn start_edit(
        &mut self,
        table: &Entity<Table<Flows>>,
        row: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (editor, label) = {
            let flows = table.read(cx).delegate();
            (
                flows.editor.clone(),
                flows
                    .rows
                    .get(row)
                    .and_then(|f| f.label.clone())
                    .unwrap_or_default(),
            )
        };
        table.update(cx, |t, cx| {
            t.delegate_mut().editing = Some(row);
            t.refresh(cx);
        });
        editor.update(cx, |e, cx| e.set_text(&label, cx));
        editor.read(cx).focus(window);
        self.last_event = format!("Édition du label de la ligne {row}").into();
    }

    /// Échap hors modale : annule l'édition en place.
    fn on_cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        let table = self.table.clone();
        let editing = table.read(cx).delegate().editing.is_some();
        if editing {
            table.update(cx, |t, cx| {
                t.delegate_mut().editing = None;
                t.refresh(cx);
            });
            self.last_event = "Édition annulée".into();
            cx.notify();
        }
    }

    /// Ajoute une notification.
    fn toast(&self, kind: ToastKind, message: &'static str, cx: &mut Context<Self>) {
        self.toasts.update(cx, |t, cx| {
            t.push(kind, message, cx);
        });
    }

    /// Barre d'outils, calquée sur celle de l'application.
    fn render_toolbar(&self, theme: &Theme, cx: &Context<Self>) -> impl IntoElement + use<> {
        let tool = |id: &'static str, icon: Icon, text: &'static str, shortcut: &'static str| {
            let button = Button::icon_only(id, icon, text)
                .ghost()
                .size(ButtonSize::Toolbar);
            if shortcut.is_empty() {
                button
            } else {
                button.tooltip(format!("{text} ({shortcut})"))
            }
        };
        let filter_on = self.filter_on;
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(theme.space_xs)
            .h(px(40.))
            .px(px(10.))
            .bg(theme.bg_bar)
            .border_b_1()
            .border_color(theme.border_divider)
            .child(
                tool("t-demarrer", Icon::Play, "Démarrer", "Ctrl+P").on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.toast(ToastKind::Info, "Capture démarrée sur enp0s31f6.", cx)
                    },
                )),
            )
            .child(tool("t-arreter", Icon::Stop, "Arrêter", "Ctrl+Maj+P").disabled(true))
            .child(
                tool("t-reinit", Icon::Refresh, "Réinitialiser", "Ctrl+Maj+R").on_click(
                    cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.confirm_open = true;
                        cx.notify();
                    }),
                ),
            )
            .child(
                tool("t-config", Icon::Gear, "Config", "Ctrl+,").on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.modal_open = true;
                        cx.notify();
                    },
                )),
            )
            .child(separator(theme))
            .child(tool("t-sauver", Icon::FileExport, "Sauvegarder", "Ctrl+S"))
            .child(tool(
                "t-projet",
                Icon::Save,
                "Enregistrer le projet (.lunar)",
                "",
            ))
            .child(tool(
                "t-ouvrir",
                Icon::FolderOpen,
                "Ouvrir un projet (.lunar)",
                "",
            ))
            .child(tool(
                "t-export-labels",
                Icon::Tag,
                "Exporter les labels",
                "",
            ))
            .child(tool("t-labels", Icon::Tags, "Gérer les labels", ""))
            .child(tool(
                "t-pcap",
                Icon::FileImport,
                "Ouvrir un fichier Pcap ou une matrice de flux",
                "Ctrl+O",
            ))
            .child(tool(
                "t-import-labels",
                Icon::FileSpreadsheet,
                "Importer un fichier de label",
                "",
            ))
            .child(separator(theme))
            .child(tool("t-logs", Icon::Logs, "Logs", "Ctrl+L"))
            .child(
                tool("t-filtre", Icon::Filter, "Filtrer", "Ctrl+F")
                    .toggled(filter_on)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.filter_on = !this.filter_on;
                        cx.notify();
                    })),
            )
            .child(tool(
                "t-investigation",
                Icon::Microscope,
                "Investigation",
                "Ctrl+I",
            ))
            .child(div().flex_1())
            .child(tool("t-quitter", Icon::Quit, "Quitter", "Ctrl+Q"))
    }

    /// Barre d'état, calquée sur celle de l'application.
    fn render_status_bar(&self, theme: &Theme) -> impl IntoElement + use<> {
        let item = |kind: Icon, text: &'static str, title: &'static str| {
            div()
                .id(title)
                .flex()
                .items_center()
                .gap(px(4.))
                .tooltip(lunar::widgets::tooltip::tooltip(title))
                .child(icon(kind).size(px(13.)).text_color(theme.text))
                .child(div().font_family(theme.font_mono.clone()).child(text))
        };
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(10.))
            .h(px(22.))
            .px(px(10.))
            .bg(theme.bg_status_bar)
            .text_size(theme.text_sm)
            .text_color(theme.text)
            .child(item(Icon::Clock, "00:12:34", "Durée d'exécution"))
            .child(item(Icon::Inbox, "1 284 512", "Trames reçues"))
            .child(item(Icon::CircleX, "0", "Trames perdues côté kernel"))
            .child(item(
                Icon::Ban,
                "0",
                "Trames perdues au niveau de l'interface",
            ))
            .child(item(Icon::Warning, "3", "Trames perdues côté application"))
            .child(item(Icon::Puzzle, "12", "Trames illisibles par le parseur"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .h(px(16.))
                    .px(px(7.))
                    .rounded(theme.radius_sm)
                    .border_1()
                    .border_color(theme.info.opacity(0.4))
                    .bg(theme.info.opacity(0.15))
                    .child(icon(Icon::Flask).size(px(12.)).text_color(theme.text))
                    .child(
                        div()
                            .font_family(theme.font_mono.clone())
                            .text_size(theme.text_xs)
                            .child("tcp port 502"),
                    ),
            )
            .child(div().flex_1())
            .child(
                div()
                    .text_color(theme.text_muted)
                    .child(self.last_event.clone()),
            )
    }

    /// Carte « Boutons ».
    fn render_buttons(&self, theme: &Theme, cx: &Context<Self>) -> impl IntoElement + use<> {
        card(theme, "Boutons")
            .child(
                row(theme)
                    .child(Button::new("b-principal", "Appliquer").primary())
                    .child(Button::new("b-secondaire", "Réinitialiser"))
                    .child(Button::new("b-danger", "Effacer").danger())
                    .child(Button::new("b-discret", "Annuler").ghost()),
            )
            .child(
                row(theme)
                    .child(
                        Button::new("b-ouvrir", "Ouvrir")
                            .icon(Icon::FolderOpen)
                            .primary(),
                    )
                    .child(Button::new("b-exporter", "Exporter").icon(Icon::Download))
                    .child(
                        Button::new("b-supprimer", "Supprimer")
                            .icon(Icon::Trash)
                            .danger(),
                    )
                    .child(
                        Button::new("b-arbitrer", "Arbitrer (2)")
                            .icon(Icon::Scale)
                            .ghost(),
                    ),
            )
            .child(
                row(theme)
                    .child(Button::new("b-off", "Désactivé").primary().disabled(true))
                    .child(
                        Button::new("b-off2", "Désactivé")
                            .icon(Icon::Lock)
                            .disabled(true),
                    )
                    .child(
                        Button::new("b-gravite", "Gravité")
                            .icon(Icon::FocusCentered)
                            .toggled(self.gravity)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.gravity = !this.gravity;
                                cx.notify();
                            })),
                    )
                    .child(Button::new("b-petit", "Petit").size(ButtonSize::Small)),
            )
            .child(
                row(theme)
                    .child(Button::icon_only("b-i1", Icon::ZoomIn, "Zoom avant"))
                    .child(Button::icon_only("b-i2", Icon::ZoomOut, "Zoom arrière"))
                    .child(Button::icon_only("b-i3", Icon::Pencil, "Éditer").ghost())
                    .child(Button::icon_only("b-i4", Icon::Trash, "Supprimer").danger())
                    .child(Button::icon_only("b-i5", Icon::Eye, "Afficher").toggled(true))
                    .child(Button::icon_only("b-i6", Icon::Copy, "Copier").disabled(true))
                    .child(Button::icon_only("b-i7", Icon::Help, "Aide").ghost()),
            )
    }

    /// Carte « Saisie ».
    fn render_inputs(&self, theme: &Theme) -> impl IntoElement + use<> {
        card(theme, "Saisie")
            .child(labelled(theme, "Nom", self.name.clone()))
            .child(labelled(theme, "Adresse (validée)", self.address.clone()))
            .child(labelled(theme, "Désactivé", self.disabled_input.clone()))
            .child(labelled(
                theme,
                "Filtre BPF (plusieurs lignes)",
                self.bpf.clone(),
            ))
    }

    /// Carte « Listes déroulantes ».
    fn render_selects(&self, theme: &Theme) -> impl IntoElement + use<> {
        card(theme, "Listes déroulantes")
            .child(labelled(theme, "Interface", self.interface.clone()))
            .child(
                row(theme)
                    .child(labelled(theme, "Protocole", self.protocol.clone()))
                    .child(labelled(theme, "Désactivée", self.disabled_select.clone())),
            )
    }

    /// Carte « Cases et choix ».
    fn render_choices(&self, theme: &Theme, cx: &Context<Self>) -> impl IntoElement + use<> {
        card(theme, "Cases et choix")
            .child(
                row(theme)
                    .child(
                        Checkbox::new("c-ipv4", "IPv4 seulement", self.ipv4_only).on_change(
                            cx.listener(|this, checked: &bool, _, cx| {
                                this.ipv4_only = *checked;
                                cx.notify();
                            }),
                        ),
                    )
                    .child(
                        Checkbox::new("c-web", "Web (80/443)", self.web).on_change(cx.listener(
                            |this, checked: &bool, _, cx| {
                                this.web = *checked;
                                cx.notify();
                            },
                        )),
                    )
                    .child(Checkbox::new("c-off", "Désactivée", true).disabled(true)),
            )
            .child(
                RadioGroup::new(
                    "r-source",
                    ["Capture en direct", "Fichiers PCAP", "Matrice de flux"],
                    Some(self.mode),
                )
                .disable_option(2)
                .horizontal()
                .on_change(cx.listener(|this, ix: &usize, _, cx| {
                    this.mode = *ix;
                    cx.notify();
                })),
            )
            .child(
                row(theme)
                    .child(Switch::new("s-gravite", "Gravité", self.gravity).on_change(
                        cx.listener(|this, on: &bool, _, cx| {
                            this.gravity = *on;
                            cx.notify();
                        }),
                    ))
                    .child(Switch::new("s-labels", "Afficher les labels", false).disabled(true)),
            )
    }

    /// Carte « Onglets et progression ».
    fn render_tabs(&self, theme: &Theme, cx: &Context<Self>) -> impl IntoElement + use<> {
        card(theme, "Onglets et progression")
            .child(
                Tabs::new(
                    "onglets",
                    [
                        Tab::from("Résumé"),
                        Tab::with_count("Flux", 10_000),
                        Tab::with_count("Endpoints", 412),
                        Tab::from("Aide").disabled(),
                    ],
                    self.tab,
                )
                .on_change(cx.listener(|this, ix: &usize, _, cx| {
                    this.tab = *ix;
                    cx.notify();
                })),
            )
            .child(ProgressBar::determinate("p-import", 0.42).label("Import : 42 %"))
            .child(ProgressBar::indeterminate("p-lecture").label("Lecture du fichier…"))
            .child(
                ProgressBar::determinate("p-canal", 0.78)
                    .thin()
                    .label("Occupation du canal"),
            )
    }

    /// Carte « Indicateurs ».
    fn render_kpis(&self, theme: &Theme) -> impl IntoElement + use<> {
        card(theme, "Indicateurs").child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(10.))
                .child(Kpi::new("Flux", "10 000"))
                .child(Kpi::new("Endpoints", "412").detail("dont 18 nouveaux"))
                .child(Kpi::new("Protocoles", "7"))
                .child(Kpi::new("Sans label", "1 429").warn(true)),
        )
    }

    /// Carte « Notifications et dialogues ».
    fn render_feedback(&self, theme: &Theme, cx: &Context<Self>) -> impl IntoElement + use<> {
        card(theme, "Notifications et dialogues")
            .child(
                row(theme)
                    .child(
                        Button::new("n-info", "Info")
                            .icon(Icon::Info)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.toast(ToastKind::Info, "Import de 3 fichiers en cours.", cx)
                            })),
                    )
                    .child(
                        Button::new("n-ok", "Succès")
                            .icon(Icon::CircleCheck)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.toast(ToastKind::Success, "Projet enregistré.", cx)
                            })),
                    )
                    .child(
                        Button::new("n-warn", "Alerte")
                            .icon(Icon::Warning)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.toast(ToastKind::Warning, "Canal saturé à 78 %.", cx)
                            })),
                    )
                    .child(
                        Button::new("n-err", "Erreur")
                            .icon(Icon::AlertCircle)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.toast(ToastKind::Error, "Capture impossible.", cx)
                            })),
                    ),
            )
            .child(
                row(theme)
                    .child(
                        Button::new("d-modale", "Ouvrir la modale").on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                this.modal_open = true;
                                cx.notify();
                            },
                        )),
                    )
                    .child(
                        Button::new("d-confirm", "Confirmation")
                            .danger()
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.confirm_open = true;
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .id("d-bulle")
                            .text_color(theme.text_muted)
                            .text_size(theme.text_sm)
                            .tooltip(tooltip_with_shortcut("Bulle avec raccourci", "Ctrl+F"))
                            .child("Survoler pour une bulle"),
                    ),
            )
    }

    /// Carte « Icônes ».
    fn render_icons(&self, theme: &Theme) -> impl IntoElement + use<> {
        let mut grid = div().flex().flex_wrap().gap(px(6.));
        for kind in Icon::ALL {
            grid = grid.child(
                div()
                    .id(SharedString::from(format!("icone-{kind:?}")))
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(30.))
                    .rounded(theme.radius_sm)
                    .bg(theme.bg_raised)
                    .tooltip(lunar::widgets::tooltip::tooltip(format!("{kind:?}")))
                    .child(icon(*kind).size(px(18.)).text_color(theme.text)),
            );
        }
        card(
            theme,
            "Icônes (Tabler 3.50.0, Material Design Icons et LUNAR)",
        )
        .child(grid)
    }

    /// Panneau latéral « Filtres ».
    fn render_panel(&self, theme: &Theme, cx: &Context<Self>) -> impl IntoElement + use<> {
        Panel::new("panneau-filtres", "Filtres")
            .width(px(300.))
            .on_close(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.last_event = "Panneau fermé (démonstration)".into();
                cx.notify();
            }))
            .child(
                div()
                    .text_color(theme.text_muted)
                    .text_size(theme.text_sm)
                    .child("Raccourcis : chaque pastille ajoute un critère au filtre."),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(6.))
                    .child(Button::new("f-ipv4", "IPv4 seulement").size(ButtonSize::Small))
                    .child(
                        Button::new("f-web", "Web (80/443)")
                            .size(ButtonSize::Small)
                            .toggled(true),
                    )
                    .child(Button::new("f-dns", "DNS (53)").size(ButtonSize::Small))
                    .child(Button::new("f-ntp", "NTP (123)").size(ButtonSize::Small)),
            )
            .child(labelled(theme, "Protocole", self.panel_protocol.clone()))
            .child(labelled(theme, "Expression BPF", self.panel_bpf.clone()))
            .child(Checkbox::new("p-arp", "Tout sauf ARP/IPv6", false))
            .footer(Button::new("p-reinit", "Réinitialiser").ghost())
            .footer(Button::new("p-appliquer", "Appliquer").primary())
    }
}

/// Carte de section de la galerie.
fn card(theme: &Theme, title: &'static str) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(theme.space_lg)
        .p(theme.space_xl)
        .rounded(theme.radius_lg)
        .border_1()
        .border_color(theme.border)
        .bg(theme.bg_surface)
        .child(
            div()
                .text_size(theme.text_sm)
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(theme.text_heading)
                .child(title.to_uppercase()),
        )
}

/// Rangée d'éléments.
fn row(theme: &Theme) -> gpui::Div {
    div().flex().flex_wrap().items_center().gap(theme.space_md)
}

/// Élément précédé d'un libellé.
fn labelled(theme: &Theme, label: &'static str, element: impl IntoElement) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(theme.space_xs)
        .child(
            div()
                .text_size(theme.text_sm)
                .text_color(theme.text_muted)
                .child(label),
        )
        .child(element)
}

/// Séparateur vertical de barre d'outils.
fn separator(theme: &Theme) -> gpui::Div {
    div()
        .w(px(1.))
        .h(px(20.))
        .mx(px(4.))
        .bg(theme.border_divider)
}

impl Render for Gallery {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        RENDERS.fetch_add(1, AtomicOrdering::Relaxed);
        let theme = cx.theme().clone();
        let toolbar = self.render_toolbar(&theme, cx);
        let status = self.render_status_bar(&theme);
        let buttons = self.render_buttons(&theme, cx);
        let choices = self.render_choices(&theme, cx);
        let tabs = self.render_tabs(&theme, cx);
        let feedback = self.render_feedback(&theme, cx);
        let panel = self.render_panel(&theme, cx);

        let column = |width: Option<gpui::Pixels>| {
            div()
                .flex()
                .flex_col()
                .gap(theme.space_lg)
                .map(|d| match width {
                    Some(width) => d.w(width).flex_none(),
                    None => d.flex_1().min_w_0(),
                })
        };
        let content = div()
            .id("contenu")
            .flex()
            .flex_1()
            .min_w_0()
            .gap(theme.space_lg)
            .p(theme.space_lg)
            .overflow_y_scroll()
            .child(
                column(Some(px(400.)))
                    .child(buttons)
                    .child(self.render_inputs(&theme))
                    .child(choices),
            )
            .child(
                column(Some(px(440.)))
                    .child(self.render_selects(&theme))
                    .child(tabs)
                    .child(self.render_kpis(&theme))
                    .child(feedback),
            )
            .child(
                column(None)
                    .child(
                        card(&theme, "Tableau virtualisé : 10 000 flux")
                            .child(div().h(px(470.)).child(self.table.clone())),
                    )
                    .child(self.render_icons(&theme)),
            );

        let mut root = div()
            .id("galerie")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_cancel))
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.bg_app)
            .text_color(theme.text)
            .text_size(theme.text_md)
            .font_family(theme.font_family.clone())
            .child(self.menu_bar.clone())
            .child(toolbar)
            .child(div().flex().flex_1().min_h_0().child(content).child(panel))
            .child(status)
            .child(self.toasts.clone());

        if self.modal_open {
            root = root.child(
                Modal::new("modale-config", "Configuration de la capture")
                    .width(px(460.))
                    .on_close(cx.listener(|this, _: &(), _, cx| {
                        this.modal_open = false;
                        this.last_event = "Modale fermée".into();
                        cx.notify();
                    }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(theme.space_md)
                            .child(img(LOGO_PNG).size(px(40.)))
                            .child(
                                div()
                                    .text_color(theme.text_muted)
                                    .child("Réglages appliqués à la prochaine capture."),
                            ),
                    )
                    .child(labelled(&theme, "Interface", self.modal_interface.clone()))
                    .child(labelled(&theme, "Filtre BPF", self.modal_filter.clone()))
                    .child(Checkbox::new("m-promisc", "Mode promiscuité", true))
                    .footer(Button::new("m-fermer", "Fermer").on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            this.modal_open = false;
                            cx.notify();
                        },
                    )))
                    .footer(
                        Button::new("m-enregistrer", "Sauvegarder")
                            .primary()
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.modal_open = false;
                                this.toast(ToastKind::Success, "Configuration enregistrée.", cx);
                                cx.notify();
                            })),
                    ),
            );
        }
        if self.confirm_open {
            root = root.child(
                Confirm::new(
                    "confirmation-reinit",
                    "Réinitialiser la capture ?",
                    "Les flux et labels non enregistrés seront perdus. Cette action est \
                     irréversible.",
                )
                .confirm_label("Réinitialiser")
                .danger()
                .on_confirm(cx.listener(|this, _: &(), _, cx| {
                    this.confirm_open = false;
                    this.toast(ToastKind::Info, "Capture réinitialisée.", cx);
                    cx.notify();
                }))
                .on_cancel(cx.listener(|this, _: &(), _, cx| {
                    this.confirm_open = false;
                    this.last_event = "Réinitialisation annulée".into();
                    cx.notify();
                })),
            );
        }
        root_actions(root, Some(&self.menu_bar))
    }
}

fn main() {
    env_logger::init();
    force_x11_backend();
    let args = Args::parse();
    Application::new()
        .with_assets(Assets)
        .run(move |cx: &mut App| {
            widgets::init(cx);
            let bounds = Bounds::centered(None, size(px(1920.), px(1080.)), cx);
            let opened = cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions {
                        title: Some("LUNAR — galerie des composants".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |window, cx| cx.new(|cx| Gallery::new(args, window, cx)),
            );
            if let Err(err) = opened {
                eprintln!("ouverture de la fenêtre impossible : {err:#}");
                cx.quit();
                return;
            }
            install_x11_wakeup(cx, &RENDERS);
            cx.activate(true);
        });
}
