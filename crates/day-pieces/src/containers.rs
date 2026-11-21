// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Layout containers that arrange child pieces: `column`, `row`, `grid`/`grid_row`, `scroll`,
//! and `zstack`, together with the `HAlign`/`VAlign` alignment enums.

use std::rc::Rc;

use day_core::*;
use day_reactive::{AnyBinding, Binding, watch};
use day_spec::props::*;
use day_spec::{Event, Insets, Point, ScrollPhase, kinds};

use crate::Decorated;

// ---------------------------------------------------------------------------
// Containers
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HAlign {
    Leading,
    #[default]
    Center,
    Trailing,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VAlign {
    Top,
    #[default]
    Center,
    Bottom,
    /// Sit the children's text on one line rather than centering their boxes
    /// (docs/baseline.md), which is what a label beside a bordered field or a larger-type value
    /// wants, since those put their text at different heights inside their boxes. A child with no
    /// text (an image, a slider) has no baseline and stays centered. Rows only; on a `column`
    /// it reads as `Center`.
    FirstBaseline,
}

#[must_use]
pub struct Column<C: PieceSeq> {
    children: C,
    spacing: f64,
    align: CrossAlign,
}

pub fn column<C: PieceSeq>(children: C) -> Column<C> {
    Column {
        children,
        spacing: 0.0,
        align: CrossAlign::Center,
    }
}

#[day_macros::builder(ColumnBuilder)]
impl<C: PieceSeq> Column<C> {
    /// The gap between children: a [`Space`](crate::Space) step or points.
    pub fn spacing(mut self, s: impl Into<f64>) -> Self {
        self.spacing = s.into();
        self
    }
    pub fn align(mut self, a: HAlign) -> Self {
        self.align = match a {
            HAlign::Leading => CrossAlign::Leading,
            HAlign::Center => CrossAlign::Center,
            HAlign::Trailing => CrossAlign::Trailing,
        };
        self
    }
}

impl<C: PieceSeq> Piece for Column<C> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let node = cx.native(
            kinds::CONTAINER,
            &ContainerProps::default(),
            Rc::new(StackLayout {
                axis: Axis::Vertical,
                spacing: self.spacing,
                align: self.align,
            }),
            Flex::default(),
            Boundary::No,
        );
        cx.under(node, |cx| self.children.build_each(cx));
        node
    }
}

/// How a [`row`] treats content wider than the window it is in
/// (docs/size-classes.md "Row fit policies").
///
/// A row's children negotiate one line; the fit policy is what happens when that line is
/// wider than the space the row is given. Every policy keeps the same children and the same
/// call shape, `row((…)).fit(…)`, so a page can move between them as its needs change. A row
/// that should become a column on a narrow window is [`adaptive`](crate::adaptive), a piece of
/// its own.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum RowFit {
    /// One line at natural sizes; what does not fit lands offscreen. The default, and the
    /// right contract for a label beside a value. Debug builds log the overflow once per
    /// container, naming the dayscript ids in reach.
    #[default]
    Clip,
    /// Break onto additional lines where the next child would overflow, like wrapped text:
    /// the shape a chip row or button strip wants. `run_spacing` is the vertical gap between
    /// lines. A line breaks on the children's natural widths; a child that grows
    /// (`.grow(Dimension::Width)`) then takes a share of what its line has left, by weight.
    /// A `spacer()` contributes nothing.
    Wrap { run_spacing: f64 },
    /// Wrap into aligned columns rather than ragged lines: every cell takes the widest
    /// child's width, and each line holds as many as the window fits. The tidier arm of
    /// [`Wrap`](RowFit::Wrap): same wrapping, but the lines stack into a grid, which is what
    /// a set of peer choices (a keypad, a palette, evenly-weighted chips) wants. The column
    /// count follows the available width. When any child grows (`.grow(Dimension::Width)`), the widest child
    /// becomes the narrowest a column gets and the columns stretch to share the whole width,
    /// the adaptive grid a gallery of tiles wants. No cell is wider than the line: on a window
    /// narrower than the widest child, the cells take its width and their text wraps. An
    /// authored, fixed column count with
    /// per-cell spans is [`grid`]'s job instead (docs/grid.md).
    WrapColumns { run_spacing: f64 },
    /// Keep the single line and make it a horizontal scroll strip instead of clipping. The
    /// strip stays one row tall and fills the width it is given.
    Scroll,
}

#[must_use]
pub struct Row<C: PieceSeq> {
    children: C,
    spacing: f64,
    align: CrossAlign,
    fit: RowFit,
}

pub fn row<C: PieceSeq>(children: C) -> Row<C> {
    Row {
        children,
        spacing: 0.0,
        align: CrossAlign::Center,
        fit: RowFit::Clip,
    }
}

#[day_macros::builder(RowBuilder)]
impl<C: PieceSeq> Row<C> {
    /// The gap between children: a [`Space`](crate::Space) step or points.
    pub fn spacing(mut self, s: impl Into<f64>) -> Self {
        self.spacing = s.into();
        self
    }
    pub fn align(mut self, a: VAlign) -> Self {
        self.align = match a {
            VAlign::Top => CrossAlign::Leading,
            VAlign::Center => CrossAlign::Center,
            VAlign::Bottom => CrossAlign::Trailing,
            VAlign::FirstBaseline => CrossAlign::FirstBaseline,
        };
        self
    }
    /// What happens when the children outgrow the row's width (docs/size-classes.md).
    pub fn fit(mut self, fit: RowFit) -> Self {
        self.fit = fit;
        self
    }
}

impl<C: PieceSeq> Row<C> {
    /// The plain one-line container every policy bottoms out in.
    fn build_line(self, cx: &mut BuildCx, axis: Axis, align: CrossAlign) -> RNode {
        let node = cx.native(
            kinds::CONTAINER,
            &ContainerProps::default(),
            Rc::new(StackLayout {
                axis,
                spacing: self.spacing,
                align,
            }),
            Flex::default(),
            Boundary::No,
        );
        cx.under(node, |cx| self.children.build_each(cx));
        node
    }
}

impl<C: PieceSeq> Piece for Row<C> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        match self.fit {
            RowFit::Clip => {
                let align = self.align;
                self.build_line(cx, Axis::Horizontal, align)
            }
            RowFit::Wrap { run_spacing } | RowFit::WrapColumns { run_spacing } => {
                let node = cx.native(
                    kinds::CONTAINER,
                    &ContainerProps::default(),
                    Rc::new(FlowLayout {
                        spacing: self.spacing,
                        run_spacing,
                        align: self.align,
                        uniform: matches!(self.fit, RowFit::WrapColumns { .. }),
                    }),
                    Flex::default(),
                    Boundary::No,
                );
                cx.under(node, |cx| self.children.build_each(cx));
                node
            }
            RowFit::Scroll => {
                // The row keeps its natural one-line measure inside a horizontal scroll
                // viewport. `grow_h` stays off: the strip is as tall as the row, not as
                // tall as whatever pane it happens to sit in.
                let strip = cx.native(
                    kinds::SCROLL,
                    &day_spec::props::ScrollProps {
                        horizontal: true,
                        ..Default::default()
                    },
                    Rc::new(ScrollLayout {
                        axis: Axis::Horizontal,
                        padding: Insets::ZERO,
                    }),
                    Flex {
                        grow_w: true,
                        ..Default::default()
                    },
                    Boundary::Yes, // scroll viewports are layout boundaries (§7.4)
                );
                cx.under(strip, |cx| {
                    let align = self.align;
                    let _ = self.build_line(cx, Axis::Horizontal, align);
                });
                strip
            }
        }
    }
}

#[must_use]
pub struct Grid<C: PieceSeq> {
    children: C,
    row_spacing: f64,
    column_spacing: f64,
    align: Alignment,
}

/// A SwiftUI-style eager grid (docs/grid.md): columns are inferred from [`grid_row`] children.
/// A column is as wide as its widest cell, a `grow_w` cell makes its column share the leftover
/// width evenly, and a non-row child becomes a full-width cell spanning every column. `spacer()`
/// inside a row is an inert empty cell that still occupies its column (a grid has explicit
/// gutters, so stack-style push-apart spacers don't apply). Cells opt into spans and per-cell
/// alignment with [`Decorated::grid_span`](crate::decorators::Decorated::grid_span) / [`Decorate::grid_align`](crate::decorators::Decorate::grid_align).
pub fn grid<C: PieceSeq>(children: C) -> Grid<C> {
    Grid {
        children,
        row_spacing: 0.0,
        column_spacing: 0.0,
        align: Alignment::Center,
    }
}

#[day_macros::builder(GridBuilder)]
impl<C: PieceSeq> Grid<C> {
    /// Set both the row and column gutters: a [`Space`](crate::Space) step or points.
    pub fn spacing(mut self, s: impl Into<f64>) -> Self {
        let s = s.into();
        self.row_spacing = s;
        self.column_spacing = s;
        self
    }
    pub fn row_spacing(mut self, s: impl Into<f64>) -> Self {
        self.row_spacing = s.into();
        self
    }
    pub fn column_spacing(mut self, s: impl Into<f64>) -> Self {
        self.column_spacing = s.into();
        self
    }
    /// Default alignment of every cell within its cell rect (cell/row overrides win).
    pub fn align(mut self, a: Alignment) -> Self {
        self.align = a;
        self
    }
}

impl<C: PieceSeq> Piece for Grid<C> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let node = cx.native(
            kinds::CONTAINER,
            &ContainerProps::default(),
            Rc::new(GridLayout {
                row_spacing: self.row_spacing,
                column_spacing: self.column_spacing,
                align: self.align,
            }),
            Flex::default(),
            Boundary::No,
        );
        cx.under(node, |cx| self.children.build_each(cx));
        node
    }
}

#[must_use]
pub struct GridRow<C: PieceSeq> {
    children: C,
    valign: Option<CrossAlign>,
}

/// One row of a [`grid`]: each child is a cell, assigned to columns left to right. Outside a
/// grid a row degrades gracefully to a plain [`row`]. Rows are transparent carriers (the grid
/// places their cells directly), so decorating a `grid_row` itself is unsupported (decorate the
/// cells, or the grid).
pub fn grid_row<C: PieceSeq>(children: C) -> GridRow<C> {
    GridRow {
        children,
        valign: None,
    }
}

#[day_macros::builder(GridRowBuilder)]
impl<C: PieceSeq> GridRow<C> {
    /// Vertical alignment override for this row's cells (the grid's alignment applies otherwise).
    pub fn align(mut self, a: VAlign) -> Self {
        self.valign = Some(match a {
            VAlign::Top => CrossAlign::Leading,
            VAlign::Center => CrossAlign::Center,
            VAlign::Bottom => CrossAlign::Trailing,
            VAlign::FirstBaseline => CrossAlign::FirstBaseline,
        });
        self
    }
}

impl<C: PieceSeq> Piece for GridRow<C> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        // A layout-only node whose StackLayout only runs when the row is not inside a grid
        // (the graceful-degrade path); a grid introspects the cells and places them itself.
        let node = cx.layout_only(
            Rc::new(StackLayout {
                axis: Axis::Horizontal,
                spacing: 0.0,
                align: self.valign.unwrap_or_default(),
            }),
            Flex {
                grid: GridFacts {
                    is_row: true,
                    row_valign: self.valign,
                    ..Default::default()
                },
                ..Default::default()
            },
            Boundary::No,
        );
        cx.under(node, |cx| self.children.build_each(cx));
        node
    }
}

/// A native scroll view around one child (docs/scroll.md). Built by [`scroll`].
#[must_use]
pub struct Scroll<P: Piece> {
    child: P,
    axis: Axis,
    target: Option<AnyBinding<Option<day_core::ScrollTarget>>>,
    indicators: ScrollIndicators,
    bounce: Option<bool>,
    snap: ScrollSnap,
    keyboard_dismiss: KeyboardDismiss,
    keyboard_taps: KeyboardTaps,
    padding: Insets,
    offset: Option<AnyBinding<Point>>,
    phase: Option<AnyBinding<ScrollPhase>>,
    zoom: Option<ZoomRange>,
    zoom_scale: Option<AnyBinding<f64>>,
}

/// Scroll `child` in the platform's own scroll view: vertical unless
/// [`horizontal`](ScrollBuilder::horizontal) says otherwise (docs/scroll.md).
pub fn scroll<P: Piece>(child: P) -> Scroll<P> {
    Scroll {
        child,
        axis: Axis::Vertical,
        target: None,
        indicators: ScrollIndicators::Automatic,
        bounce: None,
        snap: ScrollSnap::None,
        keyboard_dismiss: KeyboardDismiss::Never,
        keyboard_taps: KeyboardTaps::Deliver,
        padding: Insets::ZERO,
        offset: None,
        phase: None,
        zoom: None,
        zoom_scale: None,
    }
}

/// How long a scroll view's offset must hold still before day-core calls the scrolling over,
/// on a toolkit that does not report scroll phases itself (`Cap::ScrollPhases`).
const SCROLL_IDLE_MS: u32 = 150;

#[day_macros::builder(ScrollBuilder)]
impl<P: Piece> Scroll<P> {
    /// Scroll horizontally instead of vertically (a filmstrip of cards, a chip row). The content
    /// is measured unconstrained on the horizontal axis and the native view scrolls sideways.
    pub fn horizontal(mut self) -> Self {
        self.axis = Axis::Horizontal;
        self
    }
    /// Set the scroll axis explicitly.
    pub fn axis(mut self, axis: Axis) -> Self {
        self.axis = axis;
        self
    }

    /// Programmatic scrolling (docs/scroll.md): each `Some(target)` written to `sig` scrolls
    /// there, then the signal resets to `None`. Write-and-forget, so the same target can be
    /// sent twice in a row. The scroll is animated with the platform's own scroll animation,
    /// and jumps instead while the system's reduce-motion setting is on.
    ///
    /// ```ignore
    /// let jump = Signal::new(None);
    /// scroll(rows).scroll_target(jump);
    /// button("Bottom").action(move || jump.set(Some(ScrollTarget::Bottom)));
    /// ```
    pub fn scroll_target(mut self, target: impl Binding<Option<day_core::ScrollTarget>>) -> Self {
        self.target = Some(AnyBinding::new(target));
        self
    }

    /// Hide the scroll indicators (scroll bars). The content still scrolls.
    pub fn hide_indicators(mut self) -> Self {
        self.indicators = ScrollIndicators::Hidden;
        self
    }

    /// Stop at the content's edges: no rubber band on Apple platforms, no edge glow or
    /// stretch on Android. A preference: a toolkit without an overscroll effect ignores it.
    pub fn no_bounce(mut self) -> Self {
        self.bounce = Some(false);
        self
    }

    /// Show the platform's overscroll effect even when the content fits the viewport, so a
    /// short page still answers a pull. A preference, as [`Self::no_bounce`] is.
    pub fn always_bounce(mut self) -> Self {
        self.bounce = Some(true);
        self
    }

    /// Come to rest on whole pages: each swipe moves one viewport along the scroll axis
    /// (probe `Cap::ScrollSnap`; where it is unsupported the view scrolls freely).
    pub fn paging(mut self) -> Self {
        self.snap = ScrollSnap::Page;
        self
    }

    /// Come to rest on multiples of `interval` points along the scroll axis: a strip of
    /// equal cards that always stops with one at the leading edge (probe `Cap::ScrollSnap`).
    pub fn snap_interval(mut self, interval: f64) -> Self {
        self.snap = ScrollSnap::Interval(interval);
        self
    }

    /// When scrolling dismisses the software keyboard (docs/scroll.md "Keyboard"; probe
    /// `Cap::ScrollKeyboardDismiss`). Desktop toolkits have no software keyboard and ignore it.
    pub fn keyboard_dismiss(mut self, mode: KeyboardDismiss) -> Self {
        self.keyboard_dismiss = mode;
        self
    }

    /// What a tap inside the scroll view does while the software keyboard is up: reach the
    /// control under it (the default), or dismiss the keyboard and go no further
    /// (docs/scroll.md "Keyboard"). Honored where `Cap::ScrollKeyboardDismiss` is `Native`.
    pub fn keyboard_taps(mut self, taps: KeyboardTaps) -> Self {
        self.keyboard_taps = taps;
        self
    }

    /// Space around the content, inside the scrolling area: it scrolls with the content, so
    /// the last row can clear a floating bar while the scroll view itself stays edge to edge.
    /// Takes a number for all four sides or an `Insets`.
    pub fn content_padding(mut self, insets: impl Into<day_spec::Insets>) -> Self {
        self.padding = insets.into();
        self
    }

    /// Follow the content offset (docs/scroll.md "Offset and phase"): `offset` is written as
    /// the view scrolls, in points from the content's top-leading corner, at most once per
    /// display frame, and once more with the offset the view comes to rest at. Whatever moves
    /// the content counts: a finger, a wheel, momentum, [`Self::scroll_target`]. Drive a
    /// collapsing header or a parallax layer from it. Probe `Cap::ScrollOffset`; where it is
    /// unsupported the binding keeps its value.
    ///
    /// ```ignore
    /// let offset = Signal::new(Point::ZERO);
    /// zstack((
    ///     scroll(page).scroll_offset(offset),
    ///     header.opacity(move || 1.0 - (offset.get().y / 120.0).clamp(0.0, 1.0)),
    /// ))
    /// ```
    pub fn scroll_offset(mut self, offset: impl Binding<Point>) -> Self {
        self.offset = Some(AnyBinding::new(offset));
        self
    }

    /// Follow user-driven scrolling (docs/scroll.md "Offset and phase"): `phase` is
    /// `Dragging` from the moment the user starts moving the content, `Coasting` after the
    /// finger lifts while momentum carries it, and `Idle` once it rests. A programmatic scroll
    /// leaves it alone. `watch` it for the begin and end moments. Where `Cap::ScrollPhases`
    /// is not `Native`, Day derives `Dragging` and `Idle` from the offset reports (there is
    /// no `Coasting`), which needs `Cap::ScrollOffset`.
    pub fn scroll_phase(mut self, phase: impl Binding<ScrollPhase>) -> Self {
        self.phase = Some(AnyBinding::new(phase));
        self
    }

    /// Let the user zoom the content by pinch, between the scales in `range` (`1.0` is the
    /// content's natural size): `scroll(photo).zoom(1.0..=4.0)`. Probe `Cap::ScrollZoom`;
    /// where it is unsupported the content stays at its natural size (docs/scroll.md "Zoom").
    pub fn zoom(mut self, range: std::ops::RangeInclusive<f64>) -> Self {
        let (min, max) = (*range.start(), *range.end());
        self.zoom = Some(ZoomRange {
            min: min.min(max),
            max: max.max(min),
        });
        self
    }

    /// The zoom scale, both ways (docs/scroll.md "Zoom"): `scale` follows the user's pinch,
    /// and a value the app writes zooms there, animated, clamped to the [`Self::zoom`] range.
    /// A "fit" button is `scale.set(1.0)`.
    pub fn zoom_scale(mut self, scale: impl Binding<f64>) -> Self {
        self.zoom_scale = Some(AnyBinding::new(scale));
        self
    }
}

impl<P: Piece> Piece for Scroll<P> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        use day_spec::{Cap, Support};
        let native_phases = day_core::capability(Cap::ScrollPhases) == Support::Native;
        // Day resigns focus itself when a drag begins, where the toolkit asks it to.
        let dismiss_on_drag = self.keyboard_dismiss != KeyboardDismiss::Never
            && day_core::capability(Cap::ScrollKeyboardDismiss) == Support::Emulated;
        let wants_phase = self.phase.is_some() || dismiss_on_drag;
        let node = cx.native(
            kinds::SCROLL,
            &day_spec::props::ScrollProps {
                horizontal: matches!(self.axis, Axis::Horizontal),
                indicators: self.indicators,
                bounce: self.bounce,
                snap: self.snap,
                keyboard_dismiss: self.keyboard_dismiss,
                keyboard_taps: self.keyboard_taps,
                zoom: self.zoom,
                // Without native phases the offset reports are what the phase is derived from.
                track_offset: self.offset.is_some() || (wants_phase && !native_phases),
                track_phase: wants_phase,
            },
            Rc::new(ScrollLayout {
                axis: self.axis,
                padding: self.padding,
            }),
            Flex {
                grow_w: true,
                grow_h: true,
                ..Default::default()
            },
            Boundary::Yes, // scroll viewports are layout boundaries (§7.6)
        );
        cx.under(node, |cx| {
            let _ = self.child.build(cx);
        });
        if let Some(sig) = self.target {
            watch(
                move || sig.get(),
                move |now, _| {
                    if let Some(t) = now.clone() {
                        // Deferred one main-loop turn: this watch runs inside the reactive
                        // flush, before the turn-end layout that resizes the scroll content;
                        // an edge target (Bottom/Trailing) computed now would land on the
                        // stale content size.
                        day_reactive::on_main(move || {
                            day_core::with_tree(|tr| {
                                tr.scroll_to_target(node, &t, true);
                            });
                        });
                        sig.set(None); // consumed, ready for the next command
                    }
                },
            );
        }
        if self.offset.is_some() || wants_phase {
            let (offset, phase) = (self.offset, self.phase);
            // The phase as last told to the app, and the pending "scrolling stopped" check,
            // which the next movement replaces.
            let current = Rc::new(std::cell::Cell::new(ScrollPhase::Idle));
            let quiet: std::cell::Cell<Option<day_core::TaskHandle>> = std::cell::Cell::new(None);
            let enter: Rc<dyn Fn(ScrollPhase)> = {
                let current = current.clone();
                Rc::new(move |next: ScrollPhase| {
                    if current.replace(next) == next {
                        return;
                    }
                    if next == ScrollPhase::Dragging
                        && dismiss_on_drag
                        && let Some(focused) = day_core::focused_node()
                    {
                        day_core::with_tree(|t| t.focus_node(focused, false));
                    }
                    if let Some(phase) = &phase {
                        phase.set_if_changed(next);
                    }
                })
            };
            cx.on(node, move |event| match event {
                Event::ScrollChanged(at) => {
                    if let Some(offset) = &offset {
                        offset.set_if_changed(*at);
                    }
                    if wants_phase && !native_phases {
                        // Movement is all this toolkit reports: scrolling is under way now,
                        // and over once the offset has held still for a moment.
                        enter(ScrollPhase::Dragging);
                        if let Some(pending) = quiet.take() {
                            pending.abort();
                        }
                        let enter = enter.clone();
                        quiet.set(Some(day_core::task(async move {
                            day_core::sleep(SCROLL_IDLE_MS).await;
                            enter(ScrollPhase::Idle);
                        })));
                    }
                }
                Event::ScrollPhase(next) if native_phases => enter(*next),
                _ => {}
            });
        }
        if let Some(scale) = self.zoom_scale {
            // What the view last reported, so its own report is not sent back to it.
            let native = Rc::new(std::cell::Cell::new(1.0_f64));
            {
                let native = native.clone();
                cx.on(node, move |event| {
                    if let Event::ScrollZoomed(z) = event {
                        native.set(*z);
                        scale.set_if_changed(*z);
                    }
                });
            }
            let range = self.zoom;
            watch(
                move || scale.get(),
                move |want: &f64, _| {
                    let want = range.map_or(*want, |r| want.clamp(r.min, r.max));
                    if (want - native.get()).abs() > 1e-6 {
                        day_core::with_tree(|t| {
                            t.patch(
                                node,
                                Box::new(ScrollPatch::Zoom {
                                    scale: want,
                                    animated: true,
                                }),
                                false,
                            );
                        });
                    }
                },
            );
        }
        node
    }
}

/// Column lanes (docs/size-classes.md "Lanes"). Built by [`lanes`].
#[must_use]
pub struct Lanes<C: PieceSeq> {
    children: C,
    lane_width: f64,
    max: usize,
    spacing: f64,
    run_spacing: Option<f64>,
    balanced: bool,
}

/// Deal `children` into as many equal-width columns as the available width holds, each
/// column stacking its own: the layout a page of cards wants (docs/size-classes.md "Lanes").
///
/// The lane count is decided during layout from the width the container is given, so a
/// window dragged wider gains a lane and the same children move over: none is rebuilt, and a
/// card keeps its scroll position, its text and every other piece of state. Children are
/// dealt in turn (first to the first lane, second to the second, and so on), or with
/// [`balanced`](LanesBuilder::balanced) each to the lane that is shortest so far.
///
/// ```ignore
/// scroll(lanes(cards).lane_width(320.0).max_lanes(3).spacing(Space::L).padding(20.0))
/// ```
pub fn lanes<C: PieceSeq>(children: C) -> Lanes<C> {
    Lanes {
        children,
        lane_width: 320.0,
        max: usize::MAX,
        spacing: 8.0,
        run_spacing: None,
        balanced: false,
    }
}

#[day_macros::builder(LanesBuilder)]
impl<C: PieceSeq> Lanes<C> {
    /// The narrowest a lane gets, in points (default 320): the container has one lane for
    /// every `width` plus gap that fits, and the lanes then share the whole width equally.
    pub fn lane_width(mut self, width: f64) -> Self {
        self.lane_width = width.max(1.0);
        self
    }
    /// The most lanes, however wide the container.
    pub fn max_lanes(mut self, max: usize) -> Self {
        self.max = max.max(1);
        self
    }
    /// The gap between lanes, and between the children of a lane unless
    /// [`Self::run_spacing`] sets that apart.
    pub fn spacing(mut self, s: impl Into<f64>) -> Self {
        self.spacing = s.into();
        self
    }
    /// The gap between the children of one lane.
    pub fn run_spacing(mut self, s: impl Into<f64>) -> Self {
        self.run_spacing = Some(s.into());
        self
    }
    /// Put each child in the lane that is shortest so far, which evens the lanes out when
    /// the children differ in height. The default deals in turn, which keeps a child's lane
    /// predictable from its position.
    pub fn balanced(mut self) -> Self {
        self.balanced = true;
        self
    }
}

impl<C: PieceSeq> Piece for Lanes<C> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let node = cx.native(
            kinds::CONTAINER,
            &ContainerProps::default(),
            Rc::new(LanesLayout {
                min_width: self.lane_width,
                max: self.max,
                spacing: self.spacing,
                run_spacing: self.run_spacing.unwrap_or(self.spacing),
                balanced: self.balanced,
            }),
            Flex {
                grow_w: true,
                ..Default::default()
            },
            Boundary::No,
        );
        cx.under(node, |cx| self.children.build_each(cx));
        node
    }
}

/// A z-stack: children are layered back-to-front (the first child sits at the bottom), all
/// sharing the container bounds and positioned by the stack's [`Alignment`]. The stack sizes to
/// the union (max width/height) of its children; contrast [`Decorate::overlay`](crate::decorators::Decorate::overlay), which sizes to
/// its content and treats the overlaid piece as a non-sizing annotation. Pure composition: it is
/// the same native panel as [`column()`]/[`row()`], so there is no per-backend work.
#[must_use]
pub struct ZStack<C: PieceSeq> {
    children: C,
    align: Alignment,
}

/// Build a [`ZStack`] from a tuple, `Vec` or array of children.
pub fn zstack<C: PieceSeq>(children: C) -> ZStack<C> {
    ZStack {
        children,
        align: Alignment::Center,
    }
}

#[day_macros::builder(ZStackBuilder)]
impl<C: PieceSeq> ZStack<C> {
    /// Where children sit within the stack's bounds (default [`Alignment::Center`]).
    pub fn align(mut self, a: Alignment) -> Self {
        self.align = a;
        self
    }
}

impl<C: PieceSeq> Piece for ZStack<C> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let node = cx.native(
            kinds::CONTAINER,
            &ContainerProps::default(),
            Rc::new(OverlayLayout {
                align: self.align,
                size_to_first: false,
            }),
            Flex::default(),
            Boundary::No,
        );
        cx.under(node, |cx| self.children.build_each(cx));
        node
    }
}

// ---------------------------------------------------------------------------
// Conformance cases (docs/testing.md)
// ---------------------------------------------------------------------------

/// The containers' `#[day::test]` cases, next to their constructors.
#[cfg(feature = "conformance")]
pub(crate) mod conformance {
    use day_core::conformance::{Case, Drive, FrameExpect};
    use day_core::{AnyPiece, ScrollTarget};
    use day_reactive::Signal;
    use day_spec::{Color, kinds};

    use crate::*;

    fn block(color: Color, id: day_core::ElementId, w: f64, h: f64) -> AnyPiece {
        rectangle().fill(color).id(id).frame(w, h).any()
    }

    fn at(relative_to: &str, x: Option<f64>, y: Option<f64>) -> FrameExpect {
        FrameExpect {
            x,
            y,
            relative_to: Some(relative_to.into()),
            ..Default::default()
        }
    }

    /// A column stacks its children with the spacing it is given.
    #[day_macros::test(day_core)]
    fn column_spacing() -> Case {
        Case::new()
            .proves_duty("realize")
            .proves_duty("insert")
            .proves_duty("set_frame")
            .proves_duty("measure")
            .proves(kinds::CONTAINER)
            .page(|| {
                column((
                    block(Color::rgb(1.0, 0.0, 0.0), day_core::id!(a), 50.0, 20.0),
                    block(Color::rgb(0.0, 0.0, 1.0), day_core::id!(b), 50.0, 20.0),
                ))
                .spacing(10.0)
                .align(HAlign::Leading)
            })
            .drive(
                |d: Drive| async move { d.assert_frame("b", at("a", Some(0.0), Some(30.0))).await },
            )
    }

    /// A row lays its children side by side and centers them on its cross axis.
    #[day_macros::test(day_core)]
    fn row_alignment() -> Case {
        Case::new()
            .proves(kinds::CONTAINER)
            .page(|| {
                row((block(Color::rgb(1.0, 0.0, 0.0), day_core::id!(a), 50.0, 20.0), block(Color::rgb(0.0, 0.0, 1.0), day_core::id!(b), 50.0, 40.0)))
                    .spacing(8.0)
                    .align(VAlign::Center)
            })
            .drive(|d: Drive| async move {
                d.assert_frame("b", at("a", Some(58.0), Some(-10.0))).await
            })
    }

    /// A grid aligns its rows' cells into shared columns.
    #[day_macros::test(day_core)]
    fn grid_columns() -> Case {
        Case::new()
            .proves(kinds::CONTAINER)
            .page(|| {
                grid((
                    grid_row((
                        block(Color::rgb(1.0, 0.0, 0.0), day_core::id!(a), 40.0, 20.0),
                        block(Color::rgb(0.0, 0.0, 1.0), day_core::id!(b), 60.0, 20.0),
                    )),
                    grid_row((
                        block(Color::rgb(0.0, 0.0, 1.0), day_core::id!(c), 30.0, 20.0),
                        block(Color::rgb(1.0, 0.0, 0.0), day_core::id!(d), 30.0, 20.0),
                    )),
                ))
                .spacing(0.0)
                .align(day_core::Alignment::TopLeading)
            })
            .drive(|d: Drive| async move {
                // The second column starts after the first column's widest cell; cells sit at
                // their column's leading edge (they center by default).
                d.assert_frame("d", at("a", Some(40.0), Some(20.0))).await?;
                d.assert_frame("b", at("a", Some(40.0), Some(0.0))).await
            })
    }

    /// An absent optional cell (`None`) keeps its column, so the cells after it stay aligned
    /// with the rows that have every cell (docs/grid.md "Spanning and empty cells").
    #[day_macros::test(day_core)]
    fn grid_none_keeps_its_column() -> Case {
        Case::new()
            .proves(kinds::CONTAINER)
            .page(|| {
                let absent: Option<AnyPiece> = None;
                grid((
                    grid_row((
                        block(Color::rgb(1.0, 0.0, 0.0), day_core::id!(a), 40.0, 20.0),
                        block(Color::rgb(0.0, 0.0, 1.0), day_core::id!(b), 60.0, 20.0),
                        block(Color::rgb(1.0, 0.0, 0.0), day_core::id!(c), 30.0, 20.0),
                    )),
                    grid_row((
                        block(Color::rgb(0.0, 0.0, 1.0), day_core::id!(d), 30.0, 20.0),
                        absent,
                        block(Color::rgb(0.0, 0.0, 1.0), day_core::id!(f), 30.0, 20.0),
                    )),
                ))
                .spacing(0.0)
                .align(day_core::Alignment::TopLeading)
            })
            .drive(|d: Drive| async move {
                // The third column starts after the first two (40 + 60), in both rows.
                d.assert_frame("c", at("a", Some(100.0), Some(0.0))).await?;
                d.assert_frame("f", at("a", Some(100.0), Some(20.0))).await
            })
    }

    /// A zstack draws its children over each other, the last on top, centered by default.
    #[day_macros::test(day_core)]
    fn zstack_layers() -> Case {
        Case::new()
            .proves(kinds::CONTAINER)
            .proves_cap(day_spec::Cap::Snapshot)
            .page(|| {
                zstack((
                    block(Color::rgb(1.0, 0.0, 0.0), day_core::id!(under), 100.0, 60.0),
                    block(Color::rgb(0.0, 0.0, 1.0), day_core::id!(over), 50.0, 30.0),
                ))
                .id(day_core::id!(stack))
            })
            .shot("default")
            .drive(|d: Drive| async move {
                d.assert_frame("over", at("under", Some(25.0), Some(15.0)))
                    .await?;
                d.sample_pixel("under", 0.5, 0.5, "#0000ff").await?;
                d.sample_pixel("under", 0.05, 0.1, "#ff0000").await
            })
    }

    /// A scroll view scrolls natively to the target the app sets: the window shows what was
    /// out of view.
    #[day_macros::test(day_core)]
    fn scroll_to_target() -> Case {
        let jump = Signal::new(None::<ScrollTarget>);
        Case::new()
            .proves(kinds::SCROLL)
            .proves_duty("scroll_to")
            .proves_duty("set_scroll_content")
            .page(move || {
                let rows: Vec<AnyPiece> = (0..30)
                    .map(|i| {
                        let color = if i < 20 {
                            Color::rgb(1.0, 0.0, 0.0)
                        } else {
                            Color::rgb(0.0, 0.0, 1.0)
                        };
                        rectangle().fill(color).frame(120.0, 40.0).any()
                    })
                    .collect();
                column((
                    scroll(column(rows).spacing(0.0))
                        .scroll_target(jump)
                        .id(day_core::id!(scroll))
                        .frame(120.0, 120.0),
                    button("Bottom")
                        .action(move || jump.set(Some(ScrollTarget::Bottom)))
                        .id(day_core::id!(bottom)),
                ))
                .spacing(8.0)
            })
            .drive(|d: Drive| async move {
                d.sample_pixel("scroll", 0.5, 0.5, "#ff0000").await?;
                d.tap("bottom").await?;
                d.sample_pixel("scroll", 0.5, 0.5, "#0000ff").await
            })
    }

    /// A scroll view reports its offset after a programmatic scroll, and the bound signal
    /// follows it (docs/scroll.md "Offset and phase").
    #[day_macros::test(day_core)]
    fn scroll_offset_follows_a_programmatic_scroll() -> Case {
        Case::new()
            .proves(kinds::SCROLL)
            .proves_cap(day_spec::Cap::ScrollOffset)
            .requires(day_spec::Cap::ScrollOffset)
            .page(|| {
                let jump = Signal::new(None::<ScrollTarget>);
                let offset = Signal::new(day_spec::Point::ZERO);
                let lines: Vec<AnyPiece> = (0..30)
                    .map(|_| {
                        rectangle()
                            .fill(Color::rgb(0.5, 0.5, 0.5))
                            .frame(120.0, 40.0)
                            .any()
                    })
                    .collect();
                column((
                    scroll(column(lines).spacing(0.0))
                        .scroll_target(jump)
                        .scroll_offset(offset)
                        .id(day_core::id!(scroll))
                        .frame(120.0, 120.0),
                    label(move || format!("y {:.0}", offset.get().y)).id(day_core::id!(offset)),
                    button("Bottom")
                        .action(move || jump.set(Some(ScrollTarget::Bottom)))
                        .id(day_core::id!(bottom)),
                    button("Top")
                        .action(move || jump.set(Some(ScrollTarget::Top)))
                        .id(day_core::id!(top)),
                ))
                .spacing(8.0)
            })
            .drive(|d: Drive| async move {
                d.assert_text("offset", "y 0").await?;
                // 30 lines of 40 points in a 120-point viewport: the bottom is at 1080.
                d.tap("bottom").await?;
                d.assert_text("offset", "y 1080").await?;
                d.tap("top").await?;
                d.assert_text("offset", "y 0").await
            })
    }

    /// Content padding moves the content in and scrolls with it.
    #[day_macros::test(day_core)]
    fn scroll_content_padding_offsets_the_content() -> Case {
        Case::new()
            .proves(kinds::SCROLL)
            .page(|| {
                scroll(block(
                    Color::rgb(0.0, 0.0, 1.0),
                    day_core::id!(padded_content),
                    60.0,
                    60.0,
                ))
                .content_padding(day_spec::Insets {
                    top: 10.0,
                    leading: 20.0,
                    bottom: 10.0,
                    trailing: 20.0,
                })
                .id(day_core::id!(padded_scroll))
                .frame(160.0, 120.0)
            })
            .drive(|d: Drive| async move {
                d.assert_frame(
                    "padded_content",
                    at("padded_scroll", Some(20.0), Some(10.0)),
                )
                .await
            })
    }

    /// A zooming scroll view takes a scale the app writes, clamps it to its range, and
    /// reports the scale it settled on (docs/scroll.md "Zoom").
    #[day_macros::test(day_core)]
    fn scroll_zoom_scale_round_trips() -> Case {
        Case::new()
            .proves(kinds::SCROLL)
            .proves_cap(day_spec::Cap::ScrollZoom)
            .requires(day_spec::Cap::ScrollZoom)
            .page(|| {
                let scale = Signal::new(1.0_f64);
                column((
                    scroll(block(
                        Color::rgb(0.0, 0.5, 0.0),
                        day_core::id!(zoom_content),
                        100.0,
                        100.0,
                    ))
                    .zoom(1.0..=4.0)
                    .zoom_scale(scale)
                    .frame(120.0, 120.0),
                    label(move || format!("zoom {:.1}", scale.get())).id(day_core::id!(zoom)),
                    button("Double")
                        .action(move || scale.set(2.0))
                        .id(day_core::id!(zoom_double)),
                    button("Far")
                        .action(move || scale.set(9.0))
                        .id(day_core::id!(zoom_far)),
                ))
                .spacing(8.0)
            })
            .drive(|d: Drive| async move {
                d.assert_text("zoom", "zoom 1.0").await?;
                d.tap("zoom_double").await?;
                d.assert_text("zoom", "zoom 2.0").await?;
                // Past the range: the view stops at its maximum and says so.
                d.tap("zoom_far").await?;
                d.assert_text("zoom", "zoom 4.0").await
            })
    }

    /// Lanes deal their children into as many columns as the width holds, and move the same
    /// children when the width changes (docs/size-classes.md "Lanes").
    #[day_macros::test(day_core)]
    fn lanes_reflow_without_rebuilding() -> Case {
        Case::new()
            .page(|| {
                let width = Signal::new(430.0_f64);
                let built = Signal::new(0_u32);
                let card = move |id: day_core::ElementId, height: f64| {
                    built.update(|n| *n += 1);
                    rectangle()
                        .fill(Color::rgb(0.2, 0.4, 0.8))
                        .height(height)
                        .grow(Dimension::Width)
                        .wrapper_id(id)
                };
                column((
                    row((
                        button("Narrow")
                            .action(move || width.set(210.0))
                            .id(day_core::id!(lanes_narrow)),
                        label(move || format!("built {}", built.get()))
                            .id(day_core::id!(lanes_built)),
                    )),
                    lanes((
                        card(day_core::id!(lane_a), 80.0),
                        card(day_core::id!(lane_b), 40.0),
                        card(day_core::id!(lane_c), 30.0),
                    ))
                    .lane_width(200.0)
                    .max_lanes(3)
                    .spacing(10.0)
                    .id(day_core::id!(lanes))
                    .width(move || width.get()),
                ))
                .align(HAlign::Leading)
            })
            .drive(|d: Drive| async move {
                // 430 points hold two 200-point lanes of 210: a and c in the first, b in the
                // second.
                d.assert_size("lane_a", 210.0, 80.0).await?;
                d.assert_frame("lane_b", at("lane_a", Some(220.0), Some(0.0)))
                    .await?;
                d.assert_frame("lane_c", at("lane_a", Some(0.0), Some(90.0)))
                    .await?;
                // One lane: the same cards, stacked, none built again.
                d.tap("lanes_narrow").await?;
                d.assert_frame("lane_b", at("lane_a", Some(0.0), Some(90.0)))
                    .await?;
                d.assert_frame("lane_c", at("lane_a", Some(0.0), Some(140.0)))
                    .await?;
                d.assert_text("lanes_built", "built 3").await
            })
    }

    /// In a wrapping row the growing children share what their line has left, by weight.
    #[day_macros::test(day_core)]
    fn wrap_row_shares_each_line_by_weight() -> Case {
        Case::new()
            .proves_modifier("grow")
            .page(|| {
                let tile = |id: day_core::ElementId, weight: f64| {
                    rectangle()
                        .fill(Color::rgb(0.8, 0.3, 0.2))
                        .height(20.0)
                        .min_width(50.0)
                        .grow(Dimension::Width.weight(weight))
                        .wrapper_id(id)
                };
                row((
                    tile(day_core::id!(share_a), 1.0),
                    tile(day_core::id!(share_b), 3.0),
                    tile(day_core::id!(share_c), 1.0),
                ))
                .spacing(0.0)
                .fit(RowFit::Wrap { run_spacing: 10.0 })
                .width(140.0)
            })
            .drive(|d: Drive| async move {
                // 140 points hold two 50-point tiles; the 40 left go one to three.
                d.assert_size("share_a", 60.0, 20.0).await?;
                d.assert_size("share_b", 80.0, 20.0).await?;
                // The third wraps and has its line to itself.
                d.assert_size("share_c", 140.0, 20.0).await?;
                d.assert_frame("share_c", at("share_a", Some(0.0), Some(30.0)))
                    .await
            })
    }
}
