---
navTitle: "Text areas"
title: "Text area"
description: "The multi-line text editor piece: line hints, wrapping, and per-platform editors."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Text area

Let people write messages, notes, or other multi-line text. [`text_area`](https://daybrite.dev/api/day_pieces/fn.text_area.html) binds the text to app
state and grows with its content up to a chosen height, then scrolls within the editor.

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

let body = Signal::new(String::new());
text_area(body)
    .placeholder(res::str::write_a_message())
    .min_lines(3) // never shorter than 3 lines
    .max_lines(8) // grows to 8, then scrolls internally
    .id(id!(compose))
```

[`text_area(text)`](https://daybrite.dev/api/day_pieces/fn.text_area.html) binds a `Signal<String>` two-way: keystrokes write the signal, and setting the
signal replaces the editor's text (echo-guarded, so a programmatic set that matches the last typed
value doesn't loop). [`.placeholder(_)`](https://daybrite.dev/api/day_pieces/trait.TextAreaBuilder.html#tymethod.placeholder) sets the empty-state prompt (evaluated once, not reactive).
The height auto-grows with content between [`.min_lines(_)`](https://daybrite.dev/api/day_pieces/trait.TextAreaBuilder.html#tymethod.min_lines) (default 1) and [`.max_lines(_)`](https://daybrite.dev/api/day_pieces/trait.TextAreaBuilder.html#tymethod.max_lines) (default
`0` = unbounded, never scrolls); a non-zero max is floored to min. `TextArea` implements `Piece`, so
[`.id()`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.id)/[`.a11y()`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.a11y)/[`.frame()`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.frame) chain via `Decorate`.

### Attributes

Three attributes control the native editor, each a reactive `bool` (a constant or a signal/closure)
that updates live:

```rust
text_area(report)
    .editable(false)      // read-only (default true)
    .selectable(true)     // still copyable (default true)
    .spellcheck(false)    // no spell-correction squiggles (default true)
```

- **[`.editable(v)`](https://daybrite.dev/api/day_pieces/trait.TextAreaBuilder.html#tymethod.editable)** — `false` makes the editor read-only.
- **[`.selectable(v)`](https://daybrite.dev/api/day_pieces/trait.TextAreaBuilder.html#tymethod.selectable)** — whether the text can be selected and copied (useful with [`.editable(false)`](https://daybrite.dev/api/day_pieces/trait.TextAreaBuilder.html#tymethod.editable)
  for a read-only-but-copyable display).
- **[`.spellcheck(v)`](https://daybrite.dev/api/day_pieces/trait.TextAreaBuilder.html#tymethod.spellcheck)** — the spell-check / autocorrect highlighting.

Native support varies; a toolkit that can't honor an attribute answers the matching capability with
`Support::Unsupported`, so an app can gray out a control that would do nothing. `Emulated` means the
attribute IS honored, just not by one native property, so the test to gray a control is
[`capability(…) == Support::Unsupported`](https://daybrite.dev/api/day_core/fn.capability.html), not `!= Support::Native` (the showcase's Text Areas page
gates its three toggles that way):

| attribute | Cap | AppKit | UIKit | GTK | Qt | Android | XAML | ArkUI |
|---|---|---|---|---|---|---|---|---|
| editable | `Cap::TextEditable` | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | emulated |
| selectable | `Cap::TextSelectable` | ✓ | ✓ | — (always on) | ✓ | ✓ | emulated | emulated |
| spell-check | `Cap::TextSpellCheck` | ✓ | ✓ | — (none built in) | — (none) | ✓ | ✓ | follow-up |

GTK's [`GtkTextView`](https://docs.gtk.org/gtk4/class.TextView.html) is always selectable (no toggle), and neither GTK nor Qt ships a built-in
spell-checker (that needs libspelling/gspell or Hunspell).

**XAML** honors editable and spell-check with the plain [`TextBox`](https://learn.microsoft.com/en-us/uwp/api/windows.ui.xaml.controls.textbox) properties `IsReadOnly` and
`IsSpellCheckEnabled`. Selection is the odd one out: `IsTextSelectionEnabled` is a [`TextBlock`](https://learn.microsoft.com/en-us/uwp/api/windows.ui.xaml.controls.textblock) /
[`RichTextBlock`](https://learn.microsoft.com/en-us/uwp/api/windows.ui.xaml.controls.richtextblock) property and [`TextBox`](https://learn.microsoft.com/en-us/uwp/api/windows.ui.xaml.controls.textbox) carries no equivalent, so [`.selectable(false)`](https://daybrite.dev/api/day_pieces/trait.TextAreaBuilder.html#tymethod.selectable) is EMULATED:
the shim collapses each selection as `SelectionChanged` reports it and suppresses the context menu,
which is the other route to Copy / Select All.

**ArkUI** keeps its native [`ARKUI_NODE_TEXT_AREA`](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-arkui/capi-native-node-h.md) and emulates editable/selectable policy. The
API 18 C interface has no persistent read-only or selection-enable property: `EDITING=false`
only ends the current editing session. Day vetoes native insert/delete callbacks while read-only,
suppresses keyboard activation/autofill, and restores the held text if an input path bypasses the
veto. Rejected changes never reach the binding; programmatic writes update the held value.
Read-only text remains selectable and copyable when [`.selectable(true)`](https://daybrite.dev/api/day_pieces/trait.TextAreaBuilder.html#tymethod.selectable).

With [`.selectable(false)`](https://daybrite.dev/api/day_pieces/trait.TextAreaBuilder.html#tymethod.selectable), Day hides the native selection menu, collapses selection-change events
to a caret, and intercepts Select All/Copy/Cut shortcuts before the IME. Typing remains enabled
when [`.editable(true)`](https://daybrite.dev/api/day_pieces/trait.TextAreaBuilder.html#tymethod.editable). Both policies apply at creation and reactively, without replacing the
editor. They report `Support::Emulated`; spell-check remains `Unsupported`.

It is the multi-line sibling of [`text_field`](https://daybrite.dev/api/day_pieces/fn.text_field.html) ([docs/forms](forms.md)): a field is one line and submits on
Return; a text area keeps newlines. Both raise the soft keyboard through the focus system
([docs/focus](focus.md)), and keyboard avoidance (the focused editor scrolling clear of the keyboard) applies
to both.

### Submission

[`.on_submit(f)`](https://daybrite.dev/api/day_pieces/trait.TextAreaBuilder.html#tymethod.on_submit) turns a text area into a composer: Enter calls `f` with the bound text already
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

## Per-toolkit native realization

Platform guidance: [Apple: text fields](https://developer.apple.com/design/human-interface-guidelines/text-fields); [Android: input methods](https://developer.android.com/develop/ui/views/touch-and-input/creating-input-method); [Windows: text controls](https://learn.microsoft.com/en-us/windows/apps/design/controls/text-controls).

| AppKit | UIKit | GTK | Qt | Android | XAML | ArkUI |
|---|---|---|---|---|---|---|
| [`NSTextView`](https://developer.apple.com/documentation/appkit/nstextview) in [`NSScrollView`](https://developer.apple.com/documentation/appkit/nsscrollview) | [`UITextView`](https://developer.apple.com/documentation/uikit/uitextview) | [`GtkTextView`](https://docs.gtk.org/gtk4/class.TextView.html) in [`GtkScrolledWindow`](https://docs.gtk.org/gtk4/class.ScrolledWindow.html) | [`QPlainTextEdit`](https://doc.qt.io/qt-6/qplaintextedit.html) | multi-line [`EditText`](https://developer.android.com/reference/android/widget/EditText) | wrapping [`TextBox`](https://learn.microsoft.com/en-us/uwp/api/windows.ui.xaml.controls.textbox) | [`ARKUI_NODE_TEXT_AREA`](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-arkui/capi-native-node-h.md) |

Each toolkit keeps the `(min_lines, max_lines)` band and grows its `measure` height in a line band.
Text changes report through `Event::TextChanged(String)`; programmatic sync (`TextAreaPatch::SetText`)
is echo-guarded per toolkit, and the attribute patches (`SetEditable`/`SetSelectable`/`SetSpellCheck`)
apply the native property. The Qt and XAML renderers carry C++ shims in the matching `-sys` crate
(`shim-textarea.cpp`: Qt adds `day_textarea_set_attrs`/`set_read_only`/`set_selectable`; XAML adds
`day_textarea_xaml_set_editable`/`set_selectable`/`set_spellcheck`, applied at build as well as on
patch so an editor that starts read-only comes up that way); Android's
`DayTextArea.java` rides the framework shim (its `applyAttrs` maps editable→InputType/keyListener,
selectable→`setTextIsSelectable`, spell-check→`TYPE_TEXT_FLAG_NO_SUGGESTIONS`).

## Follow-ups

- **ArkUI spell-check**: still unsupported; verify an appropriate SDK/IME contract before
  advertising it. The API 18 C TextArea interface has no spell-check toggle.
- Styled text is a different control: [`day-piece-texteditor`](/docs/reference/texteditor) edits a whole `StyledText` — the
  document labels render and [`.markdown()`](https://daybrite.dev/api/day_spec/styled/struct.StyledText.html#method.markdown) produces — in each platform's rich-text view
  ([docs/texteditor](texteditor.md)). [`text_area`](https://daybrite.dev/api/day_pieces/fn.text_area.html) stays the plain-text one.
- Reactive placeholder; a character/line counter affordance.
