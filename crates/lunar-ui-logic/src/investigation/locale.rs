//! Comportements « fr-FR » et JavaScript que la vue d'investigation et la
//! page de synthèse tiennent du moteur JavaScript : comparaison
//! `localeCompare(…, "fr")`, `toLocaleString("fr-FR")` des entiers et des
//! décimaux, `toFixed`, `String(nombre)`, `trim`.
//!
//! Chaque fonction reproduit à l'octet la sortie de V8/ICU, vérifiée par
//! les fichiers de référence de `tests/golden/`.

use std::cmp::Ordering;
use std::sync::LazyLock;

use icu_collator::options::CollatorOptions;
use icu_collator::{Collator, CollatorBorrowed};
use icu_locale_core::locale;

/// Séparateur des milliers de `toLocaleString("fr-FR")` : espace fine
/// insécable (U+202F), et non l'espace insécable U+00A0 d'avant CLDR 34.
pub const GROUP_SEPARATOR: char = '\u{202F}';

/// Collateur français par défaut, comme `Intl.Collator("fr")` : force
/// tertiaire, ponctuation significative, pas de tri numérique. `None` si les
/// données compilées manquaient (impossible avec `compiled_data`) : on
/// retombe alors sur l'ordre des octets.
static COLLATOR_FR: LazyLock<Option<CollatorBorrowed<'static>>> =
    LazyLock::new(|| Collator::try_new(locale!("fr").into(), CollatorOptions::default()).ok());

/// `a.localeCompare(b, "fr")` : ordre alphabétique français (casse et
/// accents départagent seulement à égalité de lettres ; la ponctuation compte
/// et passe avant les chiffres, eux-mêmes avant les lettres).
pub fn compare_fr(a: &str, b: &str) -> Ordering {
    match COLLATOR_FR.as_ref() {
        Some(collator) => collator.compare(a, b),
        None => a.cmp(b),
    }
}

/// Groupe les chiffres d'une écriture décimale par trois avec
/// [`GROUP_SEPARATOR`] (règle fr : dès 1 000, regroupement minimal 1).
fn group_digits(digits: &str) -> String {
    let len = digits.chars().count();
    let mut out = String::with_capacity(digits.len() + len / 3 * 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (len - index).is_multiple_of(3) {
            out.push(GROUP_SEPARATOR);
        }
        out.push(digit);
    }
    out
}

/// `entier.toLocaleString("fr-FR")` : « 1 234 567 » (espaces fines
/// insécables).
pub fn format_integer(value: u64) -> String {
    group_digits(&value.to_string())
}

/// `valeur.toLocaleString("fr-FR", { maximumFractionDigits })` d'un nombre
/// fini : arrondi « au plus proche, moitié vers l'extérieur »
/// de l'écriture décimale la plus courte du flottant — celle d'ICU —, zéros
/// de queue retirés, virgule décimale et milliers groupés.
pub fn format_decimal(value: f64, maximum_fraction_digits: usize) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value.is_infinite() {
        return if value > 0.0 { "∞" } else { "-∞" }.to_string();
    }
    let negative = value < 0.0;
    // `Display` d'un f64 : plus courte écriture qui redonne le flottant, sans
    // exposant — la même que la conversion « shortest » d'ICU.
    let shortest = format!("{}", value.abs());
    let (int_part, frac_part) = shortest.split_once('.').unwrap_or((shortest.as_str(), ""));
    let mut digits: Vec<u8> = int_part.bytes().map(|byte| byte - b'0').collect();
    let int_len = digits.len();
    let frac: Vec<u8> = frac_part.bytes().map(|byte| byte - b'0').collect();
    let kept = frac.len().min(maximum_fraction_digits);
    digits.extend_from_slice(&frac[..kept]);
    let round_up = frac
        .get(maximum_fraction_digits)
        .is_some_and(|&digit| digit >= 5);
    let mut int_len = int_len;
    if round_up {
        let mut index = digits.len();
        loop {
            if index == 0 {
                digits.insert(0, 1);
                int_len += 1;
                break;
            }
            index -= 1;
            if digits[index] == 9 {
                digits[index] = 0;
            } else {
                digits[index] += 1;
                break;
            }
        }
    }
    let to_text = |slice: &[u8]| {
        slice
            .iter()
            .map(|digit| char::from(b'0' + digit))
            .collect::<String>()
    };
    let integer = to_text(&digits[..int_len]);
    let fraction = to_text(&digits[int_len..]);
    let fraction = fraction.trim_end_matches('0');
    let mut out = String::new();
    // Une valeur négative arrondie à zéro garde son signe (« -0 »), comme
    // chez ICU.
    if negative {
        out.push('-');
    }
    out.push_str(&group_digits(&integer));
    if !fraction.is_empty() {
        out.push(',');
        out.push_str(fraction);
    }
    out
}

/// Vrai si `value` tombe exactement au milieu de deux arrondis à `digits`
/// décimales : `value · 10^digits = k + ½`. En écrivant le flottant
/// `m · 2^e` avec `m` impair, c'est le cas si et seulement si
/// `e = -(digits + 1)` (`m · 5^digits` est impair).
fn is_exact_tie(value: f64, digits: usize) -> bool {
    if value == 0.0 || !value.is_finite() {
        return false;
    }
    let bits = value.abs().to_bits();
    let biased = i64::try_from((bits >> 52) & 0x7ff).unwrap_or(0);
    let mut mantissa = bits & ((1_u64 << 52) - 1);
    let mut exponent = if biased == 0 {
        -1074
    } else {
        mantissa |= 1_u64 << 52;
        biased - 1075
    };
    exponent += i64::from(mantissa.trailing_zeros());
    let digits = i64::try_from(digits).unwrap_or(i64::MAX);
    exponent == -(digits + 1)
}

/// `valeur.toFixed(digits)` : arrondi de la valeur binaire exacte, la moitié
/// exacte allant vers l'extérieur (« 1.25 » → « 1.3 »), là où le formatage
/// de Rust arrondit au pair.
pub fn to_fixed(value: f64, digits: usize) -> String {
    if !value.is_finite() || value.abs() >= 1e21 {
        return js_number_to_string(value);
    }
    if is_exact_tie(value, digits) {
        // Juste après la moitié, dans le sens de l'éloignement de zéro :
        // l'arrondi au plus proche donne alors celui de JavaScript.
        let away = if value > 0.0 {
            value.next_up()
        } else {
            value.next_down()
        };
        return format!("{away:.digits$}");
    }
    let text = format!("{value:.digits$}");
    // `(-0.04).toFixed(1)` donne « -0.0 » comme Rust ; `(-0).toFixed(1)`
    // donne « 0.0 » (le signe d'un zéro n'est jamais écrit par JavaScript).
    if value == 0.0 {
        text.trim_start_matches('-').to_string()
    } else {
        text
    }
}

/// `String(nombre)` (Number::toString) : écriture décimale la plus courte,
/// en notation exponentielle hors de `[1e-6, 1e21[`.
pub fn js_number_to_string(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    if value == 0.0 {
        return "0".to_string();
    }
    let magnitude = value.abs();
    if (1e-6..1e21).contains(&magnitude) {
        return format!("{value}");
    }
    // Rust écrit « 1.5e-7 » / « 1e21 » ; JavaScript ajoute le « + » des
    // exposants positifs.
    let text = format!("{value:e}");
    match text.split_once('e') {
        Some((mantissa, exponent)) if !exponent.starts_with('-') => {
            format!("{mantissa}e+{exponent}")
        }
        _ => text,
    }
}

/// Espaces retirés par `String.prototype.trim` : `White_Space` d'Unicode,
/// plus l'indicateur d'ordre des octets (U+FEFF) que JavaScript y ajoute.
fn is_js_whitespace(character: char) -> bool {
    character.is_whitespace() || character == '\u{FEFF}'
}

/// `chaîne.trim()` de JavaScript.
pub fn js_trim(value: &str) -> &str {
    value.trim_matches(is_js_whitespace)
}

/// `Number(chaîne)` de JavaScript, pour les conversions que fait le code
/// porté (octets d'une IPv4) : blancs ignorés, chaîne vide = 0, décimal avec
/// signe, fraction et exposant, préfixes `0x`/`0o`/`0b`, `Infinity`.
/// `None` = `NaN`.
pub fn js_string_to_number(value: &str) -> Option<f64> {
    let text = js_trim(value);
    if text.is_empty() {
        return Some(0.0);
    }
    let radix_body = |prefix_len: usize, radix: u32| -> Option<f64> {
        let body = &text[prefix_len..];
        if body.is_empty() || !body.chars().all(|c| c.is_digit(radix)) {
            return None;
        }
        Some(body.chars().fold(0.0_f64, |acc, c| {
            acc * f64::from(radix) + f64::from(c.to_digit(radix).unwrap_or(0))
        }))
    };
    let lower = text.get(..2).map(str::to_ascii_lowercase);
    match lower.as_deref() {
        Some("0x") => return radix_body(2, 16),
        Some("0o") => return radix_body(2, 8),
        Some("0b") => return radix_body(2, 2),
        _ => {}
    }
    let (sign, unsigned) = match text.as_bytes().first() {
        Some(b'+') => (1.0, &text[1..]),
        Some(b'-') => (-1.0, &text[1..]),
        _ => (1.0, text),
    };
    if unsigned == "Infinity" {
        return Some(sign * f64::INFINITY);
    }
    // Un seul signe : « +-5 » vaut NaN, alors que Rust lirait « -5 ».
    if unsigned.starts_with(['+', '-']) {
        return None;
    }
    // Grammaire StrUnsignedDecimalLiteral : chiffres, point, exposant ; Rust
    // accepte en plus « inf »/« nan », qu'on écarte en exigeant un chiffre.
    let valid = unsigned
        .bytes()
        .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-'))
        && unsigned
            .split(['e', 'E'])
            .next()
            .is_some_and(|mantissa| mantissa.bytes().any(|b| b.is_ascii_digit()));
    if !valid {
        return None;
    }
    unsigned.parse::<f64>().ok().map(|number| sign * number)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entiers_groupes_par_espace_fine() {
        assert_eq!(format_integer(0), "0");
        assert_eq!(format_integer(999), "999");
        assert_eq!(format_integer(1234), "1\u{202F}234");
        assert_eq!(format_integer(1_234_567), "1\u{202F}234\u{202F}567");
    }

    #[test]
    fn decimaux_comme_icu() {
        // Écriture la plus courte, puis moitié vers l'extérieur (V8/ICU :
        // 1.45 → « 1,5 », 0.15 → « 0,2 », 99.95 → « 100 »).
        assert_eq!(format_decimal(1.45, 1), "1,5");
        assert_eq!(format_decimal(1.05, 1), "1,1");
        assert_eq!(format_decimal(1.25, 1), "1,3");
        assert_eq!(format_decimal(0.15, 1), "0,2");
        assert_eq!(format_decimal(99.95, 1), "100");
        assert_eq!(format_decimal(1234.5, 1), "1\u{202F}234,5");
        assert_eq!(format_decimal(100.0, 1), "100");
        assert_eq!(format_decimal(33.333_333_333_333_336, 1), "33,3");
    }

    #[test]
    fn to_fixed_moitie_vers_le_haut() {
        assert_eq!(to_fixed(1.25, 1), "1.3");
        assert_eq!(to_fixed(1.45, 1), "1.4");
        assert_eq!(to_fixed(2.5, 0), "3");
        assert_eq!(to_fixed(1.05, 1), "1.1");
        assert_eq!(to_fixed(1.0, 1), "1.0");
        assert_eq!(to_fixed(-1.25, 1), "-1.3");
    }

    #[test]
    fn nombres_comme_string_de_javascript() {
        assert_eq!(js_number_to_string(100.0), "100");
        assert_eq!(js_number_to_string(2.0), "2");
        assert_eq!(js_number_to_string(100.0 / 3.0), "33.333333333333336");
        assert_eq!(js_number_to_string(1.5e-7), "1.5e-7");
        assert_eq!(js_number_to_string(1e21), "1e+21");
        assert_eq!(js_number_to_string(0.000_001), "0.000001");
    }

    #[test]
    fn number_de_javascript() {
        assert_eq!(js_string_to_number(""), Some(0.0));
        assert_eq!(js_string_to_number(" 12 "), Some(12.0));
        assert_eq!(js_string_to_number("0x10"), Some(16.0));
        assert_eq!(js_string_to_number("1e2"), Some(100.0));
        assert_eq!(js_string_to_number("+5"), Some(5.0));
        assert_eq!(js_string_to_number("1.5"), Some(1.5));
        assert_eq!(js_string_to_number("abc"), None);
        assert_eq!(js_string_to_number("inf"), None);
        assert_eq!(js_string_to_number("-0x10"), None);
        assert_eq!(js_string_to_number("+-5"), None);
        assert_eq!(js_string_to_number(".5"), Some(0.5));
    }

    #[test]
    fn collation_francaise() {
        let mut words = vec![
            "é",
            "e",
            "E",
            "b",
            "a",
            "A",
            "—",
            "-",
            "_",
            " ",
            "",
            "10.0.0.2",
            "10.0.0.10",
            "ARP",
            "Adresse MAC",
        ];
        words.sort_by(|a, b| compare_fr(a, b));
        assert_eq!(
            words,
            vec![
                "",
                " ",
                "_",
                "-",
                "—",
                "10.0.0.10",
                "10.0.0.2",
                "a",
                "A",
                "Adresse MAC",
                "ARP",
                "b",
                "e",
                "E",
                "é"
            ]
        );
    }
}
