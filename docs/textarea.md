# Text area (built-in)

> **Status: implemented** as a built-in piece (`kinds::TEXT_AREA`; moved in from the satellite
> `day-piece-textarea` 2026-07). A native multi-line text editor bound two-way to a string, with
> an auto-growing height band. In `day::prelude::*` — no dependency to add.

## Authoring

```rust
use day::prelude::*;

let body = Signal::new(String::new());
text_area(body)
    .placeholder("Write a message…")
    .min_lines(3)     // never shorter than 3 lines
    .max_lines(8)     // grows to 8, then scrolls internally
    .id("compose")
```

`text_area(text)` binds a `Signal<String>` two-way: keystrokes write the signal, and setting the
signal replaces the editor's text (echo-guarded, so a programmatic set that matches the last typed
value doesn't loop). `.placeholder(_)` sets the empty-state prompt (evaluated once — not reactive).
The height auto-grows with content between `.min_lines(_)` (default 1) and `.max_lines(_)` (default
`0` = unbounded, never scrolls); a non-zero max is floored to min. `TextArea` implements `Piece`, so
`.id()`/`.a11y()`/`.frame()` chain via `Decorate`.

### Attributes

Three attributes control the native editor, each a reactive `bool` (a constant or a signal/closure)
that updates live:

```rust
text_area(report)
    .editable(false)      // read-only (default true)
    .selectable(true)     // still copyable (default true)
    .spellcheck(false)    // no spell-correction squiggles (default true)
```

- **`.editable(v)`** — `false` makes the editor read-only.
- **`.selectable(v)`** — whether the text can be selected and copied (useful with `.editable(false)`
  for a read-only-but-copyable display).
- **`.spellcheck(v)`** — the spell-check / autocorrect highlighting.

Native support varies; a toolkit that can't honor an attribute answers the matching capability with
`Support::Unsupported`, so an app can gray out a control that would do nothing (the showcase's Text
Areas page does this with `capability(Cap::TextSpellCheck) == Support::Native`):

| attribute | Cap | AppKit | UIKit | GTK | Qt | Android | XAML | ArkUI |
|---|---|---|---|---|---|---|---|---|
| editable | `Cap::TextEditable` | ✓ | ✓ | ✓ | ✓ | ✓ | follow-up | follow-up |
| selectable | `Cap::TextSelectable` | ✓ | ✓ | — (always on) | ✓ | ✓ | follow-up | follow-up |
| spell-check | `Cap::TextSpellCheck` | ✓ | ✓ | — (none built in) | — (none) | ✓ | follow-up | follow-up |

GTK's `GtkTextView` is always selectable (no toggle), and neither GTK nor Qt ships a built-in
spell-checker (that needs libspelling/gspell or Hunspell). The **XAML** (`TextBox` has
`IsReadOnly`/`IsTextSelectionEnabled`/`IsSpellCheckEnabled`) and **ArkUI** editors support the
attributes natively, but their shims don't yet expose the setters — a documented follow-up; until
then those two report `Unsupported` for all three and ignore the props.

It is the multi-line sibling of `text_field` (docs/forms.md): a field is one line and submits on
Return; a text area keeps newlines. Both raise the soft keyboard through the focus system
(docs/focus.md), and keyboard avoidance (the focused editor scrolling clear of the keyboard) applies
to both.

## Per-backend native realization

| AppKit | UIKit | GTK | Qt | Android | XAML | ArkUI |
|---|---|---|---|---|---|---|
| `NSTextView` in `NSScrollView` | `UITextView` | `GtkTextView` in `GtkScrolledWindow` | `QPlainTextEdit` | multi-line `EditText` | wrapping `TextBox` | `ARKUI_NODE_TEXT_AREA` |

Each backend keeps the `(min_lines, max_lines)` band and grows its `measure` height in a line band.
Text changes report through `Event::TextChanged(String)`; programmatic sync (`TextAreaPatch::SetText`)
is echo-guarded per backend, and the attribute patches (`SetEditable`/`SetSelectable`/`SetSpellCheck`)
apply the native property. The Qt and XAML renderers carry C++ shims in the matching `-sys` crate
(`shim-textarea.cpp` — Qt adds `day_textarea_set_attrs`/`set_read_only`/`set_selectable`); Android's
`DayTextArea.java` rides the framework shim (its `applyAttrs` maps editable→InputType/keyListener,
selectable→`setTextIsSelectable`, spell-check→`TYPE_TEXT_FLAG_NO_SUGGESTIONS`).

## Verification

The Matrix app's composer (`apps/matrix/src/lib.rs`) is the shipped consumer. A mock-backend test
(`crates/day-pieces/tests/mock_e2e.rs` `picker_and_text_area_are_built_in`) asserts the two-way binding
round-trips.

## Follow-ups

- **XAML + ArkUI attribute setters**: `TextBox`/`ARKUI_NODE_TEXT_AREA` support editable/selectable/
  spell-check natively, but their shims don't expose the setters yet — they report `Unsupported` and
  ignore the props for now.
- Rich text / attributed runs (a separate `RichText` piece; DESIGN §B.5).
- Reactive placeholder; a character/line counter affordance.
