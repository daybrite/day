---
title: "Split"
description: "The split piece: two panes and the divider the user drags, side by side or stacked, the share bound to a signal; the toolkit's own splitter where there is one."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Split (`split`)

> **Status: implemented** (2026-10). Native on macos-appkit (`NSSplitView`), gtk (`GtkPaned`)
> and qt (`QSplitter`); composed on ios-uikit, android-mdc, harmony-arkui, web-dom, xaml and
> mock, where no toolkit ships a general two-pane splitter (`Cap::Split = Unsupported` there;
> the piece supplies the panes, the divider and the drag itself). Exercised by Day-Yaak,
> whose request form and response view sit in one.

A `split` puts two panes in the space it is given with a divider between them that the user
drags, the editor-beside-preview shape. Like every Day surface it projects app-owned state:

```rust
let share = Signal::new(0.5);          // the first pane's share of the length
let stacked = Signal::new(false);      // the layout toggle a toolbar button flips
split(request_form(), response_view())
    .axis(move || if stacked.get() { SplitAxis::Vertical } else { SplitAxis::Horizontal })
    .fraction(share)
```

- `.axis(..)` takes a `SplitAxis` constant, a signal or a closure: `Horizontal` (the default)
  puts the panes side by side, first leading; `Vertical` stacks them, first on top. The name
  says where the panes go, which is what an app's "layout" toggle means; the divider runs
  the other way. A live axis turns the split in place and keeps the share.
- `.fraction(signal)` binds the first pane's share, `0.0..=1.0`, two-way: a dragged divider
  writes it, and writing it moves the divider. Unbound, the split keeps a share of its own,
  starting at a half. An app that persists the share stores one number per split.
- `.min_pane(points)` is the least length either pane can be dragged to (default 80).

The split is greedy: it takes whatever its parent proposes on both axes, the way a nav host
does, so it goes where a `.grow()` child would.

## Native and composed

Where `Cap::Split` answers `Native` the piece realizes `kinds::SPLIT` and the toolkit's own
splitter draws the divider, drags it and sizes the panes:

| Toolkit | Splitter | Notes |
|---|---|---|
| macos-appkit | `NSSplitView` (thin divider), `vertical` set from the axis | pane frames native-owned, reported through `Event::FrameChanged` as an inspector pane's are; a drag reports `Event::ValueChanged` with the new share |
| gtk | `GtkPaned`, orientation from the axis, both children filling `DayCell`s | the position-notify reports the share |
| qt | `QSplitter`, orientation from the axis | `splitterMoved` reports the share |

Each pane is a `kinds::SPLIT_PANE` whose frame the splitter owns; Day lays the pane's content
out inside the size the toolkit last reported, the same native-owned-frame contract as nav
pages ([docs/navigation.md](navigation.md)) and inspector panes
([docs/inspector.md](inspector.md)). The share patch (`SplitPatch::Fraction`) moves the
divider without re-emitting the report (the from-native echo rule), and the axis patch
(`SplitPatch::Axis`) turns the splitter with its panes in place.

Everywhere else the piece composes: Day's layout places the first pane, a 9-point divider
and the second pane by the share; the divider is a canvas drawing a hairline down its middle,
carrying a `.cursor()` (column- or row-resize) and an `.on_drag()` that moves the share by the
pointer's travel as a fraction of the split's length. It carries the id `day-split-divider`,
so a walkthrough can `drag` it. The behavior is the same on every target; only the divider's
look is the toolkit's where there is one.

## What a walkthrough can assert

The share is a signal, so a label bound to it is the cross-platform assertion: the
conformance cases write the share and read the first pane's frame back, and turn the axis and
read the second pane's position. A composed split takes a `drag` step on `day-split-divider`;
a native one takes `set_value` on the split's own id, which the piece reads as the toolkit's
report.
