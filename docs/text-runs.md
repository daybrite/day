<!-- Copyright © The Daybrite Project
     SPDX-License-Identifier: CC-BY-SA-4.0 -->

# Styled runs

One label, several styles:

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

`TextBuilder` returns the plain string and the runs that style it, which is exactly what
`label().runs()` takes. `.runs_from(builder)` does both in one call.

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
    pub font: FontSpec,
    pub color: Option<Color>,
    pub strikethrough: bool,
    pub link: Option<String>,
}
```

Runs must be ascending, non-overlapping, inside the string, and on character boundaries.
`runs_are_valid` checks that once in the pieces layer, so no backend has to: overlapping ranges
would produce a different wrong answer per platform, and a range splitting a multi-byte character
would panic on `str` slicing in some backends and render mojibake in others. Runs that fail the
check are dropped with a warning and the text renders unstyled.

Text and runs travel together, in `LabelPatch::Runs(String, Vec<TextRun>)` — a range only means
something against a particular string, so patching one without the other would be a bug waiting
for the next edit.

`FontSpec::monospace` asks for the platform's fixed-pitch face. It rides the ordinary font path,
so it works on a whole label (`label("…").monospace()`) as well as on a run.

## Builder vocabulary

| Method | Run |
| --- | --- |
| `.text(s)` | unstyled, at the base font |
| `.strong(s)` | bold |
| `.emphasis(s)` | italic |
| `.code(s)` | the fixed-pitch face |
| `.colored(s, c)` | a colour |
| `.strikethrough(s)` | struck through |
| `.link(s, url)` | drawn as a link (see below) |

`.base(font)` sets the font the runs vary from; without it, runs sit on the label's own font.

## Per-toolkit

| toolkit | Mechanism | Fixed-pitch face |
| --- | --- | --- |
| AppKit | `NSAttributedString` on the `NSTextField` | `monospacedSystemFontOfSize:weight:` |
| UIKit | `NSAttributedString` on the `UILabel` | `monospacedSystemFontOfSize:weight:`, scaled by `UIFontMetrics` |
| GTK | Pango markup | `font_family="monospace"` |
| Qt | Qt rich text | `<code>`, which Qt maps to its own fixed font |
| Android | `SpannableString` spans | `TypefaceSpan("monospace")` |
| ArkUI | `ARKUI_NODE_SPAN` children | `HarmonyOS Sans Mono, monospace` |
| XAML | `Run` inlines in `TextBlock.Inlines` | `Consolas, Courier New, monospace` |
| web-dom | `<span class="day-run">` children | the `ui-monospace` stack |

Ranges convert to UTF-16 for the Apple and Android backends, which index text that way; any emoji
or CJK in the string makes the two disagree.

Link activation is `Cap::TextLinks`, and it is narrower than rendering:

| toolkit | Activation |
| --- | --- |
| GTK | `activate-link` on the label |
| Qt | `linkActivated` on the label |
| UIKit | a text-view delegate (see below) |
| Android | a `ClickableSpan` + `LinkMovementMethod` |
| XAML | `Hyperlink.Click` |
| web-dom | the anchor's click, with its navigation cancelled |
| AppKit | **not yet** — an `NSTextField` cannot hit-test a link, so this needs the same swap to a text view that UIKit does |
| ArkUI | **not yet** |

Every one of these reports the target to the app rather than opening it itself, so a label's
`.on_link()` decides. Its default opens the URL through `Toolkit::open_url`, which is what a link
in a paragraph is normally expected to do. Where activation is missing the run still draws as a
link; the tap does nothing.

Three more things worth knowing:

**GTK renders runs as markup, not as a `pango::AttrList`,** because Pango's attributes cannot
express a link and the markup dialect can. The catch is that a `GtkLabel`'s attribute list
*overrides* the attributes its markup parsed — a base weight attribute spanning the label
silently defeats a `<b>` run. So a label with runs carries no attribute list at all, and its base
font arrives as a wrapping `<span>`. It also measures from that markup: `label.text()` is the
markup with its tags stripped, and measuring it would size every run at the base font.

**Qt does not resolve a generic `monospace` family from a style attribute** — it rendered
proportional — so the fixed face comes from `<code>` instead.

**A label with a link on iOS is a `UITextView`.** UIKit reserves both selection and link hit
testing for text-input views, so `.selectable()` rebuilds the label as a read-only text view — and
a label that arrives WITH a link run is built as one from the start. That swap carries the
attributed text across rather than the plain string. A link that first appears in a later patch
cannot upgrade the backing, since `patch` has no way to hand back a new handle: seed the text with
its link, or mark the label `.selectable()`.

## Markdown

[markdown.md](./markdown.md) covers `.markdown()`, which parses inline markdown at run time and
produces exactly these runs — the ergonomic way to get them when the text is a translated string
or something a user typed.

## What `Cap` answers

`Cap::TextRuns` is Native on all eight backends. `Cap::TextLinks` is Native on six (GTK, Qt,
UIKit, Android, XAML, web-dom) and Unsupported on AppKit and ArkUI.
