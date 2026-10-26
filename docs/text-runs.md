---
title: "Styled runs"
description: "Several styles inside one label: the runs API over attributed text."
---

<!-- Copyright © The Daybrite Project
     SPDX-License-Identifier: CC-BY-SA-4.0 -->

# Styled runs

One label can carry several styles. This synthetic sentence demonstrates the run builder;
it is not a pattern for splitting a translated UI message:

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
> and its emphasis in Fluent, then use [`label`](https://daybrite.dev/api/day_pieces/fn.label.html)`(res::str::save_instructions())`[`.markdown`](https://daybrite.dev/api/day_pieces/trait.LabelBuilder.html#tymethod.markdown)`()`.
> Translators can reorder it without changing byte ranges or assembling English fragments.
> Use `TextBuilder` for structured user-authored content or styles that Markdown cannot express.
> See [Localization](/docs/how-to/localization).

`TextBuilder` returns the plain string and the runs that style it, which is exactly what
[`label`](https://daybrite.dev/api/day_pieces/fn.label.html)`()`[`.runs`](https://daybrite.dev/api/day_pieces/trait.LabelBuilder.html#tymethod.runs)`()` takes. [`.runs_from(builder)`](https://daybrite.dev/api/day_pieces/trait.LabelBuilder.html#tymethod.runs_from) does both in one call.

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](snippet-catalog.md) and [catalog setup](/docs/how-to/localization).

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
so it works on a whole label ([`label`](https://daybrite.dev/api/day_pieces/fn.label.html)`(res::str::loading())`[`.monospace`](https://daybrite.dev/api/day_pieces/trait.LabelBuilder.html#tymethod.monospace)`()`) as well as on a run.

`FontSpec::scale` is a **relative** size: it multiplies whatever the semantic style resolves to, so
a run at `1.4` still tracks the reader's text-size setting. `Font::System(pt)` is the absolute form
and does not, which is why an editor's size control moves `scale` and an imported document's
`font-size: 14px` lands in `Font::System`.

A label's runs are the character half of [`StyledText`](texteditor.md), the document type
[`.markdown()`](https://daybrite.dev/api/day_pieces/trait.LabelBuilder.html#tymethod.markdown) produces and [`day-piece-text-editor`](texteditor.md) edits. The same runs render in
both places.

## Builder vocabulary

| Method | Run |
| --- | --- |
| [`.text(s)`](https://daybrite.dev/api/day_pieces/struct.TextBuilder.html#method.text) | unstyled, at the base font |
| [`.strong(s)`](https://daybrite.dev/api/day_pieces/struct.TextBuilder.html#method.strong) | bold |
| [`.emphasis(s)`](https://daybrite.dev/api/day_pieces/struct.TextBuilder.html#method.emphasis) | italic |
| [`.code(s)`](https://daybrite.dev/api/day_pieces/struct.TextBuilder.html#method.code) | the fixed-pitch face |
| [`.colored(s, c)`](https://daybrite.dev/api/day_pieces/struct.TextBuilder.html#method.colored) | a color |
| [`.underline(s)`](https://daybrite.dev/api/day_pieces/struct.TextBuilder.html#method.underline) | underlined |
| [`.highlight(s, c)`](https://daybrite.dev/api/day_pieces/struct.TextBuilder.html#method.highlight) | a background, with a readable foreground over it |
| [`.sized(s, k)`](https://daybrite.dev/api/day_pieces/struct.TextBuilder.html#method.sized) | `k` times the base size (relative, so it still scales) |
| [`.strikethrough(s)`](https://daybrite.dev/api/day_pieces/struct.TextBuilder.html#method.strikethrough) | struck through |
| [`.link(s, url)`](https://daybrite.dev/api/day_pieces/struct.TextBuilder.html#method.link) | drawn as a link (see below) |

[`.base(font)`](https://daybrite.dev/api/day_pieces/struct.TextBuilder.html#method.base) sets the font the runs vary from; without it, runs sit on the label's own font.

## Per-toolkit

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
[`.on_link()`](https://daybrite.dev/api/day_pieces/trait.LabelBuilder.html#tymethod.on_link) decides. Its default calls [`open_link`](https://daybrite.dev/api/day_pieces/fn.open_link.html): a leading `#` navigates to the route after
the hash, and other targets open through [`Toolkit::open_url`](https://daybrite.dev/api/day_spec/trait.Toolkit.html#method.open_url). This also applies to links made
with [`TextBuilder::link`](https://daybrite.dev/api/day_pieces/struct.TextBuilder.html#method.link); see [markdown links](./markdown.md#links) for the route contract and
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

**On macOS the label itself handles the field editor's link callback.** AppKit makes the
[`NSTextField`](https://developer.apple.com/documentation/appkit/nstextfield) the shared field editor's delegate; it does not forward
`textView:clickedOnLink:atIndex:` to the field's separate control delegate. `DayLabel`, an
[`NSTextField`](https://developer.apple.com/documentation/appkit/nstextfield) subclass, handles that callback and returns `true`, suppressing AppKit's external
URL opening. It reports the original target through `Event::LinkActivated`, including `#route`
targets. The subclass is used for plain labels too, so links added by reactive markdown patches
work without replacing the view. Labels with links remain selectable for native hit testing;
removing links restores the app's explicit selection setting. See Apple's
[Working With the Field Editor](https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/TextEditing/Tasks/FieldEditor.html)
and the [native link regression](../crates/day-toolkit-appkit/tests/native_links.rs).

**GTK renders runs as markup, not as a `pango::AttrList`,** because Pango's attributes cannot
express a link and the markup dialect can. A [`GtkLabel`](https://docs.gtk.org/gtk4/class.Label.html)'s attribute list *overrides* the
attributes its markup parsed, so a base weight attribute spanning the label silently defeats a
`<b>` run. So a label with runs carries no attribute list at all, and its base
font arrives as a wrapping `<span>`. It also measures from that markup: `label.text()` is the
markup with its tags stripped, and measuring it would size every run at the base font.

**Qt does not resolve a generic `monospace` family from a style attribute** (it rendered
proportional), so the fixed face comes from `<code>` instead.

**A label with a link on iOS is a [`UITextView`](https://developer.apple.com/documentation/uikit/uitextview).** UIKit reserves both selection and link hit
testing for text-input views, so [`.selectable()`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.selectable) rebuilds the label as a read-only text view, and
a label that arrives with a link run is built as one from the start. That swap carries the
attributed text across rather than the plain string. A link that first appears in a later patch
cannot upgrade the backing, since `patch` has no way to hand back a new handle: seed the text with
its link, or mark the label [`.selectable()`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.selectable).

## Markdown

[markdown](./markdown.md) covers [`.markdown()`](https://daybrite.dev/api/day_pieces/trait.LabelBuilder.html#tymethod.markdown), which parses inline markdown at run time and
produces exactly these runs; it is the convenient way to get them when the text is a translated
string or something a user typed.

## What `Cap` answers

`Cap::TextRuns` and `Cap::TextLinks` are Native on all eight toolkits (AppKit, GTK, Qt,
UIKit, Android, WinUI, ArkUI and Web).

## Platform guidance

- [Apple: typography](https://developer.apple.com/design/human-interface-guidelines/typography)
- [Windows: typography](https://learn.microsoft.com/en-us/windows/apps/design/style/typography)
