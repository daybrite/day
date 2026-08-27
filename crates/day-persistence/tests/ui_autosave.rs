// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! A live container behind a mock-toolkit list: autosave at every turn's end must not cost the
//! UI a single patch. Reproduces the Showcase walkthrough shape (type into a bound field, the
//! row label follows) headlessly.

use day_core::AnyPiece;
use day_macros::Model;
use day_model::{Keyed, Op, Store};
use day_persistence::{ModelContainer, Sqlite, schema};
use day_pieces::prelude::*;
use day_reactive::flush_sync;
use day_spec::{Size, WindowOptions};

#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "rows")]
pub struct Task {
    #[model(id)]
    pub id: u32,
    pub name: String,
}

fn boot(root: impl FnOnce() -> AnyPiece + 'static) -> day_mock::MockProbe {
    day_core::uninstall_tree();
    let (mock, probe) = day_mock::MockToolkit::new();
    day_core::launch_with(
        mock,
        WindowOptions {
            title: "t".into(),
            size: Size::new(400.0, 600.0),
            ..Default::default()
        },
        root,
    );
    probe
}

fn count(probe: &day_mock::MockProbe, needle: &str) -> usize {
    probe.log().iter().filter(|l| l.contains(needle)).count()
}

#[test]
fn keystrokes_into_a_container_store_keep_patching_the_row_label() {
    let container =
        ModelContainer::open(Sqlite::memory(), schema![Task]).expect("open memory container");
    let store: Store<Keyed<Task>> = container.cache::<Task>();
    store.restructure("seed", Op::Insert, 1, |v| {
        for n in 1..=3u32 {
            v.push(Task {
                id: n,
                name: format!("Task {n}"),
            });
        }
    });

    let probe = boot(move || {
        list(store, |slot: ModelSlot<Task>| {
            label(move || slot.name().read())
        })
        .row_height(RowHeight::Uniform(40.0))
        .any()
    });
    let host = probe.find_by_kind("day.list")[0].0;
    probe.list_bind(host, 0, day_mock::MockHandle(9400));
    flush_sync();
    let labels = count(&probe, "update day.label");

    let mut text = String::new();
    for ch in "Renamed task".chars() {
        text.push(ch);
        store.elem(1).name().write(text.clone());
        flush_sync(); // one turn per keystroke — autosave flushes SQL between each
    }

    assert_eq!(
        count(&probe, "update day.label") - labels,
        "Renamed task".chars().count(),
        "every keystroke patched the bound row label with autosave active"
    );
    let last = probe
        .log()
        .iter()
        .rev()
        .find(|l| l.contains("update day.label"))
        .cloned()
        .unwrap_or_default();
    assert!(
        last.contains("Renamed task"),
        "final text reached the cell: {last}"
    );
}

/// `list(query, row)`: a predicate flip arrives at the native host as an animatable row
/// delta, not a reload — and a column the query never mentions produces neither.
#[cfg(feature = "pieces")]
#[test]
fn a_query_backed_list_splices_instead_of_reloading() {
    use day_persistence::Sort;

    let container =
        ModelContainer::open(Sqlite::memory(), schema![Task]).expect("open memory container");
    let store: Store<Keyed<Task>> = container.cache::<Task>();
    store.update("seed", |k| {
        *k = Keyed::new(
            (1..=5u32)
                .map(|n| Task {
                    id: n,
                    name: format!("Task {n}"),
                })
                .collect(),
        );
    });
    let query = container
        .query::<Task>()
        .filter(Task::name().contains("Task"))
        .sort(Sort::asc("id"))
        .live();

    let probe = boot(move || {
        list(query, |slot: ModelSlot<Task>| {
            label(move || slot.name().read())
        })
        .row_height(RowHeight::Uniform(40.0))
        .any()
    });
    flush_sync();
    let reloads = count(&probe, "list reload");
    let splices = count(&probe, "list splice");

    // Rename row 3 so it leaves the set: one splice (Remove), zero reloads.
    store.elem(3).name().write("renamed away".into());
    flush_sync();
    assert_eq!(
        count(&probe, "list splice") - splices,
        1,
        "one splice patch"
    );
    assert_eq!(count(&probe, "list reload") - reloads, 0, "no reload");
    assert!(
        probe
            .log()
            .iter()
            .any(|l| l.contains("list splice [Remove(2)]")),
        "the delta names the row: {:?}",
        probe
            .log()
            .iter()
            .filter(|l| l.contains("splice"))
            .collect::<Vec<_>>()
    );

    // A value edit that stays in the set and in place: no splice, no reload.
    let (r, s) = (count(&probe, "list reload"), count(&probe, "list splice"));
    store.elem(1).name().write("Task 1 edited".into());
    flush_sync();
    assert_eq!(count(&probe, "list reload") - r, 0);
    assert_eq!(
        count(&probe, "list splice") - s,
        0,
        "the row repaints itself; the set is untouched"
    );
}
