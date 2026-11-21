---
navTitle: "Canvases"
title: "Canvas"
description: "The canvas piece records a display list each toolkit replays through its native 2D API: gradients, transforms, and gestures."
---

<!-- Copyright © The Daybrite Project
     SPDX-License-Identifier: CC-BY-SA-4.0 -->

# Canvas

A canvas is a drawing area for content an app constructs from shapes, text, and images. It suits
diagrams, illustrations, and other graphics that need control over how each element is drawn.

A [`canvas`](https://docs.rs/day-pieces/latest/day_pieces/fn.canvas.html) redraws when the state
read by its drawing closure changes.

The [`canvas`](https://docs.rs/day-pieces/latest/day_pieces/fn.canvas.html) piece records a display list (a `Vec<DrawOp>`) that each toolkit replays with its
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
For transitions between two states (a value, a rect, a color, a whole chart),
[`day::tween`](https://docs.rs/day-core/latest/day_core/tween/index.html) supplies the interpolation, the timing and the frame-driven driver ([tween](tween.md)).

Text samples use generated Fluent accessors; see the [example catalog](/docs/reference/localization#reference-example-messages)
and [localization setup](/docs/how-to/localization).

## API

| Op | What it does |
| --- | --- |
| [`fill(shape, paint)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Draw.html#method.fill) | Fill with a color or a linear/radial gradient |
| [`stroke(shape, color, width)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Draw.html#method.stroke) | Stroke at a width, everything else default |
| [`stroke_styled(shape, paint, style)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Draw.html#method.stroke_styled) | Stroke with dash, cap, join, and any paint |
| [`stamp(shape, at, paint)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Draw.html#method.stamp) | Fill ONE shape at many positions, as one op ([Stamping](#stamping)) |
| [`stamp_styled(shape, at, paint, style)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Draw.html#method.stamp_styled) | The same, stroking each copy |
| [`clip(shape)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Draw.html#method.clip) / [`clipped(shape, f)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Draw.html#method.clipped) | Confine what follows to a shape |
| [`text(text, at, style)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Draw.html#method.text) | One line of text at a point, in a size, color and [font](fonts.md) |
| [`image(&bitmap, rect)`](https://docs.rs/day-pieces/latest/day_pieces/fn.image.html) / [`image_with_opacity(…)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Draw.html#method.image_with_opacity) | A decoded [image](images.md) scaled into a rect — the handle, never bytes |
| [`image_region(&bitmap, source, rect)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Draw.html#method.image_region) / [`image_region_with_opacity(…)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Draw.html#method.image_region_with_opacity) | One region of a decoded image, in its pixels, scaled into a rect — a sprite sheet's cell |
| [`save`](https://docs.rs/day-pieces/latest/day_pieces/struct.Draw.html#method.save) / [`restore`](https://docs.rs/day-pieces/latest/day_pieces/struct.Draw.html#method.restore) / [`concat(affine)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Draw.html#method.concat) | Transform and clip state |

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

[`arc_to(center, radius, start, sweep)`](https://docs.rs/day-pieces/latest/day_pieces/struct.PathBuilder.html#method.arc_to) appends a circular arc. Both are
[`Angle`](https://docs.rs/day-geometry/latest/day_geometry/struct.Angle.html)s, `0` = the +x
axis, positive sweeping **clockwise**, the same convention `Shape::Arc` and [`circle`](https://docs.rs/day-pieces/latest/day_pieces/fn.circle.html) use. It is a
*segment*, not a shape, which is what lets a figure made of arcs be one closed contour:

```rust
// A donut wedge: out along the far edge, back along the near one.
let wedge = PathBuilder::new()
    .arc_to(center, 60.0, Angle::deg(20.0), Angle::deg(100.0))    // outer edge, clockwise
    .arc_to(center, 34.0, Angle::deg(120.0), Angle::deg(-100.0))  // inner edge, back the other way
    .close()
    .build();
```

An arc joins whatever came before it — reached by a line when the subpath has a current point,
opening a new one when it does not (including straight after a [`close`](https://docs.rs/day-pieces/latest/day_pieces/struct.PathBuilder.html#method.close)). That is the difference
that matters: `Shape::Arc` draws an arc, [`arc_to`](https://docs.rs/day-pieces/latest/day_pieces/struct.PathBuilder.html#method.arc_to) builds a figure *out of* arcs, and a donut hole,
an angular inset and a corner radius can only compose in the second form. `day-piece-charts` drew
every pie and donut wedge from forty lines of hand-rolled cubics until this existed.

It emits cubics rather than an arc op on the wire. Every rasterizer under Day has its own rule for
joining an arc to the line before it and its own flattening tolerance, so the same beziers
everywhere means the same pixels everywhere. A cubic cannot *be* a circle: the radial error peaks
at 2.7 × 10⁻⁴ of the radius on the quarter turns this splits into — a quarter of a pixel on a
circle a thousand points across — which is the same tradeoff Core Graphics, cairo and every SVG
renderer make.

[`smooth_polyline(&points, tension)`](https://docs.rs/day-pieces/latest/day_pieces/struct.PathBuilder.html#method.smooth_polyline) fits a Catmull-Rom spline through points and emits it as
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
through; converting once at build time is cheaper than converting separately in each toolkit at draw time.

## Rounded corners

`Shape::RoundedRect(rect, r)` rounds every corner alike. [`Shape::rounded_rect(rect, radii)`](https://docs.rs/day-spec/latest/day_spec/enum.Shape.html#method.rounded_rect) takes
a `CornerRadii`, one radius per corner, for the shapes that round only some of them: a bar
standing on its axis ([`CornerRadii::top(r)`](https://docs.rs/day-spec/latest/day_spec/struct.CornerRadii.html#method.top)), one growing rightward from it
([`CornerRadii::right(r)`](https://docs.rs/day-spec/latest/day_spec/struct.CornerRadii.html#method.right)), a tab, a card joined to its neighbor.

```rust
// Rounded where the bar ends, square where it meets the baseline.
d.fill(Shape::rounded_rect(bar, CornerRadii::top(6.0)), TEAL);
```

Radii are fitted to the rectangle the way CSS fits `border-radius`: negative and non-finite radii
are square, and when the two radii along a side add up to more than the side, all four shrink by
the same factor. A generous radius on a short bar degrades to a pill instead of a bow tie, with no
clamping at the call site. The result is the simplest shape that draws it: `Shape::Rect` when every
corner is square, `Shape::RoundedRect` (each toolkit's own) when they agree, and a path of
[`arc_to`](https://docs.rs/day-pieces/latest/day_pieces/struct.PathBuilder.html#method.arc_to) quarter turns otherwise, so nothing new crosses the wire and every toolkit draws the same
curves. [`PathBuilder::rounded_rect(rect, radii)`](https://docs.rs/day-pieces/latest/day_pieces/struct.PathBuilder.html#method.rounded_rect) appends the same contour to a path, for a figure
made of several (a rounded frame with a hole cut out of it).

## Strokes

`StrokeStyle` carries width, cap, join, miter limit, and a dash pattern. [`StrokeStyle::width(w)`](https://docs.rs/day-spec/latest/day_spec/struct.StrokeStyle.html#method.width),
`::dashed(w, pattern)` and `::round(w)` cover the common cases; the rest is struct-update syntax.

```rust
d.stroke_styled(path, SLATE, StrokeStyle::dashed(1.0, vec![5.0, 5.0]));
d.stroke_styled(path, LinearGradient::horizontal(RUST, SKY), StrokeStyle::round(6.0));
```

The default cap is `Butt` and the default join is `Miter`, matching PDF, SVG and every native 2-D
API. Request round caps explicitly for lines with rounded ends.

## Clipping

[`clip`](https://docs.rs/day-pieces/latest/day_pieces/struct.Draw.html#method.clip) intersects the current clip, and the only way to widen it again is [`restore`](https://docs.rs/day-pieces/latest/day_pieces/struct.Draw.html#method.restore). Every
native 2-D context works this way, so there is no "unclip". [`clipped(shape, f)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Draw.html#method.clipped) wraps the
save/clip/restore for you.

## Colors

Every color a canvas takes is an `Ink` ([color.md](color.md)):
[`d.fill(shape, SystemColor::Separator)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Draw.html#method.fill),
[`d.stroke(shape, Ink::adaptive(light, dark), 1.0)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Draw.html#method.stroke), a gradient whose stops are roles. The canvas
records the color of the app's current appearance, so the display list a toolkit draws holds
fixed colors only, and reading a role or a pair subscribes the draw closure to the appearance:
the drawing records again when it changes. A drawing with fixed colors only subscribes to
nothing. For arithmetic on a color (a tint mixed from a role), resolve it yourself with
[`day::resolve(ink)`](https://docs.rs/day-core/latest/day_core/fn.resolve.html), which is tracked the same way.
[`TextStyle::color`](https://docs.rs/day-pieces/latest/day_pieces/struct.TextStyle.html#structfield.color) defaults to
`SystemColor::Text`, so canvas text is legible in both appearances without a color.

## Conic gradients

`ConicGradient` turns its color stops around a center: a color wheel, a gauge's ring, a pie
without wedges. The center is a unit point in the shape's bounding box, like the other
gradients. The start is an `Angle` in canvas space, `0` on the +x axis and clockwise, the
convention arcs use, and a stop's offset is the fraction of a full turn from `start`.

```rust
let ring = Shape::Arc { rect, start: Angle::deg(135.0), sweep: Angle::deg(270.0) };
d.stroke_styled(
    ring,
    ConicGradient::centered(Angle::deg(135.0), [(0.0, cool), (0.75, hot)]),
    StrokeStyle::round(10.0),
);
```

The colors meet at `start`. For a seamless wheel, make the first and last stops the same
color.

## Shadows

[`shadowed(shadow, f)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Draw.html#method.shadowed) draws `f` with every fill, stroke, text and image in it casting a shadow:

```rust
d.shadowed(Shadow::new(Color::rgba(0.0, 0.0, 0.0, 0.3), 16.0).offset(0.0, 6.0), |d| {
    d.fill(Shape::RoundedRect(card, 14.0), Color::WHITE);
});
```

- `blur` is the CSS `box-shadow` blur radius: the width of the soft edge.
- The offset and the blur are in points and are **not** scaled or rotated by the current
  transform. A shape drawn under
  [`scale(2.0, 2.0)`](https://docs.rs/day-geometry/latest/day_geometry/struct.Affine.html#method.scale) still casts its shadow `dy` points down.
- Each drawing call casts its own shadow, so two overlapping shapes in one scope shade each
  other. Draw them as one path when they should cast one shadow.
- The shadow is graphics state, like the clip. [`d.shadow(Some(..))`](https://docs.rs/day-pieces/latest/day_pieces/struct.Draw.html#method.shadow) sets it until the
  enclosing `restore`, and [`d.shadow(None)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Draw.html#method.shadow) stops it.

## Blend modes

[`blended(mode, f)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Draw.html#method.blended) draws `f` combined with what the canvas already holds:

```rust
d.fill(Shape::RoundedRect(ticket, 14.0), paper);
d.blended(BlendMode::Erase, |d| d.fill(Shape::Ellipse(hole), Color::BLACK));
```

| Mode | Result |
| --- | --- |
| `Normal` | The new drawing covers the old by its alpha |
| `Multiply` | Always darker; white changes nothing |
| `Screen` | Always lighter; black changes nothing |
| `Overlay` | Multiplies dark backdrops and screens light ones |
| `Darken`, `Lighten` | The darker or the lighter of the two, per channel |
| `Add` | Adds the colors, clamped |
| `Erase` | Removes what has been drawn, by the new drawing's alpha |

The backdrop is the canvas's own drawing, never the views behind it: a canvas is its own
compositing group. `Erase` leaves the canvas transparent, so whatever is behind the canvas view
shows through the hole. The mode is graphics state, scoped by `save` and `restore`.

## Toolkit support

Platform guidance: [Apple: Core Graphics](https://developer.apple.com/documentation/coregraphics); [Qt: painting](https://doc.qt.io/qt-6/paintsystem.html).

Everything above works on every toolkit except where noted.

| Toolkit | Limitation |
| --- | --- |
| **[Web](https://daybrite.dev/docs/reference/platforms/web-dom "toolkit:web-dom")** | A gradient stroke paints the gradient across the path's interior rather than only the stroked band. Canvas2D has no "convert stroke to path", so there is no region to clip to. It looks correct for thin lines and diverges as the width grows. |
| **[windows-winui](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui")** | Win2D supports geometric clips, including paths and ellipses, with the same painter for screen and bitmap export. |
| **[appkit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit")**, **[uikit](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit")** | Solid fills, solid strokes and clips of plain shapes go straight to the Core Graphics context as path calls, the rest through [`NSBezierPath`](https://developer.apple.com/documentation/appkit/nsbezierpath) or `UIBezierPath`. No visual difference; a dense drawing paints several times faster. Where a path object is still used on [AppKit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit"), quadratic segments are elevated to cubics, exactly (`NSBezierPath`'s own quadratic API is [macOS](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") 14+). |
| **[gtk](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk")**, **[arkui](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui")** | Same quadratic elevation, for the same reason: cairo and [`OH_Drawing`](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-arkgraphics2d/capi-drawing.md) have cubics. |
| **[Web](https://daybrite.dev/docs/reference/platforms/web-dom "toolkit:web-dom")** | [`font_families()`](https://docs.rs/day-core/latest/day_core/fn.font_families.html) is the CSS generic families plus the bundled fonts, not the machine's: a browser lists local fonts only through `queryLocalFonts()`, which is Chromium-only, asynchronous, and behind a permission prompt. `Cap::FontList` answers `Emulated`. |
| **[android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material")** | [`font_families()`](https://docs.rs/day-core/latest/day_core/fn.font_families.html) is read from `/system/etc/fonts.xml`, the configuration `Typeface.create` resolves names from, so the system's alias families (`sans-serif-condensed`, `sans-serif-medium`, …) appear as families of their own. |
| **[arkui](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui")** | Canvas fonts and the font list compile against the SDK's `OH_Drawing_FontMgr` API and are exercised by CI on a device, not by a local emulator run. |
| all | A weight or slant the family does not ship is synthesized where the platform does that (Skia, DirectWrite, CSS) and rounded to the nearest face elsewhere (Pango, CoreText). `face_for` on the family's `FontFamilyInfo` says which face will actually draw. |
| **appkit**, **uikit** | Conic gradients are Core Graphics' own on macOS 14 and iOS 17. Below that, AppKit computes the gradient per pixel and UIKit draws half-degree wedges, both through the shape's clip. In PDF export `Add` draws as `Normal` and drawing under `Erase` is omitted. |
| **android** | Shadows are a blurred pass per drawing call (`BlurMaskFilter`); color-bitmap glyphs cast none, and on API 24 to 27 a canvas that sets a shadow renders in a software layer. On API 24 to 28 `Multiply` over a part of the canvas with nothing drawn on it draws nothing. |
| **gtk** | Conic gradients are a cairo mesh; hard color edges between stops are not antialiased. Shadows are an offscreen mask and blur per drawing call, an image in PDF. |
| **qt** | Shadows are an offscreen layer and blur per drawing call. A canvas that uses a blend mode composes offscreen, so its text is grayscale-antialiased. In PDF export blend modes draw as `Normal`, drawing under `Erase` is omitted, and shadows and conic gradients are images. |
| **winui** | Conic gradients are a fan of 360 to 1440 flat wedges and do not apply to text. `Multiply`, `Screen`, `Overlay`, `Darken` and `Lighten` paint the canvas through two offscreen targets; a canvas too large for a texture draws them as `Normal`. |
| **web** | Conic gradients are the engine's own; on Safari before 16.1 they are a wedge fan. |
| **arkui** | A gradient-painted shape casts its shadow at full strength. A canvas that uses a blend mode is drawn through a layer its own size, so a shadow reaching past its edge is cut there. |
| **[Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt")**, **[Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material")**, **[WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui")** | Dash patterns are specified in pixels by Day and converted to those APIs' stroke-width units on the way in. A zero-width stroke falls back to a width of 1 for the conversion. |

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
content belongs in a [`label`](https://docs.rs/day-pieces/latest/day_pieces/fn.label.html) piece, which does.

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
[`day::measure_text`](https://docs.rs/day-app/latest/day/fn.measure_text.html) reports for the same text, size and font — so a drawing that frames its text
from [`measure_text`](https://docs.rs/day-core/latest/day_core/fn.measure_text.html) gets a frame that hugs what is drawn on every toolkit, and
`TextVAlign::Baseline` is `at.y + metrics.ascent` below `Top`.

Use a trailing text anchor for right-aligned labels. It aligns the text without a separate
measurement and subtraction at each call site.

(Before fonts arrived, gtk, qt, android, arkui and Web put a `Leading` anchor on the baseline
instead; a caller that compensated for that with an offset can drop it.)

The platform's font families, with the faces each ships, come from [`day::font_families()`](https://docs.rs/day-app/latest/day/fn.font_families.html); the
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
into a value. [`on_tap`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_tap) (no location) stays for the common case.

Wire both when a press should count as a pick: a press that never moves is a tap on some toolkits
and a zero-length drag on others, and since both handlers write the same value, a toolkit that
reports both costs nothing. (Day-Sketch, whose tap TOGGLES a selection, cannot be idempotent that
way and carries a 30 ms cross-recognizer guard instead — `src/canvas.rs`'s `handle_click`.)

### Keys

A canvas takes focus on a press and hears the arrows and digits through
[`.on_key`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_key);
with [`.keyboard()`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.keyboard)
declared it hears every key, with the text a press typed kept apart from the key's name, and
raises the soft keyboard on a phone. Both are [docs/menus.md](menus.md)'s "Keys follow focus"
and "The full route".

### Hover

```rust
canvas(draw).on_hover(move |at| pointer.set(at))     // Some(point) inside, None on leaving
```

`Some(point)` in the canvas's own coordinates while the pointer is over it, `None` when it leaves.
**Pointer input only.** Touch-only devices do not produce hover events. Actions available on hover must also be accessible by touch.
For chart selection, handle [`on_tap_at`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_tap_at), [`on_drag`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_drag), and [`on_hover`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_hover).

| Toolkit | Hover | Underneath |
| --- | --- | --- |
| [appkit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") | ✓ | an [`NSTrackingArea`](https://developer.apple.com/documentation/appkit/nstrackingarea) + `mouseEntered:`/`mouseMoved:`/`mouseExited:`. Like Pan, only `DayCanvas` overrides those, so a hover enabled on another view reports nothing |
| [gtk](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") | ✓ | [`GtkEventControllerMotion`](https://docs.gtk.org/gtk4/class.EventControllerMotion.html) — enter, motion, leave |
| [qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") | ✓ | `setMouseTracking(true)` plus `Enter`/`MouseMove`/`Leave` on the event filter |
| [WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") | ✓ | `PointerEntered`/`Moved`/`Exited`, gated to a Mouse or Pen device |
| [dom](https://daybrite.dev/docs/reference/platforms/web-dom "toolkit:web-dom") | ✓ | `pointerenter`/`move`/`leave`, gated on `pointerType !== 'touch'` |
| [uikit](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") | ✓ (pointer) | [`UIHoverGestureRecognizer`](https://developer.apple.com/documentation/uikit/uihovergesturerecognizer) — an iPad with a trackpad, a mouse, or a hovering Pencil. A finger-only device reports nothing |
| [android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") | ✓ (pointer) | `setOnHoverListener` — a mouse or a stylus. A finger generates no `HOVER_*` action at all |
| [arkui](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") | — | the C node API's `NODE_ON_HOVER` carries no coordinates, and the contract is a point |

The toolkits that gate on device type all do it for the same reason: the platform reports a
touch contact through the *same* enter/move/exit events, and passing those through would make every
tap look like a hover.

On exit, `location` is the last point seen INSIDE the node — none of these platforms reports a
useful coordinate with the leave event itself, and "where the pointer went out" is what a handler
wants. Put them on the canvas **before** any wrapping decorator, because
[`.frame`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.frame) and [`.corner_radius`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.corner_radius) build layout nodes of their own, and a point in a wrapper's space
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
on Qt, pinch plus a two-finger pan recognizer on iOS, a `ScaleGestureDetector` on Android, a
scale manipulation on WinUI, and a two-finger pinch recognizer on ArkUI; one-finger drags still
go to [`.on_drag`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_drag), so selection and panning coexist, and a second finger ends the drag
rather than doubling it. In a browser, two fingers pinch, and a trackpad pinch arrives as the
ctrl+wheel the browser reports it as. Each tick is a lone `Changed` whose `scale` is the tick's
own factor, and the page's own zoom is suppressed. Safari reports a trackpad pinch through its
`gesturestart`/`gesturechange`/`gestureend` events instead, and those arrive as a bracketed
pinch with a cumulative scale, like two fingers. A ctrl+wheel on WinUI and a touchpad pinch
on ArkUI arrive the same way. Wheel pans are lone `Changed` events too. DOM reports them with
negative browser wheel deltas, to express content displacement; WinUI reports each wheel tick
(shift, or a tilt wheel, for sideways); ArkUI reports a two-finger pan and wheel ticks; Android
reports horizontal pointer-wheel pans.

A third gesture turns the view, for a canvas whose content has a heading (a map's bearing, a
photo being straightened):

```rust
canvas(draw)
    .on_rotate(move |g| turn_about(g.location, g.rotation, g.phase))
```

`Rotate.rotation` is an `Angle` ([`.radians()`](https://docs.rs/day-geometry/latest/day_geometry/struct.Angle.html#method.radians), [`.degrees()`](https://docs.rs/day-geometry/latest/day_geometry/struct.Angle.html#method.degrees)), positive clockwise on screen, and cumulative like the pinch's
scale: the total turn since the gesture began, applied to the bearing captured at
`DragPhase::Began`. Where a platform reports a trackpad rotation without a gesture session,
each rotate tick arrives as a lone `Changed` carrying that tick's own angle, the way a ctrl+wheel
arrives as a pinch. A handler that saw no `Began` adds the tick to the current bearing. `location` is the
centroid of the fingers, or `Point::ZERO` where the platform reports none.

A twist and a pinch are one hand movement, so every backend delivers both from the same two
fingers rather than choosing one, and a gesture reports each quantity only when it changes.
Where it comes from: the trackpad's rotate events on macOS (bracketed by the event phases, with
the magnify recognizer still reading the same gesture), a `UIRotationGestureRecognizer` on iOS
that recognizes alongside the pinch and two-finger pan, the angle between the first two pointers
in the same touch handling that feeds Android's `ScaleGestureDetector`,
[`GtkGestureRotate`](https://docs.gtk.org/gtk4/class.GestureRotate.html) on GTK, native rotate
gestures on Qt (sharing the pinch's event filter), the same manipulation's rotation on WinUI,
and a rotation recognizer grouped in parallel with the pinch on ArkUI. In a browser, two fingers
turn through the same pointer tracking as the pinch, and Safari's
`gesturestart`/`gesturechange`/`gestureend` events carry a trackpad rotation as a bracketed,
cumulative angle alongside their scale. A one-finger drag still goes to
[`.on_drag`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_drag), and a
second finger ends it as it does for a pinch. Fingers turn a little in every pinch, so a handler
that should not rotate on a zoom ignores turns below a few degrees until the gesture commits to
one. Keep a visible way back to the default heading (a compass button), as with zoom.

On AppKit, GTK, Qt, DOM, WinUI and ArkUI, a non-canvas container with [`.on_pan`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_pan) receives horizontal wheel
movement over its descendants, while vertical scrolling keeps its native behavior. Android
containers likewise intercept horizontal pointer-wheel input. Accumulate deltas and suppress
inertial repeats when using a gesture to navigate. Keep visible navigation buttons: touch-only
devices may have no wheel input.

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
tracked read. Fifty thousand points drawn one [`fill`](https://docs.rs/day-pieces/latest/day_pieces/struct.Draw.html#method.fill) at a time is 8.4 MB of ops to build, compare
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

- [`drawing.to_bitmap(scale)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Drawing.html#method.to_bitmap) resolves to a [`Bitmap`](images.md), the same handle a decode
  returns, so [`image(&bitmap)`](https://docs.rs/day-pieces/latest/day_pieces/fn.image.html), [`Draw::image`](https://docs.rs/day-pieces/latest/day_pieces/struct.Draw.html#method.image) and [`bitmap.encode(…)`](https://docs.rs/day-core/latest/day_core/image/struct.Bitmap.html#method.encode) all take it. Its pixel size
  is the drawing's size times `scale`, each side rounded up, and its [`info().scale`](https://docs.rs/day-core/latest/day_core/image/struct.Bitmap.html#method.info) is `scale`, so
  it shows at the drawing's point size.
- [`drawing.to_png(scale)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Drawing.html#method.to_png) is [`to_bitmap`](https://docs.rs/day-pieces/latest/day_pieces/struct.Drawing.html#method.to_bitmap) followed by [`encode`](https://docs.rs/day-core/latest/day_core/image/struct.Bitmap.html#method.encode) as PNG.
- [`drawing.to_pdf()`](https://docs.rs/day-pieces/latest/day_pieces/struct.Drawing.html#method.to_pdf) is a one-page PDF the size of the drawing. `PdfDocument` builds several pages,
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
inside it with [`d.transformed(…)`](https://docs.rs/day-pieces/latest/day_pieces/struct.Draw.html#method.transformed).

A drawing that rarely changes can also be rendered once and drawn as an image afterwards, which
turns thousands of ops per frame into one.

### Offer only what the target can do

[`day::canvas_exports()`](https://docs.rs/day-app/latest/day/fn.canvas_exports.html) answers `CanvasExports { bitmap, pdf }` from `Cap::CanvasBitmap` and
`Cap::CanvasPdf`, and [`.png()`](https://docs.rs/day-core/latest/day_core/image/struct.CanvasExports.html#method.png) adds whether the toolkit encodes PNG. Keep export commands in the
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
| **[macos-appkit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit")** | yes | yes | A flipped [`CGBitmapContext`](https://developer.apple.com/documentation/coregraphics/cgcontext), and [`CGPDFContext`](https://developer.apple.com/documentation/coregraphics/cgcontext) with title and author |
| **[ios-uikit](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit")** | yes | yes | [`UIGraphicsImageRenderer`](https://developer.apple.com/documentation/uikit/uigraphicsimagerenderer) and [`UIGraphicsPDFRenderer`](https://developer.apple.com/documentation/uikit/uigraphicspdfrenderer) |
| **[macos-gtk](https://daybrite.dev/docs/reference/platforms "toolkit:macos-gtk")**, **[linux-gtk](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk")** | yes | yes | A cairo image surface, and a cairo PDF surface with title and author |
| **[macos-qt](https://daybrite.dev/docs/reference/platforms "toolkit:macos-qt")**, **[linux-qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt")** | yes | yes | [`QImage`](https://doc.qt.io/qt-6/qimage.html), and [`QPdfWriter`](https://doc.qt.io/qt-6/qpdfwriter.html) at 72 dpi so one unit is one point |
| **[android-material](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material")** | yes | yes | A [`Bitmap`](https://developer.android.com/reference/android/graphics/Bitmap)-backed [`Canvas`](https://developer.android.com/reference/android/graphics/Canvas), and [`android.graphics.pdf.PdfDocument`](https://developer.android.com/reference/android/graphics/pdf/PdfDocument). It has no metadata API, so title and author are dropped, and page sizes round to whole points |
| **[harmony-arkui](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui")** | yes | no | An `OH_Drawing_Bitmap`; [HarmonyOS](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") has no PDF drawing context |
| **[web-dom](https://daybrite.dev/docs/reference/platforms/web-dom "toolkit:web-dom")** | yes | no | A detached `<canvas>` kept as the bitmap, the one toolkit that answers on a later turn; browsers have no PDF writer |
| **[windows-winui](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui")** | yes | no | Win2D drawing sessions on a `CanvasControl` or `CanvasRenderTarget`; WIC encodes PNG. Vector PDF is not implemented |
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
- **Threading.** On macOS, iOS and Android a bitmap paints on a painting thread, not the UI
  thread: Core Graphics, `UIGraphicsImageRenderer` and a `Bitmap`-backed `Canvas` all draw into
  memory from any thread, so a long replay, such as a map tile of a thousand paths, does not
  stall the app while it renders. The toolkit resolves on the UI thread, before the paint starts,
  everything the painter would otherwise read there (the decoded images the drawing uses, the
  fonts its text needs), and the finished bitmap and its answer come back on the UI thread, in
  that order. Day keeps a few painting threads for the whole process, one short of the cores and
  never more than four, so a screenful of requests cannot crowd out the app's own work; a request
  waits its turn rather than starting a thread. Several requests may be in flight at once, and
  each answer carries its own request, so they may complete in any order. A PDF, and a bitmap on
  the other toolkits, still renders on the UI thread. Android also hands the list to its painter
  with path and polygon points as numbers rather than text, which was the other half of a tile's
  cost there.

## Performance

`CanvasProps` holds the whole op list and a change replaces it, so a canvas is cheapest when its
op count is stable and small. Two ways to keep it that way:

- **One path over many segments.** Day Trader's chart line went from one `Shape::Line` per sample
  (about 250 ops for a year of daily closes, every corner unjoined) to a single path op.
- **One stamp over many identical marks** — see [Stamping](#stamping) above.

### Reuse geometry

Keep paths and polygons stable when their geometry has not changed. Toolkits cache decoded
geometry, so animating a transform can reuse a path instead of rebuilding its segments.
