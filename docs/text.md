---
navTitle: "Text"
title: "Text & typography"
description: "Labels, semantic fonts, the text scale, line limits and truncation, spacing, selection, and the text pipeline on every toolkit."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Text & typography

Text labels identify controls, explain actions, and present information for people to read.
Consistent text styles distinguish headings from supporting content and make a page easier to scan.

[`label(...)`](https://docs.rs/day-pieces/latest/day_pieces/fn.label.html) renders native text. Its
font is chosen from a **semantic (logical) style** that maps to each platform's text styles, so a
Day app matches the OS's typography and inherits its accessibility text scaling automatically.

```rust
use day::prelude::*;

label(res::str::chapter_one()).font(Font::Title)                 // a semantic style
label(res::str::a_quiet_caption()).font(Font::Footnote)
label(res::str::read_me()).font(Font::Body).bold()               // weight
label(res::str::whispered()).italic()                            // slant
label(res::str::price()).color(Color::hex(0x27AE60))         // color
label(res::str::everything())
    .font(Font::Title2)
    .weight(FontWeight::Heavy)
    .italic()
    .color(Color::hex(0x8E44AD))
label(res::str::custom_size_sample()).font(Font::System(18.0))                // a fixed point size; prefer semantic styles
```

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](/docs/reference/localization#reference-example-messages) and [catalog setup](/docs/how-to/localization).

## Semantic styles (`Font`)

The styles, largest to smallest, mirror SwiftUI's `Font.TextStyle`:

`LargeTitle`, `Title`, `Title2`, `Title3`, `Headline`, `Subheadline`, `Body` (default), `Callout`,
`Footnote`, `Caption`, `Caption2`. Plus `System(pt)` for a custom point size, and
`Custom("Family", pt)` for a bundled custom font referenced by family name (ship the file in the
project's `resource/fonts/` directory; see [docs/resources](resources.md) for the packaging and naming rules).

Each maps to the platform's native text style where one exists, so sizes and weights match the OS:

| Toolkit | Semantic style | Accessibility scaling |
|---|---|---|
| **[UIKit](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit")** ([iOS](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit")) | [`UIFont.preferredFont(forTextStyle:)`](https://developer.apple.com/documentation/uikit/uifont) (Dynamic Type) | Yes, live, via `adjustsFontForContentSizeCategory` (Settings ▸ Accessibility ▸ Larger Text) |
| **[AppKit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit")** ([macOS](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit")) | [`NSFont.preferredFont(forTextStyle:)`](https://developer.apple.com/documentation/appkit/nsfont) | Follows the system font settings |
| **[WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui")** | point sizes (aligned to the desktop scale) | Yes, `FontSize` tracks the OS text-scale-factor |
| **[Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material")** | `sp` sizes (mobile scale, aligned to [iOS](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit")) | Yes, `sp` tracks Settings ▸ Display ▸ Font size |
| **[GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk")** | Pango point sizes | Yes, Pango sizes track GNOME's text-scaling-factor |
| **[Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt")** | QFont point sizes | Honors the system DPI/font (no separate large-text toggle) |
| **[Web](https://daybrite.dev/docs/reference/platforms/web-dom "toolkit:web-dom")** | rem-based ramp: the Apple text-style ratios with `Body` = 1, scaled by day.css's `--day-text-scale`: 0.8125 on a desktop pointer, so one Apple point is one CSS pixel and `Body` is 13px like [AppKit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit"); 1 on a touch pointer, where `html` is anchored to `-apple-system-body` and every step lands on the [iOS](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") ramp | Yes; 1rem is the browser's default-font-size preference (day.css pins `html` at `font-size: 100%`), so every style tracks it, and on [iOS](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") the anchor makes it track Dynamic Type; page zoom applies on top |

## Weight & style

- [`.weight(FontWeight::Semibold)`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.weight): `UltraLight, Thin, Light, Regular, Medium, Semibold, Bold, Heavy,
  Black` (matching [`UIFont.Weight`](https://developer.apple.com/documentation/uikit/uifont)). [`.bold()`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.bold) is shorthand for [`.weight(FontWeight::Bold)`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.weight).
- [`.italic()`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.italic): slants the text.
- Canvas text and a drawing app's font menu are a different surface — the platform's font
  list, absolute sizes, measurement — covered in [docs/fonts](fonts.md).
- A weight override keeps the style's accessibility-scaled size (on iOS the weighted font is wrapped in
  [`UIFontMetrics`](https://developer.apple.com/documentation/uikit/uifontmetrics) so it also scales with Dynamic Type).
- [`.monospace()`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.monospace): the platform's fixed-pitch face, at the same semantic size.

- [`.small_caps()`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.small_caps): lowercase letters drawn as small capitals (the OpenType `smcp`
  feature), for an eyebrow heading or an acronym in running text.
- [`.oldstyle_figures()`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.oldstyle_figures): digits that sit in running text like lowercase letters
  (`onum`). It combines with [`.tabular()`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.tabular).

Small capitals and oldstyle figures are features of the font. A face that has no such glyphs
draws its ordinary lowercase and its ordinary figures; no toolkit substitutes scaled-down
capitals. The system faces on Apple platforms, Segoe UI Variable, Roboto Flex and most bundled
text faces carry both.

To vary style *within* one label (a bold phrase, an inline `code` span, a colored word), see
[text-runs](./text-runs.md). It stays one label, so it wraps, selects and is announced as one
paragraph. A whole document with headings, lists and code blocks is the
[Markdown view](./markdown-view.md).

## Decorations

[`.underline(decoration)`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.underline) and [`.strikethrough(decoration)`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.strikethrough) draw a line under or
through the whole label. A `Decoration` is a line style and an optional color:

```rust
label(res::str::old_price()).strikethrough(Decoration::SINGLE)
label(res::str::unresolved_symbol()).underline(Decoration::wavy().color(Color::hex(0xD0021B)))
label(res::str::total()).underline(DecorationStyle::Double)      // a style converts
```

The styles are `Single`, `Double`, `Dotted` and `Wavy`. Without a color the line takes the
text color. For a line under part of the text, use
[`TextBuilder::underline_styled`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextBuilder.html#method.underline_styled)
([text-runs](./text-runs.md)). A toolkit that cannot draw a style draws the nearest line it
has, usually a single one; the table in [text-runs](./text-runs.md#what-each-toolkit-cannot-draw) lists which.

## Stable readout width

Use [`.reserving(sample)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.reserving)
to keep a changing label from shifting nearby controls. Supply the widest expected text,
including its translated units. Pair it with
[`.tabular()`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.tabular)
to give digits equal widths.

The sample accepts the same strings, localized messages, signals, and closures as `label`.
It updates when its text or locale changes. Day measures it in the label's final base font,
including weight, scale, and tabular settings, regardless of modifier order. The visible label
keeps its identity and baseline; the sample is hidden from assistive technology.

Reservation follows a label through single-child layout wrappers. For other content, it measures
the sample in `Font::Body`. It does not copy inline text-run styling or reserve padding around
the sample. Put padding after [`.reserving(...)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.reserving) when the reserved text needs insets.

## Color

[`.color(Color)`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.color) sets the text color; omit it to use the platform's default label color (which adapts to
light/dark). Colors are given as [`Color::hex(0xRRGGBB)`](https://docs.rs/day-geometry/latest/day_geometry/struct.Color.html#method.hex) or [`Color::rgba(r, g, b, a)`](https://docs.rs/day-geometry/latest/day_geometry/struct.Color.html#method.rgba).

[`.secondary()`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.secondary) asks for the platform's de-emphasized label color instead of naming one; it suits a
hint, a caption, or an empty state's "nothing selected" line:

```rust
label(res::str::nothing_selected()).secondary()
```

It is semantic for the same reason `Font::Body` is: a literal grey chosen against a light background
is close to invisible on a dark one, so only the platform can answer correctly in both. Each toolkit
uses its own: `secondaryLabelColor` on macOS and iOS, `?android:attr/textColorSecondary`, GTK's
`dim-label` style class, and on the web a mix of the inherited text color. An explicit [`.color()`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.color)
still wins, and a toolkit with no such color renders the primary one, which is legible but not
dimmed.

## Presets

A style an app applies to many labels is named once as a `TextPreset` and applied with
[`.preset(&NAME)`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.preset):

```rust
const CAPTION: TextPreset = TextPreset::new(Font::Caption).secondary();
const PRICE: TextPreset = TextPreset::new(Font::Headline).tabular().bold();

label(res::str::last_updated()).preset(&CAPTION)
label(res::str::sample_price(price)).preset(&PRICE)
```

Every `TextPreset` builder is `const` ([`small_caps`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextPreset.html#method.small_caps), [`letter_spacing`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextPreset.html#method.letter_spacing), [`line_height`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextPreset.html#method.line_height), [`max_text_scale`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextPreset.html#method.max_text_scale), [`font_scale`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextPreset.html#method.font_scale), [`weight`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextPreset.html#method.weight), [`bold`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextPreset.html#method.bold), [`italic`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextPreset.html#method.italic), [`tabular`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextPreset.html#method.tabular),
[`monospace`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextPreset.html#method.monospace), [`secondary`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextPreset.html#method.secondary), [`single_line`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextPreset.html#method.single_line), [`color`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextPreset.html#method.color)), so a preset is a constant rather than a
runtime value, and [`.preset`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.preset) applies the same steps the equivalent chain would. It replaces a
repeated [`label`](https://docs.rs/day-pieces/latest/day_pieces/fn.label.html)`(t)`[`.font`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.font)`(Font::Caption)`[`.secondary`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.secondary)`()` without adding a style system: the preset
is still semantic fonts and the platform's colors.

## Custom sizes and accessibility

`Font::System(pt)` takes an explicit point size, but it is still scaled by the platform's
accessibility text-size setting (iOS runs it through [`UIFontMetrics`](https://developer.apple.com/documentation/uikit/uifontmetrics), Android uses `sp`, GTK uses the
text-scaling factor, Web expresses it as `pt/16` rem so it rides the browser's font-size
preference), so a hard-coded size never turns into a fixed, unreadable pixel size.

[`Font::custom(res::fonts::family, pt)`](https://docs.rs/day-spec/latest/day_spec/enum.Font.html#method.custom) (and the unchecked `Font::Custom`) scales the same way; a bundled font never opts out of accessibility
sizing.

## Text scale

Semantic fonts follow the reader's text-size setting without any code. Two things are left to
the app: layout decisions that depend on how large the text is, and the few labels that must
not grow without limit.

[`day::text_scale()`](https://docs.rs/day-core/latest/day_core/fn.text_scale.html) answers how much larger than the platform's default the setting draws
body text right now: `1.0` at the default, `1.5` when text is half again as large. It is a
tracked read, so a closure that calls it runs again when the reader changes the setting while
the app is open.

```rust
// Stack the row when the text is large.
when(|| day::text_scale() >= 1.5, || column((title(), value())))
    .otherwise(|| row((title(), spacer(), value())))

// A swatch that grows with the text beside it.
rectangle().fill(tint).frame(|| 12.0 * day::text_scale(), || 12.0 * day::text_scale())
```

[`.max_text_scale(cap)`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.max_text_scale) stops one label growing past `cap` times its default
size: `1.3` lets a tab label or a badge grow 30% and no further, and `1.0` opts it out of
growth. Text set smaller than the default is never affected.

```rust
label(res::str::tab_library()).font(Font::Caption2).max_text_scale(1.3)
label(move || unread.get().to_string()).max_text_scale(1.0)     // a count in a fixed badge
```

Use a cap only where the layout cannot give the text more room. Body text, list rows and
anything a person reads at length should scale without one: a reader who set a large size
needs it most there.

Day applies the cap itself, on every toolkit: it asks the toolkit for the factor in force for
the label's style and scales the label's font down by `cap / factor` while the factor exceeds
the cap. Where the platform scales every style by one number the result is exact. On iOS,
Dynamic Type grows small styles more than large ones, so the toolkit answers per style.

| Toolkit | Where the scale comes from | Follows a change while running |
|---|---|---|
| [UIKit](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") | Dynamic Type: each style's preferred size against its size at the default category, so a small style answers for itself; `System` and `Custom` sizes through `UIFontMetrics` | Yes |
| [AppKit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") | macOS has no text-size setting, so the answer is always `1.0` | Nothing to follow |
| [WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") | Windows' Text size (`UISettings.TextScaleFactor`), one factor for every style | Yes |
| [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") | the font-size setting, per style, since Android 14 grows large sizes less than small ones | Yes: the change recreates the activity, and the new mount reports it |
| [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") | `gtk-xft-dpi` against the default resolution, which GNOME's text-scaling-factor sets; one factor for every style, and `1.0` on macOS | Yes |
| [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") | the screen's logical DPI against the platform's reference (`Xft.dpi`, set by GNOME's text-scaling-factor or KDE's forced font DPI); one factor for every style, and `1.0` on macOS | Yes |
| [ArkUI](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") | the configuration's `fontSizeScale`, one factor for every style | Yes |
| [Web](https://daybrite.dev/docs/reference/platforms/web-dom "toolkit:web-dom") | the browser's default font size against 16 px (17 px under iOS Dynamic Type), one factor for every style | Yes, checked on resize and when the page becomes visible |

Where the platform has no text-size setting the answer is `1.0`, and a cap has no effect there.

## Links

```rust
link("Day", "https://daybrite.dev")
link("Settings", "#settings").bold()
link(res::str::terms(), "https://daybrite.dev/terms").font(Font::Footnote).single_line()
```

[`link(text, target)`](https://docs.rs/day-pieces/latest/day_pieces/fn.link.html) is a label whose whole text is a link, so every label builder applies to
it: the font and weight, a reactive [`.color()`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.color), [`.max_lines()`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.max_lines), [`.truncation()`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.truncation), a
[`.preset()`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.preset). A target starting with `#` calls [`navigate`](https://docs.rs/day-pieces/latest/day_pieces/fn.navigate.html)
with the rest of the target; all other targets open in the platform's default URL handler.
URLs containing a fragment (`https://daybrite.dev/#docs`) still open externally. Route paths,
query parameters, and percent escapes follow the [navigation](./navigation.md) rules. Unknown
routes do nothing; `#` alone pops the innermost stack to its root. [`.on_link(f)`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.on_link) replaces
that default with your handler.

Where the toolkit activates link runs (`Cap::TextLinks`) the text is a native link run, with
the platform's link color, pointer and keyboard behavior and accessibility. Elsewhere it is
accent-colored text with a tap handler and a button role.

[Markdown links](./text-runs.md#links) and [`TextBuilder::link`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextBuilder.html#method.link) use the same [`open_link(target)`](https://docs.rs/day-pieces/latest/day_pieces/fn.open_link.html)
function by default. Unlike a standalone [`link`](https://docs.rs/day-pieces/latest/day_pieces/fn.link.html), inline links require `Cap::TextLinks` for
activation; see the [toolkit support table](./text-runs.md#toolkit-implementations).

## Selectable text

Text is **not** user-selectable by default on any toolkit: a label, a button's caption, and a
table cell all match each platform's native behavior, where static text can't be selected. Opt a
piece in with [`.selectable()`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.selectable):

```rust
label(res::str::sample_order()).selectable() // the reader can select and copy it
```

It applies to the piece's own widget, so on a container it makes every label within selectable
only where the platform affordance cascades (web's `user-select` inherits; the widget-flip
toolkits reach the label the modifier is on). Put it on the label itself for portable behavior.
The modifier routes to [`Toolkit::set_selectable`](https://docs.rs/day-spec/latest/day_spec/trait.Toolkit.html#method.set_selectable), which all eight toolkits implement
for a label:

| Toolkit | Affordance |
|---|---|
| [AppKit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") | [`NSTextField.setSelectable:`](https://developer.apple.com/documentation/appkit/nstextfield) |
| [UIKit](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") | the label is rebuilt as a read-only, non-scrolling [`UITextView`](https://developer.apple.com/documentation/uikit/uitextview) (below) |
| [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") | [`GtkLabel.set_selectable`](https://docs.gtk.org/gtk4/class.Label.html) |
| [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") | [`QLabel`](https://doc.qt.io/qt-6/qlabel.html) `TextSelectableByMouse`/`ByKeyboard` |
| [WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") | [`TextBlock.IsTextSelectionEnabled`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.textblock) |
| [HarmonyOS](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") | `Text` `NODE_TEXT_COPY_OPTION` (long-press → copy) |
| [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") | [`TextView.setTextIsSelectable`](https://developer.android.com/reference/android/widget/TextView) (long-press → copy) |
| [Web](https://daybrite.dev/docs/reference/platforms/web-dom "toolkit:web-dom") | `user-select: text` (the `#day-root` default is `none`) |

`Cap::TextSelectable` in the [coverage matrix](./coverage-matrix.md) is a different question:
whether a [`text_area`](./textarea.md)'s selection can be switched *off*. It says nothing
about labels.

UIKit is the one platform whose label class has no selection support to switch on: UIKit
reserves selection for [`UITextInput`](https://developer.apple.com/documentation/uikit/uitextinput) views, and SwiftUI's selectable `Text` isn't a [`UILabel`](https://developer.apple.com/documentation/uikit/uilabel)
either; it pairs its own text renderer with the system selection UI. Day ships the standard
UIKit emulation of that: `set_selectable` rebuilds the label as a read-only,
non-scrolling [`UITextView`](https://developer.apple.com/documentation/uikit/uitextview) (zero container inset and padding, so it measures like the label),
and day-core re-points the node's handle at the replacement so text/font/color updates and
layout keep flowing. The reader gets the platform's real selection: long-press or double-tap,
grabbers, and the Copy/Look Up edit menu.

Selection visuals and the copy shortcut are the platform's own. It's unmanaged: set once at mount,
and it survives Day's text updates.

### Hearing about the selection

[`.on_selection(f)`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.on_selection) makes a label selectable and calls `f` when the reader's selection
changes, for a "Quote selection" button or a lookup panel beside the text:

```rust
let quoted = Signal::new(String::new());
label(move || article.get())
    .on_selection(move |selection| quoted.set(selection.text.clone()))
```

The callback gets a [`SelectedText`](https://docs.rs/day-pieces/latest/day_pieces/struct.SelectedText.html): the selected text as the label displays it (Markdown markers
already stripped) and its byte range in that text. Both are empty when the selection is
cleared. [`capability(Cap::LabelSelection)`](https://docs.rs/day-core/latest/day_core/fn.capability.html) says whether the toolkit reports selections. Where
it does not, the label is still selectable and the callback never runs.

| Toolkit | `Cap::LabelSelection` | Notification |
|---|---|---|
| [AppKit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") | `Native` | the field editor's `textViewDidChangeSelection:` |
| [UIKit](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") | `Native` | `textViewDidChangeSelection:` on the label's `UITextView` |
| [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") | `Native` | `gtk_label_get_selection_bounds`, read back after each pointer, key and focus event, since GTK 4's label notifies no selection change; a Select All from the label's context menu is reported at the next event on the label |
| [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") | `Native` | `QLabel::selectionStart` and `selectedText`, compared after each mouse, key and focus event |
| [WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") | `Native` | `TextBlock.SelectionChanged` |
| [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") | `Native` | the `TextView`'s `onSelectionChanged`, one report per main-loop turn |
| [ArkUI](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") | `Unsupported` | none: the C API has no selection event for text at API 18 |
| [Web](https://daybrite.dev/docs/reference/platforms/web-dom "toolkit:web-dom") | `Native` | the document's `selectionchange` |

The showcase's **Text** page is a live specimen of every style, weight, italic, color, custom size,
the three bundled custom fonts, and links, with a **Selectable** toggle in the heading's corner
that opts every text piece on the page in and out of [`.selectable()`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.selectable).

## Line limits and truncation

A label wraps onto as many lines as its text needs. Two builders hold it to fewer:

```rust
label(title).single_line()                                  // one line
label(summary).max_lines(3)                                 // at most three
label(summary).max_lines(move || if expanded.get() { 0 } else { 3 })
label(path).single_line().truncation(Truncation::Middle)    // /usr/loc…/pages.rs
```

[`.max_lines(n)`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.max_lines) shows at most `n` lines, and zero means unlimited. The label measures no
taller than `n` lines, so the layout around it gets the room back, and the full text stays
available to accessibility and to `assert_text`. It takes a constant, a `Signal<u32>` or a
closure, so a "Show more" control changes the limit in place. [`.single_line()`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.single_line) keeps the
text on one line whatever the limit says.

[`.truncation(mode)`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.truncation) says where the ellipsis goes when text is left over:

| Mode | Keeps | For |
|---|---|---|
| `Truncation::Tail` (default) | the start | running text |
| `Truncation::Head` | the end | text whose end identifies it |
| `Truncation::Middle` | both ends | a file path, an identifier, a long file name with its extension |

`Head` and `Middle` are guaranteed on a one-line label ([`single_line()`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.single_line) or
[`max_lines(1)`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.max_lines)).
With more lines a toolkit applies them to the last line where the platform can and truncates
the tail where it cannot. The mode is reactive too.

Constrain the width through the enclosing layout: a label in a `row` takes the width its text
wants unless it grows or is framed.

Line limits and truncation are a duty of every toolkit, proven by the `label-max-lines` and
`label-truncation-modes` conformance cases. The first holds a long label to two lines and
then one, and passes only if it measures exactly as tall as a two-line and a one-line label
beside it.

| Toolkit | Line limit | Head and middle on one line | Head and middle on several lines |
|---|---|---|---|
| [AppKit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") | `maximumNumberOfLines` | the cell's line break mode | tail: AppKit lays a head- or middle-truncated paragraph on one line |
| [UIKit](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") | `numberOfLines`, or the text container's `maximumNumberOfLines` for a selectable or linked label | `lineBreakMode` | on the last line |
| [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") | `gtk_label_set_lines` with an ellipsis; the limit counts lines broken at `\n` too, and a cut that falls at a `\n` leaves the last line without an ellipsis | Pango's start and middle ellipsis | on the last line |
| [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") | Day shows the lines that fit and elides the last one (`QFontMetricsF::elidedText`) | Day's elision | on the last line; a label with runs truncates the tail |
| [WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") | `TextBlock.MaxLines` with a character ellipsis | Day elides the text; a label with runs truncates the tail | tail |
| [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") | `setMaxLines` | `TruncateAt.START` and `MIDDLE` on a single-line label | tail |
| [ArkUI](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") | `NODE_TEXT_MAX_LINES` | `NODE_TEXT_ELLIPSIS_MODE` | tail |
| [Web](https://daybrite.dev/docs/reference/platforms/web-dom "toolkit:web-dom") | `line-clamp` | Day elides the text; a label with runs truncates the tail | tail |

## Letter spacing and line height

```rust
label(res::str::section_eyebrow()).small_caps().letter_spacing(0.08)
label(res::str::article_body()).line_height(1.5)
label(res::str::hero_title()).font(Font::LargeTitle).letter_spacing(-0.02).line_height(1.05)
```

Both are multiples of the label's font size, so they follow the reader's text size with the
font:

- [`.letter_spacing(ems)`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.letter_spacing) adds space after each character: `0.05` is 5% of the font
  size, and a negative value tightens. Zero is the font's own spacing.
- [`.line_height(multiple)`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.line_height) sets the distance between baselines: `1.5` on a 16-point
  font is 24 points. The font's natural line height (about `1.2`) applies until this is set
  and again for zero. The extra space is split above and below each line, so a one-line label
  grows too and its text stays centered in the line box.

Each takes a constant, a signal or a closure, and a change re-measures the label. Line height
has one form, the multiple, because an absolute height in points would stop following the
reader's text size while the font kept growing.

The `label-line-height` and `label-letter-spacing` conformance cases prove the arithmetic on
each toolkit in ems of the font as drawn, so they hold wherever a point is more than one unit:
two lines at a line height of 2.0 are as tall as three at 4/3, and three characters of a
40-point font spaced by 0.75 em widen by as much as three of a 20-point font spaced by 1.5 em.

| Toolkit | Letter spacing | Line height |
|---|---|---|
| [AppKit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") | `NSKern` | an `NSParagraphStyle` with equal minimum and maximum line heights, the glyphs centered by `NSBaselineOffset` |
| [UIKit](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") | `NSKern` | an `NSParagraphStyle` with equal minimum and maximum line heights, the glyphs centered by `NSBaselineOffset` |
| [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") | Pango's `letter_spacing` | Pango's absolute line height. Both multiply the font as Pango draws it, points times the resolution over 72, so on Linux at 96 dpi a 20-point label is a 26.7-point font and its spacing and line height grow with it |
| [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") | `QFont` absolute letter spacing, in ems of the font's pixel size (on Linux a 20-point font is 26.7 pixels at 96 dpi) | a rich-text paragraph with a fixed line height, which puts each baseline at 4/5 of the line box, close to the centered split |
| [WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") | `TextBlock.CharacterSpacing` | `TextBlock.LineHeight` with `BlockLineHeight` stacking |
| [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") | `setLetterSpacing` | a line-height span that centers each line in its box |
| [ArkUI](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") | `NODE_TEXT_LETTER_SPACING` | `NODE_TEXT_LINE_HEIGHT` with `NODE_TEXT_HALF_LEADING` |
| [Web](https://daybrite.dev/docs/reference/platforms/web-dom "toolkit:web-dom") | CSS `letter-spacing` in `em` | a unitless CSS `line-height` |

## Font scale

Use [`.font_scale(1.4)`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.font_scale) to enlarge a label's resolved native font by 40%, retaining
its semantic style and accessibility scaling. It takes a constant, a signal or a closure, so a
reader-size control moves every label bound to it without rebuilding them. Explicit styled
runs keep their own font descriptors. Nonpositive or nonfinite factors use 1.0. This also
works through label decorations.

## Alignment

[`.align(TextAlign::Center)`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.align) centers a wrapped label's lines within its own width; `Trailing`
puts them at the reading direction's end edge, and `Justified` stretches every line but the
last to both edges. A toolkit that cannot justify draws leading-aligned text. `TextAlign` is
the one alignment type: a canvas text anchor and a styled document's paragraphs use it too.

## Toolkit support

Everything above works on every toolkit except where the tables above or the rows below say
otherwise.

| Toolkit | Limitation |
|---|---|
| [AppKit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") | A plain label with a letter spacing or a line height is drawn from an attributed string, so in a selected table row it keeps its own color rather than taking the selection's. |
| [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") | A justified label's last line follows the paragraph's own direction, so left-to-right text in a right-to-left subtree ends at the left. |
| [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") | `TextAlign::Justified` needs Android 8 (API 26); below it the text is leading-aligned. |
| [ArkUI](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") | [`.on_selection`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.on_selection) never reports, though the label is still selectable (`Cap::LabelSelection` answers `Unsupported`). |

## Platform guidance

- [Apple: typography](https://developer.apple.com/design/human-interface-guidelines/typography)
- [Windows: typography](https://learn.microsoft.com/en-us/windows/apps/design/style/typography)
