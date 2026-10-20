---
title: "Picker"
description: "The built-in picker piece: native segmented controls, dropdowns, and wheels, chosen per platform."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Picker (built-in)

> **Status: implemented** as a built-in piece (`kinds::PICKER`; moved in from the satellite
> `day-piece-picker` 2026-07; typed by value since 2026-10). One API, three SwiftUI-style
> stylings, each a distinct native control per toolkit, bound two-way to a typed selection. In
> `day::prelude::*`, with no dependency to add.

## Authoring

```rust
use day::prelude::*;

#[derive(Clone, Copy, PartialEq)]
enum Size { Small, Medium, Large }

let size = Signal::new(Size::Medium);
picker(size, [(Size::Small, res::str::small()), (Size::Medium, res::str::medium()), (Size::Large, res::str::large())])
    .segmented()
    .id(id!(size))                                                      // horizontal one-of-N
picker(color, colors).menu()                                         // dropdown / pop-up
picker(plan, plans).inline()                                         // vertical radio group
```

`picker(selected, options)` is bound two-way to a value of any `T: Clone + PartialEq` (an enum,
a `String`, a `usize`): `selected` is a `Signal<T>`, a day-model `Field`, or any other
`Binding<T>`, and `options` is an array or `Vec` of `(value, title)` pairs, the titles taking
the same sources as a label. The mapping between values and the native control's indexes lives
in the picker, once; there is no index to keep in step with an enum. Default style is `.menu()`;
`.segmented()` / `.inline()` / `.style(PickerStyle)` switch it. `Picker` implements `Piece`, so
`.id()`/`.a11y()`/`.frame()` chain via `Decorate`.

`picker_optional(selected, options)` binds an `Option<T>`: `None` is "nothing chosen" (no
selection where the toolkit can show none), and a user's choice writes `Some(value)`.

### Selection rules

The picker never writes the binding on its own; only a user's choice does. A bound value that
is not among the options shows no selection where the toolkit can show none, and the first
option otherwise. Empty options disable the control. When the options change, the native index
is re-derived from the value, and a native choice that arrives after the options changed is
mapped against the options the control was last given, so a late event cannot pick the wrong
value.

### Menu groups

`.separators_before([6])` inserts a native menu divider before option 6 on AppKit/UIKit.
The boundary consumes no selection index, and duplicate option labels remain independent
choices (for example, recommended fonts repeated in a complete font catalog). Boundaries
are fixed at build time, survive option patches, and are ignored when out of range or zero.
Other backends and non-menu styles present the same options without decorative grouping.

### Options that come from data

The options are fixed by default. When they are not (a list of open documents, a tab that
names its own contents), pass a closure that derives them instead of a list:

```rust
picker(tab, move || docs.get().iter().map(|d| (d.id, d.title.clone())).collect::<Vec<_>>())
    .segmented()
```

The closure runs like any other reactive read, so it re-fires when the signals it touches
change, and the new options patch the native items in place: the pop-up refills, the segmented
control relabels and grows or shrinks its buttons, the radio group does the same. The native
index is re-derived from the bound value against the new options (the selection rules above),
so a shrinking list cannot leave a native control pointing at the wrong value. The app still
owns the value: write a valid one to the binding when its option disappears.

Use it only when the options change. Rebuilding a native menu costs more than
moving a mark, and a picker whose choices are constant should stay constant.

## Per-backend native realization

| style | AppKit | UIKit | GTK | Qt | Android | XAML | ArkUI |
|---|---|---|---|---|---|---|---|
| **Menu** | `NSPopUpButton` | `UIButton`+`UIMenu` pull-down | `GtkDropDown` | `QComboBox` | `Spinner` | `ComboBox` | ArkTS `Select` (wheel on an older host) |
| **Segmented** | `NSSegmentedControl` | `UISegmentedControl` | `.linked` grouped `GtkToggleButton`s | checkable `QPushButton`s in a `QButtonGroup` | button-row `LinearLayout` (dim unselected) | horizontal `RadioButton` `StackPanel` | compact ArkTS button row (wheel on an older host) |
| **Inline** | vertical `NSStackView` of radio `NSButton`s | checkmark-row `UIStackView` | grouped `GtkCheckButton`s (radio) | `QRadioButton`s in a `QButtonGroup` | `RadioGroup` | vertical `RadioButton` `StackPanel` | `TextPicker` wheel |

ArkUI renders **Menu** as HarmonyOS's own dropdown, the ArkTS `Select` component. The C node API
has no select kind, so it is a built-in ArkTS piece the host page registers (`DaySelect.ets` in
day-arkui's host, passed to `registerDayPieces`); it is sized once for its widest option, so a choice
never changes its width. **Segmented** uses a compact row of native ArkTS buttons with an
accessible selected state, sized for the widest label. **Inline**, and hosts too old to register
the ArkTS picker, use the native `ARKUI_NODE_TEXT_PICKER` wheel. The Qt and XAML renderers each carry a C++ shim in the
matching `-sys` crate (`toolkits/day-qt-sys/src/shim-picker.cpp`,
`toolkits/day-xaml-sys/src/shim-picker.cpp`); the XAML shim boxes its XAML element into a Day handle
through the `day_xaml_box`/`day_xaml_unbox` functions. Android's Java factory
(`toolkits/day-android/java/dev/daybrite/day/piece/picker/DayPicker.java`) rides the framework shim.
All backends report selection through `Event::SelectionChanged(i64)`; programmatic selection is
echo-guarded per backend so it never loops.

## Verification

The showcase **Controls** page (`controls.rs` `pickers_section`) shows all three styles bound to one
shared selection signal, each with a live value label. Rendering and correct initial selection are
screenshot-verified on all 5 local targets (AppKit, GTK, Qt, iOS-sim, Android-emu); a mock-backend
test (`crates/day-pieces/tests/mock_e2e.rs` `picker_and_text_area_are_built_in`) asserts the two-way
binding round-trips. The walkthrough drives `select` through each styling and asserts the readouts
follow.

## Follow-ups

- Reactive `options` (currently fixed at build; only `selected` patches), mirroring the combobox's
  `Items`.
- Per-option a11y labels.

(`.enabled(e)`, constant or reactive, shipped 2026-10: `PickerProps::enabled` at realize and
`PickerPatch::Enabled` on change, disabling every style on all eight toolkits, the composite
segmented and inline styles child by child.)
