---
navTitle: "Text editors"
title: "Styled text editor"
description: "day-piece-text-editor: editing the same StyledText that labels render, in each platform's rich-text view."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Styled text editor

Let people write and format rich text in the platform’s native editor.
[`text_editor(doc)`](https://daybrite.dev/api/day_piece_text_editor/fn.text_editor.html) edits a [`StyledText`](#document-and-selection), the same document a label renders and
[`.markdown()`](https://daybrite.dev/api/day_spec/styled/struct.StyledText.html#method.markdown) produces, live in each platform's text editor: bold, italic, underline,
strikethrough, relative size, text and highlight color, plus paragraph alignment, indent, spacing
and list markers.

## Add to your app

Add [`day-piece-text-editor`](https://github.com/daybrite/day/tree/main/crates/day-piece-text-editor) to your app’s Cargo dependencies, using the same Day version as the rest of your app:

```toml
[dependencies]
day-piece-text-editor = "0.6"
```

`day build -t <target>` enables this direct dependency’s matching toolkit feature
automatically. No dependency entry is needed in `Day.toml`.

| Platforms / toolkits | Availability |
| --- | --- |
| AppKit, UIKit, GTK, Qt, Android, Web | Native or browser rich-text editing. |
| WinUI, ArkUI | Editor implementation is present; selection and editing limitations below remain unverified. |

## Authoring

```rust
use day::prelude::*;
use day_piece_text_editor::text_editor;

// Synthetic editor content, representing a document supplied by the user.
let doc = Signal::new(StyledText::markdown("**Hello**, _world_.", Font::Body));
let sel = Signal::new(0..0);
let typing = Signal::new(RunStyle::plain(Font::Body));

column((
    // Add the formatting actions the app needs.
    button(res::str::bold()).action(move || {
        let range = sel.get_untracked();
        let on = !doc
            .with_untracked(|d| d.style_of(range.clone(), Font::Body))
            .bold();
        if range.is_empty() {
            typing.update(|style| style.set_bold(!style.bold()));
        } else {
            doc.update(|d| d.apply(range.clone(), Font::Body, move |s| s.set_bold(on)));
        }
    }),
    text_editor(doc)
        .selection(sel) // two-way: caret moves write it, writing it moves the caret
        .typing_style(typing) // style for newly typed characters
        .placeholder(res::str::start_typing())
        .min_lines(8)
        .max_lines(14),
))
```

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](/docs/reference/localization#reference-example-messages) and [catalog setup](/docs/how-to/localization).

## Document and selection

[`StyledText`](https://daybrite.dev/api/day_spec/styled/struct.StyledText.html) contains the text,
character runs, and paragraph runs. Character styles include font, color, background,
underline, strikethrough, and links. Paragraph styles include alignment, indentation, list
markers, and spacing. Labels can render the same document; see [Styled text](/docs/reference/text-runs).

Selections are byte ranges into the Rust string, including on platforms that use other index
units internally. A collapsed range is a caret. Bind the range with
[`.selection(binding)`](https://daybrite.dev/api/day_piece_text_editor/trait.TextEditorBuilder.html#tymethod.selection)
to observe or move it. Keep programmatic ranges on valid UTF-8 boundaries.

Use [`style_of`](https://daybrite.dev/api/day_spec/styled/struct.StyledText.html#method.style_of)
to inspect the selected style and
[`apply`](https://daybrite.dev/api/day_spec/styled/struct.StyledText.html#method.apply) to change
character styles. [`apply_paragraph`](https://daybrite.dev/api/day_spec/styled/struct.StyledText.html#method.apply_paragraph)
changes the paragraphs touched by a range. For an empty selection, update the
[`.typing_style(binding)`](https://daybrite.dev/api/day_piece_text_editor/trait.TextEditorBuilder.html#tymethod.typing_style)
binding to style the next characters the person types.

`FontSpec::scale` scales a semantic font while retaining its response to text-size settings.
`Font::System(points)` specifies an absolute size. The editor's
[`.base(font)`](https://daybrite.dev/api/day_piece_text_editor/trait.TextEditorBuilder.html#tymethod.base)
defaults to `Font::Body`.

## Editing and formatting

Native editing updates the bound document. Changes to its styles restyle the text; an app
replacement of the document updates the editor. Formatting actions should write the document
or typing-style binding. The editor supplies no formatting toolbar.

Pasting styled text keeps the characters and uses the surrounding style. Native formatting
controls are disabled where supported because formatting is owned by the bound document.
Use app commands for bold, italic, and other formatting, and check the platform limits below.

[`.editable(false)`](https://daybrite.dev/api/day_piece_text_editor/trait.TextEditorBuilder.html#tymethod.editable)
makes the editor read-only. Use
[`.placeholder(text)`](https://daybrite.dev/api/day_piece_text_editor/trait.TextEditorBuilder.html#tymethod.placeholder)
for a localized empty-state prompt and
[`.min_lines(n)`](https://daybrite.dev/api/day_piece_text_editor/trait.TextEditorBuilder.html#tymethod.min_lines) /
[`.max_lines(n)`](https://daybrite.dev/api/day_piece_text_editor/trait.TextEditorBuilder.html#tymethod.max_lines)
to control its height.

## Import and export

`StyledText` provides
[`markdown`](https://daybrite.dev/api/day_spec/styled/struct.StyledText.html#method.markdown),
[`html`](https://daybrite.dev/api/day_spec/styled/struct.StyledText.html#method.html), and
[`rtf`](https://daybrite.dev/api/day_spec/styled/struct.StyledText.html#method.rtf) parsers, with
[`to_markdown`](https://daybrite.dev/api/day_spec/styled/struct.StyledText.html#method.to_markdown),
[`to_html`](https://daybrite.dev/api/day_spec/styled/struct.StyledText.html#method.to_html), and
[`to_rtf`](https://daybrite.dev/api/day_spec/styled/struct.StyledText.html#method.to_rtf) exporters.

These conversions are lossy. Markdown cannot retain arbitrary color, background, or font size;
a heading scale outside its six levels exports as bold. The RTF reader handles Day's output
and common paragraph control words, rather than the full RTF format. Keep the original
user document when an import/export round trip must preserve unsupported formatting.

## Code and single-line editors

[`text_editor_text(binding)`](https://daybrite.dev/api/day_piece_text_editor/fn.text_editor_text.html)
edits a string binding when the app does not store a styled document.
[`.highlight(f)`](https://daybrite.dev/api/day_piece_text_editor/trait.TextEditorBuilder.html#tymethod.highlight)
derives text runs after edits and when signals read by the highlighter change. The crate's
[`highlighter`](https://daybrite.dev/api/day_piece_text_editor/highlight/fn.highlighter.html)
supports JSON with comments, XML, HTML, GraphQL, Rust, and plain text.
[`Language::for_mime`](https://daybrite.dev/api/day_piece_text_editor/highlight/enum.Language.html#method.for_mime)
selects a language from a content type. Supply a palette appropriate for the editor's appearance.

[`.single_line()`](https://daybrite.dev/api/day_piece_text_editor/trait.TextEditorBuilder.html#tymethod.single_line)
disables wrapping, scrolls horizontally, and converts pasted newlines to spaces. Enter invokes
[`.on_submit(f)`](https://daybrite.dev/api/day_piece_text_editor/trait.TextEditorBuilder.html#tymethod.on_submit)
without inserting a newline.

[`.code()`](https://daybrite.dev/api/day_piece_text_editor/trait.TextEditorBuilder.html#tymethod.code)
adds indentation and paired bracket/quote insertion. Tab inserts spaces to the next stop;
[`.indent(n)`](https://daybrite.dev/api/day_piece_text_editor/trait.TextEditorBuilder.html#tymethod.indent)
sets its width, defaulting to two. Enter carries forward indentation and adds a level after
an opening bracket. Code mode disables spell-check and automatic prose substitutions where
the toolkit supports them. Use
[`.no_spellcheck()`](https://daybrite.dev/api/day_piece_text_editor/trait.TextEditorBuilder.html#tymethod.no_spellcheck)
when only spell-check should be disabled.

## Toolkit implementations

| Toolkit | Editor | Formatting limits |
| --- | --- | --- |
| AppKit | [`NSTextView`](https://developer.apple.com/documentation/appkit/nstextview) | Uses TextKit character and paragraph styles. |
| UIKit | [`UITextView`](https://developer.apple.com/documentation/uikit/uitextview) | Uses TextKit character and paragraph styles. |
| GTK | [`GtkTextView`](https://docs.gtk.org/gtk4/class.TextView.html) | Dotted underline becomes a single rule; wavy underline uses the error underline. |
| Qt | [`QTextEdit`](https://doc.qt.io/qt-6/qtextedit.html) | Double underline becomes a single rule. |
| Android | [`EditText`](https://developer.android.com/reference/android/widget/EditText) | Dotted and wavy underlines become a single rule; paragraph justification and paragraph spacing are unavailable. |
| WinUI | [`RichEditBox`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.richeditbox) | See the native editing validation limits below. |
| ArkUI | [`RichEditor`](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-arkui/arkui-ts/ts-basic-components-richeditor.md) | One decoration per span; strikethrough takes precedence over underline. |
| Web | Browser `contenteditable` | Browser editing and selection behavior applies. |

Toolkits without a styled editor, including the mock toolkit, fall back to plain text and
report `Support::Emulated`. The fallback preserves text but does not display its formatting.

> [!WARNING]
> **Windows / WinUI: native editing.**
> Caret stability during restyling, selection reporting, suppression of Ctrl+B/I/U, and
> empty-document placeholders remain unverified. Test these interactions in your editor.

> [!WARNING]
> **HarmonyOS / ArkUI: native editing.**
> Device behavior remains unverified, including selection, restyling, placeholders, and
> delivery of committed edits from input methods. Test the languages your app supports.

## Platform guidance

- [Apple: text fields](https://developer.apple.com/design/human-interface-guidelines/text-fields)
- [Android: input methods](https://developer.android.com/develop/ui/views/touch-and-input/creating-input-method)
- [Windows: text controls](https://learn.microsoft.com/en-us/windows/apps/design/controls/text-controls)
