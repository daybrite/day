# Date & time pickers (external piece)

> **Status: implemented** as `day-piece-datetime`, an external Day Piece registered link-time into
> each backend's renderer slice without touching day — the first external piece with renderers for
> **all seven** toolkits (including its own ArkUI NDK shim). TWO pieces —
> `date_picker(Signal<DayDate>)` and `time_picker(Signal<DayTime>)` — because a single combined
> date-time control exists on only 3 of the 7 toolkits (`NSDatePicker`,
> `UIDatePicker.dateAndTime`, `QDateTimeEdit`), while separate date and time controls realize
> natively on all of them. Combined selection is composition: `row((date_picker(d), time_picker(t)))`.

## Authoring

```rust
use day_piece_datetime::{DayDate, DayTime, date_picker, time_picker};

let date = Signal::new(DayDate::new(2026, 7, 18).unwrap());
let time = Signal::new(DayTime::new(9, 30, 0).unwrap());

column((
    date_picker(date),                       // Compact (default): field → platform chooser
    date_picker(date).inline(),              // Inline: embedded calendar / wheels
    date_picker(date)
        .min(DayDate::new(2026, 1, 1).unwrap())
        .max(DayDate::new(2026, 12, 31).unwrap()),
    time_picker(time).seconds(true),         // seconds honored on AppKit/Qt only
))
```

The bound signal is **two-way**: a user pick writes it; the app writing it updates the native
control. Values are **civil (zoneless)** — `DayDate { year, month, day }` (proleptic Gregorian,
validated constructors, `Display`/`parse_iso` speak ISO-8601) and `DayTime { hour, minute,
second }`. Day carries no time-zone database: an app that needs zoned instants attaches the zone
itself (`DayDate::today()` derives from the system clock in UTC). Interchange across native
boundaries is epoch days / seconds-of-day; every renderer pins its control's calendar to
proleptic-Gregorian UTC so the civil value never shifts by zone, while the **locale stays the
user's** — month and weekday names render localized by the platform.

`min`/`max` bound the date natively where the control supports it, and the piece clamps every
pick regardless — an out-of-range synthetic set lands on the nearest bound.

## Styles: the honest intersection

Two intents (+ `Automatic`, which is `Compact` everywhere today):

- **`Compact`** — a field/button showing the value that summons a *transient chooser*. The chooser's
  chrome is the platform's own: a popover on iOS, a **modal Material dialog** on Android, a
  calendar-popup on Qt, a flyout on Windows. Same gesture contract, different chrome — by design.
- **`Inline`** — an embedded calendar / clock / wheels.

Anything finer (wheels-vs-calendar, dialog-vs-popover) is platform identity Day does not paper
over (DESIGN.md §2: native-at-home beats identical-everywhere).

## Per-toolkit realization

| Target | Tier | Date: Compact | Date: Inline | Time |
|---|---|---|---|---|
| macos-appkit | **Native** | `NSDatePicker` textFieldAndStepper | `NSDatePicker` clockAndCalendar | same control, hour/minute[/second] elements |
| ios-uikit | **Native** | `UIDatePicker` .compact (popover) | `.inline` calendar | `.compact` keypad; Inline = `.wheels` (iOS has no inline clock) |
| android-mdc | **Native** | value button → `MaterialDatePicker` dialog (via `DayActivity`'s FragmentManager) | framework `DatePicker` calendar | button → `MaterialTimePicker` clock dialog; Inline = framework `TimePicker` |
| qt | **Native** | `QDateEdit` + calendar popup | `QCalendarWidget` | `QTimeEdit` (both styles; seconds via display format) |
| winui | **Native** | `CalendarDatePicker` flyout | `CalendarView` | `TimePicker` flyout (both styles — WinUI has no inline clock) |
| ohos-arkui | **Native** | `ARKUI_NODE_CALENDAR_PICKER` (entry → calendar popup) | `ARKUI_NODE_DATE_PICKER` wheels (native START/END) | `ARKUI_NODE_TIME_PICKER` wheels |
| gtk | Emulated | `GtkMenuButton` (locale-formatted label) → `GtkCalendar` in a `GtkPopover` | `GtkCalendar` | linked `GtkSpinButton`s h/m[/s] — GTK4/libadwaita have **no** stock date/time picker; this composes native primitives |
| mock | Emulated | generic widget; tests drive via events | " | " |

`day_piece_datetime::support()` reports the compiled backend's tier.

## What the unified pieces do NOT promise

- **Identical chrome** — Android's chooser is a modal dialog, iOS's a popover, Qt's a popup.
- **Seconds everywhere** — `.seconds(true)` is honored on AppKit and Qt (the two toolkits whose
  controls have a seconds field) and is a no-op elsewhere.
- **A combined date+time single control** — compose the two pieces; a `date_time_picker`
  convenience that upgrades to the native combined control on AppKit/UIKit/Qt is a possible v2.
- **Range / multi-date selection** — v2 territory (`UICalendarView`, `MaterialDatePicker` range,
  `CalendarView` multi-select).
- **Native min/max on every surface** — GTK's `GtkCalendar` and ArkUI's calendar picker have no
  bounds API; there the piece's own clamp bounds the *value* (the visual grid stays free), and
  a clamped pick snaps the control back on the echo patch.

## Scripting

Every picker accepts `Event::TextChanged` carrying an ISO value as a synthetic set, so dayscript's
existing `input:` step drives any picker on any backend, and ISO readouts bound to the signals
assert locale-independently:

```yaml
- input: { id: date-compact, text: "2026-11-05" }
- assert_text: { id: date-value, text: "2026-11-05" }
- input: { id: time-compact, text: "14:45" }
```

## Limits

- Android's Material dialogs live on the activity's FragmentManager; a dialog left open survives
  the piece (it dismisses with the activity, and re-opening is guarded by tag).
- ArkUI hour-cycle follows the node default (`USE_MILITARY_TIME` unset); wiring it to the system
  hour-cycle preference is a follow-up.
- `DayTime`'s `Display` shows seconds exactly when nonzero (`"09:30"` / `"09:30:07"`) — assert
  accordingly in scripts.
