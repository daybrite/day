<!-- Copyright © The Daybrite Project -->
<!-- SPDX-License-Identifier: MPL-2.0 -->

# Surfaces

A surface is the styled native view behind a piece: its fill, its rounded corners, its border
and its shadow. The toolkit draws all four, so a surface costs one native view and no canvas.
Every color in it is an [ink](/docs/reference/color), so it follows the appearance with no
rebuild.

```rust
label("Sale")
    .padding(Space::M)
    .surface(
        Surface::new()
            .fill(LinearGradient::vertical(top, bottom))
            .corner_radius(Radius::L)
            .corners(Corners::TOP)
            .border(SystemColor::Separator, 1.0)
            .shadow(Shadow::new(Color::rgba(0.0, 0.0, 0.0, 0.25), 16.0).offset(0.0, 6.0)),
    )
```

## One view, not four

[`.background(color)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.background) and [`.corner_radius(r)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.corner_radius) each wrap the piece in a view of their own. That
composes for a rounded colored surface, but it cannot give a border that follows the rounded
corners or a shadow in their shape: each modifier only knows its own view.
[`.surface(Surface)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.surface) puts everything on one view.

| Builder call | What it sets |
| --- | --- |
| [`.fill(paint)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Surface.html#method.fill) | A solid ink, or a linear, radial or conic gradient; unit points resolve against the surface's frame and follow it when it resizes |
| [`.corner_radius(r)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Surface.html#method.corner_radius) | The radius: points, a `Radius` step, or a frame-relative `Corner` (`Corner::CAPSULE`, `Corner::Fraction(f)`) |
| [`.corners(Corners)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Surface.html#method.corners) | Which corners the radius rounds. `Corners::ALL` by default; combine with `\|`, or use `TOP`, `BOTTOM`, `LEFT`, `RIGHT` |
| [`.border(color, width)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Surface.html#method.border) | A line inside the frame, following the corners |
| [`.shadow(Shadow)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Surface.html#method.shadow) | A shadow of the rounded rectangle, outside the frame |

[`.border(color, width)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.border) and [`.shadow(shadow)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.shadow) also exist as modifiers of their own, for the
square case: a border or a shadow around a piece with nothing else.

## Reactive surfaces

[`.surface(..)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.surface), [`.corner_radius(..)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.corner_radius),
[`.border(color, width)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.border) and [`.shadow(..)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.shadow) take a constant, a
signal or a closure, like [`.background(..)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.background) and [`.opacity(..)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.opacity). A change patches the native view
in place (`ContainerPatch::Surface`); a constant costs nothing after realize. A color that only
differs by appearance needs no closure at all: pass the ink.

```rust
let selected = Signal::new(false);
row(cells)
    .corner_radius(Radius::M)
    .border(SystemColor::Accent, move || if selected.get() { 2.0 } else { 0.0 })
```

## Radius tokens and frame-relative corners

[`Radius`](https://docs.rs/day-pieces/latest/day_pieces/enum.Radius.html) is the corner scale
beside `Space`: `XS`, `S`, `M`, `L` and `XL` resolve to 4, 6, 10, 14 and 20 points today on every
platform, and [`.card()`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.card) uses `Radius::M`. A radius can also follow the frame:
`Corner::CAPSULE` rounds the ends of a pill whatever its length, and `Corner::Fraction(f)` is a
fraction of half the shorter side. Day re-resolves a frame-relative radius each time layout
resizes the view, in the same layout pass that sets the new frame, so the toolkit always
receives points and the first frame the surface is shown at already has its radius.

## Rules

- **Corners are physical.** `Corners::TOP_LEFT` is the top-left corner in both layout
  directions, as `CornerRadii` is on a canvas.
- **A rounded surface clips its content** to the rounded rectangle, as [`.corner_radius(r)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.corner_radius) does.
- **A shadowed surface does not clip.** The shadow lies outside the frame, and a clip would cut
  it. Put a [`.corner_radius(r)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.corner_radius) inside the surface when the content must be cut as well.
- **A shadow takes no room in layout.** Leave padding around the piece for it. A scroll view or
  any clipping ancestor cuts a shadow that reaches past its edge.
- **`Shadow::blur` is the CSS `box-shadow` blur radius**: the width of the soft edge, twice the
  Gaussian's standard deviation.
  [`Shadow::new(color, blur).offset(dx, dy)`](https://docs.rs/day-spec/latest/day_spec/struct.Shadow.html#method.offset)
  moves it; positive
  `dy` is down.
- **The border is drawn over the content**, on every toolkit: a child that fills the surface edge
  to edge never covers it.
- **A fill is one paint.** A solid ink or a gradient, never both.

For a semantic, theme-following surface use [`.card()`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.card), which asks the toolkit for its own
grouped-content material. A `Surface` is for the cases where the app owns the look.

## Continuous corners

Corners are circular arcs. A continuous ("squircle") corner, the curvature Apple uses, is native on
AppKit and UIKit (`cornerCurve = .continuous`) and paintable on Qt, but on the others it would
mean drawing the fill, the border, the clip and the shadow from a path Day computes: the web's
`corner-shape` is Chromium-only, WinUI's `CornerRadius` and ArkUI's border radius are circular,
GTK's CSS has no such corner, and Android clips to an arbitrary path only from API 33. Day leaves
it out until that path-shaped surface exists on every toolkit.

## Toolkit support

Everything above works on every toolkit except where noted.

| Toolkit | Limitation |
| --- | --- |
| [AppKit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") | The view draws the fill and every gradient, radial and conic included, clipped to its outline; the border is the layer's, over the subviews. An animated change of a solid fill runs on the layer, and a gradient fill or a change to or from no fill applies at once. The shadow is the view's, cast by an explicit path. |
| [UIKit](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") | The gradient is a `CAGradientLayer` (radial and conic are its own types). During an animated resize the shadow jumps to its new shape. |
| [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") | A shadow is drawn by the view outside its bounds and is cut at the nearest native container (scroll content, pager page, list row) or clipping container, and by an ancestor rendered through an offscreen layer. A clipping surface with named corners clips by path. |
| [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") | A Day container draws the border after its children; on any other widget the border is an inset box shadow, under the content. A conic fill is a CSS `conic-gradient()`, which GSK's cairo renderer draws as a cairo mesh; where the system's cairo cannot paint one (Ubuntu 26.04's x86-64-v3 build), the container paints the fill the way a canvas does. A fill animation rewrites the surface's CSS each frame. A shadowed surface stops the plain containers above it clipping their content, up to the first container that clips, rounds, scrolls or is a list cell. |
| [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") | The border is a child widget drawn over the content. Radial and conic fills are Qt's own gradients. A change to a reactive surface applies at once. A shadow is clipped at the parent widget's bounds, and a surface with a shadow or a rounded clip still renders its subtree through an offscreen pixmap. |
| [Web](https://daybrite.dev/docs/reference/platforms/web-dom "toolkit:web-dom") | The border is a layer over the content. |
| [ArkUI](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") | A radial fill is a circle with its radius along the shorter side, not an ellipse stretched to the frame. A shadow is sized in pixels when the surface is realized and does not follow a later display-density change. A zero-blur shadow is softened by about half a pixel. |
| [WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") | The shadow is a Composition `DropShadow` beneath the element. A corner radius smaller than half the border width draws square. On the deprecated system-XAML variant, per-corner clips and shadow outlines fall back to all four corners. |
| [WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") | A radial fill is a `RadialGradientBrush`; a conic fill has no XAML brush and is painted with Win2D into an image brush, redrawn at each new size (on system XAML both show one stop's solid color). The border is a separate element over the children. XAML cannot animate a corner radius, so a change to a reactive surface applies at once, and a role surface (a section card, a cover's ground) keeps its realize-time look. |
