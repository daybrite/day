---
title: "Date & time pickers"
description: "Date and time controls, shared civil values, and platform presentation options."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Date & time pickers

Let people choose dates and times using familiar platform controls. `day-pieces`
provides separate date and time pickers, each bound to app state. Place them together when a
form needs both.

## Add to your app

This control is included in [`day-pieces`](https://github.com/daybrite/day/tree/main/crates/day-pieces)
and re-exported through `day::prelude::*`. No additional crate or control feature is required.
The selected toolkit supplies its native implementation; [`day::datetime::support()`](https://docs.rs/day-pieces/latest/day_pieces/datetime/fn.support.html) reports
its availability after the Day runtime starts.

| Platforms / toolkits | Availability |
| --- | --- |
| AppKit, UIKit, Qt, Android, WinUI, ArkUI, Web | Platform date/time inputs; available styles differ. |
| GTK | Composes native calendar and spin controls. |

## Authoring

```rust
use day::datetime::{DayDate, DayTime, date_picker, time_picker};

let date = Signal::new(DayDate::new(2026, 7, 18).unwrap());
let time = Signal::new(DayTime::new(9, 30, 0).unwrap());

column((
    date_picker(date),                       // Compact (default): field → platform chooser
    date_picker(date).inline(),              // Inline: embedded calendar / wheels
    date_picker(date)
        .min(DayDate::new(2026, 1, 1).unwrap())
        .max(DayDate::new(2026, 12, 31).unwrap()),
    time_picker(time).seconds(),             // seconds honored on AppKit/Qt only
))
```

The bound signal is **two-way**: a user pick writes it; the app writing it updates the native
control. Values are **civil (zoneless)**: `DayDate { year, month, day }` (proleptic Gregorian,
validated constructors, `Display`/`parse_iso` speak ISO-8601) and `DayTime { hour, minute,
second }`. Day carries no time-zone database: an app that needs zoned instants attaches the zone
itself ([`DayDate::today()`](https://docs.rs/day-pieces/latest/day_pieces/datetime/struct.DayDate.html#method.today) derives from the system clock in UTC). Interchange across native
boundaries is epoch days / seconds-of-day; every renderer pins its control's calendar to
proleptic-Gregorian UTC so the civil value never shifts by zone, while the **locale stays the
user's**: month and weekday names render localized by the platform.

[`min`](https://docs.rs/day-pieces/latest/day_pieces/datetime/trait.DatePickerBuilder.html#tymethod.min)/[`max`](https://docs.rs/day-pieces/latest/day_pieces/datetime/trait.DatePickerBuilder.html#tymethod.max) bound the date natively where the control supports it, and the piece clamps every
pick regardless; an out-of-range synthetic set lands on the nearest bound.

## Styles: the shared subset

Two intents (+ `Automatic`, which is `Compact` everywhere today):

- **`Compact`** — a field/button showing the value that summons a *transient chooser*. The chooser's
  chrome is the platform's own: a popover on iOS, a **modal Material dialog** on Android, a
  calendar-popup on Qt, a flyout on Windows. The gesture contract is the same; the chrome is each
  platform's own.
- **`Inline`** — an embedded calendar / clock / wheels.

The platform chooses the finer presentation details, such as wheels versus a calendar or
a dialog versus a popover.

## Toolkit implementations

Platform guidance: [Apple: pickers](https://developer.apple.com/design/human-interface-guidelines/pickers); [Android: date and time pickers](https://developer.android.com/develop/ui/views/components/pickers); [Windows: date and time controls](https://learn.microsoft.com/en-us/windows/apps/design/controls/date-and-time).

| Target | Tier | Date: Compact | Date: Inline | Time |
|---|---|---|---|---|
| macos-appkit | **Native** | [`NSDatePicker`](https://developer.apple.com/documentation/appkit/nsdatepicker) textFieldAndStepper | [`NSDatePicker`](https://developer.apple.com/documentation/appkit/nsdatepicker) clockAndCalendar | same control, hour/minute[/second] elements |
| ios-uikit | **Native** | [`UIDatePicker`](https://developer.apple.com/documentation/uikit/uidatepicker) .compact (popover) | [`.inline`](https://docs.rs/day-pieces/latest/day_pieces/datetime/trait.DatePickerBuilder.html#tymethod.inline) calendar | [`.compact`](https://docs.rs/day-pieces/latest/day_pieces/datetime/trait.DatePickerBuilder.html#tymethod.compact) keypad; Inline = `.wheels` (iOS has no inline clock) |
| android-material | **Native** | value button → [`MaterialDatePicker`](https://developer.android.com/reference/com/google/android/material/datepicker/MaterialDatePicker) dialog (via `DayActivity`'s FragmentManager) | framework [`DatePicker`](https://developer.android.com/reference/android/widget/DatePicker) calendar | button → [`MaterialTimePicker`](https://developer.android.com/reference/com/google/android/material/timepicker/MaterialTimePicker) clock dialog; Inline = framework [`TimePicker`](https://developer.android.com/reference/android/widget/TimePicker) |
| qt | **Native** | [`QDateEdit`](https://doc.qt.io/qt-6/qdateedit.html) + calendar popup | [`QCalendarWidget`](https://doc.qt.io/qt-6/qcalendarwidget.html) | [`QTimeEdit`](https://doc.qt.io/qt-6/qtimeedit.html) (both styles; seconds via display format) |
| windows-winui | **Native** | [`CalendarDatePicker`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.calendardatepicker) flyout | [`CalendarView`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.calendarview) | [`TimePicker`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.timepicker) flyout (both styles; WinUI has no inline clock) |
| harmony-arkui | **Native** | [`ARKUI_NODE_CALENDAR_PICKER`](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-arkui/capi-native-node-h.md) (entry → calendar popup) | [`ARKUI_NODE_DATE_PICKER`](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-arkui/capi-native-node-h.md) wheels (native START/END) | [`ARKUI_NODE_TIME_PICKER`](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-arkui/capi-native-node-h.md) wheels |
| gtk | Emulated | [`GtkMenuButton`](https://docs.gtk.org/gtk4/class.MenuButton.html) (locale-formatted label) → [`GtkCalendar`](https://docs.gtk.org/gtk4/class.Calendar.html) in a [`GtkPopover`](https://docs.gtk.org/gtk4/class.Popover.html) | [`GtkCalendar`](https://docs.gtk.org/gtk4/class.Calendar.html) | linked [`GtkSpinButton`](https://docs.gtk.org/gtk4/class.SpinButton.html)s h/m[/s]; GTK4/libadwaita have no stock date/time picker, so this composes native primitives |
| mock | Emulated | generic widget; tests drive via events | " | " |

[`day_pieces::datetime::support()`](https://docs.rs/day-pieces/latest/day_pieces/datetime/fn.support.html) reports the active toolkit's tier.

## Style limits

- **Identical chrome** — Android's chooser is a modal dialog, iOS's a popover, Qt's a popup.
- **Seconds everywhere** — [`.seconds()`](https://docs.rs/day-pieces/latest/day_pieces/datetime/struct.TimePicker.html#method.seconds) is honored on AppKit and Qt (whose
  controls have a seconds field) and is a no-op elsewhere.
- **A combined date+time single control** — compose the two pieces; a `date_time_picker`
  convenience that upgrades to the native combined control on AppKit/UIKit/Qt is a possible v2.
- **Range / multi-date selection** — v2 territory ([`UICalendarView`](https://developer.apple.com/documentation/uikit/uicalendarview), [`MaterialDatePicker`](https://developer.android.com/reference/com/google/android/material/datepicker/MaterialDatePicker) range,
  [`CalendarView`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.calendarview) multi-select).
- **Native min/max on every surface** — GTK's [`GtkCalendar`](https://docs.gtk.org/gtk4/class.Calendar.html) and ArkUI's calendar picker have no
  bounds API; there the piece's own clamp bounds the *value* (the visual grid stays free), and
  a clamped pick snaps the control back on the echo patch.

## Limits

- Android's Material dialogs live on the activity's FragmentManager; a dialog left open survives
  the piece (it dismisses with the activity, and re-opening is guarded by tag).
- ArkUI hour-cycle follows the node default (`USE_MILITARY_TIME` unset); wiring it to the system
  hour-cycle preference is a follow-up.
- `DayTime`'s `Display` shows seconds exactly when nonzero (`"09:30"` / `"09:30:07"`); assert
  accordingly in scripts.

The `DayDate` and `DayTime` value types are defined in `day-spec` and re-exported here.
Headless persistence code can use them without depending on `day-pieces` or a toolkit.
