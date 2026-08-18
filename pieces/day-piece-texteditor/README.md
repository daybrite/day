<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# day-piece-texteditor

A styled-text editor — bold, italic, underline, strikethrough, size, text and highlight
color, plus paragraph alignment, indent, spacing and list markers — bound two-way to a
`Signal<StyledText>`.

`text_editor(doc)` is the platform's own rich-text view: `NSTextView` on macOS,
`UITextView` on iOS, `GtkTextView` over a tag table, `QTextEdit`, an Android `EditText`
over its live `SpannableStringBuilder`, a XAML `RichEditBox` driven through its Text
Object Model, the ArkTS `RichEditor` on HarmonyOS, and a `contenteditable` element on the
web. There is no drawn fallback and there must not be one: IME composition, bidirectional
cursor movement, the platform's undo stack, dictation, autocorrect, touch selection
handles and the accessibility tree all come from the real control, and all of them break
invisibly in a canvas — for the users least able to work around it.

The document is `StyledText`, the same type Day's labels render and `.markdown()`
produces, so a toolbar is ordinary Rust over the bound signal:

```rust
let doc = Signal::new(StyledText::markdown("**Hello**, _world_.", Font::Body));
let sel = Signal::new(0..0);

column((
    button("B").action(move || doc.update(|d| {
        let on = !d.style_of(sel.get_untracked(), Font::Body).bold();
        d.apply(sel.get_untracked(), Font::Body, move |s| s.set_bold(on));
    })),
    text_editor(doc).selection(sel).min_lines(8),
))
```

Querying the selection's style, applying one, splitting runs at the boundaries and reading
a mixed state back for an indeterminate toolbar button are all pure functions on the
document — no controller, no round trip into the toolkit, identical on every target, and
testable on the headless one. `StyledText` also imports and exports Markdown, HTML and
RTF, so "open", "save" and "paste as Markdown" are one call each.

See `docs/texteditor.md` in the Day repository for the per-toolkit table, the attribute
ownership rule the piece is built on, and what each platform cannot represent.

Pieces are Day's reusable UI components, shipped as ordinary crates: one Rust API in
front, a real native control per platform behind it. Enable the backends you build for
with cargo features, and `day build` wires up the native side automatically.

## Part of Day

This crate is one piece of [Day](https://daybrite.dev), a Rust framework for building apps
out of each platform's real native widgets — AppKit, UIKit, Android's Material widgets,
GTK 4, Qt 6, XAML, and ArkUI — from one codebase. There is no web view and no bundled
rendering engine: when you write `button("Save")`, macOS shows an `NSButton` and Android
shows a Material button.

New to Day? Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
