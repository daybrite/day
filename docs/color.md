---
navTitle: "Colors"
title: "Colors and gradients"
description: "Fixed colors, light/dark pairs and platform roles in one value, gradient fills, and how a view follows the appearance."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Colors and gradients

Color can distinguish content, show state, and draw attention to an action. Text, shapes, or other
cues need to carry the same meaning when color alone would be hard to distinguish.

Day has two color types. [`Color`](https://docs.rs/day-geometry/latest/day_geometry/struct.Color.html)
is a concrete sRGB value an app computes with. [`Ink`](https://docs.rs/day-spec/latest/day_spec/ink/enum.Ink.html)
is what a view is drawn in: a fixed `Color`, a light/dark pair, or a platform role. Every color
parameter of a view takes `impl Into<Ink>`, so all three go anywhere a color does, and the toolkit
follows the appearance with no app code running.

```rust
const INSET: Ink = Ink::adaptive(Color::rgba(0.0, 0.0, 0.0, 0.045), Color::rgba(0.0, 0.0, 0.0, 0.26));

label(res::str::sale())
    .padding(Space::M)
    .background(INSET)                          // a pair
    .border(SystemColor::Separator, 1.0)        // a role
    .color(Color::hex(0x00e8_6a3c))             // a fixed color
```

## Solid colors

`Color` stores red, green, blue, and alpha as floating-point sRGB components. Constructors include
[`rgb`](https://docs.rs/day-geometry/latest/day_geometry/struct.Color.html#method.rgb),
[`rgba`](https://docs.rs/day-geometry/latest/day_geometry/struct.Color.html#method.rgba),
[`hex`](https://docs.rs/day-geometry/latest/day_geometry/struct.Color.html#method.hex),
[`hsl`](https://docs.rs/day-geometry/latest/day_geometry/struct.Color.html#method.hsl), and
[`hsv`](https://docs.rs/day-geometry/latest/day_geometry/struct.Color.html#method.hsv).
All produce the same RGBA representation.

[`with_alpha`](https://docs.rs/day-geometry/latest/day_geometry/struct.Color.html#method.with_alpha) changes
opacity. [`parse`](https://docs.rs/day-geometry/latest/day_geometry/struct.Color.html#method.parse) accepts
`#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, and the space-separated component form produced by
`Display`. [`to_hex_string`](https://docs.rs/day-geometry/latest/day_geometry/struct.Color.html#method.to_hex_string)
formats a hex value. For interpolation,
[`lerp_hsl`](https://docs.rs/day-geometry/latest/day_geometry/struct.Color.html#method.lerp_hsl) follows the
short hue arc, and `day::motion` blends in OKLab ([tween.md](tween.md) "Lerp").

## Inks

| Spelling | Meaning |
| --- | --- |
| [`Color::rgb(..)`](https://docs.rs/day-geometry/latest/day_geometry/struct.Color.html#method.rgb), `Ink::Fixed(c)` | The same color in every appearance. |
| [`Ink::adaptive(light, dark)`](https://docs.rs/day-spec/latest/day_spec/ink/enum.Ink.html#method.adaptive) | `light` in the light appearance, `dark` in the dark one. |
| `SystemColor::Separator`, [`Ink::system(role)`](https://docs.rs/day-spec/latest/day_spec/ink/enum.Ink.html#method.system) | The platform's color for the role. |
| [`SystemColor::Accent.with_alpha(0.14)`](https://docs.rs/day-spec/latest/day_spec/ink/enum.SystemColor.html#method.with_alpha), [`ink.with_alpha(a)`](https://docs.rs/day-spec/latest/day_spec/ink/enum.Ink.html#method.with_alpha) | The same ink at another opacity, in every appearance. |

Every constructor is `const`, so an app's palette is a set of constants.
[`Ink::with_alpha`](https://docs.rs/day-spec/latest/day_spec/ink/enum.Ink.html#method.with_alpha) sets
the alpha of both colors of a pair, and replaces a role's own alpha (a translucent role such as
`Separator` becomes exactly `a`). Inks compare with `==`: two pairs are equal when their colors
are, and a role equals the same role at the same alpha. An `Ink` is not `Hash`, since it holds
floats.

[`Ink::map(f)`](https://docs.rs/day-spec/latest/day_spec/ink/enum.Ink.html#method.map) derives an
ink by applying `f` to its color in each appearance: the readable text color on a fill, a darker
shade of a brand color. A role is resolved first, so the result is a pair.

### Where an ink goes

Every color a view takes: [`.background`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.background), a `Surface`'s fill, border and shadow,
[`.border`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.border), [`.shadow`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.shadow), a label's
[`.color`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.color),
[`TextPreset::color`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextPreset.html#method.color), a run's color and highlight, a
decoration's color, a button's
[`.tint`](https://docs.rs/day-pieces/latest/day_pieces/trait.ButtonBuilder.html#tymethod.tint), image and vector tints, list row icon and badge tints, swipe action
tints, a cover's
[`.background`](https://docs.rs/day-pieces/latest/day_pieces/trait.CoverBuilder.html#tymethod.background), a shape's `fill` and `stroke`, a gradient's stops, and every
canvas paint. A `Color`, a `SystemColor`, an `Ink`, a signal of any of them, and a closure that
returns any of them are all accepted.

### Resolving an ink

Code that needs the numbers resolves an ink with
[`day::resolve(ink)`](https://docs.rs/day-app/latest/day/fn.resolve.html): a contrast
computation, a color mixed from a role, a chart's palette. The read is tracked, so a draw closure,
a reactive modifier or an effect that calls it runs again when the appearance, the accent or the
contrast setting changes. [`day::palette()`](https://docs.rs/day-app/latest/day/fn.palette.html)
returns every role at once. A view never needs either: pass it the ink.

A canvas resolves its inks as it records ([canvas.md](canvas.md) "Colors"), and records again when the
appearance changes.

## System colors

A [`SystemColor`](https://docs.rs/day-spec/latest/day_spec/ink/enum.SystemColor.html) is a role, which
each toolkit answers with its own color, following the appearance, the user's accent and the
contrast setting. Where a platform has no such color, the role is the light/dark pair in
[`SystemColor::fallback`](https://docs.rs/day-spec/latest/day_spec/ink/enum.SystemColor.html#method.fallback).

| Role | What it is for |
| --- | --- |
| `Accent` | The app's accent: selection, the prominent button, a focus ring. |
| `Text` | Primary text. |
| `SecondaryText` | De-emphasized text: a caption, a detail line. |
| `TertiaryText` | Text a step quieter than secondary: a footnote, a hint. |
| `Placeholder` | A text field's placeholder. |
| `Link` | A link in running text. |
| `Separator` | A hairline: a rule, a list separator, a grid line. |
| `ControlFill` | A control's track or well: a slider track, a search field. |
| `Background` | The window's content background. |
| `Surface` | A region set off from the background: a panel, a card, a sidebar. |
| `GroupedBackground` | The background behind grouped sections. |
| `ElevatedSurface` | A surface raised above the content: a popover, a menu, a sheet. |
| `Selection` | The highlight behind selected text or content the app draws itself. |
| `Destructive` | A destructive action or an error. |
| `Success` | Success, a value that rose. |
| `Warning` | A warning, something to notice. |

Default text, controls, separators, and window backgrounds already use the toolkit's colors.
[Forms](/docs/reference/forms) use the platform's section-card surface, and [`.card()`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.card) draws the
grouped-content material. A role is for what an app draws itself.

## One fill vocabulary

[`Paint`](https://docs.rs/day-spec/latest/day_spec/enum.Paint.html) is what fills an area: a solid
`Ink`, or a `LinearGradient`, `RadialGradient` or `ConicGradient`. Every fill takes
`impl Into<Paint>`, so a color, a role, an ink and a gradient go into each of them:

| Where | Call |
| --- | --- |
| A view | [`.background(paint)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.background) |
| A surface | [`Surface::fill(paint)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Surface.html#method.fill) |
| A shape | [`.fill(paint)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ShapePieceBuilder.html#tymethod.fill) |
| A canvas | [`d.fill(shape, paint)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Draw.html#method.fill), [`d.stroke_styled(shape, paint, style)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Draw.html#method.stroke_styled) |

Gradient positions use `UnitPoint` coordinates relative to the filled bounds, and a radial
gradient stretches with non-square bounds. A gradient's stops take inks, so
[`LinearGradient::vertical(SystemColor::Surface, SystemColor::Background)`](https://docs.rs/day-spec/latest/day_spec/struct.LinearGradient.html#method.vertical)
follows the appearance like a solid fill. Its constructors take any iterable of
`(offset, color)` pairs:
[`LinearGradient::new(start, end, [(0.0, a), (1.0, b)])`](https://docs.rs/day-spec/latest/day_spec/struct.LinearGradient.html#method.new).

## How a toolkit follows the appearance

A view keeps the ink, not a color resolved from it, and the toolkit makes the switch:

- **AppKit, UIKit**: one dynamic `NSColor` / `UIColor` per ink, resolved by the system for the
  view's own appearance, so a window or a subtree with its own appearance is right too.
- **Web**: a role is a CSS custom property of the page and a pair is `light-dark(..)`; the
  browser switches them with `color-scheme`.
- **Android**: a light/dark switch recreates the activity, and the tree mounted again resolves
  every ink against the new theme.
- **GTK**: a role is a libadwaita color in the widget's CSS, which GTK switches itself; the
  toolkit re-applies pairs and the colors of label runs when the theme changes.
- **Qt, WinUI, ArkUI**: the toolkit re-applies every ink it has applied when the theme
  changes.

Day keeps the current [`Palette`](https://docs.rs/day-spec/latest/day_spec/ink/struct.Palette.html)
(every role as the toolkit answers it) and refreshes it on every appearance change, so a toolkit
that applies an ink itself resolves it with [`Ink::resolved()`](https://docs.rs/day-spec/latest/day_spec/ink/enum.Ink.html#method.resolved) against the right palette. No piece
is rebuilt and no closure runs.

## Inks in motion

`Ink` implements `Lerp`, so `Animated<Ink>` and a background animation work with pairs and roles.
Two fixed colors blend in OKLab. A pair blends each side, so a value mid-flight still follows an
appearance switch. A role is resolved against both appearances first. The endpoints are the inks
themselves: at the end of an animation to `SystemColor::Accent`, the view holds the role again.

## Color picker values

A [color picker](/docs/reference/colorpicker) binds a `Color`, including alpha when enabled: a
picked value is a concrete color, not an ink. It does not return a gradient. The portable value
does not retain an ICC profile, a named system color, an image pattern, or the color model used to
enter it.

Converting a wide-gamut color to sRGB can lose information. RGB also cannot preserve a user's
chosen hue when saturation or brightness is zero. The composed picker retains that editing
state while its panel is open; the bound value remains RGBA. AppKit pattern colors that cannot
be represented are ignored.

## Toolkit support

| Toolkit | Limitation |
| --- | --- |
| [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") | A light/dark switch recreates the activity: the app's `root()` runs again, root-scope state and the current route survive, and state held below the root is built again. |
| [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") | `Success` and `Warning` have no Material 3 role and are Day's light/dark pairs. |
| [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") | Day's own appearance override needs Android 12 (API 31); below it `Cap::Appearance` answers `Unsupported` and the app follows the system setting. |
| [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") | Qt has no dynamic colors, so after a palette or color-scheme change Day re-applies every ink from a per-widget table on the next event-loop turn (at once for Day's own appearance override). |
| [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") | Day's own appearance override needs Qt 6.8. |
| [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") | Under a style with no light/dark scheme, the dark appearance is a Fusion palette Day builds. |
| [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") | `Destructive`, `Success` and `Warning` are Day's light/dark pairs, and `ElevatedSurface` is the palette's `Base`. |
| [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") | The appearance is app-wide: a window cannot have its own. |
| [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") | On macOS the natively styled parts follow Day's appearance override a moment after Day's own drawing. |
| [WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") | XAML has no dynamic colors outside a `{ThemeResource}`, so after a system or app appearance change Day re-applies every ink from a per-view table, each for the appearance of the window its view is in. |
| [WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") | The roles are the Fluent theme resources read for each appearance (`Selection` is `TextControlSelectionHighlightColor` at 35% opacity). On the deprecated system-XAML variant every role is Day's light/dark pair. |
| [WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") | Navigation-menu icon tints and list swipe-action tints are resolved when the rows are set or built, and follow an appearance switch only when they are set again. |
| [WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") | An ink applied to a view that is not in a window yet resolves for the app's appearance; a window with its own appearance recolors it at the next switch. |
| [WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") | Underlines and strikethroughs are single lines in the text color: the double, dotted and wavy styles and a separate decoration color are not drawn. |
| [ArkUI](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") | `Success` and `Warning` have no system color and are Day's light/dark pairs. |
| [ArkUI](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") | The system colors and Day's appearance override come through the ArkTS host; a host project without Day's staged ArkTS code gets Day's light/dark pairs for every role and no override. |

## Platform guidance

- [Apple: color](https://developer.apple.com/design/human-interface-guidelines/color)
- [Windows: color picker](https://learn.microsoft.com/en-us/windows/apps/design/controls/color-picker)
