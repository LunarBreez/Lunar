//! Thème sombre de LUNAR : un seul jeu de jetons nommés pour toutes les vues.
//!
//! Chaque jeton documente son rôle et sa valeur ; « dérivé » signale une
//! valeur construite à partir d'un autre jeton.
//!
//! Choix d'unification :
//! - les modales et panneaux partagent le même fond (`#1e1e2e`) et la même
//!   bordure ;
//! - le sélecteur d'interface est un champ sombre identique aux autres champs ;
//! - un seul bleu de mise au point sert à tous les contrôles ;
//! - le survol des lignes de tableau ne se confond pas avec leur séparateur.

use std::sync::LazyLock;

use gpui::{App, BoxShadow, Global, Hsla, Pixels, SharedString, point, px, rgb, rgba};

/// Convertit une couleur `0xRRGGBB` en [`Hsla`].
fn hex(value: u32) -> Hsla {
    rgb(value).into()
}

/// Convertit une couleur `0xRRGGBBAA` en [`Hsla`].
fn hexa(value: u32) -> Hsla {
    rgba(value).into()
}

/// Jetons du thème : couleurs, espacements, rayons, tailles de police.
///
/// Installé comme global gpui par [`init`] ; lu par [`ActiveTheme::theme`].
#[derive(Clone, Debug)]
pub struct Theme {
    // --- Fonds -----------------------------------------------------------
    /// Fond de l'application (`#1a1a1a`).
    pub bg_app: Hsla,
    /// Barre d'outils et barre de menus (`#070416`).
    pub bg_bar: Hsla,
    /// Barre d'état (`#243452`).
    pub bg_status_bar: Hsla,
    /// Modales et panneaux (`#1e1e2e`).
    pub bg_surface: Hsla,
    /// Cartes posées sur une surface, listes déroulantes et menus (`#252535`).
    pub bg_raised: Hsla,
    /// Fond des champs de saisie et listes (`#12121e`).
    pub bg_field: Hsla,
    /// Fond du graphe (`#111`).
    pub bg_graph: Hsla,
    /// Fond des vues de contenu tabulaire (`#0b0a1f`).
    pub bg_content: Hsla,
    /// Voile derrière une modale (noir, alpha .7).
    pub bg_backdrop: Hsla,
    /// Bulle d'aide (`#111827`).
    pub bg_tooltip: Hsla,
    /// Survol d'un élément de liste ou de menu (`#2a2a50`).
    pub bg_hover: Hsla,
    /// Survol d'un bouton discret de barre d'outils (`#3f4758`).
    pub bg_ghost_hover: Hsla,

    // --- Texte -----------------------------------------------------------
    /// Texte principal (`#f5f5f5`).
    pub text: Hsla,
    /// Texte secondaire : en-têtes de liste, notes de légende (`#aaa`).
    pub text_muted: Hsla,
    /// Titres de modale (`#c0c0e0`).
    pub text_heading: Hsla,
    /// Texte saisi dans un champ (`#d0d0f0`).
    pub text_field: Hsla,
    /// Texte indicatif d'un champ vide (`#5a6b82`).
    pub text_placeholder: Hsla,
    /// Texte sur fond d'accent ou de bouton plein (`#fff`).
    pub text_on_fill: Hsla,
    /// Bleu clair de mise en valeur : en-têtes de tableau, onglet actif,
    /// valeur de KPI (`#84c3f7`).
    pub text_highlight: Hsla,

    // --- Bordures --------------------------------------------------------
    /// Bordure ordinaire : modales, champs, boutons (`#2d2d50`).
    pub border: Hsla,
    /// Bordure au survol (`#404070`).
    pub border_strong: Hsla,
    /// Séparateur de barres et d'onglets (`#252526`).
    pub border_divider: Hsla,
    /// Bordure des cartes et tuiles KPI (`#262a44`).
    pub border_card: Hsla,
    /// Bordure de bulle d'aide (`#555`).
    pub border_tooltip: Hsla,

    // --- Accents ---------------------------------------------------------
    /// Accent turquoise : état basculé d'un bouton (`#1de9b6`).
    pub accent: Hsla,
    /// Fond d'un bouton basculé (dérivé de `accent`, alpha .14).
    pub accent_soft: Hsla,
    /// Bouton principal (`#3a6eee`).
    pub primary: Hsla,
    /// Bouton principal survolé (`#4a7eff`).
    pub primary_hover: Hsla,
    /// Anneau de mise au point clavier et case cochée (`#4a7cff`).
    pub focus_ring: Hsla,

    // --- Boutons secondaires ---------------------------------------------
    /// Fond (`#2a2a45`).
    pub button_bg: Hsla,
    /// Fond survolé (`#32325a`).
    pub button_bg_hover: Hsla,
    /// Texte (`#a0a0cc`).
    pub button_text: Hsla,
    /// Texte survolé (`#c0c0e8`).
    pub button_text_hover: Hsla,

    // --- Couleurs sémantiques --------------------------------------------
    /// Danger, fond plein (`#d8392b`).
    pub danger: Hsla,
    /// Danger survolé (dérivé de `danger`, éclairci).
    pub danger_hover: Hsla,
    /// Texte d'erreur et champ invalide (`#ff6060`).
    pub danger_text: Hsla,
    /// Avertissement : filtre en attente, badge manuel (`#f0a030`).
    pub warning: Hsla,
    /// Succès (`#48bb78`).
    pub success: Hsla,
    /// Information : libellé du filtre actif (`#4a7cff`).
    pub info: Hsla,

    // --- Notifications --------------------------------------------------
    /// Fond d'une notification d'information (`#26333a`).
    pub toast_info_bg: Hsla,
    /// Bordure d'information (`#2596be`).
    pub toast_info_border: Hsla,
    /// Fond d'un succès (dérivé de `success`, assombri).
    pub toast_success_bg: Hsla,
    /// Fond d'un avertissement (`#3a2a12`).
    pub toast_warning_bg: Hsla,
    /// Bordure d'avertissement (`#c98a2b`).
    pub toast_warning_border: Hsla,
    /// Fond d'une erreur (`#3a2630`).
    pub toast_error_bg: Hsla,

    // --- Tableaux ----------------------------------------------------------
    /// Fond du tableau (`#0b0a1f`).
    pub table_bg: Hsla,
    /// Fond de l'en-tête (`#12102b`).
    pub table_header_bg: Hsla,
    /// Texte de l'en-tête (`#84c3f7`).
    pub table_header_text: Hsla,
    /// Bordure basse de l'en-tête (`#262a44`).
    pub table_header_border: Hsla,
    /// Séparateur de lignes (`#171531`).
    pub table_row_border: Hsla,
    /// Ligne survolée (dérivé du séparateur `#171531`, éclairci pour que le
    /// séparateur reste visible au survol).
    pub table_row_hover: Hsla,
    /// Ligne sélectionnée (dérivé de `#84c3f7`, alpha .16).
    pub table_row_selected: Hsla,
    /// Ligne en alerte (`#b5651d`, alpha .12).
    pub table_row_warn: Hsla,

    // --- Onglets -----------------------------------------------------------
    /// Onglet inactif (`#9fb4cc`).
    pub tab_text: Hsla,
    /// Onglet actif, texte et soulignement (`#84c3f7`).
    pub tab_active: Hsla,

    // --- Tuiles KPI --------------------------------------------------------
    /// Fond (`#12102b`).
    pub kpi_bg: Hsla,
    /// Bordure en alerte (`#b5651d`).
    pub kpi_warn_border: Hsla,
    /// Valeur en alerte (`#ffb74d`).
    pub kpi_warn_value: Hsla,

    // --- Progression et défilement ----------------------------------------
    /// Piste de progression (blanc, alpha .15).
    pub progress_track: Hsla,
    /// Remplissage (`#4299e1`).
    pub progress_fill: Hsla,
    /// Poignée de défilement (blanc, alpha .18).
    pub scrollbar_thumb: Hsla,
    /// Poignée survolée ou saisie (blanc, alpha .32).
    pub scrollbar_thumb_hover: Hsla,

    // --- Espacements (px) --------------------------------------------------
    /// 4 px : écart icône/texte, marge de bouton d'icône.
    pub space_xs: Pixels,
    /// 6 px : marge verticale des champs.
    pub space_sm: Pixels,
    /// 8 px : écart entre boutons de la barre d'outils.
    pub space_md: Pixels,
    /// 12 px : écart entre blocs de modale.
    pub space_lg: Pixels,
    /// 16 px : marge intérieure de panneau.
    pub space_xl: Pixels,
    /// 24 px : marge latérale de modale.
    pub space_xxl: Pixels,

    // --- Rayons (px) -------------------------------------------------------
    /// 4 px : boutons d'icône.
    pub radius_sm: Pixels,
    /// 6 px : champs, boutons.
    pub radius_md: Pixels,
    /// 8 px : cartes, KPI.
    pub radius_lg: Pixels,
    /// 10 px : modales.
    pub radius_xl: Pixels,

    // --- Typographie -------------------------------------------------------
    /// Police d'interface (Segoe UI sous Windows, Ubuntu sous Linux).
    pub font_family: SharedString,
    /// Police à chasse fixe : adresses, compteurs, expressions BPF.
    pub font_mono: SharedString,
    /// 11 px : libellés de KPI, badges.
    pub text_xs: Pixels,
    /// 12 px : barre d'état, bulles d'aide.
    pub text_sm: Pixels,
    /// 13 px : corps, boutons, champs, tableaux.
    pub text_md: Pixels,
    /// 16 px : titres de modale et de panneau.
    pub text_lg: Pixels,
    /// 20 px : valeur de KPI.
    pub text_kpi: Pixels,

    // --- Dimensions --------------------------------------------------------
    /// Hauteur des boutons et champs (13 px + 2 × 7 px + bordures).
    pub control_height: Pixels,
    /// Hauteur d'une ligne de tableau (13 px + 2 × 6 px, arrondi).
    pub row_height: Pixels,
    /// Hauteur de la barre de menus, plus basse que la barre d'outils
    /// au-dessus de laquelle elle s'ajoute.
    pub menu_bar_height: Pixels,
    /// Opacité d'un contrôle désactivé (.4).
    pub disabled_opacity: f32,
}

impl Theme {
    /// Thème sombre de LUNAR.
    pub fn lunar() -> Self {
        Self {
            bg_app: hex(0x1a1a1a),
            bg_bar: hex(0x070416),
            bg_status_bar: hex(0x243452),
            bg_surface: hex(0x1e1e2e),
            bg_raised: hex(0x252535),
            bg_field: hex(0x12121e),
            bg_graph: hex(0x111111),
            bg_content: hex(0x0b0a1f),
            bg_backdrop: hexa(0x000000b3),
            bg_tooltip: hex(0x111827),
            bg_hover: hex(0x2a2a50),
            bg_ghost_hover: hex(0x3f4758),

            text: hex(0xf5f5f5),
            text_muted: hex(0xaaaaaa),
            text_heading: hex(0xc0c0e0),
            text_field: hex(0xd0d0f0),
            text_placeholder: hex(0x5a6b82),
            text_on_fill: hex(0xffffff),
            text_highlight: hex(0x84c3f7),

            border: hex(0x2d2d50),
            border_strong: hex(0x404070),
            border_divider: hex(0x252526),
            border_card: hex(0x262a44),
            border_tooltip: hex(0x555555),

            accent: hex(0x1de9b6),
            accent_soft: hexa(0x1de9b624),
            primary: hex(0x3a6eee),
            primary_hover: hex(0x4a7eff),
            focus_ring: hex(0x4a7cff),

            button_bg: hex(0x2a2a45),
            button_bg_hover: hex(0x32325a),
            button_text: hex(0xa0a0cc),
            button_text_hover: hex(0xc0c0e8),

            danger: hex(0xd8392b),
            danger_hover: hex(0xe5513f),
            danger_text: hex(0xff6060),
            warning: hex(0xf0a030),
            success: hex(0x48bb78),
            info: hex(0x4a7cff),

            toast_info_bg: hex(0x26333a),
            toast_info_border: hex(0x2596be),
            toast_success_bg: hex(0x1d3326),
            toast_warning_bg: hex(0x3a2a12),
            toast_warning_border: hex(0xc98a2b),
            toast_error_bg: hex(0x3a2630),

            table_bg: hex(0x0b0a1f),
            table_header_bg: hex(0x12102b),
            table_header_text: hex(0x84c3f7),
            table_header_border: hex(0x262a44),
            table_row_border: hex(0x171531),
            table_row_hover: hex(0x1f1c40),
            table_row_selected: hexa(0x84c3f729),
            table_row_warn: hexa(0xb5651d1f),

            tab_text: hex(0x9fb4cc),
            tab_active: hex(0x84c3f7),

            kpi_bg: hex(0x12102b),
            kpi_warn_border: hex(0xb5651d),
            kpi_warn_value: hex(0xffb74d),

            progress_track: hexa(0xffffff26),
            progress_fill: hex(0x4299e1),
            scrollbar_thumb: hexa(0xffffff2e),
            scrollbar_thumb_hover: hexa(0xffffff52),

            space_xs: px(4.),
            space_sm: px(6.),
            space_md: px(8.),
            space_lg: px(12.),
            space_xl: px(16.),
            space_xxl: px(24.),

            radius_sm: px(4.),
            radius_md: px(6.),
            radius_lg: px(8.),
            radius_xl: px(10.),

            font_family: default_ui_font().into(),
            font_mono: default_mono_font().into(),
            text_xs: px(11.),
            text_sm: px(12.),
            text_md: px(13.),
            text_lg: px(16.),
            text_kpi: px(20.),

            control_height: px(30.),
            row_height: px(26.),
            menu_bar_height: px(28.),
            disabled_opacity: 0.4,
        }
    }

    /// Ombre des listes déroulantes, menus et bulles : celle des modales, avec
    /// un flou réduit.
    pub fn shadow_popover(&self) -> Vec<BoxShadow> {
        vec![BoxShadow {
            color: hexa(0x00000080),
            offset: point(px(0.), px(6.)),
            blur_radius: px(18.),
            spread_radius: px(0.),
        }]
    }

    /// Anneau de mise au point au clavier : halo de 2 px autour de la
    /// bordure, visible aussi sur les boutons pleins (le bleu de
    /// `focus_ring` se confond avec celui de `primary`).
    pub fn shadow_focus(&self) -> Vec<BoxShadow> {
        vec![BoxShadow {
            color: self.focus_ring.opacity(0.7),
            offset: point(px(0.), px(0.)),
            // Flou nul : gpui 0.2.2 ne dessine alors pas l'ombre.
            blur_radius: px(1.),
            spread_radius: px(2.),
        }]
    }

    /// Ombre des modales (`0 8px 32px`, noir alpha .5).
    pub fn shadow_modal(&self) -> Vec<BoxShadow> {
        vec![BoxShadow {
            color: hexa(0x00000080),
            offset: point(px(0.), px(8.)),
            blur_radius: px(32.),
            spread_radius: px(0.),
        }]
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::lunar()
    }
}

impl Global for Theme {}

/// Police d'interface de la plateforme.
fn default_ui_font() -> &'static str {
    if cfg!(target_os = "windows") {
        "Segoe UI"
    } else {
        "Ubuntu"
    }
}

/// Police à chasse fixe de la plateforme.
fn default_mono_font() -> &'static str {
    if cfg!(target_os = "windows") {
        "Consolas"
    } else {
        "DejaVu Sans Mono"
    }
}

/// Thème de repli quand [`init`] n'a pas été appelé (essais unitaires).
static FALLBACK: LazyLock<Theme> = LazyLock::new(Theme::lunar);

/// Installe le thème LUNAR comme global gpui.
pub fn init(cx: &mut App) {
    cx.set_global(Theme::lunar());
}

/// Accès au thème courant depuis un contexte gpui.
pub trait ActiveTheme {
    /// Thème installé par [`init`], ou le thème LUNAR par défaut.
    fn theme(&self) -> &Theme;
}

impl ActiveTheme for App {
    fn theme(&self) -> &Theme {
        self.try_global::<Theme>().unwrap_or(&FALLBACK)
    }
}
