---
navTitle: "Colors"
title: "Color and Paint"
description: "Day's current color currency and the values a native color picker can return."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Color and Paint

Specify colors for text, backgrounds, and drawings with RGBA values or semantic colors that
follow the app’s theme.

## 1. What ships

**`Color` is one struct**, in `day-geometry`:

```rust
pub struct Color { pub r: f64, pub g: f64, pub b: f64, pub a: f64 }   // sRGB, 0.0–1.0, Copy
```

Constructors cover the models an app authors in ([`rgb`](https://daybrite.dev/api/day_geometry/struct.Color.html#method.rgb) / [`rgba`](https://daybrite.dev/api/day_geometry/struct.Color.html#method.rgba) / [`hex(0xRRGGBB)`](https://daybrite.dev/api/day_geometry/struct.Color.html#method.hex) / [`hsl`](https://daybrite.dev/api/day_geometry/struct.Color.html#method.hsl) /
[`hsla`](https://daybrite.dev/api/day_geometry/struct.Color.html#method.hsla) / [`hsv`](https://daybrite.dev/api/day_geometry/struct.Color.html#method.hsv) / [`hsva`](https://daybrite.dev/api/day_geometry/struct.Color.html#method.hsva)), and all of them land in the same four sRGB components. [`to_hsl`](https://daybrite.dev/api/day_geometry/struct.Color.html#method.to_hsl) and
[`to_hsv`](https://daybrite.dev/api/day_geometry/struct.Color.html#method.to_hsv) decompose back; [`lerp_hsl`](https://daybrite.dev/api/day_geometry/struct.Color.html#method.lerp_hsl) blends along the short hue arc, where a straight line through
RGB would pass through gray. [`with_alpha`](https://daybrite.dev/api/day_geometry/struct.Color.html#method.with_alpha), [`to_hex_string`](https://daybrite.dev/api/day_geometry/struct.Color.html#method.to_hex_string) and [`parse`](https://daybrite.dev/api/day_geometry/struct.Color.html#method.parse) were added with the color
picker: [`parse`](https://daybrite.dev/api/day_geometry/struct.Color.html#method.parse) reads `#rgb` / `#rgba` / `#rrggbb` / `#rrggbbaa` and the space-separated
component form that `Display` writes, which is what a native pick crosses a JNI / C-ABI / JS
boundary as.

**`Paint` is the fill source**, in `day-spec`:

```rust
pub enum Paint { Solid(Color), Linear(LinearGradient), Radial(RadialGradient) }
```

Gradient geometry is in [`UnitPoint`]s — fractions of the filled shape's bounding box — so one
paint value works at any size. Both gradients replay through each toolkit's own native gradient
([`NSGradient`](https://developer.apple.com/documentation/appkit/nsgradient), [`CGGradient`](https://developer.apple.com/documentation/coregraphics/cggradient), a cairo pattern, [`QLinearGradient`](https://doc.qt.io/qt-6/qlineargradient.html)/[`QRadialGradient`](https://doc.qt.io/qt-6/qradialgradient.html), a XAML brush,
an ArkUI gradient, a CSS gradient): all eight canvas toolkits draw them.

## 2. Where each one is accepted

The two are not interchangeable, and `Paint` is accepted in far fewer places than `Color`.

**`Paint` is accepted by exactly two functions**, both on the canvas: [`Draw::fill`](https://daybrite.dev/api/day_pieces/struct.Draw.html#method.fill) and
[`Draw::stroke_styled`](https://daybrite.dev/api/day_pieces/struct.Draw.html#method.stroke_styled).

**Everything else takes a bare `Color`:**

| surface | API |
|---|---|
| a piece's background fill | [`Decorate::background`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.background) |
| a cover's edge-to-edge ground | [`Cover::background`](https://daybrite.dev/api/day_pieces/struct.Cover.html#method.background) |
| a vector glyph's tint | [`Vector::tint`](https://daybrite.dev/api/day_pieces/struct.Vector.html#method.tint) |
| a button's fill | [`Button::tint`](https://daybrite.dev/api/day_pieces/struct.Button.html#method.tint) |
| a label's text color | [`Label::color`](https://daybrite.dev/api/day_pieces/struct.Label.html#method.color), [`TextBuilder::colored`](https://daybrite.dev/api/day_pieces/struct.TextBuilder.html#method.colored) |
| canvas text | `TextStyle::color` |
| a plain canvas stroke | [`Draw::stroke`](https://daybrite.dev/api/day_pieces/struct.Draw.html#method.stroke) |
| a standalone shape piece | [`ShapePiece::fill`](https://daybrite.dev/api/day_pieces/struct.ShapePiece.html#method.fill) / `::stroke` |
| a nav row's icon and badge | [`NavItem::icon_tint`](https://daybrite.dev/api/day_pieces/struct.NavItem.html#method.icon_tint) / `::badge_tint` |

So a gradient is expressible on a canvas and nowhere else. An app that wants a gradient card
draws one on a canvas and lays its content over it in a [`zstack`](https://daybrite.dev/api/day_pieces/fn.zstack.html), which works but gives up the
native container, its corner radius, and its clipping.

Semantic colors are a separate question. There is no `theme::` token module, because default
appearance is native by construction: text, controls, separators and window grounds take the
platform's dynamic colors inside each toolkit, and a form card takes
`SurfaceRole::SectionCard`. Apps state only the colors they choose. An app-level light or dark
choice is the settings surface's theme row ([settings](settings.md), "The General group"); on
Android that choice recreates the activity through a crossfade, described there.

Those grounds come in PAIRS, and a toolkit has to take both halves of one. iOS is the clearest
case: `systemGroupedBackground` behind and `secondarySystemGroupedBackground` for the cards that
sit on it — the pairing every Settings-shaped screen uses, and the one that makes a split's two
columns read as one surface rather than two white sheets meeting at a seam. day-uikit paints a
navigation page with the first and `SectionCard` with the second (2026-09). It used to pair the
plain ground with a translucent `tertiarySystemFill` card; changing either alone leaves grey on
grey or a card that cannot be seen.

## 3. What a native color picker can hand back

The picker prompted this document, so this section lists what each chooser produces.

| chooser | value type | models the user can author in | alpha | beyond sRGB |
|---|---|---|---|---|
| [`NSColorPanel`](https://developer.apple.com/documentation/appkit/nscolorpanel) (macOS) | [`NSColor`](https://developer.apple.com/documentation/appkit/nscolor), in **any** color space | grayscale, RGB, CMYK, HSB, color lists, image spectrum, crayons | opt-in | yes — Display P3 and any installed ICC profile; also **pattern** colors (an image) and **named catalog** colors |
| [`UIColorPickerViewController`](https://developer.apple.com/documentation/uikit/uicolorpickerviewcontroller) (iOS) | [`UIColor`](https://developer.apple.com/documentation/uikit/uicolor) | grid, spectrum, RGB/HSB sliders, hex | opt-in | yes — Display P3 on wide-gamut devices |
| [`GtkColorDialog`](https://docs.gtk.org/gtk4/class.ColorDialog.html) | [`GdkRGBA`](https://docs.gtk.org/gdk4/struct.RGBA.html) (f32) | the GTK editor's HSV plane + hex | opt-in | no — sRGB |
| [`QColorDialog`](https://doc.qt.io/qt-6/qcolordialog.html) | [`QColor`](https://doc.qt.io/qt-6/qcolor.html) (Rgb / Hsv / Hsl / Cmyk / ExtendedRgb specs) | basic + custom palettes, HSV picker, eyedropper | opt-in | ExtendedRgb allows out-of-range components |
| XAML [`ColorPicker`](https://learn.microsoft.com/en-us/uwp/api/windows.ui.xaml.controls.colorpicker) | [`Windows.UI.Color`](https://learn.microsoft.com/en-us/uwp/api/windows.ui.color) (8-bit ARGB) | spectrum, channel sliders, hex | opt-in | no — sRGB |
| `<input type="color">` | `#rrggbb` string | the browser's own UI | new `alpha` attribute, still shipping | new `colorspace="display-p3"` attribute, still shipping |

There are four things in that table that `Color` cannot carry. Opacity is not one of them;
`Color` has had an alpha channel from the start, and the picker binds it directly.

1. **The color space.** A Display P3 red is outside sRGB. Converting it in clamps it to something
   the user did not pick, and nothing downstream can tell that happened. This gap widens as
   wide-gamut displays become the default on every Apple device and common elsewhere.
2. **The authoring model.** HSB → RGB → HSB is not the identity. Hue is undefined at zero
   saturation and at zero brightness, so a value the user reached by dragging brightness to the
   floor comes back as "black, hue 0" and the picker re-opens on red. Every real picker keeps H/S/V
   as its own state to avoid this ([day-pieces](colorpicker.md)'s composed panel does
   exactly that, per presentation), but the *bound value* still cannot express it, so the state
   dies when the panel closes.
3. **Dynamic system colors.** [`NSColor.controlAccentColor`](https://developer.apple.com/documentation/appkit/nscolor), `?attr/colorPrimary`, the XAML accent
   brushes: these are rules the OS re-evaluates on a theme change, an accent-color change, or an
   increase-contrast setting, rather than fixed values. Flattening one into four floats freezes
   it at the moment it was read. Today an app cannot bind to one at all.
4. **Pattern and named-catalog colors** (AppKit only). An image is not a color and will never fit
   in this type. The color picker's AppKit arm drops such a pick
   rather than reporting a wrong value.

No color picker on any platform returns a gradient. The color-valued surfaces described above
do not accept a `Paint`.

---

## Platform guidance

- [Apple: color](https://developer.apple.com/design/human-interface-guidelines/color)
- [Windows: color picker](https://learn.microsoft.com/en-us/windows/apps/design/controls/color-picker)
