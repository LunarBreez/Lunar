//! Raccords de la vue d'investigation : fuseau du poste, types du moteur
//! vers ceux de `lunar_ui_logic`, chemins choisis dans les sélecteurs de
//! fichiers, couleurs CSS. Fonctions pures, testées sans fenêtre.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, Offset, TimeZone, Utc};
use gpui::Hsla;
use lunar_engine::commandes::export::synthesis::SynthesisExport as EngineSynthesisExport;
use lunar_engine::commandes::flow_matrix::SurveyPeriodView as EngineSurveyPeriod;
use lunar_ui_logic::file_names::LocalDateTime;
use lunar_ui_logic::investigation::dates::local_civil;
use lunar_ui_logic::investigation::{EpochMillis, LocalZone, SurveyPeriodView, SynthesisExport};

/// Fuseau du poste, heure d'été comprise.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SystemZone;

impl LocalZone for SystemZone {
    fn offset_seconds(&self, at: EpochMillis) -> i32 {
        // Hors de la plage de chrono (±262 000 ans) : décalage d'aujourd'hui,
        // seule valeur raisonnable pour une date de toute façon absurde.
        let utc = DateTime::<Utc>::from_timestamp_millis(at.0).unwrap_or_else(Utc::now);
        Local
            .offset_from_utc_datetime(&utc.naive_utc())
            .fix()
            .local_minus_utc()
    }
}

/// Date et heure locales de `at`, pour les noms de fichiers proposés
/// ([`lunar_ui_logic::file_names`]).
pub fn local_date_time(at: EpochMillis, zone: &impl LocalZone) -> LocalDateTime {
    let civil = local_civil(at, zone);
    let narrow = |value: u32| u8::try_from(value).unwrap_or(0);
    LocalDateTime::new(
        i32::try_from(civil.year).unwrap_or(i32::MAX),
        narrow(civil.month),
        narrow(civil.day),
        narrow(civil.hour),
        narrow(civil.minute),
        narrow(civil.second),
    )
}

/// Période du relevé telle que la rend le moteur (`get_survey_period`).
pub fn survey_period(period: EngineSurveyPeriod) -> SurveyPeriodView {
    match period {
        EngineSurveyPeriod::Empty => SurveyPeriodView::Empty,
        EngineSurveyPeriod::Known { first, last } => SurveyPeriodView::Known { first, last },
        EngineSurveyPeriod::Unknown => SurveyPeriodView::Unknown,
    }
}

/// Ce que rend `export_synthesis_matrix`, pour composer la page.
pub fn synthesis_export(export: EngineSynthesisExport) -> SynthesisExport {
    SynthesisExport {
        graph: export.graph,
        csv_file_name: export.csv_file_name,
        csv_sha256: export.csv_sha256,
        csv_bytes: export.csv_bytes,
        link_type: export.link_type,
        site: export.site,
        sensor: export.sensor,
        interface: export.interface,
        app_name: lunar_engine::APP_NAME.to_string(),
        app_version: export.app_version,
        period: survey_period(export.period),
    }
}

/// Chemin choisi dans le sélecteur d'enregistrement, complété de
/// l'extension `extension` s'il n'a aucune de celles de `accepted` (le
/// sélecteur GTK ne l'ajoute pas).
/// La comparaison ignore la casse : « Synthese.HTML » reste tel quel.
pub fn with_extension(path: &Path, extension: &str, accepted: &[&str]) -> PathBuf {
    let has_one = path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| accepted.iter().any(|ok| ext.eq_ignore_ascii_case(ok)));
    if has_one {
        return path.to_path_buf();
    }
    let mut name = path.as_os_str().to_owned();
    name.push(".");
    name.push(extension);
    PathBuf::from(name)
}

/// Couleur CSS `#rgb` ou `#rrggbb` (table des couleurs de protocole) ;
/// `None` si elle est illisible.
pub fn css_hex(color: &str) -> Option<Hsla> {
    let digits = color.strip_prefix('#')?;
    if !digits.is_ascii() {
        return None;
    }
    let expanded = match digits.len() {
        3 => digits.chars().flat_map(|c| [c, c]).collect::<String>(),
        6 => digits.to_string(),
        _ => return None,
    };
    let value = u32::from_str_radix(&expanded, 16).ok()?;
    Some(Hsla::from(gpui::rgb(value)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lunar_ui_logic::file_names::synthesis_html_name;
    use lunar_ui_logic::investigation::FixedZone;
    use lunar_ui_logic::investigation::dates::parse_js_date;

    #[test]
    fn fuseau_du_poste_comme_chrono() {
        let now = Utc::now();
        let expected = Local
            .offset_from_utc_datetime(&now.naive_utc())
            .fix()
            .local_minus_utc();
        assert_eq!(
            SystemZone.offset_seconds(EpochMillis(now.timestamp_millis())),
            expected
        );
    }

    #[test]
    fn fuseau_du_poste_hors_plage_sans_panique() {
        // Bornes de `Date` JavaScript : au-delà de chrono, décalage du jour.
        let _ = SystemZone.offset_seconds(EpochMillis(8_640_000_000_000_000));
        let _ = SystemZone.offset_seconds(EpochMillis(-8_640_000_000_000_000));
    }

    #[test]
    fn date_locale_des_noms_de_fichiers() {
        let at = parse_js_date("2026-11-19T23:30:00Z").map(|at| at.0);
        let at = EpochMillis(at.unwrap_or_default());
        let paris = FixedZone::new(3600);
        assert_eq!(
            local_date_time(at, &paris),
            LocalDateTime::new(2026, 11, 20, 0, 30, 0)
        );
        assert_eq!(
            synthesis_html_name(&local_date_time(at, &FixedZone::UTC)),
            "20261119_DR_Synthese.html"
        );
    }

    #[test]
    fn periode_du_moteur() {
        assert_eq!(
            survey_period(EngineSurveyPeriod::Empty),
            SurveyPeriodView::Empty
        );
        assert_eq!(
            survey_period(EngineSurveyPeriod::Unknown),
            SurveyPeriodView::Unknown
        );
        assert_eq!(
            survey_period(EngineSurveyPeriod::Known {
                first: "2026-07-03T08:00:00Z".into(),
                last: "2026-07-03T09:00:00Z".into(),
            }),
            SurveyPeriodView::Known {
                first: "2026-07-03T08:00:00Z".into(),
                last: "2026-07-03T09:00:00Z".into(),
            }
        );
    }

    #[test]
    fn extension_ajoutee_si_absente() {
        let html = ["html", "htm"];
        assert_eq!(
            with_extension(Path::new("/tmp/synthese"), "html", &html),
            PathBuf::from("/tmp/synthese.html")
        );
        assert_eq!(
            with_extension(Path::new("/tmp/synthese.HTML"), "html", &html),
            PathBuf::from("/tmp/synthese.HTML")
        );
        assert_eq!(
            with_extension(Path::new("/tmp/synthese.htm"), "html", &html),
            PathBuf::from("/tmp/synthese.htm")
        );
        assert_eq!(
            with_extension(Path::new("/tmp/releve.v2"), "html", &html),
            PathBuf::from("/tmp/releve.v2.html"),
            "une autre extension n'est pas remplacée"
        );
        assert_eq!(
            with_extension(Path::new("/tmp/labels"), "csv", &["csv"]),
            PathBuf::from("/tmp/labels.csv")
        );
    }

    #[test]
    fn couleurs_css() {
        assert_eq!(css_hex("#84c3f7"), Some(Hsla::from(gpui::rgb(0x84c3f7))));
        assert_eq!(css_hex("#abc"), Some(Hsla::from(gpui::rgb(0xaabbcc))));
        assert_eq!(css_hex("84c3f7"), None);
        assert_eq!(css_hex("#84c3f"), None);
        assert_eq!(css_hex("#zzzzzz"), None);
        assert_eq!(css_hex("#éé"), None);
    }
}
