//! Layout containers that arrange child pieces: `column`, `row`, `grid`/`grid_row`, `scroll`,
//! and `zstack`, together with the `HAlign`/`VAlign` alignment enums.

use std::rc::Rc;

use day_core::*;
use day_reactive::{Signal, watch};
use day_spec::kinds;
use day_spec::props::*;

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

pub struct Row<C: PieceSeq> {
    children: C,
    spacing: f64,
    align: CrossAlign,
}

pub fn row<C: PieceSeq>(children: C) -> Row<C> {
    Row {
        children,
        spacing: 0.0,
        align: CrossAlign::Center,
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
        };
        self
    }
}

impl<C: PieceSeq> Piece for Row<C> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let node = cx.native(
            kinds::CONTAINER,
            &ContainerProps::default(),
            Rc::new(StackLayout {
                axis: Axis::Horizontal,
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

pub struct Grid<C: PieceSeq> {
    children: C,
    row_spacing: f64,
    column_spacing: f64,
    align: Alignment,
}

/// A SwiftUI-style eager grid (docs/grid.md): columns are inferred from [`grid_row`] children —
/// a column is as wide as its widest cell, a `grow_w` cell makes its column share the leftover
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
/// grid a row degrades gracefully to a plain [`row`]. Rows are transparent carriers — the grid
/// places their cells directly — so decorating a `grid_row` itself is unsupported (decorate the
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
        });
        self
    }
}

impl<C: PieceSeq> Piece for GridRow<C> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        // A layout-only node whose StackLayout only runs when the row is NOT inside a grid
        // (the graceful-degrade path) — a grid introspects the cells and places them itself.
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
    /// there (animated), then the signal resets to `None` — write-and-forget, so the same
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
                        day_core::with_tree(|tr| {
                            tr.scroll_to_target(node, &t, true);
                        });
                        sig.set(None); // consumed — ready for the next command
                    }
                },
            );
        }
        node
    }
}

/// A z-stack: children are layered back-to-front (the first child sits at the bottom), all
/// sharing the container bounds and positioned by the stack's [`Alignment`]. The stack sizes to
/// the UNION (max width/height) of its children — contrast [`Decorate::overlay`], which sizes to
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
