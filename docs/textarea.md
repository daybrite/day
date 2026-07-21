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

It is the multi-line sibling of `text_field` (docs/forms.md): a field is one line and submits on
Return; a text area keeps newlines. Both raise the soft keyboard through the focus system
(docs/focus.md), and keyboard avoidance (the focused editor scrolling clear of the keyboard) applies
to both.

## Per-backend native realization

| AppKit | UIKit | GTK | Qt | Android | WinUI | ArkUI |
|---|---|---|---|---|---|---|
| `NSTextView` in `NSScrollView` | `UITextView` | `GtkTextView` in `GtkScrolledWindow` | `QPlainTextEdit` | multi-line `EditText` | wrapping `TextBox` | `ARKUI_NODE_TEXT_AREA` |

Each backend keeps the `(min_lines, max_lines)` band and grows its `measure` height in a line band.
Text changes report through `Event::TextChanged(String)`; programmatic sync (`TextAreaPatch::SetText`)
is echo-guarded per backend. The Qt and WinUI renderers carry C++ shims in the matching `-sys` crate
(`shim-textarea.cpp`); Android's `DayTextArea.java` rides the framework shim.

## Verification

The Matrix app's composer (`apps/matrix/src/lib.rs`) is the shipped consumer. A mock-backend test
(`crates/day-pieces/tests/mock_e2e.rs` `picker_and_text_area_are_built_in`) asserts the two-way binding
round-trips.

## Follow-ups

- Rich text / attributed runs (a separate `RichText` piece; DESIGN §B.5).
- Reactive placeholder; a character/line counter affordance.
