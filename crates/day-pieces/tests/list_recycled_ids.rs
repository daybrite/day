// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//! Element ids inside a recycling list's rows survive the cell pool: a pooled cell stops
//! answering lookups while it is hidden and answers again once it is bound, even when it is
//! bound to the same row, which writes no signal and so re-runs neither a static `.id()` nor a
//! reactive `.id_of()`. day-core parks the ids on recycle and restores them on the rebind
//! (docs/list.md); without that, a dayscript `wait_for` on a row id fails after any data swap
//! that hands a cell the article it already showed.
use day_core::{AnyPiece, with_tree};
use day_mock::{MockHandle, MockProbe, MockToolkit};
use day_pieces::prelude::*;
use day_reactive::{Signal, flush_sync};
use day_spec::{Size, WindowOptions};

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

/// Whether a dayscript step addressing `id` would find a node right now.
fn has_id(id: &str) -> bool {
    with_tree(|t| t.find_by_id(id)).is_some()
}

#[test]
fn a_pooled_cell_rebound_to_the_same_row_answers_lookups_again() {
    let rows = Signal::new((0..20u64).collect::<Vec<u64>>());
    let probe = boot(move || {
        list(
            items(move || rows.get(), |n: &u64| n.to_string()),
            |slot: ItemSlot<u64, String>| {
                column((label(move || format!("row {}", slot.get())).id(day_core::id!(row_label)),))
                    .id_of(move || day_core::id!(row, slot.get()))
            },
        )
        .row_height(RowHeight::Uniform(40.0))
        .any()
    });
    let host = probe.find_by_kind("day.list")[0].0;
    let cell = MockHandle(9400);
    probe.list_bind(host, 0, cell);
    flush_sync();
    assert!(
        has_id("row:0"),
        "the reactive id is registered on first bind"
    );
    assert!(
        has_id("row_label"),
        "the static id is registered on first bind"
    );

    // Scrolled out: the pooled cell must stop answering, or a hidden row past a shrunk source
    // would satisfy a lookup meant for a visible one.
    probe.list_recycle(host, cell);
    assert!(!has_id("row:0"), "a pooled cell's reactive id is parked");
    assert!(!has_id("row_label"), "a pooled cell's static id is parked");

    // Bound to the same row: an unchanged slot value fires nothing, and the ids must still
    // come back: a search cleared, a refresh, any data swap that lands a cell on the row it
    // already showed.
    probe.list_bind(host, 0, cell);
    flush_sync();
    assert!(
        has_id("row:0"),
        "reactive id restored after a same-row rebind"
    );
    assert!(
        has_id("row_label"),
        "static id restored after a same-row rebind"
    );

    // Bound to another row: the reactive id follows the new row, the static one stays.
    probe.list_recycle(host, cell);
    probe.list_bind(host, 7, cell);
    flush_sync();
    assert!(has_id("row:7"), "reactive id follows the rebound row");
    assert!(!has_id("row:0"), "…and the old row's id is gone");
    assert!(
        has_id("row_label"),
        "static id restored after a cross-row rebind"
    );
}

/// Whether the dayscript ids of a row built in `cell` are registered right now.
fn row_answers(key: u64) -> bool {
    has_id(&format!("row-{key}")) || has_id("row_label")
}

#[test]
fn a_released_cell_takes_its_row_with_it_and_a_rebind_builds_afresh() {
    let built = std::rc::Rc::new(std::cell::Cell::new(0usize));
    let count = built.clone();
    let rows = Signal::new((0..20u64).collect::<Vec<u64>>());
    let probe = boot(move || {
        list(
            items(move || rows.get(), |n: &u64| n.to_string()),
            move |slot: ItemSlot<u64, String>| {
                count.set(count.get() + 1);
                column((label(move || format!("row {}", slot.get())).id(day_core::id!(row_label)),))
                    .id_of(move || day_core::id!(row, slot.get()))
            },
        )
        .row_height(RowHeight::Uniform(40.0))
        .any()
    });
    let host = probe.find_by_kind("day.list")[0].0;
    let cell = MockHandle(9500);
    probe.list_bind(host, 3, cell);
    flush_sync();
    assert!(row_answers(3) && built.get() == 1);

    // The host discards the cell: the row goes with it, ids included, with nothing parked to
    // come back later.
    probe.list_release(host, cell);
    assert!(
        !row_answers(3),
        "a released cell's ids are gone, not parked"
    );
    probe.list_bind(host, 3, cell);
    flush_sync();
    assert!(!row_answers(3) || built.get() == 2);
    assert_eq!(
        built.get(),
        2,
        "the same handle, seen again, is a new cell: built afresh"
    );
    assert!(row_answers(3), "…and answers again");

    // Releasing a cell Day never saw, or one twice, is a no-op.
    probe.list_release(host, MockHandle(9999));
    probe.list_release(host, cell);
    probe.list_release(host, cell);
    assert!(!row_answers(3));
    assert_eq!(built.get(), 2);

    // Recycle, then release: the pooled cell's parked ids do not survive the release.
    probe.list_bind(host, 5, cell);
    flush_sync();
    assert_eq!(built.get(), 3);
    probe.list_recycle(host, cell);
    assert!(!row_answers(5), "parked while pooled");
    probe.list_release(host, cell);
    probe.list_bind(host, 5, cell);
    flush_sync();
    assert_eq!(
        built.get(),
        4,
        "a rebind after the release builds, it does not restore"
    );
    assert!(row_answers(5));
}

#[test]
fn a_release_inside_a_tree_borrow_is_held_not_dropped() {
    let rows = Signal::new((0..20u64).collect::<Vec<u64>>());
    let probe = boot(move || {
        list(
            items(move || rows.get(), |n: &u64| n.to_string()),
            |slot: ItemSlot<u64, String>| {
                column((label(move || format!("row {}", slot.get())).id(day_core::id!(row_label)),))
                    .id_of(move || day_core::id!(row, slot.get()))
            },
        )
        .row_height(RowHeight::Uniform(40.0))
        .any()
    });
    let host = probe.find_by_kind("day.list")[0].0;
    let cell = MockHandle(9600);
    probe.list_bind(host, 0, cell);
    flush_sync();
    assert!(has_id("row:0"));
    // A host that tears a cell down from inside work Day started (a reload, a removal) calls
    // release while the tree is borrowed. The park would be skipped there; the release must
    // not be, or the row leaks: it runs at the borrow's end.
    with_tree(|_| {
        probe.list_release(host, cell);
        assert!(
            with_tree_is_borrowed(),
            "the release must wait: the tree is ours right now"
        );
    });
    assert!(
        !has_id("row:0"),
        "the held release ran once the borrow ended"
    );
}

/// Whether the tree cannot be entered right now (a borrow is live).
fn with_tree_is_borrowed() -> bool {
    day_core::try_with_tree(|_| ()).is_none()
}
