// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Named operations supplied by pieces, independent of their native toolkit.

use crate::{RNode, tree};
use day_spec::PieceKind;
use std::{cell::RefCell, collections::HashMap, rc::Rc};

/// Completes an operation with its textual result or a human-readable error.
/// The operation defines the payload format. Completion may be immediate or asynchronous.
pub type PieceOperationDone = Box<dyn FnOnce(Result<String, String>)>;
/// An operation on a live node of the registered kind. Copy the input if retaining it.
/// Accepted requests must complete, including when the native operation fails.
pub type PieceOperationFn = Rc<dyn Fn(RNode, &str, PieceOperationDone)>;

type Operations = HashMap<(PieceKind, &'static str), PieceOperationFn>;
thread_local! {
    static OPERATIONS: RefCell<Operations> = RefCell::new(HashMap::new());
}

/// Register a named operation for one piece kind. Use a namespaced name and document its
/// input and output. Registering the same kind and name replaces only that handler.
pub fn register_piece_operation(kind: PieceKind, name: &'static str, handler: PieceOperationFn) {
    OPERATIONS.with(|ops| ops.borrow_mut().insert((kind, name), handler));
}

/// Dispatch to the live node's kind. Returns false without calling `done` if the node or
/// operation is absent. A true result means the handler accepted responsibility for completion.
/// The registry and tree are not borrowed while the handler runs, so it may update the tree
/// or register another operation.
pub fn piece_operation(node: RNode, name: &str, input: &str, done: PieceOperationDone) -> bool {
    let Some(kind) = tree::with_tree(|t| t.node_kind(node)) else {
        return false;
    };
    dispatch(kind, node, name, input, done)
}

fn dispatch(
    kind: PieceKind,
    node: RNode,
    name: &str,
    input: &str,
    done: PieceOperationDone,
) -> bool {
    let handler = OPERATIONS.with(|ops| ops.borrow().get(&(kind, name)).cloned());
    let Some(handler) = handler else { return false };
    handler(node, input, done);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operations_are_scoped_by_kind_and_name_and_can_reenter() {
        let node = RNode::default();
        register_piece_operation(
            "test.first",
            "test.read",
            Rc::new(|_, input, done| {
                // Registration and completion may re-enter the registry without a RefCell panic.
                register_piece_operation(
                    "test.second",
                    "test.read",
                    Rc::new(|_, _, done| done(Ok("second".into()))),
                );
                done(Ok(input.to_uppercase()));
            }),
        );
        register_piece_operation(
            "test.first",
            "test.write",
            Rc::new(|_, _, done| done(Err("read-only".into()))),
        );
        let result = Rc::new(RefCell::new(None));
        let call = |kind, name, input| {
            let result = result.clone();
            dispatch(
                kind,
                node,
                name,
                input,
                Box::new(move |r| *result.borrow_mut() = Some(r)),
            )
        };
        assert!(!call("test.absent", "test.read", "hello"));
        assert!(!call("test.first", "test.absent", "hello"));
        assert!(result.borrow().is_none());
        assert!(call("test.first", "test.read", "hello"));
        assert_eq!(result.borrow_mut().take(), Some(Ok("HELLO".into())));
        assert!(call("test.second", "test.read", ""));
        assert_eq!(result.borrow_mut().take(), Some(Ok("second".into())));
        assert!(call("test.first", "test.write", ""));
        assert_eq!(result.borrow_mut().take(), Some(Err("read-only".into())));
        register_piece_operation(
            "test.first",
            "test.read",
            Rc::new(|_, _, done| done(Ok("replacement".into()))),
        );
        assert!(call("test.first", "test.read", ""));
        assert_eq!(result.borrow_mut().take(), Some(Ok("replacement".into())));
    }
}
