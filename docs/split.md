---
navTitle: "Split views"
title: "Split"
description: "The split piece: two panes and the divider the user drags, side by side or stacked, the share bound to a signal; the toolkit's own splitter where there is one."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Split

A split view places related content in two panes separated by a movable divider. It lets people
compare or work with both areas at once and adjust the space each receives.

The pane sizes in a [`split`](https://docs.rs/day-pieces/latest/day_pieces/fn.split.html) follow
app-owned state.

## Add to your app

This control is included in [`day-pieces`](https://github.com/daybrite/day/tree/main/crates/day-pieces)
and re-exported through `day::prelude::*`. No additional crate or control feature is required.

## Authoring

```rust
let share = Signal::new(0.5);          // the first pane's share of the length
let stacked = Signal::new(false);      // the layout toggle a toolbar button flips
split(request_form(), response_view())
    .axis(move || if stacked.get() { SplitAxis::Vertical } else { SplitAxis::Horizontal })
    .fraction(share)
```

- [`.axis(..)`](https://docs.rs/day-pieces/latest/day_pieces/trait.SplitBuilder.html#tymethod.axis) takes a `SplitAxis` constant, a signal or a closure: `Horizontal` (the default)
  puts the panes side by side, first leading; `Vertical` stacks them, first on top. The name
  says where the panes go, which is what an app's "layout" toggle means; the divider runs
  the other way. A live axis turns the split in place and keeps the share.
- [`.fraction(signal)`](https://docs.rs/day-pieces/latest/day_pieces/trait.SplitBuilder.html#tymethod.fraction) binds the first pane's share, `0.0..=1.0`, two-way: a dragged divider
  writes it, and writing it moves the divider. Unbound, the split keeps a share of its own,
  starting at a half. An app that persists the share stores one number per split.
- [`.min_pane(points)`](https://docs.rs/day-pieces/latest/day_pieces/trait.SplitBuilder.html#tymethod.min_pane) is the least length either pane can be dragged to (default 80).

The split is greedy: it takes whatever its parent proposes on both axes, the way a nav host
does, so it goes where a [`.grow()`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.grow) child would.

## Native and composed

Platform guidance: [Apple: split views](https://developer.apple.com/design/human-interface-guidelines/split-views).

Where `Cap::Split` answers `Native` the piece realizes `kinds::SPLIT` and the toolkit's own
splitter draws the divider, drags it and sizes the panes:

| Toolkit | Splitter | Notes |
|---|---|---|
| macos-appkit | [`NSSplitView`](https://developer.apple.com/documentation/appkit/nssplitview) (thin divider), `vertical` set from the axis | pane frames native-owned, reported through `Event::FrameChanged` as an inspector pane's are; a drag reports `Event::ValueChanged` with the new share |
| gtk | [`GtkPaned`](https://docs.gtk.org/gtk4/class.Paned.html), orientation from the axis, both children filling `DayCell`s | the position-notify reports the share |
| qt | [`QSplitter`](https://doc.qt.io/qt-6/qsplitter.html), orientation from the axis | `splitterMoved` reports the share |

Each pane is a `kinds::SPLIT_PANE` whose frame the splitter owns; Day lays the pane's content
out inside the size the toolkit last reported, the same native-owned-frame contract as nav
pages ([docs/navigation](navigation.md)) and inspector panes
([docs/inspector](inspector.md)). The share patch (`SplitPatch::Fraction`) moves the
divider without re-emitting the report (the from-native echo rule), and the axis patch
(`SplitPatch::Axis`) turns the splitter with its panes in place.

Everywhere else the piece composes: Day's layout places the first pane, a 9-point divider
and the second pane by the share; the divider is a canvas drawing a hairline down its middle,
carrying a [`.cursor()`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.cursor) (column- or row-resize) and an [`.on_drag()`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_drag) that moves the share by the
pointer's travel as a fraction of the split's length. The behavior is the same on every target; only the divider's
look is the toolkit's where there is one.
