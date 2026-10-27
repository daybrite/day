---
title: "Styled text and Markdown"
description: "Style labels with text runs and inline Markdown, including links and platform behavior."
---

<!-- Copyright © The Daybrite Project
     SPDX-License-Identifier: CC-BY-SA-4.0 -->

# Styled text and Markdown

Styled text gives parts of a passage different emphasis while keeping them in one readable flow. A
text run marks a span with formatting, such as bold text or an inline code style.

This synthetic sentence demonstrates the run builder; it is not a pattern for splitting a translated
UI message:

```rust
let (text, runs) = TextBuilder::new()
    .base(Font::Body)
    .text("Save the file as ")
    .code("notes.md")
    .text(" before you ")
    .strong("quit")
    .text(".")
    .build();

label(text).runs(runs)
```

> [!TIP]
> **Keep translated sentences whole.** For app-owned instructions, put the complete sentence
> and its emphasis in Fluent, then use [`label`](https://docs.rs/day-pieces/latest/day_pieces/fn.label.html)`(res::str::save_instructions())`[`.markdown`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.markdown)`()`.
> Translators can reorder it without changing byte ranges or assembling English fragments.
> Use `TextBuilder` for structured user-authored content or styles that Markdown cannot express.
> See [Localization](/docs/how-to/localization).

`TextBuilder` returns the plain string and the runs that style it, which is exactly what
[`label`](https://docs.rs/day-pieces/latest/day_pieces/fn.label.html)`()`[`.runs`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.runs)`()` takes. [`.runs_from(builder)`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.runs_from) does both in one call.

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](/docs/reference/localization#reference-example-messages) and [catalog setup](/docs/how-to/localization).

## Why not several labels in a row

A row of labels looks identical on one line and then goes wrong everywhere else. It wraps at the
row's boundaries rather than between words, so a narrow window breaks the sentence in the wrong
place. A drag selects one label, not the sentence. A screen reader announces each fragment as its
own element. Text runs keep it one paragraph: one wrap, one selection, one announcement.

## The run model

A `TextRun` is a **byte** range into the string plus what to do with it:

```rust
pub struct TextRun {
    pub range: std::ops::Range<usize>,
    pub font: FontSpec,          // style + weight + italic + monospace + tabular + scale
    pub color: Option<Color>,
    pub background: Option<Color>,
    pub underline: Underline,    // None | Single | Double | Dotted | Wavy
    pub strikethrough: bool,
    pub link: Option<String>,
}
```

Runs must be ascending, non-overlapping, inside the string, and on character boundaries.
`runs_are_valid` checks that once in the pieces layer, so no toolkit has to: overlapping ranges
would produce a different wrong answer per platform, and a range splitting a multi-byte character
would panic on `str` slicing in some toolkits and render mojibake in others. Runs that fail the
check are dropped with a warning and the text renders unstyled.

Text and runs travel together, in `LabelPatch::Runs(String, Vec<TextRun>)`, because a range only
means something against a particular string; patching one without the other would break on the
next edit.

`FontSpec::monospace` asks for the platform's fixed-pitch face. It rides the ordinary font path,
so it works on a whole label ([`label`](https://docs.rs/day-pieces/latest/day_pieces/fn.label.html)`(res::str::loading())`[`.monospace`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.monospace)`()`) as well as on a run.

`FontSpec::scale` is a **relative** size: it multiplies whatever the semantic style resolves to, so
a run at `1.4` still tracks the reader's text-size setting. `Font::System(pt)` is the absolute form
and does not, which is why an editor's size control moves `scale` and an imported document's
`font-size: 14px` lands in `Font::System`.

A label's runs are the character half of [`StyledText`](texteditor.md), the document type
[`.markdown()`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.markdown) produces and [`day-piece-text-editor`](texteditor.md) edits. The same runs render in
both places.

## Builder vocabulary

| Method | Run |
| --- | --- |
| [`.text(s)`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextBuilder.html#method.text) | unstyled, at the base font |
| [`.strong(s)`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextBuilder.html#method.strong) | bold |
| [`.emphasis(s)`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextBuilder.html#method.emphasis) | italic |
| [`.code(s)`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextBuilder.html#method.code) | the fixed-pitch face |
| [`.colored(s, c)`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextBuilder.html#method.colored) | a color |
| [`.underline(s)`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextBuilder.html#method.underline) | underlined |
| [`.highlight(s, c)`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextBuilder.html#method.highlight) | a background, with a readable foreground over it |
| [`.sized(s, k)`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextBuilder.html#method.sized) | `k` times the base size (relative, so it still scales) |
| [`.strikethrough(s)`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextBuilder.html#method.strikethrough) | struck through |
| [`.link(s, url)`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextBuilder.html#method.link) | drawn as a link (see below) |

[`.base(font)`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextBuilder.html#method.base) sets the font the runs vary from; without it, runs sit on the label's own font.

## Toolkit implementations

| toolkit | Mechanism | Fixed-pitch face |
| --- | --- | --- |
| AppKit | [`NSAttributedString`](https://developer.apple.com/documentation/foundation/nsattributedstring) on the [`NSTextField`](https://developer.apple.com/documentation/appkit/nstextfield) | `monospacedSystemFontOfSize:weight:` |
| UIKit | [`NSAttributedString`](https://developer.apple.com/documentation/foundation/nsattributedstring) on the [`UILabel`](https://developer.apple.com/documentation/uikit/uilabel) | `monospacedSystemFontOfSize:weight:`, scaled by [`UIFontMetrics`](https://developer.apple.com/documentation/uikit/uifontmetrics) |
| GTK | Pango markup | `font_family="monospace"` |
| Qt | Qt rich text | `<code>`, which Qt maps to its own fixed font |
| Android | `SpannableString` spans | `TypefaceSpan("monospace")` |
| ArkUI | [`ARKUI_NODE_SPAN`](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-arkui/capi-native-node-h.md) children | `HarmonyOS Sans Mono, monospace` |
| WinUI | `Run` inlines in [`TextBlock.Inlines`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.textblock) | `Consolas, Courier New, monospace` |
| Web | `<span class="day-run">` children | the `ui-monospace` stack |

Ranges convert to UTF-16 for the Apple and Android toolkits, which index text that way; any emoji
or CJK in the string makes the two disagree.

### What each toolkit cannot draw

| attribute | where it degrades |
| --- | --- |
| `Underline::Double` | Qt draws a single rule; Android draws a single rule |
| `Underline::Dotted` | GTK draws a single rule (Pango has no dotted); Android draws a single rule |
| `Underline::Wavy` | GTK draws the spell-check squiggle (`Underline::Error`); Android draws a single rule |
| `background` | everywhere except where noted below; WinUI needs a [`TextHighlighter`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.documents.texthighlighter), since a `Run` has no background |
| `scale` | GTK takes an absolute Pango size in 1/1024 pt; the relative `font_scale`/`size="N%"` attributes are Pango 1.50's, and a Pango that does not know an attribute fails the whole markup parse and renders the label empty. Qt takes points too: its CSS subset ignores a percentage |

Everything else draws on all eight.

Link activation is `Cap::TextLinks`, and it is narrower than rendering:

| toolkit | Activation |
| --- | --- |
| GTK | `activate-link` on the label |
| Qt | `linkActivated` on the label |
| UIKit | a text-view delegate (see below) |
| Android | a `ClickableSpan` + `LinkMovementMethod` |
| WinUI | `Hyperlink.Click` |
| Web | the anchor's click, with its navigation cancelled |
| AppKit | the native field editor reports to the `DayLabel` text-field subclass |
| ArkUI | native `NODE_ON_CLICK` on each link's `SPAN` |

Every one of these reports the target to the app rather than opening it itself, so a label's
[`.on_link()`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.on_link) decides. Its default calls [`open_link`](https://docs.rs/day-pieces/latest/day_pieces/fn.open_link.html): a leading `#` navigates to the route after
the hash, and other targets open through [`Toolkit::open_url`](https://docs.rs/day-spec/latest/day_spec/trait.Toolkit.html#method.open_url). This also applies to links made
with [`TextBuilder::link`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextBuilder.html#method.link); see [markdown links](./text-runs.md#links) for the route contract and
custom-handler behavior. Where activation is missing the run still draws as a link; the tap does
nothing.

**ArkUI keeps hit testing in the native text layout.** Each linked span reports its original
target to the owning label, including Unicode, custom schemes and `#route` targets. Plain
spans do not register clicks. Wrapped spans activate on their glyphs across lines, without
turning the label's trailing whitespace into a link. Links receive a default blue tint and
underline; explicit run colors remain intact. Replacing runs, returning to plain text, or
disposing a label unregisters its old span callbacks before releasing the nodes.

The [native link suite](https://github.com/daybrite/day/blob/main/docs/backend-contracts/test-infrastructure.md#native-arkui-link-tests) exercises OS taps, selection-enabled
labels, reactive targets, removal from a handler and default route navigation. The portable
`label-link-runs-reactive` case checks native text through linked/plain/relinked transitions;
injected taps alone do not prove span hit testing. Per-link keyboard traversal and screen-reader
activation are not covered by this native tap suite.

Some toolkit details affect how runs are built.



**GTK renders runs as markup, not as a `pango::AttrList`,** because Pango's attributes cannot
express a link and the markup dialect can. A [`GtkLabel`](https://docs.gtk.org/gtk4/class.Label.html)'s attribute list *overrides* the
attributes its markup parsed, so a base weight attribute spanning the label silently defeats a
`<b>` run. So a label with runs carries no attribute list at all, and its base
font arrives as a wrapping `<span>`. It also measures from that markup: `label.text()` is the
markup with its tags stripped, and measuring it would size every run at the base font.

**Qt does not resolve a generic `monospace` family from a style attribute** (it rendered
proportional), so the fixed face comes from `<code>` instead.

**A label with a link on iOS is a [`UITextView`](https://developer.apple.com/documentation/uikit/uitextview).** UIKit reserves both selection and link hit
testing for text-input views, so [`.selectable()`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.selectable) rebuilds the label as a read-only text view, and
a label that arrives with a link run is built as one from the start. That swap carries the
attributed text across rather than the plain string. A link that first appears in a later patch
cannot upgrade the backing, since `patch` has no way to hand back a new handle: seed the text with
its link, or mark the label [`.selectable()`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.selectable).

## Markdown

Add emphasis, links, and inline code to labels with Markdown. The [`.markdown()`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.markdown) modifier
turns the label’s text into styled runs and updates them when that text changes.

```rust
label(res::str::save_instructions()).markdown()
label(res::str::release_note()).markdown()          // a translated string
label(move || draft.get()).markdown()         // live, as the user types
```

[`.markdown()`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.markdown) reads a label's text as inline markdown: the markers are stripped and what they
marked becomes styled runs in the one label.

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](/docs/reference/localization#reference-example-messages) and [catalog setup](/docs/how-to/localization).

### Updating Markdown text

Markdown is parsed when the label's text changes, so translations, network content, and
user-supplied text can carry inline formatting. Use a plain label when formatting is not needed.

### The grammar

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
layout, and layout in Day is [`column`](https://docs.rs/day-pieces/latest/day_pieces/fn.column.html), [`form`](https://docs.rs/day-pieces/latest/day_pieces/fn.form.html), [`list`](https://docs.rs/day-pieces/latest/day_pieces/fn.list.html). A label is one paragraph; this is the
markup that fits inside one.

### Links

A `[text](url)` run draws as a link and reports its target when tapped:

```rust
label(res::str::terms_blurb()).markdown()                          // opens the target
label(res::str::terms_blurb()).markdown().on_link(|url| route(url)) // or handle it yourself
label(res::str::navigation_links()).markdown()
```

Without [`.on_link()`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.on_link), a target beginning with `#` navigates to the route after the hash:
`#settings` calls [`navigate("settings")`](https://docs.rs/day-pieces/latest/day_pieces/fn.navigate.html). This uses the existing [navigation](./navigation.md)
rules, including paths and query parameters (`#mail/inbox?filter=unread`), percent escapes,
and the empty route (`#`, which pops the innermost stack to its root). Unknown routes are
ignored; they never open externally. Other targets open in the platform's default handler,
including URLs with a fragment such as `https://daybrite.dev/#docs`.

The [`link`](./text.md) piece and [`TextBuilder::link`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextBuilder.html#method.link) use the same behavior. A custom
[`.on_link()`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.on_link) receives the original target and overrides both navigation and external opening.
Call [`open_link(target)`](https://docs.rs/day-pieces/latest/day_pieces/fn.open_link.html) from the handler to use the default for targets you do not handle.
The route must belong to a registered navigation surface; a separate Settings window, for
example, needs an app handler that calls [`day::open_preferences()`](https://docs.rs/day-app/latest/day/fn.open_preferences.html).

Activation is `Cap::TextLinks`, which is narrower than run rendering. Where it is missing the
link still draws; the tap does nothing. See [platform support](#toolkit-implementations) for the
per-toolkit table.

### Escaping user text

[`.markdown()`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.markdown) on a string a user supplied means their asterisks become emphasis. When the text is
data rather than markup, leave the modifier off — a plain label never interprets anything.

## What `Cap` answers

`Cap::TextRuns` and `Cap::TextLinks` are Native on all toolkits (AppKit, GTK, Qt,
UIKit, Android, WinUI, ArkUI and Web).

## Platform guidance

- [Apple: typography](https://developer.apple.com/design/human-interface-guidelines/typography)
- [Windows: typography](https://learn.microsoft.com/en-us/windows/apps/design/style/typography)
