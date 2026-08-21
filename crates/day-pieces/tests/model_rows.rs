// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The model-driven list's two promises, measured: a field edit costs the one widget showing
//! it (no reload, no rebind, nothing cloned), and a recycled cell scrolled across the whole
//! collection leaves no observation residue behind.

use std::cell::Cell;
use std::rc::Rc;

use day_core::AnyPiece;
use day_macros::Observable;
use day_mock::{MockHandle, MockProbe, MockToolkit};
use day_model::{Keyed, Store};
use day_pieces::prelude::*;
use day_reactive::flush_sync;
use day_spec::{Size, WindowOptions};

#[derive(Observable, Clone, Default, PartialEq, Debug)]
pub struct Row {
    #[obs(key)]
    pub id: u32,
    pub name: String,
    pub done: bool,
}

const ROWS: u32 = 200;

fn store() -> Store<Keyed<Row>> {
    Store::new(Keyed::new(
        (0..ROWS)
            .map(|n| Row {
                id: n,
                name: format!("row {n}"),
                done: n % 4 == 0,
            })
            .collect(),
    ))
}

fn boot(root: impl FnOnce() -> AnyPiece + 'static) -> MockProbe {
    day_core::uninstall_tree();
    let (mock, probe) = MockToolkit::new();
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

fn count(probe: &MockProbe, needle: &str) -> usize {
    probe.log().iter().filter(|l| l.contains(needle)).count()
}

#[test]
fn a_field_edit_patches_one_label_and_reloads_nothing() {
    let store = store();
    let probe = boot(move || {
        list(store, |slot: ModelSlot<Row>| {
            label(move || slot.name().read())
        })
        .row_height(RowHeight::Uniform(40.0))
        .any()
    });
    let host = probe.find_by_kind("day.list")[0].0;
    assert!(
        count(&probe, "list reload") >= 1,
        "building the list told the native host about the initial rows"
    );
    // The "native" side realizes three cells.
    for i in 0..3 {
        probe.list_bind(host, i, MockHandle(9000 + i as u64));
    }
    flush_sync();
    let (labels, reloads) = (
        count(&probe, "update day.label"),
        count(&probe, "list reload"),
    );

    store.elem(1).name().write("edited".into());
    flush_sync();

    assert_eq!(
        count(&probe, "update day.label") - labels,
        1,
        "one label patched — the row showing the edited field"
    );
    assert_eq!(
        count(&probe, "list reload") - reloads,
        0,
        "a value edit reloads nothing: the row SET did not change"
    );
}

#[test]
fn an_order_edit_reloads_and_a_value_edit_does_not() {
    let store = store();
    // A projection the ORDER of which depends on `done` (and nothing else).
    let probe = boot(move || {
        list(
            store.rows(move || {
                let mut keys: Vec<(u64, bool)> = store
                    .keys()
                    .into_iter()
                    .map(|k| {
                        (
                            k,
                            store.elem(k).done().with(|d| d.copied().unwrap_or(false)),
                        )
                    })
                    .collect();
                keys.sort_by_key(|(_, done)| *done);
                keys.into_iter().map(|(k, _)| k).collect()
            }),
            |slot: ModelSlot<Row>| label(move || slot.name().read()),
        )
        .row_height(RowHeight::Uniform(40.0))
        .any()
    });
    let host = probe.find_by_kind("day.list")[0].0;
    probe.list_bind(host, 0, MockHandle(9100));
    flush_sync();
    let reloads = count(&probe, "list reload");

    // A value the projection never reads: no reload.
    store.elem(2).name().write("renamed".into());
    flush_sync();
    assert_eq!(
        count(&probe, "list reload") - reloads,
        0,
        "the projection does not read `name`, so it did not re-run"
    );

    // A value the ORDER depends on: exactly one reload.
    store.elem(2).done().update(|d| *d = !*d);
    flush_sync();
    assert_eq!(
        count(&probe, "list reload") - reloads,
        1,
        "flipping `done` re-ran the projection and reloaded once"
    );
}

/// The massive-list claim: ONE physical cell recycled across the whole collection. The slot's
/// bindings re-track per rebind, and day-model's run-keyed claims release as they go — so the
/// observation tables end where they began, not 200 rows deep.
#[test]
fn recycling_a_cell_across_the_collection_leaves_no_claims() {
    let store = store();
    let probe = boot(move || {
        list(store, |slot: ModelSlot<Row>| {
            label(move || slot.name().read())
        })
        .row_height(RowHeight::Uniform(40.0))
        .any()
    });
    let host = probe.find_by_kind("day.list")[0].0;
    let cell = MockHandle(9200);
    probe.list_bind(host, 0, cell);
    flush_sync();
    let (paths, nodes) = (day_model::observed_paths(), day_model::interned_nodes());

    // Scroll: the same cell rebinds to every row in turn.
    for i in 1..ROWS as usize {
        probe.list_bind(host, i, cell);
        flush_sync();
    }

    assert_eq!(
        day_model::observed_paths(),
        paths,
        "no trigger claims accumulated behind the recycled cell"
    );
    assert_eq!(
        day_model::interned_nodes(),
        nodes,
        "…and no interner slots either"
    );
}

/// Correctness of following: after a rebind, the OLD row's writes no longer reach the cell and
/// the NEW row's do — including through a two-way control bound once at build.
#[test]
fn slot_bindings_follow_the_recycled_row() {
    let store = store();
    let runs = Rc::new(Cell::new(0usize));
    let r = runs.clone();
    let probe = boot(move || {
        list(store, move |slot: ModelSlot<Row>| {
            let r = r.clone();
            column((
                label(move || {
                    r.set(r.get() + 1);
                    slot.name().read()
                }),
                // A control bound ONCE at build: must follow the slot across recycles.
                text_field(slot.name()),
            ))
        })
        .row_height(RowHeight::Uniform(40.0))
        .any()
    });
    let host = probe.find_by_kind("day.list")[0].0;
    let cell = MockHandle(9300);
    probe.list_bind(host, 0, cell);
    flush_sync();
    let base = runs.get();

    // Recycle the cell to row 5.
    probe.list_bind(host, 5, cell);
    flush_sync();
    assert!(runs.get() > base, "the rebind woke the row's bindings");
    let after_rebind = runs.get();

    // The OLD row is somebody else's now.
    store.elem(0).name().write("old row".into());
    flush_sync();
    assert_eq!(
        runs.get(),
        after_rebind,
        "a write to the cell's FORMER row does not wake it"
    );

    // The NEW row is live — and the two-way control wrote through to it.
    store.elem(5).name().write("new row".into());
    flush_sync();
    assert!(
        runs.get() > after_rebind,
        "the cell follows its current row"
    );
    let tf = probe.find_by_kind("day.text_field")[0].1.text.clone();
    assert_eq!(tf, "new row", "the once-built control tracked the recycle");
}
