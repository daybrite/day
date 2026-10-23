// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day_pieces::datetime: core date and time pickers with native toolkit renderers and a composed DOM fallback
//! (docs/datepicker.md; DESIGN.md §15).
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
//! Two pieces, [`date_picker`] and [`time_picker`], rather than one combined date-time piece: a
//! single combined control exists on only 3 of the 7 toolkits (`NSDatePicker`,
//! `UIDatePicker.dateAndTime`, `QDateTimeEdit`), while separate date and time controls realize
//! natively on all of them. Each piece maps a small style intent ([`Style::Compact`] /
//! [`Style::Inline`]) to the platform's closest idiomatic control, never promising identical
//! chrome (Android's Material chooser is a modal dialog, iOS's a popover, Qt's a calendar popup;
//! that difference is the platform, docs/datepicker.md has the full table).
//!
//! The bound signal is two-way: a user pick sets it; the app setting it updates the native
//! control. Values are civil (zoneless) [`DayDate`]/[`DayTime`]; time zones are the app's
//! business. The piece's node also accepts `Event::TextChanged` carrying an ISO value
//! (`"2026-07-18"` / `"09:30"`) as a synthetic set, so dayscript's existing `input:` step drives
//! any picker on any backend.

use crate::Decorated;
use day_core::{BuildCx, Flex, Piece, RNode, with_tree};
use day_reactive::{Binding, bind_seeded};
use day_spec::Event;

pub use day_spec::controls::datetime::*;

/// How the pickers are realized on the compiled backend: `Native` (a platform picker
/// control) or `Emulated` (GTK, where GTK4/libadwaita have no stock date/time picker, so the
/// renderer composes native primitives: GtkCalendar in a popover, linked spin buttons; also mock).
pub fn support() -> day_spec::Support {
    let support = day_core::capability(day_spec::Cap::DateTimePicker);
    if support == day_spec::Support::Unsupported {
        day_spec::Support::Emulated
    } else {
        support
    }
}

/// A native date picker bound two-way to a [`DayDate`] signal. Build with [`date_picker`].
pub struct DatePicker<S: Binding<DayDate>> {
    date: S,
    style: Style,
    min: Option<DayDate>,
    max: Option<DayDate>,
}

/// `date_picker(date)`: a field summoning the platform's chooser; `.inline()` embeds it.
/// `date` is a `Signal<DayDate>` or any other two-way binding (a day-model `Field`, mapped).
pub fn date_picker<S: Binding<DayDate>>(date: S) -> DatePicker<S> {
    // web-dom's registry is populated at runtime (no `linkme` on wasm), and a constructor always
    // runs before the node it returns is realized, so this is where the arm registers itself.
    DatePicker {
        date,
        style: Style::Automatic,
        min: None,
        max: None,
    }
}

#[crate::builder(DatePickerBuilder)]
#[crate::builder(TimePickerBuilder)]
impl<S: Binding<DayDate>> DatePicker<S> {
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

impl<S: Binding<DayDate>> Piece for DatePicker<S> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let DatePicker {
            date,
            style,
            min,
            max,
        } = self;
        // Seed with the signal's value clamped into range; reflect a build-time clamp back.
        let initial = date.peek().clamped(min, max);
        if initial != date.peek() {
            date.write(initial);
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
        let dt = date.clone();
        bind_seeded(
            initial,
            move || dt.read(),
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
                date.write(d.clamped(min, max));
            }
        });
        node
    }
}

/// A native time picker bound two-way to a [`DayTime`] signal. Build with [`time_picker`].
pub struct TimePicker<S: Binding<DayTime>> {
    time: S,
    style: Style,
    seconds: bool,
}

/// `time_picker(time)`: a field summoning the platform's chooser; `.inline()` embeds it.
pub fn time_picker<S: Binding<DayTime>>(time: S) -> TimePicker<S> {
    TimePicker {
        time,
        style: Style::Automatic,
        seconds: false,
    }
}

impl<S: Binding<DayTime>> TimePicker<S> {
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
    /// Show/edit seconds; honored on AppKit and Qt, a documented no-op elsewhere
    /// (docs/datepicker.md).
    pub fn seconds(mut self) -> Self {
        self.seconds = true;
        self
    }
}

impl<S: Binding<DayTime>> Piece for TimePicker<S> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let TimePicker {
            time,
            style,
            seconds,
        } = self;
        let initial = time.peek();
        let node = cx.leaf(
            TIME_KIND,
            &TimeProps {
                time: initial,
                style,
                seconds,
            },
            Flex::default(),
        );
        let tm = time.clone();
        bind_seeded(
            initial,
            move || tm.read(),
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
                time.write(t);
            }
        });
        node
    }
}

// ---------------------------------------------------------------------------
// Per-toolkit native renderers, one file per backend (the day-piece-picker convention). Every
// module registers `Renderer`s link-time into its backend's `RENDERERS` slice; the `#[cfg]` gates
// each to its feature + target.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Unit tests: the calendar math every backend leans on.
// ---------------------------------------------------------------------------

// --- Typed builders, forwarded through `Decorated` (docs/api-style.md) ---

/// The date and time pickers' conformance cases (docs/testing.md). Every picker takes a
/// synthetic ISO `input`, on every backend.
#[cfg(feature = "conformance")]
pub mod conformance {
    use crate::*;
    use day_core::conformance::{Case, Drive};
    use day_reactive::Signal;

    use super::{DATE_KIND, DayDate, DayTime, TIME_KIND, date_picker, time_picker};

    /// A picked date reaches the bound date.
    #[day_macros::test(day_core)]
    fn date_picker_sets_date() -> Case {
        Case::new()
            .proves(DATE_KIND)
            .page(|| {
                let date = Signal::new(DayDate::new(2026, 1, 15).unwrap_or(DayDate::today()));
                column((
                    label(move || format!("date {}", date.get()))
                        .id(day_core::id!(datetime::value)),
                    date_picker(date)
                        .compact()
                        .id(day_core::id!(datetime::date)),
                ))
            })
            .drive(|d: Drive| async move {
                d.input("datetime::date", "2026-11-05").await?;
                d.assert_text("datetime::value", "date 2026-11-05").await
            })
    }

    /// A date past the picker's bounds lands on the bound.
    #[day_macros::test(day_core)]
    fn date_picker_clamps() -> Case {
        Case::new()
            .proves(DATE_KIND)
            .page(|| {
                let date = Signal::new(DayDate::new(2026, 6, 1).unwrap_or(DayDate::today()));
                let (lo, hi) = (DayDate::new(2026, 1, 1), DayDate::new(2026, 12, 31));
                let mut picker = date_picker(date).compact();
                if let (Some(lo), Some(hi)) = (lo, hi) {
                    picker = picker.min(lo).max(hi);
                }
                column((
                    label(move || format!("date {}", date.get()))
                        .id(day_core::id!(datetime::value)),
                    picker.id(day_core::id!(datetime::date)),
                ))
            })
            .drive(|d: Drive| async move {
                d.input("datetime::date", "2027-06-15").await?;
                d.assert_text("datetime::value", "date 2026-12-31").await
            })
    }

    /// A picked time reaches the bound time.
    #[day_macros::test(day_core)]
    fn time_picker_sets_time() -> Case {
        Case::new()
            .proves(TIME_KIND)
            .page(|| {
                let time = Signal::new(DayTime::new(9, 0, 0).unwrap_or(DayTime::now()));
                column((
                    label(move || format!("time {}", time.get()))
                        .id(day_core::id!(datetime::value)),
                    time_picker(time)
                        .compact()
                        .id(day_core::id!(datetime::time)),
                ))
            })
            .drive(|d: Drive| async move {
                d.input("datetime::time", "14:45").await?;
                d.assert_text("datetime::value", "time 14:45").await
            })
    }
}

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
