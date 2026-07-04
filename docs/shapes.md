# Shapes — design & implementation

> **Status: implemented (Proposal A — canvas-backed unified `shape` piece).** The `shape` piece
> (`day_pieces`), reactive `ShapeKind`/`fill`/`stroke`/`inset`/`rotate`/`scale`/`offset`, canvas
> CTM transform ops (`Save`/`Restore`/`Concat`), and `.on_tap`/`.on_drag` gestures ship on all
> five backends (AppKit, GTK, Qt, UIKit, Android) and are demonstrated by the showcase "Shapes"
> playground. This document is the SwiftUI `Circle`/`Rectangle`/`Path`/`Shape` analogue for day;
> conventions follow DESIGN.md.

## 1. Goal & constraints

Give day SwiftUI's shape ergonomics — `Circle`, `Rectangle`, `RoundedRectangle`, `Capsule`,
`Ellipse`, arbitrary `Path`, and custom shapes — as **first-class Pieces** that:

- are **bound to signals** (geometry *and* style),
- are **identified** (`.id`) and **accessible** (`.a11y`),
- are **interactive** through the (eventual) gesture seam, with **path-precise** hit testing,
- are **animatable**,
- are built **atop the internal canvas API** (so they work on all six backends day one),
- read idiomatically for day + Rust — a single data-oriented `shape` piece parameterised by kind,
  not a zoo of node types.

### What day already gives us (the substrate)

- `canvas(|d: &mut Draw, size: Size| …)` — a reactive leaf (`kinds::CANVAS`) whose closure re-runs
  on any tracked signal read (and on `FrameChanged`), records a `Vec<DrawOp>`, diffs it
  (`DrawOp: PartialEq`), and replays only on change. Backends turn `DrawOp`s into native drawing
  (CoreGraphics / cairo / QPainter / Canvas.draw* / DirectWrite) — one FFI hop per redraw.
- `Shape { Rect(Rect), RoundedRect(Rect, f64), Ellipse(Rect), Arc{rect,start_deg,sweep_deg},
  Line(Point,Point), Polygon(Vec<Point>) }` — the geometry enum (`day-spec`).
- `Draw::fill(Shape, Color)`, `Draw::stroke(Shape, Color, f64)`, `Draw::text(...)`.
- `Color { r,g,b,a }` (rgba/hex). No gradients yet.
- `Decorate` blanket impl → `.id/.id_keyed/.a11y/.frame/.padding/.any` for every Piece.
- Reserved seams: `AnimSpec { duration_ms }` threaded through `update`/`set_frame`; and
  §8.4's **"day-driven frame-clock ticker for canvas only"** — the exact hook shape animation needs.

### Prior art consulted

- **SwiftUI** — `Shape: path(in rect) -> Path`; shapes are *frame-relative* views that fill their
  proposed size; `.fill(ShapeStyle)`, `.stroke(_, lineWidth:)`, `.trim(from:to:)`, `.rotation()`;
  `Path` for arbitrary geometry; `Canvas` for dense immediate-mode drawing.
- **floem** — shapes are drawn in the render pass; no separate node per shape (dense-draw model).
- **hop/** (this workspace) — proved a native `ShapeSpec` pipeline with `Shape.fill(gradient)` and
  `LinearGradient/RadialGradient/AngularGradient` native on all four toolkits, and painted-path
  transforms (rotate/scale/offset are path-only, matching SwiftCrossUI). Directly informs §7–§8.
- **pane/ DESIGN2.md** (this workspace) — "style-as-data struct args" over the SwiftUI modifier
  tower; informs the `Paint`/`StrokeStyle` value types below.

## 2. Design decisions (with rationale)

**D1 — Shapes are *frame-relative*, not absolute.** A `Circle` inscribes the rect the layout
engine assigns; a `Rectangle` fills it; `RoundedRectangle`'s corner is the only extra parameter.
You size a shape with `.frame(w, h)` (or a bounded parent), exactly like SwiftUI. This is *more*
composable than "circle of radius r": shapes drop into stacks, grow/shrink with layout, and animate
their frame for free. (The user's `radius = 1.0` sketch is absolute; we deliberately choose the
frame-relative model and recover absolute sizing via `.frame(d, d)`.)

**D2 — One piece, parameterised by a data `ShapeKind`.** Not one node kind per shape. Rust's
enum-with-fields *is* the "params bag" — type-safe and cleaner than a mutable params closure.
Convenience free functions (`circle()`, `rounded_rectangle(12.0)`) give SwiftUI ergonomics; the
unified `shape(kind)` gives the data-oriented form. Both construct the same `ShapePiece`.

**D3 — Render atop canvas; the renderer is an implementation detail behind a stable API.** v1
*lowers* a `ShapePiece` to the existing canvas display-list (zero backend work, works everywhere,
free reactivity, free unit tests). The **same API** can later lower hot/native-fidelity shapes to a
native `SHAPE` leaf (Proposal B, §9) — a capability-gated choice, no API change. Lead with canvas.

**D4 — Reactivity is free.** Because the canvas closure re-runs on tracked reads, every shape
parameter and style may be a value, a `Signal`, or a closure with **no per-prop binding code** — a
strict simplification over native leaves. Shape params are stored as a small `Reactive<T>` source.

**D5 — Interaction is path-precise and day-side.** The shape knows its path, so on a `Tap(point)`
from its canvas leaf, day tests the point against the path *in Rust* before firing `.on_tap` — no
backend change, and more correct than bounding-box hit testing.

**D6 — Two animation paths; shapes use the canvas one.** day has (i) backend-executed animation for
native widgets (the `AnimSpec` seam) and (ii) a day-driven **canvas frame-clock** (§8.4). Shapes
animate by interpolating params CPU-side and re-recording per frame — the same way SwiftUI renders
shape animation. The shape API is animation-ready now; the frame-clock engine lands via §8.4.

## 3. Proposal A (recommended): the canvas-backed `shape` piece

### 3.1 Geometry — `ShapeKind` (data)

```rust
/// A shape's geometry, resolved against the rect the layout engine assigns (frame-relative).
#[derive(Clone)]
pub enum ShapeKind {
    Rectangle,
    RoundedRectangle { corner: Corner },
    Circle,                              // inscribed centred circle (min(w,h))
    Ellipse,                             // fills the rect
    Capsule,                             // RoundedRectangle with corner = min(w,h)/2
    // ── phase 2 (each adds one canvas op + geometry, no new node) ──
    // Arc { start: f64, sweep: f64, mode: ArcMode },   // Open | Sector | Chord
    // Polygon { sides: u32, rotation: f64 },           // regular n-gon inscribed
    // Path(PathData),                                  // arbitrary (see §8)
    // Custom(Rc<dyn Fn(Rect) -> PathData>),            // the `Shape` protocol analogue
}

/// A corner radius as absolute points or a fraction (0..1) of min(w,h).
#[derive(Clone, Copy)]
pub enum Corner { Fixed(f64), Fraction(f64) }
impl From<f64> for Corner { fn from(v: f64) -> Self { Corner::Fixed(v) } }

impl ShapeKind {
    /// Lower to a drawable path within `rect` (v1: the existing `Shape` enum; phase 2: `PathData`).
    fn resolve(&self, rect: Rect) -> Geometry { /* Circle → Ellipse(square inset & centred), … */ }
}
```

`resolve` maps each kind to the existing `day_spec::Shape` for v1 kinds — so **no `day-spec` change
is required** for Rectangle/RoundedRectangle/Circle/Ellipse/Capsule. Phase-2 kinds (Arc modes,
Polygon, Path, Custom) introduce a `PathData` primitive + one path-fill/stroke canvas op (§8).

### 3.2 Style — `Paint` and `Stroke` (data, growable)

```rust
/// A fill/stroke source. v1 renders only `Solid` (existing canvas ops); gradients are phase 2
/// (§7) and add native-gradient replay to each backend — a canvas-layer investment, not a new node.
#[derive(Clone)]
pub enum Paint {
    Solid(Color),
    // ── phase 2 ──
    // Linear  { stops: Vec<(f64, Color)>, start: UnitPoint, end: UnitPoint },
    // Radial  { stops: Vec<(f64, Color)>, center: UnitPoint, radius: f64 },
    // Angular { stops: Vec<(f64, Color)>, center: UnitPoint, start_deg: f64 },
    // Token(SemanticColor),                            // §6 theme tokens, late-bound
}
impl From<Color> for Paint { /* … */ }

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
    on_tap: Option<Rc<dyn Fn()>>,    // path-precise (§6)
    anim:   Option<Animation>,       // implicit-animate reads (§5)
    // phase 2: trim: Option<(Reactive<f64>, Reactive<f64>)>, rotation: Reactive<f64>, …
}

/// The unified, data-oriented constructor (the user's `shape(type:, params:)`).
pub fn shape(kind: impl IntoReactive<ShapeKind>) -> ShapePiece { /* … */ }

/// SwiftUI-ergonomic sugar — all construct the same `ShapePiece`.
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

`IntoReactive<T>` is a marker-trait conversion accepting `T`, `Signal<T>`, or `Fn() -> T` — the
generalisation of today's `IntoText`/`IntoFraction` into one reusable source type
`Reactive<T> = Const(T) | Dyn(Rc<dyn Fn() -> T>)`. (Adopting it also lets us collapse the existing
per-type `*Source` enums — an optional cleanup.)

### 3.4 Lowering to canvas (the whole implementation, v1)

```rust
impl Piece for ShapePiece {
    fn build(self, cx: &mut BuildCx) -> RNode {
        // A shape greedily fills its proposed size (SwiftUI semantics): a canvas leaf with grow.
        let node = cx.leaf(kinds::CANVAS, &CanvasProps::default(),
                           Flex { grow_w: true, grow_h: true, ..Default::default() });

        // Reactive redraw — identical to `canvas()`, but recording fill+stroke of the resolved kind.
        // Every `.get()` below is a tracked read, so any signal change re-records + diffs + replays.
        let (kind, fill, stroke, inset) = (self.kind, self.fill, self.stroke, self.inset);
        redraw(node, move |d, size| {
            let rect = Rect::from_size(size).inset(inset.get());
            let geom = kind.get().resolve(rect);
            if let Some(fill) = &fill   { d.fill(geom.clone(), fill.get().solid()); }
            if let Some(st)   = &stroke { d.stroke(geom, st.paint.get().solid(), st.width.get()); }
        });

        // Path-precise interaction: test the tap against the resolved path in Rust (§6).
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

`redraw(node, closure)` is the `canvas()` recording harness (a `Trigger` on `FrameChanged` + a
`bind` that records → `replay`) factored out so both `canvas()` and `shape()` share it.

**That is the entire v1 renderer.** No `day-spec`, `day-core`, or backend changes for the five
built-in kinds with solid fill/stroke. `.id`, `.a11y`, `.frame`, `.padding` come from `Decorate`
unchanged. Reactivity, diffing, and native replay come from the canvas machinery unchanged.

### 3.5 Worked example — re-express the gauge

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

A composable, identified, animatable progress ring — the SwiftUI idiom, in day.

## 4. Reactivity, identity, accessibility

- **Reactivity** — free (D4). `circle().fill(move || if on.get() { RED } else { GRAY })` re-records
  only the fill op when `on` flips; the diff replays one op. Coarse-grained *per shape*, which is
  correct: a shape is a handful of ops. (For hundreds of shapes, drop to a single `canvas()` — one
  view, one op-list — the documented escape hatch, mirroring SwiftUI `Shape` vs `Canvas`.)
- **Identity / a11y** — `Decorate` already applies `set_id`/`set_a11y` to the shape's canvas node.
  Nothing new. Shapes participate in dayscript locators and the a11y tree like any leaf.

## 5. Animation (the headline seam)

Shapes animate by **interpolating parameters and re-recording per frame** — the canvas path of
§8.4, not the native-widget `AnimSpec` path. Design:

```rust
pub struct Animation { pub curve: Curve, pub duration_ms: u32, pub delay_ms: u32 }
pub enum Curve { Linear, EaseIn, EaseOut, EaseInOut, Spring { response: f64, damping: f64 } }

/// Interpolable shape data. Implemented for f64, Color, Point, Rect, Corner, Paint, PathData.
pub trait Animatable { fn lerp(&self, to: &Self, t: f64) -> Self; }
```

Two entry points, matching SwiftUI:

- **Implicit** — `circle().fill(color_sig).animation(Animation::ease_in_out(300))`: the shape's
  reactive reads become *animated*. When `color_sig` changes, the shape captures `from → to`, and a
  **frame-clock Trigger** ticks while the transition is live; each tick recomputes `lerp(from, to,
  eased(t))` and re-records.
- **Explicit** — `with_animation(Animation::spring(...), || path.set(newValue))`: any signal writes
  in the closure animate their dependent animated shapes.

**Realisation (§8.4).** day owns one per-window frame clock (CVDisplayLink / Choreographer /
GdkFrameClock / DispatchSourceTimer), started only while ≥1 animation is live and stopped when the
pool drains (no idle wakeups). Each tick advances active `Animatable` transitions and `notify()`s a
Trigger that the animated shapes' canvas `bind`s track — so the *existing* record→diff→replay path
redraws them. This is additive: shapes ship and work with **no** animation; `.animation` is a no-op
until the clock lands, and *explicit* signal-timer animation (a `task` writing a signal each frame)
already works today.

## 6. Interaction & gestures

The shape's canvas leaf is a real native view; when the gesture seam emits `Event::Tap/LongPress`
for it, the shape does **path-precise** hit testing in Rust (D5) via `Geometry::contains(point)`
before firing `.on_tap`/`.on_long_press`. Bounding-box fallback if a kind has no cheap containment.
This is strictly more correct than SwiftUI's default (which needs `.contentShape` to get precise
hit testing) and needs no backend work beyond the gesture events day already plans.

## 7. Gradients & `ShapeStyle` (phase 2)

`Paint` grows to `Linear/Radial/Angular` gradients + semantic `Token`s. This is the one place the
canvas layer must grow: a `Fill(Shape, Paint)`/`Stroke(Shape, Paint, StrokeStyle)` op (Paint, not
just Color) + per-backend gradient replay (CGGradient, cairo pattern, QGradient/QConicalGradient,
Android shaders, WinUI brushes). **hop already implemented exactly this on four toolkits** — linear
+ radial native everywhere, angular native on Qt and hand-rendered (wedge fan) elsewhere — so the
recipe is known. It benefits raw `canvas()` too. Gradients are a canvas-layer feature, still not a
new node kind.

## 8. Arbitrary paths & custom shapes (phase 2)

```rust
pub enum PathSeg { Move(Point), Line(Point), Quad(Point,Point), Cubic(Point,Point,Point),
                   ArcTo{ rect: Rect, start_deg: f64, sweep_deg: f64 }, Close }
pub struct PathData(pub Vec<PathSeg>);

pub fn path(build: impl Fn(&mut PathBuilder) + 'static) -> ShapePiece;         // arbitrary
// The `Shape` protocol analogue — a custom kind is a closure Rect -> PathData:
pub fn shape_fn(f: impl Fn(Rect) -> PathData + 'static) -> ShapePiece;
```

Requires `PathData` + a `FillPath`/`StrokePath` canvas op + backend replay (NSBezierPath /
cairo path / QPainterPath / android.graphics.Path — all support arbitrary paths). This closes SwiftUI
`Path`/custom-`Shape` parity. `.trim(from, to)` (progress rings, draw-on animations) also lives here
(trim a resolved path by arc-length).

## 9. Proposal B (alternative): a native `SHAPE` leaf

Instead of lowering to canvas, add `kinds::SHAPE` + `ShapeProps { kind, fill, stroke }` and render
natively per backend (CAShapeLayer / GtkSnapshot / QGraphics / android.graphics / WinUI Path).

- **Pros** — native gradients/shadows/materials, native path animation, native precise hit testing,
  potentially cheaper for *many* shapes (one layer vs one canvas view each).
- **Cons** — six backend implementations; more `day-spec` surface; diverges from "atop canvas";
  reactivity needs explicit `bind_seeded` per prop (loses D4's freebie); duplicates what canvas does.

**Recommendation: A now, B as a transparent optimisation later.** Because the public API
(`shape`/`ShapeKind`/`Paint`) is renderer-agnostic (D3), a future `ShapePiece::build` may lower to a
native `SHAPE` leaf for shapes that need native fidelity (drop shadows, `Material`, buttery native
morphs) — chosen by capability or a `.native()` hint — with **no change to user code**. Ship A;
keep B in the pocket.

## 10. Phasing

- **Phase 1 (small, no backend work):** `Reactive<T>`/`IntoReactive`; `ShapeKind`
  {Rectangle, RoundedRectangle, Circle, Ellipse, Capsule}; `Paint::Solid`; `shape()` + sugar;
  `.fill/.stroke/.inset`; canvas lowering; `.id/.a11y/.frame` via `Decorate`; mock-tested op-lists;
  a `shapes` showcase playground. **Fully works on all six backends immediately.**
- **Phase 2 (canvas-layer growth):** gradients (`Paint` + gradient ops + backend replay, per hop);
  `PathData` + path ops + `path()`/`shape_fn()`; `Arc` modes, `Polygon`, `.trim`, `.rotation`.
- **Phase 3 (pillars):** the §8.4 frame clock + `Animatable`/`Animation` → `.animation`/
  `with_animation`; gesture wiring → `.on_tap` with path-precise hit testing.
- **Phase 4 (optional):** native `SHAPE` leaf (Proposal B) as a transparent lowering for fidelity.

## 11. Open questions

1. **Frame-relative vs. absolute** as the default (D1) — proposed frame-relative; confirm.
2. **Unify the `IntoX` sources** into `Reactive<T>`/`IntoReactive<T,M>` now, or keep per-type and
   add just the shape ones?
3. **`zstack`** — the gauge example wants a Z overlay piece. Is a `zstack`/`overlay` in scope
   alongside shapes, or do shapes ship first and compose later?
4. **`Draw` op growth** for gradients/paths — extend the `DrawOp` enum (breaks the packed 9-float
   encoding's assumptions) vs. a parallel richer op stream. Prefer extending with a versioned
   encoder.
5. **`.background(shape)` / `.clip(shape)`** — SwiftUI uses shapes as backgrounds/clips. Out of
   scope here, but the `ShapeKind` value is exactly what those modifiers would consume later.
