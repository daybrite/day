---
navTitle: "Markdown"
title: "Markdown labels"
description: "label(…).markdown(): inline Markdown rendered through each platform's attributed text."
---

<!-- Copyright © The Daybrite Project
     SPDX-License-Identifier: CC-BY-SA-4.0 -->

# Markdown

Add emphasis, links, and inline code to labels with Markdown. The [`.markdown()`](https://daybrite.dev/api/day_pieces/trait.LabelBuilder.html#tymethod.markdown) modifier
turns the label’s text into styled runs and updates them when that text changes.

```rust
label(res::str::save_instructions()).markdown()
label(res::str::release_note()).markdown()          // a translated string
label(move || draft.get()).markdown()         // live, as the user types
```

[`.markdown()`](https://daybrite.dev/api/day_pieces/trait.LabelBuilder.html#tymethod.markdown) reads a label's text as inline markdown: the markers are stripped and what they
marked becomes [styled runs](./text-runs.md) in the one label.

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](snippet-catalog.md) and [catalog setup](/docs/how-to/localization).

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
layout, and layout in Day is [`column`](https://daybrite.dev/api/day_pieces/fn.column.html), [`form`](https://daybrite.dev/api/day_pieces/fn.form.html), [`list`](https://daybrite.dev/api/day_pieces/fn.list.html). A label is one paragraph; this is the
markup that fits inside one.

## Links

A `[text](url)` run draws as a link and reports its target when tapped:

```rust
label(res::str::terms_blurb()).markdown()                          // opens the target
label(res::str::terms_blurb()).markdown().on_link(|url| route(url)) // or handle it yourself
label(res::str::navigation_links()).markdown()
```

Without [`.on_link()`](https://daybrite.dev/api/day_pieces/trait.LabelBuilder.html#tymethod.on_link), a target beginning with `#` navigates to the route after the hash:
`#settings` calls [`navigate("settings")`](https://daybrite.dev/api/day_pieces/fn.navigate.html). This uses the existing [navigation](./navigation.md)
rules, including paths and query parameters (`#mail/inbox?filter=unread`), percent escapes,
and the empty route (`#`, which pops the innermost stack to its root). Unknown routes are
ignored; they never open externally. Other targets open in the platform's default handler,
including URLs with a fragment such as `https://daybrite.dev/#docs`.

The [`link`](./text.md) piece and [`TextBuilder::link`](https://daybrite.dev/api/day_pieces/struct.TextBuilder.html#method.link) use the same behavior. A custom
[`.on_link()`](https://daybrite.dev/api/day_pieces/trait.LabelBuilder.html#tymethod.on_link) receives the original target and overrides both navigation and external opening.
Call [`open_link(target)`](https://daybrite.dev/api/day_pieces/fn.open_link.html) from the handler to use the default for targets you do not handle.
The route must belong to a registered navigation surface; a separate Settings window, for
example, needs an app handler that calls [`day::open_preferences()`](https://daybrite.dev/api/day/fn.open_preferences.html).

Activation is `Cap::TextLinks`, which is narrower than run rendering. Where it is missing the
link still draws; the tap does nothing. See [text-runs](./text-runs.md#per-toolkit) for the
per-toolkit table.

## Escaping user text

[`.markdown()`](https://daybrite.dev/api/day_pieces/trait.LabelBuilder.html#tymethod.markdown) on a string a user supplied means their asterisks become emphasis. When the text is
data rather than markup, leave the modifier off — a plain label never interprets anything.
