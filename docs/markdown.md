---
title: "Markdown labels"
description: "label(…).markdown(): inline Markdown rendered through each platform's attributed text."
---

<!-- Copyright © The Daybrite Project
     SPDX-License-Identifier: CC-BY-SA-4.0 -->

# Markdown

```rust
label("Save it as **notes.md** before you *quit*.").markdown()
label(tr("release-note")).markdown()          // a translated string
label(move || draft.get()).markdown()         // live, as the user types
```

`.markdown()` reads a label's text as inline markdown: the markers are stripped and what they
marked becomes [styled runs](./text-runs.md) in the one label.

## Why it parses at run time

The string a label shows is usually not a literal. It is a translation picked from the locale
bundle at run time, a value off the network, or text a user is typing. A compile-time macro can
only see literals, which is the one case that needs it least, so the parse happens on the string
the label actually receives, every time it changes.

The cost is a parse per update of a label's worth of text. The parser allocates only the output
string and its runs, and it is about two hundred lines in `day-pieces` with no crate dependency.

## The grammar

| Markdown | Result |
| --- | --- |
| `**bold**`, `__bold__` | bold |
| `*italic*`, `_italic_` | italic |
| `` `code` `` | the platform's monospaced face |
| `~~strike~~` | struck through |
| `[text](url)` | a link run, in the platform tint |
| `\*` | a literal `*` |

Styles nest: `**bold with *italic* inside**` gives three runs. A code span is literal inside, so
`` `**not bold**` `` shows the asterisks.

Anything unrecognized is text. An unclosed `**`, a stray `_` inside a word, or a `[` with no
`](…)` after it stays exactly as typed. That is markdown's own rule, and it keeps a half-typed
string in a live editor from flickering between readings on every keystroke.

**Block constructs are not parsed.** Headings, lists, quotes, tables and paragraph breaks are
layout, and layout in Day is `column`, `form`, `list`. A label is one paragraph; this is the
markup that fits inside one.

## Links

A `[text](url)` run draws as a link and reports its target when tapped:

```rust
label(tr("terms-blurb")).markdown()                          // opens the target
label(tr("terms-blurb")).markdown().on_link(|url| route(url)) // or handle it yourself
```

Without `.on_link()` the target opens in the platform's default handler, the same as the
[`link`](./text.md) piece. With it, nothing opens until the handler decides to; it can route
in-app or confirm first.

Activation is `Cap::TextLinks`, which is narrower than run rendering. Where it is missing the
link still draws; the tap does nothing. See [text-runs.md](./text-runs.md#per-toolkit) for the
per-toolkit table.

## Escaping user text

`.markdown()` on a string a user supplied means their asterisks become emphasis. When the text is
data rather than markup, leave the modifier off — a plain label never interprets anything.
