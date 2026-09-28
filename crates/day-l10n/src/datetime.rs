// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Locale-aware dates and times outside a message (docs/localization.md "Dates outside a
//! message"), backed by the same icu4x formatter as the Fluent `DATETIME()` builtin.
//!
//! A chart's time axis, a table's date column and a "last updated" readout all need a date in the
//! reader's own form (`Mar 5` in English, `5 mars` in French, `05.03.` in German, `03/05` in
//! Japanese, and `3:30 PM` against `15:30` for the clock) with no message to hang it on. Assembled
//! by hand from an English month list it is wrong in most locales, in the words, in the order,
//! and in the hour cycle.
//!
//! [`DateFields`] names *which* parts to show, which is the decision the caller actually has (an
//! axis spanning years shows years, one spanning hours shows clock times); the locale decides how
//! they are written. Formatters are cached per (locale, fields) in a thread-local; any failure
//! degrades to an ISO-8601 rendering rather than an error or a blank.

use std::cell::RefCell;
use std::collections::HashMap;

use icu_calendar::cal::Gregorian;
use icu_datetime::FixedCalendarDateTimeFormatter;
use icu_datetime::fieldsets::enums::CompositeDateTimeFieldSet;

/// Which parts of an instant to write. The locale decides their order, words and punctuation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DateFields {
    /// `2026`.
    Year,
    /// `Mar 2026` · `mars 2026` · `03/2026` · `2026/03`.
    YearMonth,
    /// `Mar` · `mars` · `Mär` · `3月`: a month on its own, for a column of months (a calendar heat
    /// map's header) whose years are elsewhere.
    Month,
    /// `Mar 5` · `5 mars` · `05.03.` · `03/05`.
    MonthDay,
    /// `Mar 5, 2026` · `5 mars 2026` · `05.03.2026` · `2026/03/05`.
    YearMonthDay,
    /// `3:30 PM` · `15:30`: the clock in the locale's own hour cycle.
    HourMinute,
    /// `3:30:15 PM` · `15:30:15`.
    HourMinuteSecond,
}

type Formatter = FixedCalendarDateTimeFormatter<Gregorian, CompositeDateTimeFieldSet>;

day_reactive::tls_slots! {
    datetime;
    /// `None` = construction failed for that locale (negative-cached to avoid re-trying per call).
    static FORMATTERS: RefCell<HashMap<(String, DateFields), Option<Formatter>>> =
        RefCell::new(HashMap::new());
}

fn build(locale: &str, fields: DateFields) -> Option<Formatter> {
    use icu_datetime::fieldsets::builder::{DateFields as Icu, FieldSetBuilder};
    use icu_datetime::options::{Length, TimePrecision};

    let parsed: icu_locale_core::Locale = locale.parse().ok()?;
    let mut b = FieldSetBuilder::new();
    // Medium: the locale's medium date form, an abbreviated month name where the locale writes
    // one (`Mar 5`, `5 mars`) and its own numeric form where it does not (`05.03.`, `03/05`).
    // Long would spell English months out in full, too wide for an axis; short would turn
    // `Mar 5` into the ambiguous `3/5`.
    b.length = Some(Length::Medium);
    match fields {
        DateFields::Year => b.date_fields = Some(Icu::Y),
        DateFields::YearMonth => b.date_fields = Some(Icu::YM),
        DateFields::Month => b.date_fields = Some(Icu::M),
        DateFields::MonthDay => b.date_fields = Some(Icu::MD),
        DateFields::YearMonthDay => b.date_fields = Some(Icu::YMD),
        DateFields::HourMinute => b.time_precision = Some(TimePrecision::Minute),
        DateFields::HourMinuteSecond => b.time_precision = Some(TimePrecision::Second),
    }
    let field_set = b.build_composite_datetime().ok()?;
    Formatter::try_new((&parsed).into(), field_set).ok()
}

/// The ISO-8601 rendering of the requested fields, for every failure path.
fn iso(fields: DateFields, (y, m, d): (i32, u8, u8), (hh, mm, ss): (u8, u8, u8)) -> String {
    match fields {
        DateFields::Year => format!("{y:04}"),
        DateFields::YearMonth => format!("{y:04}-{m:02}"),
        DateFields::Month => format!("{m:02}"),
        DateFields::MonthDay => format!("{m:02}-{d:02}"),
        DateFields::YearMonthDay => format!("{y:04}-{m:02}-{d:02}"),
        DateFields::HourMinute => format!("{hh:02}:{mm:02}"),
        DateFields::HourMinuteSecond => format!("{hh:02}:{mm:02}:{ss:02}"),
    }
}

/// Write the instant `epoch_seconds` (seconds since 1970-01-01T00:00:00Z, read as UTC civil
/// time, the convention of the Fluent `DATETIME()` builtin and of chart time axes) in `locale`,
/// showing `fields` (untracked).
pub fn format_date_in(locale: &str, epoch_seconds: f64, fields: DateFields) -> String {
    if !epoch_seconds.is_finite() {
        return format!("{epoch_seconds}");
    }
    let (Some(date), Some(time)) = crate::intl::from_epoch_seconds(epoch_seconds.floor() as i64)
    else {
        return format!("{epoch_seconds}");
    };
    let (Ok(d), Ok(t)) = (
        icu_calendar::Date::try_new_gregorian(date.0, date.1, date.2),
        icu_time::Time::try_new(time.0, time.1, time.2, 0),
    ) else {
        return iso(fields, date, time);
    };
    FORMATTERS.with(|c| {
        let mut map = c.borrow_mut();
        let formatter = map
            .entry((locale.to_string(), fields))
            .or_insert_with(|| build(locale, fields));
        match formatter {
            Some(f) => f
                .format(&icu_time::DateTime { date: d, time: t })
                .to_string(),
            None => iso(fields, date, time),
        }
    })
}

/// [`format_date_in`] in the current locale (tracked, like [`crate::format_decimal`]), so a label
/// inside a reactive closure re-renders when the locale switches.
pub fn format_date(epoch_seconds: f64, fields: DateFields) -> String {
    let locale = crate::locale().get();
    format_date_in(&locale, epoch_seconds, fields)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-03-05T15:30:15Z.
    const T: f64 = 1_772_724_615.0;

    fn f(locale: &str, fields: DateFields) -> String {
        // The exact spaces (a no-break space before `PM`, a narrow one in French) are CLDR's to
        // choose; the assertions compare with them folded to plain spaces.
        format_date_in(locale, T, fields).replace(['\u{a0}', '\u{202f}'], " ")
    }

    #[test]
    fn a_month_and_day_are_written_the_locales_way() {
        assert_eq!(f("en", DateFields::MonthDay), "Mar 5");
        assert_eq!(f("fr", DateFields::MonthDay), "5 mars");
        // German's and Japanese medium dates are numeric in CLDR: the locale's own form.
        assert_eq!(f("de", DateFields::MonthDay), "05.03.");
        assert_eq!(f("ja", DateFields::MonthDay), "03/05");
    }

    #[test]
    fn a_month_and_year_are_written_the_locales_way() {
        assert_eq!(f("en", DateFields::YearMonth), "Mar 2026");
        assert_eq!(f("fr", DateFields::YearMonth), "mars 2026");
        assert_eq!(f("de", DateFields::YearMonth), "03/2026");
        assert_eq!(f("ja", DateFields::YearMonth), "2026/03");
        assert_eq!(f("en", DateFields::Year), "2026");
        assert_eq!(f("ja", DateFields::Year), "2026年");
    }

    #[test]
    fn a_month_alone_is_the_locales_abbreviated_name() {
        assert_eq!(f("en", DateFields::Month), "Mar");
        assert_eq!(f("fr", DateFields::Month), "mars");
        assert_eq!(f("de", DateFields::Month), "Mär");
        assert_eq!(f("ja", DateFields::Month), "3月");
        assert_eq!(f("ru", DateFields::Month), "март");
    }

    #[test]
    fn a_full_date_follows_the_locales_order() {
        assert_eq!(f("en", DateFields::YearMonthDay), "Mar 5, 2026");
        assert_eq!(f("fr", DateFields::YearMonthDay), "5 mars 2026");
        assert_eq!(f("de", DateFields::YearMonthDay), "05.03.2026");
    }

    #[test]
    fn the_clock_keeps_the_locales_hour_cycle() {
        assert_eq!(f("en", DateFields::HourMinute), "3:30 PM");
        assert_eq!(f("fr", DateFields::HourMinute), "15:30");
        assert_eq!(f("de", DateFields::HourMinute), "15:30");
        assert_eq!(f("en", DateFields::HourMinuteSecond), "3:30:15 PM");
        assert_eq!(f("fr", DateFields::HourMinuteSecond), "15:30:15");
    }

    #[test]
    fn an_instant_before_the_epoch_and_a_non_finite_one_degrade_sensibly() {
        // 1969-12-31T23:00:00Z: the day before the epoch, not the epoch's own day.
        assert_eq!(
            format_date_in("en", -3600.0, DateFields::YearMonthDay),
            "Dec 31, 1969"
        );
        assert_eq!(format_date_in("en", f64::NAN, DateFields::Year), "NaN");
    }

    #[test]
    fn an_unparseable_locale_falls_back_to_iso() {
        assert_eq!(
            format_date_in("", T, DateFields::YearMonthDay),
            "2026-03-05"
        );
        assert_eq!(format_date_in("", T, DateFields::HourMinute), "15:30");
    }

    #[test]
    fn the_tracked_form_follows_the_current_locale() {
        crate::set_locale("fr");
        assert_eq!(
            format_date(T, DateFields::MonthDay).replace('\u{a0}', " "),
            "5 mars"
        );
        crate::set_locale("en");
        assert_eq!(format_date(T, DateFields::MonthDay), "Mar 5");
    }
}
