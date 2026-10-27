---
navTitle: "Colors"
title: "Colors and gradients"
description: "Choose solid colors, gradient fills, and native appearance defaults."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Colors and gradients

Color can distinguish content, show state, and draw attention to an action. Text, shapes, or other
cues need to carry the same meaning when color alone would be hard to distinguish.

Use `Color` for fixed sRGB colors and `Paint` for canvas fills and styled strokes. Leave a control's
color unset when it should follow the platform's default appearance.

## Solid colors

[`Color`](https://docs.rs/day-geometry/latest/day_geometry/struct.Color.html) stores red, green, blue, and
alpha as floating-point sRGB components. Constructors include
[`rgb`](https://docs.rs/day-geometry/latest/day_geometry/struct.Color.html#method.rgb),
[`rgba`](https://docs.rs/day-geometry/latest/day_geometry/struct.Color.html#method.rgba),
[`hex`](https://docs.rs/day-geometry/latest/day_geometry/struct.Color.html#method.hex),
[`hsl`](https://docs.rs/day-geometry/latest/day_geometry/struct.Color.html#method.hsl), and
[`hsv`](https://docs.rs/day-geometry/latest/day_geometry/struct.Color.html#method.hsv).
All produce the same RGBA representation.

```rust
let accent = Color::hex(0x2463eb);
let translucent = accent.with_alpha(0.5);
rectangle().fill(translucent).frame(80.0, 40.0)
```

[`with_alpha`](https://docs.rs/day-geometry/latest/day_geometry/struct.Color.html#method.with_alpha) changes
opacity. [`parse`](https://docs.rs/day-geometry/latest/day_geometry/struct.Color.html#method.parse) accepts
`#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, and the space-separated component form produced by
`Display`. [`to_hex_string`](https://docs.rs/day-geometry/latest/day_geometry/struct.Color.html#method.to_hex_string)
formats a hex value. For interpolation,
[`lerp_hsl`](https://docs.rs/day-geometry/latest/day_geometry/struct.Color.html#method.lerp_hsl) follows the
short hue arc rather than a straight line through RGB.

## Gradients

`Paint` represents a solid color, a `LinearGradient`, or a `RadialGradient`. Gradient positions
use `UnitPoint` coordinates relative to the drawing bounds. A radial gradient stretches with
non-square bounds.

| Surface | API |
| --- | --- |
| Canvas fill | [`Draw::fill`](https://docs.rs/day-pieces/latest/day_pieces/struct.Draw.html#method.fill) accepts a solid color or gradient. |
| Canvas styled stroke | [`Draw::stroke_styled`](https://docs.rs/day-pieces/latest/day_pieces/struct.Draw.html#method.stroke_styled) accepts `Paint`. |
| Shape fill | [`.fill_linear`](https://docs.rs/day-pieces/latest/day_pieces/trait.ShapePieceBuilder.html#tymethod.fill_linear) and [`.fill_radial`](https://docs.rs/day-pieces/latest/day_pieces/trait.ShapePieceBuilder.html#tymethod.fill_radial) accept their respective gradients. |
| Ordinary background, text color, or tint | Takes a solid `Color`; does not accept a gradient. |

For a gradient card, place a filled [shape](/docs/reference/shapes) behind its content in a
[`zstack`](https://docs.rs/day-pieces/latest/day_pieces/fn.zstack.html). Set the shape's bounds and corner
radius explicitly. See [Canvas](/docs/reference/canvas) for drawing and export support.

## Native appearance

Default text, controls, separators, and window backgrounds use colors selected by the toolkit.
[Forms](/docs/reference/forms) use the platform's section-card surface. These defaults can
respond to appearance and accessibility settings without a fixed app color.

`Color` itself is a concrete value, not a dynamic system-color token. Assigning a fixed color
replaces that default; choose suitable values for each appearance your app supports. The
[Settings](/docs/reference/settings) theme preference controls the app's light or dark appearance.

## Color picker values

A [color picker](/docs/reference/colorpicker) binds a `Color`, including alpha when enabled.
It does not return a gradient. The portable value does not retain an ICC profile, a named
system color, an image pattern, or the color model used to enter it.

Converting a wide-gamut color to sRGB can lose information. RGB also cannot preserve a user's
chosen hue when saturation or brightness is zero. The composed picker retains that editing
state while its panel is open; the bound value remains RGBA. AppKit pattern colors that cannot
be represented are ignored.

## Platform guidance

- [Apple: color](https://developer.apple.com/design/human-interface-guidelines/color)
- [Windows: color picker](https://learn.microsoft.com/en-us/windows/apps/design/controls/color-picker)
