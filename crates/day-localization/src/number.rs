// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Matching locale-aware display and parsing for editable numbers.

use crate::{format_decimal_in, format_percent_in};

/// A decimal or percentage format shared by numeric controls and readouts.
///
/// Parsing uses the current locale's decimal separator and digits; ASCII digits are also
/// accepted. Grouped input must match the locale's grouping. Percent input may omit its sign:
/// `40` and `40%` both mean `0.4`. Scientific notation, currency and units are not accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub struct NumberFormat {
    percent: bool,
    fraction_digits: usize,
}

impl NumberFormat {
    /// Decimal notation with two fraction digits.
    pub const fn decimal() -> Self {
        Self {
            percent: false,
            fraction_digits: 2,
        }
    }

    /// Percentage notation with no fraction digits; `0.4` displays as `40%` in English.
    pub const fn percent() -> Self {
        Self {
            percent: true,
            fraction_digits: 0,
        }
    }

    /// Display this many fraction digits (at most 20). Parsing retains entered precision.
    pub const fn fraction_digits(mut self, digits: usize) -> Self {
        self.fraction_digits = if digits > 20 { 20 } else { digits };
        self
    }

    /// Format using the current locale, tracking locale changes in reactive closures.
    #[must_use]
    pub fn format(self, value: f64) -> String {
        self.format_in(&crate::locale().get(), value)
    }

    /// Format in an explicit locale without subscribing to locale changes.
    #[must_use]
    pub fn format_in(self, locale: &str, value: f64) -> String {
        if self.percent {
            format_percent_in(locale, value, self.fraction_digits)
        } else {
            format_decimal_in(locale, value, self.fraction_digits)
        }
    }

    /// Parse a complete edit in the current locale. Invalid or unfinished edits return `None`.
    #[must_use]
    pub fn parse(self, text: &str) -> Option<f64> {
        self.parse_in(&crate::locale().get(), text)
    }

    /// Parse in an explicit locale. Grouping is validated rather than silently discarded.
    #[must_use]
    pub fn parse_in(self, locale: &str, text: &str) -> Option<f64> {
        // Use the same ICU data as display, including Unicode numbering-system extensions.
        let digits: Vec<char> = (0..10)
            .map(|i| {
                format_decimal_in(locale, f64::from(i), 0)
                    .chars()
                    .find(|c| !bidi(*c))
                    .unwrap_or('0')
            })
            .collect();
        let decimal = format_decimal_in(locale, 1.1, 1)
            .chars()
            .find(|c| !bidi(*c) && !digits.contains(c))?;
        let group = format_decimal_in(locale, 123_456_789.0, 0)
            .chars()
            .find(|c| !bidi(*c) && !digits.contains(c));
        let cleaned: String = text.chars().filter(|c| !bidi(*c)).collect();
        let mut text = cleaned.trim();
        if self.percent {
            // Accept a pasted locale percent sign or its ASCII keyboard equivalent at either end.
            if text.starts_with(['%', '\u{066a}']) {
                text = text[text.chars().next()?.len_utf8()..].trim();
            } else if text.ends_with(['%', '\u{066a}']) {
                text = text[..text.len() - text.chars().last()?.len_utf8()].trim();
            }
        }
        let negative = text.starts_with(['-', '\u{2212}']);
        if negative || text.starts_with('+') {
            text = &text[text.chars().next()?.len_utf8()..];
        }
        let mut ascii = String::new();
        let mut grouped_integer = String::new();
        let mut had_group = false;
        let mut fraction = false;
        for c in text.chars() {
            if let Some(d) = digits
                .iter()
                .position(|d| *d == c)
                .or_else(|| c.is_ascii_digit().then(|| (c as u8 - b'0') as usize))
            {
                let d = char::from(b'0' + d as u8);
                ascii.push(d);
                if !fraction {
                    grouped_integer.push(d);
                }
            } else if c == decimal && !fraction {
                ascii.push('.');
                fraction = true;
            } else if group.is_some_and(|g| c == g || (g.is_whitespace() && c.is_whitespace()))
                && !fraction
            {
                grouped_integer.push(',');
                had_group = true;
            } else {
                return None;
            }
        }
        if ascii.is_empty() || ascii.ends_with('.') {
            return None;
        }
        let mut value: f64 = ascii.parse().ok()?;
        if !value.is_finite() {
            return None;
        }
        if had_group {
            let integer = ascii.split('.').next()?.parse::<f64>().ok()?;
            let expected: String = format_decimal_in(locale, integer, 0)
                .chars()
                .filter(|c| !bidi(*c))
                .map(|c| {
                    digits
                        .iter()
                        .position(|d| *d == c)
                        .map_or(',', |i| char::from(b'0' + i as u8))
                })
                .collect();
            if grouped_integer != expected {
                return None;
            }
        }
        if negative {
            value = -value;
        }
        if self.percent {
            value /= 100.0;
        }
        Some(value)
    }
}

fn bidi(c: char) -> bool {
    matches!(
        c,
        '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{2066}'..='\u{2069}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn displayed_numbers_round_trip_across_locales_and_numbering_systems() {
        for locale in [
            "en-US",
            "fr",
            "de",
            "de-CH",
            "hi-IN",
            "ar-EG",
            "fa",
            "ja",
            "ko",
            "zh-CN",
            "tr",
            "en-u-nu-arab",
        ] {
            for format in [
                NumberFormat::decimal(),
                NumberFormat::percent().fraction_digits(2),
            ] {
                for value in [0.0, 0.4, -12.5, 1_234_567.25] {
                    let text = format.format_in(locale, value);
                    assert_eq!(
                        format.parse_in(locale, &text),
                        Some(value),
                        "{locale}: {text}"
                    );
                }
            }
        }
    }

    #[test]
    fn edits_respect_locale_and_reject_ambiguous_or_incomplete_input() {
        let decimal = NumberFormat::decimal();
        assert_eq!(decimal.parse_in("de", "1.234,5"), Some(1234.5));
        assert_eq!(decimal.parse_in("fr", "1 234,5"), Some(1234.5));
        assert_eq!(decimal.parse_in("ar-EG", "١٢٫٥"), Some(12.5));
        assert_eq!(decimal.parse_in("ar-EG", "12٫5"), Some(12.5));
        assert_eq!(decimal.parse_in("hi-IN", "12,34,567.5"), Some(1_234_567.5));
        assert_eq!(decimal.parse_in("en", ".5"), Some(0.5));
        for text in [
            "",
            "-",
            "1.",
            "1,2",
            "1,,234",
            "12,34,567",
            "NaN",
            "inf",
            "1e3",
            "12%",
            "1 234",
            "--2",
        ] {
            assert_eq!(decimal.parse_in("en", text), None, "{text}");
        }
        assert_eq!(decimal.parse_in("de", "1.5"), None);
        let percent = NumberFormat::percent();
        assert_eq!(percent.parse_in("en", "40"), Some(0.4));
        assert_eq!(percent.parse_in("de", "40,5 %"), Some(0.405));
        assert_eq!(percent.parse_in("ar-EG", "\u{2068}٤٠٪\u{2069}"), Some(0.4));
        assert_eq!(percent.parse_in("en", "%40%"), None);
    }
}
