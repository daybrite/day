// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Triggers come back when the thing that was watching goes away.

use day_macros::Observable;
use day_model::{Keyed, Store};
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
    use day_reactive::flush_sync;

    let store = store();
    let before = day_model::observed_paths();

    // Observation happens through COMPUTATIONS: three bindings, three fields of two elements.
    let page = Scope::child();
    page.enter(|| {
        day_reactive::bind(
            move || {
                store
                    .elem(0)
                    .name()
                    .with(|v| v.cloned().unwrap_or_default())
            },
            |_| {},
        );
        day_reactive::bind(
            move || store.elem(0).count().with(|v| v.copied().unwrap_or(0)),
            |_| {},
        );
        day_reactive::bind(
            move || {
                store
                    .elem(1)
                    .name()
                    .with(|v| v.cloned().unwrap_or_default())
            },
            |_| {},
        );
    });
    flush_sync();
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
    use day_reactive::flush_sync;

    let store = store();
    let before = day_model::observed_paths();

    let a = Scope::child();
    let b = Scope::child();
    a.enter(|| {
        day_reactive::bind(
            move || {
                store
                    .elem(2)
                    .name()
                    .with(|v| v.cloned().unwrap_or_default())
            },
            |_| {},
        );
    });
    b.enter(|| {
        day_reactive::bind(
            move || {
                store
                    .elem(2)
                    .name()
                    .with(|v| v.cloned().unwrap_or_default())
            },
            |_| {},
        );
    });
    flush_sync();
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
fn a_re_running_binding_does_not_double_count() {
    use day_reactive::{Binding as _, flush_sync};

    let store = store();
    let before = day_model::observed_paths();
    let page = Scope::child();
    page.enter(|| {
        day_reactive::bind(
            move || {
                store
                    .elem(0)
                    .name()
                    .with(|v| v.cloned().unwrap_or_default())
            },
            |_| {},
        );
    });
    flush_sync();
    let observing = day_model::observed_paths();

    // A binding re-runs and re-tracks on every change; the claim must be counted once.
    for n in 0..10 {
        store.elem(0).name().write(format!("v{n}"));
        flush_sync();
        assert_eq!(day_model::observed_paths(), observing);
    }

    page.dispose();
    assert_eq!(day_model::observed_paths(), before);
}

#[test]
fn a_reclaimed_path_still_works_when_someone_looks_again() {
    use std::cell::RefCell;
    use std::rc::Rc;

    use day_reactive::flush_sync;

    let store = store();
    let page = Scope::child();
    page.enter(|| {
        day_reactive::bind(
            move || {
                store
                    .elem(0)
                    .name()
                    .with(|v| v.cloned().unwrap_or_default())
            },
            |_| {},
        );
    });
    flush_sync();
    page.dispose();

    // Fresh trigger, same behavior — a trigger holds only a counter.
    let seen: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
    let s = seen.clone();
    let again = Scope::child();
    again.enter(|| {
        day_reactive::bind(
            move || {
                store
                    .elem(0)
                    .name()
                    .with(|v| v.cloned().unwrap_or_default())
            },
            move |v: &String| *s.borrow_mut() = v.clone(),
        );
    });
    flush_sync();
    assert_eq!(*seen.borrow(), "row 0");
    again.dispose();
}

#[test]
fn the_interner_shrinks_with_the_triggers() {
    use day_reactive::flush_sync;

    let store = store();
    let nodes_before = day_model::interned_nodes();

    let page = Scope::child();
    page.enter(|| {
        day_reactive::bind(
            move || {
                store
                    .elem(0)
                    .name()
                    .with(|v| v.cloned().unwrap_or_default())
            },
            |_| {},
        );
        day_reactive::bind(
            move || {
                store
                    .elem(1)
                    .name()
                    .with(|v| v.cloned().unwrap_or_default())
            },
            |_| {},
        );
    });
    flush_sync();
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

/// The recycling shape: ONE long-lived binding whose tracked row rotates. Claims made from
/// inside a computation belong to its current RUN and are released on re-track — so old rows'
/// triggers and interner slots do not pile up behind a recycled list cell.
#[test]
fn a_rebinding_computation_releases_the_paths_it_left() {
    use std::cell::Cell;
    use std::rc::Rc;

    use day_reactive::{Signal, flush_sync};

    let store = store();
    let sel = Signal::new(0u64);
    let runs = Rc::new(Cell::new(0usize));
    let r = runs.clone();
    let page = Scope::child();
    page.enter(|| {
        day_reactive::bind(
            move || {
                store
                    .elem(sel.get())
                    .name()
                    .with(|v| v.cloned().unwrap_or_default())
            },
            move |_| r.set(r.get() + 1),
        );
    });
    flush_sync();
    let (paths, nodes) = (day_model::observed_paths(), day_model::interned_nodes());

    // Rotate the binding across every row, twice.
    for k in [1u64, 2, 0, 1, 2, 0] {
        sel.set(k);
        flush_sync();
    }

    assert_eq!(
        day_model::observed_paths(),
        paths,
        "rotating the bound row left no trigger claims behind"
    );
    assert_eq!(
        day_model::interned_nodes(),
        nodes,
        "…and no interner slots either"
    );

    page.dispose();
}

/// Attribution: a claim made during a RE-run (which executes from the flush, not from inside
/// the binding's scope) still belongs to the binding — disposing its scope reclaims everything.
#[test]
fn a_re_run_binding_is_still_reclaimed_after_its_scope_dies() {
    use day_reactive::{Binding as _, flush_sync};

    let store = store();
    let before = day_model::observed_paths();
    let page = Scope::child();
    page.enter(|| {
        day_reactive::bind(
            move || {
                store
                    .elem(1)
                    .name()
                    .with(|v| v.cloned().unwrap_or_default())
            },
            |_| {},
        );
    });
    flush_sync();

    // Force a re-run from the flush context.
    store.elem(1).name().write("renamed".into());
    flush_sync();

    page.dispose();
    assert_eq!(
        day_model::observed_paths(),
        before,
        "the re-run's claim belonged to the binding, not to the flusher"
    );
}
