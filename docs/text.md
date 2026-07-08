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

Largest → smallest, mirroring SwiftUI's `Font.TextStyle`:

`LargeTitle`, `Title`, `Title2`, `Title3`, `Headline`, `Subheadline`, `Body` (default), `Callout`,
`Footnote`, `Caption`, `Caption2`. Plus `System(pt)` for a custom point size.

Each maps to the platform's native text style where one exists, so sizes and weights match the OS:

| Backend | Semantic style | Accessibility scaling |
|---|---|---|
| **UIKit** (iOS) | `UIFont.preferredFont(forTextStyle:)` (Dynamic Type) | Yes, live, via `adjustsFontForContentSizeCategory` (Settings ▸ Accessibility ▸ Larger Text) |
| **AppKit** (macOS) | `NSFont.preferredFont(forTextStyle:)` | Follows the system font settings |
| **WinUI** | point sizes (aligned to the desktop scale) | Yes, `FontSize` tracks the OS text-scale-factor |
| **Android** | `sp` sizes (mobile scale, aligned to iOS) | Yes, `sp` tracks Settings ▸ Display ▸ Font size |
| **GTK** | Pango point sizes | Yes, Pango sizes track GNOME's text-scaling-factor |
| **Qt** | QFont point sizes | Honors the system DPI/font (no separate large-text toggle) |

## Weight & style

- `.weight(FontWeight::Semibold)`: `UltraLight, Thin, Light, Regular, Medium, Semibold, Bold, Heavy,
  Black` (matching `UIFont.Weight`). `.bold()` is shorthand for `.weight(FontWeight::Bold)`.
- `.italic()`: slants the text.
- A weight override keeps the style's accessibility-scaled size (on iOS the weighted font is wrapped in
  `UIFontMetrics` so it also scales with Dynamic Type).

## Color

`.color(Color)` sets the text color; omit it to use the platform's default label color (which adapts to
light/dark). Colors are given as `Color::hex(0xRRGGBB)` or `Color::rgba(r, g, b, a)`.

## Custom sizes and accessibility

`Font::System(pt)` takes an explicit point size, but it is still scaled by the platform's
accessibility text-size setting (iOS runs it through `UIFontMetrics`, Android uses `sp`, GTK uses the
text-scaling factor), so a hard-coded size never turns into a fixed, unreadable pixel size.

The showcase's **Text** page is a live specimen of every style, weight, italic, color, and custom size.
