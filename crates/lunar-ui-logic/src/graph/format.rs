//! Formatage des nombres à l'identique du JavaScript : `toFixed`,
//! `toPrecision` et `toLocaleString("fr-FR")`.
//!
//! JavaScript arrondit la valeur décimale exacte du double au demi supérieur
//! (`(1.25).toFixed(1) === "1.3"`), là où `format!` de Rust arrondit au pair
//! (`"1.2"`). Les chiffres exacts sont obtenus par `format!("{:.40e}")` :
//! l'arrondi « au demi supérieur » ne dépend que du chiffre suivant le dernier
//! conservé, exact dès que la valeur n'enchaîne pas une quarantaine de 9 —
//! jamais le cas des zooms et tailles formatés ici.

/// Séparateur des milliers de `fr-FR` : espace fine insécable (U+202F).
pub(crate) const FR_GROUP_SEPARATOR: char = '\u{202F}';

/// `String.prototype.trim` : retire les blancs d'Unicode, plus l'indicateur
/// d'ordre des octets (U+FEFF) que `str::trim` garde.
pub(crate) fn js_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c.is_whitespace() || c == '\u{FEFF}')
}

/// Chiffres significatifs de `|x|` (x fini, non nul) et exposant décimal du
/// premier : `0.125` donne `([1, 2, 5, 0, …], -1)`.
fn significant_digits(x: f64) -> (Vec<u8>, i32) {
    let formatted = format!("{:.40e}", x.abs());
    let (mantissa, exponent) = formatted.split_once('e').unwrap_or((&formatted, "0"));
    let exponent = exponent.parse().unwrap_or(0);
    let digits = mantissa
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|b| b - b'0')
        .collect();
    (digits, exponent)
}

/// Garde les `keep` premiers chiffres de `digits` en arrondissant au demi
/// supérieur. Retourne les chiffres conservés et vrai si une retenue a
/// ajouté un chiffre en tête (`99` → `100`).
fn round_half_up(digits: &[u8], keep: usize) -> (Vec<u8>, bool) {
    let mut kept: Vec<u8> = (0..keep)
        .map(|i| digits.get(i).copied().unwrap_or(0))
        .collect();
    if digits.get(keep).copied().unwrap_or(0) < 5 {
        return (kept, false);
    }
    for digit in kept.iter_mut().rev() {
        if *digit == 9 {
            *digit = 0;
        } else {
            *digit += 1;
            return (kept, false);
        }
    }
    kept.insert(0, 1);
    (kept, true)
}

/// Chiffres ASCII d'une suite de chiffres décimaux.
fn ascii(digits: &[u8]) -> String {
    digits.iter().map(|d| char::from(b'0' + d)).collect()
}

/// `Number.prototype.toFixed(fraction_digits)` pour une valeur finie
/// inférieure à 10²¹ (au-delà, JavaScript passe en notation exponentielle).
pub(crate) fn to_fixed(x: f64, fraction_digits: usize) -> String {
    let sign = if x < 0.0 { "-" } else { "" };
    let n_digits = if x == 0.0 || !x.is_finite() {
        Vec::new()
    } else {
        let (digits, exponent) = significant_digits(x);
        // Nombre de chiffres de l'entier n = round(|x| × 10^f).
        let keep = exponent + 1 + fraction_digits as i32;
        if keep < 0 {
            Vec::new()
        } else {
            let (kept, _) = round_half_up(&digits, keep as usize);
            kept
        }
    };
    // n en décimal, complété à gauche pour avoir au moins un chiffre entier.
    let mut n = ascii(&n_digits).trim_start_matches('0').to_string();
    while n.len() < fraction_digits + 1 {
        n.insert(0, '0');
    }
    let negative_zero = n.bytes().all(|b| b == b'0');
    let sign = if negative_zero { "" } else { sign };
    if fraction_digits == 0 {
        return format!("{sign}{n}");
    }
    let (int_part, frac_part) = n.split_at(n.len() - fraction_digits);
    format!("{sign}{int_part}.{frac_part}")
}

/// `Number.prototype.toPrecision(precision)` (précision ≥ 1) pour une
/// valeur finie.
pub(crate) fn to_precision(x: f64, precision: usize) -> String {
    let precision = precision.max(1);
    if x == 0.0 || !x.is_finite() {
        return if precision > 1 {
            format!("0.{}", "0".repeat(precision - 1))
        } else {
            "0".to_string()
        };
    }
    let sign = if x < 0.0 { "-" } else { "" };
    let (digits, mut exponent) = significant_digits(x);
    let (mut kept, carried) = round_half_up(&digits, precision);
    if carried {
        exponent += 1;
        kept.truncate(precision);
    }
    let digits = ascii(&kept);
    let p = precision as i32;
    if exponent < -6 || exponent >= p {
        let (first, rest) = digits.split_at(1);
        let mantissa = if rest.is_empty() {
            first.to_string()
        } else {
            format!("{first}.{rest}")
        };
        let exp_sign = if exponent >= 0 { '+' } else { '-' };
        return format!("{sign}{mantissa}e{exp_sign}{}", exponent.abs());
    }
    if exponent >= 0 {
        let (int_part, frac_part) = digits.split_at(exponent as usize + 1);
        if frac_part.is_empty() {
            format!("{sign}{int_part}")
        } else {
            format!("{sign}{int_part}.{frac_part}")
        }
    } else {
        let zeros = "0".repeat((-exponent - 1) as usize);
        format!("{sign}0.{zeros}{digits}")
    }
}

/// Regroupe par milliers, à la française, une suite de chiffres entiers.
pub(crate) fn group_fr(int_digits: &str) -> String {
    let len = int_digits.chars().count();
    let mut out = String::with_capacity(len + len / 3 * 3);
    for (i, c) in int_digits.chars().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            out.push(FR_GROUP_SEPARATOR);
        }
        out.push(c);
    }
    out
}

/// Entier formaté par `toLocaleString("fr-FR")` : `12345` → `"12 345"`.
pub(crate) fn fr_integer(n: u64) -> String {
    group_fr(&n.to_string())
}

/// `value.toLocaleString("fr-FR", { maximumFractionDigits })` pour une
/// valeur finie positive : virgule décimale, zéros de fin retirés.
pub(crate) fn fr_decimal(value: f64, maximum_fraction_digits: usize) -> String {
    let fixed = to_fixed(value, maximum_fraction_digits);
    let (sign, unsigned) = match fixed.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", fixed.as_str()),
    };
    let (int_part, frac_part) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    let frac_part = frac_part.trim_end_matches('0');
    let grouped = group_fr(int_part);
    if frac_part.is_empty() {
        format!("{sign}{grouped}")
    } else {
        format!("{sign}{grouped},{frac_part}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_fixed_arrondit_au_demi_superieur_comme_javascript() {
        assert_eq!(to_fixed(1.25, 1), "1.3");
        assert_eq!(to_fixed(0.25, 1), "0.3");
        // 1.05 vaut en réalité 1.0500000000000000444… : arrondi supérieur.
        assert_eq!(to_fixed(1.05, 1), "1.1");
        // 0.35 vaut 0.34999999999999997779… : arrondi inférieur.
        assert_eq!(to_fixed(0.35, 1), "0.3");
        assert_eq!(to_fixed(2.5, 2), "2.50");
        assert_eq!(to_fixed(0.0, 1), "0.0");
        assert_eq!(to_fixed(0.04, 1), "0.0");
        assert_eq!(to_fixed(0.05, 1), "0.1");
        assert_eq!(to_fixed(9.96, 1), "10.0");
        assert_eq!(to_fixed(1023.0, 0), "1023");
    }

    #[test]
    fn to_precision_comme_javascript() {
        assert_eq!(to_precision(1.0, 2), "1.0");
        assert_eq!(to_precision(0.5, 2), "0.50");
        assert_eq!(to_precision(0.125, 2), "0.13");
        assert_eq!(to_precision(5.0, 2), "5.0");
        assert_eq!(to_precision(0.1, 2), "0.10");
        assert_eq!(to_precision(9.96, 2), "10");
        assert_eq!(to_precision(12.3, 2), "12");
        assert_eq!(to_precision(99.5, 2), "1.0e+2");
        assert_eq!(to_precision(0.000_000_123, 2), "1.2e-7");
        assert_eq!(to_precision(0.000_001_23, 2), "0.0000012");
        assert_eq!(to_precision(0.0, 2), "0.0");
    }

    #[test]
    fn fr_integer_groupe_par_milliers() {
        assert_eq!(fr_integer(0), "0");
        assert_eq!(fr_integer(999), "999");
        assert_eq!(fr_integer(1234), "1\u{202F}234");
        assert_eq!(fr_integer(1_234_567), "1\u{202F}234\u{202F}567");
    }

    #[test]
    fn fr_decimal_virgule_et_zeros_de_fin_retires() {
        assert_eq!(fr_decimal(1.2, 1), "1,2");
        assert_eq!(fr_decimal(7.0, 1), "7");
        assert_eq!(fr_decimal(80.0, 1), "80");
        assert_eq!(fr_decimal(1.25, 1), "1,3");
        assert_eq!(fr_decimal(12_345.6, 1), "12\u{202F}345,6");
    }
}
