# day-piece-datetime

Native date and time pickers: `date_picker(date)` and `time_picker(time)`, each bound
two-way to a civil `DayDate` / `DayTime` signal.

Two style intents map to each platform's closest idiomatic control. `Compact` is a field
or button that summons the platform's transient chooser — `NSDatePicker` on macOS, a
`UIDatePicker` popover on iOS, a Material dialog on Android, a `QDateEdit` calendar popup
on Qt, a `CalendarDatePicker` flyout on Windows, a `CalendarPicker` entry on HarmonyOS.
`Inline` embeds the calendar, clock, or wheels. On GTK — which has no stock picker at
all — the piece composes native primitives (`GtkCalendar` in a popover, linked spin
buttons). Combined date+time selection is composition: put both pieces in a `row`.

Values are zoneless civil dates and times (ISO-8601 in, ISO-8601 out); every control is
pinned to a Gregorian-UTC calendar with the user's locale, so month and weekday names
localize while the value never shifts by time zone. See `docs/datepicker.md` in the Day
repository for the full per-platform table and the honest non-promises.

Pieces are Day's reusable UI components, shipped as ordinary crates: one Rust API in
front, a real native control per platform behind it. Enable the backends you build for
with cargo features, and `day build` wires up the native side automatically.

## Part of Day

This crate is one piece of [Day](https://daybrite.dev), a Rust framework for building apps
out of each platform's real native widgets — AppKit, UIKit, Android's Material widgets,
GTK 4, Qt 6, WinUI, and ArkUI — from one codebase. There is no web view and no bundled
rendering engine: when you write `button("Save")`, macOS shows an `NSButton` and Android
shows a Material button.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
