// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The layout engine (DESIGN.md §7): parent-proposes/child-chooses with a proposal-keyed
//! measure cache. Layout impls are ours or user-provided; they never run reactive user code,
//! so the engine holds the single tree borrow for the whole pass.

use std::rc::Rc;

day_reactive::tls_slots! {
    layout;
    /// Nodes already warned about (see the report site below): once per node, not per
    /// frame. A rebuilt page gets a fresh node and may report again — fine.
    static REPORTED: std::cell::RefCell<std::collections::HashSet<RNode>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
}

use day_spec::*;

use crate::tree::{Flex, RNode, Tree, TreeOps};

/// Open layout protocol (§7.2). `children` are the node's direct children; group nodes
/// (`when`/`each` anchors) are layout-transparent — stacks expand them inline.
pub trait Layout: 'static {
    fn measure(&self, cx: &mut dyn LayoutOps, children: &[RNode], p: Proposal) -> Size;
    fn place(&self, cx: &mut dyn LayoutOps, children: &[RNode], bounds: Rect);
    /// Where this node's own first text baseline falls, measured from the top of a box of
    /// `size` (docs/baseline.md). `None` — the default — means the node has no baseline to
    /// offer, so a baseline-aligned parent falls back to box alignment for it.
    ///
    /// A container answers on behalf of its content: a column reports its first
    /// baseline-bearing child's, shifted by where that child sits. Leaves never reach here —
    /// [`LayoutOps::baseline_of`] asks the toolkit for those.
    fn baseline(&self, _cx: &mut dyn LayoutOps, _children: &[RNode], _size: Size) -> Option<f64> {
        None
    }
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
    /// A child's first text baseline at `size`, from the top of its box (docs/baseline.md):
    /// the toolkit answers for a native leaf, the child's own [`Layout::baseline`] for a
    /// container. `None` ⇒ no baseline to align to.
    fn baseline_of(&mut self, child: RNode, size: Size) -> Option<f64>;
    /// Report scroll content size for the CURRENT node (§7.6).
    fn set_scroll_content(&mut self, content: Size);
    /// Report that the CURRENT node's children outgrew the bounds its `place()` was given
    /// (`needed` vs `available` main-axis points). A diagnostic seam, not a layout input:
    /// the engine logs it once per node in debug builds and ignores it in release, and
    /// placement proceeds unchanged either way. Scroll containers never report — content
    /// larger than the viewport is their normal state.
    fn report_overflow(&mut self, _needed: f64, _available: f64) {}
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
    fn baseline_of(&mut self, child: RNode, size: Size) -> Option<f64> {
        baseline_node(self.tree, child, size)
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

    #[cfg(debug_assertions)]
    fn report_overflow(&mut self, needed: f64, available: f64) {
        // Once per node, not per frame: layout re-runs constantly, and the point is a greppable
        // hint, not a firehose. A rebuilt page gets a fresh node and may report again — fine.
        if REPORTED.with(|r| !r.borrow_mut().insert(self.current)) {
            return;
        }
        // Name the container by the dayscript ids in reach, because an id is the one thing a
        // developer can grep straight to the source. Ids usually sit a wrapper or two below
        // the overflowing stack (`.id` tags the piece, decorators wrap it), so this walks the
        // subtree depth-first and keeps the first few it meets.
        let mut ids: Vec<String> = Vec::new();
        let mut stack = vec![self.current];
        while let Some(node) = stack.pop() {
            if ids.len() >= 6 {
                break;
            }
            if let Some(n) = self.tree.node(node) {
                ids.extend(n.id.clone());
                stack.extend(n.children.iter().rev());
            }
        }
        let who = if ids.is_empty() {
            String::from("no .id in reach")
        } else {
            ids.join(", ")
        };
        log::warn!(
            "day layout: children overflow their container by {:.0}pt ({needed:.0} needed, \
             {available:.0} available; {who}) — give the row a fit policy: \
             .fit(RowFit::Wrap/ColumnAt/Scroll) (docs/size-classes.md)",
            needed - available,
        );
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

/// A node's first text baseline at `size`, from the top of its box (docs/baseline.md). A native
/// leaf asks the toolkit; anything else asks its own `Layout`, which answers for its content.
///
/// Cached per size and invalidated with the measure cache, so a baseline-aligned row costs one
/// toolkit call per child per measure generation rather than one per layout pass.
pub(crate) fn baseline_node<B: Toolkit>(
    tree: &mut Tree<B>,
    node: RNode,
    size: Size,
) -> Option<f64> {
    let key = Proposal::exact(size).cache_key();
    let (layout, children, handle, kind) = {
        let n = tree.node(node)?;
        if !n.needs_measure
            && let Some((k, b)) = n.baseline_cache
            && k == key
        {
            return b;
        }
        (
            n.layout.clone(),
            n.children.clone(),
            n.handle.clone(),
            n.kind,
        )
    };
    // A realized leaf's baseline is the toolkit's to report; a container's is its layout's.
    let baseline = match handle {
        Some(h) if children.is_empty() => tree.toolkit.first_baseline(&h, kind, size),
        _ => {
            let mut cx = EngineCx {
                tree,
                offset: Point::ZERO,
                current: node,
                parent_size: Size::ZERO,
            };
            layout.baseline(&mut cx, &children, size)
        }
    };
    if let Some(n) = tree.node_mut(node) {
        n.baseline_cache = Some((key, baseline));
    }
    baseline
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
                    let anim = tree.resolve_anim(node);
                    tree.toolkit.set_frame(&h, abs, anim.as_ref());
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
    // A LIST whose width changes re-lays its bound cells in THIS pass: the native table
    // resizes the physical cell views, but each cell's day content keeps the old width's
    // placement until laid out again (a trailing control would sit clipped after a
    // narrowing). Synchronous on purpose — interactive live-resize runs inside a native
    // tracking loop where deferred main-thread work may not drain until the drag ends.
    let relayout_cells = tree
        .node(node)
        .map(|n| {
            n.kind == day_spec::kinds::LIST
                && n.last_native_frame
                    .map(|f| (f.size.width - abs.size.width).abs() > 0.25)
                    .unwrap_or(false)
        })
        .unwrap_or(false);
    if let Some(n) = tree.node_mut(node) {
        n.last_native_frame = Some(abs);
    }
    if relayout_cells {
        for key in tree.list_cell_keys(node) {
            tree.list_layout_cell(node, key);
        }
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

/// A single-child wrapper's baseline is its child's, moved by where the child sits inside it
/// (docs/baseline.md).
///
/// Every wrapper below places its one child at its own top-left, so `dy` is 0 for all of them
/// but padding. Without this a decorated piece reports NO baseline, and since decorators are
/// invisible in the source — `.width(90)` on a label is still "a label" to the reader — a row
/// would silently center the very children the author asked to align. That is exactly what
/// happened to the showcase's first baseline demo.
fn wrapper_baseline(
    cx: &mut dyn LayoutOps,
    children: &[RNode],
    child_size: Size,
    dy: f64,
) -> Option<f64> {
    let &c = children.first()?;
    Some(cx.baseline_of(c, child_size)? + dy)
}

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

/// A recycled TREE cell's anchor layout (docs/tree.md): row content HUGS its own height, so
/// center it in the cell's fixed row height — [`PassThrough`] pins it to the top, which
/// reads as a misaligned row whenever the content is shorter than the row (a 16pt label in
/// a 28pt row sat 6pt high). Width still fills; a content taller than the row clamps to it.
pub struct CellCenter;

impl Layout for CellCenter {
    fn measure(&self, cx: &mut dyn LayoutOps, children: &[RNode], p: Proposal) -> Size {
        match children.first() {
            Some(&c) => cx.measure_child(c, p),
            None => Size::ZERO,
        }
    }
    fn place(&self, cx: &mut dyn LayoutOps, children: &[RNode], bounds: Rect) {
        if let Some(&c) = children.first() {
            let s = cx.measure_child(c, Proposal::exact(bounds.size));
            let y = ((bounds.size.height - s.height) / 2.0).max(0.0);
            cx.place_child(
                c,
                Rect::new(0.0, y, s.width, s.height.min(bounds.size.height)),
            );
        }
    }
    fn baseline(&self, cx: &mut dyn LayoutOps, children: &[RNode], size: Size) -> Option<f64> {
        PassThrough::forward(cx, children, size)
    }
}

impl PassThrough {
    /// Shared by every wrapper whose child measures at the wrapper's own size.
    fn forward(cx: &mut dyn LayoutOps, children: &[RNode], size: Size) -> Option<f64> {
        let &c = children.first()?;
        let cs = cx.measure_child(c, Proposal::exact(size));
        wrapper_baseline(cx, children, cs, 0.0)
    }
}

/// Paint/clip wrapper (`.background`, `.corner_radius`, the animatable layers): like
/// [`PassThrough`] for measurement, but the GRANTED rect flows to the child verbatim at place
/// time — these containers exist to paint or clip the area the parent granted, so a grow
/// stretch above them must reach the painted surface (the grid-cell card case) instead of
/// being re-hugged at every wrapper. Under a parent that places at measured size — every
/// stack, grid rigid cell, and overlay — bounds equal the measure and nothing changes.
pub struct FillThrough;

impl Layout for FillThrough {
    fn measure(&self, cx: &mut dyn LayoutOps, children: &[RNode], p: Proposal) -> Size {
        match children.first() {
            Some(&c) => cx.measure_child(c, p),
            None => Size::ZERO,
        }
    }
    fn place(&self, cx: &mut dyn LayoutOps, children: &[RNode], bounds: Rect) {
        if let Some(&c) = children.first() {
            cx.place_child(c, Rect::from_size(bounds.size));
        }
    }
    fn baseline(&self, cx: &mut dyn LayoutOps, children: &[RNode], size: Size) -> Option<f64> {
        wrapper_baseline(cx, children, size, 0.0)
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
    /// Align children on their first text baseline rather than on their boxes
    /// (docs/baseline.md). Horizontal rows only — a column's children are stacked along the
    /// axis a baseline lives on, so there is nothing to align. A child whose toolkit reports no
    /// baseline is centered, so a row of text and an image still looks right.
    FirstBaseline,
}

impl CrossAlign {
    /// Placement fraction of the free space: 0 = leading, 0.5 = center, 1 = trailing.
    fn fraction(self) -> f64 {
        match self {
            CrossAlign::Leading => 0.0,
            // No baseline to align to at this level (a column, or every child reported None):
            // centering is the fallback the variant promises.
            CrossAlign::Center | CrossAlign::FirstBaseline => 0.5,
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
        // Shrink pass: when the rigid children's natural sizes OVERFLOW a bounded main axis,
        // re-measure the overflowing ones against the space that is actually left (in order —
        // earlier children keep their natural size first). Content that fits keeps its natural
        // measure, so proposal-expanding kinds (text fields, lists) don't balloon; wrapping
        // content (a capped message bubble, a long label) folds instead of spilling out of the
        // stack.
        if let Some(mp) = main_p {
            let spacing_total = self.spacing * (kids.len().saturating_sub(1)) as f64;
            let available = (mp - spacing_total).max(0.0);
            if rigid_main > available {
                let mut budget = available;
                rigid_main = 0.0;
                for (i, &k) in kids.iter().enumerate() {
                    if self.grows_main(cx.flex_of(k)) {
                        continue;
                    }
                    if self.main(sizes[i]) > budget {
                        sizes[i] = cx.measure_child(k, self.proposal(Some(budget), cross_p));
                    }
                    let m = self.main(sizes[i]);
                    rigid_main += m;
                    budget = (budget - m).max(0.0);
                }
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

    /// Each child's first baseline at the size it settled on, or `None` where it has none
    /// (docs/baseline.md). Only a horizontal row aligns on baselines — down a column the
    /// baselines sit on the stacking axis, where aligning them would pile the children up.
    fn baselines(
        &self,
        cx: &mut dyn LayoutOps,
        kids: &[RNode],
        sizes: &[Size],
    ) -> Vec<Option<f64>> {
        if self.align != CrossAlign::FirstBaseline || self.axis != Axis::Horizontal {
            return vec![None; kids.len()];
        }
        kids.iter()
            .zip(sizes)
            .map(|(&k, &s)| cx.baseline_of(k, s))
            .collect()
    }

    /// The row's own cross-axis extent once its children are baseline-shifted: the deepest
    /// baseline plus the deepest descent below one. A row measured at the tallest child's
    /// height would clip whichever child got shifted furthest down.
    fn baseline_cross_extent(&self, baselines: &[Option<f64>], sizes: &[Size]) -> Option<f64> {
        let deepest = baselines.iter().flatten().copied().fold(f64::MIN, f64::max);
        if deepest == f64::MIN {
            return None;
        }
        let below = baselines
            .iter()
            .zip(sizes)
            .filter_map(|(b, s)| b.map(|b| self.cross(*s) - b))
            .fold(0.0, f64::max);
        Some(deepest + below)
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
        let tallest = sizes.iter().map(|&s| self.cross(s)).fold(0.0, f64::max);
        // Baseline shifting can push a child below the tallest one's bottom edge, so the row
        // takes the extent the shifted children actually occupy (docs/baseline.md).
        let stacked = self
            .baseline_cross_extent(&self.baselines(cx, &kids, &sizes), &sizes)
            .unwrap_or(0.0)
            .max(tallest);
        let cross_total = match cross_p {
            Some(cp) if grows_cross => cp,
            _ => stacked,
        };
        self.mk(main_total, cross_total)
    }

    /// A stack's own baseline is its first baseline-bearing child's, moved by where that child
    /// sits: down a column that is the child's offset along the axis, across a row every
    /// aligned child already shares the deepest baseline (docs/baseline.md).
    fn baseline(&self, cx: &mut dyn LayoutOps, children: &[RNode], size: Size) -> Option<f64> {
        let mut kids = Vec::new();
        Self::flatten(cx, children, &mut kids);
        if kids.is_empty() {
            return None;
        }
        let sizes = self.negotiate(cx, &kids, Proposal::exact(size));
        match self.axis {
            Axis::Horizontal => {
                let baselines: Vec<Option<f64>> = kids
                    .iter()
                    .zip(&sizes)
                    .map(|(&k, &s)| cx.baseline_of(k, s))
                    .collect();
                let deepest = baselines.iter().flatten().copied().fold(f64::MIN, f64::max);
                (deepest != f64::MIN).then_some(deepest)
            }
            Axis::Vertical => {
                let mut pos = 0.0;
                for (&k, &s) in kids.iter().zip(&sizes) {
                    if let Some(b) = cx.baseline_of(k, s) {
                        return Some(pos + b);
                    }
                    pos += self.main(s) + self.spacing;
                }
                None
            }
        }
    }

    fn place(&self, cx: &mut dyn LayoutOps, children: &[RNode], bounds: Rect) {
        let mut kids = Vec::new();
        Self::flatten(cx, children, &mut kids);
        if kids.is_empty() {
            return;
        }
        let sizes = self.negotiate(cx, &kids, Proposal::exact(bounds.size));
        // The children's settled extent against what this stack was actually given: rigid
        // children are free to answer a shrink proposal with their natural size (native
        // buttons and labels routinely do), and then the tail of the stack lands offscreen
        // with every assertion still green. Say so, once, where a developer will see it.
        let needed = sizes.iter().map(|&s| self.main(s)).sum::<f64>()
            + self.spacing * (kids.len() - 1) as f64;
        let available = self.main(bounds.size);
        // A container with NO extent is not overflowing, it is not laid out yet — the state a
        // native-owned surface is in until the backend reports its frame (a `cover` lays its
        // content out once before `Event::FrameChanged` arrives, §7.6/docs/cover.md). Reporting
        // it would name a real developer's ids for a transient the developer cannot act on:
        // no fit policy makes a stack fit in zero points.
        if available > 0.5 && needed > available + 0.5 {
            cx.report_overflow(needed, available);
        }
        let bounds_cross = self.cross(bounds.size);
        // Baseline row (docs/baseline.md): every child that has a baseline is shifted down so
        // they all land on the deepest one, which is the offset that keeps every child inside
        // the row. Children without one keep the centered fallback below.
        let baselines = self.baselines(cx, &kids, &sizes);
        let deepest = baselines.iter().flatten().copied().fold(f64::MIN, f64::max);
        let mut pos = 0.0;
        for (i, &k) in kids.iter().enumerate() {
            let s = sizes[i];
            let cross_off = match (self.align, baselines[i]) {
                (CrossAlign::FirstBaseline, Some(b)) => deepest - b,
                (CrossAlign::Leading, _) => 0.0,
                (CrossAlign::Center | CrossAlign::FirstBaseline, _) => {
                    ((bounds_cross - self.cross(s)) / 2.0).max(0.0)
                }
                (CrossAlign::Trailing, _) => (bounds_cross - self.cross(s)).max(0.0),
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

/// A wrapping row (docs/size-classes.md "Adaptive pieces"): children keep their natural
/// measure and lay out leading-to-trailing, breaking onto a new line where the next child
/// would overflow the proposed width. Wrapping replaces main-axis negotiation, so flex is
/// inert here: nothing grows, and a `spacer()` contributes nothing. RTL mirroring comes from
/// `place_child`, the same as every layout.
pub struct FlowLayout {
    pub spacing: f64,
    /// Vertical gap between lines.
    pub run_spacing: f64,
    /// How children align within their own line (a line is as tall as its tallest child).
    pub align: CrossAlign,
    /// Uniform columns ([`RowFit::WrapColumns`]): every cell takes the widest child's width and
    /// each line holds as many as the available width fits, so the wrapped lines align into
    /// columns. `false` packs each line at natural widths, which comes out ragged.
    pub uniform: bool,
}

/// One packed flow line: the child range it holds and its settled extent.
struct FlowLine {
    start: usize,
    end: usize,
    height: f64,
}

/// A solved flow: the size each child is placed at, and the lines they pack into. Measure and
/// place both go through this, so the two never disagree about where a line breaks.
struct FlowPlan {
    sizes: Vec<Size>,
    lines: Vec<FlowLine>,
}

impl FlowLayout {
    /// Pack natural sizes into lines under `max_w`. Unbounded → one line, the degenerate case
    /// that keeps a flow inside a horizontal scroll behaving like a plain row.
    fn pack(&self, sizes: &[Size], max_w: Option<f64>) -> Vec<FlowLine> {
        let mut lines = Vec::new();
        let mut start = 0;
        let mut w = 0.0;
        let mut h: f64 = 0.0;
        for (i, s) in sizes.iter().enumerate() {
            let grown = if i == start {
                s.width
            } else {
                w + self.spacing + s.width
            };
            if i > start && max_w.is_some_and(|m| grown > m + 0.5) {
                lines.push(FlowLine {
                    start,
                    end: i,
                    height: h,
                });
                start = i;
                w = s.width;
                h = s.height;
            } else {
                w = grown;
                h = h.max(s.height);
            }
        }
        if start < sizes.len() {
            lines.push(FlowLine {
                start,
                end: sizes.len(),
                height: h,
            });
        }
        lines
    }

    fn natural_sizes(cx: &mut dyn LayoutOps, kids: &[RNode]) -> Vec<Size> {
        kids.iter()
            .map(|&k| cx.measure_child(k, Proposal::UNCONSTRAINED))
            .collect()
    }

    /// Solve the flow for an available width: ragged lines at natural widths, or — under
    /// `uniform` — one column width for every cell with as many columns per line as fit.
    fn plan(&self, cx: &mut dyn LayoutOps, kids: &[RNode], max_w: Option<f64>) -> FlowPlan {
        let natural = Self::natural_sizes(cx, kids);
        if !self.uniform {
            let lines = self.pack(&natural, max_w);
            return FlowPlan {
                sizes: natural,
                lines,
            };
        }
        // The widest child sets the column, so no cell is ever narrower than its content.
        let cell_w = natural.iter().map(|s| s.width).fold(0.0, f64::max);
        let cols = match max_w {
            // How many whole columns fit, counting the gutter between them; never zero, or a
            // window narrower than one cell would produce no lines at all.
            Some(m) if cell_w > 0.0 => {
                (((m + self.spacing) / (cell_w + self.spacing)).floor() as usize).max(1)
            }
            // Unbounded (inside a horizontal scroll): one line, like a plain row.
            _ => kids.len().max(1),
        };
        // Re-measure at the column width: a child handed more width than it asked for can
        // settle shorter (a label unwrapping), and the line's height follows what it settles
        // on rather than what it wanted.
        let sizes: Vec<Size> = kids
            .iter()
            .map(|&k| {
                let s = cx.measure_child(k, Proposal::new(Some(cell_w), None));
                Size::new(cell_w, s.height)
            })
            .collect();
        let mut lines = Vec::new();
        let mut start = 0;
        while start < sizes.len() {
            let end = (start + cols).min(sizes.len());
            let height = sizes[start..end]
                .iter()
                .map(|s| s.height)
                .fold(0.0, f64::max);
            lines.push(FlowLine { start, end, height });
            start = end;
        }
        FlowPlan { sizes, lines }
    }
}

impl Layout for FlowLayout {
    fn measure(&self, cx: &mut dyn LayoutOps, children: &[RNode], p: Proposal) -> Size {
        let mut kids = Vec::new();
        StackLayout::flatten(cx, children, &mut kids);
        if kids.is_empty() {
            return Size::ZERO;
        }
        let plan = self.plan(cx, &kids, p.width);
        let height = plan.lines.iter().map(|l| l.height).sum::<f64>()
            + self.run_spacing * plan.lines.len().saturating_sub(1) as f64;
        // Bounded → fill the proposal (lines pack leading inside it, like wrapped text);
        // unbounded → the single packed line's own extent.
        let width = p.width.unwrap_or_else(|| {
            plan.sizes.iter().map(|s| s.width).sum::<f64>()
                + self.spacing * plan.sizes.len().saturating_sub(1) as f64
        });
        Size::new(width, height)
    }

    fn place(&self, cx: &mut dyn LayoutOps, children: &[RNode], bounds: Rect) {
        let mut kids = Vec::new();
        StackLayout::flatten(cx, children, &mut kids);
        if kids.is_empty() {
            return;
        }
        let plan = self.plan(cx, &kids, Some(bounds.size.width));
        let mut y = 0.0;
        for line in &plan.lines {
            let mut x = 0.0;
            let range = line.start..line.end;
            for (&k, &s) in kids[range.clone()].iter().zip(&plan.sizes[range]) {
                let dy = ((line.height - s.height) * self.align.fraction()).max(0.0);
                cx.place_child(k, Rect::new(x, y + dy, s.width, s.height));
                x += s.width + self.spacing;
            }
            y += line.height + self.run_spacing;
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
    fn baseline(&self, cx: &mut dyn LayoutOps, children: &[RNode], size: Size) -> Option<f64> {
        // The one wrapper with a real offset: its child starts below the top inset.
        let inner = Size::new(
            (size.width - self.insets.horizontal()).max(0.0),
            (size.height - self.insets.vertical()).max(0.0),
        );
        wrapper_baseline(cx, children, inner, self.insets.top)
    }
}

/// The `aspect_ratio` decorator (§5.2): the largest `width / height == ratio` box that fits what
/// the parent proposes, with the child given that whole box.
///
/// With ONE axis proposed it derives the other, which is what makes `.grow_w().aspect_ratio(r)`
/// take the width a container offers and compute the height from it — a canvas that keeps its
/// proportions as the window resizes.
pub struct AspectRatioLayout {
    pub ratio: f64,
}

impl Layout for AspectRatioLayout {
    fn measure(&self, cx: &mut dyn LayoutOps, _children: &[RNode], p: Proposal) -> Size {
        match (p.width, p.height) {
            (Some(w), Some(h)) => {
                if w / h > self.ratio {
                    Size::new(h * self.ratio, h)
                } else {
                    Size::new(w, w / self.ratio)
                }
            }
            (Some(w), None) => Size::new(w, w / self.ratio),
            (None, Some(h)) => Size::new(h * self.ratio, h),
            (None, None) => cx.measure_leaf(p),
        }
    }
    fn place(&self, cx: &mut dyn LayoutOps, children: &[RNode], bounds: Rect) {
        // The ratio has already decided the box; hand all of it to the child.
        for &c in children {
            cx.place_child(c, Rect::from_size(bounds.size));
        }
    }
    fn baseline(&self, cx: &mut dyn LayoutOps, children: &[RNode], size: Size) -> Option<f64> {
        PassThrough::forward(cx, children, size)
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
    fn baseline(&self, cx: &mut dyn LayoutOps, children: &[RNode], size: Size) -> Option<f64> {
        PassThrough::forward(cx, children, size)
    }
}

/// The `.max_width(w)` decorator (docs/layout.md): proposes at most `w` to the child, so
/// text wraps instead of overflowing, while narrower content still hugs. The vertical axis
/// passes through untouched.
pub struct MaxWidthLayout {
    pub max: f64,
}

impl Layout for MaxWidthLayout {
    fn measure(&self, cx: &mut dyn LayoutOps, children: &[RNode], p: Proposal) -> Size {
        let capped = Proposal::new(
            Some(p.width.map(|w| w.min(self.max)).unwrap_or(self.max)),
            p.height,
        );
        match children.first() {
            Some(&c) => {
                let s = cx.measure_child(c, capped);
                Size::new(s.width.min(self.max), s.height)
            }
            None => Size::ZERO,
        }
    }
    fn place(&self, cx: &mut dyn LayoutOps, children: &[RNode], bounds: Rect) {
        if let Some(&c) = children.first() {
            let w = bounds.size.width.min(self.max);
            let s = cx.measure_child(c, Proposal::exact(Size::new(w, bounds.size.height)));
            cx.place_child(c, Rect::from_size(Size::new(s.width.min(w), s.height)));
        }
    }
    fn baseline(&self, cx: &mut dyn LayoutOps, children: &[RNode], size: Size) -> Option<f64> {
        PassThrough::forward(cx, children, size)
    }
}

/// Reserve at least the size of a SAMPLE child, then lay the content out inside it.
///
/// The problem it solves: a numeric readout beside a slider changes width as the value changes —
/// `1` is narrower than `8` in a proportional font, and `9` → `10` adds a whole glyph — so the
/// row reflows on every drag and the control you are aiming at moves. A hardcoded width "fixes"
/// that until the reader raises their accessibility text size, at which point the number is
/// clipped.
///
/// Reserving a sample measures a real piece in the real font at the real size, so the reservation
/// scales with the text exactly as the content does. `children[0]` is the sample (built
/// transparent by [`Decorate::reserving`], so it never paints and is placed at zero size);
/// `children[1]` is the content, which receives the full reserved bounds and aligns itself.
pub struct ReserveLayout;

impl Layout for ReserveLayout {
    fn measure(&self, cx: &mut dyn LayoutOps, children: &[RNode], p: Proposal) -> Size {
        let (Some(&sample), Some(&content)) = (children.first(), children.get(1)) else {
            return children
                .first()
                .map(|&c| cx.measure_child(c, p))
                .unwrap_or(Size::ZERO);
        };
        let s = cx.measure_child(sample, p);
        let c = cx.measure_child(content, p);
        Size::new(s.width.max(c.width), s.height.max(c.height))
    }

    fn place(&self, cx: &mut dyn LayoutOps, children: &[RNode], bounds: Rect) {
        // The sample has done its work in `measure`; give it no area so it costs nothing to draw.
        if let Some(&sample) = children.first() {
            cx.place_child(sample, Rect::from_size(Size::ZERO));
        }
        if let Some(&content) = children.get(1) {
            let s = cx.measure_child(content, Proposal::exact(bounds.size));
            let _ = s;
            cx.place_child(content, Rect::from_size(bounds.size));
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
    fn baseline(&self, cx: &mut dyn LayoutOps, children: &[RNode], size: Size) -> Option<f64> {
        PassThrough::forward(cx, children, size)
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
    /// The presentation as last resolved. Shared with the piece rather than copied, so a
    /// re-present (`NavPatch::Presentation`) changes the fallback split without rebuilding the
    /// layout object the host was realized with.
    pub presentation: std::rc::Rc<std::cell::Cell<day_spec::props::NavPresentation>>,
    /// The host's sidebar page, once it has one. Identity, not position: after a re-present the
    /// pages keep their roles but a backend may have re-homed them in a different order.
    pub sidebar: std::rc::Rc<std::cell::Cell<Option<RNode>>>,
    /// The host's content-list page (`Pane::List`), once it has one — shared like `sidebar`.
    pub list: std::rc::Rc<std::cell::Cell<Option<RNode>>>,
    /// The content-list pane's preferred width (`NavProps::list_width`), for the fallback split.
    pub list_width: f64,
    /// Whether the pane is showing (`NavPatch::ListVisible`) — shared with the piece so a
    /// collapsed pane stops narrowing the detail's FALLBACK immediately, rather than one
    /// FrameChanged report later.
    pub list_visible: std::rc::Rc<std::cell::Cell<bool>>,
}

pub use day_spec::{NAV_LIST_WIDTH, NAV_SIDEBAR_WIDTH};

impl NavLayout {
    /// A host with no sidebar pane and no re-presenting to do: a tab strip's page area, or a
    /// `stack` piece, both of which stay stacked at every size.
    pub fn stack(
        sizes: std::rc::Rc<std::cell::RefCell<std::collections::HashMap<RNode, Size>>>,
    ) -> Self {
        NavLayout {
            sizes,
            presentation: std::rc::Rc::new(std::cell::Cell::new(
                day_spec::props::NavPresentation::Stack,
            )),
            sidebar: std::rc::Rc::new(std::cell::Cell::new(None)),
            list: std::rc::Rc::new(std::cell::Cell::new(None)),
            list_width: NAV_LIST_WIDTH,
            list_visible: std::rc::Rc::new(std::cell::Cell::new(true)),
        }
    }
}

impl Layout for NavLayout {
    fn measure(&self, _cx: &mut dyn LayoutOps, _children: &[RNode], p: Proposal) -> Size {
        // Greedy: the host owns the window. A nested stack merges into the enclosing host's page
        // list rather than nesting a second host (docs/navigation.md), so one NAV host still
        // spans the window.
        Size::new(p.width.unwrap_or(480.0), p.height.unwrap_or(640.0))
    }
    fn place(&self, cx: &mut dyn LayoutOps, children: &[RNode], bounds: Rect) {
        let pres = self.presentation.get();
        let split = pres.is_split();
        let sidebar = self.sidebar.get();
        let list = self.list.get();
        // The list pane narrows the detail's fallback only while it has its own pane AND it
        // is showing — a collapsed pane gives the detail its width back at once.
        let list_w = if split && list.is_some() && self.list_visible.get() {
            self.list_width
        } else {
            0.0
        };
        for &page in children {
            let reported = self.sizes.borrow().get(&page).copied();
            // Only a FALLBACK: every backend reports each page's real frame through
            // `Event::FrameChanged`, and that wins. This is what the page gets for the frame
            // before the first report arrives.
            let sz = reported.unwrap_or_else(|| {
                let is_sidebar = Some(page) == sidebar;
                let is_list = Some(page) == list;
                if pres.rows_are_chrome() {
                    // The rows are the chrome (a tab bar, a rail): the backend draws them itself
                    // and sizes its own bar, so the sidebar page is measured but not shown. Keep
                    // it at the pane width rather than zero — a zero-width menu measured here
                    // would have to re-measure from scratch the moment the window widens back
                    // into a split.
                    if is_sidebar {
                        Size::new(NAV_SIDEBAR_WIDTH, bounds.size.height)
                    } else if is_list {
                        Size::new(self.list_width, bounds.size.height)
                    } else {
                        bounds.size
                    }
                } else if !split {
                    // Includes a stacked host's list page: full width where it joins the stack
                    // (`Cap::NavContentList` Emulated); where its pane persists instead
                    // (Native, a narrow Mac window) the first FrameChanged report corrects it.
                    bounds.size
                } else if is_sidebar {
                    Size::new(NAV_SIDEBAR_WIDTH, bounds.size.height)
                } else if is_list {
                    Size::new(self.list_width, bounds.size.height)
                } else {
                    Size::new(
                        (bounds.size.width - NAV_SIDEBAR_WIDTH - list_w - 1.0).max(0.0),
                        bounds.size.height,
                    )
                }
            });
            cx.place_child_native(page, Rect::from_size(sz));
        }
    }
}

/// Inspector split (docs/inspector.md): the two pane frames are native-owned (an
/// `NSSplitView`'s panes, a dock widget), so Day lays each pane's content within the size the
/// toolkit last reported via `Event::FrameChanged` — the same native-owned-frame contract as
/// [`NavLayout`] pages. The fallback before the first report splits the host bounds at the
/// pane width when visible, and gives the content everything when hidden.
pub struct InspectorLayout {
    pub sizes: std::rc::Rc<std::cell::RefCell<std::collections::HashMap<RNode, Size>>>,
    /// Visibility as last patched — shared with the piece so a toggle changes the fallback
    /// split without rebuilding the layout object the host was realized with.
    pub visible: std::rc::Rc<std::cell::Cell<bool>>,
    /// The pane's preferred width (`InspectorProps::width`), for the fallback split.
    pub width: f64,
}

impl Layout for InspectorLayout {
    fn measure(&self, _cx: &mut dyn LayoutOps, _children: &[RNode], p: Proposal) -> Size {
        // Greedy, like a nav host: the split owns whatever its parent proposes.
        Size::new(p.width.unwrap_or(480.0), p.height.unwrap_or(640.0))
    }
    fn place(&self, cx: &mut dyn LayoutOps, children: &[RNode], bounds: Rect) {
        // Insertion order is the contract (day-spec `Builtin::Inspector`): 0 content, 1 panel.
        for (i, &pane) in children.iter().enumerate() {
            let reported = self.sizes.borrow().get(&pane).copied();
            let sz = reported.unwrap_or_else(|| {
                let panel_w = if self.visible.get() { self.width } else { 0.0 };
                if i == 0 {
                    Size::new((bounds.size.width - panel_w).max(0.0), bounds.size.height)
                } else {
                    Size::new(panel_w, bounds.size.height)
                }
            });
            cx.place_child_native(pane, Rect::from_size(sz));
        }
    }
}

/// Fullscreen cover (docs/cover.md): the COVER node occupies no space where it sits in the
/// tree (its native surface is presented over the window, outside the parent's bounds), and
/// its content is laid out at the size the backend reported via `Event::FrameChanged` — the
/// same native-owned-frame contract as [`NavLayout`] pages.
pub struct CoverLayout {
    pub size: std::rc::Rc<std::cell::RefCell<Option<Size>>>,
}

impl Layout for CoverLayout {
    fn measure(&self, _cx: &mut dyn LayoutOps, _children: &[RNode], _p: Proposal) -> Size {
        Size::new(0.0, 0.0)
    }
    fn place(&self, cx: &mut dyn LayoutOps, children: &[RNode], _bounds: Rect) {
        let Some(sz) = *self.size.borrow() else {
            return; // not presented yet — content lays out on the first FrameChanged
        };
        for &child in children {
            cx.place_child_native(child, Rect::from_size(sz));
        }
    }
}

/// Helper for constructing shared layout Rcs.
pub fn rc_layout<L: Layout>(l: L) -> Rc<dyn Layout> {
    Rc::new(l)
}
