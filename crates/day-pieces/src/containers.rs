// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Layout containers that arrange child pieces: `column`, `row`, `grid`/`grid_row`, `scroll`,
//! and `zstack`, together with the `HAlign`/`VAlign` alignment enums.

use std::rc::Rc;

use day_core::*;
use day_reactive::{Signal, watch};
use day_spec::kinds;
use day_spec::props::*;

use crate::Decorated;

// ---------------------------------------------------------------------------
// Containers
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Default)]
pub enum HAlign {
    Leading,
    #[default]
    Center,
    Trailing,
}
#[derive(Clone, Copy, Default)]
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

impl<C: PieceSeq> Column<C> {
    pub fn spacing(mut self, s: f64) -> Self {
        self.spacing = s;
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

/// [`Column`]'s builders, reachable through a decoration (§5.2); see [`LabelBuilder`] for the
/// pattern. `column(…).padding(8.0).spacing(4.0)` resolves.
pub trait ColumnBuilder: Sized {
    fn spacing(self, s: f64) -> Self;
    fn align(self, a: HAlign) -> Self;
}

impl<C: PieceSeq> ColumnBuilder for Column<C> {
    fn spacing(self, s: f64) -> Self {
        Column::spacing(self, s)
    }
    fn align(self, a: HAlign) -> Self {
        Column::align(self, a)
    }
}

impl<P: ColumnBuilder + Piece> ColumnBuilder for Decorated<P> {
    fn spacing(self, s: f64) -> Self {
        self.map_inner(|p| p.spacing(s))
    }
    fn align(self, a: HAlign) -> Self {
        self.map_inner(|p| p.align(a))
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
/// call shape, `row((…)).fit(…)`, so a page can move between them as its needs change.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum RowFit {
    /// One line at natural sizes; what does not fit lands offscreen. The default, and the
    /// right contract for a label beside a value. Debug builds log the overflow once per
    /// container, naming the dayscript ids in reach.
    #[default]
    Clip,
    /// Break onto additional lines where the next child would overflow, like wrapped text:
    /// the shape a chip row or button strip wants. `run_spacing` is the vertical gap between
    /// lines. Wrapping replaces main-axis negotiation, so `.grow()` and `spacer()` are inert.
    Wrap { run_spacing: f64 },
    /// Wrap into aligned columns rather than ragged lines: every cell takes the widest
    /// child's width, and each line holds as many as the window fits. The tidier arm of
    /// [`Wrap`](RowFit::Wrap): same wrapping, but the lines stack into a grid, which is what
    /// a set of peer choices (a keypad, a palette, evenly-weighted chips) wants. The column
    /// count follows the available width. When any child grows (`.grow_w()`), the widest child
    /// becomes the narrowest a column gets and the columns stretch to share the whole width,
    /// the adaptive grid a gallery of tiles wants. No cell is wider than the line: on a window
    /// narrower than the widest child, the cells take its width and their text wraps. An
    /// authored, fixed column count with
    /// per-cell spans is [`grid`]'s job instead (docs/grid.md).
    WrapColumns { run_spacing: f64 },
    /// Re-arrange into a leading-aligned column while the window's [`WidthClass`] is at or
    /// below the given one: the shape a label-plus-control-plus-result line wants, where
    /// wrapping members independently would tear apart what reads as one sentence. The
    /// `size_class()` read is tracked, so crossing the breakpoint re-arranges it live; app
    /// state lives in signals and survives the rebuild.
    ColumnAt(day_spec::WidthClass),
    /// Keep the single line and make it a horizontal scroll strip instead of clipping. The
    /// strip stays one row tall and fills the width it is given.
    Scroll,
}

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

impl<C: PieceSeq> Row<C> {
    pub fn spacing(mut self, s: f64) -> Self {
        self.spacing = s;
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

/// [`Row`]'s builders, reachable through a decoration (§5.2); see [`LabelBuilder`] for the
/// pattern.
pub trait RowBuilder: Sized {
    fn spacing(self, s: f64) -> Self;
    fn align(self, a: VAlign) -> Self;
    fn fit(self, fit: RowFit) -> Self;
}

impl<C: PieceSeq> RowBuilder for Row<C> {
    fn spacing(self, s: f64) -> Self {
        Row::spacing(self, s)
    }
    fn align(self, a: VAlign) -> Self {
        Row::align(self, a)
    }
    fn fit(self, fit: RowFit) -> Self {
        Row::fit(self, fit)
    }
}

impl<P: RowBuilder + Piece> RowBuilder for Decorated<P> {
    fn spacing(self, s: f64) -> Self {
        self.map_inner(|p| p.spacing(s))
    }
    fn align(self, a: VAlign) -> Self {
        self.map_inner(|p| p.align(a))
    }
    fn fit(self, fit: RowFit) -> Self {
        self.map_inner(|p| p.fit(fit))
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
            RowFit::ColumnAt(limit) => {
                let stacked = size_class().is_some_and(|c| c.width <= limit);
                if stacked {
                    // Leading, not centered: stacked, these read as a control and the line
                    // it produced, and a centered member floats away from what it belongs to.
                    self.build_line(cx, Axis::Vertical, CrossAlign::Leading)
                } else {
                    let align = self.align;
                    self.build_line(cx, Axis::Horizontal, align)
                }
            }
            RowFit::Scroll => {
                // The row keeps its natural one-line measure inside a horizontal scroll
                // viewport. `grow_h` stays off: the strip is as tall as the row, not as
                // tall as whatever pane it happens to sit in.
                let strip = cx.native(
                    kinds::SCROLL,
                    &day_spec::props::ScrollProps { horizontal: true },
                    Rc::new(ScrollLayout {
                        axis: Axis::Horizontal,
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
/// alignment with [`Decorate::grid_span`] / [`Decorate::grid_align`].
pub fn grid<C: PieceSeq>(children: C) -> Grid<C> {
    Grid {
        children,
        row_spacing: 0.0,
        column_spacing: 0.0,
        align: Alignment::Center,
    }
}

impl<C: PieceSeq> Grid<C> {
    /// Set both the row and column gutters.
    pub fn spacing(mut self, s: f64) -> Self {
        self.row_spacing = s;
        self.column_spacing = s;
        self
    }
    pub fn row_spacing(mut self, s: f64) -> Self {
        self.row_spacing = s;
        self
    }
    pub fn column_spacing(mut self, s: f64) -> Self {
        self.column_spacing = s;
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

pub struct Scroll<P: Piece> {
    child: P,
    axis: Axis,
    target: Option<Signal<Option<day_core::ScrollTarget>>>,
}

pub fn scroll<P: Piece>(child: P) -> Scroll<P> {
    Scroll {
        child,
        axis: Axis::Vertical,
        target: None,
    }
}

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
    /// there (animated), then the signal resets to `None`. Write-and-forget, so the same
    /// target can be sent twice in a row.
    ///
    /// ```ignore
    /// let jump = Signal::new(None);
    /// scroll(rows).scroll_target(jump);
    /// button("Bottom").action(move || jump.set(Some(ScrollTarget::Bottom)));
    /// ```
    pub fn scroll_target(mut self, sig: Signal<Option<day_core::ScrollTarget>>) -> Self {
        self.target = Some(sig);
        self
    }
}

impl<P: Piece> Piece for Scroll<P> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let node = cx.native(
            kinds::SCROLL,
            &day_spec::props::ScrollProps {
                horizontal: matches!(self.axis, Axis::Horizontal),
            },
            Rc::new(ScrollLayout { axis: self.axis }),
            Flex {
                grow_w: true,
                grow_h: true,
                ..Default::default()
            },
            Boundary::Yes, // scroll viewports are layout boundaries (§7.4)
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
        node
    }
}

/// A z-stack: children are layered back-to-front (the first child sits at the bottom), all
/// sharing the container bounds and positioned by the stack's [`Alignment`]. The stack sizes to
/// the union (max width/height) of its children; contrast [`Decorate::overlay`], which sizes to
/// its content and treats the overlaid piece as a non-sizing annotation. Pure composition: it is
/// the same native panel as [`column`]/[`row`], so there is no per-backend work.
pub struct ZStack<C: PieceSeq> {
    children: C,
    align: Alignment,
}

/// Build a [`ZStack`] from a tuple of children (or a [`PieceVec`]).
pub fn zstack<C: PieceSeq>(children: C) -> ZStack<C> {
    ZStack {
        children,
        align: Alignment::Center,
    }
}

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

// --- Typed builders, forwarded through `Decorated` (docs/api-style.md) ---

/// [`Grid`]'s own builders, reachable through a decoration (§5.2): `Decorated` forwards them
/// to the piece it wraps, so generic modifiers and typed ones chain in any order.
pub trait GridBuilder: Sized {
    fn spacing(self, s: f64) -> Self;
    fn row_spacing(self, s: f64) -> Self;
    fn column_spacing(self, s: f64) -> Self;
    fn align(self, a: Alignment) -> Self;
}

impl<C: PieceSeq> GridBuilder for Grid<C> {
    fn spacing(self, s: f64) -> Self {
        Grid::spacing(self, s)
    }
    fn row_spacing(self, s: f64) -> Self {
        Grid::row_spacing(self, s)
    }
    fn column_spacing(self, s: f64) -> Self {
        Grid::column_spacing(self, s)
    }
    fn align(self, a: Alignment) -> Self {
        Grid::align(self, a)
    }
}

impl<Inner: GridBuilder + Piece> GridBuilder for Decorated<Inner> {
    fn spacing(self, s: f64) -> Self {
        self.map_inner(|inner_piece| inner_piece.spacing(s))
    }
    fn row_spacing(self, s: f64) -> Self {
        self.map_inner(|inner_piece| inner_piece.row_spacing(s))
    }
    fn column_spacing(self, s: f64) -> Self {
        self.map_inner(|inner_piece| inner_piece.column_spacing(s))
    }
    fn align(self, a: Alignment) -> Self {
        self.map_inner(|inner_piece| inner_piece.align(a))
    }
}

/// [`GridRow`]'s own builders, reachable through a decoration (§5.2): `Decorated` forwards them
/// to the piece it wraps, so generic modifiers and typed ones chain in any order.
pub trait GridRowBuilder: Sized {
    fn align(self, a: VAlign) -> Self;
}

impl<C: PieceSeq> GridRowBuilder for GridRow<C> {
    fn align(self, a: VAlign) -> Self {
        GridRow::align(self, a)
    }
}

impl<Inner: GridRowBuilder + Piece> GridRowBuilder for Decorated<Inner> {
    fn align(self, a: VAlign) -> Self {
        self.map_inner(|inner_piece| inner_piece.align(a))
    }
}

/// [`Scroll`]'s own builders, reachable through a decoration (§5.2): `Decorated` forwards them
/// to the piece it wraps, so generic modifiers and typed ones chain in any order.
pub trait ScrollBuilder: Sized {
    fn horizontal(self) -> Self;
    fn axis(self, axis: Axis) -> Self;
    fn scroll_target(self, sig: Signal<Option<day_core::ScrollTarget>>) -> Self;
}

impl<P: Piece> ScrollBuilder for Scroll<P> {
    fn horizontal(self) -> Self {
        Scroll::horizontal(self)
    }
    fn axis(self, axis: Axis) -> Self {
        Scroll::axis(self, axis)
    }
    fn scroll_target(self, sig: Signal<Option<day_core::ScrollTarget>>) -> Self {
        Scroll::scroll_target(self, sig)
    }
}

impl<Inner: ScrollBuilder + Piece> ScrollBuilder for Decorated<Inner> {
    fn horizontal(self) -> Self {
        self.map_inner(|inner_piece| inner_piece.horizontal())
    }
    fn axis(self, axis: Axis) -> Self {
        self.map_inner(|inner_piece| inner_piece.axis(axis))
    }
    fn scroll_target(self, sig: Signal<Option<day_core::ScrollTarget>>) -> Self {
        self.map_inner(|inner_piece| inner_piece.scroll_target(sig))
    }
}

/// [`ZStack`]'s own builders, reachable through a decoration (§5.2): `Decorated` forwards them
/// to the piece it wraps, so generic modifiers and typed ones chain in any order.
pub trait ZStackBuilder: Sized {
    fn align(self, a: Alignment) -> Self;
}

impl<C: PieceSeq> ZStackBuilder for ZStack<C> {
    fn align(self, a: Alignment) -> Self {
        ZStack::align(self, a)
    }
}

impl<Inner: ZStackBuilder + Piece> ZStackBuilder for Decorated<Inner> {
    fn align(self, a: Alignment) -> Self {
        self.map_inner(|inner_piece| inner_piece.align(a))
    }
}

// ---------------------------------------------------------------------------
// Conformance cases (docs/testing.md)
// ---------------------------------------------------------------------------

/// The containers' `#[day::test]` cases, next to their constructors.
#[cfg(feature = "conformance")]
pub(crate) mod conformance {
    use day_core::conformance::{Case, Drive, FrameExpect};
    use day_core::{AnyPiece, PieceVec, ScrollTarget};
    use day_reactive::Signal;
    use day_spec::{Color, kinds};

    use crate::*;

    fn block(color: Color, id: &str, w: f64, h: f64) -> AnyPiece {
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
            .proves(kinds::CONTAINER)
            .page(|| {
                column((
                    block(Color::rgb(1.0, 0.0, 0.0), "a", 50.0, 20.0),
                    block(Color::rgb(0.0, 0.0, 1.0), "b", 50.0, 20.0),
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
                row((block(Color::rgb(1.0, 0.0, 0.0), "a", 50.0, 20.0), block(Color::rgb(0.0, 0.0, 1.0), "b", 50.0, 40.0)))
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
                        block(Color::rgb(1.0, 0.0, 0.0), "a", 40.0, 20.0),
                        block(Color::rgb(0.0, 0.0, 1.0), "b", 60.0, 20.0),
                    )),
                    grid_row((
                        block(Color::rgb(0.0, 0.0, 1.0), "c", 30.0, 20.0),
                        block(Color::rgb(1.0, 0.0, 0.0), "d", 30.0, 20.0),
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

    /// A zstack draws its children over each other, the last on top, centered by default.
    #[day_macros::test(day_core)]
    fn zstack_layers() -> Case {
        Case::new()
            .proves(kinds::CONTAINER)
            .proves_cap(day_spec::Cap::Snapshot)
            .page(|| {
                zstack((
                    block(Color::rgb(1.0, 0.0, 0.0), "under", 100.0, 60.0),
                    block(Color::rgb(0.0, 0.0, 1.0), "over", 50.0, 30.0),
                ))
                .id("stack")
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
                    scroll(column(PieceVec(rows)).spacing(0.0))
                        .scroll_target(jump)
                        .id("scroll")
                        .frame(120.0, 120.0),
                    button("Bottom")
                        .action(move || jump.set(Some(ScrollTarget::Bottom)))
                        .id("bottom"),
                ))
                .spacing(8.0)
            })
            .drive(|d: Drive| async move {
                d.sample_pixel("scroll", 0.5, 0.5, "#ff0000").await?;
                d.tap("bottom").await?;
                d.sample_pixel("scroll", 0.5, 0.5, "#0000ff").await
            })
    }

    day_core::tests! { column_spacing, row_alignment, grid_columns, zstack_layers, scroll_to_target }
}
