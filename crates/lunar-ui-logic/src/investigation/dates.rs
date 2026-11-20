//! Dates de la vue d'investigation et de la page de synthèse, au sens de
//! `Date` JavaScript : un instant à la milliseconde, lu depuis le RFC 3339
//! du backend, affiché en heure locale du poste au format fr-FR : mêmes
//! résultats que `new Date(texte)`, `toISOString()` et
//! `toLocaleString("fr-FR", { dateStyle, timeStyle })`.
//!
//! Aucune dépendance de fuseau : l'heure locale passe par [`LocalZone`], que
//! l'application implémente avec le fuseau du système ; les tests utilisent
//! [`FixedZone`], ce qui rend la page reproductible.

use std::time::{SystemTime, UNIX_EPOCH};

/// Borne de `Date` JavaScript : ±8,64·10¹⁵ ms autour de l'époque Unix.
const MAX_EPOCH_MILLIS: i64 = 8_640_000_000_000_000;

/// Millisecondes par jour.
const MILLIS_PER_DAY: i64 = 86_400_000;

/// Instant au sens de `Date.getTime()` : millisecondes depuis l'époque Unix,
/// en UTC.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EpochMillis(pub i64);

impl EpochMillis {
    /// Instant courant de l'horloge système (`new Date()`).
    pub fn now() -> Self {
        let millis = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(elapsed) => i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX),
            Err(before) => -i64::try_from(before.duration().as_millis()).unwrap_or(i64::MAX),
        };
        Self(millis)
    }
}

/// Fuseau d'affichage des dates : décalage par rapport à UTC, en secondes, à
/// un instant donné (heure d'été comprise). L'application l'implémente avec
/// le fuseau du poste, comme le faisait le moteur JavaScript.
pub trait LocalZone {
    /// Décalage local (secondes, positif à l'est de Greenwich) à l'instant
    /// UTC `at`.
    fn offset_seconds(&self, at: EpochMillis) -> i32;
}

/// Fuseau à décalage fixe (UTC, `Etc/GMT-2`…) : tests et pages
/// reproductibles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FixedZone {
    /// Décalage par rapport à UTC, en secondes.
    pub offset_seconds: i32,
}

impl FixedZone {
    /// Temps universel.
    pub const UTC: Self = Self { offset_seconds: 0 };

    /// Fuseau décalé de `offset_seconds` par rapport à UTC.
    pub const fn new(offset_seconds: i32) -> Self {
        Self { offset_seconds }
    }
}

impl LocalZone for FixedZone {
    fn offset_seconds(&self, _at: EpochMillis) -> i32 {
        self.offset_seconds
    }
}

/// Date et heure civiles (calendrier grégorien proleptique).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CivilDateTime {
    /// Année (astronomique : 0 = 1 av. J.-C.).
    pub year: i64,
    /// Mois, de 1 à 12.
    pub month: u32,
    /// Jour du mois, de 1 à 31.
    pub day: u32,
    /// Heure, de 0 à 23.
    pub hour: u32,
    /// Minute, de 0 à 59.
    pub minute: u32,
    /// Seconde, de 0 à 59.
    pub second: u32,
    /// Milliseconde, de 0 à 999.
    pub millisecond: u32,
}

/// Jours depuis le 1970-01-01 du jour civil donné (algorithme de H. Hinnant,
/// `days_from_civil`), valable pour toute année.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month = i64::from(month);
    let day_of_year =
        (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Jour civil d'un nombre de jours depuis le 1970-01-01 (`civil_from_days`).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (
        year,
        u32::try_from(month).unwrap_or(1),
        u32::try_from(day).unwrap_or(1),
    )
}

/// Date civile d'un instant décalé de `offset_seconds`.
fn civil_at(at: EpochMillis, offset_seconds: i32) -> CivilDateTime {
    let local = at.0 + i64::from(offset_seconds) * 1000;
    let days = local.div_euclid(MILLIS_PER_DAY);
    let in_day = local.rem_euclid(MILLIS_PER_DAY);
    let (year, month, day) = civil_from_days(days);
    let part = |value: i64| u32::try_from(value).unwrap_or(0);
    CivilDateTime {
        year,
        month,
        day,
        hour: part(in_day / 3_600_000),
        minute: part(in_day / 60_000 % 60),
        second: part(in_day / 1000 % 60),
        millisecond: part(in_day % 1000),
    }
}

/// Date et heure locales de `at` dans `zone` (`getFullYear()`, `getMonth()`…).
pub fn local_civil(at: EpochMillis, zone: &impl LocalZone) -> CivilDateTime {
    civil_at(at, zone.offset_seconds(at))
}

/// Date et heure UTC de `at` (`getUTCFullYear()`…).
pub fn utc_civil(at: EpochMillis) -> CivilDateTime {
    civil_at(at, 0)
}

/// Curseur de lecture de [`parse_js_date`].
struct Cursor<'a> {
    /// Octets restant à lire.
    rest: &'a [u8],
}

impl Cursor<'_> {
    /// Lit exactement `count` chiffres décimaux.
    fn digits(&mut self, count: usize) -> Option<i64> {
        let (head, tail) = (self.rest.get(..count)?, self.rest.get(count..)?);
        if !head.iter().all(u8::is_ascii_digit) {
            return None;
        }
        self.rest = tail;
        Some(
            head.iter()
                .fold(0, |acc, digit| acc * 10 + i64::from(digit - b'0')),
        )
    }

    /// Consomme `byte` s'il est le prochain octet.
    fn eat(&mut self, byte: u8) -> bool {
        self.eat_any(&[byte]).is_some()
    }

    /// Consomme le prochain octet s'il figure dans `bytes`.
    fn eat_any(&mut self, bytes: &[u8]) -> Option<u8> {
        let (&first, tail) = self.rest.split_first()?;
        if bytes.contains(&first) {
            self.rest = tail;
            Some(first)
        } else {
            None
        }
    }
}

/// `new Date(texte).getTime()` pour les horodatages ISO 8601 / RFC 3339 :
/// `AAAA-MM-JJ`, `AAAA-MM` ou `AAAA` (UTC), suivis de `THH:mm`, `:ss` et
/// d'une fraction de longueur quelconque (tronquée à la milliseconde), puis
/// de `Z` ou d'un décalage `±HH:mm` / `±HHmm`. Années étendues `±AAAAAA`,
/// séparateur `T`, `t` ou espace, jour jusqu'à 31 quel que soit le mois
/// (débordement sur le mois suivant) et `24:00:00` acceptés comme V8.
///
/// `None` = `Invalid Date` (`getTime()` vaut `NaN`). Écart assumé : une
/// heure sans décalage est lue en UTC, là où JavaScript la lirait en heure
/// locale — le backend écrit toujours `Z`.
pub fn parse_js_date(text: &str) -> Option<EpochMillis> {
    let mut cursor = Cursor {
        rest: text.as_bytes(),
    };
    let year = match cursor.eat_any(b"+-") {
        Some(sign) => {
            let year = cursor.digits(6)?;
            if sign == b'-' {
                if year == 0 {
                    return None;
                }
                -year
            } else {
                year
            }
        }
        None => cursor.digits(4)?,
    };
    let mut month = 1;
    let mut day = 1;
    if cursor.eat(b'-') {
        month = cursor.digits(2)?;
        if cursor.eat(b'-') {
            day = cursor.digits(2)?;
        }
    }
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let (mut hour, mut minute, mut second, mut millis) = (0, 0, 0, 0);
    let mut offset_minutes = 0;
    if cursor.eat_any(b"Tt ").is_some() {
        hour = cursor.digits(2)?;
        if !cursor.eat(b':') {
            return None;
        }
        minute = cursor.digits(2)?;
        if cursor.eat(b':') {
            second = cursor.digits(2)?;
            if cursor.eat(b'.') {
                let fraction_len = cursor
                    .rest
                    .iter()
                    .take_while(|byte| byte.is_ascii_digit())
                    .count();
                if fraction_len == 0 {
                    return None;
                }
                let (fraction, tail) = cursor.rest.split_at(fraction_len);
                cursor.rest = tail;
                // Trois chiffres au plus : au-delà de la milliseconde, ignorés.
                millis = fraction
                    .iter()
                    .chain(std::iter::repeat(&b'0'))
                    .take(3)
                    .fold(0, |acc, digit| acc * 10 + i64::from(digit - b'0'));
            }
        }
        if let Some(sign) = cursor.eat_any(b"+-") {
            let hours = cursor.digits(2)?;
            cursor.eat(b':');
            let minutes = cursor.digits(2)?;
            if hours > 23 || minutes > 59 {
                return None;
            }
            let total = hours * 60 + minutes;
            offset_minutes = if sign == b'-' { -total } else { total };
        } else {
            cursor.eat_any(b"Zz");
        }
        let midnight_end = hour == 24 && minute == 0 && second == 0 && millis == 0;
        if (hour > 23 && !midnight_end) || minute > 59 || second > 59 {
            return None;
        }
    }
    if !cursor.rest.is_empty() {
        return None;
    }
    let month = u32::try_from(month).ok()?;
    let days = days_from_civil(year, month, 1) + (day - 1);
    let time = ((hour * 60 + minute) * 60 + second) * 1000 + millis;
    let epoch = days * MILLIS_PER_DAY + time - offset_minutes * 60_000;
    (epoch.abs() <= MAX_EPOCH_MILLIS).then_some(EpochMillis(epoch))
}

/// Année au format ISO de `toISOString` : quatre chiffres entre 0 et 9999,
/// sinon six chiffres signés.
fn iso_year(year: i64) -> String {
    if (0..=9999).contains(&year) {
        format!("{year:04}")
    } else if year < 0 {
        format!("-{:06}", -year)
    } else {
        format!("+{year:06}")
    }
}

/// `date.toISOString()` : `AAAA-MM-JJTHH:mm:ss.sssZ`, en UTC.
pub fn to_iso_string(at: EpochMillis) -> String {
    let civil = utc_civil(at);
    format!(
        "{}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        iso_year(civil.year),
        civil.month,
        civil.day,
        civil.hour,
        civil.minute,
        civil.second,
        civil.millisecond
    )
}

/// Ce qu'écrit `toLocaleString` pour une date invalide.
pub const INVALID_DATE: &str = "Invalid Date";

/// Noms des mois en français (format « long »).
const MONTHS_FR: [&str; 12] = [
    "janvier",
    "février",
    "mars",
    "avril",
    "mai",
    "juin",
    "juillet",
    "août",
    "septembre",
    "octobre",
    "novembre",
    "décembre",
];

/// Année affichée par ICU : année de l'ère, sans le nom de l'ère. L'an 0
/// astronomique est « 1 » (1 av. J.-C.), -271821 est « 271822 ».
const fn era_year(year: i64) -> i64 {
    if year <= 0 { 1 - year } else { year }
}

/// Heure « medium » fr-FR : `HH:mm:ss`.
fn time_medium(civil: &CivilDateTime) -> String {
    format!("{:02}:{:02}:{:02}", civil.hour, civil.minute, civil.second)
}

/// `date.toLocaleString("fr-FR", { dateStyle: "short", timeStyle: "medium" })`
/// en heure locale : « 12/10/2026 10:30:00 ». `None` (date invalide) donne
/// « Invalid Date », comme JavaScript.
pub fn format_short_medium(at: Option<EpochMillis>, zone: &impl LocalZone) -> String {
    let Some(at) = at else {
        return INVALID_DATE.to_string();
    };
    let civil = local_civil(at, zone);
    format!(
        "{:02}/{:02}/{} {}",
        civil.day,
        civil.month,
        era_year(civil.year),
        time_medium(&civil)
    )
}

/// `date.toLocaleString("fr-FR", { dateStyle: "long", timeStyle: "medium" })`
/// en heure locale : « 12 octobre 2026 à 10:30:00 ».
pub fn format_long_medium(at: Option<EpochMillis>, zone: &impl LocalZone) -> String {
    let Some(at) = at else {
        return INVALID_DATE.to_string();
    };
    let civil = local_civil(at, zone);
    let month = usize::try_from(civil.month)
        .ok()
        .and_then(|month| MONTHS_FR.get(month.wrapping_sub(1)));
    format!(
        "{} {} {} à {}",
        civil.day,
        month.copied().unwrap_or(""),
        era_year(civil.year),
        time_medium(&civil)
    )
}

/// Complète à deux chiffres (« 5 » → « 05 »).
pub fn pad_zero(value: i64) -> String {
    if value < 10 {
        format!("0{value}")
    } else {
        value.to_string()
    }
}

/// Date locale compacte `AAAAMMJJ` de `at` (nommage des fichiers exportés).
pub fn compact_date(at: EpochMillis, zone: &impl LocalZone) -> String {
    let civil = local_civil(at, zone);
    format!(
        "{}{}{}",
        civil.year,
        pad_zero(i64::from(civil.month)),
        pad_zero(i64::from(civil.day))
    )
}

/// Date du jour, `AAAAMMJJ`.
pub fn current_date(zone: &impl LocalZone) -> String {
    compact_date(EpochMillis::now(), zone)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pad_zero_ajoute_un_zero_devant_les_nombres_inferieurs_a_10() {
        assert_eq!(pad_zero(5), "05");
        assert_eq!(pad_zero(9), "09");
    }

    #[test]
    fn pad_zero_ne_modifie_pas_les_nombres_superieurs_ou_egaux_a_10() {
        assert_eq!(pad_zero(10), "10");
        assert_eq!(pad_zero(15), "15");
    }

    #[test]
    fn get_current_date_retourne_la_date_actuelle_au_format_yyyymmdd() {
        // Le 10 mars 2024 à minuit, heure locale : l'instant et le fuseau
        // sont passés explicitement.
        let zone = FixedZone::new(3600);
        let midnight_local = EpochMillis(days_from_civil(2024, 3, 10) * MILLIS_PER_DAY - 3_600_000);
        assert_eq!(compact_date(midnight_local, &zone), "20240310");
        assert_eq!(current_date(&zone).len(), 8);
    }

    // Formats de référence de V8/ICU (TZ=Etc/GMT-2 puis UTC).

    #[test]
    fn formats_fr_fr_et_iso() {
        let at = parse_js_date("2026-10-12T08:30:00Z");
        let paris_ete = FixedZone::new(7200);
        assert_eq!(
            format_long_medium(at, &paris_ete),
            "12 octobre 2026 à 10:30:00"
        );
        assert_eq!(format_short_medium(at, &paris_ete), "12/10/2026 10:30:00");
        assert_eq!(
            format_long_medium(parse_js_date("2026-03-01T00:00:05Z"), &FixedZone::new(3600)),
            "1 mars 2026 à 01:00:05"
        );
        assert_eq!(format_short_medium(None, &paris_ete), "Invalid Date");
        assert_eq!(
            format_short_medium(parse_js_date("0999-01-01T00:00:00Z"), &FixedZone::UTC),
            "01/01/999 00:00:00"
        );
        assert_eq!(to_iso_string(EpochMillis(0)), "1970-01-01T00:00:00.000Z");
        assert_eq!(
            parse_js_date("+012026-03-04T05:06:07Z")
                .map(to_iso_string)
                .as_deref(),
            Some("+012026-03-04T05:06:07.000Z")
        );
    }

    #[test]
    fn lecture_comme_new_date() {
        let iso = |text: &str| parse_js_date(text).map(to_iso_string);
        assert_eq!(
            iso("2026-10-12T08:34:12.5005Z").as_deref(),
            Some("2026-10-12T08:34:12.500Z")
        );
        assert_eq!(
            iso("2026-02-30T00:00:00Z").as_deref(),
            Some("2026-03-02T00:00:00.000Z")
        );
        assert_eq!(
            iso("2026-10-12T24:00:00Z").as_deref(),
            Some("2026-10-13T00:00:00.000Z")
        );
        assert_eq!(iso("2026-10-12T24:00:01Z"), None);
        assert_eq!(
            iso("2026-10-12T08:30:00+02:00").as_deref(),
            Some("2026-10-12T06:30:00.000Z")
        );
        assert_eq!(iso("2026-10-12T08:30:00.Z"), None);
        assert_eq!(iso("2026-13-01T00:00:00Z"), None);
        assert_eq!(iso("bad"), None);
    }
}
