---
title: "Canvas"
description: "The canvas piece records a display list each backend replays through its native 2D API: gradients, transforms, and gestures."
---

<!-- Copyright © The Daybrite Project
     SPDX-License-Identifier: CC-BY-SA-4.0 -->

# Canvas

The `canvas` piece records a display list (a `Vec<DrawOp>`) that each backend replays with its
own native 2-D API: CoreGraphics on Apple, cairo on GTK, `QPainter` on Qt, `android.graphics` on
Android, `OH_Drawing` on HarmonyOS, XAML shapes on Windows, and Canvas2D on the web. The closure
re-records on any tracked read and on `FrameChanged`, and replay is equality-gated, so a canvas
that draws the same list twice costs nothing the second time.

```rust
canvas(|d, size| {
    d.fill(Shape::Rect(Rect::new(0.0, 0.0, size.width, size.height)), SKY);
    d.stroke(Shape::Line(a, b), INK, 2.0);
})
```

## The vocabulary

| Op | What it does |
| --- | --- |
| `fill(shape, paint)` | Fill with a color or a linear/radial gradient |
| `stroke(shape, color, width)` | Stroke at a width, everything else default |
| `stroke_styled(shape, paint, style)` | Stroke with dash, cap, join, and any paint |
| `clip(shape)` / `clipped(shape, f)` | Confine what follows to a shape |
| `text(text, at, style)` | A string at a point |
| `save` / `restore` / `concat(affine)` | Transform and clip state |

`Shape` covers `Rect`, `RoundedRect`, `Ellipse`, `Arc`, `Line`, `Polygon`, and `Path`.

## Paths

`Shape::Path` is any number of contours, straight or curved, with a fill rule. Build one with
`PathBuilder`:

```rust
let ring = PathBuilder::new()
    .rule(FillRule::EvenOdd)      // the inner circle cuts a hole
    .circle(center, 40.0)
    .circle(center, 24.0)
    .build();
d.fill(ring, TEAL);
```

`FillRule::NonZero` is the default and what glyph outlines assume: a hole needs its contour wound
the opposite way. `FillRule::EvenOdd` makes any contour inside another a hole regardless of
winding, which is what PDF's `f*` and SVG's `fill-rule: evenodd` mean.

`smooth_polyline(&points, tension)` fits a Catmull-Rom spline through points and emits it as
cubics. It passes through every point, so it is a drawing of the data rather than a fit to it.
A spline still implies values between the samples, which is why Day Tradr smooths its
sparklines and not the chart someone reads prices off.

### From SVG

`build_path!` parses SVG path data at compile time and emits the `PathBuilder` chain, so a path
costs the same at runtime as writing the chain by hand and there is no string left in the binary:

```rust
let heart = build_path!("M12,21 C5.5,15.5 2,12 2,8.5 C2,5.4 4.4,3 7.5,3 …").build();
```

The whole SVG 1.1 grammar is accepted: relative commands, `H`/`V`, the smooth forms `S`/`T`,
elliptical arcs, implicit command repetition, and SVG's number syntax (`1e2`, `.5.5`, `10-5`).
Malformed data is a compile error naming the offending character. Arcs are converted to cubics by
the macro, because an arc is the one SVG command with no counterpart in the 2-D APIs Day draws
through; converting once at build time is cheaper than converting in nine backends at draw time.

## Strokes

`StrokeStyle` carries width, cap, join, miter limit, and a dash pattern. `StrokeStyle::width(w)`,
`::dashed(w, pattern)` and `::round(w)` cover the common cases; the rest is struct-update syntax.

```rust
d.stroke_styled(path, SLATE, StrokeStyle::dashed(1.0, vec![5.0, 5.0]));
d.stroke_styled(path, LinearGradient::horizontal(RUST, SKY), StrokeStyle::round(6.0));
```

The default cap is `Butt` and the default join is `Miter`, matching PDF, SVG and every native 2-D
API. AppKit, Qt and Android used to force round caps on every canvas stroke; they now honor the
style, so a line that wants round ends has to ask for it.

## Clipping

`clip` intersects the current clip, and the only way to widen it again is `restore`. Every
native 2-D context works this way, so there is no "unclip". `clipped(shape, f)` wraps the
save/clip/restore for you.

## What each backend can and cannot do

Everything above works on every backend except where noted.

| Backend | Limitation |
| --- | --- |
| **web-dom** | A gradient stroke paints the gradient across the path's interior rather than only the stroked band. Canvas2D has no "convert stroke to path", so there is no region to clip to. It looks correct for thin lines and diverges as the width grows. |
| **xaml** | Clipping is rectangular: `UIElement.Clip` accepts only a `RectangleGeometry`, so a path, ellipse or polygon clip degrades to its bounding box, and content is still confined, just less tightly. Escaping this means moving the canvas to `Windows.UI.Composition`, whose `CompositionGeometricClip` does take a path. |
| **appkit** | Quadratic segments are elevated to cubics, exactly (`NSBezierPath`'s own quadratic API is macOS 14+). No visual difference. |
| **gtk**, **arkui** | Same quadratic elevation, for the same reason: cairo and `OH_Drawing` have cubics. |
| **qt**, **android**, **xaml** | Dash patterns are specified in pixels by Day and converted to those APIs' stroke-width units on the way in. A zero-width stroke falls back to a width of 1 for the conversion. |

Gradient strokes on Apple work by converting the stroke to the region it covers
(`CGContextReplacePathWithStrokedPath`) and drawing the gradient through that clip, which is exact.

## Text

Canvas text takes a size and a color, not a `FontSpec`: it is for labels inside a drawing, and it
carries neither the reader's font-scale setting nor RTL mirroring. Anything a user reads as
content belongs in a `label` piece, which does.

## Interaction

A canvas is a real native view, so it takes the ordinary gestures, and two of them report where
the press landed:

```rust
canvas(draw)
    .on_tap_at(move |p| pick(p))                      // Event::Tap's point
    .on_drag(move |drag| pick(drag.location))         // and every phase of a drag
    .frame(width, height)
```

Both points are in the canvas's own coordinate space, origin at its top-leading corner, which
lets a drawn control (a color wheel, a map, a waveform scrubber) turn "the user pressed here"
into a value. `on_tap` (no location) stays for the common case.

Wire both when a press should count as a pick: a press that never moves is a tap on some backends
and a zero-length drag on others, and since both handlers write the same value, a backend that
reports both costs nothing. Put them on the canvas **before** any wrapping decorator, because
`.frame` and `.corner_radius` build layout nodes of their own, and a point in a wrapper's space
is not a point in the canvas's.

The reference use is `day-piece-colorpicker`'s composed panel
([docs/colorpicker.md](colorpicker.md)): its saturation/brightness field, hue strip and opacity
strip are three canvases that read their value straight out of the press location.

### Zoom and pan

Two continuous gestures serve a canvas that is a viewport onto something larger (a drawing, a
map, a timeline):

```rust
canvas(draw)
    .on_pinch(move |g| zoom_about(g.location, g.scale, g.phase))
    .on_pan(move |g| scroll_by(g.delta))
```

`Pinch.scale` is cumulative (the total magnification since the gesture began, with `1.0` meaning
unchanged), so a handler applies it to the zoom it captured at `DragPhase::Began` rather than
multiplying every event in. `Pan.delta` is incremental (each event carries only the movement
since the previous one, as a content displacement: pan by `+= delta` and content follows the
fingers), because desktop wheels produce lone `Changed` events with no began/ended bracket to
accumulate across. Both carry a `location` in canvas coordinates for anchoring the zoom under
the fingers; a backend that cannot know it (GTK's scroll controller) reports `Point::ZERO`.

Where they come from: trackpad magnify and two-finger scroll on macOS (a plain mouse wheel also
pans), `GtkGestureZoom` and the scroll controller on GTK, native zoom gestures and wheel events
on Qt, and pinch plus a two-finger pan recognizer on iOS; one-finger drags still go to
`.on_drag`, so selection and panning coexist. The remaining backends do not deliver these
events yet; apps that offer zoom controls in a toolbar or menu (as Day-Sketch does) lose no
capability there, only the gesture shortcut.

## Performance

`CanvasProps` holds the whole op list and a change replaces it, so a canvas is cheapest when its
op count is stable and small. Prefer one path over many segments: Day Tradr's chart line went from
one `Shape::Line` per sample (about 250 ops for a year of daily closes, every corner unjoined) to
a single path op.
