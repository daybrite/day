// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

pub const DATE_KIND: &str = "day.piece.datepicker";
pub const TIME_KIND: &str = "day.piece.timepicker";

// ---------------------------------------------------------------------------
// Values: proleptic-Gregorian civil date + wall-clock time. No chrono dependency: ISO-8601
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

/// Seconds since the Unix epoch, from whatever clock this target has.
///
/// `SystemTime::now()` traps on `wasm32-unknown-unknown`: the target has no clock, and the trap
/// is not a panic that can be caught, it aborts the module. So a `DayDate::today()` anywhere on
/// the startup path took the whole web app down with `RuntimeError: Unreachable code`. The web
/// build reads the host's clock through the same JS shim day-toolkit-dom imports everything else from.
fn now_epoch_secs() -> i64 {
    #[cfg(target_arch = "wasm32")]
    {
        // SAFETY: the shim exports this for every web-dom app; a build that links this piece and
        // an older shim fails at instantiation with a LinkError naming the missing import, which
        // is the loud failure rather than the silent one.
        unsafe { day_datetime_now_secs() as i64 }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }
}

// The attribute makes this a wasm import from the instantiation's `env` object (shim.js supplies
// it) rather than a link-time-resolved symbol; without it rust-lld reports it undefined.
#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "env")]
unsafe extern "C" {
    /// Seconds since the Unix epoch (fractional), from the browser's clock.
    fn day_datetime_now_secs() -> f64;
}

impl DayDate {
    /// A validated date (`month` 1–12, `day` within the month, leap-aware); `None` otherwise.
    pub fn new(year: i32, month: u8, day: u8) -> Option<Self> {
        if (1..=12).contains(&month) && day >= 1 && day <= days_in_month(year, month) {
            Some(DayDate { year, month, day })
        } else {
            None
        }
    }

    /// Today's date derived from the system clock in UTC (Day carries no time-zone database; an
    /// app that must roll the date at local midnight should compute it with its own zone source).
    pub fn today() -> Self {
        Self::from_epoch_days(now_epoch_secs().div_euclid(86_400))
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

pub fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

pub fn days_in_month(year: i32, month: u8) -> u8 {
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
    /// A validated time (`hour` 0–23, `minute`/`second` 0–59); `None` otherwise.
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

    /// The current time of day from the system clock in UTC, the twin of [`DayDate::today`],
    /// and carrying the same caveat: Day ships no time-zone database, so an app that needs local
    /// wall-clock time should apply its own offset. Pair the two for a timestamp
    /// (`Day-Showcase-2026-08-12-14-23-05.png`).
    pub fn now() -> Self {
        Self::from_seconds_of_day(now_epoch_secs().rem_euclid(86_400))
    }

    /// Seconds since midnight, the numeric interchange form across native boundaries.
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
/// each intent maps to the platform's closest idiomatic control, so chrome differs per platform.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Style {
    /// The platform default: [`Style::Compact`] everywhere today.
    #[default]
    Automatic,
    /// A field/button showing the value that summons a transient chooser (NSDatePicker
    /// textFieldAndStepper / UIDatePicker .compact / a Material dialog launcher / QDateEdit with
    /// calendar popup / CalendarDatePicker flyout / GtkMenuButton+calendar popover /
    /// ArkUI CalendarPicker).
    Compact,
    /// An embedded calendar / clock / wheels (NSDatePicker graphical / UIDatePicker .inline /
    /// framework DatePicker+TimePicker widgets / QCalendarWidget / CalendarView / GtkCalendar /
    /// ArkUI DatePicker wheels).
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
    /// Show/edit seconds; honored on AppKit and Qt (the toolkits whose controls have a seconds
    /// field), a documented no-op elsewhere.
    pub seconds: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TimePatch {
    SetTime(DayTime),
}
