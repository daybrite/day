---
title: "Combo box"
description: "An editable text field with a native completion popup, as the external day-piece-combobox crate."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Combo box

A combo box lets people type a value or choose a suggestion. `day-piece-combobox` binds the text
and suggestion list to app state. For a choice restricted to listed values, use a
[picker](/docs/reference/picker).

## Add to your app

Defined in [`day-piece-combobox`](https://github.com/daybrite/day/tree/main/pieces/day-piece-combobox). Add this to your app’s `Cargo.toml`
`[dependencies]` table, using the same Day Git source as the rest of your app:

```toml
[dependencies]
day-piece-combobox = { git = "https://github.com/daybrite/day.git" }
```

`day build -t <target>` enables this direct dependency’s matching toolkit feature
automatically. No dependency entry is needed in `Day.toml`.

| Platforms / toolkits | Availability |
| --- | --- |
| AppKit, GTK, Qt, Android, WinUI | Native editable choice control. |
| ArkUI | Composed text input and suggestions menu. |
| UIKit, Web | No renderer; use a [picker](/docs/reference/picker) or [text field](/docs/reference/textfield). |

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](snippet-catalog.md) and [catalog setup](/docs/how-to/localization).

## Authoring

```rust
use day_piece_combobox::combo_box;

let flavors = Signal::new(vec!["vanilla".into(), "chocolate".into()]);
let flavor = Signal::new(String::new());
combo_box(flavors, flavor)
    .placeholder(res::str::type_or_pick_a_flavor())
    .id(id!(flavor))
```

`combo_box(items, text)` binds `text` two-way: typing writes the signal per keystroke, picking a
dropdown item writes the item's string, and setting the signal patches the control (echo-guarded,
the guard remembers the last value that arrived from the native control so `bind_seeded`
does not patch it straight back). `items` is reactive: a change patches the native dropdown live,
and the typed text survives the swap. `.placeholder(impl IntoText)` sets the empty-state prompt
(read once at build). `ComboBox` implements `Piece`, so `.id()`/`.a11y()`/`.frame()` chain via
`Decorate`. Like `text_field` it is a **width-growing leaf** (`grow_w = true`, natural
single-line height).

Every toolkit reports both change paths (typing and picking) through the one
`Event::TextChanged(String)`, so `dayscript`'s `input:` step drives the entry on every toolkit.
The front-end also answers a synthetic `Event::SelectionChanged(i)` by mapping `i` through the
current items, so `dayscript`'s `select:` step is the scripted menu path (native toolkits never
emit it: a native pick already arrives as text).

## Per-toolkit native realization

Platform guidance: [Apple: pickers](https://developer.apple.com/design/human-interface-guidelines/pickers); [Android: spinners](https://developer.android.com/develop/ui/views/components/spinner); [Windows: combo boxes](https://learn.microsoft.com/en-us/windows/apps/design/controls/combo-box).

| AppKit | GTK | Qt | Android | XAML | UIKit | ArkUI |
|---|---|---|---|---|---|---|
| [`NSComboBox`](https://developer.apple.com/documentation/appkit/nscombobox) | [`GtkComboBoxText`](https://docs.gtk.org/gtk4/class.ComboBoxText.html) with entry | editable [`QComboBox`](https://doc.qt.io/qt-6/qcombobox.html) | [`AutoCompleteTextView`](https://developer.android.com/reference/android/widget/AutoCompleteTextView) | editable [`ComboBox`](https://learn.microsoft.com/en-us/uwp/api/windows.ui.xaml.controls.combobox) (1809+) | — placeholder | ArkTS `TextInput` + suggestions menu |

iOS has no native combo-box control, so the piece carries **no renderer** there: day renders its
placeholder leaf, and the showcase adds a footnote saying why. Use `picker` or `text_field` on
iOS. HarmonyOS has none either, but ArkTS composes one cheaply: the piece ships an ArkTS half
(`platform/harmony/ets`) that sets a `TextInput` beside a chevron button whose `bindMenu` lists
the items, and reports typing and picks alike as `Event::TextChanged`, as the native toolkits do. The change plumbing per toolkit:

- **AppKit**: one per-node delegate serves both halves,
  [`NSControlTextEditingDelegate::controlTextDidChange:`](https://developer.apple.com/documentation/appkit/nscontroltexteditingdelegate) for keystrokes and
  [`NSComboBoxDelegate::comboBoxSelectionDidChange:`](https://developer.apple.com/documentation/appkit/nscomboboxdelegate) for picks. The selection notification fires
  *before* the control writes the pick into its own `stringValue`, so the handler reads the
  selected item's string instead. `setCompletes(true)` gives inline autocompletion. Programmatic
  `setStringValue` fires neither (no suppression needed).
- **GTK**: [`GtkComboBoxText::with_entry()`](https://docs.gtk.org/gtk4/class.ComboBoxText.html); the internal [`GtkEntry`](https://docs.gtk.org/gtk4/class.Entry.html)'s `changed` signal is the
  single change path (a pick writes the entry). It also fires on programmatic `set_text`, so a
  per-node `suppress` cell guards the sync. GTK 4.10 deprecated [`GtkComboBoxText`](https://docs.gtk.org/gtk4/class.ComboBoxText.html) without an
  editable replacement ([`GtkDropDown`](https://docs.gtk.org/gtk4/class.DropDown.html) has no entry), so the renderer keeps it under a commented
  `allow(deprecated)`.
- **Qt / XAML**: each carries its own C++ shim inside this crate (`src/lib-qt-shim.cpp`,
  `src/lib-xaml-shim.cpp`), compiled by the crate's `build.rs`. The Qt shim is a
  [`QComboBox`](https://doc.qt.io/qt-6/qcombobox.html) with `setEditable(true)` + `NoInsert`; `editTextChanged` is the single change
  path, and programmatic setters sit in `blockSignals`. The XAML shim boxes an editable
  [`ComboBox`](https://learn.microsoft.com/en-us/uwp/api/windows.ui.xaml.controls.combobox) through the `day_xaml_box` / `day_xaml_unbox` functions. Documented divergence: XAML's
  [`ComboBox`](https://learn.microsoft.com/en-us/uwp/api/windows.ui.xaml.controls.combobox) has no per-keystroke text event, so free-form text commits on Enter or focus loss
  (`TextSubmitted` / `LostFocus`) while picks report immediately (`SelectionChanged`).
- **Android**: carries its own Java factory
  (`src/DayCombo.java`), folded into the app's Gradle
  build via `[package.metadata.day.android]` with no edits to day-android. Android's combo box
  is [`AutoCompleteTextView`](https://developer.android.com/reference/android/widget/AutoCompleteTextView): suggestions prefix-filter while typing, and a tap or focus pops the
  dropdown open so the list is reachable without typing. One [`TextWatcher`](https://developer.android.com/reference/android/text/TextWatcher) reports both paths as
  `DayBridge.K_TEXT_CHANGED`; the programmatic setters guard on equality.

## Follow-ups

- Reactive placeholder (currently fixed at build).
- `Event::Submitted` on Return for "commit" semantics distinct from per-keystroke changes.
- Disabled/enabled state; a max-visible-items hint for the dropdown.

XAML is CI-only (built under `cfg(windows)` + the Windows SDK); it isn't buildable on the
macOS/Linux dev hosts.
