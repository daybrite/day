---
title: "Color and Paint proposal"
description: "Color and Paint proposal: retained contributor discussion and its implementation boundaries."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Color and Paint proposal

> [!NOTE]
> **Status: proposal; not implemented.** This contributor record preserves earlier design and implementation
> discussion. Examples here are not stable API guidance. See the [app documentation](../color.md)
> for the current entry point. Platform execution claims are historical and were not rerun in Phase 2.

## 4. Proposal

> [!WARNING]
> Everything below is a proposal. It is not implemented, and [`day-pieces`](/docs/reference/colorpicker) binds to
> today's `Color`.

There are five changes, ordered by what they pay off. Each is independently landable, and only
the fourth is a wide edit.

### 4.1 `Color` becomes a value or a role, and stays `Copy`

```rust
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub value: ColorValue,
    /// Multiplies whatever `value` resolves to. Outside the value so `.opacity(0.14)` works on a
    /// role as well as on a literal — which is most of what an app does with alpha.
    pub alpha: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ColorValue {
    /// Literal components in a stated model and space. Round-trips exactly.
    Components { space: ColorSpace, model: ColorModel, c: [f64; 4] },
    /// A platform semantic role the BACKEND resolves when it paints, so it tracks light/dark, the
    /// user's accent color, and increase-contrast instead of freezing at build time.
    Role(ColorRole),
}

pub enum ColorSpace { Srgb, DisplayP3, Rec2020 }
pub enum ColorModel { Rgb, Hsl, Hsb, Cmyk, Gray, OkLch }
pub enum ColorRole {
    Accent, Label, SecondaryLabel, PlaceholderLabel, Separator,
    WindowBackground, ControlBackground, SelectedContent, Link, Destructive,
}
```

The shape is chosen for three properties.

- **It stays `Copy` and fixed-size.** `[f64; 4]` holds CMYK's four channels and leaves the fourth
  slot unused for the three-channel models; `ColorRole` is fieldless. Props structs across
  day-spec are `Copy`, and a `Vec` in `Color` would take that away from all of them.
- **Every existing constructor survives unchanged.** `Color::hex(0xE86A3C)` builds
  `Components { space: Srgb, model: Rgb, c: [.91, .42, .24, 0.] }` with `alpha: 1.0`;
  `Color::hsv(30., .8, .9)` builds the same struct with `model: Hsb` and keeps the hue the user
  chose. Gap 2 closes by construction; a picker's HSB pick survives being stored.
- **The picker's result needs no separate type.** `Color` already says which model and which space
  it came from, so a pick is a `Color`, and re-opening the chooser can land the user back in the
  tab they were in.

The accessor every backend and most app code uses:

```rust
impl Color {
    /// sRGB components, always available — what a backend paints when it has nothing better, and
    /// what every `c.r` / `c.g` / `c.b` / `c.a` site becomes. A `Role` resolves through a
    /// documented off-platform fallback table (the backend is what really resolves one).
    pub fn srgb(&self) -> [f64; 4];
}
```

This change breaks field access. `c.r` / `c.g` / `c.b` / `c.a` appear in roughly 90 places
across the day crates and a handful in each app; each becomes `srgb()` destructuring or
`with_alpha`. That is the entire migration, and it is mechanical.

### 4.2 The toolkit SPI resolves a role instead of receiving four floats

Backends stop taking components and start taking a `Color`, resolving it themselves:

| role | AppKit / UIKit | GTK | Qt | XAML | Android | web |
|---|---|---|---|---|---|---|
| `Accent` | `controlAccentColor` / `tintColor` | `@accent_color` | `QPalette::Highlight` | `AccentFillColorDefaultBrush` | `?attr/colorPrimary` | `AccentColor` |
| `Label` | `labelColor` | `@theme_fg_color` | `QPalette::WindowText` | `TextFillColorPrimary` | `?attr/colorOnSurface` | `canvastext` |
| `Separator` | `separatorColor` | `@borders` | `QPalette::Mid` | `DividerStrokeColorDefault` | `?attr/colorOutlineVariant` | `-webkit-…` |

A role is re-resolved on the appearance change every backend already reports, so a bound role
repaints with the system. This change closes gap 3, in the shape DESIGN.md §6.3 left room for: a
typed value rather than an app-side token module, which is the part of that decision to keep.

### 4.3 A wider space rides along instead of clamping

`ColorSpace` on the value lets a Display P3 pick reach a backend that can honor it (`CGColor`
with a P3 space, `android.graphics.Color.valueOf(r, g, b, a, ColorSpace)`, CSS
`color(display-p3 …)`, `QColor::fromRgbF` with extended range) and lets everyone else convert
down once, at the edge, where the loss is visible in one place instead of at authoring time.
`Color::srgb()` is that conversion.

`Cap::WideGamut` reports whether the compiled backend honors a non-sRGB space, so an app that
cares can check.

### 4.4 `Paint` becomes the currency for surfaces, but not for marks

Promote `Paint` from canvas-only to the type every *surface* takes, with
`impl From<Color> for Paint` so existing call sites compile unchanged:

```rust
fn background<M>(self, paint: impl IntoReactive<Paint, M>) -> Decorated<Self>;  // Decorate
fn background(self, f: impl Fn(&R) -> Paint + 'static) -> Self;           // Cover
pub fn fill<M>(self, paint: impl IntoReactive<Paint, M>) -> Self;         // ShapePiece
```

The marks keep `Color`: `Vector::tint`, `Button::tint`, `NavItem::icon_tint` / `badge_tint`,
`Label::color`, `TextRun::colored`, `TextStyle::color`. A template tint and a text color are solid
by nature on every toolkit Day targets; making them accept a gradient and then documenting "the
gradient is ignored here" is worse than the restriction being in the type. A surface takes a
`Paint`, and a mark takes a `Color`.

Backends that cannot paint a gradient behind a native container answer
`Cap::GradientSurface = Unsupported` and paint the gradient's midpoint sample, so a page degrades
to a flat card rather than to nothing.

`Paint` also gains a conic shape and an interpolation choice while it is being touched:

```rust
pub enum Paint { Solid(Color), Linear(LinearGradient), Radial(RadialGradient), Conic(ConicGradient) }

pub struct GradientStops {
    pub stops: Vec<(f64, Color)>,
    /// The space stops are interpolated IN. sRGB is the historical default and the muddy one;
    /// Oklab is perceptually even, and `Hsl` sweeps the hue arc — which `Color::lerp_hsl` already
    /// does by hand for the animation path, so the framework has taken this position once already.
    pub interpolate: Interpolation,
}

pub enum Interpolation { Srgb, Oklab, HslShorter, HslLonger }
```

`Conic` is the one gradient shape every backend can draw (`QConicalGradient`, `CAConicGradient`,
a cairo mesh, `conic-gradient()`) and the one a color wheel wants.

### 4.5 Serialize the whole type the way CSS Color 4 does

`Display` and `parse` grow to cover the whole type rather than sRGB components:

```text
#e86a3c            #e86a3c80          srgb(0.91 0.42 0.24)
display-p3(1 0 0 / 0.5)               hsb(30 0.8 0.9)
cmyk(0 0.55 0.75 0.09)                oklch(0.7 0.15 40)
role(accent)                          role(label / 0.6)
```

The grammar is CSS Color 4's: a reviewed design for this problem, one web developers already
read, and a pass-through for the web arm. dayscript's `input:` step types the same grammar,
`day-lite` parses it, and it crosses every native boundary, so there is one grammar instead of
four.

## 5. What this proposal does not do

- **Patterns and image fills.** An `NSColor` pattern is an image; it belongs in an image API. The
  color picker drops such a pick.
- **ICC profiles or arbitrary color management.** Three named spaces cover what the platforms
  actually expose; a general CMM is a different project.
- **A theme token module.** §6.3's decision stands: roles are typed values on `Color` rather than
  an app-side `theme::` namespace, and the default is still to state nothing and get the
  platform's colors.
- **Making `Color` non-`Copy`.** Every shape above is chosen to preserve it.

## 6. Which change to land first

§4.4, `Paint` for surfaces, goes first. It is the gap an app hits first (there is no way to put
a gradient behind anything but a canvas), it is additive rather than breaking, and it needs no
change to the `Color` type at all.

§4.1 and §4.2 have to land together, because a role in the type means nothing until backends
resolve it. That is the larger change, and the one [day-pieces](../colorpicker.md) would
benefit from most: a picker that could offer the platform's accent color as a preset, and have
the pick stay dynamic, is a different control from one that hands back four frozen floats.
