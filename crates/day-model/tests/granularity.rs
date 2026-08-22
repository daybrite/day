// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The measurement: how far does one field write travel?
//!
//! Both halves build the SAME UI — 100 rows, each row a label reading one item's name — and count
//! how many of those 100 closures re-run when ONE item's name changes.

use std::cell::Cell;
use std::rc::Rc;

use day_core::AnyPiece;
use day_macros::Observable;
use day_mock::{MockProbe, MockToolkit};
use day_model::{Keyed, Source, Store};
use day_pieces::prelude::*;
use day_reactive::{Signal, flush_sync};
use day_spec::{Size, WindowOptions};

#[derive(Observable, Clone, Default, PartialEq, Debug)]
struct Item {
    #[obs(key)]
    pub id: u32,
    pub name: String,
    pub count: i64,
    pub done: bool,
}

const ROWS: usize = 100;

fn seed() -> Keyed<Item> {
    let v = (0..ROWS as u32)
        .map(|n| Item {
            id: n,
            name: format!("row {n}"),
            count: n as i64,
            done: false,
        })
        .collect::<Vec<_>>();
    Keyed::new(v)
}

fn boot(root: impl FnOnce() -> AnyPiece + 'static) -> MockProbe {
    day_core::uninstall_tree();
    let (mock, probe) = MockToolkit::new();
    day_core::launch_with(
        mock,
        WindowOptions {
            title: "t".into(),
            size: Size::new(400.0, 400.0),
            ..Default::default()
        },
        root,
    );
    probe
}

/// TODAY: one `Signal<Vec<Item>>`. Every row's closure re-runs on any write to any field.
#[test]
fn one_signal_wakes_every_row() {
    let items = Signal::new(seed().items().to_vec());
    let runs = Rc::new(Cell::new(0usize));
    let r = runs.clone();
    let probe = boot(move || {
        let rows: Vec<AnyPiece> = (0..ROWS)
            .map(|i| {
                let r = r.clone();
                label(move || {
                    r.set(r.get() + 1);
                    items.with(|v| v[i].name.clone())
                })
                .any()
            })
            .collect();
        column(day_core::PieceVec(rows)).any()
    });
    flush_sync();
    // Build and the measure pass each run the closures; only the DELTA after the write matters.
    let baseline = runs.get();

    // One field of one item.
    items.update(|v| v[7].name = "edited".into());
    flush_sync();

    let recomputes = runs.get() - baseline;
    let patches = probe
        .log()
        .iter()
        .filter(|l| l.starts_with("update day.label"))
        .count();
    println!("ONE SIGNAL:    recomputes={recomputes} label-patches={patches}");
    assert_eq!(
        recomputes, ROWS,
        "every row re-ran and re-cloned its string"
    );
    // The equality gate means the NATIVE side was already precise — the waste is compute.
    assert_eq!(patches, 1, "…while only one label actually changed");
}

/// PROPOSED: one trigger per (element, field). A write wakes only that field's readers.
#[test]
fn per_property_wakes_one_row() {
    let store = Store::new(seed());
    let runs = Rc::new(Cell::new(0usize));
    let r = runs.clone();
    let probe = boot(move || {
        let rows: Vec<AnyPiece> = (0..ROWS)
            .map(|i| {
                let r = r.clone();
                let name = store.elem(i as u64).name();
                label(move || {
                    r.set(r.get() + 1);
                    name.with(|v| v.cloned().unwrap_or_default())
                })
                .any()
            })
            .collect();
        column(day_core::PieceVec(rows)).any()
    });
    flush_sync();
    let baseline = runs.get();

    store.elem(7).name().write("edited".into());
    flush_sync();

    let recomputes = runs.get() - baseline;
    let patches = probe
        .log()
        .iter()
        .filter(|l| l.starts_with("update day.label"))
        .count();
    println!("PER-PROPERTY:  recomputes={recomputes} label-patches={patches}");
    assert_eq!(recomputes, 1, "only the row whose field changed re-ran");
    assert_eq!(patches, 1);
}

/// A sibling field of the SAME element does not wake a reader of this one.
#[test]
fn a_sibling_field_does_not_wake_it() {
    let store = Store::new(seed());
    let runs = Rc::new(Cell::new(0usize));
    let r = runs.clone();
    boot(move || {
        let name = store.elem(3).name();
        label(move || {
            r.set(r.get() + 1);
            name.with(|v| v.cloned().unwrap_or_default())
        })
        .any()
    });
    flush_sync();
    let baseline = runs.get();

    store.elem(3).count().update(|c| *c += 1);
    store.elem(3).done().write(true);
    flush_sync();
    assert_eq!(runs.get(), baseline, "count and done are not name");

    store.elem(3).name().write("x".into());
    flush_sync();
    assert_eq!(runs.get(), baseline + 1);
}

/// An external merge (another connection's committed write, fed through `merge_row`) wakes
/// exactly the readers of the fields it names — the precision a wholesale reload cannot offer.
#[test]
fn a_merged_row_wakes_only_the_named_fields_readers() {
    let store = Store::new(seed());
    let name_runs = Rc::new(Cell::new(0usize));
    let count_runs = Rc::new(Cell::new(0usize));
    let (n, c) = (name_runs.clone(), count_runs.clone());
    boot(move || {
        let name = store.elem(5).name();
        let count = store.elem(5).count();
        column((
            label(move || {
                n.set(n.get() + 1);
                name.with(|v| v.cloned().unwrap_or_default())
            }),
            label(move || {
                c.set(c.get() + 1);
                count.with(|v| v.copied().unwrap_or(0).to_string())
            }),
        ))
        .any()
    });
    flush_sync();
    let (name_base, count_base) = (name_runs.get(), count_runs.get());

    let mut row = seed().get(5).cloned().expect("row 5 is seeded");
    row.name = "merged".into();
    store.merge_row(5, row, &["name"]);
    flush_sync();

    assert_eq!(
        name_runs.get(),
        name_base + 1,
        "the named field's reader ran"
    );
    assert_eq!(count_runs.get(), count_base, "the sibling's reader did not");
}

/// A COARSE reader — one that asked for the whole store — still wakes on a field write. Precision
/// is something a reader opts into by what it reads, not something writes have to know about.
#[test]
fn a_coarse_reader_still_wakes() {
    let store = Store::new(seed());
    let saves = Rc::new(Cell::new(0usize));
    let s = saves.clone();
    boot(move || {
        label(move || {
            s.set(s.get() + 1);
            format!("{} rows", store.with(|v| v.map(|k| k.len()).unwrap_or(0)))
        })
        .any()
    });
    flush_sync();
    let baseline = saves.get();

    store.elem(2).name().write("x".into());
    flush_sync();
    assert_eq!(saves.get(), baseline + 1, "the whole-store reader saw it");
}

/// Observation costs nothing for paths nobody reads: a trigger exists only where something looked.
#[test]
fn unobserved_paths_have_no_cost() {
    let store = Store::new(seed());
    boot(move || {
        let name = store.elem(0).name();
        label(move || name.with(|v| v.cloned().unwrap_or_default())).any()
    });
    flush_sync();
    // One trigger for the observed field, one for its element, one for the store — the path and
    // its ancestors, created on the way in. NOT 100 rows × 3 fields.
    assert!(
        day_model::observed_paths() <= 3,
        "created {} triggers",
        day_model::observed_paths()
    );
}
