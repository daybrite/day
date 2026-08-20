// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Triggers come back when the thing that was watching goes away.

use day_macros::Observable;
use day_model::{Keyed, Source, Store};
use day_reactive::Scope;

#[derive(Observable, Clone, Default, PartialEq, Debug)]
pub struct Item {
    #[obs(key)]
    pub id: u32,
    pub name: String,
    pub count: i64,
}

fn store() -> Store<Keyed<Item>> {
    Store::new(Keyed::new(
        (0..3)
            .map(|n| Item {
                id: n,
                name: format!("row {n}"),
                count: 0,
            })
            .collect(),
    ))
}

#[test]
fn a_disposed_scope_gives_its_triggers_back() {
    let store = store();
    let before = day_model::observed_paths();

    let page = Scope::child();
    page.enter(|| {
        // Three fields of two elements: 3 paths + 2 element paths + 1 store path.
        store.elem(0).name().with(|_| {});
        store.elem(0).count().with(|_| {});
        store.elem(1).name().with(|_| {});
    });
    let observed = day_model::observed_paths();
    assert!(observed > before, "observing created triggers ({observed})");

    page.dispose();
    assert_eq!(
        day_model::observed_paths(),
        before,
        "every trigger the page created came back"
    );
}

#[test]
fn a_path_two_scopes_watch_survives_one_of_them() {
    let store = store();
    let before = day_model::observed_paths();

    let a = Scope::child();
    let b = Scope::child();
    a.enter(|| store.elem(2).name().with(|_| {}));
    b.enter(|| store.elem(2).name().with(|_| {}));
    let both = day_model::observed_paths();
    assert!(both > before);

    a.dispose();
    assert_eq!(
        day_model::observed_paths(),
        both,
        "b is still watching, so nothing was reclaimed"
    );

    b.dispose();
    assert_eq!(day_model::observed_paths(), before);
}

#[test]
fn re_tracking_in_the_same_scope_does_not_double_count() {
    let store = store();
    let before = day_model::observed_paths();
    let page = Scope::child();
    // A binding re-runs and re-tracks on every change; the claim must be counted once.
    page.enter(|| {
        for _ in 0..10 {
            store.elem(0).name().with(|_| {});
        }
    });
    page.dispose();
    assert_eq!(day_model::observed_paths(), before);
}

#[test]
fn a_reclaimed_path_still_works_when_someone_looks_again() {
    let store = store();
    let page = Scope::child();
    page.enter(|| store.elem(0).name().with(|_| {}));
    page.dispose();

    // Fresh trigger, same behavior — a trigger holds only a counter.
    let again = Scope::child();
    let seen = again.enter(|| {
        store
            .elem(0)
            .name()
            .with(|v| v.cloned().unwrap_or_default())
    });
    assert_eq!(seen, "row 0");
    again.dispose();
}

#[test]
fn the_interner_shrinks_with_the_triggers() {
    let store = store();
    let nodes_before = day_model::interned_nodes();

    let page = Scope::child();
    page.enter(|| {
        store.elem(0).name().with(|_| {});
        store.elem(1).name().with(|_| {});
    });
    assert!(
        day_model::interned_nodes() > nodes_before,
        "observing interned the element slots"
    );

    page.dispose();
    assert_eq!(
        day_model::interned_nodes(),
        nodes_before,
        "the element slots were freed with their last trigger"
    );
}

/// The reclamation safety property: a `Copy` handle held across a free re-interns through its
/// own chain on the next use, so a write through it still wakes observers who arrived later
/// under a fresh interning.
#[test]
fn a_stale_handle_heals_after_reclamation() {
    use std::cell::Cell;
    use std::rc::Rc;

    use day_reactive::{Binding, flush_sync};

    let store = store();
    // Built early, held across the life of the page that observed it.
    let held = store.elem(0).name();

    let page = Scope::child();
    page.enter(|| {
        held.with(|_| {});
    });
    page.dispose(); // the element's interner slot is freed with its trigger

    // A NEW observer interns the element afresh — a different id under the hood.
    let runs = Rc::new(Cell::new(0usize));
    let r = runs.clone();
    let watcher = Scope::child();
    watcher.enter(|| {
        day_reactive::bind(
            move || {
                store
                    .elem(0)
                    .name()
                    .with(|v| v.cloned().unwrap_or_default())
            },
            move |_| r.set(r.get() + 1),
        );
    });
    flush_sync();
    let base = runs.get();

    // Write through the STALE handle.
    held.write("edited".into());
    flush_sync();

    assert_eq!(
        runs.get(),
        base + 1,
        "the fresh observer heard the stale handle's write"
    );
    assert_eq!(
        store.with_untracked(|k| k.get(0).map(|i| i.name.clone())),
        Some("edited".into())
    );
    watcher.dispose();
}
