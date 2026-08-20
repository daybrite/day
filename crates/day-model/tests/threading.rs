// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Editing off the main thread, committed atomically, announced on the main thread.

use std::cell::Cell;
use std::rc::Rc;

use day_core::AnyPiece;
use day_macros::Observable;
use day_mock::MockToolkit;
use day_model::{Keyed, Source, Store};
use day_pieces::prelude::*;
use day_reactive::flush_sync;
use day_spec::{Size, WindowOptions};

#[derive(Observable, Clone, Default, PartialEq, Debug)]
struct Item {
    #[obs(key)]
    pub id: u32,
    pub name: String,
    pub count: i64,
}

fn boot(root: impl FnOnce() -> AnyPiece + 'static) {
    day_core::uninstall_tree();
    let (mock, _probe) = MockToolkit::new();
    day_core::launch_with(
        mock,
        WindowOptions {
            title: "t".into(),
            size: Size::new(400.0, 400.0),
            ..Default::default()
        },
        root,
    );
}

fn seed() -> Keyed<Item> {
    let v = (0..4)
        .map(|n| Item {
            id: n,
            name: format!("row {n}"),
            count: 0,
        })
        .collect::<Vec<_>>();
    Keyed::new(v)
}

#[test]
fn a_background_transaction_commits_on_release_and_announces_on_pump() {
    let store = Store::new(seed());
    let name_runs = Rc::new(Cell::new(0usize));
    let other_runs = Rc::new(Cell::new(0usize));
    let (n, o) = (name_runs.clone(), other_runs.clone());

    boot(move || {
        let name = store.elem(2).name();
        let other = store.elem(3).name();
        column((
            label(move || {
                n.set(n.get() + 1);
                name.with(|v| v.cloned().unwrap_or_default())
            }),
            label(move || {
                o.set(o.get() + 1);
                other.with(|v| v.cloned().unwrap_or_default())
            }),
        ))
        .any()
    });
    flush_sync();
    let (base_n, base_o) = (name_runs.get(), other_runs.get());

    // A worker edits two fields of one element and releases.
    std::thread::spawn(move || {
        let mut tx = store.transact();
        tx.data().get_mut(2).unwrap().name = "from a worker".into();
        tx.data().get_mut(2).unwrap().count = 42;
        store.elem(2).name().touch(tx.paths());
        store.elem(2).count().touch(tx.paths());
        // Dropping the guard IS the commit.
    })
    .join()
    .unwrap();

    // Committed: the data is there. Not yet announced: no observer has re-run.
    assert_eq!(
        store.with_untracked(|k| k.get(2).unwrap().name.clone()),
        "from a worker"
    );
    flush_sync();
    assert_eq!(name_runs.get(), base_n, "nothing woke before the pump");

    // The main thread announces.
    let announced = store.pump();
    flush_sync();
    assert_eq!(announced, 2, "both touched paths");
    assert_eq!(
        name_runs.get(),
        base_n + 1,
        "the reader of that field woke once"
    );
    assert_eq!(
        other_runs.get(),
        base_o,
        "the reader of another element did not"
    );
}

#[test]
fn a_reader_never_sees_half_a_transaction() {
    // The write lock IS the atomicity: a main-thread read during the transaction waits for it,
    // so a half-applied edit is not observable. Here the worker holds the guard until told.
    let store = Store::new(seed());
    let (tx_started, go) = (
        std::sync::Arc::new(std::sync::Barrier::new(2)),
        std::sync::Arc::new(std::sync::Barrier::new(2)),
    );
    let (a, b) = (tx_started.clone(), go.clone());

    let worker = std::thread::spawn(move || {
        let mut tx = store.transact();
        tx.data().get_mut(0).unwrap().name = "half".into();
        a.wait(); // the transaction is open and half-applied
        b.wait(); // hold it until the main thread has asked to read
        tx.data().get_mut(1).unwrap().name = "whole".into();
    });

    tx_started.wait();
    go.wait();
    worker.join().unwrap();

    // Both halves landed together; the main thread never observed only the first.
    store.with_untracked(|k| {
        assert_eq!(k.get(0).unwrap().name, "half");
        assert_eq!(k.get(1).unwrap().name, "whole");
    });
}

#[test]
fn the_change_log_is_assertable_without_any_ui() {
    // Headless observation: what did this operation announce?
    let store = Store::new(seed());
    let (_, log) = day_model::record(|| {
        store.elem(1).name().write("x".into());
        store.elem(1).count().update(|c| *c = 9);
    });
    assert_eq!(log, vec!["name", "count"]);

    // …and that a no-op write announces nothing.
    let (_, quiet) = day_model::record(|| {
        store.elem(99).name().write("nobody".into());
    });
    assert!(quiet.is_empty(), "no such element, nothing announced");
}
