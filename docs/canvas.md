---
navTitle: "Canvases"
title: "Canvas"
description: "The canvas piece records a display list each toolkit replays through its native 2D API: gradients, transforms, and gestures."
---

<!-- Copyright © The Daybrite Project
     SPDX-License-Identifier: CC-BY-SA-4.0 -->

# Canvas

Draw charts, diagrams, and custom controls with paths, text, images, and gradients. A [`canvas`](https://daybrite.dev/api/day_pieces/fn.canvas.html)
redraws when the state read by its drawing closure changes.

The [`canvas`](https://daybrite.dev/api/day_pieces/fn.canvas.html) piece records a display list (a `Vec<DrawOp>`) that each toolkit replays with its
own native 2-D API: CoreGraphics on Apple, cairo on GTK, [`QPainter`](https://doc.qt.io/qt-6/qpainter.html) on Qt, `android.graphics` on
Android, [`OH_Drawing`](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-arkgraphics2d/capi-drawing.md) on HarmonyOS, Win2D on Windows, and Canvas2D on the web. The closure
re-records on any tracked read and on `FrameChanged`, and replay is equality-gated, so a canvas
that draws the same list twice costs nothing the second time.

```rust
canvas(|d, size| {
    d.fill(Shape::Rect(Rect::new(0.0, 0.0, size.width, size.height)), SKY);
    d.stroke(Shape::Line(a, b), INK, 2.0);
})
```

For animation, use [`day::frame`](frames.md) to advance state at native display frame
opportunities and notify a `Trigger` tracked by the canvas. Keep the `FrameHandle` in your
controller; return `ControlFlow::Break(())` when motion settles and resume on the next input.
This avoids recording identical frames while idle. Frame timestamps are unclamped; simulation
clients choose their own fixed-step and catch-up policy. Day-Showcase's Animation page contains
a complete interactive example with trails, impact particles, pause/resume, and idle shutdown.
For transitions between two states (a value, a rect, a color, a whole chart), `day::tween`
supplies the interpolation, the timing and the frame-driven driver ([tween](tween.md)).

Text samples use generated Fluent accessors; see the [example catalog](snippet-catalog.md)
and [localization setup](/docs/how-to/localization).

## The vocabulary

| Op | What it does |
| --- | --- |
| [`fill(shape, paint)`](https://daybrite.dev/api/day_pieces/struct.Draw.html#method.fill) | Fill with a color or a linear/radial gradient |
| [`stroke(shape, color, width)`](https://daybrite.dev/api/day_pieces/struct.Draw.html#method.stroke) | Stroke at a width, everything else default |
| [`stroke_styled(shape, paint, style)`](https://daybrite.dev/api/day_pieces/struct.Draw.html#method.stroke_styled) | Stroke with dash, cap, join, and any paint |
| [`stamp(shape, at, paint)`](https://daybrite.dev/api/day_pieces/struct.Draw.html#method.stamp) | Fill ONE shape at many positions, as one op ([Stamping](#stamping)) |
| [`stamp_styled(shape, at, paint, style)`](https://daybrite.dev/api/day_pieces/struct.Draw.html#method.stamp_styled) | The same, stroking each copy |
| [`clip(shape)`](https://daybrite.dev/api/day_pieces/struct.Draw.html#method.clip) / [`clipped(shape, f)`](https://daybrite.dev/api/day_pieces/struct.Draw.html#method.clipped) | Confine what follows to a shape |
| [`text(text, at, style)`](https://daybrite.dev/api/day_pieces/struct.Draw.html#method.text) | One line of text at a point, in a size, color and [font](fonts.md) |
| [`image(&bitmap, rect)`](https://daybrite.dev/api/day_pieces/fn.image.html) / [`image_with_opacity(…)`](https://daybrite.dev/api/day_pieces/struct.Draw.html#method.image_with_opacity) | A decoded [image](images.md) scaled into a rect — the handle, never bytes |
| [`save`](https://daybrite.dev/api/day_pieces/struct.Draw.html#method.save) / [`restore`](https://daybrite.dev/api/day_pieces/struct.Draw.html#method.restore) / [`concat(affine)`](https://daybrite.dev/api/day_pieces/struct.Draw.html#method.concat) | Transform and clip state |

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

[`arc_to(center, radius, start_deg, sweep_deg)`](https://daybrite.dev/api/day_pieces/struct.PathBuilder.html#method.arc_to) appends a circular arc — degrees, `0` = the +x
axis, positive sweeping **clockwise**, the same convention `Shape::Arc` and [`circle`](https://daybrite.dev/api/day_pieces/fn.circle.html) use. It is a
*segment*, not a shape, which is what lets a figure made of arcs be one closed contour:

```rust
// A donut wedge: out along the far edge, back along the near one.
let wedge = PathBuilder::new()
    .arc_to(center, 60.0, 20.0, 100.0)     // outer edge, clockwise
    .arc_to(center, 34.0, 120.0, -100.0)   // inner edge, back the other way
    .close()
    .build();
```

An arc joins whatever came before it — reached by a line when the subpath has a current point,
opening a new one when it does not (including straight after a [`close`](https://daybrite.dev/api/day_pieces/struct.PathBuilder.html#method.close)). That is the difference
that matters: `Shape::Arc` draws an arc, [`arc_to`](https://daybrite.dev/api/day_pieces/struct.PathBuilder.html#method.arc_to) builds a figure *out of* arcs, and a donut hole,
an angular inset and a corner radius can only compose in the second form. `day-piece-charts` drew
every pie and donut wedge from forty lines of hand-rolled cubics until this existed.

It emits cubics rather than an arc op on the wire. Every rasterizer under Day has its own rule for
joining an arc to the line before it and its own flattening tolerance, so the same beziers
everywhere means the same pixels everywhere. A cubic cannot *be* a circle: the radial error peaks
at 2.7 × 10⁻⁴ of the radius on the quarter turns this splits into — a quarter of a pixel on a
circle a thousand points across — which is the same tradeoff Core Graphics, cairo and every SVG
renderer make.

[`smooth_polyline(&points, tension)`](https://daybrite.dev/api/day_pieces/struct.PathBuilder.html#method.smooth_polyline) fits a Catmull-Rom spline through points and emits it as
cubics. It passes through every point, so it is a drawing of the data rather than a fit to it.
A spline still implies values between the samples, which is why Day Trader smooths its
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
through; converting once at build time is cheaper than converting in nine toolkits at draw time.

## Rounded corners

`Shape::RoundedRect(rect, r)` rounds every corner alike. [`Shape::rounded_rect(rect, radii)`](https://daybrite.dev/api/day_spec/enum.Shape.html#method.rounded_rect) takes
a `CornerRadii`, one radius per corner, for the shapes that round only some of them: a bar
standing on its axis ([`CornerRadii::top(r)`](https://daybrite.dev/api/day_spec/struct.CornerRadii.html#method.top)), one growing rightward from it
([`CornerRadii::right(r)`](https://daybrite.dev/api/day_spec/struct.CornerRadii.html#method.right)), a tab, a card joined to its neighbor.

```rust
// Rounded where the bar ends, square where it meets the baseline.
d.fill(Shape::rounded_rect(bar, CornerRadii::top(6.0)), TEAL);
```

Radii are fitted to the rectangle the way CSS fits `border-radius`: negative and non-finite radii
are square, and when the two radii along a side add up to more than the side, all four shrink by
the same factor. A generous radius on a short bar degrades to a pill instead of a bow tie, with no
clamping at the call site. The result is the simplest shape that draws it: `Shape::Rect` when every
corner is square, `Shape::RoundedRect` (each toolkit's own) when they agree, and a path of
[`arc_to`](https://daybrite.dev/api/day_pieces/struct.PathBuilder.html#method.arc_to) quarter turns otherwise, so nothing new crosses the wire and every toolkit draws the same
curves. [`PathBuilder::rounded_rect(rect, radii)`](https://daybrite.dev/api/day_pieces/struct.PathBuilder.html#method.rounded_rect) appends the same contour to a path, for a figure
made of several (a rounded frame with a hole cut out of it).

## Strokes

`StrokeStyle` carries width, cap, join, miter limit, and a dash pattern. [`StrokeStyle::width(w)`](https://daybrite.dev/api/day_spec/struct.StrokeStyle.html#method.width),
`::dashed(w, pattern)` and `::round(w)` cover the common cases; the rest is struct-update syntax.

```rust
d.stroke_styled(path, SLATE, StrokeStyle::dashed(1.0, vec![5.0, 5.0]));
d.stroke_styled(path, LinearGradient::horizontal(RUST, SKY), StrokeStyle::round(6.0));
```

The default cap is `Butt` and the default join is `Miter`, matching PDF, SVG and every native 2-D
API. AppKit, Qt and Android used to force round caps on every canvas stroke; they now honor the
style, so a line that wants round ends has to ask for it.

## Clipping

[`clip`](https://daybrite.dev/api/day_pieces/struct.Draw.html#method.clip) intersects the current clip, and the only way to widen it again is [`restore`](https://daybrite.dev/api/day_pieces/struct.Draw.html#method.restore). Every
native 2-D context works this way, so there is no "unclip". [`clipped(shape, f)`](https://daybrite.dev/api/day_pieces/struct.Draw.html#method.clipped) wraps the
save/clip/restore for you.

## What each toolkit can and cannot do

Platform guidance: [Apple: Core Graphics](https://developer.apple.com/documentation/coregraphics); [Qt: painting](https://doc.qt.io/qt-6/paintsystem.html).

Everything above works on every toolkit except where noted.

| Toolkit | Limitation |
| --- | --- |
| **Web** | A gradient stroke paints the gradient across the path's interior rather than only the stroked band. Canvas2D has no "convert stroke to path", so there is no region to clip to. It looks correct for thin lines and diverges as the width grows. |
| **windows-winui** | Win2D supports geometric clips, including paths and ellipses, with the same painter for screen and bitmap export. |
| **appkit** | Quadratic segments are elevated to cubics, exactly ([`NSBezierPath`](https://developer.apple.com/documentation/appkit/nsbezierpath)'s own quadratic API is macOS 14+). No visual difference. |
| **gtk**, **arkui** | Same quadratic elevation, for the same reason: cairo and [`OH_Drawing`](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-arkgraphics2d/capi-drawing.md) have cubics. |
| **Web** | [`font_families()`](https://daybrite.dev/api/day_core/fn.font_families.html) is the CSS generic families plus the bundled fonts, not the machine's: a browser lists local fonts only through `queryLocalFonts()`, which is Chromium-only, asynchronous, and behind a permission prompt. `Cap::FontList` answers `Emulated`. |
| **android** | [`font_families()`](https://daybrite.dev/api/day_core/fn.font_families.html) is read from `/system/etc/fonts.xml`, the configuration `Typeface.create` resolves names from, so the system's alias families (`sans-serif-condensed`, `sans-serif-medium`, …) appear as families of their own. |
| **arkui** | Canvas fonts and the font list compile against the SDK's `OH_Drawing_FontMgr` API and are exercised by CI on a device, not by a local emulator run. |
| all | A weight or slant the family does not ship is synthesized where the platform does that (Skia, DirectWrite, CSS) and rounded to the nearest face elsewhere (Pango, CoreText). `face_for` on the family's `FontFamilyInfo` says which face will actually draw. |
| **qt**, **android**, **xaml** | Dash patterns are specified in pixels by Day and converted to those APIs' stroke-width units on the way in. A zero-width stroke falls back to a width of 1 for the conversion. |

Gradient strokes on Apple work by converting the stroke to the region it covers
([`CGContextReplacePathWithStrokedPath`](https://developer.apple.com/documentation/coregraphics/cgcontext/replacepathwithstrokedpath())) and drawing the gradient through that clip, which is exact.

## Text

```rust
d.text(
    &res::str::font_sample().format(),
    Point::new(8.0, 8.0),
    TextStyle {
        size: 24.0,
        color: ink,
        anchor: TextAnchor::LEADING,
        font: CanvasFont {
            family: Some("Pacifico".into()),
            weight: Some(FontWeight::Bold),
            italic: false,
        },
    },
);
d.text(
    &res::str::canvas_value(40_i64).format(),
    center,
    TextStyle {
        size: 22.0,
        color: accent,
        anchor: TextAnchor::CENTERED,
        ..Default::default()
    },
);
```

`TextStyle` is a size in absolute canvas points, a color, an anchor and a [`CanvasFont`](fonts.md):
a family (a platform family or a bundled one, `None` for the platform's face), a weight and a
slant. Fill what you set and take the rest from `..Default::default()`. One line: a newline is
drawn as the engine draws it, not as a line break.

Canvas text takes a size, not a `FontSpec`: it is for labels and type inside a drawing, and it
carries neither the reader's font-scale setting nor RTL mirroring. Anything a user reads as
content belongs in a [`label`](https://daybrite.dev/api/day_pieces/fn.label.html) piece, which does.

### Anchors

`TextAnchor` is one placement per axis, and the toolkit does the alignment:

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
[`day::measure_text`](https://daybrite.dev/api/day/fn.measure_text.html) reports for the same text, size and font — so a drawing that frames its text
from [`measure_text`](https://daybrite.dev/api/day_core/fn.measure_text.html) gets a frame that hugs what is drawn on every toolkit, and
`TextVAlign::Baseline` is `at.y + metrics.ascent` below `Top`.

**Let the anchor do it rather than measuring.** Right-aligning a label by measuring it and
subtracting the width costs a [`measure_text`](https://daybrite.dev/api/day_core/fn.measure_text.html) call per label, and the toolkit is about to lay the
same text out anyway. `day-piece-charts` drew every y-axis label that way until `Trailing`
existed; the measurement per label is now gone. (Measuring is memoized —
[docs/fonts](fonts.md#it-is-cached) — so the repeat is cheap; it is still work that the anchor
does for free and more accurately.)

(Before fonts arrived, gtk, qt, android, arkui and Web put a `Leading` anchor on the baseline
instead; a caller that compensated for that with an offset can drop it.)

The platform's font families, with the faces each ships, come from [`day::font_families()`](https://daybrite.dev/api/day/fn.font_families.html); the
font menu of a drawing app is that list, and [docs/fonts](fonts.md) covers it and the
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
into a value. [`on_tap`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.on_tap) (no location) stays for the common case.

Wire both when a press should count as a pick: a press that never moves is a tap on some toolkits
and a zero-length drag on others, and since both handlers write the same value, a toolkit that
reports both costs nothing. (Day-Sketch, whose tap TOGGLES a selection, cannot be idempotent that
way and carries a 30 ms cross-recognizer guard instead — `src/canvas.rs`'s `handle_click`.)

### Hover

```rust
canvas(draw).on_hover(move |at| pointer.set(at))     // Some(point) inside, None on leaving
```

`Some(point)` in the canvas's own coordinates while the pointer is over it, `None` when it leaves.
**Pointer input only.** Touch-only devices do not produce hover events. Actions available on hover must also be accessible by touch.
For chart selection, handle [`on_tap_at`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.on_tap_at), [`on_drag`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.on_drag), and [`on_hover`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.on_hover).

| Toolkit | Hover | Underneath |
| --- | --- | --- |
| appkit | ✓ | an [`NSTrackingArea`](https://developer.apple.com/documentation/appkit/nstrackingarea) + `mouseEntered:`/`mouseMoved:`/`mouseExited:`. Like Pan, only `DayCanvas` overrides those, so a hover enabled on another view reports nothing |
| gtk | ✓ | [`GtkEventControllerMotion`](https://docs.gtk.org/gtk4/class.EventControllerMotion.html) — enter, motion, leave |
| qt | ✓ | `setMouseTracking(true)` plus `Enter`/`MouseMove`/`Leave` on the event filter |
| xaml | ✓ | `PointerEntered`/`Moved`/`Exited`, gated to a Mouse or Pen device |
| dom | ✓ | `pointerenter`/`move`/`leave`, gated on `pointerType !== 'touch'` |
| uikit | ✓ (pointer) | [`UIHoverGestureRecognizer`](https://developer.apple.com/documentation/uikit/uihovergesturerecognizer) — an iPad with a trackpad, a mouse, or a hovering Pencil. A finger-only device reports nothing |
| android | ✓ (pointer) | `setOnHoverListener` — a mouse or a stylus. A finger generates no `HOVER_*` action at all |
| arkui | — | the C node API's `NODE_ON_HOVER` carries no coordinates, and the contract is a point |

The three toolkits that gate on device type all do it for the same reason: the platform reports a
touch contact through the *same* enter/move/exit events, and passing those through would make every
tap look like a hover.

On exit, `location` is the last point seen INSIDE the node — none of these platforms reports a
useful coordinate with the leave event itself, and "where the pointer went out" is what a handler
wants. Put them on the canvas **before** any wrapping decorator, because
[`.frame`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.frame) and [`.corner_radius`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.corner_radius) build layout nodes of their own, and a point in a wrapper's space
is not a point in the canvas's.

The reference use is [`day-pieces`](/docs/reference/colorpicker)'s composed panel
([docs/colorpicker](colorpicker.md)): its saturation/brightness field, hue strip and opacity
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
the fingers; a toolkit that cannot know it (GTK's scroll controller) reports `Point::ZERO`.

Where they come from: trackpad magnify and two-finger scroll on macOS (a plain mouse wheel also
pans), [`GtkGestureZoom`](https://docs.gtk.org/gtk4/class.GestureZoom.html) and the scroll controller on GTK, native zoom gestures and wheel events
on Qt, and pinch plus a two-finger pan recognizer on iOS; one-finger drags still go to
[`.on_drag`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.on_drag), so selection and panning coexist. DOM also delivers wheel pans (Changed phase,
with negative browser wheel deltas to express content displacement). Android delivers horizontal
pointer-wheel pans. Pinch remains unavailable on those two toolkits.

On AppKit, GTK, Qt and DOM, a non-canvas container with [`.on_pan`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.on_pan) receives horizontal wheel
movement over its descendants, while vertical scrolling keeps its native behavior. Android
containers likewise intercept horizontal pointer-wheel input. Accumulate deltas and suppress
inertial repeats when using a gesture to navigate. Keep visible navigation buttons: XAML and
ArkUI do not deliver these container pans, and touch-only devices may have no wheel input.

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
tracked read. Fifty thousand points drawn one [`fill`](https://daybrite.dev/api/day_pieces/struct.Draw.html#method.fill) at a time is 8.4 MB of ops to build, compare
against the previous frame and clone into the tree — about **3.6 ms per frame before a toolkit
draws anything**. As one stamp it is 800 KB and 1.2 ms: the equality check is 9× faster, the clone
23×, and the whole per-frame overhead 3.1×. On the wire to a serializing toolkit it is a quarter
of the numbers. Day-Viz's scatter records **120,037 marks as 40 ops**.

Toolkits draw a batch as a single native path — one [`NSBezierPath`](https://developer.apple.com/documentation/appkit/nsbezierpath), one `Path2D`, one [`QPainter`](https://doc.qt.io/qt-6/qpainter.html)
transform per copy into one geometry — so the rasterizer is entered once however many copies there
are, rather than once per mark.

## Exporting

The draw function a canvas runs can also render offscreen: to a bitmap, and from there to PNG, or
to a PDF. Export replays the same display list through the same native painter the canvas uses,
so text, gradients, clips and stamps come out as they look on screen.

Write the drawing once as a plain function, and call it from the canvas and from export:

```rust
fn draw_chart(d: &mut Draw, size: Size, series: &[f64]) { /* … */ }

// On screen: layout supplies the size, and reading the signal re-records.
canvas(move |d, size| draw_chart(d, size, &series.get()))

// Offscreen: you supply the size. Recording runs once and subscribes to nothing.
let drawing = Drawing::record(Size::new(600.0, 400.0), |d, size| {
    draw_chart(d, size, &series.get())
});
day::task(async move {
    match drawing.to_png(2.0).await {
        Ok(png) => {
            let _ = save_file(png).suggested_name("chart.png").filter("PNG", &["png"]).await;
        }
        Err(e) => warn!("export failed: {e}"),
    }
});
```

- [`drawing.to_bitmap(scale)`](https://daybrite.dev/api/day_pieces/struct.Drawing.html#method.to_bitmap) resolves to a [`Bitmap`](images.md), the same handle a decode
  returns, so [`image(&bitmap)`](https://daybrite.dev/api/day_pieces/fn.image.html), [`Draw::image`](https://daybrite.dev/api/day_pieces/struct.Draw.html#method.image) and [`bitmap.encode(…)`](https://daybrite.dev/api/day_core/image/struct.Bitmap.html#method.encode) all take it. Its pixel size
  is the drawing's size times `scale`, each side rounded up, and its [`info().scale`](https://daybrite.dev/api/day_core/image/struct.Bitmap.html#method.info) is `scale`, so
  it shows at the drawing's point size.
- [`drawing.to_png(scale)`](https://daybrite.dev/api/day_pieces/struct.Drawing.html#method.to_png) is [`to_bitmap`](https://daybrite.dev/api/day_pieces/struct.Drawing.html#method.to_bitmap) followed by [`encode`](https://daybrite.dev/api/day_core/image/struct.Bitmap.html#method.encode) as PNG.
- [`drawing.to_pdf()`](https://daybrite.dev/api/day_pieces/struct.Drawing.html#method.to_pdf) is a one-page PDF the size of the drawing. `PdfDocument` builds several pages,
  each the size of its drawing:

```rust
const LETTER: Size = Size::new(612.0, 792.0);   // PDF points, the same unit as canvas points

let mut doc = PdfDocument::new().title("Quarterly report").author("Sales");
for region in &regions {
    doc = doc.page(Drawing::record(LETTER, |d, size| draw_region(d, size, region)));
}
let pdf = doc.render().await?;
```

To fit a smaller drawing on a standard page, record at the page size and position the drawing
inside it with [`d.transformed(…)`](https://daybrite.dev/api/day_pieces/struct.Draw.html#method.transformed).

A drawing that rarely changes can also be rendered once and drawn as an image afterwards, which
turns thousands of ops per frame into one.

### Offer only what the target can do

[`day::canvas_exports()`](https://daybrite.dev/api/day/fn.canvas_exports.html) answers `CanvasExports { bitmap, pdf }` from `Cap::CanvasBitmap` and
`Cap::CanvasPdf`, and [`.png()`](https://daybrite.dev/api/day_core/image/struct.CanvasExports.html#method.png) adds whether the toolkit encodes PNG. Keep export commands in the
menu everywhere and disable the ones the target cannot run:

```rust
let exports = day::canvas_exports();
vec![
    menu_item(res::str::export_png()).enabled(exports.png()).action(export_png),
    menu_item(res::str::export_pdf()).enabled(exports.pdf).action(export_pdf),
]
```

A call the target cannot run answers `Err(ExportError::Unsupported)` without reaching the toolkit.

| Target | Bitmap | PDF | How |
| --- | --- | --- | --- |
| **macos-appkit** | yes | yes | A flipped [`CGBitmapContext`](https://developer.apple.com/documentation/coregraphics/cgcontext), and [`CGPDFContext`](https://developer.apple.com/documentation/coregraphics/cgcontext) with title and author |
| **ios-uikit** | yes | yes | [`UIGraphicsImageRenderer`](https://developer.apple.com/documentation/uikit/uigraphicsimagerenderer) and [`UIGraphicsPDFRenderer`](https://developer.apple.com/documentation/uikit/uigraphicspdfrenderer) |
| **macos-gtk**, **linux-gtk**, **windows-gtk** | yes | yes | A cairo image surface, and a cairo PDF surface with title and author |
| **macos-qt**, **linux-qt**, **windows-qt** | yes | yes | [`QImage`](https://doc.qt.io/qt-6/qimage.html), and [`QPdfWriter`](https://doc.qt.io/qt-6/qpdfwriter.html) at 72 dpi so one unit is one point |
| **android-mdc** | yes | yes | A [`Bitmap`](https://developer.android.com/reference/android/graphics/Bitmap)-backed [`Canvas`](https://developer.android.com/reference/android/graphics/Canvas), and [`android.graphics.pdf.PdfDocument`](https://developer.android.com/reference/android/graphics/pdf/PdfDocument). It has no metadata API, so title and author are dropped, and page sizes round to whole points |
| **harmony-arkui** | yes | no | An `OH_Drawing_Bitmap`; HarmonyOS has no PDF drawing context |
| **web-dom** | yes | no | A detached `<canvas>` and `createImageBitmap`, the one toolkit that answers on a later turn; browsers have no PDF writer |
| **windows-winui** | yes | no | Win2D drawing sessions on a `CanvasControl` or `CanvasRenderTarget`; WIC encodes PNG. Vector PDF is not implemented |
| **mock** | synthetic | no | A bitmap of the right size and scale with no pixels, so app logic around export runs headless |

### Rules

- **Units.** Sizes are points. PDF pages are in PDF points, 1/72 inch: US Letter is 612 × 792 and A4
  is 595 × 842.
- **Origin and background.** Top-left origin with y down, as on screen. The background is
  transparent; a drawing that needs an opaque page fills a rectangle first.
- **Theme.** Colors in a drawing are concrete, so an export looks the same in light and dark mode.
  A draw function that reads the theme resolves it when the drawing is recorded.
- **Fonts and images.** Bundled fonts and decoded images draw as they do on screen. An image whose
  handle was released before the render draws nothing.
- **Limits.** A bitmap larger than 16,384 pixels on a side or 64 megapixels in total answers
  `ExportError::TooLarge` before anything is allocated. A zero or non-finite size or scale, and a
  PDF with no pages, answer `ExportError::Invalid`.
- **Threading.** Rendering runs on the UI thread, like every other toolkit call. A very large bitmap
  or a long PDF holds the UI while it renders.

## Performance

`CanvasProps` holds the whole op list and a change replaces it, so a canvas is cheapest when its
op count is stable and small. Two ways to keep it that way:

- **One path over many segments.** Day Trader's chart line went from one `Shape::Line` per sample
  (about 250 ops for a year of daily closes, every corner unjoined) to a single path op.
- **One stamp over many identical marks** — see [Stamping](#stamping) above.

### Geometry is decoded once, not once a frame

A path and a polygon are the only shapes whose geometry does not fit the record's own slots: they
ride a side channel, as text on the serializing toolkits and as a length-prefixed run on the web.
Decoding that — parsing every segment into an [`android.graphics.Path`](https://developer.android.com/reference/android/graphics/Path), a [`QPainterPath`](https://doc.qt.io/qt-6/qpainterpath.html), a
`Path2D` — used to happen on every frame, for drawings that had not moved.

Each of those records now carries a **content key**, and toolkits keep the geometry they decoded
under it. The key is a hash of the geometry itself, never of the op's position or identity, which
is what makes it work for the way canvases are actually written: a draw closure that rebuilds its
`PathBuilder` chain from scratch every frame produces an equal path, so it keys the same and the
decoded geometry is reused. Nothing is asked of the caller — there is no cached path type to hold
on to, and no handle to thread through a draw closure.

Toolkits are free to ignore it: a key of `0` means "none offered", and a decoder that does not
know the slot decodes the payload exactly as it did before.
