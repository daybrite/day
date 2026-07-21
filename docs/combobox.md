# Combo box (external piece)

> **Status: implemented** as `day-piece-combobox`, an external Day Piece (like
> `day-piece-searchfield`) registered link-time into each backend's renderer slice without
> touching day. One API: free-form text entry PLUS a native dropdown of suggestions, bound
> two-way to a `Signal<String>` — the text IS the value — with a reactive
> `Signal<Vec<String>>` item list. Reworked 2026-07 from a selection-only dropdown (an
> `NSPopUpButton`-style control bound to an index) into the real thing; `picker`
> (docs/picker.md) is now the one-of-N control, the combo box is for values that need not be
> in the list.

## Authoring

```rust
use day_piece_combobox::combo_box;

let flavors = Signal::new(vec!["vanilla".into(), "chocolate".into()]);
let flavor = Signal::new(String::new());
combo_box(flavors, flavor).placeholder("Type or pick a flavor").id("flavor")
```

`combo_box(items, text)` binds `text` two-way: typing writes the signal per keystroke, picking a
dropdown item writes the item's string, and setting the signal patches the control (echo-guarded,
§4.4 — the guard remembers the last value that arrived from the native control so `bind_seeded`
does not patch it straight back). `items` is reactive: a change patches the native dropdown live,
and the typed text survives the swap. `.placeholder(impl IntoText)` sets the empty-state prompt
(read once at build). `ComboBox` implements `Piece`, so `.id()`/`.a11y()`/`.frame()` chain via
`Decorate`. Like `text_field` it is a **width-growing leaf** (`grow_w = true`, natural
single-line height).

Every backend reports both change paths — typing and picking — through the one
`Event::TextChanged(String)`, so `dayscript`'s `input:` step drives the entry on every backend.
The front-end also answers a synthetic `Event::SelectionChanged(i)` by mapping `i` through the
current items, so `dayscript`'s `select:` step is the scripted menu path (native backends never
emit it — a native pick already arrives as text).

## Per-backend native realization

| AppKit | GTK | Qt | Android | WinUI | UIKit | ArkUI |
|---|---|---|---|---|---|---|
| `NSComboBox` | `GtkComboBoxText` with entry | editable `QComboBox` | `AutoCompleteTextView` | editable `ComboBox` (1809+) | — placeholder | — placeholder |

iOS and HarmonyOS have no native combo-box control, so the piece deliberately carries **no
renderer** there: day renders its placeholder leaf, and the showcase adds a footnote saying why.
Use `picker` or `text_field` on those platforms. The change plumbing per backend:

- **AppKit**: one per-node delegate serves both halves —
  `NSControlTextEditingDelegate::controlTextDidChange:` for keystrokes and
  `NSComboBoxDelegate::comboBoxSelectionDidChange:` for picks. The selection notification fires
  *before* the control writes the pick into its own `stringValue`, so the handler reads the
  selected item's string instead. `setCompletes(true)` gives inline autocompletion. Programmatic
  `setStringValue` fires neither (no suppression needed).
- **GTK**: `GtkComboBoxText::with_entry()`; the internal `GtkEntry`'s `changed` signal is the
  single change path (a pick writes the entry). It also fires on programmatic `set_text`, so a
  per-node `suppress` cell guards the sync. GTK 4.10 deprecated `GtkComboBoxText` without an
  editable replacement (`GtkDropDown` has no entry), so the renderer keeps it under a commented
  `allow(deprecated)`.
- **Qt / WinUI**: each carries its own C++ shim inside this crate (`src/lib-qt-shim.cpp`,
  `src/lib-winui-shim.cpp`), compiled by the crate's `build.rs`. The Qt shim is a
  `QComboBox` with `setEditable(true)` + `NoInsert`; `editTextChanged` is the single change
  path, and programmatic setters sit in `blockSignals`. The WinUI shim boxes an editable
  `ComboBox` through the `day_winui_box` / `day_winui_unbox` seam. Documented divergence: XAML's
  `ComboBox` has no per-keystroke text event, so free-form text commits on Enter or focus loss
  (`TextSubmitted` / `LostFocus`) while picks report immediately (`SelectionChanged`).
- **Android**: carries its own Java factory
  (`android/java/dev/daybrite/day/piece/combobox/DayCombo.java`), folded into the app's Gradle
  build via `[package.metadata.day.android]` with no edits to day-android. Android's combo box
  is `AutoCompleteTextView`: suggestions prefix-filter while typing, and a tap or focus pops the
  dropdown open so the list is reachable without typing. One `TextWatcher` reports both paths as
  `DayBridge.K_TEXT_CHANGED`; the programmatic setters guard on equality.

## Verification

The showcase **Controls** page (`controls.rs` `flavor_block`) binds a `combo_box` to a `flavor`
signal with a localized three-item list, an **Add** button that pushes the typed text into the
items, and a readout mirroring the signal. The walkthrough drives all three behaviors: `select`
index 2 (menu path, asserted by localized key), `input` a literal that is in no list (free-form
path), then Add + `select` index 3 — an index that exists only after the add — proving the
reactive item list round-trips. Runs on macOS-AppKit, GTK, Qt, iOS-sim (placeholder + synthetic
steps), and the Android emulator. Rust is clippy-clean (`-D warnings`) and `cargo fmt`-clean for
every backend feature.

## Follow-ups

- Reactive placeholder (currently fixed at build).
- `Event::Submitted` on Return for "commit" semantics distinct from per-keystroke changes.
- Disabled/enabled state; a max-visible-items hint for the dropdown.

WinUI is CI-only (built under `cfg(windows)` + the Windows SDK); it isn't buildable on the
macOS/Linux dev hosts.
