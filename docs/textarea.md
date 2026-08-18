---
title: "Text area"
description: "The multi-line text editor piece: line hints, wrapping, and per-platform editors."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Text area (built-in)

> **Status: implemented** as a built-in piece (`kinds::TEXT_AREA`; moved in from the satellite
> `day-piece-textarea` 2026-07). A native multi-line text editor bound two-way to a string, with
> an auto-growing height band. In `day::prelude::*`, with no dependency to add.

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
value doesn't loop). `.placeholder(_)` sets the empty-state prompt (evaluated once, not reactive).
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
`Support::Unsupported`, so an app can gray out a control that would do nothing. `Emulated` means the
attribute IS honored, just not by one native property, so the test to gray a control is
`capability(…) == Support::Unsupported`, not `!= Support::Native` (the showcase's Text Areas page
gates its three toggles that way):

| attribute | Cap | AppKit | UIKit | GTK | Qt | Android | XAML | ArkUI |
|---|---|---|---|---|---|---|---|---|
| editable | `Cap::TextEditable` | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | follow-up |
| selectable | `Cap::TextSelectable` | ✓ | ✓ | — (always on) | ✓ | ✓ | emulated | follow-up |
| spell-check | `Cap::TextSpellCheck` | ✓ | ✓ | — (none built in) | — (none) | ✓ | ✓ | follow-up |

GTK's `GtkTextView` is always selectable (no toggle), and neither GTK nor Qt ships a built-in
spell-checker (that needs libspelling/gspell or Hunspell).

**XAML** honors editable and spell-check with the plain `TextBox` properties `IsReadOnly` and
`IsSpellCheckEnabled`. Selection is the odd one out: `IsTextSelectionEnabled` is a `TextBlock` /
`RichTextBlock` property and `TextBox` carries no equivalent, so `.selectable(false)` is EMULATED:
the shim collapses each selection as `SelectionChanged` reports it and suppresses the context menu,
which is the other route to Copy / Select All. The **ArkUI** editor supports all three natively but
its shim doesn't expose the setters yet, a documented follow-up; until then it reports
`Unsupported` for all three and ignores the props.

It is the multi-line sibling of `text_field` ([docs/forms.md](forms.md)): a field is one line and submits on
Return; a text area keeps newlines. Both raise the soft keyboard through the focus system
([docs/focus.md](focus.md)), and keyboard avoidance (the focused editor scrolling clear of the keyboard) applies
to both.

## Per-backend native realization

| AppKit | UIKit | GTK | Qt | Android | XAML | ArkUI |
|---|---|---|---|---|---|---|
| `NSTextView` in `NSScrollView` | `UITextView` | `GtkTextView` in `GtkScrolledWindow` | `QPlainTextEdit` | multi-line `EditText` | wrapping `TextBox` | `ARKUI_NODE_TEXT_AREA` |

Each backend keeps the `(min_lines, max_lines)` band and grows its `measure` height in a line band.
Text changes report through `Event::TextChanged(String)`; programmatic sync (`TextAreaPatch::SetText`)
is echo-guarded per backend, and the attribute patches (`SetEditable`/`SetSelectable`/`SetSpellCheck`)
apply the native property. The Qt and XAML renderers carry C++ shims in the matching `-sys` crate
(`shim-textarea.cpp`: Qt adds `day_textarea_set_attrs`/`set_read_only`/`set_selectable`; XAML adds
`day_textarea_xaml_set_editable`/`set_selectable`/`set_spellcheck`, applied at build as well as on
patch so an editor that starts read-only comes up that way); Android's
`DayTextArea.java` rides the framework shim (its `applyAttrs` maps editable→InputType/keyListener,
selectable→`setTextIsSelectable`, spell-check→`TYPE_TEXT_FLAG_NO_SUGGESTIONS`).

## Verification

The Day-Matrix app's composer (a standalone Day app) is the shipped consumer. A mock-backend test
(`crates/day-pieces/tests/mock_e2e.rs` `picker_and_text_area_are_built_in`) asserts the two-way binding
round-trips.

## Follow-ups

- **XAML + ArkUI attribute setters**: `TextBox`/`ARKUI_NODE_TEXT_AREA` support editable/selectable/
  spell-check natively, but their shims don't expose the setters yet; they report `Unsupported` and
  ignore the props for now.
- Styled text is a different control: `day-piece-texteditor` edits a whole `StyledText` — the
  document labels render and `.markdown()` produces — in each platform's rich-text view
  ([docs/texteditor.md](texteditor.md)). `text_area` stays the plain-text one.
- Reactive placeholder; a character/line counter affordance.
