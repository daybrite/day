// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The build layer (DESIGN.md §5.1–§5.2): pieces are descriptions consumed exactly once.
//! `BuildCx` holds no tree borrow — every operation goes through `with_tree`, so bindings
//! and structural effects created during build can re-enter safely.

use std::any::Any;
use std::rc::Rc;

use day_reactive::Scope;
use day_spec::{Event, PieceKind};

use crate::layout::Layout;
use crate::tree::{Flex, RNode, with_tree};

/// Measure-invalidation boundary (§7.4): `Yes` stops upward needs-measure propagation
/// (scroll viewports, nav pages). Named enum instead of a bare bool at call sites
/// (docs/api-style.md).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Boundary {
    No,
    Yes,
}

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
        boundary: Boundary,
    ) -> RNode {
        let n = with_tree(|t| {
            t.create_node(
                kind,
                props,
                layout,
                flex,
                true,
                boundary == Boundary::Yes,
                Scope::current(),
            )
        });
        with_tree(|t| t.attach(self.parent, n));
        n
    }

    /// Create + attach a layout-only node (wrappers, groups, spacer).
    pub fn layout_only(&mut self, layout: Rc<dyn Layout>, flex: Flex, boundary: Boundary) -> RNode {
        let n = with_tree(|t| {
            t.create_node(
                "day.layout",
                &(),
                layout,
                flex,
                false,
                boundary == Boundary::Yes,
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

    /// Already erased — hands itself back instead of boxing a box.
    ///
    /// INHERENT so it shadows the blanket `Decorate::any` (inherent methods win method
    /// resolution). Every `Decorate` modifier returns an erased piece, so `.padding(8.0).any()`
    /// and friends are common in app code and would otherwise pay a second allocation and a
    /// second indirect call for nothing.
    pub fn any(self) -> AnyPiece {
        self
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

/// A build-time branch between two piece TYPES, without erasing either (§5.1).
///
/// `if compact { a.any() } else { b.any() }` boxes whichever arm it takes; `Either` keeps both
/// concrete, so the branch costs nothing and the chosen arm's type survives into the parent:
///
/// ```ignore
/// if compact { Either::Left(row(…)) } else { Either::Right(column(…)) }
/// ```
///
/// For a branch on a SIGNAL — one that must re-evaluate — use `when(…).otherwise(…)` instead.
/// This is a plain `if` resolved once at build.
pub enum Either<A, B> {
    Left(A),
    Right(B),
}

impl<A: Piece, B: Piece> Piece for Either<A, B> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        match self {
            Either::Left(a) => a.build(cx),
            Either::Right(b) => b.build(cx),
        }
    }
}

#[cfg(test)]
mod any_piece_tests {
    use super::*;
    use slotmap::Key as _;

    fn box_addr(p: &AnyPiece) -> *const () {
        (&*p.0 as *const dyn FnOnce(&mut BuildCx) -> RNode).cast::<()>()
    }

    /// `.any()` on an already-erased piece hands the SAME allocation back. Guards the inherent
    /// method against being deleted as "redundant with `Decorate::any`" — it is what stops a
    /// second box, and method resolution silently falls back to the blanket trait without it.
    #[test]
    fn any_on_an_erased_piece_reuses_its_box() {
        let p = piece_fn(|_| RNode::null());
        let before = box_addr(&p);
        let p = p.any();
        assert_eq!(before, box_addr(&p));
    }
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
