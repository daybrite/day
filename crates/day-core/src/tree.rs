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

use crate::layout::{Layout, PassThrough};

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
}

/// Cached last-applied props for the dayscript element index (§14.2).
#[derive(Clone, Debug, Default)]
pub struct NodeProbe {
    pub text: String,
    pub value: f64,
    pub flag: bool,
    pub selected: i64,
    pub enabled: bool,
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
    // --- layout state (§7.4) ---
    pub cache: Vec<((u64, u64), Size)>,
    pub probe: NodeProbe,
    pub needs_measure: bool,
    pub last_native_frame: Option<Rect>,
    pub is_boundary: bool,
}

/// An event handler registered on a realized node.
pub type EventHandler = Rc<dyn Fn(&Event)>;

pub struct Tree<B: Toolkit> {
    pub toolkit: B,
    nodes: SlotMap<RNode, NodeData<B::Handle>>,
    root: RNode,
    window_size: Size,
    layout_dirty: bool,
    handlers: HashMap<RNode, Vec<EventHandler>>,
    release_queue: Vec<B::Handle>,
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
            cache: Vec::new(),
            probe: NodeProbe::default(),
            needs_measure: true,
            last_native_frame: None,
            is_boundary: true,
        });
        Tree {
            toolkit,
            nodes,
            root,
            window_size,
            layout_dirty: true,
            handlers: HashMap::new(),
            release_queue: Vec::new(),
        }
    }

    pub fn root(&self) -> RNode {
        self.root
    }

    pub(crate) fn node(&self, n: RNode) -> Option<&NodeData<B::Handle>> {
        self.nodes.get(n)
    }
    pub(crate) fn node_mut(&mut self, n: RNode) -> Option<&mut NodeData<B::Handle>> {
        self.nodes.get_mut(n)
    }

    /// Nearest ancestor (or self) with a native handle.
    fn native_ancestor(&self, mut n: RNode) -> RNode {
        loop {
            let Some(node) = self.nodes.get(n) else {
                return self.root;
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
        // Collect the whole subtree.
        let mut stack = vec![node];
        while let Some(n) = stack.pop() {
            let Some(data) = self.nodes.remove(n) else {
                continue;
            };
            self.handlers.remove(&n);
            if let Some(h) = data.handle {
                self.release_queue.push(h);
            }
            stack.extend(data.children);
        }
        if parent.is_null() {
            self.layout_dirty = true;
        } else {
            self.mark_needs_measure_impl(parent);
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
        let root = self.root;
        let size = self.window_size;
        let p = Proposal::exact(size);
        crate::layout::measure_node(self, root, p);
        crate::layout::place_node(self, root, Rect::from_size(size), Point::ZERO, true);
        let queue = std::mem::take(&mut self.release_queue);
        for h in queue {
            self.toolkit.release(h);
        }
    }
}

// ---------------------------------------------------------------------------
// Object-safe tree surface for pieces / bindings / handlers
// ---------------------------------------------------------------------------

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
    fn on_event(&mut self, node: RNode, h: EventHandler);
    fn handlers_for(&self, node: RNode) -> Vec<EventHandler>;
    fn set_id(&mut self, node: RNode, id: String);
    fn set_a11y(&mut self, node: RNode, a11y: A11yProps);
    fn patch(&mut self, node: RNode, patch: Box<dyn Any>, affects_size: bool);
    fn replay(&mut self, node: RNode, ops: Vec<DrawOp>);
    fn mark_needs_measure(&mut self, node: RNode);
    fn mark_layout_dirty(&mut self);
    fn layout_if_needed(&mut self);
    fn set_window_size(&mut self, s: Size);
    fn child_count(&self, node: RNode) -> usize;
    fn first_child(&self, node: RNode) -> Option<RNode>;
    fn node_kind(&self, node: RNode) -> Option<PieceKind>;
    fn node_frame(&self, node: RNode) -> Option<Rect>;
    fn node_probe(&self, node: RNode) -> Option<NodeProbe>;
    fn find_by_id(&self, id: &str) -> Option<RNode>;
    fn snapshot(&mut self) -> Result<Vec<u8>, String>;
    fn root_node(&self) -> RNode;
}

impl<B: Toolkit> TreeOps for Tree<B> {
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
            } else if let Some(p) = props.downcast_ref::<ButtonProps>() {
                probe.text = p.title.clone();
            } else if let Some(p) = props.downcast_ref::<ToggleProps>() {
                probe.flag = p.on;
            } else if let Some(p) = props.downcast_ref::<SliderProps>() {
                probe.value = p.value;
            } else if let Some(p) = props.downcast_ref::<TextFieldProps>() {
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
            cache: Vec::new(),
            probe,
            needs_measure: true,
            last_native_frame: None,
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
        self.remove_subtree_impl(node);
    }

    fn on_event(&mut self, node: RNode, h: EventHandler) {
        self.handlers.entry(node).or_default().push(h);
    }

    fn handlers_for(&self, node: RNode) -> Vec<EventHandler> {
        self.handlers.get(&node).cloned().unwrap_or_default()
    }

    fn set_id(&mut self, node: RNode, id: String) {
        if let Some(n) = self.nodes.get_mut(node) {
            n.id = Some(id.clone());
            if let Some(h) = n.handle.clone() {
                let a11y = A11yProps {
                    identifier: Some(id),
                    ..Default::default()
                };
                self.toolkit.set_a11y(&h, &a11y);
            }
        }
    }

    fn set_a11y(&mut self, node: RNode, a11y: A11yProps) {
        if let Some(n) = self.nodes.get(node)
            && let Some(h) = n.handle.clone() {
                self.toolkit.set_a11y(&h, &a11y);
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
                } else if let Some(p) = patch.downcast_ref::<TextFieldPatch>() {
                    match p {
                        TextFieldPatch::Text { text, .. } => n.probe.text = text.clone(),
                        TextFieldPatch::Enabled(e) => n.probe.enabled = *e,
                        _ => {}
                    }
                }
            }
        }
        let Some(n) = self.nodes.get(node) else {
            return;
        };
        let kind = n.kind;
        if let Some(h) = n.handle.clone() {
            self.toolkit.update(&h, kind, patch.as_ref(), None);
        }
        if affects_size {
            self.mark_needs_measure_impl(node);
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

    fn set_window_size(&mut self, s: Size) {
        if s != self.window_size {
            self.window_size = s;
            let root = self.root;
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

    fn node_frame(&self, node: RNode) -> Option<Rect> {
        self.nodes.get(node).and_then(|n| n.last_native_frame)
    }

    fn node_probe(&self, node: RNode) -> Option<NodeProbe> {
        self.nodes.get(node).map(|n| n.probe.clone())
    }

    fn find_by_id(&self, id: &str) -> Option<RNode> {
        self.nodes
            .iter()
            .find(|(_, n)| n.id.as_deref() == Some(id))
            .map(|(k, _)| k)
    }

    fn snapshot(&mut self) -> Result<Vec<u8>, String> {
        self.toolkit.snapshot_window()
    }

    fn root_node(&self) -> RNode {
        self.root
    }
}

// ---------------------------------------------------------------------------
// Thread-local tree + event pump
// ---------------------------------------------------------------------------

thread_local! {
    static TREE: RefCell<Option<Box<dyn TreeOps>>> = const { RefCell::new(None) };
    static EVENTS: RefCell<VecDeque<(NodeId, Event)>> = const { RefCell::new(VecDeque::new()) };
    static PUMP_PENDING: Cell<bool> = const { Cell::new(false) };
}

pub fn install_tree(tree: Box<dyn TreeOps>) {
    TREE.with(|t| *t.borrow_mut() = Some(tree));
}

/// Reset the thread-local tree + queues (tests).
pub fn uninstall_tree() {
    TREE.with(|t| *t.borrow_mut() = None);
    EVENTS.with(|e| e.borrow_mut().clear());
    PUMP_PENDING.set(false);
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

pub fn has_tree() -> bool {
    TREE.with(|t| t.borrow().is_some())
}

/// The enqueue-only event sink installed into every backend (§8.3). May be invoked
/// re-entrantly from inside any Toolkit method; dispatch happens at the next safe point.
pub fn enqueue_event(id: NodeId, ev: Event) {
    EVENTS.with(|e| e.borrow_mut().push_back((id, ev)));
    let tree_free = TREE.with(|t| t.try_borrow_mut().is_ok());
    if tree_free {
        pump_events();
    } else {
        PUMP_PENDING.set(true);
    }
}

/// Dispatch queued native events: each event is a fresh batch (§3.3 step 1–2).
pub fn pump_events() {
    loop {
        let item = EVENTS.with(|e| e.borrow_mut().pop_front());
        let Some((id, ev)) = item else { break };
        let node = if id == day_spec::WINDOW_NODE {
            with_tree(|t| t.root_node())
        } else {
            id_to_rnode(id)
        };
        let handlers = with_tree(|t| t.handlers_for(node));
        if handlers.is_empty() {
            continue;
        }
        day_reactive::batch(|| {
            for h in &handlers {
                h(&ev);
            }
        });
    }
}
