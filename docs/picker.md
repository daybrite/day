---
navTitle: "Pickers"
title: "Picker"
description: "The built-in picker piece: native segmented controls, dropdowns, and wheels, chosen per platform."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Picker

Let people choose one value from a defined set, using a segmented control, dropdown, or wheel.
[`picker`](https://daybrite.dev/api/day_pieces/fn.picker.html) binds the selected value to app state and is available through `day::prelude::*`.

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](snippet-catalog.md) and [catalog setup](/docs/how-to/localization).

## Add to your app

Defined in [`day-pieces`](https://github.com/daybrite/day/tree/main/crates/day-pieces) and
re-exported by `day::prelude`. An app created with `day new` already has the required dependency;
otherwise add it to `Cargo.toml`:

```toml
[dependencies]
day = { git = "https://github.com/daybrite/day.git" }
```

No additional piece crate or `Day.toml` dependency is needed. Build with the app’s selected
toolkit using `day build -t <target>`.

| Platforms / toolkits | Availability |
| --- | --- |
| AppKit, UIKit, GTK, Qt, Android, WinUI, ArkUI, Web | Built-in control; see [native realization](#per-toolkit-native-realization) for presentation differences. |

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

[`picker(selected, options)`](https://daybrite.dev/api/day_pieces/fn.picker.html) is bound two-way to a value of any `T: Clone + PartialEq` (an enum,
a `String`, a `usize`): `selected` is a `Signal<T>`, a [day-model](/docs/explanation/model) `Field`, or any other
`Binding<T>`, and `options` is an array or `Vec` of `(value, title)` pairs, the titles taking
the same sources as a label. The mapping between values and the native control's indexes lives
in the picker, once; there is no index to keep in step with an enum. Default style is [`.menu()`](https://daybrite.dev/api/day_pieces/trait.PickerBuilder.html#tymethod.menu);
[`.segmented()`](https://daybrite.dev/api/day_pieces/trait.PickerBuilder.html#tymethod.segmented) / [`.inline()`](https://daybrite.dev/api/day_pieces/trait.PickerBuilder.html#tymethod.inline) / [`.style(PickerStyle)`](https://daybrite.dev/api/day_pieces/trait.PickerBuilder.html#tymethod.style) switch it. `Picker` implements `Piece`, so
[`.id()`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.id)/[`.a11y()`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.a11y)/[`.frame()`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.frame) chain via `Decorate`.

[`picker_optional(selected, options)`](https://daybrite.dev/api/day_pieces/fn.picker_optional.html) binds an `Option<T>`: `None` is "nothing chosen" (no
selection where the toolkit can show none), and a user's choice writes `Some(value)`.

### Selection rules

The picker never writes the binding on its own; only a user's choice does. A bound value that
is not among the options shows no selection where the toolkit can show none, and the first
option otherwise. Empty options disable the control. When the options change, the native index
is re-derived from the value, and a native choice that arrives after the options changed is
mapped against the options the control was last given, so a late event cannot pick the wrong
value.

### Menu groups

[`.separators_before([6])`](https://daybrite.dev/api/day_pieces/trait.PickerBuilder.html#tymethod.separators_before) inserts a native menu divider before option 6 on AppKit/UIKit.
The boundary consumes no selection index, and duplicate option labels remain independent
choices (for example, recommended fonts repeated in a complete font catalog). Boundaries
are fixed at build time, survive option patches, and are ignored when out of range or zero.
Other toolkits and non-menu styles present the same options without decorative grouping.

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

## Per-toolkit native realization

Platform guidance: [Apple: pickers](https://developer.apple.com/design/human-interface-guidelines/pickers); [Android: spinners](https://developer.android.com/develop/ui/views/components/spinner); [Windows: combo boxes](https://learn.microsoft.com/en-us/windows/apps/design/controls/combo-box).

| style | AppKit | UIKit | GTK | Qt | Android | XAML | ArkUI |
|---|---|---|---|---|---|---|---|
| **Menu** | [`NSPopUpButton`](https://developer.apple.com/documentation/appkit/nspopupbutton) | [`UIButton`](https://developer.apple.com/documentation/uikit/uibutton)+[`UIMenu`](https://developer.apple.com/documentation/uikit/uimenu) pull-down | [`GtkDropDown`](https://docs.gtk.org/gtk4/class.DropDown.html) | [`QComboBox`](https://doc.qt.io/qt-6/qcombobox.html) | [`Spinner`](https://developer.android.com/reference/android/widget/Spinner) | [`ComboBox`](https://learn.microsoft.com/en-us/uwp/api/windows.ui.xaml.controls.combobox) | ArkTS `Select` (wheel on an older host) |
| **Segmented** | [`NSSegmentedControl`](https://developer.apple.com/documentation/appkit/nssegmentedcontrol) | [`UISegmentedControl`](https://developer.apple.com/documentation/uikit/uisegmentedcontrol) | `.linked` grouped [`GtkToggleButton`](https://docs.gtk.org/gtk4/class.ToggleButton.html)s | checkable [`QPushButton`](https://doc.qt.io/qt-6/qpushbutton.html)s in a [`QButtonGroup`](https://doc.qt.io/qt-6/qbuttongroup.html) | button-row [`LinearLayout`](https://developer.android.com/reference/android/widget/LinearLayout) (dim unselected) | horizontal `RadioButton` [`StackPanel`](https://learn.microsoft.com/en-us/uwp/api/windows.ui.xaml.controls.stackpanel) | compact ArkTS button row (wheel on an older host) |
| **Inline** | vertical [`NSStackView`](https://developer.apple.com/documentation/appkit/nsstackview) of radio [`NSButton`](https://developer.apple.com/documentation/appkit/nsbutton)s | checkmark-row [`UIStackView`](https://developer.apple.com/documentation/uikit/uistackview) | grouped [`GtkCheckButton`](https://docs.gtk.org/gtk4/class.CheckButton.html)s (radio) | [`QRadioButton`](https://doc.qt.io/qt-6/qradiobutton.html)s in a [`QButtonGroup`](https://doc.qt.io/qt-6/qbuttongroup.html) | `RadioGroup` | vertical `RadioButton` [`StackPanel`](https://learn.microsoft.com/en-us/uwp/api/windows.ui.xaml.controls.stackpanel) | [`TextPicker`](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-arkui/arkui-ts/ts-basic-components-textpicker.md) wheel |

ArkUI renders **Menu** as HarmonyOS's own dropdown, the ArkTS `Select` component. The C node API
has no select kind, so it is a built-in ArkTS piece the host page registers (`DaySelect.ets` in
day-arkui's host, passed to `registerDayPieces`); it is sized once for its widest option, so a choice
never changes its width. **Segmented** uses a compact row of native ArkTS buttons with an
accessible selected state, sized for the widest label. **Inline**, and hosts too old to register
the ArkTS picker, use the native [`ARKUI_NODE_TEXT_PICKER`](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-arkui/capi-native-node-h.md) wheel. The Qt and XAML renderers each carry a C++ shim in the
matching `-sys` crate (`toolkits/day-qt-sys/src/shim-picker.cpp`,
`toolkits/day-xaml-sys/src/shim-picker.cpp`); the XAML shim boxes its XAML element into a Day handle
through the `day_xaml_box`/`day_xaml_unbox` functions. Android's Java factory
(`toolkits/day-mdc/java/dev/daybrite/day/piece/picker/DayPicker.java`) rides the framework shim.
All toolkits report selection through `Event::SelectionChanged(i64)`; programmatic selection is
echo-guarded per toolkit so it never loops.

## Follow-ups

- Per-option a11y labels.

([`.enabled(e)`](https://daybrite.dev/api/day_pieces/trait.PickerBuilder.html#tymethod.enabled), constant or reactive, shipped 2026-10: `PickerProps::enabled` at realize and
`PickerPatch::Enabled` on change, disabling every style on all eight toolkits, the composite
segmented and inline styles child by child.)
