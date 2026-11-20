//! Période du relevé : ce que le backend en dit, et ce que la synthèse en
//! affiche (bornes, « Durée du relevé »).

use super::dates::{EpochMillis, parse_js_date};
use super::locale::format_integer;

/// Période du relevé telle que l'interface l'affiche : vide, connue
/// (premier et dernier paquet, RFC 3339 en UTC), ou inconnue quand une
/// matrice importée ne la déclarait pas.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum SurveyPeriodView {
    /// Relevé sans paquet : aucune période.
    #[default]
    Empty,
    /// Période connue : premier et dernier paquet.
    Known {
        /// Horodatage du premier paquet, RFC 3339 en UTC.
        first: String,
        /// Horodatage du dernier paquet, RFC 3339 en UTC.
        last: String,
    },
    /// Matrice importée dont le préambule ne déclarait pas de période.
    Unknown,
}

/// Ce que la synthèse affiche de la période du relevé.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PeriodSummary {
    /// Aucun paquet observé.
    Empty,
    /// Période non déclarée par une matrice importée.
    Unknown,
    /// Période connue.
    Known {
        /// Premier paquet ; `None` si l'horodatage est illisible
        /// (`Invalid Date` en JavaScript).
        first: Option<EpochMillis>,
        /// Dernier paquet ; `None` si l'horodatage est illisible.
        last: Option<EpochMillis>,
        /// Durée lisible entre les deux (cf. [`format_duration`]).
        duration: String,
    },
}

/// Durée lisible : « < 1 s », « 42 s », « 4 min 12 s », « 3 h 05 min »,
/// « 12 j 4 h ». Tronquée, jamais arrondie vers le haut. Prend des
/// millisecondes flottantes : `NaN` (date illisible) et
/// les valeurs négatives donnent « < 1 s ».
pub fn format_duration(milliseconds: f64) -> String {
    if !milliseconds.is_finite() || milliseconds < 1000.0 {
        return "< 1 s".to_string();
    }
    // Troncature voulue : `Math.floor` d'une durée positive et finie.
    let seconds = (milliseconds / 1000.0).floor() as u64;
    let (days, hours, minutes, rest) = (
        seconds / 86_400,
        seconds % 86_400 / 3600,
        seconds % 3600 / 60,
        seconds % 60,
    );
    if days > 0 {
        return format!("{} j {hours} h", format_integer(days));
    }
    if hours > 0 {
        return format!("{hours} h {minutes:02} min");
    }
    if minutes > 0 {
        return format!("{minutes} min {rest} s");
    }
    format!("{rest} s")
}

/// Période du relevé prête à afficher : bornes lues comme `new Date(…)` et
/// durée lisible.
pub fn summarize_period(period: &SurveyPeriodView) -> PeriodSummary {
    match period {
        SurveyPeriodView::Empty => PeriodSummary::Empty,
        SurveyPeriodView::Unknown => PeriodSummary::Unknown,
        SurveyPeriodView::Known { first, last } => {
            let (first, last) = (parse_js_date(first), parse_js_date(last));
            // Millisecondes exactes : `Date` borne l'instant à ±8,64·10¹⁵.
            let elapsed = match (first, last) {
                (Some(first), Some(last)) => (last.0 - first.0) as f64,
                _ => f64::NAN,
            };
            PeriodSummary::Known {
                first,
                last,
                duration: format_duration(elapsed),
            }
        }
    }
}

/// Valeur de l'indicateur « Durée du relevé » : la durée, « inconnue » pour
/// une matrice sans période déclarée, « — » sans paquet.
pub fn duration_value(period: &SurveyPeriodView) -> String {
    match summarize_period(period) {
        PeriodSummary::Known { duration, .. } => duration,
        PeriodSummary::Unknown => "inconnue".to_string(),
        PeriodSummary::Empty => "—".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duree_et_periode_du_releve() {
        assert_eq!(format_duration(0.0), "< 1 s");
        assert_eq!(format_duration(42_900.0), "42 s");
        assert_eq!(format_duration(252_500.0), "4 min 12 s");
        assert_eq!(
            format_duration(f64::from(3 * 3_600_000 + 5 * 60_000)),
            "3 h 05 min"
        );
        assert_eq!(
            format_duration(12.0 * 86_400_000.0 + 4.0 * 3_600_000.0 + 59_000.0),
            "12 j 4 h"
        );
        assert_eq!(
            summarize_period(&SurveyPeriodView::Unknown),
            PeriodSummary::Unknown
        );
        let known = summarize_period(&SurveyPeriodView::Known {
            first: "2026-10-12T08:30:00Z".to_string(),
            last: "2026-10-12T08:34:12.5Z".to_string(),
        });
        assert!(
            matches!(known, PeriodSummary::Known { ref duration, .. } if duration == "4 min 12 s")
        );
    }

    #[test]
    fn duree_des_jours_groupes_et_date_illisible() {
        assert_eq!(format_duration(1234.0 * 86_400_000.0), "1\u{202F}234 j 0 h");
        assert_eq!(format_duration(f64::NAN), "< 1 s");
        let unreadable = summarize_period(&SurveyPeriodView::Known {
            first: "?".to_string(),
            last: "2026-10-12T08:34:12Z".to_string(),
        });
        assert!(
            matches!(unreadable, PeriodSummary::Known { first: None, ref duration, .. } if duration == "< 1 s")
        );
        assert_eq!(duration_value(&SurveyPeriodView::Empty), "—");
        assert_eq!(duration_value(&SurveyPeriodView::Unknown), "inconnue");
    }
}
