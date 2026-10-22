---
title: "Color and Paint"
description: "Day's current color currency and the values a native color picker can return."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Color and Paint

The following sections describe the current color model. The [wider color proposal](design/color.md)
is a separate contributor design and is not implemented.

## 1. What ships

**`Color` is one struct**, in `day-geometry`:

```rust
pub struct Color { pub r: f64, pub g: f64, pub b: f64, pub a: f64 }   // sRGB, 0.0–1.0, Copy
```

Constructors cover the models an app authors in (`rgb` / `rgba` / `hex(0xRRGGBB)` / `hsl` /
`hsla` / `hsv` / `hsva`), and all of them land in the same four sRGB components. `to_hsl` and
`to_hsv` decompose back; `lerp_hsl` blends along the short hue arc, where a straight line through
RGB would pass through gray. `with_alpha`, `to_hex_string` and `parse` were added with the color
picker: `parse` reads `#rgb` / `#rgba` / `#rrggbb` / `#rrggbbaa` and the space-separated
component form that `Display` writes, which is what a native pick crosses a JNI / C-ABI / JS
boundary as.

**`Paint` is the fill source**, in `day-spec`:

```rust
pub enum Paint { Solid(Color), Linear(LinearGradient), Radial(RadialGradient) }
```

Gradient geometry is in [`UnitPoint`]s — fractions of the filled shape's bounding box — so one
paint value works at any size. Both gradients replay through each backend's own native gradient
(`NSGradient`, `CGGradient`, a cairo pattern, `QLinearGradient`/`QRadialGradient`, a XAML brush,
an ArkUI gradient, a CSS gradient): all eight canvas backends draw them.

## 2. Where each one is accepted

The two are not interchangeable, and `Paint` is accepted in far fewer places than `Color`.

**`Paint` is accepted by exactly two functions**, both on the canvas: `Draw::fill` and
`Draw::stroke_styled`.

**Everything else takes a bare `Color`:**

| surface | API |
|---|---|
| a piece's background fill | `Decorate::background` |
| a cover's edge-to-edge ground | `Cover::background` |
| a vector glyph's tint | `Vector::tint` |
| a button's fill | `Button::tint` |
| a label's text color | `Label::color`, `TextBuilder::colored` |
| canvas text | `TextStyle::color` |
| a plain canvas stroke | `Draw::stroke` |
| a standalone shape piece | `ShapePiece::fill` / `::stroke` |
| a nav row's icon and badge | `NavItem::icon_tint` / `::badge_tint` |

So a gradient is expressible on a canvas and nowhere else. An app that wants a gradient card
draws one on a canvas and lays its content over it in a `zstack`, which works but gives up the
native container, its corner radius, and its clipping.

Semantic colors are a separate question. There is no `theme::` token module, because default
appearance is native by construction: text, controls, separators and window grounds take the
platform's dynamic colors inside each backend, and a form card takes
`SurfaceRole::SectionCard`. Apps state only the colors they choose. An app-level light or dark
choice is the settings surface's theme row ([settings](settings.md), "The General group"); on
Android that choice recreates the activity through a crossfade, described there.

Those grounds come in PAIRS, and a backend has to take both halves of one. iOS is the clearest
case: `systemGroupedBackground` behind and `secondarySystemGroupedBackground` for the cards that
sit on it — the pairing every Settings-shaped screen uses, and the one that makes a split's two
columns read as one surface rather than two white sheets meeting at a seam. day-uikit paints a
navigation page with the first and `SectionCard` with the second (2026-09). It used to pair the
plain ground with a translucent `tertiarySystemFill` card; changing either alone leaves grey on
grey or a card that cannot be seen. DESIGN.md
[§6.3](../DESIGN.md#63-semantic-theme-tokens) records that decision and why it has held.

## 3. What a native color picker can hand back

The picker prompted this document, so this section lists what each chooser produces.

| chooser | value type | models the user can author in | alpha | beyond sRGB |
|---|---|---|---|---|
| `NSColorPanel` (macOS) | `NSColor`, in **any** color space | grayscale, RGB, CMYK, HSB, color lists, image spectrum, crayons | opt-in | yes — Display P3 and any installed ICC profile; also **pattern** colors (an image) and **named catalog** colors |
| `UIColorPickerViewController` (iOS) | `UIColor` | grid, spectrum, RGB/HSB sliders, hex | opt-in | yes — Display P3 on wide-gamut devices |
| `GtkColorDialog` | `GdkRGBA` (f32) | the GTK editor's HSV plane + hex | opt-in | no — sRGB |
| `QColorDialog` | `QColor` (Rgb / Hsv / Hsl / Cmyk / ExtendedRgb specs) | basic + custom palettes, HSV picker, eyedropper | opt-in | ExtendedRgb allows out-of-range components |
| XAML `ColorPicker` | `Windows.UI.Color` (8-bit ARGB) | spectrum, channel sliders, hex | opt-in | no — sRGB |
| `<input type="color">` | `#rrggbb` string | the browser's own UI | new `alpha` attribute, still shipping | new `colorspace="display-p3"` attribute, still shipping |

There are four things in that table that `Color` cannot carry. Opacity is not one of them;
`Color` has had an alpha channel from the start, and the picker binds it directly.

1. **The color space.** A Display P3 red is outside sRGB. Converting it in clamps it to something
   the user did not pick, and nothing downstream can tell that happened. This gap widens as
   wide-gamut displays become the default on every Apple device and common elsewhere.
2. **The authoring model.** HSB → RGB → HSB is not the identity. Hue is undefined at zero
   saturation and at zero brightness, so a value the user reached by dragging brightness to the
   floor comes back as "black, hue 0" and the picker re-opens on red. Every real picker keeps H/S/V
   as its own state to avoid this ([day-piece-colorpicker](colorpicker.md)'s composed panel does
   exactly that, per presentation), but the *bound value* still cannot express it, so the state
   dies when the panel closes.
3. **Dynamic system colors.** `NSColor.controlAccentColor`, `?attr/colorPrimary`, the XAML accent
   brushes: these are rules the OS re-evaluates on a theme change, an accent-color change, or an
   increase-contrast setting, rather than fixed values. Flattening one into four floats freezes
   it at the moment it was read. Today an app cannot bind to one at all, which is the gap
   DESIGN.md §6.3 leaves open.
4. **Pattern and named-catalog colors** (AppKit only). An image is not a color and will never fit
   in this type. The color picker's AppKit arm drops such a pick
   rather than reporting a wrong value.

Gradients come up here too. No color picker on any platform returns a gradient; the gradient gap
is in §2, where the surfaces that take a color do not take a `Paint`.

---
