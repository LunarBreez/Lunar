//! Logique de la vue d'investigation, sans rendu : état de la vue (onglet, tri, recherche, filtres, colonnes),
//! valeurs dérivées (indicateurs, onglets, lignes visibles, glossaire) et
//! textes affichés. L'interface GPUI ne fait qu'afficher ce que
//! [`InvestigationModel::derive`] calcule et appeler les méthodes de
//! [`ViewState`] sur les actions de l'utilisateur.
//!
//! Lecture approfondie de la matrice de flux courante, hors du flux
//! d'événements de capture : le snapshot du graphe (`get_graph_snapshot`)
//! est l'état de référence ; tout ce qui est affiché en dérive.

use std::cmp::Ordering;
use std::collections::HashSet;

use lunar_engine::APP_NAME;
use lunar_flows_core::graph::GraphData;

use super::dates::{LocalZone, format_short_medium};
use super::glossary::{GLOSSARY, GlossaryEntry, match_glossary, normalize_token};
use super::graph_view::GraphView;
use super::locale::{compare_fr, format_integer, js_number_to_string, js_trim};
use super::period::{PeriodSummary, SurveyPeriodView, duration_value, summarize_period};
use super::protocol_colors::color_for_protocol;
use super::tables::{
    DEFAULT_TOP_N, DeductionRow, EndpointRow, FlowRow, InvestigationStats, InvestigationSummary,
    ProtocolCount, Totals, build_deduction_rows, build_endpoint_rows, build_flow_rows,
    build_summary, compute_stats, filter_graph_by_protocols, format_bytes,
    observed_glossary_tokens, percent_of, protocol_breakdown,
};

/// Onglet de la vue.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Tab {
    /// « Résumé ».
    #[default]
    Summary,
    /// « Flux (n) ».
    Flows,
    /// « Endpoints (n) ».
    Endpoints,
    /// « Déduction (n) ».
    Deduction,
    /// « Glossaire (n) » (clé interne d'onglet : `wiki`).
    Wiki,
}

/// Les onglets, dans l'ordre de la barre.
pub const TABS: [Tab; 5] = [
    Tab::Summary,
    Tab::Flows,
    Tab::Endpoints,
    Tab::Deduction,
    Tab::Wiki,
];

/// Colonne triable : la clé pilote à la fois l'accès à la valeur et le sens
/// de comparaison (numérique ou texte) — partagée par les trois tableaux,
/// chacun n'utilisant que ses propres clés.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SortKey {
    /// Flux : source.
    Source,
    /// Flux : destination.
    Target,
    /// Flux : protocole.
    Protocol,
    /// Flux : ports.
    Ports,
    /// Flux : sens.
    Bidir,
    /// Endpoints : nom.
    Name,
    /// Endpoints : IP.
    Ip,
    /// Endpoints : MAC (triée sur la MAC brute).
    Mac,
    /// Endpoints : VLAN.
    Vlan,
    /// Endpoints : label.
    Label,
    /// Endpoints : flux incidents.
    Degree,
    /// Endpoints : flux sortants.
    FlowsOut,
    /// Endpoints : flux entrants.
    FlowsIn,
    /// Déductions : type.
    Kind,
    /// Déductions : élément observé.
    Observed,
    /// Déductions : indication.
    Deduction,
    /// Déductions : occurrences.
    Count,
    /// Flux et endpoints : paquets.
    Packets,
    /// Flux et endpoints : volume.
    Bytes,
}

impl SortKey {
    /// Toutes les clés, dans l'ordre de déclaration.
    pub const ALL: [Self; 19] = [
        Self::Source,
        Self::Target,
        Self::Protocol,
        Self::Ports,
        Self::Bidir,
        Self::Name,
        Self::Ip,
        Self::Mac,
        Self::Vlan,
        Self::Label,
        Self::Degree,
        Self::FlowsOut,
        Self::FlowsIn,
        Self::Kind,
        Self::Observed,
        Self::Deduction,
        Self::Count,
        Self::Packets,
        Self::Bytes,
    ];

    /// Nom de la clé (`'flowsOut'`…), qui est aussi le nom du champ de ligne
    /// trié.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Target => "target",
            Self::Protocol => "protocol",
            Self::Ports => "ports",
            Self::Bidir => "bidir",
            Self::Name => "name",
            Self::Ip => "ip",
            Self::Mac => "mac",
            Self::Vlan => "vlan",
            Self::Label => "label",
            Self::Degree => "degree",
            Self::FlowsOut => "flowsOut",
            Self::FlowsIn => "flowsIn",
            Self::Kind => "kind",
            Self::Observed => "observed",
            Self::Deduction => "deduction",
            Self::Count => "count",
            Self::Packets => "packets",
            Self::Bytes => "bytes",
        }
    }

    /// Clé de nom `name` (cf. [`SortKey::name`]).
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|key| key.name() == name)
    }

    /// Vrai pour les mesures, triées par défaut en ordre décroissant (« les
    /// plus gros flux d'abord ») ; le texte se trie de A à Z.
    pub const fn is_measure(self) -> bool {
        matches!(
            self,
            Self::Packets
                | Self::Bytes
                | Self::Degree
                | Self::FlowsOut
                | Self::FlowsIn
                | Self::Count
        )
    }
}

/// Colonne d'un tableau : sa clé sert au tri, à l'en-tête et au filtre
/// d'affichage (cases à cocher « Colonnes »).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColumnDef {
    /// Clé de tri et de visibilité.
    pub key: SortKey,
    /// En-tête affiché.
    pub label: &'static str,
}

/// Raccourci de construction des tables de colonnes.
const fn column(key: SortKey, label: &'static str) -> ColumnDef {
    ColumnDef { key, label }
}

/// Colonnes du tableau des flux.
pub const FLOW_COLUMNS: [ColumnDef; 7] = [
    column(SortKey::Source, "Source"),
    column(SortKey::Target, "Destination"),
    column(SortKey::Protocol, "Protocole"),
    column(SortKey::Ports, "Ports"),
    column(SortKey::Bidir, "Sens"),
    column(SortKey::Packets, "Paquets"),
    column(SortKey::Bytes, "Volume"),
];

/// Colonnes du tableau des endpoints.
pub const ENDPOINT_COLUMNS: [ColumnDef; 10] = [
    column(SortKey::Name, "Endpoint"),
    column(SortKey::Ip, "IP"),
    column(SortKey::Mac, "MAC"),
    column(SortKey::Vlan, "VLAN"),
    column(SortKey::Label, "Label"),
    column(SortKey::Degree, "Flux"),
    column(SortKey::FlowsOut, "Sortants"),
    column(SortKey::FlowsIn, "Entrants"),
    column(SortKey::Packets, "Paquets"),
    column(SortKey::Bytes, "Volume"),
];

/// Colonnes du tableau des déductions.
pub const DEDUCTION_COLUMNS: [ColumnDef; 4] = [
    column(SortKey::Kind, "Type"),
    column(SortKey::Observed, "Observé"),
    column(SortKey::Deduction, "Déduction (probable)"),
    column(SortKey::Count, "Occurrences"),
];

/// Infobulle d'un en-tête de colonne, quand il en a une.
pub const fn column_title(key: SortKey) -> Option<&'static str> {
    match key {
        SortKey::FlowsOut => {
            Some("Flux où l'endpoint émet (les flux bidirectionnels comptent dans les deux sens)")
        }
        SortKey::FlowsIn => {
            Some("Flux où l'endpoint reçoit (les flux bidirectionnels comptent dans les deux sens)")
        }
        _ => None,
    }
}

/// État de tri annoncé aux lecteurs d'écran (`aria-sort`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AriaSort {
    /// Colonne triée en ordre croissant.
    Ascending,
    /// Colonne triée en ordre décroissant.
    Descending,
    /// Colonne non triée.
    None,
}

/// Message d'échec du chargement du relevé.
pub const LOAD_ERROR: &str = "Impossible de charger le relevé courant.";
/// Message quand une importation ou une sauvegarde occupe déjà l'application.
pub const BUSY_ERROR: &str =
    "Une opération d'importation ou de sauvegarde est déjà en cours. Veuillez patienter.";
/// Message d'échec de l'export des labels à compléter.
pub const LABELS_EXPORT_ERROR: &str = "L'export des labels à compléter a échoué.";
/// Message d'échec de l'export de la synthèse.
pub const SYNTHESIS_EXPORT_ERROR: &str = "L'export de la synthèse a échoué.";
/// Message d'échec de l'enregistrement d'un label.
pub const SAVE_LABEL_ERROR: &str = "Impossible d'enregistrer le label.";
/// Texte affiché pendant le chargement.
pub const LOADING_TEXT: &str = "Chargement du relevé…";
/// Texte d'un relevé vide (suivi de [`EMPTY_SURVEY_HINT`] à la ligne).
pub const EMPTY_SURVEY_TEXT: &str = "Aucun flux dans le relevé courant. Lancez une capture ou importez une matrice, puis revenez ici.";
/// Seconde ligne du relevé vide (« Glossaire » en gras).
pub const EMPTY_SURVEY_HINT: &str = "(Le Glossaire reste consultable hors ligne.)";
/// Ligne vide du tableau des déductions.
pub const NO_DEDUCTION_TEXT: &str =
    "Rien à déduire pour l'instant (aucun port/OUI/adresse notoire reconnu dans ce relevé).";
/// Texte indicatif de l'onglet Glossaire.
pub const WIKI_HINT: &str =
    "Définitions embarquées — aucune connexion requise. Recherche via la barre du haut.";
/// Indication du champ de recherche.
pub const SEARCH_PLACEHOLDER: &str = "Rechercher (IP, MAC, label, protocole…)";
/// Indication du champ de label d'un endpoint.
pub const LABEL_PLACEHOLDER: &str = "Ajouter un label…";
/// Infobulle de l'indicateur d'IP dupliquée d'un endpoint.
pub const DUPLICATE_IP_TITLE: &str = "IP présente sur un autre VLAN";

/// État de la vue : ce que l'utilisateur a choisi et l'avancement des
/// opérations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewState {
    /// Relevé en cours de chargement (vrai à l'ouverture).
    pub loading: bool,
    /// Dernière erreur à afficher ; vide si aucune.
    pub error: String,
    /// Compte rendu du dernier export réussi ; vide si aucun.
    pub notice: String,
    /// Recherche plein texte.
    pub query: String,
    /// Onglet affiché.
    pub tab: Tab,
    /// Colonne de tri, partagée par les tableaux.
    pub sort_key: SortKey,
    /// Sens du tri.
    pub sort_asc: bool,
    /// Popover du filtre ouvert.
    pub filter_open: bool,
    /// Colonnes masquées (filtre d'affichage « Colonnes ») : partagé par les
    /// tableaux, les clés communes (paquets, volume) se masquent des deux.
    pub hidden_columns: Vec<SortKey>,
    /// Protocoles décochés dans le filtre. Vide = tout affiché ; on stocke ce
    /// qui est masqué (et non ce qui est visible) pour qu'un protocole apparu
    /// après un rafraîchissement soit visible par défaut.
    pub hidden_protocols: Vec<String>,
    /// Id de l'endpoint dont le label est en cours d'enregistrement (son champ
    /// est désactivé le temps de l'aller-retour).
    pub saving_label: Option<String>,
    /// Glossaire : tout le glossaire plutôt que les seuls termes présents.
    pub wiki_show_all: bool,
    /// Export en cours (boutons désactivés, Ctrl+S ignoré).
    pub exporting: bool,
    /// Tableau des endpoints restreint à ceux sans label.
    pub only_unlabeled: bool,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            loading: true,
            error: String::new(),
            notice: String::new(),
            query: String::new(),
            tab: Tab::Summary,
            sort_key: SortKey::Bytes,
            sort_asc: false,
            filter_open: false,
            hidden_columns: Vec::new(),
            hidden_protocols: Vec::new(),
            saving_label: None,
            wiki_show_all: false,
            exporting: false,
            only_unlabeled: false,
        }
    }
}

impl ViewState {
    /// Clic sur un en-tête : inverse le sens sur la colonne courante, sinon
    /// trie sur `key`, décroissant pour les mesures, croissant pour le texte.
    pub fn sort_by(&mut self, key: SortKey) {
        if self.sort_key == key {
            self.sort_asc = !self.sort_asc;
        } else {
            self.sort_key = key;
            self.sort_asc = !key.is_measure();
        }
    }

    /// État de tri d'une colonne pour les lecteurs d'écran.
    pub fn aria_sort(&self, key: SortKey) -> AriaSort {
        match (self.sort_key == key, self.sort_asc) {
            (false, _) => AriaSort::None,
            (true, true) => AriaSort::Ascending,
            (true, false) => AriaSort::Descending,
        }
    }

    /// Indicateur décoratif du tri : « ▲ », « ▼ » ou rien.
    pub fn sort_indicator(&self, key: SortKey) -> &'static str {
        match self.aria_sort(key) {
            AriaSort::Ascending => "▲",
            AriaSort::Descending => "▼",
            AriaSort::None => "",
        }
    }

    /// Vrai si le protocole n'est pas décoché.
    pub fn is_protocol_visible(&self, protocol: &str) -> bool {
        !self
            .hidden_protocols
            .iter()
            .any(|hidden| hidden == protocol)
    }

    /// Coche ou décoche un protocole du filtre.
    pub fn toggle_protocol(&mut self, protocol: &str) {
        if self.is_protocol_visible(protocol) {
            self.hidden_protocols.push(protocol.to_string());
        } else {
            self.hidden_protocols.retain(|hidden| hidden != protocol);
        }
    }

    /// « Tout » (`visible`) ou « Aucun » : coche ou décoche tous les
    /// protocoles proposés.
    pub fn set_all_protocols(&mut self, visible: bool, options: &[ProtocolCount]) {
        self.hidden_protocols = if visible {
            Vec::new()
        } else {
            options
                .iter()
                .map(|option| option.protocol.clone())
                .collect()
        };
    }

    /// Vrai si aucun protocole n'est décoché.
    pub fn all_protocols_visible(&self) -> bool {
        self.hidden_protocols.is_empty()
    }

    /// Vrai si la colonne n'est pas masquée.
    pub fn is_column_visible(&self, key: SortKey) -> bool {
        !self.hidden_columns.contains(&key)
    }

    /// Masque ou réaffiche une colonne.
    pub fn toggle_column(&mut self, key: SortKey) {
        if self.is_column_visible(key) {
            self.hidden_columns.push(key);
        } else {
            self.hidden_columns.retain(|&hidden| hidden != key);
        }
    }

    /// Colonnes du tableau courant, pour la section « Colonnes » du filtre
    /// (aucune sur le résumé et le glossaire, qui ne sont pas des tableaux).
    pub fn column_options(&self) -> &'static [ColumnDef] {
        match self.tab {
            Tab::Flows => &FLOW_COLUMNS,
            Tab::Endpoints => &ENDPOINT_COLUMNS,
            Tab::Deduction => &DEDUCTION_COLUMNS,
            Tab::Summary | Tab::Wiki => &[],
        }
    }

    /// « étiqueter → » du résumé : endpoints sans label, les plus actifs
    /// d'abord.
    pub fn show_unlabeled(&mut self) {
        self.only_unlabeled = true;
        self.sort_key = SortKey::Packets;
        self.sort_asc = false;
        self.tab = Tab::Endpoints;
    }

    /// Libellé du bouton de filtre : « ⚙ Filtre », suivi de « (visibles/total) »
    /// quand des protocoles sont décochés.
    pub fn filter_button_label(&self, options: &[ProtocolCount]) -> String {
        if self.all_protocols_visible() {
            return "⚙ Filtre".to_string();
        }
        // Différence signée comme en JavaScript : des protocoles masqués
        // peuvent avoir disparu au rafraîchissement.
        let visible = i128::try_from(options.len()).unwrap_or(0)
            - i128::try_from(self.hidden_protocols.len()).unwrap_or(0);
        format!("⚙ Filtre ({visible}/{})", options.len())
    }

    /// Début d'un rechargement du relevé.
    pub fn begin_reload(&mut self) {
        self.loading = true;
        self.error.clear();
    }

    /// Fin d'un rechargement : `ok` faux affiche [`LOAD_ERROR`].
    pub fn finish_reload(&mut self, ok: bool) {
        if !ok {
            self.error = LOAD_ERROR.to_string();
        }
        self.loading = false;
    }

    /// Garde d'un export (synthèse ou labels à compléter) : refusé pendant un
    /// chargement ou un autre export ; refusé avec [`BUSY_ERROR`] si une
    /// importation ou une sauvegarde occupe l'application (`importing`).
    /// Accepté : l'export commence, erreur et compte rendu sont effacés.
    pub fn begin_export(&mut self, importing: bool) -> bool {
        if self.exporting || self.loading {
            return false;
        }
        if importing {
            self.error = BUSY_ERROR.to_string();
            return false;
        }
        self.exporting = true;
        self.error.clear();
        self.notice.clear();
        true
    }

    /// Fin d'un export : compte rendu (succès), message d'erreur (échec), ou
    /// rien (annulation dans le sélecteur de fichiers).
    pub fn finish_export(&mut self, outcome: ExportOutcome) {
        match outcome {
            ExportOutcome::Done(notice) => self.notice = notice,
            ExportOutcome::Failed(message) => self.error = message.to_string(),
            ExportOutcome::Cancelled => {}
        }
        self.exporting = false;
    }
}

/// Issue d'un export.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExportOutcome {
    /// Export réussi : compte rendu à afficher.
    Done(String),
    /// Export échoué : message à afficher ([`SYNTHESIS_EXPORT_ERROR`]…).
    Failed(&'static str),
    /// Sélecteur de fichiers fermé sans choix.
    Cancelled,
}

/// Valeur d'une cellule pour le tri. Le tri est
/// numérique si les deux valeurs sont des nombres, sinon il compare
/// `String(valeur ?? "")` en ordre français (booléen : « true »/« false » ;
/// absente ou `null` : chaîne vide).
#[derive(Clone, Debug, PartialEq)]
pub enum SortValue<'a> {
    /// Nombre (paquets, volume, degré…).
    Number(f64),
    /// Texte, ou ce que `String()` fait d'un booléen ou de `null`.
    Text(&'a str),
}

/// Conversion d'un compteur en nombre JavaScript.
#[allow(clippy::cast_precision_loss)]
fn number(value: impl TryInto<u64>) -> SortValue<'static> {
    SortValue::Number(value.try_into().map_or(f64::INFINITY, |value| value as f64))
}

/// `String(booléen)`.
const fn bool_text(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

/// Ligne d'un tableau de la vue : valeur de tri par colonne et valeurs
/// textuelles pour la recherche.
pub trait InvestigationRow {
    /// Valeur de la ligne pour la colonne `key` (texte vide si la ligne n'a
    /// pas cette colonne : la clé de tri est partagée par les tableaux).
    fn sort_value(&self, key: SortKey) -> SortValue<'_>;

    /// `Object.values(ligne).map(String)` : tous les champs de la ligne, dans
    /// l'ordre de déclaration, tels que `String()` les écrit (nombres bruts,
    /// « true »/« false », `null` → « null », tableau → valeurs séparées par
    /// des virgules).
    fn search_values(&self) -> Vec<String>;
}

impl InvestigationRow for FlowRow {
    fn sort_value(&self, key: SortKey) -> SortValue<'_> {
        match key {
            SortKey::Source => SortValue::Text(&self.source),
            SortKey::Target => SortValue::Text(&self.target),
            SortKey::Protocol => SortValue::Text(&self.protocol),
            SortKey::Ports => SortValue::Text(&self.ports),
            SortKey::Bidir => SortValue::Text(bool_text(self.bidir)),
            SortKey::Packets => number(self.packets),
            SortKey::Bytes => number(self.bytes),
            _ => SortValue::Text(""),
        }
    }

    fn search_values(&self) -> Vec<String> {
        vec![
            self.id.clone(),
            self.source_id.clone(),
            self.target_id.clone(),
            self.source.clone(),
            self.target.clone(),
            self.protocol.clone(),
            self.ports.clone(),
            bool_text(self.bidir).to_string(),
            self.packets.to_string(),
            self.bytes.to_string(),
        ]
    }
}

impl InvestigationRow for EndpointRow {
    fn sort_value(&self, key: SortKey) -> SortValue<'_> {
        match key {
            SortKey::Name => SortValue::Text(&self.name),
            SortKey::Ip => SortValue::Text(&self.ip),
            SortKey::Mac => SortValue::Text(&self.mac),
            SortKey::Vlan => self.vlan.map_or(SortValue::Text(""), number),
            SortKey::Label => SortValue::Text(&self.label),
            SortKey::Degree => number(self.degree),
            SortKey::FlowsOut => number(self.flows_out),
            SortKey::FlowsIn => number(self.flows_in),
            SortKey::Packets => number(self.packets),
            SortKey::Bytes => number(self.bytes),
            _ => SortValue::Text(""),
        }
    }

    fn search_values(&self) -> Vec<String> {
        vec![
            self.id.clone(),
            self.name.clone(),
            self.ip.clone(),
            self.mac.clone(),
            self.mac_display.clone(),
            self.macs.join(","),
            self.vlan
                .map_or_else(|| "null".to_string(), |vlan| vlan.to_string()),
            self.label.clone(),
            bool_text(self.duplicate_ip).to_string(),
            self.degree.to_string(),
            self.flows_out.to_string(),
            self.flows_in.to_string(),
            self.packets.to_string(),
            self.bytes.to_string(),
        ]
    }
}

impl InvestigationRow for DeductionRow {
    fn sort_value(&self, key: SortKey) -> SortValue<'_> {
        match key {
            SortKey::Kind => SortValue::Text(self.kind.label()),
            SortKey::Observed => SortValue::Text(&self.observed),
            SortKey::Deduction => SortValue::Text(self.deduction),
            SortKey::Count => number(self.count),
            _ => SortValue::Text(""),
        }
    }

    fn search_values(&self) -> Vec<String> {
        vec![
            self.kind.label().to_string(),
            self.observed.clone(),
            self.deduction.to_string(),
            self.count.to_string(),
        ]
    }
}

/// Recherche plein texte insensible à la casse sur toutes les valeurs de la
/// ligne (endpoints, protocole, ports, label…) ; une recherche vide garde
/// tout.
pub fn filter_rows<T: InvestigationRow + Clone>(rows: &[T], query: &str) -> Vec<T> {
    let query = js_trim(query).to_lowercase();
    if query.is_empty() {
        return rows.to_vec();
    }
    rows.iter()
        .filter(|row| {
            row.search_values()
                .iter()
                .any(|value| value.to_lowercase().contains(&query))
        })
        .cloned()
        .collect()
}

/// Texte de tri d'une valeur (`String(valeur ?? "")`).
fn sort_text(value: &SortValue<'_>) -> String {
    match value {
        SortValue::Number(number) => js_number_to_string(*number),
        SortValue::Text(text) => (*text).to_string(),
    }
}

/// Comparaison de deux valeurs de tri, en ordre croissant.
fn compare_values(a: &SortValue<'_>, b: &SortValue<'_>) -> Ordering {
    match (a, b) {
        (SortValue::Number(a), SortValue::Number(b)) => a.partial_cmp(b).unwrap_or(Ordering::Equal),
        _ => compare_fr(&sort_text(a), &sort_text(b)),
    }
}

/// Tri stable des lignes sur `key`, croissant si `ascending`.
pub fn sort_rows<T: InvestigationRow>(mut rows: Vec<T>, key: SortKey, ascending: bool) -> Vec<T> {
    rows.sort_by(|a, b| {
        let order = compare_values(&a.sort_value(key), &b.sort_value(key));
        if ascending { order } else { order.reverse() }
    });
    rows
}

/// Part d'un total pour une barre, en % (plancher de 2 % pour qu'une toute
/// petite valeur reste visible ; 0 si la valeur ou le maximum est nul).
#[allow(clippy::cast_precision_loss)]
pub fn bar_width_percent(value: u64, max: u64) -> f64 {
    if max == 0 || value == 0 {
        return 0.0;
    }
    (value as f64 / max as f64 * 100.0).max(2.0)
}

/// Largeur CSS d'une barre : « 0% », « 2% »,
/// « 33.333333333333336% »…
pub fn bar_width(value: u64, max: u64) -> String {
    format!("{}%", js_number_to_string(bar_width_percent(value, max)))
}

/// Sens d'un flux : « ↔ » s'il est bidirectionnel, « → » sinon.
pub const fn arrow(bidir: bool) -> &'static str {
    if bidir { "↔" } else { "→" }
}

/// Valeur affichée, « — » si vide.
fn or_dash(value: &str) -> &str {
    if value.is_empty() { "—" } else { value }
}

/// Texte d'une cellule du tableau des flux.
pub fn flow_cell(row: &FlowRow, key: SortKey) -> String {
    match key {
        SortKey::Source => row.source.clone(),
        SortKey::Target => row.target.clone(),
        SortKey::Protocol => row.protocol.clone(),
        SortKey::Ports => row.ports.clone(),
        SortKey::Bidir => arrow(row.bidir).to_string(),
        SortKey::Packets => format_integer(row.packets),
        SortKey::Bytes => format_bytes(row.bytes),
        _ => String::new(),
    }
}

/// Infobulle d'une cellule du tableau des flux (id du nœud).
pub fn flow_cell_title(row: &FlowRow, key: SortKey) -> Option<&str> {
    match key {
        SortKey::Source => Some(&row.source_id),
        SortKey::Target => Some(&row.target_id),
        _ => None,
    }
}

/// Texte d'une cellule du tableau des endpoints. La colonne Label est un
/// champ de saisie : sa valeur est le label posé (vide : [`LABEL_PLACEHOLDER`]).
pub fn endpoint_cell(row: &EndpointRow, key: SortKey) -> String {
    match key {
        SortKey::Name => row.name.clone(),
        SortKey::Ip => format!(
            "{}{}",
            or_dash(&row.ip),
            if row.duplicate_ip { " ⚠" } else { "" }
        ),
        SortKey::Mac => {
            let extra = if row.macs.len() > 1 {
                format!(" (+{})", row.macs.len() - 1)
            } else {
                String::new()
            };
            format!("{}{extra}", or_dash(&row.mac_display))
        }
        SortKey::Vlan => row
            .vlan
            .map_or_else(|| "—".to_string(), |vlan| vlan.to_string()),
        SortKey::Label => row.label.clone(),
        SortKey::Degree => row.degree.to_string(),
        SortKey::FlowsOut => row.flows_out.to_string(),
        SortKey::FlowsIn => row.flows_in.to_string(),
        SortKey::Packets => format_integer(row.packets),
        SortKey::Bytes => format_bytes(row.bytes),
        _ => String::new(),
    }
}

/// Infobulle d'une cellule du tableau des endpoints : id du nœud, IP
/// dupliquée, liste des MAC quand il y en a plusieurs.
pub fn endpoint_cell_title(row: &EndpointRow, key: SortKey) -> Option<String> {
    match key {
        SortKey::Name => Some(row.id.clone()),
        SortKey::Ip if row.duplicate_ip => Some(DUPLICATE_IP_TITLE.to_string()),
        SortKey::Mac if row.macs.len() > 1 => Some(row.macs.join(", ")),
        _ => None,
    }
}

/// Libellé accessible du champ de label d'un endpoint.
pub fn label_input_aria(row: &EndpointRow) -> String {
    format!("Label de {}", row.name)
}

/// Texte d'une cellule du tableau des déductions.
pub fn deduction_cell(row: &DeductionRow, key: SortKey) -> String {
    match key {
        SortKey::Kind => row.kind.label().to_string(),
        SortKey::Observed => row.observed.clone(),
        SortKey::Deduction => row.deduction.to_string(),
        SortKey::Count => row.count.to_string(),
        _ => String::new(),
    }
}

/// Commande de label à envoyer au backend après une saisie.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LabelCommand {
    /// `add_label` : pose le label sur `(mac, ip)`.
    Add {
        /// MAC brute de l'endpoint.
        mac: String,
        /// IP brute de l'endpoint.
        ip: String,
        /// Label saisi, sans blancs autour.
        label: String,
    },
    /// `delete_label` : retire le label de `(mac, ip)` (champ vidé).
    Delete {
        /// MAC brute de l'endpoint.
        mac: String,
        /// IP brute de l'endpoint.
        ip: String,
    },
}

/// Pose (ou retire, si vidé) le label d'un endpoint depuis la liste : rien à
/// faire si la saisie, sans blancs autour, est le label en place.
pub fn label_command(row: &EndpointRow, value: &str) -> Option<LabelCommand> {
    let label = js_trim(value);
    if label == row.label {
        return None;
    }
    let (mac, ip) = (row.mac.clone(), row.ip.clone());
    Some(if label.is_empty() {
        LabelCommand::Delete { mac, ip }
    } else {
        LabelCommand::Add {
            mac,
            ip,
            label: label.to_string(),
        }
    })
}

/// Titre du sélecteur de l'export de synthèse.
pub const SYNTHESIS_DIALOG_TITLE: &str = "Exporter la synthèse du relevé";
/// Filtre du sélecteur de l'export de synthèse : nom et extensions.
pub const SYNTHESIS_DIALOG_FILTER: (&str, &[&str]) = ("Page HTML", &["html"]);
/// Titre du sélecteur de l'export des labels à compléter.
pub const LABELS_DIALOG_TITLE: &str = "Exporter les labels à compléter";
/// Filtre du sélecteur de l'export des labels à compléter.
pub const LABELS_DIALOG_FILTER: (&str, &[&str]) = ("Labels CSV", &["csv"]);

/// Nom proposé pour la page de synthèse (`date` : `AAAAMMJJ`, cf.
/// [`super::dates::current_date`]).
pub fn synthesis_default_file_name(date: &str) -> String {
    format!("{date}_DR_Synthese.html")
}

/// Nom proposé pour le fichier de labels à compléter.
pub fn labels_default_file_name(date: &str) -> String {
    format!("{date}_DR_labels_a_completer.csv")
}

/// Dernier segment d'un chemin, séparateurs `/` et `\` confondus.
pub fn file_name_of(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// Compte rendu d'un export de synthèse réussi.
pub fn synthesis_notice(html_path: &str, csv_file_name: &str, csv_sha256: &str) -> String {
    let sha: String = csv_sha256.chars().take(16).collect();
    format!(
        "Synthèse exportée : {} et {csv_file_name} (SHA-256 {sha}…).",
        file_name_of(html_path)
    )
}

/// Compte rendu d'un export de labels à compléter réussi.
pub fn labels_notice(count: u64, path: &str) -> String {
    format!(
        "{count} équipement(s) sans label exporté(s) dans {} : complétez la colonne label, puis réimportez le fichier (Import de labels).",
        file_name_of(path)
    )
}

/// Indicateur du bandeau.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Kpi {
    /// Libellé (« Endpoints », « Flux »…).
    pub label: &'static str,
    /// Valeur affichée.
    pub value: String,
    /// Mise en évidence (IP dupliquées).
    pub warn: bool,
    /// Infobulle (« Durée du relevé »).
    pub title: Option<String>,
}

/// Ce que montre le corps de la vue sous les onglets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyState {
    /// Chargement ([`LOADING_TEXT`]).
    Loading,
    /// Relevé vide hors glossaire ([`EMPTY_SURVEY_TEXT`]).
    EmptySurvey,
    /// Contenu de l'onglet courant.
    Tab(Tab),
}

/// Ligne d'une carte du résumé.
#[derive(Clone, Debug, PartialEq)]
pub struct SummaryLine {
    /// Nom affiché.
    pub name: String,
    /// Infobulle du nom.
    pub name_title: Option<String>,
    /// Pastille de couleur du protocole (avant le nom pour la répartition par
    /// protocole, après pour les flux).
    pub swatch: Option<&'static str>,
    /// Barre : largeur en % ([`bar_width_percent`]).
    pub bar_percent: f64,
    /// Couleur de la barre ; `None` : couleur par défaut.
    pub bar_color: Option<&'static str>,
    /// Valeur (volume ou part).
    pub value: String,
    /// Précision (« 3 flux », « 1 234 paq. »).
    pub meta: String,
}

/// Carte du résumé.
#[derive(Clone, Debug, PartialEq)]
pub struct SummaryCard {
    /// Titre.
    pub title: &'static str,
    /// Sous-titre.
    pub subtitle: &'static str,
    /// Lignes.
    pub lines: Vec<SummaryLine>,
    /// Texte à afficher quand la carte n'a aucune ligne (« — »), si elle en
    /// prévoit un.
    pub empty_text: Option<&'static str>,
}

/// Indicateurs d'alerte du bas du résumé.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SummaryFlags {
    /// « n IP dupliquée(s) ».
    pub duplicate_ips: usize,
    /// « n endpoint(s) à étiqueter » ; bouton « étiqueter → » si non nul.
    pub unlabeled: usize,
}

/// Valeurs dérivées de l'état et du relevé.
#[derive(Clone, Debug)]
pub struct InvestigationDerived {
    /// Protocoles du relevé complet, avec leur nombre de flux : stable quelle
    /// que soit la sélection, elle peuple les cases du filtre.
    pub protocol_options: Vec<ProtocolCount>,
    /// Libellé du bouton de filtre.
    pub filter_label: String,
    /// Indicateurs du relevé filtré.
    pub stats: InvestigationStats,
    /// Synthèse du relevé filtré.
    pub summary: InvestigationSummary,
    /// Bandeau d'indicateurs.
    pub kpis: Vec<Kpi>,
    /// Onglets et leurs libellés (« Flux (12) »…).
    pub tabs: Vec<(Tab, String)>,
    /// Corps de la vue.
    pub body: BodyState,
    /// Lignes des flux (avant recherche et tri).
    pub flow_rows: Vec<FlowRow>,
    /// Lignes des endpoints reliés (avant recherche et tri), restreintes aux
    /// endpoints sans label si demandé.
    pub endpoint_rows: Vec<EndpointRow>,
    /// Lignes des déductions (avant recherche et tri).
    pub deduction_rows: Vec<DeductionRow>,
    /// Flux affichés : recherche puis tri.
    pub visible_flows: Vec<FlowRow>,
    /// Endpoints affichés.
    pub visible_endpoints: Vec<EndpointRow>,
    /// Déductions affichées.
    pub visible_deductions: Vec<DeductionRow>,
    /// Entrées du glossaire présentes dans le relevé filtré.
    pub present_glossary: Vec<&'static GlossaryEntry>,
    /// Entrées affichées dans l'onglet Glossaire.
    pub wiki_entries: Vec<&'static GlossaryEntry>,
    /// Texte de l'onglet Glossaire sans entrée à afficher.
    pub wiki_empty_text: Option<String>,
}

/// La vue d'investigation : relevé chargé et état de la vue.
#[derive(Debug, Default)]
pub struct InvestigationModel {
    /// Snapshot du graphe (`get_graph_snapshot`).
    pub graph: GraphData,
    /// Période observée du relevé (`get_survey_period`), lue avec le graphe.
    pub period: SurveyPeriodView,
    /// État de la vue.
    pub state: ViewState,
}

/// Libellé « Présents dans le relevé (n) » du glossaire.
pub fn wiki_present_label(count: usize) -> String {
    format!("Présents dans le relevé ({count})")
}

/// Libellé « Tout le glossaire (n) ».
pub fn wiki_all_label() -> String {
    format!("Tout le glossaire ({})", GLOSSARY.len())
}

impl InvestigationModel {
    /// Remplace le relevé par un snapshot fraîchement chargé et termine le
    /// rechargement.
    pub fn load(&mut self, graph: GraphData, period: SurveyPeriodView) {
        self.graph = graph;
        self.period = period;
        self.state.finish_reload(true);
    }

    /// Valeur de l'indicateur « Durée du relevé » : du premier au dernier
    /// paquet observé, « inconnue » quand une matrice importée ne déclarait
    /// pas sa période.
    pub fn period_value(&self) -> String {
        duration_value(&self.period)
    }

    /// Infobulle de « Durée du relevé ».
    pub fn period_title(&self, zone: &impl LocalZone) -> String {
        match summarize_period(&self.period) {
            PeriodSummary::Unknown => {
                "Une matrice importée ne déclare pas sa période : la durée réelle ne peut pas être établie.".to_string()
            }
            PeriodSummary::Empty => "Aucun paquet observé.".to_string(),
            PeriodSummary::Known { first, last, .. } => {
                format!("Du {} au {}", format_short_medium(first, zone), format_short_medium(last, zone))
            }
        }
    }

    /// Calcule tout ce qu'affiche la vue. `zone` : fuseau des dates.
    pub fn derive(&self, zone: &impl LocalZone) -> InvestigationDerived {
        let state = &self.state;
        let graph = GraphView::new(&self.graph);
        let protocol_options = protocol_breakdown(&graph);
        // Sous-graphe restreint aux protocoles cochés : source unique des
        // lignes et des indicateurs, pour que tableaux et KPIs restent
        // cohérents.
        let allowed: HashSet<String> = protocol_options
            .iter()
            .map(|option| option.protocol.clone())
            .filter(|protocol| state.is_protocol_visible(protocol))
            .collect();
        let filtered = filter_graph_by_protocols(&graph, &allowed);

        let (summary, stats) = if state.loading {
            (
                InvestigationSummary::default(),
                InvestigationStats::default(),
            )
        } else {
            (
                build_summary(&filtered, DEFAULT_TOP_N),
                compute_stats(&filtered),
            )
        };

        let flow_rows = build_flow_rows(&filtered);
        // Sous filtre, un endpoint dont tous les flux sont masqués disparaît.
        let endpoint_rows: Vec<EndpointRow> = build_endpoint_rows(&filtered)
            .into_iter()
            .filter(|row| row.degree > 0 && (!state.only_unlabeled || row.label.is_empty()))
            .collect();
        let deduction_rows = build_deduction_rows(&filtered);
        let visible_flows = sort_rows(
            filter_rows(&flow_rows, &state.query),
            state.sort_key,
            state.sort_asc,
        );
        let visible_endpoints = sort_rows(
            filter_rows(&endpoint_rows, &state.query),
            state.sort_key,
            state.sort_asc,
        );
        let visible_deductions = sort_rows(
            filter_rows(&deduction_rows, &state.query),
            state.sort_key,
            state.sort_asc,
        );

        let present_glossary = match_glossary(&observed_glossary_tokens(&filtered));
        let wiki_entries = wiki_entries(
            if state.wiki_show_all {
                GLOSSARY.iter().collect()
            } else {
                present_glossary.clone()
            },
            &state.query,
        );
        let wiki_empty_text = wiki_entries.is_empty().then(|| {
            if !state.wiki_show_all && state.query.is_empty() {
                "Aucun terme technique reconnu dans le relevé pour l'instant. Passez à « Tout le glossaire » ou lancez une capture."
                    .to_string()
            } else {
                format!("Aucune définition ne correspond à « {} ».", state.query)
            }
        });

        let kpis = vec![
            Kpi {
                label: "Endpoints",
                value: stats.endpoints.to_string(),
                warn: false,
                title: None,
            },
            Kpi {
                label: "Flux",
                value: stats.flows.to_string(),
                warn: false,
                title: None,
            },
            Kpi {
                label: "Paquets",
                value: format_integer(stats.packets),
                warn: false,
                title: None,
            },
            Kpi {
                label: "Volume",
                value: format_bytes(stats.bytes),
                warn: false,
                title: None,
            },
            Kpi {
                label: "Protocoles",
                value: stats.protocols.to_string(),
                warn: false,
                title: None,
            },
            Kpi {
                label: "IP dupliquées",
                value: stats.duplicate_ips.to_string(),
                warn: stats.duplicate_ips > 0,
                title: None,
            },
            Kpi {
                label: "Durée du relevé",
                value: self.period_value(),
                warn: false,
                title: Some(self.period_title(zone)),
            },
        ];

        let tabs = vec![
            (Tab::Summary, "Résumé".to_string()),
            (Tab::Flows, format!("Flux ({})", flow_rows.len())),
            (
                Tab::Endpoints,
                format!("Endpoints ({})", endpoint_rows.len()),
            ),
            (
                Tab::Deduction,
                format!("Déduction ({})", deduction_rows.len()),
            ),
            (Tab::Wiki, format!("Glossaire ({})", present_glossary.len())),
        ];

        let body = if state.loading {
            BodyState::Loading
        } else if stats.flows == 0 && stats.endpoints == 0 && state.tab != Tab::Wiki {
            BodyState::EmptySurvey
        } else {
            BodyState::Tab(state.tab)
        };

        InvestigationDerived {
            filter_label: state.filter_button_label(&protocol_options),
            protocol_options,
            stats,
            summary,
            kpis,
            tabs,
            body,
            flow_rows,
            endpoint_rows,
            deduction_rows,
            visible_flows,
            visible_endpoints,
            visible_deductions,
            present_glossary,
            wiki_entries,
            wiki_empty_text,
        }
    }
}

/// Entrées affichées dans l'onglet Glossaire : `base` filtrée par la
/// recherche (terme, alias ou définition, comparés normalisés), triée par
/// catégorie puis alphabétiquement.
pub fn wiki_entries(base: Vec<&'static GlossaryEntry>, query: &str) -> Vec<&'static GlossaryEntry> {
    let query = normalize_token(js_trim(query));
    let mut entries: Vec<_> = if query.is_empty() {
        base
    } else {
        base.into_iter()
            .filter(|entry| {
                normalize_token(entry.term).contains(&query)
                    || entry
                        .aliases
                        .iter()
                        .any(|alias| normalize_token(alias).contains(&query))
                    || normalize_token(&entry.definition_for(APP_NAME)).contains(&query)
            })
            .collect()
    };
    entries.sort_by(|a, b| {
        a.category
            .order_index()
            .cmp(&b.category.order_index())
            .then_with(|| compare_fr(a.term, b.term))
    });
    entries
}

/// Cartes de l'onglet Résumé.
pub fn summary_cards(derived: &InvestigationDerived) -> Vec<SummaryCard> {
    let summary = &derived.summary;
    let total = derived.stats.bytes;
    let flows_meta = |flows: usize| format!("{flows} flux");
    let share_line = |name: &str, totals: &Totals| SummaryLine {
        name: name.to_string(),
        name_title: None,
        swatch: None,
        bar_percent: bar_width_percent(totals.bytes, total),
        bar_color: None,
        value: percent_of(totals.bytes, total),
        meta: flows_meta(totals.flows),
    };

    let max_protocol = summary.protocols.first().map_or(0, |share| share.bytes);
    let max_endpoint = summary.top_endpoints.first().map_or(0, |row| row.bytes);
    let max_flow = summary.top_flows.first().map_or(0, |row| row.bytes);
    vec![
        SummaryCard {
            title: "Répartition par protocole",
            subtitle: "par volume",
            lines: summary
                .protocols
                .iter()
                .map(|share| {
                    let color = color_for_protocol(Some(&share.protocol));
                    SummaryLine {
                        name: share.protocol.clone(),
                        name_title: None,
                        swatch: Some(color),
                        bar_percent: bar_width_percent(share.bytes, max_protocol),
                        bar_color: Some(color),
                        value: format_bytes(share.bytes),
                        meta: flows_meta(share.flows),
                    }
                })
                .collect(),
            empty_text: None,
        },
        SummaryCard {
            title: "Top émetteurs",
            subtitle: "par volume",
            lines: summary
                .top_endpoints
                .iter()
                .map(|row| SummaryLine {
                    name: row.name.clone(),
                    name_title: Some(row.id.clone()),
                    swatch: None,
                    bar_percent: bar_width_percent(row.bytes, max_endpoint),
                    bar_color: None,
                    value: format_bytes(row.bytes),
                    meta: flows_meta(row.degree),
                })
                .collect(),
            empty_text: Some("—"),
        },
        SummaryCard {
            title: "Top flux",
            subtitle: "par volume",
            lines: summary
                .top_flows
                .iter()
                .map(|row| {
                    let color = color_for_protocol(Some(&row.protocol));
                    SummaryLine {
                        name: format!("{} {} {}", row.source, arrow(row.bidir), row.target),
                        name_title: Some(format!("{} → {}", row.source_id, row.target_id)),
                        swatch: Some(color),
                        bar_percent: bar_width_percent(row.bytes, max_flow),
                        bar_color: Some(color),
                        value: format_bytes(row.bytes),
                        meta: format!("{} paq.", format_integer(row.packets)),
                    }
                })
                .collect(),
            empty_text: Some("—"),
        },
        SummaryCard {
            title: "Interne / Internet",
            subtitle: "par volume — « Internet » : une extrémité est une IP publique",
            lines: vec![
                share_line("Interne", &summary.scope.internal),
                share_line("Internet", &summary.scope.internet),
            ],
            empty_text: None,
        },
        SummaryCard {
            title: "Chiffrement",
            subtitle: "indication par volume, tirée du seul protocole reconnu, jamais du port",
            lines: summary
                .confidentiality
                .iter()
                .map(|part| share_line(part.kind.label(), &part.totals))
                .collect(),
            empty_text: None,
        },
        SummaryCard {
            title: "Usages",
            subtitle: "indication par volume, tirée d'une table fixe par protocole",
            lines: summary
                .usages
                .iter()
                .map(|part| share_line(part.usage.label(), &part.totals))
                .collect(),
            empty_text: Some("—"),
        },
    ]
}

/// Indicateurs d'alerte du bas du résumé.
pub fn summary_flags(derived: &InvestigationDerived) -> SummaryFlags {
    SummaryFlags {
        duplicate_ips: derived.stats.duplicate_ips,
        unlabeled: derived.summary.unlabeled,
    }
}

#[cfg(test)]
mod tests {
    // Comportement de la vue (les tris et la recherche sont aussi comparés
    // aux références de `tests/golden.rs`).

    use super::*;
    use crate::investigation::dates::FixedZone;
    use crate::investigation::test_support::{edge, graph, node, ports, s};
    use lunar_flows_core::graph::{Edge, Node};

    /// Trois endpoints, trois flux, un protocole par flux.
    fn sample() -> GraphData {
        graph(
            vec![
                (
                    "10.0.0.1",
                    Node {
                        id: s("a"),
                        name: s("10.0.0.1"),
                        ip: s("10.0.0.1"),
                        mac: s("00:0e:8c:11:22:33"),
                        macs: vec![s("00:0e:8c:11:22:33")],
                        label: Some(s("Automate")),
                        ..node()
                    },
                ),
                (
                    "10.0.0.2",
                    Node {
                        id: s("b"),
                        name: s("10.0.0.2"),
                        ip: s("10.0.0.2"),
                        mac: s("00:0c:29:aa:bb:cc"),
                        macs: vec![s("00:0c:29:aa:bb:cc"), s("00:50:56:01:02:03")],
                        vlan_id: Some(20),
                        duplicate_ip: true,
                        ..node()
                    },
                ),
                (
                    "8.8.8.8",
                    Node {
                        id: s("c"),
                        name: s("8.8.8.8"),
                        ip: s("8.8.8.8"),
                        color: s("#2196F3"),
                        ..node()
                    },
                ),
            ],
            vec![
                (
                    "a:b",
                    Edge {
                        id: s("e1"),
                        source: s("a"),
                        target: s("b"),
                        label: s("ModbusTCP"),
                        ports: ports(&[502]),
                        bidir: true,
                        count: 10,
                        total_bytes: 1000,
                        ..edge()
                    },
                ),
                (
                    "a:c",
                    Edge {
                        id: s("e2"),
                        source: s("a"),
                        target: s("c"),
                        label: s("DNS"),
                        ports: ports(&[53]),
                        count: 2,
                        total_bytes: 200,
                        ..edge()
                    },
                ),
                (
                    "b:c",
                    Edge {
                        id: s("e3"),
                        source: s("b"),
                        target: s("c"),
                        label: s("TLS"),
                        ports: ports(&[443]),
                        count: 3,
                        total_bytes: 3000,
                        ..edge()
                    },
                ),
            ],
        )
    }

    /// Modèle chargé avec `graph` et `period`.
    fn loaded(graph: GraphData, period: SurveyPeriodView) -> InvestigationModel {
        let mut model = InvestigationModel::default();
        model.load(graph, period);
        model
    }

    /// Ligne d'endpoint de [`sample`] d'id `id`.
    fn endpoint(id: &str) -> EndpointRow {
        let data = sample();
        build_endpoint_rows(&GraphView::new(&data))
            .into_iter()
            .find(|row| row.id == id)
            .expect("endpoint")
    }

    #[test]
    fn etat_initial_de_la_vue() {
        let state = ViewState::default();
        assert!(state.loading);
        assert_eq!(state.tab, Tab::Summary);
        assert_eq!((state.sort_key, state.sort_asc), (SortKey::Bytes, false));
        assert_eq!(state.aria_sort(SortKey::Bytes), AriaSort::Descending);
        assert_eq!(state.sort_indicator(SortKey::Bytes), "▼");
        assert_eq!(state.aria_sort(SortKey::Name), AriaSort::None);
        assert_eq!(state.sort_indicator(SortKey::Name), "");
    }

    #[test]
    fn tri_par_colonne_inverse_ou_choisit_le_sens_par_defaut() {
        let mut state = ViewState::default();
        state.sort_by(SortKey::Bytes);
        assert!(state.sort_asc, "même colonne : sens inversé");
        state.sort_by(SortKey::Name);
        assert_eq!((state.sort_key, state.sort_asc), (SortKey::Name, true));
        assert_eq!(state.sort_indicator(SortKey::Name), "▲");
        state.sort_by(SortKey::Packets);
        assert_eq!((state.sort_key, state.sort_asc), (SortKey::Packets, false));
        // Le VLAN n'est pas rangé parmi les mesures.
        state.sort_by(SortKey::Vlan);
        assert!(state.sort_asc);
        state.sort_by(SortKey::Count);
        assert!(!state.sort_asc);
        for key in SortKey::ALL {
            assert_eq!(SortKey::from_name(key.name()), Some(key));
        }
        assert_eq!(SortKey::from_name("inconnue"), None);
    }

    #[test]
    fn filtre_par_protocole_et_libelle_du_bouton() {
        let data = sample();
        let options = protocol_breakdown(&GraphView::new(&data));
        let mut state = ViewState::default();
        assert_eq!(state.filter_button_label(&options), "⚙ Filtre");
        state.toggle_protocol("DNS");
        assert!(!state.is_protocol_visible("DNS"));
        assert_eq!(state.filter_button_label(&options), "⚙ Filtre (2/3)");
        state.toggle_protocol("DNS");
        assert!(state.all_protocols_visible());
        state.set_all_protocols(false, &options);
        assert_eq!(state.filter_button_label(&options), "⚙ Filtre (0/3)");
        // Des protocoles masqués peuvent disparaître au rafraîchissement : la
        // différence reste signée, comme dans le gabarit.
        assert_eq!(state.filter_button_label(&options[..1]), "⚙ Filtre (-2/1)");
        state.set_all_protocols(true, &options);
        assert_eq!(state.filter_button_label(&options), "⚙ Filtre");
    }

    #[test]
    fn colonnes_masquables_par_onglet() {
        let mut state = ViewState::default();
        assert!(state.column_options().is_empty());
        state.tab = Tab::Flows;
        assert_eq!(state.column_options().len(), FLOW_COLUMNS.len());
        state.tab = Tab::Wiki;
        assert!(state.column_options().is_empty());
        state.toggle_column(SortKey::Packets);
        assert!(!state.is_column_visible(SortKey::Packets));
        state.toggle_column(SortKey::Packets);
        assert!(state.is_column_visible(SortKey::Packets));
        assert_eq!(
            column_title(SortKey::FlowsOut)
                .map(|title| title.starts_with("Flux où l'endpoint émet")),
            Some(true)
        );
        assert_eq!(column_title(SortKey::Name), None);
    }

    #[test]
    fn etiqueter_ouvre_les_endpoints_sans_label_les_plus_actifs() {
        let mut state = ViewState::default();
        state.show_unlabeled();
        assert!(state.only_unlabeled);
        assert_eq!(
            (state.tab, state.sort_key, state.sort_asc),
            (Tab::Endpoints, SortKey::Packets, false)
        );
    }

    #[test]
    fn exports_gardes_et_comptes_rendus() {
        let mut state = ViewState::default();
        assert!(!state.begin_export(false), "refusé pendant le chargement");
        state.finish_reload(true);
        assert!(!state.begin_export(true));
        assert_eq!(state.error, BUSY_ERROR);
        assert!(state.begin_export(false));
        assert!(state.exporting && state.error.is_empty());
        assert!(!state.begin_export(false), "un export à la fois");
        state.finish_export(ExportOutcome::Done(s("fait")));
        assert_eq!((state.notice.as_str(), state.exporting), ("fait", false));
        assert!(state.begin_export(false));
        assert!(state.notice.is_empty());
        state.finish_export(ExportOutcome::Failed(SYNTHESIS_EXPORT_ERROR));
        assert_eq!(state.error, SYNTHESIS_EXPORT_ERROR);
        assert!(state.begin_export(false));
        state.finish_export(ExportOutcome::Cancelled);
        assert!(state.error.is_empty() && state.notice.is_empty() && !state.exporting);
        state.begin_reload();
        state.finish_reload(false);
        assert_eq!((state.error.as_str(), state.loading), (LOAD_ERROR, false));
    }

    #[test]
    fn recherche_sur_les_valeurs_brutes_des_lignes() {
        let data = sample();
        let view = GraphView::new(&data);
        let flows = build_flow_rows(&view);
        let ids = |rows: Vec<FlowRow>| rows.into_iter().map(|row| row.id).collect::<Vec<_>>();
        assert_eq!(ids(filter_rows(&flows, "")), ["a:b", "a:c", "b:c"]);
        // `String(true)` : la recherche « true » trouve les flux bidirectionnels.
        assert_eq!(ids(filter_rows(&flows, "  TRUE ")), ["a:b"]);
        assert_eq!(ids(filter_rows(&flows, "443")), ["b:c"]);
        assert_eq!(ids(filter_rows(&flows, "automate")), ["a:b", "a:c"]);
        let endpoints = build_endpoint_rows(&view);
        let names =
            |rows: Vec<EndpointRow>| rows.into_iter().map(|row| row.name).collect::<Vec<_>>();
        // `String(null)` : un endpoint sans VLAN répond à « null ».
        assert_eq!(
            names(filter_rows(&endpoints, "null")),
            ["Automate", "8.8.8.8"]
        );
        // Les MAC multiples sont jointes par des virgules.
        assert_eq!(names(filter_rows(&endpoints, "bb:cc,00:50")), ["10.0.0.2"]);
    }

    #[test]
    fn tri_numerique_ou_textuel_et_stable() {
        let data = sample();
        let endpoints = build_endpoint_rows(&GraphView::new(&data));
        let names =
            |rows: Vec<EndpointRow>| rows.into_iter().map(|row| row.name).collect::<Vec<_>>();
        assert_eq!(
            names(sort_rows(endpoints.clone(), SortKey::Bytes, false)),
            ["10.0.0.2", "8.8.8.8", "Automate"]
        );
        // VLAN absent : `String(undefined ?? "")`, avant tout nombre.
        assert_eq!(
            names(sort_rows(endpoints.clone(), SortKey::Vlan, true)),
            ["Automate", "8.8.8.8", "10.0.0.2"]
        );
        assert_eq!(
            names(sort_rows(endpoints.clone(), SortKey::Vlan, false)),
            ["10.0.0.2", "Automate", "8.8.8.8"]
        );
        // Clé d'un autre tableau : tout est égal, l'ordre d'origine reste.
        assert_eq!(
            names(sort_rows(endpoints, SortKey::Protocol, false)),
            ["Automate", "10.0.0.2", "8.8.8.8"]
        );
    }

    #[test]
    fn cellules_affichees() {
        let data = sample();
        let flows = build_flow_rows(&GraphView::new(&data));
        assert_eq!(flow_cell(&flows[0], SortKey::Bidir), "↔");
        assert_eq!(flow_cell(&flows[1], SortKey::Bidir), "→");
        assert_eq!(flow_cell(&flows[0], SortKey::Bytes), "1000 o");
        assert_eq!(flow_cell_title(&flows[0], SortKey::Source), Some("a"));
        let b = endpoint("b");
        assert_eq!(endpoint_cell(&b, SortKey::Ip), "10.0.0.2 ⚠");
        assert_eq!(
            endpoint_cell_title(&b, SortKey::Ip).as_deref(),
            Some(DUPLICATE_IP_TITLE)
        );
        assert_eq!(endpoint_cell(&b, SortKey::Mac), "00:0c:29:aa:bb:cc (+1)");
        assert_eq!(
            endpoint_cell_title(&b, SortKey::Mac).as_deref(),
            Some("00:0c:29:aa:bb:cc, 00:50:56:01:02:03")
        );
        assert_eq!(endpoint_cell(&b, SortKey::Vlan), "20");
        assert_eq!(endpoint_cell(&endpoint("c"), SortKey::Vlan), "—");
        assert_eq!(endpoint_cell(&endpoint("c"), SortKey::Mac), "—");
        assert_eq!(label_input_aria(&b), "Label de 10.0.0.2");
        assert_eq!(bar_width(1, 3), "33.33333333333333%");
        assert_eq!(bar_width(1, 1000), "2%");
        assert_eq!(bar_width(0, 10), "0%");
    }

    #[test]
    fn saisie_de_label() {
        let a = endpoint("a");
        assert_eq!(label_command(&a, "  Automate "), None);
        assert_eq!(
            label_command(&a, " Automate 2 "),
            Some(LabelCommand::Add {
                mac: s("00:0e:8c:11:22:33"),
                ip: s("10.0.0.1"),
                label: s("Automate 2")
            })
        );
        assert_eq!(
            label_command(&a, "   "),
            Some(LabelCommand::Delete {
                mac: s("00:0e:8c:11:22:33"),
                ip: s("10.0.0.1")
            })
        );
        assert_eq!(label_command(&endpoint("c"), ""), None);
    }

    #[test]
    fn noms_de_fichiers_et_comptes_rendus() {
        assert_eq!(
            synthesis_default_file_name("20261012"),
            "20261012_DR_Synthese.html"
        );
        assert_eq!(
            labels_default_file_name("20261012"),
            "20261012_DR_labels_a_completer.csv"
        );
        assert_eq!(file_name_of(r"C:\Users\op\synthese.html"), "synthese.html");
        assert_eq!(file_name_of("/home/op/synthese.html"), "synthese.html");
        assert_eq!(file_name_of("synthese.html"), "synthese.html");
        assert_eq!(
            synthesis_notice("/tmp/s.html", "s.csv", &"0123456789abcdef".repeat(4)),
            "Synthèse exportée : s.html et s.csv (SHA-256 0123456789abcdef…)."
        );
        assert_eq!(
            labels_notice(3, r"D:\labels.csv"),
            "3 équipement(s) sans label exporté(s) dans labels.csv : complétez la colonne label, puis réimportez le fichier (Import de labels)."
        );
    }

    #[test]
    fn derive_pendant_le_chargement_puis_sur_le_releve() {
        let zone = FixedZone::new(7200);
        let mut model = InvestigationModel {
            graph: sample(),
            ..InvestigationModel::default()
        };
        let derived = model.derive(&zone);
        assert_eq!(derived.body, BodyState::Loading);
        assert_eq!(derived.stats, InvestigationStats::default());
        assert_eq!(
            derived.flow_rows.len(),
            3,
            "les tableaux ne dépendent pas du chargement"
        );

        model.load(
            sample(),
            SurveyPeriodView::Known {
                first: s("2026-10-12T08:00:00Z"),
                last: s("2026-10-12T09:05:00Z"),
            },
        );
        let derived = model.derive(&zone);
        assert_eq!(derived.body, BodyState::Tab(Tab::Summary));
        let kpis: Vec<_> = derived
            .kpis
            .iter()
            .map(|kpi| (kpi.label, kpi.value.as_str(), kpi.warn))
            .collect();
        assert_eq!(
            kpis,
            [
                ("Endpoints", "3", false),
                ("Flux", "3", false),
                ("Paquets", "15", false),
                ("Volume", "4.1 Kio", false),
                ("Protocoles", "3", false),
                ("IP dupliquées", "1", true),
                ("Durée du relevé", "1 h 05 min", false),
            ]
        );
        assert_eq!(
            derived.kpis[6].title.as_deref(),
            Some("Du 12/10/2026 10:00:00 au 12/10/2026 11:05:00")
        );
        let tabs: Vec<_> = derived
            .tabs
            .iter()
            .map(|(_, label)| label.as_str())
            .collect();
        assert_eq!(
            tabs[..4],
            ["Résumé", "Flux (3)", "Endpoints (3)", "Déduction (7)"]
        );
        assert!(tabs[4].starts_with("Glossaire ("));
        let visible: Vec<_> = derived
            .visible_flows
            .iter()
            .map(|row| row.id.as_str())
            .collect();
        assert_eq!(
            visible,
            ["b:c", "a:b", "a:c"],
            "tri par défaut : volume décroissant"
        );

        // Filtre : DNS décoché, KPIs et lignes suivent ; 8.8.8.8 garde TLS.
        model.state.toggle_protocol("DNS");
        model.state.only_unlabeled = true;
        let derived = model.derive(&zone);
        assert_eq!(derived.protocol_options.len(), 3, "les cases restent");
        assert_eq!(derived.filter_label, "⚙ Filtre (2/3)");
        assert_eq!((derived.stats.flows, derived.stats.packets), (2, 13));
        let endpoints: Vec<_> = derived
            .endpoint_rows
            .iter()
            .map(|row| row.id.as_str())
            .collect();
        assert_eq!(endpoints, ["b", "c"], "sans label seulement");
    }

    #[test]
    fn releve_vide_et_glossaire() {
        let zone = FixedZone::UTC;
        let mut model = loaded(GraphData::new(), SurveyPeriodView::Empty);
        let derived = model.derive(&zone);
        assert_eq!(derived.body, BodyState::EmptySurvey);
        assert_eq!(derived.kpis[6].value, "—");
        assert_eq!(
            derived.kpis[6].title.as_deref(),
            Some("Aucun paquet observé.")
        );
        assert_eq!(
            derived.wiki_empty_text.as_deref(),
            Some(
                "Aucun terme technique reconnu dans le relevé pour l'instant. Passez à « Tout le glossaire » ou lancez une capture."
            )
        );
        model.state.tab = Tab::Wiki;
        model.state.wiki_show_all = true;
        let derived = model.derive(&zone);
        assert_eq!(
            derived.body,
            BodyState::Tab(Tab::Wiki),
            "le glossaire reste consultable"
        );
        assert_eq!(derived.wiki_entries.len(), GLOSSARY.len());
        assert_eq!(derived.wiki_empty_text, None);
        model.state.query = s("zzz");
        let derived = model.derive(&zone);
        assert_eq!(
            derived.wiki_empty_text.as_deref(),
            Some("Aucune définition ne correspond à « zzz ».")
        );
        assert_eq!(
            wiki_all_label(),
            format!("Tout le glossaire ({})", GLOSSARY.len())
        );
        assert_eq!(wiki_present_label(4), "Présents dans le relevé (4)");

        let unknown = loaded(GraphData::new(), SurveyPeriodView::Unknown);
        assert_eq!(unknown.period_value(), "inconnue");
        assert!(
            unknown
                .period_title(&zone)
                .starts_with("Une matrice importée ne déclare pas sa période")
        );
    }

    #[test]
    fn glossaire_trie_par_categorie_puis_terme() {
        let entries = wiki_entries(GLOSSARY.iter().collect(), "");
        for pair in entries.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let order = a
                .category
                .order_index()
                .cmp(&b.category.order_index())
                .then_with(|| compare_fr(a.term, b.term));
            assert_ne!(order, Ordering::Greater, "{} avant {}", a.term, b.term);
        }
        // Recherche normalisée sur le terme, les alias et la définition.
        let terms: Vec<_> = wiki_entries(GLOSSARY.iter().collect(), " MODBUS ")
            .iter()
            .map(|entry| entry.term)
            .collect();
        assert!(
            terms.iter().any(|term| term.starts_with("Modbus")),
            "{terms:?}"
        );
    }

    #[test]
    fn cartes_et_alertes_du_resume() {
        let model = loaded(sample(), SurveyPeriodView::Unknown);
        let derived = model.derive(&FixedZone::UTC);
        let cards = summary_cards(&derived);
        let titles: Vec<_> = cards.iter().map(|card| card.title).collect();
        assert_eq!(
            titles,
            [
                "Répartition par protocole",
                "Top émetteurs",
                "Top flux",
                "Interne / Internet",
                "Chiffrement",
                "Usages"
            ]
        );
        let protocols = &cards[0].lines;
        assert_eq!(protocols[0].name, "TLS");
        assert_eq!(
            (protocols[0].bar_percent, protocols[0].meta.as_str()),
            (100.0, "1 flux")
        );
        assert_eq!(protocols[0].swatch, Some(color_for_protocol(Some("TLS"))));
        let top_flow = &cards[2].lines[0];
        assert_eq!(top_flow.name, "10.0.0.2 → 8.8.8.8");
        assert_eq!(top_flow.name_title.as_deref(), Some("b → c"));
        assert_eq!(top_flow.meta, "3 paq.");
        let scope: Vec<_> = cards[3]
            .lines
            .iter()
            .map(|line| (line.name.as_str(), line.value.as_str()))
            .collect();
        assert_eq!(scope, [("Interne", "23,8 %"), ("Internet", "76,2 %")]);
        assert_eq!(
            summary_flags(&derived),
            SummaryFlags {
                duplicate_ips: 1,
                unlabeled: 2
            }
        );
    }
}
