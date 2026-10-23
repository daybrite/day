---
title: "Shapes"
description: "Vector shape pieces — rectangles, circles, paths — drawn through each platform's native layer API."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Shapes: design & implementation

Draw shapes that respond to app state, animation, and gestures. The [`shape`](https://daybrite.dev/api/day_pieces/fn.shape.html) piece supports
geometry, fills, strokes, and transforms; shape groups combine several shapes in one canvas.

## 1. Goal & constraints

Give Day SwiftUI's shape ergonomics (`Circle`, `Rectangle`, `RoundedRectangle`, `Capsule`,
`Ellipse`, arbitrary `Path`, and custom shapes) as regular Pieces that:

- are bound to signals (geometry and style),
- are identified ([`.id`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.id)) and accessible ([`.a11y`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.a11y)),
- are interactive through the gesture events, with path-precise hit testing,
- are animatable,
- are built atop the internal canvas API (so they work on all six toolkits day one),
- read naturally for Day and Rust: a single data-oriented [`shape`](https://daybrite.dev/api/day_pieces/fn.shape.html) piece parameterized by kind
  rather than one node kind per shape.

### What Day already gives us (the substrate)

- [`canvas(|d: &mut Draw, size: Size| …)`](https://daybrite.dev/api/day_pieces/fn.canvas.html): a reactive leaf (`kinds::CANVAS`) whose closure re-runs
  on any tracked signal read (and on `FrameChanged`), records a `Vec<DrawOp>`, diffs it
  (`DrawOp: PartialEq`), and replays only on change. Toolkits turn `DrawOp`s into native drawing
  (CoreGraphics / cairo / [QPainter](https://doc.qt.io/qt-6/qpainter.html) / Canvas.draw* / DirectWrite), one FFI hop per redraw.
- `Shape { Rect(Rect), RoundedRect(Rect, f64), Ellipse(Rect), Arc{rect,start_deg,sweep_deg},
  Line(Point,Point), Polygon(Vec<Point>) }`: the geometry enum (`day-spec`).
- [`Draw::fill(Shape, Color)`](https://daybrite.dev/api/day_pieces/struct.Draw.html#method.fill), [`Draw::stroke(Shape, Color, f64)`](https://daybrite.dev/api/day_pieces/struct.Draw.html#method.stroke), [`Draw::text(...)`](https://daybrite.dev/api/day_pieces/struct.Draw.html#method.text).
- `Color { r,g,b,a }` (rgba/hex), a `LinearGradient` via [`.fill_linear(...)`](https://daybrite.dev/api/day_pieces/trait.ShapePieceBuilder.html#tymethod.fill_linear), or a `RadialGradient` via [`.fill_radial(...)`](https://daybrite.dev/api/day_pieces/trait.ShapePieceBuilder.html#tymethod.fill_radial) (angular is a later phase).
- `Decorate` blanket impl → [`.id`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.id)/[`.id_of`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.id_of)/[`.a11y`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.a11y)/[`.frame`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.frame)/[`.padding`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.padding)/[`.any`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.any) for every Piece.
- Reserved hooks: `AnimSpec { duration_ms }` threaded through `update`/`set_frame`; and
  the frame-clock ticker used for canvas animation.

### Prior art consulted

- **SwiftUI**: `Shape: path(in rect) -> Path`; shapes are *frame-relative* views that fill their
  proposed size; [`.fill(ShapeStyle)`](https://daybrite.dev/api/day_pieces/trait.ShapePieceBuilder.html#tymethod.fill), [`.stroke(_, lineWidth:)`](https://daybrite.dev/api/day_pieces/trait.ShapePieceBuilder.html#tymethod.stroke), `.trim(from:to:)`, [`.rotation()`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.rotation);
  `Path` for arbitrary geometry; `Canvas` for dense immediate-mode drawing.
- **floem**: shapes are drawn in the render pass; no separate node per shape (dense-draw model).
- **hop/** (this workspace): shipped a native `ShapeSpec` pipeline with [`Shape.fill(gradient)`](https://daybrite.dev/api/day_pieces/trait.ShapePieceBuilder.html#tymethod.fill) and
  `LinearGradient/RadialGradient/AngularGradient` native on all four toolkits, and painted-path
  transforms (rotate/scale/offset are path-only, matching SwiftCrossUI).
- **pane/ DESIGN2.md** (this workspace): "style-as-data struct args" over the SwiftUI modifier
  tower; informs the `Paint`/`StrokeStyle` value types below.

## 2. Design decisions (with rationale)

**D1: Shapes are frame-relative, not absolute.** A `Circle` inscribes the rect the layout
engine assigns; a `Rectangle` fills it; `RoundedRectangle`'s corner is the only extra parameter.
You size a shape with [`.frame(w, h)`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.frame) (or a bounded parent), exactly like SwiftUI. This is more
composable than "circle of radius r": shapes drop into stacks, grow/shrink with layout, and animate
their frame for free. (The user's `radius = 1.0` sketch is absolute; the frame-relative model
recovers absolute sizing via [`.frame(d, d)`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.frame).)

**D2: One piece, parameterised by a data `ShapeKind`.** There is no separate node kind per shape.
Rust's enum-with-fields is the "params bag": type-safe, and simpler than a mutable params closure.
Convenience free functions ([`circle()`](https://daybrite.dev/api/day_pieces/fn.circle.html), [`rounded_rectangle(12.0)`](https://daybrite.dev/api/day_pieces/fn.rounded_rectangle.html)) give SwiftUI ergonomics; the
unified [`shape(kind)`](https://daybrite.dev/api/day_pieces/fn.shape.html) gives the data-oriented form. Both construct the same `ShapePiece`.

**D3: Render atop canvas; the renderer is an implementation detail behind a stable API.** v1
lowers a `ShapePiece` to the existing canvas display-list (zero toolkit work, works everywhere,
free reactivity, free unit tests). The same API can later lower hot or native-fidelity shapes to a
native `SHAPE` leaf (Proposal B), a capability-gated choice with no API change.

**D4: Reactivity is free.** Because the canvas closure re-runs on tracked reads, every shape
parameter and style may be a value, a `Signal`, or a closure with no per-prop binding code, a
strict simplification over native leaves. Shape params are stored as a small `Reactive<T>` source.

**D5: Interaction is path-precise and day-side.** The shape knows its path, so on a `Tap(point)`
from its canvas leaf, Day tests the point against the path in Rust before firing [`.on_tap`](https://daybrite.dev/api/day_pieces/trait.ShapePieceBuilder.html#tymethod.on_tap). That
needs no toolkit change and is more precise than bounding-box hit testing.

**D6: Two animation paths; shapes use the canvas one.** Day has (i) toolkit-executed animation for
native widgets (the `AnimSpec` parameter) and (ii) a day-driven canvas frame-clock. Shapes
animate by interpolating params CPU-side and re-recording per frame, the same way SwiftUI renders
shape animation. See [frame scheduling](frames.md) for the frame-clock API.

## 3. Proposal A (recommended): the canvas-backed [`shape`](https://daybrite.dev/api/day_pieces/fn.shape.html) piece

### 3.1 Geometry: `ShapeKind` (data)

```rust
/// A shape's geometry, resolved against the rect the layout engine assigns (frame-relative).
#[derive(Clone)]
pub enum ShapeKind {
    Rectangle,
    RoundedRectangle { corner: Corner },
    Circle,                              // inscribed centered circle (min(w,h))
    Ellipse,                             // fills the rect
    Capsule,                             // RoundedRectangle with corner = min(w,h)/2
    Arc { start_deg: f64, sweep_deg: f64 },   // stroked arc of the inscribed ellipse
    Line { from: UnitPoint, to: UnitPoint },  // stroke-only segment between unit points
    Polygon { points: Rc<[UnitPoint]> },      // point-list polygon in unit space
    // ── phase 2 (each adds one canvas op + geometry, no new node) ──
    // Path(PathData),                                  // arbitrary
    // Custom(Rc<dyn Fn(Rect) -> PathData>),            // the `Shape` protocol analogue
}

/// A corner radius as absolute points or a fraction (0..1) of min(w,h).
#[derive(Clone, Copy)]
pub enum Corner { Fixed(f64), Fraction(f64) }
impl From<f64> for Corner { fn from(v: f64) -> Self { Corner::Fixed(v) } }

impl ShapeKind {
    /// Lower to a drawable path within `rect` (v1: the existing `Shape` enum; phase 2: `PathData`).
    fn resolve(&self, rect: Rect) -> Geometry { /* Circle → Ellipse(square inset & centered), … */ }
}
```

`resolve` maps each kind to the existing `day_spec::Shape`, so no `day-spec` change is required,
including for `Line` and `Polygon`, whose `Shape::Line`/`Shape::Polygon` ops every toolkit already
replays. [`line((fx, fy), (fx, fy))`](https://daybrite.dev/api/day_pieces/fn.line.html) and [`polygon([(fx, fy), …])`](https://daybrite.dev/api/day_pieces/fn.polygon.html) are the tuple-friendly sugar.
Line and Polygon semantics differ from the closed kinds:

- **Unit-point geometry, unclamped:** Points resolve as fractions of the rect (like gradient
  `UnitPoint`s) and may sit outside 0..1; a glyph's flourish can extend past its box.
- **No stroke-half inset:** closed kinds inset by `stroke/2` so a centered stroke stays inside the
  frame (SwiftUI `strokeBorder` behavior); Line and Polygon resolve exactly at their authored
  points, and a stroked segment touching the frame edge may clip on the clipping toolkits
  (Qt/Android/XAML), exactly as raw canvas does.
- **Line is stroke-only** ([`.fill`](https://daybrite.dev/api/day_pieces/trait.ShapePieceBuilder.html#tymethod.fill) records nothing; a segment has no interior), and its rect may
  be degenerate (a horizontal line in a zero-height sub-rect), so it skips the empty-rect bail.
- **Polygon hit-testing is path-precise** (even-odd ray cast), keeping the D5 promise.

A regular n-gon (`Polygon { sides, rotation }` in earlier drafts) is future sugar that computes
inscribed unit points; the point-list form subsumes it.

Placement: **[`.at(fx, fy, fw, fh)`](https://daybrite.dev/api/day_pieces/trait.ShapePieceBuilder.html#tymethod.at)** resolves the shape inside that fractional sub-rect of its
bounds (applied before [`.inset`](https://daybrite.dev/api/day_pieces/trait.ShapePieceBuilder.html#tymethod.inset)). It exists for composition: hand-drawn glyph code full of
[`Rect::new(ox + fx * s, oy + fy * s, fw * s, fh * s)`](https://daybrite.dev/api/day_geometry/struct.Rect.html#method.new) translates 1:1 into [`.at(fx, fy, fw, fh)`](https://daybrite.dev/api/day_pieces/trait.ShapePieceBuilder.html#tymethod.at)
children of a group. It works on a standalone shape too.

### 3.2 Style: `Paint` and `Stroke` (data, growable)

```rust
/// A fill source (day-spec). `Solid` and `Linear` are IMPLEMENTED: `DrawOp::Fill(Shape, Paint)`
/// replays native gradients on every backend (NSGradient, CGGradient, cairo pattern,
/// QLinearGradient, Android LinearGradient shader, XAML LinearGradientBrush, OH_Drawing shader
/// effect); gradient unit points resolve against the filled shape's bounding box, and the packed
/// encoding carries stops on the texts channel (kind 14 — see day-spec::encode_ops).
#[derive(Clone, Debug, PartialEq)]
pub enum Paint {
    Solid(Color),
    Linear(LinearGradient),   // { start: UnitPoint, end: UnitPoint, stops: Vec<(f64, Color)> }
    Radial(RadialGradient),   // { center: UnitPoint, radius: f64 (unit, elliptical-to-bounds), stops }
    // ── later phases ──
    // Angular { stops: Vec<(f64, Color)>, center: UnitPoint, start_deg: f64 } — native on
    //   Apple (CGContextDrawConicGradient) / Qt (QConicalGradient) / Android (SweepGradient) /
    //   OH_Drawing (sweep); needs a wedge-fan fallback on cairo + XAML XAML (the hop recipe).
    // Token(SemanticColor),                            // Theme tokens, late-bound
}
impl From<Color> for Paint { /* … */ }
impl From<LinearGradient> for Paint { /* … */ }
impl From<RadialGradient> for Paint { /* … */ }

#[derive(Clone)]
pub struct Stroke {
    pub paint: Reactive<Paint>,
    pub width: Reactive<f64>,
    // phase 2: pub cap: LineCap, pub join: LineJoin, pub dash: Vec<f64>,
}
```

### 3.3 The piece + builder

```rust
pub struct ShapePiece {
    kind:   Reactive<ShapeKind>,
    fill:   Option<Reactive<Paint>>,
    stroke: Option<Stroke>,
    inset:  Reactive<f64>,           // uniform inset before resolving (stroke-safe by default)
    // reserved seams (design now, wire later):
    on_tap: Option<Rc<dyn Fn()>>,    // path-precise
    anim:   Option<Animation>,       // implicit-animate reads
    // phase 2: trim: Option<(Reactive<f64>, Reactive<f64>)>, rotation: Reactive<f64>, …
}

/// The unified, data-oriented constructor (the user's `shape(type:, params:)`).
pub fn shape(kind: impl IntoReactive<ShapeKind>) -> ShapePiece { /* … */ }

/// SwiftUI-ergonomic sugar; all construct the same `ShapePiece`.
pub fn rectangle() -> ShapePiece                       { shape(ShapeKind::Rectangle) }
pub fn circle() -> ShapePiece                          { shape(ShapeKind::Circle) }
pub fn ellipse() -> ShapePiece                         { shape(ShapeKind::Ellipse) }
pub fn capsule() -> ShapePiece                         { shape(ShapeKind::Capsule) }
pub fn rounded_rectangle(c: impl Into<Corner>) -> ShapePiece {
    shape(ShapeKind::RoundedRectangle { corner: c.into() })
}

impl ShapePiece {
    pub fn fill(mut self, p: impl IntoReactive<Paint>) -> Self { /* … */ self }
    pub fn stroke(mut self, p: impl IntoReactive<Paint>, w: impl IntoReactive<f64>) -> Self { /* … */ self }
    pub fn inset(mut self, v: impl IntoReactive<f64>) -> Self { /* … */ self }
    pub fn on_tap(mut self, f: impl Fn() + 'static) -> Self { /* … */ self }   // reserved
    pub fn animation(mut self, a: Animation) -> Self { /* … */ self }          // reserved
}
```

`IntoReactive<T>` is a marker-trait conversion accepting `T`, `Signal<T>`, or `Fn() -> T`. It
generalizes today's `IntoText`/`IntoFraction` into one reusable source type
`Reactive<T> = Const(T) | Dyn(Rc<dyn Fn() -> T>)`. (Adopting it also lets us collapse the existing
per-type `*Source` enums, an optional cleanup.)

### 3.4 Lowering to canvas (the whole implementation, v1)

```rust
impl Piece for ShapePiece {
    fn build(self, cx: &mut BuildCx) -> RNode {
        // A shape greedily fills its proposed size (SwiftUI semantics): a canvas leaf with grow.
        let node = cx.leaf(kinds::CANVAS, &CanvasProps::default(),
                           Flex { grow_w: true, grow_h: true, ..Default::default() });

        // Reactive redraw: identical to `canvas()`, but recording fill+stroke of the resolved kind.
        // Every `.get()` below is a tracked read, so any signal change re-records + diffs + replays.
        let (kind, fill, stroke, inset) = (self.kind, self.fill, self.stroke, self.inset);
        redraw(node, move |d, size| {
            let rect = Rect::from_size(size).inset(inset.get());
            let geom = kind.get().resolve(rect);
            if let Some(fill) = &fill   { d.fill(geom.clone(), fill.get().solid()); }
            if let Some(st)   = &stroke { d.stroke(geom, st.paint.get().solid(), st.width.get()); }
        });

        // Path-precise interaction: test the tap against the resolved path in Rust.
        if let Some(on_tap) = self.on_tap {
            let (kind, inset) = (kind.clone(), inset.clone());
            cx.on(node, move |ev| if let Event::Tap(p) = ev {
                let rect = with_tree(|t| t.node_frame(node)).map(|f| Rect::from_size(f.size)).unwrap();
                if kind.get().resolve(rect.inset(inset.get())).contains(*p) { on_tap(); }
            });
        }
        node
    }
}
```

`redraw(node, closure)` is the [`canvas()`](https://daybrite.dev/api/day_pieces/fn.canvas.html) recording harness (a `Trigger` on `FrameChanged` + a
[`bind`](https://daybrite.dev/api/day_reactive/fn.bind.html) that records → `replay`) factored out so both [`canvas()`](https://daybrite.dev/api/day_pieces/fn.canvas.html) and [`shape()`](https://daybrite.dev/api/day_pieces/fn.shape.html) share it.

That is the entire v1 renderer; the five built-in kinds with solid fill/stroke need no
`day-spec`, `day-core`, or toolkit changes. [`.id`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.id), [`.a11y`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.a11y), [`.frame`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.frame), [`.padding`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.padding) come from `Decorate`
unchanged. Reactivity, diffing, and native replay come from the canvas machinery unchanged.

### 3.5 Reference example: re-express the gauge

```rust
// Today (raw canvas):
canvas(move |d, size| {
    let r = Rect::from_size(size).inset(8.0);
    d.stroke(Shape::Arc { rect: r, start_deg: 135.0, sweep_deg: 270.0 }, TRACK, 6.0);
    d.stroke(Shape::Arc { rect: r, start_deg: 135.0, sweep_deg: 270.0 * frac() }, ACCENT, 6.0);
})

// With shapes (phase 2 arcs + trim):
zstack((
    circle().stroke(TRACK, 6.0),
    circle().trim(0.0, move || value.get() / 100.0).stroke(ACCENT, 6.0).rotation(-90.0),
    text(move || format!("{:.0}", value.get())),
))
.frame(120.0, 120.0)
.a11y(|a| a.role(Role::Meter))
```

The result is a composable, identified, animatable progress ring, the same idiom SwiftUI uses.

### 3.6 Shape groups: many shapes, one canvas leaf

A `ShapePiece` is one native view. That is right for a backdrop or a swatch and wrong for a
composed glyph: a 10-row forecast whose weather icons average 8–12 shapes each would create ~200
native views (each with a trigger, an event route, and a per-view replay hop) where the raw-canvas
version had ~30. The composites close that gap:

```rust
/// Flatten shape descriptions into ONE canvas leaf, drawn in order.
pub fn shape_group(shapes: impl IntoIterator<Item = ShapePiece>) -> impl Piece;
/// Size-aware variant: children derive from the laid-out size, re-run on FrameChanged —
/// for geometry that maps data along the final width/height.
pub fn shape_group_fn(shapes: impl Fn(Size) -> Vec<ShapePiece> + 'static) -> impl Piece;

// A storm glyph — five shapes, one native view:
shape_group([
    rounded_rectangle(4.0).fill(CLOUD).at(0.08, 0.54, 0.84, 0.36),
    ellipse().fill(CLOUD).at(0.06, 0.36, 0.42, 0.42),
    ellipse().fill(CLOUD).at(0.30, 0.20, 0.46, 0.46),
    line((0.34, 0.78), (0.29, 0.98)).stroke(RAIN, 1.5),
    polygon([(0.52, 0.66), (0.40, 0.90), /* … */]).fill(BOLT),
])
.frame(30.0, 30.0)
```

Groups reuse the whole shape pipeline: each child is the same `ShapePiece` description
(fill/stroke/inset/rotate/scale/offset/[`.at`](https://daybrite.dev/api/day_pieces/trait.ShapePieceBuilder.html#tymethod.at), all reactive; a tracked read in any child
re-records the group), and both composites lower through the same `canvas_leaf` as [`canvas()`](https://daybrite.dev/api/day_pieces/fn.canvas.html) and
standalone shapes. Groups have two limits: child [`.on_tap`](https://daybrite.dev/api/day_pieces/trait.ShapePieceBuilder.html#tymethod.on_tap)/[`.on_drag`](https://daybrite.dev/api/day_pieces/trait.ShapePieceBuilder.html#tymethod.on_drag) are not wired inside a
group (put gestures on the group via [`Decorate::on_tap`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.on_tap)), and a group has no per-child identity,
because it is one leaf.

As a rule of density, a backdrop is a shape, a glyph is a group, and hundreds of shapes are a
[`canvas()`](https://daybrite.dev/api/day_pieces/fn.canvas.html).

## 4. Reactivity, identity, accessibility

- **Reactivity**: free (D4). [`circle`](https://daybrite.dev/api/day_pieces/fn.circle.html)`()`[`.fill`](https://daybrite.dev/api/day_pieces/trait.ShapePieceBuilder.html#tymethod.fill)`(move || if on`[`.get`](https://daybrite.dev/api/day_reactive/struct.Signal.html#method.get)`() { RED } else { GRAY })` re-records
  only the fill op when `on` flips; the diff replays one op. This is coarse-grained per shape, which
  is correct: a shape is a handful of ops. (For hundreds of shapes, drop to a single [`canvas()`](https://daybrite.dev/api/day_pieces/fn.canvas.html), one
  view with one op-list. That's the documented escape hatch, mirroring SwiftUI `Shape` vs `Canvas`.)
- **Identity / a11y**: `Decorate` already applies `set_id`/`set_a11y` to the shape's canvas node.
  Nothing new is needed. Shapes participate in dayscript locators and the a11y tree like any leaf.

## 5. Animation

Shapes animate by interpolating parameters and re-recording per frame, on the canvas path rather than the native-widget `AnimSpec` path:

```rust
pub struct Animation { pub curve: Curve, pub duration_ms: u32, pub delay_ms: u32 }
pub enum Curve { Linear, EaseIn, EaseOut, EaseInOut, Spring { response: f64, damping: f64 } }

/// Interpolable shape data. Implemented for f64, Color, Point, Rect, Corner, Paint, PathData.
pub trait Animatable { fn lerp(&self, to: &Self, t: f64) -> Self; }
```

Two entry points, matching SwiftUI:

- **Implicit**: [`circle`](https://daybrite.dev/api/day_pieces/fn.circle.html)`()`[`.fill`](https://daybrite.dev/api/day_pieces/trait.ShapePieceBuilder.html#tymethod.fill)`(color_sig)`[`.animation`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.animation)`(`[`Animation::ease_in_out(300)`](https://daybrite.dev/api/day_spec/struct.AnimSpec.html#method.ease_in_out)`)` makes the shape's
  reactive reads animated. When `color_sig` changes, the shape captures `from → to`, and a
  frame-clock Trigger ticks while the transition is live; each tick recomputes [`lerp(from, to,
  eased(t))`](https://daybrite.dev/api/day_core/tween/fn.lerp.html) and re-records.
- **Explicit**: [`with_animation`](https://daybrite.dev/api/day_core/fn.with_animation.html)`(`[`Animation::spring(...)`](https://daybrite.dev/api/day_spec/struct.AnimSpec.html#method.spring)`, || path`[`.set`](https://daybrite.dev/api/day_reactive/struct.Signal.html#method.set)`(newValue))` animates any signal
  writes in the closure through their dependent animated shapes.

**Realization.** Day owns one per-window frame clock (CVDisplayLink / Choreographer /
[GdkFrameClock](https://docs.gtk.org/gdk4/class.FrameClock.html) / DispatchSourceTimer), started only while ≥1 animation is live and stopped when the
pool drains (no idle wakeups). Each tick advances active `Animatable` transitions and [`notify()`](https://daybrite.dev/api/day_reactive/struct.Trigger.html#method.notify)s a
Trigger that the animated shapes' canvas [`bind`](https://daybrite.dev/api/day_reactive/fn.bind.html)s track, so the existing record→diff→replay path
redraws them. This is additive: shapes ship and work without animation; [`.animation`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.animation) is a no-op
until the clock lands, and explicit signal-timer animation (a [`task`](https://daybrite.dev/api/day_core/fn.task.html) writing a signal each frame)
already works today.

## 6. Interaction & gestures

The shape's canvas leaf is a real native view. For taps, the shape does path-precise hit
testing in Rust (D5) via `Geometry::contains(point)` before firing [`.on_tap`](https://daybrite.dev/api/day_pieces/trait.ShapePieceBuilder.html#tymethod.on_tap). A kind with no
cheap containment test falls back to its bounding box. The planned long-press equivalent
would reuse this hit testing, but neither a shape `.on_long_press` nor a general public
modifier exists yet. ArkUI implements the existing low-level `GestureKind::LongPress` hook
and delivers `Event::LongPress`; that alone does not expose the event on the shape builder.

## 7. Gradients & `ShapeStyle` (linear + radial: DONE; angular: later)

Linear and radial gradients are implemented end-to-end: [`Paint::Linear`](https://developer.android.com/reference/android/graphics/Paint)/[`Paint::Radial`](https://developer.android.com/reference/android/graphics/Paint) on the
`Fill` op, [`Draw::fill(shape, gradient)`](https://daybrite.dev/api/day_pieces/struct.Draw.html#method.fill) on raw canvas, [`.fill_linear(...)`](https://daybrite.dev/api/day_pieces/trait.ShapePieceBuilder.html#tymethod.fill_linear)/[`.fill_radial(...)`](https://daybrite.dev/api/day_pieces/trait.ShapePieceBuilder.html#tymethod.fill_radial)
on shape pieces (reactive), and native replay on every toolkit (radial: [NSGradient](https://developer.apple.com/documentation/appkit/nsgradient) center-draw,
CGContextDrawRadialGradient, cairo radial pattern, [QRadialGradient](https://doc.qt.io/qt-6/qradialgradient.html) in ObjectMode, Android
RadialGradient shader + local matrix, XAML RadialGradientBrush (OS 1903+, now the msix
MinVersion), and OH_Drawing radial shader with local matrix). Radial geometry is unit-space and
stretches elliptically to non-square bounds on every toolkit. `Paint` still grows to `Angular` +
semantic `Token`s later.
This is the one place the
canvas layer must grow: a `Fill(Shape, Paint)`/`Stroke(Shape, Paint, StrokeStyle)` op (Paint, not
just Color) + per-toolkit gradient replay ([CGGradient](https://developer.apple.com/documentation/coregraphics/cggradient), cairo pattern, QGradient/QConicalGradient,
Android shaders, XAML brushes). hop already implemented this on four toolkits (linear and radial
native everywhere; angular native on Qt, hand-rendered as a wedge fan elsewhere), so the recipe is
known. It benefits raw [`canvas()`](https://daybrite.dev/api/day_pieces/fn.canvas.html) too. Gradients are a canvas-layer feature and add no node kind.

## 8. Arbitrary paths & custom shapes (phase 2)

```rust
pub enum PathSeg { Move(Point), Line(Point), Quad(Point,Point), Cubic(Point,Point,Point),
                   ArcTo{ rect: Rect, start_deg: f64, sweep_deg: f64 }, Close }
pub struct PathData(pub Vec<PathSeg>);

pub fn path(build: impl Fn(&mut PathBuilder) + 'static) -> ShapePiece;         // arbitrary
// The `Shape` protocol analogue: a custom kind is a closure Rect -> PathData.
pub fn shape_fn(f: impl Fn(Rect) -> PathData + 'static) -> ShapePiece;
```

Requires `PathData` + a `FillPath`/`StrokePath` canvas op + toolkit replay ([NSBezierPath](https://developer.apple.com/documentation/appkit/nsbezierpath) /
cairo path / [QPainterPath](https://doc.qt.io/qt-6/qpainterpath.html) / android.graphics.Path all support arbitrary paths). This closes SwiftUI
`Path`/custom-`Shape` parity. `.trim(from, to)` (progress rings, draw-on animations) also lives here
(trim a resolved path by arc-length).

## 9. Proposal B (alternative): a native `SHAPE` leaf

Instead of lowering to canvas, add `kinds::SHAPE` + `ShapeProps { kind, fill, stroke }` and render
natively per toolkit (CAShapeLayer / GtkSnapshot / QGraphics / android.graphics / XAML Path).

- **Pros**: native gradients/shadows/materials, native path animation, native precise hit testing,
  potentially cheaper for many shapes (one layer vs one canvas view each).
- **Cons**: six toolkit implementations; more `day-spec` surface; diverges from "atop canvas";
  reactivity needs explicit [`bind_seeded`](https://daybrite.dev/api/day_reactive/fn.bind_seeded.html) per prop (loses D4's freebie); duplicates what canvas does.

**Recommendation: A now, B as a transparent optimization later.** Because the public API
([`shape`](https://daybrite.dev/api/day_pieces/fn.shape.html)/`ShapeKind`/`Paint`) is renderer-agnostic (D3), a future [`ShapePiece::build`](https://daybrite.dev/api/day_pieces/struct.ShapePiece.html#method.build) may lower to a
native `SHAPE` leaf for shapes that need native fidelity (drop shadows, `Material`, smooth native
morphs), chosen by capability or a `.native()` hint, with no change to user code.

## 10. Phasing

- **Phase 1 (small, no toolkit work):** `Reactive<T>`/`IntoReactive`; `ShapeKind`
  {Rectangle, RoundedRectangle, Circle, Ellipse, Capsule}; [`Paint::Solid`](https://developer.android.com/reference/android/graphics/Paint); [`shape()`](https://daybrite.dev/api/day_pieces/fn.shape.html) + sugar;
  [`.fill`](https://daybrite.dev/api/day_pieces/trait.ShapePieceBuilder.html#tymethod.fill)/[`.stroke`](https://daybrite.dev/api/day_pieces/trait.ShapePieceBuilder.html#tymethod.stroke)/[`.inset`](https://daybrite.dev/api/day_pieces/trait.ShapePieceBuilder.html#tymethod.inset); canvas lowering; [`.id`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.id)/[`.a11y`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.a11y)/[`.frame`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.frame) via `Decorate`; mock-tested op-lists;
  a `shapes` showcase playground. Works on all six toolkits immediately.
- **Phase 1.5 (shipped; still no toolkit work):** `Arc`; `Line`/`Polygon` unit-point kinds;
  [`.at`](https://daybrite.dev/api/day_pieces/trait.ShapePieceBuilder.html#tymethod.at) fractional placement; [`shape_group`](https://daybrite.dev/api/day_pieces/fn.shape_group.html)/[`shape_group_fn`](https://daybrite.dev/api/day_pieces/fn.shape_group_fn.html). Day Skies' weather glyphs
  and range bars are the reference consumers.
- **Phase 2 (canvas-layer growth):** gradients (`Paint` + gradient ops + toolkit replay, per hop);
  `PathData` + path ops + `path()`/`shape_fn()`; arc modes (Sector/Chord), `.trim`.
- **Phase 3 (pillars):** the frame clock + `Animatable`/`Animation` → [`.animation`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.animation)/
  [`with_animation`](https://daybrite.dev/api/day_core/fn.with_animation.html); gesture wiring → [`.on_tap`](https://daybrite.dev/api/day_pieces/trait.ShapePieceBuilder.html#tymethod.on_tap) with path-precise hit testing.
- **Phase 4 (optional):** native `SHAPE` leaf (Proposal B) as a transparent lowering for fidelity.

## 11. Open questions

1. **Frame-relative vs. absolute** as the default (D1): proposed frame-relative; confirm.
2. **Unify the `IntoX` sources** into `Reactive<T>`/`IntoReactive<T,M>` now, or keep per-type and
   add just the shape ones?
3. **[`zstack`](https://daybrite.dev/api/day_pieces/fn.zstack.html)**: the gauge example wants a Z overlay piece. Is a [`zstack`](https://daybrite.dev/api/day_pieces/fn.zstack.html)/[`overlay`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.overlay) in scope
   alongside shapes, or do shapes ship first and compose later?
4. **`Draw` op growth** for gradients/paths: extend the `DrawOp` enum (breaks the packed 9-float
   encoding's assumptions) vs. a parallel richer op stream. Prefer extending with a versioned
   encoder.
5. **[`.background(shape)`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.background) / `.clip(shape)`**: SwiftUI uses shapes as backgrounds/clips. Out of
   scope here, but the `ShapeKind` value is exactly what those modifiers would consume later.

## Platform guidance

- [Apple: Core Graphics](https://developer.apple.com/documentation/coregraphics)
- [Qt: painting](https://doc.qt.io/qt-6/paintsystem.html)
