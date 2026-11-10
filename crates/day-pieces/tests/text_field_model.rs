// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! A text field bound to a day-model field: typing is one preview session, sealed into one
//! undo unit on Return or focus loss, and an app write over the typing (an address bar's
//! completion or Escape) ends that session, so the next edit undoes back to the app's value.

use day_core::AnyPiece;
use day_macros::Observable;
use day_mock::{MockProbe, MockToolkit};
use day_model::{Keyed, Store, UndoStack};
use day_pieces::prelude::*;
use day_reactive::flush_sync;
use day_spec::{Event, NodeId, WindowOptions};

#[derive(Observable, Clone, Default, PartialEq, Debug)]
pub struct Page {
    #[obs(key)]
    pub id: u32,
    pub address: String,
}

fn boot(root: impl FnOnce() -> AnyPiece + 'static) -> MockProbe {
    day_core::uninstall_tree();
    let (toolkit, probe) = MockToolkit::new();
    day_core::launch_with(toolkit, WindowOptions::default(), root);
    probe
}

fn field(probe: &MockProbe) -> NodeId {
    let found = probe.find_by_kind("day.text_field");
    assert_eq!(found.len(), 1, "exactly one text field realized");
    NodeId(found[0].1.node)
}

fn setup() -> (Store<Keyed<Page>>, UndoStack, MockProbe, NodeId) {
    let store = Store::new(Keyed::new(vec![Page {
        id: 1,
        address: "initial".into(),
    }]));
    let stack = UndoStack::new(50);
    stack.watch(store);
    let probe = boot(move || text_field(store.elem(1).address()).any());
    flush_sync();
    let node = field(&probe);
    (store, stack, probe, node)
}

#[test]
fn an_app_replacement_ends_the_typing_session() {
    let (store, stack, probe, node) = setup();
    let address = store.elem(1).address();

    // Type, then the app replaces the text while the field is still focused.
    probe.emit(node, Event::FocusChanged(true));
    probe.emit(node, Event::TextChanged("typ".into()));
    probe.emit(node, Event::TextChanged("typed".into()));
    flush_sync();
    address.write("replacement".into());
    flush_sync();
    probe.emit(node, Event::FocusChanged(false));
    flush_sync();
    assert_eq!(
        address.peek(),
        "replacement",
        "focus loss kept the app's text"
    );

    // A later edit is its own unit, and undoing it returns to the replacement.
    probe.emit(node, Event::FocusChanged(true));
    probe.emit(node, Event::TextChanged("next".into()));
    probe.emit(node, Event::FocusChanged(false));
    flush_sync();
    assert_eq!(address.peek(), "next");
    assert!(stack.undo());
    assert_eq!(address.peek(), "replacement");
}

#[test]
fn typing_alone_is_one_undo_unit() {
    let (store, stack, probe, node) = setup();
    let address = store.elem(1).address();
    probe.emit(node, Event::FocusChanged(true));
    for text in ["d", "do", "doc", "docs"] {
        probe.emit(node, Event::TextChanged(text.into()));
    }
    probe.emit(node, Event::Submitted);
    flush_sync();
    assert_eq!(address.peek(), "docs");
    assert!(stack.undo());
    assert_eq!(address.peek(), "initial");
    assert!(!stack.undo());
}
