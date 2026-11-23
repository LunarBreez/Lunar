//! Barre de menus, actions et raccourcis clavier.
//!
//! Une seule table décrit les actions et une seule leur rangement :
//! [`MENU_ACTIONS`] porte chaque action (libellé, raccourci, vue, règle
//! d'activation, traduction clavier, aide F1), [`MENU_SECTIONS`] les
//! sous-menus, leur ordre et leurs séparateurs.

use std::fmt;
use std::ops::Index;

/// Vue affichée : la vue d'analyse (barre d'outils) ou la vue d'investigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MenuView {
    /// Vue d'analyse (capture, matrice, graphe).
    #[default]
    Analyse,
    /// Vue d'investigation (lecture approfondie du relevé).
    Investigation,
}

/// Vue qui porte une action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActionView {
    /// Disponible partout (quitter, aide).
    Any,
    /// Vue d'analyse seulement.
    Analyse,
    /// Vue d'investigation seulement.
    Investigation,
}

impl ActionView {
    /// `true` si une action de cette vue est disponible dans `view`. Une
    /// entrée d'une autre vue est toujours grisée : c'est ce qui permet à
    /// deux entrées de vues exclusives de partager un raccourci.
    pub const fn includes(self, view: MenuView) -> bool {
        matches!(
            (self, view),
            (ActionView::Any, _)
                | (ActionView::Analyse, MenuView::Analyse)
                | (ActionView::Investigation, MenuView::Investigation)
        )
    }
}

/// Identifiant d'une action de menu. L'ordre des variantes est celui de
/// [`MENU_ACTIONS`] (et du menu).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MenuActionId {
    /// Ouvrir un projet `.lunar`.
    OpenProject,
    /// Enregistrer le projet `.lunar`.
    SaveProject,
    /// Ouvrir un fichier PCAP ou une matrice de flux.
    OpenCaptureFile,
    /// Importer un fichier de labels.
    ImportLabels,
    /// Exporter la matrice de flux en CSV.
    ExportCsv,
    /// Exporter les labels.
    ExportLabels,
    /// Exporter les logs (archive de support).
    ExportLogs,
    /// Quitter.
    Quit,
    /// Démarrer la capture.
    StartCapture,
    /// Arrêter la capture.
    StopCapture,
    /// Réinitialiser le relevé.
    ResetCapture,
    /// Configuration de la capture.
    OpenConfig,
    /// Panneau de filtre BPF.
    OpenFilter,
    /// Panneau des labels.
    OpenLabels,
    /// Afficher ou masquer le graphe.
    ToggleGraph,
    /// Basculer vers la vue d'investigation.
    OpenInvestigation,
    /// Revenir à la vue d'analyse.
    BackToAnalyse,
    /// Exporter la synthèse (HTML + CSV).
    ExportSynthesis,
    /// Exporter les labels à compléter.
    ExportLabelsToComplete,
    /// Exporter les seuls flux affichés (matrice partielle).
    ExportFilteredFlows,
    /// Rafraîchir le relevé de la vue d'investigation.
    ReloadSurvey,
    /// Aide des raccourcis clavier.
    Shortcuts,
    /// À propos de LUNAR.
    About,
    /// Changelog.
    Changelog,
}

/// Nombre d'actions de menu.
pub const MENU_ACTION_COUNT: usize = 24;

impl MenuActionId {
    /// Description de l'action dans [`MENU_ACTIONS`].
    pub fn action(self) -> &'static MenuAction {
        &MENU_ACTIONS[self as usize]
    }

    /// Identifiant textuel stable (`open-project`…).
    pub fn as_str(self) -> &'static str {
        self.action().name
    }

    /// Libellé affiché.
    pub fn label(self) -> &'static str {
        self.action().label
    }

    /// Raccourci clavier, s'il y en a un.
    pub fn shortcut(self) -> Option<Shortcut> {
        self.action().shortcut
    }

    /// Vue qui porte l'action.
    pub fn view(self) -> ActionView {
        self.action().view
    }

    /// Action d'identifiant textuel `name`, ou `None`.
    pub fn parse(name: &str) -> Option<Self> {
        MENU_ACTIONS
            .iter()
            .find(|action| action.name == name)
            .map(|action| action.id)
    }

    /// Toutes les actions, dans l'ordre de la table.
    pub fn all() -> impl Iterator<Item = MenuActionId> {
        MENU_ACTIONS.iter().map(|action| action.id)
    }
}

impl fmt::Display for MenuActionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Touche d'un raccourci.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    /// Caractère, en minuscule (`'o'`, `','`).
    Char(char),
    /// Touche de fonction `F1`…`F24`.
    Function(u8),
}

/// Raccourci clavier, en données : modificateurs et touche. Ctrl vaut aussi
/// Cmd sous macOS (`CmdOrCtrl` du menu natif).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Shortcut {
    /// Ctrl (ou Cmd).
    pub ctrl: bool,
    /// Maj.
    pub shift: bool,
    /// Touche.
    pub key: Key,
}

impl Shortcut {
    /// `Ctrl+<c>`.
    pub const fn ctrl(c: char) -> Self {
        Self {
            ctrl: true,
            shift: false,
            key: Key::Char(c),
        }
    }

    /// `Ctrl+Shift+<c>`.
    pub const fn ctrl_shift(c: char) -> Self {
        Self {
            ctrl: true,
            shift: true,
            key: Key::Char(c),
        }
    }

    /// Touche de fonction seule, `F<n>`.
    pub const fn function(n: u8) -> Self {
        Self {
            ctrl: false,
            shift: false,
            key: Key::Function(n),
        }
    }

    /// Nom de la touche affiché : lettre en majuscule, `F1`.
    fn display_key(&self) -> String {
        match self.key {
            Key::Char(c) => c.to_uppercase().collect(),
            Key::Function(n) => format!("F{n}"),
        }
    }

    /// Nom de la touche en minuscules, forme comparée à l'événement clavier
    /// et forme GPUI (`o`, `,`, `f1`).
    fn lower_key(&self) -> String {
        match self.key {
            Key::Char(c) => c.to_lowercase().collect(),
            Key::Function(n) => format!("f{n}"),
        }
    }

    /// Forme affichée dans le menu et l'aide : `Ctrl+Shift+O`, `Ctrl+,`, `F1`.
    pub fn display(&self) -> String {
        let mut text = String::new();
        if self.ctrl {
            text.push_str("Ctrl+");
        }
        if self.shift {
            text.push_str("Shift+");
        }
        text.push_str(&self.display_key());
        text
    }

    /// Forme des keystrokes GPUI (`KeyBinding::new`) : `ctrl-shift-o`,
    /// `ctrl-,`, `f1`. Modificateurs dans l'ordre de `Keystroke::unparse`.
    pub fn to_gpui_keystroke(&self) -> String {
        let mut text = String::new();
        if self.ctrl {
            text.push_str("ctrl-");
        }
        if self.shift {
            text.push_str("shift-");
        }
        text.push_str(&self.lower_key());
        text
    }

    /// `true` si l'appui `event` (touche normalisée `key`) déclenche ce raccourci.
    fn matches(&self, event: &ShortcutKey<'_>, key: &str) -> bool {
        self.ctrl == (event.ctrl || event.meta)
            && self.shift == event.shift
            && self.lower_key() == key
    }
}

impl fmt::Display for Shortcut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.display())
    }
}

/// Une action de menu : identifiant, libellé, raccourci, vue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuAction {
    /// Identifiant typé.
    pub id: MenuActionId,
    /// Identifiant textuel stable (`open-project`).
    pub name: &'static str,
    /// Libellé affiché.
    pub label: &'static str,
    /// Raccourci clavier, s'il y en a un.
    pub shortcut: Option<Shortcut>,
    /// Vue qui porte l'action.
    pub view: ActionView,
}

/// Constructeur `const` d'une action, pour écrire la table en constante.
const fn action(
    id: MenuActionId,
    name: &'static str,
    label: &'static str,
    shortcut: Option<Shortcut>,
    view: ActionView,
) -> MenuAction {
    MenuAction {
        id,
        name,
        label,
        shortcut,
        view,
    }
}

/// Les actions, dans l'ordre des variantes de [`MenuActionId`] (vérifié par
/// un test) et du menu.
pub const MENU_ACTIONS: [MenuAction; MENU_ACTION_COUNT] = {
    use ActionView::{Analyse, Any, Investigation};
    use MenuActionId as A;
    [
        action(
            A::OpenProject,
            "open-project",
            "Ouvrir un projet (.lunar)…",
            Some(Shortcut::ctrl_shift('o')),
            Analyse,
        ),
        action(
            A::SaveProject,
            "save-project",
            "Enregistrer le projet (.lunar)…",
            Some(Shortcut::ctrl_shift('s')),
            Analyse,
        ),
        action(
            A::OpenCaptureFile,
            "open-capture-file",
            "Ouvrir un fichier PCAP ou une matrice de flux…",
            Some(Shortcut::ctrl('o')),
            Analyse,
        ),
        action(
            A::ImportLabels,
            "import-labels",
            "Importer un fichier de labels…",
            Some(Shortcut::ctrl_shift('l')),
            Analyse,
        ),
        action(
            A::ExportCsv,
            "export-csv",
            "Exporter la matrice de flux (CSV)…",
            Some(Shortcut::ctrl('s')),
            Analyse,
        ),
        action(
            A::ExportLabels,
            "export-labels",
            "Exporter les labels…",
            Some(Shortcut::ctrl('e')),
            Analyse,
        ),
        action(
            A::ExportLogs,
            "export-logs",
            "Exporter les logs (archive de support)…",
            Some(Shortcut::ctrl('l')),
            Analyse,
        ),
        action(A::Quit, "quit", "Quitter", Some(Shortcut::ctrl('q')), Any),
        action(
            A::StartCapture,
            "start-capture",
            "Démarrer la capture",
            Some(Shortcut::ctrl('p')),
            Analyse,
        ),
        action(
            A::StopCapture,
            "stop-capture",
            "Arrêter la capture",
            Some(Shortcut::ctrl_shift('p')),
            Analyse,
        ),
        action(
            A::ResetCapture,
            "reset-capture",
            "Réinitialiser le relevé",
            Some(Shortcut::ctrl_shift('r')),
            Analyse,
        ),
        action(
            A::OpenConfig,
            "open-config",
            "Configuration de la capture…",
            Some(Shortcut::ctrl(',')),
            Analyse,
        ),
        action(
            A::OpenFilter,
            "open-filter",
            "Filtrer…",
            Some(Shortcut::ctrl('f')),
            Analyse,
        ),
        action(
            A::OpenLabels,
            "open-labels",
            "Gérer les labels…",
            Some(Shortcut::ctrl('k')),
            Analyse,
        ),
        action(
            A::ToggleGraph,
            "toggle-graph",
            "Afficher ou masquer le graphe",
            Some(Shortcut::ctrl('g')),
            Analyse,
        ),
        action(
            A::OpenInvestigation,
            "open-investigation",
            "Vue d'investigation",
            Some(Shortcut::ctrl('i')),
            Analyse,
        ),
        action(
            A::BackToAnalyse,
            "back-to-analyse",
            "Retour à l'analyse",
            Some(Shortcut::ctrl('i')),
            Investigation,
        ),
        action(
            A::ExportSynthesis,
            "export-synthesis",
            "Exporter la synthèse (HTML + CSV)…",
            Some(Shortcut::ctrl('s')),
            Investigation,
        ),
        action(
            A::ExportLabelsToComplete,
            "export-labels-to-complete",
            "Exporter les labels à compléter…",
            Some(Shortcut::ctrl('e')),
            Investigation,
        ),
        action(
            A::ExportFilteredFlows,
            "export-filtered-flows",
            "Exporter les flux affichés (CSV partiel)…",
            None,
            Investigation,
        ),
        action(
            A::ReloadSurvey,
            "reload-survey",
            "Rafraîchir le relevé",
            None,
            Investigation,
        ),
        action(
            A::Shortcuts,
            "shortcuts",
            "Raccourcis clavier",
            Some(Shortcut::function(1)),
            Any,
        ),
        action(A::About, "about", "À propos de LUNAR", None, Any),
        action(A::Changelog, "changelog", "Changelog", None, Any),
    ]
};

// --- Rangement en sous-menus ------------------------------------------------

/// Une ligne de sous-menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuRow {
    /// Entrée d'action.
    Action(MenuActionId),
    /// Séparateur.
    Separator,
}

/// Un sous-menu de la barre : titre (avec mnémonique `&`) et lignes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuSection {
    /// Titre, `&` devant la lettre du mnémonique (`&Fichier`, `A&ide`).
    pub title: &'static str,
    /// Lignes du sous-menu.
    pub rows: &'static [MenuRow],
}

impl MenuSection {
    /// Titre affiché, sans `&` (`Fichier`, `Aide`).
    pub fn label(&self) -> String {
        self.title.replace('&', "")
    }

    /// Lettre du mnémonique (celle qui suit `&`), en minuscule.
    pub fn mnemonic(&self) -> Option<char> {
        let (_, rest) = self.title.split_once('&')?;
        rest.chars().next().map(|c| c.to_ascii_lowercase())
    }

    /// Actions du sous-menu, sans les séparateurs.
    pub fn actions(&self) -> impl Iterator<Item = MenuActionId> + '_ {
        self.rows.iter().filter_map(|row| match row {
            MenuRow::Action(id) => Some(*id),
            MenuRow::Separator => None,
        })
    }
}

/// Raccourci d'écriture des tables de sous-menus.
const fn row(id: MenuActionId) -> MenuRow {
    MenuRow::Action(id)
}

/// Séparateur, raccourci d'écriture des tables de sous-menus.
const SEP: MenuRow = MenuRow::Separator;

/// Les sous-menus de la barre, dans l'ordre d'affichage (`SECTIONS` de
/// `setup/menu.rs`). Affichage : la bascule analyse ⇄ investigation a le
/// même raccourci dans les deux sens, une seule des deux entrées est active ;
/// les exports d'Investigation doublent de même ceux de Fichier.
pub const MENU_SECTIONS: &[MenuSection] = {
    use MenuActionId as A;
    &[
        MenuSection {
            title: "&Fichier",
            rows: &[
                row(A::OpenProject),
                row(A::SaveProject),
                SEP,
                row(A::OpenCaptureFile),
                row(A::ImportLabels),
                SEP,
                row(A::ExportCsv),
                row(A::ExportLabels),
                row(A::ExportLogs),
                SEP,
                row(A::Quit),
            ],
        },
        MenuSection {
            title: "&Capture",
            rows: &[
                row(A::StartCapture),
                row(A::StopCapture),
                row(A::ResetCapture),
                SEP,
                row(A::OpenConfig),
            ],
        },
        MenuSection {
            title: "&Affichage",
            rows: &[
                row(A::OpenFilter),
                row(A::OpenLabels),
                row(A::ToggleGraph),
                SEP,
                row(A::OpenInvestigation),
                row(A::BackToAnalyse),
            ],
        },
        MenuSection {
            title: "&Investigation",
            rows: &[
                row(A::ExportSynthesis),
                row(A::ExportLabelsToComplete),
                row(A::ExportFilteredFlows),
                SEP,
                row(A::ReloadSurvey),
            ],
        },
        MenuSection {
            title: "A&ide",
            rows: &[row(A::Shortcuts), SEP, row(A::About), row(A::Changelog)],
        },
    ]
};

// --- Activation des entrées ------------------------------------------------

/// État de l'interface dont dépend l'activation des entrées : le miroir des
/// gardes des boutons de la barre d'outils et de la vue d'investigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MenuContext {
    /// Vue affichée.
    pub view: MenuView,
    /// Capture en cours.
    pub running: bool,
    /// Démarrage de capture en cours (double déclenchement interdit).
    pub starting: bool,
    /// Import ou export en cours (verrou global).
    pub importing: bool,
    /// La matrice contient déjà des données.
    pub has_data: bool,
    /// Un panneau (config, import, filtre, labels) est ouvert.
    pub panel_open: bool,
    /// Vue d'investigation : chargement du relevé en cours.
    pub loading: bool,
    /// Vue d'investigation : export en cours.
    pub exporting: bool,
}

/// `true` si l'entrée `id` est active dans l'état `ctx`.
pub fn is_action_enabled(id: MenuActionId, ctx: &MenuContext) -> bool {
    use MenuActionId as A;
    if !id.view().includes(ctx.view) {
        return false;
    }
    let idle = !ctx.importing;
    match id {
        // Fichier
        A::OpenProject | A::OpenCaptureFile => idle && !ctx.running && !ctx.has_data,
        A::SaveProject | A::ExportCsv | A::ExportLabels | A::ExportLogs => idle,
        A::ImportLabels | A::OpenConfig | A::OpenFilter | A::OpenLabels => idle && !ctx.running,
        // Capture
        A::StartCapture => idle && !ctx.running && !ctx.starting && !ctx.panel_open,
        A::StopCapture => ctx.running,
        A::ResetCapture => idle && !ctx.panel_open,
        // Affichage
        A::OpenInvestigation => !ctx.running && !ctx.panel_open,
        A::ToggleGraph | A::BackToAnalyse => true,
        // Investigation
        A::ExportSynthesis | A::ExportLabelsToComplete | A::ExportFilteredFlows => {
            idle && !ctx.loading && !ctx.exporting
        }
        A::ReloadSurvey => !ctx.loading,
        // Partout
        A::Quit | A::Shortcuts | A::About | A::Changelog => true,
    }
}

/// Activation de chaque entrée pour un état donné, indexable par
/// identifiant : `state[MenuActionId::StartCapture]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuEnabledState {
    /// Activation, dans l'ordre de [`MENU_ACTIONS`].
    enabled: [bool; MENU_ACTION_COUNT],
}

impl MenuEnabledState {
    /// `true` si l'entrée `id` est active.
    pub fn is_enabled(&self, id: MenuActionId) -> bool {
        self.enabled[id as usize]
    }

    /// Couples (action, active), dans l'ordre de la table.
    pub fn iter(&self) -> impl Iterator<Item = (MenuActionId, bool)> + '_ {
        MenuActionId::all().zip(self.enabled.iter().copied())
    }
}

impl Index<MenuActionId> for MenuEnabledState {
    type Output = bool;

    fn index(&self, id: MenuActionId) -> &bool {
        &self.enabled[id as usize]
    }
}

/// Activation de toutes les entrées pour l'état `ctx`.
pub fn menu_enabled_state(ctx: &MenuContext) -> MenuEnabledState {
    MenuEnabledState {
        enabled: MENU_ACTIONS.map(|action| is_action_enabled(action.id, ctx)),
    }
}

/// Entrée de menu prête à afficher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuItemState {
    /// Action.
    pub id: MenuActionId,
    /// Libellé.
    pub label: &'static str,
    /// Raccourci affiché à droite, s'il y en a un.
    pub shortcut: Option<Shortcut>,
    /// Entrée active (sinon grisée).
    pub enabled: bool,
}

/// Ligne de sous-menu prête à afficher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuRowState {
    /// Entrée.
    Item(MenuItemState),
    /// Séparateur.
    Separator,
}

/// Sous-menu prêt à afficher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuSectionState {
    /// Titre sans `&`.
    pub title: String,
    /// Lettre du mnémonique, en minuscule.
    pub mnemonic: Option<char>,
    /// Lignes.
    pub rows: Vec<MenuRowState>,
}

/// La barre de menus complète pour l'état `ctx` : sous-menus, entrées,
/// raccourcis et grisés. Les entrées de l'autre vue restent présentes,
/// grisées, comme dans le menu natif.
pub fn menu_bar(ctx: &MenuContext) -> Vec<MenuSectionState> {
    let state = menu_enabled_state(ctx);
    MENU_SECTIONS
        .iter()
        .map(|section| MenuSectionState {
            title: section.label(),
            mnemonic: section.mnemonic(),
            rows: section
                .rows
                .iter()
                .map(|row| match *row {
                    MenuRow::Separator => MenuRowState::Separator,
                    MenuRow::Action(id) => MenuRowState::Item(MenuItemState {
                        id,
                        label: id.label(),
                        shortcut: id.shortcut(),
                        enabled: state[id],
                    }),
                })
                .collect(),
        })
        .collect()
}

// --- Raccourcis clavier -----------------------------------------------------

/// Appui de touche utile à la décision, testable sans fenêtre : la touche
/// (`KeyboardEvent.key` ou `Keystroke::key` de GPUI) et les modificateurs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ShortcutKey<'a> {
    /// Nom de la touche : un caractère (`s`, `S`, `,`), `F1`/`f1`, ou un nom
    /// de touche spéciale (`Enter`), toujours ignoré.
    pub key: &'a str,
    /// Ctrl.
    pub ctrl: bool,
    /// Meta / Cmd / Super (vaut Ctrl).
    pub meta: bool,
    /// Maj.
    pub shift: bool,
    /// Alt : aucun raccourci ne l'utilise.
    pub alt: bool,
}

/// Touche normalisée en minuscules : un caractère, ou une touche de
/// fonction `F1`…`F99` ; `None` pour toute autre touche.
fn normalize_key(key: &str) -> Option<String> {
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Some(c.to_lowercase().collect()),
        (Some('F' | 'f'), Some(_)) => {
            let digits = &key[1..];
            (matches!(digits.len(), 1 | 2) && digits.bytes().all(|b| b.is_ascii_digit()))
                .then(|| format!("f{digits}"))
        }
        _ => None,
    }
}

/// Action déclenchée par l'appui `event` dans la vue `view`, ou `None`.
/// Même table que la barre de menus : les deux chemins ne peuvent pas
/// diverger. Les actions de l'autre vue sont ignorées.
pub fn shortcut_to_action(event: &ShortcutKey<'_>, view: MenuView) -> Option<MenuActionId> {
    if event.alt {
        return None;
    }
    let key = normalize_key(event.key)?;
    MENU_ACTIONS
        .iter()
        .filter(|action| action.view.includes(view))
        .find(|action| {
            action
                .shortcut
                .is_some_and(|shortcut| shortcut.matches(event, &key))
        })
        .map(|action| action.id)
}

/// Texte du dialogue « Raccourcis clavier » (Aide, F1), groupé par vue.
pub fn shortcuts_help_text() -> String {
    let groups = [
        ("Partout", ActionView::Any),
        ("Vue d'analyse", ActionView::Analyse),
        ("Vue d'investigation", ActionView::Investigation),
    ];
    let mut lines: Vec<String> = Vec::new();
    for (title, view) in groups {
        let actions: Vec<(&MenuAction, Shortcut)> = MENU_ACTIONS
            .iter()
            .filter(|action| action.view == view)
            .filter_map(|action| action.shortcut.map(|shortcut| (action, shortcut)))
            .collect();
        if actions.is_empty() {
            continue;
        }
        lines.push(format!("{title} :"));
        for (action, shortcut) in actions {
            let label = action.label.strip_suffix('…').unwrap_or(action.label);
            lines.push(format!("  {shortcut} : {label}"));
        }
        lines.push(String::new());
    }
    lines.push(
        "Chaque action est aussi dans la barre de menus et, pour la plupart, dans la barre d'outils."
            .to_owned(),
    );
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    use MenuActionId as A;

    /// État de repos de la vue d'analyse.
    const IDLE: MenuContext = MenuContext {
        view: MenuView::Analyse,
        running: false,
        starting: false,
        importing: false,
        has_data: false,
        panel_open: false,
        loading: false,
        exporting: false,
    };

    /// Appui de touche, sans modificateur.
    fn key(k: &str) -> ShortcutKey<'_> {
        ShortcutKey {
            key: k,
            ..ShortcutKey::default()
        }
    }

    /// Appui avec Ctrl.
    fn ctrl(k: &str) -> ShortcutKey<'_> {
        ShortcutKey {
            ctrl: true,
            ..key(k)
        }
    }

    /// Appui avec Ctrl+Maj.
    fn ctrl_shift(k: &str) -> ShortcutKey<'_> {
        ShortcutKey {
            shift: true,
            ..ctrl(k)
        }
    }

    /// Raccourcis attendus : identifiant, raccourci affiché, frappe gpui.
    const EXPECTED_SHORTCUTS: [(&str, Option<&str>, Option<&str>); MENU_ACTION_COUNT] = [
        ("open-project", Some("Ctrl+Shift+O"), Some("ctrl-shift-o")),
        ("save-project", Some("Ctrl+Shift+S"), Some("ctrl-shift-s")),
        ("open-capture-file", Some("Ctrl+O"), Some("ctrl-o")),
        ("import-labels", Some("Ctrl+Shift+L"), Some("ctrl-shift-l")),
        ("export-csv", Some("Ctrl+S"), Some("ctrl-s")),
        ("export-labels", Some("Ctrl+E"), Some("ctrl-e")),
        ("export-logs", Some("Ctrl+L"), Some("ctrl-l")),
        ("quit", Some("Ctrl+Q"), Some("ctrl-q")),
        ("start-capture", Some("Ctrl+P"), Some("ctrl-p")),
        ("stop-capture", Some("Ctrl+Shift+P"), Some("ctrl-shift-p")),
        ("reset-capture", Some("Ctrl+Shift+R"), Some("ctrl-shift-r")),
        ("open-config", Some("Ctrl+,"), Some("ctrl-,")),
        ("open-filter", Some("Ctrl+F"), Some("ctrl-f")),
        ("open-labels", Some("Ctrl+K"), Some("ctrl-k")),
        ("toggle-graph", Some("Ctrl+G"), Some("ctrl-g")),
        ("open-investigation", Some("Ctrl+I"), Some("ctrl-i")),
        ("back-to-analyse", Some("Ctrl+I"), Some("ctrl-i")),
        ("export-synthesis", Some("Ctrl+S"), Some("ctrl-s")),
        ("export-labels-to-complete", Some("Ctrl+E"), Some("ctrl-e")),
        ("export-filtered-flows", None, None),
        ("reload-survey", None, None),
        ("shortcuts", Some("F1"), Some("f1")),
        ("about", None, None),
        ("changelog", None, None),
    ];

    // --- Table des actions ---

    #[test]
    fn identifiants_uniques_et_libelles_non_vides() {
        let names: HashSet<_> = MENU_ACTIONS.iter().map(|a| a.name).collect();
        assert_eq!(names.len(), MENU_ACTIONS.len());
        for action in &MENU_ACTIONS {
            assert!(!action.label.trim().is_empty(), "{}", action.name);
        }
    }

    #[test]
    fn un_raccourci_n_est_partage_qu_entre_vues_exclusives() {
        let mut by_shortcut: HashMap<Shortcut, Vec<&MenuAction>> = HashMap::new();
        for action in &MENU_ACTIONS {
            if let Some(shortcut) = action.shortcut {
                by_shortcut.entry(shortcut).or_default().push(action);
            }
        }
        for (shortcut, actions) in by_shortcut {
            let views: HashSet<_> = actions.iter().map(|a| a.view).collect();
            assert_eq!(
                views.len(),
                actions.len(),
                "{shortcut} partagé dans une même vue"
            );
            assert!(
                !views.contains(&ActionView::Any) || actions.len() == 1,
                "{shortcut} : une action « any » ne partage pas"
            );
        }
    }

    #[test]
    fn parse_reconnait_les_identifiants_et_rejette_le_reste() {
        assert_eq!(MenuActionId::parse("start-capture"), Some(A::StartCapture));
        assert_eq!(MenuActionId::parse("version"), None);
        assert_eq!(MenuActionId::parse(""), None);
        for id in MenuActionId::all() {
            assert_eq!(MenuActionId::parse(id.as_str()), Some(id));
            assert_eq!(id.to_string(), id.as_str());
        }
    }

    #[test]
    fn repos_tout_actif_sauf_arreter_et_l_autre_vue() {
        let state = menu_enabled_state(&IDLE);
        assert!(state[A::StartCapture]);
        assert!(!state[A::StopCapture]);
        assert!(state[A::OpenProject]);
        assert!(state[A::OpenCaptureFile]);
        assert!(!state[A::ExportSynthesis]);
        assert!(!state[A::BackToAnalyse]);
        assert!(state[A::Quit]);
        assert!(state[A::About]);
    }

    #[test]
    fn capture_en_cours_memes_grises_que_la_barre_d_outils() {
        let state = menu_enabled_state(&MenuContext {
            running: true,
            ..IDLE
        });
        assert!(!state[A::StartCapture]);
        assert!(state[A::StopCapture]);
        assert!(!state[A::OpenConfig]);
        assert!(!state[A::OpenFilter]);
        assert!(!state[A::OpenLabels]);
        assert!(!state[A::ImportLabels]);
        assert!(!state[A::OpenCaptureFile]);
        assert!(!state[A::OpenProject]);
        assert!(!state[A::OpenInvestigation]);
        // Les exports restent possibles pendant la capture, comme les boutons.
        assert!(state[A::ExportCsv]);
        assert!(state[A::SaveProject]);
        assert!(state[A::ResetCapture]);
    }

    #[test]
    fn matrice_remplie_plus_d_ouverture_de_fichier_ni_de_projet() {
        let state = menu_enabled_state(&MenuContext {
            has_data: true,
            ..IDLE
        });
        assert!(!state[A::OpenCaptureFile]);
        assert!(!state[A::OpenProject]);
        assert!(state[A::ImportLabels]);
    }

    #[test]
    fn panneau_ouvert_ni_demarrer_ni_reinitialiser_ni_investigation() {
        let state = menu_enabled_state(&MenuContext {
            panel_open: true,
            ..IDLE
        });
        assert!(!state[A::StartCapture]);
        assert!(!state[A::ResetCapture]);
        assert!(!state[A::OpenInvestigation]);
        assert!(state[A::OpenFilter]);
    }

    #[test]
    fn import_en_cours_verrou_global_sur_fichiers_et_exports() {
        let state = menu_enabled_state(&MenuContext {
            importing: true,
            ..IDLE
        });
        for id in [
            A::OpenProject,
            A::SaveProject,
            A::ExportCsv,
            A::ExportLabels,
            A::ExportLogs,
            A::StartCapture,
            A::ResetCapture,
        ] {
            assert!(!state[id], "{id}");
        }
        assert!(state[A::Quit]);
    }

    #[test]
    fn investigation_exports_grises_pendant_chargement_ou_export() {
        let inv = MenuContext {
            view: MenuView::Investigation,
            ..IDLE
        };
        let state = menu_enabled_state(&inv);
        assert!(state[A::ExportSynthesis]);
        assert!(state[A::ExportLabelsToComplete]);
        assert!(state[A::ReloadSurvey]);
        assert!(state[A::BackToAnalyse]);
        assert!(!state[A::ExportCsv]);
        assert!(!state[A::StartCapture]);
        let state = menu_enabled_state(&MenuContext {
            loading: true,
            ..inv
        });
        assert!(!state[A::ExportSynthesis]);
        assert!(!state[A::ReloadSurvey]);
        let state = menu_enabled_state(&MenuContext {
            exporting: true,
            ..inv
        });
        assert!(!state[A::ExportLabelsToComplete]);
        assert!(!state[A::ExportFilteredFlows]);
        assert!(state[A::ReloadSurvey]);
    }

    #[test]
    fn raccourcis_de_la_vue_d_analyse() {
        let analyse = MenuView::Analyse;
        let cases = [
            (ctrl("s"), A::ExportCsv),
            (ctrl_shift("S"), A::SaveProject),
            (ctrl("o"), A::OpenCaptureFile),
            (ctrl_shift("O"), A::OpenProject),
            (ctrl("p"), A::StartCapture),
            (ctrl_shift("P"), A::StopCapture),
            (ctrl_shift("R"), A::ResetCapture),
            (ctrl(","), A::OpenConfig),
            (ctrl("f"), A::OpenFilter),
            (ctrl("l"), A::ExportLogs),
            (ctrl_shift("L"), A::ImportLabels),
            (ctrl("i"), A::OpenInvestigation),
            (ctrl("q"), A::Quit),
            (key("F1"), A::Shortcuts),
        ];
        for (event, expected) in cases {
            assert_eq!(
                shortcut_to_action(&event, analyse),
                Some(expected),
                "{event:?}"
            );
        }
        // Cmd vaut Ctrl.
        let meta_s = ShortcutKey {
            meta: true,
            ..key("s")
        };
        assert_eq!(shortcut_to_action(&meta_s, analyse), Some(A::ExportCsv));
    }

    #[test]
    fn la_meme_touche_change_d_action_selon_la_vue() {
        let inv = MenuView::Investigation;
        assert_eq!(
            shortcut_to_action(&ctrl("s"), inv),
            Some(A::ExportSynthesis)
        );
        assert_eq!(
            shortcut_to_action(&ctrl("e"), inv),
            Some(A::ExportLabelsToComplete)
        );
        assert_eq!(shortcut_to_action(&ctrl("i"), inv), Some(A::BackToAnalyse));
        assert_eq!(shortcut_to_action(&ctrl("p"), inv), None);
    }

    #[test]
    fn sans_ctrl_avec_alt_ou_touche_inconnue_rien() {
        let analyse = MenuView::Analyse;
        assert_eq!(shortcut_to_action(&key("s"), analyse), None);
        let ctrl_alt_s = ShortcutKey {
            alt: true,
            ..ctrl("s")
        };
        assert_eq!(shortcut_to_action(&ctrl_alt_s, analyse), None);
        // Ctrl+R n'est lié à aucune action.
        assert_eq!(shortcut_to_action(&ctrl("r"), analyse), None);
        assert_eq!(shortcut_to_action(&ctrl("Enter"), analyse), None);
        assert_eq!(shortcut_to_action(&key("F5"), analyse), None);
    }

    #[test]
    fn aide_liste_chaque_raccourci_avec_son_libelle() {
        let text = shortcuts_help_text();
        for action in &MENU_ACTIONS {
            let Some(shortcut) = action.shortcut else {
                continue;
            };
            assert!(text.contains(&shortcut.display()), "{shortcut}");
            let label = action.label.strip_suffix('…').unwrap_or(action.label);
            assert!(text.contains(label), "{label}");
        }
        assert!(text.contains("Vue d'investigation :"));
    }

    // --- Sous-menus ---

    #[test]
    fn identifiants_du_menu_uniques() {
        let ids: Vec<_> = MENU_SECTIONS
            .iter()
            .flat_map(MenuSection::actions)
            .collect();
        let unique: HashSet<_> = ids.iter().collect();
        assert_eq!(ids.len(), unique.len(), "identifiants dupliqués : {ids:?}");
    }

    #[test]
    fn chaque_action_est_rangee_dans_un_sous_menu() {
        // La table des actions et celle des sous-menus couvrent exactement
        // les mêmes identifiants.
        let in_menu: HashSet<_> = MENU_SECTIONS
            .iter()
            .flat_map(MenuSection::actions)
            .collect();
        let in_table: HashSet<_> = MenuActionId::all().collect();
        assert_eq!(in_menu, in_table);
    }

    #[test]
    fn raccourcis_attendus() {
        // Chaque action donne le raccourci affiché et la frappe gpui attendus.
        for (action, (name, display, keystroke)) in MENU_ACTIONS.iter().zip(EXPECTED_SHORTCUTS) {
            assert_eq!(action.name, name);
            let shortcut = action.shortcut;
            assert_eq!(shortcut.map(|s| s.display()).as_deref(), display, "{name}");
            assert_eq!(
                shortcut.map(|s| s.to_gpui_keystroke()).as_deref(),
                keystroke,
                "{name} : frappe gpui"
            );
        }
    }

    #[test]
    fn meme_raccourci_seulement_entre_vues_mutuellement_exclusives() {
        // Deux entrées peuvent partager un raccourci seulement si elles ne
        // sont jamais actives en même temps (analyse / investigation).
        let allowed: HashSet<(MenuActionId, MenuActionId)> = [
            (A::ExportCsv, A::ExportSynthesis),
            (A::ExportLabels, A::ExportLabelsToComplete),
            (A::OpenInvestigation, A::BackToAnalyse),
        ]
        .into_iter()
        .collect();
        let mut seen: HashMap<Shortcut, MenuActionId> = HashMap::new();
        for id in MENU_SECTIONS.iter().flat_map(MenuSection::actions) {
            if let Some(shortcut) = id.shortcut()
                && let Some(other) = seen.insert(shortcut, id)
            {
                assert!(
                    allowed.contains(&(other, id)),
                    "raccourci {shortcut} partagé par {other} et {id} hors paires autorisées"
                );
            }
        }
    }

    // --- Cohérence de la table ---

    #[test]
    fn table_dans_l_ordre_des_variantes() {
        for (index, action) in MENU_ACTIONS.iter().enumerate() {
            assert_eq!(action.id as usize, index, "{}", action.name);
            assert_eq!(action.id.action(), action);
        }
        assert_eq!(A::Changelog as usize + 1, MENU_ACTION_COUNT);
    }

    #[test]
    fn ordre_du_menu_identique_a_la_table() {
        // Les sous-menus suivent l'ordre de la table.
        let in_menu: Vec<_> = MENU_SECTIONS
            .iter()
            .flat_map(MenuSection::actions)
            .collect();
        let in_table: Vec<_> = MenuActionId::all().collect();
        assert_eq!(in_menu, in_table);
    }

    #[test]
    fn titres_et_mnemoniques_des_sous_menus() {
        let titles: Vec<_> = MENU_SECTIONS
            .iter()
            .map(|s| (s.label(), s.mnemonic()))
            .collect();
        assert_eq!(
            titles,
            [
                ("Fichier".to_owned(), Some('f')),
                ("Capture".to_owned(), Some('c')),
                ("Affichage".to_owned(), Some('a')),
                ("Investigation".to_owned(), Some('i')),
                ("Aide".to_owned(), Some('i')),
            ]
        );
    }

    #[test]
    fn separateurs_jamais_en_bord_ni_doubles() {
        for section in MENU_SECTIONS {
            let rows = section.rows;
            assert_ne!(rows.first(), Some(&SEP), "{}", section.title);
            assert_ne!(rows.last(), Some(&SEP), "{}", section.title);
            assert!(
                !rows.windows(2).any(|pair| pair == [SEP, SEP]),
                "{}",
                section.title
            );
        }
    }

    #[test]
    fn chaque_raccourci_affiche_declenche_son_action() {
        for action in &MENU_ACTIONS {
            let Some(shortcut) = action.shortcut else {
                continue;
            };
            let key_name = match shortcut.key {
                Key::Char(c) => c.to_string(),
                Key::Function(n) => format!("F{n}"),
            };
            let event = ShortcutKey {
                key: &key_name,
                ctrl: shortcut.ctrl,
                shift: shortcut.shift,
                ..ShortcutKey::default()
            };
            let views = match action.view {
                ActionView::Any => vec![MenuView::Analyse, MenuView::Investigation],
                ActionView::Analyse => vec![MenuView::Analyse],
                ActionView::Investigation => vec![MenuView::Investigation],
            };
            for view in views {
                assert_eq!(
                    shortcut_to_action(&event, view),
                    Some(action.id),
                    "{shortcut}"
                );
            }
        }
    }

    #[test]
    fn touches_gpui_en_minuscules_reconnues() {
        // GPUI donne `f1` et la lettre en minuscule même avec Maj.
        assert_eq!(
            shortcut_to_action(&key("f1"), MenuView::Investigation),
            Some(A::Shortcuts)
        );
        assert_eq!(
            shortcut_to_action(&ctrl_shift("o"), MenuView::Analyse),
            Some(A::OpenProject)
        );
        assert_eq!(normalize_key("F12").as_deref(), Some("f12"));
        assert_eq!(normalize_key("F123"), None);
        assert_eq!(normalize_key("Fx"), None);
        assert_eq!(normalize_key("é").as_deref(), Some("é"));
        assert_eq!(normalize_key(""), None);
    }

    #[test]
    fn barre_de_menus_grisee_selon_l_etat() {
        let bar = menu_bar(&MenuContext {
            running: true,
            ..IDLE
        });
        assert_eq!(bar.len(), MENU_SECTIONS.len());
        let capture = &bar[1];
        assert_eq!(capture.title, "Capture");
        assert_eq!(capture.rows.len(), 5);
        assert_eq!(capture.rows[3], MenuRowState::Separator);
        let MenuRowState::Item(stop) = capture.rows[1] else {
            panic!("entrée attendue");
        };
        assert_eq!(stop.id, A::StopCapture);
        assert_eq!(stop.label, "Arrêter la capture");
        assert_eq!(
            stop.shortcut.map(|s| s.to_gpui_keystroke()).as_deref(),
            Some("ctrl-shift-p")
        );
        assert!(stop.enabled);
        let state = menu_enabled_state(&MenuContext {
            running: true,
            ..IDLE
        });
        for section in &bar {
            for row in &section.rows {
                if let MenuRowState::Item(item) = row {
                    assert_eq!(item.enabled, state[item.id], "{}", item.id);
                }
            }
        }
    }

    #[test]
    fn texte_d_aide_attendu() {
        let expected = "\
Partout :
  Ctrl+Q : Quitter
  F1 : Raccourcis clavier

Vue d'analyse :
  Ctrl+Shift+O : Ouvrir un projet (.lunar)
  Ctrl+Shift+S : Enregistrer le projet (.lunar)
  Ctrl+O : Ouvrir un fichier PCAP ou une matrice de flux
  Ctrl+Shift+L : Importer un fichier de labels
  Ctrl+S : Exporter la matrice de flux (CSV)
  Ctrl+E : Exporter les labels
  Ctrl+L : Exporter les logs (archive de support)
  Ctrl+P : Démarrer la capture
  Ctrl+Shift+P : Arrêter la capture
  Ctrl+Shift+R : Réinitialiser le relevé
  Ctrl+, : Configuration de la capture
  Ctrl+F : Filtrer
  Ctrl+K : Gérer les labels
  Ctrl+G : Afficher ou masquer le graphe
  Ctrl+I : Vue d'investigation

Vue d'investigation :
  Ctrl+I : Retour à l'analyse
  Ctrl+S : Exporter la synthèse (HTML + CSV)
  Ctrl+E : Exporter les labels à compléter

Chaque action est aussi dans la barre de menus et, pour la plupart, dans la barre d'outils.";
        assert_eq!(shortcuts_help_text(), expected);
    }
}
