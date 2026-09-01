---
title: "Styled text editor"
description: "day-piece-texteditor: editing the same StyledText that labels render, in each platform's own rich-text view."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Styled text editor

> [!NOTE]
> **Shipped.** `day-piece-texteditor` carries all eight toolkit arms. Six are verified running
> (macos-appkit, macos-gtk, macos-qt, ios-uikit, android-mdc, web-dom; the Showcase walkthrough
> drives the editor on each). **harmony-arkui** builds and stages its ArkTS but has not been driven
> on a device, and **windows-xaml** is written blind against the Windows SDK and compiles only in
> CI, the same standing as [`day-piece-colorpicker`](colorpicker.md)'s XAML arm. Both are called
> out again in §6.

`text_editor(doc)` edits a [`StyledText`](#2-the-document), the same document a label renders and
`.markdown()` produces, live in each platform's own text editor: bold, italic, underline,
strikethrough, relative size, text and highlight color, plus paragraph alignment, indent, spacing
and list markers.

```rust
use day::prelude::*;
use day_piece_texteditor::text_editor;

let doc = Signal::new(StyledText::markdown("**Hello**, _world_.", Font::Body));
let sel = Signal::new(0..0);
let typing = Signal::new(RunStyle::plain(Font::Body));

column((
    // The toolbar is ordinary Day pieces — the piece ships no chrome of its own.
    button("B").action(move || {
        let range = sel.get_untracked();
        let on = !doc.with_untracked(|d| d.style_of(range.clone(), Font::Body)).bold();
        doc.update(|d| d.apply(range.clone(), Font::Body, move |s| s.set_bold(on)));
    }),
    text_editor(doc)
        .selection(sel)          // two-way: caret moves write it, writing it moves the caret
        .typing_style(typing)    // two-way: what the NEXT keystroke will be styled with
        .placeholder("Start typing…")
        .min_lines(8)
        .max_lines(14),
))
```

## 1. Every toolkit already has one

The finding that shaped the piece is that Day targets eight toolkits and all eight ship a styled
text editor, including web-dom. Nothing here is drawn by Day.

| Toolkit | Editor | Model it edits | Offsets it speaks |
|---|---|---|---|
| appkit | `NSTextView` (`setRichText(true)`) in an `NSScrollView` | `NSTextStorage` | UTF-16 |
| uikit | `UITextView` (itself a scroll view) | the same TextKit `NSTextStorage` | UTF-16 |
| gtk | `GtkTextView` in a `GtkScrolledWindow` | `GtkTextBuffer` + interned `GtkTextTag`s | characters |
| qt | `QTextEdit` (not the `QPlainTextEdit` `text_area` uses) | `QTextDocument` through a `QTextCursor` | UTF-16 (QChar) |
| mdc | an `EditText` subclass | its live `SpannableStringBuilder` | UTF-16 (Java `char`) |
| xaml | `RichEditBox` | RichEdit's `ITextDocument` (TOM) | UTF-16 |
| arkui | the ArkTS `RichEditor` | `RichEditorController` spans | UTF-16 (ArkTS string index) |
| dom | a `contenteditable` element | the DOM subtree, flattened by the shim | bytes, converted in JS |

The offset column matters most: a range is a **byte** range into a Rust `String`
everywhere in Day, and four different index units meet it at the boundary. The conversions
(`utf16_range`, `byte_of_utf16`, `char_range`, `byte_of_char`) live in the piece's `lib.rs` and are
tested once, on astral text, because an off-by-N there styles the wrong words and only in text
that has an emoji or a CJK character in it.

## 2. The document

[`StyledText`](../crates/day-spec/src/styled.rs) is in **day-spec**, not in the piece: a label
renders it, `.markdown()` produces it, the Markdown / HTML / RTF codecs read and write it, and the
editor edits it.

```rust
pub struct StyledText {
    pub text: String,
    pub runs: Vec<TextRun>,             // character attributes, byte ranges, sorted, disjoint
    pub paragraphs: Vec<ParagraphRun>,  // paragraph attributes, aligned to line boundaries
}
```

`TextRun` carries `font: FontSpec`, `color`, `background`, `underline: Underline`, `strikethrough`
and `link`. `ParagraphRun` carries `align`, `indent`, `list`, `list_level`, `space_before` and
`space_after`. Paragraph attributes are kept out of `TextRun`, because a paragraph attribute
applies to whole paragraphs and folding it into a character run would make invalid states
representable.

Everything a toolbar needs is a pure function over that document. It runs in Rust with no
backend duty behind it, so it is identical on every target and testable on the headless one:

```rust
/// The style across a range: where runs disagree, the differing attribute reads as OFF, which is
/// what a toolbar renders as a mixed state.
pub fn style_of(&self, range: Range<usize>, base: Font) -> RunStyle;
/// Apply a change to every run overlapping `range`, splitting runs at the boundaries.
pub fn apply(&mut self, range: Range<usize>, base: Font, f: impl Fn(&mut RunStyle));
/// The same, for the paragraphs a range touches.
pub fn apply_paragraph(&mut self, range: Range<usize>, f: impl Fn(&mut ParagraphStyle));
/// Reflow every run over an edit at `offset` that removed and inserted bytes.
pub fn reflow(&mut self, offset: usize, removed: usize, inserted: usize);
```

### Relative size

`FontSpec::scale` multiplies the semantic style's size, and is what an editor's size control moves.
`Font::System(pt)` stays the absolute form; it is where an imported document's `\fs28` or
`font-size: 14px` lands. The difference matters for accessibility: a scaled run still tracks the
reader's text-size setting, and an absolute one does not, which is why the Showcase's toolbar moves
`scale` and never `Font::System`.

### Import and export

`StyledText::{markdown, html, rtf}` parse; `to_markdown`, `to_html`, `to_rtf` write. All six are
lossy in their own ways, and [styled_codec.rs](../crates/day-spec/src/styled_codec.rs) states each
loss next to the code that causes it. The largest losses are that Markdown has no syntax for
color, background or size, so those runs export as plain text; that a heading scale that is not
one of the six levels exports as `**bold**`; and that the RTF reader is a **round-trip subset**
(what Day writes, plus the control words a word processor's plain paragraph uses) rather than a
full RTF implementation.

## 3. Who owns the attributes

**Day owns the attributes.** The native view owns the characters (typing, deletion, IME
composition, undo, autocorrect, dictation) and reports them as `Event::TextChanged`. The piece
diffs that text against the text it last knew, reflows its runs over the edit, and writes the
bound signal.
Attributes only ever travel Day → native.

That is why every arm turns the platform's own formatting UI **off**: iOS's
`allowsEditingTextAttributes`, the macOS font panel, Qt's and RichEditBox's built-in
Ctrl+B/I/U. An editor whose attributes can change from two directions has to reconcile them, and
reconciling an attributed string across eight toolkits is a much larger promise than this piece
makes. A toolbar goes through the bound signal instead, in Rust, where it behaves identically on
all nine targets.

The cost is that there is no attribute read-back. Pasting styled text keeps its
characters and takes the surrounding style ("paste and match style", which is the only paste this
model can describe), and the platform's own bold shortcut does nothing. The per-toolkit hook a
future read-back would use is `NSTextStorageDelegate`'s `editedAttributes` mask, GTK's
`apply-tag`/`remove-tag`, `QTextDocument::contentsChanged`, an Android `SpanWatcher`,
`ITextRange::Expand(tomCharFormat)`, `RichEditorController::getSpans`, and a DOM
`MutationObserver`.

### Recovering the edit

A native view reports its whole text and nothing else. The piece recovers the delta with a common
prefix / common suffix diff (`diff_edit`), pulled back to character boundaries:

- one keystroke, one deletion, one paste or one autocorrect **is** exactly that span;
- an edit that touched two separate places comes back as one span covering both, which is
  coarser but never wrong, and costs only the styling between them.

`reflow` then moves the runs: ranges before the edit are untouched, ranges after shift, a range
containing the edit grows or shrinks, and an emptied range is dropped. It is O(runs) rather than
O(document), and needs no backend cooperation, which lets one code path serve all eight arms.

### Two patches, and why

```rust
pub enum EditorPatch {
    SetDocument(StyledText),     // the text changed under the app's hand: replace it
    SetAttributes(StyledText),   // the text is what the view already holds: restyle only
    SetSelection(Range<usize>),
    SetTypingStyle(RunStyle),
    SetEditable(bool),
}
```

`SetAttributes` is the syntax-highlighting path, and the reason a live highlighter is usable:
re-tokenizing produces fresh runs for the same characters on every keystroke, and pushing those as
a document would send the caret back to wherever the app last put it, and clear the undo stack
with it. Every
arm applies attributes over the text the view already holds, inside one begin/end editing batch,
restoring the selection around the write.

It carries the whole document even though only the attributes are new: the text is what an arm
converts byte ranges against, and the four whose toolkit cannot hand back its own string cheaply
(Qt, Android, HarmonyOS, XAML) would otherwise each need a cache that a keystroke could leave one
edit stale.

### The typing style

The typing style is the one piece of editor state an app cannot derive: with a collapsed caret
there is no text to read a style off, and "what happens if I type now" is the platform's own
pending state.

It is bound two-way. Day writes it whenever the selection moves (so a toolbar shows the style of
the text the caret sits in); an app writes it to make the *next* word bold. Both directions are
guarded against echo: a selection the view reported is never patched back into it, and an
unchanged typing style is not re-sent. A `selectionchange` fires on every mouse-move of a drag, so
without those guards the web arm re-anchored the selection hundreds of times a second and a mouse
drag could not select anything at all. Three toolkits have the concept natively (Qt's
`setCurrentCharFormat`, TOM's collapsed-selection `CharacterFormat`, ArkUI's `setTypingStyle`),
and the Apple arms have `typingAttributes`. GTK and the web have nothing of the kind.

That split does not matter, because the piece **also applies the pending style in its own model**:
when the next `TextChanged` arrives with inserted characters, the style is applied to exactly those
bytes. Without that, the typing style would be cosmetic for a single frame everywhere — the native
view would style the keystroke, and Day's next attribute patch would repaint it from a model that
never heard about the style. The native call remains as the frame-one appearance.

## 4. What each arm does

The shape is the same everywhere (realize an editor, seed it, patch it, report the two events), so
what follows is what differs, and what each platform cannot represent.

**appkit.** `NSTextView` in an `NSScrollView`. `setUsesFontPanel(false)`, `setImportsGraphics(false)`,
smart quotes and dashes off (a substitution is an edit Day never asked for, in a document an app may
be re-parsing). Paragraph attributes are one `NSMutableParagraphStyle` per paragraph run. The
empty-state prompt is an `NSTextField` subview, since `NSTextView` has no placeholder.

**uikit.** The same TextKit model through `UITextView`, so the two arms differ only in class names.
`allowsEditingTextAttributes = false` is the iOS-only line: it keeps the selection's edit menu
from offering B/I/U. Italic is a descriptor trait, asked for on top of the traits the font
already has, so bold+italic stays bold.

**gtk.** GTK uses tags rather than attributes. Each distinct `RunStyle` becomes one interned
`GtkTextTag`, keyed by a canonical string and reused for the buffer's life; a fresh tag per run per
keystroke would grow the tag table without bound under a live highlighter. Offsets are characters.
The losses are that Pango has no dotted underline (it draws a single rule) and that its wavy
underline is the spell-check squiggle (`Underline::Error`). Paragraph attributes ride the same
tags, with the marker's hanging indent spelled as a negative first-line offset against a wider
left margin, the inverse of Apple's spelling of the same layout.

**qt.** `QTextEdit`, driven through a `QTextCursor` in this crate's own shim
(`src/lib-qt-shim.cpp`). The shim avoids `setHtml`, which replaces the document, moving the caret
and clearing undo. `beginEditBlock`/`endEditBlock` collapse a sweep into one undo step and one
relayout. Qt draws no double underline (it degrades to single). Monospace comes from
`QFontDatabase::systemFont(FixedFont)`, because Qt's rich text does not resolve the generic
`monospace` family from a char format; the label path hit the same limit.

**mdc.** An `EditText` subclass over its live `Editable`, with the piece's own Java
(`android/java/…/DayTextEditor.java`) staged by `[package.metadata.day.android]`. Attributes are
applied to the buffer the user is typing in, removing only the span classes this file sets, so the
IME's composing spans and the framework's selection spans survive (removing those cancels a
half-typed Japanese or Korean word). Runs cross as flat parallel int arrays, the shape
`DayBridge.setLabelRuns` already uses, with the same flag bits. The subclass exists for one reason:
`onSelectionChanged` is a protected method with no listener form. The losses are that Android has
one underline span (dotted and wavy draw a plain rule), no per-paragraph justification, and no
paragraph-spacing span.

**arkui.** The ArkUI **C** node API has no rich editor (`native_node.h` stops at
`ARKUI_NODE_TEXT_AREA`), so this arm ships its own ArkTS (`ohos/ets/Index.ets`), staged into the
app's hvigor project by `[package.metadata.day.ohos]`. `RichEditorController` is the best-shaped
controller of the eight: `updateSpanStyle` restyles without touching characters, `setTypingStyle`
is native, and `setSelection` speaks the same offsets. The whole channel is strings (one props
string at realize, `(cmd, arg)` pairs after), and reports come back through `pieceEvent` as
`Event::Custom`, which is why the text report rides a `"txt "` prefix rather than
`Event::TextChanged`. The loss is that ArkUI carries one decoration per span, so an underlined
strikethrough keeps the strikethrough.

**dom.** A `contenteditable` element, since a `<textarea>` is plain text by definition.
Contenteditable is the browser's rich text editing, and brings IME, undo, spell-check,
drag-and-drop and the accessibility tree with it. It has no fixed document model, though: Enter
inserts a `<div>` in one browser and a `<p>` in another, and a paste arrives as whatever markup it
was copied from. Day writes the DOM back in one canonical shape — the same `styled_to_html` an
export produces — and reads it through the exact inverse of that shape.
`document.execCommand` is not used anywhere: it is deprecated, differs per browser, and inserts
markup Day would then have to normalize away.

This is also the only arm that must **rebuild its view to restyle**, since markup is its only
attribute channel, so it is the only one that has to put the selection back by offset afterwards,
and the only one where the DOM ⇄ text mapping is code rather than a native index. Two rules in
`shim.js` keep that mapping consistent.

- **One traversal.** The flattening (`dayEditorText`), a DOM position's byte offset
  (`dayEditorOffset`) and a byte offset's DOM position (`dayEditorLocate`) all go through
  `dayEditorScan`. They are inverses, and one newline of disagreement shifts a restored selection
  by one character per line above it, which the user would see as the selection moving after
  pressing Bold.
- **The mapping is the serializer's, read backwards.** Day writes one block per line, so the text
  is the blocks' text joined with `\n`: an empty block is an empty line and still contributes its
  separator. Collapsing consecutive empty blocks instead made the first keystroke report a text
  with every blank line missing, which Day then read as a deletion and reflowed the paragraph runs
  onto the wrong lines.

A rewrite during IME composition is skipped outright, and the selection is never re-anchored while
a pointer drag is in progress, because `removeAllRanges` mid-drag collapses what the user is
selecting.

**xaml (unverified).** `RichEditBox` through its TOM `ITextDocument`. `GetRange(start, end)` takes
UTF-16 positions, assigning an `ITextCharacterFormat` to a range applies it in one call, and a
collapsed `Document.Selection`'s format is the typing style. `BatchDisplayUpdates` /
`ApplyDisplayUpdates` bracket a sweep. This is the one arm whose underline vocabulary is complete:
single, double, dotted and wave all exist.

It is also the one arm whose text does not arrive in Day's spelling: RichEdit's paragraph mark is a
CR and its Shift+Enter line break a VT, where the other seven report LF. The shim rewrites both to
LF on the way out, one code unit for one (never collapsing a pair), because that string's offsets
are the ones the selection and every attribute range are expressed in. An arm whose control does
the same owes the same rewrite: an app that splits its document on `\n` finds nothing otherwise,
and only on that platform.

## 5. What a toolkit with no styled editor gets, and what it must not do

None of the eight is in this position; the mock backend is, and an external toolkit
([extending.md](extending.md)) could be. The tier for those is **plain text**: realize the ordinary
`text_area`, drop the runs, report `Support::Emulated`, and keep the document's *text*
round-tripping so nothing is lost but the styling.

**Do not compose one.** This is the opposite call from
[`day-piece-colorpicker`](colorpicker.md), where drawing the whole picker on a canvas was the right
answer, and the reasoning does not transfer. A color picker
is a few shapes and a press location. A text editor is IME composition for every language that
needs one, bidirectional text and cursor movement, grapheme-cluster-aware selection, the platform's
undo stack, autocorrect and spell-check, dictation, drag-and-drop, the system edit menu,
loupe/handle selection on touch, and a full accessibility tree that a screen reader can navigate by
character, word and line. A canvas with a blinking rectangle gets all of those wrong, and the
failures show up in the locales and for the users least able to work around them. Every platform
ships this control because it is not reasonable to write.

## 6. What is verified, and where

| Target | Built | Driven | Notes |
|---|---|---|---|
| macos-appkit | ✅ | ✅ | the model the others are compared against |
| ios-uikit | ✅ | ✅ | simulator |
| macos-gtk | ✅ | ✅ | |
| macos-qt | ✅ | ✅ | |
| android-mdc | ✅ | ✅ | emulator |
| web-dom | ✅ | ✅ | headless WebKit through the walkthrough |
| harmony-arkui | ✅ | ❌ | the Rust arm compiles and the ArkTS stages; not driven on a device |
| windows-xaml | CI | ❌ | written blind against the Windows SDK |

A check on Windows has to confirm: that `RichEditBox` seeds and restyles without the caret moving,
that `SelectionChanged` reports the range the piece expects, that the swallowed Ctrl+B/I/U really do
nothing, and that the placeholder shows on an empty document. A check on HarmonyOS has to confirm
the same list plus that `onDidChange` fires for every committed edit including an IME commit.

## 7. What the piece needed outside itself

1. **day-spec** — `StyledText`, `RunStyle`, `ParagraphRun`, `Underline`, `TextRun::{background,
   underline}`, `FontSpec::scale`, and the Markdown / HTML / RTF codecs. `markdown.rs` moved here
   from day-pieces so the parser and the document live together.
2. **Eight label paths** got the two new run attributes and the scale, next to the code that
   already mapped `color` and `strikethrough`.
3. **Public font resolution per backend** — `day_gtk::{gtk_style, pango_weight}`,
   `day_qt::qt_style`, `day_android::font_style`, `day_arkui::font_vp`, `Uikit::mtm`,
   `day_uikit::set_text_input_trait`. A piece has to resolve the same scale its labels do.
4. **day-dom** grew the editor entry points: `Dom::set_html` (caret-preserving),
   `Dom::set_editor_selection`, the `listen::EDITABLE` bit, and event kind 17 (the piece channel,
   as the Android and ArkTS bridges already spell it).
5. **day-mock** grew `MockProbe::describe_patch`, so a standalone piece's own patch type shows up in
   the op log instead of `?`. Without it, a test cannot assert that a write patched attributes
   without replacing the document.
6. **`dom_renderer!`** grew a `release:` arm, matching `renderer!`.
7. **day.css** grew the contenteditable rules: the empty-state prompt (a `::before` on the first
   block, since a contenteditable is never `:empty`) and block margins (the browser's document
   defaults double-space an editor).

## 8. The Showcase's Text areas page

`Day-Showcase/src/pages/text_areas.rs` leads with the styled editor and keeps the plain `text_area`
below it, which remains the right control for a chat composer or a commit message.

- **A formatting toolbar**: B / I / U / S, relative size up and down, a highlight toggle, a
  [`color_picker`](colorpicker.md) driving the selection's text color, and an alignment picker.
  Every one of them reads `style_of` and writes `apply`; with a collapsed caret they write the
  typing style instead.
- **Three documents**: a formatted note parsed from Markdown, a Rust sample the page re-tokenizes on
  every keystroke and pushes back as fresh runs, and an empty one that shows the placeholder.
- **A selection inspector** — the range and the style under it, read straight out of the document,
  which is the surface a walkthrough asserts against on every backend without a screenshot.
- **Import and export** — Markdown, HTML and RTF out, and a Read back button that parses the result
  into the document again.

The walkthrough drives all of it: the toolbar, the document picker, the export buttons and the
round trip, on every target in the matrix.
