// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The model-driven list's two promises, measured: a field edit costs the one widget showing
//! it (the row neither reloads nor rebinds, and nothing is cloned), and a recycled cell
//! scrolled across the whole collection leaves no observation residue behind.

use std::cell::Cell;
use std::rc::Rc;

use day_core::AnyPiece;
use day_macros::Observable;
use day_mock::{MockHandle, MockProbe, MockToolkit};
use day_model::{Keyed, Op, Store};
use day_pieces::prelude::*;
use day_reactive::{Signal, flush_sync};
use day_spec::{Event, NodeId, Size, WindowOptions};

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
    // A projection whose order depends on `done` (and nothing else).
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

    // A value the order depends on: exactly one reload.
    store.elem(2).done().update(|d| *d = !*d);
    flush_sync();
    assert_eq!(
        count(&probe, "list reload") - reloads,
        1,
        "flipping `done` re-ran the projection and reloaded once"
    );
}

/// The massive-list claim: One physical cell recycled across the whole collection. The slot's
/// bindings re-track per rebind, and day-model's run-keyed claims release as they go, so the
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

/// Correctness of following: after a rebind, the old row's writes no longer reach the cell and
/// the new row's do, including through a two-way control bound once at build.
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
                // A control bound once at build: must follow the slot across recycles.
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

    // The old row is somebody else's now.
    store.elem(0).name().write("old row".into());
    flush_sync();
    assert_eq!(
        runs.get(),
        after_rebind,
        "a write to the cell's FORMER row does not wake it"
    );

    // The new row is live, and the two-way control wrote through to it.
    store.elem(5).name().write("new row".into());
    flush_sync();
    assert!(
        runs.get() > after_rebind,
        "the cell follows its current row"
    );
    let tf = probe.find_by_kind("day.text_field")[0].1.text.clone();
    assert_eq!(tf, "new row", "the once-built control tracked the recycle");
}

/// Phase-2 regression probe: with a standing change sink installed (the persistence container's
/// shape) and a turn-end callback reading the store, every one of a burst of sequential field
/// writes must still patch the row's label. The Showcase walkthrough caught a row label
/// freezing partway through a rename.
#[test]
fn sequential_writes_keep_patching_under_a_change_sink() {
    let store = store();
    let sink = day_model::install_change_sink(|change| {
        let _ = change.components.first();
    });
    day_reactive::on_turn_end(move || {
        store.with_untracked(|k| {
            let _ = k.items().len();
        });
    });

    let probe = boot(move || {
        list(store, |slot: ModelSlot<Row>| {
            label(move || slot.name().read())
        })
        .row_height(RowHeight::Uniform(40.0))
        .any()
    });
    let host = probe.find_by_kind("day.list")[0].0;
    probe.list_bind(host, 0, MockHandle(9300));
    probe.list_bind(host, 1, MockHandle(9301));
    flush_sync();
    let labels = count(&probe, "update day.label");

    let mut text = String::new();
    for ch in "Renamed task".chars() {
        text.push(ch);
        store.elem(1).name().write(text.clone());
        flush_sync();
    }

    assert_eq!(
        count(&probe, "update day.label") - labels,
        "Renamed task".chars().count(),
        "every keystroke patched the bound row label; none were dropped"
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
    day_model::remove_change_sink(sink);
}

/// A text patch inside a bound cell must re-lay-out that cell. Cell anchors live outside the
/// window trees, so the window pass never reaches them; layout_now's dirty-cell sweep does.
/// Without it, a row label edited to longer text keeps the frame its old text measured and
/// truncates the very text it was just given (the Showcase rename froze at "Re…").
#[test]
fn a_cell_label_edit_re_lays_out_its_cell() {
    let store = store();
    let probe = boot(move || {
        list(store, |slot: ModelSlot<Row>| {
            // Hug the text (no grow): the label's frame width tracks its content, so a stale
            // frame is observable as a missing set_frame after the patch.
            row((label(move || slot.name().read()),))
        })
        .row_height(RowHeight::Uniform(40.0))
        .any()
    });
    let host = probe.find_by_kind("day.list")[0].0;
    probe.list_bind(host, 1, MockHandle(9500));
    flush_sync();
    let frames = count(&probe, "set_frame");

    store
        .elem(1)
        .name()
        .write("a much longer name than before".into());
    flush_sync();

    assert!(
        count(&probe, "set_frame") > frames,
        "the patched cell was re-laid-out (its label's frame follows its text)"
    );
}

/// A text field bound through a row's slot seals the edit in progress when it is torn down
/// (`Binding::write_teardown`), and it resolves its row through the slot to do so. The slot
/// therefore has to outlive the row's own cleanups, whichever way the row goes: removed from
/// the store while the user is typing in it, or closed along with the view around it.
#[test]
fn a_field_torn_down_mid_edit_seals_through_a_slot_that_still_stands() {
    let store = Store::new(Keyed::new(vec![
        Row {
            id: 1,
            name: "one".into(),
            done: false,
        },
        Row {
            id: 2,
            name: "two".into(),
            done: false,
        },
    ]));
    let shown = Signal::new(true);
    let probe = boot(move || {
        when(
            move || shown.get(),
            move || {
                each(store.rows(move || store.keys()), |slot: ModelSlot<Row>| {
                    text_field(slot.name())
                })
            },
        )
        .any()
    });
    let field = |index: usize| NodeId(probe.find_by_kind("day.text_field")[index].1.node);
    assert_eq!(probe.find_by_kind("day.text_field").len(), 2);

    // Typing opens a session on row 1; removing the row disposes the field mid-session.
    probe.emit(field(0), Event::TextChanged("typing".into()));
    flush_sync();
    let ((), changes) = day_model::record_changes(|| {
        store.restructure("remove", Op::Delete, 1u32, |k| {
            k.remove(1);
        });
        flush_sync();
    });
    assert_eq!(
        probe.find_by_kind("day.text_field").len(),
        1,
        "the rest of the turn ran: the surviving row is the one on screen"
    );
    assert!(
        changes.iter().all(|c| c.label == "remove"),
        "the removal is all that was recorded; the gone row's edit left nothing"
    );

    // Typing in the row that stays, then closing the list around it: the edit is in the
    // store, sealed as one change.
    probe.emit(field(0), Event::TextChanged("kept".into()));
    flush_sync();
    let ((), changes) = day_model::record_changes(|| {
        shown.set(false);
        flush_sync();
    });
    assert!(probe.find_by_kind("day.text_field").is_empty());
    assert_eq!(store.elem(2u32).name().peek(), "kept");
    assert_eq!(
        changes.iter().filter(|c| c.label == "name").count(),
        1,
        "the teardown sealed the session into one record"
    );
}

#[test]
fn selection_binds_the_selected_key_both_ways() {
    let store = Store::new(Keyed::new(
        (1..=3)
            .map(|id| Row {
                id,
                ..Default::default()
            })
            .collect(),
    ));
    let selected: Signal<Option<u64>> = Signal::new(Some(2));
    let probe = boot(move || {
        list(store, |slot: ModelSlot<Row>| {
            label(move || slot.name().read())
        })
        .selection(selected)
        .any()
    });
    let host = NodeId(probe.find_by_kind("day.list")[0].0.0);
    // The initial key selects its row without scrolling.
    assert_eq!(count(&probe, "list selected [1]"), 1);
    assert_eq!(count(&probe, "list scroll"), 0);

    // The user picks a row: its key is written back, and the pick is recorded on the native
    // list (one that keeps no selection of its own paints from it) without scrolling.
    probe.emit(host, Event::SelectionChanged(2));
    flush_sync();
    assert_eq!(selected.get(), Some(3));
    assert_eq!(count(&probe, "list selected [2]"), 1);
    assert_eq!(count(&probe, "list scroll"), 0);

    // A row the app adds and selects is selected and revealed once it is in the snapshot.
    store.push(Row {
        id: 4,
        ..Default::default()
    });
    selected.set(Some(4));
    flush_sync();
    assert!(count(&probe, "list selected [3]") >= 1);
    assert_eq!(count(&probe, "list scroll"), 1);

    // A move keeps the selected row selected at its new index.
    store.move_row(3, 0);
    flush_sync();
    assert!(count(&probe, "list selected [0]") >= 1);

    // Clearing the selection natively writes `None`.
    probe.emit(host, Event::SelectionSet(vec![]));
    flush_sync();
    assert_eq!(selected.get(), None);
}

/// Boot with `kinds::LIST_ITEM` realized natively (`Cap::ListItem` = `Native`).
fn boot_native_items(root: impl FnOnce() -> AnyPiece + 'static) -> MockProbe {
    day_core::uninstall_tree();
    let (mock, probe) = MockToolkit::new();
    probe.set_native_list_items(true);
    day_core::launch_with(
        mock,
        WindowOptions {
            title: "t".into(),
            size: Size::new(400.0, 600.0),
            ..Default::default()
        },
        root,
    );
    flush_sync();
    probe
}

/// A list item realizes as the platform's row content where there is one, and each part of a
/// recycled row's item changes in place: a rename, a check mark, never a rebuild.
#[test]
fn list_items_patch_their_parts_in_place() {
    let store = Store::new(Keyed::new(vec![
        Row {
            id: 1,
            name: "Milk".into(),
            done: false,
        },
        Row {
            id: 2,
            name: "Bread".into(),
            done: false,
        },
    ]));
    let probe = boot_native_items(move || {
        list(store, |slot: ModelSlot<Row>| {
            list_item(move || slot.name().read())
                .subtitle("aisle 4")
                .checked(move || slot.done().read())
        })
        .any()
    });
    let host = probe.find_by_kind("day.list")[0].0;
    for i in 0..2 {
        probe.list_bind(host, i, MockHandle(9000 + i as u64));
    }
    flush_sync();
    assert_eq!(
        count(&probe, "realize day.list_item"),
        2,
        "{:#?}",
        probe.log()
    );
    assert_eq!(count(&probe, "title=\"Milk\" subtitle=\"aisle 4\""), 1);
    store.elem(1).done().write(true);
    store.elem(2).name().write("Rye".into());
    flush_sync();
    assert_eq!(
        count(&probe, "list_item Checked(true)"),
        1,
        "{:#?}",
        probe.log()
    );
    assert_eq!(count(&probe, "list_item Title(\"Rye\")"), 1);
    assert_eq!(count(&probe, "realize day.list_item"), 2, "nothing rebuilt");
}

/// Without native row content the same item composes from plain pieces: its title and its
/// subtitle as labels, the check mark following the row.
#[test]
fn list_items_compose_where_the_toolkit_has_no_row_content() {
    let done = Signal::new(false);
    let probe = boot(move || {
        list_item("Milk")
            .subtitle("aisle 4")
            .detail("2")
            .checked(done)
            .any()
    });
    assert_eq!(count(&probe, "realize day.list_item"), 0);
    assert_eq!(count(&probe, "text=\"Milk\""), 1, "{:#?}", probe.log());
    assert_eq!(count(&probe, "text=\"aisle 4\""), 1);
    assert_eq!(count(&probe, "text=\"2\""), 1);
    let images = || count(&probe, "realize day.image");
    let before = images();
    done.set(true);
    flush_sync();
    assert_eq!(images(), before + 1, "the check mark appears");
}
