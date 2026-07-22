//! The layout engine (DESIGN.md §7): parent-proposes/child-chooses with a proposal-keyed
//! measure cache. Layout impls are ours or user-provided; they never run reactive user code,
//! so the engine holds the single tree borrow for the whole pass.

use std::rc::Rc;

use day_spec::*;

use crate::tree::{Flex, RNode, Tree};

/// Open layout protocol (§7.2). `children` are the node's direct children; group nodes
/// (`when`/`each` anchors) are layout-transparent — stacks expand them inline.
pub trait Layout: 'static {
    fn measure(&self, cx: &mut dyn LayoutOps, children: &[RNode], p: Proposal) -> Size;
    fn place(&self, cx: &mut dyn LayoutOps, children: &[RNode], bounds: Rect);
}

/// The engine surface visible to `Layout` implementations.
pub trait LayoutOps {
    fn measure_child(&mut self, child: RNode, p: Proposal) -> Size;
    fn place_child(&mut self, child: RNode, rect: Rect);
    /// Place a child whose on-screen frame is NATIVE-owned (nav pages in splitter panes /
    /// nav-controller views): never direction-mirrored — the toolkit positions it.
    fn place_child_native(&mut self, child: RNode, rect: Rect) {
        self.place_child(child, rect);
    }
    fn flex_of(&self, child: RNode) -> Flex;
    fn children_of(&self, node: RNode) -> Vec<RNode>;
    /// Native intrinsic measurement of the CURRENT node (leaves).
    fn measure_leaf(&mut self, p: Proposal) -> Size;
    /// Report scroll content size for the CURRENT node (§7.6).
    fn set_scroll_content(&mut self, content: Size);
}

pub struct EngineCx<'a, B: Toolkit> {
    pub(crate) tree: &'a mut Tree<B>,
    pub(crate) offset: Point,
    pub(crate) current: RNode,
    /// The bounds the CURRENT node's `place()` was given — the mirroring axis for RTL.
    pub(crate) parent_size: Size,
}

impl<B: Toolkit> LayoutOps for EngineCx<'_, B> {
    fn measure_child(&mut self, child: RNode, p: Proposal) -> Size {
        measure_node(self.tree, child, p)
    }
    fn place_child(&mut self, child: RNode, rect: Rect) {
        // RTL (docs/localization): layouts compute LTR ("leading" = left); under a
        // right-to-left locale every horizontal placement mirrors around the parent's
        // width, so leading means right everywhere — rows reverse, padding swaps sides,
        // alignment flips — without any layout impl knowing about direction. Leaf CONTENT
        // (canvas drawing, text runs) is not mirrored; native text handles RTL itself.
        let rect = if crate::layout_direction() == day_geometry::LayoutDirection::Rtl {
            Rect::new(
                self.parent_size.width - rect.origin.x - rect.size.width,
                rect.origin.y,
                rect.size.width,
                rect.size.height,
            )
        } else {
            rect
        };
        place_node(self.tree, child, rect, self.offset, false);
    }
    fn place_child_native(&mut self, child: RNode, rect: Rect) {
        place_node(self.tree, child, rect, self.offset, false);
    }
    fn flex_of(&self, child: RNode) -> Flex {
        self.tree.node(child).map(|n| n.flex).unwrap_or_default()
    }
    fn children_of(&self, node: RNode) -> Vec<RNode> {
        self.tree
            .node(node)
            .map(|n| n.children.clone())
            .unwrap_or_default()
    }
    fn measure_leaf(&mut self, p: Proposal) -> Size {
        let Some(n) = self.tree.node(self.current) else {
            return Size::ZERO;
        };
        let kind = n.kind;
        let Some(h) = n.handle.clone() else {
            return Size::ZERO;
        };
        self.tree.toolkit.measure(&h, kind, p)
    }
    fn set_scroll_content(&mut self, content: Size) {
        let current = self.current;
        let Some(n) = self.tree.node_mut(current) else {
            return;
        };
        // Cache for scroll_to_target (§7.6): edge targets need content-minus-viewport math.
        n.scroll_content = Some(content);
        let Some(h) = n.handle.clone() else { return };
        self.tree.toolkit.set_scroll_content(&h, content);
    }
}

pub(crate) fn measure_node<B: Toolkit>(tree: &mut Tree<B>, node: RNode, p: Proposal) -> Size {
    let key = p.cache_key();
    let (layout, children) = {
        let Some(n) = tree.node(node) else {
            return Size::ZERO;
        };
        if !n.needs_measure
            && let Some(&(_, s)) = n.cache.iter().find(|(k, _)| *k == key)
        {
            return s;
        }
        (n.layout.clone(), n.children.clone())
    };
    let mut cx = EngineCx {
        tree,
        offset: Point::ZERO,
        current: node,
        parent_size: Size::ZERO, // placement never happens during measure
    };
    let size = layout.measure(&mut cx, &children, p);
    if let Some(n) = tree.node_mut(node) {
        n.needs_measure = false;
        if n.cache.len() >= 4 {
            n.cache.clear();
        }
        n.cache.push((key, size));
    }
    size
}

/// `rect` is in the parent NODE's coordinates; `offset` is the parent's origin in the nearest
/// native ancestor's coordinates (§7.1 — accumulated through layout-only nodes).
pub(crate) fn place_node<B: Toolkit>(
    tree: &mut Tree<B>,
    node: RNode,
    rect: Rect,
    offset: Point,
    is_root: bool,
) {
    let abs = Rect {
        origin: rect.origin.offset(offset.x, offset.y),
        size: rect.size,
    };
    let (layout, children, has_handle) = {
        let Some(n) = tree.node(node) else { return };
        (n.layout.clone(), n.children.clone(), n.handle.is_some())
    };
    let child_offset = if has_handle {
        if !is_root {
            let changed = tree
                .node(node)
                .map(|n| {
                    n.last_native_frame
                        .map(|f| !f.approx_eq(&abs, 0.25))
                        .unwrap_or(true)
                })
                .unwrap_or(false);
            if changed {
                let h = tree.node(node).and_then(|n| n.handle.clone());
                if let Some(h) = h {
                    tree.toolkit.set_frame(&h, abs, None);
                }
                if tree
                    .node(node)
                    .map(|n| n.kind == day_spec::kinds::CANVAS)
                    .unwrap_or(false)
                {
                    // Queue-only (§8.3): canvases re-record against the new size after layout.
                    crate::tree::enqueue_event(
                        crate::tree::rnode_to_id(node),
                        day_spec::Event::FrameChanged(abs.size),
                    );
                }
            }
        }
        Point::ZERO
    } else {
        abs.origin
    };
    if let Some(n) = tree.node_mut(node) {
        n.last_native_frame = Some(abs);
    }
    let mut cx = EngineCx {
        tree,
        offset: child_offset,
        current: node,
        parent_size: rect.size,
    };
    layout.place(&mut cx, &children, Rect::from_size(rect.size));
}

// ---------------------------------------------------------------------------
// Built-in layouts
// ---------------------------------------------------------------------------

/// Single-child pass-through (root, wrappers, group fallback): top-leading.
pub struct PassThrough;

impl Layout for PassThrough {
    fn measure(&self, cx: &mut dyn LayoutOps, children: &[RNode], p: Proposal) -> Size {
        match children.first() {
            Some(&c) => cx.measure_child(c, p),
            None => Size::ZERO,
        }
    }
    fn place(&self, cx: &mut dyn LayoutOps, children: &[RNode], bounds: Rect) {
        if let Some(&c) = children.first() {
            let s = cx.measure_child(c, Proposal::exact(bounds.size));
            cx.place_child(c, Rect::from_size(s));
        }
    }
}

/// Native leaf: measurement delegates to the toolkit.
pub struct LeafLayout;

impl Layout for LeafLayout {
    fn measure(&self, cx: &mut dyn LayoutOps, _children: &[RNode], p: Proposal) -> Size {
        cx.measure_leaf(p)
    }
    fn place(&self, _cx: &mut dyn LayoutOps, _children: &[RNode], _bounds: Rect) {}
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    Vertical,
    Horizontal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CrossAlign {
    Leading,
    #[default]
    Center,
    Trailing,
}

impl CrossAlign {
    /// Placement fraction of the free space: 0 = leading, 0.5 = center, 1 = trailing.
    fn fraction(self) -> f64 {
        match self {
            CrossAlign::Leading => 0.0,
            CrossAlign::Center => 0.5,
            CrossAlign::Trailing => 1.0,
        }
    }
}

/// Column/row negotiation (§7.2): rigid children first, remaining main-axis space divided
/// among flexible children; `spacer()` is maximally flexible; group anchors expand inline.
pub struct StackLayout {
    pub axis: Axis,
    pub spacing: f64,
    pub align: CrossAlign,
}

impl StackLayout {
    fn main(&self, s: Size) -> f64 {
        match self.axis {
            Axis::Vertical => s.height,
            Axis::Horizontal => s.width,
        }
    }
    fn cross(&self, s: Size) -> f64 {
        match self.axis {
            Axis::Vertical => s.width,
            Axis::Horizontal => s.height,
        }
    }
    fn mk(&self, main: f64, cross: f64) -> Size {
        match self.axis {
            Axis::Vertical => Size::new(cross, main),
            Axis::Horizontal => Size::new(main, cross),
        }
    }
    fn split(&self, p: Proposal) -> (Option<f64>, Option<f64>) {
        match self.axis {
            Axis::Vertical => (p.height, p.width),
            Axis::Horizontal => (p.width, p.height),
        }
    }
    fn proposal(&self, main: Option<f64>, cross: Option<f64>) -> Proposal {
        match self.axis {
            Axis::Vertical => Proposal::new(cross, main),
            Axis::Horizontal => Proposal::new(main, cross),
        }
    }
    fn grows_main(&self, f: Flex) -> bool {
        f.is_spacer
            || match self.axis {
                Axis::Vertical => f.grow_h,
                Axis::Horizontal => f.grow_w,
            }
    }
    fn grows_cross(&self, f: Flex) -> bool {
        match self.axis {
            Axis::Vertical => f.grow_w,
            Axis::Horizontal => f.grow_h,
        }
    }

    fn flatten(cx: &mut dyn LayoutOps, children: &[RNode], out: &mut Vec<RNode>) {
        for &c in children {
            if cx.flex_of(c).is_group {
                let inner = cx.children_of(c);
                Self::flatten(cx, &inner, out);
            } else {
                out.push(c);
            }
        }
    }

    fn negotiate(&self, cx: &mut dyn LayoutOps, kids: &[RNode], p: Proposal) -> Vec<Size> {
        let (main_p, cross_p) = self.split(p);
        let mut sizes = vec![Size::ZERO; kids.len()];
        let mut flex_idx = Vec::new();
        let mut rigid_main = 0.0;
        for (i, &k) in kids.iter().enumerate() {
            let f = cx.flex_of(k);
            if self.grows_main(f) {
                flex_idx.push(i);
            } else {
                let s = cx.measure_child(k, self.proposal(None, cross_p));
                rigid_main += self.main(s);
                sizes[i] = s;
            }
        }
        if !flex_idx.is_empty() {
            let spacing_total = self.spacing * (kids.len().saturating_sub(1)) as f64;
            match main_p {
                Some(mp) => {
                    let remaining = (mp - rigid_main - spacing_total).max(0.0);
                    let share = remaining / flex_idx.len() as f64;
                    for &i in &flex_idx {
                        let f = cx.flex_of(kids[i]);
                        sizes[i] = if f.is_spacer {
                            self.mk(share, 0.0)
                        } else {
                            cx.measure_child(kids[i], self.proposal(Some(share), cross_p))
                        };
                    }
                }
                None => {
                    for &i in &flex_idx {
                        let f = cx.flex_of(kids[i]);
                        sizes[i] = if f.is_spacer {
                            Size::ZERO
                        } else {
                            cx.measure_child(kids[i], self.proposal(None, cross_p))
                        };
                    }
                }
            }
        }
        sizes
    }
}

impl Layout for StackLayout {
    fn measure(&self, cx: &mut dyn LayoutOps, children: &[RNode], p: Proposal) -> Size {
        let mut kids = Vec::new();
        Self::flatten(cx, children, &mut kids);
        if kids.is_empty() {
            return Size::ZERO;
        }
        let (main_p, cross_p) = self.split(p);
        let sizes = self.negotiate(cx, &kids, p);
        let spacing_total = self.spacing * (kids.len() - 1) as f64;
        let has_flex = kids.iter().any(|&k| self.grows_main(cx.flex_of(k)));
        let main_total = match main_p {
            Some(mp) if has_flex => mp,
            _ => sizes.iter().map(|&s| self.main(s)).sum::<f64>() + spacing_total,
        };
        let grows_cross = kids.iter().any(|&k| self.grows_cross(cx.flex_of(k)));
        let cross_total = match cross_p {
            Some(cp) if grows_cross => cp,
            _ => sizes.iter().map(|&s| self.cross(s)).fold(0.0, f64::max),
        };
        self.mk(main_total, cross_total)
    }

    fn place(&self, cx: &mut dyn LayoutOps, children: &[RNode], bounds: Rect) {
        let mut kids = Vec::new();
        Self::flatten(cx, children, &mut kids);
        if kids.is_empty() {
            return;
        }
        let sizes = self.negotiate(cx, &kids, Proposal::exact(bounds.size));
        let bounds_cross = self.cross(bounds.size);
        let mut pos = 0.0;
        for (i, &k) in kids.iter().enumerate() {
            let s = sizes[i];
            let cross_off = match self.align {
                CrossAlign::Leading => 0.0,
                CrossAlign::Center => ((bounds_cross - self.cross(s)) / 2.0).max(0.0),
                CrossAlign::Trailing => (bounds_cross - self.cross(s)).max(0.0),
            };
            let rect = match self.axis {
                Axis::Vertical => Rect::new(cross_off, pos, s.width, s.height),
                Axis::Horizontal => Rect::new(pos, cross_off, s.width, s.height),
            };
            cx.place_child(k, rect);
            pos += self.main(s) + self.spacing;
        }
    }
}

/// A grid cell, resolved from the realized tree: the node, its starting column, its span, and
/// its layout facts (docs/grid.md).
struct GridCell {
    node: RNode,
    col: usize,
    span: usize,
    flex: Flex,
}

/// One grid row: either a `grid_row`'s cells, or a single full-width cell (a non-row child).
struct GridRowRef {
    cells: Vec<GridCell>,
    full_width: bool,
    valign: Option<CrossAlign>,
}

/// The resolved geometry both [`GridLayout::measure`] and [`GridLayout::place`] work from.
struct GridGeom {
    rows: Vec<GridRowRef>,
    col_x: Vec<f64>,
    col_w: Vec<f64>,
    row_y: Vec<f64>,
    row_h: Vec<f64>,
    /// Pass-B size per cell, indexed `[row][cell]` — placement uses these, never re-measuring.
    cell_sizes: Vec<Vec<Size>>,
    size: Size,
}

/// SwiftUI-style grid negotiation (docs/grid.md): columns are inferred from `grid_row` children,
/// a column's width is the max ideal width of its span-1 cells, a `grow_w` cell makes its column
/// flexible (leftover width split evenly — the [`StackLayout`] share rule), and a non-row child
/// is a full-width cell spanning every column. `spacer()` is an inert empty cell. The contract:
/// exactly two measure proposals per cell per layout — unconstrained (pass A) and at the final
/// column width (pass B) — and `place` re-runs the same proposals, so it measures from cache.
pub struct GridLayout {
    pub row_spacing: f64,
    pub column_spacing: f64,
    pub align: Alignment,
}

impl GridLayout {
    /// Expand group anchors and classify children into rows / full-width cells.
    fn collect(&self, cx: &mut dyn LayoutOps, children: &[RNode]) -> (Vec<GridRowRef>, usize) {
        let mut tops = Vec::new();
        StackLayout::flatten(cx, children, &mut tops);
        let mut rows = Vec::new();
        let mut ncols = 0usize;
        for &t in &tops {
            let f = cx.flex_of(t);
            if f.grid.is_row {
                let inner = cx.children_of(t);
                let mut cell_nodes = Vec::new();
                StackLayout::flatten(cx, &inner, &mut cell_nodes);
                let mut cells = Vec::new();
                let mut col = 0usize;
                for &c in &cell_nodes {
                    let cf = cx.flex_of(c);
                    let span = cf.grid.col_span.max(1) as usize;
                    cells.push(GridCell {
                        node: c,
                        col,
                        span,
                        flex: cf,
                    });
                    col += span;
                }
                ncols = ncols.max(col);
                rows.push(GridRowRef {
                    cells,
                    full_width: false,
                    valign: f.grid.row_valign,
                });
            } else {
                // A non-row child occupies a full-width row spanning every column (span is
                // patched to the final column count once it is known).
                rows.push(GridRowRef {
                    cells: vec![GridCell {
                        node: t,
                        col: 0,
                        span: 1,
                        flex: f,
                    }],
                    full_width: true,
                    valign: None,
                });
            }
        }
        (rows, ncols)
    }

    fn span_width(&self, col_w: &[f64], col: usize, span: usize) -> f64 {
        let end = (col + span).min(col_w.len());
        let cols: f64 = col_w[col..end].iter().sum();
        cols + self.column_spacing * (end.saturating_sub(col + 1)) as f64
    }

    /// The single geometry pass (docs/grid.md): pass A (unconstrained ideals → column widths),
    /// pass B (heights at final widths), then prefix sums. All decisions are closed-form — no
    /// iterative negotiation — and only `p.width` affects cell proposals, so a `measure` at
    /// `(Some(w), None)` and a `place` at `exact(w × h)` generate identical per-cell proposals.
    fn geometry(&self, cx: &mut dyn LayoutOps, children: &[RNode], p: Proposal) -> GridGeom {
        let (mut rows, ncols) = self.collect(cx, children);
        if rows.is_empty() {
            return GridGeom {
                rows,
                col_x: Vec::new(),
                col_w: Vec::new(),
                row_y: Vec::new(),
                row_h: Vec::new(),
                cell_sizes: Vec::new(),
                size: Size::ZERO,
            };
        }
        let ncols = ncols.max(1);
        for r in &mut rows {
            if r.full_width {
                r.cells[0].span = ncols;
            }
        }
        let unconstrained = Proposal::new(None, None);

        // PASS A1 — span-1 cells: rigid ideals set their column's width; grow_w flags it
        // flexible (its unconstrained ideal only matters when the grid itself is unconstrained).
        let mut col_ideal = vec![0.0f64; ncols];
        let mut col_flex = vec![false; ncols];
        let mut flex_ideal = vec![0.0f64; ncols];
        for r in rows.iter().filter(|r| !r.full_width) {
            for c in r.cells.iter().filter(|c| c.span == 1 && !c.flex.is_spacer) {
                if c.flex.grow_w {
                    col_flex[c.col] = true;
                    if p.width.is_none() {
                        let s = cx.measure_child(c.node, unconstrained);
                        flex_ideal[c.col] = flex_ideal[c.col].max(s.width);
                    }
                } else {
                    let s = cx.measure_child(c.node, unconstrained);
                    col_ideal[c.col] = col_ideal[c.col].max(s.width);
                }
            }
        }
        // PASS A2 — flexible spanning cells only flag their columns…
        for r in rows.iter().filter(|r| !r.full_width) {
            for c in r.cells.iter().filter(|c| c.span > 1 && !c.flex.is_spacer) {
                if c.flex.grow_w {
                    let end = (c.col + c.span).min(ncols);
                    col_flex[c.col..end].fill(true);
                }
            }
        }
        // …PASS A3 — then rigid spanning cells distribute any width deficit in one shot: onto
        // the spanned flexible columns if any (they absorb width anyway), else evenly.
        for r in rows.iter().filter(|r| !r.full_width) {
            for c in r.cells.iter().filter(|c| c.span > 1 && !c.flex.is_spacer) {
                if c.flex.grow_w {
                    continue;
                }
                let s = cx.measure_child(c.node, unconstrained);
                let end = (c.col + c.span).min(ncols);
                let avail = self.span_width(&col_ideal, c.col, c.span);
                let deficit = s.width - avail;
                if deficit > 0.0 {
                    let flexed: Vec<usize> = (c.col..end).filter(|&k| col_flex[k]).collect();
                    let targets = if flexed.is_empty() {
                        (c.col..end).collect()
                    } else {
                        flexed
                    };
                    let add = deficit / targets.len() as f64;
                    for k in targets {
                        col_ideal[k] += add;
                    }
                }
            }
        }
        // Full-width cells: their ideal widens the grid when nothing constrains it; a grow_w
        // full-width cell makes the grid width flexible like a flexible column does.
        let mut fw_ideal = 0.0f64;
        let mut fw_flex = false;
        for r in rows.iter().filter(|r| r.full_width) {
            let c = &r.cells[0];
            if c.flex.is_spacer {
                continue;
            }
            if c.flex.grow_w {
                fw_flex = true;
            } else {
                let s = cx.measure_child(c.node, unconstrained);
                fw_ideal = fw_ideal.max(s.width);
            }
        }

        // Resolve column widths (the StackLayout::negotiate share rule for flexible columns).
        let gutters = self.column_spacing * (ncols - 1) as f64;
        let has_flex_col = col_flex.iter().any(|&f| f);
        let mut col_w = vec![0.0f64; ncols];
        match p.width {
            Some(pw) if has_flex_col => {
                let rigid: f64 = (0..ncols)
                    .filter(|&k| !col_flex[k])
                    .map(|k| col_ideal[k])
                    .sum();
                let share = (pw - rigid - gutters).max(0.0)
                    / col_flex.iter().filter(|&&f| f).count() as f64;
                for k in 0..ncols {
                    col_w[k] = if col_flex[k] { share } else { col_ideal[k] };
                }
            }
            _ => {
                for k in 0..ncols {
                    col_w[k] = if col_flex[k] {
                        flex_ideal[k].max(col_ideal[k])
                    } else {
                        col_ideal[k]
                    };
                }
            }
        }
        let cols_total: f64 = col_w.iter().sum::<f64>() + gutters;
        let grid_w = match p.width {
            Some(pw) if has_flex_col || fw_flex => pw,
            _ => cols_total.max(fw_ideal),
        };

        // PASS B — heights at final widths (text height-for-width happens here).
        let nrows = rows.len();
        let mut cell_sizes: Vec<Vec<Size>> = Vec::with_capacity(nrows);
        let mut row_h = vec![0.0f64; nrows];
        let mut row_flex = vec![false; nrows];
        for (ri, r) in rows.iter().enumerate() {
            let mut sizes = Vec::with_capacity(r.cells.len());
            for c in &r.cells {
                if c.flex.is_spacer {
                    sizes.push(Size::ZERO);
                    continue;
                }
                let w = if r.full_width {
                    grid_w
                } else {
                    self.span_width(&col_w, c.col, c.span)
                };
                let s = cx.measure_child(c.node, Proposal::new(Some(w), None));
                row_h[ri] = row_h[ri].max(s.height);
                if c.flex.grow_h {
                    row_flex[ri] = true;
                }
                sizes.push(s);
            }
            cell_sizes.push(sizes);
        }
        // Flexible rows stretch to a height proposal (additive — this never re-measures cells,
        // which keeps measure/place proposal identity).
        let vgutters = self.row_spacing * (nrows - 1) as f64;
        if let Some(ph) = p.height
            && row_flex.iter().any(|&f| f)
        {
            let total: f64 = row_h.iter().sum::<f64>() + vgutters;
            let extra = (ph - total).max(0.0) / row_flex.iter().filter(|&&f| f).count() as f64;
            for (ri, flexed) in row_flex.iter().enumerate() {
                if *flexed {
                    row_h[ri] += extra;
                }
            }
        }
        let grid_h = row_h.iter().sum::<f64>() + vgutters;

        let mut col_x = vec![0.0f64; ncols];
        let mut x = 0.0;
        for k in 0..ncols {
            col_x[k] = x;
            x += col_w[k] + self.column_spacing;
        }
        let mut row_y = vec![0.0f64; nrows];
        let mut y = 0.0;
        for ri in 0..nrows {
            row_y[ri] = y;
            y += row_h[ri] + self.row_spacing;
        }
        GridGeom {
            rows,
            col_x,
            col_w,
            row_y,
            row_h,
            cell_sizes,
            size: Size::new(grid_w, grid_h),
        }
    }
}

impl Layout for GridLayout {
    fn measure(&self, cx: &mut dyn LayoutOps, children: &[RNode], p: Proposal) -> Size {
        self.geometry(cx, children, p).size
    }

    fn place(&self, cx: &mut dyn LayoutOps, children: &[RNode], bounds: Rect) {
        let g = self.geometry(cx, children, Proposal::exact(bounds.size));
        for (ri, r) in g.rows.iter().enumerate() {
            for (ci, c) in r.cells.iter().enumerate() {
                if c.flex.is_spacer {
                    continue;
                }
                let cell = if r.full_width {
                    Rect::new(0.0, g.row_y[ri], g.size.width, g.row_h[ri])
                } else {
                    Rect::new(
                        g.col_x[c.col],
                        g.row_y[ri],
                        self.span_width(&g.col_w, c.col, c.span),
                        g.row_h[ri],
                    )
                };
                let s = g.cell_sizes[ri][ci];
                let w = if c.flex.grow_w {
                    cell.size.width
                } else {
                    s.width.min(cell.size.width)
                };
                let h = if c.flex.grow_h {
                    cell.size.height
                } else {
                    s.height.min(cell.size.height)
                };
                // Alignment precedence per axis: cell `.grid_align` > row `.align` (vertical
                // only) > the grid's own alignment.
                let hf = match c.flex.grid.align {
                    Some(a) => a.h_fraction(),
                    None => self.align.h_fraction(),
                };
                let vf = match (c.flex.grid.align, r.valign) {
                    (Some(a), _) => a.v_fraction(),
                    (None, Some(v)) => v.fraction(),
                    (None, None) => self.align.v_fraction(),
                };
                let x = cell.origin.x + (cell.size.width - w) * hf;
                let y = cell.origin.y + (cell.size.height - h) * vf;
                cx.place_child(c.node, Rect::new(x, y, w, h));
            }
        }
        // Row nodes are transparent carriers and are deliberately never placed.
    }
}

/// Two-axis placement of a child within a container's bounds (SwiftUI's `Alignment`). Used by
/// the z-layering primitives ([`OverlayLayout`]): `zstack`, `overlay`/`overlay_aligned`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Alignment {
    TopLeading,
    Top,
    TopTrailing,
    Leading,
    #[default]
    Center,
    Trailing,
    BottomLeading,
    Bottom,
    BottomTrailing,
}

impl Alignment {
    /// Horizontal placement fraction of the free space: 0 = leading, 0.5 = center, 1 = trailing.
    fn h_fraction(self) -> f64 {
        match self {
            Alignment::TopLeading | Alignment::Leading | Alignment::BottomLeading => 0.0,
            Alignment::Top | Alignment::Center | Alignment::Bottom => 0.5,
            Alignment::TopTrailing | Alignment::Trailing | Alignment::BottomTrailing => 1.0,
        }
    }
    /// Vertical placement fraction of the free space: 0 = top, 0.5 = center, 1 = bottom.
    fn v_fraction(self) -> f64 {
        match self {
            Alignment::TopLeading | Alignment::Top | Alignment::TopTrailing => 0.0,
            Alignment::Leading | Alignment::Center | Alignment::Trailing => 0.5,
            Alignment::BottomLeading | Alignment::Bottom | Alignment::BottomTrailing => 1.0,
        }
    }
}

/// Z-layering (§overlay): children share the container bounds, stacked back-to-front in child
/// order (first child = bottom of the z-order), each positioned by a single [`Alignment`].
/// `size_to_first` reports only the FIRST child's natural size — the badge/annotation sizing of
/// [`overlay`](crate) (the annotation does not grow the frame); otherwise the layout reports the
/// UNION (max) of all children's natural sizes — the ZStack sizing of `zstack`. No native work:
/// the container is the same panel as `column`/`row`, so backends stack children by attach order.
pub struct OverlayLayout {
    pub align: Alignment,
    pub size_to_first: bool,
}

impl OverlayLayout {
    /// Expand group anchors (`when`/`each`) inline, exactly like [`StackLayout`].
    fn flatten(cx: &mut dyn LayoutOps, children: &[RNode], out: &mut Vec<RNode>) {
        for &c in children {
            if cx.flex_of(c).is_group {
                let inner = cx.children_of(c);
                Self::flatten(cx, &inner, out);
            } else {
                out.push(c);
            }
        }
    }
}

impl Layout for OverlayLayout {
    fn measure(&self, cx: &mut dyn LayoutOps, children: &[RNode], p: Proposal) -> Size {
        let mut kids = Vec::new();
        Self::flatten(cx, children, &mut kids);
        if self.size_to_first {
            return match kids.first() {
                Some(&c) => cx.measure_child(c, p),
                None => Size::ZERO,
            };
        }
        let mut size = Size::ZERO;
        for &c in &kids {
            let s = cx.measure_child(c, p);
            size.width = size.width.max(s.width);
            size.height = size.height.max(s.height);
        }
        size
    }
    fn place(&self, cx: &mut dyn LayoutOps, children: &[RNode], bounds: Rect) {
        let mut kids = Vec::new();
        Self::flatten(cx, children, &mut kids);
        for &c in &kids {
            let s = cx.measure_child(c, Proposal::exact(bounds.size));
            let x = (bounds.size.width - s.width) * self.align.h_fraction();
            let y = (bounds.size.height - s.height) * self.align.v_fraction();
            cx.place_child(c, Rect::new(x, y, s.width, s.height));
        }
    }
}

pub struct PaddingLayout {
    pub insets: Insets,
}

impl Layout for PaddingLayout {
    fn measure(&self, cx: &mut dyn LayoutOps, children: &[RNode], p: Proposal) -> Size {
        let inner = Proposal::new(
            p.width.map(|w| (w - self.insets.horizontal()).max(0.0)),
            p.height.map(|h| (h - self.insets.vertical()).max(0.0)),
        );
        let s = match children.first() {
            Some(&c) => cx.measure_child(c, inner),
            None => Size::ZERO,
        };
        Size::new(
            s.width + self.insets.horizontal(),
            s.height + self.insets.vertical(),
        )
    }
    fn place(&self, cx: &mut dyn LayoutOps, children: &[RNode], bounds: Rect) {
        if let Some(&c) = children.first() {
            let inner = bounds.inset_by(self.insets);
            let s = cx.measure_child(c, Proposal::exact(inner.size));
            cx.place_child(
                c,
                Rect {
                    origin: inner.origin,
                    size: s,
                },
            );
        }
    }
}

/// The `grow`/`grow_w`/`grow_h` decorators (§5.2): a single-child wrapper carrying grow [`Flex`]
/// so the parent stack OFFERS it the space, and a greedy measure/place so the child actually
/// FILLS it. Non-grown axes hug the child (like `frame(maxWidth: .infinity)` on one axis).
pub struct GrowLayout {
    pub w: bool,
    pub h: bool,
}

impl Layout for GrowLayout {
    fn measure(&self, cx: &mut dyn LayoutOps, children: &[RNode], p: Proposal) -> Size {
        let cs = match children.first() {
            Some(&c) => cx.measure_child(c, p),
            None => Size::ZERO,
        };
        Size::new(
            if self.w {
                p.width.unwrap_or(cs.width)
            } else {
                cs.width
            },
            if self.h {
                p.height.unwrap_or(cs.height)
            } else {
                cs.height
            },
        )
    }
    fn place(&self, cx: &mut dyn LayoutOps, children: &[RNode], bounds: Rect) {
        if let Some(&c) = children.first() {
            let cs = cx.measure_child(c, Proposal::exact(bounds.size));
            // Fill the grown axes; hug the child on the rest.
            let w = if self.w { bounds.size.width } else { cs.width };
            let h = if self.h {
                bounds.size.height
            } else {
                cs.height
            };
            cx.place_child(c, Rect::from_size(Size::new(w, h)));
        }
    }
}

pub struct FrameLayout {
    pub width: Option<f64>,
    pub height: Option<f64>,
}

impl Layout for FrameLayout {
    fn measure(&self, cx: &mut dyn LayoutOps, children: &[RNode], p: Proposal) -> Size {
        let child_p = Proposal::new(self.width.or(p.width), self.height.or(p.height));
        let s = match children.first() {
            Some(&c) => cx.measure_child(c, child_p),
            None => Size::ZERO,
        };
        Size::new(
            self.width.unwrap_or(s.width),
            self.height.unwrap_or(s.height),
        )
    }
    fn place(&self, cx: &mut dyn LayoutOps, children: &[RNode], bounds: Rect) {
        if let Some(&c) = children.first() {
            cx.place_child(c, bounds);
        }
    }
}

/// Scroll viewport (§7.6): greedy on the proposal; content measured unconstrained on the
/// scroll axis and reported via `set_scroll_content`. Children are placed in the scroll's
/// content coordinate space (the scroll node is their native ancestor).
pub struct ScrollLayout {
    pub axis: Axis,
}

impl Layout for ScrollLayout {
    fn measure(&self, cx: &mut dyn LayoutOps, children: &[RNode], p: Proposal) -> Size {
        let content_p = match self.axis {
            Axis::Vertical => Proposal::new(p.width, None),
            Axis::Horizontal => Proposal::new(None, p.height),
        };
        let cs = match children.first() {
            Some(&c) => cx.measure_child(c, content_p),
            None => Size::ZERO,
        };
        Size::new(p.width.unwrap_or(cs.width), p.height.unwrap_or(cs.height))
    }
    fn place(&self, cx: &mut dyn LayoutOps, children: &[RNode], bounds: Rect) {
        if let Some(&c) = children.first() {
            let content_p = match self.axis {
                Axis::Vertical => Proposal::new(Some(bounds.size.width), None),
                Axis::Horizontal => Proposal::new(None, Some(bounds.size.height)),
            };
            let cs = cx.measure_child(c, content_p);
            let content = match self.axis {
                Axis::Vertical => Size::new(bounds.size.width, cs.height.max(bounds.size.height)),
                Axis::Horizontal => Size::new(cs.width.max(bounds.size.width), bounds.size.height),
            };
            cx.place_child(c, Rect::from_size(content));
            cx.set_scroll_content(content);
        }
    }
}

/// Navigation host (docs/navigation.md): page FRAMES are native-owned (splitter panes,
/// nav-controller views), so `set_frame` on pages is a toolkit no-op; Day lays each page's
/// CONTENT within the size the toolkit last reported via `Event::FrameChanged`, falling
/// back to a sidebar/detail split (or the full host) of the host bounds.
pub struct NavLayout {
    pub sizes: std::rc::Rc<std::cell::RefCell<std::collections::HashMap<RNode, Size>>>,
    pub split: bool,
}

pub use day_spec::NAV_SIDEBAR_WIDTH;

impl Layout for NavLayout {
    fn measure(&self, _cx: &mut dyn LayoutOps, _children: &[RNode], p: Proposal) -> Size {
        // Greedy: the host owns the window. A nested stack merges into the enclosing host's page
        // list rather than nesting a second host (docs/navigation.md), so one NAV host still
        // spans the window.
        Size::new(p.width.unwrap_or(480.0), p.height.unwrap_or(640.0))
    }
    fn place(&self, cx: &mut dyn LayoutOps, children: &[RNode], bounds: Rect) {
        for (i, &page) in children.iter().enumerate() {
            let reported = self.sizes.borrow().get(&page).copied();
            let sz = reported.unwrap_or_else(|| {
                if self.split {
                    if i == 0 {
                        Size::new(NAV_SIDEBAR_WIDTH, bounds.size.height)
                    } else {
                        Size::new(
                            (bounds.size.width - NAV_SIDEBAR_WIDTH - 1.0).max(0.0),
                            bounds.size.height,
                        )
                    }
                } else {
                    bounds.size
                }
            });
            cx.place_child_native(page, Rect::from_size(sz));
        }
    }
}

/// Helper for constructing shared layout Rcs.
pub fn rc_layout<L: Layout>(l: L) -> Rc<dyn Layout> {
    Rc::new(l)
}
