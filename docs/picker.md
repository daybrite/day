# Picker (external piece)

> **Status: implemented** as `day-piece-picker` — an EXTERNAL Day Piece (like `day-piece-combobox`),
> registered link-time into each backend's renderer slice with **zero edits** to day. One API, three
> SwiftUI-style stylings, each a distinct NATIVE control per toolkit, bound two-way to a selection.

## Authoring

```rust
use day_piece_picker::picker;

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

| style | AppKit | UIKit | GTK | Qt | Android | WinUI |
|---|---|---|---|---|---|---|
| **Menu** | `NSPopUpButton` | `UIButton`+`UIMenu` pull-down | `GtkDropDown` | `QComboBox` | `Spinner` | `ComboBox` |
| **Segmented** | `NSSegmentedControl` | `UISegmentedControl` | `.linked` grouped `GtkToggleButton`s | checkable `QPushButton`s in a `QButtonGroup` | button-row `LinearLayout` (dim unselected) | horizontal `RadioButton` `StackPanel` |
| **Inline** | vertical `NSStackView` of radio `NSButton`s | checkmark-row `UIStackView` | grouped `GtkCheckButton`s (radio) | `QRadioButton`s in a `QButtonGroup` | `RadioGroup` | vertical `RadioButton` `StackPanel` |

The **Qt and WinUI renderers each carry their OWN C++ shim** inside this crate (`src/qt_shim.cpp`,
`src/winui_shim.cpp`), compiled by the crate's `build.rs` — no edits to day's toolkit crates. The WinUI
shim boxes its native XAML element into a day handle through the **`day_winui_box` / `day_winui_unbox`
seam** that `day-winui-sys` exports (so a piece never has to touch day-winui's private handle wrapper —
the same way the Qt shim's handle is just a raw `QWidget*`). Android builds via the `DayBridge.makePicker`
Java factory. All styles report selection through `Event::SelectionChanged(i64)`; programmatic selection
is echo-guarded per backend (idClicked-only / suppress flags / signal blocking) so it never loops.

> The `day_winui_box`/`day_winui_unbox` seam is a **general** day-winui-sys capability: any external
> piece can now carry its own native WinUI shim, exactly like the Qt shims. Before it, WinUI handles
> (a private boxed `Node*`) could only be produced inside day-winui-sys, which is why external pieces
> previously had to reuse day-winui-sys's built-in controls.

## Verification

The showcase **Pickers** page (`pickers_page`) shows all three styles, each with a live value label.
Screenshot-verified rendering + correct initial selection on **all 5** local targets (AppKit, GTK, Qt,
iOS-sim, Android-emu). The walkthrough drives `select` on each picker and asserts the bound value label
follows (`picker-*-value`) — proving the two-way binding round-trips the signal *and* the native patch on
every backend (96/96 steps).

## Follow-ups

- Reactive `options` (currently fixed at build; only `selected` patches) — mirror the combobox's `Items`.
- Disabled/enabled state; per-option a11y labels.

WinUI is CI-verified (the `windows-winui` job clippy-checks the module and runs the walkthrough's
picker steps); it isn't buildable on the macOS/Linux dev hosts (`cfg(windows)` + the Windows SDK).
