// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The realized tree: nodes own native handles (or are layout-only), a reactive scope, and
//! layout state. One `Tree<B>` per process, installed thread-local; bindings and event
//! handlers reach it through [`with_tree`] — and tree methods NEVER run user code, so the
//! single-borrow discipline holds (§3.3, §8.3).

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

use day_reactive::Scope;
use day_spec::*;
use slotmap::{Key, KeyData, SlotMap, new_key_type};

use crate::layout::{Alignment, CrossAlign, Layout, PassThrough};

new_key_type! {
    /// Realized-node key. `NodeId` (the spec-boundary id) is its FFI encoding.
    pub struct RNode;
}

pub fn rnode_to_id(n: RNode) -> NodeId {
    NodeId(n.data().as_ffi())
}
pub fn id_to_rnode(id: NodeId) -> RNode {
    RNode::from(KeyData::from_ffi(id.0))
}

/// Read-only layout facts a parent may consult about a child (§7.2 ChildRef).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Flex {
    /// Wants to fill the horizontal / vertical axis when offered space.
    pub grow_w: bool,
    pub grow_h: bool,
    /// Takes all remaining main-axis space in a stack.
    pub is_spacer: bool,
    /// Layout-transparent group (`when`/`each` anchors): stacks lay out its children inline.
    pub is_group: bool,
    /// Grid facts (docs/grid.md): consulted only by `GridLayout`; inert everywhere else.
    pub grid: GridFacts,
}

/// Per-node grid facts (docs/grid.md), carried on [`Flex`] — the shipped form of the §7.2
/// ChildRef facts surface. Set at build time by `grid_row` and the `.grid_span`/`.grid_align`
/// modifiers; only `GridLayout` reads them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GridFacts {
    /// The node is a `grid_row` carrier: its children are that row's cells.
    pub is_row: bool,
    /// Row-only: vertical alignment override for the row's cells.
    pub row_valign: Option<CrossAlign>,
    /// Cell-only: columns spanned (0 = unset = 1).
    pub col_span: u16,
    /// Cell-only: alignment override within the cell rect.
    pub align: Option<Alignment>,
}

/// Cached last-applied props for the dayscript element index (§14.2).
#[derive(Clone, Debug, Default)]
pub struct NodeProbe {
    pub text: String,
    pub value: f64,
    pub flag: bool,
    pub selected: i64,
    pub enabled: bool,
    /// Native keyboard focus, mirrored from `Event::FocusChanged` (docs/focus.md).
    pub focused: bool,
}

pub struct NodeData<H> {
    pub kind: PieceKind,
    pub handle: Option<H>,
    pub parent: RNode,
    pub children: Vec<RNode>,
    pub layout: Rc<dyn Layout>,
    pub flex: Flex,
    pub scope: Scope,
    pub id: Option<String>,
    /// Accumulated accessibility annotations (§13): merged from the piece default, `.a11y()`,
    /// and `.id()`. Stored so each `set_a11y` re-applies the full picture and `a11y_audit`
    /// (§14.2) can diff the native tree against Day's own expectation.
    pub a11y: day_spec::A11yProps,
    // --- layout state (§7.4) ---
    pub cache: Vec<((u64, u64), Size)>,
    /// The last `(size, first baseline)` this node answered (docs/baseline.md). One slot, not a
    /// list: a baseline is asked for at the size the row already settled on, so the query
    /// repeats at one size per pass. Invalidated with `needs_measure`, alongside `cache`.
    pub baseline_cache: Option<((u64, u64), Option<f64>)>,
    pub probe: NodeProbe,
    pub needs_measure: bool,
    pub last_native_frame: Option<Rect>,
    pub is_boundary: bool,
    /// Scroll-content size reported by `ScrollLayout` (§7.6) — SCROLL nodes only. Cached so
    /// `scroll_to_target` can compose edge targets (bottom = content minus viewport).
    pub scroll_content: Option<Size>,
    /// Node-scoped implicit animation (`.animation(anim)`, §8.4): when set, this node's property
    /// patches and frame changes animate even outside a `with_animation`. The ambient animation
    /// (`with_animation`) takes precedence when both are present.
    pub implicit_anim: Option<day_spec::AnimSpec>,
    /// A `.tweak` closure (or a per-toolkit ext modifier, which routes through it) ran against
    /// this node's handle (docs/tweaks.md). Read by `set_node_selectable`: a backing swap would
    /// discard that work, which warrants a loud warning rather than silent loss.
    pub tweaked: bool,
}

/// An event handler registered on a realized node.
pub type EventHandler = Rc<dyn Fn(&Event)>;

/// The recording/telemetry observer installed via [`set_event_observer`] (§14.6): called with
/// every dispatched `(NodeId, Event)`, in queue order, before the app receives it.
pub type EventObserver = dyn Fn(NodeId, &Event);

/// `TreeOps::open_window_root`'s answer — the tree-level face of
/// [`day_spec::WindowOpenReply`], carrying the adopted (or parked) root node.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WindowRootReply {
    /// The native window exists; `root` is its adopted boundary root, laid out already.
    Open(RNode),
    /// Native creation is in flight; `root` is parked (no handle, no layout) until
    /// `TreeOps::adopt_window_root` completes it.
    Pending(RNode),
    /// The toolkit cannot open windows — fall back to the cover tier.
    Unsupported,
}

/// One realized window: an adopted boundary root plus the content size it lays out at.
///
/// `windows[0]` starts as the app's first window (the `ready` root container) and later entries
/// are opened through `open_window_root` (docs/windows.md) — but the first slot is not
/// privileged: it can close like any other, after which `windows[0]` is simply the oldest
/// window still open, which is what `root()` then answers. The list is never emptied; see
/// `remove_window_root`.
struct WindowEntry {
    root: RNode,
    size: Size,
}

pub struct Tree<B: Toolkit> {
    pub toolkit: B,
    nodes: SlotMap<RNode, NodeData<B::Handle>>,
    windows: Vec<WindowEntry>,
    layout_dirty: bool,
    handlers: HashMap<RNode, Vec<EventHandler>>,
    // (kind, handle): the kind rides along so the drain can offer a satellite piece its
    // `release` hook before the backend frees the handle (§15.2).
    release_queue: Vec<(day_spec::PieceKind, B::Handle)>,
    /// Recycling-list state keyed by LIST node (docs/list.md, §10).
    lists: HashMap<RNode, crate::list::ListState>,
    /// Count of nodes carrying an implicit `.animation` (§8.4). Gates the `resolve_anim` ancestor
    /// walk: zero ⇒ non-`with_animation` patches skip it entirely (the common, O(1) path).
    implicit_anim_count: usize,
}

impl<B: Toolkit> Tree<B> {
    pub fn new(toolkit: B, root_handle: B::Handle, window_size: Size) -> Self {
        let mut nodes = SlotMap::with_key();
        let root = nodes.insert(NodeData {
            kind: kinds::CONTAINER,
            handle: Some(root_handle),
            parent: RNode::null(),
            children: Vec::new(),
            layout: Rc::new(PassThrough),
            flex: Flex::default(),
            scope: Scope::root(),
            id: None,
            a11y: Default::default(),
            cache: Vec::new(),
            baseline_cache: None,
            probe: NodeProbe::default(),
            needs_measure: true,
            last_native_frame: None,
            scroll_content: None,
            implicit_anim: None,
            tweaked: false,
            is_boundary: true,
        });
        Tree {
            toolkit,
            nodes,
            windows: vec![WindowEntry {
                root,
                size: window_size,
            }],
            layout_dirty: true,
            handlers: HashMap::new(),
            release_queue: Vec::new(),
            lists: HashMap::new(),
            implicit_anim_count: 0,
        }
    }

    /// Create a node whose native handle is a foreign cell adopted from a recycling list host —
    /// the same "wrap an externally-owned handle" trick the window root uses (docs/list.md).
    pub(crate) fn create_cell_anchor(&mut self, handle: B::Handle, scope: Scope) -> RNode {
        self.nodes.insert(NodeData {
            kind: kinds::LIST_CELL,
            handle: Some(handle),
            parent: RNode::null(),
            children: Vec::new(),
            layout: Rc::new(PassThrough),
            flex: Flex::default(),
            scope,
            id: None,
            a11y: Default::default(),
            cache: Vec::new(),
            baseline_cache: None,
            probe: NodeProbe::default(),
            needs_measure: true,
            last_native_frame: None,
            scroll_content: None,
            implicit_anim: None,
            tweaked: false,
            is_boundary: true,
        })
    }

    pub fn root(&self) -> RNode {
        self.windows[0].root
    }

    pub(crate) fn node(&self, n: RNode) -> Option<&NodeData<B::Handle>> {
        self.nodes.get(n)
    }
    pub(crate) fn node_mut(&mut self, n: RNode) -> Option<&mut NodeData<B::Handle>> {
        self.nodes.get_mut(n)
    }

    /// The animation intent for a change to `node` (§8.4): the ambient `with_animation` if one is
    /// in scope, else the nearest `.animation` on `node` or an ancestor (SwiftUI-style
    /// propagation). `None` ⇒ the change applies instantly. The ancestor walk is skipped entirely
    /// when no implicit animations exist anywhere (`implicit_anim_count == 0`).
    pub(crate) fn resolve_anim(&self, node: RNode) -> Option<day_spec::AnimSpec> {
        if let Some(a) = crate::anim::current_anim() {
            return Some(a);
        }
        if self.implicit_anim_count == 0 {
            return None;
        }
        let mut cur = node;
        loop {
            let n = self.nodes.get(cur)?;
            if n.implicit_anim.is_some() {
                return n.implicit_anim;
            }
            if n.parent == cur {
                return None;
            }
            cur = n.parent;
        }
    }

    /// Nearest ancestor (or self) with a native handle.
    fn native_ancestor(&self, mut n: RNode) -> RNode {
        loop {
            let Some(node) = self.nodes.get(n) else {
                // Stale node: fall back to the primary window's root (benign — the caller
                // is about to no-op against it).
                return self.windows[0].root;
            };
            if node.handle.is_some() {
                return n;
            }
            n = node.parent;
        }
    }

    /// In-order native descendants of `container` (its native children, not descending into them).
    fn native_descendants(&self, container: RNode, out: &mut Vec<RNode>) {
        let Some(node) = self.nodes.get(container) else {
            return;
        };
        for &c in &node.children {
            match self.nodes.get(c) {
                Some(cd) if cd.handle.is_some() => out.push(c),
                Some(_) => self.native_descendants(c, out),
                None => {}
            }
        }
    }

    /// Index that `child`'s first native node occupies (or will occupy) among `ancestor`'s
    /// native children — an in-order walk counting native roots before reaching `child`'s subtree.
    fn native_index_for(&self, ancestor: RNode, target: RNode) -> usize {
        fn walk<B: Toolkit>(tree: &Tree<B>, n: RNode, target: RNode, count: &mut usize) -> bool {
            if n == target {
                return true;
            }
            let Some(node) = tree.nodes.get(n) else {
                return false;
            };
            if node.handle.is_some() && n != target {
                // A native node counts as one slot; do not descend (its children are inside it).
                *count += 1;
                return false;
            }
            for &c in &node.children {
                if walk(tree, c, target, count) {
                    return true;
                }
            }
            false
        }
        let mut count = 0;
        let Some(anc) = self.nodes.get(ancestor) else {
            return 0;
        };
        for &c in &anc.children {
            if c == target || self.subtree_contains(c, target) {
                // Count native roots in this subtree BEFORE target.
                let mut cnt = count;
                walk(self, c, target, &mut cnt);
                return cnt;
            }
            let mut roots = Vec::new();
            match self.nodes.get(c) {
                Some(cd) if cd.handle.is_some() => count += 1,
                Some(_) => {
                    self.native_descendants(c, &mut roots);
                    count += roots.len();
                }
                None => {}
            }
        }
        count
    }

    fn subtree_contains(&self, root: RNode, target: RNode) -> bool {
        if root == target {
            return true;
        }
        let Some(node) = self.nodes.get(root) else {
            return false;
        };
        node.children
            .iter()
            .any(|&c| self.subtree_contains(c, target))
    }

    /// Attach `child` under `parent` at child-list `index`, wiring native insertion.
    fn attach_impl(&mut self, parent: RNode, child: RNode, index: usize) {
        {
            let p = self
                .nodes
                .get_mut(parent)
                .expect("attach to missing parent");
            let idx = index.min(p.children.len());
            p.children.insert(idx, child);
        }
        self.nodes
            .get_mut(child)
            .expect("attach missing child")
            .parent = parent;
        // Native wiring: every native root inside `child`'s subtree inserts under the nearest
        // native ancestor at its in-order position.
        let ancestor = self.native_ancestor(parent);
        let anc_handle = self.nodes[ancestor]
            .handle
            .clone()
            .expect("native ancestor");
        let mut roots = Vec::new();
        match self.nodes.get(child) {
            Some(cd) if cd.handle.is_some() => roots.push(child),
            Some(_) => self.native_descendants(child, &mut roots),
            None => {}
        }
        for r in roots {
            let idx = self.native_index_for(ancestor, r);
            let h = self.nodes[r].handle.clone().unwrap();
            self.toolkit.insert(&anc_handle, &h, idx);
        }
        self.mark_needs_measure_impl(parent);
    }

    fn remove_subtree_impl(&mut self, node: RNode) {
        // Detach native roots from their native ancestor, queue every handle for release,
        // drop handler entries, then remove the node records.
        let parent = self
            .nodes
            .get(node)
            .map(|n| n.parent)
            .unwrap_or(RNode::null());
        if let Some(p) = self.nodes.get_mut(parent) {
            p.children.retain(|&c| c != node);
        }
        let ancestor = self.native_ancestor(parent);
        let anc_handle = self.nodes.get(ancestor).and_then(|n| n.handle.clone());
        let mut roots = Vec::new();
        match self.nodes.get(node) {
            Some(nd) if nd.handle.is_some() => roots.push(node),
            Some(_) => self.native_descendants(node, &mut roots),
            None => {}
        }
        if let Some(anc_handle) = anc_handle {
            for r in &roots {
                let h = self.nodes[*r].handle.clone().unwrap();
                self.toolkit.remove(&anc_handle, &h);
            }
        }
        self.collect_and_release(node);
        if parent.is_null() {
            self.layout_dirty = true;
        } else {
            self.mark_needs_measure_impl(parent);
        }
    }

    /// Remove `start` and its whole day subtree from the node table: dispose list-cell
    /// scopes, drop handler entries, queue every handle for release, decrement the
    /// implicit-animation count. Shared by [`Self::remove_subtree_impl`] and
    /// `remove_window_root` so the bookkeeping cannot drift between them.
    fn collect_and_release(&mut self, start: RNode) {
        let mut stack = vec![start];
        while let Some(n) = stack.pop() {
            // A LIST node's per-cell row subtrees live in scopes owned by the list machinery
            // (not the node tree): dispose them with the node, or their bindings — e.g. a
            // localized row label — outlive the list and patch freed native cells on the
            // next locale/theme change (a use-after-free on raw-pointer backends like Qt).
            if let Some(list) = self.lists.remove(&n) {
                for (_, cell) in list.cells {
                    cell.scope.dispose();
                    // The cell's day subtree lives OUTSIDE the node tree (anchored to a
                    // native cell, not a LIST child): remove it with the list, or its nodes
                    // linger as zombies — stale element ids that hijack `find_by_id`, and
                    // handlers whose captured signals are disposed so every press no-ops.
                    stack.push(cell.anchor);
                }
            }
            let Some(data) = self.nodes.remove(n) else {
                continue;
            };
            self.handlers.remove(&n);
            if data.implicit_anim.is_some() {
                self.implicit_anim_count = self.implicit_anim_count.saturating_sub(1);
            }
            if let Some(h) = data.handle {
                // …but a cell anchor's handle is NOT day's to free: it is the native list host's
                // own cell, borrowed through `adopt` (§15.3), and the host frees its cell pool
                // when IT is released. Queueing it here deletes it a second time — heap
                // corruption on the raw-pointer backends (xaml/qt). Dropping the handle still
                // balances whatever `adopt` retained (AppKit/UIKit/GTK/Android refcounts).
                if data.kind != kinds::LIST_CELL {
                    self.release_queue.push((data.kind, h));
                }
            }
            stack.extend(data.children);
        }
    }

    fn mark_needs_measure_impl(&mut self, node: RNode) {
        let mut cur = node;
        while let Some(n) = self.nodes.get_mut(cur) {
            n.needs_measure = true;
            n.cache.clear();
            if n.is_boundary || n.parent.is_null() {
                break;
            }
            cur = n.parent;
        }
        self.layout_dirty = true;
    }

    fn layout_now(&mut self) {
        // Every window lays out independently at its own size; the release queue drains
        // once, after all of them (a released handle may be referenced by no window).
        for i in 0..self.windows.len() {
            let (root, size) = (self.windows[i].root, self.windows[i].size);
            let p = Proposal::exact(size);
            crate::layout::measure_node(self, root, p);
            crate::layout::place_node(self, root, Rect::from_size(size), Point::ZERO, true);
        }
        let queue = std::mem::take(&mut self.release_queue);
        for (kind, h) in queue {
            // The piece's own teardown runs FIRST, while the handle is still valid: it may read
            // the native view to unregister an observer. `release` then frees it (§15.2).
            self.toolkit.release_piece(kind, &h);
            self.toolkit.release(h);
        }
    }
}

// ---------------------------------------------------------------------------
// Object-safe tree surface for pieces / bindings / handlers
// ---------------------------------------------------------------------------

/// A programmatic scroll destination (§7.6, docs/scroll.md). Edges are axis extremes
/// (`Top`/`Bottom` vertical, `Leading`/`Trailing` horizontal — start/end in layout direction);
/// `Offset` pins the viewport origin to a content-space point (clamped by the platform);
/// `Id` reveals the element with that dayscript id inside its nearest enclosing scroll.
#[derive(Clone, Debug, PartialEq)]
pub enum ScrollTarget {
    Top,
    Bottom,
    Leading,
    Trailing,
    Offset(Point),
    Id(String),
}

pub trait TreeOps {
    // The object-safe seam mirrors NodeData's fields one-to-one; grouping them into a
    // params struct would just move the same list behind a constructor.
    #[allow(clippy::too_many_arguments)]
    fn create_node(
        &mut self,
        kind: PieceKind,
        props: &dyn Any,
        layout: Rc<dyn Layout>,
        flex: Flex,
        native: bool,
        is_boundary: bool,
        scope: Scope,
    ) -> RNode;
    fn attach(&mut self, parent: RNode, child: RNode);
    fn attach_at(&mut self, parent: RNode, child: RNode, index: usize);
    fn reorder_children(&mut self, parent: RNode, order: Vec<RNode>);
    fn remove_subtree(&mut self, node: RNode);
    /// Whether `node` still exists (diagnostics: stale-id probes).
    fn node_exists(&self, node: RNode) -> bool;
    /// Whether the platform renders in dark appearance (see `Toolkit::dark_mode`).
    fn dark_mode(&mut self) -> bool;
    /// Apply an app-level appearance override (see `Toolkit::set_appearance`).
    fn set_appearance(&mut self, dark: Option<bool>);
    /// Put a badge on the app icon (see `Toolkit::set_app_badge`, docs/badge.md).
    fn set_app_badge(&mut self, badge: &day_spec::AppBadge);
    fn on_event(&mut self, node: RNode, h: EventHandler);
    fn handlers_for(&self, node: RNode) -> Vec<EventHandler>;
    fn set_id(&mut self, node: RNode, id: String);
    /// Merge non-default grid cell facts onto a node's [`Flex`] — the `.grid_span`/`.grid_align`
    /// seam (docs/grid.md). Called at build time, before the first layout.
    fn set_grid_facts(&mut self, node: RNode, facts: GridFacts);
    fn set_a11y(&mut self, node: RNode, a11y: A11yProps);
    /// Attach a native gesture recognizer to `node` (docs/shapes.md): the backend then emits
    /// `Event::Tap/LongPress/Drag` for it. The node must have a native handle.
    fn enable_gesture(&mut self, node: RNode, kind: day_spec::GestureKind);
    /// Move native keyboard focus to (or away from) `node` (docs/focus.md).
    fn focus_node(&mut self, node: RNode, focused: bool);
    /// Mirror a `FocusChanged` event into the node's dayscript probe (pump-only).
    fn set_probe_focused(&mut self, node: RNode, focused: bool);
    fn set_app_menu(&mut self, items: Vec<day_spec::MenuItem>);
    fn set_context_menu(&mut self, node: RNode, items: Vec<day_spec::MenuItem>);
    /// Install `root`'s window toolbar (docs/toolbars.md). `root` is a window root — the primary
    /// root or one returned by `open_window_root`.
    fn set_window_toolbar(&mut self, root: RNode, items: Vec<day_spec::ToolbarItem>);
    /// Apply a targeted change to one item of `root`'s toolbar.
    fn patch_window_toolbar(&mut self, root: RNode, patch: day_spec::ToolbarPatch);
    /// Programmatic scroll (§7.6, docs/scroll.md): resolve `target` against a SCROLL node's
    /// content/viewport and drive `Toolkit::scroll_to`. Returns false when `node` isn't a
    /// realized scroll (the caller reports the miss; dayscript retries).
    fn scroll_to_target(&mut self, node: RNode, target: &ScrollTarget, animated: bool) -> bool;
    /// Scroll the nearest enclosing SCROLL ancestor so `node`'s frame is visible (minimal
    /// scroll, `scrollRectToVisible` semantics). False when no scroll ancestor exists.
    fn scroll_reveal(&mut self, node: RNode, animated: bool) -> bool;
    fn patch(&mut self, node: RNode, patch: Box<dyn Any>, affects_size: bool);
    fn replay(&mut self, node: RNode, ops: Vec<DrawOp>);
    /// Set (or clear) a node's implicit `.animation` (§8.4): subsequent property patches and frame
    /// changes on this node (or its descendants) animate with `anim` even outside a
    /// `with_animation`.
    fn set_implicit_anim(&mut self, node: RNode, anim: Option<day_spec::AnimSpec>);
    /// Apply an animatable opacity (0..1) to `node`'s native handle (§8.4), animating if an
    /// animation is in scope. No-op if the node has no handle.
    fn set_node_opacity(&mut self, node: RNode, opacity: f64);
    /// Apply an animatable [`Transform`] to `node`'s native handle (§8.4) — the cheap movement/
    /// scaling channel that never relayouts. No-op if the node has no handle.
    fn set_node_transform(&mut self, node: RNode, t: day_spec::Transform);
    /// Make `node`'s text user-selectable (the `.selectable()` modifier). One-shot and unmanaged;
    /// No-op if the node has no handle.
    fn set_node_selectable(&mut self, node: RNode, selectable: bool);
    /// Record that a `.tweak` closure ran against `node`'s current handle (docs/tweaks.md), so
    /// a later backing swap (`set_node_selectable` on a toolkit that rebuilds the widget) can
    /// warn about the discarded work instead of losing it silently.
    fn note_node_tweaked(&mut self, node: RNode) {
        let _ = node;
    }
    fn mark_needs_measure(&mut self, node: RNode);
    fn mark_layout_dirty(&mut self);
    fn layout_if_needed(&mut self);
    fn set_window_size(&mut self, s: Size);
    /// Report the app's current route to the backend (`Toolkit::set_route`) — web-dom mirrors
    /// it into the URL hash (docs/navigation.md). Called by the turn-end route sync when the
    /// route actually changed.
    fn set_route(&mut self, route: &str);
    fn child_count(&self, node: RNode) -> usize;
    fn first_child(&self, node: RNode) -> Option<RNode>;
    fn node_kind(&self, node: RNode) -> Option<PieceKind>;
    /// The app-authored `.id()` string on `node` (`find_by_id`'s inverse), or `None` for an id-less
    /// or disposed node. Backs the free [`id_of`] — the recorder's NodeId → id lookup (§14.6).
    fn node_id(&self, node: RNode) -> Option<String>;
    /// A CLONE of the node's native handle boxed as `Any` (None for layout-only or disposed
    /// nodes). TreeOps is object-safe, so the generic `Toolkit::Handle` can't appear here —
    /// toolkit ext modules downcast to their concrete Handle type. This is the tweaks door
    /// (docs/tweaks.md): cloning is cheap on every backend (a retain / gobject ref / GlobalRef
    /// clone / Copy pointer) and the clone never outlives the native widget's own refcounting.
    fn node_handle_any(&self, node: RNode) -> Option<Box<dyn Any>>;
    fn node_frame(&self, node: RNode) -> Option<Rect>;
    fn node_probe(&self, node: RNode) -> Option<NodeProbe>;
    /// The node's accumulated accessibility annotations (§13) — `a11y_audit`'s expectation.
    fn node_a11y(&self, node: RNode) -> Option<A11yProps>;
    /// The node's ACTUAL native a11y properties (`a11y_audit` diffs this against `node_a11y`).
    fn read_a11y(&self, node: RNode) -> Option<day_spec::A11ySnapshot>;
    /// For every node with an `.id()` and a native handle: `(id, kind, expected, actual)` — the
    /// raw material for the `a11y_audit` step (§14.2). Comparison/policy lives in day-script.
    fn a11y_nodes(&self) -> Vec<(String, PieceKind, A11yProps, day_spec::A11ySnapshot)>;
    fn find_by_id(&self, id: &str) -> Option<RNode>;
    /// Show/hide this window's `selector(Sidebar)` pane — what a
    /// [`day_spec::ToolbarItemKind::SidebarToggle`] item drives. `false` when the toolkit has
    /// no split host to toggle (or no sidebar concept at all), which is how the caller knows to
    /// render the item disabled.
    ///
    /// A DUTY rather than an action id, because the item carries no app closure: the native
    /// toolbar button and dayscript's `toolbar:` step both land here, so a walkthrough exercises
    /// exactly the path a click takes. Defaulted to `false` so a backend without a sidebar needs
    /// no code. docs/toolbars.md, docs/navigation.md.
    fn toggle_sidebar(&mut self) -> bool {
        false
    }
    fn snapshot(&mut self) -> Result<Vec<u8>, String>;
    /// Whether native transitions have settled (see `Toolkit::ui_idle`).
    fn ui_idle(&mut self) -> bool;
    fn root_node(&self) -> RNode;

    // --- secondary windows (docs/windows.md) -------------------------------------------

    /// Open a native window and adopt its content container as an additional boundary
    /// root. Inserts the root record, asks `Toolkit::open_window`, and answers per the
    /// toolkit's reply: `Open(root)` = live now (handle installed, window entry pushed),
    /// `Pending(root)` = native creation is in flight (the record is parked without a
    /// handle until [`Self::adopt_window_root`]), `Unsupported` = the placeholder was
    /// removed again — present the content as a cover instead.
    fn open_window_root(&mut self, options: &WindowOptions, kind: WindowKind) -> WindowRootReply;
    /// Complete a `Pending` open: adopt `raw` as the window's content container, install
    /// it on the parked root, and start laying out at `size`. `false` = the root is gone
    /// (closed before completion) — the caller should drop the native window again.
    fn adopt_window_root(&mut self, root: RNode, raw: day_spec::RawHandle, size: Size) -> bool;
    /// Remove an (already childless) window root: bookkeeping + handle release + window
    /// entry removal. Never the primary. Idempotent; also valid for a parked Pending root.
    fn remove_window_root(&mut self, root: RNode);
    /// A secondary window's content size changed (its `WindowResized` handler).
    fn set_root_size(&mut self, root: RNode, s: Size);
    /// Register an EXTRA layout root: a node that stays attached in the tree (native
    /// re-homing needs the parent link) but lays out independently at its own reported
    /// size — the cover-fallback surface (docs/windows.md). The primary root's PassThrough
    /// layout only descends into its first child, so a second top-level surface must drive
    /// its own layout entry. `set_root_size` resizes it; drop it before subtree removal.
    fn add_extra_layout_root(&mut self, node: RNode, size: Size);
    /// Unregister an extra layout root (the entry only — the node itself is removed by the
    /// ordinary `remove_subtree`).
    fn drop_extra_layout_root(&mut self, node: RNode);
    /// Ask the platform to close the window whose root is `root` (async — the platform
    /// confirms with `Event::WindowClosed`).
    fn close_native_window(&mut self, root: RNode);
    /// End the app — the last primary window has closed (docs/windows.md close policy).
    fn quit_app(&mut self);
    /// Bring the window whose root is `root` to front and make it key.
    fn focus_native_window(&mut self, root: RNode);
    /// Retitle the window whose root is `root`.
    fn set_native_window_title(&mut self, root: RNode, title: &str);
    /// Snapshot the window whose root is `root` (the primary answers `snapshot`).
    fn snapshot_of(&mut self, root: RNode) -> Result<Vec<u8>, String>;
    /// Toolkit capability probe (pieces pick presentation with it, e.g. `Cap::NavSplit`).
    fn capability(&self, cap: Cap) -> Support;
    /// Does the running backend deliver this lifecycle phase (docs/lifecycle.md)?
    fn supports_lifecycle(&self, phase: day_spec::Lifecycle) -> bool;
    /// Present a native modal for request `req` (docs/dialogs.md).
    fn present(&mut self, req: u64, spec: &present::PresentSpec);
    /// Dismiss the modal for `req` (programmatic resolve while it is still up).
    fn dismiss(&mut self, req: u64);
    /// Open `url` in the platform's default handler (the `link` piece's seam).
    fn open_url(&mut self, url: &str);
    /// Re-send the union of every mounted `defers_system_gestures` request (docs/cover.md).
    fn defer_system_gestures(&mut self, edges: day_spec::Edges);

    // Recycling list seam (docs/list.md, §10). Called by day-core's own `ListSource` closures
    // (via `with_tree`) when the native list pulls rows; never nested inside another borrow.
    // (`len`/`token_at` read the piece's snapshot directly and don't need the tree.)
    fn install_list(&mut self, node: RNode, driver: crate::list::ListDriver);
    /// Decide whether row `key`'s cell must be built (returns a fresh anchor) or rebound.
    fn list_prepare_cell(
        &mut self,
        node: RNode,
        key: usize,
        cell: RawHandle,
    ) -> crate::list::CellStep;
    /// Record a freshly built row for a cell.
    fn list_store_cell(
        &mut self,
        node: RNode,
        key: usize,
        anchor: RNode,
        built: crate::list::BuiltRow,
    );
    /// Lay the row out inside its cell bounds (row content width × the RowHeight).
    fn list_layout_cell(&mut self, node: RNode, key: usize);
    /// The physical-cell keys of every bound cell of the list at `node` (for a bulk re-layout
    /// after the list's own width changed).
    fn list_cell_keys(&self, node: RNode) -> Vec<usize>;
    /// Apply a data change: the native host re-queries the source.
    fn list_reload(&mut self, node: RNode);
    /// Imperatively scroll the native list so its last row is fully visible (no-op if empty).
    fn list_scroll_to_end(&mut self, node: RNode);
    /// Imperatively scroll the native list so row `row` is visible (clamped; no-op if empty).
    fn list_scroll_to_row(&mut self, node: RNode, row: usize);
    /// Programmatically sync the list's selected rows (empty = clear). The toolkit applies
    /// without re-emitting a selection event.
    fn list_set_selected(&mut self, node: RNode, rows: Vec<usize>);
    /// The list's driver, for the guard → commit path `list_try_reorder` runs outside the
    /// borrow (`None` when `node` hosts no list).
    fn list_driver(&mut self, node: RNode) -> Option<std::rc::Rc<crate::list::ListDriver>>;
}

impl<B: Toolkit> TreeOps for Tree<B> {
    fn capability(&self, cap: Cap) -> Support {
        self.toolkit.capability(cap)
    }

    fn supports_lifecycle(&self, phase: day_spec::Lifecycle) -> bool {
        self.toolkit.supports_lifecycle(phase)
    }

    fn present(&mut self, req: u64, spec: &present::PresentSpec) {
        self.toolkit.present(req, spec);
    }

    fn dismiss(&mut self, req: u64) {
        self.toolkit.dismiss(req);
    }

    fn open_url(&mut self, url: &str) {
        self.toolkit.open_url(url);
    }

    fn set_appearance(&mut self, dark: Option<bool>) {
        self.toolkit.set_appearance(dark);
    }

    fn set_app_badge(&mut self, badge: &day_spec::AppBadge) {
        self.toolkit.set_app_badge(badge);
    }

    fn defer_system_gestures(&mut self, edges: day_spec::Edges) {
        self.toolkit.defer_system_gestures(edges);
    }

    fn create_node(
        &mut self,
        kind: PieceKind,
        props: &dyn Any,
        layout: Rc<dyn Layout>,
        flex: Flex,
        native: bool,
        is_boundary: bool,
        scope: Scope,
    ) -> RNode {
        let mut probe = NodeProbe {
            enabled: true,
            ..Default::default()
        };
        {
            use day_spec::props::*;
            if let Some(p) = props.downcast_ref::<LabelProps>() {
                probe.text = p.text.clone();
            } else if let Some(p) = props.downcast_ref::<NavMenuProps>() {
                probe.selected = p.selected.map(|i| i as i64).unwrap_or(-1);
            } else if let Some(p) = props.downcast_ref::<ButtonProps>() {
                probe.text = p.title.clone();
            } else if let Some(p) = props.downcast_ref::<ToggleProps>() {
                probe.flag = p.on;
            } else if let Some(p) = props.downcast_ref::<SliderProps>() {
                probe.value = p.value;
            } else if let Some(p) = props.downcast_ref::<TextFieldProps>() {
                probe.text = p.text.clone();
            } else if let Some(p) = props.downcast_ref::<ProgressProps>() {
                // `flag` marks indeterminate; `value` holds the determinate fraction.
                probe.flag = p.value.is_none();
                probe.value = p.value.unwrap_or(0.0);
            } else if let Some(p) = props.downcast_ref::<TabsProps>() {
                probe.value = p.selected as f64;
            } else if let Some(p) = props.downcast_ref::<PickerProps>() {
                probe.selected = p.selected as i64;
                probe.value = p.selected as f64;
            } else if let Some(p) = props.downcast_ref::<TextAreaProps>() {
                probe.text = p.text.clone();
            }
        }
        let node = self.nodes.insert(NodeData {
            kind,
            handle: None,
            parent: RNode::null(),
            children: Vec::new(),
            layout,
            flex,
            scope,
            id: None,
            a11y: Default::default(),
            cache: Vec::new(),
            baseline_cache: None,
            probe,
            needs_measure: true,
            last_native_frame: None,
            scroll_content: None,
            implicit_anim: None,
            tweaked: false,
            is_boundary,
        });
        if native {
            let h = self.toolkit.realize(kind, props, rnode_to_id(node));
            self.nodes[node].handle = Some(h);
        }
        node
    }

    fn attach(&mut self, parent: RNode, child: RNode) {
        let index = self
            .nodes
            .get(parent)
            .map(|p| p.children.len())
            .unwrap_or(0);
        self.attach_impl(parent, child, index);
    }

    fn attach_at(&mut self, parent: RNode, child: RNode, index: usize) {
        self.attach_impl(parent, child, index);
    }

    fn reorder_children(&mut self, parent: RNode, order: Vec<RNode>) {
        if let Some(p) = self.nodes.get_mut(parent) {
            p.children = order;
        }
        // Full native resync of the nearest native ancestor: rebuild in-order positions.
        let ancestor = self.native_ancestor(parent);
        let anc_handle = self.nodes[ancestor]
            .handle
            .clone()
            .expect("native ancestor");
        let mut desired = Vec::new();
        self.native_descendants(ancestor, &mut desired);
        for (i, r) in desired.iter().enumerate() {
            let h = self.nodes[*r].handle.clone().unwrap();
            self.toolkit.move_child(&anc_handle, &h, i);
        }
        self.mark_needs_measure_impl(parent);
    }

    fn remove_subtree(&mut self, node: RNode) {
        // Window roots have no native parent to detach from — they go through
        // `remove_window_root` (docs/windows.md), never here.
        debug_assert!(
            !self.windows.iter().any(|w| w.root == node),
            "remove_subtree called with a window root"
        );
        self.remove_subtree_impl(node);
    }

    fn on_event(&mut self, node: RNode, h: EventHandler) {
        self.handlers.entry(node).or_default().push(h);
    }

    fn handlers_for(&self, node: RNode) -> Vec<EventHandler> {
        self.handlers.get(&node).cloned().unwrap_or_default()
    }
    fn node_exists(&self, node: RNode) -> bool {
        self.nodes.contains_key(node)
    }
    fn dark_mode(&mut self) -> bool {
        self.toolkit.dark_mode()
    }

    fn set_id(&mut self, node: RNode, id: String) {
        if let Some(n) = self.nodes.get_mut(node) {
            n.id = Some(id.clone());
            n.a11y.merge(&A11yProps {
                identifier: Some(id),
                ..Default::default()
            });
            if let Some(h) = n.handle.clone() {
                self.toolkit.set_a11y(&h, &n.a11y);
            }
        }
    }

    fn set_grid_facts(&mut self, node: RNode, facts: GridFacts) {
        if let Some(n) = self.nodes.get_mut(node) {
            let g = &mut n.flex.grid;
            if facts.is_row {
                g.is_row = true;
            }
            if facts.row_valign.is_some() {
                g.row_valign = facts.row_valign;
            }
            if facts.col_span != 0 {
                g.col_span = facts.col_span;
            }
            if facts.align.is_some() {
                g.align = facts.align;
            }
        }
    }

    fn set_a11y(&mut self, node: RNode, a11y: A11yProps) {
        if let Some(n) = self.nodes.get_mut(node) {
            // Merge onto whatever's already recorded (piece default role, an earlier `.a11y`/`.id`)
            // and re-apply the FULL picture — backends set each present field idempotently (§13).
            n.a11y.merge(&a11y);
            if let Some(h) = n.handle.clone() {
                self.toolkit.set_a11y(&h, &n.a11y);
            }
        }
    }

    fn enable_gesture(&mut self, node: RNode, kind: day_spec::GestureKind) {
        // `.on_tap`/`.on_drag` often land on a LAYOUT-ONLY wrapper (`.frame()`, `.padding()`,
        // `.grow()` produce one) that has no native view to carry a recognizer. Descend
        // through single-child layout-only nodes to the nearest native descendant and attach
        // there — but deliver events against the ORIGINAL node, where the modifier registered
        // its handler.
        let mut cur = node;
        for _ in 0..16 {
            let Some(n) = self.nodes.get(cur) else { return };
            if let Some(h) = n.handle.clone() {
                self.toolkit.enable_gesture(&h, rnode_to_id(node), kind);
                return;
            }
            match n.children.as_slice() {
                [only] => cur = *only,
                _ => break,
            }
        }
        crate::diag(format_args!(
            "day: enable_gesture({kind:?}) found no native view under the target node — \
             the gesture will not fire natively (attach it to a piece with a native handle)"
        ));
    }

    fn focus_node(&mut self, node: RNode, focused: bool) {
        if let Some(n) = self.nodes.get(node)
            && let Some(h) = n.handle.clone()
        {
            self.toolkit.focus(&h, rnode_to_id(node), focused);
        }
    }

    fn set_probe_focused(&mut self, node: RNode, focused: bool) {
        if let Some(n) = self.nodes.get_mut(node) {
            n.probe.focused = focused;
        }
    }

    fn set_app_menu(&mut self, items: Vec<day_spec::MenuItem>) {
        self.toolkit.set_app_menu(&items);
    }

    fn set_window_toolbar(&mut self, root: RNode, items: Vec<day_spec::ToolbarItem>) {
        if let Some(h) = self.nodes.get(root).and_then(|n| n.handle.clone()) {
            self.toolkit.set_toolbar(&h, &items);
        }
    }

    fn patch_window_toolbar(&mut self, root: RNode, patch: day_spec::ToolbarPatch) {
        if let Some(h) = self.nodes.get(root).and_then(|n| n.handle.clone()) {
            self.toolkit.update_toolbar(&h, &patch);
        }
    }

    fn set_context_menu(&mut self, node: RNode, items: Vec<day_spec::MenuItem>) {
        // The modifier often sits on a handle-less layout wrapper (`.padding`/`.frame` build
        // those): the native affordance needs a real view, so fall through to the first
        // native root under the node. Silently dropping the menu here is what made
        // `label(..).padding(..).context_menu(..)` dead on every backend.
        let target = if self.nodes.get(node).is_some_and(|n| n.handle.is_some()) {
            Some(node)
        } else {
            let mut roots = Vec::new();
            self.native_descendants(node, &mut roots);
            roots.first().copied()
        };
        let Some(t) = target else {
            eprintln!("day: context_menu on a subtree with no native view — menu dropped");
            return;
        };
        if let Some(h) = self.nodes.get(t).and_then(|n| n.handle.clone()) {
            self.toolkit.set_context_menu(&h, rnode_to_id(t), &items);
        }
    }

    fn scroll_to_target(&mut self, node: RNode, target: &ScrollTarget, animated: bool) -> bool {
        // `Id` routes through reveal — the element names the scroll implicitly.
        if let ScrollTarget::Id(id) = target {
            let Some(el) = self.find_by_id(id) else {
                return false;
            };
            return self.scroll_reveal(el, animated);
        }
        let Some(n) = self.nodes.get(node) else {
            return false;
        };
        if n.kind != kinds::SCROLL {
            return false;
        }
        let Some(h) = n.handle.clone() else {
            return false;
        };
        let viewport = n.last_native_frame.map(|f| f.size).unwrap_or(Size::ZERO);
        let content = n.scroll_content.unwrap_or(viewport);
        // Compose a content-space rect whose minimal reveal lands on the target
        // (`Toolkit::scroll_to` is scrollRectToVisible semantics on every backend).
        let rect = match target {
            ScrollTarget::Top | ScrollTarget::Leading => Rect::new(0.0, 0.0, 1.0, 1.0),
            ScrollTarget::Bottom => Rect::new(0.0, (content.height - 1.0).max(0.0), 1.0, 1.0),
            ScrollTarget::Trailing => Rect::new((content.width - 1.0).max(0.0), 0.0, 1.0, 1.0),
            // A viewport-sized rect: minimal reveal pins the viewport origin to the point
            // (exactly, when the point is within the scrollable range).
            ScrollTarget::Offset(p) => Rect::new(p.x, p.y, viewport.width, viewport.height),
            ScrollTarget::Id(_) => unreachable!("routed to scroll_reveal above"),
        };
        self.toolkit.scroll_to(&h, rect, animated);
        true
    }

    fn scroll_reveal(&mut self, node: RNode, animated: bool) -> bool {
        // The element's frame is relative to its nearest REALIZED native ancestor (§7);
        // accumulate native origins up to (not including) the enclosing scroll, which puts
        // the rect in the scroll's content space — what Toolkit::scroll_to expects.
        let Some(mut rect) = self.nodes.get(node).and_then(|n| n.last_native_frame) else {
            return false;
        };
        let mut anc = match self.nodes.get(node) {
            Some(n) => n.parent,
            None => return false,
        };
        loop {
            let Some(a) = self.nodes.get(anc) else {
                return false; // walked off the root: no scroll ancestor
            };
            if a.kind == kinds::SCROLL {
                let Some(h) = a.handle.clone() else {
                    return false;
                };
                self.toolkit.scroll_to(&h, rect, animated);
                return true;
            }
            if a.handle.is_some()
                && let Some(f) = a.last_native_frame
            {
                rect.origin.x += f.origin.x;
                rect.origin.y += f.origin.y;
            }
            anc = a.parent;
        }
    }

    fn patch(&mut self, node: RNode, patch: Box<dyn Any>, affects_size: bool) {
        {
            use day_spec::props::*;
            if let Some(n) = self.nodes.get_mut(node) {
                if let Some(p) = patch.downcast_ref::<LabelPatch>() {
                    if let LabelPatch::Text(t) = p {
                        n.probe.text = t.clone();
                    }
                } else if let Some(p) = patch.downcast_ref::<ButtonPatch>() {
                    match p {
                        ButtonPatch::Title(t) => n.probe.text = t.clone(),
                        ButtonPatch::Enabled(e) => n.probe.enabled = *e,
                    }
                } else if let Some(p) = patch.downcast_ref::<TogglePatch>() {
                    match p {
                        TogglePatch::On(v) => n.probe.flag = *v,
                        TogglePatch::Enabled(e) => n.probe.enabled = *e,
                    }
                } else if let Some(p) = patch.downcast_ref::<SliderPatch>() {
                    match p {
                        SliderPatch::Value(v) => n.probe.value = *v,
                        SliderPatch::Enabled(e) => n.probe.enabled = *e,
                    }
                } else if let Some(PickerPatch::Selected(i)) = patch.downcast_ref::<PickerPatch>() {
                    n.probe.selected = *i as i64;
                    n.probe.value = *i as f64;
                } else if let Some(TextAreaPatch::SetText(t)) =
                    patch.downcast_ref::<TextAreaPatch>()
                {
                    n.probe.text = t.clone();
                } else if let Some(ProgressPatch::Value(v)) = patch.downcast_ref::<ProgressPatch>()
                {
                    n.probe.flag = v.is_none();
                    n.probe.value = v.unwrap_or(0.0);
                } else if let Some(p) = patch.downcast_ref::<TextFieldPatch>() {
                    match p {
                        TextFieldPatch::Text { text, .. } => n.probe.text = text.clone(),
                        TextFieldPatch::Enabled(e) => n.probe.enabled = *e,
                        _ => {}
                    }
                } else if let Some(NavMenuPatch::Selected(sel)) =
                    patch.downcast_ref::<NavMenuPatch>()
                {
                    n.probe.selected = sel.map(|i| i as i64).unwrap_or(-1);
                } else if let Some(TabsPatch::Selected(i)) = patch.downcast_ref::<TabsPatch>() {
                    n.probe.value = *i as f64;
                }
            }
        }
        let Some(n) = self.nodes.get(node) else {
            return;
        };
        let kind = n.kind;
        if let Some(h) = n.handle.clone() {
            let anim = self.resolve_anim(node);
            self.toolkit.update(&h, kind, patch.as_ref(), anim.as_ref());
        }
        if affects_size {
            self.mark_needs_measure_impl(node);
        }
    }

    fn set_implicit_anim(&mut self, node: RNode, anim: Option<day_spec::AnimSpec>) {
        let had = self
            .nodes
            .get(node)
            .map(|n| n.implicit_anim.is_some())
            .unwrap_or(false);
        if let Some(n) = self.nodes.get_mut(node) {
            n.implicit_anim = anim;
        } else {
            return;
        }
        match (had, anim.is_some()) {
            (false, true) => self.implicit_anim_count += 1,
            (true, false) => self.implicit_anim_count = self.implicit_anim_count.saturating_sub(1),
            _ => {}
        }
    }

    fn set_node_opacity(&mut self, node: RNode, opacity: f64) {
        let Some(h) = self.nodes.get(node).and_then(|n| n.handle.clone()) else {
            return;
        };
        let anim = self.resolve_anim(node);
        self.toolkit.set_opacity(&h, opacity, anim.as_ref());
    }

    fn set_node_transform(&mut self, node: RNode, t: day_spec::Transform) {
        let Some(n) = self.nodes.get(node) else {
            return;
        };
        let Some(h) = n.handle.clone() else {
            return;
        };
        // The node's laid-out size — passed so backends resolve the transform's anchor to a pixel
        // pivot without querying the (possibly not-yet-allocated) native widget (§8.4).
        let size = n.last_native_frame.map(|f| f.size).unwrap_or(Size::ZERO);
        let anim = self.resolve_anim(node);
        self.toolkit.set_transform(&h, t, size, anim.as_ref());
    }

    fn set_node_selectable(&mut self, node: RNode, selectable: bool) {
        let Some(h) = self.nodes.get(node).and_then(|n| n.handle.clone()) else {
            return;
        };
        // A toolkit may have to rebuild the widget as a selection-capable class (UIKit's
        // label → read-only text view); adopt the replacement so patches and layout follow it.
        if let Some(new) = self.toolkit.set_selectable(&h, selectable)
            && let Some(n) = self.nodes.get_mut(node)
        {
            if n.tweaked {
                eprintln!(
                    "day: `.selectable()` rebuilt this widget as a different native class, \
                     discarding an earlier tweak's changes — apply `.selectable()` BEFORE the \
                     tweak so it runs against the widget that ships (docs/tweaks.md)."
                );
            }
            n.handle = Some(new);
        }
    }

    fn note_node_tweaked(&mut self, node: RNode) {
        if let Some(n) = self.nodes.get_mut(node) {
            n.tweaked = true;
        }
    }

    fn replay(&mut self, node: RNode, ops: Vec<DrawOp>) {
        let Some(n) = self.nodes.get(node) else {
            return;
        };
        let size = n.last_native_frame.map(|f| f.size).unwrap_or(Size::ZERO);
        if let Some(h) = n.handle.clone() {
            self.toolkit.replay(&h, &ops, size);
        }
    }

    fn mark_needs_measure(&mut self, node: RNode) {
        self.mark_needs_measure_impl(node);
    }

    fn mark_layout_dirty(&mut self) {
        self.layout_dirty = true;
    }

    fn layout_if_needed(&mut self) {
        if !self.layout_dirty {
            return;
        }
        self.layout_dirty = false;
        self.layout_now();
    }

    fn set_route(&mut self, route: &str) {
        self.toolkit.set_route(route);
    }

    fn set_window_size(&mut self, s: Size) {
        // The PRIMARY window (WINDOW_NODE routing); secondary windows resize through
        // `set_root_size` with their own root.
        if s != self.windows[0].size {
            self.windows[0].size = s;
            let root = self.windows[0].root;
            self.mark_needs_measure_impl(root);
        }
    }

    fn child_count(&self, node: RNode) -> usize {
        self.nodes.get(node).map(|n| n.children.len()).unwrap_or(0)
    }

    fn first_child(&self, node: RNode) -> Option<RNode> {
        self.nodes
            .get(node)
            .and_then(|n| n.children.first().copied())
    }

    fn node_kind(&self, node: RNode) -> Option<PieceKind> {
        self.nodes.get(node).map(|n| n.kind)
    }

    fn node_id(&self, node: RNode) -> Option<String> {
        self.nodes.get(node).and_then(|n| n.id.clone())
    }

    fn node_handle_any(&self, node: RNode) -> Option<Box<dyn Any>> {
        self.nodes
            .get(node)
            .and_then(|n| n.handle.clone())
            .map(|h| Box::new(h) as Box<dyn Any>)
    }

    fn node_frame(&self, node: RNode) -> Option<Rect> {
        self.nodes.get(node).and_then(|n| n.last_native_frame)
    }

    fn node_probe(&self, node: RNode) -> Option<NodeProbe> {
        self.nodes.get(node).map(|n| n.probe.clone())
    }

    fn node_a11y(&self, node: RNode) -> Option<A11yProps> {
        self.nodes.get(node).map(|n| n.a11y.clone())
    }

    fn read_a11y(&self, node: RNode) -> Option<day_spec::A11ySnapshot> {
        let n = self.nodes.get(node)?;
        let h = n.handle.as_ref()?;
        Some(self.toolkit.read_a11y(h))
    }

    fn a11y_nodes(&self) -> Vec<(String, PieceKind, A11yProps, day_spec::A11ySnapshot)> {
        self.nodes
            .values()
            .filter_map(|n| {
                let id = n.id.clone()?;
                let h = n.handle.as_ref()?;
                Some((id, n.kind, n.a11y.clone(), self.toolkit.read_a11y(h)))
            })
            .collect()
    }

    fn find_by_id(&self, id: &str) -> Option<RNode> {
        self.nodes
            .iter()
            .find(|(_, n)| n.id.as_deref() == Some(id))
            .map(|(k, _)| k)
    }

    fn toggle_sidebar(&mut self) -> bool {
        self.toolkit.toggle_sidebar()
    }

    fn snapshot(&mut self) -> Result<Vec<u8>, String> {
        self.toolkit.snapshot_window()
    }

    fn ui_idle(&mut self) -> bool {
        self.toolkit.ui_idle()
    }

    fn root_node(&self) -> RNode {
        self.windows[0].root
    }

    fn open_window_root(&mut self, options: &WindowOptions, kind: WindowKind) -> WindowRootReply {
        // The same "wrap an externally-owned handle" record as the primary root and the
        // list cell anchors; the handle arrives now (desktop) or at adoption (mobile).
        let root = self.nodes.insert(NodeData {
            kind: kinds::CONTAINER,
            handle: None,
            parent: RNode::null(),
            children: Vec::new(),
            layout: Rc::new(PassThrough),
            flex: Flex::default(),
            scope: Scope::root(),
            id: None,
            a11y: Default::default(),
            cache: Vec::new(),
            baseline_cache: None,
            probe: NodeProbe::default(),
            needs_measure: true,
            last_native_frame: None,
            scroll_content: None,
            implicit_anim: None,
            tweaked: false,
            is_boundary: true,
        });
        match self.toolkit.open_window(rnode_to_id(root), options, kind) {
            day_spec::WindowOpenReply::Open(h) => {
                self.nodes[root].handle = Some(h);
                self.windows.push(WindowEntry {
                    root,
                    size: options.size,
                });
                self.layout_dirty = true;
                WindowRootReply::Open(root)
            }
            day_spec::WindowOpenReply::Pending => WindowRootReply::Pending(root),
            day_spec::WindowOpenReply::Unsupported => {
                self.nodes.remove(root);
                WindowRootReply::Unsupported
            }
        }
    }

    fn adopt_window_root(&mut self, root: RNode, raw: day_spec::RawHandle, size: Size) -> bool {
        let Some(node) = self.nodes.get_mut(root) else {
            return false; // closed before the native side finished — caller drops the window
        };
        let handle = self.toolkit.adopt(raw);
        node.handle = Some(handle);
        node.needs_measure = true;
        self.windows.push(WindowEntry { root, size });
        self.layout_dirty = true;
        true
    }

    fn remove_window_root(&mut self, root: RNode) {
        // Any window may go, including the first one opened (docs/windows.md close policy) —
        // the app's life is the life of its primary windows, not of `windows[0]` specifically.
        //
        // The LAST entry is kept, because `root()` has to answer for the whole tree and callers
        // are in no position to handle "no windows". Reaching that point means the last window
        // has closed, which is the app exiting: its content is already gone (teardown removes
        // the children first) and the empty shell outlives nothing.
        if self.windows.len() <= 1 {
            self.collect_and_release(root);
            self.layout_dirty = true;
            return;
        }
        self.windows.retain(|w| w.root != root);
        // No native detach: a window root has no native parent — the platform window is
        // already closed (or the backend releases it with the content handle).
        self.collect_and_release(root);
        self.layout_dirty = true;
    }

    fn set_root_size(&mut self, root: RNode, s: Size) {
        if let Some(w) = self.windows.iter_mut().find(|w| w.root == root)
            && w.size != s
        {
            w.size = s;
            self.mark_needs_measure_impl(root);
        }
    }

    fn add_extra_layout_root(&mut self, node: RNode, size: Size) {
        if !self.windows.iter().any(|w| w.root == node) {
            self.windows.push(WindowEntry { root: node, size });
            self.layout_dirty = true;
        }
    }

    fn drop_extra_layout_root(&mut self, node: RNode) {
        debug_assert!(
            self.windows.first().map(|w| w.root) != Some(node),
            "drop_extra_layout_root called with the primary root"
        );
        self.windows.retain(|w| w.root != node);
    }

    fn close_native_window(&mut self, root: RNode) {
        if let Some(h) = self.nodes.get(root).and_then(|n| n.handle.clone()) {
            self.toolkit.close_window(&h);
        }
    }

    fn quit_app(&mut self) {
        self.toolkit.quit_app();
    }

    fn focus_native_window(&mut self, root: RNode) {
        if let Some(h) = self.nodes.get(root).and_then(|n| n.handle.clone()) {
            self.toolkit.focus_window(&h);
        }
    }

    fn set_native_window_title(&mut self, root: RNode, title: &str) {
        if let Some(h) = self.nodes.get(root).and_then(|n| n.handle.clone()) {
            self.toolkit.set_window_title(&h, title);
        }
    }

    fn snapshot_of(&mut self, root: RNode) -> Result<Vec<u8>, String> {
        if self.windows[0].root == root {
            return self.toolkit.snapshot_window();
        }
        match self.nodes.get(root).and_then(|n| n.handle.clone()) {
            Some(h) => self.toolkit.snapshot_window_of(&h),
            None => Err("window is gone".into()),
        }
    }

    fn install_list(&mut self, node: RNode, driver: crate::list::ListDriver) {
        let driver = Rc::new(driver);
        self.lists.insert(
            node,
            crate::list::ListState {
                driver: driver.clone(),
                cells: HashMap::new(),
            },
        );
        let source = crate::list::make_source(node, driver);
        if let Some(handle) = self.nodes.get(node).and_then(|n| n.handle.clone()) {
            self.toolkit.attach_list(&handle, source);
        }
    }

    fn list_prepare_cell(
        &mut self,
        node: RNode,
        key: usize,
        cell: RawHandle,
    ) -> crate::list::CellStep {
        if let Some(state) = self.lists.get(&node)
            && let Some(bound) = state.cells.get(&key)
        {
            return crate::list::CellStep::Rebind {
                rebind: bound.rebind.clone(),
                anchor: bound.anchor,
            };
        }
        // First use of this cell: adopt the native cell and anchor a fresh subtree under it.
        let handle = self.toolkit.adopt(cell);
        let anchor = self.create_cell_anchor(handle, Scope::child());
        crate::list::CellStep::Build { anchor }
    }

    fn list_store_cell(
        &mut self,
        node: RNode,
        key: usize,
        anchor: RNode,
        built: crate::list::BuiltRow,
    ) {
        if let Some(state) = self.lists.get_mut(&node) {
            state.cells.insert(
                key,
                crate::list::BoundCell {
                    anchor,
                    scope: built.scope,
                    rebind: built.rebind,
                },
            );
        }
    }

    fn list_layout_cell(&mut self, node: RNode, key: usize) {
        let Some(state) = self.lists.get(&node) else {
            return;
        };
        let anchor = match state.cells.get(&key) {
            Some(b) => b.anchor,
            None => return,
        };
        let row_height = state.driver.row_height;
        // The row's width is the list's content width; its height is the RowHeight policy.
        let width = self
            .nodes
            .get(node)
            .and_then(|n| n.last_native_frame)
            .map(|f| f.size.width)
            .unwrap_or(self.windows[0].size.width);
        let height = match row_height {
            day_spec::props::RowHeight::Uniform(h) => h,
            day_spec::props::RowHeight::Automatic => {
                crate::layout::measure_node(self, anchor, Proposal::new(Some(width), None)).height
            }
        };
        self.nodes[anchor].needs_measure = true;
        crate::layout::place_node(
            self,
            anchor,
            Rect::new(0.0, 0.0, width, height),
            Point::ZERO,
            true,
        );
    }

    fn list_cell_keys(&self, node: RNode) -> Vec<usize> {
        self.lists
            .get(&node)
            .map(|s| s.cells.keys().copied().collect())
            .unwrap_or_default()
    }

    fn list_reload(&mut self, node: RNode) {
        if let Some(handle) = self.nodes.get(node).and_then(|n| n.handle.clone()) {
            self.toolkit.update(
                &handle,
                kinds::LIST,
                &day_spec::props::ListPatch::Reload as &dyn Any,
                None,
            );
        }
    }

    fn list_scroll_to_end(&mut self, node: RNode) {
        // Empty list: nothing to scroll to (the row count is read straight from the piece's
        // snapshot — no tree access — so this guard is cheap and backend-independent).
        if self.lists.get(&node).map(|s| (s.driver.len)()).unwrap_or(0) == 0 {
            return;
        }
        if let Some(handle) = self.nodes.get(node).and_then(|n| n.handle.clone()) {
            self.toolkit.update(
                &handle,
                kinds::LIST,
                &day_spec::props::ListPatch::ScrollToEnd as &dyn Any,
                None,
            );
        }
    }

    fn list_scroll_to_row(&mut self, node: RNode, row: usize) {
        let len = self.lists.get(&node).map(|s| (s.driver.len)()).unwrap_or(0);
        if len == 0 {
            return;
        }
        if let Some(handle) = self.nodes.get(node).and_then(|n| n.handle.clone()) {
            self.toolkit.update(
                &handle,
                kinds::LIST,
                &day_spec::props::ListPatch::ScrollToRow(row.min(len - 1)) as &dyn Any,
                None,
            );
        }
    }

    fn list_set_selected(&mut self, node: RNode, rows: Vec<usize>) {
        if let Some(handle) = self.nodes.get(node).and_then(|n| n.handle.clone()) {
            self.toolkit.update(
                &handle,
                kinds::LIST,
                &day_spec::props::ListPatch::Selected(rows) as &dyn Any,
                None,
            );
        }
    }

    fn list_driver(&mut self, node: RNode) -> Option<std::rc::Rc<crate::list::ListDriver>> {
        self.lists.get(&node).map(|s| s.driver.clone())
    }
}

// ---------------------------------------------------------------------------
// Thread-local tree + event pump
// ---------------------------------------------------------------------------

thread_local! {
    static TREE: RefCell<Option<Box<dyn TreeOps>>> = const { RefCell::new(None) };
    static EVENTS: RefCell<VecDeque<(NodeId, Event)>> = const { RefCell::new(VecDeque::new()) };
    static PUMP_PENDING: Cell<bool> = const { Cell::new(false) };
    /// The event observer installed by [`set_event_observer`] and consulted by [`enqueue_events`].
    /// `Rc`, not `Box`, so the handle can be cloned out and invoked with no borrow held (the
    /// observer may re-enter Day — read the tree, or stop recording — safely).
    static EVENT_OBSERVER: RefCell<Option<Rc<EventObserver>>> = const { RefCell::new(None) };
    /// Monotonic pump counter — bumped once per [`pump_events_inner`]. The recorder ages its
    /// coalescing candidate by it: a tap folds into a navigation caused in the SAME or the NEXT
    /// pump (a signal-bound sidebar remount settles one pump late), but never a later, unrelated
    /// navigation.
    static PUMP_GEN: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

pub fn install_tree(tree: Box<dyn TreeOps>) {
    // A fresh mount starts with no route hosts; nav()/tabs() re-register during build.
    crate::nav::clear_controllers();
    TREE.with(|t| *t.borrow_mut() = Some(tree));
}

/// Reset the thread-local tree + queues (tests).
pub fn uninstall_tree() {
    crate::nav::clear_controllers();
    crate::windows::reset_windows();
    // Per-window signals are keyed by root node, and roots repeat across trees on this thread —
    // a stale entry would hand the next tree the previous one's size class.
    crate::reset_ambient();
    TREE.with(|t| *t.borrow_mut() = None);
    EVENTS.with(|e| e.borrow_mut().clear());
    PUMP_PENDING.set(false);
    // The event observer (§14.6) is thread-local state too — a test that installed one must not
    // leak it into the next tree on this thread.
    EVENT_OBSERVER.with(|o| *o.borrow_mut() = None);
}

/// Access the installed tree. Tree methods never run user code, so nesting cannot occur
/// while a borrow is held; if events were queued during the call, they are pumped after
/// the borrow is released (the "safe point" of §3.3).
pub fn with_tree<R>(f: impl FnOnce(&mut dyn TreeOps) -> R) -> R {
    let r = TREE.with(|t| {
        let mut opt = t.borrow_mut();
        let ops = opt.as_mut().expect("day: no tree installed on this thread");
        f(ops.as_mut())
    });
    if PUMP_PENDING.replace(false) {
        pump_events();
    }
    r
}

/// Query the active toolkit's support for a capability (docs). Lets app/piece code adapt its own
/// content to the backend — e.g. a page can skip a title the native nav already shows in a header
/// (`Cap::NavHeader`), or pick a presentation from `Cap::NavSplit`.
pub fn capability(cap: day_spec::Cap) -> day_spec::Support {
    with_tree(|t| t.capability(cap))
}

/// Open `url` in the platform's default handler (system browser for `http(s)`, mail client for
/// `mailto:`, …). The seam behind the [`link`](../day_pieces/fn.link.html) piece; call it directly
/// from a tap handler for a custom affordance. Fire and forget — no result, unopenable URLs are
/// ignored by the backend.
pub fn open_url(url: &str) {
    with_tree(|t| t.open_url(url));
}

/// Tell layout that a node's intrinsic size may have changed. For tweaks (docs/tweaks.md):
/// after a native call that alters a widget's preferred size (fonts, tick marks, bezel styles),
/// the measure cache along the node's path must be invalidated — Day can't see native mutations
/// it didn't make. Relayout runs at the next turn boundary as usual. No-op on a disposed node.
pub fn invalidate_size(node: RNode) {
    with_tree(|t| {
        t.mark_needs_measure(node);
        t.mark_layout_dirty();
    });
}

/// Like `with_tree`, but returns `None` instead of panicking when the tree can't be entered:
/// already borrowed, or not installed yet. A snapshot (`TreeOps::snapshot`) holds the borrow
/// while the backend draws the window synchronously, and that draw can re-enter Day through a
/// native callback — e.g. a lazy list's `viewForRow`/`connect_bind`/`cellForRow` firing during
/// `cacheDisplayInRect`. And platform style callbacks can fire before `install_tree` — e.g.
/// GTK's StyleManager emits a `dark` notify while `startup` applies a forced `DAY_THEME`
/// scheme, before `activate` mounts the tree; a panic there unwinds into a C signal trampoline
/// and aborts the process. Such callbacks use this and simply skip their work; the next real
/// layout (or the signal's first post-mount read) catches up.
pub fn try_with_tree<R>(f: impl FnOnce(&mut dyn TreeOps) -> R) -> Option<R> {
    let r = TREE.with(|t| {
        let mut opt = t.try_borrow_mut().ok()?;
        let ops = opt.as_mut()?;
        Some(f(ops.as_mut()))
    });
    if r.is_some() && PUMP_PENDING.replace(false) {
        pump_events();
    }
    r
}

pub fn has_tree() -> bool {
    TREE.with(|t| t.borrow().is_some())
}

/// The enqueue-only event sink installed into every backend (§8.3). May be invoked
/// re-entrantly from inside any Toolkit method; dispatch happens at the next safe point.
pub fn enqueue_event(id: NodeId, ev: Event) {
    enqueue_events([(id, ev)]);
}

/// Enqueue several events into ONE drain before dispatching. Backends that observe a focus
/// move at a single point (Qt's `focusChanged(old, new)`, an AppKit first-responder change)
/// deliver the loss+gain pair through this so the pump can dispatch the gain first and a
/// shared group signal never passes through `None` (docs/focus.md).
pub fn enqueue_events(evs: impl IntoIterator<Item = (NodeId, Event)>) {
    // Recording/telemetry seam (§14.6): when an observer is installed, let it see every event in
    // the exact order — and the exact form — the app is about to receive, BEFORE it is dispatched,
    // so it observes precisely what the app receives. This is the single point EVERY backend
    // funnels native events through, so the observer needs no per-toolkit code. The handle is
    // cloned out first (a cheap `Rc` bump) so NO thread-local borrow is held across the call: the
    // observer may re-enter Day — resolve an id via `id_of`, or even remove itself (stop
    // recording) — without a borrow conflict. A `None` observer costs one `Option` check and takes
    // the original zero-copy `extend` path.
    let observer = EVENT_OBSERVER.with(|o| o.borrow().clone());
    match observer {
        Some(obs) => {
            let batch: Vec<(NodeId, Event)> = evs.into_iter().collect();
            for (id, ev) in &batch {
                obs(*id, ev);
            }
            EVENTS.with(|e| e.borrow_mut().extend(batch));
        }
        None => EVENTS.with(|e| e.borrow_mut().extend(evs)),
    }
    let tree_free = TREE.with(|t| t.try_borrow_mut().is_ok());
    if tree_free {
        pump_events();
    } else {
        PUMP_PENDING.set(true);
    }
}

/// The current pump generation (see `PUMP_GEN`). Read by the recorder's nav coalescing.
pub fn pump_generation() -> u64 {
    PUMP_GEN.with(|g| g.get())
}

/// Install (or clear, with `None`) an observer that sees every event day-core dispatches, in queue
/// order, at the one point EVERY backend funnels native events through ([`enqueue_events`], §8.3) —
/// BEFORE the event reaches the app, so it observes exactly what the app receives. This is the
/// recording/telemetry seam behind [`day::record`](../day_script/record/index.html) (§14.6): a
/// higher layer captures user actions into a replayable dayscript without touching any of the
/// backends. Main-thread only; a `None` observer adds no cost to the event path. The boxed closure
/// is adopted into an `Rc` internally so [`enqueue_events`] can call it with no borrow held.
pub fn set_event_observer(observer: Option<Box<EventObserver>>) {
    EVENT_OBSERVER.with(|o| *o.borrow_mut() = observer.map(Rc::from));
}

/// Resolve a dispatched [`NodeId`] back to the app-authored `.id()` string that named its node
/// (`find_by_id`'s inverse, §5.5), or `None` for an id-less node or one no longer in the tree. The
/// recorder ([`day::record`](../day_script/record/index.html), §14.6) calls this from inside its
/// event observer to label a tap/input with the id an app would target in a dayscript step.
/// Borrow-safe: returns `None` rather than panicking if the tree is momentarily borrowed by a
/// re-entrant backend call.
pub fn id_of(node: NodeId) -> Option<String> {
    try_with_tree(|t| t.node_id(id_to_rnode(node))).flatten()
}

/// A human-readable label for a node, for annotating a recorded script (§14.6): the accessibility
/// label if one is set (`.a11y(label = …)`, the localized string the screen reader speaks),
/// otherwise the control's own visible text. `None` when the node carries neither.
pub fn label_of(node: NodeId) -> Option<String> {
    try_with_tree(|t| {
        let rnode = id_to_rnode(node);
        let a11y = t
            .node_a11y(rnode)
            .and_then(|a| a.label)
            .filter(|s| !s.is_empty());
        a11y.or_else(|| {
            t.node_probe(rnode)
                .map(|p| p.text)
                .filter(|s| !s.is_empty())
        })
    })
    .flatten()
}

/// Dispatch queued native events (see [`pump_events_inner`]), CONTAINING any panic. Native event
/// callbacks reach Day through `extern "C"` signal trampolines (GTK's `value_changed_trampoline`,
/// Qt's event filters, …) that ABORT the process on unwind (`panic_cannot_unwind`). A panic in a Day
/// event handler or its reactive drain — e.g. the reactive-cycle assertion firing during a slider
/// drag — would therefore `SIGABRT` the whole app instead of surfacing. Catch it at this single
/// backend-agnostic boundary, log it (the message carries the offending effect's source location), and
/// reset the runtime so the app keeps running (degraded) rather than crashing.
pub fn pump_events() {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(pump_events_inner));
    if let Err(payload) = result {
        let msg = payload
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic".to_string());
        crate::diag(format_args!(
            "day: a native event handler panicked and was contained — the app continues, but \
             reactive/UI state may be inconsistent until the next interaction. Cause: {msg}"
        ));
        // Drop the in-flight event batch and reset drain state so the runtime isn't wedged.
        EVENTS.with(|e| e.borrow_mut().clear());
        PUMP_PENDING.set(false);
        day_reactive::recover_from_panic();
        crate::notify_contained_panic();
    }
}

fn pump_events_inner() {
    // A new pump: bump the generation, then record any route change that a PREVIOUS pump left
    // pending (a signal-bound sidebar/stack remount settles into NAV_STACK a tick after the pump
    // that triggered it, so the tail check below reads a stale route — this start check catches it
    // at the next pump boundary, still fresh enough to fold its triggering select/tap). §14.6.
    PUMP_GEN.with(|g| g.set(g.get() + 1));
    crate::nav::maybe_notify_route_change();
    loop {
        let item = EVENTS.with(|e| e.borrow_mut().pop_front());
        let Some((id, ev)) = item else { break };
        // Presentation answers are keyed by request id, not by tree node (docs/dialogs.md).
        if let Event::PresentResult { req, result } = ev {
            crate::present::resolve_presentation(req, result);
            continue;
        }
        // Menu actions are keyed by action id, not by tree node (§ menus).
        if let Event::MenuAction(action) = ev {
            crate::menu::dispatch_menu_action(action);
            continue;
        }
        // Toolbar values are keyed by action id too (docs/toolbars.md) — borrowed, because the
        // payload owns a String and the arms below still need `ev`.
        if let Event::ToolbarChanged { action, value } = &ev {
            crate::toolbar::dispatch_toolbar_value(*action, value);
            continue;
        }
        // Lifecycle phases are app-global, not keyed by tree node (docs/lifecycle.md).
        if let Event::Lifecycle(phase) = ev {
            crate::lifecycle::dispatch_lifecycle(phase);
            continue;
        }
        // Focus loss/gain pairing (docs/focus.md): when focus moves between two Day controls,
        // the loss and gain arrive as separate events. Dispatching the queued GAIN first lets a
        // shared group signal transition `Some(A)` → `Some(B)` without an observable `None`
        // (the loss handler only clears the signal if it still names its own control).
        if ev == Event::FocusChanged(false) {
            let paired = EVENTS.with(|e| {
                let mut q = e.borrow_mut();
                let gain = q
                    .iter()
                    .position(|(gid, gev)| *gid != id && *gev == Event::FocusChanged(true));
                gain.map(|i| q.remove(i).expect("indexed event"))
            });
            if let Some((gid, gev)) = paired {
                dispatch_focus_probe(gid, &gev);
                dispatch_to_node(gid, &gev);
            }
        }
        if let Event::FocusChanged(_) = ev {
            dispatch_focus_probe(id, &ev);
        }
        dispatch_to_node(id, &ev);
    }
    day_reactive::flush_sync();
    // The route has settled now that every queued event is dispatched and reactive writes have
    // flushed. A sidebar click, a nav_link, a stack push, and a native back all change the route
    // by calling `navigate`/`pop` from an event handler — none of which pass back through
    // `enqueue_events` — so the event observer never sees them. Notifying here (deduped against
    // the last route) is how the recorder captures navigation regardless of what triggered it
    // (docs/navigation.md, §14.6).
    crate::nav::maybe_notify_route_change();
}

/// Mirror a focus event into the node's dayscript probe (`assert_focused` reads it).
fn dispatch_focus_probe(id: NodeId, ev: &Event) {
    if let Event::FocusChanged(f) = ev {
        let node = id_to_rnode(id);
        with_tree(|t| t.set_probe_focused(node, *f));
    }
}

/// Imperatively scroll a `scroll` piece (docs/scroll.md): `node` is the SCROLL node for edge
/// and offset targets (`ScrollTarget::Id` ignores it and reveals the named element in its own
/// nearest scroll). Animated on-screen; dayscript uses the unanimated variant for determinism.
/// Call with no tree borrow held.
pub fn scroll_to(node: RNode, target: ScrollTarget) {
    with_tree(|t| {
        t.scroll_to_target(node, &target, true);
    });
}

/// Run one already-routed event through its node's handlers (the tail of the pump loop).
fn dispatch_to_node(id: NodeId, ev: &Event) {
    let node = if id == day_spec::WINDOW_NODE {
        with_tree(|t| t.root_node())
    } else {
        id_to_rnode(id)
    };
    let handlers = with_tree(|t| t.handlers_for(node));
    if handlers.is_empty() {
        // A press routed to a node that no longer exists is a staleness bug somewhere
        // upstream (an element index, a native view outliving its node) — surface it in
        // debug builds instead of dropping silently; alive-but-handlerless is normal.
        if cfg!(debug_assertions)
            && matches!(ev, Event::Pressed | Event::Tap(_))
            && !with_tree(|t| t.node_exists(node))
        {
            crate::diag(format_args!(
                "day: {ev:?} dropped — node {id:?} no longer exists (stale reference)"
            ));
        }
        return;
    }
    day_reactive::batch(|| {
        for h in &handlers {
            h(ev);
        }
    });
}
