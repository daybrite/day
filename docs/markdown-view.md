---
navTitle: "Markdown view"
title: "Markdown view"
description: "Show a Markdown document that sizes to its content: headings, lists, quotes, code blocks, and streamed text that re-parses one block at a time."
---

<!-- Copyright © The Daybrite Project -->
<!-- SPDX-License-Identifier: MPL-2.0 -->

# Markdown view

[`markdown(text)`](https://docs.rs/day-pieces/latest/day_pieces/fn.markdown.html) shows a Markdown document: headings, paragraphs with inline styles and
links, bullet and numbered lists, block quotes, fenced code blocks and rules. It takes the
width it is given and the height its content needs. It does not scroll and cannot be edited,
so it fits in a chat bubble, a release note, a help page, or inside a [`scroll`](./scroll.md)
when the text can be long.

```rust
use day::prelude::*;

markdown(res::str::release_notes())

// A reply that arrives a token at a time.
let reply = Signal::new(String::new());
markdown(reply)
    .selectable_text()
    .code_accessory(move |code| {
        button(res::str::copy())
            .icon(Symbol::Copy)
            .icon_only()
            .plain()
            .action(move || {
                day_part_clipboard::set_text(&code.text());
            })
    })
```

The text is a literal, a localized message, a `Signal<String>` or a closure. For one styled
paragraph inside other text, [`label`](https://docs.rs/day-pieces/latest/day_pieces/fn.label.html)`(t)`[`.markdown()`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.markdown) is lighter
([text-runs](./text-runs.md#markdown)). To edit styled text, use the
[text editor](./texteditor.md).

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](/docs/reference/localization#reference-example-messages) and [catalog setup](/docs/how-to/localization).

## The grammar

| Markdown | Result |
| --- | --- |
| a line of text, blank-line separated | a paragraph; a line break inside it is a space |
| a line ending in two spaces or `\` | a line break inside the paragraph |
| `# Heading` to `###### Heading` | a heading, levels 1 to 6 |
| `- item`, `* item`, `+ item` | a bullet list item |
| `1. item`, `1) item` | a numbered item; the list counts up from its first number |
| an item indented under another | a nested list |
| a paragraph indented under an item | a second paragraph of that item |
| `> text` | a block quote; quotes nest, and hold lists and code |
| three or more backticks or tildes, an optional language, the code, the same fence | a code block |
| `---`, `***`, `___` | a rule |

Inside a paragraph, a heading or a list item the [inline grammar](./text-runs.md#the-grammar)
applies: bold, italic, inline code, strikethrough, links and backslash escapes. Code blocks
are literal.

A code fence that has not been closed yet runs to the end of the text, so code shows while it
is still arriving. Tables, setext headings (a line of `===` under a title), indented code
blocks, HTML blocks, images and reference-style links are read as paragraph text.

## Options

| Builder | Effect |
| --- | --- |
| [`.font(font)`](https://docs.rs/day-pieces/latest/day_pieces/trait.MarkdownBuilder.html#tymethod.font) | the font of paragraphs, list items, quotes and code (default `Font::Body`) |
| [`.color(c)`](https://docs.rs/day-pieces/latest/day_pieces/trait.MarkdownBuilder.html#tymethod.color) | the text color of every block: a constant, a signal or a closure |
| [`.spacing(s)`](https://docs.rs/day-pieces/latest/day_pieces/trait.MarkdownBuilder.html#tymethod.spacing) | the gap between blocks, a `Space` step or points (default `Space::M`) |
| [`.selectable_text()`](https://docs.rs/day-pieces/latest/day_pieces/trait.MarkdownBuilder.html#tymethod.selectable_text) | let the reader select and copy each block's text |
| [`.on_link(f)`](https://docs.rs/day-pieces/latest/day_pieces/trait.MarkdownBuilder.html#tymethod.on_link) | handle a tapped link yourself |
| [`.code_accessory(f)`](https://docs.rs/day-pieces/latest/day_pieces/trait.MarkdownBuilder.html#tymethod.code_accessory) | a piece in every code block's header |
| [`.block_ids(f)`](https://docs.rs/day-pieces/latest/day_pieces/trait.MarkdownBuilder.html#tymethod.block_ids) | an [element id](./element-ids.md) for each block's text |

Headings take the platform's title styles (`Title`, `Title2`, `Title3`, `Headline`,
`Subheadline`, `Footnote`), so they follow the type ramp and the reader's text size like every
other label, and each announces itself to a screen reader as a heading of its level.

### Links

A link opens through [`open_link`](https://docs.rs/day-pieces/latest/day_pieces/fn.open_link.html), the same policy as a [`link`](./text.md#links) and a Markdown
label: a `#route` target navigates in the app and any other target opens in the platform's
default handler. [`.on_link(f)`](https://docs.rs/day-pieces/latest/day_pieces/trait.MarkdownBuilder.html#tymethod.on_link) replaces that for the whole view. Activation is
`Cap::TextLinks` ([text-runs](./text-runs.md#links)).

### Code blocks

A code block draws its code in the platform's monospaced face on the platform's card surface.
Long lines wrap. When the fence names a language, the block's header shows it.

[`.code_accessory(f)`](https://docs.rs/day-pieces/latest/day_pieces/trait.MarkdownBuilder.html#tymethod.code_accessory) puts a piece of yours at the trailing edge of that header: a copy button,
a "Run" button, a badge. The closure runs once per code block and gets a [`MarkdownCode`](https://docs.rs/day-pieces/latest/day_pieces/struct.MarkdownCode.html).
Its [`text()`](https://docs.rs/day-pieces/latest/day_pieces/struct.MarkdownCode.html#method.text) and [`language()`](https://docs.rs/day-pieces/latest/day_pieces/struct.MarkdownCode.html#method.language) read the block as it is at the moment they are called, so an
action copies the whole code even when it was still streaming in when the button was built.

Copying is the app's to do, because the clipboard is a [part](./clipboard.md) the app chooses
to link:

```rust
markdown(reply).code_accessory(|code| {
    button(res::str::copy())
        .icon(Symbol::Copy)
        .icon_only()
        .plain()
        .action(move || {
            day_part_clipboard::set_text(&code.text());
        })
})
```

### Selection

[`.selectable_text()`](https://docs.rs/day-pieces/latest/day_pieces/trait.MarkdownBuilder.html#tymethod.selectable_text) makes each block selectable. A selection stays inside one block, because
each block is its own native label ([selectable text](./text.md#selectable-text)).

## Streamed text

A reply from a language model arrives a few characters at a time, and each arrival changes
the text. The view does two things so that the cost of one arrival does not grow with the
length of the reply.

It parses only the block being written. [`MarkdownDocument::update`](https://docs.rs/day-spec/latest/day_spec/markdown/struct.MarkdownDocument.html#method.update) finds the first byte that
differs from the text it holds, goes back to the last top-level block boundary before it, and
parses from there. A boundary is a line start with no list, quote, paragraph or code fence
open, so everything before it is settled whatever follows.

It keeps the rows it has. Each block is a row, and a row whose block kept its shape gets the
new text through a signal: one native label patch. A row is built again only when its shape
changes (a paragraph that turned into a heading, a list item that gained a level), and rows
are added and removed at the end as blocks start and go away.

Appending a word to the last paragraph of a long reply therefore costs one block parse and one
label update. `crates/day-pieces/tests/text_mock.rs` asserts exactly that against the mock
toolkit's operation log, and `crates/day-spec/src/markdown.rs` checks the incremental result
against a full parse at every prefix of a sample document.

An edit in the middle of the text is handled the same way, from the boundary before the edit.

## How it is built

The view is composed in `day-pieces` from native labels and Day's containers. There is one
implementation for every toolkit and no toolkit duty behind it.

| Block | Built from |
| --- | --- |
| paragraph, heading | a [`label(…).markdown()`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.markdown) |
| list item | a marker label (a bullet or the number) beside the item's content, indented one step per level |
| block quote | a vertical `divider` beside the quoted blocks, one per level |
| code block | a monospaced label in a [`.card()`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.card), under a header row |
| rule | a `divider` |

A native rich-text view per toolkit was the alternative. It would select across blocks, which
this composition does not. It would also be eight implementations of block layout, list
markers and code surfaces with eight sets of differences, and each would need its own
self-sizing and its own incremental update. The label already has the text behavior that
matters here on every toolkit: wrapping, runs, links, selection, accessibility and Dynamic
Type. The composition inherits all of it, and inherits every later improvement to the label.

The block model is public in [`day_spec::markdown`](https://docs.rs/day-spec/latest/day_spec/markdown/index.html): a [`MarkdownDocument`](https://docs.rs/day-spec/latest/day_spec/markdown/struct.MarkdownDocument.html) is a flat list of
[`MarkdownBlock`](https://docs.rs/day-spec/latest/day_spec/markdown/struct.MarkdownBlock.html)s, each a paragraph, heading, code block or rule with its quote depth, its list
depth and its marker. An app that wants a different look can lay those blocks out itself.

## Testing

[`.block_ids(|i| id!(reply_block, i))`](https://docs.rs/day-pieces/latest/day_pieces/trait.MarkdownBuilder.html#tymethod.block_ids) gives the text of block `i` the id `reply_block:<i>`, so
a test or a DayScript step can read it:

```rust
d.assert_text("reply_block:0", "Release notes").await?;
```

A rule has no text and takes no id. The conformance cases `markdown-view-blocks`,
`markdown-view-streams` and `markdown-view-code-accessory` run on every toolkit.

## Toolkit support

The view draws with labels, so each toolkit's label limits apply to it unchanged: line limits,
truncation and selection reports in [text](./text.md#toolkit-support), and decoration styles
and run small capitals in [text-runs](./text-runs.md#what-each-toolkit-cannot-draw). The view
adds none of its own.

## Limits

- Selection does not cross blocks.
- A code block wraps long lines; it does not scroll sideways.
- Tables are not parsed.
- Images and other inline attachments are not shown.
