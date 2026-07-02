//! The build layer (DESIGN.md §5.1–§5.2): pieces are descriptions consumed exactly once.
//! `BuildCx` holds no tree borrow — every operation goes through `with_tree`, so bindings
//! and structural effects created during build can re-enter safely.

use std::any::Any;
use std::rc::Rc;

use day_reactive::Scope;
use day_spec::{Event, PieceKind};

use crate::layout::Layout;
use crate::tree::{Flex, RNode, with_tree};

pub struct BuildCx {
    parent: RNode,
}

impl BuildCx {
    pub fn new(parent: RNode) -> Self {
        BuildCx { parent }
    }

    pub fn parent(&self) -> RNode {
        self.parent
    }

    /// Create + attach a native leaf.
    pub fn leaf(&mut self, kind: PieceKind, props: &dyn Any, flex: Flex) -> RNode {
        let n = with_tree(|t| {
            t.create_node(
                kind,
                props,
                Rc::new(crate::layout::LeafLayout),
                flex,
                true,
                false,
                Scope::current(),
            )
        });
        with_tree(|t| t.attach(self.parent, n));
        n
    }

    /// Create + attach a native node with a custom layout (containers, scroll).
    pub fn native(
        &mut self,
        kind: PieceKind,
        props: &dyn Any,
        layout: Rc<dyn Layout>,
        flex: Flex,
        is_boundary: bool,
    ) -> RNode {
        let n = with_tree(|t| {
            t.create_node(
                kind,
                props,
                layout,
                flex,
                true,
                is_boundary,
                Scope::current(),
            )
        });
        with_tree(|t| t.attach(self.parent, n));
        n
    }

    /// Create + attach a layout-only node (wrappers, groups, spacer).
    pub fn layout_only(&mut self, layout: Rc<dyn Layout>, flex: Flex, is_boundary: bool) -> RNode {
        let n = with_tree(|t| {
            t.create_node(
                "day.layout",
                &(),
                layout,
                flex,
                false,
                is_boundary,
                Scope::current(),
            )
        });
        with_tree(|t| t.attach(self.parent, n));
        n
    }

    /// Build `f` with `node` as the parent.
    pub fn under<R>(&mut self, node: RNode, f: impl FnOnce(&mut BuildCx) -> R) -> R {
        let mut cx = BuildCx { parent: node };
        f(&mut cx)
    }

    /// Register a native-event handler for a node (runs under the registration scope, §4.3).
    pub fn on(&mut self, node: RNode, h: impl Fn(&Event) + 'static) {
        let scope = Scope::current();
        let wrapped: Rc<dyn Fn(&Event)> = Rc::new(move |ev| {
            if scope.is_alive() {
                let ev = ev.clone();
                scope.enter(|| h(&ev));
            }
        });
        with_tree(|t| t.on_event(node, wrapped));
    }
}

// ---------------------------------------------------------------------------
// Piece
// ---------------------------------------------------------------------------

/// A UI description consumed once (§5.2). Returns the root realized node it created.
pub trait Piece: 'static {
    fn build(self, cx: &mut BuildCx) -> RNode;
}

/// Type-erased piece for heterogeneous branches and dynamic construction.
pub struct AnyPiece(Box<dyn FnOnce(&mut BuildCx) -> RNode>);

impl AnyPiece {
    pub fn new<P: Piece>(p: P) -> Self {
        AnyPiece(Box::new(move |cx| p.build(cx)))
    }
}

impl Piece for AnyPiece {
    fn build(self, cx: &mut BuildCx) -> RNode {
        (self.0)(cx)
    }
}

/// A piece from a closure.
pub fn piece_fn(f: impl FnOnce(&mut BuildCx) -> RNode + 'static) -> AnyPiece {
    AnyPiece(Box::new(f))
}

// ---------------------------------------------------------------------------
// PieceSeq — tuple children (§5.1), flattening recursively.
// ---------------------------------------------------------------------------

/// Children of a container: a tuple of pieces (the floem `ViewTuple` pattern — implemented
/// ONLY for tuples, `()`, and [`PieceVec`], never via a blanket, to stay coherent).
pub trait PieceSeq: 'static {
    fn build_each(self, cx: &mut BuildCx);
}

impl PieceSeq for () {
    fn build_each(self, _cx: &mut BuildCx) {}
}

/// Runtime-heterogeneous children (`column_vec`-style call sites).
pub struct PieceVec(pub Vec<AnyPiece>);

impl PieceSeq for PieceVec {
    fn build_each(self, cx: &mut BuildCx) {
        for p in self.0 {
            let _ = p.build(cx);
        }
    }
}

macro_rules! impl_piece_seq {
    ($($name:ident),+) => {
        impl<$($name: Piece),+> PieceSeq for ($($name,)+) {
            #[allow(non_snake_case)]
            fn build_each(self, cx: &mut BuildCx) {
                let ($($name,)+) = self;
                $(let _ = $name.build(cx);)+
            }
        }
    };
}

impl_piece_seq!(A);
impl_piece_seq!(A, B);
impl_piece_seq!(A, B, C);
impl_piece_seq!(A, B, C, D);
impl_piece_seq!(A, B, C, D, E);
impl_piece_seq!(A, B, C, D, E, F);
impl_piece_seq!(A, B, C, D, E, F, G);
impl_piece_seq!(A, B, C, D, E, F, G, H);
impl_piece_seq!(A, B, C, D, E, F, G, H, I);
impl_piece_seq!(A, B, C, D, E, F, G, H, I, J);
impl_piece_seq!(A, B, C, D, E, F, G, H, I, J, K);
impl_piece_seq!(A, B, C, D, E, F, G, H, I, J, K, L);
impl_piece_seq!(A, B, C, D, E, F, G, H, I, J, K, L, M);
impl_piece_seq!(A, B, C, D, E, F, G, H, I, J, K, L, M, N);
impl_piece_seq!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O);
impl_piece_seq!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, Q);
