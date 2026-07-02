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
}

impl<B: Toolkit> LayoutOps for EngineCx<'_, B> {
    fn measure_child(&mut self, child: RNode, p: Proposal) -> Size {
        measure_node(self.tree, child, p)
    }
    fn place_child(&mut self, child: RNode, rect: Rect) {
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
        let Some(n) = self.tree.node(self.current) else {
            return;
        };
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

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum CrossAlign {
    Leading,
    #[default]
    Center,
    Trailing,
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

/// Helper for constructing shared layout Rcs.
pub fn rc_layout<L: Layout>(l: L) -> Rc<dyn Layout> {
    Rc::new(l)
}
