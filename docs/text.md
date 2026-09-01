---
title: "Text & typography"
description: "Labels, symbolic fonts, dynamic type, selection, and the text pipeline on every backend."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Text & typography

`label(...)` renders native text. Its font is chosen from a **semantic (logical) style** that maps to
each platform's own text styles, so a Day app matches the OS's typography and inherits its accessibility
text scaling automatically.

```rust
use day::prelude::*;

label("Chapter One").font(Font::Title)                 // a semantic style
label("A quiet caption").font(Font::Footnote)
label("Read me").font(Font::Body).bold()               // weight
label("Whispered").italic()                            // slant
label(tr("price")).color(Color::hex(0x27AE60))         // color
label("Everything")
    .font(Font::Title2)
    .weight(FontWeight::Heavy)
    .italic()
    .color(Color::hex(0x8E44AD))
label("18 pt").font(Font::System(18.0))                // a custom size (still accessibility-scaled)
```

## Semantic styles (`Font`)

The styles, largest to smallest, mirror SwiftUI's `Font.TextStyle`:

`LargeTitle`, `Title`, `Title2`, `Title3`, `Headline`, `Subheadline`, `Body` (default), `Callout`,
`Footnote`, `Caption`, `Caption2`. Plus `System(pt)` for a custom point size, and
`Custom("Family", pt)` for a bundled custom font referenced by family name (ship the file in the
project's `resource/fonts/` directory; see [docs/resources.md](resources.md) §18.4 for the packaging and naming rules).

Each maps to the platform's native text style where one exists, so sizes and weights match the OS:

| Backend | Semantic style | Accessibility scaling |
|---|---|---|
| **UIKit** (iOS) | `UIFont.preferredFont(forTextStyle:)` (Dynamic Type) | Yes, live, via `adjustsFontForContentSizeCategory` (Settings ▸ Accessibility ▸ Larger Text) |
| **AppKit** (macOS) | `NSFont.preferredFont(forTextStyle:)` | Follows the system font settings |
| **XAML** | point sizes (aligned to the desktop scale) | Yes, `FontSize` tracks the OS text-scale-factor |
| **Android** | `sp` sizes (mobile scale, aligned to iOS) | Yes, `sp` tracks Settings ▸ Display ▸ Font size |
| **GTK** | Pango point sizes | Yes, Pango sizes track GNOME's text-scaling-factor |
| **Qt** | QFont point sizes | Honors the system DPI/font (no separate large-text toggle) |
| **Web** (`web-dom`) | rem-based ramp: the Apple text-style ratios with `Body` = 1, scaled by day.css's `--day-text-scale`: 0.8125 on a desktop pointer, so one Apple point is one CSS pixel and `Body` is 13px like AppKit; 1 on a touch pointer, where `html` is anchored to `-apple-system-body` and every step lands on the iOS ramp | Yes; 1rem is the browser's default-font-size preference (day.css pins `html` at `font-size: 100%`), so every style tracks it, and on iOS the anchor makes it track Dynamic Type; page zoom applies on top |

## Weight & style

- `.weight(FontWeight::Semibold)`: `UltraLight, Thin, Light, Regular, Medium, Semibold, Bold, Heavy,
  Black` (matching `UIFont.Weight`). `.bold()` is shorthand for `.weight(FontWeight::Bold)`.
- `.italic()`: slants the text.
- A weight override keeps the style's accessibility-scaled size (on iOS the weighted font is wrapped in
  `UIFontMetrics` so it also scales with Dynamic Type).
- `.monospace()`: the platform's fixed-pitch face, at the same semantic size.

To vary style *within* one label (a bold phrase, an inline `code` span, a colored word), see
[text-runs.md](./text-runs.md). It stays one label, so it wraps, selects and is announced as one
paragraph.

## Color

`.color(Color)` sets the text color; omit it to use the platform's default label color (which adapts to
light/dark). Colors are given as `Color::hex(0xRRGGBB)` or `Color::rgba(r, g, b, a)`.

`.secondary()` asks for the platform's de-emphasized label color instead of naming one; it suits a
hint, a caption, or an empty state's "nothing selected" line:

```rust
label(tr("nothing_selected")).secondary()
```

It is semantic for the same reason `Font::Body` is: a literal grey chosen against a light background
is close to invisible on a dark one, so only the platform can answer correctly in both. Each backend
uses its own: `secondaryLabelColor` on macOS and iOS, `?android:attr/textColorSecondary`, GTK's
`dim-label` style class, and on the web a mix of the inherited text color. An explicit `.color()`
still wins, and a backend with no such color renders the primary one, which is legible but not
dimmed.

## Custom sizes and accessibility

`Font::System(pt)` takes an explicit point size, but it is still scaled by the platform's
accessibility text-size setting (iOS runs it through `UIFontMetrics`, Android uses `sp`, GTK uses the
text-scaling factor, web-dom expresses it as `pt/16` rem so it rides the browser's font-size
preference), so a hard-coded size never turns into a fixed, unreadable pixel size.

`Font::custom(res::fonts::family, pt)` (and the unchecked `Font::Custom`) scales the same way; a bundled font never opts out of accessibility
sizing.

## Selectable text

Text is **not** user-selectable by default on any backend: a label, a button's caption, and a
table cell all match each platform's native behavior, where static text can't be selected. Opt a
piece in with `.selectable()`:

```rust
label("Order #A1B2-C3D4").selectable()   // the reader can select and copy it
```

It applies to the piece's own widget, so on a container it makes every label within selectable
only where the platform affordance cascades (web's `user-select` inherits; the widget-flip
backends reach the label the modifier is on). Put it on the label itself for portable behavior.
The modifier routes to `Toolkit::set_selectable`:

| Backend | Affordance |
|---|---|
| AppKit | `NSTextField.setSelectable:` |
| UIKit | the label is rebuilt as a read-only, non-scrolling `UITextView` (below) |
| GTK | `GtkLabel.set_selectable` |
| Qt | `QLabel` `TextSelectableByMouse`/`ByKeyboard` |
| XAML | `TextBlock.IsTextSelectionEnabled` |
| HarmonyOS | `Text` `NODE_TEXT_COPY_OPTION` (long-press → copy) |
| Android | `TextView.setTextIsSelectable` (long-press → copy) |
| web-dom | `user-select: text` (the `#day-root` default is `none`) |

UIKit is the one platform whose label class has no selection support to switch on: UIKit
reserves selection for `UITextInput` views, and SwiftUI's selectable `Text` isn't a `UILabel`
either; it pairs its own text renderer with the system selection UI. Day ships the standard
UIKit emulation of that: `set_selectable` rebuilds the label as a read-only,
non-scrolling `UITextView` (zero container inset and padding, so it measures like the label),
and day-core re-points the node's handle at the replacement so text/font/color updates and
layout keep flowing. The reader gets the platform's real selection: long-press or double-tap,
grabbers, and the Copy/Look Up edit menu.

Selection visuals and the copy shortcut are the platform's own. It's unmanaged: set once at mount,
and it survives Day's own text updates.

The showcase's **Text** page is a live specimen of every style, weight, italic, color, custom size,
the three bundled custom fonts, and links, with a **Selectable** toggle in the heading's corner
that opts every text piece on the page in and out of `.selectable()`.
