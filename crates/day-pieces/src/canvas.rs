// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The `canvas` immediate-mode drawing surface (record a `Draw` display list that backends
//! replay natively, with `frame_clock` for per-frame animation), plus the general `Reactive<T>`
//! (value / `Signal` / closure) abstraction that pieces accept for animatable inputs.

use std::cell::RefCell;
use std::rc::Rc;

use day_core::*;
use day_reactive::{Scope, Signal};
use day_spec::props::*;
use day_spec::{
    Color, CornerRadii, DrawOp, Event, ExportError, FillRule, Paint, PathSeg, Point, Rect, Shape,
    Size, StrokeStyle, kinds,
};

use crate::*;

// ---------------------------------------------------------------------------
// Canvas (§11): record a display list reactively; backends replay natively.
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct Draw {
    ops: Vec<DrawOp>,
}

impl Draw {
    /// An empty recorder, for a test that calls a draw function directly and asserts on what
    /// it records: the same list the canvas replays.
    pub fn new() -> Self {
        Draw::default()
    }
    /// What has been recorded so far, in order.
    pub fn ops(&self) -> &[DrawOp] {
        &self.ops
    }
}

/// Build a [`Shape::Path`]: several contours, straight or curved, with a fill rule.
///
/// ```ignore
/// let ring = PathBuilder::new()
///     .rule(FillRule::EvenOdd)          // the inner circle cuts a hole
///     .circle(center, 40.0)
///     .circle(center, 24.0)
///     .build();
/// d.fill(ring, Color::BLUE);
/// ```
#[derive(Clone, Debug, Default)]
pub struct PathBuilder {
    segs: Vec<PathSeg>,
    rule: FillRule,
}

impl PathBuilder {
    pub fn new() -> Self {
        Self::default()
    }
    /// Which points count as inside where contours overlap (default [`FillRule::NonZero`]).
    pub fn rule(mut self, rule: FillRule) -> Self {
        self.rule = rule;
        self
    }
    /// Start a new contour.
    pub fn move_to(mut self, p: Point) -> Self {
        self.segs.push(PathSeg::Move(p));
        self
    }
    pub fn line_to(mut self, p: Point) -> Self {
        self.segs.push(PathSeg::Line(p));
        self
    }
    /// Quadratic bezier through control point `c` to `p`.
    pub fn quad_to(mut self, c: Point, p: Point) -> Self {
        self.segs.push(PathSeg::Quad(c, p));
        self
    }
    /// Cubic bezier with control points `c1`, `c2` to `p`.
    pub fn cubic_to(mut self, c1: Point, c2: Point, p: Point) -> Self {
        self.segs.push(PathSeg::Cubic(c1, c2, p));
        self
    }
    /// Close the current contour back to its start.
    pub fn close(mut self) -> Self {
        self.segs.push(PathSeg::Close);
        self
    }
    /// A whole circular contour, as four cubics.
    ///
    /// 0.5523 is the standard circle-from-beziers constant (`4/3·tan(π/8)`); the error against a
    /// true circle is under a thousandth of the radius, which is well inside a pixel at any size
    /// a UI draws.
    /// Append a circular arc: `sweep_deg` of a circle of `radius` about `center`, starting at
    /// `start_deg`. Degrees, `0` = the +x axis, positive sweeping clockwise, the same convention
    /// [`Shape::Arc`] and [`PathBuilder::circle`] already use, so the crate has one.
    ///
    /// A *segment*, not a shape: the arc joins whatever came before it, which is what lets a
    /// donut wedge, a rounded gauge or a pie slice with a hole be one closed path. Without it
    /// every such figure is hand-rolled from cubics; `day-piece-charts` carried forty lines of
    /// exactly this to draw a wedge.
    ///
    /// Emitted as cubics rather than as an arc op on the wire. Every rasterizer under Day has its
    /// own rule for joining an arc to the line before it, and its own flattening tolerance; the
    /// same beziers everywhere means the same pixels everywhere.
    ///
    /// A cubic cannot be a circle, so this is an approximation, and the quarter turns it splits
    /// into are where that is cheapest: the radial error of a cubic quarter-circle peaks at
    /// 2.7 × 10⁻⁴ of the radius, a quarter of a pixel on a circle a thousand points across, and
    /// proportionally less on anything smaller. Splitting finer would halve nothing anyone can
    /// see and double the segment count. It is the same tradeoff Core Graphics, cairo and every
    /// SVG renderer make.
    ///
    /// Continues the current subpath when there is one, and starts a new one otherwise.
    pub fn arc_to(mut self, center: Point, radius: f64, start_deg: f64, sweep_deg: f64) -> Self {
        // Reach the arc's start: a line from wherever the path is, or a move if it is nowhere.
        // `Close` ends a subpath, so a path that just closed is "nowhere" too.
        let fresh = self.segs.last().is_none_or(|s| matches!(s, PathSeg::Close));
        let first = day_spec::arc_point(center, radius, start_deg);
        self.segs.push(if fresh {
            PathSeg::Move(first)
        } else {
            PathSeg::Line(first)
        });
        self.segs
            .extend(day_spec::arc_cubics(center, radius, start_deg, sweep_deg));
        self
    }

    pub fn circle(self, center: Point, radius: f64) -> Self {
        const K: f64 = 0.552_284_749_8;
        let (cx, cy, r, k) = (center.x, center.y, radius, radius * K);
        self.move_to(Point::new(cx + r, cy))
            .cubic_to(
                Point::new(cx + r, cy + k),
                Point::new(cx + k, cy + r),
                Point::new(cx, cy + r),
            )
            .cubic_to(
                Point::new(cx - k, cy + r),
                Point::new(cx - r, cy + k),
                Point::new(cx - r, cy),
            )
            .cubic_to(
                Point::new(cx - r, cy - k),
                Point::new(cx - k, cy - r),
                Point::new(cx, cy - r),
            )
            .cubic_to(
                Point::new(cx + k, cy - r),
                Point::new(cx + r, cy - k),
                Point::new(cx + r, cy),
            )
            .close()
    }
    /// A contour through `pts` as a Catmull-Rom spline converted to cubics: the smooth line a
    /// chart wants through its data points, without the caller doing bezier arithmetic.
    ///
    /// The curve passes through every point (unlike a plain bezier fit), and the tangent at each
    /// point follows its neighbors. `tension` 0.0 is a straight polyline and 1.0 is the
    /// standard Catmull-Rom; values above about 1.2 overshoot visibly.
    pub fn smooth_polyline(mut self, pts: &[Point], tension: f64) -> Self {
        if pts.len() < 2 {
            return match pts.first() {
                Some(p) => self.move_to(*p),
                None => self,
            };
        }
        self.segs.push(PathSeg::Move(pts[0]));
        let t = tension / 6.0;
        for i in 0..pts.len() - 1 {
            // The neighbors on each side, clamped at the ends so the first and last segments
            // keep the same construction as the middle ones.
            let p0 = pts[i.saturating_sub(1)];
            let (p1, p2) = (pts[i], pts[i + 1]);
            let p3 = pts[(i + 2).min(pts.len() - 1)];
            self.segs.push(PathSeg::Cubic(
                Point::new(p1.x + (p2.x - p0.x) * t, p1.y + (p2.y - p0.y) * t),
                Point::new(p2.x - (p3.x - p1.x) * t, p2.y - (p3.y - p1.y) * t),
                p2,
            ));
        }
        self
    }
    /// Append a rectangle with its own radius on each corner, as one closed contour: the path
    /// [`Shape::rounded_rect`] draws when its corners differ, for a figure that needs it as one
    /// contour among several (a rounded frame with a hole cut out of it).
    pub fn rounded_rect(mut self, rect: Rect, radii: CornerRadii) -> Self {
        self.segs.extend(day_spec::rounded_rect_segs(rect, radii));
        self
    }

    /// Finish, as a [`Shape`] ready for `fill`, `stroke` or `clip`.
    pub fn build(self) -> Shape {
        Shape::Path(day_spec::Path {
            segs: self.segs,
            rule: self.rule,
        })
    }
}

/// Canvas text styling (named fields per the API style rule, docs/api-style.md). Fill what you
/// set and take the rest from `..Default::default()`: 12 points, black, top-leading, the
/// platform's own face.
#[derive(Clone, Debug, PartialEq)]
pub struct TextStyle {
    /// Absolute canvas points, with no accessibility scale (docs/canvas.md "Text").
    pub size: f64,
    pub color: Color,
    pub anchor: day_spec::TextAnchor,
    /// Family, weight and slant (docs/fonts.md); the default is the platform's UI face.
    pub font: day_spec::CanvasFont,
}

impl Default for TextStyle {
    fn default() -> Self {
        TextStyle {
            size: 12.0,
            color: Color::BLACK,
            anchor: day_spec::TextAnchor::LEADING,
            font: day_spec::CanvasFont::default(),
        }
    }
}

impl Draw {
    /// Fill a shape with a solid color or a [`day_spec::LinearGradient`] (both convert to [`Paint`];
    /// gradient unit points resolve against the shape's bounding box, docs/shapes.md §3.2).
    pub fn fill(&mut self, shape: Shape, paint: impl Into<Paint>) {
        self.ops.push(DrawOp::Fill(shape, paint.into()));
    }
    /// Stroke a shape with a solid color at `width`, the everyday case.
    pub fn stroke(&mut self, shape: Shape, color: Color, width: f64) {
        self.ops.push(DrawOp::Stroke(
            shape,
            Paint::Solid(color),
            StrokeStyle::width(width),
        ));
    }
    /// Stroke with a full [`StrokeStyle`] (dash, cap, join) and any paint, gradients included.
    pub fn stroke_styled(&mut self, shape: Shape, paint: impl Into<Paint>, style: StrokeStyle) {
        self.ops
            .push(DrawOp::Stroke(shape, paint.into(), style.clone()));
    }
    /// Draw `shape` once at every position in `at`: one op for the whole batch.
    ///
    /// The template is authored around the origin and each copy is it translated by one point, so
    /// a 6-point dot is `Shape::Ellipse(Rect::new(-3.0, -3.0, 6.0, 6.0))`. Order is drawing order.
    ///
    /// Reach for this the moment a drawing has more marks than it has kinds of mark. A scatter of
    /// fifty thousand points drawn one `fill` at a time is fifty thousand ops to build, compare
    /// and clone on every frame that re-records; as one stamp it is one op and a flat array of
    /// coordinates, and a backend can put every copy into a single path (docs/canvas.md
    /// "Stamping"). Every copy shares the shape, size and paint, so group by whatever varies.
    pub fn stamp(&mut self, shape: Shape, at: Vec<Point>, paint: impl Into<Paint>) {
        self.ops.push(DrawOp::Stamp(Box::new(day_spec::Stamp {
            shape,
            at,
            paint: paint.into(),
            stroke: None,
        })));
    }

    /// [`Draw::stamp`], stroking each copy instead of filling it.
    pub fn stamp_styled(
        &mut self,
        shape: Shape,
        at: Vec<Point>,
        paint: impl Into<Paint>,
        style: StrokeStyle,
    ) {
        self.ops.push(DrawOp::Stamp(Box::new(day_spec::Stamp {
            shape,
            at,
            paint: paint.into(),
            stroke: Some(style),
        })));
    }

    /// Draw a decoded image into `rect` (docs/images.md).
    ///
    /// Takes a decoded [`Bitmap`](day_core::Bitmap), not bytes: the draw closure re-records on
    /// every tracked read, and a buffer here would be compared (and re-decoded) every frame.
    /// Decode once with `day::decode_image`, hold the handle, and draw it as often as you like.
    ///
    /// ```ignore
    /// let photo = day::decode_image(bytes).await?;          // once
    /// canvas(move |d, size| d.image(&photo, Rect::from_size(size)))
    /// ```
    ///
    /// Needs [`Cap::ImageDecode`](day_spec::Cap::ImageDecode); a backend without it draws
    /// nothing here, the same as for any op it does not know.
    pub fn image(&mut self, image: &day_core::Bitmap, rect: day_spec::Rect) {
        self.image_with_opacity(image, rect, 1.0);
    }

    /// [`Draw::image`] with an opacity in `0.0..=1.0` multiplying the image's own alpha.
    pub fn image_with_opacity(
        &mut self,
        image: &day_core::Bitmap,
        rect: day_spec::Rect,
        opacity: f64,
    ) {
        self.ops.push(DrawOp::Image {
            image: image.id(),
            rect,
            opacity: opacity.clamp(0.0, 1.0),
        });
    }

    /// Confine everything drawn afterwards to `shape`.
    ///
    /// The clip lasts until the enclosing [`Draw::restore`], so the usual shape is
    /// `save` → `clip` → draw → `restore`; [`Draw::clipped`] does exactly that for you.
    pub fn clip(&mut self, shape: Shape) {
        self.ops.push(DrawOp::Clip(shape));
    }
    /// Draw `f` clipped to `shape`, restoring the previous clip afterwards.
    pub fn clipped(&mut self, shape: Shape, f: impl FnOnce(&mut Draw)) {
        self.save();
        self.clip(shape);
        f(self);
        self.restore();
    }
    /// One line of text hung on `at` per the style's anchor (docs/canvas.md "Text"); measure it
    /// first with `day::measure_text` when the drawing needs its extent.
    pub fn text(&mut self, text: &str, at: Point, style: TextStyle) {
        self.ops.push(DrawOp::Text {
            text: text.to_owned(),
            at,
            size: style.size,
            color: style.color,
            anchor: style.anchor,
            font: style.font,
        });
    }
    /// Save the current transform/clip; pair with [`Draw::restore`].
    pub fn save(&mut self) {
        self.ops.push(DrawOp::Save);
    }
    /// Restore the transform/clip saved by the matching [`Draw::save`].
    pub fn restore(&mut self) {
        self.ops.push(DrawOp::Restore);
    }
    /// Multiply an affine onto the current transform (shape rotate/scale/offset, §11).
    pub fn concat(&mut self, m: day_geometry::Affine) {
        self.ops.push(DrawOp::Concat(m));
    }
    /// Draw within `m` applied to the CTM, restoring afterwards.
    pub fn transformed(&mut self, m: day_geometry::Affine, f: impl FnOnce(&mut Draw)) {
        self.save();
        self.concat(m);
        f(self);
        self.restore();
    }
}

/// Create + wire a reactive canvas leaf with a given flex: the draw closure re-records on any
/// tracked read and on `FrameChanged`; replay is equality-gated by `DrawOp: PartialEq` (§4.2).
/// Shared by [`canvas`] (intrinsic) and [`shape`] (grows to fill, §shapes).
pub(crate) fn canvas_leaf(
    cx: &mut BuildCx,
    flex: Flex,
    draw: impl Fn(&mut Draw, Size) + 'static,
) -> RNode {
    use day_reactive::{Trigger, bind};
    let node = cx.leaf(kinds::CANVAS, &CanvasProps::default(), flex);
    let trig = Trigger::new();
    cx.on(node, move |ev| {
        if matches!(ev, Event::FrameChanged(_)) {
            trig.notify();
        }
    });
    let draw = std::rc::Rc::new(draw);
    let d2 = draw.clone();
    bind(
        move || {
            trig.track();
            let size = with_tree(|t| t.node_frame(node))
                .map(|f| f.size)
                .unwrap_or(Size::new(0.0, 0.0));
            let mut d = Draw { ops: Vec::new() };
            (d2)(&mut d, size);
            d.ops
        },
        move |ops: &Vec<DrawOp>| {
            with_tree(|t| t.replay(node, ops.clone()));
        },
    );
    node
}

/// The drawing closure is a binding: signal reads re-record; layout size changes re-record
/// (via FrameChanged); replay is equality-gated by DrawOp's PartialEq (§4.2).
pub fn canvas(draw: impl Fn(&mut Draw, Size) + 'static) -> impl Piece {
    piece_fn(move |cx| canvas_leaf(cx, Flex::default(), draw))
}

// ---------------------------------------------------------------------------
// Export (docs/canvas.md "Exporting"): the same draw function, replayed offscreen.
// ---------------------------------------------------------------------------

/// A recorded drawing at a fixed size: what a [`canvas`] would replay, kept for export.
///
/// Write the drawing once as a plain function and call it from the canvas and from export:
///
/// ```ignore
/// fn draw_chart(d: &mut Draw, size: Size, series: &[f64]) { /* … */ }
///
/// canvas(move |d, size| draw_chart(d, size, &series.get()))           // on screen
/// let drawing = Drawing::record(Size::new(600.0, 400.0), |d, size| {
///     draw_chart(d, size, &series.get_untracked())
/// });
/// day::task(async move {
///     if let Ok(png) = drawing.to_png(2.0).await { /* save it */ }
/// });
/// ```
///
/// Rendering goes through the toolkit's own canvas painter (Core Graphics, cairo, `QPainter`,
/// an android `Canvas`, …), so an export matches the screen. Probe [`canvas_exports`] before
/// offering a command.
#[derive(Clone, Debug, PartialEq)]
pub struct Drawing {
    size: Size,
    ops: Vec<DrawOp>,
}

impl Drawing {
    /// Run `draw` once, untracked, at `size` points. Signal reads inside it subscribe nothing,
    /// so recording from a binding or a menu action is safe.
    pub fn record(size: Size, draw: impl FnOnce(&mut Draw, Size)) -> Drawing {
        let mut d = Draw::new();
        day_reactive::untrack(|| draw(&mut d, size));
        Drawing { size, ops: d.ops }
    }

    /// A drawing of ops recorded elsewhere, at `size` points.
    pub fn from_ops(size: Size, ops: Vec<DrawOp>) -> Drawing {
        Drawing { size, ops }
    }

    /// The size the drawing was recorded at, in points.
    pub fn size(&self) -> Size {
        self.size
    }

    /// The recorded display list.
    pub fn ops(&self) -> &[DrawOp] {
        &self.ops
    }

    /// Render to a bitmap of `size × scale` pixels whose `info().scale` is `scale`, so
    /// `image(&bitmap)` shows it at the drawing's point size. Await it inside `day::task`.
    ///
    /// `Err(ExportError::Unsupported)` where the toolkit has no offscreen painter
    /// ([`Cap::CanvasBitmap`](day_spec::Cap::CanvasBitmap)); `Err(ExportError::TooLarge)` above
    /// 16,384 pixels a side or 64 megapixels.
    pub fn to_bitmap(&self, scale: f64) -> day_core::RenderBitmap {
        day_core::render_bitmap(self.ops.clone(), self.size, scale)
    }

    /// Render to a bitmap and encode it as PNG: [`Drawing::to_bitmap`] then
    /// [`Bitmap::encode`]. `Err(ExportError::Unsupported)` where either half is missing
    /// ([`CanvasExports::png`] answers that up front).
    pub fn to_png(
        &self,
        scale: f64,
    ) -> impl std::future::Future<Output = Result<Vec<u8>, ExportError>> + 'static {
        let render = self.to_bitmap(scale);
        async move {
            let bitmap = render.await?;
            bitmap
                .encode(day_spec::EncodeSpec::default())
                .await
                .map_err(|e| match e {
                    day_spec::ImageError::Unsupported | day_spec::ImageError::Encode => {
                        ExportError::Unsupported
                    }
                    other => ExportError::Failed(other.to_string()),
                })
        }
    }

    /// A one-page PDF the size of the drawing. `Err(ExportError::Unsupported)` where the
    /// platform has no PDF writer ([`Cap::CanvasPdf`](day_spec::Cap::CanvasPdf)).
    pub fn to_pdf(&self) -> day_core::RenderPdf {
        PdfDocument::new().page(self.clone()).render()
    }
}

/// A PDF of one or more drawings, one per page; each page takes its drawing's size in PDF
/// points (1/72 inch, the same unit as canvas points: US Letter is 612 × 792, A4 595 × 842).
///
/// ```ignore
/// let mut doc = PdfDocument::new().title("Quarterly report");
/// for region in &regions {
///     doc = doc.page(Drawing::record(LETTER, |d, size| draw_region(d, size, region)));
/// }
/// let pdf = doc.render().await?;
/// ```
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PdfDocument {
    pages: Vec<Drawing>,
    info: day_spec::PdfInfo,
}

impl PdfDocument {
    /// An empty document; rendering it before adding a page answers `ExportError::Invalid`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append `drawing` as the next page, at the drawing's size.
    pub fn page(mut self, drawing: Drawing) -> Self {
        self.pages.push(drawing);
        self
    }

    /// The document title. Android's writer has no metadata API and ignores it.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.info.title = Some(title.into());
        self
    }

    /// The document author. Android's writer ignores it, as it does the title.
    pub fn author(mut self, author: impl Into<String>) -> Self {
        self.info.author = Some(author.into());
        self
    }

    /// How many pages have been added.
    pub fn len(&self) -> usize {
        self.pages.len()
    }

    /// Whether no page has been added yet.
    pub fn is_empty(&self) -> bool {
        self.pages.is_empty()
    }

    /// Render the document through the platform's PDF writer. Await it inside `day::task`.
    pub fn render(self) -> day_core::RenderPdf {
        let pages = self
            .pages
            .into_iter()
            .map(|d| day_spec::PdfPage {
                size: d.size,
                ops: d.ops,
            })
            .collect();
        day_core::render_pdf(pages, self.info)
    }
}

/// A frame clock (§8.4): an invisible, zero-size piece that calls `tick` every animation frame with
/// the wall-clock delta since the previous frame, for as long as it is mounted. Drop it into the
/// tree (e.g. behind a `canvas` in a `zstack`) to drive a game loop or self-driven animation: the
/// tick mutates state `Signal`s, and a `canvas` reading them re-records that frame.
///
/// Shares `day::frame`'s per-window native display scheduler (docs/frames.md); Day has no
/// timer fallback. Requests stop when the last consumer unmounts. This legacy piece retains
/// a first delta of 1/60 s and a <=100 ms clamp; the general frame API exposes raw elapsed time
/// and pause/resume handles. Inert on the mock backend.
///
/// ```ignore
/// zstack((
///     canvas(move |d, sz| draw(d, sz, state)).grow(),
///     frame_clock(move |dt| step(dt, state)),
/// ))
/// ```
pub fn frame_clock(tick: impl FnMut(std::time::Duration) + 'static) -> impl Piece {
    type TickSlot = Rc<RefCell<Option<Box<dyn FnMut(std::time::Duration)>>>>;
    // Registered on first build (in the mounting scope) and removed when that scope is disposed.
    let slot: TickSlot = Rc::new(RefCell::new(Some(Box::new(tick))));
    piece_fn(move |cx| {
        if let Some(cb) = slot.borrow_mut().take() {
            let id = day_core::add_frame_consumer(cb);
            Scope::current().on_cleanup(move || day_core::remove_frame_consumer(id));
        }
        label("").frame(0.0, 0.0).build(cx)
    })
}

// ---------------------------------------------------------------------------
// Reactive<T>: a value, a Signal, or a closure; the generalization of IntoText/IntoFraction.
// ---------------------------------------------------------------------------

/// A parameter that is either a constant or a reactive source. `get()` is a tracked read, so any
/// `Reactive` used inside a canvas draw closure makes that shape re-record when the source changes.
pub enum Reactive<T: Clone + 'static> {
    Const(T),
    Dyn(Rc<dyn Fn() -> T>),
}
impl<T: Clone + 'static> Clone for Reactive<T> {
    fn clone(&self) -> Self {
        match self {
            Reactive::Const(v) => Reactive::Const(v.clone()),
            Reactive::Dyn(f) => Reactive::Dyn(f.clone()),
        }
    }
}
impl<T: Clone + 'static> Reactive<T> {
    pub fn get(&self) -> T {
        match self {
            Reactive::Const(v) => v.clone(),
            Reactive::Dyn(f) => f(),
        }
    }
    pub fn get_untracked(&self) -> T {
        match self {
            Reactive::Const(v) => v.clone(),
            Reactive::Dyn(f) => day_reactive::untrack(|| f()),
        }
    }
}
/// Disjoint-marker conversion (like [`IntoText`]): accepts `T`, `Signal<T>`, or `Fn() -> T`.
pub trait IntoReactive<T: Clone + 'static, M> {
    fn into_reactive(self) -> Reactive<T>;
}
impl<T: Clone + 'static> IntoReactive<T, StaticMark> for T {
    fn into_reactive(self) -> Reactive<T> {
        Reactive::Const(self)
    }
}
impl<T: Clone + 'static> IntoReactive<T, SignalMark> for Signal<T> {
    fn into_reactive(self) -> Reactive<T> {
        Reactive::Dyn(Rc::new(move || self.get()))
    }
}
impl<T: Clone + 'static, F: Fn() -> T + 'static> IntoReactive<T, FnMark> for F {
    fn into_reactive(self) -> Reactive<T> {
        Reactive::Dyn(Rc::new(self))
    }
}

// ---------------------------------------------------------------------------
// Conformance cases (docs/testing.md)
// ---------------------------------------------------------------------------

/// The canvas's `#[day::test]` cases, next to its constructor.
#[cfg(feature = "conformance")]
pub(crate) mod conformance {
    use day_core::conformance::{Case, Drive};
    use day_reactive::Signal;
    use day_spec::{Cap, Color, Role, Shape, kinds};

    use crate::*;

    /// A canvas with an explicit role, label and a value that follows a signal reports all
    /// three natively, after the value changes too (the showcase gauge's shape).
    #[day_macros::test(day_core)]
    fn canvas_a11y_value() -> Case {
        let level = Signal::new(40.0f64);
        Case::new()
            .proves(kinds::CANVAS)
            .proves_modifier("a11y")
            .page(move || {
                column((
                    canvas(move |d, size| {
                        let w = size.width * level.get() / 100.0;
                        d.fill(
                            Shape::Rect(day_spec::Rect::new(0.0, 0.0, w, size.height)),
                            Color::rgb(0.2, 0.5, 0.9),
                        );
                    })
                    .a11y(move |a| {
                        a.role(Role::Meter)
                            .label("Level")
                            .value(move || format!("{:.0}", level.get()))
                    })
                    .id(day_core::id!(meter))
                    .frame(160.0, 24.0),
                    button("Raise")
                        .action(move || level.set(80.0))
                        .id(day_core::id!(raise)),
                ))
                .spacing(8.0)
            })
            .drive(|d: Drive| async move {
                d.a11y_audit(Some("meter")).await?;
                d.tap("raise").await?;
                d.a11y_audit(Some("meter")).await
            })
    }

    /// Native layout must not turn a canvas into a focus scope with no focusable children.
    #[day_macros::test(day_core)]
    fn canvas_focus_after_layout() -> Case {
        Case::new()
            .proves(kinds::CANVAS)
            .proves_duty("focus")
            .requires(Cap::Snapshot)
            .page(|| {
                column((
                    canvas(|d, size| {
                        d.fill(
                            Shape::Rect(day_spec::Rect::from_size(size)),
                            Color::hex(0xff0000),
                        );
                    })
                    .frame(80.0, 60.0)
                    .id(day_core::id!(painted)),
                    canvas(|_, _| {}).frame(40.0, 40.0).id(day_core::id!(empty)),
                ))
            })
            .drive(|d: Drive| async move {
                // A real rendered frame, rather than a timing delay: requesting focus before
                // ArkUI's first native layout hid its canvas host's loss of focusability.
                d.shot("before-focus").await?;
                d.sample_pixel("painted", 0.5, 0.5, "#ff0000").await?;
                d.focus("painted").await?;
                d.assert_focused("painted", true).await?;
                d.sample_pixel("painted", 0.5, 0.5, "#ff0000").await?;
                d.focus("empty").await?;
                d.assert_focused("empty", true).await
            })
    }

    /// Bitmap origins participate in the whole canvas transform, just like shape coordinates.
    /// Reading the recorded rectangle alone cannot detect a native replay placing it elsewhere.
    #[day_macros::test(day_core)]
    fn canvas_image_transform_and_opacity() -> Case {
        Case::new()
            .proves(kinds::CANVAS)
            .proves_cap(Cap::ImageDecode)
            .requires(Cap::ImageDecode)
            .requires(Cap::Snapshot)
            .page(|| {
                let bitmap = Signal::new(None::<day_core::Bitmap>);
                day_core::decode_image_async(
                    std::sync::Arc::new(crate::image::conformance::RED_BLUE.to_vec()),
                    move |result| bitmap.set(Some(result.expect("decode the fixture PNG"))),
                );
                column((
                    canvas(move |d, size| {
                        d.fill(Shape::Rect(day_spec::Rect::from_size(size)), Color::WHITE);
                        if let Some(image) = bitmap.get() {
                            d.transformed(
                                day_spec::Affine::scale(2.0, 2.0)
                                    .then(day_spec::Affine::rotate(std::f64::consts::FRAC_PI_2))
                                    .then(day_spec::Affine::translate(160.0, 20.0)),
                                |d| {
                                    d.image_with_opacity(
                                        &image,
                                        day_spec::Rect::new(20.0, 10.0, 40.0, 20.0),
                                        0.5,
                                    );
                                },
                            );
                        }
                    })
                    .frame(200.0, 200.0)
                    .id(day_core::id!(canvas_image::painted)),
                    label(move || {
                        if bitmap.get().is_some() {
                            "ready"
                        } else {
                            "loading"
                        }
                    })
                    .id(day_core::id!(canvas_image::state)),
                ))
            })
            .drive(|d: Drive| async move {
                d.assert_text("canvas_image::state", "ready").await?;
                // Rotated and doubled: x=100..140, red y=60..100, blue y=100..140.
                d.sample_pixel("canvas_image::painted", 0.6, 0.4, "#ff8080")
                    .await?;
                d.sample_pixel("canvas_image::painted", 0.6, 0.6, "#8080ff")
                    .await?;
                d.sample_pixel("canvas_image::painted", 0.4, 0.5, "#ffffff")
                    .await?;
                d.sample_pixel("canvas_image::painted", 0.6, 0.2, "#ffffff")
                    .await
            })
    }

    /// The drawing the export cases render: red left half, blue right half, a white disc in
    /// the middle, and a dark bar across the bottom fifth. Solid colors, so a sample has one
    /// right answer on every toolkit.
    fn export_swatch(d: &mut Draw, size: day_spec::Size) {
        let (w, h) = (size.width, size.height);
        d.fill(
            Shape::Rect(day_spec::Rect::new(0.0, 0.0, w / 2.0, h)),
            Color::hex(0xff0000),
        );
        d.fill(
            Shape::Rect(day_spec::Rect::new(w / 2.0, 0.0, w / 2.0, h)),
            Color::hex(0x0000ff),
        );
        d.fill(
            Shape::Ellipse(day_spec::Rect::new(
                w / 2.0 - 15.0,
                h / 2.0 - 15.0,
                30.0,
                30.0,
            )),
            Color::hex(0xffffff),
        );
        d.fill(
            Shape::Rect(day_spec::Rect::new(0.0, h * 0.8, w, h * 0.2)),
            Color::hex(0x202020),
        );
    }

    /// An exported bitmap shown beside the live canvas reads the same colors in the same places:
    /// the export path and the on-screen path paint with one painter, with the same origin and
    /// orientation (docs/canvas.md "Exporting").
    #[day_macros::test(day_core)]
    fn canvas_export_matches_screen() -> Case {
        let size = day_spec::Size::new(120.0, 80.0);
        Case::new()
            .proves(kinds::CANVAS)
            .proves_cap(Cap::CanvasBitmap)
            .requires(Cap::CanvasBitmap)
            .requires(Cap::Snapshot)
            .page(move || {
                let shown = Signal::new(None::<day_core::Bitmap>);
                day_core::render_bitmap_async(
                    Drawing::record(size, export_swatch).ops().to_vec(),
                    size,
                    2.0,
                    move |result| shown.set(result.ok()),
                );
                column((
                    canvas(export_swatch)
                        .frame(size.width, size.height)
                        .id(day_core::id!(export::live)),
                    keyed(
                        move || shown.get(),
                        move |bitmap| {
                            image(bitmap)
                                .stretch()
                                .frame(size.width, size.height)
                                .id(day_core::id!(export::rendered))
                        },
                    ),
                    label(move || match shown.get() {
                        Some(b) => {
                            let info = b.info();
                            format!(
                                "{}x{}@{}",
                                info.pixels.width, info.pixels.height, info.scale
                            )
                        }
                        None => "rendering".to_string(),
                    })
                    .id(day_core::id!(export::state)),
                ))
                .spacing(8.0)
            })
            .shot("default")
            .drive(|d: Drive| async move {
                // 120 × 80 points at 2× is 240 × 160 pixels that draw back at 120 × 80 points.
                d.assert_text("export::state", "240x160@2").await?;
                d.assert_size("export::rendered", 120.0, 80.0).await?;
                for id in ["export::live", "export::rendered"] {
                    d.sample_pixel(id, 0.15, 0.3, "#ff0000").await?;
                    d.sample_pixel(id, 0.85, 0.3, "#0000ff").await?;
                    d.sample_pixel(id, 0.5, 0.5, "#ffffff").await?;
                    // The bar is at the bottom in both: a flipped export would put it on top.
                    d.sample_pixel(id, 0.5, 0.92, "#202020").await?;
                    d.sample_pixel(id, 0.15, 0.08, "#ff0000").await?;
                }
                Ok(())
            })
    }

    /// Curved/path clips must retain their geometry and the transform at clipping time.
    /// The PNG round trip must preserve transparent pixels outside that geometry too.
    #[day_macros::test(day_core)]
    fn canvas_clip_transform_png() -> Case {
        fn swatch(d: &mut Draw, _: day_spec::Size) {
            use day_spec::{Point, Rect};
            // An empty data series must leave the transparent corners untouched.
            d.stamp(
                Shape::Rect(Rect::new(0.0, 0.0, 120.0, 100.0)),
                vec![],
                Color::BLACK,
            );
            d.transformed(day_geometry::Affine::translate(20.0, 10.0), |d| {
                d.clipped(Shape::Ellipse(Rect::new(0.0, 0.0, 80.0, 80.0)), |d| {
                    d.fill(
                        Shape::Rect(Rect::new(-20.0, -10.0, 160.0, 120.0)),
                        Color::hex(0xff0000),
                    );
                    let triangle = PathBuilder::new()
                        .move_to(Point::new(40.0, 0.0))
                        .line_to(Point::new(80.0, 80.0))
                        .line_to(Point::new(0.0, 80.0))
                        .close()
                        .build();
                    d.clipped(triangle, |d| {
                        d.transformed(day_geometry::Affine::translate(-20.0, -10.0), |d| {
                            d.fill(
                                Shape::Rect(Rect::new(0.0, 0.0, 120.0, 100.0)),
                                Color::hex(0x0000ff),
                            );
                        });
                    });
                });
            });
            // Restoring must remove both clips and transforms.
            d.fill(
                Shape::Rect(day_spec::Rect::new(0.0, 90.0, 120.0, 10.0)),
                Color::hex(0x00ff00),
            );
        }
        Case::new()
            .proves(kinds::CANVAS)
            .proves_cap(Cap::ImageEncode)
            .requires(Cap::CanvasBitmap)
            .requires(Cap::ImageEncode)
            .requires(Cap::Snapshot)
            .page(|| {
                let decoded = Signal::new(None::<day_core::Bitmap>);
                let status = Signal::new("rendering".to_string());
                day_core::task(async move {
                    let result = Drawing::record(day_spec::Size::new(120.0, 100.0), swatch)
                        .to_png(2.0)
                        .await;
                    match result {
                        Ok(bytes) => match day_core::decode_image(std::sync::Arc::new(bytes)).await
                        {
                            Ok(bitmap) => {
                                decoded.set(Some(bitmap));
                                status.set("ready".into());
                            }
                            Err(e) => status.set(format!("decode: {e}")),
                        },
                        Err(e) => status.set(format!("export: {e}")),
                    }
                });
                column((
                    canvas(swatch)
                        .frame(120.0, 100.0)
                        .id(day_core::id!(clip_png::live)),
                    keyed(
                        move || decoded.get(),
                        |bitmap| {
                            image(bitmap)
                                .stretch()
                                .frame(120.0, 100.0)
                                .id(day_core::id!(clip_png::decoded))
                        },
                    ),
                    label(move || status.get()).id(day_core::id!(clip_png::state)),
                ))
                .background(Color::WHITE)
            })
            .drive(|d: Drive| async move {
                d.assert_text("clip_png::state", "ready").await?;
                for id in ["clip_png::live", "clip_png::decoded"] {
                    d.sample_pixel(id, 0.2, 0.14, "#ffffff").await?;
                    d.sample_pixel(id, 0.24, 0.5, "#ff0000").await?;
                    d.sample_pixel(id, 0.5, 0.6, "#0000ff").await?;
                    d.sample_pixel(id, 0.1, 0.95, "#00ff00").await?;
                }
                Ok(())
            })
    }

    /// A PDF where the platform has a writer, and `Unsupported` where it has none: the bytes are
    /// a PDF file, and a toolkit without the cap never pretends otherwise.
    #[day_macros::test(day_core)]
    fn canvas_pdf_document() -> Case {
        Case::headless().run(|t: Drive| async move {
            let page = |w, h| Drawing::record(day_spec::Size::new(w, h), export_swatch);
            let result = PdfDocument::new()
                .title("Conformance")
                .author("Day")
                .page(page(612.0, 792.0))
                .page(page(300.0, 200.0))
                .render()
                .await;
            if !canvas_exports().pdf {
                return t.check_eq(result.err(), Some(day_spec::ExportError::Unsupported));
            }
            let pdf = match result {
                Ok(pdf) => pdf,
                Err(e) => return t.check(false, &format!("the PDF did not render: {e}")),
            };
            t.check(
                pdf.starts_with(b"%PDF-"),
                "the bytes do not start with %PDF-",
            )?;
            let tail = String::from_utf8_lossy(&pdf[pdf.len().saturating_sub(64)..]).to_string();
            t.check(tail.contains("%%EOF"), "the PDF has no %%EOF trailer")?;
            // An empty document is not a PDF of two pages: each page carries its own content.
            t.check(
                pdf.len() > 400,
                &format!("a {} byte PDF is too small", pdf.len()),
            )
        })
    }

    /// A rendered bitmap is a `Bitmap` like any decode: it reports the export scale and the
    /// rounded-up pixel size, and encodes to PNG where the toolkit encodes PNG.
    #[day_macros::test(day_core)]
    fn canvas_export_scale_and_png() -> Case {
        Case::headless().run(|t: Drive| async move {
            let drawing = Drawing::record(day_spec::Size::new(50.5, 20.0), export_swatch);
            let result = drawing.to_bitmap(3.0).await;
            if !canvas_exports().bitmap {
                return t.check_eq(result.err(), Some(day_spec::ExportError::Unsupported));
            }
            let bitmap = match result {
                Ok(b) => b,
                Err(e) => return t.check(false, &format!("the bitmap did not render: {e}")),
            };
            let info = bitmap.info();
            t.check_eq(info.pixels, day_spec::Size::new(152.0, 60.0))?;
            t.check_eq(info.scale, 3.0)?;
            if !canvas_exports().png() {
                return Ok(());
            }
            match drawing.to_png(1.0).await {
                Ok(png) => t.check_eq(
                    day_spec::ImageFormat::sniff(&png),
                    Some(day_spec::ImageFormat::Png),
                ),
                Err(e) => t.check(false, &format!("the PNG did not encode: {e}")),
            }
        })
    }
}

#[cfg(test)]
mod arc_tests {
    use super::PathBuilder;
    use day_spec::{PathSeg, Point};

    /// Walk a built path, sampling every cubic densely, and hand back the points.
    fn sample(segs: &[PathSeg], per_seg: usize) -> Vec<Point> {
        let mut out = Vec::new();
        let mut cur = Point::ZERO;
        for seg in segs {
            match seg {
                PathSeg::Move(p) | PathSeg::Line(p) => {
                    cur = *p;
                    out.push(cur);
                }
                PathSeg::Cubic(c1, c2, p) => {
                    for i in 1..=per_seg {
                        let t = i as f64 / per_seg as f64;
                        let u = 1.0 - t;
                        // de Casteljau, written out: the cubic Bernstein basis.
                        out.push(Point::new(
                            u * u * u * cur.x
                                + 3.0 * u * u * t * c1.x
                                + 3.0 * u * t * t * c2.x
                                + t * t * t * p.x,
                            u * u * u * cur.y
                                + 3.0 * u * u * t * c1.y
                                + 3.0 * u * t * t * c2.y
                                + t * t * t * p.y,
                        ));
                    }
                    cur = *p;
                }
                PathSeg::Quad(..) | PathSeg::Close => {}
            }
        }
        out
    }

    /// The property that matters: every point of the emitted curve is on the circle. Pinning the
    /// control points instead would pin the construction rather than the result, and it is the
    /// result a rasterizer draws.
    #[test]
    fn an_arc_stays_on_its_circle() {
        let c = Point::new(100.0, 50.0);
        let r = 40.0;
        for sweep in [15.0, 90.0, 180.0, 359.0, -90.0, -270.0] {
            let path = PathBuilder::new().arc_to(c, r, 30.0, sweep).build();
            let day_spec::Shape::Path(p) = path else {
                panic!("arc_to builds a path")
            };
            let worst = sample(&p.segs, 24)
                .iter()
                .map(|q| ((q.x - c.x).powi(2) + (q.y - c.y).powi(2)).sqrt() - r)
                .fold(0.0f64, |m, e| m.max(e.abs()));
            // The known peak for a cubic quarter-circle, with a little headroom. The assertion
            // uses the real figure rather than a loose one so that a future change that split the
            // sweep differently or got a handle length wrong is noticed here.
            assert!(worst < r * 2.8e-4, "sweep {sweep}: off by {worst}");
            // ...and it is that good rather than accidentally better: a construction that
            // silently started flattening to line segments would pass a one-sided bound.
            if sweep.abs() >= 90.0 {
                assert!(
                    worst > r * 1e-6,
                    "sweep {sweep}: suspiciously exact ({worst})"
                );
            }
        }
    }

    /// Degrees, 0 = +x, positive clockwise, the same convention `Shape::Arc` and `circle` use.
    /// A wrong sign here is the kind of thing that only shows up as a donut drawn inside out.
    #[test]
    fn arc_endpoints_follow_the_clockwise_degree_convention() {
        let c = Point::new(0.0, 0.0);
        let near = |a: Point, b: Point| (a.x - b.x).abs() < 1e-9 && (a.y - b.y).abs() < 1e-9;
        let ends = |start: f64, sweep: f64| {
            let day_spec::Shape::Path(p) = PathBuilder::new().arc_to(c, 10.0, start, sweep).build()
            else {
                panic!()
            };
            let pts = sample(&p.segs, 1);
            (*pts.first().unwrap(), *pts.last().unwrap())
        };
        // 0° is +x; a quarter turn clockwise lands on +y, which is down in canvas space.
        let (a, b) = ends(0.0, 90.0);
        assert!(near(a, Point::new(10.0, 0.0)), "{a:?}");
        assert!(near(b, Point::new(0.0, 10.0)), "{b:?}");
        // …and the same quarter counter-clockwise lands on -y.
        let (_, b) = ends(0.0, -90.0);
        assert!(near(b, Point::new(0.0, -10.0)), "{b:?}");
    }

    /// A rounded rectangle is a contour of its own: a frame with a rounded hole is two of them,
    /// the inner one cut out by the even-odd rule.
    #[test]
    fn a_rounded_rect_contour_opens_and_closes_its_own_subpath() {
        use day_spec::{CornerRadii, FillRule, Rect};
        let day_spec::Shape::Path(p) = PathBuilder::new()
            .rule(FillRule::EvenOdd)
            .rounded_rect(Rect::new(0.0, 0.0, 40.0, 40.0), CornerRadii::uniform(8.0))
            .rounded_rect(Rect::new(10.0, 10.0, 20.0, 20.0), CornerRadii::top(4.0))
            .build()
        else {
            panic!("a path")
        };
        let moves = p
            .segs
            .iter()
            .filter(|s| matches!(s, PathSeg::Move(_)))
            .count();
        let closes = p
            .segs
            .iter()
            .filter(|s| matches!(s, PathSeg::Close))
            .count();
        assert_eq!((moves, closes), (2, 2));
        assert_eq!(p.rule, FillRule::EvenOdd);
    }

    /// An arc joins what came before, which is why it is a segment and not a shape.
    #[test]
    fn an_arc_joins_the_current_subpath_but_starts_a_fresh_one_after_close() {
        let c = Point::new(0.0, 0.0);
        let segs = |b: PathBuilder| {
            let day_spec::Shape::Path(p) = b.build() else {
                panic!()
            };
            p.segs
        };
        // Nothing before it: the arc opens the subpath.
        let s = segs(PathBuilder::new().arc_to(c, 5.0, 0.0, 90.0));
        assert!(matches!(s[0], PathSeg::Move(_)), "{:?}", s[0]);
        // Something before it: the arc is reached by a line, so the figure is one contour.
        let s = segs(
            PathBuilder::new()
                .move_to(Point::new(9.0, 9.0))
                .arc_to(c, 5.0, 0.0, 90.0),
        );
        assert!(matches!(s[1], PathSeg::Line(_)), "{:?}", s[1]);
        // After a close there is no current point, so it opens a new subpath rather than drawing
        // a stray line back from wherever the last contour ended.
        let s = segs(
            PathBuilder::new()
                .move_to(Point::new(9.0, 9.0))
                .line_to(Point::new(1.0, 1.0))
                .close()
                .arc_to(c, 5.0, 0.0, 90.0),
        );
        assert!(matches!(s[3], PathSeg::Move(_)), "{:?}", s[3]);
        // A degenerate arc still reaches its start point and emits no curve.
        let s = segs(PathBuilder::new().arc_to(c, 5.0, 0.0, 0.0));
        assert_eq!(s.len(), 1);
        let s = segs(PathBuilder::new().arc_to(c, 0.0, 0.0, 90.0));
        assert_eq!(s.len(), 1);
    }
}
