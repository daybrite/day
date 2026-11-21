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
    pub underline: Option<Decoration>,
    pub strikethrough: Option<Decoration>,
    pub link: Option<String>,
}

pub struct Decoration {
    pub style: DecorationStyle,  // Single | Double | Dotted | Wavy
    pub color: Option<Color>,    // None = the text color
}
```

The underline and the strikethrough are the same type because every text system gives the two
lines the same two properties: how the line is drawn and what color it takes.
`Decoration::SINGLE` is the plain line, [`Decoration::double()`](https://docs.rs/day-spec/latest/day_spec/styled/struct.Decoration.html#method.double),
[`dotted()`](https://docs.rs/day-spec/latest/day_spec/styled/struct.Decoration.html#method.dotted) and [`wavy()`](https://docs.rs/day-spec/latest/day_spec/styled/struct.Decoration.html#method.wavy) are the other styles, and
[`.color(c)`](https://docs.rs/day-spec/latest/day_spec/styled/struct.Decoration.html#method.color) gives a line a color of its own.

Runs must be ascending, non-overlapping, inside the string, and on character boundaries.
`runs_are_valid` checks that once in the pieces layer, so no toolkit has to: overlapping ranges
would produce a different wrong answer per platform, and a range splitting a multi-byte character
would panic on `str` slicing in some toolkits and render mojibake in others. Runs that fail the
check are dropped with a warning and the text renders unstyled.

Text and runs travel together, in `LabelPatch::Runs(String, Vec<TextRun>)`, because a range only
means something against a particular string; patching one without the other would break on the
next edit.

[`FontSpec::small_caps`](https://docs.rs/day-spec/latest/day_spec/struct.FontSpec.html#structfield.small_caps) and
[`FontSpec::oldstyle_figures`](https://docs.rs/day-spec/latest/day_spec/struct.FontSpec.html#structfield.oldstyle_figures) ask for the font's small capitals and
its oldstyle figures (the OpenType `smcp` and `onum` features), on a run or on a whole label
([`.small_caps()`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.small_caps)).
[`FontSpec::feature_tags()`](https://docs.rs/day-spec/latest/day_spec/struct.FontSpec.html#method.feature_tags) lists the tags a descriptor asks for, so every
toolkit that takes a feature string spells the same set.

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
| [`.underline(s)`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextBuilder.html#method.underline) | underlined with one line in the text color |
| [`.underline_styled(s, d)`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextBuilder.html#method.underline_styled) | underlined with a `Decoration`: [`Decoration::wavy().color(red)`](https://docs.rs/day-spec/latest/day_spec/styled/struct.Decoration.html#method.wavy), `DecorationStyle::Double` |
| [`.small_caps(s)`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextBuilder.html#method.small_caps) | lowercase letters as small capitals |
| [`.highlight(s, c)`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextBuilder.html#method.highlight) | a background, with a readable foreground over it |
| [`.sized(s, k)`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextBuilder.html#method.sized) | `k` times the base size (relative, so it still scales) |
| [`.strikethrough(s)`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextBuilder.html#method.strikethrough) | struck through |
| [`.strikethrough_styled(s, d)`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextBuilder.html#method.strikethrough_styled) | struck through with a `Decoration` |
| [`.link(s, url)`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextBuilder.html#method.link) | drawn as a link (see below) |

[`.base(font)`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextBuilder.html#method.base) sets the font the runs vary from; without it, runs sit on the label's own font.

A diagnostic mark, a price that was, and a defined term:

```rust
TextBuilder::new()
    .text("Symbol ")
    .underline_styled("frobnicate", Decoration::wavy().color(Color::hex(0xD0021B)))
    .text(" is unresolved. Was ")
    .strikethrough_styled("$40", Decoration::SINGLE.color(Color::hex(0x8E8E93)))
    .text(", now $32 under the ")
    .underline_styled("early rate", DecorationStyle::Dotted)
```

To decorate a whole label, [`label`](https://docs.rs/day-pieces/latest/day_pieces/fn.label.html)`(t)`[`.underline(d)`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.underline) and [`.strikethrough(d)`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.strikethrough) take the same
values and follow a reactive text ([docs/text](./text.md#decorations)).

## Toolkit implementations

| toolkit | Mechanism | Fixed-pitch face |
| --- | --- | --- |
| [AppKit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") | [`NSAttributedString`](https://developer.apple.com/documentation/foundation/nsattributedstring) on the [`NSTextField`](https://developer.apple.com/documentation/appkit/nstextfield) | `monospacedSystemFontOfSize:weight:` |
| [UIKit](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") | [`NSAttributedString`](https://developer.apple.com/documentation/foundation/nsattributedstring) on the [`UILabel`](https://developer.apple.com/documentation/uikit/uilabel) | `monospacedSystemFontOfSize:weight:`, scaled by [`UIFontMetrics`](https://developer.apple.com/documentation/uikit/uifontmetrics) |
| [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") | Pango markup | `font_family="monospace"` |
| [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") | [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") rich text | `<code>`, which [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") maps to its own fixed font |
| [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") | `SpannableString` spans | `TypefaceSpan("monospace")` |
| [ArkUI](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") | [`ARKUI_NODE_SPAN`](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-arkui/capi-native-node-h.md) children | `HarmonyOS Sans Mono, monospace` |
| [WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") | `Run` inlines in [`TextBlock.Inlines`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.textblock) | `Consolas, Courier New, monospace` |
| [Web](https://daybrite.dev/docs/reference/platforms/web-dom "toolkit:web-dom") | `<span class="day-run">` children | the `ui-monospace` stack |

Ranges convert to UTF-16 for the Apple and [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") toolkits, which index text that way; any emoji
or CJK in the string makes the two disagree.

### What each toolkit cannot draw

| attribute | where it degrades |
| --- | --- |
| `DecorationStyle::Double` | [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") and [WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") draw a single rule; [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") draws a double underline but a single strikethrough (Pango has one strikethrough) |
| `DecorationStyle::Dotted` | [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") and [WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") draw a single rule; [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") draws a dotted underline but a single strikethrough |
| `DecorationStyle::Wavy` | [AppKit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") and [UIKit](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") draw a dash-dot-dot line (Apple text has no wavy pattern); [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") draws its spell-check squiggle under the text and a single strikethrough; [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") draws a wavy underline but a single strikethrough; [WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") draws a single rule |
| [`Decoration::color`](https://docs.rs/day-spec/latest/day_spec/styled/struct.Decoration.html#structfield.color) | [WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") draws both lines in the text color; [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") draws a strikethrough in the text color |
| an underline and a strikethrough on one run | [ArkUI](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") draws the strikethrough only, since a span takes one decoration |
| `small_caps` on a run | [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") draws the run's ordinary lowercase, since its rich text can only synthesize small capitals; a whole label's `small_caps` uses the face's `smcp` from Qt 6.7 |
| `small_caps`, `oldstyle_figures` | a face without the glyphs draws its ordinary lowercase and figures on every toolkit |
| `background` | [WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") draws it through a [`TextHighlighter`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.documents.texthighlighter), since a `Run` has no background |
| `scale` | [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") takes an absolute Pango size in 1/1024 pt; the relative `font_scale`/`size="N%"` attributes are Pango 1.50's, and a Pango that does not know an attribute fails the whole markup parse and renders the label empty. [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") takes points too: its CSS subset ignores a percentage |

Everything else draws on all eight, [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") and the [Web](https://daybrite.dev/docs/reference/platforms/web-dom "toolkit:web-dom") included: Android draws a styled or colored line itself over each line of the range.

GTK and Qt receive runs as markup from one shared function,
[`day_spec::runs_to_markup`](https://docs.rs/day-spec/latest/day_spec/fn.runs_to_markup.html). For
Pango it writes a styled or colored line as span attributes (`underline="double"`,
`underline="error"` for the wavy line, `underline_color`, `strikethrough_color`) and the font
features as `font_features`. For Qt it writes `text-decoration-color`; the Qt toolkit wraps a
double, dotted or wavy underline in a span with Qt's `text-underline-style` and leaves out a
run's small capitals.

Link activation is `Cap::TextLinks`, and it is narrower than rendering:

| toolkit | Activation |
| --- | --- |
| [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") | `activate-link` on the label |
| [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") | `linkActivated` on the label |
| [UIKit](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") | a text-view delegate (see below) |
| [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") | a `ClickableSpan` + `LinkMovementMethod` |
| [WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") | `Hyperlink.Click` |
| [Web](https://daybrite.dev/docs/reference/platforms/web-dom "toolkit:web-dom") | the anchor's click, with its navigation cancelled |
| [AppKit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") | the native field editor reports to the `DayLabel` text-field subclass |
| [ArkUI](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") | native `NODE_ON_CLICK` on each link's `SPAN` |

Every one of these reports the target to the app rather than opening it itself, so a label's
[`.on_link()`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.on_link) decides. Its default calls [`open_link`](https://docs.rs/day-pieces/latest/day_pieces/fn.open_link.html): a leading `#` navigates to the route after
the hash, and other targets open through [`Toolkit::open_url`](https://docs.rs/day-spec/latest/day_spec/trait.Toolkit.html#method.open_url). This also applies to links made
with [`TextBuilder::link`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextBuilder.html#method.link); see [markdown links](./text-runs.md#links) for the route contract and
custom-handler behavior. Where activation is missing the run still draws as a link; the tap does
nothing.

**[ArkUI](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") keeps hit testing in the native text layout.** Each linked span reports its original
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



**[GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") renders runs as markup, not as a `pango::AttrList`,** because Pango's attributes cannot
express a link and the markup dialect can. A [`GtkLabel`](https://docs.gtk.org/gtk4/class.Label.html)'s attribute list *overrides* the
attributes its markup parsed, so a base weight attribute spanning the label silently defeats a
`<b>` run. So a label with runs carries no attribute list at all, and its base
font arrives as a wrapping `<span>`. It also measures from that markup: `label.text()` is the
markup with its tags stripped, and measuring it would size every run at the base font.

**[Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") does not resolve a generic `monospace` family from a style attribute** (it rendered
proportional), so the fixed face comes from `<code>` instead.

**A label with a link on [iOS](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") is a [`UITextView`](https://developer.apple.com/documentation/uikit/uitextview).** [UIKit](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") reserves both selection and link hit
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

**A label is one paragraph**, and this is the markup that fits inside one. Headings, lists,
quotes, code blocks and rules belong to the [Markdown view](./markdown-view.md),
[`markdown(text)`](https://docs.rs/day-pieces/latest/day_pieces/fn.markdown.html), which lays a document out as a column of these labels.

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
