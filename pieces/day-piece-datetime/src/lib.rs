//! day-piece-datetime — native date & time pickers for every Day backend (docs/datepicker.md;
//! DESIGN.md §15 tier 1+shim).
//!
//! ```ignore
//! let date = Signal::new(DayDate::new(2026, 7, 18).unwrap());
//! let time = Signal::new(DayTime::new(9, 30, 0).unwrap());
//! column((
//!     date_picker(date),                      // Compact: field/button → platform chooser
//!     date_picker(date).inline(),             // Inline: embedded calendar / wheels
//!     time_picker(time),
//!     row((date_picker(date), time_picker(time))),  // combined date+time = composition
//! ))
//! ```
//!
//! TWO pieces — [`date_picker`] and [`time_picker`] — rather than one combined date-time piece: a
//! single combined control exists on only 3 of the 7 toolkits (`NSDatePicker`,
//! `UIDatePicker.dateAndTime`, `QDateTimeEdit`), while separate date and time controls realize
//! natively on ALL of them. Each piece maps a small style intent ([`Style::Compact`] /
//! [`Style::Inline`]) to the platform's closest idiomatic control — never promising identical
//! chrome (Android's Material chooser is a modal dialog, iOS's a popover, Qt's a calendar popup;
//! that difference IS the platform, docs/datepicker.md has the full table).
//!
//! The bound signal is TWO-WAY: a user pick sets it; the app setting it updates the native
//! control. Values are civil (zoneless) [`DayDate`]/[`DayTime`] — time zones are the app's
//! business. The piece's node also accepts `Event::TextChanged` carrying an ISO value
//! (`"2026-07-18"` / `"09:30"`) as a synthetic set, so dayscript's existing `input:` step drives
//! any picker on any backend.

use day_core::{BuildCx, Flex, Piece, RNode, with_tree};
use day_reactive::{Signal, bind_seeded};
use day_spec::Event;

pub const DATE_KIND: &str = "day.piece.datepicker";
pub const TIME_KIND: &str = "day.piece.timepicker";

// ---------------------------------------------------------------------------
// Values: proleptic-Gregorian civil date + wall-clock time. No chrono dependency — ISO-8601
// strings and epoch days / seconds-of-day are the interchange forms across native boundaries.
// ---------------------------------------------------------------------------

/// A civil (zoneless) calendar date, proleptic Gregorian. Ordered chronologically; `Display` and
/// [`DayDate::parse_iso`] speak ISO-8601 (`YYYY-MM-DD`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DayDate {
    pub year: i32,
    pub month: u8,
    pub day: u8,
}

impl Default for DayDate {
    fn default() -> Self {
        DayDate {
            year: 1970,
            month: 1,
            day: 1,
        }
    }
}

impl DayDate {
    /// A validated date (`month` 1–12, `day` within the month, leap-aware) — `None` otherwise.
    pub fn new(year: i32, month: u8, day: u8) -> Option<Self> {
        if (1..=12).contains(&month) && day >= 1 && day <= days_in_month(year, month) {
            Some(DayDate { year, month, day })
        } else {
            None
        }
    }

    /// Today's date derived from the system clock in UTC (Day carries no time-zone database; an
    /// app that must roll the date at LOCAL midnight should compute it with its own zone source).
    pub fn today() -> Self {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        Self::from_epoch_days(secs.div_euclid(86_400))
    }

    /// Days since 1970-01-01 (negative before). The numeric interchange form across native
    /// boundaries (`Event::Custom.num`).
    pub fn to_epoch_days(self) -> i64 {
        // Howard Hinnant's days_from_civil.
        let y = self.year as i64 - if self.month <= 2 { 1 } else { 0 };
        let era = if y >= 0 { y } else { y - 399 } / 400;
        let yoe = y - era * 400;
        let m = self.month as i64;
        let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + self.day as i64 - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146097 + doe - 719468
    }

    /// The date `days` after 1970-01-01 (inverse of [`DayDate::to_epoch_days`]).
    pub fn from_epoch_days(days: i64) -> Self {
        // Howard Hinnant's civil_from_days.
        let z = days + 719468;
        let era = if z >= 0 { z } else { z - 146096 } / 146097;
        let doe = z - era * 146097;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
        let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u8;
        DayDate {
            year: (y + if month <= 2 { 1 } else { 0 }) as i32,
            month,
            day,
        }
    }

    /// Parse strict ISO-8601 `YYYY-MM-DD` (the form `Display` writes). `None` on anything else.
    pub fn parse_iso(s: &str) -> Option<Self> {
        let mut it = s.split('-');
        let (y, m, d) = (it.next()?, it.next()?, it.next()?);
        if it.next().is_some() || y.len() != 4 || m.len() != 2 || d.len() != 2 {
            return None;
        }
        if [y, m, d]
            .iter()
            .any(|p| !p.bytes().all(|b| b.is_ascii_digit()))
        {
            return None;
        }
        Self::new(y.parse().ok()?, m.parse().ok()?, d.parse().ok()?)
    }

    /// This date limited to the inclusive `[min, max]` range (either bound optional).
    pub fn clamped(self, min: Option<DayDate>, max: Option<DayDate>) -> Self {
        let mut d = self;
        if let Some(min) = min
            && d < min
        {
            d = min;
        }
        if let Some(max) = max
            && d > max
        {
            d = max;
        }
        d
    }
}

impl std::fmt::Display for DayDate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// A civil (zoneless) wall-clock time. `Display` and [`DayTime::parse_iso`] speak ISO-8601
/// (`HH:MM`, or `HH:MM:SS` when seconds are nonzero).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DayTime {
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl DayTime {
    /// A validated time (`hour` 0–23, `minute`/`second` 0–59) — `None` otherwise.
    pub fn new(hour: u8, minute: u8, second: u8) -> Option<Self> {
        if hour < 24 && minute < 60 && second < 60 {
            Some(DayTime {
                hour,
                minute,
                second,
            })
        } else {
            None
        }
    }

    /// Seconds since midnight — the numeric interchange form across native boundaries.
    pub fn seconds_of_day(self) -> i64 {
        self.hour as i64 * 3600 + self.minute as i64 * 60 + self.second as i64
    }

    /// The time `secs` after midnight (inverse of [`DayTime::seconds_of_day`]; out-of-range wraps
    /// into the day).
    pub fn from_seconds_of_day(secs: i64) -> Self {
        let s = secs.rem_euclid(86_400);
        DayTime {
            hour: (s / 3600) as u8,
            minute: (s / 60 % 60) as u8,
            second: (s % 60) as u8,
        }
    }

    /// Parse ISO-8601 `HH:MM` or `HH:MM:SS`. `None` on anything else.
    pub fn parse_iso(s: &str) -> Option<Self> {
        let mut it = s.split(':');
        let (h, m) = (it.next()?, it.next()?);
        let sec = it.next();
        if it.next().is_some() || h.len() != 2 || m.len() != 2 {
            return None;
        }
        if let Some(sec) = sec
            && sec.len() != 2
        {
            return None;
        }
        if [Some(h), Some(m), sec]
            .iter()
            .flatten()
            .any(|p| !p.bytes().all(|b| b.is_ascii_digit()))
        {
            return None;
        }
        Self::new(
            h.parse().ok()?,
            m.parse().ok()?,
            sec.map_or(Some(0), |s| s.parse().ok())?,
        )
    }
}

impl std::fmt::Display for DayTime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.second == 0 {
            write!(f, "{:02}:{:02}", self.hour, self.minute)
        } else {
            write!(f, "{:02}:{:02}:{:02}", self.hour, self.minute, self.second)
        }
    }
}

// ---------------------------------------------------------------------------
// The pieces
// ---------------------------------------------------------------------------

/// How the picker presents (docs/datepicker.md has the per-toolkit table). Like `PickerStyle`,
/// each intent maps to the platform's closest idiomatic control — chrome differs by design.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Style {
    /// The platform default — [`Style::Compact`] everywhere today.
    #[default]
    Automatic,
    /// A field/button showing the value that summons a transient chooser (NSDatePicker
    /// textFieldAndStepper / UIDatePicker .compact / a Material dialog launcher / QDateEdit with
    /// calendar popup / CalendarDatePicker flyout / GtkMenuButton+calendar popover /
    /// ARKUI CalendarPicker).
    Compact,
    /// An embedded calendar / clock / wheels (NSDatePicker graphical / UIDatePicker .inline /
    /// framework DatePicker+TimePicker widgets / QCalendarWidget / CalendarView / GtkCalendar /
    /// ARKUI DatePicker wheels).
    Inline,
}

/// Full props (realize) for the date picker. `style`/`min`/`max` are set once at build; only the
/// value patches.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DateProps {
    pub date: DayDate,
    pub style: Style,
    pub min: Option<DayDate>,
    pub max: Option<DayDate>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DatePatch {
    SetDate(DayDate),
}

/// Full props (realize) for the time picker. `style`/`seconds` are set once at build; only the
/// value patches.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TimeProps {
    pub time: DayTime,
    pub style: Style,
    /// Show/edit seconds — honored on AppKit and Qt (the toolkits whose controls have a seconds
    /// field); a documented no-op elsewhere.
    pub seconds: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TimePatch {
    SetTime(DayTime),
}

/// How the pickers are realized on the compiled backend: `Native` (a real platform picker
/// control) or `Emulated` (GTK — GTK4/libadwaita have no stock date/time picker, so the renderer
/// composes native primitives: GtkCalendar in a popover, linked spin buttons; also mock).
pub fn support() -> day_spec::Support {
    if cfg!(any(
        all(feature = "appkit", target_os = "macos"),
        all(feature = "uikit", target_os = "ios"),
        all(feature = "mdc", target_os = "android"),
        feature = "qt",
        all(feature = "winui", windows),
        all(feature = "arkui", target_env = "ohos"),
    )) {
        day_spec::Support::Native
    } else {
        day_spec::Support::Emulated
    }
}

/// A native date picker bound two-way to a [`DayDate`] signal. Build with [`date_picker`].
pub struct DatePicker {
    date: Signal<DayDate>,
    style: Style,
    min: Option<DayDate>,
    max: Option<DayDate>,
}

/// `date_picker(date)` — a field summoning the platform's chooser; `.inline()` embeds it.
pub fn date_picker(date: Signal<DayDate>) -> DatePicker {
    DatePicker {
        date,
        style: Style::Automatic,
        min: None,
        max: None,
    }
}

impl DatePicker {
    pub fn compact(mut self) -> Self {
        self.style = Style::Compact;
        self
    }
    pub fn inline(mut self) -> Self {
        self.style = Style::Inline;
        self
    }
    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }
    /// Earliest selectable date (inclusive). Native where the control supports it; picks are
    /// clamped in the piece regardless.
    pub fn min(mut self, min: DayDate) -> Self {
        self.min = Some(min);
        self
    }
    /// Latest selectable date (inclusive).
    pub fn max(mut self, max: DayDate) -> Self {
        self.max = Some(max);
        self
    }
}

impl Piece for DatePicker {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let DatePicker {
            date,
            style,
            min,
            max,
        } = self;
        // Seed with the signal's value clamped into range; reflect a build-time clamp back.
        let initial = date.get_untracked().clamped(min, max);
        if initial != date.get_untracked() {
            date.set(initial);
        }
        let node = cx.leaf(
            DATE_KIND,
            &DateProps {
                date: initial,
                style,
                min,
                max,
            },
            Flex::default(),
        );
        // App writes → native (renderers no-op on an unchanged value, so a pick echoing through
        // the signal never loops).
        bind_seeded(
            initial,
            move || date.get(),
            move |d: &DayDate| {
                with_tree(|t| t.patch(node, Box::new(DatePatch::SetDate(*d)), false));
            },
        );
        // Native picks (`Custom`: ISO text in-process, epoch days across JNI/C-ABI) and
        // dayscript's `input:` step (`TextChanged` with ISO text) → signal.
        cx.on(node, move |ev| {
            let picked = match ev {
                Event::Custom { num, text, .. } => {
                    if text.is_empty() {
                        Some(DayDate::from_epoch_days(*num as i64))
                    } else {
                        DayDate::parse_iso(text)
                    }
                }
                Event::TextChanged(s) => DayDate::parse_iso(s),
                _ => None,
            };
            if let Some(d) = picked {
                date.set(d.clamped(min, max));
            }
        });
        node
    }
}

/// A native time picker bound two-way to a [`DayTime`] signal. Build with [`time_picker`].
pub struct TimePicker {
    time: Signal<DayTime>,
    style: Style,
    seconds: bool,
}

/// `time_picker(time)` — a field summoning the platform's chooser; `.inline()` embeds it.
pub fn time_picker(time: Signal<DayTime>) -> TimePicker {
    TimePicker {
        time,
        style: Style::Automatic,
        seconds: false,
    }
}

impl TimePicker {
    pub fn compact(mut self) -> Self {
        self.style = Style::Compact;
        self
    }
    pub fn inline(mut self) -> Self {
        self.style = Style::Inline;
        self
    }
    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }
    /// Show/edit seconds — honored on AppKit and Qt; a documented no-op elsewhere
    /// (docs/datepicker.md).
    pub fn seconds(mut self, seconds: bool) -> Self {
        self.seconds = seconds;
        self
    }
}

impl Piece for TimePicker {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let TimePicker {
            time,
            style,
            seconds,
        } = self;
        let initial = time.get_untracked();
        let node = cx.leaf(
            TIME_KIND,
            &TimeProps {
                time: initial,
                style,
                seconds,
            },
            Flex::default(),
        );
        bind_seeded(
            initial,
            move || time.get(),
            move |t: &DayTime| {
                with_tree(|tr| tr.patch(node, Box::new(TimePatch::SetTime(*t)), false));
            },
        );
        cx.on(node, move |ev| {
            let picked = match ev {
                Event::Custom { num, text, .. } => {
                    if text.is_empty() {
                        Some(DayTime::from_seconds_of_day(*num as i64))
                    } else {
                        DayTime::parse_iso(text)
                    }
                }
                Event::TextChanged(s) => DayTime::parse_iso(s),
                _ => None,
            };
            if let Some(t) = picked {
                time.set(t);
            }
        });
        node
    }
}

// ---------------------------------------------------------------------------
// Per-toolkit native renderers — one file per backend (the day-piece-picker convention). Every
// module registers `Renderer`s link-time into its backend's `RENDERERS` slice; the `#[cfg]` gates
// each to its feature + target.
// ---------------------------------------------------------------------------

day_pieces::glue_modules!(appkit, gtk, qt, uikit, mdc, winui, arkui);

// ---------------------------------------------------------------------------
// Unit tests: the calendar math every backend leans on.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_validation() {
        assert!(DayDate::new(2026, 7, 18).is_some());
        assert!(DayDate::new(2024, 2, 29).is_some(), "leap day");
        assert!(DayDate::new(2023, 2, 29).is_none(), "not a leap year");
        assert!(DayDate::new(2000, 2, 29).is_some(), "400-rule leap");
        assert!(DayDate::new(1900, 2, 29).is_none(), "100-rule non-leap");
        assert!(DayDate::new(2026, 4, 31).is_none());
        assert!(DayDate::new(2026, 0, 1).is_none());
        assert!(DayDate::new(2026, 13, 1).is_none());
        assert!(DayDate::new(2026, 1, 0).is_none());
    }

    #[test]
    fn epoch_days_anchors() {
        let d = |y, m, dd| DayDate::new(y, m, dd).unwrap();
        assert_eq!(d(1970, 1, 1).to_epoch_days(), 0);
        assert_eq!(d(1970, 1, 2).to_epoch_days(), 1);
        assert_eq!(d(1969, 12, 31).to_epoch_days(), -1);
        assert_eq!(d(2000, 1, 1).to_epoch_days(), 10957);
        assert_eq!(d(2026, 7, 18).to_epoch_days(), 20652);
    }

    #[test]
    fn epoch_days_round_trip() {
        // A wide swath incl. leap boundaries and negative epochs.
        for days in (-200_000..200_000).step_by(97) {
            let d = DayDate::from_epoch_days(days);
            assert_eq!(d.to_epoch_days(), days, "{d}");
            assert!(
                DayDate::new(d.year, d.month, d.day).is_some(),
                "{d} is valid"
            );
        }
    }

    #[test]
    fn date_iso_round_trip() {
        for s in ["1970-01-01", "2026-07-18", "2024-02-29", "0001-12-31"] {
            let d = DayDate::parse_iso(s).unwrap();
            assert_eq!(d.to_string(), s);
        }
        for bad in [
            "2026-7-18",
            "18-07-2026",
            "2026-02-30",
            "2026-07-18T00:00",
            "hello",
            "",
        ] {
            assert!(DayDate::parse_iso(bad).is_none(), "{bad:?} rejected");
        }
    }

    #[test]
    fn date_clamp() {
        let d = |y, m, dd| DayDate::new(y, m, dd).unwrap();
        let (min, max) = (Some(d(2026, 1, 1)), Some(d(2026, 12, 31)));
        assert_eq!(d(2025, 6, 1).clamped(min, max), d(2026, 1, 1));
        assert_eq!(d(2027, 6, 1).clamped(min, max), d(2026, 12, 31));
        assert_eq!(d(2026, 6, 1).clamped(min, max), d(2026, 6, 1));
        assert_eq!(d(2027, 6, 1).clamped(None, None), d(2027, 6, 1));
    }

    #[test]
    fn time_validation_and_seconds() {
        assert!(DayTime::new(23, 59, 59).is_some());
        assert!(DayTime::new(24, 0, 0).is_none());
        assert!(DayTime::new(0, 60, 0).is_none());
        assert!(DayTime::new(0, 0, 60).is_none());
        let t = DayTime::new(9, 30, 15).unwrap();
        assert_eq!(t.seconds_of_day(), 34_215);
        assert_eq!(DayTime::from_seconds_of_day(34_215), t);
        assert_eq!(
            DayTime::from_seconds_of_day(-1),
            DayTime::new(23, 59, 59).unwrap(),
            "wraps into the day"
        );
    }

    #[test]
    fn time_iso_round_trip() {
        assert_eq!(
            DayTime::parse_iso("09:30").unwrap(),
            DayTime::new(9, 30, 0).unwrap()
        );
        assert_eq!(
            DayTime::parse_iso("23:59:59").unwrap(),
            DayTime::new(23, 59, 59).unwrap()
        );
        // Display shows seconds exactly when nonzero.
        assert_eq!(DayTime::new(9, 30, 0).unwrap().to_string(), "09:30");
        assert_eq!(DayTime::new(9, 30, 7).unwrap().to_string(), "09:30:07");
        for bad in ["24:00", "9:30", "09:3", "09:30:5", "09-30", ""] {
            assert!(DayTime::parse_iso(bad).is_none(), "{bad:?} rejected");
        }
    }

    #[test]
    fn today_is_valid() {
        let t = DayDate::today();
        assert!(DayDate::new(t.year, t.month, t.day).is_some());
        assert!(t.year >= 2026);
    }
}
