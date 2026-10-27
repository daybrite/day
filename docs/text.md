---
navTitle: "Text"
title: "Text & typography"
description: "Labels, symbolic fonts, dynamic type, selection, and the text pipeline on every toolkit."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Text & typography

[`label(...)`](https://docs.rs/day-pieces/latest/day_pieces/fn.label.html) renders native text. Its font is chosen from a **semantic (logical) style** that maps to
each platform's text styles, so a Day app matches the OS's typography and inherits its accessibility
text scaling automatically.

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
| **UIKit** (iOS) | [`UIFont.preferredFont(forTextStyle:)`](https://developer.apple.com/documentation/uikit/uifont) (Dynamic Type) | Yes, live, via `adjustsFontForContentSizeCategory` (Settings ▸ Accessibility ▸ Larger Text) |
| **AppKit** (macOS) | [`NSFont.preferredFont(forTextStyle:)`](https://developer.apple.com/documentation/appkit/nsfont) | Follows the system font settings |
| **WinUI** | point sizes (aligned to the desktop scale) | Yes, `FontSize` tracks the OS text-scale-factor |
| **Android** | `sp` sizes (mobile scale, aligned to iOS) | Yes, `sp` tracks Settings ▸ Display ▸ Font size |
| **GTK** | Pango point sizes | Yes, Pango sizes track GNOME's text-scaling-factor |
| **Qt** | QFont point sizes | Honors the system DPI/font (no separate large-text toggle) |
| **Web** | rem-based ramp: the Apple text-style ratios with `Body` = 1, scaled by day.css's `--day-text-scale`: 0.8125 on a desktop pointer, so one Apple point is one CSS pixel and `Body` is 13px like AppKit; 1 on a touch pointer, where `html` is anchored to `-apple-system-body` and every step lands on the iOS ramp | Yes; 1rem is the browser's default-font-size preference (day.css pins `html` at `font-size: 100%`), so every style tracks it, and on iOS the anchor makes it track Dynamic Type; page zoom applies on top |

## Weight & style

- [`.weight(FontWeight::Semibold)`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.weight): `UltraLight, Thin, Light, Regular, Medium, Semibold, Bold, Heavy,
  Black` (matching [`UIFont.Weight`](https://developer.apple.com/documentation/uikit/uifont)). [`.bold()`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.bold) is shorthand for [`.weight(FontWeight::Bold)`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.weight).
- [`.italic()`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.italic): slants the text.
- Canvas text and a drawing app's font menu are a different surface — the platform's font
  list, absolute sizes, measurement — covered in [docs/fonts](fonts.md).
- A weight override keeps the style's accessibility-scaled size (on iOS the weighted font is wrapped in
  [`UIFontMetrics`](https://developer.apple.com/documentation/uikit/uifontmetrics) so it also scales with Dynamic Type).
- [`.monospace()`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.monospace): the platform's fixed-pitch face, at the same semantic size.

To vary style *within* one label (a bold phrase, an inline `code` span, a colored word), see
[text-runs](./text-runs.md). It stays one label, so it wraps, selects and is announced as one
paragraph.

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

Every `TextPreset` builder is `const` ([`font_scale`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextPreset.html#method.font_scale), [`weight`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextPreset.html#method.weight), [`bold`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextPreset.html#method.bold), [`italic`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextPreset.html#method.italic), [`tabular`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextPreset.html#method.tabular),
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

## Links

```rust
link("Day", "https://daybrite.dev")
link("Settings", "#settings").bold()
```

[`link(text, target)`](https://docs.rs/day-pieces/latest/day_pieces/fn.link.html) renders tappable accent text. A target starting with `#` calls [`navigate`](https://docs.rs/day-pieces/latest/day_pieces/fn.navigate.html)
with the rest of the target; all other targets open in the platform's default URL handler.
URLs containing a fragment (`https://daybrite.dev/#docs`) still open externally. Route paths,
query parameters, and percent escapes follow the [navigation](./navigation.md) rules. Unknown
routes do nothing; `#` alone pops the innermost stack to its root.

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
The modifier routes to [`Toolkit::set_selectable`](https://docs.rs/day-spec/latest/day_spec/trait.Toolkit.html#method.set_selectable):

| Toolkit | Affordance |
|---|---|
| AppKit | [`NSTextField.setSelectable:`](https://developer.apple.com/documentation/appkit/nstextfield) |
| UIKit | the label is rebuilt as a read-only, non-scrolling [`UITextView`](https://developer.apple.com/documentation/uikit/uitextview) (below) |
| GTK | [`GtkLabel.set_selectable`](https://docs.gtk.org/gtk4/class.Label.html) |
| Qt | [`QLabel`](https://doc.qt.io/qt-6/qlabel.html) `TextSelectableByMouse`/`ByKeyboard` |
| WinUI | [`TextBlock.IsTextSelectionEnabled`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.textblock) |
| HarmonyOS | `Text` `NODE_TEXT_COPY_OPTION` (long-press → copy) |
| Android | [`TextView.setTextIsSelectable`](https://developer.android.com/reference/android/widget/TextView) (long-press → copy) |
| Web | `user-select: text` (the `#day-root` default is `none`) |

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

The showcase's **Text** page is a live specimen of every style, weight, italic, color, custom size,
the three bundled custom fonts, and links, with a **Selectable** toggle in the heading's corner
that opts every text piece on the page in and out of [`.selectable()`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.selectable).

## Single-line labels

[`label`](https://docs.rs/day-pieces/latest/day_pieces/fn.label.html)`(title)`[`.single_line`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.single_line)`()` keeps plain text on one line as its text changes, including
recycled list rows. Constrain its width through the enclosing layout. The full text remains
available to accessibility. UIKit, AppKit, Android, GTK, DOM, WinUI and ArkUI request native
trailing ellipsis; Qt disables wrapping and clips the excess. Styled link labels may use a
text-view backing and do not guarantee this truncation behavior.

Plain labels support [`.max_lines(n)`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.max_lines) with zero meaning unlimited; [`.single_line()`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.single_line)
takes precedence. AppKit and UIKit enforce the limit and truncate the last line natively,
including during measurement. Other toolkits and selectable/link-backed text retain their
existing wrapping behavior. The limit is set at construction; a keyed subtree can rebuild
when a display preference changes.

Use [`.font_scale(1.4)`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.font_scale) on a plain label to enlarge its resolved native font by 40%, retaining
its semantic style and accessibility scaling. The factor is fixed at construction, and
explicit styled runs keep their own font descriptors. Nonpositive or nonfinite factors
use 1.0. This also works through label decorations.

## Platform guidance

- [Apple: typography](https://developer.apple.com/design/human-interface-guidelines/typography)
- [Windows: typography](https://learn.microsoft.com/en-us/windows/apps/design/style/typography)
