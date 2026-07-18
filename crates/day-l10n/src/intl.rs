//! icu4x-backed Fluent formatting (docs/localization.md "Formatted values"): registers `NUMBER()`
//! and `DATETIME()` on every bundle and installs a bundle-wide value formatter, so plain `{ $n }`
//! interpolations AND explicit function calls render locale-correctly (grouping separators,
//! locale digit systems, CLDR date/time patterns) with zero app setup.
//!
//! Design (DESIGN.md §12.2): fluent-bundle has no icu4x integration of its own — its built-in
//! `NUMBER` only merges options onto the value and `DATETIME` is unimplemented — but it provides
//! exactly the seams this module plugs: `add_function` (returns values), `set_formatter` (renders
//! numbers), and `FluentValue::Custom`/`FluentType` (renders datetimes). Formatters are memoized
//! in each bundle's own `IntlLangMemoizer`, which carries the bundle's language — so nothing here
//! captures state, and the `Send + Sync` bounds hold trivially. Every failure path degrades to a
//! visible naive rendering; formatting never panics and never produces a blank.
//! (Prior art for the DATETIME shape: the `fluent-datetime` crate, reimplemented here to keep
//! Day's civil ISO/epoch value conventions and avoid its `jiff` dependency.)

use std::borrow::Cow;

use fluent_bundle::types::{FluentNumber, FluentNumberStyle, FluentType};
use fluent_bundle::{FluentArgs, FluentBundle, FluentResource, FluentValue};
use intl_memoizer::{IntlLangMemoizer, Memoizable};
use unic_langid::LanguageIdentifier;

/// Register the formatting seams on a freshly built bundle (called from `build_bundles` for both
/// app and core-catalog bundles).
pub(crate) fn register(bundle: &mut FluentBundle<FluentResource>) {
    bundle.set_formatter(Some(format_value));
    // fluent-bundle registers no builtins by default; an error here would only mean a duplicate,
    // which is harmless (the formatter hook still renders values).
    let _ = bundle.add_function("NUMBER", number_fn);
    let _ = bundle.add_function("DATETIME", datetime_fn);
}

// ---------------------------------------------------------------------------
// NUMBER — the function merges options onto the value (so plural `select` keeps selecting on the
// number); the bundle formatter renders every FluentValue::Number through icu_decimal.
// ---------------------------------------------------------------------------

/// `NUMBER($n, style: "percent", minimumFractionDigits: 2, …)` — the fluent-bundle builtin's
/// semantics: parse/merge ECMA-402-style options, return a Number for the formatter to render.
fn number_fn<'a>(positional: &[FluentValue<'a>], named: &FluentArgs) -> FluentValue<'a> {
    match positional.first() {
        Some(FluentValue::Number(n)) => {
            let mut n = n.clone();
            n.options.merge(named);
            FluentValue::Number(n)
        }
        Some(FluentValue::String(s)) => match s.parse::<f64>() {
            Ok(v) => {
                let mut n = FluentNumber::new(v, Default::default());
                n.options.merge(named);
                FluentValue::Number(n)
            }
            Err(_) => FluentValue::Error,
        },
        _ => FluentValue::Error,
    }
}

/// The bundle-wide value formatter (`set_formatter`): numbers go through icu_decimal; everything
/// else falls through to fluent-bundle's own rendering.
fn format_value(value: &FluentValue, intls: &IntlLangMemoizer) -> Option<String> {
    match value {
        FluentValue::Number(n) => Some(format_number(n, intls)),
        _ => None,
    }
}

/// A memoized [`icu_decimal::DecimalFormatter`] per (bundle language, grouping strategy).
struct MemoDecimalFormatter(icu_decimal::DecimalFormatter);

impl Memoizable for MemoDecimalFormatter {
    type Args = (bool,); // use_grouping
    type Error = ();
    fn construct(lang: LanguageIdentifier, args: Self::Args) -> Result<Self, ()> {
        let locale: icu_locale_core::Locale = lang.to_string().parse().map_err(|_| ())?;
        let mut opts = icu_decimal::options::DecimalFormatterOptions::default();
        opts.grouping_strategy = Some(if args.0 {
            icu_decimal::options::GroupingStrategy::Auto
        } else {
            icu_decimal::options::GroupingStrategy::Never
        });
        icu_decimal::DecimalFormatter::try_new((&locale).into(), opts)
            .map(Self)
            .map_err(|_| ())
    }
}

fn format_number(n: &FluentNumber, intls: &IntlLangMemoizer) -> String {
    use fixed_decimal_shim::*;

    let Ok(mut d) = Decimal::try_from_f64(n.value, FloatPrecision::RoundTrip) else {
        return n.as_string().into_owned(); // non-finite input — naive fallback
    };

    let o = &n.options;
    if o.style == FluentNumberStyle::Percent {
        // Scale FIRST so the fraction-digit handling below applies to the percentage value
        // (72.34% — ECMA-402 percent defaults to 0 fraction digits, see `max` below).
        d.multiply_pow10(2);
        d.trim_start(); // drop the integer-zero placeholder 0.x carries across the shift
    }
    if o.minimum_significant_digits.is_some() || o.maximum_significant_digits.is_some() {
        // Significant-digit mode (ECMA-402: overrides integer/fraction digit settings).
        let start = d.nonzero_magnitude_start();
        if let Some(max) = o.maximum_significant_digits {
            d.round(start - (max.max(1) as i16) + 1);
        }
        if let Some(min) = o.minimum_significant_digits {
            d.pad_end(start - (min.max(1) as i16) + 1);
        }
    } else {
        // Fraction-digit mode. ECMA-402 defaults: at most 3 fraction digits for decimals (keeps
        // `{ 0.1 + 0.2 }`-style float noise out of translations), 0 for percent.
        let min = o.minimum_fraction_digits.unwrap_or(0);
        let default_max = if o.style == FluentNumberStyle::Percent {
            0
        } else {
            3
        };
        let max = o.maximum_fraction_digits.unwrap_or(default_max.max(min));
        d.round(-(max.min(20) as i16));
        d.trim_end(); // round() extends the range with trailing zeros — keep only what min asks
        d.pad_end(-(min.min(20) as i16));
    }
    if let Some(mi) = o.minimum_integer_digits {
        d.pad_start(mi.min(21) as i16);
    }

    let formatted = intls
        .with_try_get::<MemoDecimalFormatter, _, _>((o.use_grouping,), |f| f.0.format_to_string(&d))
        .unwrap_or_else(|_| n.as_string().into_owned());

    if o.style == FluentNumberStyle::Percent {
        // v1 approximation (icu4x percent formatting is still experimental): append the percent
        // sign, using the Arabic form when the digits themselves are Arabic-Indic.
        let arabic = formatted
            .chars()
            .any(|c| ('\u{0660}'..='\u{0669}').contains(&c));
        format!("{formatted}{}", if arabic { '\u{066A}' } else { '%' })
    } else {
        formatted
    }
}

/// `fixed_decimal` reaches this crate re-exported through `icu_decimal::input` — alias the bits
/// we use so the code reads plainly.
mod fixed_decimal_shim {
    pub(super) use icu_decimal::input::Decimal;
    // FloatPrecision lives next to Decimal in the re-exported fixed_decimal surface.
    pub(super) use icu_decimal::input::FloatPrecision;
}

// ---------------------------------------------------------------------------
// DATETIME — a FluentValue::Custom carrying the parsed civil value + options; rendering happens
// in FluentType::as_string with the bundle's memoizer (locale) via icu_datetime.
// ---------------------------------------------------------------------------

/// The parsed argument of a `DATETIME(...)` call: a civil (zoneless, proleptic-Gregorian) date
/// and/or wall-clock time, plus the requested styles.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FluentDateTime {
    date: Option<(i32, u8, u8)>,
    time: Option<(u8, u8, u8)>,
    /// `dateStyle:` — None when the input has no date part or `dateStyle: "none"`.
    date_style: Option<icu_datetime::options::Length>,
    /// `timeStyle:` — None when the input has no time part or `timeStyle: "none"`.
    time_style: Option<icu_datetime::options::Length>,
}

/// `DATETIME($when, dateStyle: "long", timeStyle: "short")` — accepts ISO-8601 strings
/// (`"2026-07-18"`, `"14:45"`, `"14:45:30"`, `"2026-07-18T14:45[:30]"`) or a number of epoch
/// SECONDS rendered as UTC civil time. Unparseable input echoes back unformatted (visible).
fn datetime_fn<'a>(positional: &[FluentValue<'a>], named: &FluentArgs) -> FluentValue<'a> {
    let parsed = match positional.first() {
        Some(FluentValue::String(s)) => parse_iso(s),
        Some(FluentValue::Number(n)) if n.value.is_finite() => {
            Some(from_epoch_seconds(n.value as i64))
        }
        _ => None,
    };
    let Some((date, time)) = parsed else {
        return positional.first().cloned().unwrap_or(FluentValue::Error);
    };

    let style = |key: &str| -> Option<Option<icu_datetime::options::Length>> {
        match named.get(key) {
            Some(FluentValue::String(s)) => match s.as_ref() {
                "full" | "long" => Some(Some(icu_datetime::options::Length::Long)),
                "medium" => Some(Some(icu_datetime::options::Length::Medium)),
                "short" => Some(Some(icu_datetime::options::Length::Short)),
                "none" => Some(None),
                _ => None, // unknown value — fall back to the input-shape default
            },
            _ => None,
        }
    };
    // Defaults by input shape: a date renders dateStyle medium, a time renders timeStyle short.
    let date_style = style("dateStyle")
        .unwrap_or(date.map(|_| icu_datetime::options::Length::Medium))
        .filter(|_| date.is_some());
    let time_style = style("timeStyle")
        .unwrap_or(time.map(|_| icu_datetime::options::Length::Short))
        .filter(|_| time.is_some());

    FluentValue::Custom(Box::new(FluentDateTime {
        date,
        time,
        date_style,
        time_style,
    }))
}

type DateParts = (Option<(i32, u8, u8)>, Option<(u8, u8, u8)>);

/// Strict ISO-8601: `YYYY-MM-DD`, `HH:MM[:SS]`, or `YYYY-MM-DDTHH:MM[:SS]` (also accepts a space
/// separator). Returns (date, time) — at least one part present.
fn parse_iso(s: &str) -> Option<DateParts> {
    let s = s.trim();
    if let Some((d, t)) = s.split_once(['T', ' ']) {
        return Some((Some(parse_date(d)?), Some(parse_time(t)?)));
    }
    if s.contains('-') {
        return Some((Some(parse_date(s)?), None));
    }
    Some((None, Some(parse_time(s)?)))
}

fn parse_date(s: &str) -> Option<(i32, u8, u8)> {
    let mut it = s.split('-');
    let (y, m, d) = (it.next()?, it.next()?, it.next()?);
    if it.next().is_some()
        || y.len() != 4
        || m.len() != 2
        || d.len() != 2
        || [y, m, d]
            .iter()
            .any(|p| !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    let (y, m, d) = (y.parse().ok()?, m.parse().ok()?, d.parse().ok()?);
    ((1..=12).contains(&m) && (1..=31).contains(&d)).then_some((y, m, d))
}

fn parse_time(s: &str) -> Option<(u8, u8, u8)> {
    let mut it = s.split(':');
    let (h, m) = (it.next()?, it.next()?);
    let sec = it.next();
    if it.next().is_some()
        || h.len() != 2
        || m.len() != 2
        || sec.is_some_and(|x| x.len() != 2)
        || [Some(h), Some(m), sec]
            .iter()
            .flatten()
            .any(|p| !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    let (h, m, s) = (
        h.parse().ok()?,
        m.parse().ok()?,
        sec.map_or(Some(0), |x| x.parse().ok())?,
    );
    (h < 24 && m < 60 && s < 60).then_some((h, m, s))
}

/// Epoch seconds → UTC civil date+time (Howard Hinnant's civil_from_days).
fn from_epoch_seconds(secs: i64) -> DateParts {
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u8;
    let year = (y + if month <= 2 { 1 } else { 0 }) as i32;
    (
        Some((year, month, day)),
        Some(((sod / 3600) as u8, (sod / 60 % 60) as u8, (sod % 60) as u8)),
    )
}

impl FluentDateTime {
    /// The formatter cache key: which parts render, at which lengths, and to what time precision.
    fn key(&self) -> (u8, u8) {
        let enc = |l: Option<icu_datetime::options::Length>| match l {
            None => 0u8,
            Some(icu_datetime::options::Length::Long) => 1,
            Some(icu_datetime::options::Length::Short) => 3,
            // Length is #[non_exhaustive]; Medium is also its Default.
            Some(_) => 2,
        };
        (enc(self.date_style), enc(self.time_style))
    }

    /// ISO fallback rendering for every failure path (visible, locale-independent).
    fn iso(&self) -> String {
        let mut out = String::new();
        if let Some((y, m, d)) = self.date {
            out.push_str(&format!("{y:04}-{m:02}-{d:02}"));
        }
        if let Some((h, mi, s)) = self.time {
            if !out.is_empty() {
                out.push(' ');
            }
            if s == 0 {
                out.push_str(&format!("{h:02}:{mi:02}"));
            } else {
                out.push_str(&format!("{h:02}:{mi:02}:{s:02}"));
            }
        }
        out
    }

    fn format(&self, intls: &IntlLangMemoizer) -> String {
        intls
            .with_try_get::<MemoDateTimeFormatter, _, _>((self.key(),), |f| {
                let date = self.date.unwrap_or((1970, 1, 1));
                let time = self.time.unwrap_or((0, 0, 0));
                let (Ok(d), Ok(t)) = (
                    icu_calendar::Date::try_new_gregorian(date.0, date.1, date.2),
                    icu_time::Time::try_new(time.0, time.1, time.2, 0),
                ) else {
                    return self.iso();
                };
                f.0.format(&icu_time::DateTime { date: d, time: t })
                    .to_string()
            })
            .unwrap_or_else(|_| self.iso())
    }
}

impl FluentType for FluentDateTime {
    fn duplicate(&self) -> Box<dyn FluentType + Send> {
        Box::new(self.clone())
    }

    fn as_string(&self, intls: &IntlLangMemoizer) -> Cow<'static, str> {
        self.format(intls).into()
    }

    fn as_string_threadsafe(
        &self,
        _intls: &intl_memoizer::concurrent::IntlLangMemoizer,
    ) -> Cow<'static, str> {
        // Day's bundles are the non-concurrent kind; the threadsafe path (unused here) renders
        // the locale-independent ISO form rather than duplicating the formatter machinery.
        self.iso().into()
    }
}

/// A memoized [`icu_datetime::FixedCalendarDateTimeFormatter`] per (bundle language, style key).
/// Fixed-Gregorian keeps the data footprint at the small end (no all-calendars fallback), which
/// matches Day's civil-value conventions (day-piece-datetime speaks proleptic Gregorian too).
struct MemoDateTimeFormatter(
    icu_datetime::FixedCalendarDateTimeFormatter<
        icu_calendar::cal::Gregorian,
        icu_datetime::fieldsets::enums::CompositeDateTimeFieldSet,
    >,
);

impl Memoizable for MemoDateTimeFormatter {
    type Args = ((u8, u8),); // FluentDateTime::key()
    type Error = ();
    fn construct(lang: LanguageIdentifier, args: Self::Args) -> Result<Self, ()> {
        use icu_datetime::fieldsets::builder::{DateFields, FieldSetBuilder};
        use icu_datetime::options::{Length, TimePrecision};

        let locale: icu_locale_core::Locale = lang.to_string().parse().map_err(|_| ())?;
        let dec = |v: u8| match v {
            1 => Some(Length::Long),
            2 => Some(Length::Medium),
            3 => Some(Length::Short),
            _ => None,
        };
        let (date_style, time_style) = (dec(args.0.0), dec(args.0.1));

        let mut b = FieldSetBuilder::new();
        // Length applies to the date part; time renders at the precision below.
        b.length = date_style.or(time_style);
        if date_style.is_some() {
            b.date_fields = Some(DateFields::YMD);
        }
        if let Some(ts) = time_style {
            // `short` shows hour+minute; longer styles include seconds.
            b.time_precision = Some(match ts {
                Length::Short => TimePrecision::Minute,
                _ => TimePrecision::Second,
            });
        }
        let field_set = b.build_composite_datetime().map_err(|_| ())?;
        icu_datetime::FixedCalendarDateTimeFormatter::try_new((&locale).into(), field_set)
            .map(Self)
            .map_err(|_| ())
    }
}
