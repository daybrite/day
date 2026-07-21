# Picker (built-in)

> **Status: implemented** as a built-in piece (`kinds::PICKER`; moved in from the satellite
> `day-piece-picker` 2026-07). One API, three SwiftUI-style stylings, each a distinct native
> control per toolkit, bound two-way to a selection. In `day::prelude::*` — no dependency to add.

## Authoring

```rust
use day::prelude::*;

let size = Signal::new(1usize);
picker(["Small", "Medium", "Large"], size).segmented().id("size")   // horizontal one-of-N
picker(colors, color).menu()                                         // dropdown / pop-up
picker(plans, plan).inline()                                         // vertical radio group
```

`picker(options, selected)` takes fixed `options` (`impl IntoIterator<Item: Into<String>>`) and a
`Signal<usize>` bound two-way (the widget writes the selected index back; setting the signal moves the
widget). Default style is `.menu()`; `.segmented()` / `.inline()` / `.style(PickerStyle)` switch it.
`Picker` implements `Piece`, so `.id()`/`.a11y()`/`.frame()` chain via `Decorate`.

## Per-backend native realization

| style | AppKit | UIKit | GTK | Qt | Android | WinUI | ArkUI |
|---|---|---|---|---|---|---|---|
| **Menu** | `NSPopUpButton` | `UIButton`+`UIMenu` pull-down | `GtkDropDown` | `QComboBox` | `Spinner` | `ComboBox` | `TextPicker` wheel |
| **Segmented** | `NSSegmentedControl` | `UISegmentedControl` | `.linked` grouped `GtkToggleButton`s | checkable `QPushButton`s in a `QButtonGroup` | button-row `LinearLayout` (dim unselected) | horizontal `RadioButton` `StackPanel` | `TextPicker` wheel |
| **Inline** | vertical `NSStackView` of radio `NSButton`s | checkmark-row `UIStackView` | grouped `GtkCheckButton`s (radio) | `QRadioButton`s in a `QButtonGroup` | `RadioGroup` | vertical `RadioButton` `StackPanel` | `TextPicker` wheel |

HarmonyOS has no segmented control, so ArkUI renders every style as the native `ARKUI_NODE_TEXT_PICKER`
wheel — the platform's option-selection idiom. The Qt and WinUI renderers each carry a C++ shim in the
matching `-sys` crate (`toolkits/day-qt-sys/src/shim-picker.cpp`,
`toolkits/day-winui-sys/src/shim-picker.cpp`); the WinUI shim boxes its XAML element into a Day handle
through the `day_winui_box`/`day_winui_unbox` seam. Android's Java factory
(`toolkits/day-android/java/dev/daybrite/day/piece/picker/DayPicker.java`) rides the framework shim.
All backends report selection through `Event::SelectionChanged(i64)`; programmatic selection is
echo-guarded per backend so it never loops.

## Verification

The showcase **Controls** page (`controls.rs` `pickers_section`) shows all three styles bound to ONE
shared selection signal, each with a live value label. Rendering and correct initial selection are
screenshot-verified on all 5 local targets (AppKit, GTK, Qt, iOS-sim, Android-emu); a mock-backend
test (`crates/day-pieces/tests/mock_e2e.rs` `picker_and_text_area_are_built_in`) asserts the two-way
binding round-trips. The walkthrough drives `select` through each styling and asserts the readouts
follow.

## Follow-ups

- Reactive `options` (currently fixed at build; only `selected` patches), mirroring the combobox's
  `Items`.
- Disabled/enabled state; per-option a11y labels.
