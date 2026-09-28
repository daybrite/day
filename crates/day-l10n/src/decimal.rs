// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Locale-aware decimal formatting (docs/localization.md "Numbers outside a message"), backed by
//! icu4x's `DecimalFormatter`, the same engine the Fluent `NUMBER()` builtin uses, reached
//! without going through a message.
//!
//! A bare number rendered with `format!("{v}")` carries an English decimal point and no digit
//! grouping, which is wrong in most locales and unreadable past four digits in any of them. Chart
//! axes, table columns and readouts all need the grouped form and none of them have a message to
//! hang it on, so the formatter is public here rather than reachable only through `tr()`.
//!
//! Formatters are cached per (locale, fraction digits) in a thread-local, the way `collate` caches
//! collators; on any failure the value degrades to `format!` rather than erroring.

use std::cell::RefCell;
use std::collections::HashMap;

use icu_decimal::DecimalFormatter;
use icu_decimal::input::{Decimal, FloatPrecision};
use icu_decimal::options::{DecimalFormatterOptions, GroupingStrategy};

day_reactive::tls_slots! {
    decimal;
    /// `None` = construction failed for that locale (negative-cached to avoid re-trying per call).
    static FORMATTERS: RefCell<HashMap<(String, bool), Option<DecimalFormatter>>> =
        RefCell::new(HashMap::new());

}

fn with_formatter<R>(
    locale: &str,
    grouping: bool,
    f: impl FnOnce(Option<&DecimalFormatter>) -> R,
) -> R {
    FORMATTERS.with(|c| {
        let mut map = c.borrow_mut();
        let entry = map
            .entry((locale.to_string(), grouping))
            .or_insert_with(|| {
                let parsed: icu_locale_core::Locale = locale.parse().ok()?;
                let mut opts = DecimalFormatterOptions::default();
                opts.grouping_strategy = Some(if grouping {
                    GroupingStrategy::Auto
                } else {
                    GroupingStrategy::Never
                });
                DecimalFormatter::try_new((&parsed).into(), opts).ok()
            });
        f(entry.as_ref())
    })
}

/// Render `v` in `locale`, rounded to `fraction_digits` and grouped by that locale's own rule
/// (untracked). `1234.5` is `1,234.5` in `en`, `1 234,5` in `fr`, `1.234,5` in `de`.
///
/// Grouping follows the locale's CLDR rule rather than a fixed every-three: `en-IN` groups the
/// lakh way, and several locales suppress grouping for four-digit numbers entirely.
pub fn format_decimal_in(locale: &str, v: f64, fraction_digits: usize) -> String {
    if !v.is_finite() {
        return format!("{v}");
    }
    let Ok(mut d) = Decimal::try_from_f64(v, FloatPrecision::RoundTrip) else {
        return format!("{v:.*}", fraction_digits);
    };
    let places = fraction_digits.min(20) as i16;
    d.round(-places);
    // `round` extends the range with trailing zeros; padding back to the same place keeps exactly
    // the digits asked for, so a column of values stays aligned.
    d.trim_end();
    d.pad_end(-places);
    with_formatter(locale, true, |f| match f {
        Some(f) => f.format_to_string(&d),
        None => format!("{v:.*}", fraction_digits),
    })
}

/// Render `v` in the current locale (tracked, like [`crate::compare`]), so a label inside a
/// reactive closure re-renders when the locale switches.
pub fn format_decimal(v: f64, fraction_digits: usize) -> String {
    let locale = crate::locale().get();
    format_decimal_in(&locale, v, fraction_digits)
}

/// Where a locale puts the percent sign, and what separates it from the number: CLDR's standard
/// percent pattern for the language, as (before, after).
///
/// icu4x's percent formatter is still experimental (outside the stable components this crate
/// depends on), so the placement is this table of CLDR patterns, keyed by language, for the
/// locales whose pattern is not a bare trailing `%`. The number itself is always the stable
/// `DecimalFormatter`'s, so grouping, the decimal mark and the digit system are exact; Arabic-
/// Indic digits take the Arabic percent sign.
pub(crate) fn percent_affixes(locale: &str, number: &str) -> (&'static str, &'static str) {
    let lang = locale.split(['-', '_']).next().unwrap_or("");
    let region = locale.split(['-', '_']).nth(1).unwrap_or("");
    if number
        .chars()
        .any(|c| ('\u{0660}'..='\u{0669}').contains(&c))
    {
        return ("", "\u{066A}");
    }
    match lang {
        // A narrow no-break space: `50 %`.
        "fr" if region != "CH" => ("", "\u{202F}%"),
        // A no-break space: `50 %`. Swiss and Liechtenstein German write `50%`.
        "de" if !matches!(region, "CH" | "LI") => ("", "\u{A0}%"),
        "es" | "ca" | "sv" | "nb" | "nn" | "no" | "da" | "fi" | "ru" | "cs" | "sk" => {
            ("", "\u{A0}%")
        }
        // The sign first: `%50`, and in Basque `% 50`.
        "tr" => ("%", ""),
        "eu" => ("%\u{A0}", ""),
        _ => ("", "%"),
    }
}

/// Render the fraction `v` as a percentage in `locale` (`0.5` is `50%` in `en`, `50 %` in `fr`,
/// `%50` in `tr`), rounded to `fraction_digits` places of the percentage and grouped by the
/// locale's rule (untracked).
pub fn format_percent_in(locale: &str, v: f64, fraction_digits: usize) -> String {
    let number = format_decimal_in(locale, v * 100.0, fraction_digits);
    if !v.is_finite() {
        return number;
    }
    let (before, after) = percent_affixes(locale, &number);
    format!("{before}{number}{after}")
}

/// [`format_percent_in`] in the current locale (tracked), so a label inside a reactive closure
/// re-renders when the locale switches.
pub fn format_percent(v: f64, fraction_digits: usize) -> String {
    let locale = crate::locale().get();
    format_percent_in(&locale, v, fraction_digits)
}

#[cfg(test)]
mod tests {
    use super::{format_decimal_in, format_percent_in};

    #[test]
    fn a_percentage_puts_its_sign_where_the_locale_does() {
        assert_eq!(format_percent_in("en", 0.5, 0), "50%");
        assert_eq!(format_percent_in("en", 0.1234, 1), "12.3%");
        assert_eq!(format_percent_in("fr", 0.5, 0), "50\u{202F}%");
        assert_eq!(format_percent_in("fr", 0.1234, 1), "12,3\u{202F}%");
        assert_eq!(format_percent_in("de", 0.5, 0), "50\u{A0}%");
        assert_eq!(format_percent_in("de-CH", 0.5, 0), "50%");
        assert_eq!(format_percent_in("tr", 0.5, 0), "%50");
        assert_eq!(format_percent_in("ja", 0.5, 0), "50%");
        // Grouped like any other number, and negative on the number's side of the sign.
        assert_eq!(format_percent_in("en", 12.5, 0), "1,250%");
        assert_eq!(format_percent_in("en", -0.25, 0), "-25%");
        assert_eq!(format_percent_in("en", f64::NAN, 0), "NaN");
    }

    #[test]
    fn grouping_and_separators_follow_the_locale() {
        assert_eq!(format_decimal_in("en", 1_234_567.0, 0), "1,234,567");
        assert_eq!(format_decimal_in("de", 1_234_567.0, 0), "1.234.567");
        // fr uses a comma for the decimal mark and a narrow no-break space between groups; the
        // exact space is CLDR's to choose, so the assertion asks what kind it is, not which.
        let fr = format_decimal_in("fr", 1234.5, 1);
        assert!(
            fr.ends_with("234,5"),
            "fr marks the decimal with a comma: {fr}"
        );
        let sep = fr
            .chars()
            .nth(1)
            .expect("a grouped four-digit number has a separator");
        assert!(sep.is_whitespace(), "fr groups with a space: {fr:?}");
    }

    #[test]
    fn fraction_digits_are_exact_in_both_directions() {
        assert_eq!(format_decimal_in("en", 2.0, 2), "2.00");
        assert_eq!(format_decimal_in("en", 2.345, 1), "2.3");
        assert_eq!(format_decimal_in("en", 0.0, 0), "0");
    }

    #[test]
    fn an_unparseable_locale_still_renders_the_number() {
        // Nothing here is allowed to swallow the value: an unusable locale falls back to root
        // data or to `format!`, and either way the digits come out.
        let s = format_decimal_in("@@@", 1500.0, 0);
        assert!(s.contains('1') && s.contains("500"), "{s}");
    }

    #[test]
    fn non_finite_values_do_not_panic() {
        assert_eq!(format_decimal_in("en", f64::NAN, 0), "NaN");
        assert_eq!(format_decimal_in("en", f64::INFINITY, 0), "inf");
    }
}
