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
| `stamp(shape, at, paint)` | Fill ONE shape at many positions, as one op ([Stamping](#stamping)) |
| `stamp_styled(shape, at, paint, style)` | The same, stroking each copy |
| `clip(shape)` / `clipped(shape, f)` | Confine what follows to a shape |
| `text(text, at, style)` | One line of text at a point, in a size, color and [font](fonts.md) |
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
| **web-dom** | `font_families()` is the CSS generic families plus the bundled fonts, not the machine's: a browser lists local fonts only through `queryLocalFonts()`, which is Chromium-only, asynchronous, and behind a permission prompt. `Cap::FontList` answers `Emulated`. |
| **android** | `font_families()` is read from `/system/etc/fonts.xml`, the configuration `Typeface.create` resolves names from, so the system's alias families (`sans-serif-condensed`, `sans-serif-medium`, …) appear as families of their own. |
| **arkui** | Canvas fonts and the font list compile against the SDK's `OH_Drawing_FontMgr` API and are exercised by CI on a device, not by a local emulator run. |
| all | A weight or slant the family does not ship is synthesized where the platform does that (Skia, DirectWrite, CSS) and rounded to the nearest face elsewhere (Pango, CoreText). `face_for` on the family's `FontFamilyInfo` says which face will actually draw. |
| **qt**, **android**, **xaml** | Dash patterns are specified in pixels by Day and converted to those APIs' stroke-width units on the way in. A zero-width stroke falls back to a width of 1 for the conversion. |

Gradient strokes on Apple work by converting the stroke to the region it covers
(`CGContextReplacePathWithStrokedPath`) and drawing the gradient through that clip, which is exact.

## Text

```rust
d.text("Aa", Point::new(8.0, 8.0), TextStyle {
    size: 24.0,
    color: ink,
    anchor: TextAnchor::LEADING,
    font: CanvasFont { family: Some("Pacifico".into()), weight: Some(FontWeight::Bold), italic: false },
});
d.text("40", center, TextStyle { size: 22.0, color: accent, anchor: TextAnchor::CENTERED, ..Default::default() });
```

`TextStyle` is a size in absolute canvas points, a color, an anchor and a [`CanvasFont`](fonts.md):
a family (a platform family or a bundled one, `None` for the platform's own face), a weight and a
slant. Fill what you set and take the rest from `..Default::default()`. One line: a newline is
drawn as the engine draws it, not as a line break.

Canvas text takes a size, not a `FontSpec`: it is for labels and type inside a drawing, and it
carries neither the reader's font-scale setting nor RTL mirroring. Anything a user reads as
content belongs in a `label` piece, which does.

### Anchors

`TextAnchor` is one placement per axis, and the backend does the alignment:

```rust
TextAnchor { h: TextAlign::Trailing, v: TextVAlign::Middle }   // an axis label, right of its tick
TextAnchor::LEADING                                            // top-leading corner (the default)
TextAnchor::CENTERED                                           // the box's center, both ways
TextAnchor::TRAILING                                           // top-trailing corner
```

| `TextAlign` | `at` is |
|---|---|
| `Leading` | the leading edge — left of LTR text, right of RTL |
| `Center` | the horizontal middle of the advance width |
| `Trailing` | the trailing edge |

| `TextVAlign` | `at` is |
|---|---|
| `Top` | the top of the line box |
| `Middle` | its vertical middle |
| `Baseline` | the typographic baseline itself |
| `Bottom` | the bottom of the line box |

Top, Middle and Bottom are edges of the **line box** — the ascent-plus-descent box
`day::measure_text` reports for the same text, size and font — so a drawing that frames its text
from `measure_text` gets a frame that hugs what is drawn on every backend, and
`TextVAlign::Baseline` is `at.y + metrics.ascent` below `Top`.

**Let the anchor do it rather than measuring.** Right-aligning a label by measuring it and
subtracting the width costs a `measure_text` call per label, and the backend is about to lay the
same text out anyway. `day-piece-charts` drew every y-axis label that way until `Trailing`
existed; the measurement per label is now gone. (Measuring is memoized —
[docs/fonts.md](fonts.md#it-is-cached) — so the repeat is cheap; it is still work that the anchor
does for free and more accurately.)

(Before fonts arrived, gtk, qt, android, arkui and web-dom put a `Leading` anchor on the baseline
instead; a caller that compensated for that with an offset can drop it.)

The platform's font families, with the faces each ships, come from `day::font_families()`; the
font menu of a drawing app is that list, and [docs/fonts.md](fonts.md) covers it and the
measurement API.

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

## Stamping

One shape at many positions is **one op**:

```rust
let dot = Shape::Ellipse(Rect::new(-3.0, -3.0, 6.0, 6.0));   // authored around the ORIGIN
d.stamp(dot, positions, color);                              // …translated to each point
d.stamp_styled(cross, positions, color, StrokeStyle::width(1.5));   // stroked instead of filled
```

Each copy is the template translated by one point, and every copy shares the shape, the size and
the paint — so anything that varies means another stamp. For a chart that is one per series, which
is what `day-piece-charts` does: it groups its point marks by symbol, size, color and stroke width
and emits a stamp per group.

**Why it exists.** A `DrawOp` is 168 bytes, and a canvas re-records its whole op list on any
tracked read. Fifty thousand points drawn one `fill` at a time is 8.4 MB of ops to build, compare
against the previous frame and clone into the tree — about **3.6 ms per frame before a backend
draws anything**. As one stamp it is 800 KB and 1.2 ms: the equality check is 9× faster, the clone
23×, and the whole per-frame overhead 3.1×. On the wire to a serializing backend it is a quarter
of the numbers. Day-Viz's scatter records **120,037 marks as 40 ops**.

Backends draw a batch as a single native path — one `NSBezierPath`, one `Path2D`, one `QPainter`
transform per copy into one geometry — so the rasterizer is entered once however many copies there
are, rather than once per mark.

## Performance

`CanvasProps` holds the whole op list and a change replaces it, so a canvas is cheapest when its
op count is stable and small. Two ways to keep it that way:

- **One path over many segments.** Day Tradr's chart line went from one `Shape::Line` per sample
  (about 250 ops for a year of daily closes, every corner unjoined) to a single path op.
- **One stamp over many identical marks** — see [Stamping](#stamping) above.
