---
navTitle: "Gestures"
title: "Gestures and pointer input"
description: "Tap, double tap, long press, press, drag, pan, pinch, rotate, raw touches, velocity, hit slop, the input kind, interaction state, and how gestures share the pointer with scroll views."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Gestures and pointer input

A gesture turns pointer movement into something an app can act on: a tap, a drag with a
velocity, a pinch. Day attaches the platform's own recognizer where there is one and reports
every gesture in the piece's coordinates, in points.

Any piece takes a gesture through a modifier. The handler runs on the UI thread, inside the
scope of the piece that declared it.

| Modifier | Handler receives | Fires |
|---|---|---|
| [`.on_tap(f)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_tap), [`.on_tap_at(f)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_tap_at) | nothing, or the `Point` | once per tap |
| [`.on_double_tap(f)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_double_tap), [`.on_double_tap_at(f)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_double_tap_at) | nothing, or the `Point` | once per double tap |
| [`.on_long_press(f)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_long_press) | the press `Point` | once, while the pointer is still down |
| [`.on_pressed(f)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_pressed) | `bool` | `true` at the pointer-down, `false` when the press ends |
| [`.on_drag(f)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_drag), [`.on_drag_along(axis, f)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_drag_along) | [`Drag`](https://docs.rs/day-pieces/latest/day_pieces/struct.Drag.html) | `Began`, `Changed`…, `Ended` |
| [`.on_pan(f)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_pan) | [`Pan`](https://docs.rs/day-pieces/latest/day_pieces/struct.Pan.html) | two-finger scroll, wheel ticks |
| [`.on_pinch(f)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_pinch), [`.on_rotate(f)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_rotate) | `Pinch`, `Rotate` | see [canvas](canvas.md) "Zoom and pan" |
| [`.on_hover(f)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_hover) | `Option<Point>` | where a pointer exists |
| [`.on_touches(f)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_touches) | `&[Touch]` | every change to the contacts |

The event structs are `#[non_exhaustive]`: read their fields freely, and build one in a test
with its constructor
([`Drag::new(phase, location, translation).with_velocity(v)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Drag.html#method.new)).

## Velocity

`Drag` and `Pan` carry `velocity`, in points per second along the piece's axes. At `Ended` it
is the release velocity, which is what a fling or a spring starts from:

```rust
ball.on_drag(move |d: Drag| match d.phase {
    DragPhase::Ended => spring.release(d.velocity),   // points per second
    _ => position.set(d.location),
})
```

Where the platform's recognizer measures velocity, Day reports that measurement at the
release. Everywhere else, and during the gesture, Day estimates it from the last tenth of a
second of movement, so every toolkit has a value. A pointer that rested for a tenth of a
second before lifting has zero velocity. [`Drag::speed()`](https://docs.rs/day-pieces/latest/day_pieces/struct.Drag.html#method.speed) is its length, and
[`Drag::start()`](https://docs.rs/day-pieces/latest/day_pieces/struct.Drag.html#method.start) is the press point (`location - translation`), which is correct on every toolkit: some recognizers
report `Began` only after the pointer has moved.

A piece that tracks its own pointer (a recognizer written over `on_touches`) can measure the
same way with [`day::VelocityTracker`](https://docs.rs/day-core/latest/day_core/gesture/struct.VelocityTracker.html).

## Press

[`.on_pressed(f)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_pressed) follows the press: `true` at the pointer-down itself, before any tap or drag
is recognized, and `false` when the press ends. A press ends at the release, when the pointer
leaves the piece's hit area, or when the platform takes the gesture away (a scroll view starts
scrolling, a system gesture begins). Every `true` is followed by exactly one `false`. Press
reporting never delays or cancels a tap, a drag or a long press on the same piece or on a piece
around it. Where a piece only follows the press, its taps and drags go on to the nearest piece
around it that handles them, exactly as if they had landed there: the point is in that piece's
coordinates, its own drag axis and double tap apply, and a drag keeps its release velocity.

For styling, use [interaction state](#state-styling), which tracks the press for you.

## Long press

[`.on_long_press(f)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_long_press) fires when a press is held still, with the press point. It fires while the
pointer is still down, and the release that follows is not a tap, so a piece with both
[`.on_tap`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_tap) and [`.on_long_press`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_long_press) runs one or the other. The delay is the platform's own where its
recognizer is used and 500 ms where Day derives the gesture; movement of more than 10 points
cancels it. A long press is not discoverable: offer the same command in a menu or a button.

## Double tap

[`.on_double_tap(f)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_double_tap) fires on two taps in quick succession. On a piece that also has
[`.on_tap`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_tap),
the single tap waits out the double-tap interval, and the two never both fire. A piece with
[`.on_double_tap`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_double_tap) alone delays nothing. The interval is the platform's where its recognizer is
used, and 300 ms within 40 points where Day derives the gesture. [`.on_double_tap_at(f)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_double_tap_at) takes
the same gesture with the point of the second tap, which a map or an image viewer zooms
around.

## Touches

[`.on_touches(f)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_touches) delivers the raw contacts on the piece for a drawing surface, a multi-finger
instrument or a recognizer of your own. Each event lists every contact that is down, plus the
ones that lifted or were cancelled in that event:

```rust
pad.on_touches(move |touches: &[Touch]| {
    for t in touches {
        match t.phase {
            TouchPhase::Began | TouchPhase::Moved => strokes.extend(t.id, t.location, t.pressure),
            TouchPhase::Ended | TouchPhase::Cancelled => strokes.finish(t.id),
            TouchPhase::Stationary => {}
        }
    }
})
```

A [`Touch`](https://docs.rs/day-spec/latest/day_spec/struct.Touch.html) has an `id` that is
stable from `Began` to `Ended`, a `location`, a `pressure` from 0 to 1 (1 where the device
reports none), and the `kind` of device. Where the platform reports a single pointer, the list
holds that one contact with id 0; ask
[`day::gesture_support(GestureKind::Touches)`](https://docs.rs/day-core/latest/day_core/gesture/fn.gesture_support.html) and offer
single-finger behavior when it answers `Emulated`.

## Hit slop

[`.hit_slop(insets)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.hit_slop) grows the area in which a piece takes pointer input, without moving or
resizing it:

```rust
circle().fill(accent).frame(12.0, 12.0).hit_slop(16.0).on_tap(select)   // a 44-point target
```

Layout, drawing and the accessibility frame keep the piece's own bounds. Every gesture on the
piece answers inside the grown area. Where two slop areas overlap, the piece in front takes the
point. Whether the slop reaches past the parent's bounds depends on the toolkit; see the
table below. [`capability(Cap::HitSlop)`](https://docs.rs/day-core/latest/day_core/fn.capability.html) reports whether the toolkit applies it; where it does
not, the piece is hit inside its own bounds.

## Gestures inside a scroll view

A drag, pinch or rotate that has begun belongs to its piece until it ends. An enclosing scroll
view does not take it over, on any toolkit.

A piece inside a scroll view often wants one direction only: a row that swipes sideways in a
list that scrolls vertically. Declare the axis and the scroll view keeps the other one:

```rust
row(content).on_drag_along(DragAxis::Horizontal, move |d: Drag| offset.set(d.translation.x))
```

A pointer that sets off along the axis is the row's drag, from `Began` to `Ended`, and the list
stays still. A pointer that sets off across it scrolls the list, and the handler hears nothing
of that gesture. Day applies the handler half of this rule on every toolkit; whether the list
scrolls during the cross-axis movement is the toolkit's half (see the table below).

## Pass-through

A piece with no handler takes no pointer input. A press on it reaches whatever is behind it:
a shape drawn over a row of buttons with [`.overlay(..)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.overlay) leaves the buttons working, and a
canvas used as decoration does not block the list under it. A piece takes pointer input when
it has a gesture modifier, a context menu, a drag source or drop target, or [`.focusable()`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.focusable),
or when it is a native control. A [`.cursor(..)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.cursor) shapes the pointer over a piece and still lets
presses through.

## Input kind

[`day::primary_pointer()`](https://docs.rs/day-core/latest/day_core/gesture/fn.primary_pointer.html)
answers which kind of pointer the person is using: `PointerKind::Touch`, `Mouse` or `Stylus`.
It follows the last input, so an iPad switches to `Mouse` when its trackpad moves and back to
`Touch` at the next finger, and it is tracked, so wording and affordances update:

```rust
label(move || if day::primary_pointer().hovers() {
    res::str::hint_click()
} else {
    res::str::hint_tap()
})
```

Before any input it is the platform's usual kind: touch on iOS, Android and HarmonyOS, a mouse
elsewhere.

## State styling

An [`InteractionState`](https://docs.rs/day-pieces/latest/day_pieces/struct.InteractionState.html)
tracks whether a piece is hovered, pressed or focused. Create one, attach it with
[`.interaction(state)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.interaction),
and read it from any reactive modifier:

```rust
let state = InteractionState::new();
row((icon, title))
    .interaction(state)
    .background(move || if state.pressed() { PRESSED } else if state.hovered() { HOVER } else { REST })
    .opacity(move || if state.pressed() { 0.7 } else { 1.0 })
    .on_tap(open)
```

Each read is tracked, so the modifiers follow the state with no handler in between. `hovered`
stays `false` on a touch-only device. `pressed` follows the [press](#press). `focused` follows
keyboard focus on a piece that can take it ([focus](focus.md)). All three read `false` while
the piece is disabled. Native controls already draw their own states; use this for composed
rows, cards and custom controls.

To disable a group of pieces, see [forms](forms.md) "Disabling a group".

## Recognizers

Each toolkit attaches its own recognizer for a gesture when a piece asks for it. Where a
toolkit has none for press, long press, double tap or touches, Day derives the gesture from
the tap and drag events, with the timings given above.
[`day::gesture_support(kind)`](https://docs.rs/day-core/latest/day_core/gesture/fn.gesture_support.html) answers `Native`, `Emulated` (derived) or `Unsupported`.

Synthetic input from [DayScript](/docs/how-to/dayscript) and [tests](testing.md) injects the
gesture event itself (`press`, `long_press`, `double_tap`, `touches`, `drag` with a `velocity`,
`pointer`). That checks the app's handling; it does not exercise the platform's recognizer, its
hit testing or its arbitration with scroll views. Check those with real input.

## Toolkit support

| Toolkit | Limitation |
|---|---|
| [AppKit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") | A Mac has no touch screen: [`.on_touches`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_touches) reports the one contact Day derives from the mouse or trackpad drag. |
| [AppKit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") | A scroll view scrolls by the wheel and the trackpad, never by a mouse drag, so a drag across a row's `on_drag_along` axis scrolls nothing. |
| [UIKit](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") | A view with a context menu keeps the long press for the menu, and [`.on_long_press`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_long_press) does not fire there. |
| [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") | Hit slop applies to containers, canvases and shapes; a label, an image or a native control keeps its own bounds. |
| [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") | Hit slop reaches no further than the parent's bounds, since every Day container clips hit testing to its box. |
| [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") | On macOS [`.on_touches`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_touches) reports one derived contact (there is no touch screen); on Linux it reports every contact. |
| [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") | A touch drag reports Day's estimated release velocity, because claiming the touch sequence ends GTK's swipe recognizer; a mouse drag reports GTK's measured velocity. |
| [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") | A mouse drag across a row's `on_drag_along` axis does not scroll the list, since GTK scroll views scroll by the wheel and by touch; a touch drag does. |
| [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") | A piece with [`.cursor(..)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.cursor) takes the pointer (GTK shows the cursor of the widget its hit test finds), so presses on it do not reach what is behind it. |
| [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") | A widget with a context menu keeps the long press for the menu, and [`.on_long_press`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_long_press) does not fire there. |
| [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") | Hit slop is Day's own routing of the press, since Qt has no per-widget hit test to override (`Cap::HitSlop` answers `Emulated`), and a scroll area's viewport clips it. |
| [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") | A piece with no handler passes presses, double clicks and the wheel through to what is behind it. |
| [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") | A long press waits Qt's press-and-hold interval, 800 ms. |
| [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") | Qt measures no velocity, so a release velocity is always Day's estimate. |
| [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") | A mouse drag never scrolls a scroll area, so a drag across a row's `on_drag_along` axis scrolls nothing; the wheel still does. |
| [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") | On macOS [`.on_touches`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_touches) reports one derived contact. |
| [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") | Hit slop is the parent's `TouchDelegate`, so it cannot reach past the parent's bounds. |
| [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") | A view with a context menu keeps the long press for the menu, and [`.on_long_press`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_long_press) does not fire there. |
| [WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") | Hit slop grows containers only, as transparent bands around the panel; a native control, a label or an image keeps its own bounds. An ancestor that clips (a scroll view, a rounded corner) clips the bands. |
| [WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") | XAML reports holding for touch and pen only; a mouse long press is Day's 500 ms timer. |
| [WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") | An element with a context menu keeps the long press for the menu, and [`.on_long_press`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_long_press) does not fire there. |
| [ArkUI](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") | Hit slop cannot reach past the parent's bounds, where ArkUI's hit test stops, and leading and trailing slop apply as left and right in either reading direction. |
| [ArkUI](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") | [`.on_hover`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_hover) never fires: the C API's hover event carries no point ([canvas](canvas.md) "Hover"). |
