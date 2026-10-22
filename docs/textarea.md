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

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](snippet-catalog.md) and [catalog setup](https://daybrite.dev/docs/how-to/localization).

## Authoring

```rust
use day::prelude::*;

let body = Signal::new(String::new());
text_area(body)
    .placeholder(res::str::write_a_message())
    .min_lines(3) // never shorter than 3 lines
    .max_lines(8) // grows to 8, then scrolls internally
    .id(id!(compose))
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
| editable | `Cap::TextEditable` | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | emulated |
| selectable | `Cap::TextSelectable` | ✓ | ✓ | — (always on) | ✓ | ✓ | emulated | emulated |
| spell-check | `Cap::TextSpellCheck` | ✓ | ✓ | — (none built in) | — (none) | ✓ | ✓ | follow-up |

GTK's `GtkTextView` is always selectable (no toggle), and neither GTK nor Qt ships a built-in
spell-checker (that needs libspelling/gspell or Hunspell).

**XAML** honors editable and spell-check with the plain `TextBox` properties `IsReadOnly` and
`IsSpellCheckEnabled`. Selection is the odd one out: `IsTextSelectionEnabled` is a `TextBlock` /
`RichTextBlock` property and `TextBox` carries no equivalent, so `.selectable(false)` is EMULATED:
the shim collapses each selection as `SelectionChanged` reports it and suppresses the context menu,
which is the other route to Copy / Select All.

**ArkUI** keeps its native `ARKUI_NODE_TEXT_AREA` and emulates editable/selectable policy. The
API 18 C interface has no persistent read-only or selection-enable property: `EDITING=false`
only ends the current editing session. Day vetoes native insert/delete callbacks while read-only,
suppresses keyboard activation/autofill, and restores the held text if an input path bypasses the
veto. Rejected changes never reach the binding; programmatic writes update the held value.
Read-only text remains selectable and copyable when `.selectable(true)`.

With `.selectable(false)`, Day hides the native selection menu, collapses selection-change events
to a caret, and intercepts Select All/Copy/Cut shortcuts before the IME. Typing remains enabled
when `.editable(true)`. Both policies apply at creation and reactively, without replacing the
editor. They report `Support::Emulated`; spell-check remains `Unsupported`.

It is the multi-line sibling of `text_field` ([docs/forms.md](forms.md)): a field is one line and submits on
Return; a text area keeps newlines. Both raise the soft keyboard through the focus system
([docs/focus.md](focus.md)), and keyboard avoidance (the focused editor scrolling clear of the keyboard) applies
to both.

### Submission

`.on_submit(f)` turns a text area into a composer: Enter calls `f` with the bound text already
current, rather than inserting a newline. The callback can read or clear the signal. Without
this modifier, Enter keeps its ordinary multiline behavior.

On **ArkUI**, the native editor advertises **Send** to the IME and routes
`NODE_TEXT_AREA_ON_SUBMIT` to the callback. Hardware Enter and the soft-keyboard Send button use
the same native action. Submission leaves the editor focused, does not clear the text unless
the callback does so, and is ignored while read-only. Pasting multiline text does not submit.
Shift+Enter inserts a newline at the current selection. API 18 has no Shift+Enter binding or
insert-text controller API, so Day dispatches an unmodified Enter through the ArkTS host while
temporarily selecting the native newline action. ArkUI performs the edit, preserving Unicode
selection, caret placement, and native undo/redo. The pre-IME modifier mask is empty on the
tested runtime, so Day reads its pressed-key list to recognize Shift and Ctrl reliably.
Custom ArkTS hosts must adopt the staged host's `registerTextAreaKey` callback for Shift+Enter;
without it, the shortcut is ignored with a diagnostic rather than replacing text outside the
native editing pipeline. `day build` stages this callback automatically for standard hosts.

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

The portable conformance cases cover initial read-only/unselectable editors and preserving
multiline text through reactive policy changes and programmatic writes. The
[native ArkUI input suite](testing.md#native-arkui-text-area-tests) additionally exercises actual
keyboard, clipboard and touch input; injected Day events cannot prove native input rejection.

## Follow-ups

- **ArkUI spell-check**: still unsupported; verify an appropriate SDK/IME contract before
  advertising it. The API 18 C TextArea interface has no spell-check toggle.
- Styled text is a different control: `day-piece-texteditor` edits a whole `StyledText` — the
  document labels render and `.markdown()` produces — in each platform's rich-text view
  ([docs/texteditor.md](texteditor.md)). `text_area` stays the plain-text one.
- Reactive placeholder; a character/line counter affordance.
