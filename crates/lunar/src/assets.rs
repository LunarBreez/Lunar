//! Ressources embarquées dans le binaire : icônes SVG et images.
//!
//! Les fichiers de `assets/` sont inclus à la compilation (`include_bytes!`)
//! et servis à gpui par [`Assets`], sans accès disque à l'exécution.
//! Provenance et licences des icônes : `assets/icons/SOURCE.md`.

use std::borrow::Cow;

use gpui::{AssetSource, SharedString, Svg, prelude::*, px, svg};

/// Déclare la table des fichiers embarqués, chemin relatif à `assets/`.
macro_rules! embedded_files {
    ($($path:literal),* $(,)?) => {
        /// Fichiers embarqués : (chemin relatif à `assets/`, contenu).
        const FILES: &[(&str, &[u8])] = &[
            $(($path, include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/", $path)))),*
        ];
    };
}

embedded_files![
    "icons/alarm.svg",
    "icons/alert-circle.svg",
    "icons/alert-triangle.svg",
    "icons/arrow-back-up.svg",
    "icons/arrow-down.svg",
    "icons/arrow-left.svg",
    "icons/arrows-sort.svg",
    "icons/arrow-up.svg",
    "icons/ban.svg",
    "icons/chart-bar.svg",
    "icons/check.svg",
    "icons/chevron-down.svg",
    "icons/chevron-left.svg",
    "icons/chevron-right.svg",
    "icons/chevron-up.svg",
    "icons/circle-check.svg",
    "icons/circle-x.svg",
    "icons/clock.svg",
    "icons/copy.svg",
    "icons/device-floppy.svg",
    "icons/dots-vertical.svg",
    "icons/download.svg",
    "icons/eye-off.svg",
    "icons/eye.svg",
    "icons/file-export.svg",
    "icons/file-import.svg",
    "icons/file-spreadsheet.svg",
    "icons/filter.svg",
    "icons/flask.svg",
    "icons/focus-centered.svg",
    "icons/folder-open.svg",
    "icons/help-circle.svg",
    "icons/inbox.svg",
    "icons/info-circle.svg",
    "icons/layout-sidebar-right.svg",
    "icons/lock.svg",
    "icons/logout.svg",
    "icons/mdi--gear.svg",
    "icons/microscope.svg",
    "icons/minus.svg",
    "icons/network.svg",
    "icons/notebook.svg",
    "icons/pencil.svg",
    "icons/player-play.svg",
    "icons/player-stop.svg",
    "icons/plus.svg",
    "icons/puzzle.svg",
    "icons/refresh.svg",
    "icons/scale.svg",
    "icons/search.svg",
    "icons/settings.svg",
    "icons/TablerCpu.svg",
    "icons/table.svg",
    "icons/tags.svg",
    "icons/tag.svg",
    "icons/trash.svg",
    "icons/upload.svg",
    "icons/x.svg",
    "icons/zoom-in.svg",
    "icons/zoom-out.svg",
    "images/logo-256.png",
];

/// Logo LUNAR, PNG 256 × 256 (dessiné pour LUNAR, voir `assets/icons/SOURCE.md`).
pub const LOGO_PNG: &str = "images/logo-256.png";

/// Source de ressources gpui servant les fichiers embarqués.
///
/// À passer à `Application::new().with_assets(Assets)`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Assets;

impl Assets {
    /// Contenu d'un fichier embarqué, chemin relatif à `assets/`.
    pub fn get(path: &str) -> Option<&'static [u8]> {
        FILES
            .iter()
            .find(|(name, _)| *name == path)
            .map(|(_, data)| *data)
    }
}

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        Ok(Self::get(path).map(Cow::Borrowed))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(FILES
            .iter()
            .filter(|(name, _)| name.starts_with(path))
            .map(|(name, _)| SharedString::new_static(name))
            .collect())
    }
}

/// Icônes de l'interface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    /// Démarrer la capture.
    Play,
    /// Arrêter la capture.
    Stop,
    /// Réinitialiser, rafraîchir.
    Refresh,
    /// Configuration, version Tabler (`settings`).
    Settings,
    /// Configuration, roue dentée (`mdi--gear.svg`).
    Gear,
    /// Sauvegarder la matrice de flux en CSV.
    FileExport,
    /// Enregistrer le projet `.lunar`.
    Save,
    /// Ouvrir un projet `.lunar`.
    FolderOpen,
    /// Exporter les labels.
    Tag,
    /// Gérer les labels.
    Tags,
    /// Ouvrir un PCAP ou une matrice.
    FileImport,
    /// Importer un fichier de labels.
    FileSpreadsheet,
    /// Quitter.
    Quit,
    /// Exporter les journaux.
    Logs,
    /// Filtrer.
    Filter,
    /// Vue d'investigation.
    Microscope,
    /// Durée d'exécution.
    Clock,
    /// Trames reçues.
    Inbox,
    /// Trames perdues côté noyau.
    CircleX,
    /// Trames perdues au niveau de l'interface.
    Ban,
    /// Avertissement, pertes côté application.
    Warning,
    /// Trames illisibles par le parseur.
    Puzzle,
    /// Occupation du canal.
    Alarm,
    /// Filtre actif dans la barre d'état.
    Flask,
    /// Fermer.
    Close,
    /// Valider.
    Check,
    /// Chevron bas (listes déroulantes).
    ChevronDown,
    /// Chevron haut.
    ChevronUp,
    /// Chevron droit (sous-menus).
    ChevronRight,
    /// Chevron gauche.
    ChevronLeft,
    /// Tri croissant.
    ArrowUp,
    /// Tri décroissant.
    ArrowDown,
    /// Colonne triable, sans tri actif.
    ArrowsSort,
    /// Information.
    Info,
    /// Succès.
    CircleCheck,
    /// Erreur.
    AlertCircle,
    /// Aide (bouton « ? » du panneau de configuration).
    Help,
    /// Rechercher.
    Search,
    /// Ajouter.
    Plus,
    /// Retirer.
    Minus,
    /// Afficher.
    Eye,
    /// Masquer.
    EyeOff,
    /// Éditer.
    Pencil,
    /// Supprimer.
    Trash,
    /// Télécharger, exporter une image.
    Download,
    /// Téléverser, importer.
    Upload,
    /// Annuler une édition.
    Undo,
    /// Retour (« ← Retour » de l'investigation).
    ArrowLeft,
    /// Arbitrer les conflits.
    Scale,
    /// Copier.
    Copy,
    /// Verrouillé.
    Lock,
    /// Réseau, interface de capture.
    Network,
    /// Processeur, interface physique (`TablerCpu.svg`).
    Cpu,
    /// Tableau.
    Table,
    /// Graphique.
    ChartBar,
    /// Panneau latéral.
    SidebarRight,
    /// Zoom avant.
    ZoomIn,
    /// Zoom arrière.
    ZoomOut,
    /// Recentrer.
    FocusCentered,
    /// Menu d'actions.
    DotsVertical,
}

impl Icon {
    /// Toutes les icônes, pour la galerie.
    pub const ALL: &'static [Icon] = &[
        Icon::Play,
        Icon::Stop,
        Icon::Refresh,
        Icon::Settings,
        Icon::Gear,
        Icon::FileExport,
        Icon::Save,
        Icon::FolderOpen,
        Icon::Tag,
        Icon::Tags,
        Icon::FileImport,
        Icon::FileSpreadsheet,
        Icon::Quit,
        Icon::Logs,
        Icon::Filter,
        Icon::Microscope,
        Icon::Clock,
        Icon::Inbox,
        Icon::CircleX,
        Icon::Ban,
        Icon::Warning,
        Icon::Puzzle,
        Icon::Alarm,
        Icon::Flask,
        Icon::Close,
        Icon::Check,
        Icon::ChevronDown,
        Icon::ChevronUp,
        Icon::ChevronRight,
        Icon::ChevronLeft,
        Icon::ArrowUp,
        Icon::ArrowDown,
        Icon::ArrowsSort,
        Icon::Info,
        Icon::CircleCheck,
        Icon::AlertCircle,
        Icon::Help,
        Icon::Search,
        Icon::Plus,
        Icon::Minus,
        Icon::Eye,
        Icon::EyeOff,
        Icon::Pencil,
        Icon::Trash,
        Icon::Download,
        Icon::Upload,
        Icon::Undo,
        Icon::ArrowLeft,
        Icon::Scale,
        Icon::Copy,
        Icon::Lock,
        Icon::Network,
        Icon::Cpu,
        Icon::Table,
        Icon::ChartBar,
        Icon::SidebarRight,
        Icon::ZoomIn,
        Icon::ZoomOut,
        Icon::FocusCentered,
        Icon::DotsVertical,
    ];

    /// Chemin de l'icône dans la source de ressources.
    pub fn path(self) -> &'static str {
        match self {
            Icon::Play => "icons/player-play.svg",
            Icon::Stop => "icons/player-stop.svg",
            Icon::Refresh => "icons/refresh.svg",
            Icon::Settings => "icons/settings.svg",
            Icon::Gear => "icons/mdi--gear.svg",
            Icon::FileExport => "icons/file-export.svg",
            Icon::Save => "icons/device-floppy.svg",
            Icon::FolderOpen => "icons/folder-open.svg",
            Icon::Tag => "icons/tag.svg",
            Icon::Tags => "icons/tags.svg",
            Icon::FileImport => "icons/file-import.svg",
            Icon::FileSpreadsheet => "icons/file-spreadsheet.svg",
            Icon::Quit => "icons/logout.svg",
            Icon::Logs => "icons/notebook.svg",
            Icon::Filter => "icons/filter.svg",
            Icon::Microscope => "icons/microscope.svg",
            Icon::Clock => "icons/clock.svg",
            Icon::Inbox => "icons/inbox.svg",
            Icon::CircleX => "icons/circle-x.svg",
            Icon::Ban => "icons/ban.svg",
            Icon::Warning => "icons/alert-triangle.svg",
            Icon::Puzzle => "icons/puzzle.svg",
            Icon::Alarm => "icons/alarm.svg",
            Icon::Flask => "icons/flask.svg",
            Icon::Close => "icons/x.svg",
            Icon::Check => "icons/check.svg",
            Icon::ChevronDown => "icons/chevron-down.svg",
            Icon::ChevronUp => "icons/chevron-up.svg",
            Icon::ChevronRight => "icons/chevron-right.svg",
            Icon::ChevronLeft => "icons/chevron-left.svg",
            Icon::ArrowUp => "icons/arrow-up.svg",
            Icon::ArrowDown => "icons/arrow-down.svg",
            Icon::ArrowsSort => "icons/arrows-sort.svg",
            Icon::Info => "icons/info-circle.svg",
            Icon::CircleCheck => "icons/circle-check.svg",
            Icon::AlertCircle => "icons/alert-circle.svg",
            Icon::Help => "icons/help-circle.svg",
            Icon::Search => "icons/search.svg",
            Icon::Plus => "icons/plus.svg",
            Icon::Minus => "icons/minus.svg",
            Icon::Eye => "icons/eye.svg",
            Icon::EyeOff => "icons/eye-off.svg",
            Icon::Pencil => "icons/pencil.svg",
            Icon::Trash => "icons/trash.svg",
            Icon::Download => "icons/download.svg",
            Icon::Upload => "icons/upload.svg",
            Icon::Undo => "icons/arrow-back-up.svg",
            Icon::ArrowLeft => "icons/arrow-left.svg",
            Icon::Scale => "icons/scale.svg",
            Icon::Copy => "icons/copy.svg",
            Icon::Lock => "icons/lock.svg",
            Icon::Network => "icons/network.svg",
            Icon::Cpu => "icons/TablerCpu.svg",
            Icon::Table => "icons/table.svg",
            Icon::ChartBar => "icons/chart-bar.svg",
            Icon::SidebarRight => "icons/layout-sidebar-right.svg",
            Icon::ZoomIn => "icons/zoom-in.svg",
            Icon::ZoomOut => "icons/zoom-out.svg",
            Icon::FocusCentered => "icons/focus-centered.svg",
            Icon::DotsVertical => "icons/dots-vertical.svg",
        }
    }
}

/// Élément SVG d'une icône, 16 × 16 px.
///
/// gpui peint un SVG comme un masque teinté par la couleur de texte de
/// l'élément lui-même (elle n'est pas héritée du parent) : appeler
/// `.text_color(…)` sur le résultat, sinon rien n'est peint.
pub fn icon(icon: Icon) -> Svg {
    svg().path(icon.path()).size(px(16.)).flex_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chaque_icone_est_embarquee() {
        for icon in Icon::ALL {
            let data = Assets::get(icon.path());
            assert!(data.is_some(), "{} absente", icon.path());
            let text = std::str::from_utf8(data.unwrap_or_default()).unwrap_or_default();
            assert!(text.contains("<svg"), "{} n'est pas un SVG", icon.path());
        }
    }

    #[test]
    fn toutes_les_variantes_sont_listees() {
        // Garde-fou : ALL doit couvrir chaque fichier d'icône embarqué.
        let icon_files = FILES
            .iter()
            .filter(|(name, _)| name.starts_with("icons/"))
            .count();
        assert_eq!(Icon::ALL.len(), icon_files);
    }

    #[test]
    fn le_logo_est_un_png() {
        let logo = Assets::get(LOGO_PNG).unwrap_or_default();
        assert!(logo.starts_with(b"\x89PNG"));
    }
}
