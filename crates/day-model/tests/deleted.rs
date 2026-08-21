// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! What a deleted row does to its readers: fields read `Default`, and `exists()` is the one
//! TRACKED guard a page needs to degrade instead of panicking.

use std::cell::Cell;
use std::rc::Rc;

use day_core::AnyPiece;
use day_macros::Observable;
use day_mock::MockToolkit;
use day_model::{Keyed, Op, Store};
use day_pieces::prelude::*;
use day_reactive::flush_sync;
use day_spec::{Size, WindowOptions};

#[derive(Observable, Clone, Default, PartialEq, Debug)]
pub struct Item {
    #[obs(key)]
    pub id: u32,
    pub name: String,
}

fn boot(root: impl FnOnce() -> AnyPiece + 'static) {
    day_core::uninstall_tree();
    let (mock, _probe) = MockToolkit::new();
    day_core::launch_with(
        mock,
        WindowOptions {
            title: "t".into(),
            size: Size::new(300.0, 300.0),
            ..Default::default()
        },
        root,
    );
}

fn store() -> Store<Keyed<Item>> {
    Store::new(Keyed::new(vec![Item {
        id: 7,
        name: "here".into(),
    }]))
}

#[test]
fn a_deleted_row_reads_default_and_exists_flips() {
    let store = store();
    let it = store.elem(7);
    let seen = Rc::new(Cell::new(true));
    let s = seen.clone();
    boot(move || {
        label(move || {
            s.set(it.exists());
            it.name().with(|v| v.cloned().unwrap_or_default())
        })
        .any()
    });
    flush_sync();
    assert!(seen.get(), "the row is there");

    store.restructure("remove", Op::Delete, 7, |k| {
        k.remove(7);
    });
    flush_sync();

    assert!(!seen.get(), "the tracked guard re-ran and saw the row gone");
    assert_eq!(
        it.name().with_untracked(|v| v.cloned().unwrap_or_default()),
        "",
        "a field of a gone row reads its Default, never panics"
    );
}

#[test]
fn exists_flips_back_when_the_key_returns() {
    let store = store();
    let it = store.elem(7);
    let seen = Rc::new(Cell::new(true));
    let s = seen.clone();
    boot(move || {
        label(move || {
            format!("{}", {
                s.set(it.exists());
                s.get()
            })
        })
        .any()
    });
    flush_sync();

    store.restructure("remove", Op::Delete, 7, |k| {
        k.remove(7);
    });
    flush_sync();
    assert!(!seen.get());

    store.restructure("push", Op::Insert, 7, |k| {
        k.push(Item {
            id: 7,
            name: "back".into(),
        })
    });
    flush_sync();
    assert!(seen.get(), "the same guard saw the key return");
}

#[test]
fn a_write_to_a_gone_row_is_a_silent_no_op() {
    let store = store();
    store.restructure("remove", Op::Delete, 7, |k| {
        k.remove(7);
    });
    let (_, log) = day_model::record(|| {
        store.elem(7).name().update(|n| n.push('x'));
    });
    assert!(log.is_empty(), "nothing to write to, so nothing announced");
}
