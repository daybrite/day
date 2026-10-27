---
title: "Shapes"
description: "Draw reactive shapes with solid or gradient fills, strokes, transforms, and gestures."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Shapes

Draw rectangles, circles, ellipses, capsules, arcs, lines, and polygons as pieces. Shapes size
with their layout bounds and redraw when their reactive properties change. Use a shape group
for a small illustration, or a [canvas](/docs/reference/canvas) for a dense drawing or arbitrary paths.

## Add to your app

This control is included in [`day-pieces`](https://github.com/daybrite/day/tree/main/crates/day-pieces)
and re-exported through `day::prelude::*`. No additional crate or control feature is required.

## Authoring

This example draws a decorative marker from two shapes in one view.

<!-- technical-example: shapes_recipe -->
```rust
use day::prelude::*;

pub fn marker() -> impl Piece {
    shape_group([
        circle().fill(Color::hex(0x2463eb)),
        circle()
            .fill(Color::hex(0xffffff))
            .at(0.35, 0.35, 0.30, 0.30),
    ])
    .frame(32.0, 32.0)
    .a11y(|a| a.decorative())
}
```

<!-- technical-catalog: shapes_recipe -->
```ftl
# This decorative drawing has no user-facing text.
```

For meaningful or interactive graphics, provide a localized accessibility label and role.
See [accessibility](/docs/reference/accessibility).

## Geometry and sizing

| Constructor | Geometry |
| --- | --- |
| [`rectangle()`](https://daybrite.dev/api/day_pieces/fn.rectangle.html) | Fills the assigned rectangle. |
| [`rounded_rectangle(corner)`](https://daybrite.dev/api/day_pieces/fn.rounded_rectangle.html) | Rectangle with a fixed corner radius in points, or a `Corner` value. |
| [`circle()`](https://daybrite.dev/api/day_pieces/fn.circle.html) | Centered circle whose diameter is the shorter side. |
| [`ellipse()`](https://daybrite.dev/api/day_pieces/fn.ellipse.html) | Ellipse filling the bounds. |
| [`capsule()`](https://daybrite.dev/api/day_pieces/fn.capsule.html) | Rounded rectangle with a radius of half the shorter side. |
| [`arc(start_deg, sweep_deg)`](https://daybrite.dev/api/day_pieces/fn.arc.html) | Arc of the bounds' ellipse; zero degrees points right and positive angles run clockwise. |
| [`line(from, to)`](https://daybrite.dev/api/day_pieces/fn.line.html) | Segment between fractional `(x, y)` points; requires a stroke. |
| [`polygon(points)`](https://daybrite.dev/api/day_pieces/fn.polygon.html) | Closed polygon through fractional `(x, y)` points. |

[`shape(kind)`](https://daybrite.dev/api/day_pieces/fn.shape.html) accepts a constant or reactive
`ShapeKind`. Shapes grow to fill available space; use
[`.frame(width, height)`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.frame)
or a bounded parent to give them a useful size.

Line and polygon coordinates are fractions of the bounds and are not clamped to 0–1.
`Corner::Fixed` is clamped to half the shorter side; `Corner::Fraction(1.0)` gives that maximum radius.

## Fills and strokes

[`.fill(color)`](https://daybrite.dev/api/day_pieces/trait.ShapePieceBuilder.html#tymethod.fill)
sets a solid fill. Use
[`.fill_linear(gradient)`](https://daybrite.dev/api/day_pieces/trait.ShapePieceBuilder.html#tymethod.fill_linear)
or [`.fill_radial(gradient)`](https://daybrite.dev/api/day_pieces/trait.ShapePieceBuilder.html#tymethod.fill_radial)
for gradients. These accept values, signals, or closures. If several fill types are set,
radial takes precedence over linear, then solid; call order does not change that precedence.

[`.stroke(color, width)`](https://daybrite.dev/api/day_pieces/trait.ShapePieceBuilder.html#tymethod.stroke)
adds a solid outline. Shapes have no fill or stroke by default. A line ignores fills.
For gradient strokes, dash patterns, and arbitrary paths, use the canvas drawing API.

Closed shapes other than polygons inset their geometry by half the stroke width to keep the
outline within the frame. Lines and polygons use their authored points exactly; strokes at
an edge may clip. See [Colors](/docs/reference/color) for color and gradient values.

## Placement and transforms

| Modifier | Effect |
| --- | --- |
| [`.at(x, y, width, height)`](https://daybrite.dev/api/day_pieces/trait.ShapePieceBuilder.html#tymethod.at) | Places the shape in a fractional subrectangle of its bounds. |
| [`.inset(points)`](https://daybrite.dev/api/day_pieces/trait.ShapePieceBuilder.html#tymethod.inset) | Insets that rectangle before resolving the geometry. |
| [`.rotate(degrees)`](https://daybrite.dev/api/day_pieces/trait.ShapePieceBuilder.html#tymethod.rotate) | Rotates the drawing around its center. |
| [`.scale(factor)`](https://daybrite.dev/api/day_pieces/trait.ShapePieceBuilder.html#tymethod.scale) | Scales the drawing uniformly around its center. |
| [`.offset(x, y)`](https://daybrite.dev/api/day_pieces/trait.ShapePieceBuilder.html#tymethod.offset) | Translates the drawing within its layout frame. |

Drawing transforms do not change the shape's allocated layout size. Inset, rotation, scale,
and offset can read reactive values. Use [animation](/docs/reference/tween) to interpolate
those values over time.

## Shape groups

[`shape_group(shapes)`](https://daybrite.dev/api/day_pieces/fn.shape_group.html) draws shapes in
order in a single canvas view. Each child's reactive properties still update the drawing.
[`shape_group_fn(f)`](https://daybrite.dev/api/day_pieces/fn.shape_group_fn.html) receives the
laid-out size and returns a vector of shapes, for geometry that depends on available space.

A group has one identity and accessibility node. Child gesture handlers are not connected;
put gestures and accessibility information on the group. Use separate shape pieces when
children need independent focus, actions, or labels.

## Gestures

[`.on_tap(f)`](https://daybrite.dev/api/day_pieces/trait.ShapePieceBuilder.html#tymethod.on_tap)
on a shape checks the tap in its transformed local coordinates. Circles and ellipses use
ellipse containment; polygons use polygon containment. Rectangles, rounded rectangles,
arcs, and lines use rectangular bounds, so hit testing does not follow every visible contour.

[`.on_drag(f)`](https://daybrite.dev/api/day_pieces/trait.ShapePieceBuilder.html#tymethod.on_drag)
reports a `Drag` with phase, local pointer location, and cumulative translation. The press
location is `location - translation`; the first reported drag event may arrive after the
pointer has already moved. Drag handling does not use the tap containment test.

## Toolkit implementations

Shapes use each toolkit's canvas renderer. Drawing support and export limits follow the
[canvas reference](/docs/reference/canvas). A shape group reduces the number of native views;
it does not provide a separate view for each child.
